//! IDA: N/A the bytes of the AI, environment and region forms, for the sample rows. A pointer's DATA row is compared where the block behind it is modelled: a location, a target, a condition item, a CSAD block, a weather entry.

use bevy_ecs::prelude::*;
use oblivion_sim::tes::data_handler::{TesDescription, TesTexture};
use oblivion_sim::tes::form::FormIdentity;
use oblivion_sim::tes::form_components::{TesAttackDamageForm, TesModel};
use oblivion_sim::tes::forms_ai::{
    NiFormArray, TesCombatStyle, TesCondition, TesIdleForm, TesPackage,
};
use oblivion_sim::tes::forms_environment::{
    TesClimate, TesEffectShader, TesLoadScreen, TesObjectAnio, TesWaterForm, TesWeather,
    WeatherEntry,
};
use oblivion_sim::tes::forms_region::TesRegion;

use super::image::{BlockCell, Image, block_of};
use super::images_items::model_part as model;

// IDA: N/A a block of little-endian dwords, `None` a word not compared.
fn words(values: &[Option<u32>]) -> Vec<BlockCell> {
    values
        .iter()
        .flat_map(|value| match value {
            Some(value) => value.to_le_bytes().map(BlockCell::Byte),
            None => [(); 4].map(|()| BlockCell::Unset),
        })
        .collect()
}

// IDA: N/A a TESCondition's head at `at`, with the first item's 0x18 bytes behind +0x00.
fn conditions(image: &mut Image, at: usize, condition: &TesCondition) {
    let first = condition.items.first();
    let block = first.map_or_else(Vec::new, |item| block_of(&item.data_00));
    image.pointer_block(at, "condition item", first.is_some(), block);
    image.pointer(at + 4, "condition next", condition.items.len() > 1);
}

// IDA: TESWeather (0x148): +0x57 stays unset.
pub fn weather_image(world: &World, form: Entity) -> Image {
    let weather = world.get::<TesWeather>(form).unwrap();
    let mut image = Image::new(0x148);
    image.form_head(world, form);
    image.texture(0x18, &weather.textures_18[0]);
    image.texture(0x24, &weather.textures_18[1]);
    model(&mut image, 0x30, &weather.model_30);
    image.bytes(0x48, &weather.data_48);
    image.bytes(0x58, &weather.fog_58);
    image.bytes(0x68, &weather.colors_68);
    let first = weather.sounds_108.first();
    let block = first.map_or_else(Vec::new, |sound| words(&[Some(sound[0]), Some(sound[1])]));
    image.pointer_block(0x108, "sound item", first.is_some(), block);
    image.pointer(0x10C, "sound next", weather.sounds_108.len() > 1);
    image.bytes(0x110, &weather.hdr_110);
    image
}

// IDA: N/A a weather list's head at `at`, with the first entry's chance behind +0x00.
fn weather_list(image: &mut Image, at: usize, list: &[WeatherEntry]) {
    let first = list.first();
    let block = first.map_or_else(Vec::new, |entry| {
        let mut block = vec![BlockCell::Word(true)];
        block.extend(words(&[Some(entry.chance)]));
        block
    });
    image.pointer_block(at, "weather item", first.is_some(), block);
    image.pointer(at + 4, "weather next", list.len() > 1);
}

// IDA: TESClimate (0x58): +0x56 and +0x57 stay unset.
pub fn climate_image(world: &World, form: Entity) -> Image {
    let climate = world.get::<TesClimate>(form).unwrap();
    let mut image = Image::new(0x58);
    image.form_head(world, form);
    model(&mut image, 0x18, &climate.model_18);
    weather_list(&mut image, 0x30, &climate.weathers_30);
    image.texture(0x38, &climate.textures_38[0]);
    image.texture(0x44, &climate.textures_38[1]);
    image.bytes(0x50, &climate.timing_50);
    image
}

