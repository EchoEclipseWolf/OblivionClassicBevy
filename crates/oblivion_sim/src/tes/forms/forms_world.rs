//! Phase 12c: the world forms: TESObjectCELL (CELL 0x58, 004D3210 / 004CD600), TESWorldSpace (WRLD 0xE0, 004F2A10 / 004F1E70 with the cell map 004F1D20), TESRoad (ROAD 0x30, 004E8FA0 / 004E9CA0), TESObjectLAND (LAND 0x28, 004C4C00 / 004C7090) and TESPathGrid (PGRD 0x54, 004E6070 / 004E74E0), with 0044DCF0's arms and the TOFT bodies 004EF030 and 004C9D20. In the C++, forms_world.
//!
//! Evidence: objdump of each address named here; P12-RUN-001's COUNT, SAMPLE and LIST 0C rows, and AC20RAW 35 for the default world. Pointer maps hold real buckets, so a later walk sees the original's order: 006A9060 hashes a key modulo the bucket count, and 00452570 links at the bucket's head.
//! TODO: [ ] vt+0x6C InitItems 004CD960 and 004F2C10, 12d. Terrain, the grid and data_handler_world are Phase 14 and later.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::pointer_list::PointerList;
use oblivion_core::x87;
// IDA: 004E5320 TESPathGrid_GetBucketKey, the +0x44 key.
use oblivion_pathing::path_grid_key;

use super::base_extra_list::{EXTRA_TYPE_0D, ExtraDataList, is_extra_chunk, load_extra_chunk};
use super::form::{FormIdentity, OriginalVtable, set_altered};
use super::form_arms::{
    FormClass, FormLoadCx, default_editor_id, no_init_item, reset_14_components,
    reset_18_components,
};
use super::form_components::{
    ComponentSlot, DataChunkTails, FormRef, TesFullName, load_data_chunk, load_full_name,
    load_texture, reset_form_components,
};
use super::forms_init_world::reference_ni_node;
use super::forms_items::{TesSubSpace, TreeGlobals};
use super::forms_misc::set_initialized;
use super::forms_references::{
    form_flags, parent_cell, reference_is_persistent, refresh_light, set_parent_cell,
};
use super::forms_script::latin1;
use crate::bs::string::BsStringT;
use crate::tes::actor::{Actor, MobileObject, TesObjectRefr};
use crate::tes::data_handler::{
    DataHandler, DataHandlerLists, FormsById, NiTLargeArrayCell, TesTexture,
};
use crate::tes::data_load::{
    DataLoadGlobals, DataLoadServices, cell_arm, construct_object,
    initialize_form_from_current_record, land_arm, path_grid_arm, road_arm, with_files, world_arm,
};
use crate::tes::form::{ModReferences, construct_tes_form, load_form_from_file};
use crate::tes::game_settings::GameSettingCollection;
use crate::tes::tes::{GameThreadLocals, TesGlobals};
use crate::tes::tes_file::{
    GRUP_FOURCC, RecordReader, TesFileId, TesFileServices, TesFileStore, advance_chunk,
    current_chunk_type, form_type_from_record_fourcc, read_current_chunk_data, reserved_form_id,
    seek_record_chunks, set_tes_file_offset, skip_group,
};

/// IDA: the form types of this family.
pub const FORM_TYPE_CELL: u8 = 0x30;
pub const FORM_TYPE_PATH_GRID: u8 = 0x34;
pub const FORM_TYPE_WORLD_SPACE: u8 = 0x35;
pub const FORM_TYPE_LAND: u8 = 0x36;
pub const FORM_TYPE_ROAD: u8 = 0x38;

/// IDA: the most-derived vtables, and each class's secondary vtables, from the constructors named below.
pub const TES_OBJECT_CELL_VTABLE: u32 = 0x00A4_69D4;
pub const TES_OBJECT_CELL_FULL_NAME_VTABLE: u32 = 0x00A4_69C0;
pub const TES_WORLD_SPACE_VTABLE: u32 = 0x00A4_829C;
pub const TES_WORLD_SPACE_FULL_NAME_VTABLE: u32 = 0x00A4_8284;
pub const TES_WORLD_SPACE_TEXTURE_VTABLE: u32 = 0x00A4_8268;
pub const TES_ROAD_VTABLE: u32 = 0x00A4_788C;
pub const TES_OBJECT_LAND_VTABLE: u32 = 0x00A4_6064;
pub const TES_OBJECT_LAND_CHILD_CELL_VTABLE: u32 = 0x00A4_605C;
pub const TES_PATH_GRID_VTABLE: u32 = 0x00A4_7644;
pub const TES_PATH_GRID_CHILD_CELL_VTABLE: u32 = 0x00A4_7638;
/// IDA: the NiTPointerMap vtables each constructor leaves.
pub const CELL_MAP_VTABLE: u32 = 0x00A4_83D0;
pub const LOD_MAP_VTABLE: u32 = 0x00A4_79D4;
pub const REFERENCE_MAP_VTABLE: u32 = 0x00A4_8228;
pub const BOOL_MAP_VTABLE: u32 = 0x00A4_8248;
pub const ROAD_MAP_VTABLE: u32 = 0x00A4_7868;
pub const PATH_GRID_REFERENCE_MAP_VTABLE: u32 = 0x00A4_75F8;
pub const PATH_GRID_POINT_MAP_VTABLE: u32 = 0x00A4_7618;

// IDA: the chunks this family's LoadForms dispatch on.
const EDID: u32 = 0x4449_4445;
const FULL: u32 = 0x4C4C_5546;
const DATA: u32 = 0x4154_4144;
const ICON: u32 = 0x4E4F_4349;
const XCLL: u32 = 0x4C4C_4358;
const XCLC: u32 = 0x434C_4358;
const CNAM: u32 = 0x4D41_4E43;
const WNAM: u32 = 0x4D41_4E57;
const MNAM: u32 = 0x4D41_4E4D;
const SNAM: u32 = 0x4D41_4E53;
const NAM0: u32 = 0x304D_414E;
const NAM2: u32 = 0x324D_414E;
const NAM9: u32 = 0x394D_414E;
const OFST: u32 = 0x5453_464F;
const PGRP: u32 = 0x5052_4750;
const PGRR: u32 = 0x5252_4750;
const PGRI: u32 = 0x4952_4750;
const PGRL: u32 = 0x4C52_4750;
const VHGT: u32 = 0x5447_4856;
const VNML: u32 = 0x4C4D_4E56;
const VCLR: u32 = 0x524C_4356;
const BTXT: u32 = 0x5458_5442;
const ATXT: u32 = 0x5458_5441;
const VTXT: u32 = 0x5458_5456;
const MPCD: u32 = 0x4443_504D;

// IDA: TESForm +0x08 bit 10, 004D7030: a persistent cell.
pub(crate) const FORM_FLAG_PERSISTENT: u32 = 0x400;

/// IDA: the statics this family's constructors count. Absent is before any land, grid or world is made.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct WorldFormGlobals {
    /// 00B35BDC, 004C1840's once flag.
    pub land_statics_built_b35bdc: u8,
    /// 00B35BE4, 004C1858's default TESLandTexture ("Default.DDS").
    pub default_land_texture_b35be4: Option<Entity>,
    /// IDA: N/A how many 004C9530 calls queued a land texture (0043B0D0): a counted boundary.
    pub land_textures_queued: u32,
    /// IDA: N/A how many records took 004E65B0's points.
    pub path_grid_points_loaded: u32,
    /// 00B36080, 004E7DF0's count. Its first call's two debug models (00B36084, 00B36088) are a boundary: only the count.
    pub path_grid_point_count_b36080: u32,
    /// 00B35BE0, 004C4C00's count.
    pub land_count_b35be0: u32,
    /// 00B35F80, 004E6070's count.
    pub path_grid_count_b35f80: u32,
    /// 00B360A0, 004EA450's count.
    pub lod_map_count_b360a0: u32,
    /// 00B36098, 00B3609C: 004EA450's two blocks, by size in bytes. 0 is a null pointer. Nothing reads them yet.
    pub lod_blocks_b36098: [u32; 2],
    /// 00B46064, copied from 00B09B28 by each 004EA450.
    pub lod_b46064: f32,
    /// 00B46068, copied from 00B09B30.
    pub lod_b46068: f32,
    /// 00B3608F, 004EBC00's copy of g_displayLODLand.
    pub display_lod_land_b3608f: u8,
}

/// IDA: the INI statics 004EA450 reads: 00B09B20 iMinGrassSize:Grass, 00B09B28, 00B09B30. Their defaults are the exe's image values, for a world with no INI; `begin_load_selected_files` reads the live ones.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct TerrainLodSettings {
    pub min_grass_size_b09b20: i32,
    pub b09b28: f32,
    pub b09b30: f32,
}

impl Default for TerrainLodSettings {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            min_grass_size_b09b20: 0x50,
            b09b28: f32::from_bits(0x40A0_0000),
            b09b30: f32::from_bits(0x42FA_0000),
        }
    }
}

/// IDA: 004EC650 TerrainLODQuad_ctor(entry) (0x60, FormHeap). Its stores in order: +0x04, +0x2C = 0; +0x48, +0x4C, +0x44 = 0.0; +0x00 = entry; +0x30..+0x3C, +0x14, +0x24, +0x20, +0x40, +0x50..+0x5C = 0; the words +0x0C..+0x12 = 0; +0x08 = 5; then the +0x2C and +0x04 releases, both null. +0x18, +0x1C, +0x28 stay FormHeap bytes (host 0; P14-RUN-001's WE rows read 0). +0x00, the entry that holds this quad, is implied by holding it. Completion: 100%.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainLodQuad {
    /// +0x04 a NiPointer, null until a LOD load.
    pub object_04: Option<Entity>,
    pub state_08: u32,
    pub words_0c: [u16; 4],
    pub field_14: u32,
    /// +0x18, stored by 004EBC00 as (x << 17).
    pub x_18: f32,
    /// +0x1C, stored by 004EBC00 as (y << 17).
    pub y_1c: f32,
    pub field_20: u32,
    pub field_24: u32,
    pub unset_28: u32,
    /// +0x2C a NiPointer, null until a LOD load.
    pub node_2c: Option<Entity>,
    /// +0x30..+0x3C the four children, which a LOD load creates. 004ECB4E tests only the first and calls all four, so one or none here.
    pub children_30: Option<Box<[TerrainLodQuad; 4]>>,
    pub field_40: u32,
    pub extent_44: f32,
    pub field_48: f32,
    pub field_4c: f32,
    pub fields_50: [u32; 4],
}

impl TerrainLodQuad {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            object_04: None,
            state_08: 5,
            words_0c: [0; 4],
            field_14: 0,
            x_18: 0.0,
            y_1c: 0.0,
            field_20: 0,
            field_24: 0,
            unset_28: 0,
            node_2c: None,
            children_30: None,
            field_40: 0,
            extent_44: 0.0,
            field_48: 0.0,
            field_4c: 0.0,
            fields_50: [0; 4],
        }
    }
}

impl Default for TerrainLodQuad {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self::new()
    }
}

/// IDA: the 0x10 map value 004EB1A0 allocates: +0x00 the quad (null when its FormHeap(0x60) fails, which the host doesn't model), +0x04 the map, whose owner is `TesWorldSpace::world_48`, +0x08 and +0x0A the LOD-space x and y.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainLodEntry {
    pub quad_00: TerrainLodQuad,
    pub x_08: i16,
    pub y_0a: i16,
}

/// IDA: N/A what the terrain LOD functions read or write outside the map.
pub struct TerrainLodServices<'a> {
    /// 00B43077 g_displayLODLand, from 00B02D70 bDisplayLODLand at WinMain 0040E92D.
    pub display_lod_land_b43077: u8,
    /// 00B33A04 g_FileFinder vt+4, 00431020 FileFinder_FindFile(path, 0, 0, -1): its status. A null g_FileFinder finds nothing, and the host's answer there is 0 too.
    pub find_file: &'a mut dyn FnMut(&str) -> i32,
    /// 004A7A60 DebugMessage_Warning.
    pub warning: &'a mut dyn FnMut(String),
    /// 0040FEC0 DebugLog_Level2Printf.
    pub debug_log: &'a mut dyn FnMut(String),
    /// IDA: N/A how many FindFile calls 004EB1A0 made.
    pub find_file_calls: u32,
}

/// IDA: 004EF7E0 TESWorldSpace_GetRootLODMap: the +0x7C parent chain to its root; `lea eax, [ecx+38h]` then names the root's map. Answers the root. Completion: 100%.
/// # Errors
/// A null form or an unresolved parent, which the original would read as a pointer.
pub fn world_lod_root(world: &World, form: Entity) -> Result<Entity, GameError> {
    let mut root = form;
    loop {
        let data = world
            .get::<TesWorldSpace>(root)
            .ok_or(GameError::unclassified(
                0x004EF7E0,
                "004EF7E0: +0x7C on a non-WRLD",
            ))?;
        match data.parent_7c {
            FormRef::Resolved(Some(parent)) => root = parent,
            FormRef::Resolved(None) | FormRef::Raw(0) => return Ok(root),
            FormRef::Raw(_) => {
                return Err(GameError::unclassified(
                    0x004EF7E0,
                    "004EF7E0: an unresolved +0x7C parent",
                ));
            }
        }
    }
}

// IDA: 004EB1A0 TerrainLODMap_FindLODFiles (__thiscall on the map), x outer, each of x and y in -32..31: BSStringT_Format(".\Data\Meshes\Landscape\LOD\%i.%02i.%02i.%i.NIF" A47AE8, +0x10's form ID, 32 * x, 32 * y, 32), then FindFile. A hit takes FormHeap(0x10), the quad, and the words; the key is (movsx x << 16) | movzx y. A key already there warns (A47988, with the entry leaking) and otherwise 00452570 and +0x04. Both arms log (A47AB0) with vt+0xD4's text. A failed FormHeap(0x10) faults, not modelled. Completion: 100%.
fn terrain_lod_find_files(
    world: &mut World,
    map_root: Entity,
    services: &mut TerrainLodServices<'_>,
) -> Result<(), GameError> {
    let owner = world
        .get::<TesWorldSpace>(map_root)
        .expect("a world")
        .world_48
        .ok_or(GameError::unclassified(
            0x004EB203,
            "004EB203: a LOD map with a null +0x10",
        ))?;
    let id = world.get::<FormIdentity>(owner).expect("a form").form_id_0c as i32;
    for x in -32_i32..32 {
        for y in -32_i32..32 {
            let path = format!(
                ".\\Data\\Meshes\\Landscape\\LOD\\{id}.{:02}.{:02}.32.NIF",
                x << 5,
                y << 5
            );
            services.find_file_calls += 1;
            if (services.find_file)(&path) == 0 {
                continue;
            }
            let entry = TerrainLodEntry {
                quad_00: TerrainLodQuad::new(),
                x_08: x as i16,
                y_0a: y as i16,
            };
            let key = ((i32::from(entry.x_08) as u32) << 16) | u32::from(entry.y_0a as u16);
            let mut data = world.get_mut::<TesWorldSpace>(map_root).expect("a world");
            if data.lod_38.get(key).is_some() {
                let (warn_x, warn_y) = (i32::from(entry.x_08), i32::from(entry.y_0a));
                (services.warning)(format!(
                    "Unable to add LOD for LOD space ({warn_x}, {warn_y}) because LOD already exists."
                ));
            } else {
                data.lod_38.set_at(key, entry);
            }
            let name = latin1(&super::form_arms::editor_id_of(world, owner));
            (services.debug_log)(format!(
                "Found terrain LOD file for {x}, {y} in worldspace '{name}'"
            ));
        }
    }
    Ok(())
}

/// IDA: 004EBC00 TerrainLODMap_Activate on the root's map: a nonempty map answers 0; otherwise 00B3608F = (00B43077 != 0), 004EB1A0, then per entry in 00452600's order with a quad whose +0x14 is set, in order: +0x08 = 5, the count, (x << 17) and (y << 17) by `fild` and `fstp` dword, +0x18, +0x1C, +0x44 = A47B20 (131072.0). The quad constructor clears +0x14, so that arm can't run right after 004EB1A0: P14-RUN-001 keeps 36 quads in state 5 at position 0. Answers `movsx eax, bp`. Completion: 100%; a null quad skips in the exe and can't exist here.
/// # Errors
/// As `terrain_lod_find_files`.
pub fn terrain_lod_activate(
    world: &mut World,
    map_root: Entity,
    services: &mut TerrainLodServices<'_>,
) -> Result<i16, GameError> {
    let data = world.get::<TesWorldSpace>(map_root).expect("a world");
    if data.lod_38.count_0c != 0 {
        return Ok(0);
    }
    world
        .get_resource_or_init::<WorldFormGlobals>()
        .display_lod_land_b3608f = u8::from(services.display_lod_land_b43077 != 0);
    terrain_lod_find_files(world, map_root, services)?;
    let mut data = world.get_mut::<TesWorldSpace>(map_root).expect("a world");
    let mut count: u16 = 0;
    for (_, entry) in data.lod_38.buckets_08.iter_mut().flatten() {
        let (x, y) = (entry.x_08, entry.y_0a);
        let quad = &mut entry.quad_00;
        if quad.field_14 == 0 {
            continue;
        }
        quad.state_08 = 5;
        count = count.wrapping_add(1);
        quad.x_18 = (i32::from(x) << 17) as f32;
        quad.y_1c = (i32::from(y) << 17) as f32;
        quad.extent_44 = f32::from_bits(0x4800_0000);
    }
    Ok(count as i16)
}

// IDA: 004EC810: state 5 returns; a state other than 4 becomes 4; 00977C50(3), the +0x2C release, 00977C50(2), +0x08 = 5. Completion: 100% but the release, which needs a LOD load.
fn terrain_lod_quad_unload(quad: &mut TerrainLodQuad) -> Result<(), GameError> {
    if quad.state_08 == 5 {
        return Ok(());
    }
    if quad.state_08 != 4 {
        quad.state_08 = 4;
    }
    if quad.node_2c.is_some() {
        return Err(GameError::unclassified(
            0x004EC83D,
            "004EC83D: 004EC810 releases a LOD quad's node",
        ));
    }
    quad.state_08 = 5;
    Ok(())
}

/// IDA: 004ECAE0 TerrainLODQuad_Detach(land LOD): state 2 becomes 4; state 3 detaches +0x2C through the node's vt+0x88 and becomes 2 (`Err`: it needs a LOD load); state 4 is 004EC810; every state then walks the four children if the first is set. Completion: 100% but state 3.
/// # Errors
/// State 3, or 004EC810's stop.
pub fn terrain_lod_quad_detach(quad: &mut TerrainLodQuad) -> Result<(), GameError> {
    match quad.state_08 {
        2 => quad.state_08 = 4,
        3 => {
            return Err(GameError::unclassified(
                0x004ECB15,
                "004ECB15: TES +0x10 vt+0x88 detaches a LOD quad's node",
            ));
        }
        4 => terrain_lod_quad_unload(quad)?,
        _ => {}
    }
    if let Some(children) = &mut quad.children_30 {
        for child in children.iter_mut() {
            terrain_lod_quad_detach(child)?;
        }
    }
    Ok(())
}

/// IDA: 004EA570 -> 004EA080(0): TerrainLODMap_Refresh(update_land) on the root's map: the other arm refreshes TES +0x10 (`Err`, no startup caller); this one walks the entries in 00452600's order, three 004ECAE0 calls on each. Completion: 100% of the 0 arm.
/// # Errors
/// As `terrain_lod_quad_detach`.
pub fn terrain_lod_refresh(
    world: &mut World,
    map_root: Entity,
    update_land: bool,
) -> Result<(), GameError> {
    if update_land {
        return Err(GameError::unclassified(
            0x004EA095,
            "004EA095: 004EA080(1) refreshes TES +0x10",
        ));
    }
    let mut data = world.get_mut::<TesWorldSpace>(map_root).expect("a world");
    for (_, entry) in data.lod_38.buckets_08.iter_mut().flatten() {
        terrain_lod_quad_detach(&mut entry.quad_00)?;
        terrain_lod_quad_detach(&mut entry.quad_00)?;
        terrain_lod_quad_detach(&mut entry.quad_00)?;
    }
    Ok(())
}

