//! HighProcess's package and procedure update on a loaded actor.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::actor::{Actor, Character, TesObjectRefr};
use super::actor_animation::ActorAnimData;
use super::actor_process::{
    BaseProcess, HighProcess, LowProcess, MiddleHighProcess, actor_light_level,
};
use crate::bs::fade_node::FadeNodeSettings;
use crate::bs::scene_graphs::{SceneGraph, SceneGraphGlobals};
use crate::gamebryo::bone_lod::{BoneLodController, set_bone_lod};
use crate::gamebryo::camera::SceneCamera;
use crate::gamebryo::scene_node::{NiNodeChildren, SceneNodePayload};
use crate::tes::actor_package::actor_in_combat_package;
use crate::tes::ai_package::{package_data, package_is_combat_like, procedure_at};
use crate::tes::forms_init_world::reference_ni_node;
use crate::tes::forms_references::form_flags;
use crate::tes::forms_references::parent_cell;
use crate::tes::forms_references::reference_position;
use crate::tes::forms_world::{TesObjectCell, cell_is_interior};
use crate::tes::game_settings::GameSettingCollection;
use crate::tes::idle_frame::IdleFrameGlobals;
use crate::tes::script_vm::ScriptVmGlobals;

// IDA: 00635E20 HighProcess_UpdateBoneLOD. The distance uses the first scene child's
// local translation. The LOD adjustment comes from the scene graph's +DC camera.
// Completion: 100% of selection and the store after SetLOD.
fn update_bone_lod(
    world: &mut World,
    process: Entity,
    actor: Entity,
    controller: Entity,
) -> Result<(), GameError> {
    let graph = world
        .resource::<SceneGraphGlobals>()
        .world
        .ok_or(GameError::unclassified(
            0x00635E20,
            "no world scene graph for bone LOD",
        ))?;
    let child = world
        .get::<NiNodeChildren>(graph)
        .and_then(|c| c.slots_ac.first().copied().flatten())
        .ok_or(GameError::unclassified(
            0x00635E52,
            "no first world scene child for bone LOD",
        ))?;
    let origin = world
        .get::<SceneCamera>(child)
        .map(|c| c.local_transform_30.translation)
        .or_else(|| {
            world
                .get::<SceneNodePayload>(child)
                .map(|n| n.local_transform.translation)
        })
        .ok_or(GameError::unclassified(
            0x00635E52,
            "first world scene child has no local transform",
        ))?;
    let position = reference_position(world, actor)?;
    let [x, y, z] = std::array::from_fn(|i| position[i] - origin[i]);
    let distance = ((y * y + x * x) + z * z).sqrt().abs();
    let camera = world
        .get::<SceneGraph>(graph)
        .and_then(|g| g.camera_dc)
        .and_then(|e| world.get::<SceneCamera>(e))
        .ok_or(GameError::unclassified(
            0x00635EBA,
            "world scene graph has no camera for bone LOD",
        ))?;
    let actor_multiplier = world
        .resource::<FadeNodeSettings>()
        .actor_lod_multiplier_b0762c
        * world.resource::<IdleFrameGlobals>().lod_fade_actor_b075f0;
    let bone_distance = world
        .resource::<GameSettingCollection>()
        .raw_at(0x00B367A0)
        .ok_or(GameError::unclassified(
            0x00635ED4,
            "missing iBoneLODDistMult",
        ))? as i32;
    let selected = oblivion_core::x87::ftol(
        (distance * 12.0 * camera.lod_adjust_120) / (actor_multiplier * bone_distance as f32),
    );
    let count = world
        .get::<BoneLodController>(controller)
        .expect("a bone LOD controller")
        .lod_count_40 as i32;
    let lod = if selected < count { selected } else { -1 };
    if lod != world.get::<HighProcess>(process).unwrap().last_bone_lod_1f0 {
        set_bone_lod(world, controller, lod);
        world
            .get_mut::<HighProcess>(process)
            .unwrap()
            .last_bone_lod_1f0 = lod;
    }
    Ok(())
}

