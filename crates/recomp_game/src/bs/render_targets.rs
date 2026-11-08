//! BSTextureManager and its render targets: 007C1FF0 (the manager), 007C2450, called from 00406950 at 00406B4E (ecx
//! 00B42F50, the 00B43104 argument unread), and the rendered-texture chain it reaches: 007C1B50 (cached slots),
//! 007C23C0/007C0D10 (kinds), 007C1430 (creation and its warning), 007C1EE0 (release to the free list),
//! BSRenderedTexture 007D6F40/007D6BD0, NiRenderedTexture 0072A9B0, NiRenderedCubeMap 009A1CE0, NiDX9Renderer
//! vt+0x108/+0x110 (00768700/00768740), NiDX9RenderedTextureData 00761920/00761730, NiDX9RenderedCubeMapData
//! 007792E0/00779210, the 00773960 format search, NiDX9TextureBufferData 0076D8C0/0076D9B0, NiRenderTargetGroup 009A21D0
//! and NiDepthStencilBuffer 0070BE00.
//!
//! No GPU target is created here. The 13 targets stay records and byte accounting, and each device call goes through the
//! `Direct3DDevice9` seam onto a `DeviceTraceEntry` list, so its failure branch can still be scripted. The C++ keeps the
//! manager in `bs/shader_manager`; it lives here so that the shader system does not have to carry it.
//!
//! Host substitutions: the QueryInterface calls 007619AF and 0076D8xx have no seam, because a `TextureHandle` has one
//! interface, and their failure arms are not reachable. An object's AddRef and Release are not modelled. A deleted object
//! is one this file never spawned, and a target dropped from the manager stays spawned (no consumer releases one yet).

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use crate::gamebryo::dx9_system::{
    D3DPOOL_DEFAULT, D3DPOOL_MANAGED, D3DRTYPE_SURFACE, D3DUSAGE_DEPTHSTENCIL,
    D3DUSAGE_RENDERTARGET, DeviceCall, DeviceTraceEntry, Direct3D9, Direct3DDevice9, Dx9DeviceDesc,
    SurfaceDesc, SurfaceHandle, TextureHandle, failed,
};
use crate::gamebryo::object_net::NiObjectNet;
use crate::gamebryo::pixel_data::{PixelFormat, pixel_format_from_d3d};
use crate::gamebryo::renderer::{
    D3DMULTISAMPLE_NONE, RendererCreation, RendererGlobals, RendererState, pixel_format_to_d3d,
};
use crate::gamebryo::texture_loading::{WarningHook, page_padding};

/// IDA: 007C145F..007C14A2 the {2, 2, 2} prefs 007C1430 passes on.
const RENDER_TARGET_PREFS: [u32; 3] = [2, 2, 2];
/// IDA: 00B2C994 (.data 0x72 D3DFMT_R32F; read by 007C0D10 and 007C1360 only).
const SHADOW_MAP_FORMAT: i32 = 0x72;

const NI_DEPTH_STENCIL_BUFFER_VTABLE: u32 = 0x00A7_E44C;
const NI_DX9_TEXTURE_BUFFER_DATA_VTABLE: u32 = 0x00A8_98E4;
const NI_2D_BUFFER_VTABLE: u32 = 0x00A8_098C;
const NI_DX9_RENDERED_TEXTURE_DATA_VTABLE: u32 = 0x00A8_8464;
const NI_DX9_RENDERED_CUBE_MAP_DATA_VTABLE: u32 = 0x00A8_A868;
const NI_RENDERED_TEXTURE_VTABLE: u32 = 0x00A8_0204;
const NI_RENDERED_CUBE_MAP_VTABLE: u32 = 0x00AB_27FC;
const NI_RENDER_TARGET_GROUP_VTABLE: u32 = 0x00AB_286C;
const BS_RENDERED_TEXTURE_VTABLE: u32 = 0x00A9_1370;

/// IDA: NiDepthStencilBuffer (0x18 bytes, vtable A7E44C) with its renderer data. The default render-target group's buffer is
/// the device's implicit depth/stencil surface (`RendererState::depth_stencil`); 0070BC70 creates an additional one
/// (NiDX9AdditionalDepthStencilBufferData 0076DD40, 0x1C bytes) in the multisample branch of 007C1FF0.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct DepthStencilBuffer {
    pub original_vtable: u32,
    pub width_08: u32,
    pub height_0c: u32,
    /// 0070BD12.
    pub byte_size_14: u32,
    /// The renderer data class.
    pub additional: bool,
    /// Renderer data +0x10.
    pub pixel_format: PixelFormat,
    /// Renderer data +0x14.
    pub d3d_format: u32,
    /// Renderer data +0x18 (00B294EC).
    pub discard: bool,
    /// Renderer data +0x0C.
    pub surface: Option<SurfaceHandle>,
}

impl Default for DepthStencilBuffer {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            original_vtable: NI_DEPTH_STENCIL_BUFFER_VTABLE,
            width_08: 0,
            height_0c: 0,
            byte_size_14: 0,
            additional: false,
            pixel_format: PixelFormat::default(),
            d3d_format: 0,
            discard: false,
            surface: None,
        }
    }
}

/// IDA: NiDX9TextureBufferData (0x18, vtable A898E4; 0076D8C0 for a texture, 0076D9B0 for a cube face): +0x0C the level-0
/// surface (GetSurfaceLevel / GetCubeMapSurface), +0x10 a FormHeap NiPixelFormat of the surface format (0076C6D0), +0x14 the
/// D3D texture (AddRef). +0x08, the Ni2DBuffer, is the owner of this value. Completion: 100% of the fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureBufferData {
    pub original_vtable: u32,
    pub surface_0c: Option<SurfaceHandle>,
    pub pixel_format_10: PixelFormat,
    pub texture_14: Option<TextureHandle>,
}

/// IDA: Ni2DBuffer (0x14, vtable A8098C; 00732050): +0x08 width, +0x0C height, +0x10 NiPointer renderer data (0070BD60).
/// Completion: 100% of the fields.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct RenderBuffer2D {
    pub original_vtable: u32,
    pub width_08: u32,
    pub height_0c: u32,
    pub renderer_data_10: Option<TextureBufferData>,
}

/// IDA: NiDX9RenderedTextureData (0x64, vtable A88464; 00761920) and NiDX9RenderedCubeMapData (0x64, vtable A8A868; 007792E0):
/// +0x0C the 00773960 table NiPixelFormat (0x44 copy, 2D only), +0x50 the D3D texture, +0x54/+0x58 size, +0x5C levels (1),
/// +0x60 bytes. Usage and pool are the arguments of the creating call. Completion: 100% of the fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderedTextureRendererData {
    pub original_vtable: u32,
    pub cube: bool,
    pub pixel_format_0c: PixelFormat,
    pub texture_50: Option<TextureHandle>,
    pub width_54: u32,
    pub height_58: u32,
    pub levels_5c: u32,
    pub texture_bytes_60: u32,
    /// Raw D3DFORMAT.
    pub d3d_format: u32,
    pub usage: u32,
    pub pool: u32,
}

impl Default for RenderedTextureRendererData {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            original_vtable: NI_DX9_RENDERED_TEXTURE_DATA_VTABLE,
            cube: false,
            pixel_format_0c: PixelFormat::default(),
            texture_50: None,
            width_54: 0,
            height_58: 0,
            levels_5c: 1,
            texture_bytes_60: 0,
            d3d_format: 0,
            usage: 0,
            pool: 0,
        }
    }
}

/// IDA: NiRenderedTexture (0x40, vtable A80204; 0072A8B0) and NiRenderedCubeMap (0x5C, vtable AB27FC; 009A1BB0): NiTexture prefs
/// +0x18..+0x20 (6, 3, 2 until the creator copies its own), +0x24 renderer data, +0x30 NiPointer<Ni2DBuffer> (2D; null for the
/// cube), the cube's six buffers at +0x44..+0x58 (+0x40 = 0), +0x34 byte from 00B3FF00, +0x38 from 00B2752C, +0x3C byte from
/// 00B27530. Width and height (vt+0x4C/+0x50) read buffer 0 (the 2D getters return 0 without a buffer).
/// Completion: 100% of the fields.
/// TODO: [ ] Bind the 00701B00 NiTexture global list (as for NiSourceTexture).
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct RenderedTextureObject {
    pub original_vtable: u32,
    pub cube: bool,
    pub prefs_18: [u32; 3],
    pub renderer_data_24: Option<RenderedTextureRendererData>,
    /// A 2D texture's +0x30 is `[0]`; the cube's +0x44.. are all six.
    pub buffers: [Option<Entity>; 6],
    pub format_override_34: u8,
    pub override_format_38: i32,
    pub render_target_3c: u8,
}

impl Default for RenderedTextureObject {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            original_vtable: NI_RENDERED_TEXTURE_VTABLE,
            cube: false,
            prefs_18: [6, 3, 2],
            renderer_data_24: None,
            buffers: [None; 6],
            format_override_34: 0,
            override_format_38: 0,
            render_target_3c: 0,
        }
    }
}

/// IDA: NiRenderTargetGroup (0x24, vtable AB286C; 009A20A0): +0x08..+0x14 four buffers, +0x18 count, +0x1C depth/stencil
/// buffer, +0x20 renderer data (cleared by 009A1FD0/009A2000 through vt+0x7C). Completion: 100% of the fields.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderTargetGroup {
    pub original_vtable: u32,
    pub buffers_08: [Option<Entity>; 4],
    pub buffer_count_18: u32,
    pub depth_stencil_1c: Option<Entity>,
}

/// IDA: BSRenderedTexture (0x24, vtable A91370; 007D6BD0): +0x08..+0x1C six NiPointer<NiRenderTargetGroup> (one per cube face;
/// only [0] for 2D), +0x20 NiPointer<NiRenderedTexture>. Completion: 100% of the fields.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderedTexture {
    pub original_vtable: u32,
    pub groups_08: [Option<Entity>; 6],
    pub texture_20: Entity,
}

/// IDA: BSTextureManager::RenderedTextureData (0x14; 007C1D46): +0x00 NiPointer<BSRenderedTexture>, +0x04 format, +0x08 usage
/// (the sixth 007C1B50 argument), +0x0C flags, +0x10 byte in use. A record sits on exactly one list, so the list owns it by
/// value. Completion: 100% of the fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderedTextureData {
    pub texture_00: Option<Entity>,
    pub format_04: i32,
    pub usage_08: i32,
    pub flags_0c: u32,
    pub in_use_10: u8,
}

/// IDA: 007C1FF0 BSTextureManager constructor (0x48 bytes) stored in 00B42F50: two
/// NiTPointerList<BSTextureManager::RenderedTextureData*> (+0x00, +0x10, vtable A901A8), two
/// NiTPointerList<NiPointer<BSRenderedTexture>> (+0x20, +0x30), +0x40 NiPointer<NiDepthStencilBuffer> and +0x44 (released to
/// null). The lists are explicit `Vec`s, in list order. `ShaderManagerGlobals::texture_manager` holds it, as 00B42F50.
/// Completion: 100% of the cold state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextureManager {
    /// The free list.
    pub texture_data_00: Vec<RenderedTextureData>,
    /// The in-use list.
    pub texture_data_10: Vec<RenderedTextureData>,
    pub rendered_textures_20: Vec<Entity>,
    pub rendered_textures_30: Vec<Entity>,
    pub depth_stencil_40: Option<Entity>,
    pub pointer_44: u32,
}

