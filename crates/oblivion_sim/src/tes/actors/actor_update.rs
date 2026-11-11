//! `00605770` Actor_Update for a Character on a LowProcess (vtables A6FC9C and A720A4, whose slots name each call here), with its time step 0.0 as `004DD850` passes it: `006497F0` the LowProcess update, `005FAAE0` the update by process time, and `00603CA0`. A new game's command 30 is the first to get here.
//!
//! Evidence: `records-run-u-status1-cover.txt` lines 1234 to 1322 in `evidence/gameplay-run001-2026-11-07/`.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::actor::{
    Actor, Character, Creature, MobileObject, TesObjectRefr, actor_set_z_rotation_if_process_allows,
};
use super::actor_process::{
    ActorProcessGlobals, BaseProcess, HIGH_PROCESS_VTABLE, LOW_PROCESS_VTABLE, LowProcess,
    MiddleHighProcess,
};
use super::player::PlayerCharacter;
use crate::tes::actor::{actor_get_actor_value, actor_is_dead};
use crate::tes::actor_process::{
    process_advance_procedure, process_evaluate_package, process_update_stamps,
};
use crate::tes::ai_conditions::ScriptArms;
use crate::tes::ai_package::{package_at_location, package_data, procedure_at};
use crate::tes::data_handler_world::doors_near;
use crate::tes::form::OriginalVtable;
use crate::tes::forms_references::form_flags;
use crate::tes::forms_references::save_load_active;
use crate::tes::game_settings::GameSettingCollection;
use crate::tes::global::{game_day, game_day_of_week};
use crate::tes::player::the_player;

// IDA: N/A the dynamic processes with an implemented Actor_Update path.
fn update_process(world: &World, actor: Entity) -> Result<Entity, GameError> {
    let process = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58
        .ok_or(GameError::unclassified(
            0x00605847,
            "00605847: Actor_Update with no process",
        ))?;
    if !matches!(
        world.get::<OriginalVtable>(process).map(|vtable| vtable.0),
        Some(LOW_PROCESS_VTABLE | HIGH_PROCESS_VTABLE)
    ) {
        return Err(GameError::unclassified(
            0x00605849,
            "00605849: Actor_Update on an intermediate process level",
        ));
    }
    Ok(process)
}

// IDA: N/A an integer GMST's value, the dword 00605874 compares.
fn setting_int(world: &World, name: &str) -> Result<i32, GameError> {
    world
        .get_resource::<GameSettingCollection>()
        .and_then(|settings| settings.int_named(name))
        .ok_or(GameError::unclassified(
            0x00605874,
            "00605874: a missing iMerchantRespawnDay setting",
        ))
}

