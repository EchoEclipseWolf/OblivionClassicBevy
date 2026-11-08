//! Phase 12a: DataHandler_LoadSelectedFiles 0044F3D0 over the shipped Data against P12-RUN-001 and P12-RUN-003, then synthetic plugins for the arms the run didn't take. Host tests earn no validation bonus.
//!
//! `every_row_is_read` holds the accounting of `fixtures/rows/phase12.rows`: each line is read here or named in `DEFERRED` with its slice. A later slice removes its groups from `DEFERRED` and adds its class to `common/images.rs`, and the table ends empty.
//! The run's own values come from `fixtures/evidence/phase12-run001-2026-10-09.records.txt`, not from text typed here.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use bevy_ecs::prelude::*;
use common::image::Image;
use common::images::image_of;
use common::images_magic_actors::{Behind, behind};
use common::load::{Loader, install_player, pre_load_default_globals};
use common::rows::{Deferred, RowBook, hex_bytes, hex_ids};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use recomp_core::fixtures;
use recomp_core::hash::{FNV1A_BASIS, fnv1a_dword};
use recomp_game::bs::archive::{ArchiveManager, ArchiveSettings};
use recomp_game::bs::file_finder::{BsFileCallbacks, FileFinder};
use recomp_game::tes::actor::TesObjectRefr;
use recomp_game::tes::actor_process::ActorProcessGlobals;
use recomp_game::tes::data_handler::{
    DataHandler, DataHandlerLists, EditorIdMap, FormsById, TesSkill, TesTexture,
    construct_data_handler,
};
use recomp_game::tes::data_load::{DataLoadGlobals, record_percent};
use recomp_game::tes::form::FormIdentity;
use recomp_game::tes::form_arms::{check_classes_against_the_map, form_class};
use recomp_game::tes::form_components::{
    DataChunkTails, FormRef, TesAttackDamageForm, TesContainer, TesEnchantableForm, TesFullName,
    TesHealthForm, TesLeveledList, TesObjectLinks, TesReactionForm, TesScriptableForm, TesUsesForm,
    TesValueForm, TesWeightForm, load_data_chunk,
};
use recomp_game::tes::forms_items::{
    TesBook, TesDoor, TesLevCreature, TesLight, TesMagicObject, TesSoulGem, TesSubSpace, TesTree,
    TesWeapon, set_change_flags,
};
use recomp_game::tes::forms_magic::{EffectItemList, EffectSetting, EffectSettingCollection};
use recomp_game::tes::forms_misc::{TesClass, TesFaction, TesLandTexture};
use recomp_game::tes::forms_region::{RegionDataKind, TesRegion, TesRegionData};
use recomp_game::tes::forms_script::Script;
use recomp_game::tes::forms_world::{TesObjectCell, TesWorldSpace};
use recomp_game::tes::game_settings::{GameSettingCollection, setting_type};
use recomp_game::tes::global::TESGlobal;
use recomp_game::tes::tes::GameThreadLocals;

/// IDA: N/A what later slices read, by row kind and form type or list offset. The types of each slice follow `form-type-map.tsv` and the C++ family files.
const DEFERRED: &[Deferred] = &[];

/// IDA: N/A the form types this file reads the COUNT, SAMPLE and DATA rows of.
const READ_TYPES: [&str; 33] = [
    "04", "05", "0B", "06", "07", "08", "0A", "0D", "0E", "12", "13", "14", "15", "16", "17", "18",
    "19", "1A", "1B", "1C", "1D", "1E", "1F", "20", "21", "22", "25", "26", "27", "28", "29", "2A",
    "2B",
];
fn shipped_root() -> PathBuf {
    let data = fixtures::data_dir();
    assert!(
        data.join("Oblivion.esm").exists(),
        "this test needs the game data: set OBLIVION_DATA_DIR ({data:?})"
    );
    data.parent().unwrap().canonicalize().unwrap()
}

/// IDA: N/A the records of P12-RUN-001, as Latin-1 lines.
fn run_records() -> Vec<String> {
    let path = fixtures::root().join("evidence/phase12-run001-2026-10-09.records.txt");
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

// IDA: N/A `key=value` of a record line, split on spaces.
fn field<'a>(line: &'a str, key: &str) -> &'a str {
    let prefix = format!("{key}=");
    line.split(' ')
        .find_map(|part| part.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("no {key} in {line}"))
}

fn hex(text: &str) -> u32 {
    u32::from_str_radix(text, 16).unwrap()
}

fn ids_fnv(ids: &[u32]) -> u32 {
    ids.iter()
        .fold(FNV1A_BASIS, |hash, id| fnv1a_dword(hash, *id))
}

// IDA: N/A a shipped load with the classes this slice has. The other arms count and skip.
fn shipped_loader() -> Loader {
    let root = shipped_root();
    let mut loader = Loader::new(&root);
    assert!(loader.warnings.is_empty(), "{:?}", loader.warnings);
    loader.load_all();
    install_player(&mut loader.world);
    loader.host.real_pre_load = true;
    loader.host.real_tail = true;
    let mut callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new(&root, &mut callbacks);
    finder.add("Data");
    loader.host.finder = Some(finder);
    loader.host.archives = Some(shipped_archives(&root));
    loader
}

// IDA: N/A the shipped archives, for 0042DB10's lookups. Plugins.txt only names which DLC archives join, past the first texture archive that type 1 asks.
fn shipped_archives(root: &Path) -> ArchiveManager {
    let mut callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new(root, &mut callbacks);
    finder.add("Data");
    let settings = ArchiveSettings {
        invalidate_older_files: false,
        ..ArchiveSettings::default()
    };
    let scratch = std::env::temp_dir().join(format!(
        "recomp_phase12_archives_{}_{}",
        std::process::id(),
        std::thread::current()
            .name()
            .unwrap_or("x")
            .replace("::", "_")
    ));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join("Plugins.txt"), "Oblivion.esm\n").unwrap();
    let mut archives = ArchiveManager::new(root, settings);
    archives.initialize_from_settings(&finder, &scratch);
    let _ = std::fs::remove_dir_all(&scratch);
    archives
}

// IDA: N/A settings for a synthetic plugin's EFIDs, past the 161 of 00417420: {code, flags, associated, name}. A new code takes a new form; one already there takes its fields, without a record's reset.
fn install_effect_settings(world: &mut World, rows: &[(u32, u32, u32, &[u8])]) {
    let saved = world.resource::<GameThreadLocals>().byte_184;
    world.resource_mut::<GameThreadLocals>().byte_184 = 1;
    let class = form_class(0x0C).unwrap();
    for &(code, flags, associated, name) in rows {
        let form = (class.construct)(world).unwrap();
        let mut setting = world.get_mut::<EffectSetting>(form).unwrap();
        setting.code_98 = code;
        setting.data_58.flags_58 = flags;
        setting.data_58.associated_60 = associated;
        world
            .get_mut::<TesFullName>(form)
            .unwrap()
            .name_04
            .set(Some(name), 0);
        world
            .resource_mut::<EffectSettingCollection>()
            .set_at(code, form);
    }
    world.resource_mut::<GameThreadLocals>().byte_184 = saved;
}

// IDA: N/A compares one row of raw bytes, marking its DATA rows read: a pointer's row by the image's offset, then any row left, which the run dumped behind a word that isn't a pointer here.
fn compare_sample(world: &World, book: &RowBook, row: &[&str], data_rows: bool) {
    let (form_type, id) = (row[1], row[2]);
    let form = world
        .resource::<FormsById>()
        .lookup(hex(id))
        .unwrap_or_else(|| panic!("{} {form_type} {id}: no form", row[0]));
    assert_eq!(
        u32::from(world.get::<FormIdentity>(form).unwrap().form_type_04),
        hex(form_type),
        "{} {form_type} {id}",
        row[0]
    );
    let image: Image = image_of(world, form).unwrap_or_else(|| panic!("{form_type}: no image"));
    let raw = hex_bytes(row[3]);
    let tag = format!("{} {form_type} {id}", row[0]);
    if data_rows {
        image.compare(&raw, &tag, &|offset| book.data(form_type, id, offset));
        compare_neighbours(world, book, form, form_type, id);
    } else {
        image.compare(&raw, &tag, &|_| None);
    }
}

// IDA: N/A a TESObject's +0x1C and +0x20 rows hold the first 0x40 bytes of the forms either side of it: the port's neighbour by its own image.
fn compare_neighbours(world: &World, book: &RowBook, form: Entity, form_type: &str, id: &str) {
    let Some(links) = world.get::<TesObjectLinks>(form) else {
        return;
    };
    for (offset, neighbour) in [(0x1C, links.previous_1c), (0x20, links.next_20)] {
        let (Some(neighbour), Some(raw)) = (neighbour, book.data(form_type, id, offset)) else {
            continue;
        };
        let tag = format!("DATA {form_type} {id} {offset:X}");
        let image = image_of(world, neighbour).unwrap_or_else(|| panic!("{tag}: no image"));
        image.compare_start(&raw, &tag, &|_| None);
    }
}

