//! NiDX9 system description built by NiDX9AdapterEnumerator_GetOrCreate 00763DE0 (Gamebryo NiDX9SystemDesc), its adapter,
//! device and display-format records, the selection helpers renderer creation uses, and the device seam.
//!
//! No Direct3D exists here. Windows types are plain numbers, `IDirect3D9` and `IDirect3DDevice9` are the scriptable
//! traits `Direct3D9` and `Direct3DDevice9`, and every call the C++ makes appends a `DeviceTraceEntry` with its original
//! site, arguments and result. Host records replace the original NiTArray and NiTPointerList/NiTPointerMap containers
//! (recorded substitution); every query, its arguments and order follow the disassembly.

use std::collections::BTreeMap;
use std::sync::Arc;

pub const D3D_OK: u32 = 0;
pub const D3DERR_NOTAVAILABLE: u32 = 0x8876_086A;
pub const E_FAIL: u32 = 0x8000_4005;
/// IDA: 00763E22 passes D3D_SDK_VERSION 32 (0x20 in P04-RUN-001).
pub const D3D_SDK_VERSION: u32 = 32;

/// IDA: N/A the `FAILED` macro.
#[must_use]
pub const fn failed(result: u32) -> bool {
    (result as i32) < 0
}

pub const D3DFMT_UNKNOWN: u32 = 0;
pub const D3DFMT_R8G8B8: u32 = 20;
pub const D3DFMT_A8R8G8B8: u32 = 21;
pub const D3DFMT_X8R8G8B8: u32 = 22;
pub const D3DFMT_R5G6B5: u32 = 23;
pub const D3DFMT_X1R5G5B5: u32 = 24;
pub const D3DFMT_A1R5G5B5: u32 = 25;
pub const D3DFMT_A4R4G4B4: u32 = 26;
pub const D3DFMT_R3G3B2: u32 = 27;
pub const D3DFMT_A8: u32 = 28;
pub const D3DFMT_A8R3G3B2: u32 = 29;
pub const D3DFMT_X4R4G4B4: u32 = 30;
pub const D3DFMT_A2B10G10R10: u32 = 31;
pub const D3DFMT_A8B8G8R8: u32 = 32;
pub const D3DFMT_X8B8G8R8: u32 = 33;
pub const D3DFMT_G16R16: u32 = 34;
pub const D3DFMT_A2R10G10B10: u32 = 35;
pub const D3DFMT_A16B16G16R16: u32 = 36;
pub const D3DFMT_P8: u32 = 41;
pub const D3DFMT_L8: u32 = 50;
pub const D3DFMT_A8L8: u32 = 51;
pub const D3DFMT_A4L4: u32 = 52;
pub const D3DFMT_V8U8: u32 = 60;
pub const D3DFMT_L6V5U5: u32 = 61;
pub const D3DFMT_X8L8V8U8: u32 = 62;
pub const D3DFMT_D16_LOCKABLE: u32 = 70;
pub const D3DFMT_D32: u32 = 71;
pub const D3DFMT_D15S1: u32 = 73;
pub const D3DFMT_D24S8: u32 = 75;
pub const D3DFMT_D24X8: u32 = 77;
pub const D3DFMT_D24X4S4: u32 = 79;
pub const D3DFMT_D16: u32 = 80;
pub const D3DFMT_L16: u32 = 81;
pub const D3DFMT_D32F_LOCKABLE: u32 = 82;
pub const D3DFMT_D24FS8: u32 = 83;
pub const D3DFMT_R16F: u32 = 111;
pub const D3DFMT_G16R16F: u32 = 112;
pub const D3DFMT_A16B16G16R16F: u32 = 113;
pub const D3DFMT_R32F: u32 = 114;
pub const D3DFMT_G32R32F: u32 = 115;
pub const D3DFMT_A32B32G32R32F: u32 = 116;
pub const D3DFMT_DXT1: u32 = 0x3154_5844;
pub const D3DFMT_DXT3: u32 = 0x3354_5844;
pub const D3DFMT_DXT5: u32 = 0x3554_5844;

pub const D3DDEVTYPE_HAL: u32 = 1;
pub const D3DDEVTYPE_REF: u32 = 2;
pub const D3DRTYPE_SURFACE: u32 = 1;
pub const D3DRTYPE_TEXTURE: u32 = 3;
pub const D3DRTYPE_VOLUMETEXTURE: u32 = 4;
pub const D3DRTYPE_CUBETEXTURE: u32 = 5;
pub const D3DUSAGE_RENDERTARGET: u32 = 1;
pub const D3DUSAGE_DEPTHSTENCIL: u32 = 2;
pub const D3DUSAGE_QUERY_FILTER: u32 = 0x0002_0000;
pub const D3DUSAGE_QUERY_POSTPIXELSHADER_BLENDING: u32 = 0x0008_0000;
pub const D3DPOOL_DEFAULT: u32 = 0;
pub const D3DPOOL_MANAGED: u32 = 1;
/// IDA: 0076112B D3DERR_INVALIDCALL.
pub const D3DERR_INVALIDCALL: u32 = 0x8876_086C;
/// IDA: N/A D3DXERR_INVALIDDATA, D3DX's answer for a file it cannot read.
pub const D3DXERR_INVALIDDATA: u32 = 0x8876_0B59;
pub const D3DCREATE_FPU_PRESERVE: u32 = 0x02;
pub const D3DCREATE_MULTITHREADED: u32 = 0x04;
pub const D3DCREATE_PUREDEVICE: u32 = 0x10;
pub const D3DCREATE_SOFTWARE_VERTEXPROCESSING: u32 = 0x20;
pub const D3DCREATE_HARDWARE_VERTEXPROCESSING: u32 = 0x40;
pub const D3DCREATE_MIXED_VERTEXPROCESSING: u32 = 0x80;

/// IDA: 004979E0 Gamebryo FrameBufferFormat (1..17) -> D3DFORMAT: 1..11 map to 20..30 (R8G8B8..X4R4G4B4), 12..17 to
/// 111..116 (R16F..A32B32G32R32F), others 0. Completion: 100%.
#[must_use]
pub const fn dx9_frame_buffer_format(index: i32) -> u32 {
    if index >= 1 && index <= 11 {
        (19 + index) as u32
    } else if index >= 12 && index <= 17 {
        (99 + index) as u32
    } else {
        D3DFMT_UNKNOWN
    }
}

