//! M0.6: the ECS rules from `docs/migration-plan.md`, proven on TESGlobal against the rows the
//! original produced (P12-RUN-001 / RUN-003 / RUN-005). The data load runs on the shipped files, through the real file pass since M4 folded the bounded reader away.

use std::time::Instant;

use bevy_ecs::prelude::*;
use oblivion_core::fixtures;
use oblivion_core::hash::{FNV1A_BASIS, fnv1a_dword};
use oblivion_sim::tes::data_handler::create_default_globals;
use oblivion_sim::tes::data_handler::*;
use oblivion_sim::tes::data_load::{DataLoadGlobals, DataLoadServices, LoadSelectedFiles};
#[cfg(test)]
use oblivion_sim::tes::form::set_form_id;
use oblivion_sim::tes::form::*;
use oblivion_sim::tes::form_arms::{AbsentArm, DataLoadHost};
use oblivion_sim::tes::game_settings::GameSettingCollection;
use oblivion_sim::tes::global::*;
use oblivion_sim::tes::tes::{GameThreadLocals, Tes, TesGlobals};

/// IDA: N/A the slice's host: only TESGlobal is wanted, and 0044AC20 is the six defaults.
struct GlobalsOnly;

impl DataLoadHost for GlobalsOnly {
    fn run_pre_load_pass(&mut self, world: &mut World) -> bool {
        create_default_globals(world).unwrap();
        false
    }
    fn run_post_load_passes(&mut self, _world: &mut World) -> bool {
        false
    }
    fn absent_arm(&mut self, _form_type: u32) -> AbsentArm {
        AbsentArm::CountAndSkip
    }
    fn arm_enabled(&mut self, form_type: u32) -> bool {
        form_type == u32::from(FORM_TYPE_GLOBAL)
    }
}

fn root() -> std::path::PathBuf {
    fixtures::data_dir()
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap()
}

// IDA: N/A a world at the 0044F3D0 window: the DataHandler, its file list with every file Loaded, a TES, the GMST store.
fn window_world() -> World {
    let mut world = World::new();
    fill_window(&mut world);
    world
}

fn fill_window(world: &mut World) {
    fill_indexes(world);
    construct_data_handler(world).unwrap();
    build_data_handler_file_list(world, "Data\\", &root(), &mut |text| panic!("{text}")).unwrap();
    let mut data = world.resource_mut::<DataHandler>();
    for id in data.files_8c8.clone() {
        data.file_store.get_mut(id).unwrap().set_loaded(true);
    }
    world.insert_resource(TesGlobals {
        tes: Some(Box::new(Tes::default())),
        ..TesGlobals::default()
    });
    world.init_resource::<GameSettingCollection>();
    world.init_resource::<DataLoadGlobals>();
}

fn new_world() -> World {
    let mut world = World::new();
    fill_indexes(&mut world);
    world
}

fn fill_indexes(world: &mut World) {
    insert_form_indexes(world);
    world.init_resource::<DataHandlerGlobals>();
    // 0044F3F9: the loader thread, which skips the form-ID arm of TESForm_Constructor.
    world.insert_resource(GameThreadLocals {
        byte_184: 1,
        ..GameThreadLocals::default()
    });
}

// IDA: N/A 0044F3D0(0, 0), a step at a time, as a host that draws between steps would.
fn run_load(world: &mut World) {
    let root = root();
    let mut warning = |text: String, _: bool| panic!("{text}");
    let mut host = GlobalsOnly;
    let mut services = DataLoadServices {
        working_directory: &root,
        warning: &mut warning,
        host: &mut host,
    };
    let mut cursor = LoadSelectedFiles::new(false, false);
    while cursor.step(world, &mut services).unwrap().is_none() {}
}

fn loaded_world() -> World {
    let mut world = window_world();
    run_load(&mut world);
    world
}

fn hex_bytes(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|at| u8::from_str_radix(&text[at * 2..at * 2 + 2], 16).unwrap())
        .collect()
}

