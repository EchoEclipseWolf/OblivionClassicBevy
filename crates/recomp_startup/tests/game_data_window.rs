//! The game data window as one sequence: Phase 08's prelude, Phase 09's TES block with the real DataHandler and file list, Phase 10's plugin selection, Phase 11's player, Phase 12's whole load and Phase 13's tail, on one World: 00406390 through the end of 00406360. Each phase test builds what the blocks before it would leave, so this is the check that those stand-ins aren't hiding anything. Values come from the P10-RUN-001, P11-RUN-001, P12-RUN-001 and P13-RUN rows. Host tests earn no validation bonus.
//!
//! `fixtures/rows/phase13.rows` is read here, every line, with the accounting of `phase12_load.rs`, and 13e's drain runs last, in the order P13-RUN-001's model map shows.
//!
//! The Phase 07 window isn't joined here: nothing in 00406360 reads it through this path, and the InterfaceHost fixture belongs with M6's driver. The SkyShader and WaterShader slots are stubs with the right vtables, as in Phases 08 and 09.

pub(crate) mod common;

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::ThreadId;
use std::time::Instant;

use bevy_ecs::prelude::*;
use common::load::TestHost;
use common::rows::{RowBook, hex_bytes};
use recomp_core::crt::MsvcRand;
use recomp_core::fixtures;
use recomp_core::reader::StreamError;
use recomp_game::bs::archive::{ArchiveAccess, ArchiveManager, ArchiveSettings};
use recomp_game::bs::file::{GameFile, OpenedFile};
use recomp_game::bs::file_finder::{
    BsFileCallbacks, FileFinder, bs_file_exists, bs_file_open_core,
};
use recomp_game::bs::model_loader::{
    IoTaskId, ModelBuildServices, ModelDrain, ModelDrainStep, QUEUED_MODEL_VTABLE, QueuedModel,
    model_map_bucket, model_tree_rows,
};
use recomp_game::bs::scene_graphs::{SceneGraphGlobals, SceneGraphSettings};
use recomp_game::bs::shader::{BsShaderGlobals, BsShaderSlot};
use recomp_game::bs::shader_classes_geometry::{SkyShader, WaterShaderObject};
use recomp_game::bs::sky_shader::{
    PrepareGeometryContext, SKY_SHADER_SLOT_INDEX, SKY_SHADER_VTABLE, WATER_SHADER_SLOT_INDEX,
    WATER_SHADER_VTABLE,
};
use recomp_game::bs::stream::{BsStreamLoad, BsStreamServices, load_bs_stream_from_path};
use recomp_game::bs::task_manager::TaskGlobals;
use recomp_game::bs::texture_palette::BsTexturePalette;
use recomp_game::gamebryo::registration::{
    NiFactoryRegistry, NiProcessDefaults, StartupCallbackScheduler, register_known_crt_callbacks,
};
use recomp_game::gamebryo::scene_update::{NoSceneControllers, SceneControllers};
use recomp_game::gamebryo::search_path::bs_search_path_candidates;
use recomp_game::gamebryo::stream_postload::StreamPostLoadRuntime;
use recomp_game::gamebryo::texture::{SourceTexture, SourceTextureLoadServices};
use recomp_game::gamebryo::texture_loading::{
    TextureLoadServices, create_texture_from_file, load_shader_texture,
};
use recomp_game::tes::actor::{Actor, Character, MobileObject, TesObjectRefr};
use recomp_game::tes::actor_process::{
    ProcessInputs, RandomSource, construct_actor_process_manager,
};
use recomp_game::tes::base_extra_list::{EXTRA_CONTAINER_CHANGES, ExtraData};
use recomp_game::tes::character_controller::{
    BhkCharacterController, ControllerInputs, HAVOK_SCALE, actor_character_controller,
};
use recomp_game::tes::data_handler::{
    DataHandler, DataHandlerLists, FormsById, build_data_handler_file_list, construct_data_handler,
};
use recomp_game::tes::data_load::{DataLoadGlobals, DataLoadServices};
use recomp_game::tes::facegen::{FaceGenMatrix, FaceGenSingleton, FaceGenState};
use recomp_game::tes::form::{FormIdentity, OriginalVtable};
use recomp_game::tes::forms_init::QuestDelayGlobals;
use recomp_game::tes::forms_script::Script;
use recomp_game::tes::forms_world::{CellRegionLocks, TesObjectCell, TesWorldSpace};
use recomp_game::tes::game_settings::GameSettingCollection;
use recomp_game::tes::inventory::InventoryGlobals;
use recomp_game::tes::os_globals::{
    HostWindow, OsGlobalsData, OsSoundGlobals, ScriptedOsPlatform, SoundSource, SoundSourceGlobal,
};
use recomp_game::tes::paths::GamePaths;
use recomp_game::tes::player::{
    PLAYER_FORM_ID, PlayerCharacter, PlayerGlobals, PlayerInputs, player_setting_inputs,
};
use recomp_game::tes::settings::{
    SettingCollections, archive_settings_from, register_static_settings,
};
use recomp_game::tes::sky::SkyInitServices;
use recomp_game::tes::tes::{
    FLAME_NODE_FORM_IDS, GameThreadLocals, TesConstructionInputs, TesGlobals, TesPreloadServices,
    tes_setting_inputs,
};
use recomp_game::tes::water::{WaterServices, WaterTextureServices};
use recomp_game::tes::window::TimeInfoState;
use recomp_startup::initialize_game_data::{
    PluginBlockInputs, ScriptsSoundInputs, SkyPreludeInputs, TesBlockInputs,
    run_load_selected_files, run_player_base_observed, run_player_init, run_player_window,
    run_plugin_block, run_scripts_sound_and_clamp, run_sky_prelude, run_tes_block, run_tes_preload,
};
use recomp_startup::initialize_scene_graphs::{SceneGraphInputs, initialize_scene_graphs};

fn pinned_collections() -> SettingCollections {
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    collections
        .ini
        .set_path(fixtures::root().join("config/Oblivion.ini"));
    assert!(collections.ini.open_ini());
    collections.ini.read_all();
    collections.ini.close();
    collections
}

pub(crate) struct Harness {
    pub root: PathBuf,
    callbacks: BsFileCallbacks,
    finder: FileFinder,
    archives: RefCell<ArchiveManager>,
    registry: NiFactoryRegistry,
}

impl Harness {
    fn new() -> Self {
        let data = fixtures::data_dir();
        assert!(
            data.join("Oblivion - Meshes.bsa").exists(),
            "this test needs the game data: set OBLIVION_DATA_DIR ({data:?})"
        );
        let root = data.parent().unwrap().canonicalize().unwrap();
        let mut callbacks = BsFileCallbacks::default();
        let mut finder = FileFinder::new(&root, &mut callbacks);
        finder.add("Data");
        let settings = ArchiveSettings {
            invalidate_older_files: false,
            retain_directory_strings: 2,
            ..ArchiveSettings::default()
        };
        let scratch = std::env::temp_dir().join(format!(
            "recomp_game_data_window_{}_{}",
            std::process::id(),
            std::thread::current()
                .name()
                .unwrap_or("x")
                .replace("::", "_")
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("Plugins.txt"), "Oblivion.esm\n").unwrap();
        let mut archives = ArchiveManager::new(&root, settings);
        archives.initialize_from_settings(&finder, &scratch);
        let _ = std::fs::remove_dir_all(&scratch);
        let mut registry = NiFactoryRegistry::default();
        registry.register_known_crt_factories();
        registry.register_winmain_bs_factories();
        // 0088B17A: TES_Constructor's, which this window doesn't run.
        for (name, constructor) in recomp_game::bs::havok::BHK_FACTORIES {
            registry.insert_or_assign(name, constructor);
        }
        Self {
            root,
            callbacks,
            finder,
            archives: RefCell::new(archives),
            registry,
        }
    }

    // IDA: N/A a second FileFinder, for the host that owns its own.
    fn fresh_finder(&self) -> FileFinder {
        let mut callbacks = BsFileCallbacks::default();
        let mut finder = FileFinder::new(&self.root, &mut callbacks);
        finder.add("Data");
        finder
    }

    // IDA: N/A a second ArchiveManager over the same files, on the run's [Archive] settings and Plugins.txt, since 0042C970 reads them. 00B338E0 is one global in the original: these answer the same records, and nothing here counts on sharing them.
    fn fresh_archives(&self, settings: ArchiveSettings) -> ArchiveManager {
        let mut archives = ArchiveManager::new(&self.root, settings);
        archives.initialize_from_settings(&self.fresh_finder(), &fixtures::root().join("config"));
        archives
    }
}

/// IDA: N/A 007023E0's services: no BSTexturePalette, so lookups miss and registrations drop. Existence goes through the ported archives.
struct NoPalette<'a>(&'a Harness);

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
        let path: String = path.iter().map(|&byte| char::from(byte)).collect();
        bs_file_exists(
            &self.0.callbacks,
            &self.0.root,
            Some(&self.0.finder),
            Some(&mut self.0.archives.borrow_mut()),
            &path,
            0,
        )
    }
}

fn boxed(file: OpenedFile) -> Box<dyn GameFile> {
    match file {
        OpenedFile::Ni(file) => Box::new(file),
        OpenedFile::Loose(file) => Box::new(file),
        OpenedFile::Packed(file) => Box::new(file),
        OpenedFile::Compressed(file) => Box::new(file),
    }
}

fn load_stream(
    harness: &Harness,
    world: &mut World,
    path: &str,
) -> Result<BsStreamLoad, StreamError> {
    let mut open = |path: &str, mode: i32, size: u32| {
        bs_file_open_core(
            &harness.finder,
            Some(&mut harness.archives.borrow_mut()),
            path,
            mode,
            size,
            0xFFFF,
        )
        .map(boxed)
    };
    let mut rand = || 0;
    let mut textures = NoPalette(harness);
    load_bs_stream_from_path(
        world,
        path,
        &mut BsStreamServices {
            open: &mut open,
            registry: &harness.registry,
            draw_rand: Some(&mut rand),
            draw_on_io_thread: false,
            source_textures: Some(&mut textures),
            post_load: StreamPostLoadRuntime::default(),
        },
    )
}

