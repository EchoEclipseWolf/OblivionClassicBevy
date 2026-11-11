//! The game's renderer creation WinMain 0040E82F..0040E88E calls: 00572DC0 RendererUIState_Initialize, and 004052F0
//! OSGlobals_InitializeRendererOrExit -> 00498340 Renderer_CreateGamebryoRenderer with the RendererInfo.txt report. The
//! Gamebryo renderer they create (NiDX9Renderer) is `crate::gamebryo::renderer`.

use oblivion_core::crt::strnicmp;

use crate::gamebryo::dx9_system::*;
use crate::gamebryo::renderer::*;
use crate::gamebryo::renderer::{D3DDEVCAPS_HWTRANSFORMANDLIGHT, D3DDEVCAPS_NPATCHES};

/// IDA: 00572DC0 RendererUIState_Initialize. 0040E82F FormHeap(8) -> each pass stores the float 0.0f at p + 4, then the byte
/// 0 at p - 8, then the dword 0 at p (fst/mov/mov at 00572DD0..00572DD6); then the object's two zero dwords.
/// Completion: 100%.
pub fn initialize_renderer_ui_state(globals: &mut RendererUiGlobals) -> RendererUiState {
    for record in &mut globals.records {
        record.scale = 0.0;
        globals.store_trace.push(4);
        record.flag = 0;
        globals.store_trace.push(0xFFFF_FFF8);
        record.value = 0;
        globals.store_trace.push(0);
    }
    RendererUiState::default()
}

/// IDA: 0049887E..00498968 the ATI/NVIDIA multi-GPU probes. Completion: 100%.
fn probe_multi_gpu(ctx: &mut RendererContext<'_>, driver: &[u8]) {
    let s = ctx.inputs.settings;
    if strnicmp(driver, b"ati", 3) == 0 {
        if let Some(module) = ctx.load_library(0x0049_889A, "ATIMGPUD.dll") {
            if let Some(query) = ctx.get_proc_address(0x0049_88AC, module, "AtiQueryMgpuCount") {
                let count = ctx.platform.ati_query_mgpu_count(query);
                ctx.call(0x0049_88B6, DeviceCall::AtiQueryMgpuCount, count as u32);
                if count > 0 {
                    ctx.result.globals.sli = true;
                    ctx.result.globals.multi_gpu_flag = false;
                }
            }
            ctx.free_library(0x0049_88CB, module);
        }
        let multisample = ctx.result.globals.multisample;
        ctx.result.globals.refraction = if multisample >= 2 {
            false
        } else {
            s.use_refraction_shader
        };
        return;
    }
    if strnicmp(driver, b"nv", 2) == 0 {
        if let Some(module) = ctx.load_library(0x0049_88FC, "NVCPL.dll") {
            if let Some(get) = ctx.get_proc_address(0x0049_890E, module, "NvCplGetDataInt") {
                let count = ctx.platform.nv_cpl_get_data_int(get, 8);
                ctx.call(
                    0x0049_892B,
                    DeviceCall::NvCplGetDataInt { key: 8 },
                    count as u32,
                );
                let mode = ctx.platform.nv_cpl_get_data_int(get, 9);
                ctx.call(
                    0x0049_8934,
                    DeviceCall::NvCplGetDataInt { key: 9 },
                    mode as u32,
                );
                if count > 0 && mode & 0x1000_0000 != 0 {
                    ctx.result.globals.sli = true;
                    ctx.result.globals.multi_gpu_flag = false;
                }
            }
            ctx.free_library(0x0049_8959, module);
        }
        ctx.result.globals.refraction = s.use_refraction_shader;
        return;
    }
    let multisample = ctx.result.globals.multisample;
    ctx.result.globals.refraction = if multisample >= 2 {
        false
    } else {
        s.use_refraction_shader
    };
}

