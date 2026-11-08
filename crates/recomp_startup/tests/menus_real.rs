//! The interface menus ported one at a time (2026-11-08), each built from its real XML in the shipped BSAs after Phase 07's
//! window (`run_winmain_interface_window`), as the original's creation functions build them. Host tests earn no
//! validation bonus: nothing here is compared with an original run, only with the exe's code and the XML.

use std::path::PathBuf;

use bevy_ecs::prelude::*;
use recomp_core::fixtures;
use recomp_core::reader::StreamError;
use recomp_game::bs::archive::{ArchiveManager, ArchiveSettings};
use recomp_game::bs::file::OpenedFile;
use recomp_game::bs::file_finder::{BsFileCallbacks, FileFinder, bs_file_open_core};
use recomp_game::bs::scene_graphs::{SceneGraphGlobals, SceneGraphSettings, construct_scene_graph};
use recomp_game::gamebryo::scene_update::NoSceneControllers;
use recomp_game::tes::game_settings::GameSettingCollection;
use recomp_game::tes::interface_manager::*;
use recomp_game::tes::settings::{SettingCollections, register_static_settings};
use recomp_game::tes::tile::*;
use recomp_game::tes::tile_values::*;
use recomp_startup::winmain_interface::run_winmain_interface_window;

#[derive(Default)]
struct TestHost {
    archives: Option<(FileFinder, ArchiveManager)>,
    warnings: Vec<String>,
    debug: Vec<String>,
    ui_sounds: Vec<u32>,
    tick: u32,
    saves: Vec<recomp_game::tes::save_files::SaveFile>,
}

impl InterfaceHost for TestHost {
    fn open_font_file(&mut self, path: &str) -> Option<OpenedFile> {
        let (finder, archives) = self.archives.as_mut()?;
        bs_file_open_core(finder, Some(archives), path, 0, 0x2800, 0x80)
    }

    fn read_tile_file(&mut self, path: &str) -> Option<Vec<u8>> {
        let (finder, archives) = self.archives.as_mut()?;
        let mut file = bs_file_open_core(finder, Some(archives), path, 0, 0x2800, 4)?;
        file.good().then(|| read_all(&mut file))
    }

    fn file_exists(&mut self, path: &str) -> bool {
        let Some((finder, archives)) = self.archives.as_mut() else {
            return false;
        };
        finder.find_file(path, 0, -1, Some(archives)).status != 0
    }

    fn texture_size(&mut self, path: &str) -> Option<[u32; 2]> {
        let (finder, archives) = self.archives.as_mut()?;
        let mut file = bs_file_open_core(finder, Some(archives), path, 0, 0x2800, 0xFFFF)?;
        let bytes = read_all(&mut file);
        let word = |offset: usize| {
            bytes
                .get(offset..offset + 4)
                .map(|field| u32::from_le_bytes(field.try_into().unwrap()))
        };
        (bytes.starts_with(b"DDS ")).then_some(())?;
        Some([word(0x10)?, word(0x0C)?])
    }

    fn movie_player_is_running(&mut self) -> bool {
        false
    }

    fn render_interface(&mut self, _argument: u32) {}

    fn mouse_axis(&mut self, _axis: i32) -> i32 {
        0
    }

    fn random_next(&mut self) -> i32 {
        0
    }

    fn loading_update_display(&mut self) {}

    fn warning(&mut self, text: &str) {
        self.warnings.push(text.to_owned());
    }

    fn output_debug_string(&mut self, text: &str) {
        self.debug.push(text.to_owned());
    }

    fn debug_log(&mut self, text: &str) {
        self.debug.push(text.to_owned());
    }

    fn tick_count(&mut self) -> u32 {
        self.tick += 16;
        self.tick
    }

    fn pause_game_sounds(&mut self) {}

    fn play_ui_sound(&mut self, sound: u32) {
        self.ui_sounds.push(sound);
    }

    fn save_file_list(&mut self) -> Vec<recomp_game::tes::save_files::SaveFile> {
        self.saves.clone()
    }

    fn create_unported_menu(&mut self, _menu_id: u32) -> Result<Option<MenuClass>, StreamError> {
        Err(StreamError("00587D20: this menu class is carried"))
    }
}

fn read_all(file: &mut OpenedFile) -> Vec<u8> {
    let mut bytes = vec![0; file.size() as usize];
    let _ = file.read(&mut bytes);
    bytes
}

