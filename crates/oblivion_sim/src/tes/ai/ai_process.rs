//! Phase 14c: 00675310 ActorProcessManager_ProcessTrackedActors and the AI package pass it runs over the level-3 list: 004F9EC0, the death and removal arms, and 005ED860: the stamps 00643640, the scheduled packages 0064A380, the evaluation 00649340, 00648E40 and the cell moves; with the actor values (00643340, 005F1970, 005F1A60) and weights they read.
//!
//! P14-RUN-001..004: 1,998 actors. The original's `rand` and the interface's menu mode come through `AiArms`: `AiRealArms` for a driver, `AiRecordedArms` for a test replaying a run's draws and logging what asked. This is this slice's `bind_real_arms` and recording arms: the C++'s Phase 15 function of that name binds 15c and 15d's arms, which come with that port.
//!
//! The pass reads no world setup output (TES +0x74, the LOD maps, the idle roots, the archives). With TLS +0x185 set, as WinMain 0040EB8B leaves it, `set_change_flags` records changes.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::ai_conditions::ConditionCaches;
use super::ai_package::{package_data, target_object, target_object_type, target_reference};
use super::condition_tables::CONDITION_FUNCTIONS;
use crate::tes::actor::process_of;
use crate::tes::actor::{Actor, Creature, TesObjectRefr, magicka_multiplier};
use crate::tes::actor_package::set_package_target;
use crate::tes::actor_process::{
    ActorProcessGlobals, BaseProcess, LowProcess, RandomSource, process_actor_value_float,
};
use crate::tes::actor_values::base_actor_value;
use crate::tes::base_extra_list::{EXTRA_REFERENCE_POINTER, follower_add};
use crate::tes::forms_init::form_type;
use crate::tes::forms_references::form_flags;
use crate::tes::player::the_player;
use oblivion_core::crt::random_next;

/// IDA: N/A what the pass asks outside the game state. A driver and a test bind these differently.
pub trait AiArms {
    /// IDA: 0047DF80 Random_Next(0) (004F4EF0 GetRandomPercent).
    /// # Errors
    /// A recording arm out of draws.
    fn random_next(&mut self, world: &mut World) -> Result<i32, GameError>;
    /// IDA: 00578F60 InterfaceManager_IsMenuMode.
    /// # Errors
    /// An arm with no answer.
    fn menu_mode(&mut self, world: &mut World) -> Result<bool, GameError>;
    /// IDA: 0057C140 InterfaceManager_IsMenuInputActive(menu), which MenuMode with an ID (004F4570) asks; only an arm with the
    /// interface can answer.
    /// # Errors
    /// An arm with no interface.
    fn menu_input_active(&mut self, _world: &mut World, _menu: u32) -> Result<bool, GameError> {
        Err(GameError::unclassified(
            0x004F458A,
            "004F458A: MenuMode with a menu ID",
        ))
    }
}

/// IDA: N/A the real arms: 0047DF80 over a CRT and the interface's answer, which Phase 21's interface fills.
pub struct AiRealArms<'a> {
    pub random: &'a mut dyn RandomSource,
    pub menu_mode: &'a mut dyn FnMut(&World) -> bool,
}

impl AiArms for AiRealArms<'_> {
    /// IDA: N/A `AiRealArms`'s side of `AiArms`.
    fn random_next(&mut self, world: &mut World) -> Result<i32, GameError> {
        let mut globals = world.resource_mut::<ActorProcessGlobals>();
        Ok(random_next(0, &mut globals.random, &mut *self.random))
    }
    /// IDA: N/A `AiRealArms`'s side of `AiArms`.
    fn menu_mode(&mut self, world: &mut World) -> Result<bool, GameError> {
        Ok((self.menu_mode)(world))
    }
}

/// IDA: N/A the recording arms: a run's draws in order, and how many went. P14-RUN-004's 157 `GetRandomPercent` values go through here, and `next` tells a test if the host drew a different number.
#[derive(Debug, Default)]
pub struct AiRecordedArms {
    pub draws: Vec<i32>,
    pub next: usize,
    pub menu_mode: bool,
    pub menu_queries: u32,
}

