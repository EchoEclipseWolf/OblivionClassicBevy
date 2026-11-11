//! The lighting slot classes: Lighting30 (slot 26) and the ShadowLight family (slots 1, 4, 13, 14, 15).

// The loops are the originals': `index` also names the program and picks its slot.
#![allow(clippy::needless_range_loop)]

use std::sync::Arc;

use crate::bs::shader::{
    BsShaderContext, BsShaderObject, BsShaderSlot, BsShaderSlots, PassSlot, ProgramSlot,
    assign_pass, construct_bs_shader_base, get_bs_shader_slot, initialize_bs_shader,
};
use crate::bs::shader_classes_common::{
    PassArray, PassBuildTarget, ProgramRecord, StageRef, TableSet, acquire_pass_into, add_constant,
    add_float4_constant, add_map, add_matrix_constant, add_predefined, add_tree_constants,
    append_default_stage, bs_shader_class, build_constant_map, finish_slot, gcvt_12, load_pixel,
    load_vertex, make_declaration, pixel_target, program_name, programs, replay_map_ops,
    replay_pass_ops, set_entries, set_entry, vertex_target,
};
use crate::bs::shader_programs::ShaderProgramDescriptor;
use crate::bs::shader_tables::{
    BRANCH_OPS, BRANCH_PATH2_OPS, BRANCH_PIXEL1, BRANCH_VERTEX1, BRANCH_VERTEX2, HAIR_PIXEL1,
    HAIR_PIXEL2, HAIR_VERTEX1, HAIR_VERTEX2, LIGHTING30_OPS, LIGHTING30_PIXEL, LIGHTING30_VERTEX,
    PARALLAX_OPS, PARALLAX_PIXEL, PARALLAX_PIXEL5, PARALLAX_VERTEX, SHADOW_LIGHT_COMMON_OPS,
    SHADOW_LIGHT_MAP_OPS, SHADOW_LIGHT_PATH1_OPS, SHADOW_LIGHT_PATH2_OPS, SHADOW_LIGHT_PIXEL1,
    SHADOW_LIGHT_PIXEL2, SHADOW_LIGHT_VERTEX1, SHADOW_LIGHT_VERTEX1_S, SHADOW_LIGHT_VERTEX2,
    SKIN_OPS, SKIN_PIXEL, SKIN_VERTEX,
};
use crate::gamebryo::shader_runtime::{
    ConstantMapId, ShaderConstantMapEntry, ShaderDeclaration, add_predefined_constant_entry,
    create_shader_constant_map, find_shader_constant_entry,
};

type DeclarationSlot = Option<Arc<ShaderDeclaration>>;

// IDA: N/A a helper, with no address of its own.
// The vt+0x50 rows and vt+0x4C tangents the lighting declarations share.
fn base_four(declaration: &mut ShaderDeclaration) {
    set_entries(
        declaration,
        &[
            (0, 0, 0, 2, 0),
            (1, 3, 3, 2, 0),
            (2, 5, 4, 3, 0),
            (3, 7, 5, 1, 0),
        ],
    );
}
// IDA: N/A a helper, with no address of its own.
fn tangents(declaration: &mut ShaderDeclaration, first: u32) {
    set_entry(declaration, 0, first, 14, 0x8000_0000, 2, 6, 0);
    set_entry(declaration, 0, first + 1, 15, 0x8000_0001, 2, 7, 0);
}
// IDA: N/A a helper, with no address of its own.
fn skinned_six(declaration: &mut ShaderDeclaration) {
    set_entries(
        declaration,
        &[
            (0, 0, 0, 2, 0),
            (1, 1, 1, 3, 0),
            (2, 2, 2, 4, 0),
            (3, 3, 3, 2, 0),
            (4, 5, 4, 3, 0),
            (5, 7, 5, 1, 0),
        ],
    );
}

// IDA: the four ShadowLight/Lighting30 declarations (00771090 6/8/4/8
// entries): the shared vt+0x50/vt+0x4C shapes. Completion: 100%.
fn create_lighting_declarations(context: &BsShaderContext) -> [DeclarationSlot; 4] {
    [
        make_declaration(context, 6, 1, |declaration| {
            base_four(declaration);
            tangents(declaration, 4);
        }),
        make_declaration(context, 8, 1, |declaration| {
            skinned_six(declaration);
            tangents(declaration, 6);
        }),
        make_declaration(context, 4, 1, base_four),
        make_declaration(context, 8, 1, |declaration| {
            base_four(declaration);
            tangents(declaration, 4);
            set_entry(declaration, 0, 6, 8, 0x8000_0002, 3, 5, 1);
            set_entry(declaration, 0, 7, 9, 0x8000_0003, 3, 5, 2);
        }),
    ]
}

// IDA: Lighting30Shader (0xA0, vtable A930C4) 007FAC40(declarations 0..3):
// base 00802540, +0x7C..+0x98 = 0, +0x24 and +0x8C = declaration 0, +0x90..
// +0x98 = declarations 1..3, +0x7C/+0x80 (second pixel/vertex maps) and
// +0x84/+0x88 (map copies) released, +0x9C = 0; the globals in
// BSShaderGlobals::lighting30.
#[derive(Default)]
pub struct Lighting30Shader {
    pub base: BsShaderObject,
    pub second_pixel_map: Option<ConstantMapId>,  // +0x7C
    pub second_vertex_map: Option<ConstantMapId>, // +0x80
    pub pixel_map_copy: Option<ConstantMapId>,    // +0x84
    pub vertex_map_copy: Option<ConstantMapId>,   // +0x88
    pub declarations: [DeclarationSlot; 4],       // +0x8C
    pub pixel_constant_group: Option<Arc<ShaderConstantMapEntry>>, // +0x9C (raw)
}
bs_shader_class!(Lighting30Shader, base, BsShaderObject);

