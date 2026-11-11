//! IDA: N/A the plugin that runs `oblivion_sim::tes::winmain`'s chain once per frame, on the main thread, with nothing else touching game state while a block runs. The simulation knows nothing of Bevy's rendering, audio or FFmpeg, so its service seams are filled from here: `ffmpeg_bink`, `movie_frames`, `audio` and `raw_input`. The screens read `oblivion_sim::tes::winmain::take_render_requests` and `in_scene_flag`, and `movie_frames::LatestMovieFrame` and `raw_input::RawInputFrame`. Completion: the schedule, and every seam but the screens'.

use std::sync::{Arc, Mutex};

use bevy::input::InputSystems;
use bevy::prelude::*;
use oblivion_sim::tes::os_globals::lock_shared;
use oblivion_sim::tes::winmain::game_frame::{FrameSet, GameFrame};
use oblivion_sim::tes::winmain::winmain_host::working_directory_of;
use oblivion_sim::tes::winmain::winmain_screens::open_main_menu;
use oblivion_sim::tes::winmain::{
    Block, WinMainConfig, WinMainProgress, WinMainSeams, drain_movie_device_queue, os_input_frame,
    render_requests, shared_files, winmain_schedule, winmain_shutdown,
};

use crate::audio::{AudioAssetReader, HostAudio, HostMusicDevice, apply_host_audio};
use crate::ffmpeg_bink::FfmpegBink;
use crate::movie_frames::{
    GamePresents, LatestMovieFrame, SharedMovieFrames, load_splash_with_ffmpeg, publish_movie_frame,
};
use crate::raw_input::{HostInputHandle, RawInputFrame, feed_raw_input};

/// IDA: N/A the simulation's own App (restructure Stage 8a): its world holds the game and its `GameFrame` schedule. The host
/// steps it once a frame (`run_winmain_frame`) and reaches it otherwise only through `with_sim`, at the places
/// `docs/audit/restructure-stage8a-accessors.md` lists.
pub struct SimWorld(pub App);

/// IDA: N/A runs `body` with the host's world and the simulation's. Completion: 100%.
/// # Panics
/// With no `SimWorld` (a host built without `WinMainPlugin`).
pub fn with_sim<R>(host: &mut World, body: impl FnOnce(&mut World, &mut World) -> R) -> R {
    let mut sim = host
        .remove_non_send::<SimWorld>()
        .expect("the simulation world");
    let result = body(host, sim.0.world_mut());
    host.insert_non_send(sim);
    result
}

/// IDA: N/A the frame's raw input as the host saw it before the step: the window's DirectInput-shaped bytes and the window
/// as the game's screen. `screens::read_input` translates it inside the Interface set, with this frame's tick.
#[derive(Resource, Clone, Debug, Default)]
pub struct HostRawInput {
    pub raw: Option<RawInputFrame>,
    pub screen: Option<crate::game_screen::GameScreen>,
}

/// IDA: N/A the exit the chain or the screens asked for, taken at the MovieExit boundary for the host to request.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct HostExitRequest(pub Option<u8>);

/// IDA: N/A how long a resumable block may run in one frame (`--frame-budget-ms`'s default).
pub const DEFAULT_FRAME_BUDGET_MS: u64 = 16;

/// IDA: N/A the chain.
#[derive(Resource)]
struct WinMainChain(Schedule);

/// IDA: N/A host only.
pub struct WinMainPlugin {
    pub config: WinMainConfig,
}

impl Plugin for WinMainPlugin {
    /// IDA: N/A no address of its own; its comments cite 0057DE50.
    fn build(&self, app: &mut App) {
        let frames = SharedMovieFrames::default();
        let audio = HostAudio::default();
        let root = working_directory_of(&self.config.data_directory);
        let device_frames = frames.clone();
        let mut sim = oblivion_sim::sim_app(
            self.config.clone(),
            WinMainSeams {
                // IDA: N/A the recorded substitution for binkw32.
                bink: Arc::new(Mutex::new(FfmpegBink::default())),
                movie_device: Box::new(move |entry| lock_shared(&device_frames.0).apply(entry)),
                splash_loader: Some(Arc::new(load_splash_with_ffmpeg)),
                music: Arc::new(Mutex::new(HostMusicDevice::new(audio.clone(), root))),
                screens: {
                    // IDA: 0057DE50 through the host's catalog, a recorded substitution for 006ADE50's SOUN lookup by editor ID.
                    let ui = audio.clone();
                    std::rc::Rc::new(std::cell::RefCell::new(
                        oblivion_sim::tes::winmain::winmain_host::ScreenSeams {
                            play_ui_sound: Box::new(move |id| {
                                let _ = ui.play_ui(id);
                            }),
                            ..Default::default()
                        },
                    ))
                },
                ..WinMainSeams::default()
            },
        );
        // IDA: N/A the recorded substitution for Havok: the physics seam's backend is Avian (`oblivion_physics`), installed
        // in the simulation's world before the game reaches a cell's world (004D4A20).
        sim.world_mut()
            .insert_non_send(oblivion_sim::bs::physics_seam::PhysicsSeam(Box::new(
                oblivion_physics::AvianPhysics::new(),
            )));
        if let Some(input) = os_input_frame(sim.world()) {
            app.insert_resource(HostInputHandle(input));
        }
        if let Some(files) = shared_files(sim.world()) {
            // IDA: 00430EF0 with the mask 00431440 passes: the game's own finder and archives, once the load has them. Any type, so a host read doesn't depend on which archive holds a sound.
            app.insert_resource(AudioAssetReader(Arc::new(move |path| {
                let mut file = files.open_core(path, 0, 0x2800, 0xFFFF)?;
                let mut bytes = vec![0; file.size() as usize];
                let read = file.read(&mut bytes);
                (read == bytes.len()).then_some(bytes)
            })));
        }
        if let Some(requests) = render_requests(sim.world()) {
            app.insert_resource(GamePresents(requests));
        }
        sim.world_mut()
            .insert_resource(WinMainChain(winmain_schedule()));
        sim.world_mut().init_resource::<HostRawInput>();
        sim.world_mut().init_resource::<HostExitRequest>();
        add_host_frame_systems(
            sim.get_schedule_mut(GameFrame)
                .expect("sim_app adds GameFrame"),
        );
        app.insert_non_send(SimWorld(sim));
        app.insert_resource(frames);
        app.insert_resource(audio);
        app.init_resource::<LatestMovieFrame>();
        app.init_resource::<RawInputFrame>();
        app.add_systems(PreUpdate, feed_raw_input.after(InputSystems));
        app.add_systems(
            Update,
            (run_winmain_frame, apply_host_audio, publish_movie_frame).chain(),
        );
        app.add_systems(Last, stop_winmain_threads);
    }
}

