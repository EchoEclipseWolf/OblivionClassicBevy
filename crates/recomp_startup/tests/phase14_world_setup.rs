//! WinMain 0040EB31..0040EBB8 on the World the whole of 00406360 leaves, with all of `game_data_window`'s checks on the way, against P14's rows. `fixtures/rows/phase14.rows` is read here, every line, with the accounting of `phase12_load.rs`. `CHG` comes from P14-RUN-001, `ACT` and `RAND` from P14-RUN-004, so the change map is compared on the terms two runs share: a key the rolls or a GetRandomPercent decide can differ. Host tests earn no validation bonus.

#[path = "game_data_window.rs"]
mod game_data_window;

use std::collections::BTreeMap;
use std::time::Instant;

use bevy_ecs::prelude::*;
use game_data_window::common::act_rows::{RUN_004_ROLL_START, Tally, compare_actor};
use game_data_window::common::image::Image;
use game_data_window::common::images_magic_actors::npc_image;
use game_data_window::common::load::TestHost;
use game_data_window::common::rows::{Deferred, RowBook, hex_bytes};
use game_data_window::{GameWindow, evidence_lines, run_00406360_seeded};
use recomp_core::crt::MsvcRand;
use recomp_core::fixtures;
use recomp_game::bs::tree_manager::TreeManagerGlobals;
use recomp_game::tes::actor::TesObjectRefr;
use recomp_game::tes::actor_process::RandomSource;
use recomp_game::tes::ai_process::{AiProcessGlobals, AiRealArms};
use recomp_game::tes::data_handler::{
    DataHandler, DataHandlerGlobals, DataHandlerLists, FormsById, construct_data_handler,
};
use recomp_game::tes::data_load::DataLoadServices;
use recomp_game::tes::default_forms::TimeGlobals;
use recomp_game::tes::form::{FormIdentity, OriginalVtable};
use recomp_game::tes::form_components::{FormRef, TesActorBaseData, TesContainer};
use recomp_game::tes::forms_ai::IdleAnimRoots;
use recomp_game::tes::forms_items::{clear_change_flags, set_change_flags};
use recomp_game::tes::forms_script::mark_temporary;
use recomp_game::tes::forms_world::{
    PointerMap, TerrainLodEntry, TerrainLodQuad, TerrainLodServices, TesWorldSpace,
    WorldFormGlobals, terrain_lod_activate, terrain_lod_quad_detach, terrain_lod_refresh,
    world_lod_root,
};
use recomp_game::tes::game_settings::GameSettingCollection;
use recomp_game::tes::global::TESGlobal;
use recomp_game::tes::inventory::InventoryGlobals;
use recomp_game::tes::os_globals::OsGlobalsData;
use recomp_game::tes::settings::{SettingCollection, SettingCollections, register_static_settings};
use recomp_game::tes::tes::{
    GameThreadLocals, Tes, TesGlobals, TesSaveLoadGame, load_form_change, set_current_worldspace,
};
use recomp_startup::winmain_world_setup::{
    DEFAULT_PLAYER_START_BITS, WorldSetupSeams, WorldspaceInputs, run_actor_locations_prelude,
    run_tracked_actors, run_world_setup, run_world_setup_finish, run_world_setup_worldspace,
};

/// Nothing is left out: every line of the file is read below.
const DEFERRED: &[Deferred] = &[];

const RUN_001: &str = "phase14-run001-2026-10-09";

fn field<'a>(line: &'a str, key: &str) -> &'a str {
    let at = line
        .find(&format!(" {key}="))
        .unwrap_or_else(|| panic!("missing key {key} in {line}"));
    line[at + key.len() + 2..].split(' ').next().unwrap()
}

fn row_field<'a>(parts: &[&'a str], key: &str) -> &'a str {
    let prefix = format!("{key}=");
    parts
        .iter()
        .find_map(|part| part.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("missing key {key} in {parts:?}"))
}

// ---- The checks ----

fn tes(world: &World) -> &Tes {
    world.resource::<TesGlobals>().tes.as_ref().unwrap()
}

fn check_tes(world: &World, book: &RowBook, tag: &str) {
    let row = book.take_one("TES", tag);
    let tes = tes(world);
    let coords: Vec<String> = tes
        .exterior_coordinates_20
        .iter()
        .map(|value| format!("{value:08X}"))
        .collect();
    assert_eq!(coords.join(","), row_field(&row, "coords"), "{tag}: +0x20");
    let last: Vec<String> = tes
        .last_exterior_coordinates_48
        .iter()
        .map(|value| format!("{value:08X}"))
        .collect();
    assert_eq!(last.join(","), row_field(&row, "last"), "{tag}: +0x48");
    let current = tes.current_world_space_74.map_or(0, |form| {
        world.get::<FormIdentity>(form).unwrap().form_id_0c
    });
    assert_eq!(
        format!("{current:X}"),
        row_field(&row, "ws"),
        "{tag}: +0x74"
    );
    assert_eq!(
        format!("{:X}", world.resource::<OsGlobalsData>().unk_18),
        row_field(&row, "osg18"),
        "{tag}: g_osGlobals +0x18"
    );
    let far = world
        .get_resource::<WorldFormGlobals>()
        .map_or(0, |globals| globals.display_lod_land_b3608f);
    assert_eq!(
        format!("{}/{far}", display_lod_land(world, book)),
        row_field(&row, "lodland"),
        "{tag}: 00B43077/00B3608F"
    );
}

// IDA: 0040E92D's value: the byte of 00B02D70 bDisplayLODLand. `winmain_shader` isn't joined to this window, so the host read goes through this resource.
#[derive(Resource)]
struct DisplayLodLand(u8);

fn display_lod_land(world: &World, _book: &RowBook) -> u8 {
    world.resource::<DisplayLodLand>().0
}

fn worlds(world: &World) -> Vec<Entity> {
    world
        .resource::<DataHandlerLists>()
        .world_spaces_0c
        .items()
        .collect()
}

fn check_worlds(world: &World, book: &RowBook, tag: &str) {
    let rows = book.take("WS", tag);
    let forms = worlds(world);
    assert_eq!(forms.len(), rows.len(), "{tag}: worlds");
    for (index, (row, form)) in rows.iter().zip(&forms).enumerate() {
        assert_eq!(row[2], index.to_string());
        let what = format!("{tag} world {index}");
        let data = world.get::<TesWorldSpace>(*form).unwrap();
        assert_eq!(
            format!("{:X}", world.get::<FormIdentity>(*form).unwrap().form_id_0c),
            row_field(row, "id"),
            "{what}: id"
        );
        let parent = match data.parent_7c {
            FormRef::Resolved(Some(parent)) => {
                world.get::<FormIdentity>(parent).unwrap().form_id_0c
            }
            FormRef::Resolved(None) => 0,
            FormRef::Raw(raw) => raw,
        };
        assert_eq!(
            format!("{parent:X}"),
            row_field(row, "parent"),
            "{what}: parent"
        );
        assert_eq!(
            format!("{:08X}", data.lod_38.original_vtable),
            row_field(row, "vt"),
            "{what}: vtable"
        );
        assert_eq!(
            format!("{:X}", data.lod_38.buckets_04),
            row_field(row, "hash"),
            "{what}: buckets"
        );
        assert_eq!(
            format!("{:X}", data.lod_38.count_0c),
            row_field(row, "count"),
            "{what}: +0x0C"
        );
        let items: usize = data.lod_38.buckets_08.iter().map(Vec::len).sum();
        assert_eq!(
            items.to_string(),
            row_field(row, "items"),
            "{what}: the walk"
        );
    }
}

fn check_lod_entries(world: &World, book: &RowBook, tamriel: Entity) {
    let rows = book.take("WE", "3C");
    let map = &world.get::<TesWorldSpace>(tamriel).unwrap().lod_38;
    let walked: Vec<(usize, u32, &TerrainLodEntry)> = map
        .buckets_08
        .iter()
        .enumerate()
        .flat_map(|(bucket, chain)| chain.iter().map(move |(key, entry)| (bucket, *key, entry)))
        .collect();
    assert_eq!(walked.len(), rows.len(), "the LOD entries");
    for (row, (bucket, key, entry)) in rows.iter().zip(&walked) {
        let what = format!("LOD entry {key:X}");
        assert_eq!(
            format!("{bucket:X}"),
            row_field(row, "bucket"),
            "{what}: bucket"
        );
        assert_eq!(format!("{key:X}"), row_field(row, "key"), "{what}: key");
        assert_eq!(entry.x_08.to_string(), row_field(row, "x"), "{what}: x");
        assert_eq!(entry.y_0a.to_string(), row_field(row, "y"), "{what}: y");
        let quad = &entry.quad_00;
        assert_eq!(
            format!("{:X}", quad.state_08),
            row_field(row, "state"),
            "{what}"
        );
        assert_eq!(
            format!("{:X}", quad.field_14),
            row_field(row, "f14"),
            "{what}"
        );
        assert_eq!(
            format!("{:08X},{:08X}", quad.x_18.to_bits(), quad.y_1c.to_bits()),
            row_field(row, "pos"),
            "{what}: +0x18, +0x1C"
        );
        assert_eq!(
            format!("{:08X}", quad.extent_44.to_bits()),
            row_field(row, "ext"),
            "{what}: +0x44"
        );
        assert_eq!(
            u32::from(quad.children_30.is_some()).to_string(),
            row_field(row, "children"),
            "{what}: +0x30"
        );
    }
}