/// IDA: 00605770 Actor_Update(time) for a Character on a LowProcess. Flag 0x800 returns. +0x78 clear with no active save takes the bypass arm, a stop. The stamp day (vt+0x34 00643690, +0x18 & 0x1FF) against 00402BA0's low byte sign-extended: equal clears +0xFD; otherwise with +0xFD clear, a day of the week (00402C40) of iMerchantRespawnDay1 or 2 respawns the 0x44 extra's container, a stop; then +0xFD = 1. vt+0x1A0 (GetActorValue(0x30)) set is a stop. Actor_IsDead(0) takes `dead_actor_update`, then the tail; a living actor with +0xC0 set, the manager's +0x98 clear or state 6 is a stop, as is +0xCC. vt+0x388 is 0 on a Character. The stamp's low byte unlike the day's drops the 0x59 extra (00422D10). vt+0x19C is 0 on a LowProcess, so with the manager's +0xA4 set or a combat package, and no type 0x18 package, a time of 0.0 takes vt+0x10 (006497F0). 00420050(day) walks the 0x21 extra. Then `update_tail`. Completion: 100% of those arms.
/// # Errors
/// The stops named.
pub fn actor_update(world: &mut World, actor: Entity, time: f32) -> Result<(), GameError> {
    if form_flags(world, actor) & 0x800 != 0 {
        return Ok(());
    }
    if the_player(world) == Some(actor)
        || (world.get::<Character>(actor).is_none() && world.get::<Creature>(actor).is_none())
    {
        return Err(GameError::unclassified(
            0x00605770,
            "00605770: Actor_Update requires a non-player Character or Creature",
        ));
    }
    let active = save_load_active(world)?;
    if world
        .get::<Actor>(actor)
        .expect("an actor")
        .processing_enabled_78
        == 0
        && !active
    {
        return Err(GameError::unclassified(
            0x006057A1,
            "006057A1: the bypassed actor's update",
        ));
    }
    let process = update_process(world, actor)?;
    let stamp = world
        .get::<LowProcess>(process)
        .expect("a LowProcess")
        .cur_packed_date_18
        & 0x1FF;
    let day = game_day(world);
    if stamp == i32::from(day as u8 as i8) {
        world
            .get_mut::<Actor>(actor)
            .expect("an actor")
            .merchant_respawn_day_processed_fd = 0;
    } else {
        if world
            .get::<Actor>(actor)
            .expect("an actor")
            .merchant_respawn_day_processed_fd
            == 0
        {
            let week = game_day_of_week(world);
            if (week == setting_int(world, "iMerchantRespawnDay1")?
                || week == setting_int(world, "iMerchantRespawnDay2")?)
                && world
                    .get::<TesObjectRefr>(actor)
                    .expect("a reference")
                    .extra_data_44
                    .get(0x44)
                    .is_some()
            {
                return Err(GameError::unclassified(
                    0x00605887,
                    "00605887: a merchant container's respawn",
                ));
            }
        }
        world
            .get_mut::<Actor>(actor)
            .expect("an actor")
            .merchant_respawn_day_processed_fd = 1;
    }
    if actor_get_actor_value(world, actor, 0x30)? != 0 {
        return Err(GameError::unclassified(
            0x006058CB,
            "006058CB: a paralyzed actor's horse",
        ));
    }
    let data = world.get::<Actor>(actor).expect("an actor");
    let bypassed = data.deferred_death_processing_c0 != 0 || data.dead_state_b0 == 6;
    if actor_is_dead(world, actor, false) {
        dead_actor_update(world, actor, process)?;
        return update_tail(world, actor, process, active, time);
    }
    if bypassed || world.resource::<ActorProcessGlobals>().manager.byte_98 == 0 {
        return Err(GameError::unclassified(
            0x00605A31,
            "00605A31: the bypassed living actor's arm",
        ));
    }
    if world
        .get::<Actor>(actor)
        .expect("an actor")
        .pending_movement_blocked_ref_cc
        != 0
    {
        return Err(GameError::unclassified(
            0x00605978,
            "00605978: a blocked movement's reference",
        ));
    }
    if stamp as u8 != day as u8 {
        world
            .get_mut::<TesObjectRefr>(actor)
            .expect("a reference")
            .into_inner()
            .extra_data_44
            .remove_by_type(0x59);
    }
    let package = world
        .get::<BaseProcess>(process)
        .expect("a process")
        .editor_package_08;
    let editor_schedules_enabled = world
        .resource::<ActorProcessGlobals>()
        .manager
        .editor_schedules_enabled_a4
        != 0;
    if !editor_schedules_enabled && package.is_some() {
        return Err(GameError::unclassified(
            0x006059C1,
            "006059C1: TESPackage_IsCombatType with +0xA4 clear",
        ));
    }
    let knocked = world
        .get::<MiddleHighProcess>(process)
        .is_some_and(|p| p.knocked_state_11c != 0);
    if !knocked
        && editor_schedules_enabled
        && !package.is_some_and(|package| package_data(world, package).type_20 == 0x18)
    {
        if time != 0.0 {
            return Err(GameError::unclassified(
                0x006059FD,
                "006059FD: vt+0x0C with a time step",
            ));
        }
        if world
            .get::<super::actor_process::HighProcess>(process)
            .is_some()
        {
            super::high_package::update_actor_package_and_procedure(world, process, actor)?;
        } else {
            low_process_update(world, process, actor)?;
        }
    }
    if world
        .get::<TesObjectRefr>(actor)
        .expect("a reference")
        .extra_data_44
        .get(0x21)
        .is_some()
    {
        return Err(GameError::unclassified(
            0x00605A19,
            "00605A19: 00420050's run-once packages",
        ));
    }
    update_tail(world, actor, process, active, time)
}

