//! The High-list body called by ActorProcessManager_UpdateHighActors at 00677EC0.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::actor::{MobileObject, TesObjectRefr};
use super::actor_process::{
    BaseProcess, HighProcess, LowProcess, MiddleHighProcess, MiddleLowProcess,
};
use crate::bs::fade_node::{BsFadeNode, FadeNodeSettings, within_scaled_fade_distance};
use crate::bs::scene_graphs::{SceneGraph, SceneGraphGlobals};
use crate::gamebryo::camera::SceneCamera;
use crate::gamebryo::scene_node::NiAvWorld;
use crate::tes::ai_package::package_data;
use crate::tes::forms_init_world::reference_ni_node;
use crate::tes::forms_references::form_flags;
use crate::tes::idle_frame::IdleFrameGlobals;

use super::actor::{Actor, actor_process_level};
use super::actor_process::{ActorListCursor, ActorListId, ActorListNode, ActorProcessGlobals};
use crate::tes::actor_process::{write_controller, write_middle_high_process};
use crate::tes::save_game::{ProcessView, SaveWriter, controller, empty_list, null_id};

// IDA: 00634CB0 HighProcess_FindBoneLODController. Clear +1EC before testing the
// reference's raw +3C, then walk the Bip01 controllers from the head.
// Completion: 100% of the lookup and exact class test.
pub(in crate::tes) fn find_bone_lod_controller(world: &mut World, process: Entity, actor: Entity) {
    use crate::gamebryo::{
        controller_time::controller_next, object_net::NiObjectNet, stream_objects::NiClass,
    };
    world
        .get_mut::<HighProcess>(process)
        .unwrap()
        .bone_lod_controller_1ec = None;
    let mut controller = world
        .get::<crate::tes::reference_3d::Reference3D>(actor)
        .and_then(|root| crate::tes::reference_3d::find_bone(world, actor, root.0, b"Bip01"))
        .and_then(|bone| world.get::<NiObjectNet>(bone))
        .and_then(|net| net.controller_0c);
    while let Some(current) = controller {
        if world.get::<NiClass>(current) == Some(&NiClass::BoneLodController) {
            world
                .get_mut::<HighProcess>(process)
                .unwrap()
                .bone_lod_controller_1ec = Some(current);
            break;
        }
        controller = controller_next(world, current);
    }
}

// IDA: 00677500 ActorProcessManager_RemoveExcessDeadActors.
// Completion: the countdown, live actor census and interval selection when no corpse is a candidate.
// TODO: [ ] Qualify state-2 corpses and remove them according to the two GMST limits.
fn remove_excess_dead_actors(world: &mut World, frame_seconds: f32) -> Result<(), GameError> {
    let mut globals = world.resource_mut::<ActorProcessGlobals>();
    globals.manager.remove_excess_dead_time_ac -= frame_seconds;
    let remaining = globals.manager.remove_excess_dead_time_ac;
    // 00677528 tests C0 after comparing zero with the stored time. Unordered also returns.
    if remaining > 0.0 || remaining.is_nan() {
        return Ok(());
    }
    let mut actors = 0u32;
    for reference in world
        .resource::<ActorProcessGlobals>()
        .manager
        .high_68
        .list
        .ordered_items()
    {
        if let Some(actor) = world.get::<Actor>(reference) {
            actors += 1;
            if actor.dead_state_b0 == 2 && actor_process_level(world, reference) == 0 {
                return Err(GameError::unclassified(
                    0x006775B2,
                    "006775B2: qualify a dead High actor for removal",
                ));
            }
        }
    }
    let complex = actors >= 25 && world.resource::<IdleFrameGlobals>().havok_step_flag_b333b8 != 0;
    let address = if complex { 0x00B37D58 } else { 0x00B37D40 };
    let interval = world
        .get_resource::<crate::tes::game_settings::GameSettingCollection>()
        .and_then(|settings| settings.raw_at(address))
        .ok_or_else(|| {
            GameError::unclassified(0x00677647, "00677647: missing dead-actor removal interval")
        })?;
    world
        .resource_mut::<ActorProcessGlobals>()
        .manager
        .remove_excess_dead_time_ac = f32::from_bits(interval);
    Ok(())
}

