//! MessageMenu (1001): ShowMessageBox, the menu's creation and its close, and the Yes/No texts.
use oblivion_core::error::GameError;

use super::*;
use crate::tes::game_settings::text_at_or_empty;
use crate::tes::interface_frame::*;
use crate::tes::interface_manager::{
    InterfaceManager, interface_update_menu_root, menu_register_tile, menu_update_cursor_depth,
};
use crate::tes::tile::{InterfaceCx, MenuClass};
use crate::tes::tile_values::*;
use crate::tes::tile_xml::tile_read_xml;

/// IDA: dword_B38CF8 sYes and unk_B38D00 sNo, the settings in 005B64C0's pushes. Completion: 100%.
pub(super) fn yes_no(cx: &InterfaceCx) -> (Vec<u8>, Vec<u8>) {
    (
        text_at_or_empty(cx.world, 0x00B3_8CF8),
        text_at_or_empty(cx.world, 0x00B3_8D00),
    )
}

/// IDA: 00579C10 ShowMessageBox (text, callback, base, buttons...): no manager, cursor or root returns false. The menu root's user0 2.0 takes 005BC8B0; anything else takes 005BCC00, which runs the same and then shows the menu at once (0057EA20(1.0), visible 2, destroyOnClose 2, state Visible). Completion: 100%.
pub fn show_message(
    cx: &mut InterfaceCx,
    text: &[u8],
    callback: Option<MessageCallback>,
    button_base: u8,
    buttons: &[Vec<u8>],
) -> Result<bool, GameError> {
    let manager = cx.world.resource::<InterfaceManager>();
    let (Some(_), Some(root)) = (manager.cursor_01c, manager.menu_root_068) else {
        return Ok(false);
    };
    let in_game = tile_numeric_trait(cx, root, TRAIT_USER0) == 2.0;
    let created = message_menu_create(cx, text, callback, button_base, buttons)?;
    if !in_game && let Some(tile) = cx.engine.tile_menu_by_id(1001) {
        interface_apply_fade(cx, tile, 1.0)?;
        tile_set_numeric_trait(cx, tile, TRAIT_VISIBLE, 2.0)?;
        tile_set_numeric_trait(cx, tile, TRAIT_DESTROY_ON_CLOSE, 2.0)?;
        if let Some(menu) = cx.engine.containing_menu(tile) {
            cx.engine.menu_mut(menu).state_24 = MENU_STATE_VISIBLE;
        }
        tile_update_scene_graph(cx, tile, false)?;
    }
    Ok(created)
}