// IDA: 00605A23 Actor_Update's dead arm, Actor_IsDead(0) answering 1. Dead state 1 reaches 00B14E98's 005E1D30 and vt+0x20C, a stop; any other takes the process's vt+0x20 (00643640) and lowers +0x84 by 00B33E9C, the frame's seconds. Then 005E1D30, the base's +0x28 bit 1 (essential), set goes on; else 005E1D70's bit 3 (respawn) set reaches the level refresh's arms, a stop. Completion: 100% of the arms with neither bit.
fn dead_actor_update(world: &mut World, actor: Entity, process: Entity) -> Result<(), GameError> {
    if world.get::<Actor>(actor).expect("an actor").dead_state_b0 == 1 {
        return Err(GameError::unclassified(
            0x00605A60,
            "00605A60: a dead state of 1",
        ));
    }
    process_update_stamps(world, process);
    let frame = world
        .get_resource::<crate::tes::script_vm::ScriptVmGlobals>()
        .map_or(0.0, |globals| globals.frame_seconds_b33e9c);
    world
        .get_mut::<Actor>(actor)
        .expect("an actor")
        .unset_check_dead_body_timer_84 -= frame;
    let flags = base_actor_flags(world, actor);
    if flags & 2 == 0
        && flags & 8 != 0
        && !crate::tes::forms_references::parent_cell(world, actor)
            .is_some_and(|cell| crate::tes::forms_world::cell_attached(world, Some(cell), false))
        && world.get::<LowProcess>(process).unwrap().float_8c != 0.0
    {
        return Err(GameError::unclassified(
            0x00605AA4,
            "00605AA4: a respawning corpse's level refresh",
        ));
    }
    Ok(())
}

// IDA: 005E1D30's and 005E1D70's +0x28 of the base, when vt+0x190 says it has one (1 on a Character).
fn base_actor_flags(world: &World, actor: Entity) -> u32 {
    world
        .get::<TesObjectRefr>(actor)
        .and_then(|reference| reference.base_form_1c)
        .and_then(|base| world.get::<crate::tes::form_components::TesActorBaseData>(base))
        .map_or(0, |data| data.flags_04)
}

// IDA: 00605B58, the arms both of Actor_Update's arms end in. With no active save, vt+0x2E4's 005FAAE0 at 00673B00's time. +0xC0 is clear. vt+0x388 is 0, so 00603CA0. Last, a non-player's exterior parent cell that doesn't hold the position moves it, a stop.
fn update_tail(
    world: &mut World,
    actor: Entity,
    process: Entity,
    active: bool,
    time: f32,
) -> Result<(), GameError> {
    if !active {
        let seconds = world.resource::<ActorProcessGlobals>().time_seconds_b3bcf0;
        update_by_process_time(world, actor, process, seconds)?;
    }
    let data = world.get::<Actor>(actor).expect("an actor");
    if data.deferred_death_processing_c0 != 0
        && world
            .get::<super::actor_process::HighProcess>(process)
            .is_some()
        && world
            .get::<crate::tes::reference_3d::Reference3D>(actor)
            .is_some()
    {
        return Err(GameError::unclassified(
            0x00605BBF,
            "deferred death processing on a loaded actor",
        ));
    }
    if world.get::<Creature>(actor).is_some() && data.horse_or_rider_d4 != 0 {
        return Err(GameError::unclassified(
            0x00605D20,
            "Actor_Update: query the creature's rider",
        ));
    }
    super::actor_movement_update::npc_update_movement_and_animation(world, actor, process, time)?;
    let reference = world.get::<TesObjectRefr>(actor).expect("a reference");
    if reference.base_form_1c.is_some()
        && let Some(cell) = reference.parent_cell_40
        && !crate::tes::forms_world::cell_is_interior(world, cell)
    {
        return Err(GameError::unclassified(
            0x00605D7D,
            "00605D7D: an exterior cell's position test",
        ));
    }
    Ok(())
}

