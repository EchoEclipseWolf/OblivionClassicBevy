//! Phase 09 (D1): TES_Constructor 00441970("Data\\", g_objectLODRoot, land-LOD node, g_sky), which the TES block of 00406360 Startup_InitializeGameData
//! (00406604..00406644, `recomp_startup::initialize_game_data`) runs on FormHeap(0xAC) and stores into 00B333A0. The constructor builds the grid arrays, runs
//! the Bethesda-side Havok setup (`bs::havok`), the task infrastructure (`bs::task_manager`), the DataHandler and its plugin file list (injected), the
//! TESSaveLoadGame, the cell buffers and the scene hookup.
//!
//! x87: the main thread is 24-bit here, so plain `f32` in the original grouping. Evidence: evidence/phase09-scope-2026-10-06/disasm-window.txt,
//! evidence/phase09-window-2026-10-06/disasm-09c.txt, P09-RUN-001.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::form::FormIdentity;
use super::form_components::TesModel;
use super::forms_init::lookup;
use super::game_settings::GameSettingCollection;
use super::settings::SettingCollection;
use super::sky::SkyGlobals;
use super::water::{
    TesWaterNodeRecord, TesWaterSurfaceRecord, WaterServices, create_tes_texture_image,
};
use crate::bs::archive::ArchiveAccess;
use crate::bs::havok::{
    HavokGlobals, HavokSettingInputs, create_default_havok_world, initialize_havok_system,
    skill_level_minimum,
};
use crate::bs::model_loader::{IoTaskId, ModelRequest, load_model, release_task};
use crate::bs::scene_graphs::{AttachChildStatus, SceneGlobalStore, attach_scene_child};
use crate::bs::task_manager::TaskGlobals;
use crate::gamebryo::registration::NiFactoryRegistry;
use crate::gamebryo::scene_node::{SceneNodePayload, spawn_scene_node};

/// IDA: vtables stored by the TES constructor family.
pub const TES_VTABLE: u32 = 0x00A3_7470;
pub const GRID_ARRAY_VTABLE: u32 = 0x00A3_D718;
pub const GRID_CELL_ARRAY_VTABLE: u32 = 0x00A3_D79C;
pub const GRID_DISTANT_ARRAY_VTABLE: u32 = 0x00A3_D7E0;
pub const BS_TEMP_NODE_MANAGER_VTABLE: u32 = 0x00A3_72EC;
pub const CHANGES_MAP_VTABLE: u32 = 0x00A3_A2EC;
pub const INTERIOR_CELL_NEW_REFERENCES_MAP_VTABLE: u32 = 0x00A3_A310;
pub const EXTERIOR_CELL_NEW_REFERENCES_MAP_VTABLE: u32 = 0x00A3_A330;
pub const NUMERIC_ID_BUFFER_MAP_VTABLE: u32 = 0x00A3_A350;
pub const NI_T_LARGE_ARRAY_UINT_VTABLE: u32 = 0x00A3_9044;

// IDA: the INI statics the constructor reads.
const GRIDS_TO_LOAD: u32 = 0x00B0_6A2C;
const GRID_DISTANT_COUNT: u32 = 0x00B0_6A90;
const GRID_DISTANT_TREE_RANGE: u32 = 0x00B0_6A98;
const FADE_DISTANCE: u32 = 0x00B0_6AB0;
const EXTERIOR_CELL_BUFFER: u32 = 0x00B0_51DC;
const INTERIOR_CELL_BUFFER: u32 = 0x00B0_51D4;
const PREEMPTIVELY_UNLOAD_CELLS: u32 = 0x00B0_51CC;
const UPDATE_TYPE: u32 = 0x00B0_520C;
const CHASE_DELTA_MULT: u32 = 0x00B0_5234;
const MOVE_LIMIT_MASS: u32 = 0x00B0_5214;
const ADD_BIPED_WHEN_KEYFRAMED: u32 = 0x00B0_521C;
const MAX_PICKS: u32 = 0x00B0_5224;
const ENTITY_BATCH_REMOVE_RATE: u32 = 0x00B0_522C;
const TINT_MIP_MAPS: u32 = 0x00B0_5244;
const SIM_TYPE: u32 = 0x00B1_1638;
const NUM_HAVOK_THREADS: u32 = 0x00B0_97D8;
const MAX_TIME: u32 = 0x00B0_97C0;

/// IDA: the implicit __declspec(thread) block of the executable (_tls_index 00BA9DE4 = 0): +0x184 cleared by the DataHandler constructor (00446EFA), +0x185 by
/// TESSaveLoadGame (0045B3CE). D4 asks for a host thread-local record; a Resource holds the main thread's block instead, since every Phase 09 writer is
/// on the main thread (host substitution).
/// TODO: [ ] A worker's own block, if a task thread reads these (M4, Phase 13).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GameThreadLocals {
    pub byte_184: u8,
    /// +0x185 change tracking: 0045B670 TESSaveLoadGame_SetChangeFlags (0045B682) and 00463A90 TESSaveLoadGame_UnloadForm return while it's clear; WinMain sets it where change records begin (0040EB8B), 005227A0 and the load functions clear it for their span.
    pub track_changes_185: u8,
    /// IDA: +0x186 (004C9FF0: the LAND / PGRD data gate; 0 at startup).
    pub byte_186: u8,
}

/// IDA: one GridCellArray slot (8 bytes): +0x00 TESObjectCELL*, +0x04 TESWaterNodeRecord* (TES_InitializeWater 0043F560 fills it).
#[derive(Debug, Default)]
pub struct GridCellEntry {
    pub cell_00: Option<Entity>,
    pub water_04: Option<Box<TesWaterNodeRecord>>,
}

/// IDA: 00483350 GridCellArray_Constructor (0x28): 00481DE0 GridArray vtable A3D718, then A3D79C; +0x24 NiPointer null; uGridsToLoad normalized (below 5 ->
/// 5, even -> +1) and written back; +0x0C size; +0x10 FormHeap(8 * size * size) memset 0; +0x14/+0x18/+0x1C = 0.0; 00B06A28 = 1; the +0x24 release arm (null
/// here); +0x20 = 0. vt+4 00482080 then stores +0x08 and +0x04 = -10000 (00481E00) and runs 004D5600. Completion: 100%.
#[derive(Debug, Default)]
pub struct GridCellArray {
    pub original_vtable: u32,
    /// IDA: unset until 00481E00 (host 0).
    pub world_x_04: i32,
    pub world_y_08: i32,
    pub size_0c: u32,
    pub grid_10: Vec<GridCellEntry>,
    pub float_14: f32,
    pub float_18: f32,
    pub float_1c: f32,
    pub byte_20: u8,
    pub pointer_24: Option<Entity>,
}

impl GridCellArray {
    // IDA: 00482150 GridCellArray_GetSlotAt(x, y): +0x10 + 8 * (y + x * size). Completion: 100%; out of range is a host stop.
    pub fn slot_at(&mut self, x: u32, y: u32) -> Result<&mut GridCellEntry, StreamError> {
        let index = y as usize + x as usize * self.size_0c as usize;
        self.grid_10
            .get_mut(index)
            .ok_or(StreamError("00482150: grid slot out of range"))
    }
}

/// IDA: one GridDistantArray entry (0x10 bytes): NiTextKey_Constructor 006D73E0 stores +0x04 = 0; 00483680 then clears +0x00, +0x08, +0x0C and +0x01. The
/// +0x02/+0x03 bytes stay uninitialized FormHeap bytes (host 0; P09-RUN-001 shows heap garbage there).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GridDistantEntry {
    pub byte_00: u8,
    pub byte_01: u8,
    pub unset_02: u16,
    pub object_04: Option<Entity>,
    pub x_08: i32,
    pub y_0c: i32,
}

/// IDA: 004838F0 GridDistantArray_Constructor (0x14). vt+4 is 00481E00. Completion: 100% of the order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GridDistantArray {
    pub original_vtable: u32,
    pub world_x_04: i32,
    pub world_y_08: i32,
    pub size_0c: u32,
    pub entries_10: Vec<GridDistantEntry>,
}

/// IDA: 00441C9D NiNode_Constructor(0) then vtable A372EC: BSTempNodeManager (0xE0 bytes; the +0xDC dword stays uninitialized). The rest of it is a
/// `NiClass::Node` entity, which shares every NiNode slot.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BsTempNodeManager {
    pub original_vtable: u32,
    pub unset_dc: u32,
}

