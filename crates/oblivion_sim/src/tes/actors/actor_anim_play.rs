//! The actor's animation-group play: `005FCAB0` Actor_UpdateAnimationGroups (the player's arms in the race menu, as run-aw
//! shows them: `records-run-aw-seq.txt` lists every call it makes in three race-menu frames, `records-run-aw-start.txt` the
//! first frame's play), `00477B60` ActorAnimData_StartGroup, `00476260` ActorAnimData_PlayGroup, `00474530`
//! ActorAnimData_PlaySequence, `00470FC0` ActorAnimData_ResetSection, `00470D20` ActorAnimData_FindPlayableGroup and the
//! player's first-person play `0065D790` (vt+0x3A4). run-aw: on the race menu's first frame the player's third-person data
//! plays `Characters\_Male\idle.kf` (Idle, section 0) through `00474530`'s menu-mode arm and `Activate(0, 1, 1.0, 0.0)`, and
//! the first-person data its own idle.kf; later frames find section 0's group already Idle and skip the play.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::actor::{Actor, MobileObject};
use super::actor_animation::{ActorAnimData, AnimSequence};
use super::actor_process::{HighProcess, MiddleHighProcess};
use super::player::PlayerCharacter;
use crate::gamebryo::controller_sequence::{
    ControllerSequence, activate_sequence, deactivate_sequence,
};
use crate::tes::anim_group::anim_group_row;
use crate::tes::tile::{InterfaceCx, TileEngine};

// IDA: N/A the anim data, read.
fn data(world: &World, anim: Entity) -> &ActorAnimData {
    world.get::<ActorAnimData>(anim).expect("an ActorAnimData")
}

// IDA: N/A the anim data, written.
fn data_mut(world: &mut World, anim: Entity) -> Mut<'_, ActorAnimData> {
    world
        .get_mut::<ActorAnimData>(anim)
        .expect("an ActorAnimData")
}

// IDA: 004706E0 ActorAnimData_GetSectionSequence(section): 5 -> +0xA0, 6 -> +0xAC, else +0xA0 + 4 * section.
pub(super) fn section_sequence(world: &World, anim: Entity, section: u32) -> Option<Entity> {
    let index = match section {
        5 => 0,
        6 => 3,
        other => other as usize,
    };
    data(world, anim).sequences_a0.get(index).copied().flatten()
}

// IDA: 00470720 ActorAnimData_GetSectionGroup(section): 5 -> +0x3C, 6 -> +0x42, else +0x3C + 2 * section.
pub(super) fn section_group(world: &World, anim: Entity, section: u32) -> u16 {
    let index = match section {
        5 => 0,
        6 => 3,
        other => other as usize,
    };
    data(world, anim)
        .groups_3c
        .get(index)
        .copied()
        .unwrap_or(0xFF)
}

// IDA: vt+0x10 GetSequence(-1) of the +0x9C entry for `group` (00470960 AnimSequenceMap_GetAt first). A Single answers its
// sequence; a Multiple draws `Random_Next` at 00470C14, which no race-menu frame of run-aq or run-aw does: a stop.
fn group_sequence(world: &World, anim: Entity, group: u16) -> Result<Option<Entity>, GameError> {
    match data(world, anim).lookup(group) {
        None => Ok(None),
        Some(AnimSequence::Single(sequence)) => Ok(*sequence),
        Some(AnimSequence::Multiple(_)) => Err(GameError::unclassified(
            0x00470C14,
            "00470C14: a Multiple group's draw in the group play",
        )),
    }
}