// IDA: 006282B0 HighProcess_ResetActorPackageAnimation. The caller has already checked sleep state.
// Completion: 00475440 on empty current/queued idle slots and the four facial channels.
// TODO: [ ] Stop and retire populated AnimIdle entries with their owned sequence.
fn reset_package_animation(
    world: &mut World,
    process: Entity,
    actor: Entity,
) -> Result<(), GameError> {
    let middle = world
        .get::<MiddleHighProcess>(process)
        .expect("a High process");
    if !matches!(middle.sleep_state_11d, 0 | 4 | 9) {
        return Ok(());
    }
    if let Some(anim) = middle.anim_data_17c {
        let data = world
            .get::<super::actor_animation::ActorAnimData>(anim)
            .expect("actor animation");
        if data.current_idle_cc.is_some() || data.queued_idle_d0.is_some() {
            return Err(GameError::unclassified(
                0x00475440,
                "00475440: stop a populated AnimIdle entry",
            ));
        }
    }
    if let Some(face) = super::actor_movement_update::actor_face_animation_data(world, actor) {
        let mut data = world
            .get_mut::<crate::tes::face_gen_nodes::BsFaceGenAnimationData>(face)
            .expect("face animation");
        crate::tes::face_gen_nodes::reset_face_keyframe_channels(
            &mut data, 0.75, true, true, true, true,
        );
    }
    Ok(())
}

// IDA: 00629B70 HighProcess_EvaluatePackage. The fade-distance caller passes false and the
// full update can force evaluation at 0063A6E9.
// Completion: sleep and package gates, integer-hour throttle, timer, and normal package-change stores.
// TODO: [ ] Running package ownership on replacement, occupied sleep furniture and owned +0x3C entries.
pub(super) fn evaluate_high_package(
    world: &mut World,
    process: Entity,
    actor: Entity,
    frame_seconds: f32,
    force: bool,
) -> Result<bool, GameError> {
    use crate::tes::actor_process::process_evaluate_package;
    use crate::tes::ai_conditions::ScriptArms;
    use crate::tes::global::game_hour;

    let middle = world
        .get::<MiddleHighProcess>(process)
        .expect("a High process");
    if !matches!(middle.sleep_state_11d, 0 | 4 | 9)
        || (!force && middle.current_package_c0.is_some())
    {
        return Ok(false);
    }
    if middle
        .current_package_c0
        .is_some_and(|p| package_data(world, p).type_20 != 21)
    {
        return Err(GameError::unclassified(
            0x00629B70,
            "forced replacement of an owned running package",
        ));
    }
    let previous = super::character_3d::actor_current_package(world, actor);
    let reequip = previous.is_some_and(|package| {
        let old = package_data(world, package);
        old.type_20 == 4 || old.flags_1c & 0x30_0000 != 0
    });
    let hour = game_hour(world);
    let editor = world
        .get::<BaseProcess>(process)
        .expect("a High process")
        .editor_package_08;
    let timer = world
        .get::<HighProcess>(process)
        .unwrap()
        .package_evaluation_timer_1ac;
    let hour_number = oblivion_core::x87::ftol(hour);
    let changed = if force
        || editor.is_none()
        || timer <= 0.0
        || world
            .get::<MiddleLowProcess>(process)
            .unwrap()
            .evaluated_hour_90
            != hour_number
    {
        let changed = process_evaluate_package(world, process, actor, force, &mut ScriptArms)?;
        world
            .get_mut::<HighProcess>(process)
            .unwrap()
            .package_evaluation_timer_1ac = 20.0;
        world
            .get_mut::<MiddleLowProcess>(process)
            .unwrap()
            .evaluated_hour_90 = hour_number;
        changed
    } else {
        false
    };
    world
        .get_mut::<HighProcess>(process)
        .unwrap()
        .package_evaluation_timer_1ac -= frame_seconds;
    if !changed {
        return Ok(false);
    }
    let mut high = world.get_mut::<HighProcess>(process).unwrap();
    high.byte_25d = 0;
    high.byte_25c = 0;
    if world
        .get::<MiddleHighProcess>(process)
        .unwrap()
        .current_package_c0
        .is_some_and(|package| package_data(world, package).type_20 != 21)
    {
        return Err(GameError::unclassified(
            0x00629CCA,
            "00629CCA: clear an owned running package",
        ));
    }
    let mut low = world.get_mut::<LowProcess>(process).unwrap();
    low.byte_1d = 0;
    low.byte_1c = 0;
    reset_package_animation(world, process, actor)?;
    let mut high = world.get_mut::<HighProcess>(process).unwrap();
    high.float_1b4 = 0.0;
    high.float_260 = 0.0;
    high.float_1b0 = 0.0;
    high.float_1e8 = 0.0;
    high.field_1cc = 0;
    high.byte_278 = 0;
    world.get_mut::<LowProcess>(process).unwrap().field_2c = None;
    world.get_mut::<HighProcess>(process).unwrap().byte_244 = 0;
    if world.get::<LowProcess>(process).unwrap().list_3c != [0; 2] {
        return Err(GameError::unclassified(
            0x00629D2C,
            "00629D2C: release the High process's owned package list",
        ));
    }
    if world.get::<Actor>(actor).unwrap().horse_or_rider_d4 != 0
        || crate::tes::actor::actor_sitting_state(world, actor) != 0
    {
        return Err(GameError::unclassified(
            0x00629D38,
            "00629D38: package change with a mounted or seated actor",
        ));
    }
    if reequip {
        let flags = world
            .get::<BaseProcess>(process)
            .unwrap()
            .editor_package_08
            .map_or(0, |package| package_data(world, package).flags_1c);
        let clothing = flags & 0x10_0000 == 0;
        let weapon = flags & 0x20_0000 == 0;
        let base = world
            .get::<TesObjectRefr>(actor)
            .unwrap()
            .base_form_1c
            .expect("actor base");
        match crate::tes::forms_init::form_type(world, base) {
            35 => crate::tes::forms_actors::auto_equip(world, actor, clothing, weapon, true)?,
            36 => {
                crate::tes::forms_actors::creature_auto_equip(world, actor, clothing, weapon, true)?
            }
            _ => {}
        }
    }
    Ok(true)
}

