//! The B-spline interpolators a KF can hold: NiBSplineData, NiBSplineBasisData, and the compressed transform and float interpolators over the NiBSplineInterpolator base.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};
use super::transform_animation::{
    QuatTransform, decode_quat_transform, normalize_approximate, quat_transform_invalid,
    set_quat_transform_rotate, set_quat_transform_scale, set_quat_transform_translate,
};

const TRUNCATED: &str = "truncated B-spline payload";

/// IDA: 006ED140 NiBSplineInterpolator (0x1C, over 006EC2B0 NiInterpolator): +0x08 the last time (NiInterpolator's -FLT_MAX), +0x0C the start time (FLT_MAX), +0x10 the end time (-FLT_MAX), +0x14 the NiBSplineData, +0x18 the NiBSplineBasisData (both counted).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BSplineInterpolatorBase {
    pub last_time_08: f32,
    pub start_time_0c: f32,
    pub end_time_10: f32,
    pub data_14: Option<Entity>,
    pub basis_18: Option<Entity>,
}

impl Default for BSplineInterpolatorBase {
    /// IDA: 006ED140 with null data and basis.
    fn default() -> Self {
        Self {
            last_time_08: -f32::MAX,
            start_time_0c: f32::MAX,
            end_time_10: -f32::MAX,
            data_14: None,
            basis_18: None,
        }
    }
}

/// IDA: 006E5BF0's factory (0x60): 006E5920(0, 0xFFFF, 0xFFFF, 0xFFFF, 0) over 006E4930 NiBSplineTransformInterpolator: +0x1C the NiQuatTransform (00B24260's translate, 00B3CBA4's rotate, -FLT_MAX scale), +0x3C / +0x40 / +0x44 the translate, rotate and scale handles into the data's shorts (0xFFFF none); then vtable A7BC5C and +0x48..+0x5C the translate, rotate and scale offsets and half ranges (FLT_MAX). Completion: the cold state, 006E5C70's load, 006E5B40's clone and 006E5990's Update (`update_comp_transform`).
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct BSplineCompTransformInterpolator {
    pub base: BSplineInterpolatorBase,
    pub value_1c: QuatTransform,
    pub translate_handle_3c: u32,
    pub rotate_handle_40: u32,
    pub scale_handle_44: u32,
    pub ranges_48: [f32; 6],
}

impl Default for BSplineCompTransformInterpolator {
    /// IDA: 006E5920(0, 0xFFFF, 0xFFFF, 0xFFFF, 0).
    fn default() -> Self {
        Self {
            base: BSplineInterpolatorBase::default(),
            value_1c: QuatTransform::default(),
            translate_handle_3c: 0xFFFF,
            rotate_handle_40: 0xFFFF,
            scale_handle_44: 0xFFFF,
            ranges_48: [f32::MAX; 6],
        }
    }
}

/// IDA: 006E6220's factory (0x2C): 006E5490(0, 0xFFFF, 0) over 006ED140 NiBSplineFloatInterpolator: +0x1C the value (the constructor doesn't store it: FormHeap's bytes, 0 here), +0x20 the handle (0xFFFF); then vtable A7BDDC and +0x24 / +0x28 the offset and half range (FLT_MAX). Completion: the cold state, 006E62A0's load and 006E6170's clone; its float Update (vt+0x5C 006E60E0) isn't ported.
/// TODO: [ ] 006E60E0 Update.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct BSplineCompFloatInterpolator {
    pub base: BSplineInterpolatorBase,
    pub value_1c: f32,
    pub handle_20: u32,
    pub offset_24: f32,
    pub half_range_28: f32,
}

impl Default for BSplineCompFloatInterpolator {
    /// IDA: 006E5490(0, 0xFFFF, 0) and A7BDDC's two floats.
    fn default() -> Self {
        Self {
            base: BSplineInterpolatorBase::default(),
            value_1c: 0.0,
            handle_20: 0xFFFF,
            offset_24: f32::MAX,
            half_range_28: f32::MAX,
        }
    }
}

