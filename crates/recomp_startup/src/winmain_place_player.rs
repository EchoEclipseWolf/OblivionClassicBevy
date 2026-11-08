//! Phase 16: the WinMain block that places the player, 0040EE7A..0040EF23. 0040EF24 starts Phase 17, and the no-cell jump at 0040EF08 lands at 0040EF54, past Phase 17's first-person refresh too. `rect` is the stack NiPoint3 Phase 15 left; `rotation` the one at the MSG's wParam, lParam, time. The callees that aren't the player's own are a trait, `PlacePlayerArms`, so that a test's recorder can stand in for each; `RealPlacePlayerArms` runs them.
//!
//! Evidence: objdump 0040EE7A..0040EF24; P16-RUN-001, 002, 003.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;
use recomp_game::bs::model_loader::{IoDrainResult, IoDrainServices, io_drain_tasks_through_stage};
use recomp_game::tes::actor::{NI_POINT3_ZERO, actor_set_z_rotation_if_process_allows};
use recomp_game::tes::ai_cell_moves::insert_reference_runtime;
use recomp_game::tes::ai_conditions::the_player;
use recomp_game::tes::character_controller::actor_set_position;
use recomp_game::tes::form::FormIdentity;
use recomp_game::tes::settings::SettingCollection;
use recomp_game::tes::tes::TesGlobals;
use recomp_game::tes::tes_grid::{
    PlacementChoice, TesGridGlobals, TesGridServices, grid_slot_cell, prepare_placement_position,
};

/// IDA: 00B06A2C uGridsToLoad, read where the block reads it.
const GRIDS_TO_LOAD: u32 = 0x00B0_6A2C;
/// IDA: 00B14F58 bBorderRegionsEnabled, which 00667600 reads.
const BORDER_REGIONS_ENABLED: u32 = 0x00B1_4F58;

/// IDA: N/A what the block calls past the player's own virtuals, by address.
pub trait PlacePlayerArms {
    /// IDA: 00404EE0 Startup_LogInfo (D4: the host log).
    fn log(&mut self, text: &str);
    /// IDA: 004D5D70(cell, &Rect, &rotation).
    /// # Errors
    /// A stop in the arm.
    fn prepare_placement_position(
        &mut self,
        world: &mut World,
        cell: Entity,
        rect: &mut [f32; 3],
        rotation: &mut [f32; 3],
    ) -> Result<PlacementChoice, StreamError>;
    /// IDA: 004D35D0(cell, g_thePlayer).
    /// # Errors
    /// A stop in the arm.
    fn insert_player(
        &mut self,
        world: &mut World,
        cell: Entity,
        player: Entity,
    ) -> Result<(), StreamError>;
    /// IDA: 00434020(g_ioManager, 5).
    /// # Errors
    /// A stop in the arm.
    fn drain_tasks_through_stage(
        &mut self,
        world: &mut World,
        stage: i32,
    ) -> Result<IoDrainResult, StreamError>;
}

/// IDA: N/A the real arms, over the grid's services and the drain's.
pub struct RealPlacePlayerArms<'a, 'b, 'c, 'd> {
    pub grid: &'a mut TesGridServices<'b, 'c>,
    /// 00433590: `ModelDrain` over the driver's build services.
    pub run_timed_tasks: &'a mut (dyn FnMut(&mut World) -> Result<(), StreamError> + 'd),
    /// Sleep.
    pub sleep: &'a mut (dyn FnMut(u32) + 'd),
    /// 00404EE0.
    pub info: &'a mut (dyn FnMut(&str) + 'd),
}

impl PlacePlayerArms for RealPlacePlayerArms<'_, '_, '_, '_> {
    /// IDA: N/A `RealPlacePlayerArms`'s side of `PlacePlayerArms`.
    fn log(&mut self, text: &str) {
        (self.info)(text);
    }

    /// IDA: N/A `RealPlacePlayerArms`'s side of `PlacePlayerArms`.
    fn prepare_placement_position(
        &mut self,
        world: &mut World,
        cell: Entity,
        rect: &mut [f32; 3],
        rotation: &mut [f32; 3],
    ) -> Result<PlacementChoice, StreamError> {
        prepare_placement_position(world, cell, rect, rotation, self.grid)
    }

    /// IDA: N/A `RealPlacePlayerArms`'s side of `PlacePlayerArms`.
    fn insert_player(
        &mut self,
        world: &mut World,
        cell: Entity,
        player: Entity,
    ) -> Result<(), StreamError> {
        insert_reference_runtime(world, cell, player)
    }