// IDA: 00677FA6..00677FC9. Reload +0x58 after evaluation, then again for the timestamp virtual.
pub fn update_distant_high_actor(
    world: &mut World,
    actor: Entity,
    frame_seconds: f32,
) -> Result<(), GameError> {
    let process = crate::tes::actor::process_of(world, actor).expect("a High process");
    evaluate_high_package(world, process, actor, frame_seconds, false)?;
    let process = crate::tes::actor::process_of(world, actor).expect("a High process");
    world
        .get_mut::<HighProcess>(process)
        .expect("a High process")
        .skipped_high_update_beyond_fade_distance_1d1 = 1;
    let process = crate::tes::actor::process_of(world, actor).expect("an actor process");
    crate::tes::actor_process::process_update_stamps(world, process);
    Ok(())
}

// IDA: 006780C0..00678167, Actor_GetDetectionTimer / Actor_DecrementDetectionTimer and
// Actor_UpdateDetection (005E0790 -> 00642880). The stagger advances even when fader 2
// suppresses detection. 00674820 adds each nearby actor once at the head.
fn update_high_detection(
    world: &mut World,
    actor: Entity,
    distance: f32,
    count: u32,
    frame_seconds: f32,
    stagger: &mut f32,
) -> Result<(), GameError> {
    use crate::tes::actor::actor_is_dead;
    use crate::tes::base_extra_list::EXTRA_GHOST;
    use crate::tes::faders::RendererFaders;

    let process = crate::tes::actor::process_of(world, actor).expect("a High process");
    let high = world.get::<HighProcess>(process).expect("a High process");
    let state = world.get::<Actor>(actor).expect("an actor").dead_state_b0;
    if high.detection_timer_204 >= 0.0
        || high.detection_timer_204.is_nan()
        || state == 3
        || actor_is_dead(world, actor, false)
    {
        world
            .get_mut::<HighProcess>(process)
            .unwrap()
            .detection_timer_204 -= frame_seconds;
        return Ok(());
    }

    let alpha = world
        .get_resource::<RendererFaders>()
        .map_or(0.0, |faders| faders.records[2].alpha_14);
    if (alpha <= 0.0 || alpha.is_nan())
        && world
            .get::<MiddleHighProcess>(process)
            .unwrap()
            .knocked_state_11c
            == 0
        && !(state == 4
            && world
                .get::<TesObjectRefr>(actor)
                .unwrap()
                .extra_data_44
                .get(EXTRA_GHOST)
                .is_some())
    {
        return Err(GameError::unclassified(
            0x006411E0,
            "006411E0: evaluate High actor detection",
        ));
    }
    let radius = world
        .get_resource::<crate::tes::game_settings::GameSettingCollection>()
        .and_then(|settings| settings.raw_at(0x00B36CD8))
        .map_or(0.0, f32::from_bits);
    if radius > distance {
        let nearby = &mut world
            .resource_mut::<ActorProcessGlobals>()
            .manager
            .near_player_60;
        if !nearby.items().any(|item| item == actor) {
            nearby.prepend_non_null(actor);
        }
    }
    *stagger += (count as f32 * 0.25) * frame_seconds;
    Ok(())
}

