//! NiTimeController: 007155A0, the time mapping and its one shared cache, and the controller chain's own side, the +0x34
//! next (006C61E0) and the +0x30 target (00715CE0). The chain's head and its prepend, remove and search are NiObjectNET's
//! (`object_net.rs`). Every controller's time state is read through `particle_controller::controller_time`.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87::float_fmod;

use super::mesh_geometry::update_geometry_data_flags_2e;
use super::object_net::{controller_head, prepend_controller, remove_controller};
use super::particle_controller::{controller_time, controller_time_mut};
use super::stream_objects::NiClass;

const INVALID_TIME: f32 = -f32::MAX;
const INVALID_LOW: f32 = f32::MAX;

/// IDA: exactly the NiTimeController fields 007155A0 reads and writes (raw floats +0x08..+0x28 and the flags). Cold
/// values are the C++ port's `ParticleTimeControllerCold`, which is owned by the particle family; a controller holds or
/// converts into this.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControllerTimeFields {
    pub flags: u16,
    pub frequency: f32,
    pub phase: f32,
    pub low_key_time: f32,
    pub high_key_time: f32,
    pub start_time: f32,
    pub last_time: f32,
    pub time_accumulator: f32,
}

impl Default for ControllerTimeFields {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            flags: 12,
            frequency: 1.0,
            phase: 0.0,
            low_key_time: INVALID_LOW,
            high_key_time: INVALID_TIME,
            start_time: INVALID_TIME,
            last_time: INVALID_TIME,
            time_accumulator: 0.0,
        }
    }
}

/// IDA: B27130 is disk-initialized cycle 3; B3FC88..94 are zero-filled BSS. There is one cache across all controllers.
/// Host substitution: the C++ port's `std::mutex` is replaced by the exclusive `&mut` borrow of this resource.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ParticleControllerTimeCache {
    pub cycle: u32,
    pub output: f32,
    pub input: f32,
    pub low_key_time: f32,
    pub high_key_time: f32,
}

impl Default for ParticleControllerTimeCache {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            cycle: 3,
            output: 0.0,
            input: 0.0,
            low_key_time: 0.0,
            high_key_time: 0.0,
        }
    }
}

// IDA: 007155A0 float __thiscall (NiTimeController*, float currentTime), vtable +0x64 -> CRT fmod; raw float fields
// +0x08..+0x28
// Completion: 100% of the recovered state, cache, wrap, reverse and clamp order with f32 stores, in the supported host
// arithmetic domain, including the original shared-cache key. Precision: 24-bit per step on the main thread after
// CreateDevice, each fmul/fadd/fsub rounded to a float as it is computed.
// TODO: [ ] Match original x87 FP exception state.
// TODO: [ ] 007155A0 guards its cache with the B3FCA0 critical section: a later phase's worker thread that evaluates a
// controller runs at its own control word (no worker is recorded; 53-bit if it keeps the default).
pub fn evaluate_particle_controller_time(
    controller: &mut ControllerTimeFields,
    current_time: f32,
    cache: &mut ParticleControllerTimeCache,
) -> f32 {
    if controller.start_time == INVALID_TIME {
        controller.start_time = current_time;
    }

    let mut delta = current_time;
    if controller.last_time == INVALID_TIME {
        controller.time_accumulator = 0.0;
        if controller.flags & 1 != 0 {
            delta = 0.0;
        }
    } else {
        delta = current_time - controller.last_time;
    }
    let cycle = u32::from((controller.flags >> 1) & 3);
    controller.time_accumulator += controller.frequency * delta;
    let mut value = controller.time_accumulator + controller.phase;
    controller.last_time = current_time;

    if controller.high_key_time == cache.high_key_time
        && controller.low_key_time == cache.low_key_time
        && value == cache.input
        && cycle == cache.cycle
    {
        return cache.output;
    }

    cache.cycle = cycle;
    cache.high_key_time = controller.high_key_time;
    cache.low_key_time = controller.low_key_time;
    cache.input = value;

    if controller.high_key_time != INVALID_TIME && controller.low_key_time != INVALID_LOW {
        if cycle == 1 {
            let width = controller.high_key_time - controller.low_key_time;
            if width != 0.0 {
                let doubled_width = width + width;
                let mut folded = float_fmod(value, doubled_width);
                if folded < 0.0 {
                    folded += doubled_width;
                }
                // The original's reverse folding uses the input directly, without first subtracting the low key time.
                // That asymmetry is kept.
                let mut reflected = folded;
                if width < reflected {
                    reflected = doubled_width - reflected;
                }
                value = reflected + controller.low_key_time;
            } else {
                value = controller.low_key_time;
            }
        } else if cycle == 0 {
            let width = controller.high_key_time - controller.low_key_time;
            if width == 0.0 {
                value = controller.low_key_time;
            } else {
                let offset = value - controller.low_key_time;
                let wrapped = float_fmod(offset, width);
                value = wrapped + controller.low_key_time;
                if controller.low_key_time > value {
                    value += width;
                }
            }
        }
    }
    if controller.high_key_time < value {
        value = controller.high_key_time;
    } else if controller.low_key_time > value {
        value = controller.low_key_time;
    }
    if controller.flags & 0x10 != 0 {
        value = controller.high_key_time - (value - controller.low_key_time);
    }
    // The original cache key omits playback bit 0x10, so sharing the cache deliberately retains the result of the
    // preceding matching controller.
    cache.output = value;
    value
}

