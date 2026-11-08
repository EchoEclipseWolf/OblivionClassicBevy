//! NiBlendBoolInterpolator, NiBlendFloatInterpolator, NiBlendTransformInterpolator and their NiBlendInterpolator base.
//!
//! The C++ `shared_ptr` source variants are `Entity` plus a `NiClass` check. A sample reaches other interpolator entities,
//! so it takes `&mut World`; the `sample_particle_blend_*` functions detach a blend's component for the call. The C++
//! `const void* target` parameter is dropped: nothing reads it in the host, and the original's NiObject target is a TODO.
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.
//! TODO: [ ] A later phase's worker thread that loads or evaluates these runs at its own control word (workers are not recorded;
//! a thread that keeps the default is 027F, 53-bit). Main-thread behaviour stays until one is reached.

#![allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "006EB5E0, 006D2600 and 00750040's branches take these negated comparisons, so a NaN follows the same path"
)]

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::animation::update_float_interpolator;
use super::bool_animation::{update_bool_interpolator, update_bool_timeline_interpolator};
use super::path_interpolator::update_path_interpolator;
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};
use super::transform_animation::{
    QuatTransform, TransformUpdateStatus, normalize_quaternion, quat_transform_invalid,
    scale_quaternion, set_quat_transform_rotate, set_quat_transform_scale,
    set_quat_transform_translate, update_transform_interpolator,
};

const NEGATIVE_MAX: f32 = -f32::MAX;
const TRUNCATED_BYTE: &str = "truncated blend interpolator byte";

/// IDA: 00B242A0 the NiBlendInterpolator array grow-by byte (image value 2).
pub const BLEND_ARRAY_GROW_BY: u8 = 2;

/// IDA: 006CD680 reads one 0x18-byte NiBlendInterpolator item: object ref, weight +4, normalized weight +8, signed priority
/// +0xC, ease +0x10. The +0x14 runtime time is not streamed.
/// TODO: [ ] Bind NiStream vtable +0x2C object-reference callback semantics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleBlendItemPayload {
    pub interpolator_index: u32,
    pub weight: f32,
    pub normalized_weight: f32,
    pub priority: i8,
    pub ease: f32,
    pub runtime_time: f32,
}

impl Default for ParticleBlendItemPayload {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            interpolator_index: NULL_REFERENCE,
            weight: 0.0,
            normalized_weight: 0.0,
            priority: 0,
            ease: 0.0,
            runtime_time: NEGATIVE_MAX,
        }
    }
}

/// IDA: 006CDC10 NiBlendInterpolator::LoadBinary, then 006EB880 / 006D2870's derived raw value at +0x30.
/// Completion: 85% of the supported modern complete payload; partial read callbacks are external.
/// TODO: [ ] Recover stream-version gating and exact immediate ref ownership.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleBlendPayload<V> {
    pub flags: u8,
    pub capacity: u8,
    pub weight_threshold: f32,
    pub active_count: u8,
    pub single_index: u8,
    pub high_priority: i8,
    pub second_priority: i8,
    pub cached_time: f32,
    pub high_weight_sum: f32,
    pub second_weight_sum: f32,
    pub high_ease: f32,
    pub items: Vec<ParticleBlendItemPayload>,
    pub single_interpolator_index: u32,
    pub value: V,
    pub bytes_consumed: usize,
}

pub type ParticleBlendBoolPayload = ParticleBlendPayload<u8>;
pub type ParticleBlendFloatPayload = ParticleBlendPayload<f32>;
/// IDA: 006CBD40: base 006CD720 only, no derived value. Below 0A01006E a discarded NiQuatTransform follows (excluded).
pub type ParticleBlendTransformPayload = ParticleBlendPayload<()>;

// IDA: 006CDC10 base LoadBinary, 006CD680 0x18-byte item LoadBinary
// Completion: 85% of the original callback order on a complete host stream
// TODO: [ ] Resolve NiStream +0x2C reference-read side effects and versions, and partial callback writes.
fn read_blend_payload<V: Default>(
    bytes: &[u8],
    read_value: impl FnOnce(&mut ByteReader<'_>) -> Result<V, StreamError>,
) -> Result<ParticleBlendPayload<V>, StreamError> {
    let mut reader = ByteReader::new(bytes);
    let mut result = ParticleBlendPayload {
        flags: reader.u8(TRUNCATED_BYTE)?,
        capacity: reader.u8(TRUNCATED_BYTE)?,
        weight_threshold: reader.f32(TRUNCATED_BYTE)?,
        active_count: 0,
        single_index: 0xFF,
        high_priority: -128,
        second_priority: -128,
        cached_time: 0.0,
        high_weight_sum: 0.0,
        second_weight_sum: 0.0,
        high_ease: 0.0,
        items: Vec::new(),
        single_interpolator_index: NULL_REFERENCE,
        value: V::default(),
        bytes_consumed: 0,
    };
    if result.flags & 1 == 0 {
        result.active_count = reader.u8(TRUNCATED_BYTE)?;
        result.single_index = reader.u8(TRUNCATED_BYTE)?;
        result.high_priority = reader.u8(TRUNCATED_BYTE)? as i8;
        result.second_priority = reader.u8(TRUNCATED_BYTE)? as i8;
        result.cached_time = reader.f32(TRUNCATED_BYTE)?;
        result.high_weight_sum = reader.f32(TRUNCATED_BYTE)?;
        result.second_weight_sum = reader.f32(TRUNCATED_BYTE)?;
        result.high_ease = reader.f32(TRUNCATED_BYTE)?;
        result.items.reserve(usize::from(result.capacity));
        for _ in 0..result.capacity {
            let interpolator_index = reader.u32(TRUNCATED_BYTE)?;
            let weight = reader.f32(TRUNCATED_BYTE)?;
            let normalized_weight = reader.f32(TRUNCATED_BYTE)?;
            let priority = reader.u8(TRUNCATED_BYTE)? as i8;
            let ease = reader.f32(TRUNCATED_BYTE)?;
            result.items.push(ParticleBlendItemPayload {
                interpolator_index,
                weight,
                normalized_weight,
                priority,
                ease,
                ..ParticleBlendItemPayload::default()
            });
        }
        result.single_interpolator_index = reader.u32(TRUNCATED_BYTE)?;
    }
    result.value = read_value(&mut reader)?;
    result.bytes_consumed = reader.position();
    Ok(result)
}

// IDA: 006CDC10 then 006EB880, vtable A7C5B4 +0x28; the original +0x30 raw bool byte
// Completion: 85% of the ordered base and derived raw-byte parse
// TODO: [ ] Bind short NiStream callbacks and original allocation failures.
pub fn parse_particle_blend_bool_payload(
    bytes: &[u8],
) -> Result<ParticleBlendBoolPayload, StreamError> {
    read_blend_payload(bytes, |reader| reader.u8(TRUNCATED_BYTE))
}

// IDA: 006CDC10 then 006D2870, vtable A7A49C +0x28; the original +0x30 float
// Completion: 85% of the ordered base and derived float parse
// TODO: [ ] Bind short NiStream callbacks and original allocation failures.
pub fn parse_particle_blend_float_payload(
    bytes: &[u8],
) -> Result<ParticleBlendFloatPayload, StreamError> {
    read_blend_payload(bytes, |reader| reader.f32(TRUNCATED_BYTE))
}

// IDA: 006CDC10 then 006EAD00's NiVector3_LoadBinary (NiBlendPoint3Interpolator), raw bits.
// Completion: 100% of the ordered base and derived parse.
pub fn parse_particle_blend_point3_payload(
    bytes: &[u8],
) -> Result<ParticleBlendPayload<[u32; 3]>, StreamError> {
    read_blend_payload(bytes, |reader| {
        Ok([
            reader.u32(TRUNCATED_BYTE)?,
            reader.u32(TRUNCATED_BYTE)?,
            reader.u32(TRUNCATED_BYTE)?,
        ])
    })
}

// IDA: 006CBD40 -> 006CD720 modern base read; no derived value
// Completion: 100% of the supported field order
// TODO: [ ] Port the < 0A01006E trailing discarded NiQuatTransform read.
pub fn parse_particle_blend_transform_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<ParticleBlendTransformPayload, StreamError> {
    if stream_version < 0x0A01_006E {
        return Err(StreamError(
            "legacy NiBlendTransformInterpolator payload unsupported",
        ));
    }
    read_blend_payload(bytes, |_| Ok(()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleBlendSampleStatus {
    Sampled,
    Failed,
    UnsupportedDirtyWeights,
    UnsupportedMissingOwner,
    UnsupportedNestedBranch,
    UnsupportedItemMetadata,
}

/// IDA: 006CC4E0 cold 0x30-byte base followed by the derived +0x30 value: +08 last time, +0C flags, +0D capacity, +0E active
/// count, +0F single index (-1), +10 / +11 priorities (0x80), +14 array, +18 single, +1C threshold 0, +20 single time, +24 /
/// +28 weight sums and +2C ease (all -FLT_MAX). Completion: 90% of the raw scalar, byte and item metadata; `Vec` owns the
/// host slots.
/// TODO: [ ] Bind x86 intrusive references and heap array identity.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleBlendCold {
    pub flags_0c: u8,
    pub capacity_0d: u8,
    pub active_count_0e: u8,
    pub single_index_0f: u8,
    pub high_priority_10: i8,
    pub second_priority_11: i8,
    pub weight_threshold_1c: f32,
    /// IDA: +0x08 NiInterpolator last time, stored by every Update.
    pub last_time_08: f32,
    /// IDA: +0x20 single-item time: streamed for non-manager blends and read by the manager-controlled single paths
    /// (006EB570 / 006CBDF0).
    pub single_time_20: f32,
    pub high_weight_sum_24: f32,
    pub second_weight_sum_28: f32,
    pub high_ease_2c: f32,
    pub single_interpolator_index: u32,
    pub items_14: Vec<ParticleBlendItemPayload>,
}

impl Default for ParticleBlendCold {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            flags_0c: 0,
            capacity_0d: 0,
            active_count_0e: 0,
            single_index_0f: 0xFF,
            high_priority_10: -128,
            second_priority_11: -128,
            weight_threshold_1c: 0.0,
            last_time_08: NEGATIVE_MAX,
            single_time_20: NEGATIVE_MAX,
            high_weight_sum_24: NEGATIVE_MAX,
            second_weight_sum_28: NEGATIVE_MAX,
            high_ease_2c: NEGATIVE_MAX,
            single_interpolator_index: NULL_REFERENCE,
            items_14: Vec::new(),
        }
    }
}

impl ParticleBlendCold {
    // IDA: 006CDC10 stores base fields before 006EB880 / 006D2870 +0x30
    // Completion: 100% of the supported scalar and item publication stage
    // TODO: [ ] Bind original partial callback store effects.
    pub fn apply_loaded_prefix<V>(&mut self, payload: &ParticleBlendPayload<V>) {
        self.flags_0c = payload.flags;
        self.capacity_0d = payload.capacity;
        self.weight_threshold_1c = payload.weight_threshold;
        if self.flags_0c & 1 == 0 {
            self.active_count_0e = payload.active_count;
            self.single_index_0f = payload.single_index;
            self.high_priority_10 = payload.high_priority;
            self.second_priority_11 = payload.second_priority;
            self.single_time_20 = payload.cached_time;
            self.high_weight_sum_24 = payload.high_weight_sum;
            self.second_weight_sum_28 = payload.second_weight_sum;
            self.high_ease_2c = payload.high_ease;
            self.items_14.clone_from(&payload.items);
            self.single_interpolator_index = payload.single_interpolator_index;
        }
    }
}

/// IDA: 006EB4C0 constructor, A7C5B4 +0x60 = 006EB750 Update. Completion: 85% of the one, clean-many and two-dirty branches
/// and raw +0x30 byte outputs.
/// TODO: [ ] Port 006CD0F0 dirty > 2 normalization and manager owners.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ParticleBlendBoolInterpolator {
    pub base: ParticleBlendCold,
    pub value_30: u8,
    pub item_sources: Vec<Option<Entity>>,
    pub single_source_18: Option<Entity>,
}

