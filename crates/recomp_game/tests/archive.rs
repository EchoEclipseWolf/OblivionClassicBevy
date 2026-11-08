//! Ported from the C++ `bs_archive` test, including P03-RUN-006 on the shipped archives.
//! Singleton-lifetime checks are gone with the singleton: a `Resource` has one owner.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::{Duration, SystemTime};

use bevy_ecs::prelude::*;
use recomp_core::crt::{stricmp, strnicmp};
use recomp_core::fixtures;
use recomp_core::hash::bsa_name_hash;
use recomp_game::bs::archive::*;
use recomp_game::bs::file::*;
use recomp_game::bs::file_finder::*;
use recomp_game::bs::texture_palette::*;
use recomp_game::gamebryo::search_path::SearchPathCreatorSlot;
use recomp_game::gamebryo::texture::SourceTextureLoadServices;

fn limit() -> Arc<AtomicU32> {
    Arc::new(AtomicU32::new(ARCHIVE_FILE_BUFFER_LIMIT_IMAGE))
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("recomp_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("Data")).unwrap();
        std::fs::create_dir_all(path.join("local")).unwrap();
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone)]
struct TestFile {
    name: &'static str,
    data: Vec<u8>,
    size_flags: u32,
}

fn file(name: &'static str, data: &[u8]) -> TestFile {
    TestFile {
        name,
        data: data.to_vec(),
        size_flags: 0,
    }
}

fn file_flagged(name: &'static str, data: &[u8], size_flags: u32) -> TestFile {
    TestFile {
        name,
        data: data.to_vec(),
        size_flags,
    }
}

/// Writes a v103 archive, sorted by hash as Bethesda's tool does.
fn write_bsa(
    path: &Path,
    folders: &[(&str, Vec<TestFile>)],
    archive_flags: u32,
    file_flags: u32,
    version: u32,
) {
    let mut folders = folders.to_vec();
    folders.sort_by_key(|(name, _)| bsa_name_hash(name.as_bytes(), None));
    let mut file_count = 0u32;
    let mut folder_names = 0u32;
    let mut file_names = 0u32;
    for (name, files) in &mut folders {
        files.sort_by_key(|entry| bsa_path_hashes(entry.name).file);
        file_count += files.len() as u32;
        folder_names += name.len() as u32 + 1;
        file_names += files
            .iter()
            .map(|entry| entry.name.len() as u32 + 1)
            .sum::<u32>();
    }
    let folder_count = folders.len() as u32;
    let data_offset =
        36 + folder_count * 16 + folder_count + folder_names + file_count * 16 + file_names;
    let mut out = b"BSA\0".to_vec();
    for value in [
        version,
        36,
        archive_flags,
        folder_count,
        file_count,
        folder_names,
        file_names,
        file_flags,
    ] {
        out.extend(value.to_le_bytes());
    }
    for (name, files) in &folders {
        out.extend(bsa_name_hash(name.as_bytes(), None).to_le_bytes());
        out.extend((files.len() as u32).to_le_bytes());
        out.extend(0u32.to_le_bytes());
    }
    let mut data: Vec<u8> = Vec::new();
    for (name, files) in &folders {
        out.push(name.len() as u8 + 1);
        out.extend(name.as_bytes());
        out.push(0);
        for entry in files {
            out.extend(bsa_path_hashes(entry.name).file.to_le_bytes());
            out.extend((entry.data.len() as u32 | entry.size_flags).to_le_bytes());
            out.extend((data_offset + data.len() as u32).to_le_bytes());
            data.extend(&entry.data);
        }
    }
    for (_, files) in &folders {
        for entry in files {
            out.extend(entry.name.as_bytes());
            out.push(0);
        }
    }
    out.extend(data);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, out).unwrap();
}

fn quiet_settings() -> ArchiveSettings {
    ArchiveSettings {
        invalidate_older_files: false,
        retain_directory_strings: 2,
        ..ArchiveSettings::default()
    }
}

fn open(
    root: &Path,
    path: &str,
    lazy: bool,
    settings: &ArchiveSettings,
    lists: &ArchiveInvalidationLists,
) -> Archive {
    Archive::open(root, path, lazy, settings, lists, limit())
}

/// Expected values from an independent model of the 006FA080 / 006FA040 byte rules.
#[test]
fn hashes_match_the_model() {
    assert_eq!(bsa_name_hash(b"a", Some(b".dds")), 0x8DDB_A9C5_6101_80E1);
    assert_eq!(
        bsa_name_hash(b"clouds", Some(b".dds")),
        0xC379_4FF7_6306_E4F3
    );
    assert_eq!(
        bsa_name_hash(b"CLOUDS", Some(b".DDS")),
        0xC379_4FF7_6306_E4F3
    );
    assert_eq!(bsa_name_hash(b"meshes", None), 0x322F_3A9A_6D06_6573);
    assert_eq!(bsa_name_hash(b"textures\\sky", None), 0xBA9F_8403_740C_6B79);
    assert_eq!(bsa_name_hash(b"x", Some(b".wav")), 0x9733_CF9E_F801_0078);
    assert_eq!(bsa_name_hash(b"head", Some(b".nif")), 0x92CD_4662_6804_E164);
    assert_eq!(bsa_name_hash(b"walk", Some(b".kf")), 0x1711_E44A_7704_6CEB);
    assert_eq!(
        bsa_name_hash(b"si", Some(b".ctl")),
        0x8D6D_9E2D_7302_0069,
        "unknown extensions"
    );
    assert_eq!(
        bsa_name_hash(b"\xE9t\xE9", Some(b".txt")),
        0x95D0_A6C2_E903_74E9,
        "bytes >= 0x80"
    );
    assert_eq!(
        bsa_name_hash(b"ab", Some(b"")),
        0x6102_0062,
        "two-character names"
    );

    let hashes = bsa_path_hashes("textures\\sky\\clouds.dds");
    assert_eq!(hashes.file, bsa_name_hash(b"clouds", Some(b".dds")));
    assert_eq!(hashes.folder, bsa_name_hash(b"textures\\sky", None));
    assert_eq!(bsa_path_hashes("x.dds").folder, bsa_name_hash(b".", None));
    assert_eq!(
        bsa_hash_path("textures\\sky\\", BsaHashMode::Folder),
        bsa_name_hash(b"textures\\sky", None)
    );
    assert_eq!(
        bsa_hash_path("a\\b.dds", BsaHashMode::File),
        bsa_name_hash(b"b", Some(b".dds"))
    );
    assert_eq!(
        bsa_hash_path("raw.dds", BsaHashMode::Raw),
        bsa_name_hash(b"raw.dds", None)
    );

    let split = split_path("C:dir.x\\name.part.ext");
    assert_eq!(split.drive, "C:");
    assert_eq!(split.directory, "dir.x\\");
    assert_eq!(split.file_name, "name.part");
    assert_eq!(split.extension, ".ext");
    let no_ext = split_path("dir.x/name");
    assert_eq!(
        (no_ext.directory.as_str(), no_ext.file_name.as_str()),
        ("dir.x/", "name")
    );
    assert!(no_ext.extension.is_empty());
    assert_eq!(stricmp(b"ABC", b"abc"), 0);
    assert!(stricmp(b"a", b"b") < 0);
    assert_eq!(strnicmp(b"Data\\x", b"data\\", 5), 0);
}

