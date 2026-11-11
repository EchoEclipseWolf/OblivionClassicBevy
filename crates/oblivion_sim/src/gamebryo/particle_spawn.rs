//! NiPSysSpawnModifier, and the child creation path it dispatches to.
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.

#![allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "00751DF0 and its probability comparison test with these negated comparisons, so a NaN takes the same branch"
)]

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::particle_data::ParticleData;
use super::particle_dynamics::{
    MODIFIER_BASE_TEXTS, ParticleModifierBase, ParticleModifierPending,
    parse_particle_modifier_payload, reader_after,
};
use super::stream_objects::{StreamObjectTable, check_link_index};
use super::transform::{
    NiTransform, invert_scene_transform_checked, rotate_vector, transform_point,
};

const TRUNCATED: &str = "truncated particle spawn payload";

#[derive(Clone, Debug, PartialEq)]
pub struct ParticleSpawnPending {
    pub base: ParticleModifierPending,
    pub generation_limit: u16,
    pub spawn_probability: f32,
    pub minimum_children: u16,
    pub maximum_children: u16,
    pub speed_chaos: f32,
    pub direction_chaos: f32,
    pub lifespan: f32,
    pub lifespan_variation: f32,
    pub bytes_consumed: usize,
}

impl Default for ParticleSpawnPending {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            base: ParticleModifierPending::default(),
            generation_limit: 1,
            spawn_probability: 1.0,
            minimum_children: 1,
            maximum_children: 1,
            speed_chaos: 0.0,
            direction_chaos: 0.0,
            lifespan: 0.0,
            lifespan_variation: 0.0,
            bytes_consumed: 0,
        }
    }
}

/// IDA: N/A host services replacing the CRT's rand and the virtual +0x60 child insertion that 00751DF0 calls.
/// `source` is mutable because a same-system child is stored in the parent's own data, where the C++ port aliases the
/// two; the dispatcher itself changes nothing in it. Completion: 100% of the explicit ordered boundary.
/// TODO: [ ] Bind adapter target and listener owners and the CRT's RNG state.
pub trait ParticleSpawnServices {
    /// IDA: 009859DD rand() returns 0..32767; one draw.
    fn draw_rand15(&mut self) -> u16;
    /// IDA: 00751DF0 invokes virtual +0x60 once per requested child, with the dispatcher's arguments.
    fn spawn_child(
        &mut self,
        current_time: f32,
        death_time: f32,
        parent_index: u16,
        source: &mut ParticleData,
    ) -> Result<(), GameError>;
}

/// IDA: 00751D10 allocates the 0x34-byte NiPSysSpawnModifier: +0x18=1, +0x1C=1, +0x20/+0x22=1, +0x24/+0x28/+0x2C/+0x30=0.
/// TODO: [ ] Bind the target ParticleSystem and listener chain.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ParticleSpawnModifier {
    pub base: ParticleModifierBase,
    pub generation_limit_18: u16,
    pub spawn_probability_1c: f32,
    pub minimum_children_20: u16,
    pub maximum_children_22: u16,
    pub speed_chaos_24: f32,
    pub direction_chaos_28: f32,
    pub lifespan_2c: f32,
    pub lifespan_variation_30: f32,
}

impl Default for ParticleSpawnModifier {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            base: ParticleModifierBase::default(),
            generation_limit_18: 1,
            spawn_probability_1c: 1.0,
            minimum_children_20: 1,
            maximum_children_22: 1,
            speed_chaos_24: 0.0,
            direction_chaos_28: 0.0,
            lifespan_2c: 0.0,
            lifespan_variation_30: 0.0,
        }
    }
}

/// IDA: 00751DF0 divides each CRT rand result by double 32767.0 and stores the quotient to a 32-bit float before
/// comparing or using it. Completion: 100% of the checked domain and float store.
/// TODO: [ ] Bind the CRT's RNG state.
fn draw_fraction(value: u16) -> Result<f32, GameError> {
    if value > 32767 {
        return Err(GameError::unclassified(
            0,
            "particle spawn random value outside rand15 domain",
        ));
    }
    Ok((f64::from(value) / 32767.0) as f32)
}

impl ParticleSpawnModifier {
    /// IDA: 00751DF0 reads generation +0x18 before any random call, draws one probability sample, then one child-count
    /// sample only if accepted. It truncates toward zero, takes fmod(sample, 1), increments only when the fraction
    /// exceeds 0.5, wraps the u16 total, replaces zero with one, and calls virtual +0x60 per child. Completion: 100% of
    /// the bounded dispatcher for an explicit ordered RNG and child service.
    /// TODO: [ ] Bind the CRT's state and a linked child adapter owner.
    pub fn dispatch(
        &self,
        current_time: f32,
        death_time: f32,
        parent_index: u16,
        source: &mut ParticleData,
        services: &mut dyn ParticleSpawnServices,
    ) -> Result<u16, GameError> {
        if parent_index >= source.active_count_48
            || source.info_5c.len() < usize::from(source.active_count_48)
        {
            return Err(GameError::unclassified(
                0,
                "particle spawn parent index outside active data",
            ));
        }
        if source.info_5c[usize::from(parent_index)].generation_18 >= self.generation_limit_18 {
            return Ok(0);
        }
        let chance = draw_fraction(services.draw_rand15())?;
        if !(self.spawn_probability_1c >= chance) {
            return Ok(0);
        }
        let random_count = draw_fraction(services.draw_rand15())?;
        let range = i32::from(self.maximum_children_22) - i32::from(self.minimum_children_20);
        let sampled = (f64::from(range) * f64::from(random_count)) as f32;
        let truncated = x87::ftol(sampled.trunc());
        let fraction = sampled % 1.0;
        let rounded = truncated.wrapping_add(i32::from(fraction > 0.5));
        let mut children = i32::from(self.minimum_children_20).wrapping_add(rounded) as u16;
        if children == 0 {
            children = 1;
        }
        for _ in 0..children {
            services.spawn_child(current_time, death_time, parent_index, source)?;
        }
        Ok(children)
    }
}

