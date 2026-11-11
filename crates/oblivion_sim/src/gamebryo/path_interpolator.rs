//! NiPathInterpolator: a position track followed by percent, with constant velocity and the follow, bank and smoothing
//! reference frame.
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;
use oblivion_core::x87::float_fmod;

use super::animation::FloatKeyData;
use super::position_animation::PositionKeyData;
use super::stream_objects::NULL_REFERENCE;
use super::transform_animation::{
    INVALID_TRANSFORM_COMPONENT, QuatTransform, TransformUpdateStatus, sample_float_key_sequence,
    set_quat_transform_rotate, set_quat_transform_translate,
};

const TRUNCATED: &str = "truncated NiPathInterpolator payload";

type Vector3 = [f32; 3];
/// Row-major NiMatrix3, in original memory order.
type Matrix3 = [f32; 9];

/// IDA: 006DBD3D f64 A68FE0 Newton tolerance (f32 1e-4 widened).
const NEWTON_TOLERANCE: f32 = f64::from_bits(0x3F1A_36E2_E000_0000) as f32;
/// IDA: 006DBD70 iteration bound 0x20.
const NEWTON_ITERATIONS: i32 = 32;
/// IDA: 006DAEB0 lazily stores f32(4.0 / 00B3F9A4) to 00B3DD30 under guard bit 00B3DD34; 00A09B90 sets
/// 00B3F9A4 = f32(f32(atan(1)) * 4) = 0x40490FDB. The result is 0x3FA2F983 (0.37 / 0.63 ulp from either midpoint).
const FOUR_OVER_PI: f32 = f32::from_bits(0x3FA2_F983);
/// IDA: 00B246E0 .data five f32 Gauss-Legendre half weights and 00B246F4 nodes mapped to [0, 1], read only by 006DB6F0.
const GAUSS_WEIGHTS: [f32; 5] = [
    f32::from_bits(0x3DF2_9CF6),
    f32::from_bits(0x3E75_0ED1),
    f32::from_bits(0x3E91_A2B4),
    f32::from_bits(0x3E75_0ED1),
    f32::from_bits(0x3DF2_9CF6),
];
const GAUSS_NODES: [f32; 5] = [
    f32::from_bits(0x3D40_24C8),
    f32::from_bits(0x3E6C_4DC0),
    f32::from_bits(0x3F00_0000),
    f32::from_bits(0x3F44_EC90),
    f32::from_bits(0x3F73_FDB3),
];
/// IDA: 00B26DC4 .data identity NiMatrix3 copied first by 006DAEB0.
const IDENTITY: Matrix3 = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
/// IDA: 00B258E8 .data NiPoint3 (0, 0, 1), the 004BF9E0 receiver in 006DB38B.
const UNIT_Z: Vector3 = [0.0, 0.0, 1.0];
/// IDA: 00B27120 .data {1, 2, 0} next-axis table read by 007150F0.
const NEXT_AXIS: [usize; 3] = [1, 2, 0];

// IDA: N/A host guard for slots every bound NiPosData type registers
// Completion: 100%; unreachable for verified segment indices, so a failure is a host defect
fn required(value: Option<Vector3>) -> Vector3 {
    value.expect("NiPosData slot unavailable for a verified path segment")
}

// IDA: 00498FE0 thiscall cross out = this x rhs (also 004BF9E0 and the 006BBEE0 / 006DB3B1 inline copies):
// f32(b.z*a.y - b.y*a.z), f32(a.z*b.x - a.x*b.z), f32(b.y*a.x - b.x*a.y)
// Completion: 100% of the verified operands
fn cross(a: &Vector3, b: &Vector3) -> Vector3 {
    [
        b[2] * a[1] - b[1] * a[2],
        a[2] * b[0] - a[0] * b[2],
        b[1] * a[0] - b[0] * a[1],
    ]
}

// IDA: shared f32((x*x + y*y) + z*z) squared length in 0043F350, 004BF9E0, 006BBEE0 and 006DB660
// Completion: 100% of the verified addition order
fn squared_length(value: &Vector3) -> f32 {
    (value[0] * value[0] + value[1] * value[1]) + value[2] * value[2]
}

// IDA: 004BF9E0 NiPoint3::UnitCross: cross, then the 0043F350 length test and f32(inverse * component) stores
// Completion: 100% of the verified branches
fn unit_cross(a: &Vector3, b: &Vector3) -> Vector3 {
    let mut value = cross(a, b);
    super::transform::normalize_vector(&mut value);
    value
}

// IDA: 0047DA10 cdecl f32 * NiPoint3: per component f32(v * s)
// Completion: 100%
fn scale(factor: f32, value: &Vector3) -> Vector3 {
    [value[0] * factor, value[1] * factor, value[2] * factor]
}

// IDA: 0047D9B0 thiscall out = rhs + this per component
// Completion: 100%
fn add(self_value: &Vector3, rhs: &Vector3) -> Vector3 {
    [
        rhs[0] + self_value[0],
        rhs[1] + self_value[1],
        rhs[2] + self_value[2],
    ]
}

