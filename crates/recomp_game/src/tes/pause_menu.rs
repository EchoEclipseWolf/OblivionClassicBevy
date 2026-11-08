//! PauseMenu (1013): 005BD960 PauseMenu_ctor's fields, 005BD990 SetField while `Data\Menus\Options\pause_menu.xml` is
//! built, and 005BDA90, which builds it, with the two state reads it makes: 00452330 (whether a save can be made now)
//! and 00579060 (the first menu mode on the stack). Its callers (0057B560, `PlayerCharacter_UpdateFrame` 00671620) and its
//! click (005BDE70), focus sound (005BD9E0) and vt+0x38 (005BDFA0) aren't ported.

use recomp_core::reader::StreamError;

use super::ai_conditions::the_player;
use super::ai_package::actor_is_dead;
use super::hud_main_menu::interface_active_top_menu_id;
use super::interface_frame::{
    MENU_STATE_FADING_IN, MENU_STATE_FADING_OUT, TRAIT_TARGET, menu_destroy, menu_enable,
};
use super::interface_manager::{InterfaceManager, menu_register_tile, menu_update_cursor_depth};
use super::player::PlayerCharacter;
use super::quick_keys_menu::{QuickKeysStatics, quick_keys_menu_close};
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::{
    TRAIT_DEPTH, TRAIT_STACKING, TRAIT_USER0, TRAIT_USER1, tile_numeric_trait,
    tile_set_numeric_trait,
};
use super::tile_xml::tile_read_xml;

/// IDA: MenuID_Pause.
pub const MENU_ID_PAUSE: u32 = 1013;
const PAUSE_XML: &str = "Data\\Menus\\Options\\pause_menu.xml";
/// IDA: MenuID_Save, MenuID_Load, MenuID_Console.
const MENU_ID_SAVE: u32 = 1039;
const MENU_ID_LOAD: u32 = 1038;
const MENU_ID_CONSOLE: u32 = 3;

/// IDA: 005BD960 PauseMenu_ctor (via 00587D20's 1013 case, FormHeap 0x3C): Menu_Constructor, the five tile pointers
/// +0x28..+0x38 null, vtable. Completion: 100%.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PauseMenu {
    /// +0x28.. fields 3, 4, 5, 7 and 8: `pause_return_button`, `pause_save_button`, `pause_load_button`,
    /// `pause_options_button` and `pause_quit_button`.
    pub tiles_28: [Option<TileId>; 5],
}

/// IDA: 005BD990's ids, in +0x28..+0x38 order.
const FIELD_IDS: [i32; 5] = [3, 4, 5, 7, 8];

impl PauseMenu {
    /// IDA: 005BD990 PauseMenu_SetField: ids 3, 4, 5, 7, 8 to +0x28..+0x38. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        if let Some(slot) = FIELD_IDS.iter().position(|&known| known == id) {
            self.tiles_28[slot] = Some(tile);
        }
    }

    /// IDA: 005BDBA1..005BDBBD: +0x28, +0x30, +0x2C, +0x34, +0x38 all nonnull. Completion: 100%.
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

// IDA: N/A the menu's PauseMenu part.
fn pause<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a PauseMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::Pause(pause) => pause,
        _ => unreachable!("a PauseMenu"),
    }
}

/// IDA: 00579060 Interface_GetFirstStackedMenuMode: no manager, cursor or root, or the root's user0 not 2.0: 0;
/// otherwise 0057CFA0(0), the stack's slot 0 (+0xE0). Completion: 100%.
#[must_use]
pub fn interface_first_stacked_menu_mode(cx: &InterfaceCx) -> u32 {
    let Some(manager) = cx.world.get_resource::<InterfaceManager>() else {
        return 0;
    };
    let (Some(_), Some(root)) = (manager.cursor_01c, manager.menu_root_068) else {
        return 0;
    };
    let first = manager.menu_mode_stack_0e0[0];
    if tile_numeric_trait(cx, root, TRAIT_USER0) == 2.0 {
        first
    } else {
        0
    }
}

