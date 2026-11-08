//! NiTransformData, NiQuatTransform and NiTransformInterpolator, with the NiQuaternion helpers they reach.
//! Quaternions keep the original memory order w, x, y, z.
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.
//! TODO: [ ] A later phase's worker thread that loads or evaluates these runs at its own control word (workers are not recorded;
//! a thread that keeps the default is 027F, 53-bit). Main-thread behaviour stays until one is reached.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};
use recomp_core::x87;

use super::animation::{FloatKey, FloatKeyData};
use super::animation_stream::decode_float_key_payload;
use super::position_animation::{PositionKeyData, decode_position_key_payload};

const TRUNCATED: &str = "truncated NiTransformData payload";

/// IDA: 00B24260 static (-FLT_MAX x3) translate and 00A026F0 initializer (-FLT_MAX x4) for 00B3CBA4 rotate; the
/// scale sentinel is -FLT_MAX.
pub const INVALID_TRANSFORM_COMPONENT: f32 = -f32::MAX;

type Quaternion = [f32; 4];

/// IDA: 00A09B90 static initializer stores (float)atan(1.0) * 4.0 to 00B3F9A4, read by 00714DB0 for w <= -1.
const LOG_CLAMP_ANGLE: f32 = f32::from_bits(0x4049_0FDB);
/// IDA: 00A7EAB0 f32 0.001 cutoff compared against |sin| in 00714DB0 / 0072F9F0.
const SINE_CUTOFF: f32 = f32::from_bits(0x3A83_126F);
/// IDA: 00A7EAB4 f32 -0.25 scale in 00714E70 NiQuaternion::Intermediate.
const INTERMEDIATE_SCALE: f32 = f32::from_bits(0xBE80_0000);
/// IDA: 00A3D65C f32 0.5 scale in 006C0C40 TCB tangent quaternions.
const TCB_HALF: f32 = f32::from_bits(0x3F00_0000);

// IDA: 00714CF0 thiscall product out = this * rhs with f32 component stores
// Completion: 100% of the verified component expressions and operand order
// Precision: 24-bit per step, with the left-to-right chains the instructions show.
fn multiply(lhs: &Quaternion, rhs: &Quaternion) -> Quaternion {
    multiply_quaternions(lhs, rhs)
}

/// IDA: 00714CF0 NiQuaternion_Multiply(this, out, rhs): out = this * rhs. Completion: 100%.
#[must_use]
pub fn multiply_quaternions(lhs: &Quaternion, rhs: &Quaternion) -> Quaternion {
    let (lw, lx, ly, lz) = (lhs[0], lhs[1], lhs[2], lhs[3]);
    let (rw, rx, ry, rz) = (rhs[0], rhs[1], rhs[2], rhs[3]);
    [
        rw * lw - rx * lx - ry * ly - rz * lz,
        lx * rw + rx * lw + rz * ly - lz * ry,
        ly * rw + ry * lw + rx * lz - rz * lx,
        lz * rw + rz * lw + lx * ry - rx * ly,
    ]
}

// IDA: 00714D80 conjugate (w, -x, -y, -z); 00714E70 inlines the same stores
// Completion: 100% of the sign-bit operation
fn conjugate(value: &Quaternion) -> Quaternion {
    [value[0], -value[1], -value[2], -value[3]]
}

// IDA: 00714CC0 negate all four components
// Completion: 100% of the sign-bit operation
fn negate(value: &Quaternion) -> Quaternion {
    [-value[0], -value[1], -value[2], -value[3]]
}

// IDA: 00714C60 thiscall out = rhs + this per component
// Completion: 100% of the operand order
fn add(self_value: &Quaternion, rhs: &Quaternion) -> Quaternion {
    [
        rhs[0] + self_value[0],
        rhs[1] + self_value[1],
        rhs[2] + self_value[2],
        rhs[3] + self_value[3],
    ]
}

// IDA: 00714C90 thiscall out = this - rhs per component
// Completion: 100% of the operand order
fn subtract(self_value: &Quaternion, rhs: &Quaternion) -> Quaternion {
    [
        self_value[0] - rhs[0],
        self_value[1] - rhs[1],
        self_value[2] - rhs[2],
        self_value[3] - rhs[3],
    ]
}

// IDA: 0072F930 thiscall quaternion * float: z, y, x products, then s * w, each stored to f32
// Completion: 100% of the verified store order
#[must_use]
pub fn scale_quaternion(value: &[f32; 4], factor: f32) -> [f32; 4] {
    let z = value[3] * factor;
    let y = value[2] * factor;
    let x = value[1] * factor;
    let w = factor * value[0];
    [w, x, y, z]
}

// IDA: 0072F990 scalar * quaternion; z, y, x, then w products are stored before 00714C40 writes (w, x, y, z)
// Completion: 100% of the store order and widths
fn scale(factor: f32, value: &Quaternion) -> Quaternion {
    scale_quaternion(value, factor)
}

// IDA: 00714DB0 NiQuaternion::Log. w <= -1 uses 00B3F9A4, w >= 1 uses 0, otherwise CRT acos; CRT sin;
// |sin| >= 0.001f selects angle/sin.
// Completion: 95% of the verified branches and f32 stores; CRT result bits may differ
// TODO: [ ] Compare CRT __CIacos / __CIsin results against the host library.
fn quaternion_log(value: &Quaternion) -> Quaternion {
    let w = value[0];
    let angle = if w <= -1.0 {
        LOG_CLAMP_ANGLE
    } else if w >= 1.0 {
        0.0
    } else {
        f64::from(w).acos() as f32
    };
    let sine = f64::from(angle).sin() as f32;
    let magnitude = sine.abs();
    let coefficient = if magnitude >= SINE_CUTOFF {
        angle / sine
    } else {
        1.0
    };
    [
        0.0,
        value[1] * coefficient,
        value[2] * coefficient,
        coefficient * value[3],
    ]
}

// IDA: 0072F9F0 NiQuaternion::Exp. angle = sqrt(y*y + x*x + z*z), x87 cos / sin, |sin| >= 0.001f selects sin/angle;
// z, y, x products stored.
// Completion: 95% of the verified branches and f32 stores; inline fsin / fcos bits may differ
// TODO: [ ] Compare x87 fsin / fcos and CRT sqrt against the host library.
fn quaternion_exp(value: &Quaternion) -> Quaternion {
    let squared = value[2] * value[2] + value[1] * value[1] + value[3] * value[3];
    let angle = squared.sqrt();
    let cosine = f64::from(angle).cos() as f32;
    let sine = f64::from(angle).sin() as f32;
    let magnitude = sine.abs();
    let coefficient = if magnitude >= SINE_CUTOFF {
        sine / angle
    } else {
        1.0
    };
    let z = value[3] * coefficient;
    let y = coefficient * value[2];
    let x = coefficient * value[1];
    [cosine, x, y, z]
}

// IDA: 00714E70 NiQuaternion::Intermediate(q0, q1, q2): inline conjugate of q1, Log(q1* q2) first, Log(q1* q0)
// second, x / y / z / w sums, -0.25 scale, Exp, then q1 * result.
// Completion: 100% of the verified operand order
fn intermediate(previous: &Quaternion, current: &Quaternion, next: &Quaternion) -> Quaternion {
    let inverse = conjugate(current);
    let log_next = quaternion_log(&multiply(&inverse, next));
    let log_previous = quaternion_log(&multiply(&inverse, previous));
    let x = log_next[1] + log_previous[1];
    let y = log_next[2] + log_previous[2];
    let z = log_next[3] + log_previous[3];
    let w = log_next[0] + log_previous[0];
    multiply(
        current,
        &quaternion_exp(&scale(INTERMEDIATE_SCALE, &[w, x, y, z])),
    )
}

/// IDA: the raw rotation key type. 006BD550 / 006BFB20 / 006BDB00 / 006C13B0 / 006BEC50 / 006C2D20 register the six
/// types into the B3D0B8 load, B3D440 fill and B3D3F4 stride tables. The serialized type is a u32.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RotationKeyType(pub u32);

