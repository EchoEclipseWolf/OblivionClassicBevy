//! `00512540`'s player arm and `0066EAF0` PlayerCharacter_MoveToCell, as a script's MoveTo reaches them: a new game's command 64, into the Imperial Prison (`1FBB9`, an interior). The cell load needs the driver's load services, which `CellLoadHost` lends.
//!
//! Evidence: `evidence/gameplay-run002-2026-11-07/` (run-x's snapshots after command 64, run-w's coverage up to the IO drain).

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::actor::{MobileObject, TesObjectRefr};
use super::character_controller::actor_set_position;
use super::move_to::release_followers;
use super::player::PlayerCharacter;
use crate::gamebryo::transform::{rotation_x, rotation_z};
use crate::tes::actor::actor_is_dead;
use crate::tes::forms_references::parent_cell;
use crate::tes::forms_references::reference_position;
use crate::tes::forms_references::reference_world;
use crate::tes::forms_world::cell_is_interior;
use crate::tes::forms_world::cell_world;
use crate::tes::movie_player::play_intro_movie;
use crate::tes::tes::tes_update;
use crate::tes::tes_grid::TesGridServices;

/// IDA: N/A what the move reaches that only the host holds: the grid's services, whose `drain` is 00434020 with the host's timed pass.
pub struct CellLoadServices<'g, 'a, 'b> {
    pub grid: &'g mut TesGridServices<'a, 'b>,
}

