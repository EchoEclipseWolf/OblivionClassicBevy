//! The FaceGen SDK's model files beside a head part's NIF: the EGM geometry morphs (`006F0A00`, "FREGM002") and the TRI
//! morph data (`006F4B50`, "FRTRI003"), both read through BSFaceGenBinaryFile (`006F5ED0`, `006F66E0`, `006F5D40`). The
//! EGT texture morphs (`00557E60` → `0054DA40`) are the generated face texture's, a host deferral (see `docs/open-items.md`).
//! Evidence: `evidence/ida-names-gameplay-2026-11-07/facegen-*` (the batches' before and after exports).
//!
//! Both readers run on the IO thread (`00435300` QueuedHead_Run, `0043EB80` QueuedHelmet_CheckFinished). The TRI's
//! differential morphs are scaled with `fild`/`fmul` and stored as floats: the product of an i16 and a float is exact in
//! either x87 precision, so it's rounded once to f32.

use crate::bs::file::OpenedFile;
use crate::tes::facegen::{FaceGenLastError, open_binary_file};

/// IDA: one EGM morph (20 bytes, `006F0A00`): +0x00 the scale, +0x04 std::vector of three i16 per vertex (6 bytes each).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EgmMorph {
    pub scale_00: f32,
    pub deltas_04: Vec<[i16; 3]>,
}

/// IDA: the 0x24 EGM object `00558770` builds: +0x00 the basis id (header word 3), +0x04 the symmetric morphs, +0x14 the
/// asymmetric morphs (two std::vector<EgmMorph> of 16 bytes, `006EF4A0`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EgmData {
    pub basis_00: u32,
    pub symmetric_04: Vec<EgmMorph>,
    pub asymmetric_14: Vec<EgmMorph>,
}

// IDA: 005598F0 the morph vector's resize (00559240 erases the tail), here to the header's count with default morphs.
fn resize_morphs(morphs: &mut Vec<EgmMorph>, count: u32) {
    morphs.resize(count as usize, EgmMorph::default());
}

/// IDA: 006F0A00 FaceGen_ReadEgm(path, out basis, symmetric, asymmetric): BSFaceGenBinaryFile("FREGM002") opened (the
/// caller's BSFile_OpenCore(path, 0, 0x2800, 1)); a 56-byte checked header: +0x00 vertex count, +0x04 symmetric count, +0x08
/// asymmetric count, +0x0C the basis id, 40 reserved bytes; the basis stored, both vectors resized (`005598F0`); per
/// symmetric morph, then per asymmetric morph: its deltas resized to the vertex count (`006F0400`), a float scale (Read 4 × 1),
/// then per vertex three i16 (Read 2 × 3). Any failed read returns false and keeps what was read. Completion: 100%.
pub fn read_egm(
    out: &mut EgmData,
    opened: Option<&mut OpenedFile>,
    path: &[u8],
    warnings: &mut Vec<String>,
    last_error: &mut FaceGenLastError,
) -> bool {
    let Some(mut file) = open_binary_file(*b"FREGM002", opened, path, warnings, last_error) else {
        return false;
    };
    let mut header = [0_u8; 56];
    if !file.read_checked(&mut header, 1, warnings, last_error) {
        return false;
    }
    let word = |index: usize| {
        u32::from_le_bytes(
            header[index * 4..index * 4 + 4]
                .try_into()
                .expect("four bytes"),
        )
    };
    let (vertices, symmetric, asymmetric) = (word(0), word(1), word(2));
    out.basis_00 = word(3);
    resize_morphs(&mut out.symmetric_04, symmetric);
    resize_morphs(&mut out.asymmetric_14, asymmetric);
    for morphs in [&mut out.symmetric_04, &mut out.asymmetric_14] {
        for morph in morphs.iter_mut() {
            // 006F0400(vertices, 0): a resize filling with zero deltas.
            morph.deltas_04.resize(vertices as usize, [0; 3]);
            let mut scale = morph.scale_00.to_le_bytes();
            if !file.read(&mut scale, 4, warnings) {
                return false;
            }
            morph.scale_00 = f32::from_le_bytes(scale);
            for delta in &mut morph.deltas_04 {
                let mut bytes = [0_u8; 6];
                if !file.read(&mut bytes, 2, warnings) {
                    return false;
                }
                *delta = [
                    i16::from_le_bytes([bytes[0], bytes[1]]),
                    i16::from_le_bytes([bytes[2], bytes[3]]),
                    i16::from_le_bytes([bytes[4], bytes[5]]),
                ];
            }
        }
    }
    true
}

