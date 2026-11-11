//! The Ni factory registry: 00714710's 59-bucket string map, name to original constructor.
//!
//! Original constructors are identities, never called. `create` spawns the class's cold components, or reports
//! `Unsupported` for a class not ported yet.
//!
//! Also here, from the same C++ file: the startup callback scheduler and its recorded list, the process singleton
//! defaults, the singleton tail, the collision dispatch tables and the shader creator registry. Every C++ member of the
//! one `NiFactoryRegistry` object is a resource of its own, and a callback reaches them through `&mut World`. The C++
//! copy-and-commit (`auto updated = *this`) guards against a throwing allocation partway; here that allocation aborts,
//! so each callback mutates in place.
//!
//! The C++ `NiPropertyState` and `NiPropertySceneNode` are `scene_state.rs`, and `UIScreenElementPalette` (007AB1D0) is
//! `screen_element_palette.rs`.

use bevy_ecs::prelude::*;
use oblivion_core::crt::stricmp;
use oblivion_core::hash::ni_string_hash;
use oblivion_core::logging::{Level, emit, enabled};

use super::super::bs::nif_objects::{BsParentVelocityModifier, BsxFlagsExtraData};
use super::animation::{FloatInterpolator, FloatKeyData};
use super::av_object_palette::DefaultAvObjectPalette;
use super::blend_interpolator::{
    ParticleBlendBoolInterpolator, ParticleBlendFloatInterpolator,
    ParticleBlendTransformInterpolator,
};
use super::bool_animation::{BoolInterpolator, BoolKeyData, BoolTimelineInterpolator};
use super::color_animation::ColorKeyData;
use super::controller_sequence::{ControllerManager, ControllerSequence};
use super::extra_data::{BinaryExtraData, StringExtraData, TextKeyExtraData};
use super::factory_tables::{
    ANIMATION_CLEANUP_NAMES, ANIMATION_FACTORIES, PARTICLE_CLEANUP_NAMES, PARTICLE_FACTORIES,
    SINGLETON_CLEANUP_NAMES, SINGLETON_FACTORIES,
};
use super::mesh_geometry::{MeshTriShapeData, MeshTriShapePayload, spawn_mesh_geometry};
use super::mesh_skin_owners::{MeshSkinData, MeshSkinInstance};
use super::mesh_skin_partition::MeshSkinPartition;
use super::mesh_tri_strips::MeshTriStripsData;
use super::object_net::{AlphaProperty, NiExtraDataName, NiObjectNet};
use super::particle_color::ParticleColorModifier;
use super::particle_controller::{ParticleEmitterController, ParticleUpdateController};
use super::particle_data::ParticleData;
use super::particle_drag::ParticleDragModifier;
use super::particle_dynamics::{
    ParticleAgeDeathModifier, ParticleBoundUpdateModifier, ParticleGrowFadeModifier,
    ParticlePositionModifier,
};
use super::particle_emitter::{ParticleBoxEmitter, ParticleCylinderEmitter, ParticleSphereEmitter};
use super::particle_gravity::ParticleGravityModifier;
use super::particle_mesh_emitter::ParticleMeshEmitter;
use super::particle_modifier_value_controller::{
    ParticleGravityStrengthController, ParticleModifierActiveController,
};
use super::particle_reset_controller::spawn_particle_reset_controller;
use super::particle_rotation::ParticleRotationModifier;
use super::particle_spawn::ParticleSpawnModifier;
use super::particle_system::spawn_particle_system;
use super::path_interpolator::PathInterpolator;
use super::position_animation::PositionKeyData;
use super::processor::{
    NiDx9TexGenSlots, NiProcessorSpecificState, host_cpuid, ni_processor_specific_initialize,
};
use super::properties::{
    StencilProperty, VertexColorProperty, WireframeProperty, ZBufferProperty, new_material_property,
};
use super::scene_derived::{spawn_billboard_node, spawn_directional_light};
use super::scene_node::{SceneNodePayload, spawn_scene_node};
use super::scene_state::ScenePropertyDefaults;
use super::stream_callbacks::LEGACY_CONTROLLER_STREAM_CALLBACK;
use super::stream_objects::NiClass;
use super::string_palette::StringPalette;
use super::texture::{
    FlipController, SourceTexture, TextureTransformController, TexturingProperty,
    spawn_texture_controller,
};
use super::transform_animation::{TransformInterpolator, TransformKeyData};
use super::transform_controller::{
    MultiTargetTransformController, TransformController, VisController, spawn_node_controller,
};

const FACTORY_BUCKETS: u32 = 59;

// IDA: N/A a helper, with no address of its own.
fn c_string(name: &str) -> &str {
    name.split('\0').next().unwrap_or(name)
}

/// IDA: 00584D10's `_stricmp` compares only inside the bucket 007DAED0's raw hash selected, so a different case is
/// the same key only when its raw-byte hash lands in the same bucket. Completion: 100% for ASCII class names.
fn keys_match(left: &str, right: &str) -> bool {
    let (left, right) = (c_string(left), c_string(right));
    ni_string_hash(left.as_bytes()) % FACTORY_BUCKETS
        == ni_string_hash(right.as_bytes()) % FACTORY_BUCKETS
        && stricmp(left.as_bytes(), right.as_bytes()) == 0
}

/// What 00712590's catalog answers for a name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactoryCreate {
    Unregistered,
    Unsupported,
    Created {
        requested_name: String,
        original_constructor: u32,
        entity: Entity,
    },
}

/// IDA: 00712590 NiFactoryRegistry: entries are host-ordered for inspection, and lookups follow the bucket rule.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct NiFactoryRegistry {
    entries: Vec<(String, u32)>,
    stream_callbacks: Vec<u32>,
}