/// IDA: N/A a body run with `CellLoadServices`.
pub type CellLoadBody<'f> =
    dyn FnMut(&mut World, &mut CellLoadServices<'_, '_, '_>) -> Result<(), GameError> + 'f;

/// IDA: N/A `CellLoadHost`'s call: the body to run with the services.
pub type CellLoadCall = fn(&mut World, &mut CellLoadBody<'_>) -> Result<(), GameError>;

/// IDA: N/A the grid loan supplied at the first scripts frame. Each call builds its own services.
#[derive(Resource, Clone, Copy)]
pub struct CellLoadHost(pub CellLoadCall);

/// IDA: 00B34420, which 004835D0 sets from the old world's +0x7C.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PlayerMoveGlobals {
    pub parent_world_b34420: u8,
    /// IDA: N/A 0066EAF0's local at [esp+0x12] (var_B4+2): a track loaded while a movie ran, resumed after it.
    pub music_pending: bool,
}

// IDA: 004BE5A0 and 004BD980 on 00B35B90 and 00B35B8C: each cancels its map's queued loader tasks (00642D90's walk, 00432130). An empty map does nothing; a held task is a stop.
fn cancel_loader_tasks(world: &World) -> Result<(), GameError> {
    let Some(tasks) = world.get_resource::<crate::bs::task_manager::TaskGlobals>() else {
        return Ok(());
    };
    for map in [
        &tasks.exterior_cell_tasks_b35b90,
        &tasks.distant_lod_tasks_b35b8c,
    ]
    .into_iter()
    .flatten()
    {
        if map.buckets_0c.iter().any(|&bucket| bucket != 0) {
            return Err(GameError::unclassified(
                0x004BE48C,
                "004BE48C: cancelling a queued cell or LOD loader task",
            ));
        }
    }
    Ok(())
}

/// IDA: 00512540(player, target, x, y, z): vt+0x198(0) (Actor_IsDead) returns; 00675D50 (`release_followers`); the target's parent cell and exterior world (004D6670) and its position (vt+0x174) plus the offsets, double-precision sums stored as floats; 004BE5A0 and 004BD980 on the two loader maps. A target with no cell is the exterior lookup or creation (0051263A), a stop. ConsoleManager_GetSingleton(1) makes the console's block, which the port doesn't hold, and its +0x31 is clear, so no toggle. Then 0066EAF0(position, rotation (0, 0, the target's Z), cell, 1), vt+0x178(0) (0065B5C0), +0x117 = 1 and 00665260. Completion: 100% of the arms an interior target reaches.
/// TODO: [ ] The console block.
/// # Errors
/// A stop above or in 0066EAF0.
pub fn move_player_to(
    world: &mut World,
    player: Entity,
    target: Entity,
    offset: [f32; 3],
) -> Result<(), GameError> {
    if actor_is_dead(world, player, false) {
        return Ok(());
    }
    release_followers(world, player)?;
    let cell = parent_cell(world, target);
    let target_rotation = world
        .get::<TesObjectRefr>(target)
        .expect("a reference")
        .rotation_20;
    let base = reference_position(world, target)?;
    let position: [f32; 3] =
        std::array::from_fn(|at| x87::stored_float(f64::from(base[at]) + f64::from(offset[at])));
    let _space = reference_world(world, target);
    cancel_loader_tasks(world)?;
    let Some(cell) = cell else {
        return Err(GameError::unclassified(
            0x0051263A,
            "0051263A: a MoveTo the player into an exterior",
        ));
    };
    let host = world
        .get_resource::<CellLoadHost>()
        .copied()
        .ok_or(GameError::unclassified(
            0x0066EAF0,
            "0066EAF0: a cell load with no host services",
        ))?;
    (host.0)(world, &mut |world, services| {
        player_move_to_cell(
            world,
            player,
            position,
            [0.0, 0.0, target_rotation[2]],
            cell,
            true,
            services,
        )
    })?;
    // 00512722 vt+0x178(0): 0065B5C0, the controller's world against the cell's (unchanged after 0066EAF0's Load3D put it
    // in, so 0088CDC0 marks the third person's collision).
    super::character_controller::actor_sync_char_controller_world(world, player)?;
    move_player_to_tail(world, player)
}

/// IDA: 0066EAF0 PlayerCharacter_MoveToCell(position, rotation, cell, reset_sky), its interior arm. The old world (004D6670) and the cell's (an interior's is null); 004835D0: 00B34420 = old world's +0x7C set. MoviePlayer_IsRunning. The parent cell's 004CECD0. vt+0x1CC 0065A3E0 (`actor_set_position`); 004D89A0 the rotation and vt+0x40(4); a horse (vt+0x380) is a stop; 004D8AA0 the parent cell, vt+0x40(4) when it changes, then the process's vt+0x500. An interior with OSGlobals +0x24: `cell_music_type` and the host's 006AB160; a load while a movie runs is kept for after it (`music_pending`), else ResumeMusic and StartMusicPlayback. Then 004455E0, 00560110, 0066ED16's sounds (counted), the equipped light, 00540380 with `reset_sky`, 004D8AA0(null), 004D35D0 and, with no node, 00434020(5); the camera block (D16), 0055FA50, 004424D0, 0066F260's intro block (`intro_block`) and the tail (`move_tail`). Completion: 100% of the interior arm the new game's move takes, with D16 and D21.
/// # Errors
/// A stop above, or in a callee.
pub fn player_move_to_cell(
    world: &mut World,
    player: Entity,
    position: [f32; 3],
    rotation: [f32; 3],
    cell: Entity,
    reset_sky: bool,
    services: &mut CellLoadServices<'_, '_, '_>,
) -> Result<(), GameError> {
    let old_space = reference_world(world, player);
    let _new_space = cell_world(world, Some(cell));
    // 0066EB23 MoviePlayer_IsRunning, read before anything moves.
    let movie_running = world
        .get_resource::<crate::tes::movie_player::MoviePlayer>()
        .is_some_and(crate::tes::movie_player::movie_player_is_running);
    // 004835D0.
    if let Some(space) = old_space {
        let held = world
            .get::<crate::tes::forms_world::TesWorldSpace>(space)
            .is_some_and(|data| {
                !matches!(
                    data.parent_7c,
                    crate::tes::form_components::FormRef::Raw(0)
                        | crate::tes::form_components::FormRef::Resolved(None)
                )
            });
        world
            .get_resource_or_init::<PlayerMoveGlobals>()
            .parent_world_b34420 = u8::from(held);
    }
    if let Some(old) = parent_cell(world, player) {
        crate::tes::forms_world::cell_remove_reference(world, old, player)?;
    }
    actor_set_position(world, player, position)?;
    world
        .get_mut::<TesObjectRefr>(player)
        .expect("the player")
        .rotation_20 = rotation;
    crate::tes::forms_items::set_change_flags(world, player, 4)?;
    if world
        .get::<PlayerCharacter>(player)
        .is_some_and(|data| data.last_ridden_horse_1e0 != 0)
    {
        return Err(GameError::unclassified(
            0x0066EB94,
            "0066EB94: the player's horse",
        ));
    }
    // 004D8AA0.
    if parent_cell(world, player) != Some(cell) {
        crate::tes::forms_items::set_change_flags(world, player, 4)?;
    }
    world
        .get_mut::<TesObjectRefr>(player)
        .expect("the player")
        .parent_cell_40 = Some(cell);
    // 004D8ADD vt+0x500, MiddleHighProcess 00651EF0: its +0x170 list's references move with the actor; null returns.
    if let Some(process) = world
        .get::<MobileObject>(player)
        .and_then(|mobile| mobile.process_58)
        && world
            .get::<super::actor_process::MiddleHighProcess>(process)
            .is_some_and(|middle| middle.field_170 != 0)
    {
        return Err(GameError::unclassified(
            0x00651F10,
            "00651F10: the player's linked references",
        ));
    }
    if !cell_is_interior(world, cell) {
        return Err(GameError::unclassified(
            0x0066EC35,
            "0066EC35: an exterior cell",
        ));
    }
    let mut music_pending = false;
    if world
        .get_resource::<crate::tes::os_globals::OsGlobalsData>()
        .is_some_and(|os| os.sound_24.is_some())
    {
        let music_type = crate::tes::forms_world::cell_resolve_music_type(world, cell)?;
        let host = world
            .get_resource::<crate::tes::os_sound_music::ScriptMusic>()
            .cloned()
            .ok_or(GameError::unclassified(
                0x0066EBF2,
                "0066EBF2: the cell's music with no host",
            ))?;
        let loaded = host.cell_music(world, music_type, !movie_running, !movie_running);
        music_pending = loaded && movie_running;
    }
    world
        .get_resource_or_init::<PlayerMoveGlobals>()
        .music_pending = music_pending;
    let mut rect = position;
    crate::tes::tes_grid::enter_interior_cell(world, Some(cell), &mut rect, services.grid)?;
    reset_tree_canopy(world)?;
    // 0066ED16: with OSGlobals +0x24, 006AC210 empties the game sounds' +0x304 map and 006AC3C0 stops those with flag 0x1000; the port plays no game sounds, so both are counted.
    if world
        .get_resource::<crate::tes::os_globals::OsGlobalsData>()
        .is_some_and(|os| os.sound_24.is_some())
    {
        count(world, 0x006A_C210);
        count(world, 0x006A_C3C0);
    }
    // 0066ED2D the process's vt+0xF0(1), 0064B270: the equipped light at +0xE8, held by 00484E80; with one, vt+0x348.
    if let Some(process) = world
        .get::<MobileObject>(player)
        .and_then(|mobile| mobile.process_58)
        && world
            .get::<super::actor_process::MiddleHighProcess>(process)
            .is_some_and(|middle| middle.light_e8.is_some())
    {
        return Err(GameError::unclassified(
            0x0066ED40,
            "0066ED40: the player's equipped light (vt+0x348)",
        ));
    }
    if reset_sky {
        sky_reset_weather(world);
    }
    // 0066ED63 004D8AA0(null): no flag for a null cell, +0x40 = null, then the process's vt+0x500 as above.
    world
        .get_mut::<TesObjectRefr>(player)
        .expect("the player")
        .parent_cell_40 = None;
    // 004D35D0 with the grid's archives: the player's 00438060 (QueuedPlayer, priority 00440C80's).
    {
        let mut load = crate::tes::reference_load::ReferenceLoadServices {
            archives: services.grid.archives.as_deref_mut(),
        };
        crate::tes::ai_cell_moves::insert_reference_queued(world, cell, player, Some(&mut load))?;
    }
    // 0066ED70 vt+0x154: with no node and no save active, the IO manager drains through stage 5.
    let node = world
        .get::<TesObjectRefr>(player)
        .expect("the player")
        .loaded_3d_3c;
    if node == 0 && !crate::tes::forms_references::save_load_active(world)? {
        let drain = services.grid.drain.as_mut().ok_or(GameError::unclassified(
            0x0066ED70,
            "0066ED70: a cell load with no drain",
        ))?;
        drain(world, 5)?;
    }
    place_camera(world, player, position, rotation)?;
    place_listener(world, player, position);
    crate::tes::player::stop_on_active_quest(world)?;
    // 0066F02E the horse's move (vt+0x380), none here; 0066F07F 007AA4D0 on the BSShaderAccumulator, the renderer's, counted.
    count(world, 0x007A_A4D0);
    if world
        .get::<PlayerCharacter>(player)
        .is_some_and(|data| data.first_person_node_5d0.is_some())
    {
        camera_block(world, player)?;
    }
    // 0066F243 0055FA50(camera, 0.0): SpeedTree's wind time 00B39F18, camera, fog into the tree manager and canopy scale, the renderer's state, counted.
    count(world, 0x0055_FA50);
    let frame = world
        .get_resource::<crate::tes::script_vm::ScriptVmGlobals>()
        .map_or(0.0, |globals| globals.frame_seconds_b33e9c);
    tes_update(world, frame)?;
    intro_block(world, player, movie_running, music_pending, services)?;
    move_tail(world, services)
}

/// IDA: N/A the main thread's CRT after each of 0066F260's three drawing calls, with a host `CrtProbe`: the loading menu's
/// close (00440AF0), the race menu (0057B7E0) and the movie's resume (00410BA0).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IntroBlockCrt {
    pub after_close: Option<(u32, u64)>,
    pub after_race_menu: Option<(u32, u64)>,
    pub after_movie: Option<(u32, u64)>,
}

