//! HUDSubtitleMenu (1010): 005A8DF0's fields, 005A8DC0 SetField while `Data\Menus\Main\hud_subtitle_menu.xml` is built,
//! and 005A8E30, which builds it. Its callers, 0057ACC0 (a subtitle, 005A9980) and 0057ADD0 (a notice, 005A95C0), and the
//! menu's per-frame vt+0x2C (005A93B0) aren't ported; the fields' roles are read from them.

use recomp_core::reader::StreamError;

use super::interface_frame::{menu_destroy, menu_enable};
use super::interface_manager::{InterfaceManager, menu_register_tile};
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::{
    TRAIT_STRING, TRAIT_USER9, TRAIT_Y, tile_numeric_trait, tile_set_numeric_trait,
    tile_set_string_trait,
};
use super::tile_xml::tile_read_xml;

/// IDA: MenuID_HUDSubtitle.
pub const MENU_ID_HUD_SUBTITLE: u32 = 1010;
const HUD_SUBTITLE_XML: &str = "Data\\Menus\\Main\\hud_subtitle_menu.xml";
/// IDA: 00A36430 String2_3, the one-space string 005A8E30 and 005A8F30 store.
pub const ONE_SPACE_A36430: &[u8] = b" ";

/// IDA: 005A93B0's states of +0x38: 1 idle (005A8F30), 2 showing (005A9980) until +0x3C runs out, 3 waiting on the
/// sound +0x40 (006B7260), 4 lingering for fSubtitleLingerTime (00B3B3D0) before 005A8F30.
pub const SUBTITLE_STATE_IDLE: u8 = 1;

/// IDA: 005A8DF0 HUDSubtitleMenu_ctor (via 00587D20's 1010 case, FormHeap 0x44): Menu_Constructor, vtable A6C0DC, +0x2C,
/// +0x30, +0x28, +0x34 and +0x40 null, +0x3C 0.0, the byte +0x38 1. Completion: 100%.
#[derive(Clone, Debug, PartialEq)]
pub struct HudSubtitleMenu {
    /// +0x28: field 1, `hudsubtitle_text`.
    pub subtitle_text_28: Option<TileId>,
    /// +0x2C/+0x30: the queued notices, a PointerList head (item, next). 005A95C0 queues them and 005A93B0 shows and
    /// frees them, neither ported: the port holds none.
    pub notices_2c: Vec<HudSubtitleNotice>,
    /// +0x34: field 2, `hudsubtitle_notice`.
    pub notice_text_34: Option<TileId>,
    /// +0x38: the subtitle's state (`SUBTITLE_STATE_*`).
    pub subtitle_state_38: u8,
    /// +0x3C: the subtitle's seconds left in states 2 and 4.
    pub subtitle_seconds_3c: f32,
    /// +0x40: the subtitle's sound (005A9980's second argument), a host value; none until 005A9980 runs.
    pub subtitle_sound_40: Option<u32>,
}

/// IDA: an entry of HUDSubtitleMenu +0x2C: +0x00 its text (as 005A93B0 compares it with the notice tile's string).
/// Nothing ported makes one. Completion: the text only.
/// TODO: [ ] +0x08 seconds, +0x0C shown, +0x0E the sound path's length, +0x10 the icon path, +0x18 the sound path (005A95C0,
/// 005A93B0), when a notice is ported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HudSubtitleNotice {
    pub text_00: Vec<u8>,
}

impl Default for HudSubtitleMenu {
    fn default() -> Self {
        Self {
            subtitle_text_28: None,
            notices_2c: Vec::new(),
            notice_text_34: None,
            subtitle_state_38: SUBTITLE_STATE_IDLE,
            subtitle_seconds_3c: 0.0,
            subtitle_sound_40: None,
        }
    }
}

impl HudSubtitleMenu {
    /// IDA: 005A8DC0 HUDSubtitleMenu_SetField: id 1 to +0x28, id 2 to +0x34. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        match id {
            1 => self.subtitle_text_28 = Some(tile),
            2 => self.notice_text_34 = Some(tile),
            _ => {}
        }
    }
}

// IDA: N/A the menu's HUDSubtitleMenu part.
fn subtitle<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a HudSubtitleMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::HudSubtitle(subtitle) => subtitle,
        _ => unreachable!("a HUDSubtitleMenu"),
    }
}

/// IDA: 005A8E30 HUDSubtitleMenu_Create: an existing 1010 menu deleted (TileMenu vt+0, ported as `map_menu_create` does
/// it); Tile_ReadXML(hud_subtitle_menu.xml) under the menu root; its containing menu, else null; a menu that isn't 1010 is
/// deleted when it has a tile, and null answers; 00584880 registers the tile. With no field 1, DebugLog_Printf
/// (00404EC0) and no return: 005A8EC1 then reads the null tile's +0x18 and faults, a host stop here. Field 1's user9 takes
/// its y, fields 1 and 2 the string " " (String2_3), Menu_Enable(0), and the root answers. No depth is taken. Completion:
/// 100%.
/// # Errors
/// A stop in the XML or the menu calls, or a missing field 1 (005A8EC1).
pub fn hud_subtitle_menu_create(cx: &mut InterfaceCx) -> Result<Option<TileId>, StreamError> {
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_HUD_SUBTITLE)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, HUD_SUBTITLE_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != MENU_ID_HUD_SUBTITLE {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(None);
    }
    menu_register_tile(cx, menu, Some(created))?;
    let Some(text) = subtitle(cx, menu).subtitle_text_28 else {
        cx.host.debug_log(
            "HUD-Subtitle Menu Creation Failed... Are your menu and art resources up to date?",
        );
        return Err(StreamError(
            "005A8EC1: HUDSubtitleMenu +0x28 is null, and the original reads it",
        ));
    };
    let y = tile_numeric_trait(cx, text, TRAIT_Y);
    tile_set_numeric_trait(cx, text, TRAIT_USER9, y)?;
    tile_set_string_trait(cx, text, TRAIT_STRING, ONE_SPACE_A36430)?;
    // 005A8EF1: +0x34 isn't checked; a null one faults in Tile_SetStringTrait's lookup as well.
    let Some(notice) = subtitle(cx, menu).notice_text_34 else {
        return Err(StreamError(
            "005A8EF1: HUDSubtitleMenu +0x34 is null, and the original reads it",
        ));
    };
    tile_set_string_trait(cx, notice, TRAIT_STRING, ONE_SPACE_A36430)?;
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(Some(created))
}
