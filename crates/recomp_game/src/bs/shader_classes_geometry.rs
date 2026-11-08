//! Geometry slot classes (05-D-4c): tall grass, distant LOD, water, sky, decals, SpeedTree leaf and frond, particles,
//! bolts, precipitation and the debug shader.

// The loops are the originals': `index` also names the program and picks its slot.
#![allow(clippy::needless_range_loop)]

use std::sync::Arc;

use bevy_ecs::prelude::{Entity, World};
use recomp_core::reader::StreamError;
use recomp_core::x87::stored_float;

use crate::bs::shader::{
    BsShaderContext, BsShaderObject, BsShaderSlot, PassSlot, ProgramSlot, assign_pass,
    construct_bs_shader_base, initialize_bs_shader,
};
use crate::bs::shader_classes_common::{
    ProgramRecord, StageRef, acquire_pass_into, add_constant, add_float4_constant,
    add_matrix_constant, add_predefined, add_tree_constants, append_default_stage,
    append_filtered_stage, bind_programs, bs_shader_class, build_constant_map,
    create_bs_shader_slot, declare_position_texture, descriptor, finish_slot, gcvt_12, load_pixel,
    load_vertex, make_declaration, pixel_target, program_name, programs, render_states,
    set_entries, set_entry, vertex_target,
};
use crate::bs::shader_tables::{
    DISTANT_LOD_PIXEL, DISTANT_LOD_VERTEX, TALL_GRASS_PIXEL1X, TALL_GRASS_PIXEL2X,
    TALL_GRASS_VERTEX1X, TALL_GRASS_VERTEX2X, WATER_PIXEL, WATER_VERTEX,
};
use crate::gamebryo::av_object::NiAvProperties;
use crate::gamebryo::registration::NiProcessDefaults;
use crate::gamebryo::renderer::RendererState;
use crate::gamebryo::scene_state::refresh_property_state;
use crate::gamebryo::screen_elements::{
    ScreenElementsQuad, construct_screen_elements_data, insert_screen_polygon,
    set_screen_rectangle, set_screen_rectangle_uv, spawn_screen_elements, update_screen_bound,
    update_screen_elements_world,
};
use crate::gamebryo::shader_runtime::{
    PassId, ShaderConstantMapEntry, add_predefined_constant_entry, add_shader_constant_entry,
    create_shader_declaration, find_shader_constant_entry, set_declaration_entry_by_parameter,
};

// IDA: TallGrassShader (0x194, vtable A9202C; RTTI TallGrassShader > BSShader).
// 007E8BB0 constructor: base 00802540, +0x7C three passes, +0x94 40 vertex
// and +0x134 9 pixel program NiPointers (one contiguous host array: the
// vertex loop of 007E6B10 writes programs 40..43 into the pixel slots and
// pass C reads them), +0x164..+0x190 twelve floats 0.0, +0x24 the
// declaration, +0x88 0.0, 00B460D0..00B460DC = src__25 (0, 0, 0, 1), +0x160
// 80 (render path < 2) or 228, +0x158 FormHeap(16 * +0x160) (not
// initialized), +0x15C 0.
#[derive(Default)]
pub struct TallGrassShader {
    pub base: BsShaderObject,
    pub passes_7c: [PassSlot; 3],
    pub field_88: f32,
    pub programs: Vec<ProgramSlot>,
    pub instance_buffer: Vec<[f32; 4]>, // +0x158
    pub field_15c: u32,
    pub instance_capacity: u16, // +0x160
    pub floats_164: [f32; 12],
}
bs_shader_class!(TallGrassShader, base, BsShaderObject);

// IDA: 007E6B10 vt+0xA8: render path 1 loads GRASS1%03i.vso 0..19 (vs_1_1)
// and GRASS1%03i.pso 0..1 (ps_1_3, pixel slots 0..1); otherwise
// GRASS2%03i.vso 20..43 (vs_2_0, program slots 20..43) and GRASS2%03i.pso
// 2..8 (ps_2_0, program slots 42..48; 6..8 only when 00404F00(0) >= 5).
// Completion: 100% with the generated tables.
fn tallgrass_load_programs(context: &mut BsShaderContext, shader: &mut TallGrassShader) {
    if context.render_path == 1 {
        for index in 0..20 {
            shader.programs[index] = load_vertex(
                context,
                &TALL_GRASS_VERTEX1X[index].owned(),
                "vs_1_1",
                &program_name("GRASS1%03i.vso", index),
            );
        }
        for index in 0..2 {
            shader.programs[40 + index] = load_pixel(
                context,
                &TALL_GRASS_PIXEL1X[index].owned(),
                "ps_1_3",
                &program_name("GRASS1%03i.pso", index),
            );
        }
        return;
    }
    for index in 20..44 {
        shader.programs[index] = load_vertex(
            context,
            &TALL_GRASS_VERTEX2X[index - 20].owned(),
            "vs_2_0",
            &program_name("GRASS2%03i.vso", index),
        );
    }
    for index in 2..9 {
        if index >= 6 && context.render_path < 5 {
            continue; // 007E7E96..007E7EA7
        }
        shader.programs[40 + index] = load_pixel(
            context,
            &TALL_GRASS_PIXEL2X[index - 2].owned(),
            "ps_2_0",
            &program_name("GRASS2%03i.pso", index),
        );
    }
}

// IDA: 007E7F70 (from 007E9A20 vt+0xAC) the TallGrass passes. 100% of the
// call order and constants.
fn tallgrass_setup_passes(context: &mut BsShaderContext, shader: &mut TallGrassShader) {
    let mut stage = StageRef::new();
    let grass_states = |context: &mut BsShaderContext, pass: PassId| {
        render_states(context, pass, &[(0x1B, 0)]);
        if context.render_path < 2 {
            render_states(context, pass, &[(0x0F, 0)]);
        } else {
            render_states(context, pass, &[(0x0F, 1), (0x19, 5), (0x18, 10)]);
            if context.hdr {
                render_states(context, pass, &[(0x1B, 1), (0x13, 5), (0x14, 6)]);
            }
        }
        render_states(
            context,
            pass,
            &[(7, 1), (0x17, 4), (0x0E, 1), (0xA8, 7), (0x1C, 0)],
        );
    };
    let pass_a = shader.passes_7c[0].expect("acquired by the caller");
    append_default_stage(context, pass_a, &mut stage, 0, 3, 2);
    append_default_stage(context, pass_a, &mut stage, 1, 1, 2);
    append_default_stage(context, pass_a, &mut stage, 2, 1, 2);
    grass_states(context, pass_a);
    bind_programs(context, pass_a, &shader.programs[0], &shader.programs[40]); // +0x94, +0x134

    let pass_b = shader.passes_7c[1].expect("acquired by the caller");
    // 007E830C..007E862A.
    append_default_stage(context, pass_b, &mut stage, 0, 3, 2);
    if context.render_path < 2 {
        append_default_stage(context, pass_b, &mut stage, 1, 3, 2);
        append_default_stage(context, pass_b, &mut stage, 2, 3, 2);
    } else {
        append_default_stage(context, pass_b, &mut stage, 1, 3, 2);
        append_default_stage(context, pass_b, &mut stage, 2, 1, 2);
        append_default_stage(context, pass_b, &mut stage, 3, 1, 2);
    }
    grass_states(context, pass_b);
    bind_programs(context, pass_b, &shader.programs[4], &shader.programs[41]); // +0xA4, +0x138

    if context.render_path >= 2 {
        // 007E8880..007E8B36.
        let pass_c = shader.passes_7c[2].expect("acquired by the caller");
        append_default_stage(context, pass_c, &mut stage, 0, 3, 2);
        append_default_stage(context, pass_c, &mut stage, 1, 3, 2);
        render_states(
            context,
            pass_c,
            &[
                (0x0F, 1),
                (0x19, 5),
                (0x18, 10),
                (0x1B, 1),
                (0x13, 9),
                (0x14, 1),
                (7, 1),
                (0x17, 4),
                (0x0E, 0),
                (0xA8, 7),
                (0x1C, 0),
            ],
        );
        // 007AECB0(+0x134), 007AEC60(+0x14C).
        bind_programs(context, pass_c, &shader.programs[40], &shader.programs[46]);
    }
    stage.reset(context);
}

// IDA: 007E9840 slot 2: declaration (5 entries, 1 stream) with vt+0x50
// (0,0,0,2,0) (1,3,3,2,0) (2,5,4,3,0) (3,7,5,1,0) and vt+0x4C (0, 4, 6,
// 0x80000000, 0, 5, 1, 0); FormHeap(0x194) 007E8BB0(declaration); vt+0x84
// 008025F0 (vt+0x98/+0xA0 00977C50, vt+0x9C 00801380), vt+0xA8 007E6B10,
// vt+0xAC 007E9A20 (three pool passes into +0x7C, then 007E7F70); +4 =
// shader; vt+0x54(declaration). Completion: 100%.
pub fn create_tallgrass_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_bs_shader_slot::<TallGrassShader>(
        context,
        5,
        1,
        |declaration| {
            set_entries(
                declaration,
                &[
                    (0, 0, 0, 2, 0),
                    (1, 3, 3, 2, 0),
                    (2, 5, 4, 3, 0),
                    (3, 7, 5, 1, 0),
                ],
            );
            set_entry(declaration, 0, 4, 6, 0x8000_0000, 0, 5, 1);
        },
        false,
        |shader, context, declaration| {
            shader.original_vtable = 0x00A9_202C;
            shader.class_name = "TallGrassShader";
            shader.programs = programs(49);
            shader.declaration.clone_from(declaration);
            context.globals.tallgrass_b460d0 = [0.0, 0.0, 0.0, 1.0];
            shader.instance_capacity = if context.render_path < 2 { 80 } else { 228 };
            shader.instance_buffer = vec![[0.0; 4]; shader.instance_capacity as usize];
        },
        |shader, context| {
            tallgrass_load_programs(context, shader);
            for pass in &mut shader.passes_7c {
                acquire_pass_into(context, pass); // 007E9A46..007E9ABD
            }
            tallgrass_setup_passes(context, shader);
        },
        false,
    )
}

// IDA: DistantLODShader (0x110, vtable A946B4) 00811C20(declaration): base
// 00802540, +0x7C the pass NiPointer, +0x8C four vertex and +0x9C two pixel
// program NiPointers, +0xB0..+0x10C = 0.0, +0x24 the declaration, +0x80
// 0.0, +0xB0..+0xBC = src__25 (0, 0, 0, 1), word +0xAC 80 (render path < 2)
// or 228, +0xA4 FormHeap(16 * +0xAC) (not initialized), +0xA8 0.
#[derive(Default)]
pub struct DistantLodShader {
    pub base: BsShaderObject,
    pub lod_pass: PassSlot, // +0x7C
    pub field_80: f32,
    pub vertex_programs: [ProgramSlot; 4], // +0x8C
    pub pixel_programs: [ProgramSlot; 2],  // +0x9C
    pub instance_buffer: Vec<[f32; 4]>,    // +0xA4
    pub field_a8: u32,
    pub instance_capacity: u16, // +0xAC
    pub floats_b0: [f32; 24],   // +0xB0..+0x10C
}
bs_shader_class!(DistantLodShader, base, BsShaderObject);

