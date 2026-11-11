//! Phase 13a: the main-thread side of 0043B420 ModelLoader_LoadModel and the QueuedModel IOTask it queues on the IOManager (00B33A10, the Phase 09 BSTaskManager with one BSTaskManagerThread). Phase 13e: the IO-thread side, 00433BC0's walk and per-task body, as `ModelDrain`.
//!
//! Ported in original order: the case-insensitive model map lookup (0043E680), the a9 reference on a cached Model, the QueuedModel constructor 00437250 (IOTask base 00436500, path copy 00434600, archive file entry 00434CB0, 00434710, 0042DB10, 0042D240, the flag byte +0x34), 0043AC40 with no parent, the queue 00435000, 00435020, 004320A0 with the 64-bit key (00436FF0, 00436560, 004365F0), and the task map insertion 00431FF0 (state 1, the map's reference, the bucket counter, the thread semaphore).
//!
//! **Host substitution (D9, the plan's "Worker threads cannot hold World"):** the original's IO thread runs 00433BC0 while the main thread queues; the host runs its body on the main thread after the queue, one pass per `ModelDrain::step`. The walk is the exe's (00433760: ascending keys through the 18 buckets, and a restart at bucket 0 after a pass that ran something), so the order is the map's walk, with only the arrival of tasks as the timing it can't know. P13-RUN-001's 3 models at 004067D7 and 8 at 00406944 are that walk with tasks arriving during it, and `game_data_window` replays them. Everything queued first, as in `run_to_end`, starts the walk at the lowest key instead of at the first task queued; tasks don't read each other's results past the model map and `draw_rand`, so only that order and the rand draws differ.
//!
//! Host substitutions: the task and Model pointers are `IoTaskId` and `ModelId`. The maps' lock-free links, marks and hazard records (00432A60, 004331F0's CAS loops) are ordered `Vec`s, because only the main thread touches them. The 8 Havok and 3 NIF 10.2.0.0 models stay queued at state 1 with `QueuedModel::host_gate` (the stream loader can't build them), where the original builds them. Evidence: objdump of each address named here; P13-RUN-001 records L, LR, MM; P15-RUN-001 records MD, NO.

pub use crate::gamebryo::scene_node::object_by_name;
use crate::gamebryo::transform::NiTransform;
use crate::tes::forms_references::save_load_flags;
use bevy_ecs::prelude::*;
use oblivion_core::crt::{c_tolower, stricmp, strnicmp};
use oblivion_core::error::GameError;

use super::archive::{ArchiveAccess, ArchiveManager, BsaRecordKey, bsa_path_hashes};
use super::file::GameFile;
use super::nif_objects::BsxFlagsExtraData;
use super::sky_shader::{PrepareGeometryContext, prepare_object_geometry};
use super::stream::{
    BsStreamLoad, BsStreamLoadStatus, BsStreamServices, load_bs_stream_with_reader,
};
use super::task_manager::{IoManager, ModelLoader, TaskGlobals};
use crate::gamebryo::av_object::{detach_property_of_type, find_property_of_type};
use crate::gamebryo::av_object_palette::DefaultAvObjectPalette;
use crate::gamebryo::controller_sequence::ControllerManager;
use crate::gamebryo::mesh_geometry::MeshTriShapePayload;
use crate::gamebryo::object_net::get_controller;
use crate::gamebryo::object_net::{NiObjectNet, find_extra_data};
use crate::gamebryo::properties::{VertexColorProperty, ZBufferProperty};
use crate::gamebryo::registration::NiProcessDefaults;
use crate::gamebryo::scene_node::{
    NiNodeChildren, SceneNodePayload, detach_child, set_child_at, spawn_scene_node,
};
use crate::gamebryo::stream_file_pipeline::StreamFileHeaderErrorCode;
use crate::gamebryo::stream_objects::{NULL_REFERENCE, NiClass};
use crate::gamebryo::wildcard::wildcard_match;
use crate::tes::helmet_tasks::HelmetTasks;

/// IDA: the vtables of the QueuedModel chain (00436506, 00436531, 00437291).
pub const BS_TASK_VTABLE: u32 = 0x00A3_6710;
pub const IO_TASK_VTABLE: u32 = 0x00A3_6968;
pub const QUEUED_MODEL_VTABLE: u32 = 0x00A3_6B6C;
/// IDA: 0043E750 `div [ebx+8]`: the model map's bucket count, from 0043E950's 00438DC0(4, 1009, 12).
pub const MODEL_MAP_BUCKETS: u32 = 1009;
/// IDA: 0043E6AC: the lookup's lower-case copy lives in 0x3E8 stack bytes.
const MODEL_KEY_BUFFER: usize = 0x3E8;
/// IDA: 00434CDB: 00434710 writes into 0x104 stack bytes.
const MODEL_PATH_BUFFER: usize = 0x104;

/// IDA: N/A a QueuedModel's pointer, a host value that's never printed or compared with evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IoTaskId(pub usize);

/// IDA: N/A what a QueuedFile's +0x18 parent points at: a reference task (QueuedReference and its subclasses), or another IO
/// task (QueuedHelmet `0043BAF0`, whose child is slot 1's QueuedModel).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TaskParent {
    Reference(super::reference_tasks::ReferenceTaskId),
    Io(IoTaskId),
}

impl From<super::reference_tasks::ReferenceTaskId> for TaskParent {
    fn from(id: super::reference_tasks::ReferenceTaskId) -> Self {
        Self::Reference(id)
    }
}

/// IDA: N/A a Model's pointer, a host value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModelId(pub usize);

/// IDA: the BSTask counters: 00B33A20 live tasks (00436511, and 00436670 down), 00B33A14 the key's low 16 bits (00436FFB, 004365FA, 004320F0), 00B33A18 keys with no file entry (00436613).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskCounters {
    pub live_tasks_b33a20: u32,
    pub sequence_b33a14: u32,
    pub unpacked_sequence_b33a18: u32,
}

/// IDA: 00436880 Model (0xC): +0x00 the FormHeap path copy, +0x04 the reference count, +0x08 the root NiNode. Only the IO-thread side builds one (`run_queued_model`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Model {
    pub path_00: Vec<u8>,
    pub references_04: u32,
    pub root_08: Option<Entity>,
}

/// IDA: 0042D240's result: the BSA file record (16 bytes: +0x08 size, +0x0C offset). The record's identity stands in for its pointer, as in `BsaRecordKey`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveFileEntry {
    pub record: BsaRecordKey,
    pub size_08: u32,
    pub offset_0c: u32,
}

/// IDA: 00437250 QueuedModel (0x38): the BSTask base (+0x04 unset, +0x08 references, +0x0C state, +0x10 and +0x14 the 64-bit key), the QueuedFile part (+0x18 parent, +0x1C children, +0x20 path, +0x24 file entry) and +0x28 Model, +0x2C 0, +0x30 the LOD argument, +0x34 the flag byte (bit 2 a7, bit 0 a8, bit 1 a9). P13-RUN-001: +0x04 and the three bytes above +0x34 are FormHeap garbage, so they aren't kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedModel {
    pub original_vtable: u32,
    pub references_08: u32,
    pub state_0c: u32,
    pub key_10: u64,
    pub parent_18: Option<TaskParent>,
    /// The +0x1C QueuedChildren's count less its finished count (+0x10): 0 with no list, or with every child finished.
    pub children_1c: u32,
    pub path_20: Vec<u8>,
    pub entry_24: Option<ArchiveFileEntry>,
    pub model_28: Option<ModelId>,
    pub field_2c: u32,
    pub lod_30: u32,
    pub flags_34: u8,
    /// IDA: N/A why the host stream loader can't build this task's model. It stays queued at state 1, where the original builds it.
    pub host_gate: Option<ModelHostGate>,
}

/// IDA: N/A the two groups of preload models the host stream loader refuses before any payload, so nothing of the original's state has moved. P15-RUN-001 shows the original builds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelHostGate {
    /// `bhk*` blocks: the Havok runtime isn't ported.
    Havok,
    /// A file at a NIF version the header reader doesn't bind (named for the 10.x files it first held back).
    Nif102,
}

/// IDA: the IOManager's task map entries (the 00432520 LockFreeMap<__int64, NiPointer<BSTask>>, 18 buckets): each bucket sorted ascending, `store` owning the tasks. `ModelDrain` walks it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IoTaskMap {
    /// IDA: N/A the tasks' memory: a released task leaves a `None`.
    pub store: Vec<Option<QueuedModel>>,
    /// +0x0C of the map, a host view: the ID of each node in bucket order.
    pub buckets: Vec<Vec<IoTaskId>>,
}

impl IoTaskMap {
    /// IDA: N/A every task in the map, by bucket, then in chain order.
    pub fn queued(&self) -> impl Iterator<Item = &QueuedModel> + '_ {
        self.buckets
            .iter()
            .flatten()
            .filter_map(|id| self.store[id.0].as_ref())
    }
}

/// IDA: one 0055F120 node of the model map: +0x00 the 0043E620 lower-case key, +0x04 the Model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelMapEntry {
    pub key: Vec<u8>,
    pub model: ModelId,
}

/// IDA: ModelLoader +0x00's entries: 1009 buckets, each sorted by strcmp (vt+0x28 0043E2B0, `>=`), equality by strcmp (vt+0x2C 0043E390). 004395D0 inserts (`insert_model`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelMap {
    pub store: Vec<Model>,
    pub buckets: Vec<Vec<ModelMapEntry>>,
}

// IDA: the CRT tolower 00984E85 as 0043E6B2 and 0043E731 call it: `movsx eax, al`, so a byte above 0x7F goes in negative and comes out unchanged. The result is a signed int, and 0043E741 adds it signed.
fn model_tolower(byte: u8) -> i32 {
    c_tolower(i32::from(byte as i8))
}

/// IDA: 0043E720 vt+0x1C: h = tolower(c) + 33 * h from 0, then `div` by the bucket count. Completion: 100%.
#[must_use]
pub fn model_map_bucket(key: &[u8]) -> u32 {
    let mut hash = 0_u32;
    for &byte in key {
        if byte == 0 {
            break;
        }
        hash = (model_tolower(byte) as u32).wrapping_add(hash.wrapping_mul(33));
    }
    hash % MODEL_MAP_BUCKETS
}

// IDA: 0043E69E..0043E6CF the lower-case copy, with no length check: past the 0x3E8 stack bytes is a host stop.
fn lower_model_key(path: &[u8]) -> Result<Vec<u8>, GameError> {
    if path.len() >= MODEL_KEY_BUFFER {
        return Err(GameError::unclassified(
            0x0043E6BE,
            "0043E6BE: a model key past the 0x3E8 stack bytes",
        ));
    }
    Ok(path.iter().map(|&byte| model_tolower(byte) as u8).collect())
}

/// IDA: 0043E680 vt+4 Get(path, &out) on the calling thread. As `model_map_lookup_on`. Completion: 100%.
/// # Errors
/// As `model_map_lookup_on`.
pub fn model_map_lookup(
    loader: &mut ModelLoader,
    path: &[u8],
) -> Result<Option<ModelId>, GameError> {
    model_map_lookup_on(loader, path, std::thread::current().id())
}

/// IDA: 0043E680 vt+4 Get(path, &out): the lower-case key, the thread's interface (TlsGetValue, 00431D60), the bucket, 0043A780's walk (0043A260). A found node's Model. Completion: 100% over the host store; the node's +0x08 mark (0043A7B0) has no host form, since nothing removes a node.
/// # Errors
/// A key past the stack buffer, or no free thread slot.
pub fn model_map_lookup_on(
    loader: &mut ModelLoader,
    path: &[u8],
    thread: std::thread::ThreadId,
) -> Result<Option<ModelId>, GameError> {
    let key = lower_model_key(path)?;
    loader.model_map_00.lock_14.register_thread(thread)?;
    let bucket = model_map_bucket(&key) as usize;
    Ok(loader
        .models
        .buckets
        .get(bucket)
        .and_then(|chain| chain.iter().find(|entry| entry.key == key))
        .map(|entry| entry.model))
}

/// IDA: 004384E0 ModelLoader_LookupAndReleaseModelMapEntry(path, release, 1) on the calling thread: a found Model's +0x04
/// drops by one (the delta-1 arm's InterlockedDecrement), then with `release` 00435A10 ModelLoader_ReleaseModelMapEntry: a
/// low word of 0 with no g_saveLoadGame, or its +0x18 without 0x800, or with 0x4000, takes the map's vt+0x10 remove and
/// the Model's destructor and free. `save_flags` is +0x18, `None` for a null g_saveLoadGame. Completion: 100% of the
/// delta-1 call; a removed Model's store slot is left empty (path and root cleared), and its root node, which the host
/// doesn't count, stays in the World.
/// # Errors
/// As `model_map_lookup_on`.
pub fn lookup_and_release_model(
    loader: &mut ModelLoader,
    path: &[u8],
    release: bool,
    save_flags: Option<u32>,
) -> Result<(), GameError> {
    let Some(model) = model_map_lookup(loader, path)? else {
        return Ok(());
    };
    let held = &mut loader.models.store[model.0];
    held.references_04 = held.references_04.wrapping_sub(1);
    if !release || held.references_04 & 0xFFFF != 0 {
        return Ok(());
    }
    if save_flags.is_some_and(|flags| flags & 0x800 != 0 && flags & 0x4000 == 0) {
        return Ok(());
    }
    let key = lower_model_key(path)?;
    let bucket = model_map_bucket(&key) as usize;
    if let Some(chain) = loader.models.buckets.get_mut(bucket) {
        chain.retain(|entry| entry.key != key);
    }
    let held = &mut loader.models.store[model.0];
    held.path_00.clear();
    held.root_08 = None;
    Ok(())
}

/// IDA: 00434710 the model path fixup, ecx (g_modelLoader) unused: longer than 3 with _stricmp(last 3, "spt") == 0 takes "Trees" (A366C4) in front; otherwise no _strnicmp(path, "Meshes\\", 7) match takes "Meshes\\" (A366BC) in front; otherwise a plain copy. Completion: 100%.
#[must_use]
pub fn fixup_model_path(path: &[u8]) -> Vec<u8> {
    if path.len() > 3 && stricmp(&path[path.len() - 3..], b"spt") == 0 {
        return [b"Trees".as_slice(), path].concat();
    }
    if strnicmp(path, b"Meshes\\", 7) != 0 {
        return [b"Meshes\\".as_slice(), path].concat();
    }
    path.to_vec()
}

// IDA: N/A the archive API's text. Paths here hold bytes, each byte a char as `path_text` does for textures.
fn path_text(path: &[u8]) -> String {
    path.iter().map(|&byte| char::from(byte)).collect()
}

/// IDA: 00434CB0(task, type, fixup) with type 0, fixup 1: a null path stores nothing; 00434710 into the stack, 006FA1B0's two hashes, then 0042DB10(0, ...): the first archive of type 0 (00B338E8), else the secondary (00B3390C), else null; 0042D240 with the path. Completion: 100% over the host archives.
/// # Errors
/// A path past the 0x104 stack bytes.
pub fn find_model_file_entry(
    archives: Option<&mut ArchiveManager>,
    path: &[u8],
) -> Result<Option<ArchiveFileEntry>, GameError> {
    let fixed = fixup_model_path(path);
    if fixed.len() >= MODEL_PATH_BUFFER {
        return Err(GameError::unclassified(
            0x00434CE7,
            "00434CE7: a model path past the 0x104 stack bytes",
        ));
    }
    let Some(archives) = archives else {
        return Ok(None);
    };
    let text = path_text(&fixed);
    let hashes = bsa_path_hashes(&text);
    let Some(record) = archives.file_record_by_type(0, hashes, &text) else {
        return Ok(None);
    };
    let file = &archives.archives()[record.archive].folders()[record.folder as usize].files
        [record.file as usize];
    Ok(Some(ArchiveFileEntry {
        record,
        size_08: file.size,
        offset_0c: file.offset,
    }))
}