/// IDA: N/A host design item D21: 00410BA0 MoviePlayer_PlayMovie's requests from 0066F2D1, which the original plays to the
/// end on the main thread. The port records each and runs the movie's sound pause and its two resumes in place
/// (`play_intro_movie`); the host plays the file. Each is the name and (pollSkipKeys, quiet, loop, preload, scaleToWidth,
/// keepCurrent).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct IntroMovieRequests {
    pub requests: Vec<(Vec<u8>, [bool; 6])>,
}

/// IDA: the INI setting sOblivionIntro:General (00B030A4; the image's "OblivionIntro.bik").
const INTRO_MOVIE_SETTING: u32 = 0x00B0_30A4;

// IDA: N/A the intro music's host call.
fn intro_music(world: &mut World, pause: bool) -> Result<(), GameError> {
    let host = world
        .get_resource::<crate::tes::os_sound_music::ScriptMusic>()
        .cloned()
        .ok_or(GameError::unclassified(
            0x0066F2B2,
            "0066F2B2: the intro's music with no host",
        ))?;
    host.intro_music(world, pause);
    Ok(())
}

/// IDA: 0066F260..0066F2EE, 0066EAF0's intro block with +0x7F8 set (the new game's move): 005732D0(2, 0.3) the black fader;
/// 00440AF0(0, 0, 0) the loading menu's close (10 draws); 0057B7E0 the race menu (11 draws); +0x7F8 = 0; 00572EC0(2, 0);
/// on g_osGlobals' sound 006A8D00 PauseMusic and 006A9B40 PauseGameSounds; 00410BA0 the intro movie (D21: its resume draws
/// 11); 006A8D50 ResumeMusic and 006A9C00 ResumeGameSounds; +0x7F8 = 0. Then 0066F2EE: a movie running at the move's start
/// waits for its thread (MoviePlayer_WaitForThread) and resumes a track loaded meanwhile. Completion: 100%.
/// # Errors
/// No interface, sound or host for these calls, or a stop in them.
fn intro_block(
    world: &mut World,
    player: Entity,
    movie_running: bool,
    music_pending: bool,
    services: &mut CellLoadServices<'_, '_, '_>,
) -> Result<(), GameError> {
    let intro = world
        .get::<PlayerCharacter>(player)
        .expect("the player")
        .new_game_intro_pending_7f8
        != 0;
    if intro {
        let sound_enabled = services
            .grid
            .game_sounds
            .ok_or(GameError::unclassified(
                0x0066F2B9,
                "0066F2B9: the intro's sounds with no settings",
            ))?
            .sound_enabled_b16178;
        let name = services
            .grid
            .ini
            .find_address(INTRO_MOVIE_SETTING)
            .and_then(|setting| setting.text.clone())
            .unwrap_or_default();
        let interface = services
            .grid
            .interface
            .as_mut()
            .ok_or(GameError::unclassified(
                0x0066F27F,
                "0066F27F: the intro block with no interface",
            ))?;
        interface(world, &mut |cx| {
            crate::tes::faders::create_fader(cx, 2, 0.3)?;
            crate::tes::tes::tes_close_loading_menu(cx)?;
            let crt = crate::tes::form_arms::crt_probe(cx.world);
            cx.world.get_resource_or_init::<IntroBlockCrt>().after_close = crt;
            crate::tes::interface_manager::interface_open_new_game_race_menu(cx)?;
            let crt = crate::tes::form_arms::crt_probe(cx.world);
            cx.world
                .get_resource_or_init::<IntroBlockCrt>()
                .after_race_menu = crt;
            Ok(())
        })?;
        world
            .get_mut::<PlayerCharacter>(player)
            .expect("the player")
            .new_game_intro_pending_7f8 = 0;
        interface(world, &mut |cx| crate::tes::faders::end_fader(cx, 2, false))?;
        if world
            .get_resource::<crate::tes::os_globals::OsGlobalsData>()
            .is_none_or(|os| os.sound_24.is_none())
        {
            return Err(GameError::unclassified(
                0x0066F2B2,
                "0066F2B2: a null sound object faults",
            ));
        }
        intro_music(world, true)?;
        crate::tes::game_sounds::pause_game_sounds(world, sound_enabled);
        play_intro_movie(world, &name, sound_enabled)?;
        let crt = crate::tes::form_arms::crt_probe(world);
        world.get_resource_or_init::<IntroBlockCrt>().after_movie = crt;
        intro_music(world, false)?;
        let mut draw = crate::tes::game_sounds::host_draw(world)?;
        crate::tes::game_sounds::resume_game_sounds(world, sound_enabled, &mut draw);
        world
            .get_mut::<PlayerCharacter>(player)
            .expect("the player")
            .new_game_intro_pending_7f8 = 0;
    }
    if movie_running {
        if let Some(movie) = world.get_resource::<crate::tes::movie_player::MoviePlayer>() {
            crate::tes::movie_player::movie_player_wait_for_thread(movie);
        }
        if music_pending {
            intro_music(world, false)?;
        }
    }
    Ok(())
}

