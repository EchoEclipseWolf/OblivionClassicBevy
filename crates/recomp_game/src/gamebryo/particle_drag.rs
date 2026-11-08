//! NiPSysDragModifier.
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.

#![allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "00758980 and 00758A7F test with these negated comparisons, so a NaN takes the same branch"
)]

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::particle_data::ParticleData;
use super::particle_dynamics::{
    ParticleModifierBase, ParticleModifierPending, parse_particle_modifier_payload, reader_after,
};
use super::raw_reference::{AvObjectRawRef, raw_object_reference};
use super::stream_objects::{NULL_REFERENCE, StreamObjectTable};
use super::transform::{
    NORMALIZE_ZERO_THRESHOLD, NiTransform, compose_scene_transform, invert_scene_transform,
    rotate_vector,
};

const TRUNCATED: &str = "truncated particle drag payload";

/// The 00758980 tick divisor, an x87 double constant (A87738).
const TICK_SECONDS: f64 = 0.033_333_301_544_189_45;

#[derive(Clone, Debug, PartialEq)]
pub struct ParticleDragPending {
    pub base: ParticleModifierPending,
    pub drag_object_index: u32,
    pub direction: [f32; 3],
    pub drag_coefficient: f32,
    pub minimum_distance: f32,
    pub maximum_distance: f32,
    pub bytes_consumed: usize,
}

impl Default for ParticleDragPending {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            base: ParticleModifierPending::default(),
            drag_object_index: NULL_REFERENCE,
            direction: [1.0, 0.0, 0.0],
            drag_coefficient: 0.05,
            minimum_distance: f32::MAX,
            maximum_distance: f32::MAX,
            bytes_consumed: 0,
        }
    }
}

/// IDA: N/A the values after 00718A80 / 0053D7A0 / 007101F0 / 0043F350 resolve the linked object's world center and
/// final drag axis. Completion: 100% of the explicit input boundary for the particle loop.
/// TODO: [ ] Build this frame from verified NiAVObject and system world transforms.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleDragResolvedFrame {
    pub center: [f32; 3],
    pub axis: [f32; 3],
}

/// IDA: 00758960 allocates 0x34 bytes and calls the 00758910 constructor: +0x18 null, +0x1C (1,0,0), +0x28 0.05,
/// +0x2C / +0x30 FLT_MAX.
/// TODO: [ ] Bind target and drag NiAVObject ownership and transforms.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ParticleDragModifier {
    pub base: ParticleModifierBase,
    /// IDA: 0075E870 NiPSysDragModifier_LinkObject: +0x18 the raw drag NiAVObject (13e).
    pub drag_object_18: AvObjectRawRef,
    /// IDA: 0075E870 +0x18's presence, from the index, kept apart from the raw reference as in C++.
    pub drag_object_linked: bool,
    pub direction_1c: [f32; 3],
    pub drag_coefficient_28: f32,
    pub minimum_distance_2c: f32,
    pub maximum_distance_30: f32,
}

impl Default for ParticleDragModifier {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            base: ParticleModifierBase::default(),
            drag_object_18: AvObjectRawRef::default(),
            drag_object_linked: false,
            direction_1c: [1.0, 0.0, 0.0],
            drag_coefficient_28: 0.05,
            minimum_distance_2c: f32::MAX,
            maximum_distance_30: f32::MAX,
        }
    }
}

/// IDA: 00718A80's guard as the C++ port applies it: a nonfinite or zero scale is rejected.
fn inverse_transform(input: &NiTransform) -> Result<NiTransform, StreamError> {
    if !input.scale.is_finite() || input.scale == 0.0 {
        return Err(StreamError("particle drag world scale invalid"));
    }
    Ok(invert_scene_transform(input))
}