// IDA: 00811640 vt+0xA8: render path 1 loads DISTLOD1%03i.vso 0..1 (vs_1_1,
// +0x8C/+0x90) and DISTLOD1000.pso (ps_1_3, +0x9C); otherwise
// DISTLOD2%03i.vso 2..3 (vs_2_0, +0x94/+0x98) and DISTLOD2001.pso (PS_2_0,
// ps_2_0, +0xA0). Completion: 100% with the generated tables.
fn distant_lod_load_programs(context: &mut BsShaderContext, shader: &mut DistantLodShader) {
    if context.render_path == 1 {
        for index in 0..2 {
            shader.vertex_programs[index] = load_vertex(
                context,
                &DISTANT_LOD_VERTEX[index].owned(),
                "vs_1_1",
                &program_name("DISTLOD1%03i.vso", index),
            );
        }
        shader.pixel_programs[0] = load_pixel(
            context,
            &DISTANT_LOD_PIXEL[0].owned(),
            "ps_1_3",
            &program_name("DISTLOD1%03i.pso", 0),
        );
        return;
    }
    for index in 2..4 {
        shader.vertex_programs[index] = load_vertex(
            context,
            &DISTANT_LOD_VERTEX[index].owned(),
            "vs_2_0",
            &program_name("DISTLOD2%03i.vso", index),
        );
    }
    shader.pixel_programs[1] = load_pixel(
        context,
        &DISTANT_LOD_PIXEL[1].owned(),
        "ps_2_0",
        &program_name("DISTLOD2%03i.pso", 1),
    );
}

// IDA: 00812470 vt+0xAC: a pool pass into +0x7C, then 00811A30: stage 0
// (00801110(0, 3, 2)), vertex +0x8C, pixel +0x9C (programs 0 even at render
// path 2+, where only 2/3 and 1 are loaded), ALPHAREF 10 (render path 2+),
// COLORWRITEENABLE 7, ALPHABLENDENABLE 0, ZENABLE 1, ZFUNC 4, ZWRITEENABLE
// 1. Completion: 100%.
fn distant_lod_setup_passes(context: &mut BsShaderContext, shader: &mut DistantLodShader) {
    acquire_pass_into(context, &mut shader.lod_pass);
    let pass = shader.lod_pass.expect("just acquired");
    let mut stage = StageRef::new();
    append_default_stage(context, pass, &mut stage, 0, 3, 2);
    bind_programs(
        context,
        pass,
        &shader.vertex_programs[0],
        &shader.pixel_programs[0],
    );
    if context.render_path >= 2 {
        render_states(context, pass, &[(0x18, 0x0A)]);
    }
    render_states(
        context,
        pass,
        &[(0xA8, 7), (0x1B, 0), (7, 1), (0x17, 4), (0x0E, 1)],
    );
    stage.reset(context);
}

// IDA: 008122A0 slot 3: the declaration (4 entries, 1 stream) with vt+0x50
// (0,0,0,2,0) (1,5,4,3,0) (2,7,5,1,0) and vt+0x4C (0, 3, 6, 0x80000000, 0,
// 5, 1, 0); FormHeap(0x110) 00811C20(declaration); vt+0x84, vt+0xA8, vt+0xAC;
// +4 = shader; vt+0x54. Completion: 100%.
pub fn create_distant_lod_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_bs_shader_slot::<DistantLodShader>(
        context,
        4,
        1,
        |declaration| {
            set_entries(
                declaration,
                &[(0, 0, 0, 2, 0), (1, 5, 4, 3, 0), (2, 7, 5, 1, 0)],
            );
            set_entry(declaration, 0, 3, 6, 0x8000_0000, 0, 5, 1);
        },
        false,
        |shader, context, declaration| {
            shader.original_vtable = 0x00A9_46B4;
            shader.class_name = "DistantLODShader";
            shader.declaration.clone_from(declaration);
            shader.field_80 = 0.0;
            shader.floats_b0[0] = 0.0; // src__25
            shader.floats_b0[1] = 0.0;
            shader.floats_b0[2] = 0.0;
            shader.floats_b0[3] = 1.0;
            shader.instance_capacity = if context.render_path < 2 { 80 } else { 228 };
            shader.instance_buffer = vec![[0.0; 4]; shader.instance_capacity as usize];
            shader.field_a8 = 0;
        },
        |shader, context| {
            distant_lod_load_programs(context, shader);
            distant_lod_setup_passes(context, shader);
        },
        false,
    )
}

// IDA: WaterShader (0x118, vtable A9170C) 007DC9A0: base 00802540, +0x7C
// sixteen pass NiPointers, +0xBC/+0xC0 the vertex programs, +0xC4 sixteen
// pixel programs, +0x104..+0x114 = 0 (NiPointers released), byte +0x20 = 1;
// the globals in BSShaderGlobals.
#[derive(Default)]
pub struct WaterShaderObject {
    pub base: BsShaderObject,
    pub water_passes: [PassSlot; 16],      // +0x7C
    pub vertex_programs: [ProgramSlot; 2], // +0xBC
    pub pixel_programs: [ProgramSlot; 16], // +0xC4
    pub fields_104: [u32; 5],              // +0x104..+0x114
}
bs_shader_class!(WaterShaderObject, base, BsShaderObject);

// IDA: 007DC9A0 the constructor tail. Completion: 100%.
fn construct_water_shader(context: &mut BsShaderContext, shader: &mut WaterShaderObject) {
    shader.original_vtable = 0x00A9_170C;
    shader.class_name = "WaterShader";
    shader.field_20 = 1;
    let globals = &mut context.globals;
    globals.water_scroll_b45e64 = 0.0;
    globals.water_shader_b45dcc = Some(17);
    globals.water_b45e68 = 0.0;
    globals.water_b45dbb = 0;
    globals.water_b45e54 = 0.0;
    globals.water_b45dc0 = 0;
    globals.water_b45e58 = 0.0;
    globals.water_normal_offset_b45ec4 = [0.0; 4];
    globals.water_depth_offset_b45ee4 = [0.0; 4];
    if context.render_path <= 2 {
        globals.water_b45db9 = 0;
    }
}

// IDA: 007DC8D0 vt+0x84: 008025F0, then +0xBC released and, for each of the
// sixteen slots, the +0xC4 program and the +0x7C pass released (all null at
// startup). Completion: 100%.
fn water_shader_initialize_tail(context: &mut BsShaderContext, shader: &mut WaterShaderObject) {
    shader.vertex_programs[0] = None;
    for index in 0..16 {
        shader.pixel_programs[index] = None;
        assign_pass(&mut context.pools, &mut shader.water_passes[index], None);
    }
}

// IDA: 007DB840: WATER000.vso (no macros) into +0xBC and WATER001.vso
// (WADING) into +0xC0 (vs_1_1); the pixel path is "water\2_0\p\water.p.hlsl"
// up to render path 4, else "water\2_ab\p\water.p.hlsl"; WATER%03i.pso 0..15
// (ps_1_3; ps_2_0 from render path 2) into +0xC4. Completion: 100%.
fn water_shader_load_programs(context: &mut BsShaderContext, shader: &mut WaterShaderObject) {
    shader.vertex_programs[0] =
        load_vertex(context, &WATER_VERTEX[0].owned(), "vs_1_1", "WATER000.vso");
    shader.vertex_programs[1] =
        load_vertex(context, &WATER_VERTEX[1].owned(), "vs_1_1", "WATER001.vso");
    let pixel_profile = if context.render_path >= 2 {
        "ps_2_0"
    } else {
        "ps_1_3"
    };
    let pixel_path = if context.render_path <= 4 {
        "water\\2_0\\p\\water.p.hlsl"
    } else {
        "water\\2_ab\\p\\water.p.hlsl"
    };
    for index in 0..16 {
        let mut record = WATER_PIXEL[index].owned();
        pixel_path.clone_into(&mut record.hlsl_path);
        shader.pixel_programs[index] = load_pixel(
            context,
            &record,
            pixel_profile,
            &program_name("WATER%03i.pso", index),
        );
    }
}

// IDA: 007DC1A0: per pass 0..15: when null, a pool pass with stages 0..4
// (00801110 (0,3,2) (1,1,2) (2,1,2) (3,3,2) (4,3,2), each 00771640(1));
// vertex +0xC0 for passes 7..10 (else +0xBC through 007AECB0), pixel
// +0xC4[i]; ZENABLE 1, ZFUNC 4, STENCILENABLE 0, ALPHATESTENABLE 1,
// ALPHAREF 0, ALPHAFUNC 5, COLORWRITEENABLE 7; pass 12 adds
// ALPHABLENDENABLE 0 and ZWRITEENABLE 1 (on +0xAC, itself); the others
// ALPHABLENDENABLE 1, SRCBLEND 5, DESTBLEND 6, ZWRITEENABLE 1, then for 7..10
// STENCILENABLE 1, STENCILREF 0, STENCILFUNC 7, STENCILMASK 8 and for 0..5
// STENCILENABLE 1, STENCILMASK 3, STENCILREF 0, STENCILFUNC 1. Completion: 100%.
fn water_shader_setup_passes(context: &mut BsShaderContext, shader: &mut WaterShaderObject) {
    for index in 0..16 {
        if shader.water_passes[index].is_none() {
            acquire_pass_into(context, &mut shader.water_passes[index]);
            let pass = shader.water_passes[index].expect("just acquired");
            let mut stage = StageRef::new();
            append_filtered_stage(context, pass, &mut stage, 0, 3, 2, 1);
            append_filtered_stage(context, pass, &mut stage, 1, 1, 2, 1);
            append_filtered_stage(context, pass, &mut stage, 2, 1, 2, 1);
            append_filtered_stage(context, pass, &mut stage, 3, 3, 2, 1);
            append_filtered_stage(context, pass, &mut stage, 4, 3, 2, 1);
            stage.reset(context);
        }
        let pass = shader.water_passes[index].expect("set above");
        let wading = (7..=10).contains(&index);
        let vertex = if wading { 1 } else { 0 };
        bind_programs(
            context,
            pass,
            &shader.vertex_programs[vertex],
            &shader.pixel_programs[index],
        );
        render_states(
            context,
            pass,
            &[
                (7, 1),
                (0x17, 4),
                (0x34, 0),
                (0x0F, 1),
                (0x18, 0),
                (0x19, 5),
                (0xA8, 7),
            ],
        );
        if index == 12 {
            render_states(
                context,
                shader.water_passes[12].expect("set above"),
                &[(0x1B, 0), (0x0E, 1)],
            );
            continue;
        }
        render_states(context, pass, &[(0x1B, 1), (0x13, 5), (0x14, 6), (0x0E, 1)]);
        if wading {
            render_states(context, pass, &[(0x34, 1), (0x39, 0), (0x37, 7), (0x38, 8)]);
        }
        if index <= 5 {
            render_states(context, pass, &[(0x34, 1), (0x38, 3), (0x39, 0), (0x37, 1)]);
        }
    }
}