// IDA: N/A the +0x34 of a chain entry; an entity with no time state has none.
// Completion: 100%
#[must_use]
pub(crate) fn controller_next(world: &World, controller: Entity) -> Option<Entity> {
    controller_time(world, controller).and_then(|time| time.next_34)
}

// IDA: N/A whether a link holds a controller. A link to an entity with no host time state counts as null, which ends a walk
// as the original's null +0x34 does.
// Completion: 100%
#[must_use]
pub(crate) fn is_time_controller(world: &World, entity: Option<Entity>) -> bool {
    entity.is_some_and(|entity| controller_time(world, entity).is_some())
}

// IDA: 006C61E0 NiTimeController_SetNext(next): an equal pointer returns (006C61EC); otherwise the old next is released, +0x34
// stored and the new one referenced.
// Completion: 100%; the reference counts have no host form.
pub(crate) fn set_next_controller(world: &mut World, controller: Entity, next: Option<Entity>) {
    let linked = is_time_controller(world, next);
    let mut time = controller_time_mut(world, controller).expect("SetNext on a null controller");
    if time.next_34.is_some() && time.next_34 == next {
        return;
    }
    time.next_linked = linked;
    time.next_34 = next;
}

// IDA: N/A a controller's vtable +0x60 (IsVertexController), by class: 00977C50 (true) on NiGeomMorpherController (A7A23C) and
// NiUVController (A7A7C4), 0069D990 (false) on NiTimeController's own table (A7EB24) and on every other bound class: the
// transform, multi-target, vis, alpha, material colour, flip, texture transform and bone LOD controllers, NiControllerManager,
// bhkBlendController and each NiPSys controller.
// Completion: 100% of the bound classes; NiUVController has no host class.
fn is_vertex_controller(world: &World, controller: Entity) -> bool {
    world.get::<NiClass>(controller) == Some(&NiClass::GeomMorpherController)
}