/// IDA: 006E71E0's factory (0x18, vtable A7C044): +0x08 the floats, +0x0C the shorts, +0x10 / +0x14 their counts. Shared by clones (vt+0x18 00700640). Completion: the cold state and 006E7640's load.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct BSplineData {
    pub floats_08: Vec<f32>,
    pub shorts_0c: Vec<i16>,
}

/// IDA: 006E7990's factory (0x28, vtable A7C0CC): NiBSplineBasis<float, 3> at +0x08: +0x08 the control point count, +0x0C..+0x18 the four basis values (the constructor leaves them: 0 here), +0x1C the last time (-1.0), +0x20 the min (0) and +0x24 the max (3). Completion: the cold state, 006E7A80's load and 006E78F0's clone (all 32 bytes from +0x08).
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct BSplineBasisData {
    pub control_points_08: u32,
    pub values_0c: [f32; 4],
    pub last_time_1c: f32,
    pub min_20: i32,
    pub max_24: i32,
}

impl Default for BSplineBasisData {
    /// IDA: 006E7990's stores.
    fn default() -> Self {
        Self {
            control_points_08: 0,
            values_0c: [0.0; 4],
            last_time_1c: -1.0,
            min_20: 0,
            max_24: 3,
        }
    }
}

impl BSplineBasisData {
    /// IDA: 006E6B50 NiBSplineBasis<float, 3>::Compute(t, &min, &max) on +0x08: a time equal to +0x1C (bitwise) answers +0x20 and
    /// +0x24; otherwise +0x1C = t, the span i = count - 1 at t >= 1, else 3 + (int)((count - 3) * t), min = i - 3, max = i, and the
    /// four values at +0x0C from u = t * (count - 3): closed forms for 4, 5 and 6 control points, the general open-uniform
    /// recurrence from 7 (none below 4). x87 at 24-bit precision (D3D9's default after CreateDevice), so f32 arithmetic with the
    /// decompiler's store points. Completion: 100% of the branches.
    pub fn compute(&mut self, time: f32) -> (i32, i32) {
        if time.to_bits() == self.last_time_1c.to_bits() {
            return (self.min_20, self.max_24);
        }
        let count = self.control_points_08 as i32;
        self.last_time_1c = time;
        let quantity = count - 3;
        let span = if time >= 1.0 {
            count - 1
        } else {
            (quantity as f32 * time) as i32 + 3
        };
        self.min_20 = span - 3;
        self.max_24 = span;
        let u = time * quantity as f32;
        let third = 0.333_333_34_f32;
        let values = &mut self.values_0c;
        match count {
            6 if span == 3 => {
                let a = 1.0 - u;
                let b = 2.0 - u;
                let half = u * 0.5;
                let a2 = a * a;
                let c = 0.5 * (half * b + a * u);
                let d = half * (u * third);
                values[0] = a * a2;
                values[1] = a2 * u + b * c;
                let e = 3.0 - u;
                values[2] = c * u + e * d;
                values[3] = u * d;
            }
            6 if span == 4 => {
                let a = u - 1.0;
                let b = 3.0 - u;
                let half = u * 0.5;
                let c = 1.0 - half;
                let d = 0.5 * a;
                let c2 = c * c;
                let e = (c * u + d * b) * third;
                let d2 = d * d;
                let f = 2.0 - u;
                values[0] = f * c2;
                values[1] = c2 * u + e * b;
                values[2] = u * e + b * d2;
                values[3] = d2 * a;
            }
            6 => {
                let a = u - 1.0;
                let b = u - 2.0;
                let c = 3.0 - u;
                let half = c * 0.5;
                let b2 = b * b;
                let d = c * third * half;
                let e = 0.5 * (half * a + c * b);
                values[0] = d * c;
                values[1] = u * d + e * c;
                values[2] = a * e + c * b2;
                values[3] = b2 * b;
            }
            5 if span == 3 => {
                let a = 1.0 - u;
                let b = 2.0 - u;
                let half = u * 0.5;
                let a2 = a * a;
                let c = 0.5 * (half * b + a * u);
                let h2 = half * half;
                values[0] = a * a2;
                values[1] = a2 * u + c * b;
                values[2] = c * u + b * h2;
                values[3] = u * h2;
            }
            5 => {
                let a = u - 1.0;
                let b = 2.0 - u;
                let half = u * 0.5;
                let a2 = a * a;
                let c = 1.0 - half;
                let c2 = c * c;
                let d = (half + a) * c;
                values[0] = c2 * b;
                values[1] = c2 * u + d * b;
                values[2] = u * d + b * a2;
                values[3] = a2 * a;
            }
            4 => {
                let a = 1.0 - u;
                let u2 = u * u;
                let a2 = a * a;
                values[0] = a2 * a;
                values[1] = a2 * (u * 3.0);
                values[2] = a * (3.0 * u2);
                values[3] = u * u2;
            }
            _ if count < 7 => {}
            _ => {
                let next = quantity + 1;
                let last = quantity + 2;
                let knot_5 = if span <= 5 { 0.0 } else { (span - 5) as f32 };
                let knot_4 = if span > 4 { (span - 4) as f32 } else { 0.0 };
                let knot_1 = if span >= next {
                    quantity as f32
                } else {
                    (span - 1) as f32
                };
                let knot_0 = if span < quantity {
                    span as f32
                } else {
                    quantity as f32
                };
                let w_a = if span == 3 { 1.0 } else { 0.5 };
                let w_b = if span == last { 1.0 } else { 0.5 };
                let w_c = match span {
                    3 => 1.0,
                    4 => 0.5,
                    _ => third,
                };
                let w_d = if span == 3 || span == last {
                    0.5
                } else {
                    third
                };
                let w_e = if span == last {
                    1.0
                } else if span == next {
                    0.5
                } else {
                    third
                };
                let d4 = u - knot_4;
                let lower = (span - 3) as f32;
                let d3 = u - lower;
                let upper = (span - 2) as f32;
                let e2 = upper - u;
                let e1 = knot_1 - u;
                let f2 = e2 * w_a;
                let f3 = d3 * w_b;
                let g = e2 * w_c * f2;
                let h = (f2 * d4 + f3 * e1) * w_d;
                let k = f3 * (d3 * w_e);
                values[0] = e2 * g;
                let d5 = u - knot_5;
                values[1] = e1 * h + g * d5;
                let e0 = knot_0 - u;
                values[2] = h * d4 + e0 * k;
                values[3] = d3 * k;
            }
        }
        (self.min_20, self.max_24)
    }
}

