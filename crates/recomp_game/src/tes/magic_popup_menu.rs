//! MagicPopupMenu (1024): 005B3D30 MagicPopupMenu_ctor's fields, 005B3D80 SetField while
//! `Data\Menus\Main\magic_popup_menu.xml` is built, and 005B3F00, which builds it with its rows hidden. Its callers
//! (0057BB20, 0057BBF0, 0057BCC0, which fill the active effects) and its per-frame vt+0x2C (005B4080, the slide animation)
//! aren't ported.

use recomp_core::reader::StreamError;

use super::interface_frame::{menu_destroy, menu_enable};
use super::interface_manager::{InterfaceManager, menu_register_tile, menu_update_cursor_depth};
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::{
    TRAIT_DEPTH, TRAIT_STACKING, TRAIT_STRING, TRAIT_VISIBLE, tile_numeric_trait,
    tile_set_numeric_trait, tile_set_string_trait,
};
use super::tile_xml::tile_read_xml;

/// IDA: MenuID_MagicPopup.
pub const MENU_ID_MAGIC_POPUP: u32 = 1024;
const MAGIC_POPUP_XML: &str = "Data\\Menus\\Main\\magic_popup_menu.xml";

/// IDA: 005B3D30 MagicPopupMenu_ctor (via 00587D20's 1024 case, FormHeap 0x60): Menu_Constructor, vtable, the ten tile
/// pointers +0x28..+0x4C null, +0x58 2, +0x5C GetTickCount, +0x54 and +0x50 0.0 (`fst`, `fstp`). Completion: 100%.
#[derive(Clone, Debug, PartialEq)]
pub struct MagicPopupMenu {
    /// +0x28 `background` (field 1), +0x2C..+0x48 `effect1..8` (fields 11..18), +0x4C `backgroundBottom` (field 21).
    pub tiles_28: [Option<TileId>; 10],
    /// +0x50 endPosX, +0x54 startPosX: the slide's ends (005B4080).
    pub end_pos_x_50: f32,
    pub start_pos_x_54: f32,
    /// +0x58: the slide's state, 2 from the ctor.
    pub animation_state_58: u32,
    /// +0x5C: the ctor's GetTickCount, a host value.
    pub tick_5c: u32,
}

impl MagicPopupMenu {
    /// IDA: 005B3D30's stores, with the host's tick. Completion: 100%.
    #[must_use]
    pub const fn constructed(tick: u32) -> Self {
        Self {
            tiles_28: [None; 10],
            end_pos_x_50: 0.0,
            start_pos_x_54: 0.0,
            animation_state_58: 2,
            tick_5c: tick,
        }
    }

    /// IDA: 005B3D80 MagicPopupMenu_SetField: id 1 to +0x28, 11..18 to +0x2C..+0x48, 21 to +0x4C; 2..10, 19 and 20 store
    /// nothing. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        match id {
            1 => self.tiles_28[0] = Some(tile),
            11..=18 => self.tiles_28[id as usize - 10] = Some(tile),
            21 => self.tiles_28[9] = Some(tile),
            _ => {}
        }
    }

    /// IDA: 005A46B0 (named QuickKeysMenu_HasRequiredTiles; HUDInfo, QuickKeys and this menu share it): +0x28..+0x4C
    /// nonnull. Completion: 100%.
    #[must_use]
    pub fn has_required_tiles(&self) -> bool {
        self.tiles_28.iter().all(Option::is_some)
    }
}

// IDA: N/A the menu's MagicPopupMenu part.
fn magic_popup<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a MagicPopupMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::MagicPopup(popup) => popup,
        _ => unreachable!("a MagicPopupMenu"),
    }
}

/// IDA: 005B3F00 MagicPopupMenu_Create: an existing 1024 menu deleted (TileMenu vt+0, ported as `map_menu_create` does
/// it); the depth (00588890 → 00584980); Tile_ReadXML(magic_popup_menu.xml) under the root; its containing menu, else
/// null; a menu that isn't 1024 is deleted when it has a tile, and null; 00584880 registers the tile; without the ten
/// tiles (005A46B0) the warning "Magic Menu Creation Failed..." and null (the menu stays). A stacking type 6006 or 102
/// takes the depth. +0x4C's string "" (00A2F7EC) and visible 1, then +0x2C..+0x48 visible 1 in order; Menu_Enable(0); the
/// root answers. Completion: 100%.
/// # Errors
/// A stop in the XML or the menu calls.
pub fn magic_popup_menu_create(cx: &mut InterfaceCx) -> Result<Option<TileId>, StreamError> {
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_MAGIC_POPUP)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, MAGIC_POPUP_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != MENU_ID_MAGIC_POPUP {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(None);
    }
    menu_register_tile(cx, menu, Some(created))?;
    if !magic_popup(cx, menu).has_required_tiles() {
        cx.host
            .warning("Magic Menu Creation Failed... Are your menu and art resources up to date?");
        return Ok(None);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let tiles = magic_popup(cx, menu)
        .tiles_28
        .map(|tile| tile.expect("the ten tiles"));
    tile_set_string_trait(cx, tiles[9], TRAIT_STRING, b"")?;
    tile_set_numeric_trait(cx, tiles[9], TRAIT_VISIBLE, 1.0)?;
    for &effect in &tiles[1..9] {
        tile_set_numeric_trait(cx, effect, TRAIT_VISIBLE, 1.0)?;
    }
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(Some(created))
}
