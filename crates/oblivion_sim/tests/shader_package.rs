//! The real `shaderpackage013.sdp` and the slots built on it at the configuration of the Phase 05 runs (render path 5,
//! HDR, SM3, FP16), compared with `SHD` rows of `fixtures/rows/phase05.rows`. Each row is checked against its evidence.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use bevy_ecs::prelude::World;
use oblivion_core::fixtures;
use oblivion_sim::bs::file::{BsFile, GameFile, NiFileMode};
use oblivion_sim::bs::shader::{
    BsShaderContext, BsShaderSlots, create_image_space_shader_list, get_bs_shader_slot,
};
use oblivion_sim::bs::shader_manager::{
    ShaderManagerGlobals, ShaderManagerInputs, ShaderPackage, initialize_shader_manager,
    texture_manager_without_device,
};
use oblivion_sim::gamebryo::renderer::{RendererGlobals, RendererState};
use oblivion_sim::gamebryo::shader_runtime::ShaderPoolCounters;

/// `SHD` rows only: another block of the file has its own format.
fn rows(kind: &str) -> Vec<Vec<String>> {
    fixtures::rows("phase05")
        .iter()
        .filter(|row| row.starts_with("SHD|"))
        .map(|row| row.splitn(6, '|').map(str::to_owned).collect::<Vec<_>>())
        .filter(|row| row[2] == kind)
        .collect()
}

fn row_value(kind: &str, key: &str) -> String {
    rows(kind)
        .into_iter()
        .find(|row| row[3] == key)
        .unwrap_or_else(|| panic!("row {kind} {key}"))[4]
        .clone()
}

#[test]
fn phase05_shader_rows_match_their_evidence() {
    let evidence = |run: &str| {
        let name = match run {
            "P05-RUN-001" => "phase05-run001-2026-10-06.records.txt",
            "P05-RUN-002" => "phase05-run002-2026-10-06.records.txt",
            _ => panic!("a run ID: {run}"),
        };
        let bytes = std::fs::read(fixtures::root().join("evidence").join(name)).unwrap();
        bytes
            .iter()
            .map(|&byte| char::from(byte))
            .collect::<String>()
    };
    let mut files = HashMap::new();
    let mut count = 0;
    for row in fixtures::rows("phase05")
        .iter()
        .filter(|row| row.starts_with("SHD|"))
    {
        let fields: Vec<&str> = row.splitn(6, '|').collect();
        assert_eq!(fields.len(), 6, "{row}");
        let text = files
            .entry(fields[1].to_owned())
            .or_insert_with(|| evidence(fields[1]));
        assert!(text.contains(fields[5]), "{row} is not in its evidence");
        count += 1;
    }
    assert!(count > 60, "{count} rows");
}

fn open_real(shaders: &Path) -> impl FnMut(&str) -> Option<Box<dyn GameFile>> + '_ {
    move |_| {
        Some(Box::new(BsFile::new(
            shaders,
            "shaderpackage013.sdp",
            NiFileMode::ReadOnly,
            0,
        )) as Box<dyn GameFile>)
    }
}

fn shaders_dir() -> std::path::PathBuf {
    let shaders = fixtures::data_dir().join("Shaders");
    assert!(
        shaders.join("shaderpackage013.sdp").exists(),
        "this test needs the game data: set OBLIVION_DATA_DIR ({shaders:?})"
    );
    shaders
}

/// `verify_shader_package`'s real half: the machine's package (P05-RUN-001: shaderpackage013.sdp, 653 entries).
#[test]
fn verify_shader_package_real() {
    let shaders = shaders_dir();
    let mut real = ShaderPackage::new(13);
    assert_eq!(real.file_name(), Some(row_value("PKG", "NAME").as_str()));
    assert!(real.load(&mut open_real(&shaders)));
    assert_eq!(real.entry_count().to_string(), row_value("PKG", "ENTRIES"));
    let entry = real.find("SLS1000.vso").expect("SLS1000.vso");
    assert!(!real.data(entry).is_empty());
}

fn run_context(package: Arc<ShaderPackage>) -> BsShaderContext {
    // The WINDOW_START snapshot of both runs: g_renderPath 5, hdr, shaders30, fp16Filtering, anisoMin.
    let renderer = RendererState {
        max_streams_6dc: 16,
        ..RendererState::default()
    };
    let mut context = BsShaderContext {
        renderer: Some(Arc::new(renderer)),
        render_path: 5,
        hdr: true,
        shaders_30: true,
        fp16_filtering: true,
        anisotropic_min_filter: true,
        ..BsShaderContext::default()
    };
    // The device caps fixture's stage counts: fixtures/config/device-caps.txt.
    context.pools.max_texture_blend_stages = 8;
    context.pools.max_sampler_stages = 16;
    context.programs.package = Some(package);
    context.programs.hdr = true;
    context.programs.render_path = 5;
    context.programs.has_device = true;
    context.programs.has_renderer = true;
    context.programs.shader_directory = "data\\shaders\\".to_owned();
    context.programs.file_time = Some(Box::new(|_| None));
    context.programs.open_file = Some(Box::new(|_| None));
    context
}