/// The shipped Data through 0044F3D0, against P12-RUN-001's records and every row this slice can read.
#[test]
fn every_row_is_read() {
    let book = RowBook::load("phase12");
    let records = run_records();
    let mut loader = shipped_loader();
    let start = Instant::now();
    let cursor = loader.run_to_tail().unwrap();
    println!("the file pass: {:.2?}", start.elapsed());
    // P12-RUN-005 and P12-RUN-006 print one `forms=` value at 0044F780 RECORDS_DONE and at 004067CC FINAL, so the passes' temporary references leave none. Its 19456 isn't the map's count, which `COUNT` covers: the runs read another word.
    let forms_before_tail = loader.world.resource::<FormsById>().count();
    assert!(
        loader.warnings.is_empty(),
        "P12-RUN-001 reached no warning: {:?}",
        &loader.warnings[..loader.warnings.len().min(5)]
    );
    assert!(
        loader.host.logs.is_empty(),
        "P12-RUN-001 reached no DebugLog: {:?}",
        loader.host.logs
    );
    let world = &loader.world;

    // Records 24..35: +0x8C0, +0x8D0, the compiled order, flags, HEDR words and mod indices.
    let handler = world.resource::<DataHandler>();
    let compiled = records
        .iter()
        .find(|line| line.starts_with("D 24 "))
        .unwrap();
    assert_eq!(handler.next_form_id_8c0(), hex(field(compiled, "8C0")));
    assert_eq!(handler.loaded_mod_count_8d0, hex(field(compiled, "8D0")));
    assert_eq!(handler.active_file_8c4.map(|id| id.0), None);
    let rows: Vec<&String> = records
        .iter()
        .filter(|line| line.starts_with("T ") && line.contains(" 44F6CB compiled "))
        .collect();
    assert_eq!(rows.len(), handler.loaded_mod_count_8d0 as usize);
    for (index, line) in rows.iter().enumerate() {
        let file = handler
            .file_store
            .get(handler.mods_by_id_8d4[index].unwrap())
            .unwrap();
        assert_eq!(file.name_1c, field(line, "name"), "compiled file {index}");
        assert_eq!(
            file.flags_3dc,
            hex(field(line, "flags")),
            "{}",
            file.name_1c
        );
        let mut words = Vec::new();
        for word in file.header_3d0 {
            words.extend(word.to_le_bytes());
        }
        assert_eq!(
            words,
            hex_bytes(field(line, "hedr")),
            "{}: +0x3D0",
            file.name_1c
        );
        assert_eq!(
            u32::from(file.mod_index_400),
            hex(field(line, "b400")),
            "{}: +0x400",
            file.name_1c
        );
    }

    // Records 39, 40.. and D 175: the totals and 00B33A94 at each 0044F0C0 entry, and its first argument.
    let load = world.resource::<DataLoadGlobals>();
    let totals = records
        .iter()
        .find(|line| line.contains(" TOTALS "))
        .unwrap();
    assert_eq!(load.record_total_b33a90, hex(field(totals, "B33A90")));
    let entries: Vec<&String> = records
        .iter()
        .filter(|line| line.contains(" FILE_ENTRY "))
        .collect();
    assert_eq!(load.file_entry_counts.len(), entries.len());
    for (count, line) in load.file_entry_counts.iter().zip(&entries) {
        assert_eq!(*count, hex(field(line, "B33A94")), "{line}");
    }
    assert_eq!(field(entries[0], "first"), "1");
    assert!(entries[1..].iter().all(|line| field(line, "first") == "0"));
    assert_eq!(
        load.file_results,
        vec![1; entries.len()],
        "no 0044F0C0 returned 0"
    );
    // Record 714: the two counters in order, at the window's end.
    let last = records
        .iter()
        .find(|line| line.starts_with("M 714 "))
        .unwrap();
    let counters = hex_bytes(field(last, "B33A90"));
    assert_eq!(load.record_total_b33a90.to_le_bytes(), counters[..4]);
    assert_eq!(load.record_count_b33a94.to_le_bytes(), counters[4..8]);
    // Record 82: the first percent change, at 00B33A94 = 31E6.
    let percent = records
        .iter()
        .find(|line| line.starts_with("A 82 "))
        .unwrap();
    assert_eq!(
        record_percent(hex(field(percent, "B33A94")), hex(field(percent, "B33A90"))),
        hex(field(percent, "eax")) as i32
    );
    assert_eq!(
        record_percent(
            hex(field(percent, "B33A94")) - 1,
            hex(field(percent, "B33A90"))
        ),
        0
    );
    assert_eq!(
        loader.host.progress.first(),
        Some(&(0, 1.0)),
        "the first report"
    );
    // C++ only, and arithmetic from the counters: 0x1C078 / 0x137D97 is 8 percent.
    assert_eq!(loader.host.progress.last(), Some(&(0, 8.0)));
    assert!(
        load.last_progress_text.starts_with("Loading Files 8% ("),
        "{}",
        load.last_progress_text
    );
    assert!(handler.loading_cd7 == 0, "+0xCD7 cleared");
    assert_eq!(
        world.resource::<GameThreadLocals>().byte_184,
        1,
        "TLS +0x184 until 0044F95A"
    );
    // BOUND: 0044F780's walk of the TESObject chain from +0x00's +0x04 through each +0x20.
    {
        let row = book.take_one_kind("BOUND");
        let mut hash = FNV1A_BASIS;
        let mut count = 0_usize;
        let mut object = world.resource::<DataHandlerLists>().objects_00.first_04;
        while let Some(form) = object {
            hash = fnv1a_dword(hash, world.get::<FormIdentity>(form).unwrap().form_id_0c);
            count += 1;
            object = world.get::<TesObjectLinks>(form).unwrap().next_20;
        }
        assert_eq!(count, row[1].parse::<usize>().unwrap(), "BOUND count");
        assert_eq!(hash, hex(row[2]), "BOUND fnv: the chain's order");
    }

    // COUNT <type> <count> <fnv> <first> <last>: the ID-sorted walk of g_formsByID.
    let forms = world.resource::<FormsById>();
    for form_type in READ_TYPES {
        let row = book.take_one("COUNT", form_type);
        let code = hex(form_type) as u8;
        let mut ids: Vec<u32> = forms
            .entries()
            .filter(|(_, form)| world.get::<FormIdentity>(*form).unwrap().form_type_04 == code)
            .map(|(id, _)| id)
            .collect();
        ids.sort_unstable();
        assert_eq!(
            ids.len(),
            row[2].parse::<usize>().unwrap(),
            "COUNT {form_type}"
        );
        assert_eq!(ids_fnv(&ids), hex(row[3]), "COUNT {form_type} fnv");
        assert_eq!(ids[0], hex(row[4]), "COUNT {form_type} first");
        assert_eq!(*ids.last().unwrap(), hex(row[5]), "COUNT {form_type} last");
    }

    // LIST <offset> <count> <fnv> <first three> <last> and PLIST <offset> <count> <fnv> <all>: the DataHandler lists, in list order. PLIST 74 is after the tail, which the run shows leaves that list as it was.
    let lists = world.resource::<DataHandlerLists>();
    let ordered = |offset: u32| -> Vec<u32> {
        lists
            .list_at(offset)
            .unwrap()
            .items()
            .map(|form| world.get::<FormIdentity>(form).unwrap().form_id_0c)
            .collect()
    };
    for offset in ["74", "54", "34", "3C", "4C", "5C", "64", "6C"] {
        let row = book.take_one("LIST", offset);
        let ids = ordered(hex(offset));
        assert_eq!(ids.len(), row[2].parse::<usize>().unwrap(), "LIST {offset}");
        assert_eq!(
            ids_fnv(&ids),
            hex(row[3]),
            "LIST {offset} fnv: the original order"
        );
        assert_eq!(ids[..3], hex_ids(row[4])[..], "LIST {offset} head");
        assert_eq!(*ids.last().unwrap(), hex(row[5]), "LIST {offset} last");
    }
    family_b_list_rows(world, &book);

    // AC20 and AC20RAW: the forms 0044AC20 leaves, before any record. This world stops before the compile, as the rows do, with only the skills and the six globals of that pass.
    let mut before = Loader::new(&shipped_root());
    let image_world = &mut before.world;
    pre_load_default_globals(image_world);
    for form_type in [
        "04", "0B", "06", "08", "0A", "16", "17", "18", "1B", "1C", "26",
    ] {
        let created = hex_ids(book.take_one("AC20", form_type)[2]);
        let mut seen = Vec::new();
        for row in book.take("AC20RAW", form_type) {
            seen.push(hex(row[2]));
            compare_sample(image_world, &book, &row, false);
        }
        assert_eq!(seen, created, "AC20 {form_type}: the row's order");
    }
    for form_type in ["09", "10", "23", "32"] {
        let created = hex_ids(book.take_one("AC20", form_type)[2]);
        let mut seen = Vec::new();
        for row in book.take("AC20RAW", form_type) {
            seen.push(hex(row[2]));
            compare_sample(image_world, &book, &row, false);
        }
        assert_eq!(seen, created, "AC20 {form_type}: the creation order");
    }
    let mut oldest_first: Vec<u32> = image_world
        .resource::<DataHandlerLists>()
        .globals_74
        .items()
        .map(|form| image_world.get::<FormIdentity>(form).unwrap().form_id_0c)
        .collect();
    oldest_first.reverse();
    assert_eq!(
        oldest_first,
        hex_ids(
            book.lines
                .iter()
                .find(|line| line.starts_with("AC20 04 "))
                .unwrap()
                .split(' ')
                .nth(2)
                .unwrap()
        )
    );

    // SAMPLE and POST: the forms' raw bytes after the record loop, and after the tail. These classes' vt+0x6C is 0060D0A0.
    let mut samples = 0;
    for form_type in READ_TYPES {
        for row in book.take("SAMPLE", form_type) {
            compare_sample(world, &book, &row, true);
            samples += 1;
        }
        // Each pointer cell of an image has read its DATA row by now. The rest of this type's are dumps behind words that only looked like pointers to the run's script: a type byte and its unset bytes, a form ID, a raw count. Nothing compares them.
        let data_ids: Vec<String> = book
            .lines
            .iter()
            .filter(|line| line.starts_with(&format!("DATA {form_type} ")))
            .map(|line| line.split(' ').nth(2).unwrap().to_owned())
            .collect();
        for id in data_ids {
            for offset in book.take_other_data(form_type, &id) {
                println!("DATA {form_type} {id} {offset:X}: no pointer here, not compared");
            }
        }
    }
    println!("samples and post rows compared: {samples}");
    family_b_sample_rows(world, &book);

    family_c_rows(world, &book);
    dialogue_rows(world, &book);

    world_family_rows(world, image_world, &book);

    // The tail, 0044F780..0044FA38, then what P12-RUN-005 dumped at 004067CC.
    let tail_start = Instant::now();
    assert_eq!(
        loader.finish(cursor),
        Ok(1),
        "0044F3D0 returns 1 over the shipped Data"
    );
    println!("the post-load tail: {:.2?}", tail_start.elapsed());
    assert_eq!(
        (loader.host.pre_load_calls, loader.host.post_load_calls),
        (1, 1)
    );
    assert!(
        loader.warnings.is_empty(),
        "P12-RUN-005 reached no warning in the passes: {} of them, {:?}",
        loader.warnings.len(),
        &loader.warnings[..loader.warnings.len().min(5)]
    );
    assert_eq!(
        loader.world.resource::<FormsById>().count(),
        forms_before_tail,
        "004067CC"
    );
    // P12-RUN-006 FINAL: 004F1AC0 ran 56 times, under its probe cap of 400. Each makes one subspace reference's entries.
    let world = &loader.world;
    let subspace_calls: usize = world
        .resource::<DataHandlerLists>()
        .world_spaces_0c
        .items()
        .filter_map(|form| world.get::<TesWorldSpace>(form).unwrap().persistent_cell_34)
        .map(|cell| {
            world
                .get::<TesObjectCell>(cell)
                .unwrap()
                .objects_48
                .items()
                .filter(|&object| {
                    world
                        .get::<TesObjectRefr>(object)
                        .unwrap()
                        .base_form_1c
                        .is_some_and(|base| {
                            world.get::<FormIdentity>(base).unwrap().form_type_04 == 0x29
                        })
                })
                .count()
        })
        .sum();
    assert_eq!(subspace_calls, 56, "P12-RUN-006 4F1AC0:56");
    assert_eq!(loader.world.resource::<GameThreadLocals>().byte_184, 0);
    assert_eq!(
        loader
            .world
            .resource::<DataLoadGlobals>()
            .current_cell_b33a9c,
        None
    );
    post_rows(&loader.world, &book);

    book.assert_accounted(DEFERRED);
    check_classes_against_the_map().unwrap();
}