// IDA: 007FAC40 the constructor tail. Completion: 100%.
fn construct_lighting30(
    context: &mut BsShaderContext,
    shader: &mut Lighting30Shader,
    declarations: &[DeclarationSlot; 4],
) {
    shader.original_vtable = 0x00A9_30C4;
    shader.class_name = "Lighting30Shader";
    shader.declaration.clone_from(&declarations[0]);
    shader.declarations.clone_from(declarations);
    shader.pixel_constant_group = None;
    let render_path = context.render_path;
    let globals = &mut context.globals.lighting30;
    globals.fields_b46964 = [0; 8];
    globals.byte_b46984 = 0;
    globals.bytes_b46930.fill(0);
    if render_path < 7 {
        globals.max_lights_b2dcfc = 8;
    }
}

// IDA: 007FB250 vt+0x84: 008025F0, then 00B47288[32], 00B46ED8[39] and
// 00B46C20[39] released, the 54 passes 00B473D0..00B474A4 released, then
// 00B46CC8 and the rep movsd copies fill 33 vectors there and 49 from
// 00B46F78 with src__25 (0, 0, 0, 1). Completion: 100%.
fn lighting30_initialize_tail(context: &mut BsShaderContext) {
    let globals = &mut context.globals.lighting30;
    globals.vertex_b47288.fill(None);
    for index in 0..39 {
        globals.pixel_b46ed8[index] = None;
        globals.pixel_ll_b46c20[index] = None;
    }
    for pass in &mut globals.passes_b473d0 {
        assign_pass(&mut context.pools, pass, None);
    }
    globals.vectors_b46cc8.fill([0.0, 0.0, 0.0, 1.0]);
    globals.constants_b46f78.fill([0.0, 0.0, 0.0, 1.0]);
}

// IDA: 007FD560 vt+0xAC: SM3%03i.vso 0..31 with Renderer_GetVertexShaderTarget()
// into 00B47288. Completion: 100% with the generated table.
fn lighting30_load_vertex(context: &mut BsShaderContext) {
    let profile = vertex_target(context);
    for index in 0..32 {
        context.globals.lighting30.vertex_b47288[index] = load_vertex(
            context,
            &LIGHTING30_VERTEX[index].owned(),
            profile,
            &program_name("SM3%03i.vso", index),
        );
    }
}

// IDA: 007FDDE0 vt+0xB0: for each record 0..38 SM3%03i.pso into 00B46ED8 with
// the record's first definition (MAXLIGHTS) patched by 00404F00(0) (the render
// path): path 7: records below 20 take "20", records 2/3/12/13 the second
// definition "2", then records 0/2/4/8/12/14/18 "15", 1/5/6/9/11/15/16 "10",
// other records below 20 "8"; path 5: records 0/4..9/11/12/14..19 "7". Records
// below 20 then load SM3LL%03i.pso into 00B46C20 with the definition "4". The
// patches stay in the record (the original edits its stack table). Completion: 100%.
fn lighting30_load_pixel(context: &mut BsShaderContext) {
    let profile = pixel_target(context);
    let set_definition = |record: &mut ShaderProgramDescriptor, item: usize, value: &str| {
        if record.macros.len() <= item {
            record.macros.resize(item + 1, Default::default());
        }
        record.macros[item].definition = Some(value.to_owned());
    };
    for index in 0..39 {
        let mut record = LIGHTING30_PIXEL[index].owned();
        if context.render_path == 7 {
            if index < 20 {
                set_definition(&mut record, 0, "20");
            }
            if [2, 3, 12, 13].contains(&index) {
                set_definition(&mut record, 1, "2");
            }
            if [0, 2, 4, 8, 12, 14, 18].contains(&index) {
                set_definition(&mut record, 0, "15");
            } else if [1, 5, 6, 9, 11, 15, 16].contains(&index) {
                set_definition(&mut record, 0, "10");
            } else if index < 20 {
                set_definition(&mut record, 0, "8");
            }
        } else if context.render_path == 5
            && [0, 4, 5, 8, 11, 14, 15, 16, 17, 9, 12, 18, 19, 6, 7].contains(&index)
        {
            set_definition(&mut record, 0, "7");
        }
        context.globals.lighting30.pixel_b46ed8[index] = load_pixel(
            context,
            &record,
            profile,
            &program_name("SM3%03i.pso", index),
        );
        if index < 20 {
            set_definition(&mut record, 0, "4");
            context.globals.lighting30.pixel_ll_b46c20[index] = load_pixel(
                context,
                &record,
                profile,
                &program_name("SM3LL%03i.pso", index),
            );
        }
    }
}

// IDA: 007FEC30 vt+0xB4: 54 pool passes into 00B473D0..00B474A4, then
// 0085E660 replayed from kLighting30Ops (one shared local stage). 100% with
// the generated table.
fn lighting30_setup_passes(context: &mut BsShaderContext) {
    let mut passes = std::mem::take(&mut context.globals.lighting30.passes_b473d0);
    for pass in &mut passes {
        acquire_pass_into(context, pass);
    }
    context.globals.lighting30.passes_b473d0 = passes;
    let globals = &context.globals.lighting30;
    let target = PassBuildTarget {
        passes: PassArray::Lighting30,
        programs: [
            globals.vertex_b47288.clone(),
            globals.pixel_b46ed8.clone(),
            globals.pixel_ll_b46c20.clone(),
            Vec::new(),
        ],
        tables: TableSet::Lighting30,
    };
    let mut stage = StageRef::new();
    replay_pass_ops(context, &LIGHTING30_OPS, &target, &mut stage);
    stage.reset(context);
}

