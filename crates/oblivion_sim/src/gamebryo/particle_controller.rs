//! NiTimeController's particle-side fields, NiPSysUpdateCtlr and NiPSysEmitterCtlr.
//!
//! A controller is an entity with one component, which embeds `ParticleTimeControllerCold`. The C++ `next_owner` variant is
//! `next_34: Option<Entity>` plus the entity's `NiClass`. An update that reaches other entities detaches the component
//! first, through `with_detached`, and writes it back at every exit, so that failures keep the stores already reached.
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.
//! TODO: [ ] A later phase's worker thread that loads or evaluates these runs at its own control word (workers are not recorded;
//! a thread that keeps the default is 027F, 53-bit). Main-thread behaviour stays until one is reached.

#![allow(
    clippy::type_complexity,
    reason = "the C++ port's injected std::function services, which stand for 007155A0 and the original target virtuals"
)]
#![allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "00750040 tests with these negated comparisons, so a NaN takes the same branch"
)]

use std::collections::HashSet;

use bevy_ecs::component::Mutable;
use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;
use oblivion_core::x87::truncated_low16;

use super::animation::{
    FloatInterpolator, INVALID_FLOAT_INTERPOLATOR_VALUE, update_float_interpolator,
};
use super::blend_interpolator::StreamLoadStatus;
use super::bool_animation::{
    BoolInterpolator, BoolKeyData, BoolKeyType, BoolTimelineInterpolator,
    INVALID_BOOL_INTERPOLATOR_VALUE, update_bool_interpolator, update_bool_timeline_interpolator,
};
use super::controller_time::ControllerTimeFields;
use super::object_net::NiObjectNet;
use super::particle_controller_wrap::{
    ParticleControllerWrappedBool, ParticleWrappedBoolStatus, ParticleWrappedWindow,
    begin_after_prior_active, begin_after_prior_inactive, end_of_active_run,
};
use super::particle_modifier_value_controller::{
    ParticleGravityStrengthController, ParticleModifierActiveController,
};
use super::particle_reset_controller::ParticleResetOnLoopController;
use super::particle_system::ParticleSystem;
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable, read_owned_c_string};

const NEGATIVE_MAX: f32 = -f32::MAX;
const TRUNCATED_U32: &str = "truncated particle controller u32";

// IDA: N/A runs `action` on a controller detached from the world, and writes it back afterwards whatever the outcome.
// Completion: 100%
pub fn with_detached<C: Component<Mutability = Mutable> + Clone, R>(
    world: &mut World,
    entity: Entity,
    action: impl FnOnce(&mut World, &mut C) -> R,
) -> Option<R> {
    let mut state = world.get::<C>(entity)?.clone();
    let result = action(world, &mut state);
    *world.get_mut::<C>(entity)? = state;
    Some(result)
}

/// IDA: 00715990 NiTimeController constructor, as host values rather than an overlay of the original 0x3C-byte x86 object.
/// `fields` is what 007155A0 uses. Completion: 100% of the observed scalar and pointer-null cold fields.
/// TODO: [ ] Bind original NiObject ownership.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleTimeControllerCold {
    pub fields: ControllerTimeFields,
    pub last_value_28: f32,
    pub byte_2c: u8,
    /// IDA: +0x30's presence. Kept apart from `target_30`, which distinguishes a null target from a stale host one.
    pub target_linked: bool,
    pub next_linked: bool,
    pub pending_next_index: u32,
    pub pending_target_index: u32,
    pub byte_38: u8,
    /// IDA: +0x34 the owning next link.
    pub next_34: Option<Entity>,
    /// IDA: +0x30 the raw target.
    pub target_30: Option<Entity>,
}

impl Default for ParticleTimeControllerCold {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            fields: ControllerTimeFields::default(),
            last_value_28: NEGATIVE_MAX,
            byte_2c: 1,
            target_linked: false,
            next_linked: false,
            pending_next_index: NULL_REFERENCE,
            pending_target_index: NULL_REFERENCE,
            byte_38: 0,
            next_34: None,
            target_30: None,
        }
    }
}

impl ParticleTimeControllerCold {
    // IDA: 00754E80 -> 00715F40 writes +0x34 reference, +0x08 flags, +0x0C / +0x10 / +0x14 / +0x18 floats, then +0x30 reference
    // Completion: 100% of the host scalar and deferred-index publication
    // TODO: [ ] Bind original NiStream callback failure mutations.
    pub const fn apply_loaded_payload(&mut self, payload: &ParticleTimeControllerPayload) {
        self.pending_next_index = payload.next_index;
        self.fields.flags = payload.flags;
        self.fields.frequency = payload.frequency;
        self.fields.phase = payload.phase;
        self.fields.low_key_time = payload.low_key_time;
        self.fields.high_key_time = payload.high_key_time;
        self.pending_target_index = payload.target_index;
    }
}

// IDA: N/A the NiTimeController part of a controller entity: a node controller's or NiControllerManager's (`NodeController`),
// else by its class tag. This is the one read of a controller's +0x30 target and +0x34 next; an entity with neither state
// ends a chain walk.
// Completion: 100% of the bound classes
#[must_use]
pub fn controller_time(world: &World, entity: Entity) -> Option<&ParticleTimeControllerCold> {
    if let Some(node) = world.get::<super::transform_controller::NodeController>(entity) {
        return Some(&node.time);
    }
    match world.get::<NiClass>(entity)? {
        NiClass::ParticleUpdateController => world
            .get::<ParticleUpdateController>(entity)
            .map(|held| &held.time),
        NiClass::ParticleEmitterController => world
            .get::<ParticleEmitterController>(entity)
            .map(|held| &held.time),
        NiClass::ParticleResetOnLoopController => world
            .get::<ParticleResetOnLoopController>(entity)
            .map(|held| &held.time),
        NiClass::ParticleModifierActiveController => world
            .get::<ParticleModifierActiveController>(entity)
            .map(|held| &held.base.time),
        NiClass::ParticleGravityStrengthController => world
            .get::<ParticleGravityStrengthController>(entity)
            .map(|held| &held.base.time),
        NiClass::ParticleEmitterLifeSpanController | NiClass::ParticleEmitterSpeedController => {
            world
                .get::<super::particle_modifier_value_controller::ParticleEmitterFloatController>(
                    entity,
                )
                .map(|held| &held.base.time)
        }
        NiClass::AlphaController | NiClass::MaterialColorController => world
            .get::<super::material_controllers::MaterialController>(entity)
            .map(|held| &held.time),
        NiClass::GeomMorpherController => world
            .get::<super::morph::GeomMorpherController>(entity)
            .map(|held| &held.time),
        NiClass::FlipController | NiClass::TextureTransformController => world
            .get::<super::texture::TextureController>(entity)
            .map(|held| &held.time),
        _ => None,
    }
}

// IDA: N/A the mutable side of `controller_time`.
// Completion: 100% of the bound classes
pub fn controller_time_mut(
    world: &mut World,
    entity: Entity,
) -> Option<Mut<'_, ParticleTimeControllerCold>> {
    if world
        .get::<super::transform_controller::NodeController>(entity)
        .is_some()
    {
        return world
            .get_mut::<super::transform_controller::NodeController>(entity)
            .map(|node| node.map_unchanged(|node| &mut node.time));
    }
    match *world.get::<NiClass>(entity)? {
        NiClass::ParticleUpdateController => world
            .get_mut::<ParticleUpdateController>(entity)
            .map(|held| held.map_unchanged(|held| &mut held.time)),
        NiClass::ParticleEmitterController => world
            .get_mut::<ParticleEmitterController>(entity)
            .map(|held| held.map_unchanged(|held| &mut held.time)),
        NiClass::ParticleResetOnLoopController => world
            .get_mut::<ParticleResetOnLoopController>(entity)
            .map(|held| held.map_unchanged(|held| &mut held.time)),
        NiClass::ParticleModifierActiveController => world
            .get_mut::<ParticleModifierActiveController>(entity)
            .map(|held| held.map_unchanged(|held| &mut held.base.time)),
        NiClass::ParticleGravityStrengthController => world
            .get_mut::<ParticleGravityStrengthController>(entity)
            .map(|held| held.map_unchanged(|held| &mut held.base.time)),
        NiClass::ParticleEmitterLifeSpanController | NiClass::ParticleEmitterSpeedController => world
            .get_mut::<super::particle_modifier_value_controller::ParticleEmitterFloatController>(
                entity,
            )
            .map(|held| held.map_unchanged(|held| &mut held.base.time)),
        NiClass::AlphaController | NiClass::MaterialColorController => world
            .get_mut::<super::material_controllers::MaterialController>(entity)
            .map(|held| held.map_unchanged(|held| &mut held.time)),
        NiClass::GeomMorpherController => world
            .get_mut::<super::morph::GeomMorpherController>(entity)
            .map(|held| held.map_unchanged(|held| &mut held.time)),
        NiClass::FlipController | NiClass::TextureTransformController => world
            .get_mut::<super::texture::TextureController>(entity)
            .map(|held| held.map_unchanged(|held| &mut held.time)),
        _ => None,
    }
}

