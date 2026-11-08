//! NiSingleInterpController subclasses that target a NiAVObject: NiTransformController / NiKeyframeController and
//! NiVisController, plus NiMultiTargetTransformController, and the NiTimeController state they share.
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.

use std::mem::take;

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::bool_animation::{update_bool_interpolator, update_bool_timeline_interpolator};
use super::controller_time::{
    ControllerTimeFields, ParticleControllerTimeCache, evaluate_particle_controller_time,
};
use super::mesh_geometry::MeshTriShapePayload;
use super::object_net::NiObjectNet;
use super::particle_blend_interpolator::{
    ParticleBlendSampleStatus, ParticleBlendTransformInterpolator, sample_particle_blend_bool,
    sample_particle_blend_transform,
};
use super::particle_controller::{
    ParticleTimeControllerCold, ParticleTimeControllerPayload, TimeGate, evaluate_time_gate,
    parse_particle_time_controller_payload,
};
use super::particle_system::ParticleSystem;
use super::path_interpolator::update_path_interpolator;
use super::scene_derived::DirectionalLight;
use super::scene_node::SceneNodePayload;
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};
use super::transform::NiTransform;
use super::transform_animation::{
    INVALID_TRANSFORM_COMPONENT, QuatTransform, TransformUpdateStatus,
    update_transform_interpolator,
};
use crate::bs::scene_graphs::SceneCamera;

const TRUNCATED_SINGLE: &str = "truncated NiSingleInterpController payload";
const TRUNCATED_MULTI: &str = "truncated NiMultiTargetTransformController payload";

/// IDA: 006CE320 -> 006D0550 -> 00715F40 modern LoadBinary shared by 006C3650 (transform) and 006D2250 (bool, vis):
/// NiTimeController fields, then (>= 0A010068) the immediate +0x3C interpolator.
/// Completion: 100% of the bounded >= 0A01006D field order; the [0A010068, 0A01006D) manager bool and the < 0A010068 deferred
/// legacy data reference are excluded.
/// TODO: [ ] The legacy deferred-data conversions in 006C3DB0 / 006D44F0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SingleInterpControllerPayload {
    pub time: ParticleTimeControllerPayload,
    pub interpolator_index: u32,
    pub bytes_consumed: usize,
}

impl Default for SingleInterpControllerPayload {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            time: ParticleTimeControllerPayload::default(),
            interpolator_index: NULL_REFERENCE,
            bytes_consumed: 0,
        }
    }
}

// IDA: 006CE320 NiSingleInterpController_LoadBinary over 006D0550 and 00715F40: the time fields, at 10.1.0.106 006D0550's
// bool for flag 0x20, then (from 0A010068) the interpolator reference.
// Completion: 100% of the supported widths and of 10.1.0.106.
// TODO: [ ] Original NiStream short-read callbacks.
pub fn parse_single_interp_controller_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<SingleInterpControllerPayload, StreamError> {
    if stream_version < 0x0A01_006D && stream_version != 0x0A01_006A {
        return Err(StreamError(
            "legacy NiSingleInterpController payload unsupported",
        ));
    }
    let mut time = parse_particle_time_controller_payload(bytes, stream_version)?;
    let mut reader = ByteReader::new(&bytes[time.bytes_consumed..]);
    // 006D0550 NiInterpController_LoadBinary: from 0A010068 to below 0A01006D a bool sets or clears flags bit 0x20.
    if (0x0A01_0068..0x0A01_006D).contains(&stream_version) {
        if reader.u8(TRUNCATED_SINGLE)? != 0 {
            time.flags |= 0x20;
        } else {
            time.flags &= !0x20;
        }
    }
    let interpolator_index = reader.u32(TRUNCATED_SINGLE)?;
    Ok(SingleInterpControllerPayload {
        time,
        interpolator_index,
        bytes_consumed: time.bytes_consumed + reader.position(),
    })
}

/// A substitute for vtable +0x64, the time map.
pub type TimeEvaluator<'a> = &'a mut dyn FnMut(&mut ControllerTimeFields, f32) -> f32;

/// A resolved 00715E70 pair: +0x34 next, then +0x30 target.
pub type ControllerLinks = (Option<Entity>, Option<Entity>);

/// What supplies vtable +0x64, the time map. The C++ port passes a `std::function`, which may be empty.
pub enum ControllerTimeMap<'a> {
    /// The empty function: reaching the map is a host error.
    Unresolved,
    /// 007155A0 over the world's one `ParticleControllerTimeCache`.
    SharedCache,
    /// A test's or caller's substitute.
    Custom(TimeEvaluator<'a>),
}

// IDA: shared head of 006C3C40 / 006D4470 / 006D1960 / 006D6CE0: flag 0x20 stores -FLT_MAX to +0x28; otherwise the 006C36B0
// gate, and a skip returns unless the interpolator's +0x94 is true (0069D990 false for every bound class). Returns true when
// the interpolator Update should run.
// Completion: 100% of the verified branch order with the supplied 007155A0 evaluator.
pub fn begin_single_interp_update(
    time: &mut ParticleTimeControllerCold,
    current_time: f32,
    evaluate: Option<TimeEvaluator<'_>>,
) -> Result<bool, StreamError> {
    if time.fields.flags & 0x0020 != 0 {
        time.last_value_28 = INVALID_TRANSFORM_COMPONENT;
        return Ok(true);
    }
    match evaluate_time_gate(time, current_time, evaluate) {
        TimeGate::Skip => Ok(false),
        TimeGate::Proceed => Ok(true),
        TimeGate::MissingEvaluator => Err(StreamError("node controller time map unresolved")),
    }
}

// IDA: N/A runs `begin_single_interp_update` with the requested time map. The exclusive world borrow replaces the C++ mutex.
// Completion: 100%
pub fn begin_controller_update(
    world: &mut World,
    time: &mut ParticleTimeControllerCold,
    current_time: f32,
    map: ControllerTimeMap<'_>,
) -> Result<bool, StreamError> {
    match map {
        ControllerTimeMap::Unresolved => begin_single_interp_update(time, current_time, None),
        ControllerTimeMap::Custom(evaluate) => {
            begin_single_interp_update(time, current_time, Some(evaluate))
        }
        ControllerTimeMap::SharedCache => {
            let mut cache = *world.get_resource_or_init::<ParticleControllerTimeCache>();
            let result = begin_single_interp_update(
                time,
                current_time,
                Some(&mut |fields, now| evaluate_particle_controller_time(fields, now, &mut cache)),
            );
            *world.resource_mut::<ParticleControllerTimeCache>() = cache;
            result
        }
    }
}

