//! The renderer WinMain 0040E82F..0040E88E creates (the WinMain block lives in `crate::tes::winmain`, and the game's own
//! creation functions, 00572DC0, 004052F0 and 00498340, in `crate::tes::os::renderer_init`):
//! - Gamebryo's NiDX9Renderer_Create 0076BD40 and NiDX9Renderer_Initialize 0076A1C0, which 00498340 calls, with the adapter
//!   and shader validation 004980D0 and the texts of the "Failed to initialize renderer" contract;
//! - the two renderer notification pairs 0040C280/0040C1B0.
//!
//! Every device-visible call (IDirect3D9, IDirect3DDevice9, Win32 window calls, file output) follows the original order and
//! arguments, as a `DeviceTraceEntry`, and answers through `RendererPlatform`, `Direct3D9` and `Direct3DDevice9`. Gamebryo's
//! renderer object graph (render target groups, buffer data, shader factories, geometry and light managers, accumulators,
//! image converters, NiPixelData wrappers) is a recorded host substitution: `RendererState` keeps the values those objects
//! hold that later phases read. Runtime evidence: P04-RUN-001, P04-RUN-002.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy_ecs::prelude::*;
use oblivion_core::x87;

use super::dx9_system::*;
use super::pixel_data::{PixelData, PixelFormat, construct_pixel_data};
use super::renderer_tables::{
    DEFAULT_RENDER_STATES, DEFAULT_SAMPLER_STATES, DEFAULT_TEXTURE_STAGE_STATES, FORMAT_BITS_TABLE,
    TEXTURE_PIXEL_FORMATS,
};
use super::texture_loading::{
    DirectImageInfo, DirectLevelDesc, DirectResourceType, RendererTextureStatus,
    SourceTextureUpload, TextureMemoryStatistics, TextureRendererBoundary, TextureRendererData,
    direct_file_facts, direct_image_info, direct_texture_levels,
};
use crate::gamebryo::camera::{CameraFrustum, CameraPort};
use crate::tes::settings::SettingCollection;

pub const D3DRS_ZENABLE: u32 = 7;
pub const D3DRS_FILLMODE: u32 = 8;
pub const D3DRS_SHADEMODE: u32 = 9;
pub const D3DRS_ZWRITEENABLE: u32 = 14;
pub const D3DRS_ALPHATESTENABLE: u32 = 15;
pub const D3DRS_SRCBLEND: u32 = 19;
pub const D3DRS_DESTBLEND: u32 = 20;
pub const D3DRS_CULLMODE: u32 = 22;
pub const D3DRS_ZFUNC: u32 = 23;
pub const D3DRS_ALPHAREF: u32 = 24;
pub const D3DRS_ALPHAFUNC: u32 = 25;
pub const D3DRS_DITHERENABLE: u32 = 26;
pub const D3DRS_ALPHABLENDENABLE: u32 = 27;
pub const D3DRS_FOGENABLE: u32 = 28;
pub const D3DRS_SPECULARENABLE: u32 = 29;
pub const D3DRS_FOGCOLOR: u32 = 34;
pub const D3DRS_FOGTABLEMODE: u32 = 35;
pub const D3DRS_RANGEFOGENABLE: u32 = 48;
pub const D3DRS_LIGHTING: u32 = 137;
pub const D3DRS_AMBIENT: u32 = 139;
pub const D3DRS_FOGVERTEXMODE: u32 = 140;
pub const D3DRS_COLORVERTEX: u32 = 141;
pub const D3DRS_NORMALIZENORMALS: u32 = 143;
pub const D3DRS_DIFFUSEMATERIALSOURCE: u32 = 145;
pub const D3DRS_AMBIENTMATERIALSOURCE: u32 = 147;
pub const D3DRS_EMISSIVEMATERIALSOURCE: u32 = 148;
pub const D3DTSS_COLOROP: u32 = 1;
pub const D3DTSS_ALPHAOP: u32 = 4;
pub const D3DTSS_TEXCOORDINDEX: u32 = 11;
pub const D3DTOP_DISABLE: u32 = 1;
pub const D3DTS_VIEW: u32 = 2;
pub const D3DTS_PROJECTION: u32 = 3;
pub const D3DMULTISAMPLE_NONE: u32 = 0;
pub const D3DMULTISAMPLE_NONMASKABLE: u32 = 1;
pub const D3DSWAPEFFECT_DISCARD: u32 = 1;
pub const D3DSWAPEFFECT_FLIP: u32 = 2;
pub const D3DSWAPEFFECT_COPY: u32 = 3;
pub const D3DPRESENT_INTERVAL_DEFAULT: u32 = 0;
pub const D3DPRESENT_INTERVAL_ONE: u32 = 1;
pub const D3DPRESENT_INTERVAL_TWO: u32 = 2;
pub const D3DPRESENT_INTERVAL_THREE: u32 = 4;
pub const D3DPRESENT_INTERVAL_FOUR: u32 = 8;
pub const D3DPRESENT_INTERVAL_IMMEDIATE: u32 = 0x8000_0000;
pub const D3DPRASTERCAPS_FOGTABLE: u32 = 0x0000_0100;
pub const D3DPRASTERCAPS_WFOG: u32 = 0x0010_0000;
pub const D3DPTADDRESSCAPS_BORDER: u32 = 0x08;
pub const D3DPTEXTURECAPS_POW2: u32 = 0x0000_0002;
pub const D3DPTEXTURECAPS_NONPOW2CONDITIONAL: u32 = 0x0000_0100;
pub const D3DPTEXTURECAPS_PROJECTED: u32 = 0x0000_0400;
pub const D3DPTEXTURECAPS_MIPCUBEMAP: u32 = 0x0001_0000;
pub const D3DPTFILTERCAPS_MINFANISOTROPIC: u32 = 0x0000_0400;
pub const D3DCAPS2_FULLSCREENGAMMA: u32 = 0x0002_0000;
pub const D3DCAPS2_DYNAMICTEXTURES: u32 = 0x2000_0000;
pub const D3DPMISCCAPS_MRTINDEPENDENTBITDEPTHS: u32 = 0x0002_0000;
pub const D3DPMISCCAPS_MRTPOSTPIXELSHADERBLENDING: u32 = 0x0008_0000;
pub const D3DDEVCAPS_HWTRANSFORMANDLIGHT: u32 = 0x0001_0000;
pub const D3DDEVCAPS_NPATCHES: u32 = 0x0100_0000;
/// IDA: 00498449 GetClassLongA(child, GCL_MENUNAME).
pub const GCL_MENUNAME: i32 = -8;
/// IDA: 0049844D GetWindowLongA(main, GWL_STYLE).
pub const GWL_STYLE: i32 = -16;
/// IDA: 00498413 the child's WS_CHILD | WS_VISIBLE (P04-RUN-001).
pub const WS_CHILD_VISIBLE: u32 = 0x5000_0000;
pub const SWP_SHOWWINDOW: u32 = 0x40;

/// IDA: 00977C50 ReturnTrue_SharedStub.
pub const ORIGINAL_RETURN_TRUE_STUB: u32 = 0x0097_7C50;
/// IDA: 00405440 Renderer_HandleTransitionCallback.
pub const ORIGINAL_TRANSITION_CALLBACK: u32 = 0x0040_5440;

const EMPTY_COMPONENT: (u32, u32, u8, u8) = (19, 5, 0, 1);

// IDA: the NiPixelFormat statics 0071B600 compares against (IDB .data bytes, read 2026-10-05).
const PAL4_FORMAT: PixelFormat = super::pixel_data::format_row(
    0x00B2_5CE0,
    1,
    4,
    2,
    0,
    [
        (16, 3, 4, 1),
        EMPTY_COMPONENT,
        EMPTY_COMPONENT,
        EMPTY_COMPONENT,
    ],
);
const PALA4_FORMAT: PixelFormat = super::pixel_data::format_row(
    0x00B2_5D28,
    1,
    4,
    3,
    0,
    [
        (16, 3, 4, 1),
        EMPTY_COMPONENT,
        EMPTY_COMPONENT,
        EMPTY_COMPONENT,
    ],
);
const PAL8_FORMAT: PixelFormat = super::pixel_data::format_row(
    0x00B2_5D70,
    1,
    8,
    2,
    0,
    [
        (16, 3, 8, 1),
        EMPTY_COMPONENT,
        EMPTY_COMPONENT,
        EMPTY_COMPONENT,
    ],
);
const PALA8_FORMAT: PixelFormat = super::pixel_data::format_row(
    0x00B2_5DB8,
    1,
    8,
    3,
    0,
    [
        (16, 3, 8, 1),
        EMPTY_COMPONENT,
        EMPTY_COMPONENT,
        EMPTY_COMPONENT,
    ],
);
const RGBA32_FORMAT: PixelFormat = super::pixel_data::format_row(
    0x00B2_5E00,
    1,
    32,
    1,
    0,
    [(0, 0, 8, 1), (1, 0, 8, 1), (2, 0, 8, 1), (3, 0, 8, 1)],
);
const RGB24_FORMAT: PixelFormat = super::pixel_data::format_row(
    0x00B2_5E48,
    1,
    24,
    0,
    0,
    [(0, 0, 8, 1), (1, 0, 8, 1), (2, 0, 8, 1), EMPTY_COMPONENT],
);
const RGBA5551_FORMAT: PixelFormat = super::pixel_data::format_row(
    0x00B2_5E90,
    1,
    16,
    1,
    0,
    [(0, 0, 5, 1), (1, 0, 5, 1), (2, 0, 5, 1), (3, 0, 1, 1)],
);
const BGRA5551_FORMAT: PixelFormat = super::pixel_data::format_row(
    0x00B2_5ED8,
    1,
    16,
    1,
    0,
    [(2, 0, 5, 1), (1, 0, 5, 1), (0, 0, 5, 1), (3, 0, 1, 1)],
);

pub(crate) const UNKNOWN_RENDERER_ERROR: &str = "Unknown error creating the Gamebryo Renderer.";
pub(crate) const ADAPTER_DESC_FAILED: &str = "GetAdapterDesc() failed.";
pub(crate) const DEVICE_CAPS_FAILED: &str = "NiXAdapaterDesc::GetDeviceCaps() failed.";

// IDA: 0076A1C0/00762110/0076BD40 error strings (00761A90 into 00B3F828, strncpy_s 0xFF).
const COULD_NOT_INITIALIZE_DIRECTX: &str = "Creation failed: Could not initialize DirectX9";
const INVALID_ADAPTER: &str = "Creation failed: Invalid Adapter";
const INVALID_DEVICE_TYPE: &str = "Creation failed: Invalid 3D device type";
const DEVICE_NOT_VALID: &str = "Creation failed: Requested device not valid";
const NO_REFERENCE_DEVICE: &str = "Creation failed: Could not create reference device";
const NO_HARDWARE_DEVICE: &str = "Creation failed: Could not create hardware device";
const NO_DEVICE_CAPS: &str = "Creation failed: Could not query device caps";
const NO_MULTI_TEXTURE: &str = "Creation failed: Device does not support single-pass multi-texture";

/// IDA: one of the three 0x18-byte records at 00B12DC8 (p = 00B12DD0 + 0x18*i).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RendererUiStateRecord {
    /// 00B12DC8 + 0x18*i (read by 00572EC0/005732D0).
    pub flag: u8,
    /// 00B12DD0 + 0x18*i (read by 00572DF0..00572F43).
    pub value: u32,
    /// 00B12DD4 + 0x18*i (read by 00572EA0).
    pub scale: f32,
}

/// IDA: the 8-byte object at 00B3A6B0.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RendererUiState {
    pub unknown_00: u32,
    pub unknown_04: u32,
}

/// IDA: the 00B12DC8 records.
/// TODO: [ ] Later phases (carried): the 00572DF0..005732D0 readers.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct RendererUiGlobals {
    pub records: [RendererUiStateRecord; 3],
    /// Host record of the store order for the fixture (float, byte, dword).
    pub store_trace: Vec<u32>,
}

/// IDA: the 00498340/004980D0/0076BD40 inputs: INI settings from the 03-E collection (g_iniSetting_* in the IDB) and the
/// .data constants 00B06C2C..00B06C48 plus the zero-filled 00B34FB4/B8/BC.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererSettings {
    /// 00B06C74 bFull Screen:Display.
    pub full_screen: bool,
    /// 00B06C5C iSize W:Display.
    pub width: i32,
    /// 00B06C64 iSize H:Display.
    pub height: i32,
    /// 00B06C7C iLocation X:Display.
    pub location_x: i32,
    /// 00B06C84 iLocation Y:Display.
    pub location_y: i32,
    /// 00B06C54 iAdapter:Display.
    pub adapter: u32,
    /// 00B06C8C iNPatches:Display.
    pub npatches: i32,
    /// 00B06CA4 bUse Shaders:Display.
    pub use_shaders: bool,
    /// 00B06CAC bUseRefractionShader:Display.
    pub use_refraction_shader: bool,
    /// 00B06CC4 bDoImageSpaceEffects:Display.
    pub image_space_effects: bool,
    /// 00B06D0C iMultiSample:Display.
    pub multisample: i32,
    /// 00B06D14 bAllowScreenShot:Display.
    pub allow_screenshot: bool,
    /// 00B06D24 iTexMipMapSkip:Display.
    pub tex_mip_skip: i32,
    /// 00B06D2C iTexMipMapMinimum:Display.
    pub tex_mip_minimum: i32,
    /// 00B06D34 bUseBlurShader:BlurShader.
    pub blur_shader: bool,
    /// 00B06DA4 bAllow20HairShader:Display.
    pub allow_20_hair: bool,
    /// 00B06DB4 bForce1XShaders:Display.
    pub force_1x_shaders: bool,
    /// 00B06DBC bForcePow2Textures:Display.
    pub force_pow2_textures: bool,
    /// 00B06DDC fSunlightDimmer:BlurShader.
    pub sunlight_dimmer_blur: f32,
    /// 00B06DE4 bDoHighDynamicRange:BlurShaderHDR.
    pub high_dynamic_range: bool,
    /// 00B06E34 fSunlightDimmer:BlurShaderHDR.
    pub sunlight_dimmer_hdr: f32,
    /// 00B06E4C fTreeDimmer:BlurShaderHDR.
    pub tree_dimmer: f32,
    /// 00B06E54 fGrassDimmer:BlurShaderHDR.
    pub grass_dimmer: f32,
    /// 00B06EAC..00B06EF4 fLightLOD1..fEyeEnvMapLOD2.
    pub lods: [f32; 10],
    /// 00B06F14 bShadowsOnGrass:Display.
    pub shadows_on_grass: bool,
    /// 00B06F1C iShadowMapResolution:Display.
    pub shadow_map_resolution: i32,
    /// 00B06F2C iShadowFilter:Display.
    pub shadow_filter: i32,
    /// 00B06F64 fGamma:Display.
    pub gamma: f32,
    /// 00B06F6C iPresentInterval:Display.
    pub present_interval: i32,
    /// 00B06F74 bCreateShaderPackage:General.
    pub create_shader_package: bool,
    /// 00B06F7C fShadowFadeTime:Display.
    pub shadow_fade_time: f32,
    /// 00B06F84 bDynamicWindowReflections:Display.
    pub dynamic_window_reflections: bool,
    /// 00B06F8C iMaxLandscapeTextures:Display.
    pub max_landscape_textures: i32,
    /// 00B06F94 bFullBrightLighting:Display.
    pub full_bright_lighting: bool,
    /// 00B06F9C bLandscapeBlend:Display.
    pub landscape_blend: bool,
    /// 00B07050 bUseWaterShader:Water.
    pub water_shader: bool,
    /// 00B07058 bUseWaterHiRes:Water.
    pub water_hi_res: bool,
    /// 00B07060 bUseWaterReflections:Water.
    pub water_reflections: bool,
    /// 00B07090 bUseWaterDisplacements:Water.
    pub water_displacements: bool,
    /// 00B06A2C uGridsToLoad:General.
    pub grids_to_load: u32,
    /// 00B06C2C.
    pub applied_gamma: f32,
    /// 00B06C30.
    pub use_flags: u32,
    /// 00B06C34 (004979E0 -> X8R8G8B8).
    pub frame_buffer_format: i32,
    /// 00B06C38 D24S8.
    pub depth_stencil_format: i32,
    /// 00B06C40.
    pub back_buffer_count: u32,
    /// 00B06C44 minimum.
    pub pixel_shader_version: u32,
    /// 00B06C48 minimum.
    pub vertex_shader_version: u32,
    /// 00B34FB4.
    pub refresh_rate: u32,
    /// 00B34FB8 (0 = HAL pure).
    pub device_desc: i32,
    /// 00B34FBC.
    pub swap_effect: i32,
    /// 00B350DC g_rendererBackgroundColor.
    pub background_red: f32,
    /// 00B350E0.
    pub background_green: f32,
    /// 00B350E4.
    pub background_blue: f32,
}