fn check_idle_map(world: &World, book: &RowBook, tag: &str) {
    let row = book.take_one("IDLEMAP", tag);
    let map = world
        .resource::<DataHandlerGlobals>()
        .idle_anim_roots_b362c0
        .as_ref()
        .unwrap();
    assert_eq!(
        format!("{:X}", map.count_0c),
        row_field(&row, "count"),
        "{tag}: +0x0C"
    );
    assert_eq!(
        world.resource::<IdleAnimRoots>().walk().count().to_string(),
        row_field(&row, "items"),
        "{tag}: the walk"
    );
    let path = world
        .resource::<GameSettingCollection>()
        .text_at(0x00B3_6BB8)
        .unwrap();
    assert_eq!(path, row_field(&row, "path").as_bytes(), "{tag}: 00B36BB8");
}

fn check_idle_root(world: &World, book: &RowBook, index: usize) {
    let key = book.take_kind("IDLEKEY");
    let root = &world.resource::<IdleAnimRoots>().roots[index];
    assert_eq!(key.len(), 1);
    assert_eq!(
        root.path_18.data_00.as_deref().unwrap(),
        key[0][1].as_bytes(),
        "005215C0's key"
    );
    let row = book.take_kind("IDLEROOT");
    let row = &row[0];
    assert_eq!(
        root.path_18.data_00.as_deref().unwrap(),
        row_field(row, "key").as_bytes()
    );
    assert_eq!(
        format!("{:08X}", root.idles.original_vtable),
        row_field(row, "vt")
    );
    assert_eq!(
        format!("{:X}", root.idles.capacity_08),
        row_field(row, "cap")
    );
    assert_eq!(format!("{:X}", root.idles.size_0c), row_field(row, "size"));
    assert_eq!(
        format!("{:X}", root.idles.count_10),
        row_field(row, "count")
    );
    assert_eq!(format!("{:X}", root.idles.grow_14), row_field(row, "grow"));
    assert_eq!(
        format!("{:X}", root.path_18.length_04),
        row_field(row, "len")
    );
    let files = root.kf_files_20.as_ref().unwrap();
    assert_eq!(files.len().to_string(), row_field(row, "kf"), "the KF list");
    // RUN-001 read 311 of the 322: 11 string reads crossed a page end, so a row can be skipped but never reordered.
    let rows = book.take_kind("IDLEKF");
    let mut names = files.iter();
    for row in &rows {
        assert!(
            names.any(|name| name.as_str() == row[1]),
            "{} is out of order or missing",
            row[1]
        );
    }
    assert_eq!(rows.len(), 311);
}

fn check_archives(host: &mut TestHost, book: &RowBook, tag: &str) {
    let manager = host.archives.as_ref().unwrap();
    let rows = book.take("ARCHIVE", tag);
    assert_eq!(manager.archives().len(), rows.len(), "{tag}: g_ArchiveList");
    // The run's list order.
    for (index, (row, archive)) in rows.iter().zip(manager.archives()).enumerate() {
        let what = format!("{tag} archive {index}");
        assert_eq!(
            format!("{:X}", archive.archive_flags()),
            row_field(row, "flags"),
            "{what}"
        );
        assert_eq!(
            format!("{:X}", archive.folder_count()),
            row_field(row, "folders"),
            "{what}"
        );
        assert_eq!(
            format!("{:X}", archive.file_count()),
            row_field(row, "files"),
            "{what}"
        );
        assert_eq!(
            format!("{:02X}", archive.runtime_flags()),
            row_field(row, "runtime"),
            "{what}: +0x194"
        );
        let names: Vec<&str> = archive
            .name_blocks()
            .iter()
            .map(|&present| if present { "1" } else { "0" })
            .collect();
        assert_eq!(
            names.join(","),
            row_field(row, "names"),
            "{what}: the name blocks"
        );
    }
}

fn check_archive_settings(host: &TestHost, book: &RowBook, tag: &str) {
    let row = book.take_one("ARCHIVES", tag);
    let settings = host.archives.as_ref().unwrap().settings();
    assert_eq!(
        format!(
            "{},{},{}",
            settings.retain_directory_strings,
            settings.retain_filename_strings,
            settings.retain_filename_offsets
        ),
        row_field(&row, "retain"),
        "{tag}: 00B04438, 00B04440, 00B04448"
    );
}

fn check_tls(world: &World, book: &RowBook, tag: &str) {
    let row = book.take_one("TLS", tag);
    let locals = world.resource::<GameThreadLocals>();
    assert_eq!(
        locals.byte_184.to_string(),
        row_field(&row, "b184"),
        "{tag}"
    );
    assert_eq!(
        locals.track_changes_185.to_string(),
        row_field(&row, "b185"),
        "{tag}"
    );
    assert_eq!(
        locals.byte_186.to_string(),
        row_field(&row, "b186"),
        "{tag}"
    );
}

fn game(world: &World) -> &TesSaveLoadGame {
    world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .unwrap()
}

/// IDA: N/A one `CHG` stage: the entries in the walk's order.
fn run_changes(book: &RowBook, tag: &str) -> Vec<(u32, [u32; 2])> {
    book.take("CHG", tag)
        .iter()
        .flat_map(|row| row[2..].iter().copied())
        .map(|item| {
            let mut parts = item.split(':');
            let mut next = || u32::from_str_radix(parts.next().unwrap(), 16).unwrap();
            let id = next();
            (id, [next(), next()])
        })
        .collect()
}

// IDA: N/A a reference whose base container holds a leveled list (form type 0x2B): the 12d rolls decide whether it has container changes, so its 0x8000000 varies between runs.
fn rolled(world: &World, id: u32, flags: u32) -> bool {
    if flags & 0x800_0000 == 0 {
        return false;
    }
    let Some(form) = world.resource::<FormsById>().lookup(id) else {
        return false;
    };
    let Some(base) = world
        .get::<TesObjectRefr>(form)
        .and_then(|refr| refr.base_form_1c)
    else {
        return false;
    };
    world.get::<TesContainer>(base).is_some_and(|container| {
        container.entries_08.iter().any(|entry| {
            matches!(entry.item, FormRef::Resolved(Some(item))
                if world.get::<FormIdentity>(item).unwrap().form_type_04 == 0x2B)
        })
    })
}

/// IDA: N/A a map's bucket for two chains' relative order.
const fn bucket(id: u32) -> u32 {
    id % 0x13AF
}