/// IDA: N/A a scripted 0047DF80 CRT: the run's draw, as Phase 11 injects it.
struct ScriptedRandom {
    time64: i64,
    rand: i32,
}

impl RandomSource for ScriptedRandom {
    fn time64(&mut self) -> i64 {
        self.time64
    }
    fn srand(&mut self, _seed: u32) {}
    fn rand(&mut self) -> i32 {
        self.rand
    }
}

fn row(name: &str, prefix: &str) -> String {
    fixtures::rows(name)
        .into_iter()
        .find(|line| line.starts_with(prefix))
        .unwrap_or_else(|| panic!("no {prefix} row in {name}"))
}

fn field<'a>(line: &'a str, key: &str) -> &'a str {
    let at = line
        .find(&format!(" {key}="))
        .unwrap_or_else(|| panic!("missing key {key}"));
    line[at + key.len() + 2..].split(' ').next().unwrap()
}

// IDA: N/A 00406360 from the Phase 05 scene graphs through its `retn 4` at 00406944, with no block's result stubbed. Completion: every block from 00406390 on.
#[test]
fn the_whole_of_00406360_runs_as_one_sequence() {
    run_00406360();
}

/// IDA: N/A all of `00406360` and its checks, leaving the world as `0040EB2C` returns. `phase14_ai` goes on from here.
pub(crate) fn run_00406360() -> World {
    run_00406360_seeded(None).world
}

/// IDA: N/A what 0040EB2C leaves, for the blocks that go on from it.
#[allow(dead_code, reason = "each test binary uses only part of it")]
pub(crate) struct GameWindow {
    pub world: World,
    pub collections: SettingCollections,
    pub harness: Harness,
    pub host: TestHost,
}

/// IDA: N/A `run_00406360`, with 0047DF80's holdrand at 12d's first roll set to a recorded stream's. `None` is the unseeded CRT, a harness value.
pub(crate) fn run_00406360_seeded(seed: Option<u32>) -> GameWindow {
    let harness = Harness::new();
    let mut collections = pinned_collections();
    let mut world = World::new();
    // The real run has the LoadingMenu up through this window: `the_script_vm_runs_what_the_census_counted`.
    world.insert_resource(recomp_game::tes::script_vm::ScriptMenuModeAnswer(true));
    let mut known = StartupCallbackScheduler::default();
    register_known_crt_callbacks(&mut known).unwrap();
    known.run(&mut world).unwrap();
    world.insert_resource(GameSettingCollection::default());
    construct_actor_process_manager(&mut world);
    let defaults = world
        .resource::<NiProcessDefaults>()
        .scene_property_defaults();

    let mut scenes = SceneGraphGlobals::default();
    let mut particles = recomp_game::gamebryo::particle_controller_schedule::ParticleControllerScheduleContext::default();
    let mut runtime = SceneControllers {
        particles: &mut particles,
        tile_textures: None,
    };
    initialize_scene_graphs(
        &mut world,
        &mut scenes,
        &mut SceneGraphInputs {
            ini: &collections.ini,
            renderer_width: 1920,
            renderer_height: 1080,
            render_path: 5,
            store_shader_flags: Box::new(|_| {}),
            renderer_present: true,
            image_space_effects: 1,
            lod_far_mode: false,
            tes_present: false,
            initialize_shader_manager: None,
            create_render_targets: None,
            load_test_sphere: None,
            property_defaults: Some(defaults),
            fallback_base_texture: None,
        },
        &mut runtime,
    )
    .unwrap();

    let mut sky_shader = SkyShader::default();
    sky_shader.base.original_vtable = SKY_SHADER_VTABLE;
    sky_shader.base.class_name = "SkyShader";
    let sky_slot = Arc::new(BsShaderSlot {
        declaration: None,
        shader: Some(Arc::new(sky_shader)),
    });
    let mut water_shader = WaterShaderObject::default();
    water_shader.base.original_vtable = WATER_SHADER_VTABLE;
    water_shader.base.class_name = "WaterShader";
    let water_slot = Arc::new(BsShaderSlot {
        declaration: None,
        shader: Some(Arc::new(water_shader)),
    });
    let mut get_shader = |index: u32| match index {
        SKY_SHADER_SLOT_INDEX => Some(Arc::clone(&sky_slot)),
        WATER_SHADER_SLOT_INDEX => Some(Arc::clone(&water_slot)),
        _ => None,
    };
    let mut prepare = PrepareGeometryContext {
        render_path: 5,
        hdr: true,
        reference_classifier: scenes.reference_classifier,
        property_defaults: defaults,
        get_shader: &mut get_shader,
        warnings: Vec::new(),
        sky_shader_trace: Vec::new(),
        water_shader_trace: Vec::new(),
        lava_b47844: 0,
        shaders_30_b42ea5: false,
        load_normal_map: None,
        unported_shader_setup: None,
        classify_trace: Vec::new(),
    };

    // Phase 08.
    let mut load = |world: &mut World, path: &str| load_stream(&harness, world, path);
    let raw = |address: u32| collections.ini.find_address(address).unwrap().raw;
    let mut init = SkyInitServices {
        renderer_hdr: true,
        precipitation: raw(0x00B1_1DE4) & 0xFF != 0,
        sun_base_size: f32::from_bits(raw(0x00B1_1E2C)),
        sun_glare_size: f32::from_bits(raw(0x00B1_1E34)),
        world_scene: scenes.world,
        weather: scenes.weather,
        load_stream: &mut load,
        warnings: Vec::new(),
    };
    let mut log_sink = |_: &'static str| {};
    let prelude = run_sky_prelude(
        &mut world,
        SkyPreludeInputs {
            scenes: &scenes,
            init: &mut init,
            prepare: &mut prepare,
            property_defaults: defaults,
            update: &mut runtime,
            log: &mut log_sink,
        },
    )
    .unwrap();

    // Phase 09, with the real DataHandler and file list, which the Phase 09 world test stubs.
    let working = harness.root.clone();
    let mut registry = harness.registry.clone();
    let (havok_settings, debris_max_velocity, debris_min_extent, step_speed) = tes_setting_inputs(
        world.resource::<GameSettingCollection>(),
        &collections.ini,
        working.as_os_str().as_encoded_bytes().to_vec(),
    )
    .unwrap();
    let mut list_warnings = Vec::new();
    let mut construct = |world: &mut World| construct_data_handler(world);
    let mut build = |world: &mut World, path: &str| {
        build_data_handler_file_list(world, path, &working, &mut |text| list_warnings.push(text))
    };
    let mut gate = |_: &mut World| {};
    let mut shader_globals = BsShaderGlobals::default();
    let mut water_warnings = Vec::new();
    let mut warning = |text: String| water_warnings.push(text);
    let mut tes_log = Vec::new();
    let mut log_sink = |text: &'static str| tes_log.push(text);
    run_tes_block(
        &mut world,
        &prelude,
        TesBlockInputs {
            ini: &mut collections.ini,
            construction: TesConstructionInputs {
                registry: &mut registry,
                havok_settings,
                data_path: "Data\\",
                scene_root: None,
                land_lod: None,
                debris_max_velocity,
                debris_min_extent,
                step_speed,
                construct_data_handler: &mut construct,
                build_file_list: &mut build,
                enable_object_load_callbacks: &mut gate,
                fail_tes_allocation: false,
            },
            water: WaterServices {
                shader: Some(&mut shader_globals),
                image_space_effects: 1,
                render_path: 5,
                prepare: Some(&mut prepare),
                textures: None,
                warning: &mut warning,
            },
            world_scene: scenes.world,
            property_defaults: defaults,
            update: &mut runtime,
            log: &mut log_sink,
        },
    )
    .unwrap();
    assert!(list_warnings.is_empty(), "{list_warnings:?}");
    assert!(water_warnings.is_empty(), "{water_warnings:?}");

    // Phase 10: the run's Plugins.txt and INI.
    let paths = GamePaths {
        use_my_games: true,
        local_data_directory: fixtures::root().join("config"),
        ..GamePaths::default()
    };
    let mut plugin_warnings = Vec::new();
    run_plugin_block(
        &mut world,
        PluginBlockInputs {
            ini: &collections.ini,
            paths: &paths,
            warning: &mut |text| plugin_warnings.push(text),
        },
    )
    .unwrap();
    assert!(plugin_warnings.is_empty(), "{plugin_warnings:?}");
    let data = world.resource::<DataHandler>();
    let files: Vec<String> = data
        .files_8c8
        .iter()
        .enumerate()
        .map(|(index, &id)| {
            let file = data.file_store.get(id).unwrap();
            format!(
                "F after file{index} name={} flags={:X}",
                file.name_1c, file.flags_3dc
            )
        })
        .collect();
    let expected: Vec<String> = fixtures::rows("phase10")
        .into_iter()
        .filter(|line| line.starts_with("F after file"))
        .collect();
    assert_eq!(files, expected, "P10-RUN-001's file list and flags");

    // Phase 11: the run's draw and tick, with its settings read from what the earlier blocks left.
    let inputs_row = row("phase11", "I ");
    let settings =
        player_setting_inputs(world.resource::<GameSettingCollection>(), &collections.ini).unwrap();
    let tick = u32::from_str_radix(field(&inputs_row, "tick"), 16).unwrap();
    let time64 = i64::from(u32::from_str_radix(field(&inputs_row, "time64"), 16).unwrap());
    let scene_settings = SceneGraphSettings {
        renderer_width: 1920,
        renderer_height: 1080,
        default_fov: f32::from_bits(settings.default_fov),
        tes_present: true,
        ..SceneGraphSettings::default()
    };
    let mut random = ScriptedRandom {
        time64,
        rand: u32::from_str_radix(field(&inputs_row, "rand"), 16).unwrap() as i32,
    };
    let mut get_tick_count = move || tick;
    let mut time = move || time64;
    let mut scene_runtime = NoSceneControllers;
    let mut logs = Vec::new();
    let player = run_player_window(
        &mut world,
        &mut PlayerInputs {
            process: ProcessInputs {
                random: &mut random,
                idle_chatter_comment_timer: settings.idle_chatter_comment_timer,
                low_process_unset_88: u32::from_str_radix(field(&inputs_row, "low88"), 16).unwrap(),
                save_load_active: false,
            },
            controller: ControllerInputs::default(),
            settings,
            world_scene_graph: scenes.world,
            scene_settings: &scene_settings,
            scene_runtime: &mut scene_runtime,
            get_tick_count: &mut get_tick_count,
            time64: &mut time,
        },
        false,
        &mut |text| logs.push(text),
    )
    .unwrap()
    .player
    .unwrap();
    assert_eq!(logs, ["Loading Files..."]);
    assert_eq!(
        world.resource::<PlayerGlobals>().the_player_b333c4,
        Some(player)
    );
    let forms = world.resource::<FormsById>();
    let final_row = row("phase11", "F final ");
    assert_eq!(
        format!("{:X}", forms.count()),
        field(&final_row, "count"),
        "P11-RUN-001's form count: the 21 skills and the player"
    );
    assert_eq!(
        format!("{:X}", world.resource::<DataHandler>().next_form_id_8c0()),
        field(&final_row, "dh8C0"),
    );
    assert_eq!(forms.lookup(0x800), None, "0x800 released");
    assert_eq!(forms.lookup(PLAYER_FORM_ID), Some(player));

    // Phase 12: 004067C7 on the same World, after the run's Plugins.txt selected the files. The harness's Phase 04 stand-in is 00B39B80 with the coefficient sizes of the shipped si.ctl, as `common::load::Loader` builds it, since this window doesn't reach Phase 04.
    let mut facegen = Box::<FaceGenState>::default();
    for (matrix, rows) in facegen.coefficients.iter_mut().zip([50_u32, 30, 50]) {
        *matrix = FaceGenMatrix {
            rows,
            cols: 1,
            data: vec![0.0; rows as usize],
        };
    }
    world.insert_resource(FaceGenSingleton(Some(facegen)));
    world.get_resource_or_init::<GameThreadLocals>();
    let mut rand = MsvcRand::default();
    if let Some(seed) = seed {
        rand.srand(seed);
    }
    let mut host = TestHost {
        real_pre_load: true,
        real_tail: true,
        archives: Some(harness.fresh_archives(archive_settings_from(&collections.ini))),
        finder: Some(harness.fresh_finder()),
        rand,
        ..TestHost::default()
    };
    let mut warnings = Vec::new();
    let load_start = Instant::now();
    let result = run_load_selected_files(
        &mut world,
        &collections.ini,
        &mut DataLoadServices {
            working_directory: &harness.root,
            warning: &mut |text, _| warnings.push(text),
            host: &mut host,
        },
    )
    .unwrap();
    println!(
        "004067C7, the file pass and its tail: {:.2?}",
        load_start.elapsed()
    );
    assert_eq!(result, 1);
    assert!(
        warnings.is_empty(),
        "{:?}",
        &warnings[..warnings.len().min(5)]
    );
    let records = evidence_lines("phase12-run001-2026-10-09");
    let data = world.resource::<DataHandler>();
    let compiled: Vec<&String> = records
        .iter()
        .filter(|line| line.starts_with("T ") && line.contains(" 44F6CB compiled "))
        .collect();
    assert_eq!(data.loaded_mod_count_8d0 as usize, compiled.len());
    assert_eq!(
        data.next_form_id_8c0(),
        0xFF00_0800,
        "P12-RUN-001 record 24"
    );
    for (index, line) in compiled.iter().enumerate() {
        let file = data
            .file_store
            .get(data.mods_by_id_8d4[index].unwrap())
            .unwrap();
        assert_eq!(field(line, "name"), file.name_1c);
        assert_eq!(field(line, "flags"), format!("{:X}", file.flags_3dc));
    }
    let load = world.resource::<DataLoadGlobals>();
    let totals = records
        .iter()
        .find(|line| line.contains(" TOTALS "))
        .unwrap();
    assert_eq!(
        field(totals, "B33A90"),
        format!("{:X}", load.record_total_b33a90)
    );
    let last = records
        .iter()
        .find(|line| line.starts_with("M 714 "))
        .unwrap();
    assert!(
        field(last, "B33A90").starts_with(&format!(
            "{:08x}{:08x}",
            load.record_total_b33a90.swap_bytes(),
            load.record_count_b33a94.swap_bytes()
        )),
        "P12-RUN-001 record 714"
    );
    // The player keeps its form through the load.
    assert_eq!(
        world.resource::<FormsById>().lookup(PLAYER_FORM_ID),
        Some(player)
    );

    run_phase13_tail(&mut world, &harness, &collections, &mut host, player);
    GameWindow {
        world,
        collections,
        harness,
        host,
    }
}

