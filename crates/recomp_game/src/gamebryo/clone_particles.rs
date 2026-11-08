//! NiObject_Clone for a NiParticleSystem and what it holds: the system (00749F90, 00749A70, ProcessClone 007495E0),
//! NiPSysData (00759D80, 00759940), the modifiers the prison's fire models use (position 007556F0, grow-fade 00756E40,
//! age-death 0075BE50 / 0075BEA0, bound-update 0075AC80, color 0075A4A0, gravity 00751800 / 00753800, spawn 007526E0, box
//! emitter 0074D270 / 00753180, all over 00752C40 and ProcessClone 00752D80), NiPSysEmitterCtlr (007501F0, 0074FD50,
//! ProcessClone 0074FDB0) and NiPSysUpdateCtlr (00753D60). `clone.rs` calls in for these classes.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::clone::{CloningProcess, clone_or_mapped, copied_time, create_clone, process_clone};
use super::particle_controller::{ParticleEmitterController, ParticleUpdateController};
use super::particle_data::ParticleData;
use super::particle_dynamics::{ParticleAgeDeathModifier, ParticleBoundUpdateModifier};
use super::particle_emitter::ParticleBoxEmitter;
use super::particle_gravity::ParticleGravityModifier;
use super::particle_system::{ParticleSystem, ParticleSystemModifierAttachment};
use super::raw_reference::AvObjectRawRef;
use super::stream_objects::NiClass;
use crate::bs::sky_shader::PrepareGeometryContext;

// IDA: the ProcessClone arms that keep a raw pointer: the clone map's entry, else the original.
fn mapped_or_same(process: &CloningProcess, original: Option<Entity>) -> Option<Entity> {
    original.map(|original| {
        process
            .clones_00
            .get(&original)
            .copied()
            .unwrap_or(original)
    })
}

/// IDA: 00749A70 NiParticleSystem_CopyMembers: NiGeometry's (the properties, the shared data, cleared states), +0xC0
/// copied, each +0xC4 modifier's vt+0x18 AddTail-ed in order, +0xE8 copied, and +0xB4 the data's vt+0x18 in place of the
/// shared one. The constructor (00749EE0) leaves +0xE4 1 and +0xEC 0; the +0xD4 name map is filled by ProcessClone.
/// Completion: 100%.
pub(super) fn clone_system(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    clone: Entity,
) -> Result<(), StreamError> {
    let source = world
        .get::<ParticleSystem>(original)
        .expect("a NiParticleSystem")
        .clone();
    let mut modifiers = Vec::with_capacity(source.modifiers_c4.len());
    for attachment in &source.modifiers_c4 {
        let copy = create_clone(world, process, attachment.owner)?;
        modifiers.push(ParticleSystemModifierAttachment {
            owner: copy,
            original_order: attachment.original_order,
            original_name: attachment.original_name.clone(),
        });
    }
    let data = match source.data_b4 {
        Some(data) => Some(create_clone(world, process, data)?),
        None => None,
    };
    let mut state = world.get_mut::<ParticleSystem>(clone).expect("copied");
    state.modifiers_c4 = modifiers;
    state.data_b4 = data;
    state.byte_e4 = true;
    state.reset_requested_ec = false;
    state.property_state_ac = None;
    state.effect_state_b0 = None;
    Ok(())
}

/// IDA: 00759940 NiPSysData_CopyMembers: new copies of the +0x1C positions, a nonnull +0x20 normals and +0x24 colors
/// through 00728890 (the vertex count, +0x2C's low six bits and 0xFC0 cleared, a null +0x28: no texture coordinates), the
/// +0x50 rotations, +0x4C sizes, +0x44 radii, +0x48 active count, and a nonnull +0x54 angles, +0x58 axes, +0x5C particle
/// info and +0x60 speeds; +0x64 and +0x66; the +0x0C bound. The constructor's (00759D80 over 0073EE80) +0x2E bit 15, a null
/// allocation group and keep/compress bytes. Completion: 100%.
pub(super) fn clone_data(world: &mut World, original: Entity, clone: Entity) {
    let source = world
        .get::<ParticleData>(original)
        .expect("a NiPSysData")
        .clone();
    *world.get_mut::<ParticleData>(clone).expect("copied") = ParticleData {
        geometry_positions_present: source.geometry_positions_present,
        geometry_normals_present: source.geometry_normals_present,
        geometry_colors_present: source.geometry_colors_present,
        vertex_count: source.vertex_count,
        bounds: source.bounds,
        positions: source.positions,
        info_5c: source.info_5c,
        colors: source.colors,
        normals: source.normals,
        radii: source.radii,
        sizes_4c: source.sizes_4c,
        rotations: source.rotations,
        rotation_angles_54: source.rotation_angles_54,
        rotation_axes: source.rotation_axes,
        rotation_speeds_60: source.rotation_speeds_60,
        active_count_48: source.active_count_48,
        added_count: source.added_count,
        added_base: source.added_base,
        ..ParticleData::default()
    };
}