// IDA: 00715E70 resolves next +0x34 with strong NiRefObjectPtr semantics, then writes target +0x30 directly
// Completion: 100% of the supported modern link order; an `Entity` substitutes for the original intrusive counts
// TODO: [ ] Port the < 0A000110 virtual +0x60 target-property side effect.
pub fn link_particle_time_controller_modern(
    time: &mut ParticleTimeControllerCold,
    next: Option<Entity>,
    target: Option<Entity>,
    stream_version: u32,
) -> Result<(), GameError> {
    if stream_version < 0x0A00_0110 {
        return Err(GameError::unclassified(
            0,
            "legacy controller link unsupported",
        ));
    }
    if next.is_some() != (time.pending_next_index != NULL_REFERENCE) {
        return Err(GameError::unclassified(
            0,
            "controller next index/owner mismatch",
        ));
    }
    if target.is_some() != (time.pending_target_index != NULL_REFERENCE) {
        return Err(GameError::unclassified(
            0,
            "controller target index/owner mismatch",
        ));
    }
    time.next_linked = next.is_some();
    time.next_34 = next;
    time.target_30 = target;
    time.target_linked = target.is_some();
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleTimeControllerPayload {
    pub next_index: u32,
    pub flags: u16,
    pub frequency: f32,
    pub phase: f32,
    pub low_key_time: f32,
    pub high_key_time: f32,
    pub target_index: u32,
    pub bytes_consumed: usize,
}

impl Default for ParticleTimeControllerPayload {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            next_index: NULL_REFERENCE,
            flags: 0,
            frequency: 0.0,
            phase: 0.0,
            low_key_time: 0.0,
            high_key_time: 0.0,
            target_index: NULL_REFERENCE,
            bytes_consumed: 0,
        }
    }
}

// IDA: 00715F40 loads NiTimeController after 00700640's NiObject no-op: next ref, u16 flags, four f32 values, target ref
// Completion: 100% of the >= 0A01006D raw field order and widths; no legacy flag remapping
// TODO: [ ] Pre-0A01006D flags and NiStream short reads.
pub fn parse_particle_time_controller_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<ParticleTimeControllerPayload, GameError> {
    // 00715F40's version arms: below 0A000101 and 0A000103 the flags are rebuilt (not reached: no stream that old is bound);
    // below 0A01006D (10.1.0.106) bit 0x20 clears.
    if stream_version < 0x0A01_006D && stream_version != 0x0A01_006A {
        return Err(GameError::unclassified(
            0,
            "legacy particle controller flags unsupported",
        ));
    }
    let mut reader = ByteReader::new(bytes);
    let next_index = reader.u32(TRUNCATED_U32)?;
    let mut flags = reader.u16("truncated particle controller flags")?;
    let frequency = reader.f32(TRUNCATED_U32)?;
    let phase = reader.f32(TRUNCATED_U32)?;
    let low_key_time = reader.f32(TRUNCATED_U32)?;
    let high_key_time = reader.f32(TRUNCATED_U32)?;
    let target_index = reader.u32(TRUNCATED_U32)?;
    if stream_version < 0x0A01_006D {
        flags &= !0x20;
    }
    Ok(ParticleTimeControllerPayload {
        next_index,
        flags,
        frequency,
        phase,
        low_key_time,
        high_key_time,
        target_index,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 006C36B0's outcome. The C++ port pasted the gate into three files; here it is one function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeGate {
    Skip,
    Proceed,
    MissingEvaluator,
}

// IDA: 006C36B0 NiTimeController_ShouldSkipUpdate: checks active, last time and byte +0x38, calls controller vtable +0x64
// when byte +0x2C is set, and skips only when no update is needed
// Completion: 100% of the scoped scalar gate and its no-output branch order
pub fn evaluate_time_gate(
    time: &mut ParticleTimeControllerCold,
    current_time: f32,
    evaluate: Option<&mut (dyn FnMut(&mut ControllerTimeFields, f32) -> f32 + '_)>,
) -> TimeGate {
    if time.fields.flags & 0x0008 == 0
        || (current_time == time.fields.last_time && time.byte_38 == 0)
    {
        return TimeGate::Skip;
    }
    if time.byte_2c == 0 {
        time.byte_38 = 0;
        return TimeGate::Proceed;
    }
    let Some(evaluate) = evaluate else {
        return TimeGate::MissingEvaluator;
    };
    let value = evaluate(&mut time.fields, current_time);
    if value != time.last_value_28 || time.byte_38 != 0 {
        time.last_value_28 = value;
        time.byte_38 = 0;
        return TimeGate::Proceed;
    }
    TimeGate::Skip
}

/// IDA: 00753BE0's first-emitter inputs: the first NiPSysEmitterCtlr in the target's controller chain, and its cached emitter
/// +0x48 lifespan (not controller +0x48).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleUpdateEmitterTiming {
    pub controller_flags: u16,
    pub emitter_lifespan: f32,
}

/// IDA: N/A the explicit inputs of 00753BE0. `None` stands for the C++ empty `std::function`.
#[derive(Default)]
pub struct ParticleUpdateControllerRuntime<'a> {
    pub first_emitter: Option<ParticleUpdateEmitterTiming>,
    pub evaluate_controller_time: Option<&'a mut dyn FnMut(&mut ControllerTimeFields, f32) -> f32>,
    pub update_target_system: Option<&'a mut dyn FnMut(f32) -> Result<(), GameError>>,
}

/// IDA: 00753BB0 NiPSysUpdateCtlr factory: 0x3C owner, low and high key times both overwritten with zero. Completion: 100% of
/// the cold fields.
/// TODO: [ ] Original FormHeap allocation failure and vtable identity.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ParticleUpdateController {
    pub time: ParticleTimeControllerCold,
}

impl Default for ParticleUpdateController {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        let mut time = ParticleTimeControllerCold::default();
        time.fields.high_key_time = 0.0;
        time.fields.low_key_time = 0.0;
        Self { time }
    }
}

impl ParticleUpdateController {
    // IDA: 00753BE0 clears flag bit 1, sets bit 2, applies the first NiPSysEmitterCtlr's cached emitter +0x48 lifespan, calls
    // 006C36B0, then conditionally invokes target vtable +0x98 with +0x28
    // Completion: 100% of the observed scheduler scalar and branch order, with exact external seams
    // TODO: [ ] The controller stays detached during the target call; bind it in place if a target update ever reads it.
    pub fn update(
        &mut self,
        current_time: f32,
        runtime: &mut ParticleUpdateControllerRuntime<'_>,
    ) -> Result<(), GameError> {
        self.time.fields.low_key_time = NEGATIVE_MAX;
        self.time.fields.high_key_time = f32::MAX;
        self.time.fields.flags = (self.time.fields.flags & 0xFFFD) | 0x0004;
        let Some(update_target) = runtime.update_target_system.as_mut() else {
            return Err(GameError::unclassified(
                0,
                "particle update controller target unresolved",
            ));
        };
        if self.time.fields.last_time != NEGATIVE_MAX
            && let Some(first) = runtime.first_emitter
            && first.controller_flags & 0x0006 == 0
            && first.emitter_lifespan < current_time - self.time.fields.last_time
        {
            self.time.fields.last_time = current_time;
        }

        // 006C36B0 returns true when no target update is needed. Its +0x64 evaluator 007155A0 owns the accumulator, last-time
        // and cached-time writes.
        let evaluate = runtime.evaluate_controller_time.as_deref_mut();
        let gate = evaluate_time_gate(&mut self.time, current_time, evaluate);
        if gate == TimeGate::MissingEvaluator {
            return Err(GameError::unclassified(
                0,
                "particle controller time map unresolved",
            ));
        }
        self.time.fields.high_key_time = 0.0;
        self.time.fields.low_key_time = 0.0;
        if gate == TimeGate::Proceed {
            update_target(self.time.last_value_28)?;
        }
        Ok(())
    }
}

