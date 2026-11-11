//! Actor: the package methods (what the actor's current package asks of it).

use crate::tes::actor::{Actor, MobileObject, TesObjectRefr, actor_is_dead, process_of};
use crate::tes::actor_process::{
    LowProcess, process_evaluate_package, process_run_scheduled, process_update_stamps,
};
use crate::tes::ai_package::{
    package_at_location, package_data, package_is_combat_like, package_location_cell,
    package_location_position, package_location_world,
};
use crate::tes::ai_process::{AiArms, AiProcessGlobals, base_of, low_mut};
use crate::tes::base_extra_list::EXTRA_HORSE;
use crate::tes::character_controller::actor_set_position;
use crate::tes::data_handler_world::doors_near;
use crate::tes::form::FormIdentity;
use crate::tes::forms_references::{
    form_flags, move_to_cell, parent_cell, reference_is_persistent, reference_position,
    set_rotation_x,
};
use crate::tes::forms_world::{cell_find_marker, cell_first_reference};
use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

// IDA: 0065AD00 == 3 at 004DD7D9. 0065AD00 itself is in `actor_process`; with no new cell it comes down to 3 unless sleeping, active or attached, so the host asks through the same reads.
pub(crate) fn desired_level_is_3(world: &World, actor: Entity) -> Result<bool, GameError> {
    Ok(crate::tes::actor_process::actor_desired_level(world, actor)? == 3)
}

/// IDA: 005E6110(require_rider_or_not_sleeping): with a process, when vt+0x380 (Character_GetMountedHorse) is set, or the argument is 0, or the process's vt+0x36C (sleep state; a LowProcess's 0) is 0, the process's +0x08 package of type 12 or 13 answers true. Completion: the process sleep state and editor package for a Character or Creature.
pub(crate) fn actor_in_combat_package(
    world: &World,
    actor: Entity,
    require: bool,
) -> Result<bool, GameError> {
    let Some(process) = world.get::<MobileObject>(actor).and_then(|m| m.process_58) else {
        return Ok(false);
    };
    let mounted = world.get::<crate::tes::actor::Character>(actor).is_some()
        && world
            .get::<crate::tes::actor::Actor>(actor)
            .is_some_and(|a| a.horse_or_rider_d4 != 0);
    if require
        && !mounted
        && world
            .get::<crate::tes::actor_process::MiddleHighProcess>(process)
            .is_some_and(|middle| middle.sleep_state_11d != 0)
    {
        return Ok(false);
    }
    let package = world
        .get::<crate::tes::actor_process::BaseProcess>(process)
        .and_then(|b| b.editor_package_08);
    Ok(package.is_some_and(|package| matches!(package_data(world, package).type_20, 12 | 13)))
}

/// IDA: 005E6CD0(keep): a current package (vt+0x184; a LowProcess's +0x08) of type 16 answers true for anyone but the player, whose +0x08 is released; otherwise with `keep` 0 the +0x08 package's type 16, 10, or 13 with procedure 1 answers true. Completion: 100% of a LowProcess or none; the player's arm stops.
pub(crate) fn actor_takes_package_procedure(
    world: &World,
    actor: Entity,
    keep: bool,
) -> Result<bool, GameError> {
    let Some(process) = world.get::<MobileObject>(actor).and_then(|m| m.process_58) else {
        return Ok(false);
    };
    let base = *world
        .get::<crate::tes::actor_process::BaseProcess>(process)
        .expect("a process");
    let current = crate::tes::character_3d::actor_current_package(world, actor);
    if current.is_some_and(|p| package_data(world, p).type_20 == 16) {
        if Some(actor) == crate::tes::player::the_player(world) {
            return Err(GameError::unclassified(
                0x005E6D02,
                "005E6D02: the player's type 16 package",
            ));
        }
        return Ok(true);
    }
    if keep {
        return Ok(false);
    }
    let Some(package) = base.editor_package_08 else {
        return Ok(false);
    };
    let kind = package_data(world, package).type_20;
    let procedure = world
        .get::<crate::tes::actor_process::MiddleHighProcess>(process)
        .filter(|middle| middle.current_package_c0.is_some())
        .map_or(base.edit_pack_procedure_type_04, |middle| {
            middle.procedure_type_cc
        });
    Ok(matches!(kind, 10 | 16) || kind == 13 && procedure == 1)
}