fn real_archives() -> (FileFinder, ArchiveManager) {
    let data = fixtures::data_dir();
    assert!(
        data.join("Oblivion - Misc.bsa").exists(),
        "this test needs the game data: set OBLIVION_DATA_DIR ({data:?})"
    );
    let root: PathBuf = data.parent().unwrap().canonicalize().unwrap();
    let scratch = std::env::temp_dir().join(format!(
        "recomp_menus_real_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join("Plugins.txt"), "Oblivion.esm\n").unwrap();
    let mut callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new(&root, &mut callbacks);
    finder.add("Data");
    let archive_settings = ArchiveSettings {
        invalidate_older_files: false,
        retain_directory_strings: 2,
        ..ArchiveSettings::default()
    };
    let mut manager = ArchiveManager::new(&root, archive_settings);
    manager.initialize_from_settings(&finder, &scratch);
    let _ = std::fs::remove_dir_all(&scratch);
    (finder, manager)
}

fn settings() -> InterfaceSettings {
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    let scene = SceneGraphSettings {
        renderer_width: 1920,
        renderer_height: 1080,
        ..SceneGraphSettings::default()
    };
    let mut settings = interface_settings_from(&collections.ini, 1920, 1080, 8, scene);
    settings.full_screen = false;
    settings
}

/// Phase 07's window on the real data: the InterfaceManager, its menu root, cursor, strings and the LoadingMenu.
fn interface_window() -> (World, InterfaceSettings, TestHost) {
    let settings = settings();
    let mut world = World::new();
    let menu = construct_scene_graph(
        &mut world,
        b"Menu",
        true,
        None,
        &settings.scene,
        &mut NoSceneControllers,
    )
    .unwrap();
    let menu_3d = construct_scene_graph(
        &mut world,
        b"3DMenu",
        false,
        None,
        &settings.scene,
        &mut NoSceneControllers,
    )
    .unwrap();
    world.insert_resource(SceneGraphGlobals {
        menu: Some(menu),
        menu_3d: Some(menu_3d),
        ..SceneGraphGlobals::default()
    });
    world.insert_resource(InterfaceMovieReads::default());
    world.insert_resource(GameSettingCollection::default());
    let mut host = TestHost {
        archives: Some(real_archives()),
        tick: 1000,
        ..TestHost::default()
    };
    run_winmain_interface_window(&mut world, &settings, &mut host, &mut NoSceneControllers)
        .unwrap();
    (world, settings, host)
}

fn with_cx<R>(
    world: &mut World,
    settings: &InterfaceSettings,
    host: &mut TestHost,
    body: impl FnOnce(&mut InterfaceCx) -> R,
) -> R {
    with_interface_cx(world, settings, host, &mut NoSceneControllers, body)
}

fn text(cx: &InterfaceCx, tile: TileId, trait_id: u16) -> String {
    String::from_utf8_lossy(&tile_string_trait(cx, tile, trait_id).unwrap_or_default()).into_owned()
}

// 005A8E30: hud_subtitle_menu.xml's menu (1010) with its two fields; field 1's user9 is its y, both strings " ", the menu
// enabled (visible 1, fading in), its depth the cursor's less 10 (00584980, from Menu_Enable); the ctor's state 1.
#[test]
fn the_hud_subtitle_menu_builds_from_its_xml() {
    use recomp_game::tes::hud_subtitle_menu::*;
    let (mut world, settings, mut host) = interface_window();
    with_cx(&mut world, &settings, &mut host, |cx| {
        let created = hud_subtitle_menu_create(cx).unwrap().expect("the menu");
        assert_eq!(
            cx.engine.tile_menu_by_id(MENU_ID_HUD_SUBTITLE),
            Some(created)
        );
        let menu = cx.engine.containing_menu(created).unwrap();
        let MenuClass::HudSubtitle(subtitle) = cx.engine.menu(menu).class.clone() else {
            panic!("a HUDSubtitleMenu");
        };
        assert_eq!(subtitle.subtitle_state_38, SUBTITLE_STATE_IDLE);
        assert_eq!(subtitle.subtitle_seconds_3c, 0.0);
        assert!(subtitle.notices_2c.is_empty());
        let text_tile = subtitle.subtitle_text_28.expect("field 1");
        let notice = subtitle.notice_text_34.expect("field 2");
        assert_eq!(text(cx, text_tile, TRAIT_STRING), " ");
        assert_eq!(text(cx, notice, TRAIT_STRING), " ");
        assert_eq!(
            tile_numeric_trait(cx, text_tile, TRAIT_USER9),
            tile_numeric_trait(cx, text_tile, TRAIT_Y),
            "user9 is the text's y"
        );
        assert_eq!(tile_numeric_trait(cx, text_tile, TRAIT_ID), 1.0);
        assert_eq!(tile_numeric_trait(cx, notice, TRAIT_ID), 2.0);
        assert_eq!(tile_numeric_trait(cx, created, TRAIT_VISIBLE), 1.0);
        assert_eq!(cx.engine.menu(menu).state_24, 8, "Menu_Enable(0) fades in");
        let cursor = cx.world.resource::<InterfaceManager>().cursor_01c.unwrap();
        assert_eq!(
            tile_numeric_trait(cx, created, TRAIT_DEPTH),
            tile_numeric_trait(cx, cursor, TRAIT_DEPTH) - 10.0,
            "00584980 puts HUDSubtitle 10 under the cursor"
        );
        // A second create deletes the first menu and builds another.
        let again = hud_subtitle_menu_create(cx)
            .unwrap()
            .expect("the menu again");
        assert_ne!(again, created);
        assert_eq!(cx.engine.tile_menu_by_id(MENU_ID_HUD_SUBTITLE), Some(again));
    });
    assert!(host.warnings.is_empty(), "{:?}", host.warnings);
    assert!(
        !host
            .debug
            .iter()
            .any(|line| line.contains("Creation Failed")),
        "{:?}",
        host.debug
    );
}

/// A player for the menus that read g_thePlayer (00B333C4), and the menu root's user0 at 2.0, as a running game holds it.
fn with_player_in_game(world: &mut World, cx_settings: &InterfaceSettings, host: &mut TestHost) {
    use recomp_game::tes::player::{PlayerCharacter, PlayerGlobals};
    let player = world
        .spawn((
            PlayerCharacter {
                pc_sleep_hours_590: 5,
                ..PlayerCharacter::default()
            },
            recomp_game::tes::actor::Actor::default(),
        ))
        .id();
    world
        .get_resource_or_init::<PlayerGlobals>()
        .the_player_b333c4 = Some(player);
    with_cx(world, cx_settings, host, |cx| {
        let root = cx
            .world
            .resource::<InterfaceManager>()
            .menu_root_068
            .unwrap();
        tile_set_numeric_trait(cx, root, TRAIT_USER0, 2.0).unwrap();
    });
}

// 0057B420: sleep_wait_menu.xml's menu (1012) with its eight fields (field 4 by the scroll prefab's user6), the slider's
// range, the player's hours in 00B3B730; waiting puts sWait on field 5 and the root's user0 at 1, sleeping at 2; with no
// TimeGlobals the hour is 12.0, the day 1.0 (Morndas) and the date Last Seed 17, 3E427 (00402B80..00402E50's defaults).
#[test]
fn the_sleep_wait_menu_builds_from_its_xml() {
    use recomp_game::tes::sleep_wait_menu::*;
    let (mut world, settings, mut host) = interface_window();
    with_cx(&mut world, &settings, &mut host, |cx| {
        assert!(
            !sleep_wait_menu_create(cx, false).unwrap(),
            "the root's user0 isn't 2.0 at the main menu"
        );
    });
    with_player_in_game(&mut world, &settings, &mut host);
    for sleeping in [false, true] {
        with_cx(&mut world, &settings, &mut host, |cx| {
            assert!(sleep_wait_menu_create(cx, sleeping).unwrap());
            let tile = cx.engine.tile_menu_by_id(MENU_ID_SLEEP_WAIT).unwrap();
            let menu = cx.engine.containing_menu(tile).unwrap();
            let MenuClass::SleepWait(state) = cx.engine.menu(menu).class.clone() else {
                panic!("a SleepWaitMenu");
            };
            assert!(
                state.tiles_28.iter().all(Option::is_some),
                "all eight fields"
            );
            for (id, name) in [
                (3, "sleep_scroll"),
                (5, "sleep_button_sleep_wait"),
                (6, "sleep_button_cancel"),
                (8, "game_info_1"),
                (9, "game_info_2"),
                (10, "sleep_scroll_disable"),
                (11, "sleep_scroll_marker"),
            ] {
                let field = state.field(id).unwrap();
                assert_eq!(cx.engine.tile(field).name_08, name.as_bytes(), "field {id}");
                assert_eq!(tile_numeric_trait(cx, field, TRAIT_ID), id as f32);
            }
            let marker = state.field(4).unwrap();
            assert_eq!(
                tile_numeric_trait(cx, marker, TRAIT_ID),
                4.0,
                "the prefab's user6"
            );
            assert_eq!(tile_numeric_trait(cx, marker, TRAIT_USER9), 0.0);
            let scroll = state.field(3).unwrap();
            assert_eq!(tile_numeric_trait(cx, scroll, TRAIT_USER5), 0.0);
            assert_eq!(tile_numeric_trait(cx, scroll, TRAIT_USER1), 1.0);
            assert_eq!(tile_numeric_trait(cx, scroll, TRAIT_USER2), 24.0);
            assert_eq!(tile_numeric_trait(cx, scroll, TRAIT_USER4), 6.0);
            assert_eq!(
                tile_numeric_trait(cx, tile, TRAIT_USER0),
                if sleeping { 2.0 } else { 1.0 }
            );
            assert_eq!(state.sleeping_4c, u8::from(sleeping));
            if !sleeping {
                assert_eq!(text(cx, state.field(5).unwrap(), TRAIT_USER0), "Wait");
            }
            assert_eq!(
                text(cx, state.field(8).unwrap(), TRAIT_STRING),
                "Morndas 12:00 pm"
            );
            assert_eq!(
                text(cx, state.field(9).unwrap(), TRAIT_STRING),
                "Last Seed 17, 3E427"
            );
            assert_eq!(tile_numeric_trait(cx, tile, TRAIT_VISIBLE), 1.0);
            assert_eq!(cx.engine.menu(menu).state_24, 8, "Menu_Enable(0) fades in");
            let statics = cx.world.resource::<SleepWaitStatics>().clone();
            assert_eq!(statics.hours_b3b730, 5, "the player's +0x590");
            assert_eq!(statics.last_hours_b14778, 1);
            assert_eq!(statics.resting_b3b728, 0);
        });
    }
    assert_eq!(host.ui_sounds, [0xB, 0xB]);
    assert!(host.warnings.is_empty(), "{:?}", host.warnings);
}

// 005BDA90: pause_menu.xml's menu (1013) with its five buttons. A living player at no menu can save, so field 4 keeps its
// target; dead (+0xB0 2), it can't, and field 3's user1 is 1; with Load first on the mode stack field 5's target is 1 and
// 00452330 refuses as well. Each create deletes the last menu and plays UI sound 2.
#[test]
fn the_pause_menu_builds_from_its_xml() {
    use recomp_game::tes::actor::Actor;
    use recomp_game::tes::interface_frame::TRAIT_TARGET;
    use recomp_game::tes::pause_menu::*;
    use recomp_game::tes::player::PlayerGlobals;
    let (mut world, settings, mut host) = interface_window();
    with_cx(&mut world, &settings, &mut host, |cx| {
        assert!(
            save_load_can_save_now(cx).is_err(),
            "00452336 with no player"
        );
    });
    with_player_in_game(&mut world, &settings, &mut host);
    let player = world.resource::<PlayerGlobals>().the_player_b333c4.unwrap();
    // (dead state, stack slot 0) -> (field 3's user1, field 4's target, field 5's target).
    for (dead_state, first_mode, expected) in [
        (0, 0, (false, false, false)),
        (2, 0, (true, true, false)),
        (0, 1038, (false, true, true)),
    ] {
        world.get_mut::<Actor>(player).unwrap().dead_state_b0 = dead_state;
        world.resource_mut::<InterfaceManager>().menu_mode_stack_0e0[0] = first_mode;
        with_cx(&mut world, &settings, &mut host, |cx| {
            assert!(pause_menu_create(cx).unwrap());
            let tile = cx.engine.tile_menu_by_id(MENU_ID_PAUSE).unwrap();
            let menu = cx.engine.containing_menu(tile).unwrap();
            let MenuClass::Pause(state) = cx.engine.menu(menu).class.clone() else {
                panic!("a PauseMenu");
            };
            for (id, name) in [
                (3, "pause_return_button"),
                (4, "pause_save_button"),
                (5, "pause_load_button"),
                (7, "pause_options_button"),
                (8, "pause_quit_button"),
            ] {
                let field = state.field(id).unwrap();
                assert_eq!(cx.engine.tile(field).name_08, name.as_bytes(), "field {id}");
            }
            let user1 = tile_numeric_trait(cx, state.field(3).unwrap(), TRAIT_USER1) == 1.0;
            let save = tile_numeric_trait(cx, state.field(4).unwrap(), TRAIT_TARGET) == 1.0;
            let load = tile_numeric_trait(cx, state.field(5).unwrap(), TRAIT_TARGET) == 1.0;
            assert_eq!(
                (user1, save, load),
                expected,
                "dead {dead_state}, first mode {first_mode}"
            );
            assert_eq!(tile_numeric_trait(cx, tile, TRAIT_VISIBLE), 1.0);
            assert_eq!(cx.engine.menu(menu).state_24, 8, "Menu_Enable(0) fades in");
            // A menu fading in under the root (the one just made, whose stacking type isn't 6008) holds the next create off.
            assert!(!pause_menu_create(cx).unwrap(), "a menu is fading");
            assert_eq!(cx.engine.tile_menu_by_id(MENU_ID_PAUSE), Some(tile));
        });
        world.resource_mut::<InterfaceManager>().menu_mode_stack_0e0[0] = 0;
        // Settle the fade so the next create isn't held off.
        with_cx(&mut world, &settings, &mut host, |cx| {
            let tile = cx.engine.tile_menu_by_id(MENU_ID_PAUSE).unwrap();
            let menu = cx.engine.containing_menu(tile).unwrap();
            cx.engine.menu_mut(menu).state_24 = 1;
        });
    }
    assert_eq!(host.ui_sounds, [2, 2, 2]);
    assert!(host.warnings.is_empty(), "{:?}", host.warnings);
}

// 005BD680: options_menu.xml's menu (1015) with its six buttons; the root's user0 is 1 at the main menu's stack and 2 with
// a Pause below on the mode stack, while a Pause above Options doesn't count. Each case has its own window: a create
// deletes the last Options menu first, and that removes its mode from the stack (005853F0's 0057CFE0).
#[test]
fn the_options_menu_builds_from_its_xml() {
    use recomp_game::tes::options_menu::*;
    for (stack, user0) in [
        ([1044, 0, 0], 1.0),
        ([1013, 1015, 0], 2.0),
        ([1015, 1013, 0], 1.0),
    ] {
        let (mut world, settings, mut host) = interface_window();
        world.resource_mut::<InterfaceManager>().menu_mode_stack_0e0[..3].copy_from_slice(&stack);
        with_cx(&mut world, &settings, &mut host, |cx| {
            assert!(options_menu_create(cx).unwrap());
            let tile = cx.engine.tile_menu_by_id(MENU_ID_OPTIONS).unwrap();
            let menu = cx.engine.containing_menu(tile).unwrap();
            let MenuClass::Options(state) = cx.engine.menu(menu).class.clone() else {
                panic!("an OptionsMenu");
            };
            for (id, name) in [
                (2, "options_return_button"),
                (5, "options_gameplay_button"),
                (6, "options_video_button"),
                (7, "options_audio_button"),
                (8, "options_controls_button"),
                (9, "options_downloads_button"),
            ] {
                let field = state.field(id).unwrap();
                assert_eq!(cx.engine.tile(field).name_08, name.as_bytes(), "field {id}");
            }
            assert_eq!(
                tile_numeric_trait(cx, tile, TRAIT_USER0),
                user0,
                "{stack:?}"
            );
            assert_eq!(tile_numeric_trait(cx, tile, TRAIT_VISIBLE), 1.0);
            assert_eq!(cx.engine.menu(menu).state_24, 8, "Menu_Enable(0) fades in");
        });
        assert!(host.warnings.is_empty(), "{:?}", host.warnings);
    }
}

// 005C05D0: quantity_menu.xml's menu (1016) with its six fields (field 4 by the scroll prefab's user6); the count on +0x40
// and +0x48 and the slider's range: 1..count, a step of count / 4 but at least 1, the marker at 0.
#[test]
fn the_quantity_menu_builds_from_its_xml() {
    use recomp_game::tes::quantity_menu::*;
    let (mut world, settings, mut host) = interface_window();
    for (count, step) in [(27, 6.0), (3, 1.0), (-5, 1.0)] {
        with_cx(&mut world, &settings, &mut host, |cx| {
            let root = cx
                .world
                .resource::<InterfaceManager>()
                .menu_root_068
                .unwrap();
            assert!(quantity_menu_create(cx, 0x33, Some(root), count, None).unwrap());
            let tile = cx.engine.tile_menu_by_id(MENU_ID_QUANTITY).unwrap();
            let menu = cx.engine.containing_menu(tile).unwrap();
            let MenuClass::Quantity(state) = cx.engine.menu(menu).class.clone() else {
                panic!("a QuantityMenu");
            };
            for (id, name) in [
                (1, "quantity_background"),
                (2, "quantity_display_text"),
                (3, "quantity_scroll"),
                (6, "quantity_button_okay"),
                (7, "quantity_button_cancel"),
            ] {
                let field = state.field(id).unwrap();
                assert_eq!(cx.engine.tile(field).name_08, name.as_bytes(), "field {id}");
            }
            assert_eq!(
                tile_numeric_trait(cx, state.field(4).unwrap(), TRAIT_ID),
                4.0
            );
            assert_eq!(state.max_count_40, count);
            assert_eq!(state.count_48, Some(count));
            assert_eq!(state.return_click_id_44, Some(0x33));
            assert_eq!(state.return_click_tile_4c, Some(root));
            assert_eq!(state.stack_reference_50, None);
            assert_eq!(state.drag_offset_54, 0.0);
            let scroll = state.field(3).unwrap();
            assert_eq!(tile_numeric_trait(cx, scroll, TRAIT_USER1), 1.0);
            assert_eq!(tile_numeric_trait(cx, scroll, TRAIT_USER2), count as f32);
            assert_eq!(
                tile_numeric_trait(cx, scroll, TRAIT_USER4),
                step,
                "count {count}"
            );
            assert_eq!(tile_numeric_trait(cx, scroll, TRAIT_USER5), 0.0);
            assert_eq!(cx.engine.menu(menu).state_24, 8, "Menu_Enable(0) fades in");
        });
    }
    assert!(host.warnings.is_empty(), "{:?}", host.warnings);
}

// 00595380: audio_menu.xml's menu (1017) with its eleven fields; each slider 0..100 by 25, at its volume's percent through
// __ftol2 (truncated), then moved back to 0 as the create leaves user5. With no sound object the create stops.
#[test]
fn the_audio_menu_builds_from_its_xml() {
    use recomp_game::tes::audio_menu::*;
    use recomp_game::tes::os_globals::{
        HostWindow, OsGlobalsData, OsSoundGlobals, ScriptedOsPlatform, scripted_input_settings,
    };
    let (mut world, settings, mut host) = interface_window();
    with_cx(&mut world, &settings, &mut host, |cx| {
        assert!(
            audio_menu_create(cx).is_err(),
            "00595499 with no sound object"
        );
    });
    let mut sound = OsSoundGlobals::new(
        HostWindow(1),
        &mut scripted_input_settings(true),
        &mut ScriptedOsPlatform::default(),
    );
    sound.master_volume_b8 = 0.8;
    sound.music_volume2_2f8 = 0.123;
    sound.foot_volume_bc = 1.0;
    sound.voice_volume_c0 = 0.0;
    sound.effects_volume_c4 = 0.999;
    world.get_resource_or_init::<OsGlobalsData>().sound_24 = Some(sound);
    with_cx(&mut world, &settings, &mut host, |cx| {
        assert!(audio_menu_create(cx).unwrap());
        let tile = cx.engine.tile_menu_by_id(MENU_ID_AUDIO).unwrap();
        let menu = cx.engine.containing_menu(tile).unwrap();
        let MenuClass::Audio(state) = cx.engine.menu(menu).class.clone() else {
            panic!("an AudioMenu");
        };
        for (id, name) in [
            (1, "audio_master_slider"),
            (3, "audio_voice_slider"),
            (5, "audio_effect_slider"),
            (7, "audio_foot_slider"),
            (9, "audio_music_slider"),
            (12, "audio_return_button"),
        ] {
            let field = state.field(id).unwrap();
            assert_eq!(cx.engine.tile(field).name_08, name.as_bytes(), "field {id}");
        }
        for id in [2, 4, 6, 8, 10] {
            assert_eq!(
                tile_numeric_trait(cx, state.field(id).unwrap(), TRAIT_ID),
                id as f32
            );
        }
        for (id, percent) in [(1, 80.0), (3, 0.0), (5, 99.0), (7, 100.0), (9, 12.0)] {
            let slider = state.field(id).unwrap();
            assert_eq!(
                tile_numeric_trait(cx, slider, TRAIT_USER7),
                percent,
                "field {id}'s value (user7, the marker's _value_scrolled_to)"
            );
            assert_eq!(tile_numeric_trait(cx, slider, TRAIT_USER1), 0.0);
            assert_eq!(tile_numeric_trait(cx, slider, TRAIT_USER2), 100.0);
            assert_eq!(tile_numeric_trait(cx, slider, TRAIT_USER4), 25.0);
            assert_eq!(tile_numeric_trait(cx, slider, TRAIT_USER5), 0.0);
        }
        assert_eq!(cx.engine.menu(menu).state_24, 8, "Menu_Enable(0) fades in");
    });
    assert!(host.warnings.is_empty(), "{:?}", host.warnings);
}

// 005A3540: gameplay_menu.xml's menu (1020) with its nine fields; the difficulty slider from fDifficultyMinValue (-1) to
// fDifficultyMaxValue (1) with steps of 0.02 and 0.5 and the player's +0x734 as its value; each button On or Off by its
// INI flag (all six are 1 in the static defaults, bCrossHair turned off here).
#[test]
fn the_gameplay_menu_builds_from_its_xml() {
    use recomp_game::tes::gameplay_menu::*;
    use recomp_game::tes::player::{PlayerCharacter, PlayerGlobals};
    let (mut world, mut settings, mut host) = interface_window();
    settings.cross_hair_b13210 = false;
    with_cx(&mut world, &settings, &mut host, |cx| {
        assert!(gameplay_menu_create(cx).is_err(), "005A36D6 with no player");
    });
    with_player_in_game(&mut world, &settings, &mut host);
    let player = world.resource::<PlayerGlobals>().the_player_b333c4.unwrap();
    world
        .get_mut::<PlayerCharacter>(player)
        .unwrap()
        .game_difficulty_level_734 = 0.25;
    with_cx(&mut world, &settings, &mut host, |cx| {
        assert!(gameplay_menu_create(cx).unwrap());
        let tile = cx.engine.tile_menu_by_id(MENU_ID_GAMEPLAY).unwrap();
        let menu = cx.engine.containing_menu(tile).unwrap();
        let MenuClass::Gameplay(state) = cx.engine.menu(menu).class.clone() else {
            panic!("a GameplayMenu");
        };
        let names = [
            (1, "gameplay_difficulty_slider"),
            (3, "gameplay_general_subtitles_button"),
            (4, "gameplay_dialogue_subtitles_button"),
            (5, "gameplay_crosshair_button"),
            (6, "gameplay_rest_save_button"),
            (7, "gameplay_wait_save_button"),
            (8, "gameplay_travel_save_button"),
            (10, "gameplay_return_button"),
        ];
        for (id, name) in names {
            let field = state.field(id).unwrap();
            assert_eq!(cx.engine.tile(field).name_08, name.as_bytes(), "field {id}");
        }
        assert_eq!(
            tile_numeric_trait(cx, state.field(2).unwrap(), TRAIT_ID),
            2.0
        );
        let slider = state.field(1).unwrap();
        assert_eq!(tile_numeric_trait(cx, slider, TRAIT_USER1), -1.0);
        assert_eq!(tile_numeric_trait(cx, slider, TRAIT_USER2), 1.0);
        assert_eq!(tile_numeric_trait(cx, slider, TRAIT_USER3), 2.0f32 / 100.0);
        assert_eq!(tile_numeric_trait(cx, slider, TRAIT_USER4), 0.5);
        assert_eq!(tile_numeric_trait(cx, slider, TRAIT_USER5), 0.0);
        for (id, expected) in [
            (3, "On"),
            (4, "On"),
            (5, "Off"),
            (6, "On"),
            (7, "On"),
            (8, "On"),
        ] {
            assert_eq!(
                text(cx, state.field(id).unwrap(), TRAIT_STRING),
                expected,
                "field {id}"
            );
        }
        assert_eq!(cx.engine.menu(menu).state_24, 8, "Menu_Enable(0) fades in");
    });
    assert!(host.warnings.is_empty(), "{:?}", host.warnings);
}

// 0059BA40: controls_menu.xml's menu (1021) with its thirteen fields. With a keyboard the device is 0 and no message
// shows; with no device the three tries end at 2 (the joystick) and sNoDevice shows. The sensitivity slider's value is
// (0.0025 - 0.0005) / 0.0095 * 100 in floats, then 0; invert Off; no joystick puts the swap button's target at 1; the 20
// shown controls get rows with ids 14..30 and 40..42, their row numbers, names and the root's height and colours.
#[test]
fn the_controls_menu_builds_from_its_xml() {
    use recomp_game::tes::controls_menu::*;
    use recomp_game::tes::game_settings_table::GAME_SETTING_DEFINITIONS;
    use recomp_game::tes::interface_frame::TRAIT_TARGET;
    use recomp_game::tes::os_globals::{
        HostInstance, InputDevice, OsGlobalsData, OsInputGlobals, ScriptedOsPlatform, Shared,
        scripted_input_settings,
    };
    for keyboard in [true, false] {
        let (mut world, mut settings, mut host) = interface_window();
        settings.mouse_sensitivity_b14ee8 = 0.0025;
        world.insert_resource(GameSettingCollection::new(GAME_SETTING_DEFINITIONS));
        with_cx(&mut world, &settings, &mut host, |cx| {
            assert!(controls_menu_create(cx).is_err(), "no input object");
        });
        let mut input = OsInputGlobals::new(
            HostInstance(1),
            &scripted_input_settings(true),
            &mut ScriptedOsPlatform::default(),
        )
        .unwrap();
        input.keyboard_0028 = keyboard.then_some(InputDevice(1));
        input.mouse_002c = None;
        input.joystick_count_18f0 = 0;
        world.get_resource_or_init::<OsGlobalsData>().input_20 = Some(Shared::new(*input));
        with_cx(&mut world, &settings, &mut host, |cx| {
            assert!(controls_menu_create(cx).unwrap());
            let tile = cx.engine.tile_menu_by_id(MENU_ID_CONTROLS).unwrap();
            let menu = cx.engine.containing_menu(tile).unwrap();
            let MenuClass::Controls(state) = cx.engine.menu(menu).class.clone() else {
                panic!("a ControlsMenu");
            };
            assert_eq!(state.device_5c, if keyboard { 0 } else { 2 });
            assert_eq!(
                cx.engine.tile_menu_by_id(1001).is_some(),
                !keyboard,
                "sNoDevice's message"
            );
            for (id, name) in [
                (1, "controls_return_button"),
                (2, "controls_vertical_scroll"),
                (4, "controls_list_pane"),
                (5, "controls_sensitivity_slider"),
                (7, "controls_invert_y_button"),
                (8, "controls_swap_button"),
                (9, "controls_more_button"),
                (10, "controls_defaults_button"),
                (11, "controls_instructions"),
                (12, "controls_device_toggle_left"),
                (13, "controls_device_toggle_right"),
            ] {
                let field = state.field(id).unwrap();
                assert_eq!(cx.engine.tile(field).name_08, name.as_bytes(), "field {id}");
            }
            assert_eq!(
                tile_numeric_trait(cx, state.field(3).unwrap(), TRAIT_ID),
                3.0
            );
            assert_eq!(
                tile_numeric_trait(cx, state.field(6).unwrap(), TRAIT_ID),
                6.0
            );
            assert_eq!(
                tile_numeric_trait(cx, state.field(5).unwrap(), TRAIT_USER5),
                0.0
            );
            assert_eq!(text(cx, state.field(7).unwrap(), TRAIT_STRING), "Off");
            assert_eq!(
                tile_numeric_trait(cx, state.field(8).unwrap(), TRAIT_TARGET),
                1.0
            );
            let pane = state.field(4).unwrap();
            // The pane's list from its head holds the newest row first; reversed, the rows are in creation order.
            let rows: Vec<TileId> = cx
                .engine
                .tile(pane)
                .children_30
                .iter()
                .rev()
                .copied()
                .filter(|&child| tile_numeric_trait(cx, child, TRAIT_ID) >= 14.0)
                .collect();
            let ids: Vec<f32> = rows
                .iter()
                .map(|&row| tile_numeric_trait(cx, row, TRAIT_ID))
                .collect();
            let mut expected: Vec<f32> = (14..=30).map(|id| id as f32).collect();
            expected.extend([40.0, 41.0, 42.0]);
            assert_eq!(ids, expected);
            for (number, &row) in rows.iter().enumerate() {
                assert_eq!(tile_numeric_trait(cx, row, TRAIT_USER0), number as f32);
                assert_eq!(
                    tile_numeric_trait(cx, row, TRAIT_HEIGHT),
                    tile_numeric_trait(cx, tile, TRAIT_USER1)
                );
                for color in [TRAIT_RED, TRAIT_GREEN, TRAIT_BLUE] {
                    assert_eq!(
                        tile_numeric_trait(cx, row, color),
                        tile_numeric_trait(cx, tile, color)
                    );
                }
            }
            assert_eq!(text(cx, rows[0], TRAIT_USER1), "Forward");
            assert_eq!(text(cx, rows[1], TRAIT_USER1), "Back");
            assert_eq!(state.byte_d4, 1);
            assert!(state.scroll_user3_dc.is_some() && state.scroll_user4_e0.is_some());
            assert_eq!(cx.engine.menu(menu).state_24, 8, "Menu_Enable(0) fades in");
        });
        assert!(host.warnings.is_empty(), "{:?}", host.warnings);
    }
}

// 005B3F00: magic_popup_menu.xml's menu (1024) with its ten fields; the bottom's string "" (held as " ") and every row hidden (visible
// 1); the ctor's slide state 2 and its tick from the host.
#[test]
fn the_magic_popup_menu_builds_from_its_xml() {
    use recomp_game::tes::magic_popup_menu::*;
    let (mut world, settings, mut host) = interface_window();
    with_cx(&mut world, &settings, &mut host, |cx| {
        let created = magic_popup_menu_create(cx).unwrap().expect("the menu");
        let menu = cx.engine.containing_menu(created).unwrap();
        let MenuClass::MagicPopup(state) = cx.engine.menu(menu).class.clone() else {
            panic!("a MagicPopupMenu");
        };
        let mut names = vec!["magicpop_background".to_owned()];
        names.extend((1..=8).map(|n| format!("magicpop_effect_{n}")));
        names.push("magicpop_background_bottom".to_owned());
        for (tile, name) in state.tiles_28.iter().zip(&names) {
            assert_eq!(cx.engine.tile(tile.unwrap()).name_08, name.as_bytes());
        }
        for tile in &state.tiles_28[1..] {
            assert_eq!(tile_numeric_trait(cx, tile.unwrap(), TRAIT_VISIBLE), 1.0);
        }
        // TileValue_SetString keeps an empty string as " " (`tile_value_set_string`).
        assert_eq!(text(cx, state.tiles_28[9].unwrap(), TRAIT_STRING), " ");
        assert_eq!(state.animation_state_58, 2);
        assert_eq!((state.end_pos_x_50, state.start_pos_x_54), (0.0, 0.0));
        assert_ne!(state.tick_5c, 0, "the host's GetTickCount");
        assert_eq!(cx.engine.menu(menu).state_24, 8, "Menu_Enable(0) fades in");
    });
    assert!(host.warnings.is_empty(), "{:?}", host.warnings);
}

// 005965C0: breath_meter_menu.xml's menu (1045) with the root as field 1 and the ribbon as field 2; 00596550(1.0) puts
// +0x30 and the ribbon's user1 at 100. The menu isn't enabled. A later 00596550(0.335) shows 33, and the same percent
// again stores nothing.
#[test]
fn the_breath_menu_builds_from_its_xml() {
    use recomp_game::tes::breath_menu::*;
    let (mut world, settings, mut host) = interface_window();
    with_cx(&mut world, &settings, &mut host, |cx| {
        let created = breath_menu_create(cx).unwrap().expect("the menu");
        let menu = cx.engine.containing_menu(created).unwrap();
        let state = |cx: &InterfaceCx| match cx.engine.menu(menu).class.clone() {
            MenuClass::Breath(state) => state,
            _ => panic!("a BreathMenu"),
        };
        let first = state(cx);
        assert_eq!(first.root_28, Some(created));
        let ribbon = first.ribbon_2c.unwrap();
        assert_eq!(cx.engine.tile(ribbon).name_08, b"breath_progress_ribbon");
        assert_eq!(first.percent_30, 100);
        assert_eq!(tile_numeric_trait(cx, ribbon, TRAIT_USER1), 100.0);
        assert_eq!(
            cx.engine.menu(menu).state_24,
            4,
            "not enabled: still the ctor's Hidden"
        );
        breath_menu_set_percent(cx, 0.335).unwrap();
        assert_eq!(state(cx).percent_30, 33);
        assert_eq!(tile_numeric_trait(cx, ribbon, TRAIT_USER1), 33.0);
        tile_set_numeric_trait(cx, ribbon, TRAIT_USER1, 7.0).unwrap();
        breath_menu_set_percent(cx, 0.335).unwrap();
        assert_eq!(
            tile_numeric_trait(cx, ribbon, TRAIT_USER1),
            7.0,
            "an unchanged percent"
        );
    });
    assert!(host.warnings.is_empty(), "{:?}", host.warnings);
}

// 005CE840: recharge_menu.xml's menu (1049) with its five fields (field 5 by the scroll's user6), the scroll's user9 back
// at 0, the ctor's 0xFF byte.
#[test]
fn the_recharge_menu_builds_from_its_xml() {
    use recomp_game::tes::recharge_menu::*;
    let (mut world, settings, mut host) = interface_window();
    with_cx(&mut world, &settings, &mut host, |cx| {
        let created = recharge_menu_create(cx).unwrap().expect("the menu");
        let menu = cx.engine.containing_menu(created).unwrap();
        let MenuClass::Recharge(state) = cx.engine.menu(menu).class.clone() else {
            panic!("a RechargeMenu");
        };
        for (id, name) in [
            (1, "recharge_FocusBox"),
            (3, "recharge_button_exit"),
            (4, "recharge_scroll_bar"),
            (6, "recharge_list_pane"),
        ] {
            let field = state.field(id).unwrap();
            assert_eq!(cx.engine.tile(field).name_08, name.as_bytes(), "field {id}");
        }
        assert_eq!(
            tile_numeric_trait(cx, state.field(5).unwrap(), TRAIT_ID),
            5.0
        );
        assert_eq!(
            tile_numeric_trait(cx, state.field(4).unwrap(), TRAIT_USER9),
            0.0
        );
        assert_eq!(state.byte_50, 0xFF);
        assert_eq!(state.soul_gem_entry_44, 0);
        assert_eq!(cx.engine.menu(menu).state_24, 8, "Menu_Enable(0) fades in");
    });
    assert!(host.warnings.is_empty(), "{:?}", host.warnings);
}

// 005DCF20: TextEditMenu.xml's menu (1051) with its three fields; the prompt on the root's user0, the default text on
// field 1 and in the input state, whose width is the field's wrapwidth 250 less 5 and whose caret is at the end.
#[test]
fn the_text_edit_menu_builds_from_its_xml() {
    use recomp_game::tes::text_edit_menu::*;
    let (mut world, settings, mut host) = interface_window();
    with_cx(&mut world, &settings, &mut host, |cx| {
        assert!(text_edit_menu_create(cx, b"Name your spell", b"Fireball").unwrap());
        let tile = cx.engine.tile_menu_by_id(MENU_ID_TEXT_EDIT).unwrap();
        let menu = cx.engine.containing_menu(tile).unwrap();
        let MenuClass::TextEdit(state) = cx.engine.menu(menu).class.clone() else {
            panic!("a TextEditMenu");
        };
        for (slot, name) in [
            "textedit_text",
            "textedit_button_ok",
            "textedit_button_back",
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(
                cx.engine.tile(state.tiles_28[slot].unwrap()).name_08,
                name.as_bytes()
            );
        }
        assert_eq!(text(cx, tile, TRAIT_USER0), "Name your spell");
        assert_eq!(
            text(cx, state.tiles_28[0].unwrap(), TRAIT_STRING),
            "Fireball"
        );
        let input = &state.input_state_34;
        assert_eq!(input.max_pixel_width_0c, 245);
        assert_eq!(input.editable_text_18, b"Fireball");
        assert_eq!(input.mirrored_text_20, b"Fireball");
        assert_eq!((input.enabled_00, input.caret_index_04), (1, 8));
        assert_eq!(input.font_index_14, 1);
        assert_eq!(cx.engine.menu(menu).state_24, 8, "Menu_Enable(0) fades in");
        assert!(
            cx.world
                .resource::<TextEditStatics>()
                .text_b3b738
                .is_empty()
        );
    });
    assert!(host.warnings.is_empty(), "{:?}", host.warnings);
}

// 005D39A0: save_menu.xml's menu (1039) with its fields; the "New Save" row (index 0, id 101) first, then one row per
// save from the host's list: a named file counts in 00B3B71C and "Save 3 - ..." doesn't.
#[test]
fn the_save_menu_builds_from_its_xml() {
    use recomp_game::tes::save_files::SaveFile;
    use recomp_game::tes::save_menu::*;
    let save = |name: &str, number| SaveFile {
        path: std::path::PathBuf::from(format!("Saves/{name}.ess")),
        readable: true,
        save_number: number,
        player: b"Prisoner".to_vec(),
        level: 2,
        location: None,
        game_days: 1,
        play_ticks: 3_723_000,
        time: [2026, 11, 0, 8, 10, 30, 0, 0],
        preview: None,
    };
    let (mut world, settings, mut host) = interface_window();
    host.saves = vec![
        save("Save 3 - Prisoner  Imperial Prison", 3),
        save("Before the sewers", 4),
    ];
    with_cx(&mut world, &settings, &mut host, |cx| {
        let created = save_menu_create(cx).unwrap().expect("the menu");
        let menu = cx.engine.containing_menu(created).unwrap();
        let MenuClass::Save(state) = cx.engine.menu(menu).class.clone() else {
            panic!("a SaveMenu");
        };
        for (tile, name) in [
            (state.field_id1_28, "save_button_return"),
            (state.field_id2_2c, "save_FocusBox"),
            (state.field_id3_34, "save_scroll_bar"),
            (state.field_id5_3c, "save_list"),
            (state.field_id6_40, "save_Picture"),
            (state.field_id7_44, "save_Text"),
            (state.list_pane_48, "save_list_pane"),
        ] {
            assert_eq!(cx.engine.tile(tile.unwrap()).name_08, name.as_bytes());
        }
        assert_eq!(state.saves_4c.len(), 2);
        let pane = state.list_pane_48.unwrap();
        let rows: Vec<TileId> = cx
            .engine
            .tile(pane)
            .children_30
            .iter()
            .rev()
            .copied()
            .filter(|&row| tile_numeric_trait(cx, row, TRAIT_ID) >= 101.0)
            .collect();
        let read: Vec<(f32, f32, String)> = rows
            .iter()
            .map(|&row| {
                (
                    tile_numeric_trait(cx, row, TRAIT_ID),
                    tile_numeric_trait(cx, row, TRAIT_USER0),
                    text(cx, row, TRAIT_USER3),
                )
            })
            .collect();
        assert_eq!(
            read,
            [
                (101.0, 0.0, "New Save".to_owned()),
                (
                    102.0,
                    1.0,
                    "Save 3 - Level 2\nPlay Time: 01:02:03".to_owned()
                ),
                (
                    103.0,
                    2.0,
                    "Before the sewers\nPlay Time: 01:02:03".to_owned()
                ),
            ]
        );
        assert_eq!(cx.engine.tile(rows[0]).name_08, b"New_Save");
        assert_eq!(cx.world.resource::<SaveMenuStatics>().named_saves_b3b71c, 1);
        assert_eq!(cx.engine.menu(menu).state_24, 8, "Menu_Enable(0) fades in");
    });
    assert!(host.warnings.is_empty(), "{:?}", host.warnings);
}
