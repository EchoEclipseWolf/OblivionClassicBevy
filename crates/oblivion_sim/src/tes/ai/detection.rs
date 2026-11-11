//! An actor's detection of another: 005F6540 Actor_GetDetectionLevel and its line-of-sight test, 005F2820
//! Actor_TestLineOfSight. A cached detection state (process vt+0x3B0) answers its level; otherwise the target's light,
//! line of sight, noise, sneak and visibility feed 005463F0's formula, and the observer's process records the result
//! (vt+0xA8). Evidence: process-manager-2026-11-10/high-movement (calls.tsv: every living High actor's 00603CA0 calls
//! 005F6540 at 00604F68) and living-light (the player's light sampled from it).

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use crate::tes::actor::TesObjectRefr;
use crate::tes::actor::process_of;
use crate::tes::player::the_player;

/// IDA: 00B3BD7C g_detectionPicksThisFrame, the line-of-sight picks made this frame (005F2820 counts each; the frame's
/// start clears it), and 00B333C0, 005F6540's count of uncached detections.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetectionGlobals {
    pub picks_this_frame_b3bd7c: i32,
    pub uncached_detections_b333c0: u32,
}

// IDA: 00683CB0 Vector_GetHeadingAngle(delta): with y 0, 4.712389f for x <= 0 else 1.5707964f; otherwise atan(x / y) (the
// quotient and the arctangent each stored a float), +pi for a negative y, then a negative angle + 2pi through fmod, an
// angle of 2pi or more through fmod. Completion: 100%.
fn heading_angle(delta: [f32; 3]) -> f32 {
    const TWO_PI: f64 = 6.283_185_482_025_146_5;
    if delta[1] == 0.0 {
        return if delta[0] <= 0.0 {
            4.712_389
        } else {
            1.570_796_4
        };
    }
    let quotient = delta[0] / delta[1];
    let mut angle = f64::from(quotient).atan() as f32;
    if delta[1] < 0.0 {
        angle = (f64::from(angle) + 3.141_592_741_012_573_2) as f32;
    }
    if angle < 0.0 {
        let wrapped = (f64::from(angle) + TWO_PI) as f32;
        return (f64::from(wrapped) % TWO_PI) as f32;
    }
    if f64::from(angle) >= TWO_PI {
        return (f64::from(angle) % TWO_PI) as f32;
    }
    angle
}

// IDA: 00683D60 Angle_GetHeadingDifference(current, target): the difference (a float); zero answers it; above pi (a double
// compare) answers 2pi minus it, at or below -pi plus 2pi. The turn direction is unused here. Completion: 100%.
fn heading_difference(current: f32, target: f32) -> f32 {
    let difference = target - current;
    if difference == 0.0 {
        return difference;
    }
    let wide = f64::from(difference);
    if difference >= 0.0 {
        if wide > 3.141_592_741_012_573_2 {
            return (6.283_185_482_025_146_5 - wide) as f32;
        }
    } else if wide <= -3.141_592_741_012_573_2 {
        return (wide + 6.283_185_482_025_146_5) as f32;
    }
    difference
}