/// IDA: 00774990, the inverse of 004979E0 (0 for other formats). Completion: 100%.
#[must_use]
pub const fn dx9_frame_buffer_index(format: u32) -> i32 {
    if format >= 20 && format <= 30 {
        format as i32 - 19
    } else if format >= 111 && format <= 116 {
        format as i32 - 99
    } else {
        0
    }
}

/// IDA: 00774EE0 bits per pixel: 8/16/24/32/64/128 by format, 0 for formats 0, 100 and the FOURCC formats it lists
/// (00774EE9..00774F70: UYVY, R8G8_B8G8, YUY2, G8R8_G8B8, DXT1..DXT5, MULTI2_ARGB8), -1 for anything else. Completion: 100%.
#[must_use]
pub const fn dx9_format_bits(format: u32) -> i32 {
    // IDA: N/A a helper, with no address of its own.
    const fn four_cc(text: &[u8; 4]) -> u32 {
        u32::from_le_bytes(*text)
    }
    const UYVY: u32 = four_cc(b"UYVY");
    const RGBG: u32 = four_cc(b"RGBG");
    const YUY2: u32 = four_cc(b"YUY2");
    const GRGB: u32 = four_cc(b"GRGB");
    const DXT1: u32 = four_cc(b"DXT1");
    const DXT2: u32 = four_cc(b"DXT2");
    const DXT3: u32 = four_cc(b"DXT3");
    const DXT4: u32 = four_cc(b"DXT4");
    const DXT5: u32 = four_cc(b"DXT5");
    const MET1: u32 = four_cc(b"MET1");
    match format {
        0 | 100 | UYVY | RGBG | YUY2 | GRGB | DXT1 | DXT2 | DXT3 | DXT4 | DXT5 | MET1 => 0,
        20 => 24,
        21 | 22 | 31 | 32 | 33 | 35 | 62 | 63 | 64 | 67 | 71 | 75 | 77 | 79 | 82 | 83 | 102
        | 112 | 114 => 32,
        23 | 24 | 25 | 26 | 29 | 30 | 34 | 40 | 51 | 60 | 61 | 70 | 73 | 80 | 81 | 101 | 111
        | 117 => 16,
        27 | 28 | 41 | 50 | 52 => 8,
        36 | 110 | 113 | 115 => 64,
        116 => 128,
        _ => -1,
    }
}

/// IDA: 00774AE0 depth and stencil bits for the 00B29944 formats: 70 and 80 -> 16/0, D32 -> 32/0, D15S1 -> 15/1, D24S8 ->
/// 24/8, D24X8 -> 24/0, D24X4S4 -> 24/4, others 0/0. Completion: 100%.
#[must_use]
pub const fn dx9_depth_stencil_bits(format: u32) -> (i32, i32) {
    match format {
        D3DFMT_D16 | D3DFMT_D16_LOCKABLE => (16, 0),
        D3DFMT_D32 => (32, 0),
        D3DFMT_D15S1 => (15, 1),
        D3DFMT_D24S8 => (24, 8),
        D3DFMT_D24X8 => (24, 0),
        D3DFMT_D24X4S4 => (24, 4),
        _ => (0, 0),
    }
}

/// IDA: 00B29944 the nine candidate depth/stencil formats in table order: 70, 71, 73, 75, 77, 79, 82, 83, 80, read from
/// the exact executable and matching the P04-RUN-001 order at 007755B1. The C++ table names its first entry `D3DFMT_D16`
/// (80) and its last `D3DFMT_D16_LOCKABLE` (70), which swaps them; these are the original's raw values.
pub const DX9_DEPTH_STENCIL_CANDIDATES: [u32; 9] = [
    D3DFMT_D16_LOCKABLE,
    D3DFMT_D32,
    D3DFMT_D15S1,
    D3DFMT_D24S8,
    D3DFMT_D24X8,
    D3DFMT_D24X4S4,
    D3DFMT_D32F_LOCKABLE,
    D3DFMT_D24FS8,
    D3DFMT_D16,
];

/// IDA: D3DCAPS9 (0x130 bytes), renderer +0x284 and NiDX9DeviceDesc +0x004. Every field stays a raw dword in original
/// order, so a field read as a float (`max_vertex_w`, the guard band, `extents_adjust`, `max_point_size`,
/// `pixel_shader_1x_max_value`) keeps its bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D3dCaps9 {
    pub device_type: u32,
    pub adapter_ordinal: u32,
    pub caps: u32,
    pub caps2: u32,
    pub caps3: u32,
    pub presentation_intervals: u32,
    pub cursor_caps: u32,
    pub dev_caps: u32,
    pub primitive_misc_caps: u32,
    pub raster_caps: u32,
    pub z_cmp_caps: u32,
    pub src_blend_caps: u32,
    pub dest_blend_caps: u32,
    pub alpha_cmp_caps: u32,
    pub shade_caps: u32,
    pub texture_caps: u32,
    pub texture_filter_caps: u32,
    pub cube_texture_filter_caps: u32,
    pub volume_texture_filter_caps: u32,
    pub texture_address_caps: u32,
    pub volume_texture_address_caps: u32,
    pub line_caps: u32,
    pub max_texture_width: u32,
    pub max_texture_height: u32,
    pub max_volume_extent: u32,
    pub max_texture_repeat: u32,
    pub max_texture_aspect_ratio: u32,
    pub max_anisotropy: u32,
    pub max_vertex_w: u32,
    pub guard_band_left: u32,
    pub guard_band_top: u32,
    pub guard_band_right: u32,
    pub guard_band_bottom: u32,
    pub extents_adjust: u32,
    pub stencil_caps: u32,
    pub fvf_caps: u32,
    pub texture_op_caps: u32,
    pub max_texture_blend_stages: u32,
    pub max_simultaneous_textures: u32,
    pub vertex_processing_caps: u32,
    pub max_active_lights: u32,
    pub max_user_clip_planes: u32,
    pub max_vertex_blend_matrices: u32,
    pub max_vertex_blend_matrix_index: u32,
    pub max_point_size: u32,
    pub max_primitive_count: u32,
    pub max_vertex_index: u32,
    pub max_streams: u32,
    pub max_stream_stride: u32,
    pub vertex_shader_version: u32,
    pub max_vertex_shader_const: u32,
    pub pixel_shader_version: u32,
    pub pixel_shader_1x_max_value: u32,
    pub dev_caps2: u32,
    pub max_npatch_tessellation_level: u32,
    pub reserved5: u32,
    pub master_adapter_ordinal: u32,
    pub adapter_ordinal_in_group: u32,
    pub number_of_adapters_in_group: u32,
    pub decl_types: u32,
    pub num_simultaneous_rts: u32,
    pub stretch_rect_filter_caps: u32,
    pub vs20_caps: u32,
    pub vs20_dynamic_flow_control_depth: u32,
    pub vs20_num_temps: u32,
    pub vs20_static_flow_control_depth: u32,
    pub ps20_caps: u32,
    pub ps20_dynamic_flow_control_depth: u32,
    pub ps20_num_temps: u32,
    pub ps20_static_flow_control_depth: u32,
    pub ps20_num_instruction_slots: u32,
    pub vertex_texture_filter_caps: u32,
    pub max_vshader_instructions_executed: u32,
    pub max_pshader_instructions_executed: u32,
    pub max_vertex_shader30_instruction_slots: u32,
    pub max_pixel_shader30_instruction_slots: u32,
}

