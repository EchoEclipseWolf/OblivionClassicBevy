//! The WinMain driver on one World, in process, with the pinned config: G3's lines against `fixtures/cpp-baseline`, the same chain frame by frame, the rows a run recorded on the World the driver leaves, and the failure paths. IDA: N/A test harness. Host tests earn no validation bonus.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use bevy_ecs::prelude::*;
use common::act_rows::{RUN_004_ROLL_START, Tally, compare_actor};
use common::rows::RowBook;
use recomp_core::crt::MsvcRand;
use recomp_core::fixtures;
use recomp_game::bs::model_loader::{model_map_bucket, model_tree_rows};
use recomp_game::bs::task_manager::TaskGlobals;
use recomp_game::gamebryo::controller_sequence::{ControllerSequence, SequenceBlendRef};
use recomp_game::tes::data_handler::FormsById;
use recomp_game::tes::tes::GameThreadLocals;
use recomp_startup::winmain::{
    Block, SettingsSource, WinMainConfig, WinMainProgress, WinMainSeams, archive_buffer_limit,
    install_winmain, io_thread_holdrand, main_thread_crt_state, run_winmain_straight_through,
    winmain_schedule, winmain_shutdown,
};

fn pinned_ini() -> PathBuf {
    fixtures::root().join("config/Oblivion.ini")
}

fn config(data: &Path, seed: Option<u32>, budget: Option<Duration>) -> WinMainConfig {
    WinMainConfig {
        data_directory: data.to_path_buf(),
        settings: SettingsSource::Pinned(pinned_ini()),
        seed,
        frame_budget: budget,
    }
}

fn real_data() -> PathBuf {
    let data = fixtures::data_dir();
    assert!(
        data.join("Oblivion.esm").exists(),
        "this test needs the game data: set OBLIVION_DATA_DIR ({data:?})"
    );
    data
}

fn run(data: &Path, seed: Option<u32>) -> World {
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(data, seed, None),
        WinMainSeams::default(),
    );
    run_winmain_straight_through(&mut world);
    winmain_shutdown(&mut world);
    world
}

fn lines(world: &World) -> Vec<String> {
    world.resource::<WinMainProgress>().lines.clone()
}

/// IDA: N/A the `[INFO] Phase-NN:` lines of one test in a `LastTest` log, in order, between its header and its `<end of output>`.
fn baseline_lines(log: &str, test: &str) -> Vec<String> {
    let path = fixtures::root().join("cpp-baseline").join(log);
    let bytes = std::fs::read(path).unwrap();
    let text: String = bytes.iter().map(|&byte| char::from(byte)).collect();
    let start = text
        .find(&format!("Testing: {test}\n"))
        .or_else(|| text.find(&format!("Testing: {test}\r\n")))
        .unwrap();
    let body = &text[start..];
    let end = body.find("<end of output>").unwrap();
    body[..end]
        .lines()
        .filter_map(|line| line.trim_end().strip_prefix("[INFO] "))
        .filter(|line| line.starts_with("Phase-"))
        .map(str::to_owned)
        .collect()
}

/// IDA: N/A G3's declared tolerances: each field's value goes, and its value is checked apart where a value can be.
/// - `Bink sound`: P06-RUN-001 and P06-RUN-002 record eax = 1 at 0040EADB; the C++ x64 FFmpeg host answers 0.
/// - `editor IDs`: C++ 7939 (x64), 7938 (x86), where P12-RUN-001 and this port give 7896: `the_editor_id_map_matches_the_original_chain_by_chain`.
/// - `item scripts not executed`: 12d's rolls.
/// - `doors not registered`: the C++ counts every door reference; 00680090's gates (006800B4, 004D7630) count fewer.
/// - `change records`: 12d's and 14c's rolls.
/// - `view`: the C++ host's own LoadingView, racing the movie thread: x64 says pending, x86 adopted. The screens agent's.
fn tolerant(line: &str) -> String {
    // The C++ stops at 11 of these models, and the original builds all 26 (`the_driver_builds_the_recorded_model_trees`).
    if line.starts_with("Phase-13e:") {
        return "Phase-13e: ~".to_owned();
    }
    let mut out = line.to_owned();
    for key in [
        "Bink sound ",
        "editor IDs ",
        "item scripts not executed ",
        "doors not registered ",
        "change records ",
        "view ",
    ] {
        if let Some(at) = out.find(key) {
            let start = at + key.len();
            let end = out[start..]
                .find([',', ' '])
                .map_or(out.len(), |length| start + length);
            out.replace_range(start..end, "~");
        }
    }
    out
}

fn value_of(lines: &[String], key: &str) -> u32 {
    let line = lines
        .iter()
        .find(|line| line.contains(key))
        .unwrap_or_else(|| panic!("no line with {key}"));
    let start = line.find(key).unwrap() + key.len();
    line[start..]
        .split([',', ' '])
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

fn assert_matches_baseline(ours: &[String], baseline: &[String]) {
    let ours: Vec<String> = ours.iter().map(|line| tolerant(line)).collect();
    let baseline: Vec<String> = baseline.iter().map(|line| tolerant(line)).collect();
    assert_eq!(ours, baseline);
}

// G3: every `Phase-NN:` line main.cpp printed at its level, in the same order, on both of the C++ logs.
#[test]
fn g3_the_phase_lines_match_the_baseline() {
    let world = run(&real_data(), Some(1));
    let ours = lines(&world);
    assert!(world.resource::<WinMainProgress>().stops.is_empty());
    assert_eq!(ours.len(), 12, "{ours:#?}");
    for log in ["LastTest-x64.log", "LastTest-x86.log"] {
        assert_matches_baseline(&ours, &baseline_lines(log, "app_startup"));
    }
    assert_eq!(value_of(&ours, "Bink sound "), 1, "P06-RUN-001");
    assert_eq!(value_of(&ours, "editor IDs "), 7896, "P12-RUN-001");
    assert_eq!(value_of(&ours, "doors not registered "), 4567);
    // Seed 1's rolls, after 005AD440's 191 draws in the load: a value of this port, no run's.
    assert_eq!(value_of(&ours, "item scripts not executed "), 1343);
    assert_eq!(value_of(&ours, "change records "), 5497);
}

// A seed fixes the whole run; another seed moves only the rolls' two fields.
#[test]
fn a_seed_fixes_the_run_and_moves_only_the_rolls() {
    let data = real_data();
    let first = lines(&run(&data, Some(0x6000_0000)));
    assert_eq!(first, lines(&run(&data, Some(0x6000_0000))));
    let other = lines(&run(&data, Some(0x6000_0001)));
    let strip =
        |lines: &[String]| -> Vec<String> { lines.iter().map(|line| tolerant(line)).collect() };
    assert_eq!(strip(&first), strip(&other));
}

// The plan's "long blocks are resumable": with a zero budget a frame runs one block or one step, and game-visible order doesn't change.
#[test]
fn a_frame_budget_changes_only_the_frame_boundaries() {
    let data = real_data();
    let straight = lines(&run(&data, Some(7)));
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&data, Some(7), Some(Duration::ZERO)),
        WinMainSeams::default(),
    );
    let mut schedule = winmain_schedule();
    let mut frames = 0_u32;
    while !world.resource::<WinMainProgress>().finished() {
        schedule.run(&mut world);
        frames += 1;
    }
    winmain_shutdown(&mut world);
    assert_eq!(lines(&world), straight);
    // Every record of 0044F0C0 and more: one step per frame.
    assert!(frames > 114_808, "{frames} frames");
    let order: Vec<Block> = world
        .resource::<WinMainProgress>()
        .block_times
        .iter()
        .map(|(block, _)| *block)
        .collect();
    assert_eq!(order.len(), 23, "{order:?}");
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "original order: {order:?}"
    );
}

// IDA: N/A the seed whose first draw (0047DF80's srand then rand, at the player's 0067B3C4) leaves `holdrand`: the LCG's inverse.
fn seed_before(holdrand: u32) -> u32 {
    // 214013's inverse modulo 2^32, by Newton.
    let mut inverse = 214_013_u32;
    for _ in 0..5 {
        inverse = inverse.wrapping_mul(2_u32.wrapping_sub(214_013_u32.wrapping_mul(inverse)));
    }
    holdrand.wrapping_sub(2_531_011).wrapping_mul(inverse)
}

// Original runs under the Linux debug setup (run-a4-audit, 2026-11-06, docs/original-debug.md): 0047DFAE's seed and
// `_holdrand` at 0040F16A, on the default start with the pinned INI. Same seed, same data, so the same draws.
#[test]
fn the_port_reaches_the_original_rand_state_from_its_seed() {
    let data = real_data();
    for (seed, end, count) in [
        (0x6AED_D567_u32, 0x4226_A472_u32, 22_725_u64), // run-a4-audit
        (0x6AED_D89C, 0xF134_B6A4, 22_472),             // run-b1-snap
        (0x6AED_DD68, 0x773B_1816, 22_422),             // run-c1-log
    ] {
        let mut world = World::new();
        install_winmain(
            &mut world,
            config(&data, Some(seed), None),
            WinMainSeams::default(),
        );
        run_winmain_straight_through(&mut world);
        let (holdrand, draws) = main_thread_crt_state(&world).unwrap();
        let io = io_thread_holdrand(&world).unwrap();
        // restoration.NIF's draws come at link, before its post-load, so the rand alone doesn't show it loaded.
        let loader = world.resource::<TaskGlobals>().model_loader_b33a1c.as_ref();
        assert!(
            loader.unwrap().models.store.iter().any(|model| model
                .path_00
                .to_ascii_lowercase()
                .ends_with(b"restoration.nif")
                && model.root_08.is_some()),
            "seed {seed:08X}"
        );
        // The file itself: four NiControllerSequence blocks of 49 items, each with an interpolator, so 196 to bind.
        let mut items = (0, 0);
        for sequence in world
            .query::<&ControllerSequence>()
            .iter(&world)
            .filter(|sequence| {
                sequence
                    .name_08
                    .as_deref()
                    .is_some_and(|name| name.starts_with(b"SpecialIdle_"))
            })
        {
            items.0 += 1;
            items.1 += sequence
                .items_14
                .iter()
                .filter(|item| item.blend_08 != SequenceBlendRef::None)
                .count();
        }
        assert_eq!(items, (4, 196), "seed {seed:08X}");
        winmain_shutdown(&mut world);
        assert_eq!((holdrand, draws), (end, count), "seed {seed:08X}");
        // run-d-models: the ten fire models' 32 draws, from the thread's own 1, by 0040EBB9, then restoration.NIF's 6 after 0040F045.
        let mut expected = MsvcRand::default();
        for _ in 0..38 {
            let _ = expected.rand();
        }
        assert_eq!(io, expected.holdrand(), "seed {seed:08X}");
    }
}

// IDA: N/A the World at the end of Phase 16 as text, for `tools/original_debug/ledger_compare.py`, from the seed of a memory
// snapshot. Only when asked: it writes a file.
#[test]
#[ignore = "writes $LEDGER_DUMP for ledger_compare.py"]
fn dump_for_the_ledger_compare() {
    let seed = u32::from_str_radix(&std::env::var("LEDGER_SEED").unwrap(), 16).unwrap();
    std::fs::write(std::env::var("LEDGER_DUMP").unwrap(), ledger_dump(seed)).unwrap();
}

// snapshot-phase16_end of run-b1-snap, seed 6AEDD89C: `ledger_compare.py` and `ledger_lists.py` found each of these sections
// the same on both sides on 2026-11-07, with the script VM in and ExtraAction no longer set aside. These hold that result: 103,510
// forms, 35,042 references, 5,475 change records, 25 lists, 2,133 cells, 85 worlds, 55,110 extras, 2,284 door nodes. Only the port
// runs here, so a change means compare against the snapshot again.
#[test]
fn the_port_still_matches_the_same_seed_snapshot() {
    let dump = ledger_dump(0x6AED_D89C);
    let mut sections = std::collections::BTreeMap::<&str, (usize, u32)>::new();
    for line in dump.lines() {
        let kind = line.split(' ').next().unwrap();
        let entry = sections.entry(kind).or_insert((0, 0x811C_9DC5));
        entry.0 += 1;
        for &byte in line.as_bytes() {
            entry.1 = (entry.1 ^ u32::from(byte)).wrapping_mul(0x0100_0193);
        }
    }
    for (kind, count, hash) in LEDGER_SECTIONS {
        assert_eq!(sections[kind], (count, hash), "section {kind}");
    }
}

const LEDGER_SECTIONS: [(&str, usize, u32); 8] = [
    ("FORM", 103_510, 0x0B46_FE9F),
    ("REF", 35_042, 0x7838_1ED6),
    ("CHG", 5_475, 0x3BC8_ED56),
    ("LIST", 25, 0x6F54_F2CA),
    ("XTRA", 55_110, 0xFC4A_5F5E),
    ("CELL", 2_133, 0x05CA_F241),
    ("WRLD", 85, 0xD153_C9E0),
    ("DOOR", 2_284, 0x2778_DCD7),
];

