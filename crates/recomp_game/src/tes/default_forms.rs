//! Phase 12d: 0044AC20 DataHandler_CreateDefaultForms, called at 0044F6CB after the compile pass and before the record loop. Each default looks its ID up in g_formsByID, most through a cast, and when absent constructs the form (FormHeap and the class constructor), gives it its ID (0046C300(id, 1)), its name (vt+0xD8), the class defaults and its list; most then take vt+0x90(0). The record loop later loads the master over the forms it also defines.
//!
//! The arms stay with their classes; this file holds the function's order, read from the exe 0044AC56..0044CD71, and its statics. P12-RUN-001's AC20 rows check the forms; no row can see the order between defaults. Evidence: objdump of 0044AC20, 0041BB80, 00518000.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::form_hierarchy::RttiClass;
use super::forms_actors::{create_default_player_npc, create_default_vampire_race};
use super::forms_dialogue::create_default_topics;
use super::forms_environment::{
    create_default_climate, create_default_effect_shaders, create_default_water,
    create_default_weather,
};
use super::forms_init::lookup;
use super::forms_items::{
    FOOTSTEP_SOUNDS, MAGIC_SOUNDS, default_clothing, default_creature_faction, default_eyes,
    default_object, default_sound,
};
use super::forms_magic::create_default_spells;
use super::forms_world::create_default_world_space;
use super::global::create_default_globals;
use super::sky::sky_set_climate_default;
use super::tes::GameThreadLocals;

/// IDA: the statics 0044AC20 stores, cleared at 0044AC5D..0044ACD5 where it says, then set to the form found or created.
#[derive(Resource, Debug, Default)]
pub struct DefaultFormStatics {
    pub door_marker_b35ea4: Option<Entity>,
    pub map_marker_b35ea8: Option<Entity>,
    pub x_marker_b35eac: Option<Entity>,
    pub x_marker_heading_b35eb0: Option<Entity>,
    pub travel_marker_b35eb4: Option<Entity>,
    pub north_marker_b35eb8: Option<Entity>,
    pub prison_marker_b35ebc: Option<Entity>,
    pub temple_marker_b35ec0: Option<Entity>,
    pub divine_marker_b35ec4: Option<Entity>,
    pub lockpick_b35ec8: Option<Entity>,
    pub skeleton_key_b35ecc: Option<Entity>,
    pub repair_hammer_b35ed0: Option<Entity>,
    pub horse_marker_b35ed4: Option<Entity>,
    pub welkynd_stone_b35ed8: Option<Entity>,
    pub varla_stone_b35edc: Option<Entity>,
    pub black_soul_gem_b35ee0: Option<Entity>,
    pub azura_stone_b35ee4: Option<Entity>,
    pub default_water_b360ac: Option<Entity>,
    pub creature_faction_b36298: Option<Entity>,
    pub eyes_reanimate_b362bc: Option<Entity>,
    pub vampire_race_b36308: Option<Entity>,
    /// 00B33AA8..00B33AB8: stored here too, but not among the clears.
    pub loot_bag_b33aa8: Option<Entity>,
    pub stolen_goods_b33aac: Option<Entity>,
    pub jail_shirt_b33ab0: Option<Entity>,
    pub jail_pants_b33ab4: Option<Entity>,
    pub jail_shoes_b33ab8: Option<Entity>,
    /// 00B33560..00B335A4, the 18 magic sounds of 0041BB80 in its order.
    pub magic_sounds_b33560: Vec<Option<Entity>>,
    /// 00B335A8 and 00B335AC, the two default spells.
    pub default_spells_b335a8: [Option<Entity>; 2],
    /// 00B335B0..00B335B8, the three effect shaders.
    pub effect_shaders_b335b0: Vec<Option<Entity>>,
    /// The 00518000 sounds, indexed by address from 00B36218. Call order differs (`FOOTSTEP_SOUND_ADDRESSES`), and 006B16E5 reads by address.
    pub footstep_sounds_b36218: [Option<Entity>; 25],
    /// IDA: N/A the calls of 00543200 whose graphics a host hasn't drawn.
    pub sky_climate_calls: u32,
}

/// IDA: 00518000's stores, in call order: form ID and static. Not in address order (the C++ keeps call order).
#[rustfmt::skip]
const FOOTSTEP_SOUND_ADDRESSES: [(u32, u32); 25] = [(0x212, 0x00B36218), (0x22B, 0x00B36220), (0x213, 0x00B3621C), (0x214, 0x00B36224), (0x215, 0x00B36228), (0x216, 0x00B3622C), (0x21F, 0x00B36230), (0x217, 0x00B36234), (0x218, 0x00B36238), (0x229, 0x00B36274), (0x228, 0x00B36278), (0x219, 0x00B3623C), (0x21A, 0x00B36240), (0x21B, 0x00B36244), (0x21C, 0x00B36248), (0x21D, 0x00B3624C), (0x21E, 0x00B36250), (0x220, 0x00B36254), (0x221, 0x00B36258), (0x222, 0x00B3625C), (0x223, 0x00B36260), (0x225, 0x00B36264), (0x226, 0x00B36268), (0x227, 0x00B3626C), (0x224, 0x00B36270)];

