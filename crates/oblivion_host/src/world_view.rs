//! IDA: N/A Track R1's first world view: the port's loaded references in the player's cell, drawn as Bevy `Mesh3d` entities lit by Bevy's light entities shaded with the SM3 programs' falloff (Track R3, `cell_lighting` and `world_lighting.wesl`: the interior's XCLL ambient, directional light and fog, the LIGH references' NiPointLights as Bevy point lights carrying the game's colours), with their base maps through `dds_image`. Each reference's tree is walked from its root (`Reference3D`) with the local transforms composed down, as `prison_gallery` does, so the picture doesn't depend on how far the port's node update has run. EditorMarker subtrees, app-culled objects, particle systems and lights aren't drawn. The player's camera is the World SceneGraph's, as the move's `0066B710` and then the RaceSexMenu's `005C2BF0` place it (at the player's face); before those, a host placement: at the player's position, 128 units up, along its Z rotation. The player camera's projection is the scene camera's frustum (`00411160` SceneGraph_SetCameraFOV: fDefaultFOV as a 4:3 angle, 59.8 degrees high at 16:9); the host views use 75 degrees across. Skinned geometry (an actor's body, clothes and head) is deformed on the host from its NiSkinInstance bones' world transforms and NiSkinData's skin-to-bone transforms and weights, as the original's software or partition skinning would place it. Completion: the statics' and the skinned geometry's positions, normals and base maps, lit and fogged with SM3000's diffuse sum and linear fog from the cell's values; not the original's per-object light lists, specular, the skin program's terms, normal maps or alpha sort.
//! TODO: [ ] The shader properties' other maps, the material colours and the original's alpha sort.

use std::collections::{BTreeMap, HashMap, HashSet};

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use oblivion_sim::bs::sky_shader::shader_texture_variant_path;
use oblivion_sim::gamebryo::av_object::NiAvProperties;
use oblivion_sim::gamebryo::mesh_geometry::{MeshTriShapeData, MeshTriShapePayload, NiGeometry};
use oblivion_sim::gamebryo::mesh_skin_owners::{MeshSkinData, MeshSkinInstance};
use oblivion_sim::gamebryo::mesh_tri_strips::MeshTriStripsData;
use oblivion_sim::gamebryo::object_net::{AlphaProperty, NiObjectNet};
use oblivion_sim::gamebryo::scene_node::{NiNodeChildren, SceneNodePayload};
use oblivion_sim::gamebryo::stream_objects::NiClass;
use oblivion_sim::gamebryo::texture::{SourceTexture, TexturingProperty};
use oblivion_sim::gamebryo::transform::{NiTransform, compose_scene_transform};
use oblivion_sim::tes::actor::TesObjectRefr;
use oblivion_sim::tes::player::the_player;
use oblivion_sim::tes::reference_3d::Reference3D;
use oblivion_sim::tes::winmain::driver_files;

use crate::cell_lighting::{
    CellLighting, POINT_LIGHT_UNIT, decode_cell_lighting, directional_direction,
    unit_exposure_ev100,
};
use crate::dds_image::decode_dds;

/// IDA: N/A the hair's draw over Bevy's lights (`hair_material.wesl`): pass 303's SM3003 (`00862B40` Lighting30Shader_GetPassForType's case 303
/// with the specular flag, which `007DA220` BSShaderPPLightingProperty_SetupGeometry sets for a material named "hair"): 005551C0's hair arm's `HairTint` (c2), the NiAlphaProperty's
/// test, and the three maps case 303 and `007D8160` BSShaderPPLightingProperty_LoadTextures / `008635E0`
/// Lighting30ShaderProperty_LoadTextures bind: the `_n` normal map (s1), the `_hl` layer map (s5,
/// property +0x104) and `g_effectAnisoTexture` (s4, `textures\effects\aniso.dds`). Its StandardMaterial carries the base map
/// (s0), the alpha blend and the cull face from the stencil property's draw mode.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct HairTintExtension {
    /// `HairTint` (the Lighting30ShaderProperty's +0xF0..+0xFC).
    #[uniform(100)]
    pub tint: Vec4,
    /// The NiAlphaProperty's test: x 1.0 when +0x18 bit 9 is set, y the function (bits 10-12), z the reference (+0x1A).
    #[uniform(100)]
    pub alpha_test: Vec4,
    /// 1.0 when the map was found, else 0.0 (an unbound stage): x the normal map, y the layer map, z the aniso map.
    #[uniform(100)]
    pub maps: Vec4,
    #[texture(101)]
    #[sampler(102)]
    pub normal_map: Handle<Image>,
    #[texture(103)]
    #[sampler(104)]
    pub layer_map: Handle<Image>,
    #[texture(105)]
    #[sampler(106)]
    pub aniso_map: Handle<Image>,
}

impl bevy::pbr::MaterialExtension for HairTintExtension {
    /// IDA: N/A `HairTintExtension`'s side of `MaterialExtension`.
    fn fragment_shader() -> bevy::shader::ShaderRef {
        bevy::shader::ShaderRef::Handle(HAIR_SHADER)
    }

    /// IDA: N/A pass 303's render states (`0085E660` Lighting30Shader_BuildPasses: ZENABLE 1, ZFUNC LESSEQUAL, ZWRITEENABLE 1):
    /// the blended hair still writes depth, where Bevy's blend pipeline turns depth writes off.
    fn specialize(
        _pipeline: &bevy::pbr::MaterialExtensionPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialExtensionKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(true);
        }
        Ok(())
    }
}

/// IDA: N/A a FaceGen part's SkinShader draw (`face_gen_material.wesl`): the skin's light passes (105/100 and 236/233) with the
/// age normal map, the texture pass (266/261, SLS1005: 005551C0's generated texture as FaceGenMap0 and FaceGenMap1, the age map
/// or g_faceGen's 0x40 grey when `map1.x` is 0.0, over the base map) and the specular passes (294/291).
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct FaceGenExtension {
    #[uniform(100)]
    pub map1: Vec4,
    #[texture(101)]
    #[sampler(102)]
    pub map0_texture: Handle<Image>,
    #[texture(103)]
    #[sampler(104)]
    pub map1_texture: Handle<Image>,
    /// x: 1.0 when the light passes' normal map (+0xC0[0], the age map's `_n`) was found, else the flat normal map's (0x80,
    /// 0x80, 0xFF, alpha 1.0); y: the specular passes' exponent (Toggles c7.z, run-bj's 10.0).
    #[uniform(105)]
    pub skin: Vec4,
    #[texture(106)]
    #[sampler(107)]
    pub normal_texture: Handle<Image>,
}

impl bevy::pbr::MaterialExtension for FaceGenExtension {
    /// IDA: N/A `FaceGenExtension`'s side of `MaterialExtension`.
    fn fragment_shader() -> bevy::shader::ShaderRef {
        bevy::shader::ShaderRef::Handle(FACE_GEN_SHADER)
    }
}

/// IDA: N/A the FaceGen parts' material: StandardMaterial lit by Bevy, with SM3006's blend.
pub type FaceGenMaterial = bevy::pbr::ExtendedMaterial<StandardMaterial, FaceGenExtension>;

/// IDA: N/A the skin's specular exponent: Toggles c7.z (00B46688 +8) at the face's three 294 passes in run-bj
/// (`records-run-bj-toggles.txt`); the property field it comes from isn't read yet.
const SKIN_SPECULAR_POWER: f32 = 10.0;

/// IDA: N/A g_faceGen +0xDB8's grey (`004783A0`, 0x40), FaceGenMap1 when no age map was found.
const FACE_GEN_GREY_40: f32 = 64.0 / 255.0;

/// IDA: N/A the world shaders: each fragment file with `world_lighting.wesl` pasted at its `// @world_lighting` line, so the
/// game's lighting is written once. Registered under these handles by `plugin`.
pub const WORLD_SHADER: Handle<bevy::shader::Shader> =
    bevy::asset::uuid_handle!("6f1d2b4e-3c8a-4f71-9a1e-0d6c5b2a7e10");
pub const HAIR_SHADER: Handle<bevy::shader::Shader> =
    bevy::asset::uuid_handle!("6f1d2b4e-3c8a-4f71-9a1e-0d6c5b2a7e11");
pub const FACE_GEN_SHADER: Handle<bevy::shader::Shader> =
    bevy::asset::uuid_handle!("6f1d2b4e-3c8a-4f71-9a1e-0d6c5b2a7e12");

// IDA: N/A a world shader's source with the shared lighting pasted in.
fn compose_world_shader(fragment: &str) -> String {
    fragment.replace("// @world_lighting", include_str!("world_lighting.wesl"))
}

/// IDA: N/A the world's material extension (`world_material.wesl`): Bevy's lights shaded with the SM3 falloff in the game's
/// numbers (`world_lighting.wesl`). No parameters of its own.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, Default)]
pub struct WorldOutputExtension {}

impl bevy::pbr::MaterialExtension for WorldOutputExtension {
    /// IDA: N/A `WorldOutputExtension`'s side of `MaterialExtension`.
    fn fragment_shader() -> bevy::shader::ShaderRef {
        bevy::shader::ShaderRef::Handle(WORLD_SHADER)
    }
}

/// IDA: N/A the world's material: StandardMaterial lit by Bevy, stored gamma-encoded.
pub type WorldMaterial = bevy::pbr::ExtendedMaterial<StandardMaterial, WorldOutputExtension>;

/// IDA: N/A the hair's material: StandardMaterial lit by Bevy, with the hair tint.
pub type HairMaterial = bevy::pbr::ExtendedMaterial<StandardMaterial, HairTintExtension>;

