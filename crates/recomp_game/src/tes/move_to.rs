//! `00512540`, the MoveTo a script command (`00512870`) reaches, for a reference other than the player, with `00675D50`'s walk over the managed actors. A new game's commands 45, 46 and 54 are the first: two goblin totem staffs, and the Forlorn Watchman just enabled.
//!
//! Evidence: `records-run-u-status1-cover.txt` and `run-v-moveto`'s snapshot.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::actor::{Actor, MobileObject, TesObjectRefr};
use super::actor_enable::reevaluate_package;
use super::actor_process::{ActorProcessGlobals, LOW_PROCESS_VTABLE, LowProcess};
use super::ai_cell_moves::{cell_attached, move_to_cell};
use super::ai_package::{package_data, parent_cell, reference_position, reference_world};
use super::ai_process::process_update_stamps;
use super::character_controller::actor_character_controller;
use super::form::OriginalVtable;
use super::form_components::FormRef;

// IDA: 00A32048, FLT_MAX: the X rotation 00512540 gives a non-player whose target's cell isn't attached.
const NOT_ATTACHED_X: f32 = f32::MAX;

// IDA: N/A the process's current package, +0x08.
fn package_of(world: &World, actor: Entity) -> Option<Entity> {
    let process = world.get::<MobileObject>(actor)?.process_58?;
    world
        .get::<super::actor_process::BaseProcess>(process)?
        .editor_package_08
}

// IDA: 00675D50(g_actorProcessManager, actor, 0). The actor's process: vt+0x448(-vt+0x444): a LowProcess answers 0 and does nothing; a HighProcess (00629650, 00629660) zeroes its +0x274, and for the player +0x738 = 0 follows. Another process is a stop. Then the four lists by 00673A50 (levels 0 to 3): each actor of each, through vt+0x190 (1 on a Character), whose vt+0x334(1) (005E6110) is in a combat package (types 0x0C and 0x0D), is a stop; else one with a type 0x0F package (005E6BA0) whose target is this actor reaches 005EAE70, a stop. Completion: 100% of the arms with neither.
pub(super) fn release_followers(world: &mut World, actor: Entity) -> Result<(), StreamError> {
    if let Some(process) = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
    {
        let vtable = world.get::<OriginalVtable>(process).map(|vtable| vtable.0);
        if vtable == Some(super::actor_process::HIGH_PROCESS_VTABLE) {
            world
                .get_mut::<super::actor_process::HighProcess>(process)
                .expect("a HighProcess")
                .field_274 = 0;
            if let Some(mut player) = world.get_mut::<super::player::PlayerCharacter>(actor) {
                player.guard_combat_active_738 = 0;
            }
        } else if vtable != Some(LOW_PROCESS_VTABLE) {
            return Err(StreamError(
                "00675D6C: vt+0x444 on a process other than a Low or HighProcess",
            ));
        }
    }
    let manager = &world.resource::<ActorProcessGlobals>().manager;
    let lists = [
        &manager.high_68,
        &manager.middle_high_00,
        &manager.low_0c,
        &manager.low_18,
    ];
    for list in lists {
        for other in list.list.items() {
            if world.get::<Actor>(other).is_none() {
                continue;
            }
            let Some(package) = package_of(world, other) else {
                continue;
            };
            let data = package_data(world, package);
            if matches!(data.type_20, 0x0C | 0x0D) {
                return Err(StreamError("00675DFF: a managed actor in combat"));
            }
            if data.type_20 == 0x0F
                && data.target_28.is_some_and(|target| {
                    // 00569E60 PackageTarget_GetReference: type 0's form.
                    target.type_00 == 0 && target.value_04 == FormRef::Resolved(Some(actor))
                })
            {
                return Err(StreamError(
                    "00675E58: 005EAE70 for a follower of the moved actor",
                ));
            }
        }
    }
    Ok(())
}

