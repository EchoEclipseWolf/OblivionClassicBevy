//! ControlsMenu (1021): 005874A0 ControlsMenu_ctor's fields, 005879A0 SetField while `Data\Menus\Options\controls_menu.xml`
//! is built, and 0059BA40, which builds it: the device pick (0059B920), the sensitivity slider, the invert button
//! (0059B640), and one `controls_template` row per shown control (0059B670) with its name from the 29 control settings
//! (00B399D0). Its caller (the Options menu's click 005BD880) and its click (0059C5E0), key remapping and per-frame
//! virtuals (0059B6E0, 0059B850, 0059BD80) aren't ported.

use oblivion_core::error::GameError;

use crate::tes::game_settings::{CONTROL_SETTING_COUNT, GameSettingCollection};
use crate::tes::interface_frame::{TRAIT_TARGET, menu_destroy, menu_enable};
use crate::tes::interface_manager::{
    InterfaceManager, menu_register_tile, menu_set_on_off_button_text, menu_update_cursor_depth,
};
use crate::tes::menus::show_message;
use crate::tes::os_globals::OsGlobalsData;
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use crate::tes::tile_values::{
    TRAIT_BLUE, TRAIT_DEPTH, TRAIT_GREEN, TRAIT_HEIGHT, TRAIT_ID, TRAIT_RED, TRAIT_STACKING,
    TRAIT_USER0, TRAIT_USER1, TRAIT_USER3, TRAIT_USER4, TRAIT_USER5, tile_numeric_trait,
    tile_set_numeric_trait, tile_set_string_trait,
};
use crate::tes::tile_xml::{tile_build_template, tile_read_xml};

/// IDA: MenuID_Controls.
pub const MENU_ID_CONTROLS: u32 = 1021;
const CONTROLS_XML: &str = "Data\\Menus\\Options\\controls_menu.xml";
/// IDA: 00B38EC8 sNoDevice, 00B38CF0 sOk; 00B38DA0 sOnButtonText, 00B38DA8 sOffButtonText.
const S_NO_DEVICE_B38EC8: u32 = 0x00B3_8EC8;
const S_OK_B38CF0: u32 = 0x00B3_8CF0;

/// IDA: 005874A0 ControlsMenu_ctor (via 00587D20's 1021 case, FormHeap 0xE8): Menu_Constructor, vtable, +0x28..+0x5B
/// and +0x60..+0xD3 zeroed, +0x5C 255, the byte +0xD4 0, +0xD8 0, the byte +0xE4 0. +0xDC and +0xE0 aren't stored
/// (FormHeap's bytes until 0059BA40 sets them). Completion: 100% of the stores.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlsMenu {
    /// +0x28.. fields 1..13 (005879A0: id - 1 + 10 dwords): 1 `controls_return_button`, 2 `controls_vertical_scroll`, 3
    /// its marker (by user6), 4 `controls_list_pane`, 5 `controls_sensitivity_slider`, 6 its marker, 7
    /// `controls_invert_y_button`, 8 `controls_swap_button`, 9 `controls_more_button`, 10 `controls_defaults_button`, 11
    /// `controls_instructions`, 12 and 13 the device toggles.
    pub tiles_28: [Option<TileId>; 13],
    /// +0x5C: the device shown (0 keyboard, 1 mouse, 2 joystick), 255 from the ctor; 0059B920 steps it.
    pub device_5c: u32,
    /// +0x60..+0xD3: zeroed by the ctor; read only by the click and remapping code, not ported.
    pub bytes_60: [u8; 0x74],
    /// +0xD4: set to 1 at the create's end; read by code not ported.
    pub byte_d4: u8,
    /// +0xD8: zeroed by the ctor.
    pub dword_d8: u32,
    /// +0xDC, +0xE0: the list scroll's user3 and user4 at creation.
    pub scroll_user3_dc: Option<f32>,
    pub scroll_user4_e0: Option<f32>,
    /// +0xE4: zeroed by the ctor.
    pub byte_e4: u8,
}

impl Default for ControlsMenu {
    fn default() -> Self {
        Self {
            tiles_28: [None; 13],
            device_5c: 255,
            bytes_60: [0; 0x74],
            byte_d4: 0,
            dword_d8: 0,
            scroll_user3_dc: None,
            scroll_user4_e0: None,
            byte_e4: 0,
        }
    }
}