// IDA: 005E8B80 Actor_ShouldUseTorch, from 00603160 Actor_UpdateTorchAndShield.
// Completion: the interior threshold, retained-torch hysteresis and evaluation timer.
// TODO: [ ] Exterior sky check and equipment mutations when the inventory supplies a torch.
fn update_torch_and_shield(
    world: &mut World,
    process: Entity,
    actor: Entity,
    cell: Entity,
    anim: Entity,
) -> Result<(), GameError> {
    if crate::tes::player::the_player(world) == Some(actor) {
        return Ok(());
    }
    let middle = world.get::<MiddleHighProcess>(process).unwrap();
    let equipped = middle.light_e8;
    let drawn = middle.weapon_drawn_115 != 0;
    let two_handed = middle.weapon_e4.filter(|e| e.worn).is_some_and(|w| {
        matches!(
            crate::tes::forms_items::weapon_type_of(world, w.form_08),
            1 | 3..=5
        )
    });
    let torch_idle = world
        .get::<ActorAnimData>(anim)
        .unwrap()
        .lookup(33)
        .is_some();
    let should_use = if !torch_idle {
        false
    } else if actor_in_combat_package(world, actor, true)? {
        equipped.is_some_and(|e| e.worn)
    } else {
        let _hour = crate::tes::global::game_hour(world);
        let night_eye = crate::tes::actor::actor_get_actor_value(world, actor, 41)?;
        let setting = |world: &World, address| {
            world
                .resource::<GameSettingCollection>()
                .raw_at(address)
                .map(f32::from_bits)
                .ok_or(GameError::unclassified(
                    0x005E8B80,
                    "missing torch evaluation setting",
                ))
        };
        let bonus = if night_eye > 0 {
            setting(world, 0x00B37A50)?
        } else {
            0.0
        };
        if !cell_is_interior(world, cell) {
            return Err(GameError::unclassified(
                0x005E8D00,
                "torch lighting in an exterior cell",
            ));
        }
        let mut retain = false;
        let use_torch = if crate::tes::forms_world::cell_resolve_music_type(world, cell)? == 2 {
            let level = actor_light_level(world, process, actor, true)?.min(100.0) - bonus;
            let threshold = setting(world, 0x00B37740)?;
            retain = level >= threshold
                && equipped.is_some_and(|e| e.worn)
                && (threshold + threshold) + threshold > level;
            level < threshold || retain
        } else {
            false
        };
        let timer = setting(world, 0x00B37A58)?;
        world
            .get_mut::<MiddleHighProcess>(process)
            .unwrap()
            .torch_evaluation_timer_bc = if retain { timer * 5.0 } else { timer };
        use_torch
    };
    if should_use && (!drawn || !two_handed) {
        if let Some(entry) = equipped {
            if !entry.worn {
                return Err(GameError::unclassified(0x00603160, "equip selected torch"));
            }
        } else if super::equipment::find_torch_inventory_entry(world, actor)?.is_some() {
            return Err(GameError::unclassified(0x00603160, "equip carried torch"));
        }
    } else if !should_use && equipped.is_some_and(|e| e.worn) {
        return Err(GameError::unclassified(0x00603160, "unequip actor torch"));
    }
    Ok(())
}

// IDA: 00603320 Actor_TryRestoreHealthOutOfCombat. 0051AC80 tests the low byte's group-table kind.
// Completion: the return paths before spell or inventory-based health recovery.
fn try_restore_health_out_of_combat(
    world: &mut World,
    actor: Entity,
    anim: Entity,
) -> Result<(), GameError> {
    let group = super::actor_anim_play::section_group(world, anim, 1);
    let attacking = group != 0xff
        && super::anim_group::ANIM_GROUPS
            .get(usize::from(group as u8))
            .is_some_and(|row| matches!(row.kind_0c, 4..=7));
    if actor_in_combat_package(world, actor, true)?
        || matches!(world.get::<Actor>(actor).unwrap().dead_state_b0, 3 | 5)
        || attacking
    {
        return Ok(());
    }
    let health = crate::tes::actor::actor_get_actor_value(world, actor, 8)?;
    if health < super::player::floor_calculated_actor_value(world, actor, 8)? {
        return Err(GameError::unclassified(
            0x006033B9,
            "out-of-combat spell or item health recovery",
        ));
    }
    Ok(())
}