impl AiArms for AiRecordedArms {
    /// IDA: N/A no address of its own; its comments cite 004F4EF0.
    fn random_next(&mut self, _world: &mut World) -> Result<i32, GameError> {
        let value = *self.draws.get(self.next).ok_or(GameError::unclassified(
            0x004F4EF0,
            "004F4EF0: more draws than the recorded run",
        ))?;
        self.next += 1;
        Ok(value)
    }
    /// IDA: N/A `AiRecordedArms`'s side of `AiArms`.
    fn menu_mode(&mut self, _world: &mut World) -> Result<bool, GameError> {
        self.menu_queries += 1;
        Ok(self.menu_mode)
    }
}

/// IDA: the handler caches, and the host's counts per arm (N/A), which a test and the log read. The warnings are 004A7A60's, which a driver hands on.
#[derive(Resource, Debug)]
pub struct AiProcessGlobals {
    pub caches: ConditionCaches,
    /// IDA: N/A the 004F9EC0 -> 00517950 runs.
    pub script_event_runs_not_executed: u32,
    pub actors_visited: u32,
    pub actors_removed: u32,
    pub packages_evaluated: u32,
    pub cell_moves: u32,
    pub random_calls: u32,
    /// IDA: N/A the form ID behind each draw, in draw order.
    pub random_subjects: Vec<u32>,
    pub doors_unlocked: u32,
    pub doors_locked: u32,
    pub condition_calls: Vec<u32>,
    pub warnings: Vec<String>,
}

impl Default for AiProcessGlobals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            caches: ConditionCaches::default(),
            script_event_runs_not_executed: 0,
            actors_visited: 0,
            actors_removed: 0,
            packages_evaluated: 0,
            cell_moves: 0,
            random_calls: 0,
            random_subjects: Vec::new(),
            doors_unlocked: 0,
            doors_locked: 0,
            condition_calls: vec![0; CONDITION_FUNCTIONS.len()],
            warnings: Vec::new(),
        }
    }
}

// ---- the actor values ----

// IDA: 006253C0, 00625410 the Creature folds: 12..=18 and 28 are 12, 19..=25 are 19, 26..=32 are 26.
pub(crate) fn fold_creature_code(world: &World, actor: Entity, code: u32) -> u32 {
    if world.get::<Creature>(actor).is_none() {
        return code;
    }
    match code {
        12..=18 | 28 => 12,
        19..=25 => 19,
        26..=32 => 26,
        _ => code,
    }
}

// IDA: the no-process arms, 5F1A42 and 005F1B2A: 005F1910's floor of 005EAD00: the base value for a non-player's codes outside 37..=39.
pub(crate) fn no_process_value(world: &World, actor: Entity, code: u32) -> Result<i32, GameError> {
    if (37..=39).contains(&code) {
        return Err(GameError::unclassified(
            0x005EAD00,
            "005EAD00: codes 37..=39 of an actor with no process",
        ));
    }
    let base = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .base_form_1c
        .ok_or(GameError::unclassified(
            0x005EAD00,
            "005EAD00: no base faults",
        ))?;
    base_actor_value(world, base, code)
}

/// IDA: 005F1A60 Actor vt+0x288 GetActorValueFloat (Creature 00625410 folds first). AV 9 multiplies the 006433E0 sum by 005F19A7's. The player is 0065E110's (`player::player_get_actor_value_float`). Completion: 100%.
pub fn actor_get_actor_value_float(
    world: &mut World,
    actor: Entity,
    actor_value: u32,
) -> Result<f32, GameError> {
    // PlayerCharacter's vt+0x288 is 0065E110.
    if Some(actor) == the_player(world) {
        return crate::tes::player::player_get_actor_value_float(world, actor, actor_value);
    }
    let code = fold_creature_code(world, actor, actor_value);
    let Some(process) = process_of(world, actor) else {
        return Ok(no_process_value(world, actor, code)? as f32);
    };
    if code == 9 {
        let multiplier = magicka_multiplier(world, actor)?;
        return Ok(process_actor_value_float(world, actor, process, 9)? * multiplier);
    }
    process_actor_value_float(world, actor, process, code)
}