/// IDA: 006E7470 NiBSplineData::GetValueDegree3Compact(data, t, out, dimension, basis, handle, offset, half range): the basis's
/// Compute (006E78D0's +0x08), 00730840 decompressing the 4 * dimension shorts from +0x0C[handle + dimension * min] (each
/// f32(s / 32767.0 * half + offset)), then out = value 0 times the first point, plus value k times point k for k up to
/// max - min. Completion: 100%; a handle past the shorts (an out-of-bounds read in the original) is a stop.
/// # Errors
/// A handle past the data's shorts.
pub fn compact_value_degree3(
    data: &BSplineData,
    basis: &mut BSplineBasisData,
    time: f32,
    out: &mut [f32],
    handle: u32,
    offset: f32,
    half_range: f32,
) -> Result<(), StreamError> {
    let dimension = out.len();
    let (min, max) = basis.compute(time);
    let start = handle as usize + dimension * usize::try_from(min).unwrap_or(0);
    let shorts = data
        .shorts_0c
        .get(start..start + 4 * dimension)
        .ok_or(StreamError(
            "006E74A0: a B-spline handle past the data's shorts",
        ))?;
    let points: Vec<f32> = shorts
        .iter()
        .map(|&short| f32::from(short) / 32767.0 * half_range + offset)
        .collect();
    let first = basis.values_0c[0];
    for (slot, point) in out.iter_mut().zip(&points) {
        *slot = point * first;
    }
    let mut at = dimension;
    for k in 1..=usize::try_from(max - min).unwrap_or(0) {
        let weight = basis.values_0c[k];
        for slot in out.iter_mut() {
            *slot += points[at] * weight;
            at += 1;
        }
    }
    Ok(())
}

