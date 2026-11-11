//! `verify_facegen_real` from the C++ `phase04_tests`: 00553550/00553140 over the real archives (Oblivion - Misc.bsa),
//! compared with P04-RUN-002's row and the float32 reference `evidence/.../facegen/ctl_reference.py`. Host tests earn no
//! validation bonus.

use oblivion_core::fixtures;
use oblivion_core::hash::{FNV1A_BASIS, fnv1a_dword};
use oblivion_sim::bs::archive::{ArchiveManager, ArchiveSettings};
use oblivion_sim::bs::file_finder::{BsFileCallbacks, FileFinder};
use oblivion_sim::gamebryo::device_caps::{DeviceCapsFixture, PinnedRendererPlatform};
use oblivion_sim::gamebryo::dx9_system::{
    D3DFMT_R5G6B5, D3DFMT_X1R5G5B5, D3DFMT_X8R8G8B8, WindowHandle,
};
use oblivion_sim::gamebryo::renderer::{RendererCreationInputs, renderer_settings_from};
use oblivion_sim::tes::facegen::*;
use oblivion_sim::tes::facegen::{FaceGenTextureRequest, renderer_facegen_upload};
use oblivion_sim::tes::os::renderer_init::initialize_renderer;
use oblivion_sim::tes::settings::{SettingCollections, register_static_settings};
use std::path::PathBuf;

fn fnv_floats(mut hash: u32, values: &[f32]) -> u32 {
    for value in values {
        hash = fnv1a_dword(hash, value.to_bits());
    }
    hash
}

/// The value of a `field=` of the row, up to the next space.
fn field<'a>(row: &'a str, name: &str) -> &'a str {
    row.split_whitespace()
        .find_map(|part| part.strip_prefix(name))
        .unwrap_or_else(|| panic!("{name} in {row}"))
}

fn hex_field(row: &str, name: &str) -> u32 {
    u32::from_str_radix(field(row, name), 16).unwrap()
}

/// `P04-RUN-002`'s FaceGen snapshot, once it has been found verbatim in the run's records.
fn facegen_row() -> String {
    let rows = fixtures::rows("phase04");
    let row = rows
        .iter()
        .filter_map(|row| row.strip_prefix("P04-RUN-002 "))
        .find(|row| row.contains(" facegen="))
        .expect("the P04-RUN-002 facegen row")
        .to_owned();
    let path = fixtures::root().join("evidence/phase04-run002-2026-10-06.records.txt");
    let evidence = std::fs::read_to_string(path).unwrap();
    assert!(
        evidence.lines().any(|line| line == row),
        "not in the records: {row}"
    );
    row
}

