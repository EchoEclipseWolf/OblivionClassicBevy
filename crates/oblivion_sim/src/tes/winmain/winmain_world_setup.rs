//! WinMain 0040EB31..0040EBB8, the block after 0040EB2C returns, one function per stretch, in original order: `run_world_setup_worldspace` 0040EB31..0040EB6D (14a), `run_actor_locations_prelude` 0040EB72..0040EB92 (14b), `run_tracked_actors` 0040EB9A..0040EB9F (14c) and `run_world_setup_finish` 0040EBA4..0040EBB4 (14d). `run_world_setup` runs all four.
//!
//! The callees live in their layers: 004431F0 and 0045B670 in `crate::tes::tes`, the LOD maps in `tes::forms_world`, 00662EE0 in `tes::player`, 0042C970 in `bs::archive`, 00447300 in `tes::forms_init`.
//! Evidence: evidence/phase14-scope-2026-10-09, P14-RUN-001.

use crate::bs::tree_manager::reset_tree_manager;
use crate::tes::actors::actor_process_frame::run_ai_package_pass;
use crate::tes::ai_process::{AiArms, AiRealArms};
use crate::tes::data_handler::{DataHandlerLists, refresh_pc_level_npcs};
use crate::tes::data_load::{DataLoadGlobals, DataLoadServices};
use crate::tes::forms_world::TerrainLodServices;
use crate::tes::player::player_load_idle_animations;
use crate::tes::settings::SettingCollection;
use crate::tes::tes::{GameThreadLocals, TesGlobals, set_current_worldspace};
use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

/// IDA: 00A2FF44 and 00A2FFE8, the WinMain stack NiPoint3 of 0040EB37..0040EB54: {2048, 2048, 128}, as raw bits. Nothing in the window reads it; Phase 15 takes it as its default position.
pub const DEFAULT_PLAYER_START_BITS: [u32; 3] = [0x4500_0000, 0x4500_0000, 0x4300_0000];

/// IDA: N/A what 0040EB31..0040EB6D reads past the world.
pub struct WorldspaceInputs<'a, 'b> {
    /// 00B07BF0: 00B051CC bPreemptivelyUnloadCells, and 0055F750's settings if it had to construct.
    pub ini: &'a SettingCollection,
    /// 00B43077, from 00B02D70 at 0040E92D (`winmain_shader`).
    pub display_lod_land_b43077: u8,
    pub services: &'a mut DataLoadServices<'b>,
    /// 0040FEC0 DebugLog_Level2Printf: 004941A0's level 2 log.
    pub level2_log: &'a mut dyn FnMut(String),
    /// 0057A0D0, which reads the interface. A world with no InterfaceManager answers false at 0057A0DE.
    pub refresh_map_menu: &'a mut dyn FnMut(&mut World) -> Result<bool, GameError>,
}

/// IDA: N/A what 0040EB31..0040EB6D leaves besides the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldspaceOutputs {
    /// The stack point, `DEFAULT_PLAYER_START_BITS`, which Phase 15 reads.
    pub default_position: [f32; 3],
    /// edx at 0040EB62, the +0x0C list's head item.
    pub world_space: Option<Entity>,
    /// 00662EE0's root, by index in `IdleAnimRoots`.
    pub idle_root: Option<usize>,
    /// IDA: N/A how many FindFile calls 004EB1A0 made.
    pub lod_find_file_calls: u32,
}

// IDA: WinMain 0040EB31..0040EB6D, in order: 0040EB31 `push ebp` (0 since 0040DFD4) with 0040EB32 0055F750(0), the stack point, edx = g_dataHandler +0x0C's head item, 0040EB62 004431F0(edx), 0040EB6D 00662EE0. Completion: 100%.
/// # Errors
/// A null g_tes, or a stop in a callee.
pub fn run_world_setup_worldspace(
    world: &mut World,
    inputs: WorldspaceInputs<'_, '_>,
) -> Result<WorldspaceOutputs, GameError> {
    let WorldspaceInputs {
        ini,
        display_lod_land_b43077,
        services,
        level2_log,
        refresh_map_menu,
    } = inputs;
    world.resource_scope(|world, mut globals: Mut<TesGlobals>| {
        reset_tree_manager(world, false, ini, &mut globals.grid_changed_b06a28)
    })?;
    let default_position = DEFAULT_PLAYER_START_BITS.map(f32::from_bits);
    let world_space = world
        .resource::<DataHandlerLists>()
        .world_spaces_0c
        .items()
        .next();
    let disable_warning = world
        .get_resource::<DataLoadGlobals>()
        .is_some_and(|load| load.disable_warning_messages_b06b18 != 0);
    let lod_find_file_calls = {
        let host = &mut *services.host;
        let warning = &mut *services.warning;
        let mut find_file = |path: &str| host.find_file(path, 0, -1);
        let mut warn = |text: String| warning(text, disable_warning);
        let mut lod = TerrainLodServices {
            display_lod_land_b43077,
            find_file: &mut find_file,
            warning: &mut warn,
            debug_log: level2_log,
            find_file_calls: 0,
        };
        set_current_worldspace(world, world_space, ini, &mut lod, refresh_map_menu)?;
        lod.find_file_calls
    };
    let idle_root = player_load_idle_animations(world, services.host)?;
    Ok(WorldspaceOutputs {
        default_position,
        world_space,
        idle_root,
        lod_find_file_calls,
    })
}