/// IDA: the B04368 table, 0042F610 sort and 0042BA90 lookup.
#[test]
fn type_table_sorts_and_searches() {
    let mut table = ArchiveTypeTable::default();
    table.sort();
    assert_eq!(
        table.extensions(),
        [
            "", "bmp", "cmp", "dds", "egm", "egt", "fnt", "h", "kf", "lip", "lod", "lsl", "mp3",
            "nif", "psh", "pso", "spt", "tex", "tga", "tri", "vsh", "vso", "wav", "xml"
        ]
    );
    for (key, types) in [
        ("dds", 2),
        (".kf", 1),
        ("x.h", 0x20),
        ("NIF", 1),
        ("ctl", 0x100),
        ("sdp", 0x100),
        ("mp3", 0x10),
        ("spt", 0x40),
    ] {
        assert_eq!(table.lookup(key), types, "{key}");
    }
}

/// IDA: 0042EE80 header and table reads, and its failure branches.
#[test]
fn archive_open_paths() {
    let temp = TempRoot::new("archive_open");
    let root = &temp.0;
    write_bsa(
        &root.join("Data/A.bsa"),
        &[
            (
                "meshes",
                vec![
                    file("x.nif", b"XNIF"),
                    file_flagged("y.nif", b"YY", 0x4000_0000),
                ],
            ),
            ("textures\\sky", vec![file("clouds.dds", b"CLOUD")]),
        ],
        3,
        3,
        0x67,
    );
    let none = ArchiveInvalidationLists::default();
    let settings = quiet_settings();
    let mut archive = open(root, "Data\\A.bsa", false, &settings, &none);
    assert_eq!(archive.status(), ArchiveLoadStatus::Loaded);
    assert!(archive.exists());
    assert_eq!((archive.folder_count(), archive.file_count()), (2, 3));
    assert_eq!(archive.runtime_flags(), 0x30);
    assert!(!archive.short_read());
    assert_eq!(archive.path(), "Data\\A.bsa");
    assert_ne!(archive.modification_time(), 0);

    let (folder, entry) = archive
        .find(
            bsa_path_hashes("meshes\\y.nif"),
            "meshes\\y.nif",
            &settings,
            root,
        )
        .unwrap();
    assert_eq!(archive.folder_name(folder).as_deref(), Some("meshes"));
    assert_eq!(archive.file_name(folder, entry).as_deref(), Some("y.nif"));
    let view = archive.packed_view(folder, entry).unwrap();
    assert_eq!(view.size, 2);
    assert!(view.compressed && view.offset != 0);
    assert!(
        archive
            .find(
                bsa_path_hashes("meshes\\z.nif"),
                "meshes\\z.nif",
                &settings,
                root
            )
            .is_none()
    );

    write_bsa(
        &root.join("Data/Bad.bsa"),
        &[("meshes", vec![file("x.nif", b"X")])],
        3,
        1,
        0x68,
    );
    let bad = open(root, "Data\\Bad.bsa", false, &settings, &none);
    assert_eq!(bad.status(), ArchiveLoadStatus::InvalidHeader);
    assert_eq!(bad.runtime_flags(), 1);

    std::fs::write(root.join("Data/Empty.bsa"), "").unwrap();
    let empty = open(root, "Data\\Empty.bsa", false, &settings, &none);
    assert_eq!(
        empty.status(),
        ArchiveLoadStatus::Loaded,
        "the default header"
    );
    assert_eq!((empty.folder_count(), empty.version()), (0, 0x67));
    assert!(empty.short_read());

    let bytes = std::fs::read(root.join("Data/A.bsa")).unwrap();
    std::fs::write(root.join("Data/Short.bsa"), &bytes[..52]).unwrap();
    let truncated = open(root, "Data\\Short.bsa", false, &settings, &none);
    assert_eq!(truncated.status(), ArchiveLoadStatus::Loaded);
    assert!(truncated.short_read());
    assert_eq!(truncated.folders().len(), 2);
    assert_eq!(truncated.folders()[1].hash, 0, "zeroed records");

    let missing = open(root, "Data\\Missing.bsa", false, &settings, &none);
    assert_eq!(missing.status(), ArchiveLoadStatus::Missing);
    assert!(!missing.exists());
    let directory = open(root, "Data", false, &settings, &none);
    assert_eq!(
        directory.status(),
        ArchiveLoadStatus::Missing,
        "_access passes, fopen fails"
    );
    assert!(!directory.exists());
    let too_long = open(root, &"a".repeat(0x104), false, &settings, &none);
    assert!(too_long.path().is_empty() && !too_long.exists());
    let lazy = open(root, "Data\\A.bsa", true, &settings, &none);
    assert_eq!(lazy.runtime_flags(), 0x08);
    assert!(lazy.folders().is_empty());
    assert_eq!(lazy.status(), ArchiveLoadStatus::Loaded);

    let bare = ArchiveSettings {
        invalidate_older_files: false,
        retain_directory_strings: 0,
        retain_filename_strings: 0,
        retain_filename_offsets: 0,
        ..ArchiveSettings::default()
    };
    let unnamed = open(root, "Data\\A.bsa", false, &bare, &none);
    assert_eq!(unnamed.runtime_flags(), 0);
    assert!(unnamed.folder_name(0).is_none());
    assert_eq!(unnamed.folders().len(), 2);
}

