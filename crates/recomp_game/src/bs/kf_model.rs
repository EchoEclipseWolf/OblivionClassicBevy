//! The model loader's KF files: `0043B840`'s map (+0x04), `00439FF0` ModelLoader_LoadKFNow and the KFModel it builds.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::archive::ArchiveManager;
use super::model_loader::{
    ModelBuildServices, find_model_file_entry, fixup_model_path, model_map_bucket,
};
use super::stream::{BsStreamLoadStatus, load_bs_stream_with_reader};
use super::task_manager::TaskGlobals;
use crate::gamebryo::controller_sequence::ControllerSequence;
use crate::gamebryo::stream_objects::NiClass;

/// IDA: N/A an index into `KfMap::store`, for a KFModel pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KfModelId(pub usize);

/// IDA: 00439140 KFModel (0x10): +0x00 the path copy, +0x04 the NiControllerSequence (top object 0), +0x08 the TESAnimGroup (`tes::anim_group::TesAnimGroup` on its own entity, a NiRefObject), +0x0C the references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KfModel {
    pub path_00: Vec<u8>,
    pub sequence_04: Option<Entity>,
    pub anim_group_08: Option<Entity>,
    pub references_0c: u32,
}

/// IDA: one node of ModelLoader +0x04's map (vtable A371DC, case-insensitive like the model map's): the lower-case key and the KFModel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KfMapEntry {
    pub key: Vec<u8>,
    pub kf: KfModelId,
}

/// IDA: ModelLoader +0x04's entries: 1009 buckets by 0043E720's hash, each sorted by strcmp, as the model map's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KfMap {
    pub store: Vec<KfModel>,
    pub buckets: Vec<Vec<KfMapEntry>>,
}

impl KfMap {
    // IDA: 0043E680's walk on +0x04 (the same vtable bodies as the model map's): the lower-case key's bucket, an equal key.
    fn lookup(&self, key: &[u8]) -> Option<KfModelId> {
        self.buckets
            .get(model_map_bucket(key) as usize)
            .and_then(|chain| chain.iter().find(|entry| entry.key == key))
            .map(|entry| entry.kf)
    }

    // IDA: 0043E760 on +0x04 with a5 = 0: an equal key fails; otherwise the node goes in at the first key at or past it.
    fn insert(&mut self, key: Vec<u8>, kf: KfModelId) -> bool {
        if self.buckets.is_empty() {
            self.buckets = vec![Vec::new(); 1009];
        }
        let chain = &mut self.buckets[model_map_bucket(&key) as usize];
        let position = chain
            .iter()
            .position(|entry| entry.key >= key)
            .unwrap_or(chain.len());
        if chain.get(position).is_some_and(|entry| entry.key == key) {
            return false;
        }
        chain.insert(position, KfMapEntry { key, kf });
        true
    }
}

// IDA: 0043E69E's lower-case key (`model_tolower`), with no length check on its 0x3E8 stack bytes.
fn lower_key(path: &[u8]) -> Result<Vec<u8>, StreamError> {
    if path.len() >= 0x3E8 {
        return Err(StreamError("0043E6BE: a KF key past the 0x3E8 stack bytes"));
    }
    Ok(path
        .iter()
        .map(|&byte| recomp_core::crt::c_tolower(i32::from(byte as i8)) as u8)
        .collect())
}

// IDA: N/A a lookup of the map's vt+4 under its lock's thread registration (00431D60).
fn kf_map_lookup(tasks: &mut TaskGlobals, path: &[u8]) -> Result<Option<KfModelId>, StreamError> {
    let key = lower_key(path)?;
    let loader = tasks
        .model_loader_b33a1c
        .as_mut()
        .ok_or(StreamError("00439FFA: a null g_modelLoader"))?;
    loader
        .kf_map_04
        .lock_14
        .register_thread(std::thread::current().id())?;
    Ok(loader.kfs.lookup(&key))
}

/// IDA: N/A a KFModel's fields, for its user.
#[must_use]
pub fn kf_model(world: &World, kf: KfModelId) -> Option<&KfModel> {
    world
        .resource::<TaskGlobals>()
        .model_loader_b33a1c
        .as_ref()?
        .kfs
        .store
        .get(kf.0)
}