// IDA: 00715CE0 NiTimeController_SetTarget(target): an equal +0x30 returns (00715CE8); an old target with a nonnull head drops
// this controller (006FFE90); +0x30 = target; a null target ends here. A target whose chain already holds this controller
// returns (00715D22, past the release of the reference taken at entry, an original leak); otherwise 006FFE60 prepends it, and
// with vtable +0x60 (IsVertexController) true the target's +0xB4 data takes +0x2E = (+0x2E & 0x0FFF) | 0x8000 (00715D40).
// Completion: 100% of the branch and store order.
// TODO: [ ] Represent the leaked temporary reference.
/// # Errors
/// A vertex controller (vtable +0x60) whose target has no geometry data, which the original dereferences at 00715D43.
pub fn set_controller_target(
    world: &mut World,
    controller: Entity,
    target: Option<Entity>,
) -> Result<(), GameError> {
    let old = controller_time(world, controller)
        .expect("SetTarget on a null controller")
        .target_30;
    if old == target {
        return Ok(());
    }
    if let Some(old) = old
        && is_time_controller(world, controller_head(world, old))
    {
        remove_controller(world, old, controller);
    }
    let mut time = controller_time_mut(world, controller).expect("checked above");
    time.target_30 = target;
    time.target_linked = target.is_some();
    let Some(target) = target else {
        return Ok(());
    };
    let mut current = controller_head(world, target);
    while let Some(entry) = current.filter(|_| is_time_controller(world, current)) {
        if entry == controller {
            return Ok(());
        }
        current = controller_next(world, entry);
    }
    prepend_controller(world, target, controller);
    if is_vertex_controller(world, controller)
        && !update_geometry_data_flags_2e(world, target, |flags| (flags & 0x0FFF) | 0x8000)
    {
        return Err(GameError::unclassified(
            0x0071_5D43,
            "00715D43: a vertex controller's target with no geometry data faults",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::mesh_geometry::{
        MeshGeometryData, MeshTriShapeData, MeshTriShapePayload, NiGeometry, spawn_mesh_geometry,
    };
    use super::super::morph::GeomMorpherController;
    use super::super::object_net::get_controller;
    use super::super::particle_controller::ParticleUpdateController;
    use super::super::particle_data::ParticleData;
    use super::super::particle_system::{ParticleSystem, spawn_particle_system};
    use super::super::texture::TextureController;
    use super::super::transform_controller::{TransformController, spawn_node_controller};
    use super::*;

    fn update_controller(world: &mut World) -> Entity {
        world
            .spawn((
                NiClass::ParticleUpdateController,
                ParticleUpdateController::default(),
            ))
            .id()
    }

    /// Ported from `particle_controller_tests`' `verify_chain_head_and_interior_splice`, on the one port of 00715CE0. The
    /// C++ fixture gave its first controller a true vtable +0x60; no NiPSys controller has one (0069D990), so the particle
    /// data's +0x2E stays here and `vertex_controller_marks_the_target_data` covers the store on a class that has.
    #[test]
    fn chain_head_and_interior_splice() {
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
        let first = update_controller(&mut world);
        let second = update_controller(&mut world);
        set_controller_target(&mut world, first, Some(first_target)).unwrap();
        set_controller_target(&mut world, second, Some(first_target)).unwrap();
        assert_eq!(controller_head(&world, first_target), Some(second));
        assert_eq!(controller_next(&world, second), Some(first));
        assert_eq!(
            world
                .get::<ParticleData>(data)
                .unwrap()
                .geometry_extra_flags_2e,
            0x1FFF
        );

        // 00715CE8: the same target changes nothing.
        set_controller_target(&mut world, first, Some(first_target)).unwrap();
        assert_eq!(controller_head(&world, first_target), Some(second));
        assert_eq!(controller_next(&world, second), Some(first));

        // An interior entry moves to another target.
        set_controller_target(&mut world, first, Some(second_target)).unwrap();
        assert_eq!(controller_head(&world, first_target), Some(second));
        assert_eq!(controller_next(&world, second), None);
        assert_eq!(controller_head(&world, second_target), Some(first));

        // The head detaches.
        set_controller_target(&mut world, second, None).unwrap();
        assert_eq!(controller_head(&world, first_target), None);
        assert_eq!(controller_next(&world, second), None);
        assert!(!controller_time(&world, second).unwrap().target_linked);
    }

    /// 00715D20..00715D22: a controller already in the new target's chain keeps the +0x30 store and leaves the chain alone.
    #[test]
    fn a_controller_already_in_the_chain_is_not_prepended() {
        let mut world = World::new();
        let target = spawn_particle_system(&mut world);
        let other = update_controller(&mut world);
        let controller = update_controller(&mut world);
        set_controller_target(&mut world, controller, Some(target)).unwrap();
        set_controller_target(&mut world, other, Some(target)).unwrap();
        // The chain is other -> controller; the controller loses its +0x30 only.
        controller_time_mut(&mut world, controller)
            .unwrap()
            .target_30 = None;
        set_controller_target(&mut world, controller, Some(target)).unwrap();
        assert_eq!(
            controller_time(&world, controller).unwrap().target_30,
            Some(target)
        );
        assert_eq!(controller_head(&world, target), Some(other));
        assert_eq!(controller_next(&world, other), Some(controller));
        assert_eq!(controller_next(&world, controller), None);
    }

    /// 00715D33..00715D57: NiGeomMorpherController's vtable +0x60 (00977C50) is true, so its target's +0xB4 data takes
    /// (+0x2E & 0x0FFF) | 0x8000 after the prepend. A target with no data is the original's fault at 00715D43, after the
    /// stores before it.
    #[test]
    fn vertex_controller_marks_the_target_data() {
        let mut world = World::new();
        let shape = spawn_mesh_geometry(
            &mut world,
            NiClass::TriShape,
            MeshTriShapePayload::default(),
        );
        let data = world
            .spawn(MeshTriShapeData {
                base: MeshGeometryData {
                    dirty_flags_2e: 0x1FFF,
                    ..MeshGeometryData::default()
                },
                ..MeshTriShapeData::default()
            })
            .id();
        world.get_mut::<NiGeometry>(shape).unwrap().data_b4 = Some(data);
        let morpher = world
            .spawn((
                NiClass::GeomMorpherController,
                GeomMorpherController::default(),
            ))
            .id();
        set_controller_target(&mut world, morpher, Some(shape)).unwrap();
        assert_eq!(controller_head(&world, shape), Some(morpher));
        assert_eq!(
            world
                .get::<MeshTriShapeData>(data)
                .unwrap()
                .base
                .dirty_flags_2e,
            0x8FFF
        );

        let bare = spawn_mesh_geometry(
            &mut world,
            NiClass::TriShape,
            MeshTriShapePayload::default(),
        );
        let stop = set_controller_target(&mut world, morpher, Some(bare)).unwrap_err();
        assert_eq!(stop.site, 0x0071_5D43);
        assert_eq!(controller_head(&world, shape), None);
        assert_eq!(controller_head(&world, bare), Some(morpher));
    }

    /// One +0x34 chain across the three host holders of NiTimeController state: a node controller, a particle controller and
    /// a texture controller. 006FFE90 takes the interior entry out and 00700010 walks past it.
    #[test]
    fn one_chain_holds_every_controller_family() {
        let mut world = World::new();
        let target = spawn_particle_system(&mut world);
        let transform = spawn_node_controller(
            &mut world,
            NiClass::TransformController,
            TransformController::default(),
        );
        let update = update_controller(&mut world);
        let flip = world
            .spawn((NiClass::FlipController, TextureController::default()))
            .id();
        for controller in [transform, update, flip] {
            set_controller_target(&mut world, controller, Some(target)).unwrap();
        }
        assert_eq!(controller_head(&world, target), Some(flip));
        assert_eq!(controller_next(&world, flip), Some(update));
        assert_eq!(controller_next(&world, update), Some(transform));
        assert_eq!(
            get_controller(&world, target, NiClass::TransformController),
            Some(transform)
        );

        remove_controller(&mut world, target, update);
        assert_eq!(controller_next(&world, flip), Some(transform));
        assert_eq!(controller_next(&world, update), None);
        assert_eq!(
            get_controller(&world, target, NiClass::TransformController),
            Some(transform)
        );
        // 006FFF52: a controller that isn't in the chain changes nothing.
        remove_controller(&mut world, target, update);
        assert_eq!(controller_head(&world, target), Some(flip));
        assert_eq!(controller_next(&world, flip), Some(transform));
    }

    /// 007155F4..0071560E at 24-bit: the frequency product rounds before the accumulator is added. Expected bits from exact
    /// per-step rounding; one rounding over the exact sum (the old `f64` form) gives 0x44B98AD7.
    #[test]
    fn accumulator_rounds_each_step() {
        let mut fields = ControllerTimeFields {
            frequency: f32::from_bits(0x3FB2_8F5C),
            last_time: f32::from_bits(0x3FD9_DB23),
            time_accumulator: f32::from_bits(0x44B7_9571),
            ..ControllerTimeFields::default()
        };
        evaluate_particle_controller_time(
            &mut fields,
            f32::from_bits(0x414E_F1AA),
            &mut ParticleControllerTimeCache::default(),
        );
        assert_eq!(fields.time_accumulator.to_bits(), 0x44B9_8AD6);
    }

    /// Ported from `particle_controller_tests`' `verify_time_cache_and_reverse_low_key`.
    #[test]
    fn time_cache_and_reverse_low_key() {
        let mut cache = ParticleControllerTimeCache::default();
        let mut first = ControllerTimeFields {
            flags: 8,
            frequency: 0.0,
            phase: 3.0,
            low_key_time: 0.0,
            high_key_time: 10.0,
            ..ControllerTimeFields::default()
        };
        assert_eq!(
            evaluate_particle_controller_time(&mut first, 1.0, &mut cache),
            3.0
        );
        let mut reverse = first;
        reverse.flags = 0x18; // playback reverse is omitted from the shared cache key
        assert_eq!(
            evaluate_particle_controller_time(&mut reverse, 1.0, &mut cache),
            3.0
        );
        let mut fresh = ParticleControllerTimeCache::default();
        let mut independent = reverse;
        assert_eq!(
            evaluate_particle_controller_time(&mut independent, 1.0, &mut fresh),
            7.0
        );
        let mut reverse_cache = ParticleControllerTimeCache::default();
        let mut pingpong = ControllerTimeFields {
            flags: 0x0A, // cycle 1
            frequency: 0.0,
            phase: 7.0,
            low_key_time: 2.0,
            high_key_time: 6.0,
            ..ControllerTimeFields::default()
        };
        assert_eq!(
            evaluate_particle_controller_time(&mut pingpong, 0.0, &mut reverse_cache),
            3.0
        );
    }
}
