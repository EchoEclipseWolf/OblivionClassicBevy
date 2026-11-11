//! Phase 13a to 13d: the arms the shipped data misses, each on a small fixture. `game_data_window.rs` holds the shipped run and its rows. These stand on the exe alone, so a pass earns nothing against the original.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::thread::ThreadId;

use bevy_ecs::prelude::*;
use common::load::{Loader, install_player};
use oblivion_core::error::GameError;
use oblivion_sim::bs::model_loader::model_map_bucket;
use oblivion_sim::bs::task_manager::TaskGlobals;
use oblivion_sim::tes::actor::TesObjectRefr;
use oblivion_sim::tes::base_extra_list::{ExtraData, ExtraDataList, new_extra_entry};
use oblivion_sim::tes::data_handler::{DataHandlerLists, FormsById, init_scripts_and_cells};
use oblivion_sim::tes::data_load::DataLoadServices;
use oblivion_sim::tes::form::FormIdentity;
use oblivion_sim::tes::form_components::{
    TesActorBaseData, TesAttributes, TesHealthForm, TesModel,
};
use oblivion_sim::tes::forms_init_world::{cell_run_reference_scripts, reference_run_scripts};
use oblivion_sim::tes::forms_script::Script;
use oblivion_sim::tes::forms_world::{CellRegionLocks, TesObjectCell};
use oblivion_sim::tes::game_settings::GameSettingCollection;
use oblivion_sim::tes::inventory::{
    ContainerChanges, InventoryEntry, InventoryGlobals, ensure_container_changes,
    put_container_changes, run_container_item_scripts,
};
use oblivion_sim::tes::player::{PlayerCharacter, PlayerGlobals, player_get_actor_value};
use oblivion_sim::tes::tes::{
    FLAME_NODE_FORM_IDS, GameThreadLocals, TesGlobals, TesPreloadServices,
};
use oblivion_sim::tes::water::{WaterServices, WaterTextureServices};
use oblivion_sim::tes::winmain::initialize_game_data::{
    run_player_base, run_player_init, run_tes_preload,
};

