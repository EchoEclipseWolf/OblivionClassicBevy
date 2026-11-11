//! The leaf LoadBinary arms of the stream graph: 007008A0's modern NiObject prefix, then the class loader.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::animation::FloatKeyData;
use super::animation_stream::{decode_bool_key_payload, decode_float_key_payload};
use super::blend_interpolator::{
    ParticleBlendBoolInterpolator, ParticleBlendBoolPayload, ParticleBlendFloatInterpolator,
    ParticleBlendFloatPayload, parse_particle_blend_bool_payload,
    parse_particle_blend_float_payload,
};
use super::bool_animation::BoolKeyData;
use super::color_animation::{ColorKeyData, decode_color_key_payload};
use super::extra_data::{
    decode_binary_extra_data_payload, decode_string_extra_data_payload,
    load_text_key_extra_data_payload,
};
use super::mesh_geometry::{
    MeshTriShapePayload, parse_mesh_tri_shape_data_payload, parse_mesh_tri_shape_payload,
};
use super::mesh_skin_owners::{
    MeshSkinInstance, MeshSkinInstancePayload, parse_mesh_skin_data_payload,
    parse_mesh_skin_instance_payload,
};
use super::mesh_skin_partition::parse_mesh_skin_partition_payload;
use super::mesh_tri_strips::{parse_mesh_tri_strips_data_payload, parse_mesh_tri_strips_payload};
use super::particle_controller::{
    ParticleEmitterController, ParticleEmitterControllerPayload, ParticleTimeControllerPayload,
    load_particle_time_controller, parse_particle_emitter_controller_payload,
    parse_particle_time_controller_payload,
};
use super::particle_data::{ParticleData, parse_particle_data_payload};
use super::particle_mesh_emitter::{
    ParticleMeshEmitter, ParticleMeshEmitterPending, parse_particle_mesh_emitter_payload,
};
use super::particle_modifier_value_controller::{
    ParticleGravityStrengthController, ParticleModifierActiveController,
    ParticleModifierValueControllerPayload, parse_particle_modifier_value_controller_payload,
};
use super::particle_reset_controller::ParticleResetOnLoopController;
use super::position_animation::{PositionKeyData, load_position_key_payload};
use super::registration::{FactoryCreate, NiFactoryRegistry};
use super::scene_derived::{
    BillboardNodePending, DirectionalLightPending, apply_billboard_node_payload,
    apply_directional_light_payload, parse_billboard_node_payload, parse_directional_light_payload,
};
use super::scene_node::{SceneNodePayload, parse_scene_node_payload};
use super::stream_allocation_groups::StreamAllocationGroups;
use super::stream_objects::NiClass;
use super::string_palette::decode_string_palette_payload;
use super::transform_animation::{TransformKeyData, load_transform_key_payload};

const MODERN_FLOOR: u32 = 0x0A01_0072;
/// IDA: 10.0.1.0, the oldest version any ported class reads (the FaceGen ears).
const LEGACY_FLOOR: u32 = 0x0A00_0100;
const TIME_CONTROLLER_FLOOR: u32 = 0x0A01_006D;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactoryPayloadStatus {
    Unregistered,
    Decoded,
    UnsupportedTarget,
    UnsupportedVersion,
    OwnerMismatch,
}

/// A leaf loader's references that wait for the link pass.
#[derive(Clone, Debug, Default)]
pub enum FactoryPendingPayload {
    #[default]
    None,
    TimeController(ParticleTimeControllerPayload),
    EmitterController(ParticleEmitterControllerPayload),
    ModifierValueController(ParticleModifierValueControllerPayload),
    BlendBool(ParticleBlendBoolPayload),
    BlendFloat(ParticleBlendFloatPayload),
    MeshEmitter(ParticleMeshEmitterPending),
    Geometry(MeshTriShapePayload),
    SkinInstance(MeshSkinInstancePayload),
    Node(SceneNodePayload),
    Billboard(BillboardNodePending),
    Light(DirectionalLightPending),
}

#[derive(Debug)]
pub struct FactoryPayloadLoadResult {
    pub status: FactoryPayloadStatus,
    pub bytes_consumed: usize,
    pub pending: FactoryPendingPayload,
}

