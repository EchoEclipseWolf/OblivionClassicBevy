//! NiPSysModifier's base payload, and the position, grow/fade, age/death and bound-update modifiers.
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;

use super::particle_data::ParticleData;
use super::particle_spawn::{ParticleSpawnModifier, ParticleSpawnServices};
use super::stream_objects::{
    NULL_REFERENCE, NiClass, StreamObjectTable, check_link_index, read_owned_c_string,
};

/// IDA: N/A the error of this file's `stream_objects::check_link_index` calls (the host's guard before 007124A0 / 00752CB0's reads).
const MODIFIER_TARGET_OUT_OF_RANGE: GameError =
    GameError::unclassified(0, "particle modifier target link out of range");

const MODERN_MODIFIER_VERSION: u32 = 0x0A01_0072;
const TRUNCATED_U32: &str = "truncated particle modifier payload";
const TRUNCATED_U16: &str = "truncated particle generation";

/// IDA: flt_B27520 (0x358637BD), 0072A6B0's minimum center distance.
const MERGE_MINIMUM_DISTANCE: f32 = super::transform::NORMALIZE_ZERO_THRESHOLD;

/// IDA: 0072A6B0 NiSphere_MergeSphere merges a second 16-byte sphere into the first: d2 = (dx*dx + dy*dy) + dz*dz and
/// rd = other.r - r (each stored as a float); a center inside the other sphere (d2 <= rd*rd) keeps this one when
/// rd < 0, else copies the other; otherwise d = sqrt(d2) (__CIsqrt), and only when d exceeds flt_B27520 the center
/// moves to other + delta * ((d - rd) / (d + d)); the radius is ((d + other.r) + r) * 0.5. Every caller runs on the
/// main thread after 0040E865 CreateDevice, whose x87 control word is 24-bit, so each operation rounds to float in
/// this order (disasm-09n.txt; P09-RUN-001 WaterRoot bound). Shared by the scene bound passes (0070A0D0, 0070A190,
/// 0070A360) and the particle bound update. Completion: 100% of the finite branch and operation order.
/// TODO: [ ] Compare the 0072A6B0 NaN branches live (the host rejects them).
pub fn merge_ni_sphere(current: &mut [f32; 4], other: &[f32; 4]) -> Result<(), GameError> {
    let dx = current[0] - other[0];
    let dy = current[1] - other[1];
    let dz = current[2] - other[2];
    let distance_squared = (dx * dx + dy * dy) + dz * dz;
    let radius_difference = other[3] - current[3];
    let radius_difference_squared = radius_difference * radius_difference;
    if !distance_squared.is_finite() || !radius_difference_squared.is_finite() {
        return Err(GameError::unclassified(
            0,
            "particle bound merge outside finite domain",
        ));
    }
    if distance_squared <= radius_difference_squared {
        if radius_difference < 0.0 {
            return Ok(());
        }
        *current = *other;
        return Ok(());
    }
    let distance = distance_squared.sqrt();
    if MERGE_MINIMUM_DISTANCE < distance {
        let factor = (distance - radius_difference) / (distance + distance);
        let x_offset = dx * factor;
        let y_offset = dy * factor;
        let z_offset = dz * factor;
        let x = other[0] + x_offset;
        let y = y_offset + other[1];
        let z = other[2] + z_offset;
        if !factor.is_finite() || !x.is_finite() || !y.is_finite() || !z.is_finite() {
            return Err(GameError::unclassified(
                0,
                "particle bound center outside finite domain",
            ));
        }
        current[0] = x;
        current[1] = y;
        current[2] = z;
    }
    let radius = ((distance + other[3]) + current[3]) * 0.5;
    if !radius.is_finite() {
        return Err(GameError::unclassified(
            0,
            "particle bound radius outside finite domain",
        ));
    }
    current[3] = radius;
    Ok(())
}

/// IDA: 00752DC0 NiPSysModifier's loaded fields: +0x08 name, +0x0C order, +0x14 active.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticleModifierBase {
    pub name_08: Option<Vec<u8>>,
    pub order_0c: u32,
    pub active_14: bool,
}

impl Default for ParticleModifierBase {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            name_08: None,
            order_0c: 0,
            active_14: true,
        }
    }
}

impl ParticleModifierBase {
    /// IDA: 006FAB60 virtual +0x58 stores the active BYTE +0x14 only. Completion: 100% of a canonical bool store.
    /// TODO: [ ] Noncanonical raw bytes.
    pub const fn set_active(&mut self, value: bool) {
        self.active_14 = value;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticleModifierPending {
    pub name: Option<Vec<u8>>,
    pub order: u32,
    pub target_index: u32,
    pub active: bool,
    pub bytes_consumed: usize,
}

impl Default for ParticleModifierPending {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            name: None,
            order: 0,
            target_index: NULL_REFERENCE,
            active: true,
            bytes_consumed: 0,
        }
    }
}

impl ParticleModifierPending {
    /// IDA: 006FAB60 virtual +0x58 stores the active BYTE +0x14 only, for the emitters that keep these fields whole.
    /// Completion: 100% of a canonical bool store.
    /// TODO: [ ] Noncanonical raw bytes.
    pub const fn set_active(&mut self, value: bool) {
        self.active = value;
    }