// IDA: 00753CC0 after 00715CE0: traverse target +0x0C through each +0x34 next; if the final node is not self, assign
// tail.next = self, target.head to self.next, then clear self.next. 006C61E0 and 00478300 confirm those pointer destinations
// Completion: 100% of the typed order when self is head or tail
// TODO: [ ] A caller: this expects 00715CE0's attach to have run, as the C++ port does.
pub fn update_controller_set_target_in_chain(
    world: &mut World,
    controller: Entity,
    target: Option<Entity>,
) -> Result<(), GameError> {
    let is_update = world.get::<NiClass>(controller) == Some(&NiClass::ParticleUpdateController);
    let (true, Some(target)) = (is_update, target) else {
        return Err(GameError::unclassified(
            0,
            "update controller target/self unresolved",
        ));
    };
    let old_target = controller_time(world, controller)
        .ok_or(GameError::unclassified(
            0,
            "update controller target/self unresolved",
        ))?
        .target_30;
    if old_target.is_some_and(|old| old != target) {
        return Err(GameError::unclassified(
            0,
            "old controller target chain unresolved",
        ));
    }
    let head = world
        .get::<NiObjectNet>(target)
        .ok_or(GameError::unclassified(
            0,
            "update controller target/self unresolved",
        ))?
        .controller_0c;
    let mut chain = Vec::new();
    let mut visited = HashSet::new();
    let mut cursor = head;
    while let Some(entry) = cursor {
        if !visited.insert(entry) {
            return Err(GameError::unclassified(
                0,
                "null target controller chain entry",
            ));
        }
        let time = controller_time(world, entry)
            .ok_or(GameError::unclassified(0, "null target controller owner"))?;
        chain.push(entry);
        cursor = time.next_34;
    }
    let found = chain
        .iter()
        .position(|&entry| entry == controller)
        .filter(|&place| place == 0 || place + 1 == chain.len())
        .ok_or(GameError::unclassified(
            0,
            "update controller base chain placement unresolved",
        ))?;
    if found == 0 && chain.len() > 1 {
        chain.remove(found);
        chain.push(controller);
    }
    {
        let mut time = controller_time_mut(world, controller).ok_or(GameError::unclassified(
            0,
            "update controller target/self unresolved",
        ))?;
        time.target_30 = Some(target);
        time.target_linked = true;
    }
    for index in 0..chain.len() {
        let next = chain.get(index + 1).copied();
        let mut time = controller_time_mut(world, chain[index])
            .ok_or(GameError::unclassified(0, "null target controller owner"))?;
        time.next_34 = next;
        time.next_linked = next.is_some();
    }
    world
        .get_mut::<NiObjectNet>(target)
        .ok_or(GameError::unclassified(
            0,
            "update controller target/self unresolved",
        ))?
        .controller_0c = chain.first().copied();
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParticleEmitterControllerPayload {
    pub time: ParticleTimeControllerPayload,
    pub birth_rate_interpolator_index: u32,
    pub modifier_name: Option<Vec<u8>>,
    pub active_interpolator_index: u32,
    pub bytes_consumed: usize,
}

impl Default for ParticleEmitterControllerPayload {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            time: ParticleTimeControllerPayload::default(),
            birth_rate_interpolator_index: NULL_REFERENCE,
            modifier_name: None,
            active_interpolator_index: NULL_REFERENCE,
            bytes_consumed: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleEmitterUpdateStatus {
    Skipped,
    ManagerSampled,
    ConstantEmitted,
    KeyedInitialized,
    KeyedQuiet,
    KeyedTransitionProcessed,
    UnsupportedTimeline,
}

/// IDA: N/A the explicit inputs of 00750260. `emit` takes the world and the detached controller, in place of the C++ captures.
#[derive(Default)]
pub struct ParticleEmitterControllerRuntime<'a> {
    pub evaluate_controller_time: Option<&'a mut dyn FnMut(&mut ControllerTimeFields, f32) -> f32>,
    /// IDA: 007502CA reads the cached NiPSysEmitter +0x48 lifespan.
    pub emitter_lifespan: f32,
    pub target_system_update_time: f32,
    pub emit: Option<
        &'a mut dyn FnMut(
            &mut World,
            &ParticleEmitterController,
            f32,
            &[f32],
        ) -> Result<(), GameError>,
    >,
    /// IDA: 007509DF reads target +0xE8 after each active rate sample; the scalar above is a fallback.
    pub read_target_system_update_time: Option<&'a mut dyn FnMut(&World) -> f32>,
}