// IDA: 007DB4B0 vt+0x88: +0x2C Scroll .. BlendRadius (c0..c12, globals);
// +0x30 WorldViewProjTranspose (vt+0x18, predefined, c0),
// WorldTranspose (vt+0x1C(key, 0x20000009, 4, Default)), Tile (c8),
// QPosAdjust (c9), DepthOffset (c10). Completion: 100%.
fn water_shader_constant_maps(context: &mut BsShaderContext, shader: &mut WaterShaderObject) {
    shader.pixel_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.pixel_constant_map,
        true,
        |map| {
            add_float4_constant(map, "Scroll", 0, "00B45E64");
            add_float4_constant(map, "EyePos", 1, "00B45DD4");
            add_float4_constant(map, "SunDir", 2, "00B45DF4");
            add_float4_constant(map, "SunColor", 3, "00B45E04");
            add_float4_constant(map, "NormalOffset", 4, "00B45EC4");
            add_float4_constant(map, "ShallowColor", 5, "00B45E14");
            add_float4_constant(map, "DeepColor", 6, "00B45E24");
            add_float4_constant(map, "ReflectionColor", 7, "00B45E34");
            add_float4_constant(map, "VarAmounts", 8, "00B45E44");
            add_float4_constant(map, "FogParam", 9, "00B45E74");
            add_float4_constant(map, "FogColor", 10, "00B45E84");
            add_float4_constant(map, "FresnelRI", 11, "00B45E94");
            add_float4_constant(map, "BlendRadius", 12, "00B45EB4");
        },
    );
    shader.vertex_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.vertex_constant_map,
        false,
        |map| {
            let _ = add_shader_constant_entry(
                map,
                "WorldViewProjTranspose",
                0x2000_0009,
                0,
                0,
                4,
                "",
                0,
                0,
                "",
                false,
            );
            let _ = add_predefined_constant_entry(map, "WorldTranspose", 0x2000_0009, 4, "");
            add_float4_constant(map, "Tile", 8, "00B45EA4");
            add_float4_constant(map, "QPosAdjust", 9, "00B45ED4");
            add_float4_constant(map, "DepthOffset", 10, "00B45EE4");
        },
    );
}

// IDA: 007DD1E0 slot 17: the two-entry declaration, FormHeap(0x118)
// 007DC9A0, vt+0x84 007DC8D0, 007DB840, 007DC1A0, vt+0x88 007DB4B0, vt+0x54,
// +4. Completion: 100%.
pub fn create_water_shader_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_bs_shader_slot::<WaterShaderObject>(
        context,
        2,
        1,
        declare_position_texture,
        false,
        |shader, context, _| construct_water_shader(context, shader),
        |shader, context| {
            water_shader_initialize_tail(context, shader);
            water_shader_load_programs(context, shader);
            water_shader_setup_passes(context, shader);
            water_shader_constant_maps(context, shader);
        },
        true,
    )
}

// IDA: SkyShader (0xE8, vtable A8F654) 007BA440: base 00802540, +0x7C..+0xD8
// = 0 (NiPointers/passes released: 0xB8/0xBC/0xC0/0xB4/0xAC passes, the rest
// programs; +0xB0 a plain field), the seven 00B43168..00B431D4 vectors =
// src__25 (0, 0, 0, 1), byte +0x20 = 1.
#[derive(Default)]
pub struct SkyShader {
    pub base: BsShaderObject,
    pub sky_vertex: ProgramSlot,          // +0x7C SKY.vso
    pub tex_vertex: ProgramSlot,          // +0x80 SKYT.vso
    pub quad_vertex: ProgramSlot,         // +0x84 SKYQUAD.vso
    pub horiz_fade_vertex: ProgramSlot,   // +0x88
    pub occlusion_vertex: ProgramSlot,    // +0x8C SKYOCC.vso
    pub clouds_vertex: ProgramSlot,       // +0x90
    pub clouds_fade_vertex: ProgramSlot,  // +0x94
    pub tex_pixel: ProgramSlot,           // +0x98 SKYTEX.pso
    pub tex_fade_pixel: ProgramSlot,      // +0x9C
    pub sky_pixel: ProgramSlot,           // +0xA0 SKY.pso
    pub sun_occlusion_pixel: ProgramSlot, // +0xA4
    pub horiz_fade_pixel: ProgramSlot,    // +0xA8
    pub tex_pass: PassSlot,               // +0xAC
    pub field_b0: u32,
    pub occlusion_pass: PassSlot,      // +0xB4
    pub quad_si_pass: PassSlot,        // +0xB8
    pub far_pass: PassSlot,            // +0xBC
    pub clouds_si_pass: PassSlot,      // +0xC0
    pub far_vertex: ProgramSlot,       // +0xC4 SKYFAR.vso
    pub clouds_si_vertex: ProgramSlot, // +0xC8
    pub quad_si_vertex: ProgramSlot,   // +0xCC
    pub si_pixel: ProgramSlot,         // +0xD0 SKYSI.pso
    pub si_sun_pixel: ProgramSlot,     // +0xD4 SKYSISUN.pso
    pub si_clouds_pixel: ProgramSlot,  // +0xD8
}
bs_shader_class!(SkyShader, base, BsShaderObject);

// IDA: 007BB280: the eighteen loads in order; vertex programs with
// Renderer_GetVertexShaderTarget(), pixel with Renderer_GetPixelShaderTarget(0);
// SKYSI.pso's SI is _gcvt(00B43158 sky brightness, 12) and SKYSISUN.pso's
// _gcvt(00B43154 sun brightness, 12) (the earlier sun conversion before
// SKYSUNOCCL.pso is overwritten unused). Completion: 100%.
fn sky_load_programs(context: &mut BsShaderContext, shader: &mut SkyShader) {
    // The eighteen records, transcribed from the stack stores before each load
    // (the lists are rebuilt between the calls). A null definition is
    // None; SI_CLOUDS of SKYCLOUDSI.vso is Default ("").
    const V: &str = "sky\\v\\sky.v.hlsl";
    const Q: &str = "sky\\v\\sky_quad.v.hlsl";
    const P: &str = "sky\\p\\sky.p.hlsl";
    let vertex = vertex_target(context);
    shader.sky_vertex = load_vertex(context, &descriptor(V, &[]), vertex, "SKY.vso");
    shader.tex_vertex = load_vertex(
        context,
        &descriptor(V, &[("TEX", None)]),
        vertex,
        "SKYT.vso",
    );
    shader.quad_vertex = load_vertex(context, &descriptor(Q, &[]), vertex, "SKYQUAD.vso");
    shader.horiz_fade_vertex = load_vertex(
        context,
        &descriptor(V, &[("HORIZFADE", None)]),
        vertex,
        "SKYHORIZFADE.vso",
    );
    shader.occlusion_vertex = load_vertex(
        context,
        &descriptor(V, &[("OCCLUSION", None)]),
        vertex,
        "SKYOCC.vso",
    );
    shader.clouds_vertex = load_vertex(
        context,
        &descriptor(V, &[("TEX", None), ("CLOUDS", None)]),
        vertex,
        "SKYCLOUDS.vso",
    );
    shader.clouds_fade_vertex = load_vertex(
        context,
        &descriptor(V, &[("HORIZFADE", None), ("TEX", None), ("CLOUDS", None)]),
        vertex,
        "SKYCLOUDSFADE.vso",
    );
    let pixel = pixel_target(context);
    shader.sky_pixel = load_pixel(context, &descriptor(P, &[]), pixel, "SKY.pso");
    shader.tex_pixel = load_pixel(
        context,
        &descriptor(P, &[("TEX", None)]),
        pixel,
        "SKYTEX.pso",
    );
    shader.tex_fade_pixel = load_pixel(
        context,
        &descriptor(P, &[("HORIZFADE", None), ("TEX", None)]),
        pixel,
        "SKYTEXFADE.pso",
    );
    shader.sun_occlusion_pixel = load_pixel(
        context,
        &descriptor(P, &[("OCCLUSION", None)]),
        pixel,
        "SKYSUNOCCL.pso",
    );
    shader.horiz_fade_pixel = load_pixel(
        context,
        &descriptor(P, &[("HORIZFADE", None)]),
        pixel,
        "SKYSHORIZFADE.pso",
    );
    shader.far_vertex = load_vertex(
        context,
        &descriptor(V, &[("DEPTH_VALUE", Some("0.999999")), ("SI", None)]),
        vertex,
        "SKYFAR.vso",
    );
    shader.clouds_si_vertex = load_vertex(
        context,
        &descriptor(
            V,
            &[
                ("DEPTH_VALUE", Some("0.999999")),
                ("SI", None),
                ("SI_CLOUDS", Some("")),
            ],
        ),
        vertex,
        "SKYCLOUDSI.vso",
    );
    shader.quad_si_vertex = load_vertex(
        context,
        &descriptor(Q, &[("SI", None)]),
        vertex,
        "SKYQUADSI.vso",
    );
    let sky = gcvt_12(context.sky_brightness);
    shader.si_pixel = load_pixel(
        context,
        &descriptor(P, &[("SI", Some(&sky))]),
        pixel,
        "SKYSI.pso",
    );
    let sun = gcvt_12(context.sun_brightness);
    shader.si_sun_pixel = load_pixel(
        context,
        &descriptor(P, &[("SI", Some(&sun)), ("SI_SUN", None)]),
        pixel,
        "SKYSISUN.pso",
    );
    shader.si_clouds_pixel = load_pixel(
        context,
        &descriptor(P, &[("SI", None), ("SI_CLOUDS", None)]),
        pixel,
        "SKYSICLOUDS.pso",
    );
}

// IDA: one sky pass of 007BC1B0; see `sky_setup_passes`.
fn sky_build(
    context: &mut BsShaderContext,
    slot: &mut PassSlot,
    second_stage: bool,
    states: &[(u32, u32)],
) -> PassId {
    if let Some(pass) = *slot {
        return pass;
    }
    acquire_pass_into(context, slot);
    let pass = slot.expect("just acquired");
    let mut stage = StageRef::new();
    append_default_stage(context, pass, &mut stage, 0, 3, 2);
    if second_stage {
        append_default_stage(context, pass, &mut stage, 1, 3, 2);
    }
    render_states(context, pass, states);
    stage.reset(context);
    pass
}