/// IDA: 00470FC0 ActorAnimData_ResetSection(section, time): 5 resets 4 and 0 first, 6 resets 1 and 2 (5 continues into
/// them too) and then section 3; with a manager (+0x98) and a section below 5 whose sequence is active (+0x44): its +0x58
/// partner through `00470B50` and a state-5 (morph) sequence's `006C4480` are stops, then Deactivate(time, 0); the section's
/// sequence = null, its group and queued group = 0xFF, its +0x48 action = -1. Completion: 100% but the two stops.
/// # Errors
/// The partner and morph arms.
pub fn anim_data_reset_section(
    world: &mut World,
    anim: Entity,
    section: u32,
    time: f32,
) -> Result<(), GameError> {
    let section = match section {
        5 => {
            anim_data_reset_section(world, anim, 4, time)?;
            anim_data_reset_section(world, anim, 0, time)?;
            anim_data_reset_section(world, anim, 1, time)?;
            anim_data_reset_section(world, anim, 2, time)?;
            3
        }
        6 => {
            anim_data_reset_section(world, anim, 1, time)?;
            anim_data_reset_section(world, anim, 2, time)?;
            3
        }
        other => other,
    };
    let index = section as usize;
    if data(world, anim).manager_98.is_some()
        && section < 5
        && let Some(sequence) = data(world, anim).sequences_a0[index]
    {
        let state = world
            .get::<ControllerSequence>(sequence)
            .expect("a sequence");
        if state.state_44 != 0 {
            if state.partner_58.is_some() {
                return Err(GameError::unclassified(
                    0x00471044,
                    "00471044: 00470B50 on the section's +0x58 partner",
                ));
            }
            if state.state_44 == 5 {
                return Err(GameError::unclassified(
                    0x0047106D,
                    "0047106D: 006C4480 on a morphing section",
                ));
            }
            deactivate_sequence(world, sequence, time, false);
        }
    }
    if section < 5 {
        let mut state = data_mut(world, anim);
        state.sequences_a0[index] = None;
        state.groups_3c[index] = 0xFF;
        state.queued_groups_70[index] = 0xFF;
        state.actions_48[index] = -1;
    } else {
        return Err(GameError::unclassified(
            0x00471088,
            "00471088: a section past 4 in 00470FC0",
        ));
    }
    Ok(())
}

/// IDA: 00470D20 ActorAnimData_FindPlayableGroup(group, nested) for a group with no hand or movement bits (`& 0xF00` and
/// `& 0xF000` clear, as `005FCAB0`'s player arm makes them): the group when its +0x9C entry has a sequence; otherwise groups
/// 7..10 try 3..6 (nested) and then a nested call answers 0, a top-level one retries with `group & 0xFF00`, and 0 answers 0.
/// Completion: 100% of those arms.
/// # Errors
/// A group with hand or movement bits, or `group_sequence`'s.
pub fn anim_data_find_playable_group(
    world: &World,
    anim: Entity,
    group: u16,
    nested: bool,
) -> Result<u16, GameError> {
    let mut group = group;
    loop {
        if group_sequence(world, anim, group)?.is_some() {
            return Ok(group);
        }
        if group & 0xFF00 != 0 {
            return Err(GameError::unclassified(
                0x00470D5A,
                "00470D5A: 00470D20 with hand or movement bits",
            ));
        }
        if group != 0 {
            let fallback = match group & 0xFF {
                7 => Some(3),
                8 => Some(4),
                9 => Some(5),
                10 => Some(6),
                _ => None,
            };
            if let Some(fallback) = fallback {
                let found = anim_data_find_playable_group(world, anim, fallback, true)?;
                if found == fallback {
                    return Ok(found);
                }
            }
        }
        if nested || group == 0 {
            return Ok(0);
        }
        group &= 0xFF00;
    }
}