// IDA: 0047C600 thiscall quaternion (w, x, y, z) to 3x3 rotation stored in NiAVObject +0x30 order: f32 doubled components, f32
// products, then the nine 1 - (a + b) / sum / difference entries.
// Completion: 100% of the verified operand order and store points.
#[must_use]
pub fn quaternion_to_rotation_matrix(quaternion: &[f32; 4]) -> [f32; 9] {
    let w = quaternion[0];
    let x = quaternion[1];
    let y = quaternion[2];
    let z = quaternion[3];
    let x2 = x * 2.0;
    let y2 = y * 2.0;
    let z2 = 2.0 * z;
    let wx = w * x2;
    let wy = w * y2;
    let wz = w * z2;
    let xx = x2 * x;
    let xy = y2 * x;
    let xz = z2 * x;
    let yy = y2 * y;
    let yz = y * z2;
    let zz = z2 * z;
    let mut matrix = [0.0f32; 9];
    matrix[0] = 1.0 - (zz + yy);
    matrix[3] = xy + wz;
    matrix[6] = xz - wy;
    matrix[1] = xy - wz;
    matrix[4] = 1.0 - (zz + xx);
    matrix[7] = yz + wx;
    matrix[2] = xz + wy;
    matrix[5] = yz - wx;
    matrix[8] = 1.0 - (xx + yy);
    matrix
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeControllerUpdateStatus {
    Skipped,
    NoInterpolator,
    InterpolatorFailed,
    Applied,
    /// Host stop: the original dereferences +0x30 without a null check.
    UnsupportedMissingTarget,
    UnsupportedInterpolatorRange,
    /// Host stop: an expired or unsupported active sequence under 006C4200.
    UnsupportedSequenceUpdate,
}

/// IDA: 00715990 NiTimeController cold state, with its +0x34 next (strong) and +0x30 target (raw) links, shared by every node
/// controller class and NiControllerManager. +0x3C sits in the class's own component.
/// Completion: 100% of the host values.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct NodeController {
    pub time: ParticleTimeControllerCold,
    pub pending_interpolator_index: u32,
    pub links_resolved: bool,
}

impl Default for NodeController {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            time: ParticleTimeControllerCold::default(),
            pending_interpolator_index: NULL_REFERENCE,
            links_resolved: false,
        }
    }
}

impl NodeController {
    // IDA: 00715F40 scalar and deferred-index publication before 006CE320.
    // Completion: 100% of the supported host field order.
    pub const fn apply_loaded_prefix(&mut self, payload: &SingleInterpControllerPayload) {
        self.time.apply_loaded_payload(&payload.time);
        self.pending_interpolator_index = payload.interpolator_index;
    }

    // IDA: 00715E70: next (strong) first, then raw target.
    // Completion: 100% of the modern >= 0A010068 association; legacy conversion is excluded.
    // TODO: [ ] Original intrusive refcount swap timing.
    pub fn link_modern(
        &mut self,
        next: Option<Entity>,
        target: Option<Entity>,
        stream_version: u32,
    ) -> Result<(), StreamError> {
        if stream_version < 0x0A01_0068 {
            return Err(StreamError("legacy node controller link unsupported"));
        }
        if next.is_some() != (self.time.pending_next_index != NULL_REFERENCE) {
            return Err(StreamError("node controller next index/owner mismatch"));
        }
        if target.is_some() != (self.time.pending_target_index != NULL_REFERENCE) {
            return Err(StreamError("node controller target index/owner mismatch"));
        }
        self.time.next_34 = next;
        self.time.next_linked = next.is_some();
        self.time.target_30 = target;
        self.time.target_linked = target.is_some();
        self.links_resolved = true;
        Ok(())
    }
}

// IDA: 00715E70 modern NiTimeController link shared by the node controllers (the C++ stream graph's
// `resolve_node_controller_links`): the next controller first, then the raw target NiNode. `Ok(None)` reports an unsupported
// class, where the C++ link returns false.
// Completion: 100% of the supported owners.
pub fn resolve_node_controller_links(
    world: &World,
    time: &ParticleTimeControllerPayload,
    table: &StreamObjectTable,
) -> Result<Option<ControllerLinks>, StreamError> {
    let mut next = None;
    if time.next_index != NULL_REFERENCE {
        let entry = table
            .objects
            .get(time.next_index as usize)
            .ok_or(StreamError("node controller next out of range"))?;
        if !world
            .get::<NiClass>(entry.entity)
            .is_some_and(|class| class.is_node_controller())
        {
            return Ok(None);
        }
        next = Some(entry.entity);
    }
    let mut target = None;
    if time.target_index != NULL_REFERENCE {
        let entry = table
            .objects
            .get(time.target_index as usize)
            .ok_or(StreamError("node controller target out of range"))?;
        if !world
            .get::<NiClass>(entry.entity)
            .is_some_and(|class| class.is_node())
        {
            return Ok(None);
        }
        target = Some(entry.entity);
    }
    Ok(Some((next, target)))
}

// IDA: the 00715E70 link on an already loaded node controller. `Ok(false)` where the C++ link reports an unsupported owner; the
// controller is unchanged on any failure.
// Completion: 100% of the supported owners.
pub fn link_node_controller(
    world: &mut World,
    controller: Entity,
    time: &ParticleTimeControllerPayload,
    table: &StreamObjectTable,
    stream_version: u32,
) -> Result<bool, StreamError> {
    let Some((next, target)) = resolve_node_controller_links(world, time, table)? else {
        return Ok(false);
    };
    world
        .get_mut::<NodeController>(controller)
        .ok_or(StreamError("node controller link target missing"))?
        .link_modern(next, target, stream_version)?;
    Ok(true)
}

