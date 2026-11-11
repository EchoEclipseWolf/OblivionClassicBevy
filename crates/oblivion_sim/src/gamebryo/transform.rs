//! NiTransform: 0x34 bytes, a 3x3 rotation, a translation and a scale.
//!
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. No direct call from WinMain before 0040E865 reaches 0053D7A0, 007100A0, 007101F0,
//! 00718A80 or 0043F350. Limit of `f32`: intermediates here cannot leave the float exponent range where x87 registers can.
//! TODO: [ ] The movie thread's 00410390 path reaches 0053D7A0 by direct calls (00582160, 005802B0, 0057ED20, 004114E0, 0070D590,
//! 0070C120) at 027F, 53-bit; no original run recorded it and no port of that path exists yet.
//! Evidence: P15-RUN-001's raw dump of health_bar01.nif at the MainMenu holds two composed rotations the 24-bit form matches and the 53-bit form doesn't (test below), so the premise now has a run behind it, for the main thread.
//! TODO: [ ] The IO thread reaches this only through 0043961E's a7 arm (00707370, a host stop until a caller sets it): P15-RUN-001 shows the world transform identity on all 86 objects of the seven models the drain builds. 00430DE0 sets no control word and the thread comes from a plain CreateThread, as the movie thread's does, so that arm will need 53-bit forms unless a capture says otherwise.

/// IDA: NiTransform, with the operation order of its compose and invert routines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NiTransform {
    pub rotation: [f32; 9],
    pub translation: [f32; 3],
    pub scale: f32,
}

impl Default for NiTransform {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            translation: [0.0; 3],
            scale: 1.0,
        }
    }
}