impl Default for ParticleBlendBoolInterpolator {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            base: ParticleBlendCold::default(),
            value_30: 2,
            item_sources: Vec::new(),
            single_source_18: None,
        }
    }
}

/// IDA: 006D24E0 constructor, A7A49C +0x5C = 006D2740 Update. Completion: 85% of the one, clean-many and two-dirty float and
/// failed sentinel branches.
/// TODO: [ ] Port 006CD0F0 dirty > 2 normalization and manager owners.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ParticleBlendFloatInterpolator {
    pub base: ParticleBlendCold,
    pub value_30: f32,
    pub item_sources: Vec<Option<Entity>>,
    pub single_source_18: Option<Entity>,
}

impl Default for ParticleBlendFloatInterpolator {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            base: ParticleBlendCold::default(),
            value_30: NEGATIVE_MAX,
            item_sources: Vec::new(),
            single_source_18: None,
        }
    }
}

/// IDA: 006CBCE0 NiBlendTransformInterpolator factory: 0x30 bytes, 006CC4E0 base, vtable A79E54, +0x4C Update 006CC3A0.
/// Completion: 90% of the single, clean-many and two-dirty branches; the B3CBD0 single-item flag is treated as its
/// normal-startup value 0.
/// TODO: [ ] Port 006CD0F0 dirty > 2 normalization and manager owners.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ParticleBlendTransformInterpolator {
    pub base: ParticleBlendCold,
    pub item_sources: Vec<Option<Entity>>,
    pub single_source_18: Option<Entity>,
}

/// IDA: N/A the shared NiBlendInterpolator base methods (vtables A79E54 / A7C5B4 / A7A49C / A79F0C, slots +0x98 / +0x9C / +0xA4)
/// over the three host blend components, in place of the C++ `blend_base` templates.
pub trait BlendInterpolator {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn slots(
        &mut self,
    ) -> (
        &mut ParticleBlendCold,
        &mut Vec<Option<Entity>>,
        &mut Option<Entity>,
    );

    // IDA: N/A keeps the host item and source vectors sized to the +0x0D capacity byte
    // Completion: 100%
    fn sync_capacity(&mut self) {
        let (base, items, _) = self.slots();
        let capacity = usize::from(base.capacity_0d);
        base.items_14
            .resize(capacity, ParticleBlendItemPayload::default());
        items.resize(capacity, None);
    }

    // IDA: 006CCFD0 ReallocateArrays: fails when 00B242A0 is 0; otherwise the capacity becomes (u8)(capacity + grow-by), old
    // items are copied (006CC890) into fresh 006CCDE0 cold items and the old array destroyed
    // Completion: 100%; a u8 wrap below the old capacity truncates on the host where the original overruns the new array
    fn reallocate_arrays(&mut self) -> bool {
        if BLEND_ARRAY_GROW_BY == 0 {
            return false;
        }
        let (base, _, _) = self.slots();
        base.capacity_0d = BLEND_ARRAY_GROW_BY.wrapping_add(base.capacity_0d);
        self.sync_capacity();
        true
    }

    // IDA: 006CC5C0 AddInterpInfo(interp, weight, priority, ease): the first free item (null +0) or a 006CCFD0 growth (failure
    // returns 0xFF); stores the reference, weight +4, priority +0xC and ease +0x10; raises the two signed priorities; the
    // first active item becomes the +0x0F / +0x18 single, a second clears them and the +0x20 single time; then the three
    // cached sums become -FLT_MAX and flag 4 is set
    // Completion: 100%
    fn add_interp_info(&mut self, source: Entity, weight: f32, priority: i8, ease: f32) -> u8 {
        self.sync_capacity();
        let capacity = self.slots().0.capacity_0d;
        let mut index = 0u8;
        while index < capacity && self.slots().1[usize::from(index)].is_some() {
            index += 1;
        }
        if index == capacity && !self.reallocate_arrays() {
            return 0xFF;
        }
        let (base, items, single) = self.slots();
        let item = &mut base.items_14[usize::from(index)];
        items[usize::from(index)] = Some(source);
        item.weight = weight;
        item.priority = priority;
        item.ease = ease;
        if priority > base.second_priority_11 {
            if priority <= base.high_priority_10 {
                if priority != base.high_priority_10 {
                    base.second_priority_11 = priority;
                }
            } else {
                base.second_priority_11 = base.high_priority_10;
                base.high_priority_10 = priority;
            }
        }
        base.active_count_0e = base.active_count_0e.wrapping_add(1);
        if base.active_count_0e == 1 {
            base.single_index_0f = index;
            *single = Some(source);
        } else {
            base.single_index_0f = 0xFF;
            base.single_time_20 = NEGATIVE_MAX;
            *single = None;
        }
        base.high_weight_sum_24 = NEGATIVE_MAX;
        base.second_weight_sum_28 = NEGATIVE_MAX;
        base.flags_0c |= 4;
        base.high_ease_2c = NEGATIVE_MAX;
        index
    }