// IDA: 007FCC80 vt+0x88: +0x30 the predefined WorldViewProjTranspose (c0,
// vt+0x1C) -> the entry NiPointer 00B46B98, SkinWorldViewProjTranspose
// (c1) -> 00B46B9C, WorldViewTranspose (c5) -> 00B46BA4,
// SkinWorldViewTranspose (c6) -> 00B46BA8, BoneMatrix3 (vt+0x18, extra
// 0x120000, c31, 54) -> 00B46BA0, ConstantGroup (c10, 21 registers, 336 bytes,
// 00B46D68), decal fade (c32, 8, 128, 00B46218) -> 00B46C14, decal proj (c40,
// 32, 512, 00B46298) -> 00B46C18; +0x2C ConstantGroup (c0, 49, 784,
// 00B46F78) -> +0x9C (raw); +0x88 = +0x30 and +0x84 = +0x2C; a second vertex
// map +0x80 (WorldViewProjTranspose c0 and SkinWorldViewProjTranspose c4
// predefined, BoneMatrix3 c14, EyePosition c8 00B46DA8, U/V Offset c9/c10
// 00B46B3C/00B46B40, InvBoundDiameter c11 00B46B44 (4 bytes each), FogParam
// c12 00B46B88, FogColor c13 00B46B78) and a second pixel map +0x7C (Fill
// Color c0 00B46B48, Rim Color c1 00B46B58, Vars c2 00B46B68). Completion: 100%.
fn lighting30_constant_maps(context: &mut BsShaderContext, shader: &mut Lighting30Shader) {
    let globals = &mut context.globals.lighting30;
    let vertex = shader.vertex_constant_map;
    shader.vertex_constant_map =
        build_constant_map(&mut context.constant_maps, vertex, false, |map| {
            let mut predefined = |key: &str, reg: u32| {
                let _ = add_predefined_constant_entry(map, key, 0x2000_0009, reg, "");
                find_shader_constant_entry(map, key)
            };
            globals.entry_b46b98 = predefined("WorldViewProjTranspose", 0);
            globals.entry_b46b9c = predefined("SkinWorldViewProjTranspose", 1);
            globals.entry_b46ba4 = predefined("WorldViewTranspose", 5);
            globals.entry_b46ba8 = predefined("SkinWorldViewTranspose", 6);
            add_predefined(map, "BoneMatrix3", 0x12_0000, 31, 54);
            globals.entry_b46ba0 = find_shader_constant_entry(map, "BoneMatrix3");
            add_matrix_constant(map, "ConstantGroup", 10, 21, 336, "00B46D68");
            add_matrix_constant(map, "decal fade", 32, 8, 128, "00B46218");
            globals.entry_b46c14 = find_shader_constant_entry(map, "decal fade");
            add_matrix_constant(map, "decal proj", 40, 32, 512, "00B46298");
            globals.entry_b46c18 = find_shader_constant_entry(map, "decal proj");
        });
    let pixel = shader.pixel_constant_map;
    shader.pixel_constant_map =
        build_constant_map(&mut context.constant_maps, pixel, true, |map| {
            add_matrix_constant(map, "ConstantGroup", 0, 49, 784, "00B46F78");
            shader.pixel_constant_group = find_shader_constant_entry(map, "ConstantGroup");
        });
    shader.vertex_map_copy = shader.vertex_constant_map;
    shader.pixel_map_copy = shader.pixel_constant_map;
    shader.second_vertex_map = build_constant_map(
        &mut context.constant_maps,
        shader.second_vertex_map,
        false,
        |map| {
            let _ =
                add_predefined_constant_entry(map, "WorldViewProjTranspose", 0x2000_0009, 0, "");
            let _ = add_predefined_constant_entry(
                map,
                "SkinWorldViewProjTranspose",
                0x2000_0009,
                4,
                "",
            );
            add_predefined(map, "BoneMatrix3", 0x12_0000, 14, 54);
            add_float4_constant(map, "EyePosition", 8, "00B46DA8");
            add_constant(map, "U Offset", 0x1000_0004, 9, 1, 4, "00B46B3C");
            add_constant(map, "V Offset", 0x1000_0004, 10, 1, 4, "00B46B40");
            add_constant(map, "InvBoundDiameter", 0x1000_0004, 11, 1, 4, "00B46B44");
            add_float4_constant(map, "FogParam", 12, "00B46B88");
            add_float4_constant(map, "FogColor", 13, "00B46B78");
        },
    );
    shader.second_pixel_map = build_constant_map(
        &mut context.constant_maps,
        shader.second_pixel_map,
        true,
        |map| {
            add_float4_constant(map, "Fill Color", 0, "00B46B48");
            add_float4_constant(map, "Rim Color", 1, "00B46B58");
            add_float4_constant(map, "Vars", 2, "00B46B68");
        },
    );
}

// IDA: 007FC7D0 slot 26 (g_rendererShaders30): four declarations from
// 00771090: +0 (6 entries) vt+0x50 (0,0,0,2,0) (1,3,3,2,0) (2,5,4,3,0)
// (3,7,5,1,0), vt+0x4C (0,4,14,0x80000000,2,6,0,0) (0,5,15,0x80000001,2,7,0,0);
// the second (8 entries) vt+0x50 (0,0,0,2,0) (1,1,1,3,0) (2,2,2,4,0)
// (3,3,3,2,0) (4,5,4,3,0) (5,7,5,1,0), vt+0x4C (0,6,14,0x80000000,2,6,0,0)
// (0,7,15,0x80000001,2,7,0,0); the third (4 entries) the +0 vt+0x50 four; the
// fourth (8 entries) those four and vt+0x4C (0,4,14,..) (0,5,15,..)
// (0,6,8,0x80000002,3,5,1,0) (0,7,9,0x80000003,3,5,2,0); FormHeap(0xA0)
// 007FAC40(the four); vt+0x84 007FB250, vt+0xA8 007FAA90 (vt+0xAC 007FD560,
// vt+0xB0 007FDDE0), vt+0xB4 007FEC30, vt+0x88 007FCC80; +4; vt+0x54. Completion: 100%.
pub fn create_lighting30_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    let declarations = create_lighting_declarations(context);
    let slot = BsShaderSlot {
        declaration: declarations[0].clone(),
        shader: None,
    };
    let mut shader = Lighting30Shader::default();
    construct_bs_shader_base(&mut shader, false);
    construct_lighting30(context, &mut shader, &declarations);
    let _ = initialize_bs_shader(context, &mut shader);
    lighting30_initialize_tail(context);
    lighting30_load_vertex(context);
    lighting30_load_pixel(context);
    lighting30_setup_passes(context);
    lighting30_constant_maps(context, &mut shader);
    finish_slot(slot, shader, false)
}