/// IDA: 00751F10's overridable steps. The C++ port's protected virtuals and its RNG.
pub trait ParticleSpawnChildHooks {
    /// IDA: 009859DD rand() returns 0..32767; one draw.
    fn draw_rand15(&mut self) -> u16;

    /// IDA: 00751F10 samples speed, polar and azimuth rand values in order, then builds the child's velocity.
    fn resolve_child_velocity(
        &mut self,
        parent_velocity: &[f32; 3],
        speed: f32,
        polar_random: f32,
        azimuth_random: f32,
        direction_chaos: f32,
    ) -> [f32; 3] {
        default_child_velocity(
            parent_velocity,
            speed,
            polar_random,
            azimuth_random,
            direction_chaos,
        )
    }

    /// IDA: 00749510 receives the target system and child index after all stores.
    /// TODO: [ ] Bind the target's modifier listener chain and virtual +0x50 calls.
    fn notify_child_added(&mut self, child_index: u16, target: &ParticleData);
}

type SpawnMatrix = [f32; 9];
const IDENTITY_MATRIX: SpawnMatrix = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// IDA: 00710190 scales the nine row-major matrix elements, storing each as f32. Completion: 100% of the element and
/// store order.
fn scale_spawn_matrix(matrix: &SpawnMatrix, scale: f32) -> SpawnMatrix {
    matrix.map(|value| value * scale)
}

/// IDA: 0070FFC0 / 00710030 copy the left matrix then add or subtract each corresponding f32; the ordered operands
/// determine the subtraction's direction. Completion: 100% of the element and store order.
fn combine_spawn_matrices(left: &SpawnMatrix, right: &SpawnMatrix, subtract: bool) -> SpawnMatrix {
    let mut result = *left;
    for index in 0..result.len() {
        result[index] = if subtract {
            result[index] - right[index]
        } else {
            result[index] + right[index]
        };
    }
    result
}

/// IDA: 00751FC0-00752565 turns three ordered random samples into a unit direction, builds a velocity skew matrix,
/// selects + or - matrix composition using the signed 0x38D1B717 threshold, then scales and rotates the direction. The
/// C++ `std::sin(float)` overloads are f32 here too; 00A09B90 / 00A09BD0 initialize pi and tau from atan(1).
/// Completion: 100% of the finite branch, operator and store topology.
/// TODO: [ ] Compare __CIsin / __CIcos bits against the runtime.
#[must_use]
pub fn default_child_velocity(
    parent_velocity: &[f32; 3],
    speed: f32,
    polar_random: f32,
    azimuth_random: f32,
    direction_chaos: f32,
) -> [f32; 3] {
    const ENGINE_PI: f32 = f32::from_bits(0x4049_0FDB);
    const ENGINE_TWO_PI: f32 = f32::from_bits(0x40C9_0FDB);
    const PARALLEL_THRESHOLD: f32 = f32::from_bits(0x38D1_B717);
    let polar = direction_chaos * polar_random * ENGINE_PI;
    let azimuth = azimuth_random * ENGINE_TWO_PI;
    let sin_polar = f64::from(polar).sin() as f32;
    let cos_azimuth = f64::from(azimuth).cos() as f32;
    let direction_x = cos_azimuth * sin_polar;
    let sin_azimuth = f64::from(azimuth).sin() as f32;
    let direction_y = sin_azimuth * sin_polar;
    let direction_z = f64::from(polar).cos() as f32;
    let direction = [direction_x, direction_y, direction_z];
    let [vx, vy, vz] = *parent_velocity;
    let skew: SpawnMatrix = [0.0, 0.0, -vx, 0.0, 0.0, -vy, vx, vy, 0.0];
    let cross_x = direction_y * vz - direction_z * vy;
    let cross_y = direction_z * vx - direction_x * vz;
    let cross_z = vy * direction_x - direction_y * vx;
    let signed_side = cross_x * vy + cross_y * -vx + cross_z * 0.0;
    let mut orientation = IDENTITY_MATRIX;
    if signed_side > PARALLEL_THRESHOLD || signed_side < -PARALLEL_THRESHOLD {
        // 00752363 / 00752415 store both reciprocal and coefficient to f32.
        let magnitude_squared = vy * vy + vx * vx + vz * vz;
        let magnitude = magnitude_squared.sqrt();
        let reciprocal = 1.0 / magnitude;
        let coefficient = (magnitude - vz) / (vy * vy + vx * vx);
        let scaled_skew = scale_spawn_matrix(&skew, coefficient);
        let negative = signed_side < -PARALLEL_THRESHOLD;
        let first = combine_spawn_matrices(&IDENTITY_MATRIX, &scaled_skew, negative);
        // 007100A0 NiMatrix33_Multiply(skew, first).
        let product = super::transform::multiply(&skew, &first);
        let scaled_product = scale_spawn_matrix(&product, reciprocal);
        orientation = combine_spawn_matrices(&IDENTITY_MATRIX, &scaled_product, negative);
    } else if vz < 0.0 {
        orientation = [-1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, -1.0];
    }
    rotate_vector(&scale_spawn_matrix(&orientation, speed), &direction)
}