    // IDA: 006CC6E0 RemoveInterpInfo(out, index): returns the item reference, clears the item (priority, weights and ease 0,
    // time -FLT_MAX), recomputes both priorities from the occupied items when the removed priority was one of them,
    // decrements the active count and, at 1, picks the remaining single (the other slot for capacity 2, else the last
    // occupied) with its time, or at 0 clears the single; then the cached sums reset and flag 4 is set
    // Completion: 100%; an out-of-range index (0xFF) is a host no-op
    fn remove_interp_info(&mut self, index: u8) -> Option<Entity> {
        self.sync_capacity();
        let (base, items, single) = self.slots();
        if index >= base.capacity_0d {
            return None;
        }
        let slot = usize::from(index);
        let released = items[slot].take();
        let item = &mut base.items_14[slot];
        let removed_priority = item.priority;
        item.priority = 0;
        item.weight = 0.0;
        item.normalized_weight = 0.0;
        item.ease = 0.0;
        item.runtime_time = NEGATIVE_MAX;
        if removed_priority == base.high_priority_10 || removed_priority == base.second_priority_11
        {
            base.second_priority_11 = -128;
            base.high_priority_10 = -128;
            for (candidate, _) in items
                .iter()
                .enumerate()
                .take(usize::from(base.capacity_0d))
                .filter(|(_, source)| source.is_some())
            {
                let priority = base.items_14[candidate].priority;
                if priority > base.second_priority_11 {
                    if priority <= base.high_priority_10 {
                        if priority < base.high_priority_10 {
                            base.second_priority_11 = priority;
                        }
                    } else {
                        base.second_priority_11 = base.high_priority_10;
                        base.high_priority_10 = priority;
                    }
                }
            }
        }
        base.active_count_0e = base.active_count_0e.wrapping_sub(1);
        let remaining = base.active_count_0e;
        if remaining == 1 {
            if base.capacity_0d == 2 {
                base.single_index_0f = u8::from(index == 0);
            } else {
                for candidate in 0..base.capacity_0d {
                    if items[usize::from(candidate)].is_some() {
                        base.single_index_0f = candidate;
                    }
                }
            }
            let chosen = usize::from(base.single_index_0f);
            *single = items[chosen];
            base.single_time_20 = base.items_14[chosen].runtime_time;
        } else if remaining == 0 {
            base.single_index_0f = 0xFF;
            *single = None;
            base.single_time_20 = NEGATIVE_MAX;
        }
        base.high_weight_sum_24 = NEGATIVE_MAX;
        base.second_weight_sum_28 = NEGATIVE_MAX;
        base.flags_0c |= 4;
        base.high_ease_2c = NEGATIVE_MAX;
        released
    }

    // IDA: 006CD680 item +0x00 original interpolator pointer
    // Completion: 85% of the checked typed publication at the serialized index
    // TODO: [ ] Bind intrusive reference counts and original null slots.
    fn bind_item(&mut self, index: usize, owner: Entity) -> Result<(), StreamError> {
        let (base, items, _) = self.slots();
        if index >= items.len() || base.items_14[index].interpolator_index == NULL_REFERENCE {
            return Err(StreamError("blend item reference absent"));
        }
        items[index] = Some(owner);
        Ok(())
    }

    // IDA: 006CDC10 / 006CD720 final +0x18 immediate single-interpolator read
    // Completion: 85% of the typed strong owner publication after the item callbacks
    // TODO: [ ] Bind original pointer-map and refcount semantics.
    fn bind_single(&mut self, owner: Entity) -> Result<(), StreamError> {
        let (base, _, single) = self.slots();
        if base.single_interpolator_index == NULL_REFERENCE {
            return Err(StreamError("blend single reference absent"));
        }
        *single = Some(owner);
        Ok(())
    }
}

macro_rules! blend_interpolator {
    ($name:ty) => {
        impl BlendInterpolator for $name {
            /// IDA: N/A a helper, with no address of its own.
            fn slots(
                &mut self,
            ) -> (
                &mut ParticleBlendCold,
                &mut Vec<Option<Entity>>,
                &mut Option<Entity>,
            ) {
                (
                    &mut self.base,
                    &mut self.item_sources,
                    &mut self.single_source_18,
                )
            }
        }
    };
}

blend_interpolator!(ParticleBlendBoolInterpolator);
blend_interpolator!(ParticleBlendFloatInterpolator);
blend_interpolator!(super::material_controllers::BlendPoint3Interpolator);
blend_interpolator!(ParticleBlendTransformInterpolator);

// IDA: N/A bounded host preflight for the serialized array and current owners
// Completion: 100%; protects host vector indexing where a malformed stream in the original may fault
// TODO: [ ] Original allocator and fault behaviour are out of scope for the host.
fn valid_many_items(base: &ParticleBlendCold, owner_count: usize) -> bool {
    base.items_14.len() == usize::from(base.capacity_0d)
        && owner_count == usize::from(base.capacity_0d)
        && base.active_count_0e <= base.capacity_0d
}

// IDA: 006CD0F0 dirty active-count == 2 clears flag bit 4, then 006CC900 scans the first two occupied slots and writes the
// normalized +0x08 weights
// Completion: 85% of the two-owner priority, ease and threshold branches with the original f32 store points. Precision:
// 24-bit per step on the main thread after CreateDevice.
// TODO: [ ] Port the > 2 owner 006CD136+ path.
fn recompute_two_weights(base: &mut ParticleBlendCold, sources: &[Option<Entity>]) -> bool {
    let mut occupied = [0usize; 2];
    let mut count = 0;
    for (index, source) in sources
        .iter()
        .enumerate()
        .take(usize::from(base.capacity_0d))
    {
        if count == occupied.len() {
            break;
        }
        if source.is_some() {
            occupied[count] = index;
            count += 1;
        }
    }
    if count != 2 {
        return false;
    }
    base.flags_0c &= 0xFB;
    let (lower, upper) = base.items_14.split_at_mut(occupied[1]);
    let first = &mut lower[occupied[0]];
    let second = &mut upper[0];
    let first_weight = first.weight * first.ease;
    let second_weight = second.weight * second.ease;
    if first_weight == 0.0 && second_weight == 0.0 {
        first.normalized_weight = 0.0;
        second.normalized_weight = 0.0;
        return true;
    }
    if first.priority > second.priority {
        if first.ease == 1.0 {
            first.normalized_weight = 1.0;
            second.normalized_weight = 0.0;
            return true;
        }
        let complement = 1.0 - first.ease;
        let first_part = first_weight * first.ease;
        let second_part = second_weight * complement;
        let reciprocal = 1.0 / (second_part + first_part);
        first.normalized_weight = first_part * reciprocal;
        second.normalized_weight = second_part * reciprocal;
    } else if first.priority < second.priority {
        if second.ease == 1.0 {
            first.normalized_weight = 0.0;
            second.normalized_weight = 1.0;
            return true;
        }
        let complement = 1.0 - second.ease;
        let first_part = first_weight * complement;
        let second_part = second_weight * second.ease;
        let reciprocal = 1.0 / (second_part + first_part);
        first.normalized_weight = first_part * reciprocal;
        second.normalized_weight = second_part * reciprocal;
    } else {
        let reciprocal = 1.0 / (second_weight + first_weight);
        first.normalized_weight = first_weight * reciprocal;
        second.normalized_weight = second_weight * reciprocal;
    }
    if base.flags_0c & 2 != 0 {
        if second.normalized_weight <= first.normalized_weight {
            first.normalized_weight = 1.0;
            second.normalized_weight = 0.0;
        } else {
            first.normalized_weight = 0.0;
            second.normalized_weight = 1.0;
        }
        return true;
    }
    if base.weight_threshold_1c > 0.0 {
        let mut first_removed = false;
        if base.weight_threshold_1c > first.normalized_weight {
            first.normalized_weight = 0.0;
            first_removed = true;
        }
        if base.weight_threshold_1c <= second.normalized_weight {
            if first_removed {
                second.normalized_weight = 1.0;
            }
        } else {
            // 006CCACA / 006CCACD: both stores go to the second item, 0 then 1, in the original's disassembly.
            second.normalized_weight = 0.0;
            second.normalized_weight = 1.0;
        }
    }
    true
}

// IDA: 006EB570 / 006EB5E0 invoke nested +0x60 bool interpolation
// Completion: 85% of the typed direct, timeline and nested call and the raw output byte
// TODO: [ ] Bind the original NiObject target pointer and all derived virtuals.
fn sample_bool_source(
    world: &mut World,
    source: Entity,
    time: f32,
    raw: &mut u8,
) -> ParticleBlendSampleStatus {
    let mut value = false;
    let okay = match world.get::<NiClass>(source) {
        Some(NiClass::BlendBoolInterpolator) => {
            return sample_particle_blend_bool(world, source, time, raw);
        }
        Some(NiClass::BoolTimelineInterpolator) => {
            update_bool_timeline_interpolator(world, source, time, &mut value)
        }
        Some(NiClass::BoolInterpolator) => {
            update_bool_interpolator(world, source, time, &mut value)
        }
        _ => return ParticleBlendSampleStatus::UnsupportedMissingOwner,
    };
    if okay {
        *raw = u8::from(value);
        ParticleBlendSampleStatus::Sampled
    } else {
        ParticleBlendSampleStatus::Failed
    }
}