impl NiFactoryRegistry {
    /// IDA: 00712590 -> 00412DA2: an existing key takes the new name pointer and value.
    pub fn insert_or_assign(&mut self, name: &str, original_constructor: u32) {
        let name = c_string(name);
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| keys_match(&entry.0, name))
        {
            name.clone_into(&mut entry.0);
            entry.1 = original_constructor;
        } else {
            self.entries.push((name.to_owned(), original_constructor));
        }
    }

    /// IDA: 007125B0 -> 004524C0.
    pub fn erase(&mut self, name: &str) -> bool {
        match self
            .entries
            .iter()
            .position(|entry| keys_match(&entry.0, name))
        {
            Some(index) => {
                self.entries.remove(index);
                true
            }
            None => false,
        }
    }

    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<u32> {
        self.entries
            .iter()
            .find(|entry| keys_match(&entry.0, name))
            .map(|entry| entry.1)
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn entries(&self) -> &[(String, u32)] {
        &self.entries
    }

    /// IDA: 00711F80 invokes deleting destructors on the 59-bucket registry and the callback array after the 73
    /// explicit removals. The unmatched NiSourceCubeMap entry goes with the registry. Completion: 100% of the host
    /// storage.
    pub fn release_storage(&mut self) {
        self.entries.clear();
        self.stream_callbacks.clear();
    }

    /// IDA: 006F98F0 BSFactory_RegisterWinMainClasses: nine classes in exact order. Completion: 100%.
    pub fn register_winmain_bs_factories(&mut self) {
        for (name, constructor) in [
            ("BSReference", 0x006F_DF70),
            ("BSNodeReferences", 0x006F_E0F0),
            ("NiBSBoneLODController", 0x006F_D6E0),
            ("BSBound", 0x006F_BA20),
            ("BSXFlags", 0x006F_A900),
            ("BSWindModifier", 0x006F_AC00),
            ("BSParentVelocityModifier", 0x006F_F000),
            ("BSFurnitureMarker", 0x006F_B340),
            ("BSPSysArrayEmitter", 0x006F_E9F0),
        ] {
            self.insert_or_assign(name, constructor);
        }
    }

    /// IDA: N/A the catalog alone, for a caller with no scheduler: the inserts of 00730D80, 006C2DB0 (with its 00714680
    /// callback) and 00748D20 in their dispatch order. `register_known_crt_callbacks` is the real route. Completion:
    /// 100% of the catalog.
    pub fn register_known_crt_factories(&mut self) {
        self.insert_all(SINGLETON_FACTORIES);
        self.insert_all(ANIMATION_FACTORIES);
        self.register_stream_callback(LEGACY_CONTROLLER_STREAM_CALLBACK);
        self.insert_all(PARTICLE_FACTORIES);
    }

    // IDA: N/A a helper, with no address of its own.
    fn insert_all(&mut self, registrations: &[(&str, u32)]) {
        for (name, constructor) in registrations {
            self.insert_or_assign(name, *constructor);
        }
    }

    /// IDA: 00714680 -> 004BACA0 fills the first empty NiTArray slot or appends; a null callback returns -1.
    /// Completion: 100% of the identity insertion.
    /// TODO: [ ] The 16-bit NiTArray capacity and allocation failure.
    pub fn register_stream_callback(&mut self, original_callback: u32) -> u32 {
        if original_callback == 0 {
            return u32::MAX;
        }
        if let Some(found) = self.stream_callbacks.iter().position(|&slot| slot == 0) {
            self.stream_callbacks[found] = original_callback;
            return found as u32;
        }
        self.stream_callbacks.push(original_callback);
        self.stream_callbacks.len() as u32 - 1
    }

    /// IDA: 007125D0 clears the first matching slot, decrementing first-free only for the last slot, and returns the
    /// scanned index. Completion: 100% of the identity removal.
    /// TODO: [ ] First-free and occupied counters if several callbacks register.
    pub fn remove_stream_callback(&mut self, original_callback: u32) -> u32 {
        let Some(found) = self
            .stream_callbacks
            .iter()
            .position(|&slot| slot == original_callback)
        else {
            return self.stream_callbacks.len() as u32;
        };
        self.stream_callbacks[found] = 0;
        if found + 1 == self.stream_callbacks.len() {
            self.stream_callbacks.pop();
        }
        found as u32
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn stream_callbacks(&self) -> &[u32] {
        &self.stream_callbacks
    }

    /// IDA: the factory call: a cold object by registered constructor address, so an alias creates the same class.
    /// Completion: 100% of the classes ported so far.
    pub fn create(&self, world: &mut World, name: &str) -> FactoryCreate {
        let Some(original_constructor) = self.find(name) else {
            return FactoryCreate::Unregistered;
        };
        let Some(entity) = spawn_cold(world, original_constructor) else {
            return FactoryCreate::Unsupported;
        };
        FactoryCreate::Created {
            requested_name: c_string(name).to_owned(),
            original_constructor,
            entity,
        }
    }
}

// IDA: N/A a helper, with no address of its own.
fn spawn_extra(world: &mut World, class: NiClass, data: impl Bundle) -> Entity {
    world.spawn((class, NiExtraDataName::default(), data)).id()
}

// IDA: N/A a helper, with no address of its own.
fn spawn_property(world: &mut World, class: NiClass, property: impl Bundle) -> Entity {
    world.spawn((class, NiObjectNet::default(), property)).id()
}

/// IDA: N/A a NiPSysData or NiPSysModifier: a NiObject with no NiObjectNET base.
fn spawn_particle(world: &mut World, class: NiClass, state: impl Bundle) -> Entity {
    world.spawn((class, state)).id()
}

// IDA: N/A no address of its own; its comments cite 006FA900.
fn spawn_cold(world: &mut World, constructor: u32) -> Option<Entity> {
    Some(match constructor {
        0x006D_7AD0 => world
            .spawn((NiClass::StringPalette, StringPalette::default()))
            .id(),
        0x0071_6B20 => spawn_extra(world, NiClass::StringExtraData, StringExtraData::default()),
        0x0072_7F20 => spawn_extra(world, NiClass::BinaryExtraData, BinaryExtraData::default()),
        // IDA: 006FBA20 BSBound_Create.
        0x006F_BA20 => spawn_extra(
            world,
            NiClass::BsBound,
            super::extra_data::BsBound::default(),
        ),
        // IDA: 006FB340 BSFurnitureMarker_Create.
        0x006F_B340 => spawn_extra(
            world,
            NiClass::BsFurnitureMarker,
            super::extra_data::BsFurnitureMarker::default(),
        ),
        0x006D_7450 => spawn_extra(
            world,
            NiClass::TextKeyExtraData,
            TextKeyExtraData::default(),
        ),
        0x0071_85B0 => spawn_property(world, NiClass::AlphaProperty, AlphaProperty::default()),
        0x0070_6CB0 => spawn_property(world, NiClass::ZBufferProperty, ZBufferProperty::default()),
        0x0070_6530 => spawn_property(
            world,
            NiClass::VertexColorProperty,
            VertexColorProperty::default(),
        ),
        0x0071_8F80 => spawn_property(world, NiClass::StencilProperty, StencilProperty::default()),
        // IDA: 0073D670 NiSpecularProperty.
        0x0073_D670 => spawn_property(
            world,
            NiClass::SpecularProperty,
            super::properties::SpecularProperty::default(),
        ),
        0x0070_9710 => {
            let material = new_material_property(world);
            spawn_property(world, NiClass::MaterialProperty, material)
        }
        0x0070_21F0 => spawn_property(world, NiClass::SourceTexture, SourceTexture::default()),
        0x0070_4A60 => spawn_property(
            world,
            NiClass::TexturingProperty,
            TexturingProperty::default(),
        ),
        0x0070_BA70 => spawn_scene_node(world, SceneNodePayload::default()),
        0x006E_3440 => world
            .spawn((NiClass::FloatKeyData, FloatKeyData::default()))
            .id(),
        0x006D_2DD0 => world
            .spawn((NiClass::FloatInterpolator, FloatInterpolator::default()))
            .id(),
        // IDA: NiBoolData and NiVisData share this constructor.
        0x006E_8740 => world
            .spawn((NiClass::BoolKeyData, BoolKeyData::default()))
            .id(),
        0x006E_8370 => world
            .spawn((NiClass::BoolInterpolator, BoolInterpolator::default()))
            .id(),
        0x006E_7E70 => world
            .spawn((
                NiClass::BoolTimelineInterpolator,
                BoolTimelineInterpolator::default(),
            ))
            .id(),
        0x006E_4520 => world
            .spawn((NiClass::ColorKeyData, ColorKeyData::default()))
            .id(),
        0x006D_9D40 => world
            .spawn((NiClass::PositionKeyData, PositionKeyData::default()))
            .id(),
        0x006E_1C00 => world
            .spawn((NiClass::TransformKeyData, TransformKeyData::default()))
            .id(),
        0x006E_71E0 => world
            .spawn((NiClass::BSplineData, super::bspline::BSplineData::default()))
            .id(),
        0x006E_7990 => world
            .spawn((
                NiClass::BSplineBasisData,
                super::bspline::BSplineBasisData::default(),
            ))
            .id(),
        0x006E_5BF0 => world
            .spawn((
                NiClass::BSplineCompTransformInterpolator,
                super::bspline::BSplineCompTransformInterpolator::default(),
            ))
            .id(),
        0x006E_6220 => world
            .spawn((
                NiClass::BSplineCompFloatInterpolator,
                super::bspline::BSplineCompFloatInterpolator::default(),
            ))
            .id(),
        0x006D_66E0 => world
            .spawn((
                NiClass::TransformInterpolator,
                TransformInterpolator::default(),
            ))
            .id(),
        0x006D_C5E0 => world
            .spawn((NiClass::PathInterpolator, PathInterpolator::default()))
            .id(),
        0x0075_9C50 => spawn_particle(world, NiClass::ParticleData, ParticleData::default()),
        0x0075_5690 => spawn_particle(
            world,
            NiClass::ParticlePositionModifier,
            ParticlePositionModifier::default(),
        ),
        0x0075_6DA0 => spawn_particle(
            world,
            NiClass::ParticleGrowFadeModifier,
            ParticleGrowFadeModifier::default(),
        ),
        0x0075_BD90 => spawn_particle(
            world,
            NiClass::ParticleAgeDeathModifier,
            ParticleAgeDeathModifier::default(),
        ),
        0x0075_A9E0 => spawn_particle(
            world,
            NiClass::ParticleBoundUpdateModifier,
            ParticleBoundUpdateModifier::default(),
        ),
        0x0075_A360 => spawn_particle(
            world,
            NiClass::ParticleColorModifier,
            ParticleColorModifier::default(),
        ),
        0x0075_8960 => spawn_particle(
            world,
            NiClass::ParticleDragModifier,
            ParticleDragModifier::default(),
        ),
        0x0075_0BA0 => spawn_particle(
            world,
            NiClass::ParticleGravityModifier,
            ParticleGravityModifier::default(),
        ),
        0x0074_E1C0 => spawn_particle(
            world,
            NiClass::ParticleRotationModifier,
            ParticleRotationModifier::default(),
        ),
        0x0075_1D10 => spawn_particle(
            world,
            NiClass::ParticleSpawnModifier,
            ParticleSpawnModifier::default(),
        ),
        0x0074_D2D0 => spawn_particle(
            world,
            NiClass::ParticleBoxEmitter,
            ParticleBoxEmitter::default(),
        ),
        0x0074_CFE0 => spawn_particle(
            world,
            NiClass::ParticleSphereEmitter,
            ParticleSphereEmitter::default(),
        ),
        0x0074_CC70 => spawn_particle(
            world,
            NiClass::ParticleCylinderEmitter,
            ParticleCylinderEmitter::default(),
        ),
        0x0074_BB20 => spawn_particle(
            world,
            NiClass::ParticleMeshEmitter,
            ParticleMeshEmitter::default(),
        ),
        0x0071_75C0 => {
            spawn_mesh_geometry(world, NiClass::TriShape, MeshTriShapePayload::default())
        }
        0x0071_FD30 => world
            .spawn((NiClass::TriShapeData, MeshTriShapeData::default()))
            .id(),
        0x0071_9A40 => {
            spawn_mesh_geometry(world, NiClass::TriStrips, MeshTriShapePayload::default())
        }
        0x0071_9E00 => world
            .spawn((NiClass::TriStripsData, MeshTriStripsData::default()))
            .id(),
        0x0072_CAA0 => world
            .spawn((NiClass::SkinPartition, MeshSkinPartition::default()))
            .id(),
        0x0072_F210 => world
            .spawn((NiClass::SkinData, MeshSkinData::default()))
            .id(),
        0x0072_BD10 => world
            .spawn((NiClass::SkinInstance, MeshSkinInstance::default()))
            .id(),
        0x0071_6A40 => world
            .spawn((
                NiClass::DefaultAvObjectPalette,
                DefaultAvObjectPalette::default(),
            ))
            .id(),
        0x006C_3F00 => spawn_node_controller(
            world,
            NiClass::TransformController,
            TransformController::default(),
        ),
        // IDA: 008AA920 bhkBlendController_Create.
        0x008A_A920 => super::transform_controller::spawn_node_controller(
            world,
            NiClass::BhkBlendController,
            super::bone_lod::BhkBlendController::default(),
        ),
        // IDA: 006FD6E0 NiBSBoneLODController_Create.
        0x006F_D6E0 => super::transform_controller::spawn_node_controller(
            world,
            NiClass::BoneLodController,
            super::bone_lod::BoneLodController::default(),
        ),
        0x006D_4620 => {
            spawn_node_controller(world, NiClass::VisController, VisController::default())
        }
        0x006D_0260 => spawn_node_controller(
            world,
            NiClass::MultiTargetTransformController,
            MultiTargetTransformController::default(),
        ),
        0x006C_5BB0 => spawn_node_controller(
            world,
            NiClass::ControllerManager,
            ControllerManager::default(),
        ),
        0x006C_7840 => world
            .spawn((NiClass::ControllerSequence, ControllerSequence::default()))
            .id(),
        0x006D_21D0 => {
            spawn_texture_controller(world, NiClass::FlipController, FlipController::default())
        }
        0x006D_7320 => spawn_texture_controller(
            world,
            NiClass::TextureTransformController,
            TextureTransformController::default(),
        ),
        // IDA: 006D23D0 NiAlphaController and 006DEF40 NiMaterialColorController (`gamebryo::material_controllers`).
        0x006D_23D0 => world
            .spawn((
                NiClass::AlphaController,
                super::material_controllers::MaterialController::new(
                    super::material_controllers::MaterialControllerKind::Alpha,
                ),
            ))
            .id(),
        0x006D_EF40 => world
            .spawn((
                NiClass::MaterialColorController,
                super::material_controllers::MaterialController::new(
                    super::material_controllers::MaterialControllerKind::Color {
                        target_color_40: 0,
                    },
                ),
            ))
            .id(),
        // IDA: 006D1520 NiGeomMorpherController and 006DDF90 NiMorphData (`gamebryo::morph`).
        0x006D_1520 => world
            .spawn((
                NiClass::GeomMorpherController,
                super::morph::GeomMorpherController::default(),
            ))
            .id(),
        0x006D_DF90 => world
            .spawn((NiClass::MorphData, super::morph::MorphData::default()))
            .id(),
        // IDA: 006DA740 NiPoint3Interpolator and 006EA8C0 NiBlendPoint3Interpolator.
        0x006D_A740 => world
            .spawn((
                NiClass::Point3Interpolator,
                super::material_controllers::Point3Interpolator::default(),
            ))
            .id(),
        0x006E_A8C0 => world
            .spawn((
                NiClass::BlendPoint3Interpolator,
                super::material_controllers::BlendPoint3Interpolator::default(),
            ))
            .id(),
        0x0074_9FE0 => spawn_particle_system(world),
        // IDA: 006FA900 constructs (006FA820 sets "BSX"), then calls NiExtraData_SetName(null): no name.
        0x006F_A900 => spawn_extra(
            world,
            NiClass::BsxFlagsExtraData,
            BsxFlagsExtraData::default(),
        ),
        // IDA: 008B71B0, 008AF070 and 008C8900, load only: no hk object (`bs::havok_stream`).
        0x008B_71B0 => spawn_particle(
            world,
            NiClass::BhkSpCollisionObject,
            crate::bs::havok_stream::BhkSpCollisionObject::default(),
        ),
        0x008A_F070 => spawn_particle(
            world,
            NiClass::BhkSimpleShapePhantom,
            crate::bs::havok_stream::BhkSimpleShapePhantom::default(),
        ),
        0x008C_8900 => spawn_particle(
            world,
            NiClass::BhkConvexVerticesShape,
            crate::bs::havok_stream::BhkConvexVerticesShape::default(),
        ),
        // IDA: 0089E960 bhkCollisionObject (0x14, the 00897600 base bhkSPCollisionObject shares); 008A41F0 bhkRigidBody and
        // 008B97A0 bhkRigidBodyT; the shapes 008C62B0, 008C33C0, 008B7E10, 008B6780, 008C93F0, 008A0EF0, 008A1B80; the constraints
        // 008C09E0 and 008B2BE0. Load and link only: no hk object (`bs::havok_stream`).
        0x0089_E960 => spawn_particle(
            world,
            NiClass::BhkCollisionObject,
            crate::bs::havok_stream::BhkSpCollisionObject::default(),
        ),
        0x008A_41F0 => spawn_particle(
            world,
            NiClass::BhkRigidBody,
            crate::bs::havok_stream::BhkRigidBody::default(),
        ),
        0x008B_97A0 => spawn_particle(
            world,
            NiClass::BhkRigidBodyT,
            crate::bs::havok_stream::BhkRigidBody::default(),
        ),
        0x008C_62B0 => spawn_particle(
            world,
            NiClass::BhkNiTriStripsShape,
            crate::bs::havok_stream::BhkShape::default(),
        ),
        0x008C_33C0 => spawn_particle(
            world,
            NiClass::BhkMoppBvTreeShape,
            crate::bs::havok_stream::BhkShape::default(),
        ),
        0x008B_7E10 => spawn_particle(
            world,
            NiClass::BhkBoxShape,
            crate::bs::havok_stream::BhkShape::default(),
        ),
        0x008B_6780 => spawn_particle(
            world,
            NiClass::BhkCapsuleShape,
            crate::bs::havok_stream::BhkShape::default(),
        ),
        0x008C_93F0 => spawn_particle(
            world,
            NiClass::BhkConvexTransformShape,
            crate::bs::havok_stream::BhkShape::default(),
        ),
        0x008A_0EF0 => spawn_particle(
            world,
            NiClass::BhkListShape,
            crate::bs::havok_stream::BhkShape::default(),
        ),
        0x008A_1B80 => spawn_particle(
            world,
            NiClass::BhkTransformShape,
            crate::bs::havok_stream::BhkShape::default(),
        ),
        0x008C_09E0 => spawn_particle(
            world,
            NiClass::BhkRagdollConstraint,
            crate::bs::havok_stream::BhkConstraint::default(),
        ),
        // IDA: 008BEFC0 bhkMalleableConstraint_Create.
        0x008B_EFC0 => world
            .spawn((
                NiClass::BhkMalleableConstraint,
                crate::bs::havok_stream::BhkConstraint::default(),
                crate::bs::havok_stream::BhkMalleableConstraint::default(),
            ))
            .id(),
        // IDA: 0088ED20 bhkBlendCollisionObject_Create (00BA7A1C + 1).
        0x0088_ED20 => {
            world
                .get_resource_or_init::<crate::bs::havok_stream::BlendCollisionStatics>()
                .count_ba7a1c += 1;
            world
                .spawn((
                    NiClass::BhkBlendCollisionObject,
                    crate::bs::havok_stream::BhkSpCollisionObject::default(),
                    crate::bs::havok_stream::BhkBlendCollisionObject::default(),
                ))
                .id()
        }
        // IDA: 008AF380 bhkSphereShape_Create.
        0x008A_F380 => spawn_particle(
            world,
            NiClass::BhkSphereShape,
            crate::bs::havok_stream::BhkShape::default(),
        ),
        0x008B_2BE0 => spawn_particle(
            world,
            NiClass::BhkLimitedHingeConstraint,
            crate::bs::havok_stream::BhkConstraint::default(),
        ),
        // IDA: 008C17B0 bhkPrismaticConstraint_CreateObject.
        0x008C_17B0 => spawn_particle(
            world,
            NiClass::BhkPrismaticConstraint,
            crate::bs::havok_stream::BhkConstraint::default(),
        ),
        0x006F_F000 => spawn_particle(
            world,
            NiClass::BsParentVelocityModifier,
            BsParentVelocityModifier::default(),
        ),
        0x0075_3BB0 => spawn_particle(
            world,
            NiClass::ParticleUpdateController,
            ParticleUpdateController::default(),
        ),
        0x0074_FDE0 => spawn_particle(
            world,
            NiClass::ParticleEmitterController,
            ParticleEmitterController::default(),
        ),
        0x0075_4E40 => spawn_particle_reset_controller(world),
        0x0075_6730 => spawn_particle(
            world,
            NiClass::ParticleModifierActiveController,
            ParticleModifierActiveController::default(),
        ),
        0x0075_7170 => spawn_particle(
            world,
            NiClass::ParticleGravityStrengthController,
            ParticleGravityStrengthController::default(),
        ),
        // IDA: 00757C30 NiPSysEmitterSpeedCtlr_Create.
        0x0075_7C30 => spawn_particle(
            world,
            NiClass::ParticleEmitterSpeedController,
            super::particle_modifier_value_controller::ParticleEmitterFloatController::default(),
        ),
        // IDA: 00757F40 NiPSysEmitterLifeSpanCtlr_Create.
        0x0075_7F40 => spawn_particle(
            world,
            NiClass::ParticleEmitterLifeSpanController,
            super::particle_modifier_value_controller::ParticleEmitterFloatController::default(),
        ),
        0x006E_B4C0 => spawn_particle(
            world,
            NiClass::BlendBoolInterpolator,
            ParticleBlendBoolInterpolator::default(),
        ),
        0x006D_24E0 => spawn_particle(
            world,
            NiClass::BlendFloatInterpolator,
            ParticleBlendFloatInterpolator::default(),
        ),
        0x006C_BCE0 => spawn_particle(
            world,
            NiClass::BlendTransformInterpolator,
            ParticleBlendTransformInterpolator::default(),
        ),
        0x0072_2490 => spawn_billboard_node(world),
        0x0071_97F0 => spawn_directional_light(world),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// The startup callback scheduler
// ---------------------------------------------------------------------------------------------------------------------

/// IDA: N/A a callback's failure: the C++ exception. A `&'static str`, so reporting one allocates nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupCallbackError(pub &'static str);

/// IDA: N/A a callback body. It takes `&mut World` for what the C++ lambda captured by reference.
pub type StartupCallbackFn = fn(&mut World) -> Result<(), StartupCallbackError>;

/// Original addresses identify evidence only; host callbacks never call x86 code. A callback can have a ported prefix
/// while other original effects remain open, so `init_complete` and `cleanup_complete` say so.
#[derive(Clone, Copy, Debug)]
pub struct StartupCallback {
    pub original_init: u32,
    pub original_cleanup: u32,
    pub init: Option<StartupCallbackFn>,
    pub cleanup: Option<StartupCallbackFn>,
    pub init_complete: bool,
    pub cleanup_complete: bool,
}

impl StartupCallback {
    /// IDA: N/A two original addresses with no host body, as the two repeat registration sites and the C++ test's
    /// `{a, b}` aggregates.
    #[must_use]
    pub const fn identity(original_init: u32, original_cleanup: u32) -> Self {
        Self {
            original_init,
            original_cleanup,
            init: None,
            cleanup: None,
            init_complete: true,
            cleanup_complete: true,
        }
    }

    /// IDA: N/A a callback with resolved host bodies.
    #[must_use]
    pub const fn new(
        original_init: u32,
        original_cleanup: u32,
        init: StartupCallbackFn,
        cleanup: StartupCallbackFn,
    ) -> Self {
        Self {
            init: Some(init),
            cleanup: Some(cleanup),
            ..Self::identity(original_init, original_cleanup)
        }
    }

    /// IDA: N/A marks a ported prefix: the original addresses stay in the pending reports.
    #[must_use]
    pub const fn pending(mut self, init_complete: bool, cleanup_complete: bool) -> Self {
        self.init_complete = init_complete;
        self.cleanup_complete = cleanup_complete;
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallbackRunResult {
    pub processor_count: u32,
    pub pending_initializers: Vec<u32>,
}

#[derive(Clone, Copy, Debug)]
struct PairState {
    callback: StartupCallback,
    ran: bool,
}

/// IDA: N/A `drain_cleanups`' report: a fixed array, so the drain allocates nothing.
struct CleanupResult {
    pending: [u32; 17],
    pending_count: usize,
    failure: Option<StartupCallbackError>,
}

// IDA: N/A a helper, with no address of its own.
fn log_verbose(message: std::fmt::Arguments<'_>) {
    if enabled(Level::Verbose) {
        emit(Level::Verbose, &message.to_string());
    }
}

/// IDA: 00747C20/00747C40 registration, 007489F0/00747BA0 startup and 00747BE0 teardown; 00748A30 returns the
/// NiLogBehavior singleton whose slot 0 (00748A40) runs from 00748490. Completion: 100% of the local dispatch. The
/// exact binary has no other static access to its 16 zeroed core slots, and 006F7030 makes the NiMemory.log open
/// attempt fail without a handle. The callback bodies' phase-03 effects are ported (C++ 03-B closure 2026-10-05,
/// translated here); their later consumers are later-phase dependencies.
///
/// The list holds plain functions, so this can be a resource. To run it from the world, take it with
/// `World::resource_scope`. A C++ destructor drains on scope exit; `Drop` has no `World` here, so that job is
/// `StartupCallbackScope`.
#[derive(Resource, Clone, Debug, Default)]
pub struct StartupCallbackScheduler {
    singleton: Option<StartupCallback>,
    pairs: Vec<PairState>,
    counters: Vec<u32>,
    processor_initialized: bool,
    run_started: bool,
    singleton_ran: bool,
    processor_count: u32,
    fail_processor_storage: bool,
}

impl StartupCallbackScheduler {
    /// IDA: N/A host-only failure injection for 007487D0's allocation branch. Completion: 100% for that branch.
    /// TODO: [ ] Reconcile the allocator's general failure policy separately.
    #[must_use]
    pub fn with_processor_storage_failure() -> Self {
        Self {
            fail_processor_storage: true,
            ..Self::default()
        }
    }

    /// IDA: 00747C20 StartupCallbacks_SetSingletonPair(initializer, cleanup). Completion: 100% of the slot assignment.
    /// TODO: [ ] Resolve the singleton callback's full body.
    pub fn set_singleton(&mut self, callback: StartupCallback) {
        log_verbose(format_args!(
            "Registered startup singleton initializer at 0x{:x}",
            callback.original_init
        ));
        self.singleton = Some(callback);
        self.singleton_ran = false;
    }

    /// IDA: 00747C40 StartupCallbacks_AppendPair(initializer, cleanup). Both parameters are verified function pointers;
    /// the original has 16 slots and no bounds branch, and the host rejects overflow before the indexed write would
    /// corrupt memory. Completion: 100%.
    /// TODO: [ ] Audit any CRT path that could exceed the original 16-pair capacity.
    pub fn register_pair(&mut self, callback: StartupCallback) -> Result<(), StartupCallbackError> {
        if self.pairs.len() >= 16 {
            return Err(StartupCallbackError(
                "Original startup callback pair capacity exceeded",
            ));
        }
        log_verbose(format_args!(
            "Registered startup callback pair at 0x{:x}",
            callback.original_init
        ));
        self.pairs.push(PairState {
            callback,
            ran: false,
        });
        Ok(())
    }

    /// IDA: 00731610/006C34C0/007492A0/0077BF60/0095A270 first-use wrappers; original wrapper signatures unresolved.
    /// Completion: 100% for a keyed first-registration gate; constructor reachability open.
    /// TODO: [ ] Resolve all paths that increment each original counter.
    pub fn register_pair_once(
        &mut self,
        original_counter: u32,
        callback: StartupCallback,
    ) -> Result<bool, StartupCallbackError> {
        if self.counters.contains(&original_counter) {
            return Ok(false);
        }
        self.counters.push(original_counter);
        if let Err(error) = self.register_pair(callback) {
            self.counters.pop();
            return Err(error);
        }
        Ok(true)
    }

    /// IDA: N/A host callback count inspection. Completion: 100% for registered host pairs.
    /// TODO: [ ] Compare the count with the original runtime after a full callback audit.
    #[must_use]
    pub fn pair_count(&self) -> usize {
        self.pairs.len()
    }

    /// IDA: 00747BA0 StartupCallbacks_RunInitializers: 007489F0 once, then the singleton and pair initializers.
    /// 007489F0 -> 007487D0 is the one-time processor count allocation and read; the host owns the storage. 00748490
    /// zeroes 16 unused core slots and 006F7030 always rejects its NiMemory.log open, so neither needs a host
    /// allocation. Host substitution: `available_parallelism` for GetSystemInfo's dwNumberOfProcessors. Completion:
    /// 100% of the local ordering; absent host callback effects stay pending.
    /// TODO: [ ] Implement each game-specific callback before claiming startup closure.
    pub fn run(&mut self, world: &mut World) -> Result<CallbackRunResult, StartupCallbackError> {
        if !self.processor_initialized {
            if !self.fail_processor_storage {
                self.processor_count = std::thread::available_parallelism()
                    .map_or(1, |count| u32::try_from(count.get()).unwrap_or(u32::MAX));
            }
            self.processor_initialized = true;
            if enabled(Level::Debug) {
                emit(
                    Level::Debug,
                    &format!("Startup processor count {}", self.processor_count),
                );
            }
        }
        self.run_started = true;
        let mut result = CallbackRunResult {
            processor_count: self.processor_count,
            pending_initializers: Vec::new(),
        };
        if let Some(singleton) = self.singleton {
            if let Some(init) = singleton.init {
                // Cleanup owns even the prefix of an initializer that fails.
                self.singleton_ran = true;
                init(world)?;
            }
            if singleton.init.is_none() || !singleton.init_complete {
                result.pending_initializers.push(singleton.original_init);
                log_verbose(format_args!(
                    "Original singleton initializer pending at 0x{:x}",
                    singleton.original_init
                ));
            }
        }
        for pair in &mut self.pairs {
            if let Some(init) = pair.callback.init {
                pair.ran = true;
                init(world)?;
            }
            if pair.callback.init.is_none() || !pair.callback.init_complete {
                result
                    .pending_initializers
                    .push(pair.callback.original_init);
                log_verbose(format_args!(
                    "Original startup initializer pending at 0x{:x}",
                    pair.callback.original_init
                ));
            }
        }
        Ok(result)
    }

    /// IDA: 00747BE0 StartupCallbacks_RunCleanups: paired cleanup in registration order, then the singleton's. Every
    /// entered callback drains before the report exists, and a failure does not stop later ones. Completion: 100% of
    /// the host bookkeeping, with no allocation.
    /// TODO: [ ] Complete each original cleanup body's own resource contract.
    fn drain_cleanups(&mut self, world: &mut World) -> CleanupResult {
        let mut result = CleanupResult {
            pending: [0; 17],
            pending_count: 0,
            failure: None,
        };
        if self.run_started {
            self.run_started = false;
            for pair in &mut self.pairs {
                let entered = std::mem::take(&mut pair.ran);
                if let Some(cleanup) = pair.callback.cleanup
                    && entered
                    && let Err(error) = cleanup(world)
                {
                    result.failure.get_or_insert(error);
                }
                if pair.callback.cleanup.is_none() || !pair.callback.cleanup_complete {
                    result.pending[result.pending_count] = pair.callback.original_cleanup;
                    result.pending_count += 1;
                }
            }
            if let Some(singleton) = self.singleton {
                let entered = std::mem::take(&mut self.singleton_ran);
                if let Some(cleanup) = singleton.cleanup
                    && entered
                    && let Err(error) = cleanup(world)
                {
                    result.failure.get_or_insert(error);
                }
                if singleton.cleanup.is_none() || !singleton.cleanup_complete {
                    result.pending[result.pending_count] = singleton.original_cleanup;
                    result.pending_count += 1;
                }
            }
        }
        self.processor_initialized = false;
        self.processor_count = 0;
        result
    }

    /// IDA: 00747BE0 StartupCallbacks_RunCleanups, called from 0040F618 after the FileFinder and palette release. The
    /// first cleanup failure returns after the drain; a failed report allocation is C++'s `bad_alloc` from the same
    /// place. Both come after all resource release. Completion: 100% of the local ordering; original heap and singleton
    /// effects open.
    /// TODO: [ ] Restore exact allocator and resource teardown for ported callbacks.
    pub fn shutdown(&mut self, world: &mut World) -> Result<Vec<u32>, StartupCallbackError> {
        let result = self.drain_cleanups(world);
        if let Some(failure) = result.failure {
            return Err(failure);
        }
        let mut report = Vec::new();
        report
            .try_reserve_exact(result.pending_count)
            .map_err(|_| StartupCallbackError("bad allocation"))?;
        report.extend_from_slice(&result.pending[..result.pending_count]);
        Ok(report)
    }
}

/// IDA: N/A `~StartupCallbackScheduler`: scope-exit cleanup for normal and failed startup exits. Host substitution: the
/// C++ destructor reaches its callbacks through captured references, and Rust's `Drop` has no `World`, so this guard
/// borrows one. Completion: 100% of the C++ destructor.
pub struct StartupCallbackScope<'w> {
    pub scheduler: StartupCallbackScheduler,
    world: &'w mut World,
}

impl<'w> StartupCallbackScope<'w> {
    /// IDA: N/A a helper, with no address of its own.
    pub fn new(world: &'w mut World, scheduler: StartupCallbackScheduler) -> Self {
        Self { scheduler, world }
    }

    /// IDA: N/A a helper, with no address of its own.
    pub fn run(&mut self) -> Result<CallbackRunResult, StartupCallbackError> {
        self.scheduler.run(self.world)
    }

    /// IDA: N/A a helper, with no address of its own.
    pub fn shutdown(&mut self) -> Result<Vec<u32>, StartupCallbackError> {
        self.scheduler.shutdown(self.world)
    }

    /// IDA: N/A a helper, with no address of its own.
    pub fn world(&mut self) -> &mut World {
        self.world
    }
}

impl Drop for StartupCallbackScope<'_> {
    /// IDA: N/A Rust's `Drop`.
    fn drop(&mut self) {
        let result = self.scheduler.drain_cleanups(self.world);
        if result.failure.is_some() {
            emit(Level::Error, "Host startup callback cleanup failed");
        }
        if result.pending_count != 0 {
            log_verbose(format_args!(
                "Original startup cleanup bodies pending: {}",
                result.pending_count
            ));
        }
    }
}

/// IDA: N/A the first-run flags of 006C2DB0, 00748D20 and 00730D80 (00B3FFB0): each callback guards its own first run,
/// and its cleanup partner clears the flag. Completion: 100% of the guards.
/// TODO: [ ] Reconcile these guards with original partial-allocation failure.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FactoryCallbackGuards {
    pub animation_active: bool,
    pub particle_active: bool,
    pub singleton_active: bool,
}

/// IDA: N/A one factory callback's arguments, as C++ `make_factory_callback`'s.
struct FactorySlice {
    registrations: &'static [(&'static str, u32)],
    cleanup_names: &'static [&'static str],
    guard: fn(&mut FactoryCallbackGuards) -> &mut bool,
    capture_particle_thread: bool,
    create_singleton_defaults: bool,
    register_animation_callback: bool,
}