impl Default for RendererSettings {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            full_screen: false,
            width: 0,
            height: 0,
            location_x: 0,
            location_y: 0,
            adapter: 0,
            npatches: 0,
            use_shaders: false,
            use_refraction_shader: false,
            image_space_effects: false,
            multisample: 0,
            allow_screenshot: false,
            tex_mip_skip: 0,
            tex_mip_minimum: 0,
            blur_shader: false,
            allow_20_hair: false,
            force_1x_shaders: false,
            force_pow2_textures: false,
            sunlight_dimmer_blur: 0.0,
            high_dynamic_range: false,
            sunlight_dimmer_hdr: 0.0,
            tree_dimmer: 0.0,
            grass_dimmer: 0.0,
            lods: [0.0; 10],
            shadows_on_grass: false,
            shadow_map_resolution: 0,
            shadow_filter: 0,
            gamma: 0.0,
            present_interval: 0,
            create_shader_package: false,
            shadow_fade_time: 0.0,
            dynamic_window_reflections: false,
            max_landscape_textures: 0,
            full_bright_lighting: false,
            landscape_blend: false,
            water_shader: false,
            water_hi_res: false,
            water_reflections: false,
            water_displacements: false,
            grids_to_load: 0,
            applied_gamma: 1.0,
            use_flags: 0x28,
            frame_buffer_format: 3,
            depth_stencil_format: 0x4B,
            back_buffer_count: 1,
            pixel_shader_version: 0x103,
            vertex_shader_version: 0x101,
            refresh_rate: 0,
            device_desc: 0,
            swap_effect: 0,
            background_red: 0.0,
            background_green: 0.0,
            background_blue: 0.0,
        }
    }
}

/// IDA: N/A a setting by its static's address; an absent one reads as false.
fn setting_bool(ini: &SettingCollection, address: u32) -> bool {
    ini.find_address(address).is_some_and(|s| s.as_bool())
}

/// IDA: N/A as `setting_bool`, reading 0.
fn setting_int(ini: &SettingCollection, address: u32) -> i32 {
    ini.find_address(address).map_or(0, |s| s.as_int())
}

/// IDA: N/A as `setting_bool`, reading 0.0.
fn setting_float(ini: &SettingCollection, address: u32) -> f32 {
    ini.find_address(address).map_or(0.0, |s| s.as_float())
}

/// IDA: the 00498340 setting reads from the 03-E collection. Completion: 100%.
#[must_use]
pub fn renderer_settings_from(ini: &SettingCollection) -> RendererSettings {
    let mut lods = [0.0; 10];
    for (i, lod) in lods.iter_mut().enumerate() {
        *lod = setting_float(ini, 0x00B0_6EAC + 8 * i as u32);
    }
    RendererSettings {
        full_screen: setting_bool(ini, 0x00B0_6C74),
        width: setting_int(ini, 0x00B0_6C5C),
        height: setting_int(ini, 0x00B0_6C64),
        location_x: setting_int(ini, 0x00B0_6C7C),
        location_y: setting_int(ini, 0x00B0_6C84),
        adapter: setting_int(ini, 0x00B0_6C54) as u32,
        npatches: setting_int(ini, 0x00B0_6C8C),
        use_shaders: setting_bool(ini, 0x00B0_6CA4),
        use_refraction_shader: setting_bool(ini, 0x00B0_6CAC),
        image_space_effects: setting_bool(ini, 0x00B0_6CC4),
        multisample: setting_int(ini, 0x00B0_6D0C),
        allow_screenshot: setting_bool(ini, 0x00B0_6D14),
        tex_mip_skip: setting_int(ini, 0x00B0_6D24),
        tex_mip_minimum: setting_int(ini, 0x00B0_6D2C),
        blur_shader: setting_bool(ini, 0x00B0_6D34),
        allow_20_hair: setting_bool(ini, 0x00B0_6DA4),
        force_1x_shaders: setting_bool(ini, 0x00B0_6DB4),
        force_pow2_textures: setting_bool(ini, 0x00B0_6DBC),
        sunlight_dimmer_blur: setting_float(ini, 0x00B0_6DDC),
        high_dynamic_range: setting_bool(ini, 0x00B0_6DE4),
        sunlight_dimmer_hdr: setting_float(ini, 0x00B0_6E34),
        tree_dimmer: setting_float(ini, 0x00B0_6E4C),
        grass_dimmer: setting_float(ini, 0x00B0_6E54),
        lods,
        shadows_on_grass: setting_bool(ini, 0x00B0_6F14),
        shadow_map_resolution: setting_int(ini, 0x00B0_6F1C),
        shadow_filter: setting_int(ini, 0x00B0_6F2C),
        gamma: setting_float(ini, 0x00B0_6F64),
        present_interval: setting_int(ini, 0x00B0_6F6C),
        create_shader_package: setting_bool(ini, 0x00B0_6F74),
        shadow_fade_time: setting_float(ini, 0x00B0_6F7C),
        dynamic_window_reflections: setting_bool(ini, 0x00B0_6F84),
        max_landscape_textures: setting_int(ini, 0x00B0_6F8C),
        full_bright_lighting: setting_bool(ini, 0x00B0_6F94),
        landscape_blend: setting_bool(ini, 0x00B0_6F9C),
        water_shader: setting_bool(ini, 0x00B0_7050),
        water_hi_res: setting_bool(ini, 0x00B0_7058),
        water_reflections: setting_bool(ini, 0x00B0_7060),
        water_displacements: setting_bool(ini, 0x00B0_7090),
        grids_to_load: setting_int(ini, 0x00B0_6A2C) as u32,
        ..RendererSettings::default()
    }
}

/// IDA: the globals 00498340, 004980D0 and 007B45F0 write for later phases. Every one starts at 0 (unloaded .data with no
/// CRT writer); P04-RUN-001 confirmed the window-time values.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct RendererGlobals {
    /// 00B06C4C g_rendererWidth.
    pub width: u32,
    /// 00B06C50 g_rendererHeight.
    pub height: u32,
    /// 00B06C30 after |= 4 (fullscreen).
    pub use_flags: u32,
    /// 00B06C44 (caps low word on success).
    pub pixel_shader_version: u32,
    /// 00B06C48.
    pub vertex_shader_version: u32,
    /// 00B06C2C.
    pub applied_gamma: f32,
    /// 00B34FA0.
    pub present_interval: i32,
    /// 00B34FA4.
    pub gamma_changed: bool,
    /// 00B34FA5 (Caps2 & D3DCAPS2_FULLSCREENGAMMA).
    pub fullscreen_gamma: bool,
    /// 00B34FA6.
    pub npatches: bool,
    /// 00B34FA8 (child, or the main window fullscreen).
    pub device_window: Option<WindowHandle>,
    /// 00B34FAC.
    pub main_window: Option<WindowHandle>,
    /// 00B34FB0.
    pub instance: u32,
    /// 00B34FC0.
    pub multisample: i32,
    /// 00B34FC8 char[260].
    pub failure_text: String,
    /// 00B350D6.
    pub fp16_blending: bool,
    /// 00B350D7.
    pub non_standard_aspect: bool,
    /// 00B43070.
    pub hdr: bool,
    /// 00B43071.
    pub fp16_filtering: bool,
    /// 00B43072 (TextureFilterCaps & MINFANISOTROPIC).
    pub anisotropic_min_filter: bool,
    /// 00B43073.
    pub bloom: bool,
    /// 00B43075.
    pub sli: bool,
    /// 00B42E96.
    pub non_pow2_textures: bool,
    /// 00B42E97 (only cleared here).
    pub multi_gpu_flag: bool,
    /// 00B42E84.
    pub refraction: bool,
    /// 00B42F32.
    pub hair_20: bool,
    /// 00B256CC = 1 (0049865C).
    pub texture_create_renderer_data: bool,
    /// 00B3F708 = 0 (00498663).
    pub texture_mipmap_by_default: bool,
    /// 00B4205C.
    pub mip_skip: i32,
    /// 00B42060.
    pub mip_minimum: i32,
    /// 00B42E98.
    pub texture_formats: [bool; 13],
    /// 00B42F48 g_renderPath.
    pub render_path: i32,
    /// 00B42D74 g_rendererPS2xPath (6 ATI, 5 NV, 2 other).
    pub ps2x_path: i32,
    /// 00B42E88 g_rendererShaderModelClass.
    pub shader_model_class: i32,
    /// 00B42F40 g_rendererShaderFlags.
    pub shader_flags: u32,
    /// 00B42EA5.
    pub shaders_30: bool,
    /// 00B42F4C (tail copies 00498D53..00498E77 from here on).
    pub sunlight_dimmer: f32,
    /// 00B42EA8.
    pub tree_dimmer: f32,
    /// 00B42F44.
    pub grass_dimmer: f32,
    /// 00B430AC.
    pub create_shader_package: bool,
    /// 00B4307C..00B430A0.
    pub lods: [f32; 10],
    /// 00B42E86.
    pub full_bright: bool,
    /// 00B430AD.
    pub dynamic_window_reflections: bool,
    /// 00B2C67E.
    pub landscape_blend: bool,
    /// 00B2C674.
    pub max_landscape_textures: i32,
    /// 00B2C684.
    pub grids_to_load: u32,
    /// 00B2C680.
    pub shadow_fade_time: f32,
    /// 00B2C67C (word).
    pub shadow_map_resolution: u16,
    /// 00B430B0.
    pub shadow_filter: i32,
    /// 00B06F14 after 0049886C.
    pub shadows_on_grass: bool,
}

/// IDA: N/A the 22 statics 007637D0 copies (00B263A0..; cases 6 and 7 have none).
#[must_use]
pub const fn original_texture_pixel_formats() -> &'static [PixelFormat; 22] {
    &TEXTURE_PIXEL_FORMATS
}

/// IDA: 00B25E48 the RGB24 source format FaceGen builds its textures in.
#[must_use]
pub const fn original_rgb24_pixel_format() -> PixelFormat {
    RGB24_FORMAT
}

/// IDA: 00700B60 NiPixelFormat_GetComponentMask. Completion: 100%.
#[must_use]
pub fn pixel_format_component_mask(format: &PixelFormat, component: u32) -> u32 {
    let mut shift = 0u8;
    let mut found = None;
    for entry in &format.components_14 {
        if entry.component == component {
            found = Some(entry);
            break;
        }
        shift = shift.wrapping_add(entry.bits);
    }
    let Some(entry) = found else {
        return 0;
    };
    if entry.bits == 0 {
        return 0;
    }
    // 00700B98..00700BBC: sum of 1 << (bits-1) .. 1 << 1, plus 1, shifted (x86 shifts mask the count).
    let mut value = 0u32;
    let mut bit = i32::from(entry.bits) - 1;
    while bit > 0 {
        value = value.wrapping_add(1u32.wrapping_shl(bit as u32));
        bit -= 1;
    }
    let result = value.wrapping_add(1).wrapping_shl(u32::from(shift));
    if format.flags_00 & 1 != 0 {
        return result;
    }
    // 00700BC5..00700BF4: byte-swapped and shifted right by 32 - bpp.
    result
        .swap_bytes()
        .wrapping_shr(32 - u32::from(format.bits_per_pixel_01))
}

/// IDA: 0071B4A0 bits of a component (0 when absent). Completion: 100%.
fn component_bits(format: &PixelFormat, component: u32) -> u8 {
    format
        .components_14
        .iter()
        .find(|entry| entry.component == component)
        .map_or(0, |entry| entry.bits)
}

/// IDA: 00700C00 NiPixelFormat component shift: the summed bits of the components before the match (0 when absent); flags
/// bit 0 clear (big endian) returns bpp - (bits + shift). Completion: 100%.
fn component_shift(format: &PixelFormat, component: u32) -> u8 {
    let mut shift = 0u8;
    for entry in &format.components_14 {
        if entry.component == component {
            if format.flags_00 & 1 != 0 {
                return shift;
            }
            return format
                .bits_per_pixel_01
                .wrapping_sub(entry.bits.wrapping_add(shift));
        }
        shift = shift.wrapping_add(entry.bits);
    }
    0
}

/// IDA: 0076BEF0 NiDX9Renderer_PixelFormatToD3DFormat. The 16-bit depth arm is 0076C27B: `0x49` for one stencil bit, `0x50`
/// otherwise, so it answers D15S1 or 80 (D3DFMT_D16); the C++ names the second D3DFMT_D16_LOCKABLE (70), which the exact
/// executable does not support. Completion: 100%.
#[must_use]
pub fn pixel_format_to_d3d(format: &PixelFormat) -> u32 {
    if format.renderer_hint_0c != 0xFFFF_FFFF {
        return format.renderer_hint_0c;
    }
    let bits = format.bits_per_pixel_01;
    let mask0 = pixel_format_component_mask(format, 0);
    match format.format_04 {
        0 => match bits {
            8 => {
                if mask0 == 0xE0 {
                    D3DFMT_R3G3B2
                } else if mask0 != 0 {
                    D3DFMT_UNKNOWN
                } else {
                    D3DFMT_L8
                }
            }
            16 => match mask0 {
                0xF800 => D3DFMT_R5G6B5,
                0x7C00 => D3DFMT_X1R5G5B5,
                0xF00 => D3DFMT_X4R4G4B4,
                0 => D3DFMT_L16,
                _ => D3DFMT_UNKNOWN,
            },
            24 => D3DFMT_R8G8B8,
            32 => match mask0 {
                0xFF_0000 => D3DFMT_X8R8G8B8,
                0xFF => D3DFMT_X8B8G8R8,
                0xFFFF_0000 => D3DFMT_G16R16,
                _ => D3DFMT_UNKNOWN,
            },
            _ => D3DFMT_UNKNOWN,
        },
        1 => match bits {
            8 => match pixel_format_component_mask(format, 9) {
                0 => D3DFMT_A8,
                15 => D3DFMT_A4L4,
                _ => D3DFMT_UNKNOWN,
            },
            16 => match mask0 {
                0x7C00 => D3DFMT_A1R5G5B5,
                0xF00 => D3DFMT_A4R4G4B4,
                0xE0 => D3DFMT_A8R3G3B2,
                0 => D3DFMT_A8L8,
                _ => D3DFMT_UNKNOWN,
            },
            32 => match mask0 {
                0xFF_0000 => D3DFMT_A8R8G8B8,
                0x3FF => D3DFMT_A2B10G10R10,
                0xFF => D3DFMT_A8B8G8R8,
                0x3FF0_0000 => D3DFMT_A2R10G10B10,
                _ => D3DFMT_UNKNOWN,
            },
            64 => D3DFMT_A16B16G16R16F,
            128 => D3DFMT_A32B32G32R32F,
            _ => D3DFMT_UNKNOWN,
        },
        2 => D3DFMT_P8,
        4 => D3DFMT_DXT1,
        5 => D3DFMT_DXT3,
        6 => D3DFMT_DXT5,
        8 => D3DFMT_V8U8,
        9 => match bits {
            16 => D3DFMT_L6V5U5,
            32 => D3DFMT_X8L8V8U8,
            _ => D3DFMT_UNKNOWN,
        },
        11 => match bits {
            8 => {
                if pixel_format_component_mask(format, 3) != 0 {
                    D3DFMT_A8
                } else {
                    D3DFMT_L8
                }
            }
            16 => D3DFMT_R16F,
            32 => D3DFMT_R32F,
            _ => D3DFMT_UNKNOWN,
        },
        12 => match bits {
            16 => D3DFMT_A8L8,
            32 => D3DFMT_G16R16F,
            64 => D3DFMT_G32R32F,
            _ => D3DFMT_UNKNOWN,
        },
        14 => match bits {
            32 => D3DFMT_A8R8G8B8,
            64 => D3DFMT_A16B16G16R16F,
            128 => D3DFMT_A32B32G32R32F,
            _ => D3DFMT_UNKNOWN,
        },
        // 0076C23B..0076C299: component 0x12 (stencil) / 0x11 (depth) bits.
        15 => match bits {
            16 => {
                if component_bits(format, 0x12) == 1 {
                    D3DFMT_D15S1
                } else {
                    D3DFMT_D16
                }
            }
            32 => match component_bits(format, 0x12) {
                4 => D3DFMT_D24X4S4,
                8 => D3DFMT_D24S8,
                _ => {
                    if component_bits(format, 0x11) == 0x18 {
                        D3DFMT_D24X8
                    } else {
                        D3DFMT_D32
                    }
                }
            },
            _ => D3DFMT_UNKNOWN,
        },
        _ => D3DFMT_UNKNOWN,
    }
}

/// IDA: the 0x44-byte NiPixelFormat image: +0 flags, +1 bits per pixel, +4 format, +8 tiling, +0xC renderer hint, +0x10
/// extra, +0x14 four 12-byte components {component, representation, bits, signed}, padding zero. Completion: 100%.
#[must_use]
pub fn pixel_format_words(format: &PixelFormat) -> [u32; 17] {
    let mut words = [0u32; 17];
    words[0] = u32::from(format.flags_00) | (u32::from(format.bits_per_pixel_01) << 8);
    words[1] = format.format_04;
    words[2] = format.tiling_08;
    words[3] = format.renderer_hint_0c;
    words[4] = format.extra_10;
    for (i, component) in format.components_14.iter().enumerate() {
        words[5 + 3 * i] = component.component;
        words[6 + 3 * i] = component.representation;
        words[7 + 3 * i] = u32::from(component.bits) | (u32::from(component.is_signed) << 8);
    }
    words
}

/// IDA: 0071AD40 NiPixelFormat_Equals / 0070E260 its negation. Completion: 100%.
#[must_use]
pub fn pixel_format_equals(left: &PixelFormat, right: &PixelFormat) -> bool {
    if right.format_04 != left.format_04
        || right.flags_00 != left.flags_00
        || right.tiling_08 != left.tiling_08
        || right.bits_per_pixel_01 != left.bits_per_pixel_01
    {
        return false;
    }
    if right.renderer_hint_0c != left.renderer_hint_0c
        && left.renderer_hint_0c != 0xFFFF_FFFF
        && right.renderer_hint_0c != 0xFFFF_FFFF
    {
        return false;
    }
    left.components_14 == right.components_14
}