/// IDA: 00439FF0 ModelLoader_LoadKFNow(path) (ecx g_modelLoader): +0x04's vt+4. A miss: off the IO thread 00432860 pauses it (D9: a host no-op), a stack QueuedKF (`004377D0`: the BSTask base, `QueuedFile_CopyPath`, `QueuedFile_FindFileEntry(0, 1)`), `00439940` QueuedKF_Run, `004378F0` QueuedKF_Finalize, 00432890; the task's +0x28 is taken, and the stack task goes (the live count back down). A found or built KFModel counts up (+0x0C) and answers. Completion: 100%.
/// # Errors
/// No ModelLoader, a key past its buffer, or a stop in the run.
pub fn load_kf_now(
    world: &mut World,
    path: &[u8],
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<KfModelId>, StreamError> {
    let mut found = kf_map_lookup(&mut world.resource_mut::<TaskGlobals>(), path)?;
    if found.is_none() {
        {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            tasks.counters.live_tasks_b33a20 = tasks.counters.live_tasks_b33a20.wrapping_add(1);
        }
        let mut entry = Ok(None);
        let mut find = |archives: Option<&mut ArchiveManager>| {
            entry = find_model_file_entry(archives, path);
        };
        match services.archives.as_mut() {
            Some(archives) => archives.with_archives(&mut find),
            None => find(None),
        }
        let entry = entry?;
        let (kf, cached) = run_queued_kf(world, path, entry, services)?;
        found = finalize_queued_kf(world, path, kf, cached)?;
        let mut tasks = world.resource_mut::<TaskGlobals>();
        tasks.counters.live_tasks_b33a20 = tasks.counters.live_tasks_b33a20.wrapping_sub(1);
    }
    if let Some(kf) = found {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let held = &mut tasks
            .model_loader_b33a1c
            .as_mut()
            .expect("looked up above")
            .kfs
            .store[kf.0];
        held.references_0c = held.references_0c.wrapping_add(1);
    }
    Ok(found)
}

// IDA: 00439940 QueuedKF_Run: +0x04's vt+4 into +0x28; a hit sets +0x2C |= 1 and returns. Otherwise 00434710 into 0x104 stack bytes, the reader (+0x24 through 0042EBC0, else a nonnull +0x20 through 0042E8E0(path, -1, 1)), 00713A30 with BSStream's vtable, 006F9980(stream, fixed path, reader); a load builds FormHeap(0x10) 00439140 KFModel(+0x20, stream) into +0x28. The stream goes. Answers +0x28 and +0x2C bit 0. Completion: 100%.
fn run_queued_kf(
    world: &mut World,
    path: &[u8],
    entry: Option<super::model_loader::ArchiveFileEntry>,
    services: &mut ModelBuildServices<'_>,
) -> Result<(Option<KfModelId>, bool), StreamError> {
    if let Some(kf) = kf_map_lookup(&mut world.resource_mut::<TaskGlobals>(), path)? {
        return Ok((Some(kf), true));
    }
    let source = fixup_model_path(path);
    if source.len() >= 0x104 {
        return Err(StreamError(
            "00439993: a KF path past the 0x104 stack bytes",
        ));
    }
    let text: String = path.iter().map(|&byte| char::from(byte)).collect();
    let mut reader = Ok(None);
    let mut read = |archives: Option<&mut ArchiveManager>| {
        reader = match (entry, archives) {
            (Some(entry), Some(archives)) => {
                super::model_loader::open_archive_entry(archives, entry)
            }
            (Some(_), None) => Err(StreamError("004399A6: 0042EBC0 with no archives")),
            (None, Some(archives)) => Ok(archives
                .open_packed(&text, -1, 1)
                .map(super::file::OpenedFile::into_game_file)),
            (None, None) => Ok(None),
        };
    };
    match services.archives.as_mut() {
        Some(archives) => archives.with_archives(&mut read),
        None => read(None),
    }
    let reader = reader?;
    let source_text: String = source.iter().map(|&byte| char::from(byte)).collect();
    let load = load_bs_stream_with_reader(world, &source_text, reader, &mut services.stream)?;
    if load.status != BsStreamLoadStatus::Loaded {
        return Ok((None, false));
    }
    let kf = construct_kf_model(world, path, &load, services)?;
    let mut tasks = world.resource_mut::<TaskGlobals>();
    let loader = tasks
        .model_loader_b33a1c
        .as_mut()
        .ok_or(StreamError("00439A2E: a null g_modelLoader"))?;
    loader.kfs.store.push(kf);
    Ok((Some(KfModelId(loader.kfs.store.len() - 1)), false))
}

// IDA: 004378F0 QueuedKF_Finalize: a nonnull +0x28 without +0x2C bit 0 goes in through +0x04's vt+0x0C(path, kf, 0); a failed insert looks the key up and takes that KFModel (0043A1E0's assign). vt+0x28 (the QueuedFile tail) has no parent here. Completion: 100%.
fn finalize_queued_kf(
    world: &mut World,
    path: &[u8],
    kf: Option<KfModelId>,
    cached: bool,
) -> Result<Option<KfModelId>, StreamError> {
    let Some(kf) = kf else {
        return Ok(None);
    };
    if cached {
        return Ok(Some(kf));
    }
    let key = lower_key(path)?;
    let mut tasks = world.resource_mut::<TaskGlobals>();
    let loader = tasks
        .model_loader_b33a1c
        .as_mut()
        .ok_or(StreamError("00437908: a null g_modelLoader"))?;
    if loader.kfs.insert(key.clone(), kf) {
        loader.kf_map_04.field_18 += 1;
        return Ok(Some(kf));
    }
    Ok(loader.kfs.lookup(&key))
}

// IDA: 00439140 KFModel_Constructor(path, stream): +0x04..+0x0C zero, the path copied, 006CB240 NiControllerSequence_GetFromStream(stream, 0): a stream at 5.0 or later takes top object 0 through NiDynamicCast(NiControllerSequence) (an older one converts, a stop here); with a sequence `00436DA0` KFModel_InitAnimGroup, else the "Could not create ControllerManager Sequence" warning. Completion: 100% of a 5.0+ stream.
fn construct_kf_model(
    world: &mut World,
    path: &[u8],
    load: &super::stream::BsStreamLoad,
    services: &mut ModelBuildServices<'_>,
) -> Result<KfModel, StreamError> {
    let mut kf = KfModel {
        path_00: path.to_vec(),
        sequence_04: None,
        anim_group_08: None,
        references_0c: 0,
    };
    let version = load
        .stream
        .as_ref()
        .map_or(0, |stream| stream.header.version);
    if version < 0x0500_0000 {
        return Err(StreamError(
            "006CB27A: a KF stream before 5.0 (006D89F0's conversion)",
        ));
    }
    kf.sequence_04 = load
        .root(0)
        .filter(|&object| world.get::<NiClass>(object) == Some(&NiClass::ControllerSequence));
    let Some(sequence) = kf.sequence_04 else {
        let mut text = b"Could not create ControllerManager Sequence for \"".to_vec();
        text.extend_from_slice(path);
        text.extend_from_slice(b"\".\r\n");
        (services.warning)(text, *services.disable_warning_messages_b06b18 != 0);
        return Ok(kf);
    };
    // 00436DA0: the sequence's +0x08 name cut at its first '_' (a NUL written in), 0051B490 into +0x08, then the name set to the path (00434930).
    {
        let mut state = world
            .get_mut::<ControllerSequence>(sequence)
            .expect("a NiControllerSequence");
        if let Some(name) = state.name_08.as_mut()
            && let Some(at) = name.iter().position(|&byte| byte == b'_')
        {
            name.truncate(at);
        }
    }
    kf.anim_group_08 =
        crate::tes::anim_group::create_from_sequence(world, sequence, path, services)?;
    world
        .get_mut::<ControllerSequence>(sequence)
        .expect("a NiControllerSequence")
        .name_08 = Some(path.to_vec());
    Ok(kf)
}

/// IDA: 00A36C7C QueuedKF's vtable (vt+4 00439940 Run, vt+8 004378F0 Finalize), on an IO task in the store.
pub const QUEUED_KF_VTABLE: u32 = 0x00A3_6C7C;

/// IDA: QueuedKF +0x28 (the KFModel) and +0x2C bit 0 (found in the map by the run) per IO task.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct QueuedKfTasks {
    pub kfs: std::collections::HashMap<super::model_loader::IoTaskId, (Option<KfModelId>, bool)>,
}