/// IDA: 00498A92..00498D4B: "%sRendererInfo.txt" with 00B3F280, fopen "w". Completion: 100%. Host substitution: with no
/// renderer, 00498D25 00497E10(null) faults reading the render target group, so the host stops before the Multisample line.
fn write_renderer_info(
    ctx: &mut RendererContext<'_>,
    info: &RendererInfoInputs,
    renderer_present: bool,
) {
    let path = ctx.inputs.my_games_root.join("RendererInfo.txt");
    ctx.result.renderer_info_path.clone_from(&path);
    let mut text = renderer_info_text(&ctx.result.globals, &ctx.inputs.settings, info);
    if !renderer_present {
        let cut = b"\tMultisample Type";
        if let Some(position) = text.windows(cut.len()).position(|window| window == cut) {
            text.truncate(position);
        }
    }
    let opened = ctx.platform.write_renderer_info(&path, &text);
    ctx.call(
        0x0049_8AA3,
        DeviceCall::WriteRendererInfo { bytes: text.len() },
        u32::from(opened),
    );
    if !opened {
        return;
    }
    ctx.result.renderer_info_text = text;
    ctx.result.renderer_info_written = true;
}

/// IDA: 0049838F..00498566: the window adjustment and the 0076BD40 call. Answers whether the renderer exists.
/// Completion: 100%.
fn create_renderer_for_window(ctx: &mut RendererContext<'_>) -> bool {
    let s = ctx.inputs.settings;
    let main_window = ctx.result.globals.main_window;
    let mut device_window = main_window;
    if s.full_screen {
        ctx.result.globals.use_flags |= 4;
        ctx.result.globals.device_window = main_window;
    } else {
        // 00498413 child of the main window, same class and procedure.
        let width = ctx.result.globals.width as i32;
        let height = ctx.result.globals.height as i32;
        let request = CreateWindowRequest {
            ex_style: 0,
            class_name: ctx.inputs.window_class.clone(),
            style: WS_CHILD_VISIBLE,
            x: 0,
            y: 0,
            width,
            height,
            parent: main_window,
            instance: ctx.result.globals.instance,
        };
        let child = ctx.platform.create_window(&request);
        ctx.call(
            0x0049_8413,
            DeviceCall::CreateWindow {
                ex_style: 0,
                style: WS_CHILD_VISIBLE,
                x: 0,
                y: 0,
                width,
                height,
            },
            child.map_or(0, |w| w.0),
        );
        ctx.result.globals.device_window = child;
        ctx.result.child_window = child;
        // 0049843D GetClassLongA(child, GCL_MENUNAME) as the bMenu argument.
        let menu_name = ctx.platform.class_long(child, GCL_MENUNAME);
        ctx.call(
            0x0049_843D,
            DeviceCall::GetClassLong {
                index: GCL_MENUNAME,
            },
            menu_name,
        );
        let menu = menu_name != 0;
        let style = ctx.platform.window_long(main_window, GWL_STYLE);
        ctx.call(
            0x0049_844D,
            DeviceCall::GetWindowLong { index: GWL_STYLE },
            style,
        );
        let rect = [0, 0, width, height];
        let adjusted = ctx.platform.adjust_window_rect(rect, style, menu);
        ctx.call(
            0x0049_8459,
            DeviceCall::AdjustWindowRect { rect, style, menu },
            1,
        );
        let moved = ctx.platform.set_window_pos(
            main_window,
            s.location_x,
            s.location_y,
            adjusted[2].wrapping_sub(adjusted[0]),
            adjusted[3].wrapping_sub(adjusted[1]),
            SWP_SHOWWINDOW,
        );
        ctx.call(
            0x0049_8488,
            DeviceCall::SetWindowPos {
                x: s.location_x,
                y: s.location_y,
                width: adjusted[2].wrapping_sub(adjusted[0]),
                height: adjusted[3].wrapping_sub(adjusted[1]),
                flags: SWP_SHOWWINDOW,
            },
            u32::from(moved),
        );
        device_window = child;
    }
    let mut multisample = if s.high_dynamic_range || s.multisample <= 1 {
        0
    } else {
        s.multisample
    };
    ctx.result.globals.multisample = multisample;
    if s.allow_screenshot && multisample == 0 {
        multisample = 1;
        ctx.result.globals.multisample = 1;
    }
    let g = &ctx.result.globals;
    let arguments = CreateArguments {
        width: g.width,
        height: g.height,
        use_flags: g.use_flags,
        device_window,
        focus_window: device_window,
        adapter: s.adapter,
        device_desc: s.device_desc,
        frame_buffer_format: s.frame_buffer_format,
        depth_stencil_format: s.depth_stencil_format,
        presentation_interval: g.present_interval,
        swap_effect: s.swap_effect,
        frame_buffer_mode: multisample as u32,
        back_buffer_count: s.back_buffer_count,
        refresh_rate: s.refresh_rate,
    };
    create_dx9_renderer(ctx, &arguments)
}

