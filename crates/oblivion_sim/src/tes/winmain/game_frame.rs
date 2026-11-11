//! The ordered calls of one game frame. The idle-only entry shares the guard and systems with the host.
use bevy_ecs::prelude::*;
use bevy_ecs::schedule::{
    LogLevel, ScheduleBuildSettings, ScheduleLabel, Schedules, SingleThreadedExecutor,
};
use oblivion_core::error::GameError;
use oblivion_core::logging::{Level, emit};

use super::winmain_menu_media::{MainMenuMedia, MainMenuMediaStage, install_script_hosts};
use super::{
    MenuMediaContext, WinMainProgress, driver_crt, menu_media_context, with_driver_interface,
};
use crate::tes::ai_conditions::ConditionCaches;
use crate::tes::ai_process::AiProcessGlobals;
use crate::tes::global::time_globals_advance;
use crate::tes::idle_frame::{IdleFrameGlobals, count_idle_call, idle_frame_resets};
use crate::tes::interface_frame::top_stacked_menu_mode;
use crate::tes::interface_manager::InterfaceManager;
use crate::tes::movie_player::MoviePlayer;
use crate::tes::os_globals::{OsGlobalsData, lock_shared};
use crate::tes::os_sound_music::SoundFrame;
use crate::tes::tes_grid::run_grid_scripts_frame;
use crate::tes::tile_values::{TRAIT_USER0, tile_numeric_trait};
use crate::tes::window::time_info_update;

/// IDA: N/A the schedule stepped once per host update.
#[derive(ScheduleLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GameFrame;
/// IDA: N/A the directly callable idle portion of the same schedule.
#[derive(ScheduleLabel, Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct IdleOnly;

/// IDA: N/A scheduling boundaries in the WinMain loop. World owns the fallible scripts, grid and world tail together.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameSet {
    WinMain,
    Begin,
    Time,
    Input,
    IoAndLod,
    Faders,
    Sound,
    Resets,
    World,
    End,
    MovieExit,
    Interface,
}

/// IDA: N/A locals of this invocation, retained from the guard sample until the final counted call.
#[derive(Resource)]
struct IdleFrame {
    context: MenuMediaContext,
    player: MoviePlayer,
    menu_mode: bool,
    ticks: u32,
    seconds: f32,
    stop: Option<GameError>,
}

/// IDA: N/A the frame for a bare world (tests): the ordered sets and the idle frame's systems. `sim_app` builds the
/// same frame from `frame_sets` and `SimPlugins` instead. The host supplies the WinMain, movie/exit and interface systems.
pub fn game_frame_schedule() -> Schedule {
    build_schedule(GameFrame)
}

/// IDA: N/A the `GameFrame` schedule with its ordered sets and no systems, for `sim_app`'s plugins to fill.
pub(crate) fn empty_game_frame() -> Schedule {
    frame_sets(GameFrame)
}

/// IDA: N/A both entry points use the same ordering and idle guards.
fn build_schedule(label: impl ScheduleLabel) -> Schedule {
    let mut schedule = frame_sets(label);
    add_systems(&mut schedule);
    schedule
}

/// IDA: N/A the WinMain loop's boundaries in the exe's order, run on one thread with ambiguity as an error.
fn frame_sets(label: impl ScheduleLabel) -> Schedule {
    use FrameSet::*;
    let mut schedule = Schedule::new(label);
    schedule.set_executor(SingleThreadedExecutor::new());
    schedule.set_build_settings(ScheduleBuildSettings {
        ambiguity_detection: LogLevel::Error,
        ..Default::default()
    });
    schedule.configure_sets(
        (
            WinMain, Begin, Time, Input, IoAndLod, Faders, Sound, Resets, World, End, MovieExit,
            Interface,
        )
            .chain(),
    );
    schedule
}

/// IDA: N/A the simulation's frame systems, every one a step of 0040D800 OSGlobals_RunIdleFrame: the entry guards, the
/// clock, the input poll, I/O and LOD, the faders, the sound frame, the resets, the scripts/world transaction and the
/// end, each run only inside a frame `begin` opened. The one list: `IdleFramePlugin` and the bare-world frame call it.
/// The subsystems' own code (`time_info_update`, the sound frame's `update_frame`, `update_process_manager`) stays in
/// their folders (restructure Stage 10, §12).
pub fn add_systems(schedule: &mut Schedule) {
    use FrameSet::*;
    schedule.add_systems(begin.in_set(Begin));
    schedule.add_systems(
        (
            update_time.in_set(Time),
            poll_input.in_set(Input),
            io_and_lod.in_set(IoAndLod),
            update_faders.in_set(Faders),
            update_sound.in_set(Sound),
            idle_frame_resets.in_set(Resets),
            scripts_and_world.in_set(World),
            end.in_set(End),
        )
            .run_if(resource_exists::<IdleFrame>),
    );
    #[cfg(debug_assertions)]
    schedule.add_systems(
        crate::tes::actor_process::check_process_level_markers
            .in_set(World)
            .after(scripts_and_world)
            .run_if(resource_exists::<IdleFrame>),
    );
}

