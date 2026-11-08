//! NiPSysModifierActiveCtlr and NiPSysGravityStrengthCtlr, over NiPSysModifierCtlr.

#![allow(
    clippy::type_complexity,
    reason = "the C++ port's injected std::function services, which stand for 007155A0 and the original target virtuals"
)]

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::animation::update_float_interpolator;
use super::controller_time::ControllerTimeFields;
use super::particle_blend_interpolator::{
    ParticleBlendSampleStatus, StreamLoadStatus, sample_particle_blend_bool,
    sample_particle_blend_float,
};
use super::particle_controller::{
    ControllerLinkStatus, ParticleTimeControllerCold, ParticleTimeControllerPayload, TimeGate,
    evaluate_time_gate, link_particle_time_controller_modern,
    parse_particle_time_controller_payload, read_controller_name, resolve_time_links,
    update_active_source,
};
use super::particle_gravity::ParticleGravityModifier;
use super::particle_system::ParticleSystem;
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};

const NEGATIVE_MAX: f32 = -f32::MAX;
const TRUNCATED_U32: &str = "truncated value controller u32";

/// IDA: 00757D40 / 0075F5B0 -> 0075E460 modern LoadBinary reads inherited 00715F40 time fields, the immediate +0x3C interpolator,
/// then the owned +0x40 name. Completion: 100% of the bounded >= 0A01006D scalar, reference and name order.
/// TODO: [ ] Legacy deferred interpolator conversion and short callbacks.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleModifierValueControllerPayload {
    pub time: ParticleTimeControllerPayload,
    pub interpolator_index: u32,
    pub modifier_name: Option<Vec<u8>>,
    pub bytes_consumed: usize,
}

impl Default for ParticleModifierValueControllerPayload {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            time: ParticleTimeControllerPayload::default(),
            interpolator_index: NULL_REFERENCE,
            modifier_name: None,
            bytes_consumed: 0,
        }
    }
}

// IDA: 00757D40 -> 0075F5B0 -> 0075E460 -> 006CE320 -> 00715F40
// Completion: 100% of the supported modern inherited fields, immediate ref index and name
// TODO: [ ] < 0A01006D flags remap and < 0A010068 deferred refs; FormHeap-owned C-string failure.
pub fn parse_particle_modifier_value_controller_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<ParticleModifierValueControllerPayload, StreamError> {
    if stream_version < 0x0A01_006D {
        return Err(StreamError("legacy value controller payload unsupported"));
    }
    let time = parse_particle_time_controller_payload(bytes, stream_version)?;
    let mut reader = ByteReader::new(bytes);
    reader.take(time.bytes_consumed, TRUNCATED_U32)?;
    let interpolator_index = reader.u32(TRUNCATED_U32)?;
    let modifier_name = read_controller_name(
        &mut reader,
        TRUNCATED_U32,
        "truncated value controller modifier name",
    )?;
    Ok(ParticleModifierValueControllerPayload {
        time,
        interpolator_index,
        modifier_name,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 0075E540 NiPSysModifierCtlr base cold fields, 0075E5C0 SetTarget, 0075E6C0 PostLoad. The two lookup stages stay separate.
/// Completion: 85% of the typed time, interpolator, name and cache state.
/// TODO: [ ] Bind 00715CE0 chain mutation and the original map node lifetime.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleModifierValueControllerCold {
    pub time: ParticleTimeControllerCold,
    pub pending_interpolator_index: u32,
    pub modifier_name_40: Option<Vec<u8>>,
    pub cached_modifier_44: Option<Entity>,
    pub cached_modifier_linked: bool,
}

impl Default for ParticleModifierValueControllerCold {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            time: ParticleTimeControllerCold::default(),
            pending_interpolator_index: NULL_REFERENCE,
            modifier_name_40: None,
            cached_modifier_44: None,
            cached_modifier_linked: false,
        }
    }
}