// IDA: 00434520 BSTaskManager vt+0x1C: the key's byte 2 (00984A90 shifts by 16, `movzx`) times 3, plus its byte 7 (`sar ecx, 0x18`, `movzx edx, cl`). Completion: 100%.
const fn task_bucket(key: u64) -> usize {
    let byte_2 = ((key >> 16) & 0xFF) as usize;
    let byte_7 = ((key >> 56) & 0xFF) as usize;
    byte_2 * 3 + byte_7
}

/// IDA: 00436560(high, offset, priority, sequence): `movsx` the high byte, `(((high << 32) + offset) * 0x100 + priority) * 0x10000 + sequence`, with the 64-bit multiply 00983C60 wrapping. Completion: 100%.
#[must_use]
pub const fn make_task_key(high: i8, offset: u32, priority: u8, sequence: u16) -> u64 {
    let key = ((high as i64 as u64) << 32).wrapping_add(offset as u64);
    let key = key.wrapping_mul(0x100).wrapping_add(priority as u64);
    key.wrapping_mul(0x10000).wrapping_add(sequence as u64)
}

// IDA: 00437490 vt+0x18 -> 00436FF0: with a file entry, 00B33A14's new value (low word), the key's byte 2, the entry's offset & 0x7FFFFFFF and vt+0x2C (00977C50: `mov al, 1`) into 00436560; without one, 004365F0: 00B33A14, the byte, 00B33A18's new value + 0x200000000 (`add`, `adc ecx, 2`), then the same two multiplies. Completion: 100%.
fn compute_task_key(task: &mut QueuedModel, counters: &mut TaskCounters) {
    counters.sequence_b33a14 = counters.sequence_b33a14.wrapping_add(1);
    let sequence = counters.sequence_b33a14 as u16;
    let priority = (task.key_10 >> 16) as u8;
    if let Some(entry) = task.entry_24 {
        task.key_10 = make_task_key(1, entry.offset_0c & 0x7FFF_FFFF, priority, sequence);
        return;
    }
    counters.unpacked_sequence_b33a18 = counters.unpacked_sequence_b33a18.wrapping_add(1);
    let unpacked = u64::from(counters.unpacked_sequence_b33a18) + 0x2_0000_0000;
    task.key_10 = unpacked
        .wrapping_mul(0x100)
        .wrapping_add(u64::from(priority))
        .wrapping_mul(0x10000)
        .wrapping_add(u64::from(sequence));
}

// IDA: 00433F20 vt+0x0C(key, task, 0) with 004331F0 and 00432A60: the thread's interface (00431D60), the bucket (vt+0x1C), a walk to the first node whose key is at or past the target (vt+0x28 00432630, signed on the high dword and unsigned on the low: `jl`, `jg`, `jb`). An equal key (vt+0x2C 00432660) with a5 = 0 fails. Otherwise 00432690's node takes a reference and vt+0x30 (004324C0) counts +0x18. 00433F8F then drops the by-value argument's reference. Completion: 100% over the host map.
fn insert_task(io: &mut IoManager, id: IoTaskId) -> Result<bool, GameError> {
    io.task_map.lock_14.register_current_thread()?;
    let key = task_of(&io.tasks, id)?.key_10;
    let bucket = task_bucket(key);
    let store = &io.tasks.store;
    let chain = io.tasks.buckets.get(bucket).ok_or(GameError::unclassified(
        0x004331F0,
        "004331F0: a task key outside the map's buckets",
    ))?;
    let position = chain
        .iter()
        .position(|other| {
            store[other.0]
                .as_ref()
                .is_some_and(|task| task.key_10 as i64 >= key as i64)
        })
        .unwrap_or(chain.len());
    let equal = chain
        .get(position)
        .and_then(|other| store[other.0].as_ref())
        .is_some_and(|task| task.key_10 == key);
    if !equal {
        io.tasks.buckets[bucket].insert(position, id);
        task_of_mut(&mut io.tasks, id)?.references_08 += 1;
        io.task_map.field_18 += 1;
    }
    task_of_mut(&mut io.tasks, id)?.references_08 -= 1;
    Ok(!equal)
}

// IDA: 00431FF0 BSTaskManager_AddTask(task): +0x0C = 1 and the by-value argument's reference (0043200E), vt+0x0C; a failed insertion returns 0 with the state left at 1. Then +0x2C[vt+0x1C(key)] + 1, and the thread vt+0x4C picks (004943A0: 0): when its +0x0C reads 0, +0x0C + 1 and ReleaseSemaphore(+0x14, 1). Completion: 100%.
fn add_task(io: &mut IoManager, id: IoTaskId) -> Result<bool, GameError> {
    let task = task_of_mut(&mut io.tasks, id)?;
    task.state_0c = 1;
    task.references_08 += 1;
    if !insert_task(io, id)? {
        return Ok(false);
    }
    let bucket = task_bucket(task_of(&io.tasks, id)?.key_10);
    *io.array_2c.get_mut(bucket).ok_or(GameError::unclassified(
        0x00432040,
        "00432040: a task bucket outside +0x2C",
    ))? += 1;
    io.threads_28
        .first()
        .ok_or(GameError::unclassified(
            0x00432053,
            "00432053: the IOManager has no task thread",
        ))?
        .wake();
    Ok(true)
}

// IDA: N/A the task of an ID.
fn task_of(map: &IoTaskMap, id: IoTaskId) -> Result<&QueuedModel, GameError> {
    map.store[id.0]
        .as_ref()
        .ok_or(GameError::unclassified(0, "a released QueuedModel"))
}

// IDA: N/A a helper, with no address of its own.
fn task_of_mut(map: &mut IoTaskMap, id: IoTaskId) -> Result<&mut QueuedModel, GameError> {
    map.store[id.0]
        .as_mut()
        .ok_or(GameError::unclassified(0, "a released QueuedModel"))
}

/// IDA: 004320A0 IOManager vt+0x3C(task): +0x38 != 6 reads the key's byte 2 and clears it when it's at or below +0x38 (`jg`, signed); task vt+0x18; 00431FF0, and each failure (00B33A14's new low word in place of the key's low word, the high words kept) tries again. Completion: 100%.
/// # Errors
/// As `insert_task`.
pub fn queue_io_task(
    io: &mut IoManager,
    counters: &mut TaskCounters,
    id: IoTaskId,
) -> Result<(), GameError> {
    if io.field_38 != 6 {
        let task = task_of_mut(&mut io.tasks, id)?;
        let priority = i32::from((task.key_10 >> 16) as u8);
        if priority <= io.field_38 as i32 {
            task.key_10 &= !0x00FF_0000;
        }
    }
    compute_task_key(task_of_mut(&mut io.tasks, id)?, counters);
    while !add_task(io, id)? {
        counters.sequence_b33a14 = counters.sequence_b33a14.wrapping_add(1);
        let sequence = u64::from(counters.sequence_b33a14 as u16);
        let task = task_of_mut(&mut io.tasks, id)?;
        task.key_10 = (task.key_10 & 0xFFFF_FFFF_FFFF_0000) + sequence;
    }
    Ok(())
}

// IDA: 00436500 IOTask(priority), then 00437250 QueuedModel(path, priority, lod, a7, a8, a9): vtable A36710, +0x08..+0x14 = 0, 00B33A20 + 1, vtable A36968, +0x10 = (low dword & FF00FFFF) + (priority << 16); +0x18..+0x24 = 0, vtable A36B6C, +0x28 = 0, +0x30 = lod, +0x2C = 0, +0x34 = 0; 00434600 the path copy; 00434CB0(0, 1) the entry; a7 sets or clears 4, a8 1, a9 2 in that order. Completion: 100% of the store order.
fn construct_queued_model(
    counters: &mut TaskCounters,
    archives: Option<&mut ArchiveManager>,
    path: &[u8],
    priority: u8,
    lod: u32,
    flags: [bool; 3],
) -> Result<QueuedModel, GameError> {
    counters.live_tasks_b33a20 = counters.live_tasks_b33a20.wrapping_add(1);
    let mut task = QueuedModel {
        original_vtable: QUEUED_MODEL_VTABLE,
        references_08: 0,
        state_0c: 0,
        key_10: u64::from(priority) << 16,
        parent_18: None,
        children_1c: 0,
        path_20: path.to_vec(),
        entry_24: None,
        model_28: None,
        field_2c: 0,
        lod_30: lod,
        flags_34: 0,
        host_gate: None,
    };
    task.entry_24 = find_model_file_entry(archives, path)?;
    for (set, bit) in flags.into_iter().zip([4, 1, 2]) {
        if set {
            task.flags_34 |= bit;
        } else {
            task.flags_34 &= !bit;
        }
    }
    Ok(task)
}

/// IDA: N/A what 0043B420 takes past its path, named: a4 priority, a5 the parent task, a6 the LOD, a7..a9 the flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelRequest {
    pub priority: u8,
    pub parent: Option<TaskParent>,
    pub lod: u32,
    pub a7: bool,
    pub a8: bool,
    pub a9: bool,
}

/// IDA: 0043B420 ModelLoader_LoadModel(out, path, a4..a9). The model map's vt+4. A hit: with a9, Model +0x04 + 1 (0043B47A); with a5 = 0, a null out (0043B517). A miss: FormHeap(0x38) and 00437250, the local reference (0043B571), 0043AC40(a5), vt+0x20 (00435000 passes 0 for a null +0x2C, then 00435020: flag 8 clear queues through g_IOManager vt+0x3C), out's reference (0043B59A), then the local one dropped (0043B5AB). Returns out. Completion: 100% of what startup reaches; both parent arms stop.
/// TODO: [ ] 0043B48C the a5 != 0 cached-task arm 00437430, and 0043AC89 0043A7E0's parent link, when a caller passes a parent (M5, the cell loads).
/// # Errors
/// No ModelLoader or IOManager (the original faults), or a stop.
pub fn load_model(
    tasks: &mut TaskGlobals,
    archives: Option<&mut ArchiveManager>,
    path: &[u8],
    request: ModelRequest,
) -> Result<Option<IoTaskId>, GameError> {
    let TaskGlobals {
        io_manager_b33a10,
        model_loader_b33a1c,
        counters,
        ..
    } = tasks;
    let loader = model_loader_b33a1c.as_mut().ok_or(GameError::unclassified(
        0x0043B44D,
        "0043B44D: a null g_modelLoader",
    ))?;
    if let Some(model) = model_map_lookup(loader, path)? {
        if request.a9 {
            loader.models.store[model.0].references_04 += 1;
        }
        let Some(parent) = request.parent else {
            return Ok(None);
        };
        // 0043B4A2 FormHeap(0x38) 00437430 QueuedModel_ConstructorFromModel: the IOTask with the priority, +0x18..+0x24 0, the Model with a reference (+0x04 up), state 5. 0043AC40 the parent, then vt+0x28 004374A0, whose flag 8 is clear: `QueuedFile_NotifyParent`, the caller's (it needs the world).
        loader.models.store[model.0].references_04 += 1;
        counters.live_tasks_b33a20 = counters.live_tasks_b33a20.wrapping_add(1);
        let task = QueuedModel {
            original_vtable: QUEUED_MODEL_VTABLE,
            references_08: 0,
            state_0c: 5,
            key_10: u64::from(request.priority) << 16,
            parent_18: None,
            children_1c: 0,
            path_20: Vec::new(),
            entry_24: None,
            model_28: Some(model),
            field_2c: 0,
            lod_30: 0,
            flags_34: 0,
            host_gate: None,
        };
        let io = io_manager_b33a10.as_mut().ok_or(GameError::unclassified(
            0x0043B4A2,
            "0043B4A2: a null g_IOManager",
        ))?;
        let id = IoTaskId(io.tasks.store.len());
        io.tasks.store.push(Some(task));
        set_parent(io, loader, id, parent)?;
        return Ok(Some(id));
    }
    let task = construct_queued_model(
        counters,
        archives,
        path,
        request.priority,
        request.lod,
        [request.a7, request.a8, request.a9],
    )?;
    let queue = task.flags_34 & 8 == 0;
    let io = io_manager_b33a10.as_mut().ok_or(GameError::unclassified(
        0x00435026,
        "00435026: a null g_IOManager",
    ))?;
    let id = IoTaskId(io.tasks.store.len());
    io.tasks.store.push(Some(task));
    task_of_mut(&mut io.tasks, id)?.references_08 += 1;
    if let Some(parent) = request.parent {
        set_parent(io, loader, id, parent)?;
    }
    if queue {
        queue_io_task(io, counters, id)?;
    }
    let task = task_of_mut(&mut io.tasks, id)?;
    task.references_08 += 1;
    task.references_08 -= 1;
    Ok(Some(id))
}

/// IDA: N/A a gameplay caller's loan of the driver's model services (the stream, the archives, the geometry prepare), for
/// main-thread code that clones and attaches parts, as 00479780 does inside an actor's update.
#[derive(Resource, Clone, Copy)]
pub struct GameplayModelServices(pub GameplayModelServicesFn);

