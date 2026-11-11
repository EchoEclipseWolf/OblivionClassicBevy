//! IDA: N/A a host tool: the Imperial Prison's models, loaded through the port's model drain (0043B420, `ModelDrain`) as the
//! cell entry loads them, then drawn as Bevy meshes in a grid, a page at a time, with a PNG screenshot of each page.
//!
//! Usage: `cargo run -p oblivion_host --example prison_gallery -- PATHS_FILE OUT_DIR` (PATHS_FILE: lines of "count path").
//! Each model is scaled to fit its cell and turned toward the camera; collision, EditorMarker nodes and app-culled
//! geometry are left out, and particle systems aren't drawn.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::WindowResolution;
use oblivion_core::crt::MsvcRand;
use oblivion_sim::bs::archive::{ArchiveManager, ArchiveSettings};
use oblivion_sim::bs::file_finder::{
    BsFileCallbacks, FileFinder, bs_file_exists, bs_file_open_core,
};
use oblivion_sim::bs::model_loader::{
    ModelBuildServices, ModelDrain, ModelRequest, load_model, release_task,
};
use oblivion_sim::bs::shader::BsShaderSlot;
use oblivion_sim::bs::shader_classes_geometry::SkyShader;
use oblivion_sim::bs::sky_shader::PrepareGeometryContext;
use oblivion_sim::bs::stream::BsStreamServices;
use oblivion_sim::bs::task_manager::TaskGlobals;
use oblivion_sim::gamebryo::av_object::NiAvProperties;
use oblivion_sim::gamebryo::mesh_geometry::{MeshTriShapeData, MeshTriShapePayload, NiGeometry};
use oblivion_sim::gamebryo::mesh_tri_strips::MeshTriStripsData;
use oblivion_sim::gamebryo::object_net::NiObjectNet;
use oblivion_sim::gamebryo::registration::{
    NiFactoryRegistry, NiProcessDefaults, StartupCallbackScheduler, register_known_crt_callbacks,
};
use oblivion_sim::gamebryo::scene_node::{NiNodeChildren, SceneNodePayload};
use oblivion_sim::gamebryo::search_path::bs_search_path_candidates;
use oblivion_sim::gamebryo::stream_objects::NiClass;
use oblivion_sim::gamebryo::stream_postload::StreamPostLoadRuntime;
use oblivion_sim::gamebryo::texture::{
    SourceTexture, SourceTextureLoadServices, TexturingProperty,
};
use oblivion_sim::gamebryo::transform::{NiTransform, compose_scene_transform};

/// IDA: 0x00A90634, the ShadowLight family's vtable, on a stub shader as `stream_graph_real` uses it.
const SHADOW_LIGHT_VTABLE: u32 = 0x00A9_0634;
const COLUMNS: usize = 8;
const ROWS: usize = 5;
const CELL: f32 = 1.25;

/// IDA: N/A one drawable piece, baked into the model's space (Bevy axes: Gamebryo's Z up is Y).
struct Piece {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
    texture: Option<String>,
    /// NiAlphaProperty +0x18 bit 0 (blend) and bit 9 (test) with its reference, from the nearest property list.
    alpha: Option<(u16, u8)>,
}

/// IDA: N/A one model of the gallery.
struct GalleryModel {
    name: String,
    pieces: Vec<Piece>,
}

/// IDA: N/A the source-texture services with no palette, as `stream_graph_real` has them.
struct NoPalette<'a> {
    root: &'a Path,
    callbacks: &'a BsFileCallbacks,
    finder: &'a FileFinder,
    archives: &'a mut ArchiveManager,
}

impl SourceTextureLoadServices for NoPalette<'_> {
    /// IDA: N/A `NoPalette`'s side of the services.
    fn search_candidates(&mut self, file_path: &[u8]) -> Vec<Vec<u8>> {
        bs_search_path_candidates(&String::from_utf8_lossy(file_path))
            .into_iter()
            .map(String::into_bytes)
            .collect()
    }
    /// IDA: N/A `NoPalette`'s side of the services.
    fn find_texture(&mut self, _path: &[u8]) -> Option<Entity> {
        None
    }
    /// IDA: N/A `NoPalette`'s side of the services.
    fn register_texture(&mut self, _path: &[u8], _texture: Entity) {}
    /// IDA: N/A `NoPalette`'s side of the services.
    fn file_exists(&mut self, path: &[u8]) -> bool {
        let text: String = path.iter().map(|&byte| char::from(byte)).collect();
        bs_file_exists(
            self.callbacks,
            self.root,
            Some(self.finder),
            Some(&mut *self.archives),
            &text,
            0,
        )
    }
}