// IDA: 0075E5EA / 0075E6E3 use the NUL-terminated +0x40 in the target's +0xD4 map
// Completion: 100% of the embedded-NUL lookup truncation for the supported typed map
// TODO: [ ] The original raw pointer lifetime.
fn find_named_modifier(
    system: &ParticleSystem,
    name: Option<&[u8]>,
) -> Result<Option<Entity>, StreamError> {
    let name = name.ok_or(StreamError("value controller modifier name unresolved"))?;
    Ok(system.find_modifier(name))
}

impl ParticleModifierValueControllerCold {
    // IDA: 00715F40 inherited scalar and deferred reference writes before 006CE320
    // Completion: 100% of the supported field publication order before the immediate interpolator
    // TODO: [ ] Original partial callback mutations.
    pub const fn apply_loaded_prefix(&mut self, payload: &ParticleModifierValueControllerPayload) {
        self.time.apply_loaded_payload(&payload.time);
        self.pending_interpolator_index = payload.interpolator_index;
    }

    // IDA: 0075E460 stores the owned +0x40 name after the direct +0x3C interpolator ref
    // Completion: 100% of the supported publication stage
    // TODO: [ ] Original C-string allocation and refcount failures.
    pub fn apply_loaded_name(&mut self, payload: &ParticleModifierValueControllerPayload) {
        self.modifier_name_40.clone_from(&payload.modifier_name);
    }

    // IDA: 0075F450 / 0075F6D0 inherited 00715E70 first, then old-version conversion; modern >= 0A010068 performs no further read
    // Completion: 100% of the modern next-before-target association
    // TODO: [ ] Legacy conversion owners and failure path.
    pub fn link_modern(
        &mut self,
        next: Option<Entity>,
        target: Option<Entity>,
        stream_version: u32,
    ) -> Result<(), StreamError> {
        if stream_version < 0x0A01_0068 {
            return Err(StreamError("legacy value controller link unsupported"));
        }
        link_particle_time_controller_modern(&mut self.time, next, target, stream_version)
    }

    // IDA: 0075E5C8 stores zero to cached +0x44 before 00715CE0 SetTarget
    // Completion: 100% of the isolated cache store; the caller performs the inherited chain mutation
    pub const fn clear_cache_before_set_target(&mut self) {
        self.cached_modifier_44 = None;
        self.cached_modifier_linked = false;
    }

    // IDA: 0075E5D4-0075E5F0 resolves the +0x40 name after base SetTarget
    // Completion: 100% of the typed lookup and target publication for a supported system
    // TODO: [ ] Bind the actual 00715CE0 mutation.
    pub fn lookup_modifier_after_set_target(
        &mut self,
        world: &World,
        target: Option<Entity>,
    ) -> Result<(), StreamError> {
        self.time.target_30 = target;
        self.time.target_linked = target.is_some();
        let Some(system) = target.and_then(|target| world.get::<ParticleSystem>(target)) else {
            return Ok(());
        };
        self.cached_modifier_44 = find_named_modifier(system, self.modifier_name_40.as_deref())?;
        self.cached_modifier_linked = self.cached_modifier_44.is_some();
        Ok(())
    }