/// IDA: N/A registers the idle frame's systems in `GameFrame`.
pub struct IdleFramePlugin;

impl bevy_app::Plugin for IdleFramePlugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.edit_schedule(GameFrame, add_systems);
    }
}

/// IDA: N/A the simulation's plugins. Only the idle frame owns frame systems (restructure Stage 10, §12).
pub struct SimPlugins;

impl bevy_app::PluginGroup for SimPlugins {
    fn build(self) -> bevy_app::PluginGroupBuilder {
        bevy_app::PluginGroupBuilder::start::<Self>().add(IdleFramePlugin)
    }
}

/// IDA: 0040D800 OSGlobals_RunIdleFrame, a frame past 0040F0D1, in the exe's order: 0040D80F 00889A30(menu mode); 0040D817
/// the first menu frame's weapon-node resets (00B3341C); 0040D893 00498F00 (counted); 0040D898 ++00B02C54; 0040D8AB
/// TimeInfo_Update(GetTickCount); 0040D8C2 00889810 bhk_AccumulateFrameTime(frame time, 00B333B8); 0040D8CD the input poll;
/// 0040D8D8 IOManager_RunTasksTimed (counted: no task is queued past the move's drain); the LOD fade factors; 0040D9CF
/// 007C1F50 (counted, D10); the image-space texture release (00B33397 clear: nothing to release); 0040DB49 00572F60(1);
/// outside menu mode the time advance; 0040DBB4 the sound frame; 0040DBB9..0040DBCB the per-frame resets; 0040DBD1
/// 004F5DB0; 0040DBDC TES_RunGridScripts; 0040DBEC 00674A20; 0040DC2A ActorProcessManager_Update outside menu mode
/// or under fully active fader 2, then queued references and package followers; in menu mode 0040DC70 0065E900 and 0040DD66 0055FA50(camera, 1); PurgeLists and 004424D0 outside menu mode
/// (the Sky's update stands for 004424D0 there), or in menu mode under a Dialog's +0x64 or fader 2 (a stop); 0040DEFC
/// 00407AE0 (counted). The interface's three calls (0040DF01..0040DF0B) are `run_screens_frame`'s, which the host runs
/// after this; the render (0040DF54 0040D4D0) is Bevy's. Completion: the menu-mode arms the frames after command 64 reach; outside
/// menu mode the arms Phase 22 ported.
/// TODO: [ ] Outside menu mode: 0040DB5B..0040DBA5 (TES_TestAllCells, 00411330, 00440400), the player's
/// frame (0040DC82..0040DD4E), 0040DD6E..0040DE3A and PurgeLists; 004424D0 under fader 2. See actor_process_frame for nested AI stops.
pub fn run_idle_frame(world: &mut World) {
    if world
        .get_resource::<Schedules>()
        .is_none_or(|s| !s.contains(IdleOnly))
    {
        world.add_schedule(build_schedule(IdleOnly));
    }
    world.run_schedule(IdleOnly);
}

/// IDA: 0040D800..0040D898 the idle-entry guards, menu-mode sample, weapon reset and frame counter.
fn begin(world: &mut World) {
    // A host stop or an exit means no menu, so no movie and no music.
    if !world
        .get_resource::<WinMainProgress>()
        .is_some_and(|progress| {
            progress.finished() && progress.stops.is_empty() && progress.exit_code.is_none()
        })
    {
        return;
    }
    if world
        .get_resource::<MainMenuMedia>()
        .is_none_or(|media| media.stage != MainMenuMediaStage::Running)
    {
        return;
    }
    let Some(player) = world.get_resource::<MoviePlayer>().cloned() else {
        return;
    };
    let Some(context) = menu_media_context(world) else {
        return;
    };
    // 0040D809 and 0040D817: the frame's two menu-mode tests (nothing between them changes the answer).
    let menu_mode = crate::tes::interface_manager::is_menu_mode(world);
    crate::bs::havok::havok_set_menu_mode(world, menu_mode);
    crate::tes::idle_frame::idle_frame_weapon_node_resets(world, menu_mode);
    count_idle_call(world, 0x0049_8F00);
    {
        let mut globals = world.get_resource_or_init::<IdleFrameGlobals>();
        globals.frame_counter_b02c54 = globals.frame_counter_b02c54.wrapping_add(1);
    }
    world.insert_resource(IdleFrame {
        context,
        player,
        menu_mode,
        ticks: 0,
        seconds: 0.0,
        stop: None,
    });
}