/// IDA: 007101F0 / 007100A0's `fld; fmul; fld; fmul; faddp; fld; fmul; faddp; fstp`: `(a*b + c*d) + e*f`.
/// Precision: 24-bit, the main thread after CreateDevice (control word 007F, P04-RUN-002), so every product and sum
/// rounds to a float as it is computed, and the first two products are added first.
fn sum3(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> f32 {
    a * b + c * d + e * f
}

/// IDA: 0053D7A0 composes parent then local: scale, matrix entries in 007100A0's order, the rotated
/// translation in 007101F0's order, three scaled components, then x/y/z translation. Every entry's
/// grouping was compared with 007100A0's and 007101F0's instructions. Precision: 24-bit per step, see
/// `sum3`. Completion: 100% of the operation order.
#[must_use]
pub fn compose_scene_transform(parent: &NiTransform, local: &NiTransform) -> NiTransform {
    let a = &parent.rotation;
    let b = &local.rotation;
    let t = &local.translation;
    let scale = local.scale * parent.scale;
    let rotation = [
        sum3(b[0], a[0], b[3], a[1], b[6], a[2]),
        sum3(b[1], a[0], b[4], a[1], a[2], b[7]),
        sum3(b[2], a[0], b[5], a[1], a[2], b[8]),
        sum3(a[4], b[3], a[3], b[0], b[6], a[5]),
        sum3(b[4], a[4], b[1], a[3], b[7], a[5]),
        sum3(b[5], a[4], b[2], a[3], b[8], a[5]),
        sum3(a[7], b[3], a[6], b[0], b[6], a[8]),
        sum3(b[4], a[7], a[6], b[1], b[7], a[8]),
        sum3(b[5], a[7], a[6], b[2], b[8], a[8]),
    ];
    let rotated_x = sum3(a[1], t[1], t[0], a[0], a[2], t[2]);
    let rotated_y = sum3(a[3], t[0], a[4], t[1], a[5], t[2]);
    let rotated_z = sum3(a[6], t[0], a[7], t[1], a[8], t[2]);
    let scaled_x = rotated_x * parent.scale;
    let scaled_y = rotated_y * parent.scale;
    let scaled_z = parent.scale * rotated_z;
    NiTransform {
        rotation,
        translation: [
            scaled_x + parent.translation[0],
            parent.translation[1] + scaled_y,
            parent.translation[2] + scaled_z,
        ],
        scale,
    }
}

/// IDA: N/A 00718A80's guard as the C++ port applies it (not original behaviour): a nonfinite or zero scale is the caller's
/// error, otherwise `invert_scene_transform`. Completion: 100% of the host check.
/// TODO: [ ] Compare the original's zero-scale behaviour.
pub fn invert_scene_transform_checked(
    input: &NiTransform,
    error: oblivion_core::error::GameError,
) -> Result<NiTransform, oblivion_core::error::GameError> {
    if !input.scale.is_finite() || input.scale == 0.0 {
        return Err(error);
    }
    Ok(invert_scene_transform(input))
}

/// IDA: 00718A80 NiTransform_Invert: the transpose, the reciprocal scale, then the negated translation
/// rotated and scaled. No finite or zero-scale guard is added. Precision: 24-bit per step, see `sum3`.
/// Completion: 100% of the operation order.
/// TODO: [ ] Match x87 exceptions and NaN payloads.
#[must_use]
pub fn invert_scene_transform(input: &NiTransform) -> NiTransform {
    let source = &input.rotation;
    let rotation = [
        source[0], source[3], source[6], source[1], source[4], source[7], source[2], source[5],
        source[8],
    ];
    let scale = 1.0 / input.scale;
    let negative = input.translation.map(|component| -component);
    let rotated = [
        sum3(
            rotation[1],
            negative[1],
            negative[0],
            rotation[0],
            rotation[2],
            negative[2],
        ),
        sum3(
            rotation[3],
            negative[0],
            rotation[4],
            negative[1],
            rotation[5],
            negative[2],
        ),
        sum3(
            rotation[6],
            negative[0],
            rotation[7],
            negative[1],
            rotation[8],
            negative[2],
        ),
    ];
    NiTransform {
        rotation,
        translation: [scale * rotated[0], rotated[1] * scale, scale * rotated[2]],
        scale,
    }
}

/// IDA: 007101F0 a row-major 3x3 matrix times a vector: three stores, with the original operand grouping.
/// Precision: 24-bit per step, see `sum3`. Completion: 100% of the operation order.
#[must_use]
pub fn rotate_vector(matrix: &[f32; 9], vector: &[f32; 3]) -> [f32; 3] {
    [
        sum3(
            matrix[1], vector[1], vector[0], matrix[0], matrix[2], vector[2],
        ),
        sum3(
            matrix[3], vector[0], matrix[4], vector[1], matrix[5], vector[2],
        ),
        sum3(
            matrix[6], vector[0], matrix[7], vector[1], matrix[8], vector[2],
        ),
    ]
}

/// IDA: 0053D4B0 rotates a position (007101F0), stores three scaled components, then adds the translation.
/// Precision: 24-bit per step, see `sum3`. Completion: 100% of the operation order.
#[must_use]
pub fn transform_point(transform: &NiTransform, position: &[f32; 3]) -> [f32; 3] {
    let rotated = rotate_vector(&transform.rotation, position);
    let scaled = rotated.map(|component| component * transform.scale);
    [
        transform.translation[0] + scaled[0],
        transform.translation[1] + scaled[1],
        transform.translation[2] + scaled[2],
    ]
}

/// IDA: flt_B27520 (0x358637BD), 0043F350's zero threshold. The value 004BF9E0 and 006BBEE0 also test (path_interpolator.rs
/// and forms_world.rs cited it as 00A372CC / flt_A372CC, the f32 1e-6).
pub const NORMALIZE_ZERO_THRESHOLD: f32 = f32::from_bits(0x3586_37BD);

/// IDA: 0043F350 stores y*y + x*x + z*z and its sqrt as floats, zeroes the vector at <= 0x358637BD, or scales it
/// by one stored reciprocal. Returns the stored length, 0 when zeroed. Precision: 24-bit per step; `__CIsqrt` (00982C30)
/// keeps the precision bits (00991B15 masks 0x300), so its `fsqrt` rounds to a float too. The `fcom` branch at 0043F395
/// tests C0|C3, so an unordered NaN length takes the zero arm. Completion: 100% of the branch and stores.
pub fn normalize_vector(vector: &mut [f32; 3]) -> f32 {
    let squared = sum3(
        vector[1], vector[1], vector[0], vector[0], vector[2], vector[2],
    );
    let length = squared.sqrt();
    if length <= NORMALIZE_ZERO_THRESHOLD || length.is_nan() {
        *vector = [0.0; 3];
        return 0.0;
    }
    let reciprocal = 1.0 / length;
    vector[0] *= reciprocal;
    vector[1] *= reciprocal;
    vector[2] *= reciprocal;
    length
}

/// IDA: 0074A0C3 / 0074A0DE snapshots, then 0074A0E8 -> 00718A80 and 0074A0FE -> 0053D7A0: the inverse
/// target composed with the source world. Completion: 100% of the order.
#[must_use]
pub fn resolve_scene_relative_transform(
    source_world: &NiTransform,
    target_world: &NiTransform,
) -> NiTransform {
    compose_scene_transform(&invert_scene_transform(target_world), source_world)
}

/// IDA: 004BF9E0 NiVector3_UnitCross(this, result, rhs): this x rhs as (rhs.z*y - rhs.y*z, z*rhs.x - x*rhs.z,
/// rhs.y*x - rhs.x*y), each a float; the squared length (y + x + z order) and its sqrt as floats; at or below 1e-6 the
/// result is zero, else each component times one stored reciprocal. Precision: 24-bit per step. Completion: 100%.
#[must_use]
pub fn unit_cross(this: &[f32; 3], rhs: &[f32; 3]) -> [f32; 3] {
    let x = rhs[2] * this[1] - rhs[1] * this[2];
    let y = this[2] * rhs[0] - this[0] * rhs[2];
    let z = rhs[1] * this[0] - rhs[0] * this[1];
    let length = (y * y + x * x + z * z).sqrt();
    if length <= 0.000_001 {
        return [0.0; 3];
    }
    let reciprocal = 1.0 / length;
    [reciprocal * x, reciprocal * y, reciprocal * z]
}

/// IDA: 0070C340 NiCamera_LookAtWorldPoint(this, target, up), the rotation it stores in +0x30: self = world translate
/// (+0x88) - target, each a float; its squared length below 1e-10 (dbl_A7E548) answers 0; 0043F350 normalizes it;
/// right = 004BF9E0(up, self), below 0.5 (flt_A3D65C) squared answers 0; up' = 004BF9E0(self, right), the same test;
/// NiMatrix33_SetColumns (0070FCC0) with -self, up' and right as columns. A camera with a parent (+0x1C) takes 00710490's
/// product with the parent's world rotation (+0x64): not taken here, the only caller's camera has none. The store into
/// +0x30 and vt+0x74 (NiCamera_UpdateWorldData) and vt+0x78 (NiCamera_UpdateWorldBound) are the caller's. `None` is the
/// 0 answer, which leaves the camera unchanged. Precision: 24-bit per step. Completion: 100% without a parent.
#[must_use]
pub fn camera_look_at_rotation(
    world_translate: &[f32; 3],
    target: &[f32; 3],
    up: &[f32; 3],
) -> Option<[f32; 9]> {
    let squared = |v: &[f32; 3]| v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    let mut direction = [
        world_translate[0] - target[0],
        world_translate[1] - target[1],
        world_translate[2] - target[2],
    ];
    if f64::from(squared(&direction)) < 1.000_000_013_351_432e-10 {
        return None;
    }
    normalize_vector(&mut direction);
    let right = unit_cross(up, &direction);
    if squared(&right) < 0.5 {
        return None;
    }
    let column1 = unit_cross(&direction, &right);
    if squared(&column1) < 0.5 {
        return None;
    }
    Some([
        -direction[0],
        column1[0],
        right[0],
        -direction[1],
        column1[1],
        right[1],
        -direction[2],
        column1[2],
        right[2],
    ])
}

// IDA: 007070B0 the engine's arctangent(a, b): both 0 answers 0; otherwise a polynomial in the smaller over the larger, every step stored as a float, with 00B3F99C pi / 2 and 00B3F9A4 pi as the static constructors leave them (snapshot-phase17_end). A division that comes out 0 answers without it. The constants are doubles, so each product is a double's rounded once. `ni_atan2` isn't `atan2`: its last bits differ, and 00711300 goes through it. Completion: 100%.
#[must_use]
pub fn ni_atan2(a: f32, b: f32) -> f32 {
    const HALF_PI: f32 = f32::from_bits(0x3FC9_0FDB);
    const PI: f32 = f32::from_bits(0x4049_0FDB);
    let step = |value: f64| value as f32;
    if b == 0.0 && a == 0.0 {
        return 0.0;
    }
    // 007070FA `test ah, 5; jp`: unordered goes the second way.
    let (t, offset) = if b.abs() < a.abs() {
        let t = b / a;
        // 00707113 `test ah, 41h`, 007071B7 `test ah, 5`.
        if t > 0.0 {
            (t, HALF_PI)
        } else if t < 0.0 {
            (t, -HALF_PI)
        } else {
            return if 0.0 < a { HALF_PI } else { -HALF_PI };
        }
    } else {
        let t = a / b;
        // 007071FF `test ah, 44h; jp`: anything but 0, a NaN too.
        if t == 0.0 {
            // 00707209 `fcomp`, `test ah, 41h; jz`: only b above 0 keeps the 0.
            return if b > 0.0 { 0.0 } else { PI };
        }
        (t, 0.0)
    };
    let square = t * t;
    let mut value = step(0.020_835_099_741_816_52 * f64::from(square));
    value = step(f64::from(value) - 0.085_133_001_208_305_36);
    value *= square;
    value = step(f64::from(value) + 0.180_141_001_939_773_56);
    value *= square;
    value = step(f64::from(value) - 0.330_299_496_650_695_8);
    value *= square;
    value = step(f64::from(value) + 0.999_866_008_758_544_9);
    value *= t;
    // 007071A7 `jnp`: an offset of 0 leaves the polynomial as it is.
    if offset != 0.0 {
        value = offset - value;
    }
    if a < 0.0 && b < 0.0 {
        value - PI
    } else if a > 0.0 && b < 0.0 {
        PI + value
    } else {
        value
    }
}

// IDA: 00711300 NiMatrix33_ToEulerAnglesXYZ(x, y, z), `__CIasin` at full precision (D6). The arms' `test ah, 41h` take a NaN to the -pi / 2 store. Its bool return has no caller here. Completion: 100%.
#[must_use]
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "00711317, 00711325, 00711372 `test ah, 41h`: an unordered compare takes the arm"
)]
pub fn matrix_to_euler_xyz(matrix: &[f32; 9]) -> [f32; 3] {
    const HALF_PI: f32 = f32::from_bits(0x3FC9_0FDB);
    let sine = matrix[2];
    let asin = if !(f64::from(sine) > -1.0) {
        -HALF_PI
    } else if !(1.0 > sine) {
        HALF_PI
    } else {
        f64::from(sine).asin() as f32
    };
    let y = -asin;
    if !(HALF_PI > y) {
        return [-ni_atan2(matrix[3], matrix[4]), y, 0.0];
    }
    if -HALF_PI < y {
        [
            -ni_atan2(-matrix[5], matrix[8]),
            y,
            -ni_atan2(-matrix[1], matrix[0]),
        ]
    } else {
        [ni_atan2(matrix[3], matrix[4]), y, 0.0]
    }
}

