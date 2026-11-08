//! The BSImageSpaceShader family (05-D-4b), with the water image-space classes of slots 19 and 20.

// The loops are the originals': `index` also names the program and picks its slot.
#![allow(clippy::needless_range_loop)]

use std::sync::Arc;

use crate::bs::shader::{
    BsShaderContext, BsShaderObject, BsShaderSlot, PassSlot, ProgramSlot, assign_pass,
};
use crate::bs::shader_classes_common::{
    ProgramRecord, StageRef, acquire_pass_into, add_float4_constant, append_default_stage,
    append_temporary_stage, bind_programs, bs_shader_class, build_constant_map,
    create_image_space_slot, descriptor, load_pixel, load_vertex, pixel_target, program_name,
    programs, push_stage, render_states, stage_defaults, stage_filter, vertex_target,
};
use crate::bs::shader_programs::ShaderProgramDescriptor;
use crate::bs::shader_tables::{
    BLUR_P20_PIXEL, BLUR_P20_VERTEX, BLUR_PIXEL, BLUR_VERTEX, COPY_PIXEL, COPY_VERTEX,
    GETHIT_PIXEL, GETHIT_VERTEX, HDR_PIXEL, HDR_VERTEX, WATER_DISPLACEMENT_PIXEL,
    WATER_DISPLACEMENT_VERTEX, WATER_HEIGHT_MAP_PIXEL, WATER_HEIGHT_MAP_VERTEX,
};
use crate::gamebryo::shader_runtime::PassId;

const NO_DEPTH_OR_BLEND_7: [(u32, u32); 5] = [(7, 0), (0x0E, 0), (0x1B, 0), (0x0F, 0), (0xA8, 7)];
const NO_DEPTH_OR_BLEND_F: [(u32, u32); 5] = [(7, 0), (0x0E, 0), (0x1B, 0), (0x0F, 0), (0xA8, 0xF)];

// IDA: an image-space slot class: BSImageSpaceShader base (008026C0) and the
// vertex/pixel program NiPointer arrays at class-specific offsets.
#[derive(Default)]
pub struct ImageSpaceShader {
    pub base: BsShaderObject,
    pub program_index: u32, // +0x90 (Blur/P20)
    pub vertex_programs: Vec<ProgramSlot>,
    pub pixel_programs: Vec<ProgramSlot>,
    pub mode: u32,     // Copy +0xA4
    pub field_ac: u32, // Blur +0xAC
    pub field_bc: u32, // P20 +0xBC
    pub field_e0: u32, // Blur +0xE0 (NiPointer)
}
bs_shader_class!(ImageSpaceShader, base, BsShaderObject);

// IDA: the loop shape of 007B06C0/007EA510: per record a vertex program
// when the vertex path is non-null and a pixel program when the pixel path
// is non-null, both named by the same index. Completion: 100%.
#[allow(clippy::too_many_arguments)] // 007B06C0 record and name order.
fn load_paired_programs(
    context: &mut BsShaderContext,
    shader: &mut ImageSpaceShader,
    vertex: &[ProgramRecord],
    pixel: &[ProgramRecord],
    vertex_format: &str,
    vertex_profile: &str,
    pixel_format: &str,
    pixel_profile: &str,
) {
    for index in 0..vertex.len() {
        if !vertex[index].is_null() {
            shader.vertex_programs[index] = load_vertex(
                context,
                &vertex[index].owned(),
                vertex_profile,
                &program_name(vertex_format, index),
            );
        }
        if !pixel[index].is_null() {
            shader.pixel_programs[index] = load_pixel(
                context,
                &pixel[index].owned(),
                pixel_profile,
                &program_name(pixel_format, index),
            );
        }
    }
}

// IDA: N/A a helper, with no address of its own.
fn start_image_space(
    shader: &mut ImageSpaceShader,
    vtable: u32,
    name: &'static str,
    vertex: usize,
    pixel: usize,
) {
    shader.original_vtable = vtable;
    shader.class_name = name;
    shader.vertex_programs = programs(vertex);
    shader.pixel_programs = programs(pixel);
}

// IDA: N/A a helper, with no address of its own.
fn current_pass(context: &mut BsShaderContext, shader: &mut BsShaderObject) -> PassId {
    let mut slot = shader.current_pass;
    acquire_pass_into(context, &mut slot);
    shader.current_pass = slot;
    slot.expect("just acquired")
}