/// IDA: 0042C1D0 older-loose invalidation, 0042E2D0 lists, and the collision check.
#[test]
fn invalidation_rules() {
    let temp = TempRoot::new("archive_invalidate");
    let root = &temp.0;
    let bsa = root.join("Data/A.bsa");
    write_bsa(
        &bsa,
        &[
            ("meshes", vec![file("x.nif", b"X"), file("y.nif", b"Y")]),
            ("other", vec![file("only.nif", b"O")]),
        ],
        3,
        1,
        0x67,
    );
    std::fs::File::options()
        .write(true)
        .open(&bsa)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(48 * 3600))
        .unwrap();
    std::fs::create_dir_all(root.join("Data/meshes")).unwrap();
    std::fs::write(root.join("Data/meshes/x.nif"), "loose").unwrap();
    let settings = ArchiveSettings::default();
    let none = ArchiveInvalidationLists::default();
    let mut archive = open(root, "Data\\A.bsa", false, &settings, &none);
    assert!(
        archive
            .find(
                bsa_path_hashes("meshes\\x.nif"),
                "meshes\\x.nif",
                &settings,
                root
            )
            .is_none()
    );
    let folder = archive
        .find_folder(bsa_name_hash(b"meshes", None), None, &settings)
        .unwrap();
    let rejected = archive.folders()[folder as usize]
        .files
        .iter()
        .find(|record| record.hash == bsa_path_hashes("x.nif").file)
        .unwrap();
    assert_eq!(
        rejected.offset & 0x7FFF_FFFF,
        0,
        "0042C1D0 clears the offset"
    );
    assert_eq!(rejected.size & 0x8000_0000, 0, "and leaves size bit 31");
    let (folder, entry) = archive
        .find(
            bsa_path_hashes("meshes\\y.nif"),
            "meshes\\y.nif",
            &settings,
            root,
        )
        .unwrap();
    assert_ne!(
        archive.folders()[folder as usize].files[entry as usize].size & 0x8000_0000,
        0
    );

    let lists = ArchiveInvalidationLists {
        folders: Some(vec![bsa_hash_path("meshes\\", BsaHashMode::Folder)]),
        files: Some(vec![bsa_hash_path("only.nif", BsaHashMode::File)]),
    };
    assert_eq!(
        open(root, "Data\\A.bsa", false, &settings, &lists).invalidated_files(),
        2
    );
    std::fs::create_dir_all(root.join("Data/other")).unwrap();
    assert_eq!(
        open(root, "Data\\A.bsa", false, &settings, &lists).invalidated_files(),
        3
    );

    let collisions = ArchiveSettings {
        check_runtime_collisions: true,
        ..quiet_settings()
    };
    let mut checked = open(root, "Data\\A.bsa", false, &collisions, &none);
    let hash = bsa_name_hash(b"meshes", None);
    assert!(
        checked
            .find_folder(hash, Some("meshes\\y.nif"), &collisions)
            .is_none(),
        "the name lacks _splitpath's trailing slash"
    );
    assert!(
        checked
            .find_folder(hash, Some("meshes\\y.nif"), &collisions)
            .is_some(),
        "the cache skips the check"
    );
}

/// Lifted game strings hold one character per original byte: the hash runs over those bytes, lengths count them, and no
/// slice falls inside a UTF-8 character. The original's 0xE9 is "é" here and one byte to 006FA1B0.
#[test]
fn non_ascii_names_keep_the_original_bytes() {
    let hashes = bsa_path_hashes("Textures\\Caf\u{E9}\\Th\u{E9}.dds");
    assert_eq!(hashes.file, bsa_name_hash(b"Th\xE9", Some(b".dds")));
    assert_eq!(hashes.folder, bsa_name_hash(b"Textures\\Caf\xE9", None));
    assert_eq!(
        bsa_hash_path("Caf\u{E9}", BsaHashMode::Raw),
        bsa_name_hash(b"Caf\xE9", None)
    );
    assert_ne!(
        hashes.file,
        bsa_name_hash("Thé".as_bytes(), Some(b".dds")),
        "UTF-8 is a different name to the hash"
    );

    // path[1] is the second original byte; the 255 limit counts characters.
    assert_eq!(split_path("\u{E9}:\\x.nif").drive, "\u{E9}:");
    let long = "\u{E9}".repeat(300);
    assert_eq!(
        split_path(&long).file_name.chars().count(),
        255,
        "cut at 255 characters, not inside one"
    );

    let temp = TempRoot::new("archive_non_ascii");
    let root = &temp.0;
    write_bsa(
        &root.join("Data/A.bsa"),
        &[("meshes", vec![file("caf\u{E9}.nif", b"AX")])],
        3,
        1,
        0x67,
    );
    let mut callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new(root, &mut callbacks);
    finder.add("Data");
    let settings = ArchiveSettings {
        invalidation_file: b"Inv\xE9.txt".to_vec(),
        archive_list: b"A.bsa".to_vec(),
        ..quiet_settings()
    };
    let mut manager = ArchiveManager::new(root, settings);
    // INI bytes that are not UTF-8 name the file a UTF-8 Linux name holds.
    std::fs::write(root.join("Inv\u{E9}.txt"), "x.nif\r\n").unwrap();
    assert_eq!(
        manager.read_invalidation_file(&finder),
        InvalidationFileStatus::Read
    );
    manager.initialize_from_settings(&finder, &root.join("local"));
    assert_eq!(manager.archives().len(), 1);
    assert_eq!(
        manager.find_packed("meshes\\caf\u{E9}.nif", 0xFFFF),
        Some(0)
    );
    assert_eq!(
        manager.find_packed("MESHES\\CAF\u{E9}.NIF", 0xFFFF),
        Some(0)
    );
    // The last three original bytes: three characters, whatever UTF-8 makes of them.
    assert_eq!(manager.find_packed("a\u{E9}bc", 0xFFFF), None);
    assert!(
        manager
            .find_matching_files("Data\\a\u{E9}b*c", "", 0xFFFF)
            .is_empty()
    );

    // FileFinder's char[260] counts original bytes: 200 of them fit, though UTF-8 needs 400.
    let nested = vec!["\u{E9}".repeat(50); 4].join("/");
    std::fs::create_dir_all(root.join("Data").join(&nested)).unwrap();
    std::fs::write(root.join("Data").join(&nested).join("n.nif"), "").unwrap();
    let relative = format!("{}\\n.nif", nested.replace('/', "\\"));
    assert_eq!(finder.find_file(&relative, 1, -1, None).status, 1);
    // 263 original bytes do not.
    assert_eq!(
        finder
            .find_file(&format!("{}\\n.nif", "\u{E9}".repeat(257)), 1, -1, None)
            .status,
        0
    );
}