impl ControlsMenu {
    /// IDA: 005879A0 ControlsMenu_SetField: ids 1..13 to +0x28..+0x58 (`(id - 1) <= 0xC`, unsigned). Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        if (1..=13).contains(&id) {
            self.tiles_28[id as usize - 1] = Some(tile);
        }
    }

    /// IDA: N/A field `id`.
    #[must_use]
    pub fn field(&self, id: i32) -> Option<TileId> {
        (1..=13)
            .contains(&id)
            .then(|| self.tiles_28[id as usize - 1])?
    }
}

// IDA: N/A the menu's ControlsMenu part.
fn controls<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a ControlsMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::Controls(controls) => controls,
        _ => unreachable!("a ControlsMenu"),
    }
}

/// IDA: 0059B670 ControlsMenu_IsControlShown(index): 0..16, 26, 27 and 28 (`n <= 0x10`, unsigned). Completion: 100%.
#[must_use]
pub const fn controls_menu_is_control_shown(index: u32) -> bool {
    index <= 0x10 || index == 26 || index == 27 || index == 28
}

/// IDA: 0059B920 ControlsMenu_SelectNextDevice: up to three tries, each stepping +0x5C (wrapping at 3) and testing
/// g_osGlobals' input: 0 the keyboard, 1 the mouse, 2 a joystick count; true at the first present. Completion: 100%.
/// # Errors
/// No input object (g_osGlobals +0x20), which the original reads.
pub fn controls_menu_select_next_device(
    cx: &mut InterfaceCx,
    menu: MenuId,
) -> Result<bool, GameError> {
    let input = cx
        .world
        .get_resource::<OsGlobalsData>()
        .and_then(|globals| globals.input_20.clone())
        .ok_or(GameError::unclassified(
            0x0059B935,
            "0059B935: no OSInputGlobals, and the original reads its devices",
        ))?;
    let input = input.lock();
    let MenuClass::Controls(state) = &mut cx.engine.menu_mut(menu).class else {
        unreachable!("a ControlsMenu")
    };
    for _ in 0..3 {
        state.device_5c = state.device_5c.wrapping_add(1);
        if state.device_5c >= 3 {
            state.device_5c = 0;
        }
        let present = match state.device_5c {
            0 => input.keyboard_0028.is_some(),
            1 => input.mouse_002c.is_some(),
            2 => input.joystick_count_18f0 != 0,
            _ => false,
        };
        if present {
            return Ok(true);
        }
    }
    Ok(false)
}

/// IDA: 00B399D0[index]'s +0 text: the setting object's current value (registered under its key), else the
/// definition's. A null entry (FormHeap failing in 009F6040) is null. Completion: 100%.
fn control_name(cx: &InterfaceCx, index: usize) -> Option<Vec<u8>> {
    let settings = cx.world.get_resource::<GameSettingCollection>()?;
    let definition = settings.control_settings()[index]?;
    let registered = definition
        .name
        .and_then(|name| settings.find(name))
        .filter(|setting| std::ptr::eq(setting.definition, definition));
    match registered {
        Some(setting) => setting.text.clone(),
        None => definition.text.map(<[u8]>::to_vec),
    }
}

