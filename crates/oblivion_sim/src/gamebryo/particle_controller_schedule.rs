//! The ordered walk over a NiParticleSystem's controller list.

#![allow(
    clippy::type_complexity,
    reason = "the C++ port's injected std::function services, which stand for 007155A0 and the original target virtuals"
)]

use std::collections::HashSet;

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::controller_time::{
    ControllerTimeFields, ParticleControllerTimeCache, evaluate_particle_controller_time,
};
use super::particle_controller::{
    ParticleEmitterController, ParticleEmitterControllerRuntime, ParticleEmitterUpdateStatus,
    ParticleUpdateController, ParticleUpdateControllerRuntime, ParticleUpdateEmitterTiming,
    controller_time, with_detached,
};
use super::particle_emitter::{ParticleBoxEmitter, ParticleCylinderEmitter, ParticleSphereEmitter};
use super::particle_mesh_emitter::ParticleMeshEmitter;
use super::particle_modifier_value_controller::{
    ParticleGravityStrengthController, ParticleGravityStrengthRuntime,
    ParticleModifierActiveController, ParticleModifierActiveRuntime,
    ParticleModifierValueUpdateStatus,
};
use super::particle_reset_controller::{
    ParticleResetControllerRuntime, ParticleResetOnLoopController, ParticleResetUpdateStatus,
};
use super::particle_system::{ParticleSystem, set_modifier_active_byte};
use super::stream_objects::NiClass;

const NEGATIVE_MAX: f32 = -f32::MAX;

/// IDA: 00753BE0 target virtual +0x98, 00750260 emission continuation. Explicit host service boundaries. The one shared 007155A0
/// cache is the `ParticleControllerTimeCache` resource, not a field here, so every controller in the process reuses it.
/// TODO: [ ] Bind the original target virtual and NiPSysEmitter +0x5C owner.
#[derive(Default)]
pub struct ParticleControllerScheduleContext<'a> {
    pub update_system: Option<&'a mut dyn FnMut(&mut World, Entity, f32)>,
    /// IDA: 00754F0C writes target +0xEC only after a strict time rewind. An optional override; the typed system's request is the
    /// default.
    pub request_target_reset: Option<&'a mut dyn FnMut(&mut World, Entity)>,
    /// The emitter controller's entity, its detached state, the emission time, the ages and the target.
    pub emit: Option<
        &'a mut dyn FnMut(&mut World, Entity, &ParticleEmitterController, f32, &[f32], Entity),
    >,
    /// IDA: 00756710 reads modifier +0x14 and calls its +0x58 virtual only when the byte differs.
    /// TODO: [ ] Bind each original modifier SetActive target.
    pub read_modifier_active: Option<&'a mut dyn FnMut(&World, Entity, Entity) -> Option<u8>>,
    pub write_modifier_active: Option<&'a mut dyn FnMut(&mut World, Entity, Entity, u8)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleControllerScheduleStatus {
    Completed,
    UnsupportedCycle,
    UnsupportedNullOwner,
    UnsupportedControllerType,
    UnsupportedTargetMismatch,
    UnsupportedCachedEmitter,
    UnsupportedEmitterType,
    UnsupportedSystemUpdateService,
    UnsupportedEmitService,
    UnsupportedEmitterBranch,
    UnsupportedResetService,
    UnsupportedResetTimeService,
    UnsupportedModifierValueBranch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleControllerScheduleResult {
    pub status: ParticleControllerScheduleStatus,
    pub visited: usize,
    pub last_emitter_status: Option<ParticleEmitterUpdateStatus>,
    pub last_reset_status: Option<ParticleResetUpdateStatus>,
    pub last_modifier_value_status: Option<ParticleModifierValueUpdateStatus>,
}

enum LifespanResult {
    Found(f32),
    Missing,
    UnsupportedType,
}

