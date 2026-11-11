//! Phase 15 (15c, 15d): the startup tail's exterior grid load and the cell transitions, in original order: 00444EC0 -> 00444C70 (the clears, 0043FFF0, 004430F0, 00444340 the fill through 004441A0, 00440590, 004402F0), 00482310, then 004455E0, 00445A10, 00444FB0 and 00441D50.
//!
//! The Bethesda state is ported: the grid arrays, the TES coordinates and +0x30, +0x78, the cell process levels and water bit, the buffers, the water records. A 3D, LOD, renderer, Havok, sound, script or actor-process call is a counted boundary in `TesGridBoundaries`, at its address (D3). Some of them own end state: the actor-process boundaries write TES +0x30 through more 00440590 calls (P15-RUN-009), and 004D5BD0's reference loop adds actors, extras and form flags (the `end_slot` rows).
//!
//! x87: the main thread is at 24 bits, so `f32` per step in the exe's grouping; `fld`/`fstp float`/`fistp` is `x87::fistp` on the float.
//! Evidence: objdump of each address named here; P15-RUN-003, 006, 007, 008, 009.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::sky::SkyGlobals;
use super::tes::{GameThreadLocals, Tes, TesGlobals, set_current_worldspace};
use super::water::{TesWaterNodeRecord, hide_water_node_record};
use crate::bs::havok::HavokGlobals;
use crate::bs::task_manager::TaskGlobals;
use crate::bs::tree_manager::TreeManagerGlobals;
use crate::gamebryo::av_object::NiAvParent;
use crate::gamebryo::scene_node::SceneNodePayload;
use crate::tes::actor::{NI_POINT3_ZERO, TesObjectRefr};
use crate::tes::base_extra_list::{EXTRA_TELEPORT, ExtraData, teleport_door};
use crate::tes::data_handler::{DataHandler, DataHandlerLists, close_all_files};
use crate::tes::data_handler_world::lookup_or_create_exterior_cell;
use crate::tes::data_load::DataLoadServices;
use crate::tes::default_forms::DefaultFormStatics;
use crate::tes::form::FormIdentity;
use crate::tes::form_components::{FormRef, TesScriptableForm};
use crate::tes::forms_dialogue::TesQuest;
use crate::tes::forms_init_world::reference_run_scripts;
use crate::tes::forms_references::{reference_position, teleport_block};
use crate::tes::forms_world::{
    CellRegionLocks, TerrainLodServices, TesObjectCell, TesObjectLand, WorldFormGlobals,
    allocate_land_data, cell_contains_position, cell_get_or_create_land, cell_grid_x, cell_grid_y,
    cell_is_interior, find_master_exterior_cell, land_height_at_position, land_height_range,
    load_interior_cell_temporary_data,
};
use crate::tes::script_vm::{ScriptVmGlobals, script_run, scripts_enabled};
use crate::tes::settings::SettingCollection;
use crate::tes::water::{set_water_node_cell, water_surface_off, water_surface_on};

// IDA: the INI statics these read at each use.
const GRIDS_TO_LOAD: u32 = 0x00B0_6A2C;
const EXTERIOR_BUFFER: u32 = 0x00B0_51DC;
const INTERIOR_BUFFER: u32 = 0x00B0_51D4;
const PREEMPTIVELY_UNLOAD_CELLS: u32 = 0x00B0_51CC;
const BACKGROUND_CELL_LOADS: u32 = 0x00B0_8960;
const DEFAULT_COC_PLACEMENT: u32 = 0x00B0_97E0;

/// IDA: N/A the counted boundaries of the grid code (D3): each original address not ported, and how many times the original order reached it. `percents` keeps the argument of each 0057B950, which no row sees.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct TesGridBoundaries {
    pub calls: BTreeMap<u32, u32>,
    pub percents: Vec<f32>,
    /// IDA: N/A 004430F0's answers, which its caller drops.
    pub reference_counts: Vec<u32>,
    /// IDA: N/A the priority argument of each 00438060, 004D3882's 00440C80 answer.
    pub model_queue_priorities: Vec<i32>,
}

impl TesGridBoundaries {
    /// IDA: N/A a helper, with no address of its own.
    pub fn count(&mut self, address: u32) {
        *self.calls.entry(address).or_default() += 1;
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn at(&self, address: u32) -> u32 {
        self.calls.get(&address).copied().unwrap_or(0)
    }

    /// IDA: N/A a 00438060, with its priority.
    pub fn queue_model(&mut self, priority: i32) {
        self.count(0x0043_8060);
        self.model_queue_priorities.push(priority);
    }
}

/// IDA: the globals the grid code owns. Defaults are the image and P15-RUN-007 at 0040EE59.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct TesGridGlobals {
    /// 00B33A69, 00444C70's guard.
    pub loading_b33a69: u8,
    /// 00B33A58, 004402F0's guard.
    pub scripts_b33a58: u8,
    /// 00B2CBC0, 1 in the image, cleared around 004EBDA0.
    pub lod_update_b2cbc0: u8,
    /// 00B3521D, 1 in P15-RUN-007.
    pub water_player_b3521d: u8,
    /// 00B0703C, 1 in the image: 00498F30 sets it and 00B3521D; 0049A8B0 clears it.
    pub water_surface_b0703c: u8,
    /// 00B43384, set for the length of a cell change.
    pub transition_b43384: u8,
    /// 00B43074, 004455E0's 1 and 00445A10's 0.
    pub interior_b43074: u8,
    /// 00B3522C, 00445A10's 1.
    pub exterior_done_b3522c: u8,
    /// 00B4312C, 00B43130: (c + 0.5) * 4096.
    pub centre_b4312c: [f32; 2],
    /// 00B43134, 00B43138: copies.
    pub centre_copy_b43134: [f32; 2],
    /// 00B43078.
    pub game_time_b43078: f32,
    /// 00B33A64, 00440C80's cached cell. Only 00440C80 writes it, so it's never cleared.
    pub load_priority_cell_b33a64: Option<Entity>,
    /// 00B33A60, 00440C80's cached answer.
    pub load_priority_b33a60: i32,
    /// IDA: N/A 00B06A2C as 00440C80 reads it, published from the INI collection by the block at 0040EE7A: 004D35D0's callers hold no collection.
    pub grids_to_load_b06a2c: Option<u32>,
    /// IDA: N/A 00B14F58 bBorderRegionsEnabled as 00667600 reads it, published the same way.
    pub border_regions_b14f58: Option<u8>,
}

impl Default for TesGridGlobals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            loading_b33a69: 0,
            scripts_b33a58: 0,
            lod_update_b2cbc0: 1,
            water_player_b3521d: 1,
            water_surface_b0703c: 1,
            transition_b43384: 0,
            interior_b43074: 0,
            exterior_done_b3522c: 0,
            centre_b4312c: [0.0; 2],
            centre_copy_b43134: [0.0; 2],
            game_time_b43078: 0.0,
            load_priority_cell_b33a64: None,
            load_priority_b33a60: 0,
            grids_to_load_b06a2c: None,
            border_regions_b14f58: None,
        }
    }
}

/// IDA: N/A what the grid code asks outside the world. Defaults are P15-RUN-007's and P15-RUN-008's answers at the startup tail. Each is a seam until its owner is ported.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TesGridAnswers {
    /// 0057BAC0 InterfaceManager_IsLoadingMenuHidden: true in P15-RUN-007.
    pub loading_menu_hidden: bool,
    /// 004F9FA0, the byte 00B09E25: true in P15-RUN-007.
    pub scripts_ready: bool,
    /// 0043E000 on the cell, whose walk of the queue the host's counted 00438060 never fills: false. No run says.
    pub cell_tasks_pending: bool,
    /// 00B42D78, set to 00405290 at 0040E8E0, called as (0, 0). No run records its answer.
    pub game_time: f32,
}

impl Default for TesGridAnswers {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            loading_menu_hidden: true,
            scripts_ready: true,
            cell_tasks_pending: false,
            game_time: 0.0,
        }
    }
}

/// IDA: N/A what the grid code reads outside the world.
pub struct TesGridServices<'a, 'b> {
    pub load: &'a mut DataLoadServices<'b>,
    pub ini: &'a SettingCollection,
    pub answers: TesGridAnswers,
    /// 00B43077, for 004431F0's 004EBC00.
    pub display_lod_land_b43077: u8,
    /// 0040FEC0, 004431F0's level 2 log.
    pub level2_log: &'a mut dyn FnMut(String),
    /// 0057A0D0.
    pub refresh_map_menu: &'a mut dyn FnMut(&mut World) -> Result<bool, GameError>,
    /// IDA: N/A the interface, for the cell entry's 00440AF0 when the caller lends it; without it (the startup tail) that call is counted.
    pub interface: Option<&'a mut GridInterfaceCall<'a>>,
    /// IDA: N/A 00434020 IOManager_DrainTasksThroughStage(stage) with the host's timed pass, when the caller lends it; without it (the startup tail) the cell entry's drain is counted.
    pub drain: Option<&'a mut GridDrainCall<'a>>,
    /// IDA: N/A whether 004D5BD0's reference loop runs (`cell_reference_loop`): the cell entry of a game that runs. Startup's cells leave it counted, as the `end_slot` rows hold its effects there (a host substitution).
    pub reference_loop: bool,
    /// IDA: N/A the settings 004CB590's sounds read, lent by a game in play; without them (startup's cells) 004CB590 is counted.
    pub game_sounds: Option<crate::tes::game_sounds::GameSoundSettings>,
    /// IDA: N/A the archives a reference's model child reads (00438060's queue), lent by a game in play.
    pub archives: Option<&'a mut dyn crate::bs::archive::ArchiveAccess>,
    /// IDA: N/A the model loader's services for a main-thread ModelLoader_LoadModelNow (0066F358's 00444A10), lent by a game
    /// in play.
    pub models: Option<&'a mut GridModelCall<'a>>,
}

/// IDA: N/A `TesGridServices::models`: runs the body with the model loader's services.
pub type GridModelCall<'f> = dyn FnMut(
        &mut World,
        &mut dyn FnMut(
            &mut World,
            &mut crate::bs::model_loader::ModelBuildServices<'_>,
        ) -> Result<(), GameError>,
    ) -> Result<(), GameError>
    + 'f;

/// IDA: N/A `TesGridServices::drain`.
pub type GridDrainCall<'f> =
    dyn FnMut(&mut World, i32) -> Result<crate::bs::model_loader::IoDrainResult, GameError> + 'f;

/// IDA: N/A `TesGridServices::interface`: runs the body with an interface context.
pub type GridInterfaceCall<'f> = dyn FnMut(
        &mut World,
        &mut dyn FnMut(&mut crate::tes::tile::InterfaceCx) -> Result<(), GameError>,
    ) -> Result<(), GameError>
    + 'f;

impl TesGridServices<'_, '_> {
    // IDA: N/A a helper, with no address of its own.
    fn ini_raw(&self, address: u32) -> Result<u32, GameError> {
        self.ini
            .find_address(address)
            .map(|setting| setting.raw)
            .ok_or(GameError::unclassified(
                0,
                "the grid code: an unregistered INI static",
            ))
    }

    // IDA: N/A an accessor.
    fn grids(&self) -> Result<u32, GameError> {
        self.ini_raw(GRIDS_TO_LOAD)
    }

    /// IDA: 004431F0 on g_tes, with `run_world_setup_worldspace`'s services.
    /// # Errors
    /// A stop in 004431F0.
    pub fn set_world(
        &mut self,
        world: &mut World,
        target: Option<Entity>,
    ) -> Result<(), GameError> {
        let disable_warning = world
            .get_resource::<crate::tes::data_load::DataLoadGlobals>()
            .is_some_and(|load| load.disable_warning_messages_b06b18 != 0);
        let host = &mut *self.load.host;
        let warning = &mut *self.load.warning;
        let mut find_file = |path: &str| host.find_file(path, 0, -1);
        let mut warn = |text: String| warning(text, disable_warning);
        let mut lod = TerrainLodServices {
            display_lod_land_b43077: self.display_lod_land_b43077,
            find_file: &mut find_file,
            warning: &mut warn,
            debug_log: self.level2_log,
            find_file_calls: 0,
        };
        set_current_worldspace(world, target, self.ini, &mut lod, self.refresh_map_menu)
    }
}

// IDA: N/A a helper, with no address of its own.
pub(in crate::tes) fn tes(world: &World) -> Result<&Tes, GameError> {
    world
        .resource::<TesGlobals>()
        .tes
        .as_deref()
        .ok_or(GameError::unclassified(0, "the grid code on a null g_tes"))
}

