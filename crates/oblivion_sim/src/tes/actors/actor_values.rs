//! The actor values a base form answers: 00519D90 GetActorValueFromBaseForm and base vt+0x128 (TESNPC 005232D0, TESCreature 0051D540), with the creature's level scaling.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use crate::tes::actor_process::actor_values_get;
use crate::tes::form_components::{
    TesActorBaseData, TesAiForm, TesAttributes, TesContainer, TesHealthForm, container_weight,
};
use crate::tes::forms_actors::{ActorBaseValues, TesCreature, TesNpc};
use crate::tes::forms_init::effective_level;
use crate::tes::game_settings::{GameSettingCollection, float_at_or_zero, int_at_or_zero};

// IDA: 0051CA10, 0051CAA0, 0051CAD0 on a creature: with TESActorBaseData flag 0x80 the 16-bit value times the level, forced to at least 1, in 16 bits; otherwise the value. The C++ reads the raw fields, which is wrong for a PC-level creature.
fn scaled_creature_value(world: &World, base: Entity, value: u16) -> Result<u16, GameError> {
    let data = world.get::<TesActorBaseData>(base).expect("an actor base");
    if data.flags_04 & 0x80 == 0 {
        return Ok(value);
    }
    let level = effective_level(world, data)?.max(1) as u16;
    Ok(value.wrapping_mul(level))
}

// IDA: 0051CC00, 0051CB80, 0051CB00 a creature's skill groups: with flag 0x80, `(u8)chop(f32(f32(setting * level) + skill))`, else the byte. The settings' cinit default is 2.0.
fn creature_skill(
    world: &World,
    base: Entity,
    index: usize,
    address: u32,
) -> Result<i32, GameError> {
    let data = world.get::<TesActorBaseData>(base).expect("an actor base");
    let byte = world.get::<TesCreature>(base).expect("a creature").data_104[index];
    if data.flags_04 & 0x80 == 0 {
        return Ok(i32::from(byte));
    }
    let level = effective_level(world, data)?.max(1);
    let setting = world
        .get_resource::<GameSettingCollection>()
        .and_then(|settings| settings.raw_at(address))
        .map_or(2.0, f32::from_bits);
    let value = setting * f32::from(level) + f32::from(byte);
    Ok(i32::from(x87::ftol(value) as u8))
}

// IDA: 00519D90 GetActorValueFromBaseForm: 0..=7 the +0x88 attributes; 8 00519C10; 9, 10 TESActorBaseData's; 11 ftol(004698B0); 12..=32 and 37..=39 zero; 33..=36 the +0x68 bytes; above, ftol(0065CB80 on +0xD0).
pub(in crate::tes) fn base_form_actor_value(
    world: &World,
    base: Entity,
    code: u32,
) -> Result<i32, GameError> {
    let creature = world.get::<TesCreature>(base).is_some();
    let data = world.get::<TesActorBaseData>(base);
    Ok(match code {
        0..=7 => i32::from(
            world
                .get::<TesAttributes>(base)
                .map_or(0, |attributes| attributes.values_04[code as usize]),
        ),
        8 => {
            let health = world
                .get::<TesHealthForm>(base)
                .map_or(0, |health| health.health_04);
            if creature {
                i32::from(scaled_creature_value(world, base, health as u16)?)
            } else {
                health as i32
            }
        }
        9 | 10 => {
            let data = data.ok_or(GameError::unclassified(
                0x00519E5C,
                "00519E5C: no TESActorBaseData faults",
            ))?;
            let value = if code == 9 {
                data.magicka_08
            } else {
                data.fatigue_0a
            };
            if creature {
                i32::from(scaled_creature_value(world, base, value)?)
            } else {
                i32::from(value)
            }
        }
        11 => x87::ftol(
            world
                .get::<TesContainer>(base)
                .map_or(Ok(0.0), |container| container_weight(world, container))?,
        ),
        12..=32 | 37..=39 => 0,
        33..=36 => i32::from(
            world
                .get::<TesAiForm>(base)
                .map_or(0, |ai| ai.ai_04[(code - 33) as usize]),
        ),
        _ => x87::ftol(
            world
                .get::<ActorBaseValues>(base)
                .map_or(0.0, |values| actor_values_get(&values.0, code as u8)),
        ),
    })
}

