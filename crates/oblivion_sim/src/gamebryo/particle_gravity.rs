//! NiPSysGravityModifier.
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::particle_data::ParticleData;
use super::particle_dynamics::{
    ModifierBaseTexts, ParticleModifierBase, parse_particle_modifier_payload, reader_after,
};
use super::raw_reference::{AvObjectRawRef, raw_object_reference};
use super::stream_objects::{NULL_REFERENCE, StreamObjectTable, check_link_index};
use super::transform::{
    NiTransform, compose_scene_transform, invert_scene_transform_checked, normalize_vector,
    rotate_vector,
};

const TRUNCATED: &str = "truncated particle gravity payload";

/// IDA: an injected `rand` (0053D480's CRT call). `None` stands for the C++ empty `std::function`.
pub type GravityRand<'a> = Option<&'a mut dyn FnMut() -> i32>;

/// IDA: 00750BC0 first composes the gravity object's world transform with the inverse particle-system transform
/// (00718A80 / 0053D7A0), then rotates the stored axis (007101F0). These are those two results in particle-system local
/// coordinates, before axis normalization. Completion: 100% of the explicit boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GravityLocalFrame {
    pub origin: [f32; 3],
    pub rotated_axis: [f32; 3],
}

/// IDA: 00750BA0 factory, 00750B50 constructs the 0x3C-byte NiPSysGravityModifier after the 00752BF0 base.
/// Completion: 100% of the cold scalars and the raw force type.
/// TODO: [ ] Bind the NiAVObject lifetime and the modifier's target owner.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ParticleGravityModifier {
    pub base: ParticleModifierBase,
    pub gravity_object_index: u32,
    /// IDA: 0075E870 (shared with NiPSysDragModifier): +0x18 the raw gravity NiAVObject (13e).
    pub gravity_object_18: AvObjectRawRef,
    pub gravity_axis_1c: [f32; 3],
    pub decay: f32,
    pub strength: f32,
    /// 0 planar, 1 spherical; other raw values are retained.
    pub force_type: u32,
    pub turbulence: f32,
    pub turbulence_scale: f32,
}

impl Default for ParticleGravityModifier {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            base: ParticleModifierBase::default(),
            gravity_object_index: NULL_REFERENCE,
            gravity_object_18: AvObjectRawRef::default(),
            gravity_axis_1c: [1.0, 0.0, 0.0],
            decay: 0.0,
            strength: 1.0,
            force_type: 0,
            turbulence: 0.0,
            turbulence_scale: 1.0,
        }
    }
}

/// IDA: N/A host finite validation for the 13-float NiTransform consumed by 00750BF9..00750C3A. Completion: 100%.
/// TODO: [ ] Validate ownership and malformed transform effects in the engine.
fn check_transform(value: &NiTransform) -> Result<(), GameError> {
    if value
        .rotation
        .iter()
        .any(|component| !component.is_finite())
    {
        return Err(GameError::unclassified(
            0,
            "particle gravity world rotation invalid",
        ));
    }
    if value
        .translation
        .iter()
        .any(|component| !component.is_finite())
    {
        return Err(GameError::unclassified(
            0,
            "particle gravity world translation invalid",
        ));
    }
    if !value.scale.is_finite() {
        return Err(GameError::unclassified(
            0,
            "particle gravity world scale invalid",
        ));
    }
    Ok(())
}

/// IDA: 0053D480 calls CRT rand, doubles the integer, divides by 32767.0, subtracts 1.0 and stores to float.
/// Completion: 100% of the draw order and float result for CRT-domain draws.
/// TODO: [ ] Connect to the verified CRT random state.
fn symmetric_rand(draw_rand: &mut GravityRand<'_>) -> Result<f32, GameError> {
    let draw = draw_rand.as_deref_mut().ok_or(GameError::unclassified(
        0,
        "particle gravity random source missing",
    ))?;
    let value = draw();
    if !(0..=32767).contains(&value) {
        return Err(GameError::unclassified(
            0,
            "particle gravity random result outside CRT domain",
        ));
    }
    // fild; fadd st,st (exact); fdiv qword rounds to a float; fsub qword rounds again.
    let quotient = (f64::from(value) * 2.0 / 32767.0) as f32;
    Ok(quotient - 1.0)
}

