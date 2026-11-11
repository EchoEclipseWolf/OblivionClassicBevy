//! Phase 04 row 04-G: WinMain 0040E893 FaceGen_EnsureInitialized 00553550 -> FaceGen_Constructor 00553140 (FormHeap
//! 0xDBC, singleton 00B39B80). The CTL reader is the FaceGen SDK FutBinaryFileC/BSFaceGenBinaryFile
//! (006F5ED0/006F6110/006F66E0/006F5D40) and FanControls.cpp (006EF020, 006EE270). All FanControls arithmetic runs after
//! CreateDevice, at the x87 24-bit precision Direct3D selects without FPU_PRESERVE (P04-RUN-002: control word 007F), so
//! every operation is plain `f32`.

pub mod face_gen_head;
pub mod face_gen_nodes;
pub mod face_gen_tasks;

use crate::gamebryo::renderer::{
    RendererCreation, create_renderer_source_texture, original_rgb24_pixel_format,
};
use bevy_ecs::prelude::*;
use oblivion_core::logging::{Level, emit};

use crate::bs::archive::ArchiveManager;
use crate::bs::file::OpenedFile;
use crate::bs::file_finder::{FileFinder, bs_file_open_core};

/// IDA: FaceGen FgMatrix (0x18): +0 rows, +4 cols, +8 std::vector<float>.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceGenMatrix {
    pub rows: u32,
    pub cols: u32,
    pub data: Vec<f32>,
}

/// IDA: 006EF020 linear control element (52 bytes): FgMatrix {1, count, coefficients} + std::string name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceGenLinearControl {
    pub coefficients: FaceGenMatrix,
    pub name: Vec<u8>,
}

/// IDA: 006EE270 phase 3 group block (0x78): five FgMatrix records in read order: c0 x 1, c2 x 1, (c0 + c2) square, c0
/// square, c2 square.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceGenGroupStatistics {
    pub matrices: [FaceGenMatrix; 5],
}

/// IDA: 006EE270 phase 1 element (0x20): FgMatrix {1, c[2t]}, +0x18 1 / sum of squares, +0x1C the float read after the
/// coefficients.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceGenGroupControl {
    pub coefficients: FaceGenMatrix,
    pub inverse_norm_squared: f32,
    pub value: f32,
}

/// IDA: 006EE270 phase 2 element (0x38): two FgMatrix {1, c0}/{1, c2}, +0x30 1 / (sum of squares of both), +0x34 the float
/// read after them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceGenPairControl {
    pub coefficients: [FaceGenMatrix; 2],
    pub inverse_norm_squared: f32,
    pub value: f32,
}

/// IDA: 006EE270 phase 4 Gram-Schmidt pair (0x30): the first control normalized and the second made orthonormal to it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceGenOrthonormalPair {
    pub first: FaceGenMatrix,
    pub second: FaceGenMatrix,
}

/// IDA: FaceGen +0xC8 FanControls block (0xCD4 bytes, constructor 00552D90).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceGenFanControls {
    /// +0x000.
    pub loaded: bool,
    /// +0x004.
    pub groups: [FaceGenGroupStatistics; 5],
    /// +0x25C [group][k][basis] with basis 0 = geometry (c0), 1 = texture (c2).
    pub group_controls: [[[FaceGenGroupControl; 2]; 2]; 5],
    /// +0x4DC (i != j read).
    pub pair_controls: [[FaceGenPairControl; 5]; 5],
    /// +0xA54 [group][basis].
    pub orthonormal: [[FaceGenOrthonormalPair; 2]; 5],
    /// +0xC34: 2x2 inverses of the Gram matrices {m_a . m_b} [group][basis].
    pub inverse_gram: [[[f32; 4]; 2]; 5],
}

/// IDA: 00553323..005533C6 NiAlphaProperty: constructed with flags 0xEC and test reference 0, then flags |= 0x200, flags =
/// flags & 0xE3FF | 0x1000, reference = 100 (alpha test GREATER 100, blend SRCALPHA/INVSRCALPHA off).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FaceGenAlphaProperty {
    pub flags: u16,
    pub test_reference: u8,
}

/// IDA: 00761270 NiSourceTexture_CreateFromPixelData's result, in the renderer's terms.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FaceGenTextureUpload {
    /// The chosen renderer +0x6F4 entry.
    pub table_case: Option<usize>,
    /// D3DFORMAT.
    pub format: u32,
    /// The converted level 0.
    pub texels: Vec<u8>,
    /// The device texture exists (a trace entry in the renderer's seam).
    pub texture: bool,
}

/// One grey texture of FaceGen +0xDB4 / +0xDB8.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FaceGenTexture {
    /// The byte written to every R, G, B.
    pub grey: u8,
    /// The 32x32 NiPixelData (00B25E48); empty after upload.
    pub rgb24: Vec<u8>,
    /// 00761270 result.
    pub upload: FaceGenTextureUpload,
}