/// Plugins.txt bytes that are not UTF-8 reach sidecars on disk, whether the disk name is UTF-8 or the same raw bytes.
#[cfg(unix)]
#[test]
fn non_utf8_sidecars_are_found_and_open() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let temp = TempRoot::new("archive_non_utf8");
    let root = &temp.0;
    let utf8 = root.join("Data/Caf\u{E9} - Extra.bsa");
    write_bsa(
        &utf8,
        &[("misc", vec![file("m.txt", b"M")])],
        3,
        0x100,
        0x67,
    );
    let raw = root
        .join("Data")
        .join(OsStr::from_bytes(b"\xC9tude - Extra.bsa"));
    std::fs::copy(&utf8, &raw).unwrap();
    std::fs::write(
        root.join("local/Plugins.txt"),
        b"Caf\xE9.esp\n\xC9tude.esp\n",
    )
    .unwrap();
    let mut callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new(root, &mut callbacks);
    finder.add("Data");
    let settings = ArchiveSettings {
        archive_list: Vec::new(),
        ..quiet_settings()
    };
    let mut manager = ArchiveManager::new(root, settings);
    assert!(manager.initialize_from_settings(&finder, &root.join("local")));
    assert_eq!(
        manager.requested_names(),
        ["Caf\u{E9} - Extra.bsa", "\u{C9}tude - Extra.bsa"]
    );
    assert_eq!(manager.archives().len(), 2, "both open through the lift");
}