/// IDA: D3DADAPTER_IDENTIFIER9 as read: Driver at +0x000 and Description at +0x200 (NiDX9AdapterDesc +0x004 and +0x204).
/// C strings, so raw bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdapterIdentifier {
    pub driver: Vec<u8>,
    pub description: Vec<u8>,
}

/// IDA: D3DDISPLAYMODE.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_rate: u32,
    pub format: u32,
}

/// IDA: D3DSURFACE_DESC as read: format, width and height.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceDesc {
    pub format: u32,
    pub width: u32,
    pub height: u32,
}

/// A host handle for a window. Never compared with evidence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowHandle(pub u32);
/// A host handle for a loaded module.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModuleHandle(pub u32);
/// A host handle for a resolved entry point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcHandle(pub u32);
/// A host handle for a surface the device returned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceHandle(pub u32);
/// A host handle for a texture the device created.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureHandle(pub u32);

/// IDA: D3DPRESENT_PARAMETERS (0x38 bytes), in field order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PresentParameters {
    pub back_buffer_width: u32,
    pub back_buffer_height: u32,
    pub back_buffer_format: u32,
    pub back_buffer_count: u32,
    pub multi_sample_type: u32,
    pub multi_sample_quality: u32,
    pub swap_effect: u32,
    pub device_window: Option<WindowHandle>,
    pub windowed: bool,
    pub enable_auto_depth_stencil: bool,
    pub auto_depth_stencil_format: u32,
    pub flags: u32,
    pub full_screen_refresh_rate: u32,
    pub presentation_interval: u32,
}

/// IDA: D3DMATRIX, row by row.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct D3dMatrix {
    pub _11: f32,
    pub _12: f32,
    pub _13: f32,
    pub _14: f32,
    pub _21: f32,
    pub _22: f32,
    pub _23: f32,
    pub _24: f32,
    pub _31: f32,
    pub _32: f32,
    pub _33: f32,
    pub _34: f32,
    pub _41: f32,
    pub _42: f32,
    pub _43: f32,
    pub _44: f32,
}

impl D3dMatrix {
    /// IDA: N/A the matrix as raw float bits, row by row, for a trace entry.
    #[must_use]
    pub fn bits(&self) -> [u32; 16] {
        [
            self._11, self._12, self._13, self._14, self._21, self._22, self._23, self._24,
            self._31, self._32, self._33, self._34, self._41, self._42, self._43, self._44,
        ]
        .map(f32::to_bits)
    }
}

/// IDA: D3DVIEWPORT9.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct D3dViewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub min_z: f32,
    pub max_z: f32,
}