const ANIMATION_SLICE: FactorySlice = FactorySlice {
    registrations: ANIMATION_FACTORIES,
    cleanup_names: ANIMATION_CLEANUP_NAMES,
    guard: |guards| &mut guards.animation_active,
    capture_particle_thread: false,
    create_singleton_defaults: false,
    register_animation_callback: true,
};

const PARTICLE_SLICE: FactorySlice = FactorySlice {
    registrations: PARTICLE_FACTORIES,
    cleanup_names: PARTICLE_CLEANUP_NAMES,
    guard: |guards| &mut guards.particle_active,
    capture_particle_thread: true,
    create_singleton_defaults: false,
    register_animation_callback: false,
};

const SINGLETON_SLICE: FactorySlice = FactorySlice {
    registrations: SINGLETON_FACTORIES,
    cleanup_names: SINGLETON_CLEANUP_NAMES,
    guard: |guards| &mut guards.singleton_active,
    capture_particle_thread: false,
    create_singleton_defaults: true,
    register_animation_callback: false,
};

// IDA: N/A a helper, with no address of its own.
fn factory_guard(world: &mut World, slice: &FactorySlice) -> bool {
    *(slice.guard)(&mut world.get_resource_or_insert_with(FactoryCallbackGuards::default))
}

// IDA: N/A a helper, with no address of its own.
fn registry_mut(world: &mut World) -> Mut<'_, NiFactoryRegistry> {
    world.get_resource_or_insert_with(NiFactoryRegistry::default)
}

/// IDA: N/A (site unresolved: the factory registration callbacks 00730D80 / 006C2DB0 / 00748D20 and the rest share this
/// body; calls 00712590). The 00712590 registry insertion inside them (map virtuals 007DAED0 hash, 00584D10 compare,
/// 004F0F60 entry acquire from the 12-byte node pool 0044D810/0044D770, 006C4F80 store); 00718630/00740FA0 and the
/// others install the singleton defaults beforehand. Each callback guards its own first run. Completion: 100% of the
/// scoped host map and default mutation; other constructors and helpers stay open.
/// TODO: [ ] Resolve the original map's allocation and collision failure branches.
fn factory_callback_init(world: &mut World, slice: &FactorySlice) {
    if factory_guard(world, slice) {
        return;
    }
    if slice.create_singleton_defaults {
        initialize_default_alpha(world);
        initialize_default_fog(world);
        initialize_default_material(world);
        initialize_pre_texturing_defaults(world);
        initialize_default_texturing(world);
        initialize_post_texturing_defaults(world);
        initialize_default_image_converter(world);
    }
    {
        let mut registry = registry_mut(world);
        registry.insert_all(slice.registrations);
        if slice.register_animation_callback {
            registry.register_stream_callback(LEGACY_CONTROLLER_STREAM_CALLBACK);
        }
    }
    if slice.create_singleton_defaults {
        initialize_singleton_tail(world);
    }
    if slice.capture_particle_thread {
        capture_particle_thread(world);
    }
    *(slice.guard)(&mut world.get_resource_or_insert_with(FactoryCallbackGuards::default)) = true;
}

/// IDA: 007125B0 -> 004524C0 cleanup of the registered names, then the singleton's releases, 00711F80 and the tail.
/// Completion: 100% of the scoped host key removal; unrelated nested cleanup effects stay open.
/// TODO: [ ] Reconcile replacement ownership with the original hash table.
fn factory_callback_cleanup(world: &mut World, slice: &FactorySlice) {
    if !factory_guard(world, slice) {
        return;
    }
    *(slice.guard)(&mut world.get_resource_or_insert_with(FactoryCallbackGuards::default)) = false;
    {
        let mut registry = registry_mut(world);
        for name in slice.cleanup_names {
            registry.erase(name);
        }
        if slice.register_animation_callback {
            registry.remove_stream_callback(LEGACY_CONTROLLER_STREAM_CALLBACK);
        }
    }
    if slice.create_singleton_defaults {
        release_default_alpha(world);
        release_default_fog(world);
        release_default_material(world);
        release_pre_texturing_defaults(world);
        release_default_texturing(world);
        release_post_texturing_defaults(world);
        release_default_image_converter(world);
        registry_mut(world).release_storage();
        release_singleton_tail(world);
    }
}

// IDA: N/A a helper, with no address of its own.
fn animation_callback_init(world: &mut World) -> Result<(), StartupCallbackError> {
    factory_callback_init(world, &ANIMATION_SLICE);
    Ok(())
}

// IDA: N/A a helper, with no address of its own.
fn animation_callback_cleanup(world: &mut World) -> Result<(), StartupCallbackError> {
    factory_callback_cleanup(world, &ANIMATION_SLICE);
    Ok(())
}

// IDA: N/A a helper, with no address of its own.
fn particle_callback_init(world: &mut World) -> Result<(), StartupCallbackError> {
    factory_callback_init(world, &PARTICLE_SLICE);
    Ok(())
}

// IDA: N/A a helper, with no address of its own.
fn particle_callback_cleanup(world: &mut World) -> Result<(), StartupCallbackError> {
    factory_callback_cleanup(world, &PARTICLE_SLICE);
    Ok(())
}

// IDA: N/A a helper, with no address of its own.
fn singleton_callback_init(world: &mut World) -> Result<(), StartupCallbackError> {
    factory_callback_init(world, &SINGLETON_SLICE);
    Ok(())
}

// IDA: N/A a helper, with no address of its own.
fn singleton_callback_cleanup(world: &mut World) -> Result<(), StartupCallbackError> {
    factory_callback_cleanup(world, &SINGLETON_SLICE);
    Ok(())
}

// IDA: N/A a helper, with no address of its own.
fn collision_callback_init(world: &mut World) -> Result<(), StartupCallbackError> {
    initialize_collision_dispatch(world);
    Ok(())
}

// IDA: N/A a helper, with no address of its own.
fn collision_callback_cleanup(world: &mut World) -> Result<(), StartupCallbackError> {
    shutdown_collision_dispatch(world);
    Ok(())
}

/// IDA: 0077BEE0's body and its 0077BF35 tail jump to 0077A070, both inside its one-time guard 00B4288C.
fn shader_callback_init(world: &mut World) -> Result<(), StartupCallbackError> {
    let mut creators = world.get_resource_or_insert_with(NiD3DShaderCreatorRegistry::default);
    if creators.initialized() {
        return Ok(());
    }
    creators.initialize();
    world
        .get_resource_or_insert_with(NiDx9TexGenSlots::default)
        .install_routines();
    Ok(())
}

// IDA: N/A a helper, with no address of its own.
fn shader_callback_cleanup(world: &mut World) -> Result<(), StartupCallbackError> {
    world
        .get_resource_or_insert_with(NiD3DShaderCreatorRegistry::default)
        .shutdown();
    Ok(())
}

/// IDA: N/A host assembly of _cinit slots 00A28470..00A2EF54 and wrappers. The registrars 007315F0/006C34A0(+00A020D0)/
/// 00749280(+00A0AA00)/0095A250/0077BF40 register only while their counter is 0; the first CRT indices (5267 singleton;
/// 41, 230, 773, 6882 pairs) fix this order, and P03-RUN-007 read counters 1/164/64/39/1. Completion: 100% of the order
/// and dedupe. The slots are ordered as _cinit reads its table. Both alternate sites increment a counter already seen
/// here, so they do not append a pair.
pub fn register_known_crt_callbacks(
    scheduler: &mut StartupCallbackScheduler,
) -> Result<(), StartupCallbackError> {
    // 00A28470 -> 009D8000 -> 006C34C0.
    scheduler.register_pair_once(
        0x00B3_CA30,
        StartupCallback::new(
            0x006C_2DB0,
            0x006C_31F0,
            animation_callback_init,
            animation_callback_cleanup,
        )
        .pending(false, false),
    )?;
    // 00A28764 -> 009DAE90 -> 007492A0.
    scheduler.register_pair_once(
        0x00B4_0818,
        StartupCallback::new(
            0x0074_8D20,
            0x0074_9060,
            particle_callback_init,
            particle_callback_cleanup,
        )
        .pending(false, false),
    )?;
    // 00A28FE0 -> 009E3CF0 -> 0095A270.
    scheduler.register_pair_once(
        0x00BA_9A68,
        StartupCallback::new(
            0x0095_9F00,
            0x0095_A1B0,
            collision_callback_init,
            collision_callback_cleanup,
        )
        .pending(false, false),
    )?;
    // 00A2B91C -> 00A020D0 -> 00A020ED, repeat of the 006C pair.
    scheduler.register_pair_once(
        0x00B3_CA30,
        StartupCallback::identity(0x006C_2DB0, 0x006C_31F0),
    )?;
    // 00A2D618 -> 00A09960 -> 00731610.
    scheduler.set_singleton(
        StartupCallback::new(
            0x0073_0D80,
            0x0073_1290,
            singleton_callback_init,
            singleton_callback_cleanup,
        )
        .pending(false, false),
    );
    // 00A2D864 -> 00A0AA00 -> 00A0AA1D, repeat of the 00748 pair.
    scheduler.register_pair_once(
        0x00B4_0818,
        StartupCallback::identity(0x0074_8D20, 0x0074_9060),
    )?;
    // 00A2EF54 -> 00A109C0 -> 0077BF60.
    scheduler.register_pair_once(
        0x00B4_2890,
        StartupCallback::new(
            0x0077_BEE0,
            0x0077_BD90,
            shader_callback_init,
            shader_callback_cleanup,
        )
        .pending(false, false),
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// The process singleton defaults
// ---------------------------------------------------------------------------------------------------------------------

/// IDA: 00740FA0 NiFogProperty, 0x2C bytes: +0x18 flags, +0x1C depth, +0x20..+0x28 color. Not `FogProperty`, which is
/// the 0x34-byte BSFogProperty with a range. Completion: 100% of the observed fields.
/// TODO: [ ] Port its base state and consumers.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct NiFogProperty {
    pub flags_18: u16,
    pub depth_1c: f32,
    pub color_20: [f32; 3],
}

/// IDA: 0073DBB0 NiShadeProperty: +0x18 flags, 1 here. Completion: 100% of the observed field.
/// TODO: [ ] Port its base state and consumers.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct NiShadeProperty {
    pub flags_18: u16,
}