/// IDA: the FaceGen object (0xDBC), 00B39B80.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceGenState {
    /// +0x000/+0x004.
    pub header_words: [u32; 2],
    /// +0x008 (zero column vectors).
    pub coefficients: [FaceGenMatrix; 4],
    /// +0x088.
    pub linear_controls: [Vec<FaceGenLinearControl>; 4],
    /// +0x0C8.
    pub fan_controls: FaceGenFanControls,
    /// +0xD9C.
    pub counts: [u32; 4],
    /// +0xDAC the BSFaceGenModelMap (00551E80, made by 00553620 on first use; 0054FE82 purges it): 0 until then.
    pub model_map_dac: u32,
    /// +0xDB0.
    pub alpha: FaceGenAlphaProperty,
    /// +0xDB4 (0x80), +0xDB8 (0x40).
    pub textures: [FaceGenTexture; 2],
    /// IDA: N/A host record.
    pub load_attempts: u32,
    pub ctl_loaded: bool,
    /// "Unable to load CTL file".
    pub load_warning: bool,
    /// FR2 ASSERT violations (6ED6D0).
    pub warnings: Vec<String>,
}

/// IDA: 00B39B80: null until 00553550 stores the object.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct FaceGenSingleton(pub Option<Box<FaceGenState>>);

/// IDA: 006F6BF0 last error (00B3F068/00B3F06C, text truncated to 255): the first code and text stick; the original never
/// clears it. Codes: 1 read failed, 2 header mismatch, 4 open failed or version mismatch. Completion: 100%.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct FaceGenLastError {
    pub code: u32,
    pub text: Vec<u8>,
}

impl FaceGenLastError {
    /// IDA: 006F6BF0: only the first error sticks; text longer than 255 bytes asserts (lastError.cpp line 65) and is cut
    /// to 255.
    pub(crate) fn set(&mut self, code: u32, text: &[u8]) {
        if self.code != 0 {
            return;
        }
        self.code = code;
        if text.len() >= 0x100 {
            fr2_assert(None, ".\\lastError.cpp", 65);
            self.text = text[..255].to_vec();
        } else {
            self.text = text.to_vec();
        }
    }
}

/// IDA: 006ED6D0 "FR2 ASSERT violation in %s line %i. Code may crash." through DebugMessage_Warning; execution continues.
/// Completion: 100%.
pub(crate) fn fr2_assert(warnings: Option<&mut Vec<String>>, file: &str, line: u32) {
    let text = format!("FR2 ASSERT violation in {file} line {line}. Code may crash.");
    emit(Level::Warning, &text);
    if let Some(warnings) = warnings {
        warnings.push(text);
    }
}

/// IDA: FutBinaryFileC 006F5ED0 / BSFaceGenBinaryFile 006F6110: +0x04 file name, +0x20 header, +0x40 file. The host borrows
/// the opened file.
pub(crate) struct FaceGenBinaryFile<'a> {
    pub(crate) filename: &'a [u8],
    pub(crate) file: Option<&'a mut OpenedFile>,
}

impl FaceGenBinaryFile<'_> {
    /// IDA: 006F5D40 vt+0x10 Read(buffer, element size, count): no file -> false; total = size * count; component size =
    /// size for 1, 2 and 4, else 4 when size % 4 == 0, else 1 after an assert (binaryFile.cpp line 445); success when the
    /// read proc returns the total. Completion: 100%.
    pub(crate) fn read(
        &mut self,
        buffer: &mut [u8],
        element_size: u32,
        warnings: &mut Vec<String>,
    ) -> bool {
        let Some(file) = self.file.as_deref_mut() else {
            return false;
        };
        let mut component = element_size;
        if !(element_size > 0 && (element_size <= 2 || element_size == 4)) {
            if element_size & 3 == 0 {
                component = 4;
            } else {
                fr2_assert(Some(warnings), ".\\binaryFile.cpp", 0x1BD);
                component = 1;
            }
        }
        file.read_components(buffer, Some(&[component])) == buffer.len()
    }

    /// IDA: 006F5E50: Read through vt+0x10; a failure records error 1 with the file name and deletes the file. Completion:
    /// 100%.
    pub(crate) fn read_checked(
        &mut self,
        buffer: &mut [u8],
        element_size: u32,
        warnings: &mut Vec<String>,
        last_error: &mut FaceGenLastError,
    ) -> bool {
        if self.file.is_none() {
            return false;
        }
        if !self.read(buffer, element_size, warnings) {
            last_error.set(1, self.filename);
            self.file = None;
            return false;
        }
        true
    }

    // IDA: N/A a helper, with no address of its own.
    pub(crate) fn read_u32(&mut self, warnings: &mut Vec<String>) -> Option<u32> {
        let mut bytes = [0; 4];
        self.read(&mut bytes, 4, warnings)
            .then(|| u32::from_le_bytes(bytes))
    }

    /// IDA: one float, its raw bits kept. A short read leaves `value` with what arrived, as the original's buffer.
    pub(crate) fn read_f32(&mut self, value: &mut f32, warnings: &mut Vec<String>) -> bool {
        let mut bytes = value.to_le_bytes();
        let result = self.read(&mut bytes, 4, warnings);
        *value = f32::from_le_bytes(bytes);
        result
    }

    /// IDA: N/A a helper, with no address of its own.
    /// The original's checked-iterator test faults on an empty vector here; the host reads nothing instead (the C++ port's
    /// behaviour). A short read keeps what arrived, as the original's vector does.
    pub(crate) fn read_floats(&mut self, data: &mut [f32], warnings: &mut Vec<String>) -> bool {
        let mut bytes: Vec<u8> = data.iter().flat_map(|value| value.to_le_bytes()).collect();
        let result = self.read(&mut bytes, 4, warnings);
        let (chunks, _) = bytes.as_chunks::<4>();
        for (value, raw) in data.iter_mut().zip(chunks) {
            *value = f32::from_le_bytes(*raw);
        }
        result
    }
}

