//! MapMenu (1023), as 0057E150's fourth creation builds it: 00579F80 over `Data\Menus\Main\map_menu.xml`, the XML's fields
//! (005B6670 SetField), 005B8FC0's place and time line, and 005BB210's tab, which `map_background`'s user0 (1, the local
//! map) picks while InterfaceManager +0x0D is still 0xFF. 0057E150 leaves the menu hidden, so 005BA4D0, the local map,
//! finds it culled and draws nothing: its `map_local_icon` rand traits stay unevaluated, as the original's move shows.

use recomp_core::reader::StreamError;

use super::ai_conditions::the_player;
use super::ai_package::{game_day, game_hour, game_month, game_year, parent_cell};
use super::forms_world::cell_is_interior;
use super::interface_frame::{
    MENU_STATE_FADING_IN, MENU_STATE_VISIBLE, menu_destroy, menu_enable, tile_hierarchy_visible,
};
use super::interface_manager::{InterfaceManager, menu_register_tile, menu_update_cursor_depth};
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::{
    TRAIT_DEPTH, TRAIT_STACKING, TRAIT_STRING, TRAIT_USER0, TRAIT_VISIBLE, tile_numeric_trait,
    tile_set_numeric_trait, tile_set_string_trait, tile_update_scene_graph,
};
use super::tile_xml::{tile_read_xml, tile_xml_clear_cached_file};

/// IDA: MenuID_Map.
pub const MENU_ID_MAP: u32 = 1023;
const MAP_XML: &str = "Data\\Menus\\Main\\map_menu.xml";

// IDA: TileTrait userN, 4014 + N.
const fn user(n: u16) -> u16 {
    TRAIT_USER0 + n
}

/// IDA: 005B7360 MapMenu_ctor (0x100, via 00587D20's 1023 case): Menu_Constructor, vtable A6CC9C; +0x28..+0x78 zero (the
/// twenty field tiles and +0x78), +0x88 / +0x8C 0.0, +0xB0, +0xB4, +0xB6, +0xC4, +0xD0, +0xE0, +0xF4, +0xF8, +0xFC zero, the
/// byte +0xDC 0, the byte +0x84 0xFF, and +0xC8 a new 8-byte list head {0, 0}. Only the fields a reached path reads are
/// kept. Completion: 100% of those.
#[derive(Clone, Debug, PartialEq)]
pub struct MapMenu {
    /// +0x28..+0x74: fields 9, 1, 2, 3, 4, 5, 11, 12, 13, 23, 31, 32, 41, 42, 46, 47, 48, 61, 62, 63.
    pub tiles_28: [Option<TileId>; 20],
    pub byte_84: u8,
    /// +0xF4: a tile 005BB210 gives user7 0 and drops.
    pub tile_f4: Option<TileId>,
}

impl Default for MapMenu {
    fn default() -> Self {
        Self {
            tiles_28: [None; 20],
            byte_84: 0xFF,
            tile_f4: None,
        }
    }
}

// IDA: 005B6670's ids in slot order, +0x28 first.
const FIELD_IDS: [i32; 20] = [
    9, 1, 2, 3, 4, 5, 11, 12, 13, 23, 31, 32, 41, 42, 46, 47, 48, 61, 62, 63,
];

impl MapMenu {
    /// IDA: 005B6670 MapMenu_SetField: the ids above to +0x28..+0x74; the rest of 1..63 store nothing. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        if let Some(slot) = FIELD_IDS.iter().position(|&known| known == id) {
            self.tiles_28[slot] = Some(tile);
        }
    }

    /// IDA: 005B65F0 MapMenu_HasRequiredTiles: all twenty nonnull. Completion: 100%.
    #[must_use]
    pub fn has_required_tiles(&self) -> bool {
        self.tiles_28.iter().all(Option::is_some)
    }

    // IDA: N/A the tile of a field id.
    fn field(&self, id: i32) -> Option<TileId> {
        FIELD_IDS
            .iter()
            .position(|&known| known == id)
            .and_then(|slot| self.tiles_28[slot])
    }
}

// IDA: N/A the menu's MapMenu part.
fn map<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a MapMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::Map(map) => map,
        _ => unreachable!("a MapMenu"),
    }
}

// IDA: N/A a field the twenty-tile check has guaranteed.
fn field(cx: &InterfaceCx, menu: MenuId, id: i32) -> TileId {
    map(cx, menu).field(id).expect("the twenty tiles")
}

