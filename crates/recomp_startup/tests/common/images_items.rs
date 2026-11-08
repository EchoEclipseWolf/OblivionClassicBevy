//! IDA: N/A the bytes of the misc, script and item classes, for the sample rows.

use bevy_ecs::prelude::*;
use recomp_game::tes::data_handler::{TesDescription, TesTexture};
use recomp_game::tes::form_components::{
    TesAttackDamageForm, TesBipedModelForm, TesContainer, TesEnchantableForm, TesFullName,
    TesHealthForm, TesLeveledList, TesModel, TesObjectLinks, TesQualityForm, TesReactionForm,
    TesScriptableForm, TesSoundFile, TesUsesForm, TesValueForm, TesWeightForm,
};
use recomp_game::tes::forms_items::{
    NiTArrayU32, TesActivator, TesAmmo, TesApparatus, TesArmor, TesBook, TesContainerObject,
    TesDoor, TesFurniture, TesGrass, TesLevCreature, TesLight, TesMagicObject, TesSoulGem,
    TesSubSpace, TesTree, TesWeapon,
};
use recomp_game::tes::forms_misc::{TesEyes, TesFaction, TesHair, TesLandTexture, TesSound};
use recomp_game::tes::forms_script::Script;

use recomp_game::tes::forms_magic::EffectItemList;

use super::image::{BlockCell, Image, block_of};
use super::images_magic_actors::object_neighbours_known;

// IDA: 004B2FF0 TESObject: +0x18 list, +0x1C previous, +0x20 next.
fn links(image: &mut Image, world: &World, form: Entity) {
    let links = world.get::<TesObjectLinks>(form).unwrap();
    image.pointer(0x18, "TESObject list", links.in_list_18);
    // A list end here can be a neighbour in the run, of a class not yet in the tree: only then are those two cells one-sided.
    if object_neighbours_known() || links.previous_1c.is_some() {
        image.pointer(0x1C, "TESObject previous", links.previous_1c.is_some());
    }
    if object_neighbours_known() || links.next_20.is_some() {
        image.pointer(0x20, "TESObject next", links.next_20.is_some());
    }
}

// IDA: TESModel (0x18) as a DATA row sees the +0x14 array: a word per entry.
pub(super) fn model_part(image: &mut Image, at: usize, model: &TesModel) {
    image.u32(at, model.original_vtable);
    image.bs_string(at + 4, "model path", &model.path_04);
    image.f32(at + 0x0C, model.bound_radius_0c);
    image.u8(at + 0x10, model.texture_count_10);
    let entries = model.texture_entries_14.as_ref();
    let block = entries.map_or_else(Vec::new, |entries| {
        entries
            .iter()
            .map(|entry| BlockCell::Word(entry.is_some()))
            .collect()
    });
    image.pointer_block(at + 0x14, "model textures", entries.is_some(), block);
}

fn model(image: &mut Image, world: &World, form: Entity, at: usize) {
    model_part(image, at, world.get::<TesModel>(form).unwrap());
}

fn full(image: &mut Image, world: &World, form: Entity, at: usize) {
    image.full_name(at, world.get::<TesFullName>(form).unwrap());
}

fn icon(image: &mut Image, world: &World, form: Entity, at: usize) {
    image.texture(at, world.get::<TesTexture>(form).unwrap());
}

// IDA: TESScriptableForm (0x0C): +0x04 the script, +0x08 byte resolved.
fn script(image: &mut Image, world: &World, form: Entity, at: usize) {
    let part = world.get::<TesScriptableForm>(form).unwrap();
    image.u32(at, part.original_vtable);
    image.form_ref(at + 4, "script", part.script_04);
    image.u8(at + 8, part.resolved_08);
}

fn value(image: &mut Image, world: &World, form: Entity, at: usize) {
    let part = world.get::<TesValueForm>(form).unwrap();
    image.u32(at, part.original_vtable);
    image.i32(at + 4, part.value_04);
}

fn weight(image: &mut Image, world: &World, form: Entity, at: usize) {
    let part = world.get::<TesWeightForm>(form).unwrap();
    image.u32(at, part.original_vtable);
    image.f32(at + 4, part.weight_04);
}

