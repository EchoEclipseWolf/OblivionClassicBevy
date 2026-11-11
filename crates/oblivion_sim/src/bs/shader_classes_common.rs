//! The helpers every BSShader slot class shares: the local stage reference, the pass and program setup in the
//! original call shape, and the two replayed op tables.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::bs::shader::{
    BsShaderClass, BsShaderContext, BsShaderSlot, PassSlot, ProgramSlot, assign_pass,
    construct_bs_shader_base, initialize_bs_shader, set_shader_declaration,
};
use crate::bs::shader_programs::{
    ShaderMacro, ShaderProgramDescriptor, load_pixel_program, load_vertex_program,
    shader_source_path,
};
use crate::gamebryo::renderer::{RendererGlobals, pixel_shader_target, vertex_shader_target};
use crate::gamebryo::shader_runtime::{
    ConstantMapId, PassId, ShaderConstantMap, ShaderConstantMapEntry, ShaderDeclaration, StageId,
    add_predefined_constant_entry, add_shader_constant_entry, add_stage_reference,
    create_shader_constant_map, create_shader_declaration, find_shader_constant_entry,
    release_pass_reference, release_stage_reference as release_stage, set_declaration_entry,
    set_declaration_entry_by_parameter, set_pass_render_state, set_pass_stage,
    set_texture_stage_defaults, set_texture_stage_filter_mode,
};
use oblivion_core::crt::gcvt;

/// IDA: N/A a descriptor record of a generated table.
#[derive(Clone, Copy, Debug)]
pub struct ProgramRecord {
    pub path: &'static str,
    pub macros: &'static [(&'static str, Option<&'static str>)],
}

impl ProgramRecord {
    /// IDA: N/A the original writes a record to the stack before each load; this is that copy.
    #[must_use]
    pub fn owned(&self) -> ShaderProgramDescriptor {
        descriptor(self.path, self.macros)
    }

    /// IDA: N/A a null HLSL path.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        self.path.is_empty()
    }
}

/// IDA: N/A table constructor.
#[must_use]
pub const fn r(
    path: &'static str,
    macros: &'static [(&'static str, Option<&'static str>)],
) -> ProgramRecord {
    ProgramRecord { path, macros }
}

/// IDA: N/A
#[must_use]
pub fn descriptor(path: &str, macros: &[(&str, Option<&str>)]) -> ShaderProgramDescriptor {
    ShaderProgramDescriptor {
        hlsl_path: path.to_owned(),
        macros: macros
            .iter()
            .map(|(name, definition)| ShaderMacro {
                name: (*name).to_owned(),
                definition: definition.map(str::to_owned),
            })
            .collect(),
    }
}

// IDA: a local NiPointer<NiD3DTextureStage> (the `recordOut` of the pass
// setups): a new pool stage is acquired into a temporary, assigned (the
// previous stage released) and the temporary released; 0075FAE0
// NiPointer_NiD3DTextureStage_Assign, 0076C890 NiPointer_NiD3DPass_Assign
// and 004027D0 (a stage/pass NiPointer slot release) are these assigns and
// releases. Completion: 100%.
#[derive(Debug, Default)]
pub struct StageRef {
    stage: Option<StageId>,
}

impl StageRef {
    /// IDA: N/A a new local NiPointer.
    #[must_use]
    pub const fn new() -> Self {
        Self { stage: None }
    }

    /// IDA: N/A see `StageRef`.
    pub fn acquire(&mut self, context: &mut BsShaderContext) -> StageId {
        let temporary = context.pools.acquire_texture_stage();
        if self.stage != Some(temporary) {
            release_stage(&mut context.pools, self.stage);
            self.stage = Some(temporary);
            add_stage_reference(&mut context.pools, self.stage);
        }
        release_stage(&mut context.pools, Some(temporary));
        temporary
    }

    /// IDA: N/A
    #[must_use]
    pub fn get(&self) -> StageId {
        self.stage.expect("a stage was acquired")
    }

    /// IDA: N/A the local NiPointer released at a builder's end. The C++ destructor's job: a `StageRef` cannot hold
    /// the context, so each scope's end calls this, and `Drop` checks that it did.
    pub fn reset(&mut self, context: &mut BsShaderContext) {
        release_stage(&mut context.pools, self.stage);
        self.stage = None;
    }
}