/// IDA: N/A one drawable piece in world space (Bevy axes: Gamebryo's Z up is Y).
struct Piece {
    /// The geometry it was walked from (the simulation's entity): the render mirror's key.
    source: Entity,
    positions: Vec<[f32; 3]>,
    /// The geometry data's normals (+0x20) in world space, or empty when it has none (Bevy computes smooth ones).
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
    texture: Option<String>,
    /// NiAlphaProperty +0x18 (bit 0 blend, bit 9 test) and its +0x1A reference, from the nearest property list.
    alpha: Option<(u16, u8)>,
    skinned: bool,
    /// 005551C0's hair tint (`HairTint`): the piece draws with `HairMaterial`.
    hair_tint: Option<[f32; 4]>,
    /// The hair's tangent space (`007B7710`'s +0x0C tangents, the TANGENT stream) in world space, w the sign that gives its
    /// +0x10 binormals as w x (normal x tangent); empty when none was made.
    tangents: Vec<[f32; 4]>,
    /// 005551C0's generated texture and FaceGenMap1's path: the piece draws with `FaceGenMaterial`.
    face_gen: Option<(
        std::sync::Arc<oblivion_sim::bs::face_gen_texture::FaceGenTextureImage>,
        Option<String>,
    )>,
    /// The nearest NiStencilProperty's draw mode (+0x18 bits 10-11), 0 with none.
    draw_mode: u16,
    /// The piece is the player's (its reference is g_thePlayer): the live view rebuilds these each frame.
    player: bool,
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
    /// References with skinned pieces: form ID, its position and its skinned vertices' centre and extent (Bevy axes).
    pub skinned: Vec<(u32, [f32; 3], [f32; 3], f32)>,
    /// The interior's lighting block, decoded (none for an exterior).
    pub lighting: Option<CellLighting>,
    /// The cell's lights as the view spawned them.
    pub lights: Vec<CellLight>,
}

/// IDA: N/A a LIGH reference's NiPointLight (004B1600's) as the view lights it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellLight {
    pub form_id: u32,
    /// The NiPointLight's world position, Gamebryo's axes.
    pub position: [f32; 3],
    /// +0xF8, the radius 004B1600 stores there.
    pub radius: f32,
    /// +0xEC diffuse.
    pub diffuse: [f32; 3],
    /// +0xDC dimmer (the base's fade, +0x88).
    pub dimmer: f32,
    /// The shader's colour, 007EDCD0's `diffuse x min(dimmer, 1)` (non-HDR).
    pub color: [f32; 3],
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

// IDA: N/A NiTransform's rotation applied to a direction, normalised (the scale drops out).
fn rotate(t: &NiTransform, v: [f32; 3]) -> [f32; 3] {
    let r = &t.rotation;
    Vec3::new(
        r[0] * v[0] + r[1] * v[1] + r[2] * v[2],
        r[3] * v[0] + r[4] * v[1] + r[5] * v[2],
        r[6] * v[0] + r[7] * v[1] + r[8] * v[2],
    )
    .normalize_or_zero()
    .to_array()
}

