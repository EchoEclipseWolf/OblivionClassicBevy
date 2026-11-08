//! Phase 12d: the InitItem and tail arms the shipped data misses: each resolver's warning with its text and what it leaves, the name sorts' one-file gate, the condition arms, the corrupt reference. Synthetic plugins through 0044F3D0 with the real 0044AC20 and the real tail. These stand on the exe and its disassembly, not on a run: P12-RUN-005 reached no warning.

mod common;

use std::path::PathBuf;

use bevy_ecs::prelude::*;
use common::load::{Loader, install_player};
use recomp_game::tes::actor::TesObjectRefr;
use recomp_game::tes::data_handler::{DataHandlerLists, FormsById};
use recomp_game::tes::default_forms::{DefaultFormStatics, TimeGlobals};
use recomp_game::tes::form::FormIdentity;
use recomp_game::tes::form_components::{
    FormRef, TesContainer, TesLeveledList, TesReactionForm, TesSpellList,
};
use recomp_game::tes::forms_environment::TesWaterForm;
use recomp_game::tes::forms_init::RaceStatics;
use recomp_game::tes::forms_items::TesBook;
use recomp_game::tes::forms_region::TesRegion;
use recomp_game::tes::forms_script::Script;
use recomp_game::tes::forms_world::TesWorldSpace;
use recomp_game::tes::tes::GameThreadLocals;

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