/// The row printer: rebuilds the original's 0x28 bytes from components. `None` marks what is not
/// compared, the same fields the C++ test skips: heap garbage and raw pointers.
fn global_image(world: &World, form: Entity) -> Vec<Option<u8>> {
    let identity = world.get::<FormIdentity>(form).unwrap();
    let vtable = world.get::<OriginalVtable>(form).unwrap();
    let editor = world.get::<EditorId>(form).unwrap();
    let global = world.get::<TESGlobal>(form).unwrap();
    let mut image = vec![None; 0x28];
    let mut put = |at: usize, bytes: &[u8]| {
        for (offset, byte) in bytes.iter().enumerate() {
            image[at + offset] = Some(*byte);
        }
    };
    put(0x00, &vtable.0.to_le_bytes());
    put(0x04, &[identity.form_type_04]);
    put(0x08, &identity.flags_08.to_le_bytes());
    put(0x0C, &identity.form_id_0c.to_le_bytes());
    put(0x1C, &editor.0.length_04.to_le_bytes());
    put(0x1E, &editor.0.capacity_06.to_le_bytes());
    put(0x20, &[global.variable_type_20]);
    put(0x24, &global.value_bits_24.to_le_bytes());
    image
}

fn compare_rows(world: &World, kind: &str, rows: &[String]) -> usize {
    let forms = world.resource::<FormsById>();
    let mut compared = 0;
    for row in rows
        .iter()
        .filter(|row| row.starts_with(&format!("{kind} 04 ")))
    {
        let parts: Vec<_> = row.split(' ').collect();
        let id = u32::from_str_radix(parts[2], 16).unwrap();
        let form = forms
            .lookup(id)
            .unwrap_or_else(|| panic!("{kind} {id:X}: no form"));
        let recorded = hex_bytes(parts[3]);
        assert_eq!(recorded.len(), 0x28, "{kind} {id:X}");
        for (at, actual) in global_image(world, form).iter().enumerate() {
            if let Some(actual) = actual {
                assert_eq!(*actual, recorded[at], "{kind} {id:X} byte +{at:02X}");
            }
        }
        compared += 1;
    }
    compared
}

fn list_ids(world: &World) -> Vec<u32> {
    world
        .resource::<DataHandlerLists>()
        .globals_74
        .items()
        .map(|form| world.get::<FormIdentity>(form).unwrap().form_id_0c)
        .collect()
}

fn check_final_rows(world: &World, rows: &[String]) {
    // COUNT <type> <count> <fnv> <first> <last>: the ID-sorted walk of g_formsByID.
    let count = rows
        .iter()
        .find(|row| row.starts_with("COUNT 04 "))
        .unwrap();
    let parts: Vec<_> = count.split(' ').collect();
    let mut ids: Vec<u32> = world
        .resource::<FormsById>()
        .entries()
        .filter(|(_, form)| world.get::<FormIdentity>(*form).unwrap().form_type_04 == 4)
        .map(|(id, _)| id)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids.len(), parts[2].parse::<usize>().unwrap(), "COUNT");
    assert_eq!(
        ids.iter()
            .fold(FNV1A_BASIS, |hash, id| fnv1a_dword(hash, *id)),
        u32::from_str_radix(parts[3], 16).unwrap(),
        "COUNT fnv"
    );
    assert_eq!(ids[0], u32::from_str_radix(parts[4], 16).unwrap());
    assert_eq!(
        *ids.last().unwrap(),
        u32::from_str_radix(parts[5], 16).unwrap()
    );

    // LIST 74 <count> <fnv> <first three> <last>: the DataHandler list, in list order.
    let list = rows.iter().find(|row| row.starts_with("LIST 74 ")).unwrap();
    let parts: Vec<_> = list.split(' ').collect();
    let ordered = list_ids(world);
    assert_eq!(ordered.len(), parts[2].parse::<usize>().unwrap(), "LIST");
    assert_eq!(
        ordered
            .iter()
            .fold(FNV1A_BASIS, |hash, id| fnv1a_dword(hash, *id)),
        u32::from_str_radix(parts[3], 16).unwrap(),
        "LIST fnv: the original order"
    );
    let first: Vec<u32> = parts[4]
        .split(',')
        .map(|id| u32::from_str_radix(id, 16).unwrap())
        .collect();
    assert_eq!(ordered[..3], first[..]);
    assert_eq!(
        *ordered.last().unwrap(),
        u32::from_str_radix(parts[5], 16).unwrap()
    );

    assert_eq!(compare_rows(world, "SAMPLE", rows), 5);
    // +0x10: the rows hold raw TESFile pointers, so check who shares one. The Oblivion.esm forms
    // agree (f48aa703), and Knights' 9000CE7 differs (e03ca903).
    let forms = world.resource::<FormsById>();
    for (id, file) in [
        (0x35, 0),
        (0x1B037, 0),
        (0x6A7E7, 0),
        (0xB9853, 0),
        (0x900_0CE7, 9),
    ] {
        let references = world
            .get::<ModReferences>(forms.lookup(id).unwrap())
            .unwrap();
        assert_eq!(references.0[0], file, "{id:X}: +0x10");
    }
    // TESGlobal's InitItem is a no-op, so the post-load rows hold the same fields.
    assert_eq!(compare_rows(world, "POST", rows), 5);
}

