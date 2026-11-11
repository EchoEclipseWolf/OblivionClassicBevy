//! The generated FaceGen face texture (D13, Build 5 goal 4): `00557E60` BSFaceGenModel_GenerateTexture sums an EGT's
//! symmetric texture morphs, weighted by the face's symmetric texture coefficients, into a BSFaceGenImage of float RGBA
//! pixels (`0054E2B0`, `0054E390`, `0054E230`), and `0054DA40` BSFaceGenImage_CreateTexture turns it into an RGBA8
//! NiPixelData (the delta from the base map, 127 meaning none) that becomes the face's FaceGen texture. `00551A00` finds
//! the age's base map beside the face's texture. Evidence: `evidence/ida-names-gameplay-2026-11-07/` (batch 69).

use super::face_gen_files::EgtData;
use crate::tes::facegen::FaceGenMatrix;
use oblivion_core::x87;

/// IDA: 00557E60's morph cap with no count given (`a4` 0): the EGT's symmetric count, at most 30.
pub const EGT_MORPH_CAP: usize = 30;

/// IDA: N/A the generated texture handed to the host: `0054DA40`'s NiPixelData (`g_defaultDescriptorSource68`, RGBA8), the
/// rows in the BSFaceGenImage's order (the EGT file's last row first).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FaceGenTextureImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// IDA: 00557E60 BSFaceGenModel_GenerateTexture's sum (from `005582??` after the model, coefficient and basis checks): the
/// image's width and height from the first symmetric morph (`00556320`(0) +0x04, +0x08); a zeroed i32 accumulator of three
/// per pixel (FormHeap 12 x w x h); for each of the first `count` morphs (0: the EGT's symmetric count capped at 30, else at
/// most that count) with `ftol`(the coefficient's row i, column 0 x 256.0) nonzero: per channel, per row, per column, the
/// accumulator += that x `ftol`(scale x 256.0) x the channel's signed byte (`movsx`, `imul`); then each channel `fild` x
/// 1/65536 (dbl_A64AA0) into the image's floats, alpha 0.0. Answers None where the original returns 0 before the sum (no
/// EGT, no first morph, a zero size, or coefficients with fewer rows than the morphs: the "did not match the EGT basis"
/// warning, pushed). Completion: 100% of the arithmetic; the model's +0x18 spin flag, `00551960`'s model map touch and the
/// texture slot's reference counts have no host form.
#[must_use]
pub fn generate_face_gen_image(
    egt: &EgtData,
    coefficients: &[FaceGenMatrix; 4],
    count: usize,
    warnings: &mut Vec<String>,
) -> Option<(u32, u32, Vec<[f32; 4]>)> {
    // 00557FA0..00557FD9: the symmetric (k 2) and asymmetric (k 3) coefficients against the EGT's two morph lists.
    for (matrix, morphs) in [
        (&coefficients[2], &egt.symmetric_04),
        (&coefficients[3], &egt.asymmetric_14),
    ] {
        if !morphs.is_empty() && (matrix.rows as usize) < morphs.len() {
            warnings.push(
                "FaceGen - Tried to apply a coordinate that did not match the EGT basis."
                    .to_owned(),
            );
            return None;
        }
    }
    let first = egt.symmetric_04.first()?;
    let (width, height) = (first.columns_04, first.rows_08);
    if width == 0 || height == 0 {
        return None;
    }
    let pixels = (width * height) as usize;
    let mut sums = vec![[0_i32; 3]; pixels];
    let available = egt.symmetric_04.len();
    let limit = if count == 0 {
        available.min(EGT_MORPH_CAP)
    } else {
        count.min(available)
    };
    let symmetric = &coefficients[2];
    for (index, morph) in egt.symmetric_04.iter().take(limit).enumerate() {
        // 005581D6..005581F0: the coefficient (row index, column 0) x 256.0, `__ftol2_sse`.
        let coefficient = symmetric.data[index * symmetric.cols as usize];
        let weight = x87::ftol(coefficient * 256.0);
        if weight == 0 {
            continue;
        }
        // 0055826E..0055828A: the scale x 256.0, `__ftol2_sse`, times the weight.
        let factor = weight.wrapping_mul(x87::ftol(morph.scale_00 * 256.0));
        let stride = (morph.columns_04 + morph.padding_0c) as usize;
        for (channel, data) in morph.channels_10.iter().enumerate() {
            for row in 0..height as usize {
                let source = &data[row * stride..row * stride + width as usize];
                let target = &mut sums[row * width as usize..(row + 1) * width as usize];
                for (sum, &byte) in target.iter_mut().zip(source) {
                    sum[channel] = sum[channel].wrapping_add(i32::from(byte).wrapping_mul(factor));
                }
            }
        }
    }
    // 00558351..0055845A: `fild` x 1/65536, `fstp` to the image's float channels.
    let image = sums
        .into_iter()
        .map(|sum| {
            let scale = |value: i32| (f64::from(value) * 0.000_015_258_789_062_5) as f32;
            [scale(sum[0]), scale(sum[1]), scale(sum[2]), 0.0]
        })
        .collect();
    Some((width, height, image))
}