// IDA: 00471230(time, out, 1) ActorAnimData_UpdateRootAndAccumRoot: with an accum root (+0x08) its translate (+0x54) =
// +0x18; 0047C990 NiAVObject_UpdateRootAndAccumRoot(+0x04, time, +0x08) (`update_root_and_accum`: the root's controllers but
// the multi-target ones, so a sequence activated this frame takes its first Update here); the accum root's translate read
// back (unused here) and zeroed.
fn anim_data_update_root(world: &mut World, anim: Entity) -> Result<(), GameError> {
    let state = data(world, anim);
    let (node, accum, position, time) = (
        state.node_04,
        state.accum_root_08,
        state.position_18,
        state.time_94,
    );
    if let Some(accum) = accum
        && let Some(mut payload) =
            world.get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(accum)
    {
        payload.local_transform.translation = position;
    }
    let node = node.ok_or(GameError::unclassified(
        0x0047C99F,
        "0047C99F: 00471230 with no root",
    ))?;
    let result = crate::gamebryo::scene_update::update_root_and_accum(
        world,
        node,
        time,
        accum,
        &mut crate::tes::reference_3d::LoadUpdateControllers,
    )?;
    if result.status != crate::gamebryo::scene_update::SceneUpdateStatus::Updated {
        return Err(GameError::unclassified(
            0x00471268,
            "00471268: 0047C990's controller update stopped",
        ));
    }
    if let Some(accum) = accum
        && let Some(mut payload) =
            world.get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(accum)
    {
        payload.local_transform.translation = [0.0; 3];
    }
    Ok(())
}

// IDA: 005EFFD0 Actor_SetCurrentAction(-1, 0) for the player with no current action (process +0x1F4 = -1): 5EFFD0's arms
// for actions 3, 5 and 6 aren't taken; the process's vt+0x2D8 stores +0x1F4 = -1 and +0x1F8 = 0.
fn player_clear_current_action(world: &mut World, actor: Entity) -> Result<(), GameError> {
    let Some(process) = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
    else {
        return Ok(());
    };
    let Some(mut high) = world.get_mut::<HighProcess>(process) else {
        return Err(GameError::unclassified(
            0x005F0194,
            "005F0194: SetCurrentAction on a process below High",
        ));
    };
    if high.current_action_1f4 != 0xFFFF {
        return Err(GameError::unclassified(
            0x005EFFE6,
            "005EFFE6: 005EFFD0 with a current action",
        ));
    }
    high.current_action_1f4 = 0xFFFF;
    high.field_1f8 = 0;
    Ok(())
}

