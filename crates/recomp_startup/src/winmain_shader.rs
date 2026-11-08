//! Phase 05 row 05-B: WinMain 0040E898..0040EAA6, the shader wiring and setting copies into the shader system globals
//! (`tes::shader_system`). Every value comes from the 03-E INI collection (the `SettingInfo` static at the quoted
//! address), and every store keeps the original order and width; `ShaderSystemGlobals` records the order.
//! Runtime evidence: P05-RUN-001 (Phase05-Run001-Window-2026-10-06.md).

use recomp_core::x87::fld_fstp_copy;
use recomp_game::shader_store;
use recomp_game::tes::settings::SettingCollection;
use recomp_game::tes::shader_system::{
    ShaderSystemGlobals, blur_shader_apply_parameters, get_hit_shader_apply_parameters,
};

// IDA: 00405150 / 00405290 identities stored at 0040E8C1 / 0040E8E0.
const SHADER_WARNING_CALLBACK_ADDRESS: u32 = 0x0040_5150;
const GAME_TIME_CALLBACK_ADDRESS: u32 = 0x0040_5290;

/// IDA: the WinMain inputs of 0040E898..0040EAA6.
pub struct ShaderWindowInputs<'a> {
    pub ini: &'a SettingCollection, // 00B07BF0 (03-E)
    pub texture_palette: bool, // 00B35300 NiPointer (Phase 03 0040E6AD): the resource's presence
    pub renderer_hdr: bool,    // 00B43070 g_rendererHDR (Phase 04 hdr)
}

// IDA: N/A host view of one SettingInfo value dword (03-E collection).
fn setting_raw(ini: &SettingCollection, address: u32) -> u32 {
    ini.find_address(address).map_or(0, |setting| setting.raw)
}

// IDA: the fld dword / fstp dword pairs of the window.
fn setting_float_copy(ini: &SettingCollection, address: u32) -> f32 {
    fld_fstp_copy(setting_raw(ini, address))
}

// IDA: movzx/mov of the low byte of a bool SettingInfo. Completion: 100%.
fn setting_byte(ini: &SettingCollection, address: u32) -> u8 {
    setting_raw(ini, address) as u8
}

// IDA: N/A a helper, with no address of its own.
fn setting_dword(ini: &SettingCollection, address: u32) -> i32 {
    setting_raw(ini, address) as i32
}