// IDA: 006D2580 / 006D2600 invoke nested +0x5C float interpolation
// Completion: 85% of the typed direct and nested call and the f32 output
// TODO: [ ] Bind the original NiObject target pointer and all derived virtuals.
fn sample_float_source(
    world: &mut World,
    source: Entity,
    time: f32,
    raw: &mut f32,
) -> ParticleBlendSampleStatus {
    match world.get::<NiClass>(source) {
        Some(NiClass::BlendFloatInterpolator) => {
            sample_particle_blend_float(world, source, time, raw)
        }
        Some(NiClass::FloatInterpolator) => {
            if update_float_interpolator(world, source, time, raw) {
                ParticleBlendSampleStatus::Sampled
            } else {
                ParticleBlendSampleStatus::Failed
            }
        }
        _ => ParticleBlendSampleStatus::UnsupportedMissingOwner,
    }
}

impl ParticleBlendBoolInterpolator {
    // IDA: 006EB880 reads base scalars before immediate item and single refs; bind refs next, value last
    // Completion: 100% of the supported prefix publication
    // TODO: [ ] Bind original stream callback partial failure.
    pub fn apply_loaded_prefix(&mut self, payload: &ParticleBlendBoolPayload) {
        self.base.apply_loaded_prefix(payload);
        self.item_sources
            .resize(usize::from(self.base.capacity_0d), None);
    }

    // IDA: 006EB880 stores derived +0x30 byte after base refs
    // Completion: 100% of the supported final value stage
    // TODO: [ ] Bind the original short callback result.
    pub const fn apply_loaded_value(&mut self, payload: &ParticleBlendBoolPayload) {
        self.value_30 = payload.value;
    }

    // IDA: 006EB750 dispatches active-count 0, 1 and many; 006EB570 single and 006EB5E0 many. The outer +0x20 last-time store
    // follows each supported result
    // Completion: 85% of the direct nested and clean or two-dirty weights with raw bool +0x30 stores
    // TODO: [ ] Port the dirty > 2 calculation.
    pub fn sample(
        &mut self,
        world: &mut World,
        current_time: f32,
        output: &mut u8,
    ) -> ParticleBlendSampleStatus {
        use ParticleBlendSampleStatus as Status;
        if self.base.active_count_0e == 0 {
            self.base.last_time_08 = current_time;
            return Status::Failed;
        }
        if self.base.active_count_0e == 1 {
            let time = if self.base.flags_0c & 1 != 0 {
                self.base.single_time_20
            } else {
                current_time
            };
            if time == NEGATIVE_MAX {
                self.base.last_time_08 = current_time;
                return Status::Failed;
            }
            let Some(source) = self.single_source_18 else {
                return Status::UnsupportedMissingOwner;
            };
            let mut raw = 0;
            let status = sample_bool_source(world, source, time, &mut raw);
            match status {
                Status::Sampled => {
                    *output = raw;
                    self.value_30 = raw;
                }
                Status::Failed => {
                    *output = 0;
                    self.value_30 = 2;
                }
                _ => return status,
            }
            self.base.last_time_08 = current_time;
            return status;
        }
        if !valid_many_items(&self.base, self.item_sources.len()) {
            return Status::UnsupportedItemMetadata;
        }
        if self.base.flags_0c & 4 != 0
            && (self.base.active_count_0e != 2
                || !recompute_two_weights(&mut self.base, &self.item_sources))
        {
            return Status::UnsupportedDirtyWeights;
        }

        let mut accumulated = 0.0f32;
        let mut remaining = 1.0f32;
        let mut any_sampled = false;
        for index in 0..usize::from(self.base.capacity_0d) {
            let item = self.base.items_14[index];
            // Item +0 is the interpolator pointer: 006CC5C0 and the stream load both set it, and a null one is skipped.
            let Some(source) = self.item_sources[index] else {
                continue;
            };
            if !(item.normalized_weight > 0.0) {
                continue;
            }
            let time = if self.base.flags_0c & 1 != 0 {
                item.runtime_time
            } else {
                current_time
            };
            if time == NEGATIVE_MAX {
                remaining -= item.normalized_weight;
                continue;
            }
            let mut raw = 0;
            let status = sample_bool_source(world, source, time, &mut raw);
            match status {
                Status::Sampled => {
                    any_sampled = true;
                    accumulated += item.normalized_weight * f32::from(raw);
                }
                Status::Failed => {
                    remaining -= item.normalized_weight;
                }
                _ => return status,
            }
        }
        if any_sampled {
            let average = accumulated / remaining;
            self.value_30 = u8::from(average >= 0.5);
            *output = self.value_30;
            self.base.last_time_08 = current_time;
            return Status::Sampled;
        }
        *output = 0;
        self.value_30 = 2;
        self.base.last_time_08 = current_time;
        Status::Failed
    }
}

impl ParticleBlendFloatInterpolator {
    // IDA: 006D2870 reads base scalars before immediate item and single refs; bind refs next, value last
    // Completion: 100% of the supported prefix publication
    // TODO: [ ] Bind original stream callback partial failure.
    pub fn apply_loaded_prefix(&mut self, payload: &ParticleBlendFloatPayload) {
        self.base.apply_loaded_prefix(payload);
        self.item_sources
            .resize(usize::from(self.base.capacity_0d), None);
    }

    // IDA: 006D2870 stores derived +0x30 float after base refs
    // Completion: 100% of the supported final value stage
    // TODO: [ ] Bind the original short callback result.
    pub const fn apply_loaded_value(&mut self, payload: &ParticleBlendFloatPayload) {
        self.value_30 = payload.value;
    }

    // IDA: 006D2740 dispatches active-count 0, 1 and many; 006D2580 single and 006D2600 many. The outer +0x20 last-time store
    // follows each supported result
    // Completion: 85% of the direct nested and clean or two-dirty float with +0x30 intermediate stores
    // TODO: [ ] Port the dirty > 2 calculation.
    pub fn sample(
        &mut self,
        world: &mut World,
        current_time: f32,
        output: &mut f32,
    ) -> ParticleBlendSampleStatus {
        use ParticleBlendSampleStatus as Status;
        if self.base.active_count_0e == 0 {
            self.base.last_time_08 = current_time;
            return Status::Failed;
        }
        if self.base.active_count_0e == 1 {
            let time = if self.base.flags_0c & 1 != 0 {
                self.base.single_time_20
            } else {
                current_time
            };
            if time == NEGATIVE_MAX {
                self.base.last_time_08 = current_time;
                return Status::Failed;
            }
            let Some(source) = self.single_source_18 else {
                if self.base.single_interpolator_index != NULL_REFERENCE {
                    return Status::UnsupportedMissingOwner;
                }
                self.value_30 = NEGATIVE_MAX;
                *output = NEGATIVE_MAX;
                self.base.last_time_08 = current_time;
                return Status::Failed;
            };
            let mut raw = 0.0;
            let status = sample_float_source(world, source, time, &mut raw);
            match status {
                Status::Sampled => {
                    self.value_30 = raw;
                    *output = self.value_30;
                }
                Status::Failed => {
                    self.value_30 = NEGATIVE_MAX;
                    *output = NEGATIVE_MAX;
                }
                _ => return status,
            }
            self.base.last_time_08 = current_time;
            return status;
        }
        if !valid_many_items(&self.base, self.item_sources.len()) {
            return Status::UnsupportedItemMetadata;
        }
        if self.base.flags_0c & 4 != 0
            && (self.base.active_count_0e != 2
                || !recompute_two_weights(&mut self.base, &self.item_sources))
        {
            return Status::UnsupportedDirtyWeights;
        }

        self.value_30 = 0.0;
        let mut remaining = 1.0f32;
        let mut any_sampled = false;
        for index in 0..usize::from(self.base.capacity_0d) {
            let item = self.base.items_14[index];
            // Item +0 is the interpolator pointer: 006CC5C0 and the stream load both set it, and a null one is skipped.
            let Some(source) = self.item_sources[index] else {
                continue;
            };
            if !(item.normalized_weight > 0.0) {
                continue;
            }
            let time = if self.base.flags_0c & 1 != 0 {
                item.runtime_time
            } else {
                current_time
            };
            if time == NEGATIVE_MAX {
                remaining -= item.normalized_weight;
                continue;
            }
            let mut raw = 0.0;
            let status = sample_float_source(world, source, time, &mut raw);
            match status {
                Status::Sampled => {
                    any_sampled = true;
                    self.value_30 += item.normalized_weight * raw;
                }
                Status::Failed => {
                    remaining -= item.normalized_weight;
                }
                _ => return status,
            }
        }
        self.value_30 /= remaining;
        if any_sampled {
            *output = self.value_30;
            self.base.last_time_08 = current_time;
            return Status::Sampled;
        }
        *output = NEGATIVE_MAX;
        self.value_30 = NEGATIVE_MAX;
        self.base.last_time_08 = current_time;
        Status::Failed
    }
}

