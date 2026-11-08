//! The pinned device: `fixtures/config/device-caps.txt` parsed, and a host behind `Direct3D9`, `Direct3DDevice9` and
//! `RendererPlatform` that answers from it. IDA: N/A throughout; this is the host's stand-in for the D3D9 runtime and Win32
//! (recorded substitution). wgpu limits never feed game state, so the Bevy build drives the renderer port through this too.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::dx9_system::*;
use super::renderer::{CreateWindowRequest, RendererPlatform, pixel_format_to_d3d};
use super::renderer_tables::TEXTURE_PIXEL_FORMATS;

/// IDA: N/A where a fixture value came from (the file's header defines each).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapSource {
    Recorded,
    Derived,
    Assumed,
}

type CapField = fn(&mut D3dCaps9) -> &mut u32;

/// IDA: N/A D3DCAPS9 in layout order: index * 4 is the offset, and the name is the SDK's.
pub const CAPS_FIELDS: [(&str, CapField); 76] = [
    ("DeviceType", |c| &mut c.device_type),
    ("AdapterOrdinal", |c| &mut c.adapter_ordinal),
    ("Caps", |c| &mut c.caps),
    ("Caps2", |c| &mut c.caps2),
    ("Caps3", |c| &mut c.caps3),
    ("PresentationIntervals", |c| &mut c.presentation_intervals),
    ("CursorCaps", |c| &mut c.cursor_caps),
    ("DevCaps", |c| &mut c.dev_caps),
    ("PrimitiveMiscCaps", |c| &mut c.primitive_misc_caps),
    ("RasterCaps", |c| &mut c.raster_caps),
    ("ZCmpCaps", |c| &mut c.z_cmp_caps),
    ("SrcBlendCaps", |c| &mut c.src_blend_caps),
    ("DestBlendCaps", |c| &mut c.dest_blend_caps),
    ("AlphaCmpCaps", |c| &mut c.alpha_cmp_caps),
    ("ShadeCaps", |c| &mut c.shade_caps),
    ("TextureCaps", |c| &mut c.texture_caps),
    ("TextureFilterCaps", |c| &mut c.texture_filter_caps),
    ("CubeTextureFilterCaps", |c| &mut c.cube_texture_filter_caps),
    ("VolumeTextureFilterCaps", |c| {
        &mut c.volume_texture_filter_caps
    }),
    ("TextureAddressCaps", |c| &mut c.texture_address_caps),
    ("VolumeTextureAddressCaps", |c| {
        &mut c.volume_texture_address_caps
    }),
    ("LineCaps", |c| &mut c.line_caps),
    ("MaxTextureWidth", |c| &mut c.max_texture_width),
    ("MaxTextureHeight", |c| &mut c.max_texture_height),
    ("MaxVolumeExtent", |c| &mut c.max_volume_extent),
    ("MaxTextureRepeat", |c| &mut c.max_texture_repeat),
    ("MaxTextureAspectRatio", |c| &mut c.max_texture_aspect_ratio),
    ("MaxAnisotropy", |c| &mut c.max_anisotropy),
    ("MaxVertexW", |c| &mut c.max_vertex_w),
    ("GuardBandLeft", |c| &mut c.guard_band_left),
    ("GuardBandTop", |c| &mut c.guard_band_top),
    ("GuardBandRight", |c| &mut c.guard_band_right),
    ("GuardBandBottom", |c| &mut c.guard_band_bottom),
    ("ExtentsAdjust", |c| &mut c.extents_adjust),
    ("StencilCaps", |c| &mut c.stencil_caps),
    ("FVFCaps", |c| &mut c.fvf_caps),
    ("TextureOpCaps", |c| &mut c.texture_op_caps),
    ("MaxTextureBlendStages", |c| &mut c.max_texture_blend_stages),
    ("MaxSimultaneousTextures", |c| {
        &mut c.max_simultaneous_textures
    }),
    ("VertexProcessingCaps", |c| &mut c.vertex_processing_caps),
    ("MaxActiveLights", |c| &mut c.max_active_lights),
    ("MaxUserClipPlanes", |c| &mut c.max_user_clip_planes),
    ("MaxVertexBlendMatrices", |c| {
        &mut c.max_vertex_blend_matrices
    }),
    ("MaxVertexBlendMatrixIndex", |c| {
        &mut c.max_vertex_blend_matrix_index
    }),
    ("MaxPointSize", |c| &mut c.max_point_size),
    ("MaxPrimitiveCount", |c| &mut c.max_primitive_count),
    ("MaxVertexIndex", |c| &mut c.max_vertex_index),
    ("MaxStreams", |c| &mut c.max_streams),
    ("MaxStreamStride", |c| &mut c.max_stream_stride),
    ("VertexShaderVersion", |c| &mut c.vertex_shader_version),
    ("MaxVertexShaderConst", |c| &mut c.max_vertex_shader_const),
    ("PixelShaderVersion", |c| &mut c.pixel_shader_version),
    ("PixelShader1xMaxValue", |c| {
        &mut c.pixel_shader_1x_max_value
    }),
    ("DevCaps2", |c| &mut c.dev_caps2),
    ("MaxNpatchTessellationLevel", |c| {
        &mut c.max_npatch_tessellation_level
    }),
    ("Reserved5", |c| &mut c.reserved5),
    ("MasterAdapterOrdinal", |c| &mut c.master_adapter_ordinal),
    ("AdapterOrdinalInGroup", |c| &mut c.adapter_ordinal_in_group),
    ("NumberOfAdaptersInGroup", |c| {
        &mut c.number_of_adapters_in_group
    }),
    ("DeclTypes", |c| &mut c.decl_types),
    ("NumSimultaneousRTs", |c| &mut c.num_simultaneous_rts),
    ("StretchRectFilterCaps", |c| &mut c.stretch_rect_filter_caps),
    ("VS20Caps.Caps", |c| &mut c.vs20_caps),
    ("VS20Caps.DynamicFlowControlDepth", |c| {
        &mut c.vs20_dynamic_flow_control_depth
    }),
    ("VS20Caps.NumTemps", |c| &mut c.vs20_num_temps),
    ("VS20Caps.StaticFlowControlDepth", |c| {
        &mut c.vs20_static_flow_control_depth
    }),
    ("PS20Caps.Caps", |c| &mut c.ps20_caps),
    ("PS20Caps.DynamicFlowControlDepth", |c| {
        &mut c.ps20_dynamic_flow_control_depth
    }),
    ("PS20Caps.NumTemps", |c| &mut c.ps20_num_temps),
    ("PS20Caps.StaticFlowControlDepth", |c| {
        &mut c.ps20_static_flow_control_depth
    }),
    ("PS20Caps.NumInstructionSlots", |c| {
        &mut c.ps20_num_instruction_slots
    }),
    ("VertexTextureFilterCaps", |c| {
        &mut c.vertex_texture_filter_caps
    }),
    ("MaxVShaderInstructionsExecuted", |c| {
        &mut c.max_vshader_instructions_executed
    }),
    ("MaxPShaderInstructionsExecuted", |c| {
        &mut c.max_pshader_instructions_executed
    }),
    ("MaxVertexShader30InstructionSlots", |c| {
        &mut c.max_vertex_shader30_instruction_slots
    }),
    ("MaxPixelShader30InstructionSlots", |c| {
        &mut c.max_pixel_shader30_instruction_slots
    }),
];

