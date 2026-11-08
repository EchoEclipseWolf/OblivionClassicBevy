//! NiObject_Clone for the animation classes a NiControllerManager holds: the manager (006C5CC0, 006C5A10, ProcessClone
//! 006C4C20), NiControllerSequence (006C87A0, 006C70A0, ProcessClone 006C7470), NiDefaultAVObjectPalette (007169C0,
//! ProcessClone 00716460), NiMultiTargetTransformController (006D02E0, 006D0170, ProcessClone 006D01D0), the blend
//! interpolators (006CC400, 006EB7B0, 006D27A0 over 006CD3D0, ProcessClone 006CCBC0), the bool interpolators (006E84B0,
//! 006E7DE0 over 006E82F0) and NiVisController (006D45A0). `clone.rs` calls in for these classes.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::clone::{CloningProcess, clone_or_mapped, copied_time, create_clone, process_clone};
use super::controller_sequence::{
    ControllerManager, ControllerSequence, SequenceBlendRef, SequenceInterpItem, add_sequence,
    store_targets,
};
use super::particle_blend_interpolator::{
    BlendInterpolator, ParticleBlendBoolInterpolator, ParticleBlendFloatInterpolator,
    ParticleBlendItemPayload, ParticleBlendTransformInterpolator,
};
use super::transform_controller::{MultiTargetTransformController, NodeController};
use crate::bs::sky_shader::PrepareGeometryContext;

// IDA: 00700710's lookup alone: the clone map's entry, else the original (the ProcessClone arms that keep a raw pointer).
fn mapped_or_same(process: &CloningProcess, original: Option<Entity>) -> Option<Entity> {
    original.map(|original| {
        process
            .clones_00
            .get(&original)
            .copied()
            .unwrap_or(original)
    })
}

/// IDA: 006C5A10 NiControllerManager_CopyMembers: `00715D80`'s time fields (the target through the clone map, the next
/// controller's vt+0x18), +0x6C cumulative; each nonnull sequence below +0x46 through its vt+0x18 and
/// `NiControllerManager_AddSequence(clone, null, 0)`; the +0x7C palette's vt+0x18. The constructor's (006C5520) empty
/// arrays and map, +0x70..+0x78 zero (the host keeps no +0x70 list: a loaded manager's is empty). Completion: 100%.
pub(super) fn clone_manager(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    clone: Entity,
) -> Result<(), StreamError> {
    let time = world
        .get::<NodeController>(original)
        .expect("a NiControllerManager has time state")
        .time
        .clone();
    let copied = copied_time(world, process, &time)?;
    world.get_mut::<NodeController>(clone).expect("copied").time = copied;
    let source = world
        .get::<ControllerManager>(original)
        .expect("a NiControllerManager")
        .clone();
    *world.get_mut::<ControllerManager>(clone).expect("copied") = ControllerManager {
        cumulative_6c: source.cumulative_6c,
        ..ControllerManager::default()
    };
    for slot in source
        .sequence_slots_40
        .iter()
        .take(usize::from(source.sequence_size_46))
        .copied()
    {
        let Some(sequence) = slot else { continue };
        let copy = create_clone(world, process, sequence)?;
        let status = add_sequence(world, clone, Some(copy), None, false)?;
        if status != super::controller_sequence::ControllerManagerAddStatus::Added {
            return Err(StreamError("006C5A6E: AddSequence of a cloned sequence"));
        }
    }
    if let Some(palette) = source.object_palette_7c {
        let copy = create_clone(world, process, palette)?;
        world
            .get_mut::<ControllerManager>(clone)
            .expect("copied")
            .object_palette_7c = Some(copy);
    }
    Ok(())
}

