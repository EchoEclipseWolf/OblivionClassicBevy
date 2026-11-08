//! NiFloatData and NiFloatInterpolator. Key algorithms are plain functions over the value types; a linked
//! NiFloatData is another entity, resolved by `update_float_interpolator`.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

/// IDA: the "no value" sentinel in NiFloatInterpolator's time and value (-FLT_MAX).
pub const INVALID_FLOAT_INTERPOLATOR_VALUE: f32 = -f32::MAX;

/// The serialized key type, kept raw so that an unregistered value stays representable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FloatKeyType(pub u8);

impl FloatKeyType {
    pub const NONE: Self = Self(0);
    pub const LINEAR: Self = Self(1);
    pub const QUADRATIC: Self = Self(2);
    pub const TBC: Self = Self(3);
    pub const STEP: Self = Self(5);
}

/// Host-owned key representation; these fields are not an original NiFloatKey ABI.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FloatKey {
    pub time: f32,
    pub value: f32,
    pub in_tangent: f32,
    pub out_tangent: f32,
    pub tbc_in_tangent: f32,
    pub tbc_out_tangent: f32,
    pub tension: f32,
    pub continuity: f32,
    pub bias: f32,
}

// IDA: 006D3690 linear scalar interpolation
// Completion: 100% of the bounded scalar formula
// TODO: [ ] Compare x87 rounding against the original when bit identity is needed.
fn linear_value(ratio: f32, first: f32, second: f32) -> f32 {
    ratio * second + (1.0 - ratio) * first
}

// IDA: 006D36B0 Hermite scalar interpolation
// Completion: 100% of the bounded cubic formula
// TODO: [ ] Compare x87 rounding against original machine instructions.
fn hermite_value(ratio: f32, first: f32, outgoing: f32, second: f32, incoming: f32) -> f32 {
    let delta = second - first;
    let cubic = outgoing + incoming - 2.0 * delta;
    let quadratic = 3.0 * delta - (incoming + 2.0 * outgoing);
    ratio * (outgoing + (cubic * ratio + quadratic) * ratio) + first
}

// IDA: N/A host input guard for 006BB270
// Completion: 100% of the supported finite key contract
// TODO: [ ] Recover original handling of unsorted and nonfinite key streams.
fn supported_type(key_type: FloatKeyType) -> bool {
    matches!(
        key_type,
        FloatKeyType::LINEAR | FloatKeyType::QUADRATIC | FloatKeyType::TBC | FloatKeyType::STEP
    )
}

/// IDA: 006E3440 NiFloatData. Cold: no keys, type 0.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct FloatKeyData {
    key_type: FloatKeyType,
    keys: Vec<FloatKey>,
}

impl FloatKeyData {
    // IDA: N/A a validated host key owner over 006E3440's cold fields
    // Completion: 100% for supported sorted finite input
    // TODO: [ ] Investigate original malformed-key and out-of-range behaviour.
    pub fn new(key_type: FloatKeyType, keys: Vec<FloatKey>) -> Result<Self, StreamError> {
        if !supported_type(key_type) || keys.is_empty() {
            return Err(StreamError("unsupported or empty float key data"));
        }
        for (index, key) in keys.iter().enumerate() {
            if !key.time.is_finite()
                || !key.value.is_finite()
                || !key.in_tangent.is_finite()
                || !key.out_tangent.is_finite()
                || !key.tbc_in_tangent.is_finite()
                || !key.tbc_out_tangent.is_finite()
                || !key.tension.is_finite()
                || !key.continuity.is_finite()
                || !key.bias.is_finite()
                || (index != 0 && key.time <= keys[index - 1].time)
            {
                return Err(StreamError(
                    "float keys must be finite and strictly time sorted",
                ));
            }
        }
        Ok(Self { key_type, keys })
    }