/// IDA: the NiTMapBase<NiTPointerAllocator<uint>, ...> map family TESSaveLoadGame builds (0x10 bytes): +0x00 vtable (the NiTMapBase one, then the derived one),
/// +0x04 hash size, +0x08 buckets (4 * size memset 0), +0x0C count 0. Completion: 100%.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NiTPointerMapRecord {
    pub original_vtable: u32,
    pub hash_size_04: u32,
    pub buckets_08: Vec<u32>,
    pub count_0c: u32,
}

// IDA: 0045A860 / 0045A940 / 0045AA80 / 0045ABE0 the NiTMapBase map family: +0x04 = size, base vtable, +0x0C = 0, FormHeap(4 * size) memset 0 -> +0x08, derived
// vtable. Completion: 100%.
fn construct_pointer_map(size: u32, derived_vtable: u32) -> NiTPointerMapRecord {
    NiTPointerMapRecord {
        original_vtable: derived_vtable,
        hash_size_04: size,
        buckets_08: vec![0; size as usize],
        count_0c: 0,
    }
}

/// IDA: NiTLargeArray<uint> (0x18): +0x00 vtable A39044, +0x04 data (4 * capacity, uninitialized), +0x08 capacity, +0x0C size, +0x10 count, +0x14 grow-by.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NiTLargeArrayU32 {
    pub original_vtable: u32,
    pub data_04: Vec<u32>,
    pub capacity_08: u32,
    pub size_0c: u32,
    pub count_10: u32,
    pub grow_by_14: u32,
}

impl NiTLargeArrayU32 {
    /// IDA: 00446C50's array: FormHeap(4 * capacity), size and count 0.
    #[must_use]
    pub fn new(capacity: u32, grow_by: u32) -> Self {
        Self {
            original_vtable: NI_T_LARGE_ARRAY_UINT_VTABLE,
            data_04: vec![0; capacity as usize],
            capacity_08: capacity,
            size_0c: 0,
            count_10: 0,
            grow_by_14: grow_by,
        }
    }

    // IDA: 00446C50 SetAt(index, value): an index at or past the size grows the size to index + 1 (a nonzero value counts), otherwise a nonzero value into an
    // empty slot counts and a zero value over a used one uncounts; the value is stored. Completion: 100%; past the capacity is a host stop.
    pub fn set_at(&mut self, index: u32, value: u32) -> Result<(), StreamError> {
        if index >= self.size_0c {
            self.size_0c = index + 1;
            if value != 0 {
                self.count_10 += 1;
            }
        } else if value != 0 {
            if self.data_04[index as usize] == 0 {
                self.count_10 += 1;
            }
        } else if self.data_04[index as usize] != 0 {
            self.count_10 -= 1;
        }
        *self.data_04.get_mut(index as usize).ok_or(StreamError(
            "00446C50: index past the NiTLargeArray capacity",
        ))? = value;
        Ok(())
    }
}

/// IDA: 0045B300 TESSaveLoadGame_Constructor (0x1C4). Fields named by offset; the bytes the constructor does not store stay uninitialized FormHeap bytes (host
/// zero). Completion: 100% of the stores.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TesSaveLoadGame {
    /// ChangesMap (0x13AF).
    pub changes_map_00: NiTPointerMapRecord,
    /// IDA: N/A the chains of +0x00's buckets, which `changes_map_00` only counts.
    pub changes_00: ChangeChains,
    /// +0x04 the save load's own map, null in every startup run (SAVELOAD map04=0). Nonzero means `changes_04` holds its chains.
    pub field_04: u32,
    /// IDA: N/A the chains of +0x04's map.
    pub changes_04: ChangeChains,
    pub interior_references_08: NiTPointerMapRecord,
    pub exterior_references_0c: NiTPointerMapRecord,
    pub exterior_references_10: NiTPointerMapRecord,
    /// +0x14..+0x40.
    pub fields_14: [u32; 12],
    pub field_4c: u32,
    pub field_50: u32,
    /// +0x54..+0x60.
    pub numeric_id_maps_54: [NiTPointerMapRecord; 4],
    pub field_64: u32,
    pub field_68: u32,
    pub field_6c: u32,
    pub byte_70: u8,
    pub byte_71: u8,
    pub array_74: NiTLargeArrayU32,
    pub array_78: NiTLargeArrayU32,
    pub byte_7c: u8,
    pub byte_7d: u8,
    /// +0x80..+0x90.
    pub fields_80: [u32; 5],
    /// +0x94..+0xA0.
    pub fields_94: [u32; 4],
    pub field_a4: u32,
    pub bytes_a8: [u8; 4],
    pub field_ac: u32,
    pub field_1c0: u32,
}

/// IDA: N/A the entry of a change map: the 8-byte block 00452C20 allocates, which the bucket node points at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChangeRecord {
    /// +0x00 the change flags.
    pub flags_00: u32,
    /// +0x04 the loaded record, which only the save loader sets.
    pub block_04: u32,
}

/// IDA: N/A a change map's buckets (vt+4 006A9060: key modulo +0x04), each chain from its head, as 00452570 links a new key and 00452600 walks. The +0x04 size of the one 0x13AF map, from 0045B300.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeChains {
    buckets: Vec<Vec<(u32, ChangeRecord)>>,
}

/// IDA: 0045B300's push of the +0x00 map's size.
pub const CHANGES_MAP_BUCKETS: u32 = 0x13AF;

impl ChangeChains {
    // IDA: N/A a helper, with no address of its own.
    fn bucket(key: u32) -> usize {
        (key % CHANGES_MAP_BUCKETS) as usize
    }

    /// IDA: 0055E000 on the map. Completion: 100%.
    #[must_use]
    pub fn get(&self, key: u32) -> Option<ChangeRecord> {
        self.buckets
            .get(Self::bucket(key))?
            .iter()
            .find(|(entry, _)| *entry == key)
            .map(|(_, record)| *record)
    }

    /// IDA: 00452C20's body without the count: a found record, otherwise a new {0, 0} at the chain's head (00452570); then `flags` ORs in when +0x04 is null. Returns whether it added. Completion: 100%; FormHeap(8) failing stores a null value there and faults at 00452C78, which the host doesn't model.
    pub fn add(&mut self, key: u32, flags: u32) -> bool {
        if self.buckets.is_empty() {
            self.buckets
                .resize_with(CHANGES_MAP_BUCKETS as usize, Vec::new);
        }
        let chain = &mut self.buckets[Self::bucket(key)];
        let (added, record) = match chain.iter().position(|(entry, _)| *entry == key) {
            Some(at) => (false, &mut chain[at].1),
            None => {
                chain.insert(0, (key, ChangeRecord::default()));
                (true, &mut chain[0].1)
            }
        };
        if record.block_04 == 0 {
            record.flags_00 |= flags;
        }
        added
    }

    // IDA: N/A a store through 0055E000's value.
    fn set_flags(&mut self, key: u32, flags: u32) {
        if let Some(chain) = self.buckets.get_mut(Self::bucket(key))
            && let Some((_, record)) = chain.iter_mut().find(|(entry, _)| *entry == key)
        {
            record.flags_00 = flags;
        }
    }

    /// IDA: 004524C0 RemoveAt. Completion: 100%.
    pub fn remove(&mut self, key: u32) -> bool {
        let Some(chain) = self.buckets.get_mut(Self::bucket(key)) else {
            return false;
        };
        let Some(at) = chain.iter().position(|(entry, _)| *entry == key) else {
            return false;
        };
        chain.remove(at);
        true
    }

    /// IDA: 00452600's order: bucket by bucket, each chain from its head. A row printer walks this.
    pub fn walk(&self) -> impl Iterator<Item = (u32, ChangeRecord)> + '_ {
        self.buckets.iter().flatten().copied()
    }

    /// IDA: N/A the number of entries, which `NiTPointerMapRecord::count_0c` holds in the original.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buckets.iter().map(Vec::len).sum()
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buckets.iter().all(Vec::is_empty)
    }
}

impl TesSaveLoadGame {
    /// IDA: 0045A500 on `this`: on the thread 00B33398 +0x10 names, +0x18 bit 0; on another, bit 18. Host substitution: `World` code all runs on the main thread (the 13e drain too, where the original's IO thread would read bit 18), so this reads bit 0. It therefore doesn't read g_osGlobals, whose null would fault. Completion: 100% of the main thread's arm.
    #[must_use]
    pub const fn active(&self) -> bool {
        self.fields_14[1] & 1 != 0
    }