/// IDA: 006C70A0 NiControllerSequence_CopyMembers: the +0x08 name, +0x0C and +0x10, new item and tag arrays of +0x0C; per item
/// with an interpolator: with an owner (+0x40) the interpolator through 00700710 and +0x0C copied, without one the
/// interpolator's and the controller's vt+0x18; +0x0D always; the tag copied (006C67F0: the palette shared, the five
/// offsets). +0x1C, the +0x20 text keys (shared), +0x24..+0x30, the +0x5C accum root name and the +0x64 string palette
/// (shared); the rest the constructor's (006C6550). Completion: 100%.
pub(crate) fn clone_sequence(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    clone: Entity,
) -> Result<(), StreamError> {
    let source = world
        .get::<ControllerSequence>(original)
        .expect("a NiControllerSequence")
        .clone();
    let owned = source.manager_40.is_some();
    let count = source.array_size_0c as usize;
    let mut items = vec![SequenceInterpItem::default(); count];
    for (index, item) in source.items_14.iter().enumerate().take(count) {
        if let Some(interpolator) = item.interpolator_00 {
            if owned {
                items[index].interpolator_00 = Some(clone_or_mapped(world, process, interpolator)?);
                items[index].byte_0c = item.byte_0c;
            } else {
                items[index].interpolator_00 = Some(create_clone(world, process, interpolator)?);
                if let Some(controller) = item.controller_04 {
                    items[index].controller_04 = Some(create_clone(world, process, controller)?);
                }
            }
        }
        items[index].priority_0d = item.priority_0d;
    }
    *world.get_mut::<ControllerSequence>(clone).expect("copied") = ControllerSequence {
        name_08: source.name_08,
        array_size_0c: source.array_size_0c,
        array_grow_by_10: source.array_grow_by_10,
        items_14: items,
        id_tags_18: source.id_tags_18,
        weight_1c: source.weight_1c,
        text_keys_20: source.text_keys_20,
        cycle_type_24: source.cycle_type_24,
        frequency_28: source.frequency_28,
        begin_key_time_2c: source.begin_key_time_2c,
        end_key_time_30: source.end_key_time_30,
        accum_root_name_5c: source.accum_root_name_5c,
        string_palette_64: source.string_palette_64,
        links_resolved: source.links_resolved,
        ..ControllerSequence::default()
    };
    Ok(())
}

/// IDA: 007169C0: a new NiDefaultAVObjectPalette (006C5360 with no scene, an empty map). Completion: 100%.
pub(super) fn clone_palette(world: &mut World, clone: Entity) {
    *world
        .get_mut::<super::av_object_palette::DefaultAvObjectPalette>(clone)
        .expect("copied") = super::av_object_palette::DefaultAvObjectPalette::default();
}

/// IDA: 006D0170 NiMultiTargetTransformController_CopyMembers: the time fields, SetInterpCount(+0x44) (006D0010: fresh
/// blends, null targets), then each blend's copy (006CC480 over 006CD3D0). Completion: 100%.
pub(super) fn clone_multi_target(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    clone: Entity,
) -> Result<(), StreamError> {
    let time = world
        .get::<NodeController>(original)
        .expect("a NiMultiTargetTransformController has time state")
        .time
        .clone();
    let copied = copied_time(world, process, &time)?;
    world.get_mut::<NodeController>(clone).expect("copied").time = copied;
    let source = world
        .get::<MultiTargetTransformController>(original)
        .expect("a NiMultiTargetTransformController")
        .clone();
    let mut blends = Vec::with_capacity(source.blend_interpolators_3c.len());
    for blend in &source.blend_interpolators_3c {
        let mut copy = ParticleBlendTransformInterpolator::default();
        copy_blend(world, process, &mut blend.clone(), &mut copy)?;
        blends.push(copy);
    }
    let mut state = MultiTargetTransformController::default();
    state.set_interpolator_count(source.interpolator_count_44);
    for (slot, blend) in state.blend_interpolators_3c.iter_mut().zip(blends) {
        *slot = blend;
    }
    *world
        .get_mut::<MultiTargetTransformController>(clone)
        .expect("copied") = state;
    Ok(())
}

/// IDA: 006CD3D0 NiBlendInterpolator_CopyMembers: +0x0C flags and +0x0D capacity, a new item array of +0x0D, +0x1C the weight
/// threshold; a blend not manager-controlled (flags bit 0 clear) also clones each item's interpolator through its vt+0x18
/// and copies the item fields, +0x0E, +0x0F, +0x20, +0x10 and +0x11. +0x18 waits for ProcessClone. Completion: 100%.
pub(super) fn copy_blend(
    world: &mut World,
    process: &mut CloningProcess,
    source: &mut dyn BlendInterpolator,
    target: &mut dyn BlendInterpolator,
) -> Result<(), StreamError> {
    let (base, sources, _) = source.slots();
    let (base, sources) = (base.clone(), sources.clone());
    let capacity = usize::from(base.capacity_0d);
    let mut items = vec![ParticleBlendItemPayload::default(); capacity];
    let mut item_sources: Vec<Option<Entity>> = vec![None; capacity];
    let managed = base.flags_0c & 1 != 0;
    if !managed {
        for index in 0..capacity {
            if let Some(Some(interpolator)) = sources.get(index) {
                item_sources[index] = Some(create_clone(world, process, *interpolator)?);
            }
            if let Some(item) = base.items_14.get(index) {
                items[index] = *item;
            }
        }
    }
    let (cold, target_sources, single) = target.slots();
    cold.flags_0c = base.flags_0c;
    cold.capacity_0d = base.capacity_0d;
    cold.weight_threshold_1c = base.weight_threshold_1c;
    cold.items_14 = items;
    *target_sources = item_sources;
    *single = None;
    if !managed {
        cold.active_count_0e = base.active_count_0e;
        cold.single_index_0f = base.single_index_0f;
        cold.single_time_20 = base.single_time_20;
        cold.high_priority_10 = base.high_priority_10;
        cold.second_priority_11 = base.second_priority_11;
        cold.single_interpolator_index = base.single_interpolator_index;
    }
    Ok(())
}