/// IDA: 00527160 vector<float>::resize(n, value); 00552240 FgMatrix(rows, cols) zero-fills through it. A retry on the same
/// state keeps earlier values, as the original's resize does.
fn resize_matrix(matrix: &mut FaceGenMatrix, rows: u32, cols: u32, count: u32) {
    matrix.rows = rows;
    matrix.cols = cols;
    matrix.data.resize(count as usize, 0.0);
}

/// IDA: 005511D0 sum of squares, row-major.
#[allow(clippy::assign_op_pattern)] // 005511D0's operand order: the new term first.
fn sum_of_squares(matrix: &FaceGenMatrix) -> f32 {
    let mut sum = 0.0f32;
    for row in 0..matrix.rows {
        for col in 0..matrix.cols {
            let value = matrix.data[(col + row * matrix.cols) as usize];
            sum = value * value + sum;
        }
    }
    sum
}

/// IDA: 00552310 matrix * scalar (new matrix).
pub(crate) fn scaled(matrix: &FaceGenMatrix, scale: f32) -> FaceGenMatrix {
    FaceGenMatrix {
        rows: matrix.rows,
        cols: matrix.cols,
        data: matrix.data.iter().map(|&value| value * scale).collect(),
    }
}

/// IDA: 00552730 transpose.
pub(crate) fn transposed(matrix: &FaceGenMatrix) -> FaceGenMatrix {
    let mut result = FaceGenMatrix {
        rows: matrix.cols,
        cols: matrix.rows,
        data: vec![0.0; matrix.rows as usize * matrix.cols as usize],
    };
    for row in 0..matrix.rows {
        for col in 0..matrix.cols {
            result.data[(col * result.cols + row) as usize] =
                matrix.data[(row * matrix.cols + col) as usize];
        }
    }
    result
}

const MATRIX_HEADER: &str = "e:\\networkprojectspc\\oblivionse\\sdk\\facegen\\matrixVT.hpp";

/// IDA: 005523C0 multiply: R[i][j] = 0, then R += A[i][p] * B[p][j] stored as float each step; a column/row mismatch
/// asserts (matrixVT.hpp 269).
#[allow(clippy::assign_op_pattern)] // 005523C0's operand order: the new term first.
pub(crate) fn multiplied(
    a: &FaceGenMatrix,
    b: &FaceGenMatrix,
    warnings: &mut Vec<String>,
) -> FaceGenMatrix {
    if a.cols != b.rows {
        fr2_assert(Some(warnings), MATRIX_HEADER, 0x10D);
    }
    let mut result = FaceGenMatrix {
        rows: a.rows,
        cols: b.cols,
        data: vec![0.0; a.rows as usize * b.cols as usize],
    };
    for i in 0..a.rows {
        for j in 0..b.cols {
            let mut out = 0.0f32;
            for p in 0..a.cols {
                out = a.data[(i * a.cols + p) as usize] * b.data[(p * b.cols + j) as usize] + out;
            }
            result.data[(i * b.cols + j) as usize] = out;
        }
    }
    result
}

/// IDA: 00552630 elementwise difference; a shape mismatch asserts (matrixVT.hpp 452).
pub(crate) fn subtracted(
    a: &FaceGenMatrix,
    b: &FaceGenMatrix,
    warnings: &mut Vec<String>,
) -> FaceGenMatrix {
    if a.rows != b.rows || a.cols != b.cols {
        fr2_assert(Some(warnings), MATRIX_HEADER, 452);
    }
    FaceGenMatrix {
        rows: a.rows,
        cols: a.cols,
        data: a
            .data
            .iter()
            .zip(&b.data)
            .map(|(&left, &right)| left - right)
            .collect(),
    }
}

