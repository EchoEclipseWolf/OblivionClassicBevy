//! OptionsMenu (1015): 005BD580 OptionsMenu_ctor's fields, 005BD5B0 SetField while `Data\Menus\Options\options_menu.xml`
//! is built, and 005BD680, which builds it. Its callers (MainMenu_HandleClick 005B656D, the Pause menu's click 005BDE70)
//! and its own click 005BD880 (which opens the Gameplay, Video, Audio and Controls menus) aren't wired or ported, so the
//! MainMenu's Options button still warns.

use oblivion_core::error::GameError;

use crate::tes::interface_frame::{menu_destroy, menu_enable};
use crate::tes::interface_manager::{
    InterfaceManager, menu_register_tile, menu_update_cursor_depth,
};
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use crate::tes::tile_values::{
    TRAIT_DEPTH, TRAIT_STACKING, TRAIT_USER0, tile_numeric_trait, tile_set_numeric_trait,
};
use crate::tes::tile_xml::tile_read_xml;

/// IDA: MenuID_Options.
pub const MENU_ID_OPTIONS: u32 = 1015;
const OPTIONS_XML: &str = "Data\\Menus\\Options\\options_menu.xml";
/// IDA: MenuID_Pause.
const MENU_ID_PAUSE: u32 = 1013;

/// IDA: 005BD580 OptionsMenu_ctor (via 00587D20's 1015 case, FormHeap 0x40): Menu_Constructor, the six tile pointers
/// +0x28..+0x3C null, vtable. Completion: 100%.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OptionsMenu {
    /// +0x28.. fields 2, 5, 6, 7, 8, 9: `options_return_button`, `options_gameplay_button`, `options_video_button`,
    /// `options_audio_button`, `options_controls_button`, `options_downloads_button`.
    pub tiles_28: [Option<TileId>; 6],
}

/// IDA: 005BD5B0's ids, in +0x28..+0x3C order.
const FIELD_IDS: [i32; 6] = [2, 5, 6, 7, 8, 9];

impl OptionsMenu {
    /// IDA: 005BD5B0 OptionsMenu_SetField: ids 2, 5, 6, 7, 8, 9 to +0x28..+0x3C. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        if let Some(slot) = FIELD_IDS.iter().position(|&known| known == id) {
            self.tiles_28[slot] = Some(tile);
        }
    }

    /// IDA: 005BD720..005BD748: all six nonnull. Completion: 100%.
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

// IDA: N/A the menu's OptionsMenu part.
fn options<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a OptionsMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::Options(options) => options,
        _ => unreachable!("an OptionsMenu"),
    }
}

/// IDA: 005BD680 OptionsMenu_Create: an existing 1015 menu deleted (TileMenu vt+0, ported as `map_menu_create` does it);
/// the depth (00588890 → 00584980); Tile_ReadXML(options_menu.xml) under the root; its containing menu, else false; a menu
/// that isn't 1015 is deleted when it has a tile, and false; 00584880 registers the tile; without the six tiles the warning
/// and false (the menu stays). A stacking type 6006 or 102 takes the depth. Then the mode stack from slot 0 (0057CFA0):
/// up to 10 slots, stopping at Options itself or after the first Pause; the root's user0 is 2 with a Pause below, else 1
/// (`setnz`, `add 1`, `fild`). Menu_Enable(0), true. Completion: 100%.
/// # Errors
/// A stop in the XML or the menu calls.
pub fn options_menu_create(cx: &mut InterfaceCx) -> Result<bool, GameError> {
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_OPTIONS)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, OPTIONS_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(false);
    };
    if cx.engine.menu(menu).id() != MENU_ID_OPTIONS {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(false);
    }
    menu_register_tile(cx, menu, Some(created))?;
    if !options(cx, menu).has_required_tiles() {
        cx.host
            .warning("Options Menu Creation Failed... Are your menu and art resources up to date?");
        return Ok(false);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let stack = cx.world.resource::<InterfaceManager>().menu_mode_stack_0e0;
    let mut under_pause = false;
    for &mode in &stack {
        if mode == MENU_ID_OPTIONS {
            break;
        }
        if mode == MENU_ID_PAUSE {
            under_pause = true;
            break;
        }
    }
    let user0 = i32::from(under_pause) + 1;
    tile_set_numeric_trait(cx, created, TRAIT_USER0, user0 as f32)?;
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(true)
}
