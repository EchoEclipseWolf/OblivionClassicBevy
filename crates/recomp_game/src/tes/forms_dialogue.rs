//! Phase 12c: the dialogue forms. TESTopic (DIAL 0x39, 0052FB40 / 005301E0) with its TopicInfoArray entries (0052FC40), TESTopicInfo (INFO 0x3A, 00530AE0 / 00530FB0) with the insertion into the current topic (0052FD20) and the override reset 00530DB0, TESQuest (QUST 0x3B, 0052AA00 / 0052A570) with its stage groups, stage items and targets, the default topics 0052FE90, and the 0044DCF0 arms 0044E474 and 0044E4C9.
//!
//! No LoadForm here reaches 0056AE20 or the condition function tables: 0056A7B0 only stores bytes, and the tables wait for InitItem in 12d. Evidence: objdump of each address named here; P12-RUN-001's COUNT, LIST, AC20, AC20RAW, SAMPLE and DATA rows.

use bevy_ecs::prelude::*;
use recomp_core::crt;
use recomp_core::reader::StreamError;

use super::data_handler::{
    DataHandlerLists, EditorIdMap, TesTexture, construct_tes_form, set_form_id,
};
use super::data_load::{DataLoadGlobals, initialize_form_from_current_record, load_form_from_file};
use super::form::{FORM_FLAG_PARTIAL, FormIdentity, OriginalVtable};
use super::form_arms::{
    FormClass, FormLoadCx, default_editor_id, editor_id_of, reset_14_components,
    reset_18_components,
};
use super::form_components::{
    ComponentSlot, DataChunkTails, FormRef, TesFullName, TesScriptableForm, load_data_chunk,
    load_full_name, load_texture, reset_form_components,
};
use super::forms_ai::{NiFormArray, TesCondition, load_condition};
use super::forms_items::set_change_flags;
use super::forms_misc::set_initialized;
use super::forms_script::{
    Script, ScriptEventList, ScriptReference, construct_script_in_place, copy_script_from,
    destroy_script_in_place, last_mod_reference, latin1, load_script_reference, map_file_form_id,
    mark_temporary, set_compiled_data,
};
use super::tes::GameThreadLocals;
use super::tes_file::BsStringT;

/// IDA: the form types of this family.
pub const FORM_TYPE_TOPIC: u8 = 0x39;
pub const FORM_TYPE_TOPIC_INFO: u8 = 0x3A;
pub const FORM_TYPE_QUEST: u8 = 0x3B;

/// IDA: the vtables, from the constructors named below.
pub const TES_TOPIC_VTABLE: u32 = 0x00A5_555C;
pub const TES_TOPIC_FULL_NAME_VTABLE: u32 = 0x00A5_5548;
pub const TES_TOPIC_INFO_VTABLE: u32 = 0x00A5_568C;
pub const TOPIC_INFO_ARRAY_VTABLE: u32 = 0x00A5_54D4;
pub const TES_QUEST_VTABLE: u32 = 0x00A5_41EC;
pub const TES_QUEST_SCRIPTABLE_VTABLE: u32 = 0x00A5_41D8;
pub const TES_QUEST_ICON_VTABLE: u32 = 0x00A5_41BC;
pub const TES_QUEST_FULL_NAME_VTABLE: u32 = 0x00A5_41A8;

// IDA: the chunks these LoadForms dispatch on.
const EDID: u32 = 0x4449_4445;
const FULL: u32 = 0x4C4C_5546;
const DATA: u32 = 0x4154_4144;
const QSTI: u32 = 0x4954_5351;
const XIDX: u32 = 0x5844_4958;
const PNAM: u32 = 0x4D41_4E50;
const NAME: u32 = 0x454D_414E;
const TCLT: u32 = 0x544C_4354;
const TCLF: u32 = 0x464C_4354;
const CTDA: u32 = 0x4144_5443;
const CTDT: u32 = 0x5444_5443;
const INDX: u32 = 0x5844_4E49;
const QSDT: u32 = 0x5444_5351;
const CNAM: u32 = 0x4D41_4E43;
const SCHR: u32 = 0x5248_4353;
const SCDA: u32 = 0x4144_4353;
const SCRO: u32 = 0x4F52_4353;
const SCRI: u32 = 0x4952_4353;
const ICON: u32 = 0x4E4F_4349;
const QSTA: u32 = 0x4154_5351;

// IDA: 0052FD6E, 00530DE5 and 00530B3E: +0x20's "no slot".
const NO_INDEX: u16 = 0xFFFF;
// IDA: 00530FEA the local `previous` starts here: no PNAM.
const NO_PREVIOUS: u32 = 0xFFFF_FFFF;

/// IDA: the dialogue statics these functions touch: 00B3650C the current topic (DIAL LoadForm), 00B36530 the static result Script (009E4220 constructs it, 00A1C540 destroys it), 00B36528 and 00B3652C the response and script caches (runtime only), 00B36580 / 00B36584 the cached response list 00530620 drains, and 00B362FC with the BSStringT 00B36300, which a stage item's destructor 0052AD40 clears.
#[derive(Resource, Debug, Default)]
pub struct DialogueGlobals {
    pub current_topic_b3650c: Option<Entity>,
    pub result_script_b36530: Option<Entity>,
    pub response_info_b36528: u32,
    pub script_info_b3652c: u32,
    /// The entries of the 00B36580 list: filled only by 00530C40 at runtime, and a nonzero count is a host stop.
    pub cached_responses_b36580: u32,
    pub stage_field_b362fc: u32,
    pub stage_text_b36300: BsStringT,
}

// ---- TESTopic ----

/// IDA: 0052FC40 the TopicInfoArray entry (0x24): +0x00 the quest, a raw form ID until 12d's InitItem resolves it and sets +0x20, +0x04 the NiTArray of infos (vtable A554D4), +0x1C a second quest slot, +0x20 the resolved byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopicInfoArray {
    pub quest_00: FormRef,
    pub infos_04: NiFormArray,
    pub quest_1c: FormRef,
    pub resolved_20: u8,
}

/// IDA: 0052FB40 TESTopic (0x3C): TESFullName +0x18 (`TesFullName` on the same entity), +0x24 the type (the constructor's argument, then DATA; +0x25..+0x27 stay unset), +0x28 the entry list, +0x30 XIDX, +0x34 the editor ID.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesTopic {
    pub type_24: u8,
    pub entries_28: Vec<TopicInfoArray>,
    pub index_30: u32,
    pub editor_34: BsStringT,
}

// IDA: 0052FB40 TESTopic_Constructor(type): 0046C460; +0x18 TESFullName inline (A322A0, +0x1C, +0x20, +0x22); the two vtables; +0x28, +0x2C, +0x34, +0x38, +0x3A, +0x30 = 0; +0x24 = type; +0x04 = 0x39. It has no 0046C2E0. 0052FEDE inlines the same stores. Completion: 100% of the store order.
fn construct_topic_of_type(world: &mut World, type_24: u8) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        OriginalVtable(TES_TOPIC_VTABLE),
        TesFullName {
            original_vtable: TES_TOPIC_FULL_NAME_VTABLE,
            name_04: BsStringT::default(),
        },
        TesTopic {
            type_24,
            ..TesTopic::default()
        },
    ));
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_TOPIC;
    Ok(form)
}

