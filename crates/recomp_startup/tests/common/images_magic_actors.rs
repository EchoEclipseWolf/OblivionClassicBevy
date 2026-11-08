//! IDA: N/A the images of Family B's classes, for the sample rows, and what a DATA row behind a pointer must start with. A pointer cell only tests null-ness, so `behind` lifts the lists' heads, FaceGen floats and effect items out of the rows too.

use bevy_ecs::prelude::*;
use recomp_game::tes::data_handler::{TesDescription, TesTexture};
use recomp_game::tes::facegen::FaceGenMatrix;
use recomp_game::tes::form::FormIdentity;
use recomp_game::tes::form_arms::form_class;
use recomp_game::tes::form_components::{
    FormRef, TesActorBaseData, TesAiForm, TesAnimation, TesAttackDamageForm, TesAttributes,
    TesContainer, TesFullName, TesHealthForm, TesLeveledList, TesModel, TesModelList,
    TesObjectLinks, TesRaceForm, TesReactionForm, TesScriptableForm, TesSpellList,
};
use recomp_game::tes::form_type_map::{ConstructArm, FORM_TYPE_MAP, construct_arm};
use recomp_game::tes::forms_actors::{
    ActorBaseValues, CreatureSoundsSlot, FaceGenUndoArray, TesCreature, TesNpc, TesRace,
};
use recomp_game::tes::forms_magic::{EffectItemList, EffectSetting, MagicFormData};

use super::image::Image;

/// IDA: N/A a TESObject's neighbours on +0x00 are any default-arm class's forms. Until every such class is in the tree a list end can be a neighbour in the run, so the +0x1C and +0x20 cells stay out of the comparison; they return by themselves.
#[must_use]
pub fn object_neighbours_known() -> bool {
    FORM_TYPE_MAP
        .iter()
        .filter(|row| construct_arm(u32::from(row.form_type)) == ConstructArm::Default)
        .all(|row| form_class(u32::from(row.form_type)).is_some())
}

// IDA: TESBoundObject's +0x18..+0x23: the list, previous and next.
fn object_links(image: &mut Image, world: &World, form: Entity) {
    let links = world.get::<TesObjectLinks>(form).unwrap();
    image.pointer(0x18, "object list", links.in_list_18);
    if object_neighbours_known() {
        image.pointer(0x1C, "object previous", links.previous_1c.is_some());
        image.pointer(0x20, "object next", links.next_20.is_some());
    }
}

// IDA: TESModel (0x18): +0x04 BSStringT, +0x0C radius, +0x10 count, +0x11..+0x13 unset, +0x14 the array.
fn model(image: &mut Image, at: usize, component: &TesModel) {
    image.u32(at, component.original_vtable);
    image.bs_string(at + 4, "model path", &component.path_04);
    image.f32(at + 0x0C, component.bound_radius_0c);
    image.u8(at + 0x10, component.texture_count_10);
    image.pointer(
        at + 0x14,
        "model texture entries",
        component.texture_entries_14.is_some(),
    );
}

// IDA: a BSSimpleList of raw form IDs: +0x00 the head's item, +0x04 its next.
fn raw_list(image: &mut Image, at: usize, role: &'static str, list: &[FormRef]) {
    image.form_ref(at, role, list.first().copied().unwrap_or(FormRef::Raw(0)));
    image.pointer(at + 4, "raw list next", list.len() > 1);
}

// IDA: TESSpellList (0x14).
fn spell_list(image: &mut Image, at: usize, list: &TesSpellList) {
    image.u32(at, list.original_vtable);
    raw_list(image, at + 4, "spell", &list.spells_04);
    raw_list(image, at + 0x0C, "leveled spell", &list.leveled_0c);
}

// IDA: FgMatrix (0x18): +0x00 rows, +0x04 cols, +0x08 the vector's allocator bytes, +0x0C/+0x10/+0x14 first, last, end.
fn facegen(image: &mut Image, at: usize, matrix: &FaceGenMatrix) {
    image.u32(at, matrix.rows);
    image.u32(at + 4, matrix.cols);
    let allocated = matrix.data.capacity() != 0;
    image.pointer(at + 0x0C, "FgMatrix first", allocated);
    image.pointer(at + 0x10, "FgMatrix last", allocated);
    image.pointer(at + 0x14, "FgMatrix end", allocated);
}

