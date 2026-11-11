//! A FaceGen head's node tree as `0052DED0` TESRace_BuildFaceGenHead builds it: the BSFaceGenAnimationData (`0054CAF0`), the
//! BSFaceGenNiNodeBiped and BSFaceGenNiNodeSkinned nodes (`0055CD70`), and each part's morphed clone under them
//! (`00555A80`'s loop: `00559870` -> `00556A70`, the part's name, texture, alpha and "skin" material properties, then the
//! skinned node for a skinned clone, else the biped node with a -90 degree Y rotation).
//!
//! Evidence: run-af's after_move (`facegen_nodes.rows`: each NPC's two nodes' children, names, classes and positions).

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use crate::bs::scene_graphs::{AttachChildStatus, attach_scene_child};
use crate::bs::sky_shader::PrepareGeometryContext;
use crate::gamebryo::av_object::{detach_first_property_of_type, property_of_type};
use crate::gamebryo::clone::{CloningProcess, clone_object};
use crate::gamebryo::mesh_geometry::{MeshTriShapeData, MeshTriShapePayload, NiGeometry};
use crate::gamebryo::mesh_skin_owners::MeshSkinInstance;
use crate::gamebryo::mesh_tri_strips::MeshTriStripsData;
use crate::gamebryo::object_net::{AlphaProperty, NiObjectNet};
use crate::gamebryo::scene_node::{SceneNodePayload, spawn_scene_node};
use crate::gamebryo::stream_objects::NiClass;
use crate::tes::game_settings::float_named_or_zero;

/// IDA: BSFaceGenKeyframeMultiple (0x14). The Vec owns the +0x0C values and supplies the +0x10 count.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceGenKeyframe {
    pub type_04: u32,
    pub time_08: f32,
    pub values_0c: Vec<f32>,
}

impl FaceGenKeyframe {
    /// IDA: 0054EA00(type, count). Completion: 100%.
    #[must_use]
    pub fn new(type_04: u32, count: usize) -> Self {
        Self {
            type_04,
            time_08: 0.0,
            values_0c: vec![0.0; count],
        }
    }

    /// IDA: 0054EA70 (vt+0x48): the value, 0.0 past the count.
    #[must_use]
    pub fn value(&self, index: usize) -> f32 {
        self.values_0c.get(index).copied().unwrap_or(0.0)
    }

    /// IDA: 0054A3E0 (vt+0x4C): past the count or an equal value answers false; else stores it. Completion: 100%.
    pub fn set_value(&mut self, index: usize, value: f32) -> bool {
        match self.values_0c.get_mut(index) {
            Some(slot) if *slot != value => {
                *slot = value;
                true
            }
            _ => false,
        }
    }

    /// IDA: 0054E820 (vt+0x54): a value in [0, 1] (`> 0` and `== 0` differ, then `<= 1`). Completion: 100%.
    #[must_use]
    pub fn is_set(&self, index: usize) -> bool {
        self.values_0c
            .get(index)
            .is_some_and(|&value| (value > 0.0) != (value == 0.0) && value <= 1.0)
    }

    /// IDA: 0054E580 (vt+0x10): an equal time answers false; else stores it. The time is +0x08 (`time_08`).
    pub fn set_time(&mut self, time: f32) -> bool {
        if self.time_08 == time {
            return false;
        }
        self.time_08 = time;
        true
    }

    /// IDA: 0054E630 (vt+0x28) Copy(other): the other's type (vt+0x08), time (vt+0x10), count (vt+0x58) and each value
    /// (vt+0x4C). Completion: 100%.
    pub fn copy_from(&mut self, other: &Self) {
        self.type_04 = other.type_04;
        self.time_08 = other.time_08;
        self.values_0c.clone_from(&other.values_0c);
    }

    /// IDA: 0054EC30 (vt+0x1C) Blend(other, weight, all, out_of_range): nothing (false) unless the types and counts agree;
    /// the weight clamped to [0, 1]; per value: without `all`, an other value within 0.001 of 0.0 (`0047DF40`) is skipped;
    /// an other value outside [0, 1] is skipped, or with `out_of_range` stores FLT_MAX; else a weight of 1.0 or more, or
    /// this value outside [0, 1], takes the other's; else this x (1 - w) + other x w. Answers whether any store changed a
    /// value. Precision: 24-bit (the main thread), each step a float. Completion: 100%.
    pub fn blend(&mut self, other: &Self, weight: f32, all: bool, out_of_range: bool) -> bool {
        if other.type_04 != self.type_04 || other.values_0c.len() != self.values_0c.len() {
            return false;
        }
        let clamped = if weight >= 0.0 { weight.min(1.0) } else { 0.0 };
        let mut changed = false;
        for index in 0..self.values_0c.len() {
            let theirs = other.value(index);
            if !all {
                // 0047DF40(theirs, 0.0, 0.001): |a - b| < 0.001.
                let difference = if 0.0 >= theirs {
                    0.0 - theirs
                } else {
                    theirs - 0.0
                };
                if difference < 0.001 {
                    continue;
                }
            }
            let value = if theirs < 0.0 || theirs > 1.0 {
                if !out_of_range {
                    continue;
                }
                FACE_GEN_UNSET
            } else {
                let ours = self.value(index);
                if weight >= 1.0 || ours < 0.0 || ours > 1.0 {
                    theirs
                } else {
                    ours * (1.0 - clamped) + theirs * clamped
                }
            };
            changed |= self.set_value(index, value);
        }
        changed
    }
}

/// IDA: 0054F630's fill with no zeroing: 0x7F7FFFFF (FLT_MAX), the keyframes' "no value".
pub const FACE_GEN_UNSET: f32 = f32::from_bits(0x7F7F_FFFF);

/// IDA: 0054BA90 BSFaceGenKeyframeList_Advance(time, list, working, out_of_range) (cdecl): an empty list sets the working
/// keyframe's time 0.0 and answers false; a head of another type answers false. Otherwise the working time += `time`;
/// while the head's time is at or below it (ordered), the head blends in whole (vt+0x1C(head, 1.0, 1, out_of_range)), the
/// working time loses the head's, and the head goes; an emptied list sets the working time 0.0. A head beyond blends by
/// the working time over its own (stored as a float). Answers whether a blend changed a value. Precision: 24-bit, each
/// step a float. Completion: 100%.
pub fn face_gen_advance_keyframe_list(
    time: f32,
    list: &mut std::collections::VecDeque<FaceGenKeyframe>,
    working: &mut FaceGenKeyframe,
    out_of_range: bool,
) -> bool {
    let Some(head) = list.front() else {
        working.set_time(0.0);
        return false;
    };
    if head.type_04 != working.type_04 {
        return false;
    }
    let mut changed = false;
    working.set_time(working.time_08 + time);
    while let Some(head) = list.front() {
        // 0054BB46: `fcomp`, C0|C3 both clear (above) or both set (unordered) takes the partial arm.
        if head.time_08.partial_cmp(&working.time_08) != Some(std::cmp::Ordering::Less)
            && head.time_08 != working.time_08
        {
            // 0054BC00: the head partway.
            let ratio = working.time_08 / head.time_08;
            let head = head.clone();
            return working.blend(&head, ratio, true, out_of_range) | changed;
        }
        let head = list.pop_front().expect("checked");
        changed |= working.blend(&head, 1.0, true, out_of_range);
        working.set_time(working.time_08 - head.time_08);
    }
    working.set_time(0.0);
    changed
}

/// IDA: BSFaceGenAnimationData (0x1E0, `0054CAF0`, an NiExtraData): twelve keyframes at +0x10, +0x34, +0x48, +0x6C, +0x90,
/// +0xA4, +0xC8, +0xEC, +0x100, +0x124, +0x148 and +0x15C (types and counts (1, 13) x3, (2, 17) x3, (0, 16) x3, (3, 1) x3),
/// seven empty NiTPointerList<BSFaceGenKeyframe*> at +0x24, +0x5C, +0x80, +0xB8, +0xDC, +0x114 and +0x138, +0x0C = 0, +0x170
/// the zero point, the floats +0x17C..+0x1BC zero (+0x194 a dword), +0x1C0 = 0, +0x1C4/+0x1C8/+0x1DC zero, and the bytes
/// +0x198 = 1, +0x1D5 = 1, the rest of +0x1D4..+0x1DB zero. No draw. Completion: 100% of the stores.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct BsFaceGenAnimationData {
    pub head_track_target_0c: u32,
    pub keyframes: Vec<FaceGenKeyframe>,
    /// The seven NiTPointerList<BSFaceGenKeyframe*> (+0x24, +0x5C, +0x80, +0xB8, +0xDC, +0x114, +0x138), head first.
    pub lists: [std::collections::VecDeque<FaceGenKeyframe>; 7],
    pub look_direction_170: [f32; 3],
    pub floats_17c: [f32; 17],
    pub byte_198: u8,
    pub field_1c0: u32,
    /// +0x1C4, and +0x1C8 the expression timer `0054A120` counts down.
    pub look_direction_time_1c4: f32,
    pub expression_timer_1c8: f32,
    /// +0x1CC and +0x1D0, `0054A120`'s two expression offsets (left as the ctor's heap bytes until its first draw; the
    /// port starts them at 0.0, which only `0054A120`'s sums read).
    pub expression_offsets_1cc: [f32; 2],
    pub bytes_1d4: [u8; 6],
    pub animation_disabled_1da: u8,
    /// Role unresolved. Cleared by the constructor.
    pub byte_1db: u8,
    pub custom_morph_1dc: f32,
}

/// IDA: 0054CAF0 BSFaceGenAnimationData_Constructor. Completion: 100%.
pub fn spawn_face_gen_animation_data(world: &mut World) -> Entity {
    let keyframes = [
        (1, 13),
        (1, 13),
        (1, 13),
        (2, 17),
        (2, 17),
        (2, 17),
        (0, 16),
        (0, 16),
        (0, 16),
        (3, 1),
        (3, 1),
        (3, 1),
    ]
    .into_iter()
    .map(|(kind, count)| FaceGenKeyframe::new(kind, count))
    .collect();
    let mut bytes_1d4 = [0; 6];
    bytes_1d4[1] = 1;
    world
        .spawn(BsFaceGenAnimationData {
            head_track_target_0c: 0,
            keyframes,
            lists: Default::default(),
            look_direction_170: [0.0; 3],
            floats_17c: [0.0; 17],
            byte_198: 1,
            field_1c0: 0,
            look_direction_time_1c4: 0.0,
            expression_timer_1c8: 0.0,
            expression_offsets_1cc: [0.0; 2],
            bytes_1d4,
            animation_disabled_1da: 0,
            byte_1db: 0,
            custom_morph_1dc: 0.0,
        })
        .id()
}

/// IDA: 00549420 BSFaceGenAnimationData_ResetKeyframeChannels. Clear each selected override
/// queue (0054ADE0) before resetting its working keyframe and adding its neutral frame (00549750).
/// Completion: all four channels. Rust owns the queued copies and their value buffers.
pub(in crate::tes) fn reset_face_keyframe_channels(
    data: &mut BsFaceGenAnimationData,
    transition: f32,
    expressions: bool,
    phonemes: bool,
    modifiers: bool,
    custom: bool,
) {
    let channels = [
        (expressions, 0, 1, 1, 13),
        (modifiers, 2, 4, 2, 17),
        (phonemes, 4, 7, 0, 16),
        (custom, 6, 10, 3, 1),
    ];
    for (selected, queue, _, _, _) in channels {
        if selected {
            data.lists[queue].clear();
        }
    }
    for (selected, queue, working, kind, count) in channels {
        if selected {
            data.keyframes[working].set_time(0.0);
            if transition > 0.0 {
                if data.animation_disabled_1da == 0 {
                    let mut frame = FaceGenKeyframe::new(kind, count);
                    frame.set_time(transition);
                    data.lists[queue].push_back(frame);
                }
            } else {
                data.keyframes[working].values_0c.fill(0.0);
            }
        }
    }
}

/// IDA: BSFaceGenNiNode (0x118, `0055CD70` over NiNode, vtable `00A64F5C`): +0xDC the animation data (vt+0x9C/vt+0xA0),
/// +0xE0 a rotation (vt+0xA4/vt+0xA8: 0055CC10 also sets +0x107 and the local rotation), +0x104 bytes (+0x105 vt+0xBC/0xC0,
/// +0x106 vt+0xAC/0xB0, +0x107), +0x108 (vt+0xB4/0xB8), +0x10C -1.0, +0x110/+0x111/+0x112 bytes (+0x111 set for the player's
/// base, form 7), +0x114 the actor (`004353D0`).
#[derive(Component, Clone, Debug, PartialEq)]
pub struct BsFaceGenNiNode {
    pub animation_data_dc: Option<Entity>,
    pub rotation_e0: [f32; 9],
    pub bytes_104: [u8; 4],
    pub byte_108: u8,
    pub float_10c: f32,
    pub bytes_110: [u8; 3],
    pub actor_114: Option<Entity>,
}

