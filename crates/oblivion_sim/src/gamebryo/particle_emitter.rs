//! NiPSysEmitter's shared payload and emission loop, and the box, sphere and cylinder volume emitters.
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.

#![allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "0074EE70 and its lifespan comparison test with these negated comparisons, so a NaN takes the same branch"
)]

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;

use super::particle_data::ParticleData;
use super::particle_dynamics::{
    MODIFIER_BASE_TEXTS, ParticleModifierPending, parse_particle_modifier_payload, reader_after,
};
use super::raw_reference::{AvObjectRawRef, raw_object_reference};
use super::stream_objects::{NULL_REFERENCE, StreamObjectTable};
use super::transform::{
    NiTransform, compose_scene_transform, invert_scene_transform_checked, rotate_vector,
    transform_point,
};

const TRUNCATED: &str = "truncated particle box emitter payload";

/// IDA: 00A09B90 / 00A09BD0's float tau.
const ENGINE_TWO_PI: f32 = f32::from_bits(0x40C9_0FDB);

/// The fields of 0074F240 and 00753010, then the box's. A volume emitter's component keeps these whole: +0x18..+0x4C
/// emitter scalars, +0x50 emitter object index, +0x54.. the box's dimensions.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleEmitterPending {
    pub base: ParticleModifierPending,
    pub speed: f32,
    pub speed_variation: f32,
    pub declination: f32,
    pub declination_variation: f32,
    pub planar_angle: f32,
    pub planar_angle_variation: f32,
    pub color: [f32; 4],
    pub initial_radius: f32,
    pub radius_variation: f32,
    pub lifespan: f32,
    pub lifespan_variation: f32,
    pub emitter_object_index: u32,
    pub width: f32,
    pub height: f32,
    pub depth: f32,
    pub bytes_consumed: usize,
}