// IDA: 0044E494 0052FB40(0). 00447DF0 has no DIAL arm.
fn construct_topic(world: &mut World) -> Result<Entity, StreamError> {
    construct_topic_of_type(world, 0)
}

// IDA: 0052FB90 TESTopic vt+0xD4: +0x34 or `Default`. Completion: 100%.
fn topic_editor_id(world: &World, form: Entity) -> Vec<u8> {
    world
        .get::<TesTopic>(form)
        .expect("a topic")
        .editor_34
        .data_00
        .clone()
        .unwrap_or_default()
}

// IDA: 0052F480 (TESTopic) and 0052A960 (TESQuest) vt+0xD8: a stack copy of the name; a set string that _stricmp matches keeps it and answers 0; otherwise 004028D0(name, 0). The answer has no reader at any call here. Completion: 100%.
fn set_editor_string(target: &mut BsStringT, name: &[u8]) {
    if let Some(current) = &target.data_00
        && crt::stricmp(current, name) == 0
    {
        return;
    }
    target.set(Some(name), 0);
}

// IDA: N/A 0052FC40's comparison for one entry. A resolved entry compares the quests' form IDs, +0x00 then +0x1C; a raw one the slots themselves.
fn entry_matches(world: &World, entry: &TopicInfoArray, quest: u32) -> bool {
    let matches = |slot: FormRef| match slot {
        FormRef::Raw(id) => id == quest,
        FormRef::Resolved(form) => form.is_some_and(|form| {
            world.get::<FormIdentity>(form).expect("a form").form_id_0c == quest
        }),
    };
    matches(entry.quest_00) || matches(entry.quest_1c)
}

/// IDA: 0052FC40 TESTopic_FindOrCreateInfoArray(quest, 0): a zero quest answers null; the first entry that matches; else a new FormHeap(0x24) entry (+0x18 = 1, the array's vtable, +0x00 = quest), appended by 0067B1E0. Answers the entry's index. Completion: 100%.
pub fn find_or_create_entry(world: &mut World, topic: Entity, quest: u32) -> Option<usize> {
    if quest == 0 {
        return None;
    }
    let found = world
        .get::<TesTopic>(topic)
        .expect("a topic")
        .entries_28
        .iter()
        .position(|entry| entry_matches(world, entry, quest));
    if found.is_some() {
        return found;
    }
    let mut topic = world.get_mut::<TesTopic>(topic).expect("a topic");
    topic.entries_28.push(TopicInfoArray {
        quest_00: FormRef::Raw(quest),
        infos_04: NiFormArray {
            original_vtable: TOPIC_INFO_ARRAY_VTABLE,
            ..NiFormArray::default()
        },
        quest_1c: FormRef::Raw(0),
        resolved_20: 0,
    });
    Some(topic.entries_28.len() - 1)
}