// IDA: N/A the base map's file name of a NiTexturingProperty in a property list, or a BSShaderPPLightingProperty's diffuse
// map (+0xBC[0]), which 007DA220 took from that NiTexturingProperty before removing it.
fn base_texture(world: &World, properties: &[Option<Entity>]) -> Option<Option<String>> {
    for property in properties.iter().flatten() {
        let texture = if let Some(texturing) = world.get::<TexturingProperty>(*property) {
            Some(
                texturing
                    .maps_1c
                    .first()
                    .copied()
                    .flatten()
                    .and_then(|map| map.texture_08),
            )
        } else {
            world
                .get::<oblivion_sim::gamebryo::properties::LightingProperty>(*property)
                .filter(|lighting| lighting.diffuse_map_bc.is_some())
                .map(|lighting| lighting.diffuse_map_bc)
        };
        if let Some(texture) = texture {
            let name = texture
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

// IDA: N/A every object's world transform under `object`, culled ones included (a skin's bones often are), composed down
// from the reference's root as the walk does; a light's NiPointLight (004B1600's) included.
fn collect_world(
    world: &World,
    object: Entity,
    parent: &NiTransform,
    out: &mut HashMap<Entity, NiTransform>,
) {
    let local = world
        .get::<SceneNodePayload>(object)
        .map(|payload| payload.local_transform)
        .or_else(|| {
            world
                .get::<MeshTriShapePayload>(object)
                .map(|payload| payload.local_transform)
        })
        .or_else(|| {
            world
                .get::<oblivion_sim::gamebryo::scene_derived::PointLight>(object)
                .map(|light| light.local_transform)
        });
    let Some(local) = local else {
        return;
    };
    let transform = compose_scene_transform(parent, &local);
    out.insert(object, transform);
    let children = world
        .get::<NiNodeChildren>(object)
        .map(|children| children.slots_ac.clone())
        .unwrap_or_default();
    for child in children.into_iter().flatten() {
        collect_world(world, child, &transform, out);
    }
}

// IDA: N/A a skinned geometry's vertices in world space: each bone's world transform composed with its NiSkinData bone's
// skin-to-bone transform (+0x00 of the 0x4C record), each vertex the sum of its weighted bone images. `None` when the skin
// has no data or a bone isn't in the reference's tree.
fn skinned_positions(
    world: &World,
    skin: Entity,
    positions: &[[f32; 3]],
    transforms: &HashMap<Entity, NiTransform>,
) -> Option<Vec<[f32; 3]>> {
    let instance = world.get::<MeshSkinInstance>(skin)?;
    let data = world.get::<MeshSkinData>(instance.data_08?)?;
    let mut out = vec![[0.0f32; 3]; positions.len()];
    for (bone, record) in instance.bone_owners_14.iter().zip(&data.bones) {
        let bone_world = transforms.get(&(*bone)?)?;
        let matrix = compose_scene_transform(bone_world, &record.transform);
        for weight in &record.weights_44 {
            let index = usize::from(weight.vertex_index);
            let Some(&vertex) = positions.get(index) else {
                continue;
            };
            let moved = apply(&matrix, vertex);
            for axis in 0..3 {
                out[index][axis] += weight.weight * moved[axis];
            }
        }
    }
    Some(out)
}

// IDA: N/A a skinned geometry's normals in world space: each vertex's normal turned by its weighted bones' rotations (as
// `skinned_positions`, without the translation), normalised. `None` as `skinned_positions`.
fn skinned_normals(
    world: &World,
    skin: Entity,
    normals: &[[f32; 3]],
    transforms: &HashMap<Entity, NiTransform>,
) -> Option<Vec<[f32; 3]>> {
    let instance = world.get::<MeshSkinInstance>(skin)?;
    let data = world.get::<MeshSkinData>(instance.data_08?)?;
    let mut out = vec![[0.0f32; 3]; normals.len()];
    for (bone, record) in instance.bone_owners_14.iter().zip(&data.bones) {
        let bone_world = transforms.get(&(*bone)?)?;
        let matrix = compose_scene_transform(bone_world, &record.transform);
        for weight in &record.weights_44 {
            let index = usize::from(weight.vertex_index);
            let Some(&normal) = normals.get(index) else {
                continue;
            };
            let turned = rotate(&matrix, normal);
            for axis in 0..3 {
                out[index][axis] += weight.weight * turned[axis];
            }
        }
    }
    Some(
        out.into_iter()
            .map(|n| Vec3::from(n).normalize_or_zero().to_array())
            .collect(),
    )
}

/// IDA: N/A where `007B7710` reads a geometry's triangles: NiTriStrips' +0x4C points as one strip, NiTriShape's through
/// vt+0x60 (`0071FCC0`, three indices at 6 x i); each with the data's +0x40 triangle count.
enum TriangleSource<'a> {
    Strips(&'a [u16], u16),
    Shape(&'a [u16], u16),
}

/// IDA: N/A `007B7710`'s TangentSpaceData (`007D7310`): its +0x0C tangents and +0x10 binormals, one each per vertex.
type TangentFrame = (Vec<[f32; 3]>, Vec<[f32; 3]>);

/// The extra data `007B7710` takes a geometry's tangent space from, before it makes one.
const TANGENT_SPACE_EXTRA: &[u8] = b"Tangent space (binormal & tangent vectors)";

// IDA: 007B7710 BSShaderManager_CreateTangentSpace(geometry), as 007DA220 (the shader property's geometry setup) calls it, then
// 007DA220's fix-up of zero vectors (+0x10 to g_unitXVector3, +0x0C to g_niPoint3UnitY). Returns (+0x0C, +0x10) in model space:
// 007D7F80 BSShaderPPLightingProperty_PackTangentSpace packs +0x0C as stream 0 (0x80000000, the declaration's TANGENT) and +0x10 as stream 1 (BINORMAL). With the
// geometry's "Tangent space (binormal & tangent vectors)" binary extra data, +0x0C is its second 12 x vertex-count bytes and
// +0x10 its first. Otherwise CreateTangentSpaceSimple's arm: per triangle (any of whose two indices are equal is skipped), the
// position step along U, ((p1 - p0)(v2 - v0) - (p2 - p0)(v1 - v0)) / ((u1 - u0)(v2 - v0) - (u2 - u0)(v1 - v0)), summed on its
// three vertices; per vertex the sum less its part along the normal is +0x0C, normal x that is +0x10, each normalised by
// `NiVector3_NormalizeAndGetLength` (zero at a length of 1e-6 or less). A strip walk toggles the winding, which the step
// doesn't depend on. `None` with no normals (the original's "trigeom with no normals" abort).
// Completion: 90% (the extra-data and generated arms, the zero fix-up)
// TODO: [ ] The "STBB" name arm (fixed 48-byte tables) and a geometry whose data isn't triangle-based return `None`.
// TODO: [ ] x87 precision: the sums are f32 here, where the original keeps some products on the FPU stack.
// TODO: [ ] The original removes the extra data after reading it (`NiObjectNET_RemoveExtraDataByName`); the view only reads.
fn tangent_space(
    world: &World,
    object: Entity,
    data: &oblivion_sim::gamebryo::mesh_geometry::MeshGeometryData,
    triangles: &TriangleSource<'_>,
) -> Option<TangentFrame> {
    let count = usize::from(data.vertex_count_08);
    let extra = world.get::<NiObjectNet>(object).and_then(|net| {
        net.extra_data.iter().find_map(|&entry| {
            let name = world.get::<oblivion_sim::gamebryo::object_net::NiExtraDataName>(entry)?;
            (name.0.as_deref() == Some(TANGENT_SPACE_EXTRA)).then_some(())?;
            world.get::<oblivion_sim::gamebryo::extra_data::BinaryExtraData>(entry)
        })
    });
    let vector = |bytes: &[u8; 12]| -> [f32; 3] {
        let word =
            |i: usize| f32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
        [word(0), word(4), word(8)]
    };
    let (mut tangent, mut binormal) = if let Some(extra) = extra {
        if extra.bytes.len() < 24 * count {
            return None;
        }
        let first: Vec<[f32; 3]> = extra.bytes[..12 * count]
            .as_chunks::<12>()
            .0
            .iter()
            .map(vector)
            .collect();
        let second: Vec<[f32; 3]> = extra.bytes[12 * count..24 * count]
            .as_chunks::<12>()
            .0
            .iter()
            .map(vector)
            .collect();
        (second, first)
    } else {
        let positions = &data.positions_1c;
        let normals = &data.normals_20;
        let uvs = &data.texture_coordinates_28;
        if normals.len() < count || positions.len() < count || uvs.len() < count {
            return None;
        }
        let mut sums = vec![[0.0f32; 3]; count];
        let (indices, triangle_count) = match triangles {
            TriangleSource::Strips(points, n) | TriangleSource::Shape(points, n) => {
                (*points, usize::from(*n))
            }
        };
        for i in 0..triangle_count {
            let first = match triangles {
                TriangleSource::Strips(..) => i,
                TriangleSource::Shape(..) => 3 * i,
            };
            let corner = |k: usize| indices.get(first + k).copied();
            let (Some(a), Some(b), Some(c)) = (corner(0), corner(1), corner(2)) else {
                break;
            };
            if a == b || b == c || a == c {
                continue;
            }
            let (a, b, c) = (usize::from(a), usize::from(b), usize::from(c));
            if a >= count || b >= count || c >= count {
                continue;
            }
            let (p0, p1, p2) = (positions[a], positions[b], positions[c]);
            let (t0, t1, t2) = (uvs[a], uvs[b], uvs[c]);
            let du1 = t1[0] - t0[0];
            let du2 = t2[0] - t0[0];
            let dv1 = t1[1] - t0[1];
            let dv2 = t2[1] - t0[1];
            let r = 1.0 / (du1 * dv2 - du2 * dv1);
            let step: [f32; 3] = std::array::from_fn(|axis| {
                let d1 = p1[axis] - p0[axis];
                let d2 = p2[axis] - p0[axis];
                (d1 * dv2 - d2 * dv1) * r
            });
            for vertex in [a, b, c] {
                for axis in 0..3 {
                    sums[vertex][axis] += step[axis];
                }
            }
        }
        let normalise = |v: [f32; 3]| -> [f32; 3] {
            let length = (v[1] * v[1] + v[0] * v[0] + v[2] * v[2]).sqrt();
            if length <= 0.000_001 {
                [0.0; 3]
            } else {
                let inverse = 1.0 / length;
                [v[0] * inverse, inverse * v[1], inverse * v[2]]
            }
        };
        let mut tangent = Vec::with_capacity(count);
        let mut binormal = Vec::with_capacity(count);
        for (sum, n) in sums.iter().zip(normals) {
            let along = n[0] * sum[0] + n[1] * sum[1] + n[2] * sum[2];
            let t = [
                sum[0] - n[0] * along,
                sum[1] - n[1] * along,
                sum[2] - n[2] * along,
            ];
            let b = [
                n[1] * t[2] - n[2] * t[1],
                n[2] * t[0] - t[2] * n[0],
                t[1] * n[0] - n[1] * t[0],
            ];
            tangent.push(normalise(t));
            binormal.push(normalise(b));
        }
        (tangent, binormal)
    };
    for v in &mut binormal {
        if *v == [0.0; 3] {
            *v = [1.0, 0.0, 0.0];
        }
    }
    for v in &mut tangent {
        if *v == [0.0; 3] {
            *v = [0.0, 1.0, 0.0];
        }
    }
    Some((tangent, binormal))
}

// IDA: N/A the walk: each node's local transform composed down, the texturing and alpha properties inherited from the
// nearest ancestor that has one.
#[allow(
    clippy::too_many_arguments,
    reason = "the properties a node passes down: texture, alpha and stencil draw mode"
)]
fn walk(
    world: &World,
    object: Entity,
    parent: &NiTransform,
    inherited: Option<String>,
    inherited_alpha: Option<(u16, u8)>,
    inherited_draw_mode: u16,
    transforms: &HashMap<Entity, NiTransform>,
    pieces: &mut Vec<Piece>,
    gaps: &mut PresentationGaps,
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
    let draw_mode = properties
        .iter()
        .flatten()
        .find_map(|&property| {
            world.get::<oblivion_sim::gamebryo::properties::StencilProperty>(property)
        })
        .map_or(inherited_draw_mode, |stencil| (stencil.flags_18 >> 10) & 3);
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
                walk(
                    world,
                    child,
                    &transform,
                    texture.clone(),
                    alpha,
                    draw_mode,
                    transforms,
                    pieces,
                    gaps,
                );
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
            let Some(geometry) = world.get::<NiGeometry>(object) else {
                return;
            };
            // A FaceGen part keeps its base map's path in place of a texturing property (D13): the view draws from it.
            let texture = world
                .get::<oblivion_sim::tes::face_gen_nodes::FaceGenPartTexture>(object)
                .map(|path| String::from_utf8_lossy(&path.0).into_owned())
                .or(texture);
            // A body part's "skin" geometry takes its race's body texture in place of the NIF's (0047AC20, D13).
            let texture =
                match world.get::<oblivion_sim::tes::biped_attach::SkinPartTexture>(object) {
                    Some(skin) => skin
                        .0
                        .as_ref()
                        .map(|path| String::from_utf8_lossy(path).into_owned()),
                    None => texture,
                };
            let (Some(data), skin) = (geometry.data_b4, geometry.skin_b8) else {
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
            let positions = match skin {
                Some(skin) => {
                    let Some(skinned) =
                        skinned_positions(world, skin, &base.positions_1c, transforms)
                    else {
                        return;
                    };
                    skinned.into_iter().map(to_bevy).collect()
                }
                None => base
                    .positions_1c
                    .iter()
                    .map(|&p| to_bevy(apply(&transform, p)))
                    .collect(),
            };
            let normals = if base.normals_20.len() == count {
                match skin {
                    Some(skin) => skinned_normals(world, skin, &base.normals_20, transforms)
                        .unwrap_or_default()
                        .into_iter()
                        .map(to_bevy)
                        .collect(),
                    None => base
                        .normals_20
                        .iter()
                        .map(|&n| to_bevy(rotate(&transform, n)))
                        .collect(),
                }
            } else {
                Vec::new()
            };
            let hair_tint = world
                .get::<oblivion_sim::tes::face_gen_nodes::HairTint>(object)
                .map(|tint| tint.0);
            // The FaceGen face and ears draw with SkinShader (run-bi), whose light passes read the normal map in tangent
            // space, so they take the same frame as the hair.
            let face_gen_part = world
                .get::<oblivion_sim::tes::face_gen_nodes::FaceGenGeneratedTexture>(object)
                .is_some();
            // The hair's and the skin's tangent space (pass 303's and the skin passes' TANGENT and BINORMAL streams), made in
            // model space as 007B7710 makes it, then turned like the normals.
            let tangents = (hair_tint.is_some() || face_gen_part)
                .then_some(())
                .and_then(|()| {
                    let triangles = if let Some(strips) = world.get::<MeshTriStripsData>(data) {
                        TriangleSource::Strips(&strips.strip_points_4c, strips.triangle_count_40)
                    } else {
                        let shape = world.get::<MeshTriShapeData>(data)?;
                        TriangleSource::Shape(&shape.triangle_list_48, shape.triangle_count_40)
                    };
                    let (tangent, binormal) = tangent_space(world, object, base, &triangles)?;
                    let turn = |vectors: &[[f32; 3]]| -> Option<Vec<[f32; 3]>> {
                        Some(match skin {
                            Some(skin) => skinned_normals(world, skin, vectors, transforms)?
                                .into_iter()
                                .map(to_bevy)
                                .collect(),
                            None => vectors
                                .iter()
                                .map(|&v| to_bevy(rotate(&transform, v)))
                                .collect(),
                        })
                    };
                    let (tangent, binormal) = (turn(&tangent)?, turn(&binormal)?);
                    if normals.len() != count {
                        return None;
                    }
                    Some(
                        tangent
                            .iter()
                            .zip(&binormal)
                            .zip(&normals)
                            .map(|((&t, &b), &n)| {
                                let t = Vec3::from(t);
                                let side = Vec3::from(n).cross(t).dot(Vec3::from(b));
                                [t.x, t.y, t.z, if side < 0.0 { -1.0 } else { 1.0 }]
                            })
                            .collect(),
                    )
                })
                .unwrap_or_default();
            pieces.push(Piece {
                source: object,
                positions,
                normals,
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
                skinned: skin.is_some(),
                draw_mode,
                hair_tint,
                tangents,
                player: false,
                face_gen: world
                    .get::<oblivion_sim::tes::face_gen_nodes::FaceGenGeneratedTexture>(object)
                    .map(|generated| {
                        let map1 = world
                            .get::<oblivion_sim::tes::face_gen_nodes::FaceGenMap1>(object)
                            .and_then(|map| map.0.as_ref())
                            .map(|path| String::from_utf8_lossy(path).into_owned());
                        (generated.0.clone(), map1)
                    }),
            });
        }
        // A point light draws through its reference's light extra (`reference_light`).
        Some(NiClass::PointLight) | None => {}
        Some(class) => gaps.count(format!("{class:?}")),
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

/// `g_effectAnisoTexture`'s file: 007B9CC5 in BSShaderManager_CreateEffectTextures, "%saniso.dds" over
/// "Data\\Textures\\Effects\\" (the finder takes it without the Data folder).
const ANISO_TEXTURE: &str = "Textures\\Effects\\aniso.dds";

// IDA: N/A a hair map with every level its file holds, as D3D9 samples it (the stage defaults' LINEAR filter on the min, mag
// and mip; 007DA220 refuses a map without mipmaps): `texture_image` keeps only the first level, which aliases the strands at
// the race menu's distance. `Rgba8UnormSrgb` for the base map; `Rgba8Unorm` for a shader map (a normal, layer or aniso map),
// whose numbers D3D9 samples without an sRGB decode. Wrapped (NiD3DTextureStage_SetDefaults(stage, 1, 2): WRAP).
fn mipped_texture_image(world: &World, path: &str, format: TextureFormat) -> Option<Image> {
    let files = driver_files(world)?;
    let mut file = files.open_core(path, 0, 0x2800, 0xFFFF)?;
    let mut bytes = vec![0; file.size() as usize];
    let _ = file.read(&mut bytes);
    let decoded = decode_dds(&bytes)?;
    // A level past 1x1 or short of its size ends the chain.
    let mut data = Vec::new();
    let mut levels = 0u32;
    let (mut width, mut height) = (decoded.width, decoded.height);
    for level in &decoded.levels {
        if level.len() != (width * height * 4) as usize {
            break;
        }
        data.extend_from_slice(level);
        levels += 1;
        if width == 1 && height == 1 {
            break;
        }
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    let mut image = Image::new(
        Extent3d {
            width: decoded.width,
            height: decoded.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        decoded.levels[0].clone(),
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    // The levels follow the first, largest first (one layer, so wgpu's layer-major order is the file's).
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels.max(1);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        ..ImageSamplerDescriptor::linear()
    });
    Some(image)
}

// IDA: N/A a shader map as its stored numbers, with its levels (`mipped_texture_image`).
fn stored_texture_image(world: &World, path: &str) -> Option<Image> {
    mipped_texture_image(world, path, TextureFormat::Rgba8Unorm)
}

/// IDA: N/A the StandardMaterial the world draws with: lit by Bevy, with no specular (SM3000, the statics' pixel shader,
/// has none: base x (lights + ambient)), so metallic 0, reflectance 0 and full roughness.
fn lit_material() -> StandardMaterial {
    StandardMaterial {
        unlit: false,
        metallic: 0.0,
        reflectance: 0.0,
        perceptual_roughness: 1.0,
        ..default()
    }
}

// IDA: N/A a colour as the game's own numbers, carried in Bevy's linear components unchanged: `world_lighting.wesl` reads the
// lights', the ambient's and the fog's colours back as the shader constants the SM3 programs get.
fn raw(color: [f32; 3]) -> Color {
    Color::linear_rgb(color[0], color[1], color[2])
}

// IDA: N/A a reference's light: the NiPointLight in its type 0x30 extra (ExtraLight, 004DD130 from 004B1600) and its world
// position from the reference's tree. `None` with no extra, no light in the tree, or a negative light (+0x7C bit 4: the
// negated diffuse can't be a Bevy light).
fn reference_light(
    world: &World,
    reference: Entity,
    transforms: &HashMap<Entity, NiTransform>,
) -> Option<CellLight> {
    let refr = world.get::<TesObjectRefr>(reference)?;
    let oblivion_sim::tes::base_extra_list::ExtraData::LightData { light_00, .. } = refr
        .extra_data_44
        .get(oblivion_sim::tes::base_extra_list::EXTRA_LIGHT)?
        .data
    else {
        return None;
    };
    let light = world.get::<oblivion_sim::gamebryo::scene_derived::PointLight>(light_00)?;
    if light.diffuse_ec.iter().any(|&value| value < 0.0) {
        return None;
    }
    let position = transforms.get(&light_00)?.translation;
    let radius = light.specular_f8[0];
    let dimmer = light.dimmer_dc;
    let color = light.diffuse_ec.map(|value| value * dimmer.min(1.0));
    Some(CellLight {
        form_id: world
            .get::<oblivion_sim::tes::form::FormIdentity>(reference)
            .map_or(0, |identity| identity.form_id_0c),
        position,
        radius,
        diffuse: light.diffuse_ec,
        dimmer,
        color,
    })
}

// IDA: N/A the cell's Bevy lights: each LIGH reference's point light (range its radius, no shadows, as SM3000 draws none),
// and the interior's directional light (00545830's sun light: the XCLL directional colour along its rotations) when that
// colour isn't black.
fn spawn_lights(world: &mut World, lighting: Option<&CellLighting>, lights: &[CellLight]) {
    for light in lights {
        world.spawn((
            PointLight {
                color: raw(light.color),
                intensity: POINT_LIGHT_UNIT,
                range: light.radius,
                radius: 0.0,
                shadow_maps_enabled: false,
                ..default()
            },
            GameLightBase {
                intensity: POINT_LIGHT_UNIT,
                range: light.radius,
            },
            Transform::from_translation(Vec3::from(to_bevy(light.position))),
        ));
    }
    if let Some(lighting) = lighting.filter(|lighting| lighting.directional != [0.0; 3]) {
        let direction = Vec3::from(to_bevy(directional_direction(lighting)));
        let up = if direction.cross(Vec3::Y).length() < 1e-4 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        world.spawn((
            DirectionalLight {
                color: raw(lighting.directional),
                illuminance: 1.0,
                shadow_maps_enabled: false,
                ..default()
            },
            GameLightBase {
                intensity: 1.0,
                range: 0.0,
            },
            Transform::default().looking_to(direction, up),
        ));
    }
}

/// IDA: N/A the view's lighting knobs (Track R3, Bevy's lights with the game's falloff, a recorded host substitution): multipliers and
/// offsets over the values `cell_lighting` derives from the exe (the XCLL ambient and fog, the LIGH lights' radius and colour),
/// so tuning never loses the original's numbers. Defaults leave them as derived.
#[derive(Resource, Reflect, serde::Serialize, serde::Deserialize, Clone, Debug)]
#[reflect(Resource)]
#[serde(default)]
pub struct LightingTuning {
    /// Times the cell's ambient brightness.
    pub ambient_scale: f32,
    /// Times every point light's intensity.
    pub light_intensity_scale: f32,
    /// Times every point light's range (its radius from the exe).
    pub light_range_scale: f32,
    /// Times the directional light's illuminance (where the cell has one).
    pub directional_scale: f32,
    /// Added to the camera's exposure, in stops (positive is brighter).
    pub exposure_stops: f32,
    /// The cell's fog on or off.
    pub fog_enabled: bool,
    /// Times the fog's start and end distances.
    pub fog_near_scale: f32,
    pub fog_far_scale: f32,
    /// The camera's tone curve (none is the original's clamp).
    pub tonemapping: ViewTonemapping,
    /// Bevy's deband dither, against banding in dark gradients.
    pub deband_dither: bool,
}

impl Default for LightingTuning {
    /// IDA: N/A Rust's `Default`: the values as derived.
    fn default() -> Self {
        Self {
            ambient_scale: 1.0,
            light_intensity_scale: 1.0,
            light_range_scale: 1.0,
            directional_scale: 1.0,
            exposure_stops: 0.0,
            fog_enabled: true,
            fog_near_scale: 1.0,
            fog_far_scale: 1.0,
            tonemapping: ViewTonemapping::None,
            deband_dither: false,
        }
    }
}

impl crate::tuning::Tunable for LightingTuning {
    const KEY: &'static str = "lighting";
    const LABEL: &'static str = "World lighting (Track R3)";
    const GROUP: &'static str = "Rendering";
}

/// IDA: N/A Bevy's tone curves, by name for the tuning file.
#[derive(
    Reflect, serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq,
)]
pub enum ViewTonemapping {
    #[default]
    None,
    Reinhard,
    ReinhardLuminance,
    AcesFitted,
    AgX,
    SomewhatBoringDisplayTransform,
    TonyMcMapface,
    BlenderFilmic,
}

impl ViewTonemapping {
    // IDA: N/A the curve as Bevy's component.
    fn bevy(self) -> bevy::core_pipeline::tonemapping::Tonemapping {
        use bevy::core_pipeline::tonemapping::Tonemapping as T;
        match self {
            Self::None => T::None,
            Self::Reinhard => T::Reinhard,
            Self::ReinhardLuminance => T::ReinhardLuminance,
            Self::AcesFitted => T::AcesFitted,
            Self::AgX => T::AgX,
            Self::SomewhatBoringDisplayTransform => T::SomewhatBoringDisplayTransform,
            Self::TonyMcMapface => T::TonyMcMapface,
            Self::BlenderFilmic => T::BlenderFilmic,
        }
    }
}

/// IDA: N/A a view light's values as derived from the exe, which `LightingTuning` scales.
#[derive(Component, Clone, Copy, Debug)]
pub struct GameLightBase {
    /// The intensity that carries the light's colour unchanged (`POINT_LIGHT_UNIT`, or 1 lux for the directional light).
    pub intensity: f32,
    /// A point light's range (0 for the directional light).
    pub range: f32,
}

/// IDA: N/A the view's camera and its derived ambient and fog, which `LightingTuning` scales.
#[derive(Component, Clone, Copy, Debug)]
pub struct WorldViewCamera {
    pub ambient: Option<Color>,
    /// Colour, near and far.
    pub fog: Option<(Color, f32, f32)>,
}

/// IDA: N/A the view's plugin: its tunable and the system applying it.
pub fn plugin(app: &mut App) {
    use crate::tuning::TuningAppExt;
    {
        let mut shaders = app
            .world_mut()
            .resource_mut::<Assets<bevy::shader::Shader>>();
        let _ = shaders.insert(
            WORLD_SHADER.id(),
            bevy::shader::Shader::from_wesl(
                compose_world_shader(include_str!("world_material.wesl")),
                "oblivion_host/world_material.wesl",
            ),
        );
        let _ = shaders.insert(
            FACE_GEN_SHADER.id(),
            bevy::shader::Shader::from_wesl(
                compose_world_shader(include_str!("face_gen_material.wesl")),
                "oblivion_host/face_gen_material.wesl",
            ),
        );
        let _ = shaders.insert(
            HAIR_SHADER.id(),
            bevy::shader::Shader::from_wesl(
                compose_world_shader(include_str!("hair_material.wesl")),
                "oblivion_host/hair_material.wesl",
            ),
        );
    }
    app.register_tunable::<LightingTuning>();
    // The live camera and player mesh sampled the previous completed game frame on the single-world host.
    app.add_systems(
        Update,
        (
            apply_lighting_tuning,
            update_render_mirror.before(crate::winmain_plugin::run_winmain_frame),
        ),
    );
}

// IDA: N/A re-applies `LightingTuning` when it changes or a view (camera or lights) is spawned.
fn apply_lighting_tuning(
    tuning: Res<LightingTuning>,
    new_lights: Query<(), Added<GameLightBase>>,
    new_cameras: Query<(), Added<WorldViewCamera>>,
    mut points: Query<(&GameLightBase, &mut PointLight)>,
    mut directionals: Query<(&GameLightBase, &mut DirectionalLight)>,
    cameras: Query<(Entity, &WorldViewCamera)>,
    mut commands: Commands,
) {
    if !tuning.is_changed() && new_lights.is_empty() && new_cameras.is_empty() {
        return;
    }
    for (base, mut light) in &mut points {
        light.intensity = base.intensity * tuning.light_intensity_scale;
        light.range = base.range * tuning.light_range_scale;
    }
    for (base, mut light) in &mut directionals {
        light.illuminance = base.intensity * tuning.directional_scale;
    }
    for (entity, view) in &cameras {
        let mut camera = commands.entity(entity);
        camera.insert((
            tuning.tonemapping.bevy(),
            bevy::camera::Exposure {
                ev100: unit_exposure_ev100() - tuning.exposure_stops,
            },
            if tuning.deband_dither {
                bevy::core_pipeline::tonemapping::DebandDither::Enabled
            } else {
                bevy::core_pipeline::tonemapping::DebandDither::Disabled
            },
        ));
        if let Some(color) = view.ambient {
            camera.insert(AmbientLight {
                color,
                brightness: tuning.ambient_scale,
                affects_lightmapped_meshes: true,
            });
        }
        match view.fog {
            Some((color, near, far)) if tuning.fog_enabled => {
                camera.insert(DistanceFog {
                    color,
                    directional_light_color: Color::NONE,
                    directional_light_exponent: 8.0,
                    falloff: FogFalloff::Linear {
                        start: near * tuning.fog_near_scale,
                        end: far * tuning.fog_far_scale,
                    },
                });
            }
            _ => {
                camera.remove::<DistanceFog>();
            }
        }
    }
}

// IDA: N/A the live views a cell view offers: the camera it opened with first, then the player's camera, the overview and
// one per actor with 3D in the player's cell.
fn register_live_views(host: &mut World, sim: &mut World, opened: CellViewCamera) {
    let mut cameras = vec![opened];
    for camera in [CellViewCamera::Player, CellViewCamera::Overview] {
        if !cameras.contains(&camera) {
            cameras.push(camera);
        }
    }
    let cell = the_player(sim)
        .and_then(|player| sim.get::<TesObjectRefr>(player))
        .and_then(|refr| refr.parent_cell_40);
    let mut actors = sim.query::<(
        &oblivion_sim::tes::form::FormIdentity,
        &TesObjectRefr,
        &oblivion_sim::tes::actor::Actor,
        &Reference3D,
    )>();
    let mut ids: Vec<u32> = actors
        .iter(sim)
        .filter(|(_, refr, _, _)| cell.is_some() && refr.parent_cell_40 == cell)
        .map(|(identity, _, _, _)| identity.form_id_0c)
        .filter(|&id| id != 0x14)
        .collect();
    ids.sort_unstable();
    for id in ids {
        let camera = CellViewCamera::Actor(id);
        if !cameras.contains(&camera) {
            cameras.push(camera);
        }
    }
    let mut views = host.resource_mut::<crate::live_view::LiveViews>();
    for camera in cameras {
        let name = match camera {
            CellViewCamera::Player => "player camera".to_owned(),
            CellViewCamera::Overview => "overview".to_owned(),
            CellViewCamera::Actor(id) => format!("actor {id:X}"),
        };
        views.add(
            name,
            Box::new(move |world: &mut World| {
                crate::winmain_plugin::with_sim(world, |_, sim| view_camera_placement(sim, camera))
            }),
        );
    }
    views.current = 0;
}

/// IDA: N/A where a host capture's camera stands (the original's camera block is Build 3's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellViewCamera {
    /// The scene camera 0066B710 placed (Camera01's world position and the player's view), or with none, the player
    /// position 128 units up along its heading.
    Player,
    /// Above and behind the player, looking down at it.
    Overview,
    /// 140 units in front of the reference with this form ID, at head height, looking back at its chest.
    Actor(u32),
}

/// IDA: N/A the kinds of simulation object under the view's roots that the presenter met and can't draw (restructure
/// Stage 8b): one count per `NiClass`, printed once at exit and readable by tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PresentationGaps {
    pub counts: BTreeMap<String, u32>,
}

