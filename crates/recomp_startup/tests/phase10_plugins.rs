//! Phase 10: the plugin selection of 00406360 (00406687..00406773) against the P10-RUN-001 dump, over the shipped Data list read only, the pinned INI and the pinned Plugins.txt the run itself read. Then the arms
//! the run did not take, which only the C++ port has exercised: Plugins.txt missing or empty, no matching line, the Oblivion.esm fallback with and without Oblivion.esm, test-file matches, the CRT text-mode line
//! rules, 00447C50's walk, 0044FAD0's clear arm, the null-DataHandler host stop and the FileName overflow report. Host tests earn no validation bonus.
//!
//! `every_row_is_read` fails if a line of `phase10.rows` goes unread. It also reads `records.txt` for what the rows do not carry: FileName, the line buffer of each match and the Plugins.txt hash. `phase10.rows` is the
//! imported `.inc`, so the hand-held values are in `phase10_run001.rows`. Fixture INI and
//! Plugins.txt files live in temp directories, never the Documents folder or the user's files.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy_ecs::prelude::*;
use recomp_core::crt::stricmp;
use recomp_core::fixtures;
use recomp_game::tes::data_handler::{
    DataHandler, build_data_handler_file_list, construct_data_handler, lookup_file_by_name,
};
use recomp_game::tes::paths::GamePaths;
use recomp_game::tes::settings::{SettingCollections, register_static_settings};
use recomp_game::tes::tes_file::TesFileData;
use recomp_startup::initialize_game_data::{PluginBlockInputs, run_plugin_block};
use recomp_startup::path_setup::original_plugins_path_overflow;
use recomp_startup::plugin_selection::{
    FALLBACK_PLUGIN_NAME, PLUGIN_BUFFER_HEAD, PLUGINS_FILE_NAME, PLUGINS_OPEN_MODE,
    PluginSelection, TEST_FILE_SETTINGS, plugins_file_name,
};

/// IDA: N/A `phase10.rows`, recording which lines a check has read.
struct Rows {
    lines: Vec<String>,
    read: RefCell<Vec<bool>>,
}

impl Drop for Rows {
    fn drop(&mut self) {
        fixtures::ledger_read("phase10", &self.read.borrow());
    }
}

impl Rows {
    fn load() -> Self {
        let lines = fixtures::rows("phase10");
        let read = RefCell::new(vec![false; lines.len()]);
        Self { lines, read }
    }

    fn compare(&self, host: &str, prefix: &str) {
        let index = self
            .lines
            .iter()
            .position(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("missing expected line {prefix}"));
        self.read.borrow_mut()[index] = true;
        assert_eq!(host, self.lines[index], "mismatch for {prefix}");
    }

    fn unread(&self) -> Vec<&str> {
        self.lines
            .iter()
            .zip(self.read.borrow().iter())
            .filter(|(_, read)| !**read)
            .map(|(line, _)| line.as_str())
            .collect()
    }
}

/// IDA: N/A the "F before|after" lines: each TESFile's +0x1C name and +0x3DC flags in list order, then the count.
fn file_lines(world: &World, rows: &Rows, label: &str) {
    let data = world.resource::<DataHandler>();
    for (index, &id) in data.files_8c8.iter().enumerate() {
        let file = data.file_store.get(id).unwrap();
        let prefix = format!("F {label} file{index} name=");
        rows.compare(
            &format!("{prefix}{} flags={:X}", file.name_1c, file.flags_3dc),
            &prefix,
        );
    }
    let prefix = format!("F {label} count=");
    rows.compare(&format!("{prefix}{}", data.files_8c8.len()), &prefix);
}

fn text(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

/// IDA: N/A a `name="value"` field of a record line.
fn quoted<'a>(line: &'a str, field: &str) -> &'a str {
    let start = line
        .find(&format!("{field}=\""))
        .unwrap_or_else(|| panic!("no {field} in {line}"))
        + field.len()
        + 2;
    let end = start + line[start..].find('"').unwrap();
    &line[start..end]
}