    // IDA: 006C1650 TBC endpoint mirroring and interior neighbours, 006C1550 tangents
    // Completion: 100% of the recovered formulas on validated finite host tracks
    // TODO: [ ] Compare generated values with original x87 results on loaded assets.
    pub fn with_generated_tbc(keys: Vec<FloatKey>) -> Result<Self, StreamError> {
        let mut result = Self::new(FloatKeyType::TBC, keys)?;
        if result.keys.len() < 2 {
            return Ok(result);
        }
        let count = result.keys.len();
        for index in 0..count {
            let key = result.keys[index];
            let first = index == 0;
            let last = index + 1 == count;
            let previous_value = if first {
                2.0 * key.value - result.keys[1].value
            } else {
                result.keys[index - 1].value
            };
            let next_value = if last {
                2.0 * key.value - result.keys[index - 1].value
            } else {
                result.keys[index + 1].value
            };
            let previous_dt = if first || last {
                1.0
            } else {
                key.time - result.keys[index - 1].time
            };
            let next_dt = if first || last {
                1.0
            } else {
                result.keys[index + 1].time - key.time
            };
            let previous_difference = key.value - previous_value;
            let next_difference = next_value - key.value;
            let half = (1.0 - key.tension) * 0.5;
            let plus_continuity = (1.0 + key.continuity) * half;
            let minus_continuity = (1.0 - key.continuity) * half;
            let plus_bias = 1.0 + key.bias;
            let minus_bias = 1.0 - key.bias;
            let factor = 2.0 / (previous_dt + next_dt);
            let tbc_in = previous_dt
                * factor
                * (plus_continuity * minus_bias * next_difference
                    + minus_continuity * plus_bias * previous_difference);
            let tbc_out = next_dt
                * factor
                * (plus_continuity * plus_bias * previous_difference
                    + minus_continuity * minus_bias * next_difference);
            result.keys[index].tbc_in_tangent = tbc_in;
            result.keys[index].tbc_out_tangent = tbc_out;
            if !tbc_in.is_finite() || !tbc_out.is_finite() {
                return Err(StreamError("generated TBC tangents must be finite"));
            }
        }
        Ok(result)
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn key_type(&self) -> FloatKeyType {
        self.key_type
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn keys(&self) -> &[FloatKey] {
        &self.keys
    }

    // IDA: 006BB270 keyed sampler and 006D3690 / 006D36B0 scalar helpers
    // Completion: 100% for supported key types and validated in-range times; no original ABI claim
    // TODO: [ ] Recover original extrapolation and malformed-storage behaviour.
    pub fn sample(&self, time: f32, cached_index: &mut u32) -> Option<f32> {
        let keys = &self.keys;
        if keys.is_empty() {
            return None;
        }
        if keys.len() == 1 || time == INVALID_FLOAT_INTERPOLATOR_VALUE {
            return Some(keys[0].value);
        }
        if !supported_type(self.key_type)
            || !time.is_finite()
            || time < keys[0].time
            || time > keys[keys.len() - 1].time
        {
            return None;
        }
        let mut first = if (*cached_index as usize) < keys.len() - 1 {
            *cached_index as usize
        } else {
            0
        };
        if keys[first].time > time {
            first = 0;
        }
        while first + 1 < keys.len() - 1 && keys[first + 1].time < time {
            first += 1;
        }
        let left = &keys[first];
        let right = &keys[first + 1];
        let ratio = (time - left.time) / (right.time - left.time);
        let result = self.interpolate(first, ratio)?;
        *cached_index = first as u32;
        Some(result)
    }

    // IDA: 006BB270 B3CFF8 per-type slot on keys (left, left+1): 006D3690 linear, 006D36B0 quadratic and TBC
    // Hermite, ratio >= 1 step
    // Completion: 100% of the per-type math shared by `sample` and exact sequence callers
    // TODO: [ ] Compare x87 rounding against original machine instructions.
    #[must_use]
    pub fn interpolate(&self, left_index: usize, ratio: f32) -> Option<f32> {
        if left_index + 1 >= self.keys.len() {
            return None;
        }
        let left = &self.keys[left_index];
        let right = &self.keys[left_index + 1];
        match self.key_type {
            FloatKeyType::LINEAR => Some(linear_value(ratio, left.value, right.value)),
            FloatKeyType::STEP => Some(if ratio >= 1.0 {
                right.value
            } else {
                left.value
            }),
            FloatKeyType::QUADRATIC => Some(hermite_value(
                ratio,
                left.value,
                left.out_tangent,
                right.value,
                right.in_tangent,
            )),
            FloatKeyType::TBC => Some(hermite_value(
                ratio,
                left.value,
                left.tbc_out_tangent,
                right.value,
                right.tbc_in_tangent,
            )),
            _ => None,
        }
    }
}

/// IDA: 006D2DD0 NiFloatInterpolator via 006EC220 / 006EBA00: the observed cold scalar state.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct FloatInterpolator {
    pub last_time_08: f32,
    pub cached_value_0c: f32,
    pub data_10: Option<Entity>,
    pub last_key_index: u32,
}

impl Default for FloatInterpolator {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            last_time_08: INVALID_FLOAT_INTERPOLATOR_VALUE,
            cached_value_0c: INVALID_FLOAT_INTERPOLATOR_VALUE,
            data_10: None,
            last_key_index: 0,
        }
    }
}