    /// IDA: N/A `RealPlacePlayerArms`'s side of `PlacePlayerArms`.
    fn drain_tasks_through_stage(
        &mut self,
        world: &mut World,
        stage: i32,
    ) -> Result<IoDrainResult, StreamError> {
        let host = &mut *self.grid.load.host;
        let mut section =
            |_: &mut World, section: i32, percent: f32| host.section_progress(section, percent);
        let mut saving = 0_u32;
        let mut save_load = |_: i32, _: f32| saving += 1;
        let result = io_drain_tasks_through_stage(
            world,
            stage,
            &mut IoDrainServices {
                run_timed_tasks: &mut *self.run_timed_tasks,
                section_progress: &mut section,
                save_load_progress: &mut save_load,
                sleep: &mut *self.sleep,
            },
        )?;
        for _ in 0..saving {
            world
                .get_resource_or_init::<recomp_game::tes::tes_grid::TesGridBoundaries>()
                .count(0x0045_23A0);
        }
        Ok(result)
    }
}

/// IDA: N/A what the block leaves besides the world and Rect.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlacePlayerResult {
    /// esi at 0040EF06.
    pub cell: Option<Entity>,
    /// 0040EEB1 ran: no interior cell.
    pub from_centre_slot: bool,
    /// The stack rotation after 004D5D70.
    pub rotation: [f32; 3],
    /// 004D5D70's choice.
    pub placement: Option<PlacementChoice>,
    /// 00434020's record.
    pub drain: Option<IoDrainResult>,
    /// 0040EF08's jump: the next instruction is 0040EF54.
    pub reached_0040ef54: bool,
}

/// IDA: WinMain 0040EE7A..0040EF23: the log; esi = g_tes +0x34; rotation = g_niPoint3Zero's dwords, stored before the test. No cell: 00482150(g_tes +0x08, n >> 1, n >> 1), `[eax]`, with n = uGridsToLoad, read here; still none skips 004D5D70. `fld`, `fstp` rotation.z, vt+0x1E8 005E0EA0 on g_thePlayer; vt+0x1CC 0065A3E0(&Rect). No cell jumps to 0040EF54. Otherwise 004D35D0(cell, g_thePlayer), 00434020(g_ioManager, 5). Completion: 100% of the block.
/// # Errors
/// A null g_tes or g_thePlayer, a missing setting, or a stop in a callee.
pub fn run_place_player(
    world: &mut World,
    ini: &SettingCollection,
    arms: &mut dyn PlacePlayerArms,
    rect: &mut [f32; 3],
) -> Result<PlacePlayerResult, StreamError> {
    let mut result = PlacePlayerResult::default();
    arms.log("Placing player...");
    let tes = world
        .resource::<TesGlobals>()
        .tes
        .as_deref()
        .ok_or(StreamError("0040EE84: a null g_tes"))?;
    let mut cell = tes.current_interior_cell_34;
    let mut rotation = NI_POINT3_ZERO.map(f32::from_bits);
    let grids = ini
        .find_address(GRIDS_TO_LOAD)
        .ok_or(StreamError("0040EEB1: uGridsToLoad isn't registered"))?
        .raw;
    let border_regions = ini
        .find_address(BORDER_REGIONS_ENABLED)
        .ok_or(StreamError(
            "00667A00: bBorderRegionsEnabled isn't registered",
        ))?
        .raw as u8;
    {
        let mut globals = world.get_resource_or_init::<TesGridGlobals>();
        globals.grids_to_load_b06a2c = Some(grids);
        globals.border_regions_b14f58 = Some(border_regions);
    }
    if cell.is_none() {
        result.from_centre_slot = true;
        cell = grid_slot_cell(world, grids >> 1, grids >> 1)?;
    }
    if let Some(cell) = cell {
        result.placement =
            Some(arms.prepare_placement_position(world, cell, rect, &mut rotation)?);
    }
    let player = the_player(world).ok_or(StreamError("0040EED9: a null g_thePlayer"))?;
    result.cell = cell;
    result.rotation = rotation;
    actor_set_z_rotation_if_process_allows(
        world,
        player,
        x87::fld_fstp_copy(rotation[2].to_bits()),
    )?;
    actor_set_position(world, player, *rect)?;
    let Some(cell) = cell else {
        result.reached_0040ef54 = true;
        return Ok(result);
    };
    arms.insert_player(world, cell, player)?;
    result.drain = Some(arms.drain_tasks_through_stage(world, 5)?);
    Ok(result)
}

/// IDA: N/A main.cpp's line for the block, as `fixtures/cpp-baseline` holds it. No original run prints it.
#[must_use]
pub fn phase_16_line(world: &World, result: &PlacePlayerResult, rect: &[f32; 3]) -> String {
    let cell_id = result.cell.map_or(0, |cell| {
        world.get::<FormIdentity>(cell).expect("a form").form_id_0c
    });
    format!(
        "Phase-16: cell {cell_id}, Rect {{{}, {}, {}}}, rotation z {}, {}",
        rect[0],
        rect[1],
        rect[2],
        result.rotation[2],
        if result.reached_0040ef54 {
            "0040EF54"
        } else {
            "0040EF24"
        }
    )
}
