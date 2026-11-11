//! The first screens on the real data, on one World: the LoadingMenu the driver leaves, then the MainMenu, its prompts and the Load list, through `run_screens_frame`. Saves come from a scratch folder beside a copy of the pinned INI, never the real Documents. IDA: N/A test harness. Host tests earn no validation bonus.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bevy_ecs::prelude::*;
use oblivion_core::fixtures;
use oblivion_sim::tes::interface_draw::{QuadTexture, interface_draw_list, interface_view};
use oblivion_sim::tes::interface_frame::*;
use oblivion_sim::tes::interface_manager::InterfaceManager;
use oblivion_sim::tes::tile::{InterfaceCx, MenuClass, TileId};
use oblivion_sim::tes::tile_values::*;
use oblivion_sim::tes::winmain::winmain_menu_media::run_idle_frame;
use oblivion_sim::tes::winmain::winmain_screens::{
    ScreenDirector, ScreenStage, open_main_menu, run_screens_frame,
};
use oblivion_sim::tes::winmain::{
    Block, SettingsSource, WinMainConfig, WinMainProgress, WinMainSeams, drain_movie_device_queue,
    install_winmain, run_winmain_straight_through, winmain_schedule, winmain_shutdown,
    with_driver_interface,
};

fn write_save(path: &Path, number: u32, modified: u64) {
    let (width, height) = (16u32, 9u32);
    let mut body = number.to_le_bytes().to_vec();
    body.extend([8u8]);
    body.extend(b"Natfoth\0");
    body.extend((3 + number as u16).to_le_bytes());
    body.extend([14u8]);
    body.extend(b"Imperial City\0");
    body.extend(12.0f32.to_le_bytes());
    body.extend((3_723_000 * number).to_le_bytes());
    for field in [2006u16, 4, 0, 5, 14, 7, 1, 0] {
        body.extend(field.to_le_bytes());
    }
    let pixels = (width * height * 3) as usize;
    body.extend((8 + pixels as u32).to_le_bytes());
    body.extend(width.to_le_bytes());
    body.extend(height.to_le_bytes());
    body.extend(vec![200u8; pixels]);
    let mut file = b"TES4SAVEGAME".to_vec();
    file.extend([0u8, 125]);
    file.extend([0u8; 16]);
    file.extend(0x7Du32.to_le_bytes());
    file.extend((body.len() as u32).to_le_bytes());
    file.extend(body);
    std::fs::write(path, file).unwrap();
    let time = std::time::UNIX_EPOCH + Duration::from_secs(modified);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
}

/// A new folder with the pinned INI and `saves` saves, the highest number the newest.
fn folder(tag: &str, saves: u32) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("recomp_screens_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(folder.join("Saves")).unwrap();
    let text = std::fs::read(fixtures::root().join("config/Oblivion.ini")).unwrap();
    std::fs::write(folder.join("Oblivion.ini"), text).unwrap();
    for number in 1..=saves {
        write_save(
            &folder.join("Saves").join(format!("Save {number}.ess")),
            number,
            1_000_000 + u64::from(number) * 1000,
        );
    }
    folder
}

struct Run {
    world: World,
    folder: PathBuf,
}

