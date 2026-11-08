//! WinMain 0040EB16..0040EB2C on the shipped data against P07-RUN-001 (1920x1080): the C++ `startup_phase07` test.
//! Every tile comes from the real `strings.xml` and `loading_menu.xml` in the BSAs, read through the ported file finder
//! and archives. Host tests earn no
//! validation bonus.
//!
//! Compared with the original run: every row of `phase07.rows` (re-derived from the run's T records as well),
//! `phase07_window.rows`, and the N records of every tile node and leaf shape. Compared with the C++ only:
//! `aspect_table`, the failure paths and the ribbon progress.

use std::path::PathBuf;

use bevy_ecs::prelude::*;
use recomp_core::fixtures;
use recomp_core::reader::StreamError;
use recomp_game::bs::archive::{ArchiveManager, ArchiveSettings};
use recomp_game::bs::file::OpenedFile;
use recomp_game::bs::file_finder::{BsFileCallbacks, FileFinder, bs_file_open_core};
use recomp_game::bs::scene_graphs::{
    SceneCamera, SceneGraph, SceneGraphGlobals, SceneGraphSettings, construct_scene_graph,
};
use recomp_game::gamebryo::av_object::{NiAvParent, NiAvProperties};
use recomp_game::gamebryo::mesh_geometry::GeometryDataIdCounter;
use recomp_game::gamebryo::object_net::{AlphaProperty, NiObjectNet};
use recomp_game::gamebryo::pixel_data::PixelData;
use recomp_game::gamebryo::properties::{FogProperty, VertexColorProperty, ZBufferProperty};
use recomp_game::gamebryo::scene_node::{NiNodeChildren, SceneNodePayload};
use recomp_game::gamebryo::scene_update::NoSceneControllers;
use recomp_game::tes::fonts::{FontManager, font_data_float, font_data_texture_count};
use recomp_game::tes::game_settings::GameSettingCollection;
use recomp_game::tes::interface_manager::*;
use recomp_game::tes::interface_scene::{TILE_SCISSOR_TRI_SHAPE_VTABLE, TileSceneGeometry};
use recomp_game::tes::loading_menu::loading_menu_update_display;
use recomp_game::tes::settings::{SettingCollections, register_static_settings};
use recomp_game::tes::tile::*;
use recomp_game::tes::tile_values::*;
use recomp_game::tes::tile_xml::tile_read_xml;
use recomp_startup::winmain_interface::run_winmain_interface_window;

// ------------------------------------------------------------------------------------------------
// The rows: `phase07.rows` is the C++ initializer table, one tile per line.

#[derive(Debug, PartialEq)]
struct ExpectedValue {
    trait_id: u32,
    number: bool,
    bits: u32,
    text: Option<Vec<u8>>,
}

#[derive(Debug, PartialEq)]
struct ExpectedTile {
    path: String,
    kind: String,
    name: Vec<u8>,
    flags: u32,
    children: usize,
    values: Vec<ExpectedValue>,
}

struct RowParser<'a> {
    bytes: &'a [char],
    at: usize,
}

impl RowParser<'_> {
    fn skip_space(&mut self) {
        while self.bytes.get(self.at) == Some(&' ') {
            self.at += 1;
        }
    }

    fn expect(&mut self, c: char) {
        self.skip_space();
        assert_eq!(
            self.bytes.get(self.at),
            Some(&c),
            "at {}: {:?}",
            self.at,
            self.bytes[self.at..].iter().take(30).collect::<String>()
        );
        self.at += 1;
    }

    fn peek(&mut self) -> char {
        self.skip_space();
        self.bytes[self.at]
    }

    /// A C string literal with `\\` and `\"` escapes, as `gen_expected.py` wrote it. Bytes are Latin-1.
    fn string(&mut self) -> Vec<u8> {
        self.expect('"');
        let mut out = Vec::new();
        loop {
            let c = self.bytes[self.at];
            self.at += 1;
            match c {
                '"' => return out,
                '\\' => {
                    out.push(self.bytes[self.at] as u8);
                    self.at += 1;
                }
                _ => out.push(c as u8),
            }
        }
    }

    fn word(&mut self) -> String {
        self.skip_space();
        let start = self.at;
        while self.bytes[self.at].is_alphanumeric() {
            self.at += 1;
        }
        self.bytes[start..self.at].iter().collect()
    }

    /// `0xFA7U`, `0U` or `5`.
    fn number(&mut self) -> u32 {
        let word = self.word();
        let word = word.strip_suffix('U').unwrap_or(&word);
        word.strip_prefix("0x").map_or_else(
            || word.parse().unwrap(),
            |hex| u32::from_str_radix(hex, 16).unwrap(),
        )
    }
}

fn parse_tile_row(line: &str) -> ExpectedTile {
    let chars: Vec<char> = line.chars().collect();
    let mut parser = RowParser {
        bytes: &chars,
        at: 0,
    };
    parser.expect('{');
    let path = String::from_utf8(parser.string()).unwrap();
    parser.expect(',');
    let kind = String::from_utf8(parser.string()).unwrap();
    parser.expect(',');
    let name = parser.string();
    parser.expect(',');
    let flags = parser.number();
    parser.expect(',');
    let children = parser.number() as usize;
    parser.expect(',');
    parser.expect('{');
    let mut values = Vec::new();
    while parser.peek() != '}' {
        parser.expect('{');
        let trait_id = parser.number();
        parser.expect(',');
        let number = parser.word() == "true";
        parser.expect(',');
        let bits = parser.number();
        parser.expect(',');
        let text = if parser.peek() == '"' {
            Some(parser.string())
        } else {
            assert_eq!(parser.word(), "nullptr");
            None
        };
        parser.expect('}');
        values.push(ExpectedValue {
            trait_id,
            number,
            bits,
            text,
        });
        if parser.peek() == ',' {
            parser.expect(',');
        }
    }
    parser.expect('}');
    parser.expect('}');
    parser.expect(',');
    ExpectedTile {
        path,
        kind,
        name,
        flags,
        children,
        values,
    }
}

fn expected_tiles() -> Vec<ExpectedTile> {
    fixtures::rows("phase07")
        .iter()
        .map(|line| parse_tile_row(line))
        .collect()
}

fn evidence() -> Vec<String> {
    let path = fixtures::root().join("evidence/phase07-run001-2026-10-06.records.txt");
    let bytes = std::fs::read(path).unwrap();
    let text: String = bytes.iter().map(|&byte| char::from(byte)).collect();
    text.lines().map(str::to_owned).collect()
}

fn hex_bytes(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|index| u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).unwrap())
        .collect()
}

fn le_bits(text: &str) -> u32 {
    u32::from_le_bytes(hex_bytes(text)[..4].try_into().unwrap())
}