/// IDA: N/A the parsed fixture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceCapsFixture {
    pub caps: D3dCaps9,
    pub adapter_count: u32,
    pub description: Vec<u8>,
    pub driver: Vec<u8>,
    pub desktop_format: u32,
    /// (format, count), in file order.
    pub mode_counts: Vec<(u32, u32)>,
    pub mode_width: u32,
    pub mode_height: u32,
    pub mode_refresh: u32,
    pub check_device_type_result: u32,
    pub render_target_formats: Vec<u32>,
    pub depth_stencil_formats: Vec<u32>,
    pub multi_sample_mask: u32,
    pub multi_sample_quality: u32,
    /// A bit per 007637D0 case.
    pub texture_cases: u32,
    /// A bit per 007637D0 case.
    pub cube_cases: u32,
    pub extra_texture_formats: Vec<u32>,
    pub fp16_blending_result: u32,
    pub fp16_filtering_result: u32,
    pub create_device_result: u32,
    pub device_caps_result: u32,
    pub class_menu_name: u32,
    pub window_style: u32,
    pub adjusted_rect: [i32; 4],
    /// Every line's name and source, in file order.
    pub sources: Vec<(String, CapSource)>,
}

// IDA: N/A a helper, with no address of its own.
fn parse_number(text: &str) -> Result<u32, String> {
    let parsed = match text.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => text.parse(),
    };
    parsed.map_err(|error| format!("bad number {text}: {error}"))
}