/// IDA: the globals the chain reads or toggles, and the rendered-texture statistics. Completion: 100% of the sources.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct RenderTargetGlobals {
    /// 00B2752C (.data initial D3DFMT_R8G8B8).
    pub override_format_b2752c: i32,
    /// 00B27530 (.data initial 1).
    pub render_target_b27530: u8,
    pub texture_bytes_b42064: u32,
    pub texture_padding_b42068: u32,
    pub cube_bytes_b42860: u32,
    pub cube_padding_b42864: u32,
    /// NiDX9Renderer +0x230 (texture -> renderer data), in insertion order.
    pub renderer_map_230: Vec<Entity>,
    /// NiDX9Renderer +0x234, in insertion order.
    pub renderer_map_234: Vec<Entity>,
}

impl Default for RenderTargetGlobals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            override_format_b2752c: 0x14,
            render_target_b27530: 1,
            texture_bytes_b42064: 0,
            texture_padding_b42068: 0,
            cube_bytes_b42860: 0,
            cube_padding_b42864: 0,
            renderer_map_230: Vec::new(),
            renderer_map_234: Vec::new(),
        }
    }
}

/// IDA: NiDepthStencilBuffer memory statistics 00B3FAB8 (bytes) and 00B3FABC (page padding) that 0070BD24/0070BD43 add to.
/// Completion: 100% host counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderMemoryStatistics {
    pub buffer_bytes: u32,
    pub buffer_padding: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextureManagerStatus {
    #[default]
    Created,
    /// The multisample branch found no depth/stencil format (0070BE00 null); the original stores null in +0x40 and continues.
    NoAdditionalDepthStencil,
}

/// IDA: N/A the renderer objects 007C1FF0 reaches: the default render-target group (renderer +0x878) buffer 0 and its
/// depth/stencil buffer, the device (+0x280), the D3D9 interface (00B42154), the adapter and device type (+0x5BC/+0x5C0), the
/// device description (+0x5D0) and 00B34FC0. `calls` is where the device calls go, in original order.
pub struct TextureManagerServices<'a> {
    pub d3d9: Option<&'a mut (dyn Direct3D9 + 'static)>,
    pub device: Option<&'a mut (dyn Direct3DDevice9 + 'static)>,
    pub renderer: Option<&'a RendererState>,
    pub device_desc: Option<&'a Dx9DeviceDesc>,
    /// 00B34FC0 g_rendererMultiSample.
    pub multisample: i32,
    pub calls: &'a mut Vec<DeviceTraceEntry>,
}

impl<'a> TextureManagerServices<'a> {
    /// IDA: N/A with no device, as the C++ fixtures have it.
    #[must_use]
    pub const fn without_device(
        renderer: Option<&'a RendererState>,
        calls: &'a mut Vec<DeviceTraceEntry>,
    ) -> Self {
        Self {
            d3d9: None,
            device: None,
            renderer,
            device_desc: None,
            multisample: 0,
            calls,
        }
    }

    /// IDA: N/A the renderer's owner lends its parts. `device_desc` comes from `RendererCreation::system`, which the caller
    /// clones first, because that `Arc` outlives this borrow.
    #[must_use]
    pub fn from_creation(
        creation: &'a mut RendererCreation,
        device_desc: Option<&'a Dx9DeviceDesc>,
    ) -> Self {
        let multisample = creation.globals.multisample;
        Self {
            d3d9: creation.d3d9.as_deref_mut(),
            device: creation.device.as_deref_mut(),
            renderer: Some(&creation.state),
            device_desc,
            multisample,
            calls: &mut creation.device_calls,
        }
    }
}

/// IDA: N/A the renderer objects and the external globals of the chain. A null device builds every object, size and statistic
/// without device calls; a null renderer makes 0072A9B0/009A1CE0 fail. The C++ guards for null globals (`invalid_argument` in
/// 007C1430, 007C1B50, 007C0D10 and 007C2450) have no Rust form: these are references.
pub struct RenderTargetServices<'a> {
    pub texture: TextureManagerServices<'a>,
    /// HDR, FP16, anisotropic, non-pow2 and the formats.
    pub globals: &'a RendererGlobals,
    /// 00B42F3E.
    pub image_space_effects: u8,
    /// 00B45FC8: kinds 6, 9..12.
    pub water_resolution_b45fc8: i32,
    /// 00B2C2B8: kinds 15, 16, 19.
    pub blur_texture_size_b2c2b8: i32,
    /// Byte 00B3FF00, which 007DE240 (WaterShaderDisplacement) also clears.
    pub format_override_b3ff00: &'a mut u8,
    /// g_depthStencilDiscard.
    pub depth_discard_b294ec: &'a mut u32,
    /// 00B3FAB8/00B3FABC.
    pub buffer_statistics: &'a mut RenderMemoryStatistics,
    /// 00B42E8C g_shaderWarningCallback.
    pub warning: Option<WarningHook<'a>>,
}

// IDA: 0072A9D8..0072AA24, 009A1D05..009A1D27 the (x & (x - 1)) tests, with a zero side refused.
// Completion: 100%.
const fn power_of_two(value: u32) -> bool {
    value != 0 && value & (value - 1) == 0
}

// IDA: renderer vt+0x7C 00762590 (+0x878 the default render-target group), then group vt+0x4C (009A1F20) of buffer 0: the back
// buffer.
// Completion: 100%.
fn default_width(services: &RenderTargetServices<'_>) -> u32 {
    services
        .texture
        .renderer
        .map_or(0, |state| state.back_buffer_desc.width)
}

// IDA: as `default_width`: group vt+0x50 (009A1F40).
// Completion: 100%.
fn default_height(services: &RenderTargetServices<'_>) -> u32 {
    services
        .texture
        .renderer
        .map_or(0, |state| state.back_buffer_desc.height)
}

// IDA: the 007C25F3..007C2605 idiom: fild of the 32-bit value (exact), fadd 2^32 (flt_A2FC78) when it is negative, __ftol2_sse
// (truncation; a value of 2^31 or more gives 0x80000000).
// Completion: 100%.
const fn unsigned_to_int32(value: u32) -> i32 {
    let as_signed = value as i32;
    if as_signed >= 0 { as_signed } else { i32::MIN }
}

// IDA: 007C0D87..007C0D9F the same load, fmul 0.5 (dbl_A2FAA0) rounded to 24 bits under the startup control word, __ftol2_sse.
// Completion: 100%.
fn half_to_int32(value: u32) -> i32 {
    let as_signed = value as i32;
    let wrapped = f64::from(as_signed) + 4_294_967_296.0;
    let loaded = if as_signed >= 0 {
        f64::from(as_signed)
    } else {
        f64::from(x87::stored_float(wrapped))
    };
    x87::ftol(x87::stored_float(loaded * 0.5))
}

// IDA: 0076183E..00761868 bytes per texel by D3D format (others: 1).
// Completion: 100%.
const fn rendered_texture_multiplier(format: i32) -> u32 {
    match format {
        20 => 3,
        21 | 22 | 114 => 4,
        23..=26 | 81 => 2,
        36 | 113 => 8,
        116 => 16,
        _ => 1,
    }
}

// IDA: 007C1BC5..007C1C61 the g_rendererTextureFormatSupport index per D3D format; formats outside the switch take the default
// arm.
// Completion: 100%.
const fn texture_format_support_index(format: i32) -> Option<usize> {
    match format {
        20 => Some(5),
        21 => Some(6),
        22 => Some(7),
        23 => Some(0),
        24 => Some(1),
        25 => Some(2),
        26 => Some(3),
        36 => Some(10),
        50 => Some(12),
        81 => Some(4),
        113 => Some(9),
        114 => Some(8),
        116 => Some(11),
        _ => None,
    }
}

// IDA: 00732050(w, h) Ni2DBuffer: FormHeap(0x14) NiObject, vtable A8098C, +0x08/+0x0C size, +0x10 null renderer data.
// Completion: 100%.
const fn create_buffer(width: u32, height: u32) -> RenderBuffer2D {
    RenderBuffer2D {
        original_vtable: NI_2D_BUFFER_VTABLE,
        width_08: width,
        height_0c: height,
        renderer_data_10: None,
    }
}

// IDA: 0076D8C0(texture, &buffer) (face `None`) and 0076D9B0(cube, face, ..., &buffer): FormHeap(0x18) NiDX9TextureBufferData,
// +0x14 the texture with AddRef, GetSurfaceLevel(0) / GetCubeMapSurface(face, 0) into +0x0C (a failure logs the D3D error string
// and deletes the data), GetDesc, +0x10 = 0076C6D0(desc.Format), then 0070BD60 on the existing buffer. Without a device the host
// takes the requested format and size. The 007320D0 branch (no buffer) is not reached: every caller owns its buffer.
// Completion: 100% of the reached branches.
fn create_texture_buffer_data(
    texture: Option<TextureHandle>,
    face: Option<u32>,
    requested: SurfaceDesc,
    buffer: &mut RenderBuffer2D,
    services: &mut TextureManagerServices<'_>,
) -> bool {
    let mut surface = None;
    let mut description = requested;
    if let (Some(device), Some(handle)) = (services.device.as_deref_mut(), texture) {
        let (result, got) = match face {
            Some(face) => {
                let answer = device.cube_map_surface(handle, face, 0);
                services.calls.push(DeviceTraceEntry {
                    site: 0x0076_DA14,
                    call: DeviceCall::CubeGetSurface { face, level: 0 },
                    result: answer.0,
                });
                answer
            }
            None => {
                let answer = device.texture_surface_level(handle, 0);
                services.calls.push(DeviceTraceEntry {
                    site: 0x0076_D91D,
                    call: DeviceCall::TextureGetSurfaceLevel { level: 0 },
                    result: answer.0,
                });
                answer
            }
        };
        if failed(result) {
            return false;
        }
        let Some(got) = got else {
            return false;
        };
        let (result, desc) = device.surface_desc(got);
        services.calls.push(DeviceTraceEntry {
            site: if face.is_some() {
                0x0076_DA50
            } else {
                0x0076_D958
            },
            call: DeviceCall::SurfaceGetDesc,
            result,
        });
        if failed(result) {
            return false;
        }
        surface = Some(got);
        description = desc;
    }
    buffer.renderer_data_10 = Some(TextureBufferData {
        original_vtable: NI_DX9_TEXTURE_BUFFER_DATA_VTABLE,
        surface_0c: surface,
        pixel_format_10: pixel_format_from_d3d(description.format),
        texture_14: texture,
    });
    true
}

