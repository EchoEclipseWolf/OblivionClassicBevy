//! The media of WinMain's menu path: 005B5F20's movie and music, which `Block::MainMenu` reaches through Tile_ReadXML; 0040F0AA's music, 0040F0BE's logo movie and 005B5C90's backdrop movie, `Block::IntroSequence`'s; then each frame's clock, poll, music and 0057F3BC's movie.
//!
//! 0040F0BE's `play_movie` blocks, as the original does, so a long `sMainMenuMovieIntro` shows only its last frame: the shipped one has one frame.

use std::rc::Rc;

use bevy_ecs::prelude::*;
use recomp_core::logging::{Level, emit};
use recomp_game::tes::actor_process::{ActorProcessGlobals, random_next};
use recomp_game::tes::ai_conditions::{ConditionCaches, ScriptRandom};
use recomp_game::tes::ai_package::time_globals_advance;
use recomp_game::tes::ai_process::AiProcessGlobals;
use recomp_game::tes::interface_frame::top_stacked_menu_mode;
use recomp_game::tes::interface_manager::InterfaceManager;
use recomp_game::tes::movie_player::{MoviePlayer, play_menu_movie, play_movie};
use recomp_game::tes::os_globals::{OsGlobalsData, lock_shared};
use recomp_game::tes::os_sound_music::{
    MUSIC_TYPE_SPECIAL, MusicDevice, MusicSettings, SoundFrame,
};
use recomp_game::tes::script_vm::run_grid_scripts_frame;
use recomp_game::tes::tile_values::{TRAIT_USER0, tile_numeric_trait};
use recomp_game::tes::window::time_info_update;

use crate::winmain::{
    MenuMediaContext, WinMainProgress, driver_crt, is_menu_mode, menu_media_context,
    with_driver_interface,
};

/// IDA: A6CB58, the 12 bytes 005B5AD4..005B5AE5 copy to the stack: "Data\Music\".
const MUSIC_FOLDER: &[u8] = b"Data\\Music\\";

/// IDA: N/A where the menu's media path has reached.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MainMenuMediaStage {
    /// `Block::IntroSequence` hasn't finished.
    #[default]
    NotStarted,
    /// Past 0040F0D1: each frame's clock, poll, music and movie.
    Running,
    /// 0040F0DB: the player has a cell, so no menu. The wait, then nothing.
    NoMenu,
}

/// IDA: 00B3B3FC's two bytes, as the media path sets them, and where it has got to. Read by the screens for the backdrop's visibility.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MainMenuMedia {
    pub stage: MainMenuMediaStage,
    /// 00B3B408 (`dword_B3B3FC + 0xC`): the backdrop movie plays. 005B62A9 and 005B5D18 pick the backdrop tile's alpha from it.
    pub backdrop_movie_b3b408: u8,
    /// 00B3B409 (`dword_B3B3FC + 0xD`): set by 005B5F20's last store.
    pub menu_constructed_b3b409: u8,
    /// IDA: N/A a screen ran `main_menu_ctor_media` itself, from its 005B5F20.
    pub ctor_ran: bool,
    /// IDA: N/A a stop in 004402F0 has ended the idle frame's scripts.
    pub scripts_stopped: bool,
    /// IDA: N/A that stop's message, for the tests.
    pub scripts_stop: Option<&'static str>,
}

/// IDA: 005B5AC0 MainMenu_StartTitleMusic. 005B5AD4..005B5B2D "Data\Music\" + sMainMenuMusic on the stack, with no length check; 005B5B2F 00B3C0EC = 1; 005B5B36 g_osGlobals->sound; a non-null one: 006AB160(8, path, 0), then 006AB420 when it returned true. Completion: 100%; a name past the 0x108-byte stack buffer overruns it in the original and only makes a longer path here.
pub fn main_menu_start_title_music(
    world: &mut World,
    music_name: &[u8],
    settings: &MusicSettings,
    device: &mut dyn MusicDevice,
) -> bool {
    let mut path = MUSIC_FOLDER.to_vec();
    path.extend_from_slice(music_name);
    let mut os = world.resource_mut::<OsGlobalsData>();
    let Some(sound) = os.sound_24.as_mut() else {
        return false;
    };
    sound.main_menu_music_b3c0ec = 1;
    let loaded = sound.load_music_track(MUSIC_TYPE_SPECIAL, Some(&path), false, settings, device);
    if loaded {
        sound.start_music_playback(settings, device);
    }
    loaded
}