// IDA: ShadowLightShader (0x9C, vtable A90634) 007C88B0(declarations 0..3):
// base 00802540, +0x7C..+0x98 = 0, +0x24 and +0x7C = declaration 0, +0x80..
// +0x88 = declarations 1..3, +0x90/+0x8C/+0x98/+0x94 (maps) released, +0x74
// = -1. Its programs, passes, per-pass tables and map entries are the
// globals in BSShaderGlobals::shadow_light.
#[derive(Default)]
pub struct ShadowLightShader {
    pub base: BsShaderObject,
    pub field_74: i32,
    pub declarations: [DeclarationSlot; 4],       // +0x7C
    pub second_pixel_map: Option<ConstantMapId>,  // +0x8C
    pub second_vertex_map: Option<ConstantMapId>, // +0x90
    pub pixel_map_copy: Option<ConstantMapId>,    // +0x94
    pub vertex_map_copy: Option<ConstantMapId>,   // +0x98
}
bs_shader_class!(ShadowLightShader, base, BsShaderObject);

// IDA: 007C88B0 the constructor tail. Completion: 100%.
fn construct_shadow_light(shader: &mut ShadowLightShader, declarations: &[DeclarationSlot; 4]) {
    shader.original_vtable = 0x00A9_0634;
    shader.class_name = "ShadowLightShader";
    shader.declaration.clone_from(&declarations[0]);
    shader.declarations.clone_from(declarations);
    shader.field_74 = -1;
}

// IDA: 007C8F50 vt+0x84 tail (after 008025F0): 00B45290[129] and
// 00B45088[129] released, the 419 passes released, the map-entry NiPointers
// 00B45018..00B45087 and 00B45518..00B4555B released, the four 0x68C-byte
// per-pass tables zeroed. Completion: 100%.
fn shadow_light_initialize_tail(context: &mut BsShaderContext) {
    let globals = &mut context.globals.shadow_light;
    globals.vertex_b45290.fill(None);
    globals.pixel_b45088.fill(None);
    for pass in &mut globals.passes_b455a0 {
        assign_pass(&mut context.pools, pass, None);
    }
    globals.entries.retain(|&address, _| {
        !((0x00B4_5018..0x00B4_5088).contains(&address)
            || (0x00B4_5518..0x00B4_555C).contains(&address))
    });
    for table in &mut globals.tables {
        table.fill(0);
    }
}

// IDA: the run-time macro values of the lighting loaders: "@SMCLASS@" =
// _itoa(00B42E88, 10), "@SPEC@" = _gcvt(00B2C2BC, 12), "@EMIS@" =
// _gcvt(00B2C2C0, 12).
fn with_runtime_macros(
    context: &BsShaderContext,
    record: &ProgramRecord,
) -> ShaderProgramDescriptor {
    let mut copy = record.owned();
    for item in &mut copy.macros {
        match item.definition.as_deref() {
            Some("@SMCLASS@") => item.definition = Some(context.shader_model_class.to_string()),
            Some("@SPEC@") => item.definition = Some(gcvt_12(context.specular_multiplier)),
            Some("@EMIS@") => item.definition = Some(gcvt_12(context.emissive_multiplier)),
            _ => {}
        }
    }
    copy
}

// IDA: the SLS record loops: a null path is skipped (SLS1/SLS1S) or releases
// the slot (SLS2); `limit` stops loading at a record index (the pixel SLS2
// records 78..81 load only from render path 5).
#[allow(clippy::too_many_arguments)] // The original's table, target and formats.
fn load_record_range(
    context: &mut BsShaderContext,
    table: &[ProgramRecord],
    slots: &mut [ProgramSlot],
    vertex: bool,
    profile: &str,
    format: &str,
    release_null: bool,
    limit: usize,
) {
    for index in 0..table.len() {
        if table[index].is_null() {
            if release_null {
                slots[index] = None;
            }
            continue;
        }
        if index >= limit {
            continue;
        }
        let record = with_runtime_macros(context, &table[index]);
        let name = program_name(format, index);
        slots[index] = if vertex {
            load_vertex(context, &record, profile, &name)
        } else {
            load_pixel(context, &record, profile, &name)
        };
    }
}

// IDA: 007CD090 vt+0xAC: SLS1%03i.vso 0..30 and SLS1S%03i.vso 0..21 (vs_1_1)
// into 00B45290[0..52]; from render path 2 SLS2%03i.vso 0..75 (vs_2_0) into
// [53..128]. The SLS1/SLS1S loops skip null records; SLS2 releases them.
// Completion: 100% with the generated tables.
fn shadow_light_load_vertex(context: &mut BsShaderContext) {
    let mut slots = std::mem::take(&mut context.globals.shadow_light.vertex_b45290);
    load_record_range(
        context,
        &SHADOW_LIGHT_VERTEX1,
        &mut slots,
        true,
        "vs_1_1",
        "SLS1%03i.vso",
        false,
        usize::MAX,
    );
    load_record_range(
        context,
        &SHADOW_LIGHT_VERTEX1_S,
        &mut slots[31..],
        true,
        "vs_1_1",
        "SLS1S%03i.vso",
        false,
        usize::MAX,
    );
    if context.render_path >= 2 {
        load_record_range(
            context,
            &SHADOW_LIGHT_VERTEX2,
            &mut slots[53..],
            true,
            "vs_2_0",
            "SLS2%03i.vso",
            true,
            usize::MAX,
        );
    }
    context.globals.shadow_light.vertex_b45290 = slots;
}

