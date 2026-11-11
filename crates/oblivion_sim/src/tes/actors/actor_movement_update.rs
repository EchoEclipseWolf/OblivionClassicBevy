//! `00603CA0` Actor_UpdateMovementAndAnimation(time), the arms `0066EAF0`'s camera block takes for the player (run-ak's
//! CALL lines: the first call with +0x588 = 1, the second with +0x588 = 0), with `00660130`, `00471C00`, `00471F20` and the
//! timer head of `00603500` Actor_UpdateHeadTracking. run-al: the first-person root's and Camera01's world transforms change
//! only in the second call (the root's placement and `00471F20`'s update); Camera01's and Bip's locals stay constant.
//!
//! **Counted boundaries** (`MovementUpdateBoundaries`): the process virtuals the arms call (vt+0x2F4, vt+0x440, vt+0x2D0,
//! vt+0x318, vt+0x424, vt+0x8, vt+0x18C), `004D7FC0`'s light data, `005E6CD0`, the named-bone lookups and the look target
//! (vt+0x4BC/vt+0x4CC, `005F7900`), which run-aa shows make no draw here. `0047CA30` (the KF pose at +0x94) runs since
//! 2026-11-09 (`anim_data_update_multi_targets`).

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::actor::{Actor, MobileObject, TesObjectRefr};
use super::player::PlayerCharacter;
use crate::gamebryo::av_object::anim_data_update_multi_targets;
use crate::tes::anim_group::{section_movement_limit, section_sets_power_attack_flag};
use crate::tes::forms_references::is_flying_creature;

/// IDA: N/A the counts of the calls `00603CA0`'s player arms count rather than make.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct MovementUpdateBoundaries {
    pub calls: std::collections::BTreeMap<u32, u32>,
}

// IDA: N/A a helper, with no address of its own.
fn count(world: &mut World, address: u32) {
    *world
        .get_resource_or_init::<MovementUpdateBoundaries>()
        .calls
        .entry(address)
        .or_default() += 1;
}

/// IDA: 00B14FB4 (00660130's eased hand-follow multiplier, 1.0 in the image) and 00B3CBD0 (00603CA0's and 00471C00's
/// byte).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct MovementUpdateGlobals {
    pub hand_follow_b14fb4: f32,
    pub byte_b3cbd0: u8,
}

impl Default for MovementUpdateGlobals {
    fn default() -> Self {
        Self {
            hand_follow_b14fb4: 1.0,
            byte_b3cbd0: 0,
        }
    }
}

// IDA: N/A a game setting's float (SettingInfo_FloatValueOrZero).
fn setting_float(world: &World, name: &str) -> Result<f32, GameError> {
    world
        .get_resource::<crate::tes::game_settings::GameSettingCollection>()
        .and_then(|settings| settings.float_named(name))
        .ok_or(GameError::unclassified(
            0x00603CA0,
            "00603CA0: a missing game setting",
        ))
}

// IDA: N/A the frame's seconds, g_timeInfo +0x0C (00B33E9C).
pub(crate) fn frame_time(world: &World) -> f32 {
    world
        .get_resource::<crate::tes::script_vm::ScriptVmGlobals>()
        .map_or(0.0, |globals| globals.frame_seconds_b33e9c)
}

/// IDA: 0065D720 PlayerCharacter vt+0x164: +0x5DC when set; else with +0x588 set TESObjectREFR_GetAnimData (004D8370, a
/// MiddleHigh process's +0x17C), else +0x5CC or, null, GetAnimData. Completion: 100% (+0x5DC is 0 in the port).
#[must_use]
pub fn player_anim_data(world: &World, player: Entity) -> Option<Entity> {
    let data = world.get::<PlayerCharacter>(player)?;
    let third = || {
        world
            .get::<MobileObject>(player)
            .and_then(|mobile| mobile.process_58)
            .and_then(|process| world.get::<super::actor_process::MiddleHighProcess>(process))
            .and_then(|middle| middle.anim_data_17c)
    };
    if data.view_flags_588[0] != 0 {
        return third();
    }
    data.first_person_anim_data_5cc.or_else(third)
}

/// IDA: 005E12B0 Actor_GetFaceGenAnimationData, 0064B350 MiddleHighProcess_GetFaceGenAnimationData.
/// +0x15C is lazy and the virtual GetNiNode returning null clears it. 004D66B0 calls
/// vt+0x138(0), which forwards to vt+0x134. 004DDFD0 searches the raw +0x3C by name
/// for a Character or player. Creature's vt+0x27C answers zero. Completion: 100% for these types.
pub fn actor_face_animation_data(world: &mut World, actor: Entity) -> Option<Entity> {
    use crate::tes::actor_process::MiddleHighProcess;
    let process = crate::tes::actor::process_of(world, actor)?;
    let cached = world
        .get::<MiddleHighProcess>(process)?
        .face_animation_data_15c;
    let face = if crate::tes::forms_init_world::reference_ni_node(world, actor).is_none() {
        None
    } else if cached.is_some() {
        return cached;
    } else if world.get::<super::actor::Character>(actor).is_some() {
        world
            .get::<crate::tes::reference_3d::Reference3D>(actor)
            .and_then(|root| {
                crate::gamebryo::object_net::object_by_name_with_palette(
                    world,
                    root.0,
                    b"BSFaceGenNiNodeSkinned",
                )
            })
            .and_then(|node| world.get::<crate::tes::face_gen_nodes::BsFaceGenNiNode>(node))
            .and_then(|node| node.animation_data_dc)
    } else {
        None
    };
    world
        .get_mut::<MiddleHighProcess>(process)?
        .face_animation_data_15c = face;
    face
}

/// IDA: 005E1370 Actor_SetRenderVisibilityFlags(set, bits) on +0xC4. Completion: 100%.
fn set_render_visibility(world: &mut World, actor: Entity, set: bool, bits: u32) {
    let mut data = world.get_mut::<Actor>(actor).expect("an actor");
    if set {
        data.render_visibility_flags_c4 |= bits;
    } else {
        data.render_visibility_flags_c4 &= !bits;
    }
}

/// IDA: 00471C00 ActorAnimData_UpdateSequences(actor) for the player: +0x04, +0x08 or +0x00 zero returns. The player's
/// visible arm (`this == g_thePlayer` skips the death and slot tests): +0xC4 bit 1 set; BSShaderAccumulator's occlusion
/// test (the renderer's, counted: not occluded) sets bit 2; the root's BSFadeNode alpha (+0xE8, which the port doesn't
/// hold: taken as above 1e-5) sets bit 4. vt+0x27C answers 2 for the player, so v10 = the player and vt+0x380 (no horse)
/// leaves 00B3CBD0 = 1; then 0047CA30(+0x94, +0x08) (`anim_data_update_multi_targets`), 00B3CBD0 = 0 and +0x00 = 0.
/// Completion: 100% of the player's arm.
/// # Errors
/// A horse.
pub fn anim_data_update_sequences(
    world: &mut World,
    anim: Entity,
    actor: Entity,
) -> Result<(), GameError> {
    let Some(data) = world.get::<super::actor_animation::ActorAnimData>(anim) else {
        return Ok(());
    };
    if data.node_04.is_none() || data.accum_root_08.is_none() || data.updated_00 == 0 {
        return Ok(());
    }
    let root = data.node_04.expect("checked animation root");
    let player = crate::tes::player::the_player(world) == Some(actor);
    let reference_node = crate::tes::forms_init_world::reference_ni_node(world, actor);
    let alpha = reference_node
        .and_then(|n| world.get::<crate::bs::fade_node::BsFadeNode>(n))
        .map(|f| f.current_alpha_e8);
    let bypass_cull = !player
        && (world.get::<Actor>(actor).expect("an actor").dead_state_b0 == 1
            || crate::tes::actor_package::actor_in_combat_package(world, actor, true)?);
    if !bypass_cull
        && (alpha.is_some_and(|v| v <= 0.0)
            || !player && !node_is_in_actor_frustum(world, Some(root))?)
    {
        set_render_visibility(world, actor, false, 1);
        let reference = world.get::<TesObjectRefr>(actor).expect("a reference");
        let mut center = reference.position_2c;
        let radius = crate::tes::form_components::form_bound_radius(world, reference.base_form_1c);
        let diameter = radius + radius;
        center[2] += 0.25 * diameter;
        let mut state = world
            .get_mut::<crate::gamebryo::scene_node::NiAvWorld>(root)
            .expect("a world bound");
        state.bound[..3].copy_from_slice(&center);
        if state.bound[3] < diameter {
            state.bound[3] = diameter;
        }
        world
            .get_mut::<super::actor_animation::ActorAnimData>(anim)
            .unwrap()
            .updated_00 = 0;
        return Ok(());
    }
    set_render_visibility(world, actor, true, 1);
    count(world, 0x007A_A3C0);
    set_render_visibility(world, actor, true, 2);
    set_render_visibility(world, actor, !alpha.is_some_and(|v| v < 0.00001), 4);
    if world
        .get::<PlayerCharacter>(actor)
        .is_some_and(|data| data.last_ridden_horse_1e0 != 0)
    {
        return Err(GameError::unclassified(
            0x00471D3A,
            "00471D3A: the player's horse in 00471C00",
        ));
    }
    if !player
        && world
            .get::<Actor>(actor)
            .expect("an actor")
            .horse_or_rider_d4
            != 0
    {
        return Err(GameError::unclassified(
            0x00471D80,
            "update the rider and mount skin nodes",
        ));
    }
    // A Creature without a rider leaves the global alone until the reset after the update.
    if world.get::<super::actor::Creature>(actor).is_none() {
        world
            .get_resource_or_init::<MovementUpdateGlobals>()
            .byte_b3cbd0 = 1;
    }
    let time = world
        .get::<super::actor_animation::ActorAnimData>(anim)
        .unwrap()
        .time_94;
    anim_data_update_multi_targets(world, root, time)?;
    world
        .get_resource_or_init::<MovementUpdateGlobals>()
        .byte_b3cbd0 = 0;
    world
        .get_mut::<super::actor_animation::ActorAnimData>(anim)
        .expect("the anim data")
        .updated_00 = 0;
    Ok(())
}