/// IDA: 005BC8B0: an old menu deleted; 00584980; 005903E0; Tile_ReadXML(message_menu.xml); the class test; 00584880; the +0x2C, +0x30, +0x28 test (IDs 2, 3, 1); the depth; +0xB0 = -1; +0x5C = callback; a null first button hides it, else its text; the rest of the buttons take IDs from 5; the message text, or sDefaultMessage; +0x60 = base; user0 = status != 1; 0057DE50(0xB); two 0058FBA0; 00585190(0); visible 2. Completion: 100% except 00572DF0(2) and 0040D160, a screen effect with no capture on this path, counted.
pub fn message_menu_create(
    cx: &mut InterfaceCx,
    text: &[u8],
    callback: Option<MessageCallback>,
    button_base: u8,
    buttons: &[Vec<u8>],
) -> Result<bool, GameError> {
    if let Some(old) = cx.engine.tile_menu_by_id(1001)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    interface_update_menu_root(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let Some(created) = tile_read_xml(cx, root, "Data\\Menus\\message_menu.xml")? else {
        return Ok(false);
    };
    let Some(menu) = cx.engine.containing_menu(created) else {
        return Ok(false);
    };
    if cx.engine.menu(menu).id() != 1001 {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(false);
    }
    menu_register_tile(cx, menu, Some(created))?;
    let MenuClass::Message(fields) = cx.engine.menu(menu).class.clone() else {
        return Ok(false);
    };
    let tiles = fields.tiles_28;
    let (Some(message_text), Some(_), Some(_), first_button) =
        (tiles[1], tiles[2], tiles[0], tiles[3])
    else {
        // 005BC99D: DebugLog_Printf, where 005B61B6's MainMenu warns.
        cx.host.debug_log(
            "Message Menu Creation Failed... Are your menu and art resources up to date?",
        );
        return Ok(false);
    };
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    cx.world
        .resource_mut::<InterfaceManager>()
        .msg_box_button_pressed_0b0 = 0xFF;
    let mut labels = buttons.iter();
    let first = labels.next();
    if let Some(button) = first_button {
        match first {
            Some(label) if !label.is_empty() => {
                tile_set_string_trait(cx, button, TRAIT_STRING, label)?;
            }
            Some(_) => {
                // 005BCA1B: sOk.
                let ok = text_at_or_empty(cx.world, 0x00B3_8CF0);
                tile_set_string_trait(cx, button, TRAIT_STRING, &ok)?;
            }
            None => tile_set_numeric_trait(cx, button, TRAIT_VISIBLE, 1.0)?,
        }
    }
    let mut id = 4usize;
    for label in labels {
        if label.is_empty() {
            break;
        }
        id += 1;
        // 005BC5E0(id).
        match tiles.get(id - 1).copied().flatten() {
            Some(button) => {
                tile_set_string_trait(cx, button, TRAIT_STRING, label)?;
                tile_set_numeric_trait(cx, button, TRAIT_VISIBLE, 2.0)?;
            }
            None => cx.host.debug_log("Missing MessageMenu button"),
        }
    }
    // 005BCB24: sDefaultMessage.
    let shown = if text.is_empty() {
        text_at_or_empty(cx.world, 0x00B3_8E10)
    } else {
        text.to_vec()
    };
    tile_set_string_trait(cx, message_text, TRAIT_STRING, &shown)?;
    if let MenuClass::Message(message) = &mut cx.engine.menu_mut(menu).class {
        message.callback_5c = callback;
        message.button_base_60 = button_base;
    }
    let status = cx
        .world
        .resource::<InterfaceManager>()
        .menu_loading_status_008;
    tile_set_numeric_trait(cx, created, TRAIT_USER0, f32::from(u8::from(status != 1)))?;
    cx.host.play_ui_sound(0xB);
    tile_update_scene_graph(cx, created, false)?;
    tile_update_scene_graph(cx, created, false)?;
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    tile_set_numeric_trait(cx, created, TRAIT_VISIBLE, 2.0)?;
    Ok(true)
}

/// IDA: 005BC6B0 MessageMenu_CloseWithSelection: destroyOnClose 2, then at once with no +0x60 node (the first-person HUD's) or a showing Load menu, which is every screen here. 005BC4A0 the destructor hands +0x5C to +0xB4. Completion: 100%.
pub fn message_menu_close(cx: &mut InterfaceCx) -> Result<(), GameError> {
    let Some(tile) = cx.engine.tile_menu_by_id(1001) else {
        return Ok(());
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(());
    };
    tile_set_numeric_trait(cx, tile, TRAIT_DESTROY_ON_CLOSE, 2.0)?;
    let callback = match &cx.engine.menu(menu).class {
        MenuClass::Message(message) => message.callback_5c,
        _ => None,
    };
    cx.world
        .resource_mut::<InterfaceManager>()
        .message_callback_0b4 = callback;
    menu_destroy(cx, menu)
}

/// IDA: MessageMenu (0x64): 005BC450's fields. +0x28..+0x58 are the 13 tiles 005BC510 stores by ID; +0x60 the 0x5C callback's index base (005BC8B0 stores its third argument); +0x5C the callback. Completion: 100% of the fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MessageMenu {
    /// +0x28..+0x58, IDs 1..13.
    pub tiles_28: [Option<TileId>; 13],
    /// +0x5C.
    pub callback_5c: Option<MessageCallback>,
    /// +0x60.
    pub button_base_60: u8,
}