// IDA: N/A a helper, with no address of its own.
fn tes_mut(world: &mut World) -> &mut Tes {
    world
        .resource_mut::<TesGlobals>()
        .into_inner()
        .tes
        .as_deref_mut()
        .expect("tested by `tes`")
}

// IDA: N/A a helper, with no address of its own.
pub(crate) fn count(world: &mut World, address: u32) {
    world
        .get_resource_or_init::<TesGridBoundaries>()
        .count(address);
}

// IDA: N/A a helper, with no address of its own.
pub(crate) fn grid_globals(world: &mut World) -> Mut<'_, TesGridGlobals> {
    world.get_resource_or_init::<TesGridGlobals>()
}

// IDA: 0045A500 on g_saveLoadGame, main thread.
fn save_load_active(world: &World) -> Result<bool, GameError> {
    Ok(save_load_flags(world)? & 1 != 0)
}

// IDA: g_saveLoadGame +0x18.
fn save_load_flags(world: &World) -> Result<u32, GameError> {
    world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .map(|game| game.fields_14[1])
        .ok_or(GameError::unclassified(
            0x00444EC1,
            "00444EC1: a null g_saveLoadGame",
        ))
}

// IDA: N/A a helper, with no address of its own.
fn havok_world(world: &World) -> bool {
    world
        .get_resource::<HavokGlobals>()
        .is_some_and(|globals| globals.world_b35c24.is_some())
}

// IDA: the cell of slot (i, j): 00482150's `[eax]`.
fn slot_cell(world: &World, i: u32, j: u32) -> Result<Option<Entity>, GameError> {
    let grid = tes(world)?
        .grid_cells_08
        .as_ref()
        .ok_or(GameError::unclassified(
            0x00482150,
            "00482150: TES +0x08 is null",
        ))?;
    let index = j as usize + i as usize * grid.size_0c as usize;
    grid.grid_10
        .get(index)
        .map(|slot| slot.cell_00)
        .ok_or(GameError::unclassified(
            0x00482150,
            "00482150: grid slot out of range",
        ))
}

// IDA: N/A no address of its own; its comments cite 00482150.
fn set_slot_cell(world: &mut World, i: u32, j: u32, cell: Option<Entity>) -> Result<(), GameError> {
    tes(world)?;
    let grid = tes_mut(world)
        .grid_cells_08
        .as_mut()
        .ok_or(GameError::unclassified(
            0x00482150,
            "00482150: TES +0x08 is null",
        ))?;
    grid.slot_at(i, j)?.cell_00 = cell;
    Ok(())
}

// IDA: N/A a helper, with no address of its own.
fn process_level(world: &World, cell: Entity) -> i8 {
    world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .process_level_26 as i8
}

// IDA: N/A a helper, with no address of its own.
fn set_process_level(world: &mut World, cell: Entity, level: u8) {
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .process_level_26 = level;
}

// IDA: 004821B0 GridCellArray_IsSlotAttached(i, j): the slot cell's level is 6. Completion: 100%.
fn slot_attached(world: &World, i: u32, j: u32) -> Result<bool, GameError> {
    Ok(slot_cell(world, i, j)?.is_some_and(|cell| process_level(world, cell) == 6))
}

// IDA: 00482240 vt+0x1C: the slot's cell null, +0x20 = 0. Completion: 100%.
fn clear_slot(world: &mut World, i: u32, j: u32) -> Result<(), GameError> {
    set_slot_cell(world, i, j, None)?;
    tes_mut(world)
        .grid_cells_08
        .as_mut()
        .expect("tested")
        .byte_20 = 0;
    Ok(())
}

// IDA: 00482200 vt+0x18: a slot with a cell runs 004D6450, then vt+0x1C. 004D6450 unloads the cell's 3D, a host stop: nothing fills a slot before these clears in Phase 15. Completion: 100% of an empty slot.
// TODO: [ ] 00482221 004D6450.
fn detach_slot(world: &mut World, i: u32, j: u32) -> Result<(), GameError> {
    if slot_cell(world, i, j)?.is_some() {
        return Err(GameError::unclassified(
            0x00482221,
            "00482221: 004D6450 detaches a slot's cell",
        ));
    }
    Ok(())
}

// IDA: 00482260 vt+0x20(from i, from j, to i, to j): copies the cell, not the water record. Completion: 100%.
fn move_slot(world: &mut World, from: (u32, u32), to: (u32, u32)) -> Result<(), GameError> {
    let cell = slot_cell(world, from.0, from.1)?;
    set_slot_cell(world, to.0, to.1, cell)
}

// IDA: N/A no address of its own; its comments cite 00444CB0.
fn cell_array_size(world: &World) -> Result<u32, GameError> {
    Ok(tes(world)?
        .grid_cells_08
        .as_ref()
        .ok_or(GameError::unclassified(
            0x00444CB0,
            "00444CB0: TES +0x08 is null",
        ))?
        .size_0c)
}

// IDA: 00481E20 vt+8 on the cell array: vt+0x18 by (i, j), i outer. Completion: 100%.
fn clear_all_cells(world: &mut World) -> Result<(), GameError> {
    let size = cell_array_size(world)?;
    for i in 0..size {
        for j in 0..size {
            detach_slot(world, i, j)?;
        }
    }
    Ok(())
}

// IDA: 00481E60 vt+0x14(dx, dy) on the cell array: +0x04, +0x08 add; `cdq` absolute values at or past the size run vt+8. Otherwise dx then dy, one step at a time: the leaving edge's vt+0x18, the moves, the new edge's vt+0x1C. Completion: 100%.
fn shift_cells(world: &mut World, mut dx: i32, mut dy: i32) -> Result<(), GameError> {
    let n = cell_array_size(world)?;
    {
        let grid = tes_mut(world).grid_cells_08.as_mut().expect("tested");
        grid.world_x_04 = grid.world_x_04.wrapping_add(dx);
        grid.world_y_08 = grid.world_y_08.wrapping_add(dy);
    }
    if dx.unsigned_abs() >= n || dy.unsigned_abs() >= n {
        return clear_all_cells(world);
    }
    while dx != 0 {
        for j in 0..n {
            if dx < 0 {
                detach_slot(world, n - 1, j)?;
                for i in (1..n).rev() {
                    move_slot(world, (i - 1, j), (i, j))?;
                }
                clear_slot(world, 0, j)?;
            } else {
                detach_slot(world, 0, j)?;
                for i in 0..n - 1 {
                    move_slot(world, (i + 1, j), (i, j))?;
                }
                clear_slot(world, n - 1, j)?;
            }
        }
        dx += if dx < 0 { 1 } else { -1 };
    }
    while dy != 0 {
        for i in 0..n {
            if dy < 0 {
                detach_slot(world, i, n - 1)?;
                for j in (1..n).rev() {
                    move_slot(world, (i, j - 1), (i, j))?;
                }
                clear_slot(world, i, 0)?;
            } else {
                detach_slot(world, i, 0)?;
                for j in 0..n - 1 {
                    move_slot(world, (i, j + 1), (i, j))?;
                }
                clear_slot(world, i, n - 1)?;
            }
        }
        dy += if dy < 0 { 1 } else { -1 };
    }
    Ok(())
}

// IDA: 00482090 vt+0x10(x, y) on the cell array: 00482050 (dx = x - +0x04, dy = y - +0x08, vt+0x14, answers either nonzero); changed: +0x14, +0x18 `fild` of x << 12, y << 12, +0x1C 0.0, 004CCCC0(&+0x14) the Havok centre, +0x20 = 0. Completion: 100%, with 004CCCC0 counted.
fn recenter_cells(world: &mut World, x: i32, y: i32) -> Result<bool, GameError> {
    let (dx, dy) = {
        let grid = tes(world)?
            .grid_cells_08
            .as_ref()
            .ok_or(GameError::unclassified(
                0x00482050,
                "00482050: TES +0x08 is null",
            ))?;
        (
            x.wrapping_sub(grid.world_x_04),
            y.wrapping_sub(grid.world_y_08),
        )
    };
    shift_cells(world, dx, dy)?;
    if dx == 0 && dy == 0 {
        return Ok(false);
    }
    let grid = tes_mut(world).grid_cells_08.as_mut().expect("tested");
    grid.float_14 = x.wrapping_shl(12) as f32;
    grid.float_18 = y.wrapping_shl(12) as f32;
    grid.float_1c = 0.0;
    count(world, 0x004C_CCC0);
    tes_mut(world)
        .grid_cells_08
        .as_mut()
        .expect("tested")
        .byte_20 = 0;
    Ok(true)
}

// IDA: 00483630 vt+0x18 on the distant array: 007B3A40 on the entry's key (a counted boundary over a map empty at startup), then vt+0x1C 00483680: an object's release is a host stop; 004BD380 on a nonnull 00B35B8C counted; +0x00, +0x08, +0x0C, +0x01 = 0. Completion: 100% with no object.
// TODO: [ ] 0048369B an entry's object, when 00483D60 is ported.
fn detach_distant(world: &mut World, index: usize) -> Result<(), GameError> {
    count(world, 0x007B_3A40);
    let has_tasks = world
        .get_resource::<TaskGlobals>()
        .is_some_and(|tasks| tasks.distant_lod_tasks_b35b8c.is_some());
    let distant = tes_mut(world).grid_distant_04.as_mut().expect("tested");
    let entry = &mut distant.entries_10[index];
    if entry.object_04.is_some() {
        return Err(GameError::unclassified(
            0x0048369B,
            "0048369B: a loaded GridDistantArray entry",
        ));
    }
    entry.byte_00 = 0;
    entry.x_08 = 0;
    entry.y_0c = 0;
    entry.byte_01 = 0;
    if has_tasks {
        count(world, 0x004B_D380);
    }
    Ok(())
}

// IDA: 00481E20 vt+8 on the distant array.
fn clear_all_distant(world: &mut World) -> Result<(), GameError> {
    let size = tes(world)?
        .grid_distant_04
        .as_ref()
        .ok_or(GameError::unclassified(
            0x00444CB7,
            "00444CB7: TES +0x04 is null",
        ))?
        .size_0c as usize;
    for index in 0..size * size {
        detach_distant(world, index)?;
    }
    Ok(())
}

// IDA: 00483B60 vt+0x10(x, y) on the distant array: 00482050, whose vt+0x14 is 00481E60 with the distant slots: a shift at or past the size clears; a smaller one moves entries (00483B90), a host stop. Changed: 00483750, which the C++ only counted: each entry at {0, 0} takes (+0x04 + i - half, +0x08 + j - half). Completion: 100% of the arms from the -10000 origin.
// TODO: [ ] 00481E60's partial shift with 00483B90 and 00483870, which a moved grid (00445200) reaches.
fn recenter_distant(world: &mut World, x: i32, y: i32) -> Result<bool, GameError> {
    let (dx, dy, n) = {
        let grid = tes(world)?
            .grid_distant_04
            .as_ref()
            .ok_or(GameError::unclassified(
                0x00482050,
                "00482050: TES +0x04 is null",
            ))?;
        (
            x.wrapping_sub(grid.world_x_04),
            y.wrapping_sub(grid.world_y_08),
            grid.size_0c,
        )
    };
    {
        let grid = tes_mut(world).grid_distant_04.as_mut().expect("tested");
        grid.world_x_04 = grid.world_x_04.wrapping_add(dx);
        grid.world_y_08 = grid.world_y_08.wrapping_add(dy);
    }
    if dx.unsigned_abs() >= n || dy.unsigned_abs() >= n {
        clear_all_distant(world)?;
    } else if dx != 0 || dy != 0 {
        return Err(GameError::unclassified(
            0x00481E60,
            "00481E60: a partial GridDistantArray shift",
        ));
    }
    if dx == 0 && dy == 0 {
        return Ok(false);
    }
    let grid = tes_mut(world).grid_distant_04.as_mut().expect("tested");
    let half = (grid.size_0c >> 1) as i32;
    for i in 0..grid.size_0c {
        for j in 0..grid.size_0c {
            let (base_x, base_y) = (grid.world_x_04, grid.world_y_08);
            let entry = &mut grid.entries_10[(i * n + j) as usize];
            if entry.x_08 == 0 && entry.y_0c == 0 {
                entry.x_08 = base_x.wrapping_add(i as i32 - half);
                entry.y_0c = base_y.wrapping_sub(half).wrapping_add(j as i32);
            }
        }
    }
    Ok(true)
}