// IDA: 00768700 renderer vt+0x108 -> 00761920 (+0x24 null only): FormHeap(0x64) 007738E0 NiDX9TextureData (texture, renderer),
// vtable A88464, +0x60 = 0; 00761730: +0x54/+0x58 = the texture's size, +0x5C = 1, the 00773960 entry of +0x74C copied into +0x0C,
// format +0xC (or +0x38 under +0x34), CreateTexture(w, h, 1, +0x3C ? RENDERTARGET : 0, format, +0x3C ? DEFAULT : MANAGED); +0x50
// the texture, +0x60 bytes, 00B42064/00B42068; then the texture's +0x24, QueryInterface(IID_IDirect3DTexture9) and 0076D8C0 onto
// the texture's buffer. The caller does the +0x230 insert on success, once the texture has an entity.
// Completion: 100% of the reached branches.
fn create_rendered_texture_data(
    texture: &mut RenderedTextureObject,
    buffer: &mut RenderBuffer2D,
    globals: &mut RenderTargetGlobals,
    services: &mut RenderTargetServices<'_>,
) -> bool {
    if texture.renderer_data_24.is_some() {
        return true; // 00768706
    }
    let state = services.texture.renderer.expect("0072A9B0 checked it");
    let mut data = RenderedTextureRendererData {
        width_54: buffer.width_08,
        height_58: buffer.height_0c,
        ..RenderedTextureRendererData::default()
    };
    let Some(index) = select_render_target_format(&state.pixel_format_tables[1], texture.prefs_18)
    else {
        return false; // Host: 0076179C would copy from a null entry.
    };
    data.pixel_format_0c = state.pixel_format_tables[1][index].expect("found by the search");
    let mut format = data.pixel_format_0c.renderer_hint_0c as i32;
    let render_target = texture.render_target_3c != 0;
    data.usage = if render_target {
        D3DUSAGE_RENDERTARGET
    } else {
        0
    };
    data.pool = if render_target {
        D3DPOOL_DEFAULT
    } else {
        D3DPOOL_MANAGED
    };
    if texture.format_override_34 != 0 {
        format = texture.override_format_38;
    }
    data.d3d_format = format as u32;
    if let Some(device) = services.texture.device.as_deref_mut() {
        let (result, created) = device.create_texture(
            data.width_54,
            data.height_58,
            1,
            data.usage,
            data.d3d_format,
            data.pool,
        );
        services.texture.calls.push(DeviceTraceEntry {
            site: 0x0076_17F1,
            call: DeviceCall::CreateTexture {
                width: data.width_54,
                height: data.height_58,
                levels: 1,
                usage: data.usage,
                format: data.d3d_format,
                pool: data.pool,
            },
            result,
        });
        if failed(result) {
            return false; // 007617F8 the D3D error string, +0x50 = 0.
        }
        data.texture_50 = created;
    }
    data.texture_bytes_60 = data
        .width_54
        .wrapping_mul(data.height_58)
        .wrapping_mul(rendered_texture_multiplier(format));
    globals.texture_bytes_b42064 = globals
        .texture_bytes_b42064
        .wrapping_add(data.texture_bytes_60);
    globals.texture_padding_b42068 = globals
        .texture_padding_b42068
        .wrapping_add(page_padding(data.texture_bytes_60));
    // 0076197C the texture's +0x24 = data; 007619AF QueryInterface; 0076D8C0. 00761A1A a failure deletes the data.
    let requested = SurfaceDesc {
        format: data.d3d_format,
        width: data.width_54,
        height: data.height_58,
    };
    if !create_texture_buffer_data(
        data.texture_50,
        None,
        requested,
        buffer,
        &mut services.texture,
    ) {
        return false;
    }
    texture.renderer_data_24 = Some(data);
    true
}

// IDA: 00768740 renderer vt+0x110 -> 007792E0: FormHeap(0x64) (0x007616A0, vtable A8A868), +0x60 = 0; 00779210: +0x54 = +0x58 = the
// edge, +0x5C = 1, the 00773960 entry of +0x7FC (not copied), CreateCubeTexture(edge, 1, RENDERTARGET, +0x34 ? +0x38 : entry
// format, DEFAULT); +0x24 of the cube; six 0076D9B0 faces onto +0x44..; 6 * w * h * entry bpp/8 into 00B42860/00B42864 and +0x60.
// The caller does the +0x234 insert on success, once the cube has an entity.
// Completion: 100% of the reached branches.
fn create_rendered_cube_data(
    cube: &mut RenderedTextureObject,
    buffers: &mut [RenderBuffer2D],
    globals: &mut RenderTargetGlobals,
    services: &mut RenderTargetServices<'_>,
) -> bool {
    if cube.renderer_data_24.is_some() {
        return true; // 00768746
    }
    let state = services.texture.renderer.expect("009A1CE0 checked it");
    let mut data = RenderedTextureRendererData {
        original_vtable: NI_DX9_RENDERED_CUBE_MAP_DATA_VTABLE,
        cube: true,
        width_54: buffers[0].width_08,
        height_58: buffers[0].width_08,
        ..RenderedTextureRendererData::default()
    };
    let Some(index) = select_render_target_format(&state.pixel_format_tables[3], cube.prefs_18)
    else {
        return false; // Host: 0077926F would read a null entry.
    };
    let entry = state.pixel_format_tables[3][index].expect("found by the search");
    let mut format = entry.renderer_hint_0c as i32;
    if cube.format_override_34 != 0 {
        format = cube.override_format_38;
    }
    data.d3d_format = format as u32;
    data.usage = D3DUSAGE_RENDERTARGET;
    data.pool = D3DPOOL_DEFAULT;
    if let Some(device) = services.texture.device.as_deref_mut() {
        let (result, created) =
            device.create_cube_texture(data.width_54, 1, data.usage, data.d3d_format, data.pool);
        services.texture.calls.push(DeviceTraceEntry {
            site: 0x0077_92A2,
            call: DeviceCall::CreateCubeTexture {
                edge: data.width_54,
                levels: 1,
                usage: data.usage,
                format: data.d3d_format,
                pool: data.pool,
            },
            result,
        });
        if failed(result) {
            return false; // 007792A9 the D3D error string, +0x50 = 0.
        }
        data.texture_50 = created;
    }
    // 00779342 the cube's +0x24 = data; 007793F5 a failed face deletes the data.
    let requested = SurfaceDesc {
        format: data.d3d_format,
        width: data.width_54,
        height: data.height_58,
    };
    for (face, buffer) in buffers.iter_mut().enumerate() {
        if !create_texture_buffer_data(
            data.texture_50,
            Some(face as u32),
            requested,
            buffer,
            &mut services.texture,
        ) {
            return false;
        }
    }
    let bytes = 6u32
        .wrapping_mul(data.width_54)
        .wrapping_mul(data.height_58)
        .wrapping_mul(u32::from(entry.bits_per_pixel_01 >> 3));
    globals.cube_bytes_b42860 = globals.cube_bytes_b42860.wrapping_add(bytes);
    data.texture_bytes_60 = data.texture_bytes_60.wrapping_add(bytes);
    globals.cube_padding_b42864 = globals
        .cube_padding_b42864
        .wrapping_add(page_padding(bytes));
    cube.renderer_data_24 = Some(data);
    true
}

// IDA: 0070BE00 NiDepthStencilBuffer_CreateCompatible(source, renderer) for a source of `width` x `height` whose renderer-data pixel
// format maps to `front_format` (`None`: no renderer data). 0070BE10: a null renderer returns null. 0070BE12..0070BE30: renderer
// vt+0x98 00701610 -> vt+0x94 00764770 with the source's renderer-data pixel format (00780D10; null without renderer data), 32 depth
// and 8 stencil bits: 00775280 on the device description with the default group's buffer-0 format (group vt+0x80 009A2070 ->
// 00497DD0) and PixelFormatToD3DFormat(front). 0070BC70 (format 0x0F only), renderer vt+0x11C -> 0076DE00 CheckDeviceFormat
// (DEPTHSTENCIL, SURFACE) and CreateDepthStencilSurface(w, h, format, NONE, 0, discard 00B294EC), statistics 00B3FAB8/00B3FABC.
// Completion: 100%.
pub fn create_compatible_depth_stencil(
    world: &mut World,
    services: &mut TextureManagerServices<'_>,
    width: u32,
    height: u32,
    front_format: Option<u32>,
    discard: u32,
    statistics: &mut RenderMemoryStatistics,
) -> Option<Entity> {
    let state = services.renderer?;
    let front_format = front_format?;
    let device_desc = services.device_desc?;
    let back_buffer = state.back_buffer_desc.format;
    let depth_format = crate::gamebryo::dx9_system::dx9_find_depth_stencil_format(
        device_desc,
        back_buffer,
        front_format,
        32,
        8,
    );
    if depth_format == 0 {
        return None;
    }
    let format = pixel_format_from_d3d(depth_format);
    // 0070BC96: only a depth/stencil pixel format (format 0x0F) continues.
    if format.format_04 != 0x0F {
        return None;
    }
    let mut buffer = DepthStencilBuffer {
        width_08: width,
        height_0c: height,
        ..DepthStencilBuffer::default()
    };
    // 0076DE11..0076DE63 PixelFormatToD3DFormat, CheckDeviceFormat(adapter, type, buffer-0 format, D3DUSAGE_DEPTHSTENCIL,
    // D3DRTYPE_SURFACE, format).
    let d3d = pixel_format_to_d3d(&format);
    if d3d == 0 {
        return None;
    }
    let d3d9 = services.d3d9.as_deref_mut()?;
    let device = services.device.as_deref_mut()?;
    let result = d3d9.check_device_format(
        state.adapter_5bc,
        state.device_type_5c0,
        back_buffer,
        D3DUSAGE_DEPTHSTENCIL,
        D3DRTYPE_SURFACE,
        d3d,
    );
    services.calls.push(DeviceTraceEntry {
        site: 0x0076_DE63,
        call: DeviceCall::CheckDeviceFormat {
            adapter: state.adapter_5bc,
            device_type: state.device_type_5c0,
            adapter_format: back_buffer,
            usage: D3DUSAGE_DEPTHSTENCIL,
            resource_type: D3DRTYPE_SURFACE,
            format: d3d,
        },
        result,
    });
    if failed(result) {
        return None;
    }
    // 0076DEA1..0076DED6 CreateDepthStencilSurface(w, h, format, NONE, 0, discard 00B294EC, &surface, NULL).
    buffer.additional = true;
    buffer.discard = discard != 0;
    let (result, surface) = device.create_depth_stencil_surface(
        width,
        height,
        d3d,
        D3DMULTISAMPLE_NONE,
        0,
        buffer.discard,
    );
    services.calls.push(DeviceTraceEntry {
        site: 0x0076_DED6,
        call: DeviceCall::CreateDepthStencilSurface {
            width,
            height,
            format: d3d,
            multi_sample: D3DMULTISAMPLE_NONE,
            multi_sample_quality: 0,
            discard: buffer.discard,
        },
        result,
    });
    if failed(result) {
        return None;
    }
    let surface = surface?;
    buffer.surface = Some(surface);
    // 0076DF04..0076DF2B GetDesc, then the renderer data pixel format.
    let (result, description) = device.surface_desc(surface);
    services.calls.push(DeviceTraceEntry {
        site: 0x0076_DF11,
        call: DeviceCall::SurfaceGetDesc,
        result,
    });
    if failed(result) {
        return None;
    }
    buffer.pixel_format = pixel_format_from_d3d(description.format);
    buffer.d3d_format = d3d;
    // 0070BD12..0070BD43: size = bits/8 * height * width; statistics.
    buffer.byte_size_14 = u32::from(format.bits_per_pixel_01 >> 3)
        .wrapping_mul(height)
        .wrapping_mul(width);
    statistics.buffer_bytes = statistics.buffer_bytes.wrapping_add(buffer.byte_size_14);
    statistics.buffer_padding = statistics
        .buffer_padding
        .wrapping_add(page_padding(buffer.byte_size_14));
    Some(world.spawn(buffer).id())
}