/// IDA: 0066F308..0066F35D: +0x764 held: 006B73E0 on it and FormHeap_Free, +0x764 = +0x760 = 0 (a stop: the move's is 0);
/// g_osGlobals +0x18 = 0; with no interior (TES +0x34) and a water surface manager (+0x54) 00498F40, a stop; 00444A10
/// TES_InitializeParticleNodeCache (`tes_initialize_particle_node_cache`: the 16 effect models through
/// ModelLoader_LoadModelNow and 00443F00's five clones each), with the grid's model services. Completion: 100%.
/// # Errors
/// The two stops, no model services, or a stop in 00444A10.
fn move_tail(
    world: &mut World,
    services: &mut CellLoadServices<'_, '_, '_>,
) -> Result<(), GameError> {
    let player = crate::tes::player::the_player(world).ok_or(GameError::unclassified(
        0x0066F308,
        "0066F308: a null player",
    ))?;
    if world
        .get::<PlayerCharacter>(player)
        .expect("the player")
        .cached_sound_handle_764
        != 0
    {
        return Err(GameError::unclassified(
            0x0066F316,
            "0066F316: 006B73E0 on the player's +0x764",
        ));
    }
    if let Some(mut os) = world.get_resource_mut::<crate::tes::os_globals::OsGlobalsData>() {
        os.unk_18 = 0;
    }
    let (interior, water) = {
        let tes = crate::tes::tes_grid::tes(world)?;
        (tes.current_interior_cell_34, tes.water_surface_54.is_some())
    };
    if interior.is_none() && water {
        return Err(GameError::unclassified(
            0x0066F34D,
            "0066F34D: 00498F40 on the water surface manager",
        ));
    }
    let models = services
        .grid
        .models
        .as_mut()
        .ok_or(GameError::unclassified(
            0x0066F358,
            "0066F358: 00444A10 with no model services",
        ))?;
    models(world, &mut |world, models| {
        crate::tes::tes::tes_initialize_particle_node_cache(world, models)
    })
}