/// IDA: 006EF020: for t = 0..3 a count, `count` controls {1, c[t], c[t] floats} and a length-prefixed name. Completion:
/// 100%.
/// TODO: [ ] A corrupt count or length aborts the host in `Vec::resize`, where the original's allocation throws.
fn read_linear_controls(state: &mut FaceGenState, file: &mut FaceGenBinaryFile<'_>) -> bool {
    let FaceGenState {
        linear_controls,
        counts,
        warnings,
        ..
    } = state;
    for (controls, &columns) in linear_controls.iter_mut().zip(counts.iter()) {
        let Some(count) = file.read_u32(warnings) else {
            return false;
        };
        // 006EEEE0 resize(count, default control).
        controls.resize(count as usize, FaceGenLinearControl::default());
        for control in controls {
            resize_matrix(&mut control.coefficients, 1, columns, columns);
            if !file.read_floats(&mut control.coefficients.data, warnings) {
                return false;
            }
            let Some(length) = file.read_u32(warnings) else {
                return false;
            };
            control.name.resize(length as usize, 0);
            if !file.read(&mut control.name, 1, warnings) {
                return false;
            }
        }
    }
    true
}

/// IDA: 006EE270 FanControls load and setup (see the types). Completion: 100%.
// 006EE270 walks basis[t] and the t-indexed slots in lockstep, and adds in this operand order.
#[allow(clippy::needless_range_loop, clippy::assign_op_pattern)]
fn load_fan_controls(state: &mut FaceGenState, file: &mut FaceGenBinaryFile<'_>) -> bool {
    let basis = [state.counts[0], state.counts[2]];
    let FaceGenState {
        fan_controls: f,
        warnings,
        ..
    } = state;
    // 006EE2BA..006EE391.
    for group in &mut f.group_controls {
        for pair in group {
            for t in 0..2 {
                let element = &mut pair[t];
                resize_matrix(&mut element.coefficients, 1, basis[t], basis[t]);
                if !file.read_floats(&mut element.coefficients.data, warnings)
                    || !file.read_f32(&mut element.value, warnings)
                {
                    return false;
                }
                element.inverse_norm_squared = 1.0 / sum_of_squares(&element.coefficients);
            }
        }
    }
    // 006EE397..006EE4A0: the 5x5 pairs, diagonal skipped.
    for a in 0..5 {
        for b in 0..5 {
            if a == b {
                continue;
            }
            let element = &mut f.pair_controls[a][b];
            let mut sum = 0.0f32;
            for t in 0..2 {
                resize_matrix(&mut element.coefficients[t], 1, basis[t], basis[t]);
                if !file.read_floats(&mut element.coefficients[t].data, warnings) {
                    return false;
                }
                sum = sum_of_squares(&element.coefficients[t]) + sum;
            }
            if !file.read_f32(&mut element.value, warnings) {
                return false;
            }
            element.inverse_norm_squared = 1.0 / sum;
        }
    }
    // 006EE4A6..006EE5FE: per group c0 x 1, c2 x 1, n x n, c0 x c0, c2 x c2.
    let joint = basis[0].wrapping_add(basis[1]);
    for group in &mut f.groups {
        for t in 0..2 {
            resize_matrix(&mut group.matrices[t], basis[t], 1, basis[t]);
            if !file.read_floats(&mut group.matrices[t].data, warnings) {
                return false;
            }
        }
        resize_matrix(
            &mut group.matrices[2],
            joint,
            joint,
            joint.wrapping_mul(joint),
        );
        if !file.read_floats(&mut group.matrices[2].data, warnings) {
            return false;
        }
        for t in 0..2 {
            let square = basis[t].wrapping_mul(basis[t]);
            resize_matrix(&mut group.matrices[3 + t], basis[t], basis[t], square);
            if !file.read_floats(&mut group.matrices[3 + t].data, warnings) {
                return false;
            }
        }
    }
    // 006EE604..006EE831: Gram-Schmidt of the two group controls.
    for g in 0..5 {
        for t in 0..2 {
            let first = &f.group_controls[g][0][t].coefficients;
            let second = &f.group_controls[g][1][t].coefficients;
            let norm = sum_of_squares(first).sqrt();
            if f64::from(norm) <= 0.001 {
                fr2_assert(Some(warnings), ".\\FanControls.cpp", 0x85);
            }
            // 006EE685 006EE130: 00552310 by the float reciprocal; 006EE69B and 006EE775 assign through 005520E0.
            let out_first = scaled(first, 1.0 / norm);
            let dot = multiplied(second, &transposed(&out_first), warnings).data[0];
            let mut out_second = subtracted(second, &scaled(&out_first, dot), warnings);
            let second_norm = sum_of_squares(&out_second).sqrt();
            if f64::from(second_norm) <= 0.001 {
                fr2_assert(Some(warnings), ".\\FanControls.cpp", 0x8D);
            }
            // 006EE80E 00551D40 in-place scale by the float reciprocal.
            let inverse = 1.0 / second_norm;
            for value in &mut out_second.data {
                *value *= inverse;
            }
            f.orthonormal[g][t] = FaceGenOrthonormalPair {
                first: out_first,
                second: out_second,
            };
        }
    }
    // 006EE837..006EE9D9: inverse of {m_a . m_b} per group and basis.
    for g in 0..5 {
        for t in 0..2 {
            let mut m = [0.0f32; 4];
            let mut index = 0;
            for a in 0..2 {
                for b in 0..2 {
                    let product = multiplied(
                        &f.group_controls[g][a][t].coefficients,
                        &transposed(&f.group_controls[g][b][t].coefficients),
                        warnings,
                    );
                    m[index] += product.data[0];
                    index += 1;
                }
            }
            let determinant = m[3] * m[0] - m[2] * m[1];
            let inverse = 1.0 / determinant;
            f.inverse_gram[g][t] = [
                m[3] * inverse,
                -m[1] * inverse,
                -m[2] * inverse,
                inverse * m[0],
            ];
        }
    }
    f.loaded = true;
    true
}