// IDA: N/A a helper, with no address of its own.
fn too_short<T>(values: &[T], required: usize) -> bool {
    !values.is_empty() && values.len() < required
}

/// IDA: 00751F10's cross-system branch: the target data and both +0x64 world transforms.
/// TODO: [ ] Resolve the transforms from linked ParticleSystems.
struct CrossSystem<'a> {
    target: &'a mut ParticleData,
    source_world: NiTransform,
    target_world: NiTransform,
}

/// IDA: 00751F10 reserves a target slot through 0074ED40 before four child rand calls, then stores the child and
/// notifies 00749510. The hooks stand in for the C++ port's subclass. Same-system children are stored in the `source`
/// that `spawn_child` receives; `source_owner` is only compared, never read.
/// TODO: [ ] Bind concrete NiAVObject world owners and target listeners.
pub struct ParticleSpawnChildAdapter<'a, H> {
    modifier: &'a ParticleSpawnModifier,
    source_owner: *const ParticleData,
    cross: Option<CrossSystem<'a>>,
    pub hooks: H,
}

impl<'a, H: ParticleSpawnChildHooks> ParticleSpawnChildAdapter<'a, H> {
    /// IDA: 00751F10 reads the modifier's +0x10 target data: the same-system branch. Completion: 100%.
    /// TODO: [ ] Retain the target ParticleSystem's identity.
    pub fn same_system(
        modifier: &'a ParticleSpawnModifier,
        target: &ParticleData,
        hooks: H,
    ) -> Self {
        Self {
            modifier,
            source_owner: target,
            cross: None,
            hooks,
        }
    }

    /// IDA: 00751F10 transforms the parent's position via 0053D4B0 and velocity via 007101F0 through source world then
    /// inverse target world before 0074ED40. Completion: 100% of the explicit frame and data association.
    /// TODO: [ ] Resolve the +0x64 world transforms from linked ParticleSystems.
    pub fn cross_system(
        modifier: &'a ParticleSpawnModifier,
        source: &ParticleData,
        target: &'a mut ParticleData,
        source_world: NiTransform,
        target_world: NiTransform,
        hooks: H,
    ) -> Self {
        Self {
            modifier,
            source_owner: source,
            cross: Some(CrossSystem {
                target,
                source_world,
                target_world,
            }),
            hooks,
        }
    }

    /// IDA: N/A the cross-system target, for inspection.
    #[must_use]
    pub fn cross_target(&self) -> Option<&ParticleData> {
        self.cross.as_ref().map(|cross| &*cross.target)
    }
}

/// IDA: N/A the parent's arrays: the separate source, or the target itself in the same-system branch. Read at each
/// store, as the C++ aliasing does.
fn parent_data<'x>(source: Option<&'x ParticleData>, target: &'x ParticleData) -> &'x ParticleData {
    source.unwrap_or(target)
}