/// IDA: a TRI labelled vertex (32 bytes, the vector at +0x60): +0x00 the vertex index, +0x04 a std::string name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TriLabelledVertex {
    pub index_00: u32,
    pub name_04: Vec<u8>,
}

/// IDA: a TRI labelled surface point (44 bytes, the vector at +0x70): +0x00 the triangle index, +0x04 three floats, +0x10
/// a std::string name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TriSurfacePoint {
    pub triangle_00: u32,
    pub coordinates_04: [f32; 3],
    pub name_10: Vec<u8>,
}

/// IDA: a TRI differential morph (44 bytes, the vector at +0x80): +0x00 a std::string name, +0x1C std::vector of float3, one
/// per base vertex, each the file's i16 times its scale.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TriDifferentialMorph {
    pub name_00: Vec<u8>,
    pub deltas_1c: Vec<[f32; 3]>,
}

/// IDA: a TRI statistical morph (48 bytes, the vector at +0x90): +0x00 a std::string name, +0x1C the index of its first
/// added vertex (the running count from the header's vertex count), +0x20 std::vector<u32> of the base vertices it moves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TriStatisticalMorph {
    pub name_00: Vec<u8>,
    pub base_index_1c: u32,
    pub vertices_20: Vec<u32>,
}

/// IDA: the 0xA0 TRI object `005586C0` builds and `006F4B50` fills: +0x00 the vertices and added vertices, +0x10 triangles,
/// +0x20 quads, +0x30 texture coordinates, +0x40 texture triangles, +0x50 texture quads, +0x60 labelled vertices, +0x70
/// labelled surface points, +0x80 differential morphs, +0x90 statistical morphs (each a 16-byte std::vector).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TriData {
    pub vertices_00: Vec<[f32; 3]>,
    pub triangles_10: Vec<[u32; 3]>,
    pub quads_20: Vec<[u32; 4]>,
    pub texture_coordinates_30: Vec<[f32; 2]>,
    pub texture_triangles_40: Vec<[u32; 3]>,
    pub texture_quads_50: Vec<[u32; 4]>,
    pub labelled_vertices_60: Vec<TriLabelledVertex>,
    pub surface_points_70: Vec<TriSurfacePoint>,
    pub differential_morphs_80: Vec<TriDifferentialMorph>,
    pub statistical_morphs_90: Vec<TriStatisticalMorph>,
}

impl TriData {
    /// IDA: 00556800 the base vertex count: the vertices' count, lowered to any statistical morph's +0x1C below it.
    /// Completion: 100%.
    #[must_use]
    pub fn base_vertex_count(&self) -> u32 {
        let mut count = self.vertices_00.len() as u32;
        for morph in &self.statistical_morphs_90 {
            if morph.base_index_1c < count {
                count = morph.base_index_1c;
            }
        }
        count
    }
}

// IDA: N/A the little-endian words of a buffer the reads filled.
fn words<const N: usize>(bytes: &[u8]) -> Vec<[u32; N]> {
    bytes
        .chunks_exact(4 * N)
        .map(|chunk| {
            std::array::from_fn(|i| {
                u32::from_le_bytes(chunk[i * 4..i * 4 + 4].try_into().expect("four bytes"))
            })
        })
        .collect()
}