/// IDA: 0052F5D0 TESTopic_FindInfo(id, reverse): each entry in list order, an empty one skipped; within it the infos forward, or from the last index down with `reverse`; the first non-null one whose form ID matches. Completion: 100%.
#[must_use]
pub fn find_info(world: &World, topic: Entity, id: u32, reverse: bool) -> Option<Entity> {
    let matches = |slot: &Option<Entity>| {
        slot.filter(|&form| world.get::<FormIdentity>(form).expect("a form").form_id_0c == id)
    };
    for entry in &world.get::<TesTopic>(topic).expect("a topic").entries_28 {
        let live = &entry.infos_04.data_04[..entry.infos_04.size_0c as usize];
        let found = if reverse {
            live.iter().rev().find_map(matches)
        } else {
            live.iter().find_map(matches)
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

// IDA: 0052EFA0 the entry of a raw quest: +0x00 equal, and for a zero quest +0x1C zero too. A resolved slot holds a pointer that the raw ID doesn't equal.
fn entry_of_raw_quest(topic: &TesTopic, quest: u32) -> Option<usize> {
    topic.entries_28.iter().position(|entry| {
        entry.quest_00 == FormRef::Raw(quest)
            && (quest != 0 || entry.quest_1c == FormRef::Raw(quest))
    })
}

// IDA: N/A an entry's array, through its topic.
fn infos_of(world: &mut World, topic: Entity, entry: usize) -> Mut<'_, NiFormArray> {
    world
        .get_mut::<TesTopic>(topic)
        .expect("a topic")
        .map_unchanged(|topic| &mut topic.entries_28[entry].infos_04)
}

// IDA: 0052EFE0 and 0052F650's body: each non-null info's +0x20 = its index (the low word).
fn renumber(world: &mut World, topic: Entity, entry: usize) {
    let (data, size) = {
        let mut infos = infos_of(world, topic, entry);
        (std::mem::take(&mut infos.data_04), infos.size_0c as usize)
    };
    for (index, slot) in data[..size].iter().enumerate() {
        if let Some(info) = slot {
            world
                .get_mut::<TesTopicInfo>(*info)
                .expect("an info")
                .index_20 = index as u16;
        }
    }
    infos_of(world, topic, entry).data_04 = data;
}

/// IDA: 0052FD20 TESTopic_InsertInfo(quest, info, previous), on the current topic. A zero quest or a topic with flag 8 answers 0. 0052FC40; then by `previous`: 0 inserts at the front; none (-1) with an unplaced info takes the slot past the last non-null one (0052ED80, with +0x20 stored first); none with a placed info changes nothing; any other leaves the info's old slot empty (00446C50) and goes after that info, found from the end of each entry (0052F5D0(previous, 1)), whose +0x20 + 1 is the index, or for an unplaced one its slot in this array + 1. Not found warns and inserts at the front. Then 005A56F0 and 0052F650. The warning's second value is the topic's ID, `[ebp+0x0C]`, where the text says TopicInfo: `ebx` held the info until 0052FDFD overwrites it. Completion: 100%.
pub fn insert_info(
    world: &mut World,
    topic: Entity,
    quest: u32,
    info: Entity,
    previous: u32,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if quest == 0 || world.get::<FormIdentity>(topic).expect("a form").flags_08 & 8 != 0 {
        return Ok(false);
    }
    let entry = find_or_create_entry(world, topic, quest).expect("a nonzero quest");
    let index_20 = world.get::<TesTopicInfo>(info).expect("an info").index_20;
    if previous == 0 {
        infos_of(world, topic, entry).insert(0, info);
    } else if previous == NO_PREVIOUS {
        if index_20 == NO_INDEX {
            let at = {
                let mut infos = infos_of(world, topic, entry);
                let size = infos.size_0c;
                let mut last = size.wrapping_sub(1) as i32;
                while last > 0 {
                    if (last as u32) < size && infos.data_04[last as usize].is_some() {
                        break;
                    }
                    last -= 1;
                }
                let at = last.wrapping_add(1) as u32;
                // 0052ED80: a slot at or past the capacity grows the block first.
                if at >= infos.capacity_08 {
                    let grown = infos.grow_14.wrapping_add(at);
                    infos.set_capacity(grown);
                }
                infos.set_at(at, Some(info));
                at
            };
            world
                .get_mut::<TesTopicInfo>(info)
                .expect("an info")
                .index_20 = at as u16;
        }
    } else {
        if index_20 != NO_INDEX {
            let mut infos = infos_of(world, topic, entry);
            if usize::from(index_20) >= infos.data_04.len() {
                return Err(StreamError(
                    "0052FD8C: 00446C50 writes past this entry's block",
                ));
            }
            infos.set_at(u32::from(index_20), None);
        }
        if let Some(found) = find_info(world, topic, previous, true) {
            let found_index = world.get::<TesTopicInfo>(found).expect("an info").index_20;
            let at = if found_index == NO_INDEX {
                let infos =
                    &world.get::<TesTopic>(topic).expect("a topic").entries_28[entry].infos_04;
                infos.data_04[..infos.size_0c as usize]
                    .iter()
                    .position(|slot| *slot == Some(found))
                    .map_or(u32::MAX, |slot| slot as u32 + 1)
            } else {
                u32::from(found_index) + 1
            };
            if at == u32::MAX {
                return Err(StreamError(
                    "0052FDD3: 0052F3C0(-1) writes before this entry's block",
                ));
            }
            infos_of(world, topic, entry).insert(at, info);
        } else {
            let current = world
                .get_resource::<DialogueGlobals>()
                .and_then(|globals| globals.current_topic_b3650c)
                .ok_or(StreamError("0052FDF8: a null 00B3650C faults"))?;
            let name = latin1(&editor_id_of(world, current));
            let current_id = world
                .get::<FormIdentity>(current)
                .expect("a form")
                .form_id_0c;
            let topic_id = world.get::<FormIdentity>(topic).expect("a form").form_id_0c;
            cx.file.services.warn(format!(
                "Could not find previous info ({previous:08X}) for TopicInfo ({topic_id:08X}) in Topic \"{name}\" ({current_id:08X})."
            ));
            infos_of(world, topic, entry).insert(0, info);
        }
    }
    // 0052FE6B 005A56F0 on this array; 0052FE77 0052F650 finds its own by the raw quest.
    infos_of(world, topic, entry).compact();
    let renumbered = entry_of_raw_quest(world.get::<TesTopic>(topic).expect("a topic"), quest);
    if let Some(renumbered) = renumbered {
        renumber(world, topic, renumbered);
    }
    Ok(true)
}

// IDA: 0044E474 the DIAL arm of 0044DCF0: an existing topic reloads (0044E291); a new one, 0052FB40(0), loads, whatever 00447050 answers, and 0067B1E0 appends it to DataHandler +0x7C. Returns 1. FormHeap failing isn't modelled. Completion: 100%.
fn topic_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    existing: Option<Entity>,
    _first: bool,
) -> Result<u32, StreamError> {
    if let Some(form) = existing {
        load_form_from_file(world, form, cx)?;
        return Ok(1);
    }
    let form = construct_topic(world)?;
    load_form_from_file(world, form, cx)?;
    world
        .resource_mut::<DataHandlerLists>()
        .topics_7c
        .append_non_null(form);
    Ok(1)
}

// IDA: 005301E0 TESTopic vt+0x1C LoadForm. The record must be DIAL (`cmp al`; else 0). 00451210; the chunks while nonzero: EDID (alloca, size 0x200, vt+0xD8 0052F480); DATA 1 byte into +0x24; QSTI 4 bytes over a zero, mapped through the topic's last mod reference (0046B680(-1), 0046BB20), 0052FC40, and with an entry 00452910(+0x0C + 100) and +0x18 = 10; FULL 0046C7A0; XIDX 4 bytes into +0x30. 00B3650C = this topic on every path past the type test. Returns 1. Completion: 100%.
fn load_topic_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_TOPIC {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            EDID => {
                let text = cx.file.read_text(0x200)?;
                let mut topic = world.get_mut::<TesTopic>(form).expect("a topic");
                set_editor_string(&mut topic.editor_34, &text);
            }
            DATA => {
                let mut bytes = [world.get::<TesTopic>(form).expect("a topic").type_24];
                cx.file.read_chunk(&mut bytes, 1)?;
                world.get_mut::<TesTopic>(form).expect("a topic").type_24 = bytes[0];
            }
            QSTI => {
                let (raw, _) = cx.file.read_dword(0)?;
                let file = last_mod_reference(world, form);
                let quest = map_file_form_id(world, cx, raw, file)?;
                if let Some(entry) = find_or_create_entry(world, form, quest) {
                    let mut infos = infos_of(world, form, entry);
                    let grown = infos.capacity_08.wrapping_add(100);
                    infos.set_capacity(grown);
                    infos.grow_14 = 10;
                }
            }
            FULL => {
                let mut name = world.get_mut::<TesFullName>(form).expect("a topic");
                load_full_name(&mut name.name_04, cx)?;
            }
            XIDX => {
                let current = world.get::<TesTopic>(form).expect("a topic").index_30;
                let (value, _) = cx.file.read_dword(current)?;
                world.get_mut::<TesTopic>(form).expect("a topic").index_30 = value;
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    world.init_resource::<DialogueGlobals>();
    world.resource_mut::<DialogueGlobals>().current_topic_b3650c = Some(form);
    Ok(true)
}

// ---- the default topics ----

// IDA: 00B110F4 the counts, 00B111B8 the tables of 12-byte {TESTopic*, form ID, name} rows, read from the exe. A group's index is its topics' +0x24.
const DEFAULT_TOPICS: [&[(u32, &[u8])]; 7] = [
    &[(0xC8, b"GREETING")],
    &[
        (0xD2, b"HELLO"),
        (0xD3, b"ANY"),
        (0xD4, b"GOODBYE"),
        (0xD5, b"Idle Chatter"),
        (0xD6, b"SPELL HELP"),
        (0xD7, b"INFO GENERAL"),
    ],
    &[
        (0xDC, b"Attack"),
        (0xDD, b"Hit"),
        (0xDE, b"Flee"),
        (0xDF, b"Steal"),
        (0xE0, b"Trespass"),
        (0xE1, b"Yield"),
        (0xE2, b"AcceptYield"),
        (0xE3, b"Pickpocket"),
        (0xE4, b"Assault"),
        (0xE5, b"Murder"),
        (0xE6, b"PowerAttack"),
        (0xE7, b"AssaultNoCrime"),
        (0xE8, b"MurderNoCrime"),
        (0xE9, b"PickpocketNoCrime"),
        (0xEA, b"StealNoCrime"),
        (0xEB, b"TrespassNoCrime"),
    ],
    &[
        (0xF0, b"AdmireSuccess"),
        (0xF1, b"AdmireFail"),
        (0xF2, b"AdmireNeutral"),
        (0xF3, b"TauntSuccess"),
        (0xF4, b"TauntFail"),
        (0xF5, b"TauntNeutral"),
        (0xF6, b"BoastSuccess"),
        (0xF7, b"BoastFail"),
        (0xF8, b"BoastNeutral"),
        (0xF9, b"JokeSuccess"),
        (0xFA, b"JokeFail"),
        (0xFB, b"JokeNeutral"),
        (0xFC, b"BribeSuccess"),
        (0xFD, b"BribeFail"),
        (0xFE, b"DemandSuccess"),
        (0xFF, b"DemandFail"),
        (0x100, b"DemandNoMoney"),
        (0x101, b"DemandNoMoreCircle"),
        (0x102, b"BribeNoMoreCircle"),
        (0x103, b"BribeNoMoney"),
        (0xAA, b"ADMIRE_HATE"),
        (0xAB, b"ADMIRE_LOVE"),
        (0xAC, b"ADMIRE_LIKE"),
        (0xAD, b"ADMIRE_DISLIKE"),
        (0xAE, b"COERCE_HATE"),
        (0xAF, b"COERCE_LOVE"),
        (0xB0, b"COERCE_LIKE"),
        (0xB1, b"COERCE_DISLIKE"),
        (0xB2, b"BOAST_HATE"),
        (0xB3, b"BOAST_LOVE"),
        (0xB4, b"BOAST_LIKE"),
        (0xB5, b"BOAST_DISLIKE"),
        (0xB6, b"JOKE_HATE"),
        (0xB7, b"JOKE_LOVE"),
        (0xB8, b"JOKE_LIKE"),
        (0xB9, b"JOKE_DISLIKE"),
        (0xBA, b"BRIBE"),
        (0xBB, b"PERSUASION_ENTER"),
        (0xBC, b"PERSUASION_EXIT"),
    ],
    &[
        (0x104, b"Noticed"),
        (0x105, b"Seen"),
        (0x106, b"Unseen"),
        (0x107, b"Lost"),
    ],
    &[
        (0x10E, b"ServiceRefusal"),
        (0x10F, b"BarterStart"),
        (0x110, b"BarterFail"),
        (0x111, b"Repair"),
        (0x112, b"Travel"),
        (0x113, b"Training"),
        (0x114, b"BarterBuyItem"),
        (0x115, b"BarterSellItem"),
        (0x116, b"BarterExit"),
        (0x117, b"BarterStolen"),
        (0x11D, b"RepairExit"),
        (0x120, b"Recharge"),
        (0x121, b"RechargeExit"),
        (0x124, b"TrainingExit"),
    ],
    &[
        (0x118, b"InfoRefusal"),
        (0x119, b"Idle"),
        (0x11A, b"ObserveCombat"),
        (0x11B, b"Corpse"),
        (0x11C, b"TimeToGo"),
    ],
];

/// IDA: the TESTopic* column of the 00B111B8 tables, by group.
#[derive(Resource, Debug, Default)]
pub struct DefaultTopicSlots {
    pub groups: [Vec<Option<Entity>>; 7],
}

/// IDA: 0052FE90 the default topics, from 0044C5A2 in 0044AC20. Each group in table order; a row whose slot is empty gets FormHeap(0x3C) with 0052FB40's stores and +0x24 = the group, 0046C300(ID, 1), vt+0xD8 0052F480, 004028D0 on the full name, 00447530 (a prepend to DataHandler +0x7C) and 00412D30 on 00B06164. Completion: 100%, with 12d placing the call.
pub fn create_default_topics(world: &mut World) -> Result<(), StreamError> {
    world.init_resource::<DefaultTopicSlots>();
    for (group, rows) in DEFAULT_TOPICS.iter().enumerate() {
        world.resource_mut::<DefaultTopicSlots>().groups[group].resize(rows.len(), None);
        for (index, &(id, name)) in rows.iter().enumerate() {
            if world.resource::<DefaultTopicSlots>().groups[group][index].is_some() {
                continue;
            }
            let form = construct_topic_of_type(world, group as u8)?;
            world.resource_mut::<DefaultTopicSlots>().groups[group][index] = Some(form);
            set_form_id(world, form, id);
            set_editor_string(
                &mut world
                    .get_mut::<TesTopic>(form)
                    .expect("just built")
                    .editor_34,
                name,
            );
            world
                .get_mut::<TesFullName>(form)
                .expect("just built")
                .name_04
                .set(Some(name), 0);
            world
                .resource_mut::<DataHandlerLists>()
                .topics_7c
                .prepend_non_null(form);
            world
                .resource_mut::<EditorIdMap>()
                .insert_or_assign(name, form);
        }
    }
    Ok(())
}

// ---- TESTopicInfo ----

/// IDA: the FormHeap(0x10) block of TESTopicInfo +0x30, cleared on allocation: +0x00 the TCLF list, +0x08 the TCLT list. Items are raw form IDs until InitItem.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TopicInfoChoices {
    pub link_from_00: Vec<FormRef>,
    pub choices_08: Vec<FormRef>,
}

/// IDA: 00530AE0 TESTopicInfo (0x38): +0x18 TESCondition, +0x20 the index in its array (0xFFFF), +0x22, +0x23 type, +0x24 next speaker, +0x25 flags (+0x26, +0x27 stay unset), +0x28 the NAME list of raw IDs, +0x30 the choices, +0x34 the record offset, or the TOFT arm's value.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct TesTopicInfo {
    pub conditions_18: TesCondition,
    pub index_20: u16,
    pub field_22: u8,
    pub type_23: u8,
    pub next_speaker_24: u8,
    pub flags_25: u8,
    pub names_28: Vec<FormRef>,
    pub choices_30: Option<TopicInfoChoices>,
    pub record_offset_34: u32,
}