// IDA: the NiTArray of both classes (0x10).
fn undo_array(image: &mut Image, at: usize, array: &FaceGenUndoArray) {
    image.u32(at, array.original_vtable);
    image.u32(at + 4, array.data_04);
    image.u16(at + 8, array.capacity_08);
    image.u16(at + 0x0A, array.size_0a);
    image.u16(at + 0x0C, array.count_0c);
    image.u16(at + 0x0E, array.grow_0e);
}

// IDA: EffectItemList (0x10).
fn effect_list(image: &mut Image, at: usize, list: &EffectItemList) {
    image.u32(at, list.original_vtable);
    image.pointer(at + 4, "effect item", !list.items_04.is_empty());
    image.pointer(at + 8, "effect next", list.items_04.len() > 1);
    image.u32(at + 0x0C, list.hostile_count_0c);
}

// IDA: EffectSetting (0xA8).
pub fn effect_setting_image(world: &World, form: Entity) -> Image {
    let mut image = Image::new(0xA8);
    image.form_head(world, form);
    model(&mut image, 0x18, world.get::<TesModel>(form).unwrap());
    image.description(0x30, world.get::<TesDescription>(form).unwrap());
    image.full_name(0x38, world.get::<TesFullName>(form).unwrap());
    image.texture(0x44, world.get::<TesTexture>(form).unwrap());
    let setting = world.get::<EffectSetting>(form).unwrap();
    image.u32(0x50, setting.field_50);
    image.u32(0x54, setting.field_54);
    image.bytes(0x58, &setting.data_58.bytes());
    // 00415960's slots hold pointers once it has run.
    image.form_ref(0x70, "light", setting.data_58.light_70);
    for (index, slot) in setting.data_58.shaders_and_sounds_78.iter().enumerate() {
        image.form_ref(0x78 + 4 * index, "shader or sound", *slot);
    }
    image.u32(0x98, setting.code_98);
    image.pointer(0x9C, "counter codes", setting.counters_9c.is_some());
    image.u32(0xA0, setting.art_counter_a0);
    image.u32(0xA4, setting.associated_counter_a4);
    image
}

// IDA: EnchantmentItem and SpellItem (0x44). +0x41..+0x43 are FormHeap bytes until an ENIT or SPIT writes them: zero here says nothing about them.
pub fn magic_form_image(world: &World, form: Entity) -> Image {
    let mut image = Image::new(0x44);
    image.form_head(world, form);
    image.full_name(0x18, world.get::<TesFullName>(form).unwrap());
    effect_list(&mut image, 0x24, world.get::<EffectItemList>(form).unwrap());
    let data = world.get::<MagicFormData>(form).unwrap();
    let bytes = data.bytes();
    let known = if data.unset_41 == [0; 3] { 0x0D } else { 0x10 };
    image.bytes(0x34, &bytes[..known]);
    image
}

// IDA: BirthSign (0x4C).
pub fn birth_sign_image(world: &World, form: Entity) -> Image {
    let mut image = Image::new(0x4C);
    image.form_head(world, form);
    image.full_name(0x18, world.get::<TesFullName>(form).unwrap());
    image.texture(0x24, world.get::<TesTexture>(form).unwrap());
    image.description(0x30, world.get::<TesDescription>(form).unwrap());
    spell_list(&mut image, 0x38, world.get::<TesSpellList>(form).unwrap());
    image
}

// IDA: TESLevSpell (0x34).
pub fn lev_spell_image(world: &World, form: Entity) -> Image {
    let mut image = Image::new(0x34);
    image.form_head(world, form);
    object_links(&mut image, world, form);
    let list = world.get::<TesLeveledList>(form).unwrap();
    image.u32(0x24, list.original_vtable);
    image.pointer(0x28, "leveled item", !list.entries_04.is_empty());
    image.pointer(0x2C, "leveled next", list.entries_04.len() > 1);
    image.u8(0x30, list.chance_none_0c);
    image.u8(0x31, list.flags_0d);
    image
}