// IDA: 0064B020 MiddleHighProcess_GetCurrentPackageProcedure.
// Completion: 100% of both package slots.
fn current_procedure_index(world: &World, process: Entity) -> u32 {
    let middle = world.get::<MiddleHighProcess>(process).unwrap();
    if middle.current_package_c0.is_some() {
        middle.procedure_type_cc
    } else {
        world
            .get::<BaseProcess>(process)
            .unwrap()
            .edit_pack_procedure_type_04
    }
}

// IDA: 00634DB0 HighProcess_UpdateIdleAnimationTimer.
// Completion: distance and animation guards, and the running timer.
// TODO: [ ] Idle playback and the random delay when the timer expires.
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "the x87 compare: a NaN takes the negated arm"
)]
fn update_idle_animation_timer(
    world: &mut World,
    process: Entity,
    actor: Entity,
    anim: Entity,
    seconds: f32,
) -> Result<(), GameError> {
    let player = crate::tes::player::the_player(world).expect("the player during High update");
    let distance = crate::tes::forms_references::reference_distance(world, actor, player, false)?;
    let limit = world
        .resource::<GameSettingCollection>()
        .raw_at(0x00B36CC8)
        .map(f32::from_bits)
        .ok_or(GameError::unclassified(
            0x00634DC5,
            "missing fAIIdleAnimationDistance",
        ))?;
    if !(limit >= distance) {
        return Ok(());
    }
    let package = super::character_3d::actor_current_package(world, actor);
    let data = world.get::<ActorAnimData>(anim).unwrap();
    if data.queued_idle_d0.is_some() {
        return Ok(());
    }
    if data.current_idle_cc.is_some() {
        return Err(GameError::unclassified(
            0x00472EB9,
            "CanStartDialogueIdle on an active AnimIdle",
        ));
    }
    let index = current_procedure_index(world, process);
    let editor = world.get::<BaseProcess>(process).unwrap().editor_package_08;
    if let Some(editor) = editor
        && package_data(world, editor).type_20 == 3
        && procedure_at(world, editor, index)? == 5
    {
        return Ok(());
    }
    if world
        .get::<MiddleHighProcess>(process)
        .unwrap()
        .sleep_state_11d
        == 9
        || crate::tes::actor::actor_get_actor_value(world, actor, 48)? != 0
    {
        return Ok(());
    }
    let idle_group = super::actor_anim_play::section_sequence(world, anim, 0)
        .and_then(|seq| world.get::<crate::bs::anim_group_sequence::AnimGroupSequence>(seq))
        .and_then(|s| world.get::<super::anim_group::TesAnimGroup>(s.group_68))
        .is_some_and(|g| matches!(g.group_type(), 0 | 1 | 27 | 33));
    if !idle_group {
        let Some(p) = package else { return Ok(()) };
        if actor == player
            || package_data(world, p).field_18 == u32::MAX
            || procedure_at(world, p, index)? != 1
        {
            return Ok(());
        }
    }
    let timer = world
        .get::<HighProcess>(process)
        .unwrap()
        .idle_animation_timer_230;
    if !(timer >= 0.0) {
        let base = world
            .get::<TesObjectRefr>(actor)
            .unwrap()
            .base_form_1c
            .ok_or(GameError::unclassified(
                0x005E1FBC,
                "idle animation queried energy without an actor base",
            ))?;
        let energy = crate::tes::actor_values::base_actor_value(world, base, 35)? as f32;
        let settings = world.resource::<GameSettingCollection>();
        let setting = |ea| {
            settings
                .raw_at(ea)
                .map(f32::from_bits)
                .ok_or(GameError::unclassified(
                    0x00546770,
                    "missing idle animation energy setting",
                ))
        };
        let adjusted = energy * setting(0x00B36988)? + setting(0x00B36980)?;
        let wait = adjusted + setting(0x00B36998)?;
        let fraction = ((100.0 - wait) / 100.0).abs();
        if fraction > 0.0 && 60.0 - fraction * 40.0 > 0.0 {
            return Err(GameError::unclassified(
                0x00634ED0,
                "select idle animation and reset its randomized delay",
            ));
        }
    }
    world
        .get_mut::<HighProcess>(process)
        .unwrap()
        .idle_animation_timer_230 = timer - seconds;
    Ok(())
}