/// IDA: 0042F610 list expansion, 0042F4C0 open, slots, 0042EA60 precedence, FileFinder and open routes.
#[test]
fn manager_finder_and_open_routes() {
    let temp = TempRoot::new("archive_manager");
    let root = &temp.0;
    write_bsa(
        &root.join("Data/A.bsa"),
        &[("meshes", vec![file("x.nif", b"AX")])],
        3,
        1,
        0x67,
    );
    write_bsa(
        &root.join("Data/B.bsa"),
        &[
            ("meshes", vec![file("x.nif", b"BX"), file("y.nif", b"BY")]),
            ("textures", vec![file("t.dds", b"T")]),
        ],
        3,
        3,
        0x67,
    );
    write_bsa(
        &root.join("Data/Foo - Extra.bsa"),
        &[("misc", vec![file("m.txt", b"M")])],
        3,
        0x100,
        0x67,
    );
    std::fs::write(
        root.join("local/Plugins.txt"),
        "# comment\nFoo.esp\nBar.esm\n",
    )
    .unwrap();
    std::fs::write(root.join("ArchiveInvalidation.txt"), "unused.nif\r\n").unwrap();
    let settings = ArchiveSettings {
        archive_list: b"A.bsa, B.bsa,  Missing.bsa".to_vec(),
        ..quiet_settings()
    };
    let mut callbacks = BsFileCallbacks::default();
    assert_eq!(
        callbacks.open,
        BsFileCallbacks::NI_FILE_OPEN,
        "the image defaults"
    );
    assert!(matches!(
        bs_file_open(&callbacks, root, None, None, "x.nif", 0, 0),
        Some(OpenedFile::Ni(_))
    ));
    let mut finder = FileFinder::new(root, &mut callbacks);
    assert_eq!(callbacks.open, BsFileCallbacks::FILE_FINDER_OPEN);
    finder.add("Data");
    let mut manager = ArchiveManager::new(root, settings.clone());
    assert!(
        !manager.initialize_from_settings(&finder, &root.join("local")),
        "a missing archive"
    );
    assert_eq!(
        manager.expanded_archive_list(),
        "A.bsa, B.bsa,  Missing.bsa, Foo - Extra.bsa"
    );
    assert_eq!(
        manager.requested_names(),
        ["A.bsa", "B.bsa", "Missing.bsa", "Foo - Extra.bsa"]
    );
    assert_eq!(manager.archives().len(), 3);
    assert_eq!(manager.archives()[0].path(), "Data\\A.bsa");
    assert_eq!(manager.first_by_type()[0], Some(0));
    assert_eq!(manager.first_by_type()[1], Some(1));
    assert_eq!(manager.first_by_type()[8], Some(2));
    assert_eq!(manager.first_by_type()[2], None);
    assert!(manager.secondary_by_type().iter().all(Option::is_none));
    assert_eq!(
        manager.invalidation_lists().files.as_ref().map(Vec::len),
        Some(1)
    );

    assert_eq!(
        manager.find_packed("Data\\meshes\\x.nif", 0xFFFF),
        Some(0),
        "the first list hit"
    );
    assert_eq!(manager.find_packed("\\meshes\\y.nif", 0xFFFF), Some(1));
    assert_eq!(manager.cached(), Some(1));
    assert_eq!(
        manager.find_packed("meshes\\x.nif", 0xFFFF),
        Some(1),
        "the cache before list order"
    );
    assert_eq!(
        manager.find_packed("DATA\\meshes\\x.nif", 0xFFFF),
        None,
        "only Data\\ and data\\ strip"
    );
    assert_eq!(manager.find_packed("textures\\t.dds", 0xFFFF), Some(1));
    assert_eq!(manager.find_packed("textures\\t.dds", 4), None);
    assert_eq!(
        manager.find_packed("misc\\m.txt", 0xFFFF),
        Some(2),
        "unknown extensions route by 0x100"
    );

    let packed = finder.find_file("Data\\meshes\\y.nif", 0, -1, Some(&mut manager));
    assert_eq!(
        (packed.status, packed.resolved.as_str()),
        (2, "Data\\meshes\\y.nif")
    );
    let loose = finder.find_file("A.bsa", 0, -1, Some(&mut manager));
    assert_eq!((loose.status, loose.resolved.as_str()), (1, "Data\\A.bsa"));
    let loose_only = finder.find_file("meshes\\y.nif", 1, -1, Some(&mut manager));
    assert_eq!(loose_only.status, 0);
    assert!(loose_only.resolved.is_empty());
    assert_eq!(
        finder
            .find_file(".\\A.bsa", 0, -1, Some(&mut manager))
            .status,
        0
    );
    assert_eq!(
        finder.find_file("\\Data\\A.bsa", 1, -1, None).resolved,
        "Data\\A.bsa"
    );
    let absolute = root.join("Data/A.bsa");
    assert_eq!(
        finder
            .find_file(absolute.to_str().unwrap(), 0, -1, Some(&mut manager))
            .status,
        1
    );

    {
        let mut routed = bs_file_open(
            &callbacks,
            root,
            Some(&finder),
            Some(&mut manager),
            "meshes\\y.nif",
            0,
            0x2800,
        )
        .unwrap();
        let OpenedFile::Packed(packed_file) = &routed else {
            panic!("status 2 opens the packed record");
        };
        assert_eq!(packed_file.cached_size(), 2);
        assert_eq!(manager.archives()[1].open_count(), 1);
        let mut bytes = [0; 2];
        assert_eq!(routed.read(&mut bytes), 2);
        assert_eq!(&bytes, b"BY");
    }
    assert_eq!(
        manager.archives()[1].open_count(),
        0,
        "0042D650 releases the archive"
    );
    {
        let mut loose_open = bs_file_open(
            &callbacks,
            root,
            Some(&finder),
            Some(&mut manager),
            "A.bsa",
            0,
            0,
        )
        .unwrap();
        assert!(matches!(loose_open, OpenedFile::Loose(_)));
        assert_eq!(loose_open.path(), Some("Data\\A.bsa"));
        assert!(loose_open.good() && !loose_open.has_handle());
        assert!(
            bs_file_open(
                &callbacks,
                root,
                Some(&finder),
                Some(&mut manager),
                "missing.nif",
                0,
                0
            )
            .is_none()
        );
        let mut created = bs_file_open(
            &callbacks,
            root,
            Some(&finder),
            Some(&mut manager),
            "new.txt",
            1,
            0,
        )
        .unwrap();
        assert!(created.good() && created.has_handle());
        assert!(root.join("new.txt").exists());
        let as_bs = bs_file_open_as_bs_file(
            Some(&finder),
            Some(&mut manager),
            "A.bsa",
            0,
            0x2800,
            0xFFFF,
        )
        .unwrap();
        assert_eq!(as_bs.path(), Some("Data\\A.bsa"));
    }
    assert!(bs_file_exists(
        &callbacks,
        root,
        Some(&finder),
        Some(&mut manager),
        "meshes\\y.nif",
        0
    ));
    assert!(!bs_file_exists(
        &callbacks,
        root,
        Some(&finder),
        Some(&mut manager),
        "missing.nif",
        0
    ));
    std::fs::write(root.join("probe.txt"), "data").unwrap();
    assert!(bs_file_exists(
        &callbacks,
        root,
        Some(&finder),
        None,
        "probe.txt",
        1
    ));
    assert_eq!(
        std::fs::metadata(root.join("probe.txt")).unwrap().len(),
        0,
        "wb truncates"
    );
    assert!(bs_file_exists(
        &callbacks,
        root,
        Some(&finder),
        None,
        "probe2.txt",
        2
    ));
    assert!(root.join("probe2.txt").exists());

    // Without a singleton the FileFinder routes return null.
    assert!(bs_file_open(&callbacks, root, None, None, "x.nif", 0, 0).is_none());
    assert!(!bs_file_exists(&callbacks, root, None, None, "x.nif", 0));

    let off = ArchiveSettings {
        use_archives: false,
        ..settings.clone()
    };
    let mut disabled = ArchiveManager::new(root, off);
    assert!(disabled.initialize_from_settings(&finder, &root.join("local")));
    assert!(disabled.archives().is_empty());
    assert!(disabled.open_archive("A.bsa", 0, &finder).is_none());

    let duplicated = ArchiveSettings {
        archive_list: b"A.bsa,A.bsa".to_vec(),
        ..settings
    };
    let mut twice = ArchiveManager::new(root, duplicated);
    assert!(twice.initialize_from_settings(&finder, &root.join("empty")));
    assert_eq!(twice.archives().len(), 2, "no 0042F4C0 dedupe");
    assert_eq!(twice.first_by_type()[0], Some(0));

    let mut search = FileFinder::new(root, &mut callbacks);
    search.add("Data");
    search.add("Data\\");
    assert_eq!(search.paths(), ["Data\\", "Data\\"], "duplicates are kept");
}

/// IDA: 00748260.
#[test]
fn endian_swap_components() {
    let mut data = [1, 2, 3, 4, 5, 6, 7, 8];
    assert_eq!(
        ni_file_swap_components(&mut data[..4], &[2, 2]),
        EndianSwapStatus::Swapped
    );
    assert_eq!(data, [2, 1, 4, 3, 5, 6, 7, 8]);
    assert_eq!(
        ni_file_swap_components(&mut data, &[4]),
        EndianSwapStatus::Swapped
    );
    assert_eq!(data, [3, 4, 1, 2, 8, 7, 6, 5]);
    assert_eq!(
        ni_file_swap_components(&mut data, &[8]),
        EndianSwapStatus::Swapped
    );
    assert_eq!(data, [5, 6, 7, 8, 2, 1, 4, 3]);
    assert_eq!(
        ni_file_swap_components(&mut data, &[1]),
        EndianSwapStatus::Swapped
    );
    assert_eq!(data[0], 5);
    assert_eq!(
        ni_file_swap_components(&mut data[..4], &[]),
        EndianSwapStatus::UnsupportedEndlessLoop
    );
}