/// IDA: N/A P10-RUN-001 raw records, Latin-1 as the importer copied them.
fn run_records() -> Vec<String> {
    let bytes =
        std::fs::read(fixtures::root().join("evidence/phase10-run001-2026-10-06.records.txt"))
            .unwrap();
    bytes
        .split(|&byte| byte == b'\n')
        .map(|line| text(line).trim_end_matches('\r').to_owned())
        .collect()
}

/// IDA: 00406687..00406773 against P10-RUN-001: the shipped Data list (0044A2B0 over "Data\\", read only) and the run's own INI and Plugins.txt, pinned in `fixtures/config`, read in place.
#[test]
fn every_row_is_read() {
    let rows = Rows::load();
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    collections
        .ini
        .set_path(fixtures::root().join("config/Oblivion.ini"));
    assert!(collections.ini.open_ini());
    collections.ini.read_all();
    collections.ini.close();

    let mut world = World::new();
    construct_data_handler(&mut world).unwrap();
    let working = fixtures::data_dir()
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    let mut list_warnings = Vec::new();
    build_data_handler_file_list(&mut world, "Data\\", &working, &mut |warning| {
        list_warnings.push(warning);
    })
    .unwrap();
    assert!(
        list_warnings.is_empty(),
        "no plugin warnings with the shipped Data: {list_warnings:?}"
    );

    // W: the published DataHandler. The port has no locale flag: `stricmp` is the __locale_changed = 0 path, which folds ASCII only.
    assert_ne!(stricmp(b"\xC9", b"\xE9"), 0);
    let locale_changed = 0;
    let handler = if world.contains_resource::<DataHandler>() {
        "set"
    } else {
        "null"
    };
    rows.compare(
        &format!("W data_handler={handler} locale_changed={locale_changed}"),
        "W ",
    );
    for (index, &address) in TEST_FILE_SETTINGS.iter().enumerate() {
        let slot = collections.ini.find_address(address).expect("registered");
        let prefix = format!("S slot{}=", index + 1);
        let value = if slot.text.is_some() { "set" } else { "null" };
        let shown = text(slot.text.as_deref().unwrap_or_default());
        rows.compare(
            &format!("{prefix}{address:X} value={value} text=\"{shown}\""),
            &prefix,
        );
    }
    file_lines(&world, &rows, "before");

    let paths = GamePaths {
        use_my_games: true,
        local_data_directory: fixtures::root().join("config"),
        ..GamePaths::default()
    };
    let mut warnings = Vec::new();
    let selection = run_plugin_block(
        &mut world,
        PluginBlockInputs {
            ini: &collections.ini,
            paths: &paths,
            warning: &mut |warning| warnings.push(warning),
        },
    )
    .unwrap();
    assert!(warnings.is_empty(), "no overflow report for a short root");

    // P and O: FileName past the root, which is what the rows keep of it.
    let root = format!("{}\\", paths.local_data_directory.to_string_lossy());
    let tail = selection
        .list
        .file_name
        .strip_prefix(&root)
        .expect("FileName starts with the root and its backslash");
    let root_tail = if selection.list.file_name.starts_with(&root) {
        "\\"
    } else {
        "none"
    };
    rows.compare(
        &format!("P root_tail={root_tail} file={PLUGINS_FILE_NAME} esm={FALLBACK_PLUGIN_NAME}"),
        "P ",
    );
    rows.compare(
        &format!(
            "O FileName=<00B3F178>{tail} mode={PLUGINS_OPEN_MODE} overflow={}",
            u8::from(selection.list.path_overflow)
        ),
        "O FileName=",
    );
    let stream = if selection.list.opened { "set" } else { "null" };
    rows.compare(&format!("O stream={stream}"), "O stream=");
    let data = world.resource::<DataHandler>();
    for (index, found) in selection.list.trace.iter().enumerate() {
        let prefix = format!("L match{} ", index + 1);
        let name = &data.file_store.get(found.file).unwrap().name_1c;
        rows.compare(
            &format!(
                "{prefix}line={} name={name} flags={:X} found={:02}",
                text(&found.line),
                found.flags_before,
                u8::from(found.found_before)
            ),
            &prefix,
        );
    }
    rows.compare(
        &format!(
            "R al={} bl={}",
            u8::from(selection.list.found),
            u8::from(selection.test_file_selected)
        ),
        "R ",
    );
    rows.compare(
        &format!(
            "Z fallback={} testfile_lookups={} matches={}",
            if selection.fallback_taken {
                "yes"
            } else {
                "no"
            },
            selection.test_file_lookups,
            selection.list.matched
        ),
        "Z ",
    );
    file_lines(&world, &rows, "after");
    assert!(rows.unread().is_empty(), "unread rows: {:?}", rows.unread());

    // What the rows do not carry, from the run's records.
    let records = run_records();
    let tagged = |tag: &str| -> Vec<&String> {
        records
            .iter()
            .filter(|line| line.split(' ').nth(2) == Some(tag))
            .collect()
    };
    // Record 54: the pinned Plugins.txt is the file the run read (the manifest holds its SHA-256).
    let manifest = std::fs::read_to_string(fixtures::root().join("manifest.json")).unwrap();
    let entry = &manifest[manifest.find("\"config/Plugins.txt\"").unwrap()..];
    let needle = "\"sha256\": \"";
    let start = entry.find(needle).unwrap() + needle.len();
    let hash = entry[start..start + 64].to_uppercase();
    let exit = records.iter().find(|line| line.contains(" EXIT ")).unwrap();
    assert!(
        exit.contains(&format!(
            "Plugins.txt SHA-256 {}...{} unchanged",
            &hash[..8],
            &hash[hash.len() - 4..]
        )),
        "{exit} against {hash}"
    );
    // The hand-held values, each checked against its source. Every row must be a known kind.
    let record = |number: &str| -> &String {
        records
            .iter()
            .find(|line| line.split(' ').nth(1) == Some(number))
            .unwrap_or_else(|| panic!("no record {number}"))
    };
    let plugins = std::fs::read(paths.local_data_directory.join("Plugins.txt")).unwrap();
    let run_rows = fixtures::rows("phase10_run001");
    assert_eq!(run_rows.len(), 3);
    for row in &run_rows {
        let parts: Vec<&str> = row.split('|').collect();
        assert_eq!(parts[0], "P10-RUN-001", "{row}");
        let value: usize = parts[3].parse().unwrap();
        match parts[1] {
            "MATCHES" => {
                assert!(
                    record(parts[2]).contains(&format!("matches={value} "))
                        || record(parts[2]).ends_with(&format!("matches={value}")),
                    "{row}"
                );
                assert_eq!(selection.list.trace.len(), value, "{row}");
                assert_eq!(tagged("404C07").len(), value, "{row}");
            }
            "RAWWIDTH" => {
                let raw = record(parts[2])
                    .split(' ')
                    .find_map(|field| field.strip_prefix("raw="))
                    .unwrap();
                assert_eq!(raw.len(), value * 2, "{row}");
                assert_eq!(PLUGIN_BUFFER_HEAD, value, "{row}");
            }
            // The whole file was read, one fgets per line: CRLF lines, none of 511 or more characters. The run did not record this count.
            "LINES" => {
                assert_eq!(
                    plugins.iter().filter(|&&byte| byte == b'\n').count(),
                    value,
                    "{row}"
                );
                assert_eq!(selection.list.lines as usize, value, "{row}");
            }
            kind => panic!("unknown run row kind {kind}"),
        }
    }
    // Record 26: FileName and its length, built from record 13's 00B3F178 (which ends with a backslash).
    let path_record = tagged("406687")
        .into_iter()
        .find(|line| line.contains("localDataPath@B3F178"))
        .unwrap();
    let run_root = quoted(path_record, "localDataPath@B3F178");
    let run_paths = GamePaths {
        use_my_games: true,
        local_data_directory: PathBuf::from(run_root.strip_suffix('\\').unwrap()),
        ..GamePaths::default()
    };
    let run_name = plugins_file_name(&run_paths, PLUGINS_FILE_NAME);
    let open_record = tagged("404B75")[0];
    assert_eq!(run_name, quoted(open_record, "FileName"));
    assert!(open_record.contains(&format!("len={}", run_name.len())));
    assert_eq!(quoted(open_record, "mode"), PLUGINS_OPEN_MODE);
    assert!(!original_plugins_path_overflow(
        &run_paths,
        PLUGINS_FILE_NAME
    ));
    // The MATCH records: the line buffer's first 24 bytes hold the terminators and what earlier, longer lines left behind, so they follow every fgets before them. Bytes `fgets` had not reached yet are the
    // original's uninitialized stack, and they are not compared.
    let matches = tagged("404C07");
    for (line, found) in matches.iter().zip(&selection.list.trace) {
        let raw = line
            .split(' ')
            .find_map(|field| field.strip_prefix("raw="))
            .unwrap();
        assert_eq!(raw.len(), PLUGIN_BUFFER_HEAD * 2, "{line}");
        let reached = found.buffer_written.min(PLUGIN_BUFFER_HEAD);
        let host: String = found.buffer_head[..reached]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(host, raw[..reached * 2], "{line}");
    }
}

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