    /// IDA: N/A the fields a link publishes.
    #[must_use]
    pub fn published(&self) -> ParticleModifierBase {
        ParticleModifierBase {
            name_08: self.name.clone(),
            order_0c: self.order,
            active_14: self.active,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParticleGrowFadePending {
    pub base: ParticleModifierPending,
    pub grow_time: f32,
    pub grow_generation: u16,
    pub fade_time: f32,
    pub fade_generation: u16,
    pub bytes_consumed: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticleAgeDeathPending {
    pub base: ParticleModifierPending,
    pub spawn_on_death: bool,
    pub spawn_index: u32,
    pub bytes_consumed: usize,
}

impl Default for ParticleAgeDeathPending {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            base: ParticleModifierPending::default(),
            spawn_on_death: false,
            spawn_index: NULL_REFERENCE,
            bytes_consumed: 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParticleBoundUpdatePending {
    pub base: ParticleModifierPending,
    pub skip_count: u16,
    pub bytes_consumed: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PositionParticle {
    pub velocity: [f32; 3],
    pub last_update_time: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GrowFadeParticle {
    pub age: f32,
    pub lifespan: f32,
    pub generation: u16,
}

/// IDA: 00755690 creates the 0x18-byte NiPSysPositionModifier via 00752BF0.
/// TODO: [ ] Bind derived finalization and the target's scheduling.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct ParticlePositionModifier {
    pub base: ParticleModifierBase,
}

impl ParticlePositionModifier {
    /// IDA: 007555F0's loop after 007598C0 finalizes the active spans. Completion: 100% of the finite loop over
    /// caller-supplied finalized particles, published together.
    /// TODO: [ ] Use `update_positions` for whole base-data updates.
    pub fn update_position_records(
        &self,
        current_time: f32,
        particles: &mut [PositionParticle],
        positions: &mut [[f32; 3]],
    ) -> Result<(), GameError> {
        if particles.len() != positions.len() {
            return Err(GameError::unclassified(
                0,
                "particle and position counts differ",
            ));
        }
        if !current_time.is_finite() {
            return Err(GameError::unclassified(0, "nonfinite particle update time"));
        }
        let mut prepared_positions = positions.to_vec();
        let mut prepared_times = Vec::with_capacity(particles.len());
        for (index, particle) in particles.iter().enumerate() {
            if !particle.last_update_time.is_finite() {
                return Err(GameError::unclassified(
                    0,
                    "nonfinite particle last-update time",
                ));
            }
            let delta_time = current_time - particle.last_update_time;
            if !delta_time.is_finite() {
                return Err(GameError::unclassified(0, "nonfinite particle time delta"));
            }
            for component in 0..3 {
                let velocity = particle.velocity[component];
                let position = positions[index][component];
                let displacement = velocity * delta_time;
                let result = position + displacement;
                if !velocity.is_finite() || !position.is_finite() || !result.is_finite() {
                    return Err(GameError::unclassified(
                        0,
                        "particle position outside finite domain",
                    ));
                }
                prepared_positions[index][component] = result;
            }
            prepared_times.push(current_time);
        }
        for index in 0..particles.len() {
            positions[index] = prepared_positions[index];
            particles[index].last_update_time = prepared_times[index];
        }
        Ok(())
    }

    /// IDA: 007555F0 invokes 007598C0 before integrating each active 0x1C record. Completion: 100% of the checked
    /// base-data finalization and finite update, published together.
    /// TODO: [ ] Dispatch derived finalizer removals and compare x87 runtime values.
    pub fn update_positions(
        &self,
        current_time: f32,
        data: &mut ParticleData,
    ) -> Result<(), GameError> {
        let mut prepared = data.clone();
        prepared.finalize_added_particles()?;
        let count = usize::from(prepared.active_count_48);
        let mut particles: Vec<PositionParticle> = prepared.info_5c[..count]
            .iter()
            .map(|info| PositionParticle {
                velocity: info.velocity_00,
                last_update_time: info.last_update_time_14,
            })
            .collect();
        self.update_position_records(
            current_time,
            &mut particles,
            &mut prepared.positions[..count],
        )?;
        for (info, particle) in prepared.info_5c[..count].iter_mut().zip(&particles) {
            info.last_update_time_14 = particle.last_update_time;
        }
        *data = prepared;
        Ok(())
    }
}

/// IDA: float bits 0x38D1B717, 00756CA0's minimum size.
const MINIMUM_SIZE: f32 = f32::from_bits(0x38D1_B717);

/// IDA: 00756DA0 creates the 0x28-byte NiPSysGrowFadeModifier via 00752BF0: times and generations zero.
/// TODO: [ ] Bind the target and update scheduling.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ParticleGrowFadeModifier {
    pub base: ParticleModifierBase,
    pub grow_time_18: f32,
    pub grow_generation_1c: u16,
    pub fade_time_20: f32,
    pub fade_generation_24: u16,
}

impl ParticleGrowFadeModifier {
    /// IDA: 00756CA0 compares raw generation words, computes grow and fade fractions, then enforces float bits
    /// 0x38D1B717 as the minimum size. Completion: 100% of the finite domain with staged publication.
    /// TODO: [ ] Compare the original's x87 rounding and particle array ABI.
    pub fn update_size_records(
        &self,
        particles: &[GrowFadeParticle],
        sizes: &mut [f32],
    ) -> Result<(), GameError> {
        if particles.len() != sizes.len() {
            return Err(GameError::unclassified(
                0,
                "particle and size counts differ",
            ));
        }
        if !self.grow_time_18.is_finite() || !self.fade_time_20.is_finite() {
            return Err(GameError::unclassified(
                0,
                "grow/fade times outside finite domain",
            ));
        }
        let mut prepared = Vec::with_capacity(particles.len());
        for particle in particles {
            if !particle.age.is_finite() || !particle.lifespan.is_finite() {
                return Err(GameError::unclassified(
                    0,
                    "particle age/lifespan outside finite domain",
                ));
            }
            let mut growth = 1.0;
            if particle.generation == self.grow_generation_1c
                && self.grow_time_18 > particle.age
                && self.grow_time_18 != 0.0
            {
                growth = particle.age / self.grow_time_18;
            }
            let remaining = particle.lifespan - particle.age;
            if !remaining.is_finite() {
                return Err(GameError::unclassified(
                    0,
                    "particle remaining lifetime outside finite domain",
                ));
            }
            let mut fade = 1.0;
            if particle.generation == self.fade_generation_24
                && self.fade_time_20 > remaining
                && self.fade_time_20 != 0.0
            {
                fade = remaining / self.fade_time_20;
            }
            let mut size = if growth < fade { growth } else { fade };
            if !size.is_finite() {
                return Err(GameError::unclassified(
                    0,
                    "particle grow/fade ratio outside finite domain",
                ));
            }
            if size < MINIMUM_SIZE {
                size = MINIMUM_SIZE;
            }
            prepared.push(size);
        }
        sizes.copy_from_slice(&prepared);
        Ok(())
    }

    /// IDA: 00756CA0 loops over NiPSysData +0x48 active count, reads +0x5C info, then writes +0x4C sizes without
    /// invoking the data finalizer. Completion: 100% of the checked active extent and staged output.
    /// TODO: [ ] Compare the original's x87 results and bind modifier scheduling.
    pub fn update_sizes(&self, data: &mut ParticleData) -> Result<(), GameError> {
        let count = usize::from(data.active_count_48);
        if data.info_5c.len() < count || data.sizes_4c.len() < count {
            return Err(GameError::unclassified(
                0,
                "particle info/size array is too short",
            ));
        }
        let particles: Vec<GrowFadeParticle> = data.info_5c[..count]
            .iter()
            .map(|info| GrowFadeParticle {
                age: info.age_0c,
                lifespan: info.lifespan_10,
                generation: info.generation_18,
            })
            .collect();
        self.update_size_records(&particles, &mut data.sizes_4c[..count])
    }
}

/// IDA: 0075BD90 constructs the 0x20-byte NiPSysAgeDeathModifier: +0x18 false, +0x1C null.
/// TODO: [ ] Bind 0075BCE1's spawn callback with a verified SpawnModifier owner.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct ParticleAgeDeathModifier {
    pub base: ParticleModifierBase,
    pub spawn_on_death_18: bool,
    /// IDA: 0075BDD0 +0x1C's deferred object: whether its index was non-null, kept apart from the typed owner.
    pub spawn_linked: bool,
    pub spawn_1c: Option<Entity>,
}

impl ParticleAgeDeathModifier {
    /// IDA: 0075BC50 traverses active 0x1C records from the last slot, stores age += current - last_update, and calls
    /// data virtual +0x58 when lifespan < age. The optional +0x1C spawn callback precedes removal. In place, as in
    /// C++: an earlier removal stays when a later record fails. Completion: 100% of the finite no-spawn path.
    /// TODO: [ ] Supply the retained spawn owner through `update_ages_with_spawn` for spawn deaths.
    pub fn update_ages(&self, current_time: f32, data: &mut ParticleData) -> Result<(), GameError> {
        if !current_time.is_finite() {
            return Err(GameError::unclassified(0, "nonfinite particle update time"));
        }
        if data.info_5c.len() < usize::from(data.active_count_48) {
            return Err(GameError::unclassified(
                0,
                "particle info array is too short",
            ));
        }
        let mut remaining = usize::from(data.active_count_48);
        while remaining > 0 {
            let index = remaining - 1;
            let info = &mut data.info_5c[index];
            if !info.age_0c.is_finite()
                || !info.lifespan_10.is_finite()
                || !info.last_update_time_14.is_finite()
            {
                return Err(GameError::unclassified(
                    0,
                    "particle age inputs outside finite domain",
                ));
            }
            // 0075BD46/49 hold the subtraction and addition in x87 until the 32-bit store at 0075BD4C; each rounds to a
            // float on the main thread after CreateDevice.
            let next_age = current_time - info.last_update_time_14 + info.age_0c;
            if !next_age.is_finite() {
                return Err(GameError::unclassified(
                    0,
                    "particle age outside finite domain",
                ));
            }
            info.age_0c = next_age;
            if info.lifespan_10 < next_age {
                if self.spawn_on_death_18 && self.spawn_linked {
                    return Err(GameError::unclassified(
                        0,
                        "particle death spawn callback unresolved",
                    ));
                }
                data.remove(index as u16)?;
            }
            remaining -= 1;
        }
        Ok(())
    }

    /// IDA: 0075BC50's +0x18/+0x1C branch; the age stores precede spawn virtual +5C and data removal +58. `spawn` is
    /// the component of `spawn_1c`, which the caller reads from the world. Completion: 100% of the finite ordered
    /// dispatch.
    /// TODO: [ ] Bind target ownership, x87 and child failure behaviour.
    pub fn update_ages_with_spawn(
        &self,
        current_time: f32,
        data: &mut ParticleData,
        spawn: Option<&ParticleSpawnModifier>,
        services: &mut dyn ParticleSpawnServices,
    ) -> Result<(), GameError> {
        if !self.spawn_on_death_18 || !self.spawn_linked {
            return self.update_ages(current_time, data);
        }
        let spawn = spawn.ok_or(GameError::unclassified(
            0,
            "particle death spawn owner unresolved",
        ))?;
        if !current_time.is_finite() || data.info_5c.len() < usize::from(data.active_count_48) {
            return Err(GameError::unclassified(0, "particle age inputs invalid"));
        }
        let mut remaining = usize::from(data.active_count_48);
        while remaining > 0 {
            let index = (remaining - 1) as u16;
            let info = &mut data.info_5c[usize::from(index)];
            if !info.age_0c.is_finite()
                || !info.lifespan_10.is_finite()
                || !info.last_update_time_14.is_finite()
            {
                return Err(GameError::unclassified(
                    0,
                    "particle age inputs outside finite domain",
                ));
            }
            let next_age = current_time - info.last_update_time_14 + info.age_0c;
            if !next_age.is_finite() {
                return Err(GameError::unclassified(
                    0,
                    "particle age outside finite domain",
                ));
            }
            info.age_0c = next_age;
            if info.lifespan_10 < next_age {
                let death_time = info.lifespan_10 + current_time - next_age;
                spawn.dispatch(current_time, death_time, index, data, services)?;
                data.remove(index)?;
            }
            remaining -= 1;
        }
        Ok(())
    }
}

/// IDA: 0075A9E0 constructs the 0x20-byte NiPSysBoundUpdateModifier, clearing +0x18, +0x1A and +0x1C.
/// TODO: [ ] Port 0075AA90's mesh-data branch and verify x87 intermediates.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ParticleBoundUpdateModifier {
    pub base: ParticleModifierBase,
    pub skip_count_18: u16,
    pub rolling_index_1a: u16,
    pub cache_1c: Vec<[f32; 4]>,
}

impl ParticleBoundUpdateModifier {
    /// IDA: 0075A870 stores the raw +0x18 word and replaces +0x1C with a max(signed word, 1)-element zeroed sphere
    /// array, leaving +0x1A alone. Completion: 100% of the allocation and zeroing.
    /// TODO: [ ] Match FormHeap's null-allocation behaviour.
    pub fn set_skip_count(&mut self, raw_count: u16) {
        let signed_count = raw_count as i16;
        let count = if signed_count > 1 { signed_count } else { 1 } as usize;
        self.cache_1c = vec![[0.0; 4]; count];
        self.skip_count_18 = raw_count;
    }

    /// IDA: 0075ACE0's base NiPSysData branch: empty data clears the stored sphere; otherwise +0x1A rotates a sampled
    /// AABB sphere through the +0x1C slots, merges nonzero-radius slots 1..stride-1 via 0072A6B0, then publishes the
    /// bound. `current_time` is used only by the separate mesh branch. Completion: 100% of the finite base branch,
    /// staged.
    /// TODO: [ ] Bind 0075AA90's mesh branch and compare the original's x87 stores.
    pub fn update_bound(
        &mut self,
        _current_time: f32,
        data: &mut ParticleData,
    ) -> Result<(), GameError> {
        let mut prepared = self.clone();
        let mut prepared_data = data.clone();
        let count = usize::from(prepared_data.active_count_48);
        if count == 0 {
            prepared_data.bounds = [0.0; 4];
            prepared.rolling_index_1a = 0;
        } else {
            if prepared.skip_count_18 == 0xFFFF {
                let automatic = (u32::from(prepared_data.vertex_count) / 50 + 1) as u16;
                prepared.set_skip_count(automatic);
            }
            let requested = i32::from(prepared.skip_count_18 as i16);
            let recommended = (count / 50 + 1) as i32;
            let stride = requested.min(recommended).max(1) as usize;
            let rolling = usize::from(prepared.rolling_index_1a);
            if prepared.cache_1c.len() < stride || rolling >= prepared.cache_1c.len() {
                return Err(GameError::unclassified(
                    0,
                    "particle bound cache is not initialized",
                ));
            }
            if prepared_data.positions.len() < count
                || prepared_data.radii.len() < count
                || prepared_data.sizes_4c.len() < count
            {
                return Err(GameError::unclassified(
                    0,
                    "particle bound arrays are too short",
                ));
            }

            let first = prepared_data.positions[0];
            let mut minimum = first;
            let mut maximum = first;
            if first.iter().any(|component| !component.is_finite()) {
                return Err(GameError::unclassified(
                    0,
                    "particle position outside finite domain",
                ));
            }
            for index in (rolling..count).step_by(stride) {
                let radius = prepared_data.sizes_4c[index] * prepared_data.radii[index];
                if !radius.is_finite() {
                    return Err(GameError::unclassified(
                        0,
                        "particle radius outside finite domain",
                    ));
                }
                for axis in 0..3 {
                    let position = prepared_data.positions[index][axis];
                    let low = position - radius;
                    let high = position + radius;
                    if !low.is_finite() || !high.is_finite() {
                        return Err(GameError::unclassified(
                            0,
                            "particle bound extent outside finite domain",
                        ));
                    }
                    if low <= minimum[axis] {
                        minimum[axis] = low;
                    }
                    if high >= maximum[axis] {
                        maximum[axis] = high;
                    }
                }
            }

            for axis in 0..3 {
                let sum = maximum[axis] + minimum[axis];
                prepared.cache_1c[rolling][axis] = sum * 0.5;
            }
            let dx = maximum[0] - minimum[0];
            let dy = maximum[1] - minimum[1];
            let dz = maximum[2] - minimum[2];
            // 0075AFCD stores dy^2 + dx^2 + dz^2 before __CIsqrt, then stores the sqrt before multiplying by one half
            // at 0075AFF2.
            let distance_squared = dy * dy + dx * dx + dz * dz;
            let distance = distance_squared.sqrt();
            prepared.cache_1c[rolling][3] = distance * 0.5;
            if prepared.cache_1c[rolling]
                .iter()
                .any(|component| !component.is_finite())
            {
                return Err(GameError::unclassified(
                    0,
                    "particle bound sphere outside finite domain",
                ));
            }
            for slot in &mut prepared.cache_1c[stride..] {
                *slot = [0.0; 4];
            }

            // The C++ reads the slot back here, so a rolling index past the stride reads the zero just stored.
            let mut combined = prepared.cache_1c[rolling];
            for index in 1..stride {
                if prepared.cache_1c[index][3] != 0.0 {
                    merge_ni_sphere(&mut combined, &prepared.cache_1c[index])?;
                }
            }
            if combined[3] == 0.0 {
                combined[3] = prepared_data.sizes_4c[0] * prepared_data.radii[0];
                if !combined[3].is_finite() {
                    return Err(GameError::unclassified(
                        0,
                        "particle bound fallback radius outside finite domain",
                    ));
                }
            }
            prepared_data.bounds = combined;
            prepared.rolling_index_1a = if rolling + 1 >= stride {
                0
            } else {
                (rolling + 1) as u16
            };
        }
        *data = prepared_data;
        *self = prepared;
        Ok(())
    }
}

/// IDA: N/A the error texts of one caller's 00752DC0 read: the pre-modern stream (`unclassified(0, ..)`), a short u32
/// (the name's length, the order and the target), the name's bytes and the active byte (each `GameError::stream`).
#[derive(Clone, Copy, Debug)]
pub struct ModifierBaseTexts {
    pub legacy: &'static str,
    pub truncated: &'static str,
    pub name: &'static str,
    pub active: &'static str,
}

/// IDA: N/A the texts of the modifiers that share this file's.
pub const MODIFIER_BASE_TEXTS: ModifierBaseTexts = ModifierBaseTexts {
    legacy: "legacy particle modifier payload unsupported",
    truncated: TRUNCATED_U32,
    name: "truncated particle modifier name",
    active: "truncated particle modifier active byte",
};

/// IDA: 00752DC0 reads the owned name, u32 order, target reference and one-byte active flag after the modern NiObject
/// no-op, for >= 0A010072. Completion: 100% of the bounded base payload; the name's raw bytes are kept past any NUL.
/// Each failure has the caller's text (`texts`).
/// TODO: [ ] Earlier NiObject formats and the original's callback errors.
pub fn parse_particle_modifier_payload(
    bytes: &[u8],
    version: u32,
    texts: &ModifierBaseTexts,
) -> Result<ParticleModifierPending, GameError> {
    if version < MODERN_MODIFIER_VERSION {
        return Err(GameError::unclassified(0, texts.legacy));
    }
    let mut reader = ByteReader::new(bytes);
    let name = read_owned_c_string(&mut reader, texts.truncated, texts.name)?;
    let order = reader.u32(texts.truncated)?;
    let target_index = reader.u32(texts.truncated)?;
    let active = reader.u8(texts.active)? != 0;
    Ok(ParticleModifierPending {
        name,
        order,
        target_index,
        active,
        bytes_consumed: reader.position(),
    })
}

/// IDA: N/A a reader over a derived payload, placed after its base.
pub(super) fn reader_after<'a>(bytes: &'a [u8], consumed: usize) -> ByteReader<'a> {
    let mut reader = ByteReader::new(bytes);
    reader
        .take(consumed, TRUNCATED_U32)
        .expect("the base payload was read from these bytes");
    reader
}

/// IDA: 007556C0 tail thunk to 00752DC0. Completion: 100%.
/// TODO: [ ] Bind graph dispatch and the target's type.
pub fn parse_particle_position_payload(
    bytes: &[u8],
    version: u32,
) -> Result<ParticleModifierPending, GameError> {
    parse_particle_modifier_payload(bytes, version, &MODIFIER_BASE_TEXTS)
}

/// IDA: 00756EB0 the base, then grow float and u16 generation, fade float and u16 generation. Completion: 100% of the
/// bounded derived payload.
/// TODO: [ ] Bind the original's NiStream callback errors.
pub fn parse_particle_grow_fade_payload(
    bytes: &[u8],
    version: u32,
) -> Result<ParticleGrowFadePending, GameError> {
    let base = parse_particle_modifier_payload(bytes, version, &MODIFIER_BASE_TEXTS)?;
    let mut reader = reader_after(bytes, base.bytes_consumed);
    let grow_time = reader.f32(TRUNCATED_U32)?;
    let grow_generation = reader.u16(TRUNCATED_U16)?;
    let fade_time = reader.f32(TRUNCATED_U32)?;
    let fade_generation = reader.u16(TRUNCATED_U16)?;
    Ok(ParticleGrowFadePending {
        base,
        grow_time,
        grow_generation,
        fade_time,
        fade_generation,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 0075BF00 the base, a byte flag at +0x18 and a deferred reference at +0x1C. Completion: 100% of the bounded
/// modern payload; the spawn type is resolved later.
/// TODO: [ ] Bind the original's NiStream callback errors.
pub fn parse_particle_age_death_payload(
    bytes: &[u8],
    version: u32,
) -> Result<ParticleAgeDeathPending, GameError> {
    let base = parse_particle_modifier_payload(bytes, version, &MODIFIER_BASE_TEXTS)?;
    let mut reader = reader_after(bytes, base.bytes_consumed);
    let spawn_on_death = reader.u8("truncated particle age/death spawn flag")? != 0;
    let spawn_index = reader.u32(TRUNCATED_U32)?;
    Ok(ParticleAgeDeathPending {
        base,
        spawn_on_death,
        spawn_index,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 0075B100 the base and one u16 skip count; both version sides of 0A010064 call 0075A870 with the word.
/// Completion: 100% of the bounded modern read.
/// TODO: [ ] Pre-0A010072 modifier base formats.
pub fn parse_particle_bound_update_payload(
    bytes: &[u8],
    version: u32,
) -> Result<ParticleBoundUpdatePending, GameError> {
    let base = parse_particle_modifier_payload(bytes, version, &MODIFIER_BASE_TEXTS)?;
    let mut reader = reader_after(bytes, base.bytes_consumed);
    let skip_count = reader.u16(TRUNCATED_U16)?;
    Ok(ParticleBoundUpdatePending {
        base,
        skip_count,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 00752CB0 consumes and discards the deferred target. Completion: 100% of the bounds-only resolution and base
/// publication.
/// TODO: [ ] Bind the concrete target when the particle-system owner is ported.
pub fn resolve_particle_position_link(
    target: &mut ParticlePositionModifier,
    pending: &ParticleModifierPending,
    object_count: usize,
) -> Result<(), GameError> {
    check_link_index(
        pending.target_index,
        object_count,
        MODIFIER_TARGET_OUT_OF_RANGE,
    )?;
    target.base = pending.published();
    Ok(())
}

/// IDA: 00752CB0 after 00756EB0's load. Completion: 100% of the bounds-only resolution and scalar publication.
/// TODO: [ ] Bind the concrete target when the particle-system owner is ported.
pub fn resolve_particle_grow_fade_link(
    target: &mut ParticleGrowFadeModifier,
    pending: &ParticleGrowFadePending,
    object_count: usize,
) -> Result<(), GameError> {
    check_link_index(
        pending.base.target_index,
        object_count,
        MODIFIER_TARGET_OUT_OF_RANGE,
    )?;
    target.base = pending.base.published();
    target.grow_time_18 = pending.grow_time;
    target.grow_generation_1c = pending.grow_generation;
    target.fade_time_20 = pending.fade_time;
    target.fade_generation_24 = pending.fade_generation;
    Ok(())
}

/// IDA: 0075BDD0 resolves the base target and the next deferred spawn reference. Completion: 100% of the checked
/// indices and publication.
pub fn resolve_particle_age_death_link(
    target: &mut ParticleAgeDeathModifier,
    pending: &ParticleAgeDeathPending,
    object_count: usize,
    spawn: Option<Entity>,
) -> Result<(), GameError> {
    check_link_index(
        pending.base.target_index,
        object_count,
        MODIFIER_TARGET_OUT_OF_RANGE,
    )?;
    check_link_index(
        pending.spawn_index,
        object_count,
        MODIFIER_TARGET_OUT_OF_RANGE,
    )?;
    if pending.spawn_index == NULL_REFERENCE && spawn.is_some() {
        return Err(GameError::unclassified(0, "null spawn index has an owner"));
    }
    target.base = pending.base.published();
    target.spawn_on_death_18 = pending.spawn_on_death;
    target.spawn_linked = pending.spawn_index != NULL_REFERENCE;
    target.spawn_1c = spawn;
    Ok(())
}

/// IDA: 00752CB0 validates the deferred base target; 0075B100 / 0075A870 keep the u16 skip setting and allocate its
/// zeroed sphere cache. Completion: 100% of the checked index and publication.
/// TODO: [ ] Bind the target owner and FormHeap allocation failures.
pub fn resolve_particle_bound_update_link(
    target: &mut ParticleBoundUpdateModifier,
    pending: &ParticleBoundUpdatePending,
    object_count: usize,
) -> Result<(), GameError> {
    check_link_index(
        pending.base.target_index,
        object_count,
        MODIFIER_TARGET_OUT_OF_RANGE,
    )?;
    target.set_skip_count(pending.skip_count);
    target.base = pending.base.published();
    Ok(())
}

const MISSING_MODIFIER: &str = "particle modifier component missing";

/// IDA: N/A `resolve_particle_position_link` on a stream object.
pub fn link_particle_position(
    world: &mut World,
    entity: Entity,
    pending: &ParticleModifierPending,
    table: &StreamObjectTable,
) -> Result<(), GameError> {
    let mut target = world
        .get_mut::<ParticlePositionModifier>(entity)
        .ok_or(GameError::from_legacy_message(MISSING_MODIFIER))?;
    resolve_particle_position_link(&mut target, pending, table.objects.len())
}

/// IDA: N/A `resolve_particle_grow_fade_link` on a stream object.
pub fn link_particle_grow_fade(
    world: &mut World,
    entity: Entity,
    pending: &ParticleGrowFadePending,
    table: &StreamObjectTable,
) -> Result<(), GameError> {
    let mut target = world
        .get_mut::<ParticleGrowFadeModifier>(entity)
        .ok_or(GameError::from_legacy_message(MISSING_MODIFIER))?;
    resolve_particle_grow_fade_link(&mut target, pending, table.objects.len())
}

/// IDA: 0075BDD0 on a stream object; the C++ graph checks the spawn link's class first. Completion: 100%.
pub fn link_particle_age_death(
    world: &mut World,
    entity: Entity,
    pending: &ParticleAgeDeathPending,
    table: &StreamObjectTable,
) -> Result<(), GameError> {
    let mut spawn = None;
    if pending.spawn_index != NULL_REFERENCE {
        let entry =
            table
                .objects
                .get(pending.spawn_index as usize)
                .ok_or(GameError::unclassified(
                    0,
                    "particle spawn link out of range",
                ))?;
        if world.get::<NiClass>(entry.entity) != Some(&NiClass::ParticleSpawnModifier) {
            return Err(GameError::unclassified(
                0,
                "particle spawn link target is not NiPSysSpawnModifier",
            ));
        }
        spawn = Some(entry.entity);
    }
    let mut target = world
        .get_mut::<ParticleAgeDeathModifier>(entity)
        .ok_or(GameError::from_legacy_message(MISSING_MODIFIER))?;
    resolve_particle_age_death_link(&mut target, pending, table.objects.len(), spawn)
}

/// IDA: N/A `resolve_particle_bound_update_link` on a stream object.
pub fn link_particle_bound_update(
    world: &mut World,
    entity: Entity,
    pending: &ParticleBoundUpdatePending,
    table: &StreamObjectTable,
) -> Result<(), GameError> {
    let mut target = world
        .get_mut::<ParticleBoundUpdateModifier>(entity)
        .ok_or(GameError::from_legacy_message(MISSING_MODIFIER))?;
    resolve_particle_bound_update_link(&mut target, pending, table.objects.len())
}

#[cfg(test)]
mod tests {
    use super::super::particle_data::ParticleInfo;
    use super::*;

    fn info(age: f32, lifespan: f32, generation: u16) -> ParticleInfo {
        ParticleInfo {
            age_0c: age,
            lifespan_10: lifespan,
            generation_18: generation,
            ..ParticleInfo::default()
        }
    }

    fn position_fixture() -> ParticleData {
        let mut data = ParticleData::default();
        for index in 0..6u16 {
            let value = f32::from(index);
            data.positions.push([value, value + 10.0, value + 20.0]);
            data.info_5c.push(ParticleInfo {
                velocity_00: [value + 1.0, 0.0, 0.0],
                age_0c: value,
                lifespan_10: 10.0,
                last_update_time_14: value,
                generation_18: index,
                unresolved_flags_1a: 0,
            });
        }
        data.active_count_48 = 2;
        data.added_base = 4;
        data.added_count = 2;
        data
    }

    #[test]
    fn position_update_finalizes_then_integrates() {
        let modifier = ParticlePositionModifier::default();
        let mut data = position_fixture();
        modifier.update_positions(8.0, &mut data).unwrap();
        assert_eq!(data.active_count_48, 4);
        assert_eq!(data.added_count, 0);
        assert_eq!(data.added_base, 4);
        assert_eq!(data.positions[2][0], 23.0);
        assert_eq!(data.positions[3][0], 24.0);
        assert_eq!(data.info_5c[2].generation_18, 5);
        assert_eq!(data.info_5c[3].generation_18, 4);
        assert_eq!(data.info_5c[2].last_update_time_14, 8.0);
        assert_eq!(data.info_5c[3].last_update_time_14, 8.0);
        assert_eq!(data.positions[4][0], 4.0);
        assert_eq!(data.info_5c[4].last_update_time_14, 4.0);

        let mut invalid = position_fixture();
        invalid.info_5c[5].velocity_00[1] = f32::INFINITY;
        let before = invalid.clone();
        assert!(modifier.update_positions(8.0, &mut invalid).is_err());
        assert_eq!(invalid, before, "an invalid update publishes nothing");
    }

    #[test]
    fn grow_fade_uses_the_active_prefix() {
        let mut modifier = ParticleGrowFadeModifier::default();
        let pending = ParticleGrowFadePending {
            grow_time: 2.0,
            fade_time: 2.0,
            grow_generation: 3,
            fade_generation: 3,
            ..ParticleGrowFadePending::default()
        };
        resolve_particle_grow_fade_link(&mut modifier, &pending, 0).unwrap();
        let mut data = ParticleData {
            info_5c: vec![info(0.5, 4.0, 3), info(3.0, 4.0, 3), info(1.0, 4.0, 3)],
            sizes_4c: vec![9.0, 9.0, 9.0],
            active_count_48: 2,
            added_base: 2,
            added_count: 1,
            ..ParticleData::default()
        };
        modifier.update_sizes(&mut data).unwrap();
        assert_eq!(data.sizes_4c, [0.25, 0.5, 9.0]);
        assert_eq!(data.active_count_48, 2);
        assert_eq!(data.added_count, 1);

        data.sizes_4c.truncate(1);
        let unchanged = data.sizes_4c.clone();
        assert!(modifier.update_sizes(&mut data).is_err());
        assert_eq!(data.sizes_4c, unchanged);
        data.sizes_4c = vec![7.0, 8.0];
        data.info_5c[1].age_0c = f32::NAN;
        assert!(modifier.update_sizes(&mut data).is_err());
        assert_eq!(data.sizes_4c, [7.0, 8.0]);
    }

    #[test]
    fn age_death_runs_backward_with_strict_lifespan() {
        let mut modifier = ParticleAgeDeathModifier::default();
        let mut data = ParticleData {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [3.0, 0.0, 0.0],
            ],
            info_5c: vec![
                info(0.25, 1.0, 0),
                info(1.0, 1.0, 1),
                info(0.0, 1.0, 2),
                info(1.5, 1.0, 3),
            ],
            active_count_48: 4,
            ..ParticleData::default()
        };
        modifier.update_ages(0.0, &mut data).unwrap();
        assert_eq!(data.active_count_48, 3);
        assert_eq!(data.info_5c[1].generation_18, 1);
        assert_eq!(data.info_5c[1].age_0c, 1.0);
        assert_eq!(data.info_5c[2].generation_18, 2);
        assert_eq!(data.positions[3][0], 3.0);
        modifier.update_ages(0.5, &mut data).unwrap();
        assert_eq!(data.active_count_48, 2);
        assert_eq!(data.info_5c[0].generation_18, 0);
        assert_eq!(data.info_5c[0].age_0c, 0.75);
        assert_eq!(data.info_5c[1].generation_18, 2);
        assert_eq!(data.info_5c[1].age_0c, 0.5);
        assert_eq!(data.info_5c[1].last_update_time_14, 0.0);

        let mut malformed = data.clone();
        malformed.info_5c[0].age_0c = f32::NAN;
        let before = malformed.positions.clone();
        let count = malformed.active_count_48;
        assert!(modifier.update_ages(0.75, &mut malformed).is_err());
        assert_eq!(malformed.positions, before);
        assert_eq!(malformed.active_count_48, count - 1);
        assert_eq!(malformed.info_5c[1].age_0c, 1.25);

        let linked = ParticleAgeDeathPending {
            spawn_on_death: true,
            spawn_index: 0,
            ..ParticleAgeDeathPending::default()
        };
        resolve_particle_age_death_link(&mut modifier, &linked, 1, None).unwrap();
        let mut survivor = data.clone();
        modifier.update_ages(0.0, &mut survivor).unwrap();
        assert_eq!(survivor.active_count_48, data.active_count_48);
        // C++ throws runtime_error here, not invalid_argument: the message tells them apart.
        assert_eq!(
            modifier.update_ages(2.0, &mut survivor),
            Err(GameError::unclassified(
                0,
                "particle death spawn callback unresolved"
            ))
        );
        assert_eq!(survivor.active_count_48, data.active_count_48);
        assert_eq!(survivor.info_5c[0].age_0c, data.info_5c[0].age_0c);
    }

    #[test]
    fn age_death_payload_widths() {
        let mut bytes = Vec::new();
        bytes.extend(0u32.to_le_bytes()); // null name
        bytes.extend(7u32.to_le_bytes()); // order
        bytes.extend(u32::MAX.to_le_bytes()); // null target
        bytes.push(0); // inactive
        bytes.push(0x80); // nonzero spawn flag
        bytes.extend(0u32.to_le_bytes()); // forward spawn owner
        let payload_size = bytes.len();
        bytes.push(0xAA);
        let pending = parse_particle_age_death_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(pending.bytes_consumed, payload_size);
        assert_eq!(pending.base.order, 7);
        assert!(!pending.base.active);
        assert!(pending.spawn_on_death);
        assert_eq!(pending.spawn_index, 0);
        for length in 0..payload_size {
            assert!(parse_particle_age_death_payload(&bytes[..length], 0x1400_0004).is_err());
        }
    }

    #[test]
    fn bound_payload_and_raw_skip_word() {
        let mut bytes = Vec::new();
        bytes.extend(0u32.to_le_bytes()); // null name
        bytes.extend(6u32.to_le_bytes()); // order
        bytes.extend(u32::MAX.to_le_bytes()); // null target
        bytes.push(0); // inactive
        bytes.extend([0xFF, 0xFF]); // raw auto skip
        let size = bytes.len();
        bytes.push(0xAB);
        let pending = parse_particle_bound_update_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(pending.bytes_consumed, size);
        assert_eq!(pending.skip_count, 0xFFFF);
        assert_eq!(pending.base.order, 6);
        assert!(!pending.base.active);
        let mut modifier = ParticleBoundUpdateModifier::default();
        resolve_particle_bound_update_link(&mut modifier, &pending, 0).unwrap();
        assert_eq!(modifier.skip_count_18, 0xFFFF);
        assert_eq!(modifier.cache_1c.len(), 1);
        assert_eq!(modifier.base.order_0c, 6);
        assert!(!modifier.base.active_14);
        modifier.set_skip_count(3);
        assert_eq!(modifier.cache_1c.len(), 3);
        assert_eq!(modifier.skip_count_18, 3);
        for length in 0..size {
            assert!(parse_particle_bound_update_payload(&bytes[..length], 0x1400_0004).is_err());
        }
    }

    #[test]
    fn bound_update_rolls_through_its_cache() {
        let mut modifier = ParticleBoundUpdateModifier::default();
        modifier.set_skip_count(1);
        let mut empty = ParticleData {
            bounds: [9.0; 4],
            ..ParticleData::default()
        };
        modifier.update_bound(0.0, &mut empty).unwrap();
        assert_eq!(empty.bounds, [0.0; 4]);
        assert_eq!(modifier.rolling_index_1a, 0);
        let mut data = ParticleData {
            vertex_count: 2,
            active_count_48: 2,
            positions: vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            radii: vec![1.0, 1.0],
            sizes_4c: vec![1.0, 1.0],
            ..ParticleData::default()
        };
        modifier.update_bound(0.0, &mut data).unwrap();
        assert_eq!(data.bounds[0], 1.0);
        assert_eq!(data.bounds[1], 0.0);
        assert_eq!(data.bounds[2], 0.0);
        assert!((data.bounds[3] - 6.0f32.sqrt()).abs() < 0.00001);
        assert_eq!(modifier.rolling_index_1a, 0);

        let mut rolling = ParticleBoundUpdateModifier::default();
        rolling.set_skip_count(2);
        let mut many = ParticleData {
            vertex_count: 60,
            active_count_48: 60,
            ..ParticleData::default()
        };
        for index in 0..60u16 {
            many.positions.push([f32::from(index), 0.0, 0.0]);
            many.radii.push(0.0);
            many.sizes_4c.push(1.0);
        }
        rolling.update_bound(0.0, &mut many).unwrap();
        assert_eq!(rolling.rolling_index_1a, 1);
        assert_eq!(rolling.cache_1c.len(), 2);
        assert_eq!(many.bounds[0], 29.0);
        assert_eq!(many.bounds[3], 29.0);
        rolling.update_bound(0.0, &mut many).unwrap();
        assert_eq!(rolling.rolling_index_1a, 0);
        assert!(many.bounds[3] >= 29.0);
        let old_bound = many.bounds;
        many.radii.truncate(1);
        assert!(rolling.update_bound(0.0, &mut many).is_err());
        assert_eq!(many.bounds, old_bound);
        assert_eq!(rolling.rolling_index_1a, 0);
    }

    /// The C++ graph's spawn link check, on tags.
    #[test]
    fn age_death_link_checks_the_spawn_class() {
        let mut world = World::new();
        let owner = world
            .spawn((
                NiClass::ParticleAgeDeathModifier,
                ParticleAgeDeathModifier::default(),
            ))
            .id();
        let spawn = world.spawn(NiClass::ParticleSpawnModifier).id();
        let other = world.spawn(NiClass::Node).id();
        let mut table = StreamObjectTable::default();
        table.push("NiPSysSpawnModifier", 0x0075_1D10, spawn);
        table.push("NiNode", 0x0070_BA70, other);
        let mut pending = ParticleAgeDeathPending {
            spawn_on_death: true,
            spawn_index: 1,
            ..ParticleAgeDeathPending::default()
        };
        assert!(link_particle_age_death(&mut world, owner, &pending, &table).is_err());
        assert_eq!(
            world.get::<ParticleAgeDeathModifier>(owner),
            Some(&ParticleAgeDeathModifier::default())
        );
        pending.spawn_index = 0;
        link_particle_age_death(&mut world, owner, &pending, &table).unwrap();
        let linked = world.get::<ParticleAgeDeathModifier>(owner).unwrap();
        assert_eq!(linked.spawn_1c, Some(spawn));
        assert!(linked.spawn_linked);
    }
}