// IDA: 007BC1B0: each pass is built only when its slot is null (a pool
// pass, stage 0 = 00801110(0, 3, 2), +0x70 also stage 1 = (1, 3, 2), then
// the render states), then its programs are bound:
//   +0x70: ZENABLE 1, ZFUNC 4, ZWRITEENABLE 0, ALPHABLENDENABLE 1, SRCBLEND 5,
//          DESTBLEND 6, STENCILENABLE 0, ALPHATESTENABLE 0, COLORWRITEENABLE 7;
//          vertex +0x7C (no pixel program).
//   +0xAC: ZENABLE 0, ZWRITEENABLE 0, ALPHABLENDENABLE 1, SRCBLEND 5,
//          DESTBLEND 2, STENCILENABLE 0, ALPHATESTENABLE 0, COLORWRITEENABLE 7;
//          +0x80/+0x98.
//   +0xB4: ALPHABLENDENABLE 1, SRCBLEND 2, DESTBLEND 2, ALPHATESTENABLE 0,
//          ZENABLE 1, ZWRITEENABLE 0, ZFUNC 4, COLORWRITEENABLE 0; +0x8C/+0xA4.
//   +0xB8: ALPHATESTENABLE 0, ALPHABLENDENABLE 0, ZENABLE 1, ZWRITEENABLE 0,
//          ZFUNC 4, COLORWRITEENABLE 8; +0xCC/+0xD0.
//   +0xBC: ALPHABLENDENABLE 1, SRCBLEND 2, DESTBLEND 2, ALPHATESTENABLE 0,
//          ZENABLE 1, ZWRITEENABLE 0, ZFUNC 4, COLORWRITEENABLE 8; +0xC4/+0xD4.
//   +0xC0: ALPHATESTENABLE 0, ALPHABLENDENABLE 1, SRCBLEND 1, DESTBLEND 6,
//          ZENABLE 1, ZWRITEENABLE 0, ZFUNC 4, COLORWRITEENABLE 8; +0xC8/+0xD8.
// Completion: 100%.
fn sky_setup_passes(context: &mut BsShaderContext, shader: &mut SkyShader) {
    let mut current = shader.current_pass;
    let pass = sky_build(
        context,
        &mut current,
        true,
        &[
            (7, 1),
            (0x17, 4),
            (0x0E, 0),
            (0x1B, 1),
            (0x13, 5),
            (0x14, 6),
            (0x34, 0),
            (0x0F, 0),
            (0xA8, 7),
        ],
    );
    shader.current_pass = current;
    context
        .pools
        .pass_mut(pass)
        .vertex_shader
        .clone_from(&shader.sky_vertex);
    let pass = sky_build(
        context,
        &mut shader.tex_pass,
        false,
        &[
            (7, 0),
            (0x0E, 0),
            (0x1B, 1),
            (0x13, 5),
            (0x14, 2),
            (0x34, 0),
            (0x0F, 0),
            (0xA8, 7),
        ],
    );
    bind_programs(context, pass, &shader.tex_vertex, &shader.tex_pixel);
    let pass = sky_build(
        context,
        &mut shader.occlusion_pass,
        false,
        &[
            (0x1B, 1),
            (0x13, 2),
            (0x14, 2),
            (0x0F, 0),
            (7, 1),
            (0x0E, 0),
            (0x17, 4),
            (0xA8, 0),
        ],
    );
    bind_programs(
        context,
        pass,
        &shader.occlusion_vertex,
        &shader.sun_occlusion_pixel,
    );
    let pass = sky_build(
        context,
        &mut shader.quad_si_pass,
        false,
        &[
            (0x0F, 0),
            (0x1B, 0),
            (7, 1),
            (0x0E, 0),
            (0x17, 4),
            (0xA8, 8),
        ],
    );
    bind_programs(context, pass, &shader.quad_si_vertex, &shader.si_pixel);
    let pass = sky_build(
        context,
        &mut shader.far_pass,
        false,
        &[
            (0x1B, 1),
            (0x13, 2),
            (0x14, 2),
            (0x0F, 0),
            (7, 1),
            (0x0E, 0),
            (0x17, 4),
            (0xA8, 8),
        ],
    );
    bind_programs(context, pass, &shader.far_vertex, &shader.si_sun_pixel);
    let pass = sky_build(
        context,
        &mut shader.clouds_si_pass,
        false,
        &[
            (0x0F, 0),
            (0x1B, 1),
            (0x13, 1),
            (0x14, 6),
            (7, 1),
            (0x0E, 0),
            (0x17, 4),
            (0xA8, 8),
        ],
    );
    bind_programs(
        context,
        pass,
        &shader.clouds_si_vertex,
        &shader.si_clouds_pixel,
    );
}

// IDA: 007BB080 vt+0x88: +0x2C Params (0x10000005, c4, size 8, this+0xE0);
// +0x30 WorldViewProjTranspose (c0), WorldTranspose (c8) (predefined, count
// 4), Color0..Color2 (c4..c6, 00B43178/00B43188/00B43198), eyepos (c7,
// 00B43168), Params (0x10000004, c12, size 4, this+0xDC). Completion: 100%.
fn sky_constant_maps(context: &mut BsShaderContext, shader: &mut SkyShader) {
    shader.pixel_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.pixel_constant_map,
        true,
        |map| {
            add_constant(map, "Params", 0x1000_0005, 4, 1, 8, "this+0xE0");
        },
    );
    shader.vertex_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.vertex_constant_map,
        false,
        |map| {
            add_predefined(map, "WorldViewProjTranspose", 0, 0, 4);
            add_predefined(map, "WorldTranspose", 0, 8, 4);
            add_float4_constant(map, "Color0", 4, "00B43178");
            add_float4_constant(map, "Color1", 5, "00B43188");
            add_float4_constant(map, "Color2", 6, "00B43198");
            add_float4_constant(map, "eyepos", 7, "00B43168");
            add_constant(map, "Params", 0x1000_0004, 12, 1, 4, "this+0xDC");
        },
    );
}

// IDA: 007BD920 slot 10: the declaration (3 entries, 1 stream) with vt+0x50
// (0,0,0,2,0) (1,5,4,3,0) (2,7,5,1,0); FormHeap(0xE8) 007BA440; vt+0x84
// 007BD0A0 (008025F0, then the +0x70 default pass released); 007BB280;
// 007BC1B0; vt+0x88 007BB080; vt+0x54; +4. Completion: 100%.
pub fn create_sky_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_bs_shader_slot::<SkyShader>(
        context,
        3,
        1,
        |declaration| {
            set_entries(
                declaration,
                &[(0, 0, 0, 2, 0), (1, 5, 4, 3, 0), (2, 7, 5, 1, 0)],
            )
        },
        false,
        |shader, context, _| {
            shader.original_vtable = 0x00A8_F654;
            shader.class_name = "SkyShader";
            context.globals.sky_vectors_b43168 = [[0.0, 0.0, 0.0, 1.0]; 7]; // src__25
            shader.field_20 = 1;
        },
        |shader, context| {
            let mut current = shader.current_pass;
            assign_pass(&mut context.pools, &mut current, None); // 007BD0A9..007BD0BD
            shader.current_pass = current;
            sky_load_programs(context, shader);
            sky_setup_passes(context, shader);
            sky_constant_maps(context, shader);
        },
        true,
    )
}

// IDA: GeometryDecalShader (0xA0, vtable A93F2C) 00805D90: base 00802540,
// +0x7C two passes, +0x84 two vertex and +0x8C two pixel program NiPointers,
// +0x94/+0x98/+0x9C constant-map entry NiPointers = 0, byte +0x20 = 1 (the
// slots and passes released, all null).
#[derive(Default)]
pub struct GeometryDecalShader {
    pub base: BsShaderObject,
    pub decal_passes: [PassSlot; 2],       // +0x7C
    pub vertex_programs: [ProgramSlot; 2], // +0x84
    pub pixel_programs: [ProgramSlot; 2],  // +0x8C
    pub world_view_proj_entry: Option<Arc<ShaderConstantMapEntry>>, // +0x94
    pub skin_world_view_proj_entry: Option<Arc<ShaderConstantMapEntry>>, // +0x98
    pub bone_matrix_entry: Option<Arc<ShaderConstantMapEntry>>, // +0x9C
}
bs_shader_class!(GeometryDecalShader, base, BsShaderObject);

// IDA: 00805320: GDECAL.vso and GDECALS.vso (vs_1_1) into +0x84/+0x88,
// GDECAL.pso (ps_1_3; ps_2_0 from render path 2) into +0x8C, then +0x90 =
// +0x8C. Completion: 100%.
fn decal_load_programs(context: &mut BsShaderContext, shader: &mut GeometryDecalShader) {
    let decal = [
        ("DECAL", Some("")),
        ("GEOMDECAL", Some("")),
        ("MAXDECALS", Some("1")),
    ];
    let skin = [
        ("DECAL", Some("")),
        ("GEOMDECAL", Some("")),
        ("MAXDECALS", Some("1")),
        ("SKIN", Some("")),
    ];
    shader.vertex_programs[0] = load_vertex(
        context,
        &descriptor("lighting\\2x\\v\\decal.v.hlsl", &decal),
        "vs_1_1",
        "GDECAL.vso",
    );
    shader.vertex_programs[1] = load_vertex(
        context,
        &descriptor("lighting\\2x\\v\\decal.v.hlsl", &skin),
        "vs_1_1",
        "GDECALS.vso",
    );
    let pixel_profile = if context.render_path >= 2 {
        "ps_2_0"
    } else {
        "ps_1_3"
    };
    shader.pixel_programs[0] = load_pixel(
        context,
        &descriptor("lighting\\2x\\p\\decal.p.hlsl", &decal),
        pixel_profile,
        "GDECAL.pso",
    );
    shader.pixel_programs[1] = shader.pixel_programs[0].clone();
}

// IDA: 00805670: pool passes into +0x7C/+0x80, then vt+0xA8 00805740: for
// each pass (a local NiPointer moved from +0x7C to +0x80; one local stage
// shared by both): when its stage count (+0x18) is below 2, stage 0 =
// 00801110(0, 1, 2) and stage 1 = (1, 3, 2); vertex +0x84/+0x88, pixel
// +0x8C/+0x90; COLORWRITEENABLE 0xF, ALPHABLENDENABLE 1, SRCBLEND 9,
// DESTBLEND 6, ALPHATESTENABLE 1, ALPHAFUNC 5, ALPHAREF 0, ZENABLE 1,
// ZFUNC 4, ZWRITEENABLE 0, STENCILENABLE 0. Completion: 100%.
fn decal_setup_passes(context: &mut BsShaderContext, shader: &mut GeometryDecalShader) {
    for pass in &mut shader.decal_passes {
        acquire_pass_into(context, pass);
    }
    let mut stage = StageRef::new();
    for index in 0..2 {
        let pass = shader.decal_passes[index].expect("just acquired");
        if context.pools.pass(pass).stage_count < 2 {
            append_default_stage(context, pass, &mut stage, 0, 1, 2);
            append_default_stage(context, pass, &mut stage, 1, 3, 2);
        }
        bind_programs(
            context,
            pass,
            &shader.vertex_programs[index],
            &shader.pixel_programs[index],
        );
        render_states(
            context,
            pass,
            &[
                (0xA8, 0xF),
                (0x1B, 1),
                (0x13, 9),
                (0x14, 6),
                (0x0F, 1),
                (0x19, 5),
                (0x18, 0),
                (7, 1),
                (0x17, 4),
                (0x0E, 0),
                (0x34, 0),
            ],
        );
    }
    stage.reset(context);
}