// IDA: BlurShader (0xE4, vtable A8CE1C) 007B0BD0: +0x94 three vertex and
// +0xA0 three pixel programs, +0xE0 NiPointer 0, +0xAC 0, +0x90 0.
// vt+0xB0 007B06C0 (ISBLUR1%03i.vso vs_1_1 / .pso ps_1_3), vt+0xB4 007B09A0:
// a pool pass into +0x70, four temporary stages (00801110(i, 3, 2)), the
// programs at +0x90, render states ZENABLE 0, ZWRITEENABLE 0,
// ALPHABLENDENABLE 0, ALPHATESTENABLE 0, COLORWRITEENABLE 7. Completion: 100%.
pub fn create_blur_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_image_space_slot::<ImageSpaceShader>(
        context,
        |shader, _| start_image_space(shader, 0x00A8_CE1C, "BlurShader", 3, 3),
        |shader, context| {
            load_paired_programs(
                context,
                shader,
                &BLUR_VERTEX,
                &BLUR_PIXEL,
                "ISBLUR1%03i.vso",
                "vs_1_1",
                "ISBLUR1%03i.pso",
                "ps_1_3",
            );
            let pass = current_pass(context, shader);
            for index in 0..4 {
                append_temporary_stage(context, pass, index, 3, 2);
            }
            let program = shader.program_index as usize;
            bind_programs(
                context,
                pass,
                &shader.vertex_programs[program],
                &shader.pixel_programs[program],
            );
            render_states(context, pass, &NO_DEPTH_OR_BLEND_7);
        },
        false,
    )
}

// IDA: BlurShader_P20 (0xD0, vtable A9224C) 007EAB50: +0x94 five vertex and
// +0xA8 five pixel programs, +0xBC 0, +0x90 0, byte +0x20 = 1. vt+0xB0
// 007EA510 (ISBLUR2%03i.vso vs_1_1 / .pso ps_2_0); vt+0xB4 007EA890: pool pass
// into +0x70; stage 0 = 00801110(0, 3, 2) then 00771640(1); stage 1 =
// 00771640(0) then 00801110(1, 3, 2) (its 00771640(2) wins); programs at
// +0x90; the five render states with COLORWRITEENABLE 7. Completion: 100%.
pub fn create_blur_p20_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_image_space_slot::<ImageSpaceShader>(
        context,
        |shader, _| {
            start_image_space(shader, 0x00A9_224C, "BlurShader_P20", 5, 5);
            shader.field_20 = 1;
        },
        |shader, context| {
            load_paired_programs(
                context,
                shader,
                &BLUR_P20_VERTEX,
                &BLUR_P20_PIXEL,
                "ISBLUR2%03i.vso",
                "vs_1_1",
                "ISBLUR2%03i.pso",
                "ps_2_0",
            );
            let pass = current_pass(context, shader);
            let mut stage = StageRef::new();
            let local = stage.acquire(context);
            stage_defaults(context, local, 0, 3, 2);
            stage_filter(context, local, 1);
            push_stage(context, pass, local);
            let local = stage.acquire(context);
            stage_filter(context, local, 0);
            stage_defaults(context, local, 1, 3, 2);
            push_stage(context, pass, local);
            let program = shader.program_index as usize;
            bind_programs(
                context,
                pass,
                &shader.vertex_programs[program],
                &shader.pixel_programs[program],
            );
            render_states(context, pass, &NO_DEPTH_OR_BLEND_7);
            stage.reset(context);
        },
        false,
    )
}

// IDA: CopyShader (0xB8, vtable A93E14) 00804790: +0x90 two vertex and
// +0x98 three pixel programs, byte +0x20 = 1, +0xA4 mode 0. vt+0xB0 008041B0:
// COPY%03i.vso with Renderer_GetVertexShaderTarget(), COPY%03i.pso with
// Renderer_GetPixelShaderTarget(0) (three records from render path 2, two
// below; null paths skipped); vt+0xB4 00804510: pool pass into +0x70, one
// stage 00801110(0, 3, 2) then 00771640(0), the render states with
// COLORWRITEENABLE 0xF, then vt+0xB8 008046C0 (+0xA4: 0/1 programs 0/0,
// 2 programs 1/1, 3/4 programs 0/2). Completion: 100%.
fn copy_bind_programs(context: &mut BsShaderContext, shader: &ImageSpaceShader) {
    let pass = shader.current_pass.expect("set by the pass setup");
    match shader.mode {
        0 | 1 => bind_programs(
            context,
            pass,
            &shader.vertex_programs[0],
            &shader.pixel_programs[0],
        ),
        2 => bind_programs(
            context,
            pass,
            &shader.vertex_programs[1],
            &shader.pixel_programs[1],
        ),
        3 | 4 => bind_programs(
            context,
            pass,
            &shader.vertex_programs[0],
            &shader.pixel_programs[2],
        ),
        _ => {}
    }
}

// IDA: 00804790 and its factory; see `copy_bind_programs`.
pub fn create_copy_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_image_space_slot::<ImageSpaceShader>(
        context,
        |shader, _| {
            start_image_space(shader, 0x00A9_3E14, "CopyShader", 2, 3);
            shader.field_20 = 1;
            shader.mode = 0;
        },
        |shader, context| {
            for index in 0..2 {
                if !COPY_VERTEX[index].is_null() {
                    shader.vertex_programs[index] = load_vertex(
                        context,
                        &COPY_VERTEX[index].owned(),
                        vertex_target(context),
                        &program_name("COPY%03i.vso", index),
                    );
                }
            }
            let pixel_count = if context.render_path >= 2 { 3 } else { 2 };
            for index in 0..pixel_count {
                if !COPY_PIXEL[index].is_null() {
                    shader.pixel_programs[index] = load_pixel(
                        context,
                        &COPY_PIXEL[index].owned(),
                        pixel_target(context),
                        &program_name("COPY%03i.pso", index),
                    );
                }
            }
            let pass = current_pass(context, shader);
            let mut stage = StageRef::new();
            let local = stage.acquire(context);
            stage_defaults(context, local, 0, 3, 2);
            stage_filter(context, local, 0);
            push_stage(context, pass, local);
            render_states(context, pass, &NO_DEPTH_OR_BLEND_F);
            copy_bind_programs(context, shader);
            stage.reset(context);
        },
        false,
    )
}