fn health(image: &mut Image, world: &World, form: Entity, at: usize) {
    let part = world.get::<TesHealthForm>(form).unwrap();
    image.u32(at, part.original_vtable);
    image.u32(at + 4, part.health_04);
}

fn damage(image: &mut Image, world: &World, form: Entity, at: usize) {
    let part = world.get::<TesAttackDamageForm>(form).unwrap();
    image.u32(at, part.original_vtable);
    image.u16(at + 4, part.damage_04);
}

// IDA: TESEnchantableForm (0x10).
fn enchantment(image: &mut Image, world: &World, form: Entity, at: usize) {
    let part = world.get::<TesEnchantableForm>(form).unwrap();
    image.u32(at, part.original_vtable);
    image.form_ref(at + 4, "enchantment", part.enchantment_04);
    image.u16(at + 8, part.charge_08);
    image.u32(at + 0x0C, part.cast_type_0c);
}

// IDA: TESBipedModelForm (0x80).
fn biped(image: &mut Image, world: &World, form: Entity, at: usize) {
    let part = world.get::<TesBipedModelForm>(form).unwrap();
    image.u32(at, part.original_vtable);
    image.u16(at + 4, part.slot_mask_04);
    image.u8(at + 6, part.flags_06);
    model_part(image, at + 0x08, &part.biped_models_08[0]);
    model_part(image, at + 0x20, &part.biped_models_08[1]);
    model_part(image, at + 0x38, &part.world_models_38[0]);
    model_part(image, at + 0x50, &part.world_models_38[1]);
    image.texture(at + 0x68, &part.icons_68[0]);
    image.texture(at + 0x74, &part.icons_68[1]);
}

// IDA: a BSSimpleList head: +0 the item, with its block; +4 the next node.
fn simple_list(
    image: &mut Image,
    at: usize,
    role: &'static str,
    count: usize,
    block: Vec<BlockCell>,
) {
    image.pointer_block(at, role, count > 0, block);
    image.pointer(at + 4, "list next", count > 1);
}

// IDA: EffectItemList (0x10), with the head item's 0x24 bytes behind +0x04.
fn effects(image: &mut Image, world: &World, form: Entity, at: usize) {
    let list = world.get::<EffectItemList>(form).unwrap();
    image.u32(at, list.original_vtable);
    let block = list.items_04.first().map_or_else(Vec::new, |item| {
        let mut block = Vec::new();
        for word in item.data_00 {
            block.extend(block_of(&word.to_le_bytes()));
        }
        block.push(BlockCell::Word(item.script_18.is_some()));
        block.push(BlockCell::Word(item.setting_1c.is_some()));
        block.extend(block_of(&item.cost_20.to_le_bytes()));
        block
    });
    simple_list(image, at + 4, "effect item", list.items_04.len(), block);
    image.u32(at + 0x0C, list.hostile_count_0c);
}

// IDA: TESObjectMISC's parts, shared with KEYM, SLGM and SGST.
fn misc_parts(image: &mut Image, world: &World, form: Entity) {
    image.form_head(world, form);
    links(image, world, form);
    full(image, world, form, 0x24);
    model(image, world, form, 0x30);
    icon(image, world, form, 0x48);
    script(image, world, form, 0x54);
    value(image, world, form, 0x60);
    weight(image, world, form, 0x68);
}

// IDA: TESObjectACTI's parts, shared with FLOR and FURN.
fn activator_parts(image: &mut Image, world: &World, form: Entity) {
    image.form_head(world, form);
    links(image, world, form);
    full(image, world, form, 0x24);
    model(image, world, form, 0x30);
    script(image, world, form, 0x48);
    image.form_ref(
        0x54,
        "sound",
        world.get::<TesActivator>(form).unwrap().sound_54,
    );
}