/// IDA: N/A the loan's shape: the body runs with the driver's model services.
pub type GameplayModelServicesFn = fn(
    &mut World,
    &mut dyn FnMut(&mut World, &mut ModelBuildServices<'_>) -> Result<(), GameError>,
) -> Result<(), GameError>;

/// IDA: N/A a gameplay caller's synchronous model load with the driver's file and renderer services.
#[derive(Resource, Clone, Copy)]
pub struct SynchronousModelLoad(pub SynchronousModelLoadFn);

/// IDA: N/A ModelLoader_LoadModelNow's shape: (path, a3, a4, a5), the model's root.
pub type SynchronousModelLoadFn =
    fn(&mut World, &[u8], bool, u32, bool) -> Result<Option<Entity>, GameError>;

/// IDA: 00439EB0 ModelLoader_LoadModelNow(path, a3, a4, a5): the map's vt+4 (0043E680); a hit's Model +0x04 counts up and
/// its +0x08 root answers. A miss: off the IO thread 00432860 pauses it (D9 runs every thread's work on the main thread, so a
/// host no-op), then a stack QueuedModel 00437250(path, 0, a4, a3 == 0, a5, 0), +0x34 |= 0x20 (build a Model even when the
/// stream fails), 00439400 QueuedModel_Run, 004395D0 QueuedModel_Finalize, 00432890; the task's +0x28 is taken, the task's
/// reference to it dropped as the task goes (00436E90, 00B33A20 - 1), and a nonnull one counts up and answers its +0x08.
/// Completion: 100%; the host task lives in the store, never in a bucket, and is released at once.
/// # Errors
/// No ModelLoader or IOManager, or a stop in the run.
pub fn load_model_now(
    world: &mut World,
    path: &[u8],
    a3: bool,
    lod: u32,
    a5: bool,
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, GameError> {
    let thread = std::thread::current().id();
    {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let loader = tasks
            .model_loader_b33a1c
            .as_mut()
            .ok_or(GameError::unclassified(
                0x00439EB7,
                "00439EB7: a null g_modelLoader",
            ))?;
        if let Some(model) = model_map_lookup_on(loader, path, thread)? {
            let held = &mut loader.models.store[model.0];
            held.references_04 = held.references_04.wrapping_add(1);
            return Ok(held.root_08);
        }
    }
    let mut built = None;
    {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let counters = &mut tasks.counters;
        let mut construct = |archives: Option<&mut ArchiveManager>| {
            built = Some(construct_queued_model(
                counters,
                archives,
                path,
                0,
                lod,
                [!a3, a5, false],
            ));
        };
        match services.archives.as_mut() {
            Some(archives) => archives.with_archives(&mut construct),
            None => construct(None),
        }
    }
    let mut task = built.expect("the body runs")?;
    task.flags_34 |= 0x20;
    task.references_08 = 1;
    let id = {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let io = io_manager_mut(&mut tasks)?;
        io.tasks.store.push(Some(task));
        IoTaskId(io.tasks.store.len() - 1)
    };
    let mut trace = ModelTaskTrace {
        path: path.to_vec(),
        ..ModelTaskTrace::default()
    };
    let mut record = ModelDrainRecord::default();
    if let Some(gate) = run_queued_model(world, id, services, &mut trace, &mut record, thread)? {
        return Err(GameError::from_legacy_message(match gate {
            ModelHostGate::Havok => "00439F33: a Havok model loaded at once",
            ModelHostGate::Nif102 => "00439F33: a NIF 10.2 model loaded at once",
        }));
    }
    finalize_queued_model(world, id, &mut trace, thread)?;
    let model = {
        let tasks = world.resource::<TaskGlobals>();
        task_of(
            &tasks
                .io_manager_b33a10
                .as_ref()
                .ok_or(GameError::unclassified(
                    0x00439F45,
                    "00439F45: a null g_IOManager",
                ))?
                .tasks,
            id,
        )?
        .model_28
    };
    let mut tasks = world.resource_mut::<TaskGlobals>();
    release_task(&mut tasks, Some(id))?;
    let Some(model) = model else {
        return Ok(None);
    };
    let held = &mut tasks
        .model_loader_b33a1c
        .as_mut()
        .ok_or(GameError::unclassified(
            0x00439F80,
            "00439F80: a null g_modelLoader",
        ))?
        .models
        .store[model.0];
    held.references_04 = held.references_04.wrapping_add(1);
    Ok(held.root_08)
}

// IDA: 0043AC40 QueuedFile_SetParent(parent): +0x18 takes the parent (with a reference), then 0043A7E0 appends the child to the parent's +0x1C QueuedChildren (made on first use, FormHeap 0x14, capacity 5 growing by 10). Completion: 100% for a reference task's parent and an IO task's (its list held as the unfinished count, `children_1c`).
pub(crate) fn set_parent(
    io: &mut IoManager,
    loader: &mut super::task_manager::ModelLoader,
    child: IoTaskId,
    parent: impl Into<TaskParent>,
) -> Result<(), GameError> {
    let parent = parent.into();
    task_of_mut(&mut io.tasks, child)?.parent_18 = Some(parent);
    match parent {
        TaskParent::Reference(parent) => {
            let task = loader
                .references
                .get_mut(parent)
                .ok_or(GameError::unclassified(
                    0x0043A7E0,
                    "0043A7E0: a released parent task",
                ))?;
            task.references_08 += 1;
            task.children_1c.push(child);
        }
        TaskParent::Io(parent) => {
            let task = task_of_mut(&mut io.tasks, parent)?;
            task.references_08 += 1;
            task.children_1c += 1;
        }
    }
    Ok(())
}

/// IDA: a caller's NiPointer release of an out task, as 004435C3..004435D9 does it: a null skips; InterlockedDecrement(+0x08), and at zero vt+0 (004393B0: Model +0x04 - 1, the path freed, 00436E90's QueuedFile part, 00436670's 00B33A20 - 1). Completion: 100% of what's modelled.
/// # Errors
/// A task already released.
pub fn release_task(tasks: &mut TaskGlobals, out: Option<IoTaskId>) -> Result<(), GameError> {
    let Some(id) = out else {
        return Ok(());
    };
    let TaskGlobals {
        io_manager_b33a10,
        model_loader_b33a1c,
        counters,
        ..
    } = tasks;
    let io = io_manager_b33a10.as_mut().ok_or(GameError::unclassified(
        0x004435C3,
        "004435C3: a null g_IOManager",
    ))?;
    let task = task_of_mut(&mut io.tasks, id)?;
    task.references_08 -= 1;
    if task.references_08 != 0 {
        return Ok(());
    }
    if let (Some(model), Some(loader)) = (task.model_28, model_loader_b33a1c.as_mut()) {
        let references = &mut loader.models.store[model.0].references_04;
        *references = references.wrapping_sub(1);
    }
    io.tasks.store[id.0] = None;
    counters.live_tasks_b33a20 = counters.live_tasks_b33a20.wrapping_sub(1);
    Ok(())
}

// ---- 13e: the IO-thread side ----

/// IDA: N/A what a pass did to one task, in walk order. `ModelDrainRecord::tasks`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelTaskTrace {
    pub path: Vec<u8>,
    pub key: u64,
    pub cached: bool,
    pub built: bool,
    pub inserted: bool,
    pub gate: Option<ModelHostGate>,
    /// IDA: N/A a failed load's status and error, for the host's log.
    pub load_failure: Option<String>,
}

/// IDA: N/A the counts of G3's `Phase-13e:` line, and the tasks in the order the walk ran them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelDrainRecord {
    pub run: u32,
    pub built: u32,
    pub cached: u32,
    pub load_failed: u32,
    pub queued_havok: u32,
    pub queued_nif_10_2: u32,
    pub tasks: Vec<ModelTaskTrace>,
}

impl ModelDrainRecord {
    /// IDA: N/A `main.cpp`'s `Phase-13e:` line, with `shader_setups_not_run` from the `unported_shader_setup` boundary.
    #[must_use]
    pub fn log_line(&self, shader_setups_not_run: u32) -> String {
        format!(
            "Phase-13e: model tasks run {} (models built {}, cached {}, failed {}), queued {} ({} Havok, {} NIF 10.2.0.0), shader setups not run {}",
            self.run,
            self.built,
            self.cached,
            self.load_failed,
            self.queued_havok + self.queued_nif_10_2,
            self.queued_havok,
            self.queued_nif_10_2,
            shader_setups_not_run
        )
    }
}

/// IDA: N/A the services 00433BC0's body reaches. `stream.draw_rand` stands for the CRT rand on the IO thread: 009859DD reads +0x14 of 0098C0F5's per-thread data, so that thread has its own sequence (`oblivion_core::crt::MsvcRand`, never the main thread's), and the walk's order decides its draws. `prepare.property_defaults` is 00707610's.
pub struct ModelBuildServices<'a> {
    pub archives: Option<&'a mut dyn ArchiveAccess>,
    pub stream: BsStreamServices<'a>,
    pub prepare: PrepareGeometryContext<'a>,
    /// 00B06B18 g_disableWarningMessages.
    pub disable_warning_messages_b06b18: &'a mut u8,
    /// 00B34390 g_showEditorMarkers.
    pub show_editor_markers_b34390: u8,
    /// 004A7A60 DebugMessage_Warning: (text, 00B06B18's state).
    pub warning: &'a mut (dyn FnMut(Vec<u8>, bool) + 'a),
    /// 0047DF80 Random_Next(0) on the main thread, for the main thread's finish (`00470BF0`'s variant pick). None: a path with no main-thread CRT, where a draw is a stop.
    pub main_random_next: Option<&'a mut (dyn FnMut() -> i32 + 'a)>,
}

/// IDA: the 0x1C-byte iterator on 00433BC0's stack (vtable A364F8): +0x08 the bucket, +0x10 and +0x14 the key, +0x18 flags (bit 0 a bucket has been entered, bit 1 the walk's end).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TaskMapIterator {
    bucket_08: u32,
    key_10: u64,
    flags_18: u8,
}

/// IDA: N/A what one `ModelDrain::step` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelDrainStep {
    /// A task ran, or went back to state 1 at a host gate.
    Ran,
    /// 00433CB2: the task found wasn't at state 1, or had a host gate.
    Skipped,
    /// 00433CED: a pass that ran something reached the end of a bucket, so the walk starts again at bucket 0.
    Restarted,
    /// 00433D5E: the walk passed every bucket with nothing to run, and the thread would wait on +0x14.
    Ended,
    /// 00433C45: a pause holds the +0x20 token, and the thread would block.
    WaitingForToken,
}

/// IDA: 00433BC0 BSTaskManagerThread's run loop past its +0x14 wait, as resumable steps on the main thread (host substitution). A step is one 00433C37..00433D64 pass, so a time budget can return between any two: order doesn't change, only frame boundaries do. `new` is 00433C0F's iterator reset after a wake. Completion: 100% of the walk, with `QueuedModel::host_gate` the one addition.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelDrain {
    iterator: TaskMapIterator,
    /// IDA: the byte at 00433BC0's [esp+0x17], "this pass ran something": 00433CD9 sets it.
    ran_17: bool,
    /// IDA: N/A the step found the token held, so the next one finishes 00433C45's wait.
    waiting_for_token: bool,
    pub record: ModelDrainRecord,
}

impl ModelDrain {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// IDA: 00433C0F's iterator reset after a wake on +0x14, keeping the record. Completion: 100%.
    pub const fn wake(&mut self) {
        self.iterator = TaskMapIterator {
            bucket_08: 0,
            key_10: 0,
            flags_18: 0,
        };
        self.ran_17 = false;
        self.waiting_for_token = false;
    }

    /// IDA: one pass of 00433C37..00433D64. The token first (00433C3C with a timeout of 0); 00433C4B's infinite wait comes back as `WaitingForToken` until the token frees, then +0x18 drops by one and the iterator resets (00433C71, leaving [esp+0x17]). 00433CA5 00433760 with `restart` = "nothing ran yet". A task: 00433CB6's state test and 00433CC0's CAS 1 -> 3, 00433CD9 "ran", vt+0x40 00434350 Run, vt+0x44 00434360 Finalize and removal. No task and not at the end: 00433CF4 the iterator resets and "ran" clears. Then 00433D20 +0x18 up and +0x20 back, 00433D45 the iterator's reference. Completion: 100%.
    /// # Errors
    /// No IOManager or ModelLoader, or a host stop in the task.
    pub fn step(
        &mut self,
        world: &mut World,
        services: &mut ModelBuildServices<'_>,
    ) -> Result<ModelDrainStep, GameError> {
        if self.iterator.flags_18 & 2 != 0 {
            return Ok(ModelDrainStep::Ended);
        }
        let io_thread = {
            let tasks = world.resource::<TaskGlobals>();
            let thread = tasks
                .io_manager_b33a10
                .as_ref()
                .ok_or(GameError::unclassified(
                    0x00433BC0,
                    "00433BC0: a null g_IOManager",
                ))?
                .threads_28
                .first()
                .ok_or(GameError::unclassified(
                    0x00433BC0,
                    "00433BC0: the IOManager has no task thread",
                ))?;
            if !thread.try_take_walk_token() {
                self.waiting_for_token = true;
                return Ok(ModelDrainStep::WaitingForToken);
            }
            if self.waiting_for_token {
                thread.count_waited_walk_token();
                self.waiting_for_token = false;
                self.iterator = TaskMapIterator::default();
            }
            thread.thread_id_08
        };
        let restart = !self.ran_17;
        let found = {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let io = tasks
                .io_manager_b33a10
                .as_mut()
                .ok_or(GameError::unclassified(
                    0x00433BC0,
                    "00433BC0: a null g_IOManager",
                ))?;
            next_task(io, &mut self.iterator, restart, io_thread)?
        };
        let outcome = match found {
            Some(id) => {
                let runnable = {
                    let tasks = world.resource::<TaskGlobals>();
                    let io = tasks
                        .io_manager_b33a10
                        .as_ref()
                        .ok_or(GameError::unclassified(
                            0x00433BC0,
                            "00433BC0: a null g_IOManager",
                        ))?;
                    let task = task_of(&io.tasks, id)?;
                    task.state_0c == 1 && task.host_gate.is_none()
                };
                let step = if runnable {
                    self.ran_17 = true;
                    self.run_task(world, id, services, io_thread)?;
                    ModelDrainStep::Ran
                } else {
                    ModelDrainStep::Skipped
                };
                give_back_token(world)?;
                release_task(&mut world.resource_mut::<TaskGlobals>(), Some(id))?;
                step
            }
            None => {
                let step = if self.iterator.flags_18 & 2 == 0 {
                    self.iterator = TaskMapIterator::default();
                    self.ran_17 = false;
                    ModelDrainStep::Restarted
                } else {
                    ModelDrainStep::Ended
                };
                give_back_token(world)?;
                step
            }
        };
        Ok(outcome)
    }

    /// IDA: N/A the steps straight through, as the tests run them. Stops at `Ended`, or at `WaitingForToken`, which nothing on this thread can free. Completion: 100%.
    /// # Errors
    /// As `step`.
    pub fn run_to_end(
        &mut self,
        world: &mut World,
        services: &mut ModelBuildServices<'_>,
    ) -> Result<ModelDrainStep, GameError> {
        loop {
            let step = self.step(world, services)?;
            if matches!(
                step,
                ModelDrainStep::Ended | ModelDrainStep::WaitingForToken
            ) {
                // The clone thread's queue and 00433590's task queue, after the IO work (D9).
                crate::tes::reference_3d::run_reference_queues(world, services)?;
                return Ok(step);
            }
        }
    }

    // IDA: 00433CBB state 3, then vt+0x40 and vt+0x44. A host gate puts the task back at state 1 and counts it.
    fn run_task(
        &mut self,
        world: &mut World,
        id: IoTaskId,
        services: &mut ModelBuildServices<'_>,
        io_thread: std::thread::ThreadId,
    ) -> Result<(), GameError> {
        let mut trace = {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let io = io_manager_mut(&mut tasks)?;
            let task = task_of_mut(&mut io.tasks, id)?;
            task.state_0c = 3;
            ModelTaskTrace {
                path: task.path_20.clone(),
                key: task.key_10,
                ..ModelTaskTrace::default()
            }
        };
        // FaceGen (background agent): a QueuedHead's vt+4 is 00435300, not QueuedModel's 00439400.
        let head = task_of(
            &io_manager_mut(&mut world.resource_mut::<TaskGlobals>())?.tasks,
            id,
        )?
        .original_vtable
            == crate::tes::face_gen_tasks::QUEUED_HEAD_VTABLE;
        let kf = task_of(
            &io_manager_mut(&mut world.resource_mut::<TaskGlobals>())?.tasks,
            id,
        )?
        .original_vtable
            == super::kf_model::QUEUED_KF_VTABLE;
        let helmet = task_of(
            &io_manager_mut(&mut world.resource_mut::<TaskGlobals>())?.tasks,
            id,
        )?
        .original_vtable
            == crate::tes::helmet_tasks::QUEUED_HELMET_VTABLE;
        if helmet {
            // A QueuedHelmet's vt+4 is 00437B40, its vt+8 00435260 (vt+0x28, 0043EB80 CheckFinished at state 4); the tail's
            // vt+0x28 is CheckFinished again, whose test and notification are `queued_file_notify_parent`'s once it has run.
            crate::tes::helmet_tasks::run_queued_helmet_in_walk(world, id, services)?;
            {
                let mut tasks = world.resource_mut::<TaskGlobals>();
                task_of_mut(&mut io_manager_mut(&mut tasks)?.tasks, id)?.state_0c = 4;
            }
            crate::tes::helmet_tasks::check_finished(world, id, services)?;
            finish_io_task_tail(world, id, io_thread)?;
            crate::tes::helmet_tasks::run_pending_helmet_checks(world, services)?;
            self.record.run += 1;
            self.record.tasks.push(trace);
            return Ok(());
        }
        if kf {
            // A QueuedKF's vt+4 is 00439940 and its vt+8 004378F0.
            super::kf_model::run_queued_kf_in_walk(world, id, services)?;
            finish_io_task_tail(world, id, io_thread)?;
            self.record.run += 1;
            self.record.tasks.push(trace);
            return Ok(());
        }
        let gate = if head {
            crate::tes::face_gen_tasks::run_queued_head_in_walk(world, id, false, services)?;
            None
        } else {
            run_queued_model(world, id, services, &mut trace, &mut self.record, io_thread)?
        };
        if let Some(gate) = gate {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let task = task_of_mut(&mut io_manager_mut(&mut tasks)?.tasks, id)?;
            task.state_0c = 1;
            task.host_gate = Some(gate);
            trace.gate = Some(gate);
            match gate {
                ModelHostGate::Havok => self.record.queued_havok += 1,
                ModelHostGate::Nif102 => self.record.queued_nif_10_2 += 1,
            }
            self.record.tasks.push(trace);
            return Ok(());
        }
        finish_queued_model(world, id, &mut trace, io_thread)?;
        // A helmet's child notified it in the finish: its vt+0x28 (0043EB80) runs here, with the walk's services (D9).
        crate::tes::helmet_tasks::run_pending_helmet_checks(world, services)?;
        self.record.run += 1;
        self.record.tasks.push(trace);
        Ok(())
    }
}