// IDA: N/A an archive manager over the default archive list.
fn archive_manager(root: &Path, finder: &FileFinder, tag: &str) -> ArchiveManager {
    let settings = ArchiveSettings {
        invalidate_older_files: false,
        retain_directory_strings: 2,
        ..ArchiveSettings::default()
    };
    let scratch = std::env::temp_dir().join(format!("prison_gallery_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join("Plugins.txt"), "Oblivion.esm\n").unwrap();
    let mut archives = ArchiveManager::new(root, settings);
    archives.initialize_from_settings(finder, &scratch);
    let _ = std::fs::remove_dir_all(&scratch);
    archives
}

// IDA: N/A Gamebryo's (x, y, z), z up, as Bevy's (x, z, -y).
fn to_bevy(v: [f32; 3]) -> [f32; 3] {
    [v[0], v[2], -v[1]]
}

// IDA: N/A NiTransform applied to a point: rotate (row-major) times scale, plus translate.
fn apply(t: &NiTransform, v: [f32; 3]) -> [f32; 3] {
    let r = &t.rotation;
    let s = t.scale;
    [
        (r[0] * v[0] + r[1] * v[1] + r[2] * v[2]) * s + t.translation[0],
        (r[3] * v[0] + r[4] * v[1] + r[5] * v[2]) * s + t.translation[1],
        (r[6] * v[0] + r[7] * v[1] + r[8] * v[2]) * s + t.translation[2],
    ]
}

// IDA: N/A the rotation alone, for normals.
fn rotate(t: &NiTransform, v: [f32; 3]) -> [f32; 3] {
    let r = &t.rotation;
    [
        r[0] * v[0] + r[1] * v[1] + r[2] * v[2],
        r[3] * v[0] + r[4] * v[1] + r[5] * v[2],
        r[6] * v[0] + r[7] * v[1] + r[8] * v[2],
    ]
}

// IDA: N/A the base map's file name of a NiTexturingProperty in a property list.
fn base_texture(world: &World, properties: &[Option<Entity>]) -> Option<Option<String>> {
    for property in properties.iter().flatten() {
        if let Some(texturing) = world.get::<TexturingProperty>(*property) {
            let name = texturing
                .maps_1c
                .first()
                .copied()
                .flatten()
                .and_then(|map| map.texture_08)
                .and_then(|texture| world.get::<SourceTexture>(texture))
                .and_then(|texture| texture.filename_34.clone())
                .map(|name| {
                    String::from_utf8_lossy(&name)
                        .trim_end_matches('\0')
                        .to_string()
                });
            return Some(name);
        }
    }
    None
}

// IDA: N/A the walk: each node's local transform composed down, EditorMarker subtrees and culled objects skipped, the
// texturing property inherited from the nearest ancestor that has one.
fn walk(
    world: &World,
    object: Entity,
    parent: &NiTransform,
    inherited: Option<String>,
    inherited_alpha: Option<(u16, u8)>,
    pieces: &mut Vec<Piece>,
) {
    let name = world
        .get::<NiObjectNet>(object)
        .and_then(|net| net.name.clone())
        .map(|name| String::from_utf8_lossy(&name).to_string())
        .unwrap_or_default();
    if name.starts_with("EditorMarker") {
        return;
    }
    let properties = world
        .get::<NiAvProperties>(object)
        .map(|list| list.0.clone())
        .unwrap_or_default();
    let texture = base_texture(world, &properties).unwrap_or(inherited);
    let alpha = properties
        .iter()
        .flatten()
        .find_map(|&property| {
            world.get::<oblivion_sim::gamebryo::object_net::AlphaProperty>(property)
        })
        .map(|alpha| (alpha.flags_18, alpha.alpha_test_ref_1a))
        .or(inherited_alpha);
    match world.get::<NiClass>(object) {
        Some(NiClass::Node | NiClass::BillboardNode) => {
            let payload = world.get::<SceneNodePayload>(object).unwrap();
            if payload.effective_av_flags & 1 != 0 {
                return;
            }
            let transform = compose_scene_transform(parent, &payload.local_transform);
            let children = world
                .get::<NiNodeChildren>(object)
                .map(|children| children.slots_ac.clone())
                .unwrap_or_default();
            for child in children.into_iter().flatten() {
                walk(world, child, &transform, texture.clone(), alpha, pieces);
            }
        }
        Some(NiClass::TriStrips | NiClass::TriShape) => {
            let payload = world.get::<MeshTriShapePayload>(object).unwrap();
            if payload.effective_av_flags & 1 != 0 {
                return;
            }
            let transform = compose_scene_transform(parent, &payload.local_transform);
            let Some(data) = world
                .get::<NiGeometry>(object)
                .and_then(|geometry| geometry.data_b4)
            else {
                return;
            };
            let (base, indices) = if let Some(strips) = world.get::<MeshTriStripsData>(data) {
                let mut indices = Vec::new();
                let mut start = 0usize;
                for &length in &strips.strip_lengths_48 {
                    let strip = &strips.strip_points_4c[start..start + usize::from(length)];
                    for i in 0..strip.len().saturating_sub(2) {
                        let (a, b, c) = (strip[i], strip[i + 1], strip[i + 2]);
                        if a == b || b == c || a == c {
                            continue;
                        }
                        if i % 2 == 0 {
                            indices.extend([a, b, c].map(u32::from));
                        } else {
                            indices.extend([a, c, b].map(u32::from));
                        }
                    }
                    start += usize::from(length);
                }
                (&strips.base, indices)
            } else if let Some(shape) = world.get::<MeshTriShapeData>(data) {
                (
                    &shape.base,
                    shape
                        .triangle_list_48
                        .iter()
                        .map(|&i| u32::from(i))
                        .collect(),
                )
            } else {
                return;
            };
            let count = base.positions_1c.len();
            if count == 0 || indices.is_empty() {
                return;
            }
            let positions = base
                .positions_1c
                .iter()
                .map(|&p| to_bevy(apply(&transform, p)))
                .collect();
            let normals = if base.normals_20.len() == count {
                base.normals_20
                    .iter()
                    .map(|&n| to_bevy(rotate(&transform, n)))
                    .collect()
            } else {
                vec![[0.0, 1.0, 0.0]; count]
            };
            let uvs = if base.texture_coordinates_28.len() >= count {
                base.texture_coordinates_28[..count].to_vec()
            } else {
                vec![[0.0, 0.0]; count]
            };
            let colors = if base.colors_24.len() == count {
                base.colors_24.clone()
            } else {
                vec![[1.0; 4]; count]
            };
            pieces.push(Piece {
                positions,
                normals,
                uvs,
                colors,
                indices,
                texture,
                alpha,
            });
        }
        _ => {}
    }
}

/// IDA: N/A loads each model through 0043B420 and the drain, then bakes its pieces. Returns the models and the failures.
fn load_gallery(paths: &[String]) -> (Vec<GalleryModel>, Vec<String>, HashMap<String, Vec<u8>>) {
    let data = oblivion_core::fixtures::data_dir();
    let mut registry = NiFactoryRegistry::default();
    registry.register_known_crt_factories();
    registry.register_winmain_bs_factories();
    for (name, constructor) in oblivion_sim::bs::havok::BHK_FACTORIES {
        registry.insert_or_assign(name, constructor);
    }
    let root = data.parent().unwrap().canonicalize().unwrap();
    let mut file_callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new(&root, &mut file_callbacks);
    finder.add("Data");
    let mut archives = archive_manager(&root, &finder, "meshes");
    let mut texture_archives = archive_manager(&root, &finder, "textures");
    let mut read_archives = archive_manager(&root, &finder, "read");

    let mut world = World::new();
    let mut known = StartupCallbackScheduler::default();
    register_known_crt_callbacks(&mut known).unwrap();
    known.run(&mut world).unwrap();
    let defaults = world
        .resource::<NiProcessDefaults>()
        .scene_property_defaults();
    let mut tasks = TaskGlobals::default();
    tasks.construct_io_manager().unwrap();
    tasks.construct_model_loader().unwrap();
    for path in paths {
        let out = load_model(
            &mut tasks,
            Some(&mut archives),
            path.as_bytes(),
            ModelRequest {
                priority: 5,
                parent: None,
                lod: 0,
                a7: false,
                a8: true,
                a9: true,
            },
        )
        .unwrap();
        release_task(&mut tasks, out).unwrap();
    }
    world.insert_resource(tasks);

    let mut open = |path: &str, mode: i32, size: u32| {
        bs_file_open_core(&finder, None, path, mode, size, 0xFFFF)
            .map(oblivion_sim::bs::file::OpenedFile::into_game_file)
    };
    let mut rand = MsvcRand::default();
    let mut draw = || rand.rand();
    let shadow = Arc::new({
        let mut shader = SkyShader::default();
        shader.base.original_vtable = SHADOW_LIGHT_VTABLE;
        BsShaderSlot {
            declaration: None,
            shader: Some(Arc::new(shader)),
        }
    });
    let mut get_shader = |_: u32| Some(Arc::clone(&shadow));
    let mut load_normal = |_: &mut World, _: &[u8]| false;
    let mut setup = |_: u32, _: Entity| true;
    let mut disabled = 0_u8;
    let mut warning = |_: Vec<u8>, _: bool| {};
    let mut textures = NoPalette {
        root: &root,
        callbacks: &file_callbacks,
        finder: &finder,
        archives: &mut texture_archives,
    };
    let mut services = ModelBuildServices {
        archives: Some(&mut archives),
        stream: BsStreamServices {
            open: &mut open,
            registry: &registry,
            draw_rand: Some(&mut draw),
            draw_on_io_thread: false,
            source_textures: Some(&mut textures),
            post_load: StreamPostLoadRuntime::default(),
        },
        prepare: PrepareGeometryContext {
            render_path: 5,
            hdr: true,
            reference_classifier: 0,
            property_defaults: defaults,
            get_shader: &mut get_shader,
            warnings: Vec::new(),
            sky_shader_trace: Vec::new(),
            water_shader_trace: Vec::new(),
            lava_b47844: 0,
            shaders_30_b42ea5: false,
            load_normal_map: Some(&mut load_normal),
            unported_shader_setup: Some(&mut setup),
            classify_trace: Vec::new(),
        },
        disable_warning_messages_b06b18: &mut disabled,
        show_editor_markers_b34390: 0,
        warning: &mut warning,
        main_random_next: None,
    };
    let mut drain = ModelDrain::new();
    let _ = drain.run_to_end(&mut world, &mut services);
    let failures: Vec<String> = drain
        .record
        .tasks
        .iter()
        .filter_map(|task| {
            task.load_failure.as_ref().map(|failure| {
                format!(
                    "{}: {}",
                    String::from_utf8_lossy(&task.path),
                    failure.chars().take(160).collect::<String>()
                )
            })
        })
        .collect();
    drop(services);

    let tasks = world.resource::<TaskGlobals>();
    let loader = tasks.model_loader_b33a1c.as_ref().unwrap();
    let mut roots: BTreeMap<Vec<u8>, Entity> = BTreeMap::new();
    for entry in loader.models.buckets.iter().flatten() {
        if let Some(root) = loader.models.store[entry.model.0].root_08 {
            roots.insert(entry.key.clone(), root);
        }
    }
    let mut models = Vec::new();
    let mut texture_bytes: HashMap<String, Vec<u8>> = HashMap::new();
    for path in paths {
        let key: Vec<u8> = path.bytes().map(|byte| byte.to_ascii_lowercase()).collect();
        let Some(&root) = roots
            .iter()
            .find(|(name, _)| name.ends_with(&key) || key.ends_with(name))
            .map(|(_, root)| root)
        else {
            continue;
        };
        let mut pieces = Vec::new();
        walk(
            &world,
            root,
            &NiTransform::default(),
            None,
            None,
            &mut pieces,
        );
        for piece in &pieces {
            if let Some(name) = &piece.texture
                && !texture_bytes.contains_key(name)
            {
                let lower = name.to_ascii_lowercase();
                let full = if lower.starts_with("textures\\") {
                    name.clone()
                } else {
                    format!("textures\\{name}")
                };
                if let Some(mut file) =
                    bs_file_open_core(&finder, Some(&mut read_archives), &full, 0, 0x2800, 0xFFFF)
                {
                    let mut bytes = vec![0; file.size() as usize];
                    if file.read(&mut bytes) == bytes.len() {
                        texture_bytes.insert(name.clone(), bytes);
                    }
                }
            }
        }
        let name = path.rsplit('\\').next().unwrap_or(path).to_string();
        models.push(GalleryModel { name, pieces });
    }
    (models, failures, texture_bytes)
}

/// IDA: N/A the pages and where the screenshots go.
#[derive(Resource)]
struct Pages {
    count: usize,
    out: PathBuf,
    current: usize,
    frames: u32,
}

/// IDA: N/A a page's entity.
#[derive(Component)]
struct OnPage(usize);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let paths: Vec<String> = std::fs::read_to_string(&args[1])
        .unwrap()
        .lines()
        .filter_map(|line| {
            line.trim()
                .split_once(' ')
                .map(|(_, path)| path.to_string())
        })
        .collect();
    let out = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&out).unwrap();
    let (models, failures, textures) = load_gallery(&paths);
    let pieces: usize = models.iter().map(|model| model.pieces.len()).sum();
    println!(
        "gallery: {} of {} models, {} pieces, {} textures read",
        models.len(),
        paths.len(),
        pieces,
        textures.len()
    );
    for failure in &failures {
        println!("failed: {failure}");
    }
    let per_page = COLUMNS * ROWS;
    let count = models.len().div_ceil(per_page);
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Prison gallery".into(),
                resolution: WindowResolution::new(1920, 1080),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.16, 0.17, 0.2)))
        .insert_resource(Pages {
            count,
            out,
            current: 0,
            frames: 0,
        })
        .insert_resource(GallerySource(Some((models, textures))))
        .add_systems(Startup, build)
        .add_systems(Update, pages)
        .run();
}