// IDA: 00700050's +0x0C head on a NiNode, which `link_scene_node` expects published first. `Ok(false)` for a class the C++ stream graph's
// `node_controller_owner` refuses; a null reference publishes nothing.
// Completion: 100% of the bound classes.
pub fn publish_node_controller_head(
    world: &mut World,
    node: Entity,
    controller_index: u32,
    table: &StreamObjectTable,
) -> Result<bool, StreamError> {
    if controller_index == NULL_REFERENCE {
        return Ok(true);
    }
    let entry = table
        .objects
        .get(controller_index as usize)
        .ok_or(StreamError("NiNode controller reference out of range"))?;
    if !world
        .get::<NiClass>(entry.entity)
        .is_some_and(|class| class.is_node_controller())
    {
        return Ok(false);
    }
    world
        .get_mut::<NiObjectNet>(node)
        .ok_or(StreamError("NiNode controller head owner missing"))?
        .controller_0c = Some(entry.entity);
    Ok(true)
}

/// IDA: 006C3F00 NiTransformController / NiKeyframeController factory: 0x40 bytes, 006CE1D0 base, vtable A79634. +0x3C holds a
/// NiTransformInterpolator (006D60B0), NiBlendTransformInterpolator (006CC3A0) or NiPathInterpolator (006DC2C0).
/// Completion: 100% of the cold state, modern load, immediate interpolator, link and the 006C3C40 target writes for the
/// supported owner types.
/// TODO: [ ] Other interpolator classes, non-node targets and the NiNode update scheduler that calls Update.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransformController {
    pub interpolator_3c: Option<Entity>,
}

/// IDA: 006D4620 NiVisController factory: 0x40 bytes, 006EC630 NiBoolInterpController base over 006CE1D0, vtable A7A674.
/// Completion: 100% of the cold state, 006D2250 modern load, 006D44F0 link and 006D4470 Update.
/// TODO: [ ] The NiNode update scheduler and other target families.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisController {
    pub interpolator_3c: Option<Entity>,
}

// IDA: N/A a cold node controller entity.
// Completion: 100%
pub fn spawn_node_controller(world: &mut World, class: NiClass, state: impl Bundle) -> Entity {
    world.spawn((class, NodeController::default(), state)).id()
}

// IDA: N/A publishes the parsed prefix, and 006CE320's immediate +0x3C reference through `store`. `Ok(false)` for a class
// `accepts` refuses. From the C++ stream graph's 006C3F00 and 006D4620 cases.
// Completion: 100% of the order: inherited fields, then the interpolator.
fn load_single_interp(
    world: &mut World,
    controller: Entity,
    payload: &SingleInterpControllerPayload,
    table: &StreamObjectTable,
    accepts: fn(NiClass) -> bool,
    out_of_range: &'static str,
    store: fn(&mut World, Entity, Entity),
) -> Result<bool, StreamError> {
    world
        .get_mut::<NodeController>(controller)
        .ok_or(StreamError("node controller load target missing"))?
        .apply_loaded_prefix(payload);
    if payload.interpolator_index == NULL_REFERENCE {
        return Ok(true);
    }
    let entry = table
        .objects
        .get(payload.interpolator_index as usize)
        .ok_or(StreamError(out_of_range))?;
    if !world
        .get::<NiClass>(entry.entity)
        .is_some_and(|class| accepts(*class))
    {
        return Ok(false);
    }
    store(world, controller, entry.entity);
    Ok(true)
}

// IDA: the 006C3650 load on a cold NiTransformController.
// Completion: 100% of the modern order.
pub fn load_transform_controller(
    world: &mut World,
    controller: Entity,
    payload: &SingleInterpControllerPayload,
    table: &StreamObjectTable,
) -> Result<bool, StreamError> {
    load_single_interp(
        world,
        controller,
        payload,
        table,
        NiClass::is_blend_transform_source,
        "transform controller interpolator out of range",
        |world, controller, interpolator| {
            world
                .get_mut::<TransformController>(controller)
                .expect("a NiTransformController entity")
                .interpolator_3c = Some(interpolator);
        },
    )
}

// IDA: the 006D2250 load on a cold NiVisController.
// Completion: 100% of the modern order.
pub fn load_vis_controller(
    world: &mut World,
    controller: Entity,
    payload: &SingleInterpControllerPayload,
    table: &StreamObjectTable,
) -> Result<bool, StreamError> {
    load_single_interp(
        world,
        controller,
        payload,
        table,
        NiClass::is_blend_bool_source,
        "vis controller interpolator out of range",
        |world, controller, interpolator| {
            world
                .get_mut::<VisController>(controller)
                .expect("a NiVisController entity")
                .interpolator_3c = Some(interpolator);
        },
    )
}

// IDA: N/A the shared head against the controller's stored time state, written back before any interpolator runs.
// Completion: 100%
fn begin_node_controller(
    world: &mut World,
    controller: Entity,
    current_time: f32,
    map: ControllerTimeMap<'_>,
) -> Result<bool, StreamError> {
    let mut time = world
        .get::<NodeController>(controller)
        .ok_or(StreamError("node controller update target missing"))?
        .time
        .clone();
    let result = begin_controller_update(world, &mut time, current_time, map);
    world
        .get_mut::<NodeController>(controller)
        .expect("checked above")
        .time = time;
    result
}

// IDA: the 006C3C40 and 006CF9D0 writes: translate +0x54 (x valid), 0047C600 rotation +0x30 (rotate.x valid).
// Completion: 100%; NaN components count as valid.
fn write_translate_and_rotation(local: &mut NiTransform, value: &QuatTransform) {
    if value.translate[0] != INVALID_TRANSFORM_COMPONENT {
        local.translation = value.translate;
    }
    if value.rotate[1] != INVALID_TRANSFORM_COMPONENT {
        local.rotation = quaternion_to_rotation_matrix(&value.rotate);
    }
}