impl Default for TesTopicInfo {
    // IDA: 00530B22..00530B48.
    fn default() -> Self {
        Self {
            conditions_18: TesCondition::default(),
            index_20: NO_INDEX,
            field_22: 0,
            type_23: 0,
            next_speaker_24: 0,
            flags_25: 0,
            names_28: Vec::new(),
            choices_30: None,
            record_offset_34: 0,
        }
    }
}

// IDA: 00530AE0 TESTopicInfo_Constructor(0): 0046C460, the vtable, 00434920 on +0x18, the fields, +0x04 = 0x3A, 0046C2E0, +0x23 = the argument. Its argument-1 arm (the +0x30 block and +0x25 | 2) isn't reached: 0044E4E5 pushes esi, which is 0. Completion: 100% of the store order at 0.
fn construct_topic_info(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        OriginalVtable(TES_TOPIC_INFO_VTABLE),
        TesTopicInfo::default(),
    ));
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_TOPIC_INFO;
    reset_form_components(world, form, &[])?;
    Ok(form)
}

// IDA: 00530560 TESTopicInfo vt+0x14: the word at +0x23 (type and next speaker), +0x25, +0x34, +0x22, +0x20 = 0xFFFF, +0x04 = 0x3A, +0x30 = 0 with no free (00530DB0 freed it), then 0046C2E0. The conditions and NAME list stay for vt+0x18, which runs first. Completion: 100%.
fn reset_topic_info(world: &mut World, form: Entity) -> Result<(), StreamError> {
    {
        let mut info = world.get_mut::<TesTopicInfo>(form).expect("an info");
        info.type_23 = 0;
        info.next_speaker_24 = 0;
        info.flags_25 = 0;
        info.record_offset_34 = 0;
        info.field_22 = 0;
        info.index_20 = NO_INDEX;
        info.choices_30 = None;
    }
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_TOPIC_INFO;
    reset_14_components(world, form)
}