/// IDA: 00579F80 InterfaceManager_CreateMapMenuIfReady (its body in the chunk 005BB6C0): with a cursor (+0x1C) and +0x60,
/// an old 1023 menu deleted; the depth (00584980); Tile_ReadXML(map_menu.xml) under the menu root; its containing menu;
/// 00584670 the cache; null answers null; a menu that isn't 1023 is deleted when it has a tile, and null answers; 00584880
/// registers the tile; without the twenty tiles (005B65F0) the warning and null; a stacking type 6006 or 102 takes the
/// depth; field 11's user5 0; 005B8FC0(0); Menu_Enable(1); Tile_UpdateSceneGraph(0); the tab: InterfaceManager +0x0D, or
/// with 0xFF field 9's user0 through __ftol2, as a signed byte above 5 is 5 and below 1 is 1 (005BB828..005BB836); 005BB210
/// (tab, 0); the root answers. Completion: 100%.
/// # Errors
/// A stop in the XML, the menu calls or the tab.
pub fn map_menu_create(cx: &mut InterfaceCx) -> Result<Option<TileId>, StreamError> {
    let ready = {
        let manager = cx.world.resource::<InterfaceManager>();
        manager.cursor_01c.is_some() && manager.player_scene_node_060.is_some()
    };
    if !ready {
        return Ok(None);
    }
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_MAP)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, MAP_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    tile_xml_clear_cached_file(cx, MAP_XML, menu);
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != MENU_ID_MAP {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(None);
    }
    menu_register_tile(cx, menu, Some(created))?;
    if !map(cx, menu).has_required_tiles() {
        cx.host
            .warning("Map Menu Creation Failed... Are your menu and art resources up to date?");
        return Ok(None);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let field_11 = field(cx, menu, 11);
    tile_set_numeric_trait(cx, field_11, user(5), 0.0)?;
    map_menu_set_title(cx, menu, 0)?;
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, true, frame_time)?;
    tile_update_scene_graph(cx, created, false)?;
    let mut tab = cx.world.resource::<InterfaceManager>().tab_ids_00a[3] as i8;
    if tab as u8 == 0xFF {
        let background = field(cx, menu, 9);
        tab = recomp_core::x87::ftol(tile_numeric_trait(cx, background, user(0))) as i8;
    }
    let tab = tab.clamp(1, 5);
    map_menu_show_tab(cx, menu, i32::from(tab))?;
    Ok(Some(created))
}

/// IDA: 005B8FC0 MapMenu_SetTitle(page): the place, 51..54 the journal's headings (B39468, B39460, B39458, B39470), any
/// other page 004DE8D0 on g_thePlayer (an interior parent cell's full name, else its worldspace's vt+0xDC); an empty one is
/// "Tamriel"; the root's user1. Then the hour (00402BD0): its __ftol2 hours, the fraction (`fisub`, `fmul` 60.0) through
/// __ftol2 as minutes; below 1 hour 12, above 12 less 12; "pm" unless 12.0 is above the hour (`fcomp`, `test ah,41h`);
/// 00402E50's date ("%s %d, 3E%d": the month's name from 00B06FA4, the day and the year), then "%s %i:%i %s", or
/// "%s %i:0%i %s" under ten minutes, as the root's user2. Completion: 100% of page 0 for an interior.
/// TODO: [ ] The journal pages (51..54) and an exterior's worldspace name (vt+0xDC).
/// # Errors
/// An exterior cell or a journal page, which aren't ported; a month outside the table.
pub fn map_menu_set_title(
    cx: &mut InterfaceCx,
    menu: MenuId,
    page: i32,
) -> Result<(), StreamError> {
    if (51..=54).contains(&page) {
        return Err(StreamError("005B8FC0: a journal page's heading is carried"));
    }
    let player = the_player(cx.world).ok_or(StreamError("005B8FC0: no player"))?;
    let mut place = match parent_cell(cx.world, player) {
        Some(cell) if cell_is_interior(cx.world, cell) => full_name(cx, cell),
        Some(_) => {
            return Err(StreamError(
                "004DE8D0: an exterior's worldspace name is carried",
            ));
        }
        None => return Err(StreamError("004DE8D0: a player with no cell is carried")),
    };
    if place.is_empty() {
        place = b"Tamriel".to_vec();
    }
    let root = cx.engine.menu(menu).tile_04.expect("a registered menu");
    tile_set_string_trait(cx, root, user(1), &place)?;
    let hour = game_hour(cx.world);
    let mut hours = recomp_core::x87::ftol(hour);
    let minutes = recomp_core::x87::ftol((hour - hours as f32) * 60.0);
    if hours < 1 {
        hours = 12;
    } else if hours > 12 {
        hours -= 12;
    }
    let suffix = if 12.0 > hour || hour.is_nan() {
        "am"
    } else {
        "pm"
    };
    let date = map_date(cx)?;
    let mut text = date;
    if minutes >= 10 {
        text.extend_from_slice(format!(" {hours}:{minutes} {suffix}").as_bytes());
    } else {
        text.extend_from_slice(format!(" {hours}:0{minutes} {suffix}").as_bytes());
    }
    tile_set_string_trait(cx, root, user(2), &text)
}

