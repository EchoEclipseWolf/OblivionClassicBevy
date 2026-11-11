//! WinMain 0040E89D..0040EAC1, Phase 05's whole window, as one sequence on the pinned caps and the shipped data, against P05-RUN-001 and
//! P05-RUN-002. The separate fixtures in `oblivion_sim` check each step; this one checks that the steps, joined in original order, leave the
//! values the runs recorded, since a later step's input is an earlier step's output. Host tests earn no validation bonus.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use oblivion_core::fixtures;
use oblivion_sim::bs::archive::{ArchiveManager, ArchiveSettings};
use oblivion_sim::bs::effect_textures::{EffectTextureGlobals, ScreenPolygon};
use oblivion_sim::bs::file::{GameFile, OpenedFile};
use oblivion_sim::bs::file_finder::{
    BsFileCallbacks, FileFinder, bs_file_exists, bs_file_open_as_bs_file, bs_file_open_core,
};
use oblivion_sim::bs::render_targets::RenderedTexture;
use oblivion_sim::bs::scene_graphs::{ClearZNode, SceneGraph, ShadowSceneNode};
use oblivion_sim::bs::shader::BsShaderSlots;
use oblivion_sim::bs::shader_classes_geometry::DebugShader;
use oblivion_sim::bs::texture_palette::BsTexturePalette;
use oblivion_sim::gamebryo::av_object::NiAvProperties;
use oblivion_sim::gamebryo::camera::SceneCamera;
use oblivion_sim::gamebryo::device_caps::{DeviceCapsFixture, PinnedRendererPlatform};
use oblivion_sim::gamebryo::dx9_system::{
    D3D_OK, D3DFMT_R5G6B5, D3DFMT_X1R5G5B5, D3DFMT_X8R8G8B8, D3DPOOL_DEFAULT, D3DRTYPE_TEXTURE,
    DeviceCall, DeviceTraceEntry, WindowHandle,
};
use oblivion_sim::gamebryo::mesh_geometry::NiGeometry;
use oblivion_sim::gamebryo::pixel_data::{PixelData, PixelFormat};
use oblivion_sim::gamebryo::properties::{FogProperty, WireframeProperty};
use oblivion_sim::gamebryo::registration::{
    StartupCallbackScheduler, register_known_crt_callbacks, set_device_image_converter_default,
};
use oblivion_sim::gamebryo::render_targets::{
    DepthStencilBuffer, RenderTargetGroup, RenderedTextureObject, rendered_texture_object_size,
};
use oblivion_sim::gamebryo::renderer::{
    RendererCreation, RendererCreationInputs, renderer_settings_from,
};
use oblivion_sim::gamebryo::scene_node::{
    NI_NODE_VTABLE, NiAvWorld, NiNodeChildren, SceneNodePayload,
};
use oblivion_sim::gamebryo::scene_update::NoSceneControllers;
use oblivion_sim::gamebryo::screen_elements::ScreenElementsQuad;
use oblivion_sim::gamebryo::texture::SourceTexture;
use oblivion_sim::tes::os::renderer_init::initialize_renderer;
use oblivion_sim::tes::os_globals::{HostInstance, OsInputGlobals, ScriptedOsPlatform};
use oblivion_sim::tes::settings::{SettingCollections, register_static_settings};
use oblivion_sim::tes::winmain::winmain_scene::{
    SceneWindow, SceneWindowServices, run_scene_window,
};

const RUN_FILES: [(&str, &str); 2] = [
    ("P05-RUN-001", "phase05-run001-2026-10-06"),
    ("P05-RUN-002", "phase05-run002-2026-10-06"),
];

fn run_prefix(run: &str) -> &'static str {
    RUN_FILES
        .iter()
        .find(|(name, _)| *name == run)
        .unwrap_or_else(|| panic!("unknown run {run}"))
        .1
}

