//! The media of WinMain's menu path: 005B5F20's movie and music, which `Block::MainMenu` reaches through Tile_ReadXML; 0040F0AA's music, 0040F0BE's logo movie and 005B5C90's backdrop movie, `Block::IntroSequence`'s; then each frame's clock, poll, music and 0057F3BC's movie.
//!
//! 0040F0BE's `play_movie` blocks, as the original does, so a long `sMainMenuMovieIntro` shows only its last frame: the shipped one has one frame.

use std::rc::Rc;

use crate::tes::actor_process::ActorProcessGlobals;
use crate::tes::form_arms::ScriptRandom;
use crate::tes::movie_player::{MoviePlayer, play_menu_movie, play_movie};
use crate::tes::os_globals::{OsGlobalsData, lock_shared};
use crate::tes::os_sound_music::ScriptMusic;
use bevy_ecs::prelude::*;
use oblivion_core::crt::random_next;
use oblivion_core::logging::{Level, emit};

use crate::tes::winmain::{MenuMediaContext, WinMainProgress, driver_crt};

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

pub use super::game_frame::run_idle_frame;
use crate::tes::main_menu::main_menu_start_title_music;

/// IDA: N/A the seams the scripts' frame installs before 004402F0: the CRT probe, the scripts' draws, the cell music, the
/// intro block's pause and resume, and the cell load for a script's MoveTo of the player.
pub(super) fn install_script_hosts(world: &mut World, context: &MenuMediaContext) {
    if world.resource::<WinMainProgress>().script_hosts_installed {
        world
            .resource_mut::<ScriptMusic>()
            .refresh(&context.music, context.settings);
        return;
    }
    let crt = driver_crt(world);
    let music_crt = crt.clone();
    let outside = crt.clone();
    let probe = crt.clone();
    world.insert_non_send(crate::tes::form_arms::CrtProbe(Rc::new(move || {
        (probe.holdrand(), probe.draws())
    })));
    world.insert_non_send(crate::tes::actor_process::MainThreadRandom(Rc::new(
        std::cell::RefCell::new(crt.clone()),
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
    world.insert_resource(ScriptMusic::new(
        context.music.clone(),
        context.settings,
        move || music_crt.random_next(),
    ));
    world.insert_resource(crate::bs::model_loader::SynchronousModelLoad(
        super::load_gameplay_model,
    ));
    world.insert_resource(crate::bs::model_loader::GameplayModelServices(
        super::gameplay_model_services,
    ));
    world.insert_resource(crate::tes::player_move::CellLoadHost(
        super::with_grid_services,
    ));
    world
        .resource_mut::<WinMainProgress>()
        .script_hosts_installed = true;
}

#[cfg(test)]
mod tests {
    use crate::tes::os_globals::{
        HostWindow, OsSoundGlobals, ScriptedOsPlatform, scripted_input_settings,
    };
    use crate::tes::os_sound_music::MUSIC_TYPE_SPECIAL;
    use crate::tes::os_sound_music::{MusicTraceCall, ScriptedMusicDevice};
    use crate::tes::winmain::MusicSettings;
    use crate::tes::winmain::menu_media_context;
    use crate::tes::winmain::winmain_menu_media::*;

    #[test]
    fn script_services_keep_the_draw_source_and_sample_music_before_dispatch() {
        use crate::tes::actor_process::construct_actor_process_manager;
        use crate::tes::form_arms::crt_probe;
        use crate::tes::winmain::{
            SettingsSource, WinMainConfig, WinMainSeams, install_winmain, io_thread_holdrand,
        };
        use oblivion_core::crt::MsvcRand;

        let mut world = World::new();
        install_winmain(
            &mut world,
            WinMainConfig {
                data_directory: "Data".into(),
                settings: SettingsSource::Pinned("fixtures/config/Oblivion.ini".into()),
                seed: Some(416),
                frame_budget: None,
            },
            WinMainSeams::default(),
        );
        construct_actor_process_manager(&mut world);
        let mut frame = menu_media_context(&world).unwrap();
        frame.settings.music_enabled_b16180 = false;
        install_script_hosts(&mut world, &frame);
        let first = world.non_send::<ScriptRandom>().clone();
        let mut expected = MsvcRand::default();
        expected.srand(416);
        assert_eq!((first.0)(&mut world), expected.rand());
        assert_eq!(
            world
                .resource::<ActorProcessGlobals>()
                .random
                .first_use_b069c3,
            0
        );
        assert_eq!((first.1)(), expected.rand());
        assert_eq!(crt_probe(&world), Some((expected.holdrand(), 2)));
        assert_eq!(io_thread_holdrand(&world), Some(1));

        let mut sound = OsSoundGlobals::new(
            HostWindow(1),
            &mut scripted_input_settings(true),
            &mut ScriptedOsPlatform::default(),
        );
        sound.current_music_type_b0 = MUSIC_TYPE_SPECIAL;
        sound.music_path_1e4 = b"success.mp3".to_vec();
        world.insert_resource(OsGlobalsData {
            sound_24: Some(sound),
            ..Default::default()
        });
        frame.settings.music_enabled_b16180 = true;
        let music = world.resource::<ScriptMusic>().clone();
        music.cell_music(&mut world, 2, false, false);
        assert_eq!(
            world
                .resource::<OsGlobalsData>()
                .sound_24
                .as_ref()
                .unwrap()
                .current_music_type_b0,
            MUSIC_TYPE_SPECIAL
        );
        install_script_hosts(&mut world, &frame);
        let next = world.non_send::<ScriptRandom>();
        assert!(Rc::ptr_eq(&first.0, &next.0) && Rc::ptr_eq(&first.1, &next.1));
        let music = world.resource::<ScriptMusic>().clone();
        music.cell_music(&mut world, 2, false, false);
        assert_eq!(
            world
                .resource::<OsGlobalsData>()
                .sound_24
                .as_ref()
                .unwrap()
                .current_music_type_b0,
            2
        );
    }

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