// IDA: 006CBDF0 / 006CBEE0 invalidate only translate.x (+0), rotate.x (+0x10) and scale (+0x1C)
// Completion: 100% of the three original stores
fn invalidate_transform(value: &mut QuatTransform) {
    value.translate[0] = NEGATIVE_MAX;
    value.rotate[1] = NEGATIVE_MAX;
    value.scale = NEGATIVE_MAX;
}

// IDA: item and single vtable +0x4C transform Update: TransformInterpolator (006D60B0), PathInterpolator (006DC2C0),
// NiBSplineCompTransformInterpolator (006E5990) and nested blend owners
// Completion: 100% of the supported typed owners
// TODO: [ ] Bind other +0x78-true interpolator classes.
pub(crate) fn sample_transform_source(
    world: &mut World,
    source: Entity,
    time: f32,
    output: &mut QuatTransform,
) -> ParticleBlendSampleStatus {
    let status = match world.get::<NiClass>(source) {
        Some(NiClass::BlendTransformInterpolator) => {
            return sample_particle_blend_transform(world, source, time, output);
        }
        Some(NiClass::TransformInterpolator) => {
            update_transform_interpolator(world, source, time, output)
        }
        Some(NiClass::PathInterpolator) => update_path_interpolator(world, source, time, output),
        // 006E5990, a KF's compressed B-spline.
        Some(NiClass::BSplineCompTransformInterpolator) => {
            return match super::bspline::update_comp_transform(world, source, time, output) {
                Ok(true) => ParticleBlendSampleStatus::Sampled,
                Ok(false) => ParticleBlendSampleStatus::Failed,
                Err(_) => ParticleBlendSampleStatus::UnsupportedNestedBranch,
            };
        }
        _ => return ParticleBlendSampleStatus::UnsupportedMissingOwner,
    };
    match status {
        TransformUpdateStatus::Valid => ParticleBlendSampleStatus::Sampled,
        TransformUpdateStatus::Invalid => ParticleBlendSampleStatus::Failed,
        TransformUpdateStatus::UnsupportedOutOfRange => {
            ParticleBlendSampleStatus::UnsupportedNestedBranch
        }
    }
}

impl ParticleBlendTransformInterpolator {
    // IDA: 006CD720 base scalars before immediate item and single refs
    // Completion: 100% of the supported prefix publication
    pub fn apply_loaded_prefix(&mut self, payload: &ParticleBlendTransformPayload) {
        self.base.apply_loaded_prefix(payload);
        self.item_sources
            .resize(usize::from(self.base.capacity_0d), None);
    }

    // IDA: 006CBDF0 single item: the manager flag reads the +0x20 single time; -FLT_MAX invalidates; otherwise the caller's
    // output goes straight to +0x4C Update, and a valid rotation is renormalized by 00715340 before 00471430
    // Completion: 100% of the verified branch order
    fn sample_single(
        &mut self,
        world: &mut World,
        current_time: f32,
        output: &mut QuatTransform,
    ) -> ParticleBlendSampleStatus {
        use ParticleBlendSampleStatus as Status;
        let time = if self.base.flags_0c & 1 != 0 {
            self.base.single_time_20
        } else {
            current_time
        };
        if time == NEGATIVE_MAX {
            invalidate_transform(output);
            return Status::Failed;
        }
        let Some(source) = self.single_source_18 else {
            return Status::UnsupportedMissingOwner;
        };
        let status = sample_transform_source(world, source, time, output);
        if status == Status::Sampled {
            if output.rotate[1] != NEGATIVE_MAX {
                let rotation = normalize_quaternion(&output.rotate);
                set_quat_transform_rotate(output, &rotation);
            }
            return status;
        }
        if status == Status::Failed {
            invalidate_transform(output);
        }
        status
    }

    // IDA: 006CBEE0 many items: weighted translate and scale sums with weights subtracted for invalid parts; quaternions
    // sign-aligned to the running sum (dot < 0 negates), scaled by weight (0072F930) and summed; translate kept only when its
    // weight is nonzero, scale when its weight >= 0.0001 (double), rotation renormalized by 00715340. The B3CBD0 single-item
    // override is 0 during normal startup
    // Completion: 100% of the verified accumulation order and store points
    // TODO: [ ] Model the B3CBD0 override when its two setters are ported.
    fn sample_many(
        &mut self,
        world: &mut World,
        current_time: f32,
        output: &mut QuatTransform,
    ) -> ParticleBlendSampleStatus {
        use ParticleBlendSampleStatus as Status;
        let mut translate_weight = 1.0f32;
        let mut scale_weight = 1.0f32;
        let mut scale_sum = 0.0f32;
        let mut translate_sum = [0.0f32; 3]; // 00B3F9A8 NiPoint3::ZERO
        let mut rotate_sum = [0.0f32; 4];
        let mut translate_valid = false;
        let mut rotate_valid = false;
        let mut scale_valid = false;
        let mut first_rotation = true;
        for index in 0..usize::from(self.base.capacity_0d) {
            let item = self.base.items_14[index];
            // Item +0 is the interpolator pointer: 006CC5C0 and the stream load both set it, and a null one is skipped.
            let Some(source) = self.item_sources[index] else {
                continue;
            };
            if 0.0 >= item.normalized_weight {
                continue;
            }
            let weight = item.normalized_weight;
            let time = if self.base.flags_0c & 1 != 0 {
                item.runtime_time
            } else {
                current_time
            };
            let mut sampled = false;
            if time != NEGATIVE_MAX {
                let mut seed = QuatTransform::default(); // B24260 / B3CBA4 invalid sentinels
                let status = sample_transform_source(world, source, time, &mut seed);
                if status == Status::Sampled {
                    sampled = true;
                    if seed.translate[0] == NEGATIVE_MAX {
                        translate_weight -= weight;
                    } else {
                        translate_valid = true;
                        let x = seed.translate[0] * weight;
                        translate_sum[0] += x;
                        let y = seed.translate[1] * weight;
                        translate_sum[1] += y;
                        let z = weight * seed.translate[2];
                        translate_sum[2] += z;
                    }
                    if seed.rotate[1] != NEGATIVE_MAX {
                        let mut rotation = seed.rotate;
                        if first_rotation {
                            first_rotation = false;
                        } else {
                            let dot = rotation[1] * rotate_sum[1]
                                + rotation[0] * rotate_sum[0]
                                + rotation[2] * rotate_sum[2]
                                + rotation[3] * rotate_sum[3];
                            if dot < 0.0 {
                                rotation = [-rotation[0], -rotation[1], -rotation[2], -rotation[3]];
                            }
                        }
                        let rotation = scale_quaternion(&rotation, weight);
                        rotate_sum[0] += rotation[0];
                        rotate_sum[1] += rotation[1];
                        rotate_sum[2] += rotation[2];
                        rotate_sum[3] += rotation[3];
                        rotate_valid = true;
                    }
                    if seed.scale == NEGATIVE_MAX {
                        scale_weight -= weight;
                    } else {
                        scale_valid = true;
                        scale_sum += seed.scale * weight;
                    }
                } else if status != Status::Failed {
                    return status;
                }
            }
            if !sampled {
                translate_weight -= weight;
                scale_weight -= weight;
            }
        }
        invalidate_transform(output);
        if translate_valid || rotate_valid || scale_valid {
            if translate_weight == 0.0 {
                translate_valid = false;
            }
            if f64::from(scale_weight) < 0.0001 {
                scale_valid = false;
            }
            if translate_valid {
                let inverse = 1.0 / translate_weight;
                set_quat_transform_translate(
                    output,
                    &[
                        inverse * translate_sum[0],
                        translate_sum[1] * inverse,
                        inverse * translate_sum[2],
                    ],
                );
            }
            if rotate_valid {
                let rotation = normalize_quaternion(&rotate_sum);
                set_quat_transform_rotate(output, &rotation);
            }
            if scale_valid {
                set_quat_transform_scale(output, scale_sum / scale_weight);
            }
        }
        if quat_transform_invalid(output) {
            Status::Failed
        } else {
            Status::Sampled
        }
    }