impl PresentationGaps {
    // IDA: N/A one more of a kind.
    fn count(&mut self, kind: String) {
        *self.counts.entry(kind).or_insert(0) += 1;
    }
}

/// IDA: N/A one drawn geometry in the mirror: its render entity, what its mesh and material were made from, and its
/// vertices' fingerprint.
struct MirrorPiece {
    render: Entity,
    shape: PieceShape,
    vertices: u64,
}

/// IDA: N/A the persistent render mirror (restructure Stage 8b): each drawn geometry of the simulation's world (its
/// entity, a lookup only, never iterated for order) and the host entity that draws it. Membership is decided by
/// reachability from the view's roots on each walk, never by change detection, since ported code detaches and re-inserts
/// components within a frame. Replaces the one-shot cell view and the player's re-spawn.
#[derive(Resource)]
pub struct RenderMirror {
    map: HashMap<Entity, MirrorPiece>,
    pub gaps: PresentationGaps,
    overview: bool,
    images: HashMap<String, Option<Handle<Image>>>,
    materials: HashMap<MaterialKey, Handle<WorldMaterial>>,
    lights: Vec<CellLight>,
    light_entities: Vec<Entity>,
    /// The last sync's counts: new, changed in place, respawned and gone pieces.
    pub last_sync: [usize; 4],
}