// IDA: N/A a resource's IOManager.
fn io_manager_mut(tasks: &mut TaskGlobals) -> Result<&mut IoManager, GameError> {
    tasks
        .io_manager_b33a10
        .as_mut()
        .ok_or(GameError::unclassified(
            0x00433BC0,
            "00433BC0: a null g_IOManager",
        ))
}

// IDA: 00433D1F, 00433D2D.
fn give_back_token(world: &World) -> Result<(), GameError> {
    world
        .resource::<TaskGlobals>()
        .io_manager_b33a10
        .as_ref()
        .and_then(|io| io.threads_28.first())
        .ok_or(GameError::unclassified(
            0x00433D1F,
            "00433D1F: the IOManager has no task thread",
        ))?
        .give_back_walk_token();
    Ok(())
}

// IDA: 00433760(iterator, &key, &out, restart): at the end flag, false. Otherwise TlsGetValue and 00431D60, then 00431E10 (bucket's first, flag bit 0 set whatever it finds) or 00432ED0 (the first key past +0x10). A hit stores its key and takes a reference (00431EEF, 00432F86). A miss: bucket + 1; at +0x08 (`jae`) the end flag and false; `restart` false returns false with the bucket advanced; otherwise bit 0 clears and the next bucket goes. Keys compare as in `insert_task`. Completion: 100%.
fn next_task(
    io: &mut IoManager,
    iterator: &mut TaskMapIterator,
    restart: bool,
    thread: std::thread::ThreadId,
) -> Result<Option<IoTaskId>, GameError> {
    if iterator.flags_18 & 2 != 0 {
        return Ok(None);
    }
    loop {
        io.task_map.lock_14.register_thread(thread)?;
        let chain =
            io.tasks
                .buckets
                .get(iterator.bucket_08 as usize)
                .ok_or(GameError::unclassified(
                    0x00433760,
                    "00433760: a bucket past the map's",
                ))?;
        let store = &io.tasks.store;
        let key_of = |id: &IoTaskId| store[id.0].as_ref().map_or(0, |task| task.key_10);
        let found = if iterator.flags_18 & 1 == 0 {
            iterator.flags_18 |= 1;
            chain.first().copied()
        } else {
            let from = iterator.key_10 as i64;
            chain.iter().find(|id| (key_of(id) as i64) > from).copied()
        };
        if let Some(id) = found {
            let task = task_of_mut(&mut io.tasks, id)?;
            iterator.key_10 = task.key_10;
            task.references_08 += 1;
            return Ok(Some(id));
        }
        iterator.bucket_08 += 1;
        if iterator.bucket_08 >= io.task_map.bucket_count_08 {
            iterator.flags_18 |= 2;
            return Ok(None);
        }
        if !restart {
            return Ok(None);
        }
        iterator.flags_18 &= !1;
    }
}

// IDA: 00435AB0 NiPointer<Model>::operator=, and the two inline copies in 00439400: an equal pointer skips; the old Model's +0x04 down (no free), the store, the new one's +0x04 up. Completion: 100%.
fn assign_model(
    tasks: &mut TaskGlobals,
    id: IoTaskId,
    model: Option<ModelId>,
) -> Result<(), GameError> {
    let TaskGlobals {
        io_manager_b33a10,
        model_loader_b33a1c,
        ..
    } = tasks;
    let task = task_of_mut(
        &mut io_manager_b33a10
            .as_mut()
            .ok_or(GameError::unclassified(
                0x00435AB0,
                "00435AB0: a null g_IOManager",
            ))?
            .tasks,
        id,
    )?;
    if task.model_28 == model {
        return Ok(());
    }
    let loader = model_loader_b33a1c.as_mut().ok_or(GameError::unclassified(
        0x00435AB0,
        "00435AB0: a null g_modelLoader",
    ))?;
    if let Some(old) = task.model_28 {
        let references = &mut loader.models.store[old.0].references_04;
        *references = references.wrapping_sub(1);
    }
    task.model_28 = model;
    if let Some(new) = model {
        let references = &mut loader.models.store[new.0].references_04;
        *references = references.wrapping_add(1);
    }
    Ok(())
}

// IDA: 0042EBC0(0, entry, -1, null): bit 31 of the entry's offset picks the slot (00B338E8[0], or +0x24 for the secondary: 0042EBDC's `type + bit * 9`), a null slot returns null; 0042EBF4's +0x1A8 count, 0042EC02's second read of the bit (a change takes the other slot), 0042E1A0(entry, -1, null), 0042EC4E the count back. Completion: 100% over the host archive, whose `open_packed_file` holds that count. The entry's record stands for its pointer, so a slot that isn't the record's archive stops; only a rebuilt archive list could do that.
pub(crate) fn open_archive_entry(
    archives: &ArchiveManager,
    entry: ArchiveFileEntry,
) -> Result<Option<Box<dyn GameFile>>, GameError> {
    let slots = if entry.offset_0c & 0x8000_0000 != 0 {
        archives.secondary_by_type()
    } else {
        archives.first_by_type()
    };
    let Some(index) = slots[0] else {
        return Ok(None);
    };
    if index != entry.record.archive {
        return Err(GameError::unclassified(
            0x0042EBE2,
            "0042EBE2: a file entry from another archive than the slot's",
        ));
    }
    Ok(archives.archives()[index]
        .open_packed_file(entry.record.folder, entry.record.file, -1, "")
        .map(super::file::OpenedFile::into_game_file))
}

// IDA: N/A the host stream loader's refusals, which leave the original's state untouched: the header and the object table are read before any payload.
fn host_gate(load: &BsStreamLoad) -> Option<ModelHostGate> {
    if load.status != BsStreamLoadStatus::LoadFailed {
        return None;
    }
    let error = load.error.as_ref()?;
    if error
        .header
        .as_ref()
        .is_some_and(|header| header.code == StreamFileHeaderErrorCode::UnsupportedVersion)
    {
        return Some(ModelHostGate::Nif102);
    }
    let table = error.graph.as_ref()?.table_error.as_ref()?;
    table
        .name
        .starts_with("bhk")
        .then_some(ModelHostGate::Havok)
}

// IDA: 00439400 QueuedModel_Run (vt+4): the map's vt+4 into +0x28 through the assign; a hit sets +0x34 |= 0x10 and returns. 00434710 into 0x104 stack bytes; the reader: +0x24 through 0042EBC0, else a nonnull +0x20 through 0042E8E0(path, -1, 1), else null; 00713A30 NiStream with BSStream's vtable A3690C; 006F9980(stream, fixed path, reader). True, or +0x34 & 0x20: FormHeap(0xC), 004390B0(model, +0x20, stream, +0x34 & 1), assign. False: +0x28 stays null, and Finalize still runs. 00436420 the stream goes. Completion: 100% with the gate.
fn run_queued_model(
    world: &mut World,
    id: IoTaskId,
    services: &mut ModelBuildServices<'_>,
    trace: &mut ModelTaskTrace,
    record: &mut ModelDrainRecord,
    io_thread: std::thread::ThreadId,
) -> Result<Option<ModelHostGate>, GameError> {
    let (path, entry, flags) = {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let TaskGlobals {
            io_manager_b33a10,
            model_loader_b33a1c,
            ..
        } = &mut *tasks;
        let task = task_of(
            &io_manager_b33a10
                .as_ref()
                .ok_or(GameError::unclassified(
                    0x00439443,
                    "00439443: a null g_IOManager",
                ))?
                .tasks,
            id,
        )?;
        let (path, entry, flags) = (task.path_20.clone(), task.entry_24, task.flags_34);
        let loader = model_loader_b33a1c.as_mut().ok_or(GameError::unclassified(
            0x00439443,
            "00439443: a null g_modelLoader",
        ))?;
        let found = model_map_lookup_on(loader, &path, io_thread)?;
        assign_model(&mut tasks, id, found)?;
        if found.is_some() {
            let io = io_manager_mut(&mut tasks)?;
            task_of_mut(&mut io.tasks, id)?.flags_34 |= 0x10;
            trace.cached = true;
            record.cached += 1;
            return Ok(None);
        }
        (path, entry, flags)
    };
    // 004394AD 00434710, with no length check on its 0x104 stack bytes.
    let source = fixup_model_path(&path);
    if source.len() >= MODEL_PATH_BUFFER {
        return Err(GameError::unclassified(
            0x004394AD,
            "004394AD: a model path past the 0x104 stack bytes",
        ));
    }
    let mut reader = Ok(None);
    let mut read = |archives: Option<&mut ArchiveManager>| {
        reader = match (entry, archives) {
            (Some(entry), Some(archives)) => open_archive_entry(archives, entry),
            (Some(_), None) => Err(GameError::unclassified(
                0x004394BE,
                "004394BE: 0042EBC0 with no archives",
            )),
            (None, Some(archives)) => Ok(archives
                .open_packed(&path_text(&path), -1, 1)
                .map(super::file::OpenedFile::into_game_file)),
            (None, None) => Ok(None),
        };
    };
    match services.archives.as_mut() {
        Some(archives) => archives.with_archives(&mut read),
        None => read(None),
    }
    let reader = reader?;
    let load =
        load_bs_stream_with_reader(world, &path_text(&source), reader, &mut services.stream)?;
    if let Some(gate) = host_gate(&load) {
        return Ok(Some(gate));
    }
    if load.status == BsStreamLoadStatus::Loaded || flags & 0x20 != 0 {
        let model = init_model(world, &path, &load, flags & 1 != 0, services)?;
        let id_model = {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let loader = tasks
                .model_loader_b33a1c
                .as_mut()
                .ok_or(GameError::unclassified(
                    0x00439526,
                    "00439526: a null g_modelLoader",
                ))?;
            loader.models.store.push(model);
            ModelId(loader.models.store.len() - 1)
        };
        assign_model(&mut world.resource_mut::<TaskGlobals>(), id, Some(id_model))?;
        trace.built = true;
        record.built += 1;
    } else {
        record.load_failed += 1;
        trace.load_failure = Some(format!("{:?} {:?}", load.status, load.error));
    }
    Ok(None)
}

/// IDA: 00B34404 and 00B34400, 004809A0's two globals: the root it's working under, and that root's NiControllerManager +0x7C object palette. The C++ calls them 00B343FC and a sequence owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct EditorMarkerState {
    root_b34404: Option<Entity>,
    palette_b34400: Option<Entity>,
}

// IDA: 004809A0 the EditorMarker strip(object), recursive, with 00B34404 and 00B34400 as `state`. A null is false. With no root: a "BSX" extra whose +0x0C has bit 5 makes this object the root, and its NiControllerManager's +0x7C the palette; no root returns false. Then a name matching "EditorMarker" (`_stricmp`; a null name skips): with 00B34390 set, +0x18 |= 1 and true, leaving the state to the root's frame. Otherwise a palette's vt+0x50("EditorMarker", null) and ("EditorMarker:0", null), the +0x1C parent's vt+0x88 detach, the state cleared if this is the root, and true. No match: each slot below +0xB6 with a nonnull vt+8, stopping at the first true; the state cleared if this is the root; the last child's result. Completion: 100%. A parent's null detach faults in the original, so it stops.
fn strip_editor_marker(
    world: &mut World,
    object: Entity,
    state: &mut EditorMarkerState,
    show_editor_markers: u8,
) -> Result<bool, GameError> {
    if state.root_b34404.is_none() {
        if let Some(bsx) = find_extra_data(world, object, b"BSX")? {
            let flags = world
                .get::<BsxFlagsExtraData>(bsx)
                .ok_or(GameError::unclassified(
                    0x004809CF,
                    "004809CF: a \"BSX\" extra with no host +0x0C",
                ))?
                .flags_0c;
            if (flags >> 5) & 1 != 0 {
                state.root_b34404 = Some(object);
                if let Some(manager) = get_controller(world, object, NiClass::ControllerManager) {
                    state.palette_b34400 = world
                        .get::<ControllerManager>(manager)
                        .and_then(|manager| manager.object_palette_7c);
                }
            }
        }
        if state.root_b34404.is_none() {
            return Ok(false);
        }
    }
    let name = world
        .get::<NiObjectNet>(object)
        .and_then(|net| net.name.as_deref());
    if name.is_some_and(|name| stricmp(name, b"EditorMarker") == 0) {
        if show_editor_markers != 0 {
            let mut payload =
                world
                    .get_mut::<SceneNodePayload>(object)
                    .ok_or(GameError::unclassified(
                        0x00480A25,
                        "00480A25: +0x18 of a non-node",
                    ))?;
            payload.effective_av_flags |= 1;
            return Ok(true);
        }
        if let Some(palette) = state.palette_b34400 {
            let mut palette =
                world
                    .get_mut::<DefaultAvObjectPalette>(palette)
                    .ok_or(GameError::unclassified(
                        0x00480A45,
                        "00480A45: +0x7C isn't a palette",
                    ))?;
            palette.set_av_object(b"EditorMarker", None);
            palette.set_av_object(b"EditorMarker:0", None);
        }
        let parent = world
            .get::<crate::gamebryo::av_object::NiAvParent>(object)
            .and_then(|parent| parent.0)
            .ok_or(GameError::unclassified(
                0x00480A5D,
                "00480A5D: an EditorMarker with no parent",
            ))?;
        detach_child(world, parent, object);
        if state.root_b34404 == Some(object) {
            *state = EditorMarkerState::default();
        }
        return Ok(true);
    }
    let mut result = false;
    let children = world
        .get::<NiNodeChildren>(object)
        .ok_or(GameError::unclassified(
            0x00480A93,
            "00480A93: +0xB6 of a non-node",
        ))?
        .slots_ac
        .clone();
    for child in children.into_iter().flatten() {
        if !world
            .get::<NiClass>(child)
            .is_some_and(|class| class.is_node())
        {
            continue;
        }
        result = strip_editor_marker(world, child, state, show_editor_markers)?;
        if result {
            break;
        }
    }
    if state.root_b34404 == Some(object) {
        *state = EditorMarkerState::default();
    }
    Ok(result)
}

// IDA: N/A a property's +0x18 flags, of the two classes 00436880 reads it on.
fn zbuffer_flags(world: &World, property: Option<Entity>) -> Result<u16, GameError> {
    property
        .and_then(|property| world.get::<ZBufferProperty>(property))
        .map(|property| property.flags_18)
        .ok_or(GameError::unclassified(
            0x00436B90,
            "00436B90: a null g_defaultZBufferProperty",
        ))
}

// IDA: N/A no address of its own; its comments cite 00436BE5.
fn vertex_color_flags(world: &World, property: Option<Entity>) -> Result<u16, GameError> {
    property
        .and_then(|property| world.get::<VertexColorProperty>(property))
        .map(|property| property.flags_18)
        .ok_or(GameError::unclassified(
            0x00436BE5,
            "00436BE5: a null g_defaultVertexColorProperty",
        ))
}