/// IDA: 006E5990 NiBSplineCompTransformInterpolator vt+0x4C Update(time, owner, out): a time equal to +0x08 copies +0x1C out
/// and answers it valid; otherwise t = (time - start) / (end - start), and each channel with a handle: translate (dimension 3,
/// +0x48 / +0x4C) through SetTranslate, rotate (4, +0x50 / +0x54) as (w, x, y, z) through 0072FAC0 and SetRotate, scale (1,
/// +0x58 / +0x5C) through SetScale; +0x1C copied out, and a valid value stores +0x08. Completion: 100%.
/// # Errors
/// A missing data or basis, or `compact_value_degree3`'s stop.
pub fn update_comp_transform(
    world: &mut World,
    interpolator: Entity,
    time: f32,
    output: &mut QuatTransform,
) -> Result<bool, StreamError> {
    let mut state = *world
        .get::<BSplineCompTransformInterpolator>(interpolator)
        .ok_or(StreamError(
            "006E5990: not a NiBSplineCompTransformInterpolator",
        ))?;
    if time == state.base.last_time_08 {
        *output = state.value_1c;
        return Ok(!quat_transform_invalid(&state.value_1c));
    }
    let t = (time - state.base.start_time_0c) / (state.base.end_time_10 - state.base.start_time_0c);
    let handles = [
        state.translate_handle_3c,
        state.rotate_handle_40,
        state.scale_handle_44,
    ];
    if handles.iter().any(|&handle| handle != 0xFFFF) {
        let (Some(data), Some(basis)) = (state.base.data_14, state.base.basis_18) else {
            return Err(StreamError(
                "006E59D4: a B-spline with no data or basis faults",
            ));
        };
        let data = world
            .get::<BSplineData>(data)
            .ok_or(StreamError(
                "006E59D4: a B-spline with no data or basis faults",
            ))?
            .clone();
        let mut basis_state = *world.get::<BSplineBasisData>(basis).ok_or(StreamError(
            "006E59D4: a B-spline with no data or basis faults",
        ))?;
        let ranges = state.ranges_48;
        let mut evaluate = |handle: u32, out: &mut [f32], range: usize| {
            compact_value_degree3(
                &data,
                &mut basis_state,
                t,
                out,
                handle,
                ranges[range],
                ranges[range + 1],
            )
        };
        if handles[0] != 0xFFFF {
            let mut value = [0.0; 3];
            evaluate(handles[0], &mut value, 0)?;
            set_quat_transform_translate(&mut state.value_1c, &value);
        }
        if handles[1] != 0xFFFF {
            let mut value = [0.0; 4];
            evaluate(handles[1], &mut value, 2)?;
            let rotation = normalize_approximate(&value);
            set_quat_transform_rotate(&mut state.value_1c, &rotation);
        }
        if handles[2] != 0xFFFF {
            let mut value = [0.0; 1];
            evaluate(handles[2], &mut value, 4)?;
            set_quat_transform_scale(&mut state.value_1c, value[0]);
        }
        *world
            .get_mut::<BSplineBasisData>(basis)
            .expect("read above") = basis_state;
    }
    *output = state.value_1c;
    let valid = !quat_transform_invalid(&state.value_1c);
    if valid {
        state.base.last_time_08 = time;
    }
    *world
        .get_mut::<BSplineCompTransformInterpolator>(interpolator)
        .expect("read above") = state;
    Ok(valid)
}

// IDA: 00712A90-style immediate reference (NiStream_ReadObjectReference): FFFFFFFF null, else the object table's entity of that class.
fn immediate(
    world: &World,
    table: &StreamObjectTable,
    index: u32,
    class: NiClass,
) -> Result<Option<Entity>, StreamError> {
    if index == NULL_REFERENCE {
        return Ok(None);
    }
    let object = table
        .objects
        .get(index as usize)
        .ok_or(StreamError("B-spline reference out of range"))?;
    if world.get::<NiClass>(object.entity) != Some(&class) {
        return Err(StreamError("B-spline reference of the wrong class"));
    }
    Ok(Some(object.entity))
}

