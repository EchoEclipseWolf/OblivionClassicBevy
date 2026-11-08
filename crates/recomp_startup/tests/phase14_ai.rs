//! Phase 14c against P14-RUN-004: `00406360` runs whole with the real load, as `game_data_window` does, then `00675310` runs over the 1,998 actors of the level-3 list, and every actor is compared with `fixtures/rows/phase14.rows` before and after.
//!
//! Draws: the pass draws `rand` through 004F4EF0 GetRandomPercent, and P14-RUN-004 recorded each value it got. The `RAND` rows go through `AiRecordedArms` in call order, so the replay needs the host to ask as often as the run did, in the run's order. 14a and 14b don't run here, since the pass reads none of their output, but TLS +0x185 is set as they leave it. `phase14_world_setup` runs the pass inside its window, drawing from the stream, against the change map rows, and holds the file's accounting.

#[path = "game_data_window.rs"]
mod game_data_window;

use std::collections::HashMap;
use std::time::Instant;

use game_data_window::common::act_rows::{
    RUN_004_ROLL_START, Tally, compare_actor, field, hex, id_of, le_hex,
};
use game_data_window::common::rows::RowBook;
use recomp_game::tes::actor_process::ActorProcessGlobals;
use recomp_game::tes::ai_process::{AiProcessGlobals, AiRecordedArms, run_ai_package_pass};
use recomp_game::tes::default_forms::TimeGlobals;
use recomp_game::tes::global::TESGlobal;
use recomp_game::tes::tes::GameThreadLocals;

