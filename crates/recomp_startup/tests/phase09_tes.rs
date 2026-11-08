//! Phase 09: the TES block 00406604..00406687 of 00406360 on the shipped data, read through the ported archives, stream and file code, against the P09-RUN-001 dump: the TES constructor's Havok
//! setup, grid arrays, task objects with their two worker threads, TESSaveLoadGame, cell buffers and scene hookup; the DataHandler, its skills and the plugin list; the water trees; the TreeManager
//! and the final world update. Then the failure and configuration arms, which only the C++ port has exercised. Host tests earn no validation bonus.
//!
//! `phase09.rows` is the imported `.inc`, and the manifest pins it, so the values the C++ test held by hand are in `phase09_run001.rows`. The `.rows` accounting in `every_row_is_read` fails if
//! a line goes unread. The pinned config INI is the run's (d243541f), so the TES runs read it where the C++ test wrote the same keys by hand.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy_ecs::prelude::*;
use recomp_core::fixtures;
use recomp_core::reader::StreamError;
use recomp_game::bs::archive::{ArchiveManager, ArchiveSettings};
use recomp_game::bs::file::{GameFile, OpenedFile};
use recomp_game::bs::file_finder::{
    BsFileCallbacks, FileFinder, bs_file_exists, bs_file_open_core,
};
use recomp_game::bs::havok::{
    HavokGlobals, HavokSettingInputs, SEEN_DATA_UPDATE_RADIUS_B35C14, havok_sim_type_rate,
    initialize_havok_system,
};
use recomp_game::bs::nif_objects::BsxFlagsExtraData;
use recomp_game::bs::scene_graphs::SceneGraphGlobals;
use recomp_game::bs::shader::{BsShaderGlobals, BsShaderSlot};
use recomp_game::bs::shader_classes_geometry::{SkyShader, WaterShaderObject};
use recomp_game::bs::sky_shader::{
    NiGeometryShader, PrepareGeometryContext, SKY_SHADER_SLOT_INDEX, SKY_SHADER_VTABLE,
    WATER_SHADER_SLOT_INDEX, WATER_SHADER_VTABLE,
};
use recomp_game::bs::stream::{BsStreamLoad, BsStreamServices, load_bs_stream_from_path};
use recomp_game::bs::task_manager::{
    BsTaskThread, LockFreeMap, LockFreeQueue, TaskCall, TaskGlobals,
};
use recomp_game::bs::tree_manager::{BSX_FLAGS_VTABLE, TreeManagerGlobals, reset_tree_manager};
use recomp_game::gamebryo::av_object::NiAvProperties;
use recomp_game::gamebryo::mesh_geometry::{MeshTriShapeData, MeshTriShapePayload, NiGeometry};
use recomp_game::gamebryo::object_net::{AlphaProperty, NiExtraDataName, NiObjectNet};
use recomp_game::gamebryo::properties::{
    MaterialIdCounter, MaterialProperty, StencilProperty, VertexColorProperty, WaterShaderProperty,
    ZBufferProperty,
};
use recomp_game::gamebryo::registration::{
    FactoryCreate, NiFactoryRegistry, NiProcessDefaults, StartupCallbackScheduler,
    register_known_crt_callbacks,
};
use recomp_game::gamebryo::scene_derived::spawn_directional_light;
use recomp_game::gamebryo::scene_node::{
    NiAvWorld, NiNodeChildren, NiNodeEffects, SceneNodePayload, spawn_scene_node,
};
use recomp_game::gamebryo::scene_state::ScenePropertyState;
use recomp_game::gamebryo::scene_update::SceneControllers;
use recomp_game::gamebryo::search_path::bs_search_path_candidates;
use recomp_game::gamebryo::stream_objects::NiClass;
use recomp_game::gamebryo::stream_postload::StreamPostLoadRuntime;
use recomp_game::gamebryo::texture::{
    FlipController, SourceTexture, SourceTextureLoadServices, TextureController, TexturingProperty,
};
use recomp_game::gamebryo::transform::NiTransform;
use recomp_game::tes::data_handler::{
    DataHandler, DataHandlerGlobals, FORMS_BY_ID_BUCKETS, FORMS_BY_ID_VTABLE, FormsById, TesSkill,
    build_data_handler_file_list, construct_data_handler,
};
use recomp_game::tes::form::{FormIdentity, ModReferences, OriginalVtable};
use recomp_game::tes::game_settings::GameSettingCollection;
use recomp_game::tes::settings::{SettingCollections, register_static_settings};
use recomp_game::tes::sky::{
    Sky, SkyAtmosphere, SkyGlobals, SkyInitServices, SkySun, construct_sky,
};
use recomp_game::tes::tes::{
    BsTempNodeManager, GameThreadLocals, NiTLargeArrayU32, NiTPointerMapRecord, Tes,
    TesConstructionInputs, TesGlobals, tes_setting_inputs,
};
use recomp_game::tes::tes_file::{
    BsStringT, TesFileData, TesFileId, TesFileServices, build_tes_file_list,
};
use recomp_game::tes::water::{
    TesWaterNodeRecord, WaterGlobals, WaterServices, WaterTextureServices,
};
use recomp_startup::initialize_game_data::{
    SkyPreludeInputs, SkyPreludeResult, TesBlockInputs, create_tes, run_sky_prelude, run_tes_block,
};
use recomp_startup::initialize_scene_graphs::{SceneGraphInputs, initialize_scene_graphs};

fn hex8(value: u32) -> String {
    format!("{value:08X}")
}

fn hex4(value: u16) -> String {
    format!("{value:04X}")
}

fn bits(values: &[f32]) -> String {
    values
        .iter()
        .map(|value| hex8(value.to_bits()))
        .collect::<Vec<_>>()
        .join(",")
}

fn set_or_null<T>(value: Option<T>) -> &'static str {
    if value.is_some() { "set" } else { "null" }
}

fn transform_bits(transform: &NiTransform) -> String {
    format!(
        "{},{},{}",
        bits(&transform.rotation),
        bits(&transform.translation),
        bits(&[transform.scale])
    )
}

fn name_of(world: &World, object: Entity) -> String {
    world
        .get::<NiObjectNet>(object)
        .and_then(|net| net.name.as_deref())
        .map_or("<null>".to_owned(), |name| {
            String::from_utf8_lossy(name).into_owned()
        })
}

fn hex_upper(bytes: impl IntoIterator<Item = u8>) -> String {
    bytes
        .into_iter()
        .map(|byte| format!("{byte:02X}"))
        .collect()
}

fn le_f32<const N: usize>(values: &[[f32; N]]) -> String {
    hex_upper(
        values
            .iter()
            .flatten()
            .flat_map(|value| value.to_bits().to_le_bytes()),
    )
}

/// IDA: N/A `phase09.rows`, recording which lines a check has read.
struct Rows {
    lines: Vec<String>,
    read: RefCell<Vec<bool>>,
}

impl Drop for Rows {
    fn drop(&mut self) {
        fixtures::ledger_read("phase09", &self.read.borrow());
    }
}

impl Rows {
    fn load() -> Self {
        let lines = fixtures::rows("phase09");
        let read = RefCell::new(vec![false; lines.len()]);
        Self { lines, read }
    }

    fn expected(&self, prefix: &str) -> &str {
        let index = self
            .lines
            .iter()
            .position(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("missing expected line {prefix}"));
        self.read.borrow_mut()[index] = true;
        &self.lines[index]
    }

    fn compare(&self, host: &str, prefix: &str) {
        assert_eq!(host, self.expected(prefix), "mismatch for {prefix}");
    }

    fn expected_with(&self, prefixes: &[&str]) -> Vec<&str> {
        let mut out = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            for prefix in prefixes {
                if line.starts_with(prefix) {
                    self.read.borrow_mut()[index] = true;
                    out.push(line.as_str());
                }
            }
        }
        out
    }

    fn compare_lines(&self, host: &[String], prefixes: &[&str], what: &str) {
        let run = self.expected_with(prefixes);
        for index in 0..host.len().max(run.len()) {
            let h = host.get(index).map_or("<missing>", String::as_str);
            let r = run.get(index).copied().unwrap_or("<missing>");
            assert_eq!(h, r, "mismatch in {what} line {index}");
        }
    }

    fn unread(&self) -> Vec<&str> {
        self.lines
            .iter()
            .zip(self.read.borrow().iter())
            .filter(|(_, read)| !**read)
            .map(|(line, _)| line.as_str())
            .collect()
    }
}

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

/// IDA: N/A the static settings with no INI read, then a few values set: the C++ arms' fixture INI.
fn static_collections(values: &[(u32, u32)]) -> SettingCollections {
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    for &(address, value) in values {
        collections.ini.find_address_mut(address).unwrap().raw = value;
    }
    collections
}

/// IDA: N/A a stand-in for the Phase 08 outputs: the scene root, land LOD and Sky.
struct StubScene {
    root: Entity,
    land_lod: Entity,
    sun_light: Entity,
    fog: Entity,
}

fn stub_scene(world: &mut World) -> StubScene {
    let root = spawn_scene_node(world, SceneNodePayload::default());
    world.get_mut::<NiObjectNet>(root).unwrap().name = Some(b"ObjectLODRoot".to_vec());
    let land_lod = spawn_scene_node(world, SceneNodePayload::default());
    let sun_light = spawn_directional_light(world);
    let fog = world.spawn(NiObjectNet::default()).id();
    let mut sky: Sky = construct_sky(world.resource::<GameSettingCollection>()).unwrap();
    sky.atmosphere_20 = Some(SkyAtmosphere {
        original_vtable: 0,
        root_04: None,
        mesh_08: None,
        fog_0c: Some(fog),
        quad_node_10: None,
        quad_14: None,
        byte_18: 0,
    });
    sky.sun_28 = Some(SkySun {
        light_1c: Some(sun_light),
        ..SkySun::default()
    });
    world.init_resource::<SkyGlobals>();
    world.resource_mut::<SkyGlobals>().singleton_b365c4 = Some(Box::new(sky));
    StubScene {
        root,
        land_lod,
        sun_light,
        fog,
    }
}

/// IDA: N/A one TES constructor run over a stand-in scene (no water).
struct TesRun {
    world: World,
    collections: SettingCollections,
    registry: NiFactoryRegistry,
    scene: StubScene,
    allocated: bool,
    data_handler_calls: u32,
    object_load_gate_calls: u32,
    file_list_path: Option<String>,
    warnings: Vec<String>,
}

fn current_directory_input() -> Vec<u8> {
    b"K:\\OblivionBot".to_vec()
}