    // IDA: 0075E6C0 invokes the base no-op, then only for a nonnull +0x30 target looks up the name and publishes +0x44; a null target
    // keeps the old +0x44
    // Completion: 100% of the supported typed PostLoad stage, without an invented SetTarget
    // TODO: [ ] The null-name raw ABI and map allocator errors.
    pub fn post_load_modifier(&mut self, world: &World) -> Result<(), StreamError> {
        let system = self
            .time
            .target_30
            .and_then(|target| world.get::<ParticleSystem>(target));
        let Some(system) = system else {
            if self.time.target_linked {
                return Err(StreamError("value controller PostLoad target expired"));
            }
            return Ok(());
        };
        self.cached_modifier_44 = find_named_modifier(system, self.modifier_name_40.as_deref())?;
        self.cached_modifier_linked = self.cached_modifier_44.is_some();
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleModifierValueUpdateStatus {
    Skipped,
    UnsupportedTimeEvaluator,
    UnsupportedInterpolator,
    UnsupportedCachedModifier,
    UnsupportedModifierType,
    UnsupportedActiveService,
    SampleUnavailable,
    Unchanged,
    Applied,
    UnsupportedBlendBranch,
}

/// IDA: 00756710 reads target +0x14 and invokes virtual +0x58 only if different. Explicit host services for the modifier owners.
/// TODO: [ ] Bind every reached concrete modifier SetActive owner.
#[derive(Default)]
pub struct ParticleModifierActiveRuntime<'a> {
    pub evaluate_controller_time: Option<&'a mut dyn FnMut(&mut ControllerTimeFields, f32) -> f32>,
    pub read_active: Option<&'a mut dyn FnMut(&World, Entity) -> Option<u8>>,
    pub write_active:
        Option<&'a mut dyn FnMut(&mut World, Entity, u8) -> Result<bool, StreamError>>,
}

#[derive(Default)]
pub struct ParticleGravityStrengthRuntime<'a> {
    pub evaluate_controller_time: Option<&'a mut dyn FnMut(&mut ControllerTimeFields, f32) -> f32>,
}

/// IDA: 00756730 NiPSysModifierActiveCtlr, size 0x48; 0075F3D0 Update; 00756710 applies the active byte via the target's +0x58
/// virtual. Completion: 80% of the direct bool and timeline interpolators and the explicit modifier setter service.
/// TODO: [ ] Blend branches and the original target virtual dispatcher.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ParticleModifierActiveController {
    pub base: ParticleModifierValueControllerCold,
    /// IDA: +0x3C a NiBoolInterpolator, NiBoolTimelineInterpolator or NiBlendBoolInterpolator.
    pub interpolator_3c: Option<Entity>,
}

/// IDA: 00757170 NiPSysGravityStrengthCtlr, size 0x48; 0075F650 Update; 00757160 writes the sampled float to NiPSysGravityModifier
/// +0x2C. Completion: 90% of the direct float interpolator and typed gravity target effect.
/// TODO: [ ] Blend branches and the exact x86 object ABI.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ParticleGravityStrengthController {
    pub base: ParticleModifierValueControllerCold,
    /// IDA: +0x3C a NiFloatInterpolator or NiBlendFloatInterpolator.
    pub interpolator_3c: Option<Entity>,
}

/// IDA: 00757F40 NiPSysEmitterLifeSpanCtlr_Create, size 0x48 over 007570F0 NiPSysModifierFloatCtlr_Constructor (vtable A87354).
/// Its Load, Link and PostLoad are the float modifier controller's, as NiPSysGravityStrengthCtlr's; it differs in CanTarget
/// (00758250: the named modifier is a NiPSysEmitter, B40B50) and ApplyValue (00757F30: the float into the cached emitter's +0x48,
/// its life span). Completion: the load, link and PostLoad; its Update (0075F650's float arm with 00757F30) isn't ported.
/// TODO: [ ] The Update and ApplyValue, and CanTarget's emitter test.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ParticleEmitterLifeSpanController {
    pub base: ParticleModifierValueControllerCold,
    /// IDA: +0x3C a NiFloatInterpolator or NiBlendFloatInterpolator.
    pub interpolator_3c: Option<Entity>,
}

impl ParticleEmitterLifeSpanController {
    // IDA: 006CE320's immediate float +0x3C reference, as the gravity controller's.
    // Completion: 100% of the typed association
    pub fn bind_interpolator_immediate(
        &mut self,
        owner: Option<Entity>,
    ) -> Result<(), StreamError> {
        if owner.is_some() != (self.base.pending_interpolator_index != NULL_REFERENCE) {
            return Err(StreamError("life span interpolator index/owner mismatch"));
        }
        self.interpolator_3c = owner;
        Ok(())
    }
}

