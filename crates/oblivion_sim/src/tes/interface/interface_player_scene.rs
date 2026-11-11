//! Phase 17: 00578CD0 InterfaceManager_InitializePlayerScene, the node, shadow scene and light the Stats tab's player stands in, and 0057D480, which builds the HUD and sets the menu root's user0 to 2.0: from here the game counts as running. That turns on 005B6040's 0057CC00 and 00579770, also here. On a start with no menus up and the player's 3D absent, they come down to updating +0x60, setting its flag and emptying the mode stack. Every arm that would do more is a stop.
//!
//! Evidence: disassembly; `run-h-phase17`, `snapshot-phase17_end` and `snapshot-menu_closed`, seed 6AEE4398.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::interface_frame::menu_destroy;
use super::interface_manager::InterfaceManager;
use super::interface_scene::{interface_update_tile_scene, tile_scene_node};
use super::tile::InterfaceCx;
use super::tile_values::{TRAIT_USER0, TRAIT_VISIBLE, tile_numeric_trait, tile_set_numeric_trait};
use crate::bs::scene_graphs::{
    AttachChildStatus, SceneGraphGlobals, ShadowSceneNode, attach_scene_child,
    construct_shadow_scene_node,
};
use crate::bs::scene_lighting::set_shadow_scene_sun_light;
use crate::gamebryo::object_net::set_object_name;
use crate::gamebryo::object_net::{AlphaProperty, NiObjectNet};
use crate::gamebryo::scene_derived::{DirectionalLight, spawn_directional_light};
use crate::gamebryo::scene_node::{SceneNodePayload, spawn_scene_node};
use crate::gamebryo::scene_update::{SceneUpdateStatus, update_scene_object};
use crate::gamebryo::stream_objects::NiClass;
use crate::gamebryo::transform::matrix_from_euler_xyz;
use crate::tes::actor::{MobileObject, TesObjectRefr};
use crate::tes::actor_process::MiddleHighProcess;
use crate::tes::hud_main_menu::{HudSettings, hud_main_menu_create, hud_update_visibility};
use crate::tes::loading_menu::loading_menu_set_section_progress_if_ready;
use crate::tes::player::PlayerCharacter;
use crate::tes::player::the_player;

/// IDA: what 0057E510 reads from the INI: fMenuPlayerLightAmbient at 00B135C8, 00B135D0, 00B135D8, and fMenuPlayerLightDiffuse at 00B135E0, 00B135E8, 00B135F0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerSceneSettings {
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
    pub hud: HudSettings,
}

// IDA: N/A a helper, with no address of its own.
fn require_attached(status: AttachChildStatus, message: &'static str) -> Result<(), GameError> {
    if status == AttachChildStatus::Attached {
        Ok(())
    } else {
        Err(GameError::from_legacy_message(message))
    }
}