/// IDA: 0070E240: formats 4..6 (DXT1/3/5). Completion: 100%.
const fn pixel_format_compressed(format: &PixelFormat) -> bool {
    format.format_04 >= 4 && format.format_04 <= 6
}

/// IDA: 0071B4D0 NiImageConverter_BuildChannelMasks for components 0..3. 0071B50B/0071B544/0071B580 store 8 - 0 for absent
/// R/G/B and 0071B5BC stores 8 for absent alpha: the same value.
struct ChannelMasks {
    masks: [u32; 4],
    shifts: [u8; 4],
    right_shifts: [u8; 4],
}

// IDA: 0071B4D0 NiImageConverter_BuildChannelMasks.
fn channel_masks(format: &PixelFormat) -> ChannelMasks {
    let mut result = ChannelMasks {
        masks: [0; 4],
        shifts: [0; 4],
        right_shifts: [0; 4],
    };
    for component in 0..4u32 {
        let i = component as usize;
        result.masks[i] = pixel_format_component_mask(format, component);
        result.shifts[i] = component_shift(format, component);
        result.right_shifts[i] = format
            .components_14
            .iter()
            .find(|entry| entry.component == component)
            .map_or(8, |entry| 8u8.wrapping_sub(entry.bits));
    }
    result
}

/// IDA: 0071B600 NiDevImageConverter_CanConvertPixelData. Completion: 100%.
fn dev_converter_can_convert(source: &PixelFormat, dest: &PixelFormat) -> bool {
    let dest_format = dest.format_04;
    let dest_bits = dest.bits_per_pixel_01;
    let masks = channel_masks(dest);
    if pixel_format_equals(source, dest) {
        return true;
    }
    if source.flags_00 & 1 == 0 || dest.flags_00 & 1 == 0 {
        return false;
    }
    if (4..=6).contains(&dest_format) {
        return false;
    }
    if dest_bits == 24
        && (masks.masks[1] != 0xFF00 || (masks.masks[0] != 0xFF && masks.masks[0] != 0xFF_0000))
    {
        return false;
    }
    let bits_ok = dest_bits == 16 || dest_bits == 24 || dest_bits == 32;
    if pixel_format_equals(source, &PAL4_FORMAT) || pixel_format_equals(source, &PALA4_FORMAT) {
        if dest_format < 2 {
            return dest_bits == 24 || dest_bits == 32;
        }
        return pixel_format_equals(dest, &PAL4_FORMAT) || pixel_format_equals(dest, &PALA4_FORMAT);
    }
    if pixel_format_equals(source, &PAL8_FORMAT) || pixel_format_equals(source, &PALA8_FORMAT) {
        if dest_format < 2 {
            return bits_ok;
        }
        return pixel_format_equals(dest, &PAL8_FORMAT) || pixel_format_equals(dest, &PALA8_FORMAT);
    }
    if pixel_format_equals(source, &RGB24_FORMAT)
        || pixel_format_equals(source, &RGBA32_FORMAT)
        || pixel_format_compressed(source)
    {
        if dest_format >= 2 && dest_format != 8 && dest_format != 9 {
            return false;
        }
        return bits_ok;
    }
    if pixel_format_equals(source, &TEXTURE_PIXEL_FORMATS[11])
        || pixel_format_equals(source, &TEXTURE_PIXEL_FORMATS[13])
    {
        if dest_format != 8 && dest_format != 9 {
            return false;
        }
        return bits_ok;
    }
    if (!pixel_format_equals(source, &RGBA5551_FORMAT)
        && !pixel_format_equals(source, &BGRA5551_FORMAT))
        || dest_bits != 16
        || masks.masks[3] != 0x8000
        || masks.masks[1] != 0x3E0
    {
        return false;
    }
    if masks.masks[0] == 0x1F {
        return masks.masks[2] == 0x7C00;
    }
    if masks.masks[0] != 0x7C00 {
        return false;
    }
    masks.masks[2] == 0x1F
}

type CanConvert = fn(&PixelFormat, &PixelFormat) -> bool;

/// IDA: 00773BA0 NiDX9TextureData_FindClosestPixelFormat. Returns the table case or nothing. `table` is renderer +0x6F4;
/// `can_convert` is the current default image converter's vt+0x0C. Completion: 100%.
fn find_closest_pixel_format(
    source: &PixelFormat,
    prefs: [u32; 3],
    table: &[Option<PixelFormat>; 22],
    can_convert: CanConvert,
) -> Option<usize> {
    let first = |order: &[usize]| -> Option<usize> {
        order.iter().copied().find(|&index| {
            table[index]
                .as_ref()
                .is_some_and(|f| can_convert(source, f))
        })
    };
    if source.tiling_08 != 0 {
        return None;
    }
    let layout = prefs[0];
    let alpha = prefs[1];
    let source_format = source.format_04;
    if layout == 4 {
        if source_format == 9 || source_format == 1 {
            return first(&[13, 12, 11]);
        }
        return first(&[11, 13, 12]);
    }
    // 00773E7C..00773FF5: the true-colour candidates for 32/16-bit layouts.
    let true_colour = || -> Option<usize> {
        // 00773870(source, 0): first component's representation, bits, signed.
        let representation = source.components_14[0].representation;
        let bits = source.components_14[0].bits;
        if source.components_14[0].component == 19 || (representation != 2 && representation != 1) {
            return None;
        }
        // 007738C0: number of components before the first empty one (max 4).
        let mut count = 0;
        while count < 4 && source.components_14[count].component != 19 {
            count += 1;
        }
        if bits == 16 {
            if count < 2
                && let Some(found) = first(&[16, 19])
            {
                return Some(found);
            }
            if count < 3
                && let Some(found) = first(&[17, 20])
            {
                return Some(found);
            }
            return first(&[18, 21]);
        }
        if bits == 32 {
            if count < 2
                && let Some(found) = first(&[19])
            {
                return Some(found);
            }
            if count < 3
                && let Some(found) = first(&[20])
            {
                return Some(found);
            }
            if let Some(found) = first(&[21]) {
                return Some(found);
            }
            if count < 2
                && let Some(found) = first(&[16])
            {
                return Some(found);
            }
            if count < 3
                && let Some(found) = first(&[17])
            {
                return Some(found);
            }
            return first(&[18]);
        }
        None
    };
    // 0077405A..007741EE.
    let tail = || -> Option<usize> {
        if layout == 3 {
            let found = if alpha == 1 {
                first(&[8, 9, 10])
            } else {
                first(&[9, 10, 8])
            };
            if found.is_some() {
                return found;
            }
        }
        if layout == 1 {
            if alpha == 1 {
                return first(&[3, 4, 5]);
            }
            return first(&[4, 5, 3]);
        }
        if let Some(found) = first(&[5]) {
            return Some(found);
        }
        if alpha == 1 {
            first(&[3, 4])
        } else {
            first(&[4, 3])
        }
    };
    match source_format {
        0 | 2 => {
            if layout == 3
                && let Some(found) = first(&[8, 9, 10])
            {
                return Some(found);
            }
            let found = if layout == 1 {
                first(&[1, 0, 2, 5])
            } else {
                first(&[2, 5, 1, 0])
            };
            if found.is_some() {
                return found;
            }
            if let Some(found) = first(&[3]) {
                return Some(found);
            }
            return first(&[4]);
        }
        1 | 3 | 12 | 13 | 14 => {}
        4 => {
            if layout != 1
                && layout != 2
                && let Some(found) = first(&[8])
            {
                return Some(found);
            }
        }
        5 => {
            if layout != 1
                && layout != 2
                && let Some(found) = first(&[9])
            {
                return Some(found);
            }
        }
        6 => {
            if layout != 1
                && layout != 2
                && let Some(found) = first(&[10])
            {
                return Some(found);
            }
        }
        8 => return first(&[11, 13, 12]),
        9 => return first(&[12, 13, 11]),
        11 => {
            let found = if alpha == 2 {
                first(&[15])
            } else {
                first(&[14])
            };
            if found.is_some() {
                return found;
            }
        }
        _ => return None,
    }
    true_colour().or_else(tail)
}

/// IDA: N/A `(byte >> right_shift) << shift`, as x86 shifts it: the count is masked to five bits, so a channel wider than 8 bits
/// keeps the original's result.
fn channel_value(byte: u8, right_shift: u8, shift: u8) -> u32 {
    u32::from(byte)
        .wrapping_shr(u32::from(right_shift))
        .wrapping_shl(u32::from(shift))
}

/// IDA: 0071D960/0071D7F0/0071D8D0 RGB24 converters (one row loop per level; width x height pixels of 3 bytes R, G, B).
/// 0071D8D0 only writes for G mask 0xFF00 with R mask 0xFF0000 (swap) or 0xFF (copy); any other 24-bit layout leaves the
/// bytes untouched. Completion: 100%.
fn convert_rgb24(source: &[u8], width: u32, height: u32, dest: &PixelFormat) -> Vec<u8> {
    let m = channel_masks(dest);
    let pixels = width as usize * height as usize;
    let mut out = vec![0u8; pixels * usize::from(dest.bits_per_pixel_01 / 8)];
    if dest.bits_per_pixel_01 == 24 {
        if m.masks[1] == 0xFF00 && m.masks[0] == 0xFF_0000 {
            for (from, to) in source
                .as_chunks::<3>()
                .0
                .iter()
                .zip(out.as_chunks_mut::<3>().0)
                .take(pixels)
            {
                to[0] = from[2];
                to[1] = from[1];
                to[2] = from[0];
            }
        } else if m.masks[1] == 0xFF00 && m.masks[0] == 0xFF {
            out[..3 * pixels].copy_from_slice(&source[..3 * pixels]);
        }
        return out;
    }
    let step = usize::from(dest.bits_per_pixel_01 / 8);
    for (from, to) in source
        .as_chunks::<3>()
        .0
        .iter()
        .zip(out.chunks_exact_mut(step))
        .take(pixels)
    {
        let value = (m.masks[3]
            & (0xFFu32.wrapping_shr(u32::from(m.right_shifts[3])))
                .wrapping_shl(u32::from(m.shifts[3])))
            | (m.masks[2] & channel_value(from[2], m.right_shifts[2], m.shifts[2]))
            | (m.masks[1] & channel_value(from[1], m.right_shifts[1], m.shifts[1]))
            | (m.masks[0] & channel_value(from[0], m.right_shifts[0], m.shifts[0]));
        if dest.bits_per_pixel_01 == 32 {
            to.copy_from_slice(&value.to_le_bytes());
        } else {
            to.copy_from_slice(&(value as u16).to_le_bytes());
        }
    }
    out
}

/// IDA: 0071DA20/0071DB00/0071DBD0 RGBA32 converters (one row loop per level; width x height pixels of 4 bytes R, G, B, A):
/// 16 and 32 bits pack each byte by the destination masks and shifts (the 16-bit form shifts the byte as a byte and the
/// word as a word, which the 32-bit pack truncated to 16 bits reproduces); 24 bits only writes for G mask 0xFF00 with R mask
/// 0xFF0000 (B, G, R from bytes 2, 1, 0) or 0xFF (copy R, G, B). Completion: 100%.
fn convert_rgba32(source: &[u8], width: u32, height: u32, dest: &PixelFormat) -> Vec<u8> {
    let m = channel_masks(dest);
    let pixels = width as usize * height as usize;
    let mut out = vec![0u8; pixels * usize::from(dest.bits_per_pixel_01 / 8)];
    if dest.bits_per_pixel_01 == 24 {
        let swap = m.masks[1] == 0xFF00 && m.masks[0] == 0xFF_0000;
        let copy = m.masks[1] == 0xFF00 && m.masks[0] == 0xFF;
        if swap || copy {
            for (from, to) in source
                .as_chunks::<4>()
                .0
                .iter()
                .zip(out.as_chunks_mut::<3>().0)
                .take(pixels)
            {
                if swap {
                    to[0] = from[2];
                    to[1] = from[1];
                    to[2] = from[0];
                } else {
                    to[0] = from[0];
                    to[1] = from[1];
                    to[2] = from[2];
                }
            }
        }
        return out;
    }
    let step = usize::from(dest.bits_per_pixel_01 / 8);
    for (from, to) in source
        .as_chunks::<4>()
        .0
        .iter()
        .zip(out.chunks_exact_mut(step))
        .take(pixels)
    {
        let value = (m.masks[0] & channel_value(from[0], m.right_shifts[0], m.shifts[0]))
            | (m.masks[2] & channel_value(from[2], m.right_shifts[2], m.shifts[2]))
            | (m.masks[1] & channel_value(from[1], m.right_shifts[1], m.shifts[1]))
            | (m.masks[3] & channel_value(from[3], m.right_shifts[3], m.shifts[3]));
        if dest.bits_per_pixel_01 == 32 {
            to.copy_from_slice(&value.to_le_bytes());
        } else {
            to.copy_from_slice(&(value as u16).to_le_bytes());
        }
    }
    out
}

/// Which state cache a `DeviceStateCall` reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceStateKind {
    RenderState,
    TextureStageState,
    SamplerState,
}

/// IDA: one device state call of the NiDX9RenderState initialization, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceStateCall {
    pub kind: DeviceStateKind,
    pub stage: u32,
    pub state: u32,
    pub value: u32,
}

/// IDA: NiDX9RenderState (0077BB00, 0x1148 bytes) caches. 007802D0 fills every render/stage/sampler cache with 0x7FFFFFFF
/// (0078031B..00780365); flags bit 0 = RasterCaps FOGTABLE && WFOG, bit 1 = z-buffer (0077BB00), bit 2 = TextureAddressCaps
/// BORDER (0077BA10); blend defaults 5/6 (+0x38/+0x3C).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderStateCache {
    pub render_states: [u32; 256],
    pub previous_render_states: [u32; 256],
    pub stage_states: [[u32; 33]; 16],
    pub sampler_states: [[u32; 14]; 16],
    pub flags: u32,
    pub source_blend_38: u32,
    pub destination_blend_3c: u32,
    /// +0x98 packed from 00B3FA90 (zero here).
    pub fog_color_98: u32,
}

impl Default for RenderStateCache {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            render_states: [0x7FFF_FFFF; 256],
            previous_render_states: [0x7FFF_FFFF; 256],
            stage_states: [[0x7FFF_FFFF; 33]; 16],
            sampler_states: [[0x7FFF_FFFF; 14]; 16],
            flags: 0,
            source_blend_38: 5,
            destination_blend_3c: 6,
            fog_color_98: 0,
        }
    }
}

/// IDA: 00B2A7C0 g_niDX9DefaultTextureStageStates is written back by 0077FF40 (COLOROP/ALPHAOP -> DISABLE for stages > 0);
/// the host keeps the mutable copy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderStateDefaults {
    pub render_states: Vec<(u32, u32)>,
    pub texture_stage_states: Vec<(u32, u32)>,
    pub sampler_states: Vec<(u32, u32)>,
}

/// IDA: N/A a fresh copy of the 00B29FB8, 00B2A7C0 and 00B2A808 tables.
#[must_use]
pub fn original_render_state_defaults() -> RenderStateDefaults {
    RenderStateDefaults {
        render_states: DEFAULT_RENDER_STATES.to_vec(),
        texture_stage_states: DEFAULT_TEXTURE_STAGE_STATES.to_vec(),
        sampler_states: DEFAULT_SAMPLER_STATES.to_vec(),
    }
}

/// IDA: 0077B000 cached SetRenderState: a call only for a changed value, the cache updated either way. Completion: 100%.
fn cached_render_state(
    calls: &mut Vec<DeviceStateCall>,
    cache: &mut RenderStateCache,
    state: u32,
    value: u32,
) {
    if cache.render_states[state as usize] != value {
        calls.push(DeviceStateCall {
            kind: DeviceStateKind::RenderState,
            stage: 0,
            state,
            value,
        });
    }
    cache.render_states[state as usize] = value;
}