// IDA: N/A the host's +0x00 map against one run's stage. A roll's key may be absent on either side, or differ by 0x8000000. A GetRandomPercent subject's key may differ by 4 alone, 004D8A30's scheduled placement, since `CHG` is RUN-001 and the draws are RUN-004's. Every other key is exact, and in each bucket the keys both sides hold keep their order, which is the order of the 00452C20 calls. Answers how many keys the rolls moved.
fn check_change_map(world: &World, run: &[(u32, [u32; 2])], tag: &str, drawn: &[u32]) -> usize {
    let host: Vec<(u32, [u32; 2])> = game(world)
        .changes_00
        .walk()
        .map(|(id, record)| (id, [record.flags_00, record.block_04]))
        .collect();
    let want: BTreeMap<u32, [u32; 2]> = run.iter().copied().collect();
    let have: BTreeMap<u32, [u32; 2]> = host.iter().copied().collect();
    let (mut failures, mut moved, mut moved_by_draws) = (Vec::new(), 0, 0);
    // A key only one side's rolls made before this stage links at a different time, so the order check leaves it out.
    let mut moved_keys = std::collections::BTreeSet::new();
    for (id, value) in &want {
        match have.get(id) {
            None if rolled(world, *id, value[0]) => moved += 1,
            None => failures.push(format!("missing {id:X}:{:X}", value[0])),
            Some(found) if found == value => {}
            Some(found)
                if found[0] ^ value[0] == 0x800_0000 && rolled(world, *id, found[0] | value[0]) =>
            {
                moved += 1;
                moved_keys.insert(*id);
            }
            Some(found) if found[0] ^ value[0] == 4 && drawn.contains(id) => {
                moved += 1;
                moved_by_draws += 1;
                moved_keys.insert(*id);
            }
            Some(found) => failures.push(format!("{id:X} host {:X} run {:X}", found[0], value[0])),
        }
    }
    for (id, value) in &have {
        if !want.contains_key(id) {
            if rolled(world, *id, value[0]) {
                moved += 1;
            } else {
                failures.push(format!("extra {id:X}:{:X}", value[0]));
            }
        }
    }
    // The chains. A bucket's shared keys, as the two walks meet them.
    let shared = |list: &[(u32, [u32; 2])], other: &BTreeMap<u32, [u32; 2]>| -> Vec<u32> {
        list.iter()
            .map(|(id, _)| *id)
            .filter(|id| other.contains_key(id) && !moved_keys.contains(id))
            .collect()
    };
    fixtures::ledger_relaxed(
        "phase14",
        &format!("CHG {tag} #roll-moved"),
        moved,
        "a roll-moved key: absent, or off by 0x8000000 on a leveled container, or by 4 on a GetRandomPercent subject; CHG is RUN-001 and its rand stream is unknown (docs/open-items.md, Phase 14 joined)",
    );
    let (run_order, host_order) = (shared(run, &have), shared(&host, &want));
    let walks_agree = run_order.len() == host_order.len()
        && run_order
            .iter()
            .zip(&host_order)
            .all(|(a, b)| a == b && bucket(*a) == bucket(*b));
    if !walks_agree {
        let first = run_order
            .iter()
            .zip(&host_order)
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        failures.push(format!(
            "the walk's order, first at {first}: run {:X} host {:X}",
            run_order.get(first).copied().unwrap_or(0),
            host_order.get(first).copied().unwrap_or(0)
        ));
    }
    println!(
        "{tag}: {} host, {} in the run, {moved} keys moved ({moved_by_draws} of them by RUN-001's own draws), {} failures",
        host.len(),
        run.len(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{tag}: {} failures: {:?}",
        failures.len(),
        &failures[..failures.len().min(12)]
    );
    assert_eq!(
        game(world).changes_map_00.count_0c as usize,
        host.len(),
        "{tag}: +0x0C"
    );
    moved
}

fn check_change_head(world: &World, book: &RowBook, tag: &str, rolled_count: usize) {
    let row = book.take_one("CHANGEMAP", tag);
    let map = &game(world).changes_map_00;
    assert_eq!(
        format!("{:08X}", map.original_vtable),
        row_field(&row, "vt"),
        "{tag}"
    );
    assert_eq!(
        format!("{:X}", map.hash_size_04),
        row_field(&row, "hash"),
        "{tag}"
    );
    // The count differs by the rolls alone.
    let items: usize = row_field(&row, "items").parse().unwrap();
    assert_eq!(
        usize::from_str_radix(row_field(&row, "count"), 16).unwrap(),
        items,
        "{tag}: the run's count and walk"
    );
    let host = game(world).changes_00.len();
    assert!(
        host.abs_diff(items) <= rolled_count,
        "{tag}: host {host}, run {items}, {rolled_count} rolled"
    );
}

fn read_time_globals(world: &World, book: &RowBook) {
    let row = book.take_one_kind("TIMEGLOBALS");
    let globals = world.resource::<TimeGlobals>();
    for (index, slot) in globals.globals_b332e0.iter().enumerate() {
        let global = world.get::<TESGlobal>(slot.unwrap()).unwrap();
        let bytes: String = global
            .value_bits_24
            .to_le_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(
            bytes,
            row_field(&row, &format!("{:X}", index * 4)),
            "time global {index}"
        );
    }
    // The rest of the row reads past the 0x18 struct, into nulls.
    for key in ["18", "1C", "20"] {
        assert_eq!(row_field(&row, key), "null");
    }
}

// RAND: RUN-004's GetRandomPercent values, 14c's to consume. What this slice settles: they're consecutive draws of one MSVC stream, from a recovered state, so a draw's replay is exact.
fn read_rand(book: &RowBook) {
    let values: Vec<i32> = book
        .take_kind("RAND")
        .iter()
        .flat_map(|row| row[1..].iter().copied())
        .map(|value| i32::from_str_radix(value, 16).unwrap())
        .collect();
    assert_eq!(values.len(), 157);
    // The holdrand after the first draw, recovered from all 157 (17 unknown bits, one answer).
    let mut stream = MsvcRand::default();
    stream.srand(0x1B8C_A0AD);
    assert_eq!(values[0], 0x1B8C, "the state's own bits");
    for (index, value) in values[1..].iter().enumerate() {
        assert_eq!(stream.rand(), *value, "draw {}", index + 1);
    }
}

fn check_autocalc(world: &World, book: &RowBook, visited: &[Entity]) {
    let rows = book.take_kind("AUTOCALC");
    let want: Vec<(u32, u32)> = rows
        .iter()
        .flat_map(|row| row[1..].iter().copied())
        .map(|item| {
            let (id, flags) = item.split_once(':').unwrap();
            (
                u32::from_str_radix(id, 16).unwrap(),
                u32::from_str_radix(flags, 16).unwrap(),
            )
        })
        .collect();
    let have: Vec<(u32, u32)> = visited
        .iter()
        .map(|&form| {
            (
                world.get::<FormIdentity>(form).unwrap().form_id_0c,
                world.get::<TesActorBaseData>(form).unwrap().flags_04,
            )
        })
        .collect();
    assert_eq!(have.len(), want.len(), "005222D0's calls");
    assert_eq!(have, want, "the NPCs in the walk's order and their +0x28");
}

// NS, `fixtures/rows/phase14_run001.rows`: P14-RUN-001's 12 NPCs dumped at 0040EBAF, after 005222D0, each line verbatim from the run's records. Every byte of each against the host's, with pointers by null-ness.
fn check_autocalc_samples(world: &World, records: &[String]) {
    let mut compared = 0;
    for line in fixtures::rows("phase14_run001") {
        assert!(records.contains(&line), "not in the run: {line}");
        let line = &line;
        let id = u32::from_str_radix(field(line, "id"), 16).unwrap();
        let raw = hex_bytes(field(line, "raw"));
        let form = world.resource::<FormsById>().lookup(id).unwrap();
        let mut image: Image = npc_image(world, form);
        // The dump holds +0x00..+0x120: through the skills and class, short of FaceGen.
        assert_eq!(raw.len(), 0x120);
        image.truncate(raw.len());
        image.compare(&raw, &format!("NS {id:X}"), &|_| None);
        compared += 1;
    }
    assert_eq!(compared, 12);
}

// IR: every root of 00B362C0 in RUN-001's walk, at one dump point: its bucket and key, the NiFormArray, the path string and the +0x20 list in order. RUN-001 read some names as '' (a read across a page end), which skip but keep the rest in order.
fn check_idle_roots(world: &World, records: &[String], tag: &str) {
    let lines: Vec<&String> = records
        .iter()
        .filter(|line| line.starts_with("IR ") && line.split(' ').nth(3) == Some(tag))
        .collect();
    let roots = world.resource::<IdleAnimRoots>();
    let mut walked = Vec::new();
    for (bucket, chain) in roots.buckets_08.iter().enumerate() {
        for &index in chain {
            walked.push((bucket, &roots.roots[index]));
        }
    }
    assert_eq!(walked.len(), lines.len(), "{tag}: the walk");
    for (line, (bucket, root)) in lines.iter().zip(&walked) {
        let key = field(line, "key");
        let what = format!("{tag} root {key}");
        assert_eq!(
            format!("{bucket:X}"),
            field(line, "bucket"),
            "{what}: bucket"
        );
        // The run's probe wrote a space in a name as '_'.
        let path = String::from_utf8_lossy(root.path_18.data_00.as_deref().unwrap_or_default())
            .replace(' ', "_");
        assert_eq!(path, key, "{what}: +0x18");
        assert_eq!(path, field(line, "path"), "{what}: +0x18");
        let raw = hex_bytes(field(line, "raw"));
        let word = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
        let half = |at: usize| u16::from_le_bytes(raw[at..at + 2].try_into().unwrap());
        assert_eq!(word(0x00), root.idles.original_vtable, "{what}: vtable");
        assert_eq!(
            word(0x04) != 0,
            !root.idles.data_04.is_empty(),
            "{what}: +0x04"
        );
        assert_eq!(word(0x08), root.idles.capacity_08, "{what}: +0x08");
        assert_eq!(word(0x0C), root.idles.size_0c, "{what}: +0x0C");
        assert_eq!(word(0x10), root.idles.count_10, "{what}: +0x10");
        assert_eq!(word(0x14), root.idles.grow_14, "{what}: +0x14");
        assert_eq!(half(0x1C), root.path_18.length_04, "{what}: +0x1C");
        assert_eq!(word(0x20) != 0, root.kf_files_20.is_some(), "{what}: +0x20");
        let rest = line.split(" kf=").nth(1).unwrap();
        let (count, names) = rest.split_once(' ').unwrap_or((rest, ""));
        let files = root.kf_files_20.as_deref().unwrap_or_default();
        assert_eq!(files.len().to_string(), count, "{what}: the list");
        let names: Vec<&str> = names.split(',').collect();
        assert_eq!(names.len(), files.len().max(1), "{what}: the run's list");
        for (name, file) in names.iter().zip(files) {
            let file = file.replace(' ', "_");
            assert!(
                name.is_empty() || *name == file,
                "{what}: run {name}, host {file}"
            );
        }
    }
}

// T: TES's whole 0xAC bytes at one dump point, with pointers by null-ness.
fn check_tes_raw(world: &World, records: &[String], tag: &str) {
    let line = records
        .iter()
        .find(|line| line.starts_with("T ") && line.split(' ').nth(3) == Some(tag))
        .unwrap();
    let raw = hex_bytes(field(line, "raw"));
    assert_eq!(raw.len(), 0xAC, "{tag}");
    let tes = tes(world);
    let word = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
    let what = |at: usize| format!("{tag}: TES +0x{at:X}");
    assert_eq!(word(0), tes.vtable_00, "{}", what(0));
    let pointers: [(usize, bool); 16] = [
        (0x04, tes.grid_distant_04.is_some()),
        (0x08, tes.grid_cells_08.is_some()),
        (0x0C, tes.scene_root_0c.is_some()),
        (0x10, tes.land_lod_10.is_some()),
        (0x14, tes.temp_node_manager_14.is_some()),
        (0x18, tes.sun_light_18.is_some()),
        (0x1C, tes.fog_1c.is_some()),
        (0x30, tes.current_exterior_cell_30.is_some()),
        (0x34, tes.current_interior_cell_34.is_some()),
        (0x38, !tes.interior_cell_buffer_38.is_empty()),
        (0x3C, !tes.exterior_cell_buffer_3c.is_empty()),
        (0x54, tes.water_surface_54.is_some()),
        (0x58, tes.water_node_58.is_some()),
        (0x5C, tes.sky_5c),
        (0x74, tes.current_world_space_74.is_some()),
        (0x78, tes.exterior_cell_list_78.is_some()),
    ];
    for (at, present) in pointers {
        assert_eq!(word(at) != 0, present, "{}", what(at));
    }
    for (index, value) in tes.exterior_coordinates_20.iter().enumerate() {
        assert_eq!(
            word(0x20 + 4 * index),
            *value as u32,
            "{}",
            what(0x20 + 4 * index)
        );
    }
    for (index, value) in tes.last_exterior_coordinates_48.iter().enumerate() {
        assert_eq!(
            word(0x48 + 4 * index),
            *value as u32,
            "{}",
            what(0x48 + 4 * index)
        );
    }
    for (index, decal) in tes.blood_decals_94.iter().enumerate() {
        assert_eq!(
            word(0x94 + 4 * index) != 0,
            decal.is_some(),
            "{}",
            what(0x94 + 4 * index)
        );
    }
    assert_eq!(word(0x40), tes.field_40, "{}", what(0x40));
    assert_eq!(word(0x44), tes.field_44, "{}", what(0x44));
    assert_eq!(raw[0x50], tes.byte_50, "{}", what(0x50));
    assert_eq!(raw[0x51], tes.cell_test_active_51, "{}", what(0x51));
    assert_eq!(raw[0x52], tes.byte_52, "{}", what(0x52));
    assert_eq!(raw[0x53], tes.cell_borders_enabled_53, "{}", what(0x53));
    assert_eq!(word(0x60), tes.field_60, "{}", what(0x60));
    assert_eq!(word(0x64), tes.field_64, "{}", what(0x64));
    assert_eq!(raw[0x68..0x6C], tes.bytes_68, "{}", what(0x68));
    assert_eq!(
        word(0x6C),
        tes.grid_center_margin_x_6c.to_bits(),
        "{}",
        what(0x6C)
    );
    assert_eq!(
        word(0x70),
        tes.grid_center_margin_y_70.to_bits(),
        "{}",
        what(0x70)
    );
    assert_eq!(
        word(0x7C) != 0,
        !tes.exterior_cell_list_next_7c.is_empty(),
        "{}",
        what(0x7C)
    );
    assert_eq!(word(0x80), tes.furniture_references_80[0], "{}", what(0x80));
    assert_eq!(word(0x84), tes.furniture_references_80[1], "{}", what(0x84));
    assert_eq!(
        word(0x88),
        tes.queued_script_references_88,
        "{}",
        what(0x88)
    );
    assert_eq!(
        word(0x8C),
        tes.actor_base_death_counts_8c[0],
        "{}",
        what(0x8C)
    );
    assert_eq!(
        word(0x90),
        tes.actor_base_death_counts_8c[1],
        "{}",
        what(0x90)
    );
    assert_eq!(
        word(0xA0),
        tes.particle_node_cache_first_a0,
        "{}",
        what(0xA0)
    );
    assert_eq!(
        word(0xA4),
        tes.particle_node_cache_next_a4,
        "{}",
        what(0xA4)
    );
    assert_eq!(raw[0xA8], tes.byte_a8, "{}", what(0xA8));
    assert_eq!(raw[0xA9], tes.texture_purge_allowed_a9, "{}", what(0xA9));
    assert_eq!(raw[0xAA..0xAC], tes.bytes_aa, "{}", what(0xAA));
}

// WE's raw and qraw: each entry's 0x10 bytes and its quad's 0x60, in the walk's order.
fn check_lod_raw(world: &World, records: &[String], tamriel: Entity) {
    let map = &world.get::<TesWorldSpace>(tamriel).unwrap().lod_38;
    let entries: Vec<&TerrainLodEntry> = map
        .buckets_08
        .iter()
        .flatten()
        .map(|(_, entry)| entry)
        .collect();
    let lines: Vec<&String> = records
        .iter()
        .filter(|line| line.starts_with("WE "))
        .collect();
    assert_eq!(lines.len(), entries.len());
    for (line, entry) in lines.iter().zip(&entries) {
        let raw = hex_bytes(field(line, "raw"));
        assert_ne!(raw[..4], [0; 4], "+0x00");
        assert_ne!(raw[4..8], [0; 4], "+0x04");
        assert_eq!(raw[8..10], entry.x_08.to_le_bytes(), "+0x08");
        assert_eq!(raw[10..12], entry.y_0a.to_le_bytes(), "+0x0A");
        let quad = &entry.quad_00;
        let raw = hex_bytes(field(line, "qraw"));
        assert_eq!(raw.len(), 0x60);
        let word = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
        assert_ne!(word(0x00), 0, "the entry");
        assert_eq!(word(0x04) != 0, quad.object_04.is_some(), "+0x04");
        assert_eq!(word(0x08), quad.state_08, "+0x08");
        for (index, half) in quad.words_0c.iter().enumerate() {
            assert_eq!(
                raw[0x0C + 2 * index..0x0E + 2 * index],
                half.to_le_bytes(),
                "+0x0C"
            );
        }
        assert_eq!(word(0x14), quad.field_14, "+0x14");
        assert_eq!(word(0x18), quad.x_18.to_bits(), "+0x18");
        assert_eq!(word(0x1C), quad.y_1c.to_bits(), "+0x1C");
        assert_eq!(word(0x20), quad.field_20, "+0x20");
        assert_eq!(word(0x24), quad.field_24, "+0x24");
        assert_eq!(word(0x28), quad.unset_28, "+0x28");
        assert_eq!(word(0x2C) != 0, quad.node_2c.is_some(), "+0x2C");
        for index in 0..4 {
            assert_eq!(
                word(0x30 + 4 * index) != 0,
                quad.children_30.is_some(),
                "+0x30"
            );
        }
        assert_eq!(word(0x40), quad.field_40, "+0x40");
        assert_eq!(word(0x44), quad.extent_44.to_bits(), "+0x44");
        assert_eq!(word(0x48), quad.field_48.to_bits(), "+0x48");
        assert_eq!(word(0x4C), quad.field_4c.to_bits(), "+0x4C");
        for (index, value) in quad.fields_50.iter().enumerate() {
            assert_eq!(word(0x50 + 4 * index), *value, "+0x50");
        }
    }
}

// AR's raw at +0x150: the BSA header's words as the host read them, and the +0x174 file flags.
fn check_archives_raw(host: &TestHost, records: &[String], tag: &str) {
    let lines: Vec<&String> = records
        .iter()
        .filter(|line| line.starts_with("AR ") && line.split(' ').nth(3) == Some(tag))
        .collect();
    let archives = host.archives.as_ref().unwrap().archives();
    assert_eq!(lines.len(), archives.len());
    for (line, archive) in lines.iter().zip(archives) {
        let raw = hex_bytes(field(line, "raw"));
        let word = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
        assert_eq!(word(0x08), archive.version(), "+0x158");
        assert_eq!(word(0x10), archive.archive_flags(), "+0x160");
        assert_eq!(word(0x24), archive.file_flags(), "+0x174");
    }
}

// SL: TESSaveLoadGame's first 0x40 bytes at one dump point.
fn check_save_load_raw(world: &World, records: &[String], tag: &str) {
    let line = records
        .iter()
        .find(|line| line.starts_with("SL ") && line.split(' ').nth(3) == Some(tag))
        .unwrap();
    let raw = hex_bytes(field(line, "raw"));
    let word = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap());
    let save = game(world);
    assert_ne!(word(0x00), 0, "{tag}: +0x00");
    assert_eq!(word(0x04), save.field_04, "{tag}: +0x04");
    assert_ne!(word(0x08), 0, "{tag}: +0x08");
    assert_ne!(word(0x0C), 0, "{tag}: +0x0C");
    assert_ne!(word(0x10), 0, "{tag}: +0x10");
    for (index, value) in save
        .fields_14
        .iter()
        .take((raw.len() - 0x14) / 4)
        .enumerate()
    {
        assert_eq!(
            word(0x14 + 4 * index),
            *value,
            "{tag}: +0x{:X}",
            0x14 + 4 * index
        );
    }
}