/// IDA: 006F5ED0 BSFaceGenBinaryFile(header) and 006F66E0 Open for a file the caller opened (BSFile_OpenCore(path, 0, 0x2800,
/// archive ? 0x100 : 1)): 006F5ED0 asserts a header length other than 8 and stores '0' at header[5]; a null file records
/// error 4; the eight header bytes compared with the file's with byte 5 taken from the expected header (006F5DE0, error 2);
/// a file version byte above '0' (signed) must equal the expected one (error 4). Completion: 100%.
pub(crate) fn open_binary_file<'a>(
    header: [u8; 8],
    opened: Option<&'a mut OpenedFile>,
    path: &'a [u8],
    warnings: &mut Vec<String>,
    last_error: &mut FaceGenLastError,
) -> Option<FaceGenBinaryFile<'a>> {
    let mut header = header;
    header[5] = b'0';
    let Some(opened) = opened else {
        last_error.set(4, path);
        return None;
    };
    let mut file = FaceGenBinaryFile {
        filename: path,
        file: Some(opened),
    };
    let mut buffer = [0u8; 9];
    if !file.read_checked(&mut buffer[..8], 1, warnings, last_error) {
        return None;
    }
    // 006F6819..006F6896: compare the header with the read bytes (006F5DE0 FgString_Compare), except that byte 5 is taken
    // from the expected header.
    let mut copy = buffer;
    copy[5] = header[5];
    let length = copy
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(copy.len());
    let compared = length.min(header.len());
    let mut order = header[..compared].cmp(&copy[..compared]) as i32;
    if order == 0 {
        order = if header.len() < length {
            -1
        } else {
            i32::from(header.len() != length)
        };
    }
    if order != 0 {
        last_error.set(2, path);
        return None;
    }
    // 006F692E..006F69C6: a version byte above '0' must equal the expected one; then the header takes the file's byte.
    if buffer[5] as i8 > b'0' as i8 && header[5] != buffer[5] {
        last_error.set(4, path);
        return None;
    }
    Some(file)
}

/// IDA: 006ED850 for an opened file (the 006F66E0 header check included). Returns false where the original does; the state
/// keeps what was read. Completion: 100%.
pub fn load_facegen_ctl(
    state: &mut FaceGenState,
    last_error: &mut FaceGenLastError,
    opened: Option<&mut OpenedFile>,
    path: &[u8],
) -> bool {
    // 006ED8DF..006ED8E8: BSFaceGenBinaryFile("FRCTL001").
    let Some(mut file) =
        open_binary_file(*b"FRCTL001", opened, path, &mut state.warnings, last_error)
    else {
        return false;
    };
    // 006ED937..006ED9A8: 24 bytes: two words, then the four counts.
    let mut words = [0u8; 24];
    if !file.read_checked(&mut words, 1, &mut state.warnings, last_error) {
        return false;
    }
    let word = |index: usize| {
        u32::from_le_bytes([
            words[index * 4],
            words[index * 4 + 1],
            words[index * 4 + 2],
            words[index * 4 + 3],
        ])
    };
    state.header_words = [word(0), word(1)];
    state.counts = [word(2), word(3), word(4), word(5)];
    read_linear_controls(state, &mut file) && load_fan_controls(state, &mut file)
}

/// What 00761270 receives for one FaceGen texture. The source format is 00B25E48 RGB24.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceGenTextureRequest<'a> {
    pub width: u32,
    pub height: u32,
    pub texels: &'a [u8],
    /// 00B256D0.
    pub texture_prefs: [u32; 3],
    /// 00B3F708 (cleared).
    pub mipmap_by_default: bool,
    /// 0071B280 after 00498614.
    pub device_image_converter: bool,
}

