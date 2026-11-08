//! Ported from the C++ `tes_settings` test, plus path setup and the pinned evidence files.

use std::path::{Path, PathBuf};

use recomp_core::fixtures;
use recomp_game::tes::blend_settings::*;
use recomp_game::tes::settings::*;
use recomp_startup::path_setup::*;
use recomp_startup::winmain_settings::*;

fn temp_directory(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("recomp_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn registered() -> SettingCollections {
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    collections
}

/// IDA: 00A283CC..00A2F7D0 registrations through 004040E0.
#[test]
fn registration_matches_the_captured_table() {
    let definitions = static_setting_definitions();
    assert_eq!(definitions.len(), 635);
    assert_eq!(definitions[0].collection, SettingCollectionKind::Registry);
    assert_eq!(definitions[0].name, "sBasePath");
    assert_eq!(definitions[0].text, Some(".\\"));

    let collections = registered();
    assert_eq!(collections.registry.settings().len(), 1);
    assert_eq!(collections.ini.settings().len(), 572);
    assert_eq!(collections.blend.settings().len(), 62);
    assert!(collections.ini.rejected_duplicates().is_empty());
    assert!(collections.blend.rejected_duplicates().is_empty());
    // PrependNonNull: the head is the last registered INI setting.
    assert_eq!(
        collections.ini.settings().front().unwrap().original_address,
        0x00B2_C35C
    );
    assert_eq!(
        collections.ini.settings().back().unwrap().original_address,
        0x00B0_2C2C
    );
    let archives = collections.ini.find_address(0x00B0_4458).unwrap();
    assert!(
        archives
            .text
            .as_ref()
            .unwrap()
            .starts_with(b"Oblivion - Meshes.bsa")
    );
    assert!(collections.ini.find_name("BUSEARCHIVES:archive").is_some());

    let mut duplicate = SettingCollection::default();
    assert!(duplicate.add(SettingInfo::new(1, "bFlag:Test", 1, None)));
    assert!(!duplicate.add(SettingInfo::new(2, "BFLAG:test", 0, None)));
    assert_eq!(duplicate.settings().len(), 1);
    assert_eq!(duplicate.rejected_duplicates().len(), 1);
}

/// IDA: 004A8800 per-type INI reads.
#[test]
fn ini_reads_per_type() {
    let root = temp_directory("settings");
    let ini = root.join("Oblivion.ini");
    std::fs::write(
        &ini,
        "[Sec]\nbOn=5\nbOff=0\ncChar=300\niInt=-7\nfFloat=2.5\nfEmpty=\nfBad=abc\n\
         sText=hello world\nrColor=10,20,300\nrShort=7\naColor=1,2,3,4\nuValue=42\n\
         hValue=9\nxOther=11\n[MAIN]\niNoSection=123\n",
    )
    .unwrap();
    let mut collection = SettingCollection::default();
    collection.set_path(&ini);
    let third = 1.0f32 / 30.0;
    let cases: [(&str, u32); 20] = [
        ("bOn:Sec", 0xAB00),
        ("bOff:Sec", 0x01),
        ("bMissing:Sec", 0x01),
        ("cChar:Sec", 0x1200),
        ("iInt:Sec", 3),
        ("iMissing:Sec", 0xFFFF_FFFF),
        ("fFloat:Sec", 0),
        ("fMissing:Sec", third.to_bits()),
        ("fEmpty:Sec", 4.0f32.to_bits()),
        ("fBad:Sec", 6.0f32.to_bits()),
        ("sText:Sec", 0),
        ("sMissing:Sec", 0),
        ("rColor:Sec", 0),
        ("rShort:Sec", 0),
        ("rMissing:Sec", 0x1234_5678),
        ("aColor:Sec", 0),
        ("uValue:Sec", 0),
        ("hValue:Sec", 0),
        ("xOther:Sec", 0),
        ("iNoSection", 0),
    ];
    for (address, (name, raw)) in (1u32..).zip(cases) {
        let text = name.starts_with('s').then_some(&b"fallback"[..]);
        assert!(collection.add(SettingInfo::new(address, name, raw, text)));
    }
    assert!(!collection.read_all(), "ReadAll needs the 004A83B0 handle");
    assert!(collection.open_ini());
    assert!(
        !collection.read_all(),
        "ReadAll ANDs the false non-float results"
    );
    assert!(collection.close());

    let value = |name: &str| collection.find_name(name).unwrap().raw;
    assert_eq!(value("bOn:Sec"), 0xAB01, "bool stores byte 0 only");
    assert_eq!(value("bOff:Sec"), 0);
    assert_eq!(value("bMissing:Sec"), 1);
    assert_eq!(value("cChar:Sec"), 0x122C, "char stores the low byte");
    assert_eq!(value("iInt:Sec"), 0xFFFF_FFF9);
    assert_eq!(value("iMissing:Sec"), 0xFFFF_FFFF);
    assert_eq!(value("fFloat:Sec"), 2.5f32.to_bits());
    assert_eq!(
        value("fMissing:Sec"),
        0.033333f32.to_bits(),
        "the \"%f\" default round trip"
    );
    assert_eq!(value("fEmpty:Sec"), 4.0f32.to_bits());
    assert_eq!(value("fBad:Sec"), 6.0f32.to_bits());
    let text = collection.find_name("sText:Sec").unwrap();
    assert_eq!(text.text.as_deref(), Some(&b"hello world"[..]));
    assert_eq!(text.name, "SText:Sec", "SetString's 'S' rewrite");
    assert_eq!(
        collection
            .find_name("sMissing:Sec")
            .unwrap()
            .text
            .as_deref(),
        Some(&b"fallback"[..])
    );
    assert_eq!(value("rColor:Sec"), 0x0A14_2CFF);
    assert_eq!(value("rShort:Sec"), 0x0700_00FF);
    assert_eq!(value("rMissing:Sec"), 0x1234_5678);
    assert_eq!(value("aColor:Sec"), 0x0102_0304);
    assert_eq!(value("uValue:Sec"), 42);
    assert_eq!(value("hValue:Sec"), 9);
    assert_eq!(value("xOther:Sec"), 11);
    assert_eq!(value("iNoSection"), 123);

    // WinMain order and the archive transfer from the real table.
    std::fs::write(
        &ini,
        "[Archive]\nbUseArchives=0\nSArchiveList=Only.bsa\niRetainDirectoryStringTable=2\n",
    )
    .unwrap();
    let mut collections = registered();
    let load = load_winmain_settings(&mut collections, &ini);
    assert!(load.ini_opened && !load.ini_read_all);
    assert!(!collections.ini.is_open() && !collections.registry.is_open());
    let archive = archive_settings_from(&collections.ini);
    assert!(!archive.use_archives);
    assert_eq!(archive.archive_list, b"Only.bsa");
    assert_eq!(archive.retain_directory_strings, 2);
    assert_eq!(archive.invalidation_file, b"ArchiveInvalidation.txt");
    assert!(archive.invalidate_older_files);
    std::fs::remove_dir_all(&root).unwrap();
}

/// IDA: the 0053AC60 gate and the 0053A1B0 / 0053A460 / 0053A720 transfers.
#[test]
fn blend_settings_gate_and_transfers() {
    let root = temp_directory("blend");
    let games = root.join("games");
    std::fs::create_dir_all(&games).unwrap();
    let version = root.join("version.ini");
    std::fs::write(&version, "[DEFAULT]\nVERSION=13\n").unwrap();
    std::fs::write(
        games.join("BlendSettings.ini"),
        "[HIT]\nsHead=0.25, 0.75\nfGetUpTime=2.0\n[DEFAULT]\nfHighTrans=0.5\nsRHand=3.0\n",
    )
    .unwrap();
    {
        let mut collections = registered();
        let mut state = BlendSettingsState::default();
        let load = initialize_blend_settings(&mut collections, &games, &version, &mut state);
        assert_eq!(load.version, 13);
        assert!(
            !load.version_gate_passed && !load.collection_opened,
            "13 fails the >= 14 gate"
        );
        assert_eq!(state.value(0x00B2_EC70), 0.4);
        assert_eq!(state.value(0x00B2_EC74), 0.6);
        assert_eq!(
            state.value(0x00B2_E6C8),
            1.0,
            "\"1.0f, 1.0\" stops after one field"
        );
        assert_eq!(state.value(0x00B2_E6CC), 1.0);
        assert_eq!(state.get_up_times[0], 1.0);
        assert_eq!(
            state.get_up_times[25], -1.0,
            "only entries >= 0 are replaced"
        );
        assert_eq!(state.knock_down_times[24], 0.25);
        assert_eq!(state.knock_down_times[31], -1.0);
        assert_eq!(state.value(0x00B2_EC58), 0.95);
        assert_eq!(state.value(0x00B2_EC54), 0.3);
        assert_eq!(state.value(0x00B2_EC5C), -10.0);
        assert_eq!(state.value(0x00B2_EC60), 1.2);
        assert_eq!(state.value(0x00B2_FD20), 0.07);
        assert_eq!(state.value(0x00B2_FD2C), 0.2);
    }
    {
        std::fs::write(&version, "[DEFAULT]\nVERSION=14\n").unwrap();
        let mut collections = registered();
        let mut state = BlendSettingsState::default();
        let load = initialize_blend_settings(&mut collections, &games, &version, &mut state);
        assert!(load.version_gate_passed && load.collection_opened && !load.collection_read_all);
        assert_eq!(state.value(0x00B2_EC70), 0.25);
        assert_eq!(state.value(0x00B2_EC74), 0.75);
        assert_eq!(state.get_up_times[0], 2.0);
        assert_eq!(state.value(0x00B2_FD20), 0.5);
        assert_eq!(state.value(0x00B2_E6C8), 3.0);
        assert_eq!(state.value(0x00B2_E6CC), 1.0);
    }
    {
        std::fs::write(&version, "[DEFAULT]\nVERSION=-1\n").unwrap();
        let mut collections = registered();
        let mut state = BlendSettingsState::default();
        let load = initialize_blend_settings(&mut collections, &games, &version, &mut state);
        assert!(
            load.version_gate_passed,
            "the 0053AC8F gate compares unsigned (jb)"
        );
    }
    let virtual_root = root.join("local");
    let found = blend_settings_version_file(&root, &virtual_root);
    assert_eq!(found, root.join("BlendSettings.ini"));
    let store = virtual_root.join("VirtualStore/Windows");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("BlendSettings.ini"), "").unwrap();
    assert_eq!(
        blend_settings_version_file(&root, &virtual_root),
        store.join("BlendSettings.ini")
    );
    std::fs::remove_dir_all(&root).unwrap();
}