/// IDA: 0055CD70 BSFaceGenNiNode_Constructor: NiNode_Constructor(0); +0xDC = 0, +0x10C = -1.0, the dword +0x104 =
/// 0x01010000 (bytes +0x106 and +0x107 = 1), +0x108 = 0, +0x110 = +0x111 = 0, +0xDC released, +0xE0 the identity,
/// +0x112 = 0, +0x114 = 0. Then `0052DED0`'s name, vt+0xA0 the animation data, vt+0xB0 (+0x106) and vt+0xB8 (+0x108) with
/// `biped`. Completion: 100%.
pub fn spawn_face_gen_ni_node(
    world: &mut World,
    name: &[u8],
    animation: Entity,
    biped: bool,
) -> Entity {
    let node = spawn_scene_node(world, SceneNodePayload::default());
    world.get_mut::<NiObjectNet>(node).expect("a NiNode").name = Some(name.to_vec());
    world.entity_mut(node).insert(BsFaceGenNiNode {
        animation_data_dc: Some(animation),
        rotation_e0: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        bytes_104: [0, 0, u8::from(biped), 1],
        byte_108: u8::from(biped),
        float_10c: -1.0,
        bytes_110: [0; 3],
        actor_114: None,
    });
    node
}

/// IDA: N/A a part's base texture path (`00555A80`'s "Textures\%s" of the race's +0x1B8 + 0x0C i path) that the original
/// turns into a NiTexturingProperty through TES_CreateTextureImage (`00442890`) and `NiAVObject_AttachProperty`. Host
/// deferral with D13's textures: the path is kept on the clone, and the view draws from it.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct FaceGenPartTexture(pub Vec<u8>);

/// IDA: the hair's tint, `005551C0`'s FaceGenHair arm stores into its shader property: the NPC's hair colour bytes (red low)
/// over 255.0 and alpha 1.0. run-am shows the property as a Lighting30ShaderProperty (vtable `00A9576C`, type 10, render
/// path 5), so the store is +0xF0..+0xFC (`00555898`); a type-5 BSShaderPPLightingProperty would take +0xA8 (`005557FF`). The
/// SM3 shaders (`SM3002`.. in shaderpackage019) read it as HairTint (c2): the base map times
/// 2 x (0.5 + vertex colour green x (tint - 0.5)). Host substitution: the port holds no shader property, so the geometry
/// carries the four floats and the view draws with them.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct HairTint(pub [f32; 4]);

/// IDA: 005551C0 FaceGen_ApplyFaceTexture(node, description), its FaceGenHair arm (005557A6..00555905): the node's vt+0x58
/// GetObjectByName("FaceGenHair"); with one, 007B8940 BSShaderManager_PrepareObjectGeometry(hair, 1, 1, 1) (the shader
/// property's build, the host's), then its type-4 property gets the description's +0x64 hair colour: each byte through
/// `fild`, `fdiv` 255.0 (00A3DDD8) and `fstp` to a float, alpha `fld1`. Completion: the tint; the rest of 005551C0 (the face
/// texture, the eyes' property flags and 00551140) is D13's.
pub fn apply_hair_tint(world: &mut World, node: Entity, hair_color: u32) -> bool {
    let Some(hair) = crate::bs::model_loader::object_by_name(world, node, b"FaceGenHair") else {
        return false;
    };
    let channel = |shift: u32| (f64::from((hair_color >> shift) & 0xFF) / 255.0) as f32;
    world
        .entity_mut(hair)
        .insert(HairTint([channel(0), channel(8), channel(16), 1.0]));
    true
}

/// IDA: N/A the texture `005551C0` gives a FaceGen part's shader property as FaceGenMap1 (vt+0x84(1, texture): +0xC0[1], the
/// SM3006 shader's s5, which multiplies the colour by twice it): the age map `00551A00` found (part 0; run-au shows
/// HeadHumanM10.dds at Age 0.0 and M60 at 1.0, its `_n` normal map in +0xC0[0]), else None for g_faceGen's grey
/// texture +0xDB8 (`004783A0`).
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct FaceGenMap1(pub Option<Vec<u8>>);

/// IDA: N/A the generated FaceGen texture `005551C0` gives a part's shader property (vt+0x80(1, texture), with +0x1C |= 0x400):
/// `00557E60`/`0054DA40`'s RGBA8 image, the delta the skin shader adds to the base map. Host form: the image on the geometry.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct FaceGenGeneratedTexture(
    pub std::sync::Arc<crate::bs::face_gen_texture::FaceGenTextureImage>,
);

/// IDA: 005551C0 FaceGen_ApplyFaceTexture(node, description), its face and ears arm (the part loop 005552E0..005556EB) for
/// parts 0..6: part 1 (the male's ears) only for sex 0 and part 2 (the female's) only for sex 1; parts 7 and 8 (the eyes)
/// have their own arms. With the part's model and texture, the node's vt+0x58 GetObjectByName(the part's name), its
/// BSFaceGenModel (`005507E0`) and a shader property of type 5..10 (after `007B8940`): the face, the ears (slot kind 14)
/// take the generated texture; the others (kind 1) don't. The NPC's own face textures (+0xA4, with bFaceGenTexturing) aren't
/// built (`fill_face_description`), so each part generates its own (`00557E60`, its EGT). Part 0 with an age byte (`006EDD30`,
/// floored) not negative as an i8 takes `00551A00`'s age map as its FaceGenMap1 (run-au: the base map stays the race's
/// texture), others g_faceGen's grey. Completion: the generated texture and FaceGenMap1; the shader property's stores (the normal
/// map `00434980`, flat normal `g_effectFlatNormalTexture`, `00551140`'s, the material's +0x4C/+0x54 adjust) are the
/// renderer's (goal 6), and FileFinder's FindFile is `services.exists` (the archives, every type).
/// # Errors
/// A null g_faceGen.
pub fn apply_face_texture(
    world: &mut World,
    node: Entity,
    description: &super::face_gen_head::FaceDescription,
    services: &mut crate::bs::face_gen_model::FaceGenFileServices<'_>,
) -> Result<(), GameError> {
    use crate::bs::face_gen_model::{BsFaceGenModelMap, model_generate_face_texture};
    let age = super::face_gen_head::face_gen_age_byte(world, &description.coefficients_00)?;
    for part in 0..7 {
        // 005552EF..00555325: the ears by sex.
        if (part == 2 && description.sex_70 == 0) || (part == 1 && description.sex_70 == 1) {
            continue;
        }
        if description.model_paths_74[part].is_empty()
            || description.texture_paths_84[part].is_empty()
        {
            continue;
        }
        let kind = if part <= 2 { 14 } else { 1 };
        let name = super::face_gen_head::PART_NAMES_B10CA8[part].as_bytes();
        let Some(object) = crate::bs::model_loader::object_by_name(world, node, name) else {
            continue;
        };
        let Some(model) = world
            .get::<FaceGenPartMorph>(object)
            .map(|morph| morph.model)
        else {
            continue;
        };
        // 0055541C..00555461: part 0 with a non-negative age byte looks for the age's base map.
        let age_path = if part == 0 && (age as i8) >= 0 {
            let texture = description.texture_paths_84[0].clone();
            crate::bs::face_gen_texture::find_age_texture(
                description.sex_70,
                age,
                &texture,
                services.exists,
            )
        } else {
            None
        };
        // 00555467..00555525: no own texture, so 00557E60 under g_faceGenModelMapLock.
        let mut held = world.resource::<BsFaceGenModelMap>().models[model].clone();
        let generated =
            model_generate_face_texture(&mut held, &description.coefficients_00, services);
        world.resource_mut::<BsFaceGenModelMap>().models[model] = held;
        // 005555EE..00555651: kind 14 takes the generated texture (+0xBC[1]) and FaceGenMap1 (+0xC0[1]); the base map
        // (+0xBC[0]) stays the part's texture.
        if kind == 14 {
            world
                .entity_mut(object)
                .insert(FaceGenMap1(age_path.map(String::into_bytes)));
            match generated {
                Some(image) => {
                    world
                        .entity_mut(object)
                        .insert(FaceGenGeneratedTexture(std::sync::Arc::new(image)));
                }
                None => {
                    world.entity_mut(object).remove::<FaceGenGeneratedTexture>();
                }
            }
        }
    }
    Ok(())
}

/// IDA: N/A the entity standing for g_faceGen's NiAlphaProperty (`00550550`), made once from `FaceGenSingleton`'s values.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceGenAlphaPropertyEntity(pub Entity);

// IDA: 00550550: g_faceGen's alpha property.
fn face_gen_alpha_property(world: &mut World) -> Result<Entity, GameError> {
    if let Some(held) = world.get_resource::<FaceGenAlphaPropertyEntity>() {
        return Ok(held.0);
    }
    let alpha = world
        .get_resource::<crate::tes::facegen::FaceGenSingleton>()
        .and_then(|singleton| singleton.0.as_deref().map(|state| state.alpha))
        .ok_or(GameError::unclassified(
            0x00550550,
            "00550550: a null g_faceGen",
        ))?;
    let entity = world
        .spawn((
            NiClass::AlphaProperty,
            AlphaProperty {
                flags_18: alpha.flags,
                alpha_test_ref_1a: alpha.test_reference,
            },
            NiObjectNet::default(),
        ))
        .id();
    world.insert_resource(FaceGenAlphaPropertyEntity(entity));
    Ok(entity)
}

