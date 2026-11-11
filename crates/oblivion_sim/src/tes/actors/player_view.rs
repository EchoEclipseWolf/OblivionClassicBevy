//! The player's view switch: `0066C580` PlayerCharacter_SetFirstPerson and `00664F70` PlayerCharacter_SwapViewNodes, as
//! RaceSexMenu_Create's `005CC17F` reaches them with 0 (the third person). Evidence: run-ag (tick 32) and run-af's
//! after_move: +0x588 = 1, the third person's node +0x18 = 6, `Player1stPerson`'s 47.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::player::{PlayerCharacter, PlayerGlobals};
use crate::gamebryo::scene_node::SceneNodePayload;
use crate::tes::reference_3d::Reference3D;

/// IDA: N/A the calls the view switch reaches that the port counts: `0066B710` (the player's camera update, Build 3 goal 2's,
/// with D16), and `00664F70`'s tail `006637C0` PlayerCharacter_UpdateViewLight, `005EA1A0` Actor_UpdateSkinProperty and the
/// process's vt+0x42C(player, 1, 1, 0).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct PlayerViewBoundaries {
    pub calls: std::collections::BTreeMap<u32, u32>,
}

// IDA: N/A a helper, with no address of its own.
fn count(world: &mut World, address: u32) {
    *world
        .get_resource_or_init::<PlayerViewBoundaries>()
        .calls
        .entry(address)
        .or_default() += 1;
}

// IDA: N/A a node's +0x18 flags.
fn flags(world: &World, node: Entity) -> u16 {
    world
        .get::<SceneNodePayload>(node)
        .map_or(0, |payload| payload.effective_av_flags)
}

// IDA: N/A a store to a node's +0x18 flags.
fn set_flags(world: &mut World, node: Entity, value: u16) {
    if let Some(mut payload) = world.get_mut::<SceneNodePayload>(node) {
        payload.effective_av_flags = value;
    }
}

/// IDA: 00664F70 PlayerCharacter_SwapViewNodes(first_person), the arm with 0: needs +0x5D0 and the third person's node
/// (Character_GetNiNode, +0x3C); a third-person node without bit 0 (already shown) returns. Its bit 0 cleared, +0x5D0's set.
/// 005E5480's magic-node arm needs a cast in progress (none here). Then 006637C0, 005EA1A0(third person's node) and the
/// process's vt+0x42C(player, 1, 1, 0), counted. The 1 arm (the first person: the node's position, rotation, 005EE1B0,
/// 00474510) is a stop. Completion: 100% of the state of the 0 arm.
/// # Errors
/// The first-person arm.
pub fn player_swap_view_nodes(
    world: &mut World,
    player: Entity,
    first_person: bool,
) -> Result<(), GameError> {
    let Some(first) = world
        .get::<PlayerCharacter>(player)
        .and_then(|data| data.first_person_node_5d0)
    else {
        return Ok(());
    };
    let Some(third) = world.get::<Reference3D>(player).map(|loaded| loaded.0) else {
        return Ok(());
    };
    if first_person {
        return Err(GameError::unclassified(
            0x00664FB0,
            "00664FB0: the switch to the first person",
        ));
    }
    if flags(world, third) & 1 == 0 {
        return Ok(());
    }
    let value = flags(world, third) & !1;
    set_flags(world, third, value);
    let value = flags(world, first) | 1;
    set_flags(world, first, value);
    count(world, 0x0066_37C0);
    count(world, 0x005E_A1A0);
    count(world, 0x0066_51FD);
    Ok(())
}