/// IDA: a NiTPointerMap's head, with real buckets: +0x00 vtable, +0x04 the bucket count, +0x08 the buckets, +0x0C the count. A chain runs from the newest entry. Completion: 100%.
#[derive(Clone, Debug, PartialEq)]
pub struct PointerMap<V> {
    pub original_vtable: u32,
    pub buckets_04: u32,
    pub buckets_08: Vec<Vec<(u32, V)>>,
    pub count_0c: u32,
}

impl<V> Default for PointerMap<V> {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            original_vtable: 0,
            buckets_04: 0,
            buckets_08: Vec::new(),
            count_0c: 0,
        }
    }
}

impl<V> PointerMap<V> {
    /// IDA: the constructor shape of 004F1D20: +0x04, +0x0C = 0, FormHeap(4 * buckets) cleared with `memset`, the derived vtable. Completion: 100%.
    #[must_use]
    pub fn new(original_vtable: u32, buckets: u32) -> Self {
        let mut buckets_08 = Vec::new();
        buckets_08.resize_with(buckets as usize, Vec::new);
        Self {
            original_vtable,
            buckets_04: buckets,
            buckets_08,
            count_0c: 0,
        }
    }

    // IDA: 006A9060 vt+4: key modulo +0x04.
    const fn bucket_of(&self, key: u32) -> usize {
        (key % self.buckets_04) as usize
    }

    /// IDA: 0055E000: the chain from the bucket's head, compared with 00763E80. Completion: 100%.
    #[must_use]
    pub fn get(&self, key: u32) -> Option<&V> {
        self.buckets_08[self.bucket_of(key)]
            .iter()
            .find(|(entry, _)| *entry == key)
            .map(|(_, value)| value)
    }

    /// IDA: 0055E000, for a store through the value. Completion: 100%.
    pub fn get_mut(&mut self, key: u32) -> Option<&mut V> {
        let bucket = self.bucket_of(key);
        self.buckets_08[bucket]
            .iter_mut()
            .find(|(entry, _)| *entry == key)
            .map(|(_, value)| value)
    }

    /// IDA: 00452570 SetAt: an entry takes the value, otherwise a new one links at the bucket's head and +0x0C counts. Completion: 100%.
    pub fn set_at(&mut self, key: u32, value: V) {
        let bucket = self.bucket_of(key);
        if let Some(slot) = self.buckets_08[bucket]
            .iter_mut()
            .find(|(entry, _)| *entry == key)
        {
            slot.1 = value;
            return;
        }
        self.buckets_08[bucket].insert(0, (key, value));
        self.count_0c = self.count_0c.wrapping_add(1);
    }

    /// IDA: 004524C0 Remove: the entry unlinks and +0x0C counts down. Completion: 100%.
    pub fn remove(&mut self, key: u32) {
        let bucket = self.bucket_of(key);
        if let Some(at) = self.buckets_08[bucket]
            .iter()
            .position(|(entry, _)| *entry == key)
        {
            self.buckets_08[bucket].remove(at);
            self.count_0c = self.count_0c.wrapping_sub(1);
        }
    }
}

/// IDA: 004D3210 TESObjectCELL (0x58): the TESFullName at +0x18 is a `TesFullName`.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesObjectCell {
    /// +0x24: bit 0 interior.
    pub flags_24: u8,
    pub field_25: u8,
    /// +0x26 the process level byte.
    pub process_level_26: u8,
    /// +0x28..+0x3B.
    pub extra_28: ExtraDataList,
    /// +0x3C the block of 004CA710: ten words for an interior, two for an exterior, whose others stay zero. Offsets: +0x00 and +0x04 the grid X and Y (exterior), +0x24 the master record offset (interior).
    pub data_3c: Option<[u32; 10]>,
    /// +0x40 an exterior's land.
    pub land_40: Option<Entity>,
    pub path_grid_44: Option<Entity>,
    /// +0x48 the objects.
    pub objects_48: PointerList<Entity>,
    /// +0x50 an exterior's world (004C9D00).
    pub world_50: Option<Entity>,
    /// +0x50 of an interior: the raw TOFT word 004C9D20 stores. It shares the slot with `world_50`, which only an exterior takes.
    pub toft_50: u32,
    /// +0x54's presence: 1 once 004D58B0 built the node (`CellSceneNode` holds it).
    pub scene_node_present_54: u32,
}

/// IDA: TESObjectCELL +0x54, the NiNode tree 004D58B0 builds.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSceneNode(pub Entity);

/// IDA: 004F2A10 TESWorldSpace (0xE0): `TesFullName` +0x18, `TesTexture` +0x24.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesWorldSpace {
    /// +0x30 the cell map (37, then 701 or 7001 buckets).
    pub cells_30: PointerMap<Entity>,
    pub persistent_cell_34: Option<Entity>,
    /// +0x38..+0x47 the terrain LOD map, with 37 buckets.
    pub lod_38: PointerMap<TerrainLodEntry>,
    /// +0x48 this.
    pub world_48: Option<Entity>,
    /// +0x4C, +0x50, 004EA450's.
    pub lod_4c: [u32; 2],
    /// +0x54 the road.
    pub road_54: Option<Entity>,
    /// +0x58 CNAM, a raw ID until 004F2C10.
    pub climate_58: FormRef,
    pub flags_5c: u8,
    /// +0x60 004F1AC0's map, by grid key, of the references whose subspace reaches a cell.
    pub subspaces_60: Option<PointerMap<PointerList<Entity>>>,
    /// +0x64..+0x73 the references, by grid key.
    pub references_64: PointerMap<PointerList<Entity>>,
    /// +0x74 the others.
    pub unsorted_74: PointerList<Entity>,
    /// +0x7C WNAM, a raw ID until 004F2C10.
    pub parent_7c: FormRef,
    /// +0x80 NAM2, a raw ID until 004F2C10.
    pub water_80: FormRef,
    /// +0x84 MNAM.
    pub map_84: [u8; 0x10],
    /// +0x94 SNAM, a raw ID.
    pub sound_94: u32,
    /// +0x98, +0x9C the NAM0 minimum; +0xA0, +0xA4 the NAM9 maximum.
    pub bounds_98: [f32; 4],
    /// +0xA8 OFST.
    pub offsets_a8: Option<Vec<u32>>,
    /// +0xAC..+0xB8 the master file's NAM0 and NAM9, raw.
    pub master_bounds_ac: [u32; 4],
    /// +0xBC a master file's record offset.
    pub record_offset_bc: u32,
    /// +0xC0 the editor ID.
    pub editor_c0: BsStringT,
    /// +0xC8..+0xD7.
    pub map_c8: PointerMap<u32>,
    /// +0xD8 004F28B0's byte.
    pub lod_loaded_d8: u8,
    pub field_dc: u32,
}

/// IDA: a 0x28 road point (004BEF70): +0x14 the position, as read; +0x20 the connections, as indexes of `TesRoad::points`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoadPoint {
    pub position_14: [f32; 3],
    pub connections_20: PointerList<usize>,
}

/// IDA: 004E8FA0 TESRoad (0x30).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesRoad {
    /// +0x18 the points in the map.
    pub points_18: u32,
    /// +0x1C..+0x2B the grid map: each value a list of `points` indexes.
    pub map_1c: PointerMap<PointerList<usize>>,
    /// +0x2C.
    pub world_2c: Option<Entity>,
    /// IDA: N/A every point 004E9CA0 allocated, inserted or not.
    pub points: Vec<RoadPoint>,
}

/// IDA: 004C6170 / 004C64E0 the 0xA0 land data block at TESObjectLAND +0x24, per quadrant: +0x04 vertices (0xD8C bytes), +0x08 normals (0xD8C), +0x0C colours (0x1210), +0x10 bytes (0x121); +0x18, +0x1C the height range; +0x20 the base textures; +0x30 the 8 additional textures; +0x40 289 x 8 layer alphas; +0x98, +0x9C the grid. +0x00 (the render nodes, 004C5640), +0x14, +0x50 (MPCD) and +0x54.. stay out: nothing in Phase 15 sets them.
#[derive(Clone, Debug, PartialEq)]
pub struct LandData {
    pub vertices_04: [Vec<[f32; 3]>; 4],
    pub normals_08: [Vec<[f32; 3]>; 4],
    pub colors_0c: [Vec<[f32; 4]>; 4],
    pub bytes_10: [Vec<u8>; 4],
    pub min_height_18: f32,
    pub max_height_1c: f32,
    pub base_textures_20: [Option<Entity>; 4],
    pub textures_30: [[Option<Entity>; 8]; 4],
    pub alpha_40: [Vec<[f32; 8]>; 4],
    pub grid_x_98: i32,
    pub grid_y_9c: i32,
}

/// IDA: 004C4C00 TESObjectLAND (0x28).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesObjectLand {
    pub child_cell_vtable_18: u32,
    /// +0x1C DATA.
    pub flags_1c: u32,
    pub cell_20: Option<Entity>,
    /// +0x24 the land data, which only an Active file or TLS +0x186 loads.
    pub data_24: Option<Box<LandData>>,
}

use crate::tes::actor_process::actor_process_manager_refresh;
use crate::tes::ai_cell_moves::{insert_reference_runtime, set_3d_null};
use crate::tes::data_handler_world::{
    CELL, ROAD, cell_child_record, init_item_toggled, invalid_coordinate_warning, next_record,
    save_load_lists_reference, seek_cell_record, seek_world_record, take_created_exterior_cell_id,
};
use crate::tes::form_arms::editor_id_of;
use crate::tes::forms_init::form_type;
use crate::tes::forms_items::set_change_flags;
use crate::tes::forms_references::{reference_start_sound, save_load_active, save_load_flags};
use crate::tes::game_sounds::GameSoundServices;
use crate::tes::os_globals::OsGlobalsData;
use crate::tes::player::player_is_sleeping;
use crate::tes::tes_grid::{TesGridBoundaries, TesGridGlobals, cell_load_priority};
/// IDA: 004E7DF0 TESPathGridPoint (0x2C) on 0067EDC0's search node, ported in `oblivion_pathing`: +0x00..+0x0C the node, +0x10 the flag byte (bit 3, under water; bit 6, in a subspace), +0x14 the position, +0x20 the connections, as indexes of the grid's array.
pub use oblivion_pathing::PathGridPoint;

/// IDA: 004E6070 TESPathGrid (0x54).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesPathGrid {
    pub child_cell_vtable_18: u32,
    pub reference_1c: u32,
    pub cell_20: Option<Entity>,
    /// +0x24 the points' NiTLargeArray-like block (vtable A47590), as 004E65B0 makes it: +0x08 its capacity, +0x0A its size, +0x0C its count are the vector's length.
    pub points_24: Option<Vec<PathGridPoint>>,
    /// +0x28 PGRI's 16-byte entries.
    pub inter_points_28: PointerList<[u8; 16]>,
    /// +0x30 DATA's word.
    pub point_count_30: u16,
    /// +0x34 by reference, the linked points.
    pub references_34: PointerMap<PointerList<usize>>,
    /// +0x44 by 004E5320's key.
    pub points_44: PointerMap<PointerList<usize>>,
}

// IDA: N/A a store to +0x04 and the most-derived vtable.
fn set_form_head(world: &mut World, form: Entity, vtable: u32, form_type: u8) {
    world.get_mut::<OriginalVtable>(form).expect("a form").0 = vtable;
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = form_type;
}

// IDA: 0043F8C0 TES_IsCellAttached(cell, flag), which reads only the cell: a null cell, or a signed +0x26 outside 2..=6, is false; 5 and 6 are true; 2, 3 and 4 are true only with a clear flag (table 0043F8EC). Completion: 100%.
pub(crate) fn cell_attached(world: &World, cell: Option<Entity>, flag: bool) -> bool {
    let Some(cell) = cell else {
        return false;
    };
    match world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .process_level_26 as i8
    {
        5 | 6 => true,
        2..=4 => !flag,
        _ => false,
    }
}

/// IDA: 004C97F0 TESObjectCELL_IsInterior: +0x24 bit 0. Completion: 100%.
#[must_use]
pub fn cell_is_interior(world: &World, cell: Entity) -> bool {
    world.get::<TesObjectCell>(cell).expect("a cell").flags_24 & 1 != 0
}

// IDA: 004BC2E0: a live reference whose base is a subspace, the position inside its radius times vt+0xEC's scale, then inside its box: the offset through 00710250, each half extent (004BC290) times the scale, strictly inside on every axis. All compares are ordered, so a NaN answers false. Completion: 100%.
fn inside_subspace(world: &World, position: [f32; 3], reference: Entity) -> bool {
    if world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08
        & 0x20
        != 0
    {
        return false;
    }
    let refr = world.get::<TesObjectRefr>(reference).expect("a reference");
    let Some(space) = refr
        .base_form_1c
        .and_then(|base| world.get::<TesSubSpace>(base))
    else {
        return false;
    };
    let scale = refr.scale_38;
    let [dx, dy, dz] = [
        position[0] - refr.position_2c[0],
        position[1] - refr.position_2c[1],
        position[2] - refr.position_2c[2],
    ];
    let distance = ((dy * dy + dx * dx) + dz * dz).sqrt();
    // `fcompp`, then `test ah, 41h`: a NaN leaves it false.
    if space.radius_2c * scale > distance {
        // 004BC3BA 004D7AF0: the base is a TESSubSpace (tested above), so the null-base fault can't arise.
        let matrix = super::forms_references::reference_initial_rotation(world, reference)
            .expect("a subspace reference has its base");
        let local = [
            (matrix[0] * dx + matrix[3] * dy) + matrix[6] * dz,
            (matrix[1] * dx + matrix[4] * dy) + matrix[7] * dz,
            (matrix[2] * dx + matrix[5] * dy) + matrix[8] * dz,
        ];
        (0..3).all(|axis| {
            let half = (f32::from(space.extents_24[axis]) * 0.5) * scale;
            half > local[axis] && -half < local[axis]
        })
    } else {
        false
    }
}

// IDA: 004BC4A0: of the list's references holding the position, the one with the smallest +0x2C radius; the first wins a tie. Completion: 100%.
fn subspace_in_list(
    world: &World,
    position: [f32; 3],
    list: impl Iterator<Item = Entity>,
) -> Option<Entity> {
    let mut smallest = f32::MAX;
    let mut found = None;
    for reference in list {
        if inside_subspace(world, position, reference) {
            let base = world
                .get::<TesObjectRefr>(reference)
                .expect("a reference")
                .base_form_1c
                .expect("a subspace");
            let radius = world
                .get::<TesSubSpace>(base)
                .expect("a subspace")
                .radius_2c;
            if smallest > radius {
                smallest = radius;
                found = Some(reference);
            }
        }
    }
    found
}

/// IDA: 004CBBB0: 004BC4A0 over the cell's +0x48 list, under the region lock, a host no-op. Completion: 100%.
#[must_use]
pub fn cell_subspace_at(world: &World, cell: Entity, position: [f32; 3]) -> Option<Entity> {
    let objects = &world.get::<TesObjectCell>(cell).expect("a cell").objects_48;
    subspace_in_list(world, position, objects.items())
}

/// IDA: 004F0600: 004F05A0's list from the world's +0x60 map, its key the same as 004F1AC0's, then 004BC4A0. A null map or a missing key answers null. Completion: 100%.
#[must_use]
pub fn world_subspace_at(world: &World, space: Entity, position: [f32; 3]) -> Option<Entity> {
    let map = world
        .get::<TesWorldSpace>(space)
        .expect("a world")
        .subspaces_60
        .as_ref()?;
    let x = x87::fistp(position[0]) >> 12;
    let y = x87::fistp(position[1]) >> 12;
    let key = ((x as u16 as u32) << 16) | u32::from(y as u16);
    subspace_in_list(world, position, map.get(key)?.items())
}

/// IDA: 004C9A80 the grid X: 0 for an interior or with no block. Completion: 100%.
#[must_use]
pub fn cell_grid_x(world: &World, cell: Entity) -> i32 {
    let data = world.get::<TesObjectCell>(cell).expect("a cell");
    if data.flags_24 & 1 != 0 {
        return 0;
    }
    data.data_3c.map_or(0, |block| block[0] as i32)
}

/// IDA: 004C9AA0 the grid Y: 0 for an interior or with no block. Completion: 100%.
#[must_use]
pub fn cell_grid_y(world: &World, cell: Entity) -> i32 {
    let data = world.get::<TesObjectCell>(cell).expect("a cell");
    if data.flags_24 & 1 != 0 {
        return 0;
    }
    data.data_3c.map_or(0, |block| block[1] as i32)
}

/// IDA: 004C9FB0 whether +0x28 has the type 0x0D extra. Completion: 100%.
#[must_use]
pub fn cell_has_type_0d(world: &World, cell: Entity) -> bool {
    world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .extra_28
        .get(EXTRA_TYPE_0D)
        .is_some()
}

// IDA: 004D3210 TESObjectCELL_Constructor: 0046C460; the TESFullName, built inline, then the cell's vtables; 00422EE0 on +0x28; +0x48, +0x4C, +0x54; +0x24, +0x26, +0x3C, +0x40, +0x44 = 0; +0x04 = 0x30; +0x50, +0x25 = 0; 0046AB80(1). Completion: 100% of the store order.
pub(crate) fn construct_cell(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_tes_form(world)?;
    set_form_head(world, form, TES_OBJECT_CELL_VTABLE, FORM_TYPE_CELL);
    world.entity_mut(form).insert((
        TesFullName {
            original_vtable: TES_OBJECT_CELL_FULL_NAME_VTABLE,
            name_04: BsStringT::default(),
        },
        TesObjectCell {
            extra_28: ExtraDataList {
                original_vtable: crate::tes::actor::EXTRA_DATA_LIST_VTABLE,
                ..ExtraDataList::default()
            },
            ..TesObjectCell::default()
        },
    ));
    set_initialized(world, form, true);
    Ok(form)
}

// IDA: N/A the extra list out of the world for a call that reads the world, and back.
fn with_cell_extras<R>(
    world: &mut World,
    cell: Entity,
    call: impl FnOnce(&World, &mut ExtraDataList) -> R,
) -> R {
    let mut list = std::mem::take(
        &mut world
            .get_mut::<TesObjectCell>(cell)
            .expect("a cell")
            .extra_28,
    );
    let result = call(world, &mut list);
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .extra_28 = list;
    result
}

// IDA: 004CA710: the old block freed; an interior's 0x28 has +0x00..+0x08, +0x14, +0x18, +0x24 = 0, +0x0C, +0x10, +0x20 = 0.0, +0x1C = 1.0; an exterior's 8 are zero. Completion: 100%.
pub(crate) fn allocate_cell_data(cell: &mut TesObjectCell) {
    let mut block = [0_u32; 10];
    if cell.flags_24 & 1 != 0 {
        block[7] = 1.0_f32.to_bits();
    }
    cell.data_3c = Some(block);
}

// IDA: N/A (site unresolved: the XCLL / XCLC arm of 004CD600; calls 00450C20). 00450C20(block, size) into the first `size`
// bytes of the +0x3C block, by 004C98B0's or 004C98A0's test.
fn load_cell_block(
    world: &mut World,
    form: Entity,
    size: usize,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let cell = world.get::<TesObjectCell>(form).expect("a cell");
    let interior = cell.flags_24 & 1 != 0;
    let wanted = if size == 0x28 { interior } else { !interior };
    let Some(mut block) = cell.data_3c.filter(|_| wanted) else {
        return Ok(());
    };
    let mut bytes = [0_u8; 0x28];
    for (target, word) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(&block) {
        *target = word.to_le_bytes();
    }
    cx.file.read_chunk(&mut bytes[..size], size as u32)?;
    for (word, source) in block.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *word = u32::from_le_bytes(*source);
    }
    world
        .get_mut::<TesObjectCell>(form)
        .expect("a cell")
        .data_3c = Some(block);
    Ok(())
}

