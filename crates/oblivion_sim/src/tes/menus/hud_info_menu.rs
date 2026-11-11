//! HUDInfoMenu: the crosshair's info panel.

use crate::tes::interface_frame::{
    MENU_STATE_FADING_IN, MENU_STATE_VISIBLE, menu_begin_fade_out, menu_destroy, menu_enable,
};
use crate::tes::interface_manager::{
    InterfaceManager, menu_register_tile, menu_update_cursor_depth,
};
use crate::tes::player::{PlayerCharacter, the_player};
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use crate::tes::tile_values::{
    TRAIT_DEPTH, TRAIT_STACKING, TRAIT_USER1, tile_numeric_trait, tile_set_numeric_trait,
};
use crate::tes::tile_xml::tile_read_xml;
use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

/// IDA: 005A4700 vt+4. Completion: 100%.
pub fn hud_info_set_field(info: &mut HudInfoMenu, id: i32, tile: TileId) {
    if (1..=11).contains(&id) {
        info.tiles_28[(id - 1) as usize] = Some(tile);
    }
}

/// IDA: 005A4840 HUDInfoMenu_Create(0), from 00582287: 00B3B33C's menu is null here, so nothing is destroyed. 00588890, Tile_ReadXML. Not a 1005 menu returns null, destroying what it made. 00584880, 005A46B0's warning over +0x28..+0x4C, the depth for a stacking type, the manager's +0x10 = user1 through `__ftol2_sse`, Menu_Enable(0). Completion: 100%.
/// # Errors
/// A stop in a callee.
pub fn hud_info_menu_create(
    cx: &mut InterfaceCx,
    frame_time: f32,
) -> Result<Option<TileId>, GameError> {
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let depth = menu_update_cursor_depth(cx)?;
    let Some(created) = tile_read_xml(cx, root, "Data\\Menus\\Main\\hud_info_menu.xml")? else {
        return Ok(None);
    };
    let Some(menu) = cx.engine.containing_menu(created) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != 1005 {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(None);
    }
    menu_register_tile(cx, menu, Some(created))?;
    let complete = match &cx.engine.menu(menu).class {
        MenuClass::HudInfo(info) => info.tiles_28[..10].iter().all(Option::is_some),
        _ => false,
    };
    if !complete {
        cx.host.warning(
            "HUD-Info Menu Creation Failed... Are your menu and art resources up to date?",
        );
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let user1 = tile_numeric_trait(cx, created, TRAIT_USER1);
    cx.world.resource_mut::<InterfaceManager>().unknown_010 = x87::ftol(user1) as u32;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(Some(created))
}

/// IDA: 005A5540 vt+0x2C: +0x58 counts to 60, then 0. A crosshair reference at +0x54 refreshes the text there, and no reached code sets one. Completion: 100% with no reference.
pub fn hud_info_update_frame(cx: &mut InterfaceCx) {
    let Some(menu) = cx.engine.tile_menu_by_id(1005) else {
        return;
    };
    let Some(menu) = cx.engine.containing_menu(menu) else {
        return;
    };
    if let MenuClass::HudInfo(info) = &mut cx.engine.menu_mut(menu).class {
        info.frames_58 += 1;
        if info.frames_58 > 60 {
            info.frames_58 = 0;
        }
    }
}

/// IDA: 005A4980 HUDInfoMenu_SetCrosshairRef(null, 0, 0), as LoadingMenu_Create calls it: no 00B3B33C menu, no player, a set player +0x5C0, or no tile returns 0. A null +0x54 with the menu fading in or visible returns 1 at once; otherwise +0x54 = null and, fading in or visible, 00584740; 1. Completion: 100% of the null reference's arm.
/// # Errors
/// A stop in 00584740.
pub fn hud_info_clear_crosshair(cx: &mut InterfaceCx, frame_time: f32) -> Result<bool, GameError> {
    let Some(menu) = cx
        .engine
        .tile_menu_by_id(1005)
        .and_then(|tile| cx.engine.containing_menu(tile))
    else {
        return Ok(false);
    };
    let Some(player) = the_player(cx.world) else {
        return Ok(false);
    };
    if cx
        .world
        .get::<PlayerCharacter>(player)
        .is_some_and(|body| body.byte_5c0 != 0)
        || cx.engine.menu(menu).tile_04.is_none()
    {
        return Ok(false);
    }
    let state = cx.engine.menu(menu).state_24;
    let shown = state == MENU_STATE_FADING_IN || state == MENU_STATE_VISIBLE;
    let MenuClass::HudInfo(info) = &mut cx.engine.menu_mut(menu).class else {
        return Err(GameError::unclassified(
            0x005A4980,
            "005A4980: 00B3B33C isn't a HUDInfoMenu",
        ));
    };
    if info.crosshair_54.is_none() && shown {
        return Ok(true);
    }
    info.crosshair_54 = None;
    if shown {
        menu_begin_fade_out(cx, menu, frame_time)?;
    }
    Ok(true)
}

/// IDA: 005A5540 HUDInfoMenu vt+0x2C. Completion: `hud_info_update_frame`'s.
/// # Errors
/// None; the slot's signature.
pub fn hud_info_frame(
    cx: &mut InterfaceCx,
    _menu: MenuId,
    _frame_time: f32,
) -> Result<(), GameError> {
    hud_info_update_frame(cx);
    Ok(())
}

/// IDA: HUDInfoMenu (0x5C): 005A4660's fields, and 005A4700's IDs 1 to 11 at +0x28..+0x50. +0x54 is the crosshair's reference and +0x58 a frame count. Completion: 100% of what a new game reaches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HudInfoMenu {
    /// +0x28..+0x50.
    pub tiles_28: [Option<TileId>; 11],
    /// +0x54, the crosshair's reference.
    pub crosshair_54: Option<Entity>,
    /// +0x58.
    pub frames_58: i32,
}