    // IDA: 00452C20(map, form, flags) on +0x00. Completion: 100%.
    fn add_to_00(&mut self, id: u32, flags: u32) {
        if self.changes_00.add(id, flags) {
            self.changes_map_00.count_0c = self.changes_map_00.count_0c.wrapping_add(1);
        }
    }

    /// IDA: 0045B700's choice, after its test of flag 0x4000: active with a +0x04 map, 00452C20 there; every other way, on +0x00. Completion: 100%.
    pub fn record_form_change(&mut self, id: u32, flags: u32) {
        if self.active() && self.field_04 != 0 {
            self.changes_04.add(id, flags);
        } else {
            self.add_to_00(id, flags);
        }
    }

    /// IDA: 0045B670's choice, after its gates: active needs +0x18 bit 3 and a +0x04 map, and adds there, or nothing happens; not active adds on +0x00. Completion: 100%.
    pub fn set_change_flags(&mut self, id: u32, flags: u32) {
        if self.active() {
            if self.fields_14[1] & 8 != 0 && self.field_04 != 0 {
                self.changes_04.add(id, flags);
            }
        } else {
            self.add_to_00(id, flags);
        }
    }

    /// IDA: 00452D60 ChangesMap_RemoveFlags(form, flags) on +0x00, as 004533D0 reaches it: +0x18 bit 12 answers untouched; a miss too; a record with no +0x04 block loses the flags; a record at zero then goes through 004524C0, whatever its block, whose release is `Err` (00452DC6). Completion: 100% of the arms but the block's release.
    /// # Errors
    /// A record with a block at zero flags.
    pub fn clear_change_flags(&mut self, id: u32, flags: u32) -> Result<(), StreamError> {
        if self.fields_14[1] & 0x1000 != 0 {
            return Ok(());
        }
        let Some(record) = self.changes_00.get(id) else {
            return Ok(());
        };
        let left = if record.block_04 == 0 {
            let left = record.flags_00 & !flags;
            self.changes_00.set_flags(id, left);
            left
        } else {
            record.flags_00
        };
        if left != 0 {
            return Ok(());
        }
        if record.block_04 != 0 {
            return Err(StreamError(
                "00452DC6: 00401E40 releases a change record's block",
            ));
        }
        self.changes_00.remove(id);
        self.changes_map_00.count_0c = self.changes_map_00.count_0c.wrapping_sub(1);
        Ok(())
    }

    /// IDA: 00452DF0(this->+0x00, id, release): +0x18 bit 12 answers false untouched; a miss answers false; a record with a block stays unless `release`; otherwise 004524C0, the block to 00B02020's 00401E40, the free. Answers whether a record was found. `Err` is 00452E50, a block no host path makes. Completion: 100% of the arms but the block's release.
    /// # Errors
    /// A record with a +0x04 block being released.
    pub fn remove_change(&mut self, id: u32, release: bool) -> Result<bool, StreamError> {
        if self.fields_14[1] & 0x1000 != 0 {
            return Ok(false);
        }
        let Some(record) = self.changes_00.get(id) else {
            return Ok(false);
        };
        if record.block_04 != 0 {
            if !release {
                return Ok(true);
            }
            return Err(StreamError(
                "00452E50: 00401E40 releases a change record's block",
            ));
        }
        self.changes_00.remove(id);
        self.changes_map_00.count_0c = self.changes_map_00.count_0c.wrapping_sub(1);
        Ok(true)
    }
}

/// IDA: 0045B780 TESSaveLoadGame_RemoveChanges(form, release) on g_saveLoadGame: a temporary form returns, otherwise a tail call of 00452DF0 on +0x00. The caller tested g_saveLoadGame. Completion: 100%.
/// # Errors
/// As `remove_change`.
pub fn remove_form_changes(
    world: &mut World,
    form: Entity,
    release: bool,
) -> Result<(), StreamError> {
    let identity = *world.get::<FormIdentity>(form).expect("a form");
    if identity.flags_08 & 0x4000 != 0 {
        return Ok(());
    }
    let mut globals = world.resource_mut::<TesGlobals>();
    let game = globals
        .save_load_game
        .as_mut()
        .ok_or(StreamError("0045B792: g_saveLoadGame is null"))?;
    game.remove_change(identity.form_id_0c, release)?;
    Ok(())
}

/// IDA: 004637C0's head (form) on g_saveLoadGame: 0045A500 off answers false; a +0x00 record with no +0x04 block answers false. Past that is the change record's load, which needs a save (`Err`). Answers the original's al. Completion: 100% of the reached arms; P14-RUN-001 has +0x18 = 0, so its second test isn't reached at startup.
/// # Errors
/// 0046385E, on a record with a block.
pub fn load_form_change(world: &World, form: Entity) -> Result<bool, StreamError> {
    let game = world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .ok_or(StreamError("00463812: g_saveLoadGame is null"))?;
    if !game.active() {
        return Ok(false);
    }
    let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
    match game.changes_00.get(id) {
        Some(record) if record.block_04 != 0 => Err(StreamError(
            "0046385E: 004637C0 loads a form's change record",
        )),
        _ => Ok(false),
    }
}

/// IDA: 004431F0 TES_SetCurrentWorldspace(world) on g_tes. Null or unchanged returns. 00B051CC bPreemptivelyUnloadCells (0 in the pinned INI, as in the run) runs the unload block; then g_osGlobals +0x18 = 0, +0x74 = world, 004425D0, the six coordinates; each DataHandler +0x0C world: 004EF7E0, then 004EBC00 on the root's map for the new world and 004EA570 for the others (both through `(world == item)`, so a child of the new world refreshes the map it just filled); 0057A0D0. The two 00977C50 calls answer true with no effect. Completion: 100% of the reached arms; the unload block stops at 0044324A, or sooner on a loaded cell.
/// # Errors
/// A null g_tes or g_osGlobals, an empty worldspace list, or a stop below.
pub fn set_current_worldspace(
    world: &mut World,
    target: Option<Entity>,
    ini: &SettingCollection,
    lod: &mut super::forms_world::TerrainLodServices<'_>,
    refresh_map_menu: &mut dyn FnMut(&mut World) -> Result<bool, StreamError>,
) -> Result<(), StreamError> {
    let Some(target) = target else {
        return Ok(());
    };
    let tes = world
        .resource::<TesGlobals>()
        .tes
        .as_ref()
        .ok_or(StreamError(
            "00443200: TES_SetCurrentWorldspace on a null g_tes",
        ))?;
    if tes.current_world_space_74 == Some(target) {
        return Ok(());
    }
    let preemptive = ini
        .find_address(PREEMPTIVELY_UNLOAD_CELLS)
        .ok_or(StreamError(
            "00B051CC: bPreemptivelyUnloadCells is not registered",
        ))?
        .raw as u8;
    if preemptive != 0 {
        preemptive_unload_cells(world)?;
    }
    world
        .get_resource_mut::<super::os_globals::OsGlobalsData>()
        .ok_or(StreamError("00443267: +0x18 on a null g_osGlobals"))?
        .unk_18 = 0;
    world
        .resource_mut::<TesGlobals>()
        .tes
        .as_mut()
        .expect("tested above")
        .current_world_space_74 = Some(target);
    tes_clear_grids(world)?;
    tes_reset_exterior_coordinates(
        world
            .resource_mut::<TesGlobals>()
            .tes
            .as_mut()
            .expect("tested above"),
    );
    let worlds: Vec<Entity> = world
        .resource::<super::data_handler::DataHandlerLists>()
        .world_spaces_0c
        .items()
        .collect();
    if worlds.is_empty() {
        return Err(StreamError("004432A0: 004EF7E0 on a null list item"));
    }
    for item in worlds {
        let root = super::forms_world::world_lod_root(world, item)?;
        if item == target {
            super::forms_world::terrain_lod_activate(world, root, lod)?;
        } else {
            super::forms_world::terrain_lod_refresh(world, root, false)?;
        }
    }
    refresh_map_menu(world)?;
    Ok(())
}