// IDA: TESActorBase's cells, +0x00..+0xE3.
fn actor_base(image: &mut Image, world: &World, form: Entity) {
    image.form_head(world, form);
    object_links(image, world, form);
    let base = world.get::<TesActorBaseData>(form).unwrap();
    image.u32(0x24, base.original_vtable);
    image.bytes(0x28, &base.acbs_bytes());
    image.form_ref(0x38, "death item", base.death_item_14);
    image.pointer(0x3C, "faction item", !base.factions_18.is_empty());
    image.pointer(0x40, "faction next", base.factions_18.len() > 1);
    let container = world.get::<TesContainer>(form).unwrap();
    image.u32(0x44, container.original_vtable);
    image.u8(0x48, container.flags_04);
    image.pointer(0x4C, "container item", !container.entries_08.is_empty());
    image.pointer(0x50, "container next", container.entries_08.len() > 1);
    spell_list(image, 0x54, world.get::<TesSpellList>(form).unwrap());
    let ai = world.get::<TesAiForm>(form).unwrap();
    image.u32(0x68, ai.original_vtable);
    image.bytes(0x6C, &ai.ai_04);
    image.u32(0x70, ai.services_08);
    image.u8(0x74, ai.trainer_skill_0c);
    image.u8(0x75, ai.trainer_level_0d);
    image.u16(0x76, ai.cleared_0e);
    raw_list(image, 0x78, "package", &ai.packages_10);
    let health = world.get::<TesHealthForm>(form).unwrap();
    image.u32(0x80, health.original_vtable);
    image.u32(0x84, health.health_04);
    let attributes = world.get::<TesAttributes>(form).unwrap();
    image.u32(0x88, attributes.original_vtable);
    image.bytes(0x8C, &attributes.values_04);
    let animation = world.get::<TesAnimation>(form).unwrap();
    image.u32(0x94, animation.original_vtable);
    image.pointer(0x98, "animation item", !animation.names_04.is_empty());
    image.pointer(0x9C, "animation next", animation.names_04.len() > 1);
    image.full_name(0xA0, world.get::<TesFullName>(form).unwrap());
    model(image, 0xAC, world.get::<TesModel>(form).unwrap());
    let script = world.get::<TesScriptableForm>(form).unwrap();
    image.u32(0xC4, script.original_vtable);
    image.form_ref(0xC8, "script", script.script_04);
    image.u8(0xCC, script.resolved_08);
    let values = &world.get::<ActorBaseValues>(form).unwrap().0;
    image.pointer(0xD0, "AV item", !values.list_00.is_empty());
    image.pointer(0xD4, "AV next", values.list_00.len() > 1);
    image.pointer(0xD8, "AV magicka", values.magicka_08.is_some());
    image.pointer(0xDC, "AV fatigue", values.fatigue_0c.is_some());
    image.pointer(0xE0, "AV indexed", values.indexed_10.is_some());
}

// IDA: TESNPC (0x200).
pub fn npc_image(world: &World, form: Entity) -> Image {
    let mut image = Image::new(0x200);
    actor_base(&mut image, world, form);
    let race = world.get::<TesRaceForm>(form).unwrap();
    image.u32(0xE4, race.original_vtable);
    image.form_ref(0xE8, "race", race.race_04);
    let npc = world.get::<TesNpc>(form).unwrap();
    image.bytes(0xEC, &npc.skills_ec);
    image.form_ref(0x104, "class", npc.class_104);
    for (index, matrix) in npc.facegen_108.iter().enumerate() {
        facegen(&mut image, 0x108 + 0x18 * index, matrix);
    }
    for (index, matrix) in npc.facegen_168.iter().enumerate() {
        facegen(&mut image, 0x168 + 0x18 * index, matrix);
    }
    image.pointer(0x1C8, "hair", npc.hair_1c8.is_some());
    image.f32(0x1CC, npc.hair_length_1cc);
    image.pointer(0x1D0, "eyes", npc.eyes_1d0.is_some());
    for (index, word) in npc.face_nodes_1d4.iter().enumerate() {
        image.u32(0x1D4 + 4 * index, *word);
    }
    image.u16(0x1E0, npc.fnam_1e0);
    image.form_ref(0x1E4, "combat style", npc.combat_style_1e4);
    image.u32(0x1E8, npc.hair_color_1e8);
    image.u32(0x1EC, npc.record_offset_1ec);
    undo_array(&mut image, 0x1F0, &npc.undo_1f0);
    image
}