// IDA: N/A a helper, with no address of its own.
fn parse_list(text: &str) -> Result<Vec<u32>, String> {
    text.split(',').map(parse_number).collect()
}

// IDA: N/A a helper, with no address of its own.
fn parse_signed_list(text: &str) -> Result<Vec<i32>, String> {
    text.split(',')
        .map(|part| {
            part.parse()
                .map_err(|error| format!("bad number {part}: {error}"))
        })
        .collect()
}

/// IDA: N/A a helper, with no address of its own.
/// `NAME VALUE SOURCE NOTE`: a quoted value may hold spaces.
fn split_line(line: &str) -> Result<(&str, &str, &str), String> {
    let (name, rest) = line
        .split_once(' ')
        .ok_or_else(|| format!("no value: {line}"))?;
    let (value, rest) = if let Some(quoted) = rest.strip_prefix('"') {
        let (value, tail) = quoted
            .split_once('"')
            .ok_or_else(|| format!("unterminated quote: {line}"))?;
        (value, tail.trim_start())
    } else {
        rest.split_once(' ')
            .ok_or_else(|| format!("no source: {line}"))?
    };
    let source = rest.split(' ').next().unwrap_or(rest);
    Ok((name, value, source))
}

/// IDA: N/A parses the fixture. Every caps field and every seam line must be present, and an unknown name is an error, so
/// a fixture edit cannot quietly leave a value at zero.
pub fn parse_device_caps(text: &str) -> Result<DeviceCapsFixture, String> {
    let mut fixture = DeviceCapsFixture::default();
    let mut seen_caps = [false; 76];
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, value, source) = split_line(line)?;
        let source = match source {
            "recorded" => CapSource::Recorded,
            "derived" => CapSource::Derived,
            "assumed" => CapSource::Assumed,
            other => return Err(format!("{name}: bad source {other}")),
        };
        fixture.sources.push((name.to_owned(), source));
        if let Some(index) = CAPS_FIELDS.iter().position(|field| field.0 == name) {
            if seen_caps[index] {
                return Err(format!("{name}: given twice"));
            }
            seen_caps[index] = true;
            *CAPS_FIELDS[index].1(&mut fixture.caps) = parse_number(value)?;
            continue;
        }
        match name {
            "Adapter.Count" => fixture.adapter_count = parse_number(value)?,
            "Adapter.Description" => fixture.description = value.as_bytes().to_vec(),
            "Adapter.Driver" => fixture.driver = value.as_bytes().to_vec(),
            "Adapter.DesktopFormat" => fixture.desktop_format = parse_number(value)?,
            "Adapter.Mode" => {
                let parts = parse_list(value)?;
                let [width, height, refresh] = parts[..] else {
                    return Err(format!("{name}: width,height,refresh"));
                };
                fixture.mode_width = width;
                fixture.mode_height = height;
                fixture.mode_refresh = refresh;
            }
            "Check.DeviceType" => fixture.check_device_type_result = parse_number(value)?,
            "Check.RenderTargetFormats" | "Check.RenderTargetFormatsMore" => {
                fixture.render_target_formats.extend(parse_list(value)?);
            }
            "Check.DepthStencilFormats" => fixture.depth_stencil_formats = parse_list(value)?,
            "Check.MultiSampleMask" => fixture.multi_sample_mask = parse_number(value)?,
            "Check.MultiSampleQuality" => fixture.multi_sample_quality = parse_number(value)?,
            "Format.TextureCases" => fixture.texture_cases = parse_number(value)?,
            "Format.CubeCases" => fixture.cube_cases = parse_number(value)?,
            "Format.ExtraTextureFormats" => fixture.extra_texture_formats = parse_list(value)?,
            "Format.FP16BlendingResult" => fixture.fp16_blending_result = parse_number(value)?,
            "Format.FP16FilteringResult" => fixture.fp16_filtering_result = parse_number(value)?,
            "Device.CreateResult" => fixture.create_device_result = parse_number(value)?,
            "Device.CapsResult" => fixture.device_caps_result = parse_number(value)?,
            "Library.Present" | "Reference.Caps" => {}
            "Window.ClassMenuName" => fixture.class_menu_name = parse_number(value)?,
            "Window.Style" => fixture.window_style = parse_number(value)?,
            "Window.AdjustedRect" => {
                fixture.adjusted_rect = parse_signed_list(value)?
                    .try_into()
                    .map_err(|_| format!("{name}: left,top,right,bottom"))?;
            }
            _ => match name.strip_prefix("Adapter.ModeCount.") {
                Some(format) => fixture
                    .mode_counts
                    .push((parse_number(format)?, parse_number(value)?)),
                None => return Err(format!("unknown field {name}")),
            },
        }
    }
    if let Some(index) = seen_caps.iter().position(|seen| !seen) {
        return Err(format!("{} is missing", CAPS_FIELDS[index].0));
    }
    for required in [
        "Adapter.Count",
        "Adapter.Description",
        "Adapter.Driver",
        "Adapter.DesktopFormat",
        "Adapter.Mode",
        "Check.DeviceType",
        "Check.RenderTargetFormats",
        "Check.DepthStencilFormats",
        "Check.MultiSampleMask",
        "Check.MultiSampleQuality",
        "Format.TextureCases",
        "Format.CubeCases",
        "Format.ExtraTextureFormats",
        "Format.FP16BlendingResult",
        "Format.FP16FilteringResult",
        "Device.CreateResult",
        "Device.CapsResult",
        "Window.ClassMenuName",
        "Window.Style",
        "Window.AdjustedRect",
    ] {
        if !fixture.sources.iter().any(|(name, _)| name == required) {
            return Err(format!("{required} is missing"));
        }
    }
    Ok(fixture)
}