// IDA: 0064F650 LowProcess_RefreshPackageTargetReference.
// Completion: the enabled target can replace an existing, different reference.
fn refresh_package_target(
    world: &mut World,
    process: Entity,
    actor: Entity,
) -> Result<(), GameError> {
    if let Some(package) = super::character_3d::actor_current_package(world, actor)
        && let Some(target) = package_data(world, package).target_28.as_ref()
        && let Some(reference) = crate::tes::ai_package::target_reference(world, target)?
        && form_flags(world, reference) & 0x20 == 0
    {
        let mut low = world.get_mut::<LowProcess>(process).unwrap();
        if low.field_2c.is_some_and(|old| old != reference) {
            low.field_2c = Some(reference);
        }
    }
    Ok(())
}

// IDA: 0064F6A0 MiddleHighProcess_AdvancePackageProcedure.
// Completion: both index stores, clamping and the end-of-package script event.
fn advance_package_procedure(
    world: &mut World,
    process: Entity,
    actor: Entity,
    step: i32,
) -> Result<(), GameError> {
    let package = super::character_3d::actor_current_package(world, actor);
    world.get_mut::<HighProcess>(process).unwrap().byte_25d = 0;
    let mut middle = world.get_mut::<MiddleHighProcess>(process).unwrap();
    let running = middle.current_package_c0.is_some();
    if running {
        middle.procedure_type_cc = middle.procedure_type_cc.wrapping_add_signed(step);
    }
    let mut base = world.get_mut::<BaseProcess>(process).unwrap();
    if !running {
        base.edit_pack_procedure_type_04 =
            base.edit_pack_procedure_type_04.wrapping_add_signed(step);
    }
    if (base.edit_pack_procedure_type_04 as i32) < 0 {
        base.edit_pack_procedure_type_04 = 0;
    }
    if let Some(package) = package {
        let count =
            crate::tes::ai_process::procedure_count(package_data(world, package).field_18 as i32);
        if current_procedure_index(world, process) as i32 >= count {
            let last = (count - 1) as u32;
            if running {
                world
                    .get_mut::<MiddleHighProcess>(process)
                    .unwrap()
                    .procedure_type_cc = last;
            } else {
                world
                    .get_mut::<BaseProcess>(process)
                    .unwrap()
                    .edit_pack_procedure_type_04 = last;
            }
            if procedure_at(world, package, last)? == 44 {
                crate::tes::ai_package::script_event_or(world, actor, 0x400, Some(package));
            }
        }
    }
    Ok(())
}