// IDA: 007CEFD0 vt+0xB0: SLS1%03i.pso 0..46 (ps_2_0) into 00B45088[0..46];
// from render path 2 SLS2%03i.pso 0..81 into [47..128] (records 78..81 only
// from render path 5; null records release). Completion: 100% with the generated tables.
fn shadow_light_load_pixel(context: &mut BsShaderContext) {
    let mut slots = std::mem::take(&mut context.globals.shadow_light.pixel_b45088);
    load_record_range(
        context,
        &SHADOW_LIGHT_PIXEL1,
        &mut slots,
        false,
        "ps_2_0",
        "SLS1%03i.pso",
        false,
        usize::MAX,
    );
    if context.render_path >= 2 {
        let limit = if context.render_path >= 5 { 82 } else { 78 };
        load_record_range(
            context,
            &SHADOW_LIGHT_PIXEL2,
            &mut slots[47..],
            false,
            "ps_2_0",
            "SLS2%03i.pso",
            true,
            limit,
        );
    }
    context.globals.shadow_light.pixel_b45088 = slots;
}

// IDA: 007D1170 vt+0xB4: 419 pool passes into 00B455A0.., each with vertex
// program 00B45290[0]; then the common builders (00820910, 00814430,
// 00815DB0, 0081AA00, 0081B120, 0081BCE0, 0081D090, 0081DC40, 0081EA70,
// 008203D0), below render path 2 0081F330, else the eleven path-2 builders
// (0082D990 .. 00839A50). Completion: 100% with the generated tables.
fn shadow_light_setup_passes(context: &mut BsShaderContext) {
    let mut passes = std::mem::take(&mut context.globals.shadow_light.passes_b455a0);
    let first_vertex = context.globals.shadow_light.vertex_b45290[0].clone();
    for pass in &mut passes {
        acquire_pass_into(context, pass);
        let id = pass.expect("just acquired");
        context
            .pools
            .pass_mut(id)
            .vertex_shader
            .clone_from(&first_vertex);
    }
    context.globals.shadow_light.passes_b455a0 = passes;
    let globals = &context.globals.shadow_light;
    let target = PassBuildTarget {
        passes: PassArray::ShadowLight,
        programs: [
            globals.vertex_b45290.clone(),
            globals.pixel_b45088.clone(),
            Vec::new(),
            Vec::new(),
        ],
        tables: TableSet::ShadowLight,
    };
    let mut stage = StageRef::new();
    replay_pass_ops(context, &SHADOW_LIGHT_COMMON_OPS, &target, &mut stage);
    stage.reset(context);
    if context.render_path < 2 {
        replay_pass_ops(context, &SHADOW_LIGHT_PATH1_OPS, &target, &mut stage);
    } else {
        replay_pass_ops(context, &SHADOW_LIGHT_PATH2_OPS, &target, &mut stage);
    }
    stage.reset(context);
}

// IDA: 007CC280 vt+0x88 (kShadowLightMapOps). Completion: 100% with the generated table.
fn shadow_light_constant_maps(context: &mut BsShaderContext, shader: &mut ShadowLightShader) {
    let mut maps = [
        shader.vertex_constant_map,
        shader.pixel_constant_map,
        shader.second_vertex_map,
        shader.second_pixel_map,
        shader.vertex_map_copy,
        shader.pixel_map_copy,
    ];
    let mut entries = std::mem::take(&mut context.globals.shadow_light.entries);
    replay_map_ops(context, &SHADOW_LIGHT_MAP_OPS, &mut maps, &mut entries);
    context.globals.shadow_light.entries = entries;
    shader.vertex_constant_map = maps[0];
    shader.pixel_constant_map = maps[1];
    shader.second_vertex_map = maps[2];
    shader.second_pixel_map = maps[3];
    shader.vertex_map_copy = maps[4];
    shader.pixel_map_copy = maps[5];
}

// IDA: 007CBDC0 slot 1: the four declarations (as Lighting30); FormHeap(0x9C)
// 007C88B0(the four); vt+0x84 007C8F50, vt+0xA8 007FAA90 (vt+0xAC 007CD090,
// vt+0xB0 007CEFD0), vt+0xB4 007D1170, vt+0xB8 (00977C50, true), vt+0x88
// 007CC280; +4; vt+0x54. Completion: 100%.
pub fn create_shadow_light_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    let declarations = create_lighting_declarations(context);
    let slot = BsShaderSlot {
        declaration: declarations[0].clone(),
        shader: None,
    };
    let mut shader = ShadowLightShader::default();
    construct_bs_shader_base(&mut shader, false);
    construct_shadow_light(&mut shader, &declarations);
    let _ = initialize_bs_shader(context, &mut shader);
    shadow_light_initialize_tail(context);
    shadow_light_load_vertex(context);
    shadow_light_load_pixel(context);
    shadow_light_setup_passes(context);
    shadow_light_constant_maps(context, &mut shader);
    finish_slot(slot, shader, false)
}

// IDA: 00808DF0 / 0080CC90 vt+0x88 of Skin, Parallax and Hair: 007B4290(1)
// (the ShadowLight slot) and its shader's +0x30, +0x2C, +0x8C, +0x90, +0x94,
// +0x98 maps NiPointer-copied in that order. Completion: 100%.
fn copy_shadow_light_maps(
    context: &mut BsShaderContext,
    slots: &mut BsShaderSlots,
    shader: &mut ShadowLightShader,
) {
    let slot = get_bs_shader_slot(slots, 1, context);
    let Some(slot) = slot else { return };
    let source = slot
        .shader
        .as_ref()
        .and_then(|shader| shader.as_any().downcast_ref::<ShadowLightShader>());
    let Some(source) = source else { return };
    shader.vertex_constant_map = source.vertex_constant_map;
    shader.pixel_constant_map = source.pixel_constant_map;
    shader.second_pixel_map = source.second_pixel_map;
    shader.second_vertex_map = source.second_vertex_map;
    shader.pixel_map_copy = source.pixel_map_copy;
    shader.vertex_map_copy = source.vertex_map_copy;
}

// IDA: the two-declaration factories of Skin, Hair and Parallax (the first
// two lighting declarations: 6 and 8 entries). Completion: 100%.
fn create_two_lighting_declarations(context: &BsShaderContext) -> [DeclarationSlot; 4] {
    [
        make_declaration(context, 6, 1, |declaration| {
            base_four(declaration);
            set_entry(declaration, 0, 4, 14, 0x8000_0000, 2, 6, 0);
            set_entry(declaration, 0, 5, 15, 0x8000_0001, 2, 7, 0);
        }),
        make_declaration(context, 8, 1, |declaration| {
            skinned_six(declaration);
            set_entry(declaration, 0, 6, 14, 0x8000_0000, 2, 6, 0);
            set_entry(declaration, 0, 7, 15, 0x8000_0001, 2, 7, 0);
        }),
        None,
        None,
    ]
}