/// IDA: NiFile, BSFile, Open, Read, ReadLine, Seek, ReadString, WriteString and SetBufferSize.
#[test]
fn bs_file_behaviour() {
    let temp = TempRoot::new("bs_file");
    let root = &temp.0;
    std::fs::write(root.join("lines.txt"), "first\r\nsecond\r\nthird").unwrap();
    {
        let mut file = BsFile::new(root, "lines.txt", NiFileMode::ReadOnly, 4);
        assert!(file.good() && !file.has_handle());
        assert_eq!(file.fixed_position(), -1);
        assert_eq!(file.read_line(260, 13), (6, b"first".to_vec()));
        assert!(file.has_handle());
        assert_eq!(file.buffer_size(), 4);
        let mut skipped = [0];
        assert_eq!(file.read(&mut skipped), 1);
        assert_eq!(skipped[0], b'\n');
        assert_eq!(file.position(), 7);
        assert_eq!(file.read_line(260, 13), (7, b"second".to_vec()));
        file.read(&mut skipped);
        assert_eq!(
            file.read_line(4, 13),
            (3, b"thi".to_vec()),
            "stops at max - 1"
        );
        assert_eq!(file.read_line(260, 13), (2, b"rd".to_vec()));
        assert_eq!(file.read_line(260, 13), (0, Vec::new()));
        assert_eq!(file.read_line(1, 13), (0, Vec::new()));
        file.seek(0, SEEK_SET);
        let mut five = [0; 5];
        assert_eq!(file.position(), 0);
        assert_eq!(file.read(&mut five), 5);
        assert_eq!(&five, b"first");
        file.seek(-2, SEEK_CUR);
        let mut two = [0; 2];
        assert_eq!(file.position(), 3);
        assert_eq!(file.read(&mut two), 2);
        assert_eq!(&two, b"st");
        assert_eq!(file.size(), 20);
        assert_eq!(file.cached_size(), 20);
        assert!(file.exists());
    }
    {
        let mut missing = BsFile::new(root, "missing.txt", NiFileMode::ReadOnly, 0);
        let mut byte = [0];
        assert!(!missing.good());
        assert_eq!(missing.read(&mut byte), 0);
        assert!(!missing.has_handle());
        let long_path = BsFile::new(root, &"a".repeat(0x104), NiFileMode::ReadOnly, 0);
        assert!(long_path.path().is_empty() && !long_path.good());
    }
    {
        let mut out = BsFile::new(root, "out.bin", NiFileMode::WriteOnly, 16);
        assert!(out.good() && out.has_handle());
        assert!(root.join("out.bin").exists());
        assert_eq!(out.write_string(b"abc"), 4);
        assert_eq!(out.position(), 8, "counted twice");
        assert_eq!(out.write_wstring(&[u16::from(b'h'), u16::from(b'i')]), 3);
        assert_eq!(out.position(), 20);
    }
    assert_eq!(
        std::fs::metadata(root.join("out.bin")).unwrap().len(),
        10,
        "dropping flushes"
    );
    {
        let mut input = BsFile::new(root, "out.bin", NiFileMode::ReadOnly, 0x2800);
        let mut text = Vec::new();
        let mut wide = Vec::new();
        assert_eq!(input.read_string(&mut text, 100), 4);
        assert_eq!(text, b"abc");
        assert_eq!(input.read_wstring(&mut wide, 100), 3);
        assert_eq!(wide, [u16::from(b'h'), u16::from(b'i')]);
        assert!(input.set_buffer_size(8));
        assert_eq!(input.buffer_size(), 8);
        assert_eq!(input.position(), 0);
        assert!(input.set_buffer_size(8));
        let mut abc = [0; 3];
        assert_eq!(input.read(&mut abc), 3);
        assert_eq!(&abc, b"abc");
        let mut limited = BsFile::new(root, "out.bin", NiFileMode::ReadOnly, 0x2800);
        let mut cut = Vec::new();
        assert_eq!(limited.read_string(&mut cut, 2), 3);
        assert_eq!(cut, b"ab");
    }
    {
        let mut plain = NiFile::new(root, "lines.txt", NiFileMode::ReadOnly, 0);
        let mut five = [0; 5];
        assert!(plain.good());
        assert_eq!(plain.read(&mut five), 5);
        assert_eq!(plain.size(), 20);
        let created = NiFile::new(root, "made.txt", NiFileMode::WriteOnly, 0);
        assert!(created.good() && root.join("made.txt").exists());
    }
}

fn zlib_record(payload: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(payload).unwrap();
    let mut record = (payload.len() as u32).to_le_bytes().to_vec();
    record.extend(encoder.finish().unwrap());
    record
}