// IDA: 006ED420 NiBSplineInterpolator::LoadBinary: 006EC2B0's modern base (no bytes), +0x0C and +0x10 (4 bytes each), then the data and basis as immediate references.
fn load_base(
    world: &World,
    table: &StreamObjectTable,
    reader: &mut ByteReader<'_>,
    base: &mut BSplineInterpolatorBase,
) -> Result<(), StreamError> {
    base.start_time_0c = f32::from_bits(reader.u32(TRUNCATED)?);
    base.end_time_10 = f32::from_bits(reader.u32(TRUNCATED)?);
    let data = reader.u32(TRUNCATED)?;
    base.data_14 = immediate(world, table, data, NiClass::BSplineData)?;
    let basis = reader.u32(TRUNCATED)?;
    base.basis_18 = immediate(world, table, basis, NiClass::BSplineBasisData)?;
    Ok(())
}

/// IDA: 006E5C70 NiBSplineCompTransformInterpolator::LoadBinary: 006E4E60 (006ED420, 006CB990's NiQuatTransform into +0x1C, the three handles +0x3C, +0x40, +0x44), then 24 bytes into +0x48. Completion: 100% of a modern stream.
/// # Errors
/// A short payload, or a reference of the wrong class.
pub fn load_comp_transform_payload(
    world: &mut World,
    table: &StreamObjectTable,
    entity: Entity,
    bytes: &[u8],
    version: u32,
) -> Result<usize, StreamError> {
    let mut state = *world
        .get::<BSplineCompTransformInterpolator>(entity)
        .expect("a NiBSplineCompTransformInterpolator");
    let mut reader = ByteReader::new(bytes);
    load_base(world, table, &mut reader, &mut state.base)?;
    let at = reader.position();
    let transform = decode_quat_transform(&bytes[at..], version)?;
    state.value_1c = transform.value;
    let mut reader = ByteReader::new(&bytes[at + transform.bytes_consumed..]);
    state.translate_handle_3c = reader.u32(TRUNCATED)?;
    state.rotate_handle_40 = reader.u32(TRUNCATED)?;
    state.scale_handle_44 = reader.u32(TRUNCATED)?;
    for range in &mut state.ranges_48 {
        *range = f32::from_bits(reader.u32(TRUNCATED)?);
    }
    *world
        .get_mut::<BSplineCompTransformInterpolator>(entity)
        .expect("checked above") = state;
    Ok(at + transform.bytes_consumed + reader.position())
}

/// IDA: 006E62A0 NiBSplineCompFloatInterpolator::LoadBinary: 006E5740 (006ED420, +0x1C, +0x20), then 8 bytes into +0x24. Completion: 100%.
/// # Errors
/// A short payload, or a reference of the wrong class.
pub fn load_comp_float_payload(
    world: &mut World,
    table: &StreamObjectTable,
    entity: Entity,
    bytes: &[u8],
) -> Result<usize, StreamError> {
    let mut state = *world
        .get::<BSplineCompFloatInterpolator>(entity)
        .expect("a NiBSplineCompFloatInterpolator");
    let mut reader = ByteReader::new(bytes);
    load_base(world, table, &mut reader, &mut state.base)?;
    state.value_1c = f32::from_bits(reader.u32(TRUNCATED)?);
    state.handle_20 = reader.u32(TRUNCATED)?;
    state.offset_24 = f32::from_bits(reader.u32(TRUNCATED)?);
    state.half_range_28 = f32::from_bits(reader.u32(TRUNCATED)?);
    *world
        .get_mut::<BSplineCompFloatInterpolator>(entity)
        .expect("checked above") = state;
    Ok(reader.position())
}

/// IDA: 006E7640 NiBSplineData::LoadBinary: the float count (+0x10) and its floats (+0x08), then from 20.0.0.1 the short count (+0x14) and its shorts (+0x0C). Completion: 100%.
/// # Errors
/// A short payload.
pub fn load_data_payload(
    world: &mut World,
    entity: Entity,
    bytes: &[u8],
    version: u32,
) -> Result<usize, StreamError> {
    let mut reader = ByteReader::new(bytes);
    let count = reader.u32(TRUNCATED)? as usize;
    let mut floats = Vec::with_capacity(count.min(bytes.len() / 4));
    for _ in 0..count {
        floats.push(f32::from_bits(reader.u32(TRUNCATED)?));
    }
    let mut shorts = Vec::new();
    if version >= 0x1400_0001 {
        let count = reader.u32(TRUNCATED)? as usize;
        shorts.reserve(count.min(bytes.len() / 2));
        for _ in 0..count {
            shorts.push(reader.u16(TRUNCATED)? as i16);
        }
    }
    let mut state = world
        .get_mut::<BSplineData>(entity)
        .expect("a NiBSplineData");
    state.floats_08 = floats;
    state.shorts_0c = shorts;
    Ok(reader.position())
}