// IDA: 0074F980 -> 0075E460 -> 006CE320 -> 006D0550 -> 00715F40
// Completion: 100% of the >= 0A01006D raw byte sequence and the distinct immediate and deferred refs
// TODO: [ ] 0074FE30's legacy interpolator-key conversion.
pub fn parse_particle_emitter_controller_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<ParticleEmitterControllerPayload, GameError> {
    if stream_version < 0x0A01_0068 {
        return Err(GameError::unclassified(
            0,
            "legacy emitter controller payload unsupported",
        ));
    }
    let time = parse_particle_time_controller_payload(bytes, stream_version)?;
    let mut reader = ByteReader::new(bytes);
    reader.take(time.bytes_consumed, TRUNCATED_U32)?;
    let birth_rate_interpolator_index = reader.u32(TRUNCATED_U32)?;
    let modifier_name = read_owned_c_string(
        &mut reader,
        TRUNCATED_U32,
        "truncated particle controller name",
    )?;
    let active_interpolator_index = reader.u32(TRUNCATED_U32)?;
    Ok(ParticleEmitterControllerPayload {
        time,
        birth_rate_interpolator_index,
        modifier_name,
        active_interpolator_index,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 0074FDE0 NiPSysEmitterCtlr factory through 0075E540, 006CE1D0 and 006D04E0. Completion: 100% of the observed cold
/// scalars and null links in host storage.
/// TODO: [ ] Wrapped, keyed-rate and blend branches, and concrete interpolator, modifier and target ownership.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ParticleEmitterController {
    pub time: ParticleTimeControllerCold,
    pub interpolator_linked: bool,
    pub modifier_name_present: bool,
    pub cached_modifier_linked: bool,
    pub field_48_linked: bool,
    pub last_emit_time_50: f32,
    pub byte_54: u8,
    pub field_58: u32,
    pub field_5c: u32,
    pub field_60: u32,
    pub pending_birth_rate_interpolator_index: u32,
    pub modifier_name_40: Option<Vec<u8>>,
    pub pending_active_interpolator_index: u32,
    /// IDA: +0x3C a NiFloatInterpolator or NiBlendFloatInterpolator.
    pub birth_rate_interpolator_3c: Option<Entity>,
    /// IDA: +0x48 a NiBoolInterpolator, NiBoolTimelineInterpolator or NiBlendBoolInterpolator.
    pub active_interpolator_48: Option<Entity>,
    /// IDA: 0075E5C0 caches target map lookup +0x44 by the +0x40 name.
    /// TODO: [ ] Bind the original NiTPointerMap bucket allocator and lifetime.
    pub cached_modifier_44: Option<Entity>,
    /// IDA: 0075026A tests original +0x4C against the selected birth-rate owner; 0074FDE0 did not explicitly initialize this
    /// field in the captured chunks.
    /// TODO: [ ] Verify FormHeap zeroing and first-update value at runtime.
    pub cached_birth_rate_interpolator_4c: Option<Entity>,
    pub queued_ages: Vec<f32>,
}

impl Default for ParticleEmitterController {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        let mut time = ParticleTimeControllerCold::default();
        time.fields.flags &= !0x20;
        Self {
            time,
            interpolator_linked: false,
            modifier_name_present: false,
            cached_modifier_linked: false,
            field_48_linked: false,
            last_emit_time_50: NEGATIVE_MAX,
            byte_54: 0,
            field_58: 0,
            field_5c: 0,
            field_60: 0,
            pending_birth_rate_interpolator_index: NULL_REFERENCE,
            modifier_name_40: None,
            pending_active_interpolator_index: NULL_REFERENCE,
            birth_rate_interpolator_3c: None,
            active_interpolator_48: None,
            cached_modifier_44: None,
            cached_birth_rate_interpolator_4c: None,
            queued_ages: Vec::new(),
        }
    }
}

// IDA: bool virtual +0x60 on a direct NiBoolInterpolator or NiBoolTimelineInterpolator
// Completion: 100% of the two typed owners
pub(super) fn update_active_source(
    world: &mut World,
    source: Entity,
    time: f32,
    output: &mut bool,
) -> bool {
    if world.get::<NiClass>(source) == Some(&NiClass::BoolTimelineInterpolator) {
        update_bool_timeline_interpolator(world, source, time, output)
    } else {
        update_bool_interpolator(world, source, time, output)
    }
}

// IDA: N/A the (data +0x10, cached value +0x0C) a bool or timeline interpolator holds
// Completion: 100%
fn active_source_state(world: &World, source: Entity) -> (Option<Entity>, u8) {
    let state = if world.get::<NiClass>(source) == Some(&NiClass::BoolTimelineInterpolator) {
        world
            .get::<BoolTimelineInterpolator>(source)
            .expect("a NiBoolTimelineInterpolator entity")
            .base
    } else {
        *world
            .get::<BoolInterpolator>(source)
            .expect("a NiBoolInterpolator entity")
    };
    (state.data_10, state.cached_value_0c)
}

impl ParticleEmitterController {
    // IDA: N/A standalone decoded-value staging; the original 0074F980 performs the immediate reference assignment between the
    // two methods below
    // Completion: 100% of the host adapter
    pub fn apply_loaded_payload(&mut self, payload: &ParticleEmitterControllerPayload) {
        self.apply_loaded_time_payload(payload);
        self.apply_loaded_modifier_payload(payload);
    }

    // IDA: 0074F980 -> 006CE320 -> 00715F40 stores inherited time fields, then resolves +0x3C
    // Completion: 100% of the pre-immediate-reference publication
    // TODO: [ ] Original stream callbacks, intrusive refs and partial reads.
    pub const fn apply_loaded_time_payload(&mut self, payload: &ParticleEmitterControllerPayload) {
        self.time.apply_loaded_payload(&payload.time);
        self.pending_birth_rate_interpolator_index = payload.birth_rate_interpolator_index;
    }

    // IDA: 0075E460 reads and stores the +0x40 name after 006CE320's immediate +0x3C; 0074F980 then queues the deferred +0x48
    // active interpolator reference
    // Completion: 100% of the supported scalar and name stage and the original publication sequence
    // TODO: [ ] Original FormHeap C-string allocation failures.
    pub fn apply_loaded_modifier_payload(&mut self, payload: &ParticleEmitterControllerPayload) {
        self.modifier_name_40.clone_from(&payload.modifier_name);
        self.modifier_name_present = self.modifier_name_40.is_some();
        self.pending_active_interpolator_index = payload.active_interpolator_index;
    }

    // IDA: 006CE320 stores the direct NiInterpolator ref at +0x3C, and 0074FE30 assigns the deferred active-bool ref at +0x48
    // Completion: 100% of the typed association for the supported non-blend interpolators
    // TODO: [ ] Original refcount ordering and fallback blend maps.
    pub fn bind_interpolators(
        &mut self,
        birth_rate: Option<Entity>,
        active: Option<Entity>,
    ) -> Result<(), GameError> {
        if birth_rate.is_none() {
            return Err(GameError::unclassified(
                0,
                "null emitter birth-rate interpolator",
            ));
        }
        if active.is_none() {
            return Err(GameError::unclassified(
                0,
                "null emitter active interpolator",
            ));
        }
        self.birth_rate_interpolator_3c = birth_rate;
        self.active_interpolator_48 = active;
        self.interpolator_linked = true;
        self.field_48_linked = true;
        Ok(())
    }

    // IDA: 006CE320 assigns the immediate NiInterpolator reference to +0x3C during LoadBinary, ahead of 0074FE30's deferred +0x48
    // Completion: 100% of the typed timing; refcounts are not modelled
    // TODO: [ ] Integrate NiStream_ReadObjectReference's exact error domain.
    pub fn bind_birth_rate_immediate(
        &mut self,
        birth_rate: Option<Entity>,
    ) -> Result<(), GameError> {
        if birth_rate.is_some() != (self.pending_birth_rate_interpolator_index != NULL_REFERENCE) {
            return Err(GameError::unclassified(
                0,
                "birth-rate index/owner mismatch",
            ));
        }
        self.birth_rate_interpolator_3c = birth_rate;
        self.interpolator_linked = birth_rate.is_some();
        Ok(())
    }

    // IDA: 0074FE30 calls 0075E480 -> 006CE1C0 -> 00754E90 -> 00715E70 before phase randomization and the deferred +0x48
    // assignment
    // Completion: 100% of the modern control order for a typed non-blend bool owner; CRT rand is injected
    // TODO: [ ] Legacy NiPSysEmitterCtlrData conversion below 0A010068.
    pub fn link_modern(
        &mut self,
        next: Option<Entity>,
        target: Option<Entity>,
        active: Option<Entity>,
        stream_version: u32,
        draw_rand: Option<&mut (dyn FnMut() -> i32 + '_)>,
        io_thread: bool,
    ) -> Result<(), GameError> {
        if stream_version < 0x0A01_0068 {
            return Err(GameError::unclassified(
                0,
                "legacy emitter controller link unsupported",
            ));
        }
        link_particle_time_controller_modern(&mut self.time, next, target, stream_version)?;
        if self.time.fields.flags & 0x0006 == 0 && self.time.fields.phase == 0.0 {
            let Some(draw_rand) = draw_rand else {
                return Err(GameError::unclassified(
                    0,
                    "controller phase rand unresolved",
                ));
            };
            self.time.fields.phase = controller_phase(draw_rand(), io_thread);
        }
        if active.is_some() != (self.pending_active_interpolator_index != NULL_REFERENCE) {
            return Err(GameError::unclassified(
                0,
                "active bool index/owner mismatch",
            ));
        }
        self.active_interpolator_48 = active;
        self.field_48_linked = active.is_some();
        Ok(())
    }

    // IDA: 0075E5C0 clears +0x44, calls 00715CE0 SetTarget, then probes the target's +0xD4 modifier-name map and caches the found
    // pointer at +0x44. 0075E5EA passes the NUL-terminated +0x40 key, so bytes after an embedded NUL do not enter the lookup
    // Completion: 100% of the lookup order after an explicit target association
    // TODO: [ ] Bind 00715CE0's chain mutation here; `controller_time::set_controller_target` does it separately.
    pub fn cache_modifier_from_target(&mut self, world: &World, target: Option<Entity>) {
        self.cached_modifier_44 = None;
        self.cached_modifier_linked = false;
        self.time.target_30 = target;
        self.time.target_linked = target.is_some();
        let (Some(target), Some(name)) = (target, self.modifier_name_40.as_deref()) else {
            return;
        };
        self.cached_modifier_44 = world
            .get::<ParticleSystem>(target)
            .and_then(|system| system.find_modifier(name));
        self.cached_modifier_linked = self.cached_modifier_44.is_some();
    }

    // IDA: 0075E6C0 NiPSysModifierCtlr::PostLoad calls the base no-op, then only with a nonnull target writes the modifier-name
    // map result to +0x44. No SetTarget or chain mutation occurs
    // Completion: 100% of the supported lookup stage
    // TODO: [ ] The original null-key crash and map allocator lifetime.
    pub fn post_load_modifier(&mut self, world: &World) -> Result<(), GameError> {
        let Some(system) = self
            .time
            .target_30
            .and_then(|target| world.get::<ParticleSystem>(target))
        else {
            return Ok(());
        };
        let name = self
            .modifier_name_40
            .as_deref()
            .ok_or(GameError::unclassified(
                0,
                "particle controller PostLoad modifier name unresolved",
            ))?;
        self.cached_modifier_44 = system.find_modifier(name);
        self.cached_modifier_linked = self.cached_modifier_44.is_some();
        Ok(())
    }

    // IDA: 00750040 rate-window helper feeding NiPSysEmitter virtual +0x5C: interval tests, two low-u16 truncated ticks, the
    // 15-child cap, age queue order, and even a zero-count Emit for accepted intervals. The two parts of the C++ helper are
    // the store stage here and the call in `emit_rate_window`
    // Completion: 100% of the verified order
    // TODO: [ ] Original NiTArray allocator failure.
    #[allow(
        clippy::too_many_arguments,
        reason = "00750040's six ordered f32 interval, time and rate arguments"
    )]
    pub(super) fn emit_rate_window(
        &mut self,
        world: &mut World,
        emit: Option<
            &mut (
                     dyn FnMut(
                &mut World,
                &ParticleEmitterController,
                f32,
                &[f32],
            ) -> Result<(), GameError>
                         + '_
                 ),
        >,
        current_time: f32,
        upper_time: f32,
        lower_time: f32,
        interval_begin: f32,
        interval_end: f32,
        rate: f32,
    ) -> Result<(), GameError> {
        if !(rate > 0.0)
            || !(interval_end > interval_begin)
            || !(upper_time > interval_begin)
            || !(lower_time < interval_end)
        {
            return Ok(());
        }
        let Some(emit) = emit else {
            return Err(GameError::unclassified(
                0,
                "particle emitter Emit callback unresolved",
            ));
        };
        // `std::min(upper, end)`, which keeps `upper` unless `end` is strictly smaller.
        let clipped_end = if interval_end < upper_time {
            interval_end
        } else {
            upper_time
        };
        let duration = clipped_end - interval_begin;
        let start_offset = if lower_time > interval_begin {
            lower_time - interval_begin
        } else {
            0.0
        };
        // 007500D7 / 007500FE: the fmul rounds to a float at 24-bit before the fistp.
        let mut tick = truncated_low16(f64::from(start_offset * rate));
        let end_tick = truncated_low16(f64::from(duration * rate));
        let mut count = end_tick.wrapping_sub(tick);
        if count > 15 {
            count = 15;
        }
        // 00750135..0075013E stores 1/rate through an m32 FSTP before the loop.
        let reciprocal = 1.0 / rate;
        self.queued_ages.clear();
        for _ in 0..count {
            tick = tick.wrapping_add(1);
            self.queued_ages
                .push(duration - f32::from(tick) * reciprocal);
        }
        emit(world, self, current_time, &self.queued_ages)
    }

    // IDA: 00750260 beginning through 00750454: a rewind clears +0x50; the target and cached emitter guard; the manager flag
    // writes +0x28; the cached emitter +0x48 lifespan threshold; the 006C36B0 gate; direct NiBool and NiFloat selection; a
    // manager-controlled owner change samples bool virtual +0x60 into +0x54; then 0074FA90 / 0053D850 and the keyed windows
    // Completion: 100% of the scoped non-blend manager branch and store order, the first keyed-bool sample and the non-wrapping
    // constant-rate keyed transition loop, capped at 20 passes. Keyed emission and the blend fallback return
    // `UnsupportedTimeline` (a host stop) after the reached writes
    // TODO: [ ] Bind the +0x44 emitter object and the full keyed 00750260 continuation from 007504F2, and x87 precision.
    pub fn update_managed_non_blend(
        &mut self,
        world: &mut World,
        current_time: f32,
        runtime: &mut ParticleEmitterControllerRuntime<'_>,
    ) -> Result<ParticleEmitterUpdateStatus, GameError> {
        use ParticleEmitterUpdateStatus as Status;
        if current_time < self.time.fields.last_time {
            self.last_emit_time_50 = NEGATIVE_MAX;
        }
        if !self.time.target_linked || !self.cached_modifier_linked {
            return Ok(Status::Skipped);
        }
        if self.time.fields.flags & 0x0020 != 0 {
            self.time.last_value_28 = NEGATIVE_MAX;
        }
        if self.time.fields.last_time != NEGATIVE_MAX
            && self.time.fields.flags & 0x0006 == 0
            && runtime.emitter_lifespan < current_time - self.time.fields.last_time
        {
            self.time.fields.last_time = current_time;
        }
        let evaluate = runtime.evaluate_controller_time.as_deref_mut();
        match evaluate_time_gate(&mut self.time, current_time, evaluate) {
            TimeGate::Skip => return Ok(Status::Skipped),
            TimeGate::MissingEvaluator => {
                return Err(GameError::unclassified(
                    0,
                    "particle controller time map unresolved",
                ));
            }
            TimeGate::Proceed => {}
        }
        // 007502F1 checks +0x48 first, then 007502FC checks +0x3C. Both null pointers return immediately after the gate, before
        // interpolation.
        let Some(active_source) = self.active_interpolator_48.filter(|_| self.field_48_linked)
        else {
            return Ok(Status::Skipped);
        };
        let Some(rate_source) = self.birth_rate_interpolator_3c else {
            return Ok(Status::Skipped);
        };

        // 00750306..007503CE: a NiBlendBoolInterpolator at +0x48 and a NiBlendFloatInterpolator at +0x3C normalize their
        // weights (006CD0F0) and give their dominant item's time (006CC550, 0074F7F0); 0074FA90 / 0053D850 then resolve each
        // to the plain interpolator it holds (a plain one answers itself). A null source returns (00750AB7). The sample time is
        // the rate blend's item time when either blend gave a time, else +0x28.
        let mut active_time = NEGATIVE_MAX;
        let mut active_source = active_source;
        if let Some(mut blend) =
            world.get_mut::<super::blend_interpolator::ParticleBlendBoolInterpolator>(active_source)
        {
            let blend = &mut *blend;
            super::blend_interpolator::normalize_blend_weights(
                &mut blend.base,
                &blend.item_sources,
            )?;
            let index = super::blend_interpolator::dominant_blend_item(&blend.base);
            if index != 0xFF {
                active_time = super::blend_interpolator::blend_item_time(&blend.base, index);
            }
            let resolved = super::blend_interpolator::blend_item_source(
                &blend.base,
                &blend.item_sources,
                blend.single_source_18,
            );
            let Some(resolved) = resolved else {
                return Ok(Status::Skipped);
            };
            active_source = resolved;
        }
        let mut rate_time = NEGATIVE_MAX;
        let mut rate_source = rate_source;
        if let Some(mut blend) =
            world.get_mut::<super::blend_interpolator::ParticleBlendFloatInterpolator>(rate_source)
        {
            let blend = &mut *blend;
            super::blend_interpolator::normalize_blend_weights(
                &mut blend.base,
                &blend.item_sources,
            )?;
            let index = super::blend_interpolator::dominant_blend_item(&blend.base);
            if index != 0xFF {
                rate_time = super::blend_interpolator::blend_item_time(&blend.base, index);
            }
            let resolved = super::blend_interpolator::blend_item_source(
                &blend.base,
                &blend.item_sources,
                blend.single_source_18,
            );
            let Some(resolved) = resolved else {
                return Ok(Status::Skipped);
            };
            rate_source = resolved;
        }
        let sample_time = if active_time != NEGATIVE_MAX || rate_time != NEGATIVE_MAX {
            rate_time
        } else {
            self.time.last_value_28
        };
        if self.time.fields.flags & 0x0020 != 0
            && self.cached_birth_rate_interpolator_4c != Some(rate_source)
        {
            self.last_emit_time_50 = sample_time;
            let mut active = self.byte_54 != 0;
            update_active_source(world, active_source, sample_time, &mut active);
            self.byte_54 = u8::from(active);
            self.cached_birth_rate_interpolator_4c = Some(rate_source);
            return Ok(Status::ManagerSampled);
        }
        self.cached_birth_rate_interpolator_4c = Some(rate_source);

        // 007506EB..00750755 samples the bool key track directly. Its first time step only initializes +0x50 and +0x54. On later
        // steps these stores precede the bounded 20-iteration transition search. The keys are cloned because the rate
        // interpolator needs the world during the walk, where the C++ port holds a shared_ptr.
        let (active_data_entity, active_cached) = active_source_state(world, active_source);
        let active_data = active_data_entity
            .map(|data| {
                world
                    .get::<BoolKeyData>(data)
                    .expect("linked NiBoolData")
                    .clone()
            })
            .filter(|data| !data.keys().is_empty());
        if let Some(active_data) = active_data {
            return self.update_keyed(world, &active_data, sample_time, rate_source, runtime);
        }

        // 0075046C obtains the active interpolator's key count. With no data, its +0xB0 predicate (006E7FE0) accepts a cached
        // value other than 2.
        if active_data_entity.is_some() || active_cached == INVALID_BOOL_INTERPOLATOR_VALUE {
            return Ok(Status::UnsupportedTimeline);
        }
        let mut active = false;
        update_active_source(world, active_source, 0.0, &mut active);
        if !active {
            return Ok(Status::Skipped);
        }

        // 007504CB..007504E7 snapshots prior +0x50 and target +0xE8, then publishes mapped time to +0x50 before checking the float
        // interpolator. 007504DF reads target +0xE8 before that publication and before the constant-rate virtual call.
        let mut previous_emit_time = self.last_emit_time_50;
        let target_update_time = match runtime.read_target_system_update_time.as_mut() {
            Some(read) => read(world),
            None => runtime.target_system_update_time,
        };
        self.last_emit_time_50 = sample_time;
        let rate_state = rate_state(world, rate_source);
        if rate_state.0.is_some() || rate_state.1 == INVALID_FLOAT_INTERPOLATOR_VALUE {
            return Ok(Status::UnsupportedTimeline);
        }
        let mut elapsed = sample_time - previous_emit_time;
        if previous_emit_time > sample_time {
            previous_emit_time = 0.0;
            elapsed = sample_time;
        }
        let mut rate = 0.0;
        if !update_float_interpolator(world, rate_source, 0.0, &mut rate) {
            return Ok(Status::Skipped);
        }
        let emit_time = target_update_time + elapsed;
        self.emit_rate_window(
            world,
            emit_reborrow(&mut runtime.emit),
            emit_time,
            sample_time,
            previous_emit_time,
            0.0,
            sample_time,
            rate,
        )?;
        Ok(Status::ConstantEmitted)
    }

    // IDA: 00750723..00750A83 the keyed-bool continuation of 00750260
    // Completion: 100% of the first sample, the wrapped hand-off and the finite sorted type-5 constant-rate windows
    // TODO: [ ] Keyed-rate paths and x87 precision from 007504F2.
    fn update_keyed(
        &mut self,
        world: &mut World,
        active_data: &BoolKeyData,
        sample_time: f32,
        rate_source: Entity,
        runtime: &mut ParticleEmitterControllerRuntime<'_>,
    ) -> Result<ParticleEmitterUpdateStatus, GameError> {
        use ParticleEmitterUpdateStatus as Status;
        // 00750723-00750755: first sample and wrapped scans share the raw bool-key sample. The wrapped continuation keeps their
        // reached +0x50 / +0x54 stores; ordinary time samples use the path below.
        if self.last_emit_time_50 == NEGATIVE_MAX || sample_time < self.last_emit_time_50 {
            let target_time = runtime.target_system_update_time;
            let emit = &mut runtime.emit;
            let mut emit_window = |world: &mut World,
                                   controller: &mut Self,
                                   window: &ParticleWrappedWindow|
             -> Result<(), GameError> {
                controller.emit_rate_window(
                    world,
                    emit_reborrow(emit),
                    window.emission_time,
                    window.upper_time,
                    window.lower_time,
                    window.interval_begin,
                    window.interval_end,
                    window.rate,
                )
            };
            let read_target = runtime.read_target_system_update_time.as_deref_mut();
            let wrapped = ParticleControllerWrappedBool::continue_after_gate_with_emit(
                world,
                self,
                active_data,
                sample_time,
                target_time,
                Some(&mut emit_window),
                read_target,
            )?;
            match wrapped.status {
                ParticleWrappedBoolStatus::InitializedFirstSample => {
                    return Ok(Status::KeyedInitialized);
                }
                ParticleWrappedBoolStatus::WrappedQuiet => return Ok(Status::KeyedQuiet),
                ParticleWrappedBoolStatus::WrappedEmitted => {
                    return Ok(Status::KeyedTransitionProcessed);
                }
                ParticleWrappedBoolStatus::NotWrapped => {}
                _ => return Ok(Status::UnsupportedTimeline),
            }
        }

        let mut first_index = 0;
        let Some(sampled) = active_data.sample(sample_time, &mut first_index) else {
            return Ok(Status::UnsupportedTimeline);
        };
        let first_tick = self.last_emit_time_50 == NEGATIVE_MAX;
        let previous_emit_time = self.last_emit_time_50;
        let previous_active = self.byte_54;
        if first_tick {
            // 00750732 precedes 00750736 on the sentinel-only exit.
            self.last_emit_time_50 = sample_time;
            self.byte_54 = sampled;
            return Ok(Status::KeyedInitialized);
        }
        // 00750752 stores raw +0x54 before 00750755 stores time +0x50.
        self.byte_54 = sampled;
        self.last_emit_time_50 = sample_time;

        // 007507AA..00750A83: when the previous bool is on, scan backward to its latest preceding off key; when off, find the next
        // on key strictly after the previous time and at or before current time. The next off key (or final key) bounds each
        // active window. Rate is sampled at that window midpoint. It stops after 20 windows. Other sign and wrap paths need their
        // separate x87 trace.
        let (rate_data, rate_cached) = rate_state(world, rate_source);
        if sample_time < previous_emit_time
            || active_data.key_type() != BoolKeyType::STEP
            || rate_data.is_some()
            || rate_cached == INVALID_FLOAT_INTERPOLATOR_VALUE
        {
            return Ok(Status::UnsupportedTimeline);
        }
        let keys = active_data.keys();
        let mut scan_time = previous_emit_time;
        let mut scan_active = previous_active;
        let mut emitted_window = false;
        for _ in 0..20 {
            let on_index = if scan_active != 0 {
                // 007507AA..007507DF: the original reverse walk keeps a one-past index and accepts the last off key at or before the
                // prior time. Zero means the current run started at key 0.
                begin_after_prior_active(keys, scan_time)
            } else {
                // 007507E6..00750825 accepts the first on key within the strict-open prior, closed current interval. `scan_time`
                // is below `sample_time` on every pass here (the wrapped case returned above), so the wrapped head scan never
                // runs; none found is `keys.len()`.
                begin_after_prior_inactive(keys, scan_time, sample_time).unwrap_or(keys.len())
            };
            if on_index == keys.len() {
                return Ok(if emitted_window {
                    Status::KeyedTransitionProcessed
                } else {
                    Status::KeyedQuiet
                });
            }
            // 00750867..00750891: the next off key, or the final key.
            let end_index = end_of_active_run(keys, on_index);
            let begin = keys[on_index].time;
            let end = keys[end_index].time;
            let midpoint = begin + (end - begin) * 0.5;
            let mut rate = 0.0;
            if !update_float_interpolator(world, rate_source, midpoint, &mut rate) {
                return Ok(Status::UnsupportedTimeline);
            }
            let upper = if end < sample_time { end } else { sample_time };
            // IDA: 007509DF reads target +0xE8 after rate virtual +0x5C.
            let target_update_time = match runtime.read_target_system_update_time.as_mut() {
                Some(read) => read(world),
                None => runtime.target_system_update_time,
            };
            let emit_time = target_update_time + sample_time - scan_time;
            self.emit_rate_window(
                world,
                emit_reborrow(&mut runtime.emit),
                emit_time,
                upper,
                scan_time,
                begin,
                end,
                rate,
            )?;
            emitted_window = true;
            if end >= sample_time {
                return Ok(Status::KeyedTransitionProcessed);
            }
            let mut scan_index = 0;
            let Some(next_active) = active_data.sample(end, &mut scan_index) else {
                return Ok(Status::UnsupportedTimeline);
            };
            scan_time = end;
            scan_active = next_active;
        }
        Ok(Status::KeyedTransitionProcessed)
    }

    // IDA: 0074F820 stores -FLT_MAX at +0x50 before 00715570 clears the +0x08 active bit, resets +0x20 last time, and conditionally
    // the +0x1C start time
    // Completion: 100% of the observed scalar writes and their conditional order
    // TODO: [ ] NiTimeController virtual chaining when a runtime owns a target.
    pub const fn stop(&mut self) {
        self.last_emit_time_50 = NEGATIVE_MAX;
        self.time.fields.flags &= !0x0008;
        self.time.fields.last_time = NEGATIVE_MAX;
        if self.time.fields.flags & 0x0001 != 0 {
            self.time.fields.start_time = NEGATIVE_MAX;
        }
    }
}

