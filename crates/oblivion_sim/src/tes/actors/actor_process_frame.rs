//! The process manager's normal frame at 00678510, entered from 0040DC2A.

use bevy_ecs::prelude::*;
use oblivion_core::{error::GameError, x87};

use super::actor::{Actor, MobileObject, TesObjectRefr, actor_process_level};
use super::actor_process::{
    ActorListId, ActorListNode, LowProcess, actor_process_list_add, actor_process_list_remove,
    release_deleted_process,
};
use crate::tes::actor_package::{actor_in_combat_package, evaluate_actor_package};
use crate::tes::actor_process::process_update_stamps;
use crate::tes::form_components::TesActorBaseData;
use crate::tes::forms_references::{form_flags, run_pending_script, save_load_active};
use crate::tes::global::game_hour;
use crate::tes::idle_frame::IdleFrameGlobals;

use super::actor_process::ActorProcessGlobals;
use crate::tes::actor::{actor_get_actor_value, process_of, set_death_state};
use crate::tes::ai_process::{AiArms, AiProcessGlobals};

// IDA: 00678510 ActorProcessManager_Update, called at 0040DC2A.
// Completion: phase stores, worker and door dispatch, clock, throttle, and all four level gates.
// TODO: [ ] Nested actor, path and High-list operations retain their own stops.
// Each callee can mutate the world. Read each enable byte only when the original reaches its branch.
pub fn update_process_manager(world: &mut World, frame_seconds: f32) -> Result<(), GameError> {
    use crate::tes::pathing::{PathingSingletons, update_path_builder};
    world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 110;
    if let Some(builder) = world
        .get_resource::<PathingSingletons>()
        .and_then(|globals| globals.path_builder_b3bf80.as_deref())
    {
        update_path_builder(builder)?;
    }
    crate::tes::cell_havok::drain_door_havok_add_queue(world)?;
    let run_lower = world
        .resource_mut::<ActorProcessGlobals>()
        .advance_process_frame_time(frame_seconds);
    world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 120;
    if run_lower {
        if world
            .resource::<ActorProcessGlobals>()
            .manager
            .low_processing_enabled_a1
            != 0
            && world.resource::<IdleFrameGlobals>().havok_step_flag_b333b8 == 0
        {
            update_low_actors(world, 0.0)?;
        }
        world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 130;
        if world
            .resource::<ActorProcessGlobals>()
            .manager
            .middle_low_processing_enabled_a3
            != 0
            && world.resource::<IdleFrameGlobals>().havok_step_flag_b333b8 == 0
        {
            update_middle_low_actors(world, 0.0, 0.0, frame_seconds)?;
        }
        world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 140;
        if world
            .resource::<ActorProcessGlobals>()
            .manager
            .middle_high_processing_enabled_a2
            != 0
        {
            update_middle_high_actors(world, 0.0, 0.0, frame_seconds)?;
        }
    }
    world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 180;
    if world
        .resource::<ActorProcessGlobals>()
        .manager
        .high_processing_enabled_a0
        != 0
    {
        super::actor_process_high::update_high_actors(world, 0.0, frame_seconds)?;
    }
    world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 190;
    Ok(())
}

impl ActorProcessGlobals {
    // IDA: 00673B10 ActorProcessManager_SetTimeSeconds(processTimeSeconds).
    // Completion: 100% of the stores. The manager discards the CRT predicate's return value.
    pub fn set_time_seconds(&mut self, seconds: f32) {
        self.time_seconds_b3bcf0 = x87::float_store(seconds);
        if self.time_seconds_b3bcf0 > 100_000.0 {
            self.time_seconds_b3bcf0 = 0.0;
        }
        if !self.time_seconds_b3bcf0.is_finite() {
            self.time_seconds_b3bcf0 = 0.0;
        }
    }

    // IDA: 00678534..006785E0 ActorProcessManager_Update, after the door Havok queue and before phase 120.
    // Completion: the clock stores and the decision to visit the three lower levels.
    pub fn advance_process_frame_time(&mut self, frame_seconds: f32) -> bool {
        let seconds = x87::float_store(frame_seconds);
        self.set_time_seconds(self.time_seconds_b3bcf0 + seconds);
        // 00678569 __ftol2_sse returns the signed FPS used by both comparisons.
        let fps = x87::ftol(1.0 / seconds);
        let elapsed = &mut self.manager.lower_level_update_accumulator_24;
        *elapsed += seconds;
        if fps >= 50 {
            return true;
        }
        // The unordered comparison takes the same reset arm as reaching one second.
        if *elapsed < 1.0 {
            *elapsed += if fps >= 10 {
                (1.0 - seconds) * 0.5
            } else {
                seconds
            };
            return false;
        }
        *elapsed = 0.0;
        true
    }
}