impl TesRun {
    /// `working_directory` set: the real DataHandler and plugin list over working_directory/Data (read only); `None`: counting stubs.
    fn new(
        mut collections: SettingCollections,
        fail_allocation: bool,
        working_directory: Option<&Path>,
    ) -> Self {
        let mut world = World::new();
        world.insert_resource(GameSettingCollection::default());
        let scene = stub_scene(&mut world);
        let mut registry = NiFactoryRegistry::default();
        let mut globals = TesGlobals::default();
        let mut data_handler_calls = 0;
        let mut object_load_gate_calls = 0;
        let mut file_list_path = None;
        let mut warnings = Vec::new();
        let (havok_settings, debris_max_velocity, debris_min_extent, step_speed) =
            tes_setting_inputs(
                world.resource::<GameSettingCollection>(),
                &collections.ini,
                current_directory_input(),
            )
            .unwrap();
        let mut construct = |world: &mut World| {
            data_handler_calls += 1;
            if working_directory.is_some() {
                construct_data_handler(world)?;
            }
            Ok(())
        };
        let mut build = |world: &mut World, path: &str| {
            file_list_path = Some(path.to_owned());
            if let Some(working) = working_directory {
                build_data_handler_file_list(world, path, working, &mut |text| {
                    warnings.push(text);
                })?;
            }
            Ok(())
        };
        let mut gate = |_: &mut World| object_load_gate_calls += 1;
        let allocated = create_tes(
            &mut world,
            &mut globals,
            &mut collections.ini,
            &mut TesConstructionInputs {
                registry: &mut registry,
                havok_settings,
                data_path: "Data\\",
                scene_root: Some(scene.root),
                land_lod: Some(scene.land_lod),
                debris_max_velocity,
                debris_min_extent,
                step_speed,
                construct_data_handler: &mut construct,
                build_file_list: &mut build,
                enable_object_load_callbacks: &mut gate,
                fail_tes_allocation: fail_allocation,
            },
        )
        .unwrap();
        world.insert_resource(globals);
        Self {
            world,
            collections,
            registry,
            scene,
            allocated,
            data_handler_calls,
            object_load_gate_calls,
            file_list_path,
            warnings,
        }
    }

    fn globals(&self) -> &TesGlobals {
        self.world.resource::<TesGlobals>()
    }

    fn tes(&self) -> &Tes {
        self.globals().tes.as_deref().unwrap()
    }

    fn setting_raw(&self, address: u32) -> u32 {
        self.collections.ini.find_address(address).unwrap().raw
    }
}

// --- 09-C: the TES constructor body ---------------------------------------------------------

fn check_tes_globals(run: &TesRun, rows: &Rows) {
    let g = run.globals();
    rows.compare(
        &format!(
            "G b3f580={:X} b33a2c={:X} b06a28={:X} b34418={:X} b3441c={:X} b34410={} b34414={} b2c334={} b2c338={}",
            g.havok_flag_b3f580,
            g.cell_cache_limit_b33a2c,
            g.grid_changed_b06a28,
            g.grid_distant_count_b34418,
            g.grid_distant_tree_range_b3441c,
            bits(&[g.grid_distant_inner_b34410]),
            bits(&[g.grid_distant_extent_b34414]),
            bits(&[g.lod_distance_b2c334]),
            bits(&[g.lod_fade_b2c338]),
        ),
        "G ",
    );
    rows.compare(
        &format!(
            "I grids={:X} exterior={:X} interior={:X} (start 5/24/3)",
            run.setting_raw(0x00B0_6A2C),
            run.setting_raw(0x00B0_51DC),
            run.setting_raw(0x00B0_51D4),
        ),
        "I ",
    );
    let c = &g.havok_copies;
    let havok = run.world.resource::<HavokGlobals>();
    rows.compare(
        &format!(
            "C copies ba7920={} ba791c={} ba7918={} b2e2f0={} b2e764={} b2e300={} ba7909={:02X} b2e2f4={} b2e2f8={} ba7a88={} b2e784={} ba7a84={} ba7a50={}",
            bits(&[c.debris_max_velocity_ba7920]),
            bits(&[c.debris_min_extent_ba791c]),
            hex8(c.update_type_ba7918),
            bits(&[c.chase_delta_mult_b2e2f0]),
            bits(&[c.move_limit_mass_b2e764]),
            hex8(c.callback_b2e300),
            c.add_biped_ba7909,
            hex8(c.max_picks_b2e2f4),
            hex8(c.batch_remove_b2e2f8),
            hex8(c.callback_ba7a88),
            bits(&[c.step_speed_squared_b2e784]),
            hex8(c.callback_ba7a84),
            hex8(havok.step_callback_ba7a50),
        ),
        "C copies",
    );
    // IDA: N/A the TES constructor's global store order.
    let order: Vec<u32> = g.store_trace.iter().map(|store| store.address).collect();
    assert_eq!(
        order[..13],
        [
            0x00B3_F580,
            0x00BA_7920,
            0x00BA_791C,
            0x00BA_7918,
            0x00B2_E2F0,
            0x00B2_E764,
            0x00B2_E300,
            0x00BA_7909,
            0x00B2_E2F4,
            0x00B2_E2F8,
            0x00BA_7A88,
            0x00B2_E784,
            0x00BA_7A84
        ],
        "TES store order"
    );
}

fn check_havok(run: &mut TesRun, rows: &Rows) {
    let h = run.world.resource::<HavokGlobals>();
    rows.compare(
        &format!(
            "H base={:X} b2eb04={} b2eb08={} b2e2dc={:X} floats={}",
            u8::from(h.base_system_initialized),
            hex8(h.error_report_callback_b2eb04),
            hex8(h.memory_callback_b2eb08),
            h.sim_type_rate_b2e2dc,
            bits(&[
                h.maximum_wind_b2e8a8,
                h.jump_move_base_ba7a60,
                h.jump_move_mult_b2e76c,
                h.jump_skill_factor_b2e774,
                h.jump_double_mult_b2e770,
                h.jump_fall_velocity_b2e778
            ]),
        ),
        "H base=",
    );
    rows.compare(
        &format!(
            "H tables={}",
            hex_upper(h.layer_tables.iter().flat_map(|word| word.to_le_bytes()))
        ),
        "H tables=",
    );
    let f = h.collision_filter.as_ref().expect("00BA7904 set");
    rows.compare(
        &format!(
            "H filter vt={} size={:X} ref={:X} if={},{},{},{}",
            hex8(f.original_vtable),
            f.memory_size_04,
            f.reference_count_06,
            hex8(f.interface_vtables_08[0]),
            hex8(f.interface_vtables_08[1]),
            hex8(f.interface_vtables_08[2]),
            hex8(f.interface_vtables_08[3]),
        ),
        "H filter",
    );
    let s = h.streambuf.as_ref().expect("00BA7FB4 set");
    rows.compare(
        &format!(
            "H streambuf vt={} size={:X} ref={:X}",
            hex8(s.original_vtable),
            s.memory_size_04,
            s.reference_count_06
        ),
        "H streambuf",
    );
    let e = h.error.as_ref().expect("00BA7FB0 set");
    rows.compare(
        &format!(
            "H error vt={} size={:X} ref={:X} reports={:X} cap={} count={:X} mask={:X} dir0={:X}",
            hex8(e.original_vtable),
            e.memory_size_04,
            e.reference_count_06,
            e.reports_08.len(),
            hex8(e.report_capacity_10),
            e.disabled_ids_14.count(),
            e.disabled_ids_14.mask(),
            e.directory_20.first().copied().unwrap_or(0),
        ),
        "H error vt",
    );
    let lines: Vec<String> = e
        .reports_08
        .iter()
        .map(|report| {
            assert_eq!(report.file, b"hkVisualDebugger.cpp", "report file name");
            format!("{:X}", report.line)
        })
        .collect();
    rows.compare(
        &format!("H error.lines={}", lines.join(",")),
        "H error.lines",
    );
    let keys = e.disabled_ids_14.keys();
    let values = e.disabled_ids_14.values();
    let key_text: Vec<String> = keys.iter().map(|&key| hex8(key)).collect();
    let value_text: Vec<String> = keys
        .iter()
        .zip(values)
        .map(|(&key, &value)| format!("{:X}", if key != 0 { value } else { 0 }))
        .collect();
    rows.compare(
        &format!("H error.keys={}", key_text.join(",")),
        "H error.keys",
    );
    rows.compare(
        &format!("H error.values={}", value_text.join(",")),
        "H error.values",
    );
    rows.compare(
        &format!(
            "H world={} b35c00={:X} b2e2d4={:X} b2e2e8={} b35c14={:X}",
            set_or_null(h.world_b35c24.as_ref()),
            h.b35c00,
            h.num_havok_threads_b2e2d4,
            bits(&[h.max_time_b2e2e8]),
            run.world
                .resource::<GameSettingCollection>()
                .raw_at(SEEN_DATA_UPDATE_RADIUS_B35C14)
                .unwrap(),
        ),
        "H world",
    );
    // The 43 bhk registrations are the last catalog entries, in order.
    let entries = run.registry.entries();
    assert!(entries.len() >= 43, "bhk registrations present");
    let bhk: Vec<String> = entries[entries.len() - 43..]
        .iter()
        .map(|(name, constructor)| format!("{name}:{constructor:X}"))
        .collect();
    rows.compare(&format!("R bhk={}", bhk.join(",")), "R bhk=");
    // Build 1 (2026-11-07) loads the prison's bhk classes as data (`bs::havok_stream`), so bhkRigidBody constructs.
    assert!(
        matches!(
            run.registry.create(&mut run.world, "bhkRigidBody"),
            FactoryCreate::Created { .. }
        ),
        "bhkRigidBody constructs as data since Build 1"
    );
}