/// IDA: 00750BC0's turbulence branches call 0053D480 three times in xyz order. Completion: 100% of the draw order and
/// stored noise for finite values.
fn turbulence_force(
    amplitude: f32,
    draw_rand: &mut GravityRand<'_>,
) -> Result<[f32; 3], GameError> {
    let x = symmetric_rand(draw_rand)? * amplitude;
    let y = symmetric_rand(draw_rand)? * amplitude;
    let z = symmetric_rand(draw_rand)? * amplitude;
    Ok([x, y, z])
}

/// IDA: float bits 0x3FCCCCCD, 00750BC0's strength scale.
const STRENGTH_SCALE: f32 = f32::from_bits(0x3FCC_CCCD);

impl ParticleGravityModifier {
    /// IDA: 007510BC..007511AC and 007514B8 / 007516C5, the per-particle direction and distance. Returns the distance.
    /// 007510BC..007511AC uses the transformed gravity axis as the force direction in the spherical decay+turbulence
    /// branch, while its radial vector supplies distance only. 007514B8 / 007516C5 dot the unnormalized local
    /// displacement with the previously normalized planar gravity axis. Completion: 100% of the branches.
    fn calculate_distance(
        &self,
        frame: &GravityLocalFrame,
        axis: &[f32; 3],
        position: &[f32; 3],
        direction: &mut [f32; 3],
    ) -> Result<f32, GameError> {
        if position.iter().any(|component| !component.is_finite()) {
            return Err(GameError::unclassified(
                0,
                "particle gravity position outside finite domain",
            ));
        }
        let mut displacement = [0.0; 3];
        for component in 0..3 {
            displacement[component] = frame.origin[component] - position[component];
        }
        if self.force_type == 1 {
            let distance = normalize_vector(&mut displacement);
            if !(self.decay != 0.0 && self.turbulence != 0.0) {
                *direction = displacement;
            }
            Ok(distance)
        } else {
            let distance =
                axis[0] * displacement[0] + axis[1] * displacement[1] + axis[2] * displacement[2];
            Ok(distance.abs())
        }
    }