/// IDA: 00556A70 BSFaceGenModel_CloneGeometry with 00559870's coefficients applied: `00700900` clones the model's geometry
/// (CreateClone then ProcessClone with a fresh cloning process); `00700790` deep-copies its data through a stream (a new
/// data object), the clone's vt+0x8C takes it; with a skin instance, its partition (+0x0C) is deep-copied the same way and
/// set (`00478300`). Then the base-morph extra data (`BSFaceGenBaseMorphExtraData`) and the model's extra (+0x20) are added
/// (not held). The data's positions are the part's morphed ones (`BSFaceGenModel_ApplyCoefficients`). Completion: the
/// geometry, its data and the partition; the two extras aren't held.
fn clone_part_geometry(
    world: &mut World,
    geometry: Entity,
    positions: &[[f32; 3]],
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<Entity, GameError> {
    let mut process = CloningProcess::default();
    let clone = clone_object(world, &mut process, geometry, prepare)?;
    let data = world
        .get::<NiGeometry>(clone)
        .and_then(|links| links.data_b4)
        .ok_or(GameError::unclassified(
            0x00556AC0,
            "00556AC0: a FaceGen geometry with no data",
        ))?;
    let copy = world.entity_mut(data).clone_and_spawn();
    if let Some(mut strips) = world.get_mut::<MeshTriStripsData>(copy) {
        strips.base.positions_1c = positions.to_vec();
    } else if let Some(mut shape) = world.get_mut::<MeshTriShapeData>(copy) {
        shape.base.positions_1c = positions.to_vec();
    } else {
        return Err(GameError::unclassified(
            0x00556B0E,
            "00556B0E: FaceGen data that is neither strips nor a shape",
        ));
    }
    world.get_mut::<NiGeometry>(clone).expect("cloned").data_b4 = Some(copy);
    if let Some(skin) = world
        .get::<NiGeometry>(clone)
        .and_then(|links| links.skin_b8)
        && let Some(partition) = world
            .get::<MeshSkinInstance>(skin)
            .and_then(|instance| instance.partition_0c)
    {
        let partition = world.entity_mut(partition).clone_and_spawn();
        world
            .get_mut::<MeshSkinInstance>(skin)
            .expect("a skin instance")
            .partition_0c = Some(partition);
    }
    Ok(clone)
}

/// IDA: one part of 00555A80 after its model: 00559870's clone (`clone_part_geometry`) named from 00B10CA8; its base texture
/// (`FaceGenPartTexture`); for the teeth (parts 4 and 5) the alpha property (type 0) replaced by g_faceGen's; for parts 0..2
/// the material property (type 2) named "skin", or a new NiMaterialProperty "skin" attached. A clone with a skin instance
/// (+0xB8) goes under the skinned node (vt+0x84, 0); any other takes the -90 degree Y rotation (0070FD80) as its local
/// rotation and goes under the biped node. Completion: 100% but the texturing property (host, D13).
/// # Errors
/// A stop in the clone or a property.
#[allow(clippy::too_many_arguments, reason = "00555A80's loop state")]
pub fn add_head_part(
    world: &mut World,
    biped: Entity,
    skinned: Entity,
    index: usize,
    name: &str,
    geometry: Entity,
    positions: &[[f32; 3]],
    texture: &[u8],
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<Entity, GameError> {
    let clone = clone_part_geometry(world, geometry, positions, prepare)?;
    world
        .get_mut::<NiObjectNet>(clone)
        .expect("a geometry")
        .name = Some(name.as_bytes().to_vec());
    let mut path = b"Textures\\".to_vec();
    path.extend_from_slice(texture);
    world.entity_mut(clone).insert(FaceGenPartTexture(path));
    if index == 4 || index == 5 {
        detach_first_property_of_type(world, clone, 0)?;
        let alpha = face_gen_alpha_property(world)?;
        crate::gamebryo::scene_state::attach_object_property(world, clone, Some(alpha))?;
    }
    if index <= 2 {
        match property_of_type(world, clone, 2)? {
            Some(material) => {
                world
                    .get_mut::<NiObjectNet>(material)
                    .ok_or(GameError::unclassified(
                        0x00555FF0,
                        "00555FF0: a material with no name slot",
                    ))?
                    .name = Some(b"skin".to_vec());
            }
            None => {
                let material = crate::gamebryo::properties::new_material_property(world);
                let material = world
                    .spawn((
                        NiClass::MaterialProperty,
                        material,
                        NiObjectNet {
                            name: Some(b"skin".to_vec()),
                            ..NiObjectNet::default()
                        },
                    ))
                    .id();
                crate::gamebryo::scene_state::attach_object_property(world, clone, Some(material))?;
            }
        }
    }
    place_under_head_node(world, biped, skinned, clone)?;
    Ok(clone)
}

/// IDA: TESNPC +0x1D4 and +0x1D8, the head nodes `00528A10` stores (NiRefObjectPtr_Assign).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FaceGenHeadNodes {
    pub biped_1d4: Option<Entity>,
    pub skinned_1d8: Option<Entity>,
}

/// IDA: 0055D090 BSFaceGenNiNode_FixSkinInstances (vt+0xC4)(node, root, warn): each child slot below +0xB6 that is a
/// geometry (vt+0x10) with a skin instance (+0xB8) holding data (+0x08) and bones (+0x14): each of the data's +0x40 bones is
/// looked up by its name under `root` (vt+0x58 GetObjectByName) and replaces the entry when found, else with `warn`,
/// "Could not find bone "%s" for the head node."; then the instance's +0x10 root parent is `node`. Completion: 100%.
pub fn fix_skin_instances(world: &mut World, node: Entity, root: Entity, warn: bool) {
    use crate::gamebryo::scene_node::NiNodeChildren;
    let children = world
        .get::<NiNodeChildren>(node)
        .map(|children| children.slots_ac.clone())
        .unwrap_or_default();
    for child in children.into_iter().flatten() {
        let Some(skin) = world
            .get::<NiGeometry>(child)
            .and_then(|links| links.skin_b8)
        else {
            continue;
        };
        let Some(instance) = world.get::<MeshSkinInstance>(skin).cloned() else {
            continue;
        };
        if instance.data_08.is_none() || instance.bone_owners_14.is_empty() {
            continue;
        }
        let mut bones = instance.bone_owners_14.clone();
        for bone in &mut bones {
            let Some(name) = bone
                .and_then(|bone| world.get::<NiObjectNet>(bone))
                .and_then(|net| net.name.clone())
            else {
                continue;
            };
            match crate::bs::model_loader::object_by_name(world, root, &name) {
                Some(found) => *bone = Some(found),
                None if warn => oblivion_core::logging::emit(
                    oblivion_core::logging::Level::Warning,
                    &format!(
                        "Could not find bone \"{}\" for the head node.",
                        String::from_utf8_lossy(&name)
                    ),
                ),
                None => {}
            }
        }
        let mut held = world.get_mut::<MeshSkinInstance>(skin).expect("read above");
        held.bone_owners_14 = bones;
        held.root_parent_owner_10 = Some(node);
    }
}

/// IDA: 00524510 TESNPC_UpdateFaceGenNodeFlags(actor, null): under the actor's +0x3C, the two FaceGen nodes, "FaceGenHair" and
/// the ears (00B10CAC, then 00B10CB0) by name with the palette (006F94A0); with both nodes, an item in the hair slot (1)
/// culls the ears and hair (+0x18 |= 1), else clears it; an item in the head slot (0) culls both nodes, else clears it.
/// Completion: 100%.
/// # Errors
/// A stop in the worn-item lookup.
pub(crate) fn update_face_gen_node_flags(
    world: &mut World,
    actor: Entity,
) -> Result<(), GameError> {
    let Some(root) = world
        .get::<crate::tes::reference_3d::Reference3D>(actor)
        .map(|loaded| loaded.0)
    else {
        return Ok(());
    };
    let find = |world: &World, name: &[u8]| {
        crate::gamebryo::object_net::object_by_name_with_palette(world, root, name)
    };
    let (Some(biped), Some(skinned)) = (
        find(world, b"BSFaceGenNiNodeBiped"),
        find(world, b"BSFaceGenNiNodeSkinned"),
    ) else {
        return Ok(());
    };
    let hair = find(world, b"FaceGenHair");
    let ears = find(world, b"FaceGenEars");
    let set = |world: &mut World, object: Option<Entity>, cull: bool| {
        let Some(object) = object else {
            return;
        };
        if let Some(mut node) = world.get_mut::<SceneNodePayload>(object) {
            if cull {
                node.effective_av_flags |= 1
            } else {
                node.effective_av_flags &= !1
            }
        } else if let Some(mut geometry) = world.get_mut::<MeshTriShapePayload>(object) {
            if cull {
                geometry.effective_av_flags |= 1
            } else {
                geometry.effective_av_flags &= !1
            }
        }
    };
    let hair_slot = crate::tes::equipment::equipped_in_slot(world, actor, 1)?.is_some();
    set(world, ears, hair_slot);
    set(world, hair, hair_slot);
    let head_slot = crate::tes::equipment::equipped_in_slot(world, actor, 0)?.is_some();
    set(world, Some(biped), head_slot);
    set(world, Some(skinned), head_slot);
    Ok(())
}

/// IDA: 00528A10 TESNPC_AttachFaceGenHead(npc, actor, skin, biped, skinned): the actor's +0x3C as a node (vt+8), the skin
/// info's bone 0 ("Bip01 Head", 00478220), and the palette (vt+0x164's animation data +0x98 manager's +0x7C). With both:
/// the biped node (an animation data with a dead actor is a stop) takes vt+0xB0(1) and vt+0xB8(1), a zero translate and
/// the identity rotation, goes under the bone (vt+0x84, 1) and into the palette (007165B0). The skinned node: its first
/// child's texturing property would run FaceGen_ApplyFaceTexture (D13: the parts keep their path, so no property is there);
/// vt+0xB0 and vt+0xB8 with `biped` null (0 here), a zero translate, vt+0xA8 (0055CC10: +0xE0 and the local rotation, +0x107
/// = 1) with 004D7C50(identity, inverse); under the root (vt+0x84, 1), into the palette, then vt+0xC4 `fix_skin_instances`
/// (root, 1). The animation data's vt+0x78(0.0, 1, 1, 1, 1, 0) resets its keyframes (state only, no draw: not held). The
/// NPC's +0x1D4/+0x1D8, RefreshEffectState and RefreshPropertyState of the root and NiAVObject_Update(root, 0.0, 0). Without
/// a bone or root, "Cannot create a head for an NPC (%d) that does not have a biped-head node." Then 00524510. Completion:
/// 100% of a living actor's arms.
/// # Errors
/// The dead actor's arm, or a stop in the update.
pub fn attach_face_gen_head(
    world: &mut World,
    npc: Entity,
    actor: Entity,
    biped: Option<Entity>,
    skinned: Option<Entity>,
    defaults: &crate::gamebryo::scene_state::ScenePropertyDefaults,
) -> Result<(), GameError> {
    use crate::gamebryo::scene_node::NiNodeChildren;
    let root = world
        .get::<crate::tes::reference_3d::Reference3D>(actor)
        .map(|loaded| loaded.0)
        .filter(|&node| world.get::<NiNodeChildren>(node).is_some());
    let bone = crate::tes::character_3d::skin_info(world, actor)
        .and_then(|skin| world.get::<crate::tes::character_3d::SkinInfo>(skin))
        .and_then(|info| info.bones_04[0].node_04);
    let palette = crate::tes::actor::process_of(world, actor)
        .and_then(|process| world.get::<crate::tes::actor_process::MiddleHighProcess>(process))
        .and_then(|middle| middle.anim_data_17c)
        .and_then(|anim| world.get::<crate::tes::actor_animation::ActorAnimData>(anim))
        .and_then(|data| data.manager_98)
        .and_then(|manager| {
            world.get::<crate::gamebryo::controller_sequence::ControllerManager>(manager)
        })
        .and_then(|manager| manager.object_palette_7c);
    let (Some(bone), Some(root)) = (bone, root) else {
        let id = world
            .get::<crate::tes::form::FormIdentity>(npc)
            .expect("a form")
            .form_id_0c;
        oblivion_core::logging::emit(
            oblivion_core::logging::Level::Warning,
            &format!(
                "Cannot create a head for an NPC ({id}) that does not have a biped-head node."
            ),
        );
        return update_face_gen_node_flags(world, actor);
    };
    place_head_nodes(world, actor, bone, root, palette, biped, skinned)?;
    world.entity_mut(npc).insert(FaceGenHeadNodes {
        biped_1d4: biped,
        skinned_1d8: skinned,
    });
    crate::gamebryo::scene_state::refresh_effect_state(world, root)?;
    crate::gamebryo::scene_state::refresh_property_state(world, root, defaults)?;
    let result = crate::gamebryo::scene_update::update_scene_object(
        world,
        root,
        0.0,
        false,
        &mut crate::tes::reference_3d::LoadUpdateControllers,
    )?;
    if result.status != crate::gamebryo::scene_update::SceneUpdateStatus::Updated {
        return Err(GameError::unclassified(
            0x00528D47,
            "00528D47: the head's NiAVObject_Update stopped",
        ));
    }
    update_face_gen_node_flags(world, actor)
}

/// IDA: the two head-node arms `00528A10` and `00525A70` share: a biped node (a dead actor's animation data a stop) takes
/// vt+0xB0(1) and vt+0xB8(1), a zero translate and the identity, goes under the head bone (vt+0x84, 1) and into the palette;
/// a skinned node takes vt+0xB0/vt+0xB8 with `biped` null, a zero translate, vt+0xA8 with the race-scaled identity (004D7C50,
/// the inverse), goes under the root (vt+0x84, 1), into the palette, then vt+0xC4 `fix_skin_instances`(root, 1). Completion:
/// 100% of a living actor's arms.
/// # Errors
/// The dead actor's arm, or a failed attach.
fn place_head_nodes(
    world: &mut World,
    actor: Entity,
    bone: Entity,
    root: Entity,
    palette: Option<Entity>,
    biped: Option<Entity>,
    skinned: Option<Entity>,
) -> Result<(), GameError> {
    let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    if let Some(biped) = biped {
        if f64::from(crate::tes::actor::actor_get_actor_value(world, actor, 8)?) <= 0.0 {
            return Err(GameError::unclassified(
                0x00528AE3,
                "00528AE3: a dead actor's FaceGen animation data",
            ));
        }
        let mut node = world
            .get_mut::<BsFaceGenNiNode>(biped)
            .expect("a BSFaceGenNiNode");
        node.bytes_104[2] = 1;
        node.byte_108 = 1;
        let mut payload = world.get_mut::<SceneNodePayload>(biped).expect("a NiNode");
        payload.local_transform.translation = [0.0; 3];
        payload.local_transform.rotation = identity;
        if attach_scene_child(world, bone, Some(biped), true) != AttachChildStatus::Attached {
            return Err(GameError::unclassified(
                0x00528B86,
                "00528B86: attaching the biped head node",
            ));
        }
        crate::gamebryo::av_object_palette::add_scene_objects(world, Some(biped), palette);
    }
    if let Some(skinned) = skinned {
        let rotation =
            crate::tes::biped_attach::race_scaled_rotation(world, actor, &identity, true)?;
        let mut node = world
            .get_mut::<BsFaceGenNiNode>(skinned)
            .expect("a BSFaceGenNiNode");
        node.bytes_104[2] = u8::from(biped.is_none());
        node.byte_108 = u8::from(biped.is_none());
        node.rotation_e0 = rotation;
        node.bytes_104[3] = 1;
        let mut payload = world
            .get_mut::<SceneNodePayload>(skinned)
            .expect("a NiNode");
        payload.local_transform.translation = [0.0; 3];
        payload.local_transform.rotation = rotation;
        if attach_scene_child(world, root, Some(skinned), true) != AttachChildStatus::Attached {
            return Err(GameError::unclassified(
                0x00528CA6,
                "00528CA6: attaching the skinned head node",
            ));
        }
        crate::gamebryo::av_object_palette::add_scene_objects(world, Some(skinned), palette);
        fix_skin_instances(world, skinned, root, true);
    }
    Ok(())
}

/// IDA: 00525A70 TESNPC_CreateHead(npc, actor, skin), the head built on the calling thread: with g_faceGen, the skin's +0x00
/// and bUseFaceGenHeads (00B120B4), and not the player's first-person skin info: the root (+0x3C vt+0x08), the skin's head
/// bone (`SkinInfo_GetBoneNode(0)`) and the animation data's palette (+0x98 +0x7C). With both: a FaceGen node already there
/// (vt+0x130, vt+0x134) skips to the update; otherwise with no cached head (+0x1D4, +0x1D8) and a race, `0052DED0`(race, ...,
/// npc, 0, 0) (no `005551C0`), +0x1E0 = race +0x2FC and +0x1D4/+0x1D8 the new nodes; the biped node (a dead actor's arm a
/// stop) under the bone with the identity, the skinned node under the root with the race-scaled identity and vt+0xC4, both
/// into the palette and their +0x114 = the actor (`place_head_nodes`); then NiAVObject_Update(root, 0.0, 0). No race warns
/// "Cannot create a head for an NPC (%d) (no race or bad race data)."; no bone or root "... that does not have a biped-head
/// node.". Completion: 100% of the path with no cached head; a cached head's clone arms (`00478C80`, NiObject_Clone, the skin
/// partitions) are a stop. The animation data's vt+0x78 keyframe reset isn't held, as in `attach_face_gen_head`.
/// # Errors
/// A cached head, the dead actor's arm, or a stop in the build or the update.
pub fn create_head(
    world: &mut World,
    npc: Entity,
    actor: Entity,
    skin: Option<Entity>,
    services: &mut crate::bs::model_loader::ModelBuildServices<'_>,
) -> Result<(), GameError> {
    use crate::gamebryo::scene_node::NiNodeChildren;
    let face_gen = world
        .get_resource::<crate::tes::facegen::FaceGenSingleton>()
        .is_some_and(|singleton| singleton.0.is_some());
    let use_heads = world
        .get_resource_or_init::<crate::tes::character_3d::FaceGenSettings>()
        .use_face_gen_heads_b120b4;
    let info = world
        .get::<crate::tes::character_3d::SkinInfo>(skin.unwrap_or(actor))
        .filter(|_| skin.is_some())
        .cloned();
    let Some(info) = info.filter(|info| face_gen && info.bip01_00.is_some() && use_heads) else {
        return Ok(());
    };
    if Some(actor) == crate::tes::player::the_player(world)
        && world
            .get::<crate::tes::player::PlayerCharacter>(actor)
            .is_some_and(|player| player.first_person_skin_info_5c8 == skin)
    {
        return Ok(());
    }
    let root = world
        .get::<crate::tes::reference_3d::Reference3D>(actor)
        .map(|loaded| loaded.0)
        .filter(|&node| world.get::<NiNodeChildren>(node).is_some());
    let bone = info.bones_04[0].node_04;
    let palette = crate::tes::actor_movement_update::player_anim_data(world, actor)
        .filter(|_| Some(actor) == crate::tes::player::the_player(world))
        .or_else(|| {
            crate::tes::actor::process_of(world, actor)
                .and_then(|process| {
                    world.get::<crate::tes::actor_process::MiddleHighProcess>(process)
                })
                .and_then(|middle| middle.anim_data_17c)
        })
        .and_then(|anim| world.get::<crate::tes::actor_animation::ActorAnimData>(anim))
        .and_then(|data| data.manager_98)
        .and_then(|manager| {
            world.get::<crate::gamebryo::controller_sequence::ControllerManager>(manager)
        })
        .and_then(|manager| manager.object_palette_7c);
    let id = world
        .get::<crate::tes::form::FormIdentity>(npc)
        .expect("a form")
        .form_id_0c;
    let (Some(bone), Some(root)) = (bone, root) else {
        oblivion_core::logging::emit(
            oblivion_core::logging::Level::Warning,
            &format!(
                "Cannot create a head for an NPC ({id}) that does not have a biped-head node."
            ),
        );
        return Ok(());
    };
    let face_node = |world: &World, name: &[u8]| {
        crate::gamebryo::object_net::object_by_name_with_palette(world, root, name)
    };
    if face_node(world, b"BSFaceGenNiNodeBiped").is_none()
        && face_node(world, b"BSFaceGenNiNodeSkinned").is_none()
    {
        let cached = world
            .get::<FaceGenHeadNodes>(npc)
            .copied()
            .unwrap_or_default();
        if cached.biped_1d4.is_some() || cached.skinned_1d8.is_some() {
            return Err(GameError::unclassified(
                0x00525BAD,
                "00525BAD: TESNPC_CreateHead with a cached head (its clone arms)",
            ));
        }
        let mut built = None;
        if let Some(race) = crate::tes::forms_actors::base_race(world, npc)? {
            super::face_gen_tasks::with_face_gen_files(
                world,
                true,
                services,
                &mut |world, files, prepare| {
                    built = Some(super::face_gen_tasks::build_face_gen_head(
                        world, npc, false, files, prepare,
                    )?);
                    Ok(())
                },
            )?;
            let fnam = world
                .get::<crate::tes::forms_actors::TesRace>(race)
                .ok_or(GameError::unclassified(
                    0x00525F4F,
                    "00525F4F: a race that isn't a TESRace",
                ))?
                .snam_2fc;
            world
                .get_mut::<crate::tes::forms_actors::TesNpc>(npc)
                .ok_or(GameError::unclassified(
                    0x00525F59,
                    "00525F59: a base that isn't an NPC",
                ))?
                .fnam_1e0 = fnam;
        }
        let (biped, skinned) =
            built.map_or((None, None), |head| (Some(head.biped), Some(head.skinned)));
        world.entity_mut(npc).insert(FaceGenHeadNodes {
            biped_1d4: biped,
            skinned_1d8: skinned,
        });
        if biped.is_none() && skinned.is_none() {
            oblivion_core::logging::emit(
                oblivion_core::logging::Level::Warning,
                &format!("Cannot create a head for an NPC ({id}) (no race or bad race data)."),
            );
        }
        place_head_nodes(world, actor, bone, root, palette, biped, skinned)?;
        for node in [biped, skinned].into_iter().flatten() {
            world
                .get_mut::<BsFaceGenNiNode>(node)
                .expect("a BSFaceGenNiNode")
                .actor_114 = Some(actor);
        }
    }
    let result = crate::gamebryo::scene_update::update_scene_object(
        world,
        root,
        0.0,
        false,
        &mut crate::tes::reference_3d::LoadUpdateControllers,
    )?;
    if result.status != crate::gamebryo::scene_update::SceneUpdateStatus::Updated {
        return Err(GameError::unclassified(
            0x005261D9,
            "005261D9: the head's NiAVObject_Update stopped",
        ));
    }
    Ok(())
}

/// IDA: 004353D0 QueuedHead_FinishForActor(head, actor, skin): `00528A10` with the actor's base and the head's +0x24/+0x28;
/// a nonzero base vt+0x128(0x45), vampirism, takes 00529530 (a stop); for anyone but the player, 00481410 on both nodes
/// writes the reference's ID into each geometry's shader property (+0x98, which the port's shaders don't hold); the
/// skinned node's vt+0x50 (NiNode_UpdateModelBound); both nodes' +0x114 = the actor. Completion: 100% but 00481410's field.
/// # Errors
/// A stop in the attach or vampirism.
pub fn queued_head_finish_for_actor(
    world: &mut World,
    head: crate::bs::model_loader::IoTaskId,
    actor: Entity,
    defaults: &crate::gamebryo::scene_state::ScenePropertyDefaults,
) -> Result<(), GameError> {
    let Some(task) = world
        .get_resource::<super::face_gen_tasks::FaceGenTasks>()
        .and_then(|tasks| tasks.heads.get(&head))
        .cloned()
    else {
        return Ok(());
    };
    let base = world
        .get::<crate::tes::actor::TesObjectRefr>(actor)
        .and_then(|refr| refr.base_form_1c)
        .ok_or(GameError::unclassified(
            0x004353D9,
            "004353D9: an actor with no base",
        ))?;
    attach_face_gen_head(world, base, actor, task.biped_24, task.skinned_28, defaults)?;
    if crate::tes::actor_values::base_actor_value(world, base, 0x45)? != 0 {
        return Err(GameError::unclassified(
            0x004353F5,
            "004353F5: 00529530, a vampire's FaceGen",
        ));
    }
    if let Some(skinned) = task.skinned_28 {
        crate::gamebryo::scene_update::update_scene_node_model_bound(world, skinned);
    }
    for node in [task.skinned_28, task.biped_24].into_iter().flatten() {
        world
            .get_mut::<BsFaceGenNiNode>(node)
            .expect("a BSFaceGenNiNode")
            .actor_114 = Some(actor);
    }
    Ok(())
}

/// IDA: 0055A3B0 BSFaceGenMorphDifferential vt+4(positions, count, 0, weight), through 0055AF70 BSFaceGenMorphDataHair vt+0x18:
/// the hair TRI's first differential morph, when it is named "HairMorph" (0055C3F0's `_stricmp`), one delta per base vertex;
/// with 0.0 < weight <= 1.0 each of the first min(count, deltas) positions takes each delta component times the weight,
/// stored as a float, then added and stored. Completion: 100%.
fn apply_hair_morph(
    positions: &mut [[f32; 3]],
    morph: Option<&crate::bs::face_gen_model::FaceGenMorphData>,
    weight: f32,
) {
    let Some(crate::bs::face_gen_model::FaceGenMorphData::Hair(tri)) = morph else {
        return;
    };
    let Some(first) = tri.differential_morphs_80.first() else {
        return;
    };
    if oblivion_core::crt::stricmp(&first.name_00, b"HairMorph") != 0 {
        return;
    }
    if !(weight > 0.0 && weight <= 1.0) {
        return;
    }
    for (position, delta) in positions.iter_mut().zip(&first.deltas_1c) {
        for axis in 0..3 {
            let scaled = delta[axis] * weight;
            position[axis] += scaled;
        }
    }
}

/// IDA: N/A the host form of a FaceGen part clone's extra data (`00559870`'s): its BSFaceGenModelExtraData (00550790 finds it:
/// the model, here its index in `BsFaceGenModelMap`) and its BSFaceGenBaseMorphExtraData (`005508A0`; the base after the
/// clone's morph). Host substitution: the port's FaceGen clones carry no extra data objects.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct FaceGenPartMorph {
    pub model: usize,
    pub base: crate::bs::face_gen_model::FaceGenBaseMorph,
}

