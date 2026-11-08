//! IDA: N/A the plugin that runs `recomp_startup::winmain`'s chain once per frame, on the main thread, with nothing else touching game state while a block runs. `recomp_game` and `recomp_startup` know nothing of Bevy's rendering, audio or FFmpeg, so the seams they expose are filled from here: `ffmpeg_bink`, `movie_frames`, `audio` and `raw_input`. The screens read `recomp_startup::winmain::take_render_requests` and `in_scene_flag`, and `movie_frames::LatestMovieFrame` and `raw_input::RawInputFrame`. Completion: the schedule, and every seam but the screens'.

use std::sync::{Arc, Mutex};

use bevy::input::InputSystems;
use bevy::prelude::*;
use recomp_game::tes::os_globals::lock_shared;
use recomp_startup::winmain::{
    Block, WinMainConfig, WinMainProgress, WinMainSeams, drain_movie_device_queue, install_winmain,
    os_input_frame, render_requests, shared_files, winmain_schedule, winmain_shutdown,
};
use recomp_startup::winmain_host::working_directory_of;
use recomp_startup::winmain_menu_media::run_main_menu_media_frame;
use recomp_startup::winmain_screens::open_main_menu;

use crate::audio::{AudioAssetReader, HostAudio, HostMusicDevice, apply_host_audio};
use crate::ffmpeg_bink::FfmpegBink;
use crate::movie_frames::{
    GamePresents, LatestMovieFrame, SharedMovieFrames, load_splash_with_ffmpeg, publish_movie_frame,
};
use crate::raw_input::{HostInputHandle, RawInputFrame, feed_raw_input};

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
        install_winmain(
            app.world_mut(),
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
                        recomp_startup::winmain_host::ScreenSeams {
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
        if let Some(input) = os_input_frame(app.world()) {
            app.insert_resource(HostInputHandle(input));
        }
        if let Some(files) = shared_files(app.world()) {
            // IDA: 00430EF0 with the mask 00431440 passes: the game's own finder and archives, once the load has them. Any type, so a host read doesn't depend on which archive holds a sound.
            app.insert_resource(AudioAssetReader(Arc::new(move |path| {
                let mut file = files.open_core(path, 0, 0x2800, 0xFFFF)?;
                let mut bytes = vec![0; file.size() as usize];
                let read = file.read(&mut bytes);
                (read == bytes.len()).then_some(bytes)
            })));
        }
        if let Some(requests) = render_requests(app.world()) {
            app.insert_resource(GamePresents(requests));
        }
        app.insert_resource(frames);
        app.insert_resource(audio);
        app.init_resource::<LatestMovieFrame>();
        app.init_resource::<RawInputFrame>();
        app.insert_resource(WinMainChain(winmain_schedule()));
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

/// IDA: N/A one frame of the chain, and the exit a block asked for. Completion: 100%.
pub fn run_winmain_frame(world: &mut World) {
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
    run_main_menu_media_frame(world);
    // The chain's `end_frame` stops with the chain; the movie thread goes on.
    drain_movie_device_queue(world);
    let code = world.resource_mut::<WinMainProgress>().exit_code.take();
    if let Some(code) = code {
        let exit = if code == 0 {
            AppExit::Success
        } else {
            AppExit::from_code(code)
        };
        crate::render_exit::request_exit(world, exit).expect("the exit guard is installed");
    }
}

/// IDA: N/A `MoviePlayerGuard`: an exit stops and joins the movie thread first.
fn stop_winmain_threads(world: &mut World) {
    if world
        .get_resource::<Messages<AppExit>>()
        .is_some_and(|exits| !exits.is_empty())
    {
        winmain_shutdown(world);
    }
}