// IDA: N/A D8, the 00B34428 reference after 00485D50 made it: 0046B590's round trip (P13-RUN-002, P13-RUN-003), and 00B34428 holds no change record, since 0045B700 and 0045B670 both skip a temporary form.
fn check_temporary_reference(world: &World) {
    let temporary = world
        .resource::<InventoryGlobals>()
        .temporary_reference_b34428
        .unwrap();
    let identity = world.get::<FormIdentity>(temporary).unwrap();
    assert_eq!(identity.form_id_0c, 0xFF00_0800, "00B34428's ID");
    assert_eq!(identity.flags_08, 0x4008, "00B34428's flags");
    assert_eq!(world.resource::<FormsById>().lookup(0xFF00_0800), None);
    assert_eq!(
        world.resource::<DataHandler>().next_form_id_8c0(),
        0xFF00_0800
    );
    assert!(game(world).changes_00.get(0xFF00_0800).is_none());
}

/// IDA: N/A 0047DF80's CRT drawn from `TestHost::rand`, the stream 12d left. 00B069C3 is clear by then, so a reseed would be a finding.
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

/// IDA: N/A ACT rows of one stage against the actors, as a list of differences.
fn act_differences(world: &World, rows: &[&Vec<&str>], tally: &mut Tally) -> Vec<String> {
    rows.iter()
        .filter_map(|row| {
            let id = u32::from_str_radix(row_field(row, "id"), 16).unwrap();
            let actor = world.resource::<FormsById>().lookup(id).unwrap();
            compare_actor(world, row, actor, tally).map(|text| format!("{id:X}: {text}"))
        })
        .collect()
}