// IDA: N/A a helper, with no address of its own.
const fn stop(status: FactoryPayloadStatus) -> FactoryPayloadLoadResult {
    FactoryPayloadLoadResult {
        status,
        bytes_consumed: 0,
        pending: FactoryPendingPayload::None,
    }
}

/// IDA: N/A the constructors this module loads and the class tag each owner must carry.
const fn leaf_class(constructor: u32) -> Option<NiClass> {
    Some(match constructor {
        0x006D_9D40 => NiClass::PositionKeyData,
        0x006E_3440 => NiClass::FloatKeyData,
        0x006E_8740 => NiClass::BoolKeyData,
        0x0075_9C50 => NiClass::ParticleData,
        0x006E_4520 => NiClass::ColorKeyData,
        0x006D_7AD0 => NiClass::StringPalette,
        0x0071_6B20 => NiClass::StringExtraData,
        0x0072_7F20 => NiClass::BinaryExtraData,
        0x006D_7450 => NiClass::TextKeyExtraData,
        0x006F_B340 => NiClass::BsFurnitureMarker,
        0x006F_BA20 => NiClass::BsBound,
        0x0075_3BB0 => NiClass::ParticleUpdateController,
        0x0074_FDE0 => NiClass::ParticleEmitterController,
        0x0075_6730 => NiClass::ParticleModifierActiveController,
        0x0075_7170 => NiClass::ParticleGravityStrengthController,
        0x0075_7C30 => NiClass::ParticleEmitterSpeedController,
        0x0075_7F40 => NiClass::ParticleEmitterLifeSpanController,
        0x006E_B4C0 => NiClass::BlendBoolInterpolator,
        0x006D_24E0 => NiClass::BlendFloatInterpolator,
        0x0074_BB20 => NiClass::ParticleMeshEmitter,
        0x0071_75C0 => NiClass::TriShape,
        0x0071_FD30 => NiClass::TriShapeData,
        0x0071_9A40 => NiClass::TriStrips,
        0x0071_9E00 => NiClass::TriStripsData,
        0x0072_2490 => NiClass::BillboardNode,
        0x0071_97F0 => NiClass::DirectionalLight,
        0x0072_CAA0 => NiClass::SkinPartition,
        0x0072_F210 => NiClass::SkinData,
        0x0072_BD10 => NiClass::SkinInstance,
        0x0070_BA70 => NiClass::Node,
        0x0075_4E40 => NiClass::ParticleResetOnLoopController,
        0x006E_1C00 => NiClass::TransformKeyData,
        _ => return None,
    })
}

// IDA: N/A a helper, with no address of its own.
fn component_mut<'w, T: Component<Mutability = bevy_ecs::component::Mutable>>(
    world: &'w mut World,
    entity: Entity,
) -> Result<Mut<'w, T>, GameError> {
    world.get_mut::<T>(entity).ok_or(GameError::unclassified(
        0,
        "factory payload owner lacks its component",
    ))
}