fn u32_bytes(value: u32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn f32_bytes(value: f32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    cat(&[kind, &(data.len() as u16).to_le_bytes(), data])
}

fn text(value: &str) -> Vec<u8> {
    let mut bytes = value.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

fn record(kind: &[u8; 4], flags: u32, id: u32, payload: &[u8]) -> Vec<u8> {
    cat(&[
        kind,
        &u32_bytes(payload.len() as u32),
        &u32_bytes(flags),
        &u32_bytes(id),
        &u32_bytes(0),
        payload,
    ])
}

fn tes4() -> Vec<u8> {
    record(
        b"TES4",
        1,
        0,
        &chunk(
            b"HEDR",
            &cat(&[&f32_bytes(0.8), &u32_bytes(100), &u32_bytes(0x800)]),
        ),
    )
}

// A SCPT: compiled size 8 runs through the boundary, 4 answers 0 first.
fn script_record(id: u32, compiled: u32, script_type: u32) -> Vec<u8> {
    let header = cat(&[
        &u32_bytes(0),
        &u32_bytes(0),
        &u32_bytes(compiled),
        &u32_bytes(0),
        &u32_bytes(script_type),
    ]);
    record(
        b"SCPT",
        0,
        id,
        &cat(&[&chunk(b"SCHR", &header), &chunk(b"SCDA", &[0; 8])]),
    )
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("recomp_phase13_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Data")).unwrap();
        Self { root }
    }

    fn run(&self, records: &[Vec<u8>]) -> Loader {
        let mut esm = tes4();
        for one in records {
            esm.extend_from_slice(one);
        }
        std::fs::write(self.root.join("Data/Base.esm"), &esm).unwrap();
        let mut loader = Loader::new(&self.root);
        loader.load_all();
        install_player(&mut loader.world);
        loader.host.real_pre_load = true;
        loader.host.real_tail = true;
        assert_eq!(loader.run(), Ok(1));
        loader
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn form(world: &World, id: u32) -> Entity {
    world
        .resource::<FormsById>()
        .lookup(id)
        .unwrap_or_else(|| panic!("no form {id:08X}"))
}

/// IDA: N/A a scripted 00442890: a palette map and a call log.
struct Textures {
    palette: HashMap<Vec<u8>, Entity>,
    created: Vec<Vec<u8>>,
    create_fails: bool,
    io_thread: ThreadId,
}

impl Textures {
    fn new(io_thread: ThreadId) -> Self {
        Self {
            palette: HashMap::new(),
            created: Vec::new(),
            create_fails: false,
            io_thread,
        }
    }
}

impl WaterTextureServices for Textures {
    fn palette_find(&mut self, _: &mut World, path: &[u8]) -> Option<Entity> {
        self.palette.get(path).copied()
    }
    fn palette_add(&mut self, _: &mut World, path: &[u8], texture: Entity) {
        self.palette.insert(path.to_vec(), texture);
    }
    fn find_file(&mut self, _path: &[u8], _flags: u32) -> Option<(u32, Vec<u8>)> {
        None
    }
    fn create_from_file(&mut self, world: &mut World, path: &[u8]) -> Option<Entity> {
        self.created.push(path.to_vec());
        (!self.create_fails).then(|| world.spawn_empty().id())
    }
    fn io_thread_id(&self) -> ThreadId {
        self.io_thread
    }
}

fn install_tasks(world: &mut World) {
    let mut tasks = TaskGlobals::default();
    tasks.construct_io_manager().unwrap();
    tasks.construct_model_loader().unwrap();
    world.insert_resource(tasks);
}

fn preload(world: &mut World, textures: &mut Textures) -> (Result<(), GameError>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut warning = |text: String| warnings.push(text);
    let result = run_tes_preload(
        world,
        &mut TesPreloadServices {
            archives: None,
            textures: WaterServices {
                shader: None,
                image_space_effects: 0,
                render_path: 0,
                prepare: None,
                textures: Some(textures),
                warning: &mut warning,
            },
        },
    );
    (result, warnings)
}

fn queued_paths(world: &World) -> Vec<Vec<u8>> {
    let io = world
        .resource::<TaskGlobals>()
        .io_manager_b33a10
        .as_ref()
        .unwrap();
    io.tasks
        .store
        .iter()
        .flatten()
        .map(|task| task.path_20.clone())
        .collect()
}

// 0044358F: `cmp byte ptr [eax], 0` per GMST, so an empty value skips only itself. The FlameNode loop stores all 21 slots, with no form or a form with no path.
#[test]
fn a_preload_skips_empty_values_and_absent_forms() {
    let fixture = Fixture::new("empties");
    let mut loader = fixture.run(&[]);
    let world = &mut loader.world;
    install_tasks(world);
    {
        let mut settings = world.resource_mut::<GameSettingCollection>();
        settings
            .setting_at_mut(0x00B3_7158)
            .unwrap()
            .set_string(b"");
        settings
            .setting_at_mut(0x00B3_71D0)
            .unwrap()
            .set_string(b"");
    }
    let mut textures = Textures::new(std::thread::current().id());
    let (result, warnings) = preload(world, &mut textures);
    result.unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    // The defaults hold the GMST image's values, 16 less the one emptied, and the default forms' models are empty until a file names them.
    let paths = queued_paths(world);
    assert_eq!(paths.len(), 15);
    assert!(!paths.iter().any(|path| path.is_empty()));
    assert_eq!(
        textures.created.len(),
        2,
        "+0x98's name was empty: {:?}",
        textures.created
    );
    let globals = world.resource::<TesGlobals>();
    let tes = globals.tes.as_ref().unwrap();
    assert!(tes.blood_decals_94[0].is_some());
    assert!(tes.blood_decals_94[1].is_none());
    assert!(tes.blood_decals_94[2].is_some());
    assert_eq!(globals.flame_node_slots_b35e50, [0; 21]);
    assert_eq!(
        globals
            .store_trace
            .iter()
            .filter(|store| (0x00B3_5E50..0x00B3_5EA4).contains(&store.address))
            .count(),
        21
    );
    let tasks = world.resource::<TaskGlobals>();
    assert_eq!(
        tasks.counters.unpacked_sequence_b33a18, 15,
        "no archives: 004365F0"
    );
    tasks.io_manager_b33a10.as_ref().unwrap();
    world.resource_mut::<TaskGlobals>().stop_worker_threads();
}

// 00443BC4: the word at +0x2C decides, 0xFFFF taking the C string's length.
#[test]
fn a_flame_node_reads_its_model_length() {
    let fixture = Fixture::new("flames");
    let mut loader = fixture.run(&[
        record(
            b"STAT",
            0,
            FLAME_NODE_FORM_IDS[0],
            &chunk(b"MODL", &text("Fire\\one.nif")),
        ),
        record(
            b"STAT",
            0,
            FLAME_NODE_FORM_IDS[1],
            &chunk(b"MODL", &text("Fire\\two.nif")),
        ),
        record(b"STAT", 0, FLAME_NODE_FORM_IDS[2], &[]),
    ]);
    let world = &mut loader.world;
    install_tasks(world);
    {
        let two = form(world, FLAME_NODE_FORM_IDS[1]);
        world.get_mut::<TesModel>(two).unwrap().path_04.length_04 = 0xFFFF;
        let mut settings = world.resource_mut::<GameSettingCollection>();
        for address in [
            0x00B3_7150_u32,
            0x00B3_7158,
            0x00B3_7160,
            0x00B3_7168,
            0x00B3_7170,
            0x00B3_7178,
            0x00B3_7180,
            0x00B3_7188,
            0x00B3_7190,
            0x00B3_7198,
            0x00B3_71A0,
            0x00B3_71A8,
            0x00B3_71B0,
            0x00B3_71B8,
            0x00B3_71C0,
            0x00B3_7B38,
            0x00B3_71C8,
            0x00B3_71D0,
            0x00B3_71D8,
        ] {
            settings.setting_at_mut(address).unwrap().set_string(b"");
        }
    }
    let mut textures = Textures::new(std::thread::current().id());
    preload(world, &mut textures).0.unwrap();
    let paths = queued_paths(world);
    assert!(paths.contains(&b"Fire\\one.nif".to_vec()));
    assert!(paths.contains(&b"Fire\\two.nif".to_vec()));
    assert_eq!(paths.len(), 2, "{paths:?}");
    // Requests go out with 5, 0, 0, 0, 1, 1: flags 3, and with no archive 004365F0's 0x200000000 makes byte 7 a 2, so bucket 5 * 3 + 2.
    let io = world
        .resource::<TaskGlobals>()
        .io_manager_b33a10
        .as_ref()
        .unwrap();
    for task in io.tasks.store.iter().flatten() {
        assert_eq!(task.flags_34, 3);
        assert_eq!((task.key_10 >> 16) as u8, 5);
        assert_eq!(task.references_08, 1, "the map's, after the out release");
    }
    assert_eq!(io.array_2c[17], 2);
    world.resource_mut::<TaskGlobals>().stop_worker_threads();
}

// 00443BC4: no type test in the exe, so a form with no model at +0x24 is a host stop.
#[test]
fn a_flame_node_form_with_no_model_stops() {
    let fixture = Fixture::new("noflame");
    let mut loader = fixture.run(&[]);
    let world = &mut loader.world;
    install_tasks(world);
    let flame = form(world, FLAME_NODE_FORM_IDS[0]);
    world.entity_mut(flame).remove::<TesModel>();
    let mut textures = Textures::new(std::thread::current().id());
    assert_eq!(
        preload(world, &mut textures).0,
        Err(GameError::unclassified(
            0x00443BC4,
            "00443BC4: a FlameNode form with no TESModel at +0x24"
        ))
    );
    world.resource_mut::<TaskGlobals>().stop_worker_threads();
}

// 00442966: off g_IOManager +0x30 each creation pauses and resumes both workers, and the palette ends a second request's creation. A failure warns, with quiet 0, and stores null.
#[test]
fn a_decal_pauses_the_workers_and_warns_on_failure() {
    let fixture = Fixture::new("pause");
    let mut loader = fixture.run(&[]);
    let world = &mut loader.world;
    install_tasks(world);
    let io_thread = world
        .resource::<TaskGlobals>()
        .io_manager_b33a10
        .as_ref()
        .unwrap()
        .main_thread_id_30;
    let mut textures = Textures::new(io_thread);
    textures.create_fails = true;
    let (result, warnings) = preload(world, &mut textures);
    result.unwrap();
    assert_eq!(warnings.len(), 3);
    assert_eq!(
        warnings[0],
        "TES::CreateTextureImage unable to create image for \"Data\\Textures\\Effects\\blooddecal.dds\"."
    );
    assert_eq!(textures.created.len(), 3);
    let tasks = world.resource::<TaskGlobals>();
    let io = tasks.io_manager_b33a10.as_ref().unwrap();
    assert_eq!((io.field_1c, io.field_20), (0, 0), "balanced");
    assert_eq!(
        io.threads_28[0].state().counter_18.load(Ordering::Acquire),
        1
    );
    let tes = world.resource::<TesGlobals>();
    assert_eq!(tes.tes.as_ref().unwrap().blood_decals_94, [None; 3]);
    // A second preload hits the palette only for what it holds: a failed creation added nothing.
    let mut textures = Textures::new(std::thread::current().id());
    textures.palette.insert(
        b"Data\\Textures\\Effects\\blooddecal.dds".to_vec(),
        Entity::PLACEHOLDER,
    );
    let (result, _) = preload(world, &mut textures);
    result.unwrap();
    assert_eq!(textures.created.len(), 2, "the palette answered the first");
    let tes = world.resource::<TesGlobals>();
    assert_eq!(
        tes.tes.as_ref().unwrap().blood_decals_94[0],
        Some(Entity::PLACEHOLDER)
    );
    world.resource_mut::<TaskGlobals>().stop_worker_threads();
}

// 004067FD: ___RTDynamicCast: no form at 7, or a form that isn't a TESNPC, stores null at +0x1C and carries on.
#[test]
fn a_player_base_casts_to_null() {
    let fixture = Fixture::new("base");
    let mut loader = fixture.run(&[]);
    let world = &mut loader.world;
    let npc = form(world, 7);
    world.resource_mut::<FormsById>().remove(7);
    let mut logs = Vec::new();
    let base = run_player_base(world, &mut |text| logs.push(text)).unwrap();
    assert_eq!(base, None);
    assert_eq!(logs, ["Initializing Player..."]);
    let player = world.resource::<PlayerGlobals>().the_player_b333c4.unwrap();
    let refr = world.get::<TesObjectRefr>(player).unwrap();
    assert_eq!(refr.base_form_1c, None);
    assert_eq!(refr.position_2c, [2048.0, 2048.0, 0.0]);
    assert_eq!(refr.rotation_20, [0.0; 3]);
    // 0065E083: a null base form answers 0 for any but 11.
    assert_eq!(player_get_actor_value(world, player, 10), Ok(0));
    assert_eq!(player_get_actor_value(world, player, 3), Ok(0));
    // 0065E05E: 11 doesn't test the base.
    assert!(player_get_actor_value(world, player, 11).is_err());
    assert!(player_get_actor_value(world, player, 72).is_err());
    world.resource_mut::<FormsById>().set_at(7, npc);
    let base = run_player_base(world, &mut |_| {}).unwrap();
    assert_eq!(base, Some(npc));
    assert_eq!(
        world.get::<TesObjectRefr>(player).unwrap().base_form_1c,
        Some(npc)
    );
}

fn player_and_npc(world: &mut World, attributes: [u8; 8], health: u32, fatigue: u16) -> Entity {
    run_player_base(world, &mut |_| {}).unwrap();
    let player = world.resource::<PlayerGlobals>().the_player_b333c4.unwrap();
    let npc = form(world, 7);
    world.get_mut::<TesAttributes>(npc).unwrap().values_04 = attributes;
    world.get_mut::<TesHealthForm>(npc).unwrap().health_04 = health;
    world.get_mut::<TesActorBaseData>(npc).unwrap().fatigue_0a = fatigue;
    player
}

// 0065E030 and its chain, the groupings at 24 bits.
#[test]
fn the_actor_values_add_in_the_exes_order() {
    let fixture = Fixture::new("values");
    let mut loader = fixture.run(&[]);
    let world = &mut loader.world;
    // Strength 30, Willpower 20, Agility 10, Endurance 40 at 0, 2, 3, 5.
    let player = player_and_npc(world, [30, 1, 20, 10, 2, 40, 3, 4], 7, 5);
    // A direct attribute: 0 + max + script + damage, truncated.
    {
        let mut body = world.get_mut::<PlayerCharacter>(player).unwrap();
        body.max_av_modifiers_204[1] = 2.75;
        body.script_av_modifiers_324[1] = 0.5;
        body.damage_av_modifiers_450[1] = -1.5;
    }
    assert_eq!(
        player_get_actor_value(world, player, 1),
        Ok(2),
        "1 + 2.75 + 0.5 - 1.5"
    );
    // Fatigue: base 5 + 30 + 40 + 10 + 20, and the +0x44C damage slot, not +0x450 + 40.
    {
        let mut body = world.get_mut::<PlayerCharacter>(player).unwrap();
        body.stamina_44c = -6.5;
        body.damage_av_modifiers_450[10] = 100.0;
    }
    assert_eq!(
        player_get_actor_value(world, player, 10),
        Ok(98),
        "105 - 6.5"
    );
    // Health: base 7 + ftol(40 * fPCBaseHealthMult), and +0x444.
    let multiplier = world
        .resource_mut::<GameSettingCollection>()
        .setting_at_mut(0x00B3_7720)
        .map(|setting| {
            setting.raw = 2.5_f32.to_bits();
        });
    assert!(multiplier.is_some());
    {
        let mut body = world.get_mut::<PlayerCharacter>(player).unwrap();
        body.health_444 = 0.25;
    }
    assert_eq!(player_get_actor_value(world, player, 8), Ok(107));
    // A negative total truncates toward zero; the floor doesn't reach it.
    {
        let mut body = world.get_mut::<PlayerCharacter>(player).unwrap();
        body.health_444 = -200.75;
    }
    assert_eq!(
        player_get_actor_value(world, player, 8),
        Ok(-93),
        "107 - 200.75"
    );
    // The arms the checks don't read stop.
    assert!(player_get_actor_value(world, player, 37).is_err());
    // The skills' arm (12, Armorer) went in with Build 1's Stats menu (005232D7).
    assert!(player_get_actor_value(world, player, 12).is_ok());
    // Magicka's arm went in with Phase 17's HUD: `the_port_matches_the_phase_17_rows` has its ratio.
    assert!(player_get_actor_value(world, player, 9).is_ok());
}

fn init(
    loader: &mut Loader,
    base: Option<Entity>,
) -> (
    Result<oblivion_sim::tes::winmain::initialize_game_data::PlayerChecks, GameError>,
    Vec<String>,
) {
    let working = loader.working_directory.clone();
    let mut warnings = Vec::new();
    let result = run_player_init(
        &mut loader.world,
        base,
        &mut DataLoadServices {
            working_directory: &working,
            warning: &mut |text, _| warnings.push(text),
            host: &mut loader.host,
        },
    );
    (result, warnings)
}

// 00406885, 004068B4: each zero logs through 00404EC0 with the first mod file's name, Fatigue first.
#[test]
fn a_zero_value_logs_the_first_file() {
    let fixture = Fixture::new("zero");
    let mut loader = fixture.run(&[record(b"NPC_", 0, 7, &[])]);
    let player = player_and_npc(&mut loader.world, [0; 8], 0, 0);
    let base = Some(form(&loader.world, 7));
    let (result, _) = init(&mut loader, base);
    let checks = result.unwrap();
    assert_eq!((checks.fatigue, checks.health), (0, 0));
    assert_eq!(
        loader.host.logs,
        [
            "ERROR: Fatigue value is 0 on the Player. Fix 'Base.esm' with the editor.",
            "ERROR: Health value is 0 on the Player. Fix 'Base.esm' with the editor.",
        ]
    );
    // InitItem's own arm: the flag ends set, 0x200000 clear.
    let flags = loader.world.get::<FormIdentity>(player).unwrap().flags_08;
    assert_ne!(flags & 8, 0);
    assert_eq!(flags & 0x20_0000, 0);
}

// 00406891: esi's +0x10 is empty, or esi is null: the original reads a bad address.
#[test]
fn a_zero_value_with_no_file_is_a_host_stop() {
    let fixture = Fixture::new("zero_nofile");
    let mut loader = fixture.run(&[]);
    player_and_npc(&mut loader.world, [0; 8], 0, 0);
    let (result, _) = init(&mut loader, None);
    assert_eq!(
        result,
        Err(GameError::unclassified(
            0x0040688C,
            "0040688C: 0046B680 on a null base form"
        ))
    );
    let base = Some(form(&loader.world, 7));
    let (result, _) = init(&mut loader, base);
    assert_eq!(
        result,
        Err(GameError::unclassified(
            0x00406891,
            "00406891: +0x1C of no mod file"
        ))
    );
}

fn interior_with(records: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut all = vec![record(b"CELL", 0, 0x901, &chunk(b"DATA", &[1]))];
    all.extend_from_slice(records);
    all
}

fn refr(id: u32, base: u32) -> Vec<u8> {
    let mut data = Vec::new();
    for value in [1.0_f32, 2.0, 3.0, 0.0, 0.0, 0.0] {
        data.extend(f32_bytes(value));
    }
    record(
        b"REFR",
        0,
        id,
        &cat(&[&chunk(b"NAME", &u32_bytes(base)), &chunk(b"DATA", &data)]),
    )
}

fn stack_with(script: Entity) -> ExtraDataList {
    let mut stack = ExtraDataList::default();
    stack.add(new_extra_entry(
        0x12,
        ExtraData::Script {
            script_0c: script,
            events_10: None,
        },
    ));
    stack
}

// 0048E840..0048E918: no script or an uninitialized one goes on to the next stack. A form of another type compares its counts first, and with no run the stack count reads 0, so the entry ends there.
#[test]
fn the_container_pass_walks_its_stacks_by_the_exes_rules() {
    let fixture = Fixture::new("stacks");
    let mut loader = fixture.run(&interior_with(&[
        script_record(0x910, 8, 0),
        script_record(0x911, 8, 0),
        record(b"CONT", 0, 0x920, &[]),
        record(b"MISC", 0, 0x921, &[]),
        refr(0x930, 0x920),
    ]));
    let world = &mut loader.world;
    let (run, other, item, reference, cell) = (
        form(world, 0x910),
        form(world, 0x911),
        form(world, 0x921),
        form(world, 0x930),
        form(world, 0x901),
    );
    // The stage of InitItem a script reaches at 004FBB60 leaves it initialized, and one form stands for "not initialized".
    world.get_mut::<FormIdentity>(run).unwrap().flags_08 |= 8;
    world.get_mut::<FormIdentity>(other).unwrap().flags_08 &= !8;
    // These stand for compiled scripts: opcode 30 ends the run at once.
    for script in [run, other] {
        world
            .get_mut::<oblivion_sim::tes::forms_script::Script>(script)
            .unwrap()
            .data_30 = Some(vec![30, 0, 0, 0, 0]);
    }
    world
        .get_mut::<TesObjectRefr>(reference)
        .unwrap()
        .parent_cell_40 = Some(cell);
    let mut changes = ContainerChanges {
        entries_00: vec![InventoryEntry {
            lists_00: Some(vec![
                ExtraDataList::default(),
                stack_with(other),
                stack_with(run),
                stack_with(run),
            ]),
            count_04: 4,
            form_08: item,
        }],
        owner_04: reference,
        float_08: -1.0,
        float_0c: -1.0,
    };
    let temporary_before = {
        // 00485D9B: the first creation made 00B34428.
        let changes = ensure_container_changes(world, reference).unwrap();
        put_container_changes(world, reference, changes);
        world
            .resource::<InventoryGlobals>()
            .temporary_reference_b34428
            .unwrap()
    };
    let runs = |world: &World| {
        let globals = world.resource::<InventoryGlobals>();
        (
            globals.container_script_passes,
            globals.container_item_script_runs,
        )
    };
    let (passes, items) = runs(world);
    assert_eq!(
        run_container_item_scripts(world, &mut changes, Some(reference)),
        Ok(false)
    );
    assert_eq!(runs(world), (passes + 1, items + 2), "the initialized two");
    let temporary = world.get::<TesObjectRefr>(temporary_before).unwrap();
    assert_eq!(temporary.parent_cell_40, None, "vt+0x194(0)");
    assert_eq!(temporary.base_form_1c, None);
    assert_eq!(temporary.position_2c, [1.0, 2.0, 3.0], "004D8A30 stays");
    // A script form of another type: `ebx` is still 0 against four stacks, so nothing after it runs.
    world.get_mut::<FormIdentity>(other).unwrap().flags_08 |= 8;
    world.get_mut::<FormIdentity>(other).unwrap().form_type_04 = 0x0E;
    let (passes, items) = runs(world);
    run_container_item_scripts(world, &mut changes, Some(reference)).unwrap();
    assert_eq!(runs(world), (passes + 1, items), "0048E908 ends the entry");
    // A null reference, and a reference with no cell, return 0 before any run.
    let (passes, items) = runs(world);
    run_container_item_scripts(world, &mut changes, None).unwrap();
    world
        .get_mut::<TesObjectRefr>(reference)
        .unwrap()
        .parent_cell_40 = None;
    world.get_mut::<FormIdentity>(other).unwrap().form_type_04 = 0x0D;
    run_container_item_scripts(world, &mut changes, Some(reference)).unwrap();
    assert_eq!(runs(world), (passes + 2, items));
    changes.entries_00.clear();
}

// 004D719E, 004D71C4: a deleted reference runs nothing; a container's pass comes before its own script. 004D7228: an event list's vt+0x40 is reached only past a zero answer.
#[test]
fn a_reference_runs_its_scripts_in_order() {
    let fixture = Fixture::new("reference");
    let mut loader = fixture.run(&interior_with(&[
        script_record(0x910, 8, 0),
        record(b"CONT", 0, 0x920, &[]),
        record(b"STAT", 0, 0x921, &[]),
        refr(0x930, 0x920),
        refr(0x931, 0x921),
    ]));
    let world = &mut loader.world;
    let (script, container, plain) = (form(world, 0x910), form(world, 0x930), form(world, 0x931));
    let counts = |world: &World| {
        let globals = world.resource::<InventoryGlobals>();
        (
            globals.container_script_passes,
            globals.reference_script_runs,
        )
    };
    let (passes, runs) = counts(world);
    world.get_mut::<FormIdentity>(container).unwrap().flags_08 |= 0x20;
    assert_eq!(reference_run_scripts(world, container), Ok(false));
    assert_eq!(counts(world), (passes, runs), "deleted");
    world.get_mut::<FormIdentity>(container).unwrap().flags_08 &= !0x20;
    assert_eq!(reference_run_scripts(world, container), Ok(false));
    assert_eq!(
        counts(world),
        (passes + 1, runs),
        "the pass, and no own script"
    );
    assert_eq!(reference_run_scripts(world, plain), Ok(false));
    assert_eq!(counts(world), (passes + 1, runs), "neither");
    // The own script, with no events: no vt+0x40.
    let set_script = |world: &mut World, events| {
        let mut body = world.get_mut::<TesObjectRefr>(plain).unwrap();
        body.extra_data_44.remove_by_type(0x12);
        body.extra_data_44.add(new_extra_entry(
            0x12,
            ExtraData::Script {
                script_0c: script,
                events_10: events,
            },
        ));
    };
    set_script(world, None);
    // TLS +0x185 open: a change record shows whether vt+0x40 ran.
    world.resource_mut::<GameThreadLocals>().track_changes_185 = 1;
    let id = world.get::<FormIdentity>(plain).unwrap().form_id_0c;
    let flags = |world: &World| {
        world
            .resource::<TesGlobals>()
            .save_load_game
            .as_ref()
            .unwrap()
            .changes_00
            .get(id)
            .map(|record| record.flags_00)
    };
    assert_eq!(reference_run_scripts(world, plain), Ok(false));
    assert_eq!(counts(world), (passes + 1, runs + 1));
    assert_eq!(flags(world), None, "no ScriptEventList, no vt+0x40");
    let events = oblivion_sim::tes::forms_script::create_script_event_list(world, script);
    set_script(world, Some(events));
    assert_eq!(reference_run_scripts(world, plain), Ok(false));
    assert_eq!(counts(world), (passes + 1, runs + 2));
    assert_eq!(flags(world), Some(0x400_0000), "vt+0x40 ran");
    world.resource_mut::<GameThreadLocals>().track_changes_185 = 0;
    // 004D71AF: a null base faults.
    world.get_mut::<TesObjectRefr>(plain).unwrap().base_form_1c = None;
    assert!(reference_run_scripts(world, plain).is_err());
}

// 004CB8C0: an actor skips with a3 set and runs without; a2 = 0 skips references with no 3D; the lock balances. 00447D80's script pass runs vt+0x6C over +0x64.
#[test]
fn a_cell_pass_skips_actors_and_balances_its_lock() {
    let fixture = Fixture::new("cell");
    let mut loader = fixture.run(&interior_with(&[
        script_record(0x910, 8, 0),
        record(b"CONT", 0, 0x920, &[]),
        refr(0x930, 0x920),
    ]));
    let working = loader.working_directory.clone();
    let world = &mut loader.world;
    let cell = form(world, 0x901);
    let player = world.resource::<PlayerGlobals>().the_player_b333c4.unwrap();
    world
        .get_mut::<TesObjectCell>(cell)
        .unwrap()
        .objects_48
        .append_non_null(player);
    let passes = |world: &World| world.resource::<InventoryGlobals>().container_script_passes;
    let before = passes(world);
    assert_eq!(
        cell_run_reference_scripts(world, cell, true, true),
        Ok(false)
    );
    assert_eq!(passes(world), before + 1, "the container only");
    assert!(world.resource::<CellRegionLocks>().entries.is_empty());
    // 004CB900, the a2 = 0 arm: neither the container nor the player has 3D (vt+0x154) or +0x08 bit 11, so both skip.
    assert_eq!(
        cell_run_reference_scripts(world, cell, false, true),
        Ok(false)
    );
    assert_eq!(passes(world), before + 1, "no reference with 3D runs");
    assert!(world.resource::<CellRegionLocks>().entries.is_empty());
    let scripts = world
        .resource::<DataHandlerLists>()
        .scripts_64
        .items()
        .count();
    assert_ne!(scripts, 0);
    let script = form(world, 0x910);
    world.get_mut::<FormIdentity>(script).unwrap().flags_08 &= !8;
    let before = passes(world);
    init_scripts_and_cells(
        world,
        &mut DataLoadServices {
            working_directory: &working,
            warning: &mut |_, _| {},
            host: &mut loader.host,
        },
    )
    .unwrap();
    let body = world.get::<Script>(script).unwrap();
    assert_eq!(body.header_18[0x10], 0);
    assert_ne!(
        world.get::<FormIdentity>(script).unwrap().flags_08 & 8,
        0,
        "vt+0x6C"
    );
    assert_eq!(passes(world), before + 1, "each +0xC4 cell");
}

// 00496EA0: nested on one thread; 00498EE0's wait for another's is a host stop. 00496F50 with no entry does nothing.
#[test]
fn a_region_lock_counts_and_stops_on_another_thread() {
    let mut world = World::new();
    let cell = world.spawn_empty().id();
    let mut locks = CellRegionLocks::default();
    locks.release(cell);
    locks.acquire(cell).unwrap();
    locks.acquire(cell).unwrap();
    assert_eq!(locks.entries[&cell].1, 2);
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let mut shared = CellRegionLocks::default();
                std::mem::swap(&mut shared.entries, &mut locks.entries);
                assert!(shared.acquire(cell).is_err());
                std::mem::swap(&mut shared.entries, &mut locks.entries);
            })
            .join()
            .unwrap();
    });
    locks.release(cell);
    assert_eq!(locks.entries[&cell].1, 1);
    locks.release(cell);
    assert!(locks.entries.is_empty());
}

// 0043E720 is reached through a path with a byte above 0x7F.
#[test]
fn a_model_key_with_a_high_byte_hashes_signed() {
    assert_ne!(model_map_bucket(b"\xE9"), u32::from(0xE9_u8) % 1009);
}
