//! LoadgameMenu (1038): its creation, its save rows and the chosen save's details.
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::main_menu::main_menu_refresh_continue;
use super::message_menu::{MessageCallback, show_message, yes_no};
use crate::tes::game_settings::text_at_or_empty;
use crate::tes::interface_frame::*;
use crate::tes::interface_manager::{
    InterfaceManager, menu_register_tile, menu_update_cursor_depth,
};
use crate::tes::interface_scene::{image_set_preview, tile_cumulative_position};
use crate::tes::save_files::SaveFile;
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileId, depth_to_locus};
use crate::tes::tile_values::*;
use crate::tes::tile_xml::{tile_build_template, tile_read_xml};

/// IDA: 005AEA60 (hideReturn): an old Load menu deleted; 00584980; Tile_ReadXML(load_menu.xml); the class test; 00584880; the depth; the flag hides +0x28; 0045D450 unless the MainMenu is there, which keeps the list; 005AE6D0 for each save, or once with no file; 0058FBA0; 00585190(0); 0057F9F0(first row, mouseover, 0). Completion: 100% of the order. 0045E6A0 ConvertXboxSaves is carried: it scans for console saves, and its answer here is taken as false.
pub fn load_menu_create(
    cx: &mut InterfaceCx,
    hide_return: bool,
) -> Result<Option<TileId>, GameError> {
    if let Some(old) = cx.engine.tile_menu_by_id(1038)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let Some(created) = tile_read_xml(cx, root, "Data\\Menus\\Options\\load_menu.xml")? else {
        return Ok(None);
    };
    let Some(menu) = cx.engine.containing_menu(created) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != 1038 {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(None);
    }
    menu_register_tile(cx, menu, Some(created))?;
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    if hide_return {
        let first = match &cx.engine.menu(menu).class {
            MenuClass::Load(load) => load.field_id1_28,
            _ => None,
        };
        if let Some(first) = first {
            tile_set_numeric_trait(cx, first, TRAIT_VISIBLE, 1.0)?;
        }
    }
    let saves = cx.host.save_file_list();
    if let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class {
        load.saves_54.clone_from(&saves);
    }
    let mut first_row = None;
    if saves.is_empty() {
        first_row = load_menu_add_row(cx, menu, 0, None)?;
    }
    for (index, save) in saves.iter().enumerate() {
        let row = load_menu_add_row(cx, menu, index as i32, Some(save))?;
        if first_row.is_none() {
            first_row = row;
        }
    }
    tile_update_scene_graph(cx, created, false)?;
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    interface_dispatch_pointer_event(cx, first_row, TRAIT_MOUSEOVER)?;
    Ok(Some(created))
}

/// IDA: 005AE6D0 (buffer, index, file, count): a row from `save_game_template` in the list pane (+0x48). A file's caption is 00464060's "name\ntime" or "Save n - location\ntime"; no file gives the B38700 setting and hides the picture. user0 = index; the name with spaces as '_', 255 bytes; user3 = caption; id = index + 101; xdefault when it matches +0x5C. Completion: 100% of the order. `tile_build_template` says why the exe's "save_game_template" finds the XML's load_game_template.
fn load_menu_add_row(
    cx: &mut InterfaceCx,
    menu: MenuId,
    index: i32,
    save: Option<&SaveFile>,
) -> Result<Option<TileId>, GameError> {
    let MenuClass::Load(load) = &cx.engine.menu(menu).class else {
        return Ok(None);
    };
    let (pane, picture, details) = (load.list_pane_48, load.field_id6_40, load.field_id7_44);
    let Some(pane) = pane else {
        return Ok(None);
    };
    let row = tile_build_template(cx, menu, pane, b"save_game_template")?;
    let caption = if let Some(save) = save {
        save.caption(
            &text_at_or_empty(cx.world, 0x00B3_86F0),
            &text_at_or_empty(cx.world, 0x00B3_8710),
            &text_at_or_empty(cx.world, 0x00B3_8718),
            &text_at_or_empty(cx.world, 0x00B3_86F8),
            &text_at_or_empty(cx.world, 0x00B3_8720),
        )
    } else {
        for hidden in [picture].into_iter().flatten() {
            tile_set_numeric_trait(cx, hidden, TRAIT_VISIBLE, 1.0)?;
        }
        if let Some(details) = details {
            tile_set_string_trait(cx, details, TRAIT_STRING, b"")?;
        }
        text_at_or_empty(cx.world, 0x00B3_8700)
    };
    let Some(row) = row else {
        return Ok(None);
    };
    tile_set_numeric_trait(cx, row, TRAIT_USER0, index as f32)?;
    let name: Vec<u8> = caption
        .iter()
        .take(255)
        .map(|&byte| if byte == b' ' { b'_' } else { byte })
        .collect();
    cx.engine.tile_mut(row).name_08 = name;
    tile_set_string_trait(cx, row, TRAIT_USER3, &caption)?;
    tile_set_numeric_trait(cx, row, TRAIT_ID, (index + 101) as f32)?;
    Ok(Some(row))
}