/// IDA: 0042E070 selection, 0042D540 / 0042D6D0, 0042C3E0 / 0042C4A0 reads, 0042C360 seek, 0042C6F0
/// inflate, 0042C910 release.
#[test]
fn archive_files_read_through_the_archive() {
    let temp = TempRoot::new("archive_files");
    let root = &temp.0;
    let payload: Vec<u8> = (0..70000usize)
        .map(|index| ((index * 7) % 251) as u8)
        .collect();
    let big = vec![b'B'; 200_000];
    write_bsa(
        &root.join("Data/F.bsa"),
        &[(
            "meshes",
            vec![
                file("plain.txt", b"line1\r\nline2\r\n"),
                file_flagged("packed.bin", &zlib_record(&payload), 0x4000_0000),
                file("big.bin", &big),
                file_flagged("broken.bin", b"\x10\x00\x00\x00\x78\x9C\x01", 0x4000_0000),
            ],
        )],
        3,
        0x1FF,
        0x67,
    );
    let mut callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new(root, &mut callbacks);
    finder.add("Data");
    let mut manager = ArchiveManager::new(root, quiet_settings());
    let index = manager.open_archive("F.bsa", 0, &finder).unwrap();
    {
        let archive = &manager.archives()[index];
        assert_eq!(archive.stream_buffer_size(), 0x2800);
        assert_eq!(archive.stream_position(), 0);
        assert!(archive.stream_has_handle() && archive.exists());
    }
    {
        let mut opened = manager
            .open_packed("Data\\meshes\\plain.txt", 0x2800, 0xFFFF)
            .unwrap();
        let OpenedFile::Packed(plain) = &mut opened else {
            panic!("an uncompressed record is an ArchiveFile");
        };
        assert_eq!(manager.archives()[index].open_count(), 1);
        assert_eq!(plain.size(), 14);
        assert_eq!(plain.buffer_size(), 14);
        assert!(plain.exists());
        assert_eq!(plain.path(), "meshes\\plain.txt");
        assert_eq!(plain.read_line(260, 13), (6, b"line1".to_vec()));
        assert_eq!(
            manager.archives()[index].stream_position(),
            plain.archive_offset() + 14
        );
        let mut skipped = [0];
        plain.read(&mut skipped);
        assert_eq!(plain.read_line(260, 13), (6, b"line2".to_vec()));
        plain.seek(0, SEEK_SET);
        let mut four = [0; 4];
        assert_eq!(plain.position(), 0);
        assert_eq!(plain.read(&mut four), 4);
        assert_eq!(&four, b"line");
        plain.seek(100, SEEK_SET);
        assert_eq!(plain.position(), 14, "clamped");
        assert_eq!(plain.read(&mut four), 0);
        plain.seek(-4, SEEK_END);
        assert_eq!(plain.position(), 10);
        assert_eq!(plain.write(&four), 0, "the read-only write slot");
    }
    assert_eq!(manager.archives()[index].open_count(), 0);
    {
        let mut opened = manager
            .open_packed("meshes\\packed.bin", 0x2800, 0xFFFF)
            .unwrap();
        let OpenedFile::Compressed(compressed) = &mut opened else {
            panic!("a flagged record is a CompressedArchiveFile");
        };
        assert!(compressed.stream_ready());
        assert_eq!(compressed.size(), 70000);
        assert_eq!(compressed.position(), 4);
        let mut out = vec![0; payload.len()];
        assert_eq!(compressed.read(&mut out), payload.len());
        assert_eq!(out, payload, "0042C6F0 inflate");
        let mut text = Vec::new();
        assert_eq!(
            compressed.read_string(&mut text),
            0,
            "unwritten in the original"
        );

        let mut again = manager
            .open_packed("meshes\\packed.bin", 0x2800, 0xFFFF)
            .unwrap();
        again.seek(1000, SEEK_SET);
        let mut ten = [0; 10];
        assert_eq!(again.read(&mut ten), 10);
        assert_eq!(&ten[..], &payload[1000..1010], "0042BB30 skips forward");
    }
    {
        let mut broken = manager
            .open_packed("meshes\\broken.bin", 0x2800, 0xFFFF)
            .unwrap();
        let mut sink = [0; 16];
        assert_eq!(broken.read(&mut sink), 0, "truncated data stops");
    }
    {
        let mut preload = manager
            .open_packed("meshes\\plain.txt", -1, 0xFFFF)
            .unwrap();
        assert_eq!(preload.buffer_valid(), 14, "options -1 preloads");
        manager.set_buffer_limit_128k();
        let mut capped = manager
            .open_packed("meshes\\big.bin", 0x2800, 0xFFFF)
            .unwrap();
        let mut whole = manager.open_packed("meshes\\big.bin", -1, 0xFFFF).unwrap();
        assert_eq!(capped.buffer_size(), 0x20000);
        assert_eq!(whole.buffer_size(), 200_000);
        manager.reset_buffer_limit();
        assert_eq!(manager.archives()[index].open_count(), 3);
    }
    assert_eq!(manager.archives()[index].open_count(), 0);
}

/// Real record hashes, and P03-RUN-006's archive order, masks and lookups, on the shipped archives.
/// IDA: 004A24F0 palette maps, 004A1D80 Get, 004A2000 Add, 0047D8F0 and 0042DB10 record keys.
#[test]
fn texture_palette() {
    assert_eq!(
        normalize_texture_path(b"C:\\Data\\Textures\\A.DDS"),
        b"textures\\a.dds"
    );
    assert_eq!(normalize_texture_path(b"\\data\\x.dds"), b"x.dds");
    assert_eq!(normalize_texture_path(b"DATA\\X"), b"x");
    assert_eq!(
        normalize_texture_path(b"Textures\\Y.dds"),
        b"textures\\y.dds"
    );
    let temp = TempRoot::new("texture_palette");
    let root = &temp.0;
    write_bsa(
        &root.join("Data/T.bsa"),
        &[("textures", vec![file("p.dds", b"P")])],
        3,
        2,
        0x67,
    );
    let settings = ArchiveSettings {
        archive_list: b"T.bsa".to_vec(),
        ..quiet_settings()
    };
    let mut callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new(root, &mut callbacks);
    finder.add("Data");
    let mut manager = ArchiveManager::new(root, settings);
    let _ = manager.initialize_from_settings(&finder, &root.join("local"));
    assert!(manager.first_by_type()[1].is_some(), "the textures slot");

    let mut world = World::new();
    let loose = world.spawn_empty().id();
    let packed = world.spawn_empty().id();
    {
        let mut palette = BsTexturePalette::default();
        assert_eq!(palette.packed_bucket_count(), 1009);
        assert_eq!(palette.loose_bucket_count(), 523);
        palette.add(b"Textures\\Loose.dds", Some(loose), Some(&mut manager));
        palette.add(b"data\\textures\\p.dds", Some(packed), Some(&mut manager));
        assert_eq!(palette.loose_count(), 1, "loose textures key by path");
        assert_eq!(palette.packed_count(), 1, "packed textures key by record");
        assert_eq!(
            palette.find(b"DATA\\TEXTURES\\LOOSE.DDS", Some(&mut manager)),
            Some(loose)
        );
        assert_eq!(
            palette.find(b"textures\\P.dds", Some(&mut manager)),
            Some(packed)
        );
        assert_eq!(palette.find(b"textures\\p.dds", None), None);
        assert_eq!(palette.loose_bucket(b"textures\\loose.dds"), 14, "007DAED0");
        palette.add(b"textures\\loose.dds", Some(packed), Some(&mut manager));
        assert_eq!(palette.loose_count(), 1);
        assert_eq!(
            palette.find(b"textures\\loose.dds", None),
            Some(packed),
            "SetAt replaces an existing key"
        );
        palette.add(b"t\\aaf.dds", Some(loose), None);
        palette.add(b"t\\aqa.dds", Some(loose), None);
        assert_eq!(
            palette.loose_bucket_keys(114),
            [b"t\\aqa.dds".to_vec(), b"t\\aaf.dds".to_vec()],
            "new keys go at the bucket head"
        );
    }

    // 007023E0 services: BSSearchPath candidates, exists callback, palette.
    let slot = SearchPathCreatorSlot {
        bs_search_path_installed: true,
    };
    let mut palette = BsTexturePalette::default();
    {
        let mut services = make_source_texture_services(
            Some(&mut palette),
            Some(&mut manager),
            Some(&finder),
            &callbacks,
            root,
            slot,
        );
        assert_eq!(
            services.search_candidates(b"textures\\p.dds"),
            [b"textures\\p.dds".to_vec()]
        );
        assert!(services.file_exists(b"textures\\p.dds"));
        assert!(!services.file_exists(b"textures\\none.dds"));
        assert_eq!(services.find_texture(b"textures\\p.dds"), None);
        services.register_texture(b"textures\\p.dds", packed);
        assert_eq!(
            services.find_texture(b"Data\\Textures\\P.DDS"),
            Some(packed)
        );
    }
    assert_eq!(palette.packed_count(), 1, "registered into the record map");
    let mut unowned = make_source_texture_services(
        None,
        Some(&mut manager),
        Some(&finder),
        &callbacks,
        root,
        slot,
    );
    unowned.register_texture(b"x.dds", packed);
    assert_eq!(unowned.find_texture(b"x.dds"), None, "a null palette");
}