/// IDA: N/A what one walk of the view's roots found: the pieces in walk order, the lights, and the report's figures.
struct Extraction {
    pieces: Vec<Piece>,
    lights: Vec<CellLight>,
    gaps: PresentationGaps,
    report: WorldViewReport,
    cell: Entity,
}

// IDA: N/A one walk of every reference with 3D in the player's cell, as the view draws them now (their world transforms,
// skins and properties). `None` with no player or cell.
fn extract_view(sim: &mut World) -> Option<Extraction> {
    let player = the_player(sim)?;
    let cell = sim.get::<TesObjectRefr>(player)?.parent_cell_40?;
    let mut query = sim.query::<(Entity, &TesObjectRefr, &Reference3D)>();
    let roots: Vec<(Entity, Entity, [f32; 3])> = query
        .iter(sim)
        .filter(|(_, refr, _)| refr.parent_cell_40 == Some(cell))
        .map(|(reference, refr, node)| (reference, node.0, refr.position_2c))
        .collect();
    let mut ids = sim.query::<(&Reference3D, &oblivion_sim::tes::form::FormIdentity)>();
    let form_of: HashMap<Entity, u32> = ids
        .iter(sim)
        .map(|(node, id)| (node.0, id.form_id_0c))
        .collect();
    let mut report = WorldViewReport {
        references: roots.len(),
        ..WorldViewReport::default()
    };
    let mut pieces = Vec::new();
    let mut lights = Vec::new();
    let mut gaps = PresentationGaps::default();
    for (reference, root, placed) in roots {
        let first = pieces.len();
        let mut transforms = HashMap::new();
        collect_world(sim, root, &NiTransform::default(), &mut transforms);
        if let Some(light) = reference_light(sim, reference, &transforms) {
            lights.push(light);
        }
        walk(
            sim,
            root,
            &NiTransform::default(),
            None,
            None,
            0,
            &transforms,
            &mut pieces,
            &mut gaps,
        );
        if reference == player {
            for piece in &mut pieces[first..] {
                piece.player = true;
            }
        }
        // A reference whose geometry lands far from its own position.
        let at = Vec3::from(to_bevy(placed));
        let far = pieces[first..]
            .iter()
            .flat_map(|piece| piece.positions.iter())
            .map(|&p| Vec3::from(p).distance(at))
            .fold(0f32, f32::max);
        let skinned: Vec<Vec3> = pieces[first..]
            .iter()
            .filter(|piece| piece.skinned)
            .flat_map(|piece| piece.positions.iter().map(|&p| Vec3::from(p)))
            .collect();
        if !skinned.is_empty() {
            let centre = skinned.iter().copied().sum::<Vec3>() / skinned.len() as f32;
            let extent = skinned
                .iter()
                .map(|p| p.distance(centre))
                .fold(0f32, f32::max);
            report.skinned.push((
                form_of.get(&root).copied().unwrap_or(0),
                at.to_array(),
                centre.to_array(),
                extent,
            ));
        }
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
    report.low = low.to_array();
    report.high = high.to_array();
    Some(Extraction {
        pieces,
        lights,
        gaps,
        report,
        cell,
    })
}

// IDA: N/A a piece's vertices as one number: positions, normals and tangents, bit for bit.
fn vertex_fingerprint(piece: &Piece) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for p in &piece.positions {
        p.map(f32::to_bits).hash(&mut hasher);
    }
    for n in &piece.normals {
        n.map(f32::to_bits).hash(&mut hasher);
    }
    for t in &piece.tangents {
        t.map(f32::to_bits).hash(&mut hasher);
    }
    hasher.finish()
}