// IDA: N/A the PLIST and POST rows of P12-RUN-005 at 004067CC.
fn post_rows(world: &World, book: &RowBook) {
    let lists = world.resource::<DataHandlerLists>();
    let mut failures = Vec::new();
    for offset in [
        "04", "0C", "14", "1C", "24", "2C", "34", "3C", "44", "4C", "54", "5C", "64", "6C", "74",
        "7C", "84", "8C", "94", "9C", "A4", "AC", "B4",
    ] {
        let row = book.take_one("PLIST", offset);
        let ids: Vec<u32> = lists
            .list_at(hex(offset))
            .unwrap()
            .items()
            .map(|form| world.get::<FormIdentity>(form).unwrap().form_id_0c)
            .collect();
        if ids.len() != row[2].parse::<usize>().unwrap() {
            failures.push(format!(
                "PLIST {offset}: {} forms, the run {}",
                ids.len(),
                row[2]
            ));
        }
        if ids_fnv(&ids) != hex(row[3]) {
            failures.push(format!("PLIST {offset}: the order's FNV"));
        }
        // The row holds the head of the list; its FNV covers the rest.
        let head = hex_ids(row[4]);
        if ids.get(..head.len()) != Some(&head[..]) {
            let at = head
                .iter()
                .zip(&ids)
                .position(|(run, host)| run != host)
                .unwrap_or(0);
            failures.push(format!("PLIST {offset}: differs from item {at}"));
        }
    }
    // The run walked the wrong head for +0xBC, so its two words are no list: read, and not compared. `docs/open-items.md` keeps the capture.
    let row = book.take_one("PLIST", "BC");
    assert_eq!(row[2], "2", "PLIST BC is the known junk row");
    fixtures::ledger_relaxed(
        "phase12",
        "PLIST BC",
        1,
        "the run walked the wrong head for +0xBC: two junk words, no list (docs/open-items.md)",
    );
    assert!(failures.is_empty(), "{}", failures.join("; "));
    let mut compared = 0;
    let mut coverage = (0, 0, 0);
    let mut failed = Vec::new();
    let types: Vec<String> = book
        .lines
        .iter()
        .filter(|line| line.starts_with("POST "))
        .map(|line| line.split(' ').nth(1).unwrap().to_owned())
        .collect();
    let mut seen: Vec<&String> = Vec::new();
    for form_type in &types {
        if seen.contains(&form_type) {
            continue;
        }
        seen.push(form_type);
        for row in book.take("POST", form_type) {
            compared += 1;
            if let Some(image) = world
                .resource::<FormsById>()
                .lookup(hex(row[2]))
                .and_then(|form| image_of(world, form))
            {
                let (bytes, pointers, unset) = image.coverage();
                coverage = (
                    coverage.0 + bytes,
                    coverage.1 + pointers,
                    coverage.2 + unset,
                );
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                compare_sample(world, book, &row, false);
            }));
            if let Err(error) = result {
                failed.push(
                    error
                        .downcast_ref::<String>()
                        .cloned()
                        .unwrap_or_else(|| format!("POST {form_type}: a panic")),
                );
            }
        }
    }
    println!(
        "POST rows compared: {compared}: {} bytes exact, {} pointers by null-ness, {} bytes unset",
        coverage.0, coverage.1, coverage.2
    );
    assert!(
        failed.is_empty(),
        "{} POST rows failed:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

// IDA: N/A the 12c world and reference rows: COUNT, SAMPLE, LIST +0x0C and +0x8B8, and the default world's AC20. These types have no DATA rows, so a pointer compares by null-ness and nothing behind one reaches a row.
fn world_family_rows(world: &World, before: &World, book: &RowBook) {
    let forms = world.resource::<FormsById>();
    for form_type in ["30", "31", "32", "33", "34", "35", "36", "38"] {
        let row = book.take_one("COUNT", form_type);
        let code = hex(form_type) as u8;
        let mut ids: Vec<u32> = forms
            .entries()
            .filter(|(_, form)| world.get::<FormIdentity>(*form).unwrap().form_type_04 == code)
            .map(|(id, _)| id)
            .collect();
        ids.sort_unstable();
        assert_eq!(
            ids.len(),
            row[2].parse::<usize>().unwrap(),
            "COUNT {form_type}"
        );
        assert_eq!(ids_fnv(&ids), hex(row[3]), "COUNT {form_type} fnv");
        assert_eq!(ids[0], hex(row[4]), "COUNT {form_type} first");
        assert_eq!(*ids.last().unwrap(), hex(row[5]), "COUNT {form_type} last");
        for row in book.take("SAMPLE", form_type) {
            compare_sample(world, book, &row, true);
        }
    }
    let lists = world.resource::<DataHandlerLists>();
    for offset in ["0C", "8B8"] {
        let row = book.take_one("LIST", offset);
        let ids: Vec<u32> = lists
            .list_at(hex(offset))
            .unwrap()
            .items()
            .map(|form| world.get::<FormIdentity>(form).unwrap().form_id_0c)
            .collect();
        assert_eq!(ids.len(), row[2].parse::<usize>().unwrap(), "LIST {offset}");
        assert_eq!(
            ids_fnv(&ids),
            hex(row[3]),
            "LIST {offset} fnv: the original order"
        );
        assert_eq!(ids[..3], hex_ids(row[4])[..], "LIST {offset} head");
        assert_eq!(*ids.last().unwrap(), hex(row[5]), "LIST {offset} last");
    }
    // No row holds +0x58, but the sampled actors' flags have no 0x20000, which 006748B0 would have set. With no sleeping player, nothing shipped queues: host fact, not a run's.
    assert_eq!(
        world
            .resource::<ActorProcessGlobals>()
            .manager
            .queued_58
            .items()
            .count(),
        0,
        "006748B0 over the shipped actors"
    );
    let created = hex_ids(book.take_one("AC20", "35")[2]);
    let mut seen = Vec::new();
    for row in book.take("AC20RAW", "35") {
        seen.push(hex(row[2]));
        compare_sample(before, book, &row, false);
    }
    assert_eq!(seen, created, "AC20 35: the creation order");
}

// IDA: N/A Family B's COUNT and LIST rows, as `every_row_is_read` reads Family A's first three. +0x8B8 waits for 12c's REFRs, though no shipped MGEF reaches it (below).
fn family_b_list_rows(world: &World, book: &RowBook) {
    let forms = world.resource::<FormsById>();
    for form_type in ["09", "0C", "0F", "10", "11", "23", "24", "40"] {
        let row = book.take_one("COUNT", form_type);
        let code = hex(form_type) as u8;
        let mut ids: Vec<u32> = forms
            .entries()
            .filter(|(_, form)| world.get::<FormIdentity>(*form).unwrap().form_type_04 == code)
            .map(|(id, _)| id)
            .collect();
        ids.sort_unstable();
        assert_eq!(
            ids.len(),
            row[2].parse::<usize>().unwrap(),
            "COUNT {form_type}"
        );
        assert_eq!(ids_fnv(&ids), hex(row[3]), "COUNT {form_type} fnv");
        assert_eq!(ids[0], hex(row[4]), "COUNT {form_type} first");
        assert_eq!(*ids.last().unwrap(), hex(row[5]), "COUNT {form_type} last");
    }
    let lists = world.resource::<DataHandlerLists>();
    for offset in ["24", "2C", "44", "8C"] {
        let row = book.take_one("LIST", offset);
        let ids: Vec<u32> = lists
            .list_at(hex(offset))
            .unwrap()
            .items()
            .map(|form| world.get::<FormIdentity>(form).unwrap().form_id_0c)
            .collect();
        assert_eq!(ids.len(), row[2].parse::<usize>().unwrap(), "LIST {offset}");
        assert_eq!(
            ids_fnv(&ids),
            hex(row[3]),
            "LIST {offset} fnv: the original order"
        );
        assert_eq!(ids[..3], hex_ids(row[4])[..], "LIST {offset} head");
        assert_eq!(*ids.last().unwrap(), hex(row[5]), "LIST {offset} last");
    }
    assert!(
        lists
            .list_8b8
            .items()
            .all(|form| world.get::<EffectSetting>(form).is_none()),
        "every shipped MGEF record names a built-in setting (the C++ test holds this)"
    );
}

// IDA: N/A Family B's SAMPLE rows and the DATA rows behind them, and POST rows for ENCH and SPEL, whose vt+0x6C is 0060D0A0. The other classes' POST rows wait for their InitItem (12d).
fn family_b_sample_rows(world: &World, book: &RowBook) {
    let mut lifted = 0;
    for form_type in ["09", "0C", "0F", "10", "11", "23", "24", "40"] {
        for row in book.take("SAMPLE", form_type) {
            compare_sample(world, book, &row, true);
            let form = world.resource::<FormsById>().lookup(hex(row[2])).unwrap();
            for (offset, expected) in behind(world, form) {
                let tag = format!("DATA {form_type} {} {offset:X}", row[2]);
                let data = book
                    .data(form_type, row[2], offset)
                    .unwrap_or_else(|| panic!("{tag}: no row"));
                match expected {
                    Behind::Bytes(bytes) => {
                        for (at, (want, run)) in bytes.iter().zip(&data).enumerate() {
                            if let Some(want) = want {
                                assert_eq!(want, run, "{tag} +{at:X}");
                            }
                        }
                    }
                    Behind::NotNull(words) => {
                        for (at, (want, run)) in
                            words.iter().zip(data.as_chunks::<4>().0).enumerate()
                        {
                            assert_eq!(*want, *run != [0; 4], "{tag} +{:X}", at * 4);
                        }
                    }
                }
                lifted += 1;
            }
        }
        if matches!(form_type, "0F" | "10") {
            for row in book.take("POST", form_type) {
                compare_sample(world, book, &row, false);
            }
        }
        let data_ids: Vec<String> = book
            .lines
            .iter()
            .filter(|line| line.starts_with(&format!("DATA {form_type} ")))
            .map(|line| line.split(' ').nth(2).unwrap().to_owned())
            .collect();
        for id in data_ids {
            book.take_other_data(form_type, &id);
        }
    }
    println!("DATA rows compared behind a pointer, Family B: {lifted}");
}

// IDA: N/A the AI, environment and region family's rows: COUNT, LIST, AC20, AC20RAW, SAMPLE, DATA, and POST for the two classes whose vt+0x6C is 0060D0A0. The others' POST rows differ from SAMPLE by what InitItem does, in 12d.
fn family_c_rows(world: &World, book: &RowBook) {
    const TYPES: [&str; 10] = ["2D", "2E", "2F", "3C", "3D", "3E", "3F", "41", "42", "43"];
    let forms = world.resource::<FormsById>();
    for form_type in TYPES {
        let row = book.take_one("COUNT", form_type);
        let code = hex(form_type) as u8;
        let mut ids: Vec<u32> = forms
            .entries()
            .filter(|(_, form)| world.get::<FormIdentity>(*form).unwrap().form_type_04 == code)
            .map(|(id, _)| id)
            .collect();
        ids.sort_unstable();
        assert_eq!(
            ids.len(),
            row[2].parse::<usize>().unwrap(),
            "COUNT {form_type}"
        );
        assert_eq!(ids_fnv(&ids), hex(row[3]), "COUNT {form_type} fnv");
        assert_eq!(ids[0], hex(row[4]), "COUNT {form_type} first");
        assert_eq!(*ids.last().unwrap(), hex(row[5]), "COUNT {form_type} last");
    }
    let lists = world.resource::<DataHandlerLists>();
    for offset in ["04", "14", "1C", "94", "9C", "A4", "AC", "B4"] {
        let row = book.take_one("LIST", offset);
        let ids: Vec<u32> = lists
            .list_at(hex(offset))
            .unwrap()
            .items()
            .map(|form| world.get::<FormIdentity>(form).unwrap().form_id_0c)
            .collect();
        assert_eq!(ids.len(), row[2].parse::<usize>().unwrap(), "LIST {offset}");
        assert_eq!(
            ids_fnv(&ids),
            hex(row[3]),
            "LIST {offset} fnv: the original order"
        );
        assert_eq!(ids[..3], hex_ids(row[4])[..], "LIST {offset} head");
        assert_eq!(*ids.last().unwrap(), hex(row[5]), "LIST {offset} last");
    }
    // AC20 and AC20RAW: the default forms before any record.
    let mut before = Loader::new(&shipped_root());
    let image_world = &mut before.world;
    pre_load_default_globals(image_world);
    for form_type in ["2D", "2E", "42", "43"] {
        let created = hex_ids(book.take_one("AC20", form_type)[2]);
        let mut seen = Vec::new();
        for row in book.take("AC20RAW", form_type) {
            seen.push(hex(row[2]));
            compare_sample(image_world, book, &row, false);
        }
        assert_eq!(seen, created, "AC20 {form_type}: the creation order");
    }
    let mut samples = 0;
    for form_type in TYPES {
        for row in book.take("SAMPLE", form_type) {
            compare_sample(world, book, &row, true);
            samples += 1;
        }
        if matches!(form_type, "2D" | "2E") {
            for row in book.take("POST", form_type) {
                compare_sample(world, book, &row, false);
                samples += 1;
            }
        }
        let data_ids: Vec<String> = book
            .lines
            .iter()
            .filter(|line| line.starts_with(&format!("DATA {form_type} ")))
            .map(|line| line.split(' ').nth(2).unwrap().to_owned())
            .collect();
        for id in data_ids {
            for offset in book.take_other_data(form_type, &id) {
                println!("DATA {form_type} {id} {offset:X}: no pointer here, not compared");
            }
        }
    }
    println!("family C samples and post rows compared: {samples}");
    region_data_lists(world);
}

// IDA: N/A the dialogue family's rows: COUNT, LIST 7C and 84, AC20 and AC20RAW for the default topics, SAMPLE and DATA. POST rows wait for each class's InitItem, 12d.
fn dialogue_rows(world: &World, book: &RowBook) {
    const TYPES: [&str; 3] = ["39", "3A", "3B"];
    let forms = world.resource::<FormsById>();
    for form_type in TYPES {
        let row = book.take_one("COUNT", form_type);
        let code = hex(form_type) as u8;
        let mut ids: Vec<u32> = forms
            .entries()
            .filter(|(_, form)| world.get::<FormIdentity>(*form).unwrap().form_type_04 == code)
            .map(|(id, _)| id)
            .collect();
        ids.sort_unstable();
        assert_eq!(
            ids.len(),
            row[2].parse::<usize>().unwrap(),
            "COUNT {form_type}"
        );
        assert_eq!(ids_fnv(&ids), hex(row[3]), "COUNT {form_type} fnv");
        assert_eq!(ids[0], hex(row[4]), "COUNT {form_type} first");
        assert_eq!(*ids.last().unwrap(), hex(row[5]), "COUNT {form_type} last");
    }
    let lists = world.resource::<DataHandlerLists>();
    for offset in ["7C", "84"] {
        let row = book.take_one("LIST", offset);
        let ids: Vec<u32> = lists
            .list_at(hex(offset))
            .unwrap()
            .items()
            .map(|form| world.get::<FormIdentity>(form).unwrap().form_id_0c)
            .collect();
        assert_eq!(ids.len(), row[2].parse::<usize>().unwrap(), "LIST {offset}");
        assert_eq!(
            ids_fnv(&ids),
            hex(row[3]),
            "LIST {offset} fnv: the original order"
        );
        assert_eq!(ids[..3], hex_ids(row[4])[..], "LIST {offset} head");
        assert_eq!(*ids.last().unwrap(), hex(row[5]), "LIST {offset} last");
    }
    // AC20 and AC20RAW: the default topics before any record.
    let mut before = Loader::new(&shipped_root());
    pre_load_default_globals(&mut before.world);
    let created = hex_ids(book.take_one("AC20", "39")[2]);
    let mut seen = Vec::new();
    for row in book.take("AC20RAW", "39") {
        seen.push(hex(row[2]));
        compare_sample(&before.world, book, &row, false);
    }
    assert_eq!(seen, created, "AC20 39: the rows' order");
    let mut samples = 0;
    for form_type in TYPES {
        for row in book.take("SAMPLE", form_type) {
            compare_sample(world, book, &row, true);
            samples += 1;
        }
        let data_ids: Vec<String> = book
            .lines
            .iter()
            .filter(|line| line.starts_with(&format!("DATA {form_type} ")))
            .map(|line| line.split(' ').nth(2).unwrap().to_owned())
            .collect();
        for id in data_ids {
            for offset in book.take_other_data(form_type, &id) {
                println!("DATA {form_type} {id} {offset:X}: no pointer here, not compared");
            }
        }
    }
    println!("dialogue samples compared: {samples}");
}

// IDA: N/A the region data lists, now the GRAS, LTEX and SOUN forms exist. No row reaches them: the DATA rows stop at each region's list head. These counts come from a scan of the shipped records, the last record of each region winning: one RDGS (0x9CA) with four grasses and parent IDs of 0, and 126 nonempty RDSD lists holding 2395 entries, every raw ID a SOUN. A count from the file isn't a run value, and the entries' order and flags stay unchecked.
fn region_data_lists(world: &World) {
    let forms = world.resource::<FormsById>();
    let (mut regions, mut grasses, mut lands, mut sound_lists, mut sounds) = (0, 0, 0, 0, 0);
    for (_, form) in forms.entries() {
        let Some(region) = world.get::<TesRegion>(form) else {
            continue;
        };
        regions += 1;
        for &data in &region.data_18.items {
            match &world.get::<TesRegionData>(data).unwrap().kind {
                RegionDataKind::Grass {
                    grasses_04,
                    count_10,
                    ..
                } => {
                    assert_eq!(grasses_04.len(), *count_10 as usize);
                    for grass in grasses_04 {
                        assert_eq!(
                            world
                                .get::<FormIdentity>(grass.grass_04)
                                .unwrap()
                                .form_type_04,
                            0x1D
                        );
                        lands += usize::from(grass.land_texture_08.is_some());
                    }
                    grasses += grasses_04.len();
                }
                RegionDataKind::Sound { sounds_0c, .. } => {
                    sound_lists += usize::from(!sounds_0c.is_empty());
                    sounds += sounds_0c.len();
                }
                _ => {}
            }
        }
    }
    println!(
        "regions {regions}: grass objects {grasses} with land textures {lands}, sound lists {sound_lists} with entries {sounds}"
    );
    assert_eq!(regions, 212);
    assert_eq!((grasses, lands), (4, 0), "the four RDGS entries of 0x9CA");
    assert_eq!((sound_lists, sounds), (126, 2395));
}

/// Resuming at record boundaries changes nothing the counters or lists can see.
#[test]
fn a_step_at_a_time_gives_the_same_load() {
    let mut straight = shipped_loader();
    assert_eq!(straight.run(), Ok(1));
    let mut stepped = shipped_loader();
    let (result, steps) = stepped.run_stepped().unwrap();
    assert_eq!(result, 1);
    let (a, b) = (
        straight.world.resource::<DataLoadGlobals>(),
        stepped.world.resource::<DataLoadGlobals>(),
    );
    assert_eq!(a.record_count_b33a94, b.record_count_b33a94);
    assert_eq!(a.file_entry_counts, b.file_entry_counts);
    assert_eq!(a.pending_arms, b.pending_arms);
    assert!(
        steps > u64::from(a.record_count_b33a94),
        "a step is at most one record: {steps}"
    );
    let ids = |world: &World| -> Vec<u32> {
        world
            .resource::<DataHandlerLists>()
            .classes_54
            .items()
            .map(|form| world.get::<FormIdentity>(form).unwrap().form_id_0c)
            .collect()
    };
    assert_eq!(ids(&straight.world), ids(&stepped.world));
    assert_eq!(straight.host.progress, stepped.host.progress);
}

/// The run time the plan asks for: every arm but the frame's skipped, on this profile. Run it alone with `--ignored --nocapture`.
#[test]
#[ignore = "measurement only"]
fn measure_the_file_pass_with_every_arm_skipped() {
    let mut loader = shipped_loader();
    loader.host.real_pre_load = false;
    loader.host.real_tail = false;
    loader.host.arm_filter = |_| false;
    let start = Instant::now();
    assert_eq!(loader.run(), Ok(1));
    let elapsed = start.elapsed();
    let load = loader.world.resource::<DataLoadGlobals>();
    println!(
        "file pass, every arm skipped: {:.3} s, {} records visited, {} skipped arms",
        elapsed.as_secs_f64(),
        load.record_count_b33a94,
        load.pending_arms.values().sum::<u32>()
    );
}

// ---- synthetic plugins ----

fn u16_bytes(value: u16) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

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
    cat(&[kind, &u16_bytes(data.len() as u16), data])
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

fn compressed_record(kind: &[u8; 4], flags: u32, id: u32, payload: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(payload).unwrap();
    let packed = encoder.finish().unwrap();
    record(
        kind,
        flags | 0x40000,
        id,
        &cat(&[&u32_bytes(payload.len() as u32), &packed]),
    )
}

fn group(label: &[u8; 4], group_type: u32, contents: &[u8]) -> Vec<u8> {
    cat(&[
        b"GRUP",
        &u32_bytes(contents.len() as u32 + 20),
        label,
        &u32_bytes(group_type),
        &u32_bytes(0),
        contents,
    ])
}

fn tes4(master: bool, version: f32, count: u32, masters: &[(&str, u64)]) -> Vec<u8> {
    let mut payload = chunk(
        b"HEDR",
        &cat(&[&f32_bytes(version), &u32_bytes(count), &u32_bytes(0x800)]),
    );
    for (name, size) in masters {
        payload.extend(chunk(b"MAST", &text(name)));
        payload.extend(chunk(b"DATA", &size.to_le_bytes()));
    }
    record(b"TES4", u32::from(master), 0, &payload)
}

/// IDA: N/A a temp working directory with `Data\` plugins.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("recomp_phase12_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Data")).unwrap();
        Self { root }
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.root.join("Data").join(name), bytes).unwrap();
    }

    fn loader(&self) -> Loader {
        let mut loader = Loader::new(&self.root);
        loader.load_all();
        loader
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn setting_named(world: &World, kind: u8) -> &'static str {
    let settings = world.resource::<GameSettingCollection>();
    recomp_game::tes::game_settings_table::GAME_SETTING_DEFINITIONS
        .iter()
        .find_map(|definition| {
            let name = definition.name?;
            (definition.kind == kind && name.as_bytes()[0] == kind && settings.find(name).is_some())
                .then_some(name)
        })
        .unwrap()
}