impl RotationKeyType {
    pub const NO_INTERPOLATION: Self = Self(0);
    pub const LINEAR: Self = Self(1);
    pub const BEZIER: Self = Self(2);
    pub const TCB: Self = Self(3);
    pub const EULER: Self = Self(4);
    pub const STEP: Self = Self(5);
}

/// Host values read from the stream; this is not the original 0x14 / 0x24 / 0x40 byte key ABI.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RotationKey {
    /// IDA: 006C0AD0 leaves +0x00 time uninitialized; the stream always fills it.
    pub time: f32,
    /// IDA: 006C0AD0 stores (0, 0, 0, 0) through 00714C40 before the read.
    pub quaternion: [f32; 4],
    /// IDA: 006BD6B0 Bezier +0x14 intermediate. Original memory is uninitialized when the key count is below two; the
    /// host keeps zero.
    pub bezier_intermediate: [f32; 4],
    /// IDA: 006C11C0 TCB +0x14 tension, +0x18 continuity, +0x1C bias.
    pub tension: f32,
    pub continuity: f32,
    pub bias: f32,
    /// IDA: 006C0C40 TCB derived quaternions at +0x20 and +0x30.
    pub tcb_a: [f32; 4],
    pub tcb_b: [f32; 4],
}

/// IDA: 006BE430 Euler key cold state: per-axis count +0x14, type +0x20, stride +0x2C, key array +0x30 and cached
/// index +0x3C, all zero. 006BEA00 then loads each axis through the shared float key tables.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EulerRotationKey {
    pub axis_counts: [u32; 3],
    pub axis_types: [u32; 3],
    pub axis_strides: [u8; 3],
    pub axes: [FloatKeyData; 3],
    /// IDA: 006BE4D0 passes +0x3C / +0x40 / +0x44 as each axis's last-index cache; it lives in the shared key data,
    /// not in the interpolator.
    pub axis_last_indices: [u32; 3],
}

/// IDA: 006E1C00 NiTransformData factory: 0x2C bytes, NiObject constructor, vtable A7B474, then u16 counts +08 / +0A
/// / +0C, u32 types +10 / +14 / +18, u8 strides +1C / +1D / +1E and key arrays +20 / +24 / +28 all zero. Rotation keys
/// are for types 1 / 2 / 3 / 5 and Euler keys for type 4; the host keeps every loaded key even when the published
/// u16 count is clamped.
/// Completion: 100% of the cold state and the 006E2180 published fields; host vectors own keys.
/// TODO: [ ] Bind original FormHeap key arrays, count cookies and destructors.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TransformKeyData {
    pub rotation_count_08: u16,
    pub position_count_0a: u16,
    pub scale_count_0c: u16,
    pub rotation_type_10: u32,
    pub position_type_14: u32,
    pub scale_type_18: u32,
    pub rotation_stride_1c: u8,
    pub position_stride_1d: u8,
    pub scale_stride_1e: u8,
    pub rotation_keys: Vec<RotationKey>,
    pub euler_keys: Vec<EulerRotationKey>,
    pub position: PositionKeyData,
    pub scale: FloatKeyData,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransformRotationBlock {
    pub count: u32,
    pub key_type: RotationKeyType,
    pub keys: Vec<RotationKey>,
    pub euler_keys: Vec<EulerRotationKey>,
}

/// IDA: 006E2180 reads rotation, then position, then scale blocks. A block's replace helper (006E1E90 / 006E1F00 /
/// 006E1F60) runs only for a nonzero count, so absent blocks leave prior owner state unchanged.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransformKeyPayload {
    pub rotation: Option<TransformRotationBlock>,
    pub position: Option<PositionKeyData>,
    pub position_count: u32,
    pub scale: Option<FloatKeyData>,
    pub scale_count: u32,
    pub bytes_consumed: usize,
}

// IDA: 006BD310 shared rotation fill. For count != 1, each next quaternion is negated when dot(current, next) < 0
// (using the already-updated current); then every w is clamped, with NaN taking the -1 branch.
// Completion: 100% of the verified loop order, comparisons and store points
#[allow(clippy::neg_cmp_op_on_partial_ord)] // IDA: 006BD310 NaN takes the -1 branch.
fn fill_rotation_continuity(keys: &mut [RotationKey]) {
    if keys.len() != 1 {
        for index in 0..keys.len().saturating_sub(1) {
            let current = keys[index].quaternion;
            let next = &mut keys[index + 1].quaternion;
            let dot = next[1] * current[1]
                + current[0] * next[0]
                + next[2] * current[2]
                + next[3] * current[3];
            if dot < 0.0 {
                *next = negate(next);
            }
        }
    }
    for key in keys {
        let w = &mut key.quaternion[0];
        if !(*w >= -1.0) {
            *w = -1.0;
        } else if *w > 1.0 {
            *w = 1.0;
        }
    }
}

// IDA: 006BD6B0 Bezier fill: no effect below two keys; otherwise 006BD310, then 00714E70 for (k0, k0, k1), each
// interior (k-1, k, k+1), and (kn-2, kn-1, kn-1).
// Completion: 100% of the verified call order and neighbour choice
fn fill_rotation_bezier(keys: &mut [RotationKey]) {
    let count = keys.len();
    if count < 2 {
        return;
    }
    fill_rotation_continuity(keys);
    keys[0].bezier_intermediate = intermediate(
        &keys[0].quaternion,
        &keys[0].quaternion,
        &keys[1].quaternion,
    );
    for index in 1..count - 1 {
        keys[index].bezier_intermediate = intermediate(
            &keys[index - 1].quaternion,
            &keys[index].quaternion,
            &keys[index + 1].quaternion,
        );
    }
    keys[count - 1].bezier_intermediate = intermediate(
        &keys[count - 2].quaternion,
        &keys[count - 1].quaternion,
        &keys[count - 1].quaternion,
    );
}

// IDA: 006C0C40 thiscall TCB tangent quaternions for (key, previous, next). `key` is a snapshot and the result is
// (tcb_a, tcb_b): every read precedes both writes.
// Completion: 100% of the verified Log, scale, add, subtract and Exp order and the f32 st / stp points; the incoming
// and outgoing tension products stay in st(0) past their `fst qword` at 006C0D0F / 006C0DEB, but at 24-bit they are
// already rounded to float there.
fn derive_tcb(
    key: &RotationKey,
    previous: &RotationKey,
    next: &RotationKey,
) -> (Quaternion, Quaternion) {
    let log_previous = quaternion_log(&multiply(&conjugate(&previous.quaternion), &key.quaternion));
    let log_next = quaternion_log(&multiply(&conjugate(&key.quaternion), &next.quaternion));
    let inverse_span = 1.0 / (next.time - previous.time);
    let one_minus_tension = 1.0 - key.tension;
    let one_minus_continuity = 1.0 - key.continuity;
    let one_plus_continuity = key.continuity + 1.0;
    let one_minus_bias = 1.0 - key.bias;
    let one_plus_bias = 1.0 + key.bias;

    let incoming_ratio = (key.time - previous.time) * inverse_span;
    let incoming_tension = incoming_ratio * one_minus_tension;
    let s1 = incoming_tension * one_plus_continuity * one_plus_bias;
    let s2 = one_minus_continuity * incoming_tension * one_minus_bias;
    let incoming = add(&scale(s2, &log_next), &scale(s1, &log_previous));
    let tcb_a = multiply(
        &key.quaternion,
        &quaternion_exp(&scale(TCB_HALF, &subtract(&incoming, &log_next))),
    );

    let outgoing_ratio = (next.time - key.time) * inverse_span;
    let outgoing_tension = outgoing_ratio * one_minus_tension;
    let s3 = outgoing_tension * one_minus_continuity * one_plus_bias;
    let s4 = one_plus_continuity * outgoing_tension * one_minus_bias;
    let outgoing = add(&scale(s4, &log_next), &scale(s3, &log_previous));
    let tcb_b = multiply(
        &key.quaternion,
        &quaternion_exp(&scale(TCB_HALF, &subtract(&log_previous, &outgoing))),
    );
    (tcb_a, tcb_b)
}