/// IDA: N/A a FaceGen texture through the created renderer's 00761270 (`create_renderer_source_texture`): the source format
/// is 00B25E48 RGB24.
/// Completion: 100% of the mapping; the upload is the renderer's. Panics without a device, where the original faults.
pub fn renderer_facegen_upload(
    creation: &mut RendererCreation,
    request: &FaceGenTextureRequest<'_>,
) -> FaceGenTextureUpload {
    let device = creation
        .device
        .as_deref_mut()
        .expect("FaceGen runs after the renderer has its device");
    let created = create_renderer_source_texture(
        device,
        &creation.state,
        &original_rgb24_pixel_format(),
        request.width,
        request.height,
        request.texels,
        request.texture_prefs,
        request.mipmap_by_default,
        request.device_image_converter,
        &mut creation.device_calls,
    );
    FaceGenTextureUpload {
        table_case: created.prepared.upload.table_case,
        format: created.prepared.upload.d3d_format,
        texels: created.prepared.texels,
        texture: created.texture.is_some(),
    }
}

/// IDA: the inputs of 00553140. `create_texture` is the renderer step (00761270, `renderer_facegen_upload`), absent where a
/// test has no renderer.
pub struct FaceGenInputs<'a> {
    pub finder: &'a FileFinder,
    pub archives: Option<&'a mut ArchiveManager>,
    pub create_texture:
        Option<&'a mut dyn FnMut(&FaceGenTextureRequest<'_>) -> FaceGenTextureUpload>,
    /// 00B256D0.
    pub texture_prefs: [u32; 3],
    /// 00B3F708 (cleared).
    pub mipmap_by_default: bool,
    /// 0071B280 after 00498614.
    pub device_image_converter: bool,
    /// 00701C26 gate (0040E62F).
    pub object_load_callbacks_enabled: bool,
}

/// IDA: 00553550: when 00B39B80 is null, FormHeap(0xDBC) + 00553140 (a failed allocation stores null). Completion: 100% of
/// the reached path; the allocation is the host's. 00553140 -> 00552ED0 FaceGen_ConstructHeader and 00552D90 are the
/// default state.
pub fn ensure_facegen_initialized(
    singleton: &mut FaceGenSingleton,
    last_error: &mut FaceGenLastError,
    mut inputs: FaceGenInputs<'_>,
) {
    if singleton.0.is_some() {
        return;
    }
    let mut state = Box::<FaceGenState>::default();
    let path = "FaceGen\\si.ctl";
    // 005531D6..00553288: one retry, then the warning.
    let mut attempt = |state: &mut FaceGenState, last_error: &mut FaceGenLastError| {
        state.load_attempts += 1;
        let mut opened = bs_file_open_core(
            inputs.finder,
            inputs.archives.as_deref_mut(),
            path,
            0,
            0x2800,
            0x100,
        );
        load_facegen_ctl(state, last_error, opened.as_mut(), path.as_bytes())
    };
    state.ctl_loaded = attempt(&mut state, last_error) || attempt(&mut state, last_error);
    if !state.ctl_loaded {
        state.load_warning = true;
        emit(Level::Warning, "Unable to load CTL file");
    }
    // 00553290..00553315: zero column vectors sized by each type's first linear control (rows = its column count, cols =
    // its row count).
    for i in 0..4 {
        let (rows, cols) = state.linear_controls[i].first().map_or((0, 0), |first| {
            (first.coefficients.cols, first.coefficients.rows)
        });
        resize_matrix(&mut state.coefficients[i], rows, cols, rows * cols);
    }
    state.model_map_dac = 0;
    // 00553323..005533C6 NiAlphaProperty.
    state.alpha.flags = 0xEC;
    state.alpha.test_reference = 0;
    state.alpha.flags |= 0x200;
    state.alpha.flags = (state.alpha.flags & 0xE3FF) | 0x1000;
    state.alpha.test_reference = 100;
    // 005533D2..00553528: two 32x32 RGB24 pixel data filled with 0x80 and 0x40, each through
    // NiSourceTexture_CreateFromPixelData with 00B256D0.
    for (texture, grey) in state.textures.iter_mut().zip([0x80u8, 0x40]) {
        texture.grey = grey;
        texture.rgb24 = vec![grey; 32 * 32 * 3];
        if let Some(create_texture) = inputs.create_texture.as_deref_mut() {
            texture.upload = create_texture(&FaceGenTextureRequest {
                width: 32,
                height: 32,
                texels: &texture.rgb24,
                texture_prefs: inputs.texture_prefs,
                mipmap_by_default: inputs.mipmap_by_default,
                device_image_converter: inputs.device_image_converter,
            });
            // 00701C26..00701C42: with the 0040E62F gate, renderer data and the constructor's +0x40 = 1 (00701D14), 007021B0
            // drops the texture's only reference to the app pixel data, destroying the NiPixelData.
            if inputs.object_load_callbacks_enabled && texture.upload.texture {
                texture.rgb24 = Vec::new();
            }
        }
    }
    singleton.0 = Some(state);
}