impl ParticleModifierActiveController {
    // IDA: 006CE320 assigns the immediate bool +0x3C reference during the modern Load, after the prefix and before the name
    // Completion: 100% of the typed direct, timeline and blend association
    // TODO: [ ] Original NiRefObject reference-count stores.
    pub fn bind_interpolator_immediate(
        &mut self,
        owner: Option<Entity>,
    ) -> Result<(), StreamError> {
        if owner.is_some() != (self.base.pending_interpolator_index != NULL_REFERENCE) {
            return Err(StreamError("active interpolator index/owner mismatch"));
        }
        self.interpolator_3c = owner;
        Ok(())
    }

    // IDA: 0075F3D0 direct bool +0x60 update, then 00756710 compares raw modifier +0x14 and calls its +0x58 setter only on a
    // difference. Direct NiBool interpolators return false from virtual +0x94, so a skipped time gate cannot enter the sample branch
    // Completion: 85% of the supported gate, bool sampling and explicit target virtual adapter
    // TODO: [ ] Bind the remaining concrete modifier SetActive implementations.
    pub fn update(
        &mut self,
        world: &mut World,
        current_time: f32,
        runtime: &mut ParticleModifierActiveRuntime<'_>,
    ) -> Result<ParticleModifierValueUpdateStatus, StreamError> {
        use ParticleModifierValueUpdateStatus as Status;
        if self.base.time.fields.flags & 0x0020 != 0 {
            self.base.time.last_value_28 = NEGATIVE_MAX;
        } else {
            let evaluate = runtime.evaluate_controller_time.as_deref_mut();
            match evaluate_time_gate(&mut self.base.time, current_time, evaluate) {
                TimeGate::MissingEvaluator => return Ok(Status::UnsupportedTimeEvaluator),
                TimeGate::Skip => return Ok(Status::Skipped),
                TimeGate::Proceed => {}
            }
        }
        let Some(source) = self.interpolator_3c else {
            return Ok(Status::UnsupportedInterpolator);
        };
        let time = self.base.time.last_value_28;
        let mut value = 0u8;
        let sampled = if world.get::<NiClass>(source) == Some(&NiClass::BlendBoolInterpolator) {
            let status = sample_particle_blend_bool(world, source, time, &mut value);
            if status != ParticleBlendSampleStatus::Sampled
                && status != ParticleBlendSampleStatus::Failed
            {
                return Ok(Status::UnsupportedBlendBranch);
            }
            status == ParticleBlendSampleStatus::Sampled
        } else {
            let mut sampled_value = false;
            let okay = update_active_source(world, source, time, &mut sampled_value);
            if okay {
                value = u8::from(sampled_value);
            }
            okay
        };
        if !sampled {
            return Ok(Status::SampleUnavailable);
        }
        let Some(cached) = self.base.cached_modifier_44 else {
            return Ok(Status::UnsupportedCachedModifier);
        };
        let (Some(read_active), Some(write_active)) =
            (runtime.read_active.as_mut(), runtime.write_active.as_mut())
        else {
            return Ok(Status::UnsupportedActiveService);
        };
        let Some(old_value) = read_active(world, cached) else {
            return Ok(Status::UnsupportedModifierType);
        };
        if old_value == value {
            return Ok(Status::Unchanged);
        }
        if !write_active(world, cached, value)? {
            return Ok(Status::UnsupportedModifierType);
        }
        Ok(Status::Applied)
    }
}

impl ParticleGravityStrengthController {
    // IDA: 006CE320 assigns the immediate float +0x3C reference during the modern Load, and permits a NiBlendFloatInterpolator. The two
    // C++ bind methods are one here, since both owners are an `Entity`
    // Completion: 100% of the typed association after the prefix and before the name
    // TODO: [ ] Original NiRefObject reference-count stores and old deferred reference paths.
    pub fn bind_interpolator_immediate(
        &mut self,
        owner: Option<Entity>,
    ) -> Result<(), StreamError> {
        if owner.is_some() != (self.base.pending_interpolator_index != NULL_REFERENCE) {
            return Err(StreamError("gravity interpolator index/owner mismatch"));
        }
        self.interpolator_3c = owner;
        Ok(())
    }