// IDA: 006C0EC0 TCB fill: no effect below two keys; otherwise 006BD310, then 006C0C40 for (k0; k0, k1), each interior
// (k; k-1, k+1), and (kn-1; kn-2, kn-1).
// Completion: 100% of the verified call order and neighbour choice
fn fill_rotation_tcb(keys: &mut [RotationKey]) {
    let count = keys.len();
    if count < 2 {
        return;
    }
    fill_rotation_continuity(keys);
    let (a, b) = derive_tcb(&keys[0], &keys[0], &keys[1]);
    keys[0].tcb_a = a;
    keys[0].tcb_b = b;
    for index in 1..count - 1 {
        let (a, b) = derive_tcb(&keys[index], &keys[index - 1], &keys[index + 1]);
        keys[index].tcb_a = a;
        keys[index].tcb_b = b;
    }
    let (a, b) = derive_tcb(&keys[count - 1], &keys[count - 2], &keys[count - 1]);
    keys[count - 1].tcb_a = a;
    keys[count - 1].tcb_b = b;
}

// IDA: 006BD550..006C2D20 B3D3F4 rotation strides; B3D3EE position and B3D3E8 float strides from their registration
// functions. Type 4 has no position or float registration.
// Completion: 100% of the verified registered values
const fn rotation_stride(key_type: u32) -> u8 {
    match key_type {
        0 | 1 | 5 => 0x14,
        2 => 0x24,
        3 => 0x40,
        4 => 0x48,
        _ => 0,
    }
}

// IDA: B3D3EE position strides, from the same registration functions.
// Completion: 100%
const fn position_stride_byte(key_type: u32) -> u8 {
    match key_type {
        0 | 1 | 5 => 0x10,
        2 => 0x40,
        3 => 0x4C,
        _ => 0,
    }
}

// IDA: B3D3E8 float strides, from the same registration functions.
// Completion: 100%
const fn float_stride_byte(key_type: u32) -> u8 {
    match key_type {
        0 | 1 | 5 => 0x08,
        2 => 0x10,
        3 => 0x1C,
        _ => 0,
    }
}

// IDA: 006BEA00 Euler key load: a discarded u32 below 0A010068, then for X / Y / Z a u32 count and, when nonzero, a u32
// type and B3D088 float key load with B3D3E8 stride. 006BE840 then runs each axis's float fill.
// Completion: 100% of the verified read order; float fills come from `decode_float_key_payload`
fn read_euler_key(
    bytes: &[u8],
    reader: &mut ByteReader<'_>,
    stream_version: u32,
) -> Result<EulerRotationKey, StreamError> {
    let mut key = EulerRotationKey::default();
    if stream_version < 0x0A01_0068 {
        reader.u32(TRUNCATED)?;
    }
    for axis in 0..3 {
        let decoded = decode_float_key_payload(&bytes[reader.position()..])?;
        reader.take(decoded.bytes_consumed, TRUNCATED)?;
        let count = decoded.data.keys().len();
        key.axis_counts[axis] = count as u32;
        if count != 0 {
            let key_type = u32::from(decoded.data.key_type().0);
            key.axis_types[axis] = key_type;
            key.axis_strides[axis] = float_stride_byte(key_type);
        }
        key.axes[axis] = decoded.data;
    }
    Ok(key)
}

// IDA: 006E2180 rotation block: u32 type, B3D0B8 load, B3D440 fill. Type 0 loads through ReturnZero_SharedVirtual, so
// 006BD310 then dereferences a null key array; unregistered types call a null table slot.
// Completion: 100% of supported types 1-5; original faults are reported as host errors
fn read_rotation_block(
    bytes: &[u8],
    reader: &mut ByteReader<'_>,
    count: u32,
    stream_version: u32,
) -> Result<TransformRotationBlock, StreamError> {
    let mut block = TransformRotationBlock {
        count,
        key_type: RotationKeyType(reader.u32(TRUNCATED)?),
        ..TransformRotationBlock::default()
    };
    match block.key_type {
        RotationKeyType::NO_INTERPOLATION => {
            return Err(StreamError(
                "NiTransformData type-0 rotation keys fault on a null array in 006BD310",
            ));
        }
        RotationKeyType::LINEAR
        | RotationKeyType::BEZIER
        | RotationKeyType::STEP
        | RotationKeyType::TCB => {
            let serialized: usize = if block.key_type == RotationKeyType::TCB {
                32
            } else {
                20
            };
            if count as usize > reader.remaining() / serialized {
                return Err(StreamError("truncated NiTransformData rotation keys"));
            }
            block.keys.reserve(count as usize);
            for _ in 0..count {
                // IDA: 006BD510: NiAnimationKey_LoadTime, then four f32 components.
                let mut key = RotationKey {
                    time: reader.f32(TRUNCATED)?,
                    ..RotationKey::default()
                };
                for component in &mut key.quaternion {
                    *component = reader.f32(TRUNCATED)?;
                }
                if block.key_type == RotationKeyType::TCB {
                    // IDA: 006C11C0: tension +0x14, continuity +0x18, bias +0x1C.
                    key.tension = reader.f32(TRUNCATED)?;
                    key.continuity = reader.f32(TRUNCATED)?;
                    key.bias = reader.f32(TRUNCATED)?;
                }
                block.keys.push(key);
            }
            if block.key_type == RotationKeyType::BEZIER {
                fill_rotation_bezier(&mut block.keys);
            } else if block.key_type == RotationKeyType::TCB {
                fill_rotation_tcb(&mut block.keys);
            } else {
                // IDA: 006BF8E0 thunk to 006BD310.
                fill_rotation_continuity(&mut block.keys);
            }
        }
        RotationKeyType::EULER => {
            block
                .euler_keys
                .reserve((count as usize).min(reader.remaining() / 12));
            for _ in 0..count {
                block
                    .euler_keys
                    .push(read_euler_key(bytes, reader, stream_version)?);
            }
        }
        _ => {
            return Err(StreamError(
                "NiTransformData rotation key type is unregistered",
            ));
        }
    }
    Ok(block)
}

// IDA: 006E2180 clamps a published count above 0xFFFF after load and fill
// Completion: 100% of the stored u16 value; the host keeps every loaded key
const fn clamp_published_count(count: u32) -> u16 {
    if count > 0xFFFF { 0xFFFF } else { count as u16 }
}

// IDA: 006E2180 after the modern empty NiObject prefix: per block u32 count, then u32 type, table load, table fill
// Completion: 100% of the supported read order and the rotation fills 006BD310 / 006BD6B0 / 006C0EC0 / 006BE840 at f32
// store points
// TODO: [ ] Match CRT acos / sin / sqrt bit results.
pub fn decode_transform_key_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<TransformKeyPayload, StreamError> {
    if stream_version < 0x0A01_0072 {
        return Err(StreamError("unsupported NiTransformData stream version"));
    }
    let mut result = TransformKeyPayload::default();
    let mut reader = ByteReader::new(bytes);

    let rotation_count = reader.u32(TRUNCATED)?;
    if rotation_count != 0 {
        result.rotation = Some(read_rotation_block(
            bytes,
            &mut reader,
            rotation_count,
            stream_version,
        )?);
    }

    // IDA: 006E2180's position block uses the B3D0A0 / B3D428 tables shared with NiPosData 006D9F90, so its count, type
    // and key reader is reused verbatim.
    let position_block = decode_position_key_payload(&bytes[reader.position()..], stream_version)?;
    reader.take(position_block.bytes_consumed, TRUNCATED)?;
    if !position_block.data.keys().is_empty() {
        result.position_count = position_block.data.keys().len() as u32;
        result.position = Some(position_block.data);
    }

    // IDA: 006E2180's scale block uses the B3D088 / B3D410 tables shared with NiFloatData 006E3690.
    let scale_block = decode_float_key_payload(&bytes[reader.position()..])?;
    reader.take(scale_block.bytes_consumed, TRUNCATED)?;
    if !scale_block.data.keys().is_empty() {
        result.scale_count = scale_block.data.keys().len() as u32;
        result.scale = Some(scale_block.data);
    }

    result.bytes_consumed = reader.position();
    Ok(result)
}