/// The files the evidence runs used, through the real sequence.
#[test]
fn pinned_config_loads_through_the_winmain_order() {
    let config = fixtures::root().join("config");
    let mut collections = registered();
    let load = load_winmain_settings(&mut collections, &config.join("Oblivion.ini"));
    assert!(load.ini_opened);
    let archive = archive_settings_from(&collections.ini);
    assert!(archive.use_archives, "the user's INI keeps archives on");
    assert!(archive.archive_list.starts_with(b"Oblivion - Meshes.bsa"));
    // P10-RUN-001: all ten sTestFile values are empty.
    for address in (0x00B0_2C90..=0x00B0_2CD8).step_by(8) {
        let setting = collections.ini.find_address(address).unwrap();
        assert_eq!(setting.text.as_deref(), Some(&b""[..]), "{address:08X}");
    }
    assert!(
        !original_path_buffer_overflow(&plan_startup_paths(
            &PathRoots {
                working: "/w".into(),
                documents: "/d".into(),
                local_app_data: "/l".into()
            },
            true
        ))
        .selected_ini
    );
}

/// IDA: 0040DFB2..0040E186 path selection and 0040E043..0040E221 preparation.
#[test]
fn path_setup_follows_the_original_order() {
    let root = temp_directory("paths");
    let working = root.join("work");
    std::fs::create_dir_all(&working).unwrap();
    let roots = PathRoots {
        working: working.clone(),
        documents: root.join("Documents"),
        local_app_data: root.join("Local"),
    };
    std::fs::create_dir_all(&roots.documents).unwrap();
    std::fs::create_dir_all(&roots.local_app_data).unwrap();

    assert!(
        use_my_games_from_bootstrap(&working.join("Oblivion.ini")),
        "the default is 1"
    );
    std::fs::write(
        working.join("Oblivion.ini"),
        "[General]\nbUseMyGamesDirectory=0\n",
    )
    .unwrap();
    assert!(!use_my_games_from_bootstrap(&working.join("Oblivion.ini")));

    let local = plan_startup_paths(&roots, false);
    assert_eq!(local.settings_directory, working);
    assert_eq!(local.local_data_directory, working);

    let paths = plan_startup_paths(&roots, true);
    assert_eq!(
        paths.settings_directory,
        roots.documents.join("My Games/Oblivion")
    );
    assert_eq!(
        paths.local_data_directory,
        roots.local_app_data.join("Oblivion")
    );
    assert_eq!(
        paths.selected_ini,
        paths.settings_directory.join("Oblivion.ini")
    );

    // No default to copy: the failure is reported and startup continues.
    let first = prepare_startup_paths(&paths);
    assert_eq!(first.directory_failures, [false; 3]);
    assert!(!first.selected_ini_existed && first.default_copy_attempted && !first.default_copied);

    std::fs::write(working.join("Oblivion_default.ini"), "default").unwrap();
    let second = prepare_startup_paths(&paths);
    assert!(second.default_copied);
    assert_eq!(std::fs::read(&paths.selected_ini).unwrap(), b"default");

    // An existing INI is preserved.
    std::fs::write(&paths.selected_ini, "mine").unwrap();
    let third = prepare_startup_paths(&paths);
    assert!(third.selected_ini_existed && !third.default_copy_attempted);
    assert_eq!(std::fs::read(&paths.selected_ini).unwrap(), b"mine");

    // An empty folder is root-relative, as the original's empty-string prefix is.
    let empty = PathRoots {
        working: working.clone(),
        ..PathRoots::default()
    };
    assert_eq!(
        plan_startup_paths(&empty, true).settings_directory,
        Path::new("/My Games/Oblivion")
    );

    assert!(!original_plugins_path_overflow(&paths, "Plugins.txt"));
    assert!(original_plugins_path_overflow(&paths, &"x".repeat(0x104)));
    assert_eq!(
        plugins_list_path(&paths, "Plugins.txt"),
        paths.local_data_directory.join("Plugins.txt")
    );
    assert!(disc_check_passed());
    std::fs::remove_dir_all(&root).unwrap();
}