/// IDA: 00751F10 from the slot reservation on: stores age = current - death_time, draws speed, polar and azimuth, then
/// lifespan, copies the optional arrays, stores last-update, and calls 00749510. `source` is `None` for the same-system
/// branch. Completion: 100% of the checked storage and order; the velocity matrix is the hook's.
/// TODO: [ ] Bind original NiAVObject frame owners and the target's listener chain.
#[allow(
    clippy::too_many_arguments,
    reason = "00751F10's dispatcher arguments, the prepared parent and its target"
)]
fn store_child<H: ParticleSpawnChildHooks>(
    modifier: &ParticleSpawnModifier,
    hooks: &mut H,
    current_time: f32,
    death_time: f32,
    parent_index: usize,
    parent_position: [f32; 3],
    parent_velocity: [f32; 3],
    parent_generation: u16,
    source: Option<&ParticleData>,
    target: &mut ParticleData,
) -> Result<(), GameError> {
    let slot_word = target.reserve_added_slot();
    if slot_word == 0xFFFF {
        return Ok(());
    }
    let slot = usize::from(slot_word);
    target.info_5c[slot].age_0c = current_time - death_time;
    // 007520FA-00752166 stores dy*dy + dx*dx + dz*dz to f32, then sqrt to another f32 before the first child draw.
    let squared_speed = parent_velocity[1] * parent_velocity[1]
        + parent_velocity[0] * parent_velocity[0]
        + parent_velocity[2] * parent_velocity[2];
    let parent_speed = squared_speed.sqrt();
    let speed_random = draw_fraction(hooks.draw_rand15())?;
    let speed_chaos = modifier.speed_chaos_24 * speed_random;
    let child_speed = (speed_chaos + 1.0) * parent_speed;
    let polar_random = draw_fraction(hooks.draw_rand15())?;
    let azimuth_random = draw_fraction(hooks.draw_rand15())?;
    let child_velocity = hooks.resolve_child_velocity(
        &parent_velocity,
        child_speed,
        polar_random,
        azimuth_random,
        modifier.direction_chaos_28,
    );
    target.info_5c[slot].velocity_00 = child_velocity;
    let lifespan_random = draw_fraction(hooks.draw_rand15())?;
    target.info_5c[slot].lifespan_10 =
        (lifespan_random - 0.5) * modifier.lifespan_variation_30 + modifier.lifespan_2c;
    target.info_5c[slot].generation_18 = (u32::from(parent_generation) + 1) as u16;
    target.positions[slot] = parent_position;
    if !parent_data(source, target).colors.is_empty() && !target.colors.is_empty() {
        target.colors[slot] = parent_data(source, target).colors[parent_index];
    }
    if !parent_data(source, target).radii.is_empty() && !target.radii.is_empty() {
        target.radii[slot] = parent_data(source, target).radii[parent_index];
    }
    if !parent_data(source, target).sizes_4c.is_empty() && !target.sizes_4c.is_empty() {
        target.sizes_4c[slot] = parent_data(source, target).sizes_4c[parent_index];
    }
    if !parent_data(source, target).rotation_angles_54.is_empty()
        && !target.rotation_angles_54.is_empty()
    {
        target.rotation_angles_54[slot] =
            parent_data(source, target).rotation_angles_54[parent_index];
        target.rotation_speeds_60[slot] =
            parent_data(source, target).rotation_speeds_60[parent_index];
    }
    if !parent_data(source, target).rotation_axes.is_empty() && !target.rotation_axes.is_empty() {
        target.rotation_axes[slot] = parent_data(source, target).rotation_axes[parent_index];
    }
    target.info_5c[slot].last_update_time_14 = current_time - target.info_5c[slot].age_0c;
    hooks.notify_child_added(slot_word, target);
    Ok(())
}

impl<H: ParticleSpawnChildHooks> ParticleSpawnServices for ParticleSpawnChildAdapter<'_, H> {
    /// IDA: N/A `ParticleSpawnChildAdapter`'s side of `ParticleSpawnServices`.
    fn draw_rand15(&mut self) -> u16 {
        self.hooks.draw_rand15()
    }

    /// IDA: 00751F10 same-system insertion, 0074ED40 allocation and 00749510 notification order, reading the parent's
    /// position and velocity and reserving a target slot before any child RNG. Completion: 100% of the bounded record
    /// and array stores for that path.
    /// TODO: [ ] Compare x87 and CRT math and the original's fixed-array ownership.
    fn spawn_child(
        &mut self,
        current_time: f32,
        death_time: f32,
        parent_index: u16,
        source: &mut ParticleData,
    ) -> Result<(), GameError> {
        if !std::ptr::eq(source, self.source_owner) {
            return Err(GameError::unclassified(
                0,
                "particle spawn source owner does not match frames",
            ));
        }
        let source_capacity = usize::from(source.vertex_count);
        if parent_index >= source.active_count_48
            || source.positions.len() < source_capacity
            || source.info_5c.len() < source_capacity
        {
            return Err(GameError::unclassified(
                0,
                "particle spawn source extent is invalid",
            ));
        }
        if too_short(&source.colors, source_capacity)
            || too_short(&source.radii, source_capacity)
            || too_short(&source.sizes_4c, source_capacity)
            || too_short(&source.rotation_angles_54, source_capacity)
            || too_short(&source.rotation_axes, source_capacity)
            || too_short(&source.rotation_speeds_60, source_capacity)
        {
            return Err(GameError::unclassified(
                0,
                "particle spawn source optional array is too short",
            ));
        }
        {
            let target: &ParticleData = match &self.cross {
                Some(cross) => cross.target,
                None => source,
            };
            let capacity = usize::from(target.vertex_count);
            if target.positions.len() < capacity
                || target.info_5c.len() < capacity
                || too_short(&target.colors, capacity)
                || too_short(&target.radii, capacity)
                || too_short(&target.sizes_4c, capacity)
                || too_short(&target.rotation_angles_54, capacity)
                || too_short(&target.rotation_axes, capacity)
                || too_short(&target.rotation_speeds_60, capacity)
            {
                return Err(GameError::unclassified(
                    0,
                    "particle spawn target array is too short",
                ));
            }
            if !source.rotation_angles_54.is_empty()
                && !target.rotation_angles_54.is_empty()
                && (source.rotation_speeds_60.is_empty() || target.rotation_speeds_60.is_empty())
            {
                return Err(GameError::unclassified(
                    0,
                    "particle spawn rotation-speed array is missing",
                ));
            }
        }
        let parent = usize::from(parent_index);
        let mut parent_position = source.positions[parent];
        let mut parent_velocity = source.info_5c[parent].velocity_00;
        let parent_generation = source.info_5c[parent].generation_18;
        match &mut self.cross {
            Some(cross) => {
                let inverse_target = invert_scene_transform_checked(
                    &cross.target_world,
                    GameError::unclassified(0, "particle spawn target world scale invalid"),
                )?;
                let world_position = transform_point(&cross.source_world, &parent_position);
                let world_velocity = rotate_vector(&cross.source_world.rotation, &parent_velocity);
                parent_position = transform_point(&inverse_target, &world_position);
                parent_velocity = rotate_vector(&inverse_target.rotation, &world_velocity);
                store_child(
                    self.modifier,
                    &mut self.hooks,
                    current_time,
                    death_time,
                    parent,
                    parent_position,
                    parent_velocity,
                    parent_generation,
                    Some(source),
                    cross.target,
                )
            }
            None => store_child(
                self.modifier,
                &mut self.hooks,
                current_time,
                death_time,
                parent,
                parent_position,
                parent_velocity,
                parent_generation,
                None,
                source,
            ),
        }
    }
}