/// IDA: N/A the host seam for `IDirect3D9`. A method answers what the original call would have: an HRESULT as `u32`, and
/// any out values. A `DisplayMode` is meaningful only on success. `create_device` may rewrite the parameters, as the real
/// call does to the back buffer count, which is what 0076A58B's retry tests.
pub trait Direct3D9: Send + Sync {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn adapter_count(&mut self) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn adapter_identifier(&mut self, adapter: u32, flags: u32) -> (u32, AdapterIdentifier);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn adapter_mode_count(&mut self, adapter: u32, format: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn enum_adapter_modes(&mut self, adapter: u32, format: u32, mode: u32) -> (u32, DisplayMode);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn adapter_display_mode(&mut self, adapter: u32) -> (u32, DisplayMode);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn device_caps(&mut self, adapter: u32, device_type: u32) -> (u32, D3dCaps9);
    /// IDA: N/A no address of its own; its comments cite 0076DED6.
    fn check_device_type(
        &mut self,
        adapter: u32,
        device_type: u32,
        display_format: u32,
        back_buffer_format: u32,
        windowed: bool,
    ) -> u32;
    /// IDA: N/A no address of its own; its comments cite 0076DED6.
    fn check_device_format(
        &mut self,
        adapter: u32,
        device_type: u32,
        adapter_format: u32,
        usage: u32,
        resource_type: u32,
        format: u32,
    ) -> u32;
    /// IDA: N/A no address of its own; its comments cite 0076DED6.
    fn check_device_multi_sample_type(
        &mut self,
        adapter: u32,
        device_type: u32,
        format: u32,
        windowed: bool,
        multi_sample_type: u32,
    ) -> (u32, u32);
    /// IDA: N/A no address of its own; its comments cite 0076DED6.
    fn check_depth_stencil_match(
        &mut self,
        adapter: u32,
        device_type: u32,
        adapter_format: u32,
        render_target_format: u32,
        depth_stencil_format: u32,
    ) -> u32;
    /// IDA: N/A no address of its own; its comments cite 0076DED6.
    fn create_device(
        &mut self,
        adapter: u32,
        device_type: u32,
        focus_window: Option<WindowHandle>,
        behavior_flags: u32,
        parameters: &mut PresentParameters,
    ) -> (u32, Option<Box<dyn Direct3DDevice9>>);
}

/// IDA: N/A the host seam for `IDirect3DDevice9` and the surfaces and textures it hands out. A surface's bytes cross at
/// `unlock_rect`, as the block the original wrote between LockRect and UnlockRect.
pub trait Direct3DDevice9: Send + Sync {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn device_caps(&mut self) -> (u32, D3dCaps9);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn render_target(&mut self, index: u32) -> (u32, Option<SurfaceHandle>);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn depth_stencil_surface(&mut self) -> (u32, Option<SurfaceHandle>);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn surface_desc(&mut self, surface: SurfaceHandle) -> (u32, SurfaceDesc);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn set_render_state(&mut self, state: u32, value: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn set_texture_stage_state(&mut self, stage: u32, state: u32, value: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn set_sampler_state(&mut self, stage: u32, state: u32, value: u32) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn set_transform(&mut self, state: u32, matrix: &D3dMatrix) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn set_viewport(&mut self, viewport: &D3dViewport) -> u32;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn set_npatch_mode(&mut self, segments: f32) -> u32;
    /// IDA: N/A no address of its own; its comments cite 0076DED6.
    fn create_texture(
        &mut self,
        width: u32,
        height: u32,
        levels: u32,
        usage: u32,
        format: u32,
        pool: u32,
    ) -> (u32, Option<TextureHandle>);
    /// IDA: N/A no address of its own; its comments cite 0076DED6.
    fn create_cube_texture(
        &mut self,
        edge: u32,
        levels: u32,
        usage: u32,
        format: u32,
        pool: u32,
    ) -> (u32, Option<TextureHandle>);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn texture_type(&mut self, texture: TextureHandle) -> u32;
    /// IDA: N/A no address of its own; its comments cite 0076DED6.
    fn texture_surface_level(
        &mut self,
        texture: TextureHandle,
        level: u32,
    ) -> (u32, Option<SurfaceHandle>);
    /// IDA: N/A no address of its own; its comments cite 0076DED6.
    fn cube_map_surface(
        &mut self,
        texture: TextureHandle,
        face: u32,
        level: u32,
    ) -> (u32, Option<SurfaceHandle>);
    /// IDA: 0076DED6 CreateDepthStencilSurface(width, height, format, multi sample, quality, discard, &surface, NULL).
    fn create_depth_stencil_surface(
        &mut self,
        width: u32,
        height: u32,
        format: u32,
        multi_sample: u32,
        multi_sample_quality: u32,
        discard: bool,
    ) -> (u32, Option<SurfaceHandle>);
    /// IDA: LockRect(NULL, 0). Answers the HRESULT and the pitch.
    fn lock_rect(&mut self, surface: SurfaceHandle) -> (u32, i32);
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn unlock_rect(&mut self, surface: SurfaceHandle, bytes: &[u8]) -> u32;
}

/// One device or Win32 call the C++ issues, with its arguments. Windows values stay plain numbers; float arguments are
/// raw bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceCall {
    LoadLibrary {
        name: String,
    },
    GetProcAddress {
        name: String,
    },
    FreeLibrary,
    Direct3DCreate9 {
        sdk_version: u32,
    },
    ReleaseD3d9,
    GetAdapterCount,
    GetAdapterIdentifier {
        adapter: u32,
        flags: u32,
    },
    GetAdapterModeCount {
        adapter: u32,
        format: u32,
    },
    EnumAdapterModes {
        adapter: u32,
        format: u32,
        mode: u32,
    },
    GetAdapterDisplayMode {
        adapter: u32,
    },
    GetDeviceCaps {
        adapter: u32,
        device_type: u32,
    },
    CheckDeviceType {
        adapter: u32,
        device_type: u32,
        display_format: u32,
        back_buffer_format: u32,
        windowed: bool,
    },
    CheckDeviceFormat {
        adapter: u32,
        device_type: u32,
        adapter_format: u32,
        usage: u32,
        resource_type: u32,
        format: u32,
    },
    CheckDeviceMultiSampleType {
        adapter: u32,
        device_type: u32,
        format: u32,
        windowed: bool,
        multi_sample_type: u32,
    },
    CheckDepthStencilMatch {
        adapter: u32,
        device_type: u32,
        adapter_format: u32,
        render_target_format: u32,
        depth_stencil_format: u32,
    },
    CreateDevice {
        adapter: u32,
        device_type: u32,
        behavior_flags: u32,
        parameters: PresentParameters,
    },
    DeviceGetDeviceCaps,
    GetRenderTarget {
        index: u32,
    },
    GetDepthStencilSurface,
    SurfaceGetDesc,
    SetRenderState {
        state: u32,
        value: u32,
    },
    SetTextureStageState {
        stage: u32,
        state: u32,
        value: u32,
    },
    SetSamplerState {
        stage: u32,
        state: u32,
        value: u32,
    },
    SetTransform {
        state: u32,
        matrix_bits: [u32; 16],
    },
    SetViewport {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        min_z_bits: u32,
        max_z_bits: u32,
    },
    SetNPatchMode {
        segments_bits: u32,
    },
    CreateTexture {
        width: u32,
        height: u32,
        levels: u32,
        usage: u32,
        format: u32,
        pool: u32,
    },
    CreateCubeTexture {
        edge: u32,
        levels: u32,
        usage: u32,
        format: u32,
        pool: u32,
    },
    CreateDepthStencilSurface {
        width: u32,
        height: u32,
        format: u32,
        multi_sample: u32,
        multi_sample_quality: u32,
        discard: bool,
    },
    /// IDA: 00760EF5 D3DXGetImageInfoFromFileInMemory(data, size, &info). Not in the executable; the answer is `direct_image_info`.
    D3dxGetImageInfoFromFileInMemory {
        size: u32,
    },
    /// IDA: 007610D3, 007610F9, 0076111E D3DXCreate(Cube|Volume)TextureFromFileInMemory(device, data, original size, &texture).
    /// `resource_type` is the D3DRTYPE that picked the export.
    D3dxCreateTextureFromFileInMemory {
        resource_type: u32,
        size: u32,
    },
    /// IDA: 007609D3 vt+0x34 GetLevelCount.
    TextureGetLevelCount,
    /// IDA: 00760A0C vt+0x44, 00760AB3 vt+0x48 GetLevelDesc(level).
    TextureGetLevelDesc {
        level: u32,
    },
    TextureGetType,
    TextureGetSurfaceLevel {
        level: u32,
    },
    CubeGetSurface {
        face: u32,
        level: u32,
    },
    LockRect,
    UnlockRect {
        pitch: i32,
        bytes: usize,
    },
    CreateWindow {
        ex_style: u32,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    GetClassLong {
        index: i32,
    },
    GetWindowLong {
        index: i32,
    },
    AdjustWindowRect {
        rect: [i32; 4],
        style: u32,
        menu: bool,
    },
    SetWindowPos {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    },
    AtiQueryMgpuCount,
    NvCplGetDataInt {
        key: i32,
    },
    WriteRendererInfo {
        bytes: usize,
    },
}

/// One call in original order. `site` is the call's own address where P04-RUN-001 or the disassembly gives it, otherwise
/// the enclosing function's. `result` is the answer the port saw (an HRESULT, or the plain value the call returns).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceTraceEntry {
    pub site: u32,
    pub call: DeviceCall,
    pub result: u32,
}

/// IDA: NiDX9DeviceDesc::DisplayFormatInfo::RenderTargetInfo depth/stencil entry: {byte valid, byte quality for type 1,
/// word mask}.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dx9DepthStencilEntry {
    pub valid: u8,
    pub quality: u8,
    pub multisample_mask: u16,
}

/// IDA: NiDX9DeviceDesc::DisplayFormatInfo::RenderTargetInfo (0x48 bytes): +0x00 windowed and +0x24 fullscreen
/// depth/stencil entries in 00B29944 order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dx9RenderTargetInfo {
    pub windowed_00: [Dx9DepthStencilEntry; 9],
    pub fullscreen_24: [Dx9DepthStencilEntry; 9],
}