// IDA: vt+0x178(0) on the moved reference: TESObjectREFR 004D66E0 passes vt+0x154's node to 00897A20, which returns on null; Character 0065B5C0 returns with no controller. A node or a controller is a stop.
fn refresh_node(world: &World, reference: Entity, where_: &'static str) -> Result<(), StreamError> {
    let actor = world.get::<Actor>(reference).is_some();
    let loaded = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .loaded_3d_3c
        != 0;
    if (actor && actor_character_controller(world, reference).is_some()) || (!actor && loaded) {
        return Err(StreamError(where_));
    }
    Ok(())
}

/// IDA: 00512540(reference, target, x, y, z) for a reference other than the player. A Character takes 00675D50 (`release_followers`). The target's parent cell and exterior world (004D6670), and its position (vt+0x174) plus the offsets, each a float sum in the x87's double precision, stored as floats. 004D8A30 SetPosition, vt+0x178(0), an actor's controller (none here). When the target's cell is attached (0043F8C0(cell, 0)), 004D8A10 takes the target's Z rotation and 004D89D0 0.0; otherwise X takes 00A32048, FLT_MAX. 004DD4B0, then for an actor with a process its vt+0x20 (00643640), 00601B80 and vt+0xB8 (00643740: the path builder's removal, a recorded substitution, and +0x34 freed, a stop when set); an actor with none only warns. Last, 00665260 with no active quest. Completion: 100% of those arms.
/// # Errors
/// A stop named above, or one of 004DD4B0's or 00601B80's.
pub fn move_reference_to(
    world: &mut World,
    reference: Entity,
    target: Entity,
    offset: [f32; 3],
) -> Result<(), StreamError> {
    let actor = world.get::<Actor>(reference).is_some();
    if actor {
        release_followers(world, reference)?;
    }
    let cell = parent_cell(world, target);
    let space = reference_world(world, target);
    let target_rotation = world
        .get::<TesObjectRefr>(target)
        .expect("a reference")
        .rotation_20;
    let base = reference_position(world, target)?;
    let position: [f32; 3] =
        std::array::from_fn(|at| x87::stored_float(f64::from(base[at]) + f64::from(offset[at])));
    // 004D8A30: +0x2C..+0x34, then vt+0x40(4).
    world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference")
        .position_2c = position;
    super::forms_items::set_change_flags(world, reference, 4)?;
    refresh_node(
        world,
        reference,
        "00512774: vt+0x178 with a node or a controller",
    )?;
    let attached = cell.is_some_and(|cell| cell_attached(world, cell, false));
    let mut refr = world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference");
    if attached {
        refr.rotation_20[2] = target_rotation[2];
        refr.rotation_20[0] = 0.0;
    } else {
        refr.rotation_20[0] = NOT_ATTACHED_X;
    }
    // 004D8A10 and 004D89D0 each end in vt+0x40(4).
    if attached {
        super::forms_items::set_change_flags(world, reference, 4)?;
    }
    super::forms_items::set_change_flags(world, reference, 4)?;
    move_to_cell(world, reference, cell, space)?;
    if actor {
        if let Some(process) = world
            .get::<MobileObject>(reference)
            .expect("an actor")
            .process_58
        {
            process_update_stamps(world, process);
            reevaluate_package(world, reference)?;
            let process = world
                .get::<MobileObject>(reference)
                .expect("an actor")
                .process_58
                .ok_or(StreamError("00512824: a process 00601B80 took away"))?;
            if world.get::<LowProcess>(process).is_none() {
                return Err(StreamError(
                    "00512830: vt+0xB8 on a process other than a LowProcess",
                ));
            }
            if world
                .get::<LowProcess>(process)
                .expect("a LowProcess")
                .pathing_34
                != 0
            {
                return Err(StreamError("00643762: freeing a LowProcess's +0x34"));
            }
        } else {
            recomp_core::logging::emit(
                recomp_core::logging::Level::Warning,
                "Actor being moved is Disabled he has no AI process.",
            );
        }
    }
    super::script_commands::stop_on_active_quest(world)
}