// IDA: TESCreature (0x140).
pub fn creature_image(world: &World, form: Entity) -> Image {
    let mut image = Image::new(0x140);
    actor_base(&mut image, world, form);
    let damage = world.get::<TesAttackDamageForm>(form).unwrap();
    image.u32(0xE4, damage.original_vtable);
    image.u16(0xE8, damage.damage_04);
    let models = world.get::<TesModelList>(form).unwrap();
    image.u32(0xEC, models.original_vtable);
    image.pointer(0xF0, "model item", !models.names_04.is_empty());
    image.pointer(0xF4, "model next", models.names_04.len() > 1);
    image.u32(0xF8, models.count_0c);
    image.pointer(0xFC, "NIFT lists", models.textures_10.is_some());
    let creature = world.get::<TesCreature>(form).unwrap();
    match &creature.sounds_100 {
        CreatureSoundsSlot::Reference(reference) => {
            image.form_ref(0x100, "sound creature", *reference);
        }
        CreatureSoundsSlot::Sounds(_) => image.pointer(0x100, "sounds", true),
    }
    image.bytes(0x104, &creature.data_104);
    image.u16(0x108, creature.soul_108);
    image.u8(0x10A, creature.reach_10a);
    image.f32(0x10C, creature.turning_10c);
    image.f32(0x110, creature.foot_weight_110);
    image.f32(0x114, creature.scale_114);
    image.form_ref(0x118, "combat style", creature.combat_style_118);
    model(&mut image, 0x11C, &creature.blood_11c);
    image.texture(0x134, &creature.decal_134);
    image
}

// IDA: TESRace (0x318).
pub fn race_image(world: &World, form: Entity) -> Image {
    let mut image = Image::new(0x318);
    image.form_head(world, form);
    image.full_name(0x18, world.get::<TesFullName>(form).unwrap());
    image.description(0x24, world.get::<TesDescription>(form).unwrap());
    spell_list(&mut image, 0x2C, world.get::<TesSpellList>(form).unwrap());
    let reaction = world.get::<TesReactionForm>(form).unwrap();
    image.u32(0x40, reaction.original_vtable);
    image.pointer(0x44, "reaction item", !reaction.reactions_04.is_empty());
    image.pointer(0x48, "reaction next", reaction.reactions_04.len() > 1);
    image.u8(0x4C, reaction.group_type_0c);
    let race = world.get::<TesRace>(form).unwrap();
    image.bytes(0x50, &race.data_50);
    image.u32(0x74, race.attributes_74.original_vtable);
    image.bytes(0x78, &race.attributes_74.values_04);
    image.u32(0x80, race.attributes_80.original_vtable);
    image.bytes(0x84, &race.attributes_80.values_04);
    image.pointer(0x8C, "hair item", !race.hairs_8c.is_empty());
    image.pointer(0x90, "hair next", race.hairs_8c.len() > 1);
    image.form_ref(0x94, "default hair", race.default_hairs_94[0]);
    image.form_ref(0x98, "default hair", race.default_hairs_94[1]);
    image.u8(0x9C, race.hair_color_9c);
    image.f32(0xA0, race.facegen_main_a0);
    image.f32(0xA4, race.facegen_face_a4);
    image.pointer(0xA8, "eyes item", !race.eyes_a8.is_empty());
    image.pointer(0xAC, "eyes next", race.eyes_a8.len() > 1);
    for (index, part) in race.body_models_b0.iter().enumerate() {
        model(&mut image, 0xB0 + 0x18 * index, part);
    }
    for (index, part) in race.face_models_e0.iter().enumerate() {
        model(&mut image, 0xE0 + 0x18 * index, part);
    }
    for (index, part) in race.face_textures_1b8.iter().enumerate() {
        image.texture(0x1B8 + 0x0C * index, part);
    }
    for (index, part) in race.body_textures_224.iter().enumerate() {
        image.texture(0x224 + 0x0C * index, part);
    }
    for (index, matrix) in race.facegen_29c.iter().enumerate() {
        facegen(&mut image, 0x29C + 0x18 * index, matrix);
    }
    image.u16(0x2FC, race.snam_2fc);
    image.form_ref(0x300, "voice", race.voices_300[0]);
    image.form_ref(0x304, "voice", race.voices_300[1]);
    undo_array(&mut image, 0x308, &race.undo_308);
    image
}