// IDA: 006C3C40 vtable +0x54: interpolator +0x4C Update at +0x28 with an all-invalid seed; target writes translate +0x54 (x
// valid), 0047C600 rotation +0x30 (rotate.x valid), fabs(scale) +0x60 (scale valid).
// Completion: 100% of the verified branches.
// TODO: [ ] Validate against original runtime animation results.
pub fn update_transform_controller(
    world: &mut World,
    controller: Entity,
    current_time: f32,
    map: ControllerTimeMap<'_>,
) -> Result<NodeControllerUpdateStatus, StreamError> {
    use NodeControllerUpdateStatus as Status;
    if !begin_node_controller(world, controller, current_time, map)? {
        return Ok(Status::Skipped);
    }
    let Some(interpolator) = world
        .get::<TransformController>(controller)
        .ok_or(StreamError("NiTransformController update target missing"))?
        .interpolator_3c
    else {
        return Ok(Status::NoInterpolator);
    };
    let base = world
        .get::<NodeController>(controller)
        .expect("checked above")
        .clone();
    let sample_time = base.time.last_value_28;
    let node = base.time.target_30;
    let mut value = QuatTransform::default(); // 006C3C40 seeds the invalid sentinels
    let result = match world.get::<NiClass>(interpolator).copied() {
        Some(NiClass::BlendTransformInterpolator) => {
            match sample_particle_blend_transform(world, interpolator, sample_time, &mut value) {
                ParticleBlendSampleStatus::Sampled => Status::Applied,
                ParticleBlendSampleStatus::Failed => Status::InterpolatorFailed,
                _ => Status::UnsupportedInterpolatorRange,
            }
        }
        Some(class @ (NiClass::TransformInterpolator | NiClass::PathInterpolator)) => {
            let status = if class == NiClass::TransformInterpolator {
                update_transform_interpolator(world, interpolator, sample_time, &mut value)
            } else {
                update_path_interpolator(world, interpolator, sample_time, &mut value)
            };
            match status {
                TransformUpdateStatus::Valid => Status::Applied,
                TransformUpdateStatus::Invalid => Status::InterpolatorFailed,
                TransformUpdateStatus::UnsupportedOutOfRange => {
                    Status::UnsupportedInterpolatorRange
                }
            }
        }
        // 006CF780, the accum root's.
        Some(NiClass::BlendAccumTransformInterpolator) => {
            match super::blend_accum::update_blend_accum(
                world,
                interpolator,
                sample_time,
                &mut value,
            ) {
                Ok(true) => Status::Applied,
                Ok(false) => Status::InterpolatorFailed,
                Err(_) => Status::UnsupportedInterpolatorRange,
            }
        }
        _ => Status::UnsupportedInterpolatorRange,
    };
    if result != Status::Applied {
        return Ok(result);
    }
    let Some(mut payload) = node.and_then(|node| world.get_mut::<SceneNodePayload>(node)) else {
        return Ok(Status::UnsupportedMissingTarget);
    };
    let local = &mut payload.local_transform;
    write_translate_and_rotation(local, &value);
    if value.scale != INVALID_TRANSFORM_COMPONENT {
        local.scale = value.scale.abs();
    }
    Ok(Status::Applied)
}

// IDA: 006D4470 vtable +0x54: interpolator +0x60 bool Update at +0x28; a nonzero result clears target +0x18 word bit 0, zero
// sets it.
// Completion: 100% of the verified branches and word-width flag write.
// TODO: [ ] Validate against original runtime visibility results.
pub fn update_vis_controller(
    world: &mut World,
    controller: Entity,
    current_time: f32,
    map: ControllerTimeMap<'_>,
) -> Result<NodeControllerUpdateStatus, StreamError> {
    use NodeControllerUpdateStatus as Status;
    if !begin_node_controller(world, controller, current_time, map)? {
        return Ok(Status::Skipped);
    }
    let Some(interpolator) = world
        .get::<VisController>(controller)
        .ok_or(StreamError("NiVisController update target missing"))?
        .interpolator_3c
    else {
        return Ok(Status::NoInterpolator);
    };
    let base = world
        .get::<NodeController>(controller)
        .expect("checked above")
        .clone();
    let sample_time = base.time.last_value_28;
    let node = base.time.target_30;
    let mut value = None;
    let mut unsupported = false;
    match world.get::<NiClass>(interpolator).copied() {
        Some(NiClass::BlendBoolInterpolator) => {
            let mut raw = 0u8;
            match sample_particle_blend_bool(world, interpolator, sample_time, &mut raw) {
                ParticleBlendSampleStatus::Sampled => value = Some(raw),
                ParticleBlendSampleStatus::Failed => {}
                _ => unsupported = true,
            }
        }
        Some(class @ (NiClass::BoolInterpolator | NiClass::BoolTimelineInterpolator)) => {
            let mut sampled = false;
            let okay = if class == NiClass::BoolTimelineInterpolator {
                update_bool_timeline_interpolator(world, interpolator, sample_time, &mut sampled)
            } else {
                update_bool_interpolator(world, interpolator, sample_time, &mut sampled)
            };
            if okay {
                value = Some(u8::from(sampled));
            }
        }
        _ => unsupported = true,
    }
    if unsupported {
        return Ok(Status::UnsupportedInterpolatorRange);
    }
    let Some(value) = value else {
        return Ok(Status::InterpolatorFailed);
    };
    let Some(mut payload) = node.and_then(|node| world.get_mut::<SceneNodePayload>(node)) else {
        return Ok(Status::UnsupportedMissingTarget);
    };
    if value != 0 {
        payload.effective_av_flags &= 0xFFFE;
    } else {
        payload.effective_av_flags |= 0x0001;
    }
    Ok(Status::Applied)
}

/// IDA: 006D0370 NiMultiTargetTransformController::LoadBinary: 006D0550 NiInterpController load (no bytes from 0A01006D on), u16
/// count +44, then 006D0010 and one 00712A90 immediate NiAVObject reference per target.
/// Completion: 100% of the bounded >= 0A01006D field order.
/// TODO: [ ] The [0A010068, 0A01006D) manager-controlled bool.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MultiTargetTransformControllerPayload {
    pub time: ParticleTimeControllerPayload,
    pub target_count: u16,
    pub target_indices: Vec<u32>,
    pub bytes_consumed: usize,
}

