//! NiPixelFormat (0x44 bytes) and NiPixelData (0070E560), shared by the renderer texture path and the
//! effect textures. The raw format statics behind 0076C3B0 live here too: the C++ keeps them in
//! `bs/shader_manager`, but they are `.data` values with nothing of the device in them.

use bevy_ecs::prelude::*;

use super::stream_objects::NiClass;

/// IDA: one of NiPixelFormat's four +0x14 {component, representation, bits, signed} entries of 12 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelFormatComponent {
    pub component: u32,
    pub representation: u32,
    pub bits: u8,
    pub is_signed: u8,
}

/// IDA: one NiPixelFormat (0x44 bytes): +0 flags, +1 bits per pixel, +4 format, +8 tiling, +0xC renderer
/// hint (D3DFORMAT or 0xFFFFFFFF), +0x10 extra (007637D0 stores the table case), +0x14 the components.
/// `source_address` is host evidence: the static it was copied from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelFormat {
    pub source_address: u32,
    pub flags_00: u8,
    pub bits_per_pixel_01: u8,
    pub format_04: u32,
    pub tiling_08: u32,
    pub renderer_hint_0c: u32,
    pub extra_10: u32,
    pub components_14: [PixelFormatComponent; 4],
}

impl Default for PixelFormat {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            source_address: 0,
            flags_00: 0,
            bits_per_pixel_01: 0,
            format_04: 0,
            tiling_08: 0,
            renderer_hint_0c: 0xFFFF_FFFF,
            extra_10: 0,
            components_14: [PixelFormatComponent::default(); 4],
        }
    }
}

/// IDA: N/A a helper, with no address of its own.
pub(super) const fn format_row(
    source_address: u32,
    flags_00: u8,
    bits_per_pixel_01: u8,
    format_04: u32,
    tiling_08: u32,
    c: [(u32, u32, u8, u8); 4],
) -> PixelFormat {
    // IDA: N/A a helper, with no address of its own.
    const fn component(c: (u32, u32, u8, u8)) -> PixelFormatComponent {
        PixelFormatComponent {
            component: c.0,
            representation: c.1,
            bits: c.2,
            is_signed: c.3,
        }
    }
    PixelFormat {
        source_address,
        flags_00,
        bits_per_pixel_01,
        format_04,
        tiling_08,
        renderer_hint_0c: 0xFFFF_FFFF,
        extra_10: 0,
        components_14: [
            component(c[0]),
            component(c[1]),
            component(c[2]),
            component(c[3]),
        ],
    }
}

