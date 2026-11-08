//! IDA: N/A Track R1's first world view: the port's loaded references in the player's cell, drawn as Bevy `Mesh3d` entities, unlit, with their base maps through `dds_image`. Each reference's tree is walked from its root (`Reference3D`) with the local transforms composed down, as `prison_gallery` does, so the picture doesn't depend on how far the port's node update has run. EditorMarker subtrees, app-culled objects, particle systems and lights aren't drawn. The camera is a host placement until Build 3's camera block (`0066F09C`): at the player's position, 128 units up, along its Z rotation, with `fDefaultFOV`'s 75 degrees across. Completion: the statics' geometry and base maps; no shaders, lighting, fog or sorting of the original's.
//! TODO: [ ] Build 3: the port's camera node (`SceneCamera`'s world transform after the camera block) in place of the host placement.
//! TODO: [ ] The shader properties' other maps, the material colours and the original's alpha sort.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use recomp_game::gamebryo::av_object::NiAvProperties;
use recomp_game::gamebryo::mesh_geometry::{MeshTriShapeData, MeshTriShapePayload, NiGeometry};
use recomp_game::gamebryo::mesh_tri_strips::MeshTriStripsData;
use recomp_game::gamebryo::object_net::{AlphaProperty, NiObjectNet};
use recomp_game::gamebryo::scene_node::{NiNodeChildren, SceneNodePayload};
use recomp_game::gamebryo::stream_objects::NiClass;
use recomp_game::gamebryo::texture::{SourceTexture, TexturingProperty};
use recomp_game::gamebryo::transform::{NiTransform, compose_scene_transform};
use recomp_game::tes::actor::TesObjectRefr;
use recomp_game::tes::ai_conditions::the_player;
use recomp_game::tes::reference_3d::Reference3D;
use recomp_startup::winmain::driver_files;

use crate::dds_image::decode_dds;

/// IDA: N/A one drawable piece in world space (Bevy axes: Gamebryo's Z up is Y).
struct Piece {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
    texture: Option<String>,
    /// NiAlphaProperty +0x18 (bit 0 blend, bit 9 test) and its +0x1A reference, from the nearest property list.
    alpha: Option<(u16, u8)>,
}

/// IDA: N/A a material by its base map and its alpha property's flags and reference.
type MaterialKey = (Option<String>, Option<(u16, u8)>);

/// IDA: N/A what the view drew, for the capture's report.
#[derive(Resource, Clone, Debug, Default)]
pub struct WorldViewReport {
    pub references: usize,
    pub pieces: usize,
    pub textures: usize,
    pub missing_textures: usize,
    /// The drawn geometry's bounds and the camera's eye, in Bevy's axes.
    pub low: [f32; 3],
    pub high: [f32; 3],
    pub eye: [f32; 3],
    /// References with a vertex over 10000 units from their own position: form ID and that distance.
    pub outliers: Vec<(u32, f32)>,
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

// IDA: N/A the walk: each node's local transform composed down, the texturing and alpha properties inherited from the
// nearest ancestor that has one.
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
        .unwrap_or_default();
    if name.starts_with(b"EditorMarker") {
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
        .find_map(|&property| world.get::<AlphaProperty>(property))
        .map(|alpha| (alpha.flags_18, alpha.alpha_test_ref_1a))
        .or(inherited_alpha);
    match world.get::<NiClass>(object) {
        Some(NiClass::Node | NiClass::BillboardNode) => {
            let Some(payload) = world.get::<SceneNodePayload>(object) else {
                return;
            };
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
            let Some(payload) = world.get::<MeshTriShapePayload>(object) else {
                return;
            };
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
            pieces.push(Piece {
                positions: base
                    .positions_1c
                    .iter()
                    .map(|&p| to_bevy(apply(&transform, p)))
                    .collect(),
                uvs: if base.texture_coordinates_28.len() >= count {
                    base.texture_coordinates_28[..count].to_vec()
                } else {
                    vec![[0.0, 0.0]; count]
                },
                colors: if base.colors_24.len() == count {
                    base.colors_24.clone()
                } else {
                    vec![[1.0; 4]; count]
                },
                indices,
                texture,
                alpha,
            });
        }
        _ => {}
    }
}

// IDA: N/A a base map through the game's own finder and archives, as the screens open theirs.
fn texture_image(world: &World, path: &str) -> Option<Image> {
    let files = driver_files(world)?;
    let mut file = files.open_core(path, 0, 0x2800, 0xFFFF)?;
    let mut bytes = vec![0; file.size() as usize];
    let _ = file.read(&mut bytes);
    let decoded = decode_dds(&bytes)?;
    let mut image = Image::new(
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
    // The NiTexturingProperty map's clamp mode 3 (WRAP_S_WRAP_T) is the default.
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        ..ImageSamplerDescriptor::linear()
    });
    Some(image)
}