// IDA: 00711440 NiMatrix33_ToEulerAnglesZXY(out first, out middle, out last) (thiscall matrix): middle = -`__CIasin`(m[7])
// (m[7] at or below -1: -pi / 2 (00B3F99C); at or above 1: pi / 2), stored as floats; at or past +pi / 2 the first is
// 0.0 - NiFastATan2(m[2], m[0]) and the last 0.0; at or below -pi / 2 the first NiFastATan2(m[2], m[0]) - 0.0 and the last
// 0.0; else the first -NiFastATan2(-m[1], m[4]) and the last -NiFastATan2(-m[6], m[8]). Completion: 100%.
#[must_use]
pub fn matrix_to_angles_zxy(matrix: &[f32; 9]) -> [f32; 3] {
    const HALF_PI: f32 = f32::from_bits(0x3FC9_0FDB);
    let sine = matrix[7];
    let asin = if sine <= -1.0 {
        -HALF_PI
    } else if sine >= 1.0 {
        HALF_PI
    } else {
        f64::from(sine).asin() as f32
    };
    let middle = -asin;
    if HALF_PI <= middle {
        let first = 0.0 - ni_atan2(matrix[2], matrix[0]);
        [first, middle, 0.0]
    } else if -HALF_PI >= middle {
        let first = ni_atan2(matrix[2], matrix[0]) - 0.0;
        [first, middle, 0.0]
    } else {
        [
            -ni_atan2(-matrix[1], matrix[4]),
            middle,
            -ni_atan2(-matrix[6], matrix[8]),
        ]
    }
}