// ---- Phase 13: 004067CC to the end of 00406360 ----

pub(crate) fn evidence_lines(run: &str) -> Vec<String> {
    let path = fixtures::root().join(format!("evidence/{run}.records.txt"));
    std::fs::read(path)
        .unwrap()
        .split(|&byte| byte == b'\n')
        .map(|line| {
            line.iter()
                .filter(|&&byte| byte != b'\r')
                .map(|&byte| char::from(byte))
                .collect()
        })
        .collect()
}

fn row_field<'a>(parts: &[&'a str], key: &str) -> &'a str {
    let prefix = format!("{key}=");
    parts
        .iter()
        .find_map(|part| part.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("no {key} in {parts:?}"))
}

fn triple(values: [f32; 3]) -> String {
    format!(
        "{:08X},{:08X},{:08X}",
        values[0].to_bits(),
        values[1].to_bits(),
        values[2].to_bits()
    )
}

// IDA: N/A one PLAYER row against the host player.
fn check_player(world: &World, book: &RowBook, tag: &str, player: Entity) {
    let row = book.take_one("PLAYER", tag);
    let identity = world.get::<FormIdentity>(player).unwrap();
    let refr = world.get::<TesObjectRefr>(player).unwrap();
    assert_eq!(
        format!("{:X}", identity.flags_08),
        row_field(&row, "flags"),
        "flags at {tag}"
    );
    let npc = world.resource::<FormsById>().lookup(7);
    assert_eq!(
        refr.base_form_1c.is_some() && refr.base_form_1c == npc,
        row_field(&row, "base_is_npc7") == "1",
        "base at {tag}"
    );
    assert_eq!(
        triple(refr.rotation_20),
        row_field(&row, "rot"),
        "rotation at {tag}"
    );
    assert_eq!(
        triple(refr.position_2c),
        row_field(&row, "pos"),
        "position at {tag}"
    );
    assert_eq!(
        format!("{:08X}", refr.scale_38.to_bits()),
        row_field(&row, "scale"),
        "scale at {tag}"
    );
}

// IDA: N/A one CTRL row: the player's controller +0x1F4 and +0x320.
fn check_controller(world: &World, book: &RowBook, tag: &str, player: Entity) {
    let row = book.take_one("CTRL", tag);
    let controller = actor_character_controller(world, player).expect("Phase 11's controller");
    let body = world.get::<BhkCharacterController>(controller).unwrap();
    assert_eq!(
        format!("{:X}", body.flags_1f4),
        row_field(&row, "f1f4"),
        "+0x1F4 at {tag}"
    );
    assert_eq!(
        format!("{:08X}", body.floats_320[0].to_bits()),
        row_field(&row, "f320"),
        "+0x320 at {tag}"
    );
}

// IDA: N/A one EXTRAS row: the player's +0x44 head.
fn check_extras(world: &World, book: &RowBook, tag: &str, player: Entity) {
    let row = book.take_one("EXTRAS", tag);
    let refr = world.get::<TesObjectRefr>(player).unwrap();
    assert_eq!(
        if refr.extra_data_44.entries.is_empty() {
            "0"
        } else {
            "nonzero"
        },
        row_field(&row, "head"),
        "+0x44 at {tag}"
    );
}

// IDA: N/A P13-RUN-001's whole 2,052-byte player dumps say which dwords moved between two points. The host's parts of the player, apart from what's named, must not have moved either.
fn raw_changes(records: &[String], from: &str, to: &str) -> Vec<usize> {
    let raw = |tag: &str| -> Vec<u8> {
        let line = records
            .iter()
            .find(|line| line.starts_with("P ") && line.split(' ').nth(3) == Some(tag))
            .unwrap();
        hex_bytes(field(line, "raw"))
    };
    let (first, second) = (raw(from), raw(to));
    assert_eq!(first.len(), 0x804);
    (0..first.len() / 4)
        .map(|at| at * 4)
        .filter(|&at| first[at..at + 4] != second[at..at + 4])
        .collect()
}

/// IDA: N/A a snapshot of the player's parts, past what the rows name.
#[derive(Clone, PartialEq, Debug)]
struct PlayerParts {
    actor: Actor,
    character: Character,
    mobile: MobileObject,
    body: PlayerCharacter,
    controller: BhkCharacterController,
    refr: TesObjectRefr,
}

fn player_parts(world: &World, player: Entity) -> PlayerParts {
    let mut controller = world
        .get::<BhkCharacterController>(actor_character_controller(world, player).unwrap())
        .unwrap()
        .clone();
    // The SDK host records aren't original bytes.
    controller.host_proxy_position = [0.0; 3];
    controller.host_proxy_set_position_calls = 0;
    controller.host_world_mark_calls = 0;
    PlayerParts {
        actor: world.get::<Actor>(player).unwrap().clone(),
        character: *world.get::<Character>(player).unwrap(),
        mobile: *world.get::<MobileObject>(player).unwrap(),
        body: world.get::<PlayerCharacter>(player).unwrap().clone(),
        controller,
        refr: world.get::<TesObjectRefr>(player).unwrap().clone(),
    }
}

/// IDA: N/A 00442890's real targets over the harness: the palette, the FileFinder, 00701E00 with 00B256D0 {6, 3, 2} and g_IOManager +0x30.
struct DecalTextures<'a> {
    harness: &'a Harness,
    io_thread: ThreadId,
    created: Vec<Vec<u8>>,
}