#[test]
fn the_ai_package_pass_matches_p14_run_004() {
    let book = RowBook::load("phase14");
    // RUN-004's rolls, so that its 0x1A extras are compared whole. This test replays the RAND rows; `phase14_world_setup` draws them from the stream and runs the blocks around the pass.
    let mut world = game_data_window::run_00406360_seeded(Some(RUN_004_ROLL_START)).world;
    // WinMain 0040EB8B, ahead of the pass.
    world.resource_mut::<GameThreadLocals>().track_changes_185 = 1;
    // The time globals (RUN-004 `TG before`): the value bits of the six slots.
    let row = book.take_one_kind("TIMEGLOBALS");
    let globals = world.resource::<TimeGlobals>().globals_b332e0;
    for (index, slot) in globals.iter().enumerate() {
        let global = world
            .get::<TESGlobal>(slot.expect("a time global"))
            .unwrap();
        assert_eq!(
            field(&row, &format!("{:X}", index * 4)),
            le_hex(global.value_bits_24),
            "time global {index}"
        );
    }
    let actors = world
        .resource::<ActorProcessGlobals>()
        .manager
        .low_18
        .list
        .items();
    recomp_core::fixtures::ledger_relaxed(
        "phase14",
        "ACT #ext",
        1,
        "an extra's raw words are heap pointers and unset bytes: its type, in order, and the words that name a form are held",
    );
    let rows = book.take_kind("ACT");
    let before: Vec<&Vec<&str>> = rows.iter().filter(|row| row[1] == "before").collect();
    let after: HashMap<u32, &Vec<&str>> = rows
        .iter()
        .filter(|row| row[1] != "before")
        .map(|row| (hex(field(row, "id")), row))
        .collect();
    assert_eq!(actors.len(), before.len(), "the level-3 list");
    assert_eq!(after.len(), before.len());
    let mut tally = Tally;
    let differences: Vec<String> = actors
        .iter()
        .zip(&before)
        .filter_map(|(&actor, row)| {
            compare_actor(&world, row, actor, &mut tally)
                .map(|text| format!("{:X}: {text}", id_of(&world, Some(actor))))
        })
        .collect();
    assert!(
        differences.is_empty(),
        "{} of {} before rows differ: {:#?}",
        differences.len(),
        actors.len(),
        &differences[..differences.len().min(12)]
    );
    println!("before: {} actors match", actors.len());

    let draws: Vec<i32> = book
        .take_kind("RAND")
        .iter()
        .flat_map(|row| {
            row[1..]
                .iter()
                .map(|value| hex(value) as i32)
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(draws.len(), 157, "P14-RUN-004's draws");
    let mut arms = AiRecordedArms {
        draws,
        ..AiRecordedArms::default()
    };
    let start = Instant::now();
    run_ai_package_pass(&mut world, &mut arms).unwrap();
    println!("00675310: {:.2?}", start.elapsed());

    let mut tally = Tally;
    let mut moved = 0;
    let mut packaged = 0;
    let differences: Vec<String> = actors
        .iter()
        .filter_map(|&actor| {
            let id = id_of(&world, Some(actor));
            let row = after
                .get(&id)
                .unwrap_or_else(|| panic!("no row for {id:X}"));
            if hex(field(row, "pkg")) != 0 {
                packaged += 1;
            }
            let start = before
                .iter()
                .find(|row| hex(field(row, "id")) == id)
                .unwrap();
            if field(row, "pos") != field(start, "pos") {
                moved += 1;
            }
            compare_actor(&world, row, actor, &mut tally).map(|text| format!("{id:X}: {text}"))
        })
        .collect();
    let globals = world.resource::<AiProcessGlobals>();
    println!(
        "after: {} of {} rows differ; run: {packaged} with a package, {moved} moved. host: visited {}, removed {}, evaluated {}, cell moves {}, doors {} unlocked {} locked, scripts at the boundary {}, draws {} of {}, menu queries {}, warnings {}; set aside: ExtraAction of {} actors",
        differences.len(),
        actors.len(),
        globals.actors_visited,
        globals.actors_removed,
        globals.packages_evaluated,
        globals.cell_moves,
        globals.doors_unlocked,
        globals.doors_locked,
        globals.script_event_runs_not_executed,
        arms.next,
        arms.draws.len(),
        arms.menu_queries,
        globals.warnings.len(),
        0,
    );
    let handlers: Vec<String> = globals
        .condition_calls
        .iter()
        .enumerate()
        .filter(|(_, count)| **count != 0)
        .map(|(index, count)| format!("{index:03X}:{count}"))
        .collect();
    println!("condition calls: {}", handlers.join(" "));
    assert!(
        differences.is_empty(),
        "{} of {} after rows differ: {:#?}",
        differences.len(),
        actors.len(),
        &differences[..differences.len().min(12)]
    );
    assert_eq!(arms.next, arms.draws.len(), "GetRandomPercent call count");
    assert!(globals.warnings.is_empty(), "{:?}", globals.warnings);
}

/// P14-RUN-005 recorded the subject of every draw. Its 158 values replayed here must ask in its order, from the same actors: a run-backed check of the walk's order that the ACT rows only see through results.
#[test]
fn the_draws_ask_in_the_order_of_p14_run_005() {
    let records = game_data_window::evidence_lines("phase14-run005-2026-10-09");
    let mut draws = Vec::new();
    let mut subjects = Vec::new();
    for line in &records {
        let parts: Vec<&str> = line.split(' ').collect();
        if parts[0] == "Q" && parts[3] == "RAND" {
            draws.push(hex(field(&parts, "value")) as i32);
            subjects.push(hex(field(&parts, "subject")));
        }
    }
    assert_eq!(draws.len(), 158);
    let mut world = game_data_window::run_00406360();
    let mut arms = AiRecordedArms {
        draws,
        ..AiRecordedArms::default()
    };
    run_ai_package_pass(&mut world, &mut arms).unwrap();
    let host = &world.resource::<AiProcessGlobals>().random_subjects;
    let same = host
        .iter()
        .zip(&subjects)
        .take_while(|(host, run)| host == run)
        .count();
    println!(
        "draw subjects: {same} in order; host {} of run {}",
        host.len(),
        subjects.len()
    );
    assert_eq!(host, &subjects, "first difference at draw {same}");
}