/// The field after `key=` in a record line, up to the next space.
fn field<'a>(line: &'a str, key: &str) -> &'a str {
    let pattern = format!(" {key}=");
    let start = line
        .find(&pattern)
        .unwrap_or_else(|| panic!("{key} in {line}"))
        + pattern.len();
    line[start..].split(' ').next().unwrap()
}

/// The quoted `name="..."` of a record line.
fn quoted_name(line: &str) -> &str {
    let start = line.find("name=\"").unwrap() + 6;
    let end = start + line[start..].find('"').unwrap();
    &line[start..end]
}

fn window_rows(kind: &str) -> Vec<Vec<String>> {
    fixtures::rows("phase07_window")
        .into_iter()
        .filter_map(|row| row.strip_prefix("WINDOW|").map(str::to_owned))
        .map(|row| row.split('|').map(str::to_owned).collect::<Vec<_>>())
        .filter(|row| row[1] == kind)
        .collect()
}

// `phase07.rows` is generated from the run's T records. Re-deriving it here means the table is checked against the
// original, not only the C++ copy of it.
#[test]
fn phase07_rows_match_their_evidence() {
    let records = evidence();
    let tiles = expected_tiles();
    assert_eq!(tiles.len(), 14);
    for tile in &tiles {
        let dump_path = match tile.path.as_str() {
            "root" => "menuRoot.final".to_owned(),
            "strings" => "strings".to_owned(),
            "cursor" => "cursor".to_owned(),
            other => format!("menuRoot.final{}", other.strip_prefix("root").unwrap()),
        };
        let prefix = format!(" {dump_path} d");
        let line = records
            .iter()
            .rfind(|line| line.starts_with("T ") && line.contains(&prefix))
            .unwrap_or_else(|| panic!("no T record for {}", tile.path));
        assert_eq!(
            quoted_name(line).as_bytes(),
            &tile.name[..],
            "{}",
            tile.path
        );
        assert_eq!(
            u32::from_str_radix(field(line, "flags"), 16).unwrap(),
            tile.flags,
            "{}",
            tile.path
        );
        assert_eq!(field(line, "kids").parse::<usize>().unwrap(), tile.children);
        let values = &line[line.find("vals=[").unwrap() + 6..];
        for value in &tile.values {
            let key = format!("{:X}=", value.trait_id);
            let at = values
                .find(&format!(" {key}"))
                .map(|at| at + 1)
                .or_else(|| values.starts_with(&key).then_some(0))
                .unwrap_or_else(|| panic!("{} {key}", tile.path));
            let rest = &values[at + key.len()..];
            if value.number {
                assert_eq!(le_bits(&rest[1..9]), value.bits, "{} {key}", tile.path);
            } else {
                let text = value.text.as_ref().unwrap();
                let expected: String = text.iter().map(|&byte| char::from(byte)).collect();
                assert!(
                    rest.starts_with(&format!("\"{expected}\"")),
                    "{} {key}",
                    tile.path
                );
            }
        }
    }
}

#[test]
fn phase07_window_rows_match_their_evidence() {
    let records = evidence();
    let manager_line = records
        .iter()
        .find(|line| line.starts_with("M 138 final"))
        .unwrap();
    let dump = hex_bytes(field(manager_line, "bytes"));
    for row in window_rows("MANAGER") {
        let offset = usize::from_str_radix(&row[2], 16).unwrap();
        let bytes = hex_bytes(&row[3]);
        assert_eq!(
            &dump[offset..offset + bytes.len()],
            &bytes[..],
            "+{offset:X}"
        );
    }
    for row in window_rows("FONT") {
        let line = records
            .iter()
            .rfind(|line| line.starts_with("F ") && line.contains(&format!(" font{} ", row[2])))
            .unwrap();
        let raw = hex_bytes(field(line, "raw"));
        assert_eq!(&raw[0..2], &[1, 0], "loaded once");
        assert_eq!(raw[8] as usize, row[2].parse::<usize>().unwrap());
        let bits = |offset: usize| u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap());
        assert_eq!(bits(0x2C), u32::from_str_radix(&row[3], 16).unwrap());
        assert_eq!(bits(0x30), u32::from_str_radix(&row[4], 16).unwrap());
        let header = hex_bytes(field(line, "hdr"));
        let line_height = u32::from_le_bytes(header[0..4].try_into().unwrap());
        assert_eq!(line_height, u32::from_str_radix(&row[5], 16).unwrap());
        assert_eq!(u32::from_le_bytes(header[4..8].try_into().unwrap()), 1);
        assert!(header[0xC..].starts_with(row[6].as_bytes()));
    }
    for row in window_rows("FADER") {
        let line = records
            .iter()
            .find(|line| line.contains(" final.mgr+0/") && quoted_name_is(line, &row[2]))
            .unwrap();
        let translate = hex_bytes(field(line, "ltr"));
        assert_eq!(
            u32::from_le_bytes(translate[4..8].try_into().unwrap()),
            u32::from_str_radix(&row[3], 16).unwrap()
        );
        assert_eq!(
            le_bits(field(line, "scale")),
            u32::from_str_radix(&row[4], 16).unwrap()
        );
    }
    let menu_line = records
        .iter()
        .find(|line| line.starts_with("S 132 5ADEA0"))
        .unwrap();
    let menu = hex_bytes(field(menu_line, "menuRaw"));
    let row = &window_rows("MENU")[0];
    let word = |offset: usize| i32::from_le_bytes(menu[offset..offset + 4].try_into().unwrap());
    assert_eq!(word(0x14), row[2].parse::<i32>().unwrap());
    assert_eq!(word(0x18), row[3].parse::<i32>().unwrap());
    assert_eq!(i32::from(menu[0x1C]), row[4].parse::<i32>().unwrap());
    assert_eq!(word(0x20), row[5].parse::<i32>().unwrap());
    assert_eq!(word(0x24), row[9].parse::<i32>().unwrap());
    assert_eq!(word(0x28), row[6].parse::<i32>().unwrap());
    assert_eq!(word(0x3C), row[7].parse::<i32>().unwrap());
    assert_eq!(word(0x40) as u32, u32::from_str_radix(&row[8], 16).unwrap());
    let hits = records.iter().find(|line| line.starts_with("K ")).unwrap();
    for row in window_rows("HITS") {
        assert!(
            hits.contains(&format!("{}:{}", row[2], row[3])),
            "{}",
            row[2]
        );
    }
    let table = records
        .iter()
        .find(|line| line.starts_with("R 151 "))
        .unwrap();
    assert!(table.contains(&format!("[{}] tilemenu=", window_rows("TILEMENU")[0][2])));
}

fn quoted_name_is(line: &str, name: &str) -> bool {
    line.contains(&format!("name=\"{name}\""))
}

// ------------------------------------------------------------------------------------------------
// The host.