/// IDA: 0040D89F..0040D8C2 update the shared clock and accumulate the Havok step before the input poll.
fn update_time(world: &mut World) {
    world.resource_scope(|world, mut frame: Mut<IdleFrame>| {
        let context = &frame.context;
        let tick = context.clock.tick();
        if world
            .get_resource::<crate::tes::window::TimeInfoHandle>()
            .is_none()
        {
            world.insert_resource(crate::tes::window::TimeInfoHandle(context.time.clone()));
        }
        let (time_info_10, frame_seconds) = {
            let mut time = context.time.lock();
            // 0040D89F..0040D8AB.
            time_info_update(&mut time, tick, context.time_multiplier_b06704);
            (time.paused_ticks_10, time.frame_seconds_0c)
        };
        let havok_flag = world.resource::<IdleFrameGlobals>().havok_step_flag_b333b8 != 0;
        crate::tes::cell_havok::bhk_accumulate_frame_time(world, frame_seconds, havok_flag);
        frame.ticks = time_info_10;
        frame.seconds = frame_seconds;
    });
}

/// IDA: 0040D8CD the main thread's device poll, using this frame's updated tick.
fn poll_input(frame: Res<IdleFrame>, os: Res<OsGlobalsData>) {
    if let Some(input) = os.input_20.as_ref() {
        // The movie thread uses the same lock order.
        let mut input = input.lock();
        let mut platform = lock_shared(&frame.context.os_platform);
        input.poll_devices(frame.ticks, &mut *platform);
    }
}

/// IDA: 0040D8D8..0040DB39 the timed I/O boundary, LOD factors and the shadow/image-space boundaries.
fn io_and_lod(world: &mut World) {
    count_idle_call(world, 0x0043_3590);
    let stop = crate::tes::idle_frame::idle_frame_lod_fade(world).err();
    count_idle_call(world, 0x007C_1F50);
    // 0040D9D4..0040DB39: with 00B33397 clear (only OSGlobals_RenderWorldToMenuBackgroundTexture sets it, and the menus
    // here don't render the world into a texture) the release arm finds nothing.
    world.resource_mut::<IdleFrame>().stop = stop;
}

/// IDA: 0040DB49 the ordinary fader update, only if LOD succeeded.
fn update_faders(world: &mut World) {
    let frame = world.resource::<IdleFrame>();
    if frame.stop.is_none() {
        let result = crate::tes::faders::fader_frame(world, true, frame.seconds);
        world.resource_mut::<IdleFrame>().stop = result.err();
    }
}

/// IDA: 0040DBAB..0040DBB4 the menu, movie and player-cell reads and the sound frame.
fn update_sound(world: &mut World) {
    world.resource_scope(|world, frame: Mut<IdleFrame>| {
        let context = &frame.context;
        // 0040DBAB, and 006AE89E's 00578FE0: no manager, cursor or root: 0; root trait 0xFAE == 2.0: 0057CF60; else 0.
        let top_menu_mode = with_driver_interface(world, |cx| {
            let manager = cx.world.get_resource::<InterfaceManager>()?;
            manager.cursor_01c?;
            let root = manager.menu_root_068?;
            let stack = manager.menu_mode_stack_0e0;
            (tile_numeric_trait(cx, root, TRAIT_USER0) == 2.0)
                .then(|| top_stacked_menu_mode(&stack))
        })
        .flatten()
        .unwrap_or(0);
        let movie_state = lock_shared(&frame.player.globals.current_b33428)
            .as_ref()
            .map(|movie| movie.state());
        // 006AD068 g_thePlayer's +0x40 as the frame reads it (the startup context's copy is Phase 16's), and an interior's
        // 004CAD00 for 006AD0E9.
        let player_cell = crate::tes::player::the_player(world).and_then(|player| {
            world
                .get::<crate::tes::actor::TesObjectRefr>(player)
                .and_then(|refr| refr.parent_cell_40)
        });
        let player_cell_music_type = player_cell
            .filter(|&cell| crate::tes::forms_world::cell_is_interior(world, cell))
            .and_then(|cell| crate::tes::forms_world::cell_resolve_music_type(world, cell).ok());
        // 006A8E80's draws, through 0047DF80 on the main thread.
        let music_draws = driver_crt(world);
        {
            let mut os = world.resource_mut::<OsGlobalsData>();
            if let Some(sound) = os.sound_24.as_mut() {
                // 0040DBB4 006AE860.
                sound.update_frame(
                    &SoundFrame {
                        sound_enabled_b16178: context.sound_enabled_b16178,
                        movie_state,
                        top_menu_mode,
                        menu_mode: frame.menu_mode,
                        time_info_10: frame.ticks,
                        player_has_cell: context.player_has_cell || player_cell.is_some(),
                        player_cell_music_type,
                    },
                    &context.settings,
                    &mut *lock_shared(&context.music),
                    &mut || music_draws.random_next(),
                );
            }
        }
    });
}

