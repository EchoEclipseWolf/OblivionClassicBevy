//! The +0x3C PostLoad pass of 00714390.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::controller_sequence::controller_manager_post_load;
use super::mesh_geometry::MeshTriShapePayload;
use super::mesh_skin_owners::MeshSkinInstance;
use super::mesh_skin_postload::{MeshSkinPostLoadStatus, postload_mesh_skin_instance};
use super::particle_controller::ParticleEmitterController;
use super::particle_data::ParticleData;
use super::particle_modifier_value_controller::{
    ParticleGravityStrengthController, ParticleModifierActiveController,
};
use super::particle_rotation::ParticleRotationModifier;
use super::particle_system::ParticleSystem;
use super::stream_callbacks::{
    StreamRootCallbackControl, StreamRootCallbackResult, StreamRootCallbackRuntime,
    dispatch_stream_root_callbacks,
};
use super::stream_graph::StreamGraph;
use super::stream_objects::{NiClass, StreamObjectEntry, StreamObjectTable};
use super::texture::SourceTexture;

/// IDA: 00700160's modern scan removes exact RTTI B3FFA8, which no supported extra matches.
const MODERN_EXTRA_FLOOR: u32 = 0x0500_000B;
/// IDA: 006CACE0 skips its legacy conversion from here on.
const SEQUENCE_MODERN_FLOOR: u32 = 0x0A01_0068;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StreamPostLoadStatus {
    #[default]
    Complete,
    UnsupportedObject,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamPostLoadResult {
    pub status: StreamPostLoadStatus,
    pub object_index: Option<usize>,
    pub original_method: Option<u32>,
}

/// `rotation_target_data` resolves the actual modifier +0x10 target rather than inferring it from a system list:
/// original AddModifier and LinkObject can overwrite it. `precache_source_textures` is 007203F0's gate 00B256CC, set
/// by 00498340 during renderer init. `release_static_texture_pixel_data` is 00701C26's gate 00B3F958, set by WinMain
/// 0040E62F. `create_source_texture_renderer_data` is renderer 00B3F928 vtable +0x104: required when precaching, and
/// reported as unsupported rather than treated as an absent renderer. `invoke_unported` runs only after built-in
/// support reports an unsupported body.
#[derive(Default)]
pub struct StreamPostLoadRuntime<'a> {
    pub rotation_target_data: Option<&'a mut dyn FnMut(usize) -> Option<Entity>>,
    pub precache_source_textures: bool,
    pub release_static_texture_pixel_data: bool,
    pub create_source_texture_renderer_data: Option<CreateRendererData<'a>>,
    pub invoke_unported: Option<InvokeUnported<'a>>,
}