/// IDA: 00512731..00512749, 00512540's tail after vt+0x178: g_thePlayer's +0x117 = 1, then
/// PlayerCharacter_UpdateActiveQuestTargetsForMovedRef(player) (00665260; no active quest: nothing). Completion: 100%; it runs
/// after 00512722's vt+0x178 (0065B5C0, Build 3 goal 1).
/// # Errors
/// 00665260's active-quest arm.
pub fn move_player_to_tail(world: &mut World, mover: Entity) -> Result<(), GameError> {
    let player = crate::tes::player::the_player(world).ok_or(GameError::unclassified(
        0x00512731,
        "00512731: a null g_thePlayer faults",
    ))?;
    let _ = mover;
    world
        .get_mut::<PlayerCharacter>(player)
        .expect("the player")
        .unset_active_quest_targets_dirty_117 = 1;
    crate::tes::player::stop_on_active_quest(world)
}

/// IDA: 0066F09C..0066F234, 0066EAF0's camera block with a first-person node (+0x5D0). With no save loading
/// (TESSaveLoadGame_TestCurrentThreadActiveFlag): the start position (vt+0x174); an exterior's land test (004D1E10) is a
/// stop; up to 100 `0065AF30`(0.16, zero, 0) steps, each ending when z moved less than 0.1 (the difference a float, its
/// fabs a float); all 100 put the start back (vt+0x1CC). Then +0x588 flipped around each of two `00603CA0`(0.0), and
/// `0066B710`(0). Completion: 100% of the block; the callees' counted calls are theirs.
/// # Errors
/// A stop in a callee.
fn camera_block(world: &mut World, player: Entity) -> Result<(), GameError> {
    let probe = crate::tes::form_arms::crt_probe;
    world.get_resource_or_init::<CameraBlockSteps>().crt_before = probe(world);
    if !crate::tes::forms_references::save_load_active(world)? {
        let start = world
            .get::<TesObjectRefr>(player)
            .expect("the player")
            .position_2c;
        if parent_cell(world, player).is_some_and(|cell| !cell_is_interior(world, cell)) {
            return Err(GameError::unclassified(
                0x0066F0E0,
                "0066F0E0: an exterior's land height (004D1E10)",
            ));
        }
        let mut steps = 0;
        while steps < 100 {
            let before = world
                .get::<TesObjectRefr>(player)
                .expect("the player")
                .position_2c[2];
            super::mobile_move::mobile_object_move(world, player, 0.16, [0.0; 3], 0)?;
            let after = world
                .get::<TesObjectRefr>(player)
                .expect("the player")
                .position_2c[2];
            let moved = x87::stored_float(f64::from(after) - f64::from(before)).abs();
            world
                .get_resource_or_init::<CameraBlockSteps>()
                .z
                .push(after);
            if moved < 0.1 {
                break;
            }
            steps += 1;
        }
        if steps == 100 {
            actor_set_position(world, player, start)?;
        }
    }
    for _ in 0..2 {
        {
            let mut data = world
                .get_mut::<PlayerCharacter>(player)
                .expect("the player");
            data.view_flags_588[0] = u8::from(data.view_flags_588[0] == 0);
        }
        super::actor_movement_update::player_update_movement_and_animation(world, player, 0.0)?;
    }
    player_update_camera(world, player, false)?;
    let camera = world
        .get_resource::<crate::bs::scene_graphs::SceneGraphGlobals>()
        .and_then(|globals| globals.world)
        .and_then(|graph| world.get::<crate::bs::scene_graphs::SceneGraph>(graph))
        .and_then(|scene| scene.camera_dc)
        .and_then(|camera| world.get::<crate::gamebryo::scene_node::NiAvWorld>(camera))
        .map(|state| state.transform);
    let after = probe(world);
    let mut steps = world.get_resource_or_init::<CameraBlockSteps>();
    steps.camera_world = camera;
    steps.crt_after = after;
    Ok(())
}

/// IDA: N/A each placement step's z (run-ak's STEP lines), the scene camera's world transform after `0066B710` (run-ak's
/// CAM line; the RaceSexMenu's `005C2BF0` moves it later in the move) and the main thread's CRT (`_holdrand`, draws) around
/// the block, for the tests.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct CameraBlockSteps {
    pub z: Vec<f32>,
    pub camera_world: Option<crate::gamebryo::transform::NiTransform>,
    pub crt_before: Option<(u32, u64)>,
    pub crt_after: Option<(u32, u64)>,
}

/// IDA: 00B3BAC8 and 00B3BAC4 (0066B710's Z rotation and pitch), 00B3BCCC (its statics' guard bits), 00B3BCB0's
/// g_niPoint3Zero_0 (the last TerrainLOD position) and 00B3BB8C's vector (the last view direction).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct PlayerCameraGlobals {
    pub z_rotation_b3bac8: f32,
    pub pitch_b3bac4: f32,
    pub guard_b3bccc: u32,
    pub last_lod_position: [f32; 3],
    pub last_direction_b3bb8c: [f32; 3],
}