// IDA: 00402E50 TimeGlobals_FormatDate: "%s %d, 3E%d" of the month's name (00B06FA4[month], the setting's text), the day
// (`movsx` of its __ftol2 byte) and the year (`fistp` under chop).
pub(super) fn map_date(cx: &InterfaceCx) -> Result<Vec<u8>, StreamError> {
    let month = game_month(cx.world);
    if !(0..=12).contains(&month) {
        return Err(StreamError("00402E50: a month outside 00B06FA4's table"));
    }
    let name = cx
        .world
        .get_resource::<super::game_settings::GameSettingCollection>()
        .and_then(|settings| settings.text_at(0x00B3_50EC + 8 * month as u32))
        .unwrap_or_default();
    let day = i32::from(game_day(cx.world) as i8);
    let year = game_year(cx.world);
    let mut text = name;
    text.extend_from_slice(format!(" {day}, 3E{year}").as_bytes());
    Ok(text)
}

// IDA: the cell's TESFullName data at +0x1C, null as "" (004DE8D0's interior arm).
fn full_name(cx: &InterfaceCx, form: bevy_ecs::entity::Entity) -> Vec<u8> {
    cx.world
        .get::<super::form_components::TesFullName>(form)
        .and_then(|name| name.name_04.data_00.clone())
        .map(|name| {
            let end = name
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(name.len());
            name[..end].to_vec()
        })
        .unwrap_or_default()
}

/// IDA: 005BB210 MapMenu_ShowTab(tab, a3): field 23 visible 1; field 9's user0 the tab and InterfaceManager +0x0D the tab;
/// field 42 visible 1 and its string " " (00A36430); +0xF4's user7 0 and +0xF4 null; then 4 and 5 005BACB0(null) with
/// fields 31 and 32 shown, 3 the same for the active quest, 2 005B91E0 (the world map), 1 005BA4D0(1) (the local map);
/// field 12's user9 -9999999 then 0. Completion: 100% of tab 1.
/// TODO: [ ] Tabs 2..5: the world map (005B91E0) and the journal (005BACB0).
/// # Errors
/// Tabs 2..5, or a local map that runs.
pub fn map_menu_show_tab(cx: &mut InterfaceCx, menu: MenuId, tab: i32) -> Result<(), StreamError> {
    let tabs = field(cx, menu, 23);
    tile_set_numeric_trait(cx, tabs, TRAIT_VISIBLE, 1.0)?;
    let background = field(cx, menu, 9);
    tile_set_numeric_trait(cx, background, user(0), tab as f32)?;
    cx.world.resource_mut::<InterfaceManager>().tab_ids_00a[3] = tab as u8;
    let heading = field(cx, menu, 42);
    tile_set_numeric_trait(cx, heading, TRAIT_VISIBLE, 1.0)?;
    tile_set_string_trait(cx, heading, TRAIT_STRING, b" ")?;
    let MenuClass::Map(state) = &mut cx.engine.menu_mut(menu).class else {
        unreachable!("a MapMenu")
    };
    let dropped = state.tile_f4.take();
    if let Some(tile) = dropped {
        tile_set_numeric_trait(cx, tile, user(7), 0.0)?;
    }
    match tab {
        1 => map_menu_show_local_map(cx, true)?,
        2 => return Err(StreamError("005B91E0: the world map tab is carried")),
        3..=5 => return Err(StreamError("005BACB0: the journal tabs are carried")),
        _ => {}
    }
    let scroll = field(cx, menu, 12);
    tile_set_numeric_trait(cx, scroll, user(9), -9_999_999.0)?;
    tile_set_numeric_trait(cx, scroll, user(9), 0.0)
}

/// IDA: 005BA4D0 MapMenu_ShowLocalMap(a1): the 1023 menu; when its root's scene hierarchy isn't app-culled (005893B0) or
/// the menu is fading in (8) or visible (1): 005B8FC0(0), then with a parent cell the local map (005B7550) and its
/// `map_local_icon` markers. A hidden menu does nothing. Completion: 100% of the gate.
/// TODO: [ ] The shown arm: 005B7550's local map texture, the markers (005B87D0), the player's arrow.
/// # Errors
/// The shown arm, which isn't ported.
pub fn map_menu_show_local_map(cx: &mut InterfaceCx, _first: bool) -> Result<(), StreamError> {
    let Some(tile) = cx.engine.tile_menu_by_id(MENU_ID_MAP) else {
        return Ok(());
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(());
    };
    let state = cx.engine.menu(menu).state_24;
    if tile_hierarchy_visible(cx, tile)
        || state == MENU_STATE_FADING_IN
        || state == MENU_STATE_VISIBLE
    {
        return Err(StreamError("005BA4D0: a shown local map is carried"));
    }
    Ok(())
}