/// IDA: 005AE240(index): the picture's texture from 004610F0 (591A80 twice), the five lines "%s\n%s\n%s\n%s\n%s" in +0x44, the picture's user0 = the screenshot's height; a missing screenshot hides it and shows B38750. Index -1 hides. Completion: 100%.
pub(super) fn load_menu_show_details(
    cx: &mut InterfaceCx,
    menu: MenuId,
    index: i32,
) -> Result<(), GameError> {
    let MenuClass::Load(load) = &cx.engine.menu(menu).class else {
        return Ok(());
    };
    let (picture, details) = (load.field_id6_40, load.field_id7_44);
    let save = usize::try_from(index)
        .ok()
        .and_then(|index| load.saves_54.get(index).cloned());
    let (Some(picture), Some(details)) = (picture, details) else {
        return Ok(());
    };
    let Some(save) = save else {
        if index == -1 {
            tile_set_numeric_trait(cx, picture, TRAIT_VISIBLE, 1.0)?;
            tile_set_string_trait(cx, details, TRAIT_STRING, b"")?;
        }
        return Ok(());
    };
    image_set_preview(cx, picture, None)?;
    tile_set_numeric_trait(cx, picture, TRAIT_VISIBLE, 2.0)?;
    let text = save.detail_text(
        &text_at_or_empty(cx.world, 0x00B3_8720),
        &text_at_or_empty(cx.world, 0x00B3_8730),
        &text_at_or_empty(cx.world, 0x00B3_8728),
    );
    tile_set_string_trait(cx, details, TRAIT_STRING, &text)?;
    tile_set_numeric_trait(
        cx,
        picture,
        TRAIT_USER0,
        save.preview.as_ref().map_or(0, |preview| preview.height) as f32,
    )?;
    let preview = save.preview.clone();
    image_set_preview(cx, picture, preview.clone())?;
    if preview.is_none() {
        tile_set_numeric_trait(cx, picture, TRAIT_VISIBLE, 1.0)?;
        let text = text_at_or_empty(cx.world, 0x00B3_8750);
        tile_set_string_trait(cx, details, TRAIT_STRING, &text)?;
    }
    Ok(())
}

/// IDA: LoadgameMenu (0x68): 005AE450's fields. 005D2C60 stores IDs 1..7 at +0x28, +0x2C, +0x34, +0x38, +0x3C, +0x40, +0x44 and ID 9 at +0x48 (+0x30 isn't an ID: 005AE560 keeps the row in focus there). +0x4C the save chosen, +0x50 005AE140's store, +0x54 the save list, +0x58 the row of a pending confirm, +0x5C the name 005AE6D0 compares, +0x64 the confirm byte. Completion: 100% of the fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadgameMenu {
    /// +0x28.
    pub field_id1_28: Option<TileId>,
    /// +0x2C: 005AE560's focus box.
    pub focus_box_2c: Option<TileId>,
    /// +0x30: the row in focus.
    pub focused_row_30: Option<TileId>,
    /// +0x34.
    pub field_id3_34: Option<TileId>,
    /// +0x38.
    pub field_id4_38: Option<TileId>,
    /// +0x3C.
    pub field_id5_3c: Option<TileId>,
    /// +0x40: 005AEED0 hides it on Return.
    pub field_id6_40: Option<TileId>,
    /// +0x44.
    pub field_id7_44: Option<TileId>,
    /// +0x48: the list pane 005AE6D0 fills.
    pub list_pane_48: Option<TileId>,
    /// +0x4C: which save, an index in the list.
    pub chosen_save_4c: Option<usize>,
    /// The list +0x54 holds: g_saveLoadGame's, kept with the class here.
    pub saves_54: Vec<SaveFile>,
    /// +0x50.
    pub scroll_50: f32,
    /// +0x58.
    pub confirm_row_58: Option<TileId>,
    /// +0x64: 005AEED0 ignores clicks while it's set.
    pub confirmation_pending_64: u8,
}