/// IDA: 005F2820 Actor_TestLineOfSight(observer, picks, target, eye, &detail, ignore_view) with picks and eye set. No target
/// 3D, or a dead target seen by an observer in a combat package (vt+0x334(1)), answers false. The process's +0x10 adds the
/// frame time. The pick budget is 00B14904 - 5, or 00B148FC below 20 frames a second (`ftol(1 / frame time)`); over it, the
/// player or an observer in combat answers true with detail 1 while 00B3B914 exceeds 00B148E4, anyone else false. Detail
/// 3, then the heading to the target (00683CB0 of target - observer) against the observer's Z rotation (00683D80): 1.658
/// radians or more answers false. Then with 00B3B77C clear, the picks from the eye through the cell's Havok world (a stop
/// here); with it set, true with detail 2. Completion: an observer that faces away from the target.
fn test_line_of_sight(
    world: &mut World,
    observer: Entity,
    target: Entity,
) -> Result<(bool, u32), GameError> {
    if crate::tes::forms_init_world::reference_ni_node(world, target).is_none() {
        return Ok((false, 3));
    }
    if crate::tes::actor::actor_is_dead(world, target, false)
        && crate::tes::actor_package::actor_in_combat_package(world, observer, true)?
    {
        return Ok((false, 3));
    }
    let frame = crate::tes::actors::actor_movement_update::frame_time(world);
    let process = process_of(world, observer)
        .ok_or(GameError::stop(0x005F28C7, "line of sight with no process"))?;
    if let Some(mut low) = world.get_mut::<crate::tes::actor_process::LowProcess>(process) {
        low.field_10 += frame;
    }
    let budget = if oblivion_core::x87::ftol(1.0 / frame) < 20 {
        21
    } else {
        40 - 5
    };
    if world
        .get_resource_or_init::<DetectionGlobals>()
        .picks_this_frame_b3bd7c
        > budget
    {
        return Err(GameError::stop(
            0x005F2903,
            "a line of sight over the frame's pick budget",
        ));
    }
    let target_position = world
        .get::<TesObjectRefr>(target)
        .expect("a reference")
        .position_2c;
    let observer_reference = world.get::<TesObjectRefr>(observer).expect("an actor");
    let position = observer_reference.position_2c;
    let delta = [
        target_position[0] - position[0],
        target_position[1] - position[1],
        target_position[2] - position[2],
    ];
    let difference = heading_difference(observer_reference.rotation_20[2], heading_angle(delta));
    if f64::from(difference.abs()) >= 1.658_062_815_666_198_7 {
        return Ok((false, 3));
    }
    line_of_sight_picks(world, observer, target, position, target_position)
}

// IDA: 005E40C0 Actor_GetEyeHeight for a non-player (vt+0x27C clear): the bound's top z less its bottom z (vt+0x15C, vt+0x158,
// a double) times 0.75, a float.
fn eye_height(world: &mut World, actor: Entity) -> Result<f32, GameError> {
    if the_player(world) == Some(actor) {
        return Err(GameError::stop(0x005E40D4, "the player's eye height"));
    }
    let top = crate::tes::actor::actor_bound_corner(world, actor, true)?;
    let bottom = crate::tes::actor::actor_bound_corner(world, actor, false)?;
    Ok(((f64::from(top[2]) - f64::from(bottom[2])) * 0.75) as f32)
}