/// IDA: 0043F350 stores y^2+x^2+z^2 and sqrt as floats, zeroes the axis at <= 0.000001, otherwise stores the float
/// reciprocal times each component. Completion: 100% of the finite branch and stores.
fn normalize_drag_axis(axis: &mut [f32; 3]) -> Result<(), StreamError> {
    let length_squared = axis[1] * axis[1] + axis[0] * axis[0] + axis[2] * axis[2];
    let length = length_squared.sqrt();
    if !length.is_finite() {
        return Err(StreamError("particle drag axis length invalid"));
    }
    if length <= NORMALIZE_ZERO_THRESHOLD {
        *axis = [0.0; 3];
        return Ok(());
    }
    let reciprocal = 1.0 / length;
    axis[0] *= reciprocal;
    axis[1] *= reciprocal;
    axis[2] *= reciprocal;
    Ok(())
}

/// IDA: N/A host finite preflight for explicit +0x64 NiTransform values. Completion: 100% of the present fields.
/// TODO: [ ] Validate the concrete NiAVObject at the graph boundary.
fn check_transform(transform: &NiTransform) -> Result<(), StreamError> {
    if transform.rotation.iter().any(|value| !value.is_finite()) {
        return Err(StreamError("particle drag rotation invalid"));
    }
    if transform.translation.iter().any(|value| !value.is_finite()) {
        return Err(StreamError("particle drag translation invalid"));
    }
    if !transform.scale.is_finite() {
        return Err(StreamError("particle drag scale invalid"));
    }
    Ok(())
}

impl ParticleDragModifier {
    /// IDA: 00758980 receives the current time and NiPSysData, then derives center and axis through the linked
    /// NiAVObject and system world transforms before this loop. Completion: 100% of the checked finite loop for an
    /// explicitly resolved frame, staged.
    /// TODO: [ ] Compare the 0.03333330154418945 divisor's results against an original run.
    pub fn update_velocities_in_frame(
        &self,
        current_time: f32,
        data: &mut ParticleData,
        frame: &ParticleDragResolvedFrame,
    ) -> Result<(), StreamError> {
        if !(self.drag_coefficient_28 > 0.0)
            || data.active_count_48 == 0
            || !self.drag_object_linked
        {
            return Ok(());
        }
        let count = usize::from(data.active_count_48);
        if data.positions.len() < count || data.info_5c.len() < count {
            return Err(StreamError("particle drag arrays are too short"));
        }
        if !current_time.is_finite()
            || !self.drag_coefficient_28.is_finite()
            || !self.minimum_distance_2c.is_finite()
            || !self.maximum_distance_30.is_finite()
        {
            return Err(StreamError("particle drag scalar outside finite domain"));
        }
        if frame.center.iter().any(|value| !value.is_finite()) {
            return Err(StreamError("particle drag center outside finite domain"));
        }
        if frame.axis.iter().any(|value| !value.is_finite()) {
            return Err(StreamError("particle drag axis outside finite domain"));
        }
        // 00758A7F stores y^2 + x^2 + z^2; the helper has already normalized the axis in the original path, but the
        // explicit frame is not altered.
        let axis_squared = frame.axis[1] * frame.axis[1]
            + frame.axis[0] * frame.axis[0]
            + frame.axis[2] * frame.axis[2];
        let distance_range = self.maximum_distance_30 - self.minimum_distance_2c;
        if !axis_squared.is_finite() || !distance_range.is_finite() {
            return Err(StreamError("particle drag frame outside finite domain"));
        }

        let mut prepared = data.clone();
        for index in 0..count {
            let position = prepared.positions[index];
            let dx = position[0] - frame.center[0];
            let dy = position[1] - frame.center[1];
            let dz = position[2] - frame.center[2];
            let distance_squared = dy * dy + dx * dx + dz * dz;
            let distance = distance_squared.sqrt();
            let last_update_time = prepared.info_5c[index].last_update_time_14;
            // 00758B22..00758B32: the fsub rounds, then the fdiv by the qword, rounded once more at the store's width.
            let elapsed = current_time - last_update_time;
            let ticks = (f64::from(elapsed) / TICK_SECONDS) as f32;
            if !distance.is_finite() || !ticks.is_finite() {
                return Err(StreamError(
                    "particle drag distance/time outside finite domain",
                ));
            }

            let mut effective_coefficient = self.drag_coefficient_28;
            if !(self.minimum_distance_2c >= distance) {
                if !(self.maximum_distance_30 > distance) {
                    continue;
                }
                let past_minimum = distance - self.minimum_distance_2c;
                effective_coefficient =
                    (1.0 - past_minimum / distance_range) * self.drag_coefficient_28;
            }
            let velocity = &mut prepared.info_5c[index].velocity_00;
            // 00758B75..00758B9D: (x + y) + z.
            let dot = velocity[0] * frame.axis[0]
                + velocity[1] * frame.axis[1]
                + velocity[2] * frame.axis[2];
            if !effective_coefficient.is_finite() || !dot.is_finite() || axis_squared == 0.0 {
                return Err(StreamError(
                    "particle drag projection outside finite domain",
                ));
            }
            // 00758BAD compares the rounded product with the qword 1.0; 00758C0C..00758C18 is (t * -c) * (dot / a).
            let multiplier = if ticks * effective_coefficient <= 1.0 {
                ticks * -effective_coefficient * (dot / axis_squared)
            } else {
                -dot / axis_squared
            };
            let vx = frame.axis[0] * multiplier;
            let vy = frame.axis[1] * multiplier;
            let vz = frame.axis[2] * multiplier;
            let next_x = vx + velocity[0];
            let next_y = velocity[1] + vy;
            let next_z = velocity[2] + vz;
            if !multiplier.is_finite()
                || !next_x.is_finite()
                || !next_y.is_finite()
                || !next_z.is_finite()
            {
                return Err(StreamError("particle drag velocity outside finite domain"));
            }
            *velocity = [next_x, next_y, next_z];
        }
        *data = prepared;
        Ok(())
    }

