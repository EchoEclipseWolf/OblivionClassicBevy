//! `0065AF30` MobileObject_Move and the controller's `vt+0x80` integrate (`00896000`), as `0066EAF0`'s placement loop
//! reaches them for the player (Build 3 goal 2, run-ak's STEP lines).
//!
//! **Host substitution:** the Havok side of `00896000` (the hkCharacterProxy's integrate `008AE890`, the hkpCharacterContext
//! state update, the listener's contact callback `00893B90`) is the physics seam's `controller_move`; the Bethesda-side flag
//! and timer stores around it are ported, with the listener's contact bits and the surface's supported state taken from the
//! seam's answer.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::actor::TesObjectRefr;
use super::character_controller::{actor_character_controller, controller_move};
use crate::bs::character_controller::{
    BhkCharacterController, controller_game_position, controller_update_capsule_growth,
};
use crate::tes::forms_references::{
    reference_is_creature_type_4, set_reference_position, set_reference_rotation,
};
use crate::tes::save_game::{ProcessView, controller};

/// IDA: 005E1800 Actor_GetJumpScale (vt+0x1F4): vt+0x284 GetActorValue(0x1A, Acrobatics) `fild`, divided by 100.0
/// (dbl_A309F0), stored as a float. Completion: 100% for the Actor, Character, Creature and player value dispatch.
/// # Errors
/// A stop in the actor value.
pub fn actor_get_jump_scale(world: &mut World, actor: Entity) -> Result<f32, GameError> {
    let value = crate::tes::actor::actor_get_actor_value(world, actor, 0x1A)?;
    Ok((f64::from(value) / 100.0) as f32)
}

/// IDA: N/A what the move hands the controller's vt+0x80: 0065AF30's stack record at var_6C (time, the reference's X, Y
/// and Z rotations, the delta, a4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControllerMoveInput {
    pub time_00: f32,
    pub rotation_04: [f32; 3],
    pub delta_10: [f32; 3],
    pub flag_1c: u32,
}