/// IDA: 00578CD0 InterfaceManager_InitializePlayerScene, its tail jump into 0057E510, which ends in 0057D480. A failed allocation's null store isn't modelled. `frame_time` stands for g_timeInfo in what the HUD's enable fades. Completion: 100% of the order.
/// # Errors
/// No manager, or a stop in a callee.
pub fn interface_initialize_player_scene(
    cx: &mut InterfaceCx,
    settings: &PlayerSceneSettings,
    frame_time: f32,
) -> Result<(), GameError> {
    let menu_3d = cx
        .world
        .get_resource::<InterfaceManager>()
        .ok_or(GameError::unclassified(
            0x00578CDE,
            "00578CDE: no InterfaceManager",
        ))?
        .menu_3d_scene_graph_004
        .ok_or(GameError::unclassified(
            0x0057E73C,
            "0057E73C: a null +0x04 faults",
        ))?;
    // 0057E539..0057E5A1: a NiNode with no child slots, with 00711580(0, 0, 3.7525...) copied to its +0x30.
    let node = spawn_scene_node(cx.world, SceneNodePayload::default());
    cx.world
        .resource_mut::<InterfaceManager>()
        .player_scene_node_060 = Some(node);
    cx.world
        .get_mut::<SceneNodePayload>(node)
        .expect("just spawned")
        .local_transform
        .rotation = matrix_from_euler_xyz(0.0, 0.0, f32::from_bits(0x4070_2845));
    // 0057E5A3..0057E5D9: the shadow scene, +0x64, its +0x11C = 1, and 007B4270(1, it).
    let shadow = construct_shadow_scene_node(cx.world);
    cx.world
        .resource_mut::<InterfaceManager>()
        .player_shadow_scene_064 = Some(shadow);
    cx.world
        .get_mut::<ShadowSceneNode>(shadow)
        .expect("just constructed")
        .byte_11c = 1;
    if let Some(mut globals) = cx.world.get_resource_mut::<SceneGraphGlobals>() {
        globals.shadow_slots[1] = Some(shadow);
    }
    // 0057E5DE..0057E623: a NiAlphaProperty, flags 0xEC with bit 0 clear, on the shadow scene.
    let alpha = cx
        .world
        .spawn((
            NiClass::AlphaProperty,
            NiObjectNet::default(),
            AlphaProperty {
                flags_18: 0x00EC & 0xFFFE,
                alpha_test_ref_1a: 0,
            },
        ))
        .id();
    crate::gamebryo::scene_state::attach_object_property(cx.world, shadow, Some(alpha))?;
    // 0057E628..0057E65E: "PlayerSceneLight" at +0x18.
    let light = spawn_directional_light(cx.world);
    cx.world.resource_mut::<InterfaceManager>().light_018 = Some(light);
    set_object_name(
        &mut cx
            .world
            .get_mut::<NiObjectNet>(light)
            .expect("just spawned")
            .name,
        Some(b"PlayerSceneLight"),
    );
    // 0057E663..0057E719: +0xB8 += 1, diffuse at +0xEC; +0xB8 += 1, ambient at +0xE0; 00711580(a, a, a) of A68FB4 to +0x30.
    let tilt = f32::from_bits(0xBEDF_66F3);
    {
        let mut state = cx
            .world
            .get_mut::<DirectionalLight>(light)
            .expect("just spawned");
        state.unknown_b8 += 1;
        state.payload.diffuse = settings.diffuse;
        state.unknown_b8 += 1;
        state.payload.ambient = settings.ambient;
        state.payload.av.local_transform.rotation = matrix_from_euler_xyz(tilt, tilt, tilt);
    }
    // 0057E71B..0057E72B vt+0x84(light, 1).
    require_attached(
        attach_scene_child(cx.world, shadow, Some(light), true),
        "0057E72B: the light was not attached",
    )?;
    // 0057E72D..0057E734 007C5850.
    set_shadow_scene_sun_light(
        &mut cx
            .world
            .get_mut::<ShadowSceneNode>(shadow)
            .expect("just constructed"),
        light,
    );
    // 0057E739..0057E74A +0x04's vt+0x84(+0x64, 0); 0057E74C..0057E75D +0x64's vt+0x84(+0x60, 0).
    require_attached(
        attach_scene_child(cx.world, menu_3d, Some(shadow), false),
        "0057E74A: the shadow scene was not attached",
    )?;
    require_attached(
        attach_scene_child(cx.world, shadow, Some(node), false),
        "0057E75D: the node was not attached",
    )?;
    // 0057E75F..0057E791 00708560(+0x60, 2): a new node has no property of that type, so what comes back is null.
    // 0057E793..0057E79C flag 1 on +0x60, +0x08 = 1.
    cx.world
        .get_mut::<SceneNodePayload>(node)
        .expect("just spawned")
        .effective_av_flags |= 1;
    cx.world
        .resource_mut::<InterfaceManager>()
        .menu_loading_status_008 = 1;
    interface_finish_player_scene(cx, settings.hud, frame_time)
}

/// IDA: 0057D480: 0057B950(3, 50.0), (3, 5.0), 005A6B00, 005A6040(1, 1), the HUD's visible = 1 when it was built, (3, 100.0), flag 1 on the cursor's node, its visible = 1, 0058E870, +0x08 = 1 again, and the root's user0 = 2.0. 0057B950 tests the manager's cursor. Completion: 100%.
/// # Errors
/// A null cursor or root, or a stop in a callee.
pub fn interface_finish_player_scene(
    cx: &mut InterfaceCx,
    settings: HudSettings,
    frame_time: f32,
) -> Result<(), GameError> {
    let cursor = cx.world.resource::<InterfaceManager>().cursor_01c;
    loading_menu_set_section_progress_if_ready(cx, 3, 50.0)?;
    loading_menu_set_section_progress_if_ready(cx, 3, 5.0)?;
    let created = hud_main_menu_create(cx, settings, frame_time)?;
    hud_update_visibility(cx, true, true, frame_time)?;
    if let Some(created) = created {
        tile_set_numeric_trait(cx, created, TRAIT_VISIBLE, 1.0)?;
    }
    loading_menu_set_section_progress_if_ready(cx, 3, 100.0)?;
    let cursor = cursor.ok_or(GameError::unclassified(
        0x0057D4E4,
        "0057D4E4: a null cursor faults",
    ))?;
    if let Some(node) = tile_scene_node(cx, cursor) {
        cx.world
            .get_mut::<SceneNodePayload>(node)
            .expect("a tile node has a payload")
            .effective_av_flags |= 1;
    }
    tile_set_numeric_trait(cx, cursor, TRAIT_VISIBLE, 1.0)?;
    interface_update_tile_scene(cx, cursor)?;
    cx.world
        .resource_mut::<InterfaceManager>()
        .menu_loading_status_008 = 1;
    let root = cx
        .world
        .resource::<InterfaceManager>()
        .menu_root_068
        .ok_or(GameError::unclassified(
            0x0057D521,
            "0057D521: a null root faults",
        ))?;
    tile_set_numeric_trait(cx, root, TRAIT_USER0, 2.0)
}

