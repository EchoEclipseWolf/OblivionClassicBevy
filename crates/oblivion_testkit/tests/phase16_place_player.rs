//! Phase 16: WinMain 0040EE7A..0040EF23 on the real game, a fresh one per path, against P16-RUN-001, 002, 003. `fixtures/rows/phase16.rows` is lifted from the three raw runs: every `key=value` of a row is copied from its record `seq`, which `the_rows_are_the_raw_records` proves. Each path runs `00406390..0040EF23` as one sequence on one World, and reads its rows.
//!
//! IDA: N/A test harness.

#[path = "game_data_window.rs"]
#[allow(
    dead_code,
    unused_imports,
    reason = "this test binary uses only part of the window"
)]
mod game_data_window;

use std::cell::RefCell;
use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use game_data_window::common::act_rows::RUN_004_ROLL_START;
use game_data_window::common::rows::hex_bytes;
use game_data_window::{GameWindow, evidence_lines, run_00406360_seeded};
use oblivion_core::crt::MsvcRand;
use oblivion_core::error::GameError;
use oblivion_core::fixtures;
use oblivion_sim::bs::character_controller::BhkCharacterController;
use oblivion_sim::bs::model_loader::{
    IoDrainResult, IoDrainServices, io_drain_percent, io_drain_tasks_through_stage,
};
use oblivion_sim::bs::task_manager::TaskGlobals;
use oblivion_sim::tes::actor::{
    Actor, MobileObject, TesObjectRefr, actor_set_z_rotation_if_process_allows,
};
use oblivion_sim::tes::actor_process::{HighProcess, MiddleHighProcess, RandomSource};
use oblivion_sim::tes::ai_process::AiRealArms;
use oblivion_sim::tes::character_controller::actor_character_controller;
use oblivion_sim::tes::data_load::DataLoadServices;
use oblivion_sim::tes::form::FormIdentity;
use oblivion_sim::tes::forms_world::{
    LandHeightTriangle, TesObjectCell, land_find_height_triangle, land_height_at_position,
    land_interpolate_height,
};
use oblivion_sim::tes::os_globals::OsGlobalsData;
use oblivion_sim::tes::player::PlayerCharacter;
use oblivion_sim::tes::player::the_player;
use oblivion_sim::tes::tes::{GameThreadLocals, Tes, TesGlobals};
use oblivion_sim::tes::tes_grid::{
    PlacementChoice, TesGridAnswers, TesGridBoundaries, TesGridServices, cell_load_priority,
    cell_water_height,
};
use oblivion_sim::tes::winmain::winmain_initial_area::{
    InitialAreaPath, InitialAreaResult, RealInitialAreaArms, phase_15_line, run_initial_area,
};
use oblivion_sim::tes::winmain::winmain_place_player::{
    PlacePlayerArms, PlacePlayerResult, RealPlacePlayerArms, phase_16_line, run_place_player,
};
use oblivion_sim::tes::winmain::winmain_world_setup::{
    WorldSetupSeams, WorldspaceInputs, run_world_setup,
};

const RUN_1: &str = "P16-RUN-001";
const RUN_2: &str = "P16-RUN-002";
const RUN_3: &str = "P16-RUN-003";

// ---- the rows ----

/// IDA: N/A a row's fields.
#[derive(Clone, Debug)]
struct Row {
    seq: u32,
    fields: BTreeMap<String, String>,
}

impl Row {
    fn get(&self, key: &str) -> &str {
        self.fields
            .get(key)
            .unwrap_or_else(|| panic!("no {key} in record {}: {:?}", self.seq, self.fields))
    }

    fn hex(&self, key: &str) -> u32 {
        u32::from_str_radix(self.get(key), 16).unwrap()
    }
}

/// IDA: N/A the rows' accounting: every line is read by a comparison, by kind, run and tag.
struct Rows {
    lines: Vec<String>,
    read: RefCell<Vec<bool>>,
}

impl Drop for Rows {
    fn drop(&mut self) {
        fixtures::ledger_read("phase16", &self.read.borrow());
    }
}

impl Rows {
    fn load() -> Self {
        let lines = fixtures::rows("phase16");
        let read = RefCell::new(vec![false; lines.len()]);
        Self { lines, read }
    }

    fn take(&self, kind: &str, run: &str, tag: &str) -> Vec<Row> {
        let prefix = format!("{kind} {run} {tag} ");
        let mut found = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            if let Some(rest) = line.strip_prefix(&prefix) {
                self.read.borrow_mut()[index] = true;
                let fields: BTreeMap<String, String> = rest
                    .split(' ')
                    .map(|part| {
                        let (key, value) = part.split_once('=').unwrap();
                        (key.to_owned(), value.to_owned())
                    })
                    .collect();
                found.push(Row {
                    seq: fields["seq"].parse().unwrap(),
                    fields,
                });
            }
        }
        found
    }

    fn one(&self, kind: &str, run: &str, tag: &str) -> Row {
        let mut rows = self.take(kind, run, tag);
        assert_eq!(rows.len(), 1, "{kind} {run} {tag}");
        rows.remove(0)
    }

    fn assert_read(&self, run: &str) {
        let read = self.read.borrow();
        let unread: Vec<&str> = self
            .lines
            .iter()
            .enumerate()
            .filter(|(index, line)| !read[*index] && line.split(' ').nth(1) == Some(run))
            .map(|(_, line)| &line[..line.len().min(60)])
            .collect();
        assert!(
            unread.is_empty(),
            "{run}: {} rows unread: {:?}",
            unread.len(),
            &unread[..unread.len().min(12)]
        );
        let total = self
            .lines
            .iter()
            .filter(|line| line.split(' ').nth(1) == Some(run))
            .count();
        println!("{run}: {total} rows read");
    }
}

// Every row's fields, in its record `seq`, with the kind and tag in place.
#[test]
fn the_rows_are_the_raw_records() {
    let rows = fixtures::rows("phase16");
    assert!(rows.len() > 1600, "{}", rows.len());
    for (run, name) in [
        (RUN_1, "phase16-run001-2026-10-10"),
        (RUN_2, "phase16-run002-2026-10-10"),
        (RUN_3, "phase16-run003-2026-10-10"),
    ] {
        let records = evidence_lines(name);
        let by_seq: BTreeMap<u32, &String> = records
            .iter()
            .filter_map(|line| Some((line.split(' ').next()?.parse().ok()?, line)))
            .collect();
        let mut seen = 0;
        for row in rows.iter().filter(|row| row.split(' ').nth(1) == Some(run)) {
            let parts: Vec<&str> = row.split(' ').collect();
            let seq: u32 = parts[3].strip_prefix("seq=").unwrap().parse().unwrap();
            let record = by_seq[&seq];
            let tokens: Vec<&str> = record.split(' ').collect();
            let kind = match parts[0] {
                "END" => "M",
                kind => kind,
            };
            assert_eq!(tokens[1], kind, "{row}");
            assert!(
                tokens.contains(&parts[2]),
                "the tag isn't in the record: {row}"
            );
            for field in &parts[4..] {
                if let Some(at) = field.strip_prefix("at=") {
                    assert_eq!(tokens[2], at, "{row}");
                } else {
                    assert!(
                        tokens.contains(field),
                        "{field} isn't in record {seq} of {run}"
                    );
                }
            }
            seen += 1;
        }
        assert!(seen > 30, "{run}: {seen}");
    }
}

// ---- the sequence ----

struct StreamRandom<'a>(&'a mut MsvcRand);

impl RandomSource for StreamRandom<'_> {
    fn time64(&mut self) -> i64 {
        panic!("0047DFA8: the first-use seed, past Phase 11's draw");
    }
    fn srand(&mut self, seed: u32) {
        self.0.srand(seed);
    }
    fn rand(&mut self) -> i32 {
        self.0.rand()
    }
}

// IDA: N/A all of 00406360 and the Phase 14 block, leaving the world at 0040EBB9, and Rect (as `phase15_initial_area`).
fn game_at_window() -> (GameWindow, [f32; 3]) {
    let mut state = run_00406360_seeded(Some(RUN_004_ROLL_START));
    let rect = {
        let GameWindow {
            world,
            collections,
            harness,
            host,
        } = &mut state;
        world.insert_resource(OsGlobalsData::default());
        let lod_land = collections.ini.find_address(0x00B0_2D70).unwrap().raw as u8;
        let mut rand = host.rand;
        let mut warnings = Vec::new();
        let mut services = DataLoadServices {
            working_directory: &harness.root,
            warning: &mut |text, _| warnings.push(text),
            host,
        };
        let mut random = StreamRandom(&mut rand);
        let mut menu_mode = |_: &World| false;
        let outputs = run_world_setup(
            world,
            WorldspaceInputs {
                ini: &collections.ini,
                display_lod_land_b43077: lod_land,
                services: &mut services,
                level2_log: &mut |_| {},
                refresh_map_menu: &mut |_| Ok(false),
            },
            WorldSetupSeams {
                log: &mut |_| {},
                arms: AiRealArms {
                    random: &mut random,
                    menu_mode: &mut menu_mode,
                },
            },
        )
        .unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        outputs.worldspace.default_position
    };
    (state, rect)
}

fn set_starting(state: &mut GameWindow, cell: &str, x: &str, y: &str, space: &str) {
    for (address, text) in [
        (0x00B0_2CE0, cell),
        (0x00B0_2CF8, x),
        (0x00B0_2D00, y),
        (0x00B0_2CF0, space),
    ] {
        state
            .collections
            .ini
            .find_address_mut(address)
            .unwrap()
            .text = Some(text.as_bytes().to_vec());
    }
}

fn tes(world: &World) -> &Tes {
    world.resource::<TesGlobals>().tes.as_deref().unwrap()
}

fn form_id(world: &World, form: Entity) -> u32 {
    world.get::<FormIdentity>(form).unwrap().form_id_0c
}