/// IDA: 00471F20 ActorAnimData_UpdateWorld: with +0x04 and +0x08, the two skin partitions held and cleared
/// (NiSkinInstance_SetPartition, no skinning in the port), NiAVObject_Update(+0x04, +0x94, true), then restored. The
/// FaceGen heads under the tree run their update (`face_gen_node_update`) before it: **host substitution** for
/// BSFaceGenNiNode's UpdateDownwardPass (`0055D4C0`), whose draws are the only ones in the update. Completion: 100% of
/// the transforms; the KF controllers run as the port's scene update runs them.
/// # Errors
/// A stop in the update.
pub fn anim_data_update_world(world: &mut World, anim: Entity) -> Result<(), GameError> {
    let Some(data) = world.get::<super::actor_animation::ActorAnimData>(anim) else {
        return Ok(());
    };
    let (Some(root), Some(_)) = (data.node_04, data.accum_root_08) else {
        return Ok(());
    };
    let time = data.time_94;
    let mut stack = vec![root];
    let mut faces = Vec::new();
    while let Some(node) = stack.pop() {
        if world
            .get::<crate::tes::face_gen_nodes::BsFaceGenNiNode>(node)
            .is_some()
        {
            faces.push(node);
        }
        if let Some(children) = world.get::<crate::gamebryo::scene_node::NiNodeChildren>(node) {
            stack.extend(children.slots_ac.iter().rev().flatten());
        }
    }
    for face in faces {
        crate::tes::face_gen_nodes::face_gen_node_update(world, face, time, true)?;
    }
    crate::tes::reference_3d::update_with_controllers(world, root, time)
}

/// IDA: 00603500 Actor_UpdateHeadTracking(time), its head as far as the scan timer's draw: no process, process vt+0x8
/// nonzero, no FaceGen animation data (process vt+0xE8) or no +0x3C return; the three named bones (counted); with any of
/// +0xC4's bits 1, 2, 4 clear +0xE0 = 0, the animation data's +0x1D5 = 0, return; +0xE0 clear sets the data's +0x1D4 = 1
/// and +0xE0 = 1; beyond fAIMaxHeadTrackDistanceFromPC from the player +0x1D5 = 0 and return (the player is 0 from
/// itself); the look target's refresh (vt+0x4CC, counted); +0x100 and +0xDC lose the frame time, floats; +0x100 <= 0 adds
/// Random_FloatInRange(0.2, 0.4) (0047E060 at 006036CB), then vt+0x4BC's target pick (counted: run-aa shows no 4..10 draw
/// after it), then `actor_update_head_tracking_tail`. Completion: 100% of the player's arms.
/// TODO: [ ] 00603500 for other actors: the look target pick, 005F7900's turn, the expression (vt+0xD0) and 0047CAF0.
/// # Errors
/// No host draw.
pub fn actor_update_head_tracking(world: &mut World, actor: Entity) -> Result<(), GameError> {
    if world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
        .is_none()
    {
        return Ok(());
    }
    count(world, 0x0060_3519);
    let Some(face) = actor_face_animation_data(world, actor) else {
        return Ok(());
    };
    if world
        .get::<crate::tes::reference_3d::Reference3D>(actor)
        .is_none()
    {
        return Ok(());
    }
    count(world, 0x0060_3560);
    let flags = world
        .get::<Actor>(actor)
        .expect("an actor")
        .render_visibility_flags_c4;
    if !flags & 7 != 0 {
        world
            .get_mut::<Actor>(actor)
            .expect("an actor")
            .head_track_visibility_active_e0 = 0;
        if let Some(mut data) =
            world.get_mut::<crate::tes::face_gen_nodes::BsFaceGenAnimationData>(face)
        {
            data.bytes_1d4[1] = 0;
        }
        return Ok(());
    }
    if world
        .get::<Actor>(actor)
        .expect("an actor")
        .head_track_visibility_active_e0
        == 0
    {
        if let Some(mut data) =
            world.get_mut::<crate::tes::face_gen_nodes::BsFaceGenAnimationData>(face)
        {
            data.bytes_1d4[0] = 1;
        }
        world
            .get_mut::<Actor>(actor)
            .expect("an actor")
            .head_track_visibility_active_e0 = 1;
    }
    if crate::tes::player::the_player(world) != Some(actor) {
        return Err(GameError::unclassified(
            0x006037F0,
            "006037F0: the head-tracking distance for an actor other than the player",
        ));
    }
    count(world, 0x0060_3631);
    let time = frame_time(world);
    let scan = {
        let mut data = world.get_mut::<Actor>(actor).expect("an actor");
        data.head_track_target_scan_timer_100 =
            x87::stored_float(f64::from(data.head_track_target_scan_timer_100) - f64::from(time));
        data.head_track_target_hold_timer_dc =
            x87::stored_float(f64::from(data.head_track_target_hold_timer_dc) - f64::from(time));
        data.head_track_target_scan_timer_100
    };
    if scan <= 0.0 {
        let draw = random_float_in_range(world, 0.2, 0.4)?;
        {
            let mut data = world.get_mut::<Actor>(actor).expect("an actor");
            data.head_track_target_scan_timer_100 =
                x87::stored_float(draw + f64::from(data.head_track_target_scan_timer_100));
        }
        count(world, 0x0060_36EA);
    }
    count(world, 0x0060_37E4);
    actor_update_head_tracking_tail(world, actor, face, time)
}

/// IDA: 00603500's tail for the player (from 006037E4): `005F7900` answers false for the player, so no body turn; the
/// expression (anim vt+0xD0, from `005E0DD0` or 50.0, building the +0x0C keyframe) is counted; the player has no look
/// target (process vt+0x4CC), so the direction is zero: anim vt+0x84 (`face_gen_set_look_direction`); `0054A0A0`(not
/// Actor_HasDirectionalMovement) into +0x198 (the port's player moves by no flags: 1); the three bones' vt+0x74 (counted);
/// `0054B010` (`face_gen_track_head`) with "Bip01 Head" under "Bip01 Spine2" under "Bip01 NonAccum" (vt+0x58 on the 3D);
/// with +0x1D5 set and bDisableHeadTracking (00B148F4) clear `0054B3E0` (`face_gen_ease_body_turn`) and `0047CAF0` (the
/// spine's share of the turn, counted). Completion: the player's arms; a look target or a body turn would be stops.
/// # Errors
/// As `face_gen_track_head`.
fn actor_update_head_tracking_tail(
    world: &mut World,
    actor: Entity,
    face: Entity,
    time: f32,
) -> Result<(), GameError> {
    use crate::tes::face_gen_nodes::{
        BsFaceGenAnimationData, face_gen_ease_body_turn, face_gen_set_look_direction,
        face_gen_track_head,
    };
    count(world, 0x0060_3858);
    {
        let mut data = world
            .get_mut::<BsFaceGenAnimationData>(face)
            .expect("FaceGen animation data");
        face_gen_set_look_direction(&mut data, [0.0; 3]);
        // 00603A7A: 0054A0A0(!Actor_HasDirectionalMovement).
        data.byte_198 = 1;
    }
    count(world, 0x0060_3AA8);
    let root = world
        .get::<crate::tes::reference_3d::Reference3D>(actor)
        .map(|loaded| loaded.0)
        .ok_or(GameError::unclassified(
            0x00603519,
            "00603519: an actor with no 3D",
        ))?;
    let find = |world: &World, node: Entity, name: &[u8]| {
        crate::bs::model_loader::object_by_name(world, node, name)
    };
    let head = find(world, root, b"Bip01 NonAccum")
        .and_then(|non_accum| find(world, non_accum, b"Bip01 Spine2"))
        .and_then(|spine| find(world, spine, b"Bip01 Head"))
        .ok_or(GameError::unclassified(
            0x00603570,
            "00603570: the head-tracking bones",
        ))?;
    face_gen_track_head(world, face, head)?;
    let active = world
        .get::<BsFaceGenAnimationData>(face)
        .expect("FaceGen animation data")
        .bytes_1d4[1]
        != 0;
    let disabled = world
        .get_resource::<crate::tes::settings::SettingCollections>()
        .and_then(|settings| settings.ini.find_address(0x00B1_48F4))
        .is_some_and(|setting| setting.raw as u8 != 0);
    if active && !disabled {
        let mut data = world
            .get_mut::<BsFaceGenAnimationData>(face)
            .expect("FaceGen animation data");
        face_gen_ease_body_turn(&mut data, time);
        count(world, 0x0047_CAF0);
    }
    Ok(())
}