/// IDA: 00474530 ActorAnimData_PlaySequence(sequence, group, section): section -1 takes the group's (00B102E8; 5 -> 0, 6 -> 3);
/// the section's current sequence and its state; a null sequence or group 0xFF answers null; a state-3 sequence is
/// deactivated; a sequence neither inactive nor cycling (+0x24) outside menu mode's player root sets the section's +0x48
/// action to 0 and answers it. Otherwise: in menu mode on the player's third-person root (and no save-load thread) the
/// sections 4, 0, 1 and 2 are reset (00470FC0 at 0.0), section 3 likewise inline (`00474662`), `005EFFD0`(player, -1, 0) and
/// `004728C0` ActorAnimData_ResetAccumRoot; with the section's old sequence active (state 1) a different section, +0xC4 or
/// the new sequence's state resets the section; else the section is reset. The section's group and sequence are stored;
/// matching movement notes and controller counts select a morph outside the player's menu root. Otherwise an active
/// source cross-fades. The largest +0x21 frame count or fAnimationDefaultBlend sets the duration, with the menu and
/// +0xC4 overrides applied before division by fAnimationMult. Below 0.01 the sequence activates directly.
/// Completion: the direct activation, morph, cross-fade and off-camera fallback arms.
/// # Errors
/// A visible pose blend, an unresolved first-person biped root, or an unsupported section reset.
pub fn anim_data_play_sequence(
    world: &mut World,
    tiles: &TileEngine,
    anim: Entity,
    sequence: Option<Entity>,
    group: u16,
    section: i32,
) -> Result<Option<Entity>, GameError> {
    let info = anim_group_row(group).ok_or(GameError::unclassified(
        0x00474563,
        "00474563: a group past the table",
    ))?;
    let original = if section == -1 {
        info.section_08
    } else {
        section as u32
    };
    let section = match original {
        5 => 0,
        6 => 3,
        other => other,
    };
    if section >= 5 {
        return Err(GameError::unclassified(
            0x004745AF,
            "004745AF: a section past 4 in 00474530",
        ));
    }
    let index = section as usize;
    let old = data(world, anim).sequences_a0[index];
    let old_state = old.map_or(0, |old| {
        world
            .get::<ControllerSequence>(old)
            .expect("a sequence")
            .state_44
    });
    let Some(sequence) = sequence else {
        return Ok(None);
    };
    if group == 0xFF {
        return Ok(None);
    }
    if world
        .get::<ControllerSequence>(sequence)
        .expect("a sequence")
        .state_44
        == 3
    {
        deactivate_sequence(world, sequence, 0.0, false);
    }
    let player = crate::tes::player::the_player(world);
    let third_root = player.and_then(|player| {
        world
            .get::<crate::tes::reference_3d::Reference3D>(player)
            .map(|root| root.0)
    });
    let node = data(world, anim).node_04;
    let menu = crate::tes::interface_manager::is_menu_mode(world);
    let on_player_root = menu && player.is_some() && node == third_root;
    let seq = world
        .get::<ControllerSequence>(sequence)
        .expect("a sequence");
    if seq.state_44 != 0 && seq.cycle_type_24 == 0 && !on_player_root {
        data_mut(world, anim).actions_48[index] = 0;
        return Ok(Some(sequence));
    }
    let reset_player_sections =
        on_player_root && !crate::tes::forms_references::save_load_active(world)?;
    if reset_player_sections {
        for reset in [4, 0, 1, 2] {
            anim_data_reset_section(world, anim, reset, 0.0)?;
        }
        if data(world, anim).manager_98.is_some()
            && let Some(third) = data(world, anim).sequences_a0[3]
        {
            let state = world.get::<ControllerSequence>(third).expect("a sequence");
            if state.state_44 != 0 {
                if state.partner_58.is_some() {
                    return Err(GameError::unclassified(
                        0x004746A2,
                        "004746A2: 00470B50 on section 3's partner",
                    ));
                }
                if state.state_44 == 5 {
                    return Err(GameError::unclassified(
                        0x004746B8,
                        "004746B8: 006C4480 on a morphing section 3",
                    ));
                }
                deactivate_sequence(world, third, 0.0, false);
            }
        }
        {
            let mut state = data_mut(world, anim);
            state.sequences_a0[3] = None;
            state.groups_3c[3] = 0xFF;
            state.queued_groups_70[3] = 0xFF;
            state.actions_48[3] = -1;
        }
        player_clear_current_action(world, player.expect("the player's root"))?;
        super::equipment_3d::reset_accum_root(world, anim)?;
    } else if old_state == 1 {
        let byte_c4 = data(world, anim).byte_c4;
        let new_state = world
            .get::<ControllerSequence>(sequence)
            .expect("a sequence")
            .state_44;
        if original != section || byte_c4 != 0 || new_state != 0 {
            anim_data_reset_section(world, anim, original, 0.0)?;
        }
    } else {
        anim_data_reset_section(world, anim, original, 0.0)?;
    }
    {
        let mut state = data_mut(world, anim);
        state.groups_3c[index] = group;
        state.sequences_a0[index] = Some(sequence);
    }
    // EDI is cleared by the reset when the old sequence was inactive. The
    // player menu arm retains its entry but skips the movement-note morph check.
    let old = old.filter(|_| reset_player_sections || old_state == 1);
    let anim_group = |s| {
        world
            .get::<crate::bs::anim_group_sequence::AnimGroupSequence>(s)
            .and_then(|s| world.get::<super::anim_group::TesAnimGroup>(s.group_68))
    };
    let next_group = anim_group(sequence).expect("a BSAnimGroupSequence");
    let old_group = old.and_then(anim_group);
    let morph = !on_player_root
        && old_group
            .is_some_and(|g| g.move_byte_20 != 0 && g.move_byte_20 == next_group.move_byte_20)
        && old.is_some_and(|source| {
            source != sequence
                && world
                    .get::<ControllerSequence>(source)
                    .unwrap()
                    .array_size_0c
                    == world
                        .get::<ControllerSequence>(sequence)
                        .unwrap()
                        .array_size_0c
        });
    let frames = old_group.map_or(0, |g| g.blend_21).max(next_group.blend_21);
    let ini = world.get_resource::<crate::tes::settings::SettingCollections>();
    let setting = |address, image| {
        ini.and_then(|s| s.ini.find_address(address))
            .map_or(image, |s| s.as_float())
    };
    let mut blend = if frames != 0 {
        f32::from(frames) / 30.0
    } else {
        setting(0x00B06538, 0.1)
    };
    let player_data = player.and_then(|p| world.get::<PlayerCharacter>(p));
    if menu && player_data.is_some_and(|p| p.first_person_biped_root_5d8 != 0) {
        return Err(GameError::stop(
            0x0047489F,
            "resolve the first-person biped root for animation blending",
        ));
    }
    if (menu && node.is_none())
        || crate::tes::interface_manager::menu_is_visible_in(world, tiles, 1036)
    {
        blend = setting(0x00B06540, 0.0);
    }
    if data(world, anim).byte_c4 != 0 {
        blend = 0.0;
    }
    blend /= setting(0x00B06530, 1.0);
    let first_person = player_data.and_then(|p| p.first_person_node_5d0);
    let mut seq = world.get_mut::<ControllerSequence>(sequence).unwrap();
    seq.offset_48 = seq.begin_key_time_2c + 0.0;
    if blend < 0.01 {
        activate_sequence(world, sequence, 0, true, 1.0, 0.0, None, false);
    } else if morph {
        crate::gamebryo::controller_sequence::start_morph(
            world,
            old.unwrap(),
            Some(sequence),
            blend,
            0,
            1.0,
            1.0,
        );
    } else {
        let cross_faded = old.is_some_and(|source| {
            if world.get::<ControllerSequence>(source).unwrap().state_44 == 0
                || world.get::<ControllerSequence>(sequence).unwrap().state_44 != 0
            {
                return false;
            }
            deactivate_sequence(world, source, blend, false);
            activate_sequence(world, sequence, 0, true, 1.0, blend, None, false)
        });
        if !cross_faded {
            let from_pose = node == first_person
                || node == third_root
                || (super::actor_movement_update::node_is_in_actor_frustum(world, node)?
                    && world
                        .resource::<crate::tes::idle_frame::IdleFrameGlobals>()
                        .havok_step_flag_b333b8
                        == 0);
            if from_pose {
                return Err(GameError::stop(
                    0x006C5C70,
                    "blend a visible actor from its current pose",
                ));
            }
            activate_sequence(world, sequence, 0, true, 1.0, 0.0, None, false);
        }
    }
    if info.kind_0c <= 7 {
        data_mut(world, anim).actions_48[index] = 0;
    }
    if data(world, anim).byte_c4 != 0 {
        data_mut(world, anim).byte_c4 = 0;
        anim_data_update_root(world, anim)?;
    }
    Ok(Some(sequence))
}