/// IDA: N/A a temp directory with a fixture INI and an Oblivion folder for Plugins.txt; the static settings are registered and the INI read as WinMain does (0040E6BE..0040E757).
struct Harness {
    temp: PathBuf,
    collections: SettingCollections,
    paths: GamePaths,
    warnings: Vec<String>,
}

impl Harness {
    fn new(ini_text: &str) -> Self {
        let temp = std::env::temp_dir().join(format!(
            "recomp_phase10_{}_{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(temp.join("Oblivion")).unwrap();
        std::fs::write(temp.join("Oblivion.ini"), ini_text).unwrap();
        let mut collections = SettingCollections::default();
        register_static_settings(&mut collections);
        collections.ini.set_path(temp.join("Oblivion.ini"));
        assert!(collections.ini.open_ini());
        collections.ini.read_all();
        collections.ini.close();
        let paths = GamePaths {
            use_my_games: true,
            local_data_directory: temp.join("Oblivion"),
            ..GamePaths::default()
        };
        Self {
            temp,
            collections,
            paths,
            warnings: Vec::new(),
        }
    }

    fn plugins_path(&self) -> PathBuf {
        self.temp.join("Oblivion/Plugins.txt")
    }

    fn write_plugins(&self, bytes: &[u8]) {
        std::fs::write(self.plugins_path(), bytes).unwrap();
    }

    fn try_run(&mut self, world: &mut World) -> Result<PluginSelection, String> {
        let warnings = &mut self.warnings;
        run_plugin_block(
            world,
            PluginBlockInputs {
                ini: &self.collections.ini,
                paths: &self.paths,
                warning: &mut |warning| warnings.push(warning),
            },
        )
        .map_err(|error| error.0.to_owned())
    }

    fn run(&mut self, world: &mut World) -> PluginSelection {
        self.try_run(world).unwrap()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.temp);
    }
}

/// IDA: N/A a DataHandler whose 0044A2B0 list holds the named TESFiles with the given flags (the original's FormHeap TESFiles; only +0x1C and +0x3DC matter to this window).
fn fixture_list(files: &[(&str, u32)]) -> World {
    let mut world = World::new();
    construct_data_handler(&mut world).unwrap();
    let mut data = world.resource_mut::<DataHandler>();
    for &(name, flags) in files {
        let id = data.file_store.push(TesFileData {
            name_1c: name.to_owned(),
            flags_3dc: flags,
            ..TesFileData::default()
        });
        data.files_8c8.push(id);
    }
    world
}

fn flags(world: &World, name: &str) -> u32 {
    world
        .resource::<DataHandler>()
        .file_store
        .iter()
        .find(|file| file.name_1c == name)
        .unwrap_or_else(|| panic!("fixture file {name} missing"))
        .flags_3dc
}

fn lookup(world: &World, name: &[u8]) -> Option<String> {
    let data = world.resource::<DataHandler>();
    lookup_file_by_name(Some(data), Some(name))
        .unwrap()
        .map(|id| data.file_store.get(id).unwrap().name_1c.clone())
}

/// IDA: 00404B7F a missing Plugins.txt returns 0; 0040674F/00406753 then take the fallback, which selects only Oblivion.esm.
#[test]
fn missing_and_empty_plugins_take_the_fallback() {
    for empty in [false, true] {
        let mut harness = Harness::new("[General]\n");
        if empty {
            harness.write_plugins(b"");
        }
        let mut world = fixture_list(&[("Oblivion.esm", 1), ("A.esp", 0)]);
        let selection = harness.run(&mut world);
        assert_eq!(
            selection.list.opened, empty,
            "the stream opens only when the file exists"
        );
        // 00404BA0 the first fgets fails on an empty file: closed, returns 0.
        assert!(selection.list.lines == 0 && !selection.list.found);
        assert!(!selection.test_file_selected && selection.test_file_lookups == 0);
        assert!(selection.fallback_taken && selection.fallback_selected);
        assert_eq!(flags(&world, "Oblivion.esm"), 5);
        assert_eq!(flags(&world, "A.esp"), 0);
    }
}

/// IDA: no matching line returns 0 (found stays 0), so the fallback runs; with no Oblivion.esm in the list the fallback lookup returns 0 and nothing is selected.
#[test]
fn fallback_arms() {
    {
        let mut harness = Harness::new("[General]\n");
        harness.write_plugins(b"Missing.esp\r\nAlso Missing.esp\r\n");
        let mut world = fixture_list(&[("Oblivion.esm", 1), ("A.esp", 2)]);
        let selection = harness.run(&mut world);
        assert!(selection.list.opened && selection.list.lines == 2 && !selection.list.found);
        assert!(selection.fallback_taken && selection.fallback_selected);
        assert_eq!(flags(&world, "Oblivion.esm"), 5);
        assert_eq!(flags(&world, "A.esp"), 2);
    }
    {
        let mut harness = Harness::new("[General]\n");
        let mut world = fixture_list(&[("A.esp", 0), ("B.esp", 0)]);
        let selection = harness.run(&mut world);
        assert!(selection.fallback_taken && !selection.fallback_selected);
        assert_eq!(flags(&world, "A.esp"), 0);
        assert_eq!(flags(&world, "B.esp"), 0);
    }
}

/// IDA: 004066E0..00406738: a matching sTestFile sets bl, so a missing Plugins.txt does not take the fallback; a non-matching value is looked up, an empty one and a null value are not.
#[test]
fn test_files() {
    let mut harness =
        Harness::new("[General]\nSTestFile1=Nope.esp\nSTestFile2=\nSTestFile4=b.ESP\n");
    // 004066E8 a null value char* (004A7980 SetString(null)) is skipped.
    harness
        .collections
        .ini
        .find_address_mut(TEST_FILE_SETTINGS[2])
        .unwrap()
        .text = None;
    let mut world = fixture_list(&[("Oblivion.esm", 1), ("B.esp", 0)]);
    let selection = harness.run(&mut world);
    assert_eq!(
        selection.test_file_lookups, 2,
        "two non-empty values are looked up"
    );
    assert!(
        selection.test_file_selected,
        "B.esp matches case-insensitively"
    );
    assert!(
        !selection.list.opened && !selection.fallback_taken,
        "a test-file match suppresses the fallback"
    );
    assert_eq!(flags(&world, "B.esp"), 4);
    assert_eq!(flags(&world, "Oblivion.esm"), 1);
    // 004066E4 a slot the host store does not hold is skipped (never null in the original).
    let empty_settings = SettingCollections::default();
    let mut other = fixture_list(&[("Oblivion.esm", 0)]);
    let none = run_plugin_block(
        &mut other,
        PluginBlockInputs {
            ini: &empty_settings.ini,
            paths: &harness.paths,
            warning: &mut |_| {},
        },
    )
    .unwrap();
    assert!(none.test_file_lookups == 0 && none.fallback_selected);
}

/// IDA: 00404BB0..00404C29 line rules over the CRT text mode: '#' lines and lines of length <= 1 are skipped, a trailing '\n' is dropped, blanks are kept, a lone CR stays in the name, a line of 511+ characters
/// is read in pieces, duplicates match again, Ctrl-Z ends the read and a last line without '\n' is looked up unchanged.
#[test]
fn line_rules() {
    let mut harness = Harness::new("[General]\n");
    let long_tail = "G".repeat(89);
    let mut plugins =
        b"# comment\r\n#A.esp\r\n\r\nA.esp\r\nb.ESP\r\nC.esp \r\nE.esp\r\r\nA.esp\r\n".to_vec();
    plugins.extend_from_slice("G".repeat(511).as_bytes());
    plugins.extend_from_slice(long_tail.as_bytes());
    plugins.extend_from_slice(b"\r\nZ.esp\x1AH.esp\r\n");
    harness.write_plugins(&plugins);
    let mut world = fixture_list(&[
        ("#A.esp", 0),
        ("A.esp", 0),
        ("B.esp", 0),
        ("C.esp", 0),
        ("E.esp", 0),
        (&long_tail, 0),
        ("Z.esp", 0),
        ("H.esp", 0),
    ]);
    let selection = harness.run(&mut world);
    assert_eq!(selection.list.lines, 11, "eleven lines before Ctrl-Z");
    assert!(
        selection.list.matched == 5 && selection.list.found,
        "five matches"
    );
    let data = world.resource::<DataHandler>();
    let order: Vec<String> = selection
        .list
        .trace
        .iter()
        .map(|found| {
            let name = &data.file_store.get(found.file).unwrap().name_1c;
            name[..name.len().min(6)].to_owned()
        })
        .collect();
    assert_eq!(order, ["A.esp", "B.esp", "A.esp", "GGGGGG", "Z.esp"]);
    let duplicate = &selection.list.trace[2];
    assert!(
        duplicate.flags_before == 4 && duplicate.found_before,
        "a duplicate line sets the flag again"
    );
    assert_eq!(flags(&world, "#A.esp"), 0, "a '#' line is skipped");
    assert_eq!(flags(&world, "C.esp"), 0, "trailing blanks are not trimmed");
    assert_eq!(flags(&world, "E.esp"), 0, "a lone CR stays in the name");
    assert_eq!(flags(&world, "H.esp"), 0, "Ctrl-Z ends the read");
    assert_eq!(
        flags(&world, "Z.esp"),
        4,
        "a last line without a newline matches"
    );
    assert_eq!(
        flags(&world, &long_tail),
        4,
        "the tail of a 600-character line matches"
    );
    assert!(!selection.fallback_taken, "found suppresses the fallback");
    // A one-character last line (length 1) is skipped; the stream still opened.
    let mut single = Harness::new("[General]\n");
    single.write_plugins(b"Q");
    let mut q = fixture_list(&[("Q", 0), ("Oblivion.esm", 0)]);
    let skipped = single.run(&mut q);
    assert!(skipped.list.lines == 1 && skipped.list.matched == 0 && skipped.fallback_selected);
    assert_eq!(flags(&q, "Q"), 0, "a one-character line is skipped");
}

/// IDA: 00404BB0 strlen stops at a NUL the line holds: "A.esp\0x\n" looks up "A.esp" with its '\n' left behind the NUL, and a line starting with NUL has length 0. Not in the C++ test; both follow the disassembly.
#[test]
fn a_nul_in_a_line_ends_it() {
    let mut harness = Harness::new("[General]\n");
    harness.write_plugins(b"A.esp\0x\r\n\0B.esp\r\n");
    let mut world = fixture_list(&[("A.esp", 0), ("B.esp", 0), ("Oblivion.esm", 0)]);
    let selection = harness.run(&mut world);
    assert_eq!(selection.list.lines, 2);
    assert_eq!(flags(&world, "A.esp"), 4);
    assert_eq!(flags(&world, "B.esp"), 0);
    assert_eq!(flags(&world, "Oblivion.esm"), 0);
}

/// IDA: 00447C50's walk: a null name returns 0 before the list is read, even with a null DataHandler; the walk stops at the first empty item; a null DataHandler with a name faults in the original (host stop).
#[test]
fn lookup_walk() {
    assert!(matches!(lookup_file_by_name(None, None), Ok(None)));
    let stopped = lookup_file_by_name(None, Some(b"Oblivion.esm")).unwrap_err();
    assert!(stopped.0.starts_with("00447C65"), "{}", stopped.0);
    let mut world = fixture_list(&[("A.esp", 0), ("B.esp", 0)]);
    assert_eq!(lookup(&world, b"b.esp").as_deref(), Some("B.esp"));
    assert_eq!(lookup(&world, b"B.es"), None, "a prefix does not match");
    assert_eq!(
        lookup(&world, b"B.esp "),
        None,
        "a longer name does not match"
    );
    assert!(matches!(
        lookup_file_by_name(world.get_resource(), None),
        Ok(None)
    ));
    // The name is a C string, and +0x1C is compared as the original's bytes with the "C" locale's ASCII folding only.
    assert_eq!(lookup(&world, b"a.ESP\0tail").as_deref(), Some("A.esp"));
    let mut lifted = fixture_list(&[("Caf\u{E9}.esp", 0)]);
    assert_eq!(
        lookup(&lifted, b"CAF\xE9.ESP").as_deref(),
        Some("Caf\u{E9}.esp")
    );
    assert_eq!(lookup(&lifted, b"CAF\xC9.ESP"), None);
    // The first empty item ends the walk even when a later node holds the name. The list stands for an empty item with a destroyed file.
    let mut data = world.resource_mut::<DataHandler>();
    let first = data.files_8c8[0];
    data.file_store.files[first.0] = None;
    assert_eq!(lookup(&world, b"B.esp"), None);
    // An empty list: the inline head's item is 0.
    lifted.resource_mut::<DataHandler>().files_8c8.clear();
    assert_eq!(lookup(&lifted, b"Caf\xE9.esp"), None);
}

/// IDA: 0044FAD0 both arms: |= 4; &= 0xFFFFFFF3 clears Loaded and Active only.
#[test]
fn set_loaded_arms() {
    let mut file = TesFileData {
        flags_3dc: 0xFF,
        ..TesFileData::default()
    };
    file.set_loaded(false);
    assert_eq!(
        file.flags_3dc, 0xF3,
        "set_loaded(false) clears bits 2 and 3"
    );
    file.flags_3dc = 0x9;
    file.set_loaded(true);
    assert_eq!(file.flags_3dc, 0xD, "set_loaded(true) sets bit 2 only");
}

/// IDA: a null DataHandler faults at the first looked-up line in 00404B00 (host stop) and at the fallback lookup when nothing is read.
#[test]
fn null_data_handler_stops() {
    let mut harness = Harness::new("[General]\n");
    harness.write_plugins(b"Oblivion.esm\r\n");
    let mut world = World::new();
    let stopped = harness.try_run(&mut world).unwrap_err();
    assert!(stopped.starts_with("00447C65"), "{stopped}");
    std::fs::remove_file(harness.plugins_path()).unwrap();
    let stopped = harness.try_run(&mut world).unwrap_err();
    assert!(stopped.starts_with("00447C65"), "{stopped}");
    // A skipped line never reaches the lookup: only the fallback faults.
    harness.write_plugins(b"#Oblivion.esm\r\nQ\r\n");
    let stopped = harness.try_run(&mut world).unwrap_err();
    assert!(stopped.starts_with("00447C65"), "{stopped}");
    // A test file faults at 0040671E, before Plugins.txt.
    let mut test_file = Harness::new("[General]\nSTestFile1=A.esp\n");
    let stopped = test_file.try_run(&mut world).unwrap_err();
    assert!(stopped.starts_with("00447C65"), "{stopped}");
}

/// IDA: 00404B1E..00404B73 the FileName char[260]: the report for a long root (the file is then simply missing here) and the ".\" root that never overflows.
#[test]
fn path_overflow() {
    let mut harness = Harness::new("[General]\n");
    let mut world = fixture_list(&[("Oblivion.esm", 0)]);
    harness.paths.local_data_directory = harness.temp.join("p".repeat(250));
    let selection = harness.run(&mut world);
    assert!(selection.list.path_overflow && harness.warnings.len() == 1);
    assert!(harness.warnings[0].contains("00404B00"));
    assert!(!selection.list.opened && selection.fallback_selected);
    let dot = GamePaths {
        use_my_games: false,
        local_data_directory: harness.temp.join("p".repeat(250)),
        ..GamePaths::default()
    };
    assert!(
        !original_plugins_path_overflow(&dot, PLUGINS_FILE_NAME),
        "the .\\ root is 2 bytes in the original"
    );
    assert_eq!(
        plugins_file_name(&dot, PLUGINS_FILE_NAME),
        ".\\.\\Plugins.txt"
    );
    let mut root = GamePaths {
        use_my_games: true,
        local_data_directory: PathBuf::from("C:\\Root\\Oblivion"),
        ..GamePaths::default()
    };
    assert_eq!(
        plugins_file_name(&root, PLUGINS_FILE_NAME),
        "C:\\Root\\Oblivion\\.\\Plugins.txt"
    );
    // A root of 245 bytes (+ '\' + 13 + NUL = 260) still fits.
    root.local_data_directory = PathBuf::from("r".repeat(245));
    assert!(!original_plugins_path_overflow(&root, PLUGINS_FILE_NAME));
    root.local_data_directory = PathBuf::from("r".repeat(246));
    assert!(original_plugins_path_overflow(&root, PLUGINS_FILE_NAME));
}

/// IDA: N/A host policy: a `plugins.txt` in another case is found as Windows finds it, a directory of that name is a null stream as Windows' fopen makes it, the ".\" root reads under the working directory, and the
/// file is never written.
#[test]
fn host_file_rules() {
    let mut harness = Harness::new("[General]\n");
    let lower = harness.temp.join("Oblivion/plugins.TXT");
    std::fs::write(&lower, b"A.esp\r\n").unwrap();
    let before = std::fs::metadata(&lower).unwrap().modified().unwrap();
    let mut world = fixture_list(&[("A.esp", 0)]);
    let selection = harness.run(&mut world);
    assert!(selection.list.opened && selection.list.found);
    assert_eq!(flags(&world, "A.esp"), 4);
    assert_eq!(std::fs::read(&lower).unwrap(), b"A.esp\r\n");
    assert_eq!(
        std::fs::metadata(&lower).unwrap().modified().unwrap(),
        before
    );
    std::fs::remove_file(&lower).unwrap();

    std::fs::create_dir(harness.plugins_path()).unwrap();
    let mut world = fixture_list(&[("Oblivion.esm", 0)]);
    let selection = harness.run(&mut world);
    assert!(!selection.list.opened && selection.fallback_selected);
    std::fs::remove_dir(harness.plugins_path()).unwrap();

    let working = harness.temp.join("Oblivion");
    std::fs::write(working.join("Plugins.txt"), b"A.esp\n").unwrap();
    harness.paths = GamePaths {
        use_my_games: false,
        local_data_directory: working,
        ..GamePaths::default()
    };
    let mut world = fixture_list(&[("A.esp", 0)]);
    let selection = harness.run(&mut world);
    assert_eq!(selection.list.file_name, ".\\.\\Plugins.txt");
    assert!(selection.list.opened && selection.list.found);
}