// IDA: 006D0370 modern field order. 006D0550 reads only the [0A010068, 0A01006D) manager bool after 00715F40.
// Completion: 100% of the supported widths.
// TODO: [ ] Original NiStream short-read callbacks.
pub fn parse_multi_target_transform_controller_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<MultiTargetTransformControllerPayload, StreamError> {
    if stream_version < 0x0A01_006D {
        return Err(StreamError(
            "legacy NiMultiTargetTransformController payload unsupported",
        ));
    }
    let time = parse_particle_time_controller_payload(bytes, stream_version)?;
    let mut reader = ByteReader::new(&bytes[time.bytes_consumed..]);
    let target_count = reader.u16(TRUNCATED_MULTI)?;
    if usize::from(target_count) > reader.remaining() / 4 {
        return Err(StreamError(
            "truncated NiMultiTargetTransformController targets",
        ));
    }
    let mut target_indices = Vec::with_capacity(usize::from(target_count));
    for _ in 0..target_count {
        target_indices.push(reader.u32(TRUNCATED_MULTI)?);
    }
    Ok(MultiTargetTransformControllerPayload {
        time,
        target_count,
        target_indices,
        bytes_consumed: time.bytes_consumed + reader.position(),
    })
}

/// IDA: 006D0260 NiMultiTargetTransformController factory: FormHeap 0x48, 006D04E0 NiInterpController base (clears flag 0x20),
/// vtable A79984, blend array +3C and target array +40 null, u16 count +44 zero. Link is the plain 00715E70 NiTimeController
/// link; PostLoad +3C is 0068F970. The +3C elements are not stream objects, so they stay by value here, as in the original
/// array. +40 holds raw pointers.
/// Completion: 100% of the cold state, load, link and the 006CF9D0 Update.
/// TODO: [ ] The NiInterpController virtuals (+70..+A0) beyond what NiControllerSequence uses.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct MultiTargetTransformController {
    pub blend_interpolators_3c: Vec<ParticleBlendTransformInterpolator>,
    pub targets_40: Vec<Option<Entity>>,
    pub interpolator_count_44: u16,
}

impl MultiTargetTransformController {
    // IDA: 006D0010 SetInterpCount: stores +44 first; a nonzero count then constructs a cookie-prefixed array of count 0x30-byte
    // NiBlendTransformInterpolators (006CBCB0, same cold state as factory 006CBCE0), increments each element's refcount, and
    // allocates a zeroed NiAVObject* array. A zero count leaves both arrays untouched; old arrays are not freed.
    // Completion: 100% of the verified order; host vectors own the elements.
    // TODO: [ ] FormHeap allocation failure and the leaked old arrays.
    pub fn set_interpolator_count(&mut self, count: u16) {
        self.interpolator_count_44 = count;
        if count == 0 {
            return;
        }
        self.blend_interpolators_3c.clear();
        self.blend_interpolators_3c.resize(
            usize::from(count),
            ParticleBlendTransformInterpolator::default(),
        );
        self.targets_40.clear();
        self.targets_40.resize(usize::from(count), None);
    }

    // IDA: 006D03C6 stores each 00712A90 result raw (no refcount).
    // Completion: 100%; null for a null reference.
    pub fn bind_target(&mut self, index: usize, target: Option<Entity>) -> Result<(), StreamError> {
        let slot = self.targets_40.get_mut(index).ok_or(StreamError(
            "NiMultiTargetTransformController target slot out of range",
        ))?;
        *slot = target;
        Ok(())
    }
}

// IDA: 006D0370 publishes the NiTimeController fields, then 006D0010, then each 00712A90 target immediately. `Ok(false)` for a
// target that is not a NiAVObject class. From the C++ stream graph's 006D0260 case.
// Completion: 100% of the supported publication order.
pub fn load_multi_target_transform_controller(
    world: &mut World,
    controller: Entity,
    payload: &MultiTargetTransformControllerPayload,
    table: &StreamObjectTable,
) -> Result<bool, StreamError> {
    world
        .get_mut::<NodeController>(controller)
        .ok_or(StreamError("multi-target load target missing"))?
        .time
        .apply_loaded_payload(&payload.time);
    let mut multi = world
        .get::<MultiTargetTransformController>(controller)
        .ok_or(StreamError("multi-target load target missing"))?
        .clone();
    multi.set_interpolator_count(payload.target_count);
    for (index, &reference) in payload.target_indices.iter().enumerate() {
        if reference == NULL_REFERENCE {
            continue;
        }
        let entry = table
            .objects
            .get(reference as usize)
            .ok_or(StreamError("multi-target controller target out of range"))?;
        if !world
            .get::<NiClass>(entry.entity)
            .is_some_and(|class| class.is_av_object())
        {
            return Ok(false);
        }
        multi.bind_target(index, Some(entry.entity))?;
    }
    *world
        .get_mut::<MultiTargetTransformController>(controller)
        .expect("checked above") = multi;
    Ok(true)
}

// IDA: N/A host view of an NiAVObject target's +0x18 flags word: NiNode, geometry, NiParticleSystem, NiDirectionalLight and NiCamera.
// The C++ port's `AVObjectOwner` has no camera; the original's raw pointer can hold one, and it has the same word. Any other class reads
// as `None`, a host stop.
// Completion: 100% of the bound families.
#[must_use]
pub fn live_av_flags(world: &World, target: Entity) -> Option<u16> {
    if let Some(payload) = world.get::<SceneNodePayload>(target) {
        return Some(payload.effective_av_flags);
    }
    if let Some(payload) = world.get::<MeshTriShapePayload>(target) {
        return Some(payload.effective_av_flags);
    }
    if let Some(system) = world.get::<ParticleSystem>(target) {
        return Some(system.effective_av_flags_18);
    }
    if let Some(light) = world.get::<DirectionalLight>(target) {
        return Some(light.payload.av.effective_av_flags);
    }
    world
        .get::<SceneCamera>(target)
        .map(|camera| camera.av_flags_18)
}