// IDA: 0043FEA0, 0043FD40 whether the buffer holds the cell, over the INI's length.
fn in_exterior_buffer(
    world: &World,
    cell: Entity,
    services: &TesGridServices<'_, '_>,
) -> Result<bool, GameError> {
    let length = services.ini_raw(EXTERIOR_BUFFER)? as usize;
    Ok(tes(world)?
        .exterior_cell_buffer_3c
        .iter()
        .take(length)
        .any(|entry| *entry == Some(cell)))
}

// IDA: N/A a helper, with no address of its own.
fn in_interior_buffer(
    world: &World,
    cell: Entity,
    services: &TesGridServices<'_, '_>,
) -> Result<bool, GameError> {
    let length = services.ini_raw(INTERIOR_BUFFER)? as usize;
    Ok(tes(world)?
        .interior_cell_buffer_38
        .iter()
        .take(length)
        .any(|entry| *entry == Some(cell)))
}

// IDA: N/A a helper, with no address of its own.
fn in_buffer(
    world: &World,
    cell: Entity,
    services: &TesGridServices<'_, '_>,
) -> Result<bool, GameError> {
    if cell_is_interior(world, cell) {
        in_interior_buffer(world, cell, services)
    } else {
        in_exterior_buffer(world, cell, services)
    }
}

// IDA: 004D58B0 TESObjectCELL_GetOrCreateNode: a set +0x54 answers. Otherwise +0x26 = 2; a NiNode root (translate zero, rotate
// identity) takes, by AddObject(child, 0): two NiNodes, culled (+0x18 bit 0) by 00B35C00 bits 0 and 1, then one 004CA7A0 per
// quadrant, one for an interior and four for an exterior: a NiNode with four NiNode children culled by bits 2, 3, 4 and 5, the
// second one's +0x18 |= 0x40. 00707370 Update(0, 0); +0x54; 004D4A20 (`cell_havok::create_interior_havok_world`: an
// interior's Havok world through the physics seam and the ExtraHavok's CellMopp); +0x26 = 3. 00B35C00 is BSS that only 00442A30's console toggles and
// bhkWorld_CreateDefault (004D5624, zero) write, so it is 0 here. Completion: 100%.
pub(crate) fn cell_node(world: &mut World, cell: Entity) -> Result<Entity, GameError> {
    if world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .scene_node_present_54
        != 0
    {
        return world
            .get::<crate::tes::forms_world::CellSceneNode>(cell)
            .map(|node| node.0)
            .ok_or(GameError::unclassified(
                0x004D58B0,
                "004D58B0: a +0x54 the host didn't build",
            ));
    }
    set_process_level(world, cell, 2);
    let cull_bits = 0_u32;
    let add = |world: &mut World, parent: Entity, cull: bool| -> Result<Entity, GameError> {
        let child = crate::gamebryo::scene_node::spawn_cold_node(world);
        {
            let mut payload = world.get_mut::<SceneNodePayload>(child).expect("spawned");
            if cull {
                payload.effective_av_flags |= 1;
            } else {
                payload.effective_av_flags &= !1;
            }
        }
        if crate::bs::scene_graphs::attach_scene_child(world, parent, Some(child), false)
            != crate::bs::scene_graphs::AttachChildStatus::Attached
        {
            return Err(GameError::unclassified(
                0x004D59A0,
                "004D59A0: AddObject on the cell's node failed",
            ));
        }
        Ok(child)
    };
    let root = crate::gamebryo::scene_node::spawn_cold_node(world);
    add(world, root, cull_bits & 1 != 0)?;
    add(world, root, cull_bits & 2 != 0)?;
    let quadrants = if cell_is_interior(world, cell) { 1 } else { 4 };
    for _ in 0..quadrants {
        // 004CA7A0.
        let quad = crate::gamebryo::scene_node::spawn_cold_node(world);
        add(world, quad, cull_bits & 4 != 0)?;
        let second = add(world, quad, cull_bits & 8 != 0)?;
        world
            .get_mut::<SceneNodePayload>(second)
            .expect("spawned")
            .effective_av_flags |= 0x40;
        add(world, quad, cull_bits & 0x10 != 0)?;
        add(world, quad, cull_bits & 0x20 != 0)?;
        if crate::bs::scene_graphs::attach_scene_child(world, root, Some(quad), false)
            != crate::bs::scene_graphs::AttachChildStatus::Attached
        {
            return Err(GameError::unclassified(
                0x004D5A0A,
                "004D5A0A: AddObject on the cell's node failed",
            ));
        }
    }
    let result = crate::gamebryo::scene_update::update_scene_object(
        world,
        root,
        0.0,
        false,
        &mut crate::gamebryo::scene_update::NoSceneControllers,
    )?;
    if result.status != crate::gamebryo::scene_update::SceneUpdateStatus::Updated {
        return Err(GameError::unclassified(
            0x004D5A18,
            "004D5A18: the cell node's update stopped",
        ));
    }
    world
        .entity_mut(cell)
        .insert(crate::tes::forms_world::CellSceneNode(root));
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .scene_node_present_54 = 1;
    // 004D5A65 004D4A20: an interior's Havok world through the seam, and the cell's CellMopp.
    super::cell_havok::create_interior_havok_world(world, cell)?;
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .process_level_26 = 3;
    Ok(root)
}

// IDA: 004D63C5..004D63D9, 004457B5..004457D0 the cell's Havok world: an interior's BaseExtraList_GetHavokWorld (the type 2 extra's +0x0C, which 004D4A20 fills; the extra alone, with only a CellMopp, answers null), an exterior's 00B35C24. Completion: 100%.
pub(in crate::tes) fn cell_has_havok_world(world: &World, cell: Entity) -> bool {
    let data = world.get::<TesObjectCell>(cell).expect("a cell");
    if data.flags_24 & 1 != 0 {
        return super::cell_havok::cell_bhk_world(world, cell).is_some();
    }
    havok_world(world)
}

// IDA: 004D63A0 TESObjectCELL_AttachToScene(root): 004D58B0; +0x26 = 5; the root's vt+0x84 AddObject; with a Havok world, an exterior's 004D4D00, world +0x19 = (00B33A34 == 0), 0088B680(00B097B8); 004E4980 with +0x44 runs 004E5550; 004CB670(1), 004CB590(1), 00707610, 00707370. Completion: 100% of the cell's stores, with the calls counted.
fn attach_cell(
    world: &mut World,
    cell: Entity,
    services: &mut TesGridServices<'_, '_>,
) -> Result<(), GameError> {
    cell_node(world, cell)?;
    set_process_level(world, cell, 5);
    count(world, 0x004D_63A0);
    if cell_has_havok_world(world, cell) {
        if !cell_is_interior(world, cell) {
            count(world, 0x004D_4D00);
        }
        count(world, 0x0088_B680);
        // 004D63EA world +0x19 = (00B33A34 == 0), 1; 004D63F7 0088B680(bHavokDebug 0): +0x14 released (none) and cleared.
        if let Some(bhk) = super::cell_havok::cell_bhk_world(world, cell) {
            let mut record = world
                .get_mut::<crate::bs::havok_world::BhkWorld>(bhk)
                .expect("a bhkWorld");
            record.byte_19 = 1;
            record.visual_debugger_14 = false;
        }
    }
    count(world, 0x004C_B670);
    match services.game_sounds {
        Some(settings) => {
            let random = world
                .get_non_send::<crate::tes::form_arms::ScriptRandom>()
                .ok_or(GameError::unclassified(
                    0x006A9D86,
                    "006A9D86: Random_Next with no host CRT",
                ))?
                .0
                .clone();
            let mut draw = |world: &mut World| random(world);
            crate::tes::forms_world::cell_update_reference_sounds(
                world,
                cell,
                &mut crate::tes::game_sounds::GameSoundServices {
                    host: &mut *services.load.host,
                    sound_enabled_b16178: settings.sound_enabled_b16178,
                    invalidate_older_files: settings.invalidate_older_files,
                    draw: &mut draw,
                },
            )?;
        }
        None => count(world, 0x004C_B590),
    }
    count(world, 0x0070_7610);
    count(world, 0x0070_7370);
    Ok(())
}

// IDA: 004D5BD0 TESObjectCELL_FinishAttach(run_havok): 004D58B0; 004CB8C0(0, 0), 004CFAF0; a buffered cell (43FD40, 43FEA0): 00707370 and 004CB790; otherwise the reference loop under the region lock (4F9EC0, 4FBF90, the actor arm, 00438060, 004CEAD0). `run_havok` runs 004D4D00. +0x26 = 6. The loop is a counted boundary per cell, and it owns the leveled actors, extras and form flags the `end_slot` rows hold past the host's. Completion: 100% of the cell's stores.
// TODO: [ ] 004D5C2E the reference loop, with 3D and scripts.
fn finish_attach(
    world: &mut World,
    cell: Entity,
    run_havok: bool,
    services: &mut TesGridServices<'_, '_>,
) -> Result<(), GameError> {
    cell_node(world, cell)?;
    let buffered = in_buffer(world, cell, services)?;
    if services.reference_loop && !buffered {
        // 004D5BE2 004CB8C0(0, 0) and 004D5BE9 004CFAF0 are still counted.
        count(world, 0x004C_B8C0);
        count(world, 0x004C_FAF0);
        // 004D5C43 00440C80(cell, null).
        let grids = services.grids()?;
        let priority = cell_load_priority(world, cell, None, grids)?;
        let mut failure = None;
        let mut queue = |world: &mut World, reference: Entity, priority: i32| {
            world
                .get_resource_or_init::<TesGridBoundaries>()
                .queue_model(priority);
            if failure.is_some() {
                return;
            }
            let mut load = crate::tes::reference_load::ReferenceLoadServices {
                archives: services.archives.as_deref_mut(),
            };
            if let Err(stop) = crate::tes::reference_load::model_loader_queue_reference(
                world,
                reference,
                priority as u8,
                &mut load,
            ) {
                failure = Some(stop);
            }
        };
        super::cell_reference_loop::finish_attach_reference_loop(
            world, cell, priority, &mut queue,
        )?;
        if let Some(stop) = failure {
            return Err(stop);
        }
    } else {
        count(world, 0x004D_5BD0);
        count(world, if buffered { 0x004C_B790 } else { 0x0043_8060 });
    }
    if run_havok {
        count(world, 0x004D_4D00);
        // 004D5D4C 004D4D00: an empty CellMopp makes 00532F50 return at once (`cell_havok::cell_center_havok_world`).
        super::cell_havok::cell_center_havok_world(world, cell)?;
    }
    set_process_level(world, cell, 6);
    Ok(())
}

// IDA: 004C79A0 TESObjectLAND_LoadForGrid(attach): 0057BAC0 true clears `attach`. Bit 3 set: a null block +0x00 (all here) with `attach` runs 004C5640; answers 1. Otherwise bit 10, or bits 0..2 with a file (0046B680(-1)), takes 004C64E0 and 004C4C80 the file read, a host stop; the rest 004C64E0, the same attach, and 1. Completion: 100% but 004C4C80.
// TODO: [ ] 004C7A00 004C4C80, a LAND read outside 004D1340. The master's own cells load theirs under TLS +0x186, but a child world's LAND has bit 0x400 (004BFDC0, skipping 004C64E0 at 004C7117) and a non-master plugin's takes 004C79DD, so **an sStartingCell in a child world or such a plugin stops here**.
fn land_load_for_grid(
    world: &mut World,
    land: Entity,
    services: &TesGridServices<'_, '_>,
) -> Result<bool, GameError> {
    let attach = !services.answers.loading_menu_hidden;
    let data = world.get::<TesObjectLand>(land).expect("a land");
    let flags = data.flags_1c;
    if flags & 8 == 0 {
        let with_file = world
            .get::<crate::tes::form::ModReferences>(land)
            .is_some_and(|mods| !mods.0.is_empty());
        if flags & 0x400 != 0 || (flags & 7 != 0 && with_file) {
            return Err(GameError::unclassified(
                0x004C7A00,
                "004C7A00: 004C4C80 reads a LAND outside 004D1340",
            ));
        }
        allocate_land_data(world, land)?;
    }
    if attach {
        count(world, 0x004C_5640);
    }
    Ok(true)
}