// IDA: 00436880 Model_Init(model, path, stream, prepare), reached through 004390B0 (+0x08 = 0). +0x04 = 0, FormHeap's copy of the path. No top-level objects (+0x210): the "Model Load Error" warning and a reload of Marker_Error.NIF, a host stop. Object 0 with vt+8: named and matching "Bip[0-9][0-9]", a new NiNode with it at slot 0 (00436942); otherwise it's the root. Anything else: each top-level object with vt+8 or vt+0x0C goes into a new NiNode in turn, and none leaves +0x08 null and returns. 004809A0(root). "BASE " + path in 0x104 stack bytes, through 006FF420. 00B06B18 = 1. A ZBuffer on the root with the default's +0x18 flags is detached, and likewise a VertexColor; either warns with 00A369F8 (the stream's +0x08 text, nonempty) or 00A369B0. 00B06B18 restored; the prepare flag runs 007B8940(root, 1, 0, 1). Completion: 100% but the zero-object arm. The NiPointer counts on the root have no host form.
fn init_model(
    world: &mut World,
    path: &[u8],
    load: &BsStreamLoad,
    prepare: bool,
    services: &mut ModelBuildServices<'_>,
) -> Result<Model, GameError> {
    let mut model = Model {
        path_00: path.to_vec(),
        references_04: 0,
        root_08: None,
    };
    if load.root_count() == 0 {
        return Err(GameError::unclassified(
            0x00436918,
            "00436918: \"Model Load Error\" and the Marker_Error.NIF reload",
        ));
    }
    let is_node = |world: &World, object: Entity| {
        world
            .get::<NiClass>(object)
            .is_some_and(|class| class.is_node())
    };
    let first = load.root(0).filter(|&object| is_node(world, object));
    if let Some(first) = first {
        let name = world
            .get::<NiObjectNet>(first)
            .and_then(|net| net.name.clone());
        let bip = match &name {
            Some(name) => wildcard_match(name, b"Bip[0-9][0-9]")?,
            None => false,
        };
        if bip {
            let root = spawn_scene_node(world, SceneNodePayload::default());
            model.root_08 = Some(root);
            set_child_at(world, root, 0, Some(first));
        } else {
            model.root_08 = Some(first);
        }
    } else {
        let mut next = 0;
        for index in 0..load.root_count() {
            let Some(object) = load.root(index) else {
                continue;
            };
            let geometry = world.get::<NiClass>(object).is_some_and(|class| {
                matches!(
                    class,
                    NiClass::TriShape
                        | NiClass::TriStrips
                        | NiClass::ParticleSystem
                        | NiClass::ScreenElements
                )
            });
            if !is_node(world, object) && !geometry {
                continue;
            }
            let root = match model.root_08 {
                Some(root) => root,
                None => {
                    let root = spawn_scene_node(world, SceneNodePayload::default());
                    model.root_08 = Some(root);
                    root
                }
            };
            set_child_at(world, root, next, Some(object));
            next += 1;
        }
        if model.root_08.is_none() {
            return Ok(model);
        }
    }
    let root = model
        .root_08
        .ok_or(GameError::unclassified(0x00436B02, "00436B02: no root"))?;
    // 00436B06.
    let mut markers = EditorMarkerState::default();
    strip_editor_marker(
        world,
        root,
        &mut markers,
        services.show_editor_markers_b34390,
    )?;
    // 00436B0B..00436B66 "BASE " and the path, by `rep movs`, with no length check.
    let mut base = b"BASE ".to_vec();
    base.extend_from_slice(path);
    if base.len() >= MODEL_PATH_BUFFER {
        return Err(GameError::unclassified(
            0x00436B51,
            "00436B51: a model name past the 0x104 stack bytes",
        ));
    }
    world
        .get_mut::<NiObjectNet>(root)
        .ok_or(GameError::unclassified(
            0x00436B66,
            "00436B66: a root with no NiObjectNET",
        ))?
        .name = Some(base);
    // 00436B6B.
    let saved = *services.disable_warning_messages_b06b18;
    *services.disable_warning_messages_b06b18 = 1;
    let defaults = world.resource::<NiProcessDefaults>();
    let (default_zbuffer, default_vertex_color) =
        (defaults.zbuffer_b3f998, defaults.vertex_color_b3f980);
    let mut reexport = false;
    if let Some(zbuffer) = find_property_of_type(world, root, 9)?
        && world
            .get::<ZBufferProperty>(zbuffer)
            .ok_or(GameError::unclassified(
                0x00436B8C,
                "00436B8C: a type-9 property's +0x18",
            ))?
            .flags_18
            == zbuffer_flags(world, default_zbuffer)?
    {
        detach_property_of_type(world, root, 9)?;
        reexport = true;
    }
    if let Some(colors) = find_property_of_type(world, root, 7)?
        && world
            .get::<VertexColorProperty>(colors)
            .ok_or(GameError::unclassified(
                0x00436BE1,
                "00436BE1: a type-7 property's +0x18",
            ))?
            .flags_18
            == vertex_color_flags(world, default_vertex_color)?
    {
        detach_property_of_type(world, root, 7)?;
        reexport = true;
    }
    if reexport {
        let creator = load
            .stream
            .as_ref()
            .and_then(|stream| stream.header.metadata[0].split(|&byte| byte == 0).next())
            .unwrap_or_default();
        let mut text = Vec::new();
        if !creator.is_empty() {
            text.extend_from_slice(creator);
            text.extend_from_slice(b": ");
        }
        text.extend_from_slice(b"Reexport '");
        text.extend_from_slice(path);
        text.extend_from_slice(b"' to get rid of the ZBuffer and/or VertextColor property.");
        (services.warning)(text, *services.disable_warning_messages_b06b18 != 0);
    }
    *services.disable_warning_messages_b06b18 = saved;
    // 00436C7C.
    if prepare {
        prepare_object_geometry(world, Some(root), 1, 0, 1, &mut services.prepare)?;
    }
    Ok(model)
}

// IDA: 00438730 Model_AttachSpineRagdollConstraints(root): a null root skips. "Bip01 Spine2", "Bip01 Spine1", "Bip01 Spine" through
// vt+0x58; a found object whose +0xA8 casts to bhkBlendCollisionObject (00BA7A20, its RTTI: 0088EA80 is vt+4 of A9643C) gets a
// 0x4C bhkBlendCollisionObjectAddRotation (0088E7C0, 00BA7A1C + 1) through 0088E880: the old object's body (+0x10) moved to it
// by 00897670 (the hk steps are no-ops with no hk object, D2), its +0x18, +0x14 and +0x20 copied, vt+0x4C SetSceneObject with
// the old target, then 00435CE0 NiAVObject_SetCollisionObject on the node. Completion: 100%.
fn attach_spine_constraints(world: &mut World, root: Option<Entity>) -> Result<(), GameError> {
    use super::havok_stream::{
        BhkBlendCollisionAddRotation, BhkBlendCollisionObject, BhkSpCollisionObject,
        BlendCollisionStatics,
    };
    let Some(root) = root else {
        return Ok(());
    };
    for name in [b"Bip01 Spine2".as_slice(), b"Bip01 Spine1", b"Bip01 Spine"] {
        let Some(found) = object_by_name(world, root, name) else {
            continue;
        };
        let Some(old) = world
            .get::<crate::gamebryo::av_object::NiAvCollision>(found)
            .map(|collision| collision.0)
        else {
            continue;
        };
        if world.get::<NiClass>(old) != Some(&NiClass::BhkBlendCollisionObject) {
            continue;
        }
        world
            .get_resource_or_init::<BlendCollisionStatics>()
            .count_ba7a1c += 1;
        let old_sp = *world
            .get::<BhkSpCollisionObject>(old)
            .expect("a collision object");
        let old_blend = *world
            .get::<BhkBlendCollisionObject>(old)
            .expect("a bhkBlendCollisionObject");
        world
            .get_mut::<BhkSpCollisionObject>(old)
            .expect("a collision object")
            .body_10 = None;
        let new = world
            .spawn((
                NiClass::BhkBlendCollisionAddRotation,
                BhkSpCollisionObject {
                    target_08: old_sp.target_08,
                    flags_0c: BhkSpCollisionObject::default().flags_0c,
                    body_10: old_sp.body_10,
                },
                BhkBlendCollisionObject {
                    hierarchy_gain_14: old_blend.hierarchy_gain_14,
                    velocity_gain_18: old_blend.velocity_gain_18,
                    stored_world_20: old_blend.stored_world_20,
                    ..BhkBlendCollisionObject::default()
                },
                BhkBlendCollisionAddRotation::default(),
            ))
            .id();
        let index = world
            .get::<SceneNodePayload>(found)
            .map_or(NULL_REFERENCE, |payload| payload.collision_index);
        super::fade_node::set_collision_object(world, found, Some(new), index)?;
    }
    Ok(())
}

// IDA: 0043E760 the map's vt+0x0C(path, model, 0), with 0055F120: the lower-case key (no length check on its 0x3E8 stack bytes), the thread's interface, the bucket. 0043A260 walks to the first node at or past the key by `strcmp` (0043E2B0 `setge`). An equal key (0043E390) with a5 = 0 frees the new node and fails. Otherwise the node goes in, with a raw Model pointer (no count), and vt+0x30 004324C0 raises +0x18. Completion: 100% over the host map.
fn insert_model(
    loader: &mut ModelLoader,
    path: &[u8],
    model: ModelId,
    thread: std::thread::ThreadId,
) -> Result<bool, GameError> {
    let key = lower_model_key(path)?;
    loader.model_map_00.lock_14.register_thread(thread)?;
    let chain = &mut loader.models.buckets[model_map_bucket(&key) as usize];
    let position = chain
        .iter()
        .position(|entry| entry.key >= key)
        .unwrap_or(chain.len());
    if chain.get(position).is_some_and(|entry| entry.key == key) {
        return Ok(false);
    }
    chain.insert(position, ModelMapEntry { key, model });
    loader.model_map_00.field_18 += 1;
    Ok(true)
}

// IDA: 004395D0 QueuedModel_Finalize (vt+8): a null +0x28 goes to vt+0x28. a9 (+0x34 & 2): Model +0x04 up. +0x34 & 0x10 skips the rest. a7 (& 4) with +0x30: the BSFadeNode and fade distances (`fade_node::finalize_fade_root`), and the Model's +0x08 is the fade node after. 00438730 on the root; the map's vt+0x0C(path, model, 0); a failed insert looks the key up and swaps +0x28 through 00435AB0. vt+0x28 004374A0: flag 8 clear goes to 00436F30, whose null +0x18 does nothing. Completion: 100%.
fn finalize_queued_model(
    world: &mut World,
    id: IoTaskId,
    trace: &mut ModelTaskTrace,
    io_thread: std::thread::ThreadId,
) -> Result<(), GameError> {
    let (model, flags, lod, path, parent) = {
        let tasks = world.resource::<TaskGlobals>();
        let task = task_of(
            &tasks
                .io_manager_b33a10
                .as_ref()
                .ok_or(GameError::unclassified(
                    0x004395D0,
                    "004395D0: a null g_IOManager",
                ))?
                .tasks,
            id,
        )?;
        (
            task.model_28,
            task.flags_34,
            task.lod_30,
            task.path_20.clone(),
            task.parent_18,
        )
    };
    if let Some(model) = model {
        let root = {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let loader = tasks
                .model_loader_b33a1c
                .as_mut()
                .ok_or(GameError::unclassified(
                    0x004395EF,
                    "004395EF: a null g_modelLoader",
                ))?;
            let held = &mut loader.models.store[model.0];
            if flags & 2 != 0 {
                held.references_04 = held.references_04.wrapping_add(1);
            }
            held.root_08
        };
        if flags & 0x10 == 0 {
            let mut root = root;
            if flags & 4 != 0 && lod != 0 {
                let grids = world
                    .get_resource::<crate::tes::tes_grid::TesGridGlobals>()
                    .and_then(|globals| globals.grids_to_load_b06a2c)
                    .ok_or(GameError::unclassified(
                        0x0043966D,
                        "0043966D: the a7 arm before 0040EE7A publishes uGridsToLoad",
                    ))?;
                let settings = world
                    .get_resource::<super::fade_node::FadeNodeSettings>()
                    .copied()
                    .unwrap_or_default();
                root =
                    super::fade_node::finalize_fade_root(world, root, lod as u8, grids, settings)?;
                let mut tasks = world.resource_mut::<TaskGlobals>();
                let loader = tasks
                    .model_loader_b33a1c
                    .as_mut()
                    .ok_or(GameError::unclassified(
                        0x00434B7D,
                        "00434B7D: a null g_modelLoader",
                    ))?;
                loader.models.store[model.0].root_08 = root;
            }
            attach_spine_constraints(world, root)?;
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let loader = tasks
                .model_loader_b33a1c
                .as_mut()
                .ok_or(GameError::unclassified(
                    0x00439733,
                    "00439733: a null g_modelLoader",
                ))?;
            if insert_model(loader, &path, model, io_thread)? {
                trace.inserted = true;
            } else {
                let found = model_map_lookup_on(loader, &path, io_thread)?;
                assign_model(&mut tasks, id, found)?;
            }
        }
    }
    // 00439776 vt+0x28 004374A0.
    if flags & 8 != 0 {
        return Err(GameError::unclassified(
            0x004374AB,
            "004374AB: flag 8 on a finished task",
        ));
    }
    // 00436F30 QueuedFile_NotifyParent, after the finish's state 5 (`finish_queued_model`).
    let _ = parent;
    Ok(())
}

// IDA: 00434360 IOManager vt+0x44(task): +0x0C = 4, task vt+8, +0x0C = 5, vt+0x10 00433FD0(key) (the thread's interface, the bucket, 00433380's walk, the node's reference dropped, vt+0x34 00438A20 +0x18 down); a removal also drops +0x2C[vt+0x1C(key)]. A key gone returns with both untouched. Completion: 100% over the host map.
fn finish_queued_model(
    world: &mut World,
    id: IoTaskId,
    trace: &mut ModelTaskTrace,
    io_thread: std::thread::ThreadId,
) -> Result<(), GameError> {
    {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        task_of_mut(&mut io_manager_mut(&mut tasks)?.tasks, id)?.state_0c = 4;
    }
    finalize_queued_model(world, id, trace, io_thread)?;
    finish_io_task_tail(world, id, io_thread)
}

/// IDA: 00433CBB..00433D0A after a task's vt+8: +0x0C = 5, the task out of its bucket (00433FD0, the map's and +0x2C's counts
/// down), and with a parent its vt+0x28 (`QueuedFile_NotifyParent`). Completion: 100%.
/// # Errors
/// A key outside the map's buckets, or a stop in the parent's notification.
pub(crate) fn finish_io_task_tail(
    world: &mut World,
    id: IoTaskId,
    io_thread: std::thread::ThreadId,
) -> Result<(), GameError> {
    let mut tasks = world.resource_mut::<TaskGlobals>();
    let io = io_manager_mut(&mut tasks)?;
    let task = task_of_mut(&mut io.tasks, id)?;
    task.state_0c = 5;
    let key = task.key_10;
    let has_parent = task.parent_18.is_some();
    io.task_map.lock_14.register_thread(io_thread)?;
    let bucket = task_bucket(key);
    let chain = io
        .tasks
        .buckets
        .get_mut(bucket)
        .ok_or(GameError::unclassified(
            0x00433FD0,
            "00433FD0: a task key outside the map's buckets",
        ))?;
    if let Some(position) = chain.iter().position(|&other| other == id) {
        chain.remove(position);
        task_of_mut(&mut io.tasks, id)?.references_08 -= 1;
        io.task_map.field_18 -= 1;
        *io.array_2c.get_mut(bucket).ok_or(GameError::unclassified(
            0x004343A9,
            "004343A9: a task bucket outside +0x2C",
        ))? -= 1;
    }
    // 00439776 vt+0x28 004374A0 runs inside vt+8 in the original, with +0x0C at 4; `QueuedFile_NotifyParent` needs 4 or more, so the parent's notification is the same at 5.
    if has_parent {
        crate::tes::reference_load::queued_file_notify_parent(world, id)?;
    }
    Ok(())
}

/// IDA: N/A what 00434020 reaches outside the world.
pub struct IoDrainServices<'a> {
    /// 00433590 IOManager_RunTasksTimed: the host's `ModelDrain` pass stands for the IO thread and the timed run.
    pub run_timed_tasks: &'a mut dyn FnMut(&mut World) -> Result<(), GameError>,
    /// 0057B950 LoadingMenu_SetSectionProgressIfReady(2, percent), the host's menu gate.
    pub section_progress: &'a mut dyn FnMut(&mut World, i32, f32),
    /// 004523A0 LoadingMenu_SetSectionProgressDuringSaveLoad(2, percent), counted.
    pub save_load_progress: &'a mut dyn FnMut(i32, f32),
    /// Sleep(ms).
    pub sleep: &'a mut dyn FnMut(u32),
}