/// IDA: 0047E060 Random_FloatInRange(a, b): a + (b - a) * rand() / 32767.0 at the main thread's 24-bit precision (each step
/// rounded to single precision), stored as a float (the seed test is the CRT's).
/// Completion: 100%.
/// # Errors
/// No host draw.
pub fn random_float_in_range(world: &mut World, low: f32, high: f32) -> Result<f64, GameError> {
    let draw = world
        .get_non_send::<crate::tes::form_arms::ScriptRandom>()
        .map(|random| random.1.clone())
        .ok_or(GameError::unclassified(
            0x0047E060,
            "0047E060: a draw with no host",
        ))?;
    let value = draw();
    // 0047E089..0047E0A4 at the main thread's 24-bit precision: each x87 step rounds to single precision.
    let span = high - low;
    let scaled = span * value as f32;
    let fraction = scaled / 32767.0_f32;
    Ok(f64::from(fraction + low))
}

/// IDA: 0047FB70 Random_Chance(p): p >= (1.0 - 0.0) * rand() / 32767.0 + 0.0, stored as a float. Completion: 100%.
/// # Errors
/// No host draw.
pub fn random_chance(world: &mut World, chance: f32) -> Result<bool, GameError> {
    let draw = world
        .get_non_send::<crate::tes::form_arms::ScriptRandom>()
        .map(|random| random.1.clone())
        .ok_or(GameError::unclassified(
            0x0047FB70,
            "0047FB70: a draw with no host",
        ))?;
    // 0047FB99..0047FBAE at 24-bit precision: (1.0 - 0.0) x rand is exact, the division rounds to single precision.
    let value = draw() as f32 / 32767.0_f32 + 0.0;
    Ok(chance >= value)
}

/// IDA: 00660130 PlayerCharacter_UpdateFirstPersonRotation: v19 = fFirstPersonHandFollowMult; sections 3 and 1's groups
/// (00470720, 0051AA00) with the weapon state (process vt+0x2D0, counted) pick 1.0 for groups 17..26 or 34..39; v19 unlike
/// 00B14FB4 eases it over fFirstPersonScaleSeconds (0.1 toward 1.0) by the frame time, clamped at v19; the Z rotation
/// (vt PlayerCharacter_GetZRotation; a sitting arm, vt+0x?? with sleep state 4, is counted as not taken) and 0070FDD0 times
/// 0070FD30(+0x20); its (0, 1, 0) column dotted with itself less z, clamped to [-1, 1], acos, times 1 - v19, negated for a
/// negative z; the product with that X rotation is +0x5D0's +0x30. Completion: 100% with the groups unset.
/// # Errors
/// A set group.
pub fn player_update_first_person_rotation(
    world: &mut World,
    player: Entity,
) -> Result<(), GameError> {
    use crate::gamebryo::transform::multiply;
    let mut follow = setting_float(world, "fFirstPersonHandFollowMult")?;
    // 0066014B TESObjectREFR_GetAnimData (004D8370: a MiddleHigh process's +0x17C).
    let third = world
        .get::<MobileObject>(player)
        .and_then(|mobile| mobile.process_58)
        .and_then(|process| world.get::<super::actor_process::MiddleHighProcess>(process))
        .and_then(|middle| middle.anim_data_17c);
    if let Some(anim) = third {
        let low = |section: usize| {
            world
                .get::<super::actor_animation::ActorAnimData>(anim)
                .map_or(0xFF, |data| (data.groups_3c[section] & 0xFF) as u32)
        };
        let (g3, g1) = (low(3), low(1));
        count(world, 0x005E_0EE0);
        if g3.wrapping_sub(17) <= 9 || g1.wrapping_sub(34) <= 5 {
            return Err(GameError::unclassified(
                0x00660192,
                "00660192: a weapon state against a set group",
            ));
        }
    }
    let eased = world
        .get_resource_or_init::<MovementUpdateGlobals>()
        .hand_follow_b14fb4;
    if follow != eased {
        let mut seconds = setting_float(world, "fFirstPersonScaleSeconds")?;
        if follow == 1.0 {
            seconds = 0.1;
        }
        let time = frame_time(world);
        let step = x87::stored_float(
            (f64::from(follow) - f64::from(eased)) * (f64::from(time) / f64::from(seconds)),
        );
        let value = x87::stored_float(f64::from(eased) + f64::from(step));
        let mut globals = world.resource_mut::<MovementUpdateGlobals>();
        globals.hand_follow_b14fb4 = value;
        if step >= 0.0 || value >= follow {
            if step <= 0.0 || follow >= value {
                follow = value;
            } else {
                globals.hand_follow_b14fb4 = follow;
            }
        } else {
            globals.hand_follow_b14fb4 = follow;
        }
    }
    let heading = super::player_move::player_heading(world, player);
    count(world, 0x0066_02A3);
    let pitch = world
        .get::<TesObjectRefr>(player)
        .expect("the player")
        .rotation_20[0];
    let base = multiply(
        &crate::gamebryo::transform::rotation_z(heading),
        &crate::gamebryo::transform::rotation_x(pitch),
    );
    let column = [base[1], base[4], base[7]];
    let mut dot = x87::stored_float(
        f64::from(column[0]) * f64::from(column[0])
            + f64::from(column[1]) * f64::from(column[1])
            + f64::from(column[2]) * 0.0,
    );
    dot = dot.clamp(-1.0, 1.0);
    let angle = f64::from(dot).acos() as f32;
    let mut tilt = x87::stored_float(f64::from(angle) * (1.0 - f64::from(follow)));
    if column[2] < 0.0 {
        // fmul by -1.0: the sign flips exactly.
        tilt = -tilt;
    }
    let rotation = multiply(&base, &crate::gamebryo::transform::rotation_x(tilt));
    let node = world
        .get::<PlayerCharacter>(player)
        .and_then(|data| data.first_person_node_5d0)
        .ok_or(GameError::unclassified(
            0x006602F2,
            "006602F2: a null +0x5D0 faults",
        ))?;
    world
        .get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(node)
        .ok_or(GameError::unclassified(
            0x006602F2,
            "006602F2: a +0x5D0 other than a NiNode",
        ))?
        .local_transform
        .rotation = rotation;
    Ok(())
}

// IDA: 005E14E0 bhkCharacterListener_SetPowerAttackFlag at 00603E1B.
// Completion: the two section tests and both listener stores.
fn update_listener_attack_flag(world: &mut World, controller: Entity, anim: Option<Entity>) {
    let active = anim.is_some_and(|anim| {
        section_sets_power_attack_flag(world, anim, 3)
            || section_sets_power_attack_flag(world, anim, 1)
    });
    let mut c = world
        .get_mut::<crate::bs::character_controller::BhkCharacterController>(controller)
        .expect("a controller");
    c.listener_power_attack_251 = u8::from(active);
    if !active {
        c.power_attack_movement_suppression_254 = 0;
    }
}