/// IDA: N/A a capture of the LoadingMenu holds the chain here: the blocks past this one don't run.
#[derive(Resource, Clone, Copy, Debug)]
pub struct HoldChainAfter(pub Block);

/// IDA: N/A one frame of the chain at the host's boundary: the window's raw input goes in, the simulation runs its
/// `GameFrame`, and the exit it took at MovieExit (and `--exit-after-frames`'s) comes out. Completion: 100%.
pub fn run_winmain_frame(host: &mut World) {
    let raw = host.get_resource::<RawInputFrame>().cloned();
    let screen = {
        let mut windows = host.query::<&Window>();
        windows
            .iter(host)
            .next()
            .map(crate::game_screen::GameScreen::from_window)
    };
    with_sim(host, |host, sim| {
        sim.insert_resource(HostRawInput { raw, screen });
        sim.run_schedule(GameFrame);
        let code = sim.resource_mut::<HostExitRequest>().0.take();
        if let Some(code) = code {
            let exit = if code == 0 {
                AppExit::Success
            } else {
                AppExit::from_code(code)
            };
            crate::render_exit::request_exit(host, exit).expect("the exit guard is installed");
        }
        crate::exit_after_frames(host, sim);
    });
}

/// IDA: N/A the simulation's `GameFrame` with the host's WinMain and movie/exit systems, for a bare world (tests).
#[cfg(test)]
pub(crate) fn host_game_frame() -> Schedule {
    let mut frame = oblivion_sim::tes::winmain::game_frame::game_frame_schedule();
    add_host_frame_systems(&mut frame);
    frame
}

/// IDA: N/A the WinMain and device/exit boundaries the host adds to the simulation's frame.
fn add_host_frame_systems(frame: &mut Schedule) {
    frame.add_systems(run_startup_and_open_menu.in_set(FrameSet::WinMain));
    frame.add_systems(drain_movies_and_take_exit.in_set(FrameSet::MovieExit));
}

/// IDA: N/A the host's WinMain budget and menu-open transaction.
fn run_startup_and_open_menu(world: &mut World) {
    let progress = world.resource::<WinMainProgress>();
    let held = world
        .get_resource::<HoldChainAfter>()
        .is_some_and(|hold| progress.next > hold.0);
    let finished = progress.finished() || held;
    if !finished {
        world.resource_scope(|world, mut chain: Mut<WinMainChain>| chain.0.run(world));
        let progress = world.resource::<WinMainProgress>();
        if progress.finished() {
            let elapsed = progress.started.elapsed();
            let lines = progress.stops.len();
            println!(
                "[INFO] WinMain driver finished in {:.2} s, {lines} host stops",
                elapsed.as_secs_f64()
            );
            for (block, time) in &progress.block_times {
                println!("[INFO] block {block:?}: {:.3} s", time.as_secs_f64());
            }
        }
    }
    if let Err(error) = open_main_menu(world) {
        crate::screens::report_screen_stop(world, &error);
    }
}

/// IDA: N/A the worker's device requests and the exit code, taken at the boundary before the interface; the host requests
/// the exit after the step (`run_winmain_frame`).
pub(crate) fn drain_movies_and_take_exit(world: &mut World) {
    // The chain's `end_frame` stops with the chain; the movie thread goes on.
    drain_movie_device_queue(world);
    let code = world.resource_mut::<WinMainProgress>().exit_code.take();
    if code.is_some() {
        world.get_resource_or_init::<HostExitRequest>().0 = code;
    }
}

/// IDA: N/A `MoviePlayerGuard`: an exit stops and joins the movie thread first.
fn stop_winmain_threads(world: &mut World) {
    if world
        .get_resource::<Messages<AppExit>>()
        .is_some_and(|exits| !exits.is_empty())
    {
        with_sim(world, |_, sim| winmain_shutdown(sim));
        // Stage 8b: what the render mirror met under its roots and can't draw, once at exit.
        if let Some(mirror) = world.get_resource::<crate::world_view::RenderMirror>() {
            println!("[INFO] presentation gaps: {:?}", mirror.gaps.counts);
        }
    }
}