/// IDA: 004052F0 -> 00498340. Completion: 100% of the reached device-visible calls. Host substitution: the failed path
/// returns with `failure_message` set instead of showing the box and calling ExitProcess(0). See `renderer_failure_message`.
/// TODO: [ ] Phase 05+ (carried): shader factories and programs, geometry and light managers, reset paths 0076A970/0076B3F0.
pub fn initialize_renderer(
    inputs: &RendererCreationInputs,
    platform: &mut dyn RendererPlatform,
) -> RendererCreation {
    let mut result = RendererCreation::default();
    let mut ctx = RendererContext {
        inputs,
        platform,
        result: &mut result,
    };
    run_renderer_creation(&mut ctx);
    result
}

/// IDA: 00498340 Renderer_CreateGamebryoRenderer. Completion: 100% of the reached device-visible calls.
fn run_renderer_creation(ctx: &mut RendererContext<'_>) {
    let inputs = ctx.inputs;
    let s = inputs.settings;
    {
        let g = &mut ctx.result.globals;
        g.use_flags = s.use_flags;
        g.applied_gamma = s.applied_gamma;
        g.pixel_shader_version = s.pixel_shader_version;
        g.vertex_shader_version = s.vertex_shader_version;
        g.shadows_on_grass = s.shadows_on_grass;
        g.multi_gpu_flag = false;
        // 0049838F..004983B2.
        g.main_window = inputs.main_window;
        g.instance = inputs.instance;
        g.present_interval = s.present_interval;
        g.width = s.width as u32;
        g.height = s.height as u32;
    }
    let renderer_present =
        validate_adapter_and_shaders(ctx, true) && create_renderer_for_window(ctx);
    if renderer_present {
        // 0049857C PostLoadNoOp(+0xA94); 00498591 SetBackgroundColor(00B350DC); 004985AE NiAlphaAccumulator sorter (host
        // substitution).
        ctx.result.state.background_color_5d4 =
            background_color_argb(s.background_red, s.background_green, s.background_blue);
        if ctx.result.globals.npatches
            && let Some(device) = ctx.result.device.as_mut()
        {
            let result = device.set_npatch_mode(s.npatches as f32);
            ctx.call(
                0x0049_85F5,
                DeviceCall::SetNPatchMode {
                    segments_bits: (s.npatches as f32).to_bits(),
                },
                result,
            );
        }
    } else {
        ctx.result.globals.failure_text = UNKNOWN_RENDERER_ERROR.to_owned();
    }
    // 00498614 NiDevImageConverter becomes the default; 0049865C/00498663.
    ctx.result.device_image_converter_default = true;
    ctx.result.globals.texture_create_renderer_data = true;
    ctx.result.globals.texture_mipmap_by_default = false;
    let system = adapter_enumerator(ctx);
    let Some(adapter) = system.as_ref().and_then(|system| system.adapter(s.adapter)) else {
        ctx.result.globals.failure_text = ADAPTER_DESC_FAILED.to_owned();
        ctx.result.failure_message = renderer_failure_message(&ctx.result.globals.failure_text);
        return;
    };
    let Some(hal) = adapter
        .hal_460
        .as_ref()
        .filter(|hal| hal.caps_004.device_type != 0)
    else {
        ctx.result.globals.failure_text = DEVICE_CAPS_FAILED.to_owned();
        ctx.result.failure_message = renderer_failure_message(&ctx.result.globals.failure_text);
        return;
    };
    if ctx.result.d3d9.is_none() {
        // 0049874B dereferences 00B42154, which is still null when 004980D0 failed before 0076BD40 ran: the original faults
        // here. The host keeps the last failure text and fails (recorded substitution).
        ctx.step("0049874B null g_d3d9 (original faults)");
        ctx.result.failure_message = renderer_failure_message(&ctx.result.globals.failure_text);
        return;
    }
    let caps = &hal.caps_004;
    let check_format = |ctx: &mut RendererContext<'_>, site: u32, usage: u32, format: u32| {
        let result = ctx
            .result
            .d3d9
            .as_mut()
            .expect("checked above")
            .check_device_format(
                0,
                D3DDEVTYPE_HAL,
                D3DFMT_X8R8G8B8,
                usage,
                D3DRTYPE_TEXTURE,
                format,
            );
        ctx.call(
            site,
            DeviceCall::CheckDeviceFormat {
                adapter: 0,
                device_type: D3DDEVTYPE_HAL,
                adapter_format: D3DFMT_X8R8G8B8,
                usage,
                resource_type: D3DRTYPE_TEXTURE,
                format,
            },
            result,
        );
        !failed(result)
    };
    ctx.result.globals.fullscreen_gamma =
        s.full_screen && caps.caps2 & D3DCAPS2_FULLSCREENGAMMA != 0;
    let mut instructions = caps.ps20_num_instruction_slots as u16;
    let description = adapter.identifier_004.description.clone();
    let driver = adapter.identifier_004.driver.clone();
    ctx.result.globals.hair_20 = s.allow_20_hair;
    ctx.result.globals.fp16_blending = check_format(
        ctx,
        0x0049_8755,
        D3DUSAGE_QUERY_POSTPIXELSHADER_BLENDING,
        D3DFMT_A16B16G16R16F,
    );
    ctx.result.globals.fp16_filtering = check_format(
        ctx,
        0x0049_877B,
        D3DUSAGE_QUERY_FILTER,
        D3DFMT_A16B16G16R16F,
    );
    ctx.result.globals.anisotropic_min_filter =
        caps.texture_filter_caps & D3DPTFILTERCAPS_MINFANISOTROPIC != 0;
    let non_pow2 = caps.texture_caps & D3DPTEXTURECAPS_POW2 == 0
        || caps.texture_caps & D3DPTEXTURECAPS_NONPOW2CONDITIONAL != 0;
    {
        let g = &mut ctx.result.globals;
        if s.high_dynamic_range && s.image_space_effects && g.fp16_blending {
            g.hdr = true;
            g.bloom = false;
        } else {
            g.hdr = false;
            g.bloom = s.image_space_effects && s.blur_shader;
        }
        g.non_pow2_textures = non_pow2 && !s.force_pow2_textures;
        g.mip_skip = s.tex_mip_skip;
        g.mip_minimum = s.tex_mip_minimum;
    }
    if instructions > 0x60 && caps.ps20_num_temps < 32 {
        instructions = 96;
    }
    let max_ps20 = i32::from(instructions);
    let path_inputs = RenderPathInputs {
        pixel_shader_version: ctx.result.globals.pixel_shader_version,
        vertex_shader_version: ctx.result.globals.vertex_shader_version,
        force_1x: s.force_1x_shaders,
        have_adapter: true,
        driver: driver.clone(),
        ps20_instructions: max_ps20,
    };
    select_render_path(&mut ctx.result.globals, &path_inputs);
    if ctx.result.globals.render_path < 5 {
        ctx.result.globals.shadows_on_grass = false;
    }
    ctx.result.globals.shadow_map_resolution = s.shadow_map_resolution as u16;
    probe_multi_gpu(ctx, &driver);
    // 00498981 fild/fdiv/fcomp at 24-bit precision once the device exists.
    ctx.result.globals.non_standard_aspect =
        ctx.result.globals.height as f32 / ctx.result.globals.width as f32 != 0.75;
    const FORMATS: [u32; 13] = [
        D3DFMT_R5G6B5,
        D3DFMT_X1R5G5B5,
        D3DFMT_A1R5G5B5,
        D3DFMT_A4R4G4B4,
        D3DFMT_L16,
        D3DFMT_R8G8B8,
        D3DFMT_A8R8G8B8,
        D3DFMT_X8R8G8B8,
        D3DFMT_R32F,
        D3DFMT_A16B16G16R16F,
        D3DFMT_A16B16G16R16,
        D3DFMT_A32B32G32R32F,
        D3DFMT_L8,
    ];
    for (i, format) in FORMATS.into_iter().enumerate() {
        ctx.result.globals.texture_formats[i] = check_format(ctx, 0x0049_8340, 0, format);
    }
    if s.gamma != ctx.result.globals.applied_gamma && s.gamma > 0.0 {
        ctx.result.globals.applied_gamma = s.gamma;
        ctx.result.globals.gamma_changed = true;
    }
    let info = RendererInfoInputs {
        description,
        driver,
        ps_version: caps.pixel_shader_version & 0xFFFF,
        vs_version: caps.vertex_shader_version & 0xFFFF,
        ps20_instructions: max_ps20,
        non_pow2,
        multisample_type: ctx.result.parameters.multi_sample_type as i32,
    };
    write_renderer_info(ctx, &info, renderer_present);
    // 00498D53..00498E77 tail copies.
    let g = &mut ctx.result.globals;
    g.sunlight_dimmer = if s.high_dynamic_range {
        s.sunlight_dimmer_hdr
    } else {
        s.sunlight_dimmer_blur
    };
    g.tree_dimmer = s.tree_dimmer;
    g.grass_dimmer = s.grass_dimmer;
    g.create_shader_package = s.create_shader_package;
    g.lods = s.lods;
    g.full_bright = s.full_bright_lighting;
    g.dynamic_window_reflections = s.dynamic_window_reflections;
    g.landscape_blend = s.landscape_blend;
    g.max_landscape_textures = s.max_landscape_textures;
    g.grids_to_load = s.grids_to_load;
    g.shadow_fade_time = s.shadow_fade_time;
    let filter = if g.render_path < 5 { 0 } else { 2 };
    g.shadow_filter = if filter >= s.shadow_filter {
        s.shadow_filter
    } else {
        filter
    };
    ctx.result.succeeded = renderer_present;
    if !renderer_present {
        ctx.result.failure_message = renderer_failure_message(&ctx.result.globals.failure_text);
    }
}