/// IDA: N/A what one 00434020 call did: `reprioritized` the tasks of 00433A40's walk, which a host record only; `host_gated` the tasks excluded from every count.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IoDrainResult {
    pub waited: bool,
    pub total: u32,
    pub loops: u32,
    pub sleeps: u32,
    pub reprioritized: u32,
    pub host_gated: u32,
    pub percents: Vec<f32>,
}

/// IDA: 004340C3..00434108 the percent: each count `fild`, signed, with `fadd` the float 2^32 when negative, which rounds; the total stored float first (004340D7); `fdiv` by that, rounded, then `fmul` the qword 100.0, `fstp` float. As `data_load::record_percent`, past its `ftol`. Completion: 100%.
#[must_use]
pub fn io_drain_percent(total: u32, remaining: u32) -> f32 {
    let exact = |value: u32| {
        let signed = value as i32;
        if signed < 0 {
            f64::from((f64::from(signed) + 4_294_967_296.0) as f32)
        } else {
            f64::from(signed)
        }
    };
    let divisor = exact(total) as f32;
    let quotient = (exact(total.wrapping_sub(remaining)) / f64::from(divisor)) as f32;
    (f64::from(quotient) * 100.0) as f32
}

// IDA: N/A the tasks the host's stream loader left at state 1, which it never builds: 00434020's counts leave them out (`QueuedModel::host_gate`, host substitution). The original builds them, and its count falls to 0.
fn gated_in_buckets(io: &IoManager, last: Option<usize>) -> u32 {
    let mut gated = 0;
    for (bucket, chain) in io.tasks.buckets.iter().enumerate() {
        if last.is_some_and(|last| bucket > last) {
            break;
        }
        for id in chain {
            if io.tasks.store[id.0]
                .as_ref()
                .is_some_and(|task| task.host_gate.is_some())
            {
                gated += 1;
            }
        }
    }
    gated
}

// IDA: 00434070..0043408D, 00434174..00434183: the sum of +0x2C[0..=3 * stage + 2], with `jbe` running past the 18 buckets in the original for a stage above 5, and the queue's and clone thread's: +0x34 vt+0x0C and 00B33A1C +0x18's +0x28 vt+0x0C, both 00438DB0's +0x18, and its +0x24.
fn stage_remaining(world: &World, last: usize) -> Result<u32, GameError> {
    let tasks = world.resource::<TaskGlobals>();
    let io = tasks
        .io_manager_b33a10
        .as_ref()
        .ok_or(GameError::unclassified(
            0x00434080,
            "00434080: a null g_IOManager",
        ))?;
    let mut sum = 0_u32;
    for index in 0..=last {
        sum = sum.wrapping_add(*io.array_2c.get(index).ok_or(GameError::unclassified(
            0x00434083,
            "00434083: +0x2C past the 18 buckets",
        ))?);
    }
    Ok(sum
        .wrapping_sub(gated_in_buckets(io, Some(last)))
        .wrapping_add(io.io_queue_34.field_18)
        .wrapping_add(clone_work(tasks)?))
}

// IDA: 00434040..00434055 the clone thread's +0x24 and its +0x28 queue's vt+0x0C.
fn clone_work(tasks: &TaskGlobals) -> Result<u32, GameError> {
    let clone = &tasks
        .model_loader_b33a1c
        .as_ref()
        .ok_or(GameError::unclassified(
            0x00434040,
            "00434040: a null g_ModelLoader",
        ))?
        .clone_thread_18;
    Ok(clone
        .field_24
        .wrapping_add(clone.reference_queue_28.field_18))
}

// IDA: N/A g_IOManager, read.
fn io_manager(world: &World) -> Result<&IoManager, GameError> {
    world
        .resource::<TaskGlobals>()
        .io_manager_b33a10
        .as_ref()
        .ok_or(GameError::unclassified(
            0x00434026,
            "00434026: a null g_IOManager",
        ))
}

// IDA: vt+0x38 00438DB0's +0x18, past the host's gated tasks.
fn manager_count(io: &IoManager) -> u32 {
    io.task_map
        .field_18
        .wrapping_sub(gated_in_buckets(io, None))
}

/// IDA: 00434020 IOManager_DrainTasksThroughStage(stage). vt+0x38, the queue and the clone thread's work all 0 goes to +0x38 = 6. Otherwise +0x38 = stage; 00433A40(stage, 0), a host record; the total, with its `jbe` over 3 * stage + 3 counters. While it's nonzero: the percent, 004523A0 under 0045A500 with +0x18 bit 11, else 0057B950, section 2; Sleep(50) when vt+0x38 is above the queue's count, unsigned; 00433590; the recount. +0x38 = 6. Completion: 100% of the control flow; the host's `run_timed_tasks` and `host_gate` are substitutions.
/// TODO: [ ] 00433A40's re-keying (vt+0x1C 00434C60 over the walk from bucket 3) and its pause, and 00433590's clock, when an IO thread runs. The queue 00438060 fills is the M6 3D's.
/// # Errors
/// A null singleton, a stage past the 18 buckets, or a stop in `run_timed_tasks`.
pub fn io_drain_tasks_through_stage(
    world: &mut World,
    stage: i32,
    services: &mut IoDrainServices<'_>,
) -> Result<IoDrainResult, GameError> {
    let mut result = IoDrainResult::default();
    {
        let tasks = world.resource::<TaskGlobals>();
        let io = io_manager(world)?;
        result.host_gated = gated_in_buckets(io, None);
        if manager_count(io) == 0 && io.io_queue_34.field_18 == 0 && clone_work(tasks)? == 0 {
            io_manager_mut(&mut world.resource_mut::<TaskGlobals>())?.field_38 = 6;
            return Ok(result);
        }
    }
    result.waited = true;
    {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let io = io_manager_mut(&mut tasks)?;
        io.field_38 = stage as u32;
        // 00433A73 `jle`: a stage above the second argument, 0, runs the walk: from bucket 3 (00433AD0) until 00433B39's first key byte 2 above the stage.
        result.reprioritized = if stage > 0 {
            io.tasks
                .buckets
                .iter()
                .skip(3)
                .flatten()
                .filter_map(|id| io.tasks.store[id.0].as_ref())
                .take_while(|task| i32::from((task.key_10 >> 16) as u8) <= stage)
                .count() as u32
        } else {
            0
        };
    }
    let last = stage.wrapping_mul(3).wrapping_add(2) as u32 as usize;
    let mut remaining = stage_remaining(world, last)?;
    result.total = remaining;
    while remaining != 0 {
        result.loops += 1;
        let percent = io_drain_percent(result.total, remaining);
        result.percents.push(percent);
        let flags = save_load_flags(world)?;
        if flags & 1 != 0 && flags & 0x800 != 0 {
            (services.save_load_progress)(2, percent);
        } else {
            (services.section_progress)(world, 2, percent);
        }
        let io = io_manager(world)?;
        if manager_count(io) > io.io_queue_34.field_18 {
            result.sleeps += 1;
            (services.sleep)(50);
        }
        (services.run_timed_tasks)(world)?;
        remaining = stage_remaining(world, last)?;
        result.host_gated = gated_in_buckets(io_manager(world)?, None);
    }
    io_manager_mut(&mut world.resource_mut::<TaskGlobals>())?.field_38 = 6;
    Ok(result)
}

// IDA: N/A the class names P15-RUN-001's records print.
const fn row_class_name(class: NiClass) -> &'static str {
    match class {
        NiClass::Node => "NiNode",
        NiClass::BillboardNode => "NiBillboardNode",
        NiClass::TriShape => "NiTriShape",
        NiClass::TriStrips => "NiTriStrips",
        NiClass::ParticleSystem => "NiParticleSystem",
        NiClass::AlphaProperty => "NiAlphaProperty",
        NiClass::ZBufferProperty => "NiZBufferProperty",
        NiClass::VertexColorProperty => "NiVertexColorProperty",
        NiClass::StencilProperty => "NiStencilProperty",
        NiClass::MaterialProperty => "NiMaterialProperty",
        NiClass::TexturingProperty => "NiTexturingProperty",
        NiClass::PpLightingProperty => "BSShaderPPLightingProperty",
        NiClass::VisController => "NiVisController",
        NiClass::ParticleEmitterController => "NiPSysEmitterCtlr",
        NiClass::ParticleModifierActiveController => "NiPSysModifierActiveCtlr",
        _ => "?",
    }
}

// IDA: N/A the +0x18 flags of a tree object.
fn row_flags(world: &World, object: Entity) -> u16 {
    if let Some(node) = world.get::<SceneNodePayload>(object) {
        return node.effective_av_flags;
    }
    if let Some(mesh) = world.get::<MeshTriShapePayload>(object) {
        return mesh.effective_av_flags;
    }
    world
        .get::<crate::gamebryo::particle_system::ParticleSystem>(object)
        .map_or(0, |system| system.effective_av_flags_18)
}

// IDA: N/A a helper, with no address of its own.
fn row_class(world: &World, object: Option<Entity>) -> String {
    object.map_or("-".to_owned(), |object| {
        world
            .get::<NiClass>(object)
            .map_or("?", |class| row_class_name(*class))
            .to_owned()
    })
}