// IDA: N/A a helper, with no address of its own.
fn table_pass_target(
    passes: PassArray,
    first: Vec<ProgramSlot>,
    second: Vec<ProgramSlot>,
) -> PassBuildTarget {
    PassBuildTarget {
        passes,
        programs: [first, second, Vec::new(), Vec::new()],
        tables: TableSet::ShadowLight,
    }
}

// IDA: SkinShader (0x114, vtable A941AC) 0080AD40(declarations 0, 1): the
// ShadowLight base (0, 1, null, null), +0x9C twenty vertex and +0xEC ten
// pixel program NiPointers, +0x78 = 0, +0x7C/+0x80 the declarations, +0x74 =
// -1; its twenty passes are 00B47718...
#[derive(Default)]
pub struct SkinShader {
    pub shadow: ShadowLightShader,
    pub vertex_programs: Vec<ProgramSlot>, // +0x9C
    pub pixel_programs: Vec<ProgramSlot>,  // +0xEC
}
bs_shader_class!(SkinShader, shadow, ShadowLightShader);

// IDA: 0080AE90 slot 14: two declarations; FormHeap(0x114) 0080AD40; vt+0x84
// 008097D0 (008025F0, the programs and 00B47718[20] released), vt+0xAC
// 0080A180 (SKIN2%03i.vso, vs_2_0, from render path 2), vt+0xB0 0080A940
// (SKIN2%03i.pso, ps_2_0, from render path 2), vt+0xB4 0087AC50 (20 pool
// passes, then vt+0xC0 0087AD10), vt+0x88 00808DF0. Completion: 100%.
pub fn create_skin_slot(
    context: &mut BsShaderContext,
    slots: &mut BsShaderSlots,
) -> Arc<BsShaderSlot> {
    let declarations = create_two_lighting_declarations(context);
    let slot = BsShaderSlot {
        declaration: declarations[0].clone(),
        shader: None,
    };
    let mut skin = SkinShader {
        vertex_programs: programs(20),
        pixel_programs: programs(10),
        ..SkinShader::default()
    };
    construct_bs_shader_base(&mut skin, false);
    construct_shadow_light(&mut skin, &declarations);
    skin.original_vtable = 0x00A9_41AC;
    skin.class_name = "SkinShader";
    let _ = initialize_bs_shader(context, &mut skin);
    for pass in &mut context.globals.skin.passes_b47718 {
        assign_pass(&mut context.pools, pass, None);
    }
    if context.render_path >= 2 {
        load_record_range(
            context,
            &SKIN_VERTEX,
            &mut skin.vertex_programs,
            true,
            "vs_2_0",
            "SKIN2%03i.vso",
            false,
            usize::MAX,
        );
        load_record_range(
            context,
            &SKIN_PIXEL,
            &mut skin.pixel_programs,
            false,
            "ps_2_0",
            "SKIN2%03i.pso",
            false,
            usize::MAX,
        );
    }
    let mut passes = std::mem::take(&mut context.globals.skin.passes_b47718);
    for pass in &mut passes {
        acquire_pass_into(context, pass);
    }
    context.globals.skin.passes_b47718 = passes;
    let target = table_pass_target(
        PassArray::Skin,
        skin.vertex_programs.clone(),
        skin.pixel_programs.clone(),
    );
    let mut stage = StageRef::new();
    replay_pass_ops(context, &SKIN_OPS, &target, &mut stage);
    copy_shadow_light_maps(context, slots, &mut skin);
    stage.reset(context);
    finish_slot(slot, skin, false)
}

// IDA: HairShader (0xFC, vtable A942A4) 0080CF20(declarations 0, 1): the
// ShadowLight base, +0x9C two member passes, +0xA4 seven and +0xCC seven
// vertex, +0xC0 three and +0xE8 three pixel program NiPointers, +0xF4/+0xF8
// the declarations, +0x78 = 0, +0x24 = +0xF8 (declaration 1), +0x74 = -1.
#[derive(Default)]
pub struct HairShader {
    pub shadow: ShadowLightShader,
    pub hair_passes: [PassSlot; 2],              // +0x9C
    pub vertex1: [ProgramSlot; 7],               // +0xA4
    pub pixel1: [ProgramSlot; 3],                // +0xC0
    pub vertex2: [ProgramSlot; 7],               // +0xCC
    pub pixel2: [ProgramSlot; 3],                // +0xE8
    pub hair_declarations: [DeclarationSlot; 2], // +0xF4
}
bs_shader_class!(HairShader, shadow, ShadowLightShader);

// IDA: 0080C8B0 vt+0xB4: for each member pass a pool pass, stages (0, 1, 2),
// (1, 1, 2), (2, 1, 2), (3, 3, 2), (4, 1, 2) and from render path 2 (00404F00(0))
// (5, 3, 2), each on its own local NiPointer; then the ShadowLight tables
// 00B43EA0/00B44530 = 0x20082/0x10C and 00B43EA4/00B44534 = 0x23982/0x13C.
// Completion: 100%.
fn hair_setup_passes(context: &mut BsShaderContext, hair: &mut HairShader) {
    for slot in &mut hair.hair_passes {
        acquire_pass_into(context, slot);
        let pass = slot.expect("just acquired");
        let stages: [[u32; 3]; 5] = [[0, 1, 2], [1, 1, 2], [2, 1, 2], [3, 3, 2], [4, 1, 2]];
        let mut locals: [StageRef; 6] = Default::default();
        for (index, row) in stages.iter().enumerate() {
            append_default_stage(context, pass, &mut locals[index], row[0], row[1], row[2]);
        }
        if context.render_path >= 2 {
            append_default_stage(context, pass, &mut locals[0], 5, 3, 2);
        }
        // The array of local NiPointers leaves scope, last first.
        for local in locals.iter_mut().rev() {
            local.reset(context);
        }
    }
    let tables = &mut context.globals.shadow_light.tables;
    tables[1][224] = 0x2_0082;
    tables[2][224] = 0x10C;
    tables[1][225] = 0x2_3982;
    tables[2][225] = 0x13C;
}