/// IDA: 0073FED0 NiRendererSpecificProperty, 0x18 bytes: no derived field in this constructor. Completion: 100%.
/// TODO: [ ] Port its base state and consumers.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NiRendererSpecificProperty;

/// IDA: 0071B190 -> 0071ADD0 -> 00733940 -> 00713A30 constructs the 0x680-byte process NiImageConverter at 00B3FD28.
/// Its embedded reader owns a critical section (the `&mut World` borrow here, a host substitution) and the stream
/// starts with 37 empty object-map buckets. Completion: cold startup state only; converter virtuals open.
/// TODO: [ ] Port virtual image loading and conversion and the full NiStream state.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct NiImageConverterDefault {
    pub stream_version: u32,
    pub object_index_buckets: [u32; 37],
    pub captured_texture_palette: bool,
}

impl Default for NiImageConverterDefault {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            stream_version: 11,
            object_index_buckets: [0; 37],
            captured_texture_palette: false,
        }
    }
}

/// IDA: 00B3FA90..98, the shared color 00740FA0 copies. The cold static image holds zero. Completion: 100% of the read.
/// TODO: [ ] Connect any startup-reachable shared-color writer before this callback.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct SharedFogColor(pub [f32; 3]);

/// IDA: the refcounted process default globals, as property entities, because identity is observable. Releasing one
/// clears its slot and leaves the entity: an owner such as a `ScenePropertyState` may still name it, and NiRefObject
/// counts are not modelled on properties, so an entity nothing names stays until the world goes (host substitution).
/// Completion: 100% of the slots and their lifetime rules at this level.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NiProcessDefaults {
    pub alpha_b3fce4: Option<Entity>,
    pub fog_b401fc: Option<Entity>,
    pub material_b3faa4: Option<Entity>,
    pub renderer_specific: Option<Entity>,
    pub shade_b401ac: Option<Entity>,
    pub stencil_b3fcf8: Option<Entity>,
    pub texturing_b3f974: Option<Entity>,
    pub vertex_color_b3f980: Option<Entity>,
    pub wireframe_b3f984: Option<Entity>,
    pub zbuffer_b3f998: Option<Entity>,
    pub image_converter_b3fd28: Option<Entity>,
    pub device_image_converter_default: bool,
}

impl NiProcessDefaults {
    /// IDA: 007317B0's nine reads, for `scene_state::construct_scene_property_state`. It skips fog. Completion: 100%.
    #[must_use]
    pub const fn scene_property_defaults(&self) -> ScenePropertyDefaults {
        ScenePropertyDefaults {
            alpha: self.alpha_b3fce4,
            material: self.material_b3faa4,
            renderer_specific: self.renderer_specific,
            shade: self.shade_b401ac,
            stencil: self.stencil_b3fcf8,
            texturing: self.texturing_b3f974,
            vertex_color: self.vertex_color_b3f980,
            wireframe: self.wireframe_b3f984,
            zbuffer: self.zbuffer_b3f998,
        }
    }
}

// IDA: N/A a helper, with no address of its own.
fn process_defaults(world: &mut World) -> Mut<'_, NiProcessDefaults> {
    world.get_resource_or_insert_with(NiProcessDefaults::default)
}

/// IDA: 00718630 NiAlphaProperty_CreateDefault replaces 00B3FCE4 with flags 0x00EC and test reference zero. Completion:
/// 100% of the observed fields; base and virtual behavior open.
/// TODO: [ ] Port the NiProperty base state.
pub fn initialize_default_alpha(world: &mut World) {
    let alpha = spawn_property(world, NiClass::AlphaProperty, AlphaProperty::default());
    process_defaults(world).alpha_b3fce4 = Some(alpha);
}

/// IDA: 007186F0 clears 00B3FCE4 after dropping its retained reference. Completion: 100% of the host release.
/// TODO: [ ] Reconcile the original deleting destructor and external references.
pub fn release_default_alpha(world: &mut World) {
    process_defaults(world).alpha_b3fce4 = None;
}

/// IDA: 00740FA0 NiFogProperty_CreateDefault copies the current shared color at 00B3FA90..98, with flags +0x18 = 0 and
/// depth +0x1C = 1.0, then replaces the refcounted 00B401FC global. Completion: 100% of the observed fields; base and
/// virtual behavior open.
/// TODO: [ ] Port consumers of the default object.
pub fn initialize_default_fog(world: &mut World) {
    let shared_color = world
        .get_resource::<SharedFogColor>()
        .copied()
        .unwrap_or_default();
    let fog = world
        .spawn((
            NiObjectNet::default(),
            NiFogProperty {
                flags_18: 0,
                depth_1c: 1.0,
                color_20: shared_color.0,
            },
        ))
        .id();
    process_defaults(world).fog_b401fc = Some(fog);
}

/// IDA: 00741080 drops the global's reference and clears 00B401FC. Completion: 100% of the host release.
/// TODO: [ ] Reconcile the original deleting destructor and retained consumers.
pub fn release_default_fog(world: &mut World) {
    process_defaults(world).fog_b401fc = None;
}

/// IDA: 007098B0 -> 007095A0 creates the 0x5C-byte material default and increments the process counter before
/// publishing the refcounted global 00B3FAA4. Completion: 100% of the observed derived fields.
/// TODO: [ ] Trace the +0x54 marker and original virtual teardown.
pub fn initialize_default_material(world: &mut World) {
    let material = new_material_property(world);
    let material = spawn_property(world, NiClass::MaterialProperty, material);
    process_defaults(world).material_b3faa4 = Some(material);
}

/// IDA: 00709960 drops the global reference and clears 00B3FAA4. Completion: 100% of the host release.
/// TODO: [ ] Reconcile retained material owners and destructor effects.
pub fn release_default_material(world: &mut World) {
    process_defaults(world).material_b3faa4 = None;
}

/// IDA: 0073FED0, 0073DBB0 and 00719010 create three direct-field defaults after material and before texturing, in
/// original singleton order. 00719010 is NiStencilProperty at 00B3FCF8, 0073DBB0 NiShadeProperty at 00B401AC.
/// Completion: 100% of the observed derived fields.
/// TODO: [ ] Port base and virtual behavior and retained consumers.
pub fn initialize_pre_texturing_defaults(world: &mut World) {
    let renderer_specific = world
        .spawn((NiObjectNet::default(), NiRendererSpecificProperty))
        .id();
    let shade = world
        .spawn((NiObjectNet::default(), NiShadeProperty { flags_18: 1 }))
        .id();
    let stencil = spawn_property(world, NiClass::StencilProperty, StencilProperty::default());
    let mut defaults = process_defaults(world);
    defaults.renderer_specific = Some(renderer_specific);
    defaults.shade_b401ac = Some(shade);
    defaults.stencil_b3fcf8 = Some(stencil);
}

/// IDA: 0073FF80, 0073DC60 and 007190D0 release the same three globals. Completion: 100% of the host releases.
/// TODO: [ ] Reconcile retained owners and deleting destructors.
pub fn release_pre_texturing_defaults(world: &mut World) {
    let mut defaults = process_defaults(world);
    defaults.renderer_specific = None;
    defaults.shade_b401ac = None;
    defaults.stencil_b3fcf8 = None;
}

/// IDA: 00705240 -> 007043B0 uses 0042BB90 seven times to populate null map slots, leaving capacity and first-free 7,
/// occupied 0 and flags 4. Published at 00B3F974. Completion: 100% of the observed cold fields.
/// TODO: [ ] Port populated map behavior and original allocation failure.
pub fn initialize_default_texturing(world: &mut World) {
    let texturing = spawn_property(
        world,
        NiClass::TexturingProperty,
        TexturingProperty::default(),
    );
    process_defaults(world).texturing_b3f974 = Some(texturing);
}

/// IDA: 007040C0 drops the global reference and clears 00B3F974. Completion: 100% of the host release.
/// TODO: [ ] Reconcile populated array cleanup and retained consumers.
pub fn release_default_texturing(world: &mut World) {
    process_defaults(world).texturing_b3f974 = None;
}

/// IDA: 007065A0, 00706A40 and 00706D20 create the final three direct-field defaults before image-converter
/// construction. Completion: 100% of the observed derived fields.
/// TODO: [ ] Port base and virtual behavior and retained consumers.
pub fn initialize_post_texturing_defaults(world: &mut World) {
    let vertex_color = spawn_property(
        world,
        NiClass::VertexColorProperty,
        VertexColorProperty::default(),
    );
    let wireframe = world
        .spawn((NiObjectNet::default(), WireframeProperty::default()))
        .id();
    let zbuffer = spawn_property(world, NiClass::ZBufferProperty, ZBufferProperty::default());
    let mut defaults = process_defaults(world);
    defaults.vertex_color_b3f980 = Some(vertex_color);
    defaults.wireframe_b3f984 = Some(wireframe);
    defaults.zbuffer_b3f998 = Some(zbuffer);
}

/// IDA: 00706650, 00706AF0 and 00706DD0 drop their globals in this order. Completion: 100% of the host releases.
/// TODO: [ ] Reconcile retained original owners and deleting destructors.
pub fn release_post_texturing_defaults(world: &mut World) {
    let mut defaults = process_defaults(world);
    defaults.vertex_color_b3f980 = None;
    defaults.wireframe_b3f984 = None;
    defaults.zbuffer_b3f998 = None;
}

/// IDA: 0071B190 -> 0071ADD0 -> 00733940 -> 00713A30 initializes a NiStream object-index map with 37 zeroed buckets and
/// a reader lock. The palette allocation at WinMain 0040E68B follows this callback, so the captured palette reference
/// is null on the cold startup path. Completion: the cold state and lifetime; virtual conversion open.
/// TODO: [ ] Port reader virtual slots +8/+16 and full stream teardown.
pub fn initialize_default_image_converter(world: &mut World) {
    let converter = world.spawn(NiImageConverterDefault::default()).id();
    process_defaults(world).image_converter_b3fd28 = Some(converter);
}

/// IDA: 0071B240 clears 00B3FD28 after dropping the global reference. Completion: 100% of the host release; nested
/// NiStream and reader destruction open.
/// TODO: [ ] Reconcile the original critical-section and map destruction.
pub fn release_default_image_converter(world: &mut World) {
    let mut defaults = process_defaults(world);
    defaults.image_converter_b3fd28 = None;
    defaults.device_image_converter_default = false;
}

/// IDA: 0071B290 NiImageConverter_SetDefault as 00498614 calls it with the 0x900-byte NiDevImageConverter from
/// 0071F860: the 0071B190 object loses its only reference (0071B2BF -> 0071B2E0 destroys it, P04-RUN-001) and the
/// device converter becomes the default. The C++ host keeps no converter object for it, and neither does this one.
/// Completion: 100% of the ownership change.
pub fn set_device_image_converter_default(world: &mut World) {
    let mut defaults = process_defaults(world);
    let old = defaults.image_converter_b3fd28.take();
    defaults.device_image_converter_default = true;
    if let Some(old) = old {
        world.despawn(old);
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The singleton tail, the particle thread and the collision dispatch
// ---------------------------------------------------------------------------------------------------------------------

/// IDA: 00739980 allocates 0x18 bytes and writes six dwords before publishing 00B40134; 0073A640 calls 006EBCB0 then
/// frees it. Field roles remain unknown. Completion: 100% of the cold fields and lifetime; populated buffer use open.
/// TODO: [ ] Reconstruct 0073AC20 consumers and nested buffer ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SingletonScratchBuffer {
    pub words: [u32; 6],
}

impl Default for SingletonScratchBuffer {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            words: [0, 0, 0, 8, 8, 0],
        }
    }
}

/// IDA: 00700C50 copies the .data block 00B25E00 over 00B3F718, replacing the CRT slot-5272 values from 0070F2E0; no
/// code that runs before it references 00B25E00 (Phase 00 closure, 2026-10-05). These are the disk bytes.
const DEFAULT_PIXEL_FORMAT_WORDS: [u32; 17] = [
    0x0000_2001,
    1,
    0,
    0xFFFF_FFFF,
    0,
    0,
    0,
    0x108,
    1,
    0,
    0x108,
    2,
    0,
    0x108,
    3,
    0,
    0x108,
];

/// IDA: the singleton tail's process state. `renderer_alias_b3f928` is 00701700's non-owning alias, which 00701630
/// clears, and `object_slots_b3f800` the ten refcounted slots 00700FD0 releases. The copied words persist after
/// cleanup. Host stop: no renderer exists yet (M2), so nothing binds the alias except a test. Completion: the cold
/// state.
/// TODO: [ ] Port each reachable slot's lazy object construction (00701020) and the startup and menu consumers of the
/// alias.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SingletonTail {
    pub scratch_b40134: Option<SingletonScratchBuffer>,
    pub descriptor_words_b3f718: [u32; 17],
    pub object_slots_b3f800: [Option<Entity>; 10],
    pub renderer_alias_b3f928: Option<Entity>,
}

impl SingletonTail {
    /// IDA: N/A host entry point for 00700FD0's ten shared-owner globals. Completion: 100% of the owner transfer.
    /// TODO: [ ] Bind verified 00701020 objects and setter predicates.
    pub fn set_object_slot(&mut self, index: usize, object: Option<Entity>) -> bool {
        match self.object_slots_b3f800.get_mut(index) {
            Some(slot) => {
                *slot = object;
                true
            }
            None => false,
        }
    }

    /// IDA: 007013A0 NiCore_SetDefaultPixelFormatAndReleaseSlots, which 00764630 calls with the renderer's default 2D
    /// pixel format (+0x854): rep movsd of 0x11 dwords at 007013AD over 00B3F718, then 007013C0..007013F2 release slots
    /// 0..9 (all empty in P04-RUN-001). Completion: 100%.
    pub fn set_default_pixel_format(&mut self, words: &[u32; 17]) {
        self.descriptor_words_b3f718 = *words;
        self.object_slots_b3f800 = [None; 10];
    }
}

/// IDA: the 0x400-byte lookup 007072E0 fills, which 00725860 publishes from the singleton tail. The resource is present
/// exactly while the original's pointer is non-null.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct SqrtMantissaTable(pub [u32; 256]);

impl Default for SqrtMantissaTable {
    /// IDA: 007072E0 fills the lookup with sqrt mantissa bits for float patterns (index | 0x3F80 / 0x4000) << 16; x87
    /// stores float results. Completion: 100% of the table.
    /// TODO: [ ] Carried: the 00725890 sqrt-lookup use.
    fn default() -> Self {
        let mut mantissas = [0; 256];
        for index in 0..128u32 {
            let first = f32::from_bits((index | 0x3F80) << 16);
            let second = f32::from_bits((index | 0x4000) << 16);
            mantissas[index as usize] = first.sqrt().to_bits() & 0x007F_FFFF;
            mantissas[index as usize + 128] = second.sqrt().to_bits() & 0x007F_FFFF;
        }
        Self(mantissas)
    }
}

/// IDA: 00739980, 00700C50 and 00725860 run after 74 factory inserts, and 00731253 tail-jumps to 00717440 (processor
/// detection and the TransformPoints and TransformVectors slots; process-global, never reset). Completion: 100% of the
/// cold fields and ordered host state.
/// TODO: [ ] Carried: later 00B3F718 readers.
pub fn initialize_singleton_tail(world: &mut World) {
    let table = SqrtMantissaTable::default();
    {
        let mut tail = world.get_resource_or_insert_with(SingletonTail::default);
        tail.scratch_b40134 = Some(SingletonScratchBuffer::default());
        tail.descriptor_words_b3f718 = DEFAULT_PIXEL_FORMAT_WORDS;
    }
    world.insert_resource(table);
    let mut processor = world.get_resource_or_insert_with(NiProcessorSpecificState::default);
    ni_processor_specific_initialize(&mut processor, host_cpuid);
}

/// IDA: 0073A640 frees the scratch object through 006EBCB0, 00700FD0 releases ten refcounted slots, and 00725870 frees
/// the lookup table. The copied 0x44-byte globals persist after cleanup. 00701630 clears the non-owning renderer alias
/// before scratch teardown. Completion: 100% of the host release; slot producers open.
/// TODO: [ ] Port populated scratch and lazy slot-creation behavior.
pub fn release_singleton_tail(world: &mut World) {
    {
        let mut tail = world.get_resource_or_insert_with(SingletonTail::default);
        tail.renderer_alias_b3f928 = None;
        tail.scratch_b40134 = None;
        tail.object_slots_b3f800 = [None; 10];
    }
    world.remove_resource::<SqrtMantissaTable>();
}

