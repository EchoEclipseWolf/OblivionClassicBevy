//! Phase 17's scene half: WinMain 0040EF24..0040EFE9. 0040EFEE, 00578CD0, is the driver's call, through the interface. `ebp` is 0 from 0040EE78 on, read from the bytes; `esi` holds the cell until 0040EF9E. The no-cell jump 0040EF08 lands at 0040EF54.
//!
//! Evidence: disassembly; `run-h-phase17/snapshot-phase16_end` and `snapshot-phase17_end`, seed 6AEE4398: the default path only, with no cell.

use crate::bs::scene_graphs::SceneGraphGlobals;
use crate::gamebryo::scene_node::{NiNodeChildren, SceneNodePayload};
use crate::gamebryo::scene_update::{SceneUpdateRuntime, SceneUpdateStatus, update_scene_object};
use crate::tes::forms_init_world::init_reference_script;
use crate::tes::player::the_player;
use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

// IDA: 0040EF5F..0040EF77, again at 0040EF8E and 0040EFC7: g_worldSceneGraph's child 0, `[eax+0B0h]`'s first word when the +0xB6 count isn't 0, otherwise 0, with no test before the stores.
fn world_child_0(world: &World) -> Option<Entity> {
    let graph = world.resource::<SceneGraphGlobals>().world?;
    let children = world.get::<NiNodeChildren>(graph)?;
    children.slots_ac.first().copied().flatten()
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
) -> Result<(), GameError> {
    let player = the_player(world).ok_or(GameError::unclassified(
        0x0040EF24,
        "0040EF24: a null g_thePlayer",
    ))?;
    if cell.is_some() {
        return Err(GameError::unclassified(
            0x0040EF32,
            "0040EF32: 00707610 on the player's third-person root, whose 3D isn't ported",
        ));
    }
    init_reference_script(world, player)?;
    // There's no null guard between the count and the stores.
    let child = world_child_0(world).ok_or(GameError::unclassified(
        0x0040EF7D,
        "0040EF7D: a null child 0 faults",
    ))?;
    world
        .get_mut::<SceneNodePayload>(child)
        .expect("a scene node")
        .local_transform
        .translation = rect;
    let rotation = crate::tes::forms_references::reference_initial_rotation(world, player)?;
    world
        .get_mut::<SceneNodePayload>(child)
        .expect("a scene node")
        .local_transform
        .rotation = rotation;
    let status = update_scene_object(world, child, 0.0, true, runtime)?;
    if status.status != SceneUpdateStatus::Updated {
        return Err(GameError::unclassified(
            0x0040EFE9,
            "0040EFE9: NiAVObject_Update reported an unsupported state",
        ));
    }
    Ok(())
}