impl FloatInterpolator {
    // IDA: N/A host owner assignment
    // Completion: 100% of the owner replacement in this module
    // TODO: [ ] Verify original SetData cache invalidation and refcount operations.
    pub fn set_data(&mut self, data: Option<Entity>) {
        self.data_10 = data;
        self.last_time_08 = INVALID_FLOAT_INTERPOLATOR_VALUE;
        self.cached_value_0c = INVALID_FLOAT_INTERPOLATOR_VALUE;
        self.last_key_index = 0;
    }

    // IDA: 006D2F60 LoadBinary writes value +0x0C, then retains data +0x10
    // Completion: 100% of the host scalar and owner assignment, without changing last time or key index
    // TODO: [ ] Compare original refcount timing on already populated objects.
    pub fn load_scalar_and_data(&mut self, value: f32, data: Option<Entity>) {
        self.cached_value_0c = value;
        self.data_10 = data;
    }

    // IDA: 006D2CA0 Update(float, unused, float*)
    // Completion: 100% of the bounded cache and output branches. `data` is what `data_10` links to.
    // TODO: [ ] Validate original runtime behaviour for malformed key bounds.
    pub fn update(&mut self, data: Option<&FloatKeyData>, time: f32, output: &mut f32) -> bool {
        if time == self.last_time_08 {
            *output = self.cached_value_0c;
            return self.cached_value_0c != INVALID_FLOAT_INTERPOLATOR_VALUE;
        }
        if let Some(data) = data {
            if let Some(value) = data.sample(time, &mut self.last_key_index) {
                self.cached_value_0c = value;
            } else if !data.keys().is_empty() {
                return false;
            }
        }
        if self.cached_value_0c == INVALID_FLOAT_INTERPOLATOR_VALUE {
            return false;
        }
        *output = self.cached_value_0c;
        self.last_time_08 = time;
        true
    }

    // IDA: 006D2A80 Collapse
    // Completion: 100% of the zero, singleton and equal linear-or-step branches
    // TODO: [ ] Recover original reference-count and key-format edge cases, and NaN key values.
    pub fn collapse(&mut self, data: Option<&FloatKeyData>) {
        let Some(data) = data else {
            return;
        };
        let keys = data.keys();
        if keys.is_empty() {
            self.data_10 = None;
            self.cached_value_0c = INVALID_FLOAT_INTERPOLATOR_VALUE;
            return;
        }
        if keys.len() > 1 {
            if data.key_type() != FloatKeyType::LINEAR && data.key_type() != FloatKeyType::STEP {
                return;
            }
            if keys.iter().any(|key| key.value != keys[0].value) {
                return;
            }
        }
        self.cached_value_0c = keys[0].value;
        self.data_10 = None;
    }
}

// IDA: N/A resolves +0x10 for 006D2CA0. The link check guarantees the class, so a dangling link is a host defect.
// Completion: 100%
pub fn update_float_interpolator(
    world: &mut World,
    interpolator: Entity,
    time: f32,
    output: &mut f32,
) -> bool {
    let mut state = *world
        .get::<FloatInterpolator>(interpolator)
        .expect("a NiFloatInterpolator entity");
    let data = state
        .data_10
        .map(|data| world.get::<FloatKeyData>(data).expect("linked NiFloatData"));
    let result = state.update(data, time, output);
    *world
        .get_mut::<FloatInterpolator>(interpolator)
        .expect("a NiFloatInterpolator entity") = state;
    result
}