// IDA: N/A the mirror brought to one extraction, in walk order: a geometry it doesn't hold is spawned (New); one whose shape
// is the same and whose vertices moved gets them written in place, and one whose shape changed is spawned again (Changed);
// a held geometry the walk no longer reached is despawned (Gone).
fn sync_mirror(host: &mut World, sim: &mut World, pieces: Vec<Piece>) {
    let mut mirror = host
        .remove_resource::<RenderMirror>()
        .expect("the view's mirror");
    let mut counts = [0usize; 4];
    let mut reached = HashSet::new();
    for piece in pieces {
        let key = piece.source;
        reached.insert(key);
        let shape = piece_shape(&piece);
        let vertices = vertex_fingerprint(&piece);
        if let Some(held) = mirror.map.get_mut(&key) {
            if held.shape == shape {
                if held.vertices != vertices {
                    write_vertices(host, held.render, piece);
                    held.vertices = vertices;
                    counts[1] += 1;
                }
                continue;
            }
            host.despawn(held.render);
            counts[2] += 1;
        } else {
            counts[0] += 1;
        }
        let player = piece.player;
        let render = spawn_piece(
            host,
            sim,
            piece,
            mirror.overview,
            &mut mirror.images,
            &mut mirror.materials,
        );
        if player {
            host.entity_mut(render).insert(PlayerPiece);
        }
        mirror.map.insert(
            key,
            MirrorPiece {
                render,
                shape,
                vertices,
            },
        );
    }
    let gone: Vec<Entity> = mirror
        .map
        .keys()
        .copied()
        .filter(|key| !reached.contains(key))
        .collect();
    for key in gone {
        if let Some(held) = mirror.map.remove(&key) {
            host.despawn(held.render);
            counts[3] += 1;
        }
    }
    mirror.last_sync = counts;
    host.insert_resource(mirror);
}

// IDA: N/A new vertices, normals and tangents written into a drawn piece's mesh.
fn write_vertices(host: &mut World, render: Entity, piece: Piece) {
    let Some(handle) = host.get::<Mesh3d>(render).map(|mesh| mesh.0.clone()) else {
        return;
    };
    let mut meshes = host.resource_mut::<Assets<Mesh>>();
    let Some(mut mesh) = meshes.get_mut(&handle) else {
        return;
    };
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, piece.positions);
    if !piece.normals.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, piece.normals);
    }
    if piece.tangents.len() == piece.colors.len() && !piece.tangents.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, piece.tangents);
    }
}

// IDA: N/A the cell's lights, spawned again when the extraction's differ from the mirror's.
fn sync_lights(host: &mut World, lighting: Option<&CellLighting>, lights: Vec<CellLight>) {
    let same = host
        .get_resource::<RenderMirror>()
        .is_some_and(|mirror| mirror.lights == lights && !mirror.light_entities.is_empty());
    if same {
        return;
    }
    let old = host
        .get_resource_mut::<RenderMirror>()
        .map(|mut mirror| std::mem::take(&mut mirror.light_entities))
        .unwrap_or_default();
    for entity in old {
        host.despawn(entity);
    }
    let before: HashSet<Entity> = {
        let mut query = host.query_filtered::<Entity, With<GameLightBase>>();
        query.iter(host).collect()
    };
    spawn_lights(host, lighting, &lights);
    let spawned: Vec<Entity> = {
        let mut query = host.query_filtered::<Entity, With<GameLightBase>>();
        query
            .iter(host)
            .filter(|entity| !before.contains(entity))
            .collect()
    };
    let mut mirror = host.resource_mut::<RenderMirror>();
    mirror.light_entities = spawned;
    mirror.lights = lights;
}