/// IDA: 007753F0 NiDX9DeviceDesc::DisplayFormatInfo (0x18 bytes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dx9DisplayFormatInfo {
    pub format_00: u32,
    /// CheckDeviceType windowed.
    pub windowed_04: bool,
    /// Always true when the mode enumerated.
    pub fullscreen_05: bool,
    /// +0x08 NiTPointerMap<D3DFORMAT, RenderTargetInfo*> (37 buckets), for lookups only.
    pub render_targets_08: BTreeMap<u32, Dx9RenderTargetInfo>,
    /// Host record of the map's insertion order (bucket iteration differs).
    pub render_target_order: Vec<u32>,
}

/// IDA: 00775780 NiDX9DeviceDesc (0x148 bytes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dx9DeviceDesc {
    pub device_type_000: u32,
    /// DeviceType 0 when GetDeviceCaps failed.
    pub caps_004: D3dCaps9,
    /// Host: GetDeviceCaps succeeded.
    pub caps_ok: bool,
    /// +0x134 list, in append order.
    pub display_formats_134: Vec<Dx9DisplayFormatInfo>,
    pub renders_windowed_144: bool,
}

/// IDA: 007759A0 NiDX9AdapterDesc::ModeDesc (0x20 bytes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dx9ModeDesc {
    pub width: u32,
    pub height: u32,
    pub bits_per_pixel: i32,
    pub format: u32,
    /// NiTPointerList in enumeration order.
    pub refresh_rates: Vec<u32>,
}

/// IDA: 00775A20 NiDX9AdapterDesc (0x468 bytes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dx9AdapterDesc {
    pub ordinal_000: u32,
    /// +0x004 (0x44C bytes).
    pub identifier_004: AdapterIdentifier,
    /// +0x450 NiTArray (max 32, grow 16).
    pub modes_450: Vec<Dx9ModeDesc>,
    pub hal_460: Option<Dx9DeviceDesc>,
    pub reference_464: Option<Dx9DeviceDesc>,
}

/// IDA: 00775DF0 NiDX9AdapterEnumerator (0x14 bytes): +0 count, +4 NiTArray. Shared with `RendererCreation`, because the
/// callers keep reading an adapter while storing into the renderer's other state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dx9SystemDesc {
    pub adapter_count: u32,
    pub adapters: Vec<Option<Dx9AdapterDesc>>,
}

impl Dx9SystemDesc {
    /// IDA: 004980E0..0049811D/00498660..00498689: index < count and < the array size, else null. Completion: 100%.
    #[must_use]
    pub fn adapter(&self, index: u32) -> Option<&Dx9AdapterDesc> {
        if index < self.adapter_count && (index as usize) < self.adapters.len() {
            self.adapters[index as usize].as_ref()
        } else {
            None
        }
    }
}

