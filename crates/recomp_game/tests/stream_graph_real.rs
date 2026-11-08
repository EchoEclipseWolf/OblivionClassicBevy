//! The model drain on the real NIFs against P15-RUN-001's Phase 13e rows, every line of `fixtures/rows/phase13e.rows`.
//!
//! This queues the 26 preload tasks of P13-RUN-001's `TASK` rows through 0043B420 and runs `ModelDrain` straight through, so a row covers the stream graph, 00436880's root, 004809A0's strip and "BASE " name, 007D1920's choice and the reference counts. The rows were captured at the MainMenu. One difference is declared and counted: the ShadowLight boundary (0080B190) leaves the NiTexturingProperty where the run has its BSShaderPPLightingProperty, on one object. `game_data_window` runs the same drain on the real load's World in the run's own order. Host tests earn no validation bonus.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bevy_ecs::prelude::*;
use recomp_core::crt::MsvcRand;
use recomp_core::fixtures;
use recomp_game::bs::archive::*;
use recomp_game::bs::file_finder::{
    BsFileCallbacks, FileFinder, bs_file_exists, bs_file_open_core,
};
use recomp_game::bs::model_loader::*;
use recomp_game::bs::shader::BsShaderSlot;
use recomp_game::bs::shader_classes_geometry::SkyShader;
use recomp_game::bs::sky_shader::{NiGeometryShader, PrepareGeometryContext};
use recomp_game::bs::stream::BsStreamServices;
use recomp_game::bs::task_manager::TaskGlobals;
use recomp_game::gamebryo::particle_system::ParticleSystem;
use recomp_game::gamebryo::registration::{
    NiFactoryRegistry, NiProcessDefaults, StartupCallbackScheduler, register_known_crt_callbacks,
};
use recomp_game::gamebryo::search_path::bs_search_path_candidates;
use recomp_game::gamebryo::stream_objects::NiClass;
use recomp_game::gamebryo::stream_postload::StreamPostLoadRuntime;
use recomp_game::gamebryo::texture::SourceTextureLoadServices;
use recomp_game::gamebryo::texture_loading::{TextureLoadServices, load_shader_texture};

/// IDA: 0x00A90634, the ShadowLight family's vtable (00A90634 +0x18 is 0080B190), on a stub shader as the C++ test uses.
const SHADOW_LIGHT_VTABLE: u32 = 0x00A9_0634;

struct NoPalette<'a> {
    root: &'a std::path::Path,
    callbacks: &'a BsFileCallbacks,
    finder: &'a FileFinder,
    archives: &'a mut ArchiveManager,
}

impl SourceTextureLoadServices for NoPalette<'_> {
    fn search_candidates(&mut self, file_path: &[u8]) -> Vec<Vec<u8>> {
        bs_search_path_candidates(&String::from_utf8_lossy(file_path))
            .into_iter()
            .map(String::into_bytes)
            .collect()
    }
    fn find_texture(&mut self, _path: &[u8]) -> Option<Entity> {
        None
    }
    fn register_texture(&mut self, _path: &[u8], _texture: Entity) {}
    fn file_exists(&mut self, path: &[u8]) -> bool {
        let text: String = path.iter().map(|&byte| char::from(byte)).collect();
        bs_file_exists(
            self.callbacks,
            self.root,
            Some(self.finder),
            Some(&mut *self.archives),
            &text,
            0,
        )
    }
}