// IDA: TESRegion (0x2C). +0x18 is the data list's {item, next, owns}, and +0x1C the area list's {item, next}. The run's dump stops at those heads.
pub fn region_image(world: &World, form: Entity) -> Image {
    let region = world.get::<TesRegion>(form).unwrap();
    let mut image = Image::new(0x2C);
    image.form_head(world, form);
    let mut data_list = vec![
        BlockCell::Word(!region.data_18.items.is_empty()),
        BlockCell::Word(region.data_18.items.len() > 1),
    ];
    data_list.push(BlockCell::Byte(region.data_18.owns_08));
    image.pointer_block(0x18, "region data list", true, data_list);
    let area_list = vec![
        BlockCell::Word(!region.areas_1c.is_empty()),
        BlockCell::Word(region.areas_1c.len() > 1),
    ];
    image.pointer_block(0x1C, "region area list", true, area_list);
    image.form_ref(0x20, "worldspace", region.worldspace_20);
    image.u32(0x24, region.field_24);
    image.f32(0x28, region.field_28);
    image
}

// IDA: N/A an idle's NiFormArray fields, behind +0x3C or at a root.
fn array_block(array: &NiFormArray) -> Vec<BlockCell> {
    let mut block = words(&[Some(array.original_vtable)]);
    block.push(BlockCell::Word(array.capacity_08 != 0));
    block.extend(words(&[
        Some(array.capacity_08),
        Some(array.size_0c),
        Some(array.count_10),
        Some(array.grow_14),
    ]));
    block
}

// IDA: N/A the head of an idle behind a pointer: its vtable, type byte and form ID.
fn idle_block(world: &World, form: Option<Entity>) -> Vec<BlockCell> {
    let Some(form) = form else {
        return Vec::new();
    };
    let identity = world.get::<FormIdentity>(form).unwrap();
    let mut block = words(&[Some(0x00A5_3774), None, None, Some(identity.form_id_0c)]);
    block[4] = BlockCell::Byte(identity.form_type_04);
    block
}

// IDA: TESIdleForm (0x48): +0x39..+0x3B stay unset.
pub fn idle_image(world: &World, form: Entity) -> Image {
    let idle = world.get::<TesIdleForm>(form).unwrap();
    let mut image = Image::new(0x48);
    image.form_head(world, form);
    model(&mut image, 0x18, world.get::<TesModel>(form).unwrap());
    conditions(&mut image, 0x30, &idle.conditions_30);
    image.u8(0x38, idle.group_38);
    let children = idle.children_3c.as_ref().map_or_else(Vec::new, array_block);
    image.pointer_block(0x3C, "children", idle.children_3c.is_some(), children);
    image.pointer_block(
        0x40,
        "parent",
        idle.parent_40.is_some(),
        idle_block(world, idle.parent_40),
    );
    image.pointer_block(
        0x44,
        "previous",
        idle.previous_44.is_some(),
        idle_block(world, idle.previous_44),
    );
    image
}

// IDA: TESPackage (0x3C): +0x21..+0x23 stay unset. The location and target have +0x01..+0x03 unset.
pub fn package_image(world: &World, form: Entity) -> Image {
    let package = world.get::<TesPackage>(form).unwrap();
    let mut image = Image::new(0x3C);
    image.form_head(world, form);
    image.u32(0x18, package.field_18);
    image.u32(0x1C, package.flags_1c);
    image.u8(0x20, package.type_20 as u8);
    let location = package.location_24.as_ref().map_or_else(Vec::new, |block| {
        let mut bytes = words(&[
            None,
            Some(block.radius_04),
            Some(block.object_08.raw_word()),
        ]);
        bytes[0] = BlockCell::Byte(block.type_00 as u8);
        bytes
    });
    image.pointer_block(0x24, "location", package.location_24.is_some(), location);
    let target = package.target_28.as_ref().map_or_else(Vec::new, |block| {
        let mut bytes = words(&[None, Some(block.value_04.raw_word()), Some(block.count_08)]);
        bytes[0] = BlockCell::Byte(block.type_00);
        bytes
    });
    image.pointer_block(0x28, "target", package.target_28.is_some(), target);
    image.bytes(0x2C, &package.schedule_2c.data_00);
    image.u32(0x30, package.schedule_2c.duration_04);
    conditions(&mut image, 0x34, &package.conditions_34);
    image
}