// IDA: 0070FE20 NiMatrix3::MakeRotation(angle, x, y, z): fsincos stored to f32, omc = f32(1 - c), f32 pair products
// (x*y*omc etc.), f32 axis*sin and squares, then the nine entries in the original (transposed) convention
// Completion: 95% of the verified store points; fsincos bits are a host approximation
// TODO: [ ] Compare x87 fsincos against the host library.
fn axis_angle_rotation(angle: f32, axis: &Vector3) -> Matrix3 {
    let cosine = f64::from(angle).cos() as f32;
    let sine = f64::from(angle).sin() as f32;
    let complement = 1.0 - cosine;
    let (x, y, z) = (axis[0], axis[1], axis[2]);
    let xy = x * y * complement;
    let xz = x * z * complement;
    let yz = y * z * complement;
    let xs = x * sine;
    let ys = y * sine;
    let zs = z * sine;
    let xx = x * x;
    let yy = y * y;
    let zz = z * z;
    [
        xx * complement + cosine,
        xy + zs,
        xz - ys,
        xy - zs,
        yy * complement + cosine,
        yz + xs,
        xz + ys,
        yz - xs,
        zz * complement + cosine,
    ]
}

// IDA: 007150F0 NiQuaternion::FromRotation into (w, x, y, z). trace = f32((m00 + m11) + m22); trace > 0 uses
// root = f32(sqrt(f32(trace + 1))); otherwise the largest diagonal i (strict <, NaN keeps 0) with j / k from 00B27120
// Completion: 100% of the verified branches, operands and f32 stores
// TODO: [ ] Compare CRT __CIsqrt bits at runtime.
fn quaternion_from_rotation(m: &Matrix3) -> [f32; 4] {
    let trace = (m[0] + m[4]) + m[8];
    if trace > 0.0 {
        let root = (trace + 1.0).sqrt();
        let w = root * 0.5;
        let factor = 0.5 / root;
        return [
            w,
            (m[7] - m[5]) * factor,
            (m[2] - m[6]) * factor,
            (m[3] - m[1]) * factor,
        ];
    }
    let mut i = 0;
    if m[0] < m[4] {
        i = 1;
    }
    if m[i * 4] < m[8] {
        i = 2;
    }
    let j = NEXT_AXIS[i];
    let k = NEXT_AXIS[j];
    let root = (((m[i * 4] - m[j * 4]) - m[k * 4]) + 1.0).sqrt();
    let mut q = [0.0; 4];
    q[1 + i] = root * 0.5;
    let factor = 0.5 / root;
    q[0] = (m[k * 3 + j] - m[j * 3 + k]) * factor;
    q[1 + j] = (m[i * 3 + j] + m[j * 3 + i]) * factor;
    q[1 + k] = (m[i * 3 + k] + m[k * 3 + i]) * factor;
    q
}

// IDA: 006DB660 segment speed: B3D250[type](t, key[i1], key[i2]) then f32(sqrt(f32 squared length))
// Completion: 100% of the verified order
// TODO: [ ] Compare CRT __CIsqrt bits at runtime.
fn segment_speed(path: &PositionKeyData, left: usize, t: f32) -> f32 {
    let first = required(path.derivative(left, t));
    squared_length(&first).sqrt()
}

// IDA: 006DB6F0 segment arc length over [0, t]: for five nodes u = f32(node*t), sum = f32(speed(u) * weight + sum),
// result f32(sum * t)
// Completion: 100%
fn segment_length(path: &PositionKeyData, left: usize, t: f32) -> f32 {
    let mut sum = 0.0;
    for node in 0..GAUSS_NODES.len() {
        let u = GAUSS_NODES[node] * t;
        let speed = segment_speed(path, left, u);
        sum += speed * GAUSS_WEIGHTS[node];
    }
    sum * t
}

#[derive(Clone, Copy, Default)]
struct PathFrame {
    tangent: Vector3,
    normal: Vector3,
    binormal: Vector3,
    curvature: f32,
}

// IDA: 006BBEE0 NiPosKey::CoordinateFrame: d1 (B3D250) then d2 (B3D668); lenSq / speed / inverse f32; tangent
// f32(d1 * inverse); curvature f32((|d1 x d2| * inverse) * inverse). When 1e-6 < |curvature| the normal is
// f32(f32(d2 * lenSq) - f32(d1 * dot)); otherwise curvature 0 and a normal built from the tangent. Then 0043F350 and
// binormal = tangent x normal.
// Completion: 100% of the verified branches and f32 stores
// TODO: [ ] Compare CRT __CIsqrt bits and x87 intermediates at runtime.
fn coordinate_frame(path: &PositionKeyData, left: usize, t: f32) -> PathFrame {
    let first = required(path.derivative(left, t));
    let second = required(path.second_derivative(left, t));
    let mut frame = PathFrame::default();
    let squared = squared_length(&first);
    let speed = squared.sqrt();
    let inverse = 1.0 / speed;
    for (tangent, component) in frame.tangent.iter_mut().zip(first) {
        *tangent = component * inverse;
    }
    let product = cross(&first, &second);
    let product_length = squared_length(&product).sqrt();
    frame.curvature = product_length * inverse * inverse;
    if super::transform::NORMALIZE_ZERO_THRESHOLD < frame.curvature.abs() {
        let dot = (first[0] * second[0] + first[1] * second[1]) + first[2] * second[2];
        for axis in 0..3 {
            let along = first[axis] * dot;
            let scaled = second[axis] * squared;
            frame.normal[axis] = scaled - along;
        }
    } else {
        frame.curvature = 0.0;
        let tangent = &frame.tangent;
        frame.normal = if tangent[0].abs() > super::transform::NORMALIZE_ZERO_THRESHOLD
            || tangent[1].abs() > super::transform::NORMALIZE_ZERO_THRESHOLD
        {
            [tangent[1], -tangent[0], 0.0]
        } else {
            [0.0, -tangent[2], tangent[1]]
        };
    }
    super::transform::normalize_vector(&mut frame.normal);
    frame.binormal = cross(&frame.tangent, &frame.normal);
    frame
}

