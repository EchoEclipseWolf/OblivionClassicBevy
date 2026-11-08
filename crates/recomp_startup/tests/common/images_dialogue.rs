//! IDA: N/A the bytes of the dialogue forms, for the sample rows. A pointer's DATA row is compared where the block behind it is modelled: a quest's script head, its first stage group and target, the second list nodes, its first condition item. Nothing dumped a stage item, an info's blocks or a topic's entries, so those stop at the head.

use bevy_ecs::prelude::*;
use recomp_game::tes::data_handler::TesTexture;
use recomp_game::tes::form::{FormIdentity, OriginalVtable};
use recomp_game::tes::form_components::{FormRef, TesFullName, TesScriptableForm};
use recomp_game::tes::forms_dialogue::{TesQuest, TesTopic, TesTopicInfo};

use super::image::{BlockCell, Image, block_of};

// IDA: N/A a little-endian dword in a block.
fn word(value: u32) -> Vec<BlockCell> {
    value.to_le_bytes().map(BlockCell::Byte).to_vec()
}

// IDA: N/A a list's head at `at`, and the second node behind +0x04: {item, next}, by null-ness.
fn list_head(
    image: &mut Image,
    at: usize,
    role: &'static str,
    length: usize,
    first: Vec<BlockCell>,
) {
    image.pointer_block(at, role, length > 0, first);
    image.pointer_block(
        at + 4,
        role,
        length > 1,
        vec![BlockCell::Word(length > 1), BlockCell::Word(length > 2)],
    );
}

// IDA: TESTopic (0x3C): +0x25..+0x27 stay unset. The head of the entry list is all a row has.
pub fn topic_image(world: &World, form: Entity) -> Image {
    let topic = world.get::<TesTopic>(form).unwrap();
    let mut image = Image::new(0x3C);
    image.form_head(world, form);
    image.full_name(0x18, world.get::<TesFullName>(form).unwrap());
    image.u8(0x24, topic.type_24);
    image.pointer(0x28, "entry item", !topic.entries_28.is_empty());
    image.pointer(0x2C, "entry next", topic.entries_28.len() > 1);
    image.u32(0x30, topic.index_30);
    image.bs_string(0x34, "editor ID", &topic.editor_34);
    image
}

// IDA: TESTopicInfo (0x38): +0x26 and +0x27 stay unset. +0x28's item is a raw form ID until InitItem.
pub fn topic_info_image(world: &World, form: Entity) -> Image {
    let info = world.get::<TesTopicInfo>(form).unwrap();
    let mut image = Image::new(0x38);
    image.form_head(world, form);
    image.pointer(0x18, "condition item", !info.conditions_18.items.is_empty());
    image.pointer(0x1C, "condition next", info.conditions_18.items.len() > 1);
    image.u16(0x20, info.index_20);
    image.u8(0x22, info.field_22);
    image.u8(0x23, info.type_23);
    image.u8(0x24, info.next_speaker_24);
    image.u8(0x25, info.flags_25);
    image.form_ref(
        0x28,
        "NAME item",
        info.names_28.first().copied().unwrap_or(FormRef::Raw(0)),
    );
    image.pointer(0x2C, "NAME next", info.names_28.len() > 1);
    image.pointer(0x30, "choices", info.choices_30.is_some());
    image.u32(0x34, info.record_offset_34);
    image
}

// IDA: TESQuest (0x68): +0x3E, +0x3F and +0x5D..+0x5F stay unset. The run's dump reaches the blocks behind +0x1C and +0x40..+0x54.
pub fn quest_image(world: &World, form: Entity) -> Image {
    let quest = world.get::<TesQuest>(form).unwrap();
    let mut image = Image::new(0x68);
    image.form_head(world, form);
    let script = world.get::<TesScriptableForm>(form).unwrap();
    image.u32(0x18, script.original_vtable);
    match script.script_04 {
        // The Script's vtable, type byte, flags and form ID.
        FormRef::Resolved(Some(target)) => {
            let identity = world.get::<FormIdentity>(target).unwrap();
            let mut block = word(world.get::<OriginalVtable>(target).unwrap().0);
            block.push(BlockCell::Byte(identity.form_type_04));
            block.extend([BlockCell::Unset, BlockCell::Unset, BlockCell::Unset]);
            block.extend(word(identity.flags_08));
            block.extend(word(identity.form_id_0c));
            image.pointer_block(0x1C, "script", true, block);
        }
        slot => image.form_ref(0x1C, "script", slot),
    }
    image.u8(0x20, script.resolved_08);
    image.texture(0x24, world.get::<TesTexture>(form).unwrap());
    image.full_name(0x30, world.get::<TesFullName>(form).unwrap());
    image.u8(0x3C, quest.flags_3c);
    image.u8(0x3D, quest.priority_3d);
    let group = quest.stages_40.first().map_or_else(Vec::new, |group| {
        vec![
            BlockCell::Byte(group.index_00),
            BlockCell::Byte(group.flags_01),
            BlockCell::Unset,
            BlockCell::Unset,
            BlockCell::Word(!group.items_04.is_empty()),
            BlockCell::Word(group.items_04.len() > 1),
        ]
    });
    list_head(
        &mut image,
        0x40,
        "stage group",
        quest.stages_40.len(),
        group,
    );
    let target = quest.targets_48.first().map_or_else(Vec::new, |target| {
        let mut block = vec![
            BlockCell::Byte(target.flags_00),
            BlockCell::Unset,
            BlockCell::Unset,
            BlockCell::Unset,
            BlockCell::Word(!target.conditions_04.items.is_empty()),
            BlockCell::Word(target.conditions_04.items.len() > 1),
        ];
        match target.target_0c {
            FormRef::Raw(raw) => block.extend(word(raw)),
            FormRef::Resolved(found) => block.push(BlockCell::Word(found.is_some())),
        }
        block
    });
    list_head(&mut image, 0x48, "target", quest.targets_48.len(), target);
    let condition = quest
        .conditions_50
        .items
        .first()
        .map_or_else(Vec::new, |item| block_of(&item.data_00));
    list_head(
        &mut image,
        0x50,
        "condition",
        quest.conditions_50.items.len(),
        condition,
    );
    image.pointer(0x58, "+0x58 block", quest.events_58.is_some());
    image.u8(0x5C, quest.byte_5c);
    image.bs_string(0x60, "editor ID", &quest.editor_60);
    image
}