// IDA: 004CD600 TESObjectCELL vt+0x1C LoadForm. `cmp al, 0x30` (else 0); a master file's record offset; 00451210; the chunks, a do-while over 0044FEA0; 0046AB80(0). FULL, EDID (0x200, vt+0xD8 004C9FC0: a nonempty name goes to 004245E0 on +0x28), XCLC (8) and XCLL (0x28) into the block by their tests, and the same 32 chunks to 004259E0 that REFR sends, where the C++ sends anything starting with X. DATA: 1 byte to +0x24. Its 0xFF compare is on a sign-extended byte and never holds. An exterior takes bit 1; an interior with bit 7 runs 00423FF0(0.0); g_dataHandler +0xCD1 clear drops bit 6; 004CA710; an interior with the offset stores it at block +0x24 (004C9D30). Completion: 100%.
fn load_cell_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_CELL {
        return Ok(false);
    }
    let master_offset = if cx.file.file()?.is_master() {
        cx.file.record_offset()?
    } else {
        0
    };
    initialize_form_from_current_record(world, form, cx)?;
    loop {
        match cx.file.chunk_type()? {
            FULL => {
                let mut name = world
                    .get_mut::<TesFullName>(form)
                    .expect("the class's name");
                load_full_name(&mut name.name_04, cx)?;
            }
            EDID => {
                let text = cx.file.read_text(0x200)?;
                if !text.is_empty() {
                    world
                        .get_mut::<TesObjectCell>(form)
                        .expect("a cell")
                        .extra_28
                        .set_editor_id(Some(&text));
                }
            }
            XCLL => load_cell_block(world, form, 0x28, cx)?,
            XCLC => load_cell_block(world, form, 8, cx)?,
            DATA => {
                let mut flags = [world.get::<TesObjectCell>(form).expect("a cell").flags_24];
                cx.file.read_chunk(&mut flags, 1)?;
                let cd1 = world
                    .get_resource::<DataHandler>()
                    .ok_or(GameError::unclassified(
                        0x004CD716,
                        "004CD716: a null g_dataHandler faults",
                    ))?
                    .bytes_cd0[1];
                let mut cell = world.get_mut::<TesObjectCell>(form).expect("a cell");
                let mut byte = flags[0];
                if byte & 1 == 0 {
                    byte |= 2;
                }
                if byte & 1 != 0 && byte & 0x80 != 0 {
                    cell.extra_28.set_water_height(0);
                }
                if cd1 == 0 {
                    byte &= 0xBF;
                }
                cell.flags_24 = byte;
                allocate_cell_data(&mut cell);
                if byte & 1 != 0 && master_offset != 0 {
                    cell.data_3c.as_mut().expect("just allocated")[9] = master_offset;
                }
            }
            chunk if is_extra_chunk(chunk) => {
                with_cell_extras(world, form, |world, list| load_extra_chunk(world, list, cx))?;
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
    }
    set_initialized(world, form, false);
    Ok(true)
}

// IDA: 004C9780 vt+0x18: 0041E450 on +0x28, then the block freed. Completion: 100%.
fn reset_cell_18(world: &mut World, form: Entity) -> Result<(), GameError> {
    let flags = world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .map(|game| game.fields_14[1]);
    let mut cell = world.get_mut::<TesObjectCell>(form).expect("a cell");
    // 0041E488 reads a null g_saveLoadGame only on a type 2 or 3 entry.
    let keep = match flags {
        Some(flags) => flags & 4 != 0,
        None if cell
            .extra_28
            .entries
            .iter()
            .any(|entry| matches!(entry.type_04, 2 | 3)) =>
        {
            return Err(GameError::unclassified(
                0x0041E488,
                "0041E488: a null g_saveLoadGame faults",
            ));
        }
        None => false,
    };
    cell.extra_28.remove_for_cell_reset(keep);
    cell.data_3c = None;
    Ok(())
}

// IDA: 004CCBC0 vt+0xD4: the type 0x0A text (0041FA30, twice); with none, `Default` for an interior, else what 00B35C0C holds: the GMST sDefaultCellName, "Wilderness" in the image. So an unnamed exterior names the 00B06164 map entry after it, as 0044F2A9 inserts any nonempty name. Completion: 100%.
fn cell_editor_id(world: &World, form: Entity) -> Vec<u8> {
    let cell = world.get::<TesObjectCell>(form).expect("a cell");
    if let Some(text) = cell.extra_28.editor_id() {
        return text.to_vec();
    }
    if cell.flags_24 & 1 != 0 {
        return Vec::new();
    }
    world
        .get_resource::<GameSettingCollection>()
        .and_then(|settings| settings.text_at(0x00B3_5C0C))
        .unwrap_or_else(|| b"Wilderness".to_vec())
}

// IDA: 00446C50 NiTLargeArray_SetAt, DataHandler +0xC0's store at `index`: past +0x0C it grows +0x0C to index + 1; a nonnull value counts +0x10 when the slot was null. 0052ED80 first grows the array to index + +0x14 when index reaches +0x08 (00452910). Completion: 100%.
pub(crate) fn add_interior_cell(array: &mut NiTLargeArrayCell, cell: Entity) {
    let index = array.size_0c;
    if index >= array.capacity_08 {
        // 00452910: the block's other bytes clear; +0x04 holds it.
        let capacity = index.wrapping_add(array.grow_by_14);
        array.capacity_08 = capacity;
        array.cells.resize(capacity as usize, None);
        array.data_04 = 1;
    }
    if index >= array.size_0c {
        array.size_0c = index + 1;
    }
    if array.cells[index as usize].is_none() {
        array.count_10 = array.count_10.wrapping_add(1);
    }
    array.cells[index as usize] = Some(cell);
}

// IDA: 004EFEF0 TESWorldSpace_AddCell(cell): null or interior 0. A persistent cell: an empty +0x34 takes it, and it takes the world (004C9D00); else 0. Otherwise the key movsx(x) << 16 | movzx(y) (from the low words): a cell there warns with its ID, name and the coordinates and gives 0; else 00452570 and 004C9D00. Completion: 100%.
pub(crate) fn world_add_cell(
    world: &mut World,
    space: Entity,
    cell: Entity,
    warn: &mut dyn FnMut(String),
) -> Result<bool, GameError> {
    if cell_is_interior(world, cell) {
        return Ok(false);
    }
    let persistent =
        world.get::<FormIdentity>(cell).expect("a form").flags_08 & FORM_FLAG_PERSISTENT != 0;
    if persistent {
        let mut data = world.get_mut::<TesWorldSpace>(space).expect("a world");
        if data.persistent_cell_34.is_some() {
            return Ok(false);
        }
        data.persistent_cell_34 = Some(cell);
    } else {
        let (x, y) = (cell_grid_x(world, cell), cell_grid_y(world, cell));
        let key = (i32::from(x as i16) << 16) as u32 | u32::from(y as u16);
        let found = world
            .get::<TesWorldSpace>(space)
            .expect("a world")
            .cells_30
            .get(key)
            .copied();
        if let Some(existing) = found {
            let id = world
                .get::<FormIdentity>(existing)
                .expect("a form")
                .form_id_0c;
            let name: String = super::form_arms::editor_id_of(world, existing)
                .iter()
                .map(|&byte| char::from(byte))
                .collect();
            warn(format!(
                "Cell ({id:08X}) {name} already exists at coord ({x}, {y} )."
            ));
            return Ok(false);
        }
        world
            .get_mut::<TesWorldSpace>(space)
            .expect("a world")
            .cells_30
            .set_at(key, cell);
    }
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .world_50 = Some(space);
    Ok(true)
}

// IDA: the current world, 00B33AA0, which when null is DataHandler +0x0C's head item.
pub(crate) fn current_or_first_world(world: &mut World) -> Option<Entity> {
    let current = world.resource::<DataLoadGlobals>().current_world_b33aa0;
    let current = current.or_else(|| {
        world
            .resource::<DataHandlerLists>()
            .world_spaces_0c
            .items()
            .next()
    });
    world.resource_mut::<DataLoadGlobals>().current_world_b33aa0 = current;
    current
}

/// IDA: 004C9D20, the TOFT word of an interior current cell: +0x50 = it. 004C97F0 is tested by 0044F1F3 before and by this body again. Completion: 100%.
pub fn cell_set_toft(world: &mut World, cell: Entity, word: u32) {
    let mut data = world.get_mut::<TesObjectCell>(cell).expect("a cell");
    if data.flags_24 & 1 != 0 {
        data.toft_50 = word;
    }
}

// ---- TESWorldSpace ----

// IDA: 004F1E00 vt+0x14 TESWorldSpace_ResetFormData: +0x7C, +0x5C; 0046B660's reserved test sets +0x5C bit 0 when false; +0x58, +0x80; the TESTexture's string freed; +0x84..+0x90, +0x94; 0046C2E0 over the slots. Completion: 100% of the store order.
fn reset_world_14(world: &mut World, form: Entity) -> Result<(), GameError> {
    let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
    let mut data = world.get_mut::<TesWorldSpace>(form).expect("a world");
    data.parent_7c = FormRef::Raw(0);
    data.flags_5c = 0;
    if !reserved_form_id(id) {
        data.flags_5c |= 1;
    }
    data.climate_58 = FormRef::Raw(0);
    data.water_80 = FormRef::Raw(0);
    world
        .get_mut::<TesTexture>(form)
        .expect("the class's texture")
        .path_04 = BsStringT::default();
    let mut data = world.get_mut::<TesWorldSpace>(form).expect("a world");
    data.map_84 = [0; 0x10];
    data.sound_94 = 0;
    reset_form_components(world, form, WORLD_SLOTS)
}

// IDA: 004EA450 the LOD map's constructor: +0x04 = 0x25, the base vtable, +0x0C = 0, FormHeap(4 * 0x25) and `memset`, the derived vtable, +0x14 = +0x18 = 0. With 00B360A0 clear, n = `idiv` 0x800 by 00B09B20: 00B36098 takes FormHeap(n * n * 0xC) and 00B3609C FormHeap(n * n * 4), each only while null. Then 00B46064 = 00B09B28, 00B360A0 + 1, and 00B46068 = 00B09B30. Completion: 100%; a zero 00B09B20 faults in the exe and answers empty blocks here.
fn construct_lod_map(world: &mut World) -> (PointerMap<TerrainLodEntry>, [u32; 2]) {
    let settings = world
        .get_resource::<TerrainLodSettings>()
        .copied()
        .unwrap_or_default();
    let mut globals = world.get_resource_or_init::<WorldFormGlobals>();
    if globals.lod_map_count_b360a0 == 0 {
        let cells = 0x800_i32
            .checked_div(settings.min_grass_size_b09b20)
            .unwrap_or(0) as u32;
        let cells = cells.wrapping_mul(cells);
        if globals.lod_blocks_b36098[0] == 0 {
            globals.lod_blocks_b36098[0] = cells.wrapping_mul(0xC);
        }
        if globals.lod_blocks_b36098[1] == 0 {
            globals.lod_blocks_b36098[1] = cells.wrapping_mul(4);
        }
    }
    globals.lod_b46064 = settings.b09b28;
    globals.lod_map_count_b360a0 = globals.lod_map_count_b360a0.wrapping_add(1);
    globals.lod_b46068 = settings.b09b30;
    (PointerMap::new(LOD_MAP_VTABLE, 0x25), [0; 2])
}

// IDA: 004F2A10 TESWorldSpace_Constructor: 0046C460; the TESFullName, built inline; 0046FFD0; the three vtables; 004EA450 on +0x38; +0x64's map with 37 buckets; +0x74, +0x78; +0xC0's string; +0xC8's map with 37 buckets; +0x04 = 0x35; +0x48 = this; a new 37-bucket map to +0x30 (004F1D20); 00A32048 (FLT_MAX) to +0x9C, +0x98; +0x34, +0x54; 00A3B888 (-FLT_MAX) to +0xA4, +0xA0; 00B3FC80 to +0xB4, +0xAC, 00B3FC84 to +0xB8, +0xB0; 004F1E00; +0xDC = 7; +0x60, +0xA8, +0xBC = 0. Completion: 100% of the store order.
pub(crate) fn construct_world(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_tes_form(world)?;
    set_form_head(world, form, TES_WORLD_SPACE_VTABLE, FORM_TYPE_WORLD_SPACE);
    let (lod_38, lod_4c) = construct_lod_map(world);
    let trees = world.get_resource::<TreeGlobals>().copied();
    let (x, y) = trees.map_or((0, 0), |globals| (globals.x_b3fc80, globals.y_b3fc84));
    world.entity_mut(form).insert((
        TesFullName {
            original_vtable: TES_WORLD_SPACE_FULL_NAME_VTABLE,
            name_04: BsStringT::default(),
        },
        TesTexture {
            original_vtable: TES_WORLD_SPACE_TEXTURE_VTABLE,
            path_04: BsStringT::default(),
        },
        TesWorldSpace {
            cells_30: PointerMap::new(CELL_MAP_VTABLE, 0x25),
            lod_38,
            world_48: Some(form),
            lod_4c,
            references_64: PointerMap::new(REFERENCE_MAP_VTABLE, 0x25),
            map_c8: PointerMap::new(BOOL_MAP_VTABLE, 0x25),
            bounds_98: [f32::MAX, f32::MAX, -f32::MAX, -f32::MAX],
            master_bounds_ac: [x, y, x, y],
            ..TesWorldSpace::default()
        },
    ));
    reset_world_14(world, form)?;
    let mut data = world.get_mut::<TesWorldSpace>(form).expect("just inserted");
    data.field_dc = 7;
    data.subspaces_60 = None;
    data.offsets_a8 = None;
    data.record_offset_bc = 0;
    Ok(form)
}

// IDA: 004F1F40..004F1F92 and 004F1FDF..004F2031: `fld`, `fcom`, then `fld`/`fstp` through a local, so the kept value is a copy. NAM0's keeps the point unless it's above the bound (`test ah, 0x41`, so a NaN takes the point); NAM9's keeps the point unless it's below (`test ah, 5; jp`, so a NaN takes the point). Completion: 100%.
fn load_world_bound(
    world: &mut World,
    form: Entity,
    at: usize,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let mut bytes = [0_u8; 8];
    cx.file.read_chunk(&mut bytes, 8)?;
    let raw = [
        u32::from_le_bytes(bytes[0..4].try_into().expect("four bytes")),
        u32::from_le_bytes(bytes[4..8].try_into().expect("four bytes")),
    ];
    let master = cx.file.file()?.is_master();
    let mut data = world.get_mut::<TesWorldSpace>(form).expect("a world");
    for (axis, &bits) in raw.iter().enumerate() {
        let old = data.bounds_98[at + axis];
        let point = f32::from_bits(bits);
        let keep_old = if at == 0 { point > old } else { point < old };
        let kept = if keep_old { old.to_bits() } else { bits };
        data.bounds_98[at + axis] = x87::fld_fstp_copy(kept);
    }
    if master {
        // 004F1F9F / 004F203E the raw words.
        if at == 0 {
            data.master_bounds_ac[0] = raw[0];
            data.master_bounds_ac[1] = raw[1];
        } else {
            data.master_bounds_ac[2] = raw[0];
            data.master_bounds_ac[3] = raw[1];
        }
    }
    Ok(())
}

// IDA: 004F1E70 TESWorldSpace vt+0x1C LoadForm. `cmp al, 0x35` (else 0); a master's record offset to +0xBC; 00451210; 0046AB80(0); the chunks. An empty cell map (+0x0C) without +0x5C bit 0 is deleted and a new one takes 7001 buckets for a reserved ID (0046B660), else 701. Returns 1. Completion: 100%.
fn load_world_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_WORLD_SPACE {
        return Ok(false);
    }
    if cx.file.file()?.is_master() {
        let offset = cx.file.record_offset()?;
        world
            .get_mut::<TesWorldSpace>(form)
            .expect("a world")
            .record_offset_bc = offset;
    }
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            NAM9 => load_world_bound(world, form, 2, cx)?,
            NAM2 => {
                let value = cx.file.read_dword(0)?.0;
                world
                    .get_mut::<TesWorldSpace>(form)
                    .expect("a world")
                    .water_80 = FormRef::Raw(value);
            }
            NAM0 => load_world_bound(world, form, 0, cx)?,
            DATA => {
                // 004F2055 0046BDA0(+0x5C, 1).
                let mut byte = [world.get::<TesWorldSpace>(form).expect("a world").flags_5c];
                load_data_chunk(&mut byte, 1, DataChunkTails::default(), cx)?;
                world
                    .get_mut::<TesWorldSpace>(form)
                    .expect("a world")
                    .flags_5c = byte[0];
            }
            FULL => {
                let mut name = world
                    .get_mut::<TesFullName>(form)
                    .expect("the class's name");
                load_full_name(&mut name.name_04, cx)?;
            }
            EDID => {
                // 004F209B: 0x200, vt+0xD8 004EF3C0: 004028D0(+0xC0, name, 0).
                let text = cx.file.read_text(0x200)?;
                world
                    .get_mut::<TesWorldSpace>(form)
                    .expect("a world")
                    .editor_c0
                    .set(Some(&text), 0);
            }
            CNAM => {
                let value = cx.file.read_dword(0)?.0;
                world
                    .get_mut::<TesWorldSpace>(form)
                    .expect("a world")
                    .climate_58 = FormRef::Raw(value);
            }
            SNAM => {
                let value = cx.file.read_dword(0)?.0;
                world
                    .get_mut::<TesWorldSpace>(form)
                    .expect("a world")
                    .sound_94 = value;
            }
            MNAM => {
                let mut bytes = world.get::<TesWorldSpace>(form).expect("a world").map_84;
                cx.file.read_chunk(&mut bytes, 0x10)?;
                world
                    .get_mut::<TesWorldSpace>(form)
                    .expect("a world")
                    .map_84 = bytes;
            }
            WNAM => {
                let value = cx.file.read_dword(0)?.0;
                world
                    .get_mut::<TesWorldSpace>(form)
                    .expect("a world")
                    .parent_7c = FormRef::Raw(value);
            }
            ICON => {
                let mut texture = world
                    .get_mut::<TesTexture>(form)
                    .expect("the class's texture");
                load_texture(&mut texture, cx)?;
            }
            OFST => {
                // 004F2150: an empty chunk does nothing; the old block goes; FormHeap((size >> 2) * 4), read whole with size 0. A size with low bits overruns the block in the original; here the bytes past the last word drop.
                let size = cx.file.chunk_size()?;
                if size != 0 {
                    let mut bytes = vec![0_u8; size as usize];
                    cx.file.read_chunk(&mut bytes, 0)?;
                    let words = bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|word| u32::from_le_bytes(*word))
                        .collect();
                    world
                        .get_mut::<TesWorldSpace>(form)
                        .expect("a world")
                        .offsets_a8 = Some(words);
                }
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    // 004F21D0..004F223D.
    let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
    let mut data = world.get_mut::<TesWorldSpace>(form).expect("a world");
    if data.cells_30.count_0c == 0 && data.flags_5c & 1 == 0 {
        let buckets = if reserved_form_id(id) { 0x1B59 } else { 0x2BD };
        data.cells_30 = PointerMap::new(CELL_MAP_VTABLE, buckets);
    }
    Ok(true)
}

// IDA: 004EF3B0 vt+0xD4: +0xC0, or `Default` "". Completion: 100%.
fn world_editor_id(world: &World, form: Entity) -> Vec<u8> {
    world
        .get::<TesWorldSpace>(form)
        .expect("a world")
        .editor_c0
        .data_00
        .clone()
        .unwrap_or_default()
}

// IDA: 004F00C0: the base's type is in {0x0A, 0x12, 0x17, 0x18, 0x1C, 0x1E, 0x1F, 0x20, 0x25, 0x29}, bytes 004F0100 over 004F00F8, read from the exe. A type past the table is false; a null base faults at 004F00D5. Completion: 100%.
fn reference_is_keyed(world: &World, reference: Entity) -> Result<bool, GameError> {
    let base = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
        .ok_or(GameError::unclassified(
            0x004F00D5,
            "004F00D5: a reference with no base faults",
        ))?;
    Ok(matches!(
        world
            .get::<FormIdentity>(base)
            .expect("a form")
            .form_type_04,
        0x0A | 0x12 | 0x17 | 0x18 | 0x1C | 0x1E | 0x1F | 0x20 | 0x25 | 0x29
    ))
}