// IDA: 007102B0 NiMatrix33_Inverse(out) (thiscall matrix) and 007103C0, which answers zeros for a failed inverse: the
// cofactors (each a x b - c x d, floats), the determinant (out0 x m0 + m1 x out3) + out6 x m2; at or below 1e-6 in size
// false; else each entry times the float 1.0 / det. Precision: 24-bit. Completion: 100%.
#[must_use]
pub fn invert_rotation_or_zero(m: &[f32; 9]) -> [f32; 9] {
    let mut out = [
        m[8] * m[4] - m[7] * m[5],
        m[7] * m[2] - m[1] * m[8],
        m[1] * m[5] - m[2] * m[4],
        m[6] * m[5] - m[3] * m[8],
        m[0] * m[8] - m[6] * m[2],
        m[3] * m[2] - m[5] * m[0],
        m[3] * m[7] - m[6] * m[4],
        m[1] * m[6] - m[7] * m[0],
        m[0] * m[4] - m[3] * m[1],
    ];
    let det = out[0] * m[0] + m[1] * out[3] + out[6] * m[2];
    if det.abs() <= 0.000_001 {
        return [0.0; 9];
    }
    let inverse = 1.0 / det;
    for value in &mut out {
        *value *= inverse;
    }
    out
}

// IDA: 007100A0 NiMatrix33_Multiply out = this * rhs, each entry f32((a_r0*b_0c + a_r1*b_1c) + a_r2*b_2c)
// (particle_spawn's former copy stored the entries column by column and wrote some first pairs as (a_r1*b_1c + a_r0*b_0c):
// the same pair summed first, so the same bits but for which NaN payload a sum of two NaNs keeps)
// Completion: 100% of the verified operands
// Precision: 24-bit per step, see `sum3`.
#[must_use]
pub fn multiply(a: &[f32; 9], b: &[f32; 9]) -> [f32; 9] {
    let mut value = [0.0; 9];
    for row in 0..3 {
        for column in 0..3 {
            value[row * 3 + column] = (a[row * 3] * b[column] + a[row * 3 + 1] * b[3 + column])
                + a[row * 3 + 2] * b[6 + column];
        }
    }
    value
}