impl Drop for StageRef {
    /// IDA: N/A Rust's `Drop`.
    fn drop(&mut self) {
        debug_assert!(
            self.stage.is_none() || std::thread::panicking(),
            "a builder's local stage must be released at the original's scope end"
        );
    }
}

// IDA: the pass-slot fill (`AcquireInto(&temp); slot = temp; temp released`).
pub fn acquire_pass_into(context: &mut BsShaderContext, slot: &mut PassSlot) {
    let temporary = context.pools.acquire_pass();
    assign_pass(&mut context.pools, slot, Some(temporary));
    release_pass_reference(&mut context.pools, Some(temporary));
}

// IDA: 00801110 on the context's renderer (*context.renderer in the C++).
pub fn stage_defaults(
    context: &mut BsShaderContext,
    stage: StageId,
    index: u32,
    address: u32,
    filter: u32,
) {
    let renderer = context
        .renderer
        .as_deref()
        .expect("00B43104 gates the slots (007BA252)");
    set_texture_stage_defaults(renderer, &mut context.pools, stage, index, address, filter);
}

// IDA: 00771640 on the context's renderer.
pub fn stage_filter(context: &mut BsShaderContext, stage: StageId, mode: u32) {
    let renderer = context
        .renderer
        .as_deref()
        .expect("00B43104 gates the slots (007BA252)");
    set_texture_stage_filter_mode(renderer, &mut context.pools, stage, mode);
}

// IDA: 00760010(pass, pass +0x14, stage).
pub fn push_stage(context: &mut BsShaderContext, pass: PassId, stage: StageId) {
    let end = context.pools.pass(pass).stage_end;
    let _ = set_pass_stage(&mut context.pools, pass, end, Some(stage));
}

// IDA: 00801110 then 00760010(pass, pass +0x14, stage): a default stage
// appended at the pass's stage end.
pub fn append_default_stage(
    context: &mut BsShaderContext,
    pass: PassId,
    stage: &mut StageRef,
    index: u32,
    address: u32,
    filter: u32,
) {
    let local = stage.acquire(context);
    stage_defaults(context, local, index, address, filter);
    push_stage(context, pass, stage.get());
}

// IDA: the pool pass of a NiPointer slot with the original local-stage
// pattern (00801110 then optionally 00771640, then 00760010).
pub fn append_filtered_stage(
    context: &mut BsShaderContext,
    pass: PassId,
    stage: &mut StageRef,
    index: u32,
    address: u32,
    filter: u32,
    filter_mode: u32,
) {
    let local = stage.acquire(context);
    stage_defaults(context, local, index, address, filter);
    stage_filter(context, local, filter_mode);
    push_stage(context, pass, stage.get());
}

// IDA: a temporary-stage append (AcquireInto(&temp), 00801110, 00760010,
// temp released), used by the loops of 007B09A0. Completion: 100%.
pub fn append_temporary_stage(
    context: &mut BsShaderContext,
    pass: PassId,
    index: u32,
    address: u32,
    filter: u32,
) {
    let stage = context.pools.acquire_texture_stage();
    stage_defaults(context, stage, index, address, filter);
    push_stage(context, pass, stage);
    release_stage(&mut context.pools, Some(stage));
}

// IDA: the `if (!+0x30) acquire; 00772CD0(group, state, value, 0)` runs.
pub fn render_states(context: &mut BsShaderContext, pass: PassId, states: &[(u32, u32)]) {
    for &(state, value) in states {
        set_pass_render_state(&mut context.pools, pass, state, value, false);
    }
}

// IDA: the +0x58 vertex and +0x44 pixel program stores of a pass setup.
pub fn bind_programs(
    context: &mut BsShaderContext,
    pass: PassId,
    vertex: &ProgramSlot,
    pixel: &ProgramSlot,
) {
    let record = context.pools.pass_mut(pass);
    record.vertex_shader.clone_from(vertex);
    record.pixel_shader.clone_from(pixel);
}