// IDA: 006E2180 clamps each stored count to 0xFFFF after load and fill, then 006E1E90 / 006E1F00 / 006E1F60 replace
// count, keys, type and stride
// Completion: 100% of the host transaction: decode fully, then publish nonzero blocks in order
// TODO: [ ] Compare original partial publication on stream or allocation failure.
pub fn load_transform_key_payload(
    target: &mut TransformKeyData,
    bytes: &[u8],
    stream_version: u32,
) -> Result<usize, StreamError> {
    let decoded = decode_transform_key_payload(bytes, stream_version)?;
    if let Some(block) = decoded.rotation {
        // IDA: 006E1E90: free old (and Euler nested) arrays, then publish.
        let raw_type = block.key_type.0;
        target.rotation_count_08 = clamp_published_count(block.count);
        target.rotation_type_10 = raw_type;
        target.rotation_stride_1c = rotation_stride(raw_type);
        target.rotation_keys = block.keys;
        target.euler_keys = block.euler_keys;
    }
    if let Some(position) = decoded.position {
        // IDA: 006E1F00 publishes count +0A, keys +24, type +14, stride +1D.
        let raw_type = position.key_type().0;
        target.position_count_0a = clamp_published_count(decoded.position_count);
        target.position_type_14 = raw_type;
        target.position_stride_1d = position_stride_byte(raw_type);
        target.position = position;
    }
    if let Some(scale) = decoded.scale {
        // IDA: 006E1F60 publishes count +0C, keys +28, type +18, stride +1E.
        let raw_type = u32::from(scale.key_type().0);
        target.scale_count_0c = clamp_published_count(decoded.scale_count);
        target.scale_type_18 = raw_type;
        target.scale_stride_1e = float_stride_byte(raw_type);
        target.scale = scale;
    }
    Ok(decoded.bytes_consumed)
}

/// IDA: 006CB990 NiQuatTransform: translate +00, rotate +0C (w, x, y, z), scale +1C. Cold: the invalid sentinels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuatTransform {
    pub translate: [f32; 3],
    pub rotate: [f32; 4],
    pub scale: f32,
}

impl Default for QuatTransform {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            translate: [INVALID_TRANSFORM_COMPONENT; 3],
            rotate: [INVALID_TRANSFORM_COMPONENT; 4],
            scale: INVALID_TRANSFORM_COMPONENT,
        }
    }
}

// IDA: 006CBC10 true only when scale, rotate.x and translate.x all equal -FLT_MAX
// Completion: 100% of the three sentinel comparisons in original order
#[must_use]
pub fn quat_transform_invalid(value: &QuatTransform) -> bool {
    value.scale == INVALID_TRANSFORM_COMPONENT
        && value.rotate[1] == INVALID_TRANSFORM_COMPONENT
        && value.translate[0] == INVALID_TRANSFORM_COMPONENT
}

// IDA: 00471390 SetTranslate: all three components non-NaN and finite. 00471430 SetRotate: x, y, z, then w checked.
// 00471560 SetScale.
// Completion: 100% of the verified acceptance predicates; rejected values keep the cache
fn all_finite(values: &[f32]) -> bool {
    values
        .iter()
        .all(|value| !(value.is_nan() || !value.is_finite()))
}

// IDA: 00471390 SetTranslate
// Completion: 100% of the acceptance rule
pub fn set_quat_transform_translate(target: &mut QuatTransform, value: &[f32; 3]) {
    if all_finite(value) {
        target.translate = *value;
    }
}

// IDA: 00471430 SetRotate
// Completion: 100% of the acceptance rule
pub fn set_quat_transform_rotate(target: &mut QuatTransform, value: &[f32; 4]) {
    if all_finite(value) {
        target.rotate = *value;
    }
}

// IDA: 00471560 SetScale
// Completion: 100% of the acceptance rule
pub fn set_quat_transform_scale(target: &mut QuatTransform, value: f32) {
    if !value.is_nan() && value.is_finite() {
        target.scale = value;
    }
}