// IDA: 004723A0 ActorAnimData_GetMovementDelta. The two flags at both non-player calls
// are complements: reference flag 0x10 rotates by the root, otherwise Z is cleared.
// Completion: the root, section-speed limits, sleep scale and reference flag arms.
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "the x87 compare: a NaN takes the negated arm"
)]
fn animation_movement_delta(
    world: &World,
    actor: Entity,
    anim: Entity,
    flags: u16,
    sleep: u8,
) -> Result<[f32; 3], GameError> {
    use super::actor_animation::ActorAnimData;
    use super::anim_group::TesAnimGroup;
    use crate::bs::anim_group_sequence::AnimGroupSequence;
    use crate::gamebryo::controller_sequence::ControllerSequence;
    let anim = world
        .get::<ActorAnimData>(anim)
        .expect("an animation data record");
    let Some(root) = anim.node_04 else {
        return Ok([0.0; 3]);
    };
    let mut delta = anim.movement_0c;
    let base = world
        .get::<TesObjectRefr>(actor)
        .and_then(|r| r.base_form_1c);
    if !base.is_some_and(|b| crate::tes::forms_init::form_type(world, b) == 0x24)
        && flags & 0xF != 0
    {
        let group = |seq| {
            world
                .get::<AnimGroupSequence>(seq)
                .and_then(|s| world.get::<TesAnimGroup>(s.group_68))
        };
        if let Some(first) = anim.sequences_a0[0]
            .and_then(group)
            .filter(|g| (3..=16).contains(&g.group_type()))
        {
            let seconds = frame_time(world);
            let mut limit = section_movement_limit(first, anim.speed_bc, seconds);
            if let Some(active) = anim.sequences_a0[3]
                .filter(|s| {
                    world
                        .get::<ControllerSequence>(*s)
                        .is_some_and(|s| s.state_44 == 1)
                })
                .and_then(group)
            {
                let other = section_movement_limit(active, anim.speed_c0, seconds);
                if !(limit >= other) {
                    limit = other;
                }
            }
            if limit > 0.0 {
                let threshold = 4.0 * limit;
                for component in &mut delta {
                    if *component > threshold {
                        *component = limit;
                    }
                    if *component < -threshold {
                        *component = -limit;
                    }
                }
            }
        }
    }
    if matches!(sleep, 3 | 5 | 8 | 10) {
        let scale = super::actor_animation::actor_scale(world, actor)?;
        delta[0] /= scale;
        delta[1] /= scale;
    }
    if crate::tes::forms_references::form_flags(world, actor) & 0x10 == 0 {
        delta[2] = 0.0;
        Ok(delta)
    } else {
        let matrix = &world
            .get::<crate::gamebryo::scene_node::SceneNodePayload>(root)
            .expect("the animation root")
            .local_transform
            .rotation;
        Ok(crate::gamebryo::transform::rotate_vector(matrix, &delta).map(|v| v + 0.0))
    }
}

/// IDA: 0063C730 HighProcess_UpdateActorMovement through the local delta at 0063CAAF.
/// The non-player caller supplies g_niPoint3Zero. An active save can skip the whole movement call.
/// Completion: non-player ragdoll and animation displacement, listener and diagonal checks.
/// # Errors
/// A missing reference node, process or physics body, or a stop reading the animation.
pub fn high_actor_displacement(
    world: &mut World,
    actor: Entity,
    process: Entity,
) -> Result<Option<[f32; 3]>, GameError> {
    use super::actor_process::{HighProcess, MiddleHighProcess};
    use super::character_controller::actor_character_controller;
    use crate::bs::character_controller::BhkCharacterController;
    if world
        .get_resource::<crate::tes::tes::TesGlobals>()
        .and_then(|g| g.save_load_game.as_ref())
        .is_some_and(|save| {
            save.active()
                || save.fields_14[1] & 0x2000 != 0
                    && world
                        .get_resource::<crate::tes::faders::RendererFaders>()
                        .is_some_and(|f| f.records[2].alpha_14 > 0.0)
        })
    {
        return Ok(None);
    }
    let middle = world
        .get::<MiddleHighProcess>(process)
        .ok_or(GameError::unclassified(
            0x0063C80E,
            "missing MiddleHigh process state",
        ))?;
    let high = world
        .get::<HighProcess>(process)
        .ok_or(GameError::unclassified(
            0x0063C730,
            "movement on a non-High process",
        ))?;
    let dead = crate::tes::actor::actor_is_dead(world, actor, false);
    let controller = actor_character_controller(world, actor);
    if !matches!(middle.knocked_state_11c, 0 | 6) || dead || high.byte_2a9 != 0 {
        let root = crate::tes::forms_init_world::reference_ni_node(world, actor).ok_or(
            GameError::unclassified(0x0063CA0B, "a ragdoll actor without a scene root"),
        )?;
        let delta = crate::bs::ragdoll::apply_root_transform(world, root, !dead)?;
        if let Some(c) = controller {
            world
                .get_mut::<BhkCharacterController>(c)
                .expect("a controller")
                .flags_1f4 |= 0x1000;
        }
        let heading = world
            .get::<TesObjectRefr>(actor)
            .expect("a reference")
            .rotation_20[2];
        let matrix = crate::gamebryo::transform::rotation_z(heading);
        let transpose = std::array::from_fn(|i| matrix[(i % 3) * 3 + i / 3]);
        return Ok(Some(crate::gamebryo::transform::rotate_vector(
            &transpose, &delta,
        )));
    }
    let flags = high.movement_flags_1fc;
    let mut delta = match middle.anim_data_17c {
        Some(anim) => animation_movement_delta(world, actor, anim, flags, middle.sleep_state_11d)?,
        None => [0.0; 3],
    };
    if controller
        .and_then(|c| world.get::<BhkCharacterController>(c))
        .is_some_and(|c| {
            c.listener_power_attack_251 != 0 && c.power_attack_movement_suppression_254 != 0
        })
    {
        delta = [0.0; 3];
    }
    if flags & 3 != 0
        && flags & 12 != 0
        && world.get::<Actor>(actor).unwrap().dead_state_b0 != 1
        && !matches!(high.current_action_1f4, 11 | 12)
        && delta[1] != 0.0
    {
        let angle = (frame_time(world) * 0.0 / delta[1]) * std::f32::consts::FRAC_PI_4;
        let side = ((f64::from(std::f32::consts::FRAC_PI_2 - angle).cos() as f32) * delta[1]).abs();
        if flags & 4 != 0 {
            delta[0] -= side;
        } else {
            delta[0] += side;
        }
        delta[1] *= f64::from(angle).cos() as f32;
    }
    Ok(Some(delta))
}

// IDA: 00B069C4 and 00B34398, local statics of 0047F7B0.
#[derive(Resource)]
struct ActorVisibilityFrustum {
    tick: u32,
    planes: [crate::gamebryo::camera::CullingPlane; 6],
}

impl Default for ActorVisibilityFrustum {
    // IDA: 0047DCA0 NiFrustumPlanes_ctor and the image's FFFFFFFF tick.
    fn default() -> Self {
        Self {
            tick: u32::MAX,
            planes: Default::default(),
        }
    }
}

// IDA: 0047F7B0 NiAVObject_IsInCameraFrustum (00718200 NiFrustumPlanes_SetFromCamera).
// Completion: null guard, one plane refresh per +10 tick and all six bound comparisons.
pub(super) fn node_is_in_actor_frustum(
    world: &mut World,
    node: Option<Entity>,
) -> Result<bool, GameError> {
    use crate::bs::scene_graphs::{SceneGraph, SceneGraphGlobals};
    use crate::gamebryo::camera::{SceneCamera, camera_frustum_planes};
    use crate::gamebryo::scene_node::NiAvWorld;
    let camera = world
        .get_resource::<SceneGraphGlobals>()
        .and_then(|g| g.world)
        .and_then(|g| world.get::<SceneGraph>(g))
        .and_then(|g| g.camera_dc);
    let Some((camera, node)) = camera.zip(node) else {
        return Ok(false);
    };
    let tick = crate::tes::window::time_info_ticks_10(world).ok_or(GameError::unclassified(
        0x0047F7B0,
        "the visibility frustum has no game clock",
    ))?;
    if world.get_resource_or_init::<ActorVisibilityFrustum>().tick != tick {
        let planes = camera_frustum_planes(
            &world
                .get::<SceneCamera>(camera)
                .expect("a camera")
                .frustum_ec,
            &world
                .get::<NiAvWorld>(camera)
                .expect("the camera world transform")
                .transform,
        );
        *world.resource_mut::<ActorVisibilityFrustum>() = ActorVisibilityFrustum { tick, planes };
    }
    let bound = world
        .get::<NiAvWorld>(node)
        .expect("a reference world bound")
        .bound;
    Ok(!world
        .resource::<ActorVisibilityFrustum>()
        .planes
        .iter()
        .any(|plane| plane.excludes(bound)))
}