// IDA: 00801030 then 008014E0 / 00801C90 for one descriptor record.
pub fn load_vertex(
    context: &mut BsShaderContext,
    descriptor: &ShaderProgramDescriptor,
    profile: &str,
    name: &str,
) -> ProgramSlot {
    let path = shader_source_path(&context.programs.shader_directory, &descriptor.hlsl_path);
    load_vertex_program(
        &mut context.programs,
        &path,
        &descriptor.macros,
        profile,
        name,
    )
}

// IDA: 00801030 then 00801C90 for one descriptor record.
pub fn load_pixel(
    context: &mut BsShaderContext,
    descriptor: &ShaderProgramDescriptor,
    profile: &str,
    name: &str,
) -> ProgramSlot {
    let path = shader_source_path(&context.programs.shader_directory, &descriptor.hlsl_path);
    load_pixel_program(
        &mut context.programs,
        &path,
        &descriptor.macros,
        profile,
        name,
    )
}

// IDA: N/A sprintf of a name format with one "%03i".
#[must_use]
pub fn program_name(format: &str, index: usize) -> String {
    format.replacen("%03i", &format!("{index:03}"), 1)
}

// IDA: the renderer profile helpers the slot loaders call:
// Renderer_GetVertexShaderTarget (007B47E0) and
// Renderer_GetPixelShaderTarget(0) (007B4780).
#[must_use]
pub fn vertex_target(context: &BsShaderContext) -> &'static str {
    // A null target (an unknown render path) reaches the loaders as a null profile; the host passes an empty string.
    vertex_shader_target(context.render_path).unwrap_or("")
}

// IDA: 007B4780 Renderer_GetPixelShaderTarget(0).
#[must_use]
pub fn pixel_target(context: &BsShaderContext) -> &'static str {
    let targets = RendererGlobals {
        render_path: context.render_path,
        ps2x_path: context.ps2x_path,
        ..RendererGlobals::default()
    };
    pixel_shader_target(&targets, false).unwrap_or("")
}

// IDA: the CRT _gcvt(value, 12, Buffer_1) of the sky loader.
#[must_use]
pub fn gcvt_12(value: f32) -> String {
    gcvt(f64::from(value), 12)
}

// IDA: N/A a slot's declaration, filled before it is shared. The C++ fills it through the shared pointer.
pub fn make_declaration(
    context: &BsShaderContext,
    entries: u32,
    streams: u32,
    declare: impl FnOnce(&mut ShaderDeclaration),
) -> Option<Arc<ShaderDeclaration>> {
    let renderer = context
        .renderer
        .as_deref()
        .expect("00B43104 gates the slots (007BA252)");
    let mut declaration = create_shader_declaration(renderer, entries, streams)?;
    declare(&mut declaration);
    Some(Arc::new(declaration))
}

// IDA: the store of +4 and vt+0x54, in either order.
pub fn finish_slot<S: BsShaderClass>(
    mut slot: BsShaderSlot,
    mut shader: S,
    declaration_first: bool,
) -> Arc<BsShaderSlot> {
    if declaration_first {
        set_shader_declaration(shader.base_mut(), slot.declaration.clone());
        slot.shader = Some(Arc::new(shader));
        return Arc::new(slot);
    }
    let declaration = slot.declaration.clone();
    let mut stored: Arc<dyn BsShaderClass> = Arc::new(shader);
    set_shader_declaration(
        Arc::get_mut(&mut stored).expect("just made").base_mut(),
        declaration,
    );
    slot.shader = Some(stored);
    Arc::new(slot)
}

// IDA: the general slot factory shape: 007AED00, 00771090(00B43104,
// entries, streams) with the class's vt+0x50/vt+0x4C entries, the class
// constructor (some take the declaration: the factory's NiObject RTTI walk
// passes it through), vt+0x84 008025F0 (vt+0x98/+0xA0 00977C50, vt+0x9C
// 00801380) plus any class tail, the class steps, then either +4 = shader
// and vt+0x54, or vt+0x54 and +4. Completion: 100%.
#[allow(clippy::too_many_arguments)] // The factory's steps, in order.
pub fn create_bs_shader_slot<S: BsShaderClass + Default>(
    context: &mut BsShaderContext,
    entries: u32,
    streams: u32,
    declare: impl FnOnce(&mut ShaderDeclaration),
    image_space: bool,
    construct: impl FnOnce(&mut S, &mut BsShaderContext, &Option<Arc<ShaderDeclaration>>),
    steps: impl FnOnce(&mut S, &mut BsShaderContext),
    declaration_first: bool,
) -> Arc<BsShaderSlot> {
    let slot = BsShaderSlot {
        declaration: make_declaration(context, entries, streams, declare),
        shader: None,
    };
    let mut shader = S::default();
    construct_bs_shader_base(shader.base_mut(), image_space);
    construct(&mut shader, context, &slot.declaration);
    let _ = initialize_bs_shader(context, shader.base_mut());
    steps(&mut shader, context);
    finish_slot(slot, shader, declaration_first)
}