fn pool_row(counters: &ShaderPoolCounters) -> String {
    format!(
        "{:x},{:x},{:x},{:x}",
        counters.capacity, counters.free_count, counters.next_batch, counters.initial_batch
    )
}

/// The 27 slots on the real package, against both runs: classes, declarations, constant maps, pass counts, pool
/// headers and the image-space list. The C++ tests have no device-less equivalent of this one.
#[test]
fn shader_slots_match_the_runs() {
    let shaders = shaders_dir();
    let renderer_state = RendererState::default();
    let renderer_globals = RendererGlobals {
        render_path: 5,
        shaders_30: true,
        hdr: true,
        fp16_filtering: true,
        ..RendererGlobals::default()
    };
    let mut slots = BsShaderSlots::default();
    let mut manager = ShaderManagerGlobals::default();
    let mut context = None;
    let mut open = open_real(&shaders);
    let mut inputs = ShaderManagerInputs {
        construct_texture_manager: texture_manager_without_device(&renderer_state),
        renderer_globals: &renderer_globals,
        max_texture_blend_stages: 8,
        renderer_reference: true,
        image_space_effects: 1,
        open_file: Box::new(&mut open),
        initialize_screen_element_palette: None,
        create_effect_textures: None,
        create_shader: Some(Box::new(|_, globals, slot| {
            let context = context.get_or_insert_with(|| {
                run_context(Arc::clone(
                    globals.package.as_ref().expect("the package loads"),
                ))
            });
            let _ = get_bs_shader_slot(&mut slots, slot as u32, context);
            Ok(())
        })),
        create_image_space_shaders: None,
    };
    let mut world = World::new();
    let result = initialize_shader_manager(&mut world, &mut manager, &mut inputs).unwrap();
    drop(inputs);
    assert!(result.ran);
    let mut context = context.expect("the slots ran");
    // No `World` here, so DebugShader's two calls are only counted; `tes::winmain`'s window test runs them.
    assert_eq!(context.counted_boundaries, [0x0070_7610, 0x0070_7370]);
    // Package use: every package program loads without a warning in the original's configuration.
    assert_eq!(context.programs.warnings, Vec::<String>::new());
    // Every creation is a trace entry; the program keeps its parsed bytecode, of the right stage.
    // The names are the tables' and the entries the package's, so this also says that every program of the package is
    // loaded exactly once at this configuration, and no other. The runs did not record a creation count.
    let calls = &context.programs.device_calls;
    assert_eq!(calls.len().to_string(), row_value("PKG", "ENTRIES"));
    assert!(calls.iter().all(|call| call.bytes > 0));
    let program = context.globals.shadow_light.vertex_b45290[0]
        .as_ref()
        .expect("SLS1000.vso");
    let bytecode = program
        .bytecode
        .as_ref()
        .expect("the package holds a token stream");
    assert!(
        bytecode.is_vertex() && bytecode.ends_with_end_token,
        "{:08X}",
        bytecode.version_token
    );
    assert!(program.vertex && program.handle.is_some());
    assert_eq!(program.original_vtable, 0x00A8_B41C);
    let pixels = context.globals.shadow_light.pixel_b45088.iter().flatten();
    assert!(pixels.clone().count() > 0);
    for pixel in pixels {
        let bytecode = pixel
            .bytecode
            .as_ref()
            .expect("the package holds a token stream");
        assert!(
            !bytecode.is_vertex() && bytecode.ends_with_end_token,
            "{}",
            pixel.package_name
        );
    }
    // SLOT rows (P05-RUN-001): vtable, entries, streams.
    let slot_rows = rows("SLOT");
    assert_eq!(slot_rows.len(), 27);
    for row in &slot_rows {
        let slot = slots.slots[row[3].parse::<usize>().unwrap()]
            .as_ref()
            .expect("a slot");
        let shader = slot.shader.as_ref().expect("a shader");
        let declaration = slot.declaration.as_ref().expect("a declaration");
        let port = format!(
            "{:X},{},{}",
            shader.original_vtable, declaration.entry_count, declaration.stream_count
        );
        assert_eq!(port, row[4], "slot {} ({})", row[3], shader.class_name);
    }
    // SLOTMAP rows (P05-RUN-002): which slots hold a constant map, which share one, and that +0x70 is set.
    let mut map_names = HashMap::new();
    let mut map_ids = HashMap::new();
    for row in rows("SLOTMAP") {
        let values: Vec<&str> = row[4].split(',').collect();
        let shader = slots.slots[row[3].parse::<usize>().unwrap()]
            .as_ref()
            .unwrap()
            .shader
            .as_ref()
            .unwrap();
        for (recorded, port) in [
            (values[0], shader.pixel_constant_map),
            (values[1], shader.vertex_constant_map),
        ] {
            assert_eq!(
                recorded != "0",
                port.is_some(),
                "slot {} map presence",
                row[3]
            );
            if let Some(port) = port {
                let known = map_names.entry(port).or_insert_with(|| recorded.to_owned());
                assert_eq!(*known, recorded, "slot {} map sharing", row[3]);
                // And the other way: one recorded pointer is one map.
                let id = map_ids.entry(recorded.to_owned()).or_insert(port);
                assert_eq!(*id, port, "slot {} map sharing", row[3]);
            }
        }
        assert_eq!(
            values[2] != "0",
            shader.current_pass.is_some(),
            "slot {} +0x70",
            row[3]
        );
    }
    // PASSES rows (P05-RUN-002) and the COUNT rows (P05-RUN-001).
    let globals = &context.globals;
    for (key, passes) in [
        ("shadowLight", &globals.shadow_light.passes_b455a0),
        ("lighting30", &globals.lighting30.passes_b473d0),
        ("skin", &globals.skin.passes_b47718),
        ("branch", &globals.branch.passes_b47790),
        ("parallax", &globals.parallax.passes_b47620),
    ] {
        assert_eq!(
            format!("{},{}", passes.iter().flatten().count(), passes.len()),
            row_value("PASSES", key),
            "{key}"
        );
    }
    assert_eq!(
        globals
            .shadow_light
            .passes_b455a0
            .iter()
            .flatten()
            .count()
            .to_string(),
        row_value("COUNT", "PASSES")
    );
    // The evidence's [54..130) and [32..54) count from 00B4528C, so index 1 is 00B45290[0].
    let vertex = &globals.shadow_light.vertex_b45290;
    assert_eq!(
        vertex[53..129].iter().flatten().count().to_string(),
        row_value("COUNT", "VS_54_130")
    );
    assert_eq!(
        vertex[31..53].iter().flatten().count().to_string(),
        row_value("COUNT", "VS_32_54")
    );
    // ENTRIES row: 00B45018 set, 00B45038 null.
    let values: Vec<String> = row_value("ENTRIES", "B45018,B45038")
        .split(',')
        .map(str::to_owned)
        .collect();
    assert_eq!(
        values[0] != "0",
        globals.shadow_light.entries[&0x00B4_5018].is_some()
    );
    assert_eq!(
        values[1] != "0",
        globals.shadow_light.entries[&0x00B4_5038].is_some()
    );
    // L30PASS0 row: +0x14, +0x18, +0x1C.
    let pass0 = context
        .pools
        .pass(globals.lighting30.passes_b473d0[0].unwrap());
    assert_eq!(
        format!(
            "{},{},{}",
            pass0.stage_end, pass0.stage_count, pass0.textured_stage_count
        ),
        row_value("L30PASS0", "end,count,textured")
    );
    // POOL rows (P05-RUN-002): the header counters at 007BA279.
    assert_eq!(
        pool_row(&context.pools.pass_counters),
        row_value("POOL", "passPool"),
        "passes"
    );
    assert_eq!(
        pool_row(&context.pools.stage_counters),
        row_value("POOL", "stagePool"),
        "stages"
    );
    assert_eq!(
        pool_row(&context.pools.stage_group_counters),
        row_value("POOL", "stageGroupPool"),
        "stage groups"
    );
    assert_eq!(
        pool_row(&context.pools.render_group_counters),
        row_value("POOL", "stateGroupPool"),
        "state groups"
    );
    assert_eq!(
        pool_row(&context.pools.render_entry_counters),
        row_value("POOL", "stateEntryPool"),
        "state entries"
    );
    // ISLIST row (P05-RUN-002): 00803A30 with refraction@B42E84 = 1.
    let list = create_image_space_shader_list(&mut slots, &mut context, true);
    let (order, copy) = {
        let value = row_value("ISLIST", "order");
        let (order, copy) = value.split_once(';').unwrap();
        (
            order
                .split(',')
                .map(|slot| slot.parse::<usize>().unwrap())
                .collect::<Vec<_>>(),
            copy.parse::<usize>().unwrap(),
        )
    };
    assert_eq!(list.shaders.len(), order.len());
    assert_eq!(
        format!("{:X},{}", list.original_vtable, list.shaders.len()),
        row_value("LIST", "IMAGE_SPACE")
    );
    assert_eq!(
        format!(
            "{:X}",
            manager.accumulator.as_ref().unwrap().original_vtable
        ),
        row_value("LIST", "ACCUMULATOR")
    );
    for (shader, slot) in list.shaders.iter().zip(order) {
        assert!(
            Arc::ptr_eq(
                shader,
                slots.slots[slot].as_ref().unwrap().shader.as_ref().unwrap()
            ),
            "slot {slot}"
        );
    }
    assert!(Arc::ptr_eq(
        list.copy_shader.as_ref().unwrap(),
        slots.slots[copy].as_ref().unwrap().shader.as_ref().unwrap()
    ));
}
