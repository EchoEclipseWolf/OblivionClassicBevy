//! NiProcessorSpecificCode (the singleton tail 00731253 -> 00717440) and the NiDX9 texture-coordinate generator slots
//! (the shader tail 0077BF35 -> 0077A070). Both run once, inside their callbacks' one-time guards (00B3FFB0 and
//! 00B4288C), and neither cleanup resets them. The C++ port's process globals are resources here.

use bevy_ecs::prelude::*;

const EXTENDED_LEAF: u32 = 0x8000_0000;
const MMX: u32 = 0x0080_0000;
const CMOV: u32 = 0x0000_8000;
const SSE: u32 = 0x0200_0000;

/// IDA: CPUID(leaf) as {eax, ebx, ecx, edx}. The host default uses `__cpuid`; tests inject fixed values.
pub type CpuidQuery = fn(u32) -> [u32; 4];

/// IDA: N/A the host's CPUID. Host substitution: another architecture has no CPUID, and reads as all zero, which takes
/// class 1 and the prescaled routines.
#[must_use]
pub fn host_cpuid(leaf: u32) -> [u32; 4] {
    #[cfg(target_arch = "x86_64")]
    {
        let registers = std::arch::x86_64::__cpuid(leaf);
        [registers.eax, registers.ebx, registers.ecx, registers.edx]
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = leaf;
        [0; 4]
    }
}

/// IDA: 00B3FCCC detected byte, 00B3FCD0 last CPUID value, 00B3FCC8 class.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NiProcessorDetection {
    pub detected: bool,
    pub cpuid_value: u32,
    pub processor_class: i32,
}

/// IDA: 007170D0 NiProcessor_Detect: sets 00B3FCCC; CPUID(0x80000000) -> 00B3FCD0. With bit 31 set it executes CPUID
/// again but loads 0x80000001 into EDX, not EAX (0071715F), so the leaf is still 0x80000000; bit 31 of that result
/// gives class 6 (every CPU with extended leaves). Otherwise CPUID(1) EDX (007171B2): MMX (0x800000) + CMOV (0x8000) ->
/// 4, plus SSE (0x2000000) -> 5; MMX only -> 2; CMOV only -> 3; neither -> 1. A faulting CPUID (SEH) stores class 0.
/// Completion: 100%; the host cannot fault, so class 0 is unreachable.
pub fn ni_detect_processor(state: &mut NiProcessorDetection, query: CpuidQuery) {
    state.detected = true;
    state.cpuid_value = query(EXTENDED_LEAF)[0];
    if state.cpuid_value & EXTENDED_LEAF != 0 {
        state.cpuid_value = query(state.cpuid_value & EXTENDED_LEAF)[0];
        if state.cpuid_value & EXTENDED_LEAF != 0 {
            state.processor_class = 6;
            return;
        }
    }
    state.cpuid_value = query(1)[3];
    if state.cpuid_value & MMX == 0 {
        state.processor_class = if state.cpuid_value & CMOV != 0 { 3 } else { 1 };
        return;
    }
    if state.cpuid_value & CMOV == 0 {
        state.processor_class = 2;
        return;
    }
    state.processor_class = 4;
    if state.cpuid_value & SSE != 0 {
        state.processor_class = 5;
    }
}

/// IDA: 00B27168 TransformPoints slot (image value 00717280); read only by 0048032A (004802E0) and 0096DD84 (0096DD40),
/// later-phase geometry code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum NiTransformPointsRoutine {
    #[default]
    Generic = 0x0071_7280,
    Prescaled = 0x0071_6FC0,
}

/// IDA: 00B27164 TransformVectors slot (image value 00717370). Its only reference in the executable is the 00716F25
/// store, so it is never read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum NiTransformVectorsRoutine {
    #[default]
    Generic = 0x0071_7370,
    Unrolled = 0x0071_6F40,
}

/// IDA: the process globals above.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NiProcessorSpecificState {
    pub detection: NiProcessorDetection,
    pub transform_points: NiTransformPointsRoutine,
    pub transform_vectors: NiTransformVectorsRoutine,
}