// IDA: 0070FD30 NiMatrix33_MakeXRotation(angle): 1, 0, 0, 0, cos, sin, 0, -sin, cos, with `fchs` on the float sine.
pub fn rotation_x(angle: f32) -> [f32; 9] {
    let (sin, cos) = f64::from(angle).sin_cos();
    let (sin, cos) = (sin as f32, cos as f32);
    [1.0, 0.0, 0.0, 0.0, cos, sin, 0.0, -sin, cos]
}

// IDA: 00711580 NiMatrix33_FromEulerAnglesXYZ(x, y, z): X * (Y * Z), each `cos` and `sin` at full precision and stored as a float (D6), each 007100A0 per step at 24 bits. Completion: 100%.
#[must_use]
pub fn matrix_from_euler_xyz(x: f32, y: f32, z: f32) -> [f32; 9] {
    let turn = |angle: f32| (f64::from(angle).cos() as f32, f64::from(angle).sin() as f32);
    let (cos_x, sin_x) = turn(x);
    let (cos_y, sin_y) = turn(y);
    let (cos_z, sin_z) = turn(z);
    let about_x = [1.0, 0.0, 0.0, 0.0, cos_x, sin_x, 0.0, -sin_x, cos_x];
    let about_y = [cos_y, 0.0, -sin_y, 0.0, 1.0, 0.0, sin_y, 0.0, cos_y];
    let about_z = [cos_z, sin_z, 0.0, -sin_z, cos_z, 0.0, 0.0, 0.0, 1.0];
    multiply(&about_x, &multiply(&about_y, &about_z))
}