// IDA: 00805080 vt+0x88: +0x2C Decal Offset (c15, 00B46198), Decal Count
// (c14, 00B4615C); +0x30 WorldViewProjTranspose (vt+0x18, c0) -> vt+0x38 into
// +0x94, SkinWorldViewProjTranspose (vt+0x1C(key, 0x20000009, 1, Default))
// -> +0x98, DecalCount (c30, 00B4615C), BoneMatrix3 (vt+0x18, extra 0x80000,
// c42, 24) -> +0x9C, decal fade (c31, 00B46218). Completion: 100%.
fn decal_constant_maps(context: &mut BsShaderContext, shader: &mut GeometryDecalShader) {
    shader.pixel_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.pixel_constant_map,
        true,
        |map| {
            add_float4_constant(map, "Decal Offset", 15, "00B46198");
            add_float4_constant(map, "Decal Count", 14, "00B4615C");
        },
    );
    let vertex = shader.vertex_constant_map;
    shader.vertex_constant_map =
        build_constant_map(&mut context.constant_maps, vertex, false, |map| {
            add_predefined(map, "WorldViewProjTranspose", 0, 0, 4);
            shader.world_view_proj_entry =
                find_shader_constant_entry(map, "WorldViewProjTranspose");
            let _ = add_predefined_constant_entry(
                map,
                "SkinWorldViewProjTranspose",
                0x2000_0009,
                1,
                "",
            );
            shader.skin_world_view_proj_entry =
                find_shader_constant_entry(map, "SkinWorldViewProjTranspose");
            add_float4_constant(map, "DecalCount", 30, "00B4615C");
            add_predefined(map, "BoneMatrix3", 0x8_0000, 42, 24);
            shader.bone_matrix_entry = find_shader_constant_entry(map, "BoneMatrix3");
            add_float4_constant(map, "decal fade", 31, "00B46218");
        });
}

// IDA: 00806210 slot 16: 00771090(00B43104, 2, 1) NiPointer-assigned to
// 00B47604 and from it to +0; 00771090(4, 1) into 00B47608; +0 vt+0x50
// (0,0,0,2,0) (1,3,3,2,0); 00B47608 vt+0x50 (0,0,0,2,0) (1,1,1,3,0)
// (2,2,2,4,0) (3,3,3,2,0); FormHeap(0xA0) 00805D90; vt+0x84 (008025F0 thunk
// 007E2D40), 00805320, 00805670, vt+0x88 00805080, vt+0x54, +4. Completion: 100%.
pub fn create_geometry_decal_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    let declaration = make_declaration(context, 2, 1, |declaration| {
        set_entries(declaration, &[(0, 0, 0, 2, 0), (1, 3, 3, 2, 0)]);
    });
    context
        .globals
        .decal_declaration_b47604
        .clone_from(&declaration);
    let slot = BsShaderSlot {
        declaration,
        shader: None,
    };
    let renderer: &RendererState = context
        .renderer
        .as_deref()
        .expect("00B43104 gates the slots (007BA252)");
    let mut skin = create_shader_declaration(renderer, 4, 1);
    if let Some(skin) = skin.as_mut() {
        set_declaration_entry_by_parameter(skin, 0, 0, 0, 2, 0);
        set_declaration_entry_by_parameter(skin, 1, 1, 1, 3, 0);
        set_declaration_entry_by_parameter(skin, 2, 2, 2, 4, 0);
        set_declaration_entry_by_parameter(skin, 3, 3, 3, 2, 0);
    }
    context.globals.decal_skin_declaration_b47608 = skin.map(Arc::new);
    let mut shader = GeometryDecalShader::default();
    construct_bs_shader_base(&mut shader, false);
    shader.original_vtable = 0x00A9_3F2C;
    shader.class_name = "GeometryDecalShader";
    shader.field_20 = 1;
    let _ = initialize_bs_shader(context, &mut shader);
    decal_load_programs(context, &mut shader);
    decal_setup_passes(context, &mut shader);
    decal_constant_maps(context, &mut shader);
    finish_slot(slot, shader, true)
}

// IDA: the SpeedTree loaders' record loops (007F0380/0080E0C0): a null path
// releases the slot instead of loading. Four vertex records (vs_1_1) and two
// pixel records (ps_1_3 with the first format below render path 2, else
// ps_2_0 with the second).
#[allow(clippy::too_many_arguments)] // The original's formats and tables.
fn speedtree_load_programs(
    context: &mut BsShaderContext,
    vertex_programs: &mut [ProgramSlot; 4],
    pixel_programs: &mut [ProgramSlot; 2],
    vertex: &[ProgramRecord; 4],
    pixel: &[ProgramRecord; 2],
    vertex_format: &str,
    pixel_format_1x: &str,
    pixel_format_2x: &str,
) {
    for index in 0..4 {
        if vertex[index].is_null() {
            vertex_programs[index] = None;
            continue;
        }
        vertex_programs[index] = load_vertex(
            context,
            &vertex[index].owned(),
            "vs_1_1",
            &program_name(vertex_format, index),
        );
    }
    let path_1x = context.render_path < 2;
    for index in 0..2 {
        if pixel[index].is_null() {
            pixel_programs[index] = None;
            continue;
        }
        pixel_programs[index] = load_pixel(
            context,
            &pixel[index].owned(),
            if path_1x { "ps_1_3" } else { "ps_2_0" },
            &program_name(
                if path_1x {
                    pixel_format_1x
                } else {
                    pixel_format_2x
                },
                index,
            ),
        );
    }
}

// IDA: SpeedTreeLeafShader (0x398, vtable A92844) 007F1200: base 00802540,
// +0x37C four vertex and +0x38C two pixel program NiPointers, +0x394 = 0
// (the pass), +0x7C 0x300 bytes zeroed (the 48 leaf-data vectors), then
// 00B46758 and 00B46768 = (1, 1, 1, 1), 00B46778 and 00B46788 = 0.
#[derive(Default)]
pub struct SpeedTreeLeafShader {
    pub base: BsShaderObject,
    pub leaf_data: Vec<[f32; 4]>,          // +0x7C
    pub vertex_programs: [ProgramSlot; 4], // +0x37C
    pub pixel_programs: [ProgramSlot; 2],  // +0x38C
    pub leaf_pass: PassSlot,               // +0x394
}
bs_shader_class!(SpeedTreeLeafShader, base, BsShaderObject);

// IDA: 007F09D0: a pool pass into +0x394, stage 0 = 00801110(0, 1, 2),
// vertex +0x37C, pixel +0x38C, ZENABLE 1, ZFUNC 4, ZWRITEENABLE 1. Completion: 100%.
fn leaf_setup_passes(context: &mut BsShaderContext, shader: &mut SpeedTreeLeafShader) {
    acquire_pass_into(context, &mut shader.leaf_pass);
    let pass = shader.leaf_pass.expect("just acquired");
    let mut stage = StageRef::new();
    append_default_stage(context, pass, &mut stage, 0, 1, 2);
    bind_programs(
        context,
        pass,
        &shader.vertex_programs[0],
        &shader.pixel_programs[0],
    );
    render_states(context, pass, &[(7, 1), (0x17, 4), (0x0E, 1)]);
    stage.reset(context);
}

// IDA: 007F07D0 vt+0x88 (+0x30 only): WorldViewProjTranspose (c0), Ambient
// Color (c5, 00B46498), Diff Color 0/1 (c6/c7, 00B464A8/00B464B8), FogParam |
// ShadowVolumeFatness (c8, 00B46638), FogColor | ShadowVolumeExtrudeDistance
// (c9, 00B46648), Tree Dimmer (g_rendererHDR only: 0x10000004, c10, 4 bytes,
// 00B42EA8), grouped constants (c11, 6 registers, 96 bytes, 00B46738),
// 007F16F0(17), leaf data (c34, 48 registers, 768 bytes, this+0x7C). Completion: 100%.
fn leaf_constant_maps(context: &mut BsShaderContext, shader: &mut SpeedTreeLeafShader) {
    let hdr = context.hdr;
    shader.vertex_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.vertex_constant_map,
        false,
        |map| {
            add_predefined(map, "WorldViewProjTranspose", 0, 0, 4);
            add_float4_constant(map, "Ambient Color", 5, "00B46498");
            add_float4_constant(map, "Diff Color 0", 6, "00B464A8");
            add_float4_constant(map, "Diff Color 1", 7, "00B464B8");
            add_float4_constant(map, "FogParam | ShadowVolumeFatness", 8, "00B46638");
            add_float4_constant(map, "FogColor | ShadowVolumeExtrudeDistance", 9, "00B46648");
            if hdr {
                add_constant(map, "Tree Dimmer", 0x1000_0004, 10, 1, 4, "00B42EA8");
            }
            add_constant(map, "grouped constants", 0x1000_0009, 11, 6, 96, "00B46738");
            add_tree_constants(map, 17);
            add_constant(map, "leaf data", 0x1000_0009, 34, 48, 768, "this+0x7C");
        },
    );
}

// IDA: 007F1410 slot 6: the declaration (4 entries, 1 stream) with vt+0x50
// (0,0,0,2,0) (1,3,3,2,0) (2,7,5,1,0) and vt+0x4C (0, 3, 8, 0x80000000, 3,
// 2, 0, 0); FormHeap(0x398) 007F1200; vt+0x84, vt+0xA8 007F0380 (STLEAF),
// 007F09D0, vt+0x88 007F07D0; +4 = shader; vt+0x54. Completion: 100%.
pub fn create_speedtree_leaf_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    const V: &str = "speedtree\\leaf.v.hlsl";
    const P: &str = "speedtree\\leaf.p.hlsl";
    static VERTEX: [ProgramRecord; 4] = [
        ProgramRecord {
            path: V,
            macros: &[],
        },
        ProgramRecord {
            path: V,
            macros: &[("FOG", Some(""))],
        },
        ProgramRecord {
            path: V,
            macros: &[("PT", Some(""))],
        },
        ProgramRecord {
            path: V,
            macros: &[("PT", Some("")), ("FOG", Some(""))],
        },
    ];
    static PIXEL: [ProgramRecord; 2] = [
        ProgramRecord {
            path: P,
            macros: &[],
        },
        ProgramRecord {
            path: P,
            macros: &[("FOG", Some(""))],
        },
    ];
    create_bs_shader_slot::<SpeedTreeLeafShader>(
        context,
        4,
        1,
        |declaration| {
            set_entries(
                declaration,
                &[(0, 0, 0, 2, 0), (1, 3, 3, 2, 0), (2, 7, 5, 1, 0)],
            );
            set_entry(declaration, 0, 3, 8, 0x8000_0000, 3, 2, 0);
        },
        false,
        |shader, context, _| {
            shader.original_vtable = 0x00A9_2844;
            shader.class_name = "SpeedTreeLeafShader";
            shader.leaf_pass = None;
            shader.leaf_data = vec![[0.0; 4]; 48];
            let globals = &mut context.globals;
            globals.leaf_vectors_b46758[0] = [1.0; 4];
            globals.leaf_vectors_b46758[1] = [1.0; 4];
            globals.leaf_vectors_b46758[2] = [0.0; 4];
            globals.leaf_vectors_b46758[3] = [0.0; 4];
        },
        |shader, context| {
            speedtree_load_programs(
                context,
                &mut shader.vertex_programs,
                &mut shader.pixel_programs,
                &VERTEX,
                &PIXEL,
                "STLEAF%03i.vso",
                "STLEAF%03i.pso",
                "STLEAF2%03i.pso",
            );
            leaf_setup_passes(context, shader);
            leaf_constant_maps(context, shader);
        },
        false,
    )
}