impl DeviceCapsFixture {
    /// IDA: N/A the fixture `fixtures/config/device-caps.txt` holds, embedded so a built game needs no fixtures folder.
    #[must_use]
    pub fn pinned() -> Self {
        parse_device_caps(include_str!("../../../../fixtures/config/device-caps.txt"))
            .expect("the pinned fixture parses")
    }

    // IDA: N/A a helper, with no address of its own.
    fn case_format_passes(&self, cases: u32, format: u32) -> bool {
        TEXTURE_PIXEL_FORMATS.iter().enumerate().any(|(case, row)| {
            case != 6 && case != 7 && cases & (1 << case) != 0 && pixel_format_to_d3d(row) == format
        })
    }
}

/// What the pinned device has done. Shared with whoever made it, because `RendererCreation` owns the device.
#[derive(Debug, Default)]
pub struct PinnedDeviceLog {
    /// The bytes of each UnlockRect, in order.
    pub unlocked: Vec<Vec<u8>>,
}

/// IDA: N/A `Direct3D9`, answering from the fixture.
pub struct PinnedDirect3D9 {
    pub fixture: Arc<DeviceCapsFixture>,
    pub log: Arc<Mutex<PinnedDeviceLog>>,
    /// Front first: the HRESULT of the next CreateDevice, then `Device.CreateResult`.
    pub create_device_results: Vec<u32>,
    /// What a failing CreateDevice leaves in the parameters' back buffer count, when it changes it.
    pub failed_back_buffer_count: Option<u32>,
}

impl Direct3D9 for PinnedDirect3D9 {
    /// IDA: N/A `PinnedDirect3D9`'s side of `Direct3D9`.
    fn adapter_count(&mut self) -> u32 {
        self.fixture.adapter_count
    }

    /// IDA: N/A `PinnedDirect3D9`'s side of `Direct3D9`.
    fn adapter_identifier(&mut self, _adapter: u32, _flags: u32) -> (u32, AdapterIdentifier) {
        (
            D3D_OK,
            AdapterIdentifier {
                driver: self.fixture.driver.clone(),
                description: self.fixture.description.clone(),
            },
        )
    }

    /// IDA: N/A `PinnedDirect3D9`'s side of `Direct3D9`.
    fn adapter_mode_count(&mut self, _adapter: u32, format: u32) -> u32 {
        self.fixture
            .mode_counts
            .iter()
            .find(|entry| entry.0 == format)
            .map_or(0, |entry| entry.1)
    }

