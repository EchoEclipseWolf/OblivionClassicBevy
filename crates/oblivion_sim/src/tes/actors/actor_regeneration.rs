//! Actor-value recovery during process updates. Actor values 8, 9 and 10 are health, magicka and fatigue.

use bevy_ecs::prelude::*;
use oblivion_core::{error::GameError, x87};

use super::actor::MobileObject;
use super::actor_process::{
    LowProcess, MiddleLowProcess, actor_values_get, add_actor_value_modifier,
};
use super::player::{PlayerCharacter, floor_calculated_actor_value};
use crate::tes::actor::actor_get_actor_value;
use crate::tes::ai_process::actor_get_actor_value_float;
use crate::tes::game_settings::GameSettingCollection;
use crate::tes::player::the_player;

// IDA: 005F2741..005F2754. LowProcess returns zero; MiddleLowProcess_GetMaxAVModifier reads +0x94.
fn maximum_modifier(world: &World, actor: Entity, code: u8) -> f32 {
    if Some(actor) == the_player(world) {
        return world
            .get::<PlayerCharacter>(actor)
            .expect("the player")
            .max_av_modifiers_204[usize::from(code)];
    }
    world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
        .and_then(|p| world.get::<MiddleLowProcess>(p))
        .map_or(0.0, |p| actor_values_get(&p.max_modifiers_94, code))
}

// IDA: 005E2BE0 Actor_DamageAV_F, positive recovery of a nonplayer's health, magicka or fatigue.
// Completion: all three process implementations update +0x70 for these codes and mark the actor changed.
fn recover_damage(
    world: &mut World,
    actor: Entity,
    code: u8,
    amount: f32,
) -> Result<(), GameError> {
    if amount > 0.0 {
        if Some(actor) == the_player(world) {
            return Err(GameError::unclassified(
                0x005F280E,
                "PlayerCharacter's damage AV recovery virtual",
            ));
        }
        if let Some(p) = world
            .get::<MobileObject>(actor)
            .expect("an actor")
            .process_58
        {
            add_actor_value_modifier(
                &mut world
                    .get_mut::<LowProcess>(p)
                    .expect("a process")
                    .damage_modifiers_70,
                code,
                amount,
                false,
            )?;
        }
        crate::tes::forms_items::set_change_flags(world, actor, 0x400000)?;
    }
    Ok(())
}

// IDA: N/A load the static GMST value at this call site.
fn setting(world: &World, address: u32) -> Result<f32, GameError> {
    world
        .get_resource::<GameSettingCollection>()
        .and_then(|settings| settings.float_at(address))
        .ok_or_else(|| GameError::unclassified(address, "missing actor regeneration setting"))
}

// IDA: 005F2720 Actor_RegenerateFatigue, with 00547F20 ActorValue_GetFatigueReturnRate.
// Completion: the process modifier and floored comparison, Endurance and GMST reads, and positive recovery.
pub(super) fn regenerate_fatigue(
    world: &mut World,
    actor: Entity,
    step: f32,
) -> Result<(), GameError> {
    let modifier = maximum_modifier(world, actor, 10);
    let current = x87::floor_by_ftol(actor_get_actor_value_float(world, actor, 10)?);
    let maximum = floor_calculated_actor_value(world, actor, 10)? as f32 + modifier;
    if current < maximum {
        let endurance = actor_get_actor_value(world, actor, 5)?;
        let term = (f64::from(endurance) * f64::from(setting(world, 0x00B37548)?)) as f32;
        let rate = term + setting(world, 0x00B37540)?;
        recover_damage(world, actor, 10, rate * step)?;
    }
    Ok(())
}

// IDA: 005F25F0 Actor_RegenerateMagicka and its inline copy at 005FAD3D after the casting test.
// Completion: 00548D60 ActorValue_GetMagickaReturnRate and the positive recovery store.
pub(super) fn regenerate_magicka(
    world: &mut World,
    actor: Entity,
    step: f32,
) -> Result<(), GameError> {
    let modifier = maximum_modifier(world, actor, 9);
    let maximum = floor_calculated_actor_value(world, actor, 9)? as f32 + modifier;
    let current = x87::floor_by_ftol(actor_get_actor_value_float(world, actor, 9)?);
    if maximum > current {
        let stunted = actor_get_actor_value(world, actor, 57)?;
        let willpower = actor_get_actor_value(world, actor, 2)?;
        let rate = if stunted > 0 {
            0.0
        } else {
            let term = (f64::from(willpower) * f64::from(setting(world, 0x00B37F38)?)) as f32;
            (term + setting(world, 0x00B37F30)?) * (maximum / 100.0)
        };
        recover_damage(world, actor, 9, rate * step)?;
    }
    Ok(())
}

// IDA: 005F2530 Actor_RestoreHealthToMaximum. The time argument passed by the caller goes unused.
// Completion: current health is read twice when the first read is below maximum.
pub(super) fn restore_health(world: &mut World, actor: Entity) -> Result<(), GameError> {
    let modifier = maximum_modifier(world, actor, 8);
    let maximum = floor_calculated_actor_value(world, actor, 8)? as f32 + modifier;
    if maximum > actor_get_actor_value_float(world, actor, 8)? {
        let amount = maximum - actor_get_actor_value_float(world, actor, 8)?;
        recover_damage(world, actor, 8, amount)?;
    }
    Ok(())
}
