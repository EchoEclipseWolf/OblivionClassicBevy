//! IDA: N/A the host's reading of a DDS file's pixels for `screens`: what the device would hold after D3DX's load. DXT1, DXT3, DXT5 and the 32-bit, 24-bit and 16-bit RGB forms the menus' files use, every level in the file, as 8-bit RGBA. A recorded substitution for D3DX and the device's formats: the blocks decode the way D3D9's fixed function does, and wgpu never sees a compressed format, so no adapter feature matters.

/// IDA: N/A one decoded file: its levels, largest first, each `width * height * 4` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Vec<u8>>,
}

// IDA: N/A a helper, with no address of its own.
fn word(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

// IDA: N/A the DXT colour endpoints' 5:6:5 to 8 bits, by the usual replication.
fn expand565(color: u16) -> [u8; 3] {
    let red = (color >> 11) as u8 & 0x1F;
    let green = (color >> 5) as u8 & 0x3F;
    let blue = color as u8 & 0x1F;
    [
        (red << 3) | (red >> 2),
        (green << 2) | (green >> 4),
        (blue << 3) | (blue >> 2),
    ]
}

// IDA: N/A a colour block: four palette entries, the three-colour form for DXT1 when c0 <= c1.
fn decode_color_block(block: &[u8], dxt1: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let (e0, e1) = (expand565(c0), expand565(c1));
    let mut palette = [[0u8; 4]; 4];
    palette[0] = [e0[0], e0[1], e0[2], 255];
    palette[1] = [e1[0], e1[1], e1[2], 255];
    if !dxt1 || c0 > c1 {
        for channel in 0..3 {
            let (a, b) = (u16::from(e0[channel]), u16::from(e1[channel]));
            palette[2][channel] = ((2 * a + b + 1) / 3) as u8;
            palette[3][channel] = ((a + 2 * b + 1) / 3) as u8;
        }
        palette[2][3] = 255;
        palette[3][3] = 255;
    } else {
        for channel in 0..3 {
            palette[2][channel] = ((u16::from(e0[channel]) + u16::from(e1[channel])) / 2) as u8;
        }
        palette[2][3] = 255;
        palette[3] = [0, 0, 0, 0];
    }
    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    std::array::from_fn(|texel| palette[(indices >> (2 * texel)) as usize & 3])
}

// IDA: N/A a helper, with no address of its own.
fn decode_blocks(
    bytes: &[u8],
    width: u32,
    height: u32,
    four_cc: &[u8; 4],
) -> Option<(Vec<u8>, usize)> {
    let block_size = if four_cc == b"DXT1" { 8 } else { 16 };
    let (columns, rows) = (width.div_ceil(4) as usize, height.div_ceil(4) as usize);
    let total = columns * rows * block_size;
    let data = bytes.get(..total)?;
    let mut out = vec![0u8; width as usize * height as usize * 4];
    for row in 0..rows {
        for column in 0..columns {
            let block = &data[(row * columns + column) * block_size..][..block_size];
            let (alpha_block, color_block) = if block_size == 16 {
                block.split_at(8)
            } else {
                (&block[..0], block)
            };
            let mut texels = decode_color_block(color_block, block_size == 8);
            if four_cc == b"DXT3" {
                for (texel, value) in texels.iter_mut().enumerate() {
                    let nibble = (alpha_block[texel / 2] >> (4 * (texel % 2))) & 0xF;
                    value.alpha_set((nibble << 4) | nibble);
                }
            } else if four_cc == b"DXT5" {
                let (a0, a1) = (u32::from(alpha_block[0]), u32::from(alpha_block[1]));
                let mut bits = 0u64;
                for (index, &byte) in alpha_block[2..8].iter().enumerate() {
                    bits |= u64::from(byte) << (8 * index);
                }
                for (texel, value) in texels.iter_mut().enumerate() {
                    let code = (bits >> (3 * texel)) as u32 & 7;
                    let alpha = match code {
                        0 => a0,
                        1 => a1,
                        _ if a0 > a1 => ((8 - code) * a0 + (code - 1) * a1 + 3) / 7,
                        6 => 0,
                        7 => 255,
                        _ => ((6 - code) * a0 + (code - 1) * a1 + 2) / 5,
                    };
                    value.alpha_set(alpha as u8);
                }
            }
            for y in 0..4 {
                for x in 0..4 {
                    let (px, py) = (column * 4 + x, row * 4 + y);
                    if px < width as usize && py < height as usize {
                        let at = (py * width as usize + px) * 4;
                        out[at..at + 4].copy_from_slice(&texels[y * 4 + x]);
                    }
                }
            }
        }
    }
    Some((out, total))
}

trait AlphaSet {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn alpha_set(&mut self, value: u8);
}

impl AlphaSet for [u8; 4] {
    /// IDA: N/A a helper, with no address of its own.
    fn alpha_set(&mut self, value: u8) {
        self[3] = value;
    }
}

// IDA: N/A one level of the masked RGB forms.
fn decode_masked(
    bytes: &[u8],
    width: u32,
    height: u32,
    bits: u32,
    masks: [u32; 4],
) -> Option<(Vec<u8>, usize)> {
    let stride = bits as usize / 8;
    let total = width as usize * height as usize * stride;
    let data = bytes.get(..total)?;
    let channel = |value: u32, mask: u32, default: u8| -> u8 {
        if mask == 0 {
            return default;
        }
        let shift = mask.trailing_zeros();
        let span = (mask >> shift) as u64;
        (u64::from((value & mask) >> shift) * 255 / span) as u8
    };
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for texel in data.chunks_exact(stride) {
        let mut value = 0u32;
        for (index, &byte) in texel.iter().enumerate() {
            value |= u32::from(byte) << (8 * index);
        }
        out.push(channel(value, masks[0], 0));
        out.push(channel(value, masks[1], 0));
        out.push(channel(value, masks[2], 0));
        out.push(channel(value, masks[3], 255));
    }
    Some((out, total))
}

/// IDA: N/A a DDS file's levels as RGBA. `None` for a form the menus don't use, which `screens` reports. Completion: 100% of the forms named above.
#[must_use]
pub fn decode_dds(bytes: &[u8]) -> Option<DecodedImage> {
    if bytes.get(..4)? != b"DDS " {
        return None;
    }
    let height = word(bytes, 12)?;
    let width = word(bytes, 16)?;
    let mips = word(bytes, 28)?.max(1);
    let flags = word(bytes, 80)?;
    let mut rest = bytes.get(128..)?;
    let mut levels = Vec::new();
    let (mut level_width, mut level_height) = (width, height);
    for _ in 0..mips {
        let (pixels, used) = if flags & 4 != 0 {
            let four_cc: [u8; 4] = bytes.get(84..88)?.try_into().ok()?;
            if !matches!(&four_cc, b"DXT1" | b"DXT3" | b"DXT5") {
                return None;
            }
            decode_blocks(rest, level_width, level_height, &four_cc)?
        } else {
            let bits = word(bytes, 88)?;
            // 007362C0: only these are RGB.
            if !matches!(bits, 16 | 24 | 32) {
                return None;
            }
            let alpha = if flags & 1 != 0 { word(bytes, 104)? } else { 0 };
            decode_masked(
                rest,
                level_width,
                level_height,
                bits,
                [word(bytes, 92)?, word(bytes, 96)?, word(bytes, 100)?, alpha],
            )?
        };
        levels.push(pixels);
        rest = &rest[used..];
        level_width = (level_width / 2).max(1);
        level_height = (level_height / 2).max(1);
    }
    Some(DecodedImage {
        width,
        height,
        levels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(width: u32, height: u32, mips: u32, flags: u32, four_cc: &[u8; 4]) -> Vec<u8> {
        let mut bytes = vec![0u8; 128];
        bytes[..4].copy_from_slice(b"DDS ");
        bytes[12..16].copy_from_slice(&height.to_le_bytes());
        bytes[16..20].copy_from_slice(&width.to_le_bytes());
        bytes[28..32].copy_from_slice(&mips.to_le_bytes());
        bytes[80..84].copy_from_slice(&flags.to_le_bytes());
        bytes[84..88].copy_from_slice(four_cc);
        bytes
    }

    #[test]
    fn dxt1_three_colour_form_has_transparent_black() {
        let mut bytes = header(4, 4, 1, 4, b"DXT1");
        // c0 = black, c1 = white, c0 <= c1: index 3 is transparent.
        bytes.extend([0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        let image = decode_dds(&bytes).unwrap();
        assert_eq!(image.levels[0][..4], [0, 0, 0, 0]);
    }

    #[test]
    fn dxt5_alpha_interpolates() {
        let mut bytes = header(4, 4, 1, 4, b"DXT5");
        // a0 = 255, a1 = 0, all codes 2.
        bytes.extend([255, 0]);
        let codes: u64 = (0..16).fold(0, |bits, texel| bits | (2u64 << (3 * texel)));
        bytes.extend(&codes.to_le_bytes()[..6]);
        bytes.extend([0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0]);
        let image = decode_dds(&bytes).unwrap();
        assert_eq!(image.levels[0][..4], [255, 255, 255, 219]);
    }

    #[test]
    fn a8r8g8b8_swizzles_and_reads_each_level() {
        let mut bytes = header(2, 1, 2, 0x41, &[0; 4]);
        bytes[88..92].copy_from_slice(&32u32.to_le_bytes());
        bytes[92..96].copy_from_slice(&0x00FF_0000u32.to_le_bytes());
        bytes[96..100].copy_from_slice(&0x0000_FF00u32.to_le_bytes());
        bytes[100..104].copy_from_slice(&0x0000_00FFu32.to_le_bytes());
        bytes[104..108].copy_from_slice(&0xFF00_0000u32.to_le_bytes());
        bytes.extend([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        let image = decode_dds(&bytes).unwrap();
        assert_eq!(image.levels[0], [3, 2, 1, 4, 7, 6, 5, 8]);
        assert_eq!(image.levels[1], [11, 10, 9, 12]);
    }

    // 007362C0's own check: only 16, 24 and 32 bits are RGB. The header's other counts are a form the menus don't use, not a panic.
    #[test]
    fn an_odd_bit_count_is_refused() {
        for bits in [0u32, 7, 8, 64] {
            let mut bytes = header(2, 1, 1, 0x41, &[0; 4]);
            bytes[88..92].copy_from_slice(&bits.to_le_bytes());
            bytes.extend([0u8; 64]);
            assert_eq!(decode_dds(&bytes), None, "{bits} bits");
        }
    }

    // A level count past the file stops at the bytes, and doesn't ask for its size up front.
    #[test]
    fn a_huge_level_count_is_refused() {
        let mut bytes = header(4, 4, u32::MAX, 4, b"DXT1");
        bytes.extend([0u8; 8]);
        assert_eq!(decode_dds(&bytes), None);
    }
}