// IDA: SpeedTreeFrondShader (0xB8, vtable A9443C) 0080E8C0: base 00802540,
// +0x7C four vertex and +0x8C two pixel program NiPointers, +0x94 = 0 (the
// pass), +0x98..+0xB4 = 0.0 (PointLightPos +0x98, DirectronalLightDir +0xA8).
#[derive(Default)]
pub struct SpeedTreeFrondShader {
    pub base: BsShaderObject,
    pub vertex_programs: [ProgramSlot; 4], // +0x7C
    pub pixel_programs: [ProgramSlot; 2],  // +0x8C
    pub frond_pass: PassSlot,              // +0x94
    pub floats_98: [f32; 8],               // +0x98..+0xB4
}
bs_shader_class!(SpeedTreeFrondShader, base, BsShaderObject);

// IDA: 0080E730: a pool pass into +0x94, stage 0 = 00801110(0, 3, 2),
// vertex +0x7C, pixel +0x8C (no render states). Completion: 100%.
fn frond_setup_passes(context: &mut BsShaderContext, shader: &mut SpeedTreeFrondShader) {
    acquire_pass_into(context, &mut shader.frond_pass);
    let pass = shader.frond_pass.expect("just acquired");
    let mut stage = StageRef::new();
    append_default_stage(context, pass, &mut stage, 0, 3, 2);
    bind_programs(
        context,
        pass,
        &shader.vertex_programs[0],
        &shader.pixel_programs[0],
    );
    stage.reset(context);
}

// IDA: 0080E500 vt+0x88 (+0x30 only): WorldViewProjTranspose (c0), Ambient
// Color (c5), Diff Color 0/1 (c6/c7), DirectronalLightDir (c8, this+0xA8),
// PointLightPos (c9, this+0x98), LightRadius (c10, 00B465A8), FogParam |
// ShadowVolumeFatness (c14), FogColor | ShadowVolumeExtrudeDistance (c15),
// Tree Dimmer (g_rendererHDR only, c16), 007F16F0(17). Completion: 100%.
fn frond_constant_maps(context: &mut BsShaderContext, shader: &mut SpeedTreeFrondShader) {
    let hdr = context.hdr;
    shader.vertex_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.vertex_constant_map,
        false,
        |map| {
            add_predefined(map, "WorldViewProjTranspose", 0, 0, 4);
            add_float4_constant(map, "Ambient Color", 5, "00B46498");
            add_float4_constant(map, "Diff Color 0", 6, "00B464A8");
            add_float4_constant(map, "Diff Color 1", 7, "00B464B8");
            add_float4_constant(map, "DirectronalLightDir", 8, "this+0xA8");
            add_float4_constant(map, "PointLightPos", 9, "this+0x98");
            add_float4_constant(map, "LightRadius", 10, "00B465A8");
            add_float4_constant(map, "FogParam | ShadowVolumeFatness", 14, "00B46638");
            add_float4_constant(
                map,
                "FogColor | ShadowVolumeExtrudeDistance",
                15,
                "00B46648",
            );
            if hdr {
                add_constant(map, "Tree Dimmer", 0x1000_0004, 16, 1, 4, "00B42EA8");
            }
            add_tree_constants(map, 17);
        },
    );
}

// IDA: 0080EE10 slot 5: the declaration (5 entries, 1 stream) with vt+0x50
// (0,0,0,2,0) (1,3,3,2,0) (2,5,4,3,0) (3,7,5,1,0) and vt+0x4C (0, 4, 8,
// 0x80000000, 3, 2, 0, 0); FormHeap(0xB8) 0080E8C0; vt+0x84, vt+0xA8 0080E0C0
// (STFROND), 0080E730, vt+0x88 0080E500; +4 = shader; vt+0x54. Completion: 100%.
pub fn create_speedtree_frond_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    const V: &str = "speedtree\\frond.v.hlsl";
    const P: &str = "speedtree\\frond.p.hlsl";
    static VERTEX: [ProgramRecord; 4] = [
        ProgramRecord {
            path: V,
            macros: &[],
        },
        ProgramRecord {
            path: V,
            macros: &[("FOG", Some(""))],
        },
        ProgramRecord {
            path: V,
            macros: &[("PT", Some(""))],
        },
        ProgramRecord {
            path: V,
            macros: &[("PT", Some("")), ("FOG", Some(""))],
        },
    ];
    static PIXEL: [ProgramRecord; 2] = [
        ProgramRecord {
            path: P,
            macros: &[],
        },
        ProgramRecord {
            path: P,
            macros: &[("FOG", Some(""))],
        },
    ];
    create_bs_shader_slot::<SpeedTreeFrondShader>(
        context,
        5,
        1,
        |declaration| {
            set_entries(
                declaration,
                &[
                    (0, 0, 0, 2, 0),
                    (1, 3, 3, 2, 0),
                    (2, 5, 4, 3, 0),
                    (3, 7, 5, 1, 0),
                ],
            );
            set_entry(declaration, 0, 4, 8, 0x8000_0000, 3, 2, 0);
        },
        false,
        |shader, _, _| {
            shader.original_vtable = 0x00A9_443C;
            shader.class_name = "SpeedTreeFrondShader";
            shader.frond_pass = None;
            shader.floats_98 = [0.0; 8];
        },
        |shader, context| {
            speedtree_load_programs(
                context,
                &mut shader.vertex_programs,
                &mut shader.pixel_programs,
                &VERTEX,
                &PIXEL,
                "STFROND%03i.vso",
                "STFROND%03i.pso",
                "STFROND2%03i.pso",
            );
            frond_setup_passes(context, shader);
            frond_constant_maps(context, shader);
        },
        false,
    )
}

// IDA: the particle/bolt pass bodies (007E3730/007F4190): a local pass
// NiPointer; stage 0 = 00801110(0, 1, 2) only when the pass has no stages
// (+0x18); the programs; then the render states. Completion: 100%.
fn particle_style_pass(
    context: &mut BsShaderContext,
    pass: PassId,
    vertex: &ProgramSlot,
    pixel: &ProgramSlot,
    states: &[(u32, u32)],
) {
    let mut stage = StageRef::new();
    if context.pools.pass(pass).stage_count == 0 {
        append_default_stage(context, pass, &mut stage, 0, 1, 2);
    }
    bind_programs(context, pass, vertex, pixel);
    render_states(context, pass, states);
    stage.reset(context);
}

// IDA: ParticleShader (0x148, vtable A91E4C) 007E2D90(declaration): base
// 00802540, +0x84 pass, +0x88/+0x8C programs = 0, +0x118..+0x144 = 0.0, byte
// +0x20 = 1, +0x7C = 0x17E, +0x24 the declaration, +0xB0 0.0, +0xF4..+0x144 =
// 0 (Velocity +0xF4, Acceleration +0x100, Color1..3 +0x118/+0x128/+0x138),
// 00B4600C (when 0) = 40 below render path 2 else 120, +0x80 FormHeap(32 *
// count) zeroed, +0xB4 64 bytes (fVars) zeroed.
#[derive(Default)]
pub struct ParticleShader {
    pub base: BsShaderObject,
    pub field_7c: u32,
    pub particle_data: Vec<[f32; 8]>, // +0x80
    pub particle_pass: PassSlot,      // +0x84
    pub vertex_program: ProgramSlot,  // +0x88
    pub pixel_program: ProgramSlot,   // +0x8C
    pub field_b0: f32,
    pub variables: [f32; 16], // +0xB4 fVars
    pub floats_f4: Vec<f32>,  // +0xF4..+0x144
}
bs_shader_class!(ParticleShader, base, BsShaderObject);

// IDA: 007E2D90 the constructor tail (after the declaration). Completion: 100%.
fn construct_particle_shader(context: &mut BsShaderContext, shader: &mut ParticleShader) {
    shader.original_vtable = 0x00A9_1E4C;
    shader.class_name = "ParticleShader";
    shader.field_20 = 1;
    shader.field_7c = 0x17E;
    shader.field_b0 = 0.0;
    shader.floats_f4 = vec![0.0; 21];
    let render_path = context.render_path;
    let count = &mut context.globals.particle_count_b4600c;
    if *count == 0 {
        *count = if render_path < 2 { 40 } else { 120 };
    }
    shader.particle_data = vec![[0.0; 8]; *count as usize];
    shader.variables = [0.0; 16];
}

// IDA: 007E3590 vt+0xA8: PARTICLE.vso (vs_2_0) into +0x88, PARTICLE.pso
// (ps_1_3; ps_2_0 from render path 2) into +0x8C. Completion: 100%.
fn particle_load_programs(context: &mut BsShaderContext, shader: &mut ParticleShader) {
    shader.vertex_program = load_vertex(
        context,
        &descriptor("particle\\v\\particle.v.hlsl", &[]),
        "vs_2_0",
        "PARTICLE.vso",
    );
    let profile = if context.render_path >= 2 {
        "ps_2_0"
    } else {
        "ps_1_3"
    };
    shader.pixel_program = load_pixel(
        context,
        &descriptor("particle\\p\\particle.p.hlsl", &[]),
        profile,
        "PARTICLE.pso",
    );
}

// IDA: 007E3D50 vt+0xAC: a pool pass into +0x84 when null, then 007E3730:
// COLORWRITEENABLE 0xF, ALPHABLENDENABLE 1, SRCBLEND 5, DESTBLEND 6,
// ALPHATESTENABLE 0, STENCILENABLE 0, ZENABLE 1, ZFUNC 4, ZWRITEENABLE 0.
// Completion: 100%.
fn particle_setup_passes(context: &mut BsShaderContext, shader: &mut ParticleShader) {
    if shader.particle_pass.is_none() {
        acquire_pass_into(context, &mut shader.particle_pass);
    }
    particle_style_pass(
        context,
        shader.particle_pass.expect("set above"),
        &shader.vertex_program,
        &shader.pixel_program,
        &[
            (0xA8, 0xF),
            (0x1B, 1),
            (0x13, 5),
            (0x14, 6),
            (0x0F, 0),
            (0x34, 0),
            (7, 1),
            (0x17, 4),
            (0x0E, 0),
        ],
    );
}