// IDA: 00527160 the std::vector<float> resize(count, 0.0): kept elements stay; a shrink keeps the allocation.
pub(crate) fn resize_matrix_data(matrix: &mut FaceGenMatrix, count: u32) {
    matrix.data.resize(count as usize, 0.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bs::file::{NiFile, NiFileMode};
    use crate::bs::file_finder::BsFileCallbacks;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("recomp_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A CTL with counts {1, 1, 1, 0}: every block of 006EF020/006EE270.
    fn small_ctl(header: &[u8; 8], truncate: bool) -> Vec<u8> {
        let mut out = Vec::new();
        out.write_all(header).unwrap();
        for word in [7u32, 9, 1, 1, 1, 0] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        // Linear controls: one per type 0..2, none for type 3.
        for type_index in 0..3u8 {
            out.extend_from_slice(&1u32.to_le_bytes());
            out.extend_from_slice(&2.0f32.to_le_bytes());
            out.extend_from_slice(&1u32.to_le_bytes());
            out.push(b'A' + type_index);
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        if truncate {
            return out;
        }
        // Group controls 5 x 2 x 2: coefficient then value.
        for i in 0..20 {
            let value: f32 = if i % 2 == 0 { 3.0 } else { 4.0 };
            out.extend_from_slice(&value.to_le_bytes());
            out.extend_from_slice(&0.5f32.to_le_bytes());
        }
        // Pairs 20 x (two coefficients, value).
        for _ in 0..20 {
            out.extend_from_slice(&1.0f32.to_le_bytes());
            out.extend_from_slice(&2.0f32.to_le_bytes());
            out.extend_from_slice(&0.25f32.to_le_bytes());
        }
        // Group statistics 5 x (1 + 1 + 4 + 1 + 1).
        for i in 0..40 {
            out.extend_from_slice(&(i as f32).to_le_bytes());
        }
        out
    }

    fn open(root: &Path, name: &str) -> OpenedFile {
        OpenedFile::Ni(NiFile::new(root, name, NiFileMode::ReadOnly, 0x400))
    }

    /// `verify_facegen_fixtures`: 006F66E0/006ED850/006EF020/006EE270 on fixture files.
    #[test]
    fn verify_facegen_fixtures() {
        let temp = TempRoot::new("facegen_fixtures");
        let root = temp.0.as_path();
        std::fs::write(root.join("good.ctl"), small_ctl(b"FRCTL001", false)).unwrap();
        let mut error = FaceGenLastError::default();
        let mut good = FaceGenState::default();
        assert!(
            load_facegen_ctl(
                &mut good,
                &mut error,
                Some(&mut open(root, "good.ctl")),
                b"good.ctl"
            ),
            "A complete fixture CTL must load"
        );
        assert_eq!(good.header_words, [7, 9]);
        assert_eq!(good.counts, [1, 1, 1, 0]);
        assert_eq!(good.linear_controls[0].len(), 1);
        assert!(good.linear_controls[3].is_empty());
        assert_eq!(good.linear_controls[2][0].name, b"C");
        assert!(good.fan_controls.loaded);
        assert_eq!(error.code, 0);

        let g0 = &good.fan_controls.group_controls[0];
        assert_eq!(g0[0][0].coefficients.data, [3.0]);
        assert_eq!(g0[0][1].value, 0.5);
        assert_eq!(
            g0[0][0].inverse_norm_squared.to_bits(),
            (1.0f32 / 9.0).to_bits()
        );

        let pairs = &good.fan_controls.pair_controls;
        assert_eq!(pairs[0][1].value, 0.25);
        assert!(
            pairs[0][0].coefficients[0].data.is_empty(),
            "the skipped diagonal"
        );
        assert_eq!(pairs[1][0].inverse_norm_squared.to_bits(), 0.2f32.to_bits());

        let groups = &good.fan_controls.groups;
        assert_eq!(groups[0].matrices[2].rows, 2);
        assert_eq!(groups[0].matrices[2].data, [2.0, 3.0, 4.0, 5.0]);
        assert_eq!(groups[4].matrices[4].data, [39.0]);

        // 1-D controls: the second is parallel to the first, so the Gram-Schmidt residual is zero and FanControls.cpp line
        // 141 asserts.
        assert_eq!(good.fan_controls.orthonormal[0][0].first.data, [1.0]);
        assert_eq!(
            good.warnings.first().map(String::as_str),
            Some("FR2 ASSERT violation in .\\FanControls.cpp line 141. Code may crash.")
        );

        std::fs::write(root.join("header.ctl"), small_ctl(b"FRCTX001", false)).unwrap();
        let mut error = FaceGenLastError::default();
        let mut header = FaceGenState::default();
        assert!(!load_facegen_ctl(
            &mut header,
            &mut error,
            Some(&mut open(root, "header.ctl")),
            b"header.ctl"
        ));
        assert_eq!((error.code, error.text.as_slice()), (2, &b"header.ctl"[..]));

        std::fs::write(root.join("version.ctl"), small_ctl(b"FRCTL101", false)).unwrap();
        let mut error = FaceGenLastError::default();
        assert!(
            !load_facegen_ctl(
                &mut FaceGenState::default(),
                &mut error,
                Some(&mut open(root, "version.ctl")),
                b"version.ctl"
            ),
            "A version byte above '0' that differs must fail"
        );
        assert_eq!(error.code, 4);

        std::fs::write(root.join("older.ctl"), small_ctl(b"FRCTL/01", false)).unwrap();
        let mut error = FaceGenLastError::default();
        assert!(
            load_facegen_ctl(
                &mut FaceGenState::default(),
                &mut error,
                Some(&mut open(root, "older.ctl")),
                b"older.ctl"
            ),
            "A version byte at or below '0' must be accepted"
        );

        std::fs::write(root.join("short.ctl"), small_ctl(b"FRCTL001", true)).unwrap();
        let mut error = FaceGenLastError::default();
        let mut truncated = FaceGenState::default();
        assert!(
            !load_facegen_ctl(
                &mut truncated,
                &mut error,
                Some(&mut open(root, "short.ctl")),
                b"short.ctl"
            ),
            "A short read inside 006EE270 must fail without an error code"
        );
        assert_eq!(error.code, 0);
        assert_eq!(truncated.counts[0], 1);

        let mut error = FaceGenLastError::default();
        let mut missing = FaceGenState::default();
        assert!(!load_facegen_ctl(
            &mut missing,
            &mut error,
            None,
            b"FaceGen\\si.ctl"
        ));
        assert_eq!(
            (error.code, error.text.as_slice()),
            (4, &b"FaceGen\\si.ctl"[..])
        );
        // First error wins.
        load_facegen_ctl(
            &mut missing,
            &mut error,
            Some(&mut open(root, "header.ctl")),
            b"header.ctl",
        );
        assert_eq!(error.code, 4, "006F6BF0 must keep the first error");
    }

    /// The branches of 00553140 around the renderer step, without a renderer. Not in the C++ test, which needs a device.
    #[test]
    fn retry_warning_and_texture_seam() {
        let temp = TempRoot::new("facegen_inputs");
        let mut callbacks = BsFileCallbacks::default();
        let finder = FileFinder::new(temp.0.as_path(), &mut callbacks);
        // No si.ctl: two attempts, error 4, the warning, and empty coefficient vectors.
        let mut singleton = FaceGenSingleton::default();
        let mut error = FaceGenLastError::default();
        let mut requests = Vec::new();
        let mut create = |request: &FaceGenTextureRequest<'_>| {
            requests.push((
                request.width,
                request.height,
                request.texels[0],
                request.texture_prefs,
            ));
            FaceGenTextureUpload {
                texture: request.texels[0] == 0x80,
                ..FaceGenTextureUpload::default()
            }
        };
        ensure_facegen_initialized(
            &mut singleton,
            &mut error,
            FaceGenInputs {
                finder: &finder,
                archives: None,
                create_texture: Some(&mut create),
                texture_prefs: [6, 3, 2],
                mipmap_by_default: false,
                device_image_converter: true,
                object_load_callbacks_enabled: true,
            },
        );
        let state = singleton.0.as_ref().unwrap();
        assert_eq!(state.load_attempts, 2);
        assert!(state.load_warning && !state.ctl_loaded);
        assert_eq!(error.code, 4);
        assert_eq!(state.coefficients[0].data.len(), 0);
        assert_eq!(
            state.alpha,
            FaceGenAlphaProperty {
                flags: 0x12EC,
                test_reference: 100
            }
        );
        assert_eq!(
            requests,
            [(32, 32, 0x80, [6, 3, 2]), (32, 32, 0x40, [6, 3, 2])]
        );
        // The 0040E62F gate drops the NiPixelData only for the texture that exists.
        assert!(state.textures[0].rgb24.is_empty());
        assert_eq!(state.textures[1].rgb24.len(), 32 * 32 * 3);
        // A second call finds the singleton.
        let before = singleton.clone();
        ensure_facegen_initialized(
            &mut singleton,
            &mut error,
            FaceGenInputs {
                finder: &finder,
                archives: None,
                create_texture: None,
                texture_prefs: [6, 3, 2],
                mipmap_by_default: false,
                device_image_converter: false,
                object_load_callbacks_enabled: false,
            },
        );
        assert_eq!(singleton, before);
    }
}