/// IDA: 005AE220 LoadgameMenu vt+0x14: the focus box (+0x2C) hidden. Completion: 100%.
/// # Errors
/// A stop in the trait write.
pub fn load_menu_mouseout(
    cx: &mut InterfaceCx,
    menu: MenuId,
    _id: i32,
    _tile: Option<TileId>,
) -> Result<(), GameError> {
    let hide = match &cx.engine.menu(menu).class {
        MenuClass::Load(load) => load.focus_box_2c,
        _ => None,
    };
    if let Some(hide) = hide {
        tile_set_numeric_trait(cx, hide, TRAIT_VISIBLE, 1.0)?;
    }
    Ok(())
}

/// IDA: 005AE560 the row's hover: below ID 101 the focus box hides. From 101 on, 0057DE50(4), +0x30 = 0, the box's depth = 00588D90(tile) - user15, its size is the row's less 2, its place the row's cumulative place, it shows, +0x30 = the row, and 005AE240(user0). Completion: 100%.
pub fn load_menu_mouseover(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: Option<TileId>,
) -> Result<(), GameError> {
    let Some(tile) = tile else {
        return Ok(());
    };
    let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class else {
        return Ok(());
    };
    let focus = load.focus_box_2c;
    if id < 101 {
        if let Some(focus) = focus {
            tile_set_numeric_trait(cx, focus, TRAIT_VISIBLE, 1.0)?;
        }
        return Ok(());
    }
    load.focused_row_30 = None;
    cx.host.play_ui_sound(4);
    let Some(focus) = focus else {
        return Ok(());
    };
    let depth = (f64::from(depth_to_locus(cx, tile))
        - f64::from(tile_numeric_trait(cx, focus, TRAIT_USER15))) as f32;
    tile_set_numeric_trait(cx, focus, TRAIT_DEPTH, depth)?;
    let width = tile_numeric_trait(cx, tile, TRAIT_WIDTH) - 2.0;
    tile_set_numeric_trait(cx, focus, TRAIT_WIDTH, width)?;
    let height = tile_numeric_trait(cx, tile, TRAIT_HEIGHT) - 2.0;
    tile_set_numeric_trait(cx, focus, TRAIT_HEIGHT, height)?;
    let [x, y] = tile_cumulative_position(cx, tile);
    tile_set_numeric_trait(cx, focus, TRAIT_X, x)?;
    tile_set_numeric_trait(cx, focus, TRAIT_Y, y)?;
    tile_set_numeric_trait(cx, focus, TRAIT_VISIBLE, 2.0)?;
    if let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class {
        load.focused_row_30 = Some(tile);
    }
    let index = x87::ftol(tile_numeric_trait(cx, tile, TRAIT_USER0));
    load_menu_show_details(cx, menu, index)
}