/// IDA: 00476260 ActorAnimData_PlayGroup(group, section): group 0xFF or no +0x9C entry answers null; else the entry's
/// vt+0x10(-1) into `00474530`. Completion: 100%.
/// # Errors
/// As `anim_data_play_sequence`, or a Multiple's draw.
pub fn anim_data_play_group(
    world: &mut World,
    tiles: &TileEngine,
    anim: Entity,
    group: u16,
    section: i32,
) -> Result<Option<Entity>, GameError> {
    if group == 0xFF || data(world, anim).lookup(group).is_none() {
        return Ok(None);
    }
    let sequence = group_sequence(world, anim, group)?;
    anim_data_play_sequence(world, tiles, anim, sequence, group, section)
}

/// IDA: 00477B60 ActorAnimData_StartGroup(group, mode, loops): the group's section (5 -> 0, 6 -> 3); group 0xFF does
/// nothing. Kinds 0 and 1: mode 0 queues it (+0x70 = group, +0x7C = loops); mode 1 clears the queue (+0x70 = 0xFF), +0x5C =
/// loops, plays it (`00476260`, -1) and runs `00471230` at +0x94; another mode does nothing. Kinds 2..7 play it and run
/// `00471230`; others nothing. Completion: 100%.
/// # Errors
/// As `anim_data_play_group`.
pub fn anim_data_start_group(
    world: &mut World,
    tiles: &TileEngine,
    anim: Entity,
    group: u16,
    mode: i32,
    loops: i32,
) -> Result<(), GameError> {
    let Some(info) = anim_group_row(group) else {
        return Ok(());
    };
    let section = match info.section_08 {
        5 => 0,
        6 => 3,
        other => other,
    } as usize;
    match info.kind_0c {
        0 | 1 => match mode {
            0 => {
                let mut state = data_mut(world, anim);
                state.queued_groups_70[section] = group;
                state.next_loops_7c[section] = loops;
            }
            1 => {
                {
                    let mut state = data_mut(world, anim);
                    state.queued_groups_70[section] = 0xFF;
                    state.loops_5c[section] = loops;
                }
                anim_data_play_group(world, tiles, anim, group, -1)?;
                anim_data_update_root(world, anim)?;
            }
            _ => {}
        },
        2..=7 => {
            anim_data_play_group(world, tiles, anim, group, -1)?;
            anim_data_update_root(world, anim)?;
        }
        _ => {}
    }
    Ok(())
}