    /// IDA: 00750BC0 gravity force update. Force type 0 is planar and 1 is spherical (00751AD0 viewer). The branch
    /// matrix follows zero/nonzero decay and turbulence; the original's loops mutate ParticleInfo velocity in place;
    /// random branches call 0053D480 three times per particle in x/y/z order. Completion: 100% of the supported finite
    /// branch, draw and write effects with a local frame.
    /// TODO: [ ] Verify the CRT exp bits against engine output.
    pub fn update_particles_in_frame(
        &self,
        data: &mut ParticleData,
        current_time: f32,
        local_frame: Option<&GravityLocalFrame>,
        mut draw_rand: GravityRand<'_>,
    ) -> Result<(), GameError> {
        if data.active_count_48 == 0 || self.gravity_object_index == NULL_REFERENCE {
            return Ok(());
        }
        let local_frame = local_frame.ok_or(GameError::unclassified(
            0,
            "linked gravity object local frame unresolved",
        ))?;
        if !current_time.is_finite()
            || !self.decay.is_finite()
            || !self.strength.is_finite()
            || !self.turbulence.is_finite()
            || !self.turbulence_scale.is_finite()
        {
            return Err(GameError::unclassified(
                0,
                "particle gravity scalar outside finite domain",
            ));
        }
        if local_frame.origin.iter().any(|value| !value.is_finite()) {
            return Err(GameError::unclassified(
                0,
                "particle gravity origin outside finite domain",
            ));
        }
        if local_frame
            .rotated_axis
            .iter()
            .any(|value| !value.is_finite())
        {
            return Err(GameError::unclassified(
                0,
                "particle gravity axis outside finite domain",
            ));
        }

        let mut axis = local_frame.rotated_axis;
        normalize_vector(&mut axis);
        if self.force_type > 1 {
            return Ok(()); // The original recognizes only planar 0 and spherical 1.
        }
        let active = usize::from(data.active_count_48);
        if active > data.info_5c.len()
            || ((self.force_type == 1 || self.decay != 0.0) && active > data.positions.len())
        {
            return Err(GameError::unclassified(
                0,
                "active count exceeds particle gravity arrays",
            ));
        }

        let strength_scale = self.strength * STRENGTH_SCALE;
        let has_decay = self.decay != 0.0;
        let has_turbulence = self.turbulence != 0.0;
        let turbulence_amplitude = if has_turbulence {
            self.turbulence_scale * self.turbulence * 500.0
        } else {
            0.0
        };

        for index in 0..active {
            let particle = &data.info_5c[index];
            if !particle.last_update_time_14.is_finite() {
                return Err(GameError::unclassified(
                    0,
                    "particle gravity last update outside finite domain",
                ));
            }
            if particle.velocity_00.iter().any(|value| !value.is_finite()) {
                return Err(GameError::unclassified(
                    0,
                    "particle gravity velocity outside finite domain",
                ));
            }
            let elapsed = current_time - particle.last_update_time_14;
            let mut direction = axis;
            let mut distance = 0.0;

            // 00750E18 / 0075120C derive the direction or distance before the random draws only when decay is zero.
            // Combined decay+turbulence paths at 00751109 / 00751659 draw first, then derive the distance.
            if (self.force_type == 1 || has_decay) && (!has_decay || !has_turbulence) {
                distance = self.calculate_distance(
                    local_frame,
                    &axis,
                    &data.positions[index],
                    &mut direction,
                )?;
            }

            let mut noise = [0.0; 3];
            if has_turbulence {
                noise = turbulence_force(turbulence_amplitude, &mut draw_rand)?;
            }
            if has_decay && has_turbulence {
                distance = self.calculate_distance(
                    local_frame,
                    &axis,
                    &data.positions[index],
                    &mut direction,
                )?;
            }

            let mut force_scale = strength_scale;
            let mut decay_factor = 1.0;
            if has_decay {
                let exponent = -self.decay * distance;
                decay_factor = f64::from(exponent).exp() as f32;
                force_scale *= decay_factor;
            }
            // 00750D54 / 00751273 store (elapsed * strength) before multiplying each normalized component when
            // turbulence is absent. Decay branches store (exp * strength) first, then multiply by elapsed.
            let step_scale = if has_turbulence {
                0.0
            } else {
                force_scale * elapsed
            };
            let particle = &mut data.info_5c[index];
            for component in 0..3 {
                let increment = if has_turbulence {
                    let mut main_force = direction[component] * strength_scale;
                    if has_decay {
                        main_force *= decay_factor;
                    }
                    let total_force = main_force + noise[component];
                    total_force * elapsed
                } else {
                    direction[component] * step_scale
                };
                particle.velocity_00[component] += increment;
            }
        }
        Ok(())
    }

    /// IDA: 00750BF9 copies the gravity object's and target system's +0x64 transforms, 00718A80 inverts the system,
    /// 0053D7A0 composes the relative frame, and 007101F0 rotates the modifier's +0x1C axis. Completion: 100% of the
    /// checked data flow for finite inputs.
    /// TODO: [ ] Verify the NiAVObject's ownership.
    pub fn resolve_frame(
        &self,
        gravity_world: &NiTransform,
        system_world: &NiTransform,
    ) -> Result<GravityLocalFrame, GameError> {
        check_transform(gravity_world)?;
        check_transform(system_world)?;
        if self.gravity_axis_1c.iter().any(|value| !value.is_finite()) {
            return Err(GameError::unclassified(
                0,
                "particle gravity stored axis invalid",
            ));
        }
        // 00718A80's guard as the C++ port applies it.
        let inverse_system = invert_scene_transform_checked(
            system_world,
            GameError::unclassified(0, "particle gravity system world scale invalid"),
        )?;
        let relative = compose_scene_transform(&inverse_system, gravity_world);
        let frame = GravityLocalFrame {
            origin: relative.translation,
            rotated_axis: rotate_vector(&relative.rotation, &self.gravity_axis_1c),
        };
        if frame.origin.iter().any(|value| !value.is_finite()) {
            return Err(GameError::unclassified(
                0,
                "particle gravity relative origin invalid",
            ));
        }
        if frame.rotated_axis.iter().any(|value| !value.is_finite()) {
            return Err(GameError::unclassified(
                0,
                "particle gravity rotated axis invalid",
            ));
        }
        Ok(frame)
    }