// IDA: 00677EC0 ActorProcessManager_UpdateHighActors. Walk +0x68 forward with manager cursors
// +0x78 (current) and +0x74 (preceding). Section 3.2 requires an exclusive transaction: a level
// change may erase either cursor before 00678331 reads them again.
// Completion: entry, live cursor traversal, per-actor update dispatch and the countdown tail.
// TODO: [ ] Detection, followers and High Actor_Update.
pub fn update_high_actors(
    world: &mut World,
    update_time: f32,
    frame_seconds: f32,
) -> Result<(), GameError> {
    let head = ActorListCursor {
        list: ActorListId::High68,
        node: ActorListNode::HEAD,
    };
    let mut globals = world.resource_mut::<ActorProcessGlobals>();
    globals.wander_destination_claimed_this_frame_b3b935 = 0;
    globals.manager.cursor_78 = Some(head);
    let actor_count = globals.manager.high_68.list.ordered_items().count() as u32;
    globals.manager.high_actor_count_a8 = actor_count;
    let mut detection_stagger = 0.0;
    while let Some(cursor) = world.resource::<ActorProcessGlobals>().manager.cursor_78 {
        let list = &world
            .resource::<ActorProcessGlobals>()
            .manager
            .list(cursor.list)
            .list;
        let Some(actor) = list.item(cursor.node) else {
            if list.next(cursor.node).is_none() {
                break;
            }
            return Err(GameError::unclassified(
                0x00677F2A,
                "00677F2A: the High cursor repeats an empty nonterminal node",
            ));
        };
        let flags = form_flags(world, actor);
        if flags & 0x20_0000 != 0 {
            return Err(GameError::unclassified(
                0x00677F3B,
                "00677F3B: the High cursor repeats a flagged actor",
            ));
        }
        if flags & (0x20 | 0x800) == 0 && actor_process_level(world, actor) == 0 {
            let player = crate::tes::player::the_player(world).ok_or_else(|| {
                GameError::unclassified(
                    0x00677F7C,
                    "00677F7C: missing player in High actor distance",
                )
            })?;
            let distance =
                crate::tes::forms_references::reference_distance(world, actor, player, false)?;
            let is_actor = world.get::<Actor>(actor).is_some();
            let distant = is_actor && should_skip_high_update_beyond_fade_distance(world, actor)?;
            if distant {
                update_distant_high_actor(world, actor, frame_seconds)?;
            } else {
                if is_actor
                    && super::character_controller::actor_character_controller(world, actor)
                        .is_none()
                    && reference_ni_node(world, actor).is_some()
                {
                    super::character_controller::actor_create_character_controller(
                        world,
                        actor,
                        Default::default(),
                    )?;
                }
                super::actor_update::actor_update(world, actor, update_time)?;
            }
            if is_actor && super::player::player_is_sleeping(world)? {
                super::actor_regeneration::restore_health(world, actor)?;
                let casting = crate::tes::actor::process_of(world, actor)
                    .and_then(|p| world.get::<MiddleHighProcess>(p))
                    .is_some_and(|p| p.current_magic_item_144 != 0);
                // flt_A379B4 is 2.0. The original only skips the middle call's recovery while casting.
                if !casting {
                    super::actor_regeneration::regenerate_magicka(world, actor, 2.0)?;
                }
                super::actor_regeneration::regenerate_fatigue(world, actor, 2.0)?;
            }
            if actor_process_level(world, actor) != 0 {
                world
                    .resource_mut::<ActorProcessGlobals>()
                    .manager
                    .cursor_78 = None;
            }
            if world
                .get::<TesObjectRefr>(actor)
                .unwrap()
                .base_form_1c
                .is_some()
                && actor_process_level(world, actor) == 0
                && is_actor
                && actor != player
                && !crate::tes::forms_references::save_load_active(world)?
            {
                update_high_detection(
                    world,
                    actor,
                    distance,
                    actor_count,
                    frame_seconds,
                    &mut detection_stagger,
                )?;
            }
            if !distant
                && world
                    .get::<TesObjectRefr>(actor)
                    .unwrap()
                    .extra_data_44
                    .get(crate::tes::base_extra_list::EXTRA_FOLLOWER)
                    .is_some()
            {
                return Err(GameError::unclassified(
                    0x00678192,
                    "00678192: process the reference's follower list",
                ));
            }
        }
        let manager = &mut world.resource_mut::<ActorProcessGlobals>().manager;
        let current = manager.cursor_78.or(manager.cursor_74).unwrap_or(head);
        manager.cursor_74 = Some(current);
        manager.cursor_78 = manager
            .list(current.list)
            .list
            .next(current.node)
            .map(|node| ActorListCursor {
                list: current.list,
                node,
            });
    }
    remove_excess_dead_actors(world, frame_seconds)
}

