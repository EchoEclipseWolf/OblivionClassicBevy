//! Phase 17's scene half: WinMain 0040EF24..0040EFE9. 0040EFEE, 00578CD0, is the driver's call, through the interface. `ebp` is 0 from 0040EE78 on, read from the bytes; `esi` holds the cell until 0040EF9E. The no-cell jump 0040EF08 lands at 0040EF54.
//!
//! Evidence: disassembly; `run-h-phase17/snapshot-phase16_end` and `snapshot-phase17_end`, seed 6AEE4398: the default path only, with no cell.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_game::bs::scene_graphs::SceneGraphGlobals;
use recomp_game::gamebryo::path_interpolator::multiply;
use recomp_game::gamebryo::scene_node::{NiNodeChildren, SceneNodePayload};
use recomp_game::gamebryo::scene_update::{
    SceneUpdateRuntime, SceneUpdateStatus, update_scene_object,
};
use recomp_game::tes::actor::TesObjectRefr;
use recomp_game::tes::ai_conditions::the_player;
use recomp_game::tes::forms_init_world::init_reference_script;
use recomp_game::tes::hud_main_menu::{base_race, npc_sex, race_scale};

// IDA: 0040EF5F..0040EF77, again at 0040EF8E and 0040EFC7: g_worldSceneGraph's child 0, `[eax+0B0h]`'s first word when the +0xB6 count isn't 0, otherwise 0, with no test before the stores.
fn world_child_0(world: &World) -> Option<Entity> {
    let graph = world.resource::<SceneGraphGlobals>().world?;
    let children = world.get::<NiNodeChildren>(graph)?;
    children.slots_ac.first().copied().flatten()
}

/// IDA: 004D7AF0 TESObjectREFR_BuildInitialRotationMatrix for the player: identity, vt+0x190 (00977C50, true) skips X and Y, 0070FDD0 of +0x28, each `fsincos` at full precision and stored as a float (D6). Then an NPC base with a race multiplies in diag(weight, weight, height) from the race's +0x68 and +0x60, by sex: a sex above 1 can't come from an NPC's flag bit, so the 0.0 arm stays out. Each 007100A0 is per step at 24 bits. Completion: 100% for the player.
/// # Errors
/// No base form, or a creature's.
pub fn player_initial_rotation(world: &World, player: Entity) -> Result<[f32; 9], StreamError> {
    let refr = world.get::<TesObjectRefr>(player).expect("a reference");
    let angle = refr.rotation_20[2];
    let (cos_z, sin_z) = (f64::from(angle).cos() as f32, f64::from(angle).sin() as f32);
    let about_z = [cos_z, sin_z, 0.0, -sin_z, cos_z, 0.0, 0.0, 0.0, 1.0];
    let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    let mut matrix = multiply(&identity, &about_z);
    let base = refr
        .base_form_1c
        .ok_or(StreamError("004D7BA3: a null base faults"))?;
    if let Some(race) = base_race(world, base)? {
        let sex = npc_sex(world, base).expect("base_race said an NPC");
        let weight = race_scale(world, race, 0x68 + 4 * sex);
        let height = race_scale(world, race, 0x60 + 4 * sex);
        let scale = [weight, 0.0, 0.0, 0.0, weight, 0.0, 0.0, 0.0, height];
        matrix = multiply(&matrix, &scale);
    }
    Ok(matrix)
}

/// IDA: WinMain 0040EF24..0040EFE9. With a cell, 00660110(player, 0), which with an argument of 0 is Character_GetNiNode, the third-person root, and not the first-person node its name says: its 00707610 and 007C6C30 need the player's 3D, which the port doesn't load, so that's a stop. Then 004D70E0(player); child 0's +0x54 = Rect, its +0x30 = 004D7AF0, and 00707370(child, 0.0, 1). Completion: 100% of the no-cell path.
/// TODO: [ ] 0040EF24..0040EF4F with the player's 3D, and rows from a start in a cell: 0040EF2A's `ebp` isn't checked by a run, since the capture took the default path.
/// # Errors
/// A cell, a null child 0, or a stop in a callee.
pub fn run_player_scene(
    world: &mut World,
    rect: [f32; 3],
    cell: Option<Entity>,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<(), StreamError> {
    let player = the_player(world).ok_or(StreamError("0040EF24: a null g_thePlayer"))?;
    if cell.is_some() {
        return Err(StreamError(
            "0040EF32: 00707610 on the player's third-person root, whose 3D isn't ported",
        ));
    }
    init_reference_script(world, player)?;
    // There's no null guard between the count and the stores.
    let child = world_child_0(world).ok_or(StreamError("0040EF7D: a null child 0 faults"))?;
    world
        .get_mut::<SceneNodePayload>(child)
        .expect("a scene node")
        .local_transform
        .translation = rect;
    let rotation = player_initial_rotation(world, player)?;
    world
        .get_mut::<SceneNodePayload>(child)
        .expect("a scene node")
        .local_transform
        .rotation = rotation;
    let status = update_scene_object(world, child, 0.0, true, runtime)?;
    if status.status != SceneUpdateStatus::Updated {
        return Err(StreamError(
            "0040EFE9: NiAVObject_Update reported an unsupported state",
        ));
    }
    Ok(())
}