/// IDA: 0066B710 PlayerCharacter_UpdateCamera(force), the first-person arm (+0x588 clear and 00B3BB04 clear). 00B3BAC8 =
/// the Z rotation (PlayerCharacter_GetZRotation), 00B3BAC4 = +0x20 (004A9720); a dead player's eye shrink is a stop.
/// 0070FDD0(Z) times 0070FD30(pitch); `005E6C10` (a shown DialogMenu, a stop here) else Camera01's (00B3BB0C) local
/// +0x3C and +0x40: 0070FDD0(-a x pi x 0.5) and 0070FD30(b x pi x 0.5), each product stored as a float; the position is
/// Camera01's world translate (+0x88). `004710B0` on the skin's bone 0 (counted: run-ak/run-al show it at 1.0 or more, so
/// the Bip move is skipped); `00531F10` on the camera collision helper (counted: 0, its phantom is in no world). The
/// World SceneGraph's first child takes the position and rotation, then NiAVObject_Update(0.0, 0); the listener
/// (006A8E10, 006A8E40 with the Z rotation's cos and sin, `place_listener`); `004D4EB0` with the cell and `005E06C0`'s
/// underwater test (counted); the statics' first-call zeros; the camera's world column 0 normalized; farther than 64 from
/// the last LOD position (or the view test, or `force`) runs TerrainLOD_UpdateForPosition and `004EA6E0` (counted) and
/// keeps the position (z 0) and the direction. Completion: 100% of the first-person arm's stores; the LOD and sound
/// calls are counted.
/// TODO: [ ] The view test within 64 (NiCamera +0xF0/+0x120's FOV ratio), the third-person and vanity arms.
/// # Errors
/// The stops named.
pub fn player_update_camera(
    world: &mut World,
    player: Entity,
    force: bool,
) -> Result<(), GameError> {
    use crate::gamebryo::transform::multiply;
    let heading = player_heading(world, player);
    let pitch = world
        .get::<TesObjectRefr>(player)
        .expect("the player")
        .rotation_20[0];
    {
        let mut globals = world.get_resource_or_init::<PlayerCameraGlobals>();
        globals.z_rotation_b3bac8 = heading;
        globals.pitch_b3bac4 = pitch;
    }
    if actor_is_dead(world, player, false) {
        return Err(GameError::unclassified(
            0x0066B75E,
            "0066B75E: a dead player's eye height",
        ));
    }
    if world
        .get::<PlayerCharacter>(player)
        .expect("the player")
        .view_flags_588[0]
        != 0
    {
        return Err(GameError::unclassified(
            0x0066B77F,
            "0066B77F: the third-person camera",
        ));
    }
    if world
        .get_resource::<crate::tes::interface_manager::InterfaceMovieReads>()
        .is_some_and(|reads| reads.tile_menu_present(1009))
    {
        return Err(GameError::unclassified(
            0x0066B800,
            "0066B800: 005E6C10 with a DialogMenu",
        ));
    }
    let camera01 = world
        .get_resource::<super::player_3d::FirstPersonNodes>()
        .and_then(|nodes| nodes.camera_b3bb0c)
        .ok_or(GameError::unclassified(
            0x0066B80B,
            "0066B80B: a null Camera01 faults",
        ))?;
    let local = world
        .get::<crate::gamebryo::scene_node::SceneNodePayload>(camera01)
        .map(|payload| payload.local_transform.rotation)
        .ok_or(GameError::unclassified(
            0x0066B80B,
            "0066B80B: Camera01 isn't a NiAVObject",
        ))?;
    let half_pi = std::f64::consts::PI as f32;
    let turn = x87::stored_float(-f64::from(local[3]) * f64::from(half_pi) * 0.5);
    let tilt = x87::stored_float(f64::from(local[4]) * f64::from(half_pi) * 0.5);
    let mut rotation = multiply(&rotation_z(heading), &rotation_x(pitch));
    rotation = multiply(&rotation, &rotation_z(turn));
    rotation = multiply(&rotation, &rotation_x(tilt));
    let translation = world
        .get::<crate::gamebryo::scene_node::NiAvWorld>(camera01)
        .map(|state| state.transform.translation)
        .ok_or(GameError::unclassified(
            0x0066B887,
            "0066B887: Camera01 with no world transform",
        ))?;
    count(world, 0x0047_10B0);
    count(world, 0x0053_1F10);
    let graph = world
        .get_resource::<crate::bs::scene_graphs::SceneGraphGlobals>()
        .and_then(|globals| globals.world)
        .ok_or(GameError::unclassified(
            0x0066BE4C,
            "0066BE4C: a null g_worldSceneGraph faults",
        ))?;
    let child = world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(graph)
        .and_then(|children| children.slots_ac.first().copied().flatten())
        .ok_or(GameError::unclassified(
            0x0066BE5A,
            "0066BE5A: a World SceneGraph with no first child faults",
        ))?;
    {
        let mut payload = world
            .get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(child)
            .ok_or(GameError::unclassified(
                0x0066BE5A,
                "0066BE5A: a first child other than a NiNode",
            ))?;
        payload.local_transform.translation = translation;
        payload.local_transform.rotation = rotation;
    }
    let mut runtime = crate::gamebryo::scene_update::NoSceneControllers;
    crate::gamebryo::scene_update::update_scene_object(world, child, 0.0, false, &mut runtime)?;
    let position = world
        .get::<TesObjectRefr>(player)
        .expect("the player")
        .position_2c;
    place_listener(world, player, position);
    if parent_cell(world, player).is_some() {
        count(world, 0x004D_4EB0);
        if world
            .get_resource::<crate::tes::os_globals::OsGlobalsData>()
            .is_some_and(|os| os.sound_24.is_some())
        {
            count(world, 0x005E_06C0);
        }
    }
    let camera = world
        .get::<crate::bs::scene_graphs::SceneGraph>(graph)
        .and_then(|scene| scene.camera_dc)
        .ok_or(GameError::unclassified(
            0x0066BFFC,
            "0066BFFC: a null scene camera faults",
        ))?;
    let world_rotation = world
        .get::<crate::gamebryo::scene_node::NiAvWorld>(camera)
        .map(|state| state.transform.rotation)
        .ok_or(GameError::unclassified(
            0x0066C006,
            "0066C006: the camera with no world transform",
        ))?;
    let mut globals = *world.get_resource_or_init::<PlayerCameraGlobals>();
    globals.guard_b3bccc |= 6;
    let offset = [
        x87::stored_float(f64::from(translation[0]) - f64::from(globals.last_lod_position[0])),
        x87::stored_float(f64::from(translation[1]) - f64::from(globals.last_lod_position[1])),
        x87::stored_float(0.0 - f64::from(globals.last_lod_position[2])),
    ];
    let column = [world_rotation[0], world_rotation[3], world_rotation[6]];
    let length = (f64::from(column[0]) * f64::from(column[0])
        + f64::from(column[1]) * f64::from(column[1])
        + f64::from(column[2]) * f64::from(column[2]))
    .sqrt();
    let direction = if length > 0.0 {
        column.map(|value| (f64::from(value) / length) as f32)
    } else {
        column
    };
    let distance = x87::stored_float(
        (f64::from(offset[1]) * f64::from(offset[1])
            + f64::from(offset[0]) * f64::from(offset[0])
            + f64::from(offset[2]) * f64::from(offset[2]))
        .sqrt(),
    );
    if distance.abs() > 64.0 || force {
        count(world, 0x004E_BDA0);
        count(world, 0x004E_A6E0);
        globals.last_lod_position = [translation[0], translation[1], 0.0];
        globals.last_direction_b3bb8c = direction;
    } else {
        return Err(GameError::unclassified(
            0x0066C0E8,
            "0066C0E8: the camera's view test within 64",
        ));
    }
    *world.resource_mut::<PlayerCameraGlobals>() = globals;
    Ok(())
}