/// IDA: 00717440 NiProcessorSpecificCode_Initialize: 007170D0 unless 00B3FCCC is set; installs 00717280/00717370
/// (00716F30/00716F20), then 00716FC0/00716F40 when the signed class is in (0, 6] (0071746E). Completion: 100%.
pub fn ni_processor_specific_initialize(state: &mut NiProcessorSpecificState, query: CpuidQuery) {
    if !state.detection.detected {
        ni_detect_processor(&mut state.detection, query);
    }
    state.transform_points = NiTransformPointsRoutine::Generic;
    state.transform_vectors = NiTransformVectorsRoutine::Generic;
    if state.detection.processor_class > 0 && state.detection.processor_class <= 6 {
        state.transform_points = NiTransformPointsRoutine::Prescaled;
        state.transform_vectors = NiTransformVectorsRoutine::Unrolled;
    }
}

/// IDA: NiTransform as the routines read it: rotation 3x3 row-major at +0x00, translation at +0x24, scale at +0x30.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NiTransformValue {
    pub rotate: [f32; 9],
    pub translate: [f32; 3],
    pub scale: f32,
}

impl Default for NiTransformValue {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            rotate: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            translate: [0.0; 3],
            scale: 1.0,
        }
    }
}

/// IDA: 00717280 TransformPoints (generic): for each of `count` (u16) points, out = ((R0*x + y*R1) + z*R2) * s + t per
/// row. Completion: 100%; the x87 intermediates are host float operations (24-bit precision control).
pub fn ni_transform_points_generic(
    count: u16,
    points: &[f32],
    out: &mut [f32],
    transform: &NiTransformValue,
) {
    let r = &transform.rotate;
    for index in 0..usize::from(count) {
        let (x, y, z) = (
            points[index * 3],
            points[index * 3 + 1],
            points[index * 3 + 2],
        );
        out[index * 3] =
            (r[0] * x + y * r[1] + z * r[2]) * transform.scale + transform.translate[0];
        out[index * 3 + 1] =
            (r[3] * x + y * r[4] + z * r[5]) * transform.scale + transform.translate[1];
        out[index * 3 + 2] =
            (r[6] * x + y * r[7] + z * r[8]) * transform.scale + transform.translate[2];
    }
}

/// IDA: 00716FC0 TransformPoints (prescaled): m = float(R * s) per element, then out = ((x*m0 + y*m1) + z*m2) + t per
/// row, and the third row multiplies m first. Completion: 100%; float intermediates.
pub fn ni_transform_points_prescaled(
    count: u16,
    points: &[f32],
    out: &mut [f32],
    transform: &NiTransformValue,
) {
    let m = transform.rotate.map(|element| element * transform.scale);
    for index in 0..usize::from(count) {
        let (x, y, z) = (
            points[index * 3],
            points[index * 3 + 1],
            points[index * 3 + 2],
        );
        out[index * 3] = x * m[0] + y * m[1] + z * m[2] + transform.translate[0];
        out[index * 3 + 1] = x * m[3] + y * m[4] + z * m[5] + transform.translate[1];
        out[index * 3 + 2] = m[6] * x + m[7] * y + m[8] * z + transform.translate[2];
    }
}

/// IDA: the 00B27168 indirect call (0048032A/0096DD84). Completion: 100%.
pub fn ni_transform_points(
    state: &NiProcessorSpecificState,
    count: u16,
    points: &[f32],
    out: &mut [f32],
    transform: &NiTransformValue,
) {
    if state.transform_points == NiTransformPointsRoutine::Prescaled {
        ni_transform_points_prescaled(count, points, out, transform);
    } else {
        ni_transform_points_generic(count, points, out, transform);
    }
}

/// IDA: the five routines 0077A070 stores. They set D3DTSS_TEXCOORDINDEX (camera-space normal 0x10000, position
/// 0x20000, reflection vector 0x30000) and the texture transform flags.
pub const NI_DX9_TEXGEN_ROUTINES: [u32; 5] = [
    0x0077_97D0,
    0x0077_99C0,
    0x0077_9BC0,
    0x0077_9D50,
    0x0077_9EE0,
];

/// IDA: 00B42870..00B42880, image value 0xFFFFFFFF each. The only references to the slots are 0077A070's stores, so no
/// code reads them; these are identities as data. Completion: 100%.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct NiDx9TexGenSlots(pub [u32; 5]);

impl Default for NiDx9TexGenSlots {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self([0xFFFF_FFFF; 5])
    }
}