/// IDA: 00748D20's stored GetCurrentThreadId. Host substitution: `ThreadId` for the Win32 value. Completion: 100% of
/// the store.
/// TODO: [ ] Reconstruct consumers of the saved thread identity.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParticleThread(pub Option<std::thread::ThreadId>);

/// IDA: 00748D20 GetCurrentThreadId after the particle registrations. Completion: 100%.
pub fn capture_particle_thread(world: &mut World) {
    world.insert_resource(ParticleThread(Some(std::thread::current().id())));
}

/// IDA: 0095D920/0095D9B0's result: a target, and whether the call arguments swap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollisionRoute {
    pub original_target: u32,
    pub reversed: bool,
}

/// IDA: 00959F00 allocates two 0x90-byte and two 0x18-byte dispatch arrays; 0095A1B0 releases all four and clears nine
/// global function pointers. Target identities only, never callable. The resource is present exactly while byte_BA9A65
/// is set.
/// TODO: [ ] Port the selected target bodies and their startup-reachable consumers.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollisionDispatchState {
    pub primary: [u32; 36],
    pub secondary: [u32; 36],
    pub tertiary: [u32; 6],
    pub quaternary: [u32; 6],
    pub global_handlers: [u32; 9],
}

/// IDA: 0095D920/0095D9B0 two-type dispatch lookup, including transposition. Completion: 100% of the bounded table
/// lookup; original target calls open.
/// TODO: [ ] Reconstruct each selected collision routine before invocation.
fn find_collision_pair(table: &[u32; 36], first: usize, second: usize) -> Option<CollisionRoute> {
    if first >= 6 || second >= 6 {
        return None;
    }
    let target = table[6 * first + second];
    if target != 0 {
        return Some(CollisionRoute {
            original_target: target,
            reversed: false,
        });
    }
    let target = table[6 * second + first];
    if target != 0 {
        return Some(CollisionRoute {
            original_target: target,
            reversed: true,
        });
    }
    None
}

/// IDA: 0095DA70/0095DAB0 direct six-entry collision dispatch lookup. Completion: 100% of the bounded host lookup;
/// original target calls open.
/// TODO: [ ] Reconstruct each selected collision routine before invocation.
fn find_collision_single(table: &[u32; 6], kind: usize) -> Option<u32> {
    table.get(kind).copied().filter(|&target| target != 0)
}

impl CollisionDispatchState {
    /// IDA: 0095D920 reads a 6x6 table at 6*first+second, falling back to the transposed slot and swapping call
    /// arguments.
    #[must_use]
    pub fn primary_pair(&self, first: usize, second: usize) -> Option<CollisionRoute> {
        find_collision_pair(&self.primary, first, second)
    }

    /// IDA: 0095D9B0, as 0095D920.
    #[must_use]
    pub fn secondary_pair(&self, first: usize, second: usize) -> Option<CollisionRoute> {
        find_collision_pair(&self.secondary, first, second)
    }

    /// IDA: 0095DA70 indexes a six-entry table by the virtual type ID at vtable +0x0C. Host rejects out-of-range and
    /// null.
    /// TODO: [ ] Resolve type-ID method ownership and dispatched effects.
    #[must_use]
    pub fn tertiary_for(&self, kind: usize) -> Option<u32> {
        find_collision_single(&self.tertiary, kind)
    }

    /// IDA: 0095DAB0, as 0095DA70.
    #[must_use]
    pub fn quaternary_for(&self, kind: usize) -> Option<u32> {
        find_collision_single(&self.quaternary, kind)
    }
}

/// IDA: 00959F00 tests byte_BA9A65 before registering NiCollisionData and storing the tables. Completion: 100% of the
/// four direct table identities and the global handler identities.
/// TODO: [ ] Reconcile original FormHeap null behavior with the 03-A policy.
/// TODO: [ ] Reconstruct each pointed-to dispatch routine.
pub fn initialize_collision_dispatch(world: &mut World) {
    if world.contains_resource::<CollisionDispatchState>() {
        return;
    }
    registry_mut(world).insert_or_assign("NiCollisionData", 0x0096_DAE0);
    let mut state = CollisionDispatchState {
        primary: [0; 36],
        secondary: [0; 36],
        tertiary: [
            0x0096_CDD0,
            0x0096_2E30,
            0x0096_07B0,
            0,
            0x0095_FC20,
            0x0095_E0E0,
        ],
        quaternary: [
            0x0096_CF80,
            0x0096_4AB0,
            0x0096_0CB0,
            0,
            0x0095_FDC0,
            0x0095_F210,
        ],
        global_handlers: [
            0x0096_8210,
            0x0096_1350,
            0x0095_F720,
            0x0096_D640,
            0x0096_CCF0,
            0x0095_FE70,
            0x0095_D830,
            0x0095_D860,
            0x0095_D8B0,
        ],
    };
    state.primary[0] = 0x0096_C550;
    state.primary[6] = 0x0096_82F0;
    state.primary[7] = 0x0096_1790;
    state.primary[8] = 0x0096_2CF0;
    state.primary[12] = 0x0096_0510;
    state.primary[14] = 0x0096_0650;
    for slot in [24, 25, 26, 29] {
        state.primary[slot] = 0x0095_FB40;
    }
    state.primary[28] = 0x0095_FBA0;
    state.primary[30] = 0x0095_DCA0;
    state.primary[31] = 0x0095_DD70;
    state.primary[32] = 0x0095_DEE0;
    state.primary[35] = 0x0095_E000;

    state.secondary[0] = 0x0096_C6A0;
    state.secondary[6] = 0x0096_46B0;
    state.secondary[7] = 0x0096_8B00;
    state.secondary[8] = 0x0096_49B0;
    state.secondary[12] = 0x0096_02C0;
    state.secondary[14] = 0x0096_03C0;
    for slot in [24, 25, 26, 27, 29] {
        state.secondary[slot] = 0x0095_FC90;
    }
    state.secondary[28] = 0x0095_FD10;
    state.secondary[30] = 0x0095_E250;
    state.secondary[31] = 0x0095_E4D0;
    state.secondary[32] = 0x0095_E980;
    state.secondary[35] = 0x0095_EEF0;
    world.insert_resource(state);
}

/// IDA: 0095A1B0 clears its guard, removes the factory, frees four allocations, nulls their globals and clears all nine
/// function slots.
/// TODO: [ ] Resolve collision table users before treating this pair as complete.
pub fn shutdown_collision_dispatch(world: &mut World) {
    if world.remove_resource::<CollisionDispatchState>().is_none() {
        return;
    }
    registry_mut(world).erase("NiCollisionData");
}

// ---------------------------------------------------------------------------------------------------------------------
// The shader creator registry, as data. Direct3D stops here: M2 owns the renderer.
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderCreatorKind {
    Hlsl,
    Assembly,
    Object,
}

/// IDA: 0077BC40/0077BC90/0077BCE0 construct three creator objects with original x86 sizes 8/16/16 bytes. The Asm and
/// Obj scalar destructor at 0077BD50 frees field +8; HLSL uses 0077BD30. RTTI at 00ACD868 lists Obj -> Asm -> base.
/// Identities and the buffer are evidence.
///
/// Host stop: the load methods behind virtual slots +36/+40 (00781170/00781350, 00782080/00782250, 007823C0/00782530)
/// reach D3DX and are not ported. wgpu cannot take this bytecode.
/// TODO: [ ] Recover the original consumer and the full D3DX metadata contract, in M2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NiD3DShaderCreator {
    pub kind: ShaderCreatorKind,
    pub original_vtable: u32,
    pub original_x86_size: usize,
    pub program_buffer: Vec<u8>,
}

impl NiD3DShaderCreator {
    // IDA: N/A a helper, with no address of its own.
    const fn new(kind: ShaderCreatorKind, original_vtable: u32, original_x86_size: usize) -> Self {
        Self {
            kind,
            original_vtable,
            original_x86_size,
            program_buffer: Vec::new(),
        }
    }

    /// IDA: RTTI 00ACD868: Obj derives from Asm, which owns the +8 buffer; HLSL does not.
    #[must_use]
    pub const fn derives_from_assembly(&self) -> bool {
        matches!(
            self.kind,
            ShaderCreatorKind::Assembly | ShaderCreatorKind::Object
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderCreatorRegistration {
    pub extension: String,
    pub kind: ShaderCreatorKind,
}

/// IDA: 00772970/007734A0/007797B0 (007797B0 -> 0075FFD0 for the 100-byte pool, then 007723C0 for the 96-byte pool)
/// install five FormHeap pool headers; 007797C0/00772B20/00773580 release them after packer cleanup. Noncallable
/// evidence.
/// TODO: [ ] Reconcile consumer object lifetimes and original pool exhaustion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderRuntimePoolDescriptor {
    pub original_global: u32,
    pub element_bytes: usize,
    pub initial_count: u32,
}

/// IDA: 00771300 selects the callback stored at (packerIndex, slotIndex). Completion: 100% of the bounded original
/// function-address selection for 0..17 x 0..32; pointed-to packing behavior and insertion allocation failure open.
/// TODO: [ ] Translate the selected functions before any host invocation.
const fn original_shader_packer_target(packer_index: usize, slot_index: usize) -> u32 {
    const UNSUPPORTED: u32 = 0x006F_7070;
    if packer_index >= 18 || slot_index >= 33 {
        return 0;
    }
    if slot_index == 0 {
        return UNSUPPORTED;
    }
    if packer_index < 4 {
        if slot_index <= 4 {
            if slot_index - 1 == packer_index {
                return 0x0076_E610;
            }
            return if slot_index - 1 < packer_index {
                0x0076_E4B0
            } else {
                0x0076_E430
            };
        }
        const GROUPS: [u32; 6] = [
            0x0076_E800,
            0x0076_E910,
            0x0076_EA10,
            0x0076_EB40,
            0x0076_EC60,
            0x0076_ED90,
        ];
        if slot_index <= 28 {
            return GROUPS[(slot_index - 5) / 4];
        }
        return 0x0076_E4B0;
    }
    if packer_index == 4 {
        if slot_index == 3 || slot_index == 4 || slot_index >= 29 {
            return 0x0076_EEB0;
        }
        if slot_index == 5 || slot_index == 9 {
            return 0x0076_F0D0;
        }
        if slot_index == 16 {
            return 0x0076_F110;
        }
        if slot_index == 20 {
            return 0x0076_F1C0;
        }
        if slot_index <= 24 && slot_index >= 21 {
            return 0x0076_F2A0;
        }
        if slot_index <= 28 && slot_index >= 25 {
            return 0x0076_F3B0;
        }
        return UNSUPPORTED;
    }
    if packer_index == 5 || packer_index == 6 || packer_index == 7 {
        if slot_index >= 29 {
            return UNSUPPORTED;
        }
        const ENTRY_FIVE: [u32; 7] = [
            0x0077_0980,
            0x0077_0AB0,
            0x0077_0BB0,
            0x0077_0CB0,
            0x0077_0DB0,
            0x0077_0EB0,
            0x0077_0FA0,
        ];
        const ENTRIES_SIX_SEVEN: [u32; 7] = [
            0x0077_01E0,
            0x0077_0300,
            0x0077_0420,
            0x0077_0540,
            0x0077_0640,
            0x0077_0740,
            0x0077_0860,
        ];
        let group = (slot_index - 1) / 4;
        return if packer_index == 5 {
            ENTRY_FIVE[group]
        } else {
            ENTRIES_SIX_SEVEN[group]
        };
    }
    0
}

/// IDA: 0076FAC0 -> 0076F520/00771300 builds 18 packer entries, each with 33 possible function slots; 0076F900 tears
/// them down. Addresses are data, never callable. Scoped registration identities; the packing functions and the
/// 0076F590 dispatcher stay open.
/// TODO: [ ] Port startup/menu-reachable NiD3DShaderDeclaration packing calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NiD3DShaderPackerRegistry {
    targets: [[u32; Self::SLOTS_PER_ENTRY]; Self::ENTRY_COUNT],
    initialized: bool,
}

impl Default for NiD3DShaderPackerRegistry {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            targets: [[0; Self::SLOTS_PER_ENTRY]; Self::ENTRY_COUNT],
            initialized: false,
        }
    }
}

impl NiD3DShaderPackerRegistry {
    pub const ENTRY_COUNT: usize = 18;
    pub const SLOTS_PER_ENTRY: usize = 33;

    /// IDA: 0076FAC0 allocates 18 NiPackerEntry objects and 00771300 fills their callback arrays. Completion: 100% of
    /// the identity registration; entries and callback execution stay data only.
    /// TODO: [ ] Port callback bodies and original partial-allocation behavior.
    pub fn initialize(&mut self) {
        if self.initialized {
            return;
        }
        for (entry, slots) in self.targets.iter_mut().enumerate() {
            for (slot, target) in slots.iter_mut().enumerate() {
                *target = original_shader_packer_target(entry, slot);
            }
        }
        self.initialized = true;
    }

    /// IDA: 0076F900 zeroes nested arrays and deletes all 18 entries. Completion: 100% of the host identity cleanup;
    /// original allocator side effects open.
    /// TODO: [ ] Reconcile the original entry allocator and exceptional cleanup.
    pub fn shutdown(&mut self) {
        *self = Self::default();
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn initialized(&self) -> bool {
        self.initialized
    }

    /// IDA: 00771300 stores selected pointers and 0076F590 reads them. Completion: 100% of the host identity lookup;
    /// original dispatch and callbacks open.
    /// TODO: [ ] Use verified host callback implementations when reached.
    #[must_use]
    pub fn original_target(&self, entry_index: usize, slot_index: usize) -> Option<u32> {
        if !self.initialized {
            return None;
        }
        self.targets
            .get(entry_index)?
            .get(slot_index)
            .copied()
            .filter(|&target| target != 0)
    }
}

/// IDA: 0077D180 -> 0077CF60 constructs the 0x38-byte NiD3DShaderFactory (base 007383C0 NiShaderFactory: NiRefObject
/// live counter 00B3FD64, which no code reads). Its global constants, shader cache and library maps start with 37/59/37
/// zeroed bucket heads, and 0077CF60 allocates and zeroes them in that order. Completion: 100% of the empty-map
/// construction.
///
/// Host stop: C++ `shader_creator.cpp`'s cache, library and global-constant methods, and the renderer notification,
/// wait for M2.
/// TODO: [ ] Port concrete NiShader and NiShaderLibrary producers and retrieval, and reconcile the unchecked map
/// allocations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NiD3DShaderFactoryState {
    global_constants: Vec<u32>,
    shaders: Vec<u32>,
    libraries: Vec<u32>,
}

impl Default for NiD3DShaderFactoryState {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            global_constants: vec![0; Self::GLOBAL_CONSTANT_BUCKETS],
            shaders: vec![0; Self::SHADER_BUCKETS],
            libraries: vec![0; Self::LIBRARY_BUCKETS],
        }
    }
}

impl NiD3DShaderFactoryState {
    pub const GLOBAL_CONSTANT_BUCKETS: usize = 37;
    pub const SHADER_BUCKETS: usize = 59;
    pub const LIBRARY_BUCKETS: usize = 37;

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn global_constants(&self) -> &[u32] {
        &self.global_constants
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn shaders(&self) -> &[u32] {
        &self.shaders
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub fn libraries(&self) -> &[u32] {
        &self.libraries
    }
}

/// IDA: 0077F680/0077F720 and 0077BD90 register a 37-bucket map of creator aliases, remove aliases by object identity
/// and destroy HLSL, Obj, Asm. Completion: the direct object and alias lifecycle, as data.
///
/// Host stop: no Direct3D object exists. 0077F7E0's renderer and device aliases (`bind_renderer_device` in C++,
/// attached from 0040E865) and the host compiler and loader calls are not ported; M2 owns the renderer. D3D formats
/// stay raw `u32`s.
/// TODO: [ ] Reconstruct original map allocation failure and consumers, device-loss rebinding and shader-program
/// consumers.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct NiD3DShaderCreatorRegistry {
    runtime_pools: [ShaderRuntimePoolDescriptor; 5],
    packers: NiD3DShaderPackerRegistry,
    shader_factory: Option<NiD3DShaderFactoryState>,
    active_shader_factory_b40120: bool,
    adapter_formats: Vec<u32>,
    hlsl: Option<NiD3DShaderCreator>,
    assembly: Option<NiD3DShaderCreator>,
    object: Option<NiD3DShaderCreator>,
    registrations: Vec<ShaderCreatorRegistration>,
    initialized: bool,
}

impl NiD3DShaderCreatorRegistry {
    pub const ORIGINAL_BUCKET_COUNT: usize = 37;
    pub const ORIGINAL_POOL_COUNT: usize = 5;