/// IDA: the modifiers' CreateClone over 00752C40 (the +0x08 name, +0x0C order, +0x14 active): each class's fields copied
/// as its CreateClone lists them; the age-death's +0x1C, the gravity's +0x18 and the box emitter's +0x50 wait for
/// ProcessClone; the bound-update modifier runs `NiPSysBoundUpdateModifier_SetSkipCount(+0x18)` (a fresh zeroed cache of
/// at least one sphere, +0x1A zero); the color modifier's `SetData(+0x18)` shares the data and reads the same key times.
/// Completion: 100% for the eight classes.
pub(super) fn clone_modifier(
    world: &mut World,
    class: NiClass,
    clone: Entity,
) -> Result<(), StreamError> {
    match class {
        NiClass::ParticlePositionModifier
        | NiClass::ParticleGrowFadeModifier
        | NiClass::ParticleSpawnModifier
        | NiClass::ParticleColorModifier => {}
        NiClass::ParticleAgeDeathModifier => {
            let mut state = world
                .get_mut::<ParticleAgeDeathModifier>(clone)
                .expect("copied");
            state.spawn_1c = None;
            state.spawn_linked = false;
        }
        NiClass::ParticleGravityModifier => {
            world
                .get_mut::<ParticleGravityModifier>(clone)
                .expect("copied")
                .gravity_object_18 = AvObjectRawRef(None);
        }
        NiClass::ParticleBoxEmitter => {
            world
                .get_mut::<ParticleBoxEmitter>(clone)
                .expect("copied")
                .emitter_object_50 = AvObjectRawRef(None);
        }
        NiClass::ParticleBoundUpdateModifier => {
            let mut state = world
                .get_mut::<ParticleBoundUpdateModifier>(clone)
                .expect("copied");
            let count = usize::from(state.skip_count_18.max(1));
            state.cache_1c = vec![[0.0; 4]; count];
            state.rolling_index_1a = 0;
        }
        _ => {
            return Err(StreamError(
                "00700610: a particle modifier with no clone rule",
            ));
        }
    }
    Ok(())
}

/// IDA: 0074FD50 NiPSysEmitterCtlr_CopyMembers: 0075E410 (006CE2C0: the time fields and the +0x3C interpolator through
/// 00700710; the +0x40 name copied), the +0x48 interpolator's vt+0x18, and with +0x08 bits 1 and 2 clear and a zero +0x10
/// phase, `rand() / 32767.0 * 100.0` (the clone thread's draw). 007501F0's constructor: +0x48 null, +0x50 -FLT_MAX,
/// +0x54..+0x60 zero, +0x44 waiting for ProcessClone. Completion: 100%; the quotient's precision on the clone thread is
/// taken as the IO thread's (53 bits), not captured.
/// TODO: [ ] The clone thread's control word for the phase.
pub(super) fn clone_emitter_controller(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    clone: Entity,
) -> Result<(), StreamError> {
    let source = world
        .get::<ParticleEmitterController>(original)
        .expect("a NiPSysEmitterCtlr")
        .clone();
    let mut time = copied_time(world, process, &source.time)?;
    let birth_rate = match source.birth_rate_interpolator_3c {
        Some(interpolator) => Some(clone_or_mapped(world, process, interpolator)?),
        None => None,
    };
    let active = match source.active_interpolator_48 {
        Some(interpolator) => Some(create_clone(world, process, interpolator)?),
        None => None,
    };
    if time.fields.flags & 6 == 0 && time.fields.phase == 0.0 {
        let draw = world
            .get_resource_or_init::<crate::tes::reference_3d::CloneThreadRand>()
            .0
            .rand();
        time.fields.phase = super::particle_controller::controller_phase(draw, true);
    }
    let cold = ParticleEmitterController::default();
    *world
        .get_mut::<ParticleEmitterController>(clone)
        .expect("copied") = ParticleEmitterController {
        time,
        interpolator_linked: birth_rate.is_some(),
        modifier_name_present: source.modifier_name_present,
        field_48_linked: active.is_some(),
        modifier_name_40: source.modifier_name_40,
        birth_rate_interpolator_3c: birth_rate,
        active_interpolator_48: active,
        ..cold
    };
    Ok(())
}