    // IDA: 006CC3A0 dispatches active count 0, 1 and many (006CD0F0 before the many path) and stores +0x08 last time after each
    // original branch. Output is written only where the original writes it (count 0 leaves it untouched)
    // Completion: 90% of the supported single, clean-many and two-dirty branches
    // TODO: [ ] Port dirty > 2 normalization and compare x87 results at runtime.
    pub fn sample(
        &mut self,
        world: &mut World,
        current_time: f32,
        output: &mut QuatTransform,
    ) -> ParticleBlendSampleStatus {
        use ParticleBlendSampleStatus as Status;
        if self.base.active_count_0e == 0 {
            self.base.last_time_08 = current_time;
            return Status::Failed;
        }
        let status = if self.base.active_count_0e == 1 {
            self.sample_single(world, current_time, output)
        } else {
            if !valid_many_items(&self.base, self.item_sources.len()) {
                return Status::UnsupportedItemMetadata;
            }
            if self.base.flags_0c & 4 != 0
                && (self.base.active_count_0e != 2
                    || !recompute_two_weights(&mut self.base, &self.item_sources))
            {
                return Status::UnsupportedDirtyWeights;
            }
            self.sample_many(world, current_time, output)
        };
        if status == Status::Sampled || status == Status::Failed {
            self.base.last_time_08 = current_time;
        }
        status
    }
}

// IDA: N/A detaches a NiBlendBoolInterpolator for 006EB750, whose sources live on other entities
// Completion: 100%
pub fn sample_particle_blend_bool(
    world: &mut World,
    blend: Entity,
    time: f32,
    output: &mut u8,
) -> ParticleBlendSampleStatus {
    let mut state = world
        .get::<ParticleBlendBoolInterpolator>(blend)
        .expect("a NiBlendBoolInterpolator entity")
        .clone();
    let status = state.sample(world, time, output);
    *world
        .get_mut::<ParticleBlendBoolInterpolator>(blend)
        .expect("a NiBlendBoolInterpolator entity") = state;
    status
}

// IDA: N/A detaches a NiBlendFloatInterpolator for 006D2740, whose sources live on other entities
// Completion: 100%
pub fn sample_particle_blend_float(
    world: &mut World,
    blend: Entity,
    time: f32,
    output: &mut f32,
) -> ParticleBlendSampleStatus {
    let mut state = world
        .get::<ParticleBlendFloatInterpolator>(blend)
        .expect("a NiBlendFloatInterpolator entity")
        .clone();
    let status = state.sample(world, time, output);
    *world
        .get_mut::<ParticleBlendFloatInterpolator>(blend)
        .expect("a NiBlendFloatInterpolator entity") = state;
    status
}

// IDA: N/A detaches a NiBlendTransformInterpolator for 006CC3A0, whose sources live on other entities
// Completion: 100%
pub fn sample_particle_blend_transform(
    world: &mut World,
    blend: Entity,
    time: f32,
    output: &mut QuatTransform,
) -> ParticleBlendSampleStatus {
    let mut state = world
        .get::<ParticleBlendTransformInterpolator>(blend)
        .expect("a NiBlendTransformInterpolator entity")
        .clone();
    let status = state.sample(world, time, output);
    *world
        .get_mut::<ParticleBlendTransformInterpolator>(blend)
        .expect("a NiBlendTransformInterpolator entity") = state;
    status
}

/// IDA: N/A the C++ stream graph's two outcomes for an immediate reference at load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamLoadStatus {
    Loaded,
    UnsupportedLinkTarget,
}

// IDA: 006CDC10's item references through 006CD680 before its final +0x18 single reference. A reference to a class outside
// `accepts` stops the load, with the reached prefix kept, as the C++ graph arm does
// Completion: 100% of the reference order
// TODO: [ ] Bind original refcount timing.
fn bind_loaded_references<V, B: BlendInterpolator>(
    world: &World,
    blend: &mut B,
    pending: &ParticleBlendPayload<V>,
    table: &StreamObjectTable,
    accepts: fn(NiClass) -> bool,
    out_of_range: [&'static str; 2],
) -> Result<StreamLoadStatus, StreamError> {
    let resolve = |index: u32, out_of_range: &'static str| -> Result<Option<Entity>, StreamError> {
        let entry = table
            .objects
            .get(index as usize)
            .ok_or(StreamError(out_of_range))?;
        Ok(world
            .get::<NiClass>(entry.entity)
            .filter(|class| accepts(**class))
            .map(|_| entry.entity))
    };
    for (slot, item) in pending.items.iter().enumerate() {
        if item.interpolator_index == NULL_REFERENCE {
            continue;
        }
        let Some(source) = resolve(item.interpolator_index, out_of_range[0])? else {
            return Ok(StreamLoadStatus::UnsupportedLinkTarget);
        };
        blend.bind_item(slot, source)?;
    }
    if pending.single_interpolator_index != NULL_REFERENCE {
        let Some(source) = resolve(pending.single_interpolator_index, out_of_range[1])? else {
            return Ok(StreamLoadStatus::UnsupportedLinkTarget);
        };
        blend.bind_single(source)?;
    }
    Ok(StreamLoadStatus::Loaded)
}

// IDA: 006EB880 on a stream object: prefix, 006CDC10's references, then the +0x30 byte
// Completion: 100% of the C++ graph arm
pub fn load_particle_blend_bool(
    world: &mut World,
    entity: Entity,
    pending: &ParticleBlendBoolPayload,
    table: &StreamObjectTable,
) -> Result<StreamLoadStatus, StreamError> {
    let mut owner = world
        .get::<ParticleBlendBoolInterpolator>(entity)
        .ok_or(StreamError("bool blend component missing"))?
        .clone();
    owner.apply_loaded_prefix(pending);
    let status = bind_loaded_references(
        world,
        &mut owner,
        pending,
        table,
        |class| {
            matches!(
                class,
                NiClass::BoolInterpolator
                    | NiClass::BoolTimelineInterpolator
                    | NiClass::BlendBoolInterpolator
            )
        },
        [
            "bool blend item index out of range",
            "bool blend single index out of range",
        ],
    );
    if matches!(status, Ok(StreamLoadStatus::Loaded)) {
        owner.apply_loaded_value(pending);
    }
    world.entity_mut(entity).insert(owner);
    status
}

// IDA: 006D2870 on a stream object: prefix, 006CDC10's references, then the +0x30 float
// Completion: 100% of the C++ graph arm
pub fn load_particle_blend_float(
    world: &mut World,
    entity: Entity,
    pending: &ParticleBlendFloatPayload,
    table: &StreamObjectTable,
) -> Result<StreamLoadStatus, StreamError> {
    let mut owner = world
        .get::<ParticleBlendFloatInterpolator>(entity)
        .ok_or(StreamError("float blend component missing"))?
        .clone();
    owner.apply_loaded_prefix(pending);
    let status = bind_loaded_references(
        world,
        &mut owner,
        pending,
        table,
        |class| {
            matches!(
                class,
                NiClass::FloatInterpolator | NiClass::BlendFloatInterpolator
            )
        },
        [
            "float blend item index out of range",
            "float blend single index out of range",
        ],
    );
    if matches!(status, Ok(StreamLoadStatus::Loaded)) {
        owner.apply_loaded_value(pending);
    }
    world.entity_mut(entity).insert(owner);
    status
}

// IDA: 006CBD40 on a stream object: prefix, then 006CD720's references; there is no derived value
// Completion: 100% of the C++ graph arm
pub fn load_particle_blend_transform(
    world: &mut World,
    entity: Entity,
    pending: &ParticleBlendTransformPayload,
    table: &StreamObjectTable,
) -> Result<StreamLoadStatus, StreamError> {
    let mut owner = world
        .get::<ParticleBlendTransformInterpolator>(entity)
        .ok_or(StreamError("transform blend component missing"))?
        .clone();
    owner.apply_loaded_prefix(pending);
    let status = bind_loaded_references(
        world,
        &mut owner,
        pending,
        table,
        |class| {
            matches!(
                class,
                NiClass::TransformInterpolator
                    | NiClass::BlendTransformInterpolator
                    | NiClass::PathInterpolator
            )
        },
        [
            "transform blend item index out of range",
            "transform blend single index out of range",
        ],
    );
    world.entity_mut(entity).insert(owner);
    status
}