/// IDA: N/A `gen_expected_13e.py`'s NODE line for one model, depth first over the child slots, walking the ordered slots and never a query: `NODE|key|path|class|name|flags|children|properties, head first|controller|+0xBC set|extras`. A name's '_' prints as ' ', as the run's records do. Every line is P15-RUN-001's form, so a test compares bytes.
#[must_use]
pub fn model_tree_rows(world: &World, key: &str, root: Entity) -> Vec<String> {
    // IDA: N/A a helper, with no address of its own.
    fn walk(world: &World, key: &str, object: Entity, path: &str, out: &mut Vec<String>) {
        let net = world.get::<NiObjectNet>(object);
        let name: String = net
            .and_then(|net| net.name.as_deref())
            .unwrap_or_default()
            .iter()
            .map(|&byte| if byte == b'_' { ' ' } else { char::from(byte) })
            .collect();
        let properties: Vec<String> = world
            .get::<crate::gamebryo::av_object::NiAvProperties>(object)
            .map(|list| {
                list.0
                    .iter()
                    .map(|&property| row_class(world, property))
                    .collect()
            })
            .unwrap_or_default();
        let slots = world
            .get::<NiNodeChildren>(object)
            .map(|children| children.slots_ac.clone())
            .unwrap_or_default();
        out.push(format!(
            "NODE|{key}|{path}|{}|{name}|{:X}|{}|{}|{}|{}|{}",
            row_class(world, Some(object)),
            row_flags(world, object),
            slots.len(),
            if properties.is_empty() {
                "-".to_owned()
            } else {
                properties.join(",")
            },
            row_class(world, net.and_then(|net| net.controller_0c)),
            u8::from(
                world
                    .get::<super::sky_shader::NiGeometryShader>(object)
                    .is_some()
            ),
            net.map_or(0, |net| net.extra_data.len()),
        ));
        for (index, child) in slots.iter().enumerate() {
            if let Some(child) = child {
                walk(world, key, *child, &format!("{path}.{index}"), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(world, key, root, "r", &mut out);
    out
}

/// IDA: N/A a model's objects with their paths in `model_tree_rows`' order, for a dump's raw words. Each with its live flags and its +0x30 local transform, from whichever component the class holds them in.
#[must_use]
pub fn model_tree_objects(world: &World, root: Entity) -> Vec<(String, Entity, u16, NiTransform)> {
    // IDA: N/A a helper, with no address of its own.
    fn walk(
        world: &World,
        object: Entity,
        path: String,
        out: &mut Vec<(String, Entity, u16, NiTransform)>,
    ) {
        let local = if let Some(node) = world.get::<SceneNodePayload>(object) {
            node.local_transform
        } else if let Some(mesh) = world.get::<MeshTriShapePayload>(object) {
            mesh.local_transform
        } else {
            world
                .get::<crate::gamebryo::particle_system::ParticleSystem>(object)
                .map_or_else(NiTransform::default, |system| system.local_transform_30)
        };
        out.push((path.clone(), object, row_flags(world, object), local));
        let slots = world
            .get::<NiNodeChildren>(object)
            .map(|children| children.slots_ac.clone())
            .unwrap_or_default();
        for (index, child) in slots.iter().enumerate() {
            if let Some(child) = child {
                walk(world, *child, format!("{path}.{index}"), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(world, root, "r".to_owned(), &mut out);
    out
}

/// IDA: 00439EB0 ModelLoader_LoadModelNow(g_modelLoader, path, 0, 0, 1) at a gameplay call site, its answer dropped, through
/// `SynchronousModelLoad`. Completion: 100% of the seam.
/// # Errors
/// No model services (a stop at `site`), or the load's.
pub fn load_model_now_dropped(world: &mut World, path: &[u8], site: u32) -> Result<(), GameError> {
    let load = world
        .get_resource::<crate::bs::model_loader::SynchronousModelLoad>()
        .ok_or_else(|| GameError::stop(site, "a gameplay model load needs model services"))?
        .0;
    load(world, path, false, 0, true)?;
    Ok(())
}

// IDA: N/A a task in the IO store.
pub(crate) fn task_mut(
    tasks: &mut TaskGlobals,
    id: IoTaskId,
) -> Result<&mut QueuedModel, GameError> {
    tasks
        .io_manager_b33a10
        .as_mut()
        .and_then(|io| io.tasks.store.get_mut(id.0))
        .and_then(Option::as_mut)
        .ok_or(GameError::unclassified(0, "a released QueuedHelmet"))
}

/// IDA: 00436E70 QueuedFile_OnChildFinished (vt+0x24) on a QueuedHelmet, from its child's QueuedFile_NotifyParent (00436F30,
/// which then drops its reference to the helmet): the finished count up, then vt+0x28 (`check_finished`). That runs only
/// when its first test passes (state 4 or more, every child finished, not 6); its notification of the helmet's own parent
/// has the same test, so anything else is a no-op. A passing one is queued for the walk's step (`pending_checks`), which has
/// the services the clone needs (D9: the original runs it on the IO thread at once). Completion: 100%.
/// # Errors
/// A released helmet.
pub fn helmet_child_finished(world: &mut World, id: IoTaskId) -> Result<(), GameError> {
    let ready = {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let task = task_mut(&mut tasks, id)?;
        task.children_1c -= 1;
        task.references_08 -= 1;
        task.state_0c >= 4 && task.children_1c == 0 && task.state_0c != 6
    };
    if ready {
        world
            .get_resource_or_init::<HelmetTasks>()
            .pending_checks
            .push(id);
    }
    Ok(())
}

/// IDA: 0043BEB0 ModelLoader_PurgeUnusedModels: a model with a reference count of 0 leaves the map. Its tree stays with the World, where the original frees it. The second loop, over the KF map, has nothing to do: the port holds no KF model, and the map is empty in the run. Returns how many went. Completion: 100% of the model map.
#[must_use]
pub fn purge_unused_models(models: &mut ModelMap) -> usize {
    let store = &models.store;
    let mut removed = 0;
    for bucket in &mut models.buckets {
        let before = bucket.len();
        bucket.retain(|entry| store[entry.model.0].references_04 & 0xFFFF != 0);
        removed += before - bucket.len();
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamebryo::object_net::NiExtraDataName;

    fn task_globals() -> TaskGlobals {
        let mut globals = TaskGlobals::default();
        globals.construct_io_manager().unwrap();
        globals.construct_model_loader().unwrap();
        globals
    }

    const REQUEST: ModelRequest = ModelRequest {
        priority: 5,
        parent: None,
        lod: 0,
        a7: false,
        a8: true,
        a9: true,
    };

    // 0043E731 `movsx`: a byte above 0x7F enters signed.
    #[test]
    fn the_model_map_hash_is_signed_and_case_insensitive() {
        assert_eq!(
            model_map_bucket(b"Effects\\B.NIF"),
            model_map_bucket(b"effects\\b.nif")
        );
        let by_hand = ((u32::from(b'a').wrapping_mul(33)).wrapping_add(0xFFFF_FFC8)) % 1009;
        assert_eq!(model_map_bucket(b"a\xC8"), by_hand);
    }

    // 00434710's three arms: a length of exactly 3 skips the "spt" test.
    #[test]
    fn the_path_fixup_has_three_arms() {
        assert_eq!(fixup_model_path(b"x.SPT"), b"Treesx.SPT");
        assert_eq!(fixup_model_path(b"spt"), b"Meshes\\spt");
        assert_eq!(fixup_model_path(b"meshes\\x.nif"), b"meshes\\x.nif");
        assert_eq!(
            fixup_model_path(b"Effects\\x.nif"),
            b"Meshes\\Effects\\x.nif"
        );
    }

    // 00436560 is signed in its high byte and wraps.
    #[test]
    fn the_task_key_places_each_part() {
        assert_eq!(make_task_key(1, 0x27E5_0757, 5, 1), 0x0127_E507_5705_0001);
        assert_eq!(make_task_key(-1, 0, 0, 0), 0xFF00_0000_0000_0000);
    }

    // No archive: 004365F0's key, with 00B33A18 in it and bucket 5 * 3 + 0.
    #[test]
    fn a_model_with_no_archive_entry_takes_the_second_counter() {
        let mut tasks = task_globals();
        let out = load_model(
            &mut tasks,
            None,
            b"Effects\\b.nif",
            ModelRequest {
                a7: true,
                a8: false,
                a9: false,
                ..REQUEST
            },
        )
        .unwrap();
        let io = tasks.io_manager_b33a10.as_ref().unwrap();
        let task = io.tasks.store[out.unwrap().0].as_ref().unwrap();
        assert_eq!(task.flags_34, 4);
        assert_eq!(task.key_10, (((0x2_0000_0001_u64 << 8) + 5) << 16) + 1);
        assert_eq!(task.references_08, 2, "out and the map");
        assert_eq!(task.state_0c, 1);
        assert_eq!(io.array_2c[task_bucket(task.key_10)], 1);
        assert_eq!(tasks.counters.unpacked_sequence_b33a18, 1);
        tasks.stop_worker_threads();
    }

    // A hit: a9's reference and a null out; no a9, no reference.
    #[test]
    fn a_cached_model_takes_a_reference_and_gives_a_null_out() {
        let mut tasks = task_globals();
        let loader = tasks.model_loader_b33a1c.as_mut().unwrap();
        loader.models.store.push(Model::default());
        let key = b"meshes\\a.nif".to_vec();
        let bucket = model_map_bucket(&key) as usize;
        loader.models.buckets[bucket].push(ModelMapEntry {
            key,
            model: ModelId(0),
        });
        let out = load_model(&mut tasks, None, b"Meshes\\A.NIF", REQUEST).unwrap();
        assert_eq!(out, None);
        let references = |tasks: &TaskGlobals| {
            tasks.model_loader_b33a1c.as_ref().unwrap().models.store[0].references_04
        };
        assert_eq!(references(&tasks), 1);
        let out = load_model(
            &mut tasks,
            None,
            b"Meshes\\A.NIF",
            ModelRequest {
                a9: false,
                ..REQUEST
            },
        )
        .unwrap();
        assert_eq!(out, None);
        assert_eq!(references(&tasks), 1);
        assert_eq!(tasks.counters.sequence_b33a14, 0, "nothing queued");
        tasks.stop_worker_threads();
    }

    // 0043AC40: a parent the loader holds no reference task for stops rather than guess.
    #[test]
    fn a_missing_parent_task_is_a_host_stop() {
        let mut tasks = task_globals();
        let request = ModelRequest {
            parent: Some(super::super::reference_tasks::ReferenceTaskId(0).into()),
            ..REQUEST
        };
        assert!(load_model(&mut tasks, None, b"x.nif", request).is_err());
        tasks.stop_worker_threads();
    }

    // 004320F0: a key already in the map fails, and the task retries with a new sequence in the low word. Its bucket counter moves only on the success.
    #[test]
    fn an_equal_key_retries_with_a_new_sequence() {
        let mut tasks = task_globals();
        let io = tasks.io_manager_b33a10.as_mut().unwrap();
        let key = (((0x2_0000_0001_u64 << 8) + 5) << 16) + 1;
        let bucket = task_bucket(key);
        io.tasks.store.push(Some(QueuedModel {
            original_vtable: QUEUED_MODEL_VTABLE,
            references_08: 0,
            state_0c: 1,
            key_10: key,
            parent_18: None,
            children_1c: 0,
            path_20: Vec::new(),
            entry_24: None,
            model_28: None,
            field_2c: 0,
            lod_30: 0,
            flags_34: 0,
            host_gate: None,
        }));
        io.tasks.buckets[bucket].push(IoTaskId(0));
        let out = load_model(&mut tasks, None, b"x.nif", REQUEST)
            .unwrap()
            .unwrap();
        let io = tasks.io_manager_b33a10.as_ref().unwrap();
        let task = io.tasks.store[out.0].as_ref().unwrap();
        assert_eq!(task.key_10, key + 1);
        assert_eq!(
            task.references_08, 2,
            "the failed try's reference went back"
        );
        assert_eq!(io.array_2c[bucket], 1);
        assert_eq!(io.tasks.buckets[bucket], [IoTaskId(0), out], "ascending");
        assert_eq!(tasks.counters.sequence_b33a14, 2);
        tasks.stop_worker_threads();
    }

    fn drain_world() -> World {
        let mut world = World::new();
        world.insert_resource(NiProcessDefaults::default());
        world.insert_resource(task_globals());
        world
    }

    fn queue(world: &mut World, key: u64, flags: u8) -> IoTaskId {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let io = tasks.io_manager_b33a10.as_mut().unwrap();
        let id = IoTaskId(io.tasks.store.len());
        io.tasks.store.push(Some(QueuedModel {
            original_vtable: QUEUED_MODEL_VTABLE,
            references_08: 1,
            state_0c: 1,
            key_10: key,
            parent_18: None,
            children_1c: 0,
            path_20: format!("x{key:X}.nif").into_bytes(),
            entry_24: None,
            model_28: None,
            field_2c: 0,
            lod_30: 0,
            flags_34: flags,
            host_gate: None,
        }));
        let bucket = task_bucket(key);
        let store = &io.tasks.store;
        let position = io.tasks.buckets[bucket]
            .partition_point(|other| (store[other.0].as_ref().unwrap().key_10 as i64) < key as i64);
        io.tasks.buckets[bucket].insert(position, id);
        io.task_map.field_18 += 1;
        io.array_2c[bucket] += 1;
        tasks.counters.live_tasks_b33a20 += 1;
        id
    }

    fn fail_open(_: &str, _: i32, _: u32) -> Option<Box<dyn GameFile>> {
        None
    }

    fn with_services<R>(run: impl FnOnce(&mut ModelBuildServices<'_>) -> R) -> R {
        let registry = crate::gamebryo::registration::NiFactoryRegistry::default();
        let mut open = fail_open;
        let mut get = |_: u32| None;
        let mut disabled = 0_u8;
        let mut warning = |_: Vec<u8>, _: bool| {};
        let mut services = ModelBuildServices {
            archives: None,
            stream: BsStreamServices {
                open: &mut open,
                registry: &registry,
                draw_rand: None,
                draw_on_io_thread: false,
                source_textures: None,
                post_load: crate::gamebryo::stream_postload::StreamPostLoadRuntime::default(),
            },
            prepare: PrepareGeometryContext {
                render_path: 5,
                hdr: true,
                reference_classifier: 0,
                property_defaults: crate::gamebryo::scene_state::ScenePropertyDefaults::default(),
                get_shader: &mut get,
                warnings: Vec::new(),
                sky_shader_trace: Vec::new(),
                water_shader_trace: Vec::new(),
                lava_b47844: 0,
                shaders_30_b42ea5: false,
                load_normal_map: None,
                unported_shader_setup: None,
                classify_trace: Vec::new(),
            },
            disable_warning_messages_b06b18: &mut disabled,
            show_editor_markers_b34390: 0,
            warning: &mut warning,
            main_random_next: None,
        };
        run(&mut services)
    }

    // A key in bucket 0 and two in bucket 2. 00433760 with "restart": the empty buckets pass inside one call. A pass that ran something returns false at each bucket's end and resets to bucket 0, so the walk stays on the low keys first. With no reader each task's load gives back no model (00439587), so each runs through Finalize and leaves the map.
    #[test]
    fn the_walk_visits_keys_in_order_and_restarts() {
        let mut world = drain_world();
        // Bucket = byte 2 * 3 + byte 7.
        let high = queue(&mut world, 0x0000_0000_0002_0001, 3);
        let low = queue(&mut world, 0x0000_0000_0000_0001, 3);
        let middle = queue(&mut world, 0x0000_0000_0002_0000, 3);
        assert_eq!(task_bucket(0x0000_0000_0002_0001), 6);
        with_services(|services| {
            let mut drain = ModelDrain::new();
            assert_eq!(
                drain.step(&mut world, services).unwrap(),
                ModelDrainStep::Ran
            );
            let keys = |drain: &ModelDrain| {
                drain
                    .record
                    .tasks
                    .iter()
                    .map(|task| task.key)
                    .collect::<Vec<u64>>()
            };
            assert_eq!(keys(&drain), [1], "the lowest bucket, in the first call");
            // The bucket's end: the pass ran something, so false, with the bucket advanced.
            assert_eq!(
                drain.step(&mut world, services).unwrap(),
                ModelDrainStep::Restarted
            );
            assert_eq!(
                drain.step(&mut world, services).unwrap(),
                ModelDrainStep::Ran
            );
            assert_eq!(
                drain.step(&mut world, services).unwrap(),
                ModelDrainStep::Ran
            );
            assert_eq!(
                keys(&drain),
                [1, 0x2_0000, 0x2_0001],
                "ascending in a bucket"
            );
            assert_eq!(
                drain.step(&mut world, services).unwrap(),
                ModelDrainStep::Restarted
            );
            assert_eq!(
                drain.step(&mut world, services).unwrap(),
                ModelDrainStep::Ended
            );
            assert_eq!(
                drain.step(&mut world, services).unwrap(),
                ModelDrainStep::Ended
            );
            assert_eq!(drain.record.run, 3);
            assert_eq!(drain.record.load_failed, 3, "the null reader's 006F99A7");
        });
        let tasks = world.resource::<TaskGlobals>();
        let io = tasks.io_manager_b33a10.as_ref().unwrap();
        assert!(io.tasks.buckets.iter().all(Vec::is_empty));
        assert_eq!(io.task_map.field_18, 0);
        assert_eq!(io.array_2c[6], 0);
        assert_eq!(tasks.counters.live_tasks_b33a20, 0, "004393B0 at zero");
        for id in [high, low, middle] {
            assert!(io.tasks.store[id.0].is_none());
        }
        // 00433C43 skips the decrement on a token taken at once, and 00433D1F counts up on every pass: 6 passes, then the ended walk stops counting.
        assert_eq!(
            io.threads_28[0]
                .state()
                .counter_18
                .load(std::sync::atomic::Ordering::Acquire),
            1 + 6
        );
    }

    // 00433CB2: a task past state 1 stays, and with nothing run the walk ends rather than restarting.
    #[test]
    fn a_task_past_state_one_is_skipped() {
        let mut world = drain_world();
        let id = queue(&mut world, 1, 3);
        {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let io = tasks.io_manager_b33a10.as_mut().unwrap();
            io.tasks.store[id.0].as_mut().unwrap().state_0c = 3;
        }
        with_services(|services| {
            let mut drain = ModelDrain::new();
            assert_eq!(
                drain.step(&mut world, services).unwrap(),
                ModelDrainStep::Skipped
            );
            assert_eq!(
                drain.run_to_end(&mut world, services).unwrap(),
                ModelDrainStep::Ended
            );
            assert_eq!(drain.record.run, 0);
        });
        let tasks = world.resource::<TaskGlobals>();
        let task = tasks.io_manager_b33a10.as_ref().unwrap().tasks.store[id.0].as_ref();
        assert_eq!(
            task.unwrap().references_08,
            1,
            "the iterator's reference went back"
        );
    }

    // 00433C37 with the token held: the host returns where the thread blocks. Once free, 00433C4B's decrement and 00433C71's iterator reset run, leaving "ran" alone, and the pass goes on.
    #[test]
    fn a_pause_holds_the_walk_at_its_token() {
        let mut world = drain_world();
        queue(&mut world, 1, 3);
        world.resource_mut::<TaskGlobals>().pause_workers().unwrap();
        let tokens = |world: &World| {
            world
                .resource::<TaskGlobals>()
                .io_manager_b33a10
                .as_ref()
                .unwrap()
                .threads_28[0]
                .state()
                .counter_18
                .load(std::sync::atomic::Ordering::Acquire)
        };
        with_services(|services| {
            let mut drain = ModelDrain::new();
            assert_eq!(
                drain.step(&mut world, services).unwrap(),
                ModelDrainStep::WaitingForToken
            );
            assert_eq!(drain.record.run, 0);
            assert_eq!(tokens(&world), 0);
            world
                .resource_mut::<TaskGlobals>()
                .resume_workers()
                .unwrap();
            assert_eq!(tokens(&world), 1);
            assert_eq!(
                drain.step(&mut world, services).unwrap(),
                ModelDrainStep::Ran
            );
            // Down one for the wait, up one at 00433D1F.
            assert_eq!(tokens(&world), 1);
            assert_eq!(drain.record.run, 1);
        });
    }

    // 00439492 a hit in the map: flag 0x10, the a9 reference, no insert, and the +0x28 reference goes at 004393BE.
    #[test]
    fn a_task_for_a_cached_model_takes_its_a9_reference_and_goes() {
        let mut world = drain_world();
        let id = queue(&mut world, 1, 3);
        let path = {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let path = tasks.io_manager_b33a10.as_ref().unwrap().tasks.store[id.0]
                .as_ref()
                .unwrap()
                .path_20
                .clone();
            let loader = tasks.model_loader_b33a1c.as_mut().unwrap();
            loader.models.store.push(Model {
                path_00: path.clone(),
                references_04: 1,
                root_08: None,
            });
            insert_model(loader, &path, ModelId(0), std::thread::current().id()).unwrap();
            path
        };
        with_services(|services| {
            let mut drain = ModelDrain::new();
            assert_eq!(
                drain.step(&mut world, services).unwrap(),
                ModelDrainStep::Ran
            );
            assert_eq!((drain.record.cached, drain.record.built), (1, 0));
            assert!(drain.record.tasks[0].cached && !drain.record.tasks[0].inserted);
        });
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let loader = tasks.model_loader_b33a1c.as_mut().unwrap();
        assert_eq!(loader.models.store[0].references_04, 2, "+1, +1, -1");
        assert_eq!(loader.model_map_00.field_18, 1);
        // 00439737: a second insert of the key fails and changes nothing.
        loader.models.store.push(Model::default());
        assert!(!insert_model(loader, &path, ModelId(1), std::thread::current().id()).unwrap());
        assert_eq!(loader.model_map_00.field_18, 1);
    }

    // 0055F120 sorts each chain by `strcmp`, and 0043E760 lowers the key.
    #[test]
    fn the_model_map_inserts_in_strcmp_order() {
        let mut tasks = task_globals();
        let loader = tasks.model_loader_b33a1c.as_mut().unwrap();
        let thread = std::thread::current().id();
        // Two keys of one bucket: h = c + 33 * h, so "bB" and "aa" (97*33 + 97 = 3298 and 98*33 + 66 = 3300) differ; find two by search.
        let keys: Vec<Vec<u8>> = (0..4000_u32)
            .map(|n| format!("k{n}").into_bytes())
            .collect();
        let first = keys[0].clone();
        let bucket = model_map_bucket(&first);
        let others: Vec<&Vec<u8>> = keys
            .iter()
            .filter(|key| model_map_bucket(key) == bucket)
            .collect();
        assert!(others.len() >= 3);
        for (index, key) in others.iter().rev().take(3).enumerate() {
            loader.models.store.push(Model::default());
            assert!(insert_model(loader, key, ModelId(index), thread).unwrap());
        }
        let chain = &loader.models.buckets[bucket as usize];
        assert!(chain.windows(2).all(|pair| pair[0].key < pair[1].key));
        assert_eq!(loader.model_map_00.field_18, 3);
        tasks.stop_worker_threads();
    }

    fn named_node(world: &mut World, name: &[u8]) -> Entity {
        let node = spawn_scene_node(world, SceneNodePayload::default());
        world.get_mut::<NiObjectNet>(node).unwrap().name = Some(name.to_vec());
        node
    }

    fn with_bsx(world: &mut World, node: Entity, flags: u32) {
        let bsx = world
            .spawn((
                NiExtraDataName(Some(b"BSX".to_vec())),
                BsxFlagsExtraData { flags_0c: flags },
            ))
            .id();
        world
            .get_mut::<NiObjectNet>(node)
            .unwrap()
            .extra_data
            .push(bsx);
    }

    // 004809A0: nothing without a BSX bit 5; a marker's slot goes null, and the last slot's size drops; 00B34390 hides instead of detaching.
    #[test]
    fn the_editor_marker_strip_keeps_its_arms() {
        let mut world = World::new();
        let root = named_node(&mut world, b"root");
        let marker = named_node(&mut world, b"editormarker");
        let after = named_node(&mut world, b"after");
        set_child_at(&mut world, root, 0, Some(marker));
        set_child_at(&mut world, root, 1, Some(after));
        let mut state = EditorMarkerState::default();
        assert!(!strip_editor_marker(&mut world, root, &mut state, 0).unwrap());
        assert_eq!(state, EditorMarkerState::default());
        assert_eq!(world.get::<NiNodeChildren>(root).unwrap().slots_ac.len(), 2);
        with_bsx(&mut world, root, 0x1F);
        assert!(
            !strip_editor_marker(&mut world, root, &mut state, 0).unwrap(),
            "bit 5 clear"
        );
        world
            .get_mut::<NiObjectNet>(root)
            .unwrap()
            .extra_data
            .clear();
        with_bsx(&mut world, root, 0x20);
        // Hidden with 00B34390 set, leaving the slot; the root's frame clears the state.
        assert!(strip_editor_marker(&mut world, root, &mut state, 1).unwrap());
        assert_eq!(state, EditorMarkerState::default());
        assert_eq!(
            world
                .get::<SceneNodePayload>(marker)
                .unwrap()
                .effective_av_flags
                & 1,
            1
        );
        // Detached: an inner slot stays as a null, and a parent's +0x1C clears.
        assert!(strip_editor_marker(&mut world, root, &mut state, 0).unwrap());
        assert_eq!(
            world.get::<NiNodeChildren>(root).unwrap().slots_ac,
            [None, Some(after)]
        );
        assert_eq!(
            world
                .get::<crate::gamebryo::av_object::NiAvParent>(marker)
                .unwrap()
                .0,
            None
        );
        // The last slot drops the size by one, and only one.
        let last = named_node(&mut world, b"EditorMarker");
        set_child_at(&mut world, root, 1, Some(last));
        assert_eq!(
            world
                .get::<crate::gamebryo::av_object::NiAvParent>(after)
                .unwrap()
                .0,
            None,
            "0070B342 the old child"
        );
        assert!(strip_editor_marker(&mut world, root, &mut state, 0).unwrap());
        assert_eq!(world.get::<NiNodeChildren>(root).unwrap().slots_ac, [None]);
        // No BSX in a non-root frame with a root set still walks by name, and a root named "EditorMarker" has no parent to detach from.
        let lone = named_node(&mut world, b"EditorMarker");
        with_bsx(&mut world, lone, 0x20);
        let mut state = EditorMarkerState::default();
        assert!(strip_editor_marker(&mut world, lone, &mut state, 0).is_err());
    }

    // 004809F2 the NiControllerManager's +0x7C, and 00480A45's two vt+0x50 removals.
    #[test]
    fn the_editor_marker_strip_clears_the_palette() {
        use crate::gamebryo::controller_sequence::ControllerManager;
        let mut world = World::new();
        let root = named_node(&mut world, b"root");
        let marker = named_node(&mut world, b"EditorMarker");
        set_child_at(&mut world, root, 0, Some(marker));
        with_bsx(&mut world, root, 0x20);
        let mut palette = DefaultAvObjectPalette::default();
        palette.set_av_object(b"EditorMarker", Some(marker));
        palette.set_av_object(b"EditorMarker:0", Some(marker));
        palette.set_av_object(b"other", Some(root));
        let palette = world.spawn(palette).id();
        let manager = world
            .spawn((
                NiClass::ControllerManager,
                ControllerManager {
                    object_palette_7c: Some(palette),
                    ..ControllerManager::default()
                },
            ))
            .id();
        world.get_mut::<NiObjectNet>(root).unwrap().controller_0c = Some(manager);
        let mut state = EditorMarkerState::default();
        assert!(strip_editor_marker(&mut world, root, &mut state, 0).unwrap());
        let held = world.get::<DefaultAvObjectPalette>(palette).unwrap();
        assert_eq!(held.get_av_object(b"EditorMarker"), None);
        assert_eq!(held.get_av_object(b"EditorMarker:0"), None);
        assert_eq!(held.get_av_object(b"other"), Some(root));
    }

    // 006FF9C0's binary search over the sorted array, and a null name's stop.
    #[test]
    fn the_extra_data_search_is_a_binary_one() {
        let mut world = World::new();
        let node = named_node(&mut world, b"n");
        let names: [&[u8]; 3] = [b"A", b"BSX", b"Z"];
        let mut ids = Vec::new();
        for name in names {
            let extra = world.spawn(NiExtraDataName(Some(name.to_vec()))).id();
            ids.push(extra);
            world
                .get_mut::<NiObjectNet>(node)
                .unwrap()
                .extra_data
                .push(extra);
        }
        assert_eq!(find_extra_data(&world, node, b"BSX").unwrap(), Some(ids[1]));
        assert_eq!(find_extra_data(&world, node, b"Z").unwrap(), Some(ids[2]));
        assert_eq!(find_extra_data(&world, node, b"B").unwrap(), None);
        let null = world.spawn(NiExtraDataName(None)).id();
        world.get_mut::<NiObjectNet>(node).unwrap().extra_data[1] = null;
        assert!(find_extra_data(&world, node, b"BSX").is_err());
    }

    fn loaded_with(root: Entity) -> BsStreamLoad {
        use crate::gamebryo::stream_file_pipeline::StreamFilePipelineSuccess;
        let mut success = StreamFilePipelineSuccess {
            header: Default::default(),
            graph: Default::default(),
            bytes_consumed: 0,
        };
        success.graph.roots.owners = vec![Some(root)];
        success.header.metadata[0] = b"Creator\0x".to_vec();
        BsStreamLoad {
            status: BsStreamLoadStatus::Loaded,
            path: String::new(),
            stream: Some(success),
            error: None,
        }
    }

    // 00436880's root choice, the two detach and warn arms, 00436C70's prepare gate, and the zero-object stop.
    #[test]
    fn model_init_keeps_its_arms() {
        use crate::gamebryo::properties::{VertexColorProperty, ZBufferProperty};
        let mut world = World::new();
        let (zbuffer_default, colors_default) = (
            world.spawn(ZBufferProperty { flags_18: 0xF }).id(),
            world.spawn(VertexColorProperty { flags_18: 8 }).id(),
        );
        world.insert_resource(NiProcessDefaults {
            zbuffer_b3f998: Some(zbuffer_default),
            vertex_color_b3f980: Some(colors_default),
            ..NiProcessDefaults::default()
        });
        let registry = crate::gamebryo::registration::NiFactoryRegistry::default();
        let mut open = fail_open;
        let mut get = |_: u32| None;
        let mut disabled = 7_u8;
        let mut warnings: Vec<(Vec<u8>, bool)> = Vec::new();
        let mut warning = |text: Vec<u8>, flag: bool| warnings.push((text, flag));
        let mut services = ModelBuildServices {
            archives: None,
            stream: BsStreamServices {
                open: &mut open,
                registry: &registry,
                draw_rand: None,
                draw_on_io_thread: false,
                source_textures: None,
                post_load: crate::gamebryo::stream_postload::StreamPostLoadRuntime::default(),
            },
            prepare: PrepareGeometryContext {
                render_path: 0,
                hdr: true,
                reference_classifier: 0,
                property_defaults: crate::gamebryo::scene_state::ScenePropertyDefaults::default(),
                get_shader: &mut get,
                warnings: Vec::new(),
                sky_shader_trace: Vec::new(),
                water_shader_trace: Vec::new(),
                lava_b47844: 0,
                shaders_30_b42ea5: false,
                load_normal_map: None,
                unported_shader_setup: None,
                classify_trace: Vec::new(),
            },
            disable_warning_messages_b06b18: &mut disabled,
            show_editor_markers_b34390: 0,
            warning: &mut warning,
            main_random_next: None,
        };
        // A plain NiNode is the root and takes "BASE <path>".
        let plain = named_node(&mut world, b"Scene");
        let model = init_model(
            &mut world,
            b"A\\b.nif",
            &loaded_with(plain),
            false,
            &mut services,
        )
        .unwrap();
        assert_eq!(model.root_08, Some(plain));
        assert_eq!(model.path_00, b"A\\b.nif");
        assert_eq!(model.references_04, 0);
        assert_eq!(
            world.get::<NiObjectNet>(plain).unwrap().name.as_deref(),
            Some(&b"BASE A\\b.nif"[..])
        );
        // "Bip01" goes under a new NiNode at slot 0.
        let bip = named_node(&mut world, b"Bip01");
        let model = init_model(
            &mut world,
            b"p.nif",
            &loaded_with(bip),
            false,
            &mut services,
        )
        .unwrap();
        let root = model.root_08.unwrap();
        assert_ne!(root, bip);
        assert_eq!(
            world.get::<NiNodeChildren>(root).unwrap().slots_ac,
            [Some(bip)]
        );
        assert_eq!(
            world
                .get::<crate::gamebryo::av_object::NiAvParent>(bip)
                .unwrap()
                .0,
            Some(root)
        );
        assert_eq!(
            world.get::<NiObjectNet>(bip).unwrap().name.as_deref(),
            Some(&b"Bip01"[..])
        );
        // A geometry first: a new NiNode holds each top-level node or geometry in turn; anything else is skipped.
        let shape = crate::gamebryo::mesh_geometry::spawn_mesh_geometry(
            &mut world,
            NiClass::TriShape,
            MeshTriShapePayload::default(),
        );
        let other = world.spawn(NiClass::AlphaProperty).id();
        let mut load = loaded_with(shape);
        load.stream.as_mut().unwrap().graph.roots.owners =
            vec![Some(shape), None, Some(other), Some(plain)];
        let model = init_model(&mut world, b"g.nif", &load, false, &mut services).unwrap();
        let root = model.root_08.unwrap();
        assert_eq!(
            world.get::<NiNodeChildren>(root).unwrap().slots_ac,
            [Some(shape), Some(plain)]
        );
        // Nothing usable leaves the root null, and nothing past it runs.
        let mut load = loaded_with(other);
        load.stream.as_mut().unwrap().graph.roots.owners = vec![Some(other)];
        let model = init_model(&mut world, b"n.nif", &load, false, &mut services).unwrap();
        assert_eq!(model.root_08, None);
        // 00436918: no top-level objects.
        let mut load = loaded_with(other);
        load.stream.as_mut().unwrap().graph.roots.owners.clear();
        assert!(init_model(&mut world, b"z.nif", &load, false, &mut services).is_err());
        // The defaults' flags detach, with the warning under the flag; other flags stay.
        let root = named_node(&mut world, b"r");
        let (same_z, same_c) = (
            world
                .spawn((NiClass::ZBufferProperty, ZBufferProperty { flags_18: 0xF }))
                .id(),
            world
                .spawn((
                    NiClass::VertexColorProperty,
                    VertexColorProperty { flags_18: 9 },
                ))
                .id(),
        );
        world
            .entity_mut(root)
            .insert(crate::gamebryo::av_object::NiAvProperties(vec![
                Some(same_c),
                Some(same_z),
            ]));
        init_model(
            &mut world,
            b"w.nif",
            &loaded_with(root),
            false,
            &mut services,
        )
        .unwrap();
        assert_eq!(
            world
                .get::<crate::gamebryo::av_object::NiAvProperties>(root)
                .unwrap()
                .0,
            [Some(same_c)]
        );
        drop(services);
        assert_eq!(disabled, 7, "00436C6B restores it");
        assert_eq!(
            warnings,
            [(
                b"Creator: Reexport 'w.nif' to get rid of the ZBuffer and/or VertextColor property."
                    .to_vec(),
                true
            )]
        );
    }

    // 004395D0's a7 arm with a LOD dereferences a null root (a host stop), and runs on with none.
    #[test]
    fn the_a7_arm_stops_only_with_a_lod() {
        let mut world = drain_world();
        let id = queue(&mut world, 1, 7);
        {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let loader = tasks.model_loader_b33a1c.as_mut().unwrap();
            loader.models.store.push(Model::default());
            let io = tasks.io_manager_b33a10.as_mut().unwrap();
            let task = io.tasks.store[id.0].as_mut().unwrap();
            task.model_28 = Some(ModelId(0));
            task.lod_30 = 4;
        }
        let thread = std::thread::current().id();
        let mut trace = ModelTaskTrace::default();
        assert!(finalize_queued_model(&mut world, id, &mut trace, thread).is_err());
        {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let io = tasks.io_manager_b33a10.as_mut().unwrap();
            io.tasks.store[id.0].as_mut().unwrap().lod_30 = 0;
        }
        finalize_queued_model(&mut world, id, &mut trace, thread).unwrap();
        assert!(trace.inserted);
    }

    #[test]
    fn the_log_line_is_g3s() {
        let record = ModelDrainRecord {
            run: 15,
            built: 7,
            cached: 8,
            load_failed: 0,
            queued_havok: 8,
            queued_nif_10_2: 3,
            tasks: Vec::new(),
        };
        assert_eq!(
            record.log_line(1),
            "Phase-13e: model tasks run 15 (models built 7, cached 8, failed 0), queued 11 (8 Havok, 3 NIF 10.2.0.0), shader setups not run 1"
        );
    }

    #[test]
    fn release_at_zero_frees_and_counts_down() {
        let mut tasks = task_globals();
        tasks
            .model_loader_b33a1c
            .as_mut()
            .unwrap()
            .models
            .store
            .push(Model {
                references_04: 3,
                ..Model::default()
            });
        let out = load_model(&mut tasks, None, b"x.nif", REQUEST).unwrap();
        assert_eq!(tasks.counters.live_tasks_b33a20, 1);
        let io = tasks.io_manager_b33a10.as_mut().unwrap();
        let task = io.tasks.store[out.unwrap().0].as_mut().unwrap();
        task.model_28 = Some(ModelId(0));
        task.references_08 = 1;
        release_task(&mut tasks, out).unwrap();
        assert_eq!(tasks.counters.live_tasks_b33a20, 0);
        assert_eq!(
            tasks.model_loader_b33a1c.as_ref().unwrap().models.store[0].references_04,
            2,
            "004393BE"
        );
        tasks.stop_worker_threads();
    }
}