type EmitService<'s, 'a> = Option<
    &'s mut (
                dyn FnMut(
        &mut World,
        &ParticleEmitterController,
        f32,
        &[f32],
    ) -> Result<(), GameError>
                    + 'a
            ),
>;

// IDA: N/A a helper, with no address of its own.
fn emit_reborrow<'s, 'a>(emit: &'s mut EmitService<'a, 'a>) -> EmitService<'s, 'a> {
    match emit {
        Some(emit) => Some(&mut **emit),
        None => None,
    }
}

// IDA: N/A a NiFloatInterpolator's (data +0x10, cached value +0x0C)
// Completion: 100%
pub(super) fn rate_state(world: &World, source: Entity) -> (Option<Entity>, f32) {
    let state = world
        .get::<FloatInterpolator>(source)
        .expect("a NiFloatInterpolator entity");
    (state.data_10, state.cached_value_0c)
}

/// IDA: N/A the C++ stream graph's outcome for a deferred link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControllerLinkStatus {
    Linked,
    UnsupportedOwner,
    UnsupportedService,
}

// IDA: 00715E70's references as the C++ graph resolves them: +0x34 any bound controller, +0x30 a NiParticleSystem
// Completion: 100% of the graph's checks; `None` is its unsupported owner
pub(super) fn resolve_time_links(
    world: &World,
    time: &ParticleTimeControllerPayload,
    table: &StreamObjectTable,
) -> Result<Option<(Option<Entity>, Option<Entity>)>, GameError> {
    let mut next = None;
    if time.next_index != NULL_REFERENCE {
        let entry = table
            .objects
            .get(time.next_index as usize)
            .ok_or(GameError::unclassified(
                0,
                "controller next index out of range",
            ))?;
        if !world
            .get::<NiClass>(entry.entity)
            .is_some_and(|class| class.is_particle_controller())
        {
            return Ok(None);
        }
        next = Some(entry.entity);
    }
    let mut target = None;
    if time.target_index != NULL_REFERENCE {
        let entry =
            table
                .objects
                .get(time.target_index as usize)
                .ok_or(GameError::unclassified(
                    0,
                    "controller target index out of range",
                ))?;
        if world.get::<NiClass>(entry.entity) != Some(&NiClass::ParticleSystem) {
            return Ok(None);
        }
        target = Some(entry.entity);
    }
    Ok(Some((next, target)))
}