#[test]
fn verify_facegen_real() {
    let row = facegen_row();
    let data = fixtures::data_dir();
    let root = data.parent().unwrap().canonicalize().unwrap();
    let scratch = std::env::temp_dir().join(format!("recomp_facegen_real_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join("Plugins.txt"), "Oblivion.esm\n").unwrap();

    let mut callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new(&root, &mut callbacks);
    finder.add("Data");
    let settings = ArchiveSettings {
        invalidate_older_files: false,
        retain_directory_strings: 2,
        ..ArchiveSettings::default()
    };
    let mut manager = ArchiveManager::new(&root, settings);
    manager.initialize_from_settings(&finder, &scratch);

    let mut singleton = FaceGenSingleton::default();
    let mut error = FaceGenLastError::default();
    ensure_facegen_initialized(
        &mut singleton,
        &mut error,
        FaceGenInputs {
            finder: &finder,
            archives: Some(&mut manager),
            create_texture: None,
            texture_prefs: [6, 3, 2],
            mipmap_by_default: false,
            device_image_converter: false,
            object_load_callbacks_enabled: false,
        },
    );
    let _ = std::fs::remove_dir_all(&scratch);

    let f = singleton.0.as_ref().unwrap();
    assert!(
        f.ctl_loaded,
        "si.ctl must load on the first attempt (P04-RUN-001)"
    );
    assert_eq!(f.load_attempts, 1);
    assert!(!f.load_warning);
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    assert_eq!(hex_field(&row, "loaded="), 1);

    assert_eq!(f.header_words, [0x7745_C425, 0x51]);
    assert_eq!(f.counts, [50, 30, 50, 0]);
    // The row's `header=` is FaceGen +0: the two words and the head of +0x008's first matrix.
    let header = field(&row, "header=");
    assert_eq!(&header[..16], "25c4457751000000");
    assert_eq!(&header[16..32], "3200000001000000");

    assert_eq!(f.linear_controls[0].len(), 56);
    assert_eq!(f.linear_controls[1].len(), 26);
    assert_eq!(f.linear_controls[2].len(), 33);
    assert!(f.linear_controls[3].is_empty());
    // The row's `counts=` are these linear-control counts, not +0xD9C's.
    let counts = row
        .split("counts=[")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .unwrap();
    let recorded: Vec<usize> = counts
        .split(", ")
        .map(|count| count.parse().unwrap())
        .collect();
    let lengths: Vec<usize> = f.linear_controls.iter().map(Vec::len).collect();
    assert_eq!(recorded, lengths);
    assert_eq!(f.linear_controls[0][0].name, b"Brow Ridge - high / low");
    assert_eq!(f.linear_controls[2][0].name, b"Beard Flushed / Pale");

    assert_eq!(f.coefficients[0].rows, 50);
    assert_eq!(f.coefficients[0].cols, 1);
    assert_eq!(f.coefficients[1].rows, 30);
    assert_eq!(f.coefficients[3].rows, 0);
    assert_eq!(f.coefficients[2].data.len(), 50);

    let mut linear = FNV1A_BASIS;
    for controls in &f.linear_controls {
        for control in controls {
            linear = fnv_floats(linear, &control.coefficients.data);
        }
    }
    let mut stats = FNV1A_BASIS;
    for group in &f.fan_controls.groups {
        for matrix in &group.matrices {
            stats = fnv_floats(stats, &matrix.data);
        }
    }
    let mut ortho = FNV1A_BASIS;
    let mut inverse = FNV1A_BASIS;
    for g in 0..5 {
        for t in 0..2 {
            ortho = fnv_floats(ortho, &f.fan_controls.orthonormal[g][t].first.data);
            ortho = fnv_floats(ortho, &f.fan_controls.orthonormal[g][t].second.data);
            inverse = fnv_floats(inverse, &f.fan_controls.inverse_gram[g][t]);
        }
    }
    assert_eq!(linear, hex_field(&row, "linear="), "linear controls");
    assert_eq!(stats, hex_field(&row, "stats="), "group statistics");
    assert_eq!(ortho, hex_field(&row, "ortho="), "Gram-Schmidt");
    assert_eq!(inverse, hex_field(&row, "inverse="), "inverse Gram");

    // From the float32 reference.
    let fan = &f.fan_controls;
    assert_eq!(
        fan.group_controls[0][0][0].inverse_norm_squared.to_bits(),
        0x3C8B_71EE
    );
    assert_eq!(
        fan.pair_controls[0][1].inverse_norm_squared.to_bits(),
        0x408A_BE72
    );
    assert_eq!(fan.orthonormal[0][0].first.data[0].to_bits(), 0xBE40_9831);
    assert_eq!(fan.orthonormal[0][0].second.data[0].to_bits(), 0xBDA6_8DBD);
    assert_eq!(fan.inverse_gram[0][0][0].to_bits(), 0x3C8D_4541);
    assert_eq!(fan.inverse_gram[0][0][1].to_bits(), 0xBC88_E633);
    assert_eq!(fan.inverse_gram[0][0][3].to_bits(), 0x3FA0_6A2B);

    // The row's `alpha=` ends with the property's +0x18 flags and +0x1A reference.
    assert_eq!(f.alpha.flags, 0x12EC);
    assert_eq!(f.alpha.test_reference, 100);
    let alpha = field(&row, "alpha=");
    assert_eq!(&alpha[alpha.len() - 8..], "ec126400");

    assert_eq!(f.textures[0].grey, 0x80);
    assert_eq!(f.textures[1].grey, 0x40);
    assert_eq!(f.textures[0].rgb24.len(), 32 * 32 * 3);
    assert_eq!(f.textures[1].rgb24.last(), Some(&0x40));
    assert!(!f.textures[0].upload.texture);
    assert_eq!(error.code, 0, "The real load must not record an error");
}

/// 0040E893 after the renderer: the two 32x32 textures go through the pinned device as P04-RUN-002 recorded them, and the
/// app pixel data is dropped once uploaded (00701C26, the 0040E62F gate).
#[test]
fn facegen_uploads_through_the_renderer() {
    let data = fixtures::data_dir();
    let root = data.parent().unwrap().canonicalize().unwrap();
    let scratch = std::env::temp_dir().join(format!("recomp_facegen_up_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join("Plugins.txt"), "Oblivion.esm\n").unwrap();
    let mut callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new(&root, &mut callbacks);
    finder.add("Data");
    let settings = ArchiveSettings {
        invalidate_older_files: false,
        retain_directory_strings: 2,
        ..ArchiveSettings::default()
    };
    let mut manager = ArchiveManager::new(&root, settings);
    manager.initialize_from_settings(&finder, &scratch);

    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    collections
        .ini
        .set_path(fixtures::root().join("config/Oblivion.ini"));
    assert!(collections.ini.open_ini());
    collections.ini.read_all();
    collections.ini.close();
    let mut platform = PinnedRendererPlatform::new(DeviceCapsFixture::pinned());
    let mut created = initialize_renderer(
        &RendererCreationInputs {
            main_window: Some(WindowHandle(0x0871_1BAC)),
            instance: 0x0040_0000,
            window_class: "Oblivion".to_owned(),
            my_games_root: PathBuf::from("/virtual/My Games/Oblivion"),
            adapter_formats: vec![D3DFMT_X8R8G8B8, D3DFMT_R5G6B5, D3DFMT_X1R5G5B5],
            settings: renderer_settings_from(&collections.ini),
        },
        &mut platform,
    );
    assert!(created.succeeded);
    let before = created.device_calls.len();

    let mut singleton = FaceGenSingleton::default();
    let mut error = FaceGenLastError::default();
    let mut create =
        |request: &FaceGenTextureRequest<'_>| renderer_facegen_upload(&mut created, request);
    ensure_facegen_initialized(
        &mut singleton,
        &mut error,
        FaceGenInputs {
            finder: &finder,
            archives: Some(&mut manager),
            create_texture: Some(&mut create),
            texture_prefs: [6, 3, 2],
            mipmap_by_default: false,
            device_image_converter: true,
            object_load_callbacks_enabled: true,
        },
    );
    let _ = std::fs::remove_dir_all(&scratch);

    let f = singleton.0.as_ref().unwrap();
    for texture in &f.textures {
        assert!(texture.upload.texture);
        assert_eq!(texture.upload.format, 22, "C 4 7607E3 fmt=22");
        assert!(texture.rgb24.is_empty());
    }
    let creates = created.device_calls[before..]
        .iter()
        .filter(|entry| entry.site == 0x0076_07E3)
        .count();
    assert_eq!(creates, 2, "P04-RUN-002 records two CreateTexture calls");
}