// IDA: 0070FD80 NiMatrix33_MakeYRotation(angle), as `face_gen_nodes::place_under_head_node` builds it: each `cos`/`sin` at
// full precision, stored as a float.
pub(crate) fn y_rotation(angle: f32) -> [f32; 9] {
    let angle = f64::from(angle);
    let (c, s) = (angle.cos() as f32, angle.sin() as f32);
    [c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c]
}

// IDA: 0070FDD0 NiMatrix33_MakeZRotation(angle): fsincos, each stored as a float, then cos, sin, 0, -sin, cos, 0, 0, 0, 1.
pub(crate) fn rotation_z(angle: f32) -> [f32; 9] {
    let (sin, cos) = f64::from(angle).sin_cos();
    let (sin, cos) = (sin as f32, cos as f32);
    [cos, sin, 0.0, -sin, cos, 0.0, 0.0, 0.0, 1.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(values: &[f32]) -> Vec<u32> {
        values.iter().map(|value| value.to_bits()).collect()
    }

    fn from_bits(words: [u32; 13]) -> NiTransform {
        let float = |index: usize| f32::from_bits(words[index]);
        NiTransform {
            rotation: std::array::from_fn(float),
            translation: [float(9), float(10), float(11)],
            scale: float(12),
        }
    }

    fn to_words(transform: &NiTransform) -> Vec<u32> {
        let mut words = bits(&transform.rotation);
        words.extend(bits(&transform.translation));
        words.push(transform.scale.to_bits());
        words
    }

    // IDA: N/A the raw words of a P15-RUN-001 `NO` record: +0x30 the local transform, +0x64 the world.
    fn recorded_transforms(number: u32) -> ([u32; 13], [u32; 13]) {
        let path =
            oblivion_core::fixtures::root().join("evidence/phase15-run001-2026-10-10.records.txt");
        let bytes = std::fs::read(path).unwrap();
        let text: String = bytes.iter().map(|&byte| char::from(byte)).collect();
        let prefix = format!("NO {number} ");
        let line = text
            .lines()
            .find(|line| line.starts_with(&prefix))
            .unwrap_or_else(|| panic!("no record {number}"));
        let raw = line
            .split(' ')
            .find_map(|part| part.strip_prefix("raw="))
            .unwrap();
        let word = |at: usize| {
            let start = at * 2;
            let byte = |index: usize| {
                u32::from(
                    u8::from_str_radix(&raw[start + index * 2..start + index * 2 + 2], 16).unwrap(),
                )
            };
            byte(0) | byte(1) << 8 | byte(2) << 16 | byte(3) << 24
        };
        (
            std::array::from_fn(|index| word(0x30 + index * 4)),
            std::array::from_fn(|index| word(0x64 + index * 4)),
        )
    }

    // Every object of P15-RUN-001's 300 `NO` dumps with an updated world (one that isn't identity, or whose parent's isn't): the parent's world composed with its local at 24 bits against the recorded +0x64, and how many elements the 53-bit form misses. Only `Health_Bar01.NIF` was updated by the MainMenu, so this is all the discrimination the run holds.
    #[test]
    fn every_updated_world_in_the_run_is_24_bit() {
        let path =
            oblivion_core::fixtures::root().join("evidence/phase15-run001-2026-10-10.records.txt");
        let bytes = std::fs::read(path).unwrap();
        let text: String = bytes.iter().map(|&byte| char::from(byte)).collect();
        let mut objects: Vec<(String, String, [u32; 13], [u32; 13])> = Vec::new();
        for line in text.lines().filter(|line| line.starts_with("NO ")) {
            let field = |name: &str| {
                line.split(' ')
                    .find_map(|part| part.strip_prefix(name))
                    .unwrap()
            };
            let raw = field("raw=");
            let word = |at: usize| {
                let byte = |index: usize| {
                    u32::from(
                        u8::from_str_radix(&raw[at * 2 + index * 2..at * 2 + index * 2 + 2], 16)
                            .unwrap(),
                    )
                };
                byte(0) | byte(1) << 8 | byte(2) << 16 | byte(3) << 24
            };
            objects.push((
                field("key=").to_owned(),
                field("path=").to_owned(),
                std::array::from_fn(|index| word(0x30 + index * 4)),
                std::array::from_fn(|index| word(0x64 + index * 4)),
            ));
        }
        const IDENTITY: [u32; 9] = [0x3F80_0000, 0, 0, 0, 0x3F80_0000, 0, 0, 0, 0x3F80_0000];
        let (mut compared, mut told_apart, mut missed) = (0, 0, 0);
        for (key, path, local, world) in &objects {
            let Some((parent_path, _)) = path.rsplit_once('.') else {
                continue;
            };
            let parent = objects
                .iter()
                .find(|object| object.0 == *key && object.1 == parent_path)
                .unwrap();
            if world[..9] == IDENTITY && parent.3[..9] == IDENTITY {
                continue;
            }
            let parent = from_bits(parent.3);
            let local = from_bits(*local);
            let composed = compose_scene_transform(&parent, &local);
            assert_eq!(to_words(&composed), world, "{key} {path}");
            compared += 1;
            let (a, b) = (parent.rotation, local.rotation);
            let wide = |x: [f32; 6]| {
                let x = x.map(f64::from);
                (x[0] * x[1] + x[2] * x[3] + x[4] * x[5]) as f32
            };
            let rotation_53 = [
                wide([b[0], a[0], b[3], a[1], b[6], a[2]]),
                wide([b[1], a[0], b[4], a[1], a[2], b[7]]),
                wide([b[2], a[0], b[5], a[1], a[2], b[8]]),
                wide([a[4], b[3], a[3], b[0], b[6], a[5]]),
                wide([b[4], a[4], b[1], a[3], b[7], a[5]]),
                wide([b[5], a[4], b[2], a[3], b[8], a[5]]),
                wide([a[7], b[3], a[6], b[0], b[6], a[8]]),
                wide([b[4], a[7], a[6], b[1], b[7], a[8]]),
                wide([b[5], a[7], a[6], b[2], b[8], a[8]]),
            ];
            let misses = bits(&rotation_53)
                .iter()
                .zip(world)
                .filter(|(got, want)| got != want)
                .count();
            missed += misses;
            if misses != 0 {
                told_apart += 1;
            }
        }
        println!(
            "{compared} updated objects: 24-bit matches all; {told_apart} tell it from 53-bit, by {missed} elements"
        );
        assert_eq!((compared, told_apart, missed), (4, 2, 4));
    }

    // The first run-backed check of the 24-bit premise: P15-RUN-001 records 233, 235, 236, Meshes\Menus\Enemy_Health_Bar\Health_Bar01.NIF at the MainMenu: r.0's world and the two lights r.0.1, r.0.2.
    #[test]
    fn a_recorded_composition_is_24_bit() {
        let (_, parent_world) = recorded_transforms(233);
        let lights = [recorded_transforms(235), recorded_transforms(236)];
        let parent = from_bits(parent_world);
        for (local, world) in lights {
            let local = from_bits(local);
            let composed = compose_scene_transform(&parent, &local);
            assert_eq!(to_words(&composed), world);
            // The same sums at 53 bits: f64 throughout, one rounding at the store.
            let (a, b) = (parent.rotation, local.rotation);
            let wide = |x: [f32; 6]| {
                let x = x.map(f64::from);
                (x[0] * x[1] + x[2] * x[3] + x[4] * x[5]) as f32
            };
            let rotation_53 = [
                wide([b[0], a[0], b[3], a[1], b[6], a[2]]),
                wide([b[1], a[0], b[4], a[1], a[2], b[7]]),
                wide([b[2], a[0], b[5], a[1], a[2], b[8]]),
                wide([a[4], b[3], a[3], b[0], b[6], a[5]]),
                wide([b[4], a[4], b[1], a[3], b[7], a[5]]),
                wide([b[5], a[4], b[2], a[3], b[8], a[5]]),
                wide([a[7], b[3], a[6], b[0], b[6], a[8]]),
                wide([b[4], a[7], a[6], b[1], b[7], a[8]]),
                wide([b[5], a[7], a[6], b[2], b[8], a[8]]),
            ];
            let misses = bits(&rotation_53)
                .iter()
                .zip(&world)
                .filter(|(got, want)| got != want)
                .count();
            assert_eq!(misses, 2, "this row tells the two apart");
        }
    }

    /// 007101F0 at 24-bit: row 0 is `(m1*v1 + v0*m0) + m2*v2`, each step rounded. The expected bits were derived with exact
    /// rational arithmetic, rounding each step to 24 bits. One rounding over the exact sum (the old `f64` form) gives 0x43A9D069.
    #[test]
    fn rotate_vector_rounds_each_step() {
        let matrix = [
            f32::from_bits(0xBEB4_3958),
            f32::from_bits(0xBF32_B021),
            f32::from_bits(0x3E9A_9FBE),
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ];
        let vector = [
            f32::from_bits(0xC4D5_C99A),
            f32::from_bits(0x430F_8000),
            f32::from_bits(0xC406_4CCD),
        ];
        assert_eq!(rotate_vector(&matrix, &vector)[0].to_bits(), 0x43A9_D06A);
    }

    /// 0043F350 at 24-bit: `(y*y + x*x) + z*z` is 0x44BBF240 per step (0x44BBF23F in one rounding), then the stored length and
    /// reciprocal. Expected bits from exact per-step rounding.
    #[test]
    fn normalize_vector_rounds_each_step() {
        let mut vector = [
            f32::from_bits(0x41A6_3D71),
            f32::from_bits(0xC193_C28F),
            f32::from_bits(0xC1D8_3D71),
        ];
        let length = normalize_vector(&mut vector);
        assert_eq!(length.to_bits(), 0x421B_1A85);
        assert_eq!(bits(&vector), [0x3F09_30BD, 0xBEF3_E11B, 0xBF32_7400]);
    }

    /// 0043F395 tests C0|C3 after `fcom`, and an unordered compare sets both: a NaN length takes the zero arm.
    #[test]
    fn a_nan_length_zeroes_the_vector() {
        let mut vector = [f32::NAN, 1.0, 1.0];
        assert_eq!(normalize_vector(&mut vector), 0.0);
        assert_eq!(vector, [0.0; 3]);
    }
}