/// IDA: N/A the host's leaf LoadBinary dispatch: 007008A0's modern NiObject prefix (no bytes) followed by the verified
/// class loaders. Parsing finishes before a value is published to the existing owner. Completion: 100% of these leaves.
/// TODO: [ ] Older NiObject base fields.
pub fn load_factory_payload_into_owner(
    world: &mut World,
    constructor: u32,
    entity: Entity,
    bytes: &[u8],
    version: u32,
    groups: Option<&mut StreamAllocationGroups>,
) -> Result<FactoryPayloadLoadResult, GameError> {
    use FactoryPayloadStatus as Status;
    let Some(class) = leaf_class(constructor) else {
        return Ok(stop(Status::UnsupportedTarget));
    };
    // Below 0A010072 only the classes whose older gates are ported (the ears' seven, as stream_graph's
    // `legacy_payload_version`) read, down to 10.0.1.0.
    let legacy_ported = matches!(
        class,
        NiClass::Node
            | NiClass::TriStrips
            | NiClass::TriStripsData
            | NiClass::MaterialProperty
            | NiClass::TexturingProperty
            | NiClass::SourceTexture
            | NiClass::DirectionalLight
    );
    if version < MODERN_FLOOR && !(legacy_ported && version >= LEGACY_FLOOR) {
        return Ok(stop(Status::UnsupportedVersion));
    }
    if world.get::<NiClass>(entity) != Some(&class) {
        return Ok(stop(Status::OwnerMismatch));
    }
    let mut pending = FactoryPendingPayload::None;
    let consumed = match constructor {
        0x006D_9D40 => {
            let mut owner = component_mut::<PositionKeyData>(world, entity)?;
            load_position_key_payload(&mut owner, bytes, version)?
        }
        // 006E2180: rotation, position, then scale; a zero-count block keeps prior owner state.
        0x006E_1C00 => {
            let mut owner = component_mut::<TransformKeyData>(world, entity)?;
            load_transform_key_payload(&mut owner, bytes, version)?
        }
        0x006E_3440 => {
            let decoded = decode_float_key_payload(bytes)?;
            if !decoded.data.keys().is_empty() {
                *component_mut::<FloatKeyData>(world, entity)? = decoded.data;
            }
            decoded.bytes_consumed
        }
        0x006E_8740 => {
            let decoded = decode_bool_key_payload(bytes, version)?;
            if !decoded.data.keys().is_empty() {
                *component_mut::<BoolKeyData>(world, entity)? = decoded.data;
            }
            decoded.bytes_consumed
        }
        0x0075_9C50 => {
            let decoded = parse_particle_data_payload(bytes, version, groups)?;
            *component_mut::<ParticleData>(world, entity)? = decoded.data;
            decoded.bytes_consumed
        }
        0x006E_4520 => {
            let decoded = decode_color_key_payload(bytes, version)?;
            if !decoded.data.keys().is_empty() {
                *component_mut::<ColorKeyData>(world, entity)? = decoded.data;
            }
            decoded.bytes_consumed
        }
        0x006D_7AD0 => {
            let (palette, consumed) = decode_string_palette_payload(bytes)?;
            world.entity_mut(entity).insert(palette);
            consumed
        }
        0x0071_6B20 => {
            let (name, data, consumed) = decode_string_extra_data_payload(bytes, version)?;
            world.entity_mut(entity).insert((name, data));
            consumed
        }
        0x0072_7F20 => {
            let (name, data, consumed) = decode_binary_extra_data_payload(bytes, version)?;
            world.entity_mut(entity).insert((name, data));
            consumed
        }
        0x006D_7450 => load_text_key_extra_data_payload(world, entity, bytes, version)?,
        // 006FB4E0; 00B3F9A0 is two pi (0x40C90FDB) by its static initializer.
        0x006F_B340 => {
            let (name, data, consumed) = super::extra_data::decode_furniture_marker_payload(
                bytes,
                version,
                f32::from_bits(0x40C9_0FDB),
            )?;
            world.entity_mut(entity).insert((name, data));
            consumed
        }
        // 006FB910.
        0x006F_BA20 => {
            let (name, data, consumed) = super::extra_data::decode_bound_payload(bytes, version)?;
            world.entity_mut(entity).insert((name, data));
            consumed
        }
        // 00754E80 -> 00715F40: scalars load now; next and target wait for 00715E70.
        0x0075_3BB0 => {
            if version < TIME_CONTROLLER_FLOOR {
                return Ok(stop(Status::UnsupportedVersion));
            }
            let decoded = parse_particle_time_controller_payload(bytes, version)?;
            load_particle_time_controller(world, entity, &decoded)?;
            let consumed = decoded.bytes_consumed;
            pending = FactoryPendingPayload::TimeController(decoded);
            consumed
        }
        // 0074F980 stores scalars and the name. Its immediate +0x3C needs the table, so the graph's own arm binds it;
        // this standalone decoder keeps the raw indexes.
        0x0074_FDE0 => {
            if version < TIME_CONTROLLER_FLOOR {
                return Ok(stop(Status::UnsupportedVersion));
            }
            let decoded = parse_particle_emitter_controller_payload(bytes, version)?;
            component_mut::<ParticleEmitterController>(world, entity)?
                .apply_loaded_payload(&decoded);
            let consumed = decoded.bytes_consumed;
            pending = FactoryPendingPayload::EmitterController(decoded);
            consumed
        }
        // 006CE320's +0x3C needs the table too; the graph's arm binds it between these two stages.
        0x0075_6730 | 0x0075_7170 | 0x0075_7F40 | 0x0075_7C30 => {
            if version < TIME_CONTROLLER_FLOOR {
                return Ok(stop(Status::UnsupportedVersion));
            }
            let decoded = parse_particle_modifier_value_controller_payload(bytes, version)?;
            if constructor == 0x0075_6730 {
                let mut owner = component_mut::<ParticleModifierActiveController>(world, entity)?;
                owner.base.apply_loaded_prefix(&decoded);
                owner.base.apply_loaded_name(&decoded);
            } else if constructor == 0x0075_7170 {
                let mut owner = component_mut::<ParticleGravityStrengthController>(world, entity)?;
                owner.base.apply_loaded_prefix(&decoded);
                owner.base.apply_loaded_name(&decoded);
            } else {
                let mut owner = component_mut::<
                    super::particle_modifier_value_controller::ParticleEmitterFloatController,
                >(world, entity)?;
                owner.base.apply_loaded_prefix(&decoded);
                owner.base.apply_loaded_name(&decoded);
            }
            let consumed = decoded.bytes_consumed;
            pending = FactoryPendingPayload::ModifierValueController(decoded);
            consumed
        }
        // The graph's arms bind each immediate reference before the derived +0x30 value store.
        0x006E_B4C0 => {
            let decoded = parse_particle_blend_bool_payload(bytes)?;
            let mut owner = component_mut::<ParticleBlendBoolInterpolator>(world, entity)?;
            owner.apply_loaded_prefix(&decoded);
            owner.apply_loaded_value(&decoded);
            let consumed = decoded.bytes_consumed;
            pending = FactoryPendingPayload::BlendBool(decoded);
            consumed
        }
        0x006D_24E0 => {
            let decoded = parse_particle_blend_float_payload(bytes)?;
            let mut owner = component_mut::<ParticleBlendFloatInterpolator>(world, entity)?;
            owner.apply_loaded_prefix(&decoded);
            owner.apply_loaded_value(&decoded);
            let consumed = decoded.bytes_consumed;
            pending = FactoryPendingPayload::BlendFloat(decoded);
            consumed
        }
        // 0074BB40 reads the NiPSysEmitter prefix and raw mesh references; 0074CA80 links them.
        0x0074_BB20 => {
            let decoded = parse_particle_mesh_emitter_payload(bytes, version)?;
            component_mut::<ParticleMeshEmitter>(world, entity)?.apply_loaded_payload(&decoded);
            let consumed = decoded.bytes_consumed;
            pending = FactoryPendingPayload::MeshEmitter(decoded);
            consumed
        }
        0x0071_75C0 | 0x0071_9A40 => {
            let decoded = if constructor == 0x0071_75C0 {
                parse_mesh_tri_shape_payload(bytes, version)?
            } else {
                parse_mesh_tri_strips_payload(bytes, version)?
            };
            let consumed = decoded.bytes_consumed;
            world.entity_mut(entity).insert(decoded.clone());
            pending = FactoryPendingPayload::Geometry(decoded);
            consumed
        }
        0x0071_FD30 => {
            let decoded = parse_mesh_tri_shape_data_payload(bytes, version, groups)?;
            world.entity_mut(entity).insert(decoded.data);
            decoded.bytes_consumed
        }
        0x0071_9E00 => {
            let decoded = parse_mesh_tri_strips_data_payload(bytes, version, groups)?;
            world.entity_mut(entity).insert(decoded.data);
            decoded.bytes_consumed
        }
        0x0072_CAA0 => {
            let decoded = parse_mesh_skin_partition_payload(bytes, version)?;
            world.entity_mut(entity).insert(decoded.owner);
            decoded.bytes_consumed
        }
        0x0072_F210 => {
            let decoded = parse_mesh_skin_data_payload(bytes, version)?;
            world.entity_mut(entity).insert(decoded.owner);
            decoded.bytes_consumed
        }
        0x0072_BD10 => {
            let decoded = parse_mesh_skin_instance_payload(bytes, version)?;
            let consumed = decoded.bytes_consumed;
            component_mut::<MeshSkinInstance>(world, entity)?.pending = decoded.clone();
            pending = FactoryPendingPayload::SkinInstance(decoded);
            consumed
        }
        0x0070_BA70 => {
            let decoded = parse_scene_node_payload(bytes, version)?;
            let consumed = decoded.bytes_consumed;
            world.entity_mut(entity).insert(decoded.clone());
            pending = FactoryPendingPayload::Node(decoded);
            consumed
        }
        0x0072_2490 => {
            let decoded = parse_billboard_node_payload(bytes, version)?;
            let consumed = decoded.bytes_consumed;
            apply_billboard_node_payload(world, entity, decoded.clone());
            pending = FactoryPendingPayload::Billboard(decoded);
            consumed
        }
        0x0071_97F0 => {
            let decoded = parse_directional_light_payload(bytes, version)?;
            let consumed = decoded.bytes_consumed;
            apply_directional_light_payload(world, entity, decoded.clone());
            pending = FactoryPendingPayload::Light(decoded);
            consumed
        }
        0x0075_4E40 => {
            let decoded = ParticleResetOnLoopController::parse_payload(bytes, version)?;
            component_mut::<ParticleResetOnLoopController>(world, entity)?
                .apply_loaded_payload(&decoded);
            let consumed = decoded.bytes_consumed;
            pending = FactoryPendingPayload::TimeController(decoded);
            consumed
        }
        _ => return Ok(stop(Status::UnsupportedTarget)),
    };
    Ok(FactoryPayloadLoadResult {
        status: Status::Decoded,
        bytes_consumed: consumed,
        pending,
    })
}