/// IDA: 006CD0F0 NiBlendInterpolator_ComputeNormalizedWeights: with +0x0C bit 2 set, the bit clears; one active item takes
/// +0x08 = 1.0 at +0x0F; two go through 006CC900 (`recompute_two_weights`). Completion: 100% of one and two active items; more
/// is a host stop (006CD136's general path).
/// # Errors
/// More than two active items, or two whose slots aren't the occupied pair.
pub fn normalize_blend_weights(
    base: &mut ParticleBlendCold,
    sources: &[Option<Entity>],
) -> Result<(), StreamError> {
    if base.flags_0c & 4 == 0 {
        return Ok(());
    }
    match base.active_count_0e {
        1 => {
            base.flags_0c &= 0xFB;
            let item = base
                .items_14
                .get_mut(usize::from(base.single_index_0f))
                .ok_or(StreamError("006CD10F: a single index outside the items"))?;
            item.normalized_weight = 1.0;
            Ok(())
        }
        2 => {
            if recompute_two_weights(base, sources) {
                Ok(())
            } else {
                Err(StreamError(
                    "006CC900: two active items not in the first two occupied slots",
                ))
            }
        }
        _ => Err(StreamError(
            "006CD136: weights over more than two active items",
        )),
    }
}

/// IDA: 006CC550: +0x0F for one active item; otherwise the first item of the +0x0D whose +0x08 weight exceeds every earlier
/// one from 0.0 (strict), else 0xFF. Completion: 100%.
#[must_use]
pub fn dominant_blend_item(base: &ParticleBlendCold) -> u8 {
    if base.active_count_0e == 1 {
        return base.single_index_0f;
    }
    let mut highest = 0.0_f32;
    let mut selected = 0xFF_u8;
    for (index, item) in base
        .items_14
        .iter()
        .enumerate()
        .take(usize::from(base.capacity_0d))
    {
        if highest < item.normalized_weight {
            selected = index as u8;
            highest = item.normalized_weight;
        }
    }
    selected
}

/// IDA: 0074F7F0(item): the single time +0x20 for one active item at +0x0F, otherwise the item's +0x14. Completion: 100%.
#[must_use]
pub fn blend_item_time(base: &ParticleBlendCold, index: u8) -> f32 {
    if base.active_count_0e == 1 && index == base.single_index_0f {
        return base.single_time_20;
    }
    base.items_14
        .get(usize::from(index))
        .map_or(NEGATIVE_MAX, |item| item.runtime_time)
}

/// IDA: 0074FA90 / 0053D850's blend arm: 006CC550's item; 0xFF answers null; one active item at that index answers +0x18,
/// otherwise the item's +0x00 interpolator. Completion: 100%.
#[must_use]
pub fn blend_item_source(
    base: &ParticleBlendCold,
    sources: &[Option<Entity>],
    single: Option<Entity>,
) -> Option<Entity> {
    let index = dominant_blend_item(base);
    if index == 0xFF {
        return None;
    }
    if base.active_count_0e == 1 && index == base.single_index_0f {
        return single;
    }
    sources.get(usize::from(index)).copied().flatten()
}

#[cfg(test)]
mod tests {
    use super::super::animation::FloatInterpolator;
    use super::super::bool_animation::BoolInterpolator;
    use super::super::transform_animation::TransformInterpolator;
    use super::*;

    fn float_source(world: &mut World, value: f32) -> Entity {
        let mut interpolator = FloatInterpolator::default();
        interpolator.load_scalar_and_data(value, None);
        world.spawn((NiClass::FloatInterpolator, interpolator)).id()
    }

    fn bool_source(world: &mut World, value: u8) -> Entity {
        let mut interpolator = BoolInterpolator::default();
        interpolator.load_scalar_and_data(value, None);
        world.spawn((NiClass::BoolInterpolator, interpolator)).id()
    }