// IDA: the STAT arm: `default_object` over a TESObjectSTAT cast.
fn stat(world: &mut World, id: u32, model: &[u8]) -> Result<Option<Entity>, StreamError> {
    default_object(world, 0x1C, RttiClass::TESObjectSTAT, id, Some(model))
}

// IDA: N/A a helper, with no address of its own.
fn misc(world: &mut World, id: u32) -> Result<Option<Entity>, StreamError> {
    default_object(world, 0x1B, RttiClass::TESObjectMISC, id, None)
}

// IDA: N/A a helper, with no address of its own.
fn soul_gem(world: &mut World, id: u32) -> Result<Option<Entity>, StreamError> {
    default_object(world, 0x26, RttiClass::TESSoulGem, id, None)
}

// IDA: a default whose lookup has no cast: any form at the ID stays.
fn uncast(
    world: &mut World,
    form_type: u8,
    id: u32,
    model: Option<&[u8]>,
) -> Result<Option<Entity>, StreamError> {
    if let Some(found) = lookup(world, id) {
        return Ok(Some(found));
    }
    default_object(world, form_type, RttiClass::TESForm, id, model)
}

/// IDA: 00402860(&g_timeGlobals 00B332E0) the six lookups, which 0044F903 makes after the topics sort.
#[derive(Resource, Debug, Default, PartialEq, Eq)]
pub struct TimeGlobals {
    pub globals_b332e0: [Option<Entity>; 6],
}

/// IDA: 00402860 TimeGlobals_Init: 0046B250 of 0x35..=0x3A, in order. Completion: 100%.
pub fn init_time_globals(world: &mut World) {
    let mut globals = [None; 6];
    for (index, slot) in globals.iter_mut().enumerate() {
        *slot = lookup(world, 0x35 + index as u32);
    }
    world.insert_resource(TimeGlobals {
        globals_b332e0: globals,
    });
}