// IDA: 00715340 NiQuaternion::Normalize: n = f32(((x*x + w*w) + y*y) + z*z), f32(sqrt), f32(1 / s), then 0072F930
// Completion: 95% of the verified order; CRT sqrt bits
#[must_use]
pub fn normalize_quaternion(value: &[f32; 4]) -> [f32; 4] {
    let n =
        ((value[1] * value[1] + value[0] * value[0]) + value[2] * value[2]) + value[3] * value[3];
    let root = n.sqrt();
    let inverse = 1.0 / root;
    scale_quaternion(value, inverse)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QuatTransformPayload {
    pub value: QuatTransform,
    pub bytes_consumed: usize,
}

// IDA: 006CB990 after NiObject's empty modern prefix: Vec3, four floats, f32 scale, then three validity bytes only
// below 0A01006E; a zero validity byte stores -FLT_MAX into translate.x, rotate.x or scale
// Completion: 100% of the supported read order
// TODO: [ ] Bind original short-read callback failure state.
pub fn decode_quat_transform(
    bytes: &[u8],
    stream_version: u32,
) -> Result<QuatTransformPayload, StreamError> {
    let mut reader = ByteReader::new(bytes);
    let mut result = QuatTransformPayload::default();
    for component in &mut result.value.translate {
        *component = reader.f32(TRUNCATED)?;
    }
    for component in &mut result.value.rotate {
        *component = reader.f32(TRUNCATED)?;
    }
    result.value.scale = reader.f32(TRUNCATED)?;
    if stream_version < 0x0A01_006E {
        const TRUNCATED_FLAG: &str = "truncated NiQuatTransform validity byte";
        if reader.u8(TRUNCATED_FLAG)? == 0 {
            result.value.translate[0] = INVALID_TRANSFORM_COMPONENT;
        }
        if reader.u8(TRUNCATED_FLAG)? == 0 {
            result.value.rotate[1] = INVALID_TRANSFORM_COMPONENT;
        }
        if reader.u8(TRUNCATED_FLAG)? == 0 {
            result.value.scale = INVALID_TRANSFORM_COMPONENT;
        }
    }
    result.bytes_consumed = reader.position();
    Ok(result)
}

// IDA: 0072FBA0 counter-warp: k = f32(f32(f32(1 - cos*A80548)^2) * A80540), result f32(t * (k + (1 + k*t*((t + t) - 3))))
// Completion: 100% of the verified f32 store points and x87 operand order
fn counter_warp(t: f32, cosine: f32) -> f32 {
    // The qword constants hold floats widened, so their f32 values are exact.
    const ATTENUATION: f32 = f64::from_bits(0x3FEA_545A_2000_0000) as f32;
    const SLOPE: f32 = f64::from_bits(0x3FE2_BC5A_2000_0000) as f32;
    let factor = 1.0 - cosine * ATTENUATION;
    let squared = factor * factor;
    let k = squared * SLOPE;
    let cubic = k * t * ((t + t) - 3.0);
    t * (k + (1.0 + cubic))
}

const NORMALIZE_C: f64 = f64::from_bits(0x3FEE_B0AB_2000_0000);

// IDA: 00A0A220 / 00A0A240 static initializers: s = f32(sqrt(A80528)); 00B3FF38 Q = f32(A80550 / s);
// 00B3FF34 P = f32((-0.5 / (s * A80528)) * A80550). Returns (P, Q), computed per call in place of the statics.
// Precision: 53-bit. These are CRT static initializers, which run on the main thread before CreateDevice (027F), so the
// `f64` chain with one store is right here, unlike everything else in this file.
// Completion: 100% of the verified expression order; CRT sqrt is correctly rounded
fn normalize_constants() -> (f32, f32) {
    let c = NORMALIZE_C;
    let scale_d = f64::from_bits(0x3FF0_0146_2000_0000);
    let root = x87::stored_float(c.sqrt());
    (
        x87::stored_float(-0.5 / (f64::from(root) * c) * scale_d),
        x87::stored_float(scale_d / f64::from(root)),
    )
}

// IDA: 0072FAC0 approximate normalize: n = f32(((y*y + x*x) + z*z) + w*w), r = f32((n - C)*P + Q), one refinement when
// n <= A80538, a second when n <= f32 3F26F151, then x, y, z, w each multiplied by r
// Completion: 100% of the verified thresholds, refinement order and store points
pub(crate) fn normalize_approximate(value: &Quaternion) -> Quaternion {
    let c = NORMALIZE_C as f32;
    let first_limit = f64::from_bits(0x3FED_496A_A000_0000) as f32;
    let second_limit = f32::from_bits(0x3F26_F151);
    let (p, q) = normalize_constants();
    let n =
        ((value[2] * value[2] + value[1] * value[1]) + value[3] * value[3]) + value[0] * value[0];
    let mut r = (n - c) * p + q;
    if n <= first_limit {
        let first = r * r * n;
        r *= (first - c) * p + q;
        if n <= second_limit {
            let second = r * r * n;
            r *= (second - c) * p + q;
        }
    }
    let x = value[1] * r;
    let y = r * value[2];
    let z = value[3] * r;
    let w = r * value[0];
    [w, x, y, z]
}

// IDA: 0072FC00 NiQuaternion::Slerp: f32 dot in w, x, y, z order; counter-warp t when t <= 0.5, otherwise
// 1 - warp(f32(1 - t)) (NaN takes this branch); z, y, x, w lerp stores; then 0072FAC0. No sign flip: 006BD310 already
// arranged shortest-path keys.
// Completion: 100% of the verified operand order
fn slerp(t: f32, from: &Quaternion, to: &Quaternion) -> Quaternion {
    let cosine = ((from[0] * to[0] + from[1] * to[1]) + from[2] * to[2]) + from[3] * to[3];
    let warped = if t <= 0.5 {
        counter_warp(t, cosine)
    } else {
        let mirrored = 1.0 - t;
        1.0 - counter_warp(mirrored, cosine)
    };
    let lerp = |a: f32, b: f32| a + (b - a) * warped;
    let z = lerp(from[3], to[3]);
    let y = lerp(from[2], to[2]);
    let x = lerp(from[1], to[1]);
    let w = lerp(from[0], to[0]);
    normalize_approximate(&[w, x, y, z])
}

// IDA: 00714F80 NiQuaternion::Squad: Slerp(t, a, b), then Slerp(t, p, q), then Slerp(f32((t + t) * (1 - t)), pq, ab)
// Completion: 100% of the verified call order
fn squad(t: f32, p: &Quaternion, a: &Quaternion, b: &Quaternion, q: &Quaternion) -> Quaternion {
    let ab = slerp(t, a, b);
    let pq = slerp(t, p, q);
    let blend = (t + t) * (1.0 - t);
    slerp(blend, &pq, &ab)
}

// IDA: 006BE4D0 Euler to quaternion: f32 half angles, x87 fcos / fsin stored to f32, then the w / x / y / z products in
// the verified order
// Completion: 95% of the verified expression order; inline fsin / fcos bits may differ
// TODO: [ ] Compare x87 fsin / fcos against the host library.
fn euler_quaternion(angles: &[f32; 3]) -> Quaternion {
    let mut cosines = [0.0; 3];
    let mut sines = [0.0; 3];
    for axis in 0..3 {
        let half = angles[axis] * 0.5;
        cosines[axis] = f64::from(half).cos() as f32;
        sines[axis] = f64::from(half).sin() as f32;
    }
    let (cx, sx) = (cosines[0], sines[0]);
    let (cy, sy) = (cosines[1], sines[1]);
    let (cz, sz) = (cosines[2], sines[2]);
    let sz_sy = sz * sy;
    let cz_cy = cz * cy;
    let w = cx * cz_cy + sx * sz_sy;
    let x = cz_cy * sx - sz_sy * cx;
    let sz_cy = sz * cy;
    let sy_cz = sy * cz;
    let y = cx * sy_cz + sx * sz_cy;
    let z = sz_cy * cx - sx * sy_cz;
    [w, x, y, z]
}

struct SequenceSegment {
    left: usize,
    ratio: f32,
}

// IDA: 006BB270 / 006BBBA0 / 006BCF70 shared search after their single-key and -FLT_MAX early returns: restart at 0
// when key[last].time > time, scan for the first later key with time >= t, ratio = f32((t - left) / (right - left)).
// Time before key 0 therefore extrapolates.
// Completion: 100% of the verified search semantics. Host stop: the original reads key[count] when no key qualifies,
// and reads a stale cached index unchecked.
fn search_sequence(
    time: f32,
    count: usize,
    last_index: u32,
    time_at: impl Fn(usize) -> f32,
) -> Option<SequenceSegment> {
    let mut last = last_index as usize;
    if last >= count {
        return None;
    }
    let mut left_time = time_at(last);
    if left_time > time {
        last = 0;
        left_time = time_at(0);
    }
    for next in last + 1..count {
        let next_time = time_at(next);
        if next_time >= time {
            return Some(SequenceSegment {
                left: last,
                ratio: ((time - left_time) / (next_time - left_time)),
            });
        }
        left_time = next_time;
        last = next;
    }
    None
}

// IDA: 006BB270 NiFloatKey_InterpolateSequence over `count` keys with a u32 last-index cache (32-bit store at 006BB43C).
// A single key or a time of -FLT_MAX returns key 0; otherwise the shared exact search, which extrapolates before key 0.
// Completion: 100% of the verified search and cache-store order. Host stop: None where the original would read
// key[count] or a stale index.
pub fn sample_float_key_sequence(
    data: &FloatKeyData,
    published_count: usize,
    time: f32,
    last_index: &mut u32,
) -> Option<f32> {
    let keys: &[FloatKey] = data.keys();
    let count = published_count.min(keys.len());
    if count == 0 {
        return None;
    }
    if count == 1 || time == INVALID_TRANSFORM_COMPONENT {
        return Some(keys[0].value);
    }
    let segment = search_sequence(time, count, *last_index, |index| keys[index].time)?;
    let value = data.interpolate(segment.left, segment.ratio);
    if value.is_some() {
        *last_index = segment.left as u32;
    }
    value
}

// IDA: 006BBBA0 position sequence through the B3D010 slots
// Completion: 100% of the early returns, search and cache store after interpolation
fn sample_position_sequence(
    data: &PositionKeyData,
    published_count: usize,
    time: f32,
    last_index: &mut u32,
) -> Option<[f32; 3]> {
    let keys = data.keys();
    let count = published_count.min(keys.len());
    if count == 0 {
        return None;
    }
    if count == 1 || time == INVALID_TRANSFORM_COMPONENT {
        return Some(keys[0].position);
    }
    let segment = search_sequence(time, count, *last_index, |index| keys[index].time)?;
    let value = data.interpolate(segment.left, segment.ratio);
    if value.is_some() {
        *last_index = segment.left as u32;
    }
    value
}

// IDA: 006BE4D0 samples X / Y / Z with each axis's own count, type, stride and +0x3C cache (empty axis -> 0.0), then
// converts to a quaternion
// Completion: 100% of the verified axis order; failure in any axis stops the host path
fn sample_euler(key: &mut EulerRotationKey, time: f32) -> Option<Quaternion> {
    let mut angles = [0.0; 3];
    for (axis, angle) in angles.iter_mut().enumerate() {
        if key.axis_counts[axis] == 0 {
            continue;
        }
        *angle = sample_float_key_sequence(
            &key.axes[axis],
            key.axis_counts[axis] as usize,
            time,
            &mut key.axis_last_indices[axis],
        )?;
    }
    Some(euler_quaternion(&angles))
}

// IDA: 006BCF70 rotation sequence: Euler (type 4) always calls B3D038 with the first key and never touches the cache;
// otherwise single-key / -FLT_MAX early return, search, then B3D028 per type: 6BD2D0 identity, 6BF8F0 slerp, 6BD660
// Bezier squad (+0x14), 6C0BF0 TCB squad (+0x20 / +0x30), 6C1FC0 step (ratio >= 1 picks right)
// Completion: 100% of the verified dispatch
fn sample_rotation_sequence(
    data: &mut TransformKeyData,
    time: f32,
    last_index: &mut u32,
) -> Option<Quaternion> {
    if data.rotation_type_10 == RotationKeyType::EULER.0 {
        return sample_euler(data.euler_keys.first_mut()?, time);
    }
    let keys = &data.rotation_keys;
    let count = usize::from(data.rotation_count_08).min(keys.len());
    if count == 0 {
        return None;
    }
    if count == 1 || time == INVALID_TRANSFORM_COMPONENT {
        return Some(keys[0].quaternion);
    }
    let segment = search_sequence(time, count, *last_index, |index| keys[index].time)?;
    let left = &keys[segment.left];
    let right = &keys[segment.left + 1];
    let ratio = segment.ratio;
    let result = match RotationKeyType(data.rotation_type_10) {
        RotationKeyType::NO_INTERPOLATION => [1.0, 0.0, 0.0, 0.0],
        RotationKeyType::LINEAR => slerp(ratio, &left.quaternion, &right.quaternion),
        RotationKeyType::BEZIER => squad(
            ratio,
            &left.quaternion,
            &left.bezier_intermediate,
            &right.bezier_intermediate,
            &right.quaternion,
        ),
        RotationKeyType::TCB => squad(
            ratio,
            &left.quaternion,
            &left.tcb_a,
            &right.tcb_b,
            &right.quaternion,
        ),
        RotationKeyType::STEP => {
            if ratio >= 1.0 {
                right.quaternion
            } else {
                left.quaternion
            }
        }
        // An unregistered table slot.
        _ => return None,
    };
    *last_index = segment.left as u32;
    Some(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformUpdateStatus {
    Valid,
    Invalid,
    /// Host stop: the original sampler would read one key past an array (time beyond the last key, NaN time, or a
    /// stale cached index).
    UnsupportedOutOfRange,
}

/// IDA: 006D66E0 NiTransformInterpolator factory: 0x38 bytes, 006EC220 base, vtable A7A83C, cold transform +0C..+28,
/// data +2C null, u16 last indices +30 / +32 / +34 zero.
/// Completion: 100% of the cold state, 006D67B0 load and 006D60B0 Update.
/// TODO: [ ] Port 006D62D0 Collapse and the other A7A83C virtuals as reached.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct TransformInterpolator {
    pub last_time_08: f32,
    pub value_0c: QuatTransform,
    pub data_2c: Option<Entity>,
    pub last_position_index: u16,
    pub last_rotation_index: u16,
    pub last_scale_index: u16,
}

impl Default for TransformInterpolator {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            last_time_08: INVALID_TRANSFORM_COMPONENT,
            value_0c: QuatTransform::default(),
            data_2c: None,
            last_position_index: 0,
            last_rotation_index: 0,
            last_scale_index: 0,
        }
    }
}