    /// IDA: 0077BEE0 NiD3DShaderSubsystem_InitializeOnce: 0077BE60 with D3DFORMAT 22, 23, 24 for 00763DE0's adapter
    /// enumeration; the five pools from 00772970, 007734A0, 007797B0 (007720D0 creates each 96-byte record's 184-byte
    /// child after lazy growth); the packers 0076FAC0; the factory at 0077D180 before 0077F6F0
    /// NiD3DShaderProgramCreator_EnsureMap; then 0077BC90 (Asm), 0077BCE0 (Obj), 0077BC40 (HLSL). 0076F590 dispatch has
    /// no static incoming reference in this executable, so the identity table is the bounded callback registration
    /// effect. The tail jump to 0077A070 is the callback's, as it needs another resource. Completion: the data.
    /// TODO: [ ] Resolve populated 00772970/007734A0/007797B0 pool records and revisit packer execution if a reachable
    /// declaration path is found.
    /// TODO: [ ] Reconcile six reviewed consumer functions, refcounts and populated-record teardown when a startup or
    /// menu caller is reached, and the original unchecked allocation failure paths.
    pub fn initialize(&mut self) {
        if self.initialized {
            return;
        }
        self.adapter_formats = vec![22, 23, 24];
        self.runtime_pools = [
            ShaderRuntimePoolDescriptor {
                original_global: 0x00B4_27A8,
                element_bytes: 20,
                initial_count: 8,
            },
            ShaderRuntimePoolDescriptor {
                original_global: 0x00B4_27AC,
                element_bytes: 16,
                initial_count: 16,
            },
            ShaderRuntimePoolDescriptor {
                original_global: 0x00B4_2838,
                element_bytes: 184,
                initial_count: 8,
            },
            ShaderRuntimePoolDescriptor {
                original_global: 0x00B4_2044,
                element_bytes: 100,
                initial_count: 8,
            },
            ShaderRuntimePoolDescriptor {
                original_global: 0x00B4_275C,
                element_bytes: 96,
                initial_count: 8,
            },
        ];
        self.packers.initialize();
        self.shader_factory = Some(NiD3DShaderFactoryState::default());
        self.active_shader_factory_b40120 = true;
        self.assembly = Some(NiD3DShaderCreator::new(
            ShaderCreatorKind::Assembly,
            0x00A8_AB5C,
            16,
        ));
        self.object = Some(NiD3DShaderCreator::new(
            ShaderCreatorKind::Object,
            0x00A8_AB90,
            16,
        ));
        self.hlsl = Some(NiD3DShaderCreator::new(
            ShaderCreatorKind::Hlsl,
            0x00A8_AB2C,
            8,
        ));
        for (extension, kind) in [
            ("vsh", ShaderCreatorKind::Assembly),
            ("psh", ShaderCreatorKind::Assembly),
            ("vso", ShaderCreatorKind::Object),
            ("pso", ShaderCreatorKind::Object),
            ("hlsl", ShaderCreatorKind::Hlsl),
        ] {
            self.registrations.push(ShaderCreatorRegistration {
                extension: extension.to_owned(),
                kind,
            });
        }
        self.initialized = true;
    }