impl WaterTextureServices for DecalTextures<'_> {
    fn palette_find(&mut self, world: &mut World, path: &[u8]) -> Option<Entity> {
        world
            .resource::<BsTexturePalette>()
            .find(path, Some(&mut self.harness.archives.borrow_mut()))
    }
    fn palette_add(&mut self, world: &mut World, path: &[u8], texture: Entity) {
        world.resource_mut::<BsTexturePalette>().add(
            path,
            Some(texture),
            Some(&mut self.harness.archives.borrow_mut()),
        );
    }
    fn find_file(&mut self, path: &[u8], _flags: u32) -> Option<(u32, Vec<u8>)> {
        let text: String = path.iter().map(|&byte| char::from(byte)).collect();
        let found = bs_file_exists(
            &self.harness.callbacks,
            &self.harness.root,
            Some(&self.harness.finder),
            Some(&mut self.harness.archives.borrow_mut()),
            &text,
            0,
        );
        Some((u32::from(found), path.to_vec()))
    }
    fn create_from_file(&mut self, world: &mut World, path: &[u8]) -> Option<Entity> {
        self.created.push(path.to_vec());
        // The 00B256CC gate off: no device on this window, so no renderer data (vt+0x5C 00701C00). A harness substitution.
        create_texture_from_file(
            world,
            path,
            [6, 3, 2],
            true,
            &mut TextureLoadServices {
                create_renderer_data_on_create: false,
                ..TextureLoadServices::default()
            },
        )
    }
    fn io_thread_id(&self) -> ThreadId {
        self.io_thread
    }
}

/// IDA: N/A 0x00A90634, the ShadowLight family's vtable, whose +0x18 0080B190 is the counted boundary, on a stub shader as the C++ test uses.
const SHADOW_LIGHT_VTABLE: u32 = 0x00A9_0634;

// IDA: N/A the model map's entries at this moment: the host's models and the gated tasks the walk has passed, each with its bucket and key. P13-RUN-001 counts what the original built: the host builds these too, or holds them at a `host_gate`.
fn model_map_view(world: &World) -> Vec<(u32, Vec<u8>)> {
    let tasks = world.resource::<TaskGlobals>();
    let loader = tasks.model_loader_b33a1c.as_ref().unwrap();
    let mut entries: Vec<(u32, Vec<u8>)> = loader
        .models
        .buckets
        .iter()
        .enumerate()
        .flat_map(|(bucket, chain)| {
            chain
                .iter()
                .map(move |entry| (bucket as u32, entry.key.clone()))
        })
        .collect();
    for task in tasks.io_manager_b33a10.as_ref().unwrap().tasks.queued() {
        if task.host_gate.is_some() {
            let key = task.path_20.to_ascii_lowercase();
            entries.push((model_map_bucket(&key), key));
        }
    }
    entries.sort();
    entries.dedup();
    entries
}

// IDA: N/A a recorded `MM` line's entries (bucket, key). Its key prints empty when the run read a node mid-insert, so the root's "BASE " name gives it.
fn recorded_model_map(line: &str) -> Vec<(u32, Vec<u8>)> {
    let list = line.split("entries=").nth(1).unwrap();
    let mut entries: Vec<(u32, Vec<u8>)> = list
        .split_once(' ')
        .map_or("", |(_, rest)| rest)
        .split(" | ")
        .map(|entry| {
            let parts: Vec<&str> = entry.splitn(5, ':').collect();
            let bucket = u32::from_str_radix(parts[0], 16).unwrap();
            let key = if parts[1].is_empty() {
                let name = parts[4].rsplit(':').next().unwrap();
                name.strip_prefix("BASE ")
                    .unwrap()
                    .as_bytes()
                    .to_ascii_lowercase()
            } else {
                parts[1].as_bytes().to_vec()
            };
            assert_eq!(model_map_bucket(&key), bucket, "{entry}");
            (bucket, key)
        })
        .collect();
    entries.sort();
    entries
}