/// IDA: 005AEED0: ignored while +0x64 is set. ID 1 is Return: the picture hides, 005AE080 (the picture cleared, destroyOnClose 2, 00584740), 005BDA20, 005B5B70(any saves), and the list stays while the MainMenu does. From 101: the row's user0 picks the save, kept in +0x4C; shift asks to delete (+0x58 the row; the delete is a later phase's), else the prompt, with B38780 when a game runs and B38768 otherwise; +0x64 = 1. Completion: 100% of what a main menu reaches. 005D2C60's slider is in `menu_handle_press`.
pub fn load_menu_handle_click(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: Option<TileId>,
) -> Result<(), GameError> {
    let MenuClass::Load(load) = &cx.engine.menu(menu).class else {
        return Ok(());
    };
    if load.confirmation_pending_64 != 0 {
        return Ok(());
    }
    let picture = load.field_id6_40;
    let any_saves = !load.saves_54.is_empty();
    if id == 1 {
        if let Some(picture) = picture {
            tile_set_numeric_trait(cx, picture, TRAIT_VISIBLE, 1.0)?;
            image_set_preview(cx, picture, None)?;
        }
        if let Some(own) = cx.engine.menu(menu).tile_04 {
            tile_set_numeric_trait(cx, own, TRAIT_DESTROY_ON_CLOSE, 2.0)?;
        }
        let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
        menu_begin_fade_out(cx, menu, frame_time)?;
        main_menu_refresh_continue(cx, any_saves)?;
    } else if id >= 101
        && let Some(row) = tile
    {
        let index = x87::ftol(tile_numeric_trait(cx, row, TRAIT_USER0));
        let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class else {
            return Ok(());
        };
        let Some(index) = usize::try_from(index)
            .ok()
            .filter(|&index| index < load.saves_54.len())
        else {
            return Ok(());
        };
        load.chosen_save_4c = Some(index);
        let shift = cx.world.resource::<InterfaceManager>().input_modifier_flags & 4 != 0;
        if shift {
            if let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class {
                load.confirm_row_58 = Some(row);
            }
            cx.host
                .warning("005AECA0: deleting a save is carried to its phase");
            return Ok(());
        }
        let text = text_at_or_empty(cx.world, 0x00B3_8768);
        let (yes, no) = yes_no(cx);
        show_message(
            cx,
            &text,
            Some(MessageCallback::LoadgameConfirm),
            1,
            &[yes, no],
        )?;
        if let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class {
            load.confirmation_pending_64 = 1;
        }
    }
    Ok(())
}

/// IDA: 005AE155..005AE186 what 005AE140 stores at +0x50. Completion: 100%.
#[must_use]
pub fn load_press_scroll(virtual_height: f32, pointer: f32, marker_y: f32) -> f32 {
    let cursor = x87::ftol(virtual_height - (virtual_height * 0.5 + pointer));
    // 5AE181 `fisub`: the int isn't rounded first.
    (f64::from(marker_y) - f64::from(cursor)) as f32
}

/// IDA: 005AE140 LoadgameMenu vt+0x08 (005821F0's press on the active tile): +0x50 = cumulative y of ID 4 - ftol(h_v - (h_v / 2 + cursor z)), a float. Completion: 100%.
/// # Errors
/// None; the slot's signature.
pub fn load_menu_press(
    cx: &mut InterfaceCx,
    menu: MenuId,
    _id: i32,
    _tile: TileId,
) -> Result<(), GameError> {
    let MenuClass::Load(load) = &cx.engine.menu(menu).class else {
        return Ok(());
    };
    let Some(marker) = load.field_id4_38 else {
        return Ok(());
    };
    let [_, y] = tile_cumulative_position(cx, marker);
    let pointer = cx.world.resource::<InterfaceManager>().cursor_state_020[2];
    let height = crate::tes::interface_manager::ui_virtual_height(cx.settings);
    let scroll = load_press_scroll(height, pointer, y);
    if let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class {
        load.scroll_50 = scroll;
    }
    Ok(())
}

/// IDA: 005AEDF0 LoadgameMenu_HandleAction, vt+0x38(action, 1.0): only action 11 (the controller's delete) with this menu the
/// active top one acts; anything else returns false. Completion: the decline; action 11 is a stop.
/// TODO: [ ] action 11 (no key sends it).
/// # Errors
/// Action 11.
pub fn load_menu_handle_action(
    _cx: &mut InterfaceCx,
    _menu: MenuId,
    action: i32,
    _scale: f32,
) -> Result<bool, GameError> {
    if action == 11 {
        return Err(GameError::unclassified(
            0x005AEE0D,
            "005AEE0D: LoadgameMenu's delete action",
        ));
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 005AE17C..005AE186: `fisub` from the float y, stored as a float: only the cursor is truncated.
    #[test]
    fn the_press_keeps_the_markers_fraction() {
        assert_eq!(load_press_scroll(1000.0, 344.5, 100.25), 100.25 - 155.0);
    }
}