impl NiDx9TexGenSlots {
    /// IDA: 0077A070 NiDX9TextureStage_InitializeTexGenFunctions. Completion: 100%.
    pub const fn install_routines(&mut self) {
        self.0 = NI_DX9_TEXGEN_ROUTINES;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_cpuid_modern(leaf: u32) -> [u32; 4] {
        [
            if leaf == 0x8000_0000 { 0x8000_0008 } else { 0 },
            0,
            0,
            0x0383_FBFF,
        ]
    }

    fn fake_cpuid_sse(leaf: u32) -> [u32; 4] {
        [
            if leaf == 0x8000_0000 { 0x0000_0002 } else { 0 },
            0,
            0,
            0x0280_8000,
        ]
    }

    fn fake_cpuid_cmov(leaf: u32) -> [u32; 4] {
        [
            if leaf == 0x8000_0000 { 0x0000_0002 } else { 0 },
            0,
            0,
            0x0000_8000,
        ]
    }

    /// `verify_processor_specific` in the C++ `startup_scaffold`.
    #[test]
    fn detection_slots_and_transform_points() {
        let mut modern = NiProcessorSpecificState::default();
        ni_processor_specific_initialize(&mut modern, fake_cpuid_modern);
        assert!(modern.detection.detected);
        assert_eq!(modern.detection.processor_class, 6);
        assert_eq!(modern.detection.cpuid_value, 0x8000_0008);
        assert_eq!(modern.transform_points, NiTransformPointsRoutine::Prescaled);
        assert_eq!(
            modern.transform_vectors,
            NiTransformVectorsRoutine::Unrolled
        );

        let mut sse = NiProcessorSpecificState::default();
        ni_processor_specific_initialize(&mut sse, fake_cpuid_sse);
        let mut cmov = NiProcessorSpecificState::default();
        ni_processor_specific_initialize(&mut cmov, fake_cpuid_cmov);
        assert_eq!(sse.detection.processor_class, 5);
        assert_eq!(sse.detection.cpuid_value, 0x0280_8000);
        assert_eq!(cmov.detection.processor_class, 3);

        let mut exception_class = NiProcessorSpecificState::default();
        exception_class.detection.detected = true;
        ni_processor_specific_initialize(&mut exception_class, fake_cpuid_modern);
        assert_eq!(exception_class.detection.processor_class, 0);
        assert_eq!(
            exception_class.transform_points,
            NiTransformPointsRoutine::Generic
        );

        let mut transform = NiTransformValue {
            rotate: [0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
            translate: [10.0, 20.0, 30.0],
            scale: 2.0,
        };
        let points = [1.0, 2.0, 3.0, -4.0, 0.5, 0.25];
        let mut generic = [0.0; 6];
        let mut prescaled = [0.0; 6];
        ni_transform_points_generic(2, &points, &mut generic, &transform);
        ni_transform_points(&modern, 2, &points, &mut prescaled, &transform);
        assert_eq!(generic, [6.0, 22.0, 36.0, 9.0, 12.0, 30.5]);
        assert_eq!(prescaled, generic);

        // (R*v)*s and (R*s)*v may round differently for inexact scales.
        transform.rotate = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];
        transform.scale = 1.1;
        let point: [f32; 3] = [0.3, 0.7, 1.9];
        let mut a = [0.0; 3];
        let mut b = [0.0; 3];
        ni_transform_points_generic(1, &point, &mut a, &transform);
        ni_transform_points_prescaled(1, &point, &mut b, &transform);
        let r = &transform.rotate;
        let expected_generic = (r[0] * point[0] + point[1] * r[1] + point[2] * r[2])
            * transform.scale
            + transform.translate[0];
        let (m0, m1, m2) = (
            r[0] * transform.scale,
            r[1] * transform.scale,
            r[2] * transform.scale,
        );
        let expected_prescaled =
            point[0] * m0 + point[1] * m1 + point[2] * m2 + transform.translate[0];
        assert_eq!(a[0], expected_generic);
        assert_eq!(b[0], expected_prescaled);

        let mut slots = NiDx9TexGenSlots::default();
        assert_eq!(slots.0[0], 0xFFFF_FFFF);
        slots.install_routines();
        assert_eq!(slots.0, NI_DX9_TEXGEN_ROUTINES);
    }
}