/// IDA: 005F2A9F..005F2E3E, 005F2820's picks with 00B3B77C clear. No parent cell answers false; an exterior's cell is the
/// worldspace's at the position (a stop here); an unattached cell answers false. The pick (004F5EA0) starts at the eye
/// (005E40C0 above the position) and ends at the target's x and y; 00535A00's SpecificItemCollector makes layer 26's
/// collision mask (00BA7DB0[26]) 0xA277F plus the target's controller layer (0065ABE0), and the pick's filter is the
/// observer's controller phantom filter's group with layer 26 (0x1A). The target's bound height (vt+0x15C less vt+0x158)
/// places the ends at 0.5, 0.75 and 0.25 of it: three picks for a player observer or target (or a combat target, a
/// stop), else one; each adds to 00B3BD7C. The first pick that hits nothing, or whose object's reference
/// (getObjectPtrFromNiNode) is the target, answers true with detail 1, 2 or 0 in that order. **Host substitution:** each
/// bhkWorld pick (00446A10 TES_PickObject, the world's vt+0x88) is the seam's ray in Havok units with the observer's system group left
/// out, its nearest hit whose layer is in layer 26's mask taken as the pick's; a hit on the target's own layer is a stop
/// (its owner lookup). Completion: an interior pick whose hits are other layers.
fn line_of_sight_picks(
    world: &mut World,
    observer: Entity,
    target: Entity,
    position: [f32; 3],
    target_position: [f32; 3],
) -> Result<(bool, u32), GameError> {
    let Some(cell) = crate::tes::forms_references::parent_cell(world, observer) else {
        return Ok((false, 3));
    };
    if !crate::tes::forms_world::cell_is_interior(world, cell) {
        return Err(GameError::stop(
            0x005F2B0A,
            "an exterior line of sight's cell lookup (0044A270)",
        ));
    }
    let Some(physics) = crate::tes::cell_havok::cell_bhk_world(world, cell)
        .and_then(|bhk| world.get::<crate::bs::havok_world::BhkWorld>(bhk))
        .and_then(|bhk| bhk.physics)
    else {
        return Ok((false, 3));
    };
    let eye = [
        position[0],
        position[1],
        eye_height(world, observer)? + position[2],
    ];
    let filter = |world: &World, actor: Entity| {
        crate::tes::character_controller::actor_character_controller(world, actor).map_or(
            0,
            |controller| {
                crate::bs::character_controller::controller_phantom_filter(world, controller)
            },
        )
    };
    let mask = 0x000A_277F_u32 | (1 << (filter(world, target) & 0x3F));
    let target_layer = filter(world, target) & 0x3F;
    let group = filter(world, observer) >> 16;
    let top = crate::tes::actor::actor_bound_corner(world, target, true)?;
    let bottom = crate::tes::actor::actor_bound_corner(world, target, false)?;
    let height = (f64::from(top[2]) - f64::from(bottom[2])) as f32;
    let player = the_player(world);
    if Some(target) != player && Some(observer) != player {
        return Err(GameError::stop(
            0x005F2C80,
            "a non-player line of sight's combat-target pick count",
        ));
    }
    let scale = |value: f32| (f64::from(value) * 0.142_876_729_369_163_5) as f32;
    let start = eye.map(scale);
    for (fraction, detail) in [(0.5_f32, 1_u32), (0.75, 2), (0.25, 0)] {
        let end = [
            target_position[0],
            target_position[1],
            fraction * height + target_position[2],
        ]
        .map(scale);
        world
            .get_resource_or_init::<DetectionGlobals>()
            .picks_this_frame_b3bd7c += 1;
        let delta = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
        let length = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
        let direction = delta.map(|value| value / length);
        let hits = crate::bs::physics_seam::seam_cast_ray(
            world,
            Some(physics),
            start,
            direction,
            length,
            group,
            16,
        );
        let mut blocked = false;
        for (_, body) in hits {
            let mut bodies = world.query::<(Entity, &crate::bs::havok_world::BhkInWorld)>();
            let entity = bodies
                .iter(world)
                .find(|(_, held)| held.physics == Some(body))
                .map(|(entity, _)| entity);
            let layer = entity.map_or(0, |entity| {
                crate::bs::havok_world::body_filter_info(world, entity) & 0x3F
            });
            if mask & (1 << layer) == 0 {
                continue;
            }
            if layer == target_layer {
                return Err(GameError::stop(
                    0x005F2DB0,
                    "a line-of-sight hit on the target's layer",
                ));
            }
            blocked = true;
            break;
        }
        if !blocked {
            return Ok((true, detail));
        }
    }
    Ok((false, 3))
}

// IDA: 005F3B50 Actor_GetBootWeight(actor): 00B333B8 (`g_havokStepFlag`, never set in the port) answers 5; a sneaking
// actor (005E0550) of Sneak mastery 2 or more answers 0; a creature's base +0x110 word; an NPC's worn slot-5 (feet) form's
// weight (00486790's selection, its +0x58), else 0. Completion: an NPC or the player.
fn boot_weight(world: &World, actor: Entity) -> Result<i32, GameError> {
    if sneaking(world, actor)
        && crate::tes::player::actor_skill_mastery_level(world, actor, 31)? >= 2
    {
        return Ok(0);
    }
    let base = world
        .get::<TesObjectRefr>(actor)
        .and_then(|reference| reference.base_form_1c);
    if base.is_some_and(|base| crate::tes::forms_init::form_type(world, base) == 0x24) {
        return Err(GameError::stop(0x005F3BA0, "a creature's foot weight"));
    }
    // 005F3C04: the selection's form's +0x58, the TESWeightForm (+0x54) of an ARMO or a CLOT, `ftol`.
    Ok(crate::tes::equipment::equipped_in_slot(world, actor, 5)?
        .and_then(|(form, _)| world.get::<crate::tes::form_components::TesWeightForm>(form))
        .map_or(0, |weight| oblivion_core::x87::ftol(weight.weight_04)))
}