/// IDA: 0066C580 PlayerCharacter_SetFirstPerson(first_person): answers whether +0x588 equalled it; +0x588 = !first_person; for
/// the third person a dead player (vt+0x198) takes 00B36B80's wheel distance, a stop; with 00B3BB04 clear and +0x5D0,
/// 00B14E4D = 1, 0066B710(player, 0) (counted: the camera update, D16) and 00664F70(!+0x588). Completion: 100% of the third
/// person's arm.
/// # Errors
/// A dead player, or 00664F70's.
pub fn player_set_first_person(
    world: &mut World,
    player: Entity,
    first_person: bool,
    vanity_b3bb04: u8,
) -> Result<bool, GameError> {
    let third = {
        let mut data = world
            .get_mut::<PlayerCharacter>(player)
            .expect("the player");
        let was = data.view_flags_588[0] == u8::from(first_person);
        data.view_flags_588[0] = u8::from(!first_person);
        was
    };
    if !first_person && crate::tes::actor::actor_is_dead(world, player, false) {
        return Err(GameError::unclassified(
            0x0066C5AE,
            "0066C5AE: a dead player's vanity wheel",
        ));
    }
    let has_first = world
        .get::<PlayerCharacter>(player)
        .is_some_and(|data| data.first_person_node_5d0.is_some());
    if vanity_b3bb04 == 0 && has_first {
        world.get_resource_or_init::<PlayerGlobals>().byte_b14e4d = 1;
        count(world, 0x0066_B710);
        let now_first = world
            .get::<PlayerCharacter>(player)
            .expect("the player")
            .view_flags_588[0]
            == 0;
        player_swap_view_nodes(world, player, now_first)?;
    }
    Ok(third)
}

/// IDA: 0066A5E0 PlayerCharacter_SyncCameraCollisionWorld: with a camera collision helper (+0x1F0) and a parent cell
/// (+0x40), the cell's world (an interior's ExtraHavok world, else 00B35C24 g_exteriorBhkWorld); when `00531F10` answers
/// another, `00531E90` sets the cell's. An exterior is a stop (the port's D2 exterior world has no entity).
/// Completion: 100% of an interior.
/// # Errors
/// An exterior cell.
pub fn player_sync_camera_collision_world(
    world: &mut World,
    player: Entity,
) -> Result<(), GameError> {
    use super::character_controller::{
        camera_collision_helper_set_world, camera_collision_helper_world,
    };
    let Some(helper) = world
        .get::<PlayerCharacter>(player)
        .and_then(|data| data.camera_collision_helper_1f0)
    else {
        return Ok(());
    };
    let Some(cell) = crate::tes::forms_references::parent_cell(world, player) else {
        return Ok(());
    };
    if !crate::tes::forms_world::cell_is_interior(world, cell) {
        return Err(GameError::unclassified(
            0x0066A61D,
            "0066A61D: the exterior's g_exteriorBhkWorld (00B35C24)",
        ));
    }
    let target = crate::tes::cell_havok::cell_bhk_world(world, cell);
    if camera_collision_helper_world(world, helper) != target {
        let calls = camera_collision_helper_set_world(world, helper, target);
        for _ in 0..calls.debug_colours_88bb60 {
            count(world, 0x0088_BB60);
        }
    }
    Ok(())
}

/// IDA: the globals `0065F080` keeps: 00B14E50 the player's camera fade (image 1.0), 00B14E54 the mount's (image 1.0) and
/// 00B3BB00 the mount it fades (null).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct PlayerCameraFadeGlobals {
    pub player_fade_b14e50: f32,
    pub mount_fade_b14e54: f32,
    pub fading_mount_b3bb00: Option<Entity>,
}

impl Default for PlayerCameraFadeGlobals {
    /// IDA: N/A the image's values.
    fn default() -> Self {
        Self {
            player_fade_b14e50: 1.0,
            mount_fade_b14e54: 1.0,
            fading_mount_b3bb00: None,
        }
    }
}

// IDA: N/A a game setting's float (SettingInfo_FloatValueOrZero).
fn setting_float(world: &World, name: &str) -> Result<f32, GameError> {
    world
        .get_resource::<crate::tes::game_settings::GameSettingCollection>()
        .and_then(|settings| settings.float_named(name))
        .ok_or(GameError::unclassified(
            0x0065F080,
            "0065F080: a missing game setting",
        ))
}