/// IDA: 0043B840 ModelLoader_QueueKF(path, priority, parent): the KF map's vt+4. A hit with a parent takes 00437890 (a QueuedKF
/// over the KFModel, counted, +0x0C = 5), QueuedFile_SetParent and vt+0x28 (`QueuedFile_NotifyParent`); a hit with no parent
/// does nothing. A miss takes 004377D0 (QueuedFile_CopyPath, QueuedFile_FindFileEntry(0, 1)), the parent and vt+0x20
/// (`queue_io_task`). Completion: 100%; the task's own reference drop leaves it in the store.
/// # Errors
/// No ModelLoader or IOManager, or a stop in the parent's notification.
pub fn queue_kf(
    world: &mut World,
    archives: Option<&mut ArchiveManager>,
    path: &[u8],
    priority: u8,
    parent: Option<super::reference_tasks::ReferenceTaskId>,
) -> Result<(), StreamError> {
    let hit = kf_map_lookup(&mut world.resource_mut::<TaskGlobals>(), path)?;
    if hit.is_some() && parent.is_none() {
        return Ok(());
    }
    let entry = if hit.is_none() {
        find_model_file_entry(archives, path)?
    } else {
        None
    };
    let id = {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let TaskGlobals {
            io_manager_b33a10,
            model_loader_b33a1c,
            counters,
            ..
        } = &mut *tasks;
        let loader = model_loader_b33a1c
            .as_mut()
            .ok_or(StreamError("0043B840: a null g_modelLoader"))?;
        let io = io_manager_b33a10
            .as_mut()
            .ok_or(StreamError("004346D0: a null g_IOManager"))?;
        counters.live_tasks_b33a20 = counters.live_tasks_b33a20.wrapping_add(1);
        let id = super::model_loader::IoTaskId(io.tasks.store.len());
        io.tasks.store.push(Some(super::model_loader::QueuedModel {
            original_vtable: QUEUED_KF_VTABLE,
            references_08: 1,
            state_0c: if hit.is_some() { 5 } else { 0 },
            key_10: u64::from(priority) << 16,
            parent_18: None,
            children_1c: 0,
            path_20: if hit.is_some() {
                Vec::new()
            } else {
                path.to_vec()
            },
            entry_24: entry,
            model_28: None,
            field_2c: 0,
            lod_30: 0,
            flags_34: 0,
            host_gate: None,
        }));
        if let Some(parent) = parent {
            super::model_loader::set_parent(io, loader, id, parent)?;
        }
        if hit.is_none() {
            super::model_loader::queue_io_task(io, counters, id)?;
        }
        id
    };
    world
        .get_resource_or_init::<QueuedKfTasks>()
        .kfs
        .insert(id, (hit, hit.is_some()));
    if hit.is_some() {
        if let Some(kf) = hit {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let held = &mut tasks
                .model_loader_b33a1c
                .as_mut()
                .expect("looked up above")
                .kfs
                .store[kf.0];
            held.references_0c = held.references_0c.wrapping_add(1);
        }
        crate::tes::reference_load::queued_file_notify_parent(world, id)?;
    }
    Ok(())
}