impl Drop for Run {
    fn drop(&mut self) {
        winmain_shutdown(&mut self.world);
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn start(tag: &str, saves: u32) -> Run {
    let folder = folder(tag, saves);
    let mut world = World::new();
    install_winmain(
        &mut world,
        WinMainConfig {
            data_directory: fixtures::data_dir(),
            settings: SettingsSource::Pinned(folder.join("Oblivion.ini")),
            seed: Some(1),
            frame_budget: None,
        },
        WinMainSeams::default(),
    );
    run_winmain_straight_through(&mut world);
    assert!(world.resource::<WinMainProgress>().finished());
    assert!(world.resource::<WinMainProgress>().stops.is_empty());
    Run { world, folder }
}

impl Run {
    /// A frame as the host runs one.
    fn frame(&mut self, input: &InterfaceInputFrame) {
        // As the host's frame does. Nothing paces the scripted movie thread, so an undrained queue takes 4 GB of tile copies.
        drain_movie_device_queue(&mut self.world);
        open_main_menu(&mut self.world).unwrap();
        run_idle_frame(&mut self.world);
        run_screens_frame(&mut self.world, input).unwrap();
    }

    fn settle(&mut self) {
        // A seeded clock gives 1 ms frames, so a two-second fade takes 2000.
        for _ in 0..20_000 {
            self.frame(&InterfaceInputFrame::default());
            let quiet = self
                .world
                .resource::<InterfaceManager>()
                .block_130
                .as_ref()
                .is_some_and(|block| block.nodes.is_empty());
            let stage = self
                .world
                .get_resource::<ScreenDirector>()
                .map(|director| director.stage);
            if quiet && stage == Some(ScreenStage::Menu) {
                for _ in 0..3 {
                    self.frame(&InterfaceInputFrame::default());
                }
                return;
            }
        }
        let block = self.world.resource::<InterfaceManager>().block_130.clone();
        panic!(
            "the screens never settled: {:?} {:?}",
            self.world.get_resource::<ScreenDirector>(),
            block
        );
    }

    fn key(&mut self, key: i32) {
        self.frame(&InterfaceInputFrame {
            keys: vec![key],
            ..InterfaceInputFrame::default()
        });
        self.settle();
    }

    fn with<R>(&mut self, body: impl FnOnce(&mut InterfaceCx) -> R) -> R {
        with_driver_interface(&mut self.world, body).unwrap()
    }

    /// Down gives the default focus, then Right to the button, then Enter.
    fn click_button(&mut self, id: f32) {
        self.key(KEY_DOWN);
        for _ in 0..8 {
            let focus = self.with(|cx| {
                let alt = cx.world.resource::<InterfaceManager>().alt_active_tile_088;
                alt.map(|tile| tile_numeric_trait(cx, tile, TRAIT_ID))
            });
            if focus == Some(id) {
                self.key(KEY_ENTER);
                return;
            }
            self.key(KEY_RIGHT);
        }
        panic!("focus never reached button {id}");
    }
}

fn named(cx: &InterfaceCx, root: TileId, name: &[u8]) -> Option<TileId> {
    if cx.engine.tile(root).name_08 == name {
        return Some(root);
    }
    cx.engine
        .tile(root)
        .children_30
        .iter()
        .find_map(|&child| named(cx, child, name))
}

fn text(cx: &InterfaceCx, tile: TileId) -> Vec<u8> {
    tile_string_trait(cx, tile, TRAIT_STRING).unwrap_or_default()
}

// The C++ `menu_test`, no saves: Continue hidden; `new` holds the priority. IDA: 005B6040.
#[test]
fn no_saves_hide_continue() {
    let mut run = start("nosaves", 0);
    run.settle();
    run.with(|cx| {
        let tile = cx.engine.tile_menu_by_id(1044).expect("the MainMenu");
        assert_eq!(tile_numeric_trait(cx, tile, TRAIT_CLASS), 1044.0);
        let button = named(cx, tile, b"main_continue").unwrap();
        assert_eq!(tile_numeric_trait(cx, button, TRAIT_VISIBLE), 1.0);
        let new = named(cx, tile, b"main_new").unwrap();
        assert_eq!(tile_numeric_trait(cx, new, TRAIT_VISIBLE), 2.0);
        assert!(tile_numeric_trait(cx, new, TRAIT_XDEFAULT) > 1.0);
        assert_eq!(cx.engine.tile_menu_by_id(1007), None, "0040F088 closed it");
    });
}

// The C++ `menu_test`, with saves: Continue shows with the priority; New gets 1.0. Version, zoom and the backdrop's node. IDA: 005B6040.
#[test]
fn saves_show_continue_and_the_version() {
    let mut run = start("saves", 2);
    run.settle();
    run.with(|cx| {
        let tile = cx.engine.tile_menu_by_id(1044).unwrap();
        let button = named(cx, tile, b"main_continue").unwrap();
        assert_eq!(tile_numeric_trait(cx, button, TRAIT_VISIBLE), 2.0);
        assert!(tile_numeric_trait(cx, button, TRAIT_XDEFAULT) > 100.0);
        let new = named(cx, tile, b"main_new").unwrap();
        assert_eq!(tile_numeric_trait(cx, new, TRAIT_XDEFAULT), 1.0);
        let version = named(cx, tile, b"version").unwrap();
        assert_eq!(text(cx, version), b"1.2.0416");
        let zoom = tile_numeric_trait(cx, version, TRAIT_ZOOM);
        assert_eq!(zoom, 75.0);
        // 005B642D `fld1; fdivrp` and 005B6438 `fmul` round at 24 bits. The bits are worked by hand from x = 10, y = 935: the f64 form gives 0x41555555.
        assert_eq!(
            tile_numeric_trait(cx, version, TRAIT_X).to_bits(),
            0x4155_5556
        );
    });
}

// Menu_Enable starts at alpha 0 and 00583F40 ends Visible; a frame in between has a fractional alpha. The C++ only forces 255. IDA: 00585190, 00583F40.
#[test]
fn the_menu_fades_in() {
    let mut run = start("fade", 0);
    open_main_menu(&mut run.world).unwrap();
    let state = run.with(|cx| {
        let tile = cx.engine.tile_menu_by_id(1044).unwrap();
        cx.engine
            .menu(cx.engine.containing_menu(tile).unwrap())
            .state_24
    });
    assert_eq!(state, MENU_STATE_FADING_IN);
    run.settle();
    run.with(|cx| {
        let tile = cx.engine.tile_menu_by_id(1044).unwrap();
        let menu = cx.engine.containing_menu(tile).unwrap();
        assert_eq!(cx.engine.menu(menu).state_24, MENU_STATE_VISIBLE);
        assert_eq!(tile_numeric_trait(cx, tile, TRAIT_VISIBLE), 2.0);
    });
}

// The C++ `input_and_actions_test` New path: 005B64C0 3 has no +0x4D; the prompt's two buttons; No closes and leaves the menu. IDA: 005B64C0, 005BC8B0, 005BCDD0.
#[test]
fn new_game_prompt_says_yes_and_no() {
    let mut run = start("new", 0);
    run.settle();
    run.click_button(3.0);
    run.with(|cx| {
        let tile = cx.engine.tile_menu_by_id(1001).expect("the prompt");
        let body = named(cx, tile, b"message_text").unwrap();
        assert_eq!(text(cx, body), b"Start new game?");
        let yes = named(cx, tile, b"button_1").unwrap();
        assert_eq!(text(cx, yes), b"Yes");
        let no = named(cx, tile, b"button_2").unwrap();
        assert_eq!(text(cx, no), b"No");
        let main = cx.engine.tile_menu_by_id(1044).unwrap();
        let menu = cx.engine.containing_menu(main).unwrap();
        let MenuClass::Main(fields) = &cx.engine.menu(menu).class else {
            panic!("the class");
        };
        assert_eq!(
            fields.confirmation_pending_4d, 0,
            "005B64C0's New arm sets nothing"
        );
    });
    run.key(KEY_DOWN);
    run.key(KEY_ENTER);
    run.with(|cx| {
        assert_eq!(
            cx.engine.tile_menu_by_id(1001),
            None,
            "005BC6B0 destroys it"
        );
        assert!(cx.engine.tile_menu_by_id(1044).is_some());
    });
}

// The C++ exit confirmation: Exit Game and Cancel, +0x4D until 005B59B0, and Yes sets quitGame. IDA: 005B64C0, 005B59B0.
#[test]
fn exit_asks_and_quits() {
    let mut run = start("exit", 0);
    run.settle();
    run.click_button(7.0);
    run.with(|cx| {
        let tile = cx.engine.tile_menu_by_id(1001).unwrap();
        assert_eq!(
            text(cx, named(cx, tile, b"button_1").unwrap()),
            b"Exit Game"
        );
        assert_eq!(text(cx, named(cx, tile, b"button_2").unwrap()), b"Cancel");
    });
    run.key(KEY_ENTER);
    assert!(oblivion_sim::tes::winmain::quit_requested(&run.world));
}

// The C++ `load_menu_test`: a row per save, newest first, captions and details. IDA: 005AEA60, 005AE6D0, 005AE240.
#[test]
fn the_load_list_has_a_row_per_save() {
    let mut run = start("load", 3);
    run.settle();
    run.click_button(4.0);
    run.with(|cx| {
        let tile = cx.engine.tile_menu_by_id(1038).expect("the Load menu");
        let menu = cx.engine.containing_menu(tile).unwrap();
        let MenuClass::Load(load) = &cx.engine.menu(menu).class else {
            panic!("the class");
        };
        assert_eq!(load.saves_54.len(), 3);
        assert!(
            load.saves_54[0].path.ends_with("Save 3.ess"),
            "0045D450's newest first"
        );
        let pane = load.list_pane_48.expect("ID 9");
        let rows = cx.engine.tile(pane).children_30.clone();
        assert_eq!(rows.len(), 3, "005AE6D0 once a save");
        let captions: Vec<_> = rows
            .iter()
            .map(|&row| lift(tile_string_trait(cx, row, TRAIT_USER3)))
            .collect();
        assert!(
            captions
                .iter()
                .any(|c| c == "Save 3 - Level 6\nPlay Time: 03:06:09"),
            "{captions:?}"
        );
        let details = load.field_id7_44.unwrap();
        let shown = lift(tile_string_trait(cx, details, TRAIT_STRING));
        assert_eq!(
            shown,
            "Natfoth\nLevel 6\nImperial City\nDay 12\n4/5/2006 14:07"
        );
    });
}

// IDA: 005AEED0: id 1 is Return, destroyOnClose and the fade out, and the Load menu goes.
#[test]
fn return_closes_the_load_menu() {
    let mut run = start("return", 1);
    run.settle();
    run.click_button(4.0);
    run.with(|cx| {
        let tile = cx.engine.tile_menu_by_id(1038).unwrap();
        let back = named(cx, tile, b"load_return_button").unwrap();
        interface_dispatch_pointer_event(cx, Some(back), TRAIT_MOUSEOVER).unwrap();
    });
    run.key(KEY_ENTER);
    run.settle();
    run.with(|cx| assert_eq!(cx.engine.tile_menu_by_id(1038), None));
}

// IDA: 0058377C..00583929: Escape on a visible Load menu is 005AEED0(1, null) when +0x28 shows (0057D3F0). On the MainMenu it does nothing: the C++'s Exit answer isn't the exe's.
#[test]
fn escape_closes_the_load_menu_only() {
    let mut run = start("escape", 1);
    run.settle();
    let escape = InterfaceInputFrame {
        escape: true,
        ..InterfaceInputFrame::default()
    };
    run.frame(&escape);
    run.settle();
    run.with(|cx| {
        assert!(cx.engine.tile_menu_by_id(1044).is_some());
        assert_eq!(cx.engine.tile_menu_by_id(1001), None);
    });
    run.click_button(4.0);
    run.with(|cx| assert!(cx.engine.tile_menu_by_id(1038).is_some()));
    run.frame(&escape);
    run.settle();
    run.with(|cx| assert_eq!(cx.engine.tile_menu_by_id(1038), None));
}

// snapshot-first_idle: the MainMenu node's y is -0.648, depth 81: the HUD's -50 + its 129 at +0x18, + 2. The HUD is there, hidden, and 00579770 left the mode stack to the MainMenu.
#[test]
fn the_main_menu_takes_the_depth_the_hud_leaves() {
    let mut run = start("depth", 0);
    run.settle();
    run.with(|cx| {
        let menu = cx.engine.tile_menu_by_id(1044).unwrap();
        assert_eq!(tile_numeric_trait(cx, menu, TRAIT_DEPTH), 81.0);
        let hud = cx.engine.tile_menu_by_id(1004).unwrap();
        assert_eq!(tile_numeric_trait(cx, hud, TRAIT_DEPTH), -50.0);
        let owner = cx.engine.containing_menu(hud).unwrap();
        assert_eq!(cx.engine.menu(owner).depth_18, 129);
        assert_eq!(cx.engine.menu(owner).state_24, 4);
        let manager = cx.world.resource::<InterfaceManager>();
        assert_eq!(manager.menu_mode_stack_0e0[..2], [1044, 0]);
    });
}

// The HUD is hidden behind the MainMenu: run-c1-log's frames show none of it, so nothing of it draws.
#[test]
fn the_hud_draws_nothing_at_the_main_menu() {
    let mut run = start("hud-hidden", 0);
    run.settle();
    run.with(|cx| {
        let view = interface_view(cx);
        let quads = interface_draw_list(cx.world, cx.engine, &view);
        let hud: Vec<_> = quads
            .iter()
            .filter_map(|quad| match &quad.texture {
                QuadTexture::File(path) => Some(path),
                _ => None,
            })
            .filter(|path| {
                path.to_ascii_lowercase()
                    .windows(4)
                    .any(|part| part == b"\\hud")
            })
            .collect();
        assert!(hud.is_empty(), "{hud:?}");
    });
}

// The draw list: the menu's images have real paths, the text has glyph quads, and the depth order puts the backdrop first. IDA: 0057F170.
#[test]
fn the_draw_list_holds_the_screen() {
    let mut run = start("draw", 0);
    run.settle();
    run.with(|cx| {
        let view = interface_view(cx);
        let quads = interface_draw_list(cx.world, cx.engine, &view);
        assert!(quads.iter().any(|quad| matches!(&quad.texture,
            QuadTexture::File(path) if path.ends_with(b"tes_oblivion_logo_final.dds"))));
        assert!(
            quads
                .iter()
                .any(|quad| matches!(quad.texture, QuadTexture::Font(_)))
        );
        let depths: Vec<_> = quads.iter().map(|quad| quad.depth).collect();
        let cursor = quads.len() - 1;
        assert!(
            depths[..cursor].windows(2).all(|pair| pair[0] >= pair[1]),
            "back to front"
        );
    });
}

fn lift(text: Option<Vec<u8>>) -> String {
    text.unwrap_or_default()
        .iter()
        .map(|&b| char::from(b))
        .collect()
}

// A real run from `Block::Finished`'s neighbour: 0040F069 is reached first.
#[test]
fn nothing_opens_before_the_chain_ends() {
    let folder = folder("early", 0);
    let mut world = World::new();
    install_winmain(
        &mut world,
        WinMainConfig {
            data_directory: fixtures::data_dir(),
            settings: SettingsSource::Pinned(folder.join("Oblivion.ini")),
            seed: Some(1),
            frame_budget: Some(Duration::ZERO),
        },
        WinMainSeams::default(),
    );
    let mut schedule = winmain_schedule();
    while world.resource::<WinMainProgress>().next <= Block::Interface {
        schedule.run(&mut world);
    }
    open_main_menu(&mut world).unwrap();
    assert!(world.get_resource::<ScreenDirector>().is_none());
    let run =
        with_driver_interface(&mut world, |cx| cx.engine.tile_menu_by_id(1007).is_some()).unwrap();
    assert!(run, "the LoadingMenu is up");
    winmain_shutdown(&mut world);
    let _ = std::fs::remove_dir_all(&folder);
}

// The C++ `load_menu_test` counts: a row each, 250 and 251 reach the last, none is lost past the six that show. 005AE6D0 makes one per save; the XML's clip window shows six. IDA: 005AEA60.
#[test]
fn every_save_gets_a_row() {
    for count in [0u32, 1, 7, 251] {
        let mut run = start(&format!("rows{count}"), count);
        run.settle();
        run.click_button(4.0);
        run.with(|cx| {
            let tile = cx.engine.tile_menu_by_id(1038).unwrap();
            let menu = cx.engine.containing_menu(tile).unwrap();
            let MenuClass::Load(load) = &cx.engine.menu(menu).class else {
                panic!("the class");
            };
            assert_eq!(load.saves_54.len(), count as usize);
            let rows = cx.engine.tile(load.list_pane_48.unwrap()).children_30.len();
            assert_eq!(
                rows,
                count.max(1) as usize,
                "no saves still make the one empty row"
            );
        });
    }
}

// The pointer: over a button it takes focus through 00581390, press and release click it, and a release over another target doesn't. A release over nothing does click: 005821F0 stores null in +0xA0 when the hit is null, and its test takes null as a match. The C++ `MenuInput::mouse_up` differs: it wants the same tile. The C++ `input_and_actions_test`'s mouse half. IDA: 005821F0.
#[test]
fn the_pointer_clicks_a_button() {
    let mut run = start("pointer", 0);
    run.settle();
    let place = |run: &mut Run, name: &'static [u8]| {
        run.with(|cx| {
            let tile = cx.engine.tile_menu_by_id(1044).unwrap();
            let button = named(cx, tile, name).unwrap();
            let text = named(cx, button, b"button_text").unwrap();
            let view = interface_view(cx);
            let quad = interface_draw_list(cx.world, cx.engine, &view)
                .into_iter()
                .find(|quad| quad.tile == text)
                .unwrap();
            [
                quad.positions[0][0] / view.renderer_width,
                quad.positions[0][1] / view.renderer_height + 0.005,
            ]
        })
    };
    let centre = place(&mut run, b"main_exit");
    let other = place(&mut run, b"main_credits");
    let at = |cursor| InterfaceInputFrame {
        cursor: Some(cursor),
        ..InterfaceInputFrame::default()
    };
    run.frame(&at(centre));
    run.frame(&at(centre));
    let focus = run.with(|cx| {
        let active = cx.world.resource::<InterfaceManager>().active_tile_098;
        active.map(|tile| tile_numeric_trait(cx, tile, TRAIT_ID))
    });
    assert_eq!(focus, Some(7.0), "the hover found Exit");
    run.frame(&InterfaceInputFrame {
        left_pressed: true,
        left_held: true,
        ..at(centre)
    });
    run.frame(&InterfaceInputFrame {
        left_held: true,
        ..at(other)
    });
    run.frame(&InterfaceInputFrame {
        left_released: true,
        ..at(other)
    });
    run.with(|cx| assert_eq!(cx.engine.tile_menu_by_id(1001), None));
    run.frame(&at(centre));
    run.frame(&at(centre));
    run.frame(&InterfaceInputFrame {
        left_pressed: true,
        left_held: true,
        ..at(centre)
    });
    run.frame(&InterfaceInputFrame {
        left_released: true,
        ..at(centre)
    });
    run.with(|cx| assert!(cx.engine.tile_menu_by_id(1001).is_some(), "the prompt"));
}

// The C++ `loading_menu_tests` XML half on the real engine, at the Interface block: the title's geometry, its string, and the ribbon's width from user3. The C++ reads these from its DOM. Expected numbers are the C++ test's. IDA: 005ADCF0, 005ADEC0.
#[test]
fn the_loading_menu_lays_out() {
    let folder = folder("loading", 0);
    let mut world = World::new();
    install_winmain(
        &mut world,
        WinMainConfig {
            data_directory: fixtures::data_dir(),
            settings: SettingsSource::Pinned(folder.join("Oblivion.ini")),
            seed: Some(1),
            frame_budget: Some(Duration::ZERO),
        },
        WinMainSeams::default(),
    );
    let mut schedule = winmain_schedule();
    while world.resource::<WinMainProgress>().next <= Block::Interface {
        schedule.run(&mut world);
    }
    with_driver_interface(&mut world, |cx| {
        let tile = cx.engine.tile_menu_by_id(1007).unwrap();
        assert_eq!(tile_numeric_trait(cx, tile, TRAIT_USER0), 0.0, "title mode");
        let symbol = named(cx, tile, b"load_symbol").unwrap();
        assert_eq!(tile_numeric_trait(cx, symbol, TRAIT_WIDTH), 330.0);
        assert_eq!(tile_numeric_trait(cx, symbol, TRAIT_HEIGHT), 466.0);
        assert_eq!(tile_numeric_trait(cx, symbol, TRAIT_Y), 250.0);
        let ribbon = named(cx, tile, b"load_progress_ribbon").unwrap();
        assert_eq!(
            tile_numeric_trait(cx, ribbon, TRAIT_WIDTH),
            1.0,
            "zero's minimum"
        );
        let label = named(cx, tile, b"load_loading").unwrap();
        assert_eq!(text(cx, label), b"Loading...");
        tile_set_numeric_trait(cx, tile, TRAIT_USER3, 50.0).unwrap();
        assert_eq!(
            tile_numeric_trait(cx, ribbon, TRAIT_WIDTH),
            90.0,
            "user3 50's ribbon"
        );
        // 005ADEC0's head: visible 2 on every call.
        tile_set_numeric_trait(cx, tile, TRAIT_VISIBLE, 1.0).unwrap();
        oblivion_sim::tes::loading_menu::loading_menu_set_section_progress(cx, 3, 100.0).unwrap();
        assert_eq!(tile_numeric_trait(cx, tile, TRAIT_VISIBLE), 2.0);
    })
    .unwrap();
    winmain_shutdown(&mut world);
    let _ = std::fs::remove_dir_all(&folder);
}

// The C++ `menu_test`'s geometry on the real engine, at the real virtual width: the C++ test's 1920 wide screen made its x's differ, and its priority of 102 came from its own runtime. Widths are the C++ test's. IDA: 005B6040, 0058E3B0.
#[test]
fn the_menu_lays_out_from_its_assets() {
    let mut run = start("layout", 1);
    run.settle();
    run.with(|cx| {
        let view = interface_view(cx);
        let tile = cx.engine.tile_menu_by_id(1044).unwrap();
        let press = named(cx, tile, b"press_start").unwrap();
        assert_eq!(tile_numeric_trait(cx, press, TRAIT_WIDTH), 190.0);
        assert_eq!(
            tile_numeric_trait(cx, press, TRAIT_X),
            (view.virtual_width - 190.0) / 2.0
        );
        assert_eq!(text(cx, press), b"Press Start");
        let layout = named(cx, tile, b"button_layout").unwrap();
        assert_eq!(tile_numeric_trait(cx, layout, TRAIT_WIDTH), 972.0);
        assert_eq!(
            tile_numeric_trait(cx, layout, TRAIT_X),
            (view.virtual_width - 972.0) / 2.0
        );
        let button = named(cx, tile, b"main_continue").unwrap();
        assert_eq!(tile_numeric_trait(cx, button, TRAIT_WIDTH), 183.0);
        assert_eq!(tile_numeric_trait(cx, button, TRAIT_ID), 2.0);
        assert_eq!(text(cx, button), b"Continue");
        assert_eq!(
            text(cx, named(cx, button, b"button_text").unwrap()),
            b"Continue"
        );
        let floating = named(cx, button, b"button_floating").unwrap();
        assert_eq!(tile_numeric_trait(cx, floating, TRAIT_WIDTH), 173.0);
        let end = named(cx, floating, b"button_end").unwrap();
        assert_eq!(tile_numeric_trait(cx, end, TRAIT_X), 77.0);
        let new = named(cx, tile, b"main_new").unwrap();
        let (target, event) = navigation_target(cx, button, TRAIT_XRIGHT).unwrap();
        assert_eq!(target, Some(new));
        assert_eq!(event, Some(TRAIT_MOUSEOVER));
        assert_eq!(
            tile_numeric_trait(
                cx,
                named(cx, tile, b"main_exit").unwrap(),
                TRAIT_CLICK_SOUND
            ),
            1.0
        );
        assert_eq!(tile_numeric_trait(cx, new, TRAIT_CLICK_SOUND), 0.0);
        let backdrop = named(cx, tile, b"backdrop").unwrap();
        assert_eq!(tile_numeric_trait(cx, backdrop, TRAIT_ID), 8.0);
    });
}

// The C++ `menu_test` with no saves: the layout's width follows Continue's visibility.
#[test]
fn no_saves_narrow_the_layout() {
    let mut run = start("narrow", 0);
    run.settle();
    run.with(|cx| {
        let view = interface_view(cx);
        let tile = cx.engine.tile_menu_by_id(1044).unwrap();
        let layout = named(cx, tile, b"button_layout").unwrap();
        assert_eq!(tile_numeric_trait(cx, layout, TRAIT_WIDTH), 764.0);
        assert_eq!(
            tile_numeric_trait(cx, layout, TRAIT_X),
            (view.virtual_width - 764.0) / 2.0
        );
    });
}

// The C++ `recreation_test`: a second 005B6040 deletes the first, leaves one in the table and clears the pending confirmation. IDA: 005B6040's head.
#[test]
fn a_second_create_replaces_the_menu() {
    let mut run = start("recreate", 0);
    run.settle();
    run.click_button(7.0);
    let first = run.with(|cx| cx.engine.tile_menu_by_id(1044).unwrap());
    run.with(|cx| {
        oblivion_sim::tes::menus::main_menu_create(cx, 0.0).unwrap();
        let second = cx.engine.tile_menu_by_id(1044).unwrap();
        assert_ne!(second, first);
        assert_eq!(cx.engine.tile(first).released_04, 1);
        let menu = cx.engine.containing_menu(second).unwrap();
        let MenuClass::Main(fields) = &cx.engine.menu(menu).class else {
            panic!("the class");
        };
        assert_eq!(fields.confirmation_pending_4d, 0);
    });
}

// 005AD440, reached from 005ADEC0 at the first 0057B950 after Oblivion.esm's LSCR group: 191 of its 337 forms have no LNAM (the 4 in DLCMehrunesRazor.esp all have one), and with a null context each draws once (5AD5CA). The MSVC stream is 214013x + 2531011: 191 draws of the seed's own stream, from before the load, land where the driver leaves it. No run records it: `_holdrand` at 004067CC is the capture.
#[test]
fn the_load_fills_the_slide_list_and_draws() {
    let folder = folder("slides", 0);
    let mut world = World::new();
    install_winmain(
        &mut world,
        WinMainConfig {
            data_directory: fixtures::data_dir(),
            settings: SettingsSource::Pinned(folder.join("Oblivion.ini")),
            seed: Some(1),
            // Phase 19 closes the menu: a budget of zero stops the chain before it.
            frame_budget: Some(std::time::Duration::ZERO),
        },
        WinMainSeams::default(),
    );
    let mut schedule = winmain_schedule();
    while world.resource::<WinMainProgress>().next < Block::CloseLoadingMenu {
        schedule.run(&mut world);
    }
    with_driver_interface(&mut world, |cx| {
        let tile = cx.engine.tile_menu_by_id(1007).unwrap();
        let menu = cx.engine.containing_menu(tile).unwrap();
        let MenuClass::Loading(loading) = &cx.engine.menu(menu).class else {
            panic!("the class");
        };
        assert_eq!(loading.slides_4c.len(), 191);
        let mut unique = loading.slides_4c.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 191, "each form once");
    })
    .unwrap();
    winmain_shutdown(&mut world);
    let _ = std::fs::remove_dir_all(&folder);
}

// 005BC8B0 at 5BCA1B and 5BCB24: an empty first button takes sOk (00B38CF0), an empty text sDefaultMessage (00B38E10): the live settings, which the load may have replaced.
#[test]
fn an_empty_prompt_takes_the_settings() {
    let mut run = start("prompt", 0);
    run.settle();
    run.with(|cx| {
        let mut settings = cx
            .world
            .resource_mut::<oblivion_sim::tes::game_settings::GameSettingCollection>();
        settings
            .setting_at_mut(0x00B3_8CF0)
            .unwrap()
            .set_string(b"Fine");
        settings
            .setting_at_mut(0x00B3_8E10)
            .unwrap()
            .set_string(b"Nothing");
        oblivion_sim::tes::menus::show_message(cx, b"", None, 0, &[Vec::new()]).unwrap();
        let tile = cx.engine.tile_menu_by_id(1001).unwrap();
        assert_eq!(
            text(cx, named(cx, tile, b"message_text").unwrap()),
            b"Nothing"
        );
        assert_eq!(text(cx, named(cx, tile, b"button_1").unwrap()), b"Fine");
    });
}