// IDA: N/A resolves +0x10 for 006D2A80.
// Completion: 100%
pub fn collapse_float_interpolator(world: &mut World, interpolator: Entity) {
    let mut state = *world
        .get::<FloatInterpolator>(interpolator)
        .expect("a NiFloatInterpolator entity");
    let data = state
        .data_10
        .map(|data| world.get::<FloatKeyData>(data).expect("linked NiFloatData"));
    state.collapse(data);
    *world
        .get_mut::<FloatInterpolator>(interpolator)
        .expect("a NiFloatInterpolator entity") = state;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(left: f32, right: f32) -> bool {
        (left - right).abs() < 0.0001
    }

    fn key(time: f32, value: f32) -> FloatKey {
        FloatKey {
            time,
            value,
            ..FloatKey::default()
        }
    }

    fn set(world: &mut World, interpolator: Entity, data: Option<Entity>) {
        world
            .get_mut::<FloatInterpolator>(interpolator)
            .unwrap()
            .set_data(data);
    }

    fn state(world: &World, interpolator: Entity) -> FloatInterpolator {
        *world.get::<FloatInterpolator>(interpolator).unwrap()
    }

    fn spawn_keys(world: &mut World, key_type: FloatKeyType, keys: Vec<FloatKey>) -> Entity {
        world.spawn(FloatKeyData::new(key_type, keys).unwrap()).id()
    }

    /// Ported from `animation_tests`: IDA 006D2DD0 / 006E3440 cold factories and 006D2CA0 update branches.
    #[test]
    fn cold_and_update() {
        let cold_data = FloatKeyData::default();
        assert_eq!(cold_data.key_type(), FloatKeyType::NONE);
        assert!(cold_data.keys().is_empty());
        let mut world = World::new();
        let interpolator = world.spawn(FloatInterpolator::default()).id();
        let cold = state(&world, interpolator);
        assert_eq!(cold.last_time_08, INVALID_FLOAT_INTERPOLATOR_VALUE);
        assert_eq!(cold.cached_value_0c, INVALID_FLOAT_INTERPOLATOR_VALUE);
        assert!(cold.data_10.is_none());
        assert_eq!(cold.last_key_index, 0);

        let mut output = 41.0;
        assert!(!update_float_interpolator(
            &mut world,
            interpolator,
            0.0,
            &mut output
        ));
        assert_eq!(output, 41.0);
        assert_eq!(
            state(&world, interpolator).last_time_08,
            INVALID_FLOAT_INTERPOLATOR_VALUE
        );
        assert!(!update_float_interpolator(
            &mut world,
            interpolator,
            INVALID_FLOAT_INTERPOLATOR_VALUE,
            &mut output
        ));
        assert_eq!(output, INVALID_FLOAT_INTERPOLATOR_VALUE);

        let data = spawn_keys(
            &mut world,
            FloatKeyType::LINEAR,
            vec![key(0.0, 2.0), key(2.0, 6.0)],
        );
        set(&mut world, interpolator, Some(data));
        assert!(update_float_interpolator(
            &mut world,
            interpolator,
            1.0,
            &mut output
        ));
        assert!(near(output, 4.0));
        assert_eq!(state(&world, interpolator).last_time_08, 1.0);
        output = 99.0;
        assert!(update_float_interpolator(
            &mut world,
            interpolator,
            1.0,
            &mut output
        ));
        assert!(near(output, 4.0));
        output = 97.0;
        assert!(!update_float_interpolator(
            &mut world,
            interpolator,
            3.0,
            &mut output
        ));
        assert_eq!(output, 97.0);
        assert_eq!(state(&world, interpolator).last_time_08, 1.0);
    }

    /// IDA: 006BB270 cached forward and backward search and 006C23C0 step threshold.
    #[test]
    fn sampling() {
        let linear = FloatKeyData::new(
            FloatKeyType::LINEAR,
            vec![key(0.0, 0.0), key(1.0, 10.0), key(2.0, 20.0)],
        )
        .unwrap();
        let mut index = 0;
        assert!(near(linear.sample(1.5, &mut index).unwrap(), 15.0));
        assert_eq!(index, 1);
        assert!(near(linear.sample(0.5, &mut index).unwrap(), 5.0));
        assert_eq!(index, 0);
        let step =
            FloatKeyData::new(FloatKeyType::STEP, vec![key(0.0, 3.0), key(1.0, 7.0)]).unwrap();
        assert_eq!(step.sample(0.999, &mut index), Some(3.0));
        assert_eq!(step.sample(1.0, &mut index), Some(7.0));
        let singleton = FloatKeyData::new(FloatKeyType::LINEAR, vec![key(5.0, 8.0)]).unwrap();
        assert_eq!(
            singleton.sample(INVALID_FLOAT_INTERPOLATOR_VALUE, &mut index),
            Some(8.0)
        );
    }

    /// IDA: 006BB6B0 / 006C14C0 and 006D36B0 Hermite evaluation.
    #[test]
    fn hermite() {
        let mut left = key(0.0, 0.0);
        let right = FloatKey {
            in_tangent: 20.0,
            tbc_in_tangent: 4.0,
            ..key(1.0, 10.0)
        };
        let quadratic = FloatKeyData::new(FloatKeyType::QUADRATIC, vec![left, right]).unwrap();
        let mut index = 0;
        assert!(near(quadratic.sample(0.0, &mut index).unwrap(), 0.0));
        assert!(near(quadratic.sample(0.5, &mut index).unwrap(), 2.5));
        assert!(near(quadratic.sample(1.0, &mut index).unwrap(), 10.0));
        left.tbc_out_tangent = 16.0;
        let tbc = FloatKeyData::new(FloatKeyType::TBC, vec![left, right]).unwrap();
        assert!(near(tbc.sample(0.5, &mut index).unwrap(), 6.5));
    }

    /// IDA: 006C1650 endpoint mirrors and interior neighbour selection, 006C1550 formula.
    #[test]
    fn generated_tbc() {
        let first = FloatKey {
            tension: 0.2,
            continuity: 0.3,
            bias: -0.4,
            ..key(0.0, 1.0)
        };
        let middle = FloatKey {
            tension: -0.25,
            continuity: 0.5,
            bias: 0.2,
            ..key(2.0, 5.0)
        };
        let last = FloatKey {
            tension: 0.1,
            continuity: -0.2,
            bias: 0.6,
            ..key(5.0, 9.0)
        };
        let data = FloatKeyData::with_generated_tbc(vec![first, middle, last]).unwrap();
        assert!(near(data.keys()[0].tbc_in_tangent, 3.584));
        assert!(near(data.keys()[0].tbc_out_tangent, 2.816));
        assert!(near(data.keys()[1].tbc_in_tangent, 3.6));
        assert!(near(data.keys()[1].tbc_out_tangent, 6.6));
        assert!(near(data.keys()[2].tbc_in_tangent, 4.032));
        assert!(near(data.keys()[2].tbc_out_tangent, 3.168));
        let mut index = 0;
        assert!(near(data.sample(3.5, &mut index).unwrap(), 7.321));
        assert_eq!(index, 1);
        let singleton = FloatKey {
            tbc_in_tangent: 11.0,
            tbc_out_tangent: 22.0,
            ..key(0.0, 5.0)
        };
        let single = FloatKeyData::with_generated_tbc(vec![singleton]).unwrap();
        assert_eq!(single.keys()[0].tbc_in_tangent, 11.0);
        assert_eq!(single.keys()[0].tbc_out_tangent, 22.0);
    }

    /// IDA: 006D2A80 collapse. The C++ `weak_ptr::expired` checks have no entity equivalent.
    #[test]
    fn collapse() {
        let mut world = World::new();
        let interpolator = world.spawn(FloatInterpolator::default()).id();
        let empty = world.spawn(FloatKeyData::default()).id();
        set(&mut world, interpolator, Some(empty));
        collapse_float_interpolator(&mut world, interpolator);
        assert!(state(&world, interpolator).data_10.is_none());
        assert_eq!(
            state(&world, interpolator).cached_value_0c,
            INVALID_FLOAT_INTERPOLATOR_VALUE
        );

        let single = spawn_keys(&mut world, FloatKeyType::LINEAR, vec![key(0.0, 12.0)]);
        set(&mut world, interpolator, Some(single));
        let mut output = 0.0;
        assert!(update_float_interpolator(
            &mut world,
            interpolator,
            4.0,
            &mut output
        ));
        assert_eq!(output, 12.0);
        let previous = state(&world, interpolator);
        collapse_float_interpolator(&mut world, interpolator);
        let collapsed = state(&world, interpolator);
        assert!(collapsed.data_10.is_none());
        assert_eq!(collapsed.cached_value_0c, 12.0);
        assert_eq!(collapsed.last_time_08, previous.last_time_08);
        assert_eq!(collapsed.last_key_index, previous.last_key_index);

        let equal = spawn_keys(
            &mut world,
            FloatKeyType::STEP,
            vec![key(0.0, 7.0), key(1.0, 7.0)],
        );
        set(&mut world, interpolator, Some(equal));
        collapse_float_interpolator(&mut world, interpolator);
        assert!(state(&world, interpolator).data_10.is_none());
        assert_eq!(state(&world, interpolator).cached_value_0c, 7.0);

        let unequal = spawn_keys(
            &mut world,
            FloatKeyType::LINEAR,
            vec![key(0.0, 1.0), key(1.0, 2.0)],
        );
        set(&mut world, interpolator, Some(unequal));
        collapse_float_interpolator(&mut world, interpolator);
        assert!(state(&world, interpolator).data_10.is_some());
        let tbc = spawn_keys(
            &mut world,
            FloatKeyType::TBC,
            vec![key(0.0, 1.0), key(1.0, 1.0)],
        );
        set(&mut world, interpolator, Some(tbc));
        collapse_float_interpolator(&mut world, interpolator);
        assert!(state(&world, interpolator).data_10.is_some());
    }

    /// IDA: N/A host supported-input validation for 006BB270.
    #[test]
    fn input_rejection() {
        assert!(FloatKeyData::new(FloatKeyType(4), vec![key(0.0, 1.0)]).is_err());
        assert!(
            FloatKeyData::new(FloatKeyType::LINEAR, vec![key(1.0, 1.0), key(0.0, 2.0)]).is_err()
        );
    }
}