    /// Ported from `transform_controller_tests`' `verify_blend_transform_single`.
    #[test]
    fn blend_transform_single() {
        let mut world = World::new();
        let mut empty = ParticleBlendTransformInterpolator::default();
        let mut output = QuatTransform::default();
        assert_eq!(
            empty.sample(&mut world, 4.0, &mut output),
            ParticleBlendSampleStatus::Failed
        );
        assert_eq!(empty.base.last_time_08, 4.0);
        assert!(quat_transform_invalid(&output));

        let fixed = QuatTransform {
            translate: [1.0, 2.0, 3.0],
            rotate: [2.0, 0.0, 0.0, 0.0],
            scale: 3.0,
        };
        let mut source = TransformInterpolator::default();
        source.load_transform_and_data(&fixed, None);
        let source = world.spawn((NiClass::TransformInterpolator, source)).id();
        let mut single = ParticleBlendTransformInterpolator::default();
        single.base.active_count_0e = 1;
        single.base.single_interpolator_index = 0;
        single.bind_single(source).unwrap();
        let mut sampled = QuatTransform::default();
        assert_eq!(
            single.sample(&mut world, 5.0, &mut sampled),
            ParticleBlendSampleStatus::Sampled
        );
        assert_eq!(sampled.translate, fixed.translate);
        assert_eq!(sampled.scale, 3.0);
        assert_eq!(sampled.rotate, [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(single.base.last_time_08, 5.0);
    }

    /// IDA: 006CC5C0 and 006CC6E0. No C++ test covers these directly.
    #[test]
    fn add_and_remove_interp_info() {
        let mut world = World::new();
        let (a, b, c) = (
            world.spawn_empty().id(),
            world.spawn_empty().id(),
            world.spawn_empty().id(),
        );
        let mut blend = ParticleBlendBoolInterpolator::default();
        assert_eq!(blend.add_interp_info(a, 1.0, 2, 0.5), 0);
        assert_eq!(blend.base.capacity_0d, 2, "006CCFD0 grew by 00B242A0");
        assert_eq!(blend.base.single_index_0f, 0);
        assert_eq!(blend.single_source_18, Some(a));
        assert_eq!(blend.base.high_priority_10, 2);
        assert_eq!(blend.base.flags_0c & 4, 4);

        assert_eq!(blend.add_interp_info(b, 1.0, 5, 0.5), 1);
        assert_eq!(blend.base.single_index_0f, 0xFF);
        assert_eq!(blend.single_source_18, None);
        assert_eq!(blend.base.high_priority_10, 5);
        assert_eq!(blend.base.second_priority_11, 2);

        assert_eq!(blend.remove_interp_info(1), Some(b));
        assert_eq!(blend.base.active_count_0e, 1);
        assert_eq!(blend.base.single_index_0f, 0, "capacity 2 picks the other");
        assert_eq!(blend.single_source_18, Some(a));
        assert_eq!(blend.base.high_priority_10, 2);
        assert_eq!(blend.base.second_priority_11, -128);
        assert_eq!(blend.remove_interp_info(0xFF), None);

        assert_eq!(blend.add_interp_info(b, 1.0, 1, 1.0), 1);
        assert_eq!(blend.add_interp_info(c, 1.0, 1, 1.0), 2);
        assert_eq!(blend.base.capacity_0d, 4);
        assert_eq!(blend.remove_interp_info(2), Some(c));
        assert_eq!(blend.remove_interp_info(1), Some(b));
        assert_eq!(blend.base.single_index_0f, 0, "the last occupied slot");
        assert_eq!(blend.remove_interp_info(0), Some(a));
        assert_eq!(blend.base.single_index_0f, 0xFF);
        assert_eq!(blend.single_source_18, None);
        assert_eq!(blend.base.single_time_20, NEGATIVE_MAX);
    }

    fn blend_bytes(value: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8, 1];
        bytes.extend(0.25f32.to_le_bytes());
        bytes.extend([1, 0, 3, 0x80]); // count, single, priorities
        for raw in [1.0f32, 2.0, 3.0, 4.0] {
            bytes.extend(raw.to_le_bytes());
        }
        bytes.extend(7u32.to_le_bytes()); // item reference
        bytes.extend(0.5f32.to_le_bytes());
        bytes.extend(0.75f32.to_le_bytes());
        bytes.push(0xFE);
        bytes.extend(1.0f32.to_le_bytes());
        bytes.extend(9u32.to_le_bytes()); // single reference
        bytes.extend(value);
        bytes
    }

    /// IDA: 006CDC10 and 006CD680's field order. No C++ test covers the byte layout directly.
    #[test]
    fn payload_order() {
        let bytes = blend_bytes(&[1]);
        let parsed = parse_particle_blend_bool_payload(&bytes).unwrap();
        assert_eq!(parsed.bytes_consumed, bytes.len());
        assert_eq!(parsed.capacity, 1);
        assert_eq!(parsed.weight_threshold, 0.25);
        assert_eq!(parsed.high_priority, 3);
        assert_eq!(parsed.second_priority, -128);
        assert_eq!(parsed.high_ease, 4.0);
        assert_eq!(parsed.items[0].interpolator_index, 7);
        assert_eq!(parsed.items[0].priority, -2);
        assert_eq!(parsed.items[0].runtime_time, NEGATIVE_MAX);
        assert_eq!(parsed.single_interpolator_index, 9);
        assert_eq!(parsed.value, 1);
        for length in 0..bytes.len() {
            assert!(parse_particle_blend_bool_payload(&bytes[..length]).is_err());
        }

        let bytes = blend_bytes(&2.5f32.to_le_bytes());
        assert_eq!(
            parse_particle_blend_float_payload(&bytes).unwrap().value,
            2.5
        );

        // Flag 1 stops after the threshold, then reads the derived value.
        let mut manager = vec![1u8, 3];
        manager.extend(0.5f32.to_le_bytes());
        manager.push(1);
        let parsed = parse_particle_blend_bool_payload(&manager).unwrap();
        assert_eq!(parsed.single_index, 0xFF);
        assert!(parsed.items.is_empty());
        assert_eq!(parsed.value, 1);
        assert!(parse_particle_blend_transform_payload(&blend_bytes(&[]), 0x0A01_006D).is_err());
    }

    /// IDA: 006D2740's single and many paths, and 006CD0F0's equal-priority two-item weights.
    #[test]
    fn float_blend_weights() {
        let mut world = World::new();
        let (a, b) = (float_source(&mut world, 2.0), float_source(&mut world, 4.0));
        let mut blend = ParticleBlendFloatInterpolator::default();
        let mut output = 0.0;
        assert_eq!(
            blend.sample(&mut world, 1.0, &mut output),
            ParticleBlendSampleStatus::Failed
        );
        assert_eq!(blend.add_interp_info(a, 1.0, 0, 1.0), 0);
        assert_eq!(
            blend.sample(&mut world, 2.0, &mut output),
            ParticleBlendSampleStatus::Sampled
        );
        assert_eq!(output, 2.0);
        assert_eq!(blend.base.last_time_08, 2.0);

        assert_eq!(blend.add_interp_info(b, 1.0, 0, 1.0), 1);
        assert_eq!(
            blend.sample(&mut world, 3.0, &mut output),
            ParticleBlendSampleStatus::Sampled
        );
        assert_eq!(blend.base.items_14[0].normalized_weight, 0.5);
        assert_eq!(blend.base.items_14[1].normalized_weight, 0.5);
        assert_eq!(blend.base.flags_0c & 4, 0, "006CD0F0 clears the dirty bit");
        assert_eq!(output, 3.0);
    }

    /// IDA: 006CC5C0 stores only the pointer, 006D2600 and 006EB5E0 skip on it and read item +0x14 under flag 1: two
    /// manager-controlled items blend with no stream index.
    #[test]
    fn manager_controlled_items_blend() {
        let mut world = World::new();
        let (a, b) = (float_source(&mut world, 2.0), float_source(&mut world, 4.0));
        let mut blend = ParticleBlendFloatInterpolator::default();
        blend.base.flags_0c = 1;
        blend.base.capacity_0d = 2;
        blend.add_interp_info(a, 1.0, 0, 1.0);
        blend.add_interp_info(b, 1.0, 0, 1.0);
        assert_eq!(blend.base.items_14[0].interpolator_index, NULL_REFERENCE);
        blend.base.items_14[0].runtime_time = 1.0;
        blend.base.items_14[1].runtime_time = 1.0;
        let mut output = 0.0;
        assert_eq!(
            blend.sample(&mut world, 1.0, &mut output),
            ParticleBlendSampleStatus::Sampled
        );
        assert_eq!(output, 3.0);

        // -FLT_MAX in +0x14 takes the item's weight out of the average.
        blend.base.items_14[1].runtime_time = NEGATIVE_MAX;
        assert_eq!(
            blend.sample(&mut world, 1.0, &mut output),
            ParticleBlendSampleStatus::Sampled
        );
        assert_eq!(output, 2.0);

        let (on, off) = (bool_source(&mut world, 1), bool_source(&mut world, 0));
        let mut blend = ParticleBlendBoolInterpolator::default();
        blend.base.flags_0c = 1;
        blend.base.capacity_0d = 2;
        blend.add_interp_info(on, 1.0, 0, 1.0);
        blend.add_interp_info(off, 1.0, 0, 1.0);
        blend.base.items_14[0].runtime_time = 1.0;
        blend.base.items_14[1].runtime_time = 1.0;
        let mut output = 9;
        assert_eq!(
            blend.sample(&mut world, 1.0, &mut output),
            ParticleBlendSampleStatus::Sampled
        );
        assert_eq!(output, 1, "0.5 rounds up");
    }

    /// IDA: 006EB5E0's rounded average and its raw +0x30 stores, and nesting through a blend entity.
    #[test]
    fn bool_blend_average_and_nesting() {
        let mut world = World::new();
        let (on, off) = (bool_source(&mut world, 1), bool_source(&mut world, 0));
        let mut inner = ParticleBlendBoolInterpolator::default();
        inner.add_interp_info(on, 1.0, 0, 1.0);
        let inner = world.spawn((NiClass::BlendBoolInterpolator, inner)).id();

        let mut blend = ParticleBlendBoolInterpolator::default();
        blend.add_interp_info(inner, 1.0, 0, 1.0);
        blend.add_interp_info(off, 1.0, 0, 1.0);
        let mut output = 9;
        assert_eq!(
            blend.sample(&mut world, 1.0, &mut output),
            ParticleBlendSampleStatus::Sampled
        );
        assert_eq!(output, 1, "0.5 rounds up");
        assert_eq!(blend.value_30, 1);
        assert_eq!(
            world
                .get::<ParticleBlendBoolInterpolator>(inner)
                .unwrap()
                .base
                .last_time_08,
            1.0,
            "the nested blend wrote back"
        );
    }

    /// IDA: the C++ graph's class checks at each immediate reference.
    #[test]
    fn load_rejects_the_wrong_source_class() {
        let mut world = World::new();
        let float = float_source(&mut world, 1.0);
        let blend = world
            .spawn((
                NiClass::BlendBoolInterpolator,
                ParticleBlendBoolInterpolator::default(),
            ))
            .id();
        let mut table = StreamObjectTable::default();
        table.push("NiFloatInterpolator", 0, float);
        let mut pending = parse_particle_blend_bool_payload(&blend_bytes(&[1])).unwrap();
        pending.items[0].interpolator_index = 0;
        pending.single_interpolator_index = NULL_REFERENCE;
        assert_eq!(
            load_particle_blend_bool(&mut world, blend, &pending, &table).unwrap(),
            StreamLoadStatus::UnsupportedLinkTarget
        );
        let owner = world.get::<ParticleBlendBoolInterpolator>(blend).unwrap();
        assert_eq!(owner.base.capacity_0d, 1, "the reached prefix stays");
        assert_eq!(owner.value_30, 2, "the value is not reached");
        pending.items[0].interpolator_index = 5;
        assert!(load_particle_blend_bool(&mut world, blend, &pending, &table).is_err());
    }

    /// The times in `controller_sequence_tests`' `verify_blend_interp_info`, which `add_and_remove_interp_info` does not read: IDA:
    /// 006CC5C0 clears +0x20 on a second item; 006CC6E0 clears the removed item's time and moves the survivor's into +0x20.
    #[test]
    fn interp_info_times() {
        let mut world = World::new();
        let (a, b) = (world.spawn_empty().id(), world.spawn_empty().id());
        let mut blend = ParticleBlendTransformInterpolator::default();
        assert_eq!(blend.add_interp_info(a, 0.5, 3, 1.0), 0);
        blend.base.single_time_20 = 7.0;
        assert_eq!(blend.add_interp_info(b, 0.25, 5, 1.0), 1);
        assert!(blend.base.single_time_20 < -1e38);
        blend.base.items_14[1].runtime_time = 4.0;
        assert_eq!(blend.remove_interp_info(0), Some(a));
        assert_eq!(blend.base.single_index_0f, 1);
        assert_eq!(blend.base.single_time_20, 4.0);
        assert!(blend.base.items_14[0].runtime_time < -1e38);
    }

    /// IDA: 006CC900's threshold arm, whose two stores the C++ port has and this file keeps: a second item below the threshold ends at 1.
    #[test]
    fn two_dirty_weights_threshold() {
        let mut world = World::new();
        let (a, b) = (world.spawn_empty().id(), world.spawn_empty().id());
        let mut blend = ParticleBlendFloatInterpolator::default();
        blend.add_interp_info(a, 3.0, 0, 1.0);
        blend.add_interp_info(b, 1.0, 0, 1.0);
        blend.base.weight_threshold_1c = 0.5;
        assert!(recompute_two_weights(&mut blend.base, &blend.item_sources));
        assert_eq!(blend.base.items_14[0].normalized_weight, 0.75);
        assert_eq!(blend.base.items_14[1].normalized_weight, 1.0);
    }
}