// IDA: TESLeveledList (0x10), with the head entry's 0x0C bytes behind +0x04.
fn leveled(image: &mut Image, world: &World, form: Entity, at: usize) {
    let list = world.get::<TesLeveledList>(form).unwrap();
    image.u32(at, list.original_vtable);
    let block = list.entries_04.first().map_or_else(Vec::new, |entry| {
        let mut block = block_of(&entry.level_00.to_le_bytes());
        block.extend([BlockCell::Unset, BlockCell::Unset]);
        match entry.form_04 {
            recomp_game::tes::form_components::FormRef::Raw(id) => {
                block.extend(block_of(&id.to_le_bytes()));
            }
            recomp_game::tes::form_components::FormRef::Resolved(found) => {
                block.push(BlockCell::Word(found.is_some()));
            }
        }
        block.extend(block_of(&entry.count_08.to_le_bytes()));
        block
    });
    simple_list(image, at + 4, "leveled entry", list.entries_04.len(), block);
    image.u8(at + 0x0C, list.chance_none_0c);
    image.u8(at + 0x0D, list.flags_0d);
}

// IDA: the array behind NiTArray<uint> +0x04.
fn seeds(image: &mut Image, at: usize, array: &NiTArrayU32) {
    image.u32(at, array.original_vtable);
    let block = array.data_04.as_ref().map_or_else(Vec::new, |data| {
        data.iter()
            .flat_map(|seed| block_of(&seed.to_le_bytes()))
            .collect()
    });
    image.pointer_block(at + 4, "tree seeds", array.data_04.is_some(), block);
    image.u16(at + 8, array.capacity_08);
    image.u16(at + 0x0A, array.size_0a);
    image.u16(at + 0x0C, array.occupied_0c);
    image.u16(at + 0x0E, array.grow_by_0e);
}