/// IDA: 00896000 bhkCharacterController_Integrate (vt+0x80)(input), the arm a still controller takes (a delta within 0.01
/// of zero, no 0x80000 or 0x100000 flag, bit 0 clear). In order: +0x1F4 &= 0xFFFFFF1F; a zero time or a null hk proxy
/// (vt+0x58; the seam's character here) answers 0. The delta's 0.01 test (008904E0); v97 = still and +0x2F0's three
/// components within 0.001 of zero (00BA7A40), which clears bit 3, else sets it. With v97 the substep count is 1.
/// `00894C70` with +0x3AC != 2 returns. +0x214 = 31, +0x218 = 0. The one substep: +0x1F4 &= 0xFFFFF9FF, |= 0x10000;
/// `00891780` (bit 0 only); the proxy's integrate (`008AE890`, the seam's move: **host substitution**); +0x1F4 &= ~0x10000;
/// with bit 0x2000 clear, 0x100 = the listener's contact (0x200) or a supported surface (+0x260 == 2 and +0x250 == 0, the
/// seam's `supported`); the state's update (`008BA170`'s vt+0x18, substituted); +0x2A0 = 11. `008939B0` (bit 0 only). The
/// answer is 2 when any of 0xE0 is set. The debug colour (`008AEBB0`, `0088BB60`) and vt+0x84 (`00891230`, the avoid box's
/// hk transform) are Havok's. Then +0x300 and +0x30C count down by the time (+0x300 reaching 0 clears +0x2F0), and
/// +0x1F4 &= ~0x1000. The listener's contact callback (`00893B90`) sets 0x400 for a contact whose normal z is at least
/// the listener's +0x20 (cos of the slope) and 0x200 for every counted contact: here a supported answer gives both.
/// Completion: 100% of the Bethesda-side stores of this arm; the Havok side is the seam.
/// TODO: [ ] +0x290..+0x2DC (the hk velocity, the up quaternion, the time and its inverse): Havok-side vectors the port doesn't hold.
/// TODO: [ ] The state machine's state (+0x1EC) and its states' stores (`008CF6E0`, `008CFC70`), the contact list +0x3BC.
/// # Errors
/// An unported arm, or no physics backend.
pub fn controller_integrate(
    world: &mut World,
    controller: Entity,
    input: &ControllerMoveInput,
) -> Result<u32, GameError> {
    let (physics, flags) = {
        let mut c = world
            .get_mut::<BhkCharacterController>(controller)
            .expect("a controller");
        c.flags_1f4 &= 0xFFFF_FF1F;
        (c.host_physics, c.flags_1f4)
    };
    if input.time_00 == 0.0 || physics.is_none() {
        return Ok(0);
    }
    let still = input.delta_10.iter().all(|&value| value.abs() <= 0.01);
    if !still {
        return Err(GameError::unclassified(
            0x0089606F,
            "0089606F: a controller move with a velocity",
        ));
    }
    if flags & 0x0018_0000 != 0 {
        return Err(GameError::unclassified(
            0x00896079,
            "00896079: a controller with 0x80000 or 0x100000",
        ));
    }
    if flags & 1 != 0 {
        return Err(GameError::unclassified(
            0x008961F4,
            "008961F4: a controller with bit 0",
        ));
    }
    let resting = {
        let c = world
            .get::<BhkCharacterController>(controller)
            .expect("a controller");
        c.zero_2f0[..3]
            .iter()
            .all(|&bits| f32::from_bits(bits).abs() <= 0.001)
    };
    let v97 = still && resting;
    {
        let mut c = world
            .get_mut::<BhkCharacterController>(controller)
            .expect("a controller");
        if v97 {
            c.flags_1f4 &= !8;
        } else {
            c.flags_1f4 |= 8;
        }
        if !v97 {
            return Err(GameError::unclassified(
                0x008961B0,
                "008961B0: a moving controller's substeps",
            ));
        }
    }
    controller_update_capsule_growth(world, controller, input.time_00)?;
    {
        let mut c = world
            .get_mut::<BhkCharacterController>(controller)
            .expect("a controller");
        c.listener_field_214 = 31;
        c.listener_field_218 = 0;
        c.flags_1f4 &= 0xFFFF_F9FF;
        c.flags_1f4 |= 0x1_0000;
    }
    // 008968CE: the proxy's integrate through the seam (gravity applies: the controller isn't flying, +0x1EC isn't 5).
    let moved = controller_move(
        world,
        controller,
        &crate::bs::physics_seam::CharacterMove {
            time: input.time_00,
            velocity: [0.0; 3],
            gravity: true,
        },
    )?;
    let mut c = world
        .get_mut::<BhkCharacterController>(controller)
        .expect("a controller");
    c.flags_1f4 &= !0x1_0000;
    if moved.supported {
        // 00893B90's contact: 0x400 for a surface at least the listener's slope, and 0x200.
        c.flags_1f4 |= 0x400;
        c.flags_1f4 |= 0x200;
    }
    if c.flags_1f4 & 0x2000 != 0 {
        return Err(GameError::unclassified(
            0x008968FE,
            "008968FE: the controller's 0x2000 arm",
        ));
    }
    if c.flags_1f4 & 0x200 != 0 || moved.supported {
        c.flags_1f4 |= 0x100;
    } else {
        c.flags_1f4 &= !0x100;
    }
    c.field_2a0 = 11;
    let answer = if c.flags_1f4 & 0xE0 != 0 { 2 } else { 0 };
    let time = input.time_00;
    if c.floats_300[0] > 0.0 {
        let left = x87::stored_float(f64::from(c.floats_300[0]) - f64::from(time));
        c.floats_300[0] = left;
        if left <= 0.0 {
            c.floats_300[0] = 0.0;
            c.zero_2f0 = [0; 4];
        }
    }
    if c.floats_300[3] > 0.0 {
        c.floats_300[3] = x87::stored_float(f64::from(c.floats_300[3]) - f64::from(time));
    }
    c.flags_1f4 &= !0x1000;
    Ok(answer)
}