// IDA: 00754E80 -> 00715F40 on a stream object, with no derived field. Update and ResetOnLoop share this
// Completion: 100% of the host publication
pub fn load_particle_time_controller(
    world: &mut World,
    entity: Entity,
    pending: &ParticleTimeControllerPayload,
) -> Result<(), GameError> {
    controller_time_mut(world, entity)
        .ok_or(GameError::unclassified(
            0,
            "particle controller component missing",
        ))?
        .apply_loaded_payload(pending);
    Ok(())
}

// IDA: 00715E70 on a stream object: NiPSysUpdateCtlr and NiPSysResetOnLoopCtlr add nothing to it
// Completion: 100% of the C++ graph arm
pub fn link_particle_time_controller(
    world: &mut World,
    entity: Entity,
    pending: &ParticleTimeControllerPayload,
    table: &StreamObjectTable,
    stream_version: u32,
) -> Result<ControllerLinkStatus, GameError> {
    let Some((next, target)) = resolve_time_links(world, pending, table)? else {
        return Ok(ControllerLinkStatus::UnsupportedOwner);
    };
    let mut time = controller_time_mut(world, entity).ok_or(GameError::unclassified(
        0,
        "particle controller component missing",
    ))?;
    link_particle_time_controller_modern(&mut time, next, target, stream_version)?;
    Ok(ControllerLinkStatus::Linked)
}