/// IDA: 004052F0: "Failed to initialize renderer.\n%s" with 00B34FC8 into a char[512] (sprintf), MessageBoxA(NULL, text,
/// "Oblivion", MB_OK), then ExitProcess(0). Completion: 100% of the text; the box and the exit are the caller's (recorded
/// substitution).
#[must_use]
pub fn renderer_failure_message(failure_text: &str) -> String {
    format!("Failed to initialize renderer.\n{failure_text}")
}

/// IDA: 00498A7E..00498D4B: the 24 fprintf lines of RendererInfo.txt ("w" text mode, so the CRT writes CR LF; see
/// `crt_text_mode_output`). MSVC printf prints "(null)" for a null %s. 00A3E128 "yes"/"no". Completion: 100%.
#[must_use]
pub fn renderer_info_text(
    g: &RendererGlobals,
    s: &RendererSettings,
    info: &RendererInfoInputs,
) -> Vec<u8> {
    let flag = |value: bool| if value { "yes" } else { "no" };
    let text = |value: Option<&'static str>| value.unwrap_or("(null)");
    let mut out = Vec::with_capacity(1024);
    out.extend_from_slice(b"Renderer Device Information:\n\t");
    out.extend_from_slice(&info.description);
    out.extend_from_slice(b"\n\t");
    out.extend_from_slice(&info.driver);
    out.push(b'\n');
    let lines = [
        format!(
            "\tRenderPath         \t\t: {}\n",
            render_path_name(g.render_path)
        ),
        format!("\tPSversion          \t\t: {:X}\n", info.ps_version),
        format!("\tVSversion          \t\t: {:X}\n", info.vs_version),
        format!(
            "\tVStarget           \t\t: {}\n",
            text(vertex_shader_target(g.render_path))
        ),
        format!(
            "\tPStarget           \t\t: {}\n",
            text(pixel_shader_target(g, false))
        ),
        format!(
            "\tPS2xtarget         \t\t: {}\n",
            text(pixel_shader_target(g, true))
        ),
        format!("\tmaxPS20inst        \t\t: {}\n", info.ps20_instructions),
        format!("\t3.0 Shaders        \t\t: {}\n", flag(g.shaders_30)),
        format!(
            "\tImage space effects\t\t: {}\n",
            flag(s.image_space_effects)
        ),
        format!("\tNonpowerof2textures\t\t: {}\n", flag(info.non_pow2)),
        format!("\tFP16ARGB blending  \t\t: {}\n", flag(g.fp16_blending)),
        format!("\tFP16ARGB filtering \t\t: {}\n", flag(g.fp16_filtering)),
        format!("\tHigh dynamic range \t\t: {}\n", flag(g.hdr)),
        format!("\tBloom lighting     \t\t: {}\n", flag(g.bloom)),
        format!("\tRefraction         \t\t: {}\n", flag(g.refraction)),
        format!("\t2.0 hair           \t\t: {}\n", flag(g.hair_20)),
        format!("\tSLI mode           \t\t: {}\n", flag(g.sli)),
        format!("\tWater shader       \t\t: {}\n", flag(s.water_shader)),
        format!("\tWater reflections  \t\t: {}\n", flag(s.water_reflections)),
        format!(
            "\tWater displacement \t\t: {}\n",
            flag(s.water_displacements)
        ),
        format!("\tWater high res     \t\t: {}\n", flag(s.water_hi_res)),
        format!("\tMultisample Type   \t\t: {}\n", info.multisample_type),
        format!("\tShader Package     \t\t: {}\n", shader_package(g)),
    ];
    for line in lines {
        out.extend_from_slice(line.as_bytes());
    }
    out
}