/// IDA: 0065D790 PlayerCharacter vt+0x3A4 (group, mode): for a group that can't be multiple (00B102E4 clear), the
/// first-person data (+0x5CC) with an entry for the group starts it (`00477B60`(group, mode, -1)). A multiple group's
/// `00474BE0` test on the third-person data is a stop. Completion: 100% of the single arm.
/// # Errors
/// The multiple arm, or `anim_data_start_group`'s.
pub fn player_start_first_person_group(
    world: &mut World,
    tiles: &TileEngine,
    player: Entity,
    group: u16,
    mode: i32,
) -> Result<(), GameError> {
    let info = anim_group_row(group).ok_or(GameError::unclassified(
        0x0065D79C,
        "0065D79C: a group past the table",
    ))?;
    if info.multiple_04 {
        return Err(GameError::unclassified(
            0x0065D7C6,
            "0065D7C6: 00474BE0 on a multiple group",
        ));
    }
    let Some(first) = world
        .get::<PlayerCharacter>(player)
        .and_then(|data| data.first_person_anim_data_5cc)
    else {
        return Err(GameError::unclassified(
            0x0065D7E3,
            "0065D7E3: 00470D00 on a null first-person data",
        ));
    };
    if data(world, first).lookup(group).is_some() {
        anim_data_start_group(world, tiles, first, group, mode, -1)?;
    }
    Ok(())
}

/// IDA: 005FCAB0 Actor_UpdateAnimationGroups(a, b) for the player as run-aw shows it in the race menu (thiscall, `retn 8`):
/// vt+0x198 IsDead(0) or the player with +0x588 clear return; vt+0x164's data, the controller (`0065A2C0`) and the process
/// must be there; process vt+0x8 (ReturnZero for a High process) runs `00633460` (+0x1E0's timer: a positive one is a
/// stop); vt+0x388 (ReturnZero) isn't the player. Section 0's sequence; vt+0x168 (unused here); `005E0EE0` (the process's
/// +0x1F4 action) -1 takes the default case. vt+0x214 (+0x648) nonzero, vt+0x18C (the process's +0x11D sleep state)
/// nonzero, an action 2..5, the controller's context state (+0x1EC) 1 or 2, a section-0 sequence of type 0x28 or 0x29,
/// the process's +0x114 or +0x115, an action 6 and the movement flags (+0x1FC) with any of 0x3F or 0x200 are stops; with
/// them clear: vt+0x27C answers 2; the horse (+0xD4) is none; the sleep state's switch takes its default and the player
/// with the race menu (1036) in the interface's menu stack (`0057CFB0`) sets the group to 1 (DynamicIdle). The group is
/// `0051A9B0`(movement 0, hand 0, group) = the group, through `00470D20`; with no current action, a group below 3, and a
/// section group different from it with an entry: `00477B60`(group, 1, -1); then the player's vt+0x3A4(group, 1); group
/// (below 3, never 0x28); section 2's sequence and the process's lit light (vt+0xF0(1)) are stops; section 3's sequence is a stop.
/// Completion: 100% of the arms run-aw's player takes.
/// TODO: [ ] 005FCAB0's other arms (combat, movement, swimming, sitting, sleeping, a horse, a light, other actors).
/// # Errors
/// Every arm above that's a stop.
pub fn actor_update_animation_groups(cx: &mut InterfaceCx, actor: Entity) -> Result<(), GameError> {
    update_animation_groups(cx.world, cx.engine, actor)
}