/// IDA: 00752780 reads the common modifier payload, then +0x18 u16, +0x1C f32, +0x20/+0x22 u16 and
/// +0x24/+0x28/+0x2C/+0x30 f32 in exact order. Completion: 100% of the bounded modern payload and raw float bits.
/// TODO: [ ] Older base formats and NiStream callback errors.
pub fn parse_particle_spawn_payload(
    bytes: &[u8],
    version: u32,
) -> Result<ParticleSpawnPending, GameError> {
    let base = parse_particle_modifier_payload(bytes, version, &MODIFIER_BASE_TEXTS)?;
    let mut reader = reader_after(bytes, base.bytes_consumed);
    let generation_limit = reader.u16(TRUNCATED)?;
    let spawn_probability = reader.f32(TRUNCATED)?;
    let minimum_children = reader.u16(TRUNCATED)?;
    let maximum_children = reader.u16(TRUNCATED)?;
    let speed_chaos = reader.f32(TRUNCATED)?;
    let direction_chaos = reader.f32(TRUNCATED)?;
    let lifespan = reader.f32(TRUNCATED)?;
    let lifespan_variation = reader.f32(TRUNCATED)?;
    Ok(ParticleSpawnPending {
        base,
        generation_limit,
        spawn_probability,
        minimum_children,
        maximum_children,
        speed_chaos,
        direction_chaos,
        lifespan,
        lifespan_variation,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 00752CB0 resolves the common target reference; SpawnModifier has no other deferred link in 00752780 or its
/// vtable link slot. Completion: 100% of the checked index and publication.
/// TODO: [ ] Retain the target ParticleSystem and bind scheduling.
pub fn resolve_particle_spawn_link(
    target: &mut ParticleSpawnModifier,
    pending: &ParticleSpawnPending,
    object_count: usize,
) -> Result<(), GameError> {
    check_link_index(
        pending.base.target_index,
        object_count,
        GameError::unclassified(0, "particle spawn target link out of range"),
    )?;
    target.base = pending.base.published();
    target.generation_limit_18 = pending.generation_limit;
    target.spawn_probability_1c = pending.spawn_probability;
    target.minimum_children_20 = pending.minimum_children;
    target.maximum_children_22 = pending.maximum_children;
    target.speed_chaos_24 = pending.speed_chaos;
    target.direction_chaos_28 = pending.direction_chaos;
    target.lifespan_2c = pending.lifespan;
    target.lifespan_variation_30 = pending.lifespan_variation;
    Ok(())
}

/// IDA: N/A `resolve_particle_spawn_link` on a stream object.
pub fn link_particle_spawn(
    world: &mut World,
    entity: Entity,
    pending: &ParticleSpawnPending,
    table: &StreamObjectTable,
) -> Result<(), GameError> {
    let mut target =
        world
            .get_mut::<ParticleSpawnModifier>(entity)
            .ok_or(GameError::unclassified(
                0,
                "particle modifier component missing",
            ))?;
    resolve_particle_spawn_link(&mut target, pending, table.objects.len())
}

#[cfg(test)]
mod tests {
    use super::super::particle_data::ParticleInfo;
    use super::*;
    use crate::gamebryo::stream_objects::NULL_REFERENCE;

    #[test]
    fn payload_and_link() {
        let mut cold = ParticleSpawnModifier::default();
        assert_eq!(cold.base.name_08, None);
        assert_eq!(cold.base.order_0c, 0);
        assert!(cold.base.active_14);
        assert_eq!(cold.generation_limit_18, 1);
        assert_eq!(cold.spawn_probability_1c, 1.0);
        assert_eq!(cold.minimum_children_20, 1);
        assert_eq!(cold.maximum_children_22, 1);
        assert_eq!(cold.speed_chaos_24, 0.0);
        assert_eq!(cold.direction_chaos_28, 0.0);
        assert_eq!(cold.lifespan_2c, 0.0);
        assert_eq!(cold.lifespan_variation_30, 0.0);

        let mut bytes = Vec::new();
        bytes.extend(5u32.to_le_bytes());
        bytes.extend(b"Spawn");
        bytes.extend(9u32.to_le_bytes()); // order
        bytes.extend(u32::MAX.to_le_bytes()); // null target
        bytes.push(0); // inactive
        bytes.extend(3u16.to_le_bytes());
        bytes.extend(0x7FC1_2345u32.to_le_bytes()); // raw NaN probability
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(7u16.to_le_bytes());
        bytes.extend(0x8000_0000u32.to_le_bytes()); // signed zero speed chaos
        bytes.extend(0x3F80_0000u32.to_le_bytes());
        bytes.extend(0x4000_0000u32.to_le_bytes());
        bytes.extend(0x4040_0000u32.to_le_bytes());
        let expected_size = bytes.len();
        bytes.push(0xAB);
        let pending = parse_particle_spawn_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(pending.bytes_consumed, expected_size);
        assert_eq!(pending.base.name.as_deref(), Some(&b"Spawn"[..]));
        assert_eq!(pending.base.order, 9);
        assert!(!pending.base.active);
        assert_eq!(pending.base.target_index, NULL_REFERENCE);
        assert_eq!(pending.generation_limit, 3);
        assert_eq!(pending.minimum_children, 2);
        assert_eq!(pending.maximum_children, 7);
        assert_eq!(pending.spawn_probability.to_bits(), 0x7FC1_2345);
        assert_eq!(pending.speed_chaos.to_bits(), 0x8000_0000);
        assert_eq!(pending.direction_chaos, 1.0);
        assert_eq!(pending.lifespan, 2.0);
        assert_eq!(pending.lifespan_variation, 3.0);
        for length in 0..expected_size {
            assert!(parse_particle_spawn_payload(&bytes[..length], 0x1400_0004).is_err());
        }
        resolve_particle_spawn_link(&mut cold, &pending, 0).unwrap();
        assert_eq!(cold.base.name_08.as_deref(), Some(&b"Spawn"[..]));
        assert_eq!(cold.base.order_0c, 9);
        assert!(!cold.base.active_14);
        assert_eq!(cold.generation_limit_18, 3);
        assert_eq!(cold.minimum_children_20, 2);
        assert_eq!(cold.maximum_children_22, 7);
        assert_eq!(cold.spawn_probability_1c.to_bits(), 0x7FC1_2345);
        let mut bad = pending.clone();
        bad.base.target_index = 1;
        assert!(resolve_particle_spawn_link(&mut cold, &bad, 1).is_err());
        assert_eq!(cold.base.name_08.as_deref(), Some(&b"Spawn"[..]));
        assert_eq!(cold.base.order_0c, 9);
    }

    #[derive(Default)]
    struct MockServices {
        draws: Vec<u16>,
        draw_index: usize,
        calls: Vec<(f32, f32, u16, *const ParticleData)>,
    }

    impl ParticleSpawnServices for MockServices {
        fn draw_rand15(&mut self) -> u16 {
            assert!(
                self.draw_index < self.draws.len(),
                "spawn made an unexpected random draw"
            );
            self.draw_index += 1;
            self.draws[self.draw_index - 1]
        }

        fn spawn_child(
            &mut self,
            current_time: f32,
            death_time: f32,
            parent_index: u16,
            source: &mut ParticleData,
        ) -> Result<(), GameError> {
            self.calls
                .push((current_time, death_time, parent_index, source));
            Ok(())
        }
    }

    fn mock(draws: &[u16]) -> MockServices {
        MockServices {
            draws: draws.to_vec(),
            ..MockServices::default()
        }
    }

    fn make_modifier(
        generation: u16,
        probability: f32,
        min: u16,
        max: u16,
    ) -> ParticleSpawnModifier {
        let pending = ParticleSpawnPending {
            generation_limit: generation,
            spawn_probability: probability,
            minimum_children: min,
            maximum_children: max,
            ..ParticleSpawnPending::default()
        };
        let mut modifier = ParticleSpawnModifier::default();
        resolve_particle_spawn_link(&mut modifier, &pending, 0).unwrap();
        modifier
    }

    #[test]
    fn dispatch_branches() {
        let mut source = ParticleData {
            active_count_48: 1,
            info_5c: vec![ParticleInfo::default()],
            ..ParticleData::default()
        };
        let modifier = make_modifier(1, 1.0, 1, 2);

        source.info_5c[0].generation_18 = 1;
        let mut gated = mock(&[]);
        assert_eq!(
            modifier.dispatch(4.0, 0.25, 0, &mut source, &mut gated),
            Ok(0)
        );
        assert_eq!(gated.draw_index, 0);
        assert!(gated.calls.is_empty());

        source.info_5c[0].generation_18 = 0;
        let modifier = make_modifier(1, 0.0, 1, 2);
        let mut failed = mock(&[32767]);
        assert_eq!(
            modifier.dispatch(4.0, 0.25, 0, &mut source, &mut failed),
            Ok(0)
        );
        assert_eq!(failed.draw_index, 1);
        assert!(failed.calls.is_empty());

        let modifier = make_modifier(1, 1.0, 1, 2);
        let mut below = mock(&[0, 16383]);
        assert_eq!(
            modifier.dispatch(4.0, 0.25, 0, &mut source, &mut below),
            Ok(1)
        );
        assert_eq!(below.draw_index, 2);
        assert_eq!(below.calls.len(), 1);
        let mut above = mock(&[0, 16384]);
        assert_eq!(
            modifier.dispatch(4.0, 0.25, 0, &mut source, &mut above),
            Ok(2)
        );
        assert_eq!(above.draw_index, 2);
        assert_eq!(above.calls.len(), 2);
        for call in &above.calls {
            assert_eq!(call.0, 4.0);
            assert_eq!(call.1, 0.25);
            assert_eq!(call.2, 0);
            assert!(std::ptr::eq(call.3, &source));
        }
        assert_eq!(source.info_5c[0].generation_18, 0);

        let modifier = make_modifier(1, 1.0, 0, 0);
        let mut zero = mock(&[0, 0]);
        assert_eq!(
            modifier.dispatch(4.0, 0.25, 0, &mut source, &mut zero),
            Ok(1)
        );
        assert_eq!(zero.draw_index, 2);
        assert_eq!(zero.calls.len(), 1);

        let mut invalid = mock(&[32768]);
        assert!(
            modifier
                .dispatch(4.0, 0.25, 0, &mut source, &mut invalid)
                .is_err()
        );
        assert_eq!(invalid.draw_index, 1);
        assert!(invalid.calls.is_empty());
    }

    #[derive(Default)]
    struct ChildHooks {
        values: Vec<u16>,
        next: usize,
        order: Vec<char>,
        notifications: Vec<u16>,
        observed_speed: f32,
        observed_polar: f32,
        observed_azimuth: f32,
        observed_chaos: f32,
    }

    impl ParticleSpawnChildHooks for ChildHooks {
        fn draw_rand15(&mut self) -> u16 {
            assert!(
                self.next < self.values.len(),
                "spawn child consumed an unexpected random draw"
            );
            self.order.push('R');
            self.next += 1;
            self.values[self.next - 1]
        }

        fn resolve_child_velocity(
            &mut self,
            parent: &[f32; 3],
            speed: f32,
            polar: f32,
            azimuth: f32,
            chaos: f32,
        ) -> [f32; 3] {
            assert_eq!(*parent, [3.0, 4.0, 0.0]);
            self.order.push('V');
            self.observed_speed = speed;
            self.observed_polar = polar;
            self.observed_azimuth = azimuth;
            self.observed_chaos = chaos;
            [1.0, 2.0, 3.0]
        }

        fn notify_child_added(&mut self, child_index: u16, target: &ParticleData) {
            self.order.push('N');
            self.notifications.push(child_index);
            let child = usize::from(child_index);
            assert_eq!(target.info_5c[child].last_update_time_14, 3.5);
            assert_eq!(target.positions[child], target.positions[0]);
        }
    }

    #[test]
    fn child_adapter_stores_in_order() {
        let pending = ParticleSpawnPending {
            speed_chaos: 0.5,
            direction_chaos: 0.25,
            lifespan: 10.0,
            lifespan_variation: 4.0,
            ..ParticleSpawnPending::default()
        };
        let mut modifier = ParticleSpawnModifier::default();
        resolve_particle_spawn_link(&mut modifier, &pending, 0).unwrap();
        let mut target = ParticleData {
            vertex_count: 3,
            active_count_48: 1,
            positions: vec![[0.0; 3]; 3],
            info_5c: vec![ParticleInfo::default(); 3],
            colors: vec![[0.0; 4]; 3],
            radii: vec![0.0; 3],
            sizes_4c: vec![0.0; 3],
            rotation_angles_54: vec![0.0; 3],
            rotation_axes: vec![[0.0; 3]; 3],
            rotation_speeds_60: vec![0.0; 3],
            ..ParticleData::default()
        };
        target.positions[0] = [7.0, 8.0, 9.0];
        target.info_5c[0].velocity_00 = [3.0, 4.0, 0.0];
        target.info_5c[0].generation_18 = 2;
        target.colors[0] = [1.0, 2.0, 3.0, 4.0];
        target.radii[0] = 5.0;
        target.sizes_4c[0] = 6.0;
        target.rotation_angles_54[0] = 7.0;
        target.rotation_axes[0] = [0.0, 1.0, 0.0];
        target.rotation_speeds_60[0] = 8.0;
        let hooks = ChildHooks {
            values: vec![32767, 0, 32767, 32767],
            ..ChildHooks::default()
        };
        let mut adapter = ParticleSpawnChildAdapter::same_system(&modifier, &target, hooks);
        adapter.spawn_child(4.0, 3.5, 0, &mut target).unwrap();
        assert_eq!(adapter.hooks.next, 4);
        assert_eq!(adapter.hooks.order, ['R', 'R', 'R', 'V', 'R', 'N']);
        assert_eq!(adapter.hooks.notifications, [1]);
        assert_eq!(adapter.hooks.observed_speed, 7.5);
        assert_eq!(adapter.hooks.observed_polar, 0.0);
        assert_eq!(adapter.hooks.observed_azimuth, 1.0);
        assert_eq!(adapter.hooks.observed_chaos, 0.25);
        assert_eq!(target.active_count_48, 1);
        assert_eq!(target.added_base, 1);
        assert_eq!(target.added_count, 1);
        assert_eq!(target.info_5c[1].age_0c, 0.5);
        assert_eq!(target.info_5c[1].lifespan_10, 12.0);
        assert_eq!(target.info_5c[1].generation_18, 3);
        assert_eq!(target.info_5c[1].velocity_00, [1.0, 2.0, 3.0]);
        assert_eq!(target.colors[1], target.colors[0]);
        assert_eq!(target.radii[1], 5.0);
        assert_eq!(target.sizes_4c[1], 6.0);
        assert_eq!(target.rotation_angles_54[1], 7.0);
        assert_eq!(target.rotation_axes[1], target.rotation_axes[0]);
        assert_eq!(target.rotation_speeds_60[1], 8.0);
        adapter.hooks.values.extend([32767, 0, 32767, 32767]);
        adapter.spawn_child(4.0, 3.5, 0, &mut target).unwrap();
        assert_eq!(target.added_count, 2);
        assert_eq!(adapter.hooks.notifications.last(), Some(&2));
        let before = adapter.hooks.next;
        adapter.spawn_child(4.0, 3.5, 0, &mut target).unwrap();
        assert_eq!(adapter.hooks.next, before);
        assert_eq!(target.added_count, 2);
        assert_eq!(adapter.hooks.notifications.len(), 2);
    }

    #[test]
    fn default_direction_branches() {
        let near_positive = default_child_velocity(&[0.0, 0.0, 1.0], 2.0, 0.0, 0.0, 1.0);
        let near_negative = default_child_velocity(&[0.0, 0.0, -1.0], 2.0, 0.0, 0.0, 1.0);
        let positive = default_child_velocity(&[1.0, 0.0, 1.0], 2.0, 1.0, 0.0, 1.0);
        let negative = default_child_velocity(&[1.0, 0.0, 1.0], 2.0, 0.0, 0.0, 1.0);
        assert_eq!(near_positive, [0.0, 0.0, 2.0]);
        assert_eq!(near_negative, [0.0, 0.0, -2.0]);
        assert!(positive.iter().all(|value| value.is_finite()));
        assert!(negative.iter().all(|value| value.is_finite()));
        assert_ne!(positive, negative);
    }

    #[derive(Default)]
    struct CrossHooks {
        next: usize,
        received_parent: [f32; 3],
        received_speed: f32,
        notified: u16,
    }

    impl ParticleSpawnChildHooks for CrossHooks {
        fn draw_rand15(&mut self) -> u16 {
            assert!(self.next < 4);
            self.next += 1;
            0
        }

        fn resolve_child_velocity(
            &mut self,
            velocity: &[f32; 3],
            speed: f32,
            _polar: f32,
            _azimuth: f32,
            _chaos: f32,
        ) -> [f32; 3] {
            self.received_parent = *velocity;
            self.received_speed = speed;
            *velocity
        }

        fn notify_child_added(&mut self, index: u16, _target: &ParticleData) {
            self.notified = index;
        }
    }

    #[test]
    fn cross_system_transforms_the_parent() {
        let modifier = ParticleSpawnModifier::default();
        let mut source = ParticleData {
            vertex_count: 1,
            active_count_48: 1,
            positions: vec![[1.0, 2.0, 0.0]],
            info_5c: vec![ParticleInfo::default()],
            ..ParticleData::default()
        };
        source.info_5c[0].velocity_00 = [1.0, 0.0, 0.0];
        let mut target = ParticleData {
            vertex_count: 1,
            positions: vec![[0.0; 3]],
            info_5c: vec![ParticleInfo::default()],
            ..ParticleData::default()
        };
        let source_world = NiTransform {
            rotation: [0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            translation: [10.0, 4.0, 0.0],
            scale: 2.0,
        };
        let target_world = NiTransform {
            translation: [4.0, 0.0, 0.0],
            scale: 2.0,
            ..NiTransform::default()
        };
        let mut adapter = ParticleSpawnChildAdapter::cross_system(
            &modifier,
            &source,
            &mut target,
            source_world,
            target_world,
            CrossHooks::default(),
        );
        adapter.spawn_child(4.0, 3.5, 0, &mut source).unwrap();
        assert_eq!(adapter.hooks.next, 4);
        assert_eq!(adapter.hooks.notified, 0);
        assert_eq!(adapter.hooks.received_parent, [0.0, 1.0, 0.0]);
        assert_eq!(adapter.hooks.received_speed, 1.0);
        let stored = adapter.cross_target().unwrap();
        assert_eq!(stored.added_count, 1);
        assert_eq!(stored.positions[0], [1.0, 3.0, 0.0]);
        assert_eq!(stored.info_5c[0].velocity_00, [0.0, 1.0, 0.0]);
        let mut unrelated = source.clone();
        assert!(adapter.spawn_child(4.0, 3.5, 0, &mut unrelated).is_err());
        assert_eq!(adapter.hooks.next, 4);
    }
}