/// IDA: 0044AC20 DataHandler_CreateDefaultForms, with TLS +0x184 set by 0044F3F9. Completion: 100% of the order and the forms; 00543200's graphics are a host boundary, counted.
/// TODO: [ ] 0044CD49 00543200's sky graphics (`sky_set_climate_default`).
pub fn create_default_forms(world: &mut World) -> Result<(), StreamError> {
    debug_assert!(world.resource::<GameThreadLocals>().byte_184 != 0);
    // 0044AC5D..0044ACD5.
    let surviving = std::mem::take(world.get_resource_or_init::<DefaultFormStatics>().as_mut());
    {
        let mut statics = world.resource_mut::<DefaultFormStatics>();
        statics.loot_bag_b33aa8 = surviving.loot_bag_b33aa8;
        statics.stolen_goods_b33aac = surviving.stolen_goods_b33aac;
        statics.jail_shirt_b33ab0 = surviving.jail_shirt_b33ab0;
        statics.jail_pants_b33ab4 = surviving.jail_pants_b33ab4;
        statics.jail_shoes_b33ab8 = surviving.jail_shoes_b33ab8;
    }
    let eyes = default_eyes(world)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .eyes_reanimate_b362bc = Some(eyes);
    let race = create_default_vampire_race(world)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .vampire_race_b36308 = Some(race);
    let water = create_default_water(world)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .default_water_b360ac = Some(water);
    let form = stat(world, 0x12, b"Marker_Horse.nif")?;
    world
        .resource_mut::<DefaultFormStatics>()
        .horse_marker_b35ed4 = form;
    let form = misc(world, 0x194)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .varla_stone_b35edc = form;
    let form = misc(world, 0x191)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .welkynd_stone_b35ed8 = form;
    let form = soul_gem(world, 0x192)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .black_soul_gem_b35ee0 = form;
    let form = soul_gem(world, 0x193)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .azura_stone_b35ee4 = form;
    let form = misc(world, 0x0A)?;
    world.resource_mut::<DefaultFormStatics>().lockpick_b35ec8 = form;
    let form = misc(world, 0x0B)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .skeleton_key_b35ecc = form;
    let form = misc(world, 0x0C)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .repair_hammer_b35ed0 = form;
    let form = stat(world, 0x01, b"MarkerTeleport.nif")?;
    world
        .resource_mut::<DefaultFormStatics>()
        .door_marker_b35ea4 = form;
    create_default_world_space(world)?;
    let form = stat(world, 0x3B, b"MarkerX.nif")?;
    world.resource_mut::<DefaultFormStatics>().x_marker_b35eac = form;
    let form = stat(world, 0x34, b"MarkerXHeading.nif")?;
    world
        .resource_mut::<DefaultFormStatics>()
        .x_marker_heading_b35eb0 = form;
    let form = stat(world, 0x10, b"Marker_Map.NIF")?;
    world.resource_mut::<DefaultFormStatics>().map_marker_b35ea8 = form;
    let form = stat(world, 0x02, b"Marker_Travel.nif")?;
    world
        .resource_mut::<DefaultFormStatics>()
        .travel_marker_b35eb4 = form;
    let form = stat(world, 0x03, b"Marker_North.nif")?;
    world
        .resource_mut::<DefaultFormStatics>()
        .north_marker_b35eb8 = form;
    let form = default_object(
        world,
        0x18,
        RttiClass::TESObjectDOOR,
        0x04,
        Some(b"Marker_Prison.nif"),
    )?;
    world
        .resource_mut::<DefaultFormStatics>()
        .prison_marker_b35ebc = form;
    let form = stat(world, 0x06, b"Marker_Temple.nif")?;
    world
        .resource_mut::<DefaultFormStatics>()
        .temple_marker_b35ec0 = form;
    let form = stat(world, 0x05, b"Marker_Divine.nif")?;
    world
        .resource_mut::<DefaultFormStatics>()
        .divine_marker_b35ec4 = form;
    let form = default_creature_faction(world)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .creature_faction_b36298 = Some(form);
    // 0x0F Gold001, 0x0E LootBag, 0x11 StolenGoods, with no cast. 0044BE74..0044BEA9: the bag's empty path takes the sack, which a new one always has.
    uncast(world, 0x1B, 0x0F, None)?;
    let form = uncast(world, 0x17, 0x0E, Some(b"Clutter\\Sack01.NIF"))?;
    world.resource_mut::<DefaultFormStatics>().loot_bag_b33aa8 = form;
    let form = uncast(world, 0x17, 0x11, None)?;
    world
        .resource_mut::<DefaultFormStatics>()
        .stolen_goods_b33aac = form;
    let form = default_clothing(world, 0x17, "Shirt", &[(2, true), (3, false)])?;
    world.resource_mut::<DefaultFormStatics>().jail_shirt_b33ab0 = form;
    let form = default_clothing(world, 0x15, "Pants", &[(2, false), (3, true)])?;
    world.resource_mut::<DefaultFormStatics>().jail_pants_b33ab4 = form;
    let form = default_clothing(world, 0x16, "Shoes", &[(2, false), (3, false), (5, true)])?;
    world.resource_mut::<DefaultFormStatics>().jail_shoes_b33ab8 = form;
    // 0044C2B5..0044C350 the FlameNode statics of 00B067C0 (0x54 bytes of IDs), 0044C4D0..0044C59C the FurnitureMarkers 100..119 (0x50 bytes): no model, no cast.
    for id in [
        0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x1E, 0x28, 0x29, 0x2A, 0x2B, 0x2C,
        0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x32,
    ] {
        uncast(world, 0x1C, id, None)?;
    }
    for id in 100..120 {
        uncast(world, 0x1C, id, None)?;
    }
    // 0044C5A2 0052FE90.
    create_default_topics(world)?;
    // 0044C5A7..0044CAD0 the six time globals.
    create_default_globals(world)?;
    // 0044CAD0..0044CB94 the player's NPC_.
    create_default_player_npc(world)?;
    // 0044CB94.. 0x15E, then 0x15F.
    create_default_weather(world)?;
    create_default_climate(world)?;
    // 0044CD42 Sky_GetSingleton, 0044CD49 00543200(sky, 0, 1).
    sky_set_climate_default(world)?;
    world.resource_mut::<DefaultFormStatics>().sky_climate_calls += 1;
    // 0044CD4E 0041BB80: the 18 sounds, the two spells, the three shaders.
    let mut sounds = Vec::with_capacity(MAGIC_SOUNDS.len());
    for (id, name) in MAGIC_SOUNDS {
        sounds.push(Some(default_sound(world, id, name, false)?));
    }
    let spells = create_default_spells(world)?;
    let shaders = create_default_effect_shaders(world)?;
    {
        let mut statics = world.resource_mut::<DefaultFormStatics>();
        statics.magic_sounds_b33560 = sounds;
        statics.default_spells_b335a8 = [Some(spells[0]), Some(spells[1])];
        statics.effect_shaders_b335b0 = shaders.into_iter().map(Some).collect();
    }
    // 0044CD53 00518000.
    let mut sounds = [None; 25];
    for (at, (id, name)) in FOOTSTEP_SOUNDS.into_iter().enumerate() {
        let (store_id, address) = FOOTSTEP_SOUND_ADDRESSES[at];
        debug_assert_eq!(store_id, id);
        sounds[((address - 0x00B3_6218) / 4) as usize] =
            Some(default_sound(world, id, name, true)?);
    }
    world
        .resource_mut::<DefaultFormStatics>()
        .footstep_sounds_b36218 = sounds;
    Ok(())
}