// IDA: N/A 00433BC0 on the World the tail left, walking in P13-RUN-001's order: the only timing given is when tasks reach the map, and the walk (00433760) does the rest. Task 0 alone is in the map when the thread wakes, so it runs first, from the top of the keys. The walk restarts at the lowest key of what has arrived, and the map's 3 entries at 004067D7 (stoneDustCloud, metalSparks, lichBloodSpray) need metalSparks to be the lowest with lichBloodSpray next, so tasks 1 to 15 have arrived and task 16 (FireTorchSmall, below them) hasn't. Then all of them are in: the walk goes on upward past lichBloodSpray, and the 8 entries at 00406944 are exactly the tasks up to waterSplash, the last key before the wrap. Each `MM` line's entries are compared with the model map plus the gated tasks passed.
fn check_model_drain(
    world: &mut World,
    harness: &Harness,
    settings: &ArchiveSettings,
    book: &RowBook,
    records: &[String],
) {
    let defaults = world
        .resource::<NiProcessDefaults>()
        .scene_property_defaults();
    let held_back = {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let io = tasks.io_manager_b33a10.as_mut().unwrap();
        std::mem::take(&mut io.tasks.buckets[16])
    };
    assert_eq!(held_back.len(), 26);
    let arrive = |world: &mut World, up_to: u64| {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let io = tasks.io_manager_b33a10.as_mut().unwrap();
        let store = &io.tasks.store;
        let key = |id: &IoTaskId| store[id.0].as_ref().unwrap().key_10;
        let mut arrived: Vec<IoTaskId> = held_back
            .iter()
            .copied()
            .filter(|id| store[id.0].is_some() && key(id) & 0xFFFF <= up_to)
            .collect();
        arrived.sort_by_key(|id| key(id) as i64);
        io.tasks.buckets[16] = arrived;
    };

    let palette = RefCell::new(std::collections::HashMap::<Vec<u8>, Entity>::new());
    let mut archives = harness.fresh_archives(settings.clone());
    let mut normal_archives = harness.fresh_archives(settings.clone());
    let (mut setups, mut slots) = (Vec::new(), Vec::new());
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
        let (read, write, normal) = (&palette, &palette, &mut normal_archives);
        load_shader_texture(
            world,
            path,
            true,
            true,
            &mut TextureLoadServices {
                create_renderer_data_on_create: false,
                file_exists: Some(Box::new(move |path: &[u8]| {
                    let text: String = path.iter().map(|&byte| char::from(byte)).collect();
                    bs_file_exists(
                        &harness.callbacks,
                        &harness.root,
                        Some(&harness.finder),
                        Some(&mut *normal),
                        &text,
                        0,
                    )
                })),
                palette_find: Some(Box::new(move |path: &[u8]| {
                    read.borrow().get(path).copied()
                })),
                palette_add: Some(Box::new(move |path: &[u8], texture: Option<Entity>| {
                    if let Some(texture) = texture {
                        write.borrow_mut().insert(path.to_vec(), texture);
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
    let mut disabled = 1_u8;
    let mut warnings = Vec::new();
    let mut warning = |text: Vec<u8>, _: bool| warnings.push(text);
    let mut open = |path: &str, mode: i32, size: u32| {
        bs_file_open_core(
            &harness.finder,
            Some(&mut harness.archives.borrow_mut()),
            path,
            mode,
            size,
            0xFFFF,
        )
        .map(boxed)
    };
    let mut io_rand = MsvcRand::default();
    let mut draw = || io_rand.rand();
    let mut textures = NoPalette(harness);
    let mut services = ModelBuildServices {
        archives: Some(&mut archives),
        stream: BsStreamServices {
            open: &mut open,
            registry: &harness.registry,
            draw_rand: Some(&mut draw),
            draw_on_io_thread: true,
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
    let run_step = |drain: &mut ModelDrain,
                    world: &mut World,
                    services: &mut ModelBuildServices<'_>,
                    until: &[u8]| {
        loop {
            let step = drain.step(world, services).unwrap();
            assert_ne!(step, ModelDrainStep::Ended, "the walk ends early");
            assert_ne!(step, ModelDrainStep::WaitingForToken);
            if step == ModelDrainStep::Ran
                && drain
                    .record
                    .tasks
                    .last()
                    .unwrap()
                    .path
                    .eq_ignore_ascii_case(until)
            {
                return;
            }
        }
    };
    let started = Instant::now();
    let mut drain = ModelDrain::new();
    // The thread wakes on task 0.
    arrive(world, 1);
    run_step(
        &mut drain,
        world,
        &mut services,
        b"Effects\\stoneDustCloud.NIF",
    );
    arrive(world, 0x10);
    run_step(
        &mut drain,
        world,
        &mut services,
        b"Effects\\metalSparks.NIF",
    );
    run_step(
        &mut drain,
        world,
        &mut services,
        b"Effects\\lichBloodSpray.nif",
    );
    let row = book.take_one("MODELMAP", "after_preload");
    let view = model_map_view(world);
    assert_eq!(view.len().to_string(), row[2], "P13-RUN-001 at 004067D7");
    let line = records
        .iter()
        .find(|line| line.starts_with("MM ") && line.contains(" after_preload "))
        .unwrap();
    assert_eq!(view, recorded_model_map(line), "the entries at 004067D7");
    // The queue has finished: every task is in the map.
    arrive(world, 0xFFFF);
    run_step(
        &mut drain,
        world,
        &mut services,
        b"Effects\\waterSplash.NIF",
    );
    let row = book.take_one("MODELMAP", "final");
    let view = model_map_view(world);
    assert_eq!(view.len().to_string(), row[2], "P13-RUN-001 at 00406944");
    let line = records
        .iter()
        .find(|line| line.starts_with("MM ") && line.contains(" final "))
        .unwrap();
    assert_eq!(view, recorded_model_map(line), "the entries at 00406944");
    let walked: Vec<String> = drain
        .record
        .tasks
        .iter()
        .map(|task| String::from_utf8_lossy(&task.path).into_owned())
        .collect();
    assert_eq!(
        walked.len(),
        16,
        "stone, metal, lich, wood, skeleton, blood, 9 dust, water: {walked:?}"
    );
    assert_eq!(
        drain.run_to_end(world, &mut services).unwrap(),
        ModelDrainStep::Ended
    );
    println!("00433BC0's drain, 26 tasks: {:.2?}", started.elapsed());
    drop(services);
    assert_eq!(
        drain.record.log_line(setups.len() as u32),
        "Phase-13e: model tasks run 26 (models built 18, cached 8, failed 0), queued 0 (0 Havok, 0 NIF 10.2.0.0), shader setups not run 2",
        "G3"
    );
    assert_eq!(slots, [1, 1]);
    assert_eq!(
        setups.iter().map(|setup| setup.0).collect::<Vec<_>>(),
        [SHADOW_LIGHT_VTABLE, SHADOW_LIGHT_VTABLE]
    );
    assert!(warnings.is_empty());
    assert_eq!(disabled, 1, "00436C67 restores the flag");
    let tasks = world.resource::<TaskGlobals>();
    let io = tasks.io_manager_b33a10.as_ref().unwrap();
    assert_eq!(io.tasks.buckets[16].len(), 0);
    assert_eq!(io.task_map.field_18, 0);
    assert_eq!(io.array_2c[16], 0);
    assert_eq!(tasks.counters.live_tasks_b33a20, 0);
    let loader = tasks.model_loader_b33a1c.as_ref().unwrap();
    assert_eq!(loader.model_map_00.field_18, 18);
    // P13-RUN-001 at 00406944: rc 9 for DustCloud, and rc 1 for the rest.
    let line = records
        .iter()
        .find(|line| line.starts_with("MM ") && line.contains(" final "))
        .unwrap();
    for entry in loader.models.buckets.iter().flatten() {
        let held = &loader.models.store[entry.model.0];
        let name = String::from_utf8_lossy(&entry.key).into_owned();
        if line.contains(&name) {
            let want = format!(":{name}:");
            let at = line.find(&want).unwrap();
            let rest = &line[at + want.len()..];
            let references = rest
                .split(":rc=")
                .nth(1)
                .unwrap()
                .split(':')
                .next()
                .unwrap();
            assert_eq!(format!("{:X}", held.references_04), references, "{name}");
        }
    }
    // Every model's tree on this World against P15-RUN-001, as `stream_graph_real` does on a small one.
    let (mut compared, mut boundary_rows) = (0, 0);
    for line in fixtures::rows("phase13e") {
        if let Some(rest) = line.strip_prefix("MODEL|") {
            let parts: Vec<&str> = rest.split('|').collect();
            let chain = &loader.models.buckets[model_map_bucket(parts[0].as_bytes()) as usize];
            let entry = chain
                .iter()
                .find(|entry| entry.key == parts[0].as_bytes())
                .unwrap();
            let held = &loader.models.store[entry.model.0];
            assert_eq!(held.references_04.to_string(), parts[1], "{line}");
            assert_eq!(held.path_00, parts[2].as_bytes(), "{line}");
            let trees = model_tree_rows(world, parts[0], held.root_08.unwrap());
            for tree_line in trees {
                let rows = fixtures::rows("phase13e");
                let wanted = rows
                    .iter()
                    .find(|row| {
                        let (a, b): (Vec<&str>, Vec<&str>) =
                            (row.split('|').collect(), tree_line.split('|').collect());
                        a[0] == "NODE" && a[1] == b[1] && a[2] == b[2]
                    })
                    .unwrap_or_else(|| panic!("no row for {tree_line}"));
                // Declared: 0080B190's property, on the one object that reaches the boundary. Every other field stays exact.
                if wanted.contains("BSShaderPPLightingProperty") {
                    fixtures::ledger_relaxed(
                        "phase13e",
                        &format!(
                            "{} #7",
                            wanted
                                .split("|BS")
                                .next()
                                .unwrap_or(wanted)
                                .rsplit_once('|')
                                .map_or(wanted.as_str(), |pair| pair.0)
                        ),
                        1,
                        "0080B190's property: the shader boundary keeps NiTexturingProperty where the row has BSShaderPPLightingProperty (docs/open-items.md, 13)",
                    );
                    let fields = |line: &str| -> Vec<String> {
                        line.split('|')
                            .enumerate()
                            .filter(|(index, _)| *index != 7)
                            .map(|(_, field)| field.to_owned())
                            .collect()
                    };
                    assert_eq!(fields(&tree_line), fields(wanted));
                    boundary_rows += 1;
                } else {
                    assert_eq!(&tree_line, wanted);
                }
                compared += 1;
            }
        }
    }
    assert_eq!((compared, boundary_rows), (86, 1));
    check_no_records(world, loader);
}

// IDA: N/A P15-RUN-001's `NO` raw dumps of the 86 objects, at the MainMenu: +0x18 the flags; +0x14 the extra count; +0x30 the local transform, 13 words; +0x64 the world transform, identity with the models never updated; +0x20 the world bound, zero. Every word, bit for bit. 9 of the run's 300 objects hold a nonzero bound, none of them here.
fn check_no_records(world: &World, loader: &recomp_game::bs::task_manager::ModelLoader) {
    let records = evidence_lines("phase15-run001-2026-10-10");
    let word = |raw: &[u8], at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
    let mut objects = 0;
    let mut rotated = 0;
    for line in fixtures::rows("phase13e") {
        let Some(rest) = line.strip_prefix("MODEL|") else {
            continue;
        };
        let key = rest.split('|').next().unwrap();
        let chain = &loader.models.buckets[model_map_bucket(key.as_bytes()) as usize];
        let entry = chain
            .iter()
            .find(|entry| entry.key == key.as_bytes())
            .unwrap();
        let root = loader.models.store[entry.model.0].root_08.unwrap();
        for (path, object, flags, local) in
            recomp_game::bs::model_loader::model_tree_objects(world, root)
        {
            let what = format!("NO {key} {path}");
            let record = records
                .iter()
                .find(|record| {
                    record.starts_with("NO ")
                        && record.contains(&format!(" key={key} path={path} "))
                })
                .unwrap_or_else(|| panic!("{what}: no record"));
            let raw_text = record
                .split(' ')
                .find_map(|part| part.strip_prefix("raw="))
                .unwrap();
            let raw = hex_bytes(raw_text);
            assert_eq!(
                u16::from_le_bytes([raw[0x18], raw[0x19]]),
                flags,
                "{what} +0x18"
            );
            let net = world
                .get::<recomp_game::gamebryo::object_net::NiObjectNet>(object)
                .unwrap();
            assert_eq!(
                usize::from(u16::from_le_bytes([raw[0x14], raw[0x15]])),
                net.extra_data.len(),
                "{what} +0x14"
            );
            let host_local: Vec<u32> = local
                .rotation
                .iter()
                .chain(&local.translation)
                .chain(std::iter::once(&local.scale))
                .map(|value| value.to_bits())
                .collect();
            let run_local: Vec<u32> = (0..13).map(|index| word(&raw, 0x30 + index * 4)).collect();
            assert_eq!(host_local, run_local, "{what} +0x30");
            if run_local[..9] != [0x3F80_0000, 0, 0, 0, 0x3F80_0000, 0, 0, 0, 0x3F80_0000] {
                rotated += 1;
            }
            let world_state = world
                .get::<recomp_game::gamebryo::scene_node::NiAvWorld>(object)
                .copied()
                .unwrap_or_default();
            let host_world: Vec<u32> = world_state
                .transform
                .rotation
                .iter()
                .chain(&world_state.transform.translation)
                .chain(std::iter::once(&world_state.transform.scale))
                .map(|value| value.to_bits())
                .collect();
            let run_world: Vec<u32> = (0..13).map(|index| word(&raw, 0x64 + index * 4)).collect();
            assert_eq!(host_world, run_world, "{what} +0x64");
            for index in 0..4 {
                assert_eq!(
                    world_state.bound[index].to_bits(),
                    word(&raw, 0x20 + index * 4),
                    "{what} +0x20"
                );
            }
            objects += 1;
        }
    }
    assert_eq!(objects, 86);
    println!("NO: 86 objects' raw words match, {rotated} with a rotated local");
    check_added_models(world, loader, &records);
}

// The models the C++ left queued, whole, against the same dumps: 8 fires with their collision object, and 3 NIF 10.2.0.0
// effects (`bs::havok_stream`, `particle_data`). Every object of the run's tree, by path.
fn check_added_models(
    world: &World,
    loader: &recomp_game::bs::task_manager::ModelLoader,
    records: &[String],
) {
    let word = |raw: &[u8], at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
    let mut objects = 0;
    let mut collisions = 0;
    for key in [
        r"fire\fireopenlarge.nif",
        r"fire\fireopenlargesmoke.nif",
        r"fire\fireopenmedium.nif",
        r"fire\fireopenmediumsmoke.nif",
        r"fire\fireopensmall.nif",
        r"fire\fireopensmallsmoke.nif",
        r"fire\firetorchlarge.nif",
        r"fire\firetorchlargesmoke.nif",
        r"effects\metalsparks.nif",
        r"effects\skeletonbloodspray.nif",
        r"effects\stonedustcloud.nif",
    ] {
        let chain = &loader.models.buckets[model_map_bucket(key.as_bytes()) as usize];
        let entry = chain
            .iter()
            .find(|entry| entry.key == key.as_bytes())
            .unwrap_or_else(|| panic!("{key}: not in the model map"));
        let root = loader.models.store[entry.model.0].root_08.unwrap();
        let tree = recomp_game::bs::model_loader::model_tree_objects(world, root);
        let needle = format!(" key={key} ");
        let run: Vec<&String> = records
            .iter()
            .filter(|record| record.starts_with("NO ") && record.contains(&needle))
            .collect();
        assert_eq!(tree.len(), run.len(), "{key}: the tree's objects");
        for (path, object, flags, local) in tree {
            let what = format!("NO {key} {path}");
            let record = run
                .iter()
                .find(|record| record.contains(&format!(" path={path} ")))
                .unwrap_or_else(|| panic!("{what}: no record"));
            let field = |name: &str| {
                record
                    .split(' ')
                    .find_map(|part| part.strip_prefix(name))
                    .unwrap_or_else(|| panic!("{what}: no {name}"))
            };
            let raw = hex_bytes(field("raw="));
            assert_eq!(
                u16::from_le_bytes([raw[0x18], raw[0x19]]),
                flags,
                "{what} +0x18"
            );
            let host_local: Vec<u32> = local
                .rotation
                .iter()
                .chain(&local.translation)
                .chain(std::iter::once(&local.scale))
                .map(|value| value.to_bits())
                .collect();
            let run_local: Vec<u32> = (0..13).map(|index| word(&raw, 0x30 + index * 4)).collect();
            assert_eq!(host_local, run_local, "{what} +0x30");
            let net = world
                .get::<recomp_game::gamebryo::object_net::NiObjectNet>(object)
                .unwrap();
            assert_eq!(
                usize::from(u16::from_le_bytes([raw[0x14], raw[0x15]])),
                net.extra_data.len(),
                "{what} +0x14"
            );
            let collision = world
                .get::<recomp_game::gamebryo::av_object::NiAvCollision>(object)
                .is_some();
            assert_eq!(field("col=") != "0", collision, "{what} col");
            collisions += usize::from(collision);
            objects += 1;
        }
    }
    println!(
        "NO: {objects} objects of the 11 added models match, {collisions} with a collision object"
    );
    assert_eq!(collisions, 8, "one for each fire");
}

// IDA: N/A 004067CC..00406944 on the World the rest of the window left, every row read.
fn run_phase13_tail(
    world: &mut World,
    harness: &Harness,
    collections: &SettingCollections,
    host: &mut TestHost,
    player: Entity,
) {
    let book = RowBook::load("phase13");
    let records = evidence_lines("phase13-run001-2026-10-09");

    // ---- at 004067CC ----
    for row in book.take("MODELMAP", "before") {
        let loader = world
            .resource::<TaskGlobals>()
            .model_loader_b33a1c
            .as_ref()
            .unwrap();
        let count: usize = loader.models.buckets.iter().map(Vec::len).sum();
        assert_eq!(count.to_string(), row[2], "the model map before 00443550");
    }
    let flame = |world: &World| -> String {
        world
            .resource::<TesGlobals>()
            .flame_node_slots_b35e50
            .iter()
            .map(|slot| format!("{slot:08X}"))
            .collect()
    };
    let row = book.take_one("FLAME", "before");
    assert_eq!(flame(world), row[2], "00B35E50 before 00443550");
    check_player(world, &book, "before", player);
    check_controller(world, &book, "CONTROLLER", player);
    check_extras(world, &book, "before", player);
    assert_eq!(
        raw_changes(&records, "before", "after_base"),
        Vec::<usize>::new(),
        "RUN-001: +0x1C already holds the NPC, from 0044AC20"
    );
    // +0x704, 00B35EBC's prison doors: the head item and its next are both nonzero in the raw dump. A count needs a walk, so this only bounds it.
    let line = records
        .iter()
        .find(|line| line.starts_with("P ") && line.split(' ').nth(3) == Some("before"))
        .unwrap();
    let raw = hex_bytes(field(line, "raw"));
    assert_ne!(raw[0x704..0x708], [0; 4], "+0x704's item");
    assert_ne!(raw[0x708..0x70C], [0; 4], "+0x704's next");
    let markers = world
        .get::<PlayerCharacter>(player)
        .unwrap()
        .jail_markers_704
        .items()
        .count();
    println!("+0x704 holds {markers}");
    assert!(markers >= 2, "two nodes at least, with {markers}");

    // ---- 13a ----
    assert_eq!(
        world.resource::<TaskGlobals>().counters.sequence_b33a14,
        0,
        "nothing was queued before 00443550"
    );
    let main_thread = std::thread::current().id();
    let io_thread = world
        .resource::<TaskGlobals>()
        .io_manager_b33a10
        .as_ref()
        .unwrap()
        .main_thread_id_30;
    assert_ne!(
        main_thread, io_thread,
        "0044295A takes the pause arm on the main thread"
    );
    world.init_resource::<BsTexturePalette>();
    let mut decals = DecalTextures {
        harness,
        io_thread,
        created: Vec::new(),
    };
    let mut water_warnings = Vec::new();
    let mut warning = |text: String| water_warnings.push(text);
    let start = Instant::now();
    run_tes_preload(
        world,
        &mut TesPreloadServices {
            archives: host
                .archives
                .as_mut()
                .map(|archives| archives as &mut dyn ArchiveAccess),
            textures: WaterServices {
                shader: None,
                image_space_effects: 0,
                render_path: 0,
                prepare: None,
                textures: Some(&mut decals),
                warning: &mut warning,
            },
        },
    )
    .unwrap();
    println!("00443550: {:.2?}", start.elapsed());
    assert!(water_warnings.is_empty(), "{water_warnings:?}");
    // TASK: at 0043B420's return in the run, so the out's reference is still in `refs`, where the host reads after the release. The key is unique by its low word.
    let tasks = world.resource::<TaskGlobals>();
    let io = tasks.io_manager_b33a10.as_ref().unwrap();
    let mut queued: Vec<&QueuedModel> = io.tasks.queued().collect();
    queued.sort_by_key(|task| task.key_10 & 0xFFFF);
    let mut task_rows = 0;
    for (index, task) in queued.iter().enumerate() {
        let row = book.take("TASK", &index.to_string());
        assert_eq!(row.len(), 1, "TASK {index}");
        let row = &row[0];
        let what = format!("task {index} {}", row[2]);
        assert_eq!(task.path_20, row[2].as_bytes(), "{what}");
        assert_eq!(
            format!("{:016X}", task.key_10),
            row_field(row, "key"),
            "{what}: key"
        );
        assert_eq!(
            format!("{:X}", task.references_08 + 1),
            row_field(row, "refs"),
            "{what}: references"
        );
        assert_eq!(
            format!("{:X}", task.state_0c),
            row_field(row, "state"),
            "{what}: state"
        );
        assert!(task.parent_18.is_none());
        assert_eq!(row_field(row, "parent"), "0", "{what}");
        assert_eq!(
            format!("{:X}", task.children_1c),
            row_field(row, "children"),
            "{what}"
        );
        assert!(task.model_28.is_none());
        assert_eq!(row_field(row, "model"), "0", "{what}");
        assert_eq!(
            format!("{:X}", task.field_2c),
            row_field(row, "f2c"),
            "{what}"
        );
        assert_eq!(
            format!("{:X}", task.lod_30),
            row_field(row, "lod"),
            "{what}"
        );
        assert_eq!(
            format!("{:X}", task.flags_34),
            row_field(row, "flags"),
            "{what}"
        );
        assert_eq!(task.original_vtable, QUEUED_MODEL_VTABLE);
        task_rows += 1;
    }
    assert_eq!(task_rows, 26);
    assert_eq!(tasks.counters.sequence_b33a14, 26, "00B33A14");
    assert_eq!(
        tasks.counters.unpacked_sequence_b33a18, 0,
        "every model has an archive entry"
    );
    assert_eq!(tasks.counters.live_tasks_b33a20, 26);
    assert_eq!(io.array_2c[16], 26, "+0x2C[16]: byte 7 is 1, byte 2 is 5");
    assert_eq!(io.task_map.field_18, 26);
    assert_eq!(io.tasks.buckets[16].len(), 26);
    assert!(
        io.tasks.buckets[16].windows(2).all(|pair| {
            let key = |id: IoTaskId| io.tasks.store[id.0].as_ref().unwrap().key_10;
            key(pair[0]) < key(pair[1])
        }),
        "the chain is ascending"
    );
    assert_eq!(io.field_1c, 0, "the pause balanced");
    assert_eq!(io.field_20, 0, "the pause balanced");
    let row_at = |index: usize| -> Vec<&str> {
        records
            .iter()
            .filter(|line| line.starts_with("LR "))
            .nth(index)
            .unwrap()
            .split(' ')
            .collect()
    };
    // LR's raw +0x24 is nonzero for every one: each found its file record.
    for (index, task) in queued.iter().enumerate() {
        let raw = hex_bytes(row_field(&row_at(index), "raw"));
        assert_eq!(raw.len(), 0x38);
        assert_ne!(raw[0x24..0x28], [0; 4], "LR {index} +0x24");
        assert!(task.entry_24.is_some());
        assert_eq!(raw[0x20..0x24] != [0; 4], !task.path_20.is_empty());
    }
    // TES +0x94..+0x9C: held by the palette and by TES (RUN-001 rc 2), with 00B256D0's prefs in TX's raw.
    let tes = world.resource::<TesGlobals>().tes.as_ref().unwrap();
    let names: [&[u8]; 3] = [
        b"Data\\Textures\\Effects\\blooddecal.dds",
        b"Data\\Textures\\Effects\\lichBloodDecal.dds",
        b"Data\\Textures\\Effects\\willothewispBloodDecal.dds",
    ];
    let palette = world.resource::<BsTexturePalette>();
    for (index, tag) in ["94", "98", "9C"].iter().enumerate() {
        let row = book.take_one("DECAL", tag);
        let texture = tes.blood_decals_94[index];
        assert_eq!(
            texture.is_some(),
            row_field(&row, "present") == "1",
            "+0x{tag}"
        );
        let texture = texture.unwrap();
        let body = world.get::<SourceTexture>(texture).unwrap();
        assert_eq!(body.filename_34.as_deref(), Some(names[index]), "+0x{tag}");
        assert_eq!(
            palette.find(names[index], Some(&mut harness.archives.borrow_mut())),
            Some(texture),
            "the palette holds +0x{tag}"
        );
        assert_eq!(row_field(&row, "rc"), "2", "the palette and TES");
        let line = records
            .iter()
            .find(|line| {
                line.starts_with("TX ") && line.contains(&format!("after_preload tes+{tag}="))
            })
            .unwrap();
        let raw = hex_bytes(field(line, "raw"));
        let word = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
        assert_eq!(word(0x18), body.pixel_layout_18, "TX +0x18");
        assert_eq!(word(0x1C), body.alpha_format_1c, "TX +0x1C");
        assert_eq!(word(0x20), body.mip_mapped_20, "TX +0x20");
    }
    assert_eq!(decals.created, names, "three 00701E00 calls, in order");
    let row = book.take_one("FLAME", "after_preload");
    assert_eq!(flame(world), row[2], "00B35E50 after 00443550");
    assert_eq!(FLAME_NODE_FORM_IDS.len(), 21);
    let stores = world
        .resource::<TesGlobals>()
        .store_trace
        .iter()
        .filter(|store| (0x00B3_5E50..0x00B3_5EA4).contains(&store.address))
        .count();
    assert_eq!(stores, 21);

    // ---- 13b ----
    let before = player_parts(world, player);
    let mut logs: Vec<&'static str> = Vec::new();
    let start = Instant::now();
    let base = run_player_base_observed(world, &mut |text| logs.push(text), &mut |world, tag| {
        check_player(world, &book, tag, player);
        if tag == "after_1cc" {
            check_controller(world, &book, "after_1cc", player);
        }
    })
    .unwrap();
    println!("004067D7..00406861: {:.2?}", start.elapsed());
    assert_eq!(logs, ["Initializing Player..."]);
    assert_eq!(base, world.resource::<FormsById>().lookup(7));
    assert_eq!(
        raw_changes(&records, "after_base", "after_1cc"),
        [0x2C, 0x30],
        "only x and y moved"
    );
    assert_eq!(
        raw_changes(&records, "after_1cc", "after_89a0"),
        Vec::<usize>::new()
    );
    let mut expected = before.clone();
    expected.refr.position_2c = [2048.0, 2048.0, 0.0];
    assert_eq!(player_parts(world, player), expected, "nothing else moved");
    let controller = actor_character_controller(world, player).unwrap();
    let body = world.get::<BhkCharacterController>(controller).unwrap();
    assert_eq!(body.host_proxy_set_position_calls, 1, "00891560");
    assert_eq!(body.host_world_mark_calls, 1, "008AC0B0");
    assert_eq!(
        body.host_proxy_position,
        [2048.0 * HAVOK_SCALE, 2048.0 * HAVOK_SCALE, 0.0]
    );

    // ---- 13c ----
    let mut warnings = Vec::new();
    let before_init = player_parts(world, player);
    let start = Instant::now();
    let checks = {
        let mut services = DataLoadServices {
            working_directory: &harness.root,
            warning: &mut |text, _| warnings.push(text),
            host,
        };
        run_player_init(world, base, &mut services).unwrap()
    };
    println!("00406866..004068CE: {:.2?}", start.elapsed());
    assert!(warnings.is_empty(), "{warnings:?}");
    check_player(world, &book, "after_inititem", player);
    check_extras(world, &book, "after_inititem", player);
    assert_eq!(
        raw_changes(&records, "after_89a0", "after_inititem"),
        [0x08],
        "only the flags moved"
    );
    assert_eq!(
        player_parts(world, player),
        before_init,
        "RUN-001: InitItem moves no other dword"
    );
    let row = book.take_one("AV", "10");
    assert_eq!(format!("{:X}", checks.fatigue), row[2], "Fatigue");
    let row = book.take_one("AV", "8");
    assert_eq!(format!("{:X}", checks.health), row[2], "Health");
    assert!(host.logs.is_empty(), "no DebugLog_Printf: {:?}", host.logs);

    // ---- at 004068D1 ----
    let world_ref = &*world;
    let temporary = world_ref
        .resource::<InventoryGlobals>()
        .temporary_reference_b34428
        .expect("00B34428 exists before 00447D80");
    let check_temporary = |world: &World, tag: &str| {
        let row = book.take_one("TEMPREF", tag);
        let identity = world.get::<FormIdentity>(temporary).unwrap();
        let refr = world.get::<TesObjectRefr>(temporary).unwrap();
        assert_eq!(
            format!("{:X}", identity.flags_08),
            row_field(&row, "flags"),
            "00B34428 flags {tag}"
        );
        assert_eq!(
            format!("{:X}", identity.form_id_0c),
            row_field(&row, "id"),
            "00B34428 ID {tag}"
        );
        assert!(refr.base_form_1c.is_none());
        assert_eq!(row_field(&row, "base"), "0", "{tag}");
        assert!(refr.parent_cell_40.is_none());
        assert_eq!(row_field(&row, "cell"), "0", "{tag}");
        assert_eq!(
            triple(refr.position_2c),
            row_field(&row, "pos"),
            "00B34428 position {tag}"
        );
    };
    check_temporary(world_ref, "before");
    // RUN-002's D 2, the temporary reference's first 0x58 bytes, and +0x8C0 past the temporary references, at 004068D1.
    let run_2 = evidence_lines("phase13-run002-2026-10-09");
    let dump = run_2.iter().find(|line| line.starts_with("D 2 ")).unwrap();
    let raw = hex_bytes(field(dump, "raw"));
    assert_eq!(raw[0..4], 0x00A4_6C44_u32.to_le_bytes(), "vtable");
    assert_eq!(
        world_ref.get::<OriginalVtable>(temporary).unwrap().0,
        0x00A4_6C44
    );
    assert_eq!(
        raw[4],
        world_ref
            .get::<FormIdentity>(temporary)
            .unwrap()
            .form_type_04,
        "+0x04"
    );
    let body = world_ref.get::<TesObjectRefr>(temporary).unwrap();
    assert_eq!(
        raw[0x18..0x1C],
        body.child_cell_vtable_18.to_le_bytes(),
        "+0x18"
    );
    assert_eq!(raw[0x38..0x3C], body.scale_38.to_le_bytes(), "+0x38");
    assert_eq!(
        format!(
            "{:X}",
            world_ref.resource::<DataHandler>().next_form_id_8c0()
        ),
        field(dump, "dh8c0"),
        "+0x8C0 at 004068D1"
    );
    let row = book.take_one_kind("DELAYS_BEFORE");
    let delays = world_ref.resource::<QuestDelayGlobals>();
    assert_eq!(
        format!("{:X}", delays.counter_b361f4),
        row_field(&row, "counter"),
        "00B361F4 before 00447D80"
    );
    // SN 3019: the eight delays and the counter, as raw dwords, and 3,010 scripts' state before the pass.
    let line = records
        .iter()
        .find(|line| line.starts_with("SN ") && line.contains(" before "))
        .unwrap();
    assert_eq!(field(line, "count"), "3010");
    let raw = hex_bytes(field(line, "delays"));
    for (at, delay) in delays.delays_b361d4.iter().enumerate() {
        assert_eq!(
            raw[at * 4..at * 4 + 4],
            delay.to_bits().to_le_bytes(),
            "delay {at} before"
        );
    }
    let scripts_before = records
        .iter()
        .filter(|line| line.starts_with("SC ") && line.split(' ').nth(3) == Some("before"));
    let mut before_rows = 0;
    for line in scripts_before {
        let raw = hex_bytes(field(line, "raw"));
        let id = u32::from_le_bytes(raw[0x0C..0x10].try_into().unwrap());
        let script = world_ref
            .resource::<FormsById>()
            .lookup(id)
            .unwrap_or_else(|| panic!("SCPT {id:X}"));
        let identity = world_ref.get::<FormIdentity>(script).unwrap();
        // The byte at +0x08 only: the run's raw flags carry the file's bits above it, and the upper dword isn't the row's.
        assert_eq!(
            raw[0x08..0x0C],
            identity.flags_08.to_le_bytes(),
            "SCPT {id:X} flags before"
        );
        let body = world_ref.get::<Script>(script).unwrap();
        assert_eq!(
            raw[0x38..0x3C],
            body.floats_34[1].to_le_bytes(),
            "SCPT {id:X} +0x38 before"
        );
        before_rows += 1;
    }
    assert_eq!(before_rows, 3010);
    // The containers with no changes block: 00485E00 makes theirs in this pass, in this order. Which ones they are depends on 0044F3D0's leveled rolls, so RUN-002 has 174 and RUN-003 177, against the host's seed.
    let lacking = containers_lacking_changes(world_ref);

    // ---- 13d ----
    let inventory = world_ref.resource::<InventoryGlobals>();
    let (created, passes, item_runs, script_runs) = (
        inventory.container_changes_created,
        inventory.container_script_passes,
        inventory.container_item_script_runs,
        inventory.reference_script_runs,
    );
    let mut osg = OsGlobalsData::default();
    let mut platform = ScriptedOsPlatform::default();
    let mut sound_settings = pinned_collections();
    osg.sound_24 = Some(OsSoundGlobals::new(
        HostWindow(1),
        &mut sound_settings,
        &mut platform,
    ));
    let mut time = TimeInfoState::default();
    let mut logs: Vec<&'static str> = Vec::new();
    let mut warnings = Vec::new();
    let start = Instant::now();
    let stored = {
        let mut services = DataLoadServices {
            working_directory: &harness.root,
            warning: &mut |text, _| warnings.push(text),
            host,
        };
        run_scripts_sound_and_clamp(
            world,
            ScriptsSoundInputs {
                services: &mut services,
                ini: &collections.ini,
                os_globals: Some(&osg),
                time: &mut time,
                log: &mut |text| logs.push(text),
            },
        )
        .unwrap()
    };
    println!("004068D1..00406944: {:.2?}", start.elapsed());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        logs,
        ["Initializing Scripts...", "Initializing Sound System..."]
    );
    // Every SCRIPT row: +0x08, +0x38, the +0x28 byte and each reference's +0x08 being nonzero.
    let mut compared = 0;
    let world_ref = &*world;
    for row in book.take_kind("SCRIPT") {
        let id = u32::from_str_radix(row[1], 16).unwrap();
        let what = format!("SCPT {id:X}");
        let script = world_ref
            .resource::<FormsById>()
            .lookup(id)
            .unwrap_or_else(|| panic!("{what}"));
        let identity = world_ref.get::<FormIdentity>(script).unwrap();
        let body = world_ref.get::<Script>(script).unwrap();
        assert_eq!(
            format!("{:X}", identity.flags_08),
            row_field(&row, "flags"),
            "{what}: flags"
        );
        assert_eq!(
            format!("{:08X}", body.floats_34[1].to_bits()),
            row_field(&row, "f38"),
            "{what}: +0x38"
        );
        assert_eq!(
            format!("{:X}", body.header_18[0x10]),
            row_field(&row, "type"),
            "{what}: type"
        );
        let pattern: String = body
            .references_40
            .iter()
            .map(|reference| if reference.form_08 != 0 { '1' } else { '0' })
            .collect();
        let pattern = if pattern.is_empty() {
            "-".to_owned()
        } else {
            pattern
        };
        assert_eq!(pattern, row_field(&row, "refs"), "{what}: references");
        compared += 1;
    }
    assert_eq!(compared, 3010);
    let delays = world_ref.resource::<QuestDelayGlobals>();
    let row = book.take_one_kind("DELAYS");
    let table: Vec<String> = delays
        .delays_b361d4
        .iter()
        .map(|delay| format!("{:08X}", delay.to_bits()))
        .collect();
    assert_eq!(table.join(","), row[1], "00B361D4");
    assert_eq!(
        format!("{:X}", delays.counter_b361f4),
        row_field(&row, "counter"),
        "00B361F4"
    );
    // PASSES: RUN-002's counts. `create` and `item_runs` follow the leveled rolls (the seed is a harness value); `container` and `script_runs` don't.
    let row = book.take_one_kind("PASSES");
    for key in ["create", "item_runs"] {
        fixtures::ledger_relaxed(
            "phase13",
            &format!("PASSES #{key}"),
            1,
            "roll-dependent: 0044F3D0's leveled rolls, and the seed is a harness value; RUN-002 and RUN-003 differ too (docs/open-items.md, 13d)",
        );
    }
    let inventory = world_ref.resource::<InventoryGlobals>();
    assert_eq!(
        (inventory.container_script_passes - passes).to_string(),
        row_field(&row, "container"),
        "0048E740 calls"
    );
    assert_eq!(
        (inventory.reference_script_runs - script_runs).to_string(),
        row_field(&row, "script_runs"),
        "004D720A calls"
    );
    let host_created = inventory.container_changes_created - created;
    let host_item_runs = inventory.container_item_script_runs - item_runs;
    println!(
        "13d: {} containers, {} object runs; rolls: {} created (RUN-002 {}, RUN-003 177), {} item runs (RUN-002 {})",
        inventory.container_script_passes - passes,
        inventory.reference_script_runs - script_runs,
        host_created,
        row_field(&row, "create"),
        host_item_runs,
        row_field(&row, "item_runs"),
    );
    assert_eq!(
        host_created as usize,
        lacking.len(),
        "00485E00 creates only what's lacking"
    );
    assert_eq!(
        host_item_runs,
        scripted_stacks(world_ref),
        "0048E740 runs every scripted stack"
    );
    // RUN-002 and RUN-003's CR records: the IDs both the run and the host create, in the same order.
    check_creation_order(world_ref, &lacking);
    check_temporary(world_ref, "after");
    let dump = run_2.iter().find(|line| line.starts_with("D 15 ")).unwrap();
    assert_eq!(
        format!(
            "{:X}",
            world_ref.resource::<DataHandler>().next_form_id_8c0()
        ),
        field(dump, "dh8c0"),
        "+0x8C0 at 004068E9"
    );
    println!(
        "00517810, counted and not run: {}",
        world_ref
            .resource::<InventoryGlobals>()
            .script_runs_not_executed
    );
    assert!(
        world_ref
            .get_resource::<CellRegionLocks>()
            .unwrap()
            .entries
            .is_empty(),
        "every lock released"
    );
    // SOUND.
    let row = book.take_one_kind("SOUND");
    let source = world_ref.resource::<SoundSourceGlobal>();
    assert_eq!(
        source.0 == Some(SoundSource::OsGlobals),
        row_field(&row, "source_is_osg") == "1",
        "00B3C0F0"
    );
    let ini = collections.ini.find_address(0x00B0_2D10).unwrap().raw;
    assert_eq!(format!("{ini:X}"), row_field(&row, "fps_ini"), "00B02D10");
    assert!(!stored, "iFPSClamp 0 takes the `ja`");
    assert_eq!(
        format!("{:08X}", time.fps_clamp_04.to_bits()),
        row_field(&row, "fps_clamp"),
        "00B33E94"
    );
    // At 00406944.
    check_player(world_ref, &book, "final", player);
    check_controller(world_ref, &book, "final", player);
    check_extras(world_ref, &book, "final", player);
    assert_eq!(
        raw_changes(&records, "after_inititem", "final"),
        Vec::<usize>::new()
    );
    check_model_drain(
        world,
        harness,
        &archive_settings_from(&collections.ini),
        &book,
        &records,
    );
    book.assert_accounted(&[]);
    let world_ref: &World = world;
    // The cell walks. P12-RUN-001 reads +0xCC as 1,916 at 004067CC, and 00447CA5 reads it once. P13-RUN-001 counted 1,914 breakpoint hits at 00447CC7, which a null slot also passes, so that count can't be short by a skip: its pump drops a hit that repeats the last token (ea, ESP, EAX, ECX) at a budget boundary, and RUN-002's exact counts above don't depend on it. Nothing else separates them, so it stays a reading of the harness, unverified.
    let data = world_ref.resource::<DataHandler>();
    let walked = data
        .cells_c0
        .cells
        .iter()
        .take(data.cells_c0.size_0c as usize)
        .flatten()
        .count();
    assert_eq!(data.cells_c0.size_0c, 1916, "P12-RUN-001 S 713");
    assert_eq!(walked, 1916, "no null slot");
    let line = records
        .iter()
        .find(|line| line.starts_with("M 8386 "))
        .unwrap();
    assert!(line.contains("('cell', 4488431, 0):85"), "00447CEF");
    // 00447CEF is `mov esi, [esi+4]`, which every list node passes with or without a persistent cell.
    let worlds = world_ref
        .resource::<DataHandlerLists>()
        .world_spaces_0c
        .items()
        .count();
    assert_eq!(worlds, 85);
}

// IDA: N/A 00447CA0's walk order over the +0xC4 cells, then each world's persistent cell.
fn walked_cells(world: &World) -> Vec<Entity> {
    let data = world.resource::<DataHandler>();
    let mut cells: Vec<Entity> = data
        .cells_c0
        .cells
        .iter()
        .take(data.cells_c0.size_0c as usize)
        .filter_map(|cell| *cell)
        .collect();
    cells.extend(
        world
            .resource::<DataHandlerLists>()
            .world_spaces_0c
            .items()
            .filter_map(|form| world.get::<TesWorldSpace>(form).unwrap().persistent_cell_34),
    );
    cells
}

// IDA: N/A the non-actor, undeleted references of a container base, in walk order, that have no 0x1A entry.
fn containers_lacking_changes(world: &World) -> Vec<u32> {
    let mut lacking = Vec::new();
    for cell in walked_cells(world) {
        for reference in world.get::<TesObjectCell>(cell).unwrap().objects_48.items() {
            let identity = world.get::<FormIdentity>(reference).unwrap();
            if identity.flags_08 & 0x20 != 0 || world.get::<Actor>(reference).is_some() {
                continue;
            }
            let refr = world.get::<TesObjectRefr>(reference).unwrap();
            let base = refr.base_form_1c.unwrap();
            if world.get::<FormIdentity>(base).unwrap().form_type_04 == 0x17
                && refr.extra_data_44.get(EXTRA_CONTAINER_CHANGES).is_none()
            {
                lacking.push(identity.form_id_0c);
            }
        }
    }
    lacking
}

// IDA: N/A what 0048E740 should have run: one per initialized SCPT stack of each container with a parent cell.
fn scripted_stacks(world: &World) -> u32 {
    let mut stacks = 0;
    for cell in walked_cells(world) {
        for reference in world.get::<TesObjectCell>(cell).unwrap().objects_48.items() {
            let identity = world.get::<FormIdentity>(reference).unwrap();
            if identity.flags_08 & 0x20 != 0 || world.get::<Actor>(reference).is_some() {
                continue;
            }
            let refr = world.get::<TesObjectRefr>(reference).unwrap();
            if world
                .get::<FormIdentity>(refr.base_form_1c.unwrap())
                .unwrap()
                .form_type_04
                != 0x17
                || refr.parent_cell_40.is_none()
            {
                continue;
            }
            let Some(entry) = refr.extra_data_44.get(EXTRA_CONTAINER_CHANGES) else {
                continue;
            };
            let ExtraData::ContainerChanges(changes) = &entry.data else {
                panic!("a 0x1A entry with no block");
            };
            for item in &changes.entries_00 {
                for stack in item.lists_00.iter().flatten() {
                    if let Some(ExtraData::Script { script_0c, .. }) =
                        stack.get(0x12).map(|extra| &extra.data)
                        && world.get::<FormIdentity>(*script_0c).unwrap().flags_08 & 8 != 0
                    {
                        stacks += 1;
                    }
                }
            }
        }
    }
    stacks
}

// IDA: N/A P13-RUN-003's CR records, with RUN-002's as a prefix check: an ID the host also creates comes in the same relative order, and the run's first twelve don't depend on the rolls.
fn check_creation_order(world: &World, lacking: &[u32]) {
    let ids = |run: &str| -> Vec<u32> {
        evidence_lines(run)
            .iter()
            .filter(|line| line.starts_with("CR "))
            .map(|line| u32::from_str_radix(field(line, "id"), 16).unwrap())
            .collect()
    };
    let run_3 = ids("phase13-run003-2026-10-09");
    let run_2 = ids("phase13-run002-2026-10-09");
    assert_eq!(run_3.len(), 177, "RUN-003 records every creation");
    assert_eq!(run_2[..2], run_3[..2]);
    assert_eq!(run_3[..12], lacking[..12], "the first twelve containers");
    let in_host: Vec<u32> = run_3
        .iter()
        .copied()
        .filter(|id| lacking.contains(id))
        .collect();
    let in_run: Vec<u32> = lacking
        .iter()
        .copied()
        .filter(|id| run_3.contains(id))
        .collect();
    assert_eq!(in_host, in_run, "shared creations come in the same order");
    println!(
        "creations shared with RUN-003: {} of {} and {}",
        in_host.len(),
        run_3.len(),
        lacking.len()
    );
    // Each CR's cell, for a reference the host has too.
    for line in evidence_lines("phase13-run003-2026-10-09")
        .iter()
        .filter(|line| line.starts_with("CR "))
    {
        let id = u32::from_str_radix(field(line, "id"), 16).unwrap();
        if let Some(reference) = world.resource::<FormsById>().lookup(id) {
            let cell = world
                .get::<TesObjectRefr>(reference)
                .unwrap()
                .parent_cell_40;
            let cell = cell.map_or(0, |cell| {
                world.get::<FormIdentity>(cell).unwrap().form_id_0c
            });
            assert_eq!(format!("{cell:X}"), field(line, "cell"), "CR {id:X}");
        }
    }
}