// IDA: vt+0x174 and 004EFE40 the grid key: `fld`, `fstp`, `fistp` of the X and Y (round to nearest here), each `sar 0xC`: movsx(x) << 16 | movzx(y). vt+0x174 is 004C8D00 (+0x2C) for a REFR; an Actor's 005F10E0 is +0x2C too unless the process's vt+0x2C0 has 0x800, which a Low, MiddleLow or MiddleHigh process's 0060CF50 never does: a High one's is a host stop. Completion: 100% of the arms a load reaches.
fn reference_grid_key(world: &World, reference: Entity) -> Result<u32, GameError> {
    use crate::tes::actor::MobileObject;
    use crate::tes::actor_process::HighProcess;
    if let Some(process) = world
        .get::<MobileObject>(reference)
        .and_then(|mobile| mobile.process_58)
        && world.get::<HighProcess>(process).is_some()
    {
        return Err(GameError::unclassified(
            0x005F10F1,
            "005F10F1: a HighProcess's vt+0x2C0",
        ));
    }
    let position = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .position_2c;
    let x = x87::fistp(x87::fld_fstp_copy(position[0].to_bits()));
    let y = x87::fistp(x87::fld_fstp_copy(position[1].to_bits()));
    Ok(grid_key(x >> 12, y >> 12))
}

/// IDA: 004F0120 TESWorldSpace_AddReference(reference): a null or flag-0x4000 reference returns; under 00B36100's lock, a type 004F00C0 refuses goes on +0x74 (00446CB0); any other on +0x64's list at its key, made when absent. Completion: 100%.
pub fn world_add_reference(
    world: &mut World,
    space: Entity,
    reference: Entity,
) -> Result<(), GameError> {
    if world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08
        & 0x4000
        != 0
    {
        return Ok(());
    }
    if !reference_is_keyed(world, reference)? {
        world
            .get_mut::<TesWorldSpace>(space)
            .expect("a world")
            .unsorted_74
            .prepend_non_null(reference);
        return Ok(());
    }
    let key = reference_grid_key(world, reference)?;
    let mut data = world.get_mut::<TesWorldSpace>(space).expect("a world");
    if data.references_64.get(key).is_none() {
        data.references_64.set_at(key, PointerList::default());
    }
    data.references_64
        .get_mut(key)
        .expect("just set")
        .prepend_non_null(reference);
    Ok(())
}

/// IDA: 004F01F0 TESWorldSpace_RemoveReference(reference): the same tests; 0065C620 from +0x74, or from the key's list, whose last item takes the list and the key with it (004524C0). The C++ stops here. C++ and disassembly only: no row reaches it. Completion: 100%.
pub fn world_remove_reference(
    world: &mut World,
    space: Entity,
    reference: Entity,
) -> Result<(), GameError> {
    if world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08
        & 0x4000
        != 0
    {
        return Ok(());
    }
    if !reference_is_keyed(world, reference)? {
        world
            .get_mut::<TesWorldSpace>(space)
            .expect("a world")
            .unsorted_74
            .remove_first_matching(reference);
        return Ok(());
    }
    let key = reference_grid_key(world, reference)?;
    let mut data = world.get_mut::<TesWorldSpace>(space).expect("a world");
    let Some(list) = data.references_64.get_mut(key) else {
        return Ok(());
    };
    list.remove_first_matching(reference);
    if list.is_empty() {
        data.references_64.remove(key);
    }
    Ok(())
}

/// IDA: 004EF030, the TOFT word's delta for a current world: a null +0xA8 or a zero delta returns. `ftol` of +0xB0, +0xB4, +0xB8, +0xAC, each `sar 0xC`. Over X from +0xAC to +0xB4 and Y from +0xB0 to +0xB8, 004EEEF0's index (width * (y - minY) + (x - minX), -1 outside the bounds) of a nonzero word adds the delta. An index past the words reads past the block in the original: a host stop. Completion: 100%.
pub fn world_apply_toft_delta(
    world: &mut World,
    space: Entity,
    delta: u32,
) -> Result<(), GameError> {
    let mut data = world.get_mut::<TesWorldSpace>(space).expect("a world");
    let bounds = data.master_bounds_ac;
    let Some(words) = data.offsets_a8.as_mut() else {
        return Ok(());
    };
    if delta == 0 {
        return Ok(());
    }
    let word = |index: usize| x87::ftol(f32::from_bits(bounds[index])) >> 12;
    let (min_x, min_y, max_x, max_y) = (word(0), word(1), word(2), word(3));
    for x in min_x..=max_x {
        for y in min_y..=max_y {
            // 004EEEF0 re-reads the same four bounds, so a loop's X and Y are in them.
            let index = (max_x - min_x + 1)
                .wrapping_mul(y - min_y)
                .wrapping_add(x - min_x);
            if index < 0 {
                continue;
            }
            let slot = words
                .get_mut(index as usize)
                .ok_or(GameError::unclassified(
                    0x004EF0C3,
                    "004EF0C3: an index past the OFST block",
                ))?;
            if *slot != 0 {
                *slot = slot.wrapping_add(delta);
            }
        }
    }
    Ok(())
}

// ---- TESRoad ----

// IDA: 004E8FA0 TESRoad_Constructor: 0046C460; the vtable; the map with 37 buckets; +0x04 = 0x38; +0x2C, +0x18 = 0; 0046C2E0 (no slots). Completion: 100% of the store order.
pub(crate) fn construct_road(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_tes_form(world)?;
    set_form_head(world, form, TES_ROAD_VTABLE, FORM_TYPE_ROAD);
    world.entity_mut(form).insert(TesRoad {
        map_1c: PointerMap::new(ROAD_MAP_VTABLE, 0x25),
        ..TesRoad::default()
    });
    Ok(form)
}

// IDA: the `fld`, `fstp`, `fistp`, `sar 0xC` of 004E8D0A..004E8D37 and 004E9097..004E90C6, then 004EF1D0 with X and Y. Completion: 100%.
fn road_key(position: [f32; 3]) -> u32 {
    let x = x87::fistp(x87::fld_fstp_copy(position[0].to_bits()));
    let y = x87::fistp(x87::fld_fstp_copy(position[1].to_bits()));
    grid_key(x >> 12, y >> 12)
}

// IDA: 0047D810(a, b, 00A379B4 = 2.0): per axis, at 24 bits, `|a - b|` below the tolerance goes on, or a NaN: the first axis's `fcom` and `jp` on `test ah, 0x41`, and the others' `test ah, 1`, all pass an unordered compare, where the C++'s `<` fails it. Completion: 100%.
fn near_point(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(&b).all(|(&a, &b)| {
        let distance = (a - b).abs();
        distance < 2.0 || distance.is_nan()
    })
}

// IDA: 004E8D00: the key's list through 0055E000, from its head while it isn't an empty head, the first point near the position. Completion: 100%.
fn road_find(road: &TesRoad, position: [f32; 3]) -> Option<usize> {
    road.map_1c
        .get(road_key(position))?
        .items()
        .find(|&index| near_point(road.points[index].position_14, position))
}

// IDA: 004E9060: a point near this one takes nothing; otherwise it joins the list at its key (made when absent, 00452570), and +0x18 counts. Completion: 100%.
fn road_insert(road: &mut TesRoad, index: usize) {
    let position = road.points[index].position_14;
    if road_find(road, position).is_some() {
        return;
    }
    let key = road_key(position);
    if road.map_1c.get(key).is_none() {
        road.map_1c.set_at(key, PointerList::default());
    }
    road.map_1c
        .get_mut(key)
        .expect("just set")
        .prepend_non_null(index);
    road.points_18 = road.points_18.wrapping_add(1);
}

// IDA: 004BEFE0(other) on a point: each joins the other's +0x20 unless the two are one or it's already there. Completion: 100%.
fn road_connect(road: &mut TesRoad, first: usize, second: usize) {
    let (mut this, mut other) = (first, second);
    while other != this
        && !road.points[this]
            .connections_20
            .items()
            .any(|item| item == other)
    {
        road.points[this].connections_20.prepend_non_null(other);
        std::mem::swap(&mut this, &mut other);
    }
}

// IDA: N/A a float triple at a chunk's bytes.
fn point_at(bytes: &[u8]) -> [f32; 3] {
    let word = |at: usize| {
        f32::from_bits(u32::from_le_bytes(
            bytes[at..at + 4].try_into().expect("four bytes"),
        ))
    };
    [word(0), word(4), word(8)]
}