    /// IDA: 0077BD90 NiD3DShaderSubsystem_ShutdownOnce removes aliases by identity before each scalar delete: HLSL,
    /// then Obj, then Asm. 0077EEB0 destroys the creator map, and 0077C270 finally destroys the NiD3DShaderFactory,
    /// clearing the active alias (00B40120) only when it still names this one. Completion: the data; 0077A070's slots
    /// are kept, as cleanup keeps them.
    /// TODO: [ ] Port side effects of the other fourth-pair cleanup children.
    pub fn shutdown(&mut self) {
        if !self.initialized {
            return;
        }
        self.registrations
            .retain(|item| item.kind != ShaderCreatorKind::Hlsl);
        self.hlsl = None;
        self.registrations
            .retain(|item| item.kind != ShaderCreatorKind::Object);
        self.object = None;
        self.registrations
            .retain(|item| item.kind != ShaderCreatorKind::Assembly);
        self.assembly = None;
        self.registrations.clear();
        self.active_shader_factory_b40120 = false;
        self.shader_factory = None;
        self.packers.shutdown();
        self.runtime_pools = [ShaderRuntimePoolDescriptor::default(); 5];
        self.adapter_formats = Vec::new();
        self.initialized = false;
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn initialized(&self) -> bool {
        self.initialized
    }

    /// IDA: 0077C0F0 returns the callback-owned NiD3DShaderFactory singleton. Completion: its presence and bucket
    /// allocation.
    #[must_use]
    pub const fn shader_factory(&self) -> Option<&NiD3DShaderFactoryState> {
        self.shader_factory.as_ref()
    }

    /// IDA: 0077D180 publishes the same factory at 00B40120. The original's alias, as a flag: only one factory exists
    /// here.
    /// TODO: [ ] Audit later active-factory replacement and consumer calls.
    #[must_use]
    pub const fn active_shader_factory(&self) -> Option<&NiD3DShaderFactoryState> {
        if self.active_shader_factory_b40120 {
            self.shader_factory.as_ref()
        } else {
            None
        }
    }

    /// IDA: 0077BEE0 calls 0077BE60 with D3DFORMAT 22, 23, 24. Completion: 100% of the ordered format registration;
    /// mode enumeration open.
    /// TODO: [ ] Port NiDX9AdapterDesc mode grouping and supported-mode policy.
    #[must_use]
    pub fn adapter_formats(&self) -> &[u32] {
        &self.adapter_formats
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn runtime_pools(&self) -> &[ShaderRuntimePoolDescriptor; 5] {
        &self.runtime_pools
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn packer_registry(&self) -> &NiD3DShaderPackerRegistry {
        &self.packers
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn registrations(&self) -> &[ShaderCreatorRegistration] {
        &self.registrations
    }

    /// IDA: N/A host lookup for the three original global creator pointers. Completion: 100% of the host state.
    #[must_use]
    pub const fn creator(&self, kind: ShaderCreatorKind) -> Option<&NiD3DShaderCreator> {
        match kind {
            ShaderCreatorKind::Hlsl => self.hlsl.as_ref(),
            ShaderCreatorKind::Assembly => self.assembly.as_ref(),
            ShaderCreatorKind::Object => self.object.as_ref(),
        }
    }

    /// IDA: N/A host lookup of keys inserted by 0077F720. Completion: 100% of the five literal registrations; dynamic
    /// keys open.
    /// TODO: [ ] Verify the original map lookup's case behavior before dynamic use.
    #[must_use]
    pub fn find(&self, extension: &str) -> Option<ShaderCreatorKind> {
        self.registrations
            .iter()
            .find(|item| item.extension == extension)
            .map(|item| item.kind)
    }

    /// IDA: N/A host-safe alias lookup for the verified five registrations. Completion: 100% within this registry.
    /// TODO: [ ] Locate the original dynamic consumer and cache and reload behavior.
    #[must_use]
    pub fn creator_for_extension(&self, extension: &str) -> Option<&NiD3DShaderCreator> {
        self.creator(self.find(extension)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The factory parts of `gamebryo_extra_data` and `gamebryo_string_palette`.
    #[test]
    fn cold_objects_and_aliases() {
        let mut world = World::new();
        let mut registry = NiFactoryRegistry::default();
        assert_eq!(
            registry.create(&mut world, "NiStringExtraData"),
            FactoryCreate::Unregistered
        );
        registry.insert_or_assign("NiStringExtraData", 0x0071_6B20);
        registry.insert_or_assign("SyntheticStringExtraAlias", 0x0071_6B20);
        registry.insert_or_assign("NiBinaryExtraData", 0x0072_7F20);
        registry.insert_or_assign("NiTextKeyExtraData", 0x006D_7450);
        registry.insert_or_assign("NiStringPalette", 0x006D_7AD0);
        let created = |world: &mut World, registry: &NiFactoryRegistry, name: &str| match registry
            .create(world, name)
        {
            FactoryCreate::Created {
                requested_name,
                original_constructor,
                entity,
            } => (requested_name, original_constructor, entity),
            other => panic!("{name}: {other:?}"),
        };
        let (_, constructor, string_data) = created(&mut world, &registry, "NiStringExtraData");
        assert_eq!(constructor, 0x0071_6B20);
        assert_eq!(
            world.get::<StringExtraData>(string_data),
            Some(&StringExtraData::default())
        );
        assert_eq!(
            world.get::<NiExtraDataName>(string_data),
            Some(&NiExtraDataName(None))
        );
        let (requested, alias_constructor, alias) =
            created(&mut world, &registry, "SyntheticStringExtraAlias");
        assert_eq!(requested, "SyntheticStringExtraAlias");
        assert_eq!(alias_constructor, constructor);
        assert_ne!(alias, string_data, "an alias makes its own object");
        let (_, _, binary) = created(&mut world, &registry, "NiBinaryExtraData");
        assert!(
            world
                .get::<BinaryExtraData>(binary)
                .unwrap()
                .bytes
                .is_empty()
        );
        let (_, _, keys) = created(&mut world, &registry, "NiTextKeyExtraData");
        assert!(world.get::<TextKeyExtraData>(keys).unwrap().keys.is_empty());
        let (_, _, palette) = created(&mut world, &registry, "NiStringPalette");
        assert_eq!(world.get::<StringPalette>(palette).unwrap().capacity(), 0);
        registry.release_storage();
        assert!(registry.entries().is_empty());
        assert!(
            world.get::<StringExtraData>(string_data).is_some(),
            "objects outlive the registry"
        );
    }

    /// IDA: an existing key takes the new spelling and value; `_stricmp` applies only inside one bucket.
    #[test]
    fn keys_follow_the_bucket_rule() {
        let mut registry = NiFactoryRegistry::default();
        registry.insert_or_assign("NiNode", 1);
        registry.insert_or_assign("NiNode", 2);
        assert_eq!(registry.entries().len(), 1);
        assert_eq!(registry.find("NiNode"), Some(2));
        assert_eq!(registry.find("NiNode\0ignored"), Some(2));
        let same_bucket = ni_string_hash(b"NiNode") % 59 == ni_string_hash(b"NINODE") % 59;
        assert_eq!(registry.find("NINODE").is_some(), same_bucket);
        assert!(registry.erase("NiNode"));
        assert!(!registry.erase("NiNode"));
    }

    #[test]
    fn winmain_classes_register_in_order() {
        let mut registry = NiFactoryRegistry::default();
        registry.register_winmain_bs_factories();
        let names: Vec<&str> = registry
            .entries()
            .iter()
            .map(|entry| entry.0.as_str())
            .collect();
        assert_eq!(names[0], "BSReference");
        assert_eq!(names[8], "BSPSysArrayEmitter");
        assert_eq!(names.len(), 9);
    }

    /// The particle arms of 00712590's catalog, with their C++ registration constructors.
    #[test]
    fn particle_factories_create_their_classes() {
        let mut world = World::new();
        let mut registry = NiFactoryRegistry::default();
        for (name, constructor, class) in [
            ("NiPSysData", 0x0075_9C50, NiClass::ParticleData),
            (
                "NiPSysPositionModifier",
                0x0075_5690,
                NiClass::ParticlePositionModifier,
            ),
            (
                "NiPSysGrowFadeModifier",
                0x0075_6DA0,
                NiClass::ParticleGrowFadeModifier,
            ),
            (
                "NiPSysAgeDeathModifier",
                0x0075_BD90,
                NiClass::ParticleAgeDeathModifier,
            ),
            (
                "NiPSysBoundUpdateModifier",
                0x0075_A9E0,
                NiClass::ParticleBoundUpdateModifier,
            ),
            (
                "NiPSysColorModifier",
                0x0075_A360,
                NiClass::ParticleColorModifier,
            ),
            (
                "NiPSysDragModifier",
                0x0075_8960,
                NiClass::ParticleDragModifier,
            ),
            (
                "NiPSysGravityModifier",
                0x0075_0BA0,
                NiClass::ParticleGravityModifier,
            ),
            (
                "NiPSysRotationModifier",
                0x0074_E1C0,
                NiClass::ParticleRotationModifier,
            ),
            (
                "NiPSysSpawnModifier",
                0x0075_1D10,
                NiClass::ParticleSpawnModifier,
            ),
            ("NiPSysBoxEmitter", 0x0074_D2D0, NiClass::ParticleBoxEmitter),
            (
                "NiPSysSphereEmitter",
                0x0074_CFE0,
                NiClass::ParticleSphereEmitter,
            ),
            (
                "NiPSysCylinderEmitter",
                0x0074_CC70,
                NiClass::ParticleCylinderEmitter,
            ),
            (
                "NiPSysMeshEmitter",
                0x0074_BB20,
                NiClass::ParticleMeshEmitter,
            ),
            ("NiParticleSystem", 0x0074_9FE0, NiClass::ParticleSystem),
            ("BSXFlags", 0x006F_A900, NiClass::BsxFlagsExtraData),
            (
                "BSParentVelocityModifier",
                0x006F_F000,
                NiClass::BsParentVelocityModifier,
            ),
            (
                "NiPSysUpdateCtlr",
                0x0075_3BB0,
                NiClass::ParticleUpdateController,
            ),
            (
                "NiPSysEmitterCtlr",
                0x0074_FDE0,
                NiClass::ParticleEmitterController,
            ),
            (
                "NiPSysResetOnLoopCtlr",
                0x0075_4E40,
                NiClass::ParticleResetOnLoopController,
            ),
            (
                "NiPSysModifierActiveCtlr",
                0x0075_6730,
                NiClass::ParticleModifierActiveController,
            ),
            (
                "NiPSysGravityStrengthCtlr",
                0x0075_7170,
                NiClass::ParticleGravityStrengthController,
            ),
            (
                "NiBlendBoolInterpolator",
                0x006E_B4C0,
                NiClass::BlendBoolInterpolator,
            ),
            (
                "NiBlendFloatInterpolator",
                0x006D_24E0,
                NiClass::BlendFloatInterpolator,
            ),
            (
                "NiBlendTransformInterpolator",
                0x006C_BCE0,
                NiClass::BlendTransformInterpolator,
            ),
        ] {
            registry.insert_or_assign(name, constructor);
            let FactoryCreate::Created { entity, .. } = registry.create(&mut world, name) else {
                panic!("{name} did not create");
            };
            assert_eq!(world.get::<NiClass>(entity), Some(&class));
        }
    }

    /// The geometry arms of 00712590's catalog, with their C++ registration constructors.
    #[test]
    fn geometry_factories_create_their_classes() {
        let mut world = World::new();
        let mut registry = NiFactoryRegistry::default();
        for (name, constructor, class) in [
            ("NiTriShape", 0x0071_75C0, NiClass::TriShape),
            ("NiTriShapeData", 0x0071_FD30, NiClass::TriShapeData),
            ("NiTriStrips", 0x0071_9A40, NiClass::TriStrips),
            ("NiTriStripsData", 0x0071_9E00, NiClass::TriStripsData),
            ("NiSkinPartition", 0x0072_CAA0, NiClass::SkinPartition),
            ("NiSkinData", 0x0072_F210, NiClass::SkinData),
            ("NiSkinInstance", 0x0072_BD10, NiClass::SkinInstance),
            (
                "NiDefaultAVObjectPalette",
                0x0071_6A40,
                NiClass::DefaultAvObjectPalette,
            ),
        ] {
            registry.insert_or_assign(name, constructor);
            let FactoryCreate::Created { entity, .. } = registry.create(&mut world, name) else {
                panic!("{name} did not create");
            };
            assert_eq!(world.get::<NiClass>(entity), Some(&class));
        }
        let FactoryCreate::Created { entity, .. } = registry.create(&mut world, "NiTriStrips")
        else {
            panic!("NiTriStrips did not create");
        };
        let payload = world.get::<MeshTriShapePayload>(entity).unwrap();
        assert_eq!(payload.effective_av_flags, 0x000E, "00708450's cold flags");
        let FactoryCreate::Created { entity, .. } = registry.create(&mut world, "NiSkinInstance")
        else {
            panic!("NiSkinInstance did not create");
        };
        assert_eq!(world.get::<MeshSkinInstance>(entity).unwrap().serial_18, -1);
    }

    #[test]
    fn material_factories_share_the_counter() {
        let mut world = World::new();
        let mut registry = NiFactoryRegistry::default();
        registry.insert_or_assign("NiMaterialProperty", 0x0070_9710);
        let ids: Vec<u32> = (0..2)
            .map(
                |_| match registry.create(&mut world, "NiMaterialProperty") {
                    FactoryCreate::Created { entity, .. } => {
                        world
                            .get::<super::super::properties::MaterialProperty>(entity)
                            .unwrap()
                            .material_id_18
                    }
                    other => panic!("{other:?}"),
                },
            )
            .collect();
        assert_eq!(ids, [2, 3]);
    }

    // The registration parts of the C++ `startup_scaffold` (`verify_startup_registration`).

    fn known_world() -> (World, StartupCallbackScheduler) {
        let mut scheduler = StartupCallbackScheduler::default();
        register_known_crt_callbacks(&mut scheduler).unwrap();
        (World::new(), scheduler)
    }

    fn defaults(world: &World) -> NiProcessDefaults {
        *world.resource::<NiProcessDefaults>()
    }

    #[test]
    fn known_callbacks_run_in_the_recorded_order() {
        let (mut world, mut known) = known_world();
        assert_eq!(
            known.pair_count(),
            4,
            "the first-use gate duplicated a pair"
        );
        let pending = known.run(&mut world).unwrap();
        assert!(pending.processor_count > 0);
        assert_eq!(
            pending.pending_initializers,
            [
                0x0073_0D80,
                0x006C_2DB0,
                0x0074_8D20,
                0x0095_9F00,
                0x0077_BEE0
            ]
        );
        let registry = world.resource::<NiFactoryRegistry>();
        assert_eq!(registry.entries().len(), 189);
        assert_eq!(registry.find("NiAlphaProperty"), Some(0x0071_85B0));
        assert_eq!(registry.find("NiMeshParticleSystem"), Some(0x0074_ECB0));
        assert_eq!(registry.find("NiScreenPolygon"), Some(0x0073_90D0));
        assert_eq!(registry.find("NiCollisionData"), Some(0x0096_DAE0));
        assert_eq!(
            registry.stream_callbacks(),
            [0x006D_43E0],
            "the animation stream callback follows its factories"
        );
        assert_eq!(
            world.resource::<ParticleThread>().0,
            Some(std::thread::current().id())
        );
        // The dispatch order is observable in the catalog: the singleton, the animation pair, the particle pair.
        let names: Vec<&str> = registry.entries().iter().map(|e| e.0.as_str()).collect();
        assert_eq!(names[0], "BSPackedAdditionalGeometryData");
        assert_eq!(names[74], "NiAlphaController");
        assert_eq!(names[74 + 63], "NiMeshParticleSystem");
        assert_eq!(names[188], "NiCollisionData");
        known.shutdown(&mut world).unwrap();
    }

    #[test]
    fn singleton_defaults_have_the_observed_fields() {
        let (mut world, mut known) = known_world();
        known.run(&mut world).unwrap();
        let retained = defaults(&world);
        let alpha = world
            .get::<AlphaProperty>(retained.alpha_b3fce4.unwrap())
            .unwrap();
        assert_eq!((alpha.flags_18, alpha.alpha_test_ref_1a), (0x00EC, 0));
        let fog = world
            .get::<NiFogProperty>(retained.fog_b401fc.unwrap())
            .unwrap();
        assert_eq!(
            (fog.flags_18, fog.depth_1c, fog.color_20),
            (0, 1.0, [0.0; 3])
        );
        let material = world
            .get::<super::super::properties::MaterialProperty>(retained.material_b3faa4.unwrap())
            .unwrap();
        assert!(material.material_id_18 > 1);
        assert_eq!(material.values.ambient, [0.5; 3]);
        assert_eq!(material.values.diffuse, [0.5; 3]);
        assert_eq!(material.values.specular, [0.0; 3]);
        assert_eq!(material.values.emissive, [0.0; 3]);
        assert_eq!(material.values.shine, 4.0);
        assert_eq!(material.values.alpha, 1.0);
        assert_eq!((material.raw_state_54, material.raw_state_58), (1, 0));
        assert!(
            world
                .get::<NiRendererSpecificProperty>(retained.renderer_specific.unwrap())
                .is_some()
        );
        assert_eq!(
            world
                .get::<NiShadeProperty>(retained.shade_b401ac.unwrap())
                .unwrap()
                .flags_18,
            1
        );
        let stencil = world
            .get::<StencilProperty>(retained.stencil_b3fcf8.unwrap())
            .unwrap();
        assert_eq!(
            (stencil.flags_18, stencil.reference_1c, stencil.mask_20),
            (0x4180, 0, 0xFFFF_FFFF)
        );
        let texturing = world
            .get::<TexturingProperty>(retained.texturing_b3f974.unwrap())
            .unwrap();
        assert_eq!(texturing.flags_18, 4);
        assert_eq!(texturing.maps_1c.len(), 7, "capacity and first-free");
        assert!(texturing.maps_1c.iter().all(Option::is_none));
        assert!(texturing.shader_maps_2c.is_none());
        assert_eq!(
            world
                .get::<VertexColorProperty>(retained.vertex_color_b3f980.unwrap())
                .unwrap()
                .flags_18,
            8
        );
        assert_eq!(
            world
                .get::<WireframeProperty>(retained.wireframe_b3f984.unwrap())
                .unwrap()
                .flags_18,
            0
        );
        assert_eq!(
            world
                .get::<ZBufferProperty>(retained.zbuffer_b3f998.unwrap())
                .unwrap()
                .flags_18,
            0x000F
        );
        let converter = world
            .get::<NiImageConverterDefault>(retained.image_converter_b3fd28.unwrap())
            .unwrap();
        assert_eq!(converter.stream_version, 11);
        assert!(!converter.captured_texture_palette);
        assert!(converter.object_index_buckets.iter().all(|&b| b == 0));
        known.shutdown(&mut world).unwrap();
    }

    #[test]
    fn fog_default_copies_the_shared_color() {
        let mut world = World::new();
        world.insert_resource(SharedFogColor([0.25, 0.5, 0.75]));
        initialize_default_fog(&mut world);
        let fog = defaults(&world).fog_b401fc.unwrap();
        assert_eq!(
            world.get::<NiFogProperty>(fog).unwrap().color_20,
            [0.25, 0.5, 0.75]
        );
    }

    /// IDA: 007317B0 through the existing scene state: nine retained defaults, fog untouched.
    #[test]
    fn scene_state_retains_the_nine_defaults() {
        use super::super::scene_state::{
            ScenePropertyState, apply_default_properties, construct_scene_property_state,
        };
        let (mut world, mut known) = known_world();
        known.run(&mut world).unwrap();
        let retained = defaults(&world);
        let state = construct_scene_property_state(&mut world, &retained.scene_property_defaults());
        let slots = world.get::<ScenePropertyState>(state).unwrap().slots;
        assert_eq!(
            slots,
            [
                retained.alpha_b3fce4,
                None,
                retained.material_b3faa4,
                retained.renderer_specific,
                retained.shade_b401ac,
                retained.stencil_b3fcf8,
                retained.texturing_b3f974,
                retained.vertex_color_b3f980,
                retained.wireframe_b3f984,
                retained.zbuffer_b3f998,
            ]
        );

        known.shutdown(&mut world).unwrap();
        // An owner keeps a released default.
        let slots = world.get::<ScenePropertyState>(state).unwrap().slots;
        assert_eq!(slots[0], retained.alpha_b3fce4);
        assert!(world.get::<AlphaProperty>(slots[0].unwrap()).is_some());
        assert_eq!(
            defaults(&world).scene_property_defaults(),
            ScenePropertyDefaults::default(),
            "nothing is left to retain once the globals clear"
        );
        let cleared = defaults(&world).scene_property_defaults();
        apply_default_properties(&mut world, state, &cleared);
        assert_eq!(
            world.get::<ScenePropertyState>(state).unwrap().slots,
            [None; 10],
            "a state re-applying the cleared globals releases what it held"
        );
    }

    fn spawn_with_net(world: &mut World, property: impl Bundle) -> Entity {
        world.spawn((NiObjectNet::default(), property)).id()
    }

    /// `verify_startup_registration`'s NiPropertyState and NiPropertySceneNode checks: a NiPropertySceneNode is a NiNode or
    /// a NiTriShape here, and a `NiPropertyOverride` a list entry.
    #[test]
    fn local_properties_merge_over_the_defaults() {
        use super::super::av_object::{NiAvParent, NiAvProperties};
        use super::super::mesh_geometry::NiGeometry;
        use super::super::scene_state::{
            ScenePropertyState, apply_default_properties, build_inherited_property_state,
            construct_scene_property_state, merge_property_list, refresh_property_state,
            update_properties_downward,
        };
        let (mut world, mut known) = known_world();
        known.run(&mut world).unwrap();
        let retained = defaults(&world);
        let scene = retained.scene_property_defaults();
        let local_fog = spawn_with_net(
            &mut world,
            NiFogProperty {
                flags_18: 0,
                depth_1c: 1.0,
                color_20: [0.0; 3],
            },
        );
        let first_alpha =
            spawn_property(&mut world, NiClass::AlphaProperty, AlphaProperty::default());
        let last_alpha =
            spawn_property(&mut world, NiClass::AlphaProperty, AlphaProperty::default());

        // A later property of one type wins, a null entry is skipped, and fog lands in its own slot.
        let merged = construct_scene_property_state(&mut world, &scene);
        let same = merge_property_list(
            &mut world,
            &[Some(local_fog), Some(first_alpha), Some(last_alpha), None],
            Some(merged),
            false,
        )
        .unwrap();
        assert_eq!(same, Some(merged));
        let slots = world.get::<ScenePropertyState>(merged).unwrap().slots;
        assert_eq!(slots[1], Some(local_fog));
        assert_eq!(slots[0], Some(last_alpha));
        assert_eq!(slots[2], retained.material_b3faa4);
        // 007317B0 restores the nine and leaves the merged fog.
        apply_default_properties(&mut world, merged, &scene);
        let slots = world.get::<ScenePropertyState>(merged).unwrap().slots;
        assert_eq!(slots[1], Some(local_fog));
        assert_eq!(slots[0], retained.alpha_b3fce4);

        // Root to leaf: an override order that never touches an ancestor's own state.
        let root = spawn_scene_node(&mut world, SceneNodePayload::default());
        let child = spawn_scene_node(&mut world, SceneNodePayload::default());
        let leaf = spawn_mesh_geometry(
            &mut world,
            NiClass::TriShape,
            MeshTriShapePayload::default(),
        );
        world
            .entity_mut(root)
            .insert(NiAvProperties(vec![Some(local_fog), Some(first_alpha)]));
        world.entity_mut(child).insert((
            NiAvProperties(vec![Some(last_alpha)]),
            NiAvParent(Some(root)),
        ));
        world.entity_mut(leaf).insert(NiAvParent(Some(child)));
        refresh_property_state(&mut world, leaf, &scene).unwrap();
        let leaf_state = world
            .get::<NiGeometry>(leaf)
            .unwrap()
            .property_state_ac
            .unwrap();
        let slots = world.get::<ScenePropertyState>(leaf_state).unwrap().slots;
        assert_eq!(slots[1], Some(local_fog));
        assert_eq!(slots[0], Some(last_alpha));
        assert_eq!(slots[2], retained.material_b3faa4);
        let inherited = build_inherited_property_state(&mut world, root, &scene)
            .unwrap()
            .unwrap();
        assert_eq!(
            world.get::<ScenePropertyState>(inherited).unwrap().slots[0],
            Some(first_alpha),
            "the ancestor's state keeps its own alpha"
        );

        // An empty list shares the inherited state.
        let empty = spawn_mesh_geometry(
            &mut world,
            NiClass::TriShape,
            MeshTriShapePayload::default(),
        );
        update_properties_downward(&mut world, empty, Some(inherited)).unwrap();
        assert_eq!(
            world.get::<NiGeometry>(empty).unwrap().property_state_ac,
            Some(inherited)
        );
        // A local list clones and leaves the shared state alone.
        let local = spawn_mesh_geometry(
            &mut world,
            NiClass::TriShape,
            MeshTriShapePayload::default(),
        );
        world
            .entity_mut(local)
            .insert(NiAvProperties(vec![Some(last_alpha)]));
        update_properties_downward(&mut world, local, Some(inherited)).unwrap();
        let cloned = world
            .get::<NiGeometry>(local)
            .unwrap()
            .property_state_ac
            .unwrap();
        assert_ne!(cloned, inherited);
        assert_eq!(
            world.get::<ScenePropertyState>(cloned).unwrap().slots[0],
            Some(last_alpha)
        );
        assert_eq!(
            world.get::<ScenePropertyState>(inherited).unwrap().slots[0],
            Some(first_alpha)
        );
        // The null state replaces what an empty list held.
        update_properties_downward(&mut world, empty, None).unwrap();
        assert_eq!(
            world.get::<NiGeometry>(empty).unwrap().property_state_ac,
            None
        );
        known.shutdown(&mut world).unwrap();
    }

    #[test]
    fn singleton_tail_cold_state_and_slots() {
        let (mut world, mut known) = known_world();
        known.run(&mut world).unwrap();
        let tail = *world.resource::<SingletonTail>();
        assert_eq!(tail.scratch_b40134.unwrap().words, [0, 0, 0, 8, 8, 0]);
        assert_eq!(tail.descriptor_words_b3f718[0], 0x2001);
        assert_eq!(tail.descriptor_words_b3f718[3], 0xFFFF_FFFF);
        assert_eq!(tail.descriptor_words_b3f718[16], 0x108);
        let table = world.resource::<SqrtMantissaTable>();
        assert_eq!(table.0[0], 0);
        assert_eq!(table.0[1], 0x7FC0);
        assert_eq!(table.0[127], 0x0034_AA5A);
        assert_eq!(table.0[128], 0x0035_04F3);
        assert_eq!(table.0[255], 0x007F_7FE0);
        assert!(
            world
                .resource::<NiProcessorSpecificState>()
                .detection
                .detected,
            "the 00731253 tail jump ran"
        );

        let renderer = world.spawn_empty().id();
        let object = world.spawn_empty().id();
        {
            let mut tail = world.resource_mut::<SingletonTail>();
            tail.renderer_alias_b3f928 = Some(renderer);
            assert!(tail.set_object_slot(4, Some(object)));
            assert_eq!(tail.object_slots_b3f800[4], Some(object));
            assert!(!tail.set_object_slot(10, Some(object)), "ten entries");
        }
        known.shutdown(&mut world).unwrap();
        let tail = *world.resource::<SingletonTail>();
        assert_eq!(tail.renderer_alias_b3f928, None);
        assert_eq!(tail.object_slots_b3f800, [None; 10]);
        assert_eq!(tail.scratch_b40134, None);
        assert!(world.get_resource::<SqrtMantissaTable>().is_none());
        assert_eq!(
            tail.descriptor_words_b3f718[0], 0x2001,
            "the copied words persist"
        );
    }

    #[test]
    fn set_default_pixel_format_replaces_words_and_releases_slots() {
        let mut tail = SingletonTail::default();
        let mut world = World::new();
        assert!(tail.set_object_slot(9, Some(world.spawn_empty().id())));
        tail.set_default_pixel_format(&[7; 17]);
        assert_eq!(tail.descriptor_words_b3f718, [7; 17]);
        assert_eq!(tail.object_slots_b3f800, [None; 10]);
    }

    #[test]
    fn device_converter_replaces_the_default() {
        let mut world = World::new();
        initialize_default_image_converter(&mut world);
        let cold = defaults(&world).image_converter_b3fd28.unwrap();
        set_device_image_converter_default(&mut world);
        assert_eq!(defaults(&world).image_converter_b3fd28, None);
        assert!(defaults(&world).device_image_converter_default);
        assert!(
            world.get_entity(cold).is_err(),
            "0071B2BF -> 0071B2E0 destroys the base converter"
        );
        release_default_image_converter(&mut world);
        assert!(!defaults(&world).device_image_converter_default);
    }

    #[test]
    fn shader_creators_are_data() {
        let (mut world, mut known) = known_world();
        known.run(&mut world).unwrap();
        let creators = world.resource::<NiD3DShaderCreatorRegistry>();
        assert_eq!(creators.adapter_formats(), [22, 23, 24]);
        let pools = creators.runtime_pools();
        assert_eq!(
            pools.map(|pool| (pool.original_global, pool.element_bytes, pool.initial_count)),
            [
                (0x00B4_27A8, 20, 8),
                (0x00B4_27AC, 16, 16),
                (0x00B4_2838, 184, 8),
                (0x00B4_2044, 100, 8),
                (0x00B4_275C, 96, 8),
            ]
        );
        let packers = creators.packer_registry();
        let mut registered_packers = 0;
        for entry in 0..NiD3DShaderPackerRegistry::ENTRY_COUNT {
            for slot in 0..NiD3DShaderPackerRegistry::SLOTS_PER_ENTRY {
                registered_packers += usize::from(packers.original_target(entry, slot).is_some());
            }
        }
        assert!(packers.initialized());
        assert_eq!(registered_packers, 274);
        for (entry, slot, target) in [
            (0, 0, 0x006F_7070),
            (0, 1, 0x0076_E610),
            (0, 2, 0x0076_E430),
            (1, 1, 0x0076_E4B0),
            (1, 2, 0x0076_E610),
            (4, 3, 0x0076_EEB0),
            (4, 5, 0x0076_F0D0),
            (4, 16, 0x0076_F110),
            (4, 20, 0x0076_F1C0),
            (4, 21, 0x0076_F2A0),
            (4, 28, 0x0076_F3B0),
            (5, 1, 0x0077_0980),
            (6, 1, 0x0077_01E0),
            (7, 32, 0x006F_7070),
            (17, 0, 0x006F_7070),
        ] {
            assert_eq!(packers.original_target(entry, slot), Some(target));
        }
        assert_eq!(packers.original_target(8, 1), None);
        assert_eq!(packers.original_target(18, 0), None);
        assert_eq!(packers.original_target(0, 33), None);

        let factory = creators.shader_factory().unwrap();
        assert_eq!(creators.active_shader_factory(), Some(factory));
        assert_eq!(factory.global_constants().len(), 37);
        assert_eq!(factory.shaders().len(), 59);
        assert_eq!(factory.libraries().len(), 37);
        assert!(factory.global_constants().iter().all(|&head| head == 0));
        assert!(factory.shaders().iter().all(|&head| head == 0));
        assert!(factory.libraries().iter().all(|&head| head == 0));

        assert!(creators.initialized());
        assert_eq!(NiD3DShaderCreatorRegistry::ORIGINAL_BUCKET_COUNT, 37);
        assert_eq!(creators.registrations().len(), 5);
        let hlsl = creators.creator(ShaderCreatorKind::Hlsl).unwrap();
        let assembly = creators.creator(ShaderCreatorKind::Assembly).unwrap();
        let object = creators.creator(ShaderCreatorKind::Object).unwrap();
        assert_eq!(
            (hlsl.original_x86_size, hlsl.original_vtable),
            (8, 0x00A8_AB2C)
        );
        assert!(!hlsl.derives_from_assembly());
        assert_eq!(
            (assembly.original_x86_size, assembly.original_vtable),
            (16, 0x00A8_AB5C)
        );
        assert!(assembly.derives_from_assembly());
        assert_eq!(
            (object.original_x86_size, object.original_vtable),
            (16, 0x00A8_AB90)
        );
        assert!(object.derives_from_assembly());
        assert_eq!(creators.find("vsh"), Some(ShaderCreatorKind::Assembly));
        assert_eq!(creators.find("psh"), Some(ShaderCreatorKind::Assembly));
        assert_eq!(creators.find("vso"), Some(ShaderCreatorKind::Object));
        assert_eq!(creators.find("pso"), Some(ShaderCreatorKind::Object));
        assert_eq!(creators.find("hlsl"), Some(ShaderCreatorKind::Hlsl));
        assert_eq!(creators.find("missing"), None);
        assert_eq!(creators.creator_for_extension("pso"), Some(object));
        assert_eq!(creators.creator_for_extension("missing"), None);
        assert_eq!(
            world.resource::<NiDx9TexGenSlots>().0,
            super::super::processor::NI_DX9_TEXGEN_ROUTINES,
            "the 0077BF35 tail jump"
        );
        known.shutdown(&mut world).unwrap();
        assert_eq!(
            world.resource::<NiDx9TexGenSlots>().0,
            super::super::processor::NI_DX9_TEXGEN_ROUTINES,
            "cleanup keeps the slots"
        );
    }

    #[test]
    fn collision_dispatch_tables_and_routes() {
        let (mut world, mut known) = known_world();
        known.run(&mut world).unwrap();
        let collision = world.resource::<CollisionDispatchState>();
        assert_eq!(collision.primary[0], 0x0096_C550);
        assert_eq!(collision.primary[29], 0x0095_FB40);
        assert_eq!(collision.secondary[27], 0x0095_FC90);
        assert_eq!(collision.secondary[35], 0x0095_EEF0);
        assert_eq!(
            collision.tertiary,
            [
                0x0096_CDD0,
                0x0096_2E30,
                0x0096_07B0,
                0,
                0x0095_FC20,
                0x0095_E0E0
            ]
        );
        assert_eq!(
            collision.quaternary,
            [
                0x0096_CF80,
                0x0096_4AB0,
                0x0096_0CB0,
                0,
                0x0095_FDC0,
                0x0095_F210
            ]
        );
        assert_eq!(collision.global_handlers[0], 0x0096_8210);
        assert_eq!(collision.global_handlers[8], 0x0095_D8B0);
        let route = |original_target, reversed| {
            Some(CollisionRoute {
                original_target,
                reversed,
            })
        };
        assert_eq!(collision.primary_pair(1, 0), route(0x0096_82F0, false));
        assert_eq!(collision.primary_pair(0, 1), route(0x0096_82F0, true));
        assert_eq!(collision.secondary_pair(0, 1), route(0x0096_46B0, true));
        assert_eq!(collision.primary_pair(1, 3), None);
        assert_eq!(collision.primary_pair(6, 0), None);
        assert_eq!(collision.tertiary_for(1), Some(0x0096_2E30));
        assert_eq!(collision.tertiary_for(3), None);
        assert_eq!(collision.quaternary_for(6), None);
        known.shutdown(&mut world).unwrap();
    }

    #[test]
    fn guards_block_a_repeat_and_reset_after_cleanup() {
        let (mut world, mut known) = known_world();
        known.run(&mut world).unwrap();
        let retained = defaults(&world);
        let retained_material_id = world
            .get::<super::super::properties::MaterialProperty>(retained.material_b3faa4.unwrap())
            .unwrap()
            .material_id_18;

        // A repeated dispatch bypasses nothing.
        assert_eq!(known.run(&mut world).unwrap().pending_initializers.len(), 5);
        assert_eq!(world.resource::<NiFactoryRegistry>().entries().len(), 189);
        assert_eq!(defaults(&world), retained);
        assert_eq!(
            world.resource::<NiFactoryRegistry>().stream_callbacks(),
            [0x006D_43E0]
        );
        assert_eq!(
            world
                .resource::<NiD3DShaderCreatorRegistry>()
                .registrations()
                .len(),
            5
        );

        // Unimplemented teardown is not silently marked complete.
        assert_eq!(
            known.shutdown(&mut world).unwrap(),
            [
                0x006C_31F0,
                0x0074_9060,
                0x0095_A1B0,
                0x0077_BD90,
                0x0073_1290
            ]
        );
        let registry = world.resource::<NiFactoryRegistry>();
        assert!(registry.entries().is_empty());
        assert!(registry.stream_callbacks().is_empty());
        assert_eq!(registry.find("NiAlphaProperty"), None);
        assert_eq!(registry.find("NiMeshParticleSystem"), None);
        assert_eq!(registry.find("NiScreenPolygon"), None);
        assert_eq!(registry.find("NiCollisionData"), None);
        assert_eq!(
            registry.find("NiSourceCubeMap"),
            None,
            "no explicit removal: 00711F80 destroys it with the registry"
        );
        assert_eq!(defaults(&world), NiProcessDefaults::default());
        // Owned objects outlive the globals.
        assert_eq!(
            world
                .get::<AlphaProperty>(retained.alpha_b3fce4.unwrap())
                .unwrap()
                .flags_18,
            0x00EC
        );
        assert_eq!(
            world
                .get::<NiFogProperty>(retained.fog_b401fc.unwrap())
                .unwrap()
                .depth_1c,
            1.0
        );
        assert_eq!(
            world
                .get::<NiFogProperty>(retained.fog_b401fc.unwrap())
                .unwrap()
                .flags_18,
            0
        );
        assert_eq!(
            world
                .get::<super::super::properties::MaterialProperty>(
                    retained.material_b3faa4.unwrap()
                )
                .unwrap()
                .values
                .ambient[0],
            0.5
        );
        assert_eq!(
            world
                .get::<NiShadeProperty>(retained.shade_b401ac.unwrap())
                .unwrap()
                .flags_18,
            1
        );
        assert_eq!(
            world
                .get::<StencilProperty>(retained.stencil_b3fcf8.unwrap())
                .unwrap()
                .mask_20,
            0xFFFF_FFFF
        );
        assert_eq!(
            world
                .get::<TexturingProperty>(retained.texturing_b3f974.unwrap())
                .unwrap()
                .maps_1c
                .len(),
            7
        );
        assert_eq!(
            world
                .get::<VertexColorProperty>(retained.vertex_color_b3f980.unwrap())
                .unwrap()
                .flags_18,
            8
        );
        assert_eq!(
            world
                .get::<NiImageConverterDefault>(retained.image_converter_b3fd28.unwrap())
                .unwrap()
                .object_index_buckets
                .len(),
            37
        );
        assert!(world.get_resource::<CollisionDispatchState>().is_none());
        let creators = world.resource::<NiD3DShaderCreatorRegistry>();
        assert!(!creators.initialized());
        assert!(creators.shader_factory().is_none());
        assert!(creators.active_shader_factory().is_none());
        assert!(creators.adapter_formats().is_empty());
        assert!(creators.runtime_pools().iter().all(|pool| {
            pool.original_global == 0 && pool.element_bytes == 0 && pool.initial_count == 0
        }));
        assert!(!creators.packer_registry().initialized());
        assert_eq!(creators.packer_registry().original_target(0, 0), None);
        assert!(creators.registrations().is_empty());
        assert!(creators.creator(ShaderCreatorKind::Hlsl).is_none());
        assert!(creators.creator(ShaderCreatorKind::Assembly).is_none());
        assert!(creators.creator(ShaderCreatorKind::Object).is_none());

        // The guards reset: every owner is new.
        assert_eq!(known.run(&mut world).unwrap().pending_initializers.len(), 5);
        assert_eq!(world.resource::<NiFactoryRegistry>().entries().len(), 189);
        let fresh = defaults(&world);
        assert!(fresh.alpha_b3fce4.is_some() && fresh.alpha_b3fce4 != retained.alpha_b3fce4);
        assert!(fresh.fog_b401fc.is_some() && fresh.fog_b401fc != retained.fog_b401fc);
        assert!(fresh.stencil_b3fcf8.is_some() && fresh.stencil_b3fcf8 != retained.stencil_b3fcf8);
        assert!(
            fresh.texturing_b3f974.is_some() && fresh.texturing_b3f974 != retained.texturing_b3f974
        );
        assert!(
            fresh.image_converter_b3fd28.is_some()
                && fresh.image_converter_b3fd28 != retained.image_converter_b3fd28
        );
        let fresh_material = fresh.material_b3faa4.unwrap();
        assert_ne!(Some(fresh_material), retained.material_b3faa4);
        assert!(
            world
                .get::<super::super::properties::MaterialProperty>(fresh_material)
                .unwrap()
                .material_id_18
                > retained_material_id
        );
        assert!(world.resource::<SingletonTail>().scratch_b40134.is_some());
        assert!(world.get_resource::<SqrtMantissaTable>().is_some());
        assert!(world.get_resource::<CollisionDispatchState>().is_some());
        let creators = world.resource::<NiD3DShaderCreatorRegistry>();
        assert!(creators.shader_factory().is_some());
        assert_eq!(creators.active_shader_factory(), creators.shader_factory());
        assert_eq!(creators.adapter_formats().len(), 3);
        assert_eq!(creators.runtime_pools()[4].element_bytes, 96);
        assert_eq!(
            creators.packer_registry().original_target(17, 0),
            Some(0x006F_7070)
        );
        assert_eq!(creators.registrations().len(), 5);
        known.shutdown(&mut world).unwrap();
    }

    #[derive(Resource, Default)]
    struct Order(Vec<i32>);

    fn push<const N: i32>(world: &mut World) -> Result<(), StartupCallbackError> {
        world.get_resource_or_init::<Order>().0.push(N);
        Ok(())
    }

    #[test]
    fn scheduler_order_capacity_and_processor_failure() {
        let mut world = World::new();
        let mut executable = StartupCallbackScheduler::default();
        executable.set_singleton(StartupCallback::new(0x10, 0x11, push::<1>, push::<4>));
        executable
            .register_pair(StartupCallback::new(0x20, 0x21, push::<2>, push::<5>))
            .unwrap();
        assert_eq!(
            executable
                .register_pair_once(0x30, StartupCallback::new(0x30, 0x31, push::<3>, push::<6>)),
            Ok(true)
        );
        assert_eq!(
            executable.register_pair_once(0x30, StartupCallback::identity(0x30, 0x31)),
            Ok(false),
            "a repeated counter appends no pair"
        );
        assert!(
            executable
                .run(&mut world)
                .unwrap()
                .pending_initializers
                .is_empty()
        );
        assert!(executable.shutdown(&mut world).unwrap().is_empty());
        assert_eq!(world.resource::<Order>().0, [1, 2, 3, 5, 6, 4]);

        let mut capacity = StartupCallbackScheduler::default();
        for index in 0..16 {
            capacity
                .register_pair(StartupCallback::identity(index + 1, index + 101))
                .unwrap();
        }
        assert_eq!(
            capacity.register_pair(StartupCallback::identity(17, 117)),
            Err(StartupCallbackError(
                "Original startup callback pair capacity exceeded"
            ))
        );
        assert_eq!(capacity.pair_count(), 16);

        let mut world = World::new();
        let mut no_processor_storage = StartupCallbackScheduler::with_processor_storage_failure();
        no_processor_storage
            .register_pair(StartupCallback::new(0x40, 0x41, push::<1>, push::<2>))
            .unwrap();
        assert_eq!(
            no_processor_storage
                .run(&mut world)
                .unwrap()
                .processor_count,
            0
        );
        assert_eq!(world.resource::<Order>().0, [1], "dispatch does not stop");
        assert_eq!(
            no_processor_storage
                .run(&mut world)
                .unwrap()
                .processor_count,
            0
        );
        assert_eq!(world.resource::<Order>().0, [1, 1]);
        assert!(
            no_processor_storage
                .shutdown(&mut world)
                .unwrap()
                .is_empty()
        );
        assert_eq!(world.resource::<Order>().0, [1, 1, 2], "cleanup is kept");
    }

    /// The scheduler runs as a resource, through `resource_scope`.
    #[test]
    fn scheduler_runs_as_a_resource() {
        let (mut world, known) = known_world();
        world.insert_resource(known);
        let pending = world
            .resource_scope(|world, mut scheduler: Mut<StartupCallbackScheduler>| {
                scheduler.run(world)
            })
            .unwrap();
        assert_eq!(pending.pending_initializers.len(), 5);
        assert_eq!(world.resource::<NiFactoryRegistry>().entries().len(), 189);
        world
            .resource_scope(|world, mut scheduler: Mut<StartupCallbackScheduler>| {
                scheduler.shutdown(world)
            })
            .unwrap();
        assert!(world.resource::<NiFactoryRegistry>().entries().is_empty());
    }

    #[test]
    fn scope_exit_runs_cleanup() {
        let mut world = World::new();
        // A failed startup exit still releases callback-owned objects.
        let outcome = (|| -> Result<(), StartupCallbackError> {
            let mut scheduler = StartupCallbackScheduler::default();
            register_known_crt_callbacks(&mut scheduler)?;
            let mut scope = StartupCallbackScope::new(&mut world, scheduler);
            scope.run()?;
            Err(StartupCallbackError("simulated startup exception"))
        })();
        assert_eq!(
            outcome,
            Err(StartupCallbackError("simulated startup exception"))
        );
        let creators = world.resource::<NiD3DShaderCreatorRegistry>();
        assert!(!creators.initialized());
        assert!(creators.shader_factory().is_none());
        assert!(creators.active_shader_factory().is_none());
        assert!(creators.adapter_formats().is_empty());
        assert!(creators.registrations().is_empty());
        assert!(world.get_resource::<CollisionDispatchState>().is_none());

        let mut world = World::new();
        {
            let mut scheduler = StartupCallbackScheduler::default();
            scheduler
                .register_pair(StartupCallback::new(0x20, 0x21, push::<1>, push::<2>))
                .unwrap();
            let mut scope = StartupCallbackScope::new(&mut world, scheduler);
            scope.run().unwrap();
        }
        assert_eq!(world.resource::<Order>().0, [1, 2]);
    }

    #[test]
    fn catalog_keys_and_winmain_factories() {
        let mut factories = NiFactoryRegistry::default();
        factories.register_winmain_bs_factories();
        assert_eq!(factories.entries().len(), 9);
        assert_eq!(factories.entries()[0].0, "BSReference");
        assert_eq!(factories.entries()[8].0, "BSPSysArrayEmitter");
        assert_eq!(factories.find("NiBSBoneLODController"), Some(0x006F_D6E0));
        assert_eq!(factories.find("UnknownNiClass"), None);
        factories.insert_or_assign("BSReference", 0x1234);
        assert_eq!(factories.entries().len(), 9);
        assert_eq!(factories.find("BSReference"), Some(0x1234));

        let mut keys = NiFactoryRegistry::default();
        keys.insert_or_assign("NiNode", 0x100);
        assert_eq!(keys.find("niNode"), Some(0x100));
        assert_eq!(keys.find("ninode"), None);
        keys.insert_or_assign("niNode", 0x200);
        keys.insert_or_assign("ninode", 0x300);
        assert_eq!(keys.entries().len(), 2);
        assert_eq!(keys.entries()[0].0, "niNode");
        assert_eq!(keys.find("NiNode"), Some(0x200));
        assert_eq!(keys.find("ninode"), Some(0x300));
        assert!(keys.erase("niNode"));
        assert_eq!(keys.find("NiNode"), None);
        assert_eq!(keys.find("ninode"), Some(0x300));
    }
}