// IDA: 0052F690 the topic of an info: DataHandler +0x7C from its head, and the first whose 0052F5D0(id, 0) is this info. Completion: 100%.
fn topic_of(world: &World, info: Entity) -> Option<Entity> {
    let id = world.get::<FormIdentity>(info).expect("a form").form_id_0c;
    world
        .resource::<DataHandlerLists>()
        .topics_7c
        .items()
        .find(|&topic| find_info(world, topic, id, false) == Some(info))
}

// IDA: 00530590 TESTopicInfo_RemoveFromTopic(topic), with the +0x20 test first. 0052F520 the entry holding the info, whose null answer faults at 005305B1; the info's slot, unbounded in the original, read within the block here; a hit 005304C0, a miss the first slot that matches, cleared the same way inline; 005A56F0; 0052EFE0. +0x20 stays for vt+0x14. Completion: 100%.
fn remove_from_topic(world: &mut World, info: Entity, topic: Entity) -> Result<(), StreamError> {
    let index = world.get::<TesTopicInfo>(info).expect("an info").index_20;
    if index == NO_INDEX {
        return Ok(());
    }
    let entry = world
        .get::<TesTopic>(topic)
        .expect("a topic")
        .entries_28
        .iter()
        .position(|entry| {
            let infos = &entry.infos_04;
            infos.data_04[..infos.size_0c as usize].contains(&Some(info))
        })
        .ok_or(StreamError("005305B1: 0052F520 found no entry"))?;
    {
        let mut infos = infos_of(world, topic, entry);
        if infos.data_04.get(usize::from(index)) == Some(&Some(info)) {
            infos.remove_at(u32::from(index));
        } else if let Some(slot) = infos.data_04[..infos.size_0c as usize]
            .iter()
            .position(|slot| *slot == Some(info))
        {
            infos.remove_at(slot as u32);
        }
        infos.compact();
    }
    renumber(world, topic, entry);
    Ok(())
}

// IDA: 00530E16..00530E42: a Script on the stack, 004FC7A0 onto the static 00B36530, the destructor. 009E4220 constructs 00B36530 before WinMain with g_dataHandler null, so with no form ID: host, on first use with the TLS byte set for the call.
fn reset_result_script(world: &mut World) -> Result<(), StreamError> {
    let target = match world.resource::<DialogueGlobals>().result_script_b36530 {
        Some(target) => target,
        None => {
            let saved = world.resource::<GameThreadLocals>().byte_184;
            world.resource_mut::<GameThreadLocals>().byte_184 = 1;
            let target = construct_script_in_place(world);
            world.resource_mut::<GameThreadLocals>().byte_184 = saved;
            let target = target?;
            world.resource_mut::<DialogueGlobals>().result_script_b36530 = Some(target);
            target
        }
    };
    let fresh = construct_script_in_place(world)?;
    copy_script_from(world, target, fresh)?;
    destroy_script_in_place(world, fresh)
}

// IDA: 00530DB0 TESTopicInfo vt+0x18. Without flag 0x4000: a placed info leaves its topic (0052F690, 00530590); 0056A750; 00530620, a host stop with entries; 00530690 the NAME list; the result Script. Always: +0x30 read, 00B36528 and 00B3652C cleared, the block freed (00530500; its word stays for vt+0x14, and the host drops it now); 0046C2F0. Completion: 100%.
fn destroy_topic_info(world: &mut World, form: Entity) -> Result<(), StreamError> {
    world.init_resource::<DialogueGlobals>();
    if world.get::<FormIdentity>(form).expect("a form").flags_08 & FORM_FLAG_PARTIAL == 0 {
        if world.get::<TesTopicInfo>(form).expect("an info").index_20 != NO_INDEX
            && let Some(topic) = topic_of(world, form)
        {
            remove_from_topic(world, form, topic)?;
        }
        world
            .get_mut::<TesTopicInfo>(form)
            .expect("an info")
            .conditions_18
            .items
            .clear();
        if world.resource::<DialogueGlobals>().cached_responses_b36580 != 0 {
            return Err(StreamError(
                "00530644: 00530620's 0052E1C0, the cached responses",
            ));
        }
        world
            .get_mut::<TesTopicInfo>(form)
            .expect("an info")
            .names_28
            .clear();
        reset_result_script(world)?;
    }
    {
        let mut globals = world.resource_mut::<DialogueGlobals>();
        globals.response_info_b36528 = 0;
        globals.script_info_b3652c = 0;
    }
    world
        .get_mut::<TesTopicInfo>(form)
        .expect("an info")
        .choices_30 = None;
    reset_18_components(world, form)
}

// IDA: 0044E4C9 the INFO arm of 0044DCF0: a new 00530AE0(0) when there is none, 00B33AA4 = the info, then 00447050's byte. Completion: 100%.
fn topic_info_arm(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    existing: Option<Entity>,
    _first: bool,
) -> Result<u32, StreamError> {
    let form = match existing {
        Some(form) => form,
        None => construct_topic_info(world)?,
    };
    world.resource_mut::<DataLoadGlobals>().current_info_b33aa4 = Some(form);
    Ok(u32::from(load_form_from_file(world, form, cx)?))
}