// IDA: 0044321C..0044325C, 004431F0's unload block: with +0x34 set, 004425D0; 0043FFF0(1, +0x74) over the exterior buffer; without +0x34, 0043FE30(1) over the interior buffer; 004C9300; with a count or al, 0043FC20(0). The buffers hold nothing at startup, and any cell in one needs 004CA030 and 00447BA0. Completion: 100% of the empty-buffer passes, which stop at 004C9300's texture walk.
fn preemptive_unload_cells(world: &mut World) -> Result<(), StreamError> {
    let interior = world
        .resource::<TesGlobals>()
        .tes
        .as_ref()
        .expect("the caller tested")
        .current_interior_cell_34;
    if interior.is_some() {
        tes_clear_grids(world)?;
    }
    let tes = world
        .resource::<TesGlobals>()
        .tes
        .as_ref()
        .expect("the caller tested");
    if tes.exterior_cell_buffer_3c.iter().any(Option::is_some) {
        return Err(StreamError(
            "00440036: 0043FFF0 unloads a buffered exterior cell",
        ));
    }
    if tes.current_interior_cell_34.is_none()
        && tes.interior_cell_buffer_38.iter().any(Option::is_some)
    {
        return Err(StreamError(
            "0043FE65: 0043FE30 unloads a buffered interior cell",
        ));
    }
    Err(StreamError(
        "0044324A: 004C9300, then 0043FC20 (bPreemptivelyUnloadCells, 0 at startup)",
    ))
}

/// IDA: 0043F220 TES_ResetExteriorCoordinates: +0x20..+0x2C and +0x48, +0x4C = 0x7FFFFFFF. Completion: 100%.
pub const fn tes_reset_exterior_coordinates(tes: &mut Tes) {
    tes.exterior_coordinates_20 = [0x7FFF_FFFF; 4];
    tes.last_exterior_coordinates_48 = [0x7FFF_FFFF; 2];
}

/// IDA: 004425D0 TES_ClearGrids. A null g_dataHandler returns. GridCellArray vt+8 00481E20 runs vt+0x18 00482200 per (i, j): a loaded cell is 004D6450's, a host stop. GridDistantArray vt+8 runs 00483630 per (i, j): 007B3A40 over 00B2C33C, then 00483680: a nonzero +0x04 is a host stop;
/// 004EF1D0 / 004B27F0 and 004BD380 on 00B35B8C are counted boundaries over the empty startup maps (P12-RUN-002); then +0x00 = 0, +0x08 = 0, +0x0C = 0, +0x01 = 0. Then 00B4310C released and cleared, 00B06A28 = 1, and 007C4D90 (a counted boundary: the tall-grass caches are empty at startup, P12-RUN-002).
/// Completion: 100% of the stores for the startup state.
/// TODO: [ ] The loaded-cell and entry-object arms (Phase 15+).
/// TODO: [ ] 00442629 007C4D90: 007C4CE0's walk of 00B2CBC4, its 007C2A40 and its stores 00B43348 = 00B4334C = 0, then the walk of 00B2CBD4 and its 007C2A40. Both maps are empty at startup and nothing here reads those two words, so nothing is stored.
/// TODO: [ ] 0048372C 004BD380 on 00B35B8C: a key found there is 00432130's. The map is empty at startup.
pub fn tes_clear_grids(world: &mut World) -> Result<(), StreamError> {
    if !world.contains_resource::<super::data_handler::DataHandler>() {
        return Ok(());
    }
    let mut globals = world.resource_mut::<TesGlobals>();
    let globals = &mut *globals;
    let tes = globals
        .tes
        .as_mut()
        .ok_or(StreamError("004425DC: TES_ClearGrids on a null g_tes"))?;
    if let Some(cells) = &tes.grid_cells_08
        && cells.grid_10.iter().any(|slot| slot.cell_00.is_some())
    {
        return Err(StreamError(
            "00482221: 004D6450 detaches a loaded grid cell (Phase 15+)",
        ));
    }
    if let Some(distant) = &mut tes.grid_distant_04 {
        for entry in &mut distant.entries_10 {
            if entry.object_04.is_some() {
                return Err(StreamError(
                    "004836A2: a GridDistantArray entry object is released (Phase 15+)",
                ));
            }
            entry.byte_00 = 0;
            entry.x_08 = 0;
            entry.y_0c = 0;
            entry.byte_01 = 0;
        }
    }
    if let Some(mut trees) = world.get_resource_mut::<crate::bs::tree_manager::TreeManagerGlobals>()
    {
        trees.pointer_b4310c = None;
    }
    world.resource_mut::<TesGlobals>().grid_changed_b06a28 = 1;
    Ok(())
}

/// IDA: 00442630 TES_UnloadCells(0, 0) at 0044F41A: the +0x7C nodes freed and +0x78 = 0; 004425D0; 007C7E50 on 007B4280(0) and 006AC210 on OSGlobals +0x24 (counted boundaries: their lists and maps are empty at startup, P12-RUN-002); 0043FFF0(0, 0) and 0043FE30(0) over the buffers and 00447BA0 on +0x34 unload loaded cells (host stops: no cell is
/// loaded at startup); +0xA8 = 1; +0x34 = 0. Completion: 100% of the stores for the startup state.
/// TODO: [ ] The cell unloads, and the nonzero-argument arms no caller here takes (Phase 15+).
pub fn tes_unload_cells(world: &mut World) -> Result<(), StreamError> {
    {
        let mut globals = world.resource_mut::<TesGlobals>();
        let tes = globals
            .tes
            .as_mut()
            .ok_or(StreamError("00442633: 00442630 on a null g_tes"))?;
        tes.exterior_cell_list_next_7c.clear();
        tes.exterior_cell_list_78 = None;
    }
    tes_clear_grids(world)?;
    let mut globals = world.resource_mut::<TesGlobals>();
    let tes = globals.tes.as_mut().expect("checked above");
    if tes.exterior_cell_buffer_3c.iter().any(Option::is_some) {
        return Err(StreamError(
            "0044001B: 0043FFF0 unloads an exterior buffer cell (Phase 15+)",
        ));
    }
    if tes.current_interior_cell_34.is_some() {
        return Err(StreamError(
            "004426AE: 00447BA0 unloads the current interior cell (Phase 15+)",
        ));
    }
    if tes.interior_cell_buffer_38.iter().any(Option::is_some) {
        return Err(StreamError(
            "0043FE54: 0043FE30 unloads an interior buffer cell (Phase 15+)",
        ));
    }
    tes.byte_a8 = 1;
    tes.current_interior_cell_34 = None;
    Ok(())
}

// IDA: 0045B300 TESSaveLoadGame_Constructor. Completion: 100% of the store order.
fn construct_save_load_game(
    globals: &mut TesGlobals,
    locals: &mut GameThreadLocals,
) -> Result<TesSaveLoadGame, StreamError> {
    // 0045B338..0045B3C0 the zero stores; +0x7D = 1.
    let mut game = TesSaveLoadGame {
        byte_7d: 1,
        ..TesSaveLoadGame::default()
    };
    // 0045B3C8 00B33B00 = this (before the TES constructor's own store).
    globals.trace(0x00B3_3B00, 4);
    // 0045B3CE TLS +0x185 = 0.
    locals.track_changes_185 = 0;
    // 0045B3D4..0045B529 the eight maps.
    game.changes_map_00 = construct_pointer_map(0x13AF, CHANGES_MAP_VTABLE);
    game.interior_references_08 =
        construct_pointer_map(37, INTERIOR_CELL_NEW_REFERENCES_MAP_VTABLE);
    game.exterior_references_0c =
        construct_pointer_map(37, EXTERIOR_CELL_NEW_REFERENCES_MAP_VTABLE);
    game.exterior_references_10 =
        construct_pointer_map(37, EXTERIOR_CELL_NEW_REFERENCES_MAP_VTABLE);
    for map in &mut game.numeric_id_maps_54 {
        *map = construct_pointer_map(37, NUMERIC_ID_BUFFER_MAP_VTABLE);
    }
    // 0045B529..0045B5AA NiTLargeArray<uint>(100, 50) -> +0x74; capacity nonzero so 00452910 is skipped; 00446C50(0, &zero).
    game.array_74 = NiTLargeArrayU32::new(100, 50);
    game.array_74.set_at(0, 0)?;
    // 0045B5AF..0045B629 NiTLargeArray<uint>(5, 1) -> +0x78; likewise.
    game.array_78 = NiTLargeArrayU32::new(5, 1);
    game.array_78.set_at(0, 0)?;
    // 0045B62E..0045B650 +0x94..+0xA0 = 0, +0x71 = +0x7C = 0x7D, +0x70 = 0.
    game.fields_94 = [0; 4];
    game.byte_71 = 0x7D;
    game.byte_7c = 0x7D;
    game.byte_70 = 0;
    Ok(game)
}