    // IDA: 0075F650 direct float +0x5C update, then 00757160 stores the raw f32 into verified NiPSysGravityModifier +0x2C without an
    // equality test
    // Completion: 90% of the supported typed gravity target and exact store order
    // TODO: [ ] Original RTTI ancestry and blend interpolator alternatives.
    pub fn update(
        &mut self,
        world: &mut World,
        current_time: f32,
        runtime: &mut ParticleGravityStrengthRuntime<'_>,
    ) -> Result<ParticleModifierValueUpdateStatus, StreamError> {
        use ParticleModifierValueUpdateStatus as Status;
        if self.base.time.fields.flags & 0x0020 != 0 {
            self.base.time.last_value_28 = NEGATIVE_MAX;
        } else {
            let evaluate = runtime.evaluate_controller_time.as_deref_mut();
            match evaluate_time_gate(&mut self.base.time, current_time, evaluate) {
                TimeGate::MissingEvaluator => return Ok(Status::UnsupportedTimeEvaluator),
                TimeGate::Skip => return Ok(Status::Skipped),
                TimeGate::Proceed => {}
            }
        }
        let Some(source) = self.interpolator_3c else {
            return Ok(Status::UnsupportedInterpolator);
        };
        let time = self.base.time.last_value_28;
        let mut strength = 0.0;
        let sampled = if world.get::<NiClass>(source) == Some(&NiClass::BlendFloatInterpolator) {
            let status = sample_particle_blend_float(world, source, time, &mut strength);
            if status != ParticleBlendSampleStatus::Sampled
                && status != ParticleBlendSampleStatus::Failed
            {
                return Ok(Status::UnsupportedBlendBranch);
            }
            status == ParticleBlendSampleStatus::Sampled
        } else {
            update_float_interpolator(world, source, time, &mut strength)
        };
        if !sampled {
            return Ok(Status::SampleUnavailable);
        }
        let system = self
            .base
            .time
            .target_30
            .and_then(|target| world.get::<ParticleSystem>(target));
        let (Some(system), Some(cached), Some(name)) = (
            system,
            self.base.cached_modifier_44,
            self.base.modifier_name_40.as_deref(),
        ) else {
            return Ok(Status::UnsupportedCachedModifier);
        };
        let Some(owner) = system.find_modifier(name) else {
            return Ok(Status::UnsupportedCachedModifier);
        };
        if world.get::<NiClass>(owner) != Some(&NiClass::ParticleGravityModifier) {
            return Ok(Status::UnsupportedModifierType);
        }
        if owner != cached {
            return Ok(Status::UnsupportedCachedModifier);
        }
        world
            .get_mut::<ParticleGravityModifier>(owner)
            .ok_or(StreamError("gravity modifier component missing"))?
            .strength = strength;
        Ok(Status::Applied)
    }
}