pub type CreateRendererData<'a> = &'a mut (dyn FnMut(&mut World, &mut SourceTexture) -> bool + 'a);
pub type InvokeUnported<'a> = &'a mut (dyn FnMut(&mut World, &StreamObjectEntry, u32) -> bool + 'a);

/// IDA: the concrete vtable +0x3C entries of the bound factories; 0068F970 is a one-instruction retn4. Completion: 100%
/// of the method identities.
/// TODO: [ ] Extend only with exact constructor and vtable evidence.
const fn post_load_method(constructor: u32) -> Option<u32> {
    Some(match constructor {
        0x0075_3BB0 | 0x0074_BB20 | 0x006E_3440 | 0x006D_2DD0 | 0x006E_8740 | 0x006E_4520
        | 0x0075_A360 | 0x0075_BD90 | 0x006D_9D40 | 0x0074_CFE0 | 0x0074_CC70 | 0x0074_D2D0
        | 0x0075_0BA0 | 0x0075_1D10 | 0x0075_8960 | 0x0075_A9E0 | 0x0075_5690 | 0x0075_6DA0
        | 0x006E_8370 | 0x006E_7E70 | 0x006D_7AD0 | 0x0071_6B20 | 0x0072_7F20 | 0x006D_7450
        | 0x0075_9C50
        | 0x0071_FD30 // NiTriShapeData
        | 0x0071_9E00 // NiTriStripsData; A7F32C+3C
        | 0x006E_B4C0 // NiBlendBoolInterpolator; A7C5B4+3C
        | 0x006D_24E0 // NiBlendFloatInterpolator; A7A49C+3C
        | 0x0072_CAA0 // NiSkinPartition
        | 0x0072_F210 // NiSkinData
        | 0x0075_4E40 // NiPSysResetOnLoopCtlr
        | 0x006E_1C00 // NiTransformData; A7B474+3C
        | 0x006D_66E0 // NiTransformInterpolator; A7A83C+3C
        | 0x006E_5BF0 // NiBSplineCompTransformInterpolator; A7BC5C+3C
        | 0x006E_6220 // NiBSplineCompFloatInterpolator; A7BDDC+3C
        | 0x006E_71E0 // NiBSplineData; A7C044+3C
        | 0x006E_7990 // NiBSplineBasisData; A7C0CC+3C
        | 0x006C_3F00 // NiTransformController; A79634+3C
        | 0x006D_4620 // NiVisController; A7A674+3C
        | 0x006C_BCE0 // NiBlendTransformInterpolator; A79E54+3C
        | 0x0071_6A40 // NiDefaultAVObjectPalette; A798C4+3C
        | 0x006D_C5E0 // NiPathInterpolator; A7AE34+3C
        | 0x006D_0260 // NiMultiTargetTransformController; A79984+3C
        | 0x006D_21D0 // NiFlipController; A7A33C+3C
        | 0x006D_7320 // NiTextureTransformController; A7A95C+3C
        | 0x006F_A900 // BSXFlags; A7D14C+3C
        | 0x006F_B340 // BSFurnitureMarker; A7D24C+3C
        | 0x006F_BA20 // BSBound; A7D2D4+3C
        | 0x006D_23D0 // NiAlphaController; A7A3EC+3C
        | 0x006D_EF40 // NiMaterialColorController; A7B024+3C
        | 0x006D_A740 // NiPoint3Interpolator; A7AD5C+3C
        | 0x006E_A8C0 // NiBlendPoint3Interpolator; A7C454+3C
        | 0x006D_1520 // NiGeomMorpherController; A7A23C+3C
        | 0x006D_DF90 // NiMorphData; A7AF64+3C
        | 0x006F_F000 // BSParentVelocityModifier; A7D53C+3C
        | 0x008B_71B0 // bhkSPCollisionObject; A655CC+3C
        | 0x008A_F070 // bhkSimpleShapePhantom; A55CBC+3C
        | 0x008C_8900 // bhkConvexVerticesShape; A996DC+3C
        | 0x0089_E960 // bhkCollisionObject; A55FCC+3C
        | 0x008A_41F0 // bhkRigidBody; A5605C+3C
        | 0x008B_97A0 // bhkRigidBodyT; A980A4+3C
        | 0x008C_62B0 // bhkNiTriStripsShape; A9952C+3C
        | 0x008C_33C0 // bhkMoppBvTreeShape; A9901C+3C
        | 0x008B_7E10 // bhkBoxShape; A6593C+3C
        | 0x008B_6780 // bhkCapsuleShape; A65794+3C
        | 0x008C_93F0 // bhkConvexTransformShape; A9978C+3C
        | 0x008A_0EF0 // bhkListShape; A96704+3C
        | 0x008A_1B80 // bhkTransformShape; A656F4+3C
        | 0x008C_09E0 // bhkRagdollConstraint; A98B24+3C
        | 0x008B_2BE0 // bhkLimitedHingeConstraint; A5653C+3C
        | 0x008C_17B0 // bhkPrismaticConstraint; A98C64+3C
        | 0x008A_A920 // bhkBlendController; A97AC4+3C
        | 0x008A_F380 // bhkSphereShape; A55D4C+3C
        | 0x0088_ED20 // bhkBlendCollisionObject; A9643C+3C
        | 0x008B_EFC0 // bhkMalleableConstraint; A987A4+3C
        => 0x0068_F970,
        0x0071_85B0 | 0x0070_6CB0 | 0x0070_6530 | 0x0070_9710 | 0x0071_8F80
        | 0x0070_BA70 // NiNode
        | 0x0072_2490 // NiBillboardNode; A45134+3C
        | 0x0071_97F0 // NiDirectionalLight; A7F1DC+3C
        | 0x0070_4A60 // NiTexturingProperty; A7DB5C+3C
        | 0x0073_D670 // NiSpecularProperty; A81484+3C

        => 0x0070_0160,
        0x0070_21F0 => 0x0072_03F0, // NiSourceTexture; A7D8CC+3C
        0x006C_7840 => 0x006C_ACE0, // NiControllerSequence; A79A34+3C
        0x006C_5BB0 => 0x006C_4260, // NiControllerManager; A79804+3C
        0x0074_FDE0 | 0x0075_6730 | 0x0075_7170 | 0x0075_7F40 => 0x0075_E6C0,
        0x0074_E1C0 => 0x0074_E2B0,
        0x0074_9FE0 | 0x0071_75C0 | 0x0071_9A40 => 0x0072_32B0,
        0x0072_BD10 => 0x0072_BE70, // NiSkinInstance
        0x006F_D6E0 => 0x006F_D7B0, // NiBSBoneLODController; A7D3D4+3C
        _ => return None,
    })
}

/// IDA: 0075E6C0 on a value that reads the world while written.
fn post_load_modifier<T: Component<Mutability = bevy_ecs::component::Mutable>>(
    world: &mut World,
    entity: Entity,
    run: impl FnOnce(&mut T, &World) -> Result<(), StreamError>,
) -> Result<bool, StreamError> {
    let Some(mut owner) = world.entity_mut(entity).take::<T>() else {
        return Ok(false);
    };
    let result = run(&mut owner, world);
    world.entity_mut(entity).insert(owner);
    result.map(|()| true)
}

/// IDA: the concrete PostLoad bodies. Completion: 100% of the current modern, no-extra-removal, no-shader and
/// NiSkinInstance no-op subset.
/// TODO: [ ] Shader retain, GetName and 007383F0, legacy skin transfer, and further extra classes.
fn invoke_supported_post_load(
    world: &mut World,
    entry: &StreamObjectEntry,
    method: u32,
    version: u32,
    object_index: usize,
    runtime: &mut StreamPostLoadRuntime<'_>,
) -> Result<bool, StreamError> {
    let entity = entry.entity;
    if world.get_entity(entity).is_err() {
        return Err(StreamError("null object in stream post-load table"));
    }
    match method {
        0x0068_F970 => Ok(true),
        0x006C_ACE0 => {
            if world.get::<NiClass>(entity) != Some(&NiClass::ControllerSequence) {
                return Err(StreamError("controller sequence PostLoad owner mismatch"));
            }
            Ok(version >= SEQUENCE_MODERN_FLOOR)
        }
        0x006C_4260 => {
            if world.get::<NiClass>(entity) != Some(&NiClass::ControllerManager) {
                return Err(StreamError("controller manager PostLoad owner mismatch"));
            }
            Ok(controller_manager_post_load(world, entity))
        }
        // 007203F0 does not chain to 00700160; only the renderer gate runs.
        0x0072_03F0 => {
            let precache = runtime.precache_source_textures;
            let release = runtime.release_static_texture_pixel_data;
            if precache && runtime.create_source_texture_renderer_data.is_none() {
                return Ok(false);
            }
            // The texture stands apart from the World for the renderer call, which spawns its pixel data.
            let mut owner = world
                .get::<SourceTexture>(entity)
                .ok_or(StreamError("source texture PostLoad owner mismatch"))?
                .clone();
            match runtime.create_source_texture_renderer_data.as_mut() {
                Some(create) => {
                    owner.post_load(
                        precache,
                        release,
                        Some(&mut |texture: &mut SourceTexture| create(world, texture)),
                    );
                }
                None => owner.post_load(precache, release, None),
            }
            *world
                .get_mut::<SourceTexture>(entity)
                .ok_or(StreamError("source texture PostLoad owner mismatch"))? = owner;
            Ok(true)
        }
        0x006F_D7B0 => {
            super::bone_lod::post_load_bone_lod(world, entity);
            Ok(true)
        }
        0x0072_BE70 => {
            let mut owner = world
                .get_mut::<MeshSkinInstance>(entity)
                .ok_or(StreamError("skin instance PostLoad owner mismatch"))?;
            Ok(postload_mesh_skin_instance(&mut owner, version, None)?
                == MeshSkinPostLoadStatus::Skipped)
        }
        0x0075_E6C0 => {
            let class = world.get::<NiClass>(entity).copied();
            match class {
                Some(NiClass::ParticleEmitterController) => {
                    post_load_modifier::<ParticleEmitterController>(
                        world,
                        entity,
                        |owner, world| owner.post_load_modifier(world),
                    )
                }
                Some(NiClass::ParticleModifierActiveController) => {
                    post_load_modifier::<ParticleModifierActiveController>(
                        world,
                        entity,
                        |owner, world| owner.base.post_load_modifier(world),
                    )
                }
                Some(NiClass::ParticleGravityStrengthController) => {
                    post_load_modifier::<ParticleGravityStrengthController>(
                        world,
                        entity,
                        |owner, world| owner.base.post_load_modifier(world),
                    )
                }
                Some(NiClass::ParticleEmitterLifeSpanController) => post_load_modifier::<
                    super::particle_modifier_value_controller::ParticleEmitterLifeSpanController,
                >(
                    world,
                    entity,
                    |owner, world| owner.base.post_load_modifier(world),
                ),
                _ => Err(StreamError("modifier controller PostLoad owner mismatch")),
            }
        }
        0x0074_E2B0 => {
            if world.get::<ParticleRotationModifier>(entity).is_none() {
                return Err(StreamError("rotation post-load owner mismatch"));
            }
            if version >= 0x1400_0002 {
                return Ok(true);
            }
            let Some(target) = runtime
                .rotation_target_data
                .as_mut()
                .and_then(|find| find(object_index))
            else {
                return Ok(false);
            };
            let modifier = world
                .get::<ParticleRotationModifier>(entity)
                .expect("checked above")
                .clone();
            let Some(mut data) = world.get_mut::<ParticleData>(target) else {
                return Ok(false);
            };
            modifier.apply_legacy_rotation_speeds(&mut data, version)?;
            Ok(true)
        }
        0x0072_32B0 => {
            if version < MODERN_EXTRA_FLOOR {
                return Ok(false);
            }
            if let Some(system) = world.get::<ParticleSystem>(entity) {
                return Ok(system.shader_flag_raw == 0);
            }
            let payload = world
                .get::<MeshTriShapePayload>(entity)
                .ok_or(StreamError("geometry post-load owner mismatch"))?;
            Ok(payload.shader_flag_raw == 0)
        }
        0x0070_0160 => Ok(version >= MODERN_EXTRA_FLOOR
            && world.get::<NiClass>(entity).is_some_and(|class| {
                class.is_property() || class.is_node() || *class == NiClass::DirectionalLight
            })),
        _ => Ok(false),
    }
}

/// IDA: 007144D6..0071450A; `next_object` is stream +0x270. Pause handoff, with no state 2 check inside the pass.
/// Completion: 100% of the ordered dispatch.
/// TODO: [ ] Original array mutation, state synchronization and ABI.
pub fn dispatch_stream_post_load(
    world: &mut World,
    table: &StreamObjectTable,
    stream_version: u32,
    next_object: &mut usize,
    control: &mut StreamRootCallbackControl<'_>,
    runtime: &mut StreamPostLoadRuntime<'_>,
) -> Result<StreamPostLoadResult, StreamError> {
    while *next_object < table.objects.len() {
        if control.load_state == 3 {
            control.load_state = 4;
            let wait = control
                .wait_for_resume
                .as_mut()
                .ok_or(StreamError("missing stream post-load pause service"))?;
            wait(&mut control.load_state);
        }
        let entry = &table.objects[*next_object];
        let method = post_load_method(entry.original_constructor);
        let mut supported = match method {
            Some(method) => invoke_supported_post_load(
                world,
                entry,
                method,
                stream_version,
                *next_object,
                runtime,
            )?,
            None => false,
        };
        if !supported && let Some(invoke) = runtime.invoke_unported.as_mut() {
            supported = invoke(world, entry, stream_version);
        }
        if !supported {
            return Ok(StreamPostLoadResult {
                status: StreamPostLoadStatus::UnsupportedObject,
                object_index: Some(*next_object),
                original_method: method,
            });
        }
        *next_object += 1;
    }
    Ok(StreamPostLoadResult::default())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamFinalizeResult {
    PostLoad(StreamPostLoadResult),
    Callbacks(StreamRootCallbackResult),
}

/// IDA: 00714390 runs every +0x3C body before the root callback tail, so an unported body shows before any callback.
/// Completion: 100% of the supported predecessor order.
/// TODO: [ ] 007126A0 terminal cleanup and error ownership.
pub fn finalize_stream_graph(
    world: &mut World,
    graph: &StreamGraph,
    stream_version: u32,
    next_post_load_object: &mut usize,
    control: &mut StreamRootCallbackControl<'_>,
    post_load_runtime: &mut StreamPostLoadRuntime<'_>,
    callback_runtime: &mut StreamRootCallbackRuntime<'_>,
) -> Result<StreamFinalizeResult, StreamError> {
    let post_load = dispatch_stream_post_load(
        world,
        &graph.table,
        stream_version,
        next_post_load_object,
        control,
        post_load_runtime,
    )?;
    if post_load.status != StreamPostLoadStatus::Complete {
        return Ok(StreamFinalizeResult::PostLoad(post_load));
    }
    dispatch_stream_root_callbacks(
        world,
        &graph.roots,
        stream_version,
        control,
        callback_runtime,
    )
    .map(StreamFinalizeResult::Callbacks)
}
