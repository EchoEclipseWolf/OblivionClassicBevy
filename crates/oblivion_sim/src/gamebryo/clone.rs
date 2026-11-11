//! NiObject cloning: `00700610` NiObject_Clone over a `00478B90` cloning process, CreateClone (vt+0x18) then ProcessClone
//! (vt+0x38), for the classes the prison's static models hold.
//!
//! Host form: a cloned object is a new entity with the original's components copied (`EntityWorldMut::clone_and_spawn`, every
//! component here being `Clone`), then each class's CopyMembers stores that differ from a copy are made explicitly: the cold
//! NiAVObject world state, the parent, the property list (re-attached head first, so reversed), the collision object, the child
//! slots, a new material ID, the fade node's fields. A class with no rule here is a stop, so nothing is copied unread.

use std::collections::{BTreeMap, BTreeSet};

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::av_object::{NiAvCollision, NiAvParent, NiAvProperties};
use super::mesh_geometry::NiGeometry;
use super::object_net::NiObjectNet;
use super::properties::{MaterialIdCounter, MaterialProperty};
use super::scene_node::{NiAvWorld, NiNodeChildren, NiNodeEffects};
use super::scene_state::{copy_scene_property_state, merge_property_list, property_type};
use super::stream_objects::NiClass;
use crate::bs::fade_node::BsFadeNode;
use crate::bs::havok_stream::{BhkRigidBody, BhkSpCollisionObject};
use crate::bs::sky_shader::{NiGeometryShader, PrepareGeometryContext};

/// IDA: 00478B90's cloning process: +0x00 the clone map (original to clone, 0x101 buckets), +0x04 the processed map (0x25),
/// +0x08 the copy type (00B3F580, which TES_Constructor sets to 1: names copied exactly), +0x0C the name suffix (00B25648
/// '$', unused at copy type 1), +0x10..+0x18 the scale.
#[derive(Clone, Debug)]
pub struct CloningProcess {
    pub clones_00: BTreeMap<Entity, Entity>,
    pub processed_04: BTreeSet<Entity>,
    pub copy_type_08: u32,
    pub scale_10: [f32; 3],
}

impl Default for CloningProcess {
    /// IDA: N/A Rust's `Default`, 00478B90's values at scale 1.
    fn default() -> Self {
        Self {
            clones_00: BTreeMap::new(),
            processed_04: BTreeSet::new(),
            copy_type_08: 1,
            scale_10: [1.0; 3],
        }
    }
}

/// IDA: the classes whose vt+0x18 is `00700640` (return this): shared by every clone, never entered in the clone map.
const fn is_shared(class: NiClass) -> bool {
    matches!(
        class,
        NiClass::SourceTexture
            | NiClass::TriStripsData
            | NiClass::TriShapeData
            | NiClass::BsxFlagsExtraData
            | NiClass::BsFurnitureMarker
            | NiClass::BsBound
            | NiClass::SkinData
            | NiClass::SkinPartition
            | NiClass::TransformKeyData
            | NiClass::BSplineData
            | NiClass::ColorKeyData
            | NiClass::BoolKeyData
            | NiClass::FloatKeyData
            | NiClass::PositionKeyData
            | NiClass::MorphData
            | NiClass::StringPalette
    )
}

// IDA: N/A the class tag.
fn class_of(world: &World, object: Entity) -> Result<NiClass, GameError> {
    world
        .get::<NiClass>(object)
        .copied()
        .ok_or(GameError::unclassified(
            0x00700610,
            "00700610: a clone of an object with no class",
        ))
}