fn ledger_dump(seed: u32) -> String {
    use recomp_game::tes::actor::TesObjectRefr;
    use recomp_game::tes::form::FormIdentity;
    use std::fmt::Write as _;
    // Phase 16's end, as the snapshot: Phase 18 adds the player's base to the change records.
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&real_data(), Some(seed), Some(Duration::ZERO)),
        WinMainSeams::default(),
    );
    let mut schedule = winmain_schedule();
    while world.resource::<WinMainProgress>().next != Block::PlayerScene {
        schedule.run(&mut world);
    }
    winmain_shutdown(&mut world);
    let id_of = |entity: Option<Entity>| {
        entity.map_or(0, |entity| {
            world.get::<FormIdentity>(entity).unwrap().form_id_0c
        })
    };
    let mut out = String::new();
    let forms: Vec<(u32, Entity)> = world.resource::<FormsById>().entries().collect();
    for (id, form) in forms {
        let identity = world.get::<FormIdentity>(form).unwrap();
        writeln!(
            out,
            "FORM {id:X} {:X} {:X}",
            identity.form_type_04, identity.flags_08
        )
        .unwrap();
        if let Some(refr) = world.get::<TesObjectRefr>(form) {
            let bits = |values: &[f32]| {
                values
                    .iter()
                    .map(|value| format!("{:08X}", value.to_bits()))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            writeln!(
                out,
                "REF {id:X} {:X} {:X} {} {} {:08X} {}",
                id_of(refr.base_form_1c),
                id_of(refr.parent_cell_40),
                bits(&refr.position_2c),
                bits(&refr.rotation_20),
                refr.scale_38.to_bits(),
                refr.extra_data_44
                    .entries
                    .iter()
                    .map(|entry| format!("{:X}", entry.type_04))
                    .collect::<Vec<_>>()
                    .join(","),
            )
            .unwrap();
        }
    }
    let game = world
        .resource::<recomp_game::tes::tes::TesGlobals>()
        .save_load_game
        .as_ref()
        .unwrap();
    for (id, record) in game.changes_00.walk() {
        writeln!(
            out,
            "CHG {id:X} {:X} {:X}",
            record.flags_00, record.block_04
        )
        .unwrap();
    }
    let lists = world.resource::<recomp_game::tes::data_handler::DataHandlerLists>();
    for offset in (0x04..=0xBC).step_by(8).chain(std::iter::once(0x8B8)) {
        if let Some(list) = lists.list_at(offset) {
            let ids: Vec<String> = list
                .items()
                .map(|form| format!("{:X}", id_of(Some(form))))
                .collect();
            writeln!(out, "LIST {offset:X} {}", ids.join(",")).unwrap();
        }
    }
    for entry in world
        .resource::<recomp_game::tes::data_handler::EditorIdMap>()
        .walk()
    {
        writeln!(
            out,
            "EDID {} {:X}",
            String::from_utf8_lossy(&entry.key)
                .trim_end_matches('\0')
                .replace(' ', "_"),
            id_of(Some(entry.form))
        )
        .unwrap();
    }
    let extra = |out: &mut String,
                 owner: u32,
                 list: &recomp_game::tes::base_extra_list::ExtraDataList| {
        use recomp_game::tes::base_extra_list::ExtraData as X;
        for entry in &list.entries {
            let value = match (&entry.resolved_0c, &entry.data) {
                (Some(form), _) => format!("R{:X}", id_of(*form)),
                (None, X::Dword(v)) => format!("{v:X}"),
                (None, X::Float(v)) => format!("{v:X}"),
                (None, X::Byte(v)) => format!("{v:X}"),
                (None, X::Word(v)) => format!("{v:X}"),
                (None, X::Cell(c)) => format!("R{:X}", id_of(Some(*c))),
                (None, X::Lod(v)) => format!("{:X},{:X},{:X}", v[0], v[1], v[2]),
                (None, X::Lock(v)) => v.iter().map(|b| format!("{b:02X}")).collect(),
                (None, X::Teleport(v)) => v
                    .iter()
                    .skip(1)
                    .map(|w| format!("{w:X}"))
                    .collect::<Vec<_>>()
                    .join(","),
                (
                    None,
                    X::StartingPosition {
                        position_0c,
                        rotation_18,
                    },
                ) => position_0c
                    .iter()
                    .chain(rotation_18)
                    .map(|w| format!("{w:X}"))
                    .collect::<Vec<_>>()
                    .join(","),
                (None, X::LeveledItem { index_0c, byte_10 }) => format!("{index_0c:X},{byte_10:X}"),
                (
                    None,
                    X::EnableStateParent {
                        parent_0c,
                        flags_10,
                    },
                ) => format!("{parent_0c:X},{flags_10:X}"),
                (None, X::ContainerChanges(block)) => block
                    .entries_00
                    .iter()
                    .map(|e| {
                        format!(
                            "{:X}:{}:{}",
                            id_of(Some(e.form_08)),
                            e.count_04,
                            e.lists_00.as_ref().map_or(0, Vec::len)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(","),
                (None, X::EnableStateChildren(v)) => v
                    .iter()
                    .map(|f| format!("{:X}", id_of(Some(*f))))
                    .collect::<Vec<_>>()
                    .join(","),
                (None, X::Followers(v)) => v
                    .iter()
                    .map(|f| format!("{:X}", id_of(Some(*f))))
                    .collect::<Vec<_>>()
                    .join(","),
                _ => "?".to_owned(),
            };
            writeln!(out, "XTRA {owner:X} {:X} {value}", entry.type_04).unwrap();
        }
    };
    let forms: Vec<(u32, Entity)> = world.resource::<FormsById>().entries().collect();
    for (id, form) in forms {
        if let Some(refr) = world.get::<TesObjectRefr>(form) {
            extra(&mut out, id, &refr.extra_data_44);
        }
        if let Some(cell) = world.get::<recomp_game::tes::forms_world::TesObjectCell>(form) {
            extra(&mut out, id, &cell.extra_28);
            let objects: Vec<String> = cell
                .objects_48
                .items()
                .map(|r| format!("{:X}", id_of(Some(r))))
                .collect();
            writeln!(
                out,
                "CELL {id:X} {:X} {:X} {:X} {:X} {:X} {:X} {} {}",
                cell.flags_24,
                cell.field_25,
                cell.process_level_26,
                id_of(cell.land_40),
                id_of(cell.path_grid_44),
                id_of(cell.world_50),
                cell.data_3c.map_or("-".to_owned(), |d| d
                    .iter()
                    .map(|w| format!("{w:X}"))
                    .collect::<Vec<_>>()
                    .join(",")),
                objects.join(",")
            )
            .unwrap();
        }
        if let Some(space) = world.get::<recomp_game::tes::forms_world::TesWorldSpace>(form) {
            let mut grid: Vec<String> = Vec::new();
            for chain in &space.references_64.buckets_08 {
                for (key, list) in chain {
                    let ids: Vec<String> = list
                        .items()
                        .map(|r| format!("{:X}", id_of(Some(r))))
                        .collect();
                    grid.push(format!("{key:X}={}", ids.join("+")));
                }
            }
            grid.sort();
            let unsorted: Vec<String> = space
                .unsorted_74
                .items()
                .map(|r| format!("{:X}", id_of(Some(r))))
                .collect();
            writeln!(
                out,
                "WRLD {id:X} {:X} {:X} {:X} {} {}",
                id_of(space.persistent_cell_34),
                id_of(space.road_54),
                space.cells_30.count_0c,
                unsorted.join(","),
                grid.join(";")
            )
            .unwrap();
        }
    }
    if let Some(doors) = world.get_resource::<recomp_game::tes::pathing::PathingDoorNodes>() {
        for node in &doors.nodes {
            writeln!(
                out,
                "DOOR {:X} {:X} {:X} {:X}",
                id_of(Some(node.door_04)),
                id_of(node.world_08),
                id_of(Some(node.linked_0c)),
                id_of(node.world_10)
            )
            .unwrap();
        }
    }
    let mut world_ref = world;
    let forms: Vec<(u32, Entity)> = world_ref.resource::<FormsById>().entries().collect();
    for (id, form) in forms {
        writeln!(out, "ENT {form:?} {id:X}").unwrap();
    }
    dump_component::<recomp_game::tes::actor::MobileObject>(
        &mut world_ref,
        "MobileObject",
        &mut out,
    );
    dump_component::<recomp_game::tes::actor::Actor>(&mut world_ref, "Actor", &mut out);
    dump_component::<recomp_game::tes::actor::Character>(&mut world_ref, "Character", &mut out);
    dump_component::<recomp_game::tes::actor::Creature>(&mut world_ref, "Creature", &mut out);
    dump_component::<recomp_game::tes::data_handler::TesSkill>(
        &mut world_ref,
        "TesSkill",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_region::TesRegion>(
        &mut world_ref,
        "TesRegion",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_misc::TesClass>(&mut world_ref, "TesClass", &mut out);
    dump_component::<recomp_game::tes::forms_misc::TesFaction>(
        &mut world_ref,
        "TesFaction",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_misc::TesHair>(&mut world_ref, "TesHair", &mut out);
    dump_component::<recomp_game::tes::forms_misc::TesEyes>(&mut world_ref, "TesEyes", &mut out);
    dump_component::<recomp_game::tes::forms_misc::TesSound>(&mut world_ref, "TesSound", &mut out);
    dump_component::<recomp_game::tes::forms_misc::TesLandTexture>(
        &mut world_ref,
        "TesLandTexture",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_environment::TesWeather>(
        &mut world_ref,
        "TesWeather",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_environment::TesClimate>(
        &mut world_ref,
        "TesClimate",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_environment::TesLoadScreen>(
        &mut world_ref,
        "TesLoadScreen",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_environment::TesObjectAnio>(
        &mut world_ref,
        "TesObjectAnio",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_environment::TesWaterForm>(
        &mut world_ref,
        "TesWaterForm",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_environment::TesEffectShader>(
        &mut world_ref,
        "TesEffectShader",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_script::Script>(&mut world_ref, "Script", &mut out);
    dump_component::<recomp_game::tes::forms_actors::TesRace>(&mut world_ref, "TesRace", &mut out);
    dump_component::<recomp_game::tes::forms_actors::TesNpc>(&mut world_ref, "TesNpc", &mut out);
    dump_component::<recomp_game::tes::forms_actors::TesCreature>(
        &mut world_ref,
        "TesCreature",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_magic::EffectSetting>(
        &mut world_ref,
        "EffectSetting",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_ai::TesIdleForm>(
        &mut world_ref,
        "TesIdleForm",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_ai::TesPackage>(
        &mut world_ref,
        "TesPackage",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_ai::TesCombatStyle>(
        &mut world_ref,
        "TesCombatStyle",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_dialogue::TesTopic>(
        &mut world_ref,
        "TesTopic",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_dialogue::TesTopicInfo>(
        &mut world_ref,
        "TesTopicInfo",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_dialogue::TesQuest>(
        &mut world_ref,
        "TesQuest",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_items::TesActivator>(
        &mut world_ref,
        "TesActivator",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_items::TesFurniture>(
        &mut world_ref,
        "TesFurniture",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_items::TesApparatus>(
        &mut world_ref,
        "TesApparatus",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_items::TesArmor>(&mut world_ref, "TesArmor", &mut out);
    dump_component::<recomp_game::tes::forms_items::TesClothing>(
        &mut world_ref,
        "TesClothing",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_items::TesBook>(&mut world_ref, "TesBook", &mut out);
    dump_component::<recomp_game::tes::forms_items::TesContainerObject>(
        &mut world_ref,
        "TesContainerObject",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_items::TesDoor>(&mut world_ref, "TesDoor", &mut out);
    dump_component::<recomp_game::tes::forms_items::TesLight>(&mut world_ref, "TesLight", &mut out);
    dump_component::<recomp_game::tes::forms_items::TesSoulGem>(
        &mut world_ref,
        "TesSoulGem",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_items::TesStatic>(
        &mut world_ref,
        "TesStatic",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_items::TesGrass>(&mut world_ref, "TesGrass", &mut out);
    dump_component::<recomp_game::tes::forms_items::TesTree>(&mut world_ref, "TesTree", &mut out);
    dump_component::<recomp_game::tes::forms_items::TesWeapon>(
        &mut world_ref,
        "TesWeapon",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_items::TesAmmo>(&mut world_ref, "TesAmmo", &mut out);
    dump_component::<recomp_game::tes::forms_items::TesSubSpace>(
        &mut world_ref,
        "TesSubSpace",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_items::TesLevItem>(
        &mut world_ref,
        "TesLevItem",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_items::TesLevCreature>(
        &mut world_ref,
        "TesLevCreature",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_items::TesMagicObject>(
        &mut world_ref,
        "TesMagicObject",
        &mut out,
    );
    dump_component::<recomp_game::tes::player::PlayerCharacter>(
        &mut world_ref,
        "PlayerCharacter",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_world::TesObjectCell>(
        &mut world_ref,
        "TesObjectCell",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_world::TesWorldSpace>(
        &mut world_ref,
        "TesWorldSpace",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_world::TesRoad>(&mut world_ref, "TesRoad", &mut out);
    dump_component::<recomp_game::tes::forms_world::TesObjectLand>(
        &mut world_ref,
        "TesObjectLand",
        &mut out,
    );
    dump_component::<recomp_game::tes::forms_world::TesPathGrid>(
        &mut world_ref,
        "TesPathGrid",
        &mut out,
    );
    dump_component::<recomp_game::tes::actor::TesObjectRefr>(
        &mut world_ref,
        "TesObjectRefr",
        &mut out,
    );
    dump_process_component::<recomp_game::tes::actor_process::BaseProcess>(
        &mut world_ref,
        "BaseProcess",
        &mut out,
    );
    dump_process_component::<recomp_game::tes::actor_process::LowProcess>(
        &mut world_ref,
        "LowProcess",
        &mut out,
    );
    dump_process_component::<recomp_game::tes::actor_process::MiddleLowProcess>(
        &mut world_ref,
        "MiddleLowProcess",
        &mut out,
    );
    dump_process_component::<recomp_game::tes::actor_process::MiddleHighProcess>(
        &mut world_ref,
        "MiddleHighProcess",
        &mut out,
    );
    dump_process_component::<recomp_game::tes::actor_process::HighProcess>(
        &mut world_ref,
        "HighProcess",
        &mut out,
    );
    let mut grids = world_ref
        .try_query::<(
            &recomp_game::tes::form::FormIdentity,
            &recomp_game::tes::forms_world::TesPathGrid,
        )>()
        .unwrap();
    for (identity, grid) in grids.iter(&world_ref) {
        let flags: Vec<String> = grid
            .points_24
            .iter()
            .flatten()
            .map(|point| format!("{:X}", point.flags_10))
            .collect();
        writeln!(out, "PGRD {:X} {}", identity.form_id_0c, flags.join(",")).unwrap();
    }
    dump_resource::<recomp_game::bs::havok::HavokGlobals>(&world_ref, "HavokGlobals", &mut out);
    dump_resource::<recomp_game::bs::render_targets::RenderTargetGlobals>(
        &world_ref,
        "RenderTargetGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::bs::scene_graphs::SceneGraphGlobals>(
        &world_ref,
        "SceneGraphGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::bs::shader_manager::ShaderManagerGlobals>(
        &world_ref,
        "ShaderManagerGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::bs::task_manager::TaskGlobals>(
        &world_ref,
        "TaskGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::bs::tree_manager::TreeManagerGlobals>(
        &world_ref,
        "TreeManagerGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::gamebryo::registration::NiProcessDefaults>(
        &world_ref,
        "NiProcessDefaults",
        &mut out,
    );
    dump_resource::<recomp_game::gamebryo::registration::SingletonTail>(
        &world_ref,
        "SingletonTail",
        &mut out,
    );
    dump_resource::<recomp_game::gamebryo::registration::NiD3DShaderCreatorRegistry>(
        &world_ref,
        "NiD3DShaderCreatorRegistry",
        &mut out,
    );
    dump_resource::<recomp_game::gamebryo::screen_element_palette::UiScreenElementPalette>(
        &world_ref,
        "UiScreenElementPalette",
        &mut out,
    );
    dump_resource::<recomp_game::tes::pathing::PathingSingletons>(
        &world_ref,
        "PathingSingletons",
        &mut out,
    );
    dump_resource::<recomp_game::tes::player::PlayerGlobals>(&world_ref, "PlayerGlobals", &mut out);
    dump_resource::<recomp_game::tes::sky::SkyGlobals>(&world_ref, "SkyGlobals", &mut out);
    dump_resource::<recomp_game::tes::tes::TesGlobals>(&world_ref, "TesGlobals", &mut out);
    dump_resource::<recomp_game::tes::tes_grid::TesGridGlobals>(
        &world_ref,
        "TesGridGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::tes::tile::TileEngine>(&world_ref, "TileEngine", &mut out);
    dump_resource::<recomp_game::tes::water::WaterGlobals>(&world_ref, "WaterGlobals", &mut out);
    writeln!(
        out,
        "HEAP B3A6E0 InterfaceManager {:?}",
        world_ref.resource::<recomp_game::tes::interface_manager::InterfaceManager>()
    )
    .unwrap();
    dump_resource::<recomp_game::tes::inventory::InventoryGlobals>(
        &world_ref,
        "InventoryGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::tes::actor_process::ActorProcessGlobals>(
        &world_ref,
        "ActorProcessGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::tes::character_controller::CharacterControllerGlobals>(
        &world_ref,
        "CharacterControllerGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::tes::data_handler::DataHandlerGlobals>(
        &world_ref,
        "DataHandlerGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::tes::data_load::DataLoadGlobals>(
        &world_ref,
        "DataLoadGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::tes::default_forms::DefaultFormStatics>(
        &world_ref,
        "DefaultFormStatics",
        &mut out,
    );
    dump_resource::<recomp_game::tes::default_forms::TimeGlobals>(
        &world_ref,
        "TimeGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::tes::forms_dialogue::DialogueGlobals>(
        &world_ref,
        "DialogueGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::tes::forms_environment::WarningCounter>(
        &world_ref,
        "WarningCounter",
        &mut out,
    );
    dump_resource::<recomp_game::tes::forms_init::RaceStatics>(&world_ref, "RaceStatics", &mut out);
    dump_resource::<recomp_game::tes::forms_init::QuestDelayGlobals>(
        &world_ref,
        "QuestDelayGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::tes::forms_items::TreeGlobals>(
        &world_ref,
        "TreeGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::tes::forms_region::RegionLoadGlobals>(
        &world_ref,
        "RegionLoadGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::tes::forms_script::ScriptGlobals>(
        &world_ref,
        "ScriptGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::tes::forms_world::WorldFormGlobals>(
        &world_ref,
        "WorldFormGlobals",
        &mut out,
    );
    dump_resource::<recomp_game::tes::forms_world::TerrainLodSettings>(
        &world_ref,
        "TerrainLodSettings",
        &mut out,
    );
    if let Some(tes) = world_ref
        .resource::<recomp_game::tes::tes::TesGlobals>()
        .tes
        .as_ref()
    {
        writeln!(out, "HEAP B333A0 Tes {tes:?}").unwrap();
    }
    if let Some(sky) = world_ref
        .resource::<recomp_game::tes::sky::SkyGlobals>()
        .singleton_b365c4
        .as_ref()
    {
        writeln!(out, "HEAP B365C4 Sky {sky:?}").unwrap();
    }
    writeln!(
        out,
        "HEAP B33B00 TesSaveLoadGame {:?}",
        world_ref
            .resource::<recomp_game::tes::tes::TesGlobals>()
            .save_load_game
            .as_ref()
            .unwrap()
    )
    .unwrap();
    out
}

// IDA: N/A up to 1,000 of a component on forms, as `{:?}`, for `tools/original_debug/field_compare.py`.
fn dump_component<T: Component + std::fmt::Debug>(world: &mut World, name: &str, out: &mut String) {
    use recomp_game::tes::form::FormIdentity;
    use std::fmt::Write as _;
    let mut query = world.query::<(&FormIdentity, &T)>();
    for (identity, value) in query.iter(world) {
        writeln!(out, "COMP {name} {:X} {value:?}", identity.form_id_0c).unwrap();
    }
}

// IDA: N/A a resource of globals, as `{:?}`: `field_compare.py` reads a field named `name_b33a2c` at 00B33A2C.
fn dump_resource<T: Resource + std::fmt::Debug>(world: &World, name: &str, out: &mut String) {
    use std::fmt::Write as _;
    if let Some(value) = world.get_resource::<T>() {
        writeln!(out, "GLOB {name} {value:?}").unwrap();
    }
}

// IDA: N/A a process object's component, under its actor: `field_compare.py` follows the actor's +0x58.
fn dump_process_component<T: Component + std::fmt::Debug>(
    world: &mut World,
    name: &str,
    out: &mut String,
) {
    use recomp_game::tes::actor::MobileObject;
    use recomp_game::tes::form::FormIdentity;
    use std::fmt::Write as _;
    let mut query = world.query::<(&FormIdentity, &MobileObject)>();
    for (identity, mobile) in query.iter(world) {
        if let Some(value) = mobile
            .process_58
            .and_then(|process| world.get::<T>(process))
        {
            writeln!(out, "PROC {name} {:X} {value:?}", identity.form_id_0c).unwrap();
        }
    }
}

// IDA: N/A FNV-1a of form IDs, each little-endian.
fn fnv_form_ids(world: &World, entities: impl Iterator<Item = Entity>) -> u32 {
    let mut hash = 0x811C_9DC5_u32;
    for entity in entities {
        let id = world
            .get::<recomp_game::tes::form::FormIdentity>(entity)
            .unwrap()
            .form_id_0c;
        for byte in id.to_le_bytes() {
            hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
        }
    }
    hash
}

// run-c1-log, 2026-11-06: 00680090's sites in order: 4,567 past the first gates at 006800D3, 2,284 past 0067F5D0.
#[test]
fn the_door_nodes_match_the_original_in_order() {
    let world = run(&real_data(), Some(1));
    let nodes = world.resource::<recomp_game::tes::pathing::PathingDoorNodes>();
    assert_eq!(nodes.gated.len(), 4_567);
    assert_eq!(nodes.nodes.len(), 2_284);
    assert_eq!(
        fnv_form_ids(&world, nodes.gated.iter().copied()),
        0x1911_5CED
    );
    assert_eq!(
        fnv_form_ids(&world, nodes.nodes.iter().map(|node| node.door_04)),
        0xEC00_A225
    );
    // 00B3BE00 in snapshot-phase16_end: 1,814 keys, 1,646 cells, 112 subspace references and 56 worlds.
    let keys = nodes.world_keys();
    let mut types = [0_u32; 3];
    for &key in keys {
        match world
            .get::<recomp_game::tes::form::FormIdentity>(key)
            .unwrap()
            .form_type_04
        {
            0x30 => types[0] += 1,
            0x31 => types[1] += 1,
            0x35 => types[2] += 1,
            other => panic!("a key of type {other:X}"),
        }
    }
    assert_eq!(types, [1_646, 112, 56]);
    let map = world
        .resource::<recomp_game::tes::pathing::PathingSingletons>()
        .world_node_map_b3be00
        .as_ref()
        .unwrap();
    assert_eq!(map.count_0c, 1_814);
}

// snapshot-load_end and snapshot-phase16_end of run-b1-snap: 00B06164's forms, bucket by bucket, each chain from its head. The hash is the original's. Dead keys read as free-list words there, and the renamed cells' old names answer null.
#[test]
fn the_editor_id_map_matches_the_original_chain_by_chain() {
    let world = run(&real_data(), Some(1));
    let map = world.resource::<recomp_game::tes::data_handler::EditorIdMap>();
    assert_eq!(map.count(), 7_896);
    assert_eq!(
        fnv_form_ids(&world, map.walk().map(|entry| entry.form)),
        0xCEE2_4219
    );
    // The two arms a hit takes, from run-c1-log: the name back in its own block, and FULL's text in the old key's.
    assert_eq!(id_of_name(&world, map, b"Nagastani"), 0x817C8);
    assert_eq!(id_of_name(&world, map, b"BravilUngolimHouse"), 0x2FF4D);
    assert_eq!(
        map.walk()
            .filter(|entry| entry.form == form_of(&world, 0x817C8))
            .count(),
        1
    );
    // A miss leaves a dead node behind, and a rename's old name goes with it.
    assert_eq!(
        map.walk()
            .filter(|entry| entry.form == form_of(&world, 0x579E6))
            .count(),
        2
    );
    assert_eq!(map.lookup(b"TestParadiseCave"), None);
    assert_eq!(id_of_name(&world, map, b"DLDunbarrowHaven"), 0x5D36A);
}

fn form_of(world: &World, id: u32) -> Entity {
    world.resource::<FormsById>().lookup(id).unwrap()
}

fn id_of_name(
    world: &World,
    map: &recomp_game::tes::data_handler::EditorIdMap,
    name: &[u8],
) -> u32 {
    world
        .get::<recomp_game::tes::form::FormIdentity>(map.lookup(name).unwrap())
        .unwrap()
        .form_id_0c
}

// snapshot-load_end: Masser and Secunda exist before any plugin loads, so they hold the exe's defaults, where the settings
// themselves read 60 and 75 by then.
#[test]
fn the_moons_keep_the_settings_from_before_the_load() {
    let world = run(&real_data(), Some(1));
    let sky = world
        .resource::<recomp_game::tes::sky::SkyGlobals>()
        .singleton_b365c4
        .as_ref()
        .unwrap();
    let masser = sky.masser_30.as_ref().unwrap();
    assert_eq!(
        (
            masser.fade_start_58,
            masser.fade_end_5c,
            masser.shadow_early_fade_60
        ),
        (55.0, 45.0, 0.5)
    );
    assert_eq!(
        (masser.speed_64, masser.z_offset_68, masser.size_6c),
        (0.25, 35.0, 94)
    );
    assert_eq!(masser.textures[1], "\\Textures\\Sky\\Masser_three_wan.dds");
    assert!(masser.textures[4].is_empty());
    let secunda = sky.secunda_34.as_ref().unwrap();
    assert_eq!(
        (secunda.fade_start_58, secunda.fade_end_5c, secunda.speed_64),
        (55.0, 40.0, 0.3)
    );
    assert_eq!((secunda.z_offset_68, secunda.size_6c), (50.0, 40));
}

// The rows on the driver's World: P14-RUN-004's whole AI pass, with `RUN_004_ROLL_START` reached through the driver's own first-use seed, so one CRT gives Phase 11's draw, 12d's rolls and 14c's 157. The test's seed is a test value, as the plan keeps it.
#[test]
fn the_driver_world_passes_the_act_rows_and_the_rand_stream() {
    let data = real_data();
    let mut check = MsvcRand::default();
    // Phase 11's draw, then 005AD440's 191 in the load (`screens::the_load_fills_the_slide_list_and_draws`), then RUN_004_ROLL_START, which the harness's load reaches without a LoadingMenu.
    let mut before_load = RUN_004_ROLL_START;
    for _ in 0..191 {
        before_load = lcg_previous(before_load);
    }
    let seed = seed_before(before_load);
    check.srand(seed);
    let _ = check.rand();
    assert_eq!(check.holdrand(), before_load);
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&data, Some(seed), Some(Duration::ZERO)),
        WinMainSeams::default(),
    );
    let mut schedule = winmain_schedule();
    // 0040EBB9, the block's end: Phase 15 and 16 haven't run.
    let mut at_load_end = None;
    loop {
        let next = world.resource::<WinMainProgress>().next;
        if next == Block::InitialArea || next == Block::Finished {
            break;
        }
        if next == Block::WorldSetup && at_load_end.is_none() {
            at_load_end = main_thread_crt_state(&world);
        }
        schedule.run(&mut world);
    }
    assert!(world.resource::<WinMainProgress>().stops.is_empty());
    assert_eq!(
        at_load_end.unwrap().0,
        lcg_previous(0x1B8C_A0AD),
        "the stream at 0040EB9F, RAND's state before its first draw"
    );
    let mut expected = MsvcRand::default();
    expected.srand(0x1B8C_A0AD);
    for _ in 1..157 {
        expected.rand();
    }
    let (holdrand, _) = main_thread_crt_state(&world).unwrap();
    assert_eq!(holdrand, expected.holdrand(), "157 draws, the RAND rows'");
    assert_eq!(
        world.resource::<GameThreadLocals>().track_changes_185,
        1,
        "0040EB8B"
    );
    let book = RowBook::load("phase14");
    let rows = book.take_kind("ACT");
    let after: Vec<&Vec<&str>> = rows.iter().filter(|row| row[1] != "before").collect();
    assert_eq!(after.len(), 1998);
    let mut tally = Tally;
    let differences: Vec<String> = after
        .iter()
        .filter_map(|row| {
            let id_text = row
                .iter()
                .find_map(|part| part.strip_prefix("id="))
                .unwrap();
            let id = u32::from_str_radix(id_text, 16).unwrap();
            let actor = world.resource::<FormsById>().lookup(id).unwrap();
            compare_actor(&world, row, actor, &mut tally).map(|text| format!("{id:X}: {text}"))
        })
        .collect();
    assert!(
        differences.is_empty(),
        "{} differ: {:#?}",
        differences.len(),
        &differences[..differences.len().min(8)]
    );
    winmain_shutdown(&mut world);
}

fn lcg_previous(state: u32) -> u32 {
    seed_before(state)
}

// P13-RUN-001's model trees on what the driver's drain built, with the real palette, renderer data and shader slots where the harness has stand-ins. 13e's walk starts with every task in the map, a recorded substitution, and the trees don't depend on it.
#[test]
fn the_driver_builds_the_recorded_model_trees() {
    let world = run(&real_data(), Some(1));
    let tasks = world.resource::<TaskGlobals>();
    let loader = tasks.model_loader_b33a1c.as_ref().unwrap();
    let rows = fixtures::rows("phase13e");
    let mut compared = 0;
    for line in &rows {
        let Some(rest) = line.strip_prefix("MODEL|") else {
            continue;
        };
        let parts: Vec<&str> = rest.split('|').collect();
        let chain = &loader.models.buckets[model_map_bucket(parts[0].as_bytes()) as usize];
        let entry = chain
            .iter()
            .find(|entry| entry.key == parts[0].as_bytes())
            .unwrap_or_else(|| panic!("{line}"));
        let held = &loader.models.store[entry.model.0];
        assert_eq!(held.path_00, parts[2].as_bytes(), "{line}");
        for tree in model_tree_rows(&world, parts[0], held.root_08.unwrap()) {
            let wanted = rows
                .iter()
                .find(|row| {
                    let (a, b): (Vec<&str>, Vec<&str>) =
                        (row.split('|').collect(), tree.split('|').collect());
                    a[0] == "NODE" && a[1] == b[1] && a[2] == b[2]
                })
                .unwrap_or_else(|| panic!("no row for {tree}"));
            // Declared: 0080B190's property, on the one object that reaches the boundary.
            if wanted.contains("BSShaderPPLightingProperty") {
                let fields = |line: &str| -> Vec<String> {
                    line.split('|')
                        .enumerate()
                        .filter(|(index, _)| *index != 7)
                        .map(|(_, field)| field.to_owned())
                        .collect()
                };
                assert_eq!(fields(&tree), fields(wanted));
            } else {
                assert_eq!(&tree, wanted);
            }
            compared += 1;
        }
    }
    assert_eq!(compared, 86);
    assert_eq!(
        tasks.io_manager_b33a10.as_ref().unwrap().tasks.buckets[16].len(),
        0,
        "the tasks left in the queue"
    );
}

// app_failure.cmake's first half: no `Data\` at all. The driver goes on through every block, as main.cpp does, and prints the same lines the C++ printed for its failure fixture. The exit 1 is `Block::MainMenu`'s, at the missing XML.
#[test]
fn a_missing_data_folder_prints_the_failure_lines() {
    let scratch =
        std::env::temp_dir().join(format!("recomp_winmain_nodata_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let world = run(&scratch.join("Data"), Some(1));
    let _ = std::fs::remove_dir_all(&scratch);
    let progress = world.resource::<WinMainProgress>();
    assert_eq!(progress.exit_code, Some(1));
    let stops: Vec<String> = progress
        .stops
        .iter()
        .map(|stop| stop.text.clone())
        .collect();
    assert!(stops.is_empty(), "{stops:?}");
    let ours = lines(&world);
    let baseline = baseline_lines("LastTest-x64.log", "app_startup_failure");
    assert_matches_baseline(&ours, &baseline);
}

// A failed FormHeap(0x28) leaves g_osGlobals null, and the original faults at 0040EAB7: the driver stops there, reports it, and stays up.
#[test]
fn a_null_os_globals_is_a_reported_stop() {
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&real_data(), Some(1), None),
        WinMainSeams {
            failed_allocations: vec![0x28],
            ..WinMainSeams::default()
        },
    );
    run_winmain_straight_through(&mut world);
    let progress = world.resource::<WinMainProgress>();
    assert_eq!(progress.exit_code, None);
    assert_eq!(progress.stops.len(), 1);
    assert_eq!(progress.stops[0].block, Block::ActivationAndMovie);
    assert!(progress.stops[0].text.contains("0040EAB7"));
    assert_eq!(progress.lines.len(), 1, "{:?}", progress.lines);
    assert!(progress.lines[0].starts_with("Phase-06 host stop: "));
    assert!(progress.finished());
}

// 0040E735: with no INI the settings keep their image values, among them 640x480. Under the pinned caps 00498340 can't make that device, so 004052F0's failure exit runs, a reported ExitProcess(0). Unverified: no run had no INI.
#[test]
fn a_missing_ini_keeps_the_image_values() {
    let mut world = World::new();
    let mut settings = config(&real_data(), Some(1), None);
    let scratch = std::env::temp_dir().join(format!("recomp_winmain_noini_{}", std::process::id()));
    settings.settings = SettingsSource::Pinned(scratch.join("Oblivion.ini"));
    install_winmain(&mut world, settings, WinMainSeams::default());
    run_winmain_straight_through(&mut world);
    winmain_shutdown(&mut world);
    let progress = world.resource::<WinMainProgress>();
    assert_eq!(progress.exit_code, Some(0), "004052F0's ExitProcess(0)");
    assert!(progress.finished());
    assert_eq!(progress.next, Block::Finished);
    assert!(progress.lines.is_empty(), "{:?}", progress.lines);
}

// IDA: N/A the pinned folder with the starting settings of an INI path, in a scratch folder.
fn starting_folder(tag: &str, cell: &str, x: &str, y: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("recomp_winmain_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let config = fixtures::root().join("config");
    for entry in std::fs::read_dir(&config).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), folder.join(entry.file_name())).unwrap();
    }
    let text = std::fs::read(config.join("Oblivion.ini")).unwrap();
    let text: String = text.iter().map(|&byte| char::from(byte)).collect();
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
    // The copy keeps a read-only fixture's mode.
    std::fs::remove_file(folder.join("Oblivion.ini")).unwrap();
    std::fs::write(folder.join("Oblivion.ini"), bytes).unwrap();
    folder
}

fn run_starting(tag: &str, cell: &str, x: &str, y: &str, seed: u32) -> World {
    let folder = starting_folder(tag, cell, x, y);
    let mut world = World::new();
    let mut settings = config(&real_data(), Some(seed), None);
    settings.settings = SettingsSource::Pinned(folder.join("Oblivion.ini"));
    install_winmain(&mut world, settings, WinMainSeams::default());
    run_winmain_straight_through(&mut world);
    winmain_shutdown(&mut world);
    let _ = std::fs::remove_dir_all(&folder);
    world
}

/// A start in a cell stops here: the player's 3D isn't ported, and 0040EF32 needs it.
const CELL_START_STOP: &str =
    "0040EF32: 00707610 on the player's third-person root, whose 3D isn't ported";

// IDA: N/A Phase 16's line: Phase 17's stop comes after it.
fn phase_16_line(progress: &WinMainProgress) -> &str {
    progress
        .lines
        .iter()
        .rev()
        .find(|line| line.starts_with("Phase-16: "))
        .expect("Phase 16 ran")
}

// P16-RUN-002's interior path through the driver, with the real 00433590 pass at 00434020. The driver is the first thing to run this with no recording arm.
#[test]
fn the_interior_path_reaches_the_cell_arm() {
    let world = run_starting("interior", "XPAichan01", "", "", 1);
    let progress = world.resource::<WinMainProgress>();
    let stops: Vec<String> = progress
        .stops
        .iter()
        .map(|stop| stop.text.clone())
        .collect();
    assert_eq!(stops, [CELL_START_STOP], "{:#?}", progress.lines);
    assert!(
        progress
            .lines
            .iter()
            .any(|line| line.starts_with("Phase-15: path editor ID cell")),
        "{:#?}",
        progress.lines
    );
    let last = phase_16_line(progress);
    assert!(last.ends_with("0040EF24"), "{last}");
    // RUN-002's cell 00010360, in decimal as main.cpp prints it, and `%g` of the rows' bits.
    assert_eq!(
        last,
        "Phase-16: cell 66400, Rect {2912.11, -366.928, 864}, rotation z -0.00840143, 0040EF24"
    );
}

// P16-RUN-003's exterior path through the driver.
#[test]
fn the_coordinates_path_reaches_the_cell_arm() {
    let world = run_starting("exterior", "", "4", "7", 1);
    let progress = world.resource::<WinMainProgress>();
    let stops: Vec<String> = progress
        .stops
        .iter()
        .map(|stop| stop.text.clone())
        .collect();
    assert_eq!(stops, [CELL_START_STOP], "{:#?}", progress.lines);
    assert!(
        progress
            .lines
            .iter()
            .any(|line| line.starts_with("Phase-15: path coordinates cell")),
        "{:#?}",
        progress.lines
    );
    let last = phase_16_line(progress);
    assert!(last.ends_with("0040EF24"), "{last}");
    // RUN-003's cell 00005FC9, and its land height 530.77.
    assert_eq!(
        last,
        "Phase-16: cell 24521, Rect {18782.7, 29551.7, 530.777}, rotation z -0.355223, 0040EF24"
    );
}

// Live-run fallout: a spread of seeds and each INI path. `RECOMP_SEED_SCAN` names how many; off by default, since each run takes its seconds.
#[test]
fn a_spread_of_seeds_reaches_the_end() {
    let count: u32 = std::env::var("RECOMP_SEED_SCAN")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(1);
    let data = real_data();
    let mut stops = Vec::new();
    for seed in 0..count {
        let word = seed.wrapping_mul(0x0101_0101);
        let default = run(&data, Some(word));
        let interior = run_starting("scan_interior", "XPAichan01", "", "", word);
        let exterior = run_starting("scan_exterior", "", "4", "7", word);
        for (path, world) in [
            ("default", default),
            ("interior", interior),
            ("exterior", exterior),
        ] {
            let progress = world.resource::<WinMainProgress>();
            for stop in progress
                .stops
                .iter()
                .filter(|stop| stop.text != CELL_START_STOP)
            {
                stops.push(format!(
                    "seed {seed} {path}: {:?}: {}",
                    stop.block, stop.text
                ));
            }
        }
    }
    assert!(stops.is_empty(), "{stops:#?}");
}

// 007203F0 with 00B256CC set and the renderer behind it, where the phase tests pass no renderer: the Sky's textures have renderer data. No row reaches this: a texture's +0x24 isn't dumped.
#[test]
fn the_precache_gives_source_textures_renderer_data() {
    let mut world = run(&real_data(), Some(1));
    let mut textures = world.query::<&recomp_game::gamebryo::texture::SourceTexture>();
    let (mut total, mut with_data, mut clouds) = (0, 0, 0);
    for texture in textures.iter(&world) {
        total += 1;
        with_data += u32::from(texture.renderer_data_24);
        let name = texture.filename_34.as_deref().unwrap_or_default();
        if name
            .to_ascii_lowercase()
            .windows(5)
            .any(|part| part == b"cloud")
        {
            clouds += 1;
            assert!(
                texture.renderer_data_24,
                "{}",
                String::from_utf8_lossy(name)
            );
        }
    }
    println!("source textures {total}, with renderer data {with_data}, clouds {clouds}");
    assert!(clouds > 0, "the clouds' textures exist");
}

// ---- past `Block::Finished`: the menu's media ----

struct MediaRun {
    world: World,
    bink: std::sync::Arc<std::sync::Mutex<recomp_game::tes::bink_library::ScriptedBink>>,
    music: std::sync::Arc<std::sync::Mutex<recomp_game::tes::os_sound_music::ScriptedMusicDevice>>,
}

fn run_with_media(settings: WinMainConfig) -> MediaRun {
    use recomp_game::tes::movie_player::{MoviePlayer, movie_player_is_running};
    use std::sync::{Arc, Mutex};
    let mut scripted = recomp_game::tes::bink_library::ScriptedBink::default();
    scripted.width = 1280;
    scripted.height = 720;
    let bink = Arc::new(Mutex::new(scripted));
    let music = Arc::new(Mutex::new(
        recomp_game::tes::os_sound_music::ScriptedMusicDevice::default(),
    ));
    let mut world = World::new();
    install_winmain(
        &mut world,
        settings,
        WinMainSeams {
            bink: bink.clone(),
            music: music.clone(),
            ..WinMainSeams::default()
        },
    );
    run_winmain_straight_through(&mut world);
    // The path both runs took: the thread is done before the MainMenu, as P06-RUN-001's `hThread34=0` shows.
    let player = world.resource::<MoviePlayer>().clone();
    while movie_player_is_running(&player) {
        std::thread::sleep(Duration::from_millis(1));
    }
    MediaRun { world, bink, music }
}

/// 005B5F20, 0040F092, 0040F0AA, 0040F0BE and 005B5C90 on the driver's world, with the seams' traces: the backdrop first, then its pause around the logo, the title music once, and the object against P06-RUN-001's `OBJECT|MENU` dump. Then a frame's poll reaches the game's input.
#[test]
fn the_menu_path_starts_its_movie_and_music() {
    use recomp_game::tes::movie_player::{BINK_PRELOAD_ALL, MOVIE_STATE_MENU_OWNED, MoviePlayer};
    use recomp_game::tes::os_globals::{OsGlobalsData, lock_shared};
    use recomp_game::tes::os_sound_music::{
        MUSIC_FLAG_LOADED, MUSIC_FLAG_PLAYING, MUSIC_TYPE_SPECIAL, MusicGraphCall, MusicTraceCall,
    };
    use recomp_startup::winmain::os_input_frame;
    use recomp_startup::winmain_menu_media::{
        MainMenuMedia, MainMenuMediaStage, run_main_menu_media_frame,
    };
    let mut run = run_with_media(config(&real_data(), Some(1), None));
    let world = &mut run.world;
    let media = *world.resource::<MainMenuMedia>();
    assert_eq!(media.stage, MainMenuMediaStage::Running);
    assert_eq!(media.backdrop_movie_b3b408, 1);
    assert_eq!(media.menu_constructed_b3b409, 1);
    {
        let bink = lock_shared(&run.bink);
        let names: Vec<String> = bink
            .opens
            .iter()
            .map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 6);
        assert_eq!(names[4], "Map loop.bik", "005B5FA0");
        assert_eq!(names[5], "Oblivion iv logo.bik", "0040F0BE");
        // Off the thread, on the main thread: preload by size for 00410C40's `1`, and no preload at 0040F0BE.
        assert_eq!(bink.opens[4].1, BINK_PRELOAD_ALL);
        assert_eq!(bink.opens[5].1, 0);
        assert_eq!(bink.pauses, [1, 0], "the backdrop, around the logo");
    }
    let player = world.resource::<MoviePlayer>().clone();
    let current = lock_shared(&player.globals.current_b33428).clone().unwrap();
    assert_eq!(current.state(), MOVIE_STATE_MENU_OWNED);
    let row = fixtures::rows("phase06")
        .into_iter()
        .find(|row| row.starts_with("MOVIE|P06-RUN-001|OBJECT|MENU|"))
        .unwrap();
    let dump = row.split('|').nth(4).unwrap().to_owned();
    let word = |offset: usize| {
        u32::from_str_radix(&dump[offset * 2..offset * 2 + 8], 16)
            .unwrap()
            .swap_bytes()
    };
    {
        let fields = current.fields();
        assert_eq!(fields.scale_14.to_bits(), word(0x14));
        assert_eq!(fields.x_18.to_bits(), word(0x18));
        assert_eq!(fields.y_1c.to_bits(), word(0x1C));
        assert_eq!(current.state(), word(0x20));
        assert_eq!(fields.frames_shown_0c, 0);
    }
    {
        let music = lock_shared(&run.music);
        const TITLE: &[u8] = b"Data\\Music\\Special\\TES4Title.mp3";
        assert_eq!(
            music.trace[0],
            (0x006A_B2FB, MusicTraceCall::Access(TITLE.to_vec()))
        );
        let renders = music
            .trace
            .iter()
            .filter(|(_, call)| matches!(call, MusicTraceCall::RenderFile(path) if path == TITLE))
            .count();
        assert_eq!(renders, 1, "0040F0AA finds it playing, and holds");
        let runs = music
            .trace
            .iter()
            .filter(|(_, call)| matches!(call, MusicTraceCall::Graph(MusicGraphCall::Run)))
            .count();
        assert_eq!(runs, 1);
    }
    {
        let os = world.resource::<OsGlobalsData>();
        let sound = os.sound_24.as_ref().unwrap();
        assert_eq!(sound.music_flags_dc, MUSIC_FLAG_LOADED | MUSIC_FLAG_PLAYING);
        assert_eq!(sound.current_music_type_b0, MUSIC_TYPE_SPECIAL);
        assert_eq!(sound.main_menu_music_b3c0ec, 1);
        assert_eq!(sound.music_unported_arms, 0);
    }
    // 0040D8CD: what the host feeds is what the game's input reads.
    let frame = os_input_frame(world).unwrap();
    lock_shared(&frame).keyboard[0x39] = 0x80;
    lock_shared(&frame).mouse.x = 7;
    run_main_menu_media_frame(world);
    {
        let os = world.resource::<OsGlobalsData>();
        let input = os.input_20.as_ref().unwrap().lock();
        assert_eq!(input.keyboard_state_18f4[0x39], 0x80);
        assert_eq!(input.mouse_state_1b20.x, 7, "D3's mouse reads, in the app");
    }
    assert_eq!(lock_shared(&frame).mouse.x, 0, "and the poll took it");
    let bink = lock_shared(&run.bink);
    assert_eq!(
        bink.copies.last().unwrap().source_width,
        256,
        "0057F3BC drew a frame"
    );
    drop(bink);
    winmain_shutdown(world);
}

/// As `open_main_menu`: a host stop means no menu, so no movie and no music. The menu's media never started.
#[test]
fn a_host_stop_starts_no_menu_media() {
    use recomp_game::tes::os_globals::lock_shared;
    use recomp_startup::winmain::{Block, HostStopReport, WinMainProgress};
    use recomp_startup::winmain_menu_media::{MainMenuMedia, run_main_menu_media_frame};
    let music = std::sync::Arc::new(std::sync::Mutex::new(
        recomp_game::tes::os_sound_music::ScriptedMusicDevice::default(),
    ));
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&real_data(), Some(1), Some(Duration::ZERO)),
        WinMainSeams {
            music: music.clone(),
            ..WinMainSeams::default()
        },
    );
    let mut schedule = winmain_schedule();
    while world.resource::<WinMainProgress>().next != Block::MainMenu {
        schedule.run(&mut world);
    }
    // As `run_block` leaves a stop: the rest of the blocks don't run.
    {
        let mut progress = world.resource_mut::<WinMainProgress>();
        progress.stops.push(HostStopReport {
            block: Block::PlacePlayer,
            text: "a test's stop".to_owned(),
        });
        progress.next = Block::Finished;
    }
    for _ in 0..3 {
        run_main_menu_media_frame(&mut world);
    }
    assert!(
        world
            .get_resource::<MainMenuMedia>()
            .is_none_or(|media| !media.ctor_ran)
    );
    assert!(lock_shared(&music).trace.is_empty(), "no music");
    winmain_shutdown(&mut world);
}

/// 0047D247 multiplies by 00B06704, which is `fGlobalTimeMultiplier:General` (009DC302 registers it), so the INI's value reaches the frame time. Seeded, the first frame's step is 0047D220's cap, 0xA6 ms.
#[test]
fn the_ini_s_time_multiplier_reaches_the_frame_time() {
    use recomp_startup::winmain::driver_frame_seconds;
    use recomp_startup::winmain_menu_media::run_main_menu_media_frame;
    let folder = starting_folder("timescale", "", "", "");
    let path = folder.join("Oblivion.ini");
    let text: String = std::fs::read(&path)
        .unwrap()
        .iter()
        .map(|&byte| char::from(byte))
        .collect();
    let text = text.replace(
        "fGlobalTimeMultiplier=1.0000",
        "fGlobalTimeMultiplier=2.0000",
    );
    let bytes: Vec<u8> = text.chars().map(|c| c as u8).collect();
    std::fs::write(&path, bytes).unwrap();
    let mut settings = config(&real_data(), Some(1), None);
    settings.settings = SettingsSource::Pinned(path);
    let mut run = run_with_media(settings);
    let world = &mut run.world;
    // A Running frame.
    for _ in 0..1 {
        run_main_menu_media_frame(world);
    }
    // 0047D238 `fmul` A30E40, 0047D241 `fstp` m32.
    let step = (f64::from(0xA6) * f64::from_bits(0x3F50_624D_E000_0000)) as f32;
    assert_eq!(driver_frame_seconds(world), step * 2.0);
    winmain_shutdown(world);
    let _ = std::fs::remove_dir_all(&folder);
}

/// 0040F06E: a player with a cell skips the menu: no music, no backdrop, only the wait.
#[test]
fn a_player_with_a_cell_starts_no_menu_media() {
    use recomp_game::tes::os_globals::lock_shared;
    use recomp_startup::winmain_menu_media::{MainMenuMedia, run_main_menu_media_frame};
    let folder = starting_folder("nomenu", "XPAichan01", "", "");
    let mut settings = config(&real_data(), Some(1), None);
    settings.settings = SettingsSource::Pinned(folder.join("Oblivion.ini"));
    let mut run = run_with_media(settings);
    let _ = std::fs::remove_dir_all(&folder);
    // Phase 17's stop at 0040EF32 comes first, and a stop means no menu path: 0040F0DB's arm waits for the player's 3D.
    assert_eq!(
        run.world.resource::<WinMainProgress>().stops[0].text,
        CELL_START_STOP
    );
    run_main_menu_media_frame(&mut run.world);
    assert!(run.world.get_resource::<MainMenuMedia>().is_none());
    assert!(lock_shared(&run.music).trace.is_empty());
    assert_eq!(lock_shared(&run.bink).opens.len(), 4, "the intro only");
    winmain_shutdown(&mut run.world);
}

// IDA: N/A the port's side of `tools/original_debug/phase17_rows.py`, in the same format and order.
fn phase_17_rows(world: &mut World) -> Vec<String> {
    use recomp_game::bs::scene_graphs::{SceneGraphGlobals, ShadowSceneNode};
    use recomp_game::gamebryo::av_object::NiAvParent;
    use recomp_game::gamebryo::object_net::NiObjectNet;
    use recomp_game::gamebryo::scene_derived::DirectionalLight;
    use recomp_game::gamebryo::scene_node::{NiAvWorld, NiNodeChildren, SceneNodePayload};
    use recomp_game::gamebryo::scene_update::update_scene_object;
    use recomp_game::gamebryo::transform::NiTransform;
    use recomp_game::tes::hud_main_menu::HUD_MAIN_MENU_ID;
    use recomp_game::tes::interface_manager::InterfaceManager;
    use recomp_game::tes::tile::{InterfaceCx, MenuClass};
    use recomp_game::tes::tile_values::{TRAIT_USER0, tile_numeric_trait};
    use recomp_startup::winmain::with_driver_interface;
    let hex = |values: &[f32]| {
        values
            .iter()
            .map(|value| format!("{:08X}", value.to_bits()))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let latin = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|&byte| char::from(byte))
            .collect::<String>()
            .replace('\n', "\\n")
    };
    with_driver_interface(world, |cx: &mut InterfaceCx| {
        let mut rows = Vec::new();
        let manager = cx.world.resource::<InterfaceManager>().clone();
        // 0057F38D's update of the 3D graph comes with a render, and 005AD980 draws by the clock: run-h-phase17 drew once after the light's attach, and the port's test clock hasn't. So it's run here once, for the light's revision and direction.
        let graph_3d = manager.menu_3d_scene_graph_004.unwrap();
        update_scene_object(cx.world, graph_3d, 0.0, false, cx.runtime).unwrap();
        rows.push(format!(
            "MANAGER status={} stack={}",
            manager.menu_loading_status_008,
            manager
                .menu_mode_stack_0e0
                .iter()
                .map(|mode| format!("{mode:08X}"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        let line = |cx: &InterfaceCx, tag: &str, node: Entity, flags: u16, local: &NiTransform| {
            let name = cx
                .world
                .get::<NiObjectNet>(node)
                .and_then(|net| net.name.clone())
                .unwrap_or_default();
            let world_transform = cx.world.get::<NiAvWorld>(node).unwrap().transform;
            format!(
                "{tag} name={} flags={flags:04X} rot={} pos={} scale={} world={}",
                latin(&name),
                hex(&local.rotation),
                hex(&local.translation),
                hex(&[local.scale]),
                hex(&world_transform.translation)
            )
        };
        let scene = |cx: &InterfaceCx, tag: &str, node: Entity| {
            let payload = cx.world.get::<SceneNodePayload>(node).unwrap();
            line(
                cx,
                tag,
                node,
                payload.effective_av_flags,
                &payload.local_transform,
            )
        };
        let globals = cx.world.resource::<SceneGraphGlobals>().clone();
        let graph = globals.world.unwrap();
        let child = cx.world.get::<NiNodeChildren>(graph).unwrap().slots_ac[0].unwrap();
        rows.push(scene(cx, "CHILD0", child));
        let node_60 = manager.player_scene_node_060.unwrap();
        let shadow = manager.player_shadow_scene_064.unwrap();
        rows.push(scene(cx, "SCENE60", node_60));
        rows.push(scene(cx, "SCENE64", shadow));
        let light = manager.light_018.unwrap();
        let state = cx.world.get::<DirectionalLight>(light).unwrap();
        rows.push(line(
            cx,
            "LIGHT",
            light,
            state.payload.av.effective_av_flags,
            &state.payload.av.local_transform,
        ));
        rows.push(format!(
            "LIGHTDATA revision={} ambient={} diffuse={} direction={}",
            state.unknown_b8,
            hex(&state.payload.ambient),
            hex(&state.payload.diffuse),
            hex(&state.direction_108)
        ));
        let children = cx.world.get::<NiNodeChildren>(shadow).unwrap();
        rows.push(format!(
            "SHADOW b11c={} slot1={} parent={} children={}",
            cx.world.get::<ShadowSceneNode>(shadow).unwrap().byte_11c,
            if globals.shadow_slots[1] == Some(shadow) {
                "same"
            } else {
                "other"
            },
            if cx.world.get::<NiAvParent>(shadow).unwrap().0 == manager.menu_3d_scene_graph_004 {
                "menu3d"
            } else {
                "other"
            },
            children.slots_ac.iter().flatten().count()
        ));
        let menus: Vec<String> = (1001..=1051)
            .filter(|&id| cx.engine.tile_menu_by_id(id).is_some())
            .map(|id: u32| id.to_string())
            .collect();
        rows.push(format!("MENUS {}", menus.join(" ")));
        let root = manager.menu_root_068.unwrap();
        rows.push(format!(
            "ROOT user0={}",
            hex(&[tile_numeric_trait(cx, root, TRAIT_USER0)])
        ));
        let hud_tile = cx.engine.tile_menu_by_id(HUD_MAIN_MENU_ID).unwrap();
        let menu = cx.engine.containing_menu(hud_tile).unwrap();
        let owner = cx.engine.menu(menu);
        let MenuClass::Hud(hud) = &owner.class else {
            panic!("not the HUD");
        };
        rows.push(format!(
            "HUD id={} state={} depth18={} mode={} zoom70={} pointer={} b8c={}",
            owner.id_20,
            owner.state_24,
            owner.depth_18,
            owner.menu_mode_14,
            hex(&[hud.zoom_70]),
            hex(&[hud.pointer_x_68, hud.pointer_y_6c]),
            hud.unknown_8c
        ));
        let slots = [
            (0x28, hud.tile_id1_28),
            (0x2C, hud.health_2c),
            (0x30, hud.magicka_30),
            (0x34, hud.fatigue_34),
            (0x38, hud.tile_id5_38),
            (0x3C, hud.tile_id6_3c),
            (0x40, hud.tile_id7_40),
            (0x44, hud.tile_id14_44),
            (0x48, hud.tile_id8_48),
            (0x4C, hud.grab_zone_4c),
            (0x50, hud.tile_id16_50),
            (0x54, hud.tile_id18_54),
            (0x58, hud.tile_id20_58),
        ];
        let names: Vec<String> = slots
            .iter()
            .map(|(offset, tile)| match tile {
                Some(tile) => format!("{offset:02X}={}", latin(&cx.engine.tile(*tile).name_08)),
                None => format!("{offset:02X}=null"),
            })
            .collect();
        rows.push(format!("SLOTS {}", names.join(" ")));
        tile_rows(cx, hud_tile, 0, &mut rows, &hex, &latin);
        rows
    })
    .unwrap()
}

// The tile lines of `phase17_rows.py` and `phase19_rows.py`.
fn tile_rows(
    cx: &recomp_game::tes::tile::InterfaceCx,
    tile: recomp_game::tes::tile::TileId,
    depth: u32,
    rows: &mut Vec<String>,
    hex: &dyn Fn(&[f32]) -> String,
    latin: &dyn Fn(&[u8]) -> String,
) {
    let own = cx.engine.tile(tile);
    use recomp_game::gamebryo::scene_node::{NiNodeChildren, SceneNodePayload};
    use recomp_game::tes::interface_scene::tile_scene_node;
    use recomp_game::tes::tile::TileKind;
    let kind = match own.kind {
        TileKind::Rect => "rect",
        TileKind::Image => "image",
        TileKind::Text => "text",
        TileKind::Model => "model",
        TileKind::Menu => "menu",
    };
    rows.push(format!(
        "TILE {depth} {kind} {} flags={:X} children={}",
        latin(&own.name_08),
        own.flags_2c,
        own.children_30.len()
    ));
    if let Some(node) = tile_scene_node(cx, tile) {
        let payload = cx.world.get::<SceneNodePayload>(node).unwrap();
        // Recorded substitutions: a tile's leaf geometry is a record (D3), and Tile3D's model isn't loaded (a counted boundary).
        let count = cx
            .world
            .get::<NiNodeChildren>(node)
            .map_or(0, |children| children.slots_ac.iter().flatten().count())
            + own
                .scene_24
                .as_ref()
                .map_or(0, |record| record.geometry.len())
            + usize::from(own.model_path_50.is_some());
        rows.push(format!(
            "NODE flags={:04X} pos={} scale={} nodechildren={count}",
            payload.effective_av_flags,
            hex(&payload.local_transform.translation),
            hex(&[payload.local_transform.scale])
        ));
    }
    for &value in &own.values_14 {
        let value = cx.engine.value(value);
        rows.push(format!(
            "VAL {} {} {} {}",
            value.trait_18,
            u8::from(value.is_number_1a),
            hex(&[value.number_04]),
            latin(&value.text_08)
        ));
    }
    for &child in &own.children_30 {
        tile_rows(cx, child, depth + 1, rows, hex, latin);
    }
}

// The rows `phase18_rows.py` reads from a snapshot at 0040F069.
fn phase_18_rows(world: &mut World) -> Vec<String> {
    use recomp_game::tes::actor::TesObjectRefr;
    use recomp_game::tes::ai_conditions::the_player;
    use recomp_game::tes::form::FormIdentity;
    use recomp_game::tes::form_components::{FormRef, TesSpellList};
    use recomp_game::tes::forms_magic::EffectItemList;
    use recomp_game::tes::hud_main_menu::HUD_MAIN_MENU_ID;
    use recomp_game::tes::os_globals::OsGlobalsData;
    use recomp_game::tes::player::PlayerCharacter;
    use recomp_game::tes::tes::TesGlobals;
    use recomp_game::tes::tile::MenuClass;
    use recomp_game::tes::tile_values::tile_string_trait;

    let player = the_player(world).unwrap();
    let base = world
        .get::<TesObjectRefr>(player)
        .unwrap()
        .base_form_1c
        .unwrap();
    let id = |world: &World, form: Entity| world.get::<FormIdentity>(form).unwrap().form_id_0c;
    let data = world.get::<PlayerCharacter>(player).unwrap();
    let mut rows = vec![format!(
        "PLAYER item={:08X} book={:08X} tk={}",
        id(world, data.active_magic_item_624.unwrap()),
        data.book_628,
        data.telekinesis_targeting_enabled_6e6
    )];
    let state = data.initial_state.clone().unwrap();
    for slot in &world.get::<TesSpellList>(base).unwrap().spells_04 {
        let FormRef::Resolved(Some(spell)) = *slot else {
            panic!("a spell slot that isn't a form");
        };
        let costs: Vec<String> = world
            .get::<EffectItemList>(spell)
            .unwrap()
            .items_04
            .iter()
            .map(|item| format!("{:08X}", item.cost_20.to_bits()))
            .collect();
        rows.push(format!(
            "SPELL {:08X} costs={}",
            id(world, spell),
            costs.join(" ")
        ));
    }
    let icon = recomp_startup::winmain::with_driver_interface(world, |cx| {
        let tile = cx.engine.tile_menu_by_id(HUD_MAIN_MENU_ID).unwrap();
        let menu = cx.engine.containing_menu(tile).unwrap();
        let MenuClass::Hud(hud) = &cx.engine.menu(menu).class else {
            panic!("the HUD's class");
        };
        let tile = hud.tile_id6_3c.unwrap();
        tile_string_trait(cx, tile, 0xFE6).unwrap_or_default()
    })
    .unwrap();
    rows.push(format!("HUDICON {}", String::from_utf8_lossy(&icon)));
    let save = world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .unwrap();
    let flags = |form: u32| {
        save.changes_00
            .get(form)
            .map_or(0, |record| record.flags_00)
    };
    rows.push(format!(
        "CHANGES base={:08X} player={:08X}",
        flags(id(world, base)),
        flags(id(world, player))
    ));
    rows.push(format!(
        "SAVE 7D={} 14={:08X}",
        save.byte_7d, save.fields_14[0]
    ));
    rows.push(format!("OS3 {}", world.resource::<OsGlobalsData>().unk_03));
    let limit = archive_buffer_limit(world).unwrap();
    rows.push(format!("LIMIT {limit:08X}"));
    for (index, chunk) in state.chunks(32).enumerate() {
        let hex: String = chunk.iter().map(|byte| format!("{byte:02X}")).collect();
        rows.push(format!("STATE {:04X} {hex}", index * 32));
    }
    rows
}

// Heap bytes the exe never stores, the same in run-h and run-i, as (offset in the state, length): process +0x88, +0xD8..+0xDF; controller +0x2E0..+0x2EF, +0x31C; actor +0x84, +0xD8; player +0x5A8.
const UNSTORED_STATE_BYTES: [(usize, usize); 7] = [
    (0x38C, 4),
    (0x3C5, 8),
    (0x494, 16),
    (0x4C0, 4),
    (0x530, 4),
    (0x539, 1),
    (0x558, 1),
];

fn mask_unstored_bytes(row: &str) -> String {
    let Some(rest) = row.strip_prefix("STATE ") else {
        return row.to_owned();
    };
    let (start, hex) = rest.split_once(' ').unwrap();
    let start = usize::from_str_radix(start, 16).unwrap();
    let mut hex = hex.as_bytes().to_vec();
    for byte in 0..hex.len() / 2 {
        let offset = start + byte;
        if UNSTORED_STATE_BYTES
            .iter()
            .any(|&(first, length)| (first..first + length).contains(&offset))
        {
            hex[byte * 2] = b'.';
            hex[byte * 2 + 1] = b'.';
        }
    }
    format!("STATE {start:04X} {}", String::from_utf8(hex).unwrap())
}

// run-i-phase18, seed 6AEE5389, snapshot-phase18_end: what the original holds at 0040F069 on the default path. Under Wine, with no audio and no saves.
#[test]
fn the_port_matches_the_phase_18_rows() {
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&real_data(), Some(0x6AEE_5389), Some(Duration::ZERO)),
        WinMainSeams::default(),
    );
    let mut schedule = winmain_schedule();
    // Phase 19 closes the LoadingMenu: these rows are 0040F069's.
    while world.resource::<WinMainProgress>().next != Block::CloseLoadingMenu {
        schedule.run(&mut world);
    }
    let stops = world.resource::<WinMainProgress>().stops.clone();
    assert!(stops.is_empty(), "{stops:?}");
    let port = phase_18_rows(&mut world);
    winmain_shutdown(&mut world);
    let expected = fixtures::rows("phase18");
    fixtures::ledger_read("phase18", &vec![true; expected.len()]);
    let port: Vec<String> = port.iter().map(|row| mask_unstored_bytes(row)).collect();
    let expected: Vec<String> = expected
        .iter()
        .map(|row| mask_unstored_bytes(row))
        .collect();
    let mut differences = Vec::new();
    for (index, (ours, theirs)) in port.iter().zip(&expected).enumerate() {
        if ours != theirs {
            differences.push(format!("row {index}\n  port {ours}\n  exe  {theirs}"));
        }
    }
    if port.len() != expected.len() {
        differences.push(format!("rows: port {}, exe {}", port.len(), expected.len()));
    }
    assert!(
        differences.is_empty(),
        "{} rows differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

// The rows `phase19_rows.py` reads from a snapshot at 0040F08D.
fn phase_19_rows(world: &mut World) -> Vec<String> {
    use recomp_game::bs::task_manager::TaskGlobals;
    use recomp_game::tes::data_handler::DataHandler;
    use recomp_game::tes::hud_reticle::HudReticle;
    use recomp_game::tes::interface_manager::InterfaceManager;
    let hex = |values: &[f32]| {
        values
            .iter()
            .map(|value| format!("{:08X}", value.to_bits()))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let latin = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|&byte| char::from(byte))
            .collect::<String>()
            .replace('\n', "\\n")
    };
    let manager = world.resource::<InterfaceManager>().clone();
    let mut rows = vec![format!(
        "MANAGER status={} stack={} mods={}",
        manager.menu_loading_status_008,
        manager
            .menu_mode_stack_0e0
            .iter()
            .map(|mode| format!("{mode:08X}"))
            .collect::<Vec<_>>()
            .join(" "),
        manager.input_modifier_flags
    )];
    let data = world.resource::<DataHandler>();
    for (index, id) in data.files_8c8.iter().enumerate() {
        let file = data.file_store.files[id.0].as_ref().unwrap();
        rows.push(format!(
            "FILE {index} master={} buffer={:08X}",
            u8::from(file.is_master()),
            file.buffer_size_228
        ));
    }
    let mut models: Vec<(String, u32)> = world
        .resource::<TaskGlobals>()
        .model_loader_b33a1c
        .as_ref()
        .unwrap()
        .models
        .buckets
        .iter()
        .flatten()
        .map(|entry| {
            let model = &world
                .resource::<TaskGlobals>()
                .model_loader_b33a1c
                .as_ref()
                .unwrap()
                .models
                .store[entry.model.0];
            (latin(&model.path_00), model.references_04 & 0xFFFF)
        })
        .collect();
    models.sort_by_key(|model| model.0.to_lowercase());
    let reticle = world.resource::<HudReticle>().clone();
    let tail = recomp_startup::winmain::with_driver_interface(world, |cx| {
        let mut rows = Vec::new();
        let menus: Vec<String> = (1001..1100)
            .filter(|&id| cx.engine.tile_menu_by_id(id).is_some())
            .map(|id| id.to_string())
            .collect();
        rows.push(format!("MENUS {}", menus.join(" ")));
        let name = |tile: Option<recomp_game::tes::tile::TileId>| {
            tile.map_or_else(
                || "null".to_owned(),
                |tile| latin(&cx.engine.tile(tile).name_08),
            )
        };
        rows.push(format!(
            "RETICLE crosshair={} detect={} sneak={} seconds={} alpha={} sneaking={}",
            name(reticle.crosshair_b3b350),
            name(reticle.detect_b3b358),
            name(reticle.sneak_b3b354),
            hex(&[
                reticle.crosshair_seconds_b140bc,
                reticle.detect_seconds_b140c0,
                reticle.sneak_seconds_b140c4
            ]),
            hex(&[reticle.sneak_alpha_b140b8]),
            reticle.sneaking_b3b361
        ));
        let mut tiles = Vec::new();
        tile_rows(
            cx,
            manager.hud_reticule_080.unwrap(),
            0,
            &mut tiles,
            &hex,
            &latin,
        );
        (rows, tiles)
    })
    .unwrap();
    let (mut head, tiles) = tail;
    let reticle_row = head.pop().unwrap();
    rows.insert(1, head.pop().unwrap());
    rows.push(reticle_row);
    for (path, references) in models {
        if !path.to_lowercase().starts_with("meshes\\menus\\") {
            rows.push(format!("MODEL {path} references={references}"));
        }
    }
    rows.extend(tiles);
    rows
}

// run-k-phase19, seed 6AEE7210, snapshot-phase19_end: what the original holds at 0040F08D on the default path. Under Wine, with no audio and no saves.
#[test]
fn the_port_matches_the_phase_19_rows() {
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&real_data(), Some(0x6AEE_7210), Some(Duration::ZERO)),
        WinMainSeams::default(),
    );
    let mut schedule = winmain_schedule();
    // Phase 20 makes the MainMenu: these rows are 0040F08D's.
    while world.resource::<WinMainProgress>().next != Block::MainMenu {
        schedule.run(&mut world);
    }
    let stops = world.resource::<WinMainProgress>().stops.clone();
    assert!(stops.is_empty(), "{stops:?}");
    let port = phase_19_rows(&mut world);
    winmain_shutdown(&mut world);
    let expected = fixtures::rows("phase19");
    fixtures::ledger_read("phase19", &vec![true; expected.len()]);
    let mut differences = Vec::new();
    for (index, (ours, theirs)) in port.iter().zip(&expected).enumerate() {
        if ours != theirs {
            differences.push(format!("row {index}\n  port {ours}\n  exe  {theirs}"));
        }
    }
    if port.len() != expected.len() {
        differences.push(format!("rows: port {}, exe {}", port.len(), expected.len()));
    }
    assert!(
        differences.is_empty(),
        "{} rows differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

// The rows `phase20_rows.py` reads from a snapshot at 0040F092.
fn phase_20_rows(world: &mut World) -> Vec<String> {
    use recomp_game::tes::interface_manager::InterfaceManager;
    use recomp_game::tes::tile::MenuClass;
    use recomp_startup::winmain_menu_media::MainMenuMedia;
    let hex = |values: &[f32]| {
        values
            .iter()
            .map(|value| format!("{:08X}", value.to_bits()))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let latin = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|&byte| char::from(byte))
            .collect::<String>()
            .replace('\n', "\\n")
    };
    let manager = world.resource::<InterfaceManager>().clone();
    let media = *world.resource::<MainMenuMedia>();
    let title = world
        .resource::<recomp_game::tes::os_globals::OsGlobalsData>()
        .sound_24
        .as_ref()
        .expect("the original's run has a sound object")
        .main_menu_music_b3c0ec;
    recomp_startup::winmain::with_driver_interface(world, |cx| {
        let mut rows = vec![format!(
            "MANAGER status={} stack={} mods={} priority={} geometry={} active={:X},{:X}",
            manager.menu_loading_status_008,
            manager
                .menu_mode_stack_0e0
                .iter()
                .map(|mode| format!("{mode:08X}"))
                .collect::<Vec<_>>()
                .join(" "),
            manager.input_modifier_flags,
            manager.default_selection_priority_08c,
            manager.geometry_changed_07c,
            u32::from(manager.active_tile_098.is_some()),
            u32::from(manager.active_menu_09c.is_some())
        )];
        let menus: Vec<String> = (1001..1100)
            .filter(|&id| cx.engine.tile_menu_by_id(id).is_some())
            .map(|id| id.to_string())
            .collect();
        rows.push(format!("MENUS {}", menus.join(" ")));
        rows.push(format!(
            "MEDIA backdrop={} constructed={} title={title}",
            media.backdrop_movie_b3b408, media.menu_constructed_b3b409
        ));
        assert!(cx.host.save_file_list().is_empty());
        rows.push("SAVES file=0 next=0".to_owned());
        let root = cx.engine.tile_menu_by_id(1044).unwrap();
        let id = cx.engine.containing_menu(root).unwrap();
        let menu = cx.engine.menu(id);
        let MenuClass::Main(main) = &menu.class else {
            panic!("the class");
        };
        let name = |tile: Option<recomp_game::tes::tile::TileId>| {
            tile.map_or_else(
                || "null".to_owned(),
                |tile| latin(&cx.engine.tile(tile).name_08),
            )
        };
        rows.push(format!(
            "MENU {} {} {} {} {} {} {}",
            name(main.continue_button_28),
            name(main.new_game_button_30),
            name(main.load_button_34),
            name(main.options_button_38),
            name(main.credits_button_3c),
            name(main.exit_button_40),
            name(main.backdrop_44)
        ));
        rows.push(format!(
            "MENUSTATE mode={:X} state={} unk4c={} pending={}",
            menu.menu_mode_14, menu.state_24, main.unk_4c, main.confirmation_pending_4d
        ));
        tile_rows(cx, root, 0, &mut rows, &hex, &latin);
        rows
    })
    .unwrap()
}

// run-m-phase20, seed 6AEE84C8, snapshot-phase20_end: what the original holds at 0040F092 on the default path. Under Wine, with no audio and no saves.
#[test]
fn the_port_matches_the_phase_20_rows() {
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&real_data(), Some(0x6AEE_84C8), Some(Duration::ZERO)),
        WinMainSeams::default(),
    );
    let mut schedule = winmain_schedule();
    // Phase 21 starts the intro's fade: these rows are 0040F092's.
    while world.resource::<WinMainProgress>().next != Block::IntroSequence {
        schedule.run(&mut world);
    }
    let stops = world.resource::<WinMainProgress>().stops.clone();
    assert!(stops.is_empty(), "{stops:?}");
    let port = phase_20_rows(&mut world);
    winmain_shutdown(&mut world);
    let expected = fixtures::rows("phase20");
    fixtures::ledger_read("phase20", &vec![true; expected.len()]);
    let mut differences = Vec::new();
    for (index, (ours, theirs)) in port.iter().zip(&expected).enumerate() {
        if ours != theirs {
            differences.push(format!("row {index}\n  port {ours}\n  exe  {theirs}"));
        }
    }
    if port.len() != expected.len() {
        differences.push(format!("rows: port {}, exe {}", port.len(), expected.len()));
    }
    assert!(
        differences.is_empty(),
        "{} rows differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

// The rows `phase21_rows.py` reads from a snapshot at 0040F0E0: Phase 20's, and the movie player's after the media.
fn phase_21_rows(world: &mut World) -> Vec<String> {
    use recomp_game::tes::movie_player::MoviePlayer;
    use recomp_game::tes::os_globals::{OsGlobalsData, lock_shared};
    let player = world.resource::<MoviePlayer>().clone();
    let globals = &player.globals;
    let movie = lock_shared(&globals.current_b33428).clone().unwrap();
    let fields = movie.fields().clone();
    let os = world.resource::<OsGlobalsData>();
    let sound = os.sound_24.as_ref().unwrap();
    let extra = [
        format!(
            "MOVIE state={} scale={:08X} x={:08X} y={:08X} thread={} waiter={} cached={} valid={} paused={}",
            movie.state(),
            fields.scale_14.to_bits(),
            fields.x_18.to_bits(),
            fields.y_1c.to_bits(),
            u32::from(lock_shared(&globals.thread_b33434).is_some()),
            globals
                .waiter_active_b33425
                .load(std::sync::atomic::Ordering::Acquire),
            u32::from(lock_shared(&globals.cached_b33440).is_some()),
            globals
                .cached_valid_b33431
                .load(std::sync::atomic::Ordering::Acquire),
            u32::from(lock_shared(&globals.paused_b3342c).is_some()),
        ),
        format!(
            "MUSIC flags={:X} type={}",
            sound.music_flags_dc, sound.current_music_type_b0
        ),
    ];
    let mut rows = phase_20_rows(world);
    rows.splice(3..3, extra);
    rows
}

// run-o-phase21, seed 6AEE9060, snapshot-phase21_end: what the original holds at 0040F0E0 on the default path. Under Wine, with no audio and no saves.
#[test]
fn the_port_matches_the_phase_21_rows() {
    use std::sync::{Arc, Mutex};
    // The run's Bink is 1280x720, by its scale of 1.5 at a height of 1080. Its RenderFile fails at 006AB3A4: `+0x70` holds a graph, `+0x74` is null and the path is empty.
    let mut bink = recomp_game::tes::bink_library::ScriptedBink::default();
    bink.width = 1280;
    bink.height = 720;
    let mut music = recomp_game::tes::os_sound_music::ScriptedMusicDevice::default();
    music.fail_render = true;
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&real_data(), Some(0x6AEE_9060), None),
        WinMainSeams {
            bink: Arc::new(Mutex::new(bink)),
            music: Arc::new(Mutex::new(music)),
            ..WinMainSeams::default()
        },
    );
    run_winmain_straight_through(&mut world);
    let stops = world.resource::<WinMainProgress>().stops.clone();
    assert!(stops.is_empty(), "{stops:?}");
    let port = phase_21_rows(&mut world);
    winmain_shutdown(&mut world);
    let expected = fixtures::rows("phase21");
    fixtures::ledger_read("phase21", &vec![true; expected.len()]);
    let mut differences = Vec::new();
    for (index, (ours, theirs)) in port.iter().zip(&expected).enumerate() {
        if ours != theirs {
            differences.push(format!("row {index}\n  port {ours}\n  exe  {theirs}"));
        }
    }
    if port.len() != expected.len() {
        differences.push(format!("rows: port {}, exe {}", port.len(), expected.len()));
    }
    assert!(
        differences.is_empty(),
        "{} rows differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

// run-q-phase22, seed 6AEE9ABF, snapshot-idle_frame_end: what the original holds at 0040F275, after the first idle frame. Under Wine, with no audio and no saves.
#[test]
fn the_port_matches_the_phase_22_rows() {
    use recomp_game::tes::interface_frame::InterfaceInputFrame;
    use recomp_startup::winmain::driver_frame_seconds;
    use recomp_startup::winmain_menu_media::run_main_menu_media_frame;
    use recomp_startup::winmain_screens::{open_main_menu, run_screens_frame};
    use std::sync::{Arc, Mutex};
    // As Phase 21's: the Bink's size and the failed RenderFile are the run's.
    let mut bink = recomp_game::tes::bink_library::ScriptedBink::default();
    bink.width = 1280;
    bink.height = 720;
    let mut music = recomp_game::tes::os_sound_music::ScriptedMusicDevice::default();
    music.fail_render = true;
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&real_data(), Some(0x6AEE_9ABF), None),
        WinMainSeams {
            bink: Arc::new(Mutex::new(bink)),
            music: Arc::new(Mutex::new(music)),
            ..WinMainSeams::default()
        },
    );
    run_winmain_straight_through(&mut world);
    let stops = world.resource::<WinMainProgress>().stops.clone();
    assert!(stops.is_empty(), "{stops:?}");
    open_main_menu(&mut world).unwrap();
    run_main_menu_media_frame(&mut world);
    run_screens_frame(&mut world, &InterfaceInputFrame::default()).unwrap();
    let frame = driver_frame_seconds(&world);
    let mut port = phase_21_rows(&mut world);
    port.insert(3, format!("TIME frame={:08X}", frame.to_bits()));
    winmain_shutdown(&mut world);
    let expected = fixtures::rows("phase22");
    fixtures::ledger_read("phase22", &vec![true; expected.len()]);
    let mut differences = Vec::new();
    for (index, (ours, theirs)) in port.iter().zip(&expected).enumerate() {
        if ours != theirs {
            differences.push(format!("row {index}\n  port {ours}\n  exe  {theirs}"));
        }
    }
    if port.len() != expected.len() {
        differences.push(format!("rows: port {}, exe {}", port.len(), expected.len()));
    }
    assert!(
        differences.is_empty(),
        "{} rows differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

// run-s-newgame, seed 6AEEB452, snapshot-new_game_end: what the original holds where 005B5DF0 returns, called at the first 0040F26A with +0xB0 = 1. Under Wine, with no audio and no saves. The Bink's size and the failed RenderFile are the run's, and one listed file stands for the folder: the roll's result isn't compared here, its draw is.
#[test]
fn the_port_matches_the_new_game_rows() {
    use recomp_game::tes::menus::main_menu_new_game;
    use recomp_startup::winmain::{main_thread_crt_state, with_driver_interface};
    use recomp_startup::winmain_screens::open_main_menu;
    use std::sync::{Arc, Mutex};
    let mut bink = recomp_game::tes::bink_library::ScriptedBink::default();
    bink.width = 1280;
    bink.height = 720;
    let mut music = recomp_game::tes::os_sound_music::ScriptedMusicDevice::default();
    music.fail_render = true;
    music.listing = vec![b"a.mp3".to_vec()];
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&real_data(), Some(0x6AEE_B452), None),
        WinMainSeams {
            bink: Arc::new(Mutex::new(bink)),
            music: Arc::new(Mutex::new(music)),
            ..WinMainSeams::default()
        },
    );
    run_winmain_straight_through(&mut world);
    let stops = world.resource::<WinMainProgress>().stops.clone();
    assert!(stops.is_empty(), "{stops:?}");
    open_main_menu(&mut world).unwrap();
    let before = main_thread_crt_state(&world).unwrap();
    with_driver_interface(&mut world, main_menu_new_game)
        .unwrap()
        .unwrap();
    let after = main_thread_crt_state(&world).unwrap();
    assert_eq!(after.1, before.1 + 1, "006A8E80's one draw");
    assert_eq!(before.0, 0xE420_CA25, "_holdrand at idle_frame_start");
    assert_eq!(after.0, 0x67E5_1454, "_holdrand at new_game_end");
    let frame = recomp_startup::winmain::driver_frame_seconds(&world);
    let quest = recomp_game::tes::forms_init::lookup(&world, 0x2466E).unwrap();
    let flags = world
        .get::<recomp_game::tes::forms_dialogue::TesQuest>(quest)
        .unwrap()
        .flags_3c;
    let player = recomp_game::tes::ai_conditions::the_player(&world).unwrap();
    let intro = world
        .get::<recomp_game::tes::player::PlayerCharacter>(player)
        .unwrap()
        .new_game_intro_pending_7f8;
    let mut port = phase_21_rows(&mut world);
    port.insert(3, format!("TIME frame={:08X}", frame.to_bits()));
    port.insert(4, format!("NEWGAME intro={intro} quest={flags}"));
    winmain_shutdown(&mut world);
    let expected = fixtures::rows("newgame");
    fixtures::ledger_read("newgame", &vec![true; expected.len()]);
    let mut differences = Vec::new();
    for (index, (ours, theirs)) in port.iter().zip(&expected).enumerate() {
        if ours != theirs {
            differences.push(format!("row {index}\n  port {ours}\n  exe  {theirs}"));
        }
    }
    if port.len() != expected.len() {
        differences.push(format!("rows: port {}, exe {}", port.len(), expected.len()));
    }
    assert!(
        differences.is_empty(),
        "{} rows differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

// Not a compare: how far a new game gets, printing its first stop. `cargo test new_game_as_far_as_it_goes -- --ignored --nocapture`.
#[test]
#[ignore = "a probe, for the next arm to port"]
fn new_game_as_far_as_it_goes() {
    use recomp_game::tes::interface_frame::InterfaceInputFrame;
    use recomp_game::tes::interface_manager::InterfaceManager;
    use recomp_game::tes::menus::main_menu_new_game;
    use recomp_game::tes::script_vm::ScriptVmCounts;
    use recomp_startup::winmain::with_driver_interface;
    use recomp_startup::winmain_menu_media::{MainMenuMedia, run_main_menu_media_frame};
    use recomp_startup::winmain_screens::{open_main_menu, run_screens_frame};
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&real_data(), Some(0x6AEE_D7A4), None),
        WinMainSeams::default(),
    );
    run_winmain_straight_through(&mut world);
    open_main_menu(&mut world).unwrap();
    // The menu fades in first: 00584740 only acts on a visible one, and no click comes sooner.
    for _ in 0..150 {
        run_main_menu_media_frame(&mut world);
        run_screens_frame(&mut world, &InterfaceInputFrame::default()).unwrap();
        std::thread::sleep(Duration::from_millis(16));
    }
    with_driver_interface(&mut world, main_menu_new_game)
        .unwrap()
        .unwrap();
    for frame in 0..14000 {
        run_main_menu_media_frame(&mut world);
        run_screens_frame(&mut world, &InterfaceInputFrame::default()).unwrap();
        std::thread::sleep(Duration::from_millis(16));
        let status = world.resource::<InterfaceManager>().menu_loading_status_008;
        if frame % 40 == 0 {
            let counts = world.resource::<ScriptVmCounts>();
            println!(
                "frame {frame}: status {status}, runs {}, gamemode {}",
                counts.statement_runs, counts.block_tests[0]
            );
        }
        if world.resource::<MainMenuMedia>().scripts_stopped {
            println!("stopped at frame {frame}, status {status}");
            break;
        }
    }
    for command in &world.resource::<ScriptVmCounts>().commands {
        println!(
            "CMD {} ref={:X} script={:X}",
            command.0, command.1, command.2
        );
    }
    winmain_shutdown(&mut world);
}

// run-v-moveto, seed 6AEED7A4, `newgame_commands_rows.py`: the original's commands from the confirmation to the player's MoveTo, and its state there. The port stops at the last, command 64, the player's MoveTo, so every other command's effect is compared.
// IDA: N/A a New Game from the main menu through the scripts' frames to their stop (command 64's move and the intro movie past
// it), with `seed`.
fn run_new_game_commands(seed: u32) -> World {
    use recomp_game::tes::interface_frame::InterfaceInputFrame;
    use recomp_game::tes::menus::main_menu_new_game;
    use recomp_startup::winmain::with_driver_interface;
    use recomp_startup::winmain_menu_media::{MainMenuMedia, run_main_menu_media_frame};
    use recomp_startup::winmain_screens::{open_main_menu, run_screens_frame};
    // One listed file stands for a music folder, so 006A8E80 draws: run-y's at menu frame 2, and the New Game's.
    let mut music = recomp_game::tes::os_sound_music::ScriptedMusicDevice::default();
    music.fail_render = true;
    music.listing = vec![b"a.mp3".to_vec()];
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&real_data(), Some(seed), None),
        WinMainSeams {
            music: std::sync::Arc::new(std::sync::Mutex::new(music)),
            ..WinMainSeams::default()
        },
    );
    run_winmain_straight_through(&mut world);
    open_main_menu(&mut world).unwrap();
    for _ in 0..150 {
        run_main_menu_media_frame(&mut world);
        run_screens_frame(&mut world, &InterfaceInputFrame::default()).unwrap();
        std::thread::sleep(Duration::from_millis(16));
    }
    with_driver_interface(&mut world, main_menu_new_game)
        .unwrap()
        .unwrap();
    let begun = std::time::Instant::now();
    while !world.resource::<MainMenuMedia>().scripts_stopped {
        run_main_menu_media_frame(&mut world);
        if world.resource::<MainMenuMedia>().scripts_stopped {
            break;
        }
        run_screens_frame(&mut world, &InterfaceInputFrame::default()).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        assert!(begun.elapsed() < Duration::from_secs(300), "no stop");
    }
    world
}

// Build 1's rand check: with run-x's seed (6AEF4049) command 64, the player's MoveTo, begins at `_holdrand` 5D949CF2; its
// main-thread draws to the scripts' stop are the 286 the port matched before 0057E150's StatsMenu was built plus the
// menu's 24 (run-aa: 3 at 005904A3, 21 at 005854EB), 251 of them by the menu's end.
#[test]
fn the_move_draws_through_the_stats_menu_match_run_x() {
    let world = run_new_game_commands(0x6AEF_4049);
    let counts = world.resource::<recomp_game::tes::script_vm::ScriptVmCounts>();
    let (start_holdrand, start_draws) = counts.command_crt[63];
    assert_eq!(counts.commands[63].0, 4254, "command 64 is the MoveTo");
    assert_eq!(
        start_holdrand, 0x5D94_9CF2,
        "command 64's starting _holdrand"
    );
    let (_, after_stats) = world
        .resource::<recomp_game::tes::interface_manager::BigFourMenuBoundaries>()
        .crt_after_stats
        .expect("0057E150 ran");
    let (_, end) = main_thread_crt_state(&world).unwrap();
    // The 59 Sky draws (004424D0) follow the drain, as in the original.
    assert_eq!(
        after_stats - start_draws,
        251,
        "the move's draws through the Stats menu"
    );
    // Build 2: run-aa's main-thread draws by site (records-run-aa-movedraws.txt, seed 6AEF4E91, 478 to after_move): 7 before the
    // loading menu (run-x's seed makes 9: its leveled picks), 192 slides (005AD5CF), 1 at 006A9D8B and 11 attach sounds
    // (006B6EA1), 14 actors (00629024, 005E2F2C), 24 Stats traits (0058C483), 132 variant picks (00470C14: the nine living NPCs'
    // 00475D80 in run-af's finish order, 108, then the player's two, 24), the camera block's 6 (0047E086, 0054C452, 0047FB96:
    // Build 3's gap, D16) and the Sky's 59 (004EED11); the 32 after it follow the intro movie's stop. Replaying the LCG from
    // command 64's start puts the port's recorded picks at the same place in the stream.
    let picks: Vec<i32> = world
        .resource::<recomp_game::tes::actor_animation::AnimationVariantDraws>()
        .draws
        .iter()
        .map(|&(_, _, draw)| draw)
        .collect();
    assert_eq!(picks.len(), 132, "the variant picks");
    let mut replay = recomp_core::crt::MsvcRand::default();
    replay.srand(start_holdrand);
    let stream: Vec<i32> = (start_draws..end).map(|_| replay.rand()).collect();
    assert_eq!(
        &stream[251..251 + 132],
        &picks[..],
        "the picks follow the Stats menu's 24"
    );
    assert_eq!(
        end - start_draws,
        9 + 192 + 12 + 14 + 24 + 132 + 59,
        "the move's draws, to the stop at the intro movie, with the camera block's 6 a gap"
    );
}

#[test]
fn the_port_runs_the_new_game_commands() {
    use recomp_game::tes::actor::Actor;
    use recomp_game::tes::ai_conditions::the_player;
    use recomp_game::tes::form::FormIdentity;
    use recomp_game::tes::forms_dialogue::TesQuest;
    use recomp_game::tes::forms_init::lookup;
    use recomp_game::tes::player::PlayerCharacter;
    use recomp_game::tes::script_vm::ScriptVmCounts;
    use recomp_startup::winmain::with_driver_interface;
    let rows = fixtures::rows("newgame_commands");
    fixtures::ledger_read("newgame_commands", &vec![true; rows.len()]);
    let mut world = run_new_game_commands(0x6AEE_D7A4);
    // Build 1's node check against run-af's after_move: no reference has 3D the original lacks, every non-actor's 3D matches,
    // and nothing is behind a host gate (`Load3DHostGates`); the 12 actors are Build 2's.
    {
        use recomp_game::tes::actor::TesObjectRefr;
        use recomp_game::tes::reference_3d::{Load3DHostGates, Reference3D};
        let gates: Vec<(u32, &str)> = world
            .get_resource::<Load3DHostGates>()
            .map(|gates| {
                gates
                    .gated
                    .iter()
                    .map(|&(reference, why)| {
                        (
                            world.get::<FormIdentity>(reference).unwrap().form_id_0c,
                            why,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let original = fixtures::rows("after_move_nodes");
        let cell = lookup(&world, 0x1FBB9).unwrap();
        let mut query = world.query::<(Entity, &TesObjectRefr, &FormIdentity)>();
        let port: std::collections::BTreeMap<u32, Entity> = query
            .iter(&world)
            .filter(|(_, refr, _)| refr.parent_cell_40 == Some(cell))
            .map(|(entity, _, id)| (id.form_id_0c, entity))
            .collect();
        assert_eq!(port.len(), original.len(), "the prison's references");
        let mut matched = 0;
        for row in &original {
            let fields: Vec<&str> = row.strip_prefix("NODE ").unwrap().split(' ').collect();
            let id = u32::from_str_radix(fields[0], 16).unwrap();
            let kind = u8::from_str_radix(fields[1], 16).unwrap();
            let node = fields[2] == "1";
            let entity = port[&id];
            let has = world.get::<Reference3D>(entity).is_some();
            assert!(node || !has, "{id:X} has 3D the original lacks");
            if !matches!(kind, 0x23 | 0x24) {
                assert_eq!(node, has, "{id:X} (type {kind:X}): 3D against after_move");
            }
            matched += usize::from(node == has);
        }
        assert!(
            gates.is_empty(),
            "no reference is behind a host gate: {gates:X?}"
        );
        assert_eq!(
            matched, 468,
            "references whose 3D matches after_move (every one: the eleven actors' Load3D and the player's 00667BE0 third person)"
        );
    }
    // Command 64 runs to the intro movie: a stop raised earlier, in the drain's finish queue say, would end it before the drain's
    // tail without failing the checks above.
    assert_eq!(
        world
            .resource::<recomp_startup::winmain_menu_media::MainMenuMedia>()
            .scripts_stop,
        Some("0066F260: the intro movie (+0x7F8) and the rest of 0066EAF0"),
        "where command 64 stops"
    );
    // Build 2: each actor task (QueuedCharacter 0043DC00, QueuedCreature 0043DD80) queues its children and finishes on the main
    // thread (0043B090) through Character's or the creature's Load3D.
    {
        use recomp_game::tes::character_3d::CharacterTaskStops;
        let tasks = world
            .get_resource::<CharacterTaskStops>()
            .cloned()
            .unwrap_or_default();
        let id = |reference: Entity| world.get::<FormIdentity>(reference).unwrap().form_id_0c;
        let stops: Vec<(u32, &str)> = tasks
            .stops
            .iter()
            .map(|&(reference, stop)| (id(reference), stop))
            .collect();
        // The player's task (QueuedPlayer, queued by 004D35D0 at 004D3882 with 00440C80's priority): 00662F60 (the base's
        // skeleton model, the first-person KF files and skeleton, the base's KF files), then 0043DC00 (the outfit equipped
        // through 005F3140 with 006600D0's skin info, the slots, the QueuedHead, no birth sign's spells) and the skeleton; its
        // finish in 00434020(5) (0043B090) runs PlayerCharacter's Load3D (00667BE0): its TESNPC_AutoEquip unequips the worn
        // stacks (00486030) and re-equips the outfit (each skin slot's form through 005F2E70's unstacked clothing arm), then
        // 006670C0; the first-person skeleton (NIF 10.1.0.106) under "Player1stPerson", its skin info and anim data; then
        // Character_Load3D for the third person (its 00475D80 setup); the first person's KF list into 00475D80 on +0x5CC; the
        // process's weapon and anim nodes, the "Player" name and its Camera01 (00B3BB10); 00578CF0's reticle (queued), 005E4DD0,
        // no vampirism, +0x16C cleared and 005EA1A0; then vt+0x148, 0065B750 with the controller Phase 11 made, stops.
        assert_eq!(
            stops,
            [(
                0x14,
                "00667FBE: vt+0x148 0065B750 with the player's controller (its phantom's 3D, the cell's Havok world, 0088D070) and the controller's vt+0x90"
            )],
            "the actor tasks' stops: {stops:X?}"
        );
        // In finish order, run-af's: the goblin shaman (15985A) and the skeleton (26BE1), dead creatures (QueuedCreature, 0043DD80:
        // their KF files, body models, slot-13 armor, weapon and ammo; the skeleton's arrows need bhkPrismaticConstraint), through
        // Load3D and 004E3490's creature arm (0051E240's equip, the folder's Idle.KF, no draws, 00475D80, 00476D10, LABEL_94);
        // then the prisoner, the three Blades, the Emperor and the four Mythic Dawn: their worn parts (0047BC40), 00475D80 with
        // the 12 picks each, the Idle's update (the B-spline bones, the accum root's 006CF780, the skeleton's controllers), and
        // the NPC arm's tail (00656270's weapon on its sheath, 00476D10, LABEL_94).
        let finished: Vec<u32> = tasks
            .finished
            .iter()
            .map(|&reference| id(reference))
            .collect();
        assert_eq!(
            finished,
            [
                0x15985A, 0x26BE1, 0x1FC43, 0x32A15, 0x32A16, 0x32A17, 0x32A18, 0x14A2C, 0x14A30,
                0x178C3, 0x178C4
            ],
            "the actors' finishes in run-af's order"
        );
        // 004E3490's creature arm: each dead creature's ActorAnimData (its MiddleHigh process's +0x17C) holds only the Idle
        // (group 0): its skeleton folder's list is "Idle.KF" alone, as 0043D000's queue found (neither folder has a Death.KF), and
        // 00476D10 ran.
        for id in [0x15985A_u32, 0x26BE1] {
            let reference = lookup(&world, id).unwrap();
            let process = world
                .get::<recomp_game::tes::actor::MobileObject>(reference)
                .unwrap()
                .process_58
                .unwrap();
            let anim = world
                .get::<recomp_game::tes::actor_process::MiddleHighProcess>(process)
                .unwrap()
                .anim_data_17c
                .unwrap();
            let data = world
                .get::<recomp_game::tes::actor_animation::ActorAnimData>(anim)
                .unwrap();
            assert_eq!(data.updated_00, 1, "{id:X}'s 00476D10");
            let groups: Vec<u16> = data
                .sequences_9c
                .iter()
                .flatten()
                .map(|(key, _)| *key)
                .collect();
            assert_eq!(groups, [0], "{id:X}'s animation groups");
        }
        // 00470BF0's picks: per NPC, twelve, for the second and later KF of each group that can be multiple (AttackLeft and
        // AttackRight by HandToHand, OneHand and TwoHand, Recoil by OneHand and TwoHand), in KF list order; then the player's 24
        // (00667BE0's two 00475D80 setups, the third person's in Character_Load3D, then the first person's).
        use recomp_game::tes::actor_animation::AnimationVariantDraws;
        let draws = &world.resource::<AnimationVariantDraws>().draws;
        assert_eq!(draws.len(), 108 + 24, "the variant picks");
        let mut per_actor: Vec<(u32, Vec<u16>)> = Vec::new();
        for &(actor, group, _) in draws {
            let id = world.get::<FormIdentity>(actor).unwrap().form_id_0c;
            match per_actor.last_mut() {
                Some((last, groups)) if *last == id => groups.push(group),
                _ => per_actor.push((id, vec![group])),
            }
        }
        let order: Vec<u32> = per_actor.iter().map(|(id, _)| *id).collect();
        assert_eq!(
            order,
            [
                0x1FC43, 0x32A15, 0x32A16, 0x32A17, 0x32A18, 0x14A2C, 0x14A30, 0x178C3, 0x178C4,
                0x14
            ],
            "the picks' actors in order"
        );
        let player_groups = &per_actor.last().unwrap().1;
        assert_eq!(
            (
                player_groups[..12] == per_actor[0].1[..],
                player_groups[12..] == per_actor[0].1[..]
            ),
            (true, true),
            "the player's two setups in the prisoner's order: {player_groups:X?}"
        );
        for (id, groups) in &per_actor[..9] {
            let mut sorted = groups.clone();
            sorted.sort_unstable();
            assert_eq!(
                sorted,
                [
                    0x114, 0x114, 0x115, 0x115, 0x214, 0x215, 0x21E, 0x21E, 0x314, 0x315, 0x31E,
                    0x31E
                ],
                "{id:X}'s variant groups"
            );
            assert_eq!(
                groups, &per_actor[0].1,
                "{id:X}'s picks in the prisoner's order"
            );
        }
        // The skin info's slots of each NPC past 005268D0 against run-af's after_move (skin_slots.rows): form and model path.
        use recomp_game::tes::character_3d::{SkinInfo, skin_info, skin_model_path};
        let original = fixtures::rows("skin_slots");
        for id in [
            0x1FC43_u32,
            0x32A15,
            0x32A16,
            0x32A17,
            0x32A18,
            0x14A2C,
            0x14A30,
            0x178C3,
            0x178C4,
        ] {
            let want: Vec<String> = original
                .iter()
                .filter_map(|row| row.strip_prefix(&format!("SKIN {id:X} ")))
                .map(str::to_owned)
                .collect();
            let reference = lookup(&world, id).unwrap();
            let skin = skin_info(&world, reference).expect("a skin info");
            let info = world.get::<SkinInfo>(skin).unwrap().clone();
            let got: Vec<String> = info
                .slots_4c
                .iter()
                .enumerate()
                .filter_map(|(slot, held)| {
                    let form = held.form_00?;
                    let path = skin_model_path(&world, held.model_04);
                    let path = if path.is_empty() {
                        "-".to_owned()
                    } else {
                        String::from_utf8_lossy(&path).into_owned()
                    };
                    let form_id = world.get::<FormIdentity>(form).unwrap().form_id_0c;
                    Some(format!("{slot:X} {form_id:X} {path}"))
                })
                .collect();
            assert_eq!(got, want, "{id:X}'s skin slots against after_move");
            // 0047BC40: each slot with a model has its part on the skeleton, named "<slot>  (<form>)" (the form's vt+0xD4 is
            // ""), skinned for the body slots and the amulet; slot 1 (the helmet or hood) waits for its QueuedHelmet (D12).
            const NAMES: [&str; 16] = [
                "Head",
                "Hair",
                "UpperBody",
                "LowerBody",
                "Hand",
                "Foot",
                "RightRing",
                "LeftRing",
                "Amulet",
                "Weapon",
                "BackWeapon",
                "SideWeapon",
                "Quiver",
                "Shield",
                "Torch",
                "Tail",
            ];
            for (slot, held) in info.slots_4c.iter().enumerate() {
                if held.model_04.is_none() {
                    continue;
                }
                let name = held
                    .node_08
                    .and_then(|node| {
                        world.get::<recomp_game::gamebryo::object_net::NiObjectNet>(node)
                    })
                    .and_then(|net| net.name.clone())
                    .map(|name| String::from_utf8_lossy(&name).into_owned());
                if slot == 1 {
                    assert_eq!(name, None, "{id:X}'s helmet waits for its QueuedHelmet");
                    continue;
                }
                let form = world
                    .get::<FormIdentity>(held.form_00.unwrap())
                    .unwrap()
                    .form_id_0c;
                assert_eq!(
                    name,
                    Some(format!("{}  ({form:08X})", NAMES[slot])),
                    "{id:X}'s slot {slot} part"
                );
                assert_eq!(
                    held.skinned_0c,
                    u32::from(matches!(slot, 2..=5 | 8)),
                    "{id:X}'s slot {slot} skinned"
                );
            }
        }
        // The player's skin info from QueuedPlayer's 0043DC00 (005268D0's outfit) against after_move's: its 3D (00667BE0)
        // isn't built yet, so only the slots' forms and models.
        {
            let want: Vec<String> = original
                .iter()
                .filter_map(|row| row.strip_prefix("SKIN 14 "))
                .map(str::to_owned)
                .collect();
            let player = lookup(&world, 0x14).unwrap();
            let skin = skin_info(&world, player).expect("the player's skin info");
            let info = world.get::<SkinInfo>(skin).unwrap().clone();
            let got: Vec<String> = info
                .slots_4c
                .iter()
                .enumerate()
                .filter_map(|(slot, held)| {
                    let form = held.form_00?;
                    let path = skin_model_path(&world, held.model_04);
                    let path = if path.is_empty() {
                        "-".to_owned()
                    } else {
                        String::from_utf8_lossy(&path).into_owned()
                    };
                    let form_id = world.get::<FormIdentity>(form).unwrap().form_id_0c;
                    Some(format!("{slot:X} {form_id:X} {path}"))
                })
                .collect();
            assert_eq!(got, want, "the player's skin slots against after_move");
        }
        // Goal 4: the player's 3D block against run-af's after_move (after_move_player.rows): +0x3C and +0x5D0, their names
        // and +0x18 flags, 00667BE0's three nodes (00B3BB0C, 00B3BB10, 00B3BB14), +0x588, +0x5D4, +0x5C8/+0x5CC and the
        // process's +0x16C.
        {
            use recomp_game::gamebryo::object_net::NiObjectNet;
            use recomp_game::gamebryo::scene_node::SceneNodePayload;
            use recomp_game::tes::actor_process::MiddleHighProcess;
            use recomp_game::tes::player::PlayerCharacter;
            use recomp_game::tes::player_3d::FirstPersonNodes;
            use recomp_game::tes::reference_3d::Reference3D;
            let player = lookup(&world, 0x14).unwrap();
            let node_row = |field: &str, node: Option<Entity>| match node {
                None => format!("PNODE {field} - -"),
                Some(node) => {
                    let name = world
                        .get::<NiObjectNet>(node)
                        .and_then(|net| net.name.clone())
                        .map_or("-".to_owned(), |name| {
                            String::from_utf8_lossy(&name).into_owned()
                        });
                    let flags = world
                        .get::<SceneNodePayload>(node)
                        .unwrap()
                        .effective_av_flags;
                    format!("PNODE {field} {name} {flags:X}")
                }
            };
            let data = world.get::<PlayerCharacter>(player).unwrap();
            let nodes = *world.resource::<FirstPersonNodes>();
            let process = world
                .get::<recomp_game::tes::actor::MobileObject>(player)
                .unwrap()
                .process_58
                .unwrap();
            let got = vec![
                node_row(
                    "3C",
                    world.get::<Reference3D>(player).map(|loaded| loaded.0),
                ),
                node_row("5D0", data.first_person_node_5d0),
                node_row("B3BB0C", nodes.camera_b3bb0c),
                node_row("B3BB10", nodes.third_person_camera_b3bb10),
                node_row("B3BB14", nodes.bip_b3bb14),
                format!("PBYTE 588 {:X}", data.view_flags_588[0]),
                format!(
                    "PBITS 5D4 {:08X}",
                    data.camera_eye_height_offset_5d4.to_bits()
                ),
                format!(
                    "PSET 5C8 {}",
                    u8::from(data.first_person_skin_info_5c8.is_some())
                ),
                format!(
                    "PSET 5CC {}",
                    u8::from(data.first_person_anim_data_5cc.is_some())
                ),
                format!(
                    "PBYTE 16C {:X}",
                    world
                        .get::<MiddleHighProcess>(process)
                        .unwrap()
                        .equipment_update_pending_16c
                ),
            ];
            // run-ag (tick 32) settles the four rows held at the port's values. +0x588 becomes 1 in 0066C580
            // PlayerCharacter_SetFirstPerson(0), from 005CC17F RaceSexMenu_Create <- 0057B8DE <- 0066F29A, the intro block past
            // the port's 0066F260 stop; its 00664F70(0) swaps +0x3C's and +0x5D0's bit 0. The third person's Camera01 loses 0x10
            // in 0088A802 (NiAVObject_VisitCollisionObjects's flags arm) under 0065B750's re-bind and 00668016, the Havok block
            // at the player's stop. The rest must equal after_move's.
            let mut want = fixtures::rows("after_move_player");
            let open = [
                ("PNODE 3C Player 6", "PNODE 3C Player 7"),
                (
                    "PNODE 5D0 Player1stPerson 47",
                    "PNODE 5D0 Player1stPerson 46",
                ),
                ("PNODE B3BB10 Camera01 6", "PNODE B3BB10 Camera01 16"),
                ("PBYTE 588 1", "PBYTE 588 0"),
            ];
            for (original, port) in open {
                let row = want
                    .iter_mut()
                    .find(|row| *row == original)
                    .expect("after_move's row");
                *row = port.to_owned();
            }
            assert_eq!(got, want, "the player's 3D block against after_move");
        }
        // The finished actor tasks have left the loader's +0x08 map (the finish's removal), as the original's do.
        {
            use recomp_game::bs::task_manager::TaskGlobals;
            let tasks = world.resource::<TaskGlobals>();
            let loader = tasks.model_loader_b33a1c.as_ref().unwrap();
            for id in [
                0x15985A_u32,
                0x26BE1,
                0x1FC43,
                0x32A15,
                0x32A16,
                0x32A17,
                0x32A18,
                0x14A2C,
                0x14A30,
                0x178C3,
                0x178C4,
            ] {
                let reference = lookup(&world, id).unwrap();
                assert!(
                    !loader.references.map_08.contains_key(&reference),
                    "{id:X}'s task left the map"
                );
            }
        }
        // FaceGen (background agent): each QueuedHead the walk ran built its NPC's morphed parts, equal to run-af's after_move
        // heads (facegen_nodes.rows) bit for bit: the face, mouth, teeth and tongue. The ears' 10.0.1.0 NIFs fail the stream
        // loader's header (LoadModelMesh's warning); the nodes, hair, eyes and textures are D12/D13.
        {
            use recomp_game::tes::face_gen_tasks::FaceGenTasks;
            let fnv = |positions: &[[f32; 3]]| {
                let mut hash = 0x811C_9DC5_u32;
                for value in positions.iter().flatten() {
                    for byte in value.to_bits().to_le_bytes() {
                        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
                    }
                }
                hash
            };
            let original = fixtures::rows("facegen_nodes");
            let heads = world.resource::<FaceGenTasks>();
            let mut checked = Vec::new();
            for id in [
                0x1FC43_u32,
                0x32A15,
                0x32A16,
                0x32A17,
                0x32A18,
                0x14A2C,
                0x14A30,
                0x178C3,
                0x178C4,
            ] {
                let reference = lookup(&world, id).unwrap();
                // The actor task has finished and left the map; its head is found by the base (QueuedHead +0x20).
                let base = world
                    .get::<recomp_game::tes::actor::TesObjectRefr>(reference)
                    .unwrap()
                    .base_form_1c
                    .unwrap();
                let head = heads
                    .heads
                    .values()
                    .find(|head| head.base_20 == base)
                    .expect("a QueuedHead for the base");
                let got: Vec<String> = head
                    .parts
                    .iter()
                    .map(|part| {
                        format!(
                            "{} {} {:X} {:X}",
                            if part.skinned { "skinned" } else { "biped" },
                            part.name,
                            part.positions.len(),
                            fnv(&part.positions)
                        )
                    })
                    .collect();
                let want: Vec<String> = original
                    .iter()
                    .filter_map(|row| row.strip_prefix(&format!("FGNODE {id:X} ")))
                    .map(|row| row.split(' ').collect::<Vec<_>>())
                    .filter(|fields| {
                        [
                            "FaceGenFace",
                            "FaceGenMouth",
                            "FaceGenTeethLower",
                            "FaceGenTeethUpper",
                            "FaceGenTongue",
                        ]
                        .contains(&fields[2])
                    })
                    .map(|fields| {
                        format!("{} {} {} {}", fields[0], fields[2], fields[4], fields[11])
                    })
                    .collect();
                assert_eq!(want.len(), 5, "{id:X}'s rows");
                let mut got_sorted = got.clone();
                got_sorted.sort();
                let mut want_sorted = want.clone();
                want_sorted.sort();
                assert_eq!(
                    got_sorted, want_sorted,
                    "{id:X}'s FaceGen parts against after_move"
                );
                checked.push(id);
            }
            assert_eq!(
                checked,
                [
                    0x1FC43, 0x32A15, 0x32A16, 0x32A17, 0x32A18, 0x14A2C, 0x14A30, 0x178C3, 0x178C4
                ]
            );
        }
    }
    // The commands, in order, as far as the port goes. Command 30 enables 90028F5; command 31 enables the marker 90013EA and with it its child 90013E8, an actor with a process. Frame 88's 004FA580 drain disables command 33's 90013E6 and its three priests. The stop is at command 64, the player's MoveTo, the first cell load.
    let expected: Vec<(u32, u32, u32)> = rows
        .iter()
        .filter_map(|line| line.strip_prefix("CMD "))
        .map(|line| {
            let mut fields = line.split(' ');
            (
                fields.next().unwrap().parse().unwrap(),
                u32::from_str_radix(fields.next().unwrap(), 16).unwrap(),
                u32::from_str_radix(fields.next().unwrap(), 16).unwrap(),
            )
        })
        .collect();
    let ran = world.resource::<ScriptVmCounts>().commands.clone();
    assert_eq!(ran.len(), 64, "{ran:X?}");
    assert_eq!(ran[..], expected[..]);
    assert_eq!(ran[30], (4129, 0x0900_13EA, 0));
    assert_eq!(ran[42], (4300, 0x0006_C400, 0));
    assert_eq!(ran[63], (4254, 0x14, 0));
    // 004CB590's sounds as the prison attaches (run-ae): the 11 sound references' instances by ID, each with its WAV's data bytes; the torch's folder gives amb_torchmounted_lp_02.wav.
    let sounds: Vec<(u32, u32, u32)> = world
        .resource::<recomp_game::tes::game_sounds::GameSounds>()
        .instances_300
        .values()
        .map(|instance| (instance.id_0c, instance.buffer_bytes_08, instance.flags_00))
        .collect();
    assert_eq!(
        sounds,
        [
            (0xB9CE, 0x19FA0, 0x2012),
            (0xB9CF, 0x6E000, 0x2012),
            (0xB9D0, 0x19FA0, 0x2012),
            (0xB9D1, 0x28926, 0x2012),
            (0xB9D2, 0x52600, 0x2012),
            (0xB9D3, 0x5D87C, 0x2011),
            (0xB9D4, 0x33888, 0x2012),
            (0xC6AF, 0x67000, 0x2012),
            (0xC6B0, 0x33888, 0x2012),
            (0x655C9, 0xDDA6E, 0x2012),
            (0x8ECD3, 0x2E20C, 0x2012),
        ]
    );
    // 004D5BD0's loop on the prison (run-ab, run-ac, run-ad): its 11 actors rebuilt as HighProcesses (005FA320), three of them, with no process before, placed on the path grid (005EB400), and the abilities 005EDA20 casts (counted at 0069AF30), base spells then race spells, in run-ad's order.
    for id in [
        0x15985A, 0x1FC43, 0x32A15, 0x32A16, 0x32A17, 0x32A18, 0x14A2C, 0x14A30, 0x178C3, 0x178C4,
        0x26BE1,
    ] {
        let actor = recomp_game::tes::forms_init::lookup(&world, id).unwrap();
        assert_eq!(
            recomp_game::tes::actor::actor_process_level(&world, actor),
            0,
            "{id:X}"
        );
    }
    assert_eq!(
        world
            .resource::<recomp_game::tes::actor_placement::PathingLocationCounts>()
            .built,
        3
    );
    // run-ad's 004E5730: the cheapest of the five points in buckets FFFA0006 and FFFB0006, and its first connection.
    let placements: Vec<(u32, usize, usize)> = world
        .resource::<recomp_game::tes::actor_placement::PathingLocationCounts>()
        .placements
        .iter()
        .map(|&(actor, point, next)| {
            (
                recomp_game::tes::forms_init::form_id(&world, actor),
                point,
                next,
            )
        })
        .collect();
    assert_eq!(
        placements,
        [
            (0x14A2C, 272, 268),
            (0x178C3, 272, 268),
            (0x178C4, 268, 272)
        ]
    );
    // run-ab: 464 of the 467 queued at priority 1 (00440C80 for an interior), the three with flag 0x800 skipped, then the player's own at 0066ED70, also 1.
    let priorities = &world
        .resource::<recomp_game::tes::tes_grid::TesGridBoundaries>()
        .model_queue_priorities;
    assert_eq!(priorities.len(), 465);
    assert!(priorities.iter().all(|&priority| priority == 1));
    let casts: Vec<(u32, u32)> = world
        .resource::<recomp_game::tes::cell_reference_loop::CastBoundaries>()
        .casts
        .iter()
        .map(|&(actor, spell)| {
            (
                recomp_game::tes::forms_init::form_id(&world, actor),
                recomp_game::tes::forms_init::form_id(&world, spell),
            )
        })
        .collect();
    assert_eq!(
        casts,
        [
            (0x1FC43, 0x47AD4),
            (0x32A15, 0x47AD1),
            (0x32A15, 0x47AD2),
            (0x32A17, 0x47AE5),
            (0x32A17, 0x47AE6),
            (0x14A2C, 0x3E91B),
            (0x14A30, 0x3E91B),
            (0x178C3, 0x3E91B),
            (0x178C4, 0x3E91B),
        ]
    );
    // MainMenu's destructor (005B5870) asks for the big-four rebuild; the cell entry's drain runs 0057B950, whose 0057E150 builds the Stats (0057A440, 005DCB70), Inventory (0057A2D0), Magic (0057A180, 005B3760), Map (00579F80) and QuickKeys (005C1290) menus from their XML and clears the flag.
    assert_eq!(
        world
            .resource::<recomp_game::tes::interface_manager::InterfaceManager>()
            .big_four_rebuild_pending_094,
        0
    );
    assert!(
        world
            .resource::<recomp_game::tes::interface_manager::BigFourMenuBoundaries>()
            .calls
            .is_empty(),
        "nothing of 0057E150 is only counted"
    );
    // 005DCB70: the Stats menu (1003) from stats_menu.xml, with its 13 XML fields and the 21 stat_skill_template rows.
    // 005C1290: the QuickKeys menu (1046) from quickkeys_menu.xml with its ten fields, and 005C1000's destroyOnClose 1.
    let quick_keys = with_driver_interface(&mut world, |cx| {
        let tile = cx.engine.tile_menu_by_id(1046)?;
        let menu = cx.engine.containing_menu(tile)?;
        match &cx.engine.menu(menu).class {
            recomp_game::tes::tile::MenuClass::QuickKeys(quick_keys) => Some((
                quick_keys.has_required_tiles(),
                recomp_game::tes::tile_values::tile_numeric_trait(
                    cx,
                    tile,
                    recomp_game::tes::interface_frame::TRAIT_DESTROY_ON_CLOSE,
                ),
            )),
            _ => None,
        }
    })
    .flatten();
    assert_eq!(quick_keys, Some((true, 1.0)), "the QuickKeys menu");
    // 00579F80: the Map menu (1023) with its twenty fields, 005B8FC0's place and time on the root, tab 1 (map_background's
    // user0) in InterfaceManager +0x0D, and the menu hidden, so 005BA4D0's local map isn't drawn.
    let map = with_driver_interface(&mut world, |cx| {
        use recomp_game::tes::tile_values::{
            TRAIT_USER0, TRAIT_VISIBLE, tile_numeric_trait, tile_string_trait,
        };
        let tile = cx.engine.tile_menu_by_id(1023)?;
        let menu = cx.engine.containing_menu(tile)?;
        let recomp_game::tes::tile::MenuClass::Map(map) = &cx.engine.menu(menu).class else {
            return None;
        };
        Some((
            map.has_required_tiles(),
            tile_string_trait(cx, tile, TRAIT_USER0 + 1)
                .map(|text| String::from_utf8_lossy(&text).into_owned()),
            tile_string_trait(cx, tile, TRAIT_USER0 + 2)
                .map(|text| String::from_utf8_lossy(&text).into_owned()),
            cx.world
                .resource::<recomp_game::tes::interface_manager::InterfaceManager>()
                .tab_ids_00a[3],
            tile_numeric_trait(cx, tile, TRAIT_VISIBLE),
        ))
    })
    .flatten();
    let magic = with_driver_interface(&mut world, |cx| {
        use recomp_game::tes::tile_values::{TRAIT_USER0, tile_numeric_trait, tile_string_trait};
        let tile = cx.engine.tile_menu_by_id(1022)?;
        let menu = cx.engine.containing_menu(tile)?;
        let recomp_game::tes::tile::MenuClass::Magic(magic) = &cx.engine.menu(menu).class else {
            return None;
        };
        let text = |tile, n| {
            tile_string_trait(cx, tile, TRAIT_USER0 + n)
                .map(|text| String::from_utf8_lossy(&text).into_owned())
        };
        let pane = magic.tiles_28[1]?;
        let rows: Vec<_> = cx
            .engine
            .tile(pane)
            .children_30
            .iter()
            .filter(|&&row| tile_numeric_trait(cx, row, 4008) >= 1001.0)
            .map(|&row| {
                (
                    text(row, 1),
                    text(row, 5),
                    text(row, 6),
                    [7, 8, 9, 10, 11, 13].map(|n| tile_numeric_trait(cx, row, TRAIT_USER0 + n)),
                    tile_numeric_trait(cx, row, 4010),
                    Some(row) == magic.selected_row_4c,
                )
            })
            .collect();
        Some((
            rows,
            text(tile, 6),
            [1, 2, 3, 4, 5].map(|n| tile_numeric_trait(cx, tile, TRAIT_USER0 + n)),
        ))
    })
    .flatten();
    // 005B3760: the Magic menu's rows as run-af's after_move snapshot holds them (MagicMenu 2001D870's pane, walked from
    // the head): name, cost, icon, user7..user11 and user13, listindex, the selected row; then the root's user6 and
    // user1..user5.
    let (magic_rows, effectiveness, headings) = magic.expect("the Magic menu");
    let row =
        |name: &str, cost: &str, icon: &str, numbers: [f32; 6], index: f32, selected: bool| {
            (
                Some(name.to_owned()),
                Some(cost.to_owned()),
                Some(icon.to_owned()),
                numbers,
                index,
                selected,
            )
        };
    assert_eq!(
        magic_rows,
        [
            row(
                "Heal Minor Wounds",
                "19",
                "Icons\\Magic\\restoration_icons\\restore_restoration.dds",
                [4.0, 2.0, 4.0, 2.0, 1.0, 2.0],
                1.0,
                true
            ),
            row(
                "Flare",
                "14",
                "Icons\\Magic\\destruction_icons\\fire_damage.dds",
                [4.0, 2.0, 1.0, 1.0, 0.0, 1.0],
                0.0,
                false
            ),
        ]
    );
    assert_eq!(effectiveness.as_deref(), Some("Spell effectiveness: 100%"));
    {
        use recomp_game::tes::inventory_entries::{
            entry_is_worn, reference_inventory_count, reference_inventory_entry,
        };
        let player = recomp_game::tes::ai_conditions::the_player(&world).unwrap();
        let count = reference_inventory_count(&mut world, player).unwrap();
        let mut seen = Vec::new();
        for index in 0..count {
            let entry = reference_inventory_entry(&mut world, player, index).unwrap();
            seen.push(entry.map(|entry| {
                (
                    world
                        .get::<recomp_game::tes::form::FormIdentity>(entry.form_08)
                        .unwrap()
                        .form_id_0c,
                    entry.count_04,
                    entry_is_worn(&entry, false),
                    entry.lists_00.as_ref().map(Vec::len),
                )
            }));
        }
        // 004D8950 / 004D88F0 at the stop: the four prison clothes in base-container order, unworn, and 0048D6C0's one past
        // the end, worn: QueuedPlayer's 0043DC00 equipped them (005268D0), and 00667BE0's TESNPC_AutoEquip unequipped
        // (00486030) and re-equipped them, as the original ends (after_move's skin slots, asserted above). run-af's
        // InventoryMenu rows, from 0057E150 before either, hold them unworn at user11 0..3 with user10 1.
        let entry = |form: u32| Some((form, 1, true, Some(1)));
        assert_eq!(count, 5);
        assert_eq!(
            seen,
            [
                entry(0x27318),
                entry(0x27319),
                entry(0x2731A),
                entry(0xBE335),
                None
            ]
        );
    }
    assert_eq!(headings, [-1.0, -1.0, -1.0, 999_999.0, 1.0]);
    // 0057A2D0: the Inventory menu's rows and root as run-af's after_move snapshot holds them (InventoryMenu 039E3BC4,
    // its pane from the head): name, listindex, id, xdefault, user2..user13; the root's user1..user9.
    let inventory = with_driver_interface(&mut world, |cx| {
        use recomp_game::tes::tile_values::{TRAIT_USER0, tile_numeric_trait, tile_string_trait};
        let tile = cx.engine.tile_menu_by_id(1002)?;
        let menu = cx.engine.containing_menu(tile)?;
        let recomp_game::tes::tile::MenuClass::Inventory(inventory) = &cx.engine.menu(menu).class
        else {
            return None;
        };
        let text = |tile, n| {
            tile_string_trait(cx, tile, TRAIT_USER0 + n)
                .map(|text| String::from_utf8_lossy(&text).into_owned())
        };
        let pane = inventory.tiles_28[1]?;
        let rows: Vec<_> = cx
            .engine
            .tile(pane)
            .children_30
            .iter()
            .filter(|&&row| tile_numeric_trait(cx, row, 4008) >= 1001.0)
            .map(|&row| {
                (
                    String::from_utf8_lossy(&cx.engine.tile(row).name_08).into_owned(),
                    [4010, 4008, 4080].map(|t| tile_numeric_trait(cx, row, t)),
                    [2, 3, 4, 5, 6, 7, 13].map(|n| text(row, n).unwrap_or_default()),
                    [8, 9, 10, 11, 12].map(|n| tile_numeric_trait(cx, row, TRAIT_USER0 + n)),
                )
            })
            .collect();
        Some((
            rows,
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 13, 14, 15, 16]
                .map(|n| tile_numeric_trait(cx, tile, TRAIT_USER0 + n)),
        ))
    })
    .flatten();
    let (inventory_rows, inventory_root) = inventory.expect("the Inventory menu");
    let s = |text: &str| text.to_owned();
    let clothes = |name: &str,
                   index: f32,
                   id: f32,
                   xdefault: f32,
                   value: &str,
                   weight: &str,
                   icon: &str,
                   at: f32| {
        (
            s(name),
            [index, id, xdefault],
            [s(value), s(weight), s(" "), s(" "), s(icon), s(" "), s(" ")],
            [2.0, 2.0, 1.0, at, 0.0],
        )
    };
    assert_eq!(
        inventory_rows,
        [
            clothes(
                "Wrist_Irons",
                3.0,
                1004.0,
                1.0,
                "0",
                "0",
                "Icons\\Clothes\\LowerClass\\Jail\\M\\JailShirtHandcuff.dds",
                3.0
            ),
            clothes(
                "Sack_Cloth_Shirt",
                2.0,
                1003.0,
                1.0,
                "1",
                "1",
                "Icons\\Clothes\\LowerClass\\05\\M\\Shirt.dds",
                1.0
            ),
            clothes(
                "Sack_Cloth_Sandals",
                1.0,
                1002.0,
                1.0,
                "1",
                "2",
                "Icons\\Clothes\\LowerClass\\05\\M\\Shoes.dds",
                2.0
            ),
            clothes(
                "Sack_Cloth_Pants",
                0.0,
                1001.0,
                102.0,
                "1",
                "1",
                "Icons\\Clothes\\LowerClass\\05\\M\\Pants.dds",
                0.0
            ),
        ]
    );
    assert_eq!(
        inventory_root,
        [
            -1.0, -1.0, 3.0, 3.0, 3.0, 4.0, 250.0, 0.0, 0.0, 3.0, 3.0, 3.0, 3.0
        ]
    );
    // The hour is the port's clock at the stop, not yet compared with a snapshot, so only the date is checked.
    let (tiles, place, title, tab, visible) = map.expect("the Map menu");
    assert!(tiles, "the Map menu's twenty fields");
    assert_eq!(place.as_deref(), Some("Imperial Prison"));
    assert!(
        title
            .as_deref()
            .is_some_and(|title| title.starts_with("Last Seed 27, 3E433 ")),
        "{title:?}"
    );
    assert_eq!((tab, visible), (1, 1.0), "tab 1, hidden");
    let skills = with_driver_interface(&mut world, |cx| {
        let tile = cx.engine.tile_menu_by_id(1003)?;
        let menu = cx.engine.containing_menu(tile)?;
        match &cx.engine.menu(menu).class {
            recomp_game::tes::tile::MenuClass::Stats(stats) => Some((
                stats.has_required_tiles(),
                stats.skill_tiles_60.iter().flatten().count(),
            )),
            _ => None,
        }
    })
    .flatten();
    assert_eq!(
        skills,
        Some((true, 21)),
        "the Stats menu and its skill rows"
    );
    assert!(
        world
            .resource::<recomp_game::tes::script_vm::ScriptVmGlobals>()
            .disabled_b361cc
            .is_empty()
    );
    assert_eq!(expected.len(), 64);
    // What the commands past the stop reach is known by their references, and their quests by name below.
    // An Enable or Disable reaches its 0x40 children too.
    let mut later: Vec<u32> = expected[63..].iter().map(|row| row.1).collect();
    let mut at = 0;
    while at < later.len() {
        let children = lookup(&world, later[at])
            .and_then(|form| world.get::<recomp_game::tes::actor::TesObjectRefr>(form))
            .and_then(|refr| refr.extra_data_44.get(0x40))
            .map(|entry| match &entry.data {
                recomp_game::tes::base_extra_list::ExtraData::EnableStateChildren(children) => {
                    children.clone()
                }
                _ => Vec::new(),
            })
            .unwrap_or_default();
        for child in children {
            let id = world.get::<FormIdentity>(child).unwrap().form_id_0c;
            if !later.contains(&id) {
                later.push(id);
            }
        }
        at += 1;
    }
    let id_of = |world: &World, form: Entity| world.get::<FormIdentity>(form).unwrap().form_id_0c;
    let player = the_player(&world).unwrap();
    let topics: Vec<u32> = world
        .get::<PlayerCharacter>(player)
        .unwrap()
        .known_topics
        .iter()
        .map(|&topic| id_of(&world, topic))
        .collect();
    let mut differences = Vec::new();
    for line in &rows {
        let fields: Vec<&str> = line.split(' ').collect();
        match fields[0] {
            // Every AddTopic is before the stop.
            "TOPICS" => {
                let want: Vec<u32> = fields[1..]
                    .iter()
                    .map(|id| u32::from_str_radix(id, 16).unwrap())
                    .collect();
                assert_eq!(topics, want);
            }
            // Commands 45, 46 and 54's MoveTos: the parent cell, then the position's and the rotation's bits.
            "PLACE" => {
                let id = u32::from_str_radix(fields[1], 16).unwrap();
                let form = lookup(&world, id).unwrap();
                let refr = world
                    .get::<recomp_game::tes::actor::TesObjectRefr>(form)
                    .unwrap();
                let cell = refr.parent_cell_40.map_or(0, |cell| id_of(&world, cell));
                let bits: Vec<String> = refr
                    .position_2c
                    .iter()
                    .chain(refr.rotation_20.iter())
                    .map(|value| format!("{:08X}", value.to_bits()))
                    .collect();
                assert_eq!(
                    format!("{cell:X} {}", bits.join(" ")),
                    fields[2..].join(" "),
                    "{line}"
                );
            }
            // Commands 48 to 52's SetUnconscious.
            "DEADSTATE" => {
                let id = u32::from_str_radix(fields[1], 16).unwrap();
                let form = lookup(&world, id).unwrap();
                assert_eq!(
                    world.get::<Actor>(form).unwrap().dead_state_b0.to_string(),
                    fields[2],
                    "{line}"
                );
            }
            // SetInChargen, command 58.
            "CHARGEN" => {
                let chargen = world
                    .get::<PlayerCharacter>(player)
                    .unwrap()
                    .is_in_chargen_6e5;
                assert_eq!(chargen.to_string(), fields[1], "{line}");
            }
            "DISPOSITION" => {
                let id = u32::from_str_radix(fields[1], 16).unwrap();
                let form = lookup(&world, id).unwrap();
                let list: Vec<String> = world
                    .get::<Actor>(form)
                    .unwrap()
                    .disposition_list
                    .iter()
                    .map(|entry| {
                        format!(
                            "{:08X},{:X}",
                            entry.modifier,
                            id_of(&world, entry.reference)
                        )
                    })
                    .collect();
                assert_eq!(list.join(" "), fields[2..].join(" "), "{line}");
            }
            "AV" => {
                let id = u32::from_str_radix(fields[1], 16).unwrap();
                let form = lookup(&world, id).unwrap();
                let list: Vec<String> = world
                    .get::<Actor>(form)
                    .unwrap()
                    .av_modifiers_88
                    .list_00
                    .iter()
                    .map(|entry| format!("{},{:08X}", entry.code_00, entry.value_04.to_bits()))
                    .collect();
                assert_eq!(list.join(" "), fields[2..].join(" "), "{line}");
            }
            "QUEST" => {
                let id = u32::from_str_radix(fields[1], 16).unwrap();
                let form = lookup(&world, id).unwrap();
                let quest = world.get::<TesQuest>(form).unwrap();
                let done: Vec<String> = quest
                    .stages_40
                    .iter()
                    .filter(|group| group.flags_01 != 0)
                    .map(|group| group.index_00.to_string())
                    .collect();
                let mine = format!(
                    "QUEST {id:X} {:X} {} {}",
                    quest.flags_3c,
                    quest.byte_5c,
                    done.join(",")
                );
                // Every quest command is before the stop, so all are compared: `StopQuest` by 2D685 and 78C6A, `SetStage` by 2480B and `StartQuest` in that stage's script.
                if mine != *line {
                    differences.push(format!("{mine} / {line}"));
                }
            }
            // Command 30's actor and command 31's child, each reset and equipped by its Enable. The child's gold is its load's roll less 1 (0048DF80) plus the Enable's roll, so its count holds both draws.
            "INVENTORY" => {
                let id = u32::from_str_radix(fields[1], 16).unwrap();
                let form = lookup(&world, id).unwrap();
                let refr = world
                    .get::<recomp_game::tes::actor::TesObjectRefr>(form)
                    .unwrap();
                let entries: Vec<String> =
                    match refr.extra_data_44.get(0x1A).map(|entry| &entry.data) {
                        Some(recomp_game::tes::base_extra_list::ExtraData::ContainerChanges(
                            changes,
                        )) => changes
                            .entries_00
                            .iter()
                            .map(|entry| {
                                let stacks: Vec<String> = entry
                                    .lists_00
                                    .iter()
                                    .flatten()
                                    .map(|list| {
                                        list.entries
                                            .iter()
                                            .map(|extra| format!("{:X}", extra.type_04))
                                            .collect::<Vec<_>>()
                                            .join("+")
                                    })
                                    .collect();
                                format!(
                                    "{:X},{},{}",
                                    id_of(&world, entry.form_08),
                                    entry.count_04,
                                    stacks.join("/")
                                )
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                // 90013E8's gold is 0048DF80's 4 less 1 plus the Enable's pick, and 35CAD's is command 53's Enable's pick: the 62nd and 65th draws from idle_frame_end, past the menu's and the New Game's music picks and the sky's 59 weather picks (run-w, run-z).
                assert_eq!(entries.join(" "), fields[2..].join(" "), "{line}");
            }
            "PROCESS" => {
                let id = u32::from_str_radix(fields[1], 16).unwrap();
                let form = lookup(&world, id).unwrap();
                let vtable = world
                    .get::<recomp_game::tes::actor::MobileObject>(form)
                    .unwrap()
                    .process_58
                    .map_or(0, |process| {
                        world
                            .get::<recomp_game::tes::form::OriginalVtable>(process)
                            .unwrap()
                            .0
                    });
                assert_eq!(format!("{vtable:X}"), fields[2], "{line}");
            }
            // Actor_Update's rotation (00649CC9's turn, wrapped), +0xFD and +0x100, and the process's procedure index and +0x84: the original's later frames keep them.
            "ACTORSTATE" => {
                let id = u32::from_str_radix(fields[1], 16).unwrap();
                let form = lookup(&world, id).unwrap();
                let rotation = world
                    .get::<recomp_game::tes::actor::TesObjectRefr>(form)
                    .unwrap()
                    .rotation_20;
                let actor = world.get::<Actor>(form).unwrap();
                let process = world
                    .get::<recomp_game::tes::actor::MobileObject>(form)
                    .unwrap()
                    .process_58
                    .unwrap();
                let index = world
                    .get::<recomp_game::tes::actor_process::BaseProcess>(process)
                    .unwrap()
                    .edit_pack_procedure_type_04;
                let byte_84 = world
                    .get::<recomp_game::tes::actor_process::LowProcess>(process)
                    .unwrap()
                    .byte_84;
                let mine = format!(
                    "{:08X} {:08X} {:08X} {} {:08X} {index} {byte_84}",
                    rotation[0].to_bits(),
                    rotation[1].to_bits(),
                    rotation[2].to_bits(),
                    actor.merchant_respawn_day_processed_fd,
                    actor.head_track_target_scan_timer_100.to_bits()
                );
                // 35CAD's +0xFD: command 53's Actor_Update sets it, as for 90028F5, and the original's is 0, which a later Actor_Update in frames 93 to 97 would give once the stamp is today. The port runs no process manager frame, so that one field is set aside.
                if id == 0x0003_5CAD {
                    fixtures::ledger_relaxed(
                        "newgame_commands",
                        "ACTORSTATE 35CAD +0xFD",
                        1,
                        "frame-dependent: the original's later frames update the actor, the port's don't",
                    );
                    let mask = |row: &str| {
                        let mut parts: Vec<String> = row.split(' ').map(str::to_owned).collect();
                        parts[3] = "*".to_owned();
                        parts.join(" ")
                    };
                    assert_eq!(mask(&mine), mask(&fields[2..].join(" ")), "{line}");
                    continue;
                }
                assert_eq!(mine, fields[2..].join(" "), "{line}");
            }
            "EXTRAS" => {
                let id = u32::from_str_radix(fields[1], 16).unwrap();
                let form = lookup(&world, id).unwrap();
                let types: Vec<String> = world
                    .get::<recomp_game::tes::actor::TesObjectRefr>(form)
                    .unwrap()
                    .extra_data_44
                    .entries
                    .iter()
                    .map(|entry| format!("{:X}", entry.type_04))
                    .collect();
                assert_eq!(types.join(" "), fields[2..].join(" "), "{line}");
            }
            "PACKAGE" => {
                let id = u32::from_str_radix(fields[1], 16).unwrap();
                let form = lookup(&world, id).unwrap();
                let process = world
                    .get::<recomp_game::tes::actor::MobileObject>(form)
                    .unwrap()
                    .process_58
                    .unwrap();
                let package = world
                    .get::<recomp_game::tes::actor_process::BaseProcess>(process)
                    .unwrap()
                    .editor_package_08
                    .map_or(0, |package| id_of(&world, package));
                assert_eq!(format!("{package:X}"), fields[2], "{line}");
            }
            "PACKAGESTART" => {
                let id = u32::from_str_radix(fields[1], 16).unwrap();
                let form = lookup(&world, id).unwrap();
                let entry = world
                    .get::<recomp_game::tes::actor::TesObjectRefr>(form)
                    .unwrap()
                    .extra_data_44
                    .get(0x1E)
                    .cloned();
                let Some(entry) = entry else {
                    assert_eq!(fields[2], "none", "{line}");
                    continue;
                };
                let recomp_game::tes::base_extra_list::ExtraData::PackageStart {
                    location_10,
                    time_1c,
                    ..
                } = entry.data
                else {
                    panic!("{line}");
                };
                let location = entry
                    .resolved_0c
                    .flatten()
                    .map_or(0, |form| id_of(&world, form));
                let mine = format!(
                    "{location:X} {:08X} {:08X} {:08X} {time_1c:08X}",
                    location_10[0], location_10[1], location_10[2]
                );
                assert_eq!(mine, fields[2..].join(" "), "{line}");
            }
            "DISABLED" => {
                let id = u32::from_str_radix(fields[1], 16).unwrap();
                let form = lookup(&world, id).unwrap();
                if world.get::<FormIdentity>(form).unwrap().flags_08 & 0x800 == 0
                    && !later.contains(&id)
                {
                    differences.push(format!("{line}: enabled in the port"));
                }
            }
            _ => {}
        }
    }
    let wanted: std::collections::HashSet<u32> = rows
        .iter()
        .filter_map(|line| line.strip_prefix("DISABLED "))
        .map(|id| u32::from_str_radix(id, 16).unwrap())
        .collect();
    let references: Vec<Entity> = world
        .query_filtered::<Entity, With<recomp_game::tes::actor::TesObjectRefr>>()
        .iter(&world)
        .collect();
    // Command 64's cell load, which the port reaches before its stop, brings in the prison's references (1FBB9) with their own flags; run-v's snapshot is from before the move, so they're past it.
    let prison = lookup(&world, 0x0001_FBB9);
    for reference in references {
        let identity = world.get::<FormIdentity>(reference).unwrap();
        let in_prison = world
            .get::<recomp_game::tes::actor::TesObjectRefr>(reference)
            .is_some_and(|refr| refr.parent_cell_40.is_some() && refr.parent_cell_40 == prison);
        if identity.flags_08 & 0x800 != 0
            && !wanted.contains(&identity.form_id_0c)
            && !later.contains(&identity.form_id_0c)
            && !in_prison
        {
            differences.push(format!("{:X}: disabled in the port", identity.form_id_0c));
        }
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
    winmain_shutdown(&mut world);
}

// x32log-census-seeded.txt, seed 6AEEA906: what the original's script VM ran from boot to 0040F26A, the first idle frame. It logged 00517330's entries, each handler by its target, and each function at 004FADD5.
#[test]
fn the_script_vm_runs_what_the_census_counted() {
    use recomp_game::tes::script_vm::ScriptVmCounts;
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&real_data(), Some(0x6AEE_A906), None),
        WinMainSeams::default(),
    );
    run_winmain_straight_through(&mut world);
    let stops = world.resource::<WinMainProgress>().stops.clone();
    let counts = world.resource::<ScriptVmCounts>().clone();
    // The first idle frame: 53 quest scripts' runs, their 53 GameMode and 6 MenuMode blocks, and 5 functions.
    recomp_startup::winmain_screens::open_main_menu(&mut world).unwrap();
    recomp_startup::winmain_menu_media::run_main_menu_media_frame(&mut world);
    let idle = world.resource::<ScriptVmCounts>().clone();
    let idle_stops = world.resource::<WinMainProgress>().stops.clone();
    winmain_shutdown(&mut world);
    assert!(stops.is_empty(), "{stops:?}");
    assert!(idle_stops.is_empty(), "{idle_stops:?}");
    assert_eq!(idle.statement_runs - counts.statement_runs, 53);
    assert_eq!(idle.block_tests[0] - counts.block_tests[0], 53);
    assert_eq!(idle.block_tests[1] - counts.block_tests[1], 6);
    assert_eq!(idle.function_calls - counts.function_calls, 5);
    assert_eq!(counts.statement_runs, 1340 + 4684);
    assert_eq!(counts.function_calls, 289 + 1);
    let blocks = counts.block_tests;
    // By the table's row. Rows 4 and 28 share 00503E10, 5 and 29 00503E90.
    let by_handler = [
        ("005022D0", blocks[0], 3629),
        ("00502270", blocks[1], 308),
        ("00503F90", blocks[2], 4051),
        ("00503D10", blocks[3], 269),
        ("00503E10", blocks[4] + blocks[28], 437),
        ("00503E90", blocks[5] + blocks[29], 114),
        ("00503F10", blocks[6], 108),
        ("00504050", blocks[8], 166),
        ("005040D0", blocks[9], 61),
        ("00504150", blocks[10], 696),
        ("00504310", blocks[13], 30),
        ("005043E0", blocks[15], 52),
        ("00504460", blocks[16], 298),
        ("005044E0", blocks[20], 100),
        ("00504560", blocks[21], 595),
        ("005045E0", blocks[22], 24),
        ("00504660", blocks[24], 420),
        ("005041D0", blocks[25], 77),
        ("005046E0", blocks[26], 47),
        ("00504760", blocks[27], 13),
        ("005045A0", blocks[30], 1735),
    ];
    let wrong: Vec<_> = by_handler
        .iter()
        .filter(|(_, ours, theirs)| ours != theirs)
        .collect();
    assert!(wrong.is_empty(), "{wrong:?}");
    assert_eq!(blocks.iter().sum::<u32>(), 13_230);
}

// run-h-phase17, seed 6AEE4398, snapshot-phase17_end: what the original holds at 0040EFF3 on the default path. Under Wine, with no audio and no saves.
#[test]
fn the_port_matches_the_phase_17_rows() {
    let mut world = World::new();
    install_winmain(
        &mut world,
        config(&real_data(), Some(0x6AEE_4398), Some(Duration::ZERO)),
        WinMainSeams::default(),
    );
    let mut schedule = winmain_schedule();
    // Phase 18 changes the HUD's icon trait: these rows are 0040EFF3's. A budget of zero stops the chain at each block.
    while world.resource::<WinMainProgress>().next != Block::StartingSpell {
        schedule.run(&mut world);
    }
    assert!(world.resource::<WinMainProgress>().stops.is_empty());
    let port = phase_17_rows(&mut world);
    winmain_shutdown(&mut world);
    let expected = fixtures::rows("phase17");
    fixtures::ledger_read("phase17", &vec![true; expected.len()]);
    let mut differences = Vec::new();
    for (index, (ours, theirs)) in port.iter().zip(&expected).enumerate() {
        if ours != theirs {
            differences.push(format!("row {index}\n  port {ours}\n  exe  {theirs}"));
        }
    }
    if port.len() != expected.len() {
        differences.push(format!("rows: port {}, exe {}", port.len(), expected.len()));
    }
    assert!(
        differences.is_empty(),
        "{} rows differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}
