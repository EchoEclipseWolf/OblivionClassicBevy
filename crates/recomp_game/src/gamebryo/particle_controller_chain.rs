//! 00715CE0 NiTimeController::SetTarget and the NiObjectNET controller list.
//!
//! The C++ port kept a separate `ParticleControllerLinkNode` and `ParticleControllerTargetState`. Here the list is the real one:
//! the head is the target's `NiObjectNet::controller_0c` and next is the controller's `next_34`, and "NiGeometryData +0x2E" is
//! `ParticleData::geometry_extra_flags_2e`, reached through the target's +0xB4 data link.

use std::collections::HashSet;

use bevy_ecs::prelude::*;

use super::object_net::NiObjectNet;
use super::particle_controller::{controller_time, controller_time_mut};
use super::particle_data::ParticleData;
use super::particle_system::ParticleSystem;

/// IDA: a controller's vtable +0x60, evaluated only after a successful head attach. Evidence covers NiPSysResetOnLoopCtlr
/// (0069D990, false); an absent component stays an explicit boundary.
/// TODO: [ ] Map +0x60 for every other controller class.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarksGeometryDirty60(pub bool);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleControllerSetTargetStatus {
    SameTarget,
    Attached,
    Detached,
    UnsupportedNullController,
    UnsupportedExpiredOldTarget,
    UnsupportedOldChain,
    UnsupportedNewChain,
    UnsupportedAlreadyInNewChain,
    UnsupportedMissingVirtualPredicate,
    UnsupportedMissingGeometryDataAfterAttach,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleControllerSetTargetStage {
    Unchanged,
    OldTargetUnlinked,
    TargetStored,
    NewHeadAttached,
    GeometryFlagsStored,
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleControllerSetTargetResult {
    pub status: ParticleControllerSetTargetStatus,
    pub stage: ParticleControllerSetTargetStage,
}

#[derive(Default)]
struct ChainInspection {
    contains_controller: bool,
    has_cycle: bool,
}

// IDA: N/A host-only preflight around 006FFE90 / 006FFE60 traversal. A link to a non-controller counts as malformed, with a cycle
// Completion: 100% of the finite-chain and duplicate check; not an original traversal branch
// TODO: [ ] Replace the guard only if original malformed-chain behaviour is required by a verified owner.
fn inspect_chain(world: &World, head: Option<Entity>, controller: Entity) -> ChainInspection {
    let mut visited = HashSet::new();
    let mut result = ChainInspection::default();
    let mut current = head;
    while let Some(entry) = current {
        if !visited.insert(entry) {
            result.has_cycle = true;
            break;
        }
        if entry == controller {
            result.contains_controller = true;
        }
        let Some(time) = controller_time(world, entry) else {
            result.has_cycle = true;
            break;
        };
        current = time.next_34;
    }
    result
}

// IDA: N/A a helper, with no address of its own.
fn head_of(world: &World, target: Entity) -> Option<Entity> {
    world
        .get::<NiObjectNet>(target)
        .and_then(|net| net.controller_0c)
}

// IDA: 006FFE90 void __thiscall NiObjectNET_RemoveController(NiObjectNET* target, NiTimeController* controller)
// Completion: 80% of the well-formed head or interior splice and the strong-next release
// TODO: [ ] Original Interlocked refcount, destructor and SEH effects.
fn unlink_controller(world: &mut World, target: Entity, controller: Entity) {
    let next = controller_time(world, controller)
        .expect("the preflight found this controller")
        .next_34;
    let mut slot_owner = None;
    let mut current = head_of(world, target);
    while current != Some(controller) {
        let entry = current.expect("the preflight found this controller in the chain");
        slot_owner = Some(entry);
        current = controller_time(world, entry)
            .expect("the preflight walked this chain")
            .next_34;
    }
    match slot_owner {
        Some(owner) => {
            controller_time_mut(world, owner)
                .expect("the preflight walked this chain")
                .next_34 = next;
        }
        None => {
            world
                .get_mut::<NiObjectNet>(target)
                .expect("the preflight read this head")
                .controller_0c = next;
        }
    }
    controller_time_mut(world, controller)
        .expect("the preflight found this controller")
        .next_34 = None;
}

// IDA: 006FFE60 void __thiscall NiObjectNET_PrependController(NiObjectNET* target, NiTimeController* controller)
// Completion: 90%: the next-owner assignment precedes the head-owner assignment
// TODO: [ ] Bind 006C61E0 / 00478300 to original intrusive refcount storage.
fn prepend_controller(world: &mut World, target: Entity, controller: Entity) {
    let head = head_of(world, target);
    controller_time_mut(world, controller)
        .expect("the preflight found this controller")
        .next_34 = head;
    world
        .get_mut::<NiObjectNet>(target)
        .expect("the preflight read this head")
        .controller_0c = Some(controller);
}

/// IDA: 00715CE0 SetTarget; 006FFE60 prepend; 006FFE90 unlink. Completion: 75% of the supported well-formed chain path, keeping
/// attach and flag order.
/// TODO: [ ] Map derived +0x60 predicates and original target and refcount owners.
/// TODO: [ ] Reproduce the inconsistent already-linked branch only with a proven original ownership contract; it skips the
/// balancing self release.
pub struct ParticleControllerChain;

impl ParticleControllerChain {
    // IDA: 00715CE0 void __thiscall NiTimeController::SetTarget(NiTimeController* self, NiObjectNET* target)
    // Completion: 75% of the well-formed chain, original branch and store order
    // TODO: [ ] Resolve the concrete +0x60 virtual for every controller class, and non-particle NiGeometryData at target +0xB4.
    // TODO: [ ] Keep the original already-present self-hold leak only after identifying a reachable owner state.
    pub fn set_target(
        world: &mut World,
        controller: Option<Entity>,
        target: Option<Entity>,
    ) -> ParticleControllerSetTargetResult {
        use ParticleControllerSetTargetStage as Stage;
        use ParticleControllerSetTargetStatus as Status;
        let result = |status, stage| ParticleControllerSetTargetResult { status, stage };

        let Some(time) = controller.and_then(|controller| controller_time(world, controller))
        else {
            return result(Status::UnsupportedNullController, Stage::Unchanged);
        };
        let controller = controller.expect("the time part came from this entity");
        let has_target = time.target_linked;
        let old_target = time
            .target_30
            .filter(|&old| world.get::<NiObjectNet>(old).is_some());
        if has_target && old_target.is_none() {
            return result(Status::UnsupportedExpiredOldTarget, Stage::Unchanged);
        }

        // 00715CE8-00715CEB: the same pointer exits before the self refcount increment.
        if old_target == target {
            return result(Status::SameTarget, Stage::Unchanged);
        }

        // The source traverses raw intrusive links without cycle protection. The host preflight leaves unsupported malformed
        // graphs unchanged.
        if let Some(old) = old_target {
            let old_chain = inspect_chain(world, head_of(world, old), controller);
            if old_chain.has_cycle || !old_chain.contains_controller {
                return result(Status::UnsupportedOldChain, Stage::Unchanged);
            }
        }
        if let Some(new) = target {
            if world.get::<NiObjectNet>(new).is_none() {
                return result(Status::UnsupportedNewChain, Stage::Unchanged);
            }
            let new_chain = inspect_chain(world, head_of(world, new), controller);
            if new_chain.has_cycle {
                return result(Status::UnsupportedNewChain, Stage::Unchanged);
            }
            if new_chain.contains_controller {
                // 00715D20-00715D22: the original jumps past its balancing decrement in this inconsistent state. Do not fabricate
                // a normalized ownership result in the host.
                return result(Status::UnsupportedAlreadyInNewChain, Stage::Unchanged);
            }
            if world.get::<MarksGeometryDirty60>(controller).is_none() {
                return result(Status::UnsupportedMissingVirtualPredicate, Stage::Unchanged);
            }
        }

        // 00715CFC-00715D0A: remove only when the old target has a controller head.
        if let Some(old) = old_target
            && head_of(world, old).is_some()
        {
            unlink_controller(world, old, controller);
        }

        // 00715D0F-00715D11: store the non-owning new target pointer.
        {
            let mut time = controller_time_mut(world, controller)
                .expect("the time part came from this entity");
            time.target_30 = target;
            time.target_linked = target.is_some();
        }

        let Some(new) = target else {
            return result(Status::Detached, Stage::Complete);
        };

        // 00715D16-00715D2E: the checked new chain is absent, so prepend.
        prepend_controller(world, new, controller);

        // 00715D33-00715D3E: derived vtable +0x60 is evaluated after attach.
        if world
            .get::<MarksGeometryDirty60>(controller)
            .expect("the preflight read this predicate")
            .0
        {
            // 00715D40-00715D57 assumes NiGeometryData at target +0xB4. An absent typed geometry owner stays an explicit boundary.
            let data = world
                .get::<ParticleSystem>(new)
                .and_then(|system| system.data_b4)
                .filter(|&data| world.get::<ParticleData>(data).is_some());
            let Some(data) = data else {
                return result(
                    Status::UnsupportedMissingGeometryDataAfterAttach,
                    Stage::NewHeadAttached,
                );
            };
            let mut data = world
                .get_mut::<ParticleData>(data)
                .expect("the filter found this data");
            data.geometry_extra_flags_2e = (data.geometry_extra_flags_2e & 0x0FFF) | 0x8000;
        }

        result(Status::Attached, Stage::Complete)
    }
}

#[cfg(test)]
mod tests {
    use super::super::particle_controller::ParticleUpdateController;
    use super::super::particle_system::spawn_particle_system;
    use super::super::stream_objects::NiClass;
    use super::*;

    fn controller(world: &mut World, marks: bool) -> Entity {
        world
            .spawn((
                NiClass::ParticleUpdateController,
                ParticleUpdateController::default(),
                MarksGeometryDirty60(marks),
            ))
            .id()
    }

    /// Ported from `particle_controller_tests`' `verify_chain_head_and_interior_splice`. The fixtures are real controllers and
    /// systems rather than the C++ link nodes; the checks are unchanged.
    #[test]
    fn chain_head_and_interior_splice() {
        use ParticleControllerSetTargetStatus as Status;
        let mut world = World::new();
        let first_target = spawn_particle_system(&mut world);
        let second_target = spawn_particle_system(&mut world);
        let data = world
            .spawn(ParticleData {
                geometry_extra_flags_2e: 0x1FFF,
                ..ParticleData::default()
            })
            .id();
        world
            .get_mut::<ParticleSystem>(first_target)
            .unwrap()
            .data_b4 = Some(data);
        let first = controller(&mut world, true);
        let second = controller(&mut world, false);
        let attach_first =
            ParticleControllerChain::set_target(&mut world, Some(first), Some(first_target));
        let attach_second =
            ParticleControllerChain::set_target(&mut world, Some(second), Some(first_target));
        assert_eq!(attach_first.status, Status::Attached);
        assert_eq!(attach_second.status, Status::Attached);
        assert_eq!(head_of(&world, first_target), Some(second));
        assert_eq!(
            controller_time(&world, second).unwrap().next_34,
            Some(first)
        );
        assert_eq!(
            world
                .get::<ParticleData>(data)
                .unwrap()
                .geometry_extra_flags_2e,
            0x8FFF
        );

        let same = ParticleControllerChain::set_target(&mut world, Some(first), Some(first_target));
        assert_eq!(same.status, Status::SameTarget);
        assert_eq!(same.stage, ParticleControllerSetTargetStage::Unchanged);
        assert_eq!(
            controller_time(&world, second).unwrap().next_34,
            Some(first)
        );

        let move_interior =
            ParticleControllerChain::set_target(&mut world, Some(first), Some(second_target));
        assert_eq!(
            move_interior.status,
            Status::UnsupportedMissingGeometryDataAfterAttach
        );
        assert_eq!(
            move_interior.stage,
            ParticleControllerSetTargetStage::NewHeadAttached
        );
        assert_eq!(head_of(&world, first_target), Some(second));
        assert_eq!(controller_time(&world, second).unwrap().next_34, None);
        assert_eq!(head_of(&world, second_target), Some(first));

        let detach_head = ParticleControllerChain::set_target(&mut world, Some(second), None);
        assert_eq!(detach_head.status, Status::Detached);
        assert_eq!(head_of(&world, first_target), None);
        assert_eq!(controller_time(&world, second).unwrap().next_34, None);
        assert!(!controller_time(&world, second).unwrap().target_linked);
    }

    /// IDA: N/A the host preflight's stops, which leave everything unchanged.
    #[test]
    fn preflight_stops_change_nothing() {
        use ParticleControllerSetTargetStatus as Status;
        let mut world = World::new();
        assert_eq!(
            ParticleControllerChain::set_target(&mut world, None, None).status,
            Status::UnsupportedNullController
        );
        let target = spawn_particle_system(&mut world);
        let unmarked = world
            .spawn((
                NiClass::ParticleUpdateController,
                ParticleUpdateController::default(),
            ))
            .id();
        assert_eq!(
            ParticleControllerChain::set_target(&mut world, Some(unmarked), Some(target)).status,
            Status::UnsupportedMissingVirtualPredicate
        );
        assert_eq!(head_of(&world, target), None);

        let looped = controller(&mut world, false);
        controller_time_mut(&mut world, looped).unwrap().next_34 = Some(looped);
        world.get_mut::<NiObjectNet>(target).unwrap().controller_0c = Some(looped);
        let other = controller(&mut world, false);
        assert_eq!(
            ParticleControllerChain::set_target(&mut world, Some(other), Some(target)).status,
            Status::UnsupportedNewChain
        );
        controller_time_mut(&mut world, looped).unwrap().next_34 = None;
        assert_eq!(
            ParticleControllerChain::set_target(&mut world, Some(looped), Some(target)).status,
            Status::UnsupportedAlreadyInNewChain
        );

        let stale = controller(&mut world, false);
        controller_time_mut(&mut world, stale)
            .unwrap()
            .target_linked = true;
        assert_eq!(
            ParticleControllerChain::set_target(&mut world, Some(stale), Some(target)).status,
            Status::UnsupportedExpiredOldTarget
        );
    }
}