// IDA: HDRShader (0x124, vtable A8FFAC) 007C0330: +0x90 eight vertex and
// +0xB0 eight pixel programs, +0xD4 thirteen pass NiPointers, +0x118/+0x11C
// NiPointers 0, +0xD0 0, byte +0x20 = 1, the global 00B43328 NiPointer
// released (null), byte +0x120 = g_rendererFP16Filtering &&
// g_rendererAnisotropicMinFilter.
#[derive(Default)]
pub struct HdrShader {
    pub image: ImageSpaceShader,
    pub hdr_passes: [PassSlot; 13],
    pub field_d0: u32,
    pub field_118: u32,
    pub field_11c: u32,
    pub fp16_anisotropic: u8, // +0x120
}
bs_shader_class!(HdrShader, image, ImageSpaceShader);

// IDA: 007BED40 vt+0xB0: HDR%03i.vso (vs_1_1) and HDR%03i.pso (ps_2_0) per
// record; under FP16 filtering record 0's TEXTAP becomes "4" (the word store
// at 007BEF77) and record 4 loses SWFILTER (its name set null, ending the
// list). Completion: 100%.
fn hdr_load_programs(context: &mut BsShaderContext, shader: &mut HdrShader) {
    for index in 0..8 {
        if !HDR_VERTEX[index].is_null() {
            shader.vertex_programs[index] = load_vertex(
                context,
                &HDR_VERTEX[index].owned(),
                "vs_1_1",
                &program_name("HDR%03i.vso", index),
            );
        }
        if HDR_PIXEL[index].is_null() {
            continue;
        }
        let mut pixel = HDR_PIXEL[index].owned();
        if index == 0 {
            if context.fp16_filtering {
                pixel.macros[0].definition = Some("4".to_owned());
            }
        } else if index == 4 && context.fp16_filtering {
            pixel.macros[2].name.clear();
        }
        shader.pixel_programs[index] = load_pixel(
            context,
            &pixel,
            "ps_2_0",
            &program_name("HDR%03i.pso", index),
        );
    }
}

// IDA: 007BFD90 vt+0xB8 the program bindings (vertex +0x58, pixel +0x44).
fn hdr_bind_programs(context: &mut BsShaderContext, shader: &HdrShader) {
    let mut bind = |pass: usize, vertex: usize, pixel: usize| {
        bind_programs(
            context,
            shader.hdr_passes[pass].expect("a built HDR pass"),
            &shader.vertex_programs[vertex],
            &shader.pixel_programs[pixel],
        );
    };
    bind(0, 3, 3);
    bind(1, 2, 2);
    bind(2, 2, 2);
    bind(3, 0, 0);
    if shader.hdr_passes[7].is_some() {
        bind(7, 3, 3); // 007AECB0 / 007AEC60
    }
    bind(8, 3, 3);
    bind(9, 4, 4);
    bind(10, 5, 5);
    bind(11, 6, 6);
    bind(12, 7, 7);
}