// IDA: 005E1DB0 Actor_HasLowLevelProcessing. Completion: the Actor virtual, including the null-base fault.
fn has_low_level_processing(world: &World, actor: Entity) -> Result<bool, GameError> {
    let data = world
        .get::<TesObjectRefr>(actor)
        .and_then(|reference| reference.base_form_1c)
        .filter(|_| world.get::<Actor>(actor).is_some())
        .and_then(|base| world.get::<TesActorBaseData>(base))
        .ok_or_else(|| {
            GameError::unclassified(
                0x005E1DD9,
                "005E1DD9: low-level processing needs an actor base",
            )
        })?;
    Ok(data.flags_04 & 0x200 == 0)
}

// IDA: 00673CDA..00673D7E in ActorProcessManager_UpdateLowActors.
// Completion: the time comparisons, timestamp virtuals and actor/script call order.
// TODO: [ ] The nested Actor_Update arms retain their own host stops.
fn update_low_actor(world: &mut World, actor: Entity, update_time: f32) -> Result<bool, GameError> {
    let process = world
        .get::<MobileObject>(actor)
        .expect("checked level 3")
        .process_58
        .expect("checked level 3");
    if !has_low_level_processing(world, actor)? && !actor_in_combat_package(world, actor, true)? {
        process_update_stamps(world, process);
        return Ok(false);
    }
    // 006599B0 MobileObject_GetProcessCurrentHour, LowProcess virtual 00629250.
    let previous = world
        .get::<LowProcess>(process)
        .expect("a LowProcess")
        .cur_hour_14;
    let current = game_hour(world);
    if previous == -1.0 {
        // 00659A20 MobileObject_UpdateProcessTimeStamps, LowProcess virtual 00643640.
        process_update_stamps(world, process);
        return Ok(false);
    }
    if current - previous >= 1.0 || current < previous || update_time != 0.0 {
        super::actor_update::actor_update(world, actor, update_time)?;
        if crate::tes::script_vm::scripts_enabled(world) {
            crate::tes::forms_init_world::reference_run_scripts(world, actor)?;
        }
        return Ok(true);
    }
    Ok(false)
}

// IDA: 0067B320 ActorProcessList_Remove on the main thread, including its conditional deletion at 0067B403.
// Completion: the Low list path uses the same unlink and release as the other process lists.
fn remove_list_actor(world: &mut World, id: ActorListId, actor: Entity) -> Result<(), GameError> {
    if actor_process_list_remove(world, id, actor, None) && form_flags(world, actor) & 0x20 != 0 {
        let active = save_load_active(world)?;
        release_deleted_process(world, actor, active);
    }
    Ok(())
}

// IDA: N/A local traversal state shared by the three forward lower-level walks.
struct ForwardActorWalk {
    list: ActorListId,
    cursor: Option<ActorListNode>,
    check_head: bool,
}

impl ForwardActorWalk {
    // IDA: N/A each walk tests the embedded head item before entering the loop.
    fn start(world: &World, list: ActorListId) -> Option<Self> {
        let head = ActorListNode::HEAD;
        world
            .resource::<ActorProcessGlobals>()
            .manager
            .list(list)
            .list
            .item(head)?;
        Some(Self {
            list,
            cursor: Some(head),
            check_head: true,
        })
    }

    // IDA: N/A read the next link before any virtual call and skip null items.
    fn next(&mut self, world: &World) -> Option<Entity> {
        let list = &world
            .resource::<ActorProcessGlobals>()
            .manager
            .list(self.list)
            .list;
        while let Some(node) = self.cursor {
            self.cursor = list.next(node);
            if let Some(actor) = list.item(node) {
                return Some(actor);
            }
        }
        None
    }

    // IDA: N/A the repeated head recheck at 00673DF6, 00674154 and 00674314.
    fn after_visit(&mut self, world: &World, actor: Entity) {
        if self.check_head {
            let head = ActorListNode::HEAD;
            let list = &world
                .resource::<ActorProcessGlobals>()
                .manager
                .list(self.list)
                .list;
            if list.item(head) == Some(actor) && list.next(head) == self.cursor {
                self.check_head = false;
            } else {
                self.cursor = Some(head);
            }
        }
    }

    // IDA: N/A the repeated state-1 move to the tail, ending traversal immediately.
    fn move_to_tail(&mut self, world: &mut World, actor: Entity) -> Result<(), GameError> {
        remove_list_actor(world, self.list, actor)?;
        actor_process_list_add(world, self.list, actor, true, false)?;
        self.cursor = None;
        self.check_head = false;
        Ok(())
    }
}

