//! HUDSubtitleMenu (1010): 005A8DF0's fields, 005A8DC0 SetField while `Data\Menus\Main\hud_subtitle_menu.xml` is built,
//! and 005A8E30, which builds it. Its callers, 0057ACC0 (a subtitle, 005A9980) and 0057ADD0 (a notice, 005A95C0), and the
//! menu's per-frame vt+0x2C (005A93B0) aren't ported; the fields' roles are read from them.

use oblivion_core::error::GameError;

use crate::tes::interface_frame::{menu_destroy, menu_enable};
use crate::tes::interface_manager::{InterfaceManager, menu_register_tile};
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use crate::tes::tile_values::{
    TRAIT_STRING, TRAIT_USER9, TRAIT_Y, tile_numeric_trait, tile_set_numeric_trait,
    tile_set_string_trait,
};
use crate::tes::tile_xml::tile_read_xml;

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
pub fn hud_subtitle_menu_create(cx: &mut InterfaceCx) -> Result<Option<TileId>, GameError> {
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
        return Err(GameError::unclassified(
            0x005A8EC1,
            "005A8EC1: HUDSubtitleMenu +0x28 is null, and the original reads it",
        ));
    };
    let y = tile_numeric_trait(cx, text, TRAIT_Y);
    tile_set_numeric_trait(cx, text, TRAIT_USER9, y)?;
    tile_set_string_trait(cx, text, TRAIT_STRING, ONE_SPACE_A36430)?;
    // 005A8EF1: +0x34 isn't checked; a null one faults in Tile_SetStringTrait's lookup as well.
    let Some(notice) = subtitle(cx, menu).notice_text_34 else {
        return Err(GameError::unclassified(
            0x005A8EF1,
            "005A8EF1: HUDSubtitleMenu +0x34 is null, and the original reads it",
        ));
    };
    tile_set_string_trait(cx, notice, TRAIT_STRING, ONE_SPACE_A36430)?;
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(Some(created))
}

/// IDA: 005A93B0 HUDSubtitleMenu vt+0x2C, each frame from `0058399D`: the 1010 menu by its tile; with neither the Pause (1035)
/// nor the Loading (1007) menu visible (005790E0), +0x28's visible is 1.0 when in menu mode with no Dialog tile (1009) or a
/// Dialog tile whose visible is 2.0, or with no player or a dead one (vt IsDead(0), read as +0xB0 nonzero), else 2.0. The +0x38 state machine: 2 and 4 count +0x3C
/// down by the frame time, 3 waits on the sound (both stops: only 005A9980, unported, leaves them), 1 does nothing. With the
/// RaceSexMenu (1036) visible, +0x34's visible = 1.0; otherwise the queued notices (none: 005A95C0 isn't ported) leave it.
/// Completion: 100% of the idle state with no notices.
/// TODO: [ ] States 2 to 4 and the notice queue's walk, with 005A9980 and 005A95C0.
/// # Errors
/// An unported state or notice.
pub fn hud_subtitle_update_frame(cx: &mut InterfaceCx, menu: MenuId) -> Result<(), GameError> {
    use crate::tes::interface_manager::menu_is_visible;
    if cx.engine.tile_menu_by_id(MENU_ID_HUD_SUBTITLE).is_none() {
        return Ok(());
    }
    if menu_is_visible(cx, 1035) || menu_is_visible(cx, 1007) {
        return Ok(());
    }
    let dialog = cx.engine.tile_menu_by_id(1009);
    let menu_mode = crate::tes::interface_manager::is_menu_mode(cx.world);
    let player = crate::tes::player::the_player(cx.world);
    let dead = player.is_some_and(|player| {
        cx.world
            .get::<crate::tes::actor::Actor>(player)
            .is_some_and(|actor| actor.dead_state_b0 != 0)
    });
    let hidden = (menu_mode
        && dialog.is_none_or(|tile| {
            tile_numeric_trait(cx, tile, crate::tes::tile_values::TRAIT_VISIBLE) == 2.0
        }))
        || player.is_none()
        || dead;
    let (text, notice, state, queued) = {
        let data = subtitle(cx, menu);
        (
            data.subtitle_text_28,
            data.notice_text_34,
            data.subtitle_state_38,
            !data.notices_2c.is_empty(),
        )
    };
    let text = text.ok_or(GameError::unclassified(
        0x005A9431,
        "005A9431: a null +0x28 faults",
    ))?;
    tile_set_numeric_trait(
        cx,
        text,
        crate::tes::tile_values::TRAIT_VISIBLE,
        if hidden { 1.0 } else { 2.0 },
    )?;
    if matches!(state, 2..=4) {
        return Err(GameError::unclassified(
            0x005A9448,
            "005A9448: a subtitle's states 2 to 4",
        ));
    }
    if menu_is_visible(cx, 1036) {
        let notice = notice.ok_or(GameError::unclassified(
            0x005A94C6,
            "005A94C6: a null +0x34 faults",
        ))?;
        return tile_set_numeric_trait(cx, notice, crate::tes::tile_values::TRAIT_VISIBLE, 1.0);
    }
    if queued {
        return Err(GameError::unclassified(
            0x005A94DD,
            "005A94DD: a queued notice",
        ));
    }
    Ok(())
}

/// IDA: 005A93B0 HUDSubtitleMenu vt+0x2C. Completion: `hud_subtitle_update_frame`'s.
/// # Errors
/// A stop in it.
pub fn hud_subtitle_frame(
    cx: &mut InterfaceCx,
    menu: MenuId,
    _frame_time: f32,
) -> Result<(), GameError> {
    crate::tes::hud_subtitle_menu::hud_subtitle_update_frame(cx, menu)
}