/// IDA: N/A the hair clone's BSFaceGenMorphDataHair (FaceGen_GetHairMorphData's answer, the TRI), host form.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct FaceGenHairMorphState {
    pub morph: Option<crate::bs::face_gen_model::FaceGenMorphData>,
}

/// IDA: 00550980 FaceGen_ApplyHairLength(geometry, length): a null geometry or a length below 0.0 returns; with the hair
/// morph data (FaceGen_GetHairMorphData) and the data locked for writing (00728AB0(+0xB4, 1)), the positions take the base
/// morph's vertices (FaceGen_CopyBaseMorphToGeometry) and the morph data's vt+0x18 (`apply_hair_morph`) at `length`, then
/// 00728B20 unlocks. Completion: 100%, over `FaceGenHairMorphState`.
pub fn apply_hair_length(world: &mut World, geometry: Entity, length: f32) {
    if length < 0.0 {
        return;
    }
    let Some(state) = world.get::<FaceGenHairMorphState>(geometry).cloned() else {
        return;
    };
    if state.morph.is_none() {
        return;
    }
    let Some(data) = world
        .get::<NiGeometry>(geometry)
        .and_then(|links| links.data_b4)
    else {
        return;
    };
    // FaceGen_CopyBaseMorphToGeometry: the base's first +0x14 vertices; with no base (005508A0 null) the positions stay.
    let mut positions =
        crate::bs::face_gen_model::geometry_positions(world, geometry).unwrap_or_default();
    if let Some(base) = world
        .get::<FaceGenPartMorph>(geometry)
        .map(|part| &part.base)
    {
        let count = (base.data_count_14 as usize)
            .min(base.vertices_0c.len())
            .min(positions.len());
        positions[..count].copy_from_slice(&base.vertices_0c[..count]);
    }
    apply_hair_morph(&mut positions, state.morph.as_ref(), length);
    if let Some(mut strips) = world.get_mut::<MeshTriStripsData>(data) {
        strips.base.positions_1c = positions;
    } else if let Some(mut shape) = world.get_mut::<MeshTriShapeData>(data) {
        shape.base.positions_1c = positions;
    }
}

