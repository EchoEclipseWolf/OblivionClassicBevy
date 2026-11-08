//! SaveMenu (1039): 005D3290 SaveMenu_ctor's fields, 005D2C60 SaveLoadMenu_SetField (the Load menu's) while
//! `Data\Menus\Options\save_menu.xml` is built, and 005D39A0, which builds it: a "New Save" row, then one row per save
//! (005D3650). Its callers (the Pause menu's click 005BDE70, the Options menu's 005BD880) and its click (005D3D20), hover
//! (005D3470), drag and wheel virtuals aren't ported, so nothing saves.

use bevy_ecs::prelude::*;
use recomp_core::crt::stricmp;
use recomp_core::reader::StreamError;

use super::game_settings::GameSettingCollection;
use super::interface_frame::{TRAIT_XDEFAULT, menu_destroy, menu_enable};
use super::interface_manager::{InterfaceManager, menu_register_tile, menu_update_cursor_depth};
use super::save_files::SaveFile;
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::{
    TRAIT_DEPTH, TRAIT_ID, TRAIT_STACKING, TRAIT_USER0, TRAIT_USER3, tile_numeric_trait,
    tile_set_numeric_trait, tile_set_string_trait,
};
use super::tile_xml::{tile_build_template, tile_read_xml};

/// IDA: MenuID_Save.
pub const MENU_ID_SAVE: u32 = 1039;
const SAVE_XML: &str = "Data\\Menus\\Options\\save_menu.xml";
/// IDA: 00B38708 sMenuDisplayNewSave; the caption words 00B386F0 sMenuDisplaySave, 00B38710, 00B38718, 00B386F8
/// sMenuDisplayPlayTime and 00B38720, as `load_menu_add_row` passes them.
const S_NEW_SAVE_B38708: u32 = 0x00B3_8708;
const CAPTION_WORDS: [u32; 5] = [
    0x00B3_86F0,
    0x00B3_8710,
    0x00B3_8718,
    0x00B3_86F8,
    0x00B3_8720,
];

/// IDA: 00B3B71C (00B3B444 + 0x2D8): 005D3650 clears it at row 1 and counts the rows whose caption has a name of its own.
/// The Load menu's 005AE6D0 counts into 00B3B3F0 instead. Its readers aren't ported. Completion: 100% of the stores.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveMenuStatics {
    pub named_saves_b3b71c: u32,
}

/// IDA: 005D3290 SaveMenu_ctor (via 00587D20's 1039 case, FormHeap 0x60): Menu_Constructor, vtable, the BSStringT +0x50
/// cleared (twice, around a FormHeap_Free), +0x28, +0x2C, +0x30, +0x34, +0x38, +0x4C, +0x58 zero, the byte +0x5C 0.
/// +0x3C..+0x48 aren't stored (FormHeap's bytes until SetField fills them). Completion: 100% of the stores.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SaveMenu {
    /// +0x28.. 005D2C60's ids 1, 2 at +0x28, +0x2C; 3..7 at +0x34..+0x44; 9 at +0x48: `save_button_return`,
    /// `save_FocusBox`, `save_scroll_bar`, its marker, `save_list`, `save_Picture` (the later of the two id 6 tiles),
    /// `save_Text`, `save_list_pane`. +0x30 isn't a field (the Load menu's focused row).
    pub field_id1_28: Option<TileId>,
    pub field_id2_2c: Option<TileId>,
    pub focused_row_30: Option<TileId>,
    pub field_id3_34: Option<TileId>,
    pub field_id4_38: Option<TileId>,
    pub field_id5_3c: Option<TileId>,
    pub field_id6_40: Option<TileId>,
    pub field_id7_44: Option<TileId>,
    pub list_pane_48: Option<TileId>,
    /// +0x4C: g_saveLoadGame's save list at creation.
    pub saves_4c: Vec<SaveFile>,
    /// +0x50: the name 005D3650 compares each row's caption with (`_stricmp`) for the row's xdefault; none from the ctor,
    /// and nothing ported sets it.
    pub default_name_50: Option<Vec<u8>>,
    /// +0x58, +0x5C: zeroed by the ctor, read by code not ported.
    pub dword_58: u32,
    pub byte_5c: u8,
}

impl SaveMenu {
    /// IDA: 005D2C60 SaveLoadMenu_SetField (shared with LoadgameMenu): 1 +0x28, 2 +0x2C, 3..7 +0x34..+0x44, 9 +0x48.
    /// Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        let slot = match id {
            1 => &mut self.field_id1_28,
            2 => &mut self.field_id2_2c,
            3 => &mut self.field_id3_34,
            4 => &mut self.field_id4_38,
            5 => &mut self.field_id5_3c,
            6 => &mut self.field_id6_40,
            7 => &mut self.field_id7_44,
            9 => &mut self.list_pane_48,
            _ => return,
        };
        *slot = Some(tile);
    }
}