/// IDA: the 0xAC-byte TES (TES-Struct.md; three-part TES/TES_mbrs/TES_vtbl in the IDB). 00441970 stores the vtable A37470 and clears the list heads, then 004419F0
/// memset(this, 0, 0xAC) clears everything again, vtable included: +0x00 stays 0 (P09-RUN-001 record 11). `sky_5c` is whether +0x5C holds the sky: the Sky itself
/// is `SkyGlobals`.
#[derive(Debug, Default)]
pub struct Tes {
    pub vtable_00: u32,
    pub grid_distant_04: Option<GridDistantArray>,
    pub grid_cells_08: Option<GridCellArray>,
    pub scene_root_0c: Option<Entity>,
    pub land_lod_10: Option<Entity>,
    pub temp_node_manager_14: Option<Entity>,
    pub sun_light_18: Option<Entity>,
    pub fog_1c: Option<Entity>,
    pub exterior_coordinates_20: [i32; 4],
    pub current_exterior_cell_30: Option<Entity>,
    pub current_interior_cell_34: Option<Entity>,
    pub interior_cell_buffer_38: Vec<Option<Entity>>,
    pub exterior_cell_buffer_3c: Vec<Option<Entity>>,
    pub field_40: u32,
    pub field_44: u32,
    pub last_exterior_coordinates_48: [i32; 2],
    pub byte_50: u8,
    pub cell_test_active_51: u8,
    pub byte_52: u8,
    pub cell_borders_enabled_53: u8,
    pub water_surface_54: Option<TesWaterSurfaceRecord>,
    pub water_node_58: Option<Box<TesWaterNodeRecord>>,
    pub sky_5c: bool,
    pub field_60: u32,
    pub field_64: u32,
    pub bytes_68: [u8; 4],
    pub grid_center_margin_x_6c: f32,
    pub grid_center_margin_y_70: f32,
    /// +0x74 (004431F0).
    pub current_world_space_74: Option<Entity>,
    /// +0x78 BSSimpleList<TESObjectCELL*> (00444340): the head item, then the nodes after it.
    pub exterior_cell_list_78: Option<Entity>,
    pub exterior_cell_list_next_7c: Vec<Entity>,
    pub furniture_references_80: [u32; 2],
    pub queued_script_references_88: u32,
    pub actor_base_death_counts_8c: [u32; 2],
    /// +0x94..+0x9C the sBloodTexture* decals (00443550).
    pub blood_decals_94: [Option<Entity>; 3],
    pub particle_node_cache_first_a0: u32,
    pub particle_node_cache_next_a4: u32,
    pub byte_a8: u8,
    pub texture_purge_allowed_a9: u8,
    pub bytes_aa: [u8; 2],
}

/// IDA: the TES_Constructor stores into Havok/movement globals (00441A09..00441A89), in store order. Defaults are the image values before the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TesHavokSettingCopies {
    /// fDebrisMaxVelocity (00B33A40).
    pub debris_max_velocity_ba7920: f32,
    /// fDebrisMinExtent (00B33A38).
    pub debris_min_extent_ba791c: f32,
    /// iUpdateType:HAVOK (00B0520C).
    pub update_type_ba7918: u32,
    /// fChaseDeltaMult:HAVOK (00B05234).
    pub chase_delta_mult_b2e2f0: f32,
    /// fMoveLimitMass:HAVOK (00B05214).
    pub move_limit_mass_b2e764: f32,
    /// 004DE010.
    pub callback_b2e300: u32,
    /// bAddBipedWhenKeyframed:HAVOK (00B0521C).
    pub add_biped_ba7909: u8,
    /// iMaxPicks:HAVOK (00B05224).
    pub max_picks_b2e2f4: u32,
    /// iEntityBatchRemoveRate:HAVOK (00B0522C).
    pub batch_remove_b2e2f8: u32,
    /// 004E2160.
    pub callback_ba7a88: u32,
    /// fCharacterControllerMultipleStepSpeed^2.
    pub step_speed_squared_b2e784: f32,
    /// nullsub_2 0060D0A0.
    pub callback_ba7a84: u32,
}

impl Default for TesHavokSettingCopies {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            debris_max_velocity_ba7920: 0.0,
            debris_min_extent_ba791c: 0.0,
            update_type_ba7918: 0,
            chase_delta_mult_b2e2f0: 0.05,
            move_limit_mass_b2e764: 95.0,
            callback_b2e300: 0x0088_A870,
            add_biped_ba7909: 0,
            max_picks_b2e2f4: 40,
            batch_remove_b2e2f8: 100,
            callback_ba7a88: 0,
            step_speed_squared_b2e784: 10.0,
            callback_ba7a84: 0,
        }
    }
}

/// IDA: the TES-side globals Phase 09 writes. Defaults are the values the host observes before the window (P09-RUN-001 records 3..5).
#[derive(Resource, Debug)]
pub struct TesGlobals {
    /// 00B333A0.
    pub tes: Option<Box<Tes>>,
    /// 00B33B00.
    pub save_load_game: Option<TesSaveLoadGame>,
    /// 00B3F580.
    pub havok_flag_b3f580: u32,
    /// 00B33A2C.
    pub cell_cache_limit_b33a2c: u32,
    /// 00B06A28.
    pub grid_changed_b06a28: u8,
    /// 00B34418.
    pub grid_distant_count_b34418: u32,
    /// 00B3441C.
    pub grid_distant_tree_range_b3441c: u32,
    /// 00B34424 (released).
    pub distant_node_b34424: Option<Entity>,
    /// 00B34414.
    pub grid_distant_extent_b34414: f32,
    /// 00B34410.
    pub grid_distant_inner_b34410: f32,
    /// 00B2C334.
    pub lod_distance_b2c334: f32,
    /// 00B2C338.
    pub lod_fade_b2c338: f32,
    pub havok_copies: TesHavokSettingCopies,
    /// 00B35E50: the 21 FlameNode model slots 00443550 clears.
    pub flame_node_slots_b35e50: [u32; 21],
    /// IDA: N/A the global stores in order (address, width).
    pub store_trace: Vec<SceneGlobalStore>,
}

impl Default for TesGlobals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            tes: None,
            save_load_game: None,
            havok_flag_b3f580: 0,
            cell_cache_limit_b33a2c: 0,
            grid_changed_b06a28: 1,
            grid_distant_count_b34418: 0,
            grid_distant_tree_range_b3441c: 0,
            distant_node_b34424: None,
            grid_distant_extent_b34414: 0.0,
            grid_distant_inner_b34410: 0.0,
            lod_distance_b2c334: 20480.0,
            lod_fade_b2c338: 32768.0,
            havok_copies: TesHavokSettingCopies::default(),
            flame_node_slots_b35e50: [0; 21],
            store_trace: Vec::new(),
        }
    }
}

impl TesGlobals {
    /// IDA: N/A records one global store.
    pub fn trace(&mut self, address: u32, width: u8) {
        self.store_trace.push(SceneGlobalStore { address, width });
    }
}

/// IDA: N/A the inputs and injected steps of the window's TES block. The Havok, task, thread-local and Sky state are `World` resources.
pub struct TesConstructionInputs<'a> {
    /// 0088B070 registrations.
    pub registry: &'a mut NiFactoryRegistry,
    pub havok_settings: HavokSettingInputs,
    /// 00406626.
    pub data_path: &'a str,
    /// ebp = g_objectLODRoot.
    pub scene_root: Option<Entity>,
    /// esi.
    pub land_lod: Option<Entity>,
    /// 00B33A40 GMST.
    pub debris_max_velocity: f32,
    /// 00B33A38 GMST.
    pub debris_min_extent: f32,
    /// 00B33A50 GMST.
    pub step_speed: f32,
    /// 00441B4F..00441B8A DataHandler 00446D80 -> 00B33A98.
    pub construct_data_handler: &'a mut dyn FnMut(&mut World) -> Result<(), StreamError>,
    /// 00441B8F 0044A2B0(g_dataHandler, dataPath).
    pub build_file_list: &'a mut dyn FnMut(&mut World, &str) -> Result<(), StreamError>,
    /// 00441D1B 00B3F958 = 1 unless bTintMipMaps.
    pub enable_object_load_callbacks: &'a mut dyn FnMut(&mut World),
    /// IDA: N/A host test seam: 00406609 FormHeap failure (TES not built).
    pub fail_tes_allocation: bool,
}