fn floats_hex(values: &[f32]) -> String {
    values
        .iter()
        .flat_map(|value| value.to_bits().to_le_bytes())
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn floats_of(text: &str) -> Vec<f32> {
    hex_bytes(text)
        .chunks(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

fn word(raw: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(raw[at..at + 4].try_into().unwrap())
}

/// IDA: N/A a point's state, in the records' terms, for its kinds.
#[derive(Clone, Debug, Default)]
struct Snapshot {
    rect: [f32; 3],
    rotation: [f32; 3],
    /// `KIND.key`, formatted as the record prints it.
    values: BTreeMap<String, String>,
    /// The change map's count, which rows read as deltas.
    change_count: u32,
    player_parts: Option<(TesObjectRefr, Actor, MobileObject, PlayerCharacter)>,
    /// 00891560's and 008AC0B0's records so far.
    proxy_calls: (u32, u32),
    /// 00438060's count so far.
    model_queue_calls: u32,
}

fn snapshot(world: &World, rect: [f32; 3], rotation: [f32; 3]) -> Snapshot {
    let mut values = BTreeMap::new();
    let mut put = |key: &str, value: String| {
        values.insert(key.to_owned(), value);
    };
    let data = tes(world);
    let id_or_0 = |cell: Option<Entity>| cell.map_or(0, |cell| form_id(world, cell));
    put(
        "TES.t30",
        format!("{:X}", id_or_0(data.current_exterior_cell_30)),
    );
    put(
        "TES.t34",
        format!("{:X}", id_or_0(data.current_interior_cell_34)),
    );
    put(
        "TES.t74",
        format!("{:X}", id_or_0(data.current_world_space_74)),
    );
    let t20: String = data
        .exterior_coordinates_20
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .map(|byte| format!("{byte:02x}"))
        .collect();
    put("TES.t20", t20);
    let grid = data.grid_cells_08.as_ref().unwrap();
    let size = grid.size_0c;
    put("TES.grids", format!("{size}"));
    let centre = grid.grid_10[(size / 2 + size / 2 * size) as usize].cell_00;
    put("TES.centre", format!("{:X}", id_or_0(centre)));

    let player = the_player(world).unwrap();
    let identity = world.get::<FormIdentity>(player).unwrap();
    let refr = world.get::<TesObjectRefr>(player).unwrap();
    let actor = world.get::<Actor>(player).unwrap();
    let body = world.get::<PlayerCharacter>(player).unwrap();
    let mobile = world.get::<MobileObject>(player).unwrap();
    put("PLAYER.id", format!("{:X}", identity.form_id_0c));
    put("PLAYER.flags", format!("{:X}", identity.flags_08));
    put("PLAYER.rot", floats_hex(&refr.rotation_20));
    put("PLAYER.pos", floats_hex(&refr.position_2c));
    put("PLAYER.scale", format!("{:X}", refr.scale_38.to_bits()));
    put(
        "PLAYER.parent",
        format!("{:X}", id_or_0(refr.parent_cell_40)),
    );
    put("PLAYER.b3c", format!("{:X}", refr.loaded_3d_3c));
    put("PLAYER.dead", format!("{:X}", actor.dead_state_b0));
    let process = mobile.process_58.unwrap();
    assert!(world.get::<HighProcess>(process).is_some());
    put("PLAYER.procvt", "A71814".to_owned());
    put("PLAYERX.p588", format!("{:X}", body.view_flags_588[0]));
    put("PLAYERX.p590", format!("{:X}", body.pc_sleep_hours_590));
    put(
        "PLAYERX.p5d0",
        format!("{:X}", u32::from(body.first_person_node_5d0.is_some())),
    );
    put(
        "PLAYERX.p6e8",
        format!("{:X}", u8::from(body.region_6e8.is_some())),
    );
    put(
        "PLAYERX.p71e",
        format!("{:X}", body.cell_fully_inside_border_region_71e),
    );
    put(
        "PLAYERX.p730",
        format!(
            "{:X}",
            u8::from(body.border_region_candidates_730.is_some())
        ),
    );
    let controller = actor_character_controller(world, player).unwrap();
    let controller = world.get::<BhkCharacterController>(controller).unwrap();
    put("CTRL.f1f4", format!("{:X}", controller.flags_1f4));
    put(
        "CTRL.f320",
        format!("{:X}", controller.floats_320[0].to_bits()),
    );
    let proxy_calls = (
        controller.host_proxy_set_position_calls,
        controller.host_world_mark_calls,
    );
    let locals = world.resource::<GameThreadLocals>();
    put("TL.b184", format!("{:X}", locals.byte_184));
    put("TL.b185", format!("{:X}", locals.track_changes_185));
    put("TL.b186", format!("{:X}", locals.byte_186));
    let tasks = world.resource::<TaskGlobals>();
    let io = tasks.io_manager_b33a10.as_ref().unwrap();
    let clone = &tasks.model_loader_b33a1c.as_ref().unwrap().clone_thread_18;
    put("IO.field_38", format!("{}", io.field_38));
    put("IO.queue_18", format!("{}", io.io_queue_34.field_18));
    put("IO.c24", format!("{:X}", clone.field_24));
    put(
        "IO.clone_18",
        format!("{}", clone.reference_queue_28.field_18),
    );
    let change_count = world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .unwrap()
        .changes_map_00
        .count_0c;
    Snapshot {
        rect,
        rotation,
        values,
        change_count,
        player_parts: Some((refr.clone(), actor.clone(), *mobile, body.clone())),
        proxy_calls,
        model_queue_calls: world
            .get_resource::<TesGridBoundaries>()
            .map_or(0, |boundaries| boundaries.at(0x0043_8060)),
    }
}

struct RecordingPlaceArms<'r, 'a, 'b, 'c, 'd> {
    real: RealPlacePlayerArms<'a, 'b, 'c, 'd>,
    snapshots: &'r mut BTreeMap<&'static str, Snapshot>,
    rect_seen: [f32; 3],
    rotation_seen: [f32; 3],
    logs: Vec<String>,
}

impl PlacePlayerArms for RecordingPlaceArms<'_, '_, '_, '_, '_> {
    fn log(&mut self, text: &str) {
        self.logs.push(text.to_owned());
        self.real.log(text);
    }

    fn prepare_placement_position(
        &mut self,
        world: &mut World,
        cell: Entity,
        rect: &mut [f32; 3],
        rotation: &mut [f32; 3],
    ) -> Result<PlacementChoice, GameError> {
        let choice = self
            .real
            .prepare_placement_position(world, cell, rect, rotation)?;
        (self.rect_seen, self.rotation_seen) = (*rect, *rotation);
        self.snapshots
            .insert("player_virtuals", snapshot(world, *rect, *rotation));
        Ok(choice)
    }

    fn insert_player(
        &mut self,
        world: &mut World,
        cell: Entity,
        player: Entity,
    ) -> Result<(), GameError> {
        let at = snapshot(world, self.rect_seen, self.rotation_seen);
        self.snapshots.insert("after_position", at);
        self.real.insert_player(world, cell, player)
    }

    fn drain_tasks_through_stage(
        &mut self,
        world: &mut World,
        stage: i32,
    ) -> Result<IoDrainResult, GameError> {
        let at = snapshot(world, self.rect_seen, self.rotation_seen);
        self.snapshots.insert("drain", at);
        self.real.drain_tasks_through_stage(world, stage)
    }
}

/// IDA: N/A a path's sequence.
struct PathRun {
    state: GameWindow,
    default_rect: [f32; 3],
    rect: [f32; 3],
    initial: InitialAreaResult,
    placed: PlacePlayerResult,
    snapshots: BTreeMap<&'static str, Snapshot>,
    logs: Vec<String>,
    infos: Vec<String>,
    sleeps: Vec<u32>,
    timed_runs: u32,
    load_warnings: Vec<String>,
}

// IDA: N/A `00406390..0040EF23` on one World: `game_at_window`, Phase 15's real arms, then Phase 16's.
fn run_sequence(cell: &str, x: &str, y: &str) -> PathRun {
    let (mut state, default_rect) = game_at_window();
    set_starting(&mut state, cell, x, y, "");
    let mut rect = default_rect;
    let mut snapshots = BTreeMap::new();
    let logs;
    let mut infos = Vec::new();
    let mut sleeps = Vec::new();
    let mut timed_runs = 0;
    let mut load_warnings = Vec::new();
    let (initial, placed) = {
        let GameWindow {
            world,
            collections,
            harness,
            host,
        } = &mut state;
        let lod_land = collections.ini.find_address(0x00B0_2D70).unwrap().raw as u8;
        let mut warning = |text: String, _: bool| load_warnings.push(text);
        let mut load = DataLoadServices {
            working_directory: &harness.root,
            warning: &mut warning,
            host,
        };
        let mut level2 = |_: String| {};
        let mut refresh = |_: &mut World| Ok(false);
        let mut grid = TesGridServices {
            load: &mut load,
            ini: &collections.ini,
            answers: TesGridAnswers::default(),
            display_lod_land_b43077: lod_land,
            level2_log: &mut level2,
            refresh_map_menu: &mut refresh,
            interface: None,
            drain: None,
            reference_loop: false,
            game_sounds: None,
            archives: None,
            models: None,
        };
        let initial = {
            let mut arms = RealInitialAreaArms { grid: &mut grid };
            run_initial_area(world, &collections.ini, &mut arms, &mut rect).unwrap()
        };
        let before = snapshot(world, rect, [0.0; 3]);
        snapshots.insert("before", before);
        let placed = {
            let mut run_timed = |_: &mut World| {
                timed_runs += 1;
                Ok(())
            };
            let mut sleep = |ms: u32| sleeps.push(ms);
            let mut info = |text: &str| infos.push(text.to_owned());
            let mut arms = RecordingPlaceArms {
                real: RealPlacePlayerArms {
                    grid: &mut grid,
                    run_timed_tasks: &mut run_timed,
                    sleep: &mut sleep,
                    info: &mut info,
                },
                snapshots: &mut snapshots,
                rect_seen: [0.0; 3],
                rotation_seen: [0.0; 3],
                logs: Vec::new(),
            };
            let placed = run_place_player(world, &collections.ini, &mut arms, &mut rect).unwrap();
            logs = arms.logs;
            placed
        };
        (initial, placed)
    };
    let world = &state.world;
    let tag = if placed.reached_0040ef54 {
        "end_nocell"
    } else {
        "end_cell"
    };
    let end = snapshot(world, rect, placed.rotation);
    snapshots.insert(tag, end);
    let coc = state.collections.ini.find_address(0x00B0_97E0).unwrap().raw;
    for snap in snapshots.values_mut() {
        snap.values.insert("TES.coc".to_owned(), format!("{coc:X}"));
    }
    PathRun {
        state,
        default_rect,
        rect,
        initial,
        placed,
        snapshots,
        logs,
        infos,
        sleeps,
        timed_runs,
        load_warnings,
    }
}

// ---- the comparisons ----

/// IDA: N/A why a `PLAYER` row's dumps have no snapshot value.
const RAW_DUMP: &str = "a raw dump with heap words: only its vtable word, presence bytes on the row the block moves, and what moves between records are held, by assert_raw_moves_only and the extra checks (reason written at M7, from the code)";

/// IDA: N/A why a cell's reference rows at the block's middle stops have no comparison.
const MID_BLOCK_REFS: &str = "the host's World here is the block's end, so a list recorded mid-block has nothing to meet: only prepare, insert and end_cell_full are held (reason written at M7, from the code)";

// IDA: N/A for the ledger: one key of one row a comparison passed over, and why. An empty reason is a key with no snapshot value and no record of why.
fn note_field(kind: &str, run: &str, tag: &str, key: &str, reason: &str) {
    fixtures::ledger_relaxed("phase16", &format!("{kind} {run} {tag} #{key}"), 1, reason);
}

// IDA: N/A for the ledger: a key a counted boundary writes in the original.
fn note_owned(kind: &str, run: &str, tag: &str, key: &str) {
    note_field(
        kind,
        run,
        tag,
        key,
        "boundary-owned: a counted boundary writes it in the original (docs/open-items.md, Phase 16)",
    );
}

// IDA: N/A `kind` at `tag`. `owned` names keys a counted boundary writes in the original, not compared here: each is in the report's no-data list. `set_aside` names keys the row carries past what the host models, by name.
fn compare_kind(rows: &Rows, run: &str, kind: &str, tag: &str, at: &Snapshot, owned: &[&str]) {
    let row = rows.one(kind, run, tag);
    for (key, value) in &row.fields {
        if key == "seq" {
            continue;
        }
        if owned.contains(&key.as_str()) {
            note_owned(kind, run, tag, key);
            continue;
        }
        let mut want = value.rsplit('/').next().unwrap();
        // Host pointers: null-ness.
        if matches!(key.as_str(), "p6e8" | "p730") && want != "0" {
            note_field(
                kind,
                run,
                tag,
                key,
                "a host pointer: null-ness only (docs/open-items.md, Phase 16)",
            );
            want = "1";
        }
        let name = format!("{kind}.{key}");
        // The caller's checks read these two where a word can be compared.
        if matches!(key.as_str(), "raw" | "extra") {
            note_field(kind, run, tag, key, RAW_DUMP);
            continue;
        }
        let Some(host) = at.values.get(&name) else {
            note_field(kind, run, tag, key, "");
            continue;
        };
        assert_eq!(host, want, "{run} {tag} {name}, record {}", row.seq);
    }
}

fn compare_win(rows: &Rows, run: &str, tag: &str, at: &Snapshot, rotation_set: bool) {
    let row = rows.one("WIN", run, tag);
    assert_eq!(floats_hex(&at.rect), row.get("rect"), "{run} {tag} rect");
    // 0040EE8D stores rotation after the `before` record, whose words are stack junk.
    if !rotation_set {
        note_field(
            "WIN",
            run,
            tag,
            "rotation",
            "0040EE8D stores it after this record: stack junk",
        );
    }
    if rotation_set {
        assert_eq!(
            floats_hex(&at.rotation),
            row.get("rotation"),
            "{run} {tag} rotation"
        );
    }
}

// IDA: N/A the player's unchanged parts across the block, with only the named ones moving.
fn assert_player_unchanged(before: &Snapshot, after: &Snapshot, what: &str) {
    let (mut refr, actor, mobile, body) = before.player_parts.clone().unwrap();
    let (after_refr, after_actor, after_mobile, after_body) = after.player_parts.clone().unwrap();
    refr.rotation_20 = after_refr.rotation_20;
    refr.position_2c = after_refr.position_2c;
    refr.parent_cell_40 = after_refr.parent_cell_40;
    assert_eq!(refr, after_refr, "{what}: TesObjectRefr");
    assert_eq!(actor, after_actor, "{what}: Actor");
    assert_eq!(mobile, after_mobile, "{what}: MobileObject");
    assert_eq!(body, after_body, "{what}: PlayerCharacter");
}

// The raw 0x104-byte dump: between two records of a run, only these offsets may move.
fn assert_raw_moves_only(run: &str, from: &Row, to: &Row, moving: std::ops::Range<usize>) {
    let (a, b) = (hex_bytes(from.get("raw")), hex_bytes(to.get("raw")));
    assert_eq!(a.len(), b.len());
    for at in 0..a.len() {
        if !moving.contains(&at) {
            assert_eq!(a[at], b[at], "{run}: raw byte {at:X} moved");
        }
    }
}

// The change map's count, in a row's `m0` dump: +0x0C.
fn change_delta(from: &Row, to: &Row) -> i64 {
    let count = |row: &Row| i64::from(word(&hex_bytes(row.get("m0")), 0xC));
    count(to) - count(from)
}

// IDA: N/A the IO rows: +0x38 and the queue and clone counts. +0x18 of the map isn't compared: `no_work` below says why.
fn compare_io(rows: &Rows, run: &str, tag: &str, at: &Snapshot) {
    let row = rows.one("IO", run, tag);
    note_field("IO", run, tag, "qraw", RAW_DUMP);
    note_field("IO", run, tag, "c28raw", RAW_DUMP);
    let raw = hex_bytes(row.get("raw"));
    let queue = hex_bytes(row.get("qraw"));
    let clone_queue = hex_bytes(row.get("c28raw"));
    assert_eq!(
        word(&raw, 0x38).to_string(),
        at.values["IO.field_38"],
        "{run} {tag} +0x38"
    );
    assert_eq!(word(&raw, 8), 0x12, "{run} {tag} the 18 buckets");
    assert_eq!(
        word(&queue, 0x18).to_string(),
        at.values["IO.queue_18"],
        "{run} {tag} queue +0x18"
    );
    assert_eq!(
        row.get("c24"),
        at.values["IO.c24"],
        "{run} {tag} clone +0x24"
    );
    assert_eq!(
        word(&clone_queue, 0x18).to_string(),
        at.values["IO.clone_18"],
        "{run} {tag} clone queue +0x18"
    );
}

// IDA: N/A a cell against CELL and REF rows. `player_first` holds the rows' head for the inserted player; FF-prefixed rows and moved actors are the counted 004D5BD0's.
fn compare_cell(rows: &Rows, run: &str, tag: &str, world: &World, cell: Entity, player_in: bool) {
    let row = rows.one("CELL", run, tag);
    let what = format!("{run} {tag}");
    let identity = world.get::<FormIdentity>(cell).unwrap();
    let data = world.get::<TesObjectCell>(cell).unwrap();
    assert_eq!(identity.form_id_0c, row.hex("id"), "{what}");
    assert_eq!(u32::from(identity.form_type_04), row.hex("type"), "{what}");
    assert_eq!(u32::from(data.flags_24), row.hex("b24"), "{what} +0x24");
    assert_eq!(u32::from(data.field_25), row.hex("b25"), "{what} +0x25");
    assert_eq!(
        u32::from(data.process_level_26),
        row.hex("b26"),
        "{what} +0x26"
    );
    assert_eq!(identity.flags_08 | 2, row.hex("flags") | 2, "{what} flags");
    assert_eq!(
        data.world_50
            .filter(|_| data.flags_24 & 1 == 0)
            .map_or(0, |space| form_id(world, space)),
        row.hex("ws"),
        "{what} world"
    );
    assert_eq!(
        data.land_40.map_or(0, |land| form_id(world, land)),
        row.hex("land"),
        "{what} land"
    );
    assert_eq!(
        data.path_grid_44.map_or(0, |grid| form_id(world, grid)),
        row.hex("pgrd"),
        "{what} path grid"
    );
    let block: Vec<u8> = data
        .data_3c
        .unwrap()
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let wanted = hex_bytes(row.get("data"));
    let compared = if data.flags_24 & 1 != 0 { 0x28 } else { 8 };
    assert_eq!(&block[..compared], &wanted[..compared], "{what} +0x3C");
    let counts = rows.one("REFS", run, tag);
    assert_eq!(form_id(world, cell), counts.hex("cell"), "{what} REFS cell");
    let run_refs = rows.take("REF", run, tag);
    fixtures::ledger_relaxed(
        "phase16",
        &format!("REF {run} {tag} #ref"),
        run_refs.len(),
        "a pointer into the original's heap: the row's ID names the reference",
    );
    if !matches!(tag, "insert" | "end_cell_full" | "prepare") {
        fixtures::ledger_relaxed("phase16", &format!("REFS {run} {tag}"), 1, MID_BLOCK_REFS);
        fixtures::ledger_relaxed(
            "phase16",
            &format!("REF {run} {tag}"),
            run_refs.len(),
            MID_BLOCK_REFS,
        );
        return;
    }
    assert_eq!(counts.get("count"), run_refs.len().to_string(), "{what}");
    for row in run_refs.iter().filter(|row| row.hex("id") >= 0xFF00_0000) {
        fixtures::ledger_relaxed(
            "phase16",
            &format!("REF {run} {tag} seq={} n={}", row.seq, row.get("n")),
            1,
            "a dynamic reference: 004D5BD0's actor boundary makes it, and only its count is held (docs/open-items.md, Phase 15)",
        );
    }
    // The world is the end of the block: the rows of `prepare` and `insert` are before 004D35D0's head.
    let host: Vec<Entity> = data
        .objects_48
        .items()
        .filter(|&form| tag == "end_cell_full" || form_id(world, form) != 0x14)
        .collect();
    let host_ids: Vec<u32> = host.iter().map(|&form| form_id(world, form)).collect();
    let static_rows: Vec<&Row> = run_refs
        .iter()
        .filter(|row| row.hex("id") < 0xFF00_0000)
        .collect();
    let run_ids: Vec<u32> = static_rows.iter().map(|row| row.hex("id")).collect();
    if tag == "prepare" {
        // The run stopped at 40: the first 40 nodes hold the same order, past the dynamic ones.
        assert_eq!(&host_ids[..run_ids.len()], &run_ids[..], "{what} order");
    } else {
        assert_eq!(host_ids, run_ids, "{what} order");
    }
    for (&reference, row) in host.iter().zip(&static_rows) {
        let id = form_id(world, reference);
        if player_in && id == 0x14 {
            continue;
        }
        let identity = world.get::<FormIdentity>(reference).unwrap();
        let refr = world.get::<TesObjectRefr>(reference).unwrap();
        let base = refr.base_form_1c.unwrap();
        assert_eq!(identity.flags_08, row.hex("flags"), "{what} {id:X} flags");
        assert_eq!(
            u32::from(identity.form_type_04),
            row.hex("type"),
            "{what} {id:X} type"
        );
        assert_eq!(
            u32::from(world.get::<FormIdentity>(base).unwrap().form_type_04),
            row.hex("btype"),
            "{what} {id:X} base type"
        );
        // The vtable, and the presence bytes: the counted 004D5BD0 adds extras the host lacks.
        let extra = hex_bytes(row.get("extra"));
        assert_eq!(
            word(&extra, 0),
            refr.extra_data_44.original_vtable,
            "{what} {id:X}"
        );
        for (&host, &run) in refr.extra_data_44.presence_08.iter().zip(&extra[8..20]) {
            assert_eq!(host & !run, 0, "{what} {id:X}: a host extra the run lacks");
        }
        assert_eq!(form_id(world, base), row.hex("base"), "{what} {id:X} base");
        assert_eq!(refr.scale_38.to_bits(), row.hex("scale"), "{what} {id:X}");
        assert_eq!(
            form_id(world, refr.parent_cell_40.unwrap()),
            row.hex("parent"),
            "{what} {id:X} parent"
        );
        if floats_hex(&refr.position_2c) != row.get("pos")
            || floats_hex(&refr.rotation_20) != row.get("rot")
        {
            assert!(
                matches!(identity.form_type_04, 0x32 | 0x33),
                "{what} {id:X}: pos or rot moved on a non-actor"
            );
        }
    }
}

// IDA: N/A the pointer of `id` in REF rows, to name the record's candidates.
fn candidates(rows: &Row, by_pointer: &BTreeMap<u32, u32>) -> [u32; 4] {
    let cands = hex_bytes(rows.get("cands"));
    [0x08, 0x0C, 0x10, 0x14].map(|at| {
        let pointer = word(&cands, at);
        if pointer == 0 {
            0
        } else {
            by_pointer[&pointer]
        }
    })
}

fn pointers(rows: &Rows, run: &str) -> BTreeMap<u32, u32> {
    rows.lines
        .iter()
        .filter(|line| line.starts_with(&format!("REF {run} end_cell_full ")))
        .map(|line| {
            let find = |key: &str| {
                let prefix = format!("{key}=");
                line.split(' ')
                    .find_map(|part| part.strip_prefix(prefix.as_str()))
                    .map(|value| u32::from_str_radix(value, 16).unwrap())
                    .unwrap()
            };
            (find("ref"), find("id"))
        })
        .collect()
}

fn check_placement(rows: &Rows, run: &str, world: &World, choice: &PlacementChoice) {
    let by_pointer = pointers(rows, run);
    let entry = rows.one("V", run, "cee90");
    // 0040EE7A's cell, which the player ends the block in.
    let player = world
        .resource::<oblivion_sim::tes::data_handler::FormsById>()
        .lookup(0x14)
        .unwrap();
    let cell = world.get::<TesObjectRefr>(player).unwrap().parent_cell_40;
    assert_eq!(
        form_id(world, cell.unwrap()),
        entry.hex("id"),
        "{run} entry cell"
    );
    assert_eq!(
        floats_hex(&choice.entry.0),
        entry.get("posv"),
        "{run} entry"
    );
    assert_eq!(
        floats_hex(&choice.entry.1),
        entry.get("rotv"),
        "{run} entry"
    );
    let chosen = rows.one("V", run, "chosen");
    let host_chosen = choice.chosen.unwrap();
    assert_eq!(
        form_id(world, host_chosen),
        chosen.hex("id"),
        "{run} chosen"
    );
    assert_eq!(
        u32::from(world.get::<FormIdentity>(host_chosen).unwrap().form_type_04),
        chosen.hex("type"),
        "{run}"
    );
    let refr = world.get::<TesObjectRefr>(host_chosen).unwrap();
    assert_eq!(
        form_id(world, refr.base_form_1c.unwrap()),
        chosen.hex("base"),
        "{run} chosen base"
    );
    let base_type = world
        .get::<FormIdentity>(refr.base_form_1c.unwrap())
        .unwrap()
        .form_type_04;
    assert_eq!(u32::from(base_type), chosen.hex("btype"), "{run}");
    assert_eq!(floats_hex(&refr.position_2c), chosen.get("pos"), "{run}");
    assert_eq!(floats_hex(&refr.rotation_20), chosen.get("rot"), "{run}");
    // [esp+0x18] heading, +0x1C XMarker, +0x20 type 0x1C, +0x24 last.
    let [heading, marker, type_1c, last] = candidates(&chosen, &by_pointer);
    let id = |form: Option<Entity>| form.map_or(0, |form| form_id(world, form));
    assert_eq!(id(choice.x_marker_heading), heading, "{run} [esp+0x18]");
    assert_eq!(id(choice.x_marker), marker, "{run} [esp+0x1C]");
    assert_eq!(id(choice.type_1c), type_1c, "{run} [esp+0x20]");
    if last < 0xFF00_0000 {
        assert_eq!(id(choice.last), last, "{run} [esp+0x24]");
    }
    let after = rows.one("V", run, "after_choice");
    assert_eq!(
        after.hex("door"),
        u32::from(choice.door),
        "{run} [esp+0x13]"
    );
    assert_eq!(
        candidates(&after, &by_pointer),
        [heading, marker, type_1c, last]
    );
    assert_eq!(after.get("rot"), chosen.get("rot"), "{run}");
    assert_eq!(after.get("pos"), chosen.get("pos"), "{run}");
}

fn check_virtuals(rows: &Rows, run: &str, rect: &[f32; 3], rotation: &[f32; 3], set_z: u32) {
    let zrot = rows.one("V", run, "zrot");
    assert_eq!(zrot.hex("arg"), rotation[2].to_bits(), "{run} vt+0x1E8");
    assert_eq!(zrot.hex("dead"), 0);
    let set = rows.one("V", run, "set_rz");
    assert_eq!(set.hex("arg"), set_z, "{run} 004D8A10");
    // 0065A507 is the negative arm, 0065A53D the unchanged one.
    assert_eq!(
        set.hex("ret"),
        if f32::from_bits(zrot.hex("arg")) < 0.0 {
            0x0065_A507
        } else {
            0x0065_A53D
        },
        "{run}"
    );
    let position = rows.one("V", run, "setpos");
    assert_eq!(position.get("pos"), floats_hex(rect), "{run} vt+0x1CC");
    let controller = rows.one("V", run, "ctrl");
    assert_eq!(controller.hex("f1f4"), 0, "{run}");
    assert_eq!(controller.hex("f320"), 0, "{run}");
}

fn check_blocks(rows: &Rows, run: &str, cell: bool) {
    let wanted: &[(&str, &str)] = if cell {
        &[
            ("prepare", "40EEC8"),
            ("player_virtuals", "40EED9"),
            ("after_position", "40EF06"),
            ("insert", "40EF0A"),
            ("drain", "40EF17"),
        ]
    } else {
        &[("player_virtuals", "40EED9"), ("after_position", "40EF06")]
    };
    for (tag, at) in wanted {
        assert_eq!(rows.one("WB", run, tag).get("at"), *at, "{run} {tag}");
    }
}

// ---- P16-RUN-001 ----

// The user's INI: +0x34 null, the centre slot (2, 2) empty, rotation 0, vt+0x1E8(0) stores +0x28 = 0, whose vt+0x40(4) adds the player's change entry; vt+0x1CC moves z to 128; the controller's +0x320 and +0x1F4; 0040EF54.
#[test]
fn run_001_the_default_path_whole() {
    let started = std::time::Instant::now();
    let rows = Rows::load();
    let run = run_sequence("", "", "");
    let world = &run.state.world;
    let at = &run.snapshots;
    assert_eq!(run.initial.path, InitialAreaPath::NoSettings);
    assert!(!run.initial.tail);
    assert!(run.load_warnings.is_empty());
    assert_eq!(run.logs, ["Placing player..."]);
    assert_eq!(run.infos, ["Placing player..."]);
    assert_eq!(
        phase_15_line(&run.initial, 0, &run.default_rect),
        "Phase-15: path no settings, cell 0, Rect {2048, 2048, 128}, tail skipped"
    );
    let line = phase_16_line(world, &run.placed, &run.rect);
    assert_eq!(
        line, "Phase-16: cell 0, Rect {2048, 2048, 128}, rotation z 0, 0040EF54",
        "G3"
    );
    let baseline =
        std::fs::read_to_string(fixtures::root().join("cpp-baseline/LastTest-x64.log")).unwrap();
    assert!(baseline.contains(&line), "the C++ prints the same line");
    assert!(run.placed.cell.is_none());
    assert!(run.placed.from_centre_slot, "0040EEB1");
    assert!(run.placed.reached_0040ef54, "0040EF08 jumps");
    assert!(run.placed.drain.is_none());
    assert_eq!(rows.one("WB", RUN_1, "no_interior").get("at"), "40EEB1");
    check_blocks(&rows, RUN_1, false);
    assert_eq!(rows.one("END", RUN_1, "end_nocell").seq, 52);
    let end = &at["end_nocell"];
    let mut virtuals = at["before"].clone();
    virtuals.rotation = [0.0; 3];
    compare_win(&rows, RUN_1, "before", &at["before"], false);
    compare_win(&rows, RUN_1, "player_virtuals", &virtuals, true);
    compare_win(&rows, RUN_1, "after_position", end, true);
    compare_win(&rows, RUN_1, "end_nocell", end, true);
    assert_eq!(
        floats_hex(&run.rect),
        "000000450000004500000043",
        "Rect {{2048, 2048, 128}}"
    );
    let before_player = rows.one("PLAYER", RUN_1, "before");
    let after_player = rows.one("PLAYER", RUN_1, "after_position");
    compare_kind(&rows, RUN_1, "TES", "before", &at["before"], &[]);
    compare_kind(&rows, RUN_1, "TES", "player_virtuals", &virtuals, &[]);
    compare_kind(&rows, RUN_1, "TES", "after_position", end, &[]);
    compare_kind(&rows, RUN_1, "TES", "end_nocell", end, &[]);
    for (key, value) in &before_player.fields {
        if let Some(host) = at["before"].values.get(&format!("PLAYER.{key}")) {
            assert_eq!(host, value.rsplit('/').next().unwrap(), "before {key}");
        } else if key != "seq" {
            note_field("PLAYER", RUN_1, "before", key, RAW_DUMP);
        }
    }
    for (key, value) in &after_player.fields {
        if let Some(host) = end.values.get(&format!("PLAYER.{key}")) {
            assert_eq!(host, value.rsplit('/').next().unwrap(), "after {key}");
        } else if key != "seq" {
            note_field("PLAYER", RUN_1, "after_position", key, RAW_DUMP);
        }
    }
    compare_kind(
        &rows,
        RUN_1,
        "PLAYER",
        "player_virtuals",
        &at["before"],
        &[],
    );
    compare_kind(&rows, RUN_1, "PLAYER", "end_nocell", end, &[]);
    assert_raw_moves_only(RUN_1, &before_player, &after_player, 0x34..0x38);
    assert_player_unchanged(&at["before"], end, "RUN-001");
    let extra = hex_bytes(after_player.get("extra"));
    let refr = world
        .get::<TesObjectRefr>(the_player(world).unwrap())
        .unwrap();
    assert_eq!(word(&extra, 0), refr.extra_data_44.original_vtable);
    assert_eq!(&extra[8..20], &refr.extra_data_44.presence_08, "RUN-001");
    for (tag, snap) in [
        ("before", &at["before"]),
        ("player_virtuals", &virtuals),
        ("after_position", end),
        ("end_nocell", end),
    ] {
        compare_kind(&rows, RUN_1, "PLAYERX", tag, snap, &[]);
        compare_kind(&rows, RUN_1, "TL", tag, snap, &[]);
        compare_io(&rows, RUN_1, tag, snap);
    }
    compare_kind(&rows, RUN_1, "CTRL", "after_position", end, &[]);
    compare_kind(&rows, RUN_1, "CTRL", "end_nocell", end, &[]);
    check_virtuals(&rows, RUN_1, &run.rect, &[0.0; 3], 0);
    let (first, last) = (
        rows.one("SL", RUN_1, "before"),
        rows.one("SL", RUN_1, "after_position"),
    );
    assert_eq!(change_delta(&first, &last), 1, "RUN-001: one new record");
    assert_eq!(
        i64::from(end.change_count) - i64::from(at["before"].change_count),
        1,
        "RUN-001: one new change record, the host"
    );
    println!(
        "RUN-001: change map {:X} in the run, the host's {:X}, at 0040EE7A",
        word(&hex_bytes(first.get("m0")), 0xC),
        at["before"].change_count
    );
    let middle = rows.one("SL", RUN_1, "player_virtuals");
    assert_eq!(change_delta(&first, &middle), 0);
    assert_eq!(change_delta(&last, &rows.one("SL", RUN_1, "end_nocell")), 0);
    let player_id = 0x14;
    let game = world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .unwrap();
    assert!(
        game.changes_00
            .get(player_id)
            .is_some_and(|record| record.flags_00 & 4 != 0),
        "the player's entry, flag 4"
    );
    let controller = world
        .get::<BhkCharacterController>(
            actor_character_controller(world, the_player(world).unwrap()).unwrap(),
        )
        .unwrap();
    assert_eq!(
        (
            end.proxy_calls.0 - at["before"].proxy_calls.0,
            end.proxy_calls.1 - at["before"].proxy_calls.1
        ),
        (1, 1),
        "00452A10, 008AC0B0"
    );
    assert_eq!(
        floats_hex(&controller.host_proxy_position),
        floats_hex(&[
            (2048.0_f64 * 0.142_876_729_369_163_5) as f32,
            (2048.0_f64 * 0.142_876_729_369_163_5) as f32,
            (128.0_f64 * 0.142_876_729_369_163_5) as f32,
        ])
    );
    rows.assert_read(RUN_1);
    println!("RUN-001 whole: {:?}", started.elapsed());
}

// ---- P16-RUN-002 ----

// XPAichan01: Phase 15 enters 00010360; 004D5D70 starts at {2048, 2048, 0}; 004D4310 finds it loaded; 004CEE90 finds every candidate and the door's point, and the XMarkerHeading 0005F2F5 wins. No lift: an interior. 004D35D0 puts the player at the head. The IO queue the 3D boundaries fill in the original is empty here.
#[test]
fn run_002_an_interior_cell_whole() {
    let started = std::time::Instant::now();
    let rows = Rows::load();
    let run = run_sequence("XPAichan01", "", "");
    let world = &run.state.world;
    let at = &run.snapshots;
    assert_eq!(run.initial.path, InitialAreaPath::EditorIdCell);
    let cell = run.placed.cell.unwrap();
    assert_eq!(form_id(world, cell), 0x10360);
    assert!(!run.placed.from_centre_slot);
    assert!(!run.placed.reached_0040ef54);
    let choice = run.placed.placement.unwrap();
    check_placement(&rows, RUN_2, world, &choice);
    assert!(choice.lift.is_none(), "an interior doesn't lift");
    let door = rows.one("V", RUN_2, "door");
    let point = choice.door_point.unwrap();
    assert_eq!(floats_hex(&point.0), door.get("pos"), "RUN-002 the door");
    assert_eq!(floats_hex(&point.1), door.get("rot"), "RUN-002 the door");
    assert_eq!(
        rows.one("V", RUN_2, "final").get("pos"),
        floats_hex(&run.rect)
    );
    check_blocks(&rows, RUN_2, true);
    assert_eq!(rows.one("END", RUN_2, "end_cell").seq, 1693);
    let drain = run.placed.drain.as_ref().unwrap();
    compare_win(&rows, RUN_2, "before", &at["before"], false);
    for tag in ["player_virtuals", "after_position", "drain", "end_cell"] {
        compare_win(&rows, RUN_2, tag, &at[tag], true);
    }
    compare_common(&rows, RUN_2, &run, 0x40C8_CB08, &["b3c"], &["p5d0"]);
    assert_eq!(
        phase_16_line(world, &run.placed, &run.rect),
        format!(
            "Phase-16: cell 66400, Rect {{{}, {}, {}}}, rotation z {}, 0040EF24",
            run.rect[0], run.rect[1], run.rect[2], run.placed.rotation[2]
        )
    );
    compare_cell(&rows, RUN_2, "prepare", world, cell, false);
    compare_cell_later(&rows, RUN_2, world, cell);
    insert_checks(&rows, RUN_2, world, cell);
    assert!(
        !drain.waited,
        "the host's queue is empty: 00438060 is counted"
    );
    assert!(run.sleeps.is_empty() && run.timed_runs == 0);
    scripted_drain(&rows, RUN_2, &mut run_state_world(run.state), 0x218);
    rows.assert_read(RUN_2);
    println!("RUN-002 whole: {:?}", started.elapsed());
}

fn run_state_world(state: GameWindow) -> World {
    let GameWindow { world, .. } = state;
    world
}

// IDA: N/A the kinds every cell path shares. `owned` the PLAYER keys, `owned_x` the PLAYERX keys that a boundary writes.
fn compare_common(
    rows: &Rows,
    run: &str,
    path: &PathRun,
    set_z: u32,
    owned: &[&str],
    owned_x: &[&str],
) {
    let at = &path.snapshots;
    let before_row = rows.one("PLAYER", run, "before");
    let virtuals = rows.one("PLAYER", run, "player_virtuals");
    let after = rows.one("PLAYER", run, "after_position");
    let regions = rows.one("PLAYER", run, "cell_regions");
    let drain = rows.one("PLAYER", run, "drain");
    let end = rows.one("PLAYER", run, "end_cell");
    for (row, snap, tag) in [
        (&before_row, &at["before"], "before"),
        (&virtuals, &at["before"], "player_virtuals"),
        (&after, &at["after_position"], "after_position"),
        (&regions, &at["after_position"], "cell_regions"),
        (&drain, &at["drain"], "drain"),
        (&end, &at["end_cell"], "end_cell"),
    ] {
        for (key, value) in &row.fields {
            if tag == "end_cell" && owned.contains(&key.as_str()) {
                note_owned("PLAYER", run, tag, key);
                continue;
            }
            if let Some(host) = snap.values.get(&format!("PLAYER.{key}")) {
                assert_eq!(
                    host,
                    value.rsplit('/').next().unwrap(),
                    "{run} {tag} PLAYER.{key}"
                );
            } else if key != "seq" {
                note_field("PLAYER", run, tag, key, RAW_DUMP);
            }
        }
    }
    assert_raw_moves_only(run, &before_row, &virtuals, 0..0);
    assert_eq!(
        at["drain"].model_queue_calls,
        at["after_position"].model_queue_calls + 1,
        "{run} 004D3882"
    );
    assert_raw_moves_only(run, &virtuals, &after, 0x28..0x38);
    assert_player_unchanged(&at["before"], &at["after_position"], run);
    assert_eq!(
        world_rotation_z(&at["after_position"]),
        set_z,
        "{run} +0x28 from 004D8A10"
    );
    for tag in [
        "before",
        "player_virtuals",
        "after_position",
        "drain",
        "end_cell",
    ] {
        let snap = if tag == "player_virtuals" {
            &at["player_virtuals"]
        } else {
            &at[tag]
        };
        compare_kind(rows, run, "TES", tag, snap, &[]);
        compare_kind(rows, run, "TL", tag, snap, &[]);
        compare_kind(rows, run, "PLAYERX", tag, snap, owned_x);
    }
    compare_kind(
        rows,
        run,
        "PLAYERX",
        "cell_regions",
        &at["after_position"],
        &[],
    );
    for tag in ["cell_regions", "after_position", "drain", "end_cell"] {
        let snap = if tag == "cell_regions" {
            &at["after_position"]
        } else {
            &at[tag]
        };
        compare_kind(rows, run, "CTRL", tag, snap, &[]);
    }
    for tag in [
        "before",
        "player_virtuals",
        "after_position",
        "drain",
        "end_cell",
    ] {
        compare_io(rows, run, tag, &at[tag]);
    }
    // Change records: one, for the player, between `player_virtuals` and `after_position`, and none past that in the row or the host.
    let sl = |tag: &str| rows.one("SL", run, tag);
    let (sl_before, sl_virtuals, sl_after, sl_drain, sl_end) = (
        sl("before"),
        sl("player_virtuals"),
        sl("after_position"),
        sl("drain"),
        sl("end_cell"),
    );
    let host_delta =
        |from: &str, to: &str| i64::from(at[to].change_count) - i64::from(at[from].change_count);
    assert_eq!(
        change_delta(&sl_before, &sl_virtuals),
        host_delta("before", "player_virtuals"),
        "{run} SL before"
    );
    assert_eq!(
        change_delta(&sl_virtuals, &sl_after),
        host_delta("player_virtuals", "after_position"),
        "{run} SL at the virtuals"
    );
    assert_eq!(
        change_delta(&sl_after, &sl_drain),
        host_delta("after_position", "drain"),
        "{run} SL at the insert"
    );
    fixtures::ledger_relaxed(
        "phase16",
        &format!("SL {run} end_cell"),
        1,
        "the absolute change count differs on every path; only deltas are held, and this last one has no host delta (docs/open-items.md, Phase 16)",
    );
    println!(
        "{run}: change map {:X} to {:X} past the drain, in the run; the host's {:X} to {:X}",
        word(&hex_bytes(sl_drain.get("m0")), 0xC),
        word(&hex_bytes(sl_end.get("m0")), 0xC),
        at["drain"].change_count,
        at["end_cell"].change_count,
    );
    check_virtuals(rows, run, &path.rect, &path.placed.rotation, set_z);
}

fn world_rotation_z(snap: &Snapshot) -> u32 {
    snap.player_parts.as_ref().unwrap().0.rotation_20[2].to_bits()
}

// The cell rows at the tags past `prepare`, and the full lists.
fn compare_cell_later(rows: &Rows, run: &str, world: &World, cell: Entity) {
    for tag in [
        "player_virtuals",
        "after_position",
        "insert",
        "drain",
        "end_cell",
        "end_cell_full",
    ] {
        compare_cell(rows, run, tag, world, cell, true);
    }
}

// 004D35D0's arms for the player.
fn insert_checks(rows: &Rows, run: &str, world: &World, cell: Entity) {
    let entry = rows.one("V", run, "insert_entry");
    assert_eq!(form_id(world, cell), entry.hex("id"), "{run}");
    let identity = world.get::<FormIdentity>(cell).unwrap();
    // Bit 2 is the counted 004D5BD0's, as in Phase 15's tail.
    assert_eq!(identity.flags_08 | 2, entry.hex("flags"), "{run}");
    let data = world.get::<TesObjectCell>(cell).unwrap();
    assert_eq!(u32::from(data.flags_24), entry.hex("b24"), "{run}");
    assert_eq!(u32::from(data.process_level_26), entry.hex("b26"), "{run}");
    assert_eq!(
        rows.one("V", run, "cell_regions").hex("id"),
        form_id(world, cell),
        "{run}"
    );
    let queue = rows.one("V", run, "model_queue");
    assert_eq!(queue.hex("a1id"), 0x14, "{run}");
    assert_eq!(queue.hex("ret"), 0x004D_3887, "{run}: the call 004D3882");
    let boundaries = world.resource::<TesGridBoundaries>();
    assert_eq!(
        boundaries.model_queue_priorities,
        [queue.hex("a2") as i32],
        "{run}: 00440C80's answer"
    );
    for address in [0x0065_B5C0, 0x0043_DF10, 0x005A_64B0, 0x0065_E860] {
        assert_eq!(boundaries.at(address), 1, "{run} {address:08X}");
    }
    assert_eq!(
        form_id(
            world,
            world
                .get::<TesObjectRefr>(the_player(world).unwrap())
                .unwrap()
                .parent_cell_40
                .unwrap()
        ),
        form_id(world, cell)
    );
    let head = data.objects_48.items().next().unwrap();
    assert_eq!(form_id(world, head), 0x14, "{run}: +0x48's head");
    // The player's new +0x318 is `004CACE0 * 0.1428767293691635`; no row sees it: an interior's or exterior's water from the world.
    let controller = world
        .get::<BhkCharacterController>(
            actor_character_controller(world, the_player(world).unwrap()).unwrap(),
        )
        .unwrap();
    let water = cell_water_height(world, cell);
    assert_eq!(
        controller.water_height_318.to_bits(),
        ((f64::from(water) * 0.142_876_729_369_163_5) as f32).to_bits()
    );
}

// The run's counts, over the real loop: +0x2C[0] and +0x18 stand for the boundary's queue. Each 00433590 moves to the next `d_loop`.
fn scripted_drain(rows: &Rows, run: &str, world: &mut World, map_count: u32) {
    let iom = rows.one("V", run, "d_iom");
    assert_eq!(iom.hex("eax"), map_count, "{run} vt+0x38 at 0043402E");
    let total_row = rows.one("V", run, "d_total");
    let total = total_row.hex("ecx");
    assert_eq!(total_row.hex("edx"), total_row.hex("edx"));
    let progress = rows.take("V", run, "d_progress");
    let loops = rows.take("V", run, "d_loop");
    assert_eq!(progress.len(), loops.len(), "{run}");
    let end = rows.one("V", run, "d_end");
    assert_eq!(end.hex("eax"), 0, "{run}");
    // Past the host's gated tasks, which the counts leave out.
    let (base, map_base) = {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let io = tasks.io_manager_b33a10.as_mut().unwrap();
        let bases = (io.array_2c[0], io.task_map.field_18);
        io.task_map.field_18 += map_count;
        io.array_2c[0] += total;
        bases
    };
    let mut remaining: Vec<u32> = loops.iter().map(|row| row.hex("eax")).collect();
    remaining.reverse();
    let mut percents = Vec::new();
    let mut sleeps = Vec::new();
    let mut run_timed = |world: &mut World| {
        let next = remaining.pop().unwrap();
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let io = tasks.io_manager_b33a10.as_mut().unwrap();
        io.array_2c[0] = base + next;
        io.task_map.field_18 = map_base + next;
        Ok(())
    };
    let mut section =
        |_: &mut World, section: i32, percent: f32| percents.push((section, percent.to_bits()));
    let mut save_load = |_: i32, _: f32| panic!("004523A0: no save load");
    let mut sleep = |ms: u32| sleeps.push(ms);
    let result = io_drain_tasks_through_stage(
        world,
        5,
        &mut IoDrainServices {
            run_timed_tasks: &mut run_timed,
            section_progress: &mut section,
            save_load_progress: &mut save_load,
            sleep: &mut sleep,
        },
    )
    .unwrap();
    assert!(result.waited, "{run}");
    assert_eq!(result.total, total, "{run} 004340B7");
    assert_eq!(result.loops as usize, progress.len(), "{run}");
    for (row, seen) in progress.iter().zip(&percents) {
        assert_eq!(row.hex("s0"), seen.0 as u32, "{run} section");
        assert_eq!(row.hex("s1"), seen.1, "{run} percent {}", row.get("n"));
    }
    assert!(sleeps.iter().all(|&ms| ms == 50));
    assert_eq!(
        world
            .resource::<TaskGlobals>()
            .io_manager_b33a10
            .as_ref()
            .unwrap()
            .field_38,
        6,
        "{run} 004341B7"
    );
}

// ---- P16-RUN-003 ----

// X 4 / Y 7: Phase 15 loads the (4, 7) grid; the centre slot 00005FC9; no XMarker, heading or door; the first base of type 0x1C, 00060420, wins; the land 000074E4's height, 004C3030's block, 530.77, lifts z from 500.2; water 0.0.
#[test]
fn run_003_an_exterior_cell_whole() {
    let started = std::time::Instant::now();
    let rows = Rows::load();
    let run = run_sequence("", "4", "7");
    let world = &run.state.world;
    assert_eq!(run.initial.path, InitialAreaPath::CoordinatesCell);
    assert!(run.initial.tail);
    let cell = run.placed.cell.unwrap();
    assert_eq!(form_id(world, cell), 0x5FC9);
    assert!(run.placed.from_centre_slot);
    assert_eq!(rows.one("WB", RUN_3, "no_interior").get("at"), "40EEB1");
    let choice = run.placed.placement.unwrap();
    check_placement(&rows, RUN_3, world, &choice);
    assert!(!choice.door);
    let (land_height, water) = choice.lift.unwrap();
    let heights = rows.one("V", RUN_3, "heights");
    assert_eq!(
        land_height.to_bits(),
        heights.hex("height"),
        "RUN-003 530.77"
    );
    assert_eq!(land_height.to_bits(), 0x4404_B1B8);
    assert_eq!(water.to_bits(), heights.hex("water"), "RUN-003 water");
    let chosen_at = world
        .get::<TesObjectRefr>(choice.chosen.unwrap())
        .unwrap()
        .position_2c;
    assert_eq!(chosen_at[2].to_bits(), heights.hex("z"), "RUN-003 z 500.2");
    let lift = rows.one("V", RUN_3, "final_lift");
    assert_eq!(lift.get("pos"), floats_hex(&chosen_at), "before the store");
    let call = rows.one("V", RUN_3, "land_call");
    let land = world.get::<TesObjectCell>(cell).unwrap().land_40.unwrap();
    assert_eq!(form_id(world, land), call.hex("id"), "RUN-003 the LAND");
    assert_eq!(call.get("pos"), floats_hex(&chosen_at));
    let entries = rows.take("V", RUN_3, "height_entry");
    assert_eq!(entries[0].get("pos"), floats_hex(&chosen_at));
    // The second is under the drain, seq 493, from the player where the block left it.
    let player = world
        .resource::<oblivion_sim::tes::data_handler::FormsById>()
        .lookup(0x14)
        .unwrap();
    assert_eq!(
        entries[1].get("pos"),
        floats_hex(&world.get::<TesObjectRefr>(player).unwrap().position_2c),
        "RUN-003 the drain's height call"
    );
    // 004C3030's block, against the 0x4E bytes 004C44C0 receives.
    let mut block = LandHeightTriangle::default();
    assert!(land_find_height_triangle(world, land, &mut block, chosen_at, false).unwrap());
    let tri = rows.one("V", RUN_3, "tri");
    assert_eq!(
        triangle_hex(&block),
        &tri.get("block")[..0x4E * 2],
        "RUN-003 the block"
    );
    assert_eq!(
        land_interpolate_height(world, land, &block)
            .unwrap()
            .to_bits(),
        heights.hex("height")
    );
    assert_eq!(
        land_height_at_position(world, land, [0.0; 3]).unwrap(),
        (false, -2048.0),
        "004C5B6D"
    );
    check_blocks(&rows, RUN_3, true);
    assert_eq!(rows.one("END", RUN_3, "end_cell").seq, 606);
    let at = &run.snapshots;
    compare_win(&rows, RUN_3, "before", &at["before"], false);
    for tag in ["player_virtuals", "after_position", "drain", "end_cell"] {
        compare_win(&rows, RUN_3, tag, &at[tag], true);
    }
    compare_common(&rows, RUN_3, &run, 0x40BD_B1DE, &["b3c", "pos"], &["p5d0"]);
    compare_cell(&rows, RUN_3, "prepare", world, cell, false);
    compare_cell_later(&rows, RUN_3, world, cell);
    insert_checks(&rows, RUN_3, world, cell);
    let body = world
        .get::<PlayerCharacter>(the_player(world).unwrap())
        .unwrap();
    println!(
        "RUN-003: +0x6E8 {:X?}, +0x71E {}, +0x71F {}, +0x730 {:X?}, +0x6EC {} sounds",
        body.region_6e8.map(|region| form_id(world, region)),
        body.cell_fully_inside_border_region_71e,
        body.border_recovery_pending_71f,
        body.border_region_candidates_730.as_ref().map(|list| list
            .iter()
            .map(|&region| form_id(world, region))
            .collect::<Vec<_>>()),
        body.region_sound_entries_6ec.items().count(),
    );
    assert!(run.sleeps.is_empty() && run.timed_runs == 0);
    assert!(!run.placed.drain.as_ref().unwrap().waited);
    scripted_drain(&rows, RUN_3, &mut run_state_world(run.state), 5);
    rows.assert_read(RUN_3);
    println!("RUN-003 whole: {:?}", started.elapsed());
}

fn triangle_hex(block: &LandHeightTriangle) -> String {
    let mut bytes = Vec::new();
    for value in [
        block.dx_00,
        block.dy_04,
        block.quadrant_local_x_08,
        block.quadrant_local_y_0c,
    ] {
        bytes.extend(value.to_le_bytes());
    }
    for value in [block.quadrant_x_10, block.quadrant_y_14, block.quadrant_18] {
        bytes.extend(value.to_le_bytes());
    }
    for value in [block.vertex_local_x_1c, block.vertex_local_y_20] {
        bytes.extend(value.to_le_bytes());
    }
    for value in [block.column_24, block.row_28, block.quadrant_2c] {
        bytes.extend(value.to_le_bytes());
    }
    for value in block.vertex_30 {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(block.vertex_3c.to_le_bytes());
    for value in block.indices_40 {
        bytes.extend(value.to_le_bytes());
    }
    bytes.push(block.odd_cell_4c);
    bytes.push(block.far_triangle_4d);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// ---- the arms no path reaches ----

/// IDA: N/A a recorder for every arm, whose 004D5D70 answers RUN-002's values.
#[derive(Default)]
struct RecordingArms {
    calls: Vec<String>,
    rotation_z_at_insert: u32,
    position_at_insert: String,
}

impl PlacePlayerArms for RecordingArms {
    fn log(&mut self, text: &str) {
        self.calls.push(format!("00404EE0 {text}"));
    }

    fn prepare_placement_position(
        &mut self,
        world: &mut World,
        cell: Entity,
        rect: &mut [f32; 3],
        rotation: &mut [f32; 3],
    ) -> Result<PlacementChoice, GameError> {
        self.calls
            .push(format!("004D5D70 {:X}", form_id(world, cell)));
        *rect = [
            f32::from_bits(0x4536_01D6),
            f32::from_bits(0xC3B7_76C1),
            f32::from_bits(0x4458_0000),
        ];
        *rotation = [0.0, -0.0, f32::from_bits(0xBC09_A62B)];
        Ok(PlacementChoice::default())
    }

    fn insert_player(
        &mut self,
        world: &mut World,
        cell: Entity,
        player: Entity,
    ) -> Result<(), GameError> {
        let refr = world.get::<TesObjectRefr>(player).unwrap();
        self.rotation_z_at_insert = refr.rotation_20[2].to_bits();
        self.position_at_insert = floats_hex(&refr.position_2c);
        self.calls.push(format!(
            "004D35D0 {:X} {:X}",
            form_id(world, cell),
            form_id(world, player)
        ));
        Ok(())
    }

    fn drain_tasks_through_stage(
        &mut self,
        _world: &mut World,
        stage: i32,
    ) -> Result<IoDrainResult, GameError> {
        self.calls.push(format!("00434020 {stage}"));
        Ok(IoDrainResult::default())
    }
}

fn find_interior(world: &mut World, name: &[u8]) -> Entity {
    oblivion_sim::tes::data_handler_world::find_cell_by_editor_id(world, name)
        .unwrap()
        .unwrap()
}

// The block's order alone, the Z rotation's arms and the priorities, on one game.
#[test]
fn the_arms_on_the_loaded_data() {
    let (mut state, rect) = game_at_window();
    let GameWindow {
        world, collections, ..
    } = &mut state;
    let player = the_player(world).unwrap();
    // No cell: the log and 0040EF54, with no arm.
    let mut arms = RecordingArms::default();
    let mut at = rect;
    let result = run_place_player(world, &collections.ini, &mut arms, &mut at).unwrap();
    assert_eq!(arms.calls, ["00404EE0 Placing player..."]);
    assert!(result.reached_0040ef54);
    // +0x34 set: its cell, without 00482150.
    let cell = find_interior(world, b"XPAichan01");
    world
        .resource_mut::<TesGlobals>()
        .tes
        .as_deref_mut()
        .unwrap()
        .current_interior_cell_34 = Some(cell);
    let mut arms = RecordingArms::default();
    let mut at = rect;
    let result = run_place_player(world, &collections.ini, &mut arms, &mut at).unwrap();
    assert_eq!(
        arms.calls,
        [
            "00404EE0 Placing player...",
            "004D5D70 10360",
            "004D35D0 10360 14",
            "00434020 5"
        ]
    );
    assert!(!result.from_centre_slot && !result.reached_0040ef54);
    assert_eq!(arms.rotation_z_at_insert, 0x40C8_CB08, "RUN-002's angle");
    assert_eq!(arms.position_at_insert, "d6013645c176b7c300005844");
    world
        .resource_mut::<TesGlobals>()
        .tes
        .as_deref_mut()
        .unwrap()
        .current_interior_cell_34 = None;

    // 0065A450's wrap, run-backed where the name says so; the rest from the bytes.
    for (radians, stored, what) in [
        (0xBC09_A62B_u32, 0x40C8_CB08_u32, "RUN-002"),
        (0xBEB5_DFD3, 0x40BD_B1DE, "RUN-003"),
        (0x0000_0000, 0x0000_0000, "RUN-001"),
        (0x8000_0000, 0x8000_0000, "-0.0: not below 0, kept"),
        (0x40C9_0FDB, 0x40C9_0FDB, "2*pi itself: not above"),
        (0x40E0_0000, 0x3F37_8128, "7.0: fmod"),
        (0xC0E0_0000, 0xBF37_8128, "-7.0: the sum stays negative"),
        (0x7FC0_0001, 0x7FC0_0001, "NaN: 0065A530"),
        (0x7F80_0001, 0x7FC0_0001, "an sNaN loads quiet"),
    ] {
        actor_set_z_rotation_if_process_allows(world, player, f32::from_bits(radians)).unwrap();
        assert_eq!(
            world.get::<TesObjectRefr>(player).unwrap().rotation_20[2].to_bits(),
            stored,
            "{what}"
        );
    }
    // 005E0EA0's gate: 0 and 4 run, others don't.
    for (dead, runs) in [
        (1, false),
        (2, false),
        (3, false),
        (4, true),
        (5, false),
        (0, true),
    ] {
        world.get_mut::<Actor>(player).unwrap().dead_state_b0 = dead;
        world.get_mut::<TesObjectRefr>(player).unwrap().rotation_20[2] = 1.0;
        actor_set_z_rotation_if_process_allows(world, player, 2.0).unwrap();
        let z = world.get::<TesObjectRefr>(player).unwrap().rotation_20[2];
        assert_eq!(z, if runs { 2.0 } else { 1.0 }, "dead state {dead}");
    }
    // 005E04F0: a knocked state returns with no store; +0x11D's arm.
    let process = world
        .get::<MobileObject>(player)
        .unwrap()
        .process_58
        .unwrap();
    world.get_mut::<TesObjectRefr>(player).unwrap().rotation_20[2] = 1.0;
    world
        .get_mut::<MiddleHighProcess>(process)
        .unwrap()
        .knocked_state_11c = 0xFF;
    actor_set_z_rotation_if_process_allows(world, player, 2.0).unwrap();
    assert_eq!(
        world.get::<TesObjectRefr>(player).unwrap().rotation_20[2],
        1.0
    );
    world
        .get_mut::<MiddleHighProcess>(process)
        .unwrap()
        .knocked_state_11c = 0;
    for (state, stops) in [
        (3_u8, false),
        (4, true),
        (5, true),
        (6, false),
        (8, false),
        (9, true),
        (10, true),
        (11, false),
    ] {
        let mut data = world.get_mut::<MiddleHighProcess>(process).unwrap();
        data.sleep_state_11d = state;
        data.furniture_reference_120 = Some(cell);
        let result = actor_set_z_rotation_if_process_allows(world, player, 2.0);
        assert_eq!(result.is_err(), stops, "+0x11D {state}");
    }
    let mut data = world.get_mut::<MiddleHighProcess>(process).unwrap();
    data.sleep_state_11d = 4;
    data.furniture_reference_120 = None;
    actor_set_z_rotation_if_process_allows(world, player, 2.0).unwrap();
    let mut data = world.get_mut::<MiddleHighProcess>(process).unwrap();
    data.sleep_state_11d = 0;

    // 00440C80: an interior is 1, and the cache answers for it after that. An exterior by its distance.
    assert_eq!(cell_load_priority(world, cell, None, 5).unwrap(), 1);
    assert_eq!(
        cell_load_priority(world, cell, None, 5).unwrap(),
        1,
        "cached"
    );
}

// bDefaultCOCPlacement, read from the INI at the call, and the other grid cells' choices: the C++'s order check, over the real cells, and the last-reference fallback RUN-004 never ran.
#[test]
fn the_placement_arms_over_the_grid() {
    let run = run_sequence("", "4", "7");
    let mut state = run.state;
    let GameWindow {
        world,
        collections,
        harness,
        host,
    } = &mut state;
    let lod_land = collections.ini.find_address(0x00B0_2D70).unwrap().raw as u8;
    let mut warning = |_: String, _: bool| {};
    let mut load = DataLoadServices {
        working_directory: &harness.root,
        warning: &mut warning,
        host,
    };
    let mut level2 = |_: String| {};
    let mut refresh = |_: &mut World| Ok(false);
    let slots: Vec<Option<Entity>> = tes(world)
        .grid_cells_08
        .as_ref()
        .unwrap()
        .grid_10
        .iter()
        .map(|slot| slot.cell_00)
        .collect();
    let mut by_last = 0;
    let mut by_type = 0;
    let mut grid = TesGridServices {
        load: &mut load,
        ini: &collections.ini,
        answers: TesGridAnswers::default(),
        display_lod_land_b43077: lod_land,
        level2_log: &mut level2,
        refresh_map_menu: &mut refresh,
        interface: None,
        drain: None,
        reference_loop: false,
        game_sounds: None,
        archives: None,
        models: None,
    };
    for cell in slots.into_iter().flatten() {
        let (mut position, mut rotation) = ([0.0; 3], [0.0; 3]);
        let choice = oblivion_sim::tes::tes_grid::prepare_placement_position(
            world,
            cell,
            &mut position,
            &mut rotation,
            &mut grid,
        )
        .unwrap();
        let expected = choice
            .x_marker_heading
            .or(choice.x_marker)
            .or(if choice.door {
                None
            } else {
                choice.type_1c.or(choice.last)
            });
        assert_eq!(choice.chosen, expected, "the 004CF08C order");
        if expected == choice.last && choice.type_1c.is_none() {
            by_last += 1;
        }
        if expected == choice.type_1c {
            by_type += 1;
        }
        let (land, water) = choice.lift.expect("every grid cell has a land");
        assert!(position[2] >= land && position[2] >= water, "004CF141");
        // 004CF14E stores only below; otherwise z is the reference's.
        if let Some(chosen) = choice.chosen {
            let z = world.get::<TesObjectRefr>(chosen).unwrap().position_2c[2];
            let top = if water > land { water } else { land };
            assert_eq!(
                position[2].to_bits(),
                (if z < top { top } else { z }).to_bits(),
                "the lift"
            );
        }
    }
    assert!(by_type > 0);
    println!("the grid: {by_type} cells by type 0x1C, {by_last} by the last reference");
    // 00440C80 on an exterior: 5FC9 at (4, 7).
    let exterior = run_cell_of(&grid_center(world));
    let (x, y) = (4, 7);
    let at_grid = |gx: i32, gy: i32| {
        [
            gx.wrapping_shl(12) as f32 + 2048.0,
            gy.wrapping_shl(12) as f32 + 2048.0,
            0.0,
        ]
    };
    for (position, priority) in [
        (at_grid(x, y), 1),
        (at_grid(x - 1, y), 1),
        (at_grid(x, y + 1), 1),
        (at_grid(x - 2, y), 3),
        (at_grid(x, y + 2), 1),
        (at_grid(x + 2, y + 2), 3),
        (at_grid(x + 2, y), 1),
        (at_grid(x, y - 2), 3),
    ] {
        world
            .resource_mut::<oblivion_sim::tes::tes_grid::TesGridGlobals>()
            .load_priority_cell_b33a64 = None;
        assert_eq!(
            cell_load_priority(world, exterior, Some(position), 5).unwrap(),
            priority,
            "{position:?}"
        );
    }
    // Set, an exterior stops at the grid's center and rotation; an interior goes on.
    collections.ini.find_address_mut(0x00B0_97E0).unwrap().raw = 1;
    let mut grid = TesGridServices {
        load: &mut load,
        ini: &collections.ini,
        answers: TesGridAnswers::default(),
        display_lod_land_b43077: lod_land,
        level2_log: &mut level2,
        refresh_map_menu: &mut refresh,
        interface: None,
        drain: None,
        reference_loop: false,
        game_sounds: None,
        archives: None,
        models: None,
    };
    let (mut position, mut rotation) = ([9.0; 3], [9.0; 3]);
    let choice = oblivion_sim::tes::tes_grid::prepare_placement_position(
        world,
        exterior,
        &mut position,
        &mut rotation,
        &mut grid,
    )
    .unwrap();
    assert_eq!(choice, PlacementChoice::default());
    assert_eq!(position, [18432.0, 30720.0, 0.0], "(4 << 12) + 2048");
    assert_eq!(rotation, [0.0; 3]);
    let interior = find_interior(world, b"XPAichan01");
    let choice = oblivion_sim::tes::tes_grid::prepare_placement_position(
        world,
        interior,
        &mut position,
        &mut rotation,
        &mut grid,
    )
    .unwrap();
    assert!(choice.chosen.is_some(), "an interior ignores it");
    assert_eq!(choice.entry.0, [2048.0, 2048.0, 0.0], "interior grid 0, 0");
}

fn grid_center(world: &World) -> Option<Entity> {
    tes(world).grid_cells_08.as_ref().unwrap().grid_10[12].cell_00
}

fn run_cell_of(cell: &Option<Entity>) -> Entity {
    cell.expect("the centre slot")
}

// ---- x87 ----

// Every float a row holds that 24-bit and 53-bit forms could split: Z rotation, the land block's and height's steps, the percents. The 24-bit form matches all, or a row test above fails; this counts where the 53-bit form misses.
#[test]
fn the_rows_against_the_53_bit_forms() {
    let rows = Rows::load();
    let two_pi = 6.283_185_482_025_146_5_f64;
    let mut compared = 0;
    let mut differs = 0;
    let mut tally = |form_53: u32, recorded: u32, what: &str| {
        compared += 1;
        if form_53 != recorded {
            differs += 1;
            println!("{what}: 53-bit {form_53:08X}, run {recorded:08X}");
        }
    };
    // Z rotation.
    for run in [RUN_2, RUN_3] {
        let radians = f32::from_bits(rows.one("V", run, "zrot").hex("arg"));
        let sum = (f64::from(radians) + two_pi) as f32;
        let form = (f64::from(sum) % two_pi) as f32;
        tally(
            form.to_bits(),
            rows.one("V", run, "set_rz").hex("arg"),
            "0065A4E1",
        );
    }
    // The block's floats, RUN-003: `land_find_height_triangle`'s steps with `f64` between stores.
    let tri = rows.one("V", RUN_3, "tri");
    let raw = hex_bytes(tri.get("block"));
    let float_at = |at: usize| f32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
    let int_at = |at: usize| i32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
    let position = floats_of(rows.one("V", RUN_3, "land_call").get("pos"));
    let (x, y) = (position[0], position[1]);
    let (ox, oy) = (4_f64 * 4096.0, 7_f64 * 4096.0);
    tally(
        ((f64::from(x) - ox) as f32).to_bits(),
        float_at(0).to_bits(),
        "+0x00",
    );
    tally(
        ((f64::from(y) - oy) as f32).to_bits(),
        float_at(4).to_bits(),
        "+0x04",
    );
    let qx = ox + f64::from(int_at(0x10)) * 2048.0;
    let qy = oy + f64::from(int_at(0x14)) * 2048.0;
    tally(
        ((f64::from(x) - qx) as f32).to_bits(),
        float_at(8).to_bits(),
        "+0x08",
    );
    tally(
        ((f64::from(y) - qy) as f32).to_bits(),
        float_at(0xC).to_bits(),
        "+0x0C",
    );
    let vx = qx + f64::from(int_at(0x24)) * 128.0;
    let vy = qy + f64::from(int_at(0x28)) * 128.0;
    tally(
        ((f64::from(x) - vx) as f32).to_bits(),
        float_at(0x1C).to_bits(),
        "+0x1C",
    );
    tally(
        ((f64::from(y) - vy) as f32).to_bits(),
        float_at(0x20).to_bits(),
        "+0x20",
    );
    // The height needs the land: a game, once.
    let run = run_sequence("", "4", "7");
    let world = &run.state.world;
    let cell = run.placed.cell.unwrap();
    let land = world.get::<TesObjectCell>(cell).unwrap().land_40.unwrap();
    let mut block = LandHeightTriangle::default();
    let chosen = run.placed.placement.unwrap().chosen.unwrap();
    let at = world.get::<TesObjectRefr>(chosen).unwrap().position_2c;
    land_find_height_triangle(world, land, &mut block, at, false).unwrap();
    let data = world
        .get::<oblivion_sim::tes::forms_world::TesObjectLand>(land)
        .unwrap()
        .data_24
        .as_ref()
        .unwrap();
    let vertices = &data.vertices_04[block.quadrant_18 as usize];
    let z = |k: usize| f64::from(vertices[block.indices_40[k] as usize][2]);
    let (fx, fy) = (
        f64::from(block.vertex_local_x_1c),
        f64::from(block.vertex_local_y_20),
    );
    let inverse = 0.0078125;
    let form = if block.odd_cell_4c == 0 {
        let a = f64::from(((z(0) - z(1)) * inverse) as f32);
        let b = f64::from(((z(2) - z(0)) * inverse) as f32);
        if block.far_triangle_4d == 0 {
            (z(1) + fx * a) + fy * b
        } else {
            (z(1) + fy * a) + fx * b
        }
    } else if block.far_triangle_4d != 0 {
        let c = f64::from(((z(0) - z(2)) * inverse) as f32);
        let t = (128.0 - fx) * c;
        let d = f64::from(((z(0) - z(1)) * inverse) as f32);
        let u = (128.0 - fy) * d;
        z(0) - (u + t)
    } else {
        let e = f64::from(((z(1) - z(0)) * inverse) as f32);
        let p = e * fx + z(0);
        let f = f64::from(((z(2) - z(0)) * inverse) as f32);
        p + f * fy
    };
    let height = rows.one("V", RUN_3, "heights").hex("height");
    tally((form as f32).to_bits(), height, "004C44C0 530.77");
    assert_eq!(height, 0x4404_B1B8);
    // The percents.
    for run in [RUN_2, RUN_3] {
        let total = rows.one("V", run, "d_total").hex("ecx");
        let mut remaining = total;
        let loops = rows.take("V", run, "d_loop");
        for (row, next) in rows.take("V", run, "d_progress").iter().zip(&loops) {
            let form = f64::from(total - remaining) / f64::from(total as f32) * 100.0;
            tally((form as f32).to_bits(), row.hex("s1"), "00434108");
            assert_eq!(io_drain_percent(total, remaining).to_bits(), row.hex("s1"));
            remaining = next.hex("eax");
        }
    }
    println!("x87: {compared} row values; the 53-bit form misses {differs}");
}

// ---- G2: the driver's own World ----

// IDA: N/A the pinned folder with the starting settings of an INI path, as `winmain_driver`'s, in a scratch folder.
fn driver_folder(tag: &str, cell: &str, x: &str, y: &str) -> std::path::PathBuf {
    let folder = std::env::temp_dir().join(format!("recomp_g2_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let config = fixtures::root().join("config");
    for entry in std::fs::read_dir(&config).unwrap() {
        let path = entry.unwrap().path();
        std::fs::write(
            folder.join(path.file_name().unwrap()),
            std::fs::read(&path).unwrap(),
        )
        .unwrap();
    }
    let text: String = std::fs::read(config.join("Oblivion.ini"))
        .unwrap()
        .iter()
        .map(|&byte| char::from(byte))
        .collect();
    let mut out = String::new();
    for line in text.lines() {
        out.push_str(line);
        if line.trim().eq_ignore_ascii_case("[General]") {
            out.push_str(&format!(
                "\r\nsStartingCell={cell}\r\nsStartingCellX={x}\r\nsStartingCellY={y}"
            ));
        }
        out.push_str("\r\n");
    }
    let bytes: Vec<u8> = out.chars().map(|c| c as u8).collect();
    std::fs::write(folder.join("Oblivion.ini"), bytes).unwrap();
    folder
}

// IDA: N/A the seed whose first-use draw leaves the CRT 191 draws before `RUN_004_ROLL_START`, as `winmain_driver`'s rand test: the harness's rolls on the driver's CRT.
fn driver_seed() -> u32 {
    const INVERSE: u32 = 0xB9B3_3155; // 214013's inverse, mod 2^32.
    let previous = |state: u32| state.wrapping_sub(2_531_011).wrapping_mul(INVERSE);
    let mut state = RUN_004_ROLL_START;
    for _ in 0..191 {
        state = previous(state);
    }
    previous(state)
}

/// IDA: N/A G2's `the driver isn't what phase16_place_player runs through`: each path through `oblivion_sim::tes::winmain` on its own World, snapshot at the block's start and end, bit for bit against the harness's sequence and against the rows at the same two places.
#[test]
fn the_driver_ends_where_the_harness_ends() {
    use oblivion_sim::tes::winmain::{
        Block, SettingsSource, WinMainConfig, WinMainLocals, WinMainProgress, WinMainSeams,
        install_winmain, winmain_schedule, winmain_shutdown,
    };
    let rows = Rows::load();
    let seed = driver_seed();
    for (run, cell, x, y, end_tag, owned, owned_x) in [
        (RUN_1, "", "", "", "end_nocell", &[][..], &[][..]),
        (
            RUN_2,
            "XPAichan01",
            "",
            "",
            "end_cell",
            &["b3c"][..],
            &["p5d0"][..],
        ),
        (
            RUN_3,
            "",
            "4",
            "7",
            "end_cell",
            &["b3c", "pos"][..],
            &["p5d0"][..],
        ),
    ] {
        let harness = run_sequence(cell, x, y);
        let folder = driver_folder(&run.replace('-', "_"), cell, x, y);
        let mut world = World::new();
        install_winmain(
            &mut world,
            WinMainConfig {
                data_directory: fixtures::data_dir(),
                settings: SettingsSource::Pinned(folder.join("Oblivion.ini")),
                seed: Some(seed),
                frame_budget: Some(std::time::Duration::ZERO),
            },
            WinMainSeams::default(),
        );
        let mut schedule = winmain_schedule();
        let mut before = None;
        // Phase 16's end: the next block is Phase 17's, which stops on a cell.
        while world.resource::<WinMainProgress>().next != Block::PlayerScene {
            if world.resource::<WinMainProgress>().next == Block::PlacePlayer && before.is_none() {
                let rect = world.non_send::<WinMainLocals>().rect();
                before = Some(snapshot(&world, rect, [0.0; 3]));
            }
            schedule.run(&mut world);
        }
        let progress = world.resource::<WinMainProgress>();
        assert!(progress.stops.is_empty(), "{run}: {:?}", progress.stops);
        let rect = world.non_send::<WinMainLocals>().rect();
        let mut end = snapshot(&world, rect, [0.0; 3]);
        // `TES.coc` is the INI's word, which the driver's collections keep private; the sequence reads the same file.
        let coc = harness.snapshots[end_tag].values["TES.coc"].clone();
        end.values.insert("TES.coc".to_owned(), coc);
        let before = before.unwrap();
        let without_coc = |snap: &Snapshot| {
            let mut values = snap.values.clone();
            values.remove("TES.coc");
            values
        };
        // Bit for bit: the two sequences hold the same words.
        assert_eq!(
            without_coc(&before),
            without_coc(&harness.snapshots["before"]),
            "{run} before"
        );
        assert_eq!(
            without_coc(&end),
            without_coc(&harness.snapshots[end_tag]),
            "{run} end"
        );
        assert_eq!(floats_hex(&rect), floats_hex(&harness.rect), "{run} Rect");
        assert_eq!(
            i64::from(end.change_count) - i64::from(before.change_count),
            i64::from(harness.snapshots[end_tag].change_count)
                - i64::from(harness.snapshots["before"].change_count),
            "{run} new change records"
        );
        // The change maps: the same keys and flags, but for what the rolls move. The harness keeps an adapter's rand for the actors where the driver has one CRT, so a cell path's leveled rolls differ: only bit 0x8000000 of a key, and whether that key exists because of it.
        let walk = |world: &World| -> BTreeMap<u32, u32> {
            world
                .resource::<TesGlobals>()
                .save_load_game
                .as_ref()
                .unwrap()
                .changes_00
                .walk()
                .map(|(id, record)| (id, record.flags_00))
                .collect()
        };
        let (driver, sequence) = (walk(&world), walk(&harness.state.world));
        let mut moved = 0;
        for id in driver.keys().chain(sequence.keys()) {
            let (one, other) = (driver.get(id), sequence.get(id));
            if one != other {
                let differ = one.copied().unwrap_or(0) ^ other.copied().unwrap_or(0);
                assert_eq!(differ, 0x800_0000, "{run} key {id:X}: {one:X?} {other:X?}");
                moved += 1;
            }
        }
        if run == RUN_1 {
            assert_eq!(moved, 0, "the default path enters no cell");
        }
        println!(
            "{run}: driver against the sequence: {moved} keys moved by the rolls, counted twice when one side lacks a key's other side"
        );
        // And the rows, on the driver's own values.
        assert_eq!(
            floats_hex(&rect),
            rows.one("WIN", run, end_tag).get("rect"),
            "{run} WIN"
        );
        for (kind, skip) in [
            ("TES", &[][..]),
            ("TL", &[][..]),
            ("PLAYERX", owned_x),
            ("CTRL", &[][..]),
            ("PLAYER", owned),
        ] {
            compare_kind(&rows, run, kind, end_tag, &end, skip);
        }
        winmain_shutdown(&mut world);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