// IDA: WinMain 0040EB72..0040EB92, in order: 0040EB72 0042C970 (a null g_ArchiveList returns at 0042C97B); 0040EB86 pushes the log's text and 0040EB8B stores TLS +0x185 = 1 before the 0040EB92 call. From the store on, vt+0x40 records. Completion: 100%.
pub fn run_actor_locations_prelude(
    world: &mut World,
    services: &mut DataLoadServices<'_>,
    log: &mut dyn FnMut(&'static str),
) {
    services.host.apply_archive_cache_settings();
    world.resource_mut::<GameThreadLocals>().track_changes_185 = 1;
    log("Initializing Actor Locations...");
}

// IDA: WinMain 0040EB9A..0040EB9F: ecx = g_actorProcessManager, 00675310 ActorProcessManager_ProcessTrackedActors. A driver gives `AiRealArms`; a test may replay a run's draws through `AiRecordedArms`. Completion: 100% of the call site.
/// # Errors
/// A stop in the pass.
pub fn run_tracked_actors(world: &mut World, arms: &mut dyn AiArms) -> Result<(), GameError> {
    run_ai_package_pass(world, arms)
}

// IDA: WinMain 0040EBA4..0040EBB4, in order: 0040EBAA 00447300 on g_dataHandler, then 0040EBB4 Startup_LogInfo("Loading initial area..."). Answers the NPCs visited. Completion: 100%.
/// # Errors
/// A stop in 005222D0.
pub fn run_world_setup_finish(
    world: &mut World,
    log: &mut dyn FnMut(&'static str),
) -> Result<Vec<Entity>, GameError> {
    let visited = refresh_pc_level_npcs(world)?;
    log("Loading initial area...");
    Ok(visited)
}

/// IDA: N/A what `run_world_setup` reads past `WorldspaceInputs`.
pub struct WorldSetupSeams<'a> {
    /// 00404EE0 Startup_LogInfo (host logging; D4).
    pub log: &'a mut dyn FnMut(&'static str),
    /// 00675310's CRT and menu mode.
    pub arms: AiRealArms<'a>,
}

/// IDA: N/A what the whole block leaves.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldSetupOutputs {
    pub worldspace: WorldspaceOutputs,
    /// The NPCs 00447300 ran 005222D0 on.
    pub autocalc_npcs: Vec<Entity>,
}

// IDA: WinMain 0040EB31..0040EBB8 as one sequence. 0040EBB9 starts Phase 15. Completion: 100% of the order.
/// # Errors
/// A stop in a callee.
pub fn run_world_setup(
    world: &mut World,
    inputs: WorldspaceInputs<'_, '_>,
    seams: WorldSetupSeams<'_>,
) -> Result<WorldSetupOutputs, GameError> {
    let WorldSetupSeams { log, mut arms } = seams;
    let WorldspaceInputs {
        ini,
        display_lod_land_b43077,
        services,
        level2_log,
        refresh_map_menu,
    } = inputs;
    let worldspace = run_world_setup_worldspace(
        world,
        WorldspaceInputs {
            ini,
            display_lod_land_b43077,
            services,
            level2_log,
            refresh_map_menu,
        },
    )?;
    run_actor_locations_prelude(world, services, log);
    run_tracked_actors(world, &mut arms)?;
    let autocalc_npcs = run_world_setup_finish(world, log)?;
    Ok(WorldSetupOutputs {
        worldspace,
        autocalc_npcs,
    })
}