// IDA: the two-entry image-space declaration many classes reuse:
// vt+0x50 (0,0,0,2,0) and (1,7,5,1,0).
pub fn declare_position_texture(declaration: &mut ShaderDeclaration) {
    set_declaration_entry_by_parameter(declaration, 0, 0, 0, 2, 0);
    set_declaration_entry_by_parameter(declaration, 1, 7, 5, 1, 0);
}

// IDA: the image-space factory shape shared by the 11 classes:
// 007AED00, 00771090(00B43104, 2, 1) with vt+0x50 (0,0,0,2,0) and
// (1,7,5,1,0), the class constructor (no declaration argument), vt+0x84
// 008025F0 (vt+0x98/+0xA0 00977C50, vt+0x9C 00801380), then the class's
// steps (vt+0xB0 programs, vt+0xB4 passes), +4 = shader, vt+0x54 (the water
// factories 007E1430/007DE710 run vt+0x54 before the +4 store). Completion: 100%.
pub fn create_image_space_slot<S: BsShaderClass + Default>(
    context: &mut BsShaderContext,
    construct: impl FnOnce(&mut S, &mut BsShaderContext),
    steps: impl FnOnce(&mut S, &mut BsShaderContext),
    declaration_first: bool,
) -> Arc<BsShaderSlot> {
    create_bs_shader_slot::<S>(
        context,
        2,
        1,
        declare_position_texture,
        true,
        |shader, context, _| construct(shader, context),
        steps,
        declaration_first,
    )
}

// IDA: the +0x2C/+0x30 fill of vt+0x88: when the slot is null, FormHeap(0x34)
// with 009A4750 (NiD3DSCM_Pixel) or 009A8490 (NiD3DSCM_Vertex) on the
// shader's renderer (+0x14), NiPointer assign, then the entries.
pub fn build_constant_map(
    maps: &mut Vec<ShaderConstantMap>,
    slot: Option<ConstantMapId>,
    pixel: bool,
    fill: impl FnOnce(&mut ShaderConstantMap),
) -> Option<ConstantMapId> {
    if slot.is_some() {
        return slot;
    }
    let mut map = create_shader_constant_map(pixel);
    fill(&mut map);
    Some(add_map(maps, map))
}

// IDA: N/A a new map's place.
pub fn add_map(maps: &mut Vec<ShaderConstantMap>, map: ShaderConstantMap) -> ConstantMapId {
    maps.push(map);
    ConstantMapId(maps.len() as u32 - 1)
}

// IDA: a constant-map entry of the water vt+0x88 builders: flags 0x10000007,
// register class 0, one register, variable "" (Default), size 16, stride 4,
// no copy.
pub fn add_float4_constant(map: &mut ShaderConstantMap, key: &str, reg: u32, data: &str) {
    let _ = add_shader_constant_entry(map, key, 0x1000_0007, 0, reg, 1, "", 16, 4, data, false);
}

// IDA: the vt+0x88 constant-map entries with a Default variable, stride 4
// and no copy.
pub fn add_constant(
    map: &mut ShaderConstantMap,
    key: &str,
    flags: u32,
    reg: u32,
    count: u32,
    size: u32,
    data: &str,
) {
    let _ = add_shader_constant_entry(map, key, flags, 0, reg, count, "", size, 4, data, false);
}

// IDA: a predefined entry through vt+0x18 (flags 0x20000009, a null
// variable, size/stride/data 0).
pub fn add_predefined(map: &mut ShaderConstantMap, key: &str, extra: u32, reg: u32, count: u32) {
    let _ = add_shader_constant_entry(
        map,
        key,
        0x2000_0009,
        extra,
        reg,
        count,
        "",
        0,
        0,
        "",
        false,
    );
}