/// IDA: 00439940 QueuedKF_Run and 004378F0 QueuedKF_Finalize on an IO task in the walk: the run's KFModel (`run_queued_kf`), then
/// the map's insert (`finalize_queued_kf`), into +0x28. Completion: 100%.
/// # Errors
/// As `run_queued_kf`.
pub fn run_queued_kf_in_walk(
    world: &mut World,
    id: super::model_loader::IoTaskId,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), StreamError> {
    let (path, entry) = {
        let tasks = world.resource::<TaskGlobals>();
        let task = tasks
            .io_manager_b33a10
            .as_ref()
            .and_then(|io| io.tasks.store.get(id.0))
            .and_then(Option::as_ref)
            .ok_or(StreamError("00439940: a released QueuedKF"))?;
        (task.path_20.clone(), task.entry_24)
    };
    {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        if let Some(task) = tasks
            .io_manager_b33a10
            .as_mut()
            .and_then(|io| io.tasks.store.get_mut(id.0))
            .and_then(Option::as_mut)
        {
            task.state_0c = 4;
        }
    }
    let (kf, cached) = run_queued_kf(world, &path, entry, services)?;
    let kf = finalize_queued_kf(world, &path, kf, cached)?;
    world
        .get_resource_or_init::<QueuedKfTasks>()
        .kfs
        .insert(id, (kf, cached));
    Ok(())
}