/// IDA: 0065F080 PlayerCharacter_ResolveCameraCollision(player, edx, position, look, immediate), the `immediate` arm
/// (a5 = 1, RaceSexMenu's 005C2DBB): self = position - look (floats), its length from 0043F350; with a camera
/// collision helper (+0x1F0), `005326B0`(look, position copy) and on a hit the length to the hit, sqrt as a float. Then
/// 00B3BACC: below the length it gains fChase3rdPersonZUnitsPerSecond x the frame's seconds up to the length, else it is the
/// length; clamped to fVanityModeWheelMin and fVanityModeWheelMax. The player's vt+0x380 (Character_GetMountedHorse, +0xD4);
/// with the RaceSexMenu (1036) open the fade-in arm: 00B14E50 below 1.0 (and 00B3BB00 set) reach `005EE1B0`, stops here.
/// `position` becomes look + self x 00B3BACC, each a float. `race_sex_open` is TileMenu_GetByMenuID(1036), the caller's
/// answer. Precision: 24-bit per step. Completion: 100% of the immediate arm with the RaceSexMenu open.
/// TODO: [ ] The smoothed arm (a5 = 0: 00B14E4D, 00B3BB38, fChase3rdPersonVanityXYMult and fChase3rdPersonXYMult), the
/// closed-menu arm (`005328B0`'s two casts, Actor_GetScaledBoundHeight, +0x589) and `005EE1B0`'s fade.
/// # Errors
/// The stops named, or one in the cast.
pub fn player_resolve_camera_collision(
    world: &mut World,
    player: Entity,
    position: &mut [f32; 3],
    look: &[f32; 3],
    immediate: bool,
    race_sex_open: bool,
) -> Result<(), GameError> {
    if !immediate {
        return Err(GameError::unclassified(
            0x0065F0BF,
            "0065F0BF: the smoothed camera arm (a5 = 0)",
        ));
    }
    let mut direction = [
        position[0] - look[0],
        position[1] - look[1],
        position[2] - look[2],
    ];
    let mut length = crate::gamebryo::transform::normalize_vector(&mut direction);
    if let Some(helper) = world
        .get::<PlayerCharacter>(player)
        .and_then(|data| data.camera_collision_helper_1f0)
    {
        let mut hit = *position;
        if super::character_controller::camera_collision_helper_cast(
            world, helper, *look, &mut hit,
        )? {
            let delta = [hit[0] - look[0], hit[1] - look[1], hit[2] - look[2]];
            length = (delta[1] * delta[1] + delta[0] * delta[0] + delta[2] * delta[2]).sqrt();
        }
    }
    let frame_seconds = world
        .get_resource::<crate::tes::script_vm::ScriptVmGlobals>()
        .map_or(0.0, |globals| globals.frame_seconds_b33e9c);
    let rate = setting_float(world, "fChase3rdPersonZUnitsPerSecond")? * frame_seconds;
    let minimum = setting_float(world, "fVanityModeWheelMin")?;
    let maximum = setting_float(world, "fVanityModeWheelMax")?;
    let distance = {
        let mut globals = world.resource_mut::<PlayerGlobals>();
        let current = globals.vanity_wheel_copy_b3bacc;
        let mut distance = if current < length {
            let grown = current + rate;
            if grown <= length { grown } else { length }
        } else {
            length
        };
        if minimum > distance {
            distance = minimum;
        }
        if maximum < distance {
            distance = maximum;
        }
        globals.vanity_wheel_copy_b3bacc = distance;
        distance
    };
    let mount = world
        .get::<super::actor::Actor>(player)
        .map_or(0, |actor| actor.horse_or_rider_d4);
    if mount != 0 {
        return Err(GameError::unclassified(
            0x0065F428,
            "0065F428: a mounted player's camera",
        ));
    }
    if !race_sex_open {
        return Err(GameError::unclassified(
            0x0065F445,
            "0065F445: the closed-menu arm's casts (005328B0)",
        ));
    }
    let fade = *world.get_resource_or_init::<PlayerCameraFadeGlobals>();
    if fade.player_fade_b14e50 < 1.0 {
        return Err(GameError::unclassified(
            0x0065F565,
            "0065F565: the player's fade-in (005EE1B0)",
        ));
    }
    if fade.fading_mount_b3bb00.is_some() {
        return Err(GameError::unclassified(
            0x0065F5D6,
            "0065F5D6: the mount's fade (005EE1B0)",
        ));
    }
    let offset = direction.map(|value| value * distance);
    *position = [
        offset[0] + look[0],
        offset[1] + look[1],
        look[2] + offset[2],
    ];
    Ok(())
}