fn tes4(master: bool) -> Vec<u8> {
    record(
        b"TES4",
        u32::from(master),
        0,
        &chunk(
            b"HEDR",
            &cat(&[&f32_bytes(0.8), &u32_bytes(100), &u32_bytes(0x800)]),
        ),
    )
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("recomp_phase12d_{}_{name}", std::process::id()));
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
        install_player(&mut loader.world);
        loader.host.real_pre_load = true;
        loader.host.real_tail = true;
        loader
    }

    fn run(&self, records: &[Vec<u8>]) -> Loader {
        let mut esm = tes4(true);
        for one in records {
            esm.extend_from_slice(one);
        }
        self.write("Base.esm", &esm);
        let mut loader = self.loader();
        assert_eq!(loader.run(), Ok(1));
        loader
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn form(loader: &Loader, id: u32) -> Entity {
    loader
        .world
        .resource::<FormsById>()
        .lookup(id)
        .unwrap_or_else(|| panic!("no form {id:08X}"))
}

fn warned(loader: &Loader, want: &str) {
    // g_disableWarningMessages is 1 in P12-RUN-001, so each comes through the recorder's prefix.
    let want = format!("[disabled] {want}");
    assert!(
        loader.warnings.contains(&want),
        "no {want:?} in {:?}",
        loader.warnings
    );
}

fn flags(loader: &Loader, id: u32) -> u32 {
    loader
        .world
        .get::<FormIdentity>(form(loader, id))
        .unwrap()
        .flags_08
}

// 0046E6B0: a miss warns, with the \r\n, and the block leaves; a hit is stored.
#[test]
fn a_reaction_to_nothing_warns_and_leaves() {
    let fixture = Fixture::new("reaction");
    let loader = fixture.run(&[
        record(b"FACT", 0, 0x900, &chunk(b"EDID", &text("Other"))),
        record(
            b"FACT",
            0,
            0x901,
            &cat(&[
                &chunk(b"XNAM", &cat(&[&u32_bytes(0x999), &u32_bytes(5)])),
                &chunk(b"XNAM", &cat(&[&u32_bytes(0x900), &u32_bytes(6)])),
            ]),
        ),
    ]);
    warned(
        &loader,
        "Form (00000999) not found for reaction of pForm (00000901) ''.\r\n",
    );
    let reactions = loader
        .world
        .get::<TesReactionForm>(form(&loader, 0x901))
        .unwrap();
    assert_eq!(reactions.reactions_04.len(), 1);
    assert_eq!(
        reactions.reactions_04[0].faction,
        FormRef::Resolved(Some(form(&loader, 0x900)))
    );
    assert_ne!(flags(&loader, 0x901) & 8, 0, "0046AB80(1)");
}

// 0046D070: a leveled entry that isn't there leaves the list and warns with the owner's ID, its vt+0xD4 being empty.
#[test]
fn a_leveled_entry_to_nothing_warns_and_leaves() {
    let fixture = Fixture::new("leveled");
    let loader = fixture.run(&[record(
        b"LVLI",
        0,
        0x901,
        &chunk(
            b"LVLO",
            &cat(&[&[1, 0, 0, 0], &u32_bytes(0x999), &[1, 0, 0, 0]]),
        ),
    )]);
    warned(
        &loader,
        "Unable to find Leveled Object Form (00000999) for owner object (00000901).",
    );
    let list = loader
        .world
        .get::<TesLeveledList>(form(&loader, 0x901))
        .unwrap();
    assert!(list.entries_04.is_empty());
}

// 00469E90 and 004695D0: a container item that isn't a TESBoundObject warns and leaves; +0x04 bit 0 sets.
#[test]
fn a_container_item_to_nothing_warns_and_leaves() {
    let fixture = Fixture::new("container");
    let loader = fixture.run(&[
        record(b"MISC", 0, 0x900, &[]),
        record(
            b"CONT",
            0,
            0x901,
            &cat(&[
                &chunk(b"CNTO", &cat(&[&u32_bytes(0x900), &u32_bytes(2)])),
                &chunk(b"CNTO", &cat(&[&u32_bytes(0x999), &u32_bytes(3)])),
            ]),
        ),
    ]);
    warned(
        &loader,
        "Unable to find container object (00000999) on owner object (00000901).",
    );
    let container = loader
        .world
        .get::<TesContainer>(form(&loader, 0x901))
        .unwrap();
    assert_eq!(container.flags_04 & 1, 1);
    assert_eq!(container.entries_08.len(), 1);
    assert_eq!(
        container.entries_08[0].item,
        FormRef::Resolved(Some(form(&loader, 0x900)))
    );
}

// 0046FC10: a birth sign's list: a miss warns and leaves, a TESLevSpell moves to +0x0C, another class just leaves. 0046F350 prepends, so the walk meets them last first.
#[test]
fn a_spell_list_sorts_its_entries() {
    let fixture = Fixture::new("spells");
    let loader = fixture.run(&[
        record(b"LVSP", 0, 0x900, &[]),
        record(b"MISC", 0, 0x902, &[]),
        record(
            b"BSGN",
            0,
            0x901,
            &cat(&[
                &chunk(b"SPLO", &u32_bytes(0x900)),
                &chunk(b"SPLO", &u32_bytes(0x902)),
                &chunk(b"SPLO", &u32_bytes(0x999)),
            ]),
        ),
    ]);
    warned(
        &loader,
        "Unable to find spell (00000999) for owner object (00000901).",
    );
    let spells = loader
        .world
        .get::<TesSpellList>(form(&loader, 0x901))
        .unwrap();
    assert!(spells.spells_04.is_empty());
    assert_eq!(
        spells.leveled_0c,
        [FormRef::Resolved(Some(form(&loader, 0x900)))]
    );
}

// 004ED830: POST 42's pointers: a zero slot stays, a hit is a pointer, a miss warns and nulls.
#[test]
fn a_water_resolves_its_three_related_waters() {
    let fixture = Fixture::new("water");
    let loader = fixture.run(&[record(
        b"WATR",
        0,
        0x901,
        &chunk(
            b"GNAM",
            &cat(&[&u32_bytes(0x18), &u32_bytes(0), &u32_bytes(0x999)]),
        ),
    )]);
    warned(
        &loader,
        "Could not find WaterForm (00000999) for water type '' (00000901).",
    );
    let water = loader
        .world
        .get::<TesWaterForm>(form(&loader, 0x901))
        .unwrap();
    assert_eq!(
        water.related_a0,
        [
            FormRef::Resolved(Some(form(&loader, 0x18))),
            FormRef::Raw(0),
            FormRef::Resolved(None)
        ]
    );
}

// 004A3430: only a hit is stored, so a miss keeps the raw word, and nothing warns.
#[test]
fn a_region_keeps_a_raw_word_on_a_miss() {
    let fixture = Fixture::new("region");
    let loader = fixture.run(&[record(
        b"REGN",
        0,
        0x901,
        &chunk(b"WNAM", &u32_bytes(0x999)),
    )]);
    let region = loader.world.get::<TesRegion>(form(&loader, 0x901)).unwrap();
    assert_eq!(region.worldspace_20, FormRef::Raw(0x999));
    assert!(loader.warnings.is_empty(), "{:?}", loader.warnings);
}

// 004B53E0: a taught skill reads as a signed byte + 12, and outside 12..=33 it becomes 0xFF.
#[test]
fn a_book_with_a_bad_skill_teaches_nothing() {
    let fixture = Fixture::new("book");
    let data = |skill: u8| cat(&[&[0, skill], &u32_bytes(1), &f32_bytes(1.0)]);
    let loader = fixture.run(&[
        record(b"BOOK", 0, 0x901, &chunk(b"DATA", &data(0x15))),
        record(b"BOOK", 0, 0x902, &chunk(b"DATA", &data(0x16))),
        record(b"BOOK", 0, 0x903, &chunk(b"DATA", &data(0x80))),
    ]);
    let teaches = |id| {
        loader
            .world
            .get::<TesBook>(form(&loader, id))
            .unwrap()
            .teaches_89
    };
    assert_eq!(teaches(0x901), 0x15, "33");
    assert_eq!(teaches(0x902), 0xFF, "34");
    assert_eq!(teaches(0x903), 0xFF, "a negative byte");
}

// 004FBB60 on a script's own vt+0x6C isn't in a pass; a stage script is reached through its quest, with warnings counted out: a reference that isn't there clears the compiled size.
#[test]
fn a_stage_script_with_a_missing_reference_stops_compiling() {
    let fixture = Fixture::new("stage");
    let header = cat(&[
        &u32_bytes(0),
        &u32_bytes(1),
        &u32_bytes(8),
        &u32_bytes(0),
        &u32_bytes(0),
    ]);
    let loader = fixture.run(&[record(
        b"QUST",
        0,
        0x901,
        &cat(&[
            &chunk(b"INDX", &[1, 0]),
            &chunk(b"QSDT", &[0]),
            &chunk(b"SCHR", &header),
            &chunk(b"SCDA", &[0; 8]),
            &chunk(b"SCRO", &u32_bytes(0x999)),
        ]),
    )]);
    warned(
        &loader,
        "Could not find referenced object (00000999) on Script (00000000) ''.  Script will not be executed.",
    );
    let quest = loader
        .world
        .get::<recomp_game::tes::forms_dialogue::TesQuest>(form(&loader, 0x901))
        .unwrap();
    let script = quest.stages_40[0].items_04[0].script_0c;
    let body = loader.world.get::<Script>(script).unwrap();
    assert!(body.references_40.is_empty());
    assert_eq!(body.header_18[8..12], [0; 4], "004FBC5E +0x20 = 0");
    assert_eq!(
        quest.stages_40[0].items_04[0].quest_68,
        FormRef::Resolved(Some(form(&loader, 0x901)))
    );
    assert_ne!(
        loader.world.get::<FormIdentity>(script).unwrap().flags_08 & 8,
        0
    );
}

fn condition(flags: u8, comparison: u32, function: u16, parameter: u32) -> Vec<u8> {
    cat(&[
        &[flags, 0, 0, 0],
        &u32_bytes(comparison),
        &u32::from(function).to_le_bytes(),
        &u32_bytes(parameter),
        &u32_bytes(0),
        &u32_bytes(0),
    ])
}

// 0056AE20: a function with no handler warns with the owner and file; a form parameter that isn't there warns with vt+0x74; a global comparison that isn't there warns with the owner again.
#[test]
fn condition_items_warn_with_the_original_text() {
    let fixture = Fixture::new("condition");
    let loader = fixture.run(&[record(
        b"QUST",
        0,
        0x901,
        &cat(&[
            // 0x00 MessageBox has no handler; 0x01 GetDistance takes a reference.
            &chunk(b"CTDA", &condition(0, 0, 0, 0)),
            &chunk(b"CTDA", &condition(4, 0x998, 1, 0x999)),
        ]),
    )]);
    warned(
        &loader,
        "Form QUST  (00000901) in file Base.esm contains bad condition item data. Function \"MessageBox\" is not a condition function.",
    );
    warned(
        &loader,
        "Unable to find Function Info TESForm (00000999) in TESConditionItem Parameter Init for QUST Form '' (00000901).",
    );
    warned(
        &loader,
        "Unable to find Value TESGlobal (00000998) in TESConditionItem Init for form:\n\nForm QUST  (00000901).",
    );
}

// 0044F8C4..0044F8F6: with exactly one Loaded file the topics keep their order; 00402860 runs anyway. 0044D610 sorts the classes regardless, with `Default` for no name.
#[test]
fn one_loaded_file_skips_the_topic_sorts() {
    let fixture = Fixture::new("sorts");
    let loader = fixture.run(&[
        record(b"CLAS", 0, 0x901, &chunk(b"FULL", &text("Zed"))),
        record(b"CLAS", 0, 0x902, &chunk(b"FULL", &text("alpha"))),
        record(b"DIAL", 0, 0x903, &chunk(b"FULL", &text("Zed"))),
        record(b"DIAL", 0, 0x904, &chunk(b"FULL", &text("Alpha"))),
    ]);
    let world = &loader.world;
    let lists = world.resource::<DataHandlerLists>();
    let ids = |list: &recomp_core::pointer_list::PointerList<Entity>| -> Vec<u32> {
        list.items()
            .map(|form| world.get::<FormIdentity>(form).unwrap().form_id_0c)
            .collect()
    };
    assert_eq!(ids(&lists.classes_54), [0x902, 0x901], "_stricmp");
    let topics = ids(&lists.topics_7c);
    let at = |id| topics.iter().position(|&one| one == id).unwrap();
    assert!(at(0x903) < at(0x904), "appended, and not sorted");
    let times = world.resource::<TimeGlobals>();
    assert_eq!(
        times.globals_b332e0[0],
        Some(form(&loader, 0x35)),
        "GameYear"
    );
    assert_eq!(times.globals_b332e0[5], Some(form(&loader, 0x3A)));
    assert_eq!(world.resource::<GameThreadLocals>().byte_184, 0);
}

fn interior(id: u32) -> Vec<u8> {
    record(b"CELL", 0, id, &chunk(b"DATA", &[1]))
}

fn name_data(base: u32, position: [f32; 3], rotation: [f32; 3]) -> Vec<u8> {
    let mut data = Vec::new();
    for value in position.iter().chain(&rotation) {
        data.extend(f32_bytes(*value));
    }
    cat(&[&chunk(b"NAME", &u32_bytes(base)), &chunk(b"DATA", &data)])
}

// 004DF87E: a non-finite position warns and takes g_niPoint3Zero; 00426270 under the same flag; the 0x200000 bit clears at the end.
#[test]
fn a_corrupt_reference_takes_zeroes() {
    let fixture = Fixture::new("corrupt");
    let mut esm = tes4(true);
    esm.extend(record(b"STAT", 0, 0x900, &[]));
    esm.extend(interior(0x901));
    esm.extend(record(
        b"REFR",
        0,
        0x902,
        &name_data(0x900, [f32::NAN, 1.0, 2.0], [0.0, f32::INFINITY, 0.0]),
    ));
    fixture.write("Base.esm", &esm);
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    warned(
        &loader,
        "Corrupt location found on reference, setting to (0, 0, 0).",
    );
    warned(
        &loader,
        "Corrupt angle found on reference, setting to (0, 0, 0).",
    );
    let reference = loader
        .world
        .get::<TesObjectRefr>(form(&loader, 0x902))
        .unwrap();
    assert_eq!(reference.position_2c, [0.0; 3]);
    assert_eq!(reference.rotation_20, [0.0; 3]);
    assert_ne!(flags(&loader, 0x902) & 8, 0);
    assert_eq!(flags(&loader, 0x902) & 0x20_0000, 0);
}

// 0044AC20's statics, and 0052B6A0's models from a GameSetting's text, once.
#[test]
fn the_statics_are_the_forms_at_their_ids() {
    let fixture = Fixture::new("statics");
    let loader = fixture.run(&[record(b"RACE", 0, 0x901, &[])]);
    let statics = loader.world.resource::<DefaultFormStatics>();
    assert_eq!(statics.north_marker_b35eb8, Some(form(&loader, 0x03)));
    assert_eq!(statics.prison_marker_b35ebc, Some(form(&loader, 0x04)));
    assert_eq!(statics.jail_shoes_b33ab8, Some(form(&loader, 0x16)));
    assert_eq!(statics.magic_sounds_b33560.len(), 18);
    assert_eq!(statics.footstep_sounds_b36218.len(), 25);
    // 00518000 stores 0x22B at B36220 and 0x213 at B3621C, 0x229 at B36274: by address, not call order.
    assert_eq!(
        statics.footstep_sounds_b36218[1],
        Some(form(&loader, 0x213))
    );
    assert_eq!(
        statics.footstep_sounds_b36218[2],
        Some(form(&loader, 0x22B))
    );
    assert_eq!(
        statics.footstep_sounds_b36218[23],
        Some(form(&loader, 0x229))
    );
    assert_eq!(statics.sky_climate_calls, 1);
    let race = loader.world.resource::<RaceStatics>();
    assert!(race.set_b3630c);
    assert_eq!(race.model_paths.len(), 13);
    // No WRLD holds a default OFST with no master: 004F2EAA needs one, and Tamriel's is default-built here.
    assert!(
        loader
            .world
            .get::<TesWorldSpace>(form(&loader, 0x3C))
            .unwrap()
            .offsets_a8
            .is_none()
    );
}