// IDA: 007E3160 vt+0x88: +0x30 WorldViewProjTranspose (c0), Color1..3
// (c8..c10, this+0x118/+0x128/+0x138), Velocity (c11, 12 bytes, this+0xF4),
// Acceleration (c12, 12 bytes, this+0x100), fVars (c4, 4 registers, 64
// bytes, this+0xB4), particle data (c15, 2 * 00B4600C registers, 32 * count
// bytes, the +0x80 buffer); then an empty +0x2C pixel map. Completion: 100%.
fn particle_constant_maps(context: &mut BsShaderContext, shader: &mut ParticleShader) {
    let render_path = context.render_path;
    let count = &mut context.globals.particle_count_b4600c;
    shader.vertex_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.vertex_constant_map,
        false,
        |map| {
            add_predefined(map, "WorldViewProjTranspose", 0, 0, 4);
            add_float4_constant(map, "Color1", 8, "this+0x118");
            add_float4_constant(map, "Color2", 9, "this+0x128");
            add_float4_constant(map, "Color3", 10, "this+0x138");
            add_constant(map, "Velocity", 0x1000_0007, 11, 1, 12, "this+0xF4");
            add_constant(map, "Acceleration", 0x1000_0007, 12, 1, 12, "this+0x100");
            add_matrix_constant(map, "fVars", 4, 4, 64, "this+0xB4");
            if *count == 0 {
                *count = if render_path < 2 { 40 } else { 120 };
            }
            let particles = *count as u32;
            add_matrix_constant(
                map,
                "particle data",
                15,
                2 * particles,
                32 * particles,
                "[this+0x80]",
            );
        },
    );
    shader.pixel_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.pixel_constant_map,
        true,
        |_| {},
    );
}

// IDA: 007E33A0 slot 22: the declaration (5 entries, 1 stream) with vt+0x50
// (0,0,0,2,0) (1,3,3,2,0) (2,5,4,3,0) (3,7,5,1,0) and vt+0x4C (0, 4, 6,
// 0x80000000, 0, 10, 1, 0); FormHeap(0x148) 007E2D90(declaration); vt+0x84,
// vt+0xA8, vt+0xAC, vt+0x88; +4 = shader; vt+0x54. Completion: 100%.
pub fn create_particle_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_bs_shader_slot::<ParticleShader>(
        context,
        5,
        1,
        |declaration| {
            set_entries(
                declaration,
                &[
                    (0, 0, 0, 2, 0),
                    (1, 3, 3, 2, 0),
                    (2, 5, 4, 3, 0),
                    (3, 7, 5, 1, 0),
                ],
            );
            set_entry(declaration, 0, 4, 6, 0x8000_0000, 0, 10, 1);
        },
        false,
        |shader, context, declaration| {
            shader.declaration.clone_from(declaration);
            construct_particle_shader(context, shader);
        },
        |shader, context| {
            particle_load_programs(context, shader);
            particle_setup_passes(context, shader);
            particle_constant_maps(context, shader);
        },
        false,
    )
}

// IDA: BoltShader (0x1A4, vtable A92D8C) 007F3890(declaration): base
// 00802540, +0x178 pass, +0x19C/+0x1A0 programs, byte +0x20 = 1, +0x7C =
// 0x17F, +0x24 the declaration, +0x84 0.0, +0x128 EyePosition = 0,
// +0x17C CoreColor and +0x18C GlowColor = (1, 1, 1, 1) (00B25AE0), 00B468FC
// (when 0) = 75 below render path 2 else 235, +0x80 FormHeap(16 * 4 *
// ceil((float)(count * 0.25))) zeroed, +0x138 64 bytes (fVars) and +0x88 0xA0
// bytes zeroed.
#[derive(Default)]
pub struct BoltShader {
    pub base: BsShaderObject,
    pub field_7c: u32,
    pub bolt_data: Vec<[f32; 4]>, // +0x80
    pub field_84: f32,
    pub bytes_88: Vec<u8>,           // +0x88
    pub eye_position: [f32; 4],      // +0x128
    pub variables: [f32; 16],        // +0x138 fVars
    pub bolt_pass: PassSlot,         // +0x178
    pub core_color: [f32; 4],        // +0x17C
    pub glow_color: [f32; 4],        // +0x18C
    pub vertex_program: ProgramSlot, // +0x19C
    pub pixel_program: ProgramSlot,  // +0x1A0
}
bs_shader_class!(BoltShader, base, BsShaderObject);

// IDA: 007F3890 the constructor tail. The quarter count is the x87 product
// (double 0.25) stored to a float before _ceil and the truncating fistp.
// Completion: 100%.
fn construct_bolt_shader(context: &mut BsShaderContext, shader: &mut BoltShader) {
    shader.original_vtable = 0x00A9_2D8C;
    shader.class_name = "BoltShader";
    shader.field_20 = 1;
    shader.field_7c = 0x17F;
    shader.field_84 = 0.0;
    shader.eye_position = [0.0; 4];
    shader.core_color = [1.0; 4];
    shader.glow_color = [1.0; 4];
    let render_path = context.render_path;
    let count = &mut context.globals.bolt_count_b468fc;
    if *count == 0 {
        *count = if render_path < 2 { 75 } else { 235 };
    }
    let quarter = stored_float(f64::from(*count) * 0.25);
    let groups = f64::from(quarter).ceil() as i64;
    shader.bolt_data = vec![[0.0; 4]; (4 * groups) as usize];
    shader.variables = [0.0; 16];
    shader.bytes_88 = vec![0; 0xA0];
}

// IDA: 007F3FF0 vt+0xA8: BOLT.vso (vs_1_1) into +0x19C, BOLT.pso (ps_1_3;
// ps_2_0 from render path 2) into +0x1A0. Completion: 100%.
fn bolt_load_programs(context: &mut BsShaderContext, shader: &mut BoltShader) {
    shader.vertex_program = load_vertex(
        context,
        &descriptor("bolt\\v\\bolt.v.hlsl", &[]),
        "vs_1_1",
        "BOLT.vso",
    );
    let profile = if context.render_path >= 2 {
        "ps_2_0"
    } else {
        "ps_1_3"
    };
    shader.pixel_program = load_pixel(
        context,
        &descriptor("bolt\\p\\bolt.p.hlsl", &[]),
        profile,
        "BOLT.pso",
    );
}

// IDA: 007F46E0 vt+0xAC: a pool pass into +0x178 when null, then 007F4190:
// ALPHAFUNC 1, COLORWRITEENABLE 0xF, ALPHABLENDENABLE 1, SRCBLEND 5,
// DESTBLEND 2, ALPHATESTENABLE 0, STENCILENABLE 0, ZENABLE 1, ZFUNC 4,
// ZWRITEENABLE 0. Completion: 100%.
fn bolt_setup_passes(context: &mut BsShaderContext, shader: &mut BoltShader) {
    if shader.bolt_pass.is_none() {
        acquire_pass_into(context, &mut shader.bolt_pass);
    }
    particle_style_pass(
        context,
        shader.bolt_pass.expect("set above"),
        &shader.vertex_program,
        &shader.pixel_program,
        &[
            (0x16, 1),
            (0xA8, 0xF),
            (0x1B, 1),
            (0x13, 5),
            (0x14, 2),
            (0x0F, 0),
            (0x34, 0),
            (7, 1),
            (0x17, 4),
            (0x0E, 0),
        ],
    );
}

// IDA: 007F3C10 vt+0x88: +0x30 WorldViewProjTranspose (c0), fVars (c4, 4
// registers, 64 bytes, this+0x138), EyePosition (c8, this+0x128), bolt data
// (c14, 00B468FC registers, 16 * count bytes, the +0x80 buffer); +0x2C
// CoreColor (c0, this+0x17C), GlowColor (c1, this+0x18C). Completion: 100%.
fn bolt_constant_maps(context: &mut BsShaderContext, shader: &mut BoltShader) {
    let render_path = context.render_path;
    let count = &mut context.globals.bolt_count_b468fc;
    shader.vertex_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.vertex_constant_map,
        false,
        |map| {
            add_predefined(map, "WorldViewProjTranspose", 0, 0, 4);
            add_matrix_constant(map, "fVars", 4, 4, 64, "this+0x138");
            add_float4_constant(map, "EyePosition", 8, "this+0x128");
            if *count == 0 {
                *count = if render_path < 2 { 75 } else { 235 };
            }
            let bolts = *count as u32;
            add_matrix_constant(map, "bolt data", 14, bolts, 16 * bolts, "[this+0x80]");
        },
    );
    shader.pixel_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.pixel_constant_map,
        true,
        |map| {
            add_float4_constant(map, "CoreColor", 0, "this+0x17C");
            add_float4_constant(map, "GlowColor", 1, "this+0x18C");
        },
    );
}

// IDA: 007F3E00 slot 23: the ParticleShader declaration shape; FormHeap(0x1A4)
// 007F3890(declaration); vt+0x84, vt+0xA8, vt+0xAC, vt+0x88; +4; vt+0x54.
// Completion: 100%.
pub fn create_bolt_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_bs_shader_slot::<BoltShader>(
        context,
        5,
        1,
        |declaration| {
            set_entries(
                declaration,
                &[
                    (0, 0, 0, 2, 0),
                    (1, 3, 3, 2, 0),
                    (2, 5, 4, 3, 0),
                    (3, 7, 5, 1, 0),
                ],
            );
            set_entry(declaration, 0, 4, 6, 0x8000_0000, 0, 10, 1);
        },
        false,
        |shader, context, declaration| {
            shader.declaration.clone_from(declaration);
            construct_bolt_shader(context, shader);
        },
        |shader, context| {
            bolt_load_programs(context, shader);
            bolt_setup_passes(context, shader);
            bolt_constant_maps(context, shader);
        },
        false,
    )
}

// IDA: PrecipitationShader (0xBC, vtable A925F4) 007EE800: base 00802540,
// +0xAC = 0, +0xA0..+0xA8 (params) = 0.0, +0xB0..+0xB8 (ambient color) =
// (1, 1, 1) (00B25AC4), byte +0x20 = 1. Its programs and pass are the
// globals 00B466E0[4], 00B46708[2] and 00B46704.
#[derive(Default)]
pub struct PrecipitationShader {
    pub base: BsShaderObject,
    pub velocity: [f32; 3],   // +0x7C
    pub minimum: [f32; 3],    // +0x88
    pub maximum: [f32; 3],    // +0x94
    pub parameters: [f32; 3], // +0xA0
    pub field_ac: u32,
    pub ambient_color: [f32; 3], // +0xB0
}
bs_shader_class!(PrecipitationShader, base, BsShaderObject);

// IDA: 007EF5C0 vt+0x84: 008025F0, then 00B466E0[0..3] and 00B46708[0..1]
// released and the 00B46704 pass released (all null on the first call).
// Completion: 100%.
fn precipitation_initialize_tail(context: &mut BsShaderContext) {
    let globals = &mut context.globals;
    globals.precipitation_vertex_b466e0.fill(None);
    globals.precipitation_pixel_b46708.fill(None);
    assign_pass(
        &mut context.pools,
        &mut globals.precipitation_pass_b46704,
        None,
    );
}