#[derive(Default)]
struct TestHost {
    archives: Option<(FileFinder, ArchiveManager)>,
    /// Files served without archives, for the fixtures of the failure paths.
    inline_files: Vec<(String, Vec<u8>)>,
    reads: InterfaceMovieReads,
    /// (file, whether 00B3A6E0 was published when it was read).
    reads_seen: Vec<(String, bool)>,
    renders: Vec<u32>,
    warnings: Vec<String>,
    debug: Vec<String>,
    movie_running: bool,
    sound_pauses: u32,
    tick: u32,
    tick_step: u32,
    unported_menu: Option<u32>,
    menu_stop: bool,
}

impl InterfaceHost for TestHost {
    fn open_font_file(&mut self, path: &str) -> Option<OpenedFile> {
        self.reads_seen
            .push((path.to_owned(), self.reads.interface_manager_present()));
        let (finder, archives) = self.archives.as_mut()?;
        bs_file_open_core(finder, Some(archives), path, 0, 0x2800, 0x80)
    }

    fn read_tile_file(&mut self, path: &str) -> Option<Vec<u8>> {
        self.reads_seen
            .push((path.to_owned(), self.reads.interface_manager_present()));
        if let Some((_, bytes)) = self.inline_files.iter().find(|(name, _)| name == path) {
            return Some(bytes.clone());
        }
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
        // The C++ harness's DdsImage: dwHeight at +0x0C, dwWidth at +0x10 of the DDS header.
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
        self.movie_running
    }

    fn render_interface(&mut self, argument: u32) {
        self.renders.push(argument);
    }

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
        self.tick += self.tick_step;
        self.tick
    }

    fn pause_game_sounds(&mut self) {
        self.sound_pauses += 1;
    }

    fn create_unported_menu(&mut self, menu_id: u32) -> Result<Option<MenuClass>, StreamError> {
        if self.menu_stop {
            return Err(StreamError("00587D20: this menu class is carried"));
        }
        Ok(self.unported_menu.map(|_| MenuClass::Unported(menu_id)))
    }
}

fn read_all(file: &mut OpenedFile) -> Vec<u8> {
    let mut bytes = vec![0; file.size() as usize];
    let _ = file.read(&mut bytes);
    bytes
}

fn settings_at(width: i32, height: i32) -> InterfaceSettings {
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    let scene = SceneGraphSettings {
        renderer_width: width,
        renderer_height: height,
        ..SceneGraphSettings::default()
    };
    // P07-RUN-001's first input byte has 8, and the display is windowed.
    let mut settings = interface_settings_from(&collections.ini, width, height, 8, scene);
    settings.full_screen = false;
    settings
}

fn prepared_world(settings: &InterfaceSettings, reads: &InterfaceMovieReads) -> World {
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
    world.insert_resource(reads.clone());
    world.insert_resource(GameSettingCollection::default());
    world
}

fn real_archives() -> (FileFinder, ArchiveManager) {
    let data = fixtures::data_dir();
    assert!(
        data.join("Oblivion - Misc.bsa").exists(),
        "this test needs the game data: set OBLIVION_DATA_DIR ({data:?})"
    );
    let root: PathBuf = data.parent().unwrap().canonicalize().unwrap();
    let scratch =
        std::env::temp_dir().join(format!("recomp_phase07_window_{}", std::process::id()));
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

fn tile_at(world: &World, path: &str) -> TileId {
    let manager = world.resource::<InterfaceManager>();
    let engine = world.resource::<TileEngine>();
    match path {
        "strings" => manager.strings_06c.unwrap(),
        "cursor" => manager.cursor_01c.unwrap(),
        _ => {
            let mut tile = manager.menu_root_068.unwrap();
            for index in path.split('/').skip(1) {
                let index: usize = index.parse().unwrap();
                tile = engine.tile(tile).children_30[index];
            }
            tile
        }
    }
}

const fn kind_name(type_id: u32) -> &'static str {
    match type_id {
        901 => "rect",
        902 => "image",
        903 => "text",
        905 => "menu",
        _ => "?",
    }
}

fn translate_bits(world: &World, node: Entity) -> [u32; 3] {
    world
        .get::<SceneNodePayload>(node)
        .unwrap()
        .local_transform
        .translation
        .map(f32::to_bits)
}

fn scene_node(world: &World, tile: TileId) -> Entity {
    world
        .resource::<TileEngine>()
        .tile(tile)
        .scene_24
        .as_ref()
        .and_then(|record| record.node)
        .unwrap()
}

fn geometry(world: &World, tile: TileId) -> Vec<TileSceneGeometry> {
    world
        .resource::<TileEngine>()
        .tile(tile)
        .scene_24
        .as_ref()
        .unwrap()
        .geometry
        .clone()
}

fn name_of(world: &World, entity: Entity) -> Option<Vec<u8>> {
    world.get::<NiObjectNet>(entity).unwrap().name.clone()
}

fn check_manager_rows(world: &World) {
    let manager = world.resource::<InterfaceManager>();
    for row in window_rows("MANAGER") {
        let bytes = hex_bytes(&row[3]);
        let actual: Vec<u8> = match row[2].as_str() {
            "008" => vec![manager.menu_loading_status_008],
            "009" => vec![manager.menu_transition_in_progress_009],
            "00A" => manager.tab_ids_00a.to_vec(),
            "010" => manager.unknown_010.to_le_bytes().to_vec(),
            "074" => manager.camera_distance_074.to_le_bytes().to_vec(),
            "07C" => vec![manager.geometry_changed_07c],
            "08C" => manager
                .default_selection_priority_08c
                .to_le_bytes()
                .to_vec(),
            "0B0" => vec![manager.msg_box_button_pressed_0b0],
            "0B8" => vec![manager.purge_textures_0b8],
            "0B9" => vec![manager.cursor_moved_0b9],
            "0E0" => manager.menu_mode_stack_0e0[0].to_le_bytes().to_vec(),
            "0E4" => manager.menu_mode_stack_0e0[1].to_le_bytes().to_vec(),
            other => panic!("no field for +{other}"),
        };
        assert_eq!(actual, bytes, "InterfaceManager +{}", row[2]);
    }
    assert_eq!(
        manager.block_130,
        Some(InterfaceBlock130 {
            unknown_0: 0,
            unknown_4: 0.0,
            unknown_8: 0.001,
            self_c: true,
            unknown_10: 0,
            nodes: Vec::new(),
            blend_progress: 0.0,
        })
    );
    assert_eq!(manager.look_at_108, Some(InterfaceLookAtHelper::default()));
    assert_eq!(manager.cursor_state_020, [0.0; 6]);
    assert_eq!(
        world.resource::<MenuModeChanged>().0,
        1,
        "00B42D54, set by 1007 (C++ only)"
    );
    assert!(world.resource::<TileEngine>().menu_array_allocated_b3a708);
    let alpha = world
        .get::<AlphaProperty>(manager.alpha_property_078.unwrap())
        .unwrap();
    assert_eq!(alpha.flags_18, 0x00ED, "alpha property flags (N 359)");
}