    /// IDA: N/A `PinnedDirect3D9`'s side of `Direct3D9`.
    fn enum_adapter_modes(&mut self, _adapter: u32, format: u32, _mode: u32) -> (u32, DisplayMode) {
        (
            D3D_OK,
            DisplayMode {
                width: self.fixture.mode_width,
                height: self.fixture.mode_height,
                refresh_rate: self.fixture.mode_refresh,
                format,
            },
        )
    }

    /// IDA: N/A `PinnedDirect3D9`'s side of `Direct3D9`.
    fn adapter_display_mode(&mut self, _adapter: u32) -> (u32, DisplayMode) {
        (
            D3D_OK,
            DisplayMode {
                width: self.fixture.mode_width,
                height: self.fixture.mode_height,
                refresh_rate: self.fixture.mode_refresh,
                format: self.fixture.desktop_format,
            },
        )
    }

    /// IDA: N/A `PinnedDirect3D9`'s side of `Direct3D9`.
    fn device_caps(&mut self, _adapter: u32, device_type: u32) -> (u32, D3dCaps9) {
        let mut caps = self.fixture.caps;
        caps.device_type = device_type;
        (D3D_OK, caps)
    }

    /// IDA: N/A `PinnedDirect3D9`'s side of `Direct3D9`.
    fn check_device_type(
        &mut self,
        _adapter: u32,
        _device_type: u32,
        _display_format: u32,
        _back_buffer_format: u32,
        _windowed: bool,
    ) -> u32 {
        self.fixture.check_device_type_result
    }

    /// IDA: N/A `PinnedDirect3D9`'s side of `Direct3D9`.
    fn check_device_format(
        &mut self,
        _adapter: u32,
        _device_type: u32,
        _adapter_format: u32,
        usage: u32,
        resource_type: u32,
        format: u32,
    ) -> u32 {
        let fixture = &self.fixture;
        let passes = match usage {
            D3DUSAGE_RENDERTARGET => fixture.render_target_formats.contains(&format),
            D3DUSAGE_DEPTHSTENCIL => fixture.depth_stencil_formats.contains(&format),
            D3DUSAGE_QUERY_POSTPIXELSHADER_BLENDING => return fixture.fp16_blending_result,
            D3DUSAGE_QUERY_FILTER => return fixture.fp16_filtering_result,
            _ if resource_type == D3DRTYPE_CUBETEXTURE => {
                fixture.case_format_passes(fixture.cube_cases, format)
            }
            _ => {
                fixture.case_format_passes(fixture.texture_cases, format)
                    || fixture.extra_texture_formats.contains(&format)
            }
        };
        if passes { D3D_OK } else { D3DERR_NOTAVAILABLE }
    }

    /// IDA: N/A `PinnedDirect3D9`'s side of `Direct3D9`.
    fn check_device_multi_sample_type(
        &mut self,
        _adapter: u32,
        _device_type: u32,
        _format: u32,
        _windowed: bool,
        multi_sample_type: u32,
    ) -> (u32, u32) {
        let passes = multi_sample_type == 0
            || (multi_sample_type <= 16
                && self.fixture.multi_sample_mask & (1 << (multi_sample_type - 1)) != 0);
        (
            if passes { D3D_OK } else { D3DERR_NOTAVAILABLE },
            self.fixture.multi_sample_quality,
        )
    }

    /// IDA: N/A `PinnedDirect3D9`'s side of `Direct3D9`.
    fn check_depth_stencil_match(
        &mut self,
        _adapter: u32,
        _device_type: u32,
        _adapter_format: u32,
        _render_target_format: u32,
        depth_stencil_format: u32,
    ) -> u32 {
        if self
            .fixture
            .depth_stencil_formats
            .contains(&depth_stencil_format)
        {
            D3D_OK
        } else {
            D3DERR_NOTAVAILABLE
        }
    }