// IDA: failure texts copied into 00B34FC8 (00497B20 Renderer_SetFailureText and the inline copies at
// 004982DC/0049830A/004985F9/00498694/004986D0).
const NO_DEVICE_DESCRIPTION: &str = "No D3D Device description found.";

const BAD_ADAPTER: &str = "Bad Adapter Number or Adapter not found.";

const WINDOWED_NOT_SUPPORTED: &str = "Windowed mode not supported on this Adapter.";

const MODE_NOT_FOUND: &str = "Desired render mode not found on Adapter.";

const SHADER_VERSIONS: &str = "Pixel and Vertex Shader versions incorrect.  Requires a Geforce4 4400 or Radeon 8500 or better.";

const HARDWARE_TNL: &str = "Hardware T&L required but not supported by Adapter.";

/// IDA: 004980D0 Renderer_ValidateAdapterAndShaders. Completion: 100%.
pub(crate) fn validate_adapter_and_shaders(
    ctx: &mut RendererContext<'_>,
    match_fullscreen_size: bool,
) -> bool {
    let s = ctx.inputs.settings;
    let Some(system) = adapter_enumerator(ctx) else {
        ctx.result.globals.failure_text = NO_DEVICE_DESCRIPTION.to_owned();
        return false;
    };
    let g = &mut ctx.result.globals;
    let Some(adapter) = system.adapter(s.adapter) else {
        g.failure_text = BAD_ADAPTER.to_owned();
        return false;
    };
    let hal = adapter.hal_460.as_ref();
    if !s.full_screen
        && !hal.is_some_and(|hal| hal.caps_004.device_type != 0 && hal.renders_windowed_144)
    {
        g.failure_text = WINDOWED_NOT_SUPPORTED.to_owned();
        return false;
    }
    let wanted = dx9_frame_buffer_format(s.frame_buffer_format);
    let found = adapter.modes_450.iter().rev().any(|mode| {
        (!match_fullscreen_size
            || !s.full_screen
            || (g.width == mode.width && g.height == mode.height))
            && mode.bits_per_pixel as u32 >= 0x20
            && mode.format == wanted
    });
    if !found {
        g.failure_text = MODE_NOT_FOUND.to_owned();
        return false;
    }
    let usable_hal = hal.filter(|hal| hal.caps_004.device_type != 0);
    let Some(hal) = usable_hal.filter(|_| s.use_shaders) else {
        g.vertex_shader_version = 0;
        g.pixel_shader_version = 0;
        select_render_path(
            g,
            &RenderPathInputs {
                pixel_shader_version: 0,
                vertex_shader_version: 0,
                force_1x: s.force_1x_shaders,
                have_adapter: false,
                driver: Vec::new(),
                ps20_instructions: 0,
            },
        );
        return !s.use_shaders;
    };
    let caps = &hal.caps_004;
    let mut result = false;
    if caps.dev_caps & D3DDEVCAPS_HWTRANSFORMANDLIGHT != 0 {
        let vertex = caps.vertex_shader_version & 0xFFFF;
        let pixel = caps.pixel_shader_version & 0xFFFF;
        if vertex < g.vertex_shader_version || pixel < g.pixel_shader_version {
            g.failure_text = SHADER_VERSIONS.to_owned();
        } else {
            g.vertex_shader_version = vertex;
            g.pixel_shader_version = pixel;
            result = true;
        }
    } else {
        g.failure_text = HARDWARE_TNL.to_owned();
    }
    g.npatches = s.npatches > 1 && caps.dev_caps & D3DDEVCAPS_NPATCHES != 0;
    result
}