/// IDA: N/A a form of these classes, as the row dumps it.
#[must_use]
pub fn image(world: &World, form: Entity, form_type: u8) -> Image {
    let size = match form_type {
        0x06 | 0x0A | 0x25 => 0x44,
        0x07 => 0x4C,
        0x08 | 0x0E | 0x2B => 0x34,
        0x0D => 0x50,
        0x12 => 0x58,
        0x13 | 0x17 => 0x7C,
        0x14 => 0xE8,
        0x15 => 0x8C,
        0x16 => 0xDC,
        0x18 | 0x1B | 0x27 => 0x70,
        0x19 | 0x1E | 0x28 => 0x80,
        0x1A => 0x90,
        0x1C => 0x3C,
        0x1D | 0x20 => 0x5C,
        0x1F => 0x64,
        0x21 => 0xA0,
        0x22 => 0x84,
        0x26 => 0x74,
        0x29 => 0x30,
        0x2A => 0x88,
        _ => panic!("{form_type:02X}: no size"),
    };
    let mut image = Image::new(size);
    match form_type {
        0x06 => {
            image.form_head(world, form);
            full(&mut image, world, form, 0x18);
            let reaction = world.get::<TesReactionForm>(form).unwrap();
            image.u32(0x24, reaction.original_vtable);
            let block = reaction.reactions_04.first().map_or_else(Vec::new, |item| {
                let mut block = match item.faction {
                    recomp_game::tes::form_components::FormRef::Raw(id) => {
                        block_of(&id.to_le_bytes())
                    }
                    recomp_game::tes::form_components::FormRef::Resolved(found) => {
                        vec![BlockCell::Word(found.is_some())]
                    }
                };
                block.extend(block_of(&item.modifier.to_le_bytes()));
                block
            });
            simple_list(
                &mut image,
                0x28,
                "reaction",
                reaction.reactions_04.len(),
                block,
            );
            image.u8(0x30, reaction.group_type_0c);
            let faction = world.get::<TesFaction>(form).unwrap();
            image.u8(0x34, faction.flags_34);
            image.f32(0x38, faction.crime_gold_multiplier_38);
            let block = faction.ranks_3c.first().map_or_else(Vec::new, |rank| {
                let mut block = Vec::new();
                for text in [&rank.male_00, &rank.female_08] {
                    block.push(BlockCell::Word(text.data_00.is_some()));
                    block.extend(block_of(&text.length_04.to_le_bytes()));
                    block.extend(block_of(&text.capacity_06.to_le_bytes()));
                }
                block.extend(block_of(&rank.insignia_10.original_vtable.to_le_bytes()));
                block.push(BlockCell::Word(rank.insignia_10.path_04.data_00.is_some()));
                block.extend(block_of(&rank.insignia_10.path_04.length_04.to_le_bytes()));
                block.extend(block_of(
                    &rank.insignia_10.path_04.capacity_06.to_le_bytes(),
                ));
                block
            });
            simple_list(&mut image, 0x3C, "rank", faction.ranks_3c.len(), block);
        }
        0x07 => {
            image.form_head(world, form);
            full(&mut image, world, form, 0x18);
            model(&mut image, world, form, 0x24);
            icon(&mut image, world, form, 0x3C);
            image.u8(0x48, world.get::<TesHair>(form).unwrap().flags_48);
        }
        0x08 => {
            image.form_head(world, form);
            full(&mut image, world, form, 0x18);
            icon(&mut image, world, form, 0x24);
            image.u8(0x30, world.get::<TesEyes>(form).unwrap().flags_30);
        }
        0x0A => {
            image.form_head(world, form);
            links(&mut image, world, form);
            let file = world.get::<TesSoundFile>(form).unwrap();
            image.u32(0x24, file.original_vtable);
            image.bs_string(0x28, "sound file", &file.path_04);
            let sound = world.get::<TesSound>(form).unwrap();
            image.bs_string(0x30, "editor ID", &sound.editor_id_30);
            image.bytes(0x38, &sound.data_38_bytes());
        }
        0x0D => {
            let script = world.get::<Script>(form).unwrap();
            image.form_head(world, form);
            image.bytes(0x18, &script.header_18);
            image.u32(0x2C, script.text_2c);
            let block = script
                .data_30
                .as_ref()
                .map_or_else(Vec::new, |data| block_of(data));
            image.pointer_block(0x30, "compiled data", script.data_30.is_some(), block);
            for (index, float) in script.floats_34.iter().enumerate() {
                image.f32(0x34 + 4 * index, *float);
            }
            let block = script.references_40.first().map_or_else(Vec::new, |item| {
                let mut block = vec![BlockCell::Word(item.name_00.data_00.is_some())];
                block.extend(block_of(&item.name_00.length_04.to_le_bytes()));
                block.extend(block_of(&item.name_00.capacity_06.to_le_bytes()));
                block.extend(block_of(&item.form_08.to_le_bytes()));
                block.extend(block_of(&item.variable_0c.to_le_bytes()));
                block
            });
            simple_list(
                &mut image,
                0x40,
                "script reference",
                script.references_40.len(),
                block,
            );
            let block = script.variables_48.first().map_or_else(Vec::new, |item| {
                let mut block = block_of(&item.data_00);
                block.push(BlockCell::Word(item.name_18.data_00.is_some()));
                block.extend(block_of(&item.name_18.length_04.to_le_bytes()));
                block.extend(block_of(&item.name_18.capacity_06.to_le_bytes()));
                block
            });
            simple_list(
                &mut image,
                0x48,
                "script variable",
                script.variables_48.len(),
                block,
            );
        }
        0x0E => {
            image.form_head(world, form);
            icon(&mut image, world, form, 0x18);
            let land = world.get::<TesLandTexture>(form).unwrap();
            image.u32(0x24, land.object_24);
            image.bytes(0x28, &land.havok_28);
            match land.grasses_2c.first() {
                Some(first) => image.form_ref(0x2C, "grass", *first),
                None => image.u32(0x2C, 0),
            }
            image.pointer(0x30, "list next", land.grasses_2c.len() > 1);
        }
        0x12 => activator_parts(&mut image, world, form),
        0x13 => {
            misc_parts(&mut image, world, form);
            let quality = world.get::<TesQualityForm>(form).unwrap();
            image.u32(0x70, quality.original_vtable);
            image.f32(0x74, quality.quality_04);
            image.u8(0x78, world.get::<TesApparatus>(form).unwrap().type_78);
        }
        0x14 | 0x16 => {
            image.form_head(world, form);
            links(&mut image, world, form);
            full(&mut image, world, form, 0x24);
            script(&mut image, world, form, 0x30);
            enchantment(&mut image, world, form, 0x3C);
            value(&mut image, world, form, 0x4C);
            weight(&mut image, world, form, 0x54);
            if form_type == 0x14 {
                health(&mut image, world, form, 0x5C);
                biped(&mut image, world, form, 0x64);
                image.u16(0xE4, world.get::<TesArmor>(form).unwrap().armor_rating_e4);
            } else {
                biped(&mut image, world, form, 0x5C);
            }
        }
        0x15 => {
            image.form_head(world, form);
            links(&mut image, world, form);
            full(&mut image, world, form, 0x24);
            model(&mut image, world, form, 0x30);
            icon(&mut image, world, form, 0x48);
            script(&mut image, world, form, 0x54);
            enchantment(&mut image, world, form, 0x60);
            value(&mut image, world, form, 0x70);
            weight(&mut image, world, form, 0x78);
            image.description(0x80, world.get::<TesDescription>(form).unwrap());
            let book = world.get::<TesBook>(form).unwrap();
            image.u8(0x88, book.flags_88);
            image.u8(0x89, book.teaches_89);
        }
        0x17 => {
            image.form_head(world, form);
            links(&mut image, world, form);
            let container = world.get::<TesContainer>(form).unwrap();
            image.u32(0x24, container.original_vtable);
            image.u8(0x28, container.flags_04);
            let block = container.entries_08.first().map_or_else(Vec::new, |entry| {
                let mut block = block_of(&entry.count.to_le_bytes());
                match entry.item {
                    recomp_game::tes::form_components::FormRef::Raw(id) => {
                        block.extend(block_of(&id.to_le_bytes()));
                    }
                    recomp_game::tes::form_components::FormRef::Resolved(found) => {
                        block.push(BlockCell::Word(found.is_some()));
                    }
                }
                block
            });
            simple_list(
                &mut image,
                0x2C,
                "container entry",
                container.entries_08.len(),
                block,
            );
            full(&mut image, world, form, 0x34);
            model(&mut image, world, form, 0x40);
            script(&mut image, world, form, 0x58);
            weight(&mut image, world, form, 0x64);
            let object = world.get::<TesContainerObject>(form).unwrap();
            image.form_ref(0x70, "open sound", object.open_sound_70);
            image.form_ref(0x74, "close sound", object.close_sound_74);
            image.u8(0x78, object.flags_78);
        }
        0x18 => {
            image.form_head(world, form);
            links(&mut image, world, form);
            full(&mut image, world, form, 0x24);
            model(&mut image, world, form, 0x30);
            script(&mut image, world, form, 0x48);
            let door = world.get::<TesDoor>(form).unwrap();
            image.pointer(0x58, "open sound", door.open_sound_58.is_some());
            image.pointer(0x5C, "close sound", door.close_sound_5c.is_some());
            image.pointer(0x60, "loop sound", door.loop_sound_60.is_some());
            image.u8(0x64, door.flags_64);
            image.form_ref(
                0x68,
                "teleport item",
                door.teleports_68.first().copied().unwrap_or_default(),
            );
            image.pointer(0x6C, "list next", door.teleports_68.len() > 1);
        }
        0x19 | 0x28 => {
            image.form_head(world, form);
            links(&mut image, world, form);
            full(&mut image, world, form, 0x24);
            effects(&mut image, world, form, 0x30);
            model(&mut image, world, form, 0x40);
            icon(&mut image, world, form, 0x58);
            script(&mut image, world, form, 0x64);
            weight(&mut image, world, form, 0x70);
            let object = world.get::<TesMagicObject>(form).unwrap();
            image.i32(0x78, object.value_78);
            image.u8(0x7C, object.flags_7c);
            if let Some(rest) = object.rest_7d {
                image.bytes(0x7D, &rest);
            }
        }
        0x1A => {
            image.form_head(world, form);
            links(&mut image, world, form);
            full(&mut image, world, form, 0x24);
            model(&mut image, world, form, 0x30);
            icon(&mut image, world, form, 0x48);
            script(&mut image, world, form, 0x54);
            weight(&mut image, world, form, 0x60);
            value(&mut image, world, form, 0x68);
            let light = world.get::<TesLight>(form).unwrap();
            image.bytes(0x70, &light.data_70_bytes());
            image.f32(0x88, light.fade_88);
            image.form_ref(0x8C, "sound", light.sound_8c);
        }
        0x1B | 0x27 => misc_parts(&mut image, world, form),
        0x1C => {
            image.form_head(world, form);
            links(&mut image, world, form);
            model(&mut image, world, form, 0x24);
        }
        0x1D => {
            image.form_head(world, form);
            links(&mut image, world, form);
            model(&mut image, world, form, 0x24);
            let grass = world.get::<TesGrass>(form).unwrap();
            let bytes = grass.data_3c_bytes();
            image.bytes(0x3C, &bytes[..3]);
            image.bytes(0x40, &bytes[4..6]);
            image.bytes(0x44, &bytes[8..0x1D]);
        }
        0x1E => {
            image.form_head(world, form);
            links(&mut image, world, form);
            model(&mut image, world, form, 0x24);
            icon(&mut image, world, form, 0x3C);
            let tree = world.get::<TesTree>(form).unwrap();
            seeds(&mut image, 0x48, &tree.seeds_48);
            for (index, word) in tree.parameters_58.iter().enumerate() {
                image.u32(0x58 + 4 * index, *word);
            }
            image.u32(0x78, tree.billboard_78[0]);
            image.u32(0x7C, tree.billboard_78[1]);
        }
        // The dump reads past the block, since the form is +0x0C into it: the last 0x0C bytes aren't this form's.
        0x1F => activator_parts(&mut image, world, form),
        0x20 => {
            activator_parts(&mut image, world, form);
            image.u32(
                0x58,
                world.get::<TesFurniture>(form).unwrap().marker_flags_58,
            );
        }
        0x21 => {
            image.form_head(world, form);
            links(&mut image, world, form);
            full(&mut image, world, form, 0x24);
            model(&mut image, world, form, 0x30);
            icon(&mut image, world, form, 0x48);
            script(&mut image, world, form, 0x54);
            enchantment(&mut image, world, form, 0x60);
            value(&mut image, world, form, 0x70);
            weight(&mut image, world, form, 0x78);
            health(&mut image, world, form, 0x80);
            damage(&mut image, world, form, 0x88);
            image.bytes(0x90, &world.get::<TesWeapon>(form).unwrap().data_90);
        }
        0x22 => {
            image.form_head(world, form);
            links(&mut image, world, form);
            full(&mut image, world, form, 0x24);
            model(&mut image, world, form, 0x30);
            icon(&mut image, world, form, 0x48);
            enchantment(&mut image, world, form, 0x54);
            value(&mut image, world, form, 0x64);
            weight(&mut image, world, form, 0x6C);
            damage(&mut image, world, form, 0x74);
            image.bytes(0x7C, &world.get::<TesAmmo>(form).unwrap().data_7c);
        }
        0x25 => {
            image.form_head(world, form);
            links(&mut image, world, form);
            leveled(&mut image, world, form, 0x24);
            script(&mut image, world, form, 0x34);
            image.form_ref(
                0x40,
                "template",
                world.get::<TesLevCreature>(form).unwrap().template_40,
            );
        }
        0x26 => {
            misc_parts(&mut image, world, form);
            let gem = world.get::<TesSoulGem>(form).unwrap();
            image.u8(0x70, gem.soul_70);
            image.u8(0x71, gem.capacity_71);
        }
        0x29 => {
            image.form_head(world, form);
            links(&mut image, world, form);
            let space = world.get::<TesSubSpace>(form).unwrap();
            for (index, extent) in space.extents_24.iter().enumerate() {
                image.u16(0x24 + 2 * index, *extent);
            }
            image.f32(0x2C, space.radius_2c);
        }
        0x2A => {
            misc_parts(&mut image, world, form);
            let uses = world.get::<TesUsesForm>(form).unwrap();
            image.u32(0x70, uses.original_vtable);
            image.u8(0x74, uses.uses_04);
            effects(&mut image, world, form, 0x78);
        }
        0x2B => {
            image.form_head(world, form);
            links(&mut image, world, form);
            leveled(&mut image, world, form, 0x24);
        }
        _ => unreachable!(),
    }
    image
}