#[test]
fn a_missing_master_returns_0_with_the_tls_byte_clear() {
    let fixture = Fixture::new("missing");
    fixture.write("Plugin.esp", &tes4(false, 0.8, 0, &[("Missing.esm", 0)]));
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(0));
    assert_eq!(loader.world.resource::<GameThreadLocals>().byte_184, 0);
    assert_eq!(loader.host.logs, ["Unable to find masterfile: Missing.esm"]);
    assert_eq!(loader.host.pre_load_calls, 0, "0044AC20 isn't reached");
    // 0044F521's list walk never ran: nothing was compiled, and +0x8C0 keeps its value.
    let handler = loader.world.resource::<DataHandler>();
    assert_eq!(handler.loaded_mod_count_8d0, 0);
    assert_eq!(handler.next_form_id_8c0(), 0x800);
    assert!(
        loader
            .warnings
            .iter()
            .any(|text| text == "[disabled] Missing Masterfile: Missing.esm"),
        "{:?}",
        loader.warnings
    );
}

#[test]
fn a_newer_file_is_dropped_and_a_changed_master_warns() {
    let fixture = Fixture::new("newer");
    fixture.write("Base.esm", &tes4(true, 1.0, 1, &[]));
    fixture.write("Newer.esp", &tes4(false, 1.5, 0, &[]));
    fixture.write("Changed.esp", &tes4(false, 0.8, 0, &[("Base.esm", 12345)]));
    // 0x7FC00000: exactly the arm the C++ gets wrong. 004504D3 is `jp`, so a NaN version isn't newer.
    fixture.write("Nan.esp", &tes4(false, f32::NAN, 0, &[]));
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    assert!(!loader.file("Newer.esp").is_loaded(), "Newer.esp unloaded");
    assert!(loader.file("Nan.esp").is_loaded(), "a NaN version stays");
    let has = |want: &str| loader.warnings.iter().any(|text| text == want);
    assert!(
        has("File Newer.esp is a higher version than this EXE can load."),
        "the flag cleared around it: {:?}",
        loader.warnings
    );
    assert!(
        has(
            "[disabled] One of the files that \"Changed.esp\" is dependent on has changed since the last save."
        ),
        "{:?}",
        loader.warnings
    );
    assert!(
        has(
            "[disabled] One or more plugins could not find the correct versions of the master files they depend on. Errors may occur during load or game play. Check the \"Warnings.txt\" file for more information.\r\n"
        ),
        "{:?}",
        loader.warnings
    );
    assert_eq!(
        loader
            .world
            .resource::<DataLoadGlobals>()
            .disable_warning_messages_b06b18,
        1,
        "restored"
    );
    // 0044FDE0 finds Base.esm relative to the working directory, not Data\: not found, so 12345 is compared with 0. A plugin with a size of 0 raises nothing, as every shipped plugin does.
    let handler = loader.world.resource::<DataHandler>();
    assert_eq!(handler.next_form_id_8c0(), 0xFF00_0800);
}

#[test]
fn records_through_the_frame() {
    let fixture = Fixture::new("records");
    let probe = fixture.loader();
    let float_setting = setting_named(&probe.world, b'f');
    let string_setting = setting_named(&probe.world, b's');
    drop(probe);
    let mut esm = tes4(true, 1.0, 6, &[]);
    let gmst = cat(&[
        &compressed_record(
            b"GMST",
            0,
            0x900,
            &cat(&[
                &chunk(b"EDID", &text(float_setting)),
                &chunk(b"DATA", &f32_bytes(12.5)),
            ]),
        ),
        &record(
            b"GMST",
            0,
            0x901,
            &cat(&[
                &chunk(b"EDID", &text(string_setting)),
                &chunk(b"DATA", &text("Fixture text")),
            ]),
        ),
    ]);
    esm.extend(group(b"GMST", 0, &gmst));
    // SKIL 0x3D with a DATA chunk.
    esm.extend(group(
        b"SKIL",
        0,
        &record(
            b"SKIL",
            0,
            0x3D,
            &cat(&[
                &chunk(b"EDID", &text("Armorer")),
                &chunk(
                    b"DATA",
                    &cat(&[
                        &u32_bytes(0x0C),
                        &u32_bytes(5),
                        &u32_bytes(0),
                        &f32_bytes(1.5),
                        &f32_bytes(1.0),
                    ]),
                ),
                &chunk(b"DESC", b"x"),
            ]),
        ),
    ));
    // A master's exterior block group (type 4) is skipped whole: its GLOB never reaches the arm.
    esm.extend(group(
        &[0; 4],
        4,
        &record(b"GLOB", 0, 0x950, &chunk(b"EDID", &text("Skip"))),
    ));
    // TOFT -1 ends the file.
    esm.extend(record(b"TOFT", 0, 0xFFFF_FFFF, &[]));
    esm.extend(group(b"GLOB", 0, &record(b"GLOB", 0, 0x951, &[])));
    fixture.write("Base.esm", &esm);
    // The second file: a wrong-type record (a GLOB with the skill ID 0x3D) and a partial form with no base.
    let mut esp = tes4(false, 0.8, 2, &[("Base.esm", 0)]);
    esp.extend(group(
        b"GLOB",
        0,
        &cat(&[
            &record(b"GLOB", 0, 0x3D, &[]),
            &record(b"GLOB", 0x4000, 0x12_3456, &[]),
        ]),
    ));
    fixture.write("Plugin.esp", &esp);
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    let world = &loader.world;
    let settings = world.resource::<GameSettingCollection>();
    let loaded = settings.find(float_setting).unwrap();
    assert_eq!(f32::from_bits(loaded.raw), 12.5, "a compressed GMST");
    assert_eq!(loaded.loads, 1);
    let loaded = settings.find(string_setting).unwrap();
    assert_eq!(loaded.text.as_deref(), Some(&b"Fixture text"[..]));
    assert_eq!(
        loaded.heap_name.as_ref().unwrap()[0],
        b'S',
        "the key moved to an 'S' block"
    );
    let skill = world
        .get::<TesSkill>(world.resource::<DataHandler>().skills_d8[0])
        .unwrap();
    assert_eq!(
        (skill.skill_2c, skill.field_30, skill.float_38),
        (0x0C, 5, 1.5)
    );
    assert_ne!(skill.description_18.offset_04, 0, "the SKIL DESC offset");
    // Neither the skipped group's GLOB nor the GLOB after the TOFT end was built, and the two GLOBs of the second file stopped in the frame.
    assert_eq!(
        world
            .resource::<DataHandlerLists>()
            .globals_74
            .items()
            .count(),
        0
    );
    let has = |want: &str| loader.warnings.iter().any(|text| text == want);
    assert!(
        has(
            "[disabled] Form (0000003D) in file \"Plugin.esp\" has wrong type.\n\nShould be:\tSKIL\nIs:\t\tGLOB"
        ),
        "{:?}",
        loader.warnings
    );
    assert!(
        has("[disabled] Partial Form (00123456) in file \"Plugin.esp\" could not find base form."),
        "{:?}",
        loader.warnings
    );
    assert!(
        has(
            "[disabled] DataHandler: unrecognized form\r\nLook in the <MyGames>Warnings.txt file for more info.\r\n"
        ),
        "{:?}",
        loader.warnings
    );
    let load = world.resource::<DataLoadGlobals>();
    assert_eq!(load.file_results, [1, 0]);
    // TES4, GRUP, GMST, GMST, GRUP, SKIL, GRUP(4), TOFT = 8 in the master; TES4, GRUP, GLOB, GLOB = 4 in the plugin.
    assert_eq!(load.file_entry_counts[1], 8);
    assert_eq!(load.record_count_b33a94, 12);
    // 00450080's master arm: Plugin.esp's top byte 0 names Base.esm, whose index is 0, so 0x123456's top byte stays 0.
    assert_eq!(loader.file("Plugin.esp").mod_index_400, 1);
    assert!(loader.warnings.iter().all(|text| !text.contains("Unknown")));
}

#[test]
fn the_compressed_record_arms() {
    let fixture = Fixture::new("compressed");
    // A stream that isn't zlib (-3), a stream cut short (0, "did not terminate"), an inflated length past the stream's end (still Z_STREAM_END with room left), and a skipped ignored record.
    let mut esm = tes4(true, 1.0, 5, &[]);
    let mut bad = u32_bytes(32);
    bad.extend([0xFF; 16]);
    esm.extend(record(b"GMST", 0x40000, 0x901, &bad));
    let whole = compressed_record(b"GMST", 0, 0x902, &chunk(b"EDID", &text("sNothing")));
    let mut cut = whole.clone();
    let cut_at = cut.len() - 6;
    cut.truncate(cut_at);
    let new_size = (cut.len() - 0x14) as u32;
    cut[4..8].copy_from_slice(&new_size.to_le_bytes());
    esm.extend(cut);
    esm.extend(compressed_record(
        b"GMST",
        0,
        0x903,
        &chunk(b"EDID", &text("sNothing")),
    ));
    esm.extend(record(b"GMST", 0x1000, 0x904, &[]));
    esm.extend(record(
        b"GMST",
        0,
        0x905,
        &chunk(b"EDID", &text("sNothing")),
    ));
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    // 00450F7A retries 00450550 while +0x414 is null, reading on from where the file stands, so one bad record warns more than once, as in the original. The header pass at 0044A2B0 reaches the first bad record too, with the flag clear.
    let count = |want: &str| loader.warnings.iter().filter(|text| *text == want).count();
    assert!(
        count("[disabled] TESFile: Error inflating ZLib stream.") >= 1,
        "{:?}",
        loader.warnings
    );
    assert!(
        count("TESFile: Error inflating ZLib stream.") >= 1,
        "{:?}",
        loader.warnings
    );
    assert!(
        count("[disabled] TESFile: Error: ZLib stream did not terminate.") >= 1,
        "{:?}",
        loader.warnings
    );
    // TES4, three compressed GMSTs and the last; the ignored one isn't counted.
    assert_eq!(
        loader
            .world
            .resource::<DataLoadGlobals>()
            .record_count_b33a94,
        5
    );
}

#[test]
fn a_master_arm_maps_form_ids_and_an_absent_arm_counts() {
    let fixture = Fixture::new("masters");
    let mut esm = tes4(true, 1.0, 2, &[]);
    esm.extend(group(b"STAT", 0, &record(b"STAT", 0, 0x77, &[])));
    fixture.write("Base.esm", &esm);
    let mut esp = tes4(false, 0.8, 4, &[("Base.esm", 0)]);
    // Top byte 0 is the master, top byte 1 is this file, and a CLAS with the file's own index.
    esp.extend(group(
        b"CLAS",
        0,
        &cat(&[
            &record(
                b"CLAS",
                0,
                0x0100_0ABC,
                &cat(&[&chunk(b"EDID", &text("Own")), &chunk(b"DATA", &[0; 0x34])]),
            ),
            &record(b"CLAS", 0, 0x0000_0DEF, &chunk(b"FULL", &text("Mastered"))),
        ]),
    ));
    fixture.write("Plugin.esp", &esp);
    let mut loader = fixture.loader();
    // STAT has its class now: the filter keeps it absent for this test.
    loader.host.arm_filter = |form_type| form_type != 0x1C;
    assert_eq!(loader.run(), Ok(1));
    let world = &loader.world;
    let forms = world.resource::<FormsById>();
    assert!(forms.lookup(0x0100_0ABC).is_some(), "the file's own index");
    assert!(forms.lookup(0x0000_0DEF).is_some(), "the master's index");
    // The all-zero DATA has equal attributes: the invalid-class warning, with the empty editor ID. The second CLAS has the default attributes and skills, which are valid.
    assert!(loader.warnings.iter().any(|text| text == "[disabled] Class  contains invalid data. Make sure all attributes and skills are unique."), "{:?}", loader.warnings);
    assert_eq!(
        loader
            .warnings
            .iter()
            .filter(|text| text.contains("invalid data"))
            .count(),
        1
    );
    let load = world.resource::<DataLoadGlobals>();
    assert_eq!(load.pending_arms.get(&0x1C), Some(&1), "the STAT counted");
}

#[test]
fn an_unknown_record_warns_in_the_default_arm() {
    let fixture = Fixture::new("unknown");
    let mut esm = tes4(true, 1.0, 2, &[]);
    esm.extend(record(b"ZZZZ", 0, 0x901, &[]));
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    let has = |want: &str| loader.warnings.iter().any(|text| text == want);
    assert!(
        has("[disabled] TESDataHandler trying to create TESForm for unknown type 'NONE'."),
        "{:?}",
        loader.warnings
    );
    assert!(
        has("[disabled] Unknown ZZZZ_ID in ConstructObject."),
        "{:?}",
        loader.warnings
    );
    assert_eq!(loader.world.resource::<DataLoadGlobals>().file_results, [0]);
    assert_eq!(
        loader
            .world
            .resource::<DataLoadGlobals>()
            .disable_warning_messages_b06b18,
        1
    );
}

#[test]
fn an_ordinary_group_is_entered_in_a_plugin_and_a_type_4_group_too() {
    let fixture = Fixture::new("groups");
    let mut esp = tes4(false, 0.8, 3, &[]);
    // 00448C60: a non-master with no save-load flag enters every group.
    esp.extend(group(
        &[0; 4],
        4,
        &record(b"GLOB", 0, 0x0000_0950, &chunk(b"EDID", &text("Entered"))),
    ));
    fixture.write("Plugin.esp", &esp);
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    let world = &loader.world;
    assert!(
        world.resource::<EditorIdMap>().lookup(b"entered").is_some(),
        "the editor-ID map, folded"
    );
    assert_eq!(
        world
            .resource::<DataHandlerLists>()
            .globals_74
            .items()
            .count(),
        1
    );
    assert_eq!(world.resource::<DataLoadGlobals>().record_count_b33a94, 3);
    let form = world
        .resource::<DataHandlerLists>()
        .globals_74
        .items()
        .next()
        .unwrap();
    assert_eq!(world.get::<TESGlobal>(form).unwrap().variable_type_20, b's');
}