// IDA: 004E9CA0 TESRoad vt+0x1C LoadForm. `cmp al, 0x38` (else 0); 00451210; 0046AB80(0); EDID (0x200, vt+0xD8 006FDFF0 keeps nothing) and PGRP: size >> 4 records of 16 bytes, read whole. Each makes a 0x28 point (004BEF70), takes its position (004BEF50, raw) and goes to 004E9060; byte +0x0C counts connections. With any, an advance, and a PGRR chunk: count * 12 bytes, read whole; each record's next connections look up their position (004E8D00), and a found point connects (004BEFE0). Returns 1. Completion: 100%.
fn load_road_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_ROAD {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        if chunk == EDID {
            cx.file.read_text(0x200)?;
        } else if chunk == PGRP {
            let count = cx.file.chunk_size()? >> 4;
            let mut records = vec![0_u8; count as usize * 16];
            cx.file.read_chunk(&mut records, count * 16)?;
            let mut road = world.get_mut::<TesRoad>(form).expect("a road");
            let mut created = Vec::with_capacity(count as usize);
            let mut connections = 0_u32;
            for record in records.as_chunks::<16>().0 {
                let index = road.points.len();
                road.points.push(RoadPoint {
                    position_14: point_at(record),
                    connections_20: PointerList::default(),
                });
                road_insert(&mut road, index);
                created.push(index);
                connections = connections.wrapping_add(u32::from(record[12]));
            }
            if connections != 0 && cx.file.advance_chunk()? && cx.file.chunk_type()? == PGRR {
                let mut links = vec![0_u8; connections as usize * 12];
                cx.file.read_chunk(&mut links, connections * 12)?;
                let mut road = world.get_mut::<TesRoad>(form).expect("a road");
                let mut cursor = 0_usize;
                for (record, &index) in records.as_chunks::<16>().0.iter().zip(&created) {
                    for _ in 0..record[12] {
                        let position = point_at(&links[cursor * 12..]);
                        if let Some(target) = road_find(&road, position) {
                            road_connect(&mut road, index, target);
                        }
                        cursor += 1;
                    }
                }
            }
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    Ok(true)
}

// IDA: 004E9050 vt+0x18, jumping to 004E8C20: every list's points and nodes freed, 007C2A40 the map's entries, +0x18 = 0. The map keeps its buckets. Completion: 100%.
fn reset_road_18(world: &mut World, form: Entity) -> Result<(), GameError> {
    let mut road = world.get_mut::<TesRoad>(form).expect("a road");
    let (vtable, buckets) = (road.map_1c.original_vtable, road.map_1c.buckets_04);
    road.map_1c = PointerMap::new(vtable, buckets);
    road.points.clear();
    road.points_18 = 0;
    Ok(())
}

// ---- TESObjectLAND ----

// IDA: 004C1840 the land statics' data: 004C1858 FormHeap(0x34) 004C93D0 with the path "Default.DDS" (A46028) into 00B35BE4. The quadrant offsets 00B35BA8, 00B35B98 and the 00B35BB8, 00B35BCC, 00B35BD0, 00B35BD8 templates are `land_quadrant_offsets` and the fills of `allocate_land_data`, as they never change. Completion: the data only.
// TODO: [ ] 004C18B5..004C1B0B the texturing properties and strip lists, and 004C1B11's warning (A45FF8) on a zero INI 00B08B44 with the texture coordinates 00B35BD4: terrain drawing, M6.
fn construct_land_statics(world: &mut World) -> Result<(), GameError> {
    let texture = super::forms_misc::construct_land_texture(world)?;
    world
        .get_mut::<TesTexture>(texture)
        .expect("the class's texture")
        .path_04
        .set(Some(b"Default.DDS"), 0);
    world
        .get_resource_or_init::<WorldFormGlobals>()
        .default_land_texture_b35be4 = Some(texture);
    Ok(())
}

// IDA: 004C1CA2..004C1CED 00B35BA8 = ((i % 2) << 11) - 0x800, 00B35B98 = ((i / 2) << 11) - 0x800, by quadrant. Completion: 100%.
const fn land_quadrant_offsets(quadrant: usize) -> (f32, f32) {
    (
        (((quadrant % 2) << 11) as i32 - 0x800) as f32,
        (((quadrant / 2) << 11) as i32 - 0x800) as f32,
    )
}

// IDA: 004C4C00 TESObjectLAND_Constructor: 0046C460; the vtables; 00B35BDC clear runs 004C1840 once; +0x1C, +0x20; +0x04 = 0x36; +0x24; 00B35BE0 counts. Completion: 100% of the object.
pub(crate) fn construct_land(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_tes_form(world)?;
    set_form_head(world, form, TES_OBJECT_LAND_VTABLE, FORM_TYPE_LAND);
    if world
        .get_resource_or_init::<WorldFormGlobals>()
        .land_statics_built_b35bdc
        == 0
    {
        construct_land_statics(world)?;
    }
    let mut globals = world.resource_mut::<WorldFormGlobals>();
    globals.land_statics_built_b35bdc = 1;
    globals.land_count_b35be0 = globals.land_count_b35be0.wrapping_add(1);
    world.entity_mut(form).insert(TesObjectLand {
        child_cell_vtable_18: TES_OBJECT_LAND_CHILD_CELL_VTABLE,
        ..TesObjectLand::default()
    });
    Ok(form)
}

// IDA: 004BFDC0 TESObjectLAND_SetCell(cell): +0x20; a nonnull cell's world (004C9CF0: an exterior's +0x50) with a +0x7C sets +0x1C bit 10, otherwise it clears. Completion: 100%.
pub(crate) fn land_set_cell(world: &mut World, land: Entity, cell: Entity) {
    let data = world.get::<TesObjectCell>(cell).expect("a cell");
    let space = if data.flags_24 & 1 != 0 {
        None
    } else {
        data.world_50
    };
    let parent = space.is_some_and(|space| {
        !world
            .get::<TesWorldSpace>(space)
            .expect("a world")
            .parent_7c
            .is_empty()
    });
    let mut data = world.get_mut::<TesObjectLand>(land).expect("a land");
    data.cell_20 = Some(cell);
    if parent {
        data.flags_1c |= 0x400;
    } else {
        data.flags_1c &= !0x400;
    }
}

/// IDA: 004C64E0 TESObjectLAND_AllocateLandData: an existing block with +0x04 answers false. FormHeap(0xA0) 004C6170: FLT_MAX and -FLT_MAX at +0x18, +0x1C, 00B35BE4 in the four +0x20 slots, the rest null. +0x98, +0x9C the grid of the +0x20 cell (a null cell faults). Each quadrant's templates: vertices x = col * 128 + 00B35BA8, y = row * 128 + 00B35B98, z = -2048 (004C1D00); colours {1, 1, 1, 0}, normals {0, 0, 1}, bytes 1; the zeroed 0x2420 alpha block; eight null textures. Completion: 100%.
/// # Errors
/// A null +0x20 cell.
pub fn allocate_land_data(world: &mut World, land: Entity) -> Result<bool, GameError> {
    let data = world.get::<TesObjectLand>(land).expect("a land");
    if data.data_24.is_some() {
        return Ok(false);
    }
    let cell = data.cell_20.ok_or(GameError::unclassified(
        0x004C655D,
        "004C655D: 004C9A80 on a null +0x20 cell",
    ))?;
    let default_texture = world
        .get_resource::<WorldFormGlobals>()
        .and_then(|globals| globals.default_land_texture_b35be4);
    let vertices = |quadrant: usize| {
        let (x, y) = land_quadrant_offsets(quadrant);
        (0..289_usize)
            .map(|index| {
                let (row, col) = (index / 17, index % 17);
                [(col << 7) as f32 + x, (row << 7) as f32 + y, -2048.0]
            })
            .collect::<Vec<_>>()
    };
    let block = LandData {
        vertices_04: [vertices(0), vertices(1), vertices(2), vertices(3)],
        normals_08: std::array::from_fn(|_| vec![[0.0, 0.0, 1.0]; 289]),
        colors_0c: std::array::from_fn(|_| vec![[1.0, 1.0, 1.0, 0.0]; 289]),
        bytes_10: std::array::from_fn(|_| vec![1; 0x121]),
        min_height_18: f32::MAX,
        max_height_1c: f32::MIN,
        base_textures_20: [default_texture; 4],
        textures_30: [[None; 8]; 4],
        alpha_40: std::array::from_fn(|_| vec![[0.0; 8]; 289]),
        grid_x_98: cell_grid_x(world, cell),
        grid_y_9c: cell_grid_y(world, cell),
    };
    world
        .get_mut::<TesObjectLand>(land)
        .expect("a land")
        .data_24 = Some(Box::new(block));
    Ok(true)
}

// IDA: 004C7090's vertex index into a 33 x 33 chunk: 16 * ((q % 2) + 33 * (q / 2)) + 33 * (v / 17) + v % 17, written as (v / 17) << 4 + origin + v.
const fn land_chunk_index(quadrant: usize, vertex: usize) -> usize {
    16 * ((quadrant % 2) + 33 * (quadrant / 2)) + 16 * (vertex / 17) + vertex
}

// IDA: 004BF020, 004BF040 the block's grid, or the +0x20 cell's. Completion: 100%.
fn land_grid(world: &World, land: Entity) -> (i32, i32) {
    let data = world.get::<TesObjectLand>(land).expect("a land");
    if let Some(block) = &data.data_24 {
        return (block.grid_x_98, block.grid_y_9c);
    }
    data.cell_20.map_or((0, 0), |cell| {
        (cell_grid_x(world, cell), cell_grid_y(world, cell))
    })
}

// IDA: 004C7460 VHGT, in the 24-bit grouping: the base float; each signed byte adds to the running value, and the row's last restarts from the row's first (`[edi-0x80]`). Then per quadrant and vertex 004C7567 `fistp` rounds to an integer, which is times 8 by three doublings and `fild`ed: the C++ multiplied the float, which differs on any fractional base. x and y from `fild` and `fadd`. The range is an else-if: a new minimum skips the maximum. Completion: 100%.
fn load_land_heights(block: &mut LandData, raw: &[u8]) {
    block.min_height_18 = f32::MAX;
    block.max_height_1c = f32::MIN;
    let mut base = f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
    let mut heights = vec![0.0_f32; 1089];
    for n in 1..0x442_usize {
        let value = f32::from(raw[n + 3] as i8) + base;
        heights[n - 1] = value;
        base = if n % 33 == 0 { heights[n - 33] } else { value };
    }
    for quadrant in 0..4 {
        let (offset_x, offset_y) = land_quadrant_offsets(quadrant);
        for vertex in 0..289 {
            let (row, col) = (vertex / 17, vertex % 17);
            let height = heights[land_chunk_index(quadrant, vertex)];
            let z = x87::fistp(height).wrapping_mul(8) as f32;
            block.vertices_04[quadrant][vertex] = [
                (col << 7) as f32 + offset_x,
                (row << 7) as f32 + offset_y,
                z,
            ];
            if block.min_height_18 > z {
                block.min_height_18 = z;
            } else if block.max_height_1c < z {
                block.max_height_1c = z;
            }
        }
    }
}

// IDA: 004C7290 VNML: each signed byte `fild`ed over the qword 127.0 (`fdiv`, 24-bit) and stored, then 0043F350. Completion: 100%.
fn load_land_normals(block: &mut LandData, raw: &[u8]) {
    for quadrant in 0..4 {
        for vertex in 0..289 {
            let at = 3 * land_chunk_index(quadrant, vertex);
            let normal = &mut block.normals_08[quadrant][vertex];
            for axis in 0..3 {
                normal[axis] = f32::from(raw[at + axis] as i8) / 127.0;
            }
            // 0043F350 NiVector3_NormalizeAndGetLength.
            crate::gamebryo::transform::normalize_vector(normal);
        }
    }
}

// IDA: 004C73B0 VCLR: each unsigned byte over the qword 255.0 (`fdiv`, 24-bit), alpha 1.0. Completion: 100%.
fn load_land_colors(block: &mut LandData, raw: &[u8]) {
    for quadrant in 0..4 {
        for vertex in 0..289 {
            let at = 3 * land_chunk_index(quadrant, vertex);
            block.colors_0c[quadrant][vertex] = [
                f32::from(raw[at]) / 255.0,
                f32::from(raw[at + 1]) / 255.0,
                f32::from(raw[at + 2]) / 255.0,
                1.0,
            ];
        }
    }
}

// IDA: 004BF270(quadrant, position, layer, value): the range tests; a value above 0.0 stores, any other stores 0.0 (the second arm also tests the row pointer, which 004C64E0 always sets). Completion: 100%.
fn land_set_alpha(block: &mut LandData, quadrant: u8, position: u16, layer: u16, value: f32) {
    if quadrant >= 4 || position >= 0x121 || layer >= 8 {
        return;
    }
    block.alpha_40[quadrant as usize][position as usize][layer as usize] =
        if value > 0.0 { value } else { 0.0 };
}

// IDA: N/A (site unresolved: the VHGT / VNML / VCLR reads of 004C7090; calls 00450C20). 00450C20(size 0): the whole chunk into the local array. `minimum` is what the arm's loops read. A shorter chunk leaves stale stack, which the host can't model.
fn read_land_chunk(
    cx: &mut FormLoadCx<'_, '_>,
    minimum: usize,
    stop: &'static str,
) -> Result<Vec<u8>, GameError> {
    let size = cx.file.chunk_size()? as usize;
    if size < minimum {
        return Err(GameError::from_legacy_message(stop));
    }
    let mut raw = vec![0_u8; size];
    cx.file.read_chunk(&mut raw, 0)?;
    Ok(raw)
}

// IDA: 004C7090 TESObjectLAND vt+0x1C LoadForm. `cmp al, 0x36` (else 0); +0x0C before 00451210: a changed nonzero one warns. An Active file (0044FAF0) or TLS +0x186 (004C9FF0), with +0x1C bit 10 clear, runs 004C64E0 and takes the data arms. DATA: bit 10 kept over a 4-byte read, bit 3 cleared. VHGT and VNML under DATA bit 0, VCLR under bit 1. BTXT: the mapped ID's TESLandTexture by the quadrant byte, a null warns a46208, else 004C9530. ATXT: a layer past 7 warns a461D0 and takes 7; a zero ID takes 00B35BE4 (a zero 00B35BE4 stores null without 004C9530); a missing one warns a46188; the +0x28 layer and +0x2C quadrant. VTXT with both set: a size off 8 warns a46250; each entry's opacity above 1.0 is over the qword 100.0, then 004BF270; both reset to -1. The end sets +0x1C bit 3 with data, or clears it. Returns 1. Completion: 100% but MPCD. A quadrant byte past 3 overruns the block in the exe: a host stop.
// TODO: [ ] 004C717F MPCD with 004C2230: no file in the data has a plain MPCD; compressed records weren't searched.
fn load_land_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_LAND {
        return Ok(false);
    }
    let previous = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
    initialize_form_from_current_record(world, form, cx)?;
    let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
    if previous != 0 && previous != id {
        let name = cx.file.file()?.name_1c.clone();
        cx.file.services.warn(format!(
            "Potentially duplicate Land ({id:08x}) encountered in file {name}."
        ));
    }
    let active = cx.file.file()?.is_active();
    let wanted = active || world.resource::<GameThreadLocals>().byte_186 != 0;
    let load_data =
        wanted && world.get::<TesObjectLand>(form).expect("a land").flags_1c & 0x400 == 0;
    if load_data {
        allocate_land_data(world, form)?;
    }
    let (mut layer, mut quadrant) = (-1_i32, -1_i32);
    loop {
        let chunk = cx.file.chunk_type()?;
        let flags = world.get::<TesObjectLand>(form).expect("a land").flags_1c;
        match chunk {
            DATA => {
                let keep = flags & 0x400 != 0;
                let (mut value, _) = cx.file.read_dword(flags)?;
                value &= !8;
                if keep {
                    value |= 0x400;
                } else {
                    value &= !0x400;
                }
                world
                    .get_mut::<TesObjectLand>(form)
                    .expect("a land")
                    .flags_1c = value;
            }
            VHGT if load_data && flags & 1 != 0 => {
                let raw = read_land_chunk(cx, 4 + 1089, "004C7471: a VHGT chunk under 1093 bytes")?;
                let mut land = world.get_mut::<TesObjectLand>(form).expect("a land");
                load_land_heights(land.data_24.as_mut().expect("004C64E0 ran"), &raw);
            }
            VNML if load_data && flags & 1 != 0 => {
                let raw = read_land_chunk(cx, 3267, "004C7255: a VNML chunk under 3267 bytes")?;
                let mut land = world.get_mut::<TesObjectLand>(form).expect("a land");
                load_land_normals(land.data_24.as_mut().expect("004C64E0 ran"), &raw);
            }
            VCLR if load_data && flags & 2 != 0 => {
                let raw = read_land_chunk(cx, 3267, "004C7377: a VCLR chunk under 3267 bytes")?;
                let mut land = world.get_mut::<TesObjectLand>(form).expect("a land");
                load_land_colors(land.data_24.as_mut().expect("004C64E0 ran"), &raw);
            }
            BTXT if load_data => {
                let mut raw = [0_u8; 8];
                cx.file.read_chunk(&mut raw, 8)?;
                let id = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
                let file = cx.file.id;
                let id = crate::tes::tes_file::map_file_form_id(world, cx, id, Some(file))?;
                let found = world.resource::<FormsById>().lookup(id).filter(|&texture| {
                    world
                        .get::<super::forms_misc::TesLandTexture>(texture)
                        .is_some()
                });
                let block_index = raw[4];
                if block_index >= 4 {
                    return Err(GameError::unclassified(
                        0x004C777F,
                        "004C777F: a BTXT quadrant past 3",
                    ));
                }
                world
                    .get_mut::<TesObjectLand>(form)
                    .expect("a land")
                    .data_24
                    .as_mut()
                    .expect("004C64E0 ran")
                    .base_textures_20[block_index as usize] = found;
                if found.is_none() {
                    let (x, y) = land_grid(world, form);
                    cx.file.services.warn(format!(
                        "Land ({x}, {y}) unable to find base texture ID ({id:08X}) for block {block_index}."
                    ));
                } else {
                    world
                        .resource_mut::<WorldFormGlobals>()
                        .land_textures_queued += 1;
                }
            }
            ATXT if load_data => {
                let mut raw = [0_u8; 8];
                cx.file.read_chunk(&mut raw, 8)?;
                let raw_id = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
                let block_index = raw[4];
                let mut position = u16::from_le_bytes([raw[6], raw[7]]);
                if position > 7 {
                    let (x, y) = land_grid(world, form);
                    cx.file.services.warn(format!(
                        "Land ({x}, {y}) clamping invalid index {position} for block {block_index}."
                    ));
                    position = 7;
                }
                let found = if raw_id != 0 {
                    let file = cx.file.id;
                    let id = crate::tes::tes_file::map_file_form_id(world, cx, raw_id, Some(file))?;
                    let found = world.resource::<FormsById>().lookup(id).filter(|&texture| {
                        world
                            .get::<super::forms_misc::TesLandTexture>(texture)
                            .is_some()
                    });
                    if found.is_none() {
                        let (x, y) = land_grid(world, form);
                        cx.file.services.warn(format!(
                            "Land ({x}, {y}) unable to find additional texture ID ({id:08X}) for block {block_index}."
                        ));
                    }
                    found
                } else {
                    world
                        .resource::<WorldFormGlobals>()
                        .default_land_texture_b35be4
                };
                if block_index >= 4 {
                    return Err(GameError::unclassified(
                        0x004C793D,
                        "004C793D: an ATXT quadrant past 3",
                    ));
                }
                world
                    .get_mut::<TesObjectLand>(form)
                    .expect("a land")
                    .data_24
                    .as_mut()
                    .expect("004C64E0 ran")
                    .textures_30[block_index as usize][position as usize] = found;
                if found.is_some() {
                    world
                        .resource_mut::<WorldFormGlobals>()
                        .land_textures_queued += 1;
                }
                layer = i32::from(position);
                quadrant = i32::from(block_index);
            }
            VTXT if load_data => {
                if quadrant >= 0 && layer >= 0 {
                    let size = cx.file.chunk_size()?;
                    if size & 7 == 0 {
                        let mut raw = vec![0_u8; size as usize];
                        cx.file.read_chunk(&mut raw, size)?;
                        let mut land = world.get_mut::<TesObjectLand>(form).expect("a land");
                        let block = land.data_24.as_mut().expect("004C64E0 ran");
                        for entry in raw.as_chunks::<8>().0 {
                            let position = u16::from_le_bytes([entry[0], entry[1]]);
                            let mut opacity =
                                f32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);
                            if opacity > 1.0 {
                                opacity /= 100.0;
                            }
                            land_set_alpha(block, quadrant as u8, position, layer as u16, opacity);
                        }
                    } else {
                        let (x, y) = land_grid(world, form);
                        let name = cx.file.file()?.name_1c.clone();
                        cx.file.services.warn(format!(
                            "Land ({x}, {y}) found unrecognized vertex texture data in file {name}."
                        ));
                    }
                }
                layer = -1;
                quadrant = -1;
            }
            MPCD if load_data => {
                return Err(GameError::unclassified(
                    0x004C717F,
                    "004C717F: LAND MPCD (004C2230)",
                ));
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
    }
    let mut land = world.get_mut::<TesObjectLand>(form).expect("a land");
    if load_data {
        land.flags_1c |= 8;
    } else {
        land.flags_1c &= !8;
    }
    Ok(true)
}

/// IDA: 004C46B0 TESObjectLAND_GetHeightRange(out): no block answers {FLT_MAX, -FLT_MAX}. The compares are `fcomp` with qwords equal to the two floats: a +0x18 at FLT_MAX or a +0x1C at -FLT_MAX rebuilds. With DATA bit 0, 004C4630 per quadrant over 0x121 vertices (each its own min and max) folds in by `>` and `<`; otherwise both -2048.0. Completion: 100%, with 004C1DD0's +0x00 render arm out: nothing sets +0x00, and +0x04 is always set.
#[must_use]
pub fn land_height_range(world: &mut World, land: Entity) -> [f32; 2] {
    let mut data = world.get_mut::<TesObjectLand>(land).expect("a land");
    let flags = data.flags_1c;
    let Some(block) = data.data_24.as_mut() else {
        return [f32::MAX, f32::MIN];
    };
    if block.min_height_18 == f32::MAX || block.max_height_1c == f32::MIN {
        if flags & 1 != 0 {
            for quadrant in 0..4 {
                let (mut low, mut high) = (f32::MAX, f32::MIN);
                for vertex in &block.vertices_04[quadrant] {
                    if low > vertex[2] {
                        low = vertex[2];
                    }
                    if high < vertex[2] {
                        high = vertex[2];
                    }
                }
                if block.min_height_18 > low {
                    block.min_height_18 = low;
                }
                if block.max_height_1c < high {
                    block.max_height_1c = high;
                }
            }
        } else {
            block.min_height_18 = -2048.0;
            block.max_height_1c = -2048.0;
        }
    }
    [block.min_height_18, block.max_height_1c]
}

/// IDA: the 0x50 stack block of 004C5B50 that 004C3030 fills and 004C44C0 reads; the offsets are in the names. +0x4E, +0x4F aren't stored.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LandHeightTriangle {
    pub dx_00: f32,
    pub dy_04: f32,
    pub quadrant_local_x_08: f32,
    pub quadrant_local_y_0c: f32,
    pub quadrant_x_10: i32,
    pub quadrant_y_14: i32,
    pub quadrant_18: i32,
    pub vertex_local_x_1c: f32,
    pub vertex_local_y_20: f32,
    pub column_24: i32,
    pub row_28: i32,
    pub quadrant_2c: i32,
    pub vertex_30: [f32; 3],
    pub vertex_3c: i32,
    pub indices_40: [i32; 3],
    pub odd_cell_4c: u8,
    pub far_triangle_4d: u8,
}

/// IDA: 004C1E4F..004C1E76: what 004C1DD0 answers with no vertex block, z from 00A37448.
const LAND_VERTEX_MISSING: [f32; 3] = [0.0, 0.0, -2048.0];

// IDA: 004C1DD0(quadrant, index, out): +0x24 null, or its +0x04 null: {0, 0, -2048} (`fld ds:00A37448` at 004C1E5F). +0x04[quadrant] nonnull: the index's 3 dwords. Otherwise +0x00's render arm, which nothing sets, so zeros. An index past the 0x121 vertices reads past the block in the original: a stop. Completion: 100% without +0x00.
fn land_vertex(block: Option<&LandData>, quadrant: i32, index: i32) -> Result<[f32; 3], GameError> {
    let Some(block) = block else {
        return Ok(LAND_VERTEX_MISSING);
    };
    let vertices = usize::try_from(quadrant)
        .ok()
        .and_then(|quadrant| block.vertices_04.get(quadrant))
        .ok_or(GameError::unclassified(
            0x004C1DE5,
            "004C1DE5: a land quadrant out of range",
        ))?;
    if vertices.is_empty() {
        return Ok(LAND_VERTEX_MISSING);
    }
    usize::try_from(index)
        .ok()
        .and_then(|index| vertices.get(index))
        .copied()
        .ok_or(GameError::unclassified(
            0x004C1DEB,
            "004C1DEB: a land vertex out of range",
        ))
}

// IDA: the __ftol2 of a land coordinate times 1/2048 or 1/128, and the 0x800007FF, 0x8000007F remainder: a nonzero exact multiple of `size`, of the truncated coordinate, takes the lower cell.
fn land_cell_index(local: f32, inverse: f32, size: i32) -> i32 {
    let index = x87::ftol(local * inverse);
    let whole = x87::ftol(local);
    if whole.wrapping_rem(size) == 0 && whole != 0 {
        index.wrapping_sub(1)
    } else {
        index
    }
}

/// IDA: 004C3030 TESObjectLAND_FindHeightTriangle(block, position, recurse), at 24 bits. The origin: `fild` the grid, `fmul` 4096.0, `fstp` float. False with no +0x24, or with neither its +0x04 nor +0x00. False below the origin (`test ah, 0x41`) or past the origin + 4096.0, summed at 24 bits (`test ah, 5`, `jnp`), x then y: an unordered compare passes all four. +0x00, +0x04 the offsets. +0x10, +0x14 the quadrant at 1/2048, +0x18 = x + 2 * y. The quadrant's origin `fiadd`s `q << 11`. +0x08, +0x0C the local offsets. The cell's centre is `fild(grid << 12)`, `fadd` the qword 2048.0. +0x24, +0x28 the cell at 1/128, whose truncated offsets `[esp+0x28]`, `[esp+0x10]` stay. +0x1C = x - float(fiadd(col << 7, origin x)), +0x2C = +0x18, +0x20 likewise. Then with index = row * 17 + col: (row + col) even (004C32EF): +0x44 index, +0x48 index + 18, +0x4C 0, and fy `fcomp` fx: only below takes index + 1 at +0x40 and +0x4D 0; otherwise, an unordered compare too, index + 17 and 1. Odd (004C33A0): fx + fy, rounded, `fcomp` the qword 128.0: +0x44 index + 1, +0x48 index + 17, +0x4C 1; above takes index + 18 and +0x4D 1; otherwise index and 0. +0x3C: with both truncated position coordinates multiples of 128, the grid vertex (each truncated offset `/ 128` toward zero), through `fild`, `fstp` float, `fistp`; otherwise the nearest of the three (`dy*dy + dx*dx` per step, `__CIsqrt`, `fcomp`: the first always, then only a strictly smaller). +0x30: that vertex plus the centre, z `fadd` the qword 0.0. With `recurse`, the answer of 004C3030(block, +0x30, 0). Completion: 100%.
/// # Errors
/// A vertex out of range, which the original reads anyway.
pub fn land_find_height_triangle(
    world: &World,
    land: Entity,
    block: &mut LandHeightTriangle,
    position: [f32; 3],
    recurse: bool,
) -> Result<bool, GameError> {
    let (grid_x, grid_y) = land_grid(world, land);
    let origin_x = (f64::from(grid_x) * 4096.0) as f32;
    let origin_y = (f64::from(grid_y) * 4096.0) as f32;
    let data = world
        .get::<TesObjectLand>(land)
        .expect("a land")
        .data_24
        .as_deref();
    if data.is_none() {
        return Ok(false);
    }
    let [x, y, _] = position;
    if origin_x > x || (origin_x + 4096.0) < x || origin_y > y || (origin_y + 4096.0) < y {
        return Ok(false);
    }
    block.dx_00 = x - origin_x;
    block.dy_04 = y - origin_y;
    const QUADRANT_SCALE: f32 = 0.000_488_281_25;
    const CELL_SCALE: f32 = 0.007_812_5;
    block.quadrant_x_10 = land_cell_index(block.dx_00, QUADRANT_SCALE, 2048);
    block.quadrant_y_14 = land_cell_index(block.dy_04, QUADRANT_SCALE, 2048);
    block.quadrant_18 = block
        .quadrant_x_10
        .wrapping_add(block.quadrant_y_14.wrapping_mul(2));
    let quadrant_x = (f64::from(origin_x) + f64::from(block.quadrant_x_10.wrapping_shl(11))) as f32;
    let quadrant_y = (f64::from(origin_y) + f64::from(block.quadrant_y_14.wrapping_shl(11))) as f32;
    block.quadrant_local_x_08 = x - quadrant_x;
    block.quadrant_local_y_0c = y - quadrant_y;
    let center_x = (f64::from(grid_x.wrapping_shl(12)) + 2048.0) as f32;
    let center_y = (f64::from(grid_y.wrapping_shl(12)) + 2048.0) as f32;
    block.column_24 = land_cell_index(block.quadrant_local_x_08, CELL_SCALE, 128);
    let truncated_x = x87::ftol(block.quadrant_local_x_08);
    block.row_28 = land_cell_index(block.quadrant_local_y_0c, CELL_SCALE, 128);
    let truncated_y = x87::ftol(block.quadrant_local_y_0c);
    let vertex_x = (f64::from(block.column_24.wrapping_shl(7)) + f64::from(quadrant_x)) as f32;
    block.vertex_local_x_1c = x - vertex_x;
    block.quadrant_2c = block.quadrant_18;
    let vertex_y = (f64::from(block.row_28.wrapping_shl(7)) + f64::from(quadrant_y)) as f32;
    block.vertex_local_y_20 = y - vertex_y;
    let (fx, fy) = (block.vertex_local_x_1c, block.vertex_local_y_20);
    let index = block.row_28.wrapping_mul(17).wrapping_add(block.column_24);
    if block.row_28.wrapping_add(block.column_24).wrapping_rem(2) == 0 {
        block.indices_40[2] = index.wrapping_add(18);
        block.odd_cell_4c = 0;
        block.indices_40[1] = index;
        if fy < fx {
            block.far_triangle_4d = 0;
            block.indices_40[0] = index.wrapping_add(1);
        } else {
            block.far_triangle_4d = 1;
            block.indices_40[0] = index.wrapping_add(17);
        }
    } else {
        let sum = fx + fy;
        block.indices_40[1] = index.wrapping_add(1);
        block.indices_40[2] = index.wrapping_add(17);
        block.odd_cell_4c = 1;
        if sum > 128.0 {
            block.far_triangle_4d = 1;
            block.indices_40[0] = index.wrapping_add(18);
        } else {
            block.far_triangle_4d = 0;
            block.indices_40[0] = index;
        }
    }
    if x87::ftol(x).wrapping_rem(128) == 0 && x87::ftol(y).wrapping_rem(128) == 0 {
        let grid_vertex = (truncated_y / 128)
            .wrapping_mul(17)
            .wrapping_add(truncated_x / 128);
        block.vertex_3c = x87::fistp(grid_vertex as f32);
    } else {
        let mut best = 0.0_f32;
        let mut nearest = 0;
        for k in 0..3 {
            let vertex = land_vertex(data, block.quadrant_18, block.indices_40[k])?;
            let delta_x = (center_x + vertex[0]) - x;
            let delta_y = (center_y + vertex[1]) - y;
            let distance = (delta_y * delta_y + delta_x * delta_x).sqrt();
            if k == 0 || best > distance {
                best = distance;
                nearest = k;
            }
        }
        block.vertex_3c = block.indices_40[nearest];
    }
    let vertex = land_vertex(data, block.quadrant_18, block.vertex_3c)?;
    block.vertex_30 = [vertex[0] + center_x, vertex[1] + center_y, vertex[2] + 0.0];
    if recurse {
        let at = block.vertex_30;
        return land_find_height_triangle(world, land, block, at, false);
    }
    Ok(true)
}