/// IDA: 004CACE0 TESObjectCELL_GetWaterHeight: +0x24 bit 1 takes the type 4 extra's float (0041F870: 0.0 with none); otherwise -FLT_MAX. Completion: 100%.
#[must_use]
pub fn cell_water_height(world: &World, cell: Entity) -> f32 {
    let data = world.get::<TesObjectCell>(cell).expect("a cell");
    if data.flags_24 & 2 == 0 {
        return f32::MIN;
    }
    match data.extra_28.get(4).map(|entry| &entry.data) {
        Some(ExtraData::Float(bits)) => f32::from_bits(*bits),
        _ => 0.0,
    }
}

/// IDA: N/A which water record a call is on: a grid slot's +0x04 or TES +0x58.
#[derive(Clone, Copy, Debug)]
pub enum WaterRecordSlot {
    Grid(u32, u32),
    Interior,
}

// IDA: N/A the record out of TES for a call that reads the world, and back.
fn with_water_record<R>(
    world: &mut World,
    slot: WaterRecordSlot,
    call: impl FnOnce(&mut World, &mut TesWaterNodeRecord) -> Result<R, GameError>,
) -> Result<R, GameError> {
    tes(world)?;
    let tes_data = tes_mut(world);
    let mut record = match slot {
        WaterRecordSlot::Grid(i, j) => tes_data
            .grid_cells_08
            .as_mut()
            .ok_or(GameError::unclassified(
                0x00482150,
                "00482150: TES +0x08 is null",
            ))?
            .slot_at(i, j)?
            .water_04
            .take(),
        WaterRecordSlot::Interior => tes_data.water_node_58.take(),
    }
    .ok_or(GameError::unclassified(
        0x0049A000,
        "0049A000: a null water record",
    ))?;
    let result = call(world, &mut record);
    let tes_data = tes_mut(world);
    match slot {
        WaterRecordSlot::Grid(i, j) => {
            tes_data
                .grid_cells_08
                .as_mut()
                .expect("tested")
                .slot_at(i, j)?
                .water_04 = Some(record);
        }
        WaterRecordSlot::Interior => tes_data.water_node_58 = Some(record),
    }
    result
}

/// IDA: 0043FED0 TES_BufferExteriorCell(cell). Start 0 for a level 5 or 6 cell, for a null entry at uGridsToLoad^2 - 1, or when that square isn't below the length; otherwise at the square. Each entry takes the carried cell until the cell's old place. A carried cell past the end is 00447BA0's, a host stop. Completion: 100% with no cell dropped. The C++ stops on a buffer as long as the grid, where the exe starts at 0.
/// # Errors
/// A cell dropped, or a grid square past the buffer, which the exe overruns.
pub fn buffer_exterior_cell(
    world: &mut World,
    cell: Entity,
    services: &TesGridServices<'_, '_>,
) -> Result<(), GameError> {
    let grids = services.grids()?;
    let length = services.ini_raw(EXTERIOR_BUFFER)?;
    let square = grids.wrapping_mul(grids);
    let level = process_level(world, cell);
    let buffer = &tes(world)?.exterior_cell_buffer_3c;
    let last_free = buffer
        .get(square.wrapping_sub(1) as usize)
        .ok_or(GameError::unclassified(
            0x0043FEF6,
            "0043FEF6: the grid past the exterior buffer",
        ))?
        .is_none();
    let from_start = level == 5 || level == 6 || last_free || square >= length;
    let buffer = &mut tes_mut(world).exterior_cell_buffer_3c;
    let mut carried = Some(cell);
    for index in (if from_start { 0 } else { square })..length {
        let previous = buffer[index as usize];
        buffer[index as usize] = carried;
        if previous == Some(cell) {
            return Ok(());
        }
        carried = previous;
    }
    if carried.is_some() {
        return Err(GameError::unclassified(
            0x0043FF34,
            "0043FF34: the exterior buffer drops a cell (00447BA0)",
        ));
    }
    Ok(())
}

/// IDA: 0043FD70 TES_BufferInteriorCell(cell): a held cell moves to the front; otherwise the last entry's cell goes to 00447BA0, a host stop, the others shift, and the cell takes the front. Completion: 100% with no cell dropped.
/// # Errors
/// A cell dropped.
pub fn buffer_interior_cell(
    world: &mut World,
    cell: Entity,
    services: &TesGridServices<'_, '_>,
) -> Result<(), GameError> {
    let length = services.ini_raw(INTERIOR_BUFFER)? as usize;
    tes(world)?;
    let buffer = &mut tes_mut(world).interior_cell_buffer_38;
    if let Some(at) = buffer
        .iter()
        .take(length)
        .position(|entry| *entry == Some(cell))
    {
        for index in (1..=at).rev() {
            buffer[index] = buffer[index - 1];
        }
        buffer[0] = Some(cell);
        return Ok(());
    }
    if length == 0 {
        return Err(GameError::unclassified(
            0x0043FE20,
            "0043FE20: an empty interior buffer",
        ));
    }
    if buffer[length - 1].is_some() {
        return Err(GameError::unclassified(
            0x0043FDF9,
            "0043FDF9: the interior buffer drops a cell (00447BA0)",
        ));
    }
    for index in (1..length).rev() {
        buffer[index] = buffer[index - 1];
    }
    buffer[0] = Some(cell);
    Ok(())
}

// IDA: N/A a helper, with no address of its own.
fn buffer_cell(
    world: &mut World,
    cell: Entity,
    services: &TesGridServices<'_, '_>,
) -> Result<(), GameError> {
    if cell_is_interior(world, cell) {
        buffer_interior_cell(world, cell, services)
    } else {
        buffer_exterior_cell(world, cell, services)
    }
}

// IDA: 0043FFF0 TES_UnloadExteriorBufferCells(keep, world): a nonnull entry, with `keep` and level 5 or 6 (table 00440080) kept, 004CA030's lock kept, another world's kept; the rest go to 00447BA0, cleared and counted. Completion: 100% of an empty buffer, since 00447BA0 unloads a cell.
// TODO: [ ] 00440052 00447BA0.
fn unload_exterior_buffer_cells(world: &World, keep: bool) -> Result<u32, GameError> {
    for entry in tes(world)?.exterior_cell_buffer_3c.iter().flatten() {
        if keep && matches!(process_level(world, *entry), 5 | 6) {
            continue;
        }
        return Err(GameError::unclassified(
            0x00440052,
            "00440052: 00447BA0 unloads a buffered exterior cell",
        ));
    }
    Ok(0)
}

/// IDA: 0043FA40 TES_GetExteriorCellAtGrid(x, y): the slot at (x + g/2 - +0x20, y + g/2 - +0x24), by unsigned tests, when in range and holding a cell; otherwise 00447740(x, y, +0x74, 0) with a world. Completion: 100%.
/// # Errors
/// A stop in 00447740.
pub fn exterior_cell_at_grid(
    world: &mut World,
    x: i32,
    y: i32,
    services: &mut TesGridServices<'_, '_>,
) -> Result<Option<Entity>, GameError> {
    let grids = services.grids()?;
    let half = grids >> 1;
    let data = tes(world)?;
    let i = half
        .wrapping_sub(data.exterior_coordinates_20[0] as u32)
        .wrapping_add(x as u32);
    let j = half
        .wrapping_sub(data.exterior_coordinates_20[1] as u32)
        .wrapping_add(y as u32);
    let space = data.current_world_space_74;
    if i < grids
        && j < grids
        && let Some(cell) = slot_cell(world, i, j)?
    {
        return Ok(Some(cell));
    }
    match space {
        Some(space) => {
            lookup_or_create_exterior_cell(world, x, y, Some(space), false, services.load)
        }
        None => Ok(None),
    }
}

/// IDA: 00440590 TES_UpdateCurrentExteriorCell(position, &height): `fistp` of x and y (round to nearest); an exact multiple of 4096 whose `fild` lies above the coordinate steps down one; >> 12. height = -2048.0. +0x30 refreshes through 0043FA40 when unset or on another grid. A cell of level 2..6 runs 004CE3C0 and 004C5B50, whose answer and height it returns; otherwise false, with the -2048.0. 004CE3C0's null, for a persistent cell, is `this` in 004C5B50, where the original faults: a stop. Completion: 100%.
/// # Errors
/// A stop in 0043FA40 or 004CE3C0.
pub fn update_current_exterior_cell(
    world: &mut World,
    position: [f32; 3],
    services: &mut TesGridServices<'_, '_>,
) -> Result<(bool, f32), GameError> {
    let step = |rounded: i32, value: f32| {
        if rounded.wrapping_rem(4096) == 0 && f64::from(rounded) > f64::from(value) {
            rounded.wrapping_sub(1)
        } else {
            rounded
        }
    };
    let x = step(x87::fistp(position[0]), position[0]) >> 12;
    let y = step(x87::fistp(position[1]), position[1]) >> 12;
    let current = tes(world)?.current_exterior_cell_30;
    let same =
        current.is_some_and(|cell| cell_grid_x(world, cell) == x && cell_grid_y(world, cell) == y);
    if !same {
        let cell = exterior_cell_at_grid(world, x, y, services)?;
        tes_mut(world).current_exterior_cell_30 = cell;
    }
    let Some(cell) = tes(world)?.current_exterior_cell_30 else {
        return Ok((false, -2048.0));
    };
    if !(2..=6).contains(&process_level(world, cell)) {
        return Ok((false, -2048.0));
    }
    let land = cell_get_or_create_land(world, cell)?.ok_or(GameError::unclassified(
        0x00440670,
        "00440670: 004C5B50 on a persistent cell's null",
    ))?;
    land_height_at_position(world, land, position)
}

/// IDA: 004430F0 TES_CountGridReferences(mode). Over the square around +0x20, +0x24, x outer. Mode 0 skips a slot whose 004821B0 holds. Mode 1 skips when 00482150's answer is nonzero: that's the slot's address, never null, so every slot skips and the count is 0: the C++ read the slot's cell (00443149..00443152). Others skip nothing. Then 004F1630, else 00447740(create 1); a cell not at level 3 adds its +0x48 items. Nonzero runs 00440AF0(1, 0, 0), a counted boundary. Answers the count. Completion: 100%.
/// # Errors
/// A stop in the loads.
pub fn count_grid_references(
    world: &mut World,
    mode: u32,
    services: &mut TesGridServices<'_, '_>,
) -> Result<u32, GameError> {
    let grids = services.grids()?;
    let half = (grids >> 1) as i32;
    let data = tes(world)?;
    let (x0, y0) = (
        data.exterior_coordinates_20[0].wrapping_sub(half),
        data.exterior_coordinates_20[1].wrapping_sub(half),
    );
    let mut total = 0_u32;
    for i in 0..grids {
        for j in 0..grids {
            let skip = match mode {
                0 => slot_attached(world, i, j)?,
                1 => true,
                _ => false,
            };
            let space = tes(world)?.current_world_space_74;
            let Some(space) = space.filter(|_| !skip) else {
                continue;
            };
            let (x, y) = (x0.wrapping_add(i as i32), y0.wrapping_add(j as i32));
            let mut cell = find_master_exterior_cell(world, space, x, y, services.load)?;
            if cell.is_none() {
                cell =
                    lookup_or_create_exterior_cell(world, x, y, Some(space), true, services.load)?;
            }
            let Some(cell) = cell else {
                continue;
            };
            if process_level(world, cell) != 3 {
                let items = world
                    .get::<TesObjectCell>(cell)
                    .expect("a cell")
                    .objects_48
                    .items()
                    .count();
                total = total.wrapping_add(items as u32);
            }
        }
    }
    if total != 0 {
        count(world, 0x0044_0AF0);
    }
    world
        .get_resource_or_init::<TesGridBoundaries>()
        .reference_counts
        .push(total);
    Ok(total)
}