// IDA: 005E0550 Actor_IsSneaking: a process whose movement flags (vt+0x2C0, a High process's +0x1FC) hold 0x400 without 0x800.
fn sneaking(world: &World, actor: Entity) -> bool {
    process_of(world, actor)
        .and_then(|process| world.get::<crate::tes::actor_process::HighProcess>(process))
        .is_some_and(|high| high.movement_flags_1fc & 0xC00 == 0x400)
}

/// IDA: 005463F0 Detection_CalculateLevel(observer skill, target skill, seen, distance, blindness, light, chameleon, boot
/// weight, moving, sneaking, attacking, in combat, running, swimming, sleeping, exterior), as read from the x87 code (the
/// stores are floats, the stack doubles). fSneakMaxDistance (times fSneakExteriorDistanceMult outside); beyond it and not
/// attacking answers 0. The sound term: (fSneakBootWeightMult * weight + fSneakBootWeightBase while moving), times
/// fSneakRunningMult when running, plus fSneakTargetInCombatBonus, times the distance factor, fSneakSoundLosMult unless seen
/// and fSneakSoundsMult, at least 0. The light term: (light + fDetectionSneakLightMod) times seen, the factor,
/// (100 - blindness) over 100, (100 - chameleon) over 100 and fSneakLightMult, at least 0. The skill term: the factor
/// times the observer's skill, less the target's while sneaking, times fSneakSkillMult, both skills capped at 100. fSneakTargetAttackBonus when attacking; swimming zeroes the sound and
/// takes fSneakSwimmingLightMult; sleeping makes the light fSneakSleepBonus. The total from fSneakBaseValue: 1.0 or more,
/// zero, negative or NaN answers `ftol`; otherwise 1. Completion: 100%.
#[allow(
    clippy::too_many_arguments,
    reason = "the original's 16 cdecl arguments, in order"
)]
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "the x87 compare: a NaN takes the negated arm"
)]
pub fn detection_formula(
    world: &World,
    observer_skill: i32,
    target_skill: i32,
    seen: bool,
    distance: f32,
    blindness: i32,
    light: i32,
    chameleon: i32,
    boot_weight: i32,
    moving: bool,
    sneaking: bool,
    attacking: bool,
    in_combat: bool,
    running: bool,
    swimming: bool,
    sleeping: bool,
    exterior: bool,
) -> i32 {
    let setting = |address: u32| {
        f32::from_bits(
            world
                .resource::<crate::tes::game_settings::GameSettingCollection>()
                .raw_at(address)
                .unwrap_or(0),
        )
    };
    let f = f64::from;
    let mut max_distance = setting(0x00B3_6708);
    if exterior {
        max_distance = (f(setting(0x00B3_6748)) * f(max_distance)) as f32;
    }
    if f(distance) > f(max_distance) && !attacking {
        return 0;
    }
    let factor = ((f(max_distance) - f(distance)) / f(max_distance)) as f32;
    let mut sound_base = 0.0_f32;
    if moving {
        sound_base =
            (f64::from(boot_weight) * f(setting(0x00B3_66F8)) + f(setting(0x00B3_66F0))) as f32;
    }
    let combat_bonus = if in_combat { setting(0x00B3_66E8) } else { 0.0 };
    let running_mult = if running { setting(0x00B3_6720) } else { 1.0 };
    let los_mult = if seen { 1.0 } else { setting(0x00B3_6718) };
    let mut seen_factor = if seen { 1.0_f32 } else { 0.0 };
    let mut sound = ((f(running_mult) * f(sound_base) + f(combat_bonus))
        * f(factor)
        * f(los_mult)
        * f(setting(0x00B3_6740))) as f32;
    if 0.0 > sound {
        sound = 0.0;
    }
    let lit = (f64::from(light) + f(setting(0x00B3_6CB8))) * (f(seen_factor) * f(factor));
    let lit = (100.0 - f64::from(blindness)) * lit / 100.0;
    let mut light_term =
        ((100.0 - f64::from(chameleon)) / 100.0 * lit * f(setting(0x00B3_6738))) as f32;
    if 0.0 > light_term {
        light_term = 0.0;
    }
    seen_factor = if sneaking { 1.0 } else { 0.0 };
    let observer_skill = observer_skill.min(100);
    let target_skill = target_skill.min(100);
    let skill = ((f(factor) * f64::from(observer_skill) - f64::from(target_skill) * f(seen_factor))
        * f(setting(0x00B3_6710))) as f32;
    let attack = if attacking { setting(0x00B3_66E0) } else { 0.0 };
    if swimming {
        sound = 0.0;
        light_term = (f(setting(0x00B3_6730)) * f(light_term)) as f32;
    }
    if sleeping {
        light_term = setting(0x00B3_6728);
    }
    let total = (f(setting(0x00B3_6700)) + f(sound) + f(light_term) + f(attack) + f(skill)) as f32;
    if total >= 1.0 || !(total > 0.0) {
        return oblivion_core::x87::ftol(total);
    }
    1
}