// IDA: 00530FB0 TESTopicInfo vt+0x1C LoadForm. The record must be INFO (`cmp al`; else 0). A file with +0x3DC bit 4 clear (0044FAB0) gives +0x34 the record offset. previous = -1, quest = 0; 00451210. The chunks while nonzero:
// DATA 3 bytes into +0x23..+0x25; a type at or above 7, signed, runs vt+0x8C(1) 0046A9E0 (+0x08 | 0x20, then vt+0x40(0x40000001)); a type of 1 runs 00530BA0(0), which frees any block and allocates a cleared one. NAME 4 bytes over a zero, 0067B1E0 onto +0x28. TCLF and TCLT make the block when it's missing, then 4 bytes over a zero, 0067B1E0 onto +0x00 and +0x08. QSTI 4 bytes over the local, 0046BB20 through this file. CTDA and CTDT 0056A7B0. PNAM 4 bytes over the local, and nonzero 0046BB20 through this file.
// Past the chunks, without flag 0x20: 0052FD20(quest, this, previous) on 00B3650C, and its 0 warns. A null 00B3650C faults on either path. Returns 1. Completion: 100%.
fn load_topic_info_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_TOPIC_INFO {
        return Ok(false);
    }
    let file = cx.file.file()?;
    if file.flags_3dc & 0x10 == 0 {
        let offset = file.record_offset_25c;
        world
            .get_mut::<TesTopicInfo>(form)
            .expect("an info")
            .record_offset_34 = offset;
    }
    let mut previous = NO_PREVIOUS;
    let mut quest = 0;
    initialize_form_from_current_record(world, form, cx)?;
    let this_file = Some(cx.file.id);
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            DATA => {
                let mut info = world.get_mut::<TesTopicInfo>(form).expect("an info");
                let mut bytes = [info.type_23, info.next_speaker_24, info.flags_25];
                cx.file.read_chunk(&mut bytes, 3)?;
                info.type_23 = bytes[0];
                info.next_speaker_24 = bytes[1];
                info.flags_25 = bytes[2];
                if bytes[0] as i8 >= 7 {
                    world
                        .get_mut::<FormIdentity>(form)
                        .expect("a form")
                        .flags_08 |= 0x20;
                    set_change_flags(world, form, 0x4000_0001)?;
                }
                if bytes[0] == 1 {
                    world
                        .get_mut::<TesTopicInfo>(form)
                        .expect("an info")
                        .choices_30 = Some(TopicInfoChoices::default());
                }
            }
            NAME => {
                let (id, _) = cx.file.read_dword(0)?;
                if id != 0 {
                    world
                        .get_mut::<TesTopicInfo>(form)
                        .expect("an info")
                        .names_28
                        .push(FormRef::Raw(id));
                }
            }
            TCLF | TCLT => {
                let mut info = world.get_mut::<TesTopicInfo>(form).expect("an info");
                let block = info.choices_30.get_or_insert_default();
                let (id, _) = cx.file.read_dword(0)?;
                if id != 0 {
                    if chunk == TCLF {
                        block.link_from_00.push(FormRef::Raw(id));
                    } else {
                        block.choices_08.push(FormRef::Raw(id));
                    }
                }
            }
            QSTI => {
                (quest, _) = cx.file.read_dword(quest)?;
                quest = map_file_form_id(world, cx, quest, this_file)?;
            }
            CTDA | CTDT => {
                let mut info = world.get_mut::<TesTopicInfo>(form).expect("an info");
                load_condition(&mut info.conditions_18, cx)?;
            }
            PNAM => {
                (previous, _) = cx.file.read_dword(previous)?;
                if previous != 0 {
                    previous = map_file_form_id(world, cx, previous, this_file)?;
                }
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    if world.get::<FormIdentity>(form).expect("a form").flags_08 & 0x20 == 0 {
        let topic = world
            .get_resource::<DialogueGlobals>()
            .and_then(|globals| globals.current_topic_b3650c)
            .ok_or(StreamError("005311B7: 0052FD20 on a null 00B3650C faults"))?;
        if !insert_info(world, topic, quest, form, previous, cx)? {
            let name = latin1(&editor_id_of(world, topic));
            let topic_id = world.get::<FormIdentity>(topic).expect("a form").form_id_0c;
            let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
            cx.file.services.warn(format!(
                "Unable to insert topic info ({id:08X}) into topic '{name}' ({topic_id:08X}), quest ({quest:08X})."
            ));
        }
    }
    Ok(true)
}

// ---- TESQuest ----

/// IDA: 0052ACC0 the quest stage item (0x6C): +0x00 flags (QSDT), +0x04 TESCondition, +0x0C an embedded Script (SCHR, SCDA, SCRO; temporary, its header's +0x10 = 1), +0x5C the record offset, +0x60 the index, +0x61 has a log entry (CNAM), +0x64 and +0x68 = 0 until runtime and InitItem. The Script is an entity this item owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestStageItem {
    pub flags_00: u8,
    pub conditions_04: TesCondition,
    pub script_0c: Entity,
    pub record_offset_5c: u32,
    pub index_60: u8,
    pub log_61: u8,
    pub field_64: u32,
    pub quest_68: FormRef,
}

/// IDA: 0052AC20 the quest stage group (0x0C): +0x00 the stage index, +0x01 flags (INDX), +0x02 and +0x03 unset, +0x04 the stage item list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestStageGroup {
    pub index_00: u8,
    pub flags_01: u8,
    pub items_04: Vec<QuestStageItem>,
}

/// IDA: 0052B310 the quest target (0x14): +0x00 flags, +0x01..+0x03 unset, +0x04 TESCondition, +0x0C the target (a raw form ID until 0052B340 in InitItem).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuestTarget {
    pub flags_00: u8,
    pub conditions_04: TesCondition,
    pub target_0c: FormRef,
}

/// IDA: 0052AA00 TESQuest (0x68): TESScriptableForm +0x18, TESIcon +0x24, TESFullName +0x30 (components on the same entity), +0x3C flags and +0x3D priority (DATA; +0x3E, +0x3F unset), +0x40 the stage groups, +0x48 the targets, +0x50 TESCondition, +0x58 a block no load sets, +0x5C a byte (+0x5D..+0x5F unset), +0x60 the editor ID.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesQuest {
    pub flags_3c: u8,
    pub priority_3d: u8,
    pub stages_40: Vec<QuestStageGroup>,
    pub targets_48: Vec<QuestTarget>,
    pub conditions_50: TesCondition,
    pub events_58: Option<ScriptEventList>,
    pub byte_5c: u8,
    pub editor_60: BsStringT,
}

const QUEST_SLOTS: &[ComponentSlot] = &[
    ComponentSlot::FullName,
    ComponentSlot::Texture,
    ComponentSlot::Scriptable,
];

// IDA: 0052AA00 TESQuest_Constructor (00447DF0 arm 00448828): 0046C460; 0046EF60 on +0x18; 0046FFD0 on +0x24 with TESIcon's vtable; TESFullName on +0x30 inline; the four vtables; +0x3C, +0x3D, the two lists; 00434920 on +0x50; +0x60..+0x66; +0x04 = 0x3B; +0x58; +0x3C | 1; vt+0x40(4); the byte +0x5C = 0; 0046C2E0. Completion: 100% of the store order.
fn construct_quest(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        OriginalVtable(TES_QUEST_VTABLE),
        TesScriptableForm {
            original_vtable: TES_QUEST_SCRIPTABLE_VTABLE,
            script_04: FormRef::Raw(0),
            resolved_08: 0,
        },
        TesTexture {
            original_vtable: TES_QUEST_ICON_VTABLE,
            path_04: BsStringT::default(),
        },
        TesFullName {
            original_vtable: TES_QUEST_FULL_NAME_VTABLE,
            name_04: BsStringT::default(),
        },
        TesQuest::default(),
    ));
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_QUEST;
    reset_quest(world, form)?;
    Ok(form)
}

