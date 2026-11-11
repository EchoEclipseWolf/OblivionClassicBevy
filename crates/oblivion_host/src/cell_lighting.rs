//! IDA: N/A Track R3's lighting of the cell view: Bevy's light entities carrying the game's numbers, shaded with the SM3
//! falloff by `world_lighting.wesl` (user, 2026-11-08: Bevy's lights with the game's falloff). The values come from the exe's
//! sources; carrying them in Bevy's lights is a host substitution recorded in `docs/open-items.md` (Track R3):
//! - the interior cell's lighting block (TESObjectCELL +0x3C, XCLL): ambient, directional and fog colours and the fog
//!   distances, read as the cell getters `004C98C0`, `004C9920`, `004C99C0`, `004C9A20`, `004C9A40`, `004C9980` and
//!   `004C99A0` read it, with the fog distances clamped as the Sky's interior arm (`00541DD0`) does;
//! - each LIGH reference's NiPointLight as `004B1600` made it: diffuse (+0xEC), dimmer (+0xDC, the fade) and radius (+0xF8,
//!   where 004B1600 stores it), which `007EDCD0` hands the SM3 pixel shader as colour `diffuse x min(dimmer, 1)` and
//!   position `.w` = radius;
//! - the SM3 shaders' falloff (`SM3000.pso`: `1 - saturate(d / r)^2` times N.L, no inverse square), which the shader
//!   applies in place of Bevy's.
//!
//! Completion: the cell's ambient, directional light and fog, and its point lights, as the shader's constants.
//! TODO: [ ] Per-object light lists: the original lights a geometry with the lights whose sphere reaches its bound
//! (`007D2EA0`: |light - bound centre| - bound radius < radius), at most 20 (`00B2DCFC`); Bevy's clusters light each
//! fragment within the range, with no cap.
//! TODO: [ ] Flicker and pulse lights (LIGH +0x7C bits 8, 0x40, 0x80) animate the dimmer per frame; the capture holds it.
//! TODO: [ ] A negative light (+0x7C bit 4, negated diffuse) can't be a Bevy light and is left out.

use std::f32::consts::PI;

/// IDA: 00541DD0's interior arm: the fog's far distance with none (or past it) in the cell.
pub const FOG_FAR_LIMIT: f32 = 163_840.0;

/// IDA: 00541DD0's interior arm: the near distance's fraction of the far one when the cell's is unset or past the far one.
pub const FOG_NEAR_FRACTION: f32 = 0.17;

/// IDA: N/A the decoded XCLL block (TESObjectCELL +0x3C, 0x28 bytes) of an interior cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellLighting {
    /// +0x00 (004C98C0): the ambient colour's bytes / 255 (R, G, B).
    pub ambient: [f32; 3],
    /// +0x04 (004C9920): the directional colour, the Sky's sun light diffuse in an interior (005418F0, 00545830).
    pub directional: [f32; 3],
    /// +0x08 (004C99C0): the fog colour.
    pub fog_color: [f32; 3],
    /// +0x0C (004C9A20) and +0x10 (004C9A40) after 00541DD0's clamps: the fog's start and end.
    pub fog_near: f32,
    pub fog_far: f32,
    /// +0x14 (004C9980) and +0x18 (004C99A0): the directional light's rotations in degrees, about Z then Y (00545830).
    pub rotation_xy: i32,
    pub rotation_z: i32,
    /// +0x20 (004C9A60): the fog clip distance, which SceneGraph_GetFarPlaneDistance uses with no LOD land.
    pub fog_clip: f32,
}

// IDA: 004C98C0 / 004C9920 / 004C99C0: the low three bytes of a colour word over 255.0 (`movzx al`, `ah`, `shr 10h`).
fn color_bytes(word: u32) -> [f32; 3] {
    let bytes = word.to_le_bytes();
    [
        f32::from(bytes[0]) / 255.0,
        f32::from(bytes[1]) / 255.0,
        f32::from(bytes[2]) / 255.0,
    ]
}

/// IDA: the cell getters `004C98C0`, `004C9920`, `004C99C0`, `004C9A20`, `004C9A40`, `004C9980`, `004C99A0` and `004C9A60`
/// over an interior's block, and `00541DD0`'s interior fog distances: far is the cell's, or 163840 when it is 0 or less or
/// past 163840; near is the cell's, or 0.17 x far when it is 0 or less or past far. Word 7 (+0x1C) isn't read here.
/// Completion: 100% of those reads; `00541DD0`'s re-fit against fNearDistance and the far plane runs only with no LOD land
/// (00B3608F clear), which the pinned INI's bDisplayLODLand=1 doesn't take.
#[must_use]
pub fn decode_cell_lighting(block: &[u32; 10]) -> CellLighting {
    let raw_far = f32::from_bits(block[4]);
    let fog_far = if raw_far <= 0.0 || raw_far > FOG_FAR_LIMIT {
        FOG_FAR_LIMIT
    } else {
        raw_far
    };
    let raw_near = f32::from_bits(block[3]);
    let fog_near = if raw_near <= 0.0 || fog_far < raw_near {
        FOG_NEAR_FRACTION * fog_far
    } else {
        raw_near
    };
    CellLighting {
        ambient: color_bytes(block[0]),
        directional: color_bytes(block[1]),
        fog_color: color_bytes(block[2]),
        fog_near,
        fog_far,
        rotation_xy: block[5] as i32,
        rotation_z: block[6] as i32,
        fog_clip: f32::from_bits(block[8]),
    }
}

