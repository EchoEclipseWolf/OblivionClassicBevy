//! The C++ `startup_scaffold` test's `verify_startup_assets`. `plan_archive_initialization` is the C++ host's menu
//! shortcut and is not ported (M6 uses the real manager), so its configuration checks run through `ArchiveManager`, which
//! holds the original logic it summarized: 0042F610's list, Plugins.txt sidecars and bUseArchives gate.

use std::path::{Path, PathBuf};

use oblivion_core::host_path;
use oblivion_sim::bs::archive::{ArchiveLoadStatus, ArchiveManager, ArchiveSettings};
use oblivion_sim::bs::file_finder::{BsFileCallbacks, FileFinder};
use oblivion_sim::tes::blend_settings::{BlendSettingsState, initialize_blend_settings};
use oblivion_sim::tes::settings::{
    SettingCollections, archive_settings_from, register_static_settings,
};

struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fresh_root() -> Cleanup {
    let root = std::env::temp_dir().join(format!("recomp_assets_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Data")).unwrap();
    std::fs::create_dir_all(root.join("local")).unwrap();
    Cleanup(root)
}

fn finder_at(root: &Path) -> FileFinder {
    let mut callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new(root, &mut callbacks);
    finder.add("Data");
    finder
}

/// The C++ `find_loose_file`: FindFile with flag 1 added, found as a loose file.
fn loose(finder: &FileFinder, name: &str, flags: u32) -> Option<PathBuf> {
    let result = finder.find_file(name, flags | 1, -1, None);
    (result.status == 1).then(|| host_path::resolve(finder.working_directory(), &result.resolved))
}

fn settings_from_ini(ini: &Path) -> ArchiveSettings {
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    collections.ini.set_path(ini);
    if collections.ini.open_ini() {
        collections.ini.read_all();
        collections.ini.close();
    }
    archive_settings_from(&collections.ini)
}

/// IDA: 004316E0 / 00431B60 and 0042F610.
#[test]
fn verify_startup_assets() {
    let mut callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new("", &mut callbacks);
    finder.add("Data");
    finder.add("Other\\");
    assert_eq!(finder.paths(), ["Data\\", "Other\\"]);

    let cleanup = fresh_root();
    let root = &cleanup.0;
    std::fs::write(root.join("Direct.bsa"), "").unwrap();
    std::fs::write(root.join("Data/Rooted.bsa"), "").unwrap();
    let rooted = finder_at(root);
    assert!(loose(&rooted, "Direct.bsa", 0).is_some());
    assert_eq!(
        loose(&rooted, "Rooted.bsa", 0),
        Some(root.join("Data/Rooted.bsa"))
    );
    assert!(loose(&rooted, "Data\\Rooted.bsa", 0).is_some());
    assert!(loose(&rooted, "Rooted.bsa", 2).is_some());
    assert!(loose(&rooted, "Rooted.bsa", 4).is_none());
    assert!(loose(&rooted, ".\\Rooted.bsa", 0).is_none());
    assert!(
        loose(&rooted, root.join("Direct.bsa").to_str().unwrap(), 0).is_some(),
        "an absolute path"
    );
    assert!(loose(&rooted, "Missing.bsa", 0).is_none());
    // path[1] == ':' adds flags 5, so a drive path takes no search root.
    assert!(loose(&rooted, "Z:\\Rooted.bsa", 0).is_none());

    // The configured list, Plugins.txt sidecars and the order of the lookups.
    let ini = root.join("Oblivion.ini");
    std::fs::write(
        &ini,
        "[Archive]\nbUseArchives=1\nSInvalidationFile=CustomInvalidation.txt\n\
         SArchiveList=Oblivion - Misc.bsa, Oblivion - Textures - Compressed.bsa\n",
    )
    .unwrap();
    std::fs::write(
        root.join("local/Plugins.txt"),
        "# disabled\nTest.esp\nOther.esm\n",
    )
    .unwrap();
    std::fs::write(root.join("Data/Test - Extra.bsa"), "").unwrap();
    std::fs::write(root.join("Data/Other.bsa"), "").unwrap();
    let settings = settings_from_ini(&ini);
    assert!(settings.use_archives);
    assert_eq!(settings.invalidation_file, b"CustomInvalidation.txt");
    let mut manager = ArchiveManager::new(root, settings);
    assert!(
        !manager.initialize_from_settings(&rooted, &root.join("local")),
        "both listed masters are missing"
    );
    assert_eq!(
        manager.requested_names(),
        [
            "Oblivion - Misc.bsa",
            "Oblivion - Textures - Compressed.bsa",
            "Test - Extra.bsa"
        ]
    );
    assert_eq!(manager.archives().len(), 1);
    assert_eq!(manager.archives()[0].status(), ArchiveLoadStatus::Loaded);
    assert_eq!(
        loose(&rooted, "Test - Extra.bsa", 0),
        Some(root.join("Data/Test - Extra.bsa"))
    );
    drop(manager);

    // BlendSettings VERSION gate.
    let blend = root.join("BlendSettings.ini");
    std::fs::write(&blend, "[DEFAULT]\nVERSION=14\n").unwrap();
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    let mut state = BlendSettingsState::default();
    assert!(
        initialize_blend_settings(&mut collections, root, &blend, &mut state).version_gate_passed
    );

    // A missing INI keeps the original's six masters.
    let defaults = settings_from_ini(&root.join("Missing.ini"));
    let list = String::from_utf8(defaults.archive_list.clone()).unwrap();
    let masters: Vec<&str> = list.split(',').map(str::trim).collect();
    assert_eq!(masters.len(), 6);
    assert_eq!(masters[0], "Oblivion - Meshes.bsa");
    assert_eq!(masters[5], "Oblivion - Misc.bsa");

    // bUseArchives=0 opens nothing and looks nothing up.
    let disabled = root.join("Disabled.ini");
    std::fs::write(&disabled, "[Archive]\nbUseArchives=0\n").unwrap();
    let settings = settings_from_ini(&disabled);
    assert!(!settings.use_archives);
    let mut manager = ArchiveManager::new(root, settings);
    assert!(manager.initialize_from_settings(&rooted, &root.join("local")));
    assert!(manager.requested_names().is_empty());
    assert!(manager.archives().is_empty());
    assert!(
        manager
            .open_archive("Test - Extra.bsa", 0, &rooted)
            .is_none()
    );
}