// IDA: 00753C1B-00753C4A scans target NiObjectNET +0x0C, follows each NiTimeController +0x34, and stops at the first NiPSysEmitterCtlr
// RTTI hit
// Completion: 100% of the ordered typed selection across the bound classes. `Err` is the C++ `malformed`
// TODO: [ ] Follow inherited RTTI ancestry for future derived controller owners.
fn first_emitter_controller(world: &World, head: Option<Entity>) -> Result<Option<Entity>, ()> {
    let mut cursor = head;
    let mut visited = HashSet::new();
    while let Some(entity) = cursor {
        if world.get_entity(entity).is_err() || !visited.insert(entity) {
            return Err(());
        }
        if world.get::<NiClass>(entity) == Some(&NiClass::ParticleEmitterController) {
            return Ok(Some(entity));
        }
        cursor = controller_time(world, entity).and_then(|time| time.next_34);
    }
    Ok(None)
}

// IDA: 00753C47 reads the first NiPSysEmitterCtlr's +0x44 cached modifier, then 00753C7D reads that NiPSysEmitter's +0x48 lifespan;
// 007502CA reads the calling NiPSysEmitterCtlr's own cached +0x44 emitter +0x48
// Completion: 85% of the checked typed Box, Sphere, Cylinder and Mesh identity and inherited value
// TODO: [ ] The original modifier-map raw pointer and remaining emitters.
fn resolve_cached_emitter_lifespan(
    world: &World,
    controller: &ParticleEmitterController,
    target: &ParticleSystem,
) -> LifespanResult {
    let (Some(name), Some(cached)) = (
        controller.modifier_name_40.as_deref(),
        controller.cached_modifier_44,
    ) else {
        return LifespanResult::Missing;
    };
    let Some(owner) = target.find_modifier(name) else {
        return LifespanResult::Missing;
    };
    if owner != cached {
        return LifespanResult::Missing;
    }
    let lifespan = match world.get::<NiClass>(owner) {
        Some(NiClass::ParticleBoxEmitter) => world
            .get::<ParticleBoxEmitter>(owner)
            .map(|emitter| emitter.state.lifespan),
        Some(NiClass::ParticleSphereEmitter) => world
            .get::<ParticleSphereEmitter>(owner)
            .map(|emitter| emitter.state.lifespan),
        Some(NiClass::ParticleCylinderEmitter) => world
            .get::<ParticleCylinderEmitter>(owner)
            .map(|emitter| emitter.state.lifespan),
        Some(NiClass::ParticleMeshEmitter) => world
            .get::<ParticleMeshEmitter>(owner)
            .map(|emitter| emitter.state.lifespan),
        _ => return LifespanResult::UnsupportedType,
    };
    lifespan.map_or(LifespanResult::UnsupportedType, LifespanResult::Found)
}

// IDA: N/A a helper, with no address of its own.
const fn lifespan_failure(result: &LifespanResult) -> ParticleControllerScheduleStatus {
    match result {
        LifespanResult::UnsupportedType => ParticleControllerScheduleStatus::UnsupportedEmitterType,
        _ => ParticleControllerScheduleStatus::UnsupportedCachedEmitter,
    }
}

// IDA: N/A a helper, with no address of its own.
const fn modifier_value_status_continues(status: ParticleModifierValueUpdateStatus) -> bool {
    matches!(
        status,
        ParticleModifierValueUpdateStatus::Skipped
            | ParticleModifierValueUpdateStatus::SampleUnavailable
            | ParticleModifierValueUpdateStatus::Unchanged
            | ParticleModifierValueUpdateStatus::Applied
    )
}

/// IDA: 00753BE0 first-emitter scan through NiObjectNET +0x0C head and NiTimeController +0x34 next, then 00750260 / 00753BE0 controller
/// updates. Completion: 80% of the typed ordered traversal with one shared 007155A0 cache.
/// TODO: [ ] The original top-level AVObject controller update caller and order.
pub struct ParticleControllerScheduler;

impl ParticleControllerScheduler {
    // IDA: 00753BE0 first-emitter scan and Update target call; 00750260 emitter update; 007155A0 shared mapped-time cache
    // Completion: 80% of the ordered typed dispatch from the supplied head
    // TODO: [ ] Bind the head directly from `NiObjectNet`, and the actual top-level caller and order.
    pub fn run(
        world: &mut World,
        object_net_head: Option<Entity>,
        target: Entity,
        current_time: f32,
        context: &mut ParticleControllerScheduleContext<'_>,
    ) -> Result<ParticleControllerScheduleResult, GameError> {
        if world.get::<ParticleSystem>(target).is_none() {
            return Err(GameError::unclassified(
                0,
                "controller schedule target is not a NiParticleSystem",
            ));
        }
        world.init_resource::<ParticleControllerTimeCache>();
        world.resource_scope(|world, mut cache: Mut<ParticleControllerTimeCache>| {
            Self::run_with_cache(
                world,
                &mut cache,
                object_net_head,
                target,
                current_time,
                context,
            )
        })
    }

