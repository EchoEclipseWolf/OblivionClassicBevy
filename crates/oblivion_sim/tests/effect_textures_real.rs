//! The NiDDSReader and the 00720F80 cube path over the shipped archives, against P05-RUN-002's memory. The C++ tests only read
//! hand-built files here, so this goes beyond them. Host tests earn no validation bonus.

use std::cell::RefCell;
use std::path::PathBuf;

use bevy_ecs::prelude::*;
use oblivion_core::fixtures;
use oblivion_sim::bs::archive::{ArchiveManager, ArchiveSettings};
use oblivion_sim::bs::file::{GameFile, OpenedFile};
use oblivion_sim::bs::file_finder::{
    BsFileCallbacks, FileFinder, bs_file_exists, bs_file_open_core,
};
use oblivion_sim::gamebryo::device_caps::{DeviceCapsFixture, PinnedRendererPlatform};
use oblivion_sim::gamebryo::dx9_system::{
    D3D_OK, D3DFMT_R5G6B5, D3DFMT_X1R5G5B5, D3DFMT_X8R8G8B8, D3DRTYPE_TEXTURE, D3DXERR_INVALIDDATA,
    DeviceCall, WindowHandle,
};
use oblivion_sim::gamebryo::pixel_data::PixelData;
use oblivion_sim::gamebryo::renderer::{
    RendererCreation, RendererCreationInputs, renderer_settings_from,
};
use oblivion_sim::gamebryo::texture::SourceTexture;
use oblivion_sim::gamebryo::texture_loading::{
    DirectResourceType, TextureLoadServices, TextureRendererBoundary, create_cube_from_files,
    create_texture_from_file,
};
use oblivion_sim::tes::os::renderer_init::initialize_renderer;
use oblivion_sim::tes::settings::{SettingCollections, register_static_settings};

/// This test's rows of `fixtures/rows/phase05.rows`: `run|CUBE|slot|levels|faces|bytes|w0|format|fnvs|line`.
fn cube_row(slot: &str) -> Vec<String> {
    fixtures::rows("phase05")
        .into_iter()
        .filter_map(|row| row.strip_prefix("TARGETS|").map(str::to_owned))
        .map(|row| row.split('|').map(str::to_owned).collect::<Vec<_>>())
        .find(|row| row[1] == "CUBE" && row[2] == slot)
        .unwrap_or_else(|| panic!("no row for {slot}"))
}

fn boxed(file: OpenedFile) -> Box<dyn GameFile> {
    match file {
        OpenedFile::Ni(file) => Box::new(file),
        OpenedFile::Loose(file) => Box::new(file),
        OpenedFile::Packed(file) => Box::new(file),
        OpenedFile::Compressed(file) => Box::new(file),
    }
}

fn path_text(path: &[u8]) -> String {
    path.iter().map(|&byte| char::from(byte)).collect()
}

