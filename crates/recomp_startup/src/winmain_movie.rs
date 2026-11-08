//! Phase 06: WinMain 0040EADB..0040EB11 after the activation replay (`winmain_window`): the Bink sound gate, the MoviePlayer
//! thread, the camera aliases and the pathing singletons. The classes are in `recomp_game::tes`. Runtime evidence:
//! P06-RUN-001, P06-RUN-002.

use recomp_game::tes::movie_player::{
    MoviePlayer, configure_bink_sound, start_async_movie_sequence,
};
use recomp_game::tes::pathing::{
    PathingPlatform, PathingSingletons, ensure_path_builder, ensure_pathing_world_node_map,
};

/// IDA: N/A what the window's two calls left in eax, for a caller's record: the original ignores both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MovieWindowResult {
    /// 0040EADB 00410310.
    pub bink_sound: i32,
    /// 0040EAEA 00410E40: ResumeThread's result.
    pub thread_resume: u32,
}

/// IDA: WinMain 0040EADB..0040EB11. 0040EADB 00410310; 0040EAE0..0040EAEA 00410E40(sIntroSequence 00B03084, ebp = 0, -1);
/// 0040EAEF..0040EB06 the two raw aliases of g_worldSceneGraph +0xDC, which `publish_camera_aliases` is, so that their place
/// in the order stays here; 0040EB0C 0067FCF0; 0040EB11 00683030. Completion: 100% of the order.
/// `publish_camera_aliases` is `initialize_scene_graphs::publish_camera_aliases` bound to the scene globals.
pub fn run_winmain_movie_window(
    player: &MoviePlayer,
    publish_camera_aliases: impl FnOnce(),
    pathing: &mut PathingSingletons,
    pathing_platform: &mut dyn PathingPlatform,
) -> MovieWindowResult {
    let bink_sound = configure_bink_sound(player);
    let thread_resume = start_async_movie_sequence(
        player,
        &player.services.settings.intro_sequence_b03084,
        false,
        -1,
    );
    publish_camera_aliases();
    ensure_pathing_world_node_map(pathing, pathing_platform);
    ensure_path_builder(pathing, pathing_platform);
    MovieWindowResult {
        bink_sound,
        thread_resume,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recomp_core::fixtures;
    use recomp_core::host_path::original_length;
    use recomp_game::tes::bink_library::ScriptedBink;
    use recomp_game::tes::interface_manager::InterfaceMovieReads;
    use recomp_game::tes::movie_player::{
        MOVIE_THREAD_CONTROL_WORD, MovieOsGlobals, MoviePlayerServices, MovieSettings,
        ScriptedMovieDevice, ScriptedMoviePlatform, movie_player_shutdown, movie_settings_from,
    };
    use recomp_game::tes::os_globals::{
        OsGlobalsData, ScriptedOsPlatform, Shared, lock_shared, scripted_input_settings,
    };
    use recomp_game::tes::pathing::RecordedPathingPlatform;
    use recomp_game::tes::settings::{SettingCollections, register_static_settings};
    use recomp_game::tes::window::SharedTimeInfo;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    fn rows(kind: &str) -> Vec<Vec<String>> {
        fixtures::rows("phase06")
            .into_iter()
            .filter_map(|row| row.strip_prefix("MOVIE|").map(str::to_owned))
            .map(|row| row.split('|').map(str::to_owned).collect::<Vec<_>>())
            .filter(|row| row[1] == kind)
            .collect()
    }

    /// The settings the evidence runs read, from the pinned INI (`fixtures/config/Oblivion.ini`).
    fn pinned_settings() -> MovieSettings {
        let mut collections = SettingCollections::default();
        register_static_settings(&mut collections);
        collections
            .ini
            .set_path(fixtures::root().join("config/Oblivion.ini"));
        assert!(collections.ini.open_ini());
        collections.ini.read_all();
        collections.ini.close();
        movie_settings_from(&collections.ini)
    }

    #[test]
    fn movie_settings_match_run_001() {
        let settings = pinned_settings();
        let row = &rows("SETTINGS")[0];
        assert_eq!(u8::from(settings.sound_enabled_b16178), 1);
        assert_eq!(row[2], "01");
        assert_eq!(
            String::from_utf8_lossy(&settings.intro_sequence_b03084),
            row[3]
        );
        assert_eq!(
            original_length(&row[3]),
            settings.intro_sequence_b03084.len()
        );
        assert_eq!(u8::from(settings.preload_intro_sequence_b030b4), 1);
        assert_eq!(row[4], "01");
        assert_eq!(settings.preload_size_limit_b030bc.to_string(), row[5]);
        assert_eq!(settings.intro_sequence_priority_b030c4.to_string(), row[6]);
        let wait: String = settings
            .static_screen_wait_time_b030ac
            .to_le_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(wait, row[7]);
    }

    /// The window's order and the two runs' values: 00410310's result, the thread's priority and flag, and the movies in the
    /// order the thread opened them. The camera aliases keep their place between the thread and the pathing calls.
    #[test]
    fn movie_window_matches_the_runs() {
        let bink = Arc::new(Mutex::new(ScriptedBink::default()));
        let settings = pinned_settings();
        let device = Arc::new(Mutex::new(ScriptedMovieDevice::default()));
        let mut ini = scripted_input_settings(true);
        let mut os = ScriptedOsPlatform::default();
        let sound = recomp_game::tes::os_globals::OsSoundGlobals::new(
            recomp_game::tes::os_globals::HostWindow(1),
            &mut ini,
            &mut os,
        );
        let globals = OsGlobalsData {
            sound_24: Some(sound),
            main_thread_id_10: ScriptedMoviePlatform::MAIN_THREAD_ID,
            ..OsGlobalsData::default()
        };
        let player = MoviePlayer::new(MoviePlayerServices {
            bink: bink.clone(),
            device: Some(device),
            platform: Arc::new(Mutex::new(ScriptedMoviePlatform::default())),
            os_platform: Arc::new(Mutex::new(ScriptedOsPlatform::default())),
            renderer_width: 1920,
            renderer_height: 1080,
            os_globals: Some(MovieOsGlobals::from(&globals)),
            time: SharedTimeInfo::default(),
            interface: InterfaceMovieReads::default(),
            video_directory: fixtures::data_dir().join("Video"),
            settings,
        });
        let mut pathing = PathingSingletons::default();
        let mut platform = RecordedPathingPlatform::default();
        let mut aliases_ran = false;
        let result =
            run_winmain_movie_window(&player, || aliases_ran = true, &mut pathing, &mut platform);
        assert!(aliases_ran);
        assert!(pathing.world_node_map_b3be00.is_some());
        assert!(pathing.path_builder_b3bf80.is_some());
        assert_eq!(result.thread_resume, 1, "A suspended thread resumes once");
        for row in rows("SOUND") {
            assert_eq!(result.bink_sound.to_string(), row[2], "{row:?}");
        }
        let started = rows("STARTED");
        assert_eq!(started.len(), 2);
        for row in started {
            assert_eq!(
                player
                    .globals
                    .thread_priority_b33438
                    .load(Ordering::Acquire)
                    .to_string(),
                row[2],
                "{row:?}"
            );
            assert_eq!(
                format!(
                    "{:02}",
                    player.globals.scale_to_width_b33427.load(Ordering::Acquire)
                ),
                row[3],
                "{row:?}"
            );
        }
        lock_shared(&player.globals.host_thread_owner)
            .clone()
            .unwrap()
            .wait();
        let opens = lock_shared(&bink).opens.clone();
        let open_rows = rows("OPEN");
        assert_eq!(open_rows.len(), 8, "both runs");
        for run in ["P06-RUN-001", "P06-RUN-002"] {
            let names: Vec<&String> = open_rows
                .iter()
                .filter(|row| row[0] == run)
                .map(|row| &row[3])
                .collect();
            let ours: Vec<String> = opens
                .iter()
                .map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
            assert_eq!(names, ours.iter().collect::<Vec<_>>(), "{run}");
        }
        assert!(lock_shared(&player.globals.thread_b33434).is_none());
        for row in rows("SEQUENCE_RET") {
            assert_eq!(
                format!(
                    "{:02}",
                    player.globals.skip_requested_b33426.load(Ordering::Acquire)
                ),
                row[2],
                "{row:?}"
            );
        }
        movie_player_shutdown(&player);
    }

    /// The word the row records is the one the module's arithmetic assumes; nothing sets it.
    #[test]
    fn thread_control_word_matches_run_002() {
        let row = rows("CONTROL");
        let thread = row.iter().find(|row| row[2] == "THREAD").unwrap();
        assert_eq!(format!("{MOVIE_THREAD_CONTROL_WORD:04X}"), thread[3]);
    }

    /// `Shared` keeps one input for both sides: the movie thread's clone sees the main thread's poll.
    #[test]
    fn input_is_shared_with_the_movie_view() {
        let settings = scripted_input_settings(true);
        let mut platform = ScriptedOsPlatform::default();
        let input = recomp_game::tes::os_globals::OsInputGlobals::new(
            recomp_game::tes::os_globals::HostInstance(0x0040_0000),
            &settings,
            &mut platform,
        )
        .unwrap();
        let globals = OsGlobalsData {
            input_20: Some(Shared::new(*input)),
            ..OsGlobalsData::default()
        };
        let view = MovieOsGlobals::from(&globals);
        let mut state = [0; 256];
        state[0x39] = 0x80;
        platform.keyboard_states.push_back(state);
        globals
            .input_20
            .as_ref()
            .unwrap()
            .lock()
            .poll_devices(0, &mut platform);
        assert_eq!(
            view.input_20.as_ref().unwrap().lock().keyboard_state_18f4[0x39],
            0x80
        );
    }
}