// IDA: 0075F5B0 on a stream object: prefix, immediate +0x3C, then the name. An unsupported class stops with the prefix kept, as the
// C++ graph arm does
// Completion: 100% of the C++ graph arm
pub fn load_particle_modifier_value_controller(
    world: &mut World,
    entity: Entity,
    pending: &ParticleModifierValueControllerPayload,
    table: &StreamObjectTable,
) -> Result<StreamLoadStatus, StreamError> {
    let class = *world
        .get::<NiClass>(entity)
        .ok_or(StreamError("modifier value controller owner mismatch"))?;
    let mut source = None;
    let mut unsupported = false;
    if pending.interpolator_index != NULL_REFERENCE {
        let entry = table
            .objects
            .get(pending.interpolator_index as usize)
            .ok_or(StreamError(
                if class == NiClass::ParticleModifierActiveController {
                    "active controller interpolator out of range"
                } else {
                    "gravity controller interpolator out of range"
                },
            ))?;
        let accepted = matches!(
            (class, world.get::<NiClass>(entry.entity)),
            (
                NiClass::ParticleModifierActiveController,
                Some(
                    NiClass::BoolInterpolator
                        | NiClass::BoolTimelineInterpolator
                        | NiClass::BlendBoolInterpolator,
                ),
            ) | (
                NiClass::ParticleGravityStrengthController
                    | NiClass::ParticleEmitterLifeSpanController,
                Some(NiClass::FloatInterpolator | NiClass::BlendFloatInterpolator),
            )
        );
        if accepted {
            source = Some(entry.entity);
        } else {
            unsupported = true;
        }
    }
    match class {
        NiClass::ParticleModifierActiveController => {
            let mut owner = world
                .get_mut::<ParticleModifierActiveController>(entity)
                .ok_or(StreamError("modifier value controller owner mismatch"))?;
            owner.base.apply_loaded_prefix(pending);
            if unsupported {
                return Ok(StreamLoadStatus::UnsupportedLinkTarget);
            }
            owner.bind_interpolator_immediate(source)?;
            owner.base.apply_loaded_name(pending);
        }
        NiClass::ParticleGravityStrengthController => {
            let mut owner = world
                .get_mut::<ParticleGravityStrengthController>(entity)
                .ok_or(StreamError("modifier value controller owner mismatch"))?;
            owner.base.apply_loaded_prefix(pending);
            if unsupported {
                return Ok(StreamLoadStatus::UnsupportedLinkTarget);
            }
            owner.bind_interpolator_immediate(source)?;
            owner.base.apply_loaded_name(pending);
        }
        NiClass::ParticleEmitterLifeSpanController => {
            let mut owner = world
                .get_mut::<ParticleEmitterLifeSpanController>(entity)
                .ok_or(StreamError("modifier value controller owner mismatch"))?;
            owner.base.apply_loaded_prefix(pending);
            if unsupported {
                return Ok(StreamLoadStatus::UnsupportedLinkTarget);
            }
            owner.bind_interpolator_immediate(source)?;
            owner.base.apply_loaded_name(pending);
        }
        _ => return Err(StreamError("modifier value controller owner mismatch")),
    }
    Ok(StreamLoadStatus::Loaded)
}

// IDA: 0075F450 / 0075F6D0 modern LinkObject only resolves next then target; the cache lookup waits for PostLoad
// Completion: 100% of the C++ graph arm
pub fn link_particle_modifier_value_controller(
    world: &mut World,
    entity: Entity,
    pending: &ParticleModifierValueControllerPayload,
    table: &StreamObjectTable,
    stream_version: u32,
) -> Result<ControllerLinkStatus, StreamError> {
    let Some((next, target)) = resolve_time_links(world, &pending.time, table)? else {
        return Ok(ControllerLinkStatus::UnsupportedOwner);
    };
    match world.get::<NiClass>(entity) {
        Some(NiClass::ParticleModifierActiveController) => world
            .get_mut::<ParticleModifierActiveController>(entity)
            .ok_or(StreamError("modifier value controller owner mismatch"))?
            .base
            .link_modern(next, target, stream_version)?,
        Some(NiClass::ParticleGravityStrengthController) => world
            .get_mut::<ParticleGravityStrengthController>(entity)
            .ok_or(StreamError("modifier value controller owner mismatch"))?
            .base
            .link_modern(next, target, stream_version)?,
        Some(NiClass::ParticleEmitterLifeSpanController) => world
            .get_mut::<ParticleEmitterLifeSpanController>(entity)
            .ok_or(StreamError("modifier value controller owner mismatch"))?
            .base
            .link_modern(next, target, stream_version)?,
        _ => return Err(StreamError("modifier value controller owner mismatch")),
    }
    Ok(ControllerLinkStatus::Linked)
}