// IDA: the 0x10000009 multi-register constant entries.
pub fn add_matrix_constant(
    map: &mut ShaderConstantMap,
    key: &str,
    reg: u32,
    count: u32,
    size: u32,
    data: &str,
) {
    add_constant(map, key, 0x1000_0009, reg, count, size, data);
}

// IDA: 007F16F0(map, register): TreeData (c register, 00B467A0) and
// WindMatrices (c register + 1, 16 registers, 256 bytes, 00B467B8). Completion: 100%.
pub fn add_tree_constants(map: &mut ShaderConstantMap, reg: u32) {
    add_constant(map, "TreeData", 0x1000_0007, reg, 1, 16, "00B467A0");
    add_constant(
        map,
        "WindMatrices",
        0x1000_0009,
        reg + 1,
        16,
        256,
        "00B467B8",
    );
}

/// IDA: the op kinds of the pass builders' instruction traces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassBuildKind {
    Guard,
    Stage,
    State,
    Vertex,
    Pixel,
    Table,
    Release,
    Stages7,
    Address1,
    Flush,
    StageAt,
    TableCopy,
}

// IDA: N/A
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassBuildOp {
    pub kind: PassBuildKind,
    pub pass: u16,
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub d: u32,
}

/// IDA: N/A table constructor.
#[must_use]
pub const fn o(kind: PassBuildKind, pass: u16, a: u32, b: u32, c: u32, d: u32) -> PassBuildOp {
    PassBuildOp {
        kind,
        pass,
        a,
        b,
        c,
        d,
    }
}

/// IDA: N/A which pass global array a builder writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassArray {
    Lighting30,
    ShadowLight,
    Skin,
    Branch,
    Parallax,
}

/// IDA: N/A which per-pass tables a builder writes: 00B46988 and 00B46A60, or the four ShadowLight tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableSet {
    Lighting30,
    ShadowLight,
}

// IDA: N/A the global arrays a pass-builder table addresses.
#[derive(Clone, Debug)]
pub struct PassBuildTarget {
    pub passes: PassArray,
    /// Copies of the program arrays, as nothing loads a program during a replay.
    pub programs: [Vec<ProgramSlot>; 4],
    pub tables: TableSet,
}

// IDA: N/A a helper, with no address of its own.
fn pass_array(context: &mut BsShaderContext, array: PassArray) -> &mut Vec<PassSlot> {
    let globals = &mut context.globals;
    match array {
        PassArray::Lighting30 => &mut globals.lighting30.passes_b473d0,
        PassArray::ShadowLight => &mut globals.shadow_light.passes_b455a0,
        PassArray::Skin => &mut globals.skin.passes_b47718,
        PassArray::Branch => &mut globals.branch.passes_b47790,
        PassArray::Parallax => &mut globals.parallax.passes_b47620,
    }
}

// IDA: N/A a helper, with no address of its own.
fn table(context: &mut BsShaderContext, set: TableSet, index: u32) -> &mut Vec<u32> {
    match (set, index) {
        (TableSet::Lighting30, 0) => &mut context.globals.lighting30.flags_b46988,
        (TableSet::Lighting30, 1) => &mut context.globals.lighting30.second_b46a60,
        (TableSet::ShadowLight, 0..=3) => &mut context.globals.shadow_light.tables[index as usize],
        _ => panic!("a null table in {set:?}: {index}"),
    }
}

// IDA: 0085E570(pass): a local stage NiPointer; when the pass has fewer than
// seven stages (+0x18), stages 0..6 = 00801110(i, 1, 2) appended. Completion: 100%.
pub fn append_seven_stages(context: &mut BsShaderContext, pass: PassId) {
    let mut stage = StageRef::new();
    if context.pools.pass(pass).stage_count >= 7 {
        return;
    }
    for index in 0..7 {
        append_default_stage(context, pass, &mut stage, index, 1, 2);
    }
    stage.reset(context);
}