// ---- the LowProcess package slots ----

// IDA: N/A a helper, with no address of its own.
pub(crate) fn base_of(world: &World, process: Entity) -> BaseProcess {
    *world.get::<BaseProcess>(process).expect("a process")
}

// IDA: N/A a helper, with no address of its own.
pub(crate) fn low_mut(world: &mut World, process: Entity) -> Mut<'_, LowProcess> {
    world.get_mut::<LowProcess>(process).expect("a LowProcess")
}

// IDA: 00673980 the sequence length by the table at 006739AC, with the 0x2C end; -1 past 0x27.
pub(crate) const fn procedure_count(kind: i32) -> i32 {
    match kind {
        0 | 7 | 12 | 14 | 15 | 16 | 19 | 21 | 24 | 25 | 29 | 31 | 34..=39 => 2,
        1 | 2 | 3 | 6 | 9 | 13 | 17 | 18 | 20 | 23 => 3,
        4 | 5 | 10 | 22 | 26 | 28 | 30 | 32 => 4,
        8 | 11 | 27 | 33 => 5,
        _ => -1,
    }
}

// IDA: 00568BB0 TESPackage_ApplyTarget(actor): type 0 the reference, a deleted one through its 0x22 extra; a Follow or Escort of an actor lists the actor as its follower, an Accompany the reverse; 005E03C0 records it, null too.
// TODO: [ ] 00568C45, 00568BE1 the type 1 and 2 searches through 00446B90 and 00567730; 00568D88 following the player.
pub(crate) fn package_apply_target(
    world: &mut World,
    package: Entity,
    actor: Entity,
) -> Result<(), GameError> {
    let data = package_data(world, package);
    let kind = data.type_20 as u8;
    let flags = data.flags_1c;
    let Some(target) = data.target_28 else {
        return Ok(());
    };
    let mut reference = None;
    match target.type_00 {
        0 => {
            reference = target_reference(world, &target)?;
            if let Some(form) = reference
                && form_flags(world, form) & 0x20 != 0
            {
                let list = &world
                    .get::<TesObjectRefr>(form)
                    .expect("a reference")
                    .extra_data_44;
                reference = list
                    .get(EXTRA_REFERENCE_POINTER)
                    .and_then(|entry| entry.resolved_0c.flatten())
                    .filter(|&form| world.get::<TesObjectRefr>(form).is_some());
            }
        }
        1 => {
            return Err(GameError::unclassified(
                0x00568C45,
                "00568C45: the type 1 target search",
            ));
        }
        2 => {
            return Err(GameError::unclassified(
                0x00568BE1,
                "00568BE1: the type 2 target search",
            ));
        }
        _ => {}
    }
    if let Some(form) = reference
        && world.get::<Actor>(form).is_some()
        && flags & 0x800 == 0
    {
        if kind == 1 || kind == 7 {
            if Some(form) == the_player(world) {
                return Err(GameError::unclassified(
                    0x00568D88,
                    "00568D88: following the player",
                ));
            }
            follower_add(world, form, actor);
        } else if kind == 2 {
            follower_add(world, actor, form);
        }
    }
    set_package_target(world, actor, reference);
    Ok(())
}

// IDA: 00567CA0 whether the target is a follow-able actor.
pub(crate) fn package_target_is_actor(world: &World, package: Entity) -> Result<bool, GameError> {
    let Some(target) = package_data(world, package).target_28 else {
        return Ok(false);
    };
    Ok(match target.type_00 {
        0 => {
            target_reference(world, &target)?.is_some_and(|form| world.get::<Actor>(form).is_some())
        }
        1 => target_object(&target)?.is_some_and(|form| matches!(form_type(world, form), 35 | 36)),
        2 => target_object_type(&target).wrapping_sub(15) <= 1,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_procedure_counts_follow_the_table() {
        assert_eq!(procedure_count(0), 2);
        assert_eq!(procedure_count(1), 3);
        assert_eq!(procedure_count(8), 5);
        assert_eq!(procedure_count(40), -1);
        assert_eq!(procedure_count(-1), -1);
    }
}