// IDA: N/A the menu's SaveMenu part.
fn save_menu<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a SaveMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::Save(save) => save,
        _ => unreachable!("a SaveMenu"),
    }
}

// IDA: N/A a static string Setting's text by address, as `menus::game_setting_text` reads it.
fn setting_text(cx: &InterfaceCx, address: u32) -> Vec<u8> {
    cx.world
        .get_resource::<GameSettingCollection>()
        .and_then(|settings| settings.text_at(address))
        .unwrap_or_default()
}

/// IDA: 005D3650 SaveMenu_AddRow(buffer, index, file, count): row 1 clears 00B3B71C; a `save_game_template` row under
/// +0x48 (00585410). With a file, 00464060's caption into the buffer: a name of its own gives "%s\n%s: %s" and counts in
/// 00B3B71C, an empty one "%s %i - %s\n%s: %s" (`SaveFile::caption`); without one the buffer is the caller's. With a row:
/// user0 the index (`fild`), its name the buffer with ' ' as '_' (at most 255 bytes), user3 the buffer, id index + 101;
/// when +0x50 matches the buffer (`_stricmp`; a null either side never matches), InterfaceManager +0x8C is stepped and
/// the row's xdefault takes it (`fild`, with 2^32 added below 0). `count` isn't read. Completion: 100%.
/// # Errors
/// A null list pane (+0x48), which the original passes to 00585410; a stop in the template or the traits.
pub fn save_menu_add_row(
    cx: &mut InterfaceCx,
    menu: MenuId,
    buffer: &[u8],
    index: i32,
    save: Option<&SaveFile>,
) -> Result<Option<TileId>, StreamError> {
    if index == 1 {
        cx.world
            .get_resource_or_init::<SaveMenuStatics>()
            .named_saves_b3b71c = 0;
    }
    let pane = save_menu(cx, menu).list_pane_48.ok_or(StreamError(
        "005D36AF: SaveMenu +0x48 is null, and the original builds its rows under it",
    ))?;
    let row = tile_build_template(cx, menu, pane, b"save_game_template")?;
    let caption = match save {
        Some(save) => {
            let words = CAPTION_WORDS.map(|address| setting_text(cx, address));
            if save.caption_is_named() {
                cx.world
                    .get_resource_or_init::<SaveMenuStatics>()
                    .named_saves_b3b71c += 1;
            }
            save.caption(&words[0], &words[1], &words[2], &words[3], &words[4])
        }
        None => buffer.to_vec(),
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
    let matches = save_menu(cx, menu)
        .default_name_50
        .as_deref()
        .is_some_and(|name| stricmp(name, &caption) == 0);
    if matches {
        let priority = {
            let mut manager = cx.world.resource_mut::<InterfaceManager>();
            manager.default_selection_priority_08c =
                manager.default_selection_priority_08c.wrapping_add(1);
            manager.default_selection_priority_08c
        };
        // 005D3855..005D386A: fild of the dword, with 2^32 added when its sign bit is set.
        tile_set_numeric_trait(cx, row, TRAIT_XDEFAULT, f64::from(priority) as f32)?;
    }
    Ok(Some(row))
}

/// IDA: 005D39A0 SaveMenu_Create: an existing 1039 menu deleted (TileMenu vt+0, ported as `map_menu_create` does it);
/// the depth (00588890 → 00584980); Tile_ReadXML(save_menu.xml) under the root; its containing menu, else null; a menu
/// that isn't 1039 is deleted when it has a tile, and null; 00584880 registers the tile. No field is checked. A stacking
/// type 6006 or 102 takes the depth. 0045E6A0 ConvertXboxSaves (carried, as in `load_menu_create`: its answer is taken
/// as none); 005D3650(sMenuDisplayNewSave's text, 0, null, 0); 0045D450 (the host's save list) into +0x4C; then
/// 005D3650(buffer, n, save, count) for each save from 1. Menu_Enable(0); the root answers. Completion: 100% of the
/// order, with 0045E6A0 carried.
/// # Errors
/// A stop in the XML, the rows or the menu calls.
pub fn save_menu_create(cx: &mut InterfaceCx) -> Result<Option<TileId>, StreamError> {
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_SAVE)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, SAVE_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != MENU_ID_SAVE {
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
    let new_save = setting_text(cx, S_NEW_SAVE_B38708);
    save_menu_add_row(cx, menu, &new_save, 0, None)?;
    let saves = cx.host.save_file_list();
    if let MenuClass::Save(state) = &mut cx.engine.menu_mut(menu).class {
        state.saves_4c.clone_from(&saves);
    }
    for (index, save) in saves.iter().enumerate() {
        save_menu_add_row(cx, menu, &[], index as i32 + 1, Some(save))?;
    }
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(Some(created))
}
