//! Phase 12c: synthetic plugins for the DIAL, INFO and QUST arms the shipped data doesn't take, or takes where no row can see them. Each expected value comes from the disassembly named beside it. Host tests earn no validation bonus.

mod common;

use std::path::PathBuf;

use bevy_ecs::prelude::*;
use common::load::{Loader, pre_load_default_globals};
use oblivion_core::error::GameError;
use oblivion_sim::tes::data_handler::{DataHandlerLists, EditorIdMap, FormsById, TesTexture};
use oblivion_sim::tes::form::FormIdentity;
use oblivion_sim::tes::form_components::{FormRef, TesFullName};
use oblivion_sim::tes::forms_dialogue::{DialogueGlobals, TesQuest, TesTopic, TesTopicInfo};
use oblivion_sim::tes::forms_script::Script;

fn u32_bytes(value: u32) -> Vec<u8> {
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

fn tes4(master: bool, masters: &[&str]) -> Vec<u8> {
    let mut payload = chunk(
        b"HEDR",
        &cat(&[&1.0_f32.to_le_bytes(), &u32_bytes(8), &u32_bytes(0x800)]),
    );
    for name in masters {
        payload.extend(chunk(b"MAST", &text(name)));
        payload.extend(chunk(b"DATA", &0_u64.to_le_bytes()));
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
            std::env::temp_dir().join(format!("recomp_phase12c_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Data")).unwrap();
        Self { root }
    }

    fn write(&self, name: &str, records: &[Vec<u8>], masters: &[&str]) {
        let mut bytes = tes4(masters.is_empty(), masters);
        for one in records {
            bytes.extend_from_slice(one);
        }
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

fn form(loader: &Loader, id: u32) -> Entity {
    loader.world.resource::<FormsById>().lookup(id).unwrap()
}

fn info(loader: &Loader, id: u32) -> &TesTopicInfo {
    loader.world.get::<TesTopicInfo>(form(loader, id)).unwrap()
}

// IDA: N/A an entry's infos, in array order, by form ID.
fn order(loader: &Loader, topic: u32, entry: usize) -> Vec<Option<u32>> {
    let world = &loader.world;
    let topic = world.get::<TesTopic>(form(loader, topic)).unwrap();
    let infos = &topic.entries_28[entry].infos_04;
    infos.data_04[..infos.size_0c as usize]
        .iter()
        .map(|slot| slot.map(|one| world.get::<FormIdentity>(one).unwrap().form_id_0c))
        .collect()
}

fn dial(id: u32, quest: u32) -> Vec<u8> {
    record(b"DIAL", 0, id, &chunk(b"QSTI", &u32_bytes(quest)))
}

fn plain_info(id: u32, quest: u32, previous: Option<u32>) -> Vec<u8> {
    let mut payload = chunk(b"QSTI", &u32_bytes(quest));
    if let Some(previous) = previous {
        payload.extend(chunk(b"PNAM", &u32_bytes(previous)));
    }
    record(b"INFO", 0, id, &payload)
}

// 0052FE20: no PNAM takes the slot past the last non-null one. 0052FE5D: PNAM 0 goes to the front. 0052FD9F: any other goes after it. 0052FDF2: a miss warns and goes to the front, with the topic's ID where the text says TopicInfo (`mov ebx, [ebp+0Ch]` at 0052FDFD). 0052F650 renumbers.
#[test]
fn an_info_goes_where_its_pnam_says() {
    let fixture = Fixture::new("pnam");
    fixture.write(
        "Base.esm",
        &[
            dial(0x900, 0x950),
            plain_info(0x901, 0x950, None),
            plain_info(0x902, 0x950, None),
            plain_info(0x903, 0x950, Some(0)),
            plain_info(0x904, 0x950, Some(0x901)),
            plain_info(0x905, 0x950, Some(0xDEAD)),
        ],
        &[],
    );
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    assert_eq!(
        order(&loader, 0x900, 0),
        [
            Some(0x905),
            Some(0x903),
            Some(0x901),
            Some(0x904),
            Some(0x902)
        ]
    );
    for (index, id) in [0x905, 0x903, 0x901, 0x904, 0x902].into_iter().enumerate() {
        assert_eq!(info(&loader, id).index_20, index as u16, "{id:X}");
    }
    assert_eq!(
        loader.warnings,
        [
            "[disabled] Could not find previous info (0000DEAD) for TopicInfo (00000900) in Topic \"\" (00000900)."
        ]
    );
    let topic = loader.world.get::<TesTopic>(form(&loader, 0x900)).unwrap();
    let infos = &topic.entries_28[0].infos_04;
    // 005302BC 00452910(+0x0C + 100) and +0x18 = 10, and A554D4 from 0052FCE0. No slot was ever empty at 005A56F0, so the capacity stays.
    assert_eq!(
        (
            infos.original_vtable,
            infos.capacity_08,
            infos.size_0c,
            infos.count_10,
            infos.grow_14
        ),
        (0x00A5_54D4, 100, 5, 5, 10)
    );
    assert_eq!(topic.entries_28[0].quest_00, FormRef::Raw(0x950));
    assert_eq!(topic.entries_28[0].quest_1c, FormRef::Raw(0));
    assert_eq!(topic.entries_28[0].resolved_20, 0);
}

// 0052FC40: an entry per quest, in order of first sight, whose QSTI grows by 100 each time; a zero quest makes none. 0052F5D0(previous, 1) looks through every entry, so +0x20 can come from another quest's array.
#[test]
fn a_topic_keeps_an_entry_per_quest() {
    let fixture = Fixture::new("entries");
    fixture.write(
        "Base.esm",
        &[
            record(
                b"DIAL",
                0,
                0x900,
                &cat(&[
                    &chunk(b"QSTI", &u32_bytes(0x950)),
                    &chunk(b"QSTI", &u32_bytes(0x951)),
                    &chunk(b"QSTI", &u32_bytes(0x950)),
                    &chunk(b"QSTI", &u32_bytes(0)),
                    &chunk(b"EDID", &text("Topic")),
                    &chunk(b"FULL", &text("A Topic")),
                    &chunk(b"DATA", &[4]),
                    &chunk(b"XIDX", &u32_bytes(0x77)),
                ]),
            ),
            plain_info(0x901, 0x950, None),
            plain_info(0x902, 0x950, None),
            plain_info(0x903, 0x951, Some(0x902)),
        ],
        &[],
    );
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    let topic_form = form(&loader, 0x900);
    let topic = loader.world.get::<TesTopic>(topic_form).unwrap();
    assert_eq!(topic.entries_28.len(), 2);
    assert_eq!(topic.entries_28[0].infos_04.capacity_08, 200);
    assert_eq!((topic.type_24, topic.index_30), (4, 0x77));
    assert_eq!(topic.editor_34.data_00.as_deref(), Some(&b"Topic"[..]));
    assert_eq!(
        loader
            .world
            .get::<TesFullName>(topic_form)
            .unwrap()
            .name_04
            .data_00
            .as_deref(),
        Some(&b"A Topic"[..])
    );
    // 0x902 sits at 1 of the other quest's array, so 0x903 goes to 2 of its own: 0052F3C0's past-the-end arm, then 005A56F0 closes the gap.
    assert_eq!(order(&loader, 0x900, 1), [Some(0x903)]);
    assert_eq!(info(&loader, 0x903).index_20, 0);
    assert!(
        loader
            .world
            .resource::<EditorIdMap>()
            .lookup(b"topic")
            .is_some(),
        "0044F296 maps a topic's editor ID, _stricmp"
    );
    assert!(loader.warnings.is_empty());
}

// 0053104D `cmp byte [esi], 7; jl`: signed, so 0x80 isn't deleted. 00531060 type 1 makes the block. 005311A0 flag 0x20 skips 0052FD20. 0052FD24 a zero quest answers 0, and 005311C4 warns.
#[test]
fn an_infos_type_byte_is_signed_and_a_zero_quest_warns() {
    let fixture = Fixture::new("data");
    let data = |bytes: [u8; 3]| chunk(b"DATA", &bytes);
    fixture.write(
        "Base.esm",
        &[
            dial(0x900, 0x950),
            record(
                b"INFO",
                0,
                0x901,
                &cat(&[&chunk(b"QSTI", &u32_bytes(0x950)), &data([0x80, 2, 3])]),
            ),
            record(
                b"INFO",
                0,
                0x902,
                &cat(&[&chunk(b"QSTI", &u32_bytes(0x950)), &data([7, 0, 0])]),
            ),
            record(
                b"INFO",
                0,
                0x903,
                &cat(&[&chunk(b"QSTI", &u32_bytes(0x950)), &data([1, 0, 0])]),
            ),
            record(b"INFO", 0, 0x904, &data([0, 0, 0])),
        ],
        &[],
    );
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    let flags = |loader: &Loader, id| {
        loader
            .world
            .get::<FormIdentity>(form(loader, id))
            .unwrap()
            .flags_08
    };
    let first = info(&loader, 0x901);
    assert_eq!(
        (first.type_23, first.next_speaker_24, first.flags_25),
        (0x80, 2, 3)
    );
    assert_eq!(flags(&loader, 0x901) & 0x20, 0);
    assert_eq!(first.index_20, 0);
    assert_eq!(flags(&loader, 0x902) & 0x20, 0x20);
    assert_eq!(info(&loader, 0x902).index_20, 0xFFFF);
    assert!(info(&loader, 0x902).choices_30.is_none());
    assert!(info(&loader, 0x903).choices_30.is_some());
    assert_eq!(order(&loader, 0x900, 0), [Some(0x901), Some(0x903)]);
    assert_eq!(
        loader.warnings,
        [
            "[disabled] Unable to insert topic info (00000904) into topic '' (00000900), quest (00000000)."
        ]
    );
}

// 005310A3 and 00531113 each make the block before reading. 0067B1E0 drops a zero. The record offset comes from +0x25C.
#[test]
fn an_infos_lists_keep_raw_ids() {
    let fixture = Fixture::new("lists");
    fixture.write(
        "Base.esm",
        &[
            dial(0x900, 0x950),
            record(
                b"INFO",
                0,
                0x901,
                &cat(&[
                    &chunk(b"QSTI", &u32_bytes(0x950)),
                    &chunk(b"NAME", &u32_bytes(0xA1)),
                    &chunk(b"NAME", &u32_bytes(0)),
                    &chunk(b"NAME", &u32_bytes(0xA2)),
                    &chunk(b"TCLT", &u32_bytes(0)),
                    &chunk(b"TCLT", &u32_bytes(0xB1)),
                    &chunk(b"TCLF", &u32_bytes(0xC1)),
                    &chunk(b"CTDA", &[0x60; 0x18]),
                    &chunk(b"CTDT", &[0x70; 0x14]),
                ]),
            ),
        ],
        &[],
    );
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    let one = info(&loader, 0x901);
    assert_eq!(one.names_28, [FormRef::Raw(0xA1), FormRef::Raw(0xA2)]);
    let block = one.choices_30.as_ref().unwrap();
    assert_eq!(block.choices_08, [FormRef::Raw(0xB1)]);
    assert_eq!(block.link_from_00, [FormRef::Raw(0xC1)]);
    assert_ne!(one.record_offset_34, 0);
    assert_eq!(one.conditions_18.items.len(), 2);
    assert_eq!(one.conditions_18.items[0].data_00[0x14], 0);
    // 0056A9B2 the chunk-was-CTDT byte, over nothing here: the 0x14-byte chunk stops before it.
    assert_eq!(one.conditions_18.items[1].data_00[0x14], 1);
    assert_eq!(one.conditions_18.items[1].data_00[0x13], 0x70);
}

// 00530DB0, with 00530560 after it: a placed info leaves its topic, and the conditions, NAME list and block go; the type word goes too, including +0x24. 00530E31 runs 004FC7A0 onto 00B36530.
#[test]
fn an_override_resets_and_reinserts() {
    let fixture = Fixture::new("override");
    fixture.write(
        "Base.esm",
        &[
            dial(0x900, 0x950),
            record(
                b"INFO",
                0,
                0x901,
                &cat(&[
                    &chunk(b"QSTI", &u32_bytes(0x950)),
                    &chunk(b"DATA", &[3, 9, 5]),
                    &chunk(b"NAME", &u32_bytes(0xA1)),
                    &chunk(b"TCLF", &u32_bytes(0xC1)),
                    &chunk(b"CTDA", &[0x60; 0x18]),
                ]),
            ),
            plain_info(0x902, 0x950, None),
            plain_info(0x903, 0x950, None),
        ],
        &[],
    );
    fixture.write(
        "Over.esp",
        &[
            dial(0x900, 0x950),
            plain_info(0x901, 0x950, Some(0x903)),
            // Flag 0x20 at 0044DE05 skips the pair, and 005311A6 the insertion.
            record(b"INFO", 0x20, 0x902, &chunk(b"QSTI", &u32_bytes(0x950))),
        ],
        &["Base.esm"],
    );
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    let one = info(&loader, 0x901);
    assert_eq!(
        (one.type_23, one.next_speaker_24, one.flags_25),
        (0, 0, 0),
        "00530562 clears the word at +0x23"
    );
    assert!(one.names_28.is_empty());
    assert!(one.choices_30.is_none());
    assert!(one.conditions_18.items.is_empty());
    assert_eq!(
        order(&loader, 0x900, 0),
        [Some(0x902), Some(0x903), Some(0x901)]
    );
    assert_eq!(info(&loader, 0x901).index_20, 2);
    // The reload added no second topic or entry.
    let lists = loader.world.resource::<DataHandlerLists>();
    assert_eq!(lists.topics_7c.items().count(), 1);
    let globals = loader.world.resource::<DialogueGlobals>();
    let result = globals.result_script_b36530.unwrap();
    assert_eq!(globals.response_info_b36528, 0);
    let identity = loader.world.get::<FormIdentity>(result).unwrap();
    assert_eq!((identity.form_type_04, identity.flags_08), (0x0D, 8));
    let mut scripts = 0;
    let world = &loader.world;
    for entity in world.iter_entities() {
        if entity.contains::<Script>() {
            scripts += 1;
        }
    }
    assert_eq!(scripts, 1, "00530E42 destroys the stack Script");
}

// 0044F1C2 `mov [eax+0x34], edi`: past the sentinels, a current info takes the ID, before the world and cell tests. -2 reaches 00738500, which only returns, and ends the file.
#[test]
fn a_toft_record_sets_the_current_infos_offset() {
    let fixture = Fixture::new("toft");
    fixture.write(
        "Base.esm",
        &[
            dial(0x900, 0x950),
            plain_info(0x901, 0x950, None),
            plain_info(0x902, 0x950, None),
            record(b"TOFT", 0, 0x1234, &[]),
            record(b"TOFT", 0, 0x1235, &[]),
            record(b"TOFT", 0, 0xFFFF_FFFE, &[]),
            plain_info(0x903, 0x950, None),
        ],
        &[],
    );
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    assert_ne!(info(&loader, 0x901).record_offset_34, 0x1235);
    assert_eq!(info(&loader, 0x902).record_offset_34, 0x1235);
    assert_eq!(
        loader.world.resource::<FormsById>().lookup(0x903),
        None,
        "-2 ended the file"
    );
}

// 005311B7 and 005311C6 both read the null 00B3650C.
#[test]
fn an_info_with_no_topic_is_a_host_stop() {
    let fixture = Fixture::new("notopic");
    fixture.write("Base.esm", &[plain_info(0x901, 0x950, None)], &[]);
    let mut loader = fixture.loader();
    assert_eq!(
        loader.run(),
        Err(GameError::unclassified(
            0x005311B7,
            "005311B7: 0052FD20 on a null 00B3650C faults"
        ))
    );
}

fn quest_records() -> Vec<u8> {
    let mut header = [0_u8; 0x14];
    header[0x10] = 9;
    cat(&[
        &chunk(b"EDID", &text("MQ01")),
        &chunk(b"FULL", &text("Deliverance")),
        &chunk(b"ICON", &text("q.dds")),
        &chunk(b"DATA", &[6, 7]),
        // +0x50: no item, no target.
        &chunk(b"CTDA", &[1; 0x18]),
        // 0052A78C: no group yet, so nothing.
        &chunk(b"QSDT", &[0xEE]),
        // A short INDX reads 2 bytes straight in.
        &chunk(b"INDX", &[5, 1]),
        &chunk(b"QSDT", &[3]),
        // 0052A719 sets the byte and never reads the chunk.
        &chunk(b"CNAM", &text("log")),
        &chunk(b"CTDA", &[2; 0x18]),
        &chunk(b"SCHR", &header),
        &chunk(b"SCDA", &[1, 2, 3]),
        &chunk(b"SCRO", &u32_bytes(0x1234)),
        // An 8-byte INDX takes bytes 0 and 1 and leaves `item` alone.
        &chunk(b"INDX", &[6, 2, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA]),
        &chunk(b"CTDT", &[3; 0x18]),
        // The index byte keeps counting across groups.
        &chunk(b"QSDT", &[4]),
        // 0052A640 clears group and item.
        &chunk(b"QSTA", &cat(&[&u32_bytes(0x4321), &[0x11, 0, 0, 0]])),
        &chunk(b"CTDA", &[4; 0x18]),
        &chunk(b"CNAM", &text("dropped")),
        &chunk(b"SCHR", &[0xFF; 0x14]),
        &chunk(b"QSDT", &[0xEE]),
    ])
}

// 0052A570 with 0052AC60, 0052ACC0, 0052B1F0 and 0052B3F0.
#[test]
fn a_quest_loads_its_stages_and_targets() {
    let fixture = Fixture::new("quest");
    fixture.write(
        "Base.esm",
        &[record(b"QUST", 0, 0x950, &quest_records())],
        &[],
    );
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    let world = &loader.world;
    let quest_form = form(&loader, 0x950);
    let quest = world.get::<TesQuest>(quest_form).unwrap();
    assert_eq!((quest.flags_3c, quest.priority_3d), (6, 7));
    assert_eq!(quest.editor_60.data_00.as_deref(), Some(&b"MQ01"[..]));
    assert_eq!(
        world
            .get::<TesTexture>(quest_form)
            .unwrap()
            .path_04
            .data_00
            .as_deref(),
        Some(&b"q.dds"[..])
    );
    assert_eq!(
        world.get::<FormIdentity>(quest_form).unwrap().flags_08 & 8,
        0
    );
    assert_eq!(quest.conditions_50.items.len(), 1);
    assert_eq!(quest.conditions_50.items[0].data_00[0], 1);
    assert_eq!(quest.stages_40.len(), 2);
    let (first, second) = (&quest.stages_40[0], &quest.stages_40[1]);
    assert_eq!((first.index_00, first.flags_01), (5, 1));
    assert_eq!((second.index_00, second.flags_01), (6, 2));
    assert_eq!(first.items_04.len(), 1);
    let item = &first.items_04[0];
    assert_eq!((item.flags_00, item.index_60, item.log_61), (3, 0, 1));
    assert_ne!(item.record_offset_5c, 0);
    // Both conditions after the first QSDT went to it, the CTDT past the second INDX too.
    let bytes: Vec<u8> = item
        .conditions_04
        .items
        .iter()
        .map(|c| c.data_00[0])
        .collect();
    assert_eq!(bytes, [2, 3]);
    let script = world.get::<Script>(item.script_0c).unwrap();
    assert_eq!(script.header_18[0x10], 9, "SCHR replaced 0052AD18's 1");
    assert_eq!(script.header_18[8..12], 3_u32.to_le_bytes(), "004F9E0F");
    assert_eq!(script.data_30.as_deref(), Some(&[1, 2, 3][..]));
    assert_eq!(script.references_40.len(), 1);
    assert_eq!(script.references_40[0].form_08, 0x1234);
    let identity = world.get::<FormIdentity>(item.script_0c).unwrap();
    assert_eq!(
        (
            identity.form_type_04,
            identity.form_id_0c,
            identity.flags_08
        ),
        (0x0D, 0, 0x4000),
        "0046AB80(0), then 0046B590; the TLS byte left the ID 0"
    );
    let later = &second.items_04[0];
    assert_eq!((later.flags_00, later.index_60, later.log_61), (4, 1, 0));
    assert_eq!(
        world.get::<Script>(later.script_0c).unwrap().header_18[0x10],
        1,
        "0052AD18, with the SCHR past QSTA dropped"
    );
    assert_eq!(quest.targets_48.len(), 1);
    let target = &quest.targets_48[0];
    assert_eq!(target.target_0c, FormRef::Raw(0x4321));
    assert_eq!(target.flags_00, 0x11);
    assert_eq!(target.conditions_04.items[0].data_00[0], 4);
    assert_eq!(
        world
            .resource::<DataHandlerLists>()
            .quests_84
            .items()
            .count(),
        1
    );
    assert!(world.resource::<EditorIdMap>().lookup(b"mq01").is_some());
}

// 0052A520 frees the stages through 0052AD40 and 004FC980, the targets and conditions; 0052A500 sets +0x3C bit 0 and keeps +0x3D. The components go through 0046B170 and 0046B090.
#[test]
fn a_quest_override_frees_what_it_held() {
    let fixture = Fixture::new("questreload");
    fixture.write(
        "Base.esm",
        &[record(b"QUST", 0, 0x950, &quest_records())],
        &[],
    );
    fixture.write(
        "Over.esp",
        &[record(b"QUST", 0, 0x950, &chunk(b"EDID", &text("MQ02")))],
        &["Base.esm"],
    );
    let mut loader = fixture.loader();
    assert_eq!(loader.run(), Ok(1));
    let world = &loader.world;
    let quest_form = form(&loader, 0x950);
    let quest = world.get::<TesQuest>(quest_form).unwrap();
    assert!(quest.stages_40.is_empty() && quest.targets_48.is_empty());
    assert!(quest.conditions_50.items.is_empty());
    assert_eq!((quest.flags_3c, quest.priority_3d), (7, 7));
    assert_eq!(quest.editor_60.data_00.as_deref(), Some(&b"MQ02"[..]));
    assert_eq!(
        world
            .get::<TesFullName>(quest_form)
            .unwrap()
            .name_04
            .data_00,
        None
    );
    assert_eq!(
        world.get::<TesTexture>(quest_form).unwrap().path_04.data_00,
        None
    );
    let mut scripts = 0;
    for entity in world.iter_entities() {
        if entity.contains::<Script>() {
            scripts += 1;
        }
    }
    assert_eq!(scripts, 0, "004FC980 took both stage Scripts");
    assert_eq!(
        world
            .resource::<DataHandlerLists>()
            .quests_84
            .items()
            .count(),
        1
    );
}

#[test]
fn a_quests_overruns_are_host_stops() {
    for (name, payload, stop) in [
        (
            "schr",
            cat(&[
                &chunk(b"INDX", &[0, 0]),
                &chunk(b"QSDT", &[0]),
                &chunk(b"SCHR", &[0; 0x15]),
            ]),
            "0052B2BA: an SCHR chunk past 0x14 bytes overruns +0x2C",
        ),
        (
            "qsta",
            chunk(b"QSTA", &[0; 9]),
            "0052B41F: a QSTA past 8 bytes overruns the stack",
        ),
        (
            "scro",
            cat(&[
                &chunk(b"INDX", &[0, 0]),
                &chunk(b"QSDT", &[0]),
                &chunk(b"SCRO", &[0; 2]),
            ]),
            "0052B26D: an SCRO under 4 bytes stores the item's own address",
        ),
    ] {
        let fixture = Fixture::new(name);
        fixture.write("Base.esm", &[record(b"QUST", 0, 0x950, &payload)], &[]);
        let mut loader = fixture.loader();
        assert_eq!(
            loader.run(),
            Err(GameError::from_legacy_message(stop)),
            "{name}"
        );
    }
}

// 0052FE90 over the tables at 00B111B8: each group prepends, so the last row of group 6 leads. 0052F480's _stricmp keeps the default name against a record's other case, and the list arm takes the existing form (0044E291), with no second list entry.
#[test]
fn the_default_topics_and_a_record_over_one() {
    let fixture = Fixture::new("defaults");
    fixture.write(
        "Base.esm",
        &[
            record(
                b"DIAL",
                0,
                0xC8,
                &cat(&[
                    &chunk(b"EDID", &text("greeting")),
                    &chunk(b"FULL", &text("Greetings")),
                ]),
            ),
            record(b"DIAL", 0, 0x901, &chunk(b"EDID", &text("Later"))),
        ],
        &[],
    );
    let mut loader = fixture.loader();
    loader.host.pre_load = pre_load_default_globals;
    assert_eq!(loader.run(), Ok(1));
    let world = &loader.world;
    let ids: Vec<u32> = world
        .resource::<DataHandlerLists>()
        .topics_7c
        .items()
        .map(|one| world.get::<FormIdentity>(one).unwrap().form_id_0c)
        .collect();
    assert_eq!(ids.len(), 86);
    assert_eq!(ids[..3], [0x11C, 0x11B, 0x11A]);
    assert_eq!(ids[84..], [0xC8, 0x901], "the DIAL arm appends");
    let greeting = form(&loader, 0xC8);
    let topic = world.get::<TesTopic>(greeting).unwrap();
    assert_eq!(topic.editor_34.data_00.as_deref(), Some(&b"GREETING"[..]));
    assert_eq!(topic.type_24, 0);
    assert_eq!(
        world
            .get::<TesFullName>(greeting)
            .unwrap()
            .name_04
            .data_00
            .as_deref(),
        Some(&b"Greetings"[..])
    );
    assert_eq!(
        world.get::<TesTopic>(form(&loader, 0x11C)).unwrap().type_24,
        6
    );
    assert_eq!(
        world.resource::<EditorIdMap>().lookup(b"TimeToGo"),
        Some(form(&loader, 0x11C))
    );
}