// IDA: 0052A500 TESQuest vt+0x14, the same tail 0052AAAD runs inline: +0x3C | 1, vt+0x40(4), the byte +0x5C = 0, 0046C2E0. +0x3C's other bits and +0x3D stay. Completion: 100%.
fn reset_quest(world: &mut World, form: Entity) -> Result<(), StreamError> {
    world.get_mut::<TesQuest>(form).expect("a quest").flags_3c |= 1;
    set_change_flags(world, form, 4)?;
    world.get_mut::<TesQuest>(form).expect("a quest").byte_5c = 0;
    reset_14_components(world, form)
}

// IDA: 0052AD40 the stage item's destructor, in its store order: 00B362FC = 0, 00B36300 freed with its two words, +0x64 (0060D0A0 and a free; nothing sets it through a load), 004FC980 on +0x0C, 0056A7A0 on +0x04. Completion: 100%.
fn destroy_stage_item(world: &mut World, item: QuestStageItem) -> Result<(), StreamError> {
    world.init_resource::<DialogueGlobals>();
    {
        let mut globals = world.resource_mut::<DialogueGlobals>();
        globals.stage_field_b362fc = 0;
        globals.stage_text_b36300 = BsStringT::default();
    }
    if item.field_64 != 0 {
        return Err(StreamError("0052ADA5: a stage item's +0x64 block"));
    }
    destroy_script_in_place(world, item.script_0c)
}

// IDA: 0052A520 TESQuest vt+0x18: 0056A750 on +0x50; 00529760 the stage groups, each through 0052B300 / 0052B190 and its items through 0052AD40; 005297C0 the targets (0052B330); +0x58 (004FB4E0, a host stop); 0046C2F0. Completion: 100% of the arms a load reaches.
fn destroy_quest(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let stages = {
        let mut quest = world.get_mut::<TesQuest>(form).expect("a quest");
        quest.conditions_50.items.clear();
        std::mem::take(&mut quest.stages_40)
    };
    for group in stages {
        for item in group.items_04 {
            destroy_stage_item(world, item)?;
        }
    }
    {
        let mut quest = world.get_mut::<TesQuest>(form).expect("a quest");
        quest.targets_48.clear();
        if quest.events_58.is_some() {
            return Err(StreamError("0052A543: 004FB4E0 on the +0x58 block"));
        }
    }
    reset_18_components(world, form)
}

// IDA: 0052AAF0 TESQuest vt+0xD4: +0x60 or `Default`. Completion: 100%.
fn quest_editor_id(world: &World, form: Entity) -> Vec<u8> {
    world
        .get::<TesQuest>(form)
        .expect("a quest")
        .editor_60
        .data_00
        .clone()
        .unwrap_or_default()
}

// IDA: 0052ACC0 QuestStageItem_Constructor: 00434920 on +0x04; 004FBAA0 on +0x0C; +0x00; 0046AB80(0) on the Script; 0046B590 on it; the byte +0x34 (the header's +0x10) = 1; +0x5C, +0x64, +0x68, +0x60, +0x61. Completion: 100% of the store order.
fn construct_stage_item(world: &mut World) -> Result<QuestStageItem, StreamError> {
    let script = construct_script_in_place(world)?;
    set_initialized(world, script, false);
    mark_temporary(world, script)?;
    world
        .get_mut::<Script>(script)
        .expect("just built")
        .header_18[0x10] = 1;
    Ok(QuestStageItem {
        flags_00: 0,
        conditions_04: TesCondition::default(),
        script_0c: script,
        record_offset_5c: 0,
        index_60: 0,
        log_61: 0,
        field_64: 0,
        quest_68: FormRef::Raw(0),
    })
}

// IDA: 0052B1F0 QuestStageItem_LoadChunk(file). QSDT: +0x5C = the record offset, 1 byte into +0x00. SCHR: the whole chunk into the Script's +0x18, with no limit; 0060D0A0 is a `retn`. SCDA: alloca, memset, the whole chunk, 004F9DF0. SCRO: a cleared 0x10 block, 4 bytes into a local, +0x08, 0067B1E0 onto the Script's +0x40. Returns 1, which no caller reads. Completion: 100%.
// TODO: [ ] 0052B2BA an SCHR past 0x14 bytes writes on over +0x2C and +0x30 in the original, and 0052B20C an SCRO under 4 bytes stores the local's start, the item's address: host stops here.
fn load_stage_chunk(
    item: &mut QuestStageItem,
    script: &mut Script,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    match cx.file.chunk_type()? {
        QSDT => {
            item.record_offset_5c = cx.file.record_offset()?;
            let mut bytes = [item.flags_00];
            cx.file.read_chunk(&mut bytes, 1)?;
            item.flags_00 = bytes[0];
        }
        SCHR => {
            if cx.file.chunk_size()? > 0x14 {
                return Err(StreamError(
                    "0052B2BA: an SCHR chunk past 0x14 bytes overruns +0x2C",
                ));
            }
            cx.file.read_chunk(&mut script.header_18, 0)?;
        }
        SCDA => {
            let mut source = vec![0_u8; cx.file.chunk_size()? as usize];
            cx.file.read_chunk(&mut source, 0)?;
            set_compiled_data(script, &source);
        }
        SCRO => {
            if cx.file.chunk_size()? < 4 {
                return Err(StreamError(
                    "0052B26D: an SCRO under 4 bytes stores the item's own address",
                ));
            }
            let (id, _) = cx.file.read_dword(0)?;
            script.references_40.push(ScriptReference {
                form_08: id,
                ..ScriptReference::default()
            });
        }
        _ => {}
    }
    Ok(())
}

// IDA: N/A 0052B1F0 on an item, with its Script out of the world for the call.
fn stage_chunk_with_script(
    world: &mut World,
    item: &mut QuestStageItem,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    let script = item.script_0c;
    let mut held = std::mem::take(&mut *world.get_mut::<Script>(script).expect("a script"));
    let result = load_stage_chunk(item, &mut held, cx);
    *world.get_mut::<Script>(script).expect("a script") = held;
    result
}

// IDA: 0052AC60 QuestStageGroup_LoadINDX(file): the type test, then an 8-byte chunk through a local gives bytes 0 and 1, and any other size reads 2 bytes straight into +0x00. Completion: 100%.
fn load_stage_group(
    group: &mut QuestStageGroup,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    if cx.file.chunk_type()? != INDX {
        return Ok(());
    }
    if cx.file.chunk_size()? == 8 {
        let mut bytes = [0_u8; 8];
        cx.file.read_chunk(&mut bytes, 8)?;
        group.flags_01 = bytes[1];
        group.index_00 = bytes[0];
    } else {
        let mut bytes = [group.index_00, group.flags_01];
        cx.file.read_chunk(&mut bytes, 2)?;
        group.index_00 = bytes[0];
        group.flags_01 = bytes[1];
    }
    Ok(())
}