fn check_tile_rows(world: &World) {
    let engine = world.resource::<TileEngine>();
    for expected in expected_tiles() {
        let id = tile_at(world, &expected.path);
        let tile = engine.tile(id);
        let site = format!("{} ({})", expected.path, lift_text(&expected.name));
        assert_eq!(kind_name(tile.type_id()), expected.kind, "{site}: kind");
        assert_eq!(tile.name_08, expected.name, "{site}: name");
        assert_eq!(tile.flags_2c, expected.flags, "{site}: flags");
        assert_eq!(
            tile.children_30.len(),
            expected.children,
            "{site}: children"
        );
        // The P07-RUN-001 dumper stops after 80 values (the strings tile).
        if expected.values.len() == 80 {
            assert!(tile.values_14.len() >= 80, "{site}: value count");
        } else {
            assert_eq!(
                tile.values_14.len(),
                expected.values.len(),
                "{site}: value count"
            );
        }
        // In the original's order, so a trait can't pass as its neighbour with the same value.
        let order: Vec<u32> = tile
            .values_14
            .iter()
            .take(expected.values.len())
            .map(|&id| u32::from(engine.value(id).trait_18))
            .collect();
        let wanted: Vec<u32> = expected.values.iter().map(|value| value.trait_id).collect();
        assert_eq!(order, wanted, "{site}: trait order");
        for value in &expected.values {
            let stored = tile
                .values_14
                .iter()
                .map(|&id| engine.value(id))
                .find(|stored| u32::from(stored.trait_18) == value.trait_id)
                .unwrap_or_else(|| panic!("{site}: trait {:X} missing", value.trait_id));
            if value.number {
                assert!(
                    stored.is_number_1a,
                    "{site}: trait {:X} kind",
                    value.trait_id
                );
                assert_eq!(
                    stored.number_04.to_bits(),
                    value.bits,
                    "{site}: trait {:X}",
                    value.trait_id
                );
            } else {
                assert!(
                    !stored.is_number_1a,
                    "{site}: trait {:X} kind",
                    value.trait_id
                );
                assert_eq!(
                    Some(&stored.text_08),
                    value.text.as_ref(),
                    "{site}: trait {:X}",
                    value.trait_id
                );
            }
        }
    }
}

/// The N records of each tile node: name, flags, translate, scale, and the leaf shape's vtable and translate. The C++
/// test compares five of these by hand.
fn check_tile_nodes(world: &World) {
    let records = evidence();
    let mut paths: Vec<String> = expected_tiles()
        .into_iter()
        .map(|tile| tile.path)
        .filter(|path| path != "strings")
        .collect();
    paths.sort();
    let mut compared = 0;
    for path in paths {
        let dump_path = match path.as_str() {
            "root" => "menuRoot.final".to_owned(),
            "cursor" => "cursor".to_owned(),
            other => format!("menuRoot.final{}", other.strip_prefix("root").unwrap()),
        };
        let node_prefix = format!(" {dump_path}.n d0 ");
        let line = records
            .iter()
            .find(|line| line.starts_with("N ") && line.contains(&node_prefix))
            .unwrap_or_else(|| panic!("no N record for {path}"));
        let tile = tile_at(world, &path);
        let node = scene_node(world, tile);
        let payload = world.get::<SceneNodePayload>(node).unwrap();
        let name = quoted_name(line);
        let stored = name_of(world, node);
        if name == "<null>" {
            assert_eq!(stored, None, "{path}: node name");
        } else {
            assert_eq!(
                stored.as_deref(),
                Some(name.as_bytes()),
                "{path}: node name"
            );
        }
        assert_eq!(
            u16::from_str_radix(field(line, "flags"), 16).unwrap(),
            payload.effective_av_flags,
            "{path}: node flags"
        );
        let translate = hex_bytes(field(line, "ltr"));
        assert_eq!(
            payload
                .local_transform
                .translation
                .map(f32::to_le_bytes)
                .concat(),
            translate,
            "{path}: node translate"
        );
        assert_eq!(
            payload.local_transform.scale.to_bits(),
            le_bits(field(line, "scale")),
            "{path}: node scale"
        );
        // The first leaf: NiTriShape or BSScissorTriShape at .n/0 (the node's child list holds it in the original; D3
        // keeps it as a record).
        let shape_prefix = format!(" {dump_path}.n/0 d1 ");
        let shape = records
            .iter()
            .find(|line| line.starts_with("N ") && line.contains(&shape_prefix))
            .unwrap_or_else(|| panic!("no shape record for {path}"));
        let record = geometry(world, tile);
        let leaf = record
            .first()
            .unwrap_or_else(|| panic!("{path}: no geometry"));
        let vtable = shape.split(' ').nth(5).unwrap().split(':').next().unwrap();
        assert_eq!(
            format!("{:X}", leaf.original_vtable),
            vtable,
            "{path}: shape vtable"
        );
        let translate = hex_bytes(field(shape, "ltr"));
        assert_eq!(
            leaf.translate_54.map(f32::to_le_bytes).concat(),
            translate,
            "{path}: shape translate"
        );
        compared += 1;
    }
    assert_eq!(compared, 13);
}

fn check_fonts(world: &World) {
    let fonts = world.resource::<FontManager>();
    assert_eq!(fonts.unknown_14, 0);
    for row in window_rows("FONT") {
        let id: usize = row[2].parse().unwrap();
        let font = fonts.font_infos_00[id - 1].as_ref().unwrap();
        assert_eq!(font.load_count_00, 1, "font {id}");
        assert_eq!(font.id_08 as usize, id, "font {id}");
        assert_eq!(
            font.top_extent_2c.to_bits(),
            u32::from_str_radix(&row[3], 16).unwrap(),
            "font {id} +0x2C"
        );
        assert_eq!(
            font.bottom_extent_30.to_bits(),
            u32::from_str_radix(&row[4], 16).unwrap(),
            "font {id} +0x30"
        );
        assert_eq!(
            font_data_float(&font.font_data_38, 0).to_bits(),
            u32::from_str_radix(&row[5], 16).unwrap(),
            "font {id} data +0"
        );
        assert_eq!(font_data_texture_count(&font.font_data_38), 1, "font {id}");
        assert!(font.font_data_38[0xC..].starts_with(row[6].as_bytes()));
        let texture = font.textures_0c[0].unwrap_or_else(|| panic!("font {id}: texture"));
        let pixels = world.get::<PixelData>(texture).unwrap();
        assert!(!pixels.pixels_50.is_empty(), "font {id}: pixels");
        assert!(font.textures_0c[1].is_none());
    }
}