// IDA: the replay of one generated builder table; `stage` is the builder's
// shared local stage NiPointer. Completion: 100% with the generated tables.
pub fn replay_pass_ops(
    context: &mut BsShaderContext,
    ops: &[PassBuildOp],
    target: &PassBuildTarget,
    stage: &mut StageRef,
) {
    let mut index = 0;
    while index < ops.len() {
        let op = ops[index];
        index += 1;
        let pass_index = op.pass as usize;
        match op.kind {
            PassBuildKind::Table => {
                table(context, target.tables, op.a)[pass_index] = op.b;
                continue;
            }
            PassBuildKind::Flush => {
                stage.reset(context); // the builder's local stage at its end
                continue;
            }
            PassBuildKind::TableCopy => {
                let value = table(context, target.tables, op.c)[op.d as usize];
                table(context, target.tables, op.a)[pass_index] = value;
                continue;
            }
            PassBuildKind::Release => {
                let mut slot = pass_array(context, target.passes)[pass_index];
                assign_pass(&mut context.pools, &mut slot, None);
                pass_array(context, target.passes)[pass_index] = slot;
                continue;
            }
            _ => {}
        }
        let pass = pass_array(context, target.passes)[pass_index].expect("a builder's pass");
        match op.kind {
            PassBuildKind::Guard => {
                if context.pools.pass(pass).stage_count >= op.a {
                    index += op.b as usize;
                }
            }
            PassBuildKind::Stage | PassBuildKind::StageAt => {
                let local = stage.acquire(context);
                stage_defaults(context, local, op.a, op.b, op.c);
                if op.d != 0 {
                    context.pools.stage_mut(local).texture_global = op.d; // 0076C910
                    context.effect_texture_requests.push(op.d);
                    let texture = context
                        .effect_texture
                        .as_mut()
                        .and_then(|texture| texture(op.d));
                    context.pools.stage_mut(local).texture = texture;
                }
                let slot_index = if op.kind == PassBuildKind::StageAt {
                    op.a
                } else {
                    context.pools.pass(pass).stage_end
                };
                let _ = set_pass_stage(&mut context.pools, pass, slot_index, Some(local));
            }
            PassBuildKind::State => {
                set_pass_render_state(&mut context.pools, pass, op.a, op.b, op.c != 0);
            }
            PassBuildKind::Vertex => {
                context
                    .pools
                    .pass_mut(pass)
                    .vertex_shader
                    .clone_from(&target.programs[op.a as usize][op.b as usize]);
            }
            PassBuildKind::Pixel => {
                context
                    .pools
                    .pass_mut(pass)
                    .pixel_shader
                    .clone_from(&target.programs[op.a as usize][op.b as usize]);
            }
            PassBuildKind::Stages7 => append_seven_stages(context, pass),
            PassBuildKind::Address1 => {
                let second = context.pools.pass(pass).stages.get(1).copied().flatten();
                if let Some(second) = second {
                    let renderer = context
                        .renderer
                        .as_deref()
                        .expect("00B43104 gates the slots (007BA252)");
                    let first = renderer.texture_stage_globals[18 + 2 * op.a as usize];
                    let next = renderer.texture_stage_globals[19 + 2 * op.a as usize];
                    let states = context.pools.stage_states_mut(second);
                    states.set_sampler_state(1, first, false, false);
                    states.set_sampler_state(2, next, false, false);
                }
            }
            _ => {}
        }
    }
}

/// IDA: the op kinds of the 007CC280 map trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapBuildKind {
    Create,
    Add,
    Predefined,
    Get,
    Copy,
}

// IDA: N/A
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapBuildOp {
    pub kind: MapBuildKind,
    pub map: u8,
    pub path2: u8,
    pub key: Option<&'static str>,
    pub flags: u32,
    pub extra: u32,
    pub reg: u32,
    pub count: u32,
    pub size: u32,
    pub stride: u32,
    pub data: Option<&'static str>,
    pub global: u32,
}

/// IDA: N/A table constructor.
#[allow(clippy::too_many_arguments)] // One row of the generated table.
#[must_use]
pub const fn m(
    kind: MapBuildKind,
    map: u8,
    path2: u8,
    key: Option<&'static str>,
    flags: u32,
    extra: u32,
    reg: u32,
    count: u32,
    size: u32,
    stride: u32,
    data: Option<&'static str>,
    global: u32,
) -> MapBuildOp {
    MapBuildOp {
        kind,
        map,
        path2,
        key,
        flags,
        extra,
        reg,
        count,
        size,
        stride,
        data,
        global,
    }
}