/// IDA: 006E7A80 NiBSplineBasisData::LoadBinary: the control point count into +0x08. Completion: 100%.
/// # Errors
/// A short payload.
pub fn load_basis_payload(
    world: &mut World,
    entity: Entity,
    bytes: &[u8],
) -> Result<usize, StreamError> {
    let mut reader = ByteReader::new(bytes);
    let count = reader.u32(TRUNCATED)?;
    world
        .get_mut::<BSplineBasisData>(entity)
        .expect("a NiBSplineBasisData")
        .control_points_08 = count;
    Ok(reader.position())
}

/// IDA: the B-spline interpolators' CopyMembers past the copy itself (006ED2B0): +0x08 is the new object's (-FLT_MAX), the data shared, the basis through 00700710 (the clone map, else its vt+0x18 006E78F0). Completion: 100%.
/// # Errors
/// As `create_clone`.
pub(super) fn fix_clone(
    world: &mut World,
    process: &mut super::clone::CloningProcess,
    clone: Entity,
) -> Result<(), StreamError> {
    let base = match world.get::<NiClass>(clone).copied() {
        Some(NiClass::BSplineCompTransformInterpolator) => {
            world
                .get::<BSplineCompTransformInterpolator>(clone)
                .expect("copied")
                .base
        }
        Some(NiClass::BSplineCompFloatInterpolator) => {
            world
                .get::<BSplineCompFloatInterpolator>(clone)
                .expect("copied")
                .base
        }
        _ => return Ok(()),
    };
    let basis = match base.basis_18 {
        Some(basis) => Some(super::clone::clone_or_mapped(world, process, basis)?),
        None => None,
    };
    let fixed = BSplineInterpolatorBase {
        last_time_08: -f32::MAX,
        basis_18: basis,
        ..base
    };
    if let Some(mut state) = world.get_mut::<BSplineCompTransformInterpolator>(clone) {
        state.base = fixed;
    } else if let Some(mut state) = world.get_mut::<BSplineCompFloatInterpolator>(clone) {
        state.base = fixed;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 006E6B50: the four values are a partition of unity over each control point count's closed form and the general recurrence.
    #[test]
    fn the_basis_sums_to_one() {
        for count in [4_u32, 5, 6, 7, 10, 33] {
            for step in 0..=20 {
                let time = step as f32 / 20.0;
                let mut basis = BSplineBasisData {
                    control_points_08: count,
                    ..BSplineBasisData::default()
                };
                let (min, max) = basis.compute(time);
                assert_eq!(max - min, 3);
                assert!(max < count as i32, "{count} {time}");
                let sum: f32 = basis.values_0c.iter().sum();
                assert!(
                    (sum - 1.0).abs() < 1e-5,
                    "{count} at {time}: {:?}",
                    basis.values_0c
                );
                assert!(basis.values_0c.iter().all(|&value| value >= -1e-6));
            }
        }
    }

    // 00730840 and 006E7470: a constant curve decompresses to its value.
    #[test]
    fn a_constant_curve_evaluates_to_its_value() {
        let data = BSplineData {
            floats_08: Vec::new(),
            shorts_0c: vec![16384; 8 * 2],
        };
        let mut basis = BSplineBasisData {
            control_points_08: 8,
            ..BSplineBasisData::default()
        };
        let mut out = [0.0; 2];
        compact_value_degree3(&data, &mut basis, 0.4, &mut out, 0, 1.0, 2.0).unwrap();
        let expected = 16384.0 / 32767.0 * 2.0 + 1.0;
        assert!(
            out.iter().all(|&value| (value - expected).abs() < 1e-5),
            "{out:?}"
        );
    }
}