// IDA: 005EFAE0 Actor_Move. The camera-distance threshold controls when the avoidance
// phantom refreshes after a position change (0088D900), in Havok units.
// Completion: prison movement without an active path, mounted tilt or fall-damage event.
// TODO: [ ] Dialogue, path steering, mounted tilt and fall damage.
fn actor_move(
    world: &mut World,
    actor: Entity,
    process: Entity,
    seconds: f32,
    movement: [f32; 3],
    flags: u16,
) -> Result<(), GameError> {
    use super::character_controller::actor_character_controller;
    use crate::bs::character_controller::BhkCharacterController;
    use crate::bs::character_controller::BhkWorldObjectRecord;
    use crate::bs::character_controller::HAVOK_SCALE;
    if world
        .get_resource::<crate::tes::interface_manager::InterfaceMovieReads>()
        .is_some_and(|r| r.tile_menu_present(1009))
    {
        return Err(GameError::unclassified(
            0x005E6C10,
            "test the actor's visible dialogue",
        ));
    }
    if world
        .get::<super::actor_process::LowProcess>(process)
        .is_some_and(|p| p.pathing_34 != 0)
    {
        return Err(GameError::unclassified(
            0x005EFAE0,
            "steer with an active movement path",
        ));
    }
    let controller = actor_character_controller(world, actor);
    if let Some(c) = controller.and_then(|c| world.get::<BhkCharacterController>(c)) {
        if c.context_state_1ec == 5
            && world
                .get::<Actor>(actor)
                .expect("an actor")
                .horse_or_rider_d4
                != 0
        {
            return Err(GameError::unclassified(0x005EFC27, "mounted movement tilt"));
        }
        if let Some(avoid_box) = c.avoid_box_368 {
            let player = crate::tes::player::the_player(world).ok_or(GameError::unclassified(
                0x005EFC79,
                "missing player position",
            ))?;
            let target = world.get::<TesObjectRefr>(actor).unwrap().position_2c;
            let origin = world.get::<TesObjectRefr>(player).unwrap().position_2c;
            // 004D7E30 TESObjectREFR_GetDistanceToPoint.
            let [x, y, z] = std::array::from_fn(|i| target[i] - origin[i]);
            let mut distance = (x * x + y * y + z * z).sqrt();
            let node = crate::tes::forms_init_world::reference_ni_node(world, actor);
            if !node_is_in_actor_frustum(world, node)? {
                distance += 1000.0;
            }
            let threshold = if distance > 1000.0 {
                if distance < 4096.0 {
                    10.0
                } else if distance < 8192.0 {
                    25.0
                } else {
                    50.0
                }
            } else {
                0.0
            };
            world
                .get_mut::<BhkWorldObjectRecord>(avoid_box)
                .expect("a controller avoid box")
                .update_distance_threshold_60 = threshold * HAVOK_SCALE;
        }
    }
    super::mobile_move::mobile_object_move(world, actor, seconds, movement, u32::from(flags))?;
    if controller
        .and_then(|c| world.get::<BhkCharacterController>(c))
        .is_some_and(|c| c.flags_1f4 & 0x80 != 0)
    {
        return Err(GameError::unclassified(
            0x005EFD2F,
            "consume the controller's fall impact",
        ));
    }
    Ok(())
}

// IDA: 0063CB42..0063CD50, post-move cell check.
// Completion: interiors within their static geometry bound and cells without that node.
fn check_moved_actor_bounds(world: &World, actor: Entity) -> Result<(), GameError> {
    use crate::gamebryo::scene_node::{NiAvWorld, NiNodeChildren};
    use crate::tes::forms_world::{CellSceneNode, cell_is_interior};
    let Some(cell) = crate::tes::forms_references::parent_cell(world, actor) else {
        return Ok(());
    };
    if !cell_is_interior(world, cell) {
        return Err(GameError::unclassified(
            0x0063CB42,
            "check exterior actor movement against land height",
        ));
    }
    // 00441800 TESObjectCELL_GetQuadrantSubnode(cell, 0, 2).
    let child = |node| {
        world
            .get::<NiNodeChildren>(node)?
            .slots_ac
            .get(2)
            .copied()
            .flatten()
    };
    let Some(node) = world
        .get::<CellSceneNode>(cell)
        .and_then(|n| child(n.0))
        .and_then(child)
    else {
        return Ok(());
    };
    let [x, y, z, radius] = world
        .get::<NiAvWorld>(node)
        .expect("a quadrant geometry node")
        .bound;
    let [px, py, pz] = world.get::<TesObjectRefr>(actor).unwrap().position_2c;
    let (dx, dy, dz) = (x - px, y - py, z - pz);
    if radius >= (dx * dx + dy * dy + dz * dz).sqrt() {
        return Ok(());
    }
    Err(GameError::unclassified(
        0x0063C730,
        "find pathgrid placement outside the interior bound",
    ))
}

/// IDA: 0063C730 HighProcess_UpdateActorMovement, non-player dispatch at 005E0CE0.
/// Completion: save guard, displacement, Actor_Move and the interior quadrant check.
/// # Errors
/// An unsupported reference, controller or out-of-bounds movement arm.
pub fn high_process_update_actor_movement(
    world: &mut World,
    actor: Entity,
    process: Entity,
) -> Result<(), GameError> {
    let seconds = frame_time(world);
    if let Some(movement) = high_actor_displacement(world, actor, process)? {
        if crate::tes::forms_references::form_flags(world, actor) & 0x10 != 0 {
            return Err(GameError::unclassified(
                0x0063CAB5,
                "refresh position on the reference flag 0x10 arm",
            ));
        }
        let flags = world
            .get::<super::actor_process::HighProcess>(process)
            .expect("High movement")
            .movement_flags_1fc;
        actor_move(world, actor, process, seconds, movement, flags)?;
        check_moved_actor_bounds(world, actor)?;
    }
    Ok(())
}

// IDA: 005F12D0 Actor_UpdateControllerCombatOrImmobileFlag and the camera-distance
// checks at 0060559C..0060573B. 005EA2F0 and 005EA320 set the two listener bits.
// Completion: world and first-person cameras, player eye-height fallback and the distance test.
// TODO: [ ] Renderer occlusion-volume submission when enabled.
fn update_npc_controller_flags(
    world: &mut World,
    actor: Entity,
    root: Entity,
    controller: Option<Entity>,
) -> Result<(), GameError> {
    use crate::bs::character_controller::BhkCharacterController;
    use crate::gamebryo::scene_node::NiAvWorld;
    if let Some(c) = controller {
        let immobile = crate::tes::actor_package::actor_in_combat_package(world, actor, true)?
            || crate::tes::actor::actor_get_actor_value(world, actor, 4)? == 0;
        let mut c = world.get_mut::<BhkCharacterController>(c).unwrap();
        if immobile {
            c.flags_1f4 |= 0x4000;
        } else {
            c.flags_1f4 &= !0x4000;
        }
    }
    let player = crate::tes::player::the_player(world).expect("the player");
    let pc = world.get::<PlayerCharacter>(player).unwrap();
    let eye_node = world
        .get_resource::<super::player_3d::FirstPersonNodes>()
        .and_then(|nodes| {
            (pc.view_flags_588[0] != 0)
                .then_some(nodes.third_person_camera_b3bb10)
                .flatten()
                .or(nodes.camera_b3bb0c)
        });
    let eye = if let Some(n) = eye_node {
        world
            .get::<NiAvWorld>(n)
            .expect("player camera world transform")
            .transform
            .translation
    } else {
        let height =
            super::actor_animation::actor_scale(world, player)? * pc.camera_eye_height_offset_5d4;
        let mut eye = world.get::<TesObjectRefr>(player).unwrap().position_2c;
        eye[2] += height;
        eye
    };
    let [x, y, z, r] = world.get::<NiAvWorld>(root).unwrap().bound;
    let (dx, dy, dz) = (eye[0] - x, eye[1] - y, eye[2] - z);
    let surface = (dy * dy + dx * dx + dz * dz).sqrt() - r * 1.25;
    let globals = world.get_resource::<crate::tes::idle_frame::IdleFrameGlobals>();
    let step = globals.is_some_and(|g| g.havok_step_flag_b333b8 != 0);
    let beyond = world
        .get::<crate::bs::fade_node::BsFadeNode>(root)
        .is_some_and(|fade| {
            let multiplier = world
                .get_resource::<crate::bs::fade_node::FadeNodeSettings>()
                .copied()
                .unwrap_or_default()
                .actor_lod_multiplier_b0762c
                * globals.map_or(1.0, |g| g.lod_fade_actor_b075f0);
            let scaled = surface / multiplier;
            fade.far_squared_e4 < scaled * scaled
        });
    if let Some(c) = controller {
        let mut c = world.get_mut::<BhkCharacterController>(c).unwrap();
        if beyond {
            c.flags_1f4 |= 0x40000;
        } else {
            c.flags_1f4 &= !0x40000;
        }
        if step {
            c.flags_1f4 |= 0x100000;
        } else {
            c.flags_1f4 &= !0x100000;
        }
    }
    Ok(())
}