/// IDA: 004C44C0 TESObjectLAND_InterpolateHeightTriangle(block, &height), at 24 bits: 004C1DD0 for +0x40, +0x44, +0x48. +0x4C clear: a = (z0 - z1) * 1/128, b = (z2 - z0) * 1/128, each stored; +0x4D clear: (z1 + fx * a) + fy * b; set: (z1 + fy * a) + fx * b. +0x4C set and +0x4D set: c = (z0 - z2) * 1/128, stored; t = (128 - fx) * c; d = (z0 - z1) * 1/128, stored; u = (128 - fy) * d; z0 - (u + t). +0x4C set and +0x4D clear: e = (z1 - z0) * 1/128, stored; p = e * fx + z0; f = (z2 - z0) * 1/128, stored; p + f * fy. Answers true. Completion: 100%.
/// # Errors
/// A vertex out of range, which the original reads anyway.
pub fn land_interpolate_height(
    world: &World,
    land: Entity,
    block: &LandHeightTriangle,
) -> Result<f32, GameError> {
    let data = world
        .get::<TesObjectLand>(land)
        .expect("a land")
        .data_24
        .as_deref();
    let z0 = land_vertex(data, block.quadrant_18, block.indices_40[0])?[2];
    let z1 = land_vertex(data, block.quadrant_18, block.indices_40[1])?[2];
    let z2 = land_vertex(data, block.quadrant_18, block.indices_40[2])?[2];
    const INVERSE: f32 = 0.007_812_5;
    let (fx, fy) = (block.vertex_local_x_1c, block.vertex_local_y_20);
    Ok(if block.odd_cell_4c == 0 {
        let a = (z0 - z1) * INVERSE;
        let b = (z2 - z0) * INVERSE;
        if block.far_triangle_4d == 0 {
            (z1 + fx * a) + fy * b
        } else {
            (z1 + fy * a) + fx * b
        }
    } else if block.far_triangle_4d != 0 {
        let c = (z0 - z2) * INVERSE;
        let t = (128.0 - fx) * c;
        let d = (z0 - z1) * INVERSE;
        let u = (128.0 - fy) * d;
        z0 - (u + t)
    } else {
        let e = (z1 - z0) * INVERSE;
        let p = e * fx + z0;
        let f = (z2 - z0) * INVERSE;
        p + f * fy
    })
}

/// IDA: 004C5B50 TESObjectLAND_GetHeightAtPosition(position, &height): 004C3030(block, position, 0); false stores flt_A37448, -2048.0, and answers false; otherwise 004C44C0's height and true. Completion: 100%.
/// # Errors
/// A vertex out of range, which the original reads anyway.
pub fn land_height_at_position(
    world: &World,
    land: Entity,
    position: [f32; 3],
) -> Result<(bool, f32), GameError> {
    let mut block = LandHeightTriangle::default();
    if !land_find_height_triangle(world, land, &mut block, position, false)? {
        return Ok((false, -2048.0));
    }
    Ok((true, land_interpolate_height(world, land, &block)?))
}

/// IDA: 004CE3C0 TESObjectCELL_GetOrCreateLand: null for an interior or a persistent cell; a missing +0x40 is FormHeap(0x28) 004C4C00, stored, then 004BFDC0. Completion: 100%.
/// # Errors
/// A stop in the constructor.
pub fn cell_get_or_create_land(
    world: &mut World,
    cell: Entity,
) -> Result<Option<Entity>, GameError> {
    let persistent =
        world.get::<FormIdentity>(cell).expect("a form").flags_08 & FORM_FLAG_PERSISTENT != 0;
    let data = world.get::<TesObjectCell>(cell).expect("a cell");
    if data.flags_24 & 1 != 0 || persistent {
        return Ok(None);
    }
    if let Some(land) = data.land_40 {
        return Ok(Some(land));
    }
    let land = construct_land(world)?;
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .land_40 = Some(land);
    land_set_cell(world, land, cell);
    Ok(Some(land))
}

// ---- TESPathGrid ----

// IDA: 004E6070 TESPathGrid_Constructor: 0046C460; the vtables; +0x1C, +0x28, +0x2C; +0x34's and +0x44's maps with 37 buckets; +0x04 = 0x34; +0x1C again, +0x20, +0x24, +0x30; 0046C2E0 passes over type 0x34; 00B35F80 counts. Completion: 100% of the store order.
pub(crate) fn construct_path_grid(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_tes_form(world)?;
    set_form_head(world, form, TES_PATH_GRID_VTABLE, FORM_TYPE_PATH_GRID);
    world.entity_mut(form).insert(TesPathGrid {
        child_cell_vtable_18: TES_PATH_GRID_CHILD_CELL_VTABLE,
        references_34: PointerMap::new(PATH_GRID_REFERENCE_MAP_VTABLE, 0x25),
        points_44: PointerMap::new(PATH_GRID_POINT_MAP_VTABLE, 0x25),
        ..TesPathGrid::default()
    });
    let mut globals = world.get_resource_or_init::<WorldFormGlobals>();
    globals.path_grid_count_b35f80 = globals.path_grid_count_b35f80.wrapping_add(1);
    Ok(form)
}