// IDA: 00673C10 ActorProcessManager_UpdateLowActors(manager, updateTime, unused).
// Completion: the forward +0x18 walk, stamp and update dispatch, head restart and removal order.
// Exclusive transaction (§3.2): save the local next link before calls. Actor_Update and scripts
// can change levels or the list. Neither persistent manager cursor is used to advance the walk.
// Host substitution: the temporary prepended pointer list uses a Vec drained in reverse.
pub fn update_low_actors(world: &mut World, update_time: f32) -> Result<(), GameError> {
    let Some(mut walk) = ForwardActorWalk::start(world, ActorListId::Low18) else {
        return Ok(());
    };
    let mut removals = Vec::new();
    while let Some(actor) = walk.next(world) {
        let flags = form_flags(world, actor);
        if flags & 0x20_0000 != 0 {
            continue;
        }
        if flags & (0x800 | 0x20) != 0 || actor_process_level(world, actor) != 3 {
            removals.push(actor);
        } else if update_low_actor(world, actor, update_time)? {
            if world
                .get::<Actor>(actor)
                .is_some_and(|data| data.dead_state_b0 == 1)
            {
                walk.move_to_tail(world, actor)?;
            }
            if actor_process_level(world, actor) != 3 {
                walk.cursor = None;
            }
        }
        walk.after_visit(world, actor);
    }
    for actor in removals.into_iter().rev() {
        remove_list_actor(world, ActorListId::Low18, actor)?;
    }
    Ok(())
}

// IDA: 00674030..00674071 / 0067436D..006743AE, process vt+0x168 and +0x164.
// Completion: a positive delay consumes the entire visit, even when it crosses zero.
fn defer_actor_update(world: &mut World, process: Entity, frame_seconds: f32) -> bool {
    let mut low = world
        .get_mut::<LowProcess>(process)
        .expect("a process with LowProcess members");
    if low.actor_update_delay_seconds_28 > 0.0 {
        low.actor_update_delay_seconds_28 -= frame_seconds;
        return true;
    }
    false
}

// IDA: 00673E90 ActorProcessManager_UpdateMiddleLowActors, forward +0x0C, local cursor.
// Completion: the ordered actor walk, phase stores, delay, and deferred unlink on the original list.
// Exclusive transaction (§3.2): virtual update and scripts can mutate the current level immediately.
// TODO: [ ] Nested Actor_Update arms on MiddleLow processes retain their own stops.
pub fn update_middle_low_actors(
    world: &mut World,
    force_time: f32,
    update_time: f32,
    frame_seconds: f32,
) -> Result<(), GameError> {
    let Some(mut walk) = ForwardActorWalk::start(world, ActorListId::MiddleLow0c) else {
        return Ok(());
    };
    world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 131;
    let mut removals = Vec::new();
    while let Some(actor) = walk.next(world) {
        let flags = form_flags(world, actor);
        if flags & 0x20_0000 != 0 {
            continue;
        }
        if flags & (0x800 | 0x20) != 0 || actor_process_level(world, actor) != 2 {
            world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 132;
            removals.push(actor);
        } else {
            let process = world
                .get::<MobileObject>(actor)
                .expect("checked level 2")
                .process_58
                .expect("checked level 2");
            if !has_low_level_processing(world, actor)?
                && !actor_in_combat_package(world, actor, true)?
            {
                world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 133;
                process_update_stamps(world, process);
            } else {
                world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 134;
                let previous = world
                    .get::<LowProcess>(process)
                    .expect("a MiddleLowProcess")
                    .cur_hour_14;
                let current = game_hour(world);
                if previous == -1.0 {
                    process_update_stamps(world, process);
                } else {
                    if current - previous >= 0.3 || current < previous || force_time != 0.0 {
                        world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 135;
                        if !defer_actor_update(world, process, frame_seconds) {
                            super::actor_update::actor_update(world, actor, update_time)?;
                            world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 136;
                            if crate::tes::script_vm::scripts_enabled(world) {
                                crate::tes::forms_init_world::reference_run_scripts(world, actor)?;
                            }
                            world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 137;
                            if world
                                .get::<Actor>(actor)
                                .is_some_and(|data| data.dead_state_b0 == 1)
                            {
                                world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 138;
                                walk.move_to_tail(world, actor)?;
                            }
                            if actor_process_level(world, actor) != 2 {
                                walk.cursor = None;
                            }
                        }
                    }
                    world.resource_mut::<IdleFrameGlobals>().phase_b3bcf4 = 505;
                }
            }
        }
        walk.after_visit(world, actor);
    }
    // 006741A7..006741CB deliberately takes manager + 0x18, even though the walk used +0x0C.
    for actor in removals.into_iter().rev() {
        remove_list_actor(world, ActorListId::Low18, actor)?;
    }
    Ok(())
}