/// IDA: 005B5F20 MainMenu_ctor's media stores. 005B5F78..005B5F91 00B3B408 = current movie's +0x20 != 0, or 0 without one; 005B5FA0 while that's 0, 00410C40(sMainMenuMovie, 1) sets it; 005B5FB3 00B3C0EC = 1; 005B5FC1 OSSoundGlobals_StopSoundsMatchingFlags(3), whose map walk visits nothing while the host keeps no game sounds; with sound and no movie, or a movie in state 0 or 2: 006A8D50, then 005B5AC0; 00B3B409 = 1. Returns 00B3B408. Completion: 100% of these; the Menu constructor and the tile fields are `main_menu_constructed`'s.
pub fn main_menu_ctor_media(world: &mut World, context: &MenuMediaContext) -> bool {
    let Some(player) = world.get_resource::<MoviePlayer>().cloned() else {
        return false;
    };
    let current = lock_shared(&player.globals.current_b33428).clone();
    let mut backdrop = current.as_ref().is_some_and(|movie| movie.state() != 0);
    if !backdrop {
        backdrop = play_menu_movie(&player, &context.main_menu_movie_b03094, true);
    }
    let current = lock_shared(&player.globals.current_b33428).clone();
    let start = current
        .as_ref()
        .is_none_or(|movie| matches!(movie.state(), 0 | 2));
    let mut music = lock_shared(&context.music);
    let mut has_sound = false;
    if let Some(sound) = world.resource_mut::<OsGlobalsData>().sound_24.as_mut() {
        sound.main_menu_music_b3c0ec = 1;
        has_sound = true;
        if start {
            sound.resume_music_playback(&mut *music);
        }
    }
    if has_sound && start {
        main_menu_start_title_music(
            world,
            &context.main_menu_music_b14374,
            &context.settings,
            &mut *music,
        );
    }
    drop(music);
    let mut media = world.get_resource_or_init::<MainMenuMedia>();
    media.backdrop_movie_b3b408 = u8::from(backdrop);
    media.menu_constructed_b3b409 = 1;
    media.ctor_ran = true;
    backdrop
}

/// IDA: 005B5C90 MainMenu_BeginIntroFadeAndBackdrop's movie: 005B5CE4..005B5CF7 00B3B408 = current's +0x20 != 0, or 0 without one; while 0, 005B5D07 00410C40(sMainMenuMovie, 1) sets it. Completion: 100% of the movie; the tile traits around it (0FB1, 0FB2, 0FA1) are the screens', and 00B3B408 tells them which of 005B5D18's and 005B5D3A's values applies.
pub fn main_menu_begin_backdrop_movie(player: &MoviePlayer, movie_name: &str) -> bool {
    let current = lock_shared(&player.globals.current_b33428).clone();
    let mut backdrop = current.is_some_and(|movie| movie.state() != 0);
    if !backdrop {
        backdrop = play_menu_movie(player, movie_name, true);
    }
    backdrop
}

/// IDA: 0040F0AA MainMenu_StartTitleMusic and 0040F0B5..0040F0BE MoviePlayer_PlayMovie(sMainMenuMovieIntro, 1, 1, 0, 0, 0, 0), on the main thread. Returns whether the logo played. Completion: 100%.
pub fn main_menu_title_and_logo(world: &mut World, context: &MenuMediaContext) -> bool {
    let Some(player) = world.get_resource::<MoviePlayer>().cloned() else {
        return false;
    };
    {
        let mut music = lock_shared(&context.music);
        let started = main_menu_start_title_music(
            world,
            &context.main_menu_music_b14374,
            &context.settings,
            &mut *music,
        );
        emit(
            Level::Debug,
            &format!("MainMenu title music 0040F0AA: new track {started}"),
        );
    }
    play_movie(
        &player,
        &context.main_menu_movie_intro_b0308c,
        true,
        true,
        false,
        false,
        false,
        false,
    )
}