// IDA: 007C1FF0. With g_rendererMultiSample < 2 +0x40 holds the default group's depth/stencil buffer (renderer vt+0x7C -> group
// vt+0x74); otherwise 00B294EC = 0, 0070BE00(group vt+0x70(0), renderer) creates a compatible buffer, then 00B294EC = 1. +0x44 is
// released and cleared (already null). A null renderer is the C++ port's `invalid_argument`.
// Completion: 100% of the order.
// TODO: [ ] Compare the multisample branch on a multisample configuration.
pub fn construct_texture_manager(
    world: &mut World,
    services: &mut TextureManagerServices<'_>,
    discard_flag: &mut u32,
    statistics: &mut RenderMemoryStatistics,
) -> Result<(TextureManager, TextureManagerStatus), StreamError> {
    let Some(state) = services.renderer else {
        return Err(StreamError("007C1FF0: renderer state required"));
    };
    let mut manager = TextureManager::default();
    let mut status = TextureManagerStatus::Created;
    // 007C2076: cmp g_rendererMultiSample, 2; jl.
    if services.multisample < 2 {
        // 007C20B2..007C20BC: the default group's depth/stencil buffer (renderer vt+0x7C 00762590, group vt+0x74 004D9B40).
        let buffer = DepthStencilBuffer {
            width_08: state.depth_stencil_desc.width,
            height_0c: state.depth_stencil_desc.height,
            d3d_format: state.depth_stencil_desc.format,
            pixel_format: pixel_format_from_d3d(state.depth_stencil_desc.format),
            surface: state.depth_stencil,
            ..DepthStencilBuffer::default()
        };
        manager.depth_stencil_40 = Some(world.spawn(buffer).id());
    } else {
        // 007C2086 00B294EC = 0, 0070BE00, 007C20A6 00B294EC = 1. 007C2098 0070BE00(group vt+0x70(0) 009A2060: the back buffer,
        // renderer).
        *discard_flag = 0;
        manager.depth_stencil_40 = create_compatible_depth_stencil(
            world,
            services,
            state.back_buffer_desc.width,
            state.back_buffer_desc.height,
            Some(state.back_buffer_desc.format),
            *discard_flag,
            statistics,
        );
        if manager.depth_stencil_40.is_none() {
            status = TextureManagerStatus::NoAdditionalDepthStencil;
        }
        *discard_flag = 1;
    }
    // 007C20C1..007C20E4: +0x44 released and cleared.
    manager.pointer_44 = 0;
    Ok((manager, status))
}

// IDA: 00773960(prefs, table) the render-target NiPixelFormat search over a 22-entry table (renderer +0x74C for textures, +0x7FC for
// cube maps): layout 6 -> 2, alpha 3 -> 0, the layout/alpha cases and their fallback chains. Returns the table index.
// Completion: 100%.
#[must_use]
pub fn select_render_target_format(
    table: &[Option<PixelFormat>; 22],
    prefs: [u32; 3],
) -> Option<usize> {
    // 00773964..00773979.
    let layout = if prefs[0] as i32 == 6 {
        2
    } else {
        prefs[0] as i32
    };
    let alpha = if prefs[1] as i32 == 3 {
        0
    } else {
        prefs[1] as i32
    };
    let mut order: Vec<usize> = Vec::with_capacity(12);
    // 0077397C..007739AD: layout 0/5 tries +0x18 (alpha 0) or +0x1C (1, 2).
    let plain = layout == 0 || layout == 5;
    if plain && alpha == 0 {
        order.push(6);
    } else if plain && (alpha == 1 || alpha == 2) {
        order.push(7);
    }
    // 007739EA LABEL_22 and 00773A45 LABEL_33 tails by alpha.
    let tail_22: &[usize] = match alpha {
        0 => &[1, 0, 2, 5, 3, 4],
        1 => &[3, 4, 5],
        2 => &[4, 5],
        _ => &[],
    };
    let tail_33: &[usize] = match alpha {
        0 => &[2, 5, 1, 0, 3, 4],
        1 => &[5, 3, 4],
        2 => &[5, 4],
        _ => &[],
    };
    match layout {
        0 | 3 | 5 => {
            if alpha == 0 || alpha == 1 {
                order.push(8);
            } else if alpha == 2 {
                order.push(9);
            }
            order.extend(tail_22);
        }
        1 => order.extend(tail_22),
        2 => order.extend(tail_33),
        4 => {
            order.push(13);
            if alpha != 0 {
                order.extend([12, 11]);
            } else {
                order.extend([11, 12]);
            }
            order.extend(tail_33);
        }
        7 => {
            if alpha != 0 {
                order.extend([15, 3, 4, 5]);
            } else {
                order.extend([14, 1, 0, 2, 5, 3, 4]);
            }
        }
        8 => order.extend([16, 19, 17, 20, 18, 21]),
        9 => order.extend([19, 20, 21, 16, 17, 18]),
        10 => order.extend([17, 20, 18, 21]),
        11 => order.extend([20, 21, 17, 18]),
        12 => order.extend([18, 21]),
        13 => order.extend([21, 18]),
        _ => {}
    }
    order.into_iter().find(|&index| table[index].is_some())
}

// IDA: 0072A9B0(w, h, renderer, prefs) NiRenderedTexture::Create. 0072A9D8..0072AA24: a null renderer, or a zero or non-power-of-two
// size without renderer flags 8/4 (00769F20), fails. 0072AA2C..0072AA90 FormHeap(0x40) 0072A8B0, the prefs, +0x30 = 00732050
// Ni2DBuffer(w, h), +0x34/+0x38/+0x3C from 00B3FF00/00B2752C/00B27530. 0072AA9F renderer vt+0x108; 0072AAAD a failure deletes the
// texture.
// Completion: 100%.
pub fn create_ni_rendered_texture(
    world: &mut World,
    width: u32,
    height: u32,
    prefs: [u32; 3],
    globals: &mut RenderTargetGlobals,
    services: &mut RenderTargetServices<'_>,
) -> Option<Entity> {
    let state = services.texture.renderer?;
    let flags = state.texture_caps_flags_5e0;
    if (!power_of_two(width) || !power_of_two(height)) && flags & 8 == 0 && flags & 4 == 0 {
        return None;
    }
    let mut texture = RenderedTextureObject {
        prefs_18: prefs,
        format_override_34: *services.format_override_b3ff00,
        override_format_38: globals.override_format_b2752c,
        render_target_3c: globals.render_target_b27530,
        ..RenderedTextureObject::default()
    };
    let mut buffer = create_buffer(width, height);
    if !create_rendered_texture_data(&mut texture, &mut buffer, globals, services) {
        return None;
    }
    texture.buffers[0] = Some(world.spawn(buffer).id());
    let entity = world.spawn((NiObjectNet::default(), texture)).id();
    globals.renderer_map_230.push(entity); // 0076872B
    Some(entity)
}

// IDA: 009A1CE0(edge, renderer, prefs) NiRenderedCubeMap::Create. 009A1D05..009A1D27: a null renderer or a zero or non-power-of-two
// edge fails. 009A1D2F..009A1DE1 FormHeap(0x5C) 009A1BB0, the prefs, six 00732050 buffers (edge x edge) at +0x44, the three bytes.
// 009A1DED renderer vt+0x110; 009A1DFC a failure deletes the cube.
// Completion: 100%; not reached at startup (no 007C2450 flags carry 0x10).
pub fn create_ni_rendered_cube_map(
    world: &mut World,
    edge: u32,
    prefs: [u32; 3],
    globals: &mut RenderTargetGlobals,
    services: &mut RenderTargetServices<'_>,
) -> Option<Entity> {
    services.texture.renderer?;
    if !power_of_two(edge) {
        return None;
    }
    let mut cube = RenderedTextureObject {
        original_vtable: NI_RENDERED_CUBE_MAP_VTABLE,
        cube: true,
        prefs_18: prefs,
        format_override_34: *services.format_override_b3ff00,
        override_format_38: globals.override_format_b2752c,
        render_target_3c: globals.render_target_b27530,
        ..RenderedTextureObject::default()
    };
    let mut buffers = vec![create_buffer(edge, edge); 6];
    if !create_rendered_cube_data(&mut cube, &mut buffers, globals, services) {
        return None;
    }
    for (slot, buffer) in cube.buffers.iter_mut().zip(buffers) {
        *slot = Some(world.spawn(buffer).id());
    }
    let entity = world.spawn((NiObjectNet::default(), cube)).id();
    globals.renderer_map_234.push(entity); // 0076876B
    Some(entity)
}

// IDA: 009A21D0(buffer, renderer, depth) NiRenderTargetGroup_CreateForBuffer. 009A21F2..009A2223: null renderer or buffer, or vt+0x9C
// (max buffers per group, +0x894) below 1, fail. 009A2225..009A2277 FormHeap(0x24) 009A20A0, +0x18 = 1, vt+0x68 AttachBuffer(buffer, 0)
// (009A1FD0: index 0 < count), then vt+0x6C AttachDepthStencilBuffer(depth) (009A2000) when non-null.
// Completion: 100%.
pub fn create_render_target_group(
    world: &mut World,
    buffer: Option<Entity>,
    depth: Option<Entity>,
    services: &RenderTargetServices<'_>,
) -> Option<Entity> {
    let state = services.texture.renderer?;
    let buffer = buffer?;
    if state.max_render_targets_894 < 1 {
        return None;
    }
    let mut buffers = [None; 4];
    buffers[0] = Some(buffer);
    let group = RenderTargetGroup {
        original_vtable: NI_RENDER_TARGET_GROUP_VTABLE,
        buffers_08: buffers,
        buffer_count_18: 1,
        depth_stencil_1c: depth,
    };
    Some(world.spawn(group).id())
}

// IDA: 0072A990 and 0072A9A0: a buffer's size, 0 without one.
// Completion: 100%.
fn buffer_size(world: &World, buffer: Option<Entity>) -> (u32, u32) {
    buffer
        .and_then(|entity| world.get::<RenderBuffer2D>(entity))
        .map_or((0, 0), |buffer| (buffer.width_08, buffer.height_0c))
}

// IDA: 0072A990 (2D: the +0x30 buffer, 0 without one) / 009A1B90 (cube: +0x44), and 0072A9A0 / 009A1BA0 for the height.
// Completion: 100%.
#[must_use]
pub fn rendered_texture_object_size(world: &World, object: Option<Entity>) -> (u32, u32) {
    object
        .and_then(|entity| world.get::<RenderedTextureObject>(entity))
        .map_or((0, 0), |object| buffer_size(world, object.buffers[0]))
}

// IDA: 007D6BD0(texture, want_depth, depth) BSRenderedTexture constructor: 007D6C0A..007D6C77 vtable A91370, six null groups, +0x20 =
// texture; with want_depth (007D6C93 the NiRenderedCubeMap RTTI walk, BAA880) the given buffer when it covers the texture (cube:
// buffer 0), else 0070BE00 on the texture's buffer, whose front format is its renderer data's pixel format (none without renderer
// data); then 007D6D44..007D6DA2 one 009A21D0 group (groups 1..5 released) or 007D6DA8..007D6E1C six (cube faces).
// Completion: 100%.
pub fn construct_bs_rendered_texture(
    world: &mut World,
    texture: Entity,
    want_depth: bool,
    depth: Option<Entity>,
    services: &mut RenderTargetServices<'_>,
) -> Entity {
    let object = world
        .get::<RenderedTextureObject>(texture)
        .expect("a NiRenderedTexture");
    let cube = object.cube;
    let buffers = object.buffers;
    let (width, height) = buffer_size(world, buffers[0]);
    let mut used = None;
    if want_depth {
        let covering = depth.filter(|&entity| {
            world
                .get::<DepthStencilBuffer>(entity)
                .is_some_and(|buffer| buffer.width_08 >= width && buffer.height_0c >= height)
        });
        used = match covering {
            Some(covering) => Some(covering),
            None => {
                // 0070BE00; 0070BE10 returns null without a source.
                let source = buffers[0].and_then(|entity| world.get::<RenderBuffer2D>(entity));
                match source {
                    Some(source) => {
                        let front = source
                            .renderer_data_10
                            .as_ref()
                            .map(|data| pixel_format_to_d3d(&data.pixel_format_10));
                        create_compatible_depth_stencil(
                            world,
                            &mut services.texture,
                            width,
                            height,
                            front,
                            *services.depth_discard_b294ec,
                            services.buffer_statistics,
                        )
                    }
                    None => None,
                }
            }
        };
    }
    let mut groups = [None; 6];
    if cube {
        for (slot, buffer) in groups.iter_mut().zip(buffers) {
            *slot = create_render_target_group(world, buffer, used, services);
        }
    } else {
        groups[0] = create_render_target_group(world, buffers[0], used, services);
    }
    world
        .spawn(RenderedTexture {
            original_vtable: BS_RENDERED_TEXTURE_VTABLE,
            groups_08: groups,
            texture_20: texture,
        })
        .id()
}