// IDA: N/A floats from a buffer the reads filled.
fn floats<const N: usize>(bytes: &[u8]) -> Vec<[f32; N]> {
    words::<N>(bytes)
        .into_iter()
        .map(|word| word.map(f32::from_bits))
        .collect()
}

// IDA: the name arm of 006F4B50's labelled and morph reads: a u32 length; a nonzero length reads that many bytes, and the
// name is the first length - 1 of them (a length of 1 gives an empty name); zero reads nothing and leaves the name as it was.
fn read_name(
    file: &mut crate::tes::facegen::FaceGenBinaryFile<'_>,
    name: &mut Vec<u8>,
    warnings: &mut Vec<String>,
) -> bool {
    let Some(length) = file.read_u32(warnings) else {
        return false;
    };
    if length == 0 {
        return true;
    }
    let mut bytes = vec![0_u8; length as usize];
    if !file.read(&mut bytes, 1, warnings) {
        return false;
    }
    bytes.truncate(length as usize - 1);
    *name = bytes;
    true
}

// IDA: N/A one Read of `count` elements of `size` bytes into a fresh buffer.
fn read_block(
    file: &mut crate::tes::facegen::FaceGenBinaryFile<'_>,
    size: u32,
    count: u32,
    warnings: &mut Vec<String>,
) -> Option<Vec<u8>> {
    let mut bytes = vec![0_u8; (size * count) as usize];
    (count == 0 || file.read(&mut bytes, size, warnings)).then_some(bytes)
}