/// IDA: N/A the outcome of `decode_registered_factory_payload`. `entity` holds the loaded object only on `Decoded`.
#[derive(Debug)]
pub struct FactoryPayloadResult {
    pub status: FactoryPayloadStatus,
    pub requested_name: String,
    pub original_constructor: Option<u32>,
    pub bytes_consumed: usize,
    pub entity: Option<Entity>,
    pub pending: FactoryPendingPayload,
}

/// IDA: N/A the host's dispatch through the 00712590 registry, then 007008A0's modern no-op NiObject prefix. Completion: 100% of the host leaf loading
/// through the shared existing-owner decoder; a failed load despawns the cold object, as dropping it does in C++.
/// TODO: [ ] A verified block producer and graph link resolution.
pub fn decode_registered_factory_payload(
    world: &mut World,
    registry: &NiFactoryRegistry,
    class_name: &str,
    bytes: &[u8],
    version: u32,
) -> Result<FactoryPayloadResult, GameError> {
    let requested_name = class_name.split('\0').next().unwrap_or(class_name);
    let mut result = FactoryPayloadResult {
        status: FactoryPayloadStatus::Unregistered,
        requested_name: requested_name.to_owned(),
        original_constructor: registry.find(class_name),
        bytes_consumed: 0,
        entity: None,
        pending: FactoryPendingPayload::None,
    };
    let Some(constructor) = result.original_constructor else {
        return Ok(result);
    };
    if leaf_class(constructor).is_none() {
        result.status = FactoryPayloadStatus::UnsupportedTarget;
        return Ok(result);
    }
    if version < MODERN_FLOOR {
        result.status = FactoryPayloadStatus::UnsupportedVersion;
        return Ok(result);
    }
    let FactoryCreate::Created { entity, .. } = registry.create(world, class_name) else {
        return Err(GameError::unclassified(
            0,
            "registered leaf lacks a host constructor",
        ));
    };
    let loaded =
        match load_factory_payload_into_owner(world, constructor, entity, bytes, version, None) {
            Ok(loaded) => loaded,
            Err(error) => {
                world.despawn(entity);
                return Err(error);
            }
        };
    match loaded.status {
        FactoryPayloadStatus::Decoded => {}
        FactoryPayloadStatus::UnsupportedVersion => {
            world.despawn(entity);
            result.status = loaded.status;
            return Ok(result);
        }
        _ => {
            world.despawn(entity);
            return Err(GameError::unclassified(
                0,
                "registered leaf owner differs from payload decoder",
            ));
        }
    }
    result.status = loaded.status;
    result.bytes_consumed = loaded.bytes_consumed;
    result.entity = Some(entity);
    result.pending = loaded.pending;
    Ok(result)
}