    /// IDA: 00758980 copies both +0x64 world transforms, inverts the system transform via 00718A80, composes it with
    /// the drag object via 0053D7A0, rotates the +0x1C direction through 007101F0 and normalizes via 0043F350.
    /// Completion: 100% of the checked finite derivation of center and axis.
    /// TODO: [ ] Bind the concrete linked object and system transforms, and x87 validation.
    pub fn resolve_frame(
        &self,
        drag_world: &NiTransform,
        system_world: &NiTransform,
    ) -> Result<ParticleDragResolvedFrame, StreamError> {
        check_transform(drag_world)?;
        check_transform(system_world)?;
        if self.direction_1c.iter().any(|value| !value.is_finite()) {
            return Err(StreamError("particle drag direction invalid"));
        }
        let inverse_system = inverse_transform(system_world)?;
        let relative = compose_scene_transform(&inverse_system, drag_world);
        let mut frame = ParticleDragResolvedFrame {
            center: relative.translation,
            axis: rotate_vector(&relative.rotation, &self.direction_1c),
        };
        normalize_drag_axis(&mut frame.axis)?;
        if frame.center.iter().any(|value| !value.is_finite()) {
            return Err(StreamError("particle drag relative center invalid"));
        }
        Ok(frame)
    }

    /// IDA: 00758980's transform chain followed by its active velocity loop. Completion: 100% of the checked finite
    /// composition of the base-data path.
    /// TODO: [ ] Bind exact NiAVObject world owners and runtime validate.
    pub fn update_velocities(
        &self,
        current_time: f32,
        data: &mut ParticleData,
        drag_world: &NiTransform,
        system_world: &NiTransform,
    ) -> Result<(), StreamError> {
        if !(self.drag_coefficient_28 > 0.0)
            || data.active_count_48 == 0
            || !self.drag_object_linked
        {
            return Ok(());
        }
        let frame = self.resolve_frame(drag_world, system_world)?;
        self.update_velocities_in_frame(current_time, data, &frame)
    }
}