// IDA: 007C1430(renderer, w, h, format, usage, flags): 007C1457..007C1496 power-of-two square rounding of max(w, h) unless
// g_rendererNonPow2Textures; 007C14A6..007C14E2 with a format, 00B3FF00 = 1 and 00B2752C = format; flags 8 clears 00B27530, 0x80
// clears 00B294EC, 0x100 drops the +0x40 depth buffer; 0x10 -> 007C14FC..007C1542 009A1CE0 + FormHeap(0x24) 007D6BD0(cube, 1, depth),
// else 007C155B 007D6F40(w, h, {2, 2, 2}, !(flags & 4), depth): 0072A9B0 then 007D6BD0; 007C156A..007C1585 the globals are restored as
// constants (0, 1, 1) and 007C158C..007C15A2 a null result calls 00B42E8C("Unable to create rendered texture", NULL). The usage
// argument is not read.
// Completion: 100%.
#[allow(clippy::too_many_arguments)] // 007C1430(renderer, w, h, format, usage, flags).
pub fn create_rendered_texture(
    world: &mut World,
    manager: &TextureManager,
    width: u32,
    height: u32,
    format: i32,
    _usage: i32,
    flags: u32,
    globals: &mut RenderTargetGlobals,
    services: &mut RenderTargetServices<'_>,
) -> Option<Entity> {
    let mut texture_width = width;
    let mut texture_height = height;
    if !services.globals.non_pow2_textures {
        let largest = width.max(height) as i32;
        let mut size = 1i32;
        while size < largest {
            size = size.wrapping_mul(2);
        }
        texture_width = size as u32;
        texture_height = size as u32;
    }
    if format != 0 {
        *services.format_override_b3ff00 = 1;
        globals.override_format_b2752c = format;
    }
    if flags & 8 != 0 {
        globals.render_target_b27530 = 0;
    }
    if flags & 0x80 != 0 {
        *services.depth_discard_b294ec = 0;
    }
    let depth = if flags & 0x100 == 0 {
        manager.depth_stencil_40
    } else {
        None
    };
    let result = if flags & 0x10 != 0 {
        create_ni_rendered_cube_map(world, texture_width, RENDER_TARGET_PREFS, globals, services)
            .map(|cube| construct_bs_rendered_texture(world, cube, true, depth, services))
    } else {
        create_ni_rendered_texture(
            world,
            texture_width,
            texture_height,
            RENDER_TARGET_PREFS,
            globals,
            services,
        )
        .map(|texture| {
            construct_bs_rendered_texture(world, texture, flags & 4 == 0, depth, services)
        })
    };
    if format != 0 {
        *services.format_override_b3ff00 = 0;
    }
    if flags & 8 != 0 {
        globals.render_target_b27530 = 1;
    }
    if flags & 0x80 != 0 {
        *services.depth_discard_b294ec = 1;
    }
    if result.is_none()
        && let Some(warning) = services.warning.as_mut()
    {
        warning(b"Unable to create rendered texture");
    }
    result
}

// IDA: 007C1B50(renderer, w, h, flags, format, usage). 007C1B5D..007C1B9D the default group's buffer-0 size. 007C1BA0..007C1C70: format
// 36 falls back to 113 when 00B42E98[10] is clear (flags 8 skips the check). 007C1C74..007C1D3C the first free-list (+0x00) record whose
// texture size matches (0 = the default group's size), format, usage and flags | 0x22 is reused: 007C1DC9 007AA860 removes the node and
// 007C1DE0 +0x10 = 1. Otherwise 007C1D46..007C1DBE a new 0x14 record from 007C1430 (a non-positive size takes the default group's).
// 007C1DEB..007C1E23 the record is appended to the +0x10 list and its texture returned (null when the creation failed).
// Completion: 100%.
#[allow(clippy::too_many_arguments)] // 007C1B50(renderer, w, h, flags, format, usage).
pub fn get_rendered_texture(
    world: &mut World,
    manager: &mut TextureManager,
    width: i32,
    height: i32,
    flags: u32,
    format: i32,
    usage: i32,
    globals: &mut RenderTargetGlobals,
    services: &mut RenderTargetServices<'_>,
) -> Option<Entity> {
    let group_width = default_width(services) as i32;
    let group_height = default_height(services) as i32;
    let mut format = format;
    if format != 0 && flags & 8 == 0 {
        let supported = texture_format_support_index(format)
            .is_some_and(|index| services.globals.texture_formats[index]);
        if !supported && format == 36 {
            format = 113;
        }
    }
    let matched = manager.texture_data_00.iter().position(|record| {
        let object = record
            .texture_00
            .and_then(|entity| world.get::<RenderedTexture>(entity))
            .map(|rendered| rendered.texture_20);
        let (texture_width, texture_height) = rendered_texture_object_size(world, object);
        let texture_width = texture_width as i32;
        let texture_height = texture_height as i32;
        (texture_width == width || (width == 0 && texture_width == group_width))
            && (texture_height == height || (height == 0 && texture_height == group_height))
            && record.format_04 == format
            && record.usage_08 == usage
            && (record.flags_0c | 0x22) == (flags | 0x22)
    });
    let record = match matched {
        Some(index) => {
            let mut record = manager.texture_data_00.remove(index);
            record.in_use_10 = 1;
            record
        }
        None => {
            let (create_width, create_height) = if width <= 0 || height <= 0 {
                (group_width as u32, group_height as u32)
            } else {
                (width as u32, height as u32)
            };
            let texture = create_rendered_texture(
                world,
                manager,
                create_width,
                create_height,
                format,
                usage,
                flags,
                globals,
                services,
            );
            RenderedTextureData {
                texture_00: texture,
                format_04: format,
                usage_08: usage,
                flags_0c: flags,
                in_use_10: 1,
            }
        }
    };
    manager.texture_data_10.push(record);
    record.texture_00
}

/// IDA: 007C0D10's outputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderTargetRequest {
    pub width: i32,
    pub height: i32,
    pub format: i32,
    pub usage: i32,
    pub flags: u32,
}

// IDA: 007C0D10(renderer, kind, ...) the 25-kind size/format/flags table (usage always 0; kind 23 reads the read-only .data 00B2C994 =
// 0x72 and the word 00B2C67C). Kind 5 stores the height twice and leaves the width output, kinds above 24 leave every output but the
// usage: 007C23C0's locals are uninitialized there, the host passes zero (recorded substitution; neither is reached at startup).
// Completion: 100%.
#[must_use]
pub fn render_target_kind(kind: u32, services: &RenderTargetServices<'_>) -> RenderTargetRequest {
    let renderer = services.globals;
    let screen_width = default_width(services);
    let screen_height = default_height(services);
    let water = services.water_resolution_b45fc8;
    let blur = services.blur_texture_size_b2c2b8;
    let image_space = u32::from(services.image_space_effects != 0);
    let mut out = RenderTargetRequest::default();
    let square = |out: &mut RenderTargetRequest, size: i32, flags: u32, format: i32| {
        out.width = size;
        out.height = size;
        out.flags = flags;
        out.format = format;
    };
    match kind {
        0 | 2 => square(&mut out, 256, 38, 113),
        1 => {
            // 007C0D67..007C0DFF: width 256, height max(256, ftol(h * 0.5)). The >= 256 arm reads the height again (same value).
            out.flags = 38;
            out.width = 256;
            out.height = if half_to_int32(screen_height) >= 256 {
                half_to_int32(screen_height)
            } else {
                256
            };
            out.format = 113;
        }
        3 => {
            out.width = screen_width as i32;
            out.height = screen_height as i32;
            out.flags = 38;
            out.format = 0;
        }
        4 => {
            out.width = screen_width as i32;
            out.height = screen_height as i32;
            out.flags = if renderer.multisample < 2 { 34 } else { 162 };
            out.format = if renderer.hdr { 113 } else { 0 };
        }
        5 => {
            // 007C0EF2..007C0F75: the height output receives both conversions.
            out.height = unsigned_to_int32(screen_width);
            out.height = unsigned_to_int32(screen_height);
            out.flags = 4 * image_space + 34;
            out.format = 0;
        }
        6 => square(&mut out, water, 6, 116),
        7 => square(&mut out, 256, 6, 36),
        8 => square(&mut out, 128, 6, 36),
        9 => {
            out.width = water;
            out.height = 1;
            out.flags = 12;
            out.format = 81;
        }
        10 => {
            out.width = water;
            out.height = 16;
            out.flags = 12;
            out.format = 116;
        }
        11 => square(&mut out, water, 12, 114),
        12 => square(&mut out, water, 12, 116),
        13 => square(&mut out, 256, 2, 21),
        14 => square(&mut out, 256, 6, 0),
        15 | 16 | 19 => square(&mut out, blur, 6, 0),
        17 | 18 => square(&mut out, 128, 6, 0),
        20 => {
            out.width = screen_width as i32;
            out.height = screen_height as i32;
            out.flags = 70;
            out.format = 0;
        }
        21 => square(&mut out, 256, 2, 0),
        22 => square(&mut out, 512, 6, 114),
        23 => square(
            &mut out,
            i32::from(renderer.shadow_map_resolution),
            6,
            SHADOW_MAP_FORMAT,
        ),
        24 => square(&mut out, 256, 16, 0),
        _ => {}
    }
    out
}

// IDA: 007C23C0(renderer, kind): 007C23EC 007C0D10, 007C2412 007C1B50(w, h, flags, format, usage).
// Completion: 100%.
pub fn get_rendered_texture_by_kind(
    world: &mut World,
    manager: &mut TextureManager,
    kind: u32,
    globals: &mut RenderTargetGlobals,
    services: &mut RenderTargetServices<'_>,
) -> Option<Entity> {
    let request = render_target_kind(kind, services);
    get_rendered_texture(
        world,
        manager,
        request.width,
        request.height,
        request.flags,
        request.format,
        request.usage,
        globals,
        services,
    )
}

// IDA: 007C1EE0(texture): 007C1EEB a null texture returns; 007C1EED..007C1F1F the first +0x10 record holding it; 007C1F2B 005B1E20
// AddTail on +0x00; 007C1F3F 007AA860 removes the node; the in-use byte stays set.
// Completion: 100%.
pub fn release_rendered_texture(manager: &mut TextureManager, texture: Option<Entity>) {
    let Some(texture) = texture else {
        return;
    };
    let found = manager
        .texture_data_10
        .iter()
        .position(|record| record.texture_00 == Some(texture));
    if let Some(found) = found {
        let record = manager.texture_data_10.remove(found);
        manager.texture_data_00.push(record);
    }
}