/// IDA: the three blend classes' CreateClone (006CC400, 006EB7B0 with +0x30 copied, 006D27A0 with +0x30 copied) over a
/// cold blend. Completion: 100%.
pub(super) fn clone_blend(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    clone: Entity,
) -> Result<(), StreamError> {
    if let Some(mut source) = world
        .get::<ParticleBlendBoolInterpolator>(original)
        .cloned()
    {
        let mut copy = ParticleBlendBoolInterpolator::default();
        copy_blend(world, process, &mut source, &mut copy)?;
        copy.value_30 = source.value_30;
        *world
            .get_mut::<ParticleBlendBoolInterpolator>(clone)
            .expect("copied") = copy;
    } else if let Some(mut source) = world
        .get::<super::material_controllers::BlendPoint3Interpolator>(original)
        .cloned()
    {
        // 006EAC40: +0x30..+0x38 the value and +0x3C copied over 006EA8C0's invalid point.
        let mut copy = super::material_controllers::BlendPoint3Interpolator::default();
        copy_blend(world, process, &mut source, &mut copy)?;
        copy.value_bits_30 = source.value_bits_30;
        copy.valid_3c = source.valid_3c;
        *world
            .get_mut::<super::material_controllers::BlendPoint3Interpolator>(clone)
            .expect("copied") = copy;
    } else if let Some(mut source) = world
        .get::<ParticleBlendFloatInterpolator>(original)
        .cloned()
    {
        let mut copy = ParticleBlendFloatInterpolator::default();
        copy_blend(world, process, &mut source, &mut copy)?;
        copy.value_30 = source.value_30;
        *world
            .get_mut::<ParticleBlendFloatInterpolator>(clone)
            .expect("copied") = copy;
    } else {
        let mut source = world
            .get::<ParticleBlendTransformInterpolator>(original)
            .expect("a NiBlendTransformInterpolator")
            .clone();
        let mut copy = ParticleBlendTransformInterpolator::default();
        copy_blend(world, process, &mut source, &mut copy)?;
        *world
            .get_mut::<ParticleBlendTransformInterpolator>(clone)
            .expect("copied") = copy;
    }
    Ok(())
}

/// IDA: 006E84B0 / 006E7DE0: a new NiBoolInterpolator (+0x0C 2, +0x10 and +0x14 0; the timeline's +0x18 and +0x1C 0), then
/// 006E82F0: +0x0C, the +0x10 data (shared) and +0x14 copied; +0x08 keeps the constructor's. Completion: 100%.
pub(super) fn clone_bool_interpolator(world: &mut World, original: Entity, clone: Entity) {
    use super::bool_animation::{BoolInterpolator, BoolTimelineInterpolator};
    if let Some(source) = world.get::<BoolTimelineInterpolator>(original).cloned() {
        let copy = BoolTimelineInterpolator {
            base: BoolInterpolator {
                cached_value_0c: source.base.cached_value_0c,
                data_10: source.base.data_10,
                last_key_index: source.base.last_key_index,
                ..BoolInterpolator::default()
            },
            previous_key_index: 0,
            previous_raw_value: 0,
        };
        *world
            .get_mut::<BoolTimelineInterpolator>(clone)
            .expect("copied") = copy;
        return;
    }
    let source = *world
        .get::<BoolInterpolator>(original)
        .expect("a NiBoolInterpolator");
    *world.get_mut::<BoolInterpolator>(clone).expect("copied") = BoolInterpolator {
        cached_value_0c: source.cached_value_0c,
        data_10: source.data_10,
        last_key_index: source.last_key_index,
        ..BoolInterpolator::default()
    };
}