// IDA: N/A the +0x30 / +0x54 / +0x60 local transform written by 006CF9D0, for the classes `live_av_flags` reads.
// Completion: 100% of the bound families.
fn live_local_transform(world: &mut World, target: Entity) -> Option<Mut<'_, NiTransform>> {
    if world.get::<SceneNodePayload>(target).is_some() {
        return world
            .get_mut::<SceneNodePayload>(target)
            .map(|payload| payload.map_unchanged(|payload| &mut payload.local_transform));
    }
    if world.get::<MeshTriShapePayload>(target).is_some() {
        return world
            .get_mut::<MeshTriShapePayload>(target)
            .map(|payload| payload.map_unchanged(|payload| &mut payload.local_transform));
    }
    if world.get::<ParticleSystem>(target).is_some() {
        return world
            .get_mut::<ParticleSystem>(target)
            .map(|system| system.map_unchanged(|system| &mut system.local_transform_30));
    }
    if world.get::<DirectionalLight>(target).is_some() {
        return world
            .get_mut::<DirectionalLight>(target)
            .map(|light| light.map_unchanged(|light| &mut light.payload.av.local_transform));
    }
    world
        .get_mut::<SceneCamera>(target)
        .map(|camera| camera.map_unchanged(|camera| &mut camera.local_transform_30))
}

// IDA: 006CF9D0 vtable +0x54: requires flag 0x08 only (no 006C36B0 gate, raw time). One all-invalid NiQuatTransform seed (B24260,
// B3CBA4 and A79E10, set once at 006CF9E4) is shared by every iteration. Per index with a nonnull target whose +0x18 bit 0x02 is
// set: blend +0x4C Update(time, target, value); on success translate +0x54 (translate.x valid), 0047C600 rotation +0x30 (rotate.x
// valid), fabs(scale) +0x60, or 1.0 when scale is invalid and flag 0x40 is set.
// Completion: 100% of the verified branches; NaN components count as valid.
// TODO: [ ] Validate against original runtime animation results.
pub fn update_multi_target_transform_controller(
    world: &mut World,
    controller: Entity,
    time: f32,
) -> Result<NodeControllerUpdateStatus, StreamError> {
    use NodeControllerUpdateStatus as Status;
    let flags = world
        .get::<NodeController>(controller)
        .ok_or(StreamError("multi-target update target missing"))?
        .time
        .fields
        .flags;
    if flags & 0x0008 == 0 {
        return Ok(Status::Skipped);
    }
    let count = usize::from(
        world
            .get::<MultiTargetTransformController>(controller)
            .ok_or(StreamError("multi-target update target missing"))?
            .interpolator_count_44,
    );
    let mut value = QuatTransform::default();
    for index in 0..count {
        let multi = world
            .get::<MultiTargetTransformController>(controller)
            .expect("checked above");
        if index >= multi.targets_40.len() || index >= multi.blend_interpolators_3c.len() {
            return Ok(Status::UnsupportedInterpolatorRange);
        }
        let Some(target) = multi.targets_40[index] else {
            continue;
        };
        let Some(target_flags) = live_av_flags(world, target) else {
            return Ok(Status::UnsupportedMissingTarget);
        };
        if target_flags & 0x0002 == 0 {
            continue;
        }
        // The element leaves its array across the Update so that its sources can use the world.
        let mut blend = take(
            &mut world
                .get_mut::<MultiTargetTransformController>(controller)
                .expect("checked above")
                .blend_interpolators_3c[index],
        );
        let status = blend.sample(world, time, &mut value);
        world
            .get_mut::<MultiTargetTransformController>(controller)
            .expect("checked above")
            .blend_interpolators_3c[index] = blend;
        if status == ParticleBlendSampleStatus::Failed {
            continue;
        }
        if status != ParticleBlendSampleStatus::Sampled {
            return Ok(Status::UnsupportedInterpolatorRange);
        }
        let mut local = live_local_transform(world, target).expect("flags were read above");
        write_translate_and_rotation(&mut local, &value);
        if value.scale != INVALID_TRANSFORM_COMPONENT {
            local.scale = value.scale.abs();
        } else if flags & 0x0040 != 0 {
            local.scale = 1.0;
        }
    }
    Ok(Status::Applied)
}

#[cfg(test)]
mod tests {
    use super::super::bool_animation::BoolInterpolator;
    use super::super::particle_blend_interpolator::BlendInterpolator;
    use super::super::scene_node::spawn_scene_node;
    use super::super::transform_animation::TransformInterpolator;
    use super::*;

    // 006D0550 at 10.1.0.106: the time fields with 00715F40's flag 0x20 cleared, then a bool for flag 0x20, then the
    // interpolator reference.
    #[test]
    fn a_single_interp_controller_reads_its_10_1_0_106_bool() {
        let mut bytes = Vec::new();
        time_controller(&mut bytes, 7, 0x2C, 9);
        bytes.push(1);
        bytes.extend(5_u32.to_le_bytes());
        let payload = parse_single_interp_controller_payload(&bytes, 0x0A01_006A).unwrap();
        assert_eq!(payload.time.flags, 0x2C);
        assert_eq!(payload.interpolator_index, 5);
        assert_eq!(payload.bytes_consumed, bytes.len());
        bytes[26] = 0;
        let payload = parse_single_interp_controller_payload(&bytes, 0x0A01_006A).unwrap();
        assert_eq!(payload.time.flags, 0x0C);
    }

    const MODERN: u32 = 0x1400_0004;
    const NEGATIVE_MAX: f32 = -f32::MAX;

    fn identity(_: &mut ControllerTimeFields, time: f32) -> f32 {
        time
    }

    /// IDA: 00715F40 modern NiTimeController fields: next, flags, frequency, phase, low and high key time, target.
    fn time_controller(output: &mut Vec<u8>, next: u32, flags: u16, target: u32) {
        output.extend(next.to_le_bytes());
        output.extend(flags.to_le_bytes());
        for value in [1.0f32, 0.0, 0.0, 2.0] {
            output.extend(value.to_le_bytes());
        }
        output.extend(target.to_le_bytes());
    }

    fn time_with_flags(flags: u16) -> ParticleTimeControllerCold {
        let mut time = ParticleTimeControllerCold::default();
        time.fields.flags = flags;
        time
    }

    fn transform_source(world: &mut World, value: &QuatTransform) -> Entity {
        let mut interpolator = TransformInterpolator::default();
        interpolator.load_transform_and_data(value, None);
        world
            .spawn((NiClass::TransformInterpolator, interpolator))
            .id()
    }