/// IDA: N/A the loaded models and their textures' bytes, by name.
type GalleryData = (Vec<GalleryModel>, HashMap<String, Vec<u8>>);

/// IDA: N/A the loaded models, handed to `build`.
#[derive(Resource)]
struct GallerySource(Option<GalleryData>);

// IDA: N/A the cell of a model on its page, in world units, the grid centred on the origin.
fn cell_origin(slot: usize) -> Vec3 {
    let column = slot % COLUMNS;
    let row = slot / COLUMNS;
    Vec3::new(
        (column as f32 - (COLUMNS as f32 - 1.0) / 2.0) * CELL,
        ((ROWS as f32 - 1.0) / 2.0 - row as f32) * CELL,
        0.0,
    )
}

fn build(
    mut commands: Commands,
    mut source: ResMut<GallerySource>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let (models, textures) = source.0.take().unwrap();
    let mut image_handles: HashMap<String, Handle<Image>> = HashMap::new();
    for (name, bytes) in &textures {
        if let Some(decoded) = oblivion_host::dds_image::decode_dds(bytes) {
            let image = Image::new(
                Extent3d {
                    width: decoded.width,
                    height: decoded.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                decoded.levels[0].clone(),
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            let mut image = image;
            // The game's maps wrap (the NiTexturingProperty map's clamp mode 3, WRAP_S_WRAP_T, is the default).
            image.sampler =
                bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
                    address_mode_u: bevy::image::ImageAddressMode::Repeat,
                    address_mode_v: bevy::image::ImageAddressMode::Repeat,
                    ..bevy::image::ImageSamplerDescriptor::linear()
                });
            image_handles.insert(name.clone(), images.add(image));
        }
    }
    let plain = materials.add(StandardMaterial {
        base_color: Color::srgb(0.8, 0.8, 0.8),
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let mut material_handles: HashMap<String, Handle<StandardMaterial>> = HashMap::new();
    let per_page = COLUMNS * ROWS;
    let turn = Quat::from_rotation_x(0.35) * Quat::from_rotation_y(-0.6);
    for (index, model) in models.iter().enumerate() {
        let page = index / per_page;
        let slot = index % per_page;
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for piece in &model.pieces {
            for p in &piece.positions {
                let v = Vec3::from(*p);
                low = low.min(v);
                high = high.max(v);
            }
        }
        if model.pieces.is_empty() {
            continue;
        }
        let centre = (low + high) / 2.0;
        let extent = (high - low).max_element().max(1e-3);
        let scale = CELL * 0.8 / extent;
        let origin = cell_origin(slot);
        let parent = commands
            .spawn((
                Transform::from_translation(origin)
                    .with_rotation(turn)
                    .with_scale(Vec3::splat(scale)),
                Visibility::Hidden,
                OnPage(page),
            ))
            .id();
        for piece in &model.pieces {
            let mut mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            );
            let positions: Vec<[f32; 3]> = piece
                .positions
                .iter()
                .map(|p| (Vec3::from(*p) - centre).to_array())
                .collect();
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, piece.normals.clone());
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, piece.uvs.clone());
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, piece.colors.clone());
            mesh.insert_indices(Indices::U32(piece.indices.clone()));
            let material = match piece
                .texture
                .as_ref()
                .and_then(|name| image_handles.get(name).map(|h| (name, h)))
            {
                Some((name, image)) => material_handles
                    .entry(format!("{name}|{:?}", piece.alpha))
                    .or_insert_with(|| {
                        materials.add(StandardMaterial {
                            base_color_texture: Some(image.clone()),
                            double_sided: true,
                            cull_mode: None,
                            alpha_mode: match piece.alpha {
                                Some((flags, reference)) if flags & 0x200 != 0 => {
                                    AlphaMode::Mask(f32::from(reference) / 255.0)
                                }
                                Some((flags, _)) if flags & 1 != 0 => AlphaMode::Blend,
                                _ => AlphaMode::Opaque,
                            },
                            ..default()
                        })
                    })
                    .clone(),
                None => plain.clone(),
            };
            let child = commands
                .spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material)))
                .id();
            commands.entity(parent).add_child(child);
        }
        commands.spawn((
            Text::new(
                model
                    .name
                    .trim_end_matches(".NIF")
                    .trim_end_matches(".nif")
                    .to_string(),
            ),
            TextFont {
                font_size: bevy::text::FontSize::Px(13.0),
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.9, 0.6)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(
                    1920.0 / 2.0 + (origin.x - CELL / 2.0) / (COLUMNS as f32 * CELL) * 1920.0 + 6.0,
                ),
                top: Val::Px(
                    1080.0 / 2.0 - (origin.y + CELL / 2.0) / (ROWS as f32 * CELL + 0.6) * 1080.0
                        + 4.0,
                ),
                ..default()
            },
            Visibility::Hidden,
            OnPage(page),
        ));
    }
    commands.spawn((
        Camera3d::default(),
        Projection::from(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::Fixed {
                width: COLUMNS as f32 * CELL,
                height: ROWS as f32 * CELL + 0.6,
            },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(0.0, 0.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 9000.0,
            ..default()
        },
        Transform::from_xyz(3.0, 6.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        brightness: 900.0,
        ..default()
    });
}

fn pages(
    mut commands: Commands,
    mut pages: ResMut<Pages>,
    mut shown: Query<(&OnPage, &mut Visibility)>,
    mut exit: MessageWriter<AppExit>,
) {
    pages.frames += 1;
    if pages.frames == 1 {
        let current = pages.current;
        for (page, mut visibility) in &mut shown {
            *visibility = if page.0 == current {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
    } else if pages.frames == 40 {
        let path = pages
            .out
            .join(format!("prison_gallery_{:02}.png", pages.current + 1));
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    } else if pages.frames == 70 {
        pages.current += 1;
        pages.frames = 0;
        if pages.current >= pages.count {
            exit.write(AppExit::Success);
        }
    }
}