// IDA: 0040DBE6..0040DC61 in OSGlobals_RunIdleFrame. Each phase store precedes its callee.
// Completion: the manager, queued reference and menu/fader decisions in the caller's transaction.
// TODO: [ ] ActorProcessManager_UpdateQueuedPackageFollowers on a nonempty queue.
fn process_actor_frame(world: &mut World, frame_seconds: f32) -> Result<(), GameError> {
    use crate::tes::actor_process::ActorProcessGlobals;
    use crate::tes::actors::actor_process_frame::process_manager_update_queued;
    use crate::tes::actors::actor_process_frame::update_process_manager;
    world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 0;
    process_manager_update_queued(world)?;
    world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 50;
    if !crate::tes::interface_manager::is_menu_mode(world)
        || crate::tes::faders::fader_active(world, 2)
    {
        world
            .resource_mut::<ActorProcessGlobals>()
            .detection_picks_this_frame_b333bc = 0;
        world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 100;
        update_process_manager(world, frame_seconds)?;
        world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 200;
        process_manager_update_queued(world)?;
        world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 300;
        if !world
            .resource::<ActorProcessGlobals>()
            .manager
            .queued_58
            .is_empty()
        {
            return Err(GameError::unclassified(
                0x00674972,
                "00674972: update queued package followers",
            ));
        }
        world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 400;
    }
    Ok(())
}

/// IDA: 0040DBD1..0040DEA1 the scripts/world transaction. Its error supersedes the saved LOD/fader stop.
fn scripts_and_world(world: &mut World) {
    world.resource_scope(|world, mut frame: Mut<IdleFrame>| {
        // 0040DBDC 004402F0. A stop ends the scripts and nothing else: the menu's clock, music and movie go on.
        if !world.resource::<MainMenuMedia>().scripts_stopped {
            install_script_hosts(world, &frame.context);
            // 0040DB9F, outside menu mode, with the rest of 0040DB7A..0040DBAA not ported: `docs/open-items.md`.
            let advanced = if frame.menu_mode {
                Ok(())
            } else {
                time_globals_advance(world, frame.seconds)
            };
            // 0040DBD1 004F5DB0.
            if let Some(mut globals) = world.get_resource_mut::<AiProcessGlobals>() {
                globals.caches = ConditionCaches::default();
            }
            let sky_draws = driver_crt(world);
            let ran = advanced
                .and_then(|()| run_grid_scripts_frame(world, frame.seconds))
                .and_then(|()| {
                    // The scripts can open or close a menu. 0040DBFB and 0040DC61 both read it again.
                    process_actor_frame(world, frame.seconds)?;
                    if crate::tes::interface_manager::is_menu_mode(world) {
                        return menu_mode_frame_tail(world);
                    }
                    // 0040DEA1: the non-menu player and grid calls preceding sky are still carried.
                    crate::tes::sky_update::sky_update(world, &mut || Ok(sky_draws.random_next()))
                });
            let stop = ran
                .err()
                .map(|error| error.message)
                .or(frame.stop.take().map(|error| error.message));
            if let Some(stop) = stop {
                emit(
                    Level::Error,
                    &format!("0040DBDC: the idle frame's scripts stopped: {stop}"),
                );
                let mut media = world.resource_mut::<MainMenuMedia>();
                media.scripts_stopped = true;
                media.scripts_stop = Some(stop);
            }
        }
    });
}

/// IDA: 0040DEFC count the debug-text call and drop the frame samples before the interface runs.
fn end(world: &mut World) {
    count_idle_call(world, 0x0040_7AE0);
    world.remove_resource::<IdleFrame>();
}

/// IDA: 0040DC70..0040DEA1, the menu-mode tail after the process-manager calls.
/// Completion: the bow, camera and tree calls. The dialog and fader world update remains a stop.
pub(crate) fn menu_mode_frame_tail(
    world: &mut World,
) -> Result<(), oblivion_core::error::GameError> {
    use oblivion_core::error::GameError;
    crate::tes::player::player_menu_bow_check(world)?;
    crate::tes::idle_frame::camera_tree_frame(world, true)?;
    // 0057AB70: a Dialog menu's +0x64 (the port has no DialogMenu class: any Dialog tile is a stop).
    let dialog = with_driver_interface(world, |cx| cx.engine.tile_menu_by_id(1009).is_some())
        .unwrap_or(false);
    if dialog || crate::tes::faders::fader_active(world, 2) {
        return Err(GameError::unclassified(
            0x0040DE48,
            "0040DE48: PurgeLists and 004424D0 under a Dialog or fader 2",
        ));
    }
    Ok(())
}