// IDA: 00674200 ActorProcessManager_UpdateMiddleHighActors, forward +0x00, local cursor.
// Completion: ordered walk, timestamp and delay, controller, actor and script dispatch for Actors.
// Exclusive transaction (§3.2): the actor and script calls may immediately unlink the next node.
// TODO: [ ] Nested Actor_Update and controller creation carry their existing host stops.
pub fn update_middle_high_actors(
    world: &mut World,
    force_time: f32,
    update_time: f32,
    frame_seconds: f32,
) -> Result<(), GameError> {
    let Some(mut walk) = ForwardActorWalk::start(world, ActorListId::MiddleHigh00) else {
        return Ok(());
    };
    let mut removals = Vec::new();
    while let Some(actor) = walk.next(world) {
        let flags = form_flags(world, actor);
        if flags & 0x20_0000 != 0 {
            continue;
        }
        if flags & (0x800 | 0x20) != 0 || actor_process_level(world, actor) != 1 {
            removals.push(actor);
        } else {
            let process = world
                .get::<MobileObject>(actor)
                .expect("checked level 1")
                .process_58
                .expect("checked level 1");
            let previous = world
                .get::<LowProcess>(process)
                .expect("a MiddleHighProcess")
                .cur_hour_14;
            let current = game_hour(world);
            if previous == -1.0 {
                process_update_stamps(world, process);
            } else if (current - previous >= 0.15 || previous > current || force_time != 0.0)
                && !defer_actor_update(world, process, frame_seconds)
            {
                if world.get::<Actor>(actor).is_some()
                    && super::character_controller::actor_character_controller(world, actor)
                        .is_none()
                    && crate::tes::forms_init_world::reference_ni_node(world, actor).is_some()
                {
                    super::character_controller::actor_create_character_controller(
                        world,
                        actor,
                        Default::default(),
                    )?;
                }
                super::actor_update::actor_update(world, actor, update_time)?;
                if crate::tes::script_vm::scripts_enabled(world) {
                    crate::tes::forms_init_world::reference_run_scripts(world, actor)?;
                }
                if world
                    .get::<Actor>(actor)
                    .is_some_and(|data| data.dead_state_b0 == 1)
                {
                    walk.move_to_tail(world, actor)?;
                }
                if actor_process_level(world, actor) != 1 {
                    walk.cursor = None;
                }
            }
        }
        walk.after_visit(world, actor);
    }
    for actor in removals.into_iter().rev() {
        remove_list_actor(world, ActorListId::MiddleHigh00, actor)?;
        // 006744C0 vt+0x1FC: ReturnFalse_SharedVirtual for Actor, Character and Creature.
        if world.get::<Actor>(actor).is_none() {
            return Err(GameError::unclassified(
                0x006744C0,
                "006744C0: middle-high cleanup on a non-actor virtual",
            ));
        }
    }
    Ok(())
}

/// IDA: 00675310 ActorProcessManager_ProcessTrackedActors (`__thiscall` on 00B3BD00) over the +0x18 list from its head: an empty head returns; 004F9EC0; a live, based actor with Health above 0 runs 005ED860 and the walk goes on; a based actor at Health <= 0 gets 005E6680(2); then with a process, 0067B320(list, actor, previous) and the walk resumes after the previous node, or at the head; with none the walk goes on. A non-actor item spins on the same node in the exe, which the list's entities can't be. Completion: 100%.
pub fn run_ai_package_pass(world: &mut World, arms: &mut dyn AiArms) -> Result<(), GameError> {
    world.init_resource::<AiProcessGlobals>();
    let mut node = Some(ActorListNode::HEAD);
    let mut previous: Option<ActorListNode> = None;
    while let Some(current) = node {
        let manager = &world.resource::<ActorProcessGlobals>().manager;
        let list = &manager.low_18.list;
        let Some(actor) = list.item(current) else {
            if list.next(current).is_none() {
                return Ok(());
            }
            return Err(GameError::unclassified(
                0x00675333,
                "00675333: a null actor in the level-3 list faults",
            ));
        };
        world.resource_mut::<AiProcessGlobals>().actors_visited += 1;
        run_pending_script(world, actor)?;
        let deleted = form_flags(world, actor) & 0x20 != 0;
        let based = world
            .get::<TesObjectRefr>(actor)
            .expect("an actor")
            .base_form_1c
            .is_some();
        if !deleted && based && actor_get_actor_value(world, actor, 8)? > 0 {
            evaluate_actor_package(world, actor, arms)?;
            previous = Some(current);
            node = world
                .resource::<ActorProcessGlobals>()
                .manager
                .low_18
                .list
                .next(current);
            continue;
        }
        if !deleted && based && actor_get_actor_value(world, actor, 8)? <= 0 {
            set_death_state(world, actor, 2)?;
        }
        if process_of(world, actor).is_none() {
            previous = Some(current);
            node = world
                .resource::<ActorProcessGlobals>()
                .manager
                .low_18
                .list
                .next(current);
            continue;
        }
        world.resource_mut::<AiProcessGlobals>().actors_removed += 1;
        let active = save_load_active(world)?;
        actor_process_list_remove(world, ActorListId::Low18, actor, previous);
        release_deleted_process(world, actor, active);
        let list = &world.resource::<ActorProcessGlobals>().manager.low_18.list;
        node = match previous {
            Some(previous) => list.next(previous),
            None => Some(ActorListNode::HEAD),
        };
    }
    Ok(())
}