/// IDA: 007753F0 DisplayFormatInfo constructor (a5 display format, a6 fullscreen, a7 windowed). For each FrameBufferFormat
/// 1..17 CheckDeviceFormat(adapter, type, display, RENDERTARGET, SURFACE, rt) >= 0 -> a zeroed 0x48 RenderTargetInfo; the
/// windowed and fullscreen multisample masks of `display` for types 1..16; then for each 00B29944 depth format
/// CheckDeviceFormat(.., display, DEPTHSTENCIL, SURFACE, ds) >= 0 and CheckDepthStencilMatch >= 0 -> windowed valid = a7,
/// fullscreen valid = a6, and for types 1..16 CheckDeviceMultiSampleType(adapter, type, ds, TRUE/FALSE, type) sets the
/// masks and, for type 1, the quality byte. An RT with no depth match is freed. Completion: 100%.
fn make_display_format_info(
    d3d9: &mut dyn Direct3D9,
    adapter: u32,
    device_type: u32,
    display: u32,
    fullscreen: bool,
    windowed: bool,
    trace: &mut Vec<DeviceTraceEntry>,
) -> Dx9DisplayFormatInfo {
    let mut info = Dx9DisplayFormatInfo {
        format_00: display,
        windowed_04: windowed,
        fullscreen_05: fullscreen,
        ..Dx9DisplayFormatInfo::default()
    };
    for index in 0..0x12 {
        if index == 0 {
            continue;
        }
        let rt = dx9_frame_buffer_format(index);
        let result = d3d9.check_device_format(
            adapter,
            device_type,
            display,
            D3DUSAGE_RENDERTARGET,
            D3DRTYPE_SURFACE,
            rt,
        );
        trace.push(DeviceTraceEntry {
            site: 0x0077_5493,
            call: DeviceCall::CheckDeviceFormat {
                adapter,
                device_type,
                adapter_format: display,
                usage: D3DUSAGE_RENDERTARGET,
                resource_type: D3DRTYPE_SURFACE,
                format: rt,
            },
            result,
        });
        if failed(result) {
            continue;
        }
        let mut value = Dx9RenderTargetInfo::default();
        let mut windowed_mask = 0u32;
        let mut fullscreen_mask = 0u32;
        for i in 0..0x10u32 {
            if windowed {
                let (result, _) =
                    d3d9.check_device_multi_sample_type(adapter, device_type, display, true, i + 1);
                trace.push(DeviceTraceEntry {
                    site: 0x0077_5517,
                    call: DeviceCall::CheckDeviceMultiSampleType {
                        adapter,
                        device_type,
                        format: display,
                        windowed: true,
                        multi_sample_type: i + 1,
                    },
                    result,
                });
                if !failed(result) {
                    windowed_mask |= 1 << i;
                }
            }
            if fullscreen {
                let (result, _) = d3d9.check_device_multi_sample_type(
                    adapter,
                    device_type,
                    display,
                    false,
                    i + 1,
                );
                trace.push(DeviceTraceEntry {
                    site: 0x0077_554D,
                    call: DeviceCall::CheckDeviceMultiSampleType {
                        adapter,
                        device_type,
                        format: display,
                        windowed: false,
                        multi_sample_type: i + 1,
                    },
                    result,
                });
                if !failed(result) {
                    fullscreen_mask |= 1 << i;
                }
            }
        }
        let mut any = false;
        for (k, &ds) in DX9_DEPTH_STENCIL_CANDIDATES.iter().enumerate() {
            let result = d3d9.check_device_format(
                adapter,
                device_type,
                display,
                D3DUSAGE_DEPTHSTENCIL,
                D3DRTYPE_SURFACE,
                ds,
            );
            trace.push(DeviceTraceEntry {
                site: 0x0077_55B1,
                call: DeviceCall::CheckDeviceFormat {
                    adapter,
                    device_type,
                    adapter_format: display,
                    usage: D3DUSAGE_DEPTHSTENCIL,
                    resource_type: D3DRTYPE_SURFACE,
                    format: ds,
                },
                result,
            });
            if failed(result) {
                continue;
            }
            let result = d3d9.check_depth_stencil_match(adapter, device_type, display, rt, ds);
            trace.push(DeviceTraceEntry {
                site: 0x0077_55D2,
                call: DeviceCall::CheckDepthStencilMatch {
                    adapter,
                    device_type,
                    adapter_format: display,
                    render_target_format: rt,
                    depth_stencil_format: ds,
                },
                result,
            });
            if failed(result) {
                continue;
            }
            value.windowed_00[k].valid = u8::from(windowed);
            any = true;
            value.fullscreen_24[k].valid = u8::from(fullscreen);
            for sample in 1..=16u32 {
                let bit = 1u32 << (sample - 1);
                // The original's `mask | bit` tests are always true.
                if (windowed_mask | bit) != 0 {
                    let (result, level) =
                        d3d9.check_device_multi_sample_type(adapter, device_type, ds, true, sample);
                    trace.push(DeviceTraceEntry {
                        site: 0x0077_562F,
                        call: DeviceCall::CheckDeviceMultiSampleType {
                            adapter,
                            device_type,
                            format: ds,
                            windowed: true,
                            multi_sample_type: sample,
                        },
                        result,
                    });
                    if !failed(result) {
                        if sample == 1 {
                            value.windowed_00[k].quality = level as u8;
                        }
                        value.windowed_00[k].multisample_mask |= bit as u16;
                    }
                }
                if (fullscreen_mask | bit) != 0 {
                    let (result, level) = d3d9.check_device_multi_sample_type(
                        adapter,
                        device_type,
                        ds,
                        false,
                        sample,
                    );
                    trace.push(DeviceTraceEntry {
                        site: 0x0077_566A,
                        call: DeviceCall::CheckDeviceMultiSampleType {
                            adapter,
                            device_type,
                            format: ds,
                            windowed: false,
                            multi_sample_type: sample,
                        },
                        result,
                    });
                    if !failed(result) {
                        if sample == 1 {
                            value.fullscreen_24[k].quality = level as u8;
                        }
                        value.fullscreen_24[k].multisample_mask |= bit as u16;
                    }
                }
            }
        }
        if any {
            // 004525xx NiTMapUInt32UInt8_SetAt: a new key appends.
            if !info.render_targets_08.contains_key(&rt) {
                info.render_target_order.push(rt);
            }
            info.render_targets_08.insert(rt, value);
        }
    }
    info
}

/// IDA: 00775780 NiDX9DeviceDesc(d3d9, adapter, type, desktop format, formats): GetDeviceCaps(adapter, type) < 0 ->
/// caps.DeviceType = 0 and no formats. Otherwise for every listed format and mode: EnumAdapterModes >= 0 ->
/// CheckDeviceType(adapter, type, mode, mode, FALSE) (result ignored, the mode counts as fullscreen-capable); then
/// CheckDeviceType(adapter, type, desktop, mode, TRUE) >= 0 -> windowed; when either, append a new DisplayFormatInfo for a
/// mode format not yet listed. +0x144 = the desktop format's info has the windowed flag. Completion: 100%.
fn make_device_desc(
    d3d9: &mut dyn Direct3D9,
    adapter: u32,
    device_type: u32,
    desktop: u32,
    formats: &[u32],
    trace: &mut Vec<DeviceTraceEntry>,
) -> Dx9DeviceDesc {
    let mut device = Dx9DeviceDesc {
        device_type_000: device_type,
        ..Dx9DeviceDesc::default()
    };
    // 00775780 keeps the D3DDISPLAYMODE on its stack across iterations, so a failed EnumAdapterModes reuses the previous
    // mode's format in the windowed check. Host substitution: the first failure would read stack bytes; here it reads zero.
    let mut desc = DisplayMode::default();
    let (result, caps) = d3d9.device_caps(adapter, device_type);
    trace.push(DeviceTraceEntry {
        site: 0x0077_57BF,
        call: DeviceCall::GetDeviceCaps {
            adapter,
            device_type,
        },
        result,
    });
    if failed(result) {
        device.caps_004.device_type = 0;
    } else {
        device.caps_004 = caps;
        device.caps_ok = true;
        for &format in formats {
            if format == D3DFMT_UNKNOWN {
                continue;
            }
            let count = d3d9.adapter_mode_count(adapter, format);
            trace.push(DeviceTraceEntry {
                site: 0x0077_5803,
                call: DeviceCall::GetAdapterModeCount { adapter, format },
                result: count,
            });
            for mode in 0..count {
                let mut fullscreen = false;
                let mut windowed = false;
                let (result, enumerated) = d3d9.enum_adapter_modes(adapter, format, mode);
                trace.push(DeviceTraceEntry {
                    site: 0x0077_5841,
                    call: DeviceCall::EnumAdapterModes {
                        adapter,
                        format,
                        mode,
                    },
                    result,
                });
                if !failed(result) {
                    desc = enumerated;
                    let result = d3d9.check_device_type(
                        adapter,
                        device_type,
                        desc.format,
                        desc.format,
                        false,
                    );
                    trace.push(DeviceTraceEntry {
                        site: 0x0077_585D,
                        call: DeviceCall::CheckDeviceType {
                            adapter,
                            device_type,
                            display_format: desc.format,
                            back_buffer_format: desc.format,
                            windowed: false,
                        },
                        result,
                    });
                    fullscreen = true;
                }
                let result =
                    d3d9.check_device_type(adapter, device_type, desktop, desc.format, true);
                trace.push(DeviceTraceEntry {
                    site: 0x0077_587F,
                    call: DeviceCall::CheckDeviceType {
                        adapter,
                        device_type,
                        display_format: desktop,
                        back_buffer_format: desc.format,
                        windowed: true,
                    },
                    result,
                });
                if !failed(result) {
                    windowed = true;
                }
                if !fullscreen && !windowed {
                    continue;
                }
                let listed = device
                    .display_formats_134
                    .iter()
                    .any(|info| info.format_00 == desc.format);
                if !listed {
                    let info = make_display_format_info(
                        d3d9,
                        adapter,
                        device_type,
                        desc.format,
                        fullscreen,
                        windowed,
                        trace,
                    );
                    device.display_formats_134.push(info);
                }
            }
        }
    }
    device.renders_windowed_144 = device
        .display_formats_134
        .iter()
        .find(|info| info.format_00 == desktop)
        .is_some_and(|info| info.windowed_04);
    device
}