#[cfg(test)]
mod tests {
    use super::super::animation::FloatInterpolator;
    use super::super::bool_animation::BoolInterpolator;
    use super::super::particle_controller::with_detached;
    use super::super::particle_rotation::ParticleRotationModifier;
    use super::super::particle_system::{attach_particle_modifier, spawn_particle_system};
    use super::*;

    const VERSION: u32 = 0x1400_0004;

    fn identity(_: &mut ControllerTimeFields, time: f32) -> f32 {
        time
    }

    /// IDA: the field order of 0075E460. The C++ tests reach this class only through the unported stream graph tests.
    #[test]
    fn payload_order() {
        let mut bytes = Vec::new();
        bytes.extend(3u32.to_le_bytes());
        bytes.extend([0x34, 0x12]);
        for raw in [1.0f32, 2.0, 3.0, 4.0] {
            bytes.extend(raw.to_le_bytes());
        }
        bytes.extend(7u32.to_le_bytes());
        bytes.extend(9u32.to_le_bytes());
        bytes.extend(4u32.to_le_bytes());
        bytes.extend(b"Grav");
        let parsed = parse_particle_modifier_value_controller_payload(&bytes, VERSION).unwrap();
        assert_eq!(parsed.bytes_consumed, bytes.len());
        assert_eq!(parsed.time.flags, 0x1234);
        assert_eq!(parsed.interpolator_index, 9);
        assert_eq!(parsed.modifier_name.as_deref(), Some(&b"Grav"[..]));
        for length in 0..bytes.len() {
            assert!(
                parse_particle_modifier_value_controller_payload(&bytes[..length], VERSION)
                    .is_err()
            );
        }
        assert!(parse_particle_modifier_value_controller_payload(&bytes, 0x0A01_006C).is_err());
    }

    /// IDA: 0075F650 and 00757160: the sampled value goes to the cached gravity modifier without an equality test.
    #[test]
    fn gravity_strength_writes_the_cached_modifier() {
        let mut world = World::new();
        let system = spawn_particle_system(&mut world);
        let mut gravity = ParticleGravityModifier::default();
        gravity.base.name_08 = Some(b"Grav".to_vec());
        let gravity = world
            .spawn((NiClass::ParticleGravityModifier, gravity))
            .id();
        attach_particle_modifier(&mut world, system, gravity).unwrap();
        let mut source = FloatInterpolator::default();
        source.load_scalar_and_data(6.5, None);
        let source = world.spawn((NiClass::FloatInterpolator, source)).id();

        let mut controller = ParticleGravityStrengthController::default();
        controller.base.time.fields.flags = 0x0A;
        controller.base.modifier_name_40 = Some(b"Grav".to_vec());
        controller.base.pending_interpolator_index = 0;
        controller
            .bind_interpolator_immediate(Some(source))
            .unwrap();
        controller
            .base
            .lookup_modifier_after_set_target(&world, Some(system))
            .unwrap();
        let mut evaluate = identity;
        let mut runtime = ParticleGravityStrengthRuntime {
            evaluate_controller_time: Some(&mut evaluate),
        };
        assert_eq!(
            controller.update(&mut world, 1.0, &mut runtime).unwrap(),
            ParticleModifierValueUpdateStatus::Applied
        );
        assert_eq!(
            world
                .get::<ParticleGravityModifier>(gravity)
                .unwrap()
                .strength,
            6.5
        );
        assert_eq!(
            controller.update(&mut world, 1.0, &mut runtime).unwrap(),
            ParticleModifierValueUpdateStatus::Skipped,
            "the same time skips at the gate"
        );
    }

