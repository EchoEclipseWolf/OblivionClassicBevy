//! NiPSysResetOnLoopCtlr.

#![allow(
    clippy::type_complexity,
    reason = "the C++ port's injected std::function services, which stand for 007155A0 and the original target virtuals"
)]

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::controller_time::ControllerTimeFields;
use super::particle_controller::{
    ParticleTimeControllerCold, ParticleTimeControllerPayload, TimeGate, evaluate_time_gate,
    link_particle_time_controller_modern, parse_particle_time_controller_payload,
};
use super::stream_objects::NiClass;

const NEGATIVE_MAX: f32 = -f32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleResetUpdateStatus {
    NoTarget,
    SkippedByTimeGate,
    SampledFirstTime,
    AdvancedOrUnchanged,
    ResetRequested,
    UnsupportedExpiredTarget,
    UnsupportedTimeEvaluator,
    UnsupportedResetService,
}

/// IDA: N/A 00754EC0's explicit services. `request_target_reset` receives the world and the controller as they stand at
/// 00754F0C's byte store, before the final +0x3C store.
#[derive(Default)]
pub struct ParticleResetControllerRuntime<'a> {
    pub evaluate_controller_time: Option<&'a mut dyn FnMut(&mut ControllerTimeFields, f32) -> f32>,
    pub request_target_reset: Option<&'a mut dyn FnMut(&mut World, &ParticleResetOnLoopController)>,
}

/// IDA: 00754E40 NiPSysResetOnLoopCtlr factory, 00754EC0 Update, inherited 00715F40 LoadBinary, 00715E70 LinkObject and 00715CE0
/// SetTarget. Completion: 80% of the cold state, modern payload and link, well-formed chain and rewind decision.
/// TODO: [ ] Original FormHeap and refcounts, and x87 mapped-time behaviour externally.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ParticleResetOnLoopController {
    pub time: ParticleTimeControllerCold,
    pub previous_mapped_time_3c: f32,
}

impl Default for ParticleResetOnLoopController {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            time: ParticleTimeControllerCold::default(),
            previous_mapped_time_3c: NEGATIVE_MAX,
        }
    }
}

// IDA: 00754E40 NiPSysResetOnLoopCtlr::CreateObject allocates 0x40, invokes 00715990, and writes -FLT_MAX to +0x3C. Its vtable +0x58
// is 00715CE0 itself and its +0x60 is 0069D990 (xor al, al; retn), so SetTarget (`controller_time::set_controller_target`)
// never writes the geometry data's +0x2E.
// Completion: 100% of the cold scalar
pub fn spawn_particle_reset_controller(world: &mut World) -> Entity {
    world
        .spawn((
            NiClass::ParticleResetOnLoopController,
            ParticleResetOnLoopController::default(),
        ))
        .id()
}

impl ParticleResetOnLoopController {
    // IDA: 00754E80 thunk to 00715F40 NiTimeController::LoadBinary
    // Completion: 100% of the supported modern inherited payload order
    // TODO: [ ] Original legacy flag remapping and short reads.
    pub fn parse_payload(
        bytes: &[u8],
        stream_version: u32,
    ) -> Result<ParticleTimeControllerPayload, GameError> {
        parse_particle_time_controller_payload(bytes, stream_version)
    }

    // IDA: 00754E80 -> 00715F40 writes inherited fields only; +0x3C remains cold
    // Completion: 100% of the host scalar and deferred-index publication
    // TODO: [ ] Original NiStream callback failure mutations.
    pub const fn apply_loaded_payload(&mut self, payload: &ParticleTimeControllerPayload) {
        self.time.apply_loaded_payload(payload);
    }

    // IDA: 00754E90 thunk to 00715E70 NiTimeController::LinkObject
    // Completion: 100% of the supported modern next-before-target owner association
    // TODO: [ ] The legacy < 0A000110 geometry dirty side effect.
    pub fn link_modern(
        &mut self,
        next: Option<Entity>,
        target: Option<Entity>,
        stream_version: u32,
    ) -> Result<(), GameError> {
        link_particle_time_controller_modern(&mut self.time, next, target, stream_version)
    }