/// IDA: 00B33A30, which 004424D0 accumulates.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct TesUpdateGlobals {
    pub time_b33a30: f32,
}

// IDA: 0065DA60 PlayerCharacter_GetZRotation: the reference's +0x28 (0065ABB0) plus +0x61C, a float; below 0.0 it gains 2pi, above 2pi it loses it, a float. Completion: 100%.
pub(crate) fn player_heading(world: &World, player: Entity) -> f32 {
    let two_pi = 6.283_185_5_f32;
    let base = world
        .get::<TesObjectRefr>(player)
        .expect("the player")
        .rotation_20[2];
    let offset = world
        .get::<PlayerCharacter>(player)
        .map_or(0.0, |data| data.z_rotation_offset_61c);
    let heading = x87::stored_float(f64::from(offset) + f64::from(base));
    if heading < 0.0 {
        x87::stored_float(f64::from(heading) + f64::from(two_pi))
    } else if two_pi < heading {
        x87::stored_float(f64::from(heading) - f64::from(two_pi))
    } else {
        heading
    }
}

// IDA: 0066EF95..0066F026 with OSGlobals +0x24: 006A8E10(position): +0x80, +0x84, +0x88 = z + 128.0 (00A3F428); then the heading's `_CIcos` and `_CIsin`, each a float, into 006A8E40(sin, cos, 0): +0x8C, +0x90, +0x94, then +0x98 = +0x9C = 0.0, +0xA0 = 1.0. Completion: 100%.
fn place_listener(world: &mut World, player: Entity, position: [f32; 3]) {
    let cos = f64::from(player_heading(world, player)).cos() as f32;
    let sin = f64::from(player_heading(world, player)).sin() as f32;
    let Some(mut os) = world.get_resource_mut::<crate::tes::os_globals::OsGlobalsData>() else {
        return;
    };
    let Some(sound) = os.sound_24.as_mut() else {
        return;
    };
    sound.listener_position_80 = [
        position[0],
        position[1],
        x87::stored_float(f64::from(position[2]) + 128.0),
    ];
    sound.listener_front_8c = [sin, cos, 0.0];
    sound.listener_top_98 = [0.0, 0.0, 1.0];
}

