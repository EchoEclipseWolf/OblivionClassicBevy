//! QuantityMenu (1016): 005C04F0 QuantityMenu_ctor's fields, 005C0530 SetField while `Data\Menus\quantity_menu.xml` is
//! built, and 005C05D0, which builds it for a count. Its callers (the Container menu 0059A400, the Inventory menu
//! 005ABBC0, 004B28E0) and its click (005C0820, which answers the Container or Inventory menu's vt+0xC or splits a
//! world reference's stack), drag (005C09A0, 005C0A00) and key (005C0AC0) virtuals aren't ported; the fields' roles are
//! read from them.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use crate::tes::interface_frame::{menu_destroy, menu_enable};
use crate::tes::interface_manager::{
    InterfaceManager, menu_register_tile, menu_update_cursor_depth,
};
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use crate::tes::tile_values::{
    TRAIT_DEPTH, TRAIT_STACKING, TRAIT_USER1, TRAIT_USER2, TRAIT_USER4, TRAIT_USER5,
    tile_numeric_trait, tile_set_numeric_trait,
};
use crate::tes::tile_xml::tile_read_xml;

/// IDA: MenuID_Quantity.
pub const MENU_ID_QUANTITY: u32 = 1016;
const QUANTITY_XML: &str = "Data\\Menus\\quantity_menu.xml";

/// IDA: 005C04F0 QuantityMenu_ctor (via 00587D20's 1016 case, FormHeap 0x58): Menu_Constructor, +0x54 0.0, the six tile
/// pointers +0x28..+0x3C null, +0x50 null, vtable, +0x40 1. +0x44, +0x48 and +0x4C aren't stored (FormHeap's bytes until
/// 005C05D0 sets them). Completion: 100%.
#[derive(Clone, Debug, PartialEq)]
pub struct QuantityMenu {
    /// +0x28.. fields 1, 2, 3, 4, 6, 7: `quantity_background`, `quantity_display_text`, `quantity_scroll`, its marker
    /// (by the scroll prefab's user6), `quantity_button_okay`, `quantity_button_cancel`.
    pub tiles_28: [Option<TileId>; 6],
    /// +0x40: the most that can be chosen.
    pub max_count_40: i32,
    /// +0x44: the click id 005C0820 hands back to the Container or Inventory menu's vt+0xC (`None` until set).
    pub return_click_id_44: Option<i32>,
    /// +0x48: the count, a copy of +0x40 at creation.
    pub count_48: Option<i32>,
    /// +0x4C: the clicked tile 005C0820 hands back with +0x44.
    pub return_click_tile_4c: Option<TileId>,
    /// +0x50: a world reference whose stack 005C0820 splits instead (004B28E0's), null otherwise.
    pub stack_reference_50: Option<Entity>,
    /// +0x54: the marker's drag offset (005C09A0).
    pub drag_offset_54: f32,
}

impl Default for QuantityMenu {
    fn default() -> Self {
        Self {
            tiles_28: [None; 6],
            max_count_40: 1,
            return_click_id_44: None,
            count_48: None,
            return_click_tile_4c: None,
            stack_reference_50: None,
            drag_offset_54: 0.0,
        }
    }
}

/// IDA: 005C0530's ids, in +0x28..+0x3C order.
const FIELD_IDS: [i32; 6] = [1, 2, 3, 4, 6, 7];

impl QuantityMenu {
    /// IDA: 005C0530 QuantityMenu_SetField: ids 1, 2, 3, 4, 6, 7 to +0x28..+0x3C. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        if let Some(slot) = FIELD_IDS.iter().position(|&known| known == id) {
            self.tiles_28[slot] = Some(tile);
        }
    }

    /// IDA: 005C0674..005C0696: +0x2C, +0x28, +0x30, +0x34, +0x38, +0x3C all nonnull. Completion: 100%.
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

// IDA: N/A the menu's QuantityMenu part.
fn quantity<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a QuantityMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::Quantity(quantity) => quantity,
        _ => unreachable!("a QuantityMenu"),
    }
}

/// IDA: 005C05D0 QuantityMenu_Create(unused, clickId, clickTile, count, stackReference): the first argument (the callers'
/// address of a result global) isn't read. An existing 1016 menu deleted (TileMenu vt+0, ported as `map_menu_create`
/// does it); the depth (00588890 → 00584980); Tile_ReadXML(quantity_menu.xml) under the root; its containing menu, else
/// false; a menu that isn't 1016 is deleted when it has a tile, and false; 00584880 registers the tile; without the six
/// tiles the warning and false (the menu stays). A stacking type 6006 or 102 takes the depth. +0x44 the click id, +0x40
/// and +0x48 the count, +0x4C the tile, +0x50 the reference; field 3's user1 1, user2 the count (`fild`, stored as a
/// float), user4 count / 4 (`cdq; and edx,3; sar 2`, toward zero) or 1 when that isn't above 1, user5 the count then 0.
/// Menu_Enable(0), true. Completion: 100%.
/// # Errors
/// A stop in the XML or the menu calls.
pub fn quantity_menu_create(
    cx: &mut InterfaceCx,
    click_id: i32,
    click_tile: Option<TileId>,
    count: i32,
    stack_reference: Option<Entity>,
) -> Result<bool, GameError> {
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_QUANTITY)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, QUANTITY_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(false);
    };
    if cx.engine.menu(menu).id() != MENU_ID_QUANTITY {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(false);
    }
    menu_register_tile(cx, menu, Some(created))?;
    if !quantity(cx, menu).has_required_tiles() {
        cx.host.warning(
            "Quantity Menu Creation Failed... Are your menu and art resources up to date?",
        );
        return Ok(false);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let scroll = {
        let MenuClass::Quantity(state) = &mut cx.engine.menu_mut(menu).class else {
            unreachable!("a QuantityMenu")
        };
        state.return_click_id_44 = Some(click_id);
        state.max_count_40 = count;
        state.return_click_tile_4c = click_tile;
        state.count_48 = Some(count);
        state.stack_reference_50 = stack_reference;
        state.field(3).expect("the six tiles")
    };
    tile_set_numeric_trait(cx, scroll, TRAIT_USER1, 1.0)?;
    // 005C0726 fild, then fstp to a float local: an int above 2^24 rounds here.
    let count_float = count as f32;
    tile_set_numeric_trait(cx, scroll, TRAIT_USER2, count_float)?;
    let step = (count / 4).max(1);
    tile_set_numeric_trait(cx, scroll, TRAIT_USER4, step as f32)?;
    tile_set_numeric_trait(cx, scroll, TRAIT_USER5, count_float)?;
    tile_set_numeric_trait(cx, scroll, TRAIT_USER5, 0.0)?;
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(true)
}