    fn node_payload(world: &World, node: Entity) -> &SceneNodePayload {
        world.get::<SceneNodePayload>(node).unwrap()
    }

    /// IDA: 006CE320 single-interp read and 006D0370 multi-target read.
    #[test]
    fn payloads() {
        let mut single = Vec::new();
        time_controller(&mut single, 7, 0x000C, 3);
        single.extend(9u32.to_le_bytes());
        single.push(0xA5);
        let parsed = parse_single_interp_controller_payload(&single, MODERN).unwrap();
        assert_eq!(parsed.bytes_consumed, 30);
        assert_eq!(parsed.time.next_index, 7);
        assert_eq!(parsed.time.flags, 0x000C);
        assert_eq!(parsed.time.high_key_time, 2.0);
        assert_eq!(parsed.time.target_index, 3);
        assert_eq!(parsed.interpolator_index, 9);

        let mut multi = Vec::new();
        time_controller(&mut multi, NULL_REFERENCE, 0x0008, 0);
        multi.extend(2u16.to_le_bytes());
        multi.extend(4u32.to_le_bytes());
        multi.extend(NULL_REFERENCE.to_le_bytes());
        let targets = parse_multi_target_transform_controller_payload(&multi, MODERN).unwrap();
        assert_eq!(targets.bytes_consumed, multi.len());
        assert_eq!(targets.target_count, 2);
        assert_eq!(targets.target_indices, [4, NULL_REFERENCE]);

        assert!(
            parse_single_interp_controller_payload(&single, 0x0A01_006C).is_err(),
            "legacy"
        );
        multi.truncate(multi.len() - 2);
        assert!(
            parse_multi_target_transform_controller_payload(&multi, MODERN).is_err(),
            "truncated list"
        );
    }