/// IDA: 00545830's interior arm: the sun light's rotation is 0070FDD0(xy degrees x 0.0174533) (about Z) times
/// NiMatrix33_MakeYRotation(z degrees x 0.0174533); a NiDirectionalLight shines along its rotation's first column. Returns
/// that column in Gamebryo's axes. Completion: the matrix as read; the product's order (NiMatrix33_Multiply's this x other)
/// and the first-column convention aren't checked against a capture.
#[must_use]
pub fn directional_direction(lighting: &CellLighting) -> [f32; 3] {
    let xy = lighting.rotation_xy as f32 * 0.017_453_292;
    let z = lighting.rotation_z as f32 * 0.017_453_292;
    // Rz = [c s 0; -s c 0; 0 0 1], Ry = [c 0 -s; 0 1 0; s 0 c], row-major; (Rz Ry) e0 = Rz (cos z, 0, sin z).
    [xy.cos() * z.cos(), -xy.sin() * z.cos(), z.sin()]
}

/// IDA: N/A the EV100 whose Bevy exposure (`exp2(-ev100) / 1.2`) is 1.0, so light values are plain multipliers.
#[must_use]
pub fn unit_exposure_ev100() -> f32 {
    -(1.2f32).log2()
}

/// IDA: N/A the point light intensity that carries a colour unchanged: Bevy divides a point light's lumens by 4 pi into
/// candela before it packs `colour x intensity` for the shader.
pub const POINT_LIGHT_UNIT: f32 = 4.0 * PI;

#[cfg(test)]
mod tests {
    use super::*;

    // The block's words as an interior's XCLL would hold them.
    fn block(near: f32, far: f32) -> [u32; 10] {
        [
            0x0020_1810,
            0x0000_0000,
            0x0030_2010,
            near.to_bits(),
            far.to_bits(),
            90,
            45,
            1.0f32.to_bits(),
            3000.0f32.to_bits(),
            0,
        ]
    }

    #[test]
    fn colours_take_the_low_three_bytes_red_first() {
        let lighting = decode_cell_lighting(&block(100.0, 2000.0));
        assert_eq!(lighting.ambient, [16.0 / 255.0, 24.0 / 255.0, 32.0 / 255.0]);
        assert_eq!(lighting.directional, [0.0; 3]);
        assert_eq!(
            lighting.fog_color,
            [16.0 / 255.0, 32.0 / 255.0, 48.0 / 255.0]
        );
        assert_eq!((lighting.rotation_xy, lighting.rotation_z), (90, 45));
        assert_eq!(lighting.fog_clip, 3000.0);
    }

    // 00541DD0: far 0 or past 163840 becomes 163840; near 0 or past far becomes 0.17 x far.
    #[test]
    fn fog_distances_take_the_sky_clamps() {
        let set = decode_cell_lighting(&block(100.0, 2000.0));
        assert_eq!((set.fog_near, set.fog_far), (100.0, 2000.0));
        let none = decode_cell_lighting(&block(0.0, 0.0));
        assert_eq!(none.fog_far, FOG_FAR_LIMIT);
        assert_eq!(none.fog_near, FOG_NEAR_FRACTION * FOG_FAR_LIMIT);
        let past = decode_cell_lighting(&block(3000.0, 2000.0));
        assert_eq!(past.fog_near, FOG_NEAR_FRACTION * 2000.0);
        let huge = decode_cell_lighting(&block(10.0, 200_000.0));
        assert_eq!(huge.fog_far, FOG_FAR_LIMIT);
    }

    #[test]
    fn exposure_is_one() {
        let ev100 = unit_exposure_ev100();
        assert!((2f32.powf(-ev100) / 1.2 - 1.0).abs() < 1e-6);
    }

    // 00545830 with no rotation: the light shines along +X; 90 degrees about Z turns it to -Y; 90 about Y to +Z.
    #[test]
    fn directional_rotations() {
        let mut lighting = decode_cell_lighting(&block(1.0, 2.0));
        (lighting.rotation_xy, lighting.rotation_z) = (0, 0);
        assert_eq!(directional_direction(&lighting), [1.0, 0.0, 0.0]);
        (lighting.rotation_xy, lighting.rotation_z) = (90, 0);
        let d = directional_direction(&lighting);
        assert!(d[0].abs() < 1e-6 && (d[1] + 1.0).abs() < 1e-6);
        (lighting.rotation_xy, lighting.rotation_z) = (0, 90);
        let d = directional_direction(&lighting);
        assert!(d[0].abs() < 1e-6 && (d[2] - 1.0).abs() < 1e-6);
    }
}
