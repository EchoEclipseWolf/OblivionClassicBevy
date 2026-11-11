//! NiPosData. Host values read from the stream; this is not the original 16/64/76-byte key ABI. Only the fields
//! present in the serialized type are meaningful.
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.
//! TODO: [ ] A later phase's worker thread that loads or evaluates these runs at its own control word (workers are not recorded;
//! a thread that keeps the default is 027F, 53-bit). Main-thread behaviour stays until one is reached.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;

const TRUNCATED: &str = "truncated NiPosData payload";

/// IDA: 00B23D84 .data i32 20 (no writer) bounds the 006BC780 / 006C0430 sample loop.
const CURVATURE_SAMPLES: i32 = 20;
/// IDA: 00A00610 stores f32(1.0 / 20) to 00B3C2F4.
const CURVATURE_STEP: f32 = f32::from_bits(0x3D4C_CCCD);
/// IDA: 006BC480 / 006BC560 f64 constants A3D0C0 2.0, A30E48 3.0 and A3F3A0 6.0.
const DERIVATIVE_TWO: f32 = 2.0;
const DERIVATIVE_THREE: f32 = 3.0;
const DERIVATIVE_SIX: f32 = 6.0;

/// IDA: 006BC260 / 006BF880 / 006BCF10 / 006C0A70 dispatch slots. Names for types 2 and 3 stay numeric until their
/// interpolation semantics are verified. The serialized type is a u32, kept raw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PositionKeyType(pub u32);