/// IDA: 0059BA40 ControlsMenu_Create: an existing 1021 menu deleted (TileMenu vt+0, ported as `map_menu_create` does it);
/// the depth (00588890 → 00584980); Tile_ReadXML(controls_menu.xml) under the root; its containing menu, else false; a
/// menu that isn't 1021 is deleted when it has a tile, and false; 00584880 registers the tile. 0059B920 false: the
/// message sNoDevice with one button sOk (ShowMessageBox, no callback, base 1). Without the thirteen tiles the warning and
/// false (the menu stays). A stacking type 6006 or 102 takes the depth. Field 5's user5 (fMouseSensitivity - 0.0005) /
/// 0.0095 * 100 (doubles that are those floats, at the main thread's 24 bits, stored as a float), then 0; field 7 On or
/// Off by bInvertYValues; with no joystick field 8's target 1. Then for each of the 29 controls shown (0059B670) a
/// `controls_template` row under field 4 (00585410), and with one: its id 14 + index, user0 the row number (counted only
/// for built rows), user1 the control's name, height the root's user1, red, green and blue the root's. Field 2's user5
/// -9999999 then 0; +0xDC and +0xE0 its user3 and user4; +0xD4 1; Menu_Enable(0), true. Completion: 100%.
/// # Errors
/// No input object (0059B920); a null control name (0059BC6F, where the original stores a null string); a stop in the XML,
/// the template or the menu calls.
pub fn controls_menu_create(cx: &mut InterfaceCx) -> Result<bool, GameError> {
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_CONTROLS)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, CONTROLS_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(false);
    };
    if cx.engine.menu(menu).id() != MENU_ID_CONTROLS {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(false);
    }
    menu_register_tile(cx, menu, Some(created))?;
    if !controls_menu_select_next_device(cx, menu)? {
        let (text, ok) = {
            let settings = cx.world.get_resource::<GameSettingCollection>();
            let text = |address| {
                settings
                    .and_then(|settings| settings.text_at(address))
                    .unwrap_or_default()
            };
            (text(S_NO_DEVICE_B38EC8), text(S_OK_B38CF0))
        };
        show_message(cx, &text, None, 1, &[ok])?;
    }
    if !controls(cx, menu).tiles_28.iter().all(Option::is_some) {
        cx.host.warning(
            "Controls Menu Creation Failed... Are your menu and art resources up to date?",
        );
        return Ok(false);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let fields = controls(cx, menu).clone();
    let field = |id| fields.field(id).expect("the thirteen tiles");
    // 0059BB69..0059BB85: fsub dbl_A59B38, fdiv dbl_A6B760, fmul dbl_A309F0, each rounding to a float at 24 bits.
    let sensitivity = (cx.settings.mouse_sensitivity_b14ee8 - 0.0005) / 0.0095 * 100.0;
    tile_set_numeric_trait(cx, field(5), TRAIT_USER5, sensitivity)?;
    tile_set_numeric_trait(cx, field(5), TRAIT_USER5, 0.0)?;
    let invert = cx.settings.invert_y_values_b14f38;
    menu_set_on_off_button_text(cx, Some(field(7)), invert)?;
    let joysticks = cx
        .world
        .get_resource::<OsGlobalsData>()
        .and_then(|globals| globals.input_20.clone())
        .map(|input| input.lock().joystick_count_18f0)
        .ok_or(GameError::unclassified(
            0x0059BBC9,
            "0059BBC9: no OSInputGlobals",
        ))?;
    if joysticks == 0 {
        tile_set_numeric_trait(cx, field(8), TRAIT_TARGET, 1.0)?;
    }
    let mut row = 0i32;
    for index in 0..CONTROL_SETTING_COUNT as u32 {
        if !controls_menu_is_control_shown(index) {
            continue;
        }
        let Some(child) = tile_build_template(cx, menu, field(4), b"controls_template")? else {
            continue;
        };
        tile_set_numeric_trait(cx, child, TRAIT_ID, (index as i32 + 14) as f32)?;
        tile_set_numeric_trait(cx, child, TRAIT_USER0, row as f32)?;
        row += 1;
        let name = control_name(cx, index as usize).ok_or(GameError::unclassified(
            0x0059BC6F,
            "0059BC6F: a null control setting, whose null string the original stores",
        ))?;
        tile_set_string_trait(cx, child, TRAIT_USER1, &name)?;
        let height = tile_numeric_trait(cx, created, TRAIT_USER1);
        tile_set_numeric_trait(cx, child, TRAIT_HEIGHT, height)?;
        for color in [TRAIT_RED, TRAIT_GREEN, TRAIT_BLUE] {
            let value = tile_numeric_trait(cx, created, color);
            tile_set_numeric_trait(cx, child, color, value)?;
        }
    }
    let scroll = field(2);
    tile_set_numeric_trait(cx, scroll, TRAIT_USER5, -9_999_999.0)?;
    tile_set_numeric_trait(cx, scroll, TRAIT_USER5, 0.0)?;
    let user3 = tile_numeric_trait(cx, scroll, TRAIT_USER3);
    let user4 = tile_numeric_trait(cx, scroll, TRAIT_USER4);
    if let MenuClass::Controls(state) = &mut cx.engine.menu_mut(menu).class {
        state.scroll_user3_dc = Some(user3);
        state.scroll_user4_e0 = Some(user4);
        state.byte_d4 = 1;
    }
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(true)
}