/// IDA: 00452330 TESSaveLoadGame_CanSaveNow (a thiscall on g_saveLoadGame that reads none of it): the player's death state
/// (005E0DC0, +0xB0) neither 2 nor 1, 0065D140 (+0x71D or +0x71C set) false, and both 00578FE0's top menu and
/// 00579060's first one Save, none, Pause or the console. Completion: 100%.
/// # Errors
/// No player: 00452336 reads null's +0xB0.
pub fn save_load_can_save_now(cx: &InterfaceCx) -> Result<bool, StreamError> {
    let player = the_player(cx.world).ok_or(StreamError(
        "00452336: no player, and the original reads its +0xB0",
    ))?;
    let state = cx
        .world
        .get::<super::actor::Actor>(player)
        .ok_or(StreamError(
            "00452336: the player has no Actor part to read +0xB0 from",
        ))?
        .dead_state_b0;
    if state == 2 || state == 1 {
        return Ok(false);
    }
    let data = cx.world.get::<PlayerCharacter>(player).ok_or(StreamError(
        "0045235B: the player has no PlayerCharacter part to read +0x71C from",
    ))?;
    if data.sit_sleep_exit_pending_71d != 0 || data.sit_sleep_entry_pending_71c != 0 {
        return Ok(false);
    }
    let allowed = |id: u32| matches!(id, MENU_ID_SAVE | 0 | MENU_ID_PAUSE | MENU_ID_CONSOLE);
    Ok(allowed(interface_active_top_menu_id(cx)) && allowed(interface_first_stacked_menu_mode(cx)))
}

/// IDA: 005BDA90 PauseMenu_Create: any child of the menu root whose menu has a tile and is fading in (8) or out (2) with
/// a stacking type other than 6008 (`fcomp`, `test ah,44h; jp`) answers false at once. An existing 1013 menu deleted
/// (TileMenu vt+0, ported as `map_menu_create` does it); the depth (00588890 → 00584980); Tile_ReadXML(pause_menu.xml)
/// under the root; its containing menu, else false; a menu that isn't 1013 is deleted when it has a tile, and false.
/// InterfaceManager_PlayUISound(2), then 00584880 registers the tile; without the five tiles the warning and false (the
/// menu stays). A stacking type 6006 or 102 takes the depth. 00452330 false: field 4's target 1; 00579060 Load: field 5's
/// target 1; no player, or the player's vt+0x198 (005E33B0 Actor_IsDead(0)): field 3's user1 1. With 00B3B43D set,
/// 005C1000 QuickKeysMenu_Close. Menu_Enable(0), true. Completion: 100%.
/// # Errors
/// No player at 00452336 (the original faults there before its own null test at 005BDC66); a stop in the XML or the menu
/// calls.
pub fn pause_menu_create(cx: &mut InterfaceCx) -> Result<bool, StreamError> {
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let children = root.map_or_else(Vec::new, |root| cx.engine.tile(root).children_30.clone());
    for child in children {
        let Some(menu) = cx.engine.containing_menu(child) else {
            continue;
        };
        if cx.engine.menu(menu).tile_04.is_none() {
            continue;
        }
        let state = cx.engine.menu(menu).state_24;
        if (state == MENU_STATE_FADING_IN || state == MENU_STATE_FADING_OUT)
            && tile_numeric_trait(cx, child, TRAIT_STACKING) != 6008.0
        {
            return Ok(false);
        }
    }
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_PAUSE)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, PAUSE_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(false);
    };
    if cx.engine.menu(menu).id() != MENU_ID_PAUSE {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(false);
    }
    cx.host.play_ui_sound(2);
    menu_register_tile(cx, menu, Some(created))?;
    if !pause(cx, menu).has_required_tiles() {
        cx.host
            .warning("Pause Menu Creation Failed... Are your menu and art resources up to date?");
        return Ok(false);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let fields = pause(cx, menu).clone();
    let field = |id| fields.field(id).expect("the five tiles");
    if !save_load_can_save_now(cx)? {
        tile_set_numeric_trait(cx, field(4), TRAIT_TARGET, 1.0)?;
    }
    if interface_first_stacked_menu_mode(cx) == MENU_ID_LOAD {
        tile_set_numeric_trait(cx, field(5), TRAIT_TARGET, 1.0)?;
    }
    let dead = the_player(cx.world).is_none_or(|player| actor_is_dead(cx.world, player, false));
    if dead {
        tile_set_numeric_trait(cx, field(3), TRAIT_USER1, 1.0)?;
    }
    if cx
        .world
        .get_resource_or_init::<QuickKeysStatics>()
        .byte_b3b43d
        != 0
    {
        quick_keys_menu_close(cx)?;
    }
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(true)
}