impl PositionKeyType {
    pub const NONE: Self = Self(0);
    pub const LINEAR: Self = Self(1);
    pub const TYPE2: Self = Self(2);
    pub const TYPE3: Self = Self(3);
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PositionKey {
    pub time: f32,
    pub position: [f32; 3],
    pub type2_first: [f32; 3],
    pub type2_second: [f32; 3],
    pub type3_parameters: [f32; 3],
    /// IDA: 006BC600 / 006C0170 generate three polynomial vectors per segment. For type 3, the first two are also
    /// the incoming and outgoing tangent slots.
    pub coefficient_1: [f32; 3],
    pub coefficient_2: [f32; 3],
    pub coefficient_3: [f32; 3],
    pub type3_incoming: [f32; 3],
}

// IDA: 006BC780 / 006C0430 cross product with f32 component stores, identical to 00498FE0:
// (a.y*b.z - a.z*b.y, a.z*b.x - a.x*b.z, a.x*b.y - a.y*b.x)
// Precision: 24-bit, each product and the difference rounded to a float. Completion: 100% of the verified operands.
fn cross_product(a: &[f32; 3], b: &[f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

// IDA: 006BC780 / 006C0430 squared length stored as f32((x*x + y*y) + z*z)
// Completion: 100% of the verified addition order
fn squared_length(value: &[f32; 3]) -> f32 {
    (value[0] * value[0] + value[1] * value[1]) + value[2] * value[2]
}

// IDA: 006BC260 / 006BF880 / 006BCF10 / 006C0A70 initialized key strides
// Completion: 100% of the verified values for types 1, 2 and 3
// TODO: [ ] Recover additional type table entries as reached.
fn position_stride(key_type: PositionKeyType) -> u32 {
    match key_type {
        PositionKeyType::LINEAR => 16,
        PositionKeyType::TYPE2 => 64,
        PositionKeyType::TYPE3 => 76,
        _ => 0,
    }
}

// IDA: 00709430 Vec3 read: three raw floats.
// Completion: 100% of the read order
fn vector3(reader: &mut ByteReader<'_>) -> Result<[f32; 3], GameError> {
    Ok([
        reader.f32(TRUNCATED)?,
        reader.f32(TRUNCATED)?,
        reader.f32(TRUNCATED)?,
    ])
}

// IDA: 006BC600 post-processes type-2 key segments
// Completion: 100% of the coefficient equations for the 64-byte layout; host float rounding differs from x87
// TODO: [ ] Compare results against an original run on runtime key arrays.
fn derive_type2(keys: &mut [PositionKey]) {
    for index in 0..keys.len().saturating_sub(1) {
        let second = keys[index + 1];
        let first = &mut keys[index];
        first.coefficient_1 = first.type2_second;
        for axis in 0..3 {
            let delta = second.position[axis] - first.position[axis];
            let start = first.type2_second[axis];
            let end = second.type2_first[axis];
            let double_start = start * 2.0;
            let end_plus_start = end + double_start;
            let triple_delta = delta * 3.0;
            first.coefficient_2[axis] = triple_delta - end_plus_start;
            let double_delta = delta * 2.0;
            let tangent_sum = end + start;
            first.coefficient_3[axis] = tangent_sum - double_delta;
        }
    }
}

// IDA: 006BFF30 computes type-3 incoming and outgoing TCB tangents for one key
// Completion: 100% of the coefficient terms and adjacent-time scale on supported finite inputs
// TODO: [ ] Match zero-time interval behaviour.
fn derive_type3_tangent(
    key: &mut PositionKey,
    previous: &[f32; 3],
    next: &[f32; 3],
    previous_interval: f32,
    next_interval: f32,
) {
    let tension_weight = (1.0 - key.type3_parameters[0]) * 0.5;
    let continuity = key.type3_parameters[1];
    let bias = key.type3_parameters[2];
    let one_plus_continuity = 1.0 + continuity;
    let one_minus_continuity = 1.0 - continuity;
    let one_plus_bias = 1.0 + bias;
    let one_minus_bias = 1.0 - bias;
    let continuity_plus_weight = one_plus_continuity * tension_weight;
    let continuity_minus_weight = tension_weight * one_minus_continuity;
    let previous_in = continuity_minus_weight * one_plus_bias;
    let next_in = continuity_plus_weight * one_minus_bias;
    let previous_out = continuity_plus_weight * one_plus_bias;
    let next_out = one_minus_bias * continuity_minus_weight;
    let interval_scale = 2.0 / (previous_interval + next_interval);
    let incoming_scale = previous_interval * interval_scale;
    let outgoing_scale = interval_scale * next_interval;
    for axis in 0..3 {
        let before = key.position[axis] - previous[axis];
        let after = next[axis] - key.position[axis];
        let incoming_previous = before * previous_in;
        let incoming_next = after * next_in;
        let outgoing_previous = before * previous_out;
        let outgoing_next = after * next_out;
        let incoming = incoming_previous + incoming_next;
        let outgoing = outgoing_previous + outgoing_next;
        key.type3_incoming[axis] = incoming * incoming_scale;
        key.coefficient_1[axis] = outgoing * outgoing_scale;
    }
}

// IDA: 006C0170 synthesizes endpoint neighbours, computes interior TCB tangents, then the same cubic segment
// coefficients as 006BC600
// Completion: 100% of the supported field mapping and equations; x87 arithmetic remains open
// TODO: [ ] Verify malformed timestamps and original floating-point rounding.
fn derive_type3(keys: &mut [PositionKey]) {
    if keys.len() < 2 {
        return;
    }
    let count = keys.len();
    for index in 0..count {
        let key = keys[index];
        let mut previous = [0.0; 3];
        let mut next = [0.0; 3];
        let mut previous_interval = 1.0;
        let mut next_interval = 1.0;
        if index == 0 {
            next = keys[1].position;
            for axis in 0..3 {
                let twice = 2.0 * key.position[axis];
                previous[axis] = twice - next[axis];
            }
        } else if index + 1 == count {
            previous = keys[index - 1].position;
            for axis in 0..3 {
                let twice = 2.0 * key.position[axis];
                next[axis] = twice - previous[axis];
            }
        } else {
            previous = keys[index - 1].position;
            next = keys[index + 1].position;
            previous_interval = key.time - (keys[index - 1].time);
            next_interval = keys[index + 1].time - key.time;
        }
        derive_type3_tangent(
            &mut keys[index],
            &previous,
            &next,
            previous_interval,
            next_interval,
        );
    }
    for index in 0..count - 1 {
        let second = keys[index + 1];
        let first = &mut keys[index];
        for axis in 0..3 {
            let delta = second.position[axis] - first.position[axis];
            let start = first.coefficient_1[axis];
            let end = second.type3_incoming[axis];
            let double_start = start * 2.0;
            let end_plus_start = end + double_start;
            let triple_delta = delta * 3.0;
            first.coefficient_2[axis] = triple_delta - end_plus_start;
            let double_delta = delta * 2.0;
            let tangent_sum = start + end;
            first.coefficient_3[axis] = tangent_sum - double_delta;
        }
    }
}

/// IDA: 006D9D40 NiPosData factory initializes +8 / +C / +10 / +14 to zero.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct PositionKeyData {
    key_type: PositionKeyType,
    stride: u32,
    keys: Vec<PositionKey>,
}

impl PositionKeyData {
    // IDA: 006D9E40 positive key replacement and 006D9F90 LoadBinary
    // Completion: 100% of the supported raw stream fields and verified polynomial coefficients
    pub fn new(key_type: PositionKeyType, mut keys: Vec<PositionKey>) -> Result<Self, GameError> {
        let stride = position_stride(key_type);
        if stride == 0 || keys.is_empty() {
            return Err(GameError::unclassified(
                0,
                "unsupported or empty NiPosData keys",
            ));
        }
        if key_type == PositionKeyType::TYPE2 {
            derive_type2(&mut keys);
        } else if key_type == PositionKeyType::TYPE3 {
            derive_type3(&mut keys);
        }
        Ok(Self {
            key_type,
            stride,
            keys,
        })
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn key_type(&self) -> PositionKeyType {
        self.key_type
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn stride(&self) -> u32 {
        self.stride
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub fn keys(&self) -> &[PositionKey] {
        &self.keys
    }

    // IDA: 006BF3E0 / 006BC3A0 / 006BFCD0 type 1 / 2 / 3 interpolation
    // Completion: 100% of the finite ordered-key host domain; x87 rounding remains open
    // TODO: [ ] Compare the runtime search boundary.
    pub fn sample(&self, time: f32, cached_left: &mut u32) -> Option<[f32; 3]> {
        let keys = &self.keys;
        if keys.is_empty() {
            return None;
        }
        if keys.len() == 1 || time == -f32::MAX {
            return Some(keys[0].position);
        }
        if !time.is_finite() {
            return None;
        }
        for (index, key) in keys.iter().enumerate() {
            if !key.time.is_finite() || (index != 0 && key.time <= keys[index - 1].time) {
                return None;
            }
        }
        if time < keys[0].time || time > keys[keys.len() - 1].time {
            return None;
        }
        let mut left = if (*cached_left as usize) < keys.len() - 1 {
            *cached_left as usize
        } else {
            0
        };
        if keys[left].time > time {
            left = 0;
        }
        while left + 1 < keys.len() - 1 && keys[left + 1].time < time {
            left += 1;
        }
        let first = &keys[left];
        let second = &keys[left + 1];
        let ratio = (time - first.time) / (second.time - first.time);
        let result = self.interpolate(left, ratio)?;
        *cached_left = left as u32;
        Some(result)
    }

    // IDA: 006BBBA0 B3D010 per-type slot for keys (left, left+1): 006BF3E0 linear, 006BC3A0 / 006BFCD0 cubic
    // Completion: 100% of the per-segment math shared by `sample` and exact sequence callers
    #[must_use]
    pub fn interpolate(&self, left: usize, ratio: f32) -> Option<[f32; 3]> {
        if left + 1 >= self.keys.len() {
            return None;
        }
        let first = &self.keys[left];
        let second = &self.keys[left + 1];
        let mut result = [0.0; 3];
        for (axis, component) in result.iter_mut().enumerate() {
            if self.key_type == PositionKeyType::LINEAR {
                let complement = 1.0 - ratio;
                let first_part = first.position[axis] * complement;
                let second_part = second.position[axis] * ratio;
                *component = second_part + first_part;
            } else if self.key_type == PositionKeyType::TYPE2
                || self.key_type == PositionKeyType::TYPE3
            {
                let third_part = first.coefficient_3[axis] * ratio;
                let inner = first.coefficient_2[axis] + third_part;
                let second_part = inner * ratio;
                let middle = first.coefficient_1[axis] + second_part;
                let first_part = middle * ratio;
                *component = first.position[axis] + first_part;
            } else {
                return None;
            }
        }
        Some(result)
    }

    // IDA: B3D250 per-type first derivative for keys (left, left+1): 006BF480 linear key1 - key0; 006BC480 Bezier
    // (+1C / +28 / +34) and 006BFDB0 TCB (+28 / +34 / +40). Per axis: f32(c3 * f32(t*3)), f32(c2 * 2), f32 sum,
    // f32(sum * t), f32(c1 + product).
    // Completion: 100% of the verified operand order and f32 store points
    #[must_use]
    pub fn derivative(&self, left: usize, ratio: f32) -> Option<[f32; 3]> {
        if left + 1 >= self.keys.len() {
            return None;
        }
        let first = &self.keys[left];
        let second = &self.keys[left + 1];
        let mut result = [0.0; 3];
        if self.key_type == PositionKeyType::LINEAR {
            for (axis, component) in result.iter_mut().enumerate() {
                *component = second.position[axis] - first.position[axis];
            }
            return Some(result);
        }
        if self.key_type != PositionKeyType::TYPE2 && self.key_type != PositionKeyType::TYPE3 {
            return None;
        }
        let tripled = ratio * DERIVATIVE_THREE;
        for (axis, component) in result.iter_mut().enumerate() {
            let cubic = first.coefficient_3[axis] * tripled;
            let quadratic = first.coefficient_2[axis] * DERIVATIVE_TWO;
            let inner = quadratic + cubic;
            let scaled = inner * ratio;
            *component = first.coefficient_1[axis] + scaled;
        }
        Some(result)
    }

    // IDA: B3D668 per-type second derivative: 006BBB80 copies the zero vector 00B3F9A8 (zero-filled BSS, no writer)
    // for linear keys; 006BC560 / 006BFE90 per axis: f32(c3 * f32(t*6)), f32(c2 * 2), f32 sum.
    // Completion: 100% of the verified operand order and f32 store points
    #[must_use]
    pub fn second_derivative(&self, left: usize, ratio: f32) -> Option<[f32; 3]> {
        if left + 1 >= self.keys.len() {
            return None;
        }
        if self.key_type == PositionKeyType::LINEAR {
            return Some([0.0; 3]);
        }
        if self.key_type != PositionKeyType::TYPE2 && self.key_type != PositionKeyType::TYPE3 {
            return None;
        }
        let first = &self.keys[left];
        let sextupled = ratio * DERIVATIVE_SIX;
        let mut result = [0.0; 3];
        for (axis, component) in result.iter_mut().enumerate() {
            let cubic = first.coefficient_3[axis] * sextupled;
            let quadratic = first.coefficient_2[axis] * DERIVATIVE_TWO;
            *component = quadratic + cubic;
        }
        Some(result)
    }

    // IDA: B3D130 per-type maximum curvature: 006BF3D0 returns 0.0 for linear; 006BC780 / 006C0430 start at 0.0 and
    // a count of one returns it; for each segment i = 0..20, t = f32(i * 0.05f),
    // k = f32(f32(sqrt(f32 |d1 x d2|^2)) / f32 |d1|^2), stored when max < k (NaN never replaces).
    // Completion: 100% of the verified loop bounds, comparison and stores
    // TODO: [ ] Compare results at runtime.
    #[must_use]
    pub fn max_curvature(&self) -> f32 {
        if self.key_type != PositionKeyType::TYPE2 && self.key_type != PositionKeyType::TYPE3 {
            return 0.0;
        }
        let mut maximum = 0.0;
        for left in 0..self.keys.len().saturating_sub(1) {
            for sample in 0..=CURVATURE_SAMPLES {
                let t = sample as f32 * CURVATURE_STEP; // fild, then fmul: 0..=20 is exact
                let (Some(first), Some(second)) =
                    (self.derivative(left, t), self.second_derivative(left, t))
                else {
                    return maximum;
                };
                let product_length = (squared_length(&cross_product(&first, &second))).sqrt();
                let curvature = product_length / squared_length(&first);
                if maximum < curvature {
                    maximum = curvature;
                }
            }
        }
        maximum
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PositionKeyPayload {
    pub data: PositionKeyData,
    pub bytes_consumed: usize,
}

// IDA: 006D9F90: the modern NiObject prefix is empty; count, then type only for a nonzero count; then the per-type
// readers 006BF730 / 006BCDA0 / 006C0980
// Completion: 100% of the bounded modern >=0x0A010072 read order for types 1, 2 and 3
// TODO: [ ] Port additional initialized key types and original short reads.
pub fn decode_position_key_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<PositionKeyPayload, GameError> {
    if stream_version < 0x0A01_0072 {
        return Err(GameError::unclassified(
            0,
            "unsupported NiPosData stream version",
        ));
    }
    let mut reader = ByteReader::new(bytes);
    let count = reader.u32(TRUNCATED)?;
    if count == 0 {
        return Ok(PositionKeyPayload {
            data: PositionKeyData::default(),
            bytes_consumed: reader.position(),
        });
    }
    let key_type = PositionKeyType(reader.u32(TRUNCATED)?);
    if position_stride(key_type) == 0 {
        return Err(GameError::unclassified(0, "unsupported NiPosData key type"));
    }
    let serialized_size: usize = match key_type {
        PositionKeyType::LINEAR => 16,
        PositionKeyType::TYPE2 => 40,
        _ => 28,
    };
    if count as usize > reader.remaining() / serialized_size {
        return Err(GameError::stream("truncated NiPosData keys"));
    }
    let mut keys = Vec::with_capacity(count as usize);
    for _ in 0..count {
        // IDA: 006BC1C0: NiAnimationKey_LoadTime, then 00709430 Vec3.
        let mut key = PositionKey {
            time: reader.f32(TRUNCATED)?,
            position: vector3(&mut reader)?,
            ..PositionKey::default()
        };
        if key_type == PositionKeyType::TYPE2 {
            // IDA: 006BCDA0: two further Vec3 reads, at original +0x10 / +0x1C.
            key.type2_first = vector3(&mut reader)?;
            key.type2_second = vector3(&mut reader)?;
        } else if key_type == PositionKeyType::TYPE3 {
            // IDA: 006C0880: three scalar reads at original +0x10 / +0x14 / +0x18.
            key.type3_parameters = vector3(&mut reader)?;
        }
        keys.push(key);
    }
    Ok(PositionKeyPayload {
        data: PositionKeyData::new(key_type, keys)?,
        bytes_consumed: reader.position(),
    })
}

// IDA: 006D9F90 count-zero keeps prior keys; a positive replacement reaches 006D9E40
// Completion: 100% of the host transaction before publication
// TODO: [ ] Compare original destructive allocation-failure behaviour.
pub fn load_position_key_payload(
    target: &mut PositionKeyData,
    bytes: &[u8],
    stream_version: u32,
) -> Result<usize, GameError> {
    let decoded = decode_position_key_payload(bytes, stream_version)?;
    if !decoded.data.keys().is_empty() {
        *target = decoded.data;
    }
    Ok(decoded.bytes_consumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODERN_VERSION: u32 = 0x1400_0004;

    fn word(output: &mut Vec<u8>, value: u32) {
        output.extend(value.to_le_bytes());
    }

    fn key(output: &mut Vec<u8>, key_type: PositionKeyType, time: f32, position: [f32; 3]) {
        word(output, time.to_bits());
        for value in position {
            word(output, value.to_bits());
        }
        let extra = match key_type {
            PositionKeyType::TYPE2 => 6,
            PositionKeyType::TYPE3 => 3,
            _ => 0,
        };
        for _ in 0..extra {
            word(output, 0.0f32.to_bits());
        }
    }

    fn payload(key_type: PositionKeyType) -> Vec<u8> {
        let mut bytes = Vec::new();
        word(&mut bytes, 2);
        word(&mut bytes, key_type.0);
        key(&mut bytes, key_type, 0.0, [0.0, -0.0, 0.0]);
        key(&mut bytes, key_type, 1.0, [10.0, 20.0, 30.0]);
        bytes
    }

    /// Ported from `position_animation_tests`: IDA 006D9D40 / 006D9F90 and the strides.
    #[test]
    fn load() {
        let mut cold = PositionKeyData::default();
        assert_eq!(cold.key_type(), PositionKeyType::NONE);
        assert_eq!(cold.stride(), 0);
        assert!(cold.keys().is_empty());
        for (key_type, stride, width) in [
            (PositionKeyType::LINEAR, 16, 16),
            (PositionKeyType::TYPE2, 64, 40),
            (PositionKeyType::TYPE3, 76, 28),
        ] {
            let mut bytes = payload(key_type);
            bytes.push(0xA5);
            let decoded = decode_position_key_payload(&bytes, MODERN_VERSION).unwrap();
            assert_eq!(decoded.bytes_consumed, 8 + 2 * width);
            assert_eq!(decoded.data.key_type(), key_type);
            assert_eq!(decoded.data.stride(), stride);
            assert_eq!(decoded.data.keys().len(), 2);
            assert_eq!(decoded.data.keys()[0].position[1].to_bits(), 0x8000_0000);
            cold = decoded.data;
            let original = cold.keys().as_ptr();
            let mut zero = vec![0; 4];
            word(&mut zero, 0xFFFF_FFFF);
            assert_eq!(
                load_position_key_payload(&mut cold, &zero, MODERN_VERSION),
                Ok(4)
            );
            assert_eq!(cold.key_type(), key_type);
            assert_eq!(cold.keys().as_ptr(), original);
        }
    }

    /// IDA: 006BC600 cubic coefficients and 006C0170 endpoint TCB tangents.
    #[test]
    fn samples() {
        for key_type in [
            PositionKeyType::LINEAR,
            PositionKeyType::TYPE2,
            PositionKeyType::TYPE3,
        ] {
            let data = decode_position_key_payload(&payload(key_type), MODERN_VERSION)
                .unwrap()
                .data;
            let mut cached = 100;
            let middle = data.sample(0.5, &mut cached).unwrap();
            assert_eq!(middle[0], 5.0);
            assert_eq!(middle[2], 15.0);
            assert_eq!(cached, 0);
            assert_eq!(data.sample(1.0, &mut cached).unwrap()[0], 10.0);
            assert_eq!(data.sample(-f32::MAX, &mut cached).unwrap()[0], 0.0);
            assert_eq!(cached, 0);
            assert_eq!(data.sample(-1.0, &mut cached), None);
            assert_eq!(data.sample(f32::NAN, &mut cached), None);
        }
        let cubic = decode_position_key_payload(&payload(PositionKeyType::TYPE2), MODERN_VERSION)
            .unwrap()
            .data;
        assert_eq!(cubic.keys()[0].coefficient_1[0], 0.0);
        assert_eq!(cubic.keys()[0].coefficient_2[0], 30.0);
        assert_eq!(cubic.keys()[0].coefficient_3[0], -20.0);
    }

    /// IDA: 006D9F90's supported version, count and type dispatch and bounded reads.
    #[test]
    fn rejections() {
        let bytes = payload(PositionKeyType::TYPE3);
        for length in 0..bytes.len() {
            assert!(decode_position_key_payload(&bytes[..length], MODERN_VERSION).is_err());
        }
        let mut unknown = bytes.clone();
        unknown[4] = 5;
        assert!(decode_position_key_payload(&unknown, MODERN_VERSION).is_err());
        assert!(decode_position_key_payload(&bytes, 0x0A01_0071).is_err());
        let mut existing =
            decode_position_key_payload(&payload(PositionKeyType::LINEAR), MODERN_VERSION)
                .unwrap()
                .data;
        assert!(load_position_key_payload(&mut existing, &bytes[..12], MODERN_VERSION).is_err());
        assert_eq!(existing.key_type(), PositionKeyType::LINEAR);
        assert_eq!(existing.keys().len(), 2);
    }
}