/// IDA: 004441A0 TES_LoadGridSlot(i, j, x, y). No +0x74 returns. 004F1630, else 00447740(create 1) and 004C9F90(1) (a null faults). An attached slot only re-buffers the cell. Otherwise the unused text; 00482170(i, j, cell): the slot, then 0049A000; 00482390(root, i, j): 004D63A0, then 0049A000. 004CE3C0, 004C79A0(1); false runs 004C6730. Then 004C46B0's minimum against 004CACE0 by `fcomp` (`test ah, 0x41`): above sets +0x24 bit 1, anything else clears. Without it 00499FF0 on the slot's record. 004C5BA0(TES +0x53), 004E7610 with +0x44: counted. Completion: 100%.
/// # Errors
/// A stop in the loads.
pub fn load_grid_slot(
    world: &mut World,
    i: u32,
    j: u32,
    x: i32,
    y: i32,
    services: &mut TesGridServices<'_, '_>,
) -> Result<(), GameError> {
    let Some(space) = tes(world)?.current_world_space_74 else {
        return Ok(());
    };
    let mut found = find_master_exterior_cell(world, space, x, y, services.load)?;
    if found.is_none() {
        found = lookup_or_create_exterior_cell(world, x, y, Some(space), true, services.load)?;
        let cell = found.ok_or(GameError::unclassified(
            0x004441F9,
            "004441F9: 004C9F90 on a null cell",
        ))?;
        world
            .get_mut::<TesObjectCell>(cell)
            .expect("a cell")
            .flags_24 |= 0x10;
    }
    if slot_attached(world, i, j)? {
        if let Some(cell) = found {
            buffer_cell(world, cell, services)?;
        }
        return Ok(());
    }
    let cell = found.ok_or(GameError::unclassified(
        0x00444247,
        "00444247: +0x0C of a null cell",
    ))?;
    set_slot_cell(world, i, j, Some(cell))?;
    with_water_record(world, WaterRecordSlot::Grid(i, j), |world, record| {
        set_water_node_cell(world, record, Some(cell))
    })?;
    attach_cell(world, cell, services)?;
    with_water_record(world, WaterRecordSlot::Grid(i, j), |world, record| {
        set_water_node_cell(world, record, Some(cell))
    })?;
    let land = cell_get_or_create_land(world, cell)?.ok_or(GameError::unclassified(
        0x00444295,
        "00444295: 004C79A0 on a null land",
    ))?;
    if !land_load_for_grid(world, land, services)? {
        count(world, 0x004C_6730);
    }
    let range = land_height_range(world, land);
    let above = cell_water_height(world, cell) > range[0];
    let mut data = world.get_mut::<TesObjectCell>(cell).expect("a cell");
    if above {
        data.flags_24 |= 2;
    } else {
        data.flags_24 &= !2;
    }
    if !above {
        with_water_record(world, WaterRecordSlot::Grid(i, j), |world, record| {
            hide_water_node_record(world, record)
        })?;
    }
    count(world, 0x004C_5BA0);
    if world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .path_grid_44
        .is_some()
    {
        count(world, 0x004E_7610);
    }
    Ok(())
}

// IDA: 00444483..004444CC, x87 24-bit: `fild` the count over `fild` the square (a value with the sign bit adds 00A2FC78's 2^32), `fdivp` rounds to float, `fmul qword` 100.0 rounds to float. Completion: 100%.
fn section_percent(done: i32, square: u32) -> f32 {
    let mut total = square as i32 as f32;
    if (square as i32) < 0 {
        total += 4_294_967_296.0;
    }
    (f64::from(done as f32 / total) * 100.0) as f32
}

/// IDA: 00444340 TES_FillExteriorGrid. 0057BAC0 twice. 004E9F40 (00B3608F) runs 00483D60(+0x74). g_saveLoadGame +0x18 |= 0x100; 00889E00. Each slot, x outer: 004441A0; 00447740(0): a buffered cell re-buffers; the progress with 0057BAC0 or an active loading save: 004523A0 for the save, else 0057B950. 0088D1D0(0); +0x18 &= ~0x100; a +0x1C runs 0045FDA0, a host stop; 00889E00. Then each slot again with a world: 00447740(0), and a nonnull cell: at level 5, 004D5BD0(!first answer), the buffer, 004D58B0, 00707610, 00707980 on it and on TES +0x10, 00707370 with no +0xB8; the +0x78 list: an old head item moves to a new node at +0x7C's head; +0x78 = the cell. 0088D1D0(0); 00B4310C released; 00B06A28 = 1. Completion: 100%.
/// # Errors
/// A stop in the loads.
pub fn fill_exterior_grid(
    world: &mut World,
    services: &mut TesGridServices<'_, '_>,
) -> Result<(), GameError> {
    let grids = services.grids()?;
    let half = (grids >> 1) as i32;
    let data = tes(world)?;
    let (x0, y0) = (
        data.exterior_coordinates_20[0].wrapping_sub(half),
        data.exterior_coordinates_20[1].wrapping_sub(half),
    );
    let hidden = services.answers.loading_menu_hidden;
    if world
        .get_resource::<WorldFormGlobals>()
        .is_some_and(|globals| globals.display_lod_land_b3608f != 0)
    {
        count(world, 0x0048_3D60);
    }
    world
        .resource_mut::<TesGlobals>()
        .save_load_game
        .as_mut()
        .expect("tested")
        .fields_14[1] |= 0x100;
    let havok = havok_world(world);
    if havok {
        count(world, 0x0088_9E00);
    }
    let mut progress = 0_i32;
    for i in 0..grids {
        for j in 0..grids {
            let (x, y) = (x0.wrapping_add(i as i32), y0.wrapping_add(j as i32));
            load_grid_slot(world, i, j, x, y, services)?;
            let space = tes(world)?.current_world_space_74;
            if let Some(cell) =
                lookup_or_create_exterior_cell(world, x, y, space, false, services.load)?
                && in_buffer(world, cell, services)?
            {
                buffer_cell(world, cell, services)?;
            }
            let flags = save_load_flags(world)?;
            let saving = flags & 1 != 0 && flags & 0x800 != 0;
            if hidden || saving {
                progress += 1;
                let percent = section_percent(progress, grids.wrapping_mul(grids));
                if saving {
                    count(world, 0x0045_23A0);
                } else {
                    count(world, 0x0057_B950);
                    world
                        .resource_mut::<TesGridBoundaries>()
                        .percents
                        .push(percent);
                }
            }
        }
    }
    if havok {
        count(world, 0x0088_D1D0);
    }
    {
        let mut globals = world.resource_mut::<TesGlobals>();
        let game = globals.save_load_game.as_mut().expect("tested");
        game.fields_14[1] &= !0x100;
        if game.fields_14[2] != 0 {
            return Err(GameError::unclassified(
                0x00444557,
                "00444557: 0045FDA0 with g_saveLoadGame +0x1C",
            ));
        }
    }
    if havok {
        count(world, 0x0088_9E00);
    }
    for i in 0..grids {
        for j in 0..grids {
            let Some(space) = tes(world)?.current_world_space_74 else {
                continue;
            };
            let (x, y) = (x0.wrapping_add(i as i32), y0.wrapping_add(j as i32));
            let Some(cell) =
                lookup_or_create_exterior_cell(world, x, y, Some(space), false, services.load)?
            else {
                continue;
            };
            if process_level(world, cell) == 5 {
                finish_attach(world, cell, !hidden, services)?;
                buffer_cell(world, cell, services)?;
                cell_node(world, cell)?;
                count(world, 0x0070_7610);
                count(world, 0x0070_7980);
                count(world, 0x0070_7610);
                count(world, 0x0070_7980);
                count(world, 0x0070_7370);
            }
            let tes_data = tes_mut(world);
            if let Some(head) = tes_data.exterior_cell_list_78 {
                tes_data.exterior_cell_list_next_7c.insert(0, head);
            }
            tes_data.exterior_cell_list_78 = Some(cell);
        }
    }
    if havok {
        count(world, 0x0088_D1D0);
    }
    if let Some(mut trees) = world.get_resource_mut::<TreeManagerGlobals>() {
        trees.pointer_b4310c = None;
    }
    world.resource_mut::<TesGlobals>().grid_changed_b06a28 = 1;
    Ok(())
}

// IDA: 004EBDA0 with 00B2CBC0 cleared around it, as 00444DAD and 00445CD3 hold it. Completion: 100% of the guard.
fn lod_update(world: &mut World) {
    grid_globals(world).lod_update_b2cbc0 = 0;
    count(world, 0x004E_BDA0);
    grid_globals(world).lod_update_b2cbc0 = 1;
}

// IDA: 004402F0 under its guard: 00B33A58 clear and 004F9FA0: the quests' 00529AA0, the interior's or attached slots' 004CB8C0, the +0x88 list's 004D7190 and free, 004FA580, a counted boundary. Completion: 100% of the guard.
fn run_grid_scripts(world: &mut World, services: &TesGridServices<'_, '_>) {
    if grid_globals(world).scripts_b33a58 == 0 && services.answers.scripts_ready {
        count(world, 0x0044_02F0);
    }
}

/// IDA: 00444C70 TES_LoadExteriorGrid(position, reload). No g_dataHandler, an interior, or 00B33A69 set returns. 00B33A69 = 1. Reload: vt+8 on +0x08 and +0x04, 00B06A28 = 1, 00B4310C released; vt+0x10(+0x20, +0x24) on both; 0043FFF0(0, 0), 0043FC20(0), 004430F0(0), 00444340, 00434020(4); 00440590; 004EBDA0; unless a cell test 005732D0; 00441610, 00678750, 00675F40, 00675FC0. Otherwise vt+0x10 on both, 004430F0(1), 00444340, 004EF7E0(+0x74), 004EA6E0(the player's position, 1). Then 004402F0, 00707370 on TES +0x10, 0043FC20(0), 00447130; 00B33A69 = 0. Completion: 100% of the control flow.
/// # Errors
/// A stop in the loads, or a reload with a null position, which faults.
pub fn load_exterior_grid(
    world: &mut World,
    position: Option<[f32; 3]>,
    reload: bool,
    services: &mut TesGridServices<'_, '_>,
) -> Result<(), GameError> {
    let data = tes(world)?;
    let interior = data.current_interior_cell_34.is_some();
    let (x, y) = (
        data.exterior_coordinates_20[0],
        data.exterior_coordinates_20[1],
    );
    if !world.contains_resource::<DataHandler>()
        || interior
        || grid_globals(world).loading_b33a69 != 0
    {
        return Ok(());
    }
    grid_globals(world).loading_b33a69 = 1;
    if reload {
        clear_all_cells(world)?;
        clear_all_distant(world)?;
        world.resource_mut::<TesGlobals>().grid_changed_b06a28 = 1;
        if let Some(mut trees) = world.get_resource_mut::<TreeManagerGlobals>() {
            trees.pointer_b4310c = None;
        }
        recenter_cells(world, x, y)?;
        recenter_distant(world, x, y)?;
        unload_exterior_buffer_cells(world, false)?;
        count(world, 0x0043_FC20);
        count_grid_references(world, 0, services)?;
        fill_exterior_grid(world, services)?;
        count(world, 0x0043_4020);
        let position = position.ok_or(GameError::unclassified(
            0x00444D52,
            "00444D52: 00444C70(reload) on a null position",
        ))?;
        update_current_exterior_cell(world, position, services)?;
        lod_update(world);
        let data = tes(world)?;
        if data.cell_test_active_51 == 0 && data.byte_52 == 0 {
            count(world, 0x0057_32D0);
        }
        count(world, 0x0044_1610);
        count(world, 0x0067_8750);
        count(world, 0x0067_5F40);
        count(world, 0x0067_5FC0);
    } else {
        recenter_cells(world, x, y)?;
        recenter_distant(world, x, y)?;
        count_grid_references(world, 1, services)?;
        fill_exterior_grid(world, services)?;
        count(world, 0x004E_F7E0);
        count(world, 0x004E_A6E0);
    }
    run_grid_scripts(world, services);
    count(world, 0x0070_7370);
    count(world, 0x0043_FC20);
    close_all_files(world, services)?;
    grid_globals(world).loading_b33a69 = 0;
    Ok(())
}

/// IDA: 00444EC0 TES_UpdateExteriorGridFromPosition(position, reload). Open with g_dataHandler, no +0x34 and TLS +0x184 clear, or with g_saveLoadGame +0x18 bit 11; closed answers that bit's false. A null +0x74 takes the +0x0C head through 004431F0, whose resets the saved +0x20, +0x24 undo when there is no position. +0x20, +0x24 = `fstp` float, `fistp` (nearest), `sar 12`; +0x28, +0x2C copy. 00444C70. Without an interior 00499E40; 0049A200 unless 0045A500; 00537D40. Answers 00537D40's eax, a boundary here: true. Completion: 100% of the control flow.
/// # Errors
/// A stop below.
pub fn update_exterior_grid_from_position(
    world: &mut World,
    position: Option<[f32; 3]>,
    reload: bool,
    services: &mut TesGridServices<'_, '_>,
) -> Result<bool, GameError> {
    let flags = save_load_flags(world)?;
    let data = tes(world)?;
    let open = world.contains_resource::<DataHandler>()
        && data.current_interior_cell_34.is_none()
        && world.resource::<GameThreadLocals>().byte_184 == 0;
    if !open && flags & 0x800 == 0 {
        return Ok(false);
    }
    let (old_x, old_y) = (
        data.exterior_coordinates_20[0],
        data.exterior_coordinates_20[1],
    );
    if data.current_world_space_74.is_none() {
        let head = world
            .resource::<DataHandlerLists>()
            .world_spaces_0c
            .items()
            .next();
        services.set_world(world, head)?;
    }
    let tes_data = tes_mut(world);
    match position {
        Some(position) => {
            tes_data.exterior_coordinates_20[0] = x87::fistp(position[0]) >> 12;
            tes_data.exterior_coordinates_20[1] = x87::fistp(position[1]) >> 12;
        }
        None => {
            tes_data.exterior_coordinates_20[0] = old_x;
            tes_data.exterior_coordinates_20[1] = old_y;
        }
    }
    tes_data.exterior_coordinates_20[2] = tes_data.exterior_coordinates_20[0];
    tes_data.exterior_coordinates_20[3] = tes_data.exterior_coordinates_20[1];
    load_exterior_grid(world, position, reload, services)?;
    if tes(world)?.current_interior_cell_34.is_none() {
        count(world, 0x0049_9E40);
    }
    if !save_load_active(world)? {
        count(world, 0x0049_A200);
    }
    count(world, 0x0053_7D40);
    Ok(true)
}