// IDA: 0074F980 on a stream object: time fields, immediate +0x3C (a NiFloatInterpolator or NiBlendFloatInterpolator), then the name and deferred index
// Completion: 100% of the C++ graph arm
pub fn load_particle_emitter_controller(
    world: &mut World,
    entity: Entity,
    pending: &ParticleEmitterControllerPayload,
    table: &StreamObjectTable,
) -> Result<StreamLoadStatus, GameError> {
    let mut birth_rate = None;
    let mut unsupported = false;
    if pending.birth_rate_interpolator_index != NULL_REFERENCE {
        let entry = table
            .objects
            .get(pending.birth_rate_interpolator_index as usize)
            .ok_or(GameError::unclassified(
                0,
                "controller immediate birth-rate index out of range",
            ))?;
        if world
            .get::<NiClass>(entry.entity)
            .is_some_and(|class| class.is_blend_float_source())
        {
            birth_rate = Some(entry.entity);
        } else {
            unsupported = true;
        }
    }
    let mut controller =
        world
            .get_mut::<ParticleEmitterController>(entity)
            .ok_or(GameError::unclassified(
                0,
                "particle controller component missing",
            ))?;
    controller.apply_loaded_time_payload(pending);
    if unsupported {
        return Ok(StreamLoadStatus::UnsupportedLinkTarget);
    }
    controller.bind_birth_rate_immediate(birth_rate)?;
    controller.apply_loaded_modifier_payload(pending);
    Ok(StreamLoadStatus::Loaded)
}

