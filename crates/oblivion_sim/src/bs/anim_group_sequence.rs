//! BSAnimGroupSequence: a NiControllerSequence with its TESAnimGroup (+0x68).

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use crate::gamebryo::clone::CloningProcess;
use crate::gamebryo::controller_sequence::ControllerSequence;
use crate::gamebryo::stream_objects::NiClass;

// IDA: 0049FD90 BSAnimGroupSequence(group, sequence) (FormHeap 0x6C): 006C7FB0(sequence's +0x08 name, 0, 1, 0), vtable A49964, +0x68 the group (counted), +0x20 the text keys, +0x60 the accum root and +0x5C the accum root's name copied, then a cloning process and 006C9420 (the new arrays freed, 006C70A0 NiControllerSequence_CopyMembers from the KF's); +0x60 isn't one CopyMembers writes, so it stays the KF's.
pub(crate) fn new_anim_group_sequence(
    world: &mut World,
    group: Entity,
    source: Entity,
) -> Result<Entity, GameError> {
    let state = world
        .get::<ControllerSequence>(source)
        .expect("a KF's sequence")
        .clone();
    let sequence = world
        .spawn((
            NiClass::ControllerSequence,
            ControllerSequence {
                name_08: state.name_08.clone(),
                array_grow_by_10: 1,
                text_keys_20: state.text_keys_20,
                accum_root_name_5c: state.accum_root_name_5c.clone(),
                ..ControllerSequence::default()
            },
            AnimGroupSequence { group_68: group },
        ))
        .id();
    let mut process = CloningProcess::default();
    crate::gamebryo::clone_animation::clone_sequence(world, &mut process, source, sequence)?;
    world
        .get_mut::<ControllerSequence>(sequence)
        .expect("just spawned")
        .accum_root_60 = state.accum_root_60;
    Ok(sequence)
}

/// IDA: BSAnimGroupSequence's own field: +0x68 the TESAnimGroup.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimGroupSequence {
    pub group_68: Entity,
}

// IDA: 0049F4A0 BSAnimGroupSequence_GetScaledTime(time): a state of 1..3 answers ComputeScaledTime(+0x48 + time, 1) (006C5FC0,
// which stores +0x34, +0x38 and +0x3C); any other 0.0.
pub(crate) fn section_scaled_time(world: &mut World, sequence: Entity, time: f32) -> f32 {
    let mut seq = world
        .get_mut::<ControllerSequence>(sequence)
        .expect("a sequence");
    if !(1..=3).contains(&seq.state_44) {
        return 0.0;
    }
    let at = oblivion_core::x87::stored_float(f64::from(seq.offset_48) + f64::from(time));
    seq.compute_scaled_time(at, true)
}