// IDA: 005F1330 Actor_ShouldSkipHighUpdateBeyondFadeDistance.
// Completion: the normal High-process virtuals, including consuming the one-shot full update.
// An actor with a persistent base takes the full-update arm without reading the process or camera.
pub fn should_skip_high_update_beyond_fade_distance(
    world: &mut World,
    actor: Entity,
) -> Result<bool, GameError> {
    // 004D9B10 TESObjectREFR_IsBaseFormPersistent calls the base form's +0x78 (004D7030 for NPC and Creature).
    if world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .base_form_1c
        .is_some_and(|base| form_flags(world, base) & 0x400 != 0)
    {
        return Ok(false);
    }
    let process = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58
        .ok_or_else(|| GameError::unclassified(0x005F1341, "005F1341: null actor process"))?;
    let base = world.get::<BaseProcess>(process).expect("a High process");
    if let Some(package) = base.editor_package_08 {
        let kind = package_data(world, package).type_20;
        if !matches!(kind, 5 | 6) || base.edit_pack_procedure_type_04 == 0 {
            return Ok(false);
        }
    }
    let mut middle = world
        .get_mut::<MiddleHighProcess>(process)
        .expect("a High process");
    if middle.full_update_pending_16d != 0 {
        // 006295B0 / 006295A0: a pending update overrides the distance once.
        middle.full_update_pending_16d = 0;
        return Ok(false);
    }
    let Some(node) = reference_ni_node(world, actor) else {
        return Ok(false);
    };
    let Some(fade) = world.get::<BsFadeNode>(node) else {
        return Ok(false);
    };
    let camera = world
        .get_resource::<SceneGraphGlobals>()
        .and_then(|state| state.world)
        .and_then(|graph| world.get::<SceneGraph>(graph))
        .and_then(|graph| graph.camera_dc)
        .ok_or_else(|| {
            GameError::unclassified(0x005F13B8, "005F13B8: missing world scene camera")
        })?;
    let settings = world
        .get_resource::<FadeNodeSettings>()
        .copied()
        .unwrap_or_default();
    Ok(!within_scaled_fade_distance(
        fade,
        world.get::<NiAvWorld>(node).expect("fade world transform"),
        world
            .get::<NiAvWorld>(camera)
            .expect("camera world transform"),
        world
            .get::<SceneCamera>(camera)
            .expect("a scene camera")
            .lod_adjust_120,
        0.8,
        settings.actor_lod_multiplier_b0762c,
        world
            .get_resource::<IdleFrameGlobals>()
            .map_or(1.0, |state| state.lod_fade_actor_b075f0),
    ))
}