// IDA: 007BF1B0 vt+0xB4 the thirteen HDR passes. Completion: 100% of the order.
fn hdr_setup_passes(context: &mut BsShaderContext, shader: &mut HdrShader) {
    let mut stage = StageRef::new();
    let stage_with_filter = |context: &mut BsShaderContext,
                             shader: &HdrShader,
                             stage: &mut StageRef,
                             pass: usize,
                             index: u32,
                             mode: u32| {
        let local = stage.acquire(context);
        stage_defaults(context, local, index, 3, 2);
        stage_filter(context, local, mode);
        push_stage(
            context,
            shader.hdr_passes[pass].expect("just acquired"),
            local,
        );
    };
    let fp16 = u32::from(context.fp16_filtering);
    acquire_pass_into(context, &mut shader.hdr_passes[0]);
    stage_with_filter(context, shader, &mut stage, 0, 0, 0);
    acquire_pass_into(context, &mut shader.hdr_passes[1]);
    stage_with_filter(context, shader, &mut stage, 1, 0, 0);
    acquire_pass_into(context, &mut shader.hdr_passes[2]);
    stage_with_filter(context, shader, &mut stage, 2, 0, 0);
    acquire_pass_into(context, &mut shader.hdr_passes[3]);
    stage_with_filter(context, shader, &mut stage, 3, 0, fp16);
    for pass in 4..7 {
        assign_pass(&mut context.pools, &mut shader.hdr_passes[pass], None);
    }
    if shader.fp16_anisotropic != 0 {
        // 007BF5C3..007BF66F explicit MAG 2, MIN 3, MIP 0.
        acquire_pass_into(context, &mut shader.hdr_passes[7]);
        let local = stage.acquire(context);
        stage_defaults(context, local, 0, 3, 2);
        let states = context.pools.stage_states_mut(local);
        states.set_sampler_state(5, 2, false, false);
        states.set_sampler_state(6, 3, false, false);
        states.set_sampler_state(7, 0, false, false);
        push_stage(context, shader.hdr_passes[7].expect("just acquired"), local);
    } else {
        assign_pass(&mut context.pools, &mut shader.hdr_passes[7], None);
    }
    acquire_pass_into(context, &mut shader.hdr_passes[8]);
    stage_with_filter(context, shader, &mut stage, 8, 0, 0);
    acquire_pass_into(context, &mut shader.hdr_passes[9]);
    stage_with_filter(context, shader, &mut stage, 9, 0, fp16);
    stage_with_filter(context, shader, &mut stage, 9, 1, 0);
    stage_with_filter(context, shader, &mut stage, 9, 2, 0);
    acquire_pass_into(context, &mut shader.hdr_passes[10]);
    stage_with_filter(context, shader, &mut stage, 10, 0, 0);
    acquire_pass_into(context, &mut shader.hdr_passes[11]);
    stage_with_filter(context, shader, &mut stage, 11, 0, 0);
    stage_with_filter(context, shader, &mut stage, 11, 1, 0);
    acquire_pass_into(context, &mut shader.hdr_passes[12]);
    stage_with_filter(context, shader, &mut stage, 12, 0, 0);
    for pass in shader.hdr_passes.into_iter().flatten() {
        render_states(context, pass, &NO_DEPTH_OR_BLEND_F);
    }
    hdr_bind_programs(context, shader);
    stage.reset(context);
}

// IDA: 007C0B80 slot 8 (g_rendererHDR only).
pub fn create_hdr_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_image_space_slot::<HdrShader>(
        context,
        |shader, context| {
            start_image_space(shader, 0x00A8_FFAC, "HDRShader", 8, 8);
            shader.field_20 = 1;
            context.globals.hdr_b43328_cleared = true;
            shader.fp16_anisotropic =
                u8::from(context.fp16_filtering && context.anisotropic_min_filter);
        },
        |shader, context| {
            hdr_load_programs(context, shader);
            hdr_setup_passes(context, shader);
        },
        false,
    )
}

// IDA: GethitShader (0xB4, vtable A92394) 007EC7D0: +0x94 three vertex and
// +0xA0 three pixel programs, +0xAC/+0xB0 pass NiPointers 0, +0x90 0, byte
// +0x20 = 1.
#[derive(Default)]
pub struct GethitShader {
    pub image: ImageSpaceShader,
    pub pass_ac: PassSlot,
    pub pass_b0: PassSlot,
}
bs_shader_class!(GethitShader, image, ImageSpaceShader);

// IDA: 007EBEE0 vt+0xB4: +0x70 with stage 0 (filter mode 0) and stage 1
// (mode 1), programs 2/2; +0xAC with stage 0 (mode 1), programs 1/1; +0xB0
// with stage 0 (mode 1), programs 0/0; each pass ZENABLE 0, ZWRITEENABLE 0,
// ALPHABLENDENABLE 0, ALPHATESTENABLE 0, COLORWRITEENABLE 7. Completion: 100%.
fn gethit_setup_passes(context: &mut BsShaderContext, shader: &mut GethitShader) {
    let mut stage = StageRef::new();
    let add_stage = |context: &mut BsShaderContext,
                     stage: &mut StageRef,
                     pass: PassId,
                     index: u32,
                     mode: u32| {
        let local = stage.acquire(context);
        stage_defaults(context, local, index, 3, 2);
        stage_filter(context, local, mode);
        push_stage(context, pass, local);
    };
    let finish =
        |context: &mut BsShaderContext, shader: &GethitShader, pass: PassId, program: usize| {
            bind_programs(
                context,
                pass,
                &shader.vertex_programs[program],
                &shader.pixel_programs[program],
            );
            render_states(context, pass, &NO_DEPTH_OR_BLEND_7);
        };
    let pass = current_pass(context, shader);
    add_stage(context, &mut stage, pass, 0, 0);
    add_stage(context, &mut stage, pass, 1, 1);
    finish(context, shader, pass, 2);
    acquire_pass_into(context, &mut shader.pass_ac);
    let pass = shader.pass_ac.expect("just acquired");
    add_stage(context, &mut stage, pass, 0, 1);
    finish(context, shader, pass, 1);
    acquire_pass_into(context, &mut shader.pass_b0);
    let pass = shader.pass_b0.expect("just acquired");
    add_stage(context, &mut stage, pass, 0, 1);
    finish(context, shader, pass, 0);
    stage.reset(context);
}