    /// IDA: 00750BC0's full gravity-object and system transform, then the active force loop. Completion: 100% of the
    /// composition with the supported force branches.
    /// TODO: [ ] Bind resolved graph owners and compare runtime results.
    pub fn update_particles(
        &self,
        data: &mut ParticleData,
        current_time: f32,
        gravity_world: &NiTransform,
        system_world: &NiTransform,
        draw_rand: GravityRand<'_>,
    ) -> Result<(), GameError> {
        if data.active_count_48 == 0 || self.gravity_object_index == NULL_REFERENCE {
            return Ok(());
        }
        let frame = self.resolve_frame(gravity_world, system_world)?;
        self.update_particles_in_frame(data, current_time, Some(&frame), draw_rand)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParticleGravityLoadResult {
    pub modifier: ParticleGravityModifier,
    pub target_index: u32,
    pub bytes_consumed: usize,
}

/// IDA: 00751920 NiPSysGravityModifier_LoadBinary: the 00752DC0 base (`parse_particle_modifier_payload`), then its own
/// fields with the 00709430 vector3 reader. Completion: 100% of the modern >= 0A010072 widths and order, with
/// >= 0A00010E turbulence fields; references stay deferred and raw.
/// TODO: [ ] Bind NiStream callbacks and the pre-modern NiObject payload.
pub fn parse_particle_gravity_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<ParticleGravityLoadResult, GameError> {
    let base = parse_particle_modifier_payload(
        bytes,
        stream_version,
        &ModifierBaseTexts {
            legacy: "legacy particle modifier prefix unsupported",
            truncated: TRUNCATED,
            name: "truncated particle gravity name",
            active: "truncated particle gravity active byte",
        },
    )?;
    let mut reader = reader_after(bytes, base.bytes_consumed);
    let mut modifier = ParticleGravityModifier::default();
    modifier.base.name_08 = base.name;
    modifier.base.order_0c = base.order;
    let target_index = base.target_index;
    modifier.base.active_14 = base.active;
    modifier.gravity_object_index = reader.u32(TRUNCATED)?;
    for component in &mut modifier.gravity_axis_1c {
        *component = reader.f32(TRUNCATED)?;
    }
    modifier.decay = reader.f32(TRUNCATED)?;
    modifier.strength = reader.f32(TRUNCATED)?;
    modifier.force_type = reader.u32(TRUNCATED)?;
    if stream_version >= 0x0A00_010E {
        modifier.turbulence = reader.f32(TRUNCATED)?;
        modifier.turbulence_scale = reader.f32(TRUNCATED)?;
    }
    Ok(ParticleGravityLoadResult {
        modifier,
        target_index,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 00752CB0 consumes the base target; 0075E870 +0x18 is the next deferred reference (raw). This is the C++ graph's
/// inline arm; here the reference resolves first, so a failure publishes nothing. Completion: 100%.
pub fn link_particle_gravity(
    world: &mut World,
    entity: Entity,
    fields: &ParticleGravityLoadResult,
    table: &StreamObjectTable,
) -> Result<(), GameError> {
    check_link_index(
        fields.target_index,
        table.objects.len(),
        GameError::unclassified(0, "particle gravity target out of range"),
    )?;
    let reference = raw_object_reference(table, fields.modifier.gravity_object_index)?;
    let mut target =
        world
            .get_mut::<ParticleGravityModifier>(entity)
            .ok_or(GameError::unclassified(
                0,
                "particle modifier component missing",
            ))?;
    *target = fields.modifier.clone();
    target.gravity_object_18 = reference;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::particle_data::ParticleInfo;
    use super::*;

    /// 0053D480 at 24-bit: `fdiv qword` rounds to a float, then `fsub qword`. 32/32767 - 1 is 0xBF7FC000 per step and 0xBF7FBFFF
    /// in one rounding (the old `f64` form).
    #[test]
    fn symmetric_rand_rounds_the_quotient_first() {
        let mut draw = || 16;
        let mut source: GravityRand<'_> = Some(&mut draw);
        assert_eq!(symmetric_rand(&mut source).unwrap().to_bits(), 0xBF7F_C000);
    }

    #[test]
    fn payload() {
        let cold = ParticleGravityModifier::default();
        assert_eq!(cold.base.name_08, None);
        assert!(cold.base.active_14);
        assert_eq!(cold.base.order_0c, 0);
        assert_eq!(cold.gravity_object_index, NULL_REFERENCE);
        assert_eq!(cold.gravity_axis_1c, [1.0, 0.0, 0.0]);
        assert_eq!(cold.decay, 0.0);
        assert_eq!(cold.strength, 1.0);
        assert_eq!(cold.force_type, 0);
        assert_eq!(cold.turbulence, 0.0);
        assert_eq!(cold.turbulence_scale, 1.0);
        let mut bytes = Vec::new();
        bytes.extend(1u32.to_le_bytes());
        bytes.push(b'G');
        bytes.extend(7u32.to_le_bytes());
        bytes.extend(u32::MAX.to_le_bytes());
        bytes.push(0);
        bytes.extend(2u32.to_le_bytes()); // gravity object index
        for word in [
            0x3F80_0000u32,
            0x8000_0000,
            0x7FC1_2345,
            0x3F00_0000,
            0x4000_0000,
            0x1122_3344,
            0x4040_0000,
            0x4080_0000,
        ] {
            bytes.extend(word.to_le_bytes());
        }
        let size = bytes.len();
        bytes.push(0xAC);
        let parsed = parse_particle_gravity_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(parsed.bytes_consumed, size);
        let modifier = &parsed.modifier;
        assert_eq!(modifier.base.name_08.as_deref(), Some(&b"G"[..]));
        assert_eq!(modifier.base.order_0c, 7);
        assert!(!modifier.base.active_14);
        assert_eq!(parsed.target_index, NULL_REFERENCE);
        assert_eq!(modifier.gravity_object_index, 2);
        assert_eq!(modifier.gravity_axis_1c[1].to_bits(), 0x8000_0000);
        assert_eq!(modifier.gravity_axis_1c[2].to_bits(), 0x7FC1_2345);
        assert_eq!(modifier.decay, 0.5);
        assert_eq!(modifier.strength, 2.0);
        assert_eq!(modifier.force_type, 0x1122_3344);
        assert_eq!(modifier.turbulence, 3.0);
        assert_eq!(modifier.turbulence_scale, 4.0);
        for length in 0..size {
            assert!(parse_particle_gravity_payload(&bytes[..length], 0x1400_0004).is_err());
        }
        assert!(parse_particle_gravity_payload(&bytes, 0x0A01_0071).is_err());
    }

    fn data() -> ParticleData {
        ParticleData {
            active_count_48: 1,
            info_5c: vec![ParticleInfo::default()],
            positions: vec![[0.0; 3]],
            ..ParticleData::default()
        }
    }

    #[test]
    fn force_update_branches() {
        let mut particle = data();
        let mut gravity = ParticleGravityModifier::default();
        gravity
            .update_particles_in_frame(&mut particle, 1.0, None, None)
            .unwrap();
        assert_eq!(particle.info_5c[0].velocity_00, [0.0; 3]);
        gravity.gravity_object_index = 0;
        assert!(
            gravity
                .update_particles_in_frame(&mut particle, 1.0, None, None)
                .is_err()
        );
        let frame = GravityLocalFrame {
            origin: [0.0; 3],
            rotated_axis: [1.0, 0.0, 0.0],
        };
        gravity
            .update_particles_in_frame(&mut particle, 1.0, Some(&frame), None)
            .unwrap();
        assert!((particle.info_5c[0].velocity_00[0] - 1.6).abs() < 0.000001);
        assert_eq!(particle.info_5c[0].velocity_00[1], 0.0);
        assert_eq!(particle.info_5c[0].last_update_time_14, 0.0);
        gravity.force_type = 1;
        let radial = GravityLocalFrame {
            origin: [2.0, 0.0, 0.0],
            rotated_axis: [0.0, 1.0, 0.0],
        };
        gravity
            .update_particles_in_frame(&mut particle, 1.0, Some(&radial), None)
            .unwrap();
        assert!((particle.info_5c[0].velocity_00[0] - 3.2).abs() < 0.000001);
        assert_eq!(particle.info_5c[0].velocity_00[1], 0.0);
        gravity.decay = 1.0;
        gravity.force_type = 0;
        let mut particle = data();
        particle.positions[0] = [2.0, 0.0, 0.0];
        gravity
            .update_particles_in_frame(&mut particle, 1.0, Some(&frame), None)
            .unwrap();
        assert!((particle.info_5c[0].velocity_00[0] - 1.6 * (-2.0f32).exp()).abs() < 0.000001);
        gravity.decay = 0.0;
        gravity.turbulence = 0.01;
        gravity.turbulence_scale = 1.0;
        let mut particle = data();
        let draws = [0, 16384, 32767];
        let mut next = 0;
        let mut draw = || {
            assert!(next < draws.len(), "gravity consumed an extra random draw");
            next += 1;
            draws[next - 1]
        };
        gravity
            .update_particles_in_frame(&mut particle, 1.0, Some(&frame), Some(&mut draw))
            .unwrap();
        assert_eq!(next, 3);
        assert!((particle.info_5c[0].velocity_00[0] + 3.4).abs() < 0.00001);
        assert!(particle.info_5c[0].velocity_00[1].abs() < 0.001);
        assert!((particle.info_5c[0].velocity_00[2] - 5.0).abs() < 0.00001);
        gravity.force_type = 0x1122_3344;
        let mut particle = data();
        let mut next = 0;
        let mut draw = || {
            next += 1;
            0
        };
        gravity
            .update_particles_in_frame(&mut particle, 1.0, Some(&frame), Some(&mut draw))
            .unwrap();
        assert_eq!(next, 0);
        assert_eq!(particle.info_5c[0].velocity_00, [0.0; 3]);

        // 007510BC combined spherical/decay/turbulence uses the radial distance but applies its main force along the
        // stored transformed axis.
        gravity.force_type = 1;
        gravity.decay = 1.0;
        let mut particle = data();
        particle.positions[0] = [0.0, 2.0, 0.0];
        let mut next = 0;
        let mut draw = || {
            next += 1;
            16383
        };
        gravity
            .update_particles_in_frame(&mut particle, 1.0, Some(&frame), Some(&mut draw))
            .unwrap();
        assert_eq!(next, 3);
        assert!((particle.info_5c[0].velocity_00[0] - 1.6 * (-2.0f32).exp()).abs() < 0.001);
        assert!(particle.info_5c[0].velocity_00[1].abs() < 0.001);
    }

    #[test]
    fn transform_frame() {
        let gravity = ParticleGravityModifier {
            gravity_object_index: 0,
            ..ParticleGravityModifier::default()
        };
        let identity = NiTransform::default();
        let neutral = gravity.resolve_frame(&identity, &identity).unwrap();
        assert_eq!(neutral.origin, [0.0; 3]);
        assert_eq!(neutral.rotated_axis, [1.0, 0.0, 0.0]);
        let gravity_world = NiTransform {
            rotation: [0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            translation: [10.0, 4.0, 0.0],
            scale: 1.0,
        };
        let mut system_world = NiTransform {
            translation: [4.0, 0.0, 0.0],
            scale: 2.0,
            ..NiTransform::default()
        };
        let relative = gravity
            .resolve_frame(&gravity_world, &system_world)
            .unwrap();
        assert_eq!(relative.origin, [3.0, 2.0, 0.0]);
        assert_eq!(relative.rotated_axis, [0.0, 1.0, 0.0]);
        let mut particle = data();
        gravity
            .update_particles(&mut particle, 1.0, &gravity_world, &system_world, None)
            .unwrap();
        assert!((particle.info_5c[0].velocity_00[1] - 1.6).abs() < 0.000001);
        assert_eq!(particle.info_5c[0].velocity_00[0], 0.0);
        system_world.scale = 0.0;
        assert!(
            gravity
                .resolve_frame(&gravity_world, &system_world)
                .is_err()
        );
    }

    /// A random source missing when turbulence wants one, the C++ empty `std::function`.
    #[test]
    fn missing_random_source_is_rejected() {
        let gravity = ParticleGravityModifier {
            gravity_object_index: 0,
            turbulence: 1.0,
            ..ParticleGravityModifier::default()
        };
        let frame = GravityLocalFrame {
            origin: [0.0; 3],
            rotated_axis: [1.0, 0.0, 0.0],
        };
        assert_eq!(
            gravity.update_particles_in_frame(&mut data(), 1.0, Some(&frame), None),
            Err(GameError::unclassified(
                0,
                "particle gravity random source missing"
            ))
        );
    }
}