/// One block's rows of `phase05.rows`, without the block's prefix, split on `|`.
fn block(prefix: &str) -> Vec<Vec<String>> {
    fixtures::rows("phase05")
        .into_iter()
        .filter(|row| row.starts_with(prefix))
        .map(|row| {
            row[prefix.len()..]
                .split('|')
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn rows(prefix: &str, kind: &str) -> Vec<Vec<String>> {
    block(prefix)
        .into_iter()
        .filter(|row| row[1] == kind)
        .collect()
}

fn hex_bytes(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn records(run: &str) -> String {
    let path = fixtures::root()
        .join("evidence")
        .join(format!("{}.records.txt", run_prefix(run)));
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
    bytes.iter().map(|&byte| char::from(byte)).collect()
}

/// A memory snapshot: the first line's address and the bytes.
fn snapshot(run: &str, file: &str) -> (usize, Vec<u8>) {
    let path = fixtures::root()
        .join("evidence")
        .join(format!("{}.snap-{file}.hex", run_prefix(run)));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
    let mut base = None;
    let mut bytes = Vec::new();
    for line in text.lines() {
        let (address, data) = line.split_once(' ').expect("address and bytes");
        let address = usize::from_str_radix(address, 16).unwrap();
        let first = *base.get_or_insert(address);
        assert_eq!(address, first + bytes.len(), "{file} is contiguous");
        bytes.extend(hex_bytes(data.trim()));
    }
    (base.expect("a line"), bytes)
}

/// Every `SCENE` row against the evidence it came from.
#[test]
fn scene_rows_match_their_evidence() {
    let mut count = 0;
    for row in block("SCENE|") {
        let run = row[0].as_str();
        match row[1].as_str() {
            "SNAP" | "PTR" => {
                let (file, value) = (&row[2], row.last().unwrap());
                let (base, bytes) = snapshot(run, file);
                let offset = |text: &str| {
                    let number = usize::from_str_radix(text, 16).unwrap();
                    // The accumulator tail's offsets are from the object's start, as the head file's first line gives it; the globals'
                    // are addresses.
                    if file == "accumulator-tail" {
                        number + snapshot(run, "accumulator-head").0 - base
                    } else if file.starts_with("globals") {
                        number - base
                    } else {
                        number
                    }
                };
                if row[1] == "SNAP" {
                    let at = offset(&row[4]);
                    let wanted = hex_bytes(value);
                    assert_eq!(bytes[at..at + wanted.len()], wanted, "{row:?}");
                } else {
                    let at = offset(&row[3]);
                    let mut wanted = hex_bytes(value);
                    wanted.reverse();
                    assert_eq!(bytes[at..at + 4], wanted, "{row:?}");
                }
            }
            "RDATA" => {
                let text = records(run);
                let line = text
                    .lines()
                    .find(|line| {
                        line.contains(&format!("tex@{}=", row[2])) && line.contains(" rdata=")
                    })
                    .unwrap_or_else(|| panic!("no record for {row:?}"));
                assert!(line.contains(&format!("{{vt={} ", row[3])), "{row:?}");
                assert!(line.contains(&row[4]), "{row:?}");
                assert!(
                    line.contains("rdata=") && line.contains(&format!("{{vt={} ", row[11])),
                    "{row:?}"
                );
                assert!(line.contains(&row[12]), "{row:?}");
                assert!(line.contains(&format!("name={}", row[13])), "{row:?}");
            }
            "POLYGON" => {
                assert!(
                    records(run).contains(&format!("tex@{}=", row[2])),
                    "{row:?}"
                );
            }
            "RET" => {
                let text = records(run);
                assert!(
                    text.contains(&format!(" {}={}", row[2], row[3]))
                        || text.contains(&format!("@{}={}", row[2], row[3])),
                    "{row:?}"
                );
            }
            "ORDER" => {
                let text = records(run);
                let line = text
                    .lines()
                    .find(|line| line.split(' ').nth(1) == Some(row[3].as_str()))
                    .unwrap_or_else(|| panic!("no record {row:?}"));
                assert_eq!(line.split(' ').nth(2), Some(row[2].as_str()), "{row:?}");
            }
            other => panic!("unknown SCENE kind {other}"),
        }
        count += 1;
    }
    assert!(count > 300, "{count} rows");
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

/// The run's `{flags}{bits}0000{format}{tiling}{hint}{extra}` dump of a NiPixelFormat.
fn format_dump(format: &PixelFormat) -> String {
    format!(
        "{:02x}{:02x}0000{:08x}{:08x}{:08x}{:08x}",
        format.flags_00,
        format.bits_per_pixel_01,
        format.format_04.swap_bytes(),
        format.tiling_08.swap_bytes(),
        format.renderer_hint_0c.swap_bytes(),
        format.extra_10.swap_bytes()
    )
}

fn pinned_collections() -> SettingCollections {
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    collections
        .ini
        .set_path(fixtures::root().join("config/Oblivion.ini"));
    assert!(collections.ini.open_ini());
    collections.ini.read_all();
    collections.ini.close();
    collections
}

fn pinned_renderer(collections: &SettingCollections) -> RendererCreation {
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

fn slot_of(textures: &EffectTextureGlobals, slot: &str) -> Option<Entity> {
    match slot {
        "B430D4" => textures.radial_alias,
        "B430D8" => None, // No writer before 0040EAC8, in the run or here.
        "B430DC" => textures.flat_normal,
        "B430E0" => textures.black,
        "B430E4" => textures.window_environment_02,
        "B430E8" => textures.terrain_noise,
        "B430EC" => textures.normalization_cube,
        "B430F0" => textures.flat_normal_alpha,
        "B430F4" => textures.reflection_cube,
        "B430F8" => textures.aniso,
        "B43100" => textures.window_environment_01,
        "B43110" => textures.radial,
        "B43114" => textures.eye_reflection_cube,
        "B43118" => textures.screen_polygon,
        "B4311C" => textures.local_map_tint,
        "B43120" => textures.opaque_black,
        "B43128" => textures.blood_decal,
        other => panic!("no slot {other}"),
    }
}

fn le_f32(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// The port's bytes for one `SNAP` row of an AV object.
fn av_object_bytes(world: &World, object: Entity, label: &str, vtable: u32) -> Option<Vec<u8>> {
    let local = world
        .get::<SceneNodePayload>(object)
        .map(|payload| payload.local_transform);
    let av_world = world.get::<NiAvWorld>(object)?;
    Some(match label {
        "vtable" => vtable.to_le_bytes().to_vec(),
        "flags" => world
            .get::<SceneNodePayload>(object)?
            .effective_av_flags
            .to_le_bytes()
            .to_vec(),
        "local_translation" => le_f32(&local?.translation),
        "world_translation" => le_f32(&av_world.transform.translation),
        "properties" => (world
            .get::<NiAvProperties>(object)
            .map_or(0, |list| list.0.len()) as u32)
            .to_le_bytes()
            .to_vec(),
        "children" => (world.get::<NiNodeChildren>(object)?.slots_ac.len() as u16)
            .to_le_bytes()
            .to_vec(),
        _ => return None,
    })
}

fn camera_bytes(world: &World, graph: Entity, label: &str) -> Vec<u8> {
    let entity = world.get::<SceneGraph>(graph).unwrap().camera_dc.unwrap();
    let camera = world.get::<SceneCamera>(entity).unwrap();
    let av = world.get::<NiAvWorld>(entity).unwrap();
    let f = camera.frustum_ec;
    let v = camera.viewport_110;
    match label {
        "vtable" => camera.original_vtable.to_le_bytes().to_vec(),
        "flags" => camera.av_flags_18.to_le_bytes().to_vec(),
        "bound" => le_f32(&av.bound),
        "local_rotation" => le_f32(&camera.local_transform_30.rotation),
        "local_translation" => le_f32(&camera.local_transform_30.translation),
        "local_scale" => le_f32(&[camera.local_transform_30.scale]),
        "world_rotation" => le_f32(&av.transform.rotation),
        "world_translation" => le_f32(&av.transform.translation),
        "world_scale" => le_f32(&[av.transform.scale]),
        "world_to_clip" => le_f32(&camera.world_to_clip_ac),
        "frustum" => le_f32(&[f.left, f.right, f.top, f.bottom, f.near_plane, f.far_plane]),
        "ortho" => vec![u8::from(f.orthographic)],
        "near_min" => le_f32(&[camera.min_near_plane_108]),
        "ratio" => le_f32(&[camera.max_far_near_ratio_10c]),
        "viewport" => le_f32(&[v.left, v.right, v.top, v.bottom]),
        "lod" => le_f32(&[camera.lod_adjust_120]),
        other => panic!("no camera label {other}"),
    }
}

/// The port's bytes for one `SNAP` row, by file and label. An unmapped row is a failure, so nothing here is skipped by accident.
fn snap_bytes(
    world: &World,
    window: &SceneWindow,
    creation: &RendererCreation,
    flags_before: u32,
    file: &str,
    label: &str,
) -> Vec<u8> {
    let scene = &window.scene_graphs;
    let u8s = |value: bool| vec![u8::from(value)];
    if let Some(name) = file.strip_prefix("camera-") {
        let graph = match name {
            "world" => scene.world,
            "menu" => scene.menu,
            "menu3d" => scene.menu_3d,
            other => panic!("no camera {other}"),
        };
        return camera_bytes(world, graph.unwrap(), label);
    }
    if let Some(name) = file.strip_prefix("scene-") {
        let (object, vtable) = match name {
            "world" | "menu" | "menu3d" => {
                let graph = match name {
                    "world" => scene.world,
                    "menu" => scene.menu,
                    _ => scene.menu_3d,
                }
                .unwrap();
                let state = world.get::<SceneGraph>(graph).unwrap();
                match label {
                    "unknown_e0" => return state.unknown_e0.to_le_bytes().to_vec(),
                    "menu_mode" => return u8s(state.menu_mode_e8),
                    "fov" => return le_f32(&[state.camera_fov_ec]),
                    _ => (graph, state.original_vtable),
                }
            }
            "sky" => (scene.sky.unwrap(), NI_NODE_VTABLE),
            "weather" => (scene.weather.unwrap(), NI_NODE_VTABLE),
            "lodTrees" => (scene.lod_trees.unwrap(), NI_NODE_VTABLE),
            "objectLOD" => (scene.object_lod_root.unwrap(), NI_NODE_VTABLE),
            "magicProjectile" => (scene.magic_projectile_root.unwrap(), NI_NODE_VTABLE),
            "lodRoot" => {
                let node = scene.lod_root.unwrap();
                let state = world.get::<ClearZNode>(node).unwrap();
                match label {
                    "byte_dc" => return vec![state.byte_dc],
                    "byte_dd" => return vec![state.byte_dd],
                    "float_e0" => return le_f32(&[state.float_e0]),
                    "float_e4" => return le_f32(&[state.float_e4]),
                    _ => (node, state.original_vtable),
                }
            }
            "fog" => {
                let fog = world.get::<FogProperty>(scene.fog.unwrap()).unwrap();
                return match label {
                    "vtable" => FogProperty::ORIGINAL_VTABLE.to_le_bytes().to_vec(),
                    "flags" => fog.flags_18.to_le_bytes().to_vec(),
                    "depth" => le_f32(&[fog.depth_1c]),
                    "color" => le_f32(&fog.color_20),
                    "start" => le_f32(&[fog.start_distance_2c]),
                    "end" => le_f32(&[fog.end_distance_30]),
                    other => panic!("no fog label {other}"),
                };
            }
            "wireframe" => {
                let wireframe = world
                    .get::<WireframeProperty>(scene.wireframe.unwrap())
                    .unwrap();
                return match label {
                    "vtable" => WireframeProperty::ORIGINAL_VTABLE.to_le_bytes().to_vec(),
                    "flags" => wireframe.flags_18.to_le_bytes().to_vec(),
                    other => panic!("no wireframe label {other}"),
                };
            }
            other => panic!("no scene file {other}"),
        };
        return av_object_bytes(world, object, label, vtable)
            .unwrap_or_else(|| panic!("no label {file} {label}"));
    }
    if file == "shadowNode" {
        let node = scene.shadow_scene.unwrap();
        let state = world.get::<ShadowSceneNode>(node).unwrap();
        let null = |present: bool| u32::from(present).to_le_bytes().to_vec();
        return match label {
            "word_e0" => state.word_e0.to_le_bytes().to_vec(),
            "list_e4" => (state.light_list_e4.len() as u32).to_le_bytes().to_vec(),
            "list_f4" => state.light_list_count_f4.to_le_bytes().to_vec(),
            "unknown_108" => state.unknown_108.to_le_bytes().to_vec(),
            "unknown_10c" => state.unknown_10c.to_le_bytes().to_vec(),
            "light_118" => null(state.light_118.is_some()),
            "byte_11c" => vec![state.byte_11c],
            "pointer_120" => state.pointer_120.to_le_bytes().to_vec(),
            "pointer_124" => state.pointer_124.to_le_bytes().to_vec(),
            "unknown_128" => state.unknown_128.to_le_bytes().to_vec(),
            "byte_12c" => vec![state.byte_12c],
            _ => av_object_bytes(world, node, label, state.original_vtable)
                .unwrap_or_else(|| panic!("no label {file} {label}")),
        };
    }
    if file.starts_with("accumulator") {
        let accumulator = window.shader_manager.accumulator.as_ref().unwrap();
        return match label {
            "vtable" => accumulator.original_vtable.to_le_bytes().to_vec(),
            "dword_08" => accumulator.dword_08.to_le_bytes().to_vec(),
            "byte_34" => vec![accumulator.byte_34],
            "byte_35" => vec![accumulator.byte_35],
            "byte_c0" => vec![accumulator.byte_c0],
            "float_c4" => le_f32(&[accumulator.float_c4]),
            "bytes_21e0" => accumulator.bytes_21e0.to_vec(),
            "word_21e4" => accumulator.word_21e4.to_le_bytes().to_vec(),
            "word_21e6" => accumulator.word_21e6.to_le_bytes().to_vec(),
            "dword_2228" => accumulator.dword_2228.to_le_bytes().to_vec(),
            "byte_2268" => vec![accumulator.byte_2268],
            other => panic!("no accumulator label {other}"),
        };
    }
    if let Some(name) = file.strip_prefix("slot") {
        let index: usize = name.strip_suffix("-shader").unwrap().parse().unwrap();
        let slot = window.shader_slots.slots[index].as_ref().unwrap();
        let shader = slot.shader.as_ref().unwrap();
        return match label {
            "vtable" => shader.original_vtable.to_le_bytes().to_vec(),
            "field_08" => shader.field_08.to_le_bytes().to_vec(),
            "field_0c" => shader.field_0c.to_le_bytes().to_vec(),
            "initialized" => vec![shader.initialized],
            other => panic!("no slot label {other}"),
        };
    }
    assert_eq!(file, "globals-B42C00");
    let g = &creation.globals;
    let s = &window.shader_system;
    let context = &window.shader_context;
    match label {
        "local_map" => vec![s.local_map_shader],
        "refraction" => u8s(g.refraction),
        "byte_b42e85" => vec![scene.byte_b42e85],
        "non_pow2" => u8s(g.non_pow2_textures),
        "tex_formats" => g.texture_formats.iter().map(|&set| u8::from(set)).collect(),
        "shaders30" => u8s(g.shaders_30),
        "image_space" => vec![s.image_space_effects],
        "shader_flags_at_7ba279" => flags_before.to_le_bytes().to_vec(),
        "render_path" => g.render_path.to_le_bytes().to_vec(),
        "shader_directory" => [scene.shader_directory.as_bytes(), &[0]].concat(),
        "lod_tree_bias" => le_f32(&[s.lod_tree_mip_bias]),
        "local_tree_bias" => le_f32(&[s.local_tree_mip_bias]),
        "hard_drive_cache" => vec![s.use_hard_drive_cache],
        "hdr" => u8s(g.hdr),
        "fp16" => u8s(g.fp16_filtering),
        "aniso" => u8s(g.anisotropic_min_filter),
        "display_lod" => vec![s.display_lod_land],
        "sun" => le_f32(&[s.sun_brightness]),
        "sky" => le_f32(&[s.sky_brightness]),
        "hdr_block" => {
            let mut block = le_f32(&[
                s.bright_clamp,
                s.bright_clamp_interior,
                s.bright_scale,
                s.bright_scale_interior,
                s.blur_radius_hdr,
                s.blur_radius_hdr_interior,
                s.eye_adapt_speed,
                s.eye_adapt_speed_interior,
                s.emissive_hdr_mult,
                s.emissive_hdr_mult_interior,
                s.upper_lum_clamp,
                s.upper_lum_clamp_interior,
                s.target_lum,
                s.target_lum_interior,
            ]);
            block.extend_from_slice(&s.blur_passes_hdr.to_le_bytes());
            block.extend_from_slice(&s.blur_passes_hdr_interior.to_le_bytes());
            block
        }
        "water_res" => context
            .globals
            .water_resolution_b45fc8
            .to_le_bytes()
            .to_vec(),
        "particle_count" => context.globals.particle_count_b4600c.to_le_bytes().to_vec(),
        "floats" => le_f32(&window.shader_manager.floats_b4616c),
        other => panic!("no global label {other}"),
    }
}

fn shader_row(kind: &str, key: &str) -> String {
    block("SHD|")
        .into_iter()
        .find(|row| row[1] == kind && row[2] == key)
        .unwrap_or_else(|| panic!("no row {kind} {key}"))[3]
        .clone()
}

fn check_shader_rows(window: &SceneWindow) {
    let slots: &BsShaderSlots = &window.shader_slots;
    let context = &window.shader_context;
    // SLOT rows (P05-RUN-001): vtable, entries, streams.
    for row in rows("SHD|", "SLOT") {
        let slot = slots.slots[row[2].parse::<usize>().unwrap()]
            .as_ref()
            .unwrap();
        let shader = slot.shader.as_ref().unwrap();
        let declaration = slot.declaration.as_ref().unwrap();
        assert_eq!(
            format!(
                "{:X},{},{}",
                shader.original_vtable, declaration.entry_count, declaration.stream_count
            ),
            row[3],
            "slot {}",
            row[2]
        );
    }
    // SLOTMAP rows (P05-RUN-002): which slots hold a constant map, which share one, and that +0x70 is set.
    let mut map_names = HashMap::new();
    let mut map_ids = HashMap::new();
    fixtures::ledger_relaxed(
        "phase05",
        "SHD|P05-RUN-002|SLOTMAP|",
        rows("SHD|", "SLOTMAP").len(),
        "pointers into the original's heap: null-ness and which slots share a map are held, never the value",
    );
    for row in rows("SHD|", "SLOTMAP") {
        let values: Vec<&str> = row[3].split(',').collect();
        let shader = slots.slots[row[2].parse::<usize>().unwrap()]
            .as_ref()
            .unwrap()
            .shader
            .as_ref()
            .unwrap();
        for (recorded, port) in [
            (values[0], shader.pixel_constant_map),
            (values[1], shader.vertex_constant_map),
        ] {
            assert_eq!(recorded != "0", port.is_some(), "slot {} map", row[2]);
            if let Some(port) = port {
                let known = map_names.entry(port).or_insert_with(|| recorded.to_owned());
                assert_eq!(*known, recorded, "slot {} map sharing", row[2]);
                assert_eq!(
                    *map_ids.entry(recorded.to_owned()).or_insert(port),
                    port,
                    "slot {} map sharing",
                    row[2]
                );
            }
        }
        assert_eq!(
            values[2] != "0",
            shader.current_pass.is_some(),
            "slot {}",
            row[2]
        );
    }
    let globals = &context.globals;
    for (key, passes) in [
        ("shadowLight", &globals.shadow_light.passes_b455a0),
        ("lighting30", &globals.lighting30.passes_b473d0),
        ("skin", &globals.skin.passes_b47718),
        ("branch", &globals.branch.passes_b47790),
        ("parallax", &globals.parallax.passes_b47620),
    ] {
        assert_eq!(
            format!("{},{}", passes.iter().flatten().count(), passes.len()),
            shader_row("PASSES", key),
            "{key}"
        );
    }
    assert_eq!(
        globals
            .shadow_light
            .passes_b455a0
            .iter()
            .flatten()
            .count()
            .to_string(),
        shader_row("COUNT", "PASSES")
    );
    let vertex = &globals.shadow_light.vertex_b45290;
    assert_eq!(
        vertex[53..129].iter().flatten().count().to_string(),
        shader_row("COUNT", "VS_54_130")
    );
    assert_eq!(
        vertex[31..53].iter().flatten().count().to_string(),
        shader_row("COUNT", "VS_32_54")
    );
    let entries = shader_row("ENTRIES", "B45018,B45038");
    let (first, second) = entries.split_once(',').unwrap();
    assert_eq!(
        first != "0",
        globals.shadow_light.entries[&0x00B4_5018].is_some()
    );
    assert_eq!(
        second != "0",
        globals.shadow_light.entries[&0x00B4_5038].is_some()
    );
    let pass0 = context
        .pools
        .pass(globals.lighting30.passes_b473d0[0].unwrap());
    assert_eq!(
        format!(
            "{},{},{}",
            pass0.stage_end, pass0.stage_count, pass0.textured_stage_count
        ),
        shader_row("L30PASS0", "end,count,textured")
    );
    let pool = |counters: &oblivion_sim::gamebryo::shader_runtime::ShaderPoolCounters| {
        format!(
            "{:x},{:x},{:x},{:x}",
            counters.capacity, counters.free_count, counters.next_batch, counters.initial_batch
        )
    };
    assert_eq!(
        pool(&context.pools.pass_counters),
        shader_row("POOL", "passPool")
    );
    assert_eq!(
        pool(&context.pools.stage_counters),
        shader_row("POOL", "stagePool")
    );
    assert_eq!(
        pool(&context.pools.stage_group_counters),
        shader_row("POOL", "stageGroupPool")
    );
    assert_eq!(
        pool(&context.pools.render_group_counters),
        shader_row("POOL", "stateGroupPool")
    );
    assert_eq!(
        pool(&context.pools.render_entry_counters),
        shader_row("POOL", "stateEntryPool")
    );
    // ISLIST and LIST rows.
    let list = window.image_space_list.as_ref().expect("00B42F3E is set");
    let value = shader_row("ISLIST", "order");
    let (order, copy) = value.split_once(';').unwrap();
    let order: Vec<usize> = order.split(',').map(|slot| slot.parse().unwrap()).collect();
    assert_eq!(
        format!("{:X},{}", list.original_vtable, list.shaders.len()),
        shader_row("LIST", "IMAGE_SPACE")
    );
    assert_eq!(list.shaders.len(), order.len());
    for (shader, slot) in list.shaders.iter().zip(order) {
        assert!(Arc::ptr_eq(
            shader,
            slots.slots[slot].as_ref().unwrap().shader.as_ref().unwrap()
        ));
    }
    assert!(Arc::ptr_eq(
        list.copy_shader.as_ref().unwrap(),
        slots.slots[copy.parse::<usize>().unwrap()]
            .as_ref()
            .unwrap()
            .shader
            .as_ref()
            .unwrap()
    ));
    let accumulator = window.shader_manager.accumulator.as_ref().unwrap();
    assert_eq!(
        format!("{:X}", accumulator.original_vtable),
        shader_row("LIST", "ACCUMULATOR")
    );
    // PKG rows: the package loaded, and every program in it became one trace entry.
    assert_eq!(
        context.programs.device_calls.len().to_string(),
        shader_row("PKG", "ENTRIES")
    );
    assert_eq!(context.programs.warnings, Vec::<String>::new());
}

fn check_target_rows(world: &World, window: &SceneWindow, creation: &RendererCreation) {
    let number = |text: &str| text.parse::<u32>().unwrap();
    let hex = |text: &str| u32::from_str_radix(text, 16).unwrap();
    let manager = window.shader_manager.texture_manager.as_ref().unwrap();
    assert_eq!(rows("TARGETS|", "REL").len(), 13);
    for row in rows("TARGETS|", "REL") {
        let index = number(&row[2]) as usize - 1;
        let rendered = world
            .get::<RenderedTexture>(window.targets[index].unwrap())
            .unwrap();
        assert_eq!(rendered.original_vtable, 0x00A9_1370);
        let object = world
            .get::<RenderedTextureObject>(rendered.texture_20)
            .unwrap();
        let data = object.renderer_data_24.unwrap();
        assert_eq!(object.original_vtable, 0x00A8_0204);
        assert_eq!(object.prefs_18, [2, 2, 2]);
        assert_eq!(
            u32::from(object.format_override_34),
            hex(&row[3]),
            "{row:?}"
        );
        assert_eq!(object.override_format_38 as u32, hex(&row[4]), "{row:?}");
        assert_eq!(u32::from(object.render_target_3c), hex(&row[5]), "{row:?}");
        let (width, height) = rendered_texture_object_size(world, Some(rendered.texture_20));
        assert_eq!(width, number(&row[6]), "{row:?}");
        assert_eq!(height, number(&row[7]), "{row:?}");
        assert_eq!(data.original_vtable, 0x00A8_8464);
        assert_eq!(data.texture_bytes_60, number(&row[8]), "{row:?}");
        assert_eq!(format_dump(&data.pixel_format_0c), row[9], "{row:?}");
        let group = world
            .get::<RenderTargetGroup>(rendered.groups_08[0].unwrap())
            .unwrap();
        assert_eq!(group.buffer_count_18, number(&row[10]), "{row:?}");
        assert_eq!(group.depth_stencil_1c.is_some(), row[11] == "1", "{row:?}");
    }
    assert!(manager.texture_data_10.is_empty());
    for row in rows("TARGETS|", "FREE") {
        let record = manager.texture_data_00[number(&row[2]) as usize];
        assert_eq!(record.format_04 as u32, number(&row[3]), "{row:?}");
        assert_eq!(record.flags_0c, hex(&row[4]), "{row:?}");
        assert_eq!(record.usage_08, 0);
        assert_eq!(record.in_use_10, 1);
    }
    for row in rows("TARGETS|", "GET") {
        let index = number(&row[2]) as usize - 1;
        let record = manager.texture_data_00[index];
        assert_eq!(record.flags_0c, hex(&row[5]), "{row:?}");
        assert_eq!(record.format_04 as u32, number(&row[6]), "{row:?}");
        assert_eq!(record.texture_00, window.targets[index]);
    }
    // The RT_DONE globals, read from the port's own chain here: the render-target test in oblivion_sim sets these inputs by hand.
    let global = |name: &str| {
        let rows = rows("TARGETS|", "GLOBAL");
        let row = rows.iter().find(|row| row[2] == name).unwrap();
        hex_bytes(&row[3])
    };
    let g = &creation.globals;
    let s = &window.shader_system;
    let u8s = |value: bool| vec![u8::from(value)];
    let targets = &window.render_targets;
    let totals = [
        targets.texture_bytes_b42064.to_le_bytes(),
        targets.texture_padding_b42068.to_le_bytes(),
    ]
    .concat();
    let cubes = [
        targets.cube_bytes_b42860.to_le_bytes(),
        targets.cube_padding_b42864.to_le_bytes(),
    ]
    .concat();
    let buffers = [
        window.shader_manager.memory.buffer_bytes.to_le_bytes(),
        window.shader_manager.memory.buffer_padding.to_le_bytes(),
    ]
    .concat();
    let formats: Vec<u8> = g.texture_formats.iter().map(|&set| u8::from(set)).collect();
    for (name, port) in [
        ("hdr@B43070", u8s(g.hdr)),
        ("fp16Filtering@B43071", u8s(g.fp16_filtering)),
        ("anisoMin@B43072", u8s(g.anisotropic_min_filter)),
        ("nonPow2@B42E96", u8s(g.non_pow2_textures)),
        ("multisample@B34FC0", g.multisample.to_le_bytes().to_vec()),
        ("texFormats@B42E98", formats),
        ("imageSpace@B42F3E", vec![s.image_space_effects]),
        (
            "fmtOverride@B3FF00",
            vec![window.shader_context.globals.water_b3ff00],
        ),
        (
            "overrideFormat@B2752C",
            targets.override_format_b2752c.to_le_bytes().to_vec(),
        ),
        ("renderTarget@B27530", vec![targets.render_target_b27530]),
        (
            "depthDiscard@B294EC",
            window
                .shader_manager
                .depth_discard_flag
                .to_le_bytes()
                .to_vec(),
        ),
        ("rtBytes@B42064", totals),
        ("cubeBytes@B42860", cubes),
        ("bufferBytes@B3FAB8", buffers),
        (
            "waterRes@B45FC8",
            window
                .shader_context
                .globals
                .water_resolution_b45fc8
                .to_le_bytes()
                .to_vec(),
        ),
    ] {
        assert_eq!(port, global(name), "{name}");
    }
    let depth_rows = rows("TARGETS|", "DEPTH");
    let depth = world
        .get::<DepthStencilBuffer>(manager.depth_stencil_40.unwrap())
        .unwrap();
    assert_eq!(depth.original_vtable, hex(&depth_rows[0][2]));
    assert_eq!(depth.width_08, hex(&depth_rows[0][3]));
    assert_eq!(depth.height_0c, hex(&depth_rows[0][4]));
}

fn check_effect_rows(world: &World, window: &SceneWindow) {
    let textures = &window.effect_textures;
    // TEX rows (P05-RUN-002), all 15 with renderer data, with the five file-backed slots through the D3DX stand-ins.
    let tex = rows("TARGETS|", "TEX");
    assert_eq!(tex.len(), 15);
    for row in &tex {
        let texture = world
            .get::<SourceTexture>(
                slot_of(textures, &row[2]).unwrap_or_else(|| panic!("{row:?} is null")),
            )
            .unwrap();
        let data = texture.renderer_24.as_ref().expect("renderer data");
        assert_eq!(format!("{:X}", data.original_vtable), row[3], "{row:?}");
        assert_eq!(data.width_54.to_string(), row[4], "{row:?}");
        assert_eq!(data.height_58.to_string(), row[5], "{row:?}");
        assert_eq!(data.levels_5c.to_string(), row[6], "{row:?}");
        assert_eq!(data.texture_bytes_60.to_string(), row[7], "{row:?}");
    }
    // RDATA rows (P05-RUN-001): the object's own bytes too.
    let rdata = rows("SCENE|", "RDATA");
    assert_eq!(rdata.len(), 15);
    for row in &rdata {
        let texture = world
            .get::<SourceTexture>(slot_of(textures, &row[2]).unwrap())
            .unwrap();
        assert_eq!(format!("{:X}", texture.original_vtable), row[3], "{row:?}");
        let prefs = [
            texture.pixel_layout_18,
            texture.alpha_format_1c,
            texture.mip_mapped_20,
        ];
        let dump: Vec<u8> = prefs.iter().flat_map(|value| value.to_le_bytes()).collect();
        assert_eq!(dump, hex_bytes(&row[4]), "{row:?} prefs");
        assert_eq!(
            texture.app_pixel_data_3c.is_some(),
            row[5] == "1",
            "{row:?} +0x3C"
        );
        assert_eq!(
            u8::from(texture.static_40),
            row[6].parse::<u8>().unwrap(),
            "{row:?} +0x40"
        );
        assert_eq!(
            u8::from(texture.direct_to_renderer_41),
            row[7].parse::<u8>().unwrap(),
            "{row:?} +0x41"
        );
        let data = texture.renderer_24.as_ref().unwrap();
        assert_eq!(data.width_54.to_string(), row[8], "{row:?}");
        assert_eq!(data.height_58.to_string(), row[9], "{row:?}");
        assert_eq!(data.levels_5c.to_string(), row[10], "{row:?}");
        assert_eq!(format!("{:X}", data.original_vtable), row[11], "{row:?}");
        assert_eq!(format_dump(&data.pixel_format_0c), row[12], "{row:?} +0x0C");
        let name = texture
            .platform_filename_38
            .as_deref()
            .map_or("-".to_owned(), path_text);
        assert_eq!(name, row[13], "{row:?} name");
    }
    // NAME rows (P05-RUN-001).
    for row in rows("TARGETS|", "NAME") {
        let texture = world
            .get::<SourceTexture>(slot_of(textures, &row[2]).unwrap())
            .unwrap();
        assert_eq!(
            texture.platform_filename_38.as_deref(),
            Some(row[3].as_bytes())
        );
    }
    // CUBE rows (P05-RUN-002): the merged pixel data of the three cubes.
    for row in rows("TARGETS|", "CUBE") {
        let cube = world
            .get::<SourceTexture>(slot_of(textures, &row[2]).unwrap())
            .unwrap();
        let pixels = world
            .get::<PixelData>(cube.app_pixel_data_3c.expect("the cube keeps +0x3C"))
            .unwrap();
        assert_eq!(pixels.levels_60.to_string(), row[3], "{row:?}");
        assert_eq!(pixels.faces_6c.to_string(), row[4], "{row:?}");
        assert_eq!(pixels.level(0, 0).len().to_string(), row[5], "{row:?}");
        assert_eq!(pixels.widths_54[0].to_string(), row[6], "{row:?}");
        assert_eq!(format_dump(&pixels.format_08), row[7], "{row:?}");
        for (face, wanted) in row[8].split(',').enumerate() {
            let mut hash = 0x811C_9DC5u32;
            for &byte in pixels.level(face as u32, 0) {
                hash ^= u32::from(byte);
                hash = hash.wrapping_mul(0x0100_0193);
            }
            assert_eq!(format!("{hash:08X}"), wanted, "{} face {face}", row[2]);
        }
    }
    // POLYGON row (P05-RUN-001).
    let row = &rows("SCENE|", "POLYGON")[0];
    let polygon = world
        .get::<ScreenPolygon>(textures.screen_polygon.unwrap())
        .unwrap();
    assert_eq!(format!("{:X}", polygon.original_vtable), row[3]);
    assert_eq!(polygon.vertex_count_0c.to_string(), row[4]);
    assert_eq!(polygon.texture_coords_14.is_empty(), row[5] == "0");
    assert_eq!(polygon.colors_18.is_empty(), row[6] == "0");
}

/// The P05-RUN-002 global snapshot's pointers: which are null, and which two slots share one object.
fn check_pointer_rows(window: &SceneWindow) {
    let mut seen: HashMap<String, Option<Entity>> = HashMap::new();
    for row in rows("SCENE|", "PTR") {
        fixtures::ledger_relaxed(
            "phase05",
            &format!("SCENE|{}|PTR|{}|{} #5", row[0], row[2], row[3]),
            1,
            "a pointer into the original's heap: null-ness and which slots share an object are held, never the value",
        );
        let recorded_null = row[4] == "00000000";
        let port = match row[3].as_str() {
            // Live at the snapshot, which is 007BA279: `SceneWindow` is after 007BA271, so this one is the package row below.
            "B430B8" => {
                assert!(!recorded_null);
                assert!(
                    window.shader_manager.package.is_none(),
                    "007BA271 released it"
                );
                continue;
            }
            "B42F50" => {
                assert_eq!(
                    !recorded_null,
                    window.shader_manager.texture_manager.is_some()
                );
                continue;
            }
            "B430FC" => {
                assert_eq!(!recorded_null, window.shader_manager.accumulator.is_some());
                continue;
            }
            "B43104" => {
                assert_eq!(!recorded_null, window.scene_graphs.renderer_reference);
                continue;
            }
            slot => slot_of(&window.effect_textures, slot),
        };
        assert_eq!(!recorded_null, port.is_some(), "{row:?}");
        if !recorded_null {
            let known = seen.entry(row[4].clone()).or_insert(port);
            assert_eq!(*known, port, "{row:?} sharing");
        }
    }
    assert_eq!(
        window.effect_textures.radial, window.effect_textures.radial_alias,
        "00B430D4 is 00B43110"
    );
}

/// The whole window, on one `World`.
#[test]
fn phase05_window_matches_the_runs() {
    // Before the window: what the earlier phases leave.
    let mut world = World::new();
    let mut known = StartupCallbackScheduler::default();
    register_known_crt_callbacks(&mut known).unwrap();
    known.run(&mut world).unwrap();
    let collections = pinned_collections();
    let mut creation = pinned_renderer(&collections);
    if creation.device_image_converter_default {
        set_device_image_converter_default(&mut world);
    }
    let flags_before = creation.globals.shader_flags;

    let data = fixtures::data_dir();
    assert!(
        data.join("Shaders/shaderpackage013.sdp").exists(),
        "this test needs the game data: set OBLIVION_DATA_DIR ({data:?})"
    );
    let root = data.parent().unwrap().canonicalize().unwrap();
    let scratch =
        std::env::temp_dir().join(format!("recomp_phase05_window_{}", std::process::id()));
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
    let archives = RefCell::new(manager);
    let palette = RefCell::new(BsTexturePalette::new(1009));

    let mut platform = ScriptedOsPlatform::default();
    let input = OsInputGlobals::new(HostInstance(1), &collections, &mut platform).unwrap();
    platform.trace.clear();
    let mut runtime = NoSceneControllers;

    let window = run_scene_window(
        &mut world,
        SceneWindowServices {
            ini: &collections.ini,
            creation: &mut creation,
            load_direct_hint: true,
            drop_static_app_data: true,
            texture_palette: true,
            file_finder: true,
            open_texture_file: Box::new(|path| {
                bs_file_open_core(
                    &finder,
                    Some(&mut archives.borrow_mut()),
                    &path_text(path),
                    0,
                    0x8000,
                    0xFFFF,
                )
                .map(boxed)
            }),
            texture_file_exists: Box::new(|path| {
                bs_file_exists(
                    &callbacks,
                    &root,
                    Some(&finder),
                    Some(&mut archives.borrow_mut()),
                    &path_text(path),
                    0,
                )
            }),
            palette_find: Some(Box::new(|path| {
                palette
                    .borrow()
                    .find(path, Some(&mut archives.borrow_mut()))
            })),
            palette_add: Some(Box::new(|path, texture| {
                palette
                    .borrow_mut()
                    .add(path, texture, Some(&mut archives.borrow_mut()));
            })),
            open_package: Box::new(|path| {
                bs_file_open_as_bs_file(
                    Some(&finder),
                    Some(&mut archives.borrow_mut()),
                    path,
                    0,
                    0,
                    0xFFFF,
                )
                .map(boxed)
            }),
            open_program: Some(Box::new(|_| None)),
            program_file_time: Some(Box::new(|_| None)),
            shader_cache_directory: String::new(),
            os_input: Some(&input),
            os_platform: &mut platform,
            runtime: &mut runtime,
        },
    )
    .unwrap();

    // Original order: the ORDER rows are P05-RUN-001's probe hits, and each row's sequence is later than the one before.
    fixtures::ledger_relaxed(
        "phase05",
        "SCENE|P05-RUN-001|ORDER|",
        rows("SCENE|", "ORDER").len(),
        "the probe's own record numbers, not a value of the game: only their order means anything, and `window.order` holds the port's",
    );
    for (what, reason) in [
        (
            "SCENE|P05-RUN-001|SNAP #5",
            "an offset into the dump's label, before the bytes the row holds",
        ),
        (
            "SCENE|P05-RUN-002|SNAP #5",
            "an offset into the dump's label, before the bytes the row holds",
        ),
        (
            "SCENE|P05-RUN-002|PTR #4",
            "the address the row is named by; its slot is read through the name of the run's dump",
        ),
    ] {
        fixtures::ledger_relaxed("phase05", what, 1, reason);
    }
    let sequence: Vec<u32> = rows("SCENE|", "ORDER")
        .iter()
        .map(|row| row[3].parse().unwrap())
        .collect();
    assert!(sequence.windows(2).all(|pair| pair[0] < pair[1]));
    let mut expected = vec![0x007B_A0F0, 0x007A_B1D0, 0x007B_8990];
    expected.extend([0x007B_4290; 27]);
    expected.extend([0x0080_3A30, 0x007C_2450, 0x0048_14C0, 0x0040_3160]);
    assert_eq!(window.order, expected);
    assert!(window.scene_result.pending_steps.is_empty());
    assert!(window.manager_result.pending_steps.is_empty());
    assert!(window.manager_result.ran);
    assert!(window.warnings.is_empty(), "{:?}", window.warnings);

    // Every SNAP row, by file and label.
    for row in rows("SCENE|", "SNAP") {
        let port = snap_bytes(&world, &window, &creation, flags_before, &row[2], &row[3]);
        assert_eq!(port, hex_bytes(&row[5]), "{row:?}");
    }
    // SCENE_RET: the store at 00406B66. The snapshot at 007BA279 is before it, so that file has no row for it.
    assert_eq!(window.scene_graphs.reference_classifier, 0x004E_0CF0);
    let shader = &window.shader_system;
    for row in rows("SCENE|", "RET") {
        let scene = &window.scene_graphs;
        let port = match row[2].as_str() {
            "shaderFlags@B42F40" => creation.globals.shader_flags.to_le_bytes().to_vec(),
            "B2C678@B2C678" => scene.shadow_flags.to_le_bytes().to_vec(),
            "warningCb@B42E8C" => shader.warning_callback.to_le_bytes().to_vec(),
            "timeCb@B42D78" => shader.game_time_callback.to_le_bytes().to_vec(),
            "compileFlag_B2DD4C@B2DD4C" => {
                vec![u8::from(window.shader_context.programs.compile_fallback)]
            }
            "B2C2BC@B2C2BC" => le_f32(&[shader.specular_multiplier, shader.emissive_multiplier]),
            "B42EA7" => vec![scene.byte_b42ea7],
            "B42E85" => vec![scene.byte_b42e85],
            "B44EE4" => scene.float_b44ee4.to_le_bytes().to_vec(),
            "B33396" => vec![scene.static_menu_background],
            other => panic!("no RET {other}"),
        };
        assert_eq!(port, hex_bytes(&row[3]), "{row:?}");
    }
    assert_eq!(window.scene_graphs.shader_directory, "data\\shaders\\");

    check_pointer_rows(&window);
    check_effect_rows(&world, &window);
    check_target_rows(&world, &window, &creation);
    check_shader_rows(&window);

    // The SHD WINDOW rows and the HDR arm's copies.
    let s = &window.shader_system;
    assert_eq!(
        vec![s.image_space_effects],
        hex_bytes(&shader_row("WINDOW", "imageSpaceCopy"))
    );
    assert_eq!(
        s.blend_type.to_le_bytes().to_vec(),
        hex_bytes(&shader_row("WINDOW", "B2C1E4"))
    );
    assert_eq!(
        [
            s.sun_brightness.to_le_bytes(),
            s.sky_brightness.to_le_bytes()
        ]
        .concat(),
        hex_bytes(&shader_row("WINDOW", "B43154"))
    );

    // 0040EAB1..0040EAC1: P05-RUN-001's result, ignored by the original.
    assert!(window.input_flush.called && window.input_flush.device);
    assert_eq!(
        format!("{:08X}", window.input_flush.result),
        shader_row("FLUSH", "RESULT")
    );

    // 004814C0 stopped at the null ModelLoader (P05-RUN-001: modelLoader@B33A1C=0).
    assert!(matches!(
        window.test_sphere,
        Some(oblivion_sim::bs::shader_manager::TestSphereStatus::NoModelLoader)
    ));
    assert!(
        window.scene_graphs.fallback_quad.is_none(),
        "P05-RUN-002: B333EC is null"
    );

    // Not from a run: the C++ port's trace of what the device saw. Five D3DX loads in slot order, each info before creation.
    let d3dx: Vec<&DeviceTraceEntry> = creation
        .device_calls
        .iter()
        .filter(|entry| {
            matches!(
                entry.call,
                DeviceCall::D3dxGetImageInfoFromFileInMemory { .. }
                    | DeviceCall::D3dxCreateTextureFromFileInMemory { .. }
            )
        })
        .collect();
    assert_eq!(d3dx.len(), 10, "{d3dx:?}");
    for pair in d3dx.chunks(2) {
        let DeviceCall::D3dxGetImageInfoFromFileInMemory { size: info_size } = pair[0].call else {
            panic!("{pair:?}");
        };
        assert_eq!(pair[0].result, D3D_OK);
        assert_eq!(pair[1].site, 0x0076_10D3);
        assert_eq!(pair[1].result, D3D_OK);
        assert_eq!(
            pair[1].call,
            DeviceCall::D3dxCreateTextureFromFileInMemory {
                resource_type: D3DRTYPE_TEXTURE,
                size: info_size
            }
        );
    }
    let targets = creation
        .device_calls
        .iter()
        .filter(|entry| matches!(entry.call, DeviceCall::CreateTexture { pool, .. } if pool == D3DPOOL_DEFAULT))
        .count();
    assert_eq!(targets, 13);

    // Not from a run: the pass tables' stages hold the effect entities 0076C910 read (the runs recorded only counts).
    let context = &window.shader_context;
    assert!(context.effect_texture_requests.contains(&0x00B4_3110));
    let mut stages = 0;
    for pass in context.globals.shadow_light.passes_b455a0.iter().flatten() {
        for stage in context.pools.pass(*pass).stages.iter().flatten() {
            let stage = context.pools.stage(*stage);
            if stage.texture_global != 0 {
                assert_eq!(
                    stage.texture,
                    window.effect_textures.stage_texture(stage.texture_global)
                );
                stages += 1;
            }
        }
    }
    assert!(stages > 0);

    // DebugShader's two boundaries ran for real on slot 9: a NiScreenElements entity with a default property state.
    assert!(window.shader_context.counted_boundaries.is_empty());
    let slot = window.shader_slots.slots[9].as_ref().unwrap();
    let debug = slot
        .shader
        .as_ref()
        .unwrap()
        .as_any()
        .downcast_ref::<DebugShader>()
        .expect("slot 9 is the DebugShader");
    let quad = debug.screen_elements_entity.expect("the +0xBC entity");
    assert!(world.get::<ScreenElementsQuad>(quad).is_some());
    assert!(
        world
            .get::<NiGeometry>(quad)
            .unwrap()
            .property_state_ac
            .is_some()
    );
}