// IDA: 007EC950 slot 11 (render path >= 2): vt+0xB0 007EBC80 (ISHIT2%03i.vso
// vs_1_1 / .pso ps_2_0), vt+0xB4 007EBEE0.
pub fn create_gethit_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_image_space_slot::<GethitShader>(
        context,
        |shader, _| {
            start_image_space(shader, 0x00A9_2394, "GethitShader", 3, 3);
            shader.field_20 = 1;
        },
        |shader, context| {
            load_paired_programs(
                context,
                shader,
                &GETHIT_VERTEX,
                &GETHIT_PIXEL,
                "ISHIT2%03i.vso",
                "vs_1_1",
                "ISHIT2%03i.pso",
                "ps_2_0",
            );
            gethit_setup_passes(context, shader);
        },
        false,
    )
}

// IDA: the single-program image-space classes: Nighteye/Refraction keep the
// programs at +0x90/+0x94 and the pass at +0x9C (vt+0xB8 007F56E0 binds
// them); Map/MenuBG keep the pass at +0x94 and the programs at +0x98/+0x9C
// (vt+0xB8 007AF510).
#[derive(Default)]
pub struct SingleProgramShader {
    pub image: ImageSpaceShader,
    pub class_pass: PassSlot,
    pub field_98: u32,        // Nighteye/Refraction +0x98
    pub field_a0: u32,        // Refraction +0xA0 (NiPointer)
    pub map_floats: [f32; 4], // Map +0xB0..+0xBC
    pub map_c0: u32,          // Map +0xC0 (NiPointer)
    pub menu_b0: u8,          // MenuBG byte +0xB0
    pub menu_b4: u32,         // MenuBG +0xB4 (NiPointer)
}
bs_shader_class!(SingleProgramShader, image, ImageSpaceShader);

// IDA: N/A a helper, with no address of its own.
fn bind_single_programs(context: &mut BsShaderContext, shader: &SingleProgramShader) {
    let pass = shader.class_pass.expect("set by the pass setup");
    bind_programs(
        context,
        pass,
        &shader.vertex_programs[0],
        &shader.pixel_programs[0],
    );
}

// IDA: one program pair with an explicit source path and macro list.
#[allow(clippy::too_many_arguments)] // Vertex, then pixel, as the loaders run.
fn load_single_programs(
    context: &mut BsShaderContext,
    shader: &mut SingleProgramShader,
    vertex: &ShaderProgramDescriptor,
    vertex_profile: &str,
    vertex_name: &str,
    pixel: &ShaderProgramDescriptor,
    pixel_profile: &str,
    pixel_name: &str,
) {
    shader.vertex_programs[0] = load_vertex(context, vertex, vertex_profile, vertex_name);
    shader.pixel_programs[0] = load_pixel(context, pixel, pixel_profile, pixel_name);
}

// IDA: the image-space vertex record shared by 007F5350 and 007AF0A0. Like the
// other program records in this file, the original writes it to the stack
// before each load, so every loader builds its own copy. Completion: 100%.
fn image_space_base_vertex() -> ShaderProgramDescriptor {
    descriptor("imagespace\\1x\\v\\base.v.hlsl", &[])
}

// IDA: N/A a helper, with no address of its own.
fn start_single(shader: &mut SingleProgramShader, vtable: u32, name: &'static str) {
    start_image_space(shader, vtable, name, 1, 1);
    shader.field_20 = 1;
}

// IDA: N/A a helper, with no address of its own.
// The single pass shape: stages by (index, filter mode), the render states with COLORWRITEENABLE 0xF, then vt+0xB8.
fn single_pass(
    context: &mut BsShaderContext,
    shader: &mut SingleProgramShader,
    stages: &[(u32, u32)],
) {
    acquire_pass_into(context, &mut shader.class_pass);
    let mut stage = StageRef::new();
    let pass = shader.class_pass.expect("just acquired");
    for &(index, mode) in stages {
        let local = stage.acquire(context);
        stage_defaults(context, local, index, 3, 2);
        stage_filter(context, local, mode);
        push_stage(context, pass, local);
    }
    render_states(context, pass, &NO_DEPTH_OR_BLEND_F);
    bind_single_programs(context, shader);
    stage.reset(context);
}

// IDA: NighteyeShader (0xC0, vtable A92EB4) 007F5780; vt+0xB0 007F5350
// (NIGHTEYE000.vso vs_1_1, NIGHTEYE000.pso ps_1_3); vt+0xB4 007F54F0: pass
// +0x9C, stage 0 (mode 0), the render states (guarded by a non-null pass)
// with COLORWRITEENABLE 0xF, vt+0xB8. Completion: 100%.
pub fn create_nighteye_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_image_space_slot::<SingleProgramShader>(
        context,
        |shader, _| start_single(shader, 0x00A9_2EB4, "NighteyeShader"),
        |shader, context| {
            let pixel = descriptor("imagespace\\1x\\p\\nighteye.p.hlsl", &[]);
            load_single_programs(
                context,
                shader,
                &image_space_base_vertex(),
                "vs_1_1",
                "NIGHTEYE000.vso",
                &pixel,
                "ps_1_3",
                "NIGHTEYE000.pso",
            );
            single_pass(context, shader, &[(0, 0)]);
        },
        false,
    )
}