    /// IDA: 00756710: the read comes first, and the write happens only on a difference.
    #[test]
    fn active_writes_only_on_a_difference() {
        let mut world = World::new();
        let system = spawn_particle_system(&mut world);
        let mut rotation = ParticleRotationModifier::default();
        rotation.base.name_08 = Some(b"Rot".to_vec());
        let rotation = world
            .spawn((NiClass::ParticleRotationModifier, rotation))
            .id();
        attach_particle_modifier(&mut world, system, rotation).unwrap();
        let mut source = BoolInterpolator::default();
        source.load_scalar_and_data(0, None);
        let source = world.spawn((NiClass::BoolInterpolator, source)).id();
        let mut controller = ParticleModifierActiveController::default();
        controller.base.time.fields.flags = 0x0A;
        controller.base.modifier_name_40 = Some(b"Rot".to_vec());
        controller.base.pending_interpolator_index = 0;
        controller
            .bind_interpolator_immediate(Some(source))
            .unwrap();
        controller
            .base
            .lookup_modifier_after_set_target(&world, Some(system))
            .unwrap();
        let controller = world
            .spawn((NiClass::ParticleModifierActiveController, controller))
            .id();
        let mut evaluate = identity;
        let mut read = |world: &World, modifier: Entity| {
            world
                .get::<ParticleSystem>(system)
                .unwrap()
                .modifier_active_byte(world, modifier)
        };
        let mut writes = 0;
        let mut write = |world: &mut World, modifier: Entity, value: u8| {
            writes += 1;
            super::super::particle_system::set_modifier_active_byte(world, system, modifier, value)
        };
        let mut runtime = ParticleModifierActiveRuntime {
            evaluate_controller_time: Some(&mut evaluate),
            read_active: Some(&mut read),
            write_active: Some(&mut write),
        };
        let mut run = |world: &mut World, time: f32| {
            with_detached::<ParticleModifierActiveController, _>(
                world,
                controller,
                |world, state| state.update(world, time, &mut runtime).unwrap(),
            )
            .unwrap()
        };
        assert_eq!(
            run(&mut world, 1.0),
            ParticleModifierValueUpdateStatus::Applied
        );
        assert!(
            !world
                .get::<ParticleRotationModifier>(rotation)
                .unwrap()
                .base
                .active_14
        );
        assert_eq!(
            run(&mut world, 2.0),
            ParticleModifierValueUpdateStatus::Unchanged
        );
        assert_eq!(writes, 1);
    }

    /// IDA: the C++ graph's class checks at 006CE320.
    #[test]
    fn load_checks_the_interpolator_class() {
        let mut world = World::new();
        let float = world
            .spawn((NiClass::FloatInterpolator, FloatInterpolator::default()))
            .id();
        let mut table = StreamObjectTable::default();
        table.push("NiFloatInterpolator", 0, float);
        let active = world
            .spawn((
                NiClass::ParticleModifierActiveController,
                ParticleModifierActiveController::default(),
            ))
            .id();
        let gravity = world
            .spawn((
                NiClass::ParticleGravityStrengthController,
                ParticleGravityStrengthController::default(),
            ))
            .id();
        let pending = ParticleModifierValueControllerPayload {
            time: ParticleTimeControllerPayload::default(),
            interpolator_index: 0,
            modifier_name: Some(b"Name".to_vec()),
            bytes_consumed: 0,
        };
        assert_eq!(
            load_particle_modifier_value_controller(&mut world, active, &pending, &table).unwrap(),
            StreamLoadStatus::UnsupportedLinkTarget
        );
        let stopped = world
            .get::<ParticleModifierActiveController>(active)
            .unwrap();
        assert_eq!(stopped.base.pending_interpolator_index, 0);
        assert_eq!(stopped.base.modifier_name_40, None);
        assert_eq!(
            load_particle_modifier_value_controller(&mut world, gravity, &pending, &table).unwrap(),
            StreamLoadStatus::Loaded
        );
        let loaded = world
            .get::<ParticleGravityStrengthController>(gravity)
            .unwrap();
        assert_eq!(loaded.interpolator_3c, Some(float));
        assert_eq!(loaded.base.modifier_name_40.as_deref(), Some(&b"Name"[..]));
        assert_eq!(
            link_particle_modifier_value_controller(&mut world, gravity, &pending, &table, VERSION)
                .unwrap(),
            ControllerLinkStatus::Linked
        );
    }
}