// IDA: 00603CA0 Actor_UpdateMovementAndAnimation, non-player dispatch.
// Completion: entry through the knocked-state and dry-interior water displacement calls.
// TODO: [ ] Finish the non-player animation, movement and controller update.
pub(super) fn npc_update_movement_and_animation(
    world: &mut World,
    actor: Entity,
    process: Entity,
    time: f32,
) -> Result<(), GameError> {
    use super::actor_process::MiddleHighProcess;
    let node = crate::tes::forms_init_world::reference_ni_node(world, actor);
    let face = actor_face_animation_data(world, actor);
    let controller = super::character_controller::actor_character_controller(world, actor);
    let anim = world
        .get::<MiddleHighProcess>(process)
        .and_then(|p| p.anim_data_17c);
    if node.is_none()
        || !crate::tes::forms_references::parent_cell(world, actor)
            .and_then(|cell| world.get::<crate::tes::forms_world::TesObjectCell>(cell))
            .is_some_and(|cell| cell.process_level_26 == 6)
    {
        return Ok(());
    }
    if time >= 900.0 {
        if let Some(mut middle) = world.get_mut::<MiddleHighProcess>(process) {
            middle.full_update_pending_16d = 1;
        }
        return Ok(());
    }
    if let Some(controller) = controller {
        update_listener_attack_flag(world, controller, anim);
    }
    // 006545E0 MiddleHighProcess_UpdateKnockedState: the ordinary corpse clears +11C.
    let dead_state = world.get::<Actor>(actor).expect("an actor").dead_state_b0;
    if let Some(mut middle) = world.get_mut::<MiddleHighProcess>(process) {
        if matches!(dead_state, 1 | 2) {
            middle.knocked_state_11c = 0;
        } else {
            let knocked = middle.knocked_state_11c;
            if crate::tes::ai_process::actor_get_actor_value_float(world, actor, 10)? < 0.0
                || crate::tes::actor::actor_get_actor_value(world, actor, 0x30)? != 0
                || dead_state == 6
                || knocked != 0
            {
                return Err(GameError::unclassified(
                    0x006545E0,
                    "actor knockdown transition",
                ));
            }
        }
    }
    // 0063F700 HighProcess_UpdateWadingWater returns before reading the actor for a dry interior.
    let displacements = world
        .get_resource::<crate::tes::settings::SettingCollections>()
        .and_then(|s| s.ini.find_address(0x00B07090))
        .is_some_and(|s| s.raw as u8 != 0);
    let dry_interior = world
        .get_resource::<crate::tes::tes::TesGlobals>()
        .and_then(|g| g.tes.as_ref())
        .and_then(|tes| tes.current_interior_cell_34)
        .and_then(|cell| world.get::<crate::tes::forms_world::TesObjectCell>(cell))
        .is_some_and(|cell| cell.flags_24 & 2 == 0);
    if displacements && !dry_interior {
        return Err(GameError::unclassified(
            0x0063F736,
            "update wading-water displacement in a water cell",
        ));
    }
    let health_bar = world
        .get_resource::<crate::tes::settings::SettingCollections>()
        .and_then(|s| s.ini.find_address(0x00B14E90))
        .is_some_and(|s| s.raw as u8 != 0);
    if health_bar {
        return Err(GameError::unclassified(
            0x00603E56,
            "update the actor health bar",
        ));
    }
    let middle = world
        .get::<MiddleHighProcess>(process)
        .ok_or(GameError::unclassified(
            0x00603E56,
            "loaded movement update below MiddleHigh",
        ))?;
    if (middle.sleep_state_11d == 9 || dead_state == 3) && face.is_some() {
        return Err(GameError::unclassified(
            0x00603E81,
            "check the sleep or death facial override",
        ));
    }
    if dead_state == 6 && matches!(middle.knocked_state_11c, 1 | 3) {
        return Err(GameError::unclassified(
            0x00603ED7,
            "advance the unconscious actor's recovery",
        ));
    }
    let package = super::character_3d::actor_current_package(world, actor);
    let combat = crate::tes::actor_package::actor_in_combat_package(world, actor, true)?;
    let package = package.map(|p| crate::tes::ai_package::package_data(world, p));
    let keep_action = combat || package.is_some_and(|p| p.type_20 == 8);
    let force_sneak = world.get::<Actor>(actor).expect("an actor").force_sneak_c9 != 0
        || package.as_ref().is_some_and(|p| p.flags_1c & 0x20000 != 0);
    if let Some(mut high) = world.get_mut::<super::actor_process::HighProcess>(process) {
        // 005E5670 tests action 6, after the combat and package type checks.
        if !keep_action && high.current_action_1f4 == 6 {
            return Err(GameError::unclassified(
                0x00603FAA,
                "reset the combat action",
            ));
        }
        // 005E0610/005E05F0 -> 00631B90. Actor_IsSneaking excludes a simultaneous 0x800.
        if force_sneak {
            high.movement_flags_1fc |= 0x400;
        } else if high.movement_flags_1fc & 0xC00 == 0x400 {
            high.movement_flags_1fc &= !0x400;
        }
    }
    // All non-player Character and Creature receivers reach 005E0CE0 with g_niPoint3Zero.
    high_process_update_actor_movement(world, actor, process)?;
    if world.get::<Actor>(actor).unwrap().horse_or_rider_d4 != 0 {
        return Err(GameError::unclassified(
            0x00604008,
            "position the actor with its mount or rider",
        ));
    }
    let root = node.expect("a loaded actor");
    let middle = world.get::<MiddleHighProcess>(process).unwrap();
    let (sleep, knocked) = (middle.sleep_state_11d, middle.knocked_state_11c);
    let reference = world.get::<TesObjectRefr>(actor).unwrap();
    let mut position = reference.position_2c;
    let mut rotation = crate::gamebryo::transform::rotation_z(reference.rotation_20[2]);
    let dead = crate::tes::actor::actor_is_dead(world, actor, false);
    if sleep == 0 && !dead && knocked == 0 {
        position[2] += setting_float(world, "fActorAnimZAdjust")?;
    }
    world
        .get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(root)
        .unwrap()
        .local_transform
        .translation = position;
    if let Some(tilt) = controller
        .and_then(|c| world.get::<crate::bs::character_controller::BhkCharacterController>(c))
        .map(|c| c.floats_320[3])
        .filter(|v| *v != 0.0)
    {
        rotation = crate::gamebryo::transform::multiply(
            &rotation,
            &crate::gamebryo::transform::rotation_x(tilt),
        );
    }
    let rotation = super::biped_attach::race_scaled_rotation(world, actor, &rotation, false)?;
    world
        .get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(root)
        .unwrap()
        .local_transform
        .rotation = rotation;
    if world
        .get::<TesObjectRefr>(actor)
        .unwrap()
        .extra_data_44
        .get(crate::tes::base_extra_list::EXTRA_LIGHT)
        .is_some()
    {
        return Err(GameError::unclassified(
            0x00604247,
            "update the actor's attached shader light",
        ));
    }
    if let Some(anim) = anim {
        anim_data_update_sequences(world, anim, actor)?;
        if dead_state != 3 && (!dead || dead_state == 1) && sleep == 0 && knocked == 0 {
            actor_update_head_tracking(world, actor)?;
        }
        anim_data_update_world(world, anim)?;
        if matches!(sleep, 3 | 5 | 8 | 10) {
            return Err(GameError::unclassified(
                0x006043EC,
                "adjust animated height in the actor's sleep state",
            ));
        }
    }
    world
        .get_resource_or_init::<MovementUpdateGlobals>()
        .byte_b3cbd0 = 0;
    if let Some(controller) = controller
        && !crate::tes::interface_manager::is_menu_mode(world)
        && !dead
    {
        update_actor_water_and_breath(world, actor, process, controller)?;
    }
    start_actor_dialogue(world, actor, process, anim, dead, dead_state)?;
    if dead_state == 1 && anim.is_some() {
        return Err(GameError::unclassified(
            0x00604E2C,
            "the dying actor's animation update",
        ));
    }
    if dead {
        let seconds = frame_time(world);
        let mut high = world
            .get_mut::<super::actor_process::HighProcess>(process)
            .unwrap();
        if high.death_state_timer_22c == f32::MAX {
            high.death_state_timer_22c = 0.0;
        }
        high.death_state_timer_22c -= seconds;
    }
    let middle = world.get::<MiddleHighProcess>(process).unwrap();
    if middle.byte_160 != 0 {
        return Err(GameError::unclassified(
            0x006054CB,
            "pending actor process update (+0x160)",
        ));
    }
    // 006054CB process vt+0x318 (0063CDC0 MiddleHighProcess_UpdateEquipment3D), which tests +0x16C itself.
    if middle.equipment_update_pending_16c != 0 {
        let services = world
            .get_resource::<crate::bs::model_loader::GameplayModelServices>()
            .ok_or(GameError::stop(
                0x0063CDC0,
                "an equipment rebuild needs model services",
            ))?
            .0;
        services(world, &mut |world, services| {
            super::equipment_3d::update_equipment_3d(world, process, actor, false, services)
        })?;
    }
    super::actor_fade::actor_update_fade(world, process, actor)?;
    update_npc_controller_flags(world, actor, root, controller)
}

/// IDA: 0060E320 PlayerCharacter_IsTrespassing (vt+0x354): with a parent cell, 004CABC0 TESObjectCELL_IsOwnedAgainst
/// (cell, player): a guard (Actor_IsGuard) answers false; a cell whose extra list (+0x28) has no owner (0x27), or with flags
/// (+0x24) 0x20 or 0x40, answers false; an owner is a stop here. Completion: an unowned cell.
fn player_trespassing(world: &World, player: Entity) -> Result<bool, GameError> {
    let Some(cell) = crate::tes::forms_references::parent_cell(world, player) else {
        return Ok(false);
    };
    let cell = world
        .get::<crate::tes::forms_world::TesObjectCell>(cell)
        .expect("a cell");
    if cell
        .extra_28
        .get(crate::tes::base_extra_list::EXTRA_OWNER)
        .is_none()
        || cell.flags_24 & 0x60 != 0
    {
        return Ok(false);
    }
    Err(GameError::stop(0x004CAC02, "an owned cell's trespass test"))
}