/// IDA: 0077BB00 NiDX9RenderState_Create then 00778F60 -> NiDX9RenderState_UpdateRenderState 0077B7B0 ->
/// 0077FE50/0077FF40/cached sets, then 007766E0 -> NiDX9LightManager_InitializeRenderStates 00776240. Returns the device
/// calls in order and updates the caches and defaults exactly as the original. 0077B6E0, 0077B210 and 00780A20 are host-only.
/// Completion: 100% of the P04-RUN-001 sequence (103 + 8x(7+5) + 8 + 2).
pub fn render_state_initialization_calls(
    defaults: &mut RenderStateDefaults,
    cache: &mut RenderStateCache,
    caps: &D3dCaps9,
    max_texture_blend_stages: u32,
    z_buffer: bool,
) -> Vec<DeviceStateCall> {
    let mut calls = Vec::new();
    // 0077BA10 (vt+0x108 of 0077BB00).
    if caps.raster_caps & D3DPRASTERCAPS_FOGTABLE != 0
        && caps.raster_caps & D3DPRASTERCAPS_WFOG != 0
    {
        cache.flags |= 1;
    } else {
        cache.flags &= !1;
    }
    if caps.texture_address_caps & D3DPTADDRESSCAPS_BORDER != 0 {
        cache.flags |= 4;
    } else {
        cache.flags &= !4;
    }
    // 0077BB5D..0077BBCE: ZENABLE in the static table = z-buffer flag.
    for (state, value) in &mut defaults.render_states {
        if *state == D3DRS_ZENABLE {
            *value = u32::from(z_buffer);
        }
    }
    if z_buffer {
        cache.flags |= 2;
    }
    // 0077B7B0 -> 0077FE50: every pair, uncached.
    for &(state, value) in &defaults.render_states {
        cache.render_states[state as usize] = value;
        cache.previous_render_states[state as usize] = value;
        calls.push(DeviceStateCall {
            kind: DeviceStateKind::RenderState,
            stage: 0,
            state,
            value,
        });
    }
    // 0077FF40: stages and samplers (the host cache holds 16 stages, the D3D9 maximum for MaxTextureBlendStages).
    let stages = max_texture_blend_stages.min(16);
    for stage in 0..stages {
        for (state, value) in &mut defaults.texture_stage_states {
            if *state == D3DTSS_TEXCOORDINDEX {
                *value = stage;
            } else if (*state == D3DTSS_COLOROP || *state == D3DTSS_ALPHAOP) && stage != 0 {
                *value = D3DTOP_DISABLE;
            }
            cache.stage_states[stage as usize][*state as usize] = *value;
            calls.push(DeviceStateCall {
                kind: DeviceStateKind::TextureStageState,
                stage,
                state: *state,
                value: *value,
            });
        }
        for &(state, value) in &defaults.sampler_states {
            let slot = &mut cache.sampler_states[stage as usize][state as usize];
            *slot = value.wrapping_add(1);
            // 0077B610 cached set: differs by construction.
            if *slot != value {
                *slot = value;
                calls.push(DeviceStateCall {
                    kind: DeviceStateKind::SamplerState,
                    stage,
                    state,
                    value,
                });
            }
        }
    }
    // 0077B7F6..0077B93B cached sets.
    let source_blend = cache.source_blend_38;
    let destination_blend = cache.destination_blend_3c;
    let fog_table = if cache.flags & 1 != 0 { 3 } else { 0 };
    for (state, value) in [
        (D3DRS_CULLMODE, 2),
        (D3DRS_FILLMODE, 3),
        (D3DRS_SHADEMODE, 2),
        (D3DRS_ALPHABLENDENABLE, 0),
        (D3DRS_SRCBLEND, source_blend),
        (D3DRS_DESTBLEND, destination_blend),
        (D3DRS_ALPHATESTENABLE, 0),
        (D3DRS_ALPHAFUNC, 8),
        (D3DRS_ALPHAREF, 0),
        (D3DRS_ZFUNC, 8),
        (D3DRS_ZWRITEENABLE, 0),
        (D3DRS_DITHERENABLE, 0),
        (D3DRS_SPECULARENABLE, 0),
        (D3DRS_FOGENABLE, 0),
        (D3DRS_FOGTABLEMODE, fog_table),
        (D3DRS_FOGVERTEXMODE, 3),
        (D3DRS_RANGEFOGENABLE, 0),
        (D3DRS_FOGCOLOR, 0),
        (D3DRS_NORMALIZENORMALS, 1),
        (D3DRS_ZENABLE, 1),
        (D3DRS_COLORVERTEX, 1),
    ] {
        cached_render_state(&mut calls, cache, state, value);
    }
    // 0077B946..0077B9FF: fog colour from 00B3FA90 (0, 0, 0 here).
    cache.fog_color_98 = 0xFF00_0000;
    // 007766E0 -> 00776240 NiDX9LightManager_InitializeRenderStates.
    for (state, value) in [
        (D3DRS_AMBIENT, 0),
        (D3DRS_LIGHTING, 0),
        (D3DRS_EMISSIVEMATERIALSOURCE, 0),
        (D3DRS_AMBIENTMATERIALSOURCE, 0),
        (D3DRS_DIFFUSEMATERIALSOURCE, 0),
    ] {
        cached_render_state(&mut calls, cache, state, value);
    }
    calls
}

/// IDA: 00762640 NiDX9Renderer_SetCameraData results.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraData {
    pub view: D3dMatrix,
    pub inverse_view: D3dMatrix,
    pub projection: D3dMatrix,
    pub viewport: D3dViewport,
    pub near_plane: f32,
    pub far_minus_near: f32,
}

/// IDA: 00762640 NiDX9Renderer_SetCameraData. 24-bit x87 site: the device was created without D3DCREATE_FPU_PRESERVE, so
/// control word 007F makes every operation round to float (P04-RUN-002, 0049851E) and plain `f32` arithmetic reproduces it;
/// the extended exponent range only matters for overflow and underflow, which these values cannot reach. Completion: 100%.
#[must_use]
#[allow(clippy::too_many_arguments)]
#[allow(clippy::many_single_char_names)]
pub fn camera_data(
    location: [f32; 3],
    direction: [f32; 3],
    up: [f32; 3],
    right: [f32; 3],
    frustum: &CameraFrustum,
    port: &CameraPort,
    left_right_swap: bool,
    target_width: u32,
    target_height: u32,
) -> CameraData {
    // 00762729..0076277A: -(axis . location), summed x, y, z.
    let dot = |axis: [f32; 3]| {
        let xy = axis[0] * location[0] + axis[1] * location[1];
        xy + location[2] * axis[2]
    };
    let view = D3dMatrix {
        _11: right[0],
        _12: up[0],
        _13: direction[0],
        _14: 0.0,
        _21: right[1],
        _22: up[1],
        _23: direction[1],
        _24: 0.0,
        _31: right[2],
        _32: up[2],
        _33: direction[2],
        _34: 0.0,
        _41: -dot(right),
        _42: -dot(up),
        _43: -dot(direction),
        _44: 1.0,
    };
    let inverse_view = D3dMatrix {
        _11: right[0],
        _12: right[1],
        _13: right[2],
        _14: 0.0,
        _21: up[0],
        _22: up[1],
        _23: up[2],
        _24: 0.0,
        _31: direction[0],
        _32: direction[1],
        _33: direction[2],
        _34: 0.0,
        _41: location[0],
        _42: location[1],
        _43: location[2],
        _44: 1.0,
    };
    // 00762884..007628D7: differences, sums and the reciprocal.
    let far_minus_near = frustum.far_plane - frustum.near_plane;
    let rl = frustum.right - frustum.left;
    let rpl = frustum.right + frustum.left;
    let tb = frustum.top - frustum.bottom;
    let tpb = frustum.bottom + frustum.top;
    let inverse = 1.0f32 / far_minus_near;
    let signed_rpl = if left_right_swap { rpl } else { -rpl };
    let mut projection = D3dMatrix {
        _11: (if left_right_swap { -2.0f32 } else { 2.0 }) / rl,
        _22: 2.0f32 / tb,
        ..D3dMatrix::default()
    };
    if frustum.orthographic {
        // 007628E1..00762996.
        projection._41 = signed_rpl / rl;
        projection._42 = -tpb / tb;
        projection._33 = inverse;
        projection._43 = -(inverse * frustum.near_plane);
        projection._44 = 1.0;
    } else {
        // 0076299D..00762A56.
        projection._31 = signed_rpl / rl;
        projection._32 = -tpb / tb;
        projection._33 = frustum.far_plane * inverse;
        projection._43 = -(inverse * (frustum.far_plane * frustum.near_plane));
        projection._34 = 1.0;
    }
    // 00762A76..00762BAA: RT group width and height as floats, truncating fistp.
    let width = target_width as f32;
    let height = target_height as f32;
    let viewport = D3dViewport {
        x: x87::ftol64_low(port.left * width),
        y: x87::ftol64_low((1.0f32 - port.top) * height),
        width: x87::ftol64_low((port.right - port.left) * width),
        height: x87::ftol64_low((port.top - port.bottom) * height),
        min_z: 0.0,
        max_z: 1.0,
    };
    CameraData {
        view,
        inverse_view,
        projection,
        viewport,
        near_plane: frustum.near_plane,
        far_minus_near,
    }
}

/// IDA: 007623D0 NiDX9Renderer_ExpandDeviceType: (device type, behaviour flags). Completion: 100%.
#[must_use]
pub const fn expand_device_type(device_desc: i32) -> Option<(u32, u32)> {
    match device_desc {
        0 => Some((
            D3DDEVTYPE_HAL,
            D3DCREATE_PUREDEVICE | D3DCREATE_HARDWARE_VERTEXPROCESSING,
        )),
        1 => Some((D3DDEVTYPE_HAL, D3DCREATE_HARDWARE_VERTEXPROCESSING)),
        2 => Some((D3DDEVTYPE_HAL, D3DCREATE_MIXED_VERTEXPROCESSING)),
        3 => Some((D3DDEVTYPE_HAL, D3DCREATE_SOFTWARE_VERTEXPROCESSING)),
        4 => Some((D3DDEVTYPE_REF, D3DCREATE_SOFTWARE_VERTEXPROCESSING)),
        5 => Some((D3DDEVTYPE_REF, D3DCREATE_HARDWARE_VERTEXPROCESSING)),
        6 => Some((D3DDEVTYPE_REF, D3DCREATE_MIXED_VERTEXPROCESSING)),
        _ => None,
    }
}

/// IDA: 00761CE0. Completion: 100%.
#[must_use]
pub const fn renderer_multisample_type(frame_buffer_mode: i32) -> u32 {
    if frame_buffer_mode < 0 {
        D3DMULTISAMPLE_NONMASKABLE
    } else if frame_buffer_mode >= 2 && frame_buffer_mode <= 16 {
        frame_buffer_mode as u32
    } else {
        D3DMULTISAMPLE_NONE
    }
}

/// IDA: 00761DA0. Completion: 100%.
#[must_use]
pub const fn renderer_present_interval(presentation_interval: i32) -> u32 {
    match presentation_interval {
        0 => D3DPRESENT_INTERVAL_IMMEDIATE,
        1 => D3DPRESENT_INTERVAL_ONE,
        2 => D3DPRESENT_INTERVAL_TWO,
        3 => D3DPRESENT_INTERVAL_THREE,
        4 => D3DPRESENT_INTERVAL_FOUR,
        _ => D3DPRESENT_INTERVAL_DEFAULT,
    }
}

/// IDA: 00B42070 (0x76 bytes): a newtype, because an array this long has no `Default`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatBitsTable(pub [u8; 0x76]);

impl Default for FormatBitsTable {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self([0; 0x76])
    }
}

/// IDA: values the NiDX9Renderer (0xB00) and the NiDX9 statics hold after creation that later phases read. Offsets are
/// object offsets. `background_color_5d4` is 0 only until 007641A0 runs.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct RendererState {
    pub create_width_a58: u32,
    pub create_height_a5c: u32,
    pub create_use_flags_a60: u32,
    pub create_device_window_a64: Option<WindowHandle>,
    pub create_focus_window_a68: Option<WindowHandle>,
    pub create_adapter_a6c: u32,
    pub create_device_desc_a70: i32,
    pub create_frame_buffer_format_a74: i32,
    pub create_depth_stencil_format_a78: i32,
    pub create_presentation_interval_a7c: i32,
    pub create_swap_effect_a80: i32,
    pub create_frame_buffer_mode_a84: u32,
    pub create_back_buffer_count_a88: u32,
    pub create_refresh_rate_a8c: u32,
    pub device_window_3b4: Option<WindowHandle>,
    pub focus_window_3b8: Option<WindowHandle>,
    pub adapter_5bc: u32,
    pub device_type_5c0: u32,
    pub behavior_flags_5c4: u32,
    pub software_vertex_processing_5c8: bool,
    pub mixed_vertex_processing_5c9: bool,
    pub caps_284: D3dCaps9,
    pub background_color_5d4: u32,
    pub texture_caps_flags_5e0: u32,
    pub max_vertex_blend_matrices_6d8: u32,
    pub max_streams_6dc: u32,
    pub pixel_shader_version_6e0: u32,
    pub vertex_shader_version_6e4: u32,
    pub mipmap_cube_maps_6e8: bool,
    pub dynamic_textures_6e9: bool,
    pub max_render_targets_894: u32,
    pub independent_bit_depths_898: bool,
    pub mrt_post_pixel_shader_blending_899: bool,
    pub shader_version_table_914: [u32; 10],
    /// (100h)/(100w) != 75.
    pub non_standard_aspect_a90: bool,
    /// +0x6F4/+0x74C/+0x7A4/+0x7FC: TEXTURE, TEXTURE (unread RT flag 1), CUBETEXTURE, CUBETEXTURE (flag 1).
    pub pixel_format_tables: [[Option<PixelFormat>; 22]; 4],
    /// +0x854..+0x860, as the table case.
    pub default_formats_854: [Option<usize>; 4],
    /// +0x874 (+0x10 of the table-0 default).
    pub default_format_index_874: u32,
    pub back_buffer_desc: SurfaceDesc,
    pub depth_stencil_desc: SurfaceDesc,
    /// NiDX9ImplicitBufferData.
    pub back_buffer: Option<SurfaceHandle>,
    /// 00B42610.
    pub depth_stencil: Option<SurfaceHandle>,
    /// +0x8F0 NiSourceTexture.
    pub default_texture_8f0: Option<TextureHandle>,
    pub state_defaults: RenderStateDefaults,
    pub state_cache: RenderStateCache,
    /// Host record of the issued calls.
    pub state_calls: Vec<DeviceStateCall>,
    pub camera: CameraData,
    /// 00B28CB0.
    pub max_texture_blend_stages: u32,
    /// 00B28CB4.
    pub max_simultaneous_textures: u32,
    /// 00B28CB8.
    pub max_sampler_stages: u32,
    /// 00B420E8..00B4214C.
    pub texture_stage_globals: [u32; 26],
    /// 00B4215C.
    pub default_texture_transform_flags: u32,
    /// 00B420E6.
    pub nvperfhud: bool,
    /// 00B42070 (007641A0).
    pub format_bits: FormatBitsTable,
}

/// IDA: N/A the arguments of 00498413 CreateWindowExA that are not constants of the seam (a null title, menu and lpParam).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateWindowRequest {
    pub ex_style: u32,
    /// A host class (recorded substitution); the original passes the main window's "Oblivion" class.
    pub class_name: String,
    pub style: u32,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub parent: Option<WindowHandle>,
    pub instance: u32,
}

/// IDA: N/A the host seam for the Win32 calls 00498340 and its callees make, and for the `fopen`/`fprintf` output of
/// RendererInfo.txt. Results are scriptable, so every failure branch stays reachable.
pub trait RendererPlatform {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn load_library(&mut self, name: &str) -> Option<ModuleHandle>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn get_proc_address(&mut self, module: ModuleHandle, name: &str) -> Option<ProcHandle>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn free_library(&mut self, module: ModuleHandle) -> bool;
    /// IDA: 00763E22/00761E2F calling the resolved Direct3DCreate9.
    fn direct3d_create9(
        &mut self,
        entry: ProcHandle,
        sdk_version: u32,
    ) -> Option<Box<dyn Direct3D9>>;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn create_window(&mut self, request: &CreateWindowRequest) -> Option<WindowHandle>;
    /// IDA: 0049843D GetClassLongA(child, GCL_MENUNAME).
    fn class_long(&mut self, window: Option<WindowHandle>, index: i32) -> u32;
    /// IDA: 0049844D GetWindowLongA(main, GWL_STYLE).
    fn window_long(&mut self, window: Option<WindowHandle>, index: i32) -> u32;
    /// IDA: 00498459 AdjustWindowRect: the rect after the call, `[left, top, right, bottom]`.
    fn adjust_window_rect(&mut self, rect: [i32; 4], style: u32, menu: bool) -> [i32; 4];
    /// IDA: 00498488 SetWindowPos(main, NULL, x, y, cx, cy, flags).
    fn set_window_pos(
        &mut self,
        window: Option<WindowHandle>,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> bool;
    /// IDA: 004988B6 calling the resolved AtiQueryMgpuCount.
    fn ati_query_mgpu_count(&mut self, entry: ProcHandle) -> i32;
    /// IDA: 0049892B/00498934 calling the resolved NvCplGetDataInt(key, &value): the value after the call.
    fn nv_cpl_get_data_int(&mut self, entry: ProcHandle, key: i32) -> i32;
    /// IDA: 00498AA3 fopen(path, "w") and the fprintf lines. `text` has LF line ends: the CRT's text mode turns them into
    /// CR LF (`crt_text_mode_output`). Answers whether the file opened.
    fn write_renderer_info(&mut self, path: &Path, text: &[u8]) -> bool;
}

/// IDA: N/A what the CRT's "w" text mode writes for `text`: each LF becomes CR LF.
#[must_use]
pub fn crt_text_mode_output(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() + 32);
    for &byte in text {
        if byte == b'\n' {
            out.push(b'\r');
        }
        out.push(byte);
    }
    out
}

/// IDA: 0076A1C0 inputs and outputs captured for the run report and fixtures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceCreationRecord {
    pub adapter: u32,
    pub device_type: u32,
    pub behavior_flags: u32,
    pub focus_window: Option<WindowHandle>,
    pub parameters: PresentParameters,
    pub result: u32,
    pub attempts: u32,
}

impl Default for DeviceCreationRecord {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            adapter: 0,
            device_type: 0,
            behavior_flags: 0,
            focus_window: None,
            parameters: PresentParameters::default(),
            result: E_FAIL,
            attempts: 0,
        }
    }
}