/// The rect and image shapes' materials against the N prop records: ambient, diffuse and specular bits, and IDs relative to the first (the dump's raw stops at +0x40). Not in the C++ test. The IDs' base depends on earlier phases, so only the order is compared.
fn check_tile_materials(world: &World) {
    let records = evidence();
    let mut first: Option<(u32, u32)> = None;
    for expected in expected_tiles() {
        if expected.path == "strings" || expected.kind == "text" {
            continue;
        }
        let dump_path = match expected.path.as_str() {
            "root" => "menuRoot.final".to_owned(),
            "cursor" => "cursor".to_owned(),
            other => format!("menuRoot.final{}", other.strip_prefix("root").unwrap()),
        };
        let prefix = format!(" {dump_path}.n/0  prop ");
        let line = records
            .iter()
            .find(|line| line.contains(&prefix) && line.contains("A7E1F4:"))
            .unwrap_or_else(|| panic!("no material for {}", expected.path));
        let raw = hex_bytes(field(line, "raw"));
        let leaf = geometry(world, tile_at(world, &expected.path)).remove(0);
        assert!(leaf.has_material, "{}", expected.path);
        let run_id = u32::from_le_bytes(raw[0x18..0x1C].try_into().unwrap());
        let (run_base, port_base) = *first.get_or_insert((run_id, leaf.material_id_18));
        assert_eq!(
            run_id - run_base,
            leaf.material_id_18 - port_base,
            "{}: material ID order",
            expected.path
        );
        let colors = [&leaf.ambient, &leaf.diffuse, &leaf.specular]
            .map(|color| color.map(f32::to_le_bytes).concat())
            .concat();
        assert_eq!(
            &raw[0x1C..0x40],
            &colors[..],
            "{}: material colours",
            expected.path
        );
        if expected.kind == "image" {
            let texturing = records
                .iter()
                .find(|line| line.contains(&prefix) && line.contains("A7DB5C:"))
                .unwrap_or_else(|| panic!("no texturing for {}", expected.path));
            assert_eq!(
                u32::from(leaf.texturing_flags),
                le_bits(field(texturing, "flags")),
                "{}: texturing flags",
                expected.path
            );
        }
    }
}

/// The two scene graphs' child slots, property lists and camera rotation against the final N records. Not in the C++ test, apart from the Main Root slots.
fn check_scene_graphs(world: &World) {
    let records = evidence();
    let manager = world.resource::<InterfaceManager>();
    let graphs = [
        (
            "final.mgr+0",
            manager.menu_scene_graph_000.unwrap(),
            &["A3D604", "A2FDB4"][..],
        ),
        (
            "final.mgr+4",
            manager.menu_3d_scene_graph_004.unwrap(),
            &["A90584", "A3D604", "A2FDB4"][..],
        ),
    ];
    for (dump, graph, properties) in graphs {
        // Properties, head first, by vtable.
        let prefix = format!(" {dump}  prop ");
        let vtables: Vec<&str> = records
            .iter()
            .filter(|line| line.contains(&prefix))
            .map(|line| line.split(' ').nth(6).unwrap().split(':').next().unwrap())
            .collect();
        assert_eq!(vtables, properties, "{dump}: the record's properties");
        let held = &world.get::<NiAvProperties>(graph).unwrap().0;
        assert_eq!(held.len(), properties.len(), "{dump}");
        for (&property, &vtable) in held.iter().zip(properties) {
            let entity = world.entity(property.unwrap());
            let matches = match vtable {
                "A3D604" => entity.contains::<AlphaProperty>(),
                "A2FDB4" => entity.contains::<ZBufferProperty>(),
                _ => entity.contains::<FogProperty>(),
            };
            assert!(matches, "{dump}: property {vtable}");
        }
        // Child slots: the record's names, where the dump read one (it shows "" for a few objects that have names).
        let slots = &world.get::<NiNodeChildren>(graph).unwrap().slots_ac;
        let children_prefix = format!(" {dump}  children ");
        let line = records
            .iter()
            .find(|line| line.contains(&children_prefix))
            .unwrap();
        let sizes = hex_bytes(field(line, "b4"));
        assert_eq!(slots.len(), usize::from(sizes[0]), "{dump}: child count");
        for (index, slot) in slots.iter().enumerate() {
            let prefix = format!(" {dump}/{index} d1 ");
            let line = records
                .iter()
                .find(|line| line.contains(&prefix))
                .unwrap_or_else(|| panic!("{dump}/{index}"));
            let name = quoted_name(line);
            if !name.is_empty() {
                assert_eq!(
                    name_of(world, slot.unwrap()).as_deref(),
                    Some(name.as_bytes()),
                    "{dump}/{index}"
                );
            }
        }
        // The camera at slot 0's slot 0: 0057ED86's rotation.
        let prefix = format!(" {dump}/0/0 d2 ");
        let line = records.iter().find(|line| line.contains(&prefix)).unwrap();
        let camera = world.get::<SceneGraph>(graph).unwrap().camera_dc.unwrap();
        let rotation = world
            .get::<SceneCamera>(camera)
            .unwrap()
            .local_transform_30
            .rotation;
        assert_eq!(
            rotation.map(f32::to_le_bytes).concat(),
            hex_bytes(field(line, "lrot")),
            "{dump}: camera rotation"
        );
    }
    // 0057ED20's vertex colour property on the two roots.
    for root in [
        manager.main_root_054.unwrap(),
        manager.cursor_root_058.unwrap(),
    ] {
        let held = &world.get::<NiAvProperties>(root).unwrap().0;
        let property = world.get::<VertexColorProperty>(held[0].unwrap()).unwrap();
        assert_eq!(property.flags_18, 0x28, "N 370, N 384");
    }
}