/// IDA: the head every 0057CC00 and 00579770 callee shares: a manager, its cursor and root, and the root's user0 at 2.0. Completion: 100%.
#[must_use]
pub fn interface_game_is_running(cx: &InterfaceCx) -> bool {
    let Some(manager) = cx.world.get_resource::<InterfaceManager>() else {
        return false;
    };
    let (Some(_), Some(root)) = (manager.cursor_01c, manager.menu_root_068) else {
        return false;
    };
    tile_numeric_trait(cx, root, TRAIT_USER0) == 2.0
}

// IDA: 0057C420, 0057C5F0, 0057C7C0, 0057A480 InterfaceManager_Update*MenuUI(false, true): with no such menu, 005790E0 answers false and the else arm has no menu to close, so what's left is 00707370(+0x60, 0.0, 0).
fn close_big_four_menu(cx: &mut InterfaceCx, id: u32, stop: &'static str) -> Result<(), GameError> {
    if cx.engine.tile_menu_by_id(id).is_some() {
        return Err(GameError::from_legacy_message(stop));
    }
    let node = cx
        .world
        .resource::<InterfaceManager>()
        .player_scene_node_060
        .ok_or(GameError::unclassified(
            0x0057C5E1,
            "0057C5E1: a null +0x60 faults",
        ))?;
    let status = update_scene_object(cx.world, node, 0.0, false, cx.runtime)?;
    if status.status != SceneUpdateStatus::Updated {
        return Err(GameError::unclassified(
            0x0057C5E1,
            "0057C5E1: NiAVObject_Update reported an unsupported state",
        ));
    }
    Ok(())
}

/// IDA: 0057CC00 InterfaceManager_CloseInventoryMenusAndRestorePlayer: when a game runs, close Map, Magic, Inventory and Stats, 0057ECB0, 0057AA90 (005982A0), 0057B600 (005BDCD0), and 005E7230(player, null, 1). Completion: 100% of the arms with no such menus and nothing equipped.
/// TODO: [ ] The menus' own arms, the first-person biped and the equipped items' visuals, as they come up.
/// # Errors
/// An arm that isn't ported.
pub fn interface_close_inventory_menus_and_restore_player(
    cx: &mut InterfaceCx,
) -> Result<(), GameError> {
    if !interface_game_is_running(cx) {
        return Ok(());
    }
    close_big_four_menu(cx, 1023, "0057C4B2: closing a Map menu is carried")?;
    close_big_four_menu(cx, 1022, "0057C682: closing a Magic menu is carried")?;
    close_big_four_menu(cx, 1002, "0057C852: closing an Inventory menu is carried")?;
    close_big_four_menu(cx, 1003, "0057A512: closing a Stats menu is carried")?;
    let player = the_player(cx.world).ok_or(GameError::unclassified(
        0x0057ECB1,
        "0057ECB1: a null g_thePlayer faults",
    ))?;
    // 0057ECB0: +0x60's vt+0x88 RemoveObject(+0x5D8), +0x60 flag 1, 00668D00(player, 0), which with no biped does nothing.
    if cx
        .world
        .get::<PlayerCharacter>(player)
        .ok_or(GameError::unclassified(
            0x0057ECB1,
            "0057ECB1: g_thePlayer is no PlayerCharacter",
        ))?
        .first_person_biped_root_5d8
        != 0
    {
        return Err(GameError::unclassified(
            0x0057ECB7,
            "0057ECB7: removing a first-person biped is carried",
        ));
    }
    let node = cx
        .world
        .resource::<InterfaceManager>()
        .player_scene_node_060
        .ok_or(GameError::unclassified(
            0x0057ECC0,
            "0057ECC0: a null +0x60 faults",
        ))?;
    cx.world
        .get_mut::<SceneNodePayload>(node)
        .expect("a scene node")
        .effective_av_flags |= 1;
    // 005982A0: with no Container menu, nothing. A Negotiate, Quantity or Message menu only returns sooner.
    if cx.engine.tile_menu_by_id(1008).is_some() {
        return Err(GameError::unclassified(
            0x005982E7,
            "005982E7: closing a Container menu is carried",
        ));
    }
    // 0057B600 -> 005BDCD0: 00B3B740 has no writer here, and run-h-phase17 reads 0, so 005DDCD0 skips the LOD arm. Then no Pause menu to close.
    if cx.engine.tile_menu_by_id(1013).is_some() {
        return Err(GameError::unclassified(
            0x0057B638,
            "0057B638: a Pause menu is carried",
        ));
    }
    // 005E7230: no process returns. +0xCD4's byte has no writer here. With nothing equipped each arm falls to 005E7A06's frees, and the two FindEquippedInBipedSlot lookups only allocate what those frees free.
    if let Some(process) = cx
        .world
        .get::<MobileObject>(player)
        .and_then(|mobile| mobile.process_58)
    {
        let equipped = cx
            .world
            .get::<MiddleHighProcess>(process)
            .ok_or(GameError::unclassified(
                0x005E7255,
                "005E7255: the player's process has no equipment",
            ))?
            .clone();
        if equipped.weapon_e4.is_some()
            || equipped.light_e8.is_some()
            || equipped.ammo_ec.is_some()
            || equipped.shield_f0.is_some()
        {
            return Err(GameError::unclassified(
                0x005E72A8,
                "005E72A8: equipped items' visuals are carried",
            ));
        }
    }
    Ok(())
}