/// Result of 004052F0 / 00498340, and the renderer's owner from then on.
#[derive(Resource, Default)]
pub struct RendererCreation {
    pub succeeded: bool,
    /// 00B42150.
    pub d3d9_module: Option<ModuleHandle>,
    /// 00B42154, the persistent interface.
    pub d3d9: Option<Box<dyn Direct3D9>>,
    /// 00B42158, the entry point.
    pub d3d9_create: Option<ProcHandle>,
    /// Renderer +0x280.
    pub device: Option<Box<dyn Direct3DDevice9>>,
    /// The 00498413 child (windowed).
    pub child_window: Option<WindowHandle>,
    pub parameters: PresentParameters,
    /// 00B34FC4 / 00763DE0 singleton.
    pub system: Option<Arc<Dx9SystemDesc>>,
    /// 00B3F828 g_niDX9RendererErrorString.
    pub renderer_error: String,
    pub globals: RendererGlobals,
    pub state: RendererState,
    pub creation: DeviceCreationRecord,
    /// Host record of the steps.
    pub trace: Vec<String>,
    /// Every device and Win32 call, in original order.
    pub device_calls: Vec<DeviceTraceEntry>,
    /// 00498614..00498647: NiDevImageConverter became the default converter. The caller mirrors it into
    /// `NiProcessDefaults` with `set_device_image_converter_default`.
    pub device_image_converter_default: bool,
    pub renderer_info_path: PathBuf,
    /// The exact text with LF line ends; empty when the file could not be opened.
    pub renderer_info_text: Vec<u8>,
    pub renderer_info_written: bool,
    /// 004052F0: shown text when the creation failed (box then ExitProcess(0)).
    pub failure_message: String,
}

/// IDA: N/A the inputs 00498340 reads from globals.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RendererCreationInputs {
    pub main_window: Option<WindowHandle>,
    pub instance: u32,
    /// Host class (recorded substitution).
    pub window_class: String,
    /// 00B3F280 g_oblivionMyGamesPath.
    pub my_games_root: PathBuf,
    /// g_dx9AdapterFormats (Phase 03).
    pub adapter_formats: Vec<u32>,
    pub settings: RendererSettings,
}

pub(crate) struct RendererContext<'a> {
    pub(crate) inputs: &'a RendererCreationInputs,
    pub(crate) platform: &'a mut dyn RendererPlatform,
    pub(crate) result: &'a mut RendererCreation,
}

impl RendererContext<'_> {
    // IDA: N/A no address of its own; its comments cite 00B3F828.
    fn error(&mut self, text: &str) {
        // 00B3F828 strncpy_s(.., 0x100, text, 0xFF). The texts are ASCII.
        self.result.renderer_error = text[..text.len().min(0xFF)].to_owned();
    }

    // IDA: N/A a helper, with no address of its own.
    pub(crate) fn step(&mut self, step: &str) {
        self.result.trace.push(step.to_owned());
    }

    // IDA: N/A a helper, with no address of its own.
    pub(crate) fn call(&mut self, site: u32, call: DeviceCall, result: u32) {
        self.result
            .device_calls
            .push(DeviceTraceEntry { site, call, result });
    }

    // IDA: N/A a helper, with no address of its own.
    pub(crate) fn load_library(&mut self, site: u32, name: &str) -> Option<ModuleHandle> {
        let module = self.platform.load_library(name);
        self.call(
            site,
            DeviceCall::LoadLibrary {
                name: name.to_owned(),
            },
            module.map_or(0, |m| m.0),
        );
        module
    }

    // IDA: N/A a helper, with no address of its own.
    pub(crate) fn get_proc_address(
        &mut self,
        site: u32,
        module: ModuleHandle,
        name: &str,
    ) -> Option<ProcHandle> {
        let entry = self.platform.get_proc_address(module, name);
        self.call(
            site,
            DeviceCall::GetProcAddress {
                name: name.to_owned(),
            },
            entry.map_or(0, |e| e.0),
        );
        entry
    }

    // IDA: N/A a helper, with no address of its own.
    pub(crate) fn free_library(&mut self, site: u32, module: ModuleHandle) {
        let freed = self.platform.free_library(module);
        self.call(site, DeviceCall::FreeLibrary, u32::from(freed));
    }

    /// IDA: see the words here, which name the addresses this relates to.
    /// The shared head of 00763DE0 and 00761DF0: load D3D9.DLL when the cached creator is null, then call the creator.
    fn create_d3d9(
        &mut self,
        load_site: u32,
        resolve_site: u32,
        create_site: u32,
    ) -> Option<Box<dyn Direct3D9>> {
        if self.result.d3d9_create.is_none() {
            let module = self.load_library(load_site, "D3D9.DLL");
            self.result.d3d9_module = module;
            let module = module?;
            let entry = self.get_proc_address(resolve_site, module, "Direct3DCreate9");
            self.result.d3d9_create = entry;
            entry?;
        }
        let entry = self.result.d3d9_create?;
        let d3d9 = self.platform.direct3d_create9(entry, D3D_SDK_VERSION);
        self.call(
            create_site,
            DeviceCall::Direct3DCreate9 {
                sdk_version: D3D_SDK_VERSION,
            },
            u32::from(d3d9.is_some()),
        );
        d3d9
    }
}

/// IDA: 00763DE0 NiDX9AdapterEnumerator_GetOrCreate: on the first call load D3D9.DLL when the cached Direct3DCreate9 pointer
/// 00B42158 is null, Direct3DCreate9(32), FormHeap(0x14) enumerator over the shared adapter-format list, then Release the
/// interface; a failure leaves the singleton null and is retried on the next call. Completion: 100% of the calls.
pub(crate) fn adapter_enumerator(ctx: &mut RendererContext<'_>) -> Option<Arc<Dx9SystemDesc>> {
    if let Some(system) = &ctx.result.system {
        return Some(system.clone());
    }
    let mut d3d9 = ctx.create_d3d9(0x0076_3DFB, 0x0076_3E10, 0x0076_3E22)?;
    let system = enumerate_dx9_system(
        d3d9.as_mut(),
        &ctx.inputs.adapter_formats,
        &mut ctx.result.device_calls,
    );
    ctx.result.system = Some(system.clone());
    drop(d3d9);
    ctx.call(0x0076_3E50, DeviceCall::ReleaseD3d9, 0);
    ctx.step("00763DE0 adapter enumerator");
    Some(system)
}

/// IDA: 00761DF0 D3D9_EnsureInterface: 0 on success, -1 on failure. Completion: 100%.
fn ensure_d3d9(ctx: &mut RendererContext<'_>) -> i32 {
    if ctx.result.d3d9.is_some() {
        return 0;
    }
    match ctx.create_d3d9(0x0076_1E09, 0x0076_1E1E, 0x0076_1E2F) {
        Some(d3d9) => {
            ctx.result.d3d9 = Some(d3d9);
            0
        }
        None => -1,
    }
}

/// IDA: 0076A36C..0076A378 test caps.DeviceType before the pointer; the host tests the pointer first (recorded: a null
/// descriptor faults there).
fn valid_device(adapter: &Dx9AdapterDesc, device_type: u32) -> Option<&Dx9DeviceDesc> {
    let desc = if device_type == D3DDEVTYPE_HAL {
        adapter.hal_460.as_ref()
    } else {
        adapter.reference_464.as_ref()
    };
    desc.filter(|d| d.caps_004.device_type != 0)
}

/// IDA: 00761E60 NiDX9Renderer_InitPresentParams inputs: the pure part.
#[derive(Clone, Copy)]
struct PresentParameterInputs {
    device_window: Option<WindowHandle>,
    width: u32,
    height: u32,
    use_flags: u32,
    frame_buffer_mode: u32,
    back_buffer_count: u32,
    swap_effect: i32,
    refresh_rate: u32,
    presentation_interval: i32,
}

/// IDA: 00761E60 NiDX9Renderer_InitPresentParams. Completion: 100%.
fn init_present_params(
    ctx: &mut RendererContext<'_>,
    adapter: &Dx9AdapterDesc,
    device: &Dx9DeviceDesc,
    input: &PresentParameterInputs,
    frame_buffer_format: i32,
    depth_stencil_format: u32,
) -> Option<PresentParameters> {
    let windowed = input.use_flags & 4 == 0;
    let bits = if input.use_flags & 0x10 != 0 { 16 } else { 32 };
    let mut format = frame_buffer_format;
    if format == 0 {
        format = dx9_find_frame_buffer_format(device, windowed, bits);
        if format == 0 {
            ctx.error("Creation failed: Could not find desired framebuffer format");
            return None;
        }
    }
    let back_buffer = dx9_frame_buffer_format(format);
    let mut depth = depth_stencil_format;
    if depth == D3DFMT_UNKNOWN {
        depth = dx9_find_depth_stencil_format(
            device,
            back_buffer,
            back_buffer,
            bits,
            (input.use_flags & 8) as i32,
        );
        if depth == D3DFMT_UNKNOWN {
            ctx.error("Creation failed: Could not find desired depth/stencil format");
            return None;
        }
    }
    let multisample = renderer_multisample_type(input.frame_buffer_mode as i32);
    let swap_effect = if input.frame_buffer_mode <= 1 && input.swap_effect != 1 {
        match input.swap_effect {
            2 => D3DSWAPEFFECT_FLIP,
            3 => D3DSWAPEFFECT_COPY,
            _ => D3DSWAPEFFECT_DISCARD,
        }
    } else {
        D3DSWAPEFFECT_DISCARD
    };
    let mut pp = PresentParameters {
        back_buffer_format: back_buffer,
        back_buffer_width: input.width,
        back_buffer_height: input.height,
        multi_sample_type: multisample,
        multi_sample_quality: if multisample == D3DMULTISAMPLE_NONMASKABLE {
            input.frame_buffer_mode & 0x7FFF_FFFF
        } else {
            0
        },
        auto_depth_stencil_format: depth,
        back_buffer_count: input.back_buffer_count,
        enable_auto_depth_stencil: input.use_flags & 1 == 0,
        windowed,
        swap_effect,
        full_screen_refresh_rate: input.refresh_rate,
        device_window: input.device_window,
        flags: u32::from(input.frame_buffer_mode == 1),
        presentation_interval: renderer_present_interval(input.presentation_interval),
    };
    let state_adapter = ctx.result.state.adapter_5bc;
    let state_type = ctx.result.state.device_type_5c0;
    let d3d9 = ctx
        .result
        .d3d9
        .as_mut()
        .expect("0076BD40 runs 00761DF0 first");
    let (result, quality) = d3d9.check_device_multi_sample_type(
        state_adapter,
        state_type,
        back_buffer,
        windowed,
        multisample,
    );
    ctx.call(
        0x0076_2032,
        DeviceCall::CheckDeviceMultiSampleType {
            adapter: state_adapter,
            device_type: state_type,
            format: back_buffer,
            windowed,
            multi_sample_type: multisample,
        },
        result,
    );
    if failed(result) {
        pp.multi_sample_type = D3DMULTISAMPLE_NONE;
    } else if quality <= pp.multi_sample_quality {
        pp.multi_sample_quality = quality.wrapping_sub(1);
    }
    pp.back_buffer_count = pp.back_buffer_count.clamp(1, 3);
    if pp.windowed {
        pp.full_screen_refresh_rate = 0;
    } else if !dx9_match_fullscreen_mode(
        adapter,
        pp.back_buffer_format,
        pp.back_buffer_width,
        pp.back_buffer_height,
        &mut pp.full_screen_refresh_rate,
    ) {
        ctx.error("Creation failed: Could not match desired fullscreen mode");
        return None;
    }
    if pp.windowed {
        pp.presentation_interval = D3DPRESENT_INTERVAL_IMMEDIATE;
    } else if device.caps_004.presentation_intervals & pp.presentation_interval == 0 {
        pp.presentation_interval = D3DPRESENT_INTERVAL_ONE;
    }
    Some(pp)
}

/// IDA: 00762110 NiDX9Renderer_InitializeDeviceCaps. Completion: 100%.
fn initialize_device_caps(ctx: &mut RendererContext<'_>, pp: &PresentParameters) -> bool {
    let (result, caps) = ctx
        .result
        .device
        .as_mut()
        .expect("00762110 runs after CreateDevice")
        .device_caps();
    ctx.call(0x0076_2126, DeviceCall::DeviceGetDeviceCaps, result);
    if failed(result) {
        ctx.error(NO_DEVICE_CAPS);
        return false;
    }
    ctx.result.state.caps_284 = caps;
    if caps.max_texture_blend_stages < 2 || caps.max_simultaneous_textures < 2 {
        ctx.error(NO_MULTI_TEXTURE);
        return false;
    }
    let state = &mut ctx.result.state;
    state.texture_caps_flags_5e0 = 0;
    if caps.texture_caps & D3DPTEXTURECAPS_POW2 != 0 {
        if caps.texture_caps & D3DPTEXTURECAPS_NONPOW2CONDITIONAL != 0 {
            state.texture_caps_flags_5e0 = 4;
        }
    } else {
        state.texture_caps_flags_5e0 = 8;
    }
    // +0x5C8/+0x5C9 still hold the 007641A0 zeros here (set after return).
    state.max_vertex_blend_matrices_6d8 = caps.max_vertex_blend_matrices;
    if state.software_vertex_processing_5c8 && !state.mixed_vertex_processing_5c9 {
        state.max_vertex_blend_matrices_6d8 = 4;
    }
    if state.max_vertex_blend_matrices_6d8 >= 4 {
        state.texture_caps_flags_5e0 |= 2;
    }
    state.mipmap_cube_maps_6e8 = caps.texture_caps & D3DPTEXTURECAPS_MIPCUBEMAP != 0;
    state.dynamic_textures_6e9 = caps.caps2 & D3DCAPS2_DYNAMICTEXTURES != 0;
    state.max_streams_6dc = caps.max_streams;
    state.pixel_shader_version_6e0 = caps.pixel_shader_version;
    state.vertex_shader_version_6e4 = caps.vertex_shader_version;
    state.max_render_targets_894 = if pp.multi_sample_type == D3DMULTISAMPLE_NONE {
        caps.num_simultaneous_rts
    } else {
        1
    };
    state.independent_bit_depths_898 =
        caps.primitive_misc_caps & D3DPMISCCAPS_MRTINDEPENDENTBITDEPTHS != 0;
    state.mrt_post_pixel_shader_blending_899 =
        caps.primitive_misc_caps & D3DPMISCCAPS_MRTPOSTPIXELSHADERBLENDING != 0;
    state.shader_version_table_914 = [
        caps.pixel_shader_version,
        0xFFFF_0000,
        caps.pixel_shader_version,
        caps.vertex_shader_version,
        0xFFFE_0000,
        caps.vertex_shader_version,
        0xFFFE_0000,
        0xFFFE_0000,
        0xFFFE_0000,
        2,
    ];
    state.max_texture_blend_stages = caps.max_texture_blend_stages; // 0075FA40
    state.max_simultaneous_textures = caps.max_simultaneous_textures; // 0075FA50
    // 0075FA60
    state.max_sampler_stages = if state.pixel_shader_version_6e0 < 0xFFFF_0200 {
        caps.max_texture_blend_stages
    } else {
        16
    };
    // 007622D6..0076236C: 00B420E8..00B4214C in store order.
    let mut store = |address: u32, value: u32| {
        state.texture_stage_globals[((address - 0x00B4_20E8) / 4) as usize] = value;
    };
    store(0x00B4_2144, 3);
    store(0x00B4_2138, 3);
    store(0x00B4_2130, 3);
    store(0x00B4_2134, 3);
    store(0x00B4_20F0, 0);
    store(0x00B4_20FC, 0);
    for address in [
        0x00B4_2148,
        0x00B4_214C,
        0x00B4_2140,
        0x00B4_213C,
        0x00B4_20EC,
        0x00B4_20E8,
    ] {
        store(address, 1);
    }
    for address in [
        0x00B4_20F8,
        0x00B4_20F4,
        0x00B4_2104,
        0x00B4_2100,
        0x00B4_2108,
    ] {
        store(address, 2);
    }
    for address in [
        0x00B4_2110,
        0x00B4_210C,
        0x00B4_2114,
        0x00B4_211C,
        0x00B4_2118,
    ] {
        store(address, 1);
    }
    for address in [0x00B4_2120, 0x00B4_2128, 0x00B4_2124] {
        store(address, 2);
    }
    store(0x00B4_212C, 1);
    state.default_texture_transform_flags = if caps.texture_caps & D3DPTEXTURECAPS_PROJECTED != 0 {
        0x103
    } else {
        2
    };
    true
}

/// IDA: 007637D0 NiDX9Renderer_InitializeTextureFormatTable: each case's static copied, +0xC = 0076BEF0 format, +0x10 =
/// case, then CheckDeviceFormat(+0x5BC adapter, +0x5C0 type, adapter format, 0, type, fmt) keeps only D3D_OK. The adapter
/// format is +0xC of the render target group's buffer 0 pixel format (009A1F80), which 0076C6D0 -> 0076C3B0 built from the
/// back-buffer D3DFORMAT and stores unchanged at +0xC. The second argument (0/1) is unread. Completion: 100%.
fn initialize_texture_format_table(
    ctx: &mut RendererContext<'_>,
    table: usize,
    resource_type: u32,
) {
    let adapter_format = ctx.result.state.back_buffer_desc.format;
    let adapter = ctx.result.state.adapter_5bc;
    let device_type = ctx.result.state.device_type_5c0;
    for (index, row) in TEXTURE_PIXEL_FORMATS.iter().enumerate() {
        if index == 6 || index == 7 {
            continue;
        }
        let mut format = *row;
        format.renderer_hint_0c = pixel_format_to_d3d(&format);
        format.extra_10 = index as u32;
        ctx.result.state.pixel_format_tables[table][index] = None;
        let result = ctx
            .result
            .d3d9
            .as_mut()
            .expect("0076BD40 runs 00761DF0 first")
            .check_device_format(
                adapter,
                device_type,
                adapter_format,
                0,
                resource_type,
                format.renderer_hint_0c,
            );
        ctx.call(
            0x0076_3D55,
            DeviceCall::CheckDeviceFormat {
                adapter,
                device_type,
                adapter_format,
                usage: 0,
                resource_type,
                format: format.renderer_hint_0c,
            },
            result,
        );
        if result == D3D_OK {
            ctx.result.state.pixel_format_tables[table][index] = Some(format);
        }
    }
}