// The C++ `verify_window`: 00582160 at 1920x1080 against P07-RUN-001. All one test, as a window's later values depend
// on its earlier ones.
#[test]
fn phase07_window_against_p07_run_001() {
    let settings = settings_at(1920, 1080);
    let reads = InterfaceMovieReads::default();
    let mut world = prepared_world(&settings, &reads);
    let mut host = TestHost {
        archives: Some(real_archives()),
        reads: reads.clone(),
        tick: 1000,
        ..TestHost::default()
    };
    run_winmain_interface_window(&mut world, &settings, &mut host, &mut NoSceneControllers)
        .unwrap();

    // 00B3A6E0 published, and the D6 reads agree.
    assert!(world.contains_resource::<InterfaceManager>());
    assert!(reads.interface_manager_present());
    assert!(reads.tile_menu_present(1007));
    assert!(!reads.tile_menu_present(MENU_ID_MESSAGE));
    // The publish comes after the ctor's reads (the fonts) and before the initializer's (strings.xml): 005821C7.
    let first_tile_read = host
        .reads_seen
        .iter()
        .position(|(path, _)| path.ends_with(".xml"))
        .unwrap();
    assert_eq!(
        host.reads_seen[first_tile_read].0,
        "Data\\Menus\\strings.xml"
    );
    assert!(
        host.reads_seen[first_tile_read].1,
        "published before 00581CC0"
    );
    assert_eq!(
        host.reads_seen
            .iter()
            .filter(|(path, seen)| !seen && (path.ends_with(".fnt") || path.ends_with(".tex")))
            .count(),
        10,
        "five .fnt and five .tex before the publish"
    );
    assert!(
        host.reads_seen[..first_tile_read]
            .iter()
            .all(|(_, seen)| !seen),
        "nothing before the fonts sees the manager"
    );

    check_manager_rows(&world);
    check_fonts(&world);
    check_tile_rows(&world);
    check_tile_nodes(&world);
    check_tile_materials(&world);
    check_scene_graphs(&world);

    // Faders (the C++ fader check, against the run's N records through phase07_window.rows).
    let faders = *world.resource::<RendererUiFaderNodes>();
    for row in window_rows("FADER") {
        let node = if row[2].contains("Below") {
            faders.below_00.unwrap()
        } else {
            faders.above_04.unwrap()
        };
        assert_eq!(name_of(&world, node).as_deref(), Some(row[2].as_bytes()));
        let payload = world.get::<SceneNodePayload>(node).unwrap();
        assert_eq!(
            payload.local_transform.translation[1].to_bits(),
            u32::from_str_radix(&row[3], 16).unwrap()
        );
        assert_eq!(
            payload.local_transform.scale.to_bits(),
            u32::from_str_radix(&row[4], 16).unwrap()
        );
    }

    // The node tree.
    let manager = world.resource::<InterfaceManager>().clone();
    let main_root = manager.main_root_054.unwrap();
    let root_node = scene_node(&world, manager.menu_root_068.unwrap());
    assert_eq!(
        world.get::<NiAvParent>(root_node).unwrap().0,
        Some(main_root),
        "MenuRoot node under Main Root"
    );
    let slots = &world.get::<NiNodeChildren>(main_root).unwrap().slots_ac;
    assert_eq!(
        slots.len(),
        3,
        "Main Root slots: light, MenuRoot, DebugText (cursor slot removed)"
    );
    assert_eq!(slots[1], Some(root_node));
    assert_eq!(slots[2], manager.debug_text_root_070);
    assert_eq!(
        translate_bits(&world, root_node),
        [0, 0x8000_0000, 0x8000_0000]
    );
    let strings = manager.strings_06c.unwrap();
    assert_eq!(
        translate_bits(&world, scene_node(&world, strings)),
        [0xC455_5555, 0x8000_0000, 0x43F0_0000],
        "Strings translate"
    );
    let loading = tile_at(&world, "root/0");
    assert_eq!(
        translate_bits(&world, scene_node(&world, loading)),
        [0xC455_5555, 0xBC83_126F, 0x43F0_0000],
        "LoadingMenu translate"
    );
    let cursor = manager.cursor_01c.unwrap();
    let cursor_node = scene_node(&world, cursor);
    assert_eq!(
        world.get::<NiAvParent>(cursor_node).unwrap().0,
        manager.cursor_root_058
    );
    assert_eq!(
        world
            .get::<SceneNodePayload>(cursor_node)
            .unwrap()
            .effective_av_flags,
        0x000F
    );
    assert_eq!(translate_bits(&world, cursor_node), [0, 0xBF61_47AF, 0]);
    let cursor_properties = &world.get::<NiAvProperties>(cursor_node).unwrap().0;
    assert_eq!(cursor_properties[0], manager.alpha_property_078, "N 353");
    let main = scene_node(&world, tile_at(&world, "root/0/2"));
    assert_eq!(
        world
            .get::<SceneNodePayload>(main)
            .unwrap()
            .effective_av_flags,
        0x000F,
        "load_main culled"
    );
    let debug_text = tile_at(&world, "root/0/0");
    assert_eq!(name_of(&world, scene_node(&world, debug_text)), None);
    let shapes = geometry(&world, debug_text);
    assert_eq!(shapes.len(), 1);
    assert_eq!(shapes[0].original_vtable, TILE_SCISSOR_TRI_SHAPE_VTABLE);
    assert_eq!(shapes[0].translate_54[0].to_bits(), 0x4270_0000);
    assert_eq!(shapes[0].translate_54[2].to_bits(), 0xC45D_4000);

    // The menu registry and the menu (S 132).
    let engine = world.resource::<TileEngine>();
    assert_eq!(engine.tile_menu_by_id(1007), Some(loading));
    assert_eq!(engine.tile_menu_by_id(1001), None);
    let slot: usize = window_rows("TILEMENU")[0][2].parse().unwrap();
    assert_eq!(engine.tile_menus[slot], Some(loading));
    let menu = engine.menu(engine.containing_menu(loading).unwrap());
    let row = &window_rows("MENU")[0];
    assert_eq!(menu.menu_mode_14, row[2].parse::<u32>().unwrap());
    assert_eq!(menu.depth_18, row[3].parse::<i32>().unwrap(), "menu +0x18");
    assert_eq!(
        i32::from(menu.owns_templates_1c),
        row[4].parse::<i32>().unwrap()
    );
    assert_eq!(menu.id_20, row[5].parse::<u32>().unwrap());
    assert_eq!(menu.state_24, row[9].parse::<u32>().unwrap(), "menu +0x24");
    assert_eq!(menu.tile_04, Some(loading));
    let MenuClass::Loading(state) = &menu.class else {
        panic!("not a LoadingMenu");
    };
    assert_eq!(state.current_section_28, row[6].parse::<i32>().unwrap());
    assert_eq!(state.target_3c, row[7].parse::<i32>().unwrap());
    assert_eq!(
        state.slide_remaining_40.to_bits(),
        u32::from_str_radix(&row[8], 16).unwrap()
    );
    assert_eq!(state.slide_context_44, 0);
    assert_eq!(state.last_step_68, 1000);
    assert_eq!(host.sound_pauses, 1, "005AD158");

    // The gates: two 00579260 and two 0057F170 (K 405).
    let hits = |address: &str| {
        window_rows("HITS")
            .into_iter()
            .find(|row| row[2] == address)
            .unwrap()[3]
            .parse::<usize>()
            .unwrap()
    };
    assert_eq!(host.renders, vec![0; hits("57F170")]);
    assert_eq!(hits("579260"), hits("57F170"));
    // 005AD0D0 LoadingMenu_ctor.
    assert_eq!(
        engine
            .menus
            .iter()
            .filter(|menu| matches!(menu.class, MenuClass::Loading(_)))
            .count(),
        hits("5AD0D0")
    );

    // P08-RUN-002 (the C++'s comments): the Phase 07 shapes took 15 data IDs from a fresh counter.
    assert_eq!(world.resource::<GeometryDataIdCounter>().0, 0x10);

    // No warning on the way.
    assert_eq!(host.warnings, Vec::<String>::new());
}