/// IDA: base vt+0x128: TESNPC 005232D0 (skills 12..=32 from +0xEC, 37..=39 zero, else 00519D90), TESCreature 0051D540 (the three skill groups, else 00519D90). Completion: 100%.
pub fn base_actor_value(world: &World, base: Entity, code: u32) -> Result<i32, GameError> {
    if let Some(npc) = world.get::<TesNpc>(base) {
        if (12..=32).contains(&code) {
            return Ok(i32::from(npc.skills_ec[(code - 12) as usize]));
        }
        return base_form_actor_value(world, base, code);
    }
    if world.get::<TesCreature>(base).is_some() {
        return match code {
            // 0051D544..0051D572: plain ranges. Only the actor folds at 006253C0, 00625410 send 28 to 12.
            12..=18 => creature_skill(world, base, 1, 0x00B3_62A4),
            19..=25 => creature_skill(world, base, 2, 0x00B3_62AC),
            26..=32 => creature_skill(world, base, 3, 0x00B3_62B4),
            _ => base_form_actor_value(world, base, code),
        };
    }
    Err(GameError::unclassified(
        0x006433B7,
        "006433B7: base vt+0x128 on a non-actor base",
    ))
}

// IDA: 00547B90 CalculateLuckAdjustedSkill(skill, luck): `fild` luck times fActorLuckSkillMult (00B37390), stored a float
// (00547B9A); `fiadd` iActorLuckSkillBase (00B37388), stored a float (00547BA8); `fild` skill plus that. 100.0 at or above
// 100.0, 0.0 at or below 0.0, else the sum, stored a float. A whole number below 2^24 converts exactly, so each step here
// is one float rounding, as the stores are.
pub(crate) fn luck_adjusted_skill(world: &World, skill: i32, luck: i32) -> f32 {
    let scaled = luck as f32 * float_at_or_zero(world, 0x00B3_7390);
    let luck = scaled + int_at_or_zero(world, 0x00B3_7388) as f32;
    let value = skill as f32 + luck;
    // 00547BB4..00547BD0: below 100.0, then at or below 0.0; a NaN fails both and gives 100.
    if value <= 0.0 {
        0.0
    } else if value >= 100.0 || value.is_nan() {
        100.0
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tes::actor::TesObjectRefr;

    // 0051CA10: a PC-level creature's health is the 16-bit value times the level, at least 1.
    #[test]
    fn a_pc_level_creature_scales_its_health() {
        let mut world = World::new();
        let player = world.spawn(TesObjectRefr::default()).id();
        world.init_resource::<crate::tes::player::PlayerGlobals>();
        world
            .resource_mut::<crate::tes::player::PlayerGlobals>()
            .the_player_b333c4 = Some(player);
        let base = world
            .spawn((
                TesCreature::default(),
                TesHealthForm {
                    original_vtable: 0,
                    health_04: 7,
                },
                TesActorBaseData {
                    flags_04: 0x80,
                    level_0e: 3,
                    ..TesActorBaseData::default()
                },
            ))
            .id();
        assert_eq!(base_actor_value(&world, base, 8).unwrap(), 21);
        world.get_mut::<TesActorBaseData>(base).unwrap().flags_04 = 0;
        assert_eq!(base_actor_value(&world, base, 8).unwrap(), 7);
        // Two bytes only.
        world.get_mut::<TesHealthForm>(base).unwrap().health_04 = 0x1_0007;
        assert_eq!(base_actor_value(&world, base, 8).unwrap(), 7);
    }
}