/// IDA: 005F6540 Actor_GetDetectionLevel(observer, light_arg, recompute, target, &seen, combat, a7, a8) with recompute and
/// a7 clear, as 00603CA0 calls it for the player. No process answers 0, seen 0. The observer's detection state for the
/// target (vt+0x3B0) answers its level (+0x0C) and seen (+0x08) unless missing; a target with no 3D answers -1. Otherwise:
/// the distance (004D7E90), 00B333C0 + 1, the target's light (its process vt+0x3AC, 00655FE0, `ftol`), scaled by
/// fDetectionNightEyeBonus with the observer's Night-Eye (AV 41) and capped at 100; 005F2820's line of sight; then the
/// noise, sneak and visibility inputs and 005463F0 (stops here). Completion: an uncached detection whose observer faces
/// away from the target, through the line of sight.
/// # Errors
/// The stops above.
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "the x87 compare: a NaN takes the negated arm"
)]
pub fn actor_detection_level(
    world: &mut World,
    observer: Entity,
    target: Entity,
    combat: bool,
) -> Result<(i32, bool), GameError> {
    use crate::tes::actor_process::{DetectionList, DetectionState};
    let Some(process) = process_of(world, observer) else {
        return Ok((0, false));
    };
    let cached = world.get::<DetectionList>(process).and_then(|list| {
        list.0
            .iter()
            .find(|state| state.target_00 == target)
            .copied()
    });
    if let Some(state) = cached {
        let level = state.level_0c;
        if crate::tes::forms_init_world::reference_ni_node(world, target).is_none() {
            return Ok((-1, state.seen_08 != 0));
        }
        if level != i32::MAX {
            return Ok((level, state.seen_08 != 0));
        }
    }
    if crate::tes::forms_init_world::reference_ni_node(world, target).is_none() {
        return Ok((-1, false));
    }
    let distance =
        crate::tes::forms_references::reference_distance(world, observer, target, false)?;
    world
        .get_resource_or_init::<DetectionGlobals>()
        .uncached_detections_b333c0 += 1;
    let target_process = process_of(world, target).ok_or(GameError::stop(
        0x005F65E8,
        "a detected target with no process",
    ))?;
    if world
        .get::<crate::tes::actor_process::MiddleHighProcess>(target_process)
        .is_none()
    {
        return Err(GameError::stop(
            0x005F65E8,
            "the light of a target without a MiddleHigh process",
        ));
    }
    let light = crate::tes::actor_process::actor_light_level(world, target_process, target, false)?;
    let mut light = oblivion_core::x87::ftol(light);
    if crate::tes::ai_process::actor_get_actor_value_float(world, observer, 41)? > 0.0 {
        let bonus = world
            .resource::<crate::tes::game_settings::GameSettingCollection>()
            .find("fDetectionNightEyeBonus")
            .map(|setting| f32::from_bits(setting.raw))
            .ok_or(GameError::stop(
                0x005F6615,
                "a missing fDetectionNightEyeBonus",
            ))?;
        light = oblivion_core::x87::ftol((f64::from(light) * f64::from(bonus)) as f32).min(100);
    }
    let (seen, _detail) = test_line_of_sight(world, observer, target)?;
    // 005F664C: the observer's combat package's target (vt+0x330, 006135F0) is the line of sight's, a stop here.
    if crate::tes::actor_package::actor_in_combat_package(world, observer, false)? {
        return Err(GameError::stop(
            0x005F665E,
            "an observer's combat target line of sight (00612810)",
        ));
    }
    let mut weight = boot_weight(world, target)?;
    let target_sneaking = sneaking(world, target);
    let mastery = |world: &World| crate::tes::player::actor_skill_mastery_level(world, target, 31);
    if target_sneaking && mastery(world)? >= 2 {
        weight = 0;
    }
    let flags = world
        .get::<crate::tes::actor_process::HighProcess>(target_process)
        .map_or(0, |high| high.movement_flags_1fc);
    let (mut moving, mut running) = (flags & 0xF != 0, flags & 0x200 != 0);
    if target_sneaking && mastery(world)? >= 3 {
        moving = false;
        running = false;
    }
    let observer_cell = crate::tes::forms_references::parent_cell(world, observer);
    let observer_position = world
        .get::<TesObjectRefr>(observer)
        .expect("an actor")
        .position_2c;
    let swimming = {
        let Some(cell) = observer_cell else {
            return Err(GameError::stop(0x005F6760, "an observer with no cell"));
        };
        let height = crate::tes::actor::actor_height(world, observer)? * 0.875;
        let level = f64::from(observer_position[2]) + f64::from(height);
        !(f64::from(crate::tes::tes_grid::cell_water_height(world, cell)) <= level)
    };
    let sleeping = world
        .get::<crate::tes::actor_process::MiddleHighProcess>(process)
        .is_some_and(|middle| middle.sleep_state_11d == 9);
    let exterior = crate::tes::forms_references::parent_cell(world, target)
        .is_none_or(|cell| !crate::tes::forms_world::cell_is_interior(world, cell));
    let skill_of = |world: &mut World, actor: Entity| -> Result<i32, GameError> {
        let luck = oblivion_core::x87::ftol(crate::tes::ai_process::actor_get_actor_value_float(
            world, actor, 7,
        )?);
        let sneak = oblivion_core::x87::ftol(crate::tes::ai_process::actor_get_actor_value_float(
            world, actor, 31,
        )?);
        Ok(oblivion_core::x87::ftol(
            crate::tes::actor_values::luck_adjusted_skill(world, sneak, luck),
        ))
    };
    let observer_skill = skill_of(world, observer)?;
    let target_skill = skill_of(world, target)?;
    let mut level = if crate::tes::actor::actor_get_actor_value(world, target, 47)? > 0 {
        -100
    } else {
        let chameleon = crate::tes::actor::actor_get_actor_value(world, target, 46)?;
        if chameleon >= 100 {
            -100
        } else {
            let blindness = crate::tes::actor::actor_get_actor_value(world, observer, 45)?;
            detection_formula(
                world,
                observer_skill,
                target_skill,
                seen,
                distance,
                blindness,
                light,
                chameleon,
                weight,
                moving,
                target_sneaking,
                false,
                combat,
                running,
                swimming,
                sleeping,
                exterior,
            )
        }
    };
    if f64::from(distance) <= 900.0 {
        let bonus = world
            .get::<crate::tes::actor_process::HighProcess>(target_process)
            .map_or(0.0, |high| high.float_2ac);
        level = oblivion_core::x87::ftol((f64::from(bonus) + f64::from(level)) as f32);
    }
    // 005F6926: recompute clear: the state (3 for a positive level) through vt+0xA8 (00631BE0) unless the observer is the
    // player.
    if the_player(world) != Some(observer) {
        let state = if level > 0 { 3 } else { 0 };
        let mut list = world
            .get::<DetectionList>(process)
            .cloned()
            .unwrap_or_default();
        let entry = DetectionState {
            target_00: target,
            state_04: state,
            seen_08: u8::from(seen),
            level_0c: level,
        };
        match list.0.iter_mut().find(|held| held.target_00 == target) {
            Some(held) => *held = entry,
            None => list.0.insert(0, entry),
        }
        world.entity_mut(process).insert(list);
    }
    if world
        .resource::<crate::tes::actor_process::ActorProcessGlobals>()
        .manager
        .byte_9a
        != 0
    {
        return Err(GameError::stop(
            0x005F6952,
            "the process manager's detection display",
        ));
    }
    Ok((level, seen))
}