// IDA: 007C2450. 007C2457..007C255D with g_rendererHDR 1x1 (x3, flags 0x24), 4x4, 16x16, 64x64 (0x26), all D3DFMT 113, then kinds 2, 0,
// 4 and kind 1 under FP16 filtering and anisotropic min filter (else null; all null without HDR, 007C2563..007C2587); 007C258B..007C25D6
// the image-space screen target (0x26, format 0) under 00B42F3E; 007C25DA..007C26D4 two screen-size targets (flags 4 * 00B42F3E + 0x22,
// format 0; sizes through the unsigned-to-float ftol idiom); 007C26D8..007C26F6 007C1EE0 for each non-null of the 13 in order. Returns the
// 13 locals.
// Completion: 100%.
pub fn create_render_targets(
    world: &mut World,
    manager: &mut TextureManager,
    globals: &mut RenderTargetGlobals,
    services: &mut RenderTargetServices<'_>,
) -> [Option<Entity>; 13] {
    let mut targets = [None; 13];
    let hdr = services.globals.hdr;
    let filtered = services.globals.fp16_filtering && services.globals.anisotropic_min_filter;
    let image_space = services.image_space_effects != 0;
    if hdr {
        const SIZES: [(i32, u32); 6] = [
            (1, 0x24),
            (1, 0x24),
            (1, 0x24),
            (4, 0x26),
            (16, 0x26),
            (64, 0x26),
        ];
        for (slot, (size, flags)) in targets.iter_mut().zip(SIZES) {
            *slot =
                get_rendered_texture(world, manager, size, size, flags, 113, 0, globals, services);
        }
        targets[6] = get_rendered_texture_by_kind(world, manager, 2, globals, services);
        targets[7] = get_rendered_texture_by_kind(world, manager, 0, globals, services);
        targets[8] = get_rendered_texture_by_kind(world, manager, 4, globals, services);
        if filtered {
            targets[9] = get_rendered_texture_by_kind(world, manager, 1, globals, services);
        }
    }
    if image_space {
        let width = default_width(services) as i32;
        let height = default_height(services) as i32;
        targets[10] =
            get_rendered_texture(world, manager, width, height, 0x26, 0, 0, globals, services);
    }
    let screen_flags = 4 * u32::from(image_space) + 0x22;
    for slot in &mut targets[11..13] {
        let width = unsigned_to_int32(default_width(services));
        let height = unsigned_to_int32(default_height(services));
        *slot = get_rendered_texture(
            world,
            manager,
            width,
            height,
            screen_flags,
            0,
            0,
            globals,
            services,
        );
    }
    for target in targets {
        if target.is_some() {
            release_rendered_texture(manager, target);
        }
    }
    targets
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;

    use recomp_core::fixtures;

    use super::*;
    use crate::gamebryo::device_caps::{DeviceCapsFixture, PinnedRendererPlatform};
    use crate::gamebryo::dx9_system::{D3DFMT_R5G6B5, D3DFMT_X1R5G5B5, D3DFMT_X8R8G8B8};
    use crate::gamebryo::renderer::{
        RendererCreationInputs, RendererSettings, initialize_renderer, renderer_settings_from,
    };
    use crate::tes::settings::{SettingCollections, register_static_settings};

    const D3DFMT_A8R8G8B8: u32 = 21;
    const D3DFMT_D24S8: u32 = 75;

    /// This block's rows of `fixtures/rows/phase05.rows`, as `[kind, ..., needle]`. The file has other blocks, whose formats are
    /// theirs; only `TARGETS|` lines are read here.
    pub(crate) fn rows(kind: &str) -> Vec<Vec<String>> {
        fixtures::rows("phase05")
            .into_iter()
            .filter_map(|row| row.strip_prefix("TARGETS|").map(str::to_owned))
            .map(|row| row.split('|').map(str::to_owned).collect::<Vec<_>>())
            .filter(|row| row[1] == kind)
            .collect()
    }

    pub(crate) fn evidence(run: &str) -> String {
        let file = match run {
            "P05-RUN-001" => "phase05-run001-2026-10-06.records.txt",
            "P05-RUN-002" => "phase05-run002-2026-10-06.records.txt",
            other => panic!("unknown run {other}"),
        };
        let path = fixtures::root().join("evidence").join(file);
        let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        bytes.iter().map(|&byte| char::from(byte)).collect()
    }

    fn number(text: &str) -> u32 {
        text.parse()
            .unwrap_or_else(|_| panic!("a decimal number: {text}"))
    }

    fn hex(text: &str) -> u32 {
        u32::from_str_radix(text, 16).unwrap_or_else(|_| panic!("a hex number: {text}"))
    }

    /// IDA: N/A every row's last column is a line of its run's evidence, and every value column is that line's text.
    #[test]
    fn phase05_target_rows_match_their_evidence() {
        let lines = fixtures::rows("phase05");
        let mine: Vec<&String> = lines
            .iter()
            .filter(|row| row.starts_with("TARGETS|"))
            .collect();
        assert!(mine.len() >= 60, "only {} rows", mine.len());
        for row in mine {
            let columns: Vec<&str> = row.split('|').collect();
            let (run, kind, needle) = (columns[1], columns[2], columns[columns.len() - 1]);
            assert!(evidence(run).contains(needle), "{row} is not in {run}");
            let wanted: Vec<String> = match kind {
                "GET" => vec![
                    format!("n={} ", columns[3]),
                    format!(
                        "w={} h={} flags={} fmt={} usage=0",
                        columns[4], columns[5], columns[6], columns[7]
                    ),
                ],
                "REL" => vec![
                    format!("n={} ", columns[3]),
                    format!("b34={} f38={} b3C={} ", columns[4], columns[5], columns[6]),
                    format!("w={} h={} rd=", columns[7], columns[8]),
                    format!(
                        "rdw={} rdh={} lv=1 bytes={} pfmt={} ",
                        columns[7], columns[8], columns[9], columns[10]
                    ),
                    format!("g0count={} ", columns[11]),
                    if columns[12] == "1" {
                        "g0depth=".to_owned()
                    } else {
                        "g0depth=0".to_owned()
                    },
                ],
                "FREE" => vec![
                    format!("free[{}] ", columns[3]),
                    format!("fmt={} usage=0 flags={} inuse=01", columns[4], columns[5]),
                ],
                "GLOBAL" => vec![format!("{}={}", columns[3], columns[4])],
                "DEPTH" => vec![
                    format!("vt={} ", columns[3]),
                    format!(
                        "{:08x}{:08x}",
                        hex(columns[4]).swap_bytes(),
                        hex(columns[5]).swap_bytes()
                    ),
                ],
                "CUBE" => vec![
                    format!("PIXELDATA {} pd=", columns[3]),
                    format!(
                        "levels={} faces={} bytesPerFace={} w0={} fmt={} faces={}",
                        columns[4], columns[5], columns[6], columns[7], columns[8], columns[9]
                    ),
                ],
                "TEX" => vec![
                    format!("tex@{}=", columns[3]),
                    format!(
                        "vt={} w={} h={} lv={} bytes={}",
                        columns[4], columns[5], columns[6], columns[7], columns[8]
                    ),
                ],
                "NAME" => vec![
                    format!("tex@{}=", columns[3]),
                    format!("name={} ", columns[4]),
                ],
                other => panic!("unknown row kind {other}"),
            };
            for text in wanted {
                assert!(needle.contains(&text), "{row}: no {text}");
            }
        }
    }

    /// IDA: N/A owns everything the services borrow, so a check can change a field between two calls.
    struct RenderTargetFixture {
        world: World,
        state: RendererState,
        renderer: RendererGlobals,
        manager: TextureManager,
        globals: RenderTargetGlobals,
        b3ff00: u8,
        discard: u32,
        statistics: RenderMemoryStatistics,
        calls: Vec<DeviceTraceEntry>,
        warnings: Vec<Vec<u8>>,
    }

    impl RenderTargetFixture {
        /// IDA: N/A a device-less 007C2450 fixture: the default group is 1920x1080, the render-target tables hold A8R8G8B8 at
        /// index 5 and R5G6B5 at 4.
        fn new() -> Self {
            let mut state = RendererState::default();
            state.back_buffer_desc.width = 1920;
            state.back_buffer_desc.height = 1080;
            state.back_buffer_desc.format = D3DFMT_X8R8G8B8;
            state.texture_caps_flags_5e0 = 4;
            state.max_render_targets_894 = 4;
            for table in [1, 3] {
                state.pixel_format_tables[table][5] = Some(pixel_format_from_d3d(D3DFMT_A8R8G8B8));
                state.pixel_format_tables[table][4] = Some(pixel_format_from_d3d(D3DFMT_R5G6B5));
            }
            let renderer = RendererGlobals {
                hdr: true,
                fp16_filtering: true,
                anisotropic_min_filter: true,
                non_pow2_textures: true,
                texture_formats: [true; 13],
                ..RendererGlobals::default()
            };
            let mut world = World::new();
            let depth = world
                .spawn(DepthStencilBuffer {
                    width_08: 1920,
                    height_0c: 1080,
                    ..DepthStencilBuffer::default()
                })
                .id();
            Self {
                world,
                state,
                renderer,
                manager: TextureManager {
                    depth_stencil_40: Some(depth),
                    ..TextureManager::default()
                },
                globals: RenderTargetGlobals::default(),
                b3ff00: 0,
                discard: 1,
                statistics: RenderMemoryStatistics::default(),
                calls: Vec::new(),
                warnings: Vec::new(),
            }
        }

        fn with<R>(
            &mut self,
            body: impl FnOnce(
                &mut World,
                &mut TextureManager,
                &mut RenderTargetGlobals,
                &mut RenderTargetServices<'_>,
            ) -> R,
        ) -> R {
            let warnings = &mut self.warnings;
            let mut services = RenderTargetServices {
                texture: TextureManagerServices::without_device(Some(&self.state), &mut self.calls),
                globals: &self.renderer,
                image_space_effects: 1,
                water_resolution_b45fc8: 128,
                blur_texture_size_b2c2b8: 256,
                format_override_b3ff00: &mut self.b3ff00,
                depth_discard_b294ec: &mut self.discard,
                buffer_statistics: &mut self.statistics,
                warning: Some(Box::new(|text: &[u8]| warnings.push(text.to_vec()))),
            };
            body(
                &mut self.world,
                &mut self.manager,
                &mut self.globals,
                &mut services,
            )
        }

        fn kind(&mut self, kind: u32) -> RenderTargetRequest {
            self.with(|_, _, _, services| render_target_kind(kind, services))
        }

        fn create_targets(&mut self) -> [Option<Entity>; 13] {
            self.with(|world, manager, globals, services| {
                create_render_targets(world, manager, globals, services)
            })
        }

        fn get(&mut self, width: i32, height: i32, flags: u32, format: i32) -> Option<Entity> {
            self.with(|world, manager, globals, services| {
                get_rendered_texture(
                    world, manager, width, height, flags, format, 0, globals, services,
                )
            })
        }

        fn rendered(&self, target: Option<Entity>) -> RenderedTexture {
            *self.world.get::<RenderedTexture>(target.unwrap()).unwrap()
        }

        fn object(&self, target: Option<Entity>) -> &RenderedTextureObject {
            self.world
                .get::<RenderedTextureObject>(self.rendered(target).texture_20)
                .unwrap()
        }

        fn group(&self, target: Option<Entity>, index: usize) -> RenderTargetGroup {
            *self
                .world
                .get::<RenderTargetGroup>(self.rendered(target).groups_08[index].unwrap())
                .unwrap()
        }

        fn text(&self, target: Option<Entity>) -> String {
            target_text(&self.world, target)
        }
    }

    /// IDA: N/A `target_text` in the C++ test.
    fn target_text(world: &World, target: Option<Entity>) -> String {
        let Some(rendered) = target.and_then(|entity| world.get::<RenderedTexture>(entity)) else {
            return "null".to_owned();
        };
        let (width, height) = rendered_texture_object_size(world, Some(rendered.texture_20));
        let object = world
            .get::<RenderedTextureObject>(rendered.texture_20)
            .unwrap();
        format!(
            "{width}x{height}/{}",
            object.renderer_data_24.unwrap().d3d_format
        )
    }

    /// Ported from `verify_texture_manager`. IDA: 007C1FF0.
    #[test]
    fn texture_manager() {
        let mut world = World::new();
        let mut state = RendererState::default();
        state.depth_stencil_desc.width = 1920;
        state.depth_stencil_desc.height = 1080;
        state.depth_stencil_desc.format = D3DFMT_D24S8;
        let mut calls = Vec::new();
        let mut discard = 1;
        let mut statistics = RenderMemoryStatistics::default();
        let (manager, status) = {
            let mut services = TextureManagerServices::without_device(Some(&state), &mut calls);
            construct_texture_manager(&mut world, &mut services, &mut discard, &mut statistics)
                .unwrap()
        };
        assert_eq!(status, TextureManagerStatus::Created);
        let depth = world
            .get::<DepthStencilBuffer>(manager.depth_stencil_40.unwrap())
            .unwrap();
        assert!(!depth.additional);
        assert_eq!(depth.width_08, 1920);
        assert_eq!(depth.d3d_format, D3DFMT_D24S8);
        assert!(manager.texture_data_00.is_empty());
        assert!(manager.rendered_textures_20.is_empty());
        assert_eq!(manager.pointer_44, 0);
        assert_eq!(discard, 1);

        // 007C1FF0 multisample branch without a device must store null.
        let mut services = TextureManagerServices::without_device(Some(&state), &mut calls);
        services.multisample = 2;
        let (multisample, status) =
            construct_texture_manager(&mut world, &mut services, &mut discard, &mut statistics)
                .unwrap();
        assert!(multisample.depth_stencil_40.is_none());
        assert_eq!(status, TextureManagerStatus::NoAdditionalDepthStencil);
        assert_eq!(discard, 1);
        assert_eq!(statistics.buffer_bytes, 0);

        // Not in the C++ test: the `invalid_argument` arm.
        let mut services = TextureManagerServices::without_device(None, &mut calls);
        assert_eq!(
            construct_texture_manager(&mut world, &mut services, &mut discard, &mut statistics)
                .unwrap_err()
                .0,
            "007C1FF0: renderer state required"
        );
    }

    fn table_with(present: &[usize]) -> [Option<PixelFormat>; 22] {
        let mut table = [None; 22];
        for &index in present {
            table[index] = Some(PixelFormat::default());
        }
        table
    }

    /// Ported from `verify_render_target_formats`. IDA: 00773960.
    #[test]
    fn render_target_formats() {
        let table = table_with(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(select_render_target_format(&table, [2, 2, 2]), Some(5));
        assert_eq!(select_render_target_format(&table, [6, 3, 0]), Some(2));
        assert_eq!(select_render_target_format(&table, [0, 0, 0]), Some(6));
        assert_eq!(select_render_target_format(&table, [5, 2, 0]), Some(7));
        assert_eq!(select_render_target_format(&table, [3, 2, 0]), Some(9));
        assert_eq!(select_render_target_format(&table, [1, 1, 0]), Some(3));
        assert_eq!(select_render_target_format(&table, [14, 0, 0]), None);
        let table = table_with(&[4]);
        assert_eq!(select_render_target_format(&table, [2, 2, 2]), Some(4));
        assert_eq!(select_render_target_format(&table, [7, 0, 0]), Some(4));
        assert_eq!(select_render_target_format(&table, [8, 0, 0]), None);
        let table = table_with(&[18, 21]);
        assert_eq!(select_render_target_format(&table, [12, 0, 0]), Some(18));
        assert_eq!(select_render_target_format(&table, [13, 0, 0]), Some(21));
        assert_eq!(select_render_target_format(&table, [9, 0, 0]), Some(21));
    }

    /// Ported from `verify_render_target_kinds`. IDA: 007C0D10.
    #[test]
    fn render_target_kinds() {
        let mut fixture = RenderTargetFixture::new();
        let kind1 = fixture.kind(1);
        let kind4 = fixture.kind(4);
        let kind5 = fixture.kind(5);
        fixture.renderer.shadow_map_resolution = 1024;
        let kind23 = fixture.kind(23);
        assert_eq!((kind1.width, kind1.height), (256, 540));
        assert_eq!((kind1.flags, kind1.format), (38, 113));
        assert_eq!((kind4.width, kind4.height), (1920, 1080));
        assert_eq!((kind4.flags, kind4.format), (34, 113));
        assert_eq!((kind5.width, kind5.height, kind5.flags), (0, 1080, 38));
        assert_eq!((kind23.width, kind23.format, kind23.flags), (1024, 0x72, 6));
        fixture.state.back_buffer_desc.height = 400;
        fixture.renderer.multisample = 2;
        fixture.renderer.hdr = false;
        let floored = fixture.kind(1);
        let multisample = fixture.kind(4);
        assert_eq!(floored.height, 256);
        assert_eq!((multisample.flags, multisample.format), (162, 0));
    }

    /// Ported from `verify_render_targets`. IDA: 007C2450 and the 007C1B50/007C1430/007C1EE0 chain without a device.
    #[test]
    fn render_targets() {
        let mut fixture = RenderTargetFixture::new();
        let targets = fixture.create_targets();
        let text: Vec<String> = targets.iter().map(|&target| fixture.text(target)).collect();
        assert_eq!(
            text.join(" "),
            "1x1/113 1x1/113 1x1/113 4x4/113 16x16/113 64x64/113 256x256/113 256x256/113 1920x1080/113 256x540/113 \
             1920x1080/21 1920x1080/21 1920x1080/21",
            "007C2450 targets differ"
        );
        // Kind 4 (flags 34) reuses the +0x40 buffer; flags 0x24/0x26 take none.
        let group8 = fixture.group(targets[8], 0);
        assert_eq!(group8.depth_stencil_1c, fixture.manager.depth_stencil_40);
        assert!(fixture.group(targets[0], 0).depth_stencil_1c.is_none());
        assert!(fixture.rendered(targets[8]).groups_08[1].is_none());
        assert_eq!(group8.buffer_count_18, 1);
        assert_eq!(group8.buffers_08[0], fixture.object(targets[8]).buffers[0]);
        assert_eq!(fixture.object(targets[8]).render_target_3c, 1);
        assert_eq!(fixture.object(targets[8]).format_override_34, 1);
        assert_eq!(fixture.object(targets[10]).format_override_34, 0);
        // 007C1EE0 moved all 13 to the free list in order; the globals are back.
        assert_eq!(fixture.manager.texture_data_00.len(), 13);
        assert!(fixture.manager.texture_data_10.is_empty());
        assert_eq!(fixture.manager.texture_data_00[8].texture_00, targets[8]);
        assert_eq!(fixture.manager.texture_data_00[8].flags_0c, 34);
        assert_eq!(fixture.manager.texture_data_00[12].flags_0c, 38);
        assert_eq!(fixture.b3ff00, 0);
        assert_eq!(fixture.globals.render_target_b27530, 1);
        assert_eq!(fixture.discard, 1);
        assert_eq!(fixture.globals.override_format_b2752c, 113);
        assert_eq!(fixture.globals.texture_bytes_b42064, 43_661_464);
        assert_eq!(fixture.globals.texture_padding_b42068, 18_280);
        assert_eq!(fixture.globals.renderer_map_230.len(), 13);
        assert!(fixture.warnings.is_empty());
        // 007C1B50 reuse: flags compare with | 0x22, the first free match wins.
        let reused = fixture.get(1, 1, 0x04, 113);
        let screen = fixture.get(0, 0, 0x26, 0);
        assert_eq!(reused, targets[0]);
        assert_eq!(screen, targets[10]);
        assert_eq!(fixture.manager.texture_data_10.len(), 2);
        assert_eq!(fixture.manager.texture_data_00.len(), 11);
        release_rendered_texture(&mut fixture.manager, reused);
        assert_eq!(
            fixture.manager.texture_data_00.last().unwrap().texture_00,
            targets[0]
        );
        assert_eq!(fixture.manager.texture_data_10.len(), 1);
        // An unsupported 36 falls back to 113 (flags 8 skips the check).
        fixture.renderer.texture_formats[10] = false;
        let fallback = fixture.get(8, 8, 0x26, 36);
        let kept = fixture.get(8, 8, 0x2E, 36);
        assert_eq!(fixture.manager.texture_data_10[1].format_04, 113);
        let data = fixture.object(fallback).renderer_data_24.unwrap();
        assert_eq!(data.d3d_format, 113);
        assert_eq!(fixture.manager.texture_data_10[2].format_04, 36);
        assert_eq!(fixture.object(kept).render_target_3c, 0);
        let data = fixture.object(kept).renderer_data_24.unwrap();
        assert_eq!(data.pool, D3DPOOL_MANAGED);
        assert_eq!(data.usage, 0);
        assert_eq!(fixture.globals.render_target_b27530, 1);

        // Without HDR only the three screen targets; power-of-two rounding and a depth buffer that no longer covers kind 4's
        // square.
        let mut plain = RenderTargetFixture::new();
        plain.renderer.hdr = false;
        plain.renderer.non_pow2_textures = false;
        let screens = plain.create_targets();
        assert!(screens[0].is_none());
        assert!(screens[9].is_none());
        assert_eq!(plain.text(screens[10]), "2048x2048/21");
        assert_eq!(plain.manager.texture_data_00.len(), 3);
        let rounded = plain.with(|world, manager, globals, services| {
            create_rendered_texture(world, manager, 1920, 1080, 113, 0, 34, globals, services)
        });
        assert_eq!(plain.text(rounded), "2048x2048/113");
        assert!(plain.group(rounded, 0).depth_stencil_1c.is_none());

        // 0072A9B0 rejects a non-power-of-two size without renderer flags 8/4: the record keeps a null texture and 007C1430 warns.
        let mut rejected = RenderTargetFixture::new();
        rejected.state.texture_caps_flags_5e0 = 0;
        let none = rejected.get(1920, 1080, 0x26, 0);
        assert!(none.is_none());
        assert_eq!(rejected.manager.texture_data_10.len(), 1);
        assert!(rejected.manager.texture_data_10[0].texture_00.is_none());
        assert_eq!(
            rejected.warnings,
            [b"Unable to create rendered texture".to_vec()]
        );

        // The cube path (flags 0x10): six faces, six groups sharing one depth.
        let mut cube = RenderTargetFixture::new();
        let created = cube.with(|world, manager, globals, services| {
            create_rendered_texture(world, manager, 64, 64, 0, 0, 0x10, globals, services)
        });
        assert!(cube.object(created).cube);
        assert_eq!(cube.object(created).original_vtable, 0x00AB_27FC);
        assert!(cube.object(created).renderer_data_24.unwrap().cube);
        assert_eq!(
            cube.group(created, 5).buffers_08[0],
            cube.object(created).buffers[5]
        );
        assert_eq!(
            cube.group(created, 0).depth_stencil_1c,
            cube.manager.depth_stencil_40
        );
        assert_eq!(cube.globals.cube_bytes_b42860, 6 * 64 * 64 * 4);
        assert_eq!(cube.globals.renderer_map_234.len(), 1);
    }

    fn pinned_settings() -> RendererSettings {
        let mut collections = SettingCollections::default();
        register_static_settings(&mut collections);
        collections
            .ini
            .set_path(fixtures::root().join("config/Oblivion.ini"));
        assert!(collections.ini.open_ini());
        collections.ini.read_all();
        collections.ini.close();
        renderer_settings_from(&collections.ini)
    }

    /// IDA: N/A `RendererCreation` from the pinned caps: the device of the two Phase 05 runs, behind the seam.
    pub(crate) fn pinned_renderer() -> RendererCreation {
        let inputs = RendererCreationInputs {
            main_window: Some(crate::gamebryo::dx9_system::WindowHandle(0x0871_1BAC)),
            instance: 0x0040_0000,
            window_class: "Oblivion".to_owned(),
            my_games_root: PathBuf::from("/virtual/My Games/Oblivion"),
            adapter_formats: vec![D3DFMT_X8R8G8B8, D3DFMT_R5G6B5, D3DFMT_X1R5G5B5],
            settings: pinned_settings(),
        };
        let mut platform = PinnedRendererPlatform::new(DeviceCapsFixture::pinned());
        let created = initialize_renderer(&inputs, &mut platform);
        assert!(created.succeeded);
        created
    }

    fn global_row(name: &str) -> Vec<u8> {
        let rows = rows("GLOBAL");
        let row = rows
            .iter()
            .find(|row| row[2] == name)
            .unwrap_or_else(|| panic!("no global {name}"));
        (0..row[3].len() / 2)
            .map(|i| u8::from_str_radix(&row[3][2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    fn global_u32(name: &str) -> u32 {
        let bytes = global_row(name);
        let mut word = [0; 4];
        word[..bytes.len().min(4)].copy_from_slice(&bytes[..bytes.len().min(4)]);
        u32::from_le_bytes(word)
    }

    /// Not in the C++ test, which has no device. The pinned device and the P05-RUN-002 inputs through 007C2450 give the run's 13
    /// 007C1B50 calls, the state 007C1EE0 saw, the free list and the statistics, with the device calls on the trace.
    #[test]
    fn render_targets_match_p05_run_002() {
        assert_eq!(rows("GET").len(), 13);
        assert_eq!(rows("REL").len(), 13);
        assert_eq!(rows("FREE").len(), 13);
        let mut creation = pinned_renderer();
        // RT_DONE's inputs, as the run read them.
        creation.globals.hdr = global_u32("hdr@B43070") != 0;
        creation.globals.fp16_filtering = global_u32("fp16Filtering@B43071") != 0;
        creation.globals.anisotropic_min_filter = global_u32("anisoMin@B43072") != 0;
        creation.globals.non_pow2_textures = global_u32("nonPow2@B42E96") != 0;
        assert_eq!(
            creation.globals.multisample as u32,
            global_u32("multisample@B34FC0")
        );
        let formats = global_row("texFormats@B42E98");
        for (slot, byte) in creation.globals.texture_formats.iter_mut().zip(&formats) {
            *slot = *byte != 0;
        }
        let image_space = global_u32("imageSpace@B42F3E") as u8;
        let water = global_u32("waterRes@B45FC8") as i32;
        let system = creation.system.clone().unwrap();
        let desc = system
            .adapter(creation.state.adapter_5bc)
            .and_then(|adapter| adapter.hal_460.as_ref());
        let first_call = creation.device_calls.len();
        let globals_copy = creation.globals.clone();

        let mut world = World::new();
        let mut discard = global_u32("depthDiscard@B294EC");
        let mut b3ff00 = global_u32("fmtOverride@B3FF00") as u8;
        let mut statistics = RenderMemoryStatistics::default();
        let mut globals = RenderTargetGlobals::default();
        let mut texture = TextureManagerServices::from_creation(&mut creation, desc);
        let (mut manager, status) =
            construct_texture_manager(&mut world, &mut texture, &mut discard, &mut statistics)
                .unwrap();
        assert_eq!(status, TextureManagerStatus::Created);
        let mut services = RenderTargetServices {
            texture,
            globals: &globals_copy,
            image_space_effects: image_space,
            water_resolution_b45fc8: water,
            blur_texture_size_b2c2b8: 256,
            format_override_b3ff00: &mut b3ff00,
            depth_discard_b294ec: &mut discard,
            buffer_statistics: &mut statistics,
            warning: None,
        };
        let targets = create_render_targets(&mut world, &mut manager, &mut globals, &mut services);
        drop(services);

        // The 13 locals, in the order 007C1EE0 saw them.
        for row in rows("REL") {
            let index = number(&row[2]) as usize - 1;
            let rendered = *world
                .get::<RenderedTexture>(targets[index].unwrap())
                .unwrap();
            assert_eq!(rendered.original_vtable, 0x00A9_1370);
            let object = world
                .get::<RenderedTextureObject>(rendered.texture_20)
                .unwrap();
            let data = object.renderer_data_24.unwrap();
            assert_eq!(object.original_vtable, 0x00A8_0204);
            assert_eq!(object.prefs_18, [2, 2, 2]);
            assert_eq!(
                u32::from(object.format_override_34),
                hex(&row[3]),
                "{row:?}"
            );
            assert_eq!(object.override_format_38 as u32, hex(&row[4]), "{row:?}");
            assert_eq!(u32::from(object.render_target_3c), hex(&row[5]), "{row:?}");
            let (width, height) = rendered_texture_object_size(&world, Some(rendered.texture_20));
            assert_eq!(width, number(&row[6]), "{row:?}");
            assert_eq!(height, number(&row[7]), "{row:?}");
            assert_eq!(data.original_vtable, 0x00A8_8464);
            assert_eq!((data.width_54, data.height_58), (width, height));
            assert_eq!(data.levels_5c, 1);
            assert_eq!(data.texture_bytes_60, number(&row[8]), "{row:?}");
            // pfmt: flags, bits, the format word, tiling, the D3D format and the table case, bytes as the run dumped them.
            let f = data.pixel_format_0c;
            let pfmt = format!(
                "{:02x}{:02x}0000{:08x}{:08x}{:08x}{:08x}",
                f.flags_00,
                f.bits_per_pixel_01,
                f.format_04.swap_bytes(),
                f.tiling_08.swap_bytes(),
                f.renderer_hint_0c.swap_bytes(),
                f.extra_10.swap_bytes()
            );
            assert_eq!(pfmt, row[9], "{row:?}");
            let group = world
                .get::<RenderTargetGroup>(rendered.groups_08[0].unwrap())
                .unwrap();
            assert_eq!(group.original_vtable, 0x00AB_286C);
            assert_eq!(group.buffer_count_18, number(&row[10]), "{row:?}");
            assert_eq!(group.depth_stencil_1c.is_some(), row[11] == "1", "{row:?}");
            assert!(rendered.groups_08[1..].iter().all(Option::is_none));
        }
        // 00406B53: the free list, whose records hold the 13 in order.
        assert!(manager.texture_data_10.is_empty());
        for row in rows("FREE") {
            let record = manager.texture_data_00[number(&row[2]) as usize];
            assert_eq!(record.format_04 as u32, number(&row[3]), "{row:?}");
            assert_eq!(record.flags_0c, hex(&row[4]), "{row:?}");
            assert_eq!(record.usage_08, 0);
            assert_eq!(record.in_use_10, 1);
        }
        // The requests the 13 007C1B50 calls carried: a record keeps its flags and format.
        for row in rows("GET") {
            let index = number(&row[2]) as usize - 1;
            let record = manager.texture_data_00[index];
            assert_eq!(record.flags_0c, hex(&row[5]), "{row:?}");
            assert_eq!(record.format_04 as u32, number(&row[6]), "{row:?}");
            assert_eq!(record.texture_00, targets[index]);
            // The 007C1B50 size, on what the record's BSRenderedTexture holds.
            let target = world
                .get::<RenderedTexture>(targets[index].unwrap())
                .unwrap();
            let data = world
                .get::<RenderedTextureObject>(target.texture_20)
                .unwrap()
                .renderer_data_24
                .as_ref()
                .unwrap();
            assert_eq!(data.width_54, number(&row[3]), "{row:?}");
            assert_eq!(data.height_58, number(&row[4]), "{row:?}");
        }
        // RT_DONE: 43,661,464 bytes and 18,280 of padding, as two dwords of little-endian bytes.
        let totals = global_row("rtBytes@B42064");
        assert_eq!(
            totals[..4],
            globals.texture_bytes_b42064.to_le_bytes(),
            "00B42064"
        );
        assert_eq!(
            totals[4..],
            globals.texture_padding_b42068.to_le_bytes(),
            "00B42068"
        );
        assert_eq!(globals.texture_bytes_b42064, 43_661_464);
        assert_eq!(globals.texture_padding_b42068, 18_280);
        assert_eq!(globals.cube_bytes_b42860, global_u32("cubeBytes@B42860"));
        assert_eq!(statistics.buffer_bytes, global_u32("bufferBytes@B3FAB8"));
        assert_eq!(
            globals.override_format_b2752c as u32,
            global_u32("overrideFormat@B2752C")
        );
        assert_eq!(u32::from(b3ff00), 0);
        assert_eq!(
            u32::from(globals.render_target_b27530),
            global_u32("renderTarget@B27530")
        );
        assert_eq!(discard, global_u32("depthDiscard@B294EC"));
        // +0x40: the implicit buffer, at the back buffer's size.
        let depth_row = &rows("DEPTH")[0];
        let depth = world
            .get::<DepthStencilBuffer>(manager.depth_stencil_40.unwrap())
            .unwrap();
        assert_eq!(depth.original_vtable, hex(&depth_row[2]));
        assert_eq!(depth.width_08, hex(&depth_row[3]));
        assert_eq!(depth.height_0c, hex(&depth_row[4]));

        // The device saw one CreateTexture, GetSurfaceLevel and GetDesc per target, in that order, and nothing else.
        let calls = &creation.device_calls[first_call..];
        assert_eq!(calls.len(), 3 * 13);
        for (index, call) in calls.chunks(3).enumerate() {
            let data = world
                .get::<RenderedTextureObject>(
                    world
                        .get::<RenderedTexture>(targets[index].unwrap())
                        .unwrap()
                        .texture_20,
                )
                .unwrap()
                .renderer_data_24
                .unwrap();
            assert_eq!(call[0].site, 0x0076_17F1);
            assert_eq!(
                call[0].call,
                DeviceCall::CreateTexture {
                    width: data.width_54,
                    height: data.height_58,
                    levels: 1,
                    usage: D3DUSAGE_RENDERTARGET,
                    format: data.d3d_format,
                    pool: D3DPOOL_DEFAULT,
                }
            );
            assert_eq!(call[1].site, 0x0076_D91D);
            assert_eq!(
                call[1].call,
                DeviceCall::TextureGetSurfaceLevel { level: 0 }
            );
            assert_eq!(call[2].site, 0x0076_D958);
            assert_eq!(call[2].call, DeviceCall::SurfaceGetDesc);
        }
    }

    /// Not in the C++ test, which notes the branch is uncompared: 007C1FF0's multisample arm over the pinned device. The run had
    /// multisample 0, so this checks the port against the C++ only.
    #[test]
    fn texture_manager_multisample_device_arm() {
        let mut creation = pinned_renderer();
        let system = creation.system.clone().unwrap();
        let desc = system
            .adapter(creation.state.adapter_5bc)
            .and_then(|adapter| adapter.hal_460.as_ref());
        let first_call = creation.device_calls.len();
        let mut world = World::new();
        let mut discard = 1;
        let mut statistics = RenderMemoryStatistics::default();
        let mut services = TextureManagerServices::from_creation(&mut creation, desc);
        services.multisample = 2;
        let (manager, status) =
            construct_texture_manager(&mut world, &mut services, &mut discard, &mut statistics)
                .unwrap();
        assert_eq!(status, TextureManagerStatus::Created);
        assert_eq!(discard, 1);
        let depth = world
            .get::<DepthStencilBuffer>(manager.depth_stencil_40.unwrap())
            .unwrap();
        assert!(depth.additional);
        assert!(!depth.discard, "00B294EC is 0 for the call");
        assert_eq!(depth.d3d_format, D3DFMT_D24S8);
        assert_eq!(depth.byte_size_14, 4 * 1920 * 1080);
        assert_eq!(statistics.buffer_bytes, 4 * 1920 * 1080);
        assert_eq!(statistics.buffer_padding, page_padding(4 * 1920 * 1080));
        let calls = &creation.device_calls[first_call..];
        let sites: Vec<u32> = calls.iter().map(|call| call.site).collect();
        assert_eq!(sites, [0x0076_DE63, 0x0076_DED6, 0x0076_DF11]);
        assert!(matches!(
            calls[1].call,
            DeviceCall::CreateDepthStencilSurface {
                width: 1920,
                height: 1080,
                format: D3DFMT_D24S8,
                multi_sample: 0,
                multi_sample_quality: 0,
                discard: false,
            }
        ));
    }
}
