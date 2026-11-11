//! NiRenderedTexture, Ni2DBuffer, NiDepthStencilBuffer, NiRenderTargetGroup and the NiDX9Renderer
//! functions that create them.

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
use bevy_ecs::prelude::*;

pub(crate) const NI_DEPTH_STENCIL_BUFFER_VTABLE: u32 = 0x00A7_E44C;

const NI_DX9_TEXTURE_BUFFER_DATA_VTABLE: u32 = 0x00A8_98E4;

const NI_2D_BUFFER_VTABLE: u32 = 0x00A8_098C;

pub(crate) const NI_DX9_RENDERED_TEXTURE_DATA_VTABLE: u32 = 0x00A8_8464;

const NI_DX9_RENDERED_CUBE_MAP_DATA_VTABLE: u32 = 0x00A8_A868;

pub(crate) const NI_RENDERED_TEXTURE_VTABLE: u32 = 0x00A8_0204;

const NI_RENDERED_CUBE_MAP_VTABLE: u32 = 0x00AB_27FC;

const NI_RENDER_TARGET_GROUP_VTABLE: u32 = 0x00AB_286C;

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
pub(crate) fn buffer_size(world: &World, buffer: Option<Entity>) -> (u32, u32) {
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