/// IDA: 00554260's clone of the hair: 00559870's clone with the morphed positions, "FaceGenHair", 00550980(clone, length):
/// with length >= 0.0, the base-morph positions (the morphed ones) through the hair morph (`apply_hair_morph`); the texture
/// path (the NiTexturingProperty with clamp 3 and filter 2 is D13's); g_faceGen's alpha property attached when the clone has
/// no alpha property (type 0); a skinned clone under the skinned node, else the -90 degree Y rotation and the biped node.
/// Completion: 100% but the texturing property.
/// # Errors
/// A stop in the clone or the attach.
#[allow(clippy::too_many_arguments, reason = "00554260's state")]
pub fn add_hair(
    world: &mut World,
    biped: Entity,
    skinned: Entity,
    geometry: Entity,
    positions: &[[f32; 3]],
    morph: Option<&crate::bs::face_gen_model::FaceGenMorphData>,
    length: f32,
    texture: &[u8],
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<Entity, GameError> {
    let clone = make_hair_clone(world, geometry, positions, morph, length, texture, prepare)?;
    place_under_head_node(world, biped, skinned, clone)?;
    Ok(clone)
}

/// IDA: the hair clone `00554260` and `005C50A0`'s rebuild arm share: 00559870's clone with the morphed positions,
/// "FaceGenHair", 00550980(clone, length) (with length >= 0.0, the morphed positions through the hair morph), the texture path
/// (the NiTexturingProperty with clamp 3 and filter 2 is D13's), and g_faceGen's alpha property when it has none (type 0).
/// Completion: 100% but the texturing property.
/// # Errors
/// A stop in the clone.
pub fn make_hair_clone(
    world: &mut World,
    geometry: Entity,
    positions: &[[f32; 3]],
    morph: Option<&crate::bs::face_gen_model::FaceGenMorphData>,
    length: f32,
    texture: &[u8],
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<Entity, GameError> {
    let mut moved = positions.to_vec();
    if length >= 0.0 {
        apply_hair_morph(&mut moved, morph, length);
    }
    let clone = clone_part_geometry(world, geometry, &moved, prepare)?;
    world
        .get_mut::<NiObjectNet>(clone)
        .expect("a geometry")
        .name = Some(b"FaceGenHair".to_vec());
    world.entity_mut(clone).insert(FaceGenHairMorphState {
        morph: morph.cloned(),
    });
    let mut path = b"Textures\\".to_vec();
    path.extend_from_slice(texture);
    world.entity_mut(clone).insert(FaceGenPartTexture(path));
    if property_of_type(world, clone, 0)?.is_none() {
        let alpha = face_gen_alpha_property(world)?;
        crate::gamebryo::scene_state::attach_object_property(world, clone, Some(alpha))?;
    }
    Ok(clone)
}

// IDA: 00555A80's and 00554260's placement: a clone with a skin instance (+0xB8) under the skinned node (vt+0x84, 0); any other
// takes 0070FD80's -90 degree Y rotation and goes under the biped node.
fn place_under_head_node(
    world: &mut World,
    biped: Entity,
    skinned: Entity,
    clone: Entity,
) -> Result<(), GameError> {
    let skin = world
        .get::<NiGeometry>(clone)
        .and_then(|links| links.skin_b8);
    let parent = if skin.is_some() {
        skinned
    } else {
        let angle = f64::from(-1.570_796_4_f32);
        let (c, s) = (angle.cos() as f32, angle.sin() as f32);
        world
            .get_mut::<MeshTriShapePayload>(clone)
            .ok_or(GameError::unclassified(
                0x00556088,
                "00556088: a FaceGen part that isn't a geometry",
            ))?
            .local_transform
            .rotation = [c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c];
        biped
    };
    if attach_scene_child(world, parent, Some(clone), false) != AttachChildStatus::Attached {
        return Err(GameError::unclassified(
            0x00556097,
            "00556097: attaching a FaceGen part",
        ));
    }
    Ok(())
}

/// IDA: 005547F0's clone of an eye: 00559870's clone with the morphed positions, its name, its texture path (the texturing
/// property is D13's), and the placement (`place_under_head_node`). Completion: 100% but the texturing property.
/// # Errors
/// A stop in the clone or the attach.
#[allow(clippy::too_many_arguments, reason = "005547F0's state")]
pub fn add_eye(
    world: &mut World,
    biped: Entity,
    skinned: Entity,
    name: &str,
    geometry: Entity,
    positions: &[[f32; 3]],
    texture: &[u8],
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<Entity, GameError> {
    let clone = clone_part_geometry(world, geometry, positions, prepare)?;
    world
        .get_mut::<NiObjectNet>(clone)
        .expect("a geometry")
        .name = Some(name.as_bytes().to_vec());
    world
        .entity_mut(clone)
        .insert(FaceGenPartTexture(texture.to_vec()));
    place_under_head_node(world, biped, skinned, clone)?;
    Ok(clone)
}

/// IDA: N/A the FaceGen update's calls the port counts rather than makes (the keyframe schedules and the head's matrices).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct FaceGenUpdateBoundaries {
    pub calls: std::collections::BTreeMap<u32, u32>,
}

// IDA: N/A a helper, with no address of its own.
fn count_face(world: &mut World, address: u32) {
    *world
        .get_resource_or_init::<FaceGenUpdateBoundaries>()
        .calls
        .entry(address)
        .or_default() += 1;
}

// IDA: N/A an INI byte setting's value.
fn ini_byte(world: &World, address: u32) -> u8 {
    world
        .get_resource::<crate::tes::settings::SettingCollections>()
        .and_then(|settings| settings.ini.find_address(address))
        .map_or(0, |setting| setting.raw as u8)
}

/// IDA: 0054A120 BSFaceGenAnimationData_UpdateExpressionTimer(expression, a, b, time): +0x1C8 loses the time; at or below 0
/// the expression's arm draws a new hold and offsets (-1, 0, 5..8: Random_Chance(0.3) holds 2..3 with zero offsets, else
/// 0.5..1.5, +0x1D0 in -0.05..0.15 and +0x1CC in -0.1..0.1; 1: 0.5..1.5, +0x1D0 = 0 and Chance(0.5) a zero +0x1CC, else
/// -0.5..0.5; 2, 4, 9..12: Chance(0.3) holds 3..4 with zero offsets, else as the first with -0.2..0.2; 3: 2..3, then
/// Chance(0.3) zero offsets, else +0x1D0 in -0.15..-0.05 and +0x1CC in -0.4..0.4). Then a += +0x1CC and b += +0x1D0.
/// Completion: 100%.
/// # Errors
/// No host draw.
pub fn face_gen_update_expression_timer(
    world: &mut World,
    animation: Entity,
    expression: i32,
    values: &mut [f32; 2],
    time: f32,
) -> Result<(), GameError> {
    use crate::tes::actor_movement_update::{random_chance, random_float_in_range};
    let timer = {
        let mut data = world
            .get_mut::<BsFaceGenAnimationData>(animation)
            .expect("FaceGen animation data");
        data.expression_timer_1c8 = (f64::from(data.expression_timer_1c8) - f64::from(time)) as f32;
        data.expression_timer_1c8
    };
    if timer <= 0.0 {
        let range = |world: &mut World, low: f32, high: f32| -> Result<f32, GameError> {
            Ok(random_float_in_range(world, low, high)? as f32)
        };
        // (hold, offset 1D0, offset 1CC) where None leaves the field.
        let (hold, second, first): (f32, Option<f32>, Option<f32>) = match expression {
            -1 | 0 | 5..=8 => {
                if random_chance(world, 0.3)? {
                    (range(world, 2.0, 3.0)?, Some(0.0), Some(0.0))
                } else {
                    let hold = range(world, 0.5, 1.5)?;
                    let second = range(world, -0.05, 0.15)?;
                    (hold, Some(second), Some(range(world, -0.1, 0.1)?))
                }
            }
            1 => {
                let hold = range(world, 0.5, 1.5)?;
                if random_chance(world, 0.5)? {
                    (hold, Some(0.0), Some(0.0))
                } else {
                    (hold, Some(0.0), Some(range(world, -0.5, 0.5)?))
                }
            }
            2 | 4 | 9..=12 => {
                if random_chance(world, 0.3)? {
                    (range(world, 3.0, 4.0)?, Some(0.0), Some(0.0))
                } else {
                    let hold = range(world, 0.5, 1.5)?;
                    let second = range(world, -0.05, 0.15)?;
                    (hold, Some(second), Some(range(world, -0.2, 0.2)?))
                }
            }
            3 => {
                let hold = range(world, 2.0, 3.0)?;
                if random_chance(world, 0.3)? {
                    (hold, Some(0.0), Some(0.0))
                } else {
                    let second = range(world, -0.15, -0.05)?;
                    (hold, Some(second), Some(range(world, -0.4, 0.4)?))
                }
            }
            _ => {
                let data = world
                    .get::<BsFaceGenAnimationData>(animation)
                    .expect("FaceGen animation data");
                (data.expression_timer_1c8, None, None)
            }
        };
        let mut data = world
            .get_mut::<BsFaceGenAnimationData>(animation)
            .expect("FaceGen animation data");
        data.expression_timer_1c8 = hold;
        if let Some(second) = second {
            data.expression_offsets_1cc[1] = second;
        }
        if let Some(first) = first {
            data.expression_offsets_1cc[0] = first;
        }
    }
    let data = world
        .get::<BsFaceGenAnimationData>(animation)
        .expect("FaceGen animation data");
    values[0] = (f64::from(data.expression_offsets_1cc[0]) + f64::from(values[0])) as f32;
    values[1] = (f64::from(data.expression_offsets_1cc[1]) + f64::from(values[1])) as f32;
    Ok(())
}

/// IDA: 0054C6B0 BSFaceGenAnimationData_UpdateHeadTrack(time, parent): +0x1DA set answers false; `0054A0B0` picks the
/// expression with the largest keyframe value in (0, 1] (+0x48's vt+0x48 for -1..12; a fresh data's are all 0.0, so -1:
/// the port reads +0x48's values for 0..12 and takes slot -1 as 0.0); below 13 `face_gen_update_expression_timer` on
/// +0x19C and +0x1A0; each difference from +0x18C / +0x190 clamped to fTrackSpeed x time and added; `0054B8C0` with the
/// offsets from +0x184 / +0x188 (the keyframes', counted). Answers true. Completion: 100% of the floats.
/// # Errors
/// No host draw.
pub fn face_gen_update_head_track(
    world: &mut World,
    animation: Entity,
    time: f32,
) -> Result<bool, GameError> {
    let (blocked, expression, mut values) = {
        let data = world
            .get::<BsFaceGenAnimationData>(animation)
            .expect("FaceGen animation data");
        let mut best = 0.0_f32;
        let mut expression = -1;
        for (index, &value) in data.keyframes[2].values_0c.iter().take(13).enumerate() {
            if value > 0.0 && value <= 1.0 && best < value {
                best = value;
                expression = index as i32;
            }
        }
        (
            data.animation_disabled_1da != 0,
            expression,
            [data.floats_17c[8], data.floats_17c[9]],
        )
    };
    if blocked {
        return Ok(false);
    }
    if expression < 13 {
        face_gen_update_expression_timer(world, animation, expression, &mut values, time)?;
    }
    let speed = float_named_or_zero(world, "fTrackSpeed");
    let limit = (f64::from(speed) * f64::from(time)) as f32;
    {
        let mut data = world
            .get_mut::<BsFaceGenAnimationData>(animation)
            .expect("FaceGen animation data");
        let clamp = |value: f32| value.clamp(-limit, limit);
        let first = clamp((f64::from(values[0]) - f64::from(data.floats_17c[4])) as f32);
        let second = clamp((f64::from(values[1]) - f64::from(data.floats_17c[5])) as f32);
        data.floats_17c[4] = (f64::from(first) + f64::from(data.floats_17c[4])) as f32;
        data.floats_17c[5] = (f64::from(data.floats_17c[5]) + f64::from(second)) as f32;
    }
    // 0054C7E3..0054C7F4: 0054B8C0(+0x18C - +0x184, +0x190 - +0x188).
    let (yaw, pitch) = {
        let data = world
            .get::<BsFaceGenAnimationData>(animation)
            .expect("FaceGen animation data");
        (
            data.floats_17c[4] - data.floats_17c[2],
            data.floats_17c[5] - data.floats_17c[3],
        )
    };
    let limits = (
        float_named_or_zero(world, "fTrackEyeXY"),
        float_named_or_zero(world, "fTrackEyeZ"),
    );
    let mut data = world
        .get_mut::<BsFaceGenAnimationData>(animation)
        .expect("FaceGen animation data");
    face_gen_set_eye_look(&mut data.keyframes[5], yaw, pitch, limits);
    Ok(true)
}

/// IDA: 0054B8C0 BSFaceGenAnimationData_SetEyeLook(yaw, pitch) (thiscall data, `retn 8`): fTrackEyeXY (00B39B10) and
/// fTrackEyeZ (00B39B18) in degrees x pi/180 (dbl_A31C78), each stored as a float and clamped to [0, pi/2] (below 0.0
/// stored 0.0; above dbl_A641E0 the float 1.5707964); the yaw clamped to +-the first, the pitch to +-the second; then on
/// +0xA4's values (vt+0x4C): LookRight (10) the yaw over its limit when the yaw is 0 or more, else 0.0; LookLeft (9) the
/// yaw over minus its limit when below 0, else 0.0; LookUp (11) and LookDown (8) the same from the pitch. Each quotient a
/// float (24-bit). Completion: 100%.
pub fn face_gen_set_eye_look(
    keyframe: &mut FaceGenKeyframe,
    yaw: f32,
    pitch: f32,
    limits: (f32, f32),
) {
    let radians = |degrees: f32| {
        let value = (f64::from(degrees) * f64::from(f32::from_bits(0x3C8E_FA35))) as f32;
        let value = if value < 0.0 { 0.0 } else { value };
        if f64::from(value) > std::f64::consts::FRAC_PI_2 {
            1.570_796_4_f32
        } else {
            value
        }
    };
    let horizontal = radians(limits.0);
    let vertical = radians(limits.1);
    let clamp = |value: f32, limit: f32| {
        if value > limit {
            limit
        } else if -limit > value {
            -limit
        } else {
            value
        }
    };
    let yaw = clamp(yaw, horizontal);
    let pitch = clamp(pitch, vertical);
    if yaw >= 0.0 {
        keyframe.set_value(10, yaw / horizontal);
        keyframe.set_value(9, 0.0);
    } else {
        keyframe.set_value(10, 0.0);
        keyframe.set_value(9, yaw / -horizontal);
    }
    if pitch >= 0.0 {
        keyframe.set_value(11, pitch / vertical);
        keyframe.set_value(8, 0.0);
    } else {
        keyframe.set_value(11, 0.0);
        keyframe.set_value(8, pitch / -vertical);
    }
}

/// IDA: 0054C090 BSFaceGenAnimationData vt+0xDC(time, ticks): a time at or below 0 answers false. +0x1D7 = 0. The +0x10
/// keyframe's vt+0x3C(time, +0x0C) (`0054F170`, head tracking toward +0x0C's target: none here, so false), then
/// `0054BA90` on each list into its working keyframe, and the four result keyframes the morphs read (`0055D1B0`):
/// +0x48 takes +0x10 (copy) when that updated, then +0x34 (blend, 1.0, 0, 0) when its list did, or +0x34 alone; with
/// neither, vt+0xD4 releases +0x0C. +0xA4 from (+0x5C -> +0x6C, 0) and (+0x80 -> +0x90, 1); +0x100 from (+0xB8 -> +0xC8, 0)
/// and (+0xDC -> +0xEC, 1); +0x15C from (+0x114 -> +0x124, 0) and (+0x138 -> +0x148, 1): the first's copy, then the
/// second blended, or the second's copy. +0x1D7 = any result changed. With +0x1DA clear, an empty blink list (+0x5C,
/// count +0x68), fBlinkDownTime, fBlinkUpTime and fBlinkDelayMin above 0 and fBlinkDelayMin <= fBlinkDelayMax:
/// Random_Next(0) (0047DF80 at 0054C452) scaled into the delay range (min + draw x (max - min) / 32767, each step a
/// float), and vt+0xA4 (`0054A8A0`) three times, each pushing a 17-value keyframe (type 2; the first two values, the
/// eyes, set and the rest FLT_MAX) onto +0x5C: 0.0 after the delay, 1.0 over fBlinkDownTime, 0.0 over fBlinkUpTime.
/// Then +0x1DC / 100 against +0x15C's and +0x124's first value (a difference is a stop: vt+0xB0 and vt+0xC4).
/// Precision: 24-bit (the main thread). Completion: 100% but the head-track keyframe's target and the custom morph's
/// change, stops.
/// # Errors
/// No host draw, a head-track target, or a custom morph change.
pub fn face_gen_update_keyframes(
    world: &mut World,
    animation: Entity,
    time: f32,
) -> Result<bool, GameError> {
    if time <= 0.0 {
        return Ok(false);
    }
    let mut data = world
        .get::<BsFaceGenAnimationData>(animation)
        .cloned()
        .expect("FaceGen animation data");
    data.bytes_1d4[3] = 0;
    if data.head_track_target_0c != 0 {
        return Err(GameError::unclassified(
            0x0054C0D9,
            "0054C0D9: the head-track keyframe's target (0054F170)",
        ));
    }
    let mut lists = std::mem::take(&mut data.lists);
    let keyframes = &mut data.keyframes;
    // 0054C0E3: the modifiers, from +0x10 (no target: false) and (+0x24 -> +0x34, 1).
    let modifiers = face_gen_advance_keyframe_list(time, &mut lists[0], &mut keyframes[1], true);
    let mut any = false;
    if modifiers {
        let source = keyframes[1].clone();
        keyframes[2].copy_from(&source);
        any = true;
    }
    // The other three results from their list pairs.
    for (first_list, first, second_list, second, result) in
        [(1, 3, 2, 4, 5), (3, 6, 4, 7, 8), (5, 9, 6, 10, 11)]
    {
        let changed_first = face_gen_advance_keyframe_list(
            time,
            &mut lists[first_list],
            &mut keyframes[first],
            false,
        );
        let changed_second = face_gen_advance_keyframe_list(
            time,
            &mut lists[second_list],
            &mut keyframes[second],
            true,
        );
        if changed_first {
            let source = keyframes[first].clone();
            keyframes[result].copy_from(&source);
            if changed_second {
                let other = keyframes[second].clone();
                keyframes[result].blend(&other, 1.0, false, false);
            }
            any = true;
        } else if changed_second {
            let source = keyframes[second].clone();
            keyframes[result].copy_from(&source);
            any = true;
        }
    }
    data.bytes_1d4[3] = u8::from(any);
    let blocked = data.animation_disabled_1da != 0;
    let blinking = !lists[1].is_empty();
    data.lists = lists;
    world.entity_mut(animation).insert(data);
    count_face(world, 0x0054_BA90);
    let down = float_named_or_zero(world, "fBlinkDownTime");
    let up = float_named_or_zero(world, "fBlinkUpTime");
    let low = float_named_or_zero(world, "fBlinkDelayMin");
    let high = float_named_or_zero(world, "fBlinkDelayMax");
    if !blocked && !blinking && down > 0.0 && up > 0.0 && low > 0.0 && low <= high {
        let span = high - low;
        let draw = world
            .get_non_send::<crate::tes::form_arms::ScriptRandom>()
            .map(|random| random.1.clone())
            .ok_or(GameError::unclassified(
                0x0054C44D,
                "0054C44D: a draw with no host",
            ))?;
        let value = draw();
        // 24-bit: the product, the quotient and the sum each round to a float.
        let delay = low + value as f32 * span / 32767.0;
        count_face(world, 0x0054_C494);
        let mut data = world
            .get_mut::<BsFaceGenAnimationData>(animation)
            .expect("FaceGen animation data");
        // 0054A8A0: a zeroed 17-value keyframe (type 2) given the set's values, kept when its time is 0 or more
        // (+0x1DA was tested above) and its type agrees with the list's head (`0054F350`).
        for (eyes, keyframe_time) in [(0.0, delay), (1.0, down), (0.0, up)] {
            if keyframe_time >= 0.0 {
                let mut keyframe = FaceGenKeyframe::new(2, 17);
                keyframe.values_0c.fill(FACE_GEN_UNSET);
                keyframe.values_0c[0] = eyes;
                keyframe.values_0c[1] = eyes;
                keyframe.time_08 = keyframe_time;
                data.lists[1].push_back(keyframe);
            }
        }
    }
    // 0054C523: the custom morph's value (+0x1DC / 100) against +0x15C and +0x124.
    let data = world
        .get::<BsFaceGenAnimationData>(animation)
        .expect("FaceGen animation data");
    let custom = (f64::from(data.custom_morph_1dc) / 100.0) as f32;
    if custom != data.keyframes[11].value(0) || custom != data.keyframes[9].value(0) {
        return Err(GameError::unclassified(
            0x0054C55E,
            "0054C55E: the custom morph's keyframe (vt+0xB0, vt+0xC4)",
        ));
    }
    Ok(data.bytes_1d4[3] != 0)
}

/// IDA: 0054C5D0 BSFaceGenAnimationData vt+0x84 SetLookDirection(x, y, z, unused): a zero +0x170 with a nonzero direction
/// sets +0x1C0 = 1 and +0x1C4 = 0.0; a zero direction sets +0x1C0 = 0; +0x170 = the direction; +0x1D5 = 1 (exact float
/// compares, `Vec3_NotEqualExact`). Completion: 100%.
pub fn face_gen_set_look_direction(data: &mut BsFaceGenAnimationData, direction: [f32; 3]) {
    let zero = |vector: &[f32; 3]| vector.iter().all(|&value| value == 0.0);
    if zero(&data.look_direction_170) && !zero(&direction) {
        data.field_1c0 = 1;
        data.look_direction_time_1c4 = 0.0;
    }
    if zero(&direction) {
        data.field_1c0 = 0;
    }
    data.look_direction_170 = direction;
    data.bytes_1d4[1] = 1;
}

// IDA: 0054A450 the yaw limits: fTrackXY (00B39AF8) x pi/180 (dbl_A31C78) stored as a float, clamped to [0, pi / 2]
// (above it the float 1.5707964); the lower limit its negation. Answers (lower, upper).
fn track_yaw_limits(world: &World) -> (f32, f32) {
    let upper = (f64::from(float_named_or_zero(world, "fTrackXY"))
        * f64::from(f32::from_bits(0x3C8E_FA35))) as f32;
    let upper = if upper < 0.0 { 0.0 } else { upper };
    let upper = if f64::from(upper) > std::f64::consts::FRAC_PI_2 {
        1.570_796_4_f32
    } else {
        upper
    };
    (-upper, upper)
}

// IDA: 0054A4B0 the pitch limits: fTrackMinZ (00B39B00) x pi/180, a float, at most 0.0 and at least -pi / 2 (the float
// -1.5707964); fTrackMaxZ (00B39B08) likewise, at least 0.0 and at most pi / 2. Answers (lower, upper).
fn track_pitch_limits(world: &World) -> (f32, f32) {
    let radians = |name: &str| {
        (f64::from(float_named_or_zero(world, name)) * f64::from(f32::from_bits(0x3C8E_FA35)))
            as f32
    };
    let lower = radians("fTrackMinZ");
    let lower = if lower > 0.0 { 0.0 } else { lower };
    let lower = if f64::from(lower) < -std::f64::consts::FRAC_PI_2 {
        -1.570_796_4_f32
    } else {
        lower
    };
    let upper = radians("fTrackMaxZ");
    let upper = if upper < 0.0 { 0.0 } else { upper };
    let upper = if f64::from(upper) > std::f64::consts::FRAC_PI_2 {
        1.570_796_4_f32
    } else {
        upper
    };
    (lower, upper)
}

/// IDA: 0054B010 BSFaceGenAnimationData_TrackHead(time, head, non-accum) (thiscall data, `retn 0Ch`): +0x1DA set answers
/// false. With +0x1D5 clear or a zero-length +0x170 (`__CIsqrt` of the squares, a float): the head bone's local rotation
/// (+0x30) under its six parents', multiplied ((((((P6 x P5) x P4) x P3) x P1) x P2) x head) (P1 and P2 in that order, as
/// 0054B1D9.. pushes them), times the inverse (`007103C0`) of NiMatrix33_FromEulerAnglesXYZ(1.5524637, 0.1352797,
/// 1.5694213), through `00711440` into the yaw and pitch; a look direction is a stop. +0x19C/+0x1A0 = the yaw and pitch;
/// the speeds: the axis with the larger error from +0x184/+0x188 (each `fabs` of a float difference) turns at
/// fTrackSpeed (00B39AF0) and the other at its error over the larger's time; +0x1AC = ((1 - (range - |+0x184|) / range) x
/// 0.7 + 0.3) x fTrackSpeed (the yaw range of `0054A450`); +0x1B0 = 0.75 x the yaw when +0x198 is set and |yaw| > pi / 4,
/// else 0.0; +0x1B4 = 0.0; +0x1B8/+0x1BC = the yaw and pitch; with +0x1D4 set (cleared) +0x184/+0x188 and +0x18C/+0x190
/// take the yaw and pitch, +0x17C = +0x1B0, +0x180 = 0.0. Precision: 24-bit. Completion: 100% of the bone arm.
/// # Errors
/// A look direction, or a head bone without six parents.
pub fn face_gen_track_head(
    world: &mut World,
    animation: Entity,
    head: Entity,
) -> Result<bool, GameError> {
    use crate::gamebryo::transform::{
        invert_rotation_or_zero, matrix_from_euler_xyz, matrix_to_angles_zxy, multiply,
    };
    let data = world
        .get::<BsFaceGenAnimationData>(animation)
        .cloned()
        .expect("FaceGen animation data");
    if data.animation_disabled_1da != 0 {
        return Ok(false);
    }
    let length = {
        let [x, y, z] = data.look_direction_170;
        (f64::from(y * y + x * x + z * z).sqrt()) as f32
    };
    if data.bytes_1d4[1] != 0 && length > 0.0 {
        return Err(GameError::unclassified(
            0x0054B046,
            "0054B046: the look direction's arm (a look target)",
        ));
    }
    let rotation = |entity: Entity| {
        world
            .get::<crate::gamebryo::scene_node::SceneNodePayload>(entity)
            .map(|payload| payload.local_transform.rotation)
    };
    let parent = |entity: Entity| {
        world
            .get::<crate::gamebryo::av_object::NiAvParent>(entity)
            .and_then(|parent| parent.0)
    };
    let mut chain = vec![head];
    for _ in 0..6 {
        let next = parent(*chain.last().expect("one")).ok_or(GameError::unclassified(
            0x0054B170,
            "0054B170: a head bone with fewer than six parents",
        ))?;
        chain.push(next);
    }
    let local = |index: usize| {
        rotation(chain[index]).ok_or(GameError::unclassified(
            0x0054B18D,
            "0054B18D: a head bone chain without local rotations",
        ))
    };
    let mut product = multiply(&local(6)?, &local(5)?);
    product = multiply(&product, &local(4)?);
    product = multiply(&product, &local(3)?);
    product = multiply(&product, &local(1)?);
    product = multiply(&product, &local(2)?);
    product = multiply(&product, &local(0)?);
    let other = matrix_from_euler_xyz(
        f32::from_bits(0x3FC6_B721),
        f32::from_bits(0x3E0A_86C3),
        f32::from_bits(0x3FC8_E2CC),
    );
    let relative = multiply(&product, &invert_rotation_or_zero(&other));
    let [yaw, _, pitch] = matrix_to_angles_zxy(&relative);
    let speed = float_named_or_zero(world, "fTrackSpeed");
    let (_, range) = track_yaw_limits(world);
    let mut data = world
        .get_mut::<BsFaceGenAnimationData>(animation)
        .expect("FaceGen animation data");
    let still = data.byte_198 != 0;
    let f = &mut data.floats_17c;
    f[8] = yaw;
    f[9] = pitch;
    let yaw_error = (f[2] - yaw).abs();
    let pitch_error = (f[3] - pitch).abs();
    if pitch_error >= yaw_error {
        f[11] = speed;
        let time = pitch_error / f[11];
        f[10] = yaw_error / time;
    } else {
        f[10] = speed;
        let time = yaw_error / f[10];
        f[11] = pitch_error / time;
    }
    let turned = f[2].abs();
    let left = range - turned;
    f[12] = ((1.0 - left / range) * 0.7 + 0.3) * speed;
    f[13] = if still && yaw.abs() > std::f32::consts::FRAC_PI_4 {
        0.75 * yaw
    } else {
        0.0
    };
    f[14] = 0.0;
    f[15] = yaw;
    f[16] = pitch;
    if data.bytes_1d4[0] != 0 {
        data.bytes_1d4[0] = 0;
        let f = &mut data.floats_17c;
        f[2] = yaw;
        f[3] = pitch;
        f[0] = f[13];
        f[1] = 0.0;
        f[4] = yaw;
        f[5] = pitch;
    }
    Ok(true)
}

/// IDA: 0054B3E0 BSFaceGenAnimationData_EaseBodyTurn(time, head, out yaw, out pitch): with +0x1DA set both 0.0; else
/// +0x17C/+0x180 move toward +0x1B0/+0x1B4 by at most +0x1AC x time x 0.5, then clamp to +-pi / 4 and +-pi / 8 (floats);
/// answers them. Precision: 24-bit. Completion: 100%.
pub fn face_gen_ease_body_turn(data: &mut BsFaceGenAnimationData, time: f32) -> (f32, f32) {
    if data.animation_disabled_1da != 0 {
        return (0.0, 0.0);
    }
    let f = &mut data.floats_17c;
    let limit = f[12] * time * 0.5;
    let clamp = |value: f32| {
        if value > limit {
            limit
        } else if -limit > value {
            -limit
        } else {
            value
        }
    };
    let yaw_step = clamp(f[13] - f[0]);
    let pitch_step = clamp(f[14] - f[1]);
    f[0] += yaw_step;
    f[1] += pitch_step;
    f[0] = f[0].clamp(-std::f32::consts::FRAC_PI_4, std::f32::consts::FRAC_PI_4);
    f[1] = f[1].clamp(-std::f32::consts::FRAC_PI_8, std::f32::consts::FRAC_PI_8);
    (f[0], f[1])
}

/// IDA: 0054B5A0's angles (the head turn, from `0055D4C0`): with +0x1DA clear, the yaw and pitch errors (+0x1B8 - +0x184,
/// +0x1BC - +0x188, floats, each brought into [-pi, pi] by 2 pi) clamped to +-(+0x1A4 / +0x1A8 x time) and added to
/// +0x184/+0x188; then +0x184 kept within +0x17C + 0.75 x the yaw limits and +0x188 within +0x180 + 0.5 x the pitch
/// limits. The rotation it builds from them goes into the head bone's world rotation (no host form: the morphs and
/// 0054B010 read none of it). Precision: 24-bit. Completion: 100% of the angles.
pub fn face_gen_turn_head(world: &mut World, animation: Entity, time: f32) {
    let yaw_limits = track_yaw_limits(world);
    let pitch_limits = track_pitch_limits(world);
    let mut data = world
        .get_mut::<BsFaceGenAnimationData>(animation)
        .expect("FaceGen animation data");
    if data.animation_disabled_1da != 0 {
        return;
    }
    let f = &mut data.floats_17c;
    let wrap = |mut value: f32| {
        if f64::from(value) > 3.141_592_741_012_573_2 {
            value = (f64::from(value) - 6.283_185_482_025_146_5) as f32;
        }
        if f64::from(value) < -3.141_592_741_012_573_2 {
            value = (f64::from(value) + 6.283_185_482_025_146_5) as f32;
        }
        value
    };
    let yaw_error = wrap(f[15] - f[2]);
    let pitch_error = wrap(f[16] - f[3]);
    let clamp = |value: f32, limit: f32| {
        if value > limit {
            limit
        } else if -limit > value {
            -limit
        } else {
            value
        }
    };
    let yaw_step = clamp(yaw_error, f[10] * time);
    let pitch_step = clamp(pitch_error, time * f[11]);
    f[2] += yaw_step;
    f[3] += pitch_step;
    let low = yaw_limits.0 * 0.75 + f[0];
    let high = 0.75 * yaw_limits.1 + f[0];
    if high < f[2] {
        f[2] = high;
    } else if low > f[2] {
        f[2] = low;
    }
    let low = pitch_limits.0 * 0.5 + f[1];
    let high = 0.5 * pitch_limits.1 + f[1];
    if high < f[3] {
        f[3] = high;
    } else if low > f[3] {
        f[3] = low;
    }
}

/// IDA: 0055B000's name tables: the expressions (00B11FF0, 13), the modifiers (00B12028, 17), the phonemes (00B12070, 16)
/// and the custom morph (00B12024, 1), each looked up in the head's TRI (`_stricmp`) among its differential morphs, then its
/// statistical morphs.
pub const FACE_GEN_EXPRESSIONS_B11FF0: [&str; 13] = [
    "Anger",
    "Fear",
    "Happy",
    "Sad",
    "Surprise",
    "DispAnger1",
    "DispAnger2",
    "DispAnger3",
    "DispAnger4",
    "DispHappy1",
    "DispHappy2",
    "DispHappy3",
    "DispHappy4",
];
pub const FACE_GEN_MODIFIERS_B12028: [&str; 17] = [
    "BlinkLeft",
    "BlinkRight",
    "BrowDownLeft",
    "BrowDownRight",
    "BrowInLeft",
    "BrowInRight",
    "BrowUpLeft",
    "BrowUpRight",
    "LookDown",
    "LookLeft",
    "LookRight",
    "LookUp",
    "SquintLeft",
    "SquintRight",
    "HeadPitch",
    "HeadRoll",
    "HeadYaw",
];
pub const FACE_GEN_PHONEMES_B12070: [&str; 16] = [
    "Aah", "BigAah", "BMP", "ChJSh", "DST", "Eee", "Eh", "FV", "I", "K", "N", "Oh", "OohQ", "R",
    "Th", "W",
];
pub const FACE_GEN_CUSTOM_B12024: [&str; 1] = ["VampireMorph"];

/// IDA: N/A one of BSFaceGenMorphDataHead's morphs (`0055B000`): a TRI differential morph (`0055A3B0` applies it) or a
/// statistical one (`0055A800`).
enum HeadMorph<'a> {
    Differential(&'a crate::bs::face_gen_files::TriDifferentialMorph),
    Statistical(&'a crate::bs::face_gen_files::TriStatisticalMorph),
}

// IDA: 0055B000's lookup of one name: the differential morphs first, then the statistical ones (`_stricmp`).
fn head_morph<'a>(
    tri: &'a crate::bs::face_gen_files::TriData,
    name: &str,
) -> Option<HeadMorph<'a>> {
    tri.differential_morphs_80
        .iter()
        .find(|morph| oblivion_core::crt::stricmp(&morph.name_00, name.as_bytes()) == 0)
        .map(HeadMorph::Differential)
        .or_else(|| {
            tri.statistical_morphs_90
                .iter()
                .find(|morph| oblivion_core::crt::stricmp(&morph.name_00, name.as_bytes()) == 0)
                .map(HeadMorph::Statistical)
        })
}

/// IDA: 0055A3B0 BSFaceGenMorphDifferential_Apply(positions, count, extra, weight) and 0055A800
/// BSFaceGenMorphStatistical_Apply: with 0.0 < weight <= 1.0, a differential morph adds each of its first min(count,
/// deltas) deltas times the weight to the positions; a statistical morph moves each listed base vertex v by (the base
/// morph's vertex +0x1C + i less its vertex v) times the weight (the base morph's vertices, `extra` vt+0x54). Each step
/// stored as a float (24-bit on the main thread). Completion: 100%.
fn apply_head_morph(
    morph: &HeadMorph<'_>,
    positions: &mut [[f32; 3]],
    count: usize,
    base: &[[f32; 3]],
    weight: f32,
) {
    if !(weight > 0.0 && weight <= 1.0) {
        return;
    }
    match morph {
        HeadMorph::Differential(morph) => {
            let count = count.min(morph.deltas_1c.len()).min(positions.len());
            for (position, delta) in positions[..count].iter_mut().zip(&morph.deltas_1c) {
                for axis in 0..3 {
                    let scaled = delta[axis] * weight;
                    position[axis] += scaled;
                }
            }
        }
        HeadMorph::Statistical(morph) => {
            for (i, &vertex) in morph.vertices_20.iter().enumerate() {
                let vertex = vertex as usize;
                let added = morph.base_index_1c as usize + i;
                let (Some(from), Some(to), Some(position)) =
                    (base.get(vertex), base.get(added), positions.get_mut(vertex))
                else {
                    continue;
                };
                for axis in 0..3 {
                    let difference = to[axis] - from[axis];
                    let scaled = difference * weight;
                    position[axis] += scaled;
                }
            }
        }
    }
}

/// IDA: 0054F5E0(a, b, first, second) as 0055C900 calls it for the look pairs: d = b - a (`a x -1.0 + b`, a float); below
/// 0: first = -d, second = 0.0; else first = 0.0, second = d. Completion: 100%.
fn split_pair(a: f32, b: f32) -> (f32, f32) {
    // `a x -1.0 + b` and `-1.0 x d`: an exact negation.
    let difference = -a + b;
    if difference < 0.0 {
        (-difference, 0.0)
    } else {
        (0.0, difference)
    }
}

/// IDA: 0055D1B0 BSFaceGenNiNode_ApplyAnimationMorphs(node, force): for each child (+0xB0, count +0xB6) whose geometry has
/// data (+0xB4), a FaceGen model (`00550800`, the morph data) and a base morph (`FaceGen_FindBaseMorphExtraData`), the data
/// locked (`00728AB0`, `00728B60`): with `force` or +0x104 set, the base morph's first +0x14 vertices copied in
/// (FaceGen_CopyBaseMorphToGeometry) and +0x104 cleared. The distance from g_faceGenNodeCamera (the World SceneGraph's
/// camera, `0055CBF0`) to the node's world translation (+0x88): within fTalkingDistance + 10 (00B120D4, a quarter under
/// g_havokStepFlag), or +0x112 set, or menu mode: `0055CAA0` the 16 phonemes (the anim data's +0x100, the morph data's
/// +0x10); and within fLodDistance + 10 (00B120CC) or the same: `0055C850` the 13 expressions (+0x48, +0x08),
/// `0055C900` the 17 modifiers (+0xA4, +0x0C; LookDown/LookUp and LookLeft/LookRight from `0054F5E0` on the pairs 8/11 and
/// 9/10, HeadPitch/Roll/Yaw skipped) and `0055CB50` the custom morph (+0x15C, +0x14). Each weight in (0, 1] copies the base
/// morph once (unless copied) and applies its morph. Then `00728B20`. **Stops:** the else-arms (the anim data's +0x1DA set,
/// vt+0x98; or nothing applied within fLodDistance - 10: vt+0x78's reset), which menu mode never reaches. Completion: 100%
/// of the morphs; the data lock has no host form.
/// # Errors
/// The stops named.
pub fn face_gen_apply_animation_morphs(
    world: &mut World,
    node: Entity,
    force: bool,
    menu_mode: bool,
) -> Result<(), GameError> {
    use crate::bs::face_gen_model::{BsFaceGenModelMap, FaceGenMorphData};
    let Some(face) = world.get::<BsFaceGenNiNode>(node).cloned() else {
        return Ok(());
    };
    let Some(animation) = face.animation_data_dc else {
        return Ok(());
    };
    let children: Vec<Entity> = world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(node)
        .map(|children| children.slots_ac.iter().flatten().copied().collect())
        .unwrap_or_default();
    let camera = world
        .get_resource::<crate::bs::scene_graphs::SceneGraphGlobals>()
        .and_then(|globals| globals.world)
        .and_then(|graph| world.get::<crate::bs::scene_graphs::SceneGraph>(graph))
        .and_then(|graph| graph.camera_dc)
        .and_then(|camera| world.get::<crate::gamebryo::scene_node::NiAvWorld>(camera))
        .map_or([0.0; 3], |state| state.transform.translation);
    let here = world
        .get::<crate::gamebryo::scene_node::NiAvWorld>(node)
        .map_or([0.0; 3], |state| state.transform.translation);
    let lod = ini_float(world, 0x00B1_20CC, 500.0);
    let talking = ini_float(world, 0x00B1_20D4, 2000.0);
    let data = world
        .get::<BsFaceGenAnimationData>(animation)
        .cloned()
        .expect("FaceGen animation data");
    let mut pending_force = face.bytes_104[0] != 0;
    for child in children {
        let Some(part) = world.get::<FaceGenPartMorph>(child).cloned() else {
            continue;
        };
        let Some(geometry_data) = world
            .get::<NiGeometry>(child)
            .and_then(|links| links.data_b4)
        else {
            continue;
        };
        let Some(model) = world
            .get_resource::<BsFaceGenModelMap>()
            .and_then(|map| map.models.get(part.model))
            .cloned()
        else {
            continue;
        };
        let Some(FaceGenMorphData::Head(tri)) = model.morph_data_1c.as_ref() else {
            continue;
        };
        let mut positions =
            crate::bs::face_gen_model::geometry_positions(world, child).unwrap_or_default();
        let count = positions.len();
        let base = &part.base.vertices_0c;
        let copy_base = |positions: &mut Vec<[f32; 3]>| {
            let count = (part.base.data_count_14 as usize)
                .min(base.len())
                .min(positions.len());
            positions[..count].copy_from_slice(&base[..count]);
        };
        let mut copied = false;
        if force || pending_force {
            copy_base(&mut positions);
            copied = true;
            pending_force = false;
            if let Some(mut face) = world.get_mut::<BsFaceGenNiNode>(node) {
                face.bytes_104[0] = 0;
            }
        }
        // 0055D2C5..0055D30C: the camera's distance (hkVector4::length3, a float).
        let offset = [
            camera[0] - here[0],
            camera[1] - here[1],
            camera[2] - here[2],
        ];
        let distance = ((f64::from(offset[0]) * f64::from(offset[0])
            + f64::from(offset[1]) * f64::from(offset[1])
            + f64::from(offset[2]) * f64::from(offset[2]))
        .sqrt()) as f32;
        let always = face.bytes_110[2] != 0 || menu_mode;
        let mut phonemes_ran = false;
        let mut all_ran = false;
        let apply_set = |names: &[&str],
                         weights: &dyn Fn(usize) -> Option<f32>,
                         positions: &mut Vec<[f32; 3]>,
                         copied: &mut bool| {
            for (index, name) in names.iter().enumerate() {
                let Some(weight) = weights(index) else {
                    continue;
                };
                if !(weight > 0.0 && weight <= 1.0) {
                    continue;
                }
                let Some(morph) = head_morph(tri, name) else {
                    continue;
                };
                if !*copied {
                    copy_base(positions);
                    *copied = true;
                }
                apply_head_morph(&morph, positions, count, base, weight);
            }
        };
        if talking + 10.0 >= distance || always {
            let near = lod + 10.0 >= distance || always;
            // 0055CAA0: the phonemes.
            let phonemes = &data.keyframes[8];
            apply_set(
                &FACE_GEN_PHONEMES_B12070,
                &|i| Some(phonemes.value(i)),
                &mut positions,
                &mut copied,
            );
            phonemes_ran = true;
            if near {
                // 0055C850: the expressions.
                let expressions = &data.keyframes[2];
                apply_set(
                    &FACE_GEN_EXPRESSIONS_B11FF0,
                    &|i| Some(expressions.value(i)),
                    &mut positions,
                    &mut copied,
                );
                // 0055C900: the modifiers, the look pairs split.
                let modifiers = &data.keyframes[5];
                let (down, up) = split_pair(modifiers.value(8), modifiers.value(11));
                let (left, right) = split_pair(modifiers.value(9), modifiers.value(10));
                let weight = |i: usize| match i {
                    8 => Some(down),
                    11 => Some(up),
                    9 => Some(left),
                    10 => Some(right),
                    14..=16 => None,
                    _ => Some(modifiers.value(i)),
                };
                apply_set(
                    &FACE_GEN_MODIFIERS_B12028,
                    &weight,
                    &mut positions,
                    &mut copied,
                );
                // 0055CB50: the custom morph.
                let custom = &data.keyframes[11];
                apply_set(
                    &FACE_GEN_CUSTOM_B12024,
                    &|i| Some(custom.value(i)),
                    &mut positions,
                    &mut copied,
                );
                all_ran = true;
            }
        }
        if data.animation_disabled_1da != 0 {
            if !all_ran {
                return Err(GameError::unclassified(
                    0x0055D3F0,
                    "0055D3F0: the anim data's +0x1DA arm (vt+0x98, 0055C900)",
                ));
            }
        } else if !phonemes_ran && lod - 10.0 < distance {
            return Err(GameError::unclassified(
                0x0055D46A,
                "0055D46A: the anim data's reset (vt+0x78)",
            ));
        }
        // 00728B20: the positions back into the data.
        if let Some(mut strips) = world.get_mut::<MeshTriStripsData>(geometry_data) {
            strips.base.positions_1c = positions;
        } else if let Some(mut shape) = world.get_mut::<MeshTriShapeData>(geometry_data) {
            shape.base.positions_1c = positions;
        }
    }
    Ok(())
}

// IDA: N/A an INI float by its value's address (the image value when the collection has none).
fn ini_float(world: &World, address: u32, image: f32) -> f32 {
    world
        .get_resource::<crate::tes::settings::SettingCollections>()
        .and_then(|settings| settings.ini.find_address(address))
        .map_or(image, |setting| f32::from_bits(setting.raw))
}

/// IDA: 0055D4C0 BSFaceGenNiNode vt+0x60 UpdateDownwardPass(time, controllers), its FaceGen arms before NiNode's pass: app
/// culled (+0x18 bit 0) or no animation data (vt+0x9C) goes straight to the pass; bUseFaceGenLOD with +0x111 clear and a
/// positive time is `0055CC60`, a stop; without controllers and +0x1D8 clear, or the same time as +0x10C with +0x1D8 clear,
/// straight to the pass. +0x10C = time. +0x106 clear or the data's +0x1D5 clear: +0x1D7; else +0x1D8 = 0 (vt+0x94) and
/// `face_gen_update_keyframes` with the frame's ticks ((00B39DB0 - 00B39DB4) x 0.001). With +0x106, +0x1D5, +0x108 and
/// bDisableHeadTracking clear: `0054B5A0`'s head matrix onto the parent's world rotation and `0047CA90` (counted), then
/// `face_gen_update_head_track`. +0x30 = +0xE0 (vt+0xA4); a change runs `0055D1B0` (`face_gen_apply_animation_morphs`). After the pass the
/// data's +0x1D5 = 1. **Host substitution:** the port's scene update has no BSFaceGenNiNode class, so `00471F20` calls this
/// before its NiAVObject_Update; nothing between them draws. Completion: the draws and the stores named.
/// # Errors
/// A stop above, or no host draw.
pub fn face_gen_node_update(
    world: &mut World,
    node: Entity,
    time: f32,
    controllers: bool,
) -> Result<(), GameError> {
    let culled = world
        .get::<crate::gamebryo::scene_node::SceneNodePayload>(node)
        .is_some_and(|payload| payload.effective_av_flags & 1 != 0);
    let Some(face) = world.get::<BsFaceGenNiNode>(node).cloned() else {
        return Ok(());
    };
    let Some(animation) = face.animation_data_dc else {
        return Ok(());
    };
    if culled {
        return Ok(());
    }
    if ini_byte(world, 0x00B1_20C4) != 0 && face.bytes_110[1] == 0 && time > 0.0 {
        return Err(GameError::unclassified(
            0x0055D50E,
            "0055D50E: bUseFaceGenLOD's 0055CC60",
        ));
    }
    let pending = world
        .get::<BsFaceGenAnimationData>(animation)
        .expect("FaceGen animation data")
        .bytes_1d4[4];
    if (!controllers || time == face.float_10c) && pending == 0 {
        return Ok(());
    }
    world
        .get_mut::<BsFaceGenNiNode>(node)
        .expect("a FaceGen node")
        .float_10c = time;
    let ticks = world
        .get_resource::<crate::tes::window::WindowProcedureState>()
        .map(|state| *state.time.lock())
        .unwrap_or_default();
    let delta = (f64::from(ticks.ticks_b39db0.wrapping_sub(ticks.previous_ticks_b39db4) as i32)
        * 0.001_000_000_047_497_451_3) as f32;
    let active = world
        .get::<BsFaceGenAnimationData>(animation)
        .expect("FaceGen animation data")
        .bytes_1d4[1]
        != 0;
    let mut changed = if face.bytes_104[2] == 0 || !active {
        world
            .get::<BsFaceGenAnimationData>(animation)
            .expect("FaceGen animation data")
            .bytes_1d4[3]
            != 0
    } else {
        world
            .get_mut::<BsFaceGenAnimationData>(animation)
            .expect("FaceGen animation data")
            .bytes_1d4[4] = 0;
        face_gen_update_keyframes(world, animation, delta)?
    };
    if face.bytes_104[2] != 0 && active && face.byte_108 != 0 && ini_byte(world, 0x00B1_48F4) == 0 {
        // 0055D5F7: 0054B5A0's angles (its rotation into the head bone's world rotation and 0047CA90 have no host form).
        face_gen_turn_head(world, animation, delta);
        count_face(world, 0x0047_CA90);
        if face_gen_update_head_track(world, animation, delta)? {
            changed = true;
        }
    }
    if let Some(mut payload) = world.get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(node)
    {
        payload.local_transform.rotation = face.rotation_e0;
    }
    if changed {
        let menu_mode = crate::tes::interface_manager::is_menu_mode(world);
        // 0055D858 pushes [esp+8] (var_98), a stack byte 0055D4C0 never writes: the force argument is whatever an earlier
        // call left there. run-bd and run-be (`records-run-bd-trace.txt`, `records-run-be-update.txt`) logged it as 1 in
        // every call, both nodes, before and after a rebuild, so the base is copied on each apply (a frame with no
        // weight puts the part back to its base). Host reading: the observed value.
        face_gen_apply_animation_morphs(world, node, true, menu_mode)?;
    }
    world
        .get_mut::<BsFaceGenAnimationData>(animation)
        .expect("FaceGen animation data")
        .bytes_1d4[1] = 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_reset_replaces_selected_facial_queues_and_preserves_the_base_channels() {
        let mut world = World::new();
        let face = spawn_face_gen_animation_data(&mut world);
        let mut data = world.get_mut::<BsFaceGenAnimationData>(face).unwrap();
        let mut previous = FaceGenKeyframe::new(1, 13);
        previous.set_time(1.0);
        previous.values_0c.fill(0.5);
        for queue in &mut data.lists {
            queue.push_back(previous.clone());
        }
        for frame in &mut data.keyframes {
            frame.set_time(1.0);
            frame.values_0c.fill(0.5);
        }
        reset_face_keyframe_channels(&mut data, 0.75, true, true, true, true);
        for (queue, working, kind, width) in
            [(0, 1, 1, 13), (2, 4, 2, 17), (4, 7, 0, 16), (6, 10, 3, 1)]
        {
            let mut neutral = FaceGenKeyframe::new(kind, width);
            neutral.set_time(0.75);
            assert_eq!(data.lists[queue].len(), 1);
            assert_eq!(data.lists[queue][0], neutral);
            assert_eq!(data.keyframes[working].time_08, 0.0);
            assert!(data.keyframes[working].values_0c.iter().all(|v| *v == 0.5));
        }
        for queue in [1, 3, 5] {
            assert_eq!(data.lists[queue][0], previous);
        }
        reset_face_keyframe_channels(&mut data, 0.0, false, true, false, false);
        assert!(data.lists[4].is_empty());
        assert!(data.keyframes[7].values_0c.iter().all(|v| *v == 0.0));
        data.animation_disabled_1da = 1;
        reset_face_keyframe_channels(&mut data, 0.75, false, false, false, true);
        assert!(data.lists[6].is_empty());
        assert_eq!(data.keyframes[10].value(0), 0.5);
        assert_eq!(data.lists[0][0].time_08, 0.75);
    }

    // IDA: N/A 0055D4C0 on a fresh head with a running clock (00B39DB0 16 ms past 00B39DB4): 0054C452's blink delay, then
    // 0054A120's chance and, when it fails, its three ranges, as run-aa's 435..439 show.
    // IDA: 0054BA90 and 0054EC30 over the blink's three keyframes (0.0 after a delay of 1.0, 1.0 over 0.5, 0.0 over 0.5):
    // the working keyframe's eyes stay 0.0 through the delay, then move toward 1.0 by the time over the head's.
    #[test]
    fn the_blink_list_interpolates_the_eyes() {
        let blink = |eyes: f32, time: f32| {
            let mut keyframe = FaceGenKeyframe::new(2, 17);
            keyframe.values_0c.fill(FACE_GEN_UNSET);
            keyframe.values_0c[0] = eyes;
            keyframe.values_0c[1] = eyes;
            keyframe.time_08 = time;
            keyframe
        };
        let mut list: std::collections::VecDeque<FaceGenKeyframe> =
            [blink(0.0, 1.0), blink(1.0, 0.5), blink(0.0, 0.5)]
                .into_iter()
                .collect();
        let mut working = FaceGenKeyframe::new(2, 17);
        assert!(!face_gen_advance_keyframe_list(
            0.4,
            &mut list,
            &mut working,
            false
        ));
        assert_eq!((working.time_08, working.value(0)), (0.4, 0.0));
        // 1.2: the delay's keyframe goes whole (0.2 left), the closing one blends by 0.2 / 0.5.
        assert!(face_gen_advance_keyframe_list(
            0.8,
            &mut list,
            &mut working,
            false
        ));
        assert_eq!(list.len(), 2);
        let ratio = (0.4_f32 + 0.8 - 1.0) / 0.5;
        assert_eq!(working.value(0), 0.0 * (1.0 - ratio) + 1.0 * ratio);
        assert_eq!(working.value(2), 0.0, "an unset value isn't blended in");
        // Past both remaining keyframes: the list empties and the time returns to 0.0.
        face_gen_advance_keyframe_list(2.0, &mut list, &mut working, false);
        assert!(list.is_empty());
        assert_eq!((working.time_08, working.value(0)), (0.0, 0.0));
    }

    #[test]
    fn a_fresh_head_draws_the_blink_and_the_expression_timer() {
        let mut world = World::new();
        world.insert_resource(crate::tes::game_settings::GameSettingCollection::default());
        let state = crate::tes::window::WindowProcedureState::default();
        {
            let mut time = state.time.lock();
            time.previous_ticks_b39db4 = 4096;
            time.ticks_b39db0 = 4112;
        }
        world.insert_resource(state);
        let draws = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let rand = std::rc::Rc::new(std::cell::RefCell::new(
            oblivion_core::crt::MsvcRand::default(),
        ));
        rand.borrow_mut().srand(0x5D94_9CF2);
        let (log, source) = (draws.clone(), rand.clone());
        world.insert_non_send(crate::tes::form_arms::ScriptRandom(
            std::rc::Rc::new(|_: &mut World| 0),
            std::rc::Rc::new(move || {
                let value = source.borrow_mut().rand();
                log.borrow_mut().push(value);
                value
            }),
        ));
        let animation = spawn_face_gen_animation_data(&mut world);
        let node = spawn_face_gen_ni_node(&mut world, b"BSFaceGenNiNodeBiped", animation, true);
        face_gen_node_update(&mut world, node, 0.0, true).unwrap();
        let made = draws.borrow().clone();
        let chance = (f64::from(made[1]) / 32767.0) as f32;
        let want = if 0.3 >= chance { 3 } else { 5 };
        assert_eq!(
            made.len(),
            want,
            "the blink, the chance and the expression's ranges: {made:?}"
        );
        let data = world.get::<BsFaceGenAnimationData>(animation).unwrap();
        assert!(data.expression_timer_1c8 > 0.0, "a new expression hold");
        assert_eq!(data.bytes_1d4[1], 1);
    }
}