// IDA: the replay of one generated map table. Completion: 100% with the generated table.
pub fn replay_map_ops(
    context: &mut BsShaderContext,
    ops: &[MapBuildOp],
    maps: &mut [Option<ConstantMapId>; 6],
    entries: &mut BTreeMap<u32, Option<Arc<ShaderConstantMapEntry>>>,
) {
    let mut skipping = false;
    for op in ops {
        let slot = op.map as usize;
        if op.kind == MapBuildKind::Create {
            skipping = maps[slot].is_some();
            if !skipping {
                let map = create_shader_constant_map(op.map == 1 || op.map == 3);
                maps[slot] = Some(add_map(&mut context.constant_maps, map));
            }
            continue;
        }
        if skipping || (op.path2 != 0 && context.render_path < 2) {
            continue;
        }
        if op.kind == MapBuildKind::Copy {
            maps[slot] = maps[op.reg as usize];
            continue;
        }
        let key = op.key.unwrap_or("");
        let map = &mut context.constant_maps[maps[slot].expect("a created map").0 as usize];
        match op.kind {
            MapBuildKind::Add => {
                let _ = add_shader_constant_entry(
                    map,
                    key,
                    op.flags,
                    op.extra,
                    op.reg,
                    op.count,
                    "",
                    op.size,
                    op.stride,
                    op.data.unwrap_or(""),
                    false,
                );
            }
            MapBuildKind::Predefined => {
                let _ = add_predefined_constant_entry(map, key, op.extra, op.reg, "");
            }
            MapBuildKind::Get => {
                entries.insert(op.global, find_shader_constant_entry(map, key));
            }
            MapBuildKind::Create | MapBuildKind::Copy => {}
        }
    }
}

// IDA: N/A the entry of a declaration the many classes set, in vt+0x4C's order.
#[allow(clippy::too_many_arguments)] // 0076FC40 vt+0x4C argument order.
pub fn set_entry(
    declaration: &mut ShaderDeclaration,
    stream: u32,
    index: u32,
    register: u32,
    parameter: u32,
    parameter_type: u32,
    usage: u32,
    usage_index: u32,
) {
    set_declaration_entry(
        declaration,
        stream,
        index,
        register,
        parameter,
        parameter_type,
        usage,
        usage_index,
        0,
    );
}

// IDA: N/A vt+0x50.
pub fn set_entries(declaration: &mut ShaderDeclaration, rows: &[(u32, u32, u32, u32, u32)]) {
    for &(index, register, parameter, parameter_type, stream) in rows {
        set_declaration_entry_by_parameter(
            declaration,
            index,
            register,
            parameter,
            parameter_type,
            stream,
        );
    }
}

// IDA: N/A a helper, with no address of its own.
// Glue so a program array of any size starts empty.
#[must_use]
pub fn programs(count: usize) -> Vec<ProgramSlot> {
    vec![None; count]
}

// Shared base-class access is through deref coercion; this macro writes the three trait methods and the two `Deref`s.
macro_rules! bs_shader_class {
    ($name:ident, $field:ident, $base:ty) => {
        impl std::ops::Deref for $name {
            type Target = $base;
            /// IDA: N/A a helper, with no address of its own.
            fn deref(&self) -> &$base {
                &self.$field
            }
        }
        impl std::ops::DerefMut for $name {
            /// IDA: N/A a helper, with no address of its own.
            fn deref_mut(&mut self) -> &mut $base {
                &mut self.$field
            }
        }
        impl $crate::bs::shader::BsShaderClass for $name {
            /// IDA: N/A a helper, with no address of its own.
            fn base(&self) -> &$crate::bs::shader::BsShaderObject {
                self
            }
            /// IDA: N/A a helper, with no address of its own.
            fn base_mut(&mut self) -> &mut $crate::bs::shader::BsShaderObject {
                self
            }
            /// IDA: N/A a helper, with no address of its own.
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
    };
}
pub(crate) use bs_shader_class;