/// IDA: the callbacks 005BC8B0's second argument takes on the paths this port reaches: 005B5D60, 005B5DF0, 005B59B0, 005AE190 and the race menu's 005C9F70, 005C9FC0 and 005C2BA0. Host: an enum for the function pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageCallback {
    MainMenuContinue,
    MainMenuNewGame,
    MainMenuExit,
    LoadgameConfirm,
    /// 005C9F70: the race menu's Reset confirm.
    RaceSexReset,
    /// 005C9FC0: the race menu's Random confirm.
    RaceSexRandom,
    /// 005C2BA0: the race menu's Done confirm.
    RaceSexDone,
}

/// IDA: 005BC890 MessageMenu vt+0x14: the focus box (+0x30, field 3) hidden. Completion: 100%.
/// # Errors
/// A stop in the trait write.
pub fn message_menu_mouseout(
    cx: &mut InterfaceCx,
    menu: MenuId,
    _id: i32,
    _tile: Option<TileId>,
) -> Result<(), GameError> {
    let hide = match &cx.engine.menu(menu).class {
        MenuClass::Message(message) => message.tiles_28[2],
        _ => None,
    };
    if let Some(hide) = hide {
        tile_set_numeric_trait(cx, hide, TRAIT_VISIBLE, 1.0)?;
    }
    Ok(())
}

/// IDA: 005BCDD0 MessageMenu_HandleClick: ID 2 is 005BCCB0 (a scrolled text page, which no prompt here uses); from 4 on, 0057DE50(1), the four active fields cleared, +0xB0 = id + base - 4, 005BC6B0. Completion: 100% of the buttons.
pub fn message_menu_handle_click(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    _tile: Option<TileId>,
) -> Result<(), GameError> {
    if id < 4 {
        return Ok(());
    }
    cx.host.play_ui_sound(1);
    let base = match &cx.engine.menu(menu).class {
        MenuClass::Message(message) => message.button_base_60,
        _ => 0,
    };
    {
        let mut manager = cx.world.resource_mut::<InterfaceManager>();
        manager.active_tile_098 = None;
        manager.active_menu_09c = None;
        manager.unknown_0a0 = 0;
        manager.unknown_0a4 = 0;
        manager.msg_box_button_pressed_0b0 = (id + i32::from(base) - 4) as u8;
    }
    message_menu_close(cx)
}

/// IDA: 005BC770: for a tile with an ID of 4 or more, the focus box (+0x30) takes depth = 00588D90(tile) - the depth of its ancestor below the root - 1, the tile's size and position, and shows; otherwise it hides. Completion: 100%.
pub fn message_menu_mouseover(
    cx: &mut InterfaceCx,
    menu: MenuId,
    _id: i32,
    tile: Option<TileId>,
) -> Result<(), GameError> {
    let MenuClass::Message(message) = &cx.engine.menu(menu).class else {
        return Ok(());
    };
    let Some(focus) = message.tiles_28[2] else {
        return Ok(());
    };
    let target = tile.filter(|&tile| tile_numeric_trait(cx, tile, TRAIT_ID) >= 4.0);
    let Some(target) = target else {
        return tile_set_numeric_trait(cx, focus, TRAIT_VISIBLE, 1.0);
    };
    let depth = depth_to_locus(cx, target);
    let mut top = target;
    while let Some(parent) = cx.engine.tile(top).parent {
        if cx.engine.tile(parent).parent.is_none() {
            break;
        }
        top = parent;
    }
    let value =
        (f64::from(depth) - f64::from(tile_numeric_trait(cx, top, TRAIT_DEPTH)) - 1.0) as f32;
    tile_set_numeric_trait(cx, focus, TRAIT_DEPTH, value)?;
    for trait_id in [TRAIT_WIDTH, TRAIT_HEIGHT, TRAIT_X, TRAIT_Y] {
        let value = tile_numeric_trait(cx, target, trait_id);
        tile_set_numeric_trait(cx, focus, trait_id, value)?;
    }
    tile_set_numeric_trait(cx, focus, TRAIT_VISIBLE, 2.0)
}