// The holdrand after RAND's last draw, from the state after its first.
fn stream_after_draws(draws: usize) -> u32 {
    let mut stream = MsvcRand::default();
    stream.srand(0x1B8C_A0AD);
    for _ in 1..draws {
        stream.rand();
    }
    stream.holdrand()
}

// IDA: N/A WinMain 0040EB31..0040EBB8 on the real load, row by row at each of RUN-001's dump points.
#[test]
fn the_world_setup_window_against_p14_run_001() {
    let book = RowBook::load("phase14");
    let records = evidence_lines(RUN_001);
    let started = Instant::now();
    let mut game_state = run_00406360_seeded(Some(RUN_004_ROLL_START));
    println!("00406360: {:.2?}", started.elapsed());
    let GameWindow {
        world,
        collections,
        harness,
        host,
    } = &mut game_state;
    // 00B33398, which the window leaves on a test's stack.
    world.insert_resource(OsGlobalsData::default());
    let ini: &SettingCollection = &collections.ini;
    let lod_land = ini.find_address(0x00B0_2D70).unwrap().raw as u8;
    world.insert_resource(DisplayLodLand(lod_land));
    assert_eq!(lod_land, 1, "the pinned INI's bDisplayLODLand");

    // ---- at 0040EB31 ----
    check_temporary_reference(world);
    check_tes_raw(world, &records, "before");
    check_idle_roots(world, &records, "before");
    check_archives_raw(host, &records, "before");
    check_save_load_raw(world, &records, "before");
    check_tes(world, &book, "before");
    check_worlds(world, &book, "before");
    check_idle_map(world, &book, "before");
    check_archives(host, &book, "before");
    check_archive_settings(host, &book, "before");
    check_tls(world, &book, "tls_before");
    let saveload = book.take_one_kind("SAVELOAD");
    assert_eq!(
        format!("{:X}", game(world).fields_14[1]),
        row_field(&saveload, "flags18")
    );
    assert_eq!(
        format!("{:X}", game(world).field_04),
        row_field(&saveload, "map04")
    );
    read_time_globals(world, &book);
    read_rand(&book);
    let change_before = run_changes(&book, "before");
    let change_tracked = run_changes(&book, "after_tracked");
    let change_final = run_changes(&book, "final");
    let moved = check_change_map(world, &change_before, "before", &[]);
    check_change_head(world, &book, "before", moved);
    let act_rows = book.take_kind("ACT");
    let act_before: Vec<&Vec<&str>> = act_rows.iter().filter(|row| row[1] == "before").collect();
    let act_after: Vec<&Vec<&str>> = act_rows.iter().filter(|row| row[1] != "before").collect();
    assert_eq!((act_before.len(), act_after.len()), (1998, 1998));
    let mut tally = Tally;
    let differences = act_differences(world, &act_before, &mut tally);
    assert!(
        differences.is_empty(),
        "ACT before: {} differ: {:#?}",
        differences.len(),
        &differences[..differences.len().min(12)]
    );
    let manager = world
        .resource::<TreeManagerGlobals>()
        .tree_manager_b39e04
        .as_ref()
        .map(|manager| manager as *const _);
    world.resource_mut::<OsGlobalsData>().unk_18 = 0x1234;

    // ---- 14a ----
    let start = Instant::now();
    let (mut warnings, mut level2) = (Vec::new(), Vec::new());
    let mut map_menu_calls = 0;
    let outputs = {
        let mut services = DataLoadServices {
            working_directory: &harness.root,
            warning: &mut |text, _| warnings.push(text),
            host,
        };
        run_world_setup_worldspace(
            world,
            WorldspaceInputs {
                ini,
                display_lod_land_b43077: lod_land,
                services: &mut services,
                level2_log: &mut |text| level2.push(text),
                refresh_map_menu: &mut |_| {
                    map_menu_calls += 1;
                    Ok(false)
                },
            },
        )
        .unwrap()
    };
    println!("0040EB31..0040EB6D: {:.2?}", start.elapsed());
    assert!(warnings.is_empty(), "no duplicate LOD: {warnings:?}");
    assert_eq!(map_menu_calls, 1, "0057A0D0 once");
    assert_eq!(
        world
            .resource::<TreeManagerGlobals>()
            .tree_manager_b39e04
            .as_ref()
            .map(|manager| manager as *const _),
        manager,
        "0055F750(0) keeps g_treeManager"
    );
    assert_eq!(
        outputs.default_position.map(f32::to_bits),
        DEFAULT_PLAYER_START_BITS,
        "the stack point"
    );
    assert_eq!(
        outputs.lod_find_file_calls,
        64 * 64,
        "004EB1A0's 4,096 lookups"
    );
    let tamriel = outputs.world_space.unwrap();
    assert_eq!(world.get::<FormIdentity>(tamriel).unwrap().form_id_0c, 0x3C);
    check_tes(world, &book, "after_set_ws");
    check_worlds(world, &book, "after_set_ws");
    check_lod_entries(world, &book, tamriel);
    check_tes_raw(world, &records, "after_set_ws");
    check_lod_raw(world, &records, tamriel);
    assert_eq!(level2.len(), 36, "one level 2 log per file");
    assert_eq!(
        level2[0],
        "Found terrain LOD file for -3, -3 in worldspace 'Tamriel'"
    );
    check_idle_map(world, &book, "after_idle");
    check_idle_roots(world, &records, "after_idle");
    let idle_root = outputs.idle_root.unwrap();
    check_idle_root(world, &book, idle_root);

    // ---- 14b ----
    let root_files = world.resource::<IdleAnimRoots>().roots[idle_root]
        .kf_files_20
        .clone();
    let mut logs: Vec<&'static str> = Vec::new();
    {
        let mut services = DataLoadServices {
            working_directory: &harness.root,
            warning: &mut |text, _| warnings.push(text),
            host,
        };
        run_actor_locations_prelude(world, &mut services, &mut |text| logs.push(text));
    }
    assert_eq!(logs, ["Initializing Actor Locations..."]);
    check_archives(host, &book, "after_archives");
    check_archive_settings(host, &book, "after_archives");
    check_archives_raw(host, &records, "after_archives");
    check_tls(world, &book, "tls_after");
    // 0042CC00's re-read: the released names come back through the file.
    let again = host.archives.as_mut().unwrap().find_matching_files(
        "Data\\Meshes\\Characters\\_1stPerson\\*.KF",
        "Characters\\_1stPerson\\IdleAnims",
        1,
    );
    assert_eq!(
        Some(again),
        root_files,
        "0042CC00 reads the released names back"
    );

    // ---- 14c, with TLS +0x185 set, drawing from the stream ----
    assert_eq!(
        host.rand.holdrand(),
        lcg_previous(0x1B8C_A0AD),
        "the stream at 0040EB9F"
    );
    let start = Instant::now();
    {
        let mut random = StreamRandom(&mut host.rand);
        let mut menu_mode = |_: &World| false;
        run_tracked_actors(
            world,
            &mut AiRealArms {
                random: &mut random,
                menu_mode: &mut menu_mode,
            },
        )
        .unwrap();
    }
    println!("0040EB9A..0040EB9F: {:.2?}", start.elapsed());
    assert_eq!(
        host.rand.holdrand(),
        stream_after_draws(157),
        "157 draws, the RAND rows'"
    );
    let mut tally = Tally;
    let differences = act_differences(world, &act_after, &mut tally);
    println!("ACT after: {} of 1,998 differ", differences.len());
    assert!(
        differences.is_empty(),
        "ACT after: {:#?}",
        &differences[..differences.len().min(12)]
    );
    let ai = world.resource::<AiProcessGlobals>();
    assert!(ai.warnings.is_empty(), "{:?}", ai.warnings);
    let drawn = ai.random_subjects.clone();
    let moved = check_change_map(world, &change_tracked, "after_tracked", &drawn);
    check_change_head(world, &book, "after_tracked", moved);

    // ---- 14d ----
    let start = Instant::now();
    let mut logs: Vec<&'static str> = Vec::new();
    let visited = run_world_setup_finish(world, &mut |text| logs.push(text)).unwrap();
    println!("0040EBA4..0040EBB4: {:.2?}", start.elapsed());
    assert_eq!(logs, ["Loading initial area..."]);
    check_autocalc(world, &book, &visited);
    check_autocalc_samples(world, &records);
    let moved = check_change_map(world, &change_final, "final", &drawn);
    check_change_head(world, &book, "final", moved);
    // The delta 14d makes alone: exactly the 1,679 NPCs, each with 21C.
    let tracked: BTreeMap<u32, [u32; 2]> = change_tracked.iter().copied().collect();
    let added: Vec<&(u32, [u32; 2])> = change_final
        .iter()
        .filter(|(id, _)| !tracked.contains_key(id))
        .collect();
    assert_eq!(added.len(), 1679);
    assert!(added.iter().all(|(_, value)| *value == [0x21C, 0]));
    check_tes(world, &book, "final");
    check_tes_raw(world, &records, "final");
    check_save_load_raw(world, &records, "final");
    check_tls(world, &book, "final");
    book.assert_accounted(DEFERRED);
    println!("the block with its checks: {:.2?}", started.elapsed());
}