/// IDA: 00604BED..00604E2C, 00603CA0's dialogue start (LABEL_209) for a non-player. The combat byte is vt+0x334(1)
/// (005E6110). Any of these skips the start: 005E6CD0(0); the player; no animation data (vt+0x164, the process's +0x17C);
/// 00472EA0 ActorAnimData_CanStartDialogueIdle false (a queued idle +0xD0, or a current one +0xCC that is not playing); a
/// dead state of 3; death; sleep state 9; knocked down (vt+0x19C); the player sneaking (005E0550: 0x400 without 0x800);
/// a current package (process vt+0x244, +0xC0); and the rest of the chain. Skipped, a dialogue state byte (vt+0x200,
/// +0x228) dispatches the topic (vt+0x1A4), a stop. Not skipped, 005F6540's detection of the player, a stop.
/// Completion: an actor that skips at or before its current package.
/// # Errors
/// The stops above.
fn start_actor_dialogue(
    world: &mut World,
    actor: Entity,
    process: Entity,
    anim: Option<Entity>,
    dead: bool,
    dead_state: u32,
) -> Result<(), GameError> {
    let player = crate::tes::player::the_player(world);
    let skip = || -> Result<Option<bool>, GameError> {
        if crate::tes::actor_package::actor_takes_package_procedure(world, actor, false)?
            || player == Some(actor)
        {
            return Ok(Some(true));
        }
        let Some(anim) = anim else {
            return Ok(Some(true));
        };
        let data = world
            .get::<super::actor_animation::ActorAnimData>(anim)
            .expect("animation data");
        if data.queued_idle_d0.is_some() {
            return Ok(Some(true));
        }
        if data.current_idle_cc.is_some() {
            return Err(GameError::stop(
                0x00472EB6,
                "a current idle's dialogue start test",
            ));
        }
        let middle = world
            .get::<super::actor_process::MiddleHighProcess>(process)
            .expect("a High process");
        if dead_state == 3 || dead || middle.sleep_state_11d == 9 || middle.knocked_state_11c != 0 {
            return Ok(Some(true));
        }
        let sneaking = player
            .and_then(|player| {
                world
                    .get::<MobileObject>(player)
                    .and_then(|mobile| mobile.process_58)
            })
            .and_then(|process| world.get::<super::actor_process::HighProcess>(process))
            .is_some_and(|high| high.movement_flags_1fc & 0xC00 == 0x400);
        if sneaking || middle.current_package_c0.is_some() {
            return Ok(Some(true));
        }
        Ok(None)
    };
    match skip()? {
        Some(true) => {
            let byte = world
                .get::<super::actor_process::HighProcess>(process)
                .map_or(0, |high| high.dialogue_state_byte_228);
            if byte != 0 {
                return Err(GameError::unclassified(
                    0x00605123,
                    "dispatch current actor dialogue",
                ));
            }
            Ok(())
        }
        _ => {
            // 00604EFE.. the rest of the chain: the player's vt+0x354 (0060E320: its cell's ownership against it), an NPC base
            // (005E32D0), the process's vt+0x33C(0) (+0x220), the dialogue byte and the player's +0x600.
            let player = player.expect("the player");
            if player_trespassing(world, player)?
                || world
                    .get::<TesObjectRefr>(actor)
                    .and_then(|reference| reference.base_form_1c)
                    .is_none_or(|base| crate::tes::forms_init::form_type(world, base) != 0x23)
                || world
                    .get::<super::actor_process::HighProcess>(process)
                    .is_some_and(|high| high.fields_220[0] != 0)
                || world
                    .get::<super::actor_process::HighProcess>(process)
                    .is_some_and(|high| high.dialogue_state_byte_228 != 0)
                || world
                    .get::<super::player::PlayerCharacter>(player)
                    .is_some_and(|data| data.dialogue_cooldown_active_600 != 0)
            {
                return Err(GameError::stop(
                    0x00604F3E,
                    "a skip after the current-package test",
                ));
            }
            let combat = crate::tes::actor_package::actor_in_combat_package(world, actor, true)?;
            let (level, _seen) =
                crate::tes::detection::actor_detection_level(world, actor, player, combat)?;
            if level > 0 {
                return Err(GameError::stop(
                    0x00604F70,
                    "starting dialogue with a detected player",
                ));
            }
            Ok(())
        }
    }
}

// IDA: 005E06C0 Actor_IsSubmergedAtHeightFraction(actor, position, cell, fraction): no cell answers false; the scaled bound
// height (005E0660) times the fraction, stored a float (005E06DE); the position's z `fadd`s it (005E06E5) and is stored a
// qword (005E06E9): at the main thread's 24-bit precision that sum rounds to a float, so it is an `f32` add. The cell's
// water height (004CACE0) is compared with it (005E06F2 `fcomp`, 005E06FA `test ah, 41h`): submerged only when the water
// is above it; a NaN on either side answers false. Completion: 100%.
pub(crate) fn submerged_at_height_fraction(
    world: &mut World,
    actor: Entity,
    position: [f32; 3],
    cell: Option<Entity>,
    fraction: f32,
) -> Result<bool, GameError> {
    let Some(cell) = cell else {
        return Ok(false);
    };
    let height = crate::tes::actor::actor_height(world, actor)? * fraction;
    let level = position[2] + height;
    Ok(crate::tes::tes_grid::cell_water_height(world, cell) > level)
}

/// IDA: 006044E0..00604BED, 00603CA0's water and breath block for a non-player with a controller, out of menu mode and
/// alive. Three 005E06C0 tests at the parent cell's water: 0.01 (in water), 0.7 and 0.875 of the scaled height. The
/// maximum breath is 00548960 Actor_GetMaxSwimBreath (fActorSwimBreathBase + Endurance * fActorSwimBreathMult, x87, stored a
/// float). +0xD8 takes the 0.01 answer after the entered-water test. An NPC (005EA680 Actor_IsNPCOrValidMover: an NPC base, or a creature that
/// walks) out of 0.875's water, or not swimming (0x800), holds the maximum (process vt+0x1F0 00629830 HighProcess_SetSwimBreath, +0x238). Then the
/// controller's +0x338 is 0.1 for a water-walker out of deep water or a flier, else 1.6. For neither: with no 0.7 water,
/// not swimming and the context state (+0x1E0 +0x08) not 5, and having not just entered water, nothing; no water damage
/// out of water. Completion: an NPC out of water.
/// # Errors
/// The arms in water, a creature's, a water-walker's and the player's.
fn update_actor_water_and_breath(
    world: &mut World,
    actor: Entity,
    process: Entity,
    controller: Entity,
) -> Result<(), GameError> {
    let base = world
        .get::<TesObjectRefr>(actor)
        .and_then(|reference| reference.base_form_1c);
    if base.is_none_or(|base| crate::tes::forms_init::form_type(world, base) != 0x23) {
        return Err(GameError::stop(
            0x005EA680,
            "a creature's water and breath movement type",
        ));
    }
    if Some(actor) == crate::tes::player::the_player(world) {
        return Err(GameError::stop(
            0x0060466C,
            "the player's breath and the BreathMenu",
        ));
    }
    let cell = crate::tes::forms_references::parent_cell(world, actor);
    let position = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .position_2c;
    let in_water = submerged_at_height_fraction(world, actor, position, cell, 0.01)?;
    let chest = submerged_at_height_fraction(world, actor, position, cell, 0.7)?;
    let head = submerged_at_height_fraction(world, actor, position, cell, 0.875)?;
    let endurance = crate::tes::actor::actor_get_actor_value(world, actor, 5)?;
    let max_breath = (f64::from(endurance)
        * f64::from(setting_float(world, "fActorSwimBreathMult")?)
        + f64::from(setting_float(world, "fActorSwimBreathBase")?)) as f32;
    let entered = world.get::<Actor>(actor).expect("an actor").in_water_d8 == 0 && in_water;
    world.get_mut::<Actor>(actor).expect("an actor").in_water_d8 = u8::from(in_water);
    let swimming = world
        .get::<super::actor_process::HighProcess>(process)
        .is_some_and(|high| high.movement_flags_1fc & 0x800 != 0);
    if head && swimming {
        return Err(GameError::stop(
            0x00604604,
            "an actor holding its breath under water",
        ));
    }
    world
        .get_mut::<super::actor_process::HighProcess>(process)
        .ok_or(GameError::stop(
            0x006045E8,
            "a breath store on a process that isn't High",
        ))?
        .swim_breath_238 = max_breath;
    let water_walking = crate::tes::actor::actor_get_actor_value(world, actor, 0x38)? != 0;
    let flying = is_flying_creature(world, actor);
    let factor = if (!head && water_walking) || flying {
        0.1
    } else {
        1.6
    };
    world
        .get_mut::<crate::bs::character_controller::BhkCharacterController>(controller)
        .expect("a controller")
        .floats_320[6] = factor;
    if water_walking || flying {
        return Err(GameError::stop(
            0x006048CB,
            "a water-walker's or flier's swim state",
        ));
    }
    if chest {
        return Err(GameError::stop(0x006048E0, "an actor entering deep water"));
    }
    let context_state = world
        .get::<crate::bs::character_controller::BhkCharacterController>(controller)
        .expect("a controller")
        .context_state_1ec;
    if swimming || context_state == 5 {
        return Err(GameError::stop(
            0x0060491B,
            "an actor leaving its swim state",
        ));
    }
    if entered {
        return Err(GameError::stop(
            0x0060494A,
            "an actor's entered-water splash",
        ));
    }
    if in_water {
        return Err(GameError::stop(0x00604B30, "an actor's water damage"));
    }
    Ok(())
}

