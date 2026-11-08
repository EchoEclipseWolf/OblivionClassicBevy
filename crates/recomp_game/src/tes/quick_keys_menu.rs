//! QuickKeysMenu (1046), as 0057E150's last creation builds it: 005C1290 over `Data\Menus\Main\quickkeys_menu.xml`, the
//! XML's fields (005C0BB0 SetField) and 005C1000, which hides the slots and starts the menu's fade-out and removal at
//! once. Unlike the other four, an existing QuickKeys menu is kept, there's no cursor check and the XML cache isn't cleared.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::interface_frame::{
    MENU_STATE_FADING_OUT, TRAIT_DESTROY_ON_CLOSE, menu_begin_fade_out, menu_destroy, menu_enable,
};
use super::interface_manager::{InterfaceManager, menu_register_tile, menu_update_cursor_depth};
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::{
    TRAIT_ALPHA, TRAIT_DEPTH, TRAIT_STACKING, TRAIT_USER0, TRAIT_VISIBLE, tile_numeric_trait,
    tile_set_numeric_trait,
};
use super::tile_xml::tile_read_xml;

/// IDA: MenuID_QuickKeys.
pub const MENU_ID_QUICK_KEYS: u32 = 1046;
const QUICK_KEYS_XML: &str = "Data\\Menus\\Main\\quickkeys_menu.xml";

/// IDA: 005C0B50 QuickKeysMenu_ctor's statics: dword_B3B418..B3B434 zeroed in its loop, then B3B424..B3B438 -1 (stored
/// in the order 430, 424, 434, 428, 438, 42C) and the byte B3B420 0; 005C1000 clears the byte B3B43D. No reader is
/// ported. Completion: 100% of the values.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct QuickKeysStatics {
    /// B3B418, B3B41C, B3B420 (its low byte), B3B424 .. B3B438.
    pub dwords_b3b418: [i32; 9],
    pub byte_b3b43d: u8,
}

/// IDA: 005C0B50 QuickKeysMenu_ctor (via 00587D20's 1046 case): Menu_Constructor, vtable A6D1F4, the ten tile pointers
/// +0x28..+0x4C null; the statics above. Completion: 100%.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuickKeysMenu {
    /// +0x28.. fields 6..15.
    pub tiles_28: [Option<TileId>; 10],
}

impl QuickKeysMenu {
    /// IDA: 005C0BB0 QuickKeysMenu_SetField: ids 6..15 to +0x28..+0x4C. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        if (6..=15).contains(&id) {
            self.tiles_28[id as usize - 6] = Some(tile);
        }
    }

    /// IDA: 005A46B0: all ten nonnull. Completion: 100%.
    #[must_use]
    pub fn has_required_tiles(&self) -> bool {
        self.tiles_28.iter().all(Option::is_some)
    }
}

/// IDA: 005C0B50's statics, as the 1046 case constructs the menu. Completion: 100%.
pub fn quick_keys_menu_constructed(world: &mut World) -> QuickKeysMenu {
    let mut statics = world.get_resource_or_init::<QuickKeysStatics>();
    statics.dwords_b3b418[..8].fill(0);
    statics.dwords_b3b418[3..9].fill(-1);
    statics.dwords_b3b418[2] &= !0xFF;
    QuickKeysMenu::default()
}

// IDA: N/A the menu's QuickKeysMenu part.
fn quick_keys<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a QuickKeysMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::QuickKeys(quick_keys) => quick_keys,
        _ => unreachable!("a QuickKeysMenu"),
    }
}

/// IDA: 005C1290 QuickKeysMenu_CreateIfAbsent: an existing 1046 tile answers true; the depth (00584980); Tile_ReadXML
/// (quickkeys_menu.xml) under the menu root; its containing menu, else false; a menu that isn't 1046 is deleted when it
/// has a tile, and false answers; 00584880 registers the tile; with the ten tiles, a stacking type 6006 or 102 takes the
/// depth, user0 is InterfaceManager +0x08 != 1, field 7's user1 0 and visible 1, 005C1000, Menu_Enable(0) and true;
/// without them the warning, the menu deleted, false. Completion: 100%.
/// # Errors
/// A stop in the XML or the menu calls.
pub fn quick_keys_menu_create(cx: &mut InterfaceCx) -> Result<bool, StreamError> {
    if cx.engine.tile_menu_by_id(MENU_ID_QUICK_KEYS).is_some() {
        return Ok(true);
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, QUICK_KEYS_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(false);
    };
    if cx.engine.menu(menu).id() != MENU_ID_QUICK_KEYS {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(false);
    }
    menu_register_tile(cx, menu, Some(created))?;
    if !quick_keys(cx, menu).has_required_tiles() {
        cx.host.warning(
            "Quick Keys Menu Creation Failed... Are your menu and art resources up to date?",
        );
        menu_destroy(cx, menu)?;
        return Ok(false);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let status = cx
        .world
        .resource::<InterfaceManager>()
        .menu_loading_status_008;
    tile_set_numeric_trait(cx, created, TRAIT_USER0, f32::from(u8::from(status != 1)))?;
    let slots = quick_keys(cx, menu).tiles_28[1].expect("the ten tiles");
    tile_set_numeric_trait(cx, slots, TRAIT_USER0 + 1, 0.0)?;
    tile_set_numeric_trait(cx, slots, TRAIT_VISIBLE, 1.0)?;
    quick_keys_menu_close(cx)?;
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(true)
}

/// IDA: 005C1000 QuickKeysMenu_Close: with a 1046 menu not fading out (state 2), field 7 visible 1, field 6 alpha 0,
/// B3B43D 0, then the menu tile's destroyOnClose 1 and 00584740 Menu_BeginFadeOutAndRemoveMode. Completion: 100%.
/// # Errors
/// A stop in the trait writes.
pub fn quick_keys_menu_close(cx: &mut InterfaceCx) -> Result<(), StreamError> {
    let Some(tile) = cx.engine.tile_menu_by_id(MENU_ID_QUICK_KEYS) else {
        return Ok(());
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(());
    };
    if cx.engine.menu(menu).state_24 == MENU_STATE_FADING_OUT {
        return Ok(());
    }
    let tiles = quick_keys(cx, menu).tiles_28;
    if let Some(slots) = tiles[1] {
        tile_set_numeric_trait(cx, slots, TRAIT_VISIBLE, 1.0)?;
    }
    if let Some(background) = tiles[0] {
        tile_set_numeric_trait(cx, background, TRAIT_ALPHA, 0.0)?;
    }
    cx.world
        .get_resource_or_init::<QuickKeysStatics>()
        .byte_b3b43d = 0;
    // 005C1053 jumps to the chunk 005C0D20: the menu looked up again.
    let Some(tile) = cx.engine.tile_menu_by_id(MENU_ID_QUICK_KEYS) else {
        return Ok(());
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(());
    };
    tile_set_numeric_trait(cx, tile, TRAIT_DESTROY_ON_CLOSE, 1.0)?;
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_begin_fade_out(cx, menu, frame_time)
}