/// IDA: 00482310 GridCellArray_InitializeLoadedCells. +0x20 set returns. Each slot cell, i outer: 004CE3C0, 004C5640 on its land, 004D4D00. 0049A200 on TES +0x54. +0x20 = 1. Completion: 100%, with those counted.
/// # Errors
/// A stop in 004CE3C0.
pub fn initialize_loaded_cells(world: &mut World) -> Result<(), GameError> {
    let size = cell_array_size(world)?;
    if tes(world)?.grid_cells_08.as_ref().expect("tested").byte_20 != 0 {
        return Ok(());
    }
    for i in 0..size {
        for j in 0..size {
            if let Some(cell) = slot_cell(world, i, j)? {
                cell_get_or_create_land(world, cell)?;
                count(world, 0x004C_5640);
                count(world, 0x004D_4D00);
            }
        }
    }
    count(world, 0x0049_A200);
    tes_mut(world)
        .grid_cells_08
        .as_mut()
        .expect("tested")
        .byte_20 = 1;
    Ok(())
}

// IDA: 00440270 TES_UpdateWaterSurfaceForGrid: a slot cell with +0x24 bit 1 runs 00498F30 only with 00B0703C clear; none runs 0049A8B0(1) only with it set. The C++ ran one or the other whatever the flag. Completion: 100%.
fn update_water_surface(
    world: &mut World,
    services: &TesGridServices<'_, '_>,
) -> Result<(), GameError> {
    let grids = services.grids()?;
    let mut water = false;
    for i in 0..grids {
        for j in 0..grids {
            if slot_cell(world, i, j)?.is_some_and(|cell| {
                world.get::<TesObjectCell>(cell).expect("a cell").flags_24 & 2 != 0
            }) {
                water = true;
            }
        }
    }
    let flag = grid_globals(world).water_surface_b0703c != 0;
    if water {
        if !flag {
            water_surface_on(world);
        }
    } else if flag {
        water_surface_off(world);
    }
    Ok(())
}

// IDA: 00441E52..00441E76, 00445B65..00445B73: TES +0x10's parent's +0x18 bit 0. Null at either step skips. Completion: 100%.
fn set_land_lod_parent_culled(world: &mut World, culled: bool) -> Result<(), GameError> {
    let Some(lod) = tes(world)?.land_lod_10 else {
        return Ok(());
    };
    let Some(parent) = world.get::<NiAvParent>(lod).and_then(|parent| parent.0) else {
        return Ok(());
    };
    if let Some(mut payload) = world.get_mut::<SceneNodePayload>(parent) {
        if culled {
            payload.effective_av_flags |= 1;
        } else {
            payload.effective_av_flags &= 0xFFFE;
        }
    }
    Ok(())
}

/// IDA: 00441D50 TES_SetGridCellsDetached(detached, unused). No g_dataHandler returns. 00889E10 or 00889E00. Each slot, i outer: detached: a level 6 cell's 004D6450, a host stop, then 00499FF0 on the slot's record; attached: a level 3 cell's 00482390 and 004D5BD0(0). 0088BD60(0) or 0088D1D0(0). Attached: 00677360, and 0049A200 only with 00B0703C. The land LOD's parent's bit, 00707370 on TES +0x10. Completion: 100%.
/// # Errors
/// As above.
pub fn set_grid_cells_detached(
    world: &mut World,
    detached: bool,
    services: &mut TesGridServices<'_, '_>,
) -> Result<(), GameError> {
    if !world.contains_resource::<DataHandler>() {
        return Ok(());
    }
    let havok = havok_world(world);
    if havok {
        count(world, if detached { 0x0088_9E10 } else { 0x0088_9E00 });
    }
    let grids = services.grids()?;
    for i in 0..grids {
        for j in 0..grids {
            let cell = slot_cell(world, i, j)?;
            if detached {
                if cell.is_some_and(|cell| process_level(world, cell) == 6) {
                    return Err(GameError::unclassified(
                        0x00441DBD,
                        "00441DBD: 004D6450 detaches a grid cell",
                    ));
                }
                with_water_record(world, WaterRecordSlot::Grid(i, j), |world, record| {
                    hide_water_node_record(world, record)
                })?;
            } else if let Some(cell) = cell.filter(|&cell| process_level(world, cell) == 3) {
                attach_cell(world, cell, services)?;
                with_water_record(world, WaterRecordSlot::Grid(i, j), |world, record| {
                    set_water_node_cell(world, record, Some(cell))
                })?;
                finish_attach(world, cell, false, services)?;
            }
        }
    }
    if havok {
        count(world, if detached { 0x0088_BD60 } else { 0x0088_D1D0 });
    }
    if !detached {
        count(world, 0x0067_7360);
        if grid_globals(world).water_surface_b0703c != 0 {
            count(world, 0x0049_A200);
        }
    }
    set_land_lod_parent_culled(world, detached)?;
    count(world, 0x0070_7370);
    Ok(())
}

/// IDA: 00444FB0 TES_UpdateExteriorGrid(position, allow). The +0x7C nodes freed, +0x78 = 0. No g_dataHandler answers 0. With either coordinate unset: a position stores +0x20..+0x2C; 00440AF0(1, 0, 0); 00444EC0(position, 0); answers 1. Otherwise the margins: half is 4096.0, 3072.0 at uGridsToLoad 3; half - |fistp x - (+0x20 << 12) - 0x800| to +0x6C, the same for y to +0x70, each `fisub` 24-bit. Both above 0 with no loading save: the 00B08960 distant requests under half * 0.5 move +0x28, +0x2C (004BE090 per grid line, counted), answers 0. Past that (00445200) the grid moves, a host stop. Completion: 100% of the arms but the move.
/// # Errors
/// A moved grid, or a stop below.
/// TODO: [ ] 00445200..0044555C the moved grid, from a player's move (Phase 17+).
pub fn update_exterior_grid(
    world: &mut World,
    position: Option<[f32; 3]>,
    services: &mut TesGridServices<'_, '_>,
) -> Result<bool, GameError> {
    tes(world)?;
    {
        let tes_data = tes_mut(world);
        tes_data.exterior_cell_list_next_7c.clear();
        tes_data.exterior_cell_list_78 = None;
    }
    if !world.contains_resource::<DataHandler>() {
        return Ok(false);
    }
    let grids = services.grids()?;
    let data = tes(world)?;
    let coordinates = data.exterior_coordinates_20;
    if coordinates[0] == 0x7FFF_FFFF || coordinates[1] == 0x7FFF_FFFF {
        if let Some(position) = position {
            let tes_data = tes_mut(world);
            tes_data.exterior_coordinates_20[0] = x87::fistp(position[0]) >> 12;
            tes_data.exterior_coordinates_20[1] = x87::fistp(position[1]) >> 12;
            tes_data.exterior_coordinates_20[2] = tes_data.exterior_coordinates_20[0];
            tes_data.exterior_coordinates_20[3] = tes_data.exterior_coordinates_20[1];
        }
        count(world, 0x0044_0AF0);
        update_exterior_grid_from_position(world, position, false, services)?;
        return Ok(true);
    }
    let position = position.ok_or(GameError::unclassified(
        0x00445005,
        "00445005: 00444FB0 on a null position",
    ))?;
    let (px, py) = (x87::fistp(position[0]), x87::fistp(position[1]));
    let half: f32 = if grids == 3 { 3072.0 } else { 4096.0 };
    let dx = px
        .wrapping_sub(coordinates[0].wrapping_shl(12))
        .wrapping_sub(0x800);
    let dy = py
        .wrapping_sub(coordinates[1].wrapping_shl(12))
        .wrapping_sub(0x800);
    let margin_x = (f64::from(half) - f64::from(dx.wrapping_abs())) as f32;
    let margin_y = (f64::from(half) - f64::from(dy.wrapping_abs())) as f32;
    {
        let tes_data = tes_mut(world);
        tes_data.grid_center_margin_x_6c = margin_x;
        tes_data.grid_center_margin_y_70 = margin_y;
    }
    let flags = save_load_flags(world)?;
    if !(margin_x > 0.0 && margin_y > 0.0) || flags & 0x800 != 0 {
        return Err(GameError::unclassified(
            0x00445200,
            "00445200: 00444FB0 moves a loaded grid",
        ));
    }
    let requests = services.ini_raw(BACKGROUND_CELL_LOADS)? as u8 != 0;
    if !requests || flags & 1 != 0 || tes(world)?.current_interior_cell_34.is_some() {
        return Ok(false);
    }
    let quarter = (f64::from(half) * 0.5) as f32;
    if !(margin_x < quarter || margin_y < quarter) {
        return Ok(false);
    }
    let (cx, cy) = (px >> 12, py >> 12);
    let half_grid = grids >> 1;
    let side = u64::from(half_grid) * 2 + 1;
    let (old_x, old_y) = (coordinates[2], coordinates[3]);
    if cx != old_x {
        for _ in 0..side {
            count(world, 0x004B_E090);
        }
        tes_mut(world).exterior_coordinates_20[2] = cx;
    }
    if cy != old_y {
        for _ in 0..side {
            count(world, 0x004B_E090);
        }
        tes_mut(world).exterior_coordinates_20[3] = cy;
    }
    if cx != old_x && cy != old_y {
        count(world, 0x004B_E090);
    }
    Ok(false)
}