// IDA: TESCombatStyle (0x98): before a CSTD, the padding bytes stay unset.
pub fn combat_style_image(world: &World, form: Entity) -> Image {
    const PADDING: [usize; 15] = [
        0x1A, 0x1B, 0x3E, 0x3F, 0x4D, 0x4E, 0x4F, 0x5D, 0x5E, 0x5F, 0x6A, 0x6B, 0x89, 0x8A, 0x8B,
    ];
    let style = world.get::<TesCombatStyle>(form).unwrap();
    let mut image = Image::new(0x98);
    image.form_head(world, form);
    for (index, byte) in style.data_18.iter().enumerate() {
        let at = 0x18 + index;
        if style.data_written || !PADDING.contains(&at) {
            image.u8(at, *byte);
        }
    }
    let advanced = style
        .advanced_94
        .as_ref()
        .map_or_else(Vec::new, |block| block_of(block));
    image.pointer_block(0x94, "CSAD", style.advanced_94.is_some(), advanced);
    image
}

// IDA: TESLoadScreen (0x3C): +0x2C the LNAM list's head, whose block is the last chunk read.
pub fn load_screen_image(world: &World, form: Entity) -> Image {
    let screen = world.get::<TesLoadScreen>(form).unwrap();
    let mut image = Image::new(0x3C);
    image.form_head(world, form);
    image.texture(0x18, world.get::<TesTexture>(form).unwrap());
    image.description(0x24, world.get::<TesDescription>(form).unwrap());
    let first = screen.locations_2c.first();
    let block = first.map_or_else(Vec::new, |location| {
        words(&[Some(location[0]), Some(location[1]), Some(location[2])])
    });
    image.pointer_block(0x2C, "location item", first.is_some(), block);
    image.pointer(0x30, "location next", screen.locations_2c.len() > 1);
    image.bs_string(0x34, "text", &screen.text_34);
    image
}

// IDA: TESObjectANIO (0x34).
pub fn anio_image(world: &World, form: Entity) -> Image {
    let anio = world.get::<TesObjectAnio>(form).unwrap();
    let mut image = Image::new(0x34);
    image.form_head(world, form);
    model(&mut image, 0x18, world.get::<TesModel>(form).unwrap());
    image.form_ref(0x30, "idle", anio.idle_30);
    image
}

// IDA: TESWaterForm (0xAC): +0x1E, +0x1F, +0x2E and +0x2F stay unset, and +0x75..+0x77 until a DATA reaches them.
pub fn water_image(world: &World, form: Entity) -> Image {
    let water = world.get::<TesWaterForm>(form).unwrap();
    let damage = world.get::<TesAttackDamageForm>(form).unwrap();
    let mut image = Image::new(0xAC);
    image.form_head(world, form);
    image.u32(0x18, damage.original_vtable);
    image.u16(0x1C, damage.damage_04);
    image.texture(0x20, world.get::<TesTexture>(form).unwrap());
    image.u8(0x2C, water.opacity_2c);
    image.u8(0x2D, water.flags_2d);
    image.bs_string(0x30, "material", &water.material_30);
    image.form_ref(0x38, "sound", water.sound_38);
    for (index, byte) in water.data_3c.iter().enumerate() {
        if !(0x39..=0x3B).contains(&index) || usize::from(water.data_read) > index {
            image.u8(0x3C + index, *byte);
        }
    }
    for (index, id) in water.related_a0.iter().enumerate() {
        image.form_ref(0xA0 + 4 * index, "related water", *id);
    }
    image
}

// IDA: TESEffectShader (0x110): +0x19..+0x1B stay unset until a DATA reaches them.
pub fn effect_shader_image(world: &World, form: Entity) -> Image {
    let shader = world.get::<TesEffectShader>(form).unwrap();
    let mut image = Image::new(0x110);
    image.form_head(world, form);
    for (index, byte) in shader.data_18.iter().enumerate() {
        if !(1..=3).contains(&index) || usize::from(shader.data_read) > index {
            image.u8(0x18 + index, *byte);
        }
    }
    image.texture(0xF8, &shader.texture_f8);
    image.texture(0x104, &shader.texture_104);
    image
}