// IDA: N/A the raw value of a static SettingInfo the constructor reads (the original addresses the object directly).
fn ini_raw(ini: &SettingCollection, address: u32) -> Result<u32, StreamError> {
    ini.find_address(address)
        .map(|setting| setting.raw)
        .ok_or(StreamError(
            "TES_Constructor: INI setting missing from the static collection",
        ))
}

// IDA: N/A a write-back to a static SettingInfo.
fn ini_mut(ini: &mut SettingCollection, address: u32) -> Result<&mut u32, StreamError> {
    ini.find_address_mut(address)
        .map(|setting| &mut setting.raw)
        .ok_or(StreamError(
            "TES_Constructor: INI setting missing from the static collection",
        ))
}

// IDA: N/A fills the GMST and INI inputs of the TES block from the stores the original reads: the GameSettingCollection (00B33A38/00B33A40/00B33A50, 00B374xx,
// 00B376F8, 00B3A508) and the INI collection (00B11638, 00B097C0, 00B097D8); 00B33A34 is 0 at startup; `current_directory` stands for GetCurrentDirectoryA
// (005351F6). Completion: 100% of the sources. Returns (settings, debris max velocity, debris min extent, step speed).
pub fn tes_setting_inputs(
    settings: &GameSettingCollection,
    ini: &SettingCollection,
    current_directory: Vec<u8>,
) -> Result<(HavokSettingInputs, f32, f32, f32), StreamError> {
    let gmst = |key: &str| {
        settings
            .find(key)
            .map(|setting| f32::from_bits(setting.raw))
            .ok_or(StreamError("TES_Constructor: missing GMST"))
    };
    Ok((
        HavokSettingInputs {
            maximum_wind: gmst("fMaximumWind")?,
            jump_move_base: gmst("fJumpMoveBase")?,
            jump_move_mult: gmst("fJumpMoveMult")?,
            jump_double_mult: gmst("fJumpDoubleMult")?,
            jump_fall_velocity_min: gmst("fJumpFallVelocityMin")?,
            skill_expert_min: skill_level_minimum(settings, 3),
            sim_type: ini_raw(ini, SIM_TYPE)? as i32,
            num_havok_threads: ini_raw(ini, NUM_HAVOK_THREADS)? as i32,
            max_time: f32::from_bits(ini_raw(ini, MAX_TIME)?),
            b33a34: 0,
            current_directory,
        },
        gmst("fDebrisMaxVelocity")?,
        gmst("fDebrisMinExtent")?,
        gmst("fCharacterControllerMultipleStepSpeed")?,
    ))
}

// IDA: 00483320 / 00483350 uGridsToLoad normalization: below 5 -> 5 (stored); an even value -> value + 1 (stored); odd values are left alone. Completion: 100%.
fn normalize_grids_to_load(ini: &mut SettingCollection) -> Result<u32, StreamError> {
    let grids = ini_mut(ini, GRIDS_TO_LOAD)?;
    if *grids < 5 {
        *grids = 5;
    } else if *grids & 1 == 0 {
        *grids += 1;
    }
    Ok(*grids)
}

// IDA: 00483350 GridCellArray_Constructor. Completion: 100% of the order.
fn construct_grid_cell_array(
    globals: &mut TesGlobals,
    ini: &mut SettingCollection,
) -> Result<GridCellArray, StreamError> {
    // 0048337B 00481DE0 GridArray vtable, 00483382 GridCellArray vtable; 0048338C +0x24 = 0.
    // 0048338F..004833B1 the normalization, then +0x0C = size.
    let size_0c = normalize_grids_to_load(ini)?;
    let mut grid_10 = Vec::new();
    // 004833C8..004833E2 FormHeap(8 * size^2) -> +0x10, memset 0.
    grid_10.resize_with(size_0c as usize * size_0c as usize, GridCellEntry::default);
    // 0048340D 00B06A28 = 1.
    globals.grid_changed_b06a28 = 1;
    globals.trace(0x00B0_6A28, 1);
    // 004833EA..00483407 +0x14/+0x18/+0x1C = 0.0; 00483414..00483437 the +0x24 release (null here), 00483440 +0x20 = 0.
    Ok(GridCellArray {
        original_vtable: GRID_CELL_ARRAY_VTABLE,
        size_0c,
        grid_10,
        ..GridCellArray::default()
    })
}

// IDA: 00483680 GridDistantArray_ClearEntry(i, j) on a fresh entry: +0x04 is null (no release, no 004EF1D0/004B27F0); 00B35B8C is still null at construction
// (004BDCD0 runs after), so 004BD380 is skipped; then +0x00, +0x08, +0x0C and +0x01 are cleared. Completion: 100% of the constructor's arms.
fn clear_grid_distant_entry(
    entry: &mut GridDistantEntry,
    tasks: &TaskGlobals,
) -> Result<(), StreamError> {
    if entry.object_04.is_some() {
        return Err(StreamError(
            "00483696: a loaded distant entry is not reached at construction",
        ));
    }
    if tasks.distant_lod_tasks_b35b8c.is_some() {
        return Err(StreamError(
            "0048372C: 004BD380 with 00B35B8C set is not reached here",
        ));
    }
    entry.byte_00 = 0;
    entry.x_08 = 0;
    entry.y_0c = 0;
    entry.byte_01 = 0;
    Ok(())
}

// IDA: 004838F0 GridDistantArray_Constructor. Completion: 100% of the order.
fn construct_grid_distant_array(
    globals: &mut TesGlobals,
    ini: &mut SettingCollection,
    tasks: &TaskGlobals,
) -> Result<GridDistantArray, StreamError> {
    // 0048391C 00481DE0, 00483929 vtable A3D7E0; 0048392F 00483320.
    let grids = normalize_grids_to_load(ini)?;
    // 00483934..00483953 00B34418 / 00B3441C copies, +0x0C = size.
    let count = ini_raw(ini, GRID_DISTANT_COUNT)?;
    let tree_range = ini_raw(ini, GRID_DISTANT_TREE_RANGE)?;
    globals.grid_distant_count_b34418 = count;
    globals.trace(0x00B3_4418, 4);
    globals.grid_distant_tree_range_b3441c = tree_range;
    globals.trace(0x00B3_441C, 4);
    let size_0c = grids + 2 * count;
    // 00483978..004839A0 FormHeap(16 * n + 4), count prefix, eh vector constructor with NiTextKey_Constructor (+0x04 = 0) -> +0x10.
    let mut grid = GridDistantArray {
        original_vtable: GRID_DISTANT_ARRAY_VTABLE,
        size_0c,
        entries_10: vec![GridDistantEntry::default(); size_0c as usize * size_0c as usize],
        ..GridDistantArray::default()
    };
    // 004839AC..004839D7 00483680(i, j) row-major.
    for entry in &mut grid.entries_10 {
        clear_grid_distant_entry(entry, tasks)?;
    }
    // 004839D9..00483A09 release 00B34424 when set.
    if globals.distant_node_b34424.is_some() {
        globals.distant_node_b34424 = None;
        globals.trace(0x00B3_4424, 4);
    }
    // 00483A09..00483A51 B34414 = float((uGridsToLoad << 12) * 0.5) + float(00B34418 << 12) (unsigned fild with the 2^32 fix-up; 24-bit).
    let half = x87::u32_to_x87_float(grids << 12) * 0.5;
    globals.grid_distant_extent_b34414 = half + x87::u32_to_x87_float(count << 12);
    globals.trace(0x00B3_4414, 4);
    // 00483A57..00483A7B B34410 = B34414 - fFadeDistance:DistantLOD; B2C334 = B34410; B2C338 = fFadeDistance.
    let fade = f32::from_bits(ini_raw(ini, FADE_DISTANCE)?);
    globals.grid_distant_inner_b34410 = globals.grid_distant_extent_b34414 - fade;
    globals.trace(0x00B3_4410, 4);
    globals.lod_distance_b2c334 = globals.grid_distant_inner_b34410;
    globals.trace(0x00B2_C334, 4);
    globals.lod_fade_b2c338 = fade;
    globals.trace(0x00B2_C338, 4);
    Ok(grid)
}