/// IDA: 0065AF30 MobileObject_Move(time, delta, a4). Actor_GetCharController; vt+0xE8 (the player's ReturnFalse); the X,
/// Y, Z rotation each FLT_MAX, NaN or not finite gives the warning "MobileObject::Move called on '%s' (%08X) with invalid
/// angle." and SetRotation(0, 0, 0). iHavokSkipFrameCountTEST (00B14E44) nonzero is the frame-skip arm, a stop. No
/// controller or vt+0x198(1) (Actor_IsDead) is the plain arm: the delta turned by 0070FDD0(Z) and added to the position.
/// Otherwise the controller arm: with vt+0xE8 false, +0x310 = vt+0x1F4 (`actor_get_jump_scale`), then `004D74D0` (true is
/// the sneak/swim arm, a stop); the record for vt+0x80; +0x1F4 bit 0x800 (keyframed) and bit 0 are stops; vt+0x80
/// (`controller_integrate`); bit 1 (the lift) is a stop; with 0x800 clear the position is 005E1500's; then
/// TESObjectREFR_SetPosition (004D8A30). Completion: 100% of the arms the player's placement takes (run-ak's CALL lines).
/// # Errors
/// The stops named.
pub fn mobile_object_move(
    world: &mut World,
    actor: Entity,
    time: f32,
    delta: [f32; 3],
    flag: u32,
) -> Result<(), GameError> {
    let controller = actor_character_controller(world, actor);
    // Actor, Character, Creature and PlayerCharacter all dispatch +E8 to 0069D990.
    if world.get::<super::actor::Actor>(actor).is_none() {
        return Err(GameError::unclassified(
            0x0065AF52,
            "0065AF52: vt+0xE8 on a non-actor mobile object",
        ));
    }
    let rotation = world
        .get::<TesObjectRefr>(actor)
        .expect("a reference")
        .rotation_20;
    if rotation
        .iter()
        .any(|&angle| angle == f32::MAX || angle.is_nan() || !angle.is_finite())
    {
        oblivion_core::logging::emit(
            oblivion_core::logging::Level::Warning,
            "MobileObject::Move called with invalid angle.",
        );
        set_reference_rotation(world, actor, [0.0; 3])?;
    }
    let frame_skip = world
        .get_resource::<crate::tes::settings::SettingCollections>()
        .and_then(|settings| settings.ini.find_address(0x00B1_4E44))
        .map_or(0, |setting| setting.raw);
    if frame_skip != 0 {
        return Err(GameError::unclassified(
            0x0065B02A,
            "0065B02A: iHavokSkipFrameCountTEST's frame-skip arm",
        ));
    }
    let rotation = world
        .get::<TesObjectRefr>(actor)
        .expect("a reference")
        .rotation_20;
    let Some(controller) =
        controller.filter(|_| !crate::tes::actor::actor_is_dead(world, actor, true))
    else {
        let offset = crate::gamebryo::transform::rotate_vector(
            &crate::gamebryo::transform::rotation_z(rotation[2]),
            &delta,
        );
        let position = world
            .get::<TesObjectRefr>(actor)
            .expect("a reference")
            .position_2c;
        return set_reference_position(
            world,
            actor,
            std::array::from_fn(|i| position[i] + offset[i]),
        );
    };
    let jump = actor_get_jump_scale(world, actor)?;
    world
        .get_mut::<BhkCharacterController>(controller)
        .expect("a controller")
        .floats_300[4] = jump;
    if reference_is_creature_type_4(world, actor)? {
        return Err(GameError::unclassified(
            0x0065B231,
            "005E13A0's sneak or swim arm",
        ));
    }
    let input = ControllerMoveInput {
        time_00: time,
        rotation_04: rotation,
        delta_10: delta,
        flag_1c: flag,
    };
    let flags = world
        .get::<BhkCharacterController>(controller)
        .expect("a controller")
        .flags_1f4;
    if flags & 0x800 != 0 {
        return Err(GameError::unclassified(
            0x0065B2A0,
            "0065B2A0: the keyframed controller's arm",
        ));
    }
    if flags & 1 != 0 {
        return Err(GameError::unclassified(
            0x0065B420,
            "0065B420: the controller's bit 0 (+0x330)",
        ));
    }
    controller_integrate(world, controller, &input)?;
    let flags = world
        .get::<BhkCharacterController>(controller)
        .expect("a controller")
        .flags_1f4;
    if flags & 2 != 0 {
        return Err(GameError::unclassified(
            0x0065B459,
            "0065B459: the controller's lift (008913C0)",
        ));
    }
    let position = if flags & 0x800 == 0 {
        let position = controller_game_position(world, controller)?;
        if flags & 1 != 0 {
            return Err(GameError::unclassified(
                0x0065B4DA,
                "0065B4DA: the controller's bit 0 after the move",
            ));
        }
        position
    } else {
        return Err(GameError::unclassified(
            0x0065B4C1,
            "0065B4C1: a keyframed controller's position",
        ));
    };
    set_reference_position(world, actor, position)
}

// IDA: 00659BE0 and its callees' constants at version 125: 00632370, 00650F50, 00658BC0, 00647060, 0060D0E0 and 00651AD0.
pub(crate) fn process_save_size(world: &World, process: &ProcessView<'_>) -> u32 {
    let package = if process.base.editor_package_08.is_some() {
        4
    } else {
        0
    };
    let base_process = 4 + package + 8;
    let low = base_process + 11 + 1 + 1 + 4 + 8 + 4 + 8 + 4 + 1;
    let middle_low = low + 4;
    let middle_high = middle_low + 2 + 5 + 4 + 51 + 2 + 8 + 1 + 2 + 10 + 1 + 4;
    let controller_size = controller(world, process).map_or(0, |_| 12 + 44 + 12 + 8);
    middle_high + 54 + 4 + 45 + 5 + 5 + 14 + controller_size + 2 + 30 + 8 + 4 + 24
}