/// IDA: 0054DA40 BSFaceGenImage_CreateTexture(low, high, scale, prefs) (thiscall image), as 00557E60 calls it (-255.0,
/// 255.0, 0.5, 00B26040): with no texture yet and pixels, a w x h NiPixelData (`g_defaultDescriptorSource68`); per pixel,
/// per channel (red, green, blue, alpha) the float clamped to [low, high] (below low or unordered: low; above high:
/// high), stored as a float, `00404F70` Float_FloorByFtol, less low, times scale, `fistp` under chop, its low byte.
/// Completion: 100% of the bytes; the image converter (byte_B11F8C's `0071B8D0`), byte_B05244's `0047F590`,
/// NiSourceTexture_CreateFromPixelData with g_faceGenTextureFormatPrefs and `00480000`'s format conversion are the
/// renderer's (the host uploads the bytes).
#[must_use]
pub fn face_gen_image_bytes(
    width: u32,
    height: u32,
    image: &[[f32; 4]],
    low: f32,
    high: f32,
    scale: f32,
) -> FaceGenTextureImage {
    let byte = |value: f32| {
        // 0054DB99..0054DBB9: `fcom` low (C0|C3 both clear, or both set: low), then high (C0: high).
        let clamped = if low > value || value.is_nan() {
            low
        } else if high < value {
            high
        } else {
            value
        };
        let floored = x87::floor_by_ftol(clamped);
        // 0054DBCA..0054DBF5: (floor - low) x scale, `fistp` with RC chop, the low byte.
        let product = (f64::from(floored) - f64::from(low)) * f64::from(scale);
        (product.trunc() as i64) as u8
    };
    let rgba = image.iter().flat_map(|pixel| pixel.map(byte)).collect();
    FaceGenTextureImage {
        width,
        height,
        rgba,
    }
}

/// IDA: 00557E60 then 0054DA40 as `005551C0` runs them for a part: the generated texture, or None where 00557E60 returns 0
/// (the slot then keeps `00523D80`'s grey texture).
#[must_use]
pub fn generate_face_gen_texture(
    egt: &EgtData,
    coefficients: &[FaceGenMatrix; 4],
    warnings: &mut Vec<String>,
) -> Option<FaceGenTextureImage> {
    let (width, height, image) = generate_face_gen_image(egt, coefficients, 0, warnings)?;
    Some(face_gen_image_bytes(
        width, height, &image, -255.0, 255.0, 0.5,
    ))
}

/// IDA: 00551A00 FaceGen_FindAgeTexture(out, sex, age, texture) (cdecl): none for a null texture, an age past 100 or a sex
/// of 2 or more; the texture's path up to its last '.' (none without one); 'M', or 'F' for sex 1;
/// "Textures\%s%c%d.dds" with the age, and when FileFinder doesn't find it, the same with the age's decade, then each decade
/// below, down to 0; the found path, else none (the output emptied). `exists` stands for g_FileFinder's FindFile(path, 0,
/// 0, -1). Completion: 100%.
pub fn find_age_texture(
    sex: i32,
    age: u8,
    texture: &[u8],
    exists: &mut dyn FnMut(&str) -> bool,
) -> Option<String> {
    if age > 100 || sex >= 2 {
        return None;
    }
    let text = String::from_utf8_lossy(texture);
    let stem = &text[..text.rfind('.')?];
    let letter = if sex == 1 { 'F' } else { 'M' };
    let path = format!("Textures\\{stem}{letter}{age}.dds");
    if exists(&path) {
        return Some(path);
    }
    let mut decade = i32::from(age / 10) * 10;
    while decade >= 0 {
        let path = format!("Textures\\{stem}{letter}{decade}.dds");
        if exists(&path) {
            return Some(path);
        }
        decade -= 10;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bs::face_gen_files::EgtMorph;

    fn matrix(values: &[f32]) -> FaceGenMatrix {
        FaceGenMatrix {
            rows: values.len() as u32,
            cols: 1,
            data: values.to_vec(),
        }
    }

    #[test]
    fn the_sum_is_integer_and_the_bytes_centre_on_127() {
        // One 2x1 morph, scale 0.5 (ftol 128), coefficient 1.0 (ftol 256): 256 x 128 x byte / 65536 = byte / 2.
        let morph = EgtMorph {
            scale_00: 0.5,
            columns_04: 2,
            rows_08: 1,
            padding_0c: 6,
            channels_10: [
                vec![10, -10, 0, 0, 0, 0, 0, 0],
                vec![0; 8],
                vec![127, -128, 0, 0, 0, 0, 0, 0],
            ],
        };
        let egt = EgtData {
            basis_00: 0,
            symmetric_04: vec![morph],
            asymmetric_14: Vec::new(),
        };
        let coefficients = [matrix(&[0.0]), matrix(&[]), matrix(&[1.0]), matrix(&[])];
        let mut warnings = Vec::new();
        let (w, h, image) = generate_face_gen_image(&egt, &coefficients, 0, &mut warnings).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(image[0], [5.0, 0.0, 63.5, 0.0]);
        assert_eq!(image[1], [-5.0, 0.0, -64.0, 0.0]);
        let bytes = face_gen_image_bytes(w, h, &image, -255.0, 255.0, 0.5);
        // (floor(v) + 255) x 0.5, chopped: 5 -> 130, 0 -> 127, 63.5 -> 159, -5 -> 125, -64 -> 95.
        assert_eq!(bytes.rgba, vec![130, 127, 159, 127, 125, 127, 95, 127]);
    }

    #[test]
    fn the_age_texture_falls_back_by_decades() {
        let present = ["Textures\\Characters\\Imperial\\HeadHumanM30.dds"];
        let mut exists = |path: &str| present.contains(&path);
        assert_eq!(
            find_age_texture(0, 37, b"Characters\\Imperial\\HeadHuman.dds", &mut exists).as_deref(),
            Some("Textures\\Characters\\Imperial\\HeadHumanM30.dds")
        );
        assert_eq!(
            find_age_texture(1, 37, b"Characters\\Imperial\\HeadHuman.dds", &mut exists),
            None
        );
        assert_eq!(find_age_texture(0, 101, b"x.dds", &mut exists), None);
        assert_eq!(find_age_texture(0, 30, b"noextension", &mut exists), None);
    }
}