fn check_tes_object(run: &TesRun, rows: &Rows) {
    assert!(
        run.allocated && run.globals().tes.is_some(),
        "00B333A0 published"
    );
    let t = run.tes();
    let mut lists: Vec<String> = Vec::new();
    lists.push(format!("{:X}", u8::from(t.exterior_cell_list_78.is_some())));
    lists.push(format!(
        "{:X}",
        u8::from(!t.exterior_cell_list_next_7c.is_empty())
    ));
    lists.extend(t.furniture_references_80.iter().map(|v| format!("{v:X}")));
    lists.push(format!("{:X}", t.queued_script_references_88));
    lists.extend(
        t.actor_base_death_counts_8c
            .iter()
            .map(|v| format!("{v:X}")),
    );
    lists.extend(
        t.blood_decals_94
            .iter()
            .map(|v| format!("{:X}", u8::from(v.is_some()))),
    );
    lists.push(format!("{:X}", t.particle_node_cache_first_a0));
    lists.push(format!("{:X}", t.particle_node_cache_next_a4));
    rows.compare(
        &format!(
            "T tes vt={} coords={},{},{},{} cur={:X},{:X} f40={:X} f44={:X} last={},{} bytes50={:X},{:X},{:X},{:X} water={:X},{:X} f60={:X} f64={:X} bytes68={:X},{:X},{:X},{:X} margin={},{} ws={:X} lists={} a8={:X} a9={:X} aa={:X},{:X}",
            hex8(t.vtable_00),
            hex8(t.exterior_coordinates_20[0] as u32),
            hex8(t.exterior_coordinates_20[1] as u32),
            hex8(t.exterior_coordinates_20[2] as u32),
            hex8(t.exterior_coordinates_20[3] as u32),
            u8::from(t.current_exterior_cell_30.is_some()),
            u8::from(t.current_interior_cell_34.is_some()),
            t.field_40,
            t.field_44,
            hex8(t.last_exterior_coordinates_48[0] as u32),
            hex8(t.last_exterior_coordinates_48[1] as u32),
            t.byte_50,
            t.cell_test_active_51,
            t.byte_52,
            t.cell_borders_enabled_53,
            u8::from(t.water_surface_54.is_some()),
            u8::from(t.water_node_58.is_some()),
            t.field_60,
            t.field_64,
            t.bytes_68[0],
            t.bytes_68[1],
            t.bytes_68[2],
            t.bytes_68[3],
            bits(&[t.grid_center_margin_x_6c]),
            bits(&[t.grid_center_margin_y_70]),
            u8::from(t.current_world_space_74.is_some()),
            lists.join(","),
            t.byte_a8,
            t.texture_purge_allowed_a9,
            t.bytes_aa[0],
            t.bytes_aa[1],
        ),
        "T tes ",
    );
    let zero = |values: &[Option<Entity>]| u8::from(values.iter().all(Option::is_none));
    rows.compare(
        &format!(
            "T cellBuffers interior={:X}/zero={} exterior={:X}/zero={}",
            t.interior_cell_buffer_38.len(),
            zero(&t.interior_cell_buffer_38),
            t.exterior_cell_buffer_3c.len(),
            zero(&t.exterior_cell_buffer_3c),
        ),
        "T cellBuffers",
    );
    let gc = t.grid_cells_08.as_ref().unwrap();
    let cells_zero = gc
        .grid_10
        .iter()
        .all(|entry| entry.cell_00.is_none() && entry.water_04.is_none());
    rows.compare(
        &format!(
            "T gridCell vt={} x={} y={} size={:X} entries={:X} zero={} floats={} b20={:X} p24={:X}",
            hex8(gc.original_vtable),
            gc.world_x_04,
            gc.world_y_08,
            gc.size_0c,
            gc.grid_10.len(),
            u8::from(cells_zero),
            bits(&[gc.float_14, gc.float_18, gc.float_1c]),
            gc.byte_20,
            u8::from(gc.pointer_24.is_some()),
        ),
        "T gridCell",
    );
    let gd = t.grid_distant_04.as_ref().unwrap();
    let cleared = gd.entries_10.iter().all(|entry| {
        entry.byte_00 == 0
            && entry.byte_01 == 0
            && entry.object_04.is_none()
            && entry.x_08 == 0
            && entry.y_0c == 0
    });
    rows.compare(
        &format!(
            "T gridDistant vt={} x={} y={} size={:X} count={:X} cleared={}",
            hex8(gd.original_vtable),
            gd.world_x_04,
            gd.world_y_08,
            gd.size_0c,
            gd.entries_10.len(),
            u8::from(cleared),
        ),
        "T gridDistant",
    );
    let world = &run.world;
    let temp = t.temp_node_manager_14.unwrap();
    let children = &world
        .get::<NiNodeChildren>(run.scene.root)
        .unwrap()
        .slots_ac;
    let parent = world
        .get::<recomp_game::gamebryo::av_object::NiAvParent>(temp)
        .unwrap()
        .0;
    rows.compare(
        &format!(
            "T temp vt={} flags={} parent={} root={} count={}",
            hex8(
                world
                    .get::<BsTempNodeManager>(temp)
                    .unwrap()
                    .original_vtable
            ),
            hex4(
                world
                    .get::<SceneNodePayload>(temp)
                    .unwrap()
                    .effective_av_flags
            ),
            if parent == Some(run.scene.root) {
                "root"
            } else {
                "?"
            },
            name_of(world, run.scene.root),
            children.len(),
        ),
        "T temp",
    );
    let linked = |value: Option<Entity>, want: Entity| {
        if value == Some(want) { "set" } else { "null" }
    };
    rows.compare(
        &format!(
            "T links land={} sky={} sun={} fog={}",
            linked(t.land_lod_10, run.scene.land_lod),
            if t.sky_5c { "set" } else { "null" },
            linked(t.sun_light_18, run.scene.sun_light),
            linked(t.fog_1c, run.scene.fog),
        ),
        "T links",
    );
}

fn lockfree_line(name: &str, map: &LockFreeMap) -> String {
    let nonzero = map.buckets_0c.iter().filter(|&&value| value != 0).count();
    format!(
        "L {name} vt={} buckets={:X} arg10={:X} f18={:X} lock={:X}/{:X} nonzero={nonzero}",
        hex8(map.original_vtable),
        map.bucket_count_08,
        map.field_10,
        map.field_18,
        map.lock_14.count_00,
        map.lock_14.field_0c,
    )
}

fn queue_line(name: &str, queue: &LockFreeQueue) -> String {
    format!(
        "Q {name} vt={} same=1 node={}{} arg0c={:X} f18={:X} lock={:X}/{:X}",
        hex8(queue.original_vtable),
        hex8(queue.sentinel[0]),
        hex8(queue.sentinel[1]),
        queue.field_0c,
        queue.field_18,
        queue.lock_14.count_00,
        queue.lock_14.field_0c,
    )
}