#[test]
fn a_tes4_dele_chunk_goes_to_3f8() {
    let fixture = Fixture::new("dele");
    let hedr = chunk(
        b"HEDR",
        &cat(&[&f32_bytes(1.0), &u32_bytes(1), &u32_bytes(0x800)]),
    );
    let dele = chunk(
        b"DELE",
        &cat(&[&u32_bytes(0x1122_3344), &u32_bytes(0x5566_7788)]),
    );
    fixture.write("Base.esm", &record(b"TES4", 1, 0, &cat(&[&hedr, &dele])));
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    let file = loader.file("Base.esm");
    assert_eq!((file.field_3f8, file.field_3fc), (0x1122_3344, 0x5566_7788));
}

/// 0044DD48: the first file looks up reserved IDs only, so a repeated ID builds a second form there, and a later file reloads the one the map holds, through the reset pair.
#[test]
fn the_first_file_looks_up_only_reserved_ids() {
    let fixture = Fixture::new("first");
    let global = |id: u32, name: &str, value: f32| {
        record(
            b"GLOB",
            0,
            id,
            &cat(&[
                &chunk(b"EDID", &text(name)),
                &chunk(b"FLTV", &f32_bytes(value)),
            ]),
        )
    };
    let mut esm = tes4(true, 1.0, 3, &[]);
    esm.extend(group(
        b"GLOB",
        0,
        &cat(&[&global(0x901, "One", 1.0), &global(0x901, "Two", 2.0)]),
    ));
    fixture.write("Base.esm", &esm);
    let mut esp = tes4(false, 0.8, 2, &[("Base.esm", 0)]);
    esp.extend(group(b"GLOB", 0, &global(0x901, "Reloaded", 3.0)));
    fixture.write("Plugin.esp", &esp);
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    let world = &loader.world;
    assert_eq!(
        world
            .resource::<DataHandlerLists>()
            .globals_74
            .items()
            .count(),
        2,
        "two forms, and no third"
    );
    let held = world.resource::<FormsById>().lookup(0x901).unwrap();
    let global = world.get::<TESGlobal>(held).unwrap();
    assert_eq!(
        f32::from_bits(global.value_bits_24),
        3.0,
        "the map's form is the second, reloaded"
    );
    let map = world.resource::<EditorIdMap>();
    assert_eq!(map.lookup(b"reloaded"), Some(held));
    assert_eq!(
        map.lookup(b"two"),
        Some(held),
        "the older key stays in the map"
    );
    assert!(map.lookup(b"one").is_some());
    // A master flag set by the first load stays through 00447057, though Plugin.esp isn't a master.
    let identity = world.get::<FormIdentity>(held).unwrap();
    assert_eq!(identity.flags_08 & 1, 1);
}

#[test]
fn the_toft_arms() {
    let fixture = Fixture::new("toft");
    let mut esm = tes4(true, 1.0, 4, &[]);
    // An offset with no current form does nothing; -2 reaches 00738500, which only returns, and ends the file.
    esm.extend(record(b"TOFT", 0, 0x1234, &[]));
    esm.extend(record(b"TOFT", 0, 0xFFFF_FFFE, &[]));
    esm.extend(group(b"GLOB", 0, &record(b"GLOB", 0, 0x951, &[])));
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    assert_eq!(
        loader
            .world
            .resource::<DataHandlerLists>()
            .globals_74
            .items()
            .count(),
        0
    );
    assert_eq!(
        loader
            .world
            .resource::<DataLoadGlobals>()
            .record_count_b33a94,
        3
    );
}

#[test]
fn an_absent_arm_stops_by_default() {
    use recomp_game::tes::data_load::{DataLoadServices, load_selected_files};
    use recomp_game::tes::form_arms::DataLoadHost;
    struct StopHost;
    impl DataLoadHost for StopHost {
        fn run_pre_load_pass(&mut self, _world: &mut World) -> bool {
            false
        }
        // CELL has a class now: the host's filter makes it absent.
        fn arm_enabled(&mut self, form_type: u32) -> bool {
            form_type != 0x30
        }
    }
    let fixture = Fixture::new("stop");
    let mut esm = tes4(true, 1.0, 2, &[]);
    esm.extend(record(b"CELL", 0, 0x901, &[]));
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    let error = load_selected_files(
        &mut loader.world,
        false,
        false,
        &mut DataLoadServices {
            working_directory: &fixture.root,
            warning: &mut |_, _| {},
            host: &mut StopHost,
        },
    )
    .unwrap_err();
    assert_eq!(error.0, "Phase-12 pending: 0044DE2E arm of form type CELL");
    // The store went back on the way out.
    assert_eq!(
        loader
            .world
            .resource::<DataHandler>()
            .file_store
            .iter()
            .count(),
        1
    );
}

#[test]
fn a_chunk_longer_than_its_limit_warns_with_the_original_text() {
    let fixture = Fixture::new("truncate");
    let mut esm = tes4(true, 1.0, 2, &[]);
    // A float GMST whose DATA is 6 bytes: 004510E0 reads 3 of them, with the fourth 0.
    esm.extend(record(b"GMST", 0, 0x901, &[]));
    fixture.write("Base.esm", &esm);
    let probe = fixture.loader();
    let name = setting_named(&probe.world, b'f');
    drop(probe);
    let mut esm = tes4(true, 1.0, 2, &[]);
    esm.extend(record(
        b"GMST",
        0,
        0x901,
        &cat(&[&chunk(b"EDID", &text(name)), &chunk(b"DATA", b"abc\0ef")]),
    ));
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    assert!(
        loader.warnings.iter().any(|text| text == "[disabled] Chunk size 6 too big in chunk DATA_ID in form GMST_ID.\r\nMax size is 4, data truncated to \"abc\".\r\n"),
        "{:?}",
        loader.warnings
    );
    let raw = loader
        .world
        .resource::<GameSettingCollection>()
        .find(name)
        .unwrap()
        .raw;
    assert_eq!(raw.to_le_bytes(), [b'a', b'b', b'c', 0]);
}

#[test]
fn the_editor_id_map_hash_and_chain_order() {
    let mut map = EditorIdMap::default();
    let mut world = World::new();
    let (first, second, third) = (
        world.spawn_empty().id(),
        world.spawn_empty().id(),
        world.spawn_empty().id(),
    );
    assert_eq!(EditorIdMap::hash(b""), 0);
    // 004A80A0: 'a' = 0x61; "ab" = 33 * 0x61 + 0x62.
    assert_eq!(EditorIdMap::hash(b"AB"), 33 * 0x61 + 0x62);
    // A byte above 0x7F hashes as a signed char: 33 * 0x61 - 0x80 here.
    assert_eq!(
        EditorIdMap::hash(b"a\x80"),
        (33_u32 * 0x61).wrapping_sub(0x80) % 0xFA1
    );
    map.insert_or_assign(b"TestKey", first);
    map.insert_or_assign(b"testkey", second);
    assert_eq!(map.count(), 1, "a case-insensitive hit replaces");
    let bucket = EditorIdMap::hash(b"TestKey");
    assert_eq!(
        map.bucket(bucket)[0].key,
        b"testkey",
        "the key is replaced too (+0x10 is 0)"
    );
    assert_eq!(map.lookup(b"TESTKEY"), Some(second));
    // Find a key in the same bucket: a miss goes to the head.
    let other = (0..u32::MAX)
        .map(|index| format!("k{index}").into_bytes())
        .find(|key| EditorIdMap::hash(key) == bucket && key != b"testkey")
        .unwrap();
    map.insert_or_assign(&other, third);
    assert_eq!(map.bucket(bucket)[0].form, third);
    assert_eq!(map.bucket(bucket)[1].form, second);
}

// run-c1-log's three kinds: LIFO over the blocks of the key's size freed after it.
#[test]
fn a_cell_override_reads_the_old_key_blocks_contents() {
    use recomp_game::tes::data_handler::FreedCellKey;
    let mut world = World::new();
    let cell = world.spawn_empty().id();
    let insert = |map: &mut EditorIdMap, key: &[u8], full: &[u8], freed_after: Vec<u32>| {
        map.note_freed_cell_key(FreedCellKey {
            form: cell,
            key: key.to_vec(),
            freed_after,
        });
        map.insert_after_cell_override(key, full, cell);
    };
    // Nothing of its size covers it: the name takes its block back.
    let mut map = EditorIdMap::default();
    map.insert_or_assign(b"BravilUngolimHouse", cell);
    insert(&mut map, b"BravilUngolimHouse", b"House", vec![0x14, 6]);
    assert_eq!(map.count(), 1);
    // FULL's old block is on top, so the name takes that and FULL takes the key's, with the same text: a hit.
    let mut map = EditorIdMap::default();
    map.insert_or_assign(b"Nagastani", cell);
    insert(&mut map, b"Nagastani", b"Nagastani", vec![0x14, 10]);
    assert_eq!(map.count(), 1);
    // With other text in the block, a miss.
    let mut map = EditorIdMap::default();
    map.insert_or_assign(b"Nagastani", cell);
    insert(&mut map, b"Nagastani", b"Nagastanis", vec![0x14, 10]);
    assert_eq!(map.count(), 2);
    assert_eq!(
        map.bucket(EditorIdMap::hash(b"Nagastani"))[1].key,
        b"Nagastanis"
    );
    // Two blocks on top, and FULL is another size: the key's stays free, a miss.
    let mut map = EditorIdMap::default();
    map.insert_or_assign(b"CursedMine03", cell);
    insert(
        &mut map,
        b"CursedMine03",
        b"Cursed Mine",
        vec![0x14, 0x10, 0x10],
    );
    assert_eq!(map.count(), 2);
    assert_eq!(map.bucket(EditorIdMap::hash(b"CursedMine03"))[1].key, b"");
    // The 0x14 entry takes the block on top first when the name is that size too.
    let mut map = EditorIdMap::default();
    map.insert_or_assign(b"WarehouseWeapons", cell);
    insert(&mut map, b"WarehouseWeapons", b"Weapons", vec![0x14]);
    assert_eq!(map.count(), 1);
}

#[test]
fn the_setting_type_table() {
    let cases: BTreeMap<u8, u32> = BTreeMap::from([
        (b'S', 6),
        (b's', 6),
        (b'a', 8),
        (b'b', 0),
        (b'c', 1),
        (b'f', 5),
        (b'h', 2),
        (b'i', 3),
        (b'r', 7),
        (b'u', 4),
        (b'x', 9),
        (b'R', 9),
    ]);
    for (byte, kind) in cases {
        assert_eq!(setting_type(Some(&[byte])), kind, "{}", char::from(byte));
    }
    assert_eq!(setting_type(None), 9);
}

#[test]
fn the_percent_is_float_at_each_step() {
    assert_eq!(record_percent(0x31E6, 0x137D97), 1);
    assert_eq!(record_percent(0x1C078, 0x137D97), 8);
    assert_eq!(
        record_percent(1, 0),
        i32::MIN,
        "a total of 0: +inf is out of range for 009828C0"
    );
    assert_eq!(record_percent(0, 0), i32::MIN, "and 0/0 a NaN");
}