// IDA: 0066ED9D..0066EF95. The Z rotation wraps: above pi (00A3D5B8) it loses 2pi (00A3D5B0) until it isn't, below -pi (00A491E0) it gains 2pi until it isn't, stored as a float each step. The view is identity times 0070FDD0(z) (007100A0); the pitch is x - y for z at or above 0.0, else y + x, a float, through 0070FD30. With a player node (vt+0x154, 006600F0) its +0x30 takes the pitch. The pitch times the view goes to the World SceneGraph's first child's +0x30 rotation and +0x54 the position, then NiAVObject_Update(0.0, 0) on it, and TES +0x0C's 00707610 and 00707980. Completion: 100% with no player node.
fn place_camera(
    world: &mut World,
    player: Entity,
    position: [f32; 3],
    rotation: [f32; 3],
) -> Result<(), GameError> {
    use crate::gamebryo::transform::multiply;
    const IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    let pi = std::f32::consts::PI;
    let two_pi = 6.283_185_5_f32;
    let mut z = rotation[2];
    while z > pi {
        z = x87::stored_float(f64::from(z) - f64::from(two_pi));
    }
    while z < -pi {
        z = x87::stored_float(f64::from(z) + f64::from(two_pi));
    }
    let view = multiply(&IDENTITY, &rotation_z(z));
    let pitch_angle = if z >= 0.0 {
        x87::stored_float(f64::from(rotation[0]) - f64::from(rotation[1]))
    } else {
        x87::stored_float(f64::from(rotation[1]) + f64::from(rotation[0]))
    };
    let pitch = rotation_x(pitch_angle);
    // 0066EEA2..0066EECC vt+0x154 (006600F0 PlayerCharacter_GetNiNode: +0x5D0 unless +0x588 is set or it's null, else
    // 00612220's +0x3C): a node takes the pitch as its +0x30 rotation.
    let node = world.get::<PlayerCharacter>(player).and_then(|data| {
        if data.view_flags_588[0] != 0 {
            None
        } else {
            data.first_person_node_5d0
        }
    });
    let node = node.or_else(|| {
        world
            .get::<crate::tes::reference_3d::Reference3D>(player)
            .map(|loaded| loaded.0)
            .filter(|&node| {
                world
                    .get::<crate::gamebryo::scene_node::SceneNodePayload>(node)
                    .is_some_and(|payload| payload.effective_av_flags & 1 == 0)
            })
    });
    if let Some(node) = node
        && let Some(mut payload) =
            world.get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(node)
    {
        payload.local_transform.rotation = pitch;
    }
    let combined = multiply(&pitch, &view);
    let graph = world
        .get_resource::<crate::bs::scene_graphs::SceneGraphGlobals>()
        .and_then(|globals| globals.world)
        .ok_or(GameError::unclassified(
            0x0066EEF7,
            "0066EEF7: a null g_worldSceneGraph faults",
        ))?;
    let child = world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(graph)
        .and_then(|children| children.slots_ac.first().copied().flatten())
        .ok_or(GameError::unclassified(
            0x0066EF0D,
            "0066EF0D: a World SceneGraph with no first child faults",
        ))?;
    {
        let mut payload = world
            .get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(child)
            .ok_or(GameError::unclassified(
                0x0066EF0D,
                "0066EF0D: a first child other than a NiNode",
            ))?;
        payload.local_transform.rotation = combined;
        payload.local_transform.translation = position;
    }
    let mut runtime = crate::gamebryo::scene_update::NoSceneControllers;
    crate::gamebryo::scene_update::update_scene_object(world, child, 0.0, false, &mut runtime)?;
    let root = crate::tes::tes_grid::tes(world)?
        .scene_root_0c
        .ok_or(GameError::unclassified(
            0x0066EF82,
            "0066EF82: a null TES +0x0C",
        ))?;
    let defaults = world
        .resource::<crate::gamebryo::registration::NiProcessDefaults>()
        .scene_property_defaults();
    crate::gamebryo::scene_state::refresh_property_state(world, root, &defaults)?;
    crate::gamebryo::scene_state::refresh_effect_state(world, root)
}

/// IDA: N/A the counts of the move's calls the port doesn't make.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct PlayerMoveBoundaries {
    pub calls: std::collections::BTreeMap<u32, u32>,
}

// IDA: N/A a helper, with no address of its own.
pub(crate) fn count(world: &mut World, address: u32) {
    *world
        .get_resource_or_init::<PlayerMoveBoundaries>()
        .calls
        .entry(address)
        .or_default() += 1;
}

// IDA: 00560110: the tree manager (0055F750(0) when null, a stop here); with its +0x04 set, that and 00B43108 are released and 00440420 reloads Data\Textures\Trees\CanopyShadow.dds, a stop. A null +0x04 returns. Completion: 100% of the null arm.
fn reset_tree_canopy(world: &World) -> Result<(), GameError> {
    let globals = world.get_resource::<crate::bs::tree_manager::TreeManagerGlobals>();
    let Some(manager) = globals.and_then(|globals| globals.tree_manager_b39e04.as_ref()) else {
        return Err(GameError::unclassified(
            0x0056011B,
            "0056011B: 00560110 with no tree manager",
        ));
    };
    if manager.pointer_04.is_some() {
        return Err(GameError::unclassified(
            0x00560158,
            "00560158: reloading the canopy shadow",
        ));
    }
    Ok(())
}

// IDA: 00540380 on the Sky: +0xF4 = 0.0; +0x1C, +0x14 and +0x10 = 0; +0xFC = (+0xFC & ~8) | 1; the clouds' 0053BBC0 and the precipitation's 0053D6C0, the sky renderer's, counted. Completion: 100% of the state.
fn sky_reset_weather(world: &mut World) {
    let (clouds, precipitation) = {
        let Some(mut globals) = world.get_resource_mut::<crate::tes::sky::SkyGlobals>() else {
            return;
        };
        let Some(sky) = globals.singleton_b365c4.as_mut() else {
            return;
        };
        sky.float_f4 = 0.0;
        sky.weather_override_1c = 0;
        sky.weather_14 = 0;
        sky.first_weather_10 = 0;
        sky.dword_fc = (sky.dword_fc & !8) | 1;
        (sky.clouds_2c.is_some(), sky.precipitation_38.is_some())
    };
    if clouds {
        count(world, 0x0053_BBC0);
    }
    if precipitation {
        count(world, 0x0053_D6C0);
    }
}
