//! NiTimeController Start and Stop over a controller list.

use std::collections::HashSet;

use bevy_ecs::prelude::*;

use super::particle_controller::{
    ParticleEmitterController, ParticleTimeControllerCold, controller_time, controller_time_mut,
};
use super::stream_objects::NiClass;

const NEGATIVE_MAX: f32 = -f32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleControllerLifecycleStatus {
    Completed,
    UnsupportedNullOwner,
    UnsupportedCycle,
    UnsupportedControllerType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleControllerLifecycleResult {
    pub status: ParticleControllerLifecycleStatus,
    pub visited: usize,
}

// IDA: 00715540 void __thiscall NiTimeController::Start(NiTimeController* self, float currentTime)
// Completion: 100%: flags OR, last-time sentinel store, then the optional start-time store
const fn start_known_controller(time: &mut ParticleTimeControllerCold, current_time: f32) {
    time.fields.flags |= 0x0008;
    time.fields.last_time = NEGATIVE_MAX;
    if time.fields.flags & 0x0001 != 0 {
        time.fields.start_time = current_time;
    }
}

// IDA: 00715570 void __thiscall NiTimeController::Stop(NiTimeController* self)
// Completion: 100%: active-bit clear, last-time sentinel, conditional start sentinel
const fn stop_known_controller(time: &mut ParticleTimeControllerCold) {
    time.fields.flags &= 0xFFF7;
    time.fields.last_time = NEGATIVE_MAX;
    if time.fields.flags & 0x0001 != 0 {
        time.fields.start_time = NEGATIVE_MAX;
    }
}

// IDA: N/A host dispatcher over NiObjectNET +0x0C and controller +0x34. A dangling entity stands for the C++ typed null owner
// Completion: 80% of the head-to-next order; the caller of the original lifecycle virtuals is unresolved
// TODO: [ ] Verify the original list caller and mutations during virtual dispatch.
fn walk_chain(
    world: &mut World,
    head: Option<Entity>,
    mut action: impl FnMut(&mut World, Entity) -> bool,
) -> ParticleControllerLifecycleResult {
    use ParticleControllerLifecycleStatus as Status;
    let mut result = ParticleControllerLifecycleResult {
        status: Status::Completed,
        visited: 0,
    };
    let mut cursor = head;
    let mut visited = HashSet::new();
    while let Some(entity) = cursor {
        if world.get_entity(entity).is_err() {
            result.status = Status::UnsupportedNullOwner;
            return result;
        }
        if !visited.insert(entity) {
            result.status = Status::UnsupportedCycle;
            return result;
        }
        result.visited += 1;
        if !action(world, entity) {
            result.status = Status::UnsupportedControllerType;
            return result;
        }
        cursor = controller_time(world, entity).and_then(|time| time.next_34);
    }
    result
}

/// IDA: 00715540 NiTimeController::Start, 00715570 Stop, 0074F820 NiPSysEmitterCtlr::Stop, each reached via original vtable +0x4C /
/// +0x50. Completion: 85% of the typed scalar lifecycle and the supplied head-to-next walk.
/// TODO: [ ] The original ObjectNET list-caller ABI and timing source.
pub struct ParticleControllerLifecycle;

impl ParticleControllerLifecycle {
    // IDA: 00715540 is vtable +0x4C for NiTimeController and the five bound classes; no derived override in those vtables
    // Completion: 100% of the known per-node scalar effects, 80% of the host chain traversal
    // TODO: [ ] The original ObjectNET lifecycle caller.
    pub fn start_chain(
        world: &mut World,
        head: Option<Entity>,
        current_time: f32,
    ) -> ParticleControllerLifecycleResult {
        walk_chain(world, head, |world, entity| {
            match controller_time_mut(world, entity) {
                Some(mut time) => {
                    start_known_controller(&mut time, current_time);
                    true
                }
                None => false,
            }
        })
    }

    // IDA: 00715570 vtable +0x50; NiPSysEmitterCtlr's +0x50 is 0074F820, storing -FLT_MAX at +0x50 before tail-jumping 00715570
    // Completion: 100% of the known per-node scalar effects, 80% of the host chain traversal
    // TODO: [ ] The exact original virtual list caller.
    pub fn stop_chain(
        world: &mut World,
        head: Option<Entity>,
    ) -> ParticleControllerLifecycleResult {
        walk_chain(world, head, |world, entity| {
            if world.get::<NiClass>(entity) == Some(&NiClass::ParticleEmitterController) {
                return match world.get_mut::<ParticleEmitterController>(entity) {
                    Some(mut emitter) => {
                        emitter.stop();
                        true
                    }
                    None => false,
                };
            }
            match controller_time_mut(world, entity) {
                Some(mut time) => {
                    stop_known_controller(&mut time);
                    true
                }
                None => false,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::particle_controller::ParticleUpdateController;
    use super::*;

    /// Ported from `particle_controller_lifecycle_tests`' `verify_head_next_start_stop`.
    #[test]
    fn head_next_start_stop() {
        const SENTINEL: f32 = -f32::MAX;
        let mut world = World::new();
        let mut emitter = ParticleEmitterController::default();
        emitter.time.fields.flags = 0x0000; // preserve start time
        emitter.time.fields.start_time = 9.0;
        emitter.time.fields.last_time = 10.0;
        emitter.last_emit_time_50 = 11.0;
        let emitter = world
            .spawn((NiClass::ParticleEmitterController, emitter))
            .id();
        let mut head = ParticleUpdateController::default();
        head.time.fields.flags = 0x0001; // conditional start-time store
        head.time.fields.start_time = 4.0;
        head.time.fields.last_time = 5.0;
        head.time.next_34 = Some(emitter);
        let head = world.spawn((NiClass::ParticleUpdateController, head)).id();

        let started = ParticleControllerLifecycle::start_chain(&mut world, Some(head), 42.0);
        assert_eq!(started.status, ParticleControllerLifecycleStatus::Completed);
        assert_eq!(started.visited, 2);
        let fields = world
            .get::<ParticleUpdateController>(head)
            .unwrap()
            .time
            .fields;
        assert_eq!(fields.flags, 0x0009);
        assert_eq!(fields.last_time, SENTINEL);
        assert_eq!(fields.start_time, 42.0);
        let state = world.get::<ParticleEmitterController>(emitter).unwrap();
        assert_eq!(state.time.fields.flags, 0x0008);
        assert_eq!(state.time.fields.last_time, SENTINEL);
        assert_eq!(state.time.fields.start_time, 9.0);
        assert_eq!(state.last_emit_time_50, 11.0);

        let stopped = ParticleControllerLifecycle::stop_chain(&mut world, Some(head));
        assert_eq!(stopped.status, ParticleControllerLifecycleStatus::Completed);
        assert_eq!(stopped.visited, 2);
        let fields = world
            .get::<ParticleUpdateController>(head)
            .unwrap()
            .time
            .fields;
        assert_eq!(fields.flags, 0x0001);
        assert_eq!(fields.last_time, SENTINEL);
        assert_eq!(fields.start_time, SENTINEL);
        let state = world.get::<ParticleEmitterController>(emitter).unwrap();
        assert_eq!(state.time.fields.flags, 0);
        assert_eq!(state.time.fields.last_time, SENTINEL);
        assert_eq!(state.time.fields.start_time, 9.0);
        assert_eq!(state.last_emit_time_50, SENTINEL);
    }

    /// Ported from `particle_controller_lifecycle_tests`' `verify_null_and_cycle_boundaries`. A despawned entity stands for the C++
    /// typed null `shared_ptr`.
    #[test]
    fn null_and_cycle_boundaries() {
        let mut world = World::new();
        let empty = ParticleControllerLifecycle::start_chain(&mut world, None, 1.0);
        assert_eq!(empty.status, ParticleControllerLifecycleStatus::Completed);
        assert_eq!(empty.visited, 0);

        let typed_null = world.spawn_empty().id();
        world.despawn(typed_null);
        let null_owner = ParticleControllerLifecycle::stop_chain(&mut world, Some(typed_null));
        assert_eq!(
            null_owner.status,
            ParticleControllerLifecycleStatus::UnsupportedNullOwner
        );
        assert_eq!(null_owner.visited, 0);

        let first = world
            .spawn((
                NiClass::ParticleUpdateController,
                ParticleUpdateController::default(),
            ))
            .id();
        let mut second = ParticleEmitterController::default();
        second.time.next_34 = Some(first);
        let second = world
            .spawn((NiClass::ParticleEmitterController, second))
            .id();
        controller_time_mut(&mut world, first).unwrap().next_34 = Some(second);
        let cycle = ParticleControllerLifecycle::start_chain(&mut world, Some(first), 7.0);
        assert_eq!(
            cycle.status,
            ParticleControllerLifecycleStatus::UnsupportedCycle
        );
        assert_eq!(cycle.visited, 2);
        assert_eq!(
            controller_time(&world, first).unwrap().fields.start_time,
            -f32::MAX
        );
    }

    /// IDA: N/A a linked object that is not one of the bound controllers.
    #[test]
    fn other_classes_stop_the_walk() {
        let mut world = World::new();
        let data = world.spawn(NiClass::FloatKeyData).id();
        let result = ParticleControllerLifecycle::start_chain(&mut world, Some(data), 1.0);
        assert_eq!(
            result.status,
            ParticleControllerLifecycleStatus::UnsupportedControllerType
        );
        assert_eq!(result.visited, 1);
    }
}