/// IDA: 00768C10 NiDX9Renderer_InitializeTextureFormatsAndDefault and 00764630 NiDX9Renderer_InitializeDefaultFormats. The
/// 4x4 0xFF NiPixelData per table is host-internal (no device call). The 2x1 default texture goes through
/// NiSourceTexture_CreateFromPixelData with prefs {6, 0, 0} while the Phase 03 base converter (equality only) is the default.
/// Completion: 100%.
fn initialize_texture_formats_and_default(ctx: &mut RendererContext<'_>) {
    initialize_texture_format_table(ctx, 0, D3DRTYPE_TEXTURE);
    initialize_texture_format_table(ctx, 1, D3DRTYPE_TEXTURE);
    initialize_texture_format_table(ctx, 2, D3DRTYPE_CUBETEXTURE);
    initialize_texture_format_table(ctx, 3, D3DRTYPE_CUBETEXTURE);
    const ORDER: [usize; 6] = [1, 0, 2, 4, 3, 5];
    let state = &mut ctx.result.state;
    for k in 0..4 {
        state.default_formats_854[k] = ORDER
            .into_iter()
            .find(|&index| state.pixel_format_tables[k][index].is_some());
    }
    let Some(default_case) = state.default_formats_854[0] else {
        // 00768C66 passes the null +0x854 to 0070E560 without a check (the original faults copying the format); the
        // host skips the texture.
        ctx.step("00768C66 no default texture format (original faults)");
        return;
    };
    let source = state.pixel_format_tables[0][default_case].expect("the case was found present");
    state.default_format_index_874 = source.extra_10;
    // 00768C9F..00768CD3: 16-bit {0x0000, 0xFFFF}; 32-bit {0, 0xFFFFFFFF}; other widths leave the new NiPixelData bytes as
    // allocated (zero here).
    let mut texels = vec![0u8; 2 * usize::from(source.bits_per_pixel_01 / 8)];
    if source.bits_per_pixel_01 == 16 {
        texels = vec![0x00, 0x00, 0xFF, 0xFF];
    } else if source.bits_per_pixel_01 == 32 {
        texels = vec![0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
    }
    // 00768CEC: prefs {6, 0, 0}; the Phase 03 base converter (equality) is still the default, so 00773BA0 picks the default
    // format itself.
    let device = ctx
        .result
        .device
        .as_deref_mut()
        .expect("00768C10 runs after CreateDevice");
    let created = create_renderer_source_texture(
        device,
        &ctx.result.state,
        &source,
        2,
        1,
        &texels,
        [6, 0, 0],
        false,
        false,
        &mut ctx.result.device_calls,
    );
    ctx.result.state.default_texture_8f0 = created.texture;
}

/// IDA: the device side of `render_state_initialization_calls`: 0077FE8D and 0077B027 SetRenderState, 0077FFD9
/// SetTextureStageState, 0077B610 SetSamplerState. Completion: 100%.
fn apply_state_calls(ctx: &mut RendererContext<'_>) {
    let calls = std::mem::take(&mut ctx.result.state.state_calls);
    let uncached = ctx.result.state.state_defaults.render_states.len();
    let device = ctx
        .result
        .device
        .as_deref_mut()
        .expect("0077BB00 runs after CreateDevice");
    for (index, call) in calls.iter().enumerate() {
        let entry = match call.kind {
            DeviceStateKind::RenderState => DeviceTraceEntry {
                site: if index < uncached {
                    0x0077_FE8D
                } else {
                    0x0077_B027
                },
                result: device.set_render_state(call.state, call.value),
                call: DeviceCall::SetRenderState {
                    state: call.state,
                    value: call.value,
                },
            },
            DeviceStateKind::TextureStageState => DeviceTraceEntry {
                site: 0x0077_FFD9,
                result: device.set_texture_stage_state(call.stage, call.state, call.value),
                call: DeviceCall::SetTextureStageState {
                    stage: call.stage,
                    state: call.state,
                    value: call.value,
                },
            },
            DeviceStateKind::SamplerState => DeviceTraceEntry {
                site: 0x0077_B610,
                result: device.set_sampler_state(call.stage, call.state, call.value),
                call: DeviceCall::SetSamplerState {
                    stage: call.stage,
                    state: call.state,
                    value: call.value,
                },
            },
        };
        ctx.result.device_calls.push(entry);
    }
    ctx.result.state.state_calls = calls;
}

/// The arguments of 0076BD40.
#[derive(Clone, Copy)]
pub(crate) struct CreateArguments {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) use_flags: u32,
    pub(crate) device_window: Option<WindowHandle>,
    pub(crate) focus_window: Option<WindowHandle>,
    pub(crate) adapter: u32,
    pub(crate) device_desc: i32,
    pub(crate) frame_buffer_format: i32,
    pub(crate) depth_stencil_format: i32,
    pub(crate) presentation_interval: i32,
    pub(crate) swap_effect: i32,
    pub(crate) frame_buffer_mode: u32,
    pub(crate) back_buffer_count: u32,
    pub(crate) refresh_rate: u32,
}

/// IDA: 0076A487..0076A505 NVPerfHUD scan. Completion: 100%.
fn scan_nvperfhud(ctx: &mut RendererContext<'_>) {
    ctx.result.state.nvperfhud = false;
    let d3d9 = ctx.result.d3d9.as_mut().expect("00761DF0 ran");
    let count = d3d9.adapter_count();
    ctx.call(0x0076_A49D, DeviceCall::GetAdapterCount, count);
    if count == 0 {
        return;
    }
    let mut index = 0;
    loop {
        let d3d9 = ctx.result.d3d9.as_mut().expect("00761DF0 ran");
        let (result, identifier) = d3d9.adapter_identifier(index, 0);
        ctx.call(
            0x0076_A4C6,
            DeviceCall::GetAdapterIdentifier {
                adapter: index,
                flags: 0,
            },
            result,
        );
        if identifier.description == b"NVIDIA NVPerfHUD" {
            ctx.result.state.adapter_5bc = index;
            ctx.result.state.device_type_5c0 = D3DDEVTYPE_REF;
            ctx.result.state.nvperfhud = true;
            break;
        }
        index += 1;
        let d3d9 = ctx.result.d3d9.as_mut().expect("00761DF0 ran");
        let count = d3d9.adapter_count();
        ctx.call(0x0076_A4ED, DeviceCall::GetAdapterCount, count);
        if index >= count {
            break;
        }
    }
}

/// IDA: 0076A545 and the retry at 0076A5B2 CreateDevice: a failure that changed the back buffer count tries once more. Answers
/// whether the device was created, with the +0xA90 store in between. Completion: 100%.
fn create_device(ctx: &mut RendererContext<'_>, pp: &mut PresentParameters) -> bool {
    let saved_back = pp.back_buffer_count;
    let adapter = ctx.result.state.adapter_5bc;
    let device_type = ctx.result.state.device_type_5c0;
    let focus = ctx.result.state.focus_window_3b8;
    let behavior = ctx.result.state.behavior_flags_5c4;
    let attempt = |ctx: &mut RendererContext<'_>, pp: &mut PresentParameters, site: u32| {
        ctx.result.creation.attempts += 1;
        let parameters = *pp;
        let (result, device) = ctx
            .result
            .d3d9
            .as_mut()
            .expect("00761DF0 ran")
            .create_device(adapter, device_type, focus, behavior, pp);
        ctx.result.device = device;
        ctx.call(
            site,
            DeviceCall::CreateDevice {
                adapter,
                device_type,
                behavior_flags: behavior,
                parameters,
            },
            result,
        );
        result
    };
    let mut result = attempt(ctx, pp, 0x0076_A545);
    // 0076A54D..0076A563: unsigned (100h)/(100w) != 75. The division is of the two scaled sizes as written, so every
    // landscape mode gives 0 and the flag is set for any of them; P04-RUN-001 recorded +0xA90 = 1 at 1920x1080. A zero
    // width faults in the original; the host leaves the flag clear.
    ctx.result.state.non_standard_aspect_a90 = pp
        .back_buffer_height
        .wrapping_mul(100)
        .checked_div(pp.back_buffer_width.wrapping_mul(100))
        .is_some_and(|ratio| ratio != 75);
    if failed(result) && saved_back != pp.back_buffer_count {
        result = attempt(ctx, pp, 0x0076_A5B2);
    }
    let creation = &mut ctx.result.creation;
    creation.adapter = adapter;
    creation.device_type = device_type;
    creation.behavior_flags = behavior;
    creation.focus_window = focus;
    creation.parameters = *pp;
    creation.result = result;
    !failed(result)
}

/// IDA: 0076A1C0 NiDX9Renderer_Initialize. Completion: 100% of the device-visible calls; the shader, geometry and light
/// managers and the render target group are host substitutions (their device AddRefs are not repeated).
/// TODO: [ ] Phase 05+ (carried): shader factories and programs, geometry and light managers, reset paths 0076A970/0076B3F0.
fn initialize_dx9_renderer(ctx: &mut RendererContext<'_>, a: &CreateArguments) -> bool {
    {
        let state = &mut ctx.result.state;
        state.create_width_a58 = a.width;
        state.create_height_a5c = a.height;
        state.create_use_flags_a60 = a.use_flags;
        state.create_device_desc_a70 = a.device_desc;
        state.create_frame_buffer_format_a74 = a.frame_buffer_format;
        state.create_depth_stencil_format_a78 = a.depth_stencil_format;
        state.create_presentation_interval_a7c = a.presentation_interval;
        state.create_swap_effect_a80 = a.swap_effect;
        state.create_device_window_a64 = a.device_window;
        state.create_focus_window_a68 = a.focus_window;
        state.create_adapter_a6c = a.adapter;
        state.create_frame_buffer_mode_a84 = a.frame_buffer_mode;
        state.create_back_buffer_count_a88 = a.back_buffer_count;
        state.create_refresh_rate_a8c = a.refresh_rate;
    }
    let system = adapter_enumerator(ctx);
    {
        let state = &mut ctx.result.state;
        state.adapter_5bc = a.adapter;
        state.focus_window_3b8 = a.focus_window;
        state.device_window_3b4 = a.device_window;
    }
    let mut device_desc = a.device_desc;
    let Some((device_type, behavior)) = expand_device_type(device_desc) else {
        ctx.error(INVALID_DEVICE_TYPE);
        return false;
    };
    ctx.result.state.device_type_5c0 = device_type;
    ctx.result.state.behavior_flags_5c4 = behavior;
    if a.use_flags & 0x40 != 0 {
        ctx.result.state.behavior_flags_5c4 |= D3DCREATE_FPU_PRESERVE;
    }
    if a.use_flags & 0x20 != 0 {
        ctx.result.state.behavior_flags_5c4 |= D3DCREATE_MULTITHREADED;
    }
    let adapter = system
        .as_ref()
        .and_then(|s| s.adapter(ctx.result.state.adapter_5bc));
    let Some(adapter) = adapter else {
        ctx.error(INVALID_ADAPTER);
        return false;
    };
    if valid_device(adapter, ctx.result.state.device_type_5c0).is_none() {
        ctx.error(INVALID_DEVICE_TYPE);
        return false;
    }
    let inputs = PresentParameterInputs {
        device_window: ctx.result.state.device_window_3b4,
        width: a.width,
        height: a.height,
        use_flags: a.use_flags,
        frame_buffer_mode: a.frame_buffer_mode,
        back_buffer_count: a.back_buffer_count,
        swap_effect: a.swap_effect,
        refresh_rate: a.refresh_rate,
        presentation_interval: a.presentation_interval,
    };
    let mut pp = PresentParameters::default();
    loop {
        let mut created = false;
        let current = system
            .as_ref()
            .and_then(|s| s.adapter(ctx.result.state.adapter_5bc));
        let device = current.and_then(|c| valid_device(c, ctx.result.state.device_type_5c0));
        if let (Some(current), Some(device)) = (current, device) {
            if let Some(parameters) = init_present_params(
                ctx,
                current,
                device,
                &inputs,
                a.frame_buffer_format,
                a.depth_stencil_format as u32,
            ) {
                pp = parameters;
                scan_nvperfhud(ctx);
                created = create_device(ctx, &mut pp);
            }
        } else {
            ctx.error(DEVICE_NOT_VALID);
        }
        if created {
            break;
        }
        // 0076A5BE/LABEL_36: next device description.
        if device_desc == 4 {
            ctx.error(NO_REFERENCE_DEVICE);
            return false;
        }
        device_desc += 1;
        if device_desc == 4 {
            return false;
        }
        let Some((device_type, behavior)) = expand_device_type(device_desc) else {
            ctx.error(INVALID_DEVICE_TYPE);
            return false;
        };
        ctx.result.state.device_type_5c0 = device_type;
        ctx.result.state.behavior_flags_5c4 = behavior;
    }
    if ctx.result.device.is_none() {
        ctx.error(NO_HARDWARE_DEVICE);
        return false;
    }
    ctx.result.parameters = pp;
    if !initialize_device_caps(ctx, &pp) {
        return false;
    }
    let behavior = ctx.result.state.behavior_flags_5c4;
    ctx.result.state.mixed_vertex_processing_5c9 = behavior & D3DCREATE_MIXED_VERTEXPROCESSING != 0;
    ctx.result.state.software_vertex_processing_5c8 =
        behavior & D3DCREATE_HARDWARE_VERTEXPROCESSING == 0;
    // 0076DF70 NiDX9ImplicitBufferData_Create: GetRenderTarget(0), GetDesc.
    let device = ctx.result.device.as_mut().expect("checked above");
    let (result, back_buffer) = device.render_target(0);
    ctx.call(
        0x0076_DFE3,
        DeviceCall::GetRenderTarget { index: 0 },
        result,
    );
    if !failed(result) {
        ctx.result.state.back_buffer = back_buffer;
        if let Some(surface) = back_buffer {
            let device = ctx.result.device.as_mut().expect("checked above");
            let (result, desc) = device.surface_desc(surface);
            ctx.call(0x0076_E00A, DeviceCall::SurfaceGetDesc, result);
            ctx.result.state.back_buffer_desc = desc;
        }
    }
    // 0076E0B0 NiDX9ImplicitDepthStencilBufferData_Create.
    let device = ctx.result.device.as_mut().expect("checked above");
    let (result, depth_stencil) = device.depth_stencil_surface();
    ctx.call(0x0076_E100, DeviceCall::GetDepthStencilSurface, result);
    if !failed(result) {
        ctx.result.state.depth_stencil = depth_stencil;
        if let Some(surface) = depth_stencil {
            let device = ctx.result.device.as_mut().expect("checked above");
            let (result, desc) = device.surface_desc(surface);
            ctx.call(0x0076_E12A, DeviceCall::SurfaceGetDesc, result);
            ctx.result.state.depth_stencil_desc = desc;
        }
    }
    // 009A2130 render target group, 00768980 window map: host substitution.
    initialize_texture_formats_and_default(ctx);
    // 0077BB00 render state (z-buffer 1), 00778F60 shader system, 007766E0 light manager: the device state calls in order.
    let mut defaults = original_render_state_defaults();
    let mut cache = std::mem::take(&mut ctx.result.state.state_cache);
    let max_stages = ctx.result.state.max_texture_blend_stages;
    let caps = ctx.result.state.caps_284;
    let calls =
        render_state_initialization_calls(&mut defaults, &mut cache, &caps, max_stages, true);
    ctx.result.state.state_defaults = defaults;
    ctx.result.state.state_cache = cache;
    ctx.result.state.state_calls = calls;
    apply_state_calls(ctx);
    // 0076A8A2..0076A8B5: a nonzero back-buffer data +0x24 calls render state vt+0x38(1), which NiDX9RenderState (vtable
    // 00A8A9F4) binds to 0068F970 NiObject_PostLoadNoOp: no effect to port.
    // 0076A8BD..0076A923 SetCameraData(00B3F9A8 zero, UNIT_X, UNIT_Y, UNIT_Z, frustum {-1, 1, 1, -1, 0.1, 1, perspective:
    // 0070DE60 clears +0x18}, port {0, 1, 1, 0}).
    let target = ctx.result.state.back_buffer_desc;
    let camera = camera_data(
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        &CameraFrustum {
            left: -1.0,
            right: 1.0,
            top: 1.0,
            bottom: -1.0,
            near_plane: 0.1,
            far_plane: 1.0,
            orthographic: false,
        },
        &CameraPort {
            left: 0.0,
            right: 1.0,
            top: 1.0,
            bottom: 0.0,
        },
        false,
        target.width,
        target.height,
    );
    ctx.result.state.camera = camera;
    let device = ctx.result.device.as_mut().expect("checked above");
    let result = device.set_transform(D3DTS_VIEW, &camera.view);
    ctx.call(
        0x0076_2813,
        DeviceCall::SetTransform {
            state: D3DTS_VIEW,
            matrix_bits: camera.view.bits(),
        },
        result,
    );
    let device = ctx.result.device.as_mut().expect("checked above");
    let result = device.set_transform(D3DTS_PROJECTION, &camera.projection);
    ctx.call(
        0x0076_2640,
        DeviceCall::SetTransform {
            state: D3DTS_PROJECTION,
            matrix_bits: camera.projection.bits(),
        },
        result,
    );
    let device = ctx.result.device.as_mut().expect("checked above");
    let result = device.set_viewport(&camera.viewport);
    ctx.call(
        0x0076_2640,
        DeviceCall::SetViewport {
            x: camera.viewport.x,
            y: camera.viewport.y,
            width: camera.viewport.width,
            height: camera.viewport.height,
            min_z_bits: camera.viewport.min_z.to_bits(),
            max_z_bits: camera.viewport.max_z.to_bits(),
        },
        result,
    );
    ctx.step("0076A1C0 NiDX9Renderer_Initialize");
    true
}