impl TransformInterpolator {
    // IDA: 006D67B0 stores the 006CB990 transform, then swaps the +0x2C data reference (old released after the new
    // pointer is read)
    // Completion: 100% of the host order
    // TODO: [ ] Bind intrusive refcount timing against the original.
    pub fn load_transform_and_data(&mut self, value: &QuatTransform, data: Option<Entity>) {
        self.value_0c = *value;
        self.data_2c = data;
    }

    // IDA: 006D60B0 vtable +0x4C transform Update. `data` is what `data_2c` links to; it is mutable because the Euler
    // caches live in the shared key data.
    // Completion: 95% of the verified branches, sequence search and per-type math; x87 and CRT trigonometry bits remain
    // host approximations
    // TODO: [ ] Compare against captured original x87 results on real keys.
    pub fn update(
        &mut self,
        data: Option<&mut TransformKeyData>,
        time: f32,
        output: &mut QuatTransform,
    ) -> TransformUpdateStatus {
        if time == self.last_time_08 {
            *output = self.value_0c;
            return if quat_transform_invalid(&self.value_0c) {
                TransformUpdateStatus::Invalid
            } else {
                TransformUpdateStatus::Valid
            };
        }
        if let Some(data) = data {
            if data.position_count_0a != 0 {
                let mut index = u32::from(self.last_position_index);
                let Some(sampled) = sample_position_sequence(
                    &data.position,
                    usize::from(data.position_count_0a),
                    time,
                    &mut index,
                ) else {
                    return TransformUpdateStatus::UnsupportedOutOfRange;
                };
                if all_finite(&sampled) {
                    self.value_0c.translate = sampled;
                }
                self.last_position_index = index as u16;
            }
            if data.rotation_count_08 != 0 {
                let mut index = u32::from(self.last_rotation_index);
                let Some(sampled) = sample_rotation_sequence(data, time, &mut index) else {
                    return TransformUpdateStatus::UnsupportedOutOfRange;
                };
                if all_finite(&sampled) {
                    self.value_0c.rotate = sampled;
                }
                self.last_rotation_index = index as u16;
            }
            if data.scale_count_0c != 0 {
                let mut index = u32::from(self.last_scale_index);
                let Some(sampled) = sample_float_key_sequence(
                    &data.scale,
                    usize::from(data.scale_count_0c),
                    time,
                    &mut index,
                ) else {
                    return TransformUpdateStatus::UnsupportedOutOfRange;
                };
                if all_finite(&[sampled]) {
                    self.value_0c.scale = sampled;
                }
                self.last_scale_index = index as u16;
            }
        }
        *output = self.value_0c;
        if quat_transform_invalid(&self.value_0c) {
            return TransformUpdateStatus::Invalid;
        }
        self.last_time_08 = time;
        TransformUpdateStatus::Valid
    }
}