/// IDA: the 47 0x44-byte .data statics 0076C3B0 copies, from the exact executable bytes (the C++ `d3d_pixel_formats.inc`).
#[rustfmt::skip]
const D3D_PIXEL_FORMAT_STATICS: [PixelFormat; 47] = [
    format_row(0x00B25D70, 1, 8, 2, 0, [(16, 3, 8, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B25E00, 1, 32, 1, 0, [(0, 0, 8, 1), (1, 0, 8, 1), (2, 0, 8, 1), (3, 0, 8, 1)]),
    format_row(0x00B25F20, 1, 16, 8, 0, [(5, 0, 8, 1), (6, 0, 8, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B25FB0, 1, 0, 4, 0, [(4, 4, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B25FF8, 1, 0, 5, 0, [(4, 4, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26040, 1, 0, 6, 0, [(4, 4, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26088, 1, 128, 1, 0, [(0, 2, 32, 1), (1, 2, 32, 1), (2, 2, 32, 1), (3, 2, 32, 1)]),
    format_row(0x00B260D0, 1, 64, 1, 0, [(0, 1, 16, 1), (1, 1, 16, 1), (2, 1, 16, 1), (3, 1, 16, 1)]),
    format_row(0x00B26118, 1, 32, 11, 0, [(0, 2, 32, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26160, 1, 32, 12, 0, [(0, 1, 16, 1), (1, 1, 16, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B261A8, 1, 64, 12, 0, [(0, 2, 32, 1), (1, 2, 32, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B261F0, 1, 16, 11, 0, [(0, 1, 16, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26280, 1, 8, 11, 0, [(3, 0, 8, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B262C8, 1, 10, 0, 0, [(2, 0, 2, 1), (1, 0, 3, 1), (0, 0, 3, 1), (14, 5, 2, 1)]),
    format_row(0x00B26310, 1, 16, 1, 0, [(2, 0, 2, 1), (1, 0, 3, 1), (0, 0, 3, 1), (3, 0, 8, 1)]),
    format_row(0x00B263E8, 1, 16, 0, 0, [(2, 0, 5, 1), (1, 0, 6, 1), (0, 0, 5, 1), (19, 5, 0, 1)]),
    format_row(0x00B26478, 1, 16, 1, 0, [(2, 0, 5, 1), (1, 0, 5, 1), (0, 0, 5, 1), (3, 0, 1, 1)]),
    format_row(0x00B264C0, 1, 16, 0, 0, [(2, 0, 5, 1), (1, 0, 5, 1), (0, 0, 5, 1), (14, 5, 1, 1)]),
    format_row(0x00B26508, 1, 16, 1, 0, [(2, 0, 4, 1), (1, 0, 4, 1), (0, 0, 4, 1), (3, 0, 4, 1)]),
    format_row(0x00B26550, 1, 16, 0, 0, [(2, 0, 4, 1), (1, 0, 4, 1), (0, 0, 4, 1), (14, 0, 4, 1)]),
    format_row(0x00B26598, 1, 24, 0, 0, [(2, 0, 8, 1), (1, 0, 8, 1), (0, 0, 8, 1), (19, 5, 0, 1)]),
    format_row(0x00B265E0, 1, 32, 1, 0, [(2, 0, 8, 1), (1, 0, 8, 1), (0, 0, 8, 1), (3, 0, 8, 1)]),
    format_row(0x00B26628, 1, 32, 0, 0, [(2, 0, 8, 1), (1, 0, 8, 1), (0, 0, 8, 1), (14, 5, 8, 1)]),
    format_row(0x00B26670, 1, 32, 0, 0, [(0, 0, 8, 1), (1, 0, 8, 1), (2, 0, 8, 1), (14, 5, 8, 1)]),
    format_row(0x00B266B8, 1, 32, 1, 0, [(0, 0, 10, 1), (1, 0, 10, 1), (2, 0, 10, 1), (3, 0, 2, 1)]),
    format_row(0x00B26700, 1, 32, 1, 0, [(2, 0, 10, 1), (1, 0, 10, 1), (0, 0, 10, 1), (3, 0, 2, 1)]),
    format_row(0x00B26748, 1, 32, 12, 0, [(1, 0, 16, 1), (0, 0, 16, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26790, 1, 16, 9, 0, [(5, 0, 5, 1), (6, 0, 5, 1), (9, 0, 6, 0), (19, 5, 0, 0)]),
    format_row(0x00B267D8, 1, 16, 12, 0, [(16, 3, 8, 1), (3, 0, 8, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26820, 1, 8, 11, 0, [(9, 0, 8, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26868, 1, 16, 11, 0, [(9, 0, 16, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B268B0, 1, 16, 12, 0, [(9, 0, 8, 1), (3, 0, 8, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B268F8, 1, 8, 12, 0, [(9, 0, 4, 1), (3, 0, 4, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26940, 1, 32, 8, 0, [(5, 0, 8, 1), (6, 0, 8, 1), (7, 0, 8, 1), (8, 0, 8, 1)]),
    format_row(0x00B26988, 1, 32, 8, 0, [(5, 0, 16, 1), (6, 0, 16, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B269D0, 1, 32, 8, 0, [(5, 0, 10, 1), (6, 0, 11, 1), (7, 0, 11, 1), (19, 5, 0, 1)]),
    format_row(0x00B26A18, 1, 32, 9, 0, [(5, 0, 8, 1), (6, 0, 8, 1), (9, 0, 8, 0), (14, 5, 8, 0)]),
    format_row(0x00B26AA8, 1, 0, 11, 0, [(14, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26AF0, 1, 16, 11, 0, [(14, 5, 16, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26B38, 1, 32, 11, 0, [(14, 5, 32, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26B80, 1, 64, 11, 0, [(14, 5, 64, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26BC8, 1, 16, 15, 0, [(17, 0, 16, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26C10, 1, 32, 15, 0, [(17, 0, 32, 1), (19, 5, 0, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26C58, 1, 16, 15, 0, [(18, 0, 1, 1), (17, 0, 15, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26CA0, 1, 32, 15, 0, [(18, 0, 8, 1), (17, 0, 24, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26D30, 1, 32, 15, 0, [(14, 0, 8, 1), (17, 0, 24, 1), (19, 5, 0, 1), (19, 5, 0, 1)]),
    format_row(0x00B26D78, 1, 32, 15, 0, [(18, 0, 4, 1), (14, 0, 4, 1), (17, 0, 24, 1), (19, 5, 0, 1)]),
];

/// IDA: the 0076C3B0 switch: D3DFORMAT to static address.
#[rustfmt::skip]
const D3D_PIXEL_FORMAT_CASES: [(u32, u32); 101] = [
    (0x00000014, 0x00B26598),
    (0x00000015, 0x00B265E0),
    (0x00000016, 0x00B26628),
    (0x00000017, 0x00B263E8),
    (0x00000018, 0x00B264C0),
    (0x00000019, 0x00B26478),
    (0x0000001A, 0x00B26508),
    (0x0000001B, 0x00B262C8),
    (0x0000001C, 0x00B26280),
    (0x0000001D, 0x00B26310),
    (0x0000001E, 0x00B26550),
    (0x0000001F, 0x00B266B8),
    (0x00000020, 0x00B25E00),
    (0x00000021, 0x00B26670),
    (0x00000022, 0x00B26748),
    (0x00000023, 0x00B26700),
    (0x00000024, 0x00B260D0),
    (0x00000025, 0x00B26AA8),
    (0x00000026, 0x00B26AA8),
    (0x00000027, 0x00B26AA8),
    (0x00000028, 0x00B267D8),
    (0x00000029, 0x00B25D70),
    (0x0000002A, 0x00B26AA8),
    (0x0000002B, 0x00B26AA8),
    (0x0000002C, 0x00B26AA8),
    (0x0000002D, 0x00B26AA8),
    (0x0000002E, 0x00B26AA8),
    (0x0000002F, 0x00B26AA8),
    (0x00000030, 0x00B26AA8),
    (0x00000031, 0x00B26AA8),
    (0x00000032, 0x00B26820),
    (0x00000033, 0x00B268B0),
    (0x00000034, 0x00B268F8),
    (0x00000035, 0x00B26AA8),
    (0x00000036, 0x00B26AA8),
    (0x00000037, 0x00B26AA8),
    (0x00000038, 0x00B26AA8),
    (0x00000039, 0x00B26AA8),
    (0x0000003A, 0x00B26AA8),
    (0x0000003B, 0x00B26AA8),
    (0x0000003C, 0x00B25F20),
    (0x0000003D, 0x00B26790),
    (0x0000003E, 0x00B26A18),
    (0x0000003F, 0x00B26940),
    (0x00000040, 0x00B26988),
    (0x00000041, 0x00B26AA8),
    (0x00000042, 0x00B26AA8),
    (0x00000043, 0x00B269D0),
    (0x00000044, 0x00B26AA8),
    (0x00000045, 0x00B26AA8),
    (0x00000046, 0x00B26BC8),
    (0x00000047, 0x00B26C10),
    (0x00000048, 0x00B26AA8),
    (0x00000049, 0x00B26C58),
    (0x0000004A, 0x00B26AA8),
    (0x0000004B, 0x00B26CA0),
    (0x0000004C, 0x00B26AA8),
    (0x0000004D, 0x00B26D30),
    (0x0000004E, 0x00B26AA8),
    (0x0000004F, 0x00B26D78),
    (0x00000050, 0x00B26BC8),
    (0x00000051, 0x00B26868),
    (0x00000052, 0x00B26B38),
    (0x00000053, 0x00B26B38),
    (0x00000054, 0x00B26AA8),
    (0x00000055, 0x00B26AA8),
    (0x00000056, 0x00B26AA8),
    (0x00000057, 0x00B26AA8),
    (0x00000058, 0x00B26AA8),
    (0x00000059, 0x00B26AA8),
    (0x0000005A, 0x00B26AA8),
    (0x0000005B, 0x00B26AA8),
    (0x0000005C, 0x00B26AA8),
    (0x0000005D, 0x00B26AA8),
    (0x0000005E, 0x00B26AA8),
    (0x0000005F, 0x00B26AA8),
    (0x00000060, 0x00B26AA8),
    (0x00000061, 0x00B26AA8),
    (0x00000062, 0x00B26AA8),
    (0x00000063, 0x00B26AA8),
    (0x00000064, 0x00B26AA8),
    (0x00000065, 0x00B26AA8),
    (0x00000066, 0x00B26AA8),
    (0x00000067, 0x00B26AA8),
    (0x00000068, 0x00B26AA8),
    (0x00000069, 0x00B26AA8),
    (0x0000006A, 0x00B26AA8),
    (0x0000006B, 0x00B26AA8),
    (0x0000006C, 0x00B26AA8),
    (0x0000006D, 0x00B26AA8),
    (0x0000006E, 0x00B26B80),
    (0x0000006F, 0x00B261F0),
    (0x00000070, 0x00B26160),
    (0x00000071, 0x00B260D0),
    (0x00000072, 0x00B26118),
    (0x00000073, 0x00B261A8),
    (0x00000074, 0x00B26088),
    (0x00000075, 0x00B26AF0),
    (0x31545844, 0x00B25FB0),
    (0x33545844, 0x00B25FF8),
    (0x35545844, 0x00B26040),
];

const DEFAULT_D3D_PIXEL_FORMAT_STATIC: u32 = 0x00B2_6AA8;

/// IDA: N/A one of the raw .data statics behind 0076C3B0 by address (the copy before the +0xC/+0x10
/// stores); `None` when the address is not one. Completion: 100%.
#[must_use]
pub fn d3d_pixel_format_static(address: u32) -> Option<PixelFormat> {
    D3D_PIXEL_FORMAT_STATICS
        .iter()
        .find(|format| format.source_address == address)
        .copied()
}

/// IDA: 0076C3B0 NiDX9_PixelFormatFromD3DFormat(format, dst): one static selected by the switch (DXT1/3/5
/// by their FOURCC, 00B26AA8 for any other), then +0xC = format, +0x10 = 0. Takes the raw D3DFORMAT
/// value; no device is involved. Completion: 100%, table from the exact executable bytes.
#[must_use]
pub fn pixel_format_from_d3d(format: u32) -> PixelFormat {
    let source = D3D_PIXEL_FORMAT_CASES
        .iter()
        .find(|case| case.0 == format)
        .map_or(DEFAULT_D3D_PIXEL_FORMAT_STATIC, |case| case.1);
    let mut result = d3d_pixel_format_static(source).expect("every case names a static");
    result.renderer_hint_0c = format;
    result.extra_10 = 0;
    result
}

/// IDA: 0070E560 NiPixelData constructor (0x70 bytes, vtable A7E624): NiObject base, +0x08 NiPixelFormat copy
/// (0x44), +0x4C palette = null, +0x60 levels, +0x6C faces, +0x64 bytes per pixel, per-level widths, heights
/// and offsets, 00732280 AllocatePixels, +0x68 revision = 1. Faces are addressed at face * offsets[levels]
/// (007207F0, 00760938, 0071EB3A). Completion: 100% of the layout values.
/// TODO: [ ] Bind the palette (+0x4C) when a palettized format is reached.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct PixelData {
    pub original_vtable: u32,
    pub format_08: PixelFormat,
    pub levels_60: u32,
    pub faces_6c: u32,
    pub bytes_per_pixel_64: u32,
    pub revision_68: u32,
    /// `(offsets[levels] + 3) & !3`.
    pub allocation_stride: u32,
    pub widths_54: Vec<u32>,
    pub heights_58: Vec<u32>,
    /// `levels + 1` entries.
    pub offsets_5c: Vec<u32>,
    pub pixels_50: Vec<u8>,
}

impl PixelData {
    // IDA: N/A a helper, with no address of its own.
    fn level_range(&self, face: u32, index: u32) -> std::ops::Range<usize> {
        let stride = *self.offsets_5c.last().expect("levels + 1 offsets") as usize;
        let start = face as usize * stride + self.offsets_5c[index as usize] as usize;
        let length =
            self.offsets_5c[index as usize + 1].wrapping_sub(self.offsets_5c[index as usize]);
        start..start + length as usize
    }

    /// IDA: N/A level `index` of face `face`: pixels + face * offsets[levels] + offsets[index].
    /// Completion: 100% of the original addressing.
    #[must_use]
    pub fn level(&self, face: u32, index: u32) -> &[u8] {
        &self.pixels_50[self.level_range(face, index)]
    }

    /// IDA: N/A as `level`, writable.
    #[must_use]
    pub fn level_mut(&mut self, face: u32, index: u32) -> &mut [u8] {
        let range = self.level_range(face, index);
        &mut self.pixels_50[range]
    }
}

/// IDA: 0070E2F0 NiPixelData_ComputeMipmapLevels(w, h): 0 for a zero side, else max(log2 w, log2 h) + 1, by
/// shift counts (0070E2F4..0070E332). Completion: 100%.
#[must_use]
pub const fn pixel_data_mipmap_levels(width: u32, height: u32) -> u32 {
    if width == 0 || height == 0 {
        return 0;
    }
    let mut width_levels = 1;
    let mut value = width >> 1;
    while value != 0 {
        width_levels += 1;
        value >>= 1;
    }
    let mut height_levels = 1;
    let mut value = height >> 1;
    while value != 0 {
        height_levels += 1;
        value >>= 1;
    }
    if height_levels < width_levels {
        width_levels
    } else {
        height_levels
    }
}

/// IDA: 0070E560 + 00732280. Compressed formats 4..6 have +0x64 = 0 and
/// ((w+3)&~3)*((h+3)&~3)/16 blocks of 8 bytes for format 4, 16 otherwise; zero-bit formats w*h/2. The face
/// stride is rounded up to 4 for the allocation. Completion: 100%. Host substitution: the pixel bytes the
/// original leaves unset are zero-filled.
#[must_use]
pub fn construct_pixel_data(
    width: u32,
    height: u32,
    format: &PixelFormat,
    mipmap_levels: u32,
    faces: u32,
) -> PixelData {
    // 0070E5AD..0070E5CC: the format copy; 0070E5E4..0070E5FA levels and faces.
    let levels = if mipmap_levels != 0 {
        mipmap_levels
    } else {
        pixel_data_mipmap_levels(width, height)
    };
    let mut widths = vec![0; levels as usize];
    let mut heights = vec![0; levels as usize];
    let mut offsets = vec![0u32; levels as usize + 1];
    let mut level_width = width;
    let mut level_height = height;
    let compressed = format.format_04 >= 4 && format.format_04 <= 6;
    let bytes_per_pixel = if compressed {
        0
    } else {
        u32::from(format.bits_per_pixel_01) >> 3
    };
    // 0070E6BC..0070E720: 8-byte blocks for format 4, 16 otherwise.
    let block = if format.format_04 == 4 { 8u32 } else { 16 };
    for index in 0..levels as usize {
        let w = if level_width != 0 { level_width } else { 1 };
        let h = if level_height != 0 { level_height } else { 1 };
        widths[index] = w;
        heights[index] = h;
        let size = if compressed {
            ((h.wrapping_add(3) & !3).wrapping_mul(w.wrapping_add(3) & !3) >> 4).wrapping_mul(block)
        } else if bytes_per_pixel != 0 {
            // 0070E64F.
            bytes_per_pixel.wrapping_mul(h).wrapping_mul(w)
        } else {
            // 0070E6A2: zero-bit formats.
            h.wrapping_mul(w) >> 1
        };
        offsets[index + 1] = offsets[index].wrapping_add(size);
        level_width >>= 1;
        level_height >>= 1;
    }
    // 00732280: face stride rounded up to 4 bytes.
    let allocation_stride = offsets[levels as usize].wrapping_add(3) & !3;
    PixelData {
        original_vtable: 0x00A7_E624,
        format_08: *format,
        levels_60: levels,
        faces_6c: faces,
        bytes_per_pixel_64: bytes_per_pixel,
        revision_68: 1, // 0070E77E
        allocation_stride,
        widths_54: widths,
        heights_58: heights,
        offsets_5c: offsets,
        pixels_50: vec![0; allocation_stride as usize * faces as usize],
    }
}

/// IDA: N/A spawns a pixel data object, the owner a C++ `shared_ptr<PixelData>` pointed at.
pub fn spawn_pixel_data(world: &mut World, data: PixelData) -> Entity {
    world.spawn((NiClass::PixelData, data)).id()
}

/// IDA: N/A FNV-1a of the used pixel bytes of every face and level (fixtures and the P05-RUN-002
/// comparison). Completion: 100% host checksum.
#[must_use]
pub fn pixel_data_fnv1a(data: &PixelData) -> u32 {
    let mut hash = 0x811C_9DC5u32;
    for face in 0..data.faces_6c {
        for level in 0..data.levels_60 {
            for &byte in data.level(face, level) {
                hash ^= u32::from(byte);
                hash = hash.wrapping_mul(0x0100_0193);
            }
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    const D3DFMT_A8R8G8B8: u32 = 21;
    const D3DFMT_D24S8: u32 = 75;
    const D3DFMT_R16F: u32 = 111;
    const D3DFMT_DXT1: u32 = 0x3154_5844;
    const D3DFMT_DXT5: u32 = 0x3554_5844;

    /// Ported from `verify_d3d_pixel_formats` in `tests/startup/phase05_tests.cpp`.
    #[test]
    fn d3d_pixel_formats() {
        let depth = pixel_format_from_d3d(D3DFMT_D24S8);
        assert_eq!(depth.format_04, 0x0F);
        assert_eq!(depth.bits_per_pixel_01, 32);
        assert_eq!(depth.renderer_hint_0c, D3DFMT_D24S8);
        assert_eq!(depth.extra_10, 0);
        let argb = pixel_format_from_d3d(D3DFMT_A8R8G8B8);
        assert_eq!(argb.bits_per_pixel_01, 32);
        assert_eq!(argb.renderer_hint_0c, 21);
        let dxt1 = pixel_format_from_d3d(D3DFMT_DXT1);
        let dxt5 = pixel_format_from_d3d(D3DFMT_DXT5);
        assert_ne!(dxt1.source_address, dxt5.source_address);
        assert_eq!(dxt1.renderer_hint_0c, 0x3154_5844);
        let unknown = pixel_format_from_d3d(0x1234_5678);
        assert_eq!(unknown.source_address, 0x00B2_6AA8);
        assert_eq!(unknown.renderer_hint_0c, 0x1234_5678);
        assert_ne!(
            pixel_format_from_d3d(D3DFMT_R16F).source_address,
            0x00B2_6AA8
        );
        assert!(d3d_pixel_format_static(0x1234).is_none());
    }

    /// Ported from `verify_pixel_data` in `tests/startup/phase05_tests.cpp`. The `effect_*_format` helpers
    /// are not ported; these are the statics they name (the RGB24 row carries `original_rgb24_pixel_format`'s
    /// 24-bit layout).
    #[test]
    fn pixel_data_layout() {
        assert_eq!(pixel_data_mipmap_levels(128, 128), 8);
        assert_eq!(pixel_data_mipmap_levels(256, 16), 9);
        assert_eq!(pixel_data_mipmap_levels(0, 4), 0);

        let bgra32 = d3d_pixel_format_static(0x00B2_65E0).unwrap();
        let bgra = construct_pixel_data(128, 128, &bgra32, 1, 1);
        assert_eq!(bgra.bytes_per_pixel_64, 4);
        assert_eq!(bgra.levels_60, 1);
        assert_eq!(bgra.offsets_5c, [0, 65536]);
        assert_eq!(bgra.allocation_stride, 65536);
        assert_eq!(bgra.revision_68, 1);
        assert_eq!(bgra.original_vtable, 0x00A7_E624);

        let rgb24 = d3d_pixel_format_static(0x00B2_6598).unwrap();
        let rgb = construct_pixel_data(16, 16, &rgb24, 1, 1);
        assert_eq!(rgb.bytes_per_pixel_64, 3);
        assert_eq!(rgb.offsets_5c[1], 768);
        assert_eq!(rgb.allocation_stride, 768);

        let dxt1 = construct_pixel_data(64, 64, &pixel_format_from_d3d(D3DFMT_DXT1), 0, 1);
        assert_eq!(dxt1.levels_60, 7);
        assert_eq!(dxt1.bytes_per_pixel_64, 0);
        assert_eq!(dxt1.offsets_5c[1], 2048);
        assert_eq!(dxt1.widths_54[6], 1);
        assert_eq!(dxt1.offsets_5c[7] - dxt1.offsets_5c[6], 8);

        let rgba32 = d3d_pixel_format_static(0x00B2_5E00).unwrap();
        let cube = construct_pixel_data(4, 4, &rgba32, 1, 6);
        assert_eq!(cube.faces_6c, 6);
        assert_eq!(cube.pixels_50.len(), 6 * 64);
        assert_eq!(cube.level(5, 0).len(), 64);
    }
}