/// The data directory's parent is the working root, as in the C++ test.
#[test]
fn shipped_archives_match_the_original_run() {
    let data = fixtures::data_dir();
    let root = data.parent().unwrap().canonicalize().unwrap();
    let none = ArchiveInvalidationLists::default();
    let meshes = open(
        &root,
        "Data\\Oblivion - Meshes.bsa",
        false,
        &quiet_settings(),
        &none,
    );
    assert_eq!(meshes.status(), ArchiveLoadStatus::Loaded);
    assert_eq!(meshes.types(), 0x41);
    let mut checked = 0;
    for folder in 0..meshes.folder_count() {
        let name = meshes.folder_name(folder).unwrap();
        assert_eq!(
            bsa_name_hash(name.as_bytes(), None),
            meshes.folders()[folder as usize].hash
        );
        for entry in 0..meshes.folders()[folder as usize].count {
            let file_name = meshes.file_name(folder, entry).unwrap();
            assert_eq!(
                bsa_path_hashes(&file_name).file,
                meshes.folders()[folder as usize].files[entry as usize].hash
            );
            checked += 1;
        }
    }
    assert_eq!(checked, meshes.file_count());

    let scratch = TempRoot::new("real_plugins");
    std::fs::write(
        scratch.0.join("Plugins.txt"),
        "Oblivion.esm\nDLCShiveringIsles.esp\nDLCHorseArmor.esp\nDLCOrrery.esp\nDLCVileLair.esp\n\
         DLCMehrunesRazor.esp\nDLCSpellTomes.esp\nDLCThievesDen.esp\nDLCBattlehornCastle.esp\n\
         DLCFrostcrag.esp\nKnights.esp\n",
    )
    .unwrap();
    let mut callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new(&root, &mut callbacks);
    finder.add("Data");
    let mut manager = ArchiveManager::new(&root, quiet_settings());
    manager.initialize_from_settings(&finder, &scratch.0);
    let expected: [(&str, u16); 17] = [
        ("Oblivion - Meshes.bsa", 0x41),
        ("Oblivion - Textures - Compressed.bsa", 0x02),
        ("Oblivion - Sounds.bsa", 0x08),
        ("Oblivion - Voices1.bsa", 0x10),
        ("Oblivion - Voices2.bsa", 0x10),
        ("Oblivion - Misc.bsa", 0x1A4),
        ("DLCShiveringIsles - Meshes.bsa", 0x41),
        ("DLCShiveringIsles - Sounds.bsa", 0x08),
        ("DLCShiveringIsles - Textures.bsa", 0x02),
        ("DLCShiveringIsles - Voices.bsa", 0x10),
        ("DLCHorseArmor.bsa", 0x1B),
        ("DLCOrrery.bsa", 0x1B),
        ("DLCVileLair.bsa", 0x1B),
        ("DLCThievesDen.bsa", 0x1B),
        ("DLCBattlehornCastle.bsa", 0x1B),
        ("DLCFrostcrag.bsa", 0x1B),
        ("Knights.bsa", 0x1B),
    ];
    let order: Vec<String> = manager.archives().iter().map(Archive::path).collect();
    assert_eq!(order.len(), expected.len(), "{order:?}");
    for (index, (name, types)) in expected.iter().enumerate() {
        let archive = &manager.archives()[index];
        assert_eq!(archive.path(), format!("Data\\{name}"));
        assert_eq!(archive.types(), *types, "{name}");
        assert_eq!(archive.archive_flags() & 3, 3, "{name}");
    }
    {
        let mut texture = manager
            .open_packed("Data\\Textures\\Effects\\eyereflection.dds", 0x2800, 0xFFFF)
            .unwrap();
        let mut header = [0; 8];
        assert_eq!(texture.read(&mut header), 8);
        assert_eq!(&header[..4], b"DDS ");
        assert_eq!(header[4], 124);
        let size = texture.size() as usize;
        let mut rest = vec![0; size];
        assert_eq!(texture.read(&mut rest), size - 8, "inflates completely");
    }
    assert_eq!(manager.find_packed("FaceGen\\si.ctl", 0xFFFF), Some(5));
    assert_eq!(
        manager.find_packed("Data\\Shaders\\shaderpackage013.sdp", 0xFFFF),
        None
    );
    assert_eq!(
        manager.find_packed("Data\\Textures\\Effects\\eyereflection.dds", 0xFFFF),
        Some(1)
    );
    assert_eq!(
        manager.find_packed("Data\\DistantLOD\\TestTree.cmp", 0xFFFF),
        None
    );
    assert_eq!(manager.cached(), Some(1));
}