fn wait_until_waiting(thread: &BsTaskThread) -> bool {
    for _ in 0..500 {
        if thread.waiting() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

fn check_tasks(run: &TesRun, rows: &Rows) {
    let tasks = run.world.resource::<TaskGlobals>();
    let io = tasks.io_manager_b33a10.as_ref().unwrap();
    let task = &io.threads_28[0];
    rows.compare(
        &format!(
            "O io vt={} entries_buckets={:X} arg10={:X} f18={:X} f1c={:X} f20={:X} threads={:X} f38={:X} thread30={}",
            hex8(io.original_vtable),
            io.task_map.bucket_count_08,
            io.task_map.field_10,
            io.task_map.field_18,
            io.field_1c,
            io.field_20,
            io.thread_count_24,
            io.field_38,
            if io.main_thread_id_30 == task.thread_id_08 { "first" } else { "?" },
        ),
        "O io vt",
    );
    rows.compare(
        &format!(
            "O io.lock count={:X} f0c={:X}",
            io.task_map.lock_14.count_00, io.task_map.lock_14.field_0c
        ),
        "O io.lock",
    );
    let all_zero = |values: &[u32]| u8::from(values.iter().all(|&value| value == 0));
    rows.compare(
        &format!(
            "O io.zero buckets={} array2c={}",
            all_zero(&io.task_map.buckets_0c),
            u8::from(all_zero(&io.array_2c) == 1 && io.array_2c.len() == 18),
        ),
        "O io.zero",
    );
    let state = task.state();
    let counter =
        |value: &std::sync::atomic::AtomicI32| value.load(std::sync::atomic::Ordering::Acquire);
    rows.compare(
        &format!(
            "T taskThread vt={} c0c={:X} m10={:X} c18={:X} m1c={:X} manager=io",
            hex8(task.original_vtable),
            counter(&state.counter_0c),
            state.maximum_10,
            counter(&state.counter_18),
            state.maximum_1c,
        ),
        "T taskThread",
    );
    let loader = tasks.model_loader_b33a1c.as_ref().unwrap();
    let clone = &loader.clone_thread_18;
    let state = clone.thread.state();
    rows.compare(
        &format!(
            "T cloneThread vt={} c0c={:X} m10={:X} c18={:X} m1c={:X} f24={:X}",
            hex8(clone.thread.original_vtable),
            counter(&state.counter_0c),
            state.maximum_10,
            counter(&state.counter_18),
            state.maximum_1c,
            clone.field_24,
        ),
        "T cloneThread",
    );
    rows.compare(&lockfree_line("map0", &loader.model_map_00), "L map0");
    rows.compare(&lockfree_line("map1", &loader.kf_map_04), "L map1");
    rows.compare(&lockfree_line("map2", &loader.reference_map_08), "L map2");
    rows.compare(&lockfree_line("map3", &loader.idle_map_0c), "L map3");
    rows.compare(&lockfree_line("map4", &loader.helmet_map_10), "L map4");
    rows.compare(
        &lockfree_line("B35B90", tasks.exterior_cell_tasks_b35b90.as_ref().unwrap()),
        "L B35B90",
    );
    rows.compare(
        &lockfree_line("B35B8C", tasks.distant_lod_tasks_b35b8c.as_ref().unwrap()),
        "L B35B8C",
    );
    rows.compare(&queue_line("ioQueue", &io.io_queue_34), "Q ioQueue");
    rows.compare(
        &queue_line("distant3D", &loader.distant_3d_queue_14),
        "Q distant3D",
    );
    rows.compare(
        &queue_line("cloneQueue", &clone.reference_queue_28),
        "Q cloneQueue",
    );
    // D3: both workers run and block on their +0x14 semaphore (P09-RUN-001 record 669: both threads inside their run loops' first wait).
    assert!(
        wait_until_waiting(task),
        "BSTaskManagerThread reaches its wait"
    );
    assert!(
        wait_until_waiting(&clone.thread),
        "BackgroundCloneThread reaches its wait"
    );
    assert_ne!(
        task.thread_id_08, clone.thread.thread_id_08,
        "distinct worker threads"
    );
    assert_eq!(task.wait_trace(), [0x0043_3BC0], "00433BC0's wait");
    assert_eq!(clone.thread.wait_trace(), [0x0043_CC20], "0043CC20's wait");
    assert_ne!(tasks.performance_frequency_b33a08, 0, "00B33A08 frequency");
    // The original's order: the IOManager's thread, its frequency, then the ModelLoader's.
    let semaphore = |site: u32, initial: i32| TaskCall::CreateSemaphore {
        site,
        initial,
        maximum: 1,
    };
    assert_eq!(
        tasks.trace,
        [
            semaphore(0x0043_0E28, 0),
            semaphore(0x0043_0E45, 1),
            TaskCall::CreateThread {
                site: 0x0043_0E5A,
                kind: 2
            },
            TaskCall::ResumeThread {
                site: 0x0043_0DF0,
                kind: 2
            },
            TaskCall::QueryPerformanceFrequency { site: 0x0043_44F8 },
            semaphore(0x0043_0E28, 0),
            semaphore(0x0043_0E45, 1),
            TaskCall::CreateThread {
                site: 0x0043_0E5A,
                kind: 3
            },
            TaskCall::ResumeThread {
                site: 0x0043_0DF0,
                kind: 3
            },
        ],
        "the thread call order"
    );
}

fn check_save_load(run: &TesRun, rows: &Rows) {
    let s = run.globals().save_load_game.as_ref().unwrap();
    let join = |values: &[u32]| {
        values
            .iter()
            .map(|value| format!("{value:X}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    let bytes: Vec<u32> = s.bytes_a8.iter().map(|&byte| u32::from(byte)).collect();
    rows.compare(
        &format!(
            "V save f04={:X} f14={} f4c={:X} f50={:X} f64={:X} f68={:X} f6c={:X} b70={:X} b71={:X} b7c={:X} b7d={:X} f80={} f94={} fa4={:X} ba8={} fac={:X} f1c0={:X}",
            s.field_04,
            join(&s.fields_14),
            s.field_4c,
            s.field_50,
            s.field_64,
            s.field_68,
            s.field_6c,
            s.byte_70,
            s.byte_71,
            s.byte_7c,
            s.byte_7d,
            join(&s.fields_80),
            join(&s.fields_94),
            s.field_a4,
            join(&bytes),
            s.field_ac,
            s.field_1c0,
        ),
        "V save",
    );
    let map_line = |offset: &str, map: &NiTPointerMapRecord| {
        format!(
            "V map+{offset} vt={} size={:X} count={:X}",
            hex8(map.original_vtable),
            map.hash_size_04,
            map.count_0c
        )
    };
    rows.compare(&map_line("0", &s.changes_map_00), "V map+0 ");
    rows.compare(&map_line("8", &s.interior_references_08), "V map+8 ");
    rows.compare(&map_line("C", &s.exterior_references_0c), "V map+C ");
    rows.compare(&map_line("10", &s.exterior_references_10), "V map+10 ");
    rows.compare(&map_line("54", &s.numeric_id_maps_54[0]), "V map+54 ");
    rows.compare(&map_line("58", &s.numeric_id_maps_54[1]), "V map+58 ");
    rows.compare(&map_line("5C", &s.numeric_id_maps_54[2]), "V map+5C ");
    rows.compare(&map_line("60", &s.numeric_id_maps_54[3]), "V map+60 ");
    let array_line = |offset: &str, array: &NiTLargeArrayU32| {
        format!(
            "V array+{offset} vt={} cap={:X} size={:X} count={:X} grow={:X} data0={:X}",
            hex8(array.original_vtable),
            array.capacity_08,
            array.size_0c,
            array.count_10,
            array.grow_by_14,
            array.data_04[0],
        )
    };
    rows.compare(&array_line("74", &s.array_74), "V array+74");
    rows.compare(&array_line("78", &s.array_78), "V array+78");
    assert_eq!(
        run.world.resource::<GameThreadLocals>().track_changes_185,
        0,
        "TLS +0x185 cleared"
    );
}

fn tes_run001(rows: &Rows) {
    let mut run = TesRun::new(pinned_collections(), false, None);
    assert_eq!(run.data_handler_calls, 1);
    assert_eq!(
        run.file_list_path.as_deref(),
        Some("Data\\"),
        "DataHandler and file-list steps run once with Data\\"
    );
    assert_eq!(
        run.object_load_gate_calls, 1,
        "00B3F958 stored without bTintMipMaps"
    );
    check_tes_globals(&run, rows);
    check_havok(&mut run, rows);
    check_tes_object(&run, rows);
    check_tasks(&run, rows);
    check_save_load(&run, rows);
    // The workers join here, with no hang: the shutdown under test.
    run.world
        .resource_mut::<TaskGlobals>()
        .stop_worker_threads();
}

// --- 09-D: the DataHandler, its skills and the plugin list ---------------------------------

fn bs_string(text: &BsStringT) -> String {
    format!(
        "{}/{:X}/{:X}",
        text.data_00
            .as_deref()
            .map_or("<null>".to_owned(), |data| String::from_utf8_lossy(data)
                .into_owned()),
        text.length_04,
        text.capacity_06
    )
}

fn check_data_handler(run: &TesRun, rows: &Rows) {
    let world = &run.world;
    let d = world.resource::<DataHandler>();
    let lists = world
        .resource::<recomp_game::tes::data_handler::DataHandlerLists>()
        .globals_74
        .is_empty();
    let cd0: Vec<String> = d.bytes_cd0.iter().map(|byte| format!("{byte:X}")).collect();
    let b8 = format!(
        "{:X},{:X},{:X},{:X}",
        d.unknown_8b8[0], d.unknown_8b8[1], d.unknown_8b8[2], d.unknown_8b8[3]
    );
    rows.compare(
        &format!(
            "B dh lists={} cells vt={} data={:X} cap={:X} size={:X} count={:X} grow={:X} 8b8={b8} 8d0={:X} mods={} cd0={} cdc={:X}",
            if lists { "zero" } else { "set" },
            hex8(d.cells_c0.original_vtable),
            d.cells_c0.data_04,
            d.cells_c0.capacity_08,
            d.cells_c0.size_0c,
            d.cells_c0.count_10,
            d.cells_c0.grow_by_14,
            d.loaded_mod_count_8d0,
            if d.mods_by_id_8d4.iter().all(Option::is_none) { "zero" } else { "set" },
            cd0.join(","),
            d.field_cdc,
        ),
        "B dh ",
    );
    let globals = world.resource::<DataHandlerGlobals>();
    let idle = globals.idle_anim_roots_b362c0.as_ref().unwrap();
    rows.compare(
        &format!(
            "B parts bound={:X},{:X},{:X},{:X} region vt={} list={:X},{:X} owns={:X} rdm vt={} f04={:X} idle vt={} size={:X} count={:X} b10={:X} b35460={:X}",
            d.bound_objects_00[0],
            d.bound_objects_00[1],
            d.bound_objects_00[2],
            d.bound_objects_00[3],
            hex8(d.region_list_bc.original_vtable),
            d.region_list_bc.list_04[0],
            d.region_list_bc.list_04[1],
            d.region_list_bc.owns_0c,
            hex8(d.region_data_manager_cd8.original_vtable),
            d.region_data_manager_cd8.field_04,
            hex8(idle.original_vtable),
            idle.hash_size_04,
            idle.count_0c,
            idle.byte_10,
            globals.region_list_count_b35460,
        ),
        "B parts",
    );
    for (index, &skill) in d.skills_d8.iter().enumerate() {
        let identity = world.get::<FormIdentity>(skill).unwrap();
        let vtable = world.get::<OriginalVtable>(skill).unwrap().0;
        let mods = world.get::<ModReferences>(skill).unwrap();
        let k = world.get::<TesSkill>(skill).unwrap();
        let levels: Vec<String> = k
            .level_descriptions_40
            .iter()
            .map(|level| format!("{}/{:X}", hex8(level.original_vtable), level.offset_04))
            .collect();
        let prefix = format!("B skill{index} ");
        rows.compare(
            &format!(
                "{prefix}vt={} type={:X} flags={:X} id={:X} mods={},{} desc={}/{:X} tex={}/{:X}/{:X}/{:X} skill={:X} f30={:X} f34={:X} f38={} f3c={} levels={}",
                hex8(vtable),
                identity.form_type_04,
                identity.flags_08,
                identity.form_id_0c,
                u8::from(!mods.0.is_empty()),
                u8::from(mods.0.len() > 1),
                hex8(k.description_18.original_vtable),
                k.description_18.offset_04,
                hex8(k.texture_20.original_vtable),
                u8::from(k.texture_20.path_04.data_00.is_some()),
                k.texture_20.path_04.length_04,
                k.texture_20.path_04.capacity_06,
                k.skill_2c,
                k.field_30,
                k.specialization_34,
                bits(&[k.float_38]),
                bits(&[k.float_3c]),
                levels.join(","),
            ),
            &prefix,
        );
    }
    let forms = world.resource::<FormsById>();
    let mut chains = String::new();
    for key in 0x3D_u32..=0x51 {
        let chain: Vec<String> = forms
            .bucket(key as usize % FORMS_BY_ID_BUCKETS)
            .iter()
            .map(|&(entry, form)| {
                let skill = d.skills_d8.iter().position(|&skill| skill == form);
                format!("{entry:X}=skill{}", skill.map_or(-1, |index| index as i64))
            })
            .collect();
        chains.push_str(&format!(" {key:X}:{}", chain.join("|")));
    }
    rows.compare(
        &format!(
            "B forms vt={} size={FORMS_BY_ID_BUCKETS:X} count={:X}{chains}",
            hex8(FORMS_BY_ID_VTABLE),
            forms.count(),
        ),
        "B forms",
    );
}

fn check_file_list(run: &TesRun, rows: &Rows, bs_file_row: &str) {
    let d = run.world.resource::<DataHandler>();
    let mut order = Vec::new();
    for (index, id) in d.files_8c8.iter().enumerate() {
        let f: &TesFileData = d.file_store.get(*id).unwrap();
        order.push(f.name_1c.clone());
        let f22c: Vec<String> = f
            .fields_22c
            .iter()
            .map(|word| format!("{word:X}"))
            .collect();
        let r = &f.record_23c;
        let masters: Vec<String> = f
            .masters_3e0
            .iter()
            .map(|master| String::from_utf8_lossy(master).into_owned())
            .collect();
        let sizes: Vec<String> = f
            .master_sizes_3e8
            .iter()
            .map(|block| hex_upper(block.iter().copied()))
            .collect();
        let chunk: String = f
            .chunk_type_250
            .to_le_bytes()
            .iter()
            .map(|&b| char::from(b))
            .collect();
        let fd = &f.find_data_290;
        let prefix = format!("F file{index} ");
        rows.compare(
            &format!(
                "{prefix}name={} dir={} err={:X} f224={:X} f228={:X} f22c={} rec={},{},{},{},{} chunk={chunk}/{:X} size={:X} off={:X},{:X},{:X} f27c={:X},{:X} groups={:X},{:X} hdr={:X} hedr={},{},{} flags={:X} count={:X} f3f8={:X},{:X} modidx={:X} author={} desc={} dc={:X},{:X} fd={:X},{}{},{:X},{:X},{} masters={} sizes={} resolved={}",
                f.name_1c,
                f.directory_120,
                f.error_state_00,
                f.field_224,
                f.buffer_size_228,
                f22c.join(","),
                hex8(r.type_23c),
                hex8(r.size_240),
                hex8(r.flags_244),
                hex8(r.form_id_248),
                hex8(r.tracking_24c),
                f.chunk_size_254,
                f.file_size_258,
                f.record_offset_25c,
                f.chunk_offset_260,
                f.fetched_264,
                f.field_27c,
                f.field_280,
                f.open_groups_284[0],
                f.open_groups_284[1],
                f.header_read_28c,
                hex8(f.header_3d0[0]),
                hex8(f.header_3d0[1]),
                hex8(f.header_3d0[2]),
                f.flags_3dc,
                f.master_count_3f0,
                f.field_3f8,
                f.field_3fc,
                f.mod_index_400,
                bs_string(&f.author_404),
                bs_string(&f.description_40c),
                u8::from(f.decompressed_414.is_some()),
                f.decompressed_length_418,
                fd.attributes,
                hex8((fd.last_write_time >> 32) as u32),
                hex8(fd.last_write_time as u32),
                fd.size_high,
                fd.size_low,
                fd.file_name,
                masters.join("|"),
                sizes.join("|"),
                if f.master_files_allocated { "set" } else { "null" },
            ),
            &prefix,
        );
        let b = f
            .bs_file_10
            .as_ref()
            .expect("the TESFile keeps its BSFile open");
        // The vtable/procs are the BSFile's original identities (phase09_run001.rows); the protected +0x28/+0x2C/+0x34/+0x38/+0x140/+0x144 fields are 0 from the constructor.
        let bprefix = format!("F bs{index} ");
        rows.compare(
            &format!(
                "{bprefix}{bs_file_row} bufsize={:X} valid={:X} pos={:X} mode={:X} good={:X} b28=0 f2c=0 fixed={} f34=0 f38=0 path={} f140=0 f144=0 position={:X} f14c={:X} cached={:X}",
                b.buffer_size(),
                b.buffer_valid(),
                b.core().buffer_position(),
                b.core().mode(),
                u8::from(b.good()),
                hex8(b.fixed_position() as u32),
                b.path(),
                b.position(),
                b.position_shadow(),
                b.cached_size(),
            ),
            &bprefix,
        );
    }
    rows.compare(&format!("F order={}", order.join(",")), "F order=");
    let constructed: Vec<String> = d
        .file_store
        .iter()
        .map(|file| file.name_1c.clone())
        .collect();
    rows.compare(
        &format!("F constructed={}", constructed.join(",")),
        "F constructed=",
    );
    let cache = run.world.resource::<DataHandlerGlobals>().form_type_cache;
    rows.compare(
        &format!(
            "F cache b33c1c={:X} b33c20={}",
            cache.last_type_b33c1c,
            hex8(cache.last_fourcc_b33c20)
        ),
        "F cache",
    );
}

fn data_handler_run001(rows: &Rows, working: &Path, bs_file_row: &str) {
    let run = TesRun::new(pinned_collections(), false, Some(working));
    assert!(
        run.warnings.is_empty(),
        "no plugin warnings with the shipped Data: {:?}",
        run.warnings
    );
    check_data_handler(&run, rows);
    check_file_list(&run, rows, bs_file_row);
    assert_eq!(
        run.world.resource::<GameThreadLocals>().byte_184,
        0,
        "TLS +0x184 cleared"
    );
}

// --- 09-E: the water records and the WaterShader prepare ------------------------------------

/// IDA: N/A what stays the same across world runs: the files and the factories.
struct Harness {
    root: PathBuf,
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
            "recomp_phase09_{}_{}",
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
        Self {
            root,
            callbacks,
            finder,
            archives: RefCell::new(archives),
            registry,
        }
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

/// IDA: N/A a fixture texture set for the bUseWaterShader-off arm: FindFile finds water00..water<frames - 1> and CreateFromFile names a SourceTexture.
#[derive(Default)]
struct WaterTextureFixture {
    frames: u32,
    palette: BTreeMap<Vec<u8>, Entity>,
    created: Vec<Vec<u8>>,
}

impl WaterTextureServices for WaterTextureFixture {
    fn palette_find(&mut self, _: &mut World, path: &[u8]) -> Option<Entity> {
        self.palette.get(path).copied()
    }
    fn palette_add(&mut self, _: &mut World, path: &[u8], texture: Entity) {
        self.palette.insert(path.to_vec(), texture);
    }
    fn find_file(&mut self, path: &[u8], _: u32) -> Option<(u32, Vec<u8>)> {
        let hit = (0..self.frames)
            .any(|frame| format!("Textures\\water\\water{frame:02}.DDS").as_bytes() == path);
        Some(if hit {
            (2, path.to_vec())
        } else {
            (0, Vec::new())
        })
    }
    fn create_from_file(&mut self, world: &mut World, path: &[u8]) -> Option<Entity> {
        self.created.push(path.to_vec());
        Some(
            world
                .spawn((
                    NiClass::SourceTexture,
                    NiObjectNet::default(),
                    SourceTexture {
                        filename_34: Some(path.to_vec()),
                        ..SourceTexture::default()
                    },
                ))
                .id(),
        )
    }
    fn io_thread_id(&self) -> std::thread::ThreadId {
        std::thread::current().id()
    }
}

#[derive(Clone, Copy)]
struct Options {
    water_shader: bool,
    image_space_effects: u8,
    water_render_path: i32,
    frames: Option<u32>,
    existing_tree_manager: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            water_shader: true,
            image_space_effects: 1,
            water_render_path: 5,
            frames: None,
            existing_tree_manager: false,
        }
    }
}

/// IDA: N/A one 00406360 run from the Phase 05 scene graphs through the Phase 08 prelude, the TES constructor, 0043F560, 0055F750 and the world update.
struct WorldRun {
    world: World,
    scenes: SceneGraphGlobals,
    prelude: SkyPreludeResult,
    collections: SettingCollections,
    shader_globals: BsShaderGlobals,
    water_shader_trace: Vec<Entity>,
    lava_b47844: u32,
    warnings: Vec<String>,
    log: Vec<&'static str>,
    fixture: WaterTextureFixture,
    /// IDA: N/A the counters at window start: geometry data ID, effect UID, material ID.
    counters_at_window: (u16, u32, u32),
}

fn counters(world: &World) -> (u16, u32, u32) {
    (
        world
            .get_resource::<recomp_game::gamebryo::mesh_geometry::GeometryDataIdCounter>()
            .map_or(0, |counter| counter.0),
        world
            .get_resource::<recomp_game::gamebryo::scene_derived::DynamicEffectUidCounter>()
            .map_or(0, |counter| counter.0),
        world
            .get_resource::<MaterialIdCounter>()
            .map_or(0, |counter| counter.0),
    )
}

impl WorldRun {
    fn new(harness: &Harness, options: Options) -> Self {
        let mut collections = pinned_collections();
        let mut world = World::new();
        let mut known = StartupCallbackScheduler::default();
        register_known_crt_callbacks(&mut known).unwrap();
        known.run(&mut world).unwrap();
        world.insert_resource(GameSettingCollection::default());
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
                image_space_effects: options.image_space_effects,
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
        let mut load = |world: &mut World, path: &str| load_stream(harness, world, path);
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
        let mut log = Vec::new();
        let prelude = {
            let mut log_sink = |_: &'static str| {};
            run_sky_prelude(
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
            .unwrap()
        };
        let counters_at_window = counters(&world);
        // 00406604..00406687: TES, 0043F560, 0055F750(0), the world update.
        collections.ini.find_address_mut(0x00B0_7050).unwrap().raw =
            u32::from(options.water_shader);
        if options.existing_tree_manager {
            world.init_resource::<TreeManagerGlobals>();
            world
                .resource_mut::<TreeManagerGlobals>()
                .tree_manager_b39e04 =
                Some(recomp_game::bs::tree_manager::BsTreeManager::default());
        }
        let mut registry = harness.registry.clone();
        let (havok_settings, debris_max_velocity, debris_min_extent, step_speed) =
            tes_setting_inputs(
                world.resource::<GameSettingCollection>(),
                &collections.ini,
                current_directory_input(),
            )
            .unwrap();
        let mut construct = |_: &mut World| Ok(());
        let mut build = |_: &mut World, _: &str| Ok(());
        let mut gate = |_: &mut World| {};
        let mut shader_globals = BsShaderGlobals::default();
        let mut warnings = Vec::new();
        let mut warning = |text: String| warnings.push(text);
        let mut fixture = WaterTextureFixture {
            frames: options.frames.unwrap_or(0),
            ..WaterTextureFixture::default()
        };
        let mut log_sink = |text: &'static str| log.push(text);
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
                    image_space_effects: options.image_space_effects,
                    render_path: options.water_render_path,
                    prepare: Some(&mut prepare),
                    textures: options
                        .frames
                        .map(|_| &mut fixture as &mut dyn WaterTextureServices),
                    warning: &mut warning,
                },
                world_scene: scenes.world,
                property_defaults: defaults,
                update: &mut runtime,
                log: &mut log_sink,
            },
        )
        .unwrap();
        let water_shader_trace = std::mem::take(&mut prepare.water_shader_trace);
        let lava_b47844 = prepare.lava_b47844;
        Self {
            world,
            scenes,
            prelude,
            collections,
            shader_globals,
            water_shader_trace,
            lava_b47844,
            warnings,
            log,
            fixture,
            counters_at_window,
        }
    }

    fn tes(&self) -> &Tes {
        self.world.resource::<TesGlobals>().tes.as_deref().unwrap()
    }

    fn raw(&self, address: u32) -> u32 {
        self.collections.ini.find_address(address).unwrap().raw
    }

    fn water_root(&self) -> Entity {
        self.world
            .resource::<WaterGlobals>()
            .water_root_b35230
            .unwrap()
    }
}

/// IDA: N/A prints the WaterRoot tree in the gen_expected.py line format.
struct WaterPrinter<'r> {
    world: &'r World,
    names: HashMap<Entity, String>,
    printed: HashSet<Entity>,
    first_material: Option<u32>,
    first_data: Option<u16>,
}

impl<'r> WaterPrinter<'r> {
    fn new(run: &'r WorldRun) -> Self {
        let mut printer = Self {
            world: &run.world,
            names: HashMap::new(),
            printed: HashSet::new(),
            first_material: None,
            first_data: None,
        };
        if let Some(light) = run.tes().sun_light_18 {
            printer.names.insert(light, "sun".to_owned());
        }
        if let Some(fog) = run.tes().fog_1c {
            printer.names.insert(fog, "fog".to_owned());
        }
        printer.assign(run.water_root(), "waterRoot");
        printer
    }

    fn assign(&mut self, object: Entity, path: &str) {
        self.names.entry(object).or_insert_with(|| path.to_owned());
        let world = self.world;
        if let Some(list) = world.get::<NiAvProperties>(object) {
            for (index, property) in list.0.iter().enumerate() {
                if let Some(property) = property {
                    self.names
                        .entry(*property)
                        .or_insert_with(|| format!("{path}.p{index}"));
                }
            }
        }
        if let Some(children) = world.get::<NiNodeChildren>(object) {
            for (index, child) in children.slots_ac.iter().enumerate() {
                if let Some(child) = child {
                    self.assign(*child, &format!("{path}.{index}"));
                }
            }
        }
    }

    fn sym(&self, identity: Option<Entity>) -> String {
        identity.map_or("null".to_owned(), |entity| {
            self.names
                .get(&entity)
                .cloned()
                .unwrap_or_else(|| "other".to_owned())
        })
    }

    fn property_lines(&mut self, object: Entity, path: &str, out: &mut Vec<String>) {
        let world = self.world;
        let Some(list) = world.get::<NiAvProperties>(object) else {
            return;
        };
        for (index, property) in list.0.iter().enumerate() {
            let property = property.unwrap();
            let tag = format!("{path}.p{index}");
            if !self.printed.insert(property) {
                out.push(format!("P {tag} ref={}", self.sym(Some(property))));
                continue;
            }
            let body = if let Some(alpha) = world.get::<AlphaProperty>(property) {
                format!(
                    "NiAlphaProperty flags={} ref={}",
                    hex4(alpha.flags_18),
                    alpha.alpha_test_ref_1a
                )
            } else if let Some(z) = world.get::<ZBufferProperty>(property) {
                format!("NiZBufferProperty flags={}", hex4(z.flags_18))
            } else if let Some(texturing) = world.get::<TexturingProperty>(property) {
                format!("NiTexturingProperty flags={}", hex4(texturing.flags_18))
            } else if let Some(stencil) = world.get::<StencilProperty>(property) {
                format!(
                    "NiStencilProperty flags={} ref={:X} mask={}",
                    hex4(stencil.flags_18),
                    stencil.reference_1c,
                    hex8(stencil.mask_20)
                )
            } else if let Some(material) = world.get::<MaterialProperty>(property) {
                let first = *self.first_material.get_or_insert(material.material_id_18);
                material_body(material, first)
            } else if let Some(water) = world.get::<WaterShaderProperty>(property) {
                format!(
                    "WaterShaderProperty vt={} flags={} pass_info={} alpha={} last={} list0={}",
                    hex8(WaterShaderProperty::ORIGINAL_VTABLE),
                    hex4(water.flags_18),
                    water.pass_info_1c,
                    bits(&[water.alpha_20]),
                    water.last_pass_state_24,
                    water.render_pass_counts[0],
                )
            } else {
                "?".to_owned()
            };
            out.push(format!("P {tag} {body}"));
        }
    }

    fn walk(&mut self, object: Entity, path: &str, out: &mut Vec<String>) {
        let world = self.world;
        let av = world.get::<NiAvWorld>(object).unwrap();
        let props = world
            .get::<NiAvProperties>(object)
            .map_or(0, |list| list.0.len());
        let name = name_of(world, object);
        if let Some(children) = world.get::<NiNodeChildren>(object) {
            let payload = world.get::<SceneNodePayload>(object).unwrap();
            let effects = &world.get::<NiNodeEffects>(object).unwrap().0;
            out.push(format!(
                "A {path} NiNode name={name} flags={} bound={} local={} world={} children={} effects={} props={props}",
                hex4(payload.effective_av_flags),
                bits(&av.bound),
                transform_bits(&payload.local_transform),
                transform_bits(&av.transform),
                children.slots_ac.len(),
                effects.len(),
            ));
            self.property_lines(object, path, out);
            for (index, effect) in effects.iter().enumerate() {
                out.push(format!(
                    "E {path} effect{index}={}",
                    self.sym(Some(*effect))
                ));
            }
            for (index, child) in children.slots_ac.iter().enumerate() {
                self.walk(child.unwrap(), &format!("{path}.{index}"), out);
            }
            return;
        }
        assert_eq!(
            world.get::<NiClass>(object),
            Some(&NiClass::TriShape),
            "water tree leaf must be a NiTriShape"
        );
        let payload = world.get::<MeshTriShapePayload>(object).unwrap();
        let links = world.get::<NiGeometry>(object).unwrap();
        out.push(format!(
            "A {path} NiTriShape name={name} flags={} bound={} local={} world={} state={} estate={} shader={} props={props}",
            hex4(payload.effective_av_flags),
            bits(&av.bound),
            transform_bits(&payload.local_transform),
            transform_bits(&av.transform),
            set_or_null(links.property_state_ac),
            set_or_null(links.effect_state_b0),
            set_or_null(world.get::<NiGeometryShader>(object)),
        ));
        self.property_lines(object, path, out);
        if let Some(state) = links.property_state_ac {
            let state = world.get::<ScenePropertyState>(state).unwrap();
            let mut q = format!("Q {path}");
            for slot in state.slots {
                q.push(' ');
                q.push_str(&self.sym(slot));
            }
            out.push(q);
        }
        let data_entity = links.data_b4.unwrap();
        if !self.printed.insert(data_entity) {
            out.push(format!("G {path} ref"));
            return;
        }
        let shape = world.get::<MeshTriShapeData>(data_entity).unwrap();
        let d = &shape.base;
        let first = *self.first_data.get_or_insert(d.data_id_0a);
        let array = |empty: bool, letter: char| if empty { '-' } else { letter };
        let arrays: String = [
            array(d.positions_1c.is_empty(), 'p'),
            array(d.normals_20.is_empty(), 'n'),
            array(d.colors_24.is_empty(), 'c'),
            array(d.texture_coordinates_28.is_empty(), 't'),
        ]
        .iter()
        .collect();
        out.push(format!(
            "G {path} NiTriShapeData count={} id=+{} bound={} arrays={arrays} format={} dirty={} keep={} compress={} triangles={} pos={} nrm={} col={} uv={} tri={}",
            d.vertex_count_08,
            d.data_id_0a - first,
            bits(&d.bounds_0c),
            hex4(d.geometry_flags_2c),
            hex4(d.dirty_flags_2e),
            d.keep_flags_30,
            d.compress_flags_31,
            shape.triangle_count_40,
            le_f32(&d.positions_1c),
            le_f32(&d.normals_20),
            le_f32(&d.colors_24),
            le_f32(&d.texture_coordinates_28),
            hex_upper(shape.triangle_list_48.iter().flat_map(|index| index.to_le_bytes())),
        ));
    }

    fn tree(&mut self, root: Entity) -> Vec<String> {
        let mut out = Vec::new();
        self.walk(root, "waterRoot", &mut out);
        out
    }
}

fn material_body(material: &MaterialProperty, first: u32) -> String {
    let v = &material.values;
    format!(
        "NiMaterialProperty id=+{} colors={},{},{},{} shine={} alpha={} 54={} 58={}",
        material.material_id_18 - first,
        bits(&v.ambient),
        bits(&v.diffuse),
        bits(&v.specular),
        bits(&v.emissive),
        bits(&[v.shine]),
        bits(&[v.alpha]),
        material.raw_state_54,
        material.raw_state_58,
    )
}

fn water_node_line(tag: &str, r: &TesWaterNodeRecord, printer: &WaterPrinter<'_>) -> String {
    let shapes: Vec<String> = r
        .shapes_08
        .iter()
        .map(|shape| printer.sym(*shape))
        .collect();
    format!(
        "W node {tag} b00={:X} node={} shapes={} path={} tex={} f14={:X} grid={:X} flip={} p20={} alpha={} f28={} f2c={} f30={} b34={:X}",
        r.byte_00,
        printer.sym(r.node_04),
        shapes.join("|"),
        set_or_null(r.texture_path_0c.as_ref()),
        set_or_null(r.texture_10),
        u8::from(r.cell_14.is_some()),
        r.grid_count_18,
        set_or_null(r.flip_controller_1c),
        set_or_null(r.pointer_20),
        printer.sym(r.alpha_24),
        bits(&[r.float_28]),
        bits(&[r.float_2c]),
        bits(&[r.float_30]),
        r.reset_done_34,
    )
}

const WATER_TREE_PREFIXES: [&str; 5] = [
    "A waterRoot",
    "P waterRoot",
    "E waterRoot",
    "Q waterRoot",
    "G waterRoot",
];

fn check_water(run: &WorldRun, rows: &Rows) {
    let tes = run.tes();
    let s = tes
        .water_surface_54
        .as_ref()
        .expect("TES +0x54 water record");
    assert!(tes.water_node_58.is_some(), "TES +0x58 water record");
    let mut printer = WaterPrinter::new(run);
    let pointers: Vec<&str> = s
        .pointers_00
        .iter()
        .map(|pointer| set_or_null(*pointer))
        .collect();
    rows.compare(
        &format!(
            "W surface p00={} f18={} f20={:X} f24={:X} b28={:X} b29={:X} f2c={} vt30={} list={:X},{:X},{:X} f40={:X} f44={} p48={}",
            pointers.join(","),
            bits(&[s.float_18]),
            s.dword_20,
            s.dword_24,
            s.byte_28,
            s.byte_29,
            bits(&[s.float_2c]),
            hex8(s.wading_list_vtable_30),
            s.wading_head_34,
            s.wading_tail_38,
            s.wading_count_3c,
            s.dword_40,
            bits(&[s.float_44]),
            set_or_null(s.pointer_48),
        ),
        "W surface",
    );
    let mut nodes = vec![water_node_line(
        "interior",
        tes.water_node_58.as_ref().unwrap(),
        &printer,
    )];
    let cells = tes.grid_cells_08.as_ref().unwrap();
    for x in 0..5 {
        for y in 0..5 {
            let slot = &cells.grid_10[y + x * cells.size_0c as usize];
            nodes.push(water_node_line(
                &format!("slot{}", x * 5 + y),
                slot.water_04.as_ref().unwrap(),
                &printer,
            ));
        }
    }
    rows.compare_lines(&nodes, &["W node "], "water records");
    assert_eq!(
        tes.water_node_58
            .as_ref()
            .unwrap()
            .texture_path_0c
            .as_deref(),
        Some(&b"Textures\\water\\water"[..]),
        "0049E6D3 texture base path"
    );
    let w = run.world.resource::<WaterGlobals>();
    let colors: Vec<f32> = w
        .shallow_color_b45e14
        .iter()
        .chain(&w.deep_color_b45e24)
        .chain(&w.reflection_color_b45e34)
        .copied()
        .collect();
    rows.compare(
        &format!(
            "W globals b45db9={:X} b45dbc={:X} colors={} b45ea4={} b45fd0={:X} b3521d={:X} b35220={:X} b35264={:X} b3526c={:X} b35254={} b35260={:X} b3524c={} b07048={} b07088={:X}",
            run.shader_globals.water_b45db9,
            w.depth_grids_b45dbc,
            bits(&colors),
            bits(&[w.tile_b45ea4]),
            u8::from(run.shader_globals.water_hi_res_b45fd0),
            w.byte_b3521d,
            w.dword_b35220,
            run.raw(0x00B3_5264),
            w.surface_texture_guard_b3526c,
            bits(&w.normal_b35254),
            w.tri_shape_guard_b35260,
            hex8(run.raw(0x00B3_524C)),
            hex8(run.raw(0x00B0_7048)),
            run.raw(0x00B0_7088),
        ),
        "W globals",
    );
    rows.compare_lines(
        &printer.tree(run.water_root()),
        &WATER_TREE_PREFIXES,
        "water tree",
    );
    // 0043F6BA: one WaterShader +0x18 per water geometry, in tree order; the lava byte stays clear (no shape carries a material).
    let shapes = rows
        .lines
        .iter()
        .filter(|line| line.starts_with("A waterRoot") && line.contains(" NiTriShape "))
        .count();
    assert_eq!(
        run.water_shader_trace.len(),
        shapes,
        "every water geometry prepared"
    );
    assert_eq!(run.lava_b47844, 0, "00B477F0+0x54 stays 0");
    assert!(
        run.warnings.is_empty(),
        "no water warnings: {:?}",
        run.warnings
    );
}

// --- 09-F: the TreeManager and the final world update -------------------------------------

fn check_tree_manager(run: &WorldRun, first_material: u32, rows: &Rows) {
    let world = &run.world;
    let g = world.resource::<TreeManagerGlobals>();
    let m = g.tree_manager_b39e04.as_ref().expect("00B39E04 published");
    let name = |identity: Option<Entity>| -> &str {
        match identity {
            None => "null",
            id if id == m.zbuffer_08 => "tm+8",
            id if id == m.material_0c => "tm+C",
            id if id == m.vertex_color_10 => "tm+10",
            id if id == m.alpha_14 => "tm+14",
            id if id == m.bsx_flags_18 => "bsx",
            _ => "set",
        }
    };
    rows.compare(
        &format!(
            "R tm f00={:X} p04={} zbuffer={} material={} vc={} alpha={} bsx={} f1c={} b20={:X} b21={:X} b22={:X} b23={:X} map={} b44f04={} b4297c={:X} b4297d={:X} wind={}/{}",
            m.cached_properties_00,
            name(m.pointer_04),
            name(m.zbuffer_08),
            name(m.material_0c),
            name(m.vertex_color_10),
            name(m.alpha_14),
            name(m.bsx_flags_18),
            bits(&[m.float_1c]),
            m.byte_20,
            m.byte_21,
            m.byte_22,
            m.force_full_lod_23,
            set_or_null(m.tree_nodes_24.as_ref()),
            bits(&[g.canopy_shadow_scale_b44f04]),
            g.byte_b4297c,
            g.byte_b4297d,
            g.wind_matrices_b42a10.count_00,
            if g.wind_matrices_b42a10.allocated_04 { "set" } else { "null" },
        ),
        "R tm ",
    );
    rows.compare(
        &format!(
            "P tm+8 NiZBufferProperty flags={}",
            hex4(
                world
                    .get::<ZBufferProperty>(m.zbuffer_08.unwrap())
                    .unwrap()
                    .flags_18
            )
        ),
        "P tm+8 ",
    );
    rows.compare(
        &format!(
            "P tm+C {}",
            material_body(
                world
                    .get::<MaterialProperty>(m.material_0c.unwrap())
                    .unwrap(),
                first_material
            )
        ),
        "P tm+C ",
    );
    rows.compare(
        &format!(
            "P tm+10 NiVertexColorProperty flags={}",
            hex4(
                world
                    .get::<VertexColorProperty>(m.vertex_color_10.unwrap())
                    .unwrap()
                    .flags_18
            )
        ),
        "P tm+10 ",
    );
    let alpha = world.get::<AlphaProperty>(m.alpha_14.unwrap()).unwrap();
    rows.compare(
        &format!(
            "P tm+14 NiAlphaProperty flags={} ref={:X}",
            hex4(alpha.flags_18),
            alpha.alpha_test_ref_1a
        ),
        "P tm+14 ",
    );
    let bsx = m.bsx_flags_18.unwrap();
    rows.compare(
        &format!(
            "R bsx vt={} flags={:X} name={}",
            hex8(BSX_FLAGS_VTABLE),
            world.get::<BsxFlagsExtraData>(bsx).unwrap().flags_0c,
            world
                .get::<NiExtraDataName>(bsx)
                .unwrap()
                .0
                .as_deref()
                .map_or("<null>".to_owned(), |name| String::from_utf8_lossy(name)
                    .into_owned()),
        ),
        "R bsx ",
    );
    let map = m.tree_nodes_24.as_ref().unwrap();
    rows.compare(
        &format!(
            "L treeMap vt={} buckets={:X} f10={:X} f18={:X} lock={:X}/{:X} nonzero={}",
            hex8(map.original_vtable),
            map.bucket_count_08,
            map.field_10,
            map.field_18,
            map.lock_14.count_00,
            map.lock_14.field_0c,
            if map.buckets_0c.iter().all(|&bucket| bucket == 0) {
                "0"
            } else {
                "set"
            },
        ),
        "L treeMap ",
    );
    let identity: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    assert_eq!(
        g.wind_matrices_b42a10.matrices_04, [identity; 4],
        "007A66B0/007A6670 identity wind matrices"
    );
}

fn final_line(world: &World, tag: &str, class: &str, node: Entity) -> String {
    let payload = world.get::<SceneNodePayload>(node).unwrap();
    let av = world.get::<NiAvWorld>(node).unwrap();
    format!(
        "S {tag} {class} name={} flags={} bound={} local={} world={} children={} effects={} props={}",
        name_of(world, node),
        hex4(payload.effective_av_flags),
        bits(&av.bound),
        transform_bits(&payload.local_transform),
        transform_bits(&av.transform),
        world.get::<NiNodeChildren>(node).unwrap().slots_ac.len(),
        world.get::<NiNodeEffects>(node).unwrap().0.len(),
        world
            .get::<NiAvProperties>(node)
            .map_or(0, |list| list.0.len()),
    )
}

fn check_final_world(run: &WorldRun, rows: &Rows) {
    let world = &run.world;
    let scene = run.scenes.world.unwrap();
    let child = |node: Entity, index: usize| {
        world.get::<NiNodeChildren>(node).unwrap().slots_ac[index]
            .expect("final dump child must be a node")
    };
    let mut host = Vec::new();
    let line = final_line(world, "world", "SceneGraph", scene);
    let start = line.find(" name=").unwrap();
    let end = line.find(" flags=").unwrap();
    host.push(format!("{} name=-{}", &line[..start], &line[end..]));
    host.push(final_line(world, "world.0", "NiNode", child(scene, 0)));
    host.push(final_line(
        world,
        "world.1",
        "ShadowSceneNode",
        child(scene, 1),
    ));
    let objlod = run.prelude.scene_root.unwrap();
    host.push(final_line(
        world,
        "objlod.0",
        "BSTempNodeManager",
        child(objlod, 0),
    ));
    host.push(final_line(world, "objlod.1", "NiNode", child(objlod, 1)));
    rows.compare_lines(&host, &["S world", "S objlod"], "final world");
}

fn world_run001(harness: &Harness, rows: &Rows) -> WorldRun {
    let run = WorldRun::new(harness, Options::default());
    check_water(&run, rows);
    let mut printer = WaterPrinter::new(&run);
    let _ = printer.tree(run.water_root());
    let first_material = printer.first_material.unwrap();
    check_tree_manager(&run, first_material, rows);
    check_final_world(&run, rows);
    // The final update leaves the water tree as the 00406649 dump.
    let mut after = WaterPrinter::new(&run);
    rows.compare_lines(
        &after.tree(run.water_root()),
        &WATER_TREE_PREFIXES,
        "water tree after the world update",
    );
    assert_eq!(
        run.log,
        ["Initializing TreeManager...", "Initializing Menus..."],
        "the two Startup_LogInfo lines"
    );
    run
}

/// Every line of `phase09.rows` read by a check, then `phase09_run001.rows` held against the evidence it came from, as `phase08_sky` does. One test, as the accounting spans them all.
#[test]
fn every_row_is_read() {
    let rows = Rows::load();
    tes_run001(&rows);
    let root = fixtures::data_dir()
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap();
    // The run-held rows, against the evidence.
    let bytes =
        std::fs::read(fixtures::root().join("evidence/phase09-run001-2026-10-06.records.txt"))
            .unwrap();
    let records: Vec<String> = bytes
        .split(|&byte| byte == b'\n')
        .map(|line| line.iter().map(|&byte| char::from(byte)).collect())
        .collect();
    let record = |number: &str| -> &str {
        records
            .iter()
            .find(|line| line.split(' ').nth(1) == Some(number))
            .unwrap_or_else(|| panic!("no record {number}"))
    };
    let run_rows = fixtures::rows("phase09_run001");
    assert_eq!(run_rows.len(), 8);
    let mut bs_file_row = String::new();
    let mut held_counters = Vec::new();
    for row in &run_rows {
        let parts: Vec<&str> = row.split('|').collect();
        assert_eq!(parts[0], "P09-RUN-001", "{row}");
        match parts[1] {
            "BSFILE" => {
                assert!(
                    record(parts[2]).contains(&format!("raw={}", parts[4])),
                    "{row}"
                );
                let identity: Vec<&str> = parts[3].split(',').collect();
                bs_file_row = format!("vt={} procs={},{}", identity[0], identity[1], identity[2]);
            }
            "WORKER" => {
                let line = record(parts[2]);
                let worker = line
                    .split(" | ")
                    .find(|part| part.contains(parts[3]))
                    .unwrap();
                assert!(worker.contains(&format!("rets=['{}'", parts[4])), "{row}");
            }
            "HITS" => assert!(record(parts[2]).contains(parts[3]), "{row}"),
            "COUNTERS" => {
                assert!(record(parts[2]).contains(parts[3]), "{row}");
                held_counters.push(parts[3].to_owned());
            }
            other => panic!("unknown row kind {other}"),
        }
    }
    // 712590's 43 hits are the registrations `check_havok` read; 7B8940's 1 hit is the prepare below.
    data_handler_run001(&rows, &root, &bs_file_row);
    let harness = Harness::new();
    let run = world_run001(&harness, &rows);
    assert_eq!(rows.unread(), Vec::<&str>::new(), "unread rows");
    // P09-RUN-001 counters: each step's advance. Absolute values depend on what ran before the window, which the harness does not reproduce in full.
    let parse = |text: &str| -> (u32, u32, u32) {
        let field = |index: usize| {
            u32::from_str_radix(
                text.split(' ')
                    .nth(index)
                    .unwrap()
                    .rsplit('=')
                    .next()
                    .unwrap(),
                16,
            )
            .unwrap()
        };
        (field(0), field(1), field(2))
    };
    let start = parse(&held_counters[0]);
    let after_water = parse(&held_counters[1]);
    let after_tree = parse(&held_counters[2]);
    let (geometry_at_window, effect_at_window, material_at_window) = run.counters_at_window;
    let (geometry, effect, material) = counters(&run.world);
    assert_eq!(
        u32::from(geometry - geometry_at_window),
        after_tree.0 - start.0,
        "word_B27504 advance"
    );
    assert_eq!(
        effect - effect_at_window,
        after_tree.1 - start.1,
        "00B259FC advance"
    );
    assert_eq!(
        material - material_at_window,
        after_tree.2 - start.2,
        "00B25AF4 advance"
    );
    assert_eq!(
        after_water.2 + 1,
        after_tree.2,
        "the TreeManager's one material"
    );
    eprintln!(
        "window start counters: host geom={geometry_at_window:X} effect={effect_at_window:X} material={material_at_window:X}, original {:X} {:X} {:X}",
        start.0, start.1, start.2
    );
}

// IDA: 00483350 / 00483320 uGridsToLoad normalization: 4 -> 5, 6 -> 7 (the INI value is written back); the exterior cell buffer grows to g(g+2)+1 and a zero interior buffer becomes 1.
#[test]
fn grid_normalization() {
    for (grids, want) in [(4, 5), (6, 7), (7, 7)] {
        let collections =
            static_collections(&[(0x00B0_6A2C, grids), (0x00B0_51D4, 0), (0x00B0_51DC, 10)]);
        let run = TesRun::new(collections, false, None);
        assert_eq!(
            run.setting_raw(0x00B0_6A2C),
            want,
            "uGridsToLoad written back"
        );
        let tes = run.tes();
        assert_eq!(
            tes.grid_cells_08.as_ref().unwrap().size_0c,
            want,
            "grid size"
        );
        assert_eq!(
            tes.grid_distant_04.as_ref().unwrap().size_0c,
            want + 2 * 25,
            "distant size"
        );
        assert_eq!(
            run.setting_raw(0x00B0_51DC),
            want * (want + 2) + 1,
            "exterior cell buffer grows"
        );
        assert_eq!(run.setting_raw(0x00B0_51D4), 1, "interior buffer >= 1");
        assert_eq!(
            run.globals().cell_cache_limit_b33a2c,
            want * want,
            "00B33A2C = g*g"
        );
    }
}

// IDA: 00441D13 bTintMipMaps on: 00B3F958 is not stored.
#[test]
fn tint_mip_maps() {
    let run = TesRun::new(static_collections(&[(0x00B0_5244, 1)]), false, None);
    assert_eq!(run.object_load_gate_calls, 0, "bTintMipMaps skips 00B3F958");
}

// IDA: 008896F0 all iSimType arms and 005350F0's "TES4" prefix copy.
#[test]
fn havok_arms() {
    for (sim_type, rate) in [(0, 1), (1, 4), (2, 2), (3, 5), (4, 9), (7, 4)] {
        assert_eq!(havok_sim_type_rate(sim_type), rate, "008896F0 arms");
    }
    let mut globals = HavokGlobals::default();
    let mut registry = NiFactoryRegistry::default();
    let mut inputs = HavokSettingInputs {
        current_directory: b"C:\\Games\\TES4\\Oblivion".to_vec(),
        ..HavokSettingInputs::default()
    };
    initialize_havok_system(&mut globals, &mut registry, &inputs);
    assert_eq!(
        globals.error.as_ref().unwrap().directory_20,
        b"C:\\Games\\",
        "TES4 prefix copy"
    );
    inputs.current_directory = b"C:\\games\\tes4\\x".to_vec();
    initialize_havok_system(&mut globals, &mut registry, &inputs);
    assert_eq!(
        globals.error.as_ref().unwrap().directory_20,
        b"C:\\games\\",
        "tes4 prefix copy"
    );
    assert_eq!(registry.entries().len(), 43, "0088B070 runs once");
}

// IDA: 00406609 a failed TES allocation publishes null without running the constructor.
#[test]
fn tes_allocation_failure() {
    let run = TesRun::new(static_collections(&[]), true, None);
    assert!(
        !run.allocated && run.globals().tes.is_none(),
        "00B333A0 null"
    );
    assert_eq!(run.data_handler_calls, 0, "no constructor");
}

// IDA: N/A fixture plugins in a temp Data directory (never the real Data).
fn record_bytes(
    kind: &[u8; 4],
    flags: u32,
    form_id: u32,
    chunks: &[(&[u8; 4], Vec<u8>)],
) -> Vec<u8> {
    let mut body = Vec::new();
    for (chunk, data) in chunks {
        body.extend_from_slice(*chunk);
        body.extend_from_slice(&(data.len() as u16).to_le_bytes());
        body.extend_from_slice(data);
    }
    let mut record = kind.to_vec();
    record.extend_from_slice(&(body.len() as u32).to_le_bytes());
    record.extend_from_slice(&flags.to_le_bytes());
    record.extend_from_slice(&form_id.to_le_bytes());
    record.extend_from_slice(&0_u32.to_le_bytes());
    record.extend_from_slice(&body);
    record
}

fn hedr(next_form_id: u32) -> Vec<u8> {
    let mut data = 1.0_f32.to_le_bytes().to_vec();
    data.extend_from_slice(&3_u32.to_le_bytes());
    data.extend_from_slice(&next_form_id.to_le_bytes());
    data
}

fn cstring(text: &str) -> Vec<u8> {
    let mut data = text.as_bytes().to_vec();
    data.push(0);
    data
}

fn write_plugin(path: &Path, mut bytes: Vec<u8>, seconds: u64, group: bool) {
    if group {
        // A GRUP after the TES4 record (the header read stops there: hdr = 1).
        bytes.extend_from_slice(b"GRUP");
        bytes.extend_from_slice(&0x14_u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 12]);
    }
    std::fs::write(path, bytes).unwrap();
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000 + seconds),
    )
    .unwrap();
}