/// IDA: 00700610 NiObject_Clone(process): vt+0x18 CreateClone, then vt+0x38 ProcessClone on the original. `prepare` is what a
/// geometry's ProcessClone hands its shader's vt+0x18. Completion: 100% for the classes below; any other is a stop.
pub fn clone_object(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<Entity, GameError> {
    let result = create_clone(world, process, original)
        .and_then(|clone| process_clone(world, process, original, prepare).map(|()| clone));
    // A stop leaves no partial clone behind (host rule).
    if result.is_err() {
        for &clone in process.clones_00.values() {
            if world.get_entity(clone).is_ok() {
                world.despawn(clone);
            }
        }
    }
    result
}

// IDA: 00700710: the clone map's entry, else vt+0x18.
pub(super) fn clone_or_mapped(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
) -> Result<Entity, GameError> {
    if let Some(&clone) = process.clones_00.get(&original) {
        return Ok(clone);
    }
    create_clone(world, process, original)
}

/// IDA: vt+0x18 CreateClone: the class's constructor, then its CopyMembers chain, which begins with 00700770 (the clone map's
/// entry). `00700300` NiObjectNET: the name at copy type 1, each extra (vt+0x50 true for the bound extras) through its vt+0x18
/// and AddExtraDataWithGeneratedName, the controller head through its vt+0x18. `00707E90` NiAVObject: the +0x18 dword and the
/// 52 transform bytes from +0x30 (copied), each property below type 10 through 00700710 and AttachProperty (AddHead), the
/// collision object through its vt+0x18 and NiAVObject_SetCollisionObject. `0070AC60` NiNode: the child slots' count, and each
/// nonnull child's vt+0x18 through vt+0x90 SetChildAt(i). `004A0F90` BSFadeNode: +0xE0, +0xE4, +0xEC, +0xDC copied (+0xE8 keeps
/// the constructor's 1.0), then vt+0x78 and vt+0x50. `00723020` NiGeometry: vt+0x8C SetModelData shares the data. `007097B0`
/// NiMaterialProperty: a new material ID. `004B2650` TESObjectExtraData: the name and +0x0C copied. `008976D0` bhkNiCollisionObject: the +0x0C flags, the body through 00700710.
/// `008A4E30` bhkRigidBody: `0089F5D0` keeps the shape when its vt+0x8C (`008A2760`: the process scale equals 00B258F4) says
/// shareable, +0x18 = (the original's | the new one's) & ~0x38, the constraints each through vt+0x18; at another scale the shape
/// is copied and both carry `BhkHostScale` (host substitution D2). Completion: 100% for these classes; a skinned geometry, a node with effects, the other controllers are stops.
pub fn create_clone(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
) -> Result<Entity, GameError> {
    let class = class_of(world, original)?;
    if is_shared(class) {
        return Ok(original);
    }
    if !matches!(
        class,
        NiClass::Node
            | NiClass::BillboardNode
            | NiClass::TriStrips
            | NiClass::TriShape
            | NiClass::MaterialProperty
            | NiClass::PpLightingProperty
            | NiClass::Lighting30Property
            | NiClass::TexturingProperty
            | NiClass::VertexColorProperty
            | NiClass::AlphaProperty
            | NiClass::ZBufferProperty
            | NiClass::StencilProperty
            | NiClass::SpecularProperty
            | NiClass::StringExtraData
            | NiClass::BinaryExtraData
            | NiClass::TextKeyExtraData
            | NiClass::FadeNodeMaxAlphaExtraData
            | NiClass::TesObjectExtraData
            | NiClass::BhkCollisionObject
            | NiClass::BhkSpCollisionObject
            | NiClass::BhkBlendCollisionObject
            | NiClass::BhkBlendCollisionAddRotation
            | NiClass::BhkMalleableConstraint
            | NiClass::BoneLodController
            | NiClass::BhkBlendController
            | NiClass::BhkRigidBody
            | NiClass::BhkRigidBodyT
            | NiClass::BhkRagdollConstraint
            | NiClass::BhkLimitedHingeConstraint
            | NiClass::BhkPrismaticConstraint
            | NiClass::AlphaController
            | NiClass::MaterialColorController
            | NiClass::TextureTransformController
            | NiClass::Point3Interpolator
            | NiClass::FloatInterpolator
            | NiClass::TransformController
            | NiClass::TransformInterpolator
            | NiClass::BSplineCompTransformInterpolator
            | NiClass::BSplineCompFloatInterpolator
            | NiClass::BSplineBasisData
            | NiClass::BhkSimpleShapePhantom
            | NiClass::GeomMorpherController
            | NiClass::ControllerManager
            | NiClass::ControllerSequence
            | NiClass::DefaultAvObjectPalette
            | NiClass::MultiTargetTransformController
            | NiClass::BlendTransformInterpolator
            | NiClass::BlendBoolInterpolator
            | NiClass::BlendFloatInterpolator
            | NiClass::BlendPoint3Interpolator
            | NiClass::BoolInterpolator
            | NiClass::BoolTimelineInterpolator
            | NiClass::VisController
            | NiClass::ParticleSystem
            | NiClass::ParticleData
            | NiClass::ParticlePositionModifier
            | NiClass::ParticleGrowFadeModifier
            | NiClass::ParticleAgeDeathModifier
            | NiClass::ParticleBoundUpdateModifier
            | NiClass::ParticleColorModifier
            | NiClass::ParticleGravityModifier
            | NiClass::ParticleSpawnModifier
            | NiClass::ParticleBoxEmitter
            | NiClass::ParticleEmitterController
            | NiClass::ParticleUpdateController
            | NiClass::ParticleDragModifier
            | NiClass::ParticleRotationModifier
            | NiClass::ParticleMeshEmitter
            | NiClass::ParticleModifierActiveController
    ) {
        return Err(GameError::from_legacy_message(NO_CLONE_RULE));
    }
    let clone = world.entity_mut(original).clone_and_spawn();
    // A component that isn't `Clone` would be left out silently.
    if world.entity(clone).archetype().component_count()
        != world.entity(original).archetype().component_count()
    {
        return Err(GameError::unclassified(
            0x00700610,
            "00700610: a component the host clone can't copy",
        ));
    }
    process.clones_00.insert(original, clone);
    // 00700300 NiObjectNET: extras and the controller head.
    if let Some(net) = world.get::<NiObjectNet>(original).cloned() {
        let mut extras = Vec::with_capacity(net.extra_data.len());
        for extra in net.extra_data {
            extras.push(create_clone(world, process, extra)?);
        }
        let head = match net.controller_0c {
            Some(head) => Some(create_clone(world, process, head)?),
            None => None,
        };
        world
            .get_mut::<NiObjectNet>(clone)
            .expect("copied")
            .controller_0c = head;
        world
            .get_mut::<NiObjectNet>(clone)
            .expect("copied")
            .extra_data = extras;
    }
    if class.is_av_object() {
        // NiAVObject_Constructor's world state and parent; the +0x18 dword and the local transform are copied.
        world
            .entity_mut(clone)
            .insert((NiAvWorld::default(), NiAvParent(None)));
        // 00707E40: the properties below type 10, head first, each AddHead.
        let originals = world
            .get::<NiAvProperties>(original)
            .map(|properties| properties.0.clone())
            .unwrap_or_default();
        let mut properties: Vec<Option<Entity>> = Vec::new();
        for property in originals {
            if property_type(world, property)? < 10 {
                let property = property.expect("property_type rejects null");
                properties.insert(0, Some(clone_or_mapped(world, process, property)?));
            }
        }
        world.entity_mut(clone).insert(NiAvProperties(properties));
        // 00707E90: the collision object's vt+0x18, then NiAVObject_SetCollisionObject.
        world.entity_mut(clone).remove::<NiAvCollision>();
        if let Some(collision) = world
            .get::<NiAvCollision>(original)
            .map(|collision| collision.0)
        {
            let copy = create_clone(world, process, collision)?;
            world.entity_mut(clone).insert(NiAvCollision(copy));
            if let Some(mut object) = world.get_mut::<BhkSpCollisionObject>(copy)
                && object.target_08 != Some(clone)
            {
                object.target_08 = Some(clone);
            }
        }
    }
    match class {
        NiClass::Node | NiClass::BillboardNode => {
            // 007223E0 NiBillboardNode: NiNode_CopyMembers, +0xDC copied, +0xE0 the constructor's 0.0.
            if let Some(mut billboard) = world.get_mut::<super::scene_derived::BillboardNode>(clone)
            {
                billboard.unknown_e0 = 0.0;
            }
            if world
                .get::<NiNodeEffects>(original)
                .is_some_and(|effects| !effects.0.is_empty())
            {
                return Err(GameError::unclassified(
                    0x0070BA00,
                    "0070BA00: a clone of a node with dynamic effects",
                ));
            }
            let slots = world
                .get::<NiNodeChildren>(original)
                .expect("a NiNode has child slots")
                .slots_ac
                .clone();
            world
                .get_mut::<NiNodeChildren>(clone)
                .expect("copied")
                .slots_ac = vec![None; slots.len()];
            for (index, child) in slots.into_iter().enumerate() {
                let Some(child) = child else {
                    continue;
                };
                let copy = create_clone(world, process, child)?;
                crate::gamebryo::scene_node::set_child_at(world, clone, index, Some(copy));
            }
            if let Some(fade) = world.get::<BsFadeNode>(original).copied() {
                world.entity_mut(clone).insert(BsFadeNode {
                    current_alpha_e8: 1.0,
                    ..fade
                });
                super::scene_update::update_node_world_bound(world, clone)?;
                super::scene_update::update_scene_node_model_bound(world, clone);
            }
        }
        NiClass::TriStrips | NiClass::TriShape => {
            let mut geometry = world.get_mut::<NiGeometry>(clone).expect("copied");
            // 00723050 replaces +0xB8 (the skin instance) and rebuilds +0xAC with the shader; +0xB0 stays the constructor's null.
            geometry.property_state_ac = None;
            geometry.effect_state_b0 = None;
        }
        // 007D9810 / 00863720 construct a fresh lighting property. Its base copy
        // (007ECB10) leaves the constructor's light list and cursor intact.
        NiClass::PpLightingProperty | NiClass::Lighting30Property => {
            let mut property = world
                .get_mut::<super::properties::LightingProperty>(clone)
                .expect("copied shader property");
            property.classifier_68 = 0;
            property.last_pass_state_24 = 0;
            property.lights_6c.clear();
            property.next_light_7c = None;
        }
        NiClass::MaterialProperty => {
            let mut counter = world.get_resource_or_insert_with(MaterialIdCounter::default);
            counter.0 = counter.0.wrapping_add(1);
            let id = counter.0;
            world
                .get_mut::<MaterialProperty>(clone)
                .expect("copied")
                .material_id_18 = id;
        }
        // 0088EC10 bhkBlendCollisionObject and 0088E950 its AddRotation subclass: the constructor (00BA7A1C + 1; +0x1C 8, +0x24
        // 0.0), bhkNiCollisionObject_CopyMembers (the body below), then +0x18, +0x14 and +0x20 (and the AddRotation's +0x28
        // rotation) copied.
        NiClass::BhkBlendCollisionObject | NiClass::BhkBlendCollisionAddRotation => {
            world
                .get_resource_or_init::<crate::bs::havok_stream::BlendCollisionStatics>()
                .count_ba7a1c += 1;
            {
                let mut blend = world
                    .get_mut::<crate::bs::havok_stream::BhkBlendCollisionObject>(clone)
                    .expect("copied");
                blend.motion_type_1c = 8;
                blend.float_24 = 0;
            }
            let body = world
                .get::<BhkSpCollisionObject>(original)
                .and_then(|object| object.body_10);
            let body = match body {
                Some(body) => Some(clone_or_mapped(world, process, body)?),
                None => None,
            };
            world
                .get_mut::<BhkSpCollisionObject>(clone)
                .expect("copied")
                .body_10 = body;
        }
        // 006FD660 NiBSBoneLODController and 008AB9B0 bhkBlendController: 00715D80's time fields, then +0x3C and +0x40 (the LOD
        // controller) or +0x3C and the keys (the blend controller); the LOD groups wait for the ProcessClone.
        NiClass::BoneLodController | NiClass::BhkBlendController => {
            let source = world
                .get::<super::transform_controller::NodeController>(original)
                .expect("a time controller")
                .time
                .clone();
            let copied = copied_time(world, process, &source)?;
            world
                .get_mut::<super::transform_controller::NodeController>(clone)
                .expect("copied")
                .time = copied;
            if let Some(mut groups) = world.get_mut::<super::bone_lod::BoneLodController>(clone) {
                groups.groups_44 = Vec::new();
            }
        }
        NiClass::BhkCollisionObject | NiClass::BhkSpCollisionObject => {
            let body = world
                .get::<BhkSpCollisionObject>(original)
                .and_then(|object| object.body_10);
            let body = match body {
                Some(body) => Some(clone_or_mapped(world, process, body)?),
                None => None,
            };
            world
                .get_mut::<BhkSpCollisionObject>(clone)
                .expect("copied")
                .body_10 = body;
        }
        NiClass::BhkRigidBody | NiClass::BhkRigidBodyT => {
            // 0089F5D0: at a process scale other than (1, 1, 1) the shape isn't shareable (`008A2760`) and is cloned.
            if process.scale_10 != [1.0; 3] {
                let shape = world.get::<BhkRigidBody>(clone).expect("copied").shape;
                if let Some(shape) = shape {
                    let copy = world.entity_mut(shape).clone_and_spawn();
                    world
                        .entity_mut(copy)
                        .insert(crate::bs::havok_stream::BhkHostScale(process.scale_10[0]));
                    world.get_mut::<BhkRigidBody>(clone).expect("copied").shape = Some(copy);
                }
                world
                    .entity_mut(clone)
                    .insert(crate::bs::havok_stream::BhkHostScale(process.scale_10[0]));
            }
            // The new body's constructor (008A4150) leaves +0x18 zero.
            let originals = {
                let mut body = world.get_mut::<BhkRigidBody>(clone).expect("copied");
                body.field_18 &= !0x38;
                std::mem::take(&mut body.constraints_10)
            };
            // 008A4E30: each constraint from the head through its vt+0x18 (no map lookup), then 008A46C0's push to the front.
            let mut constraints = Vec::with_capacity(originals.len());
            for constraint in originals {
                constraints.insert(0, create_clone(world, process, constraint)?);
            }
            world
                .get_mut::<BhkRigidBody>(clone)
                .expect("copied")
                .constraints_10 = constraints;
        }
        // 008A0860 bhkConstraint_CopyMembers: the data copied (a process scale other than 1 scales it through the cinfo's
        // vt+0: host substitution D2, `BhkHostScale`); +0x0C's entity takes its clone from the map alone, else null; with the
        // original +0x0C set, +0x10's entity comes through the map or its vt+0x18.
        // 008BF050 bhkMalleableConstraint: its 0x10 constructor (the bhkConstraint and malleable counts 00BA7D4C and 00BA8088, which
        // the port doesn't keep), then bhkConstraint_CopyMembers, as the ragdoll's. 008C1840 bhkPrismaticConstraint's is the same
        // shape (count 00BA80C4).
        NiClass::BhkRagdollConstraint
        | NiClass::BhkLimitedHingeConstraint
        | NiClass::BhkPrismaticConstraint
        | NiClass::BhkMalleableConstraint => {
            let source = world
                .get::<crate::bs::havok_stream::BhkConstraint>(original)
                .expect("a constraint")
                .clone();
            let entity_a = source
                .entity_a
                .and_then(|entity| process.clones_00.get(&entity).copied());
            let entity_b = match (source.entity_a, source.entity_b) {
                (Some(_), Some(entity)) => Some(clone_or_mapped(world, process, entity)?),
                _ => None,
            };
            let mut constraint = world
                .get_mut::<crate::bs::havok_stream::BhkConstraint>(clone)
                .expect("copied");
            constraint.entity_a = entity_a;
            constraint.entity_b = entity_b;
            if process.scale_10 != [1.0; 3] {
                world
                    .entity_mut(clone)
                    .insert(crate::bs::havok_stream::BhkHostScale(process.scale_10[0]));
            }
        }
        NiClass::AlphaController
        | NiClass::MaterialColorController
        | NiClass::TextureTransformController
        | NiClass::TransformController
        | NiClass::VisController => {
            clone_time_controller(world, process, original, clone)?;
        }
        NiClass::ControllerManager => {
            super::clone_animation::clone_manager(world, process, original, clone)?;
        }
        NiClass::ParticleSystem => {
            super::clone_particles::clone_system(world, process, original, clone)?;
        }
        NiClass::ParticleData => super::clone_particles::clone_data(world, original, clone),
        NiClass::ParticlePositionModifier
        | NiClass::ParticleGrowFadeModifier
        | NiClass::ParticleAgeDeathModifier
        | NiClass::ParticleBoundUpdateModifier
        | NiClass::ParticleColorModifier
        | NiClass::ParticleGravityModifier
        | NiClass::ParticleSpawnModifier
        | NiClass::ParticleBoxEmitter
        | NiClass::ParticleDragModifier
        | NiClass::ParticleRotationModifier
        | NiClass::ParticleMeshEmitter => {
            super::clone_particles::clone_modifier(world, class, clone)?
        }
        NiClass::ParticleModifierActiveController => {
            super::clone_particles::clone_active_controller(world, process, original, clone)?;
        }
        NiClass::ParticleEmitterController => {
            super::clone_particles::clone_emitter_controller(world, process, original, clone)?;
        }
        NiClass::ParticleUpdateController => {
            super::clone_particles::clone_update_controller(world, process, original, clone)?;
        }
        NiClass::ControllerSequence => {
            super::clone_animation::clone_sequence(world, process, original, clone)?;
        }
        NiClass::DefaultAvObjectPalette => super::clone_animation::clone_palette(world, clone),
        NiClass::MultiTargetTransformController => {
            super::clone_animation::clone_multi_target(world, process, original, clone)?;
        }
        NiClass::BlendTransformInterpolator
        | NiClass::BlendBoolInterpolator
        | NiClass::BlendFloatInterpolator
        | NiClass::BlendPoint3Interpolator => {
            super::clone_animation::clone_blend(world, process, original, clone)?;
        }
        NiClass::BoolInterpolator | NiClass::BoolTimelineInterpolator => {
            super::clone_animation::clone_bool_interpolator(world, original, clone);
        }
        NiClass::GeomMorpherController => clone_geom_morpher(world, process, original, clone)?,
        // 006D68F0 / 006D6660: the 32 bytes from +0x0C (the value), the shared +0x2C data and the three key indices copied; +0x08's
        // last time is the constructor's -FLT_MAX.
        // 006E5B40 / 006E6170 (and 006E78F0 for the basis: all 32 bytes, nothing more).
        NiClass::BSplineCompTransformInterpolator | NiClass::BSplineCompFloatInterpolator => {
            super::bspline::fix_clone(world, process, clone)?;
        }
        NiClass::TransformInterpolator => {
            world
                .get_mut::<super::transform_animation::TransformInterpolator>(clone)
                .expect("copied")
                .last_time_08 = super::transform_animation::INVALID_TRANSFORM_COMPONENT;
        }
        // 008AF0E0: 008AF020's new phantom, then 0089F5D0 bhkWorldObject_CopyMembers: the cinfo copied, the shape kept when its
        // vt+0x8C says it is shareable at the process scale (1) and otherwise cloned (`BhkHostScale`, D2), then 0089D610 the
        // hk object, which the host doesn't build.
        NiClass::BhkSimpleShapePhantom => {
            if process.scale_10 != [1.0; 3] {
                let shape = world
                    .get::<crate::bs::havok_stream::BhkSimpleShapePhantom>(clone)
                    .expect("copied")
                    .shape;
                if let Some(shape) = shape {
                    let copy = world.entity_mut(shape).clone_and_spawn();
                    world
                        .entity_mut(copy)
                        .insert(crate::bs::havok_stream::BhkHostScale(process.scale_10[0]));
                    world
                        .get_mut::<crate::bs::havok_stream::BhkSimpleShapePhantom>(clone)
                        .expect("copied")
                        .shape = Some(copy);
                }
                world
                    .entity_mut(clone)
                    .insert(crate::bs::havok_stream::BhkHostScale(process.scale_10[0]));
            }
        }
        // 006DA6B0 / 006D2D50: the value and the shared key data copied; +0x08's last time is the constructor's -FLT_MAX.
        NiClass::FloatInterpolator => {
            world
                .get_mut::<super::animation::FloatInterpolator>(clone)
                .expect("copied")
                .last_time_08 = super::animation::INVALID_FLOAT_INTERPOLATOR_VALUE;
        }
        _ => {}
    }
    Ok(clone)
}

/// IDA: `00715D80` NiTimeController's CopyMembers: +0x08..+0x24 copied (+0x28 and the bytes after keep the constructor's),
/// +0x30 the target's clone when the clone map holds it (else null, with no SetTarget), +0x34 the next controller's
/// vt+0x18; `006CE2C0` NiSingleInterpController: +0x3C through 00700710. `006D7290` NiTextureTransformController copies +0x40
/// and its map fields; its +0x44 map stays the constructor's null. Completion: 100% for the material and texture-transform
/// controllers.
fn clone_time_controller(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    clone: Entity,
) -> Result<(), GameError> {
    let time = if let Some(controller) =
        world.get::<super::material_controllers::MaterialController>(original)
    {
        (controller.time.clone(), controller.interpolator_3c)
    } else if let Some(controller) =
        world.get::<super::transform_controller::NodeController>(original)
    {
        let interpolator =
            match world.get::<super::transform_controller::TransformController>(original) {
                Some(transform) => transform.interpolator_3c,
                None => {
                    world
                        .get::<super::transform_controller::VisController>(original)
                        .ok_or(GameError::unclassified(
                            0x006C3E80,
                            "006C3E80: a node controller with no interpolator slot",
                        ))?
                        .interpolator_3c
                }
            };
        (controller.time.clone(), interpolator)
    } else {
        let controller = world
            .get::<super::texture::TextureController>(original)
            .ok_or(GameError::unclassified(
                0x00715D80,
                "00715D80: a controller with no time state",
            ))?;
        (controller.time.clone(), controller.interpolator_3c)
    };
    let (source, interpolator) = time;
    let copied = copied_time(world, process, &source)?;
    let interpolator = match interpolator {
        Some(interpolator) => Some(clone_or_mapped(world, process, interpolator)?),
        None => None,
    };
    if let Some(mut controller) =
        world.get_mut::<super::material_controllers::MaterialController>(clone)
    {
        controller.time = copied;
        controller.interpolator_3c = interpolator;
        return Ok(());
    }
    if let Some(mut controller) =
        world.get_mut::<super::transform_controller::NodeController>(clone)
    {
        controller.time = copied;
        if let Some(mut transform) =
            world.get_mut::<super::transform_controller::TransformController>(clone)
        {
            transform.interpolator_3c = interpolator;
        } else {
            world
                .get_mut::<super::transform_controller::VisController>(clone)
                .expect("copied")
                .interpolator_3c = interpolator;
        }
        return Ok(());
    }
    let mut controller = world
        .get_mut::<super::texture::TextureController>(clone)
        .expect("copied");
    controller.time = copied;
    controller.interpolator_3c = interpolator;
    if let Some(mut transform) = world.get_mut::<super::texture::TextureTransformController>(clone)
    {
        transform.map_44 = None;
    }
    Ok(())
}

/// IDA: 006D1750 NiGeomMorpherController's CreateClone: a new 0x5C controller (00715C10's NiInterpController, an empty +0x40
/// weight array), then 006D1480: `00715D80`'s time fields (the target through the clone map), +0x3C, and with +0x50 data
/// 006D11F0(data, target nonnull): the data taken, `NiGeomMorpherController_AllocateInterpolatorSlots` (the morph count's
/// null slots and zero weights), with a target vt+0xA8 (006D07B0: the target's geometry data's +0x0C bound is
/// `0072A0F0` of morph 0's vectors, merged (`0072A6B0`) with each later morph's); `00B3CE18` is 0 at runtime (run-af), so
/// +0x5A keeps the copy; then each slot's interpolator through `00700710` and vt+0x84. Completion: 100%; the bound's steps
/// run at the host's float precision where the clone thread's control word isn't captured.
fn clone_geom_morpher(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    clone: Entity,
) -> Result<(), GameError> {
    let source = world
        .get::<super::morph::GeomMorpherController>(original)
        .expect("a NiGeomMorpherController")
        .clone();
    let target = source
        .time
        .target_30
        .and_then(|target| process.clones_00.get(&target).copied());
    let next = match source.time.next_34 {
        Some(next) => Some(create_clone(world, process, next)?),
        None => None,
    };
    let morphs = source
        .data_50
        .and_then(|data| world.get::<super::morph::MorphData>(data))
        .map_or(0, |data| data.targets_10.len());
    {
        let mut controller = world
            .get_mut::<super::morph::GeomMorpherController>(clone)
            .expect("copied");
        let cold = super::particle_controller::ParticleTimeControllerCold::default();
        controller.time = super::particle_controller::ParticleTimeControllerCold {
            fields: source.time.fields,
            target_linked: target.is_some(),
            next_linked: next.is_some(),
            target_30: target,
            next_34: next,
            pending_next_index: source.time.pending_next_index,
            pending_target_index: source.time.pending_target_index,
            ..cold
        };
        controller.interpolators_54 = vec![None; morphs];
        controller.weights_40 = vec![0.0; morphs];
    }
    if let (Some(data), Some(target)) = (source.data_50, target) {
        morph_target_bound(world, data, target)?;
    }
    for (index, interpolator) in source.interpolators_54.into_iter().enumerate().take(morphs) {
        let copy = match interpolator {
            Some(interpolator) => Some(clone_or_mapped(world, process, interpolator)?),
            None => None,
        };
        world
            .get_mut::<super::morph::GeomMorpherController>(clone)
            .expect("copied")
            .interpolators_54[index] = copy;
    }
    Ok(())
}

// IDA: 006D07B0 NiGeomMorpherController vt+0xA8, for a target geometry: its +0xB4 data's +0x0C bound from the morphs' vectors.
fn morph_target_bound(world: &mut World, data: Entity, target: Entity) -> Result<(), GameError> {
    let morph = world
        .get::<super::morph::MorphData>(data)
        .expect("a NiMorphData")
        .clone();
    let count = morph.vertex_count_0c as usize;
    let vectors = |index: usize| -> Vec<[f32; 3]> {
        morph.targets_10[index]
            .vector_bits
            .iter()
            .map(|bits| bits.map(f32::from_bits))
            .collect()
    };
    let bound = if morph.targets_10.is_empty() {
        [0.0; 4]
    } else {
        let (center, radius) =
            super::screen_elements::compute_bound_from_vertices(&vectors(0), count);
        let mut bound = [center[0], center[1], center[2], radius];
        for index in 1..morph.targets_10.len() {
            let (center, radius) =
                super::screen_elements::compute_bound_from_vertices(&vectors(index), count);
            super::particle_dynamics::merge_ni_sphere(
                &mut bound,
                &[center[0], center[1], center[2], radius],
            )?;
        }
        bound
    };
    let Some(geometry_data) = world
        .get::<NiGeometry>(target)
        .and_then(|geometry| geometry.data_b4)
    else {
        return Err(GameError::unclassified(
            0x006D0848,
            "006D0848: a morpher target with no geometry data",
        ));
    };
    if let Some(mut data) = world.get_mut::<super::mesh_geometry::MeshTriShapeData>(geometry_data) {
        data.base.bounds_0c = bound;
    } else if let Some(mut data) =
        world.get_mut::<super::mesh_tri_strips::MeshTriStripsData>(geometry_data)
    {
        data.base.bounds_0c = bound;
    } else {
        return Err(GameError::unclassified(
            0x006D0848,
            "006D0848: a morpher target's data isn't NiTriShapeData or NiTriStripsData",
        ));
    }
    Ok(())
}

/// IDA: `00715D80`'s time fields for a clone: +0x08..+0x24 copied (+0x28 and after the constructor's), +0x30 the target's
/// clone when the clone map holds it (else null, with no SetTarget), +0x34 the next controller's vt+0x18. Completion: 100%.
pub(super) fn copied_time(
    world: &mut World,
    process: &mut CloningProcess,
    source: &super::particle_controller::ParticleTimeControllerCold,
) -> Result<super::particle_controller::ParticleTimeControllerCold, GameError> {
    let target = source
        .target_30
        .and_then(|target| process.clones_00.get(&target).copied());
    let next = match source.next_34 {
        Some(next) => Some(create_clone(world, process, next)?),
        None => None,
    };
    let cold = super::particle_controller::ParticleTimeControllerCold::default();
    Ok(super::particle_controller::ParticleTimeControllerCold {
        fields: source.fields,
        target_linked: target.is_some(),
        next_linked: next.is_some(),
        target_30: target,
        next_34: next,
        pending_next_index: source.pending_next_index,
        pending_target_index: source.pending_target_index,
        ..cold
    })
}

// IDA: N/A the stop for a class with no clone rule.
const NO_CLONE_RULE: &str = "00700610: a clone of a class with no rule";

/// IDA: vt+0x38 ProcessClone on the original: `00700750` marks the processed map; `006FFE10` NiObjectNET walks its extras;
/// `00707AB0` NiAVObject its unprocessed properties and its collision object; `0070BA00` NiNode its children; `00723050`
/// NiGeometry: NiGeometry_SetShader(clone, shader) and the shader's vt+0x18 on the clone, then +0xAC = 007077D0(the clone's list,
/// a 00731620 copy of the original's +0xAC, 0) under 00B3FA00. Completion: 100% for the classes `create_clone` takes.
pub(super) fn process_clone(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<(), GameError> {
    let class = class_of(world, original)?;
    process.processed_04.insert(original);
    if is_shared(class) {
        return Ok(());
    }
    let clone = *process
        .clones_00
        .get(&original)
        .ok_or(GameError::unclassified(
            0x00700750,
            "00700750: ProcessClone of an object never cloned",
        ))?;
    if let Some(net) = world.get::<NiObjectNet>(original).cloned() {
        for extra in net.extra_data {
            process.processed_04.insert(extra);
        }
        // 006FFE10: the controller head's vt+0x38.
        if let Some(head) = net.controller_0c {
            process_clone(world, process, head, prepare)?;
        }
    }
    if class.is_av_object() {
        let properties = world
            .get::<NiAvProperties>(original)
            .map(|properties| properties.0.clone())
            .unwrap_or_default();
        for property in properties.into_iter().flatten() {
            process.processed_04.insert(property);
        }
        if let Some(collision) = world
            .get::<NiAvCollision>(original)
            .map(|collision| collision.0)
        {
            process.processed_04.insert(collision);
            if let Some(body) = world
                .get::<BhkSpCollisionObject>(collision)
                .and_then(|object| object.body_10)
            {
                process.processed_04.insert(body);
            }
        }
    }
    match class {
        NiClass::Node | NiClass::BillboardNode => {
            let children = world
                .get::<NiNodeChildren>(original)
                .expect("a NiNode has child slots")
                .slots_ac
                .clone();
            for child in children.into_iter().flatten() {
                process_clone(world, process, child, prepare)?;
            }
        }
        NiClass::ControllerManager => {
            super::clone_animation::process_manager(world, process, original, prepare)?;
        }
        // 007495E0: NiGeometry's ProcessClone (the shader and the merged property state), then the system's.
        NiClass::ParticleSystem => {
            if let Some(shader) = world
                .get::<NiGeometryShader>(original)
                .map(|shader| shader.0.clone())
            {
                world
                    .entity_mut(clone)
                    .insert(NiGeometryShader(shader.clone()));
                crate::bs::sky_shader::shader_slot_18(world, shader.as_ref(), clone, prepare)?;
            }
            let state = world
                .get::<super::particle_system::ParticleSystem>(original)
                .and_then(|system| system.property_state_ac);
            let copy = state.map(|state| copy_scene_property_state(world, state));
            let properties = world
                .get::<NiAvProperties>(clone)
                .map(|properties| properties.0.clone())
                .unwrap_or_default();
            let merged = merge_property_list(world, &properties, copy, false)?;
            if let Some(mut state) = merged
                .and_then(|merged| world.get_mut::<super::scene_state::ScenePropertyState>(merged))
            {
                state.host_references += 1;
            }
            world
                .get_mut::<super::particle_system::ParticleSystem>(clone)
                .expect("copied")
                .property_state_ac = merged;
            super::clone_particles::process_system(world, process, original, prepare)?;
        }
        NiClass::ParticleEmitterController => {
            super::clone_particles::process_emitter_controller(
                world, process, original, clone, prepare,
            )?;
        }
        NiClass::ParticleUpdateController => {
            super::clone_animation::process_next(world, process, original, prepare)?;
        }
        NiClass::ParticleModifierActiveController => {
            super::clone_particles::process_active_controller(
                world, process, original, clone, prepare,
            )?;
        }
        NiClass::MultiTargetTransformController => {
            super::clone_animation::process_multi_target(world, process, original, prepare)?;
        }
        NiClass::BlendTransformInterpolator
        | NiClass::BlendBoolInterpolator
        | NiClass::BlendFloatInterpolator
        | NiClass::BlendPoint3Interpolator => {
            super::clone_animation::process_blend(world, process, original, clone, prepare)?;
        }
        // 008AB9B0's controller takes 00715E40 alone; 006FD910 NiBSBoneLODController's: 00715E40, then each group rebuilt on the
        // clone from the original's nodes through the clone map (a node not there, or a null one, becomes null), then the
        // clone's +0x3C taken, reset to -1 and handed to 006FD5D0 SetLOD.
        NiClass::BhkBlendController => {
            super::clone_animation::process_next(world, process, original, prepare)?;
        }
        NiClass::BoneLodController => {
            super::clone_animation::process_next(world, process, original, prepare)?;
            let groups: Vec<Vec<Option<Entity>>> = world
                .get::<super::bone_lod::BoneLodController>(original)
                .expect("a NiBSBoneLODController")
                .groups_44
                .iter()
                .map(|group| {
                    group
                        .iter()
                        .map(|node| node.and_then(|node| process.clones_00.get(&node).copied()))
                        .collect()
                })
                .collect();
            let lod = {
                let mut owner = world
                    .get_mut::<super::bone_lod::BoneLodController>(clone)
                    .expect("copied");
                owner.groups_44 = groups;
                let lod = owner.lod_3c;
                owner.lod_3c = -1;
                lod
            };
            super::bone_lod::set_bone_lod(world, clone, lod);
        }
        // 006CE2F0 NiSingleInterpController's ProcessClone: 00715E40, then the +0x3C interpolator's vt+0x38.
        NiClass::TransformController
        | NiClass::VisController
        | NiClass::AlphaController
        | NiClass::MaterialColorController
        | NiClass::TextureTransformController => {
            super::clone_animation::process_next(world, process, original, prepare)?;
            let interpolator = super::clone_animation::single_interpolator(world, original);
            if let Some(interpolator) = interpolator {
                process_clone(world, process, interpolator, prepare)?;
            }
        }
        // 006D08D0: 00715E40, then each interpolator slot's vt+0x38.
        NiClass::GeomMorpherController => {
            super::clone_animation::process_next(world, process, original, prepare)?;
            let slots = world
                .get::<super::morph::GeomMorpherController>(original)
                .expect("a NiGeomMorpherController")
                .interpolators_54
                .clone();
            for interpolator in slots.into_iter().flatten() {
                process_clone(world, process, interpolator, prepare)?;
            }
        }
        NiClass::TriStrips | NiClass::TriShape => {
            // 00723050: +0xB8 through the clone map, else its vt+0x18 and vt+0x38.
            if let Some(skin) = world
                .get::<NiGeometry>(original)
                .and_then(|geometry| geometry.skin_b8)
            {
                let copy = match process.clones_00.get(&skin) {
                    Some(&copy) => copy,
                    None => {
                        let copy = clone_skin_instance(world, process, skin)?;
                        process.processed_04.insert(skin);
                        copy
                    }
                };
                world.get_mut::<NiGeometry>(clone).expect("copied").skin_b8 = Some(copy);
            }
            let Some(shader) = world
                .get::<NiGeometryShader>(original)
                .map(|shader| shader.0.clone())
            else {
                return Ok(());
            };
            world
                .entity_mut(clone)
                .insert(NiGeometryShader(shader.clone()));
            crate::bs::sky_shader::shader_slot_18(world, shader.as_ref(), clone, prepare)?;
            let state = world
                .get::<NiGeometry>(original)
                .and_then(|geometry| geometry.property_state_ac);
            let copy = state.map(|state| copy_scene_property_state(world, state));
            let properties = world
                .get::<NiAvProperties>(clone)
                .map(|properties| properties.0.clone())
                .unwrap_or_default();
            let merged = merge_property_list(world, &properties, copy, false)?;
            // NiRefObjectPtr_CopyAssign on +0xAC.
            if let Some(mut state) = merged
                .and_then(|merged| world.get_mut::<super::scene_state::ScenePropertyState>(merged))
            {
                state.host_references += 1;
            }
            world
                .get_mut::<NiGeometry>(clone)
                .expect("copied")
                .property_state_ac = merged;
        }
        _ => {}
    }
    Ok(())
}

/// IDA: 0072BFA0 NiSkinInstance's CreateClone: when neither the +0x10 root parent nor a +0x14 bone is in the clone map
/// (`0072BCA0`), the instance itself; otherwise a cold 0x2C instance (+0x18 = -1) whose `0072BC00` takes the +0x08 data and
/// +0x0C partition (shared), +0x10 the root's clone or the root, and +0x14 each bone's clone or the bone (the data's +0x40
/// count). Completion: 100%.
fn clone_skin_instance(
    world: &mut World,
    process: &mut CloningProcess,
    skin: Entity,
) -> Result<Entity, GameError> {
    let instance = world
        .get::<super::mesh_skin_owners::MeshSkinInstance>(skin)
        .ok_or(GameError::unclassified(
            0x0072BFA0,
            "0072BFA0: a skin with no NiSkinInstance",
        ))?
        .clone();
    let mapped =
        |entity: Option<Entity>| entity.and_then(|entity| process.clones_00.get(&entity).copied());
    if mapped(instance.root_parent_owner_10).is_none()
        && instance
            .bone_owners_14
            .iter()
            .all(|&bone| mapped(bone).is_none())
    {
        return Ok(skin);
    }
    let root = mapped(instance.root_parent_owner_10).or(instance.root_parent_owner_10);
    let bones = instance
        .bone_owners_14
        .iter()
        .map(|&bone| mapped(bone).or(bone))
        .collect();
    let copy = world.entity_mut(skin).clone_and_spawn();
    process.clones_00.insert(skin, copy);
    let mut cloned = world
        .get_mut::<super::mesh_skin_owners::MeshSkinInstance>(copy)
        .expect("copied");
    cloned.root_parent_owner_10 = root;
    cloned.bone_owners_14 = bones;
    cloned.serial_18 = -1;
    cloned.cold_tail_words_1c = [0; 4];
    Ok(copy)
}