/// IDA: N/A what a DATA row must start with: bytes (`None` is a byte the original leaves unset), or the null-ness of each dword.
pub enum Behind {
    Bytes(Vec<Option<u8>>),
    NotNull(Vec<bool>),
}

fn known(bytes: &[u8]) -> Vec<Option<u8>> {
    bytes.iter().copied().map(Some).collect()
}

fn word(value: u32) -> Vec<Option<u8>> {
    known(&value.to_le_bytes())
}

// IDA: a TESForm's head as a row dumps it behind a pointer to it: +0x04 the type, +0x0C the ID.
fn form_behind(world: &World, form: Entity) -> Behind {
    let identity = world.get::<FormIdentity>(form).unwrap();
    let mut bytes = vec![None; 4];
    bytes.push(Some(identity.form_type_04));
    bytes.extend([None; 7]);
    bytes.extend(word(identity.form_id_0c));
    Behind::Bytes(bytes)
}

fn raw_of(slot: FormRef) -> u32 {
    match slot {
        FormRef::Raw(id) => id,
        FormRef::Resolved(_) => panic!("a raw slot before InitItem"),
    }
}

// IDA: a list node past the head, behind a next pointer: {item, next}.
fn raw_node_behind(list: &[FormRef]) -> Option<Behind> {
    let second = list.get(1)?;
    let mut bytes = word(raw_of(*second));
    bytes.extend([None; 4]);
    Some(Behind::Bytes(bytes))
}

// IDA: the floats behind FgMatrix +0x0C; a row holds 0x40 bytes.
fn floats_behind(matrix: &FaceGenMatrix) -> Option<Behind> {
    (!matrix.data.is_empty()).then(|| {
        Behind::Bytes(
            matrix
                .data
                .iter()
                .flat_map(|value| word(value.to_bits()))
                .collect(),
        )
    })
}

fn text_behind(text: &[u8]) -> Behind {
    let mut bytes = known(text);
    bytes.push(Some(0));
    Behind::Bytes(bytes)
}

// IDA: what TESActorBase's pointers lead to.
fn actor_base_behind(world: &World, form: Entity, out: &mut Vec<(u32, Behind)>) {
    let base = world.get::<TesActorBaseData>(form).unwrap();
    if let Some(first) = base.factions_18.first() {
        let mut bytes = word(raw_of(first.faction_00));
        bytes.push(Some(first.rank_04 as u8));
        out.push((0x3C, Behind::Bytes(bytes)));
    }
    let container = world.get::<TesContainer>(form).unwrap();
    if let Some(first) = container.entries_08.first() {
        let mut bytes = word(first.count as u32);
        bytes.extend(word(raw_of(first.item)));
        out.push((0x4C, Behind::Bytes(bytes)));
    }
    let spells = world.get::<TesSpellList>(form).unwrap();
    if let Some(node) = raw_node_behind(&spells.spells_04) {
        out.push((0x5C, node));
    }
    let ai = world.get::<TesAiForm>(form).unwrap();
    if let Some(node) = raw_node_behind(&ai.packages_10) {
        out.push((0x7C, node));
    }
    let animation = world.get::<TesAnimation>(form).unwrap();
    if let Some(first) = animation.names_04.first() {
        out.push((0x98, text_behind(first)));
    }
    let script = world.get::<TesScriptableForm>(form).unwrap();
    if let FormRef::Resolved(Some(found)) = script.script_04 {
        out.push((0xC8, form_behind(world, found)));
    }
    let values = &world.get::<ActorBaseValues>(form).unwrap().0;
    for (offset, entry) in [(0xD8, &values.magicka_08), (0xDC, &values.fatigue_0c)] {
        if let Some(entry) = entry {
            let mut bytes = vec![Some(entry.code_00), None, None, None];
            bytes.extend(word(entry.value_04.to_bits()));
            out.push((offset, Behind::Bytes(bytes)));
        }
    }
}