// IDA: 007FAA90 vt+0xA8 -> vt+0xAC 007EF000 (PRECIP%03i.vso 0..3 with
// Renderer_GetVertexShaderTarget() into 00B466E0) and vt+0xB0 007EF1C0
// (PRECIP%03i.pso 0..1 with Renderer_GetPixelShaderTarget(0) into
// 00B46708). Completion: 100%.
fn precipitation_load_programs(context: &mut BsShaderContext) {
    const V: &str = "precipitation\\precipitation.v.hlsl";
    const P: &str = "precipitation\\precipitation.p.hlsl";
    let vertex = [
        descriptor(V, &[("BILLBOARD_UP", None)]),
        descriptor(V, &[("BILLBOARD_FACE", None)]),
        descriptor(V, &[("BILLBOARD_UP", None), ("SNOW", None)]),
        descriptor(V, &[("BILLBOARD_FACE", None), ("SNOW", None)]),
    ];
    let pixel = [descriptor(P, &[]), descriptor(P, &[("SNOW", None)])];
    for index in 0..4 {
        context.globals.precipitation_vertex_b466e0[index] = load_vertex(
            context,
            &vertex[index],
            vertex_target(context),
            &program_name("PRECIP%03i.vso", index),
        );
    }
    for index in 0..2 {
        context.globals.precipitation_pixel_b46708[index] = load_pixel(
            context,
            &pixel[index],
            pixel_target(context),
            &program_name("PRECIP%03i.pso", index),
        );
    }
}

// IDA: 007EF2F0 vt+0xB4: a pool pass into 00B46704, stage 0 =
// 00801110(0, 3, 2) then 00771640(1), vertex 00B466E0[0], pixel
// 00B46708[0], ZENABLE 1, ZWRITEENABLE 0, ZFUNC 4, ALPHABLENDENABLE 1,
// SRCBLEND 5, DESTBLEND 6, ALPHATESTENABLE 0, COLORWRITEENABLE 7. Completion: 100%.
fn precipitation_setup_passes(context: &mut BsShaderContext) {
    let mut slot = context.globals.precipitation_pass_b46704;
    acquire_pass_into(context, &mut slot);
    context.globals.precipitation_pass_b46704 = slot;
    let pass = slot.expect("just acquired");
    let mut stage = StageRef::new();
    append_filtered_stage(context, pass, &mut stage, 0, 3, 2, 1);
    let vertex = context.globals.precipitation_vertex_b466e0[0].clone();
    let pixel = context.globals.precipitation_pixel_b46708[0].clone();
    bind_programs(context, pass, &vertex, &pixel);
    render_states(
        context,
        pass,
        &[
            (7, 1),
            (0x0E, 0),
            (0x17, 4),
            (0x1B, 1),
            (0x13, 5),
            (0x14, 6),
            (0x0F, 0),
            (0xA8, 7),
        ],
    );
    stage.reset(context);
}

// IDA: 007EE9C0 vt+0x88: +0x2C ambient color (0x10000006, c0, 12 bytes,
// this+0xB0); +0x30 WorldViewProjTranspose (c0), eye pos (c8, 00B46710),
// velocity (c9, this+0x7C), "min" (c10, this+0x88), "max" (c11, this+0x94),
// params (c12, this+0xA0), cam up (c13, 00B466F8), all 0x10000006 with 12
// bytes. Completion: 100%.
fn precipitation_constant_maps(context: &mut BsShaderContext, shader: &mut PrecipitationShader) {
    shader.pixel_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.pixel_constant_map,
        true,
        |map| {
            add_constant(map, "ambient color", 0x1000_0006, 0, 1, 12, "this+0xB0");
        },
    );
    shader.vertex_constant_map = build_constant_map(
        &mut context.constant_maps,
        shader.vertex_constant_map,
        false,
        |map| {
            add_predefined(map, "WorldViewProjTranspose", 0, 0, 4);
            add_constant(map, "eye pos", 0x1000_0006, 8, 1, 12, "00B46710");
            add_constant(map, "velocity", 0x1000_0006, 9, 1, 12, "this+0x7C");
            add_constant(map, "min", 0x1000_0006, 10, 1, 12, "this+0x88");
            add_constant(map, "max", 0x1000_0006, 11, 1, 12, "this+0x94");
            add_constant(map, "params", 0x1000_0006, 12, 1, 12, "this+0xA0");
            add_constant(map, "cam up", 0x1000_0006, 13, 1, 12, "00B466F8");
        },
    );
}

// IDA: 007EEE40 slot 27: the declaration (3 entries, 2 streams) with vt+0x50
// (0,0,0,2,0) (1,7,5,1,0) and vt+0x4C (1, 0, 6, 0x80000000, 2, 5, 1, 0);
// FormHeap(0xBC) 007EE800; vt+0x84 007EF5C0, vt+0xA8 007FAA90, vt+0xB4
// 007EF2F0, vt+0x88 007EE9C0, vt+0x54, +4. Completion: 100%.
pub fn create_precipitation_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    create_bs_shader_slot::<PrecipitationShader>(
        context,
        3,
        2,
        |declaration| {
            set_entries(declaration, &[(0, 0, 0, 2, 0), (1, 7, 5, 1, 0)]);
            set_entry(declaration, 1, 0, 6, 0x8000_0000, 2, 5, 1);
        },
        false,
        |shader, _, _| {
            shader.original_vtable = 0x00A9_25F4;
            shader.class_name = "PrecipitationShader";
            shader.field_ac = 0;
            shader.parameters = [0.0; 3];
            shader.ambient_color = [1.0; 3];
            shader.field_20 = 1;
        },
        |shader, context| {
            precipitation_initialize_tail(context);
            precipitation_load_programs(context);
            precipitation_setup_passes(context);
            precipitation_constant_maps(context, shader);
        },
        true,
    )
}

// IDA: DebugShader (0xCC, vtable A92FCC) 007FA730: base 00802540, +0x7C
// sixteen NiPointers released, +0xBC (the screen elements), +0xC0/+0xC4
// (vertex/pixel programs) = 0, byte +0x20 = 1.
#[derive(Default)]
pub struct DebugShader {
    pub base: BsShaderObject,
    pub slots_7c: [u32; 16],                         // +0x7C NiPointers
    pub screen_elements: Option<ScreenElementsQuad>, // +0xBC without a `World`
    pub screen_elements_entity: Option<Entity>,      // +0xBC with one
    pub vertex_program: ProgramSlot,                 // +0xC0
    pub pixel_program: ProgramSlot,                  // +0xC4
}
bs_shader_class!(DebugShader, base, BsShaderObject);

// IDA: 007FA580 vt+0x84 tail (after 008025F0): FormHeap(0xC0) NiScreenElements
// over FormHeap(0x70) 00703230(0, 0, 1, 1, 1, 4, 1, 2, 1) into +0xBC;
// InsertPolygon(4, 0, null), SetRectangle(0, -1.0, 1.0, 2.0, -2.0),
// UpdateBound, SetRectangleUV(0, 0, 0.0, 0.0, 1.0, 1.0), 00707610
// RefreshPropertyState (no parent: a default NiPropertyState through vt+0x6C
// 00722B30), 00707370 Update(0.0, 1) (vt+0x60 00707580: no controllers, the
// identity world transform, world bound = model bound). With a `World` those two run on a NiScreenElements entity; without one
// the slot keeps the value and they are counted boundaries. 100% for the reached state; the screen elements are not drawn here.
fn debug_create_screen_elements(
    context: &mut BsShaderContext,
    shader: &mut DebugShader,
    world: Option<&mut World>,
) -> Result<(), StreamError> {
    let mut quad = ScreenElementsQuad::default();
    construct_screen_elements_data(&mut quad);
    let _ = insert_screen_polygon(&mut quad, 4);
    let _ = set_screen_rectangle(&mut quad, 0, -1.0, 1.0, 2.0, -2.0);
    update_screen_bound(&mut quad);
    let _ = set_screen_rectangle_uv(&mut quad, 0, 0, 0.0, 0.0, 1.0, 1.0);
    if let Some(world) = world {
        let defaults = world
            .get_resource::<NiProcessDefaults>()
            .ok_or(StreamError("00707610: no process property defaults"))?
            .scene_property_defaults();
        let entity = spawn_screen_elements(world, quad, NiAvProperties::default());
        refresh_property_state(world, entity, &defaults)?;
        update_screen_elements_world(world, entity);
        shader.screen_elements_entity = Some(entity);
    } else {
        shader.screen_elements = Some(quad);
        context.counted_boundaries.push(0x0070_7610);
        context.counted_boundaries.push(0x0070_7370);
    }
    Ok(())
}

// IDA: 007FA090: DEBUG.vso (base_old.v.hlsl, vs_1_1) into +0xC0, DEBUG.pso
// (copy.p.hlsl, ps_1_3) into +0xC4. Completion: 100%.
fn debug_load_programs(context: &mut BsShaderContext, shader: &mut DebugShader) {
    shader.vertex_program = load_vertex(
        context,
        &descriptor("imagespace\\1x\\v\\base_old.v.hlsl", &[]),
        "vs_1_1",
        "DEBUG.vso",
    );
    shader.pixel_program = load_pixel(
        context,
        &descriptor("imagespace\\1x\\p\\copy.p.hlsl", &[]),
        "ps_1_3",
        "DEBUG.pso",
    );
}

// IDA: 007FA220: a pool pass into +0x70 (replacing the default pass), stage
// 0 = 00801110(0, 3, 2) then 00771640(0), vertex +0xC0, pixel +0xC4, ZENABLE
// 0, ZWRITEENABLE 0, ALPHABLENDENABLE 0, SRCBLEND 2, DESTBLEND 2,
// COLORWRITEENABLE 0xF. Completion: 100%.
fn debug_setup_passes(context: &mut BsShaderContext, shader: &mut DebugShader) {
    let mut slot = shader.current_pass;
    acquire_pass_into(context, &mut slot);
    shader.current_pass = slot;
    let pass = slot.expect("just acquired");
    let mut stage = StageRef::new();
    append_filtered_stage(context, pass, &mut stage, 0, 3, 2, 0);
    bind_programs(context, pass, &shader.vertex_program, &shader.pixel_program);
    render_states(
        context,
        pass,
        &[
            (7, 0),
            (0x0E, 0),
            (0x1B, 0),
            (0x13, 2),
            (0x14, 2),
            (0xA8, 0xF),
        ],
    );
    stage.reset(context);
}

// IDA: 007FA910 slot 9: the two-entry declaration, FormHeap(0xCC) 007FA730,
// vt+0x84 007FA580, 007FA090, 007FA220, vt+0x54, +4. Completion: 100%.
pub fn create_debug_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    // Without a `World` the two calls are only counted, so nothing can fail.
    create_debug_slot_with(context, None).expect("no fallible step runs without a World")
}

// IDA: 007FA910, with the 00707610 and 00707370 boundaries run when `world` is given. The World is threaded through this one slot,
// the only class that reaches those calls.
// Completion: 100%.
pub fn create_debug_slot_with(
    context: &mut BsShaderContext,
    world: Option<&mut World>,
) -> Result<Arc<BsShaderSlot>, StreamError> {
    let mut failure = Ok(());
    let slot = create_bs_shader_slot::<DebugShader>(
        context,
        2,
        1,
        declare_position_texture,
        false,
        |shader, _, _| {
            shader.original_vtable = 0x00A9_2FCC;
            shader.class_name = "DebugShader";
            shader.field_20 = 1;
        },
        |shader, context| {
            failure = debug_create_screen_elements(context, shader, world);
            debug_load_programs(context, shader);
            debug_setup_passes(context, shader);
        },
        true,
    );
    failure.map(|()| slot)
}