// IDA: 006497F0 LowProcess vt+0x10(actor). vt+0x18(actor, 0) (00649340) answering a change is a stop. A package whose procedure type isn't -1, outside the jail test (the player in jail or pursued, with the package's flag 0x10000), takes 00644CE0, whose target reference is a stop, and the procedure at +0x04 (the 00B152B0 rows). Procedure 0 at the location: with +0x84 clear flag 0x20 locks doors and +0x84 = 1; flag 0x100 unlocks; with no +0x30 target and a location of type 3 vt+0x1E8 turns the actor to vt+0xF0's z (005E10D0, +0xF4); then vt+0x188(actor, 1). Any other procedure, or walking to the location, is a stop. On level 3 a procedure of 0x2C ends the package, a stop. 006825C0 and 006826D0 look the actor up in the path builder's three maps and take it out, where the host has none (bUseBackgroundPathing, a recorded substitution). Then vt+0x20 (00643640). Completion: 100% of those arms.
fn low_process_update(world: &mut World, process: Entity, actor: Entity) -> Result<(), GameError> {
    if process_evaluate_package(world, process, actor, false, &mut ScriptArms)? {
        return Err(GameError::unclassified(
            0x00649859,
            "00649859: a package change in the LowProcess update",
        ));
    }
    let base = *world.get::<BaseProcess>(process).expect("a process");
    if let Some(package) = base.editor_package_08
        && package_data(world, package).field_18 != 0xFFFF_FFFF
    {
        let data = package_data(world, package);
        let player = world
            .get::<PlayerCharacter>(the_player(world).ok_or(GameError::unclassified(
                0x006498BD,
                "006498BD: a null g_thePlayer",
            ))?)
            .expect("the player");
        let jailed =
            player.prison_days_remaining_608 > 0 || player.jail_guard_pursuit_active_610 != 0;
        if !(jailed && data.flags_1c & 0x1_0000 != 0) {
            if data.target_28.is_some() {
                return Err(GameError::unclassified(
                    0x00644CF3,
                    "00644CF3: a package target's reference",
                ));
            }
            let flags = data.flags_1c;
            let location_type = data.location_24.map(|location| location.type_00);
            match procedure_at(world, package, base.edit_pack_procedure_type_04)? {
                0 => {
                    if !package_at_location(world, package, actor, false, -1.0)? {
                        return Err(GameError::unclassified(
                            0x0064991F,
                            "0064991F: walking to the package location",
                        ));
                    }
                    if world
                        .get::<LowProcess>(process)
                        .expect("a LowProcess")
                        .byte_84
                        == 0
                    {
                        if flags & 0x20 != 0 {
                            doors_near(world, actor, true)?;
                        }
                        world
                            .get_mut::<LowProcess>(process)
                            .expect("a LowProcess")
                            .byte_84 = 1;
                    }
                    if flags & 0x100 != 0 {
                        doors_near(world, actor, false)?;
                    }
                    let target = world
                        .get::<LowProcess>(process)
                        .expect("a LowProcess")
                        .field_30;
                    if target.is_some() {
                        return Err(GameError::unclassified(
                            0x00649C6A,
                            "00649C6A: a +0x30 target's heading",
                        ));
                    }
                    if location_type == Some(3) {
                        let rotation = world
                            .get::<Actor>(actor)
                            .expect("an actor")
                            .starting_z_rotation_f4;
                        actor_set_z_rotation_if_process_allows(world, actor, rotation)?;
                    }
                    process_advance_procedure(world, process, actor, 1)?;
                }
                _ => {
                    return Err(GameError::unclassified(
                        0x00649904,
                        "00649904: a LowProcess procedure other than 0",
                    ));
                }
            }
            let base = *world.get::<BaseProcess>(process).expect("a process");
            if let Some(package) = base.editor_package_08
                && procedure_at(world, package, base.edit_pack_procedure_type_04)? == 0x2C
            {
                return Err(GameError::unclassified(
                    0x0064A0EC,
                    "0064A0EC: the package's end on level 3",
                ));
            }
        }
    }
    process_update_stamps(world, process);
    Ok(())
}