    /// IDA: N/A `PinnedDirect3D9`'s side of `Direct3D9`.
    fn create_device(
        &mut self,
        _adapter: u32,
        _device_type: u32,
        _focus_window: Option<WindowHandle>,
        _behavior_flags: u32,
        parameters: &mut PresentParameters,
    ) -> (u32, Option<Box<dyn Direct3DDevice9>>) {
        let result = if self.create_device_results.is_empty() {
            self.fixture.create_device_result
        } else {
            self.create_device_results.remove(0)
        };
        if failed(result) {
            if let Some(count) = self.failed_back_buffer_count {
                parameters.back_buffer_count = count;
            }
            return (result, None);
        }
        (
            result,
            Some(Box::new(PinnedDevice {
                fixture: self.fixture.clone(),
                log: self.log.clone(),
                parameters: *parameters,
                textures: Vec::new(),
                surfaces: Vec::new(),
            })),
        )
    }
}

/// IDA: N/A `Direct3DDevice9`, answering from the fixture. Surface 1 is the back buffer and 2 the depth stencil; the rest
/// index `surfaces`.
struct PinnedDevice {
    fixture: Arc<DeviceCapsFixture>,
    log: Arc<Mutex<PinnedDeviceLog>>,
    parameters: PresentParameters,
    /// (resource type, format, width, height).
    textures: Vec<(u32, u32, u32, u32)>,
    surfaces: Vec<SurfaceDesc>,
}

impl PinnedDevice {
    // IDA: N/A a helper, with no address of its own.
    fn new_texture(
        &mut self,
        kind: u32,
        format: u32,
        width: u32,
        height: u32,
    ) -> (u32, Option<TextureHandle>) {
        self.textures.push((kind, format, width, height));
        (D3D_OK, Some(TextureHandle(self.textures.len() as u32)))
    }

    // IDA: N/A a helper, with no address of its own.
    fn new_surface(&mut self, texture: TextureHandle, level: u32) -> (u32, Option<SurfaceHandle>) {
        let (_, format, width, height) = self.textures[texture.0 as usize - 1];
        self.surfaces.push(SurfaceDesc {
            format,
            width: (width >> level).max(1),
            height: (height >> level).max(1),
        });
        (D3D_OK, Some(SurfaceHandle(self.surfaces.len() as u32 + 2)))
    }

    // IDA: N/A a helper, with no address of its own.
    fn desc(&self, surface: SurfaceHandle) -> SurfaceDesc {
        match surface.0 {
            1 => SurfaceDesc {
                format: self.parameters.back_buffer_format,
                width: self.parameters.back_buffer_width,
                height: self.parameters.back_buffer_height,
            },
            2 => SurfaceDesc {
                format: self.parameters.auto_depth_stencil_format,
                width: self.parameters.back_buffer_width,
                height: self.parameters.back_buffer_height,
            },
            index => self.surfaces[index as usize - 3],
        }
    }
}