    /// IDA: 0047C600 quaternion to NiAVObject +0x30 rotation (row-major): identity bits; 90 degrees about Z within f32 product
    /// rounding.
    #[test]
    fn quaternion_matrix() {
        assert_eq!(
            quaternion_to_rotation_matrix(&[1.0, 0.0, 0.0, 0.0]),
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
        );
        let rotated = quaternion_to_rotation_matrix(&[0.707_106_77, 0.0, 0.0, 0.707_106_77]);
        let expected = [0.0f32, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        for (actual, expected) in rotated.iter().zip(expected) {
            assert!((actual - expected).abs() <= 1e-6, "{actual} {expected}");
        }
    }

    /// IDA: shared head of 006C3C40 / 006D4470: flag 0x20 stores -FLT_MAX to +0x28 and runs; inactive or unchanged time skips;
    /// otherwise 007155A0 remaps.
    #[test]
    fn update_gate() {
        let mut managed = time_with_flags(0x0020);
        managed.last_value_28 = 3.0;
        assert!(begin_single_interp_update(&mut managed, 1.0, None).unwrap());
        assert_eq!(managed.last_value_28, NEGATIVE_MAX);

        let mut inactive = time_with_flags(0);
        assert!(!begin_single_interp_update(&mut inactive, 1.0, Some(&mut identity)).unwrap());

        let mut active = time_with_flags(0x0008);
        assert!(begin_single_interp_update(&mut active, 1.5, Some(&mut identity)).unwrap());
        assert_eq!(active.last_value_28, 1.5);
        assert!(
            !begin_single_interp_update(&mut active, 1.5, Some(&mut identity)).unwrap(),
            "unchanged mapped time"
        );
    }

    /// IDA: the gate's two host time maps: the C++ port's empty function fails only when reached, and 007155A0 runs over the world's
    /// one cache.
    #[test]
    fn time_maps() {
        let mut world = World::new();
        let mut time = time_with_flags(0x0008);
        assert_eq!(
            begin_controller_update(&mut world, &mut time, 1.0, ControllerTimeMap::Unresolved),
            Err(StreamError("node controller time map unresolved"))
        );
        // A real key range: the cold one clamps to -FLT_MAX, which equals the cold +0x28, so the gate would skip.
        time.fields.low_key_time = 0.0;
        time.fields.high_key_time = 10.0;
        assert_eq!(
            begin_controller_update(&mut world, &mut time, 1.0, ControllerTimeMap::SharedCache),
            Ok(true)
        );
        assert_eq!(time.fields.last_time, 1.0);
        assert_eq!(time.last_value_28, 1.0);
        assert_eq!(
            world.resource::<ParticleControllerTimeCache>().output,
            time.last_value_28,
            "the cache is written back"
        );
    }

    /// IDA: 006C3C40 NiTransformController Update and 006D4470 NiVisController Update writing the target NiNode translate,
    /// rotation, scale and +0x18 bit 0.
    #[test]
    fn node_controllers() {
        let mut world = World::new();
        let fixed = QuatTransform {
            translate: [1.0, 2.0, 3.0],
            rotate: [1.0, 0.0, 0.0, 0.0],
            scale: -2.0,
        };
        let interpolator = transform_source(&mut world, &fixed);
        let node = spawn_scene_node(&mut world, SceneNodePayload::default());
        let transform = spawn_node_controller(
            &mut world,
            NiClass::TransformController,
            TransformController::default(),
        );
        {
            let mut base = world.get_mut::<NodeController>(transform).unwrap();
            base.time.fields.flags = 0x0008;
            base.time.target_30 = Some(node);
        }
        let mut evaluate = identity;
        assert_eq!(
            update_transform_controller(
                &mut world,
                transform,
                1.0,
                ControllerTimeMap::Custom(&mut evaluate)
            ),
            Ok(NodeControllerUpdateStatus::NoInterpolator),
            "an interpolator-less controller writes nothing"
        );
        world
            .get_mut::<NodeController>(transform)
            .unwrap()
            .time
            .last_value_28 = NEGATIVE_MAX;
        world
            .get_mut::<TransformController>(transform)
            .unwrap()
            .interpolator_3c = Some(interpolator);
        assert_eq!(
            update_transform_controller(
                &mut world,
                transform,
                2.0,
                ControllerTimeMap::Custom(&mut evaluate)
            ),
            Ok(NodeControllerUpdateStatus::Applied)
        );
        let local = node_payload(&world, node).local_transform;
        assert_eq!(local.translation, fixed.translate);
        assert_eq!(
            local.rotation,
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(local.scale, 2.0, "fabs scale");

        let mut bool_state = BoolInterpolator::default();
        bool_state.load_scalar_and_data(0, None);
        let visible = world.spawn((NiClass::BoolInterpolator, bool_state)).id();
        let vis = spawn_node_controller(
            &mut world,
            NiClass::VisController,
            VisController {
                interpolator_3c: Some(visible),
            },
        );
        {
            let mut base = world.get_mut::<NodeController>(vis).unwrap();
            base.time.fields.flags = 0x0008;
            base.time.target_30 = Some(node);
        }
        world
            .get_mut::<SceneNodePayload>(node)
            .unwrap()
            .effective_av_flags = 0x000E;
        assert_eq!(
            update_vis_controller(
                &mut world,
                vis,
                1.0,
                ControllerTimeMap::Custom(&mut evaluate)
            ),
            Ok(NodeControllerUpdateStatus::Applied)
        );
        assert_eq!(
            node_payload(&world, node).effective_av_flags,
            0x000F,
            "false sets the cull bit"
        );
        world
            .get_mut::<BoolInterpolator>(visible)
            .unwrap()
            .load_scalar_and_data(1, None);
        assert_eq!(
            update_vis_controller(
                &mut world,
                vis,
                2.0,
                ControllerTimeMap::Custom(&mut evaluate)
            ),
            Ok(NodeControllerUpdateStatus::Applied)
        );
        assert_eq!(
            node_payload(&world, node).effective_av_flags,
            0x000E,
            "true clears the cull bit"
        );
    }

    /// IDA: 006CF9D0's target view: the +0x18 word and local transform of each class the C++ port's raw slot can hold.
    #[test]
    fn multi_target_views() {
        let mut world = World::new();
        let system = super::super::particle_system::spawn_particle_system(&mut world);
        let light = super::super::scene_derived::spawn_directional_light(&mut world);
        let camera = crate::bs::scene_graphs::construct_scene_camera(&mut world);
        world
            .get_mut::<ParticleSystem>(system)
            .unwrap()
            .effective_av_flags_18 = 0x0102;
        assert_eq!(live_av_flags(&world, system), Some(0x0102));
        assert_eq!(live_av_flags(&world, light), Some(0x000E));
        assert_eq!(live_av_flags(&world, camera), Some(0x000E));
        for target in [system, light, camera] {
            live_local_transform(&mut world, target).unwrap().scale = 3.0;
        }
        assert_eq!(
            world
                .get::<ParticleSystem>(system)
                .unwrap()
                .local_transform_30
                .scale,
            3.0
        );
        assert_eq!(
            world
                .get::<DirectionalLight>(light)
                .unwrap()
                .payload
                .av
                .local_transform
                .scale,
            3.0
        );
        assert_eq!(
            world
                .get::<SceneCamera>(camera)
                .unwrap()
                .local_transform_30
                .scale,
            3.0
        );
        let other = world.spawn_empty().id();
        assert_eq!(live_av_flags(&world, other), None, "a host stop");
    }

    /// IDA: 006D0010 SetInterpCount and 006CF9D0 Update: flag 0x08 only, target bit 0x02, shared seed, fabs scale and the 0x40
    /// scale reset, for a single-source blend.
    #[test]
    fn multi_target() {
        let mut world = World::new();
        let mut state = MultiTargetTransformController::default();
        assert_eq!(state.interpolator_count_44, 0);
        assert!(state.blend_interpolators_3c.is_empty() && state.targets_40.is_empty());
        state.set_interpolator_count(0);
        assert!(
            state.blend_interpolators_3c.is_empty(),
            "zero allocates nothing"
        );
        state.set_interpolator_count(2);
        assert_eq!(state.interpolator_count_44, 2);
        assert_eq!(state.blend_interpolators_3c.len(), 2);
        assert_eq!(state.targets_40.len(), 2);
        assert!(state.targets_40[0].is_none());

        let fixed = QuatTransform {
            translate: [4.0, 5.0, 6.0],
            rotate: [1.0, 0.0, 0.0, 0.0],
            scale: INVALID_TRANSFORM_COMPONENT,
        };
        let source = transform_source(&mut world, &fixed);
        for blend in &mut state.blend_interpolators_3c {
            blend.base.active_count_0e = 1;
            blend.base.single_interpolator_index = 0;
            blend.bind_single(source).unwrap();
        }
        let moved = spawn_scene_node(
            &mut world,
            SceneNodePayload {
                effective_av_flags: 0x0002,
                ..SceneNodePayload::default()
            },
        );
        let skipped = spawn_scene_node(
            &mut world,
            SceneNodePayload {
                effective_av_flags: 0,
                ..SceneNodePayload::default()
            },
        );
        state.bind_target(0, Some(moved)).unwrap();
        state.bind_target(1, Some(skipped)).unwrap();
        assert!(state.bind_target(2, None).is_err(), "out-of-range slot");
        let controller =
            spawn_node_controller(&mut world, NiClass::MultiTargetTransformController, state);

        // 00715990 cold flags are 0x0C (active); clear bit 0x08 for the skip case.
        world
            .get_mut::<NodeController>(controller)
            .unwrap()
            .time
            .fields
            .flags = 0x0004;
        assert_eq!(
            update_multi_target_transform_controller(&mut world, controller, 1.0),
            Ok(NodeControllerUpdateStatus::Skipped)
        );
        world
            .get_mut::<NodeController>(controller)
            .unwrap()
            .time
            .fields
            .flags = 0x0048;
        world
            .get_mut::<SceneNodePayload>(moved)
            .unwrap()
            .local_transform
            .scale = 7.0;
        assert_eq!(
            update_multi_target_transform_controller(&mut world, controller, 1.0),
            Ok(NodeControllerUpdateStatus::Applied)
        );
        let local = node_payload(&world, moved).local_transform;
        assert_eq!(local.translation, fixed.translate);
        assert_eq!(local.scale, 1.0, "the 0x40 scale reset");
        assert_eq!(
            node_payload(&world, skipped).local_transform.translation,
            [0.0; 3],
            "a target without bit 0x02 is not written"
        );
    }
}