/// IDA: N/A spawns the view: the mirror of every reference with 3D in the player's cell, and the camera `camera` names.
/// Returns `None` with no player or cell, or no reference with the actor camera's form ID. A capture extracts once and waits
/// for the render; a live view's mirror follows the game each frame (`update_render_mirror`).
pub fn spawn_cell_view(
    host: &mut World,
    sim: &mut World,
    camera: CellViewCamera,
) -> Option<WorldViewReport> {
    let overview = camera == CellViewCamera::Overview;
    let Extraction {
        pieces,
        lights,
        gaps,
        mut report,
        cell,
    } = extract_view(sim)?;
    host.insert_resource(RenderMirror {
        map: HashMap::new(),
        gaps,
        overview,
        images: HashMap::new(),
        materials: HashMap::new(),
        lights: Vec::new(),
        light_entities: Vec::new(),
        last_sync: [0; 4],
    });
    sync_mirror(host, sim, pieces);
    let lighting = sim
        .get::<oblivion_sim::tes::forms_world::TesObjectCell>(cell)
        .filter(|cell| cell.flags_24 & 1 != 0)
        .and_then(|cell| cell.data_3c)
        .map(|block| decode_cell_lighting(&block));
    sync_lights(host, lighting.as_ref(), lights.clone());
    report.lighting = lighting;
    report.lights = lights;
    {
        let mirror = host.resource::<RenderMirror>();
        report.textures = mirror
            .images
            .values()
            .filter(|image| image.is_some())
            .count();
        report.missing_textures = mirror.images.len() - report.textures;
    }
    let (eye, forward, up) = view_camera_placement(sim, camera)?;
    report.eye = eye.to_array();
    let (width, height, physical) = {
        let mut windows = host.query::<&Window>();
        windows
            .iter(host)
            .next()
            .map_or((1920.0, 1080.0, UVec2::new(1920, 1080)), |window| {
                (
                    window.resolution.width(),
                    window.resolution.height(),
                    UVec2::new(window.physical_width(), window.physical_height()),
                )
            })
    };
    let aspect = width / height;
    // The game camera's projection when the view is the scene camera's: 00411160 SceneGraph_SetCameraFOV's frustum, which
    // treats fDefaultFOV as a 4:3 angle (the world graph's extents scaled by 0.75 / (H / W)). Otherwise the host's 75 degrees
    // across.
    let fov = scene_camera_vertical_fov(sim)
        .filter(|_| camera == CellViewCamera::Player)
        .unwrap_or_else(|| 2.0 * ((75f32.to_radians() / 2.0).tan() / aspect).atan());
    let target = spawn_world_composite(host, physical, Vec2::new(width, height));
    let camera_entity = host.spawn_empty().id();
    host.entity_mut(camera_entity).insert((
        Camera3d::default(),
        // The world draws into its own sRGB-format image (`spawn_world_composite`), which the interface's stack then
        // draws under the menus.
        bevy::camera::RenderTarget::Image(target.into()),
        Msaa::Off,
        Camera {
            order: -2,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        Projection::from(PerspectiveProjection {
            fov,
            near: 5.0,
            far: 100_000.0,
            aspect_ratio: aspect,
            ..default()
        }),
        Transform::from_translation(eye).looking_to(forward, up),
        // Track R3: Bevy's lighting in plain multipliers (exposure 1.0) with no tone curve, as the original's D3D9 target
        // clamps; the cell's ambient and fog on this camera only.
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        bevy::camera::Exposure {
            ev100: unit_exposure_ev100(),
        },
        WorldViewCamera {
            ambient: lighting.as_ref().map(|lighting| raw(lighting.ambient)),
            fog: lighting
                .as_ref()
                .map(|lighting| (raw(lighting.fog_color), lighting.fog_near, lighting.fog_far)),
        },
        crate::live_view::LiveViewCamera,
    ));
    register_live_views(host, sim, camera);
    if let Some(lighting) = lighting {
        host.entity_mut(camera_entity).insert((
            // 004C98C0's ambient, the SM3 shaders' AmbientColor (c0, 007EDCD0 from the sun light's ambient) with
            // ToggleADTS.x 1.
            AmbientLight {
                color: raw(lighting.ambient),
                brightness: 1.0,
                affects_lightmapped_meshes: true,
            },
            // SM3000.vso's fog: saturate((far - d) / (far - near)) blends to FogColor, which is Bevy's linear falloff.
            DistanceFog {
                color: raw(lighting.fog_color),
                directional_light_color: Color::NONE,
                directional_light_exponent: 8.0,
                falloff: FogFalloff::Linear {
                    start: lighting.fog_near,
                    end: lighting.fog_far,
                },
            },
        ));
    }
    Some(report)
}

/// IDA: N/A marks a mesh entity drawn from the player's 3D.
#[derive(Component)]
pub struct PlayerPiece;

/// IDA: N/A what a piece's mesh and material were made from, apart from the vertices themselves: a piece whose shape is
/// unchanged gets new vertices in place; any other change (a new head, hair, eyes, race or texture) spawns the player again.
#[derive(Clone, Debug, PartialEq)]
struct PieceShape {
    texture: Option<String>,
    vertices: usize,
    indices: Vec<u32>,
    alpha: Option<(u16, u8)>,
    draw_mode: u16,
    hair_tint: Option<[u32; 4]>,
    face_gen: Option<(usize, Option<String>)>,
}

// IDA: N/A a piece's shape.
fn piece_shape(piece: &Piece) -> PieceShape {
    PieceShape {
        texture: piece.texture.clone(),
        vertices: piece.positions.len(),
        indices: piece.indices.clone(),
        alpha: piece.alpha,
        draw_mode: piece.draw_mode,
        hair_tint: piece.hair_tint.map(|tint| tint.map(f32::to_bits)),
        face_gen: piece
            .face_gen
            .as_ref()
            .map(|(image, map1)| (std::sync::Arc::as_ptr(image) as usize, map1.clone())),
    }
}

/// IDA: N/A the live view's mirror each frame, as the game's frames change the cell (the race menu's options, the head's
/// rebuilds, the face's animation): the view's roots walked again and the mirror synced, with the cell's lights. The player
/// camera's view follows the scene camera. Captures don't run it: they picture the extraction at their stop.
pub fn update_render_mirror(world: &mut World) {
    if !world
        .get_resource::<crate::live_view::LiveViews>()
        .is_some_and(|views| views.active)
        || !world.contains_resource::<RenderMirror>()
    {
        return;
    }
    crate::winmain_plugin::with_sim(world, update_render_mirror_in);
}

// IDA: N/A `update_render_mirror` with both worlds: the walk from the game's, the mirror in the host's.
fn update_render_mirror_in(host: &mut World, sim: &mut World) {
    // The player camera's view follows the scene camera the game moves (005C2BF0 from the race menu's turn); the fly camera
    // and the other views stay where they are.
    let following = {
        let views = host.resource::<crate::live_view::LiveViews>();
        !views.flying
            && views
                .views
                .get(views.current)
                .is_some_and(|view| view.name == "player camera")
    };
    if following
        && let Some((eye, forward, up)) = view_camera_placement(sim, CellViewCamera::Player)
    {
        let mut cameras =
            host.query_filtered::<&mut Transform, With<crate::live_view::LiveViewCamera>>();
        for mut transform in cameras.iter_mut(host) {
            *transform = Transform::from_translation(eye).looking_to(forward, up);
        }
    }
    let Some(extraction) = extract_view(sim) else {
        return;
    };
    let lighting = sim
        .get::<oblivion_sim::tes::forms_world::TesObjectCell>(extraction.cell)
        .filter(|cell| cell.flags_24 & 1 != 0)
        .and_then(|cell| cell.data_3c)
        .map(|block| decode_cell_lighting(&block));
    host.resource_mut::<RenderMirror>().gaps = extraction.gaps;
    sync_mirror(host, sim, extraction.pieces);
    sync_lights(host, lighting.as_ref(), extraction.lights);
    let [new, changed, respawned, gone] = host.resource::<RenderMirror>().last_sync;
    if new + respawned + gone > 0 {
        println!(
            "[INFO] render mirror: {new} new, {changed} changed in place, {respawned} spawned again, {gone} gone"
        );
    }
}

/// IDA: N/A one piece's mesh and material, spawned: the hair's, a FaceGen part's or a world material (shared by base map and alpha
/// through `materials`; base maps cached in `images`). Answers the entity.
fn spawn_piece(
    host: &mut World,
    sim: &mut World,
    mut piece: Piece,
    overview: bool,
    images: &mut HashMap<String, Option<Handle<Image>>>,
    materials: &mut HashMap<MaterialKey, Handle<WorldMaterial>>,
) -> Entity {
    let image = piece.texture.as_ref().and_then(|name| {
        if !images.contains_key(name) {
            let made = texture_image(sim, name);
            if made.is_none() {
                println!("[WARNING] world view: can't draw texture {name}");
            }
            let handle = made.map(|image| host.resource_mut::<Assets<Image>>().add(image));
            images.insert(name.clone(), handle);
        }
        images[name].clone()
    });
    let alpha_mode = match piece.alpha {
        Some((flags, reference)) if flags & 0x200 != 0 => {
            AlphaMode::Mask(f32::from(reference) / 255.0)
        }
        Some((flags, _)) if flags & 1 != 0 => AlphaMode::Blend,
        _ => AlphaMode::Opaque,
    };
    // Every mesh keeps its main-world copy, so the mirror can write new vertices into it (restructure Stage 8b).
    let usage = RenderAssetUsages::all();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, usage);
    let has_normals = !piece.normals.is_empty();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, piece.positions);
    if has_normals {
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, piece.normals);
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, piece.uvs);
    if let Some(tint) = piece.hair_tint {
        // The hair: the vertex green weighs the tint (SM3003's v1.y), so it goes in the second UV set.
        let greens: Vec<[f32; 2]> = piece.colors.iter().map(|c| [c[1], 0.0]).collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, greens);
        // The shader reads a TANGENT stream: with none made, +X and no normal map.
        let has_frame = piece.tangents.len() == piece.colors.len();
        let tangents = if has_frame {
            piece.tangents
        } else {
            vec![[1.0, 0.0, 0.0, 1.0]; piece.colors.len()]
        };
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
        mesh.insert_indices(Indices::U32(piece.indices));
        if !has_normals {
            mesh.compute_smooth_normals();
        }
        // The maps pass 303 binds besides the base map, as their stored numbers (D3D9 samples them unconverted).
        let variant = |host: &mut World, sim: &mut World, suffix: &[u8]| -> Option<Handle<Image>> {
            let path =
                shader_texture_variant_path(piece.texture.as_ref()?.as_bytes(), suffix).ok()?;
            let path = String::from_utf8_lossy(&path).into_owned();
            let made = stored_texture_image(sim, &path).or_else(|| {
                path.get(5..)
                    .filter(|_| path[..5].eq_ignore_ascii_case("Data\\"))
                    .and_then(|rest| stored_texture_image(sim, rest))
            })?;
            Some(host.resource_mut::<Assets<Image>>().add(made))
        };
        let normal_map = if has_frame {
            variant(host, sim, b"_n")
        } else {
            None
        };
        let layer_map = variant(host, sim, b"_hl");
        let aniso_map = stored_texture_image(sim, ANISO_TEXTURE)
            .map(|made| host.resource_mut::<Assets<Image>>().add(made));
        if aniso_map.is_none() {
            println!("[WARNING] world view: can't draw texture {ANISO_TEXTURE}");
        }
        // A stage with no texture: its sampler state's border, never read through `maps`.
        let blank = host.resource_mut::<Assets<Image>>().add(Image::default());
        let (alpha_flags, alpha_reference) = piece.alpha.unwrap_or((0, 0));
        // The base map with its levels, apart from the view's shared first-level image.
        let base_map = piece
            .texture
            .as_deref()
            .and_then(|path| mipped_texture_image(sim, path, TextureFormat::Rgba8UnormSrgb))
            .map(|made| host.resource_mut::<Assets<Image>>().add(made))
            .or(image);
        let material = host
            .resource_mut::<Assets<HairMaterial>>()
            .add(HairMaterial {
                base: StandardMaterial {
                    base_color_texture: base_map,
                    // The NiAlphaProperty's blend (bit 0) draws blended; its test runs in the shader (Bevy's mask
                    // can't stack on a blend).
                    alpha_mode: if alpha_flags & 1 != 0 {
                        AlphaMode::Blend
                    } else {
                        AlphaMode::Opaque
                    },
                    double_sided: piece.draw_mode == 3,
                    cull_mode: match piece.draw_mode {
                        3 => None,
                        2 => Some(bevy::render::render_resource::Face::Front),
                        _ => Some(bevy::render::render_resource::Face::Back),
                    },
                    ..lit_material()
                },
                extension: HairTintExtension {
                    tint: Vec4::from(tint),
                    alpha_test: Vec4::new(
                        if alpha_flags & 0x200 != 0 { 1.0 } else { 0.0 },
                        f32::from((alpha_flags >> 10) & 7),
                        f32::from(alpha_reference),
                        0.0,
                    ),
                    maps: Vec4::new(
                        if normal_map.is_some() { 1.0 } else { 0.0 },
                        if layer_map.is_some() { 1.0 } else { 0.0 },
                        if aniso_map.is_some() { 1.0 } else { 0.0 },
                        0.0,
                    ),
                    normal_map: normal_map.unwrap_or_else(|| blank.clone()),
                    layer_map: layer_map.unwrap_or_else(|| blank.clone()),
                    aniso_map: aniso_map.unwrap_or(blank),
                },
            });
        let mesh = host.resource_mut::<Assets<Mesh>>().add(mesh);
        return host.spawn((Mesh3d(mesh), MeshMaterial3d(material))).id();
    }
    if piece.face_gen.is_some() {
        // The skin's light passes read a TANGENT stream: with none made, +X (the flat normal map then gives N).
        let tangents = if piece.tangents.len() == piece.colors.len() {
            std::mem::take(&mut piece.tangents)
        } else {
            vec![[1.0, 0.0, 0.0, 1.0]; piece.colors.len()]
        };
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, piece.colors);
    mesh.insert_indices(Indices::U32(piece.indices));
    if !has_normals {
        mesh.compute_smooth_normals();
    }
    if let Some((generated, map1_path)) = piece.face_gen {
        // SM3006's blend: the generated texture as its stored numbers (Rgba8Unorm), the age map as a base map is.
        let map0 = Image::new(
            bevy::render::render_resource::Extent3d {
                width: generated.width,
                height: generated.height,
                depth_or_array_layers: 1,
            },
            bevy::render::render_resource::TextureDimension::D2,
            generated.rgba.clone(),
            bevy::render::render_resource::TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        let map0 = host.resource_mut::<Assets<Image>>().add(map0);
        let map1 = map1_path
            .as_deref()
            .and_then(|path| texture_image(sim, path));
        let has_map1 = map1.is_some();
        let map1 = match map1 {
            Some(image) => host.resource_mut::<Assets<Image>>().add(image),
            None => map0.clone(),
        };
        // The light passes' normal map (sub_848FD0: the property's +0xC0[0]): run-au found the age map's `_n` there
        // (HeadHumanM10_n.dds and M60_n), which 007D8160 takes from the base map's otherwise; as stored numbers.
        let normal_source = map1_path.as_deref().or(piece.texture.as_deref());
        let normal = normal_source
            .and_then(|path| shader_texture_variant_path(path.as_bytes(), b"_n").ok())
            .map(|path| String::from_utf8_lossy(&path).into_owned())
            .and_then(|path| {
                stored_texture_image(sim, &path).or_else(|| {
                    path.get(5..)
                        .filter(|_| path[..5].eq_ignore_ascii_case("Data\\"))
                        .and_then(|rest| stored_texture_image(sim, rest))
                })
            });
        let has_normal = normal.is_some();
        let normal = match normal {
            Some(image) => host.resource_mut::<Assets<Image>>().add(image),
            None => map0.clone(),
        };
        let material = host
            .resource_mut::<Assets<FaceGenMaterial>>()
            .add(FaceGenMaterial {
                base: StandardMaterial {
                    base_color_texture: image,
                    double_sided: !overview,
                    cull_mode: overview.then_some(bevy::render::render_resource::Face::Back),
                    alpha_mode,
                    ..lit_material()
                },
                extension: FaceGenExtension {
                    map1: Vec4::new(if has_map1 { 1.0 } else { 0.0 }, FACE_GEN_GREY_40, 0.0, 0.0),
                    map0_texture: map0,
                    map1_texture: map1,
                    skin: Vec4::new(
                        if has_normal { 1.0 } else { 0.0 },
                        SKIN_SPECULAR_POWER,
                        0.0,
                        0.0,
                    ),
                    normal_texture: normal,
                },
            });
        let mesh = host.resource_mut::<Assets<Mesh>>().add(mesh);
        return host.spawn((Mesh3d(mesh), MeshMaterial3d(material))).id();
    }
    let key = (image.as_ref().and(piece.texture.clone()), piece.alpha);
    let material = if let Some(material) = materials.get(&key) {
        material.clone()
    } else {
        let material = WorldMaterial {
            base: StandardMaterial {
                base_color_texture: image,
                double_sided: !overview,
                cull_mode: overview.then_some(bevy::render::render_resource::Face::Back),
                alpha_mode,
                ..lit_material()
            },
            extension: WorldOutputExtension {},
        };
        let handle = host.resource_mut::<Assets<WorldMaterial>>().add(material);
        materials.insert(key, handle.clone());
        handle
    };
    let mesh = host.resource_mut::<Assets<Mesh>>().add(mesh);
    host.spawn((Mesh3d(mesh), MeshMaterial3d(material))).id()
}