// IDA: 00441970 TES_Constructor. Completion: 100% of the store order of the reached branches. Unchecked dereferences the original would fault on are errors.
// TODO: [ ] 00B3F958 and the object-load callbacks it gates: the caller's closure until M4 reads them.
pub fn construct_tes(
    world: &mut World,
    globals: &mut TesGlobals,
    ini: &mut SettingCollection,
    inputs: &mut TesConstructionInputs<'_>,
) -> Result<Tes, StreamError> {
    world.init_resource::<HavokGlobals>();
    world.init_resource::<TaskGlobals>();
    world.init_resource::<GameThreadLocals>();
    // 0044199E..004419EA the vtable (TES_VTABLE) and list-head stores have no effect: 004419F0 memset(this, 0, 0xAC) zeroes every field, the vtable included.
    let mut tes = Tes::default();
    // 004419F5 00B3F580 = 1: `mov dword ptr`, and 00478C12 reads a dword (the C++ has a byte).
    globals.havok_flag_b3f580 = 1;
    globals.trace(0x00B3_F580, 4);
    // 004419FF 005350F0; 00441A04 00537760 (00BA7A50 = 005376E0).
    {
        let mut havok = world.resource_mut::<HavokGlobals>();
        initialize_havok_system(&mut havok, inputs.registry, &inputs.havok_settings);
        havok.step_callback_ba7a50 = 0x0053_76E0;
    }
    // 00441A09..00441A89 the copies, in store order.
    globals.havok_copies.debris_max_velocity_ba7920 = inputs.debris_max_velocity;
    globals.trace(0x00BA_7920, 4);
    globals.havok_copies.debris_min_extent_ba791c = inputs.debris_min_extent;
    globals.trace(0x00BA_791C, 4);
    globals.havok_copies.update_type_ba7918 = ini_raw(ini, UPDATE_TYPE)?;
    globals.trace(0x00BA_7918, 4);
    globals.havok_copies.chase_delta_mult_b2e2f0 = f32::from_bits(ini_raw(ini, CHASE_DELTA_MULT)?);
    globals.trace(0x00B2_E2F0, 4);
    globals.havok_copies.move_limit_mass_b2e764 = f32::from_bits(ini_raw(ini, MOVE_LIMIT_MASS)?);
    globals.trace(0x00B2_E764, 4);
    globals.havok_copies.callback_b2e300 = 0x004D_E010;
    globals.trace(0x00B2_E300, 4);
    globals.havok_copies.add_biped_ba7909 = ini_raw(ini, ADD_BIPED_WHEN_KEYFRAMED)? as u8;
    globals.trace(0x00BA_7909, 1);
    globals.havok_copies.max_picks_b2e2f4 = ini_raw(ini, MAX_PICKS)?;
    globals.trace(0x00B2_E2F4, 4);
    globals.havok_copies.batch_remove_b2e2f8 = ini_raw(ini, ENTITY_BATCH_REMOVE_RATE)?;
    globals.trace(0x00B2_E2F8, 4);
    globals.havok_copies.callback_ba7a88 = 0x004E_2160;
    globals.trace(0x00BA_7A88, 4);
    globals.havok_copies.step_speed_squared_b2e784 = inputs.step_speed * inputs.step_speed;
    globals.trace(0x00B2_E784, 4);
    globals.havok_copies.callback_ba7a84 = 0x0060_D0A0;
    globals.trace(0x00BA_7A84, 4);
    // 00441A93..00441AB3 FormHeap(0x28) 00483350 -> +0x08 (a failed allocation stores 0 and 00441AB6 faults).
    let mut grid_cells = construct_grid_cell_array(globals, ini)?;
    // 00441AC2 vt+4 00482080: 00481E00 (+0x08 = -10000, then +0x04 = -10000), then 004D5600.
    grid_cells.world_y_08 = -10000;
    grid_cells.world_x_04 = -10000;
    tes.grid_cells_08 = Some(grid_cells);
    world.resource_scope(|world, mut settings: Mut<GameSettingCollection>| {
        create_default_havok_world(
            &mut world.resource_mut::<HavokGlobals>(),
            &mut settings,
            &inputs.havok_settings,
        );
    });
    // 00441AC4..00441AE6 FormHeap(0x14) 004838F0 -> +0x04; 00441AF5 vt+4 00481E00.
    let mut grid_distant = {
        let tasks = world.resource::<TaskGlobals>();
        construct_grid_distant_array(globals, ini, tasks)?
    };
    grid_distant.world_y_08 = -10000;
    grid_distant.world_x_04 = -10000;
    tes.grid_distant_04 = Some(grid_distant);
    {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        // 00441AF7 004BE8A0, 00441AFC 004BDCD0.
        tasks.create_exterior_cell_task_map();
        tasks.create_distant_lod_task_map();
        // 00441B01..00441B2A FormHeap(0x3C) 00434480 -> 00B33A10.
        tasks.construct_io_manager()?;
    }
    globals.trace(0x00B3_3A10, 4);
    // 00441B23..00441B59 FormHeap(0x1C) 0043E950 -> 00B33A1C.
    world
        .resource_mut::<TaskGlobals>()
        .construct_model_loader()?;
    globals.trace(0x00B3_3A1C, 4);
    // 00441B4F..00441B8A FormHeap(0xCE0) 00446D80 -> 00B33A98.
    (inputs.construct_data_handler)(world)?;
    globals.trace(0x00B3_3A98, 4);
    // 00441B8F 0044A2B0(g_dataHandler, dataPath).
    (inputs.build_file_list)(world, inputs.data_path)?;
    // 00441B94..00441BB9 FormHeap(0x1C4) 0045B300 -> 00B33B00.
    let save_load_game = {
        let mut locals = world.resource_mut::<GameThreadLocals>();
        construct_save_load_game(globals, &mut locals)?
    };
    globals.save_load_game = Some(save_load_game);
    globals.trace(0x00B3_3B00, 4);
    // 00441BBE..00441BDE uExterior Cell Buffer = max(it, g * (g + 2) + 1), written back only when it grows.
    let grids = ini_raw(ini, GRIDS_TO_LOAD)?;
    let minimum_exterior = grids * (grids + 2) + 1;
    let exterior = ini_mut(ini, EXTERIOR_CELL_BUFFER)?;
    if *exterior < minimum_exterior {
        *exterior = minimum_exterior;
    }
    let exterior = *exterior;
    // 00441BDE..00441BF4 uInterior Cell Buffer: 0 -> 1 (written back).
    let interior = ini_mut(ini, INTERIOR_CELL_BUFFER)?;
    if *interior < 1 {
        *interior = 1;
    }
    let interior = *interior;
    // 00441BF4..00441C2D FormHeap(4 * interior) -> +0x38, FormHeap(4 * exterior) -> +0x3C; 00441C30..00441C58 both cleared.
    tes.interior_cell_buffer_38 = vec![None; interior as usize];
    tes.exterior_cell_buffer_3c = vec![None; exterior as usize];
    // 00441C5A..00441C80 +0x20..+0x2C, +0x48, +0x4C = 0x7FFFFFFF; +0x44 = +0x40 = 0; +0x0C = the scene root.
    tes.exterior_coordinates_20 = [0x7FFF_FFFF; 4];
    tes.last_exterior_coordinates_48 = [0x7FFF_FFFF; 2];
    tes.field_44 = 0;
    tes.field_40 = 0;
    tes.scene_root_0c = inputs.scene_root;
    // 00441C83..00441CB6 FormHeap(0xE0) NiNode(0), vtable A372EC -> +0x14.
    let temp = spawn_scene_node(world, SceneNodePayload::default());
    world.entity_mut(temp).insert(BsTempNodeManager {
        original_vtable: BS_TEMP_NODE_MANAGER_VTABLE,
        unset_dc: 0,
    });
    tes.temp_node_manager_14 = Some(temp);
    // 00441CAC..00441CC5 a nonnull root: vtable +0x84 AttachChild(temp, 0).
    if let Some(root) = inputs.scene_root
        && attach_scene_child(world, root, Some(temp), false) != AttachChildStatus::Attached
    {
        return Err(StreamError("00441CC5: BSTempNodeManager attach failed"));
    }
    // 00441CC7..00441CE5 +0x10 land LOD, +0x5C sky, +0x18 the sun light (005411C0), +0x1C the fog property (0053FB50).
    tes.land_lod_10 = inputs.land_lod;
    let sky = world
        .get_resource::<SkyGlobals>()
        .and_then(|sky| sky.singleton_b365c4.as_deref());
    tes.sky_5c = sky.is_some();
    tes.sun_light_18 = sky
        .and_then(|sky| sky.sun_28.as_ref())
        .and_then(|sun| sun.light_1c);
    tes.fog_1c = sky
        .and_then(|sky| sky.atmosphere_20.as_ref())
        .and_then(|atmosphere| atmosphere.fog_0c);
    // 00441CE8..00441CF1 +0x69 = 0, +0x74 = 0, +0x30 = 0, +0xA8 = 1.
    tes.bytes_68[1] = 0;
    tes.current_world_space_74 = None;
    tes.current_exterior_cell_30 = None;
    tes.byte_a8 = 1;
    // 00441CF8..00441D08 00B33A2C = max(it, g * g).
    if globals.cell_cache_limit_b33a2c < grids * grids {
        globals.cell_cache_limit_b33a2c = grids * grids;
        globals.trace(0x00B3_3A2C, 4);
    }
    // 00441D0D +0x53 = 0, 00441D10 +0x58 = 0.
    tes.cell_borders_enabled_53 = 0;
    tes.water_node_58 = None;
    // 00441D13..00441D1B 00B3F958 = 1 unless bTintMipMaps.
    if ini_raw(ini, TINT_MIP_MAPS)? & 0xFF == 0 {
        (inputs.enable_object_load_callbacks)(world);
        globals.trace(0x00B3_F958, 1);
    }
    // 00441D22..00441D2E +0x88 = 0, +0x51 = 0, +0x52 = 0, +0xA9 = 1.
    tes.queued_script_references_88 = 0;
    tes.cell_test_active_51 = 0;
    tes.byte_52 = 0;
    tes.texture_purge_allowed_a9 = 1;
    Ok(tes)
}