/// The quality tail of 0046BDA0 truncates to a byte, where the C++ copies the bits.
#[test]
fn the_data_chunk_tails_in_cast_order() {
    let fixture = Fixture::new("tails");
    let payload = cat(&[
        &[1, 2],
        &[7],
        &u32_bytes(1000),
        &u32_bytes(30),
        &f32_bytes(2.5),
        &f32_bytes(300.75),
        &u16_bytes(9),
        &[1, 2, 3, 4, 5, 6, 7, 8],
    ]);
    let mut esm = tes4(true, 1.0, 2, &[]);
    esm.extend(record(b"ZZZZ", 0, 0x901, &chunk(b"DATA", &payload)));
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    let mut world = World::new();
    world.init_resource::<GameThreadLocals>();
    let handler_id = loader.world.resource::<DataHandler>().files_8c8[0];
    let mut data = loader.world.resource_mut::<DataHandler>();
    let mut cache = recomp_game::tes::tes_file::FormTypeLookupCache::default();
    let mut warning = |_: String, _: bool| {};
    let mut services = recomp_game::tes::tes_file::TesFileServices {
        working_directory: &fixture.root,
        warning: &mut warning,
        disable_warning_messages_b06b18: 0,
        close_suppressed: true,
        form_type_cache: &mut cache,
    };
    let (directory, name) = {
        let file = data.file_store.get_mut(handler_id).unwrap();
        (file.directory_120.clone(), file.name_1c.clone())
    };
    recomp_game::tes::tes_file::open_tes_file(
        &mut data.file_store,
        handler_id,
        &directory,
        &name,
        0,
        false,
        &mut services,
    )
    .unwrap();
    recomp_game::tes::tes_file::skip_record(&mut data.file_store, handler_id, true, &mut services)
        .unwrap();
    struct NoHost;
    impl recomp_game::tes::form_arms::DataLoadHost for NoHost {}
    let mut cx = recomp_game::tes::form_arms::FormLoadCx {
        file: recomp_game::tes::tes_file::RecordReader {
            files: &mut data.file_store,
            id: handler_id,
            services: &mut services,
        },
        host: &mut NoHost,
    };
    let mut destination = [0_u8; 2];
    let (mut uses, mut value, mut health, mut weight, mut quality, mut damage, mut attributes) =
        (0_u8, 0, 0, 0.0, 0.0, 0, [0; 8]);
    load_data_chunk(
        &mut destination,
        2,
        DataChunkTails {
            uses: Some(&mut uses),
            value: Some(&mut value),
            value_change: None,
            health: Some(&mut health),
            weight: Some(&mut weight),
            quality: Some(&mut quality),
            attack_damage: Some(&mut damage),
            attributes: Some(&mut attributes),
        },
        &mut cx,
    )
    .unwrap();
    assert_eq!(destination, [1, 2]);
    assert_eq!((uses, value, health, weight), (7, 1000, 30, 2.5));
    assert_eq!(quality, 44.0, "300.75 chops to 300, whose low byte is 44");
    assert_eq!(damage, 9);
    assert_eq!(attributes, [1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn a_leveled_list_keeps_level_order_and_a_container_prepends() {
    use recomp_game::tes::form_components::{FormRef, TesContainer};
    let mut list = TesLeveledList::default();
    for (level, form) in [(5, 1), (1, 2), (5, 3), (9, 4)] {
        list.add_entry(level, 1, form);
    }
    let forms: Vec<FormRef> = list.entries_04.iter().map(|entry| entry.form_04).collect();
    assert_eq!(
        forms,
        [
            FormRef::Raw(2),
            FormRef::Raw(1),
            FormRef::Raw(3),
            FormRef::Raw(4)
        ],
        "after equal levels, before a greater one"
    );
    let mut container = TesContainer::default();
    container.add_entry(1, 2);
    container.add_entry(2, 3);
    container.add_entry(1, 9);
    assert_eq!(container.entries_08[0].item, FormRef::Raw(2));
    assert_eq!(
        container.entries_08[1].count, 9,
        "the same item takes the new count"
    );
    container.flags_04 = 1;
    container.add_entry(3, 1);
    assert_eq!(container.entries_08.len(), 2, "bit 0 set adds nothing");
}

#[test]
fn a_class_constructor_stores_its_defaults() {
    let fixture = Fixture::new("class");
    fixture.write("Base.esm", &tes4(true, 1.0, 0, &[]));
    let mut loader = fixture.loader();
    let class = recomp_game::tes::form_arms::form_class(5).unwrap();
    let form = (class.construct)(&mut loader.world).unwrap();
    let data = loader.world.get::<TesClass>(form).unwrap();
    assert_eq!(data.attributes_38, [0, 1]);
    assert_eq!(
        data.major_skills_44,
        [0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12]
    );
    assert_eq!(
        loader.world.get::<FormIdentity>(form).unwrap().form_type_04,
        5
    );
}

// IDA: N/A a form by ID.
fn form_at(world: &World, id: u32) -> Entity {
    world
        .resource::<FormsById>()
        .lookup(id)
        .unwrap_or_else(|| panic!("no form {id:08X}"))
}

/// The arms of the AI, environment and region family that the shipped Data doesn't reach, each from its disassembly.
#[test]
fn family_c_arms_the_shipped_data_misses() {
    use recomp_game::tes::forms_ai::TesPackage;
    use recomp_game::tes::forms_environment::{TesClimate, TesWeather};
    use recomp_game::tes::forms_region::{RegionDataKind, TesRegion, TesRegionData};

    let fixture = Fixture::new("family_c");
    let mut payload = Vec::new();
    // 004EDFA6..004EDFDB a 12-byte DATA, then a 13-byte one that's skipped; 004EDFE0 a short NAM0, skipped; SNAM's ID mapped by the file.
    payload.extend(group(
        b"WTHR",
        0,
        &record(
            b"WTHR",
            0,
            0x901,
            &cat(&[
                &chunk(b"DATA", &[1; 12]),
                &chunk(b"DATA", &[2; 13]),
                &chunk(b"NAM0", &[3; 16]),
                &chunk(b"SNAM", &cat(&[&u32_bytes(0x0500_0ABC), &u32_bytes(7)])),
            ]),
        ),
    ));
    // 004EEE9B a WLST entry that isn't a weather warns with the flag set, and the rest sort by chance.
    payload.extend(group(
        b"CLMT",
        0,
        &record(
            b"CLMT",
            0,
            0x902,
            &chunk(
                b"WLST",
                &cat(&[
                    &u32_bytes(0x901),
                    &u32_bytes(10),
                    &u32_bytes(0x903),
                    &u32_bytes(20),
                ]),
            ),
        ),
    ));
    payload.extend(group(
        b"PACK",
        0,
        &cat(&[
            // 00566620 an unrecognized PKDT; 005696C0 a PLDT type outside the table; 0056A0A0 a PTDT type over 2.
            &record(
                b"PACK",
                0,
                0x905,
                &cat(&[
                    &chunk(b"PKDT", &[1, 2, 3]),
                    &chunk(
                        b"PLDT",
                        &cat(&[&u32_bytes(9), &u32_bytes(0x44), &u32_bytes(5)]),
                    ),
                    &chunk(
                        b"PTDT",
                        &cat(&[&u32_bytes(3), &u32_bytes(0x55), &u32_bytes(6)]),
                    ),
                ]),
            ),
            // 005665D7 a 4-byte PKDT; 0056A7B0 a 0x14-byte CTDT keeps zero past it, with +0x14 set.
            &record(
                b"PACK",
                0,
                0x906,
                &cat(&[
                    &chunk(b"PKDT", &[0x34, 0x12, 2, 9]),
                    &chunk(b"CTDT", &[0xAA; 0x14]),
                ]),
            ),
        ]),
    ));
    // The point lists: the square's corners go in reversed, an odd size warns, a bow tie is dropped.
    let points = |corners: &[(f32, f32)]| -> Vec<u8> {
        corners
            .iter()
            .flat_map(|&(x, y)| cat(&[&f32_bytes(x), &f32_bytes(y)]))
            .collect()
    };
    payload.extend(group(
        b"REGN",
        0,
        &cat(&[
            &record(
                b"REGN",
                0,
                0x907,
                &cat(&[
                    // No current data yet: nothing loads.
                    &chunk(b"RDMP", &text("Lost")),
                    &chunk(b"RDAT", &cat(&[&u32_bytes(4), &[8, 101, 0, 0]])),
                    &chunk(b"RDMP", &text("First")),
                    &chunk(b"RPLI", &u32_bytes(0x20)),
                    &chunk(b"RPLD", &[0; 9]),
                    &chunk(
                        b"RPLD",
                        &points(&[(0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]),
                    ),
                    &chunk(b"RPLI", &u32_bytes(0x30)),
                    &chunk(
                        b"RPLD",
                        &points(&[(0.0, 0.0), (2.0, 2.0), (2.0, 0.0), (0.0, 2.0)]),
                    ),
                ]),
            ),
            // No RDAT of its own: 00B3541C is still the region before's.
            &record(b"REGN", 0, 0x908, &chunk(b"RDMP", &text("Second"))),
        ]),
    ));
    // CSTD: a NaN at +0x74 stays, a zero at +0x78 and a negative +0x88 go back to their settings.
    let mut cstd = vec![0_u8; 0x7C];
    cstd[0x74 - 0x18..0x78 - 0x18].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
    cstd[0x8C - 0x18..0x90 - 0x18].copy_from_slice(&1.5_f32.to_bits().to_le_bytes());
    cstd[0x88 - 0x18] = 0xFF;
    payload.extend(group(
        b"CSTY",
        0,
        &record(b"CSTY", 0, 0x909, &chunk(b"CSTD", &cstd)),
    ));
    let idle = |id: u32, parent: u32, previous: u32| {
        record(
            b"IDLE",
            0,
            id,
            &cat(&[
                &chunk(b"MODL", &text("A\\IdleAnims\\x.kf")),
                &chunk(b"DATA", &cat(&[&u32_bytes(parent), &u32_bytes(previous)])),
            ]),
        )
    };
    payload.extend(group(
        b"IDLE",
        0,
        &cat(&[
            &idle(0x90A, 0, 0),
            &idle(0x90B, 0x90A, 0),
            &idle(0x90C, 0x90A, 0x90B),
            &idle(0x90D, 0x7777, 0),
        ]),
    ));
    let mut esm = tes4(true, 1.0, 13, &[]);
    esm.extend(payload);
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    let world = &loader.world;

    let weather = world
        .get::<TesWeather>(form_at(world, 0x901))
        .expect("a weather");
    assert_eq!(weather.data_48[..12], [1; 12]);
    assert_eq!(weather.data_48[12..], [0xFF; 3]);
    assert_eq!(weather.colors_68, [0; 0xA0]);
    assert_eq!(weather.sounds_108, [[0x0000_0ABC, 7]], "mod index 0 on top");

    let climate = world
        .get::<TesClimate>(form_at(world, 0x902))
        .expect("a climate");
    assert_eq!(climate.weathers_30.len(), 1);
    assert_eq!(climate.weathers_30[0].chance, 10);

    let package = world
        .get::<TesPackage>(form_at(world, 0x905))
        .expect("a package");
    assert_eq!((package.flags_1c, package.type_20), (0, 0));
    let location = package.location_24.unwrap();
    assert_eq!(
        (location.type_00, location.radius_04, location.object_08),
        (9, 0, FormRef::Raw(0)),
        "a location built by 005696B0 and 00569840: type 9 copies nothing"
    );
    let target = package.target_28.unwrap();
    assert_eq!(
        (target.type_00, target.value_04, target.count_08),
        (3, FormRef::Raw(0), 6),
        "a target with +0x04 untouched"
    );
    let package = world
        .get::<TesPackage>(form_at(world, 0x906))
        .expect("a package");
    assert_eq!((package.flags_1c, package.type_20), (0x1234, 2));
    assert!(package.location_24.is_some() && package.target_28.is_some());
    let mut expected = [0xAA_u8; 0x18];
    expected[0x14..].copy_from_slice(&[1, 0, 0, 0]);
    assert_eq!(package.conditions_34.items[0].data_00, expected);

    let region = world
        .get::<TesRegion>(form_at(world, 0x907))
        .expect("a region");
    assert_eq!(region.areas_1c.len(), 1, "the bow tie is gone");
    assert_eq!(region.areas_1c[0].edge_falloff_20, 0x20);
    assert_eq!(region.areas_1c[0].count_24, 4);
    assert_eq!(region.areas_1c[0].points_00[0].y, 2.0, "prepended");
    assert_eq!(region.areas_1c[0].min_x_10, 0.0);
    assert_eq!(region.areas_1c[0].max_y_1c, 2.0);
    assert_eq!(region.data_18.items.len(), 1);
    let data = world
        .get::<TesRegionData>(region.data_18.items[0])
        .expect("region data");
    assert_eq!(data.flags_04, 8);
    assert_eq!(data.priority_06, 0x32, "a priority over 100 is ignored");
    let RegionDataKind::Map { name_08 } = &data.kind else {
        panic!("a map");
    };
    assert_eq!(
        name_08.data_00.as_deref(),
        Some(&b"Second"[..]),
        "00B3541C outlives its region"
    );
    assert!(
        world
            .get::<TesRegion>(form_at(world, 0x908))
            .unwrap()
            .data_18
            .items
            .is_empty()
    );

    let style = world
        .get::<recomp_game::tes::forms_ai::TesCombatStyle>(form_at(world, 0x909))
        .expect("a combat style");
    let settings = world.resource::<GameSettingCollection>();
    let word =
        |at: usize| u32::from_le_bytes(style.data_18[at - 0x18..at - 0x14].try_into().unwrap());
    assert_eq!(word(0x74), f32::NAN.to_bits(), "a NaN stays");
    assert_eq!(word(0x78), settings.raw_at(0x00B3_5464 + 0x214).unwrap());
    assert_eq!(word(0x8C), 1.5_f32.to_bits());
    assert_eq!(
        style.data_18[0x88 - 0x18],
        settings.raw_at(0x00B3_5464 + 0x314).unwrap() as u8
    );

    let (top, first, second) = (
        form_at(world, 0x90A),
        form_at(world, 0x90B),
        form_at(world, 0x90C),
    );
    let idle_of = |form| {
        world
            .get::<recomp_game::tes::forms_ai::TesIdleForm>(form)
            .unwrap()
    };
    let children = idle_of(top).children_3c.as_ref().unwrap();
    assert_eq!(
        (children.at(0), children.at(1)),
        (Some(first), Some(second))
    );
    assert_eq!(idle_of(second).previous_44, Some(first));
    assert_eq!(idle_of(first).parent_40, Some(top));
    let roots = world.resource::<recomp_game::tes::forms_ai::IdleAnimRoots>();
    assert_eq!(roots.roots.len(), 1);
    assert_eq!(
        roots.roots[0].path_18.data_00.as_deref(),
        Some(&b"A\\IdleAnims"[..])
    );
    assert_eq!(
        (
            roots.roots[0].idles.at(0),
            roots.roots[0].idles.at(1),
            roots.roots[0].idles.size_0c
        ),
        (Some(form_at(world, 0x90D)), Some(top), 2),
        "0x90D's missing parent makes it a root idle too, inserted at 0 with flag bit 5 clear"
    );

    assert_eq!(
        loader.warnings,
        [
            "[disabled] Error while loading weather list...cannot identify TESWeather from form ID: (00000903)",
            "[disabled] File 'Base.esm' contains package data of unrecognized type.",
            "[disabled] Invalid Region Point List data in file \"Base.esm\".",
            "[disabled] Could not find parent idle (00007777) for IDLE Form '' (0000090D).",
        ],
        "and no \"Warnings were encountered\": nothing increments 00B34D88"
    );
}

/// A reload runs the reset pair: an idle leaves its parent's children, and goes back; a package and a region lose what they held.
#[test]
fn family_c_reloads_through_the_reset_pair() {
    use recomp_game::tes::forms_ai::{TesIdleForm, TesPackage};
    use recomp_game::tes::forms_region::TesRegion;

    let fixture = Fixture::new("family_c_reload");
    let idle = |id: u32, parent: u32, previous: u32| {
        record(
            b"IDLE",
            0,
            id,
            &cat(&[
                &chunk(b"MODL", &text("A\\IdleAnims\\x.kf")),
                &chunk(b"ANAM", &[9]),
                &chunk(b"DATA", &cat(&[&u32_bytes(parent), &u32_bytes(previous)])),
            ]),
        )
    };
    let mut esm = tes4(true, 1.0, 6, &[]);
    esm.extend(group(
        b"IDLE",
        0,
        &cat(&[
            &idle(0x901, 0, 0),
            &idle(0x902, 0x901, 0),
            &idle(0x903, 0x901, 0x902),
        ]),
    ));
    esm.extend(group(
        b"PACK",
        0,
        &record(
            b"PACK",
            0,
            0x904,
            &cat(&[
                &chunk(b"PKDT", &cat(&[&u32_bytes(0x80), &[5, 0, 0, 0]])),
                &chunk(
                    b"PLDT",
                    &cat(&[&u32_bytes(0), &u32_bytes(0x44), &u32_bytes(5)]),
                ),
            ]),
        ),
    ));
    esm.extend(group(
        b"REGN",
        0,
        &record(
            b"REGN",
            0,
            0x905,
            &cat(&[
                &chunk(b"WNAM", &u32_bytes(0x77)),
                &chunk(b"RDAT", &cat(&[&u32_bytes(4), &[0, 0, 0, 0]])),
            ]),
        ),
    ));
    fixture.write("Base.esm", &esm);
    let mut plugin = tes4(false, 0.8, 3, &[("Base.esm", 0)]);
    plugin.extend(group(b"IDLE", 0, &idle(0x902, 0x901, 0x903)));
    plugin.extend(group(
        b"PACK",
        0,
        &record(b"PACK", 0, 0x904, &chunk(b"PKDT", &[1, 0, 0, 0])),
    ));
    plugin.extend(group(b"REGN", 0, &record(b"REGN", 0, 0x905, &[])));
    fixture.write("Second.esp", &plugin);
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    assert_eq!(loader.warnings, Vec::<String>::new());
    let world = &loader.world;
    let (top, moved, stayed) = (
        form_at(world, 0x901),
        form_at(world, 0x902),
        form_at(world, 0x903),
    );
    let idle_of = |form| world.get::<TesIdleForm>(form).unwrap();
    let children = idle_of(top).children_3c.as_ref().unwrap();
    assert_eq!(
        (children.at(0), children.at(1), children.size_0c),
        (Some(stayed), Some(moved), 2),
        "005204F0 took it out, and 00520780 put it after its new previous"
    );
    assert_eq!(idle_of(moved).previous_44, Some(stayed));
    assert_eq!(idle_of(stayed).previous_44, None, "005204F0's store");
    assert_eq!(idle_of(moved).group_38, 9);
    let package = world.get::<TesPackage>(form_at(world, 0x904)).unwrap();
    assert_eq!((package.flags_1c, package.type_20), (1, 0));
    assert!(package.location_24.is_none(), "00566190 freed it");
    assert!(
        package.target_28.is_none(),
        "005661E0 with the type it already has returns"
    );
    assert_eq!(package.field_18, 0xFFFF_FFFF);
    let region = world.get::<TesRegion>(form_at(world, 0x905)).unwrap();
    assert!(region.data_18.items.is_empty(), "004A2E20");
    assert_eq!(
        region.worldspace_20,
        recomp_game::tes::form_components::FormRef::Raw(0x77),
        "0046C2E0 doesn't touch +0x20"
    );
}

// ---- the misc, script and item arms the run didn't take ----

fn loaded(name: &str, records: &[Vec<u8>]) -> (Fixture, Loader) {
    let fixture = Fixture::new(name);
    let mut esm = tes4(true, 1.0, records.len() as u32 + 1, &[]);
    for one in records {
        esm.extend_from_slice(one);
    }
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    install_effect_settings(
        &mut loader.world,
        &[
            (0x4141_4141, 0x0100_0011, 0x2A, b"Plain"),
            (0x4646_4553, 0x20, 0, b"Scripted"),
            (0x4242_4242, 0x40, 7, b"Far"),
        ],
    );
    assert_eq!(loader.run(), Ok(1));
    (fixture, loader)
}

fn form_of(loader: &Loader, id: u32) -> Entity {
    loader.world.resource::<FormsById>().lookup(id).unwrap()
}

#[test]
fn a_weapon_with_a_bad_type_warns_and_clears_only_the_byte() {
    let data = cat(&[
        &u32_bytes(0x0102_0306),
        &f32_bytes(1.0),
        &f32_bytes(2.0),
        &u32_bytes(5),
        &u32_bytes(100),
        &u32_bytes(200),
        &f32_bytes(3.5),
        &u16_bytes(9),
    ]);
    let (_fixture, loader) = loaded(
        "weapon",
        &[record(
            b"WEAP",
            0,
            0x901,
            &cat(&[&chunk(b"DATA", &data), &chunk(b"ANAM", &u16_bytes(44))]),
        )],
    );
    let form = form_of(&loader, 0x901);
    let world = &loader.world;
    let weapon = world.get::<TesWeapon>(form).unwrap();
    assert_eq!(weapon.data_90[..4], [0, 3, 2, 1], "0x4BB327 a byte store");
    assert_eq!(world.get::<TesValueForm>(form).unwrap().value_04, 100);
    assert_eq!(world.get::<TesHealthForm>(form).unwrap().health_04, 200);
    assert_eq!(world.get::<TesWeightForm>(form).unwrap().weight_04, 3.5);
    assert_eq!(world.get::<TesAttackDamageForm>(form).unwrap().damage_04, 9);
    let enchantment = world.get::<TesEnchantableForm>(form).unwrap();
    assert_eq!((enchantment.charge_08, enchantment.cast_type_0c), (44, 2));
    assert!(
        loader
            .warnings
            .iter()
            .any(|text| text == "[disabled] Clearing invalid type on weapon '' (00000901)."),
        "{:?}",
        loader.warnings
    );
    assert!(world.get::<FormIdentity>(form).unwrap().flags_08 & 8 == 0);
}

#[test]
fn a_negative_weapon_type_stays_since_the_compare_is_signed() {
    let (_fixture, loader) = loaded(
        "weapon_signed",
        &[record(b"WEAP", 0, 0x901, &chunk(b"DATA", &[0x80; 0x10]))],
    );
    let weapon = loader
        .world
        .get::<TesWeapon>(form_of(&loader, 0x901))
        .unwrap();
    assert_eq!(weapon.data_90[0], 0x80);
    assert!(loader.warnings.is_empty(), "{:?}", loader.warnings);
}

/// 0x4B0EA0 `test ah, 0x44; jp`: only an equal compare stores, so -0.0 takes the default and a NaN doesn't.
#[test]
fn a_light_defaults_a_zero_fov_and_falloff() {
    let data = |falloff: u32, fov: u32| {
        cat(&[
            &u32_bytes(1),
            &u32_bytes(2),
            &u32_bytes(3),
            &u32_bytes(4),
            &u32_bytes(falloff),
            &u32_bytes(fov),
        ])
    };
    let (_fixture, loader) = loaded(
        "light",
        &[
            record(b"LIGH", 0, 0x901, &chunk(b"DATA", &data(0, 0x8000_0000))),
            record(
                b"LIGH",
                0,
                0x902,
                &chunk(b"DATA", &data(0x7FC0_0000, 0x4000_0000)),
            ),
            // No DATA: the two stay the 0.0 stored before the chunks.
            record(b"LIGH", 0, 0x903, &chunk(b"FNAM", &f32_bytes(0.5))),
        ],
    );
    let light = |id| *loader.world.get::<TesLight>(form_of(&loader, id)).unwrap();
    assert_eq!((light(0x901).falloff_80, light(0x901).fov_84), (1.0, 90.0));
    assert_eq!(light(0x902).fov_84, 2.0);
    assert!(light(0x902).falloff_80.is_nan());
    assert_eq!(
        (
            light(0x903).falloff_80,
            light(0x903).fov_84,
            light(0x903).fade_88
        ),
        (0.0, 0.0, 0.5)
    );
}

#[test]
fn a_subspace_truncates_and_computes_its_radius() {
    let dnam = cat(&[&f32_bytes(100.9), &f32_bytes(70000.5), &f32_bytes(-1.5)]);
    let (_fixture, loader) = loaded(
        "subspace",
        &[
            record(b"SBSP", 0, 0x901, &chunk(b"DNAM", &dnam)),
            record(b"SBSP", 0, 0x902, &chunk(b"EDID", &text("Default"))),
        ],
    );
    let space = |id| {
        *loader
            .world
            .get::<TesSubSpace>(form_of(&loader, id))
            .unwrap()
    };
    // fistp under chop keeps the low word: 70000 is 0x11170, and -1 is 0xFFFF.
    assert_eq!(space(0x901).extents_24, [100, 0x1170, 0xFFFF]);
    let half = [50.0_f32, 2232.0, 32767.5];
    let sum = half[0] * half[0] + half[1] * half[1];
    assert_eq!(
        space(0x901).radius_2c,
        (sum + half[2] * half[2]).sqrt(),
        "each step at float"
    );
    assert_eq!(space(0x902).extents_24, [400, 400, 200]);
    assert_eq!(space(0x902).radius_2c, 300.0, "from the extents, always");
}

#[test]
fn a_door_resolves_sounds_while_loading_and_prepends_teleports() {
    let (_fixture, loader) = loaded(
        "door",
        &[
            record(b"SOUN", 0, 0x900, &chunk(b"EDID", &text("Hinge"))),
            record(
                b"DOOR",
                0,
                0x901,
                &cat(&[
                    &chunk(b"SNAM", &u32_bytes(0x900)),
                    // A form of another class and an absent ID are both silent nulls.
                    &chunk(b"ANAM", &u32_bytes(0x901)),
                    &chunk(b"BNAM", &u32_bytes(0x999)),
                    &chunk(b"TNAM", &u32_bytes(0xA)),
                    &chunk(b"TNAM", &u32_bytes(0)),
                    &chunk(b"TNAM", &u32_bytes(0xB)),
                    &chunk(b"FNAM", &[3]),
                ]),
            ),
        ],
    );
    let door = loader
        .world
        .get::<TesDoor>(form_of(&loader, 0x901))
        .unwrap();
    assert_eq!(door.open_sound_58, Some(form_of(&loader, 0x900)));
    assert_eq!((door.close_sound_5c, door.loop_sound_60), (None, None));
    assert_eq!(door.flags_64, 3);
    assert_eq!(door.teleports_68, [FormRef::Raw(0xB), FormRef::Raw(0xA)]);
    assert!(loader.warnings.is_empty(), "{:?}", loader.warnings);
}

/// 0046F020: a script that resolves, one that doesn't, and the editor-less owner's arm.
#[test]
fn a_scri_resolves_through_the_owners_file_or_warns() {
    let script = record(
        b"SCPT",
        0,
        0x900,
        &cat(&[
            &chunk(
                b"SCHR",
                &cat(&[
                    &u32_bytes(0),
                    &u32_bytes(1),
                    &u32_bytes(4),
                    &u32_bytes(2),
                    &u32_bytes(0),
                ]),
            ),
            &chunk(b"SCDA", &[1, 2, 3, 4]),
            &chunk(b"SLSD", &[7; 0x18]),
            &chunk(b"SCVR", &text("first")),
            &chunk(b"SCRO", &u32_bytes(0x123)),
            &chunk(b"SCRV", &u32_bytes(0x456)),
            &chunk(b"RNAM", &u32_bytes(1)),
        ]),
    );
    let (_fixture, loader) = loaded(
        "scri",
        &[
            script,
            record(b"SCPT", 0, 0x904, &chunk(b"EDID", &text("Unbuilt"))),
            record(b"ACTI", 0, 0x901, &chunk(b"SCRI", &u32_bytes(0x900))),
            record(b"ACTI", 0, 0x902, &chunk(b"SCRI", &u32_bytes(0x777))),
            record(b"ACTI", 0, 0x903, &chunk(b"FULL", &text("None"))),
        ],
    );
    let world = &loader.world;
    let part = |id| {
        *world
            .get::<TesScriptableForm>(form_of(&loader, id))
            .unwrap()
    };
    assert_eq!(
        part(0x901).script_04,
        FormRef::Resolved(Some(form_of(&loader, 0x900)))
    );
    assert_eq!(part(0x901).resolved_08, 1);
    assert_eq!(part(0x902).script_04, FormRef::Resolved(None));
    assert_eq!(part(0x902).resolved_08, 1);
    assert_eq!(part(0x903).script_04, FormRef::Raw(0));
    assert_eq!(part(0x903).resolved_08, 0, "no SCRI: 0046F020 isn't called");
    let has = |want: &str| loader.warnings.iter().any(|text| text == want);
    assert!(
        has("[disabled] Unable to find script (00000777) on owner object (00000902)."),
        "{:?}",
        loader.warnings
    );
    assert!(
        has("[disabled] Script '' in file 'Base.esm' has not been compiled.\r\n"),
        "{:?}",
        loader.warnings
    );
    let script = world.get::<Script>(form_of(&loader, 0x900)).unwrap();
    assert_eq!(script.header_18[4], 1);
    assert_eq!(script.data_30.as_deref(), Some(&[1, 2, 3, 4][..]));
    assert_eq!(script.variables_48[0].data_00, [7; 0x18]);
    assert_eq!(
        script.variables_48[0].name_18.data_00.as_deref(),
        Some(&b"first"[..])
    );
    assert_eq!(
        (
            script.references_40[0].form_08,
            script.references_40[1].variable_0c
        ),
        (0x123, 0x456)
    );
}

#[test]
fn an_oversized_schr_is_a_host_stop() {
    let fixture = Fixture::new("schr");
    let mut esm = tes4(true, 1.0, 2, &[]);
    esm.extend(record(b"SCPT", 0, 0x900, &chunk(b"SCHR", &[0; 0x15])));
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    assert_eq!(
        loader.run().unwrap_err().0,
        "004FC2F8: an SCHR chunk past 0x14 bytes overruns +0x2C"
    );
}

/// The exe compares 0x4C544D44, "DMTL", where the C++ has "DOTL".
#[test]
fn a_static_dmtl_is_a_host_stop_and_dotl_is_nothing() {
    let (_fixture, loader) = loaded(
        "stat_dotl",
        &[record(b"STAT", 0, 0x901, &chunk(b"DOTL", &[0; 4]))],
    );
    assert!(loader.warnings.is_empty(), "{:?}", loader.warnings);
    let fixture = Fixture::new("stat_dmtl");
    let mut esm = tes4(true, 1.0, 2, &[]);
    esm.extend(record(b"STAT", 0, 0x901, &chunk(b"DMTL", &[0; 4])));
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    assert_eq!(
        loader.run().unwrap_err().0,
        "004B9BE2: 004B3460, the STAT DMTL chunk"
    );
}

#[test]
fn leveled_lists_decode_their_bytes_and_order_entries() {
    let entry = |level: u32, form: u32, count: u32| {
        chunk(
            b"LVLO",
            &cat(&[&u32_bytes(level), &u32_bytes(form), &u32_bytes(count)]),
        )
    };
    let (_fixture, loader) = loaded(
        "leveled",
        &[
            record(
                b"LVLI",
                0,
                0x901,
                &cat(&[
                    &chunk(b"LVLD", &[0x85]),
                    &entry(5, 0xA, 1),
                    &entry(1, 0xB, 2),
                    // A short entry keeps the local's count of 1.
                    &chunk(b"LVLO", &cat(&[&u32_bytes(9), &u32_bytes(0xC)])),
                    &chunk(b"DATA", &[1]),
                ]),
            ),
            record(
                b"LVLC",
                0,
                0x902,
                &cat(&[
                    &chunk(b"LVLF", &[4]),
                    &chunk(b"LVLD", &[0x05]),
                    &chunk(b"TNAM", &u32_bytes(0x77)),
                ]),
            ),
        ],
    );
    let world = &loader.world;
    let list = world
        .get::<TesLeveledList>(form_of(&loader, 0x901))
        .unwrap();
    assert_eq!(list.chance_none_0c, 5);
    assert_eq!(list.flags_0d, 3, "LVLD's bit 0 and DATA's bit 1");
    let levels: Vec<(u16, i16)> = list
        .entries_04
        .iter()
        .map(|entry| (entry.level_00, entry.count_08))
        .collect();
    assert_eq!(levels, [(1, 2), (5, 1), (9, 1)]);
    let creature = form_of(&loader, 0x902);
    let list = world.get::<TesLeveledList>(creature).unwrap();
    assert_eq!((list.chance_none_0c, list.flags_0d), (5, 4));
    assert_eq!(
        world.get::<TesLevCreature>(creature).unwrap().template_40,
        FormRef::Raw(0x77)
    );
    for id in [0x901, 0x902] {
        let flags = world
            .get::<FormIdentity>(form_of(&loader, id))
            .unwrap()
            .flags_08;
        assert_eq!(flags & 8, 0, "0046AB80(0) after the chunks");
    }
}

#[test]
fn a_tree_appends_nonzero_seeds_and_gates_its_sizes() {
    let (_fixture, loader) = loaded(
        "tree",
        &[record(
            b"TREE",
            0,
            0x901,
            &cat(&[
                &chunk(
                    b"SNAM",
                    &cat(&[&u32_bytes(5), &u32_bytes(0), &u32_bytes(6)]),
                ),
                // Not a multiple of 4: nothing. The wrong sizes leave the defaults.
                &chunk(b"SNAM", &[1, 2, 3]),
                &chunk(b"CNAM", &[0; 0x1F]),
                &chunk(b"BNAM", &cat(&[&f32_bytes(3.0), &f32_bytes(4.0)])),
            ]),
        )],
    );
    let tree = loader
        .world
        .get::<TesTree>(form_of(&loader, 0x901))
        .unwrap();
    let seeds = &tree.seeds_48;
    assert_eq!(seeds.data_04.as_deref(), Some(&[5, 6][..]));
    assert_eq!(
        (
            seeds.capacity_08,
            seeds.size_0a,
            seeds.occupied_0c,
            seeds.grow_by_0e
        ),
        (2, 2, 2, 1)
    );
    assert_eq!(tree.parameters_58[0], 2.5_f32.to_bits());
    assert_eq!(tree.parameters_58[5], 0, "+0x6C, unset, is host zero");
    assert_eq!(tree.billboard_78, [3.0_f32.to_bits(), 4.0_f32.to_bits()]);
}

#[test]
fn a_soul_gem_reads_its_bytes_only_at_size_1_and_a_faction_holds_ranks() {
    let (_fixture, loader) = loaded(
        "gem",
        &[
            record(
                b"SLGM",
                0,
                0x901,
                &cat(&[
                    &chunk(b"SOUL", &[2, 9]),
                    &chunk(b"SLCP", &[3]),
                    &chunk(b"DATA", &cat(&[&u32_bytes(25), &f32_bytes(0.5)])),
                ]),
            ),
            record(
                b"FACT",
                0,
                0x902,
                &cat(&[
                    // A title before any rank reads nothing, and a rank's RNAM data isn't read.
                    &chunk(b"MNAM", &text("Skipped")),
                    &chunk(b"RNAM", &u32_bytes(3)),
                    &chunk(b"MNAM", &text("Lord")),
                    &chunk(b"FNAM", &text("Lady")),
                    &chunk(b"INAM", &text("rank.dds")),
                    &chunk(b"XNAM", &cat(&[&u32_bytes(0x55), &u32_bytes(0xFFFF_FFFB)])),
                    &chunk(b"CNAM", &f32_bytes(2.0)),
                    &chunk(b"DATA", &[6]),
                ]),
            ),
            record(
                b"LTEX",
                0,
                0x903,
                &cat(&[
                    &chunk(b"HNAM", &[1, 2, 3]),
                    &chunk(b"SNAM", &[4]),
                    &chunk(b"GNAM", &u32_bytes(0)),
                    &chunk(b"GNAM", &u32_bytes(0x66)),
                    &chunk(b"GNAM", &u32_bytes(0x67)),
                    &chunk(b"MODL", &text("a.dds")),
                ]),
            ),
        ],
    );
    let world = &loader.world;
    let gem_form = form_of(&loader, 0x901);
    let gem = world.get::<TesSoulGem>(gem_form).unwrap();
    assert_eq!((gem.soul_70, gem.capacity_71), (0, 3));
    assert_eq!(world.get::<TesValueForm>(gem_form).unwrap().value_04, 25);
    assert_eq!(world.get::<TesWeightForm>(gem_form).unwrap().weight_04, 0.5);
    let faction_form = form_of(&loader, 0x902);
    let faction = world.get::<TesFaction>(faction_form).unwrap();
    assert_eq!(faction.ranks_3c.len(), 1);
    assert_eq!(
        faction.ranks_3c[0].male_00.data_00.as_deref(),
        Some(&b"Lord"[..])
    );
    assert_eq!(
        faction.ranks_3c[0].female_08.data_00.as_deref(),
        Some(&b"Lady"[..])
    );
    assert_eq!(
        faction.ranks_3c[0].insignia_10.path_04.data_00.as_deref(),
        Some(&b"rank.dds"[..])
    );
    assert_eq!(faction.flags_34, 6);
    assert_eq!(faction.crime_gold_multiplier_38, 2.0);
    let reaction = world.get::<TesReactionForm>(faction_form).unwrap();
    assert_eq!(reaction.reactions_04[0].faction, FormRef::Raw(0x55));
    assert_eq!(reaction.reactions_04[0].modifier, -5);
    let land_form = form_of(&loader, 0x903);
    let land = world.get::<TesLandTexture>(land_form).unwrap();
    assert_eq!(land.havok_28, [1, 2, 3, 4]);
    assert_eq!(land.grasses_2c, [FormRef::Raw(0x66), FormRef::Raw(0x67)]);
    assert_eq!(
        world
            .get::<TesTexture>(land_form)
            .unwrap()
            .path_04
            .data_00
            .as_deref(),
        Some(&b"a.dds"[..])
    );
}

/// 004154B0 and 00413130 over each arm.
#[test]
fn effect_items_through_the_list() {
    let efit = |code: u32, actor_value: u32| {
        chunk(
            b"EFIT",
            &cat(&[
                &u32_bytes(code),
                &u32_bytes(5),
                &u32_bytes(6),
                &u32_bytes(7),
                &u32_bytes(1),
                &u32_bytes(actor_value),
            ]),
        )
    };
    let efid = |code: u32| chunk(b"EFID", &u32_bytes(code));
    let scit = chunk(
        b"SCIT",
        &cat(&[
            &u32_bytes(0x0100_0ABC),
            &u32_bytes(2),
            &u32_bytes(0x9999_9999),
            &u32_bytes(0x0101_0101),
        ]),
    );
    let (_fixture, loader) = loaded(
        "effects",
        &[
            record(
                b"ALCH",
                0,
                0x901,
                &cat(&[
                    &chunk(b"FULL", &text("Potion")),
                    &efid(0x4141_4141),
                    &efit(0x4141_4141, 3),
                    &efid(0x4242_4242),
                    &efit(0x4242_4242, 3),
                    &efid(0x4646_4553),
                    &efit(0x4646_4553, 3),
                    &scit,
                    &chunk(b"FULL", &text("Spelled")),
                    &chunk(b"ENIT", &cat(&[&u32_bytes(50), &[1, 2, 3, 4]])),
                    &chunk(b"DATA", &f32_bytes(0.25)),
                ]),
            ),
            // A code with no setting; an EFID with no EFIT; no items at all.
            record(
                b"INGR",
                0,
                0x902,
                &cat(&[
                    &chunk(b"FULL", &text("Root")),
                    &efid(0x5A5A_5A5A),
                    &efid(0x4141_4141),
                    &chunk(b"ENIT", &[9; 8]),
                ]),
            ),
            record(
                b"SGST",
                0,
                0x903,
                &cat(&[
                    &chunk(b"FULL", &text("Stone")),
                    &efid(0x4141_4141),
                    &efit(0x4141_4141, 3),
                    &chunk(
                        b"DATA",
                        &cat(&[&[4], &u32_bytes(10)[..], &f32_bytes(1.5)[..]]),
                    ),
                ]),
            ),
        ],
    );
    let world = &loader.world;
    let alchemy = form_of(&loader, 0x901);
    let list = world.get::<EffectItemList>(alchemy).unwrap();
    assert_eq!(list.items_04.len(), 3);
    assert_eq!(list.hostile_count_0c, 2, "bit 0 of the first, SCIT's byte");
    // Bit 24 takes the associated value over the chunk's.
    assert_eq!(list.items_04[0].data_00, [0x4141_4141, 5, 6, 7, 1, 0x2A]);
    assert_eq!(list.items_04[1].data_00[5], 3);
    assert_eq!(list.items_04[0].cost_20, -1.0);
    let script = list.items_04[2].script_18.as_ref().unwrap();
    assert_eq!(script.school_04, 2);
    assert_eq!(script.visual_10, 0, "a visual with no setting clears");
    assert_eq!(script.hostile_14, 1);
    assert_eq!(script.name_08.data_00.as_deref(), Some(&b"Spelled"[..]));
    // Top byte 1 is past Base.esm's masters, so its own mod index, 0.
    assert_eq!(script.script_00, 0x0000_0ABC);
    let object = world.get::<TesMagicObject>(alchemy).unwrap();
    assert_eq!(
        (object.value_78, object.flags_7c, object.rest_7d),
        (50, 1, Some([2, 3, 4]))
    );
    assert_eq!(world.get::<TesWeightForm>(alchemy).unwrap().weight_04, 0.25);
    let has = |want: &str| loader.warnings.iter().any(|text| text == want);
    assert!(
        has("[disabled] Unable to find EffectSetting 1515870810 in spell '' (00000902)."),
        "{:?}",
        loader.warnings
    );
    assert!(
        has("[disabled] Unable to load EffectItem 'Plain' in spell '' (00000902)."),
        "{:?}",
        loader.warnings
    );
    assert!(
        has("[disabled] Magic Item (00000902) Root has no effects defined."),
        "{:?}",
        loader.warnings
    );
    let stone = form_of(&loader, 0x903);
    assert_eq!(world.get::<TesUsesForm>(stone).unwrap().uses_04, 4);
    assert_eq!(world.get::<TesValueForm>(stone).unwrap().value_04, 10);
    assert_eq!(world.get::<TesWeightForm>(stone).unwrap().weight_04, 1.5);
}

/// 00413130 warns and returns 1 for an EFIT code with no setting, and then 00413470 reads through the null +0x1C: the original faults.
#[test]
fn an_efit_with_no_setting_warns_then_stops() {
    let fixture = Fixture::new("efit_stop");
    let mut esm = tes4(true, 1.0, 2, &[]);
    esm.extend(record(
        b"SGST",
        0,
        0x901,
        &cat(&[
            &chunk(b"FULL", &text("Stone")),
            &chunk(b"EFID", &u32_bytes(0x4141_4141)),
            &chunk(b"EFIT", &[0x5A; 0x18]),
        ]),
    ));
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    install_effect_settings(&mut loader.world, &[(0x4141_4141, 0x10, 0, b"")]);
    assert_eq!(
        loader.run().unwrap_err().0,
        "00413470: an EffectItem without a setting faults"
    );
    assert!(
        loader.warnings.iter().any(|text| text
            == "[disabled] Unknown EffectSetting '1515870810' encountered when loading EffectItem in spell 'Stone'"),
        "{:?}",
        loader.warnings
    );
}

#[test]
fn an_effect_with_no_collection_is_a_host_stop() {
    let fixture = Fixture::new("effect_stop");
    let mut esm = tes4(true, 1.0, 2, &[]);
    esm.extend(record(b"INGR", 0, 0x901, &chunk(b"EFID", &u32_bytes(1))));
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    loader.world.remove_resource::<EffectSettingCollection>();
    assert_eq!(
        loader.run().unwrap_err().0,
        "00416870: the EffectSettingCollection is required (it is built before WinMain)"
    );
}

/// 004703C0's tail reaches 0045B670, which returns while TLS +0x184 is set or +0x185 clear and otherwise goes on to the change record.
#[test]
fn the_value_tail_passes_through_vt_40() {
    let world_with = |byte_184, track_changes_185| {
        let mut world = World::new();
        world.init_resource::<GameThreadLocals>();
        construct_data_handler(&mut world).unwrap();
        let mut locals = world.resource_mut::<GameThreadLocals>();
        locals.byte_184 = byte_184;
        locals.track_changes_185 = track_changes_185;
        let form = world
            .spawn(FormIdentity {
                form_type_04: 0x1B,
                flags_08: 0,
                form_id_0c: 1,
            })
            .id();
        (world, form)
    };
    let (mut world, form) = world_with(1, 0);
    assert_eq!(set_change_flags(&mut world, form, 8), Ok(()));
    let (mut world, form) = world_with(1, 1);
    assert_eq!(set_change_flags(&mut world, form, 8), Ok(()));
    let (mut world, form) = world_with(0, 1);
    assert_eq!(
        set_change_flags(&mut world, form, 8).unwrap_err().0,
        "0045B6B4: 0045A500 on a null g_saveLoadGame"
    );
}

/// Reloading runs the reset pair: a second record of a form keeps nothing the first set, but what the reset leaves.
#[test]
fn a_reload_runs_each_classes_reset_pair() {
    let fixture = Fixture::new("reload");
    let mut esm = tes4(true, 1.0, 3, &[]);
    esm.extend(record(
        b"BOOK",
        0,
        0x901,
        &cat(&[
            &chunk(
                b"DATA",
                &cat(&[&[5, 6], &u32_bytes(1)[..], &f32_bytes(1.0)[..]]),
            ),
            &chunk(b"ENAM", &u32_bytes(0xE)),
        ]),
    ));
    esm.extend(record(
        b"CONT",
        0,
        0x902,
        &chunk(b"CNTO", &cat(&[&u32_bytes(0xA), &u32_bytes(2)])),
    ));
    fixture.write("Base.esm", &esm);
    let mut esp = tes4(false, 0.8, 3, &[("Base.esm", 0)]);
    esp.extend(record(b"BOOK", 0, 0x901, &chunk(b"EDID", &text("Again"))));
    esp.extend(record(
        b"CONT",
        0,
        0x902,
        &chunk(b"CNTO", &cat(&[&u32_bytes(0xB), &u32_bytes(3)])),
    ));
    fixture.write("Plugin.esp", &esp);
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    let world = &loader.world;
    let book = form_of(&loader, 0x901);
    let data = world.get::<TesBook>(book).unwrap();
    assert_eq!((data.flags_88, data.teaches_89), (0, 0xFF));
    let enchantment = world.get::<TesEnchantableForm>(book).unwrap();
    assert_eq!(enchantment.enchantment_04, FormRef::Raw(0));
    assert_eq!(enchantment.cast_type_0c, 0, "004B53C8 after the components");
    let container = world.get::<TesContainer>(form_of(&loader, 0x902)).unwrap();
    assert_eq!(container.entries_08.len(), 1, "vt+0x18 freed the first");
    assert_eq!(container.entries_08[0].item, FormRef::Raw(0xB));
    // Neither reload appended a second time: the object list holds two.
    assert_eq!(world.resource::<DataHandlerLists>().objects_00.count_00, 2);
}