/// IDA: 006F4B50 FaceGen_ReadTri(path, out): BSFaceGenBinaryFile("FRTRI003") opened (BSFile_OpenCore(path, 0, 0x2800, 1)); a
/// 56-byte checked header: vertices, triangles, quads, labelled vertices, labelled surface points, texture coordinates, the
/// extension flags, differential morphs, statistical morphs, added vertices, 16 reserved bytes. The vectors are sized from it
/// (vertices + added), then read in order: the vertices (12 bytes each), triangles (12), quads (16); each labelled vertex
/// (an index, a name), each surface point (a triangle, 12 bytes, a name); with extension bit 0, the texture coordinates (8
/// bytes) and texture triangles and quads, or, with no coordinate count, one coordinate per vertex and the geometry's
/// triangles and quads copied (`006F1470`, `006F14D0`); each differential morph (a name, a float scale, the header's vertex
/// count of three i16, each times the scale); each statistical morph (a name, its first added vertex = the running count,
/// a u32 count, the indices). The running count must end at vertices + added (else error 1). Completion: 100%.
pub fn read_tri(
    out: &mut TriData,
    opened: Option<&mut OpenedFile>,
    path: &[u8],
    warnings: &mut Vec<String>,
    last_error: &mut FaceGenLastError,
) -> bool {
    let Some(mut file) = open_binary_file(*b"FRTRI003", opened, path, warnings, last_error) else {
        return false;
    };
    let mut header = [0_u8; 56];
    if !file.read_checked(&mut header, 1, warnings, last_error) {
        return false;
    }
    let word = |index: usize| {
        u32::from_le_bytes(
            header[index * 4..index * 4 + 4]
                .try_into()
                .expect("four bytes"),
        )
    };
    let (vertices, triangles, quads) = (word(0), word(1), word(2));
    let (labelled, surface, coordinates, extension) = (word(3), word(4), word(5), word(6));
    let (differential, statistical, added) = (word(7), word(8), word(9));
    // 006F4C5C..006F4D31: the sizes.
    out.vertices_00
        .resize((vertices.wrapping_add(added)) as usize, [0.0; 3]);
    out.triangles_10.resize(triangles as usize, [0; 3]);
    out.quads_20.resize(quads as usize, [0; 4]);
    out.labelled_vertices_60
        .resize(labelled as usize, TriLabelledVertex::default());
    out.surface_points_70
        .resize(surface as usize, TriSurfacePoint::default());
    out.differential_morphs_80
        .resize(differential as usize, TriDifferentialMorph::default());
    out.statistical_morphs_90
        .resize(statistical as usize, TriStatisticalMorph::default());
    let Some(bytes) = read_block(&mut file, 12, vertices.wrapping_add(added), warnings) else {
        return false;
    };
    out.vertices_00 = floats::<3>(&bytes);
    let Some(bytes) = read_block(&mut file, 12, triangles, warnings) else {
        return false;
    };
    out.triangles_10 = words::<3>(&bytes);
    let Some(bytes) = read_block(&mut file, 16, quads, warnings) else {
        return false;
    };
    out.quads_20 = words::<4>(&bytes);
    for entry in &mut out.labelled_vertices_60 {
        let Some(index) = file.read_u32(warnings) else {
            return false;
        };
        entry.index_00 = index;
        if !read_name(&mut file, &mut entry.name_04, warnings) {
            return false;
        }
    }
    for entry in &mut out.surface_points_70 {
        let Some(triangle) = file.read_u32(warnings) else {
            return false;
        };
        entry.triangle_00 = triangle;
        let Some(bytes) = read_block(&mut file, 12, 1, warnings) else {
            return false;
        };
        entry.coordinates_04 = floats::<3>(&bytes)[0];
        if !read_name(&mut file, &mut entry.name_10, warnings) {
            return false;
        }
    }
    if extension & 1 != 0 {
        if coordinates == 0 {
            // 006F51BE's fall-through (no coordinate count): one coordinate per vertex, the geometry's triangles and quads reused.
            let Some(bytes) = read_block(&mut file, 8, vertices, warnings) else {
                return false;
            };
            out.texture_coordinates_30 = floats::<2>(&bytes);
            out.texture_triangles_40 = out.triangles_10.clone();
            out.texture_quads_50 = out.quads_20.clone();
        } else {
            let Some(bytes) = read_block(&mut file, 8, coordinates, warnings) else {
                return false;
            };
            out.texture_coordinates_30 = floats::<2>(&bytes);
            let Some(bytes) = read_block(&mut file, 12, triangles, warnings) else {
                return false;
            };
            out.texture_triangles_40 = words::<3>(&bytes);
            let Some(bytes) = read_block(&mut file, 16, quads, warnings) else {
                return false;
            };
            out.texture_quads_50 = words::<4>(&bytes);
        }
    }
    for morph in &mut out.differential_morphs_80 {
        if !read_name(&mut file, &mut morph.name_00, warnings) {
            return false;
        }
        let mut scale = 0.0_f32;
        if !file.read_f32(&mut scale, warnings) {
            return false;
        }
        morph.deltas_1c = vec![[0.0; 3]; vertices as usize];
        for delta in &mut morph.deltas_1c {
            let mut bytes = [0_u8; 6];
            if !file.read(&mut bytes, 2, warnings) {
                return false;
            }
            // fild each i16, fmul the scale, fstp: the product is exact in the x87's mantissa, so one rounding to f32.
            *delta = std::array::from_fn(|i| {
                let value = i16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]);
                (f64::from(value) * f64::from(scale)) as f32
            });
        }
    }
    let mut running = vertices;
    for morph in &mut out.statistical_morphs_90 {
        if !read_name(&mut file, &mut morph.name_00, warnings) {
            return false;
        }
        morph.base_index_1c = running;
        let Some(count) = file.read_u32(warnings) else {
            return false;
        };
        let Some(bytes) = read_block(&mut file, 4, count, warnings) else {
            return false;
        };
        morph.vertices_20 = words::<1>(&bytes).into_iter().map(|[v]| v).collect();
        running = running.wrapping_add(count);
    }
    if running != vertices.wrapping_add(added) {
        last_error.set(1, path);
        return false;
    }
    true
}