/// IDA: 00443550's model GMSTs in load order: 00B37150..00B371C0 step 8, then sSplashParticles 00B37B38.
const PRELOAD_MODEL_SETTINGS: [u32; 16] = [
    0x00B3_7150,
    0x00B3_7158,
    0x00B3_7160,
    0x00B3_7168,
    0x00B3_7170,
    0x00B3_7178,
    0x00B3_7180,
    0x00B3_7188,
    0x00B3_7190,
    0x00B3_7198,
    0x00B3_71A0,
    0x00B3_71A8,
    0x00B3_71B0,
    0x00B3_71B8,
    0x00B3_71C0,
    0x00B3_7B38,
];
/// IDA: the sBloodTexture* GMSTs, for TES +0x94, +0x98, +0x9C.
const PRELOAD_TEXTURE_SETTINGS: [u32; 3] = [0x00B3_71C8, 0x00B3_71D0, 0x00B3_71D8];
/// IDA: 00B067C0 g_flameNodeFormIDs, the 21 dwords 00443BB1 reads (byte offset below 0x54), checked against the exe's bytes.
pub const FLAME_NODE_FORM_IDS: [u32; 21] = [
    0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x1E, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D,
    0x2E, 0x2F, 0x30, 0x31, 0x32,
];
/// IDA: 00443A44's sprintf target: `char[260]` on 00443550's stack.
const DECAL_PATH_BUFFER: usize = 260;

/// IDA: N/A what 00443550 reaches past the world: 00B338E0's archives for 0042DB10 and 00442890's services.
pub struct TesPreloadServices<'a, 'b> {
    pub archives: Option<&'a mut dyn ArchiveAccess>,
    pub textures: WaterServices<'a, 'b>,
}

// IDA: the shape each model GMST and each FlameNode repeats: 0043B420(g_modelLoader, &out, path, 5, 0, 0, 0, 1, 1), then the out release when it's nonnull (004435C3, vt+0 at zero).
fn preload_model(
    world: &mut World,
    path: &[u8],
    archives: Option<&mut (dyn ArchiveAccess + '_)>,
) -> Result<Option<IoTaskId>, StreamError> {
    let mut tasks = world
        .get_resource_mut::<TaskGlobals>()
        .ok_or(StreamError("004435B4: a null g_modelLoader"))?;
    let request = ModelRequest {
        priority: 5,
        parent: None,
        lod: 0,
        a7: false,
        a8: true,
        a9: true,
    };
    let out = match archives {
        Some(archives) => {
            let mut out = None;
            archives.with_archives(&mut |manager| {
                out = Some(load_model(&mut tasks, manager, path, request));
            });
            out.expect("with_archives runs its body")?
        }
        None => load_model(&mut tasks, None, path, request)?,
    };
    release_task(&mut tasks, out)?;
    Ok(out)
}

// IDA: N/A a static string GMST's value, which 00443550 dereferences for its first byte: a null value faults, a host stop.
fn preload_setting(world: &World, address: u32) -> Result<Vec<u8>, StreamError> {
    world
        .get_resource::<GameSettingCollection>()
        .and_then(|settings| settings.text_at(address))
        .ok_or(StreamError("0044358F: a null string GMST value"))
}

/// IDA: 00443550 TES_PreloadDefaultAssets, in original order. 0044358F..00443A20 the 16 model GMSTs: a value whose first byte isn't 0 is requested and released. 00443A22..00443BA5 the three decals: a nonempty name through sprintf("%s\\%s\\%s", "Data", "Textures", name) into a char[260], 00442890(&out, buffer, quiet 0, archiveOnly 0), 0055E2A0 NiRefObjectPtr_CopyAssign into TES +0x94 + 4i (which stores a null result too), the out release. 00443BAF..00443C40 the 21 FlameNodes: 0046B250, and for a form, the word at +0x2C (0xFFFF: strlen of +0x28); nonzero asks +0x24 vt+0x14 (00412AB0: +0x04 or "") for the path and requests it; 00B35E50[i] = 0 regardless. No type test: +0x24 is a TESModel only by what the IDs name. Completion: 100% of the order; 13e runs what's queued.
/// # Errors
/// A null g_tes (00443A62's store faults), a form with no `TesModel`, a name past the buffer, or a stop below.
pub fn preload_default_assets(
    world: &mut World,
    services: &mut TesPreloadServices<'_, '_>,
) -> Result<(), StreamError> {
    if world
        .get_resource::<TesGlobals>()
        .is_none_or(|globals| globals.tes.is_none())
    {
        return Err(StreamError("00443598: 00443550 on a null g_tes"));
    }
    for address in PRELOAD_MODEL_SETTINGS {
        let path = preload_setting(world, address)?;
        if path.first().is_some_and(|&byte| byte != 0) {
            preload_model(world, &path, services.archives.as_deref_mut())?;
        }
    }
    for (slot, address) in PRELOAD_TEXTURE_SETTINGS.into_iter().enumerate() {
        let name = preload_setting(world, address)?;
        if name.first().is_none_or(|&byte| byte == 0) {
            continue;
        }
        let path = [b"Data\\Textures\\".as_slice(), &name].concat();
        if path.len() >= DECAL_PATH_BUFFER {
            return Err(StreamError(
                "00443A44: a decal path past the 260-byte buffer",
            ));
        }
        let texture = create_tes_texture_image(world, &path, false, false, &mut services.textures)?;
        let mut globals = world.resource_mut::<TesGlobals>();
        globals.tes.as_mut().expect("just checked").blood_decals_94[slot] = texture;
    }
    for (index, id) in FLAME_NODE_FORM_IDS.into_iter().enumerate() {
        if let Some(form) = lookup(world, id) {
            let model = world.get::<TesModel>(form).ok_or(StreamError(
                "00443BC4: a FlameNode form with no TESModel at +0x24",
            ))?;
            let text = &model.path_04;
            let length = if text.length_04 == 0xFFFF {
                let data = text
                    .data_00
                    .as_deref()
                    .ok_or(StreamError("00443BD5: strlen of a null +0x28"))?;
                data.iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(data.len())
            } else {
                usize::from(text.length_04)
            };
            if length != 0 {
                let path = text.data_00.clone().unwrap_or_default();
                preload_model(world, &path, services.archives.as_deref_mut())?;
            }
        }
        let mut globals = world.resource_mut::<TesGlobals>();
        globals.flame_node_slots_b35e50[index] = 0;
        globals.trace(0x00B3_5E50 + 4 * index as u32, 4);
    }
    Ok(())
}