/// IDA: 007B45F0 Renderer_SelectRenderPath, ending in 007B4590 Renderer_UpdateShaderModelClass. Completion: 100%.
pub fn select_render_path(g: &mut RendererGlobals, input: &RenderPathInputs) {
    let ps_major = (input.pixel_shader_version >> 8) & 0xFF;
    let vs_major = (input.vertex_shader_version >> 8) & 0xFF;
    let mut path;
    if ps_major < 2 || vs_major < 2 {
        if ps_major != 0 && vs_major != 0 {
            path = 1;
            g.render_path = 1;
            g.hdr = false;
            g.shader_flags = 15;
        } else {
            path = 0;
            g.render_path = 0;
            g.hdr = false;
        }
        g.shader_flags &= !0x10;
    } else {
        path = 2;
        g.render_path = 2;
        g.shader_flags = 47;
        if input.have_adapter {
            let mut vendor = false;
            if strnicmp(&input.driver, b"ati", 3) == 0 {
                vendor = true;
                if input.ps20_instructions > 96 {
                    if g.render_path == 2 {
                        path = 6;
                        g.render_path = 6;
                    }
                    g.ps2x_path = 6;
                    if input.ps20_instructions > 255 {
                        g.shaders_30 = true;
                    }
                } else {
                    if g.render_path == 2 {
                        path = 4;
                        g.render_path = 4;
                    }
                    g.ps2x_path = 6;
                }
            } else if strnicmp(&input.driver, b"nv", 2) == 0 {
                vendor = true;
                if input.ps20_instructions > 96 {
                    if g.render_path == 2 {
                        path = 5;
                        g.render_path = 5;
                    }
                    g.ps2x_path = 5;
                    if input.ps20_instructions > 255 {
                        g.shaders_30 = true;
                    }
                } else {
                    if g.render_path == 2 {
                        path = 3;
                        g.render_path = 3;
                    }
                    g.ps2x_path = 5;
                }
            } else {
                path = 2;
                g.render_path = 2;
                g.ps2x_path = 2;
            }
            if vendor && path < 2 {
                g.shader_flags &= !0x10;
            }
        }
    }
    // 007B4590 Renderer_UpdateShaderModelClass.
    g.shader_model_class = match g.render_path {
        3 | 4 => 6,
        5..=7 => 8,
        _ => 2,
    };
    if input.force_1x && path != 0 {
        g.render_path = 1;
    }
}

/// IDA: 007B7070. Completion: 100%.
#[must_use]
pub const fn render_path_name(path: i32) -> &'static str {
    match path {
        0 => "BSSM_SV_NONE",
        1 => "BSSM_SV_1_X",
        2 => "BSSM_SV_2_0",
        3 => "BSSM_SV_2_A96",
        4 => "BSSM_SV_2_B96",
        5 => "BSSM_SV_2_A",
        6 => "BSSM_SV_2_B",
        7 => "BSSM_SV_3_0",
        _ => "???",
    }
}
