//! TextEditMenu (1051): 005879C0 TextEditMenu_ctor's fields with its MenuTextInputState (0057FE70), 00587AE0 SetField
//! while `Data\Menus\Dialog\TextEditMenu.xml` is built, and 005DCF20, which builds it with a prompt and a default text
//! (00583DD0, 0057FF20 and 0057DD90 on the input state). Its caller (005976B0) and its click (005DD1B0), key input and
//! per-frame virtuals (005DD120, 005DD160) aren't ported.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use crate::tes::interface_frame::{menu_destroy, menu_enable};
use crate::tes::interface_manager::{
    InterfaceManager, menu_register_tile, menu_update_cursor_depth, ui_virtual_width,
};
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use crate::tes::tile_values::{
    TRAIT_DEPTH, TRAIT_STACKING, TRAIT_STRING, TRAIT_USER0, TRAIT_WRAP_WIDTH, tile_numeric_trait,
    tile_set_numeric_trait, tile_set_string_trait, tile_string_trait,
};
use crate::tes::tile_xml::tile_read_xml;

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
    /// +0x08 the caret's blink: '|' while set, 0x7F while clear (00580120); 0057DDE0 flips it.
    pub caret_shown_08: u8,
    /// +0x0C: -1 from the ctor.
    pub max_pixel_width_0c: i32,
    /// +0x10 g_timeInfo +0x10 at the caret's last flip (0057DDE0).
    pub caret_flip_ticks_10: u32,
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
            caret_shown_08: 0,
            max_pixel_width_0c: -1,
            caret_flip_ticks_10: 0,
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

    /// IDA: 0057D2D0 MenuTextInputState_SetFont(font) (`retn 4`): +0x14 = font - 1 for a font of 1..3 (`(font - 1) <= 2`,
    /// unsigned), else 0. Completion: 100%.
    pub fn set_font(&mut self, font: i32) {
        let index = font.wrapping_sub(1) as u32;
        self.font_index_14 = if index > 2 { 0 } else { index };
    }

    /// IDA: 0057DDE0 MenuTextInputState_UpdateCaretBlink: past 500 ticks (`ja`, unsigned) since +0x10, +0x10 = g_timeInfo
    /// +0x10 and +0x08 flips. Completion: 100%.
    pub fn update_caret_blink(&mut self, time_info_10: u32) {
        if time_info_10.wrapping_sub(self.caret_flip_ticks_10) > 0x1F4 {
            self.caret_flip_ticks_10 = time_info_10;
            self.caret_shown_08 = u8::from(self.caret_shown_08 == 0);
        }
    }

    /// IDA: 00580120 MenuTextInputState_GetDisplayText: +0x18 copied to a 1024-byte buffer; for each index 0..=length
    /// (`jle`, the terminator included), an enabled state puts the caret ('|' with +0x08 set, else 0x7F) before the byte at
    /// +0x04 (signed); +0x20 = the result up to its terminator, and an empty one becomes " " (00A36430). Completion: 100%;
    /// the original's 1024-byte buffers overflow past 1022 bytes, which the host doesn't reproduce.
    pub fn display_text(&mut self) -> Vec<u8> {
        let length = self.editable_text_18.len();
        let caret = self.caret_index_04 as i32;
        let mut text = Vec::with_capacity(length + 1);
        for index in 0..length {
            if self.enabled_00 != 0 && index as i32 == caret {
                text.push(if self.caret_shown_08 != 0 { b'|' } else { 0x7F });
            }
            text.push(self.editable_text_18[index]);
        }
        if self.enabled_00 != 0 && length as i32 == caret {
            text.push(if self.caret_shown_08 != 0 { b'|' } else { 0x7F });
        }
        if let Some(end) = text.iter().position(|&byte| byte == 0) {
            text.truncate(end);
        }
        if text.is_empty() {
            text.push(b' ');
        }
        self.mirrored_text_20.clone_from(&text);
        text
    }

    /// IDA: 0057FF50 MenuTextInputState_HandleKey(key) (`retn 4`), when +0x00 is set: the text (+0x18, its length word or
    /// strlen at 0xFFFF) into a buffer, then by key: Backspace (0x80000000) with the caret past 0 shifts the text left over
    /// caret - 1 (when that's inside it), caret - 1, the last byte cut, stored; Left (0x80000001) caret - 1 past 0; Right
    /// caret + 1 short of the length; Home 0; End the length; Delete (0x80000007) inside the text shifts it left over the
    /// caret, cuts the last byte and stores; Enter (0x80000008) clears +0x00; PageUp and PageDown (0x80000009, 0x8000000A)
    /// nothing; anything else, Up and Down (0x80000003, 0x80000004) included, inserts the key's low byte at the caret
    /// (signed compares), and with a width limit (+0x0C not -1) only when `fits` (0057DE00) passes it; then caret + 1 and
    /// stored. Completion: 100%; the original's 1024-byte buffer overflows past 1022 bytes, which the host doesn't.
    pub fn handle_key(&mut self, key: u32, fits: impl FnOnce(&Self, &[u8]) -> bool) {
        if self.enabled_00 == 0 {
            return;
        }
        let mut text = self.editable_text_18.clone();
        let length = text.len() as i32;
        let caret = self.caret_index_04 as i32;
        match key {
            0x8000_0000 => {
                if caret > 0 {
                    let at = caret - 1;
                    if at < length {
                        text.remove(at as usize);
                        text.push(0);
                    }
                    self.caret_index_04 = at as u32;
                    if length > 0 {
                        text[(length - 1) as usize] = 0;
                    }
                    self.store_text(&text);
                }
            }
            0x8000_0001 => {
                if caret > 0 {
                    self.caret_index_04 = (caret - 1) as u32;
                }
            }
            0x8000_0002 => {
                if caret < length {
                    self.caret_index_04 = (caret + 1) as u32;
                }
            }
            0x8000_0005 => self.caret_index_04 = 0,
            0x8000_0006 => self.caret_index_04 = length as u32,
            0x8000_0007 => {
                if caret < length {
                    text.remove(caret as usize);
                    text.push(0);
                    text[(length - 1) as usize] = 0;
                    self.store_text(&text);
                }
            }
            0x8000_0008 => self.enabled_00 = 0,
            0x8000_0009 | 0x8000_000A => {}
            _ => {
                let at = if length > caret {
                    caret as usize
                } else {
                    length as usize
                };
                text.insert(at, key as u8);
                if self.max_pixel_width_0c == -1 || fits(self, &text) {
                    self.caret_index_04 = self.caret_index_04.wrapping_add(1);
                    self.store_text(&text);
                }
            }
        }
    }

    // IDA: N/A 004028D0 BSStringT_Set on +0x18 from the buffer, up to its terminator.
    fn store_text(&mut self, text: &[u8]) {
        let end = text
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(text.len());
        self.editable_text_18 = text[..end].to_vec();
    }

    /// IDA: 0057DE00 MenuTextInputState_FitsWidth(candidate) (`retn 4`): 005739C0 on FontManager's font +0x14 (max -1, wrap
    /// width 2147483648.0 (flt_A68D9C), no alternate height, start 0); true when `ftol`(width) <= +0x0C (signed).
    /// Completion: 100%; a missing font measures 0, where the original faults.
    #[must_use]
    pub fn fits_width(
        &self,
        fonts: Option<&crate::tes::fonts::FontManager>,
        candidate: &[u8],
    ) -> bool {
        let width = fonts
            .and_then(|fonts| fonts.font_infos_00.get(self.font_index_14 as usize))
            .and_then(Option::as_ref)
            .map_or(0.0, |font| {
                crate::tes::interface_text::font_measure_text(
                    font,
                    candidate,
                    -1,
                    2_147_483_648.0,
                    false,
                    0,
                )
                .1
            });
        x87::ftol(width) <= self.max_pixel_width_0c
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
) -> Result<bool, GameError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    // 00580120: the caret goes before the byte at +0x04 while enabled ('|' or 0x7F by +0x08), at the end too (the loop
    // includes the terminator); disabled shows the text alone, and an empty result shows " ".
    #[test]
    fn the_display_text_places_the_caret() {
        let mut input = MenuTextInputState::default();
        assert_eq!(input.display_text(), b" ");
        input.set_text(b"Abc");
        input.set_enabled(true);
        assert_eq!(input.caret_index_04, 3);
        assert_eq!(input.display_text(), b"Abc\x7F");
        input.caret_shown_08 = 1;
        input.caret_index_04 = 1;
        assert_eq!(input.display_text(), b"A|bc");
        assert_eq!(input.mirrored_text_20, b"A|bc");
        input.set_enabled(false);
        assert_eq!(input.display_text(), b"Abc");
    }

    // 0057FF50's arms with no width limit: inserts at the caret, Backspace and Delete around it, the caret moves, Up's
    // low byte inserted (it isn't a case), PageUp nothing, Enter ends the field and a disabled state ignores keys.
    #[test]
    fn handle_key_edits_as_the_exe() {
        let mut input = MenuTextInputState::default();
        input.set_enabled(true);
        let never = |_: &MenuTextInputState, _: &[u8]| false;
        for &key in b"Abd" {
            input.handle_key(u32::from(key), never);
        }
        assert_eq!(
            (input.editable_text_18.as_slice(), input.caret_index_04),
            (&b"Abd"[..], 3)
        );
        input.handle_key(0x8000_0001, never);
        input.handle_key(u32::from(b'c'), never);
        assert_eq!(
            (input.editable_text_18.as_slice(), input.caret_index_04),
            (&b"Abcd"[..], 3)
        );
        input.handle_key(0x8000_0000, never);
        assert_eq!(
            (input.editable_text_18.as_slice(), input.caret_index_04),
            (&b"Abd"[..], 2)
        );
        input.handle_key(0x8000_0005, never);
        input.handle_key(0x8000_0007, never);
        assert_eq!(
            (input.editable_text_18.as_slice(), input.caret_index_04),
            (&b"bd"[..], 0)
        );
        input.handle_key(0x8000_0006, never);
        assert_eq!(input.caret_index_04, 2);
        input.handle_key(0x8000_0002, never);
        assert_eq!(input.caret_index_04, 2, "Right stops at the end");
        input.handle_key(0x8000_0003, never);
        assert_eq!(input.editable_text_18, b"bd\x03");
        input.handle_key(0x8000_0009, never);
        assert_eq!(input.editable_text_18, b"bd\x03");
        input.handle_key(0x8000_0008, never);
        assert_eq!(input.enabled_00, 0);
        input.handle_key(u32::from(b'x'), never);
        assert_eq!(input.editable_text_18, b"bd\x03");
    }

    // With a width limit, an insert that fails 0057DE00 is dropped: the caret and text stay.
    #[test]
    fn handle_key_drops_what_does_not_fit() {
        let mut input = MenuTextInputState::default();
        input.set_max_pixel_width(105, 0.0);
        input.set_enabled(true);
        input.handle_key(u32::from(b'a'), |_, text| text.len() <= 1);
        input.handle_key(u32::from(b'b'), |_, text| text.len() <= 1);
        assert_eq!(
            (input.editable_text_18.as_slice(), input.caret_index_04),
            (&b"a"[..], 1)
        );
    }

    // 0057DDE0 flips past 500 ticks (unsigned), and 0057D2D0 maps fonts 1..3 to 0..2, anything else to 0.
    #[test]
    fn the_caret_blinks_and_the_font_maps() {
        let mut input = MenuTextInputState::default();
        input.update_caret_blink(500);
        assert_eq!((input.caret_shown_08, input.caret_flip_ticks_10), (0, 0));
        input.update_caret_blink(501);
        assert_eq!((input.caret_shown_08, input.caret_flip_ticks_10), (1, 501));
        input.update_caret_blink(1002);
        assert_eq!(input.caret_shown_08, 0);
        for (font, index) in [(0, 0), (1, 0), (2, 1), (3, 2), (4, 0), (-1, 0)] {
            input.set_font(font);
            assert_eq!(input.font_index_14, index, "font {font}");
        }
    }
}