fn face_fnv(data: &PixelData, face: u32) -> u32 {
    let mut hash = 0x811C_9DC5u32;
    for &byte in data.level(face, 0) {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// IDA: N/A `RendererCreation` from the pinned caps, the renderer of the run.
fn pinned_renderer() -> RendererCreation {
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    collections
        .ini
        .set_path(fixtures::root().join("config/Oblivion.ini"));
    assert!(collections.ini.open_ini());
    collections.ini.read_all();
    collections.ini.close();
    let inputs = RendererCreationInputs {
        main_window: Some(WindowHandle(0x0871_1BAC)),
        instance: 0x0040_0000,
        window_class: "Oblivion".to_owned(),
        my_games_root: PathBuf::from("/virtual/My Games/Oblivion"),
        adapter_formats: vec![D3DFMT_X8R8G8B8, D3DFMT_R5G6B5, D3DFMT_X1R5G5B5],
        settings: renderer_settings_from(&collections.ini),
    };
    let mut platform = PinnedRendererPlatform::new(DeviceCapsFixture::pinned());
    let created = initialize_renderer(&inputs, &mut platform);
    assert!(created.succeeded);
    created
}

/// This test's rows: `run|TEX|slot|renderer data vtable|w|h|levels|bytes|line`.
fn texture_row(slot: &str) -> Vec<String> {
    fixtures::rows("phase05")
        .into_iter()
        .filter_map(|row| row.strip_prefix("TARGETS|").map(str::to_owned))
        .map(|row| row.split('|').map(str::to_owned).collect::<Vec<_>>())
        .find(|row| row[1] == "TEX" && row[2] == slot)
        .unwrap_or_else(|| panic!("no row for {slot}"))
}

#[test]
fn effect_cubes_match_p05_run_002() {
    let data = fixtures::data_dir();
    let root = data.parent().unwrap().canonicalize().unwrap();
    let scratch = std::env::temp_dir().join(format!("recomp_effect_real_{}", std::process::id()));
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
    let _ = std::fs::remove_dir_all(&scratch);

    let mut creation = pinned_renderer();
    let archives = RefCell::new(manager);
    let mut services = TextureLoadServices {
        open_file: Some(Box::new(|path| {
            bs_file_open_core(
                &finder,
                Some(&mut archives.borrow_mut()),
                &path_text(path),
                0,
                0x8000,
                0xFFFF,
            )
            .map(boxed)
        })),
        file_exists: Some(Box::new(|path| {
            bs_file_exists(
                &callbacks,
                &root,
                Some(&finder),
                Some(&mut archives.borrow_mut()),
                &path_text(path),
                0,
            )
        })),
        renderer: Some(&mut creation),
        ..TextureLoadServices::default()
    };
    let mut world = World::new();
    for (slot, name) in [
        (
            "B430F4",
            b"Data\\Textures\\Effects\\reflection.dds".as_slice(),
        ),
        (
            "B43114",
            b"Data\\Textures\\Effects\\eyereflection.dds".as_slice(),
        ),
    ] {
        let row = cube_row(slot);
        let cube = create_cube_from_files(&mut world, [name; 6], [6, 3, 2], &mut services).unwrap();
        let cube = world.get::<SourceTexture>(cube).unwrap();
        let pixels = world
            .get::<PixelData>(cube.app_pixel_data_3c.expect("all six faces read"))
            .unwrap();
        assert_eq!(pixels.levels_60.to_string(), row[3], "{slot}");
        assert_eq!(pixels.faces_6c.to_string(), row[4], "{slot}");
        assert_eq!(pixels.level(0, 0).len().to_string(), row[5], "{slot}");
        assert_eq!(pixels.widths_54[0].to_string(), row[6], "{slot}");
        let format = pixels.format_08;
        assert_eq!(
            format!(
                "{:02x}{:02x}0000{:08x}{:08x}{:08x}{:08x}",
                format.flags_00,
                format.bits_per_pixel_01,
                format.format_04.swap_bytes(),
                format.tiling_08.swap_bytes(),
                format.renderer_hint_0c.swap_bytes(),
                format.extra_10.swap_bytes()
            ),
            row[7],
            "{slot}"
        );
        for (face, wanted) in row[8].split(',').enumerate() {
            assert_eq!(
                format!("{:08X}", face_fnv(pixels, face as u32)),
                wanted,
                "{slot} face {face}"
            );
        }
        // The cube's renderer data, through the pinned device.
        let row = texture_row(slot);
        let data = cube.renderer_24.as_ref().expect("renderer data");
        assert_eq!(format!("{:X}", data.original_vtable), row[3], "{slot}");
        assert_eq!(data.width_54.to_string(), row[4], "{slot}");
        assert_eq!(data.height_58.to_string(), row[5], "{slot}");
        assert_eq!(data.levels_5c.to_string(), row[6], "{slot}");
        assert_eq!(data.texture_bytes_60.to_string(), row[7], "{slot}");
    }
}

/// `SCENE|run|RDATA|slot|..|+0C..+1F hex|name` of P05-RUN-001: the renderer data's pixel format, for the five file-backed slots.
fn format_row(slot: &str) -> String {
    let prefix = format!("SCENE|P05-RUN-001|RDATA|{slot}|");
    let rows = fixtures::rows("phase05");
    let row = rows
        .iter()
        .find(|row| row.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no row for {slot}"));
    row.split('|').nth(13).unwrap().to_owned()
}

/// The five file-backed slots through the 00760DA0 direct load: the D3DX answers come from each shipped file, and everything the runs recorded of
/// the renderer data follows. Every field here is the file's, so nothing is filled in; `pixel_bytes_68` was not recorded by either run.
#[test]
fn effect_file_slots_match_the_runs() {
    let data = fixtures::data_dir();
    let root = data.parent().unwrap().canonicalize().unwrap();
    let scratch = std::env::temp_dir().join(format!("recomp_effect_files_{}", std::process::id()));
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
    let _ = std::fs::remove_dir_all(&scratch);

    let mut creation = pinned_renderer();
    let first_call = creation.device_calls.len();
    let archives = RefCell::new(manager);
    let mut services = TextureLoadServices {
        open_file: Some(Box::new(|path| {
            bs_file_open_core(
                &finder,
                Some(&mut archives.borrow_mut()),
                &path_text(path),
                0,
                0x8000,
                0xFFFF,
            )
            .map(boxed)
        })),
        renderer: Some(&mut creation),
        ..TextureLoadServices::default()
    };
    let mut world = World::new();
    let mut levels = Vec::new();
    for (slot, name) in [
        ("B43100", "WindowEnvironmentMap01"),
        ("B430E4", "WindowEnvironmentMap02"),
        ("B430F8", "aniso"),
        ("B43128", "blooddecal"),
        ("B430E8", "terrainnoise"),
    ] {
        let path = format!("Data\\Textures\\Effects\\{name}.dds");
        let texture =
            create_texture_from_file(&mut world, path.as_bytes(), [6, 3, 2], true, &mut services)
                .unwrap_or_else(|| panic!("{slot} is null"));
        let texture = world.get::<SourceTexture>(texture).unwrap();
        assert!(texture.app_pixel_data_3c.is_none(), "{slot} went direct");
        let rendered = texture.renderer_24.as_ref().expect("renderer data");
        assert!(rendered.direct_load && rendered.device_texture, "{slot}");
        let row = texture_row(slot);
        assert_eq!(format!("{:X}", rendered.original_vtable), row[3], "{slot}");
        assert_eq!(rendered.width_54.to_string(), row[4], "{slot}");
        assert_eq!(rendered.height_58.to_string(), row[5], "{slot}");
        assert_eq!(rendered.levels_5c.to_string(), row[6], "{slot}");
        assert_eq!(rendered.texture_bytes_60.to_string(), row[7], "{slot}");
        let format = rendered.pixel_format_0c;
        assert_eq!(
            format!(
                "{:02x}{:02x}0000{:08x}{:08x}{:08x}{:08x}",
                format.flags_00,
                format.bits_per_pixel_01,
                format.format_04.swap_bytes(),
                format.tiling_08.swap_bytes(),
                format.renderer_hint_0c.swap_bytes(),
                format.extra_10.swap_bytes()
            ),
            format_row(slot),
            "{slot}"
        );
        levels.push(rendered.levels_5c);
    }
    drop(services);

    // The trace: for each load the two D3DX calls, the level count, the type and one GetLevelDesc per level, each at its original site.
    let mut calls = creation.device_calls[first_call..].iter();
    for count in levels {
        let info = calls.next().unwrap();
        assert_eq!(info.site, 0x0076_0EF5);
        assert_eq!(info.result, D3D_OK);
        let DeviceCall::D3dxGetImageInfoFromFileInMemory { size } = info.call else {
            panic!("{info:?}");
        };
        let create = calls.next().unwrap();
        assert_eq!(create.site, 0x0076_10D3);
        assert_eq!(create.result, D3D_OK);
        assert_eq!(
            create.call,
            DeviceCall::D3dxCreateTextureFromFileInMemory {
                resource_type: D3DRTYPE_TEXTURE,
                size,
            }
        );
        let level_count = calls.next().unwrap();
        assert_eq!(level_count.site, 0x0076_09D3);
        assert_eq!(level_count.call, DeviceCall::TextureGetLevelCount);
        assert_eq!(level_count.result, count);
        let kind = calls.next().unwrap();
        assert_eq!((kind.site, kind.result), (0x0076_09EC, D3DRTYPE_TEXTURE));
        for level in 0..count {
            let desc = calls.next().unwrap();
            assert_eq!(desc.site, 0x0076_0A0C);
            assert_eq!(desc.call, DeviceCall::TextureGetLevelDesc { level });
        }
    }
    assert!(calls.next().is_none());

    // The failure branch: a file D3DX cannot read answers on the trace, and the caller sees null.
    let before = creation.device_calls.len();
    assert_eq!(creation.image_info(b"not a dds"), None);
    assert_eq!(
        creation.create_direct_texture(DirectResourceType::Texture, b"not a dds", 9),
        None
    );
    let failed = &creation.device_calls[before..];
    assert_eq!(failed.len(), 2);
    assert!(
        failed
            .iter()
            .all(|entry| entry.result == D3DXERR_INVALIDDATA)
    );
}
