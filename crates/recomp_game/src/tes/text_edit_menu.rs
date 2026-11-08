//! TextEditMenu (1051): 005879C0 TextEditMenu_ctor's fields with its MenuTextInputState (0057FE70), 00587AE0 SetField
//! while `Data\Menus\Dialog\TextEditMenu.xml` is built, and 005DCF20, which builds it with a prompt and a default text
//! (00583DD0, 0057FF20 and 0057DD90 on the input state). Its caller (005976B0) and its click (005DD1B0), key input and
//! per-frame virtuals (005DD120, 005DD160) aren't ported.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::interface_frame::{menu_destroy, menu_enable};
use super::interface_manager::{
    InterfaceManager, menu_register_tile, menu_update_cursor_depth, ui_virtual_width,
};
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::{
    TRAIT_DEPTH, TRAIT_STACKING, TRAIT_STRING, TRAIT_USER0, TRAIT_WRAP_WIDTH, tile_numeric_trait,
    tile_set_numeric_trait, tile_set_string_trait, tile_string_trait,
};
use super::tile_xml::tile_read_xml;

/// IDA: MenuID_TextEdit.
pub const MENU_ID_TEXT_EDIT: u32 = 1051;
const TEXT_EDIT_XML: &str = "Data\\Menus\\Dialog\\TextEditMenu.xml";

/// IDA: 00B3B738, a BSStringT (+0 data, +4 length, +6 capacity) the ctor frees and clears. Its readers (the click
/// 005DD1B0) aren't ported. Completion: 100% of the ctor's stores.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct TextEditStatics {
    pub text_b3b738: Vec<u8>,
}

/// IDA: MenuTextInputState (0x28): 0057FE70's fields. Completion: 100% of the fields the create reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuTextInputState {
    /// +0x00.
    pub enabled_00: u8,
    /// +0x04.
    pub caret_index_04: u32,
    /// +0x08: zeroed by the ctor; its readers aren't ported.
    pub byte_08: u8,
    /// +0x0C: -1 from the ctor.
    pub max_pixel_width_0c: i32,
    /// +0x10: zeroed by the ctor; its readers aren't ported.
    pub dword_10: u32,
    /// +0x14: 1 from the ctor.
    pub font_index_14: u32,
    /// +0x18, +0x20 BSStringTs.
    pub editable_text_18: Vec<u8>,
    pub mirrored_text_20: Vec<u8>,
}

impl Default for MenuTextInputState {
    /// IDA: 0057FE70 MenuTextInputState_ctor: both strings empty, +0x00 0, +0x04 0, +0x08 0, +0x0C -1, +0x10 0, +0x14 1.
    /// Completion: 100%.
    fn default() -> Self {
        Self {
            enabled_00: 0,
            caret_index_04: 0,
            byte_08: 0,
            max_pixel_width_0c: -1,
            dword_10: 0,
            font_index_14: 1,
            editable_text_18: Vec::new(),
            mirrored_text_20: Vec::new(),
        }
    }
}

impl MenuTextInputState {
    /// IDA: 00583DD0 MenuTextInputState_SetMaxPixelWidth(width): +0x0C = width - 5, and when that's negative (`jns`)
    /// __ftol2 of 0057D7A0's virtual width. Completion: 100%.
    pub fn set_max_pixel_width(&mut self, width: i32, virtual_width: f32) {
        self.max_pixel_width_0c = width.wrapping_sub(5);
        if self.max_pixel_width_0c < 0 {
            self.max_pixel_width_0c = x87::ftol(virtual_width);
        }
    }

    /// IDA: 0057FF20 MenuTextInputState_SetText: BSStringT_Set on +0x18, then +0x20. Completion: 100%.
    pub fn set_text(&mut self, text: &[u8]) {
        self.editable_text_18 = text.to_vec();
        self.mirrored_text_20 = text.to_vec();
    }