/// IDA: N/A spawns the view: the meshes of every reference with 3D in the player's cell, and the camera (at the player, or
/// with `overview` above and behind the drawn geometry's bounds, looking at their centre). Returns `None` with no player or
/// cell.
pub fn spawn_cell_view(world: &mut World, overview: bool) -> Option<WorldViewReport> {
    let player = the_player(world)?;
    let (cell, position, rotation) = {
        let refr = world.get::<TesObjectRefr>(player)?;
        (refr.parent_cell_40?, refr.position_2c, refr.rotation_20)
    };
    let mut query = world.query::<(&TesObjectRefr, &Reference3D)>();
    let roots: Vec<(Entity, [f32; 3])> = query
        .iter(world)
        .filter(|(refr, _)| refr.parent_cell_40 == Some(cell))
        .map(|(refr, node)| (node.0, refr.position_2c))
        .collect();
    let mut ids = world.query::<(&Reference3D, &recomp_game::tes::form::FormIdentity)>();
    let form_of: HashMap<Entity, u32> = ids
        .iter(world)
        .map(|(node, id)| (node.0, id.form_id_0c))
        .collect();
    let mut report = WorldViewReport {
        references: roots.len(),
        ..WorldViewReport::default()
    };
    let mut pieces = Vec::new();
    for (root, placed) in roots {
        let first = pieces.len();
        walk(
            world,
            root,
            &NiTransform::default(),
            None,
            None,
            &mut pieces,
        );
        // A reference whose geometry lands far from its own position.
        let at = Vec3::from(to_bevy(placed));
        let far = pieces[first..]
            .iter()
            .flat_map(|piece| piece.positions.iter())
            .map(|&p| Vec3::from(p).distance(at))
            .fold(0f32, f32::max);
        if far > 10_000.0 {
            report
                .outliers
                .push((form_of.get(&root).copied().unwrap_or(0), far));
        }
    }
    report.pieces = pieces.len();
    let (low, high) = pieces.iter().flat_map(|piece| piece.positions.iter()).fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(low, high), &p| (low.min(Vec3::from(p)), high.max(Vec3::from(p))),
    );
    let mut images: HashMap<String, Option<Handle<Image>>> = HashMap::new();
    let mut materials: HashMap<MaterialKey, Handle<StandardMaterial>> = HashMap::new();
    for piece in pieces {
        let image = piece.texture.as_ref().and_then(|name| {
            if !images.contains_key(name) {
                let made = texture_image(world, name);
                if made.is_none() {
                    println!("[WARNING] world view: can't draw texture {name}");
                }
                let handle = made.map(|image| world.resource_mut::<Assets<Image>>().add(image));
                images.insert(name.clone(), handle);
            }
            images[name].clone()
        });
        let key = (image.as_ref().and(piece.texture.clone()), piece.alpha);
        let material = if let Some(material) = materials.get(&key) {
            material.clone()
        } else {
            let material = StandardMaterial {
                base_color_texture: image,
                unlit: true,
                double_sided: !overview,
                cull_mode: overview.then_some(bevy::render::render_resource::Face::Back),
                alpha_mode: match piece.alpha {
                    Some((flags, reference)) if flags & 0x200 != 0 => {
                        AlphaMode::Mask(f32::from(reference) / 255.0)
                    }
                    Some((flags, _)) if flags & 1 != 0 => AlphaMode::Blend,
                    _ => AlphaMode::Opaque,
                },
                ..default()
            };
            let handle = world
                .resource_mut::<Assets<StandardMaterial>>()
                .add(material);
            materials.insert(key, handle.clone());
            handle
        };
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, piece.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, piece.uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, piece.colors);
        mesh.insert_indices(Indices::U32(piece.indices));
        let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh);
        world.spawn((Mesh3d(mesh), MeshMaterial3d(material)));
    }
    report.low = low.to_array();
    report.high = high.to_array();
    report.textures = images.values().filter(|image| image.is_some()).count();
    report.missing_textures = images.len() - report.textures;
    // The host placement: eye height 128 over the player, facing its Z rotation (Gamebryo's heading: 0 is +Y, clockwise).
    let heading = rotation[2];
    let (eye, forward) = if overview {
        // Above and behind the player, looking down at it: back faces culled, so the ceilings drop out.
        let centre = Vec3::from(to_bevy(position));
        let back = Vec3::from(to_bevy([-heading.sin(), -heading.cos(), 0.0]));
        let eye = centre + back * 900.0 + Vec3::Y * 1400.0;
        (eye, centre - eye)
    } else {
        (
            Vec3::from(to_bevy([position[0], position[1], position[2] + 128.0])),
            Vec3::from(to_bevy([heading.sin(), heading.cos(), 0.0])),
        )
    };
    report.eye = eye.to_array();
    let (width, height) = {
        let mut windows = world.query::<&Window>();
        windows
            .iter(world)
            .next()
            .map_or((1920.0, 1080.0), |window| {
                (window.resolution.width(), window.resolution.height())
            })
    };
    let aspect = width / height;
    let horizontal = 75f32.to_radians();
    world.spawn((
        Camera3d::default(),
        // The interface's camera settings, so both draw into one target and the menus can draw over the world.
        bevy::camera::CompositingSpace::Srgb,
        Msaa::Off,
        Camera {
            order: -1,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        Projection::from(PerspectiveProjection {
            fov: 2.0 * ((horizontal / 2.0).tan() / aspect).atan(),
            near: 5.0,
            far: 100_000.0,
            aspect_ratio: aspect,
            ..default()
        }),
        Transform::from_translation(eye).looking_to(forward, Vec3::Y),
    ));
    Some(report)
}