/// IDA: 00753D60: a new NiPSysUpdateCtlr and `00715D80`'s time fields. Completion: 100%.
pub(super) fn clone_update_controller(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    clone: Entity,
) -> Result<(), StreamError> {
    let source = world
        .get::<ParticleUpdateController>(original)
        .expect("a NiPSysUpdateCtlr")
        .time
        .clone();
    let time = copied_time(world, process, &source)?;
    world
        .get_mut::<ParticleUpdateController>(clone)
        .expect("copied")
        .time = time;
    Ok(())
}

/// IDA: 007495E0 NiParticleSystem's ProcessClone (after NiGeometry's): the +0xD4 name map's entries go into the clone's
/// through the clone map (the host keeps the map by list index, which the in-order clone list preserves), each modifier's
/// vt+0x38, then the +0xB4 data's vt+0x38. Completion: 100%.
pub(super) fn process_system(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<(), StreamError> {
    let source = world
        .get::<ParticleSystem>(original)
        .expect("a NiParticleSystem")
        .clone();
    for attachment in &source.modifiers_c4 {
        process_modifier(world, process, attachment.owner)?;
    }
    if let Some(data) = source.data_b4 {
        process_clone(world, process, data, prepare)?;
    }
    Ok(())
}

// IDA: 00752D80 (the +0x10 target through the clone map; the host keeps no modifier target) and the classes' arms: 0075BEA0's
// +0x1C, 00753800's +0x18 and 00753180's +0x50 through the clone map, else the original's.
fn process_modifier(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
) -> Result<(), StreamError> {
    process.processed_04.insert(original);
    let clone = *process
        .clones_00
        .get(&original)
        .ok_or(StreamError("00752D80: a modifier never cloned"))?;
    if let Some(source) = world.get::<ParticleAgeDeathModifier>(original).cloned() {
        let mapped = mapped_or_same(process, source.spawn_1c);
        let mut state = world
            .get_mut::<ParticleAgeDeathModifier>(clone)
            .expect("cloned");
        state.spawn_1c = mapped;
        state.spawn_linked = source.spawn_linked;
    } else if let Some(source) = world.get::<ParticleGravityModifier>(original).cloned() {
        let mapped = mapped_or_same(process, source.gravity_object_18.0);
        world
            .get_mut::<ParticleGravityModifier>(clone)
            .expect("cloned")
            .gravity_object_18 = AvObjectRawRef(mapped);
    } else if let Some(source) = world.get::<ParticleBoxEmitter>(original).cloned() {
        let mapped = mapped_or_same(process, source.emitter_object_50.0);
        world
            .get_mut::<ParticleBoxEmitter>(clone)
            .expect("cloned")
            .emitter_object_50 = AvObjectRawRef(mapped);
    }
    Ok(())
}

/// IDA: 0074FDB0 NiPSysEmitterCtlr's ProcessClone: 0075E670 (006CE2F0: the next controller's and the +0x3C interpolator's
/// vt+0x38; with targets on both, +0x44 through the clone map), then the +0x48 interpolator's vt+0x38. Completion: 100%.
pub(super) fn process_emitter_controller(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    clone: Entity,
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<(), StreamError> {
    super::clone_animation::process_next(world, process, original, prepare)?;
    let source = world
        .get::<ParticleEmitterController>(original)
        .expect("a NiPSysEmitterCtlr")
        .clone();
    if let Some(interpolator) = source.birth_rate_interpolator_3c {
        process_clone(world, process, interpolator, prepare)?;
    }
    let target = world
        .get::<ParticleEmitterController>(clone)
        .expect("cloned")
        .time
        .target_30;
    if source.time.target_30.is_some() && target.is_some() {
        let cached = source
            .cached_modifier_44
            .and_then(|modifier| process.clones_00.get(&modifier).copied());
        let mut state = world
            .get_mut::<ParticleEmitterController>(clone)
            .expect("cloned");
        state.cached_modifier_44 = cached;
        state.cached_modifier_linked = cached.is_some();
    }
    if let Some(interpolator) = source.active_interpolator_48 {
        process_clone(world, process, interpolator, prepare)?;
    }
    Ok(())
}