// IDA: 0044A2B0 arms over fixtures: masters first by write time, the master-dependency reorder (A.esm's master B.esm moves ahead), a missing master (D.esp), an invalid header (Bad.esp warns and stays
// listed), an empty file (skipped) and a stale list entry whose file is gone (dropped by the first pass).
#[test]
fn file_list_fixtures() {
    let temp = std::env::temp_dir().join(format!("recomp_phase09_list_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    let data = temp.join("Data");
    std::fs::create_dir_all(&data).unwrap();
    let sizes = vec![0_u8; 8];
    write_plugin(
        &data.join("A.esm"),
        record_bytes(
            b"TES4",
            1,
            0,
            &[
                (b"HEDR", hedr(0x900)),
                (b"MAST", cstring("B.esm")),
                (b"DATA", sizes.clone()),
            ],
        ),
        100,
        true,
    );
    write_plugin(
        &data.join("B.esm"),
        record_bytes(
            b"TES4",
            1,
            0,
            &[(b"HEDR", hedr(0x10)), (b"CNAM", cstring("author"))],
        ),
        200,
        true,
    );
    write_plugin(
        &data.join("C.esp"),
        record_bytes(
            b"TES4",
            0,
            0,
            &[
                (b"HEDR", hedr(0x800)),
                (b"SNAM", cstring("desc")),
                (b"MAST", cstring("A.esm")),
                (b"DATA", sizes.clone()),
            ],
        ),
        50,
        true,
    );
    write_plugin(
        &data.join("D.esp"),
        record_bytes(
            b"TES4",
            0,
            0,
            &[
                (b"HEDR", hedr(0x800)),
                (b"MAST", cstring("Missing.esm")),
                (b"DATA", sizes),
            ],
        ),
        300,
        false,
    );
    write_plugin(&data.join("Empty.esp"), Vec::new(), 1, false);
    let mut bad = b"ABCD".to_vec();
    bad.extend_from_slice(&[b'x'; 30]);
    write_plugin(&data.join("Bad.esp"), bad, 10, false);

    let mut store = recomp_game::tes::tes_file::TesFileStore::default();
    // A stale entry for a file that no longer exists (the first pass's failed header read drops it).
    let stale = store.push(TesFileData {
        directory_120: "Data\\".to_owned(),
        name_1c: "Gone.esp".to_owned(),
        ..TesFileData::default()
    });
    let mut list: Vec<TesFileId> = vec![stale];
    let mut warnings = Vec::new();
    let mut warning = |text: String, _disabled: bool| warnings.push(text);
    let mut cache = recomp_game::tes::tes_file::FormTypeLookupCache::default();
    build_tes_file_list(
        &mut list,
        &mut store,
        "Data\\",
        0x2800,
        &mut TesFileServices {
            working_directory: &temp,
            warning: &mut warning,
            disable_warning_messages_b06b18: 0,
            close_suppressed: true,
            form_type_cache: &mut cache,
        },
    )
    .unwrap();
    let files: Vec<&TesFileData> = list.iter().map(|&id| store.get(id).unwrap()).collect();
    let order: Vec<&str> = files.iter().map(|file| file.name_1c.as_str()).collect();
    assert_eq!(
        order,
        ["B.esm", "A.esm", "Bad.esp", "C.esp", "D.esp"],
        "fixture order"
    );
    assert_eq!(
        warnings,
        ["File 'Bad.esp' is not a valid TES file."],
        "only Bad.esp warns"
    );
    let (b, a, bad, c, d) = (files[0], files[1], files[2], files[3], files[4]);
    assert_eq!(
        b.header_3d0[2], 0xFF00_0800,
        "B.esm next ID raised to 0x800"
    );
    assert_eq!(b.author_404.data_00.as_deref(), Some(&b"author"[..]));
    assert_eq!(b.author_404.length_04, 6);
    assert_eq!(b.flags_3dc, 3, "B.esm master flag");
    assert_eq!(b.header_read_28c, 1);
    assert!(a.master_files_allocated);
    assert_eq!(a.master_files_3f4, [Some(list[0])], "A.esm resolves B.esm");
    assert_eq!(c.master_files_3f4, [Some(list[1])], "C.esp resolves A.esm");
    assert_eq!(c.description_40c.data_00.as_deref(), Some(&b"desc"[..]));
    assert_eq!(c.flags_3dc & 1, 0);
    assert!(d.master_files_allocated);
    assert_eq!(
        d.master_files_3f4,
        [None],
        "D.esp keeps a null slot for Missing.esm"
    );
    assert_eq!(d.header_read_28c, 1, "no GRUP: end of file");
    assert!(
        !bad.master_files_allocated && bad.master_count_3f0 == 0 && bad.flags_3dc == 2,
        "Bad.esp has no masters"
    );
    assert_eq!(
        store.iter().count(),
        6,
        "Gone.esp stays owned (the original leaks it)"
    );
    drop(files);
    let _ = std::fs::remove_dir_all(&temp);
}

// IDA: 0043F699 / 0049DE13 / 0049E164 without the water shader: no prepare, every shape gets a NiTexturingProperty with the base texture (filter 2), one NiFlipController per record retargeted to the last
// shape's property (00715CE0 drops it from the earlier ones), uSurfaceFrameCount rewritten to the frames found and the "Water art assets" warning when fewer.
#[test]
fn water_without_shader() {
    let harness = Harness::new();
    let run = WorldRun::new(
        &harness,
        Options {
            water_shader: false,
            frames: Some(3),
            ..Options::default()
        },
    );
    let world = &run.world;
    assert!(run.water_shader_trace.is_empty(), "no WaterShader prepare");
    assert_eq!(run.raw(0x00B0_7088), 3, "0049DFD3 frames found");
    let missing = run
        .warnings
        .iter()
        .filter(|text| *text == "Water art assets don't match .ini settings.")
        .count();
    assert_eq!(
        missing, 1,
        "the first record warns (later records see 3 frames)"
    );
    // Frames 00..02 are created once (the palette serves the rest).
    assert_eq!(run.fixture.created.len(), 3, "three textures created");
    assert_eq!(run.fixture.created[0], b"Textures\\water\\water00.DDS");
    assert_eq!(run.fixture.created[2], b"Textures\\water\\water02.DDS");
    let interior = run.tes().water_node_58.as_ref().unwrap();
    let base = interior.texture_10.unwrap();
    assert_eq!(
        world
            .get::<SourceTexture>(base)
            .unwrap()
            .filename_34
            .as_deref(),
        Some(&b"Textures\\water\\water00.DDS"[..]),
        "+0x10 base texture"
    );
    let flip = interior.flip_controller_1c.unwrap();
    let frames = &world.get::<FlipController>(flip).unwrap().textures_40;
    assert_eq!(frames.len(), 3, "flip frames 0..2");
    assert_eq!(frames[0], Some(base));
    let state = world.get::<TextureController>(flip).unwrap();
    assert_eq!(
        state.time.fields.flags, 9,
        "flip flags (12 & !8 | 1) & !6 | 8"
    );
    let mut heads = 0;
    for (index, shape) in interior.shapes_08.iter().enumerate() {
        let list = &world.get::<NiAvProperties>(shape.unwrap()).unwrap().0;
        assert_eq!(list.len(), 1, "one texturing property per shape");
        let property = list[0].unwrap();
        let texturing = world.get::<TexturingProperty>(property).unwrap();
        assert_eq!(texturing.flags_18, 4);
        let map = texturing.maps_1c[0].as_ref().unwrap();
        assert_eq!(map.flags_04, 0x3200, "base map: filter mode 2");
        assert_eq!(map.texture_08, Some(base), "base map: texture");
        let head = world.get::<NiObjectNet>(property).unwrap().controller_0c;
        if head.is_some() {
            heads += 1;
        }
        if index + 1 == interior.shapes_08.len() {
            assert_eq!(
                state.time.target_30,
                Some(property),
                "the last shape's property holds the controller"
            );
            assert_eq!(head, Some(flip));
        }
    }
    assert_eq!(heads, 1, "00715CE0 leaves the controller on one property");
}

// IDA: 0043F6AB the prepare needs render path 2+: below it the shapes keep no type-4 property.
#[test]
fn water_low_render_path() {
    let harness = Harness::new();
    let run = WorldRun::new(
        &harness,
        Options {
            water_render_path: 1,
            ..Options::default()
        },
    );
    assert!(
        run.water_shader_trace.is_empty(),
        "no prepare below render path 2"
    );
    let shape = run.tes().water_node_58.as_ref().unwrap().shapes_08[0].unwrap();
    assert!(
        run.world
            .get::<NiAvProperties>(shape)
            .is_none_or(|list| list.0.is_empty())
            && run.world.get::<NiGeometryShader>(shape).is_none(),
        "no WaterShaderProperty"
    );
}

// IDA: 0055F77C an existing 00B39E04 with argument 0 is kept unchanged; 0055F783 with a nonzero argument it is destroyed (0055F550: 00B06A28 = 1) and rebuilt.
#[test]
fn tree_manager_arms() {
    let harness = Harness::new();
    let mut run = WorldRun::new(
        &harness,
        Options {
            existing_tree_manager: true,
            ..Options::default()
        },
    );
    {
        let g = run.world.resource::<TreeManagerGlobals>();
        assert!(
            g.tree_manager_b39e04.as_ref().unwrap().zbuffer_08.is_none()
                && g.wind_matrices_b42a10.count_00 == 0,
            "0055F781: an existing manager is kept (no constructor effects)"
        );
    }
    let mut grid_changed = 0;
    let marker = run.world.spawn_empty().id();
    run.world
        .resource_mut::<TreeManagerGlobals>()
        .pointer_b43108 = Some(marker);
    reset_tree_manager(
        &mut run.world,
        true,
        &run.collections.ini,
        &mut grid_changed,
    )
    .unwrap();
    let g = run.world.resource::<TreeManagerGlobals>();
    assert_eq!(grid_changed, 1);
    assert!(g.pointer_b43108.is_none());
    assert!(g.tree_manager_b39e04.as_ref().unwrap().zbuffer_08.is_some());
    assert_eq!(
        g.wind_matrices_b42a10.count_00, 4,
        "0055F785: destroyed (00B06A28 = 1, 00B43108 released) and rebuilt"
    );
}