/// IDA: 00603CA0 Actor_UpdateMovementAndAnimation(time) for the player, the arms run-ak's two calls take. vt+0x154 the
/// node (`player_ni_node`), process vt+0xE8 (the FaceGen data), the controller, vt+0x164 (`player_anim_data`); no node or a
/// parent cell not at level 6 (+0x26) returns; a time of 900 or more is a stop. v156 = the player in first person
/// (+0x588 clear). With a controller: `00470720`/`0051ACC0` on sections 3 and 1 (the in-water test), `005E14E0` on the
/// listener (+0x251 the answer, a zero clears +0x254). The process's vt+0x2F4 and vt+0x440 (counted); bHealthBarShowing's
/// vt+0x60 (a stop when set); the sleep and death tests (a dead or sleeping player is a stop). vt+0x27C answers 2, so
/// 005E0CE0 is skipped; `005E1030` (no horse) false and vt+0x388 false. v156: +0x5D0's +0x54 = the position with z +
/// +0x5D4 (a float sum), then `00660130`. Otherwise +0x3C's +0x54 = the position with z + fActorAnimZAdjust (0065ABB0 the Z
/// rotation, 00B3BAA8 clear, not sleeping or dead, vt+0x19C false), and +0x30 = 0070FDD0(Z) times 0070FD30(+0x32C) when
/// that is nonzero, through TESObjectREFR_ApplyRaceScaleToRotation(0). `004D7FC0`'s light (counted). With anim data:
/// `00471C00`, `00603500` unless v156, `00471F20`; the sleep-state switch (0 here). 00B3CBD0 = 0. Not v156: the controller,
/// InterfaceManager_IsMenuMode (true during the move) skips the breath block; `005E6CD0` and the process calls (counted).
/// Both: process vt+0x8 (counted) and `00633250` (`actor_fade::actor_update_fade`), `005F12D0` (+0x1F4 bit 0x4000 by vt+0x334(1), counted as false, or a zero
/// Speed), process vt+0x18C's controller and `005EA320`(00B333B8: 0, which nothing in the port sets) clears 0x100000.
/// Completion: 100% of the player's arms in the move; the counted calls are listed in the module doc.
/// # Errors
/// The stops named.
pub fn player_update_movement_and_animation(
    world: &mut World,
    player: Entity,
    time: f32,
) -> Result<(), GameError> {
    let Some(node) = super::player_3d::player_ni_node(world, player) else {
        return Ok(());
    };
    let controller = super::character_controller::actor_character_controller(world, player);
    let anim = player_anim_data(world, player);
    let level = crate::tes::forms_references::parent_cell(world, player)
        .and_then(|cell| world.get::<crate::tes::forms_world::TesObjectCell>(cell))
        .map(|cell| cell.process_level_26);
    if level != Some(6) {
        return Ok(());
    }
    if time >= 900.0 {
        return Err(GameError::unclassified(
            0x00603D4E,
            "00603D4E: a time of 900 or more",
        ));
    }
    let first_person = world
        .get::<PlayerCharacter>(player)
        .expect("the player")
        .view_flags_588[0]
        == 0;
    if let Some(controller) = controller {
        update_listener_attack_flag(world, controller, anim);
    }
    count(world, 0x0060_3E34);
    count(world, 0x0060_3E42);
    let health_bar = world
        .get_resource::<crate::tes::settings::SettingCollections>()
        .and_then(|settings| settings.ini.find_address(0x00B1_4E90))
        .map_or(0, |setting| setting.raw);
    if health_bar & 0xFF != 0 {
        return Err(GameError::unclassified(
            0x00603E73,
            "00603E73: bHealthBarShowing's process vt+0x60",
        ));
    }
    if world.get::<Actor>(player).expect("an actor").dead_state_b0 != 0 {
        return Err(GameError::unclassified(
            0x00603E8C,
            "00603E8C: a dead or knocked player",
        ));
    }
    if world
        .get::<PlayerCharacter>(player)
        .is_some_and(|data| data.last_ridden_horse_1e0 != 0)
    {
        return Err(GameError::unclassified(
            0x00604010,
            "00604010: the player's horse",
        ));
    }
    let position = world
        .get::<TesObjectRefr>(player)
        .expect("the player")
        .position_2c;
    if first_person {
        let eye = world
            .get::<PlayerCharacter>(player)
            .expect("the player")
            .camera_eye_height_offset_5d4;
        let z = x87::stored_float(f64::from(eye) + f64::from(position[2]));
        world
            .get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(node)
            .ok_or(GameError::unclassified(
                0x006040D0,
                "006040D0: a node other than a NiNode",
            ))?
            .local_transform
            .translation = [position[0], position[1], z];
        player_update_first_person_rotation(world, player)?;
    } else {
        let heading = world
            .get::<TesObjectRefr>(player)
            .expect("the player")
            .rotation_20[2];
        let adjust = setting_float(world, "fActorAnimZAdjust")?;
        let z = x87::stored_float(f64::from(adjust) + f64::from(position[2]));
        let mut rotation = crate::gamebryo::transform::rotation_z(heading);
        if let Some(controller) = controller {
            let tilt = world
                .get::<crate::bs::character_controller::BhkCharacterController>(controller)
                .expect("a controller")
                .floats_320[3];
            if tilt != 0.0 {
                rotation = crate::gamebryo::transform::multiply(
                    &rotation,
                    &crate::gamebryo::transform::rotation_x(tilt),
                );
            }
        }
        let rotation = super::biped_attach::race_scaled_rotation(world, player, &rotation, false)?;
        let mut payload = world
            .get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(node)
            .ok_or(GameError::unclassified(
                0x00604190,
                "00604190: a node other than a NiNode",
            ))?;
        payload.local_transform.translation = [position[0], position[1], z];
        payload.local_transform.rotation = rotation;
    }
    count(world, 0x004D_7FC0);
    if let Some(anim) = anim {
        anim_data_update_sequences(world, anim, player)?;
        if !first_person {
            actor_update_head_tracking(world, player)?;
        }
        anim_data_update_world(world, anim)?;
    }
    world
        .get_resource_or_init::<MovementUpdateGlobals>()
        .byte_b3cbd0 = 0;
    if !first_person && controller.is_some() {
        if !crate::tes::interface_manager::is_menu_mode(world) {
            return Err(GameError::unclassified(
                0x006044CE,
                "006044CE: the breath and water block",
            ));
        }
        count(world, 0x005E_6CD0);
    }
    // 00605525 00633250, the process's fade (`actor_fade`).
    if let Some(process) = crate::tes::actor::process_of(world, player) {
        super::actor_fade::actor_update_fade(world, process, player)?;
    }
    if let Some(controller) = controller {
        // 005F12D0: vt+0x334(1) (counted, false) or a zero Speed (4) sets 0x4000.
        count(world, 0x005F_12EB);
        let speed = super::player::player_get_actor_value(world, player, 4)?;
        let mut c = world
            .get_mut::<crate::bs::character_controller::BhkCharacterController>(controller)
            .expect("a controller");
        if speed == 0 {
            c.flags_1f4 |= 0x4000;
        } else {
            c.flags_1f4 &= !0x4000;
        }
        // 0060573B 005EA320(00B333B8).
        c.flags_1f4 &= !0x10_0000;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bs::character_controller::BhkCharacterController;
    use crate::tes::actor_animation::ActorAnimData;

    #[test]
    fn only_the_first_four_power_attack_groups_keep_the_listener_state() {
        let mut world = World::new();
        let anim = world.spawn(ActorAnimData::default()).id();
        let controller = world.spawn(BhkCharacterController::default()).id();
        for (group, active) in [
            (0xFF, false),
            (0xFFFF, false),
            (21, false),
            (22, true),
            (0x319, true),
            (26, false),
            (34, false),
        ] {
            for section in [1, 3] {
                let mut data = world.get_mut::<ActorAnimData>(anim).unwrap();
                data.groups_3c = [0xFFFF; 5];
                data.groups_3c[section] = group;
                world
                    .get_mut::<BhkCharacterController>(controller)
                    .unwrap()
                    .power_attack_movement_suppression_254 = 17;
                update_listener_attack_flag(&mut world, controller, Some(anim));
                let record = world.get::<BhkCharacterController>(controller).unwrap();
                assert_eq!(record.listener_power_attack_251, u8::from(active));
                assert_eq!(
                    record.power_attack_movement_suppression_254,
                    if active { 17 } else { 0 }
                );
            }
        }
    }
}