/// IDA: 006C4C20 NiControllerManager's ProcessClone, on the original: 00715E40 (the next controller's vt+0x38), then each
/// sequence below +0x46: its vt+0x38 (006C7470) and `NiControllerSequence_StoreTargets(sequence, +0x30)` on the original's
/// own (its items are bound already, so it changes nothing); the +0x70 list (empty here); the palette's vt+0x38.
/// Completion: 100%.
pub(super) fn process_manager(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<(), StreamError> {
    process_next(world, process, original, prepare)?;
    let source = world
        .get::<ControllerManager>(original)
        .expect("a NiControllerManager")
        .clone();
    let target = world
        .get::<NodeController>(original)
        .expect("a NiControllerManager has time state")
        .time
        .target_30;
    for slot in source
        .sequence_slots_40
        .iter()
        .take(usize::from(source.sequence_size_46))
        .copied()
    {
        let Some(sequence) = slot else { continue };
        process_sequence(world, process, sequence, prepare)?;
        let _ = store_targets(world, sequence, target);
    }
    if let Some(palette) = source.object_palette_7c {
        process_palette(world, process, palette)?;
    }
    Ok(())
}

/// IDA: 006C7470 NiControllerSequence's ProcessClone: per item with an interpolator: without an owner the interpolator's and
/// the controller's vt+0x38; with one, an active sequence (+0x44 nonzero) takes the interpolator through the clone map (else
/// the original's), an inactive one runs its vt+0x38; then the controller and the +0x08 blend through the clone map, else the
/// original's (a multi-target slot is inside the original controller and stays). Then +0x40, +0x58 and +0x60 likewise.
/// Completion: 100%.
fn process_sequence(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<(), StreamError> {
    process.processed_04.insert(original);
    let clone = *process
        .clones_00
        .get(&original)
        .ok_or(StreamError("006C7470: a sequence never cloned"))?;
    let source = world
        .get::<ControllerSequence>(original)
        .expect("a NiControllerSequence")
        .clone();
    for (index, item) in source.items_14.iter().enumerate() {
        let Some(interpolator) = item.interpolator_00 else {
            continue;
        };
        if source.manager_40.is_none() {
            process_clone(world, process, interpolator, prepare)?;
            if let Some(controller) = item.controller_04 {
                process_clone(world, process, controller, prepare)?;
            }
            continue;
        }
        if source.state_44 != 0 {
            let mapped = mapped_or_same(process, Some(interpolator));
            world
                .get_mut::<ControllerSequence>(clone)
                .expect("cloned")
                .items_14[index]
                .interpolator_00 = mapped;
        } else {
            process_clone(world, process, interpolator, prepare)?;
        }
        let controller = mapped_or_same(process, item.controller_04);
        let blend = match item.blend_08 {
            SequenceBlendRef::Transform(blend) => {
                SequenceBlendRef::Transform(mapped_or_same(process, Some(blend)).expect("some"))
            }
            SequenceBlendRef::Bool(blend) => {
                SequenceBlendRef::Bool(mapped_or_same(process, Some(blend)).expect("some"))
            }
            SequenceBlendRef::Float(blend) => {
                SequenceBlendRef::Float(mapped_or_same(process, Some(blend)).expect("some"))
            }
            SequenceBlendRef::Point3(blend) => {
                SequenceBlendRef::Point3(mapped_or_same(process, Some(blend)).expect("some"))
            }
            other => other,
        };
        let mut state = world.get_mut::<ControllerSequence>(clone).expect("cloned");
        state.items_14[index].controller_04 = controller;
        state.items_14[index].blend_08 = blend;
    }
    let mut state = world.get_mut::<ControllerSequence>(clone).expect("cloned");
    state.manager_40 = mapped_or_same(process, source.manager_40);
    state.partner_58 = mapped_or_same(process, source.partner_58);
    state.accum_root_60 = mapped_or_same(process, source.accum_root_60);
    Ok(())
}

/// IDA: 00716460 NiDefaultAVObjectPalette's ProcessClone: the clone's +0x1C scene through the clone map (a scene not in it
/// leaves the lookup's out holding the clone palette: a stop); each entry whose object the map holds goes in through vt+0x50
/// SetAVObject, in the map's bucket order. Completion: 100%.
fn process_palette(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
) -> Result<(), StreamError> {
    use super::av_object_palette::DefaultAvObjectPalette;
    process.processed_04.insert(original);
    let clone = *process
        .clones_00
        .get(&original)
        .ok_or(StreamError("00716460: a palette never cloned"))?;
    let source = world
        .get::<DefaultAvObjectPalette>(original)
        .expect("a NiDefaultAVObjectPalette")
        .clone();
    if let Some(scene) = source.scene() {
        let mapped = process
            .clones_00
            .get(&scene)
            .copied()
            .ok_or(StreamError("007164A3: a palette scene outside the clone"))?;
        world
            .get_mut::<DefaultAvObjectPalette>(clone)
            .expect("cloned")
            .set_scene(Some(mapped));
    }
    for bucket in source.buckets() {
        for entry in bucket {
            if let Some(&mapped) = process.clones_00.get(&entry.value) {
                world
                    .get_mut::<DefaultAvObjectPalette>(clone)
                    .expect("cloned")
                    .set_av_object(&entry.key, Some(mapped));
            }
        }
    }
    Ok(())
}

/// IDA: 006D01D0 NiMultiTargetTransformController's ProcessClone: 00715E40, then per blend below +0x44 its ProcessClone
/// (006CCBC0) and a nonnull target through the clone map (null when it isn't there). Completion: 100%.
pub(super) fn process_multi_target(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<(), StreamError> {
    process_next(world, process, original, prepare)?;
    let clone = *process
        .clones_00
        .get(&original)
        .ok_or(StreamError("006D01D0: a controller never cloned"))?;
    let mut source = world
        .get::<MultiTargetTransformController>(original)
        .expect("a NiMultiTargetTransformController")
        .clone();
    for index in 0..usize::from(source.interpolator_count_44) {
        let single = process_blend_items(
            world,
            process,
            &mut source.blend_interpolators_3c[index],
            prepare,
        )?;
        let target = source.targets_40.get(index).copied().flatten();
        let mapped = target.and_then(|target| process.clones_00.get(&target).copied());
        let mut state = world
            .get_mut::<MultiTargetTransformController>(clone)
            .expect("cloned");
        let (_, _, slot) = state.blend_interpolators_3c[index].slots();
        *slot = single;
        if target.is_some() {
            state.targets_40[index] = mapped;
        }
    }
    Ok(())
}

// IDA: 006CCBC0's walk: each item interpolator's vt+0x38; the answer is +0x18 through the clone map, else the original's.
fn process_blend_items(
    world: &mut World,
    process: &mut CloningProcess,
    blend: &mut dyn BlendInterpolator,
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<Option<Entity>, StreamError> {
    let (base, sources, single) = blend.slots();
    let capacity = usize::from(base.capacity_0d);
    let (sources, single) = (sources.clone(), *single);
    for interpolator in sources.into_iter().take(capacity).flatten() {
        process_clone(world, process, interpolator, prepare)?;
    }
    Ok(mapped_or_same(process, single))
}

/// IDA: 006CCBC0 NiBlendInterpolator's ProcessClone for an entity blend: the items' vt+0x38, the clone's +0x18. Completion: 100%.
pub(super) fn process_blend(
    world: &mut World,
    process: &mut CloningProcess,
    original: Entity,
    clone: Entity,
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<(), StreamError> {
    macro_rules! blend {
        ($kind:ty) => {{
            let mut source = world.get::<$kind>(original).expect("a blend").clone();
            let single = process_blend_items(world, process, &mut source, prepare)?;
            let mut state = world.get_mut::<$kind>(clone).expect("cloned");
            let (_, _, slot) = state.slots();
            *slot = single;
        }};
    }
    if world
        .get::<ParticleBlendBoolInterpolator>(original)
        .is_some()
    {
        blend!(ParticleBlendBoolInterpolator);
    } else if world
        .get::<super::material_controllers::BlendPoint3Interpolator>(original)
        .is_some()
    {
        blend!(super::material_controllers::BlendPoint3Interpolator);
    } else if world
        .get::<ParticleBlendFloatInterpolator>(original)
        .is_some()
    {
        blend!(ParticleBlendFloatInterpolator);
    } else {
        blend!(ParticleBlendTransformInterpolator);
    }
    Ok(())
}

/// IDA: 00715E40 NiTimeController's ProcessClone: the +0x34 next controller's vt+0x38. Completion: 100%.
pub(super) fn process_next(
    world: &mut World,
    process: &mut CloningProcess,
    controller: Entity,
    prepare: &mut PrepareGeometryContext<'_>,
) -> Result<(), StreamError> {
    let next = super::controller_sequence::next_of(world, controller);
    if let Some(next) = next {
        process_clone(world, process, next, prepare)?;
    }
    Ok(())
}

// IDA: N/A a single-interpolator controller's +0x3C, by its host component.
pub(super) fn single_interpolator(world: &World, controller: Entity) -> Option<Entity> {
    world
        .get::<super::transform_controller::TransformController>(controller)
        .map(|held| held.interpolator_3c)
        .or_else(|| {
            world
                .get::<super::transform_controller::VisController>(controller)
                .map(|held| held.interpolator_3c)
        })
        .or_else(|| {
            world
                .get::<super::material_controllers::MaterialController>(controller)
                .map(|held| held.interpolator_3c)
        })
        .or_else(|| {
            world
                .get::<super::texture::TextureController>(controller)
                .map(|held| held.interpolator_3c)
        })
        .flatten()
}