/// IDA: 006DBF20 NiPathInterpolator::LoadBinary after the modern empty NiKeyBasedInterpolator prefix: u16 flags +0C,
/// u32 bank direction (stored +38), f32 max bank angle +28, f32 smoothing +2C, u16 follow axis +30, then two 00712A20
/// deferred link IDs (path NiPosData, percent NiFloatData).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathInterpolatorPayload {
    pub flags: u16,
    pub bank_direction: i32,
    pub max_bank_angle: f32,
    pub smoothing: f32,
    pub follow_axis: i16,
    pub path_data_index: u32,
    pub percent_data_index: u32,
    pub bytes_consumed: usize,
}

impl Default for PathInterpolatorPayload {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            flags: 0,
            bank_direction: 0,
            max_bank_angle: 0.0,
            smoothing: 0.0,
            follow_axis: 0,
            path_data_index: NULL_REFERENCE,
            percent_data_index: NULL_REFERENCE,
            bytes_consumed: 0,
        }
    }
}

// IDA: 006DBF20 modern field order (24 bytes); 006EC2B0 -> 006EBA80 -> NiObject has no modern base bytes
// Completion: 100% of the read order and supported widths
// TODO: [ ] Port older NiObject base fields if a legacy stream reaches it; bind the short-read failure state.
pub fn parse_path_interpolator_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<PathInterpolatorPayload, GameError> {
    if stream_version < 0x0A01_0072 {
        return Err(GameError::unclassified(
            0,
            "legacy NiPathInterpolator payload unsupported",
        ));
    }
    let mut reader = ByteReader::new(bytes);
    let flags = reader.u16(TRUNCATED)?;
    let bank_direction = reader.u32(TRUNCATED)? as i32;
    let max_bank_angle = reader.f32(TRUNCATED)?;
    let smoothing = reader.f32(TRUNCATED)?;
    let follow_axis = reader.u16(TRUNCATED)? as i16;
    let path_data_index = reader.u32(TRUNCATED)?;
    let percent_data_index = reader.u32(TRUNCATED)?;
    Ok(PathInterpolatorPayload {
        flags,
        bank_direction,
        max_bank_angle,
        smoothing,
        follow_axis,
        path_data_index,
        percent_data_index,
        bytes_consumed: reader.position(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathLinkStatus {
    Linked,
    /// Host stop: 006DBB10 with no path keys allocates zero bytes, writes partial[0] and loops count-1 = 0xFFFFFFFF
    /// times.
    UnsupportedConstantVelocityWithoutKeys,
}

#[derive(Clone, Copy)]
struct Segment {
    left: usize,
    ratio: f32,
}

/// IDA: 006DC5E0 NiPathInterpolator factory: FormHeap 0x5C bytes, 006DC1D0 constructor with null path and percent
/// data, vtable A7AE34 (+1C load 006DBF20, +20 link 006DC000, +3C PostLoad 0068F970, +4C Update 006DC2C0, +78 true).
/// Cold values come from 00B26DC4 / 006DC1D0: `rotation_3c` from 00B3EBA0, filled by 00A08840 with -FLT_MAX x4, and
/// `position_4c` from 00B24FC8 .data (-FLT_MAX x3).
/// Completion: 100% of the cold state, load, link (constant-velocity table, max curvature) and the Update with the
/// follow, bank and smoothing reference frame.
/// TODO: [ ] Port the remaining A7AE34 virtuals (clone, collapse, save) as reached.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct PathInterpolator {
    /// Never written by this class.
    pub last_time_08: f32,
    pub flags_0c: u16,
    pub last_path_index_10: u32,
    pub last_percent_index_14: u32,
    pub path_18: Option<Entity>,
    pub percent_1c: Option<Entity>,
    /// A FormHeap array; empty is null.
    pub partial_lengths_20: Vec<f32>,
    pub total_length_24: f32,
    pub max_bank_angle_28: f32,
    pub smoothing_2c: f32,
    pub follow_axis_30: i16,
    pub max_curvature_34: f32,
    pub bank_direction_38: i32,
    pub rotation_3c: [f32; 4],
    pub position_4c: [f32; 3],
    pub path_last_time_58: f32,
}

impl Default for PathInterpolator {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            last_time_08: INVALID_TRANSFORM_COMPONENT,
            flags_0c: 0x0003,
            last_path_index_10: 0,
            last_percent_index_14: 0,
            path_18: None,
            percent_1c: None,
            partial_lengths_20: Vec::new(),
            total_length_24: -1.0,
            max_bank_angle_28: 0.0,
            smoothing_2c: 0.0,
            follow_axis_30: 0,
            max_curvature_34: 0.0,
            bank_direction_38: 1,
            rotation_3c: [INVALID_TRANSFORM_COMPONENT; 4],
            position_4c: [INVALID_TRANSFORM_COMPONENT; 3],
            path_last_time_58: INVALID_TRANSFORM_COMPONENT,
        }
    }
}

impl PathInterpolator {
    // IDA: flag bits read by 006DC000 / 006DBDB0 / 006DAEB0 / 006DC2C0. Names follow verified behaviour; Gamebryo
    // spellings (CVDATAUPDATE, CURVETYPEOPEN, ALLOWFLIP, BANK, CONSTVELOCITY, FOLLOW, FLIP) are correlation only.
    pub const CONSTANT_VELOCITY_DIRTY_FLAG: u16 = 0x0001;
    pub const OPEN_CURVE_FLAG: u16 = 0x0002;
    pub const FRENET_FRAME_FLAG: u16 = 0x0004;
    pub const BANK_FLAG: u16 = 0x0008;
    pub const CONSTANT_VELOCITY_FLAG: u16 = 0x0010;
    pub const FOLLOW_FLAG: u16 = 0x0020;
    pub const FLIP_FLAG: u16 = 0x0040;

    // IDA: 006DBF20 stores flags, bank direction, max bank angle, smoothing and follow axis in read order
    // Completion: 100% of the field publication
    pub fn apply_loaded_fields(&mut self, payload: &PathInterpolatorPayload) {
        self.flags_0c = payload.flags;
        self.bank_direction_38 = payload.bank_direction;
        self.max_bank_angle_28 = payload.max_bank_angle;
        self.smoothing_2c = payload.smoothing;
        self.follow_axis_30 = payload.follow_axis;
    }

    // IDA: 006DC000 LinkObject: 006EC2D0 base no-op, then 007124A0 path +18 and percent +1C swaps, flags |= 1, constant
    // velocity recomputes +24 through 006DBB10 and clears bit 0, then a nonempty path stores B3D130[type](keys, count)
    // to +34. 006DC0BF tests the count unsigned against zero before the B3D130 call.
    // Completion: 100% of the verified order; entity links replace intrusive refcounts
    // TODO: [ ] Reproduce original intrusive refcount swap timing.
    pub fn link(
        &mut self,
        path: Option<(Entity, &PositionKeyData)>,
        percent: Option<Entity>,
    ) -> PathLinkStatus {
        self.path_18 = path.map(|(entity, _)| entity);
        self.percent_1c = percent;
        let path = path.map(|(_, data)| data);
        self.flags_0c |= Self::CONSTANT_VELOCITY_DIRTY_FLAG;
        if self.flags_0c & Self::CONSTANT_VELOCITY_FLAG != 0
            && self.flags_0c & Self::CONSTANT_VELOCITY_DIRTY_FLAG != 0
        {
            let Some(total) = self.constant_velocity_total_length(path) else {
                return PathLinkStatus::UnsupportedConstantVelocityWithoutKeys;
            };
            self.total_length_24 = total;
            self.flags_0c &= !Self::CONSTANT_VELOCITY_DIRTY_FLAG;
        }
        if let Some(path) = path.filter(|path| !path.keys().is_empty()) {
            self.max_curvature_34 = path.max_curvature();
        }
        PathLinkStatus::Linked
    }

    // IDA: 006DBB10: keep +24 unless 0.0 > total (0.0 <, ==, or NaN keep it); otherwise free +20, allocate count floats,
    // partial[0] = 0.0 and partial[i+1] = f32(partial[i] + f32(006DB6F0(i, i+1, 1.0))), then total = partial[count-1]
    // Completion: 100% of the verified branches and store points
    // TODO: [ ] Bind FormHeap allocation failure and the zero-count fault.
    #[allow(clippy::neg_cmp_op_on_partial_ord)] // IDA: 006DBB10 a NaN total is kept.
    fn constant_velocity_total_length(&mut self, path: Option<&PositionKeyData>) -> Option<f32> {
        if !(self.total_length_24 < 0.0) {
            return Some(self.total_length_24);
        }
        let path = path.filter(|path| !path.keys().is_empty())?;
        let count = path.keys().len();
        self.partial_lengths_20.clear();
        self.partial_lengths_20.resize(count, 0.0);
        for left in 0..count - 1 {
            let length = segment_length(path, left, 1.0);
            self.partial_lengths_20[left + 1] = self.partial_lengths_20[left] + length;
        }
        self.total_length_24 = self.partial_lengths_20[count - 1];
        Some(self.total_length_24)
    }

    // IDA: 006DBBE0 GetPercentLoc. pct == 0 gives (0, 0.0); pct < 0 wraps to f32(f32(fmod(pct, 1)) + 1); pct == 1 gives
    // (count-2, 1.0); pct > 1 becomes f32(fmod(pct, 1)); distance = f32(pct * total); first segment whose
    // partial[i+1] >= distance; target = f32(distance - partial[i]); t = f32(target / (partial[i+1] - partial[i]));
    // then up to 32 Newton steps t = f32(t - f32(len - target) / speed) until f32 |error| <= 1e-4 (NaN keeps iterating).
    // Completion: 100% of the verified branches. Host stops: an exhausted search (the original continues with t = the
    // integer bits of count) and a missing +20 table (the original faults).
    // TODO: [ ] Compare CRT __CIfmod and x87 intermediates at runtime.
    #[allow(clippy::neg_cmp_op_on_partial_ord)] // IDA: 006DBBE0 a NaN partial length does not stop the search.
    fn locate_constant_velocity(
        &self,
        path: &PositionKeyData,
        mut percent: f32,
    ) -> Option<Segment> {
        let count = path.keys().len();
        if percent == 0.0 {
            return Some(Segment {
                left: 0,
                ratio: 0.0,
            });
        }
        if percent < 0.0 {
            percent = float_fmod(percent, 1.0) + 1.0;
        }
        if percent == 1.0 {
            return Some(Segment {
                left: count - 2,
                ratio: 1.0,
            });
        }
        if 1.0 < percent {
            percent = float_fmod(percent, 1.0);
        }
        let partial = &self.partial_lengths_20;
        if partial.len() != count {
            return None;
        }
        let distance = percent * self.total_length_24;
        let mut left = 0;
        while left + 1 < count && !(partial[left + 1] >= distance) {
            left += 1;
        }
        if left + 1 >= count {
            return None;
        }
        let target = distance - partial[left];
        let mut ratio = target / (partial[left + 1] - partial[left]);
        let mut iteration = 0;
        loop {
            let error = segment_length(path, left, ratio) - target;
            if error.abs() <= NEWTON_TOLERANCE {
                break;
            }
            let speed = segment_speed(path, left, ratio);
            ratio -= error / speed;
            iteration += 1;
            if iteration >= NEWTON_ITERATIONS {
                break;
            }
        }
        Some(Segment { left, ratio })
    }

    // IDA: 006DBDB0 GetLoc. Constant velocity delegates to 006DBBE0 and stores the left index to +10. Otherwise
    // key0.time >= pct gives (0, 0.0) and last.time <= pct gives (count-2, 1.0), neither touching +10. The search starts
    // at +10, accepts key[i].time == pct or (key[i] < pct < key[i+1]), wraps once from count-2 to 0, stores +10 and
    // returns t = f32((pct - key[i]) / (key[i+1] - key[i])).
    // Completion: 100% of the verified comparisons. Host stop: the original computes with an uninitialized right index
    // when +10 >= count-1 or the search is exhausted (NaN pct).
    fn locate(&mut self, path: &PositionKeyData, percent: f32) -> Option<Segment> {
        if self.flags_0c & Self::CONSTANT_VELOCITY_FLAG != 0 {
            let found = self.locate_constant_velocity(path, percent);
            if let Some(found) = found {
                self.last_path_index_10 = found.left as u32;
            }
            return found;
        }
        let keys = path.keys();
        let count = keys.len();
        if keys[0].time >= percent {
            return Some(Segment {
                left: 0,
                ratio: 0.0,
            });
        }
        if keys[count - 1].time <= percent {
            return Some(Segment {
                left: count - 2,
                ratio: 1.0,
            });
        }
        let mut index = self.last_path_index_10 as usize;
        let mut may_wrap = true;
        while index < count - 1 {
            let left_time = keys[index].time;
            let right_time = keys[index + 1].time;
            if left_time == percent || (left_time < percent && right_time > percent) {
                self.last_path_index_10 = index as u32;
                return Some(Segment {
                    left: index,
                    ratio: ((percent - left_time) / (right_time - left_time)),
                });
            }
            if index == count - 2 && may_wrap {
                index = 0;
                may_wrap = false;
            } else {
                index += 1;
            }
        }
        None
    }

    // IDA: 006DAEB0 CalcRefFrame into a copy of the 00B26DC4 identity. Frenet flag 0x04: 006BBEE0 at t; with smoothing
    // s != 0 (NaN included), (1-s) <= t blends toward the next segment (open curves stop at the end, closed ones wrap to
    // 0) with a = f32(((1-t)+s)/(s+s)), and otherwise s >= t blends from the previous segment (closed wraps to count-1)
    // with c = f32((s-t)/(s+s)); normals are 0047DA10-scaled, 0047D9B0-added and normalized, curvature blended in f32,
    // binormal = tangent x normal. Columns are tangent, -normal, -binormal. Without 0x04 the columns are tangent,
    // unit((0,0,1) x tangent) and tangent x that. Flip 0x40 negates columns 0 / 1; follow axis 1 / 2 swaps column 0 with
    // the negated column 1 / 2; bank 0x08 rotates by f32(bankDir * maxBank), scaled by atan(k/kmax)*4/pi when kmax > k,
    // about the original tangent and premultiplies.
    // Completion: 100% of the verified branches, operands and store points
    // TODO: [ ] Compare CRT __CIatan and x87 fsincos bits at runtime.
    fn reference_frame(&self, path: &PositionKeyData, segment: Segment) -> Matrix3 {
        let mut frame = IDENTITY;
        let count = path.keys().len();
        if count < 2 {
            return frame;
        }
        let left = segment.left;
        let t = segment.ratio;
        let smoothing = self.smoothing_2c;
        let tangent;
        let mut curvature;
        if self.flags_0c & Self::FRENET_FRAME_FLAG != 0 {
            let current = coordinate_frame(path, left, t);
            tangent = current.tangent;
            let mut normal = current.normal;
            let mut binormal = current.binormal;
            curvature = current.curvature;
            if smoothing != 0.0 {
                let one_minus_smoothing = 1.0 - smoothing;
                if one_minus_smoothing <= t {
                    let mut next = left + 1;
                    if self.flags_0c & Self::OPEN_CURVE_FLAG == 0 && next == count - 1 {
                        next = 0;
                    }
                    if next < count - 1 {
                        let a = ((1.0 - t) + smoothing) / (smoothing + smoothing);
                        let b = 1.0 - a;
                        let end_time = 1.0 - smoothing;
                        let ending = coordinate_frame(path, left, end_time);
                        let following = coordinate_frame(path, next, smoothing);
                        normal = add(&scale(a, &ending.normal), &scale(b, &following.normal));
                        super::transform::normalize_vector(&mut normal);
                        curvature = following.curvature * b + a * ending.curvature;
                        binormal = cross(&tangent, &normal);
                    }
                } else if smoothing >= t {
                    let mut previous = left;
                    if self.flags_0c & Self::OPEN_CURVE_FLAG == 0 && previous == 0 {
                        previous = count - 1;
                    }
                    if previous >= 1 {
                        let c = (smoothing - t) / (smoothing + smoothing);
                        let e = 1.0 - c;
                        let starting = coordinate_frame(path, left, smoothing);
                        let start_time = 1.0 - smoothing;
                        let preceding = coordinate_frame(path, previous - 1, start_time);
                        normal = add(&scale(e, &starting.normal), &scale(c, &preceding.normal));
                        super::transform::normalize_vector(&mut normal);
                        curvature = preceding.curvature * c + e * starting.curvature;
                        binormal = cross(&tangent, &normal);
                    }
                }
            }
            for row in 0..3 {
                frame[row * 3] = tangent[row];
                frame[row * 3 + 1] = -normal[row];
                frame[row * 3 + 2] = -binormal[row];
            }
        } else {
            let current = coordinate_frame(path, left, t);
            tangent = current.tangent;
            curvature = current.curvature;
            let up = unit_cross(&UNIT_Z, &tangent);
            let side = cross(&tangent, &up);
            for row in 0..3 {
                frame[row * 3] = tangent[row];
                frame[row * 3 + 1] = up[row];
                frame[row * 3 + 2] = side[row];
            }
        }
        if self.flags_0c & Self::FLIP_FLAG != 0 {
            for row in 0..3 {
                frame[row * 3] = -tangent[row];
                frame[row * 3 + 1] = -frame[row * 3 + 1];
            }
        }
        if self.follow_axis_30 == 1 || self.follow_axis_30 == 2 {
            let swapped = self.follow_axis_30 as usize;
            for row in 0..3 {
                let column0 = frame[row * 3];
                frame[row * 3] = -frame[row * 3 + swapped];
                frame[row * 3 + swapped] = column0;
            }
        }
        if self.flags_0c & Self::BANK_FLAG != 0 {
            let mut angle =
                (f64::from(self.bank_direction_38) * f64::from(self.max_bank_angle_28)) as f32;
            if self.max_curvature_34 > curvature {
                let ratio = curvature / self.max_curvature_34;
                let arc = f64::from(ratio).atan() as f32;
                angle *= arc * FOUR_OVER_PI;
            }
            frame = super::transform::multiply(&axis_angle_rotation(angle, &tangent), &frame);
        }
        frame
    }

    // IDA: 006DC2C0 Update. time == +08 (never written here, so only -FLT_MAX): a position equal to 00B24FC8 stores
    // -FLT_MAX into translate.x, rotate.x and scale and returns false; otherwise SetTranslate(+4C) and, with follow,
    // SetRotate(+3C), true. Else null percent data or keys and path data or keys return false;
    // percent = f32(006BB270 over the percent keys and +14); 006DBDB0; follow computes +3C = 007150F0(006DAEB0);
    // +4C = 006DAE50 (B3D010 slot); then the setters and +58 = time. Output is changed only through 00471390 / 00471430
    // or the three -FLT_MAX stores.
    // Completion: 95% of the verified branches and math; x87 precision and CRT sqrt, atan, fmod and fsincos bits remain
    // host approximations. Host stops: reads beyond the path keys (count < 2) or past the percent keys.
    // TODO: [ ] Compare against captured original x87 results on real paths.
    pub fn update(
        &mut self,
        path: Option<&PositionKeyData>,
        percent: Option<&FloatKeyData>,
        time: f32,
        output: &mut QuatTransform,
    ) -> TransformUpdateStatus {
        if time == self.last_time_08 {
            if self.position_4c[0] == INVALID_TRANSFORM_COMPONENT
                && self.position_4c[1] == INVALID_TRANSFORM_COMPONENT
                && self.position_4c[2] == INVALID_TRANSFORM_COMPONENT
            {
                output.translate[0] = INVALID_TRANSFORM_COMPONENT;
                output.rotate[1] = INVALID_TRANSFORM_COMPONENT;
                output.scale = INVALID_TRANSFORM_COMPONENT;
                return TransformUpdateStatus::Invalid;
            }
            set_quat_transform_translate(output, &self.position_4c);
            if self.flags_0c & Self::FOLLOW_FLAG != 0 {
                set_quat_transform_rotate(output, &self.rotation_3c);
            }
            return TransformUpdateStatus::Valid;
        }
        let Some(percent) = percent.filter(|percent| !percent.keys().is_empty()) else {
            return TransformUpdateStatus::Invalid;
        };
        let Some(path) = path.filter(|path| !path.keys().is_empty()) else {
            return TransformUpdateStatus::Invalid;
        };
        let mut percent_index = self.last_percent_index_14;
        let Some(sampled) =
            sample_float_key_sequence(percent, percent.keys().len(), time, &mut percent_index)
        else {
            return TransformUpdateStatus::UnsupportedOutOfRange;
        };
        self.last_percent_index_14 = percent_index;
        if path.keys().len() < 2 {
            return TransformUpdateStatus::UnsupportedOutOfRange;
        }
        let Some(segment) = self.locate(path, sampled) else {
            return TransformUpdateStatus::UnsupportedOutOfRange;
        };
        if self.flags_0c & Self::FOLLOW_FLAG != 0 {
            self.rotation_3c = quaternion_from_rotation(&self.reference_frame(path, segment));
        }
        self.position_4c = required(path.interpolate(segment.left, segment.ratio));
        set_quat_transform_translate(output, &self.position_4c);
        if self.flags_0c & Self::FOLLOW_FLAG != 0 {
            set_quat_transform_rotate(output, &self.rotation_3c);
        }
        self.path_last_time_58 = time;
        TransformUpdateStatus::Valid
    }
}

// IDA: N/A resolves 006DC000's two link targets. The stream link check guarantees the classes, so a dangling link is a
// host defect.
// Completion: 100%
pub fn link_path_interpolator(
    world: &mut World,
    interpolator: Entity,
    path: Option<Entity>,
    percent: Option<Entity>,
) -> PathLinkStatus {
    let mut state = std::mem::take(
        &mut *world
            .get_mut::<PathInterpolator>(interpolator)
            .expect("a NiPathInterpolator entity"),
    );
    let path = path.map(|path| {
        (
            path,
            world
                .get::<PositionKeyData>(path)
                .expect("linked NiPosData"),
        )
    });
    let status = state.link(path, percent);
    *world
        .get_mut::<PathInterpolator>(interpolator)
        .expect("a NiPathInterpolator entity") = state;
    status
}

// IDA: N/A resolves +18 and +1C for 006DC2C0.
// Completion: 100%
pub fn update_path_interpolator(
    world: &mut World,
    interpolator: Entity,
    time: f32,
    output: &mut QuatTransform,
) -> TransformUpdateStatus {
    let mut state = std::mem::take(
        &mut *world
            .get_mut::<PathInterpolator>(interpolator)
            .expect("a NiPathInterpolator entity"),
    );
    let path = state.path_18.map(|path| {
        world
            .get::<PositionKeyData>(path)
            .expect("linked NiPosData")
    });
    let percent = state
        .percent_1c
        .map(|data| world.get::<FloatKeyData>(data).expect("linked NiFloatData"));
    let status = state.update(path, percent, time, output);
    *world
        .get_mut::<PathInterpolator>(interpolator)
        .expect("a NiPathInterpolator entity") = state;
    status
}

#[cfg(test)]
mod tests {
    use super::super::animation_stream::decode_float_key_payload;
    use super::super::position_animation::decode_position_key_payload;
    use super::super::transform_animation::quat_transform_invalid;
    use super::*;

    const MODERN_VERSION: u32 = 0x1400_0004;
    const NEGATIVE_MAX: f32 = -f32::MAX;
    /// The test's 0.70710677F.
    const ROOT_HALF: f32 = f32::from_bits(0x3F35_04F3);

    fn near(actual: f32, expected: f32, tolerance: f32) -> bool {
        (actual - expected).abs() <= tolerance
    }

    fn u32_le(output: &mut Vec<u8>, value: u32) {
        output.extend(value.to_le_bytes());
    }

    fn f32_le(output: &mut Vec<u8>, value: f32) {
        u32_le(output, value.to_bits());
    }

    /// IDA: 006D9F90 linear NiPosData: (0,0,0) at 0, (10,0,0) at 0.5, (10,10,0) at 1: an L-shaped path with two 10-unit
    /// segments.
    fn l_path(world: &mut World) -> Entity {
        let mut bytes = Vec::new();
        u32_le(&mut bytes, 3);
        u32_le(&mut bytes, 1);
        for value in [
            0.0, 0.0, 0.0, 0.0, 0.5, 10.0, 0.0, 0.0, 1.0, 10.0, 10.0, 0.0,
        ] {
            f32_le(&mut bytes, value);
        }
        let data = decode_position_key_payload(&bytes, MODERN_VERSION)
            .unwrap()
            .data;
        world.spawn(data).id()
    }

    /// IDA: 006E3690 linear NiFloatData percent track 0 -> 1 over [0, 1].
    fn unit_percent(world: &mut World) -> Entity {
        let mut bytes = Vec::new();
        u32_le(&mut bytes, 2);
        u32_le(&mut bytes, 1);
        for value in [0.0, 0.0, 1.0, 1.0] {
            f32_le(&mut bytes, value);
        }
        let data = decode_float_key_payload(&bytes).unwrap().data;
        world.spawn(data).id()
    }

    fn spawn_loaded(world: &mut World, flags: u16) -> Entity {
        let payload = PathInterpolatorPayload {
            flags,
            bank_direction: 1,
            ..PathInterpolatorPayload::default()
        };
        let mut path = PathInterpolator::default();
        path.apply_loaded_fields(&payload);
        world.spawn(path).id()
    }

    fn state(world: &World, entity: Entity) -> &PathInterpolator {
        world.get::<PathInterpolator>(entity).unwrap()
    }

    /// Ported from `path_interpolator_tests`: IDA 006DBF20's modern read and 006DC1D0's cold state.
    #[test]
    fn payload_and_cold_state() {
        let mut bytes = Vec::new();
        bytes.extend(0x0031u16.to_le_bytes());
        u32_le(&mut bytes, 0xFFFF_FFFF);
        f32_le(&mut bytes, 0.5);
        f32_le(&mut bytes, 0.25);
        bytes.extend(0xFFFEu16.to_le_bytes());
        u32_le(&mut bytes, 4);
        u32_le(&mut bytes, 5);
        bytes.push(0xA5);
        let payload = parse_path_interpolator_payload(&bytes, MODERN_VERSION).unwrap();
        assert_eq!(payload.bytes_consumed, 24);
        assert_eq!(payload.flags, 0x0031);
        assert_eq!(payload.bank_direction, -1);
        assert_eq!(payload.max_bank_angle, 0.5);
        assert_eq!(payload.smoothing, 0.25);
        assert_eq!(payload.follow_axis, -2);
        assert_eq!(payload.path_data_index, 4);
        assert_eq!(payload.percent_data_index, 5);

        let mut cold = PathInterpolator::default();
        assert_eq!(cold.flags_0c, 0x0003);
        assert_eq!(cold.total_length_24, -1.0);
        assert_eq!(cold.bank_direction_38, 1);
        assert_eq!(cold.last_time_08, NEGATIVE_MAX);
        assert_eq!(cold.path_last_time_58, NEGATIVE_MAX);
        assert_eq!(cold.position_4c[0], NEGATIVE_MAX);
        assert_eq!(cold.rotation_3c[0], NEGATIVE_MAX);
        assert!(cold.path_18.is_none());
        assert!(cold.percent_1c.is_none());
        cold.apply_loaded_fields(&payload);
        assert_eq!(cold.flags_0c, 0x0031);
        assert_eq!(cold.bank_direction_38, -1);
        assert_eq!(cold.max_bank_angle_28, 0.5);
        assert_eq!(cold.smoothing_2c, 0.25);
        assert_eq!(cold.follow_axis_30, -2);

        assert!(parse_path_interpolator_payload(&bytes[..20], MODERN_VERSION).is_err());
    }

    /// IDA: 006DC000 link and 006DC2C0 / 006DBDB0 Update over a linear path.
    #[test]
    fn key_time_update() {
        let mut world = World::new();
        let path = spawn_loaded(&mut world, 0x0002);
        let (data, percent) = (l_path(&mut world), unit_percent(&mut world));
        assert_eq!(
            link_path_interpolator(&mut world, path, Some(data), Some(percent)),
            PathLinkStatus::Linked
        );
        assert_eq!(state(&world, path).flags_0c, 0x0003);
        assert_eq!(state(&world, path).max_curvature_34, 0.0);

        let mut output = QuatTransform::default();
        assert_eq!(
            update_path_interpolator(&mut world, path, NEGATIVE_MAX, &mut output),
            TransformUpdateStatus::Invalid
        );
        assert!(quat_transform_invalid(&output));
        assert_eq!(
            update_path_interpolator(&mut world, path, 0.25, &mut output),
            TransformUpdateStatus::Valid
        );
        assert_eq!(output.translate, [5.0, 0.0, 0.0]);
        assert_eq!(output.rotate[1], NEGATIVE_MAX);
        assert_eq!(state(&world, path).path_last_time_58, 0.25);
        assert_eq!(state(&world, path).last_path_index_10, 0);
        assert_eq!(
            update_path_interpolator(&mut world, path, 0.75, &mut output),
            TransformUpdateStatus::Valid
        );
        assert_eq!(output.translate, [10.0, 5.0, 0.0]);
        assert_eq!(state(&world, path).last_path_index_10, 1);
        assert_eq!(
            update_path_interpolator(&mut world, path, 1.0, &mut output),
            TransformUpdateStatus::Valid
        );
        assert_eq!(output.translate, [10.0, 10.0, 0.0]);
        assert_eq!(
            update_path_interpolator(&mut world, path, 1.5, &mut output),
            TransformUpdateStatus::UnsupportedOutOfRange
        );

        let unlinked = world.spawn(PathInterpolator::default()).id();
        let mut unlinked_output = QuatTransform::default();
        assert_eq!(
            update_path_interpolator(&mut world, unlinked, 0.5, &mut unlinked_output),
            TransformUpdateStatus::Invalid
        );
    }

    /// IDA: 006DAEB0's follow frame without Frenet and 007150F0's FromRotation trace branch.
    #[test]
    fn follow_rotation() {
        let mut world = World::new();
        let path = spawn_loaded(&mut world, 0x0022);
        let (data, percent) = (l_path(&mut world), unit_percent(&mut world));
        link_path_interpolator(&mut world, path, Some(data), Some(percent));
        let mut output = QuatTransform::default();
        assert_eq!(
            update_path_interpolator(&mut world, path, 0.25, &mut output),
            TransformUpdateStatus::Valid
        );
        assert_eq!(output.rotate, [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(
            update_path_interpolator(&mut world, path, 0.75, &mut output),
            TransformUpdateStatus::Valid
        );
        assert!(near(output.rotate[0], ROOT_HALF, 1e-6));
        assert_eq!(output.rotate[1], 0.0);
        assert_eq!(output.rotate[2], 0.0);
        assert!(near(output.rotate[3], ROOT_HALF, 1e-6));
    }

    /// IDA: 006DBB10's constant-velocity table and 006DBBE0 GetPercentLoc with Newton refinement; zero keys stop the host.
    #[test]
    fn constant_velocity() {
        let mut world = World::new();
        let path = spawn_loaded(&mut world, 0x0012);
        let (data, percent) = (l_path(&mut world), unit_percent(&mut world));
        assert_eq!(
            link_path_interpolator(&mut world, path, Some(data), Some(percent)),
            PathLinkStatus::Linked
        );
        let linked = state(&world, path);
        assert_eq!(linked.flags_0c & 0x0001, 0);
        assert_eq!(linked.partial_lengths_20.len(), 3);
        assert_eq!(linked.partial_lengths_20[0], 0.0);
        assert!(near(linked.partial_lengths_20[1], 10.0, 1e-4));
        assert!(near(linked.total_length_24, 20.0, 1e-4));
        let mut output = QuatTransform::default();
        assert_eq!(
            update_path_interpolator(&mut world, path, 0.25, &mut output),
            TransformUpdateStatus::Valid
        );
        assert!(near(output.translate[0], 5.0, 1e-3));
        assert_eq!(output.translate[1], 0.0);
        assert_eq!(
            update_path_interpolator(&mut world, path, 0.75, &mut output),
            TransformUpdateStatus::Valid
        );
        assert_eq!(output.translate[0], 10.0);
        assert!(near(output.translate[1], 5.0, 1e-3));

        let empty = spawn_loaded(&mut world, 0x0010);
        let no_keys = world.spawn(PositionKeyData::default()).id();
        let percent = unit_percent(&mut world);
        assert_eq!(
            link_path_interpolator(&mut world, empty, Some(no_keys), Some(percent)),
            PathLinkStatus::UnsupportedConstantVelocityWithoutKeys
        );
    }
}