/// The six 0044AC20 globals, before any record, against RUN-003's raw bytes.
#[test]
fn default_globals_match_the_original() {
    let rows = fixtures::rows("phase12");
    let mut world = new_world();
    create_default_globals(&mut world).unwrap();
    assert_eq!(compare_rows(&world, "AC20RAW", &rows), 6);
    let ac20 = rows.iter().find(|row| row.starts_with("AC20 04 ")).unwrap();
    let created: Vec<u32> = ac20
        .split(' ')
        .nth(2)
        .unwrap()
        .split(',')
        .map(|id| u32::from_str_radix(id, 16).unwrap())
        .collect();
    let mut oldest_first = list_ids(&world);
    oldest_first.reverse();
    assert_eq!(oldest_first, created);
}

/// All 95 forms load from the 11 shipped files into entities, in the original's list order, a record at a time.
#[test]
fn loaded_globals_match_every_recorded_row() {
    let rows = fixtures::rows("phase12");
    check_final_rows(&loaded_world(), &rows);
}

/// The rows must not depend on Bevy's storage: other entities and a different spawn layout
/// reorder queries and change every `Entity`, and the rows still match through the indexes.
#[test]
fn rows_do_not_depend_on_entity_values_or_query_order() {
    let rows = fixtures::rows("phase12");
    let mut world = window_world();
    for _ in 0..1000 {
        world.spawn(FormIdentity {
            form_type_04: 0x7F,
            flags_08: 0,
            form_id_0c: 0,
        });
    }
    let noise: Vec<Entity> = world
        .query_filtered::<Entity, With<FormIdentity>>()
        .iter(&world)
        .filter(|entity| world.get::<FormIdentity>(*entity).unwrap().form_type_04 == 0x7F)
        .collect();
    for entity in noise.iter().step_by(2) {
        world.despawn(*entity);
    }
    run_load(&mut world);
    check_final_rows(&world, &rows);
}

/// Ordered blocks run as exclusive systems in an explicit chain, through a real Bevy schedule.
#[test]
fn the_slice_runs_in_a_bevy_schedule() {
    fn phase_window(world: &mut World) {
        fill_window(world);
    }
    fn phase_load(world: &mut World) {
        run_load(world);
    }
    let mut world = World::new();
    let mut schedule = Schedule::default();
    schedule.add_systems((phase_window, phase_load).chain());
    schedule.run(&mut world);
    check_final_rows(&world, &fixtures::rows("phase12"));
    assert_eq!(
        world.resource::<EditorIdMap>().lookup(b"gameyear"),
        world.resource::<FormsById>().lookup(0x35),
        "a case-insensitive name lookup"
    );
}

/// The cost check for 103,510 forms (P12-RUN-001), not a parity test. Run it with
/// `--ignored --nocapture`. Each form gets six components and goes through the same indexed path.
#[test]
#[ignore = "measurement only"]
fn measure_form_count_cost() {
    let mut world = new_world();
    let start = Instant::now();
    for index in 0..103_510u32 {
        let form = construct_global(&mut world).unwrap();
        set_form_id(&mut world, form, 0x1000 + index);
        world
            .resource_mut::<DataHandlerLists>()
            .globals_74
            .prepend_non_null(form);
        world
            .resource_mut::<EditorIdMap>()
            .insert_or_assign(&index.to_le_bytes(), form);
    }
    let spawned = start.elapsed();
    let start = Instant::now();
    let walked = list_ids(&world)
        .iter()
        .fold(FNV1A_BASIS, |hash, id| fnv1a_dword(hash, *id));
    let walk = start.elapsed();
    let start = Instant::now();
    let found = (0..103_510u32)
        .filter(|index| {
            world
                .resource::<FormsById>()
                .lookup(0x1000 + index)
                .is_some()
        })
        .count();
    let lookup = start.elapsed();
    assert_eq!(found, 103_510);
    println!(
        "103510 forms: spawn+index {spawned:?}, ordered walk {walk:?} ({walked:08X}), lookups {lookup:?}"
    );
}