    // IDA: N/A no address of its own; its comments cite 007509DF.
    fn run_with_cache(
        world: &mut World,
        cache: &mut ParticleControllerTimeCache,
        object_net_head: Option<Entity>,
        target: Entity,
        current_time: f32,
        context: &mut ParticleControllerScheduleContext<'_>,
    ) -> Result<ParticleControllerScheduleResult, GameError> {
        use ParticleControllerScheduleStatus as Status;
        let mut result = ParticleControllerScheduleResult {
            status: Status::Completed,
            visited: 0,
            last_emitter_status: None,
            last_reset_status: None,
            last_modifier_value_status: None,
        };
        let mut evaluate = |fields: &mut ControllerTimeFields, sample_time: f32| {
            evaluate_particle_controller_time(fields, sample_time, cache)
        };
        let mut cursor = object_net_head;
        let mut visited = HashSet::new();

        while let Some(entity) = cursor {
            if world.get_entity(entity).is_err() {
                result.status = Status::UnsupportedNullOwner;
                return Ok(result);
            }
            if !visited.insert(entity) {
                result.status = Status::UnsupportedCycle;
                return Ok(result);
            }
            result.visited += 1;

            match world.get::<NiClass>(entity).copied() {
                Some(NiClass::ParticleUpdateController) => {
                    let time = &world
                        .get::<ParticleUpdateController>(entity)
                        .ok_or(GameError::unclassified(
                            0,
                            "particle controller component missing",
                        ))?
                        .time;
                    if !time.target_linked || time.target_30 != Some(target) {
                        result.status = Status::UnsupportedTargetMismatch;
                        return Ok(result);
                    }
                    let last_time = time.fields.last_time;
                    let Ok(first_emitter) = first_emitter_controller(world, object_net_head) else {
                        result.status = Status::UnsupportedCycle;
                        return Ok(result);
                    };
                    let mut timing = None;
                    if let Some(emitter) = first_emitter {
                        let emitter = world.get::<ParticleEmitterController>(emitter).ok_or(
                            GameError::unclassified(0, "particle controller component missing"),
                        )?;
                        let mut first = ParticleUpdateEmitterTiming {
                            controller_flags: emitter.time.fields.flags,
                            emitter_lifespan: 0.0,
                        };
                        if last_time != NEGATIVE_MAX && first.controller_flags & 0x0006 == 0 {
                            let system = world
                                .get::<ParticleSystem>(target)
                                .expect("checked at entry");
                            match resolve_cached_emitter_lifespan(world, emitter, system) {
                                LifespanResult::Found(value) => first.emitter_lifespan = value,
                                failure => {
                                    result.status = lifespan_failure(&failure);
                                    return Ok(result);
                                }
                            }
                        }
                        timing = Some(first);
                    }
                    let mut service_missing = false;
                    let outcome = with_detached::<ParticleUpdateController, _>(
                        world,
                        entity,
                        |world, state| {
                            let mut update_target = |mapped: f32| {
                                let Some(update) = context.update_system.as_mut() else {
                                    service_missing = true;
                                    return Err(GameError::unclassified(
                                        0,
                                        "particle system update service unresolved",
                                    ));
                                };
                                update(world, target, mapped);
                                Ok(())
                            };
                            let mut runtime = ParticleUpdateControllerRuntime {
                                first_emitter: timing,
                                evaluate_controller_time: Some(&mut evaluate),
                                update_target_system: Some(&mut update_target),
                            };
                            state.update(current_time, &mut runtime)
                        },
                    )
                    .expect("read above");
                    if service_missing {
                        result.status = Status::UnsupportedSystemUpdateService;
                        return Ok(result);
                    }
                    outcome?;
                }
                Some(NiClass::ParticleEmitterController) => {
                    let emitter = world.get::<ParticleEmitterController>(entity).ok_or(
                        GameError::unclassified(0, "particle controller component missing"),
                    )?;
                    if emitter.time.target_linked && emitter.time.target_30 != Some(target) {
                        result.status = Status::UnsupportedTargetMismatch;
                        return Ok(result);
                    }
                    let system = world
                        .get::<ParticleSystem>(target)
                        .expect("checked at entry");
                    let mut emitter_lifespan = 0.0;
                    if emitter.time.target_linked && emitter.cached_modifier_linked {
                        match resolve_cached_emitter_lifespan(world, emitter, system) {
                            LifespanResult::Found(value) => emitter_lifespan = value,
                            failure => {
                                result.status = lifespan_failure(&failure);
                                return Ok(result);
                            }
                        }
                    }
                    let target_system_update_time = system.system_update_time_e8;
                    // IDA: 007509DF reads +0xE8 after each rate virtual call: a typed live read at the original branch point.
                    let mut read_target = |world: &World| {
                        world
                            .get::<ParticleSystem>(target)
                            .expect("checked at entry")
                            .system_update_time_e8
                    };
                    let mut emit_missing = false;
                    let services = &mut *context;
                    let mut emit = |world: &mut World,
                                    controller: &ParticleEmitterController,
                                    time: f32,
                                    ages: &[f32]| {
                        let Some(emit) = services.emit.as_mut() else {
                            emit_missing = true;
                            return Err(GameError::unclassified(
                                0,
                                "particle emit service unresolved",
                            ));
                        };
                        emit(world, entity, controller, time, ages, target);
                        Ok(())
                    };
                    let mut runtime = ParticleEmitterControllerRuntime {
                        evaluate_controller_time: Some(&mut evaluate),
                        emitter_lifespan,
                        target_system_update_time,
                        emit: Some(&mut emit),
                        read_target_system_update_time: Some(&mut read_target),
                    };
                    let outcome = with_detached::<ParticleEmitterController, _>(
                        world,
                        entity,
                        |world, state| {
                            state.update_managed_non_blend(world, current_time, &mut runtime)
                        },
                    )
                    .expect("read above");
                    if emit_missing {
                        result.status = Status::UnsupportedEmitService;
                        return Ok(result);
                    }
                    let status = outcome?;
                    result.last_emitter_status = Some(status);
                    if status == ParticleEmitterUpdateStatus::UnsupportedTimeline {
                        result.status = Status::UnsupportedEmitterBranch;
                        return Ok(result);
                    }
                }
                Some(NiClass::ParticleResetOnLoopController) => {
                    // IDA: 00754EC0 clears flags & ~6 before its target and gate check; the typed target identity guard is a host
                    // preflight boundary.
                    let time = &world
                        .get::<ParticleResetOnLoopController>(entity)
                        .ok_or(GameError::unclassified(
                            0,
                            "particle controller component missing",
                        ))?
                        .time;
                    if time.target_linked && time.target_30 != Some(target) {
                        result.status = Status::UnsupportedTargetMismatch;
                        return Ok(result);
                    }
                    // IDA: 00754F0C writes target +0xEC only on a strict rewind. The typed default invokes the byte-equivalent setter
                    // at that same stage; an explicit service may replace the adapter.
                    let services = &mut *context;
                    let mut request = |world: &mut World, _: &ParticleResetOnLoopController| {
                        if let Some(request) = services.request_target_reset.as_mut() {
                            request(world, target);
                        } else if let Some(mut system) = world.get_mut::<ParticleSystem>(target) {
                            system.request_reset();
                        }
                    };
                    let mut runtime = ParticleResetControllerRuntime {
                        evaluate_controller_time: Some(&mut evaluate),
                        request_target_reset: Some(&mut request),
                    };
                    let status = with_detached::<ParticleResetOnLoopController, _>(
                        world,
                        entity,
                        |world, state| state.update(world, current_time, &mut runtime),
                    )
                    .expect("read above");
                    result.last_reset_status = Some(status);
                    // The typed target always supplies this callback; the direct Reset adapter failure stays a defensive frontier.
                    let stop = match status {
                        ParticleResetUpdateStatus::UnsupportedResetService => {
                            Some(Status::UnsupportedResetService)
                        }
                        ParticleResetUpdateStatus::UnsupportedTimeEvaluator => {
                            Some(Status::UnsupportedResetTimeService)
                        }
                        ParticleResetUpdateStatus::UnsupportedExpiredTarget => {
                            Some(Status::UnsupportedTargetMismatch)
                        }
                        _ => None,
                    };
                    if let Some(stop) = stop {
                        result.status = stop;
                        return Ok(result);
                    }
                }
                Some(NiClass::ParticleModifierActiveController) => {
                    let time = &world
                        .get::<ParticleModifierActiveController>(entity)
                        .ok_or(GameError::unclassified(
                            0,
                            "particle controller component missing",
                        ))?
                        .base
                        .time;
                    if time.target_linked && time.target_30 != Some(target) {
                        result.status = Status::UnsupportedTargetMismatch;
                        return Ok(result);
                    }
                    let services = &mut *context;
                    let mut read = |world: &World, identity: Entity| {
                        if let Some(read) = services.read_modifier_active.as_mut() {
                            read(world, target, identity)
                        } else {
                            world
                                .get::<ParticleSystem>(target)
                                .expect("checked at entry")
                                .modifier_active_byte(world, identity)
                        }
                    };
                    let mut write = |world: &mut World, identity: Entity, value: u8| {
                        if let Some(write) = services.write_modifier_active.as_mut() {
                            write(world, target, identity, value);
                            Ok(true)
                        } else {
                            set_modifier_active_byte(world, target, identity, value)
                        }
                    };
                    let mut runtime = ParticleModifierActiveRuntime {
                        evaluate_controller_time: Some(&mut evaluate),
                        read_active: Some(&mut read),
                        write_active: Some(&mut write),
                    };
                    let status = with_detached::<ParticleModifierActiveController, _>(
                        world,
                        entity,
                        |world, state| state.update(world, current_time, &mut runtime),
                    )
                    .expect("read above")?;
                    result.last_modifier_value_status = Some(status);
                    if !modifier_value_status_continues(status) {
                        result.status = Status::UnsupportedModifierValueBranch;
                        return Ok(result);
                    }
                }
                Some(NiClass::ParticleGravityStrengthController) => {
                    let time = &world
                        .get::<ParticleGravityStrengthController>(entity)
                        .ok_or(GameError::unclassified(
                            0,
                            "particle controller component missing",
                        ))?
                        .base
                        .time;
                    if time.target_linked && time.target_30 != Some(target) {
                        result.status = Status::UnsupportedTargetMismatch;
                        return Ok(result);
                    }
                    let mut runtime = ParticleGravityStrengthRuntime {
                        evaluate_controller_time: Some(&mut evaluate),
                    };
                    let status = with_detached::<ParticleGravityStrengthController, _>(
                        world,
                        entity,
                        |world, state| state.update(world, current_time, &mut runtime),
                    )
                    .expect("read above")?;
                    result.last_modifier_value_status = Some(status);
                    if !modifier_value_status_continues(status) {
                        result.status = Status::UnsupportedModifierValueBranch;
                        return Ok(result);
                    }
                }
                _ => {
                    result.status = Status::UnsupportedControllerType;
                    return Ok(result);
                }
            }
            cursor = controller_time(world, entity).and_then(|time| time.next_34);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::super::animation::FloatInterpolator;
    use super::super::bool_animation::BoolInterpolator;
    use super::super::particle_controller::controller_time_mut;
    use super::super::particle_mesh_emitter::{
        ParticleMeshEmitterPending, resolve_particle_mesh_emitter_link,
    };
    use super::super::particle_system::{attach_particle_modifier, spawn_particle_system};
    use super::*;

    fn mesh_emitter(world: &mut World) -> Entity {
        let mut mesh = ParticleMeshEmitter::default();
        let mut fields = ParticleMeshEmitterPending::default();
        fields.base.base.name = Some(b"Mesh".to_vec());
        fields.base.lifespan = 5.0;
        resolve_particle_mesh_emitter_link(&mut mesh, &fields, 0).unwrap();
        world.spawn((NiClass::ParticleMeshEmitter, mesh)).id()
    }

    /// Ported from `particle_controller_schedule_tests`' `verify_head_next_lifespan_and_services`.
    #[test]
    fn head_next_lifespan_and_services() {
        use ParticleControllerScheduleStatus as Status;
        let mut world = World::new();
        let system = spawn_particle_system(&mut world);
        let mut emitter = ParticleEmitterController::default();
        emitter.time.target_linked = true;
        emitter.time.target_30 = Some(system);
        emitter.time.fields.flags = 8;
        emitter.modifier_name_40 = Some(b"Mesh".to_vec());
        let emitter = world
            .spawn((NiClass::ParticleEmitterController, emitter))
            .id();
        let mut update = ParticleUpdateController::default();
        update.time.target_linked = true;
        update.time.target_30 = Some(system);
        update.time.next_34 = Some(emitter);
        update.time.fields.last_time = 0.0;
        let update = world
            .spawn((NiClass::ParticleUpdateController, update))
            .id();

        let mut empty = ParticleControllerScheduleContext::default();
        let missing_cache =
            ParticleControllerScheduler::run(&mut world, Some(update), system, 1.0, &mut empty)
                .unwrap();
        assert_eq!(missing_cache.status, Status::UnsupportedCachedEmitter);
        assert_eq!(missing_cache.visited, 1);

        let mesh = mesh_emitter(&mut world);
        attach_particle_modifier(&mut world, system, mesh).unwrap();
        with_detached::<ParticleEmitterController, _>(&mut world, emitter, |world, state| {
            state.cache_modifier_from_target(world, Some(system));
        });
        assert_eq!(
            world
                .get::<ParticleEmitterController>(emitter)
                .unwrap()
                .cached_modifier_44,
            Some(mesh)
        );
        let missing_update =
            ParticleControllerScheduler::run(&mut world, Some(update), system, 1.0, &mut empty)
                .unwrap();
        assert_eq!(
            missing_update.status,
            Status::UnsupportedSystemUpdateService
        );
        assert_eq!(missing_update.visited, 1);

        let mut updates = 0;
        let mut mapped = 0.0;
        let mut update_system = |_: &mut World, dispatched: Entity, time: f32| {
            assert_eq!(dispatched, system, "dispatched to the wrong target");
            updates += 1;
            mapped = time;
        };
        let mut context = ParticleControllerScheduleContext {
            update_system: Some(&mut update_system),
            ..ParticleControllerScheduleContext::default()
        };
        let completed =
            ParticleControllerScheduler::run(&mut world, Some(update), system, 2.0, &mut context)
                .unwrap();
        assert_eq!(completed.status, Status::Completed);
        assert_eq!(completed.visited, 2);
        assert_eq!(updates, 1);
        assert_eq!(
            mapped,
            world
                .get::<ParticleUpdateController>(update)
                .unwrap()
                .time
                .last_value_28
        );
        assert_eq!(
            completed.last_emitter_status,
            Some(ParticleEmitterUpdateStatus::Skipped)
        );
        assert_ne!(
            world.resource::<ParticleControllerTimeCache>().input,
            NEGATIVE_MAX
        );

        controller_time_mut(&mut world, update).unwrap().next_34 = Some(update);
        let mut empty = ParticleControllerScheduleContext::default();
        let cycle =
            ParticleControllerScheduler::run(&mut world, Some(update), system, 3.0, &mut empty)
                .unwrap();
        assert_eq!(cycle.status, Status::UnsupportedCycle);
    }

    /// Ported from `particle_controller_schedule_tests`' `verify_missing_service_only_on_invocation`.
    #[test]
    fn missing_service_only_on_invocation() {
        let mut world = World::new();
        let system = spawn_particle_system(&mut world);
        let mut update = ParticleUpdateController::default();
        update.time.target_linked = true;
        update.time.target_30 = Some(system);
        update.time.fields.flags = 0; // inactive gate, no target virtual call
        let update = world
            .spawn((NiClass::ParticleUpdateController, update))
            .id();
        let mut context = ParticleControllerScheduleContext::default();
        let skipped =
            ParticleControllerScheduler::run(&mut world, Some(update), system, 1.0, &mut context)
                .unwrap();
        assert_eq!(skipped.status, ParticleControllerScheduleStatus::Completed);
        assert_eq!(skipped.visited, 1);
    }

    /// Ported from `particle_controller_schedule_tests`' `verify_emit_service_is_lazy`.
    #[test]
    fn emit_service_is_lazy() {
        let mut world = World::new();
        let system = spawn_particle_system(&mut world);
        let mesh = mesh_emitter(&mut world);
        attach_particle_modifier(&mut world, system, mesh).unwrap();
        let mut rate = FloatInterpolator::default();
        rate.load_scalar_and_data(5.0, None);
        let rate = world.spawn((NiClass::FloatInterpolator, rate)).id();
        let mut active = BoolInterpolator::default();
        active.load_scalar_and_data(1, None);
        let active = world.spawn((NiClass::BoolInterpolator, active)).id();
        let mut emitter = ParticleEmitterController {
            modifier_name_40: Some(b"Mesh".to_vec()),
            ..ParticleEmitterController::default()
        };
        emitter.cache_modifier_from_target(&world, Some(system));
        emitter.time.fields.flags = 0x0A;
        emitter.time.fields.low_key_time = 0.0;
        emitter.time.fields.high_key_time = 10.0;
        emitter
            .bind_interpolators(Some(rate), Some(active))
            .unwrap();
        let emitter = world
            .spawn((NiClass::ParticleEmitterController, emitter))
            .id();

        let mut empty = ParticleControllerScheduleContext::default();
        let missing =
            ParticleControllerScheduler::run(&mut world, Some(emitter), system, 1.0, &mut empty)
                .unwrap();
        assert_eq!(
            missing.status,
            ParticleControllerScheduleStatus::UnsupportedEmitService
        );
        assert_eq!(missing.visited, 1);

        let mut callbacks = 0;
        let mut emit = |_: &mut World,
                        caller: Entity,
                        _: &ParticleEmitterController,
                        _: f32,
                        ages: &[f32],
                        dispatched: Entity| {
            assert_eq!(caller, emitter);
            assert_eq!(dispatched, system);
            assert!(!ages.is_empty());
            callbacks += 1;
        };
        let mut context = ParticleControllerScheduleContext {
            emit: Some(&mut emit),
            ..ParticleControllerScheduleContext::default()
        };
        let completed =
            ParticleControllerScheduler::run(&mut world, Some(emitter), system, 2.0, &mut context)
                .unwrap();
        assert_eq!(
            completed.status,
            ParticleControllerScheduleStatus::Completed
        );
        assert_eq!(
            completed.last_emitter_status,
            Some(ParticleEmitterUpdateStatus::ConstantEmitted)
        );
        assert_eq!(callbacks, 1);
    }

    /// IDA: 00754F0C through the scheduler's typed default, and the null owner and class stops. The C++ scheduler tests do not reach
    /// these.
    #[test]
    fn reset_default_and_stops() {
        use ParticleControllerScheduleStatus as Status;
        let mut world = World::new();
        let system = spawn_particle_system(&mut world);
        let reset =
            super::super::particle_reset_controller::spawn_particle_reset_controller(&mut world);
        {
            let mut state = world
                .get_mut::<ParticleResetOnLoopController>(reset)
                .unwrap();
            state.time.target_linked = true;
            state.time.target_30 = Some(system);
            state.time.fields.flags = 0x08;
            // The cold key times clamp every mapped time to -FLT_MAX, which the gate would skip.
            state.time.fields.low_key_time = 0.0;
            state.time.fields.high_key_time = 10.0;
            state.previous_mapped_time_3c = 100.0;
        }
        let mut context = ParticleControllerScheduleContext::default();
        let done =
            ParticleControllerScheduler::run(&mut world, Some(reset), system, 1.0, &mut context)
                .unwrap();
        assert_eq!(done.status, Status::Completed);
        assert_eq!(
            done.last_reset_status,
            Some(ParticleResetUpdateStatus::ResetRequested)
        );
        assert!(
            world
                .get::<ParticleSystem>(system)
                .unwrap()
                .reset_requested_ec
        );

        let dangling = world.spawn_empty().id();
        world.despawn(dangling);
        assert_eq!(
            ParticleControllerScheduler::run(&mut world, Some(dangling), system, 1.0, &mut context)
                .unwrap()
                .status,
            Status::UnsupportedNullOwner
        );
        let data = world.spawn(NiClass::FloatKeyData).id();
        assert_eq!(
            ParticleControllerScheduler::run(&mut world, Some(data), system, 1.0, &mut context)
                .unwrap()
                .status,
            Status::UnsupportedControllerType
        );
    }
}