// IDA: N/A 00406390..0040EBB8 on one World through `run_world_setup`, the form a driver calls, with every block's checks to 00406944 and the block's results after it. Completion: every block in the window.
#[test]
fn the_whole_of_00406390_to_0040ebb8_runs_as_one_sequence() {
    let mut game_state = run_00406360_seeded(Some(RUN_004_ROLL_START));
    let GameWindow {
        world,
        collections,
        harness,
        host,
    } = &mut game_state;
    world.insert_resource(OsGlobalsData::default());
    let lod_land = collections.ini.find_address(0x00B0_2D70).unwrap().raw as u8;
    let mut logs: Vec<&'static str> = Vec::new();
    let mut warnings = Vec::new();
    let mut level2 = Vec::new();
    let mut rand = host.rand;
    let outputs = {
        let mut services = DataLoadServices {
            working_directory: &harness.root,
            warning: &mut |text, _| warnings.push(text),
            host,
        };
        let mut random = StreamRandom(&mut rand);
        let mut menu_mode = |_: &World| false;
        run_world_setup(
            world,
            WorldspaceInputs {
                ini: &collections.ini,
                display_lod_land_b43077: lod_land,
                services: &mut services,
                level2_log: &mut |text| level2.push(text),
                refresh_map_menu: &mut |_| Ok(false),
            },
            WorldSetupSeams {
                log: &mut |text| logs.push(text),
                arms: AiRealArms {
                    random: &mut random,
                    menu_mode: &mut menu_mode,
                },
            },
        )
        .unwrap()
    };
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        logs,
        ["Initializing Actor Locations...", "Loading initial area..."]
    );
    assert_eq!(level2.len(), 36);
    assert_eq!(rand.holdrand(), stream_after_draws(157), "the pass's draws");
    assert_eq!(outputs.autocalc_npcs.len(), 1679);
    assert_eq!(world.resource::<GameThreadLocals>().track_changes_185, 1);
    let book = RowBook::load("phase14");
    check_tes_final(world, &book);
    let rows = book.take_kind("ACT");
    let after: Vec<&Vec<&str>> = rows.iter().filter(|row| row[1] != "before").collect();
    let differences = act_differences(world, &after, &mut Tally);
    assert!(
        differences.is_empty(),
        "{:#?}",
        &differences[..differences.len().min(12)]
    );
    let change_final = run_changes(&book, "final");
    let drawn = world.resource::<AiProcessGlobals>().random_subjects.clone();
    check_change_map(world, &change_final, "final", &drawn);
}

fn check_tes_final(world: &World, book: &RowBook) {
    let row = book.take_one("TES", "final");
    let current = tes(world).current_world_space_74.unwrap();
    assert_eq!(
        format!(
            "{:X}",
            world.get::<FormIdentity>(current).unwrap().form_id_0c
        ),
        row_field(&row, "ws")
    );
}

// ---- The recorded CRT stream against 12d's rolls ----

const LCG_MUL: u32 = 214_013;
const LCG_ADD: u32 = 2_531_011;

fn lcg_previous(state: u32) -> u32 {
    // The inverse of 214013 modulo 2^32.
    let mut inverse: u32 = 1;
    for _ in 0..6 {
        inverse = inverse.wrapping_mul(2u32.wrapping_sub(LCG_MUL.wrapping_mul(inverse)));
    }
    state.wrapping_sub(LCG_ADD).wrapping_mul(inverse)
}

fn lcg_distance(from: u32, to: u32) -> Option<u32> {
    let mut state = from;
    for steps in 0..2_000_000 {
        if state == to {
            return Some(steps);
        }
        state = state.wrapping_mul(LCG_MUL).wrapping_add(LCG_ADD);
    }
    None
}

// IDA: N/A P14-RUN-004's stream ends in a known state at 00675310's first draw. If the main thread draws nothing else between 12d's first roll and that draw, some start k draws back makes the host's whole 00406360 consume exactly k: a fixed point. A hit is then checked against RUN-004's own ACT rows, whose `1A` extras say which actors' containers rolled a change. `P14_RAND_FROM`, `P14_RAND_TO` set the range of k. Ignored: it runs the load once per k.
#[test]
#[ignore = "a search over whole loads: P14_RAND_FROM and P14_RAND_TO"]
fn search_the_roll_start_of_a_recorded_stream() {
    let env = |name: &str, default: u32| {
        std::env::var(name)
            .ok()
            .and_then(|text| text.parse().ok())
            .unwrap_or(default)
    };
    let (from, to) = (env("P14_RAND_FROM", 0), env("P14_RAND_TO", 0));
    // The holdrand before the first draw of RUN-004, and of RUN-005.
    for (name, after_first) in [("RUN-004", 0x1B8C_A0AD_u32), ("RUN-005", 0x2005_F3E0)] {
        let target = lcg_previous(after_first);
        let mut start = target;
        for _ in 0..from {
            start = lcg_previous(start);
        }
        for k in from..=to {
            let mut game = run_00406360_seeded(Some(start));
            let end = game.host.rand.holdrand();
            let used = lcg_distance(start, end).unwrap();
            if end == target {
                println!("{name}: fixed point at k={k}: start {start:08X}");
                if name == "RUN-004" {
                    check_rolls_against_act(&mut game.world);
                }
            } else if k == from {
                println!("{name}: k={k} used {used}");
                if name == "RUN-004" {
                    check_rolls_against_act(&mut game.world);
                }
            }
            start = lcg_previous(start);
        }
    }
}

// IDA: N/A k = 22063, the smallest of RUN-004's fixed points in k = 21600..=22799. There are 117, since a start's rolls can fall into step with a nearby start's; 24 of them (22063..=22074, 22082..=22093) also agree with every ACT row, and the others differ on 2 to 11 of 1,998. From this holdrand the rolls end exactly where RUN-004's 00675310 takes the stream up, and each tracked actor holds a 0x1A extra exactly when the run's did. That's evidence for the roll order and draw count near the end of the rolls. It doesn't prove this is the run's start: no time-shaped seed was found behind it, so the stream's origin is unresolved. It checks one bit per actor, and nothing about which item a roll picked, the non-actor containers, or `PASSES create` and `item_runs`, which come from other runs.
#[test]
fn the_rolls_replay_from_run_004s_stream_at_its_fixed_point() {
    let mut game = run_00406360_seeded(Some(RUN_004_ROLL_START));
    let end = game.host.rand.holdrand();
    assert_eq!(
        end,
        lcg_previous(0x1B8C_A0AD),
        "the state before RUN-004's first 00675310 draw"
    );
    assert_eq!(lcg_distance(RUN_004_ROLL_START, end), Some(22063));
    assert_eq!(check_rolls_against_act(&mut game.world), (1998, 0));
    // The next draw is RAND's first.
    assert_eq!(game.host.rand.rand(), 0x1B8C);
}