// IDA: 005FCAB0 Actor_UpdateAnimationGroups(1, 1) on a living actor.
pub(super) fn update_animation_groups(
    world: &mut World,
    tiles: &TileEngine,
    actor: Entity,
) -> Result<(), GameError> {
    let dead = world.get::<Actor>(actor).expect("an actor").dead_state_b0;
    if matches!(dead, 1 | 2 | 6) {
        return Ok(());
    }
    let player = world.get::<PlayerCharacter>(actor);
    if player.is_some_and(|p| p.view_flags_588[0] == 0) {
        return Ok(());
    }
    let field_648 = player.map_or(0, |p| p.field_648);
    let is_player = player.is_some();
    let horse = world.get::<Actor>(actor).unwrap().horse_or_rider_d4;
    let anim = if is_player {
        super::actor_movement_update::player_anim_data(world, actor)
    } else {
        world
            .get::<MobileObject>(actor)
            .and_then(|m| m.process_58)
            .and_then(|process| world.get::<MiddleHighProcess>(process))
            .and_then(|p| p.anim_data_17c)
    };
    let Some(anim) = anim else { return Ok(()) };
    let Some(controller) = super::character_controller::actor_character_controller(world, actor)
    else {
        return Ok(());
    };
    let Some(process) = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
    else {
        return Ok(());
    };
    let Some(high) = world.get::<HighProcess>(process) else {
        return Err(GameError::unclassified(
            0x005FCB39,
            "005FCB39: 005FCAB0 with a process below High",
        ));
    };
    // 005FCB4A 00633460: the +0x1E0 timer.
    if high.movement_flags_timer_1e0 > 0.0 {
        let seconds = world
            .resource::<crate::tes::script_vm::ScriptVmGlobals>()
            .frame_seconds_b33e9c;
        let awake = world
            .get::<MiddleHighProcess>(process)
            .unwrap()
            .sleep_state_11d
            == 0;
        let mut high = world.get_mut::<HighProcess>(process).unwrap();
        high.movement_flags_timer_1e0 -= seconds;
        if awake {
            high.movement_flags_1fc |= high.timed_movement_flags_1e4 as i8 as i16 as u16;
        }
        if high.movement_flags_timer_1e0 <= 0.0 {
            high.movement_flags_1fc &= !0x30;
        }
    }
    let high = world.get::<HighProcess>(process).unwrap();
    let (action, movement) = (high.current_action_1f4, high.movement_flags_1fc);
    let middle = world
        .get::<MiddleHighProcess>(process)
        .expect("a High process");
    let (sleep, weapon_draw_requested_114, weapon_drawn_115, light) = (
        middle.sleep_state_11d,
        middle.weapon_draw_requested_114,
        middle.weapon_drawn_115,
        middle.light_e8.is_some(),
    );
    let section_0 = section_sequence(world, anim, 0);
    if action != 0xFFFF {
        return Err(GameError::unclassified(
            0x005FCBC3,
            "005FCBC3: 005FCAB0's action switch",
        ));
    }
    if field_648 != 0 {
        return Err(GameError::unclassified(
            0x005FD7F8,
            "005FD7F8: 005FCAB0 with +0x648 set",
        ));
    }
    if sleep != 0 {
        return Err(GameError::unclassified(
            0x005FD912,
            "005FD912: 005FCAB0 with a sleep state",
        ));
    }
    let context = world
        .get::<crate::bs::character_controller::BhkCharacterController>(controller)
        .expect("the process's controller")
        .context_state_1ec;
    if context != 0 {
        return Err(GameError::unclassified(
            0x005FD942,
            "005FD942: 005FCAB0's swimming or flying context",
        ));
    }
    if let Some(sequence) = section_0 {
        let group = world
            .get::<crate::bs::anim_group_sequence::AnimGroupSequence>(sequence)
            .map(|seq| seq.group_68);
        if let Some(group) = group {
            let kind = world
                .get::<super::anim_group::TesAnimGroup>(group)
                .map(super::anim_group::TesAnimGroup::group_type);
            if matches!(kind, Some(0x28 | 0x29)) {
                return Err(GameError::unclassified(
                    0x005FD9C6,
                    "005FD9C6: 005FCAB0's section-0 group 0x28 or 0x29",
                ));
            }
        }
    }
    if weapon_draw_requested_114 != 0 || weapon_drawn_115 != 0 {
        return Err(GameError::unclassified(
            0x005FDA2E,
            "005FDA2E: 005FCAB0 with the process's +0x114 or +0x115",
        ));
    }
    // 005FDC16..005FDDDF: 0x800, 0x2000 and 0x400 pick the hand; the low four bits a direction (with 0x200 a fast one) and
    // 0x10 / 0x20 a turn.
    if movement & 0x2E3F != 0 {
        return Err(GameError::unclassified(
            0x005FDC13,
            "005FDC13: 005FCAB0 with movement flags",
        ));
    }
    if horse != 0 {
        return Err(GameError::unclassified(
            0x005FDC64,
            "005FDC64: 005FCAB0 with a horse",
        ));
    }
    // 005FDCAB: the player, with 1036 in InterfaceManager +0xE0's ten entries (0057CFB0), takes group 1.
    let in_stack = world
        .get_resource::<crate::tes::interface_manager::InterfaceManager>()
        .is_some_and(|manager| manager.menu_mode_stack_0e0.contains(&1036));
    let picked: u16 = u16::from(is_player && in_stack);
    // 005FDE44 0051A9B0(0, 0, group): the group itself.
    let group = anim_data_find_playable_group(world, anim, picked, false)?;
    // 005FDE64..005FE0B8: no current action; a group below 3 (not 0xF or 0x10) takes neither arm.
    if group & 0xFF >= 3 {
        return Err(GameError::unclassified(
            0x005FDFD0,
            "005FDFD0: 005FCAB0 with a group of 3 or more",
        ));
    }
    let info = anim_group_row(group).expect("below 3");
    // 005FE0CA: a section group equal to it, or no entry (00470D00), skips to 005FE178.
    if section_group(world, anim, info.section_08) != group
        && data(world, anim).lookup(group).is_some()
    {
        anim_data_start_group(world, tiles, anim, group, 1, -1)?;
        // 005FE0EF: no current action; vt+0x3A4(group, 1); the group isn't 0x28.
        if is_player {
            player_start_first_person_group(world, tiles, actor, group, 1)?;
        }
    }
    if light {
        return Err(GameError::unclassified(
            0x005FE194,
            "005FE194: 005FCAB0 with a light",
        ));
    }
    if section_sequence(world, anim, 2).is_some() {
        return Err(GameError::unclassified(
            0x005FE232,
            "005FE232: 005FCAB0 with a section-2 sequence",
        ));
    }
    if section_sequence(world, anim, 3).is_some() {
        return Err(GameError::unclassified(
            0x005FE283,
            "005FE283: 005FCAB0 with a section-3 sequence",
        ));
    }
    Ok(())
}