// IDA: RefractionShader (0xC4, vtable A9336C) 00800B70; vt+0xB0 00800750
// (REFRACT2000.vso vs_1_1 with TEX2, REFRACT2000.pso ps_2_0); vt+0xB4
// 00800900: pass +0x9C, stages 0 and 1 (mode 0), the render states with
// COLORWRITEENABLE 0xF, vt+0xB8 007F56E0. Completion: 100%.
pub fn create_refraction_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_image_space_slot::<SingleProgramShader>(
        context,
        |shader, _| start_single(shader, 0x00A9_336C, "RefractionShader"),
        |shader, context| {
            let vertex = descriptor("imagespace\\1x\\v\\base.v.hlsl", &[("TEX2", Some(""))]);
            let pixel = descriptor("imagespace\\2x\\p\\refraction_P20.p.hlsl", &[]);
            load_single_programs(
                context,
                shader,
                &vertex,
                "vs_1_1",
                "REFRACT2000.vso",
                &pixel,
                "ps_2_0",
                "REFRACT2000.pso",
            );
            single_pass(context, shader, &[(0, 0), (1, 0)]);
        },
        false,
    )
}

// IDA: MapShader (0xC4, vtable A8CCDC) 007AF820; vt+0xB0 007AF0A0 (MAP000.vso
// vs_1_1, MAP000.pso ps_2_0); vt+0xB4 007AF240: pass +0x94, stages 0..2
// (mode 0), the render states with COLORWRITEENABLE 0xF, vt+0xB8 007AF510.
// Completion: 100%.
pub fn create_map_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_image_space_slot::<SingleProgramShader>(
        context,
        |shader, _| start_single(shader, 0x00A8_CCDC, "MapShader"),
        |shader, context| {
            let pixel = descriptor("imagespace\\2x\\p\\map_P20.p.hlsl", &[]);
            load_single_programs(
                context,
                shader,
                &image_space_base_vertex(),
                "vs_1_1",
                "MAP000.vso",
                &pixel,
                "ps_2_0",
                "MAP000.pso",
            );
            single_pass(context, shader, &[(0, 0), (1, 0), (2, 0)]);
        },
        false,
    )
}

// IDA: MenuBGShader (0xB8, vtable A8CFD4) 007B1C70: +0x94 pass, +0x98/+0x9C
// programs, byte +0xB0 0, +0xB4 0, +0x90 0, byte +0x20 1; vt+0xB0 007B14C0
// (MENUBG000.vso/.pso with vs_1_1/ps_1_3, or vs_2_0/ps_2_0 under
// g_rendererHDR); vt+0xB4 007B1670: pass +0x94, stage 0 (mode 1), stage 1
// (mode 0), the render states with COLORWRITEENABLE 0xF, vt+0xB8 007AF510.
// Completion: 100%.
pub fn create_menu_bg_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_image_space_slot::<SingleProgramShader>(
        context,
        |shader, _| start_single(shader, 0x00A8_CFD4, "MenuBGShader"),
        |shader, context| {
            let vertex_profile = if context.hdr { "vs_2_0" } else { "vs_1_1" };
            let pixel_profile = if context.hdr { "ps_2_0" } else { "ps_1_3" };
            let vertex = descriptor("imagespace\\1x\\v\\menuBG.v.hlsl", &[]);
            let pixel = descriptor("imagespace\\1x\\p\\menuBG.p.hlsl", &[]);
            load_single_programs(
                context,
                shader,
                &vertex,
                vertex_profile,
                "MENUBG000.vso",
                &pixel,
                pixel_profile,
                "MENUBG000.pso",
            );
            single_pass(context, shader, &[(0, 1), (1, 0)]);
        },
        false,
    )
}

// IDA: WaterShaderHeightMap (0x10C, vtable A91B74) 007E0ED0: base 008026C0,
// +0xB0 vertex and +0xB4 seven pixel program NiPointers, +0xD0..+0xF4 = 0
// (+0xD4 the pass; +0xD8..+0xE4/+0xEC NiPointers released), byte +0x20 = 1,
// +0x100 Time and +0x104 fPassNum 0.0, +0x90/+0xA0 the texRatio0/1 vectors
// (not initialized), the globals in BSShaderGlobals.
#[derive(Default)]
pub struct WaterHeightMapShader {
    pub image: ImageSpaceShader,
    pub field_d0: u32,
    pub water_pass: PassSlot, // +0xD4
    pub fields_d8: [u32; 8],  // +0xD8..+0xF4
    pub time: f32,            // +0x100
    pub pass_number: f32,     // +0x104
}
bs_shader_class!(WaterHeightMapShader, image, ImageSpaceShader);

// IDA: 007E0ED0 the constructor tail after the program arrays. Completion: 100%.
fn construct_water_height_map(context: &mut BsShaderContext, shader: &mut WaterHeightMapShader) {
    start_image_space(shader, 0x00A9_1B74, "WaterShaderHeightMap", 1, 7);
    shader.field_20 = 1;
    let globals = &mut context.globals;
    globals.water_height_map_b45fe0 = Some(19);
    shader.field_d0 = 0;
    globals.water_b45fc4 = 5.0;
    globals.water_b45fc0 = 90.0;
    globals.water_b45fd8 = 0.5;
    globals.water_b45fd4 = 1.0;
    shader.time = 0.0;
    shader.pass_number = 0.0;
    if globals.water_hi_res_b45fd0 {
        globals.water_resolution_b45fc8 = 256;
        globals.water_passes_b45fcc = 8;
    } else {
        globals.water_resolution_b45fc8 = 128;
        globals.water_passes_b45fcc = 7;
    }
    globals.water_b45fd1 = 1;
    let inverse = 1.0 / globals.water_resolution_b45fc8 as f32;
    globals.water_resolution_b45fe4 = [inverse; 4];
}