// IDA: 005E6E00 Actor_UpdatePackageArrival: a living actor with a process, a package and a cell, not combat-like; flag 8 locks, else 0x40 unlocks; at the location with +0x84 clear, flag 0x20 locks and stores +0x84 = 1, else 0x100 unlocks. The process is re-read at each of its calls.
pub(crate) fn package_arrival(world: &mut World, actor: Entity) -> Result<(), GameError> {
    let Some(process) = process_of(world, actor) else {
        return Ok(());
    };
    if actor_is_dead(world, actor, false) {
        return Ok(());
    }
    let Some(package) = base_of(world, process).editor_package_08 else {
        return Ok(());
    };
    if parent_cell(world, actor).is_none() || package_is_combat_like(package_data(world, package)) {
        return Ok(());
    }
    let flags = package_data(world, package).flags_1c;
    if flags & 8 != 0 {
        doors_near(world, actor, true)?;
    } else if flags & 0x40 != 0 {
        doors_near(world, actor, false)?;
    }
    if !package_at_location(world, package, actor, false, -1.0)? {
        return Ok(());
    }
    let Some(process) = process_of(world, actor) else {
        return Ok(());
    };
    if world
        .get::<LowProcess>(process)
        .is_some_and(|low| low.byte_84 != 0)
    {
        return Ok(());
    }
    if flags & 0x20 != 0 {
        doors_near(world, actor, true)?;
        low_mut(world, process).byte_84 = 1;
        return Ok(());
    }
    if flags & 0x100 != 0 {
        doors_near(world, actor, false)?;
    }
    Ok(())
}

// IDA: 005E03C0 process vt+0xD0 (Low 00643890): a nonnull target gets 0x40000 (004D6F80(1)); +0x2C.
pub(crate) fn set_package_target(world: &mut World, actor: Entity, target: Option<Entity>) {
    let Some(process) = process_of(world, actor) else {
        return;
    };
    if let Some(target) = target {
        world
            .get_mut::<FormIdentity>(target)
            .expect("a form")
            .flags_08 |= 0x4_0000;
    }
    low_mut(world, process).field_2c = target;
}

/// IDA: 005ED860 Actor_UpdatePackageLocation (`this` the actor): the process is re-read at each of the exe's nine reads, since a move could change it. Completion: 100%.
pub fn evaluate_actor_package(
    world: &mut World,
    actor: Entity,
    arms: &mut dyn AiArms,
) -> Result<(), GameError> {
    if form_flags(world, actor) & 0x800 != 0 {
        return Ok(());
    }
    let process = process_of(world, actor).ok_or(GameError::unclassified(
        0x005ED874,
        "005ED874: no process faults",
    ))?;
    world.resource_mut::<AiProcessGlobals>().packages_evaluated += 1;
    process_update_stamps(world, process);
    if reference_is_persistent(world, actor)? {
        process_run_scheduled(world, process, actor, false, arms)?;
    }
    let process = process_of(world, actor).ok_or(GameError::unclassified(
        0x005ED896,
        "005ED896: no process faults",
    ))?;
    process_evaluate_package(world, process, actor, false, arms)?;
    let process = process_of(world, actor).ok_or(GameError::unclassified(
        0x005ED8A3,
        "005ED8A3: no process faults",
    ))?;
    let package = base_of(world, process).editor_package_08;
    if let Some(package) = package
        && package_data(world, package).type_20 > 2
        && !package_at_location(world, package, actor, false, -1.0)?
    {
        let mut horse = None;
        if package_data(world, package).flags_1c & 0x80_0000 != 0 {
            let list = &world
                .get::<TesObjectRefr>(actor)
                .expect("an actor")
                .extra_data_44;
            if let Some(entry) = list.get(EXTRA_HORSE) {
                horse = match entry.resolved_0c {
                    Some(form) => form.filter(|&form| world.get::<Actor>(form).is_some()),
                    None => {
                        return Err(GameError::unclassified(
                            0x00420840,
                            "00420840: an unresolved travel horse",
                        ));
                    }
                };
            }
        }
        let cell = package_location_cell(world, package, actor)?;
        let space = package_location_world(world, package, actor)?;
        move_to_cell(world, actor, cell, space)?;
        set_rotation_x(world, actor, crate::tes::ai::ai_package::FLT_MAX)?;
        let process = process_of(world, actor).ok_or(GameError::unclassified(
            0x005ED939,
            "005ED939: no process faults",
        ))?;
        let package = base_of(world, process).editor_package_08;
        let location = package.and_then(|package| package_data(world, package).location_24);
        if location.is_some_and(|location| location.type_00 == 1) {
            if let Some(cell) =
                cell.filter(|&cell| crate::tes::forms_world::cell_is_interior(world, cell))
            {
                let marker =
                    cell_find_marker(world, cell).or_else(|| cell_first_reference(world, cell));
                if let Some(marker) = marker {
                    let position = reference_position(world, marker)?;
                    actor_set_position(world, actor, position)?;
                }
            }
        } else if let Some(package) = package {
            let position = package_location_position(world, package, actor)?;
            actor_set_position(world, actor, position)?;
            if let Some(horse) = horse {
                move_to_cell(world, horse, cell, space)?;
                set_rotation_x(world, horse, crate::tes::ai::ai_package::FLT_MAX)?;
                let position = package_location_position(world, package, actor)?;
                actor_set_position(world, horse, position)?;
            }
        }
    }
    package_arrival(world, actor)
}