/// IDA: N/A a frame past 0040F0D1: 0040D800's clock, 0040D8CD poll, 0040DBB4 music, 0040DBDC scripts and 0057F3BC movie.
/// TODO: [ ] 0040D800's order holds for these calls, read at 0040D89F..0040D8CD and 0040DBB4; the render at 0057F3BC runs from a later call of it and isn't placed exactly.
pub fn run_main_menu_media_frame(world: &mut World) {
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
    {
        {
            let tick = context.clock.tick();
            let time_info_10 = {
                let mut time = context.time.lock();
                // 0040D89F..0040D8AB.
                time_info_update(&mut time, tick, context.time_multiplier_b06704);
                time.paused_ticks_10
            };
            // 0040DBAB, and 006AE89E's 00578FE0: no manager, cursor or root: 0; root trait 0xFAE == 2.0: 0057CF60; else 0.
            let menu_mode = is_menu_mode(world);
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
            let movie_state = lock_shared(&player.globals.current_b33428)
                .as_ref()
                .map(|movie| movie.state());
            // 006A8E80's draws, through 0047DF80 on the main thread.
            let music_draws = driver_crt(world);
            let mut os = world.resource_mut::<OsGlobalsData>();
            if let Some(input) = os.input_20.as_ref() {
                // 0040D8CD. The movie thread's order: the input, then the platform.
                let mut input = input.lock();
                let mut platform = lock_shared(&context.os_platform);
                input.poll_devices(time_info_10, &mut *platform);
            }
            if let Some(sound) = os.sound_24.as_mut() {
                // 0040DBB4 006AE860.
                sound.update_frame(
                    &SoundFrame {
                        sound_enabled_b16178: context.sound_enabled_b16178,
                        movie_state,
                        top_menu_mode,
                        menu_mode,
                        time_info_10,
                        player_has_cell: context.player_has_cell,
                    },
                    &context.settings,
                    &mut *lock_shared(&context.music),
                    &mut || music_draws.random_next(),
                );
            }
            // 0040DBDC 004402F0. A stop ends the scripts and nothing else: the menu's clock, music and movie go on.
            if !world.resource::<MainMenuMedia>().scripts_stopped {
                let frame_seconds = context.time.lock().frame_seconds_0c;
                let crt = driver_crt(world);
                let crt_for_music = crt.clone();
                let outside = crt.clone();
                let probe = crt.clone();
                world.insert_non_send(recomp_game::tes::ai_conditions::CrtProbe(Rc::new(
                    move || (probe.holdrand(), probe.draws()),
                )));
                world.insert_non_send(ScriptRandom(
                    Rc::new(move |world: &mut World| {
                        crt.with_world_flag(world, |world, random| {
                            let mut globals = world.resource_mut::<ActorProcessGlobals>();
                            random_next(0, &mut globals.random, random)
                        })
                    }),
                    Rc::new(move || outside.random_next()),
                ));
                // 0066EAF0's music for an interior: 006AB160 with no path, then ResumeMusic and StartMusicPlayback.
                let music_crt = crt_for_music.clone();
                let music_device = context.music.clone();
                let music_settings = context.settings;
                world.insert_non_send(recomp_game::tes::player_move::CellMusicHost(Rc::new(
                    move |world: &mut World, music_type, resume, start| {
                        let mut device = lock_shared(&music_device);
                        let mut random = || music_crt.random_next();
                        let mut os = world.resource_mut::<OsGlobalsData>();
                        let Some(sound) = os.sound_24.as_mut() else {
                            return false;
                        };
                        let loaded = sound.load_random_music_track(
                            music_type,
                            true,
                            &music_settings,
                            &mut *device,
                            &mut random,
                        );
                        if loaded && resume {
                            sound.resume_music_playback(&mut *device);
                        }
                        if loaded && start {
                            sound.start_music_playback(&music_settings, &mut *device);
                        }
                        loaded
                    },
                )));
                // 0066EAF0's cell load, for a script's MoveTo of the player.
                world.insert_non_send(recomp_game::tes::player_move::CellLoadHost(Rc::new(
                    |world: &mut World, body| crate::winmain::with_grid_services(world, body),
                )));
                // 0040DB9F, outside menu mode, with the rest of 0040DB7A..0040DBAA not ported: `docs/open-items.md`.
                let advanced = if menu_mode {
                    Ok(())
                } else {
                    time_globals_advance(world, frame_seconds)
                };
                // 0040DBD6 004F5DB0.
                if let Some(mut globals) = world.get_resource_mut::<AiProcessGlobals>() {
                    globals.caches = ConditionCaches::default();
                }
                // 0040DEA1 004424D0 outside menu mode, of which 00542F20's state: the scripts' frame calls between (0040DBEC..0040DE71: the process manager, the player, the grid) aren't ported, `docs/open-items.md`. In menu mode the frame reaches 004424D0 only past 0057AB70 and 00572E30(2), which the menus here don't.
                let sky_draws = driver_crt(world);
                let ran = advanced
                    .and_then(|()| run_grid_scripts_frame(world, frame_seconds))
                    .and_then(|()| {
                        if menu_mode {
                            return Ok(());
                        }
                        recomp_game::tes::sky_update::sky_update(world, &mut || {
                            Ok(sky_draws.random_next())
                        })
                    });
                if let Err(error) = ran {
                    emit(
                        Level::Error,
                        &format!("0040DBDC: the idle frame's scripts stopped: {}", error.0),
                    );
                    let mut media = world.resource_mut::<MainMenuMedia>();
                    media.scripts_stopped = true;
                    media.scripts_stop = Some(error.0);
                }
            }
            // 0057F3A5..0057F3BC's menu-owned movie presents from the render: `DriverInterfaceHost::render_interface`.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recomp_game::tes::os_globals::{
        HostWindow, OsSoundGlobals, ScriptedOsPlatform, scripted_input_settings,
    };
    use recomp_game::tes::os_sound_music::{MusicTraceCall, ScriptedMusicDevice};

    #[test]
    fn the_title_path_is_the_setting_after_the_folder() {
        let mut world = World::new();
        let mut settings = scripted_input_settings(true);
        let mut platform = ScriptedOsPlatform::default();
        world.insert_resource(OsGlobalsData {
            sound_24: Some(OsSoundGlobals::new(
                HostWindow(1),
                &mut settings,
                &mut platform,
            )),
            ..OsGlobalsData::default()
        });
        let music = MusicSettings {
            music_enabled_b16180: true,
            main_menu_music_volume_b16188: 0.6,
        };
        let mut device = ScriptedMusicDevice::default();
        assert!(main_menu_start_title_music(
            &mut world,
            b"Special\\TES4Title.mp3",
            &music,
            &mut device
        ));
        assert_eq!(
            device.trace[0],
            (
                0x006A_B2FB,
                MusicTraceCall::Access(b"Data\\Music\\Special\\TES4Title.mp3".to_vec())
            )
        );
        let os = world.resource::<OsGlobalsData>();
        assert_eq!(os.sound_24.as_ref().unwrap().main_menu_music_b3c0ec, 1);
    }

    #[test]
    fn no_sound_object_skips_the_store_and_the_load() {
        let mut world = World::new();
        world.insert_resource(OsGlobalsData::default());
        let music = MusicSettings {
            music_enabled_b16180: true,
            main_menu_music_volume_b16188: 0.6,
        };
        let mut device = ScriptedMusicDevice::default();
        assert!(!main_menu_start_title_music(
            &mut world,
            b"x.mp3",
            &music,
            &mut device
        ));
        assert!(device.trace.is_empty());
    }
}