    /// IDA: 0057DD90 MenuTextInputState_SetEnabled(enabled): when +0x00 is clear and `enabled`, the caret goes to the end
    /// of +0x18 (its length word, or strlen at 0xFFFF); then +0x00 = enabled. Completion: 100%.
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled_00 == 0 && enabled {
            self.caret_index_04 = self.editable_text_18.len() as u32;
        }
        self.enabled_00 = u8::from(enabled);
    }
}

/// IDA: 005879C0 TextEditMenu_ctor (via 00587D20's 1051 case, FormHeap 0x5C): Menu_Constructor, vtable, 0057FE70 on
/// +0x34, +0x28..+0x30 null; then 00B3B738's data freed and cleared, its two words 0. Completion: 100%.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextEditMenu {
    /// +0x28.. fields 1, 2, 3: `textedit_text`, `textedit_button_ok`, `textedit_button_back`.
    pub tiles_28: [Option<TileId>; 3],
    /// +0x34.
    pub input_state_34: MenuTextInputState,
}

impl TextEditMenu {
    /// IDA: 00587AE0 TextEditMenu_SetField: ids 1..3 to +0x28..+0x30 (`(id - 1) <= 2`, unsigned). Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        if (1..=3).contains(&id) {
            self.tiles_28[id as usize - 1] = Some(tile);
        }
    }
}

/// IDA: 005879C0's stores. Completion: 100%.
pub fn text_edit_menu_constructed(world: &mut World) -> TextEditMenu {
    world
        .get_resource_or_init::<TextEditStatics>()
        .text_b3b738
        .clear();
    TextEditMenu::default()
}

// IDA: N/A the menu's TextEditMenu part.
fn text_edit<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a TextEditMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::TextEdit(text_edit) => text_edit,
        _ => unreachable!("a TextEditMenu"),
    }
}

/// IDA: 005DCF20 TextEditMenu_CreateFromXML(prompt, default): an existing 1051 menu deleted (TileMenu vt+0, ported as
/// `map_menu_create` does it); the depth (00588890 → 00584980); Tile_ReadXML(TextEditMenu.xml) under the root; its
/// containing menu, else false; a menu that isn't 1051 is deleted when it has a tile, and false; 00584880 registers the
/// tile; without the three tiles the warning and false (the menu stays). A stacking type 6006 or 102 takes the depth.
/// The root's user0 the prompt, field 1's string the default; on +0x34: 00583DD0(__ftol2 of field 1's wrapwidth),
/// 0057FF20(field 1's string as it now reads), 0057DD90(1). Menu_Enable(0), true. Completion: 100%.
/// # Errors
/// A stop in the XML or the menu calls.
pub fn text_edit_menu_create(
    cx: &mut InterfaceCx,
    prompt: &[u8],
    default_text: &[u8],
) -> Result<bool, StreamError> {
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_TEXT_EDIT)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, TEXT_EDIT_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(false);
    };
    if cx.engine.menu(menu).id() != MENU_ID_TEXT_EDIT {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(false);
    }
    menu_register_tile(cx, menu, Some(created))?;
    if !text_edit(cx, menu).tiles_28.iter().all(Option::is_some) {
        cx.host.warning(
            "Text Edit Menu Creation Failed... Are your menu and art resources up to date?",
        );
        return Ok(false);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let text = text_edit(cx, menu).tiles_28[0].expect("the three tiles");
    tile_set_string_trait(cx, created, TRAIT_USER0, prompt)?;
    tile_set_string_trait(cx, text, TRAIT_STRING, default_text)?;
    let wrap_width = x87::ftol(tile_numeric_trait(cx, text, TRAIT_WRAP_WIDTH));
    let shown = tile_string_trait(cx, text, TRAIT_STRING).unwrap_or_default();
    let virtual_width = ui_virtual_width(cx.settings);
    if let MenuClass::TextEdit(state) = &mut cx.engine.menu_mut(menu).class {
        let input = &mut state.input_state_34;
        input.set_max_pixel_width(wrap_width, virtual_width);
        input.set_text(&shown);
        input.set_enabled(true);
    }
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(true)
}
