//! RechargeMenu (1049): 005CE680 RechargeMenu_ctor's fields, 005CE730 SetField while `Data\Menus\recharge_menu.xml` is
//! built, and 005CE840, which builds it with its scroll reset. Its callers (005CFB50 with a soul gem's entry, 005CFCE0),
//! which fill the gem and the list, and its click, mouseover and per-frame virtuals (005CFD90, 005CF770, 005CF980,
//! 005CEB70, 005CEA80, 005CEB50) aren't ported.

use recomp_core::reader::StreamError;

use super::interface_frame::{menu_destroy, menu_enable};
use super::interface_manager::{InterfaceManager, menu_register_tile, menu_update_cursor_depth};
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::{
    TRAIT_DEPTH, TRAIT_STACKING, TRAIT_USER9, tile_numeric_trait, tile_set_numeric_trait,
};
use super::tile_xml::tile_read_xml;

/// IDA: MenuID_Recharge.
pub const MENU_ID_RECHARGE: u32 = 1049;
const RECHARGE_XML: &str = "Data\\Menus\\recharge_menu.xml";

/// IDA: 005CE680 RechargeMenu_ctor (via 00587D20's 1049 case, FormHeap 0x54): Menu_Constructor, +0x28..+0x3C, +0x44,
/// +0x48 and +0x4C zero, vtable, the byte +0x50 0xFF. +0x40 isn't stored (FormHeap's bytes; nothing ported reads it).
/// Completion: 100% of the stores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RechargeMenu {
    /// +0x28.. fields 1, 3, 4, 5, 6: `recharge_FocusBox`, `recharge_button_exit`, `recharge_scroll_bar`, its marker (by
    /// user6), `recharge_list_pane`.
    pub tiles_28: [Option<TileId>; 5],
    /// +0x3C, +0x48: zeroed by the ctor, roles not read.
    pub dword_3c: u32,
    pub dword_48: u32,
    /// +0x44: the soul gem's inventory entry, stored by 005CFB50 (0 until then).
    pub soul_gem_entry_44: u32,
    /// +0x4C: the soul gem's charge, stored by 005CFB50 (004BC070 of 00484BF0).
    pub soul_gem_charge_4c: u32,
    /// +0x50: 0xFF from the ctor.
    pub byte_50: u8,
}

impl Default for RechargeMenu {
    fn default() -> Self {
        Self {
            tiles_28: [None; 5],
            dword_3c: 0,
            dword_48: 0,
            soul_gem_entry_44: 0,
            soul_gem_charge_4c: 0,
            byte_50: 0xFF,
        }
    }
}

/// IDA: 005CE730's ids, in +0x28..+0x38 order.
const FIELD_IDS: [i32; 5] = [1, 3, 4, 5, 6];

impl RechargeMenu {
    /// IDA: 005CE730 RechargeMenu_SetField: ids 1, 3, 4, 5, 6 to +0x28..+0x38. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        if let Some(slot) = FIELD_IDS.iter().position(|&known| known == id) {
            self.tiles_28[slot] = Some(tile);
        }
    }

    /// IDA: 005CE8E1..005CE8FD: +0x28..+0x38 all nonnull. Completion: 100%.
    #[must_use]
    pub fn has_required_tiles(&self) -> bool {
        self.tiles_28.iter().all(Option::is_some)
    }

    /// IDA: N/A the tile at +0x28 + 4 * slot.
    #[must_use]
    pub fn field(&self, id: i32) -> Option<TileId> {
        let slot = FIELD_IDS.iter().position(|&known| known == id)?;
        self.tiles_28[slot]
    }
}

// IDA: N/A the menu's RechargeMenu part.
fn recharge<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a RechargeMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::Recharge(recharge) => recharge,
        _ => unreachable!("a RechargeMenu"),
    }
}

/// IDA: 005CE840 RechargeMenu_Create: an existing 1049 menu deleted (TileMenu vt+0, ported as `map_menu_create` does
/// it); the depth (00588890 → 00584980); Tile_ReadXML(recharge_menu.xml) under the root; its containing menu, else null;
/// a menu that isn't 1049 is deleted when it has a tile, and null; 00584880 registers the tile; without the five tiles
/// the warning and null (the menu stays). A stacking type 6006 or 102 takes the depth. Field 4's user9 -999999 then 0;
/// Menu_Enable(0); the root answers. Completion: 100%.
/// # Errors
/// A stop in the XML or the menu calls.
pub fn recharge_menu_create(cx: &mut InterfaceCx) -> Result<Option<TileId>, StreamError> {
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_RECHARGE)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, RECHARGE_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != MENU_ID_RECHARGE {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(None);
    }
    menu_register_tile(cx, menu, Some(created))?;
    if !recharge(cx, menu).has_required_tiles() {
        cx.host.warning(
            "Recharge Menu Creation Failed... Are your menu and art resources up to date?",
        );
        return Ok(None);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let scroll = recharge(cx, menu).field(4).expect("the five tiles");
    tile_set_numeric_trait(cx, scroll, TRAIT_USER9, -999_999.0)?;
    tile_set_numeric_trait(cx, scroll, TRAIT_USER9, 0.0)?;
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(Some(created))
}