// IDA: 007DFBD0: WATERHMAP.vso (imagespace\1x\v\base.v.hlsl, no macros,
// vs_1_1) into +0xB0, then WATERHMAP%03i.pso 0..6 (ps_1_3; ps_2_0 from render
// path 2) into +0xB4. Completion: 100% with the generated tables.
fn water_height_map_load_programs(
    context: &mut BsShaderContext,
    shader: &mut WaterHeightMapShader,
) {
    shader.vertex_programs[0] = load_vertex(
        context,
        &WATER_HEIGHT_MAP_VERTEX[0].owned(),
        "vs_1_1",
        "WATERHMAP.vso",
    );
    let pixel_profile = if context.render_path >= 2 {
        "ps_2_0"
    } else {
        "ps_1_3"
    };
    for index in 0..7 {
        if WATER_HEIGHT_MAP_PIXEL[index].is_null() {
            continue;
        }
        shader.pixel_programs[index] = load_pixel(
            context,
            &WATER_HEIGHT_MAP_PIXEL[index].owned(),
            pixel_profile,
            &program_name("WATERHMAP%03i.pso", index),
        );
    }
}

// IDA: 007DFEE0: when +0xD4 is null, a pool pass and stages 0..2
// (00801110(i, 3, 2) then 00771640(0)); then ZENABLE 0, ZWRITEENABLE 0,
// ALPHABLENDENABLE 0, ALPHATESTENABLE 0, COLORWRITEENABLE 0xF; vertex
// program +0xB0, pixel program +0xB4. Completion: 100%.
fn water_height_map_setup_passes(context: &mut BsShaderContext, shader: &mut WaterHeightMapShader) {
    if shader.water_pass.is_none() {
        acquire_pass_into(context, &mut shader.water_pass);
        let mut stage = StageRef::new();
        let pass = shader.water_pass.expect("just acquired");
        for index in 0..3 {
            let local = stage.acquire(context);
            stage_defaults(context, local, index, 3, 2);
            stage_filter(context, local, 0);
            push_stage(context, pass, local);
        }
        stage.reset(context);
    }
    let pass = shader.water_pass.expect("set above");
    render_states(context, pass, &NO_DEPTH_OR_BLEND_F);
    bind_programs(
        context,
        pass,
        &shader.vertex_programs[0],
        &shader.pixel_programs[0],
    );
}

// IDA: 007DFA30 vt+0x88: +0x2C Time (c0, +0x100), fPassNum (c1, +0x104),
// fResolution (c2, 00B45FE4); +0x30 texRatio0 (c6, +0x90), texRatio1 (c7,
// +0xA0). Completion: 100%.
fn water_height_map_constant_maps(
    context: &mut BsShaderContext,
    shader: &mut WaterHeightMapShader,
) {
    shader.pixel_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.pixel_constant_map,
        true,
        |map| {
            add_float4_constant(map, "Time", 0, "this+0x100");
            add_float4_constant(map, "fPassNum", 1, "this+0x104");
            add_float4_constant(map, "fResolution", 2, "00B45FE4");
        },
    );
    shader.vertex_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.vertex_constant_map,
        false,
        |map| {
            add_float4_constant(map, "texRatio0", 6, "this+0x90");
            add_float4_constant(map, "texRatio1", 7, "this+0xA0");
        },
    );
}

// IDA: 007E1430 slot 19: the image-space declaration, FormHeap(0x10C)
// 007E0ED0, vt+0x84, 007DFBD0, 007DFEE0, vt+0x88 007DFA30, vt+0x54, +4.
pub fn create_water_height_map_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_image_space_slot::<WaterHeightMapShader>(
        context,
        |shader, context| construct_water_height_map(context, shader),
        |shader, context| {
            water_height_map_load_programs(context, shader);
            water_height_map_setup_passes(context, shader);
            water_height_map_constant_maps(context, shader);
        },
        true,
    )
}

// IDA: WaterShaderDisplacement (0x128, vtable A9193C) 007DE240: base
// 008026C0, +0xB4 eight vertex and +0xD4 eight pixel program NiPointers,
// +0xF8..+0x10C = 0 (+0xF8 the pass), byte +0x20 = 1, +0xF4 = 0, +0x114 0.0,
// +0x118 0.1, +0x11C 0.1, +0x120 1.0, +0x124 0.05, +0x90 = 0; +0x110 Time
// and the +0x94/+0xA4 texRatio vectors are not initialized.
#[derive(Default)]
pub struct WaterDisplacementShader {
    pub image: ImageSpaceShader,
    pub field_90: u32,
    pub field_f4: u32,
    pub water_pass: PassSlot, // +0xF8
    pub fields_fc: [u32; 5],  // +0xFC..+0x10C
    pub floats_114: [f32; 5], // +0x114..+0x124
}
bs_shader_class!(WaterDisplacementShader, image, ImageSpaceShader);