// IDA: WinMain 0040E898..0040EAA6. 0040E89D Startup_LogInfo("Initializing Shader System...") is obsolete (host log);
// then the four stores, 007B4870, the five copies and the 0040E938 jz on g_rendererHDR: the HDR arm's 21 stores, or
// 007B4830 plus four stores. Completion: 100% of the order and widths.
pub fn run_shader_window(globals: &mut ShaderSystemGlobals, inputs: &ShaderWindowInputs<'_>) {
    let ini = inputs.ini;
    // 0040E8C1..0040E8E0.
    shader_store!(
        globals,
        warning_callback,
        SHADER_WARNING_CALLBACK_ADDRESS,
        0x00B4_2E8C,
        4
    );
    shader_store!(
        globals,
        image_space_effects,
        setting_byte(ini, 0x00B0_6CC4),
        0x00B4_2F3E,
        1
    );
    shader_store!(
        globals,
        texture_palette,
        inputs.texture_palette,
        0x00B4_2EBC,
        4
    );
    shader_store!(
        globals,
        game_time_callback,
        GAME_TIME_CALLBACK_ADDRESS,
        0x00B4_2D78,
        4
    );
    // 0040E8A2..0040E8ED: fHitTexOffset/fBlockedTexOffset/fBlurAmmount:GethitShader.
    get_hit_shader_apply_parameters(
        globals,
        setting_float_copy(ini, 0x00B0_6D8C),
        setting_float_copy(ini, 0x00B0_6D94),
        setting_float_copy(ini, 0x00B0_6D9C),
    );
    // 0040E8F2..0040E932.
    shader_store!(
        globals,
        local_tree_mip_bias,
        setting_float_copy(ini, 0x00B0_2D78),
        0x00B4_30A8,
        4
    );
    shader_store!(
        globals,
        lod_tree_mip_bias,
        setting_float_copy(ini, 0x00B0_2D80),
        0x00B4_30A4,
        4
    );
    shader_store!(
        globals,
        use_hard_drive_cache,
        setting_byte(ini, 0x00B0_2D60),
        0x00B4_30AE,
        1
    );
    shader_store!(
        globals,
        display_lod_land,
        setting_byte(ini, 0x00B0_2D70),
        0x00B4_3077,
        1
    );
    shader_store!(
        globals,
        local_map_shader,
        setting_byte(ini, 0x00B0_6D1C),
        0x00B4_2D40,
        1
    );
    // 0040E920 cmp g_rendererHDR, 0 / 0040E938 jz 0040EA3A.
    if inputs.renderer_hdr {
        // 0040E93E..0040EA32, then 0040EA38 jmp 0040EAA6.
        shader_store!(
            globals,
            specular_multiplier,
            setting_float_copy(ini, 0x00B0_6E24),
            0x00B2_C2BC,
            4
        );
        shader_store!(
            globals,
            emissive_multiplier,
            setting_float_copy(ini, 0x00B0_6E2C),
            0x00B2_C2C0,
            4
        );
        shader_store!(
            globals,
            blend_type,
            setting_dword(ini, 0x00B0_6DEC),
            0x00B2_C1E4,
            4
        );
        shader_store!(
            globals,
            blur_passes_hdr,
            setting_dword(ini, 0x00B0_6DF4),
            0x00B4_3220,
            4
        );
        shader_store!(
            globals,
            blur_radius_hdr,
            setting_float_copy(ini, 0x00B0_6DFC),
            0x00B4_31F8,
            4
        );
        shader_store!(
            globals,
            blur_passes_hdr_interior,
            setting_dword(ini, 0x00B0_6E6C),
            0x00B4_3224,
            4
        );
        shader_store!(
            globals,
            blur_radius_hdr_interior,
            setting_float_copy(ini, 0x00B0_6E74),
            0x00B4_31FC,
            4
        );
        shader_store!(
            globals,
            bright_clamp,
            setting_float_copy(ini, 0x00B0_6E04),
            0x00B4_31E8,
            4
        );
        shader_store!(
            globals,
            bright_clamp_interior,
            setting_float_copy(ini, 0x00B0_6E7C),
            0x00B4_31EC,
            4
        );
        shader_store!(
            globals,
            bright_scale,
            setting_float_copy(ini, 0x00B0_6E0C),
            0x00B4_31F0,
            4
        );
        shader_store!(
            globals,
            bright_scale_interior,
            setting_float_copy(ini, 0x00B0_6E84),
            0x00B4_31F4,
            4
        );
        shader_store!(
            globals,
            eye_adapt_speed,
            setting_float_copy(ini, 0x00B0_6E3C),
            0x00B4_3200,
            4
        );
        shader_store!(
            globals,
            eye_adapt_speed_interior,
            setting_float_copy(ini, 0x00B0_6E8C),
            0x00B4_3204,
            4
        );
        shader_store!(
            globals,
            emissive_hdr_mult,
            setting_float_copy(ini, 0x00B0_6E44),
            0x00B4_3208,
            4
        );
        shader_store!(
            globals,
            emissive_hdr_mult_interior,
            setting_float_copy(ini, 0x00B0_6E94),
            0x00B4_320C,
            4
        );
        shader_store!(
            globals,
            upper_lum_clamp,
            setting_float_copy(ini, 0x00B0_6E5C),
            0x00B4_3210,
            4
        );
        shader_store!(
            globals,
            upper_lum_clamp_interior,
            setting_float_copy(ini, 0x00B0_6E9C),
            0x00B4_3214,
            4
        );
        shader_store!(
            globals,
            target_lum,
            setting_float_copy(ini, 0x00B0_6E64),
            0x00B4_3218,
            4
        );
        shader_store!(
            globals,
            target_lum_interior,
            setting_float_copy(ini, 0x00B0_6EA4),
            0x00B4_321C,
            4
        );
        shader_store!(
            globals,
            sun_brightness,
            setting_float_copy(ini, 0x00B0_6E14),
            0x00B4_3154,
            4
        );
        shader_store!(
            globals,
            sky_brightness,
            setting_float_copy(ini, 0x00B0_6E1C),
            0x00B4_3158,
            4
        );
    } else {
        // 0040EA3A..0040EA6E: iBlendType, iNumBlurpasses, fBlurRadius,
        // fAlphaAddInterior, fAlphaAddExterior, iBlurTexSize:BlurShader.
        let _ = blur_shader_apply_parameters(
            globals,
            setting_dword(ini, 0x00B0_6D3C),
            setting_dword(ini, 0x00B0_6D44),
            setting_float_copy(ini, 0x00B0_6D4C),
            setting_float_copy(ini, 0x00B0_6D5C),
            setting_float_copy(ini, 0x00B0_6D64),
            setting_dword(ini, 0x00B0_6D54),
        );
        // 0040EA73..0040EAA0.
        shader_store!(
            globals,
            sun_brightness,
            setting_float_copy(ini, 0x00B0_6D6C),
            0x00B4_3154,
            4
        );
        shader_store!(
            globals,
            sky_brightness,
            setting_float_copy(ini, 0x00B0_6D74),
            0x00B4_3158,
            4
        );
        shader_store!(
            globals,
            specular_multiplier,
            setting_float_copy(ini, 0x00B0_6D7C),
            0x00B2_C2BC,
            4
        );
        shader_store!(
            globals,
            emissive_multiplier,
            setting_float_copy(ini, 0x00B0_6D84),
            0x00B2_C2C0,
            4
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recomp_core::fixtures;
    use recomp_game::tes::settings::{SettingCollections, register_static_settings};
    use recomp_game::tes::shader_system::ShaderGlobalStore;
    use std::path::{Path, PathBuf};

    struct TempRoot(PathBuf);
    impl TempRoot {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
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

    // IDA: N/A fixture: the 03-E INI collection read from a fixture INI.
    fn load_collection(ini: &Path, text: &str) -> SettingCollections {
        std::fs::write(ini, text).unwrap();
        let mut collections = SettingCollections::default();
        register_static_settings(&mut collections);
        collections.ini.set_path(ini);
        assert!(collections.ini.open_ini());
        let _ = collections.ini.read_all();
        collections.ini.close();
        collections
    }

    fn store(address: u32, width: u8) -> ShaderGlobalStore {
        ShaderGlobalStore { address, width }
    }

    // IDA: 0040E8C1..0040E932 and 007B4870 stores, shared by both arms.
    fn prefix_trace() -> Vec<ShaderGlobalStore> {
        [
            (0x00B4_2E8C, 4),
            (0x00B4_2F3E, 1),
            (0x00B4_2EBC, 4),
            (0x00B4_2D78, 4),
            (0x00B4_6128, 4),
            (0x00B4_612C, 4),
            (0x00B4_6130, 4),
            (0x00B4_30A8, 4),
            (0x00B4_30A4, 4),
            (0x00B4_30AE, 1),
            (0x00B4_3077, 1),
            (0x00B4_2D40, 1),
        ]
        .map(|(address, width)| store(address, width))
        .to_vec()
    }

    /// `SHD` rows of `phase05.rows`: `SHD|run|KIND|key|value|evidence text`. Other blocks are theirs.
    fn shader_rows(kind: &str) -> Vec<Vec<String>> {
        fixtures::rows("phase05")
            .iter()
            .filter(|row| row.starts_with("SHD|"))
            .map(|row| row.split('|').map(str::to_owned).collect::<Vec<_>>())
            .filter(|row| row[2] == kind)
            .collect()
    }

    fn row_hex(key: &str) -> Vec<u8> {
        let rows = shader_rows("WINDOW");
        let row = rows
            .iter()
            .find(|row| row[3] == key)
            .unwrap_or_else(|| panic!("row {key}"));
        (0..row[4].len() / 2)
            .map(|i| u8::from_str_radix(&row[4][2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    /// `verify_shader_window`: 0040E898..0040EAA6 with the INI defaults (P05-RUN-001 values) and with selected
    /// values, both arms; 0040E938 reads RendererGlobals::hdr (D1).
    #[test]
    fn verify_shader_window() {
        let temp = TempRoot::new("oblivion-recomp-phase05-window");
        let defaults = load_collection(&temp.0.join("Empty.ini"), "[General]\n");

        let mut hdr = ShaderSystemGlobals::default();
        run_shader_window(
            &mut hdr,
            &ShaderWindowInputs {
                ini: &defaults.ini,
                texture_palette: true,
                renderer_hdr: true,
            },
        );
        // P05-RUN-001 SCENE_CALL snapshot (HDR arm, default INI values).
        assert_eq!(hdr.warning_callback, 0x0040_5150);
        assert_eq!(hdr.game_time_callback, 0x0040_5290);
        assert_eq!(hdr.image_space_effects, row_hex("imageSpaceCopy")[0]);
        assert!(hdr.texture_palette);
        assert_eq!(hdr.hit_tex_offset, 0.005);
        assert_eq!(hdr.blocked_tex_offset, 0.001);
        assert_eq!(hdr.blur_amount, 0.5);
        assert_eq!(hdr.local_tree_mip_bias, -0.25);
        assert_eq!(hdr.lod_tree_mip_bias, -0.75);
        assert_eq!(hdr.use_hard_drive_cache, 0);
        assert_eq!(hdr.display_lod_land, 1);
        assert_eq!(hdr.local_map_shader, 1);
        // The HDR block 00B431E8..00B43224, as bytes in address order.
        let mut block = Vec::new();
        for value in [
            hdr.bright_clamp,
            hdr.bright_clamp_interior,
            hdr.bright_scale,
            hdr.bright_scale_interior,
            hdr.blur_radius_hdr,
            hdr.blur_radius_hdr_interior,
            hdr.eye_adapt_speed,
            hdr.eye_adapt_speed_interior,
            hdr.emissive_hdr_mult,
            hdr.emissive_hdr_mult_interior,
            hdr.upper_lum_clamp,
            hdr.upper_lum_clamp_interior,
            hdr.target_lum,
            hdr.target_lum_interior,
        ] {
            block.extend_from_slice(&value.to_le_bytes());
        }
        block.extend_from_slice(&hdr.blur_passes_hdr.to_le_bytes());
        block.extend_from_slice(&hdr.blur_passes_hdr_interior.to_le_bytes());
        assert_eq!(
            block,
            row_hex("hdrBlock"),
            "the HDR block differs from P05-RUN-001"
        );
        assert_eq!(hdr.blend_type.to_le_bytes().to_vec(), row_hex("B2C1E4"));
        let mut brightness = hdr.sun_brightness.to_le_bytes().to_vec();
        brightness.extend_from_slice(&hdr.sky_brightness.to_le_bytes());
        assert_eq!(brightness, row_hex("B43154"));
        assert_eq!(hdr.specular_multiplier, 1.0);
        assert_eq!(hdr.emissive_multiplier, 1.0);
        let mut expected = prefix_trace();
        expected.extend(
            [
                0x00B2_C2BC,
                0x00B2_C2C0,
                0x00B2_C1E4,
                0x00B4_3220,
                0x00B4_31F8,
                0x00B4_3224,
                0x00B4_31FC,
                0x00B4_31E8,
                0x00B4_31EC,
                0x00B4_31F0,
                0x00B4_31F4,
                0x00B4_3200,
                0x00B4_3204,
                0x00B4_3208,
                0x00B4_320C,
                0x00B4_3210,
                0x00B4_3214,
                0x00B4_3218,
                0x00B4_321C,
                0x00B4_3154,
                0x00B4_3158,
            ]
            .map(|address| store(address, 4)),
        );
        assert_eq!(hdr.store_trace, expected, "HDR arm store order or widths");
        assert_eq!(
            hdr.blur_passes, 0,
            "the HDR arm must not write the 007B4830 globals"
        );
        assert_eq!(hdr.blur_texture_size, 0);
        assert_eq!(hdr.alpha_add_interior, 0.0);

        let selected = load_collection(
            &temp.0.join("Oblivion.ini"),
            "[Display]\nbDoImageSpaceEffects=0\nbLocalMapShader=0\n\
             [SpeedTree]\nfLocalTreeMipMapLODBias=0.75\nfLODTreeMipMapLODBias=-1.25\n\
             [General]\nbUseHardDriveCache=1\n[LOD]\nbDisplayLODLand=0\n\
             [GethitShader]\nfHitTexOffset=0.25\nfBlockedTexOffset=0.125\n\
             fBlurAmmount=0.75\n\
             [BlurShader]\niBlendType=6\niNumBlurpasses=5\nfBlurRadius=0.125\n\
             fAlphaAddInterior=0.625\nfAlphaAddExterior=0.375\niBlurTexSize=512\n\
             fSISpecularMult=2.25\nfSIEmmisiveMult=1.75\nfSunBrightness=0.25\n\
             fSkyBrightness=0.875\n",
        );
        let mut blur = ShaderSystemGlobals::default();
        run_shader_window(
            &mut blur,
            &ShaderWindowInputs {
                ini: &selected.ini,
                texture_palette: false,
                renderer_hdr: false,
            },
        );
        assert_eq!(blur.image_space_effects, 0);
        assert!(!blur.texture_palette);
        assert_eq!(blur.hit_tex_offset, 0.25);
        assert_eq!(blur.blocked_tex_offset, 0.125);
        assert_eq!(blur.blur_amount, 0.75);
        assert_eq!(blur.local_tree_mip_bias, 0.75);
        assert_eq!(blur.lod_tree_mip_bias, -1.25);
        assert_eq!(blur.use_hard_drive_cache, 1);
        assert_eq!(blur.display_lod_land, 0);
        assert_eq!(blur.local_map_shader, 0);
        assert_eq!(blur.blend_type, 6);
        assert_eq!(blur.blur_passes, 5);
        assert_eq!(blur.blur_radius, 0.125);
        assert_eq!(blur.alpha_add_interior, 0.625);
        assert_eq!(blur.alpha_add_exterior, 0.375);
        assert_eq!(blur.blur_texture_size, 512);
        assert_eq!(blur.sun_brightness, 0.25);
        assert_eq!(blur.sky_brightness, 0.875);
        assert_eq!(blur.specular_multiplier, 2.25);
        assert_eq!(blur.emissive_multiplier, 1.75);
        assert_eq!(blur.bright_clamp, 0.0);
        assert_eq!(blur.blur_passes_hdr, 0);
        let mut expected = prefix_trace();
        expected.extend(
            [
                0x00B2_C1EC,
                0x00B2_C2B0,
                0x00B2_C1E4,
                0x00B2_C1E8,
                0x00B2_C2B4,
                0x00B2_C2B8,
                0x00B4_3154,
                0x00B4_3158,
                0x00B2_C2BC,
                0x00B2_C2C0,
            ]
            .map(|address| store(address, 4)),
        );
        assert_eq!(blur.store_trace, expected, "blur arm store order or widths");
        let mut direct = ShaderSystemGlobals::default();
        assert_eq!(
            blur_shader_apply_parameters(&mut direct, 3, 1, 0.5, 0.25, 0.125, 64),
            3,
            "007B4830 must return blendType"
        );
    }

    /// `verify_x87_copy`: the x87 fld/fstp copy quiets a signaling NaN (bit 22) and keeps every other bit pattern.
    #[test]
    fn verify_x87_copy() {
        let temp = TempRoot::new("oblivion-recomp-phase05-nan");
        let mut collections = load_collection(&temp.0.join("Empty.ini"), "[General]\n");
        collections
            .ini
            .find_address_mut(0x00B0_2D78)
            .expect("fLocalTreeMipMapLODBias must be registered")
            .raw = 0x7F80_0001; // signaling NaN
        collections.ini.find_address_mut(0x00B0_2D80).unwrap().raw = 0x8000_0000;
        let mut globals = ShaderSystemGlobals::default();
        run_shader_window(
            &mut globals,
            &ShaderWindowInputs {
                ini: &collections.ini,
                texture_palette: false,
                renderer_hdr: true,
            },
        );
        assert_eq!(globals.local_tree_mip_bias.to_bits(), 0x7FC0_0001);
        assert_eq!(globals.lod_tree_mip_bias.to_bits(), 0x8000_0000);
    }
}