    // IDA: 00754EC0 void __thiscall NiPSysResetOnLoopCtlr::Update(NiPSysResetOnLoopCtlr* self, float currentTime)
    // Completion: 85% of the flags, gate, sentinel, strict rewind, target byte and final store order
    // TODO: [ ] Bind original mapped-time +0x64 storage ABI.
    pub fn update(
        &mut self,
        world: &mut World,
        current_time: f32,
        runtime: &mut ParticleResetControllerRuntime<'_>,
    ) -> ParticleResetUpdateStatus {
        use ParticleResetUpdateStatus as Status;
        // 00754EC3: clear cycle-mode bits even when there is no target or update.
        self.time.fields.flags &= 0xFFF9;

        // 00754EC9: target +0x30 is tested before the time gate.
        if !self.time.target_linked {
            return Status::NoTarget;
        }
        if !self
            .time
            .target_30
            .is_some_and(|target| world.get_entity(target).is_ok())
        {
            return Status::UnsupportedExpiredTarget;
        }

        let evaluate = runtime.evaluate_controller_time.as_deref_mut();
        match evaluate_time_gate(&mut self.time, current_time, evaluate) {
            TimeGate::MissingEvaluator => return Status::UnsupportedTimeEvaluator,
            TimeGate::Skip => return Status::SkippedByTimeGate,
            TimeGate::Proceed => {}
        }

        // 00754EE0-00754EF7: the sentinel copies mapped +0x28 into +0x3C first.
        let first_sample = self.previous_mapped_time_3c == NEGATIVE_MAX;
        if first_sample {
            self.previous_mapped_time_3c = self.time.last_value_28;
        }

        // 00754EFA-00754F0C: ordered strict-less comparison; unordered does not raise reset. The target byte is stored before the
        // final +0x3C store.
        let rewind = self.time.last_value_28 < self.previous_mapped_time_3c;
        if rewind {
            let Some(request) = runtime.request_target_reset.as_mut() else {
                return Status::UnsupportedResetService;
            };
            request(world, self);
        }

        // 00754F13-00754F16: unconditional final mapped-time store after the gate.
        self.previous_mapped_time_3c = self.time.last_value_28;
        if rewind {
            Status::ResetRequested
        } else if first_sample {
            Status::SampledFirstTime
        } else {
            Status::AdvancedOrUnchanged
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::controller_time::set_controller_target;
    use super::super::object_net::NiObjectNet;
    use super::super::particle_controller::with_detached;
    use super::super::particle_data::ParticleData;
    use super::super::particle_system::{ParticleSystem, spawn_particle_system};
    use super::*;

    /// Ported from `particle_controller_tests`' `verify_reset_sentinel_and_strict_rewind`. The C++ fixture's chain target and
    /// system are one entity; its geometry flags are the system's data.
    #[test]
    fn reset_sentinel_and_strict_rewind() {
        use ParticleResetUpdateStatus as Status;
        let mut world = World::new();
        let reset = spawn_particle_reset_controller(&mut world);
        {
            let cold = world.get::<ParticleResetOnLoopController>(reset).unwrap();
            assert_eq!(cold.previous_mapped_time_3c.to_bits(), 0xFF7F_FFFF);
            assert_eq!(cold.time.fields.flags, 12);
        }
        let system = spawn_particle_system(&mut world);
        let data = world
            .spawn(ParticleData {
                geometry_extra_flags_2e: 0x1234,
                ..ParticleData::default()
            })
            .id();
        world.get_mut::<ParticleSystem>(system).unwrap().data_b4 = Some(data);
        set_controller_target(&mut world, reset, Some(system)).unwrap();
        assert_eq!(
            world.get::<NiObjectNet>(system).unwrap().controller_0c,
            Some(reset)
        );
        assert_eq!(
            world
                .get::<ParticleData>(data)
                .unwrap()
                .geometry_extra_flags_2e,
            0x1234
        );
        assert!(
            world
                .get::<ParticleResetOnLoopController>(reset)
                .unwrap()
                .time
                .target_linked
        );

        let mapped = std::cell::Cell::new(5.0f32);
        let mut resets = 0;
        let mut evaluate = |_: &mut ControllerTimeFields, _: f32| mapped.get();
        let mut request = |_: &mut World, snapshot: &ParticleResetOnLoopController| {
            assert_eq!(snapshot.previous_mapped_time_3c, 5.0);
            assert_eq!(snapshot.time.last_value_28, 4.0);
            resets += 1;
        };
        let mut runtime = ParticleResetControllerRuntime {
            evaluate_controller_time: Some(&mut evaluate),
            request_target_reset: Some(&mut request),
        };
        let mut run = |world: &mut World, time: f32| {
            with_detached::<ParticleResetOnLoopController, _>(world, reset, |world, state| {
                (state.update(world, time, &mut runtime), state.clone())
            })
            .unwrap()
        };

        world
            .get_mut::<ParticleResetOnLoopController>(reset)
            .unwrap()
            .time
            .fields
            .flags = 0x0E;
        let (status, state) = run(&mut world, 1.0);
        assert_eq!(status, Status::SampledFirstTime);
        assert_eq!(state.time.fields.flags, 8);
        assert_eq!(state.previous_mapped_time_3c, 5.0);

        world
            .get_mut::<ParticleResetOnLoopController>(reset)
            .unwrap()
            .time
            .byte_38 = 1;
        let (status, state) = run(&mut world, 2.0);
        assert_eq!(status, Status::AdvancedOrUnchanged);
        assert_eq!(state.previous_mapped_time_3c, 5.0);

        mapped.set(4.0);
        let (status, state) = run(&mut world, 3.0);
        assert_eq!(status, Status::ResetRequested);
        assert_eq!(state.previous_mapped_time_3c, 4.0);

        set_controller_target(&mut world, reset, None).unwrap();
        let (status, _) = run(&mut world, 4.0);
        assert_eq!(status, Status::NoTarget);
        assert_eq!(
            world.get::<NiObjectNet>(system).unwrap().controller_0c,
            None
        );
        assert_eq!(resets, 1);
    }
}