// IDA: 0080D940 slot 13: two declarations; FormHeap(0xFC) 0080CF20; vt+0x84
// 0080B5D0 (the programs and member passes released), vt+0xAC 0080BFB0
// (HAIR1%03i.vso vs_1_1; HAIR2%03i.vso vs_2_0 from render path 2), vt+0xB0
// 0080C560 (HAIR1%03i.pso ps_1_3; HAIR2%03i.pso ps_2_0 from render path 2),
// vt+0xB4 0080C8B0, vt+0xB8 0080B2F0 (true), vt+0x88 0080CC90. Completion: 100%.
pub fn create_hair_slot(
    context: &mut BsShaderContext,
    slots: &mut BsShaderSlots,
) -> Arc<BsShaderSlot> {
    let declarations = create_two_lighting_declarations(context);
    let slot = BsShaderSlot {
        declaration: declarations[0].clone(),
        shader: None,
    };
    let mut hair = HairShader::default();
    construct_bs_shader_base(&mut hair, false);
    construct_shadow_light(&mut hair, &declarations);
    hair.original_vtable = 0x00A9_42A4;
    hair.class_name = "HairShader";
    hair.hair_declarations = [declarations[0].clone(), declarations[1].clone()];
    hair.declaration.clone_from(&declarations[1]);
    let _ = initialize_bs_shader(context, &mut hair);
    for pass in &mut hair.hair_passes {
        assign_pass(&mut context.pools, pass, None);
    }
    load_record_range(
        context,
        &HAIR_VERTEX1,
        &mut hair.vertex1,
        true,
        "vs_1_1",
        "HAIR1%03i.vso",
        false,
        usize::MAX,
    );
    if context.render_path >= 2 {
        load_record_range(
            context,
            &HAIR_VERTEX2,
            &mut hair.vertex2,
            true,
            "vs_2_0",
            "HAIR2%03i.vso",
            false,
            usize::MAX,
        );
    }
    load_record_range(
        context,
        &HAIR_PIXEL1,
        &mut hair.pixel1,
        false,
        "ps_1_3",
        "HAIR1%03i.pso",
        false,
        usize::MAX,
    );
    if context.render_path >= 2 {
        load_record_range(
            context,
            &HAIR_PIXEL2,
            &mut hair.pixel2,
            false,
            "ps_2_0",
            "HAIR2%03i.pso",
            false,
            usize::MAX,
        );
    }
    hair_setup_passes(context, &mut hair);
    copy_shadow_light_maps(context, slots, &mut hair);
    finish_slot(slot, hair, false)
}

// IDA: ParallaxShader (0x1A4, vtable A9405C) 00809080(declarations 0, 1): the
// ShadowLight base, +0x9C 36 vertex and +0x12C 30 pixel program NiPointers,
// +0x78 = 0, +0x74 = -1; its sixty passes are 00B47620...
#[derive(Default)]
pub struct ParallaxShader {
    pub shadow: ShadowLightShader,
    pub vertex_programs: Vec<ProgramSlot>, // +0x9C
    pub pixel_programs: Vec<ProgramSlot>,  // +0x12C
}
bs_shader_class!(ParallaxShader, shadow, ShadowLightShader);

// IDA: 008091D0 slot 15: two declarations; FormHeap(0x1A4) 00809080; vt+0x84
// 008066E0, vt+0xAC 00807450 (PAR2%03i.vso vs_2_0 from render path 2),
// vt+0xB0 00808190 (PAR2%03i.pso 0..25 from render path 2, 26..29 from path
// 5), vt+0xB4 008649A0 (60 pool passes, vt+0xC0 00864950 -> vt+0xC4..+0xD8
// 00864A60, 00868910, 0086CC90, 00870970, 008717B0, 00871C30), vt+0x88
// 00808DF0. Completion: 100%.
pub fn create_parallax_slot(
    context: &mut BsShaderContext,
    slots: &mut BsShaderSlots,
) -> Arc<BsShaderSlot> {
    let declarations = create_two_lighting_declarations(context);
    let slot = BsShaderSlot {
        declaration: declarations[0].clone(),
        shader: None,
    };
    let mut parallax = ParallaxShader {
        vertex_programs: programs(36),
        pixel_programs: programs(30),
        ..ParallaxShader::default()
    };
    construct_bs_shader_base(&mut parallax, false);
    construct_shadow_light(&mut parallax, &declarations);
    parallax.original_vtable = 0x00A9_405C;
    parallax.class_name = "ParallaxShader";
    let _ = initialize_bs_shader(context, &mut parallax);
    for pass in &mut context.globals.parallax.passes_b47620 {
        assign_pass(&mut context.pools, pass, None);
    }
    if context.render_path >= 2 {
        load_record_range(
            context,
            &PARALLAX_VERTEX,
            &mut parallax.vertex_programs,
            true,
            "vs_2_0",
            "PAR2%03i.vso",
            false,
            usize::MAX,
        );
        load_record_range(
            context,
            &PARALLAX_PIXEL,
            &mut parallax.pixel_programs,
            false,
            "ps_2_0",
            "PAR2%03i.pso",
            false,
            usize::MAX,
        );
        if context.render_path >= 5 {
            for index in 0..4 {
                parallax.pixel_programs[26 + index] = load_pixel(
                    context,
                    &PARALLAX_PIXEL5[index].owned(),
                    "ps_2_0",
                    &program_name("PAR2%03i.pso", 26 + index),
                );
            }
        }
    }
    let mut passes = std::mem::take(&mut context.globals.parallax.passes_b47620);
    for pass in &mut passes {
        acquire_pass_into(context, pass);
    }
    context.globals.parallax.passes_b47620 = passes;
    let target = table_pass_target(
        PassArray::Parallax,
        parallax.vertex_programs.clone(),
        parallax.pixel_programs.clone(),
    );
    let mut stage = StageRef::new();
    replay_pass_ops(context, &PARALLAX_OPS, &target, &mut stage);
    copy_shadow_light_maps(context, slots, &mut parallax);
    stage.reset(context);
    finish_slot(slot, parallax, false)
}