/// IDA: 00674A20 (ActorProcessManager +0x58's walk, from 0040DBEC each frame and from the AI pass): unless the player sleeps,
/// each queued reference's arms (its 3D, its cell's attachment, the 0x20000 flag); then each item's vt+0xA0(1). With the
/// list empty (nothing the port runs queues one) both walks see nothing. Completion: 100% of the empty list.
/// TODO: [ ] 00674A20's walk over queued references.
/// # Errors
/// A queued reference.
pub fn process_manager_update_queued(world: &mut World) -> Result<(), GameError> {
    let queued = world
        .get_resource::<crate::tes::actor_process::ActorProcessGlobals>()
        .is_some_and(|globals| globals.manager.queued_58.items().next().is_some());
    if queued {
        return Err(GameError::unclassified(
            0x00674A49,
            "00674A49: 00674A20's queued references",
        ));
    }
    Ok(())
}

/// IDA: 00674E10 on g_actorProcessManager (thiscall): 00B3BCF8, 00674A20's current entry, is cleared when it is `actor` (nothing in the port sets it), then 004466B0 removes the first match from +0x58. Completion: 100%.
pub fn process_manager_drop_queued(world: &mut World, actor: Entity) {
    world
        .resource_mut::<crate::tes::actor_process::ActorProcessGlobals>()
        .manager
        .queued_58
        .remove_first_matching(actor);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tes::actor_process::construct_actor_process_manager;
    use bevy_ecs::world::World;

    fn low_actor(
        world: &mut World,
        form_flags: u32,
        base_flags: u32,
        stamp: f32,
    ) -> (Entity, Entity) {
        let base = world
            .spawn(TesActorBaseData {
                flags_04: base_flags,
                ..Default::default()
            })
            .id();
        let process = crate::tes::actor_process::construct_low_process_for_copy(world);
        world.get_mut::<LowProcess>(process).unwrap().cur_hour_14 = stamp;
        let actor = world
            .spawn((
                crate::tes::form::FormIdentity {
                    flags_08: form_flags,
                    form_id_0c: 0,
                    form_type_04: 0x32,
                },
                Actor::default(),
                MobileObject {
                    process_58: Some(process),
                },
                TesObjectRefr {
                    base_form_1c: Some(base),
                    ..Default::default()
                },
            ))
            .id();
        crate::tes::actor_process::actor_process_manager_add(world, actor, 3, true, false).unwrap();
        (actor, process)
    }

    #[test]
    fn dispatcher_drains_doors_before_the_clock_and_obeys_the_havok_gate() {
        use crate::tes::cell_havok::DoorHavokAddQueue;
        use crate::tes::pathing::{PathBuilder, PathingSingletons};
        let mut world = World::new();
        construct_actor_process_manager(&mut world);
        world.init_resource::<IdleFrameGlobals>();
        let (disabled, _) = low_actor(&mut world, 0x800, 0, 0.0);
        let door = world.spawn(TesObjectRefr::default()).id();
        world.init_resource::<DoorHavokAddQueue>();
        world.resource_mut::<DoorHavokAddQueue>().0.push_front(door);
        world.insert_resource(PathingSingletons {
            path_builder_b3bf80: Some(Box::new(PathBuilder {
                thread_request_40: 1,
                ..Default::default()
            })),
            ..Default::default()
        });
        assert!(update_process_manager(&mut world, 0.01).is_err());
        assert_eq!(world.resource::<IdleFrameGlobals>().phase_b3bcf4, 110);
        assert_eq!(
            world.resource::<ActorProcessGlobals>().time_seconds_b3bcf0,
            0.0
        );
        assert_eq!(world.resource::<DoorHavokAddQueue>().0.front(), Some(&door));

        world
            .resource_mut::<PathingSingletons>()
            .path_builder_b3bf80
            .as_mut()
            .unwrap()
            .thread_request_40 = 0;
        world
            .resource_mut::<IdleFrameGlobals>()
            .havok_step_flag_b333b8 = 1;
        update_process_manager(&mut world, 0.01).unwrap();
        assert!(world.resource::<DoorHavokAddQueue>().0.is_empty());
        assert_eq!(
            world
                .resource::<ActorProcessGlobals>()
                .manager
                .low_18
                .list
                .items(),
            [disabled]
        );
        assert_eq!(
            world.resource::<ActorProcessGlobals>().time_seconds_b3bcf0,
            0.01
        );
        assert_eq!(world.resource::<IdleFrameGlobals>().phase_b3bcf4, 190);
        world
            .resource_mut::<IdleFrameGlobals>()
            .havok_step_flag_b333b8 = 0;
        update_process_manager(&mut world, 0.01).unwrap();
        assert!(
            world
                .resource::<ActorProcessGlobals>()
                .manager
                .low_18
                .list
                .items()
                .is_empty()
        );
    }

    #[test]
    fn low_walk_stamps_skips_and_removes_in_original_order() {
        let mut world = World::new();
        construct_actor_process_manager(&mut world);
        let (first, unstamped) = low_actor(&mut world, 0, 0, -1.0);
        let (second, no_processing) = low_actor(&mut world, 0, 0x200, 0.0);
        let (third, fresh) = low_actor(&mut world, 0, 0, 11.5);
        let (disabled, _) = low_actor(&mut world, 0x800, 0, -1.0);
        let (skipped, untouched) = low_actor(&mut world, 0x20_0820, 0, -1.0);
        let (deleted, _) = low_actor(&mut world, 0x20, 0, -1.0);
        // The last discovered removal unlinks before its save/load lookup faults. The
        // pending disabled actor remains linked. Earlier timestamp stores survive the stop.
        assert!(update_low_actors(&mut world, 0.0).is_err());
        assert_eq!(
            world
                .resource::<ActorProcessGlobals>()
                .manager
                .low_18
                .list
                .items(),
            [first, second, third, disabled, skipped]
        );
        assert!(
            world
                .get::<super::super::actor_process::ProcessLevelLow>(deleted)
                .is_none()
        );
        for process in [unstamped, no_processing] {
            assert_eq!(world.get::<LowProcess>(process).unwrap().cur_hour_14, 12.0);
            assert_ne!(
                world.get::<LowProcess>(process).unwrap().cur_packed_date_18,
                0
            );
        }
        assert_eq!(world.get::<LowProcess>(fresh).unwrap().cur_hour_14, 11.5);
        assert_eq!(
            world.get::<LowProcess>(untouched).unwrap().cur_hour_14,
            -1.0
        );
        update_low_actors(&mut world, 0.0).unwrap();
        assert_eq!(
            world
                .resource::<ActorProcessGlobals>()
                .manager
                .low_18
                .list
                .items(),
            [first, second, third, skipped]
        );
    }

    #[test]
    fn middle_low_uses_its_delay_and_removes_stale_entries_from_low() {
        use crate::tes::actor_process::{actor_process_manager_add, construct_middle_low_process};
        let mut world = World::new();
        construct_actor_process_manager(&mut world);
        world.init_resource::<IdleFrameGlobals>();
        let (actor, _) = low_actor(&mut world, 0, 0, 0.0);
        let middle = construct_middle_low_process(&mut world);
        world.get_mut::<MobileObject>(actor).unwrap().process_58 = Some(middle);
        actor_process_manager_add(&mut world, actor, 2, true, false).unwrap();
        world.get_mut::<LowProcess>(middle).unwrap().cur_hour_14 = -1.0;
        update_middle_low_actors(&mut world, 0.0, 0.0, 0.25).unwrap();
        assert_eq!(world.resource::<IdleFrameGlobals>().phase_b3bcf4, 134);
        assert_eq!(world.get::<LowProcess>(middle).unwrap().cur_hour_14, 12.0);
        world
            .get_mut::<LowProcess>(middle)
            .unwrap()
            .actor_update_delay_seconds_28 = 0.125;
        update_middle_low_actors(&mut world, 0.0, 0.0, 0.25).unwrap();
        assert_eq!(
            world
                .get::<LowProcess>(middle)
                .unwrap()
                .actor_update_delay_seconds_28,
            0.125
        );
        update_middle_low_actors(&mut world, 1.0, 0.0, 0.25).unwrap();
        assert_eq!(world.resource::<IdleFrameGlobals>().phase_b3bcf4, 505);
        assert_eq!(
            world
                .get::<LowProcess>(middle)
                .unwrap()
                .actor_update_delay_seconds_28,
            -0.125
        );
        world
            .get_mut::<crate::tes::form::FormIdentity>(actor)
            .unwrap()
            .flags_08 |= 0x800;
        update_middle_low_actors(&mut world, 0.0, 0.0, 0.25).unwrap();
        assert_eq!(world.resource::<IdleFrameGlobals>().phase_b3bcf4, 132);
        let manager = &world.resource::<ActorProcessGlobals>().manager;
        assert_eq!(manager.middle_low_0c.list.items(), [actor]);
        assert!(manager.low_18.list.items().is_empty());
    }

    #[test]
    fn middle_high_updates_only_due_actors_and_cleans_its_own_list() {
        use crate::tes::actor_process::{
            MIDDLE_HIGH_PROCESS_VTABLE, MiddleHighProcess, actor_process_manager_add,
        };
        use crate::tes::form::{FormIdentity, OriginalVtable};
        let mut world = World::new();
        construct_actor_process_manager(&mut world);
        update_middle_high_actors(&mut world, 1.0, 0.0, 0.25).unwrap();
        let (actor, process) = low_actor(&mut world, 0, 0, 11.9);
        world.entity_mut(process).insert((
            OriginalVtable(MIDDLE_HIGH_PROCESS_VTABLE),
            MiddleHighProcess::default(),
        ));
        actor_process_manager_add(&mut world, actor, 1, true, false).unwrap();
        world
            .get_mut::<LowProcess>(process)
            .unwrap()
            .actor_update_delay_seconds_28 = 0.125;
        update_middle_high_actors(&mut world, 0.0, 0.0, 0.25).unwrap();
        assert_eq!(
            world
                .get::<LowProcess>(process)
                .unwrap()
                .actor_update_delay_seconds_28,
            0.125
        );
        world.get_mut::<LowProcess>(process).unwrap().cur_hour_14 = 11.8;
        update_middle_high_actors(&mut world, 0.0, 0.0, 0.25).unwrap();
        assert_eq!(
            world
                .get::<LowProcess>(process)
                .unwrap()
                .actor_update_delay_seconds_28,
            -0.125
        );
        world.get_mut::<FormIdentity>(actor).unwrap().flags_08 = 0x800;
        update_middle_high_actors(&mut world, 0.0, 0.0, 0.25).unwrap();
        let manager = &world.resource::<ActorProcessGlobals>().manager;
        assert!(manager.middle_high_00.list.items().is_empty());
        assert_eq!(manager.low_18.list.items(), [actor]);
    }

    #[test]
    fn lower_lists_match_the_original_post_close_sweep() {
        use crate::tes::actor::{Character, Creature};
        use crate::tes::actor_process::{
            BaseProcess, actor_process_manager_add, construct_low_process_for_copy,
            construct_middle_low_process,
        };
        use crate::tes::default_forms::TimeGlobals;
        use crate::tes::form::{FormIdentity, OriginalVtable};
        use crate::tes::forms_ai::TesPackage;
        use crate::tes::global::TESGlobal;

        let mut world = World::new();
        construct_actor_process_manager(&mut world);
        world.init_resource::<IdleFrameGlobals>();
        let rows = include_str!("../../../../../fixtures/rows/process-manager-lower-sweep.rows");
        let mut lines = rows.lines().filter(|line| !line.starts_with('#'));
        let hex = |s: &str| u32::from_str_radix(s, 16).unwrap();
        let mut times = lines.next().unwrap().split_whitespace();
        let globals = std::array::from_fn(|_| {
            Some(
                world
                    .spawn(TESGlobal {
                        variable_type_20: b'f',
                        value_bits_24: hex(times.next().unwrap()),
                    })
                    .id(),
            )
        });
        assert!(times.next().is_none());
        world.insert_resource(TimeGlobals {
            globals_b332e0: globals,
        });
        let mut expected = Vec::new();
        for line in lines {
            let v: Vec<_> = line.split_whitespace().collect();
            let (level, process) = match v[0] {
                "low" => (3, construct_low_process_for_copy(&mut world)),
                "ml" => (2, construct_middle_low_process(&mut world)),
                _ => panic!("unknown list"),
            };
            assert_eq!(world.get::<OriginalVtable>(process).unwrap().0, hex(v[4]));
            let base = world
                .spawn(TesActorBaseData {
                    flags_04: hex(v[3]),
                    ..Default::default()
                })
                .id();
            let identity = FormIdentity {
                form_id_0c: hex(v[1]),
                flags_08: hex(v[2]),
                form_type_04: v[9].parse().unwrap(),
            };
            let actor = world
                .spawn((
                    identity,
                    Actor::default(),
                    MobileObject {
                        process_58: Some(process),
                    },
                    TesObjectRefr {
                        base_form_1c: Some(base),
                        ..Default::default()
                    },
                ))
                .id();
            match identity.form_type_04 {
                50 => {
                    world.entity_mut(actor).insert(Character::default());
                }
                51 => {
                    world.entity_mut(actor).insert(Creature::default());
                }
                kind => panic!("unexpected reference type {kind}"),
            }
            let mut low = world.get_mut::<LowProcess>(process).unwrap();
            low.cur_hour_14 = f32::from_bits(hex(v[5]));
            low.cur_packed_date_18 = hex(v[6]) as i32;
            low.actor_update_delay_seconds_28 = f32::from_bits(hex(v[7]));
            if v[8] != "-" {
                let package = world
                    .spawn(TesPackage {
                        type_20: v[8].parse().unwrap(),
                        ..Default::default()
                    })
                    .id();
                world
                    .get_mut::<BaseProcess>(process)
                    .unwrap()
                    .editor_package_08 = Some(package);
            }
            actor_process_manager_add(&mut world, actor, level, true, false).unwrap();
            expected.push((
                level,
                identity.form_id_0c,
                process,
                hex(v[10]),
                hex(v[6]),
                hex(v[7]),
            ));
        }
        update_low_actors(&mut world, 0.0).unwrap();
        update_middle_low_actors(&mut world, 0.0, 0.0, f32::from_bits(0x3E29FBE8)).unwrap();
        let manager = &world.resource::<ActorProcessGlobals>().manager;
        let actual_ids = manager
            .low_18
            .list
            .items()
            .into_iter()
            .map(|a| (3, a))
            .chain(
                manager
                    .middle_low_0c
                    .list
                    .items()
                    .into_iter()
                    .map(|a| (2, a)),
            )
            .map(|(level, actor)| (level, world.get::<FormIdentity>(actor).unwrap().form_id_0c));
        assert!(actual_ids.eq(expected.iter().map(|&(level, id, ..)| (level, id))));
        assert_eq!(expected.len(), 1875);
        for (_, id, process, hour, date, delay) in expected {
            let low = world.get::<LowProcess>(process).unwrap();
            assert_eq!(
                (
                    low.cur_hour_14.to_bits(),
                    low.cur_packed_date_18 as u32,
                    low.actor_update_delay_seconds_28.to_bits()
                ),
                (hour, date, delay),
                "form {id:08X}"
            );
        }
    }

    #[test]
    fn process_clock_matches_eight_frames_after_done() {
        let mut world = World::new();
        construct_actor_process_manager(&mut world);
        let mut state = world.resource_mut::<ActorProcessGlobals>();
        let rows = include_str!("../../../../../fixtures/rows/process-manager-after-done.rows");
        for (index, row) in rows.lines().skip(1).enumerate() {
            let words: Vec<_> = row.split_whitespace().collect();
            let bits = |column: usize| u32::from_str_radix(words[column], 16).unwrap();
            if index == 0 {
                state.time_seconds_b3bcf0 = f32::from_bits(bits(1));
                state.manager.lower_level_update_accumulator_24 = f32::from_bits(bits(4));
            }
            assert_eq!(state.time_seconds_b3bcf0.to_bits(), bits(1));
            assert_eq!(
                state.manager.lower_level_update_accumulator_24.to_bits(),
                bits(4)
            );
            let lower_levels = state.advance_process_frame_time(f32::from_bits(bits(2)));
            assert_eq!(state.time_seconds_b3bcf0.to_bits(), bits(3));
            assert_eq!(
                state.manager.lower_level_update_accumulator_24.to_bits(),
                bits(5)
            );
            assert_eq!(
                lower_levels,
                words[6].starts_with("low,"),
                "frame {}",
                index + 1
            );
        }
        assert_eq!(rows.lines().skip(1).count(), 8);
    }

    #[test]
    fn process_clock_thresholds_and_reset() {
        let mut world = World::new();
        construct_actor_process_manager(&mut world);
        let mut state = world.resource_mut::<ActorProcessGlobals>();
        for value in [-1.0, -0.0, 100_000.0] {
            state.set_time_seconds(value);
            assert_eq!(state.time_seconds_b3bcf0.to_bits(), value.to_bits());
        }
        for value in [100_001.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            state.set_time_seconds(value);
            assert_eq!(state.time_seconds_b3bcf0.to_bits(), 0);
        }
        assert!(state.advance_process_frame_time(1.0 / 50.0));
        assert_eq!(state.manager.lower_level_update_accumulator_24, 1.0 / 50.0);
        state.manager.lower_level_update_accumulator_24 = 0.0;
        assert!(!state.advance_process_frame_time(0.1));
        assert_eq!(
            state.manager.lower_level_update_accumulator_24,
            0.1 + (1.0 - 0.1) * 0.5
        );
        state.manager.lower_level_update_accumulator_24 = f32::NAN;
        assert!(state.advance_process_frame_time(0.125));
        assert_eq!(state.manager.lower_level_update_accumulator_24.to_bits(), 0);
    }
}