/// IDA: 00758D70 reads the deferred drag object, vector3, coefficient and two distance thresholds after the common
/// modifier payload. Completion: 100% of the bounded modern byte sequence, raw float bits kept.
/// TODO: [ ] Bind the original's callback errors and premodern base prefix.
pub fn parse_particle_drag_payload(
    bytes: &[u8],
    version: u32,
) -> Result<ParticleDragPending, StreamError> {
    let base = parse_particle_modifier_payload(bytes, version)?;
    let mut reader = reader_after(bytes, base.bytes_consumed);
    let drag_object_index = reader.u32(TRUNCATED)?;
    let direction = [
        reader.f32(TRUNCATED)?,
        reader.f32(TRUNCATED)?,
        reader.f32(TRUNCATED)?,
    ];
    let drag_coefficient = reader.f32(TRUNCATED)?;
    let minimum_distance = reader.f32(TRUNCATED)?;
    let maximum_distance = reader.f32(TRUNCATED)?;
    Ok(ParticleDragPending {
        base,
        drag_object_index,
        direction,
        drag_coefficient,
        minimum_distance,
        maximum_distance,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 00752CB0 and 0075E870 each resolve one deferred object reference. Completion: 100% of the index or null.
/// TODO: [ ] Bind the object table and type checks.
fn validate_reference(index: u32, object_count: usize) -> Result<(), StreamError> {
    if index != NULL_REFERENCE && index as usize >= object_count {
        return Err(StreamError("particle drag link out of range"));
    }
    Ok(())
}

/// IDA: 0075E870 resolves the common target then the +0x18 object reference. Completion: 100% of the checked indices
/// and scalar publication; the raw reference is `link_particle_drag`'s.
/// TODO: [ ] Type-check and retain a concrete NiAVObject owner; presence alone is not original link parity.
pub fn resolve_particle_drag_link(
    target: &mut ParticleDragModifier,
    pending: &ParticleDragPending,
    object_count: usize,
) -> Result<(), StreamError> {
    validate_reference(pending.base.target_index, object_count)?;
    validate_reference(pending.drag_object_index, object_count)?;
    target.base = pending.base.published();
    target.drag_object_linked = pending.drag_object_index != NULL_REFERENCE;
    target.direction_1c = pending.direction;
    target.drag_coefficient_28 = pending.drag_coefficient;
    target.minimum_distance_2c = pending.minimum_distance;
    target.maximum_distance_30 = pending.maximum_distance;
    Ok(())
}

/// IDA: 0075E870 on a stream object: the scalars, then +0x18 the next deferred reference (raw). Completion: 100%.
pub fn link_particle_drag(
    world: &mut World,
    entity: Entity,
    pending: &ParticleDragPending,
    table: &StreamObjectTable,
) -> Result<(), StreamError> {
    let mut target = world
        .get_mut::<ParticleDragModifier>(entity)
        .ok_or(StreamError("particle modifier component missing"))?;
    resolve_particle_drag_link(&mut target, pending, table.objects.len())?;
    target.drag_object_18 = raw_object_reference(table, pending.drag_object_index)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::particle_data::ParticleInfo;
    use super::*;

    /// 00758B75..00758B9D adds x and y first, then z, each step rounded; `multiplier` is then -dot / axis^2. Expected bits from
    /// exact per-step rounding. The old form, z and y first with one rounding, gives 0x424CD930, 0xC2B29204, 0xC297C8FD.
    #[test]
    fn projection_keeps_the_instruction_grouping() {
        let mut modifier = ParticleDragModifier::default();
        let pending = ParticleDragPending {
            drag_object_index: 0,
            drag_coefficient: 1.0,
            minimum_distance: 1000.0,
            maximum_distance: 2000.0,
            ..ParticleDragPending::default()
        };
        resolve_particle_drag_link(&mut modifier, &pending, 1).unwrap();
        let frame = ParticleDragResolvedFrame {
            center: [0.0; 3],
            axis: [0xBF77_8D50, 0xBEEC_8B44, 0xBDDF_3B64].map(f32::from_bits),
        };
        let mut data = ParticleData {
            active_count_48: 1,
            positions: vec![[0.0; 3]],
            info_5c: vec![ParticleInfo {
                velocity_00: [0xC3DB_C51F, 0xC3A1_E000, 0xC303_35C3].map(f32::from_bits),
                lifespan_10: 1.0,
                ..ParticleInfo::default()
            }],
            ..ParticleData::default()
        };
        modifier
            .update_velocities_in_frame(1.0, &mut data, &frame)
            .unwrap();
        assert_eq!(
            data.info_5c[0].velocity_00.map(f32::to_bits),
            [0x424C_D920, 0xC2B2_9208, 0xC297_C8FE]
        );
    }

    fn ticks() -> f32 {
        TICK_SECONDS as f32
    }

    #[test]
    fn payload_and_link() {
        let mut cold = ParticleDragModifier::default();
        assert_eq!(cold.base.name_08, None);
        assert_eq!(cold.base.order_0c, 0);
        assert!(cold.base.active_14);
        assert!(!cold.drag_object_linked);
        assert_eq!(cold.direction_1c, [1.0, 0.0, 0.0]);
        assert_eq!(cold.drag_coefficient_28, 0.05);
        assert_eq!(cold.minimum_distance_2c, f32::MAX);
        assert_eq!(cold.maximum_distance_30, f32::MAX);
        let mut bytes = Vec::new();
        bytes.extend(4u32.to_le_bytes());
        bytes.extend(b"Drag");
        bytes.extend(8u32.to_le_bytes());
        bytes.extend(u32::MAX.to_le_bytes()); // null target
        bytes.push(0); // inactive
        bytes.extend(1u32.to_le_bytes()); // linked drag object
        for word in [
            0x7FC1_2345u32,
            0x8000_0000,
            0x3F80_0000,
            0x3D4C_CCCD,
            0x3F80_0000,
            0x4040_0000,
        ] {
            bytes.extend(word.to_le_bytes());
        }
        let size = bytes.len();
        bytes.push(0xAB);
        let pending = parse_particle_drag_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(pending.bytes_consumed, size);
        assert_eq!(pending.base.name.as_deref(), Some(&b"Drag"[..]));
        assert_eq!(pending.base.order, 8);
        assert!(!pending.base.active);
        assert_eq!(pending.drag_object_index, 1);
        assert_eq!(pending.direction[0].to_bits(), 0x7FC1_2345);
        assert_eq!(pending.direction[1].to_bits(), 0x8000_0000);
        assert_eq!(pending.drag_coefficient, 0.05);
        assert_eq!(pending.minimum_distance, 1.0);
        assert_eq!(pending.maximum_distance, 3.0);
        for length in 0..size {
            assert!(parse_particle_drag_payload(&bytes[..length], 0x1400_0004).is_err());
        }
        resolve_particle_drag_link(&mut cold, &pending, 2).unwrap();
        assert_eq!(cold.base.name_08.as_deref(), Some(&b"Drag"[..]));
        assert_eq!(cold.base.order_0c, 8);
        assert!(!cold.base.active_14);
        assert!(cold.drag_object_linked);
        assert_eq!(cold.direction_1c[0].to_bits(), 0x7FC1_2345);
        let mut bad = pending.clone();
        bad.drag_object_index = 2;
        assert!(resolve_particle_drag_link(&mut cold, &bad, 2).is_err());
        assert_eq!(cold.base.name_08.as_deref(), Some(&b"Drag"[..]));
        assert!(cold.drag_object_linked);
    }

    fn velocity(x: f32, y: f32) -> ParticleInfo {
        ParticleInfo {
            velocity_00: [x, y, 0.0],
            lifespan_10: 1.0,
            ..ParticleInfo::default()
        }
    }

    #[test]
    fn distance_comparisons_and_damping() {
        let mut modifier = ParticleDragModifier::default();
        let mut pending = ParticleDragPending {
            drag_object_index: 0,
            drag_coefficient: 0.05,
            minimum_distance: 0.0,
            maximum_distance: 10.0,
            ..ParticleDragPending::default()
        };
        resolve_particle_drag_link(&mut modifier, &pending, 1).unwrap();
        let frame = ParticleDragResolvedFrame {
            center: [0.0; 3],
            axis: [1.0, 0.0, 0.0],
        };
        let mut data = ParticleData {
            active_count_48: 3,
            positions: vec![[0.0, 0.0, 0.0], [5.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
            info_5c: vec![velocity(10.0, 2.0); 3],
            ..ParticleData::default()
        };
        modifier
            .update_velocities_in_frame(ticks(), &mut data, &frame)
            .unwrap();
        assert!((data.info_5c[0].velocity_00[0] - 9.5).abs() < 0.00001);
        assert!((data.info_5c[1].velocity_00[0] - 9.75).abs() < 0.00001);
        assert_eq!(data.info_5c[2].velocity_00, [10.0, 2.0, 0.0]);
        assert_eq!(data.info_5c[0].last_update_time_14, 0.0);
        modifier
            .update_velocities_in_frame(1.0, &mut data, &frame)
            .unwrap();
        assert_eq!(data.info_5c[0].velocity_00, [0.0, 2.0, 0.0]);
        assert_eq!(data.info_5c[2].velocity_00[0], 10.0);
        assert_eq!(data.info_5c[0].last_update_time_14, 0.0);

        let mut malformed = data.clone();
        malformed.positions[2] = [0.0; 3];
        malformed.info_5c[2].velocity_00[0] = f32::NAN;
        let before_first = malformed.info_5c[0].velocity_00;
        assert!(
            modifier
                .update_velocities_in_frame(ticks(), &mut malformed, &frame)
                .is_err()
        );
        assert_eq!(malformed.info_5c[0].velocity_00, before_first);
        assert!(malformed.info_5c[2].velocity_00[0].is_nan());

        let mut no_link = ParticleDragModifier::default();
        no_link
            .update_velocities_in_frame(f32::NAN, &mut malformed, &frame)
            .unwrap();
        assert_eq!(malformed.info_5c[0].velocity_00, before_first);
        pending.drag_coefficient = 0.0;
        resolve_particle_drag_link(&mut no_link, &pending, 1).unwrap();
        no_link
            .update_velocities_in_frame(f32::NAN, &mut malformed, &frame)
            .unwrap();
        assert_eq!(malformed.info_5c[0].velocity_00, before_first);
    }

    #[test]
    fn transform_frame() {
        let mut modifier = ParticleDragModifier::default();
        let pending = ParticleDragPending {
            drag_object_index: 0,
            drag_coefficient: 0.05,
            minimum_distance: 0.0,
            maximum_distance: 10.0,
            ..ParticleDragPending::default()
        };
        resolve_particle_drag_link(&mut modifier, &pending, 1).unwrap();
        let identity = NiTransform::default();
        let neutral = modifier.resolve_frame(&identity, &identity).unwrap();
        assert_eq!(neutral.center, [0.0; 3]);
        assert_eq!(neutral.axis, [1.0, 0.0, 0.0]);
        let drag = NiTransform {
            rotation: [0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            translation: [10.0, 4.0, 0.0],
            scale: 1.0,
        };
        let mut system = NiTransform {
            translation: [4.0, 0.0, 0.0],
            scale: 2.0,
            ..NiTransform::default()
        };
        let relative = modifier.resolve_frame(&drag, &system).unwrap();
        assert_eq!(relative.center, [3.0, 2.0, 0.0]);
        assert_eq!(relative.axis, [0.0, 1.0, 0.0]);
        let mut data = ParticleData {
            active_count_48: 1,
            positions: vec![[3.0, 2.0, 0.0]],
            info_5c: vec![velocity(0.0, 10.0)],
            ..ParticleData::default()
        };
        modifier
            .update_velocities(ticks(), &mut data, &drag, &system)
            .unwrap();
        assert!((data.info_5c[0].velocity_00[1] - 9.5).abs() < 0.00001);
        assert_eq!(data.info_5c[0].velocity_00[0], 0.0);
        system.scale = 0.0;
        assert!(modifier.resolve_frame(&drag, &system).is_err());
    }

    /// The C++ graph's raw reference store (13e).
    #[test]
    fn link_stores_the_raw_object() {
        let mut world = World::new();
        let owner = world.spawn(ParticleDragModifier::default()).id();
        let object = world.spawn_empty().id();
        let mut table = StreamObjectTable::default();
        table.push("NiNode", 0x0070_BA70, object);
        let pending = ParticleDragPending {
            drag_object_index: 0,
            ..ParticleDragPending::default()
        };
        link_particle_drag(&mut world, owner, &pending, &table).unwrap();
        let linked = world.get::<ParticleDragModifier>(owner).unwrap();
        assert_eq!(linked.drag_object_18, AvObjectRawRef(Some(object)));
        assert!(linked.drag_object_linked);
    }
}