/// IDA: 007759A0 ModeDesc(mode): width, height, format, the first refresh in the list, bits = 00774EE0(format).
/// Completion: 100%.
fn make_mode(mode: &DisplayMode) -> Dx9ModeDesc {
    Dx9ModeDesc {
        width: mode.width,
        height: mode.height,
        format: mode.format,
        refresh_rates: vec![mode.refresh_rate],
        bits_per_pixel: dx9_format_bits(mode.format),
    }
}

/// IDA: 00775A20 NiDX9AdapterDesc(d3d9, ordinal, formats): GetAdapterIdentifier (flags 0); for every listed format and mode
/// index EnumAdapterModes >= 0 either appends the refresh to every existing ModeDesc with the same format, width and
/// height, or appends a new ModeDesc; GetAdapterDisplayMode; FormHeap(0x148) HAL and REF device descriptors over the
/// desktop format. Completion: 100%.
fn make_adapter_desc(
    d3d9: &mut dyn Direct3D9,
    ordinal: u32,
    formats: &[u32],
    trace: &mut Vec<DeviceTraceEntry>,
) -> Dx9AdapterDesc {
    let (result, identifier) = d3d9.adapter_identifier(ordinal, 0);
    trace.push(DeviceTraceEntry {
        site: 0x0077_5A83,
        call: DeviceCall::GetAdapterIdentifier {
            adapter: ordinal,
            flags: 0,
        },
        result,
    });
    let mut adapter = Dx9AdapterDesc {
        ordinal_000: ordinal,
        identifier_004: identifier,
        ..Dx9AdapterDesc::default()
    };
    adapter.modes_450.reserve(32);
    for &format in formats {
        if format == D3DFMT_UNKNOWN {
            continue;
        }
        let count = d3d9.adapter_mode_count(ordinal, format);
        trace.push(DeviceTraceEntry {
            site: 0x0077_5ABD,
            call: DeviceCall::GetAdapterModeCount {
                adapter: ordinal,
                format,
            },
            result: count,
        });
        for mode in 0..count {
            let (result, desc) = d3d9.enum_adapter_modes(ordinal, format, mode);
            trace.push(DeviceTraceEntry {
                site: 0x0077_5AE8,
                call: DeviceCall::EnumAdapterModes {
                    adapter: ordinal,
                    format,
                    mode,
                },
                result,
            });
            if failed(result) {
                continue;
            }
            let mut appended = false;
            for existing in &mut adapter.modes_450 {
                if existing.format == format
                    && existing.width == desc.width
                    && existing.height == desc.height
                {
                    existing.refresh_rates.push(desc.refresh_rate);
                    appended = true;
                }
            }
            if !appended {
                adapter.modes_450.push(make_mode(&desc));
            }
        }
    }
    let (result, desktop) = d3d9.adapter_display_mode(ordinal);
    trace.push(DeviceTraceEntry {
        site: 0x0077_5C36,
        call: DeviceCall::GetAdapterDisplayMode { adapter: ordinal },
        result,
    });
    adapter.hal_460 = Some(make_device_desc(
        d3d9,
        ordinal,
        D3DDEVTYPE_HAL,
        desktop.format,
        formats,
        trace,
    ));
    adapter.reference_464 = Some(make_device_desc(
        d3d9,
        ordinal,
        D3DDEVTYPE_REF,
        desktop.format,
        formats,
        trace,
    ));
    adapter
}

/// IDA: 00775CC0/00775DF0: count = GetAdapterCount(); NiTArray_SetSize(count); FormHeap(0x468) NiDX9AdapterDesc per ordinal.
/// Completion: 100% (the original's null on a failed allocation is not modelled: the host aborts).
pub fn enumerate_dx9_system(
    d3d9: &mut dyn Direct3D9,
    adapter_formats: &[u32],
    trace: &mut Vec<DeviceTraceEntry>,
) -> Arc<Dx9SystemDesc> {
    let adapter_count = d3d9.adapter_count();
    trace.push(DeviceTraceEntry {
        site: 0x0077_5CD1,
        call: DeviceCall::GetAdapterCount,
        result: adapter_count,
    });
    let adapters = (0..adapter_count)
        .map(|ordinal| Some(make_adapter_desc(d3d9, ordinal, adapter_formats, trace)))
        .collect();
    Arc::new(Dx9SystemDesc {
        adapter_count,
        adapters,
    })
}

/// IDA: 007751F0(windowed, bits): windowed -> the first display-format info with the windowed flag; otherwise the first
/// with the fullscreen flag and bits == wanted; returns 00774990 of its format (0 when none). Completion: 100% static
/// (P04-RUN-001 passed an explicit format).
#[must_use]
pub fn dx9_find_frame_buffer_format(device: &Dx9DeviceDesc, windowed: bool, bits: i32) -> i32 {
    device
        .display_formats_134
        .iter()
        .find(|info| {
            if windowed {
                info.windowed_04
            } else {
                info.fullscreen_05 && dx9_format_bits(info.format_00) == bits
            }
        })
        .map_or(0, |info| dx9_frame_buffer_index(info.format_00))
}

