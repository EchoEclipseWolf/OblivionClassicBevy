//! The Bevy host: the replacement for the C++ Win32 and D3D9 host (D1). It opens the window and runs the WinMain driver of `recomp_startup` a frame at a time. Game code never depends on this crate.

pub mod audio;
pub mod dds_image;
pub mod ffmpeg_bink;
pub mod movie_frames;
pub mod raw_input;
pub mod render_exit;
pub mod screens;
pub mod winmain_plugin;
pub mod world_view;

use bevy::audio::AudioPlugin;
use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution};
use recomp_game::gamebryo::renderer::renderer_settings_from;
use recomp_game::tes::settings::{SettingCollections, register_static_settings};
use recomp_startup::winmain::{WinMainConfig, WinMainProgress, selected_paths};
use std::path::{Path, PathBuf};

/// IDA: 0040E7E7 / 00498340: the title 00A2FBB4 and the `iSize W` / `iSize H` size of the device window. The Win32 class,
/// style and bootstrap rectangle stay recorded calls in `recomp_startup::winmain_window`. Completion: 100% of the title and
/// size; `bFull Screen` and `iLocation` are unused here.
/// TODO: [ ] M6: `bFull Screen`, `iLocation`, focus, and the path of the user's own INI.
#[must_use]
pub fn window_from_ini(ini: &Path) -> Window {
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    collections.ini.set_path(ini.to_path_buf());
    // 0040E735: a failed open keeps the image values, as the driver's own load does.
    if collections.ini.open_ini() {
        collections.ini.read_all();
        collections.ini.close();
    }
    let settings = renderer_settings_from(&collections.ini);
    Window {
        title: "Oblivion".to_owned(),
        resolution: WindowResolution::new(settings.width as u32, settings.height as u32),
        present_mode: PresentMode::AutoVsync,
        // G5: KWin's title bar clamps a 1920x1080 client to 1052 on the virtual display. A recorded substitution: the original's windowed style (00CA0000) has a caption.
        decorations: false,
        ..default()
    }
}

/// IDA: N/A how a run is started: `main`'s arguments.
#[derive(Clone, Debug)]
pub struct HostOptions {
    pub winmain: WinMainConfig,
    /// `--exit-after-frames`: this many updates after the driver finishes.
    pub exit_after_frames: Option<u32>,
    /// `--menu-capture` and its siblings.
    pub capture: Option<screens::CapturePlan>,
    /// `--no-audio-device`: never open the machine's audio device.
    pub no_audio_device: bool,
    /// `--dump-movie-frames`: a folder for pictures of the movie frames.
    pub dump_movie_frames: Option<PathBuf>,
}

/// IDA: N/A whether this run may open the machine's audio device. Never on the virtual display, unless the test named its own ALSA config (a null or file device), so that a test can't reach the user's speakers. Completion: 100%.
#[must_use]
pub fn may_open_audio_device(no_audio_device: bool) -> bool {
    !no_audio_device
        && (std::env::var_os("OBLIVION_VIRTUAL_DISPLAY_STATE").is_none()
            || std::env::var_os("ALSA_CONFIG_PATH").is_some())
}

/// IDA: N/A host only: exits after this many updates past the driver's end, for tests on the virtual display.
#[derive(Resource)]
struct ExitAfter(u32);

/// IDA: N/A host only. Completion: 100%.
fn exit_after_frames(world: &mut World) {
    if !world
        .get_resource::<WinMainProgress>()
        .is_some_and(WinMainProgress::finished)
    {
        return;
    }
    let mut remaining = world.resource_mut::<ExitAfter>();
    let before = remaining.0;
    remaining.0 = before.saturating_sub(1);
    if before != 1 {
        return;
    }
    if let Some(latest) = world.get_resource::<movie_frames::LatestMovieFrame>() {
        println!(
            "movie frames {} last {}x{} visible {} presented {}",
            latest.generation, latest.size.x, latest.size.y, latest.visible, latest.presented
        );
    }
    if let Some(device) = world.get_resource::<audio::AudioDeviceReport>() {
        println!("audio device {device:?}");
    }
    let mut windows = world.query::<&Window>();
    for window in windows.iter(world) {
        println!(
            "window {} {}x{}",
            window.title,
            window.resolution.physical_width(),
            window.resolution.physical_height()
        );
    }
    render_exit::request_exit(world, AppExit::Success).expect("the exit guard is installed");
}

/// IDA: N/A the host app: the window, the WinMain driver, the screens, and the frames' sound, movies and input. Completion: through 0040EF23, and the menu path past it.
pub fn host_app(options: HostOptions) -> App {
    let HostOptions {
        winmain,
        exit_after_frames: exit_after,
        capture,
        no_audio_device,
        dump_movie_frames,
    } = options;
    let paths = selected_paths(&winmain.settings, &winmain.data_directory);
    let mut app = App::new();
    let open_audio = may_open_audio_device(no_audio_device);
    let plugins = DefaultPlugins.set(WindowPlugin {
        primary_window: Some(window_from_ini(&paths.selected_ini)),
        ..default()
    });
    if open_audio {
        app.add_plugins(plugins);
        app.init_resource::<audio::AudioDeviceReport>();
    } else {
        app.add_plugins(plugins.disable::<AudioPlugin>());
        app.insert_resource(audio::AudioDeviceReport::Disabled);
    }
    app.add_plugins(render_exit::RenderExitPlugin);
    if let Some(folder) = dump_movie_frames {
        app.insert_resource(movie_frames::DumpMovieFrames { folder, written: 0 });
    }
    app.add_plugins(winmain_plugin::WinMainPlugin { config: winmain });
    app.add_plugins(screens::ScreensPlugin { capture });
    if let Some(frames) = exit_after {
        app.insert_resource(ExitAfter(frames));
        app.add_systems(
            Update,
            exit_after_frames.after(winmain_plugin::run_winmain_frame),
        );
    }
    app
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P07 was captured at 1920x1080, which is what the pinned INI asks for.
    #[test]
    fn pinned_ini_asks_for_the_captured_size() {
        let ini = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/config/Oblivion.ini");
        let window = window_from_ini(&ini);
        assert_eq!(window.title, "Oblivion");
        assert_eq!(window.resolution.physical_width(), 1920);
        assert_eq!(window.resolution.physical_height(), 1080);
    }
}