// IDA: 007DE240 the constructor tail. Completion: 100%.
fn construct_water_displacement(
    context: &mut BsShaderContext,
    shader: &mut WaterDisplacementShader,
) {
    start_image_space(shader, 0x00A9_193C, "WaterShaderDisplacement", 8, 8);
    shader.field_20 = 1;
    let globals = &mut context.globals;
    globals.water_displacement_b45f54 = Some(20);
    shader.field_f4 = 0;
    globals.water_b3ff00 = 0;
    shader.floats_114 = [0.0, 0.1, 0.1, 1.0, 0.05];
    globals.water_blend_b45f44 = 0.0;
    globals.water_rain_b45f58 = [0.1, 0.6, 0.985, 0.01];
    globals.water_wading_b45f68 = [0.4, 0.6, 0.985, 0.05];
    shader.field_90 = 0;
}

// IDA: 007DDD90: when +0xF8 is null, a pool pass and stages 0 and 1
// (00801110(i, 3, 2), no filter call); then the five render states with
// COLORWRITEENABLE 0xF and ALPHAFUNC (0x16) 1; vertex +0xB4, pixel +0xD4.
// Completion: 100%.
fn water_displacement_setup_passes(
    context: &mut BsShaderContext,
    shader: &mut WaterDisplacementShader,
) {
    if shader.water_pass.is_none() {
        acquire_pass_into(context, &mut shader.water_pass);
        let mut stage = StageRef::new();
        let pass = shader.water_pass.expect("just acquired");
        append_default_stage(context, pass, &mut stage, 0, 3, 2);
        append_default_stage(context, pass, &mut stage, 1, 3, 2);
        stage.reset(context);
    }
    let pass = shader.water_pass.expect("set above");
    render_states(
        context,
        pass,
        &[
            (7, 0),
            (0x0E, 0),
            (0x1B, 0),
            (0x0F, 0),
            (0xA8, 0xF),
            (0x16, 1),
        ],
    );
    bind_programs(
        context,
        pass,
        &shader.vertex_programs[0],
        &shader.pixel_programs[0],
    );
}

// IDA: 007DD6B0 vt+0x88: +0x2C Time (c0, +0x110), BlendAmount (c1,
// 00B45F44), TextureOffset (c2, 00B45FAC), fDamp (c3, 00B45F40), RainVars
// (c4, 00B45F58), WadingVars (c5, 00B45F68); +0x30 texRatio0 (c6, +0x94),
// texRatio1 (c7, +0xA4), TransMatrixRowOne (c8, 00B45F78), TransMatrixRowTwo
// (c9, 00B45F88). Completion: 100%.
fn water_displacement_constant_maps(
    context: &mut BsShaderContext,
    shader: &mut WaterDisplacementShader,
) {
    shader.pixel_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.pixel_constant_map,
        true,
        |map| {
            add_float4_constant(map, "Time", 0, "this+0x110");
            add_float4_constant(map, "BlendAmount", 1, "00B45F44");
            add_float4_constant(map, "TextureOffset", 2, "00B45FAC");
            add_float4_constant(map, "fDamp", 3, "00B45F40");
            add_float4_constant(map, "RainVars", 4, "00B45F58");
            add_float4_constant(map, "WadingVars", 5, "00B45F68");
        },
    );
    shader.vertex_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.vertex_constant_map,
        false,
        |map| {
            add_float4_constant(map, "texRatio0", 6, "this+0x94");
            add_float4_constant(map, "texRatio1", 7, "this+0xA4");
            add_float4_constant(map, "TransMatrixRowOne", 8, "00B45F78");
            add_float4_constant(map, "TransMatrixRowTwo", 9, "00B45F88");
        },
    );
}

// IDA: 007DE710 slot 20: the image-space declaration, FormHeap(0x128)
// 007DE240, vt+0x84, 007DD920 (WATERDISPLACE%03i.vso/.pso 0..7: vs_1_1 and
// ps_1_3, or vs_2_0 and ps_2_0 from render path 2), 007DDD90, vt+0x88
// 007DD6B0, vt+0x54, +4.
pub fn create_water_displacement_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_image_space_slot::<WaterDisplacementShader>(
        context,
        |shader, context| construct_water_displacement(context, shader),
        |shader, context| {
            let path_2 = context.render_path >= 2;
            load_paired_programs(
                context,
                shader,
                &WATER_DISPLACEMENT_VERTEX,
                &WATER_DISPLACEMENT_PIXEL,
                "WATERDISPLACE%03i.vso",
                if path_2 { "vs_2_0" } else { "vs_1_1" },
                "WATERDISPLACE%03i.pso",
                if path_2 { "ps_2_0" } else { "ps_1_3" },
            );
            water_displacement_setup_passes(context, shader);
            water_displacement_constant_maps(context, shader);
        },
        true,
    )
}