/// IDA: 0076BD40 NiDX9Renderer_Create: 00761DF0, then FormHeap(0xB00) + 00769BE0 constructor (007641A0 defaults), then
/// 0076A1C0. Completion: 100%.
pub(crate) fn create_dx9_renderer(ctx: &mut RendererContext<'_>, a: &CreateArguments) -> bool {
    if ensure_d3d9(ctx) == -1 {
        ctx.error(COULD_NOT_INITIALIZE_DIRECTX);
        return false;
    }
    let state = &mut ctx.result.state;
    state.background_color_5d4 = 0xFF80_8080;
    state.software_vertex_processing_5c8 = false;
    state.mixed_vertex_processing_5c9 = false;
    state.format_bits = FormatBitsTable(FORMAT_BITS_TABLE);
    if !initialize_dx9_renderer(ctx, a) {
        ctx.result.device = None;
        return false;
    }
    true
}

/// IDA: 00769F90 NiDX9Renderer_SetBackgroundColor. 00769FA0..0076A02A run after CreateDevice: 24-bit x87 site, plain `f32`.
/// Completion: 100%.
pub(crate) fn background_color_argb(red: f32, green: f32, blue: f32) -> u32 {
    let channel = |value: f32| x87::ftol(value * 255.0) as u32;
    (((channel(red) | 0xFFFF_FF00) << 8 | channel(green)) << 8) | channel(blue)
}

/// IDA: 0040C280/0040C1B0 and the removers 0040C2F0/0040C220 work on two NiTArray pairs; this is one array: +4 base
/// (capacity = slots.len()), +0xA first free, +0xC non-null count, +0xE grow step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginalPointerArray {
    pub slots: Vec<u32>,
    pub size: u16,
    pub used: u16,
    pub grow_by: u16,
}

impl Default for OriginalPointerArray {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            size: 0,
            used: 0,
            grow_by: 1,
        }
    }
}

impl OriginalPointerArray {
    /// IDA: 004E4A10 NiTArray_SetSize: shrinking below the first-free size clears the tail and clamps size; the new
    /// capacity is allocated and copied. Completion: 100%.
    pub fn set_size(&mut self, capacity: u32) {
        if capacity < u32::from(self.size) {
            for i in capacity..u32::from(self.size) {
                if self.slots[i as usize] != 0 {
                    self.slots[i as usize] = 0;
                    self.used = self.used.wrapping_sub(1);
                }
            }
            self.size = capacity as u16;
        }
        self.slots.resize(capacity as usize, 0);
    }

    /// IDA: 0042BB90 NiTArray_AddAtIndex (no capacity check; the caller grows): size = max(size, index + 1); a non-null
    /// value counts. Completion: 100%.
    pub fn add_at_index(&mut self, index: u32, value: u32) -> u32 {
        let past_size = index >= u32::from(self.size);
        if past_size {
            self.size = (index + 1) as u16;
        }
        // 0042BBA7..0042BBE6: past the size a nonzero value counts, whatever the slot holds. Inside it, nonzero over zero counts up, zero over nonzero counts down, and the rest don't count.
        let old = if past_size {
            0
        } else {
            self.slots[index as usize]
        };
        if value != 0 && old == 0 {
            self.used = self.used.wrapping_add(1);
        } else if value == 0 && old != 0 {
            self.used = self.used.wrapping_sub(1);
        }
        self.slots[index as usize] = value;
        index
    }

    /// IDA: 00405020 NiTArray_RemoveAt. Completion: 100%.
    pub fn remove_at(&mut self, index: u32) -> u32 {
        if index >= u32::from(self.size) {
            return 0;
        }
        let value = self.slots[index as usize];
        self.slots[index as usize] = 0;
        if value != 0 {
            self.used = self.used.wrapping_sub(1);
        }
        if index == u32::from(self.size) - 1 {
            self.size -= 1;
        }
        value
    }
}

/// IDA: two NiTArray pairs on the renderer: +0xA98/+0xAA8 reset notifications, +0xAB8/+0xAC8 lost-device notifications. The
/// callbacks are original-address tokens.
/// TODO: [ ] Carried: the invokers with 0076A970/0076B3F0.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct RendererCallbackPairs {
    /// +0xA98 (bool(*)(bool, void*)).
    pub reset_functions: OriginalPointerArray,
    /// +0xAA8.
    pub reset_data: OriginalPointerArray,
    /// +0xAB8 (bool(*)(void*)).
    pub lost_device_functions: OriginalPointerArray,
    /// +0xAC8.
    pub lost_device_data: OriginalPointerArray,
}

/// IDA: 0040C280/0040C1B0's shared body. Completion: 100%.
fn add_pair(
    functions: &mut OriginalPointerArray,
    data: &mut OriginalPointerArray,
    function: u32,
    value: u32,
) -> u32 {
    let index = u32::from(functions.size);
    if index as usize >= functions.slots.len() {
        functions.set_size(index + u32::from(functions.grow_by));
    }
    functions.add_at_index(index, function);
    if index as usize >= data.slots.len() {
        data.set_size(index + u32::from(data.grow_by));
    }
    data.add_at_index(index, value);
    index
}

/// IDA: 0040C2F0/0040C220's shared body. Completion: 100%.
fn remove_pair(
    functions: &mut OriginalPointerArray,
    data: &mut OriginalPointerArray,
    function: u32,
) -> bool {
    for index in 0..u32::from(functions.size) {
        if functions.slots[index as usize] == function {
            functions.remove_at(index);
            data.remove_at(index);
            return true;
        }
    }
    false
}

impl RendererCallbackPairs {
    /// IDA: 0040C280: index = functions.size; index >= capacity -> SetSize(index + grow); AddAtIndex(index, function); the
    /// data array grows on its own capacity with the same index. Returns the index. Completion: 100%.
    pub fn add_lost_device(&mut self, function: u32, data: u32) -> u32 {
        add_pair(
            &mut self.lost_device_functions,
            &mut self.lost_device_data,
            function,
            data,
        )
    }

    /// IDA: 0040C1B0, as `add_lost_device`. Completion: 100%.
    pub fn add_reset(&mut self, function: u32, data: u32) -> u32 {
        add_pair(
            &mut self.reset_functions,
            &mut self.reset_data,
            function,
            data,
        )
    }

    /// IDA: 0040C2F0: first index whose function matches, then remove from both arrays; false when absent. Completion: 100%.
    pub fn remove_lost_device(&mut self, function: u32) -> bool {
        remove_pair(
            &mut self.lost_device_functions,
            &mut self.lost_device_data,
            function,
        )
    }

    /// IDA: 0040C220, as `remove_lost_device`. Completion: 100%.
    pub fn remove_reset(&mut self, function: u32) -> bool {
        remove_pair(&mut self.reset_functions, &mut self.reset_data, function)
    }
}

/// IDA: the 007B45F0 inputs: low words from 004980D0, the adapter flag, the driver name and the PS 2.0 slot count.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderPathInputs {
    pub pixel_shader_version: u32,
    pub vertex_shader_version: u32,
    pub force_1x: bool,
    pub have_adapter: bool,
    pub driver: Vec<u8>,
    pub ps20_instructions: i32,
}

/// IDA: 007B47E0. `None` is the original's null. Completion: 100%.
#[must_use]
pub const fn vertex_shader_target(path: i32) -> Option<&'static str> {
    match path {
        1 => Some("vs_1_1"),
        2..=6 => Some("vs_2_0"),
        7 => Some("vs_3_0"),
        _ => None,
    }
}

/// IDA: 007B4780. `None` is the original's null. Completion: 100%.
#[must_use]
pub const fn pixel_shader_target(g: &RendererGlobals, ps2x: bool) -> Option<&'static str> {
    match if ps2x { g.ps2x_path } else { g.render_path } {
        1 => Some("ps_1_3"),
        2 => Some("ps_2_0"),
        3 | 5 => Some("ps_2_a"),
        4 | 6 => Some("ps_2_b"),
        7 => Some("ps_3_0"),
        _ => None,
    }
}

/// IDA: 007DAB80. Completion: 100%.
#[must_use]
pub const fn shader_package(g: &RendererGlobals) -> i32 {
    let filtering = if g.fp16_filtering { 1 } else { 0 };
    match g.render_path {
        1 => 1,
        2 => 2,
        3 => 5,
        4 => 8,
        5 => {
            if g.shaders_30 {
                if g.hdr { 12 + filtering } else { 4 }
            } else if g.hdr {
                10 + filtering
            } else {
                3
            }
        }
        6 => {
            if g.shaders_30 {
                if g.hdr { 16 + filtering } else { 7 }
            } else if g.hdr {
                14 + filtering
            } else {
                6
            }
        }
        7 => {
            if g.hdr {
                18 + filtering
            } else {
                9
            }
        }
        _ => 0,
    }
}

/// `prepare_renderer_source_texture`'s answer: the boundary's value plus the converted level 0. The texels are this file's
/// business, so they stay out of `SourceTextureUpload`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedSourceTexture {
    pub upload: SourceTextureUpload,
    pub texels: Vec<u8>,
}

/// `create_renderer_source_texture`'s answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatedSourceTexture {
    pub prepared: PreparedSourceTexture,
    pub texture: Option<TextureHandle>,
}

/// IDA: 00761270 NiDX9SourceTextureData_Create's format choice and conversion, as 00701FC0 NiSourceTexture_CreateFromPixelData
/// with 00B256CC set -> 00701C00 -> renderer vt+0x104 00763560 (under the renderer lock) reaches it, for one-level, one-face
/// pixel data: power-of-two (or textureCapsFlags & 8) -> 00773BA0 (`find_closest_pixel_format`'s port) against renderer +0x6F4 with the current default converter's CanConvertPixelData
/// (Phase 03 base: equality; after 00498647: NiDevImageConverter 0071B600), the conversion (RGB24 0071D7F0/0071D8D0/0071D960,
/// or the same data). This is the format choice and conversion without the device calls. Completion: 100% for the reached
/// inputs (RGB24, RGBA32 (05-D-3) or an already-matching format, no mipmap generation).
/// TODO: [ ] Later phases (carried): non-power-of-two 00761400 fallback, the other 0071EA60 converters and 0071B8D0 mipmap
/// generation.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn prepare_renderer_source_texture(
    state: &RendererState,
    source: &PixelFormat,
    width: u32,
    height: u32,
    texels: &[u8],
    prefs: [u32; 3],
    mipmap_by_default: bool,
    device_image_converter: bool,
) -> PreparedSourceTexture {
    let mut upload = SourceTextureUpload {
        table_case: None,
        table_format: PixelFormat::default(),
        d3d_format: D3DFMT_UNKNOWN,
        device_texture: false,
    };
    let empty = |upload: SourceTextureUpload| PreparedSourceTexture {
        upload,
        texels: Vec::new(),
    };
    let pow2 = |value: u32| value != 0 && value & (value - 1) == 0;
    if (!pow2(width) || !pow2(height)) && state.texture_caps_flags_5e0 & 8 == 0 {
        return empty(upload); // 007613FA non-power-of-two path: carried
    }
    let can_convert: CanConvert = if device_image_converter {
        dev_converter_can_convert
    } else {
        pixel_format_equals
    };
    upload.table_case =
        find_closest_pixel_format(source, prefs, &state.pixel_format_tables[0], can_convert);
    let Some(case) = upload.table_case else {
        return empty(upload); // 007614C2 00701400 fallback: carried
    };
    let dest = state.pixel_format_tables[0][case].expect("00773BA0 chose a present entry");
    upload.table_format = dest;
    // 0076150B..00761528: YES, or DEFAULT with 00B3F708 set.
    let mipmap = prefs[2] == 1 || (prefs[2] == 2 && mipmap_by_default);
    if mipmap {
        return empty(upload); // 0071B8D0 mipmap generation: carried
    }
    // 00761549 default converter vt+0x10 ConvertPixelData (0071B300 base, as P04-RUN-001 recorded for the default texture;
    // 0071EA60 device).
    let converted = if pixel_format_equals(source, &dest) {
        texels.to_vec()
    } else if device_image_converter && pixel_format_equals(source, &RGB24_FORMAT) {
        convert_rgb24(texels, width, height, &dest)
    } else if device_image_converter && pixel_format_equals(source, &RGBA32_FORMAT) {
        convert_rgba32(texels, width, height, &dest) // 0071F1DC
    } else {
        return empty(upload); // other 0071EA60 converters: carried
    };
    upload.d3d_format = dest.renderer_hint_0c;
    PreparedSourceTexture {
        upload,
        texels: converted,
    }
}

/// IDA: 00760700/00760CC0/00760860 for one-level, one-face pixel data: CreateTexture(w, h, levels - skip, 0, fmt, MANAGED),
/// GetType twice, GetSurfaceLevel(0), GetDesc, LockRect(NULL, 0), one memcpy when the pitch equals the row bytes else
/// per-row min(pitch, row), UnlockRect, Release. Mip levels: pow2 or textureCapsFlags & 8 -> the data's level count.
/// Completion: 100% for the level counts these textures have (1).
#[allow(clippy::too_many_arguments)]
fn upload_source_texture(
    device: &mut dyn Direct3DDevice9,
    width: u32,
    height: u32,
    format: u32,
    bytes_per_pixel: u32,
    texels: &[u8],
    skip_levels: u32,
    calls: &mut Vec<DeviceTraceEntry>,
) -> Option<TextureHandle> {
    let levels = 1u32;
    let skip = skip_levels.min(levels - 1);
    let mut w = width;
    let mut h = height;
    for _ in 0..skip {
        w = (if w & 1 != 0 { w + 1 } else { w }) >> 1;
        h = (if h & 1 != 0 { h + 1 } else { h }) >> 1;
    }
    let (result, texture) = device.create_texture(w, h, levels - skip, 0, format, D3DPOOL_MANAGED);
    calls.push(DeviceTraceEntry {
        site: 0x0076_07E3,
        call: DeviceCall::CreateTexture {
            width: w,
            height: h,
            levels: levels - skip,
            usage: 0,
            format,
            pool: D3DPOOL_MANAGED,
        },
        result,
    });
    if failed(result) {
        return None;
    }
    let texture = texture?;
    let mut get_type = |device: &mut dyn Direct3DDevice9, site: u32| {
        let kind = device.texture_type(texture);
        calls.push(DeviceTraceEntry {
            site,
            call: DeviceCall::TextureGetType,
            result: kind,
        });
        kind
    };
    if get_type(device, 0x0076_0D13) == D3DRTYPE_CUBETEXTURE
        || get_type(device, 0x0076_0D39) == D3DRTYPE_VOLUMETEXTURE
    {
        return Some(texture);
    }
    for level in 0..levels - skip {
        let (result, surface) = device.texture_surface_level(texture, level);
        calls.push(DeviceTraceEntry {
            site: 0x0076_0CD5,
            call: DeviceCall::TextureGetSurfaceLevel { level },
            result,
        });
        let (false, Some(surface)) = (failed(result), surface) else {
            break;
        };
        let (result, desc) = device.surface_desc(surface);
        calls.push(DeviceTraceEntry {
            site: 0x0076_0873,
            call: DeviceCall::SurfaceGetDesc,
            result,
        });
        if failed(result) {
            continue;
        }
        let (result, pitch) = device.lock_rect(surface);
        calls.push(DeviceTraceEntry {
            site: 0x0076_08A1,
            call: DeviceCall::LockRect,
            result,
        });
        if failed(result) {
            continue;
        }
        let row = width as usize * bytes_per_pixel as usize;
        let rows = height.min(desc.height) as usize;
        let bytes = copied_rows(texels, row, pitch, rows);
        let result = device.unlock_rect(surface, &bytes);
        calls.push(DeviceTraceEntry {
            site: 0x0076_09AE,
            call: DeviceCall::UnlockRect {
                pitch,
                bytes: bytes.len(),
            },
            result,
        });
    }
    Some(texture)
}

/// IDA: 00760CC0's copy, the block between LockRect and UnlockRect: one memcpy of rows * row bytes when the row bytes equal the pitch, else per
/// row min(row, pitch) with the destination advanced by the full pitch. Bytes the source does not have stay zero, where the
/// original would read past its allocation.
fn copied_rows(source: &[u8], row: usize, pitch: i32, rows: usize) -> Vec<u8> {
    let stride = pitch as usize;
    let mut out = vec![0u8; stride * rows];
    let count = row.min(stride);
    for y in 0..rows {
        let from = source.get(y * row..).unwrap_or(&[]);
        let length = count.min(from.len());
        out[y * stride..y * stride + length].copy_from_slice(&from[..length]);
    }
    out
}