// IDA: SpeedTreeBranchShader (0x134, vtable A9459C) 00810AD0(declaration):
// the ShadowLight base (declaration, null x3), +0x9C 28 vertex (STB1 0..9,
// STB2 10..27) and +0x10C ten pixel program NiPointers; its 28 passes are
// 00B47790...
#[derive(Default)]
pub struct SpeedTreeBranchShader {
    pub shadow: ShadowLightShader,
    pub vertex_programs: Vec<ProgramSlot>, // +0x9C
    pub pixel_programs: Vec<ProgramSlot>,  // +0x10C
}
bs_shader_class!(SpeedTreeBranchShader, shadow, ShadowLightShader);

// IDA: 00810950 vt+0x88: new +0x30 (NiD3DSCM_Vertex) and +0x2C (NiD3DSCM_Pixel)
// maps when null, +0x94 = +0x2C, +0x98 = +0x30, then 007F16F0(+0x30, 37).
// Completion: 100%.
fn branch_constant_maps(context: &mut BsShaderContext, shader: &mut SpeedTreeBranchShader) {
    if shader.vertex_constant_map.is_none() {
        shader.vertex_constant_map = Some(add_map(
            &mut context.constant_maps,
            create_shader_constant_map(false),
        ));
    }
    if shader.pixel_constant_map.is_none() {
        shader.pixel_constant_map = Some(add_map(
            &mut context.constant_maps,
            create_shader_constant_map(true),
        ));
    }
    shader.pixel_map_copy = shader.pixel_constant_map;
    shader.vertex_map_copy = shader.vertex_constant_map;
    let map = shader.vertex_constant_map.expect("set above");
    add_tree_constants(&mut context.constant_maps[map.0 as usize], 37);
}

// IDA: 00810B90 slot 4 (after slot 1): the declaration (6 entries, 1 stream)
// with vt+0x50 (0,0,0,2,0) (1,3,3,2,0) (2,7,5,1,0) and vt+0x4C (0, 3, 8,
// 0x80000000, 2, 6, 0, 0) (0, 4, 9, 0x80000001, 2, 7, 0, 0) (0, 5, 10,
// 0x80000002, 3, 2, 0, 0); FormHeap(0x134) 00810AD0; vt+0x84 0080F1C0,
// vt+0xAC 0080FCE0 (STB1%03i.vso vs_1_1; STB2%03i.vso vs_2_0 from render path
// 2), vt+0xB0 00810630 (STB1%03i.pso ps_1_3), vt+0xB4 00883190 (28 pool passes,
// vt+0xC4 00883260, from render path 2 vt+0xC8 00884D50), vt+0x88 00810950.
// Completion: 100%.
pub fn create_speedtree_branch_slot(context: &mut BsShaderContext) -> Arc<BsShaderSlot> {
    let declaration = make_declaration(context, 6, 1, |declaration| {
        set_entries(
            declaration,
            &[(0, 0, 0, 2, 0), (1, 3, 3, 2, 0), (2, 7, 5, 1, 0)],
        );
        set_entry(declaration, 0, 3, 8, 0x8000_0000, 2, 6, 0);
        set_entry(declaration, 0, 4, 9, 0x8000_0001, 2, 7, 0);
        set_entry(declaration, 0, 5, 10, 0x8000_0002, 3, 2, 0);
    });
    let slot = BsShaderSlot {
        declaration: declaration.clone(),
        shader: None,
    };
    let mut branch = SpeedTreeBranchShader {
        vertex_programs: programs(28),
        pixel_programs: programs(10),
        ..SpeedTreeBranchShader::default()
    };
    construct_bs_shader_base(&mut branch, false);
    construct_shadow_light(&mut branch, &[declaration, None, None, None]);
    branch.original_vtable = 0x00A9_459C;
    branch.class_name = "SpeedTreeBranchShader";
    let _ = initialize_bs_shader(context, &mut branch);
    for pass in &mut context.globals.branch.passes_b47790 {
        assign_pass(&mut context.pools, pass, None);
    }
    load_record_range(
        context,
        &BRANCH_VERTEX1,
        &mut branch.vertex_programs,
        true,
        "vs_1_1",
        "STB1%03i.vso",
        true,
        usize::MAX,
    );
    if context.render_path >= 2 {
        load_record_range(
            context,
            &BRANCH_VERTEX2,
            &mut branch.vertex_programs[10..],
            true,
            "vs_2_0",
            "STB2%03i.vso",
            false,
            usize::MAX,
        );
    }
    load_record_range(
        context,
        &BRANCH_PIXEL1,
        &mut branch.pixel_programs,
        false,
        "ps_1_3",
        "STB1%03i.pso",
        true,
        usize::MAX,
    );
    let mut passes = std::mem::take(&mut context.globals.branch.passes_b47790);
    for pass in &mut passes {
        acquire_pass_into(context, pass);
    }
    context.globals.branch.passes_b47790 = passes;
    let globals = &context.globals.shadow_light;
    let target = PassBuildTarget {
        passes: PassArray::Branch,
        programs: [
            branch.vertex_programs.clone(),
            branch.pixel_programs.clone(),
            globals.vertex_b45290.clone(),
            globals.pixel_b45088.clone(),
        ],
        tables: TableSet::ShadowLight,
    };
    let mut stage = StageRef::new();
    replay_pass_ops(context, &BRANCH_OPS, &target, &mut stage);
    stage.reset(context);
    if context.render_path >= 2 {
        replay_pass_ops(context, &BRANCH_PATH2_OPS, &target, &mut stage);
    }
    branch_constant_maps(context, &mut branch);
    stage.reset(context);
    finish_slot(slot, branch, false)
}