// IDA: N/A which of RUN-004's actors hold a 0x1A extra, against the host's. Answers how many agree and differ.
fn check_rolls_against_act(world: &mut World) -> (usize, usize) {
    use recomp_game::tes::base_extra_list::EXTRA_CONTAINER_CHANGES;
    let (mut same, mut differ) = (0, 0);
    for line in fixtures::rows("phase14") {
        if !line.starts_with("ACT before ") {
            continue;
        }
        let id = u32::from_str_radix(field(&line, "id"), 16).unwrap();
        let run = field(&line, "ext")
            .split(',')
            .any(|entry| entry.starts_with("1A:"));
        let form = world.resource::<FormsById>().lookup(id).unwrap();
        let host = world
            .get::<TesObjectRefr>(form)
            .is_some_and(|refr| refr.extra_data_44.get(EXTRA_CONTAINER_CHANGES).is_some());
        if run == host {
            same += 1;
        } else {
            differ += 1;
        }
    }
    let inventory = world.resource::<InventoryGlobals>();
    println!(
        "  ACT before's 1A extras: {same} agree, {differ} differ; created {}, passes {}, item runs {}, script runs {}",
        inventory.container_changes_created,
        inventory.container_script_passes,
        inventory.container_item_script_runs,
        inventory.reference_script_runs
    );
    (same, differ)
}

// ---- The arms no row reaches ----

fn small_world() -> (World, Entity) {
    let mut world = World::new();
    world.init_resource::<GameThreadLocals>();
    world.insert_resource(TesGlobals {
        save_load_game: Some(TesSaveLoadGame::default()),
        ..TesGlobals::default()
    });
    construct_data_handler(&mut world).unwrap();
    let form = world
        .spawn((FormIdentity {
            form_type_04: 0x1B,
            flags_08: 0,
            form_id_0c: 0x1234,
        },))
        .id();
    (world, form)
}

fn changes(world: &World) -> Vec<(u32, u32)> {
    game(world)
        .changes_00
        .walk()
        .map(|(id, record)| (id, record.flags_00))
        .collect()
}

#[test]
fn the_0045b670_gates_and_its_maps() {
    let (mut world, form) = small_world();
    // +0x185 clear.
    set_change_flags(&mut world, form, 4).unwrap();
    assert!(changes(&world).is_empty(), "0045B689");
    world.resource_mut::<GameThreadLocals>().track_changes_185 = 1;
    world.resource_mut::<GameThreadLocals>().byte_184 = 1;
    set_change_flags(&mut world, form, 4).unwrap();
    assert!(changes(&world).is_empty(), "0045B692");
    world.resource_mut::<GameThreadLocals>().byte_184 = 0;
    world.resource_mut::<DataHandler>().bytes_cd0[4] = 1;
    set_change_flags(&mut world, form, 4).unwrap();
    assert!(changes(&world).is_empty(), "0045B6A1");
    world.resource_mut::<DataHandler>().bytes_cd0[4] = 0;
    world.get_mut::<FormIdentity>(form).unwrap().flags_08 = 0x4000;
    set_change_flags(&mut world, form, 4).unwrap();
    assert!(changes(&world).is_empty(), "0045B6B0: a temporary form");
    world.get_mut::<FormIdentity>(form).unwrap().flags_08 = 0;
    // The flags OR on one entry.
    set_change_flags(&mut world, form, 4).unwrap();
    set_change_flags(&mut world, form, 0x10).unwrap();
    assert_eq!(changes(&world), [(0x1234, 0x14)], "00452C84");
    assert_eq!(game(&world).changes_map_00.count_0c, 1);
    let mut globals = world.resource_mut::<TesGlobals>();
    let save = globals.save_load_game.as_mut().unwrap();
    // Active with no bit 3: nothing, even with a map.
    save.fields_14[1] = 1;
    save.field_04 = 1;
    save.set_change_flags(0x55, 1);
    assert!(save.changes_04.is_empty(), "0045B6C6");
    // Active, bit 3 and a +0x04 map.
    save.fields_14[1] = 1 | 8;
    save.set_change_flags(0x55, 1);
    assert_eq!(save.changes_04.get(0x55).unwrap().flags_00, 1, "0045B6D5");
    // Active, bit 3, no map: nothing, where 0045B700 falls back to +0x00.
    save.field_04 = 0;
    save.set_change_flags(0x56, 1);
    assert!(save.changes_00.get(0x56).is_none(), "0045B6CD");
    save.record_form_change(0x56, 2);
    assert_eq!(save.changes_00.get(0x56).unwrap().flags_00, 2, "0045B74F");
    // 0045B700 with +0x04 goes there.
    save.field_04 = 1;
    save.record_form_change(0x57, 2);
    assert_eq!(save.changes_04.get(0x57).unwrap().flags_00, 2, "0045B745");
    save.field_04 = 0;
    save.fields_14[1] = 0;
}

#[test]
fn the_00452d60_arms() {
    let (mut world, form) = small_world();
    // No TLS gate, unlike vt+0x40.
    world
        .resource_mut::<TesGlobals>()
        .save_load_game
        .as_mut()
        .unwrap()
        .record_form_change(0x1234, 3);
    clear_change_flags(&mut world, form, 1).unwrap();
    assert_eq!(changes(&world), [(0x1234, 2)], "00452DAD");
    // +0x18 bit 12 answers untouched.
    world
        .resource_mut::<TesGlobals>()
        .save_load_game
        .as_mut()
        .unwrap()
        .fields_14[1] = 0x1000;
    clear_change_flags(&mut world, form, 2).unwrap();
    assert_eq!(changes(&world), [(0x1234, 2)], "00452D72");
    world
        .resource_mut::<TesGlobals>()
        .save_load_game
        .as_mut()
        .unwrap()
        .fields_14[1] = 0;
    // A temporary form returns first.
    world.get_mut::<FormIdentity>(form).unwrap().flags_08 = 0x4000;
    clear_change_flags(&mut world, form, 2).unwrap();
    assert_eq!(changes(&world), [(0x1234, 2)], "004533DD");
    world.get_mut::<FormIdentity>(form).unwrap().flags_08 = 0;
    // At zero, the record goes, and a later key links at the head again.
    clear_change_flags(&mut world, form, 2).unwrap();
    assert!(changes(&world).is_empty(), "00452DBA");
    assert_eq!(game(&world).changes_map_00.count_0c, 0);
}

#[test]
fn a_new_key_links_at_its_chains_head() {
    let mut save = TesSaveLoadGame::default();
    // 3 and 3 + 0x13AF share a bucket.
    save.record_form_change(3, 1);
    save.record_form_change(3 + 0x13AF, 2);
    save.record_form_change(2, 4);
    let walked: Vec<u32> = save.changes_00.walk().map(|(id, _)| id).collect();
    assert_eq!(walked, [2, 3 + 0x13AF, 3], "00452570 and 00452600");
    assert_eq!(save.changes_map_00.count_0c, 3);
}

#[test]
fn removing_a_change_record() {
    let mut save = TesSaveLoadGame::default();
    save.record_form_change(7, 1);
    // +0x18 bit 12 answers false untouched.
    save.fields_14[1] = 0x1000;
    assert_eq!(save.remove_change(7, true), Ok(false), "00452E02");
    assert_eq!(save.changes_map_00.count_0c, 1);
    save.fields_14[1] = 0;
    // A miss.
    assert_eq!(save.remove_change(8, false), Ok(false), "00452E2C");
    // A hit.
    assert_eq!(save.remove_change(7, false), Ok(true), "00452E60");
    assert_eq!(save.changes_map_00.count_0c, 0);
    assert!(save.changes_00.is_empty());
}

#[test]
fn a_temporary_mark_removes_the_record_then_sets_the_flag() {
    let (mut world, form) = small_world();
    world
        .resource_mut::<TesGlobals>()
        .save_load_game
        .as_mut()
        .unwrap()
        .record_form_change(0x1234, 8);
    mark_temporary(&mut world, form).unwrap();
    assert!(changes(&world).is_empty(), "0046B5A5");
    assert_eq!(
        world.get::<FormIdentity>(form).unwrap().flags_08,
        0x4000,
        "0046B5AA"
    );
    // A form that's already temporary keeps its record: 0045B78D.
    let other = world
        .spawn(FormIdentity {
            form_type_04: 0x1B,
            flags_08: 0x4000,
            form_id_0c: 0x99,
        })
        .id();
    world
        .resource_mut::<TesGlobals>()
        .save_load_game
        .as_mut()
        .unwrap()
        .record_form_change(0x99, 8);
    mark_temporary(&mut world, other).unwrap();
    assert_eq!(changes(&world), [(0x99, 8)]);
}