// IDA: 0074FE30 on a stream object: next, target and the deferred +0x48, a bool, timeline or blend bool interpolator. Modern
// LinkObject never invokes SetTarget or the cache lookup
// Completion: 100% of the C++ graph arm
pub fn link_particle_emitter_controller(
    world: &mut World,
    entity: Entity,
    pending: &ParticleEmitterControllerPayload,
    table: &StreamObjectTable,
    stream_version: u32,
    draw_rand: Option<&mut (dyn FnMut() -> i32 + '_)>,
    io_thread: bool,
) -> Result<ControllerLinkStatus, GameError> {
    let Some((next, target)) = resolve_time_links(world, &pending.time, table)? else {
        return Ok(ControllerLinkStatus::UnsupportedOwner);
    };
    let mut active = None;
    if pending.active_interpolator_index != NULL_REFERENCE {
        let entry = table
            .objects
            .get(pending.active_interpolator_index as usize)
            .ok_or(GameError::unclassified(
                0,
                "controller active index out of range",
            ))?;
        if !world
            .get::<NiClass>(entry.entity)
            .is_some_and(|class| class.is_blend_bool_source())
        {
            return Ok(ControllerLinkStatus::UnsupportedOwner);
        }
        active = Some(entry.entity);
    }
    if pending.time.flags & 6 == 0 && pending.time.phase == 0.0 && draw_rand.is_none() {
        return Ok(ControllerLinkStatus::UnsupportedService);
    }
    let mut controller =
        world
            .get_mut::<ParticleEmitterController>(entity)
            .ok_or(GameError::unclassified(
                0,
                "particle controller component missing",
            ))?;
    controller.link_modern(next, target, active, stream_version, draw_rand, io_thread)?;
    Ok(ControllerLinkStatus::Linked)
}

/// IDA: 0074FF37 `fild; fdiv qword; fmul qword; fstp dword`: 32767, then 100. On the main thread (007F, a 24-bit run of
/// the x87) the quotient rounds, then the product. On the IO thread (027F) only the store rounds: all 9 of the 38 stored
/// phases that tell the forms apart are 53-bit (linux-capture-2026-11-06, under wined3d). Completion: 100%.
#[must_use]
pub fn controller_phase(rand: i32, io_thread: bool) -> f32 {
    let quotient = f64::from(rand) / 32767.0;
    if io_thread {
        (quotient * 100.0) as f32
    } else {
        quotient as f32 * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::super::particle_system::spawn_particle_system;
    use super::*;

    // The IO thread's own stores: run-c1-log's rand at 0074FF37, run-b1-snap's +0x10 at 0040F16A. These 9 of the 38 are the
    // ones where the two forms differ, then 3 where they don't.
    #[test]
    fn the_io_thread_stores_the_53_bit_phase() {
        for (rand, bits) in [
            (18467, 0x4261_6F23_u32),
            (19169, 0x426A_00F4),
            (29358, 0x42B3_3146),
            (24464, 0x4295_522B),
            (11942, 0x4211_C7E4),
            (4827, 0x416B_B357),
            (5436, 0x4184_B809),
            (14604, 0x4232_46E5),
            (18716, 0x4264_7949),
            (41, 0x3E00_2100),
            (6334, 0x419A_A4B5),
            (26500, 0x42A1_BF83),
        ] {
            assert_eq!(controller_phase(rand, true).to_bits(), bits, "{rand}");
        }
        assert_eq!(controller_phase(18467, false).to_bits(), 0x4261_6F24);
    }

    const VERSION: u32 = 0x1400_0004;

    fn spawn_float(world: &mut World, value: f32) -> Entity {
        let mut rate = FloatInterpolator::default();
        rate.load_scalar_and_data(value, None);
        world.spawn((NiClass::FloatInterpolator, rate)).id()
    }

    /// Ported from `particle_controller_tests`' `verify_payload_width_and_cold_state`.
    #[test]
    fn payload_width_and_cold_state() {
        let update = ParticleUpdateController::default();
        let emitter = ParticleEmitterController::default();
        assert_eq!(update.time.fields.flags, 12);
        assert_eq!(update.time.fields.low_key_time, 0.0);
        assert_eq!(update.time.fields.high_key_time, 0.0);
        assert_eq!(emitter.time.fields.flags, 12);
        assert_eq!(emitter.active_interpolator_48, None);
        assert_eq!(emitter.birth_rate_interpolator_3c, None);
        assert!(!emitter.field_48_linked);

        let mut bytes = Vec::new();
        bytes.extend(3u32.to_le_bytes()); // next reference
        bytes.extend([0x34, 0x12]); // u16 flags, not u32
        for raw in [0x3F80_0000u32, 0x4000_0000, 0x4040_0000, 0x4080_0000] {
            bytes.extend(raw.to_le_bytes());
        }
        bytes.extend(7u32.to_le_bytes()); // target reference
        let time = parse_particle_time_controller_payload(&bytes, VERSION).unwrap();
        assert_eq!(time.bytes_consumed, 26);
        assert_eq!(time.next_index, 3);
        assert_eq!(time.flags, 0x1234);
        assert_eq!(time.target_index, 7);
        assert_eq!(time.high_key_time.to_bits(), 0x4080_0000);
        assert!(parse_particle_time_controller_payload(&bytes[..25], VERSION).is_err());

        bytes.extend(9u32.to_le_bytes()); // immediate birth-rate reference
        bytes.extend(0u32.to_le_bytes()); // signed name length: absent
        bytes.extend(11u32.to_le_bytes()); // deferred active reference
        let parsed = parse_particle_emitter_controller_payload(&bytes, VERSION).unwrap();
        assert_eq!(parsed.bytes_consumed, 38);
        assert_eq!(parsed.birth_rate_interpolator_index, 9);
        assert_eq!(parsed.active_interpolator_index, 11);
        assert_eq!(parsed.modifier_name, None);
    }

    /// Ported from `particle_controller_tests`' `verify_null_active_owner_gate`.
    #[test]
    fn null_active_owner_gate() {
        let mut world = World::new();
        let mut controller = ParticleEmitterController::default();
        controller.time.fields.flags = 0x0A;
        controller
            .link_modern(None, None, None, VERSION, None, false)
            .unwrap();
        assert_eq!(controller.active_interpolator_48, None);
        assert!(!controller.field_48_linked);
        controller.time.target_linked = true;
        controller.cached_modifier_linked = true;
        let mut evaluations = 0;
        let mut emissions = 0;
        let mut evaluate = |_: &mut ControllerTimeFields, _: f32| {
            evaluations += 1;
            1.0
        };
        let mut emit = |_: &mut World, _: &ParticleEmitterController, _: f32, _: &[f32]| {
            emissions += 1;
            Ok(())
        };
        let mut runtime = ParticleEmitterControllerRuntime {
            evaluate_controller_time: Some(&mut evaluate),
            emit: Some(&mut emit),
            ..ParticleEmitterControllerRuntime::default()
        };
        assert_eq!(
            controller
                .update_managed_non_blend(&mut world, 1.0, &mut runtime)
                .unwrap(),
            ParticleEmitterUpdateStatus::Skipped
        );
        assert_eq!(evaluations, 1);
        assert_eq!(emissions, 0);
    }

    /// Ported from `particle_controller_tests`' `verify_emitter_load_publication_order`.
    #[test]
    fn emitter_load_publication_order() {
        let mut world = World::new();
        let mut controller = ParticleEmitterController {
            modifier_name_40: Some(b"Old".to_vec()),
            modifier_name_present: true,
            pending_active_interpolator_index: 42,
            ..ParticleEmitterController::default()
        };
        let payload = ParticleEmitterControllerPayload {
            time: ParticleTimeControllerPayload {
                next_index: 7,
                flags: 0x1234,
                ..ParticleTimeControllerPayload::default()
            },
            birth_rate_interpolator_index: 2,
            modifier_name: Some(b"New".to_vec()),
            active_interpolator_index: 3,
            bytes_consumed: 0,
        };
        controller.apply_loaded_time_payload(&payload);
        assert_eq!(controller.time.pending_next_index, 7);
        assert_eq!(controller.time.fields.flags, 0x1234);
        assert_eq!(controller.pending_birth_rate_interpolator_index, 2);
        assert_eq!(controller.modifier_name_40.as_deref(), Some(&b"Old"[..]));
        assert_eq!(controller.pending_active_interpolator_index, 42);
        assert_eq!(controller.birth_rate_interpolator_3c, None);
        let immediate = spawn_float(&mut world, 0.0);
        controller
            .bind_birth_rate_immediate(Some(immediate))
            .unwrap();
        assert_eq!(controller.birth_rate_interpolator_3c, Some(immediate));
        assert_eq!(controller.modifier_name_40.as_deref(), Some(&b"Old"[..]));
        assert_eq!(controller.pending_active_interpolator_index, 42);
        controller.apply_loaded_modifier_payload(&payload);
        assert_eq!(controller.modifier_name_40.as_deref(), Some(&b"New"[..]));
        assert!(controller.modifier_name_present);
        assert_eq!(controller.pending_active_interpolator_index, 3);
        assert_eq!(controller.birth_rate_interpolator_3c, Some(immediate));
        assert_eq!(controller.cached_modifier_44, None);
    }

    /// Ported from `particle_controller_tests`' `verify_keyed_windows`.
    #[test]
    fn keyed_windows() {
        let mut world = World::new();
        let rate = spawn_float(&mut world, 5.0);
        let keys = BoolKeyData::new(
            BoolKeyType::STEP,
            [(0.0, 0), (1.0, 1), (2.0, 0), (3.0, 1), (4.0, 0)]
                .iter()
                .map(|&(time, value)| super::super::bool_animation::BoolKey { time, value })
                .collect(),
        )
        .unwrap();
        let keys = world.spawn((NiClass::BoolKeyData, keys)).id();
        let mut interpolator = BoolInterpolator::default();
        interpolator.set_data(Some(keys));
        let active = world.spawn((NiClass::BoolInterpolator, interpolator)).id();
        let mut controller = ParticleEmitterController::default();
        controller.time.fields.flags = 0x0A;
        controller.time.target_linked = true;
        controller.cached_modifier_linked = true;
        controller
            .bind_interpolators(Some(rate), Some(active))
            .unwrap();
        let mut counts = Vec::new();
        let mut times = Vec::new();
        let mut emitted_ages: Vec<Vec<f32>> = Vec::new();
        let mut evaluate = |_: &mut ControllerTimeFields, time: f32| time;
        let mut emit = |_: &mut World, _: &ParticleEmitterController, time: f32, ages: &[f32]| {
            times.push(time);
            counts.push(ages.len());
            emitted_ages.push(ages.to_vec());
            Ok(())
        };
        let mut runtime = ParticleEmitterControllerRuntime {
            target_system_update_time: 10.0,
            evaluate_controller_time: Some(&mut evaluate),
            emit: Some(&mut emit),
            ..ParticleEmitterControllerRuntime::default()
        };
        assert_eq!(
            controller
                .update_managed_non_blend(&mut world, 0.0, &mut runtime)
                .unwrap(),
            ParticleEmitterUpdateStatus::KeyedInitialized
        );
        assert_eq!(
            controller
                .update_managed_non_blend(&mut world, 4.0, &mut runtime)
                .unwrap(),
            ParticleEmitterUpdateStatus::KeyedTransitionProcessed
        );
        assert_eq!(times, [14.0, 12.0]);
        assert_eq!(counts, [5, 5]);
        assert!(emitted_ages[0][0] > *emitted_ages[0].last().unwrap());
        assert!(emitted_ages[1][0] > *emitted_ages[1].last().unwrap());
        assert_eq!(controller.byte_54, 0);
    }

    /// IDA: 00750040's rejection, cap and even a zero-count Emit. The C++ helper had no direct test.
    #[test]
    fn rate_window_bounds() {
        let mut world = World::new();
        let mut controller = ParticleEmitterController::default();
        controller
            .emit_rate_window(&mut world, None, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0)
            .unwrap();
        assert!(
            controller
                .emit_rate_window(&mut world, None, 0.0, 1.0, 0.0, 0.0, 1.0, 5.0)
                .is_err(),
            "an accepted interval needs Emit"
        );
        let mut seen = Vec::new();
        let mut emit = |_: &mut World, _: &ParticleEmitterController, _: f32, ages: &[f32]| {
            seen.push(ages.len());
            Ok(())
        };
        controller
            .emit_rate_window(&mut world, Some(&mut emit), 0.0, 1.0, 0.0, 0.0, 1.0, 100.0)
            .unwrap();
        controller
            .emit_rate_window(&mut world, Some(&mut emit), 0.0, 1.0, 0.0, 0.0, 1.0, 0.5)
            .unwrap();
        assert_eq!(seen, [15, 0]);
    }

    /// IDA: 00753BE0's stores around the gate, and the service required at entry.
    #[test]
    fn update_controller_gate_and_stores() {
        let mut update = ParticleUpdateController::default();
        update.time.fields.flags = 0x000A;
        let mut mapped = None;
        let mut evaluate = |_: &mut ControllerTimeFields, time: f32| time;
        let mut target = |time: f32| {
            mapped = Some(time);
            Ok(())
        };
        let mut runtime = ParticleUpdateControllerRuntime {
            first_emitter: None,
            evaluate_controller_time: Some(&mut evaluate),
            update_target_system: Some(&mut target),
        };
        update.update(2.0, &mut runtime).unwrap();
        assert_eq!(mapped, Some(2.0));
        assert_eq!(update.time.fields.flags, 0x000C);
        assert_eq!(update.time.fields.low_key_time, 0.0);
        assert_eq!(update.time.fields.high_key_time, 0.0);
        assert!(
            update
                .update(3.0, &mut ParticleUpdateControllerRuntime::default())
                .is_err()
        );
    }

    /// IDA: 00753CC0 moves self from the head to the tail.
    #[test]
    fn update_controller_moves_to_the_tail() {
        let mut world = World::new();
        let target = spawn_particle_system(&mut world);
        let other = world
            .spawn((
                NiClass::ParticleEmitterController,
                ParticleEmitterController::default(),
            ))
            .id();
        let mut update = ParticleUpdateController::default();
        update.time.next_34 = Some(other);
        let update = world
            .spawn((NiClass::ParticleUpdateController, update))
            .id();
        world.get_mut::<NiObjectNet>(target).unwrap().controller_0c = Some(update);
        update_controller_set_target_in_chain(&mut world, update, Some(target)).unwrap();
        assert_eq!(
            world.get::<NiObjectNet>(target).unwrap().controller_0c,
            Some(other)
        );
        assert_eq!(
            controller_time(&world, other).unwrap().next_34,
            Some(update)
        );
        assert_eq!(controller_time(&world, update).unwrap().next_34, None);
        assert_eq!(
            controller_time(&world, update).unwrap().target_30,
            Some(target)
        );
        assert!(update_controller_set_target_in_chain(&mut world, update, None).is_err());
    }

    /// IDA: the C++ graph's class checks at 0074F980 and 0074FE30.
    #[test]
    fn load_and_link_check_classes() {
        let mut world = World::new();
        let system = spawn_particle_system(&mut world);
        let rate = spawn_float(&mut world, 1.0);
        let active = world
            .spawn((NiClass::BoolInterpolator, BoolInterpolator::default()))
            .id();
        let emitter = world
            .spawn((
                NiClass::ParticleEmitterController,
                ParticleEmitterController::default(),
            ))
            .id();
        let mut table = StreamObjectTable::default();
        table.push("NiParticleSystem", 0, system);
        table.push("NiFloatInterpolator", 0, rate);
        table.push("NiBoolInterpolator", 0, active);
        let mut pending = ParticleEmitterControllerPayload {
            time: ParticleTimeControllerPayload {
                flags: 0x0A,
                target_index: 0,
                ..ParticleTimeControllerPayload::default()
            },
            birth_rate_interpolator_index: 2,
            modifier_name: Some(b"Emitter".to_vec()),
            active_interpolator_index: 2,
            bytes_consumed: 0,
        };
        assert_eq!(
            load_particle_emitter_controller(&mut world, emitter, &pending, &table).unwrap(),
            StreamLoadStatus::UnsupportedLinkTarget
        );
        pending.birth_rate_interpolator_index = 1;
        assert_eq!(
            load_particle_emitter_controller(&mut world, emitter, &pending, &table).unwrap(),
            StreamLoadStatus::Loaded
        );
        assert_eq!(
            link_particle_emitter_controller(
                &mut world, emitter, &pending, &table, VERSION, None, false
            )
            .unwrap(),
            ControllerLinkStatus::Linked
        );
        let linked = world.get::<ParticleEmitterController>(emitter).unwrap();
        assert_eq!(linked.birth_rate_interpolator_3c, Some(rate));
        assert_eq!(linked.active_interpolator_48, Some(active));
        assert_eq!(linked.time.target_30, Some(system));

        let mut no_rand = pending.clone();
        no_rand.time.flags = 0;
        assert_eq!(
            link_particle_emitter_controller(
                &mut world, emitter, &no_rand, &table, VERSION, None, false
            )
            .unwrap(),
            ControllerLinkStatus::UnsupportedService
        );
    }
}