fn archive_manager(root: &std::path::Path, finder: &FileFinder, tag: &str) -> ArchiveManager {
    let settings = ArchiveSettings {
        invalidate_older_files: false,
        retain_directory_strings: 2,
        ..ArchiveSettings::default()
    };
    let scratch = std::env::temp_dir().join(format!(
        "recomp_stream_graph_real_{tag}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join("Plugins.txt"), "Oblivion.esm\n").unwrap();
    let mut archives = ArchiveManager::new(root, settings);
    archives.initialize_from_settings(finder, &scratch);
    let _ = std::fs::remove_dir_all(&scratch);
    archives
}

#[test]
fn the_drain_builds_the_recorded_models() {
    let data = fixtures::data_dir();
    if !data.exists() {
        eprintln!("skipped: no game data at {data:?}");
        return;
    }
    let mut registry = NiFactoryRegistry::default();
    registry.register_known_crt_factories();
    registry.register_winmain_bs_factories();
    for (name, constructor) in recomp_game::bs::havok::BHK_FACTORIES {
        registry.insert_or_assign(name, constructor);
    }
    let root = data.parent().unwrap().canonicalize().unwrap();
    let mut file_callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new(&root, &mut file_callbacks);
    finder.add("Data");
    let mut archives = archive_manager(&root, &finder, "queue");
    let mut texture_archives = archive_manager(&root, &finder, "textures");
    let mut normal_archives = archive_manager(&root, &finder, "normals");

    let mut world = World::new();
    let mut known = StartupCallbackScheduler::default();
    register_known_crt_callbacks(&mut known).unwrap();
    known.run(&mut world).unwrap();
    let defaults = world
        .resource::<NiProcessDefaults>()
        .scene_property_defaults();
    let mut tasks = TaskGlobals::default();
    tasks.construct_io_manager().unwrap();
    tasks.construct_model_loader().unwrap();
    // P13-RUN-001's 26 requests: priority 5 is byte 2 of each key, and flags 3 is a8 and a9.
    for line in fixtures::rows("phase13")
        .iter()
        .filter(|line| line.starts_with("TASK "))
    {
        let parts: Vec<&str> = line.split(' ').collect();
        let out = load_model(
            &mut tasks,
            Some(&mut archives),
            parts[2].as_bytes(),
            ModelRequest {
                priority: 5,
                parent: None,
                lod: 0,
                a7: false,
                a8: true,
                a9: true,
            },
        )
        .unwrap();
        release_task(&mut tasks, out).unwrap();
    }
    world.insert_resource(tasks);

    let palette: RefCell<HashMap<Vec<u8>, Entity>> = RefCell::new(HashMap::new());
    let mut probes = Vec::new();
    let mut setups = Vec::new();
    let (mut open, mut rand) = (
        |path: &str, mode: i32, size: u32| {
            bs_file_open_core(&finder, None, path, mode, size, 0xFFFF)
                .map(recomp_game::bs::file::OpenedFile::into_game_file)
        },
        MsvcRand::default(),
    );
    let mut draw = || rand.rand();
    let mut slots: Vec<u32> = Vec::new();
    let shadow = Arc::new({
        let mut shader = SkyShader::default();
        shader.base.original_vtable = SHADOW_LIGHT_VTABLE;
        BsShaderSlot {
            declaration: None,
            shader: Some(Arc::new(shader)),
        }
    });
    let mut get_shader = |index: u32| {
        slots.push(index);
        Some(Arc::clone(&shadow))
    };
    let mut load_normal = |world: &mut World, path: &[u8]| {
        probes.push(path.to_vec());
        let palette_read = &palette;
        let palette_write = &palette;
        let normal = &mut normal_archives;
        let (root, finder, callbacks) = (&root, &finder, &file_callbacks);
        load_shader_texture(
            world,
            path,
            true,
            true,
            &mut TextureLoadServices {
                create_renderer_data_on_create: false,
                file_exists: Some(Box::new(move |path: &[u8]| {
                    let text: String = path.iter().map(|&byte| char::from(byte)).collect();
                    bs_file_exists(callbacks, root, Some(finder), Some(&mut *normal), &text, 0)
                })),
                palette_find: Some(Box::new(move |path: &[u8]| {
                    palette_read.borrow().get(path).copied()
                })),
                palette_add: Some(Box::new(move |path: &[u8], texture: Option<Entity>| {
                    if let Some(texture) = texture {
                        palette_write.borrow_mut().insert(path.to_vec(), texture);
                    }
                })),
                ..TextureLoadServices::default()
            },
        )
        .is_some()
    };
    let mut setup = |vtable: u32, geometry: Entity| {
        setups.push((vtable, geometry));
        true
    };
    let mut disabled = 0_u8;
    let mut warnings = Vec::new();
    let mut warning = |text: Vec<u8>, _: bool| warnings.push(text);
    let mut textures = NoPalette {
        root: &root,
        callbacks: &file_callbacks,
        finder: &finder,
        archives: &mut texture_archives,
    };
    let mut services = ModelBuildServices {
        archives: Some(&mut archives),
        stream: BsStreamServices {
            open: &mut open,
            registry: &registry,
            draw_rand: Some(&mut draw),
            draw_on_io_thread: false,
            source_textures: Some(&mut textures),
            post_load: StreamPostLoadRuntime::default(),
        },
        prepare: PrepareGeometryContext {
            render_path: 5,
            hdr: true,
            reference_classifier: 0,
            property_defaults: defaults,
            get_shader: &mut get_shader,
            warnings: Vec::new(),
            sky_shader_trace: Vec::new(),
            water_shader_trace: Vec::new(),
            lava_b47844: 0,
            shaders_30_b42ea5: false,
            load_normal_map: Some(&mut load_normal),
            unported_shader_setup: Some(&mut setup),
            classify_trace: Vec::new(),
        },
        disable_warning_messages_b06b18: &mut disabled,
        show_editor_markers_b34390: 0,
        warning: &mut warning,
        main_random_next: None,
    };
    let mut drain = ModelDrain::new();
    assert_eq!(
        drain.run_to_end(&mut world, &mut services).unwrap(),
        ModelDrainStep::Ended
    );
    let classified = std::mem::take(&mut services.prepare.classify_trace);
    drop(services);
    assert_eq!(
        drain.record.log_line(setups.len() as u32),
        "Phase-13e: model tasks run 26 (models built 18, cached 8, failed 0), queued 0 (0 Havok, 0 NIF 10.2.0.0), shader setups not run 2"
    );
    assert!(warnings.is_empty());
    assert_eq!(disabled, 0, "00436C67 restores it");
    assert_eq!(
        slots,
        [1, 1],
        "two geometries reach a shader slot, at index 1"
    );
    assert_eq!(setups.len(), 2);
    assert!(classified.iter().filter(|(_, index)| *index == 1).count() == 2);
    println!(
        "normal map probes {}, classifications {}",
        probes.len(),
        classified.len()
    );

    // Every task has run.
    let tasks = world.resource::<TaskGlobals>();
    let io = tasks.io_manager_b33a10.as_ref().unwrap();
    assert_eq!(io.tasks.buckets[16].len(), 0);
    assert_eq!(io.task_map.field_18, 0);
    assert_eq!(io.array_2c[16], 0);
    assert_eq!(tasks.counters.live_tasks_b33a20, 0);

    // Every line of phase13e.rows.
    let rows = fixtures::rows("phase13e");
    let loader = tasks.model_loader_b33a1c.as_ref().unwrap();
    let mut models: BTreeMap<&[u8], &Model> = BTreeMap::new();
    for entry in loader.models.buckets.iter().flatten() {
        models.insert(&entry.key, &loader.models.store[entry.model.0]);
    }
    assert_eq!(loader.model_map_00.field_18, 18, "0055F229 vt+0x30");
    let mut host_lines: Vec<String> = Vec::new();
    let mut failures = Vec::new();
    let mut compared_models = 0;
    let mut boundary_rows = 0;
    let mut compared_nodes = 0;
    for line in &rows {
        if let Some(rest) = line.strip_prefix("MODEL|") {
            let parts: Vec<&str> = rest.split('|').collect();
            let Some(model) = models.get(parts[0].as_bytes()) else {
                failures.push(format!("no model {}", parts[0]));
                continue;
            };
            if model.references_04.to_string() != parts[1] {
                failures.push(format!("{} references {}", parts[0], model.references_04));
            }
            if model.path_00 != parts[2].as_bytes() {
                failures.push(format!("{} path", parts[0]));
            }
            let found = model_tree_rows(&world, parts[0], model.root_08.unwrap());
            host_lines.extend(found);
            compared_models += 1;
        }
    }
    let mut host: BTreeMap<String, String> = BTreeMap::new();
    for line in &host_lines {
        let parts: Vec<&str> = line.split('|').collect();
        host.insert(format!("{}|{}", parts[1], parts[2]), line.clone());
    }
    for line in rows.iter().filter(|line| line.starts_with("NODE|")) {
        let parts: Vec<&str> = line.split('|').collect();
        let Some(got) = host.get(&format!("{}|{}", parts[1], parts[2])) else {
            failures.push(format!("missing: {line}"));
            continue;
        };
        compared_nodes += 1;
        // Declared: 0080B190's property, on the object that reaches the boundary.
        if parts[7].contains("BSShaderPPLightingProperty") {
            boundary_rows += 1;
            let mut want: Vec<&str> = parts[7]
                .split(',')
                .map(|name| {
                    if name == "BSShaderPPLightingProperty" {
                        "NiTexturingProperty"
                    } else {
                        name
                    }
                })
                .collect();
            want.sort_unstable();
            let got_parts: Vec<&str> = got.split('|').collect();
            let mut have: Vec<&str> = got_parts[7].split(',').collect();
            have.sort_unstable();
            if want != have {
                failures.push(format!("{got}\n  != {line}"));
            }
            let (mut a, mut b) = (got_parts.clone(), parts.clone());
            a[7] = "";
            b[7] = "";
            if a != b {
                failures.push(format!("{got}\n  != {line}"));
            }
            continue;
        }
        if got != line {
            failures.push(format!("{got}\n  != {line}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!((compared_models, compared_nodes), (7, 86));
    assert_eq!(boundary_rows, 1);
    assert_eq!(host_lines.len(), 86, "no object the rows do not list");
    let particles = host_lines
        .iter()
        .filter(|line| line.contains("|NiParticleSystem|"))
        .count();
    assert!(particles > 0);
    // Each geometry holds +0xBC only where a row says so; the particle systems answered 0 at 007D19E9.
    for entry in &classified {
        if world.get::<NiClass>(entry.0) == Some(&NiClass::ParticleSystem) {
            assert_eq!(entry.1, 0);
            assert!(world.get::<ParticleSystem>(entry.0).is_some());
            assert!(world.get::<NiGeometryShader>(entry.0).is_none());
        }
    }
}