#[test]
fn the_004637c0_head() {
    let (mut world, form) = small_world();
    assert_eq!(load_form_change(&world, form), Ok(false), "not active");
    let mut globals = world.resource_mut::<TesGlobals>();
    let save = globals.save_load_game.as_mut().unwrap();
    save.fields_14[1] = 1;
    assert_eq!(load_form_change(&world, form), Ok(false), "no record");
    world
        .resource_mut::<TesGlobals>()
        .save_load_game
        .as_mut()
        .unwrap()
        .record_form_change(0x1234, 1);
    assert_eq!(load_form_change(&world, form), Ok(false), "no block");
}

fn world_form(world: &mut World, id: u32, parent: FormRef) -> Entity {
    let form = world
        .spawn((FormIdentity {
            form_type_04: 0x35,
            flags_08: 0,
            form_id_0c: id,
        },))
        .id();
    world.entity_mut(form).insert((
        OriginalVtable(0),
        TesWorldSpace {
            lod_38: PointerMap::new(0x00A4_79D4, 0x25),
            world_48: Some(form),
            parent_7c: parent,
            ..TesWorldSpace::default()
        },
    ));
    form
}

fn lod_services<'a>(
    find: &'a mut dyn FnMut(&str) -> i32,
    warning: &'a mut dyn FnMut(String),
    log: &'a mut dyn FnMut(String),
) -> TerrainLodServices<'a> {
    TerrainLodServices {
        display_lod_land_b43077: 1,
        find_file: find,
        warning,
        debug_log: log,
        find_file_calls: 0,
    }
}

#[test]
fn the_root_lod_map_follows_the_parent_chain() {
    let mut world = World::new();
    let root = world_form(&mut world, 1, FormRef::Resolved(None));
    let middle = world_form(&mut world, 2, FormRef::Resolved(Some(root)));
    let child = world_form(&mut world, 3, FormRef::Resolved(Some(middle)));
    assert_eq!(world_lod_root(&world, child), Ok(root));
    assert_eq!(world_lod_root(&world, root), Ok(root));
    let raw = world_form(&mut world, 4, FormRef::Raw(0x10));
    assert!(world_lod_root(&world, raw).is_err());
}

#[test]
fn a_nonempty_lod_map_answers_zero_without_a_lookup() {
    let mut world = World::new();
    let form = world_form(&mut world, 1, FormRef::Resolved(None));
    world
        .get_mut::<TesWorldSpace>(form)
        .unwrap()
        .lod_38
        .set_at(5, TerrainLodEntry::default());
    let (mut find, mut warning, mut log) = (|_: &str| 1, |_: String| {}, |_: String| {});
    let mut services = lod_services(&mut find, &mut warning, &mut log);
    assert_eq!(
        terrain_lod_activate(&mut world, form, &mut services),
        Ok(0),
        "004EBC0A"
    );
    assert_eq!(services.find_file_calls, 0);
    assert!(
        world.get_resource::<WorldFormGlobals>().is_none(),
        "00B3608F isn't stored"
    );
}

#[test]
fn the_lod_file_names_and_keys() {
    let mut world = World::new();
    let form = world_form(&mut world, 60, FormRef::Resolved(None));
    let mut paths = Vec::new();
    let mut find = |path: &str| {
        paths.push(path.to_owned());
        i32::from(path.ends_with("60.-32.32.32.NIF"))
    };
    let (mut warnings, mut logs) = (Vec::new(), Vec::new());
    {
        let mut warning = |text: String| warnings.push(text);
        let mut log = |text: String| logs.push(text);
        let mut services = lod_services(&mut find, &mut warning, &mut log);
        assert_eq!(terrain_lod_activate(&mut world, form, &mut services), Ok(0));
        assert_eq!(services.find_file_calls, 4096);
    }
    assert_eq!(
        paths[0],
        ".\\Data\\Meshes\\Landscape\\LOD\\60.-1024.-1024.32.NIF"
    );
    assert_eq!(
        paths[64 * 32 + 32],
        ".\\Data\\Meshes\\Landscape\\LOD\\60.00.00.32.NIF"
    );
    assert_eq!(
        paths[64 * 31 + 33],
        ".\\Data\\Meshes\\Landscape\\LOD\\60.-32.32.32.NIF"
    );
    let key = (-1_i32 as u32) << 16 | 1;
    let map = &world.get::<TesWorldSpace>(form).unwrap().lod_38;
    assert_eq!(map.count_0c, 1);
    assert_eq!(map.get(key).unwrap().x_08, -1);
    assert_eq!(map.get(key).unwrap().y_0a, 1);
    assert_eq!(logs.len(), 1);
    assert!(warnings.is_empty());
    assert_eq!(
        world.resource::<WorldFormGlobals>().display_lod_land_b3608f,
        1,
        "004EBC1F"
    );
}

#[test]
fn the_lod_quad_states() {
    let mut quad = TerrainLodQuad::new();
    assert_eq!(quad.state_08, 5, "004EC6D2");
    // 2 becomes 4, then 004EC810's 5.
    quad.state_08 = 2;
    terrain_lod_quad_detach(&mut quad).unwrap();
    assert_eq!(quad.state_08, 4, "004ECB44");
    terrain_lod_quad_detach(&mut quad).unwrap();
    assert_eq!(quad.state_08, 5, "004EC866");
    // 3 stops, and so does releasing a node.
    quad.state_08 = 3;
    assert!(terrain_lod_quad_detach(&mut quad).is_err());
    quad.state_08 = 4;
    quad.node_2c = Some(Entity::from_raw_u32(1).unwrap());
    assert!(terrain_lod_quad_detach(&mut quad).is_err());
    assert_eq!(quad.state_08, 4);
    quad.node_2c = None;
    // The children go along, in any state of the parent.
    let mut child = TerrainLodQuad::new();
    child.state_08 = 2;
    quad.state_08 = 5;
    quad.children_30 = Some(Box::new([
        child.clone(),
        child.clone(),
        child.clone(),
        child,
    ]));
    terrain_lod_quad_detach(&mut quad).unwrap();
    assert!(
        quad.children_30
            .as_ref()
            .unwrap()
            .iter()
            .all(|child| child.state_08 == 4)
    );
}

#[test]
fn the_lod_refresh_runs_three_passes_and_the_land_arm_stops() {
    let mut world = World::new();
    let form = world_form(&mut world, 1, FormRef::Resolved(None));
    let mut entry = TerrainLodEntry::default();
    entry.quad_00.state_08 = 2;
    world
        .get_mut::<TesWorldSpace>(form)
        .unwrap()
        .lod_38
        .set_at(1, entry);
    terrain_lod_refresh(&mut world, form, false).unwrap();
    // 2, 4, 5, 5.
    assert_eq!(
        world
            .get::<TesWorldSpace>(form)
            .unwrap()
            .lod_38
            .get(1)
            .unwrap()
            .quad_00
            .state_08,
        5
    );
    assert!(
        terrain_lod_refresh(&mut world, form, true).is_err(),
        "004EA095"
    );
}

fn world_with_tes() -> World {
    let mut world = World::new();
    world.insert_resource(TesGlobals {
        tes: Some(Box::new(Tes::default())),
        ..TesGlobals::default()
    });
    world.insert_resource(DataHandlerLists::default());
    world.insert_resource(OsGlobalsData {
        unk_18: 7,
        ..OsGlobalsData::default()
    });
    world
}

#[test]
fn a_null_or_unchanged_world_returns_and_the_unload_block_stops() {
    let mut world = world_with_tes();
    let mut collections = SettingCollections::default();
    register_static_settings(&mut collections);
    let form = world_form(&mut world, 1, FormRef::Resolved(None));
    let (mut find, mut warning, mut log) = (|_: &str| 0, |_: String| {}, |_: String| {});
    let mut menu = |_: &mut World| Ok(false);
    let mut services = lod_services(&mut find, &mut warning, &mut log);
    set_current_worldspace(&mut world, None, &collections.ini, &mut services, &mut menu).unwrap();
    assert_eq!(world.resource::<OsGlobalsData>().unk_18, 7, "004431FA");
    world
        .resource_mut::<TesGlobals>()
        .tes
        .as_mut()
        .unwrap()
        .current_world_space_74 = Some(form);
    set_current_worldspace(
        &mut world,
        Some(form),
        &collections.ini,
        &mut services,
        &mut menu,
    )
    .unwrap();
    assert_eq!(world.resource::<OsGlobalsData>().unk_18, 7, "00443203");
    // bPreemptivelyUnloadCells.
    collections.ini.find_address_mut(0x00B0_51CC).unwrap().raw = 1;
    let other = world_form(&mut world, 2, FormRef::Resolved(None));
    let error = set_current_worldspace(
        &mut world,
        Some(other),
        &collections.ini,
        &mut services,
        &mut menu,
    )
    .unwrap_err();
    assert_eq!(error.0.split(':').next(), Some("0044324A"));
    assert_eq!(
        world.resource::<OsGlobalsData>().unk_18,
        7,
        "the store comes after the block"
    );
}