// IDA: 004E74E0 TESPathGrid vt+0x1C LoadForm. `cmp al, 0x34` (else 0); +0x0C before 00451210: a changed nonzero one warns. Active or TLS +0x186 lets PGRI, PGRL, PGRP and PGRR reach 004E65B0, whose byte is the result; a host stop, where the C++ stops on the gate alone. EDID (0x200, vt+0xD8 006FDFF0 keeps nothing). DATA: 2 bytes into a local (its bytes past an empty chunk are the stack's; host zero), to +0x30. Flag 0x20 clears +0x30. Returns the result, 1 without those chunks. Completion: 100% of the arms a shipped load reaches.
// TODO: [ ] 004E75C7 004E65B0 the points, with Active-file loading (Phase 14).
fn load_path_grid_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_PATH_GRID {
        return Ok(false);
    }
    let previous = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
    initialize_form_from_current_record(world, form, cx)?;
    let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
    if previous != 0 && previous != id {
        let name = cx.file.file()?.name_1c.clone();
        cx.file.services.warn(format!(
            "Potentially duplicate PathGrid ({id:08x}) encountered in file {name}."
        ));
    }
    let load_points =
        cx.file.file()?.is_active() || world.resource::<GameThreadLocals>().byte_186 != 0;
    let mut result = true;
    loop {
        match cx.file.chunk_type()? {
            DATA => {
                let mut bytes = [0_u8; 2];
                cx.file.read_chunk(&mut bytes, 2)?;
                world
                    .get_mut::<TesPathGrid>(form)
                    .expect("a grid")
                    .point_count_30 = u16::from_le_bytes(bytes);
            }
            EDID => {
                cx.file.read_text(0x200)?;
            }
            PGRI | PGRL | PGRP | PGRR if load_points => {
                result = load_path_grid_points(world, form, cx)?;
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
    }
    if world.get::<FormIdentity>(form).expect("a form").flags_08 & 0x20 != 0 {
        world
            .get_mut::<TesPathGrid>(form)
            .expect("a grid")
            .point_count_30 = 0;
    }
    Ok(result)
}

/// IDA: N/A the +0x34 map's key for a reference: the original keys it by the TESObjectREFR pointer, the port by the entity's low bits (a recorded host substitution: lookups agree, bucket order doesn't; no reader walks the buckets). Completion: 100% of the substitution.
#[must_use]
pub fn path_grid_reference_key(reference: Entity) -> u32 {
    reference.to_bits() as u32
}

// IDA: the text 004E67B0 and 004E6ABE build once, for the two warnings. Completion: 100%.
fn path_grid_cell_text(world: &World, form: Entity) -> String {
    let name = |form: Entity| -> String {
        super::form_arms::editor_id_of(world, form)
            .iter()
            .map(|&byte| char::from(byte))
            .collect()
    };
    let Some(cell) = world.get::<TesPathGrid>(form).expect("a grid").cell_20 else {
        return "UNKNOWN".to_owned();
    };
    if cell_is_interior(world, cell) {
        return name(cell);
    }
    let (x, y) = (cell_grid_x(world, cell), cell_grid_y(world, cell));
    let space = world.get::<TesObjectCell>(cell).expect("a cell").world_50;
    match space {
        Some(space) => format!("{} ({x}, {y}) in world {}", name(cell), name(space)),
        None => format!("{} ({x}, {y})", name(cell)),
    }
}

// IDA: 004E65B0 TESPathGrid_LoadPoints(file). A zero +0x30 answers 0 at once. +0x24 made if null. The water height: -FLT_MAX, or 004CACE0 for a cell with +0x24 bit 1. Each chunk to the end: PGRP: 004E4A10 sizes the array to +0x30; each 16-byte entry: 004E7DF0 (00B36080's two debug models, a counted boundary), the position, 0042BB90 into the array; the water above its z sets flag 8; the +0x48 list's or the world's subspace list's 004BC4A0 sets flag 0x40; 004E5380; the connection counts add up. No connections at all answers 1 here. PGRR, after a PGRP: a size other than 2 * the total warns A47720 and reads nothing (the block's bytes are FormHeap's: host zero); each point's count byte runs down over the indexes: one in range, nonnull and not itself goes on the point's +0x20 (00446CB0); answers 1. PGRL: the first word is a reference (46BB20 maps it), a miss warns A47790 and frees, otherwise each next index in range goes through 004E50E0. PGRI: a whole number of 16-byte entries, each a FormHeap block by 00446CB0. Completion: 100%.
fn load_path_grid_points(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    let grid = world.get::<TesPathGrid>(form).expect("a grid");
    let count = grid.point_count_30 as usize;
    if count == 0 {
        return Ok(false);
    }
    let cell = grid.cell_20;
    if grid.points_24.is_none() {
        world
            .get_mut::<TesPathGrid>(form)
            .expect("a grid")
            .points_24 = Some(Vec::new());
    }
    let water = cell.map_or(f32::MIN, |cell| {
        if world.get::<TesObjectCell>(cell).expect("a cell").flags_24 & 2 != 0 {
            crate::tes::tes_grid::cell_water_height(world, cell)
        } else {
            f32::MIN
        }
    });
    let mut result = false;
    let mut raw_points: Option<Vec<u8>> = None;
    let mut total_connections = 0_usize;
    loop {
        let chunk = cx.file.chunk_type()?;
        if chunk == 0 {
            break;
        }
        match chunk {
            PGRP => {
                let mut raw = vec![0_u8; count * 16];
                cx.file.read_chunk(&mut raw, (count * 16) as u32)?;
                let mut points = Vec::with_capacity(count);
                for entry in raw.as_chunks::<16>().0 {
                    let word = |at: usize| {
                        f32::from_le_bytes([entry[at], entry[at + 1], entry[at + 2], entry[at + 3]])
                    };
                    let position = [word(0), word(4), word(8)];
                    let mut point = PathGridPoint {
                        position_14: position,
                        ..PathGridPoint::default()
                    };
                    if water > position[2] {
                        point.flags_10 |= 8;
                    }
                    // 004E69F0..004E6A29: an interior asks its cell, an exterior its world.
                    let in_subspace = cell.is_some_and(|cell| {
                        if cell_is_interior(world, cell) {
                            cell_subspace_at(world, cell, position).is_some()
                        } else {
                            world
                                .get::<TesObjectCell>(cell)
                                .expect("a cell")
                                .world_50
                                .is_some_and(|space| {
                                    world_subspace_at(world, space, position).is_some()
                                })
                        }
                    });
                    if in_subspace {
                        point.flags_10 |= 0x40;
                    }
                    total_connections += entry[12] as usize;
                    points.push(point);
                }
                let mut data = world.get_mut::<TesPathGrid>(form).expect("a grid");
                for (index, point) in points.iter().enumerate() {
                    let key = path_grid_key(point.position_14);
                    if data.points_44.buckets_04 != 0 {
                        if data.points_44.get(key).is_none() {
                            data.points_44.set_at(key, PointerList::default());
                        }
                        data.points_44
                            .get_mut(key)
                            .expect("just set")
                            .prepend_non_null(index);
                    }
                }
                data.points_24 = Some(points);
                raw_points = Some(raw);
                let mut globals = world.get_resource_or_init::<WorldFormGlobals>();
                globals.path_grid_point_count_b36080 = globals
                    .path_grid_point_count_b36080
                    .wrapping_add(count as u32);
                if total_connections == 0 {
                    result = true;
                }
            }
            PGRR if raw_points.is_some() => {
                let size = cx.file.chunk_size()? as usize;
                let mut indexes = vec![0_u8; total_connections * 2];
                if size == total_connections * 2 {
                    cx.file.read_chunk(&mut indexes, size as u32)?;
                } else {
                    let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
                    let text = path_grid_cell_text(world, form);
                    cx.file.services.warn(format!(
                        "PathGrid ({id:08X}) in cell {text} found different connection data than it was expecting:\r\n\tExpected: {total_connections}\t Found: {}",
                        size / 2
                    ));
                }
                let raw = raw_points.as_ref().expect("guarded");
                let mut data = world.get_mut::<TesPathGrid>(form).expect("a grid");
                let points = data.points_24.as_mut().expect("PGRP made it");
                let mut at = 0;
                for (index, point) in points.iter_mut().enumerate().take(count) {
                    for _ in 0..raw[index * 16 + 12] {
                        let other =
                            u16::from_le_bytes([indexes[at * 2], indexes[at * 2 + 1]]) as usize;
                        at += 1;
                        if other < count && other != index {
                            point.connections_20.prepend_non_null(other);
                        }
                    }
                }
                result = true;
            }
            PGRL => {
                let size = cx.file.chunk_size()? as usize;
                if size != 0 {
                    let mut raw = vec![0_u8; size];
                    cx.file.read_chunk(&mut raw, 0)?;
                    let words: Vec<u32> = raw
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|bytes| u32::from_le_bytes(*bytes))
                        .collect();
                    if let Some(&first) = words.first() {
                        let file = cx.file.id;
                        let id =
                            crate::tes::tes_file::map_file_form_id(world, cx, first, Some(file))?;
                        let found = world
                            .resource::<FormsById>()
                            .lookup(id)
                            .filter(|&reference| world.get::<TesObjectRefr>(reference).is_some());
                        if let Some(reference) = found {
                            let mut data = world.get_mut::<TesPathGrid>(form).expect("a grid");
                            for &index in &words[1..] {
                                let exists = data.points_24.is_some()
                                    && (index as usize) < count
                                    && data.points_24.as_ref().expect("tested").len()
                                        > index as usize;
                                if !exists {
                                    continue;
                                }
                                let key = path_grid_reference_key(reference);
                                if data.references_34.get(key).is_none() {
                                    data.references_34.set_at(key, PointerList::default());
                                }
                                let list = data.references_34.get_mut(key).expect("just set");
                                if !list.items().any(|item| item == index as usize) {
                                    list.prepend_non_null(index as usize);
                                }
                            }
                        } else {
                            let text = path_grid_cell_text(world, form);
                            cx.file.services.warn(format!(
                                "Could not find reference ({id:08X}) for linked points in pathgrid for cell {text}."
                            ));
                        }
                    }
                }
            }
            PGRI => {
                let size = cx.file.chunk_size()? as usize;
                if size.is_multiple_of(16) {
                    let mut raw = vec![0_u8; size];
                    cx.file.read_chunk(&mut raw, size as u32)?;
                    let mut data = world.get_mut::<TesPathGrid>(form).expect("a grid");
                    for entry in raw.as_chunks::<16>().0 {
                        data.inter_points_28.prepend_non_null(*entry);
                    }
                }
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
    }
    world
        .get_resource_or_init::<WorldFormGlobals>()
        .path_grid_points_loaded += 1;
    Ok(result)
}

// IDA: 004E4BD0 vt+0x14: a set +0x1C is released, a host stop; +0x20, +0x24, +0x30; 0046C2E0 passes over the type. The C++ agrees. Completion: 100% of the control flow.
fn reset_path_grid_14(world: &mut World, form: Entity) -> Result<(), GameError> {
    let mut grid = world.get_mut::<TesPathGrid>(form).expect("a grid");
    if grid.reference_1c != 0 {
        return Err(GameError::unclassified(
            0x004E4BDE,
            "004E4BDE: releasing a path grid's +0x1C",
        ));
    }
    grid.cell_20 = None;
    grid.points_24 = None;
    grid.point_count_30 = 0;
    Ok(())
}

// IDA: 004E74B0 vt+0x18: 004E5400, 004E6CA0, 004E4ED0, 004E5280 free the points, gated on +0x1C, +0x24, +0x28 and +0x2C, none of which a shipped load sets. Then 0046C2F0 passes over the type. Only the maps' counts could tell. Completion: 100% of the arms a shipped load reaches.
// TODO: [ ] 004E5400, 004E6CA0, 004E4ED0, 004E5280, with 004E65B0.
fn reset_path_grid_18(world: &mut World, form: Entity) -> Result<(), GameError> {
    let grid = world.get::<TesPathGrid>(form).expect("a grid");
    if grid.reference_1c != 0 || grid.points_24.is_some() || !grid.inter_points_28.is_empty() {
        return Err(GameError::unclassified(
            0x004E74B3,
            "004E74B3: freeing a path grid's points",
        ));
    }
    Ok(())
}

// IDA: 00466E80's slots for TESWorldSpace: its TESFullName and TESTexture.
const WORLD_SLOTS: &[ComponentSlot] = &[ComponentSlot::FullName, ComponentSlot::Texture];

/// IDA: this family's classes. vt+0x6C InitItems 004CD960 and 004F2C10 are 12d's; the others' is 0060D0A0.
pub const CLASSES: &[FormClass] = &[
    FormClass {
        form_type: FORM_TYPE_CELL,
        vtable: TES_OBJECT_CELL_VTABLE,
        construct: construct_cell,
        load_form: load_cell_form,
        reset_14: reset_14_components,
        reset_18: reset_cell_18,
        editor_id: cell_editor_id,
        init_item: super::forms_init_world::init_cell,
        slots: &[ComponentSlot::FullName],
        special_arm: Some(cell_arm),
    },
    FormClass {
        form_type: FORM_TYPE_PATH_GRID,
        vtable: TES_PATH_GRID_VTABLE,
        construct: construct_path_grid,
        load_form: load_path_grid_form,
        reset_14: reset_path_grid_14,
        reset_18: reset_path_grid_18,
        editor_id: default_editor_id,
        init_item: no_init_item,
        slots: &[],
        special_arm: Some(path_grid_arm),
    },
    FormClass {
        form_type: FORM_TYPE_WORLD_SPACE,
        vtable: TES_WORLD_SPACE_VTABLE,
        construct: construct_world,
        load_form: load_world_form,
        reset_14: reset_world_14,
        reset_18: reset_18_components,
        editor_id: world_editor_id,
        init_item: super::forms_init_world::init_world_space,
        slots: WORLD_SLOTS,
        special_arm: Some(world_arm),
    },
    FormClass {
        form_type: FORM_TYPE_LAND,
        vtable: TES_OBJECT_LAND_VTABLE,
        construct: construct_land,
        load_form: load_land_form,
        reset_14: reset_14_components,
        reset_18: reset_18_components,
        editor_id: default_editor_id,
        init_item: no_init_item,
        slots: &[],
        special_arm: Some(land_arm),
    },
    FormClass {
        form_type: FORM_TYPE_ROAD,
        vtable: TES_ROAD_VTABLE,
        construct: construct_road,
        load_form: load_road_form,
        reset_14: reset_14_components,
        reset_18: reset_road_18,
        editor_id: default_editor_id,
        init_item: no_init_item,
        slots: &[],
        special_arm: Some(road_arm),
    },
];

/// IDA: 00B35C80 the cell region lock manager: a NiTMap of TESObjectCELL* to {thread ID, count} under its +0x80 critical section. A host `HashMap`, only looked up: nothing iterates it. Completion: 100% of the reached arms.
#[derive(Resource, Debug, Default, PartialEq, Eq)]
pub struct CellRegionLocks {
    pub entries: std::collections::HashMap<Entity, (std::thread::ThreadId, u32)>,
}

impl CellRegionLocks {
    /// IDA: 00496EA0 CellRegionLock_Acquire(cell): under the lock, 00496DF0's lookup: no entry stores {thread, 1}; this thread's entry stores count + 1. Another thread's goes to WaitForSingleObject (00498EE0) and tries again: a host stop, as only the main thread takes these before Phase 15. Completion: 100% of the reached arms.
    /// # Errors
    /// Another thread's entry.
    pub fn acquire(&mut self, cell: Entity) -> Result<(), GameError> {
        let thread = std::thread::current().id();
        match self.entries.get_mut(&cell) {
            None => {
                self.entries.insert(cell, (thread, 1));
                Ok(())
            }
            Some(entry) if entry.0 == thread => {
                entry.1 += 1;
                Ok(())
            }
            Some(_) => Err(GameError::unclassified(
                0x00496F31,
                "00496F31: a cell region lock another thread holds (00498EE0)",
            )),
        }
    }

    /// IDA: 00496F50 CellRegionLock_Release(cell): no entry does nothing; a count reaching 0 removes it (004524C0); otherwise it stores count - 1, with no thread test. Completion: 100%.
    pub fn release(&mut self, cell: Entity) {
        if let Some(entry) = self.entries.get_mut(&cell) {
            entry.1 -= 1;
            if entry.1 == 0 {
                self.entries.remove(&cell);
            }
        }
    }
}

// IDA: N/A 00496EA0 CellRegionLock_Acquire on 00B35C80 (`CellRegionLocks::acquire`), the resource made on first use.
pub(crate) fn cell_region_lock(world: &mut World, cell: Entity) -> Result<(), GameError> {
    world
        .get_resource_or_init::<CellRegionLocks>()
        .acquire(cell)
}

// IDA: N/A 00496F50 CellRegionLock_Release on 00B35C80 (`CellRegionLocks::release`).
pub(crate) fn cell_region_unlock(world: &mut World, cell: Entity) {
    world.resource_mut::<CellRegionLocks>().release(cell);
}

/// IDA: 004CCBF0 TESObjectCELL vt+0x90(1): 0046C640; an altered exterior cell outside a load passes it to its world. Completion: 100%.
pub fn cell_set_altered(world: &mut World, cell: Entity) {
    set_altered(world, cell);
    let flags = form_flags(world, cell);
    let data = world.get::<TesObjectCell>(cell).expect("a cell");
    let space = data.world_50;
    if flags & 0x4000 == 0
        && flags & 2 != 0
        && data.flags_24 & 1 == 0
        && world.resource::<GameThreadLocals>().byte_184 == 0
        && let Some(space) = space
    {
        set_altered(world, space);
    }
}

/// IDA: 004CECD0 TESObjectCELL_RemoveReference(reference): the removal from +0x48 (0065C620) between CellRegionLock_Acquire
/// (004CECE6) and _Release (004CECFA), which no TLS test guards; a persistent cell (flag 0x400) clears the 0x11 extra
/// (004CED0D) and, for an exterior with a world, the world's entry (004CED20); otherwise vt+0x194(null) (004CED34), and a
/// reference with neither flag 0x4000 nor 004DB4A0 runs the cell's vt+0x90(1) (004CED65, `cell_set_altered`) when
/// DataHandler +0xCD4 is clear: g_dataHandler is read with no null test (004CED4B), so a null one faults (host stop).
/// Completion: 100%.
/// # Errors
/// A null DataHandler, another thread's lock, and what the world's removal and vt+0x194 raise.
pub fn cell_remove_reference(
    world: &mut World,
    cell: Entity,
    reference: Entity,
) -> Result<(), GameError> {
    cell_region_lock(world, cell)?;
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .remove_first_matching(reference);
    cell_region_unlock(world, cell);
    if form_flags(world, cell) & 0x400 != 0 {
        world
            .get_mut::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44
            .set_persistent_cell(None);
        let data = world.get::<TesObjectCell>(cell).expect("a cell");
        if data.flags_24 & 1 == 0
            && let Some(space) = data.world_50
        {
            world_remove_reference(world, space, reference)?;
        }
        return Ok(());
    }
    set_parent_cell(world, reference, None)?;
    if form_flags(world, reference) & 0x4000 == 0 && !reference_is_persistent(world, reference)? {
        let handler = world
            .get_resource::<DataHandler>()
            .ok_or(GameError::unclassified(
                0x004CED4B,
                "004CED4B: a null DataHandler faults",
            ))?;
        if handler.bytes_cd0[4] == 0 {
            cell_set_altered(world, cell);
        }
    }
    Ok(())
}

/// IDA: N/A what `cell_insert_reference_head` leaves its caller of 004D35D0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum CellInsertRest {
    /// The function returned (004D3664, 004D38DF) or ended in 004D38D4's 004D80C0(0).
    Done,
    /// None of 004D3731, 004D3742 and 004D374D jumped: 004D3753..004D38D2 are still to run
    /// (`ai_cell_moves::insert_reference_attached_arms`).
    AttachedArms,
}

/// IDA: 004D35D0 TESObjectCELL_InsertPlayerAndRefresh(reference), from its entry to 004D374D with the tail at 004D38D4:
/// vt+0x170, null returns. A persistent cell (flag 0x400): the prepend to +0x48 between CellRegionLock_Acquire (004D3603) and
/// _Release (004D3617), 004247B0(cell), an exterior one with a world 004F0120, and outside a load (TLS +0x184 clear) the
/// cell's vt+0x90(1); it returns without 004D80C0. Any other cell: 006ECC80's previous cell runs 004CECD0; the prepend and
/// vt+0x194(cell) under the lock (004D3680..004D36A1); without flag 0x800 a non-null vt+0x154 on a reference that isn't a
/// MobileObject (the cast at 004D36CF) runs vt+0x148, a host stop; without flag 0x4000, 004DB4A0 false and TLS +0x184
/// clear run the cell's vt+0x90(1) (004D3724). Then TLS +0x184 set, flag 0x4000 or flag 0x800 ends in 004D80C0(0).
/// Otherwise the attached arms follow: they call the cell reference loop and the reference load, which are above this
/// file, so this function answers `AttachedArms` and its caller runs them. Completion: 100% of this range with no 3D.
/// TODO: [ ] 004D36DB vt+0x148 for a plain reference with 3D.
/// # Errors
/// The stop above, another thread's lock, and what 004CECD0, 004F0120, vt+0x194 and 004D80C0 raise.
pub fn cell_insert_reference_head(
    world: &mut World,
    cell: Entity,
    reference: Entity,
) -> Result<CellInsertRest, GameError> {
    if world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
        .is_none()
    {
        return Ok(CellInsertRest::Done);
    }
    let loading = world.resource::<GameThreadLocals>().byte_184 != 0;
    if form_flags(world, cell) & 0x400 != 0 {
        // 004D35FD..004D3617.
        cell_region_lock(world, cell)?;
        world
            .get_mut::<TesObjectCell>(cell)
            .expect("a cell")
            .objects_48
            .prepend_non_null(reference);
        cell_region_unlock(world, cell);
        world
            .get_mut::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44
            .set_persistent_cell(Some(cell));
        let data = world.get::<TesObjectCell>(cell).expect("a cell");
        if data.flags_24 & 1 == 0
            && let Some(space) = data.world_50
        {
            world_add_reference(world, space, reference)?;
        }
        // 004D3638..004D3660.
        if !loading {
            cell_set_altered(world, cell);
        }
        return Ok(CellInsertRest::Done);
    }
    // 004D3667..004D3675.
    if let Some(previous) = parent_cell(world, reference) {
        cell_remove_reference(world, previous, reference)?;
    }
    // 004D367A..004D36A1.
    cell_region_lock(world, cell)?;
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .prepend_non_null(reference);
    set_parent_cell(world, reference, Some(cell))?;
    cell_region_unlock(world, cell);
    // 004D36A6..004D36E5: the cast's target is B05538, MobileObject.
    if form_flags(world, reference) & 0x800 == 0
        && reference_ni_node(world, reference).is_some()
        && world.get::<MobileObject>(reference).is_none()
    {
        return Err(GameError::unclassified(
            0x004D36DB,
            "004D36DB: vt+0x148 with loaded 3D",
        ));
    }
    // 004D36E7..004D3724.
    if form_flags(world, reference) & 0x4000 == 0
        && !reference_is_persistent(world, reference)?
        && !loading
    {
        cell_set_altered(world, cell);
    }
    // 004D3726..004D374D.
    let flags = form_flags(world, reference);
    if loading || flags & 0x4000 != 0 || flags & 0x800 != 0 {
        // 004D38D4.
        refresh_light(world, reference)?;
        return Ok(CellInsertRest::Done);
    }
    Ok(CellInsertRest::AttachedArms)
}

/// IDA: 004CAD00 TESObjectCELL_ResolveMusicType(cell, null) for an interior: under CellRegionLock (00496EA0/00496F50, an
/// uncontended pair here), 00B35E10's first-use bit sets the cached position (00B35E04) to zero; the cell's ExtraCellMusicType
/// (`0041F9C0`, extra 0x0B's +0x0C, XCMT's byte) answers when nonzero or when the cell is an interior (+0x24 bit 0). Completion:
/// 100% of an interior; an exterior (the region list's type-7 data and the worldspace's fallback) is a stop.
/// # Errors
/// An exterior cell.
pub fn cell_resolve_music_type(world: &World, cell: Entity) -> Result<u16, GameError> {
    let data = world
        .get::<TesObjectCell>(cell)
        .ok_or(GameError::unclassified(0x004CAD00, "004CAD00: not a cell"))?;
    let music = data
        .extra_28
        .get(super::base_extra_list::EXTRA_CELL_MUSIC_TYPE)
        .map_or(0, |entry| match entry.data {
            super::base_extra_list::ExtraData::Byte(value) => i32::from(value as i8),
            _ => 0,
        });
    if music != 0 || data.flags_24 & 1 != 0 {
        return Ok(music as u16);
    }
    Err(GameError::unclassified(
        0x004CAD5E,
        "004CAD5E: an exterior cell's region music",
    ))
}

#[cfg(test)]
mod subspace_tests {
    use super::*;

    fn subspace(world: &mut World, position: [f32; 3], rotation: [f32; 3], scale: f32) -> Entity {
        let base = world
            .spawn(TesSubSpace {
                extents_24: [200, 100, 100],
                radius_2c: 150.0,
            })
            .id();
        world
            .spawn((
                FormIdentity {
                    form_type_04: 0x31,
                    flags_08: 0,
                    form_id_0c: 1,
                },
                TesObjectRefr {
                    base_form_1c: Some(base),
                    position_2c: position,
                    rotation_20: rotation,
                    scale_38: scale,
                    ..TesObjectRefr::default()
                },
            ))
            .id()
    }

    // 004BC2E0: strictly inside the radius and the half extents, the box turned by 0070FDD0 and scaled.
    #[test]
    fn a_position_is_inside_a_subspace_box_as_it_turns() {
        let mut world = World::new();
        let plain = subspace(&mut world, [10.0, 0.0, 0.0], [0.0; 3], 1.0);
        assert!(inside_subspace(&world, [109.0, 0.0, 0.0], plain));
        assert!(
            !inside_subspace(&world, [110.0, 0.0, 0.0], plain),
            "the edge is out"
        );
        assert!(!inside_subspace(&world, [10.0, 50.0, 0.0], plain));
        let turned = subspace(
            &mut world,
            [0.0; 3],
            [0.0, 0.0, std::f32::consts::FRAC_PI_2],
            1.0,
        );
        assert!(inside_subspace(&world, [0.0, 90.0, 0.0], turned));
        assert!(!inside_subspace(&world, [90.0, 0.0, 0.0], turned));
        let scaled = subspace(&mut world, [0.0; 3], [0.0; 3], 2.0);
        assert!(inside_subspace(&world, [0.0, 90.0, 0.0], scaled));
        world.get_mut::<FormIdentity>(plain).unwrap().flags_08 = 0x20;
        assert!(!inside_subspace(&world, [50.0, 0.0, 0.0], plain), "deleted");
    }

    // 004BC4A0: the smallest radius holding the position, the first on a tie.
    #[test]
    fn the_smallest_subspace_wins_and_the_first_on_a_tie() {
        let mut world = World::new();
        let first = subspace(&mut world, [0.0; 3], [0.0; 3], 1.0);
        let second = subspace(&mut world, [0.0; 3], [0.0; 3], 1.0);
        let small = subspace(&mut world, [0.0; 3], [0.0; 3], 1.0);
        let base = world
            .get::<TesObjectRefr>(small)
            .unwrap()
            .base_form_1c
            .unwrap();
        world.get_mut::<TesSubSpace>(base).unwrap().radius_2c = 120.0;
        assert_eq!(
            subspace_in_list(&world, [1.0; 3], [first, second].into_iter()),
            Some(first)
        );
        assert_eq!(
            subspace_in_list(&world, [1.0; 3], [first, second, small].into_iter()),
            Some(small)
        );
        assert_eq!(
            subspace_in_list(&world, [500.0; 3], [first].into_iter()),
            None
        );
    }
}

/// IDA: 004C9CF0 an exterior cell's +0x50.
#[must_use]
pub fn cell_world(world: &World, cell: Option<Entity>) -> Option<Entity> {
    let data = world.get::<TesObjectCell>(cell?)?;
    if data.flags_24 & 1 != 0 {
        return None;
    }
    data.world_50
}

/// IDA: 004D3753..004D38D2, the attached arms of 004D35D0, reached with TLS +0x184 clear and neither flag 0x4000 nor 0x800 on the reference: vt+0x154 again, then 00440B70(cell, 0). An attached cell with no node: an actor outside process levels 5 and 6, or any actor while the player sleeps, takes 0043B5C0 (an actor only) and 004D80C0(0); otherwise an actor runs vt+0x1A4 unless 0045A500 holds without +0x18 bit 4, and 00440C80(cell, null) gives 00438060 its priority. A cell that isn't attached: an awake player and +0x18 bit 5 clear give an actor vt+0x150(0). Every arm ends in 004D80C0(0). Completion: 100% of the arms with no 3D; 00438060 is made when a caller lends the reference load's services (`queue`), and is a counted boundary otherwise, whose priority argument `TesGridBoundaries` keeps.
/// TODO: [ ] 004D377D an attached cell with 3D (004CEAD0).
pub(crate) fn insert_reference_attached_arms(
    world: &mut World,
    cell: Entity,
    reference: Entity,
    queue: Option<&mut crate::tes::reference_load::ReferenceLoadServices<'_, '_>>,
) -> Result<(), GameError> {
    // 004D3753..004D375D.
    let loaded = reference_ni_node(world, reference).is_some();
    if cell_attached(world, Some(cell), false) {
        if loaded {
            return Err(GameError::unclassified(
                0x004D377D,
                "004D377D: 004CEAD0 a loaded reference",
            ));
        }
        let actor = world.get::<Actor>(reference).is_some();
        let level = world
            .get::<TesObjectCell>(cell)
            .expect("a cell")
            .process_level_26;
        let load = !actor || level == 6 || level == 5;
        if (actor && player_is_sleeping(world)?) || !load {
            // 004D379D..004D37C0.
            if actor {
                actor_process_manager_refresh(world, reference)?;
            }
            return refresh_light(world, reference);
        }
        // 004D3830..004D386A an actor, unless 0045A500 holds without +0x18 bit 4: vt+0x1A4.
        if actor && (!save_load_active(world)? || save_load_flags(world)? & 0x10 != 0) {
            crate::tes::actor::actor_rebuild_high_process(world, reference)?;
        }
        // 004D386C..004D3882 00440C80(cell, null), 00438060.
        let grids = world
            .get_resource::<TesGridGlobals>()
            .and_then(|globals| globals.grids_to_load_b06a2c)
            .ok_or(GameError::unclassified(
                0x004D3875,
                "004D3875: 00440C80 before 0040EE7A publishes uGridsToLoad",
            ))?;
        let priority = cell_load_priority(world, cell, None, grids)?;
        // The priority is recorded as 004D5BD0's loop records its own; the call is made when the services are lent.
        world
            .get_resource_or_init::<TesGridBoundaries>()
            .queue_model(priority);
        if let Some(services) = queue {
            crate::tes::reference_load::model_loader_queue_reference(
                world,
                reference,
                priority as u8,
                services,
            )?;
        }
        // 004D3887.
        return refresh_light(world, reference);
    }
    // 004D3897.
    if world.get::<Actor>(reference).is_some()
        && !player_is_sleeping(world)?
        && save_load_flags(world)? & 0x20 == 0
    {
        set_3d_null(world, reference)?;
    }
    refresh_light(world, reference)
}

// IDA: 004F03A0 the world's persistent-cell insert. 004EF1F0 creates a missing cell: FormHeap 0x58, 004D3210, a host stop.
pub(crate) fn world_insert_persistent(
    world: &mut World,
    space: Entity,
    reference: Entity,
) -> Result<(), GameError> {
    if form_flags(world, reference) & 0x4000 != 0 {
        return Ok(());
    }
    let cell = world
        .get::<TesWorldSpace>(space)
        .expect("a world")
        .persistent_cell_34
        .ok_or(GameError::unclassified(
            0x004EF1F0,
            "004EF1F0: creating a world's persistent cell",
        ))?;
    insert_reference_runtime(world, cell, reference)
}

// IDA: 004F03D0 the world's persistent-cell removal.
pub(crate) fn world_remove_persistent(
    world: &mut World,
    space: Entity,
    reference: Entity,
) -> Result<(), GameError> {
    match world
        .get::<TesWorldSpace>(space)
        .expect("a world")
        .persistent_cell_34
    {
        Some(cell) => cell_remove_reference(world, cell, reference),
        None => Ok(()),
    }
}

// IDA: 004C9860 the cell's +0x24 bit 6, then vt+0x40(8).
pub(crate) fn cell_set_flag_40(
    world: &mut World,
    cell: Entity,
    value: bool,
) -> Result<(), GameError> {
    let mut data = world.get_mut::<TesObjectCell>(cell).expect("a cell");
    if value {
        data.flags_24 |= 0x40;
    } else {
        data.flags_24 &= !0x40;
    }
    set_change_flags(world, cell, 8)
}

/// IDA: 004CBA50 a cell's first reference: the +0x48 head item.
#[must_use]
pub fn cell_first_reference(world: &World, cell: Entity) -> Option<Entity> {
    world
        .get::<TesObjectCell>(cell)?
        .objects_48
        .items()
        .next()
        .filter(|&form| world.get::<TesObjectRefr>(form).is_some())
}

// IDA: 004CBB20(0x1C, 1) as 005ED965 calls it, inline in the C++: the first non-deleted reference of base type 0x1C in +0x48.
pub(crate) fn cell_find_marker(world: &World, cell: Entity) -> Option<Entity> {
    world
        .get::<TesObjectCell>(cell)?
        .objects_48
        .items()
        .find(|&form| {
            let Some(data) = world.get::<TesObjectRefr>(form) else {
                return false;
            };
            form_flags(world, form) & 0x20 == 0
                && data
                    .base_form_1c
                    .is_some_and(|base| form_type(world, base) == 0x1C)
        })
}

const TLOD: u32 = 0x444F_4C54;

// IDA: 004EFEA9..004EFEB8, 004EF1D0 and 004EFE6B, the grid key: movsx of x's low word << 16 | movzx of y's. Completion: 100%.
#[must_use]
pub const fn grid_key(x: i32, y: i32) -> u32 {
    (((x as i16 as i32) << 16) as u32) | (y as u16 as u32)
}

/// IDA: 004F06A0 TESWorldSpace_FindCellByEditorID(name): the +0x30 map in 00452600's order, bucket by bucket and each chain from its head, the first cell whose vt+0xD4 matches (`_stricmp`). A null value is skipped. Completion: 100%.
#[must_use]
pub fn world_find_cell_by_editor_id(world: &World, space: Entity, name: &[u8]) -> Option<Entity> {
    let data = world.get::<TesWorldSpace>(space).expect("a world");
    data.cells_30
        .buckets_08
        .iter()
        .flatten()
        .find_map(|&(_, cell)| {
            (oblivion_core::crt::stricmp(&editor_id_of(world, cell), name) == 0).then_some(cell)
        })
}

/// IDA: 004EFE80 TESWorldSpace_LookupCell(x, y): out of range warns (A37AD8) and answers null; otherwise the +0x30 map. Answers whether it warned, for the callers with their own seam. Completion: 100%.
#[must_use]
pub fn world_lookup_cell(world: &World, space: Entity, x: i32, y: i32) -> (Option<Entity>, bool) {
    if x > 0x7FFF || y > 0x7FFF || x < -0x8000 || y < -0x8000 {
        return (None, true);
    }
    let found = world
        .get::<TesWorldSpace>(space)
        .expect("a world")
        .cells_30
        .get(grid_key(x, y))
        .copied();
    (found, false)
}

// IDA: 004EEEF0 the OFST index over +0xAC..+0xB8, each `__ftol2_sse` truncated and >> 12; out of range -1; (max x - min x + 1) * (y - min y) - min x + x, with wrapping `imul`. The +0xB8 conversion comes after x's test. Completion: 100%.
fn ofst_index(data: &TesWorldSpace, x: i32, y: i32) -> i32 {
    let bound = |index: usize| x87::ftol(f32::from_bits(data.master_bounds_ac[index])) >> 12;
    let (min_x, min_y, max_x) = (bound(0), bound(1), bound(2));
    if x > max_x || x < min_x {
        return -1;
    }
    let max_y = bound(3);
    if y > max_y || y < min_y {
        return -1;
    }
    (max_x.wrapping_sub(min_x).wrapping_add(1))
        .wrapping_mul(y.wrapping_sub(min_y))
        .wrapping_sub(min_x)
        .wrapping_add(x)
}

/// IDA: 004EF520 TESWorldSpace_SeekExteriorCellRecord(file, x, y). A master with +0xA8: 004EEEF0's index, -1 or a zero entry false, else 00451460(entry + +0xBC), its answer dropped, true. Otherwise 00451920, the WRLD record skipped into, then the walk: a GRUP of type 1 enters; 4 by the block key, 5 by the sub-block key (the block key (x >> 5, y >> 5) and sub-block (x >> 3, y >> 3) compared in fallthrough, so a type 1 compares too and can't be refused); 6, 8, 9, 10 skip; 2, 3, 7 and past 10 end it. A CELL's grid, persistent 0x7FFFFFFF, from XCLC; a match runs 00451160. TLOD and ROAD skip; any other record ends it. Completion: 100%.
/// # Errors
/// A read failure in the host.
pub fn seek_exterior_cell_record(
    world: &World,
    space: Entity,
    file: Option<TesFileId>,
    x: i32,
    y: i32,
    files: &mut TesFileStore,
    services: &mut TesFileServices<'_>,
) -> Result<bool, GameError> {
    let data = world.get::<TesWorldSpace>(space).expect("a world");
    if let Some(id) = file
        && files.get_mut(id)?.is_master()
        && let Some(table) = &data.offsets_a8
    {
        let index = ofst_index(data, x, y);
        if index == -1 {
            return Ok(false);
        }
        let entry = *table.get(index as usize).ok_or(GameError::unclassified(
            0x004EF55E,
            "004EF55E: an OFST index past the table",
        ))?;
        if entry == 0 {
            return Ok(false);
        }
        set_tes_file_offset(
            files,
            id,
            entry.wrapping_add(data.record_offset_bc),
            services,
        )?;
        return Ok(true);
    }
    let block = grid_key(x >> 5, y >> 5);
    let sub_block = grid_key(x >> 3, y >> 3);
    let Some(id) = file else {
        return Ok(false);
    };
    if !seek_world_record(world, files, id, space, services)? {
        return Ok(false);
    }
    next_record(files, id, services)?;
    let mut found = false;
    loop {
        let header = files.get_mut(id)?.record_23c;
        if header.type_23c == GRUP_FOURCC {
            // The table 004EF730 over 004EF71C: 1 clears `skip` and compares both; 4 compares both; 5 compares the sub-block; 6, 8, 9, 10 skip. 2, 3, 7 and past 10 leave done set.
            let mut skip = true;
            match header.form_id_248 {
                1 => skip = false,
                4 => {
                    if block == header.flags_244 || sub_block == header.flags_244 {
                        skip = false;
                    }
                }
                5 => {
                    if sub_block == header.flags_244 {
                        skip = false;
                    }
                }
                6 | 8..=10 => {}
                _ => break,
            }
            if skip {
                skip_group(files, id, services)?;
            } else {
                next_record(files, id, services)?;
            }
        } else if header.type_23c == CELL {
            let mut grid = [0_i32; 2];
            if header.flags_244 & 0x400 != 0 {
                grid[0] = 0x7FFF_FFFF;
            } else {
                let file = files.get_mut(id)?;
                let mut chunk = current_chunk_type(file, services)?;
                let mut read = false;
                while chunk != 0 && !read {
                    if chunk == crate::tes::data_handler_world::XCLC {
                        let mut bytes = [0_u8; 8];
                        read_current_chunk_data(file, &mut bytes, 8, services)?;
                        grid[0] = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                        grid[1] = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                        read = true;
                    }
                    if !advance_chunk(file, services)? {
                        break;
                    }
                    chunk = current_chunk_type(file, services)?;
                }
            }
            if grid == [x, y] {
                found = true;
                seek_record_chunks(files.get_mut(id)?, services)?;
                break;
            }
            next_record(files, id, services)?;
        } else if header.type_23c == TLOD || header.type_23c == ROAD {
            next_record(files, id, services)?;
        } else {
            break;
        }
    }
    Ok(found)
}

/// IDA: 004D1340 TESObjectCELL_LoadTemporaryReferences(file). A null file answers 0, not a master 1. On the cell's CELL record: another ID answers 0; 004518B0(1); not a child record answers 1; not a type 6 GRUP answers 0; 004518B0(1); not a GRUP answers 0; a type 8 is skipped (00451580), and a following non-child answers 1. Any other record falls to the same test: a GRUP of type 9 or 10, else 0. 004518B0(1); 00B33A9C = the cell; TLS +0x184 = 1; a type 9 GRUP inside is entered; TLS +0x186 = 1; each record of form type 0x31..0x34 or 0x36 whose form isn't loaded and isn't listed (00453970) runs 0044DCF0(file, 0), whose 0 clears the answer; 004518B0(1), and a type 9 GRUP labelled with this cell is entered. TLS +0x186, +0x184 and 00B33A9C clear whatever they held. Completion: 100%.
/// # Errors
/// A stop in 0044DCF0.
pub fn load_cell_temporary_references(
    world: &mut World,
    cell: Entity,
    file: Option<TesFileId>,
    cx_files: &mut TesFileStore,
    services: &mut TesFileServices<'_>,
    host: &mut dyn crate::tes::form_arms::DataLoadHost,
) -> Result<bool, GameError> {
    let Some(id) = file else {
        return Ok(false);
    };
    if !cx_files.get_mut(id)?.is_master() {
        return Ok(true);
    }
    let own = world.get::<FormIdentity>(cell).expect("a form").form_id_0c;
    let header = cx_files.get_mut(id)?.record_23c;
    if header.type_23c == CELL {
        if header.form_id_248 != own {
            return Ok(false);
        }
        next_record(cx_files, id, services)?;
        let header = cx_files.get_mut(id)?.record_23c;
        if !cell_child_record(header, services) {
            return Ok(true);
        }
        if header.type_23c != GRUP_FOURCC || header.form_id_248 != 6 {
            return Ok(false);
        }
        next_record(cx_files, id, services)?;
        let header = cx_files.get_mut(id)?.record_23c;
        if header.type_23c != GRUP_FOURCC {
            return Ok(false);
        }
        if header.form_id_248 == 8 {
            skip_group(cx_files, id, services)?;
            let header = cx_files.get_mut(id)?.record_23c;
            if !cell_child_record(header, services) {
                return Ok(true);
            }
        }
    }
    let header = cx_files.get_mut(id)?.record_23c;
    if header.type_23c != GRUP_FOURCC || !matches!(header.form_id_248, 9 | 10) {
        return Ok(false);
    }
    let mut have = next_record(cx_files, id, services)?;
    world.resource_mut::<DataLoadGlobals>().current_cell_b33a9c = Some(cell);
    world.resource_mut::<GameThreadLocals>().byte_184 = 1;
    if have {
        let header = cx_files.get_mut(id)?.record_23c;
        if header.type_23c == GRUP_FOURCC && header.form_id_248 == 9 {
            have = next_record(cx_files, id, services)?;
        }
    }
    world.resource_mut::<GameThreadLocals>().byte_186 = 1;
    let mut result = true;
    let mut failure = None;
    loop {
        if !have {
            break;
        }
        let header = cx_files.get_mut(id)?.record_23c;
        let kind = form_type_from_record_fourcc(header.type_23c, services.form_type_cache) as u8;
        if !((0x31..=0x34).contains(&kind) || kind == 0x36) {
            break;
        }
        let key = header.form_id_248;
        let existing = world.resource::<FormsById>().lookup(key);
        let listed = match save_load_lists_reference(world, key) {
            Ok(listed) => listed,
            Err(stop) => {
                failure = Some(stop);
                break;
            }
        };
        if existing.is_none() && !listed {
            let mut cx = FormLoadCx {
                file: RecordReader {
                    files: cx_files,
                    id,
                    services,
                },
                host,
            };
            match construct_object(world, &mut cx, false) {
                Ok(0) => result = false,
                Ok(_) => {}
                Err(stop) => {
                    failure = Some(stop);
                    break;
                }
            }
        }
        if !next_record(cx_files, id, services)? {
            break;
        }
        let header = cx_files.get_mut(id)?.record_23c;
        if header.type_23c == GRUP_FOURCC
            && header.form_id_248 == 9
            && header.flags_244 == own
            && !next_record(cx_files, id, services)?
        {
            break;
        }
    }
    let mut tls = world.resource_mut::<GameThreadLocals>();
    tls.byte_186 = 0;
    tls.byte_184 = 0;
    world.resource_mut::<DataLoadGlobals>().current_cell_b33a9c = None;
    match failure {
        Some(stop) => Err(stop),
        None => Ok(result),
    }
}

// IDA: 004CC540 TESObjectCELL_ContainsPosition(pos): an interior false; each coordinate `fstp` float, `fistp` under the round-to-nearest control word, `sar 12`, against the block's grid (0 with no block). Completion: 100%.
pub(crate) fn cell_contains_position(world: &World, cell: Entity, position: [f32; 3]) -> bool {
    if cell_is_interior(world, cell) {
        return false;
    }
    x87::fistp(position[0]) >> 12 == cell_grid_x(world, cell)
        && x87::fistp(position[1]) >> 12 == cell_grid_y(world, cell)
}

/// IDA: 004F03F0 TESWorldSpace_InsertGridReferences(cell). 00401020 on the 00B36100 lock (one thread here). The +0x64 list at the cell's key goes into the cell (004D35D0); then every +0x74 reference whose parent is another cell and whose vt+0x174 position lies in the cell (004CC540). Completion: 100%.
/// # Errors
/// A stop in 004D35D0.
pub fn world_insert_grid_references(
    world: &mut World,
    space: Entity,
    cell: Entity,
) -> Result<(), GameError> {
    let key = grid_key(cell_grid_x(world, cell), cell_grid_y(world, cell));
    let data = world.get::<TesWorldSpace>(space).expect("a world");
    let keyed: Vec<Entity> = data
        .references_64
        .get(key)
        .map(|list| list.items().collect())
        .unwrap_or_default();
    for reference in keyed {
        crate::tes::ai_cell_moves::insert_reference_runtime(world, cell, reference)?;
    }
    let others: Vec<Entity> = world
        .get::<TesWorldSpace>(space)
        .expect("a world")
        .unsorted_74
        .items()
        .collect();
    for reference in others {
        if parent_cell(world, reference) == Some(cell) {
            continue;
        }
        let position = crate::tes::forms_references::reference_position(world, reference)?;
        if cell_contains_position(world, cell, position) {
            crate::tes::ai_cell_moves::insert_reference_runtime(world, cell, reference)?;
        }
    }
    Ok(())
}

/// IDA: 004F1630 TESWorldSpace_FindMasterExteriorCellAtGrid(x, y). 004EFE80; 00B33AA0 = this, whatever follows; a cell with +0x24 bit 0x10 answers at once. For each nonnull mod file (the count taken before the loop) that is a master and seeks (004EF520): with no cell yet, 004D3210, 004C9800(0), 004CA710, 004C9AC0, 004EFEF0 (its answer dropped), 00459790 with 0046C300, 00447050, whose byte is the answer so far; then 004D1340, whose false clears it. A cell: 004C9F90(1), the toggle around vt+0x6C, 004F03F0. A false answer warns A48380. Completion: 100%, with a save's hit a stop.
/// # Errors
/// A stop below.
pub fn find_master_exterior_cell(
    world: &mut World,
    space: Entity,
    x: i32,
    y: i32,
    services: &mut DataLoadServices<'_>,
) -> Result<Option<Entity>, GameError> {
    let (mut cell, out_of_range) = world_lookup_cell(world, space, x, y);
    if out_of_range {
        crate::tes::data_handler_world::warn(world, services, invalid_coordinate_warning());
    }
    world.resource_mut::<DataLoadGlobals>().current_world_b33aa0 = Some(space);
    if let Some(found) = cell
        && world.get::<TesObjectCell>(found).expect("a cell").flags_24 & 0x10 != 0
    {
        return Ok(cell);
    }
    let mut loaded = true;
    let count = world
        .get::<ModReferences>(space)
        .map_or(0, |mods| mods.0.len());
    for index in 0..count {
        let file = crate::tes::form::mod_file_at_index(world, space, index as i32);
        let Some(id) = file else {
            continue;
        };
        let step = with_files(world, services, |world, files, file_services, host| {
            if !files.get_mut(id)?.is_master()
                || !seek_exterior_cell_record(world, space, Some(id), x, y, files, file_services)?
            {
                return Ok((cell, None));
            }
            let mut answer = None;
            let current = match cell {
                Some(found) => found,
                None => {
                    let new = construct_cell(world)?;
                    {
                        let mut data = world.get_mut::<TesObjectCell>(new).expect("a cell");
                        data.flags_24 &= !1;
                        allocate_cell_data(&mut data);
                        let block = data.data_3c.as_mut().expect("just allocated");
                        block[0] = x as u32;
                        block[1] = y as u32;
                    }
                    world_add_cell(world, space, new, &mut |text| file_services.warn(text))?;
                    take_created_exterior_cell_id(world)?;
                    let mut cx = FormLoadCx {
                        file: RecordReader {
                            files: &mut *files,
                            id,
                            services: &mut *file_services,
                        },
                        host: &mut *host,
                    };
                    answer = Some(load_form_from_file(world, new, &mut cx)?);
                    new
                }
            };
            if !load_cell_temporary_references(
                world,
                current,
                Some(id),
                files,
                file_services,
                host,
            )? {
                answer = Some(false);
            }
            Ok((Some(current), answer))
        })?;
        cell = step.0;
        if let Some(answer) = step.1 {
            loaded = answer;
        }
    }
    if let Some(found) = cell {
        world
            .get_mut::<TesObjectCell>(found)
            .expect("a cell")
            .flags_24 |= 0x10;
        init_item_toggled(world, found, services)?;
        world_insert_grid_references(world, space, found)?;
    }
    if !loaded {
        let name: String = editor_id_of(world, space)
            .iter()
            .map(|&byte| char::from(byte))
            .collect();
        let id = world.get::<FormIdentity>(space).expect("a form").form_id_0c;
        crate::tes::data_handler_world::warn(
            world,
            services,
            format!(
                "Failed to load temporary data for cell ({x}, {y}) in worldspace '{name}' ({id:08X})."
            ),
        );
    }
    Ok(cell)
}

/// IDA: 004D4310 TESObjectCELL_LoadTemporaryData. +0x24 bit 0x10 answers true. For each nonnull mod file that is a master: an interior's +0x50 (004C9D10) nonzero goes to 00451460, else 00451920 with 004CC5D0; a false seek or a false 004D1340 clears the answer. +0x24 |= 0x10; the toggle around vt+0x6C. Completion: 100%.
/// # Errors
/// A stop below.
pub fn load_interior_cell_temporary_data(
    world: &mut World,
    cell: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<bool, GameError> {
    if world.get::<TesObjectCell>(cell).expect("a cell").flags_24 & 0x10 != 0 {
        return Ok(true);
    }
    let mut loaded = true;
    let count = world
        .get::<ModReferences>(cell)
        .map_or(0, |mods| mods.0.len());
    for index in 0..count {
        let Some(id) = crate::tes::form::mod_file_at_index(world, cell, index as i32) else {
            continue;
        };
        let good = with_files(world, services, |world, files, file_services, host| {
            if !files.get_mut(id)?.is_master() {
                return Ok(true);
            }
            let data = world.get::<TesObjectCell>(cell).expect("a cell");
            let offset = if data.flags_24 & 1 != 0 {
                data.toft_50
            } else {
                0
            };
            let found = if offset != 0 {
                set_tes_file_offset(files, id, offset, file_services)?
            } else {
                seek_cell_record(world, files, id, cell, file_services)?
            };
            Ok(found
                && load_cell_temporary_references(
                    world,
                    cell,
                    Some(id),
                    files,
                    file_services,
                    host,
                )?)
        })?;
        if !good {
            loaded = false;
        }
    }
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .flags_24 |= 0x10;
    init_item_toggled(world, cell, services)?;
    Ok(loaded)
}

/// IDA: 004CB590 TESObjectCELL_UpdateReferenceSounds(cell, start) under the region lock: with OSSoundGlobals +0x24, each reference whose base isn't flagged 0x800 and is a sound (0x0A), a light (0x1A), an activator (0x12), or a door (0x18) in a cell without form flag 0x2000, gets 004D9310(start). Completion: 100% with `start` 1.
/// # Errors
/// A stop in 004D9310.
pub fn cell_update_reference_sounds(
    world: &mut World,
    cell: Entity,
    services: &mut GameSoundServices<'_>,
) -> Result<(), GameError> {
    if world
        .get_resource::<OsGlobalsData>()
        .is_none_or(|os| os.sound_24.is_none())
    {
        return Ok(());
    }
    let cell_flags = world.get::<FormIdentity>(cell).expect("a form").flags_08;
    let references: Vec<Entity> = world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .items()
        .collect();
    for reference in references {
        let Some(base) = world
            .get::<TesObjectRefr>(reference)
            .and_then(|r| r.base_form_1c)
        else {
            continue;
        };
        if world.get::<FormIdentity>(base).expect("a form").flags_08 & 0x800 != 0 {
            continue;
        }
        let kind = form_type(world, base);
        if matches!(kind, 0x0A | 0x1A | 0x12) || (kind == 0x18 && cell_flags & 0x2000 == 0) {
            reference_start_sound(world, reference, base, services)?;
        }
    }
    Ok(())
}