// IDA: 005FAAE0 Actor vt+0x370(seconds) on a Character with a LowProcess: the step is seconds less +0xBC, or 0 after a later or negative +0xBC (seconds itself inside (0, 0.3)). vt+0x2E4 (005F43E0) does nothing with no magic node and no 3D: MagicCaster vt+0x0C asks the process's vt+0x2A8, 0; vt+0x2B8 is false; 00699080 asks vt+0x30, 0; 006A2280 returns at the null node. A living actor's 005E6190 is a type 12 package, a stop; vt+0x14C is 0 on a LowProcess. 005F2720 and the inline magicka arm restore a positive amount below the respective maximum. 005E7A60 walks the +0x9C list, a stop when it holds anything. The player's +0x594 sleep, and vt+0x2D8 does nothing. Then +0xBC = seconds. Completion: the LowProcess route outside the stated stops.
fn update_by_process_time(
    world: &mut World,
    actor: Entity,
    process: Entity,
    seconds: f32,
) -> Result<(), GameError> {
    let last = world
        .get::<Actor>(actor)
        .expect("an actor")
        .last_process_update_time_bc;
    let mut step = seconds - last;
    if last > seconds || last < 0.0 {
        world
            .get_mut::<Actor>(actor)
            .expect("an actor")
            .last_process_update_time_bc = seconds;
        step = 0.0;
        if seconds > 0.0 && seconds < 0.3 {
            world
                .get_mut::<Actor>(actor)
                .expect("an actor")
                .last_process_update_time_bc = 0.0;
            step = seconds;
        }
    }
    // 005F43E0 Actor_UpdateMagicComponents. The High and MiddleHigh virtuals can
    // supply a current item, a pending casting update, and a nonempty effect list.
    let middle = world.get::<MiddleHighProcess>(process);
    if middle.is_some_and(|p| p.current_magic_item_144 != 0) {
        return Err(GameError::unclassified(
            0x005F4400,
            "validate the current cast item's animation",
        ));
    }
    if middle.is_some_and(|p| p.cast_animation_update_flag_14c != 0) {
        return Err(GameError::unclassified(
            0x005F4446,
            "update the actor's casting animation",
        ));
    }
    let data = world.get::<Actor>(actor).expect("an actor");
    if data.magic_caster_5c.magic_node_04 != 0 {
        return Err(GameError::unclassified(
            0x005F445B,
            "005F445B: the magic node's update",
        ));
    }
    super::magic_cast::update_target_effects(world, actor, step)?;
    if !actor_is_dead(world, actor, false) {
        if world
            .get::<BaseProcess>(process)
            .expect("a process")
            .editor_package_08
            .is_some_and(|package| package_data(world, package).type_20 == 12)
        {
            return Err(GameError::unclassified(
                0x005FAB7C,
                "005FAB7C: a combat controller's update",
            ));
        }
        super::actor_regeneration::regenerate_fatigue(world, actor, step)?;
        // 005FAD33: the LowProcess casting virtual returns zero.
        super::actor_regeneration::regenerate_magicka(world, actor, step)?;
        if world
            .get::<Actor>(actor)
            .expect("an actor")
            .greater_power_list_9c
            != [0; 2]
        {
            return Err(GameError::unclassified(
                0x005FAE49,
                "005FAE49: 005E7A60's timers",
            ));
        }
        let player = the_player(world).ok_or(GameError::unclassified(
            0x005FAE4E,
            "005FAE4E: a null g_thePlayer",
        ))?;
        if world
            .get::<PlayerCharacter>(player)
            .expect("the player")
            .pc_sleep_hours_set_594
            != 0
        {
            super::actor_regeneration::restore_health(world, actor)?;
        }
    }
    world
        .get_mut::<Actor>(actor)
        .expect("an actor")
        .last_process_update_time_bc = seconds;
    Ok(())
}
