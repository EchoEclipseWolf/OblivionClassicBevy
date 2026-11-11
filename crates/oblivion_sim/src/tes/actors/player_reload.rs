//! The player's 3D dropped and loaded again, as `005CDA30` (the race menu's race change) does when the new race's tail
//! differs from the old one's: TESObjectREFR_Set3D(null) (`004E0F80`), the NPC's skeleton set, and Character_Load3D
//! (`00659F30`). Each arm is ported for the race menu's player (no weapon, shield, torch, quiver, light, horse, sound or
//! teleport); anything else is a stop at its site.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::character_3d::{SkinInfo, SkinSlotAt, release_skin_slot};
use crate::bs::model_loader::ModelBuildServices;

// IDA: 00478CE0 (the weapon, slot 9: 00478780(+0xDC, 1, 0)), then with the owner an actor (vt+0x190) whose base isn't a
// creature (type 0x24, 005E3270) and with a process whose +0x115 is set (005E0DA0, vt+0x304 0064AD30), the animation data's
// +0xC8 = the owner: a stop (the race menu's player has no weapon out).
pub(crate) fn release_weapon(world: &mut World, skin: Entity) -> Result<(), GameError> {
    release_skin_slot(world, skin, SkinSlotAt::Skin(9), true, None)?;
    let owner = world.get::<SkinInfo>(skin).expect("a skin info").owner_150;
    let drawn = crate::tes::actor::process_of(world, owner)
        .and_then(|process| world.get::<super::actor_process::MiddleHighProcess>(process))
        .is_some_and(|middle| middle.weapon_drawn_115 != 0);
    if drawn {
        return Err(GameError::unclassified(
            0x00478D5D,
            "00478D5D: a drawn weapon's animation data +0xC8 (00478CE0)",
        ));
    }
    Ok(())
}

// IDA: 0065AB80 the player's vt+0x144 on its node (+0x3C, `node`): vt+0x1C8 (`0065A380`: with a character controller,
// `00893950` (+0x1F4 bit 0x8000's display node detached, a stop when set; the bit cleared) and bhkCharacterController_SetWorld
// (null) (`controller_set_world`)); then NiAVObject_RemoveCollisionFromWorld(node, 1, 0) with a node.
pub(crate) fn player_remove_from_world(
    world: &mut World,
    player: Entity,
    node: Option<Entity>,
) -> Result<(), GameError> {
    if let Some(controller) = super::character_controller::actor_character_controller(world, player)
    {
        let mut c = world
            .get_mut::<crate::bs::character_controller::BhkCharacterController>(controller)
            .ok_or(GameError::unclassified(
                0x0065A38F,
                "0065A38F: a controller slot that isn't a controller",
            ))?;
        if c.flags_1f4 & 0x8000 != 0 {
            return Err(GameError::unclassified(
                0x00891175,
                "00891175: the controller's display node (00891160)",
            ));
        }
        c.flags_1f4 &= !0x8000;
        crate::bs::character_controller::controller_set_world(world, controller, None);
    }
    crate::bs::havok_world::remove_collision_from_world(world, node, true, false)?;
    Ok(())
}

/// IDA: 00659F30 Character_Load3D: `004E4080`'s body (`load_3d_body`); with a node and a process, the node's "BBX" extra
/// (NiObjectNET_GetExtraData) into the process's vt+0x470 (`00629590`: +0x188) when there is one; an actor's node into the
/// shadow scene's lighting (`SceneLighting_FindOrCreateNodeEntry`, counted). Completion: 100%.
/// # Errors
/// A stop in the load.
pub fn character_load_3d(
    world: &mut World,
    actor: Entity,
    services: &mut ModelBuildServices<'_>,
) -> Result<Option<Entity>, GameError> {
    let node = crate::tes::reference_3d::load_3d_body(world, actor, services)?;
    if let Some(node) = node {
        // 00659F45 NiObjectNET_GetExtraData(node, "BBX") into the process's vt+0x470 (00629590: +0x188).
        let bound = crate::gamebryo::object_net::find_extra_data(world, node, b"BBX")?;
        if let Some(bound) = bound
            && let Some(process) = crate::tes::actor::process_of(world, actor)
            && let Some(mut middle) =
                world.get_mut::<super::actor_process::MiddleHighProcess>(process)
        {
            middle.bound_188 = Some(bound);
        }
        super::equipment_3d::count(world, 0x0065_9F80);
    }
    Ok(node)
}