impl Direct3DDevice9 for PinnedDevice {
    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn device_caps(&mut self) -> (u32, D3dCaps9) {
        (self.fixture.device_caps_result, self.fixture.caps)
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn render_target(&mut self, _index: u32) -> (u32, Option<SurfaceHandle>) {
        (D3D_OK, Some(SurfaceHandle(1)))
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn depth_stencil_surface(&mut self) -> (u32, Option<SurfaceHandle>) {
        (D3D_OK, Some(SurfaceHandle(2)))
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn surface_desc(&mut self, surface: SurfaceHandle) -> (u32, SurfaceDesc) {
        (D3D_OK, self.desc(surface))
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn set_render_state(&mut self, _state: u32, _value: u32) -> u32 {
        D3D_OK
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn set_texture_stage_state(&mut self, _stage: u32, _state: u32, _value: u32) -> u32 {
        D3D_OK
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn set_sampler_state(&mut self, _stage: u32, _state: u32, _value: u32) -> u32 {
        D3D_OK
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn set_transform(&mut self, _state: u32, _matrix: &D3dMatrix) -> u32 {
        D3D_OK
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn set_viewport(&mut self, _viewport: &D3dViewport) -> u32 {
        D3D_OK
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn set_npatch_mode(&mut self, _segments: f32) -> u32 {
        D3D_OK
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn create_texture(
        &mut self,
        width: u32,
        height: u32,
        _levels: u32,
        _usage: u32,
        format: u32,
        _pool: u32,
    ) -> (u32, Option<TextureHandle>) {
        self.new_texture(D3DRTYPE_TEXTURE, format, width, height)
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn create_cube_texture(
        &mut self,
        edge: u32,
        _levels: u32,
        _usage: u32,
        format: u32,
        _pool: u32,
    ) -> (u32, Option<TextureHandle>) {
        self.new_texture(D3DRTYPE_CUBETEXTURE, format, edge, edge)
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn texture_type(&mut self, texture: TextureHandle) -> u32 {
        self.textures[texture.0 as usize - 1].0
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn texture_surface_level(
        &mut self,
        texture: TextureHandle,
        level: u32,
    ) -> (u32, Option<SurfaceHandle>) {
        self.new_surface(texture, level)
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn cube_map_surface(
        &mut self,
        texture: TextureHandle,
        _face: u32,
        level: u32,
    ) -> (u32, Option<SurfaceHandle>) {
        self.new_surface(texture, level)
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn create_depth_stencil_surface(
        &mut self,
        width: u32,
        height: u32,
        format: u32,
        _multi_sample: u32,
        _multi_sample_quality: u32,
        _discard: bool,
    ) -> (u32, Option<SurfaceHandle>) {
        self.surfaces.push(SurfaceDesc {
            format,
            width,
            height,
        });
        (D3D_OK, Some(SurfaceHandle(self.surfaces.len() as u32 + 2)))
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn lock_rect(&mut self, surface: SurfaceHandle) -> (u32, i32) {
        let desc = self.desc(surface);
        let pitch = match desc.format {
            D3DFMT_DXT1 => desc.width.div_ceil(4) * 8,
            D3DFMT_DXT3 | D3DFMT_DXT5 => desc.width.div_ceil(4) * 16,
            format => desc.width * (dx9_format_bits(format).max(8) as u32 / 8),
        };
        (D3D_OK, pitch as i32)
    }

    /// IDA: N/A `PinnedDevice`'s side of `Direct3DDevice9`.
    fn unlock_rect(&mut self, _surface: SurfaceHandle, bytes: &[u8]) -> u32 {
        self.log
            .lock()
            .expect("no test holds the log across a panic")
            .unlocked
            .push(bytes.to_vec());
        D3D_OK
    }
}

/// IDA: N/A `RendererPlatform`, answering from the fixture. Nothing touches a window or a disk: a RendererInfo.txt lands in
/// `written`, as the LF text the port produced.
pub struct PinnedRendererPlatform {
    pub fixture: Arc<DeviceCapsFixture>,
    pub log: Arc<Mutex<PinnedDeviceLog>>,
    /// False: LoadLibraryA("D3D9.DLL") fails.
    pub d3d9_library: bool,
    /// False: Direct3DCreate9 returns null.
    pub direct3d_create9: bool,
    /// False: fopen fails.
    pub file_opens: bool,
    /// The next CreateDevice answers, front first, for every interface this creates.
    pub create_device_results: Vec<u32>,
    pub failed_back_buffer_count: Option<u32>,
    pub written: Vec<(PathBuf, Vec<u8>)>,
    next_window: u32,
}

impl PinnedRendererPlatform {
    /// IDA: N/A a helper, with no address of its own.
    #[must_use]
    pub fn new(fixture: DeviceCapsFixture) -> Self {
        Self {
            fixture: Arc::new(fixture),
            log: Arc::default(),
            d3d9_library: true,
            direct3d_create9: true,
            file_opens: true,
            create_device_results: Vec::new(),
            failed_back_buffer_count: None,
            written: Vec::new(),
            next_window: 0x100,
        }
    }
}

const D3D9_MODULE: ModuleHandle = ModuleHandle(0xD3D9);
const CREATE9_ENTRY: ProcHandle = ProcHandle(0x00D3_D9C9);

impl RendererPlatform for PinnedRendererPlatform {
    /// IDA: attempt 3 of P04-RUN-001 recorded LoadLibraryA("NVCPL.dll") returning 0.
    fn load_library(&mut self, name: &str) -> Option<ModuleHandle> {
        (self.d3d9_library && name == "D3D9.DLL").then_some(D3D9_MODULE)
    }

    /// IDA: N/A `PinnedRendererPlatform`'s side of `RendererPlatform`.
    fn get_proc_address(&mut self, module: ModuleHandle, name: &str) -> Option<ProcHandle> {
        (module == D3D9_MODULE && name == "Direct3DCreate9").then_some(CREATE9_ENTRY)
    }

    /// IDA: N/A `PinnedRendererPlatform`'s side of `RendererPlatform`.
    fn free_library(&mut self, _module: ModuleHandle) -> bool {
        true
    }

    /// IDA: N/A `PinnedRendererPlatform`'s side of `RendererPlatform`.
    fn direct3d_create9(
        &mut self,
        _entry: ProcHandle,
        _sdk_version: u32,
    ) -> Option<Box<dyn Direct3D9>> {
        self.direct3d_create9.then(|| {
            Box::new(PinnedDirect3D9 {
                fixture: self.fixture.clone(),
                log: self.log.clone(),
                create_device_results: self.create_device_results.clone(),
                failed_back_buffer_count: self.failed_back_buffer_count,
            }) as Box<dyn Direct3D9>
        })
    }

    /// IDA: N/A `PinnedRendererPlatform`'s side of `RendererPlatform`.
    fn create_window(&mut self, _request: &CreateWindowRequest) -> Option<WindowHandle> {
        self.next_window += 1;
        Some(WindowHandle(self.next_window))
    }

    /// IDA: N/A `PinnedRendererPlatform`'s side of `RendererPlatform`.
    fn class_long(&mut self, _window: Option<WindowHandle>, _index: i32) -> u32 {
        self.fixture.class_menu_name
    }

    /// IDA: N/A `PinnedRendererPlatform`'s side of `RendererPlatform`.
    fn window_long(&mut self, _window: Option<WindowHandle>, _index: i32) -> u32 {
        self.fixture.window_style
    }

    /// IDA: N/A `PinnedRendererPlatform`'s side of `RendererPlatform`.
    fn adjust_window_rect(&mut self, _rect: [i32; 4], _style: u32, _menu: bool) -> [i32; 4] {
        self.fixture.adjusted_rect
    }

    /// IDA: N/A `PinnedRendererPlatform`'s side of `RendererPlatform`.
    fn set_window_pos(
        &mut self,
        _window: Option<WindowHandle>,
        _x: i32,
        _y: i32,
        _width: i32,
        _height: i32,
        _flags: u32,
    ) -> bool {
        true
    }

    /// IDA: N/A `PinnedRendererPlatform`'s side of `RendererPlatform`.
    fn ati_query_mgpu_count(&mut self, _entry: ProcHandle) -> i32 {
        0
    }

    /// IDA: N/A `PinnedRendererPlatform`'s side of `RendererPlatform`.
    fn nv_cpl_get_data_int(&mut self, _entry: ProcHandle, _key: i32) -> i32 {
        0
    }

    /// IDA: N/A `PinnedRendererPlatform`'s side of `RendererPlatform`.
    fn write_renderer_info(&mut self, path: &Path, text: &[u8]) -> bool {
        if self.file_opens {
            self.written.push((path.to_owned(), text.to_vec()));
        }
        self.file_opens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_fixture_parses_with_every_field() {
        let fixture = DeviceCapsFixture::pinned();
        assert_eq!(fixture.caps.pixel_shader_version, 0xFFFF_0300);
        assert_eq!(fixture.description, b"NVIDIA GeForce RTX 4090");
        assert_eq!(fixture.driver, b"nvldumd.dll");
        assert_eq!(fixture.mode_counts, [(22, 78), (23, 0), (24, 0)]);
        assert_eq!(fixture.adjusted_rect, [-3, -26, 1923, 1083]);
    }

    #[test]
    fn the_parser_rejects_what_would_leave_a_zero() {
        let text = include_str!("../../../../fixtures/config/device-caps.txt");
        let missing = text.replace("RasterCaps 0x", "# RasterCaps 0x");
        assert_eq!(
            parse_device_caps(&missing).unwrap_err(),
            "RasterCaps is missing"
        );
        let unknown = format!("{text}NotACap 1 recorded note\n");
        assert_eq!(
            parse_device_caps(&unknown).unwrap_err(),
            "unknown field NotACap"
        );
        let source = text.replace(
            "RasterCaps 0x07732191 recorded",
            "RasterCaps 0x07732191 guessed",
        );
        assert_eq!(
            parse_device_caps(&source).unwrap_err(),
            "RasterCaps: bad source guessed"
        );
        let twice = format!("{text}RasterCaps 0 recorded note\n");
        assert_eq!(
            parse_device_caps(&twice).unwrap_err(),
            "RasterCaps: given twice"
        );
    }
}