// IDA: 006324F0 at version 125 with flags 4.
pub(crate) fn write_high_process(
    out: &mut SaveWriter,
    world: &World,
    process: &ProcessView<'_>,
) -> Result<(), GameError> {
    write_middle_high_process(out, world, process)?;
    let h = process.high;
    out.u8(h.dialogue_state_byte_228);
    out.u8(h.byte_23c);
    out.u8(h.byte_25c);
    out.u8(h.byte_25d);
    out.u16(h.movement_flags_1fc);
    out.u16(h.current_action_1f4);
    out.u16(h.word_208);
    out.u32(h.last_bone_lod_1f0 as u32);
    out.f32(h.package_evaluation_timer_1ac);
    out.f32(h.detection_timer_204);
    out.f32(h.float_21c);
    out.f32(h.death_state_timer_22c);
    out.f32(h.idle_animation_timer_230);
    out.f32(h.float_234);
    out.f32(h.float_248);
    out.f32(h.float_198);
    out.f32(h.float_1b0);
    out.f32(h.float_1b4);
    out.f32(h.float_19c);
    out.u32(h.field_1cc);
    out.u8(h.byte_244);
    out.words(&h.point_20c);
    out.u32(h.unset_254);
    out.u32(h.field_2bc);
    out.f32(h.float_2c0);
    out.f32(h.swim_breath_238);
    out.f32(h.float_1a8);
    out.f32(h.float_1b8);
    out.f32(h.float_260);
    out.u8(h.byte_278);
    out.u32(h.field_274);
    out.u8(h.byte_290);
    out.f32(h.float_28c);
    out.u32(null_id(h.field_218, "00632815: the +0x218 form's ID")?);
    out.u32(null_id(h.field_1a4, "00632836: the +0x1A4 form's ID")?);
    out.u32(null_id(h.field_2c4, "0063285D: the +0x2C4 form's ID")?);
    empty_list(
        h.detection_list_18c.unwrap_or_default(),
        "0063289E: a detection entry",
    )?;
    out.u16(0);
    let size: u16 = controller(world, process).map_or(0, |_| 76);
    out.u16(size);
    if let Some(controller) = controller(world, process) {
        write_controller(out, controller)?;
    }
    for index in 0..5 {
        out.u32(null_id(
            h.action_targets_2c8[index],
            "006329CB: an action target",
        )?);
        out.u8(h.action_active_2dc[index]);
    }
    out.u32(null_id(h.field_2e4, "00632A1B: the +0x2E4 form's ID")?);
    out.u8(h.byte_2e8);
    out.f32(h.float_2ac);
    out.f32(h.float_2b0);
    out.u32(null_id(h.field_258, "00632A96: the +0x258 form's ID")?);
    out.u8(h.byte_1d0);
    out.f32(h.float_1d8);
    out.f32(h.float_1dc);
    out.f32(h.movement_flags_timer_1e0);
    out.u8(h.byte_2a8);
    out.u8(h.timed_movement_flags_1e4);
    out.f32(h.float_1e8);
    out.f32(h.float_240);
    out.u8(h.byte_2a8);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tes::form::FormIdentity;
    use crate::tes::forms_ai::TesPackage;

    #[test]
    fn fade_guard_consumes_the_pending_update_only_after_the_package_check() {
        let mut world = World::new();
        let base_form = world
            .spawn(FormIdentity {
                form_type_04: 0x23,
                flags_08: 0x400,
                form_id_0c: 1,
            })
            .id();
        let package = world
            .spawn(TesPackage {
                type_20: 2,
                ..Default::default()
            })
            .id();
        let process = world
            .spawn((
                BaseProcess {
                    editor_package_08: Some(package),
                    edit_pack_procedure_type_04: 0,
                },
                MiddleHighProcess {
                    full_update_pending_16d: 1,
                    ..Default::default()
                },
            ))
            .id();
        let actor = world
            .spawn((
                TesObjectRefr {
                    base_form_1c: Some(base_form),
                    ..Default::default()
                },
                MobileObject { process_58: None },
            ))
            .id();
        assert!(!should_skip_high_update_beyond_fade_distance(&mut world, actor).unwrap());
        world.get_mut::<FormIdentity>(base_form).unwrap().flags_08 = 0;
        world.get_mut::<MobileObject>(actor).unwrap().process_58 = Some(process);
        assert!(!should_skip_high_update_beyond_fade_distance(&mut world, actor).unwrap());
        assert_eq!(
            world
                .get::<MiddleHighProcess>(process)
                .unwrap()
                .full_update_pending_16d,
            1
        );
        world.get_mut::<TesPackage>(package).unwrap().type_20 = 6;
        assert!(!should_skip_high_update_beyond_fade_distance(&mut world, actor).unwrap());
        assert_eq!(
            world
                .get::<MiddleHighProcess>(process)
                .unwrap()
                .full_update_pending_16d,
            1
        );
        world
            .get_mut::<BaseProcess>(process)
            .unwrap()
            .edit_pack_procedure_type_04 = 1;
        assert!(!should_skip_high_update_beyond_fade_distance(&mut world, actor).unwrap());
        assert_eq!(
            world
                .get::<MiddleHighProcess>(process)
                .unwrap()
                .full_update_pending_16d,
            0
        );
    }
}