// IDA: 00635F10 HighProcess_UpdateTravelProcedure, procedure zero at 0063ACF0.
// Completion: arrival at an actor's starting location without a path or furniture.
// TODO: [ ] Moving actors, reference markers, path construction and furniture.
fn update_travel_procedure(
    world: &mut World,
    process: Entity,
    actor: Entity,
    package: Entity,
) -> Result<(), GameError> {
    use crate::tes::ai_package::{
        location_reference, package_at_location, package_location_position,
    };
    let data = package_data(world, package);
    let (kind, location, flags, has_target) = (
        data.type_20,
        data.location_24,
        data.flags_1c,
        data.target_28.is_some(),
    );
    if kind == 9 {
        return Err(GameError::unclassified(
            0x00635F3A,
            "travel to a package target",
        ));
    }
    let point = package_location_position(world, package, actor)?;
    let _distance =
        crate::tes::forms_references::distance_to(reference_position(world, actor)?, point);
    let location_ref = location
        .as_ref()
        .map(|l| location_reference(world, l))
        .transpose()?
        .flatten();
    let low = world.get::<LowProcess>(process).unwrap();
    let running = world
        .get::<MiddleHighProcess>(process)
        .unwrap()
        .current_package_c0;
    let destination = if running.is_none() {
        low.field_30.or(location_ref)
    } else {
        location_ref
    };
    if destination.is_some() {
        return Err(GameError::unclassified(
            0x00635F9F,
            "travel with a destination reference",
        ));
    }
    let at_location = package_at_location(world, package, actor, false, -1.0)?;
    // 005677B0, radius kind 1: +4 unless zero, then ten for type 3, otherwise the GMST.
    let radius = location.map_or(0, |l| l.radius() as i32);
    let _path_radius = if radius != 0 {
        radius
    } else if location.is_some_and(|l| l.type_00 == 3) {
        10
    } else {
        world
            .resource::<GameSettingCollection>()
            .raw_at(0x00B36B28)
            .ok_or(GameError::unclassified(
                0x005677B0,
                "missing iAIDistanceRadiusMinLocation",
            ))? as i32
    };
    if !at_location {
        return Err(GameError::stop(
            0x006364B7,
            "travel path to an unreached package location",
        ));
    }
    if world.get::<HighProcess>(process).unwrap().field_1cc != 0 {
        return Err(GameError::unclassified(
            0x00636B36,
            "travel arrival with a pending target",
        ));
    }
    if world.get::<LowProcess>(process).unwrap().pathing_34 != 0 {
        return Err(GameError::unclassified(
            0x0064ADA0,
            "travel arrival checks an existing path",
        ));
    }
    // 00629EE0 first dispatches 00629B50, then 00643740, then stops a mounted horse.
    world
        .get_mut::<HighProcess>(process)
        .unwrap()
        .movement_flags_1fc &= 0xfcc0;
    world.get_mut::<MiddleHighProcess>(process).unwrap().byte_d0 = 1;
    if let Some(builder) = world
        .get_resource::<crate::tes::pathing::PathingSingletons>()
        .and_then(|p| p.path_builder_b3bf80.as_ref())
        && (builder.thread_request_40 != 0
            || builder.actor_maps_10.iter().any(|map| map.count_0c != 0))
    {
        return Err(GameError::unclassified(
            0x006826D0,
            "cancel an actor's pending path request",
        ));
    }
    if world.get::<Character>(actor).is_some()
        && world.get::<Actor>(actor).unwrap().horse_or_rider_d4 != 0
    {
        return Err(GameError::unclassified(
            0x00629F2A,
            "stop the actor's mounted horse",
        ));
    }
    if location.is_some_and(|l| l.type_00 == 3)
        && world
            .get::<MiddleHighProcess>(process)
            .unwrap()
            .sleep_state_11d
            == 0
        && world.get::<HighProcess>(process).unwrap().byte_2e8 == 0
    {
        let angle = world.get::<Actor>(actor).unwrap().starting_z_rotation_f4;
        let tau = std::f32::consts::TAU;
        let target = if angle < 0.0 {
            oblivion_core::x87::float_fmod(angle, tau) + tau
        } else if angle >= tau {
            oblivion_core::x87::float_fmod(angle, tau)
        } else {
            angle
        };
        let heading = world.get::<TesObjectRefr>(actor).unwrap().rotation_20[2];
        let delta = target - heading;
        let difference = if delta > std::f32::consts::PI {
            tau - delta
        } else if delta <= -std::f32::consts::PI {
            delta + tau
        } else {
            delta
        };
        let limit = world
            .resource::<GameSettingCollection>()
            .raw_at(0x00B36C18)
            .ok_or(GameError::unclassified(
                0x00636CF0,
                "missing iActorKeepTurnDegree",
            ))? as i32;
        if (limit as f32).to_radians() < difference.abs() {
            return Err(GameError::unclassified(
                0x00636D18,
                "turn the actor to its travel destination heading",
            ));
        }
        world
            .get_mut::<HighProcess>(process)
            .unwrap()
            .movement_flags_1fc &= !0x30;
    }
    advance_package_procedure(world, process, actor, 1)?;
    if flags & 2 != 0 && flags & 0x400 == 0 && !has_target {
        world
            .get_mut::<HighProcess>(process)
            .unwrap()
            .package_evaluation_timer_1ac = 0.0;
    }
    if kind == 6 && flags & 0x400 != 0 && package_data(world, package).schedule_2c.duration_04 == 0
    {
        let day = crate::tes::global::game_day_byte(world);
        crate::tes::base_extra_list::run_once_add(world, actor, package, day);
    }
    if world.get::<LowProcess>(process).unwrap().byte_84 == 0 {
        if flags & 0x20 != 0 {
            crate::tes::data_handler_world::doors_near(world, actor, true)?;
        }
        world.get_mut::<LowProcess>(process).unwrap().byte_84 = 1;
    }
    if flags & 0x100 != 0 {
        crate::tes::data_handler_world::doors_near(world, actor, false)?;
    }
    if world.get::<MiddleHighProcess>(process).unwrap().bytes_168[1] == 0 && flags & 0x300000 != 0 {
        return Err(GameError::unclassified(
            0x00636E50,
            "adjust equipment on travel arrival",
        ));
    }
    Ok(())
}