/// IDA: as `prepare_renderer_source_texture`, then 00760700 and 00760CC0 (00B42050 g_niDX9MipmapSkipLevel is 0 through
/// Phase 04). Completion: 100% for the reached inputs.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn create_renderer_source_texture(
    device: &mut dyn Direct3DDevice9,
    state: &RendererState,
    source: &PixelFormat,
    width: u32,
    height: u32,
    texels: &[u8],
    prefs: [u32; 3],
    mipmap_by_default: bool,
    device_image_converter: bool,
    calls: &mut Vec<DeviceTraceEntry>,
) -> CreatedSourceTexture {
    let mut prepared = prepare_renderer_source_texture(
        state,
        source,
        width,
        height,
        texels,
        prefs,
        mipmap_by_default,
        device_image_converter,
    );
    let mut texture = None;
    if prepared.upload.d3d_format != D3DFMT_UNKNOWN && prepared.upload.table_case.is_some() {
        texture = upload_source_texture(
            device,
            width,
            height,
            prepared.upload.d3d_format,
            u32::from(prepared.upload.table_format.bits_per_pixel_01 / 8),
            &prepared.texels,
            0,
            calls,
        );
        prepared.upload.device_texture = texture.is_some();
    }
    CreatedSourceTexture { prepared, texture }
}

/// IDA: 00760860 NiDX9SourceTextureData_CopyLevelToSurface(data, level, surface, face): GetDesc, LockRect(NULL, 0), rows =
/// min(height, desc height), row bytes = bpp * width; compressed formats (4..6) use pitch / 4, DXT1 rows of ((w + 3) >> 1) &
/// ~1 bytes, others (w + 3) & ~3, and (h + 3) & ~3 rows; one memcpy when the row bytes equal the pitch, else per row
/// min(row, pitch) with the destination advanced by the full pitch; UnlockRect. Faces start at face * offsets[levels].
/// Completion: 100%.
pub fn copy_pixel_level_to_surface(
    device: &mut dyn Direct3DDevice9,
    surface: SurfaceHandle,
    data: &PixelData,
    level: u32,
    face: u32,
    calls: &mut Vec<DeviceTraceEntry>,
) {
    // 00760868..007608BF GetDesc, LockRect(NULL, 0).
    let (result, description) = device.surface_desc(surface);
    calls.push(DeviceTraceEntry {
        site: 0x0076_0873,
        call: DeviceCall::SurfaceGetDesc,
        result,
    });
    if failed(result) {
        return;
    }
    let (result, locked_pitch) = device.lock_rect(surface);
    calls.push(DeviceTraceEntry {
        site: 0x0076_08A1,
        call: DeviceCall::LockRect,
        result,
    });
    if failed(result) {
        return;
    }
    let width = data.widths_54[level as usize];
    let mut rows = data.heights_58[level as usize].min(description.height);
    let mut row_bytes = data.bytes_per_pixel_64.wrapping_mul(width);
    let mut pitch = locked_pitch as u32;
    if (4..=6).contains(&data.format_08.format_04) {
        // 007608F8..0076091B.
        pitch >>= 2;
        row_bytes = if description.format == D3DFMT_DXT1 {
            (width.wrapping_add(3) >> 1) & 0x7FF_FFFE
        } else {
            width.wrapping_add(3) & 0xFFF_FFFC
        };
        rows = rows.wrapping_add(3) & 0xFFFF_FFFC;
    }
    let stride = *data.offsets_5c.last().expect("levels + 1 offsets");
    let start = stride as usize * face as usize + data.offsets_5c[level as usize] as usize;
    let source = data.pixels_50.get(start..).unwrap_or(&[]);
    let bytes = if row_bytes == pitch {
        // 0076097E one memcpy of rows * row bytes.
        let total = rows as usize * row_bytes as usize;
        let mut out = vec![0u8; (locked_pitch as usize * rows as usize).max(total)];
        let length = total.min(source.len());
        out[..length].copy_from_slice(&source[..length]);
        out
    } else {
        // 00760925..00760968: min(row, pitch) per row, destination += full pitch.
        let count = row_bytes.min(pitch) as usize;
        let mut out = vec![0u8; locked_pitch as usize * rows as usize];
        for y in 0..rows as usize {
            let from = source.get(y * row_bytes as usize..).unwrap_or(&[]);
            let length = count.min(from.len());
            let at = y * locked_pitch as usize;
            out[at..at + length].copy_from_slice(&from[..length]);
        }
        out
    };
    let result = device.unlock_rect(surface, &bytes);
    calls.push(DeviceTraceEntry {
        site: 0x0076_09AE,
        call: DeviceCall::UnlockRect {
            pitch: locked_pitch,
            bytes: bytes.len(),
        },
        result,
    });
}

/// IDA: 0071E790 without mipmap generation (+ 0071EA60 for the reached pairs): a new NiPixelData in the destination format
/// with the source's levels and faces (0071E91D..0071E951), then per face and level the equal-format memcpy
/// (0071EB30..0071EB6D), the RGB24 (0071D7F0/0071D8D0/0071D960) or the RGBA32 (0071DA20/0071DB00/0071DBD0) converter. `None`
/// for any other pair (carried). Completion: 100% for the ported pairs.
#[must_use]
pub fn convert_pixel_data(source: &PixelData, dest: &PixelFormat) -> Option<PixelData> {
    let mut result = construct_pixel_data(
        source.widths_54[0],
        source.heights_58[0],
        dest,
        source.levels_60,
        source.faces_6c,
    );
    let equal = pixel_format_equals(&source.format_08, dest);
    for face in 0..source.faces_6c {
        for level in 0..source.levels_60 {
            let from = source.level(face, level);
            let to = result.level_mut(face, level);
            if equal {
                to.copy_from_slice(from);
                continue;
            }
            let width = source.widths_54[level as usize];
            let height = source.heights_58[level as usize];
            let converted = if pixel_format_equals(&source.format_08, &RGB24_FORMAT) {
                convert_rgb24(from, width, height, dest)
            } else if pixel_format_equals(&source.format_08, &RGBA32_FORMAT) {
                convert_rgba32(from, width, height, dest)
            } else {
                return None;
            };
            let length = converted.len().min(to.len());
            to[..length].copy_from_slice(&converted[..length]);
        }
    }
    Some(result)
}

/// IDA: 00774550 NiDX9SourceCubeMapData::Create for application pixel data (+0x3C): no cube format -> no_cube_formats; a
/// non-square or non-power-of-two face -> replacement; 00773BA0 on renderer +0x7A4 (or a palettized source) -> replacement
/// when it fails; mipmap = prefs[2] == 1, or 2 with 00B3F708 and renderer +0x6E8; +0x0C = the table entry, +0x54 = +0x58 =
/// the face width; 0071E790; +0x68 = offsets[levels] * faces; +0x5C = renderer +0x6E8 ? levels : 1; +0x7C = the converted
/// revision; 00774420 CreateCubeTexture(width, +0x5C, 0, format, MANAGED) and the cube counters; 007744D0 per face.
/// Completion: 100% of the reached branches.
#[allow(clippy::too_many_arguments)]
pub fn create_renderer_cube_texture(
    device: Option<&mut (dyn Direct3DDevice9 + 'static)>,
    state: &RendererState,
    data: &PixelData,
    prefs: [u32; 3],
    mipmap_by_default: bool,
    device_image_converter: bool,
    out: &mut TextureRendererData,
    statistics: &mut TextureMemoryStatistics,
    calls: &mut Vec<DeviceTraceEntry>,
) -> RendererTextureStatus {
    *out = TextureRendererData {
        original_vtable: 0x00A8_9F80,
        cube: true,
        ..TextureRendererData::default()
    };
    // 0077455B..00774594: any cube format in renderer +0x7A4.
    let table = &state.pixel_format_tables[2];
    if table.iter().all(Option::is_none) {
        return RendererTextureStatus::NoCubeFormats;
    }
    // 00774636..00774650: square power-of-two faces.
    let width = data.widths_54[0];
    let pow2 = |value: u32| value != 0 && value & (value - 1) == 0;
    if width != data.heights_58[0] || !pow2(width) {
        return RendererTextureStatus::ReplacementTexture;
    }
    // 007746A7..007746D1: 00773BA0 against the cube table; palettized sources fail.
    let can_convert: CanConvert = if device_image_converter {
        dev_converter_can_convert
    } else {
        pixel_format_equals
    };
    out.table_case = find_closest_pixel_format(&data.format_08, prefs, table, can_convert);
    let Some(case) = out
        .table_case
        .filter(|_| data.format_08.format_04 != 2 && data.format_08.format_04 != 3)
    else {
        return RendererTextureStatus::ReplacementTexture;
    };
    // 00774736..0077475E the mipmap byte +0x65.
    out.mipmap_65 =
        prefs[2] == 1 || (prefs[2] == 2 && mipmap_by_default && state.mipmap_cube_maps_6e8);
    if out.mipmap_65 && data.levels_60 <= 1 {
        return RendererTextureStatus::MipmapGeneration;
    }
    // 00774762..0077477D +0x0C, +0x58, +0x54.
    out.pixel_format_0c = table[case].expect("00773BA0 chose a present entry");
    out.width_54 = width;
    out.height_58 = width;
    // 00774790 default converter vt+0x10 ConvertPixelData.
    let Some(converted) = convert_pixel_data(data, &out.pixel_format_0c) else {
        return RendererTextureStatus::UnsupportedConversion;
    };
    out.pixel_bytes_68 = converted
        .offsets_5c
        .last()
        .expect("levels + 1 offsets")
        .wrapping_mul(converted.faces_6c);
    out.levels_5c = if state.mipmap_cube_maps_6e8 {
        converted.levels_60
    } else {
        1
    };
    out.source_revision = converted.revision_68;
    out.d3d_format = out.pixel_format_0c.renderer_hint_0c;
    let Some(device) = device else {
        return RendererTextureStatus::DeviceFailure;
    };
    // 00774420 CreateCubeTexture(width, levels, 0, format, MANAGED).
    let (result, cube) =
        device.create_cube_texture(width, out.levels_5c, 0, out.d3d_format, D3DPOOL_MANAGED);
    calls.push(DeviceTraceEntry {
        site: 0x0077_4420,
        call: DeviceCall::CreateCubeTexture {
            edge: width,
            levels: out.levels_5c,
            usage: 0,
            format: out.d3d_format,
            pool: D3DPOOL_MANAGED,
        },
        result,
    });
    let (false, Some(cube)) = (failed(result), cube) else {
        return RendererTextureStatus::DeviceFailure;
    };
    out.texture_bytes_60 = (u32::from(out.pixel_format_0c.bits_per_pixel_01) >> 3)
        .wrapping_mul(out.levels_5c)
        .wrapping_mul(width)
        .wrapping_mul(width)
        .wrapping_mul(6);
    statistics.cube_bytes = statistics.cube_bytes.wrapping_add(out.texture_bytes_60);
    let page = out.texture_bytes_60 & 0xFFFF_F000;
    statistics.cube_padding =
        statistics
            .cube_padding
            .wrapping_add(if page == out.texture_bytes_60 {
                0
            } else {
                page.wrapping_sub(out.texture_bytes_60).wrapping_add(0x1000)
            });
    // 0077484C..0077485F 007744D0 for the six faces.
    for face in 0..6 {
        for level in 0..out.levels_5c {
            let (result, surface) = device.cube_map_surface(cube, face, level);
            calls.push(DeviceTraceEntry {
                site: 0x0077_44D0,
                call: DeviceCall::CubeGetSurface { face, level },
                result,
            });
            let (false, Some(surface)) = (failed(result), surface) else {
                break;
            };
            copy_pixel_level_to_surface(device, surface, &converted, level, face, calls);
        }
    }
    out.device_texture = true;
    RendererTextureStatus::Created
}

impl TextureRendererBoundary for RendererCreation {
    /// IDA: N/A `RendererCreation`'s side of `TextureRendererBoundary`.
    fn has_device(&self) -> bool {
        self.device.is_some()
    }

    /// IDA: N/A `RendererCreation`'s side of `TextureRendererBoundary`.
    fn upload_source_texture(
        &mut self,
        format: &PixelFormat,
        width: u32,
        height: u32,
        texels: &[u8],
        prefs: [u32; 3],
    ) -> SourceTextureUpload {
        let mipmap = self.globals.texture_mipmap_by_default;
        let converter = self.device_image_converter_default;
        match self.device.as_deref_mut() {
            Some(device) => {
                create_renderer_source_texture(
                    device,
                    &self.state,
                    format,
                    width,
                    height,
                    texels,
                    prefs,
                    mipmap,
                    converter,
                    &mut self.device_calls,
                )
                .prepared
                .upload
            }
            None => {
                prepare_renderer_source_texture(
                    &self.state,
                    format,
                    width,
                    height,
                    texels,
                    prefs,
                    mipmap,
                    converter,
                )
                .upload
            }
        }
    }

    /// IDA: N/A `RendererCreation`'s side of `TextureRendererBoundary`.
    fn create_cube_texture(
        &mut self,
        data: &PixelData,
        prefs: [u32; 3],
        out: &mut TextureRendererData,
        statistics: &mut TextureMemoryStatistics,
    ) -> RendererTextureStatus {
        let mipmap = self.globals.texture_mipmap_by_default;
        let converter = self.device_image_converter_default;
        create_renderer_cube_texture(
            self.device.as_deref_mut(),
            &self.state,
            data,
            prefs,
            mipmap,
            converter,
            out,
            statistics,
            &mut self.device_calls,
        )
    }

    /// IDA: 00760EF5 D3DXGetImageInfoFromFileInMemory. Host substitution: the DLL's answer as data, from `direct_image_info`.
    fn image_info(&mut self, bytes: &[u8]) -> Option<DirectImageInfo> {
        let info = direct_image_info(bytes);
        self.device_calls.push(DeviceTraceEntry {
            site: 0x0076_0EF5,
            call: DeviceCall::D3dxGetImageInfoFromFileInMemory {
                size: bytes.len() as u32,
            },
            result: if info.is_some() {
                D3D_OK
            } else {
                D3DXERR_INVALIDDATA
            },
        });
        info
    }

    /// IDA: 007610D3 / 007610F9 / 0076111E D3DXCreate(Cube|Volume)TextureFromFileInMemory, 007609C0's 007609D3 GetLevelCount, 007609EC
    /// GetType and 00760A0C / 00760AB3 GetLevelDesc. Host substitution: the levels as data from `direct_texture_levels`; the texture
    /// the DLL would create is asked of the device seam, so a scripted device can fail it. A DDS texture this cannot describe is
    /// a host stop, with D3DXERR_INVALIDDATA on the trace, which is what a caller sees for an unreadable file.
    fn create_direct_texture(
        &mut self,
        resource_type: DirectResourceType,
        bytes: &[u8],
        original_size: u32,
    ) -> Option<Vec<Option<DirectLevelDesc>>> {
        let (site, level_site, d3d_type) = match resource_type {
            DirectResourceType::Texture => (0x0076_10D3, 0x0076_0A0C, D3DRTYPE_TEXTURE),
            DirectResourceType::CubeTexture => (0x0076_10F9, 0x0076_0AB3, D3DRTYPE_CUBETEXTURE),
            // The header reader refuses a volume, so no answer ever names one.
            DirectResourceType::VolumeTexture | DirectResourceType::Other => return None,
        };
        let levels = direct_file_facts(bytes)
            .filter(|facts| facts.resource_type == resource_type)
            .and_then(|facts| direct_texture_levels(&facts).map(|levels| (facts, levels)));
        let result = match (&levels, self.device.as_deref_mut()) {
            (Some((facts, levels)), Some(device)) => {
                let count = levels.len() as u32;
                let (result, texture) = if resource_type == DirectResourceType::CubeTexture {
                    device.create_cube_texture(facts.width, count, 0, facts.format, D3DPOOL_MANAGED)
                } else {
                    device.create_texture(
                        facts.width,
                        facts.height,
                        count,
                        0,
                        facts.format,
                        D3DPOOL_MANAGED,
                    )
                };
                if failed(result) || texture.is_some() {
                    result
                } else {
                    D3DERR_INVALIDCALL
                }
            }
            _ => D3DXERR_INVALIDDATA,
        };
        self.device_calls.push(DeviceTraceEntry {
            site,
            call: DeviceCall::D3dxCreateTextureFromFileInMemory {
                resource_type: d3d_type,
                size: original_size,
            },
            result,
        });
        if failed(result) {
            return None;
        }
        let (_, levels) = levels?;
        self.device_calls.push(DeviceTraceEntry {
            site: 0x0076_09D3,
            call: DeviceCall::TextureGetLevelCount,
            result: levels.len() as u32,
        });
        self.device_calls.push(DeviceTraceEntry {
            site: 0x0076_09EC,
            call: DeviceCall::TextureGetType,
            result: d3d_type,
        });
        for level in 0..levels.len() as u32 {
            self.device_calls.push(DeviceTraceEntry {
                site: level_site,
                call: DeviceCall::TextureGetLevelDesc { level },
                result: D3D_OK,
            });
        }
        Some(levels.into_iter().map(Some).collect())
    }
}

/// IDA: the 00498A7E..00498D4B inputs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RendererInfoInputs {
    /// Adapter identifier Description (+0x204).
    pub description: Vec<u8>,
    /// Adapter identifier Driver (+0x004).
    pub driver: Vec<u8>,
    /// Device desc caps.PixelShaderVersion.
    pub ps_version: u32,
    /// Device desc caps.VertexShaderVersion.
    pub vs_version: u32,
    pub ps20_instructions: i32,
    pub non_pow2: bool,
    /// 00497E10 back-buffer data + 0x14, read +0x10.
    pub multisample_type: i32,
}

#[cfg(test)]
#[path = "renderer_tests.rs"]
mod tests;