impl Default for ParticleEmitterPending {
    /// IDA: 0074EDA0 sets emitter floats zero, white RGBA, radius 1; 0074D2D0 clears width, height and depth.
    fn default() -> Self {
        Self {
            base: ParticleModifierPending::default(),
            speed: 0.0,
            speed_variation: 0.0,
            declination: 0.0,
            declination_variation: 0.0,
            planar_angle: 0.0,
            planar_angle_variation: 0.0,
            color: [1.0; 4],
            initial_radius: 1.0,
            radius_variation: 0.0,
            lifespan: 0.0,
            lifespan_variation: 0.0,
            emitter_object_index: NULL_REFERENCE,
            width: 0.0,
            height: 0.0,
            depth: 0.0,
            bytes_consumed: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParticleSphereEmitterPending {
    pub base: ParticleEmitterPending,
    pub radius: f32,
    pub bytes_consumed: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParticleCylinderEmitterPending {
    pub base: ParticleEmitterPending,
    pub radius: f32,
    pub height: f32,
    pub bytes_consumed: usize,
}

/// IDA: 009859DD CRT rand gives one 0..32767 sample per request; 0074D120 invokes it exactly three times per box
/// sample. Completion: 100% of the explicit ordered boundary.
/// TODO: [ ] Bind the CRT's state in a verified runtime owner.
pub trait ParticleEmitterRandom {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn draw_rand15(&mut self) -> u16;
}

/// IDA: 0074EE70 uses the CRT's rand, the virtual +0x60 frame and shape call, then 00749510's notification.
/// Completion: 100% of the explicit boundary.
/// TODO: [ ] Bind the linked ParticleSystem and NiAVObject and the CRT's state.
pub trait ParticleEmitterRuntime: ParticleEmitterRandom {
    /// IDA: 007530C0 resolves a relative frame for each spawned particle.
    /// TODO: [ ] Resolve concrete +0x64 world transforms from linked objects.
    fn resolve_relative_transform(&mut self) -> NiTransform;
    /// IDA: 00749510 runs after all child stores. `target` is what the original's notification reads.
    /// TODO: [ ] Bind the target's modifier listener virtual +0x50 chain.
    fn notify_particle_added(&mut self, index: u16, target: &ParticleData);
}

/// IDA: 0074D120 divides each CRT rand result by double 32767.0 and stores the quotient as one f32 before its box-width
/// multiplication. Completion: 100% of the checked domain and float store.
/// TODO: [ ] Compare x87 division and the CRT's state at runtime.
fn draw_fraction<R: ParticleEmitterRandom + ?Sized>(random: &mut R) -> Result<f32, GameError> {
    let value = random.draw_rand15();
    if value > 32767 {
        return Err(GameError::unclassified(
            0,
            "particle emitter random value outside rand15 domain",
        ));
    }
    Ok((f64::from(value) / 32767.0) as f32)
}

/// IDA: 0074EE70 forms (rand+rand)/32767.0-1.0 for each angular and radius variation and stores the result to f32.
/// Completion: 100% of the checked draw and store.
/// TODO: [ ] Compare x87 division and the CRT's state.
fn draw_centered<R: ParticleEmitterRandom + ?Sized>(random: &mut R) -> Result<f32, GameError> {
    let value = random.draw_rand15();
    if value > 32767 {
        return Err(GameError::unclassified(
            0,
            "particle emitter random value outside rand15 domain",
        ));
    }
    // fild; fadd st,st (exact); fdiv qword rounds to a float; fsub qword rounds again.
    let wide = f64::from(value);
    Ok(((wide + wide) / 32767.0) as f32 - 1.0)
}

/// IDA: 007530C0 uses identity when the +0x50 emitter object is null; otherwise inverse(target world) composed with
/// emitter world via 00718A80 / 0053D7A0, before virtual +0x68. Completion: 100% of the control and transform order
/// with explicit resolved frames.
/// TODO: [ ] Retain concrete emitter and target NiAVObject identities and the scheduler.
fn relative_frame(
    emitter_world: Option<&NiTransform>,
    target_world: &NiTransform,
) -> Result<NiTransform, GameError> {
    let Some(emitter_world) = emitter_world else {
        return Ok(NiTransform::default());
    };
    let inverse_target = invert_scene_transform_checked(
        target_world,
        GameError::unclassified(0, "particle emitter target scale invalid"),
    )?;
    Ok(compose_scene_transform(&inverse_target, emitter_world))
}

/// IDA: 0074F240's fields, which every NiPSysEmitter reads: six emitter floats, RGBA, radius, optional radius variation
/// at >= 0A030002, lifespan and variation. `truncated` keeps each caller's C++ message. Completion: 100% of the bounded
/// modern order and raw float values.
/// TODO: [ ] Legacy modifier prefixes and NiStream callback errors.
pub(super) fn parse_emitter_scalars<'a>(
    bytes: &'a [u8],
    version: u32,
    truncated: &'static str,
) -> Result<(ParticleEmitterPending, ByteReader<'a>), GameError> {
    let mut pending = ParticleEmitterPending {
        base: parse_particle_modifier_payload(bytes, version, &MODIFIER_BASE_TEXTS)?,
        ..ParticleEmitterPending::default()
    };
    let mut reader = reader_after(bytes, pending.base.bytes_consumed);
    pending.speed = reader.f32(truncated)?;
    pending.speed_variation = reader.f32(truncated)?;
    pending.declination = reader.f32(truncated)?;
    pending.declination_variation = reader.f32(truncated)?;
    pending.planar_angle = reader.f32(truncated)?;
    pending.planar_angle_variation = reader.f32(truncated)?;
    for channel in &mut pending.color {
        *channel = reader.f32(truncated)?;
    }
    pending.initial_radius = reader.f32(truncated)?;
    if version >= 0x0A03_0002 {
        pending.radius_variation = reader.f32(truncated)?;
    }
    pending.lifespan = reader.f32(truncated)?;
    pending.lifespan_variation = reader.f32(truncated)?;
    Ok((pending, reader))
}

/// IDA: 00753010 the shared VolumeEmitter payload: 0074F240, then one deferred object. Completion: 100%.
/// TODO: [ ] Legacy modifier prefixes and NiStream callback errors.
fn parse_emitter_base_payload(
    bytes: &[u8],
    version: u32,
) -> Result<(ParticleEmitterPending, ByteReader<'_>), GameError> {
    let (mut pending, mut reader) = parse_emitter_scalars(bytes, version, TRUNCATED)?;
    pending.emitter_object_index = reader.u32(TRUNCATED)?;
    pending.bytes_consumed = reader.position();
    Ok((pending, reader))
}

/// IDA: 00753180 follows the base modifier link with the +0x50 emitter-object lookup. Completion: 100% of the bounds
/// check before publication.
/// TODO: [ ] Graph type and owner checks.
fn validate_emitter_links(
    pending: &ParticleEmitterPending,
    object_count: usize,
) -> Result<(), GameError> {
    if (pending.base.target_index != NULL_REFERENCE
        && pending.base.target_index as usize >= object_count)
        || (pending.emitter_object_index != NULL_REFERENCE
            && pending.emitter_object_index as usize >= object_count)
    {
        return Err(GameError::unclassified(
            0,
            "particle emitter link out of range",
        ));
    }
    Ok(())
}

/// IDA: 0074D310 calls 00753010, then reads the +0x54 / +0x58 / +0x5C dimensions. Completion: 100% of the bounded modern
/// byte sequence and raw f32 fields.
/// TODO: [ ] NiStream callback errors.
pub fn parse_particle_box_emitter_payload(
    bytes: &[u8],
    version: u32,
) -> Result<ParticleEmitterPending, GameError> {
    let (mut pending, mut reader) = parse_emitter_base_payload(bytes, version)?;
    pending.width = reader.f32(TRUNCATED)?;
    pending.height = reader.f32(TRUNCATED)?;
    pending.depth = reader.f32(TRUNCATED)?;
    pending.bytes_consumed = reader.position();
    Ok(pending)
}

/// IDA: 0074D010 calls the shared 00753010, then reads the +0x54 radius f32. Completion: 100% of the bounded payload and
/// raw bits.
/// TODO: [ ] NiStream's short-read callback status.
pub fn parse_particle_sphere_emitter_payload(
    bytes: &[u8],
    version: u32,
) -> Result<ParticleSphereEmitterPending, GameError> {
    let (base, mut reader) = parse_emitter_base_payload(bytes, version)?;
    let radius = reader.f32(TRUNCATED)?;
    Ok(ParticleSphereEmitterPending {
        base,
        radius,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 0074CCA0 calls the shared 00753010, then reads +0x54 radius and +0x58 height as two f32 words. Completion: 100%
/// of the bounded payload and raw bits.
/// TODO: [ ] NiStream's short-read callback status.
pub fn parse_particle_cylinder_emitter_payload(
    bytes: &[u8],
    version: u32,
) -> Result<ParticleCylinderEmitterPending, GameError> {
    let (base, mut reader) = parse_emitter_base_payload(bytes, version)?;
    let radius = reader.f32(TRUNCATED)?;
    let height = reader.f32(TRUNCATED)?;
    Ok(ParticleCylinderEmitterPending {
        base,
        radius,
        height,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 0074D2D0 constructs the 0x60-byte NiPSysBoxEmitter through 00752FD0 and 0074EDA0.
/// TODO: [ ] Bind the emitter-object NiAVObject and update scheduling.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ParticleBoxEmitter {
    /// IDA: 0074EDA0 / 0074F240 +0x18.. scalars, 00753010 +0x50 index, 0074D310 +0x54 / +0x58 / +0x5C dimensions.
    pub state: ParticleEmitterPending,
    /// IDA: 00753030 NiPSysVolumeEmitter_LinkObject: +0x50 the raw emitter NiAVObject (13e).
    pub emitter_object_50: AvObjectRawRef,
}

impl ParticleBoxEmitter {
    /// IDA: 0074D120 draws z, y, x, writes the local box position using the matching dimension and 0.5 subtraction,
    /// applies the 0053D4B0 transform and rotates the incoming velocity with 007101F0. Completion: 100% of the draw
    /// order and field stores for an explicit relative frame.
    /// TODO: [ ] Compare x87 results and link the frame owners.
    pub fn generate<R: ParticleEmitterRandom + ?Sized>(
        &self,
        emitter_to_target: &NiTransform,
        position: &mut [f32; 3],
        velocity: &mut [f32; 3],
        random: &mut R,
    ) -> Result<(), GameError> {
        let random_z = draw_fraction(random)?;
        let random_y = draw_fraction(random)?;
        let random_x = draw_fraction(random)?;
        let local = [
            (random_x - 0.5) * self.state.width,
            (random_y - 0.5) * self.state.height,
            (random_z - 0.5) * self.state.depth,
        ];
        *position = transform_point(emitter_to_target, &local);
        *velocity = rotate_vector(&emitter_to_target.rotation, velocity);
        Ok(())
    }

    /// IDA: 007530C0 then virtual +0x68. Completion: 100% of the explicit dispatch.
    /// TODO: [ ] Resolve live NiAVObject +0x64 transforms from linked objects.
    pub fn generate_from_world_frames<R: ParticleEmitterRandom + ?Sized>(
        &self,
        emitter_world: Option<&NiTransform>,
        target_world: &NiTransform,
        position: &mut [f32; 3],
        velocity: &mut [f32; 3],
        random: &mut R,
    ) -> Result<(), GameError> {
        let relative = relative_frame(emitter_world, target_world)?;
        self.generate(&relative, position, velocity, random)
    }

    /// IDA: 0074EE70 shared +0x5C with the box's +0x60 / +0x68 placement. Completion: 100% of the bounded base-data loop
    /// and conditional random order.
    /// TODO: [ ] Bind the scheduler and the NiAVObject and runtime owners.
    pub fn emit<R: ParticleEmitterRuntime>(
        &self,
        current_time: f32,
        particle_ages: &[f32],
        target: &mut ParticleData,
        runtime: &mut R,
    ) -> Result<(), GameError> {
        emit_particles(
            &self.state,
            current_time,
            particle_ages,
            target,
            runtime,
            |relative, position, velocity, service| {
                self.generate(relative, position, velocity, service)
            },
        )
    }
}

/// IDA: 0074CFE0 constructs the 0x58-byte NiPSysSphereEmitter through the shared Volume and Emitter constructors, with
/// the +0x54 radius zero.
/// TODO: [ ] Bind concrete source and target NiAVObject owners and emitter update.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ParticleSphereEmitter {
    pub state: ParticleEmitterPending,
    /// IDA: 00753030 NiPSysVolumeEmitter_LinkObject: +0x50 the raw emitter NiAVObject (13e).
    pub emitter_object_50: AvObjectRawRef,
    pub radius_54: f32,
}

impl ParticleSphereEmitter {
    /// IDA: 0074CE30 draws radius, azimuth, polar as three sequential rand calls, with tau initialized by 00A09B90 /
    /// 00A09BD0. It builds spherical x/y/z, transforms the position via 0053D4B0, then rotates the velocity via 007101F0.
    /// The C++ `std::sin(float)` overloads are f32 here too. Completion: 100% of the draw, branch and store sequence.
    /// TODO: [ ] Compare the fsincos and __CIsin / __CIcos bits externally.
    pub fn generate<R: ParticleEmitterRandom + ?Sized>(
        &self,
        emitter_to_target: &NiTransform,
        position: &mut [f32; 3],
        velocity: &mut [f32; 3],
        random: &mut R,
    ) -> Result<(), GameError> {
        let radius_random = draw_fraction(random)?;
        let sampled_radius = self.radius_54 * radius_random;
        let azimuth_random = draw_fraction(random)?;
        let azimuth = ENGINE_TWO_PI * azimuth_random;
        let polar_random = draw_fraction(random)?;
        let polar = ENGINE_TWO_PI * polar_random;
        let cosine_azimuth = f64::from(azimuth).cos() as f32;
        let sine_azimuth = f64::from(azimuth).sin() as f32;
        let cosine_polar = f64::from(polar).cos() as f32;
        let sine_polar = f64::from(polar).sin() as f32;
        let horizontal = sine_polar * sampled_radius;
        let local = [
            cosine_azimuth * horizontal,
            horizontal * sine_azimuth,
            sampled_radius * cosine_polar,
        ];
        *position = transform_point(emitter_to_target, &local);
        *velocity = rotate_vector(&emitter_to_target.rotation, velocity);
        Ok(())
    }

    /// IDA: 007530C0 then virtual +0x68. Completion: 100% of the explicit dispatch.
    /// TODO: [ ] Bind concrete linked world transform owners.
    pub fn generate_from_world_frames<R: ParticleEmitterRandom + ?Sized>(
        &self,
        emitter_world: Option<&NiTransform>,
        target_world: &NiTransform,
        position: &mut [f32; 3],
        velocity: &mut [f32; 3],
        random: &mut R,
    ) -> Result<(), GameError> {
        let relative = relative_frame(emitter_world, target_world)?;
        self.generate(&relative, position, velocity, random)
    }

    /// IDA: 0074EE70 shared +0x5C with 0074CE30's placement. Completion: 100% of the bounded base-data loop and
    /// conditional random order.
    /// TODO: [ ] Bind the scheduler and target listeners.
    pub fn emit<R: ParticleEmitterRuntime>(
        &self,
        current_time: f32,
        particle_ages: &[f32],
        target: &mut ParticleData,
        runtime: &mut R,
    ) -> Result<(), GameError> {
        emit_particles(
            &self.state,
            current_time,
            particle_ages,
            target,
            runtime,
            |relative, position, velocity, service| {
                self.generate(relative, position, velocity, service)
            },
        )
    }
}

/// IDA: 0074CC70 constructs the 0x5C-byte NiPSysCylinderEmitter through the shared constructors, with +0x54 radius and
/// +0x58 height zero.
/// TODO: [ ] Bind concrete source and target NiAVObject owners and emitter update.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ParticleCylinderEmitter {
    pub state: ParticleEmitterPending,
    /// IDA: 00753030 NiPSysVolumeEmitter_LinkObject: +0x50 the raw emitter NiAVObject (13e).
    pub emitter_object_50: AvObjectRawRef,
    pub radius_54: f32,
    pub height_58: f32,
}

impl ParticleCylinderEmitter {
    /// IDA: 0074CAD0 draws radius, azimuth, computes and stores x and y, then draws height and stores z. It transforms
    /// the position and rotates the existing velocity. Completion: 100% of the three-draw order and f32 stores.
    /// TODO: [ ] Compare the fsincos and __CIsin / __CIcos bits externally.
    pub fn generate<R: ParticleEmitterRandom + ?Sized>(
        &self,
        emitter_to_target: &NiTransform,
        position: &mut [f32; 3],
        velocity: &mut [f32; 3],
        random: &mut R,
    ) -> Result<(), GameError> {
        let radius_random = draw_fraction(random)?;
        let sampled_radius = self.radius_54 * radius_random;
        let azimuth_random = draw_fraction(random)?;
        let azimuth = ENGINE_TWO_PI * azimuth_random;
        let cosine = f64::from(azimuth).cos() as f32;
        let sine = f64::from(azimuth).sin() as f32;
        position[0] = cosine * sampled_radius;
        position[1] = sampled_radius * sine;
        let height_random = draw_fraction(random)?;
        position[2] = (height_random - 0.5) * self.height_58;
        *position = transform_point(emitter_to_target, position);
        *velocity = rotate_vector(&emitter_to_target.rotation, velocity);
        Ok(())
    }

    /// IDA: 007530C0 then virtual +0x68. Completion: 100% of the explicit dispatch.
    /// TODO: [ ] Bind concrete linked world transform owners.
    pub fn generate_from_world_frames<R: ParticleEmitterRandom + ?Sized>(
        &self,
        emitter_world: Option<&NiTransform>,
        target_world: &NiTransform,
        position: &mut [f32; 3],
        velocity: &mut [f32; 3],
        random: &mut R,
    ) -> Result<(), GameError> {
        let relative = relative_frame(emitter_world, target_world)?;
        self.generate(&relative, position, velocity, random)
    }

    /// IDA: 0074EE70 shared +0x5C with 0074CAD0's placement. Completion: 100% of the bounded base-data loop and
    /// conditional random order.
    /// TODO: [ ] Bind the scheduler and target listeners.
    pub fn emit<R: ParticleEmitterRuntime>(
        &self,
        current_time: f32,
        particle_ages: &[f32],
        target: &mut ParticleData,
        runtime: &mut R,
    ) -> Result<(), GameError> {
        emit_particles(
            &self.state,
            current_time,
            particle_ages,
            target,
            runtime,
            |relative, position, velocity, service| {
                self.generate(relative, position, velocity, service)
            },
        )
    }
}

/// IDA: N/A host extent preflight for the fixed-size NiPSysData arrays. Completion: 100% of the checked storage before
/// any RNG, counter or particle mutation.
/// TODO: [ ] Subclass arrays and the original's heap ownership.
fn check_emitter_arrays(data: &ParticleData) -> Result<(), GameError> {
    let capacity = usize::from(data.vertex_count);
    if data.positions.len() < capacity
        || data.info_5c.len() < capacity
        || (!data.colors.is_empty() && data.colors.len() < capacity)
        || (!data.radii.is_empty() && data.radii.len() < capacity)
        || (!data.sizes_4c.is_empty() && data.sizes_4c.len() < capacity)
    {
        return Err(GameError::unclassified(
            0,
            "particle emitter target array is too short",
        ));
    }
    Ok(())
}

/// IDA: 0074EE70 shared NiPSysEmitter +0x5C loop. For each age it draws one lifespan value; accepted ages reserve a slot,
/// draw speed, declination and planar, run virtual +0x60's shape placement, optionally draw radius, write size and last
/// update, then invoke 00749510. A capacity failure returns from the whole loop. The C++ `std::sin(float)` overloads are
/// f32 here too. Completion: 100% of the bounded branch, draw and store topology for base NiPSysData.
/// TODO: [ ] Bind the CRT's state, derived arrays and the target owner.
fn emit_particles<R: ParticleEmitterRuntime>(
    fields: &ParticleEmitterPending,
    current_time: f32,
    particle_ages: &[f32],
    target: &mut ParticleData,
    runtime: &mut R,
    mut shape: impl FnMut(&NiTransform, &mut [f32; 3], &mut [f32; 3], &mut R) -> Result<(), GameError>,
) -> Result<(), GameError> {
    if particle_ages.len() > usize::from(u16::MAX) {
        return Err(GameError::unclassified(
            0,
            "particle emitter candidate count exceeds u16",
        ));
    }
    check_emitter_arrays(target)?;
    for &particle_age in particle_ages {
        let lifespan_random = draw_fraction(runtime)?;
        let lifespan = (lifespan_random - 0.5) * fields.lifespan_variation + fields.lifespan;
        if !(lifespan >= particle_age) {
            continue;
        }
        let slot_word = target.reserve_added_slot();
        if slot_word == 0xFFFF {
            return Ok(());
        }
        let slot = usize::from(slot_word);
        let speed_random = draw_fraction(runtime)?;
        let speed = (speed_random - 0.5) * fields.speed_variation + fields.speed;
        let declination_random = draw_centered(runtime)?;
        let declination = fields.declination_variation * declination_random + fields.declination;
        let planar_random = draw_centered(runtime)?;
        let planar = fields.planar_angle_variation * planar_random + fields.planar_angle;
        let sine_declination = f64::from(declination).sin() as f32;
        let cosine_planar = f64::from(planar).cos() as f32;
        let direction_x = cosine_planar * sine_declination;
        let sine_planar = f64::from(planar).sin() as f32;
        let direction_y = sine_planar * sine_declination;
        let cosine_declination = f64::from(declination).cos() as f32;
        let info = &mut target.info_5c[slot];
        info.generation_18 = 0;
        info.unresolved_flags_1a = 0; // 0074EE70 writes one zero dword at +0x18.
        info.velocity_00[0] = direction_x * speed;
        info.velocity_00[1] = direction_y * speed;
        info.velocity_00[2] = speed * cosine_declination;
        info.age_0c = particle_age;
        info.lifespan_10 = lifespan;
        let relative = runtime.resolve_relative_transform();
        shape(
            &relative,
            &mut target.positions[slot],
            &mut target.info_5c[slot].velocity_00,
            runtime,
        )?;
        if !target.colors.is_empty() {
            target.colors[slot] = fields.color;
        }
        if !target.radii.is_empty() {
            let radius_random = draw_centered(runtime)?;
            target.radii[slot] = fields.radius_variation * radius_random + fields.initial_radius;
        }
        if !target.sizes_4c.is_empty() {
            target.sizes_4c[slot] = 1.0;
        }
        target.info_5c[slot].last_update_time_14 = current_time - target.info_5c[slot].age_0c;
        runtime.notify_particle_added(slot_word, target);
    }
    Ok(())
}

/// IDA: 00753180 resolves the common target and +0x50 emitter-object index. Completion: 100% of the checked bounds
/// before publication; the raw reference is `link_particle_box_emitter`'s.
/// TODO: [ ] Resolve concrete NiAVObject and ParticleSystem types and ownership.
pub fn resolve_particle_box_emitter_link(
    target: &mut ParticleBoxEmitter,
    pending: &ParticleEmitterPending,
    object_count: usize,
) -> Result<(), GameError> {
    validate_emitter_links(pending, object_count)?;
    target.state = pending.clone();
    Ok(())
}

/// IDA: 00753180's common deferred target and emitter-object lookup occurs before the sphere's +0x54 radius is used.
/// Completion: 100% of the checked publication.
/// TODO: [ ] Resolve graph ParticleSystem and NiAVObject owners.
pub fn resolve_particle_sphere_emitter_link(
    target: &mut ParticleSphereEmitter,
    pending: &ParticleSphereEmitterPending,
    object_count: usize,
) -> Result<(), GameError> {
    validate_emitter_links(&pending.base, object_count)?;
    target.state = pending.base.clone();
    target.radius_54 = pending.radius;
    Ok(())
}

/// IDA: 00753180's common deferred target and emitter-object lookup occurs before the cylinder's +0x54 / +0x58
/// dimensions are used. Completion: 100% of the checked publication.
/// TODO: [ ] Resolve graph ParticleSystem and NiAVObject owners.
pub fn resolve_particle_cylinder_emitter_link(
    target: &mut ParticleCylinderEmitter,
    pending: &ParticleCylinderEmitterPending,
    object_count: usize,
) -> Result<(), GameError> {
    validate_emitter_links(&pending.base, object_count)?;
    target.state = pending.base.clone();
    target.radius_54 = pending.radius;
    target.height_58 = pending.height;
    Ok(())
}

const MISSING_EMITTER: &str = "particle modifier component missing";

/// IDA: 00753030 on a stream object: 00752CB0 consumes the base deferred target, then +0x50 takes the next deferred
/// reference as a raw pointer. Completion: 100%.
pub fn link_particle_box_emitter(
    world: &mut World,
    entity: Entity,
    pending: &ParticleEmitterPending,
    table: &StreamObjectTable,
) -> Result<(), GameError> {
    let mut target = world
        .get_mut::<ParticleBoxEmitter>(entity)
        .ok_or(GameError::from_legacy_message(MISSING_EMITTER))?;
    resolve_particle_box_emitter_link(&mut target, pending, table.objects.len())?;
    target.emitter_object_50 = raw_object_reference(table, pending.emitter_object_index)?;
    Ok(())
}

/// IDA: 00753030 on a stream object. Completion: 100%.
pub fn link_particle_sphere_emitter(
    world: &mut World,
    entity: Entity,
    pending: &ParticleSphereEmitterPending,
    table: &StreamObjectTable,
) -> Result<(), GameError> {
    let mut target = world
        .get_mut::<ParticleSphereEmitter>(entity)
        .ok_or(GameError::from_legacy_message(MISSING_EMITTER))?;
    resolve_particle_sphere_emitter_link(&mut target, pending, table.objects.len())?;
    target.emitter_object_50 = raw_object_reference(table, pending.base.emitter_object_index)?;
    Ok(())
}

/// IDA: 00753030 on a stream object. Completion: 100%.
pub fn link_particle_cylinder_emitter(
    world: &mut World,
    entity: Entity,
    pending: &ParticleCylinderEmitterPending,
    table: &StreamObjectTable,
) -> Result<(), GameError> {
    let mut target = world
        .get_mut::<ParticleCylinderEmitter>(entity)
        .ok_or(GameError::from_legacy_message(MISSING_EMITTER))?;
    resolve_particle_cylinder_emitter_link(&mut target, pending, table.objects.len())?;
    target.emitter_object_50 = raw_object_reference(table, pending.base.emitter_object_index)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::particle_data::ParticleInfo;
    use super::*;

    fn word(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }

    fn fixture(modern: bool) -> ParticleEmitterPending {
        let mut bytes = Vec::new();
        word(&mut bytes, 3);
        bytes.extend(b"Box");
        word(&mut bytes, 7);
        word(&mut bytes, u32::MAX);
        bytes.push(0);
        for value in [
            0x3F80_0000u32,
            0x4000_0000,
            0x4040_0000,
            0x4080_0000,
            0x40A0_0000,
            0x40C0_0000,
            0x3F80_0000,
            0x3F00_0000,
            0x3E80_0000,
            0x3E00_0000,
            0x40E0_0000,
        ] {
            word(&mut bytes, value);
        }
        if modern {
            word(&mut bytes, 0x8000_0000); // signed zero radius variation
        }
        word(&mut bytes, 0x4100_0000); // lifespan 8
        word(&mut bytes, 0x4110_0000); // variation 9
        word(&mut bytes, u32::MAX); // null emitter object
        word(&mut bytes, 0x4000_0000); // width 2
        word(&mut bytes, 0x4080_0000); // height 4
        word(&mut bytes, 0x40C0_0000); // depth 6
        let size = bytes.len();
        bytes.push(0xAA);
        let version = if modern { 0x1400_0004 } else { 0x0A01_0072 };
        let pending = parse_particle_box_emitter_payload(&bytes, version).unwrap();
        assert_eq!(pending.bytes_consumed, size);
        assert_eq!(pending.base.name.as_deref(), Some(&b"Box"[..]));
        assert_eq!(pending.base.order, 7);
        assert!(!pending.base.active);
        assert_eq!(pending.speed, 1.0);
        assert_eq!(pending.color[3], 0.125);
        assert_eq!(pending.initial_radius, 7.0);
        assert_eq!(pending.lifespan, 8.0);
        assert_eq!(pending.width, 2.0);
        assert_eq!(pending.height, 4.0);
        assert_eq!(pending.depth, 6.0);
        if modern {
            assert_eq!(pending.radius_variation.to_bits(), 0x8000_0000);
        } else {
            assert_eq!(pending.radius_variation, 0.0);
        }
        for length in 0..size {
            assert!(parse_particle_box_emitter_payload(&bytes[..length], version).is_err());
        }
        pending
    }

    #[derive(Default)]
    struct Random {
        values: Vec<u16>,
        next: usize,
    }

    impl ParticleEmitterRandom for Random {
        fn draw_rand15(&mut self) -> u16 {
            assert!(
                self.next < self.values.len(),
                "box emitter consumed an extra random draw"
            );
            self.next += 1;
            self.values[self.next - 1]
        }
    }

    #[test]
    fn box_emitter() {
        let mut cold = ParticleBoxEmitter::default();
        assert_eq!(cold.state.base.name, None);
        assert_eq!(cold.state.base.order, 0);
        assert!(cold.state.base.active);
        assert_eq!(cold.state.color, [1.0; 4]);
        assert_eq!(cold.state.initial_radius, 1.0);
        assert_eq!(cold.state.radius_variation, 0.0);
        assert_eq!(cold.state.width, 0.0);
        assert_eq!(cold.state.height, 0.0);
        assert_eq!(cold.state.depth, 0.0);
        assert_eq!(cold.state.emitter_object_index, NULL_REFERENCE);
        let mut pending = fixture(true);
        let _ = fixture(false);
        resolve_particle_box_emitter_link(&mut cold, &pending, 0).unwrap();
        assert_eq!(cold.state.base.name.as_deref(), Some(&b"Box"[..]));
        assert_eq!(cold.state.base.order, 7);
        assert!(!cold.state.base.active);
        assert_eq!(cold.state.width, 2.0);
        assert_eq!(cold.state.height, 4.0);
        assert_eq!(cold.state.depth, 6.0);
        pending.emitter_object_index = 1;
        assert!(resolve_particle_box_emitter_link(&mut cold, &pending, 1).is_err());
        assert_eq!(cold.state.emitter_object_index, NULL_REFERENCE);

        let relative = NiTransform {
            rotation: [0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            translation: [10.0, 4.0, 0.0],
            scale: 2.0,
        };
        let mut position = [0.0; 3];
        let mut velocity = [1.0, 0.0, 0.0];
        let mut random = Random {
            values: vec![0, 16384, 32767], // z, y, x
            next: 0,
        };
        cold.generate(&relative, &mut position, &mut velocity, &mut random)
            .unwrap();
        assert_eq!(random.next, 3);
        assert!((position[0] - 10.0).abs() < 0.001);
        assert_eq!(position[1], 6.0);
        assert_eq!(position[2], -6.0);
        assert_eq!(velocity, [0.0, 1.0, 0.0]);
        random.next = 0;
        position = [0.0; 3];
        velocity = [1.0, 0.0, 0.0];
        let target_world = NiTransform {
            translation: [4.0, 0.0, 0.0],
            scale: 2.0,
            ..NiTransform::default()
        };
        cold.generate_from_world_frames(
            Some(&relative),
            &target_world,
            &mut position,
            &mut velocity,
            &mut random,
        )
        .unwrap();
        assert_eq!(random.next, 3);
        assert!((position[0] - 3.0).abs() < 0.001);
        assert_eq!(position[1], 3.0);
        assert_eq!(position[2], -3.0);
        assert_eq!(velocity, [0.0, 1.0, 0.0]);
    }

    fn shape_payload() -> Vec<u8> {
        let mut bytes = Vec::new();
        word(&mut bytes, 0);
        word(&mut bytes, 3);
        word(&mut bytes, u32::MAX);
        bytes.push(1);
        for _ in 0..6 {
            word(&mut bytes, 0);
        }
        for _ in 0..4 {
            word(&mut bytes, 0x3F80_0000);
        }
        word(&mut bytes, 0x3F80_0000); // initial radius
        word(&mut bytes, 0); // radius variation
        word(&mut bytes, 0);
        word(&mut bytes, 0); // lifespan and variation
        word(&mut bytes, u32::MAX); // null emitter object
        bytes
    }

    #[test]
    fn sphere_and_cylinder() {
        let mut sphere = ParticleSphereEmitter::default();
        let mut cylinder = ParticleCylinderEmitter::default();
        assert_eq!(sphere.state.base.name, None);
        assert_eq!(sphere.radius_54, 0.0);
        assert_eq!(cylinder.state.base.name, None);
        assert_eq!(cylinder.radius_54, 0.0);
        assert_eq!(cylinder.height_58, 0.0);
        let mut sphere_bytes = shape_payload();
        word(&mut sphere_bytes, 0x4000_0000); // radius 2
        let mut cylinder_bytes = shape_payload();
        word(&mut cylinder_bytes, 0x4000_0000); // radius 2
        word(&mut cylinder_bytes, 0x4080_0000); // height 4
        let sphere_pending =
            parse_particle_sphere_emitter_payload(&sphere_bytes, 0x1400_0004).unwrap();
        let cylinder_pending =
            parse_particle_cylinder_emitter_payload(&cylinder_bytes, 0x1400_0004).unwrap();
        assert_eq!(sphere_pending.bytes_consumed, sphere_bytes.len());
        assert_eq!(cylinder_pending.bytes_consumed, cylinder_bytes.len());
        assert_eq!(sphere_pending.radius, 2.0);
        assert_eq!(cylinder_pending.radius, 2.0);
        assert_eq!(cylinder_pending.height, 4.0);
        resolve_particle_sphere_emitter_link(&mut sphere, &sphere_pending, 0).unwrap();
        resolve_particle_cylinder_emitter_link(&mut cylinder, &cylinder_pending, 0).unwrap();
        assert_eq!(sphere.radius_54, 2.0);
        assert_eq!(cylinder.radius_54, 2.0);
        assert_eq!(cylinder.height_58, 4.0);
        assert_eq!(sphere.state.base.order, 3);
        let identity = NiTransform::default();
        let mut position = [0.0; 3];
        let mut velocity = [1.0, 0.0, 0.0];
        let mut random = Random {
            values: vec![32767, 0, 0], // radius, azimuth, polar
            next: 0,
        };
        sphere
            .generate(&identity, &mut position, &mut velocity, &mut random)
            .unwrap();
        assert_eq!(random.next, 3);
        assert!(position[0].abs() < 0.001);
        assert!(position[1].abs() < 0.001);
        assert_eq!(position[2], 2.0);
        assert_eq!(velocity, [1.0, 0.0, 0.0]);
        random.next = 0;
        random.values = vec![32767, 0, 32767]; // radius, azimuth, height
        position = [0.0; 3];
        velocity = [1.0, 0.0, 0.0];
        cylinder
            .generate(&identity, &mut position, &mut velocity, &mut random)
            .unwrap();
        assert_eq!(random.next, 3);
        assert_eq!(position, [2.0, 0.0, 2.0]);
        assert_eq!(velocity, [1.0, 0.0, 0.0]);
        let emitter_world = NiTransform {
            translation: [10.0, 4.0, 0.0],
            ..NiTransform::default()
        };
        let target_world = NiTransform {
            translation: [4.0, 0.0, 0.0],
            scale: 2.0,
            ..NiTransform::default()
        };
        random.next = 0;
        position = [0.0; 3];
        velocity = [1.0, 0.0, 0.0];
        cylinder
            .generate_from_world_frames(
                Some(&emitter_world),
                &target_world,
                &mut position,
                &mut velocity,
                &mut random,
            )
            .unwrap();
        assert_eq!(random.next, 3);
        assert_eq!(position, [4.0, 2.0, 1.0]);
    }

    #[derive(Default)]
    struct Runtime {
        values: Vec<u16>,
        next: usize,
        frames: usize,
        notifications: Vec<u16>,
    }

    impl ParticleEmitterRandom for Runtime {
        fn draw_rand15(&mut self) -> u16 {
            assert!(
                self.next < self.values.len(),
                "emitter loop consumed an extra random draw"
            );
            self.next += 1;
            self.values[self.next - 1]
        }
    }

    impl ParticleEmitterRuntime for Runtime {
        fn resolve_relative_transform(&mut self) -> NiTransform {
            self.frames += 1;
            NiTransform::default()
        }

        fn notify_particle_added(&mut self, index: u16, target: &ParticleData) {
            self.notifications.push(index);
            assert_eq!(target.info_5c[usize::from(index)].last_update_time_14, 3.5);
            assert_eq!(target.sizes_4c[usize::from(index)], 1.0);
        }
    }

    fn emission_data(capacity: u16) -> ParticleData {
        let count = usize::from(capacity);
        ParticleData {
            vertex_count: capacity,
            positions: vec![[0.0; 3]; count],
            info_5c: vec![ParticleInfo::default(); count],
            colors: vec![[0.0; 4]; count],
            radii: vec![0.0; count],
            sizes_4c: vec![0.0; count],
            ..ParticleData::default()
        }
    }

    #[test]
    fn emission_loop() {
        let pending = ParticleEmitterPending {
            speed: 2.0,
            lifespan: 2.0,
            initial_radius: 1.0,
            color: [0.25, 0.5, 0.75, 1.0],
            width: 2.0,
            height: 4.0,
            depth: 6.0,
            ..ParticleEmitterPending::default()
        };
        let mut emitter = ParticleBoxEmitter::default();
        resolve_particle_box_emitter_link(&mut emitter, &pending, 0).unwrap();
        let mut target = emission_data(2);
        let mut runtime = Runtime {
            values: vec![0; 12],
            ..Runtime::default()
        };
        emitter
            .emit(4.0, &[3.0, 0.5], &mut target, &mut runtime)
            .unwrap();
        assert_eq!(runtime.next, 9);
        assert_eq!(runtime.frames, 1);
        assert_eq!(runtime.notifications, [0]);
        assert_eq!(target.active_count_48, 0);
        assert_eq!(target.added_base, 0);
        assert_eq!(target.added_count, 1);
        assert_eq!(target.info_5c[0].age_0c, 0.5);
        assert_eq!(target.info_5c[0].lifespan_10, 2.0);
        assert_eq!(target.info_5c[0].velocity_00, [0.0, 0.0, 2.0]);
        assert_eq!(target.positions[0], [-1.0, -2.0, -3.0]);
        assert_eq!(target.colors[0], pending.color);
        assert_eq!(target.radii[0], 1.0);
        assert_eq!(target.sizes_4c[0], 1.0);
        assert_eq!(target.info_5c[0].generation_18, 0);
        target.active_count_48 = 2;
        target.added_count = 0;
        runtime.next = 0;
        runtime.frames = 0;
        runtime.notifications.clear();
        emitter
            .emit(4.0, &[0.0], &mut target, &mut runtime)
            .unwrap();
        assert_eq!(runtime.next, 1);
        assert_eq!(runtime.frames, 0);
        assert!(runtime.notifications.is_empty());
        assert_eq!(target.added_count, 0);

        let sphere_pending = ParticleSphereEmitterPending {
            base: pending.clone(),
            radius: 2.0,
            bytes_consumed: 0,
        };
        let mut sphere = ParticleSphereEmitter::default();
        resolve_particle_sphere_emitter_link(&mut sphere, &sphere_pending, 0).unwrap();
        let mut target = emission_data(1);
        runtime.values = vec![0; 8];
        runtime.next = 0;
        sphere.emit(4.0, &[0.5], &mut target, &mut runtime).unwrap();
        assert_eq!(runtime.next, 8);
        assert_eq!(target.added_count, 1);
        assert_eq!(target.positions[0], [0.0, 0.0, 0.0]);
        let cylinder_pending = ParticleCylinderEmitterPending {
            base: pending,
            radius: 2.0,
            height: 4.0,
            bytes_consumed: 0,
        };
        let mut cylinder = ParticleCylinderEmitter::default();
        resolve_particle_cylinder_emitter_link(&mut cylinder, &cylinder_pending, 0).unwrap();
        let mut target = emission_data(1);
        runtime.next = 0;
        cylinder
            .emit(4.0, &[0.5], &mut target, &mut runtime)
            .unwrap();
        assert_eq!(runtime.next, 8);
        assert_eq!(target.added_count, 1);
        assert_eq!(target.positions[0], [0.0, 0.0, -2.0]);
    }

    /// The C++ graph's raw reference store (13e).
    #[test]
    fn link_stores_the_raw_object() {
        let mut world = World::new();
        let owner = world.spawn(ParticleSphereEmitter::default()).id();
        let object = world.spawn_empty().id();
        let mut table = StreamObjectTable::default();
        table.push("NiNode", 0x0070_BA70, object);
        let mut pending = ParticleSphereEmitterPending::default();
        pending.base.emitter_object_index = 0;
        link_particle_sphere_emitter(&mut world, owner, &pending, &table).unwrap();
        let linked = world.get::<ParticleSphereEmitter>(owner).unwrap();
        assert_eq!(linked.emitter_object_50, AvObjectRawRef(Some(object)));
    }
}