/// IDA: N/A the render layer of the world's image in the interface's stack.
pub const WORLD_COMPOSITE_LAYER: usize = 30;

// IDA: N/A D3D9's back buffer holds the world in 8-bit gamma numbers. A 3D camera on the window can't: Bevy gives its main
// texture the interface's raw `Rgba8Unorm` but resolves the view to linear (only Camera2d views encode into a compositing
// space), so the world was stored as linear 8-bit and every tone below about 13/255 on screen rounded to black. Here the
// world renders into an `Rgba8UnormSrgb` image (the GPU encodes on store, D3D9's 8-bit gamma grid), and a Camera2d in the
// interface's `CompositingSpace::Srgb`, ordered under the menus, draws it as a plain sprite (decoded to linear, encoded back
// into the gamma target: the same numbers to within one level). The image takes the window's size, and follows it through
// `game_screen::follow_window` when the window is resized or rescaled. Returns the image.
fn spawn_world_composite(world: &mut World, physical: UVec2, logical: Vec2) -> Handle<Image> {
    let mut image = Image::new_target_texture(
        physical.x.max(1),
        physical.y.max(1),
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        None,
    );
    image.sampler = bevy::image::ImageSampler::nearest();
    let handle = world.resource_mut::<Assets<Image>>().add(image);
    let layer = bevy::camera::visibility::RenderLayers::layer(WORLD_COMPOSITE_LAYER);
    world.spawn((
        Camera2d,
        bevy::camera::CompositingSpace::Srgb,
        Msaa::Off,
        Camera {
            order: -1,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        layer.clone(),
    ));
    world.spawn((
        Sprite {
            image: handle.clone(),
            custom_size: Some(logical),
            ..default()
        },
        layer,
        // `game_screen::follow_window` keeps the image and the sprite at the window's size.
        crate::game_screen::WindowSizedTarget {
            image: handle.clone(),
        },
    ));
    handle
}

/// IDA: N/A where the view's camera stands for `camera`: its eye, forward and up in Bevy's axes. `None` when an actor view
/// names no reference with that form ID, or there is no player. Live views call it to switch cameras without respawning.
pub fn view_camera_placement(
    world: &mut World,
    camera: CellViewCamera,
) -> Option<(Vec3, Vec3, Vec3)> {
    let overview = camera == CellViewCamera::Overview;
    let player = the_player(world)?;
    let (position, rotation) = {
        let refr = world.get::<TesObjectRefr>(player)?;
        (refr.position_2c, refr.rotation_20)
    };
    // The host placement: eye height 128 over the player, facing its Z rotation (Gamebryo's heading: 0 is +Y, clockwise).
    let heading = rotation[2];
    let mut up = Vec3::Y;
    let (eye, forward) = if let CellViewCamera::Actor(id) = camera {
        let mut ids = world.query::<(&oblivion_sim::tes::form::FormIdentity, &TesObjectRefr)>();
        let (at, facing) = ids
            .iter(world)
            .find(|(identity, _)| identity.form_id_0c == id)
            .map(|(_, refr)| (refr.position_2c, refr.rotation_20[2]))?;
        let front = [facing.sin(), facing.cos(), 0.0];
        let eye = Vec3::from(to_bevy([
            at[0] + front[0] * 140.0,
            at[1] + front[1] * 140.0,
            at[2] + 110.0,
        ]));
        let target = Vec3::from(to_bevy([at[0], at[1], at[2] + 80.0]));
        (eye, target - eye)
    } else if overview {
        // Above and behind the player, looking down at it: back faces culled, so the ceilings drop out.
        let centre = Vec3::from(to_bevy(position));
        let back = Vec3::from(to_bevy([-heading.sin(), -heading.cos(), 0.0]));
        let eye = centre + back * 900.0 + Vec3::Y * 1400.0;
        (eye, centre - eye)
    } else if let Some((at, rotation)) = scene_camera(world) {
        // The camera 0066B710 placed (Build 3): the World SceneGraph's camera, which looks along its world rotation's first
        // column with the second as up (NiCamera's world direction and up vector).
        up = Vec3::from(to_bevy([rotation[1], rotation[4], rotation[7]]));
        (
            Vec3::from(to_bevy(at)),
            Vec3::from(to_bevy([rotation[0], rotation[3], rotation[6]])),
        )
    } else {
        (
            Vec3::from(to_bevy([position[0], position[1], position[2] + 128.0])),
            Vec3::from(to_bevy([heading.sin(), heading.cos(), 0.0])),
        )
    };
    Some((eye, forward, up))
}

/// IDA: N/A the World SceneGraph's camera (+0xDC) world translate and row-major rotation, once the move's 0066B710 has
/// placed it; `None` before that (no scene graph, no camera, or a camera never updated).
/// IDA: N/A (host). The vertical field of view of the World SceneGraph camera's frustum (NiCamera +0xEC, left/right/top/
/// bottom at unit distance, as 00411160 set it): the angle between the top and bottom planes. None before 00411160 has
/// run (SceneGraph +0xEC still 0) or for an orthographic frustum. Completion: the symmetric perspective case; an
/// off-centre frustum's shift isn't carried to Bevy.
fn scene_camera_vertical_fov(world: &World) -> Option<f32> {
    let graph = world
        .get_resource::<oblivion_sim::bs::scene_graphs::SceneGraphGlobals>()?
        .world?;
    let state = world.get::<oblivion_sim::bs::scene_graphs::SceneGraph>(graph)?;
    if state.camera_fov_ec == 0.0 {
        return None;
    }
    let camera = world.get::<oblivion_sim::gamebryo::camera::SceneCamera>(state.camera_dc?)?;
    let frustum = camera.frustum_ec;
    (!frustum.orthographic && frustum.top > frustum.bottom)
        .then(|| frustum.top.atan() - frustum.bottom.atan())
}

fn scene_camera(world: &World) -> Option<([f32; 3], [f32; 9])> {
    let graph = world
        .get_resource::<oblivion_sim::bs::scene_graphs::SceneGraphGlobals>()?
        .world?;
    let camera = world
        .get::<oblivion_sim::bs::scene_graphs::SceneGraph>(graph)?
        .camera_dc?;
    let state = world.get::<oblivion_sim::gamebryo::scene_node::NiAvWorld>(camera)?;
    let rotation = state.transform.rotation;
    if rotation.iter().all(|&value| value == 0.0) {
        return None;
    }
    Some((state.transform.translation, rotation))
}