// IDA: 0063A210 HighProcess_UpdateActorPackageAndProcedure.
// Completion: loaded living actors through the first at-location travel procedure.
// TODO: [ ] Further procedures, animation groups and path cleanup.
pub(super) fn update_actor_package_and_procedure(
    world: &mut World,
    process: Entity,
    actor: Entity,
) -> Result<(), GameError> {
    if reference_ni_node(world, actor).is_none() || form_flags(world, actor) & 0x820 != 0 {
        return Ok(());
    }
    let Some(cell) = parent_cell(world, actor) else {
        return Ok(());
    };
    let high = world.get::<HighProcess>(process).unwrap();
    if world.get::<TesObjectCell>(cell).unwrap().process_level_26 != 6
        || matches!(high.field_2bc, 3 | 4)
    {
        return Ok(());
    }
    let Some(anim) = world
        .get::<MiddleHighProcess>(process)
        .unwrap()
        .anim_data_17c
    else {
        return Ok(());
    };
    if !world
        .get::<ActorAnimData>(anim)
        .unwrap()
        .deferred_b4
        .is_empty()
    {
        return Ok(());
    }
    let seconds = world.resource::<ScriptVmGlobals>().frame_seconds_b33e9c;
    let mut high = world.get_mut::<HighProcess>(process).unwrap();
    let skipped = high.skipped_high_update_beyond_fade_distance_1d1;
    let timer = high.death_state_timer_22c - seconds;
    high.skipped_high_update_beyond_fade_distance_1d1 = 0;
    high.death_state_timer_22c = timer;
    if skipped != 0 {
        super::actor_placement::actor_place_on_path_grid(world, actor)?;
    }
    if let Some(controller) = world
        .get::<HighProcess>(process)
        .unwrap()
        .bone_lod_controller_1ec
        && reference_ni_node(world, actor).is_some()
        && !(world.get::<Character>(actor).is_some()
            && world.get::<Actor>(actor).unwrap().horse_or_rider_d4 != 0)
    {
        update_bone_lod(world, process, actor, controller)?;
    }
    if world.get::<HighProcess>(process).unwrap().byte_1d0 != 0 {
        return Err(GameError::unclassified(
            0x0063A355,
            "High actor's pending +1D0 update",
        ));
    }
    // The port has no writer of 00B3C0AB (zero in the captured frames).
    if world.get::<MiddleHighProcess>(process).unwrap().bytes_168[2] != 0 {
        return Err(GameError::unclassified(0x0063A36F, "actor +16A update"));
    }
    if world.get::<Actor>(actor).unwrap().dead_state_b0 == 3 {
        return Err(GameError::unclassified(
            0x0063A393,
            "death-state 3 animation update",
        ));
    }
    if !crate::tes::forms_references::save_load_active(world)?
        && actor_in_combat_package(world, actor, true)?
    {
        return Err(GameError::unclassified(
            0x0063A3BE,
            "combat controller and package target",
        ));
    }
    if world.get::<HighProcess>(process).unwrap().byte_1a0 != 0 {
        return Err(GameError::unclassified(
            0x0063A42C,
            "pending dialogue menu cleanup",
        ));
    }
    let torch_timer = world
        .get::<MiddleHighProcess>(process)
        .unwrap()
        .torch_evaluation_timer_bc;
    // 0063A466: 005E06C0 at half the height, with the actor's own position.
    let position = world.get::<TesObjectRefr>(actor).unwrap().position_2c;
    if super::actor_movement_update::submerged_at_height_fraction(
        world,
        actor,
        position,
        Some(cell),
        0.5,
    )? {
        if world
            .get::<MiddleHighProcess>(process)
            .unwrap()
            .light_e8
            .is_some()
        {
            return Err(GameError::unclassified(
                0x0063A487,
                "unequip a submerged actor's light",
            ));
        }
    } else if torch_timer > 0.0
        && (cell_is_interior(world, cell)
            || world
                .resource::<GameSettingCollection>()
                .raw_at(0x00B37A58)
                .is_some_and(|bits| f32::from_bits(bits) >= torch_timer))
    {
        world
            .get_mut::<MiddleHighProcess>(process)
            .unwrap()
            .torch_evaluation_timer_bc -= seconds;
    } else {
        update_torch_and_shield(world, process, actor, cell, anim)?;
    }
    let mut high = world.get_mut::<HighProcess>(process).unwrap();
    if high.float_2b0 <= 0.0 {
        high.float_2ac = 0.0;
    } else {
        high.float_2b0 -= seconds;
    }
    if high.byte_290 != 0 {
        return Err(GameError::unclassified(
            0x0063A521,
            "submerged actor's +28C countdown",
        ));
    }
    let is_player = crate::tes::player::the_player(world) == Some(actor);
    if !is_player {
        try_restore_health_out_of_combat(world, actor, anim)?;
    }
    let mounted = world.get::<Character>(actor).is_some()
        && world.get::<Actor>(actor).unwrap().horse_or_rider_d4 != 0;
    let current = super::character_3d::actor_current_package(world, actor);
    if !is_player && mounted && current.is_some_and(|p| package_data(world, p).type_20 == 22) {
        return Err(GameError::unclassified(
            0x0063A5C3,
            "horse ownership for a mounted package",
        ));
    }
    let duration = world.get::<HighProcess>(process).unwrap().float_260;
    let mut force = false;
    if duration != 0.0 {
        let hour = crate::tes::global::game_hour(world);
        let elapsed = (hour - world.get::<LowProcess>(process).unwrap().field_0c).abs();
        if duration <= elapsed * 100.0 {
            world
                .get_mut::<HighProcess>(process)
                .unwrap()
                .package_evaluation_timer_1ac = 0.0;
            advance_package_procedure(world, process, actor, 3)?;
            force = true;
            crate::tes::ai_package::script_event_or(world, actor, 0x400, current);
            if let Some(p) = current
                && package_data(world, p).flags_1c & 0x400 != 0
            {
                let day = crate::tes::global::game_day_byte(world);
                crate::tes::base_extra_list::run_once_add(world, actor, p, day);
            }
        }
    }
    let editor = world.get::<BaseProcess>(process).unwrap().editor_package_08;
    if let Some(package) = editor {
        force = package_data(world, package).flags_1c & 0x8000 != 0;
    }
    let running = world
        .get::<MiddleHighProcess>(process)
        .unwrap()
        .current_package_c0;
    let changed = !is_player
        && running.is_none()
        && editor.is_none_or(|p| !package_is_combat_like(package_data(world, p)))
        && super::actor_process_high::evaluate_high_package(world, process, actor, seconds, force)?;
    let current = super::character_3d::actor_current_package(world, actor);
    let middle = world.get::<MiddleHighProcess>(process).unwrap();
    if changed || (current.is_none() && middle.byte_d0 == 0) {
        return Err(GameError::unclassified(
            0x0063A70D,
            "start a newly selected High package",
        ));
    }
    let sleep = middle.sleep_state_11d;
    if middle.current_package_c0.is_none()
        && (matches!(sleep, 5 | 10) || (mounted && matches!(sleep, 2 | 3)))
    {
        return Err(GameError::unclassified(
            0x0063A7C6,
            "change mount or furniture on a sleeping package",
        ));
    }
    update_idle_animation_timer(world, process, actor, anim, seconds)?;
    let base = world.get::<BaseProcess>(process).unwrap();
    if running.is_none()
        && current.is_some_and(|p| !package_is_combat_like(package_data(world, p)))
        && let Some(editor) = base.editor_package_08
        && procedure_at(world, editor, base.edit_pack_procedure_type_04)? != 0
    {
        let flags = package_data(world, current.unwrap()).flags_1c;
        if world.get::<HighProcess>(process).unwrap().byte_25c != 0 || flags & 0x201 == 0x201 {
            return Err(GameError::unclassified(
                0x0063A8AE,
                "package reevaluation with active procedure",
            ));
        }
    }
    let mut high = world.get_mut::<HighProcess>(process).unwrap();
    if high.float_248 > 0.0 {
        high.float_248 -= seconds;
        if high.float_248 <= 0.0 {
            return Err(GameError::unclassified(
                0x0063A93F,
                "expired High package timer at +248",
            ));
        }
    }
    refresh_package_target(world, process, actor)?;
    if !is_player {
        let want_drawn = world.get::<LowProcess>(process).unwrap().byte_1f != 0
            || world.get::<HighProcess>(process).unwrap().byte_244 != 0;
        let drawn = world
            .get::<MiddleHighProcess>(process)
            .unwrap()
            .weapon_drawn_115
            != 0;
        if (want_drawn && !drawn)
            || (!want_drawn && drawn && !actor_in_combat_package(world, actor, true)?)
        {
            return Err(GameError::unclassified(
                0x005E6D70,
                "change the actor's weapon draw state",
            ));
        }
    }
    if let Some(p) = current
        && package_data(world, p).type_20 == 18
        && let Some(target) = world.get::<LowProcess>(process).unwrap().field_2c
        && super::actor::actor_process_level(world, target) != 0
    {
        return Err(GameError::unclassified(
            0x0063AA6E,
            "package target changed process level",
        ));
    }
    if mounted || !matches!(sleep, 0 | 4 | 9) {
        return Err(GameError::unclassified(
            0x0063AA94,
            "mounted or sleeping package procedure",
        ));
    }
    if let Some(p) = current
        && package_data(world, p).field_18 != u32::MAX
    {
        // DebugText_FindLineValue at 0063ABD4 only supplies the diagnostic text on a later branch.
        let index = current_procedure_index(world, process);
        let procedure = procedure_at(world, p, index)?;
        if package_data(world, p)
            .location_24
            .is_some_and(|l| l.type_00 > 3)
            && procedure != 44
        {
            return Err(GameError::unclassified(
                0x0063ABF0,
                "search-type package location",
            ));
        }
        match procedure {
            0 => update_travel_procedure(world, process, actor, p)?,
            9 | 11 | 19 | 25 => {}
            _ => {
                return Err(GameError::stop(
                    0x0063ACD5,
                    "High package procedure dispatch",
                ));
            }
        }
    }
    if reference_ni_node(world, actor).is_some()
        && world
            .get::<super::player::PlayerCharacter>(actor)
            .is_none_or(|p| p.view_flags_588[0] != 0)
    {
        world.resource_scope(|world, tiles: Mut<crate::tes::tile::TileEngine>| {
            super::actor_anim_play::update_animation_groups(world, &tiles, actor)
        })?;
    }
    let background = world
        .get_resource::<crate::tes::settings::SettingCollections>()
        .and_then(|s| s.ini.find_address(0x00B15800))
        .is_none_or(|setting| setting.as_bool());
    if background
        && let Some(builder) = world
            .get_resource::<crate::tes::pathing::PathingSingletons>()
            .and_then(|p| p.path_builder_b3bf80.as_ref())
        && (builder.thread_request_40 != 0
            || builder.actor_maps_10.iter().any(|map| map.count_0c != 0))
    {
        return Err(GameError::stop(
            0x006825C0,
            "resolve pending path requests for the High actor",
        ));
    }
    let process =
        crate::tes::actor::process_of(world, actor).expect("High actor process after animation");
    crate::tes::actor_process::process_update_stamps(world, process);
    Ok(())
}