/// IDA: 004455E0 TES_EnterInteriorCell(cell, position). See the body by address. The same cell runs 00444FB0(position, 0). From no interior; leaving one (004D6450, 007B84E0, 00660110) and 00440120 are host stops. Completion: 100% of the control flow from no interior.
/// # Errors
/// As above, or a stop below.
/// TODO: [ ] 004456D2 leaving another interior; 00445733 00440120.
pub fn enter_interior_cell(
    world: &mut World,
    cell: Option<Entity>,
    position: &mut [f32; 3],
    services: &mut TesGridServices<'_, '_>,
) -> Result<(), GameError> {
    let data = tes(world)?;
    let cell_test = data.cell_test_active_51 != 0 || data.byte_52 != 0;
    let current = data.current_interior_cell_34;
    grid_globals(world).transition_b43384 = 1;
    if !cell_test {
        count(world, 0x0040_D160);
    }
    grid_globals(world).interior_b43074 = 1;
    count(world, 0x007A_B6F0);
    // 0044561C: a null 00B35C24 faults.
    world
        .resource_mut::<HavokGlobals>()
        .world_b35c24
        .as_mut()
        .ok_or(GameError::unclassified(
            0x0044561C,
            "0044561C: +0x19 on a null 00B35C24",
        ))?
        .byte_19 = 0;
    count(world, 0x0088_B680);
    if current == cell {
        update_exterior_grid(world, Some(*position), services)?;
        grid_globals(world).transition_b43384 = 0;
        return Ok(());
    }
    // 00445654..00445684.
    if services.ini_raw(PREEMPTIVELY_UNLOAD_CELLS)? as u8 != 0 && current.is_none() {
        count(world, 0x004C_9300);
        unload_exterior_buffer_cells(world, true)?;
    }
    world
        .get_resource_mut::<crate::tes::os_globals::OsGlobalsData>()
        .ok_or(GameError::unclassified(
            0x0044568C,
            "0044568C: +0x18 on a null g_osGlobals",
        ))?
        .unk_18 = 0;
    // 0040FDA0 asked here, as the original asks: the thread can end while this block runs. No MoviePlayer is P15-RUN-008's false.
    let movie_running = world
        .get_resource::<crate::tes::movie_player::MoviePlayer>()
        .is_some_and(crate::tes::movie_player::movie_player_is_running);
    if cell.is_some() && !save_load_active(world)? && !movie_running {
        count(world, 0x0040_D4D0);
    }
    count(world, 0x007C_7E50);
    let had_interior = current.is_some();
    if had_interior {
        return Err(GameError::unclassified(
            0x004456D9,
            "004456D9: 004D6450 leaves another interior",
        ));
    }
    // 00445713..00445733 43FD40 with a null cell compares nulls too, and a null slot then answers true.
    let buffered = match cell {
        Some(cell) => in_interior_buffer(world, cell, services)?,
        None => {
            let length = services.ini_raw(INTERIOR_BUFFER)? as usize;
            tes(world)?
                .interior_cell_buffer_38
                .iter()
                .take(length)
                .any(Option::is_none)
        }
    };
    if !buffered {
        let length = services.ini_raw(INTERIOR_BUFFER)? as usize;
        if tes(world)?
            .interior_cell_buffer_38
            .get(length.wrapping_sub(1))
            .copied()
            .flatten()
            .is_some()
        {
            return Err(GameError::unclassified(
                0x00445733,
                "00445733: 00440120 unloads the last interior cell",
            ));
        }
    }
    tes_mut(world).current_interior_cell_34 = cell;
    if let Some(cell) = cell {
        // 00445743..00445765 only +0x48 is tested.
        if tes(world)?.last_exterior_coordinates_48[0] == 0x7FFF_FFFF {
            let tes_data = tes_mut(world);
            tes_data.last_exterior_coordinates_48[0] = tes_data.exterior_coordinates_20[0];
            tes_data.last_exterior_coordinates_48[1] = tes_data.exterior_coordinates_20[1];
            set_grid_cells_detached(world, true, services)?;
        }
        let tes_data = tes_mut(world);
        tes_data.exterior_coordinates_20[0] = x87::fistp(position[0]) >> 12;
        tes_data.exterior_coordinates_20[1] = x87::fistp(position[1]) >> 12;
        crate::tes::forms_world::load_interior_cell_temporary_data(world, cell, services.load)?;
        let active = save_load_active(world)?;
        let havok = cell_has_havok_world(world, cell);
        if havok {
            count(world, 0x0088_9E00);
        }
        let references = world
            .get::<TesObjectCell>(cell)
            .expect("a cell")
            .objects_48
            .items()
            .count();
        if references != 0 && process_level(world, cell) != 3 {
            // 00445803 00440AF0(1, 0, 0): the loading menu, through the lent interface.
            match services.interface.as_mut() {
                Some(interface) => interface(world, &mut |cx| {
                    crate::tes::tes::tes_show_loading_menu(cx).map(|_| ())
                })?,
                None => count(world, 0x0044_0AF0),
            }
        }
        attach_cell(world, cell, services)?;
        if world
            .get::<TesObjectCell>(cell)
            .expect("a cell")
            .path_grid_44
            .is_some()
        {
            count(world, 0x004E_7610);
        }
        finish_attach(world, cell, true, services)?;
        count(world, 0x0043_E000);
        if services.answers.cell_tasks_pending {
            count(world, 0x0044_0AF0);
        }
        count(world, 0x0044_0190);
        if !active {
            // 00445857 00434020(1), through the lent drain.
            match services.drain.as_mut() {
                Some(drain) => {
                    drain(world, 1)?;
                }
                None => count(world, 0x0043_4020),
            }
        }
        if havok {
            count(world, 0x0088_D1D0);
        }
        if !active {
            if !cell_test {
                count(world, 0x005A_DB40);
                // 00445886..00445898 RendererUIState_CreateFader(2, fFadeToBlackFadeSeconds (00B33A48)), through the lent
                // interface; without it (the startup tail) counted.
                let seconds = world
                    .get_resource::<crate::tes::game_settings::GameSettingCollection>()
                    .and_then(|settings| settings.find("fFadeToBlackFadeSeconds"))
                    .map_or(0.3, |setting| f32::from_bits(setting.raw));
                match services.interface.as_mut() {
                    Some(interface) => interface(world, &mut |cx| {
                        crate::tes::faders::create_fader(cx, 2, seconds)
                    })?,
                    None => count(world, 0x0057_32D0),
                }
            }
            count(world, 0x0067_7360);
            count(world, 0x0044_1610);
            count(world, 0x0067_8750);
            count(world, 0x0067_5F40);
            count(world, 0x0067_5FC0);
        }
        count(world, 0x0043_FC20);
        close_all_files(world, services)?;
        cell_node(world, cell)?;
        count(world, 0x0070_7610);
        count(world, 0x0070_7980);
        count(world, 0x0070_7370);
        count(world, 0x004C_B670);
        let water = world.get::<TesObjectCell>(cell).expect("a cell").flags_24 & 2 != 0;
        if water {
            if tes(world)?.water_surface_54.is_some() {
                water_surface_on(world);
            }
            with_water_record(world, WaterRecordSlot::Interior, |world, record| {
                set_water_node_cell(world, record, Some(cell))
            })?;
            let data = tes(world)?;
            if data.current_interior_cell_34.is_some() && data.water_node_58.is_some() {
                count(world, 0x0049_B5F0);
            }
        } else {
            if tes(world)?.water_node_58.is_some() {
                with_water_record(world, WaterRecordSlot::Interior, |world, record| {
                    hide_water_node_record(world, record)
                })?;
            }
            if tes(world)?.water_surface_54.is_some() {
                water_surface_off(world);
            }
        }
        buffer_interior_cell(world, cell, services)?;
    }
    if !had_interior {
        count(world, 0x007A_9CF0);
        count(world, 0x007C_4D90);
        count(world, 0x007B_2130);
        // 00482670: a +0x24 texture releases 00B4310C, the texture, +0x24, then 00B06A28 = 1. Null in P15-RUN-007.
        if tes_mut(world)
            .grid_cells_08
            .as_mut()
            .expect("tested")
            .pointer_24
            .take()
            .is_some()
        {
            count(world, 0x0048_2670);
            if let Some(mut trees) = world.get_resource_mut::<TreeManagerGlobals>() {
                trees.pointer_b4310c = None;
            }
            world.resource_mut::<TesGlobals>().grid_changed_b06a28 = 1;
        }
    }
    if cell_test {
        count(world, 0x0057_8E30);
    }
    count(world, 0x0043_FC20);
    count(world, 0x005A_89A0);
    grid_globals(world).transition_b43384 = 0;
    Ok(())
}

/// IDA: 00445A10 TES_ExitInteriorCell(position). See the body by address. Leaving a current interior (004D6450, 007C7E50, 00660110, 00447BA0 or 004CB010, 00440270, 00441D50(0) or 00440880) is a host stop: Phase 15 has none. Completion: 100% of the control flow from no interior.
/// # Errors
/// As above, or a stop below.
/// TODO: [ ] 00445A70..00445B61 leaving an interior, with the sky's 005403D0 at 00445D4C.
pub fn exit_interior_cell(
    world: &mut World,
    position: &mut [f32; 3],
    services: &mut TesGridServices<'_, '_>,
) -> Result<(), GameError> {
    let data = tes(world)?;
    let cell_test = data.cell_test_active_51 != 0 || data.byte_52 != 0;
    let current = data.current_interior_cell_34;
    grid_globals(world).transition_b43384 = 1;
    if !cell_test {
        count(world, 0x0040_D160);
    }
    grid_globals(world).interior_b43074 = 0;
    count(world, 0x007A_B6F0);
    count(world, 0x007B_2130);
    world
        .resource_mut::<HavokGlobals>()
        .world_b35c24
        .as_mut()
        .ok_or(GameError::unclassified(
            0x00445A5A,
            "00445A5A: +0x19 on a null 00B35C24",
        ))?
        .byte_19 = 1;
    if current.is_some() {
        return Err(GameError::unclassified(
            0x00445A72,
            "00445A72: 004D6450 leaves the current interior",
        ));
    }
    // 00445B65..00445B73, 00445B87 0088B680(00B097B8).
    set_land_lod_parent_culled(world, false)?;
    count(world, 0x0088_B680);
    let active = save_load_active(world)?;
    update_exterior_grid(world, Some(*position), services)?;
    // 00445BA7..00445BE8 `fild`, `fadd` qword 0.5 rounding to float, `fmul` by qword 4096.0.
    let data = tes(world)?;
    let centre = |value: i32| ((f64::from(value) + 0.5) as f32) * 4096.0;
    let (centre_x, centre_y) = (
        centre(data.exterior_coordinates_20[0]),
        centre(data.exterior_coordinates_20[1]),
    );
    let (x, y) = (
        data.exterior_coordinates_20[0],
        data.exterior_coordinates_20[1],
    );
    {
        let mut globals = grid_globals(world);
        globals.centre_b4312c = [centre_x, centre_y];
        globals.centre_copy_b43134 = [centre_x, centre_y];
        globals.game_time_b43078 = services.answers.game_time;
    }
    count(world, 0x00B4_2D78);
    update_water_surface(world, services)?;
    exterior_cell_at_grid(world, x, y, services)?;
    count(world, 0x0043_E000);
    if services.answers.cell_tasks_pending {
        count(world, 0x0044_0AF0);
    }
    if !active {
        let havok = havok_world(world);
        if havok {
            count(world, 0x0088_9E00);
        }
        count(world, 0x0043_4020);
        initialize_loaded_cells(world)?;
        if havok {
            count(world, 0x0088_D1D0);
        }
        update_current_exterior_cell(world, *position, services)?;
        lod_update(world);
        if !cell_test {
            count(world, 0x0057_32D0);
        }
        count(world, 0x0067_7360);
        count(world, 0x0044_1610);
        count(world, 0x0067_8750);
        count(world, 0x0067_5F40);
        count(world, 0x0067_5FC0);
    }
    // 00445D71 0043FB40: a level 6 slot cell's 004CB670(1), 007C7EB0.
    let grids = services.grids()?;
    for i in 0..grids {
        for j in 0..grids {
            if slot_cell(world, i, j)?.is_some_and(|cell| process_level(world, cell) == 6) {
                count(world, 0x004C_B670);
            }
        }
    }
    count(world, 0x007C_7EB0);
    let sky_dc = world
        .get_resource::<SkyGlobals>()
        .and_then(|globals| globals.singleton_b365c4.as_ref())
        .map_or(0, |sky| sky.dword_dc);
    if sky_dc != 0 {
        count(world, 0x0054_3BB0);
    }
    count(world, 0x0070_7610);
    count(world, 0x0070_7980);
    count(world, 0x005A_89A0);
    if cell_test {
        count(world, 0x0057_8E30);
    }
    count(world, 0x0043_FC20);
    let mut globals = grid_globals(world);
    globals.exterior_done_b3522c = 1;
    globals.transition_b43384 = 0;
    Ok(())
}

/// IDA: what 004CEE90 chose. `x_marker` is [esp+0x1C], `x_marker_heading` [esp+0x18], `door` the byte [esp+0x13], `type_1c` [esp+0x20], `last` [esp+0x24], `chosen` esi at 004CF0AF. `lift` holds the height inputs when an exterior cell had a land: the land height and the water height, as 004CF11E left them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlacementChoice {
    pub x_marker: Option<Entity>,
    pub x_marker_heading: Option<Entity>,
    pub door: bool,
    pub type_1c: Option<Entity>,
    pub last: Option<Entity>,
    pub chosen: Option<Entity>,
    pub lift: Option<(f32, f32)>,
    /// IDA: N/A the position and rotation at 004CEE90's entry, a host record.
    pub entry: ([f32; 3], [f32; 3]),
    /// IDA: N/A the door's point and rotation 004CEFD9 stored, a host record.
    pub door_point: Option<([f32; 3], [f32; 3])>,
}

// IDA: 006899C0 and 0042B430: three dwords of the block.
fn floats_at(block: &[u32; 7], first: usize) -> [f32; 3] {
    [
        f32::from_bits(block[first]),
        f32::from_bits(block[first + 1]),
        f32::from_bits(block[first + 2]),
    ]
}

// IDA: 004CEF2C and its copies: an interior cell takes any reference; an exterior one 004CC540 on vt+0x174's position.
fn placement_inside(world: &World, cell: Entity, reference: Entity) -> Result<bool, GameError> {
    if cell_is_interior(world, cell) {
        return Ok(true);
    }
    let at = reference_position(world, reference)?;
    Ok(cell_contains_position(world, cell, at))
}