/// IDA: 00579770 InterfaceManager_CloseAllMenusIfMenuMode: when a game runs, 0057CEE0: from slot 9 down, destroy each mode's menu and clear the slot, then 005B41E0 (MagicPopup), 0057CC00 again, 005964B0 (Breath), 0059D890 (00B3B294 = 0, no reader here) and +0x08 = 4. Completion: 100% of the arms with no MagicPopup or Breath menu.
/// # Errors
/// A MagicPopup or Breath menu, or a stop in 0057CC00.
pub fn interface_close_all_menus_if_menu_mode(cx: &mut InterfaceCx) -> Result<(), GameError> {
    if !interface_game_is_running(cx) {
        return Ok(());
    }
    for slot in (0..10).rev() {
        let mode = cx.world.resource::<InterfaceManager>().menu_mode_stack_0e0[slot];
        if mode == 0 {
            continue;
        }
        if let Some(tile) = cx.engine.tile_menu_by_id(mode)
            && let Some(menu) = cx.engine.containing_menu(tile)
        {
            menu_destroy(cx, menu)?;
        }
        cx.world
            .resource_mut::<InterfaceManager>()
            .menu_mode_stack_0e0[slot] = 0;
    }
    if cx.engine.tile_menu_by_id(1024).is_some() {
        return Err(GameError::unclassified(
            0x005B41F4,
            "005B41F4: closing a MagicPopup menu is carried",
        ));
    }
    interface_close_inventory_menus_and_restore_player(cx)?;
    if cx.engine.tile_menu_by_id(1045).is_some() {
        return Err(GameError::unclassified(
            0x005964C1,
            "005964C1: closing a Breath menu is carried",
        ));
    }
    cx.world
        .resource_mut::<InterfaceManager>()
        .menu_loading_status_008 = 4;
    Ok(())
}

/// IDA: 00578CF0 InterfaceManager_UpdateReticleIfPlayerHas3D(mode): the player's vt+0x1A8 00612220, which for the player is +0x3C. `run-h-phase17` holds it null at 005B6093. Completion: 100% of the arm with no 3D.
/// TODO: [ ] 0057D940, when the player has 3D.
/// # Errors
/// The unported arm.
pub fn interface_update_reticle_if_player_has_3d(cx: &InterfaceCx) -> Result<(), GameError> {
    let Some(player) = the_player(cx.world) else {
        return Ok(());
    };
    if cx
        .world
        .get::<TesObjectRefr>(player)
        .is_some_and(|refr| refr.loaded_3d_3c != 0)
    {
        return Err(GameError::unclassified(
            0x00578D0D,
            "00578D0D: 0057D940 the reticle is carried",
        ));
    }
    Ok(())
}