// user3 (0xFB1) drives the ribbon through the value engine. The C++ checks this through its host projection: 50 ticks
// of 101 ms, ribbon width 90 (loading_menu_tests). Against the C++ only.
#[test]
fn the_ribbon_follows_the_progress() {
    let settings = settings_at(1920, 1080);
    let reads = InterfaceMovieReads::default();
    let mut world = prepared_world(&settings, &reads);
    let mut host = TestHost {
        archives: Some(real_archives()),
        reads,
        ..TestHost::default()
    };
    run_winmain_interface_window(&mut world, &settings, &mut host, &mut NoSceneControllers)
        .unwrap();
    let loading = tile_at(&world, "root/0");
    let ribbon = tile_at(&world, "root/0/1/1");
    let menu = world
        .resource::<TileEngine>()
        .containing_menu(loading)
        .unwrap();
    let mut runtime = NoSceneControllers;
    host.tick_step = 101;
    with_interface_cx(&mut world, &settings, &mut host, &mut runtime, |cx| {
        assert_eq!(
            tile_numeric_trait(cx, ribbon, TRAIT_WIDTH),
            1.0,
            "Zero ribbon minimum"
        );
        let MenuClass::Loading(state) = &mut cx.engine.menu_mut(menu).class else {
            panic!("not a LoadingMenu");
        };
        state.set_section_progress(3, 100.0).unwrap();
        for _ in 0..50 {
            loading_menu_update_display(cx, false).unwrap();
        }
        assert_eq!(tile_numeric_trait(cx, loading, TRAIT_USER3), 50.0);
        assert_eq!(
            tile_numeric_trait(cx, ribbon, TRAIT_WIDTH),
            90.0,
            "Visible tick XML projection"
        );
        // Slides: the empty list keeps user0 at 0.
        assert_eq!(tile_numeric_trait(cx, loading, TRAIT_USER0), 0.0);
    });
}

// The C++ `verify_aspect`. Against the C++ only: the run has one size and all four safe-zone settings 20.
#[test]
fn aspect_table() {
    let check = |width, height, w: f32, h: f32| {
        let settings = settings_at(width, height);
        assert_eq!(
            ui_virtual_width(&settings).to_bits(),
            w.to_bits(),
            "{width}x{height}"
        );
        assert_eq!(
            ui_virtual_height(&settings).to_bits(),
            h.to_bits(),
            "{width}x{height}"
        );
    };
    check(1920, 1080, 1706.6666, 960.0);
    check(1280, 1024, 1200.0, 960.0);
    check(1024, 768, 1280.0, 960.0);
    check(768, 1024, 1280.0, 1024.0 / 768.0 * 1280.0);
    let mut settings = settings_at(1920, 1080);
    settings.safe_zone = [20, 21, 22, 23];
    assert_eq!(
        interface_safe_zone_x(&settings),
        22.0,
        "wide safe zone (H/W < 0.74)"
    );
    assert_eq!(interface_safe_zone_y(&settings), 23.0);
    settings.renderer_width = 1024;
    settings.renderer_height = 768;
    assert_eq!(interface_safe_zone_x(&settings), 20.0, "4:3 safe zone");
    assert_eq!(interface_safe_zone_y(&settings), 21.0);
}

const MINIMAL_LOADING: &str = "<menu name=\"LoadingMenu\"><class>&LoadingMenu;</class>\
    <stackingtype>&no_click_past;</stackingtype><depth>3</depth>\
    <rect name=\"child\"><width>100</width><height><copy src=\"parent()\" trait=\"width\"/><div>2</div></height></rect></menu>";

fn minimal_world(host: &mut TestHost, settings: &InterfaceSettings) -> World {
    let reads = InterfaceMovieReads::default();
    host.reads = reads.clone();
    prepared_world(settings, &reads)
}

// Without files: every font warns and stays unloaded, strings.xml and loading_menu.xml fail to parse, and the window
// still ends. The cursor's node is built anyway; its texture is simply not found. (Handoff 07-G-3.)
#[test]
fn missing_files_follow_the_failure_paths() {
    let settings = settings_at(1920, 1080);
    let mut host = TestHost::default();
    let mut world = minimal_world(&mut host, &settings);
    run_winmain_interface_window(&mut world, &settings, &mut host, &mut NoSceneControllers)
        .unwrap();
    let fonts = world.resource::<FontManager>();
    for font in &fonts.font_infos_00 {
        let font = font.as_ref().unwrap();
        assert_eq!(font.load_count_00, 0, "loadedSuccessfully stays 0");
        assert!(font.font_data_38.is_empty());
    }
    assert_eq!(
        host.warnings
            .iter()
            .filter(|text| text.starts_with("Font file not found: "))
            .count(),
        5
    );
    assert!(
        host.warnings
            .iter()
            .any(|text| text.contains("strings.xml"))
    );
    assert!(
        host.warnings
            .iter()
            .any(|text| text.contains("loading_menu.xml"))
    );
    let manager = world.resource::<InterfaceManager>();
    assert_eq!(manager.strings_06c, None);
    assert!(manager.cursor_01c.is_some());
    assert_eq!(manager.menu_loading_status_008, 1, "005ADDF0 not reached");
    assert_eq!(world.resource::<TileEngine>().tile_menu_by_id(1007), None);
    assert!(
        host.renders.is_empty(),
        "the null return comes before the gates"
    );
}

fn minimal_window(host: &mut TestHost, class_text: Option<&str>) -> (World, InterfaceSettings) {
    let settings = settings_at(1920, 1080);
    let xml = class_text.map_or_else(
        || MINIMAL_LOADING.to_owned(),
        |class| MINIMAL_LOADING.replace("&LoadingMenu;", class),
    );
    host.inline_files = vec![("Data\\Menus\\loading_menu.xml".to_owned(), xml.into_bytes())];
    let mut world = minimal_world(host, &settings);
    run_winmain_interface_window(&mut world, &settings, host, &mut NoSceneControllers).unwrap();
    (world, settings)
}

// 005ADD4F: the XML's menu has another class, so the create returns null. Its 1001 slot is registered, which the movie
// thread's read sees.
#[test]
fn a_wrong_menu_class_returns_null() {
    let mut host = TestHost {
        unported_menu: Some(1001),
        ..TestHost::default()
    };
    let (world, _) = minimal_window(&mut host, Some("&MessageMenu;"));
    let manager = world.resource::<InterfaceManager>();
    assert_eq!(manager.menu_loading_status_008, 1);
    assert!(host.renders.is_empty());
    assert!(host.reads.tile_menu_present(1001));
    assert!(!host.reads.tile_menu_present(1007));
}