/// IDA: 004CEE90 TESObjectCELL_FindPlacementMarker(&position, &rotation). Under the region lock (00496EA0, 00496F50) over +0x48: a node with no next and no item ends the walk; a form flag 0x20 skips the item. Per item, each test skipped once its slot is full: vt+0x170 (004D9B40, +0x1C) is 00B35EAC; is 00B35EB0; inside the cell (vt+0x174, 004CC540) for an exterior. Then, with no door yet: 004D7630's block, 0042B410 its +0x00 door, 004D7630 on that door, 006899C0 its +0x04: an interior or a point inside: 3 dwords to position and 0042B430's +0x10's to rotation, at once, and the byte. Then the first base of form type 0x1C, inside; then any reference inside is `last`. After the release: heading, XMarker; with no door: type 0x1C, last. One chosen copies vt+0x174's 3 dwords and +0x20..+0x28. An exterior cell with a land (004CE3C0, which makes one): height = 0.0, 004C5B50, whose answer is dropped; the water height (+0x24 bit 1: 0041F870; else -FLT_MAX, as 004CACE0); `fcom` water, height: only above takes water; z below that takes it. Completion: 100%; 005F1107's swimming ring stays a stop in `reference_position`.
/// TODO: [ ] 005F1107, when a run reaches it.
/// # Errors
/// A form's null base at 004CF024, a door the load left unresolved, or a stop below.
pub fn find_placement_marker(
    world: &mut World,
    cell: Entity,
    position: &mut [f32; 3],
    rotation: &mut [f32; 3],
) -> Result<PlacementChoice, GameError> {
    let mut choice = PlacementChoice {
        entry: (*position, *rotation),
        ..PlacementChoice::default()
    };
    let statics = world.get_resource::<DefaultFormStatics>();
    let (x_marker, x_marker_heading) = statics.map_or((None, None), |statics| {
        (statics.x_marker_b35eac, statics.x_marker_heading_b35eb0)
    });
    world
        .get_resource_or_init::<CellRegionLocks>()
        .acquire(cell)?;
    let items: Vec<Entity> = world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .items()
        .collect();
    let interior = cell_is_interior(world, cell);
    for item in items {
        if world.get::<FormIdentity>(item).expect("a form").flags_08 & 0x20 != 0 {
            continue;
        }
        let base = world
            .get::<TesObjectRefr>(item)
            .expect("a reference")
            .base_form_1c;
        if choice.x_marker.is_none() && base == x_marker && placement_inside(world, cell, item)? {
            choice.x_marker = Some(item);
        }
        if choice.x_marker_heading.is_none()
            && base == x_marker_heading
            && placement_inside(world, cell, item)?
        {
            choice.x_marker_heading = Some(item);
        }
        if !choice.door && teleport_block(world, item).is_some() {
            let resolved = world
                .get::<TesObjectRefr>(item)
                .expect("a reference")
                .extra_data_44
                .get(EXTRA_TELEPORT)
                .and_then(|entry| entry.resolved_0c);
            if resolved.is_none() {
                return Err(GameError::unclassified(
                    0x004CEF8B,
                    "004CEF8B: a door the load left unresolved",
                ));
            }
            if let Some(door) = teleport_door(world, item)
                && let Some(linked) = teleport_block(world, door)
            {
                let point = floats_at(&linked, 1);
                if interior || cell_contains_position(world, cell, point) {
                    *position = point;
                    *rotation = floats_at(&linked, 4);
                    choice.door = true;
                    choice.door_point = Some((*position, *rotation));
                }
            }
        }
        if choice.type_1c.is_none() {
            let base = base.ok_or(GameError::unclassified(
                0x004CF024,
                "004CF024: a reference with a null base",
            ))?;
            if world
                .get::<FormIdentity>(base)
                .expect("a form")
                .form_type_04
                == 0x1C
                && placement_inside(world, cell, item)?
            {
                choice.type_1c = Some(item);
            }
        }
        if placement_inside(world, cell, item)? {
            choice.last = Some(item);
        }
    }
    world.resource_mut::<CellRegionLocks>().release(cell);
    let mut chosen = choice.x_marker_heading.or(choice.x_marker);
    if chosen.is_none() && !choice.door {
        chosen = choice.type_1c.or(choice.last);
    }
    if let Some(chosen) = chosen {
        choice.chosen = Some(chosen);
        *position = reference_position(world, chosen)?;
        *rotation = world
            .get::<TesObjectRefr>(chosen)
            .expect("a reference")
            .rotation_20;
    }
    if interior {
        return Ok(choice);
    }
    let Some(land) = cell_get_or_create_land(world, cell)? else {
        return Ok(choice);
    };
    let (_, land_height) = land_height_at_position(world, land, *position)?;
    let water = cell_water_height(world, cell);
    choice.lift = Some((land_height, water));
    let height = if water > land_height {
        water
    } else {
        land_height
    };
    if position[2] < height {
        position[2] = height;
    }
    Ok(choice)
}

/// IDA: 004D5D70 TESObjectCELL_PreparePlacementPosition(&position, &rotation). The grid (0, 0 for an interior or no +0x3C): `shl 12`, `add 0x800`, both on the register, wrapping; `fild`, `fstp` float, x then y; `fldz`, stored; rotation = g_niPoint3Zero's dwords. 00B097E0 bDefaultCOCPlacement set, read at this call, on an exterior cell ends there. Otherwise 004D4310, whose answer is dropped, and 004CEE90. Completion: 100%.
/// # Errors
/// A stop below.
pub fn prepare_placement_position(
    world: &mut World,
    cell: Entity,
    position: &mut [f32; 3],
    rotation: &mut [f32; 3],
    services: &mut TesGridServices<'_, '_>,
) -> Result<PlacementChoice, GameError> {
    let center = |grid: i32| grid.wrapping_shl(12).wrapping_add(0x800) as f32;
    *position = [
        center(cell_grid_x(world, cell)),
        center(cell_grid_y(world, cell)),
        0.0,
    ];
    *rotation = NI_POINT3_ZERO.map(f32::from_bits);
    if services.ini_raw(DEFAULT_COC_PLACEMENT)? as u8 != 0 && !cell_is_interior(world, cell) {
        return Ok(PlacementChoice::default());
    }
    load_interior_cell_temporary_data(world, cell, services.load)?;
    find_placement_marker(world, cell, position, rotation)
}

/// IDA: 00440C80 TES_GetCellLoadPriority(cell, position). The cached cell (00B33A64) answers 00B33A60. Otherwise, with h = 00B06A2C >> 1: an interior takes (h, h). An exterior's x is `4C9A80 + (h - X)`, X from `fistp` of position's x `sar 12`, or TES +0x20 with a null position; y likewise, +0x24. dx = x - h, dy = y - h. `cmp dx, dy`, signed: above takes |dx|, otherwise |dy|. In bytes, `(d + 1) * 10`: above 20 (unsigned) gives 40, otherwise 20: the 00440E2C bytes send 20 to 1 and 40 to 3, so 0, 2 and 4 are never produced. The answer and the cell go in the cache. Completion: 100%.
/// # Errors
/// A null g_tes.
pub fn cell_load_priority(
    world: &mut World,
    cell: Entity,
    position: Option<[f32; 3]>,
    grids_to_load: u32,
) -> Result<i32, GameError> {
    let globals = grid_globals(world);
    if globals.load_priority_cell_b33a64 == Some(cell) {
        return Ok(globals.load_priority_b33a60);
    }
    let half = (grids_to_load >> 1) as i32;
    let (mut x, mut y) = (half, half);
    if !cell_is_interior(world, cell) {
        let (from_x, from_y) = match position {
            Some(at) => (x87::fistp(at[0]) >> 12, x87::fistp(at[1]) >> 12),
            None => {
                let data = tes(world)?;
                (
                    data.exterior_coordinates_20[0],
                    data.exterior_coordinates_20[1],
                )
            }
        };
        x = cell_grid_x(world, cell).wrapping_add(half.wrapping_sub(from_x));
        y = cell_grid_y(world, cell).wrapping_add(half.wrapping_sub(from_y));
    }
    let (dx, dy) = (x.wrapping_sub(half), y.wrapping_sub(half));
    let distance = if dx > dy {
        dx.wrapping_abs()
    } else {
        dy.wrapping_abs()
    };
    let scaled = (distance as u8).wrapping_add(1).wrapping_mul(10);
    let priority = if scaled > 20 { 3 } else { 1 };
    let mut globals = grid_globals(world);
    globals.load_priority_b33a60 = priority;
    globals.load_priority_cell_b33a64 = Some(cell);
    Ok(priority)
}

/// IDA: the cell of 00482150's slot, its `[eax]`.
/// # Errors
/// A null g_tes or +0x08, or a slot out of range.
pub fn grid_slot_cell(world: &World, i: u32, j: u32) -> Result<Option<Entity>, GameError> {
    slot_cell(world, i, j)
}

/// IDA: 004402F0 TES_RunGridScripts, as 0040DBDC calls it each idle frame, under 00B33A58 and 004F9FA0 (00B09E25, 1 in every run): each quest with +0x3C bit 0 through 00529AA0 (00440333); then 00440347 the interior's 004CB8C0(cell, 0, 0), or the attached grid cells' at level 6 (004403E4, a stop: no exterior is reached); then the +0x88 list (00440368 each reference's 004D7190) and 004403A5 004FA580. `frame_seconds` is 00B33E9C. Completion: 100% but the exterior grid's walk.
/// TODO: [ ] 004403C0..004403F6, the grid cells' walk at level 6, for an exterior.
/// # Errors
/// A loaded exterior cell, or a stop below.
pub fn run_grid_scripts_frame(world: &mut World, frame_seconds: f32) -> Result<(), GameError> {
    world
        .get_resource_or_init::<ScriptVmGlobals>()
        .frame_seconds_b33e9c = frame_seconds;
    if !scripts_enabled(world) {
        return Ok(());
    }
    let quests: Vec<Entity> = world
        .resource::<DataHandlerLists>()
        .quests_84
        .items()
        .collect();
    for quest in quests {
        let data = world.get::<TesQuest>(quest).expect("a quest");
        if data.flags_3c & 1 == 0 {
            continue;
        }
        // 00529AA0: +0x1C, the scriptable form's script, on +0x58's list.
        let script = world
            .get::<TesScriptableForm>(quest)
            .and_then(|part| match part.script_04 {
                FormRef::Resolved(script) => script,
                FormRef::Raw(_) => None,
            });
        let Some(script) = script else {
            continue;
        };
        let mut events = world
            .get_mut::<TesQuest>(quest)
            .expect("a quest")
            .into_inner()
            .events_58
            .take();
        let result = script_run(world, script, None, events.as_mut(), Some(quest));
        world
            .get_mut::<TesQuest>(quest)
            .expect("a quest")
            .into_inner()
            .events_58 = events;
        result?;
    }
    let tes = world
        .resource::<TesGlobals>()
        .tes
        .as_ref()
        .ok_or(GameError::unclassified(
            0x004402F0,
            "004402F0: a null g_tes",
        ))?;
    if let Some(interior) = tes.current_interior_cell_34 {
        // 00440347: the walk's answer only picks the grid's early exit, which an interior doesn't have.
        crate::tes::forms_init_world::cell_run_reference_scripts(world, interior, false, false)?;
    } else if tes.grid_cells_08.as_ref().is_some_and(|cells| {
        cells.grid_10.iter().any(|slot| {
            slot.cell_00
                .and_then(|cell| world.get::<TesObjectCell>(cell))
                .is_some_and(|cell| cell.process_level_26 == 6)
        })
    }) {
        return Err(GameError::unclassified(
            0x004403E4,
            "004403E4: 004CB8C0 for a grid cell at level 6",
        ));
    }
    // 0044034C..0044039D: the +0x88 list taken, each reference's 004D7190, then freed.
    let queued = std::mem::take(&mut world.resource_mut::<ScriptVmGlobals>().queued_references_88);
    if let Some(tes) = world.resource_mut::<TesGlobals>().tes.as_mut() {
        tes.queued_script_references_88 = 0;
    }
    for reference in queued {
        reference_run_scripts(world, reference)?;
    }
    // 004403A5 004FA580: 004E4690 on each of 00B361CC, then the list is freed.
    let disabled = std::mem::take(&mut world.resource_mut::<ScriptVmGlobals>().disabled_b361cc);
    for reference in disabled {
        crate::tes::script::script_commands::disable_reference(world, reference)?;
    }
    Ok(())
}
