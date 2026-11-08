//! IDA: N/A each form class's bytes, for the sample rows. To add a class, add a function and one line in `image_of`. Later families can put theirs in a file of their own beside this one.

use bevy_ecs::prelude::*;
use recomp_game::tes::data_handler::{TesDescription, TesSkill, TesTexture};
use recomp_game::tes::form::FormIdentity;
use recomp_game::tes::form_components::TesFullName;
use recomp_game::tes::forms_misc::TesClass;
use recomp_game::tes::global::{EditorId, TESGlobal};

use super::image::Image;
use super::images_magic_actors::{
    birth_sign_image, creature_image, effect_setting_image, lev_spell_image, magic_form_image,
    npc_image, race_image,
};

/// IDA: N/A the image of a form by its type byte; `None` is a class with no image yet.
#[must_use]
pub fn image_of(world: &World, form: Entity) -> Option<Image> {
    let form_type = world.get::<FormIdentity>(form)?.form_type_04;
    Some(match form_type {
        0x04 => global_image(world, form),
        0x05 => class_image(world, form),
        0x0B => skill_image(world, form),
        0x2D => super::images_ai_env::weather_image(world, form),
        0x39 => super::images_dialogue::topic_image(world, form),
        0x3A => super::images_dialogue::topic_info_image(world, form),
        0x3B => super::images_dialogue::quest_image(world, form),
        0x2E => super::images_ai_env::climate_image(world, form),
        0x2F => super::images_ai_env::region_image(world, form),
        0x3C => super::images_ai_env::idle_image(world, form),
        0x3D => super::images_ai_env::package_image(world, form),
        0x3E => super::images_ai_env::combat_style_image(world, form),
        0x3F => super::images_ai_env::load_screen_image(world, form),
        0x41 => super::images_ai_env::anio_image(world, form),
        0x42 => super::images_ai_env::water_image(world, form),
        0x43 => super::images_ai_env::effect_shader_image(world, form),
        0x09 => race_image(world, form),
        0x0C => effect_setting_image(world, form),
        0x0F | 0x10 => magic_form_image(world, form),
        0x11 => birth_sign_image(world, form),
        0x23 => npc_image(world, form),
        0x24 => creature_image(world, form),
        0x40 => lev_spell_image(world, form),
        0x30 => super::images_world::cell_image(world, form),
        0x31 => super::images_world::reference_image(world, form),
        0x32 | 0x33 => super::images_world::actor_reference_image(world, form),
        0x34 => super::images_world::path_grid_image(world, form),
        0x35 => super::images_world::world_space_image(world, form),
        0x36 => super::images_world::land_image(world, form),
        0x38 => super::images_world::road_image(world, form),
        0x06 | 0x07 | 0x08 | 0x0A | 0x0D | 0x0E | 0x12..=0x22 | 0x25..=0x2B => {
            super::images_items::image(world, form, form_type)
        }
        _ => return None,
    })
}

// IDA: TESGlobal (0x28): +0x18 BSStringT, +0x20 variable type, +0x24 value.
fn global_image(world: &World, form: Entity) -> Image {
    let mut image = Image::new(0x28);
    image.form_head(world, form);
    image.bs_string(0x18, "editor ID", &world.get::<EditorId>(form).unwrap().0);
    let global = world.get::<TESGlobal>(form).unwrap();
    image.u8(0x20, global.variable_type_20);
    image.u32(0x24, global.value_bits_24);
    image
}

// IDA: TESClass (0x6C): +0x18 TESFullName, +0x24 TESDescription, +0x2C TESTexture, +0x38 DATA.
fn class_image(world: &World, form: Entity) -> Image {
    let mut image = Image::new(0x6C);
    image.form_head(world, form);
    image.full_name(0x18, world.get::<TesFullName>(form).unwrap());
    image.description(0x24, world.get::<TesDescription>(form).unwrap());
    image.texture(0x2C, world.get::<TesTexture>(form).unwrap());
    image.bytes(0x38, &world.get::<TesClass>(form).unwrap().data_38_bytes());
    image
}

// IDA: TESSkill (0x60): +0x18 TESDescription, +0x20 TESTexture, +0x2C..+0x40 DATA, +0x40..+0x60 four TESDescriptions.
fn skill_image(world: &World, form: Entity) -> Image {
    let skill = world.get::<TesSkill>(form).unwrap();
    let mut image = Image::new(0x60);
    image.form_head(world, form);
    image.description(0x18, &skill.description_18);
    image.texture(0x20, &skill.texture_20);
    image.bytes(0x2C, &skill.data_2c_bytes());
    for (index, description) in skill.level_descriptions_40.iter().enumerate() {
        image.description(0x40 + 8 * index, description);
    }
    image
}