// IDA: 0052B3F0 QuestTarget_LoadQSTA(file): the type test, then the whole chunk into 8 zeroed stack bytes: +0x0C the first dword, +0x00 byte 4. Completion: 100%.
// TODO: [ ] 0052B41F a QSTA past 8 bytes overruns the stack in the original: a host stop here.
fn load_quest_target(
    target: &mut QuestTarget,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    if cx.file.chunk_type()? != QSTA {
        return Ok(());
    }
    if cx.file.chunk_size()? > 8 {
        return Err(StreamError(
            "0052B41F: a QSTA past 8 bytes overruns the stack",
        ));
    }
    let mut bytes = [0_u8; 8];
    cx.file.read_chunk(&mut bytes, 0)?;
    target.target_0c = FormRef::Raw(u32::from_le_bytes(
        bytes[..4].try_into().expect("four bytes"),
    ));
    target.flags_00 = bytes[4];
    Ok(())
}

// IDA: 0052A570 TESQuest vt+0x1C LoadForm. The record must be QUST (`cmp al`; else 0). group, item and target are null; 00451210; 0046AB80(0); the next index byte is 0. The chunks while nonzero:
// DATA 0046BDA0(+0x3C, 2). QSTA clears group and item, then a new target (0052B310, 0052B3F0), 0067B1E0 onto +0x48. INDX a new group (0052AC20, 0052AC60), 0067B1E0 onto +0x40, leaving item as it was. QSDT with a group: a new item (0052ACC0), 0052B1F0, its +0x60 = the index, which goes up as a byte, 0067B1E0 onto the group's +0x04. CNAM with an item: +0x61 = 1, the chunk itself unread. SCHR, SCDA, SCRO with an item: 0052B1F0. CTDA and CTDT: 0056A7B0 onto the item's +0x04, or else the target's, or else +0x50. ICON 004700E0. EDID (alloca, size 0x200, vt+0xD8). SCRI over a zero, +0x1C, 0046F020. FULL 0046C7A0. Returns 1. Completion: 100%.
fn load_quest_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_QUEST {
        return Ok(false);
    }
    let mut group: Option<usize> = None;
    let mut item: Option<(usize, usize)> = None;
    let mut target: Option<usize> = None;
    let mut next_index = 0_u8;
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            DATA => {
                let mut quest = world.get_mut::<TesQuest>(form).expect("a quest");
                let mut bytes = [quest.flags_3c, quest.priority_3d];
                load_data_chunk(&mut bytes, 2, DataChunkTails::default(), cx)?;
                quest.flags_3c = bytes[0];
                quest.priority_3d = bytes[1];
            }
            QSTA => {
                group = None;
                item = None;
                let mut block = QuestTarget::default();
                load_quest_target(&mut block, cx)?;
                let mut quest = world.get_mut::<TesQuest>(form).expect("a quest");
                quest.targets_48.push(block);
                target = Some(quest.targets_48.len() - 1);
            }
            INDX => {
                let mut block = QuestStageGroup::default();
                load_stage_group(&mut block, cx)?;
                let mut quest = world.get_mut::<TesQuest>(form).expect("a quest");
                quest.stages_40.push(block);
                group = Some(quest.stages_40.len() - 1);
            }
            QSDT => {
                if let Some(at) = group {
                    let mut block = construct_stage_item(world)?;
                    stage_chunk_with_script(world, &mut block, cx)?;
                    block.index_60 = next_index;
                    next_index = next_index.wrapping_add(1);
                    let mut quest = world.get_mut::<TesQuest>(form).expect("a quest");
                    let items = &mut quest.stages_40[at].items_04;
                    items.push(block);
                    item = Some((at, items.len() - 1));
                }
            }
            CNAM => {
                if let Some((at, index)) = item {
                    world.get_mut::<TesQuest>(form).expect("a quest").stages_40[at].items_04
                        [index]
                        .log_61 = 1;
                }
            }
            SCHR | SCDA | SCRO => {
                if let Some((at, index)) = item {
                    let mut block = world.get::<TesQuest>(form).expect("a quest").stages_40[at]
                        .items_04[index]
                        .clone();
                    stage_chunk_with_script(world, &mut block, cx)?;
                    world.get_mut::<TesQuest>(form).expect("a quest").stages_40[at].items_04
                        [index] = block;
                }
            }
            CTDA | CTDT => {
                let mut quest = world.get_mut::<TesQuest>(form).expect("a quest");
                let condition = if let Some((at, index)) = item {
                    &mut quest.stages_40[at].items_04[index].conditions_04
                } else if let Some(index) = target {
                    &mut quest.targets_48[index].conditions_04
                } else {
                    &mut quest.conditions_50
                };
                load_condition(condition, cx)?;
            }
            ICON => {
                let mut icon = world.get_mut::<TesTexture>(form).expect("a quest");
                load_texture(&mut icon, cx)?;
            }
            EDID => {
                let text = cx.file.read_text(0x200)?;
                let mut quest = world.get_mut::<TesQuest>(form).expect("a quest");
                set_editor_string(&mut quest.editor_60, &text);
            }
            SCRI => load_script_reference(world, form, cx)?,
            FULL => {
                let mut name = world.get_mut::<TesFullName>(form).expect("a quest");
                load_full_name(&mut name.name_04, cx)?;
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    Ok(true)
}

// ---- the classes ----

/// IDA: this family's classes. DIAL and INFO have no 00447DF0 arm, so `construct` is what their `special_arm` runs. vt+0x6C is 12d's.
pub const CLASSES: &[FormClass] = &[
    FormClass {
        form_type: FORM_TYPE_TOPIC,
        vtable: TES_TOPIC_VTABLE,
        construct: construct_topic,
        load_form: load_topic_form,
        reset_14: reset_14_components,
        reset_18: reset_18_components,
        editor_id: topic_editor_id,
        init_item: super::forms_init::init_topic,
        slots: &[ComponentSlot::FullName],
        special_arm: Some(topic_arm),
    },
    FormClass {
        form_type: FORM_TYPE_TOPIC_INFO,
        vtable: TES_TOPIC_INFO_VTABLE,
        construct: construct_topic_info,
        load_form: load_topic_info_form,
        reset_14: reset_topic_info,
        reset_18: destroy_topic_info,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_topic_info,
        slots: &[],
        special_arm: Some(topic_info_arm),
    },
    FormClass {
        form_type: FORM_TYPE_QUEST,
        vtable: TES_QUEST_VTABLE,
        construct: construct_quest,
        load_form: load_quest_form,
        reset_14: reset_quest,
        reset_18: destroy_quest,
        editor_id: quest_editor_id,
        init_item: super::forms_init::init_quest,
        slots: QUEST_SLOTS,
        special_arm: None,
    },
];