/// IDA: 00775280(adapter format, rt, bits, stencil): the first display info with that format; stencil 0 -> 007750F0(rt,
/// bits, 0); bits > 16 -> stencil 1 ? (24, 1) : (24, 8); else stencil 1 ? (15, 1) : (24, 8). 007750F0: the fullscreen
/// entries (+0x24) of the RT info; for each valid candidate keep it when |depth - bits| <= best and |stencil - wanted| <=
/// best (later wins). Completion: 100% static (P04-RUN-001 passed D24S8 explicitly).
#[must_use]
pub fn dx9_find_depth_stencil_format(
    device: &Dx9DeviceDesc,
    adapter_format: u32,
    render_target: u32,
    bits: i32,
    stencil: i32,
) -> u32 {
    let Some(info) = device
        .display_formats_134
        .iter()
        .find(|candidate| candidate.format_00 == adapter_format)
    else {
        return D3DFMT_UNKNOWN;
    };
    let (want_bits, want_stencil) = if stencil == 0 {
        (bits, stencil)
    } else if bits > 0x10 {
        (24, if stencil == 1 { 1 } else { 8 })
    } else if stencil != 1 {
        (24, 8)
    } else {
        (15, 1)
    };
    let Some(found) = info.render_targets_08.get(&render_target) else {
        return D3DFMT_UNKNOWN;
    };
    let mut best_depth = 0i32;
    let mut best_stencil = 0i32;
    let mut best = D3DFMT_UNKNOWN;
    for (i, &candidate) in DX9_DEPTH_STENCIL_CANDIDATES.iter().enumerate() {
        if found.fullscreen_24[i].valid == 0 {
            continue;
        }
        let (depth, stencil_bits) = dx9_depth_stencil_bits(candidate);
        if (best_depth - want_bits).abs() >= (depth - want_bits).abs()
            && (best_stencil - want_stencil).abs() >= (stencil_bits - want_stencil).abs()
        {
            best_depth = depth;
            best_stencil = stencil_bits;
            best = candidate;
        }
    }
    best
}

/// IDA: 00775320(format, w, h, &refresh): the last mode with that format and size wins; *refresh = 00775090 nearest refresh
/// (0 stays 0). Returns whether a mode matched. 00775090: the result starts at 0 and only later entries strictly nearer
/// than the first replace it. Completion: 100% static (fullscreen only).
pub fn dx9_match_fullscreen_mode(
    adapter: &Dx9AdapterDesc,
    format: u32,
    width: u32,
    height: u32,
    refresh: &mut u32,
) -> bool {
    let mut matched = false;
    for mode in &adapter.modes_450 {
        if mode.format != format || mode.width != width || mode.height != height {
            continue;
        }
        matched = true;
        if *refresh == 0 {
            continue;
        }
        let wanted = *refresh as i32;
        let mut best = (mode.refresh_rates[0] as i32)
            .wrapping_sub(wanted)
            .wrapping_abs() as u32;
        let mut result = 0;
        for &rate in &mode.refresh_rates[1..] {
            let distance = (rate as i32).wrapping_sub(wanted).wrapping_abs() as u32;
            if distance < best {
                best = distance;
                result = rate;
            }
        }
        *refresh = result;
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_buffer_formats_round_trip() {
        assert_eq!(dx9_frame_buffer_format(3), D3DFMT_X8R8G8B8);
        assert_eq!(dx9_frame_buffer_format(12), D3DFMT_R16F);
        assert_eq!(dx9_frame_buffer_format(17), D3DFMT_A32B32G32R32F);
        assert_eq!(dx9_frame_buffer_format(18), D3DFMT_UNKNOWN);
        assert_eq!(dx9_frame_buffer_index(D3DFMT_X8R8G8B8), 3);
        assert_eq!(dx9_frame_buffer_index(D3DFMT_R16F), 12);
        assert_eq!(dx9_frame_buffer_index(50), 0);
    }

    #[test]
    fn format_bits_and_depth_candidates() {
        assert_eq!(dx9_format_bits(D3DFMT_X8R8G8B8), 32);
        assert_eq!(dx9_format_bits(D3DFMT_DXT1), 0);
        assert_eq!(dx9_format_bits(0x1234), -1);
        assert_eq!(DX9_DEPTH_STENCIL_CANDIDATES[0], 70);
        assert_eq!(DX9_DEPTH_STENCIL_CANDIDATES[8], 80);
        assert_eq!(dx9_depth_stencil_bits(D3DFMT_D24S8), (24, 8));
        assert_eq!(dx9_depth_stencil_bits(D3DFMT_D32F_LOCKABLE), (0, 0));
    }

    #[test]
    fn fullscreen_mode_keeps_the_original_refresh_rule() {
        let adapter = Dx9AdapterDesc {
            modes_450: vec![Dx9ModeDesc {
                width: 640,
                height: 480,
                format: D3DFMT_X8R8G8B8,
                bits_per_pixel: 32,
                refresh_rates: vec![60, 75, 85],
            }],
            ..Dx9AdapterDesc::default()
        };
        let mut refresh = 74;
        assert!(dx9_match_fullscreen_mode(
            &adapter,
            D3DFMT_X8R8G8B8,
            640,
            480,
            &mut refresh
        ));
        assert_eq!(refresh, 75);
        // The first entry being nearest leaves the result at 0.
        let mut refresh = 60;
        assert!(dx9_match_fullscreen_mode(
            &adapter,
            D3DFMT_X8R8G8B8,
            640,
            480,
            &mut refresh
        ));
        assert_eq!(refresh, 0);
        let mut refresh = 60;
        assert!(!dx9_match_fullscreen_mode(
            &adapter,
            D3DFMT_X8R8G8B8,
            800,
            600,
            &mut refresh
        ));
        assert_eq!(refresh, 60);
    }

    #[test]
    fn depth_stencil_choice_prefers_the_later_equal_candidate() {
        let mut target = Dx9RenderTargetInfo::default();
        // D24S8 (index 3) and D24X8 (index 4) valid; a 32-bit stencil request wants (24, 8).
        target.fullscreen_24[3].valid = 1;
        target.fullscreen_24[4].valid = 1;
        let mut info = Dx9DisplayFormatInfo {
            format_00: D3DFMT_X8R8G8B8,
            ..Dx9DisplayFormatInfo::default()
        };
        info.render_targets_08.insert(D3DFMT_X8R8G8B8, target);
        let device = Dx9DeviceDesc {
            display_formats_134: vec![info],
            ..Dx9DeviceDesc::default()
        };
        assert_eq!(
            dx9_find_depth_stencil_format(&device, D3DFMT_X8R8G8B8, D3DFMT_X8R8G8B8, 32, 8),
            D3DFMT_D24S8
        );
        assert_eq!(
            dx9_find_depth_stencil_format(&device, D3DFMT_R5G6B5, D3DFMT_X8R8G8B8, 32, 8),
            D3DFMT_UNKNOWN
        );
    }
}