// IDA: N/A resolves +0x2C for 006D60B0. The link check guarantees the class, so a dangling link is a host defect.
// Completion: 100%
pub fn update_transform_interpolator(
    world: &mut World,
    interpolator: Entity,
    time: f32,
    output: &mut QuatTransform,
) -> TransformUpdateStatus {
    let mut state = *world
        .get::<TransformInterpolator>(interpolator)
        .expect("a NiTransformInterpolator entity");
    let mut data = state.data_2c.map(|data| {
        world
            .get_mut::<TransformKeyData>(data)
            .expect("linked NiTransformData")
    });
    let result = state.update(data.as_deref_mut(), time, output);
    *world
        .get_mut::<TransformInterpolator>(interpolator)
        .expect("a NiTransformInterpolator entity") = state;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 00714CF0 at 24-bit: `rw*lw - rx*lx - ry*ly - rz*lz`, each step rounded. Expected bits from exact per-step rounding; one
    /// rounding over the exact value (the old `f64` form) gives 0x3F7AB031.
    #[test]
    fn quaternion_product_rounds_each_step() {
        let from = |words: [u32; 4]| words.map(f32::from_bits);
        let left = from([0xBF0B_020C, 0x3F34_7AE1, 0xBEE1_47AE, 0xBF17_8D50]);
        let right = from([0xBF5C_AC08, 0x3DF3_B646, 0x3E6A_7EFA, 0x3F55_C28F]);
        assert_eq!(multiply(&left, &right)[0].to_bits(), 0x3F7A_B032);
    }

    /// 0072FBA0 at 24-bit, every step rounded. Expected bits from exact per-step rounding with the A80548 and A80540 qwords; the
    /// old form, which kept the cubic term and its sums unrounded, gives 0x3EC0A36A.
    #[test]
    fn counter_warp_rounds_each_step() {
        let warped = counter_warp(f32::from_bits(0x3EB1_26E9), f32::from_bits(0x3E2D_0E56));
        assert_eq!(warped.to_bits(), 0x3EC0_A369);
    }

    const MODERN_VERSION: u32 = 0x1400_0004;
    const NEGATIVE_MAX: f32 = -f32::MAX;
    /// The test's 0.70710677F.
    const ROOT_HALF: f32 = f32::from_bits(0x3F35_04F3);
    /// The test's 1.5707964F.
    const QUARTER_TURN: f32 = f32::from_bits(0x3FC9_0FDB);

    fn near(actual: f32, expected: f32, tolerance: f32) -> bool {
        (actual - expected).abs() <= tolerance
    }

    fn u32_le(output: &mut Vec<u8>, value: u32) {
        output.extend(value.to_le_bytes());
    }

    fn f32_le(output: &mut Vec<u8>, value: f32) {
        u32_le(output, value.to_bits());
    }

    fn rotation_key(output: &mut Vec<u8>, time: f32, quaternion: [f32; 4]) {
        f32_le(output, time);
        for component in quaternion {
            f32_le(output, component);
        }
    }

    fn linear_position_block(output: &mut Vec<u8>) {
        u32_le(output, 2);
        u32_le(output, 1);
        f32_le(output, 0.0);
        for value in [0.0, 0.0, 0.0] {
            f32_le(output, value);
        }
        f32_le(output, 1.0);
        for value in [10.0, 20.0, 30.0] {
            f32_le(output, value);
        }
    }

    fn linear_scale_block(output: &mut Vec<u8>) {
        u32_le(output, 2);
        u32_le(output, 1);
        f32_le(output, 0.0);
        f32_le(output, 1.0);
        f32_le(output, 1.0);
        f32_le(output, 3.0);
    }

    /// Ported from `transform_animation_tests`: IDA 006E2180 / 006BD310's publication with the continuity sign flip,
    /// w clamp, strides and zero-count blocks keeping prior state.
    #[test]
    fn transform_data_load() {
        let mut bytes = Vec::new();
        u32_le(&mut bytes, 3);
        u32_le(&mut bytes, 1); // linear rotation
        rotation_key(&mut bytes, 0.0, [1.0, 0.0, 0.0, 0.0]);
        rotation_key(&mut bytes, 1.0, [-1.0, 0.0, 0.0, 0.0]); // dot < 0: negated
        rotation_key(&mut bytes, 2.0, [2.0, 0.0, 0.0, 0.0]); // w clamped to 1
        linear_position_block(&mut bytes);
        linear_scale_block(&mut bytes);
        bytes.push(0xA5); // a trailing byte belongs to the next object

        let mut data = TransformKeyData::default();
        assert_eq!(data.rotation_count_08, 0);
        assert_eq!(data.position_count_0a, 0);
        assert_eq!(data.scale_count_0c, 0);
        assert_eq!(data.rotation_stride_1c, 0);
        assert!(data.rotation_keys.is_empty());
        assert_eq!(
            load_transform_key_payload(&mut data, &bytes, MODERN_VERSION),
            Ok(132)
        );
        assert_eq!(data.rotation_count_08, 3);
        assert_eq!(data.rotation_type_10, 1);
        assert_eq!(data.rotation_stride_1c, 0x14);
        assert_eq!(data.position_count_0a, 2);
        assert_eq!(data.position_type_14, 1);
        assert_eq!(data.position_stride_1d, 0x10);
        assert_eq!(data.scale_count_0c, 2);
        assert_eq!(data.scale_type_18, 1);
        assert_eq!(data.scale_stride_1e, 0x08);
        assert_eq!(data.rotation_keys.len(), 3);
        assert_eq!(data.rotation_keys[1].quaternion[0], 1.0);
        assert!(data.rotation_keys[1].quaternion[1].is_sign_negative());
        assert_eq!(data.rotation_keys[2].quaternion[0], 1.0);
        assert_eq!(data.position.keys().len(), 2);
        assert_eq!(data.position.keys()[1].position[2], 30.0);
        assert_eq!(data.scale.keys().len(), 2);
        assert_eq!(data.scale.keys()[1].value, 3.0);

        let mut empty = Vec::new();
        u32_le(&mut empty, 0);
        u32_le(&mut empty, 0);
        u32_le(&mut empty, 0);
        assert_eq!(
            load_transform_key_payload(&mut data, &empty, MODERN_VERSION),
            Ok(12)
        );
        assert_eq!(data.rotation_count_08, 3);
        assert_eq!(data.position_count_0a, 2);
        assert_eq!(data.scale_count_0c, 2);
        assert_eq!(data.rotation_keys.len(), 3);

        for key_type in [0, 6] {
            let mut bad = Vec::new();
            u32_le(&mut bad, 1);
            u32_le(&mut bad, key_type);
            rotation_key(&mut bad, 0.0, [1.0, 0.0, 0.0, 0.0]);
            assert!(decode_transform_key_payload(&bad, MODERN_VERSION).is_err());
        }
        assert!(decode_transform_key_payload(&bytes[..40], MODERN_VERSION).is_err());
        assert!(decode_transform_key_payload(&bytes, 0x0A01_0071).is_err());
    }

    fn spawn_interpolator(world: &mut World, bytes: &[u8]) -> Entity {
        let mut data = TransformKeyData::default();
        load_transform_key_payload(&mut data, bytes, MODERN_VERSION).unwrap();
        let data = world.spawn(data).id();
        let mut interpolator = TransformInterpolator::default();
        interpolator.load_transform_and_data(&QuatTransform::default(), Some(data));
        world.spawn(interpolator).id()
    }

    /// IDA: 006D60B0 Update over linear translate and scale and slerped rotation.
    #[test]
    fn interpolator_update() {
        let mut bytes = Vec::new();
        u32_le(&mut bytes, 2);
        u32_le(&mut bytes, 1);
        rotation_key(&mut bytes, 0.0, [1.0, 0.0, 0.0, 0.0]);
        rotation_key(&mut bytes, 1.0, [ROOT_HALF, 0.0, 0.0, ROOT_HALF]); // 90 degrees about Z
        linear_position_block(&mut bytes);
        linear_scale_block(&mut bytes);

        let cold = TransformInterpolator::default();
        assert_eq!(cold.last_time_08, NEGATIVE_MAX);
        assert!(cold.data_2c.is_none());
        assert!(quat_transform_invalid(&cold.value_0c));

        let mut world = World::new();
        let interpolator = spawn_interpolator(&mut world, &bytes);
        let mut output = QuatTransform::default();
        assert_eq!(
            update_transform_interpolator(&mut world, interpolator, 0.5, &mut output),
            TransformUpdateStatus::Valid
        );
        assert_eq!(output.translate, [5.0, 10.0, 15.0]);
        assert_eq!(output.scale, 2.0);
        assert!(near(output.rotate[0], 0.923_879_5, 1e-3));
        assert_eq!(output.rotate[1], 0.0);
        assert_eq!(output.rotate[2], 0.0);
        assert!(near(output.rotate[3], 0.382_683_4, 1e-3));
        let state = *world.get::<TransformInterpolator>(interpolator).unwrap();
        assert_eq!(state.last_time_08, 0.5);
        assert_eq!(state.last_position_index, 0);
        assert_eq!(state.last_rotation_index, 0);
        assert_eq!(state.last_scale_index, 0);

        let mut cached = QuatTransform::default();
        assert_eq!(
            update_transform_interpolator(&mut world, interpolator, 0.5, &mut cached),
            TransformUpdateStatus::Valid
        );
        assert_eq!(cached.translate, output.translate);
        assert_eq!(cached.scale, output.scale);

        assert_eq!(
            update_transform_interpolator(&mut world, interpolator, -1.0, &mut output),
            TransformUpdateStatus::Valid
        );
        assert_eq!(output.translate, [-10.0, -20.0, -30.0]);
        assert_eq!(output.scale, -1.0);
        assert_eq!(
            update_transform_interpolator(&mut world, interpolator, 5.0, &mut output),
            TransformUpdateStatus::UnsupportedOutOfRange
        );

        let empty = world.spawn(TransformInterpolator::default()).id();
        let mut empty_output = QuatTransform::default();
        assert_eq!(
            update_transform_interpolator(&mut world, empty, 0.0, &mut empty_output),
            TransformUpdateStatus::Invalid
        );
        assert!(quat_transform_invalid(&empty_output));

        let fixed = QuatTransform {
            translate: [1.0, 2.0, 3.0],
            rotate: [1.0, 0.0, 0.0, 0.0],
            scale: 4.0,
        };
        let mut constant = TransformInterpolator::default();
        constant.load_transform_and_data(&fixed, None);
        let constant = world.spawn(constant).id();
        let mut constant_output = QuatTransform::default();
        assert_eq!(
            update_transform_interpolator(&mut world, constant, 7.0, &mut constant_output),
            TransformUpdateStatus::Valid
        );
        assert_eq!(constant_output.translate, fixed.translate);
        assert_eq!(constant_output.scale, 4.0);
        assert_eq!(
            world
                .get::<TransformInterpolator>(constant)
                .unwrap()
                .last_time_08,
            7.0
        );
    }

    /// IDA: 006C1FC0 step rotation (ratio >= 1 picks the right key) and 006BE4D0 Euler type 4.
    #[test]
    fn step_and_euler_rotation() {
        let mut step = Vec::new();
        u32_le(&mut step, 2);
        u32_le(&mut step, 5);
        rotation_key(&mut step, 0.0, [1.0, 0.0, 0.0, 0.0]);
        rotation_key(&mut step, 1.0, [0.0, 1.0, 0.0, 0.0]);
        u32_le(&mut step, 0);
        u32_le(&mut step, 0);
        let mut world = World::new();
        let stepped = spawn_interpolator(&mut world, &step);
        let data = world
            .get::<TransformInterpolator>(stepped)
            .unwrap()
            .data_2c
            .unwrap();
        let loaded = world.get::<TransformKeyData>(data).unwrap();
        assert_eq!(loaded.rotation_type_10, 5);
        assert_eq!(loaded.rotation_stride_1c, 0x14);
        let mut output = QuatTransform {
            translate: [0.0; 3],
            ..QuatTransform::default()
        };
        update_transform_interpolator(&mut world, stepped, 0.5, &mut output);
        assert_eq!(output.rotate, [1.0, 0.0, 0.0, 0.0]);
        update_transform_interpolator(&mut world, stepped, 1.0, &mut output);
        assert_eq!(output.rotate, [0.0, 1.0, 0.0, 0.0]);

        let mut euler = Vec::new();
        u32_le(&mut euler, 1);
        u32_le(&mut euler, 4);
        u32_le(&mut euler, 0); // X axis: no keys
        u32_le(&mut euler, 0); // Y axis: no keys
        u32_le(&mut euler, 1); // Z axis: one linear key
        u32_le(&mut euler, 1);
        f32_le(&mut euler, 0.0);
        f32_le(&mut euler, QUARTER_TURN);
        u32_le(&mut euler, 0);
        u32_le(&mut euler, 0);
        let mut euler_data = TransformKeyData::default();
        assert_eq!(
            load_transform_key_payload(&mut euler_data, &euler, MODERN_VERSION),
            Ok(euler.len())
        );
        assert_eq!(euler_data.rotation_type_10, 4);
        assert_eq!(euler_data.rotation_stride_1c, 0x48);
        assert_eq!(euler_data.euler_keys.len(), 1);
        assert_eq!(euler_data.euler_keys[0].axis_counts, [0, 0, 1]);
        let euler_interpolator = spawn_interpolator(&mut world, &euler);
        let mut euler_output = QuatTransform::default();
        update_transform_interpolator(&mut world, euler_interpolator, 3.0, &mut euler_output);
        assert!(near(euler_output.rotate[0], ROOT_HALF, 1e-6));
        assert_eq!(euler_output.rotate[1], 0.0);
        assert_eq!(euler_output.rotate[2], 0.0);
        assert!(near(euler_output.rotate[3], ROOT_HALF, 1e-6));
    }

    /// IDA: 006CBC10, 00471390 / 00471430 / 00471560, 0072F930, 00715340 and the 006CB990 stream read.
    #[test]
    fn quat_transform_helpers() {
        let mut value = QuatTransform::default();
        assert!(quat_transform_invalid(&value));
        value.translate[0] = 0.0;
        assert!(!quat_transform_invalid(&value));

        let mut target = QuatTransform::default();
        set_quat_transform_translate(&mut target, &[1.0, f32::NAN, 2.0]);
        set_quat_transform_rotate(&mut target, &[f32::NAN, 0.0, 0.0, 0.0]);
        set_quat_transform_scale(&mut target, f32::INFINITY);
        assert!(quat_transform_invalid(&target));
        set_quat_transform_translate(&mut target, &[1.0, 2.0, 3.0]);
        set_quat_transform_rotate(&mut target, &[1.0, 0.0, 0.0, 0.0]);
        set_quat_transform_scale(&mut target, 2.0);
        assert_eq!(target.translate[1], 2.0);
        assert_eq!(target.rotate[0], 1.0);
        assert_eq!(target.scale, 2.0);

        assert_eq!(
            scale_quaternion(&[1.0, 2.0, 3.0, 4.0], 2.0),
            [2.0, 4.0, 6.0, 8.0]
        );
        assert_eq!(
            normalize_quaternion(&[0.0, 3.0, 0.0, 4.0]),
            [0.0, 0.6, 0.0, 0.8]
        );

        let mut bytes = Vec::new();
        for component in [1.0, 2.0, 3.0, 1.0, 0.0, 0.0, 0.0, 5.0] {
            f32_le(&mut bytes, component);
        }
        let modern = decode_quat_transform(&bytes, 0x0A01_006E).unwrap();
        assert_eq!(modern.bytes_consumed, 32);
        assert_eq!(modern.value.translate[2], 3.0);
        assert_eq!(modern.value.rotate[0], 1.0);
        assert_eq!(modern.value.scale, 5.0);
        bytes.extend([1, 0, 1]);
        let legacy = decode_quat_transform(&bytes, 0x0A01_006D).unwrap();
        assert_eq!(legacy.bytes_consumed, 35);
        assert_eq!(legacy.value.translate[0], 1.0);
        assert_eq!(legacy.value.rotate[1], NEGATIVE_MAX);
        assert_eq!(legacy.value.scale, 5.0);
    }

    /// IDA: 006BB270 NiFloatKey_InterpolateSequence's early returns, search restart and cache store, and the host stops.
    #[test]
    fn float_sequence() {
        let mut bytes = Vec::new();
        u32_le(&mut bytes, 3);
        u32_le(&mut bytes, 1);
        for value in [0.0, 0.0, 1.0, 10.0, 2.0, 30.0] {
            f32_le(&mut bytes, value);
        }
        let decoded = decode_float_key_payload(&bytes).unwrap();
        let mut last = 0;
        assert_eq!(
            sample_float_key_sequence(&decoded.data, 1, 1.5, &mut last),
            Some(0.0)
        );
        assert_eq!(
            sample_float_key_sequence(&decoded.data, 3, NEGATIVE_MAX, &mut last),
            Some(0.0)
        );
        assert_eq!(
            sample_float_key_sequence(&decoded.data, 3, 1.5, &mut last),
            Some(20.0)
        );
        assert_eq!(last, 1);
        assert_eq!(
            sample_float_key_sequence(&decoded.data, 3, 0.5, &mut last),
            Some(5.0)
        );
        assert_eq!(last, 0);
        assert_eq!(
            sample_float_key_sequence(&decoded.data, 3, 3.0, &mut last),
            None
        );
        last = 5;
        assert_eq!(
            sample_float_key_sequence(&decoded.data, 3, 1.0, &mut last),
            None
        );
    }
}