// 00587D20's other classes: a host stop by default, and a null class logs and leaves the menu out of the table. The
// ContainerMenu (1008) stands for an unported class, as the InventoryMenu did before Build 1 ported it.
#[test]
fn an_unported_menu_class_stops_or_logs() {
    let mut host = TestHost {
        menu_stop: true,
        ..TestHost::default()
    };
    let settings = settings_at(1920, 1080);
    host.inline_files = vec![(
        "Data\\Menus\\loading_menu.xml".to_owned(),
        MINIMAL_LOADING
            .replace("&LoadingMenu;", "&ContainerMenu;")
            .into_bytes(),
    )];
    let mut world = minimal_world(&mut host, &settings);
    assert!(
        run_winmain_interface_window(&mut world, &settings, &mut host, &mut NoSceneControllers)
            .is_err()
    );

    let mut host = TestHost::default();
    let (_, _) = minimal_window(&mut host, Some("&ContainerMenu;"));
    assert!(
        host.debug
            .iter()
            .any(|text| text == "ERROR: Cant find menu class!")
    );
    assert!(!host.reads.tile_menu_present(1008));
}

// 00579260 / 005792B0: no render while the movie runs; a render clears +0x7C; no cursor means neither.
#[test]
fn the_movie_gates() {
    let mut host = TestHost {
        movie_running: true,
        ..TestHost::default()
    };
    let (mut world, settings) = minimal_window(&mut host, None);
    assert!(host.renders.is_empty(), "the movie is running");
    assert_eq!(world.resource::<InterfaceManager>().geometry_changed_07c, 1);
    with_interface_cx(
        &mut world,
        &settings,
        &mut host,
        &mut NoSceneControllers,
        |cx| {
            assert!(interface_movie_gate(cx));
        },
    );
    host.movie_running = false;
    with_interface_cx(
        &mut world,
        &settings,
        &mut host,
        &mut NoSceneControllers,
        |cx| {
            assert!(!interface_movie_gate(cx));
            interface_render_if_no_movie(cx, 7).unwrap();
        },
    );
    assert_eq!(host.renders, [7]);
    assert_eq!(world.resource::<InterfaceManager>().geometry_changed_07c, 0);
    world.resource_mut::<InterfaceManager>().cursor_01c = None;
    host.movie_running = true;
    with_interface_cx(
        &mut world,
        &settings,
        &mut host,
        &mut NoSceneControllers,
        |cx| {
            assert!(
                !interface_movie_gate(cx),
                "no cursor: false, IsRunning not asked"
            );
            interface_render_if_no_movie(cx, 9).unwrap();
        },
    );
    assert_eq!(host.renders, [7]);
}

// The publish needs the MoviePlayer's clone of the reads in the World.
#[test]
fn the_movie_reads_must_be_in_the_world() {
    let settings = settings_at(1920, 1080);
    let mut host = TestHost::default();
    let mut world = minimal_world(&mut host, &settings);
    world.remove_resource::<InterfaceMovieReads>();
    assert!(
        run_winmain_interface_window(&mut world, &settings, &mut host, &mut NoSceneControllers)
            .is_err()
    );
    assert!(!world.contains_resource::<InterfaceManager>());
}

// 00582160(0, 1): the getter never creates; a second create does nothing.
#[test]
fn the_getter_never_creates() {
    let settings = settings_at(1920, 1080);
    let mut host = TestHost::default();
    let mut world = minimal_world(&mut host, &settings);
    assert!(
        !run_interface_manager_get_singleton(
            &mut world,
            &settings,
            &mut host,
            &mut NoSceneControllers,
            false,
            true
        )
        .unwrap()
    );
    assert!(!host.reads.interface_manager_present());
    run_winmain_interface_window(&mut world, &settings, &mut host, &mut NoSceneControllers)
        .unwrap();
    let tiles = world.resource::<TileEngine>().tiles.len();
    assert!(
        run_interface_manager_get_singleton(
            &mut world,
            &settings,
            &mut host,
            &mut NoSceneControllers,
            true,
            true
        )
        .unwrap()
    );
    assert_eq!(world.resource::<TileEngine>().tiles.len(), tiles);
}

// The XML engine on inline text: 0058B800's parent(), the copy action, div, stacking 102 -> 6006, the value list in
// trait order and the child count.
#[test]
fn the_engine_on_a_small_menu() {
    let mut host = TestHost::default();
    let (world, _) = minimal_window(&mut host, None);
    let engine = world.resource::<TileEngine>();
    let menu = engine.tile_menu_by_id(1007).unwrap();
    assert_eq!(engine.tile(menu).name_08, b"LoadingMenu");
    let child = engine.tile(menu).children_30[0];
    let trait_of = |tile: TileId, id: u16| {
        engine
            .tile(tile)
            .values_14
            .iter()
            .map(|&value| engine.value(value))
            .find(|value| value.trait_18 == id)
            .map(|value| value.number_04)
    };
    assert_eq!(trait_of(child, TRAIT_WIDTH), Some(100.0));
    // The source is the menu, whose width is 0: copy 0, then div 2.
    assert_eq!(trait_of(child, TRAIT_HEIGHT), Some(0.0));
    assert_eq!(trait_of(menu, TRAIT_STACKING), Some(6006.0));
    assert_eq!(trait_of(menu, TRAIT_CHILD_COUNT), Some(1.0));
    let traits: Vec<u16> = engine
        .tile(menu)
        .values_14
        .iter()
        .map(|&value| engine.value(value).trait_18)
        .collect();
    let mut sorted = traits.clone();
    sorted.sort_unstable();
    assert_eq!(traits, sorted);
}

// 0058DBD0's errors set 00B3B0A0, so 00590420 returns null. An include of a missing prefab splices nothing (recorded
// host choice).
#[test]
fn xml_errors_fail_the_read() {
    let settings = settings_at(1920, 1080);
    let mut host = TestHost {
        inline_files: vec![
            ("bad.xml".to_owned(), b"<rect><></rect>".to_vec()),
            (
                "include.xml".to_owned(),
                b"<rect name=\"a\"><include src=\"none.xml\"/><width>5</width></rect>".to_vec(),
            ),
        ],
        ..TestHost::default()
    };
    let mut world = minimal_world(&mut host, &settings);
    run_winmain_interface_window(&mut world, &settings, &mut host, &mut NoSceneControllers)
        .unwrap();
    with_interface_cx(
        &mut world,
        &settings,
        &mut host,
        &mut NoSceneControllers,
        |cx| {
            assert_eq!(tile_read_xml(cx, None, "bad.xml").unwrap(), None);
            let tile = tile_read_xml(cx, None, "include.xml").unwrap().unwrap();
            assert_eq!(tile_numeric_trait(cx, tile, TRAIT_WIDTH), 5.0);
        },
    );
    assert!(
        host.warnings
            .iter()
            .any(|text| text.contains("Empty tag name"))
    );
}