/// IDA: N/A the DATA rows a sample of this form must hold, by offset.
#[must_use]
pub fn behind(world: &World, form: Entity) -> Vec<(u32, Behind)> {
    let mut out = Vec::new();
    match world.get::<FormIdentity>(form).unwrap().form_type_04 {
        0x09 => {
            let race = world.get::<TesRace>(form).unwrap();
            if let Some(first) = race.hairs_8c.first() {
                out.push((0x8C, form_behind(world, *first)));
            }
            if let Some(first) = race.eyes_a8.first() {
                out.push((0xA8, form_behind(world, *first)));
            }
            let reaction = world.get::<TesReactionForm>(form).unwrap();
            if let Some(first) = reaction.reactions_04.first() {
                let mut bytes = word(raw_of(first.faction));
                bytes.extend(word(first.modifier as u32));
                out.push((0x44, Behind::Bytes(bytes)));
            }
            let spells = world.get::<TesSpellList>(form).unwrap();
            if let Some(node) = raw_node_behind(&spells.spells_04) {
                out.push((0x34, node));
            }
            for (index, matrix) in race.facegen_29c.iter().enumerate() {
                if let Some(floats) = floats_behind(matrix) {
                    out.push((0x29C + 0x18 * index as u32 + 0x0C, floats));
                }
            }
        }
        0x0C => {
            let setting = world.get::<EffectSetting>(form).unwrap();
            if let Some(codes) = &setting.counters_9c {
                out.push((
                    0x9C,
                    Behind::Bytes(codes.iter().flat_map(|code| word(*code)).collect()),
                ));
            }
        }
        0x0F | 0x10 => {
            let list = world.get::<EffectItemList>(form).unwrap();
            if let Some(item) = list.items_04.first() {
                let mut bytes = Vec::new();
                for value in item.data_00 {
                    bytes.extend(word(value));
                }
                if item.script_18.is_none() {
                    bytes.extend(word(0));
                } else {
                    bytes.extend([None; 4]);
                }
                bytes.extend([None; 4]);
                bytes.extend(word(item.cost_20.to_bits()));
                out.push((0x28, Behind::Bytes(bytes)));
            }
        }
        0x11 => {
            let spells = world.get::<TesSpellList>(form).unwrap();
            if let Some(node) = raw_node_behind(&spells.spells_04) {
                out.push((0x40, node));
            }
        }
        0x23 => {
            actor_base_behind(world, form, &mut out);
            let npc = world.get::<TesNpc>(form).unwrap();
            for (base, set) in [(0x108, &npc.facegen_108), (0x168, &npc.facegen_168)] {
                for (index, matrix) in set.iter().enumerate() {
                    if let Some(floats) = floats_behind(matrix) {
                        out.push((base + 0x18 * index as u32 + 0x0C, floats));
                    }
                }
            }
            if let Some(hair) = npc.hair_1c8 {
                out.push((0x1C8, form_behind(world, hair)));
            }
            if let Some(eyes) = npc.eyes_1d0 {
                out.push((0x1D0, form_behind(world, eyes)));
            }
        }
        0x24 => {
            actor_base_behind(world, form, &mut out);
            let models = world.get::<TesModelList>(form).unwrap();
            if let Some(first) = models.names_04.first() {
                out.push((0xF0, text_behind(first)));
            }
            if let CreatureSoundsSlot::Sounds(sounds) =
                &world.get::<TesCreature>(form).unwrap().sounds_100
            {
                out.push((
                    0x100,
                    Behind::NotNull(sounds.lists.iter().map(Option::is_some).collect()),
                ));
            }
        }
        0x40 => {
            let list = world.get::<TesLeveledList>(form).unwrap();
            if let Some(first) = list.entries_04.first() {
                let mut bytes = known(&first.level_00.to_le_bytes());
                bytes.extend([None; 2]);
                bytes.extend(word(raw_of(first.form_04)));
                bytes.extend(known(&first.count_08.to_le_bytes()));
                out.push((0x28, Behind::Bytes(bytes)));
            }
        }
        _ => {}
    }
    out
}
