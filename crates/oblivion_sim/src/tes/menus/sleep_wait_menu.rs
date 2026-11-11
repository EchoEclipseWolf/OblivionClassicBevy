//! SleepWaitMenu (1012): its fields as 005D6870 SleepWaitMenu_ctor sets them, 005D68D0 SetField while
//! `Data\Menus\sleep_wait_menu.xml` is built, and 0057B420 (its body in the chunk 005D6D20), which builds it for sleeping or
//! waiting. Its callers (`PlayerCharacter_UpdateFrame` 00671620, 004AE8C0 a bed's activation) and its click, mouseover and
//! per-frame virtuals (005D6A80, 005D6C90, 005D6CD0, 005D6BC0) aren't ported; the statics' roles are read from 005D6A80.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use crate::tes::global::{game_hour, map_date, time_globals_day_name};
use crate::tes::interface_frame::{menu_destroy, menu_enable};
use crate::tes::interface_manager::{
    InterfaceManager, menu_register_tile, menu_update_cursor_depth,
};
use crate::tes::player::PlayerCharacter;
use crate::tes::player::the_player;
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use crate::tes::tile_values::{
    TRAIT_DEPTH, TRAIT_STACKING, TRAIT_STRING, TRAIT_USER0, TRAIT_USER1, TRAIT_USER2, TRAIT_USER4,
    TRAIT_USER5, TRAIT_USER9, tile_numeric_trait, tile_set_numeric_trait, tile_set_string_trait,
};
use crate::tes::tile_xml::tile_read_xml;

/// IDA: MenuID_SleepWait.
pub const MENU_ID_SLEEP_WAIT: u32 = 1012;
const SLEEP_WAIT_XML: &str = "Data\\Menus\\sleep_wait_menu.xml";
/// IDA: 00B38AC8 sWait, the GMST 009F20FF registers ("Wait").
const S_WAIT_B38AC8: u32 = 0x00B3_8AC8;

/// IDA: the statics 0057B420 and 005D6A80 share. Completion: 100% of the values 0057B420 writes.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct SleepWaitStatics {
    /// 00B14778: the hours last chosen (005D6AFF stores the slider's ftol), 1 in the image; the slider starts there.
    pub last_hours_b14778: i32,
    /// 00B3B728: a rest under way (005D6B2A sets it when the rest begins); 0057B420 clears it.
    pub resting_b3b728: u8,
    /// 00B3B729: cleared by 0057B420; 005D70F7 and 005D7100 read and write it in the per-frame arm, not ported.
    pub byte_b3b729: u8,
    /// 00B3B72C: cleared by 0057B420; 005D69A5, 005D69E9 and 005D70D1 use it, not ported.
    pub byte_b3b72c: u8,
    /// 00B3B730: the hours asked for: the player's +0x590 at creation, the slider's at 005D6AEB.
    pub hours_b3b730: i32,
}

impl Default for SleepWaitStatics {
    fn default() -> Self {
        Self {
            last_hours_b14778: 1,
            resting_b3b728: 0,
            byte_b3b729: 0,
            byte_b3b72c: 0,
            hours_b3b730: 0,
        }
    }
}

/// IDA: 005D6870 SleepWaitMenu_ctor (via 00587D20's 1012 case, FormHeap 0x50): Menu_Constructor, the eight tile
/// pointers +0x28..+0x44 null, vtable, the byte +0x4C 1. +0x48 isn't stored (FormHeap's bytes; nothing ported reads it).
/// Completion: 100%.
#[derive(Clone, Debug, PartialEq)]
pub struct SleepWaitMenu {
    /// +0x28.. fields 3 (`sleep_scroll`, the hours), 4 (its marker, by user6), 5 (`sleep_button_sleep_wait`), 6
    /// (`sleep_button_cancel`), 8 (`game_info_1`, the time), 9 (`game_info_2`, the date), 10 (`sleep_scroll_disable`) and
    /// 11 (`sleep_scroll_marker`).
    pub tiles_28: [Option<TileId>; 8],
    /// +0x4C: sleeping (1) rather than waiting (0); 005D6AF9 copies it to the player's +0x594.
    pub sleeping_4c: u8,
}

impl Default for SleepWaitMenu {
    fn default() -> Self {
        Self {
            tiles_28: [None; 8],
            sleeping_4c: 1,
        }
    }
}

/// IDA: 005D68D0's ids, in +0x28..+0x44 order.
const FIELD_IDS: [i32; 8] = [3, 4, 5, 6, 8, 9, 10, 11];

impl SleepWaitMenu {
    /// IDA: 005D68D0 SleepWaitMenu_SetField: ids 3, 4, 5, 6, 8, 9, 10, 11 to +0x28..+0x44. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        if let Some(slot) = FIELD_IDS.iter().position(|&known| known == id) {
            self.tiles_28[slot] = Some(tile);
        }
    }

    /// IDA: 005D68A0 SleepWaitMenu_HasRequiredTiles: +0x28..+0x3C nonnull (fields 3, 4, 5, 6, 8, 9). Completion: 100%.
    #[must_use]
    pub fn has_required_tiles(&self) -> bool {
        self.tiles_28[..6].iter().all(Option::is_some)
    }

    /// IDA: N/A the tile at +0x28 + 4 * slot.
    #[must_use]
    pub fn field(&self, id: i32) -> Option<TileId> {
        let slot = FIELD_IDS.iter().position(|&known| known == id)?;
        self.tiles_28[slot]
    }
}

// IDA: N/A the menu's SleepWaitMenu part.
fn sleep_wait<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a SleepWaitMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::SleepWait(sleep_wait) => sleep_wait,
        _ => unreachable!("a SleepWaitMenu"),
    }
}

/// IDA: 0057B420 SleepWaitMenu_Create(sleeping), with its body in the chunk 005D6D20: with a manager, its cursor (+0x1C)
/// and menu root (+0x68), and the root's user0 exactly 2.0 (`fcomp`, `test ah,44h; jp`), an existing SleepWaitMenu is
/// deleted (its vt+0); 00B3B728 = 0; a 1012 menu still there answers false. Then the depth (00588890 → 00584980),
/// Tile_ReadXML(sleep_wait_menu.xml) under the root, its containing menu, else false; a menu that isn't 1012 is deleted
/// when it has a tile, and false; 00584880 registers the tile; without the six tiles (005D68A0) the warning and false (the
/// menu stays). A stacking type 6006 or 102 takes the depth. Field 4's user9 -1e9 then 0; field 3's user5 00B14778 (fild)
/// then 0, user1 1, user2 24, user4 6; 00B3B730 the player's +0x590; 00B3B72C 0. Waiting (0): field 5's user0 the sWait
/// text and +0x4C 0, the root's user0 1; sleeping: the root's user0 2. The time: 00402BD0's hour as a float, its __ftol2
/// as a signed byte (`movsx eax,al`), the fraction (`fild`, `fsub`) times 60.0 through __ftol2 as a signed byte; the shown
/// hour 12 below 1, less 12 above 12; "pm" unless the byte hour is below 12; "%s %d:%02d %s" of 00402C90's day name, the
/// hour, the minutes and the suffix as field 8's string; 00402E50's date as field 9's; 00B3B729 = 0;
/// InterfaceManager_PlayUISound(0xB); Menu_Enable(0); true. Completion: 100%.
/// # Errors
/// No player at 005D6EEA, where the original reads null's +0x590; a stop in the XML or the menu calls; a month outside
/// 00402E50's table.
pub fn sleep_wait_menu_create(cx: &mut InterfaceCx, sleeping: bool) -> Result<bool, GameError> {
    let Some(manager) = cx.world.get_resource::<InterfaceManager>() else {
        return Ok(false);
    };
    let (Some(_), Some(root)) = (manager.cursor_01c, manager.menu_root_068) else {
        return Ok(false);
    };
    if tile_numeric_trait(cx, root, TRAIT_USER0) != 2.0 {
        return Ok(false);
    }
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_SLEEP_WAIT)
        && let Some(menu) = cx.engine.containing_menu(old)
        && matches!(cx.engine.menu(menu).class, MenuClass::SleepWait(_))
    {
        menu_destroy(cx, menu)?;
    }
    cx.world
        .get_resource_or_init::<SleepWaitStatics>()
        .resting_b3b728 = 0;
    if cx
        .engine
        .tile_menu_by_id(MENU_ID_SLEEP_WAIT)
        .and_then(|tile| cx.engine.containing_menu(tile))
        .is_some()
    {
        return Ok(false);
    }
    let depth = menu_update_cursor_depth(cx)?;
    let created = tile_read_xml(cx, Some(root), SLEEP_WAIT_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(false);
    };
    if cx.engine.menu(menu).id() != MENU_ID_SLEEP_WAIT {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(false);
    }
    menu_register_tile(cx, menu, Some(created))?;
    if !sleep_wait(cx, menu).has_required_tiles() {
        cx.host
            .warning("Sleep Menu Creation Failed... Are your menu and art resources up to date?");
        return Ok(false);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let fields = sleep_wait(cx, menu).clone();
    let field = |id| fields.field(id).expect("the six tiles");
    tile_set_numeric_trait(cx, field(4), TRAIT_USER9, -1_000_000_000.0)?;
    tile_set_numeric_trait(cx, field(4), TRAIT_USER9, 0.0)?;
    let last_hours = cx
        .world
        .get_resource_or_init::<SleepWaitStatics>()
        .last_hours_b14778;
    // 005D6E76 fild: an int above 2^24 rounds here.
    tile_set_numeric_trait(cx, field(3), TRAIT_USER5, last_hours as f32)?;
    tile_set_numeric_trait(cx, field(3), TRAIT_USER5, 0.0)?;
    tile_set_numeric_trait(cx, field(3), TRAIT_USER1, 1.0)?;
    tile_set_numeric_trait(cx, field(3), TRAIT_USER2, 24.0)?;
    tile_set_numeric_trait(cx, field(3), TRAIT_USER4, 6.0)?;
    let sleep_hours = the_player(cx.world)
        .and_then(|player| cx.world.get::<PlayerCharacter>(player))
        .map(|player| player.pc_sleep_hours_590)
        .ok_or(GameError::unclassified(
            0x005D6EEA,
            "005D6EEA: no player, and the original reads its +0x590",
        ))?;
    {
        let mut statics = cx.world.resource_mut::<SleepWaitStatics>();
        statics.hours_b3b730 = sleep_hours;
        statics.byte_b3b72c = 0;
    }
    let mode = if sleeping {
        2.0
    } else {
        let wait = cx
            .world
            .get_resource::<crate::tes::game_settings::GameSettingCollection>()
            .and_then(|settings| settings.text_at(S_WAIT_B38AC8))
            .unwrap_or_default();
        tile_set_string_trait(cx, field(5), TRAIT_USER0, &wait)?;
        if let MenuClass::SleepWait(state) = &mut cx.engine.menu_mut(menu).class {
            state.sleeping_4c = 0;
        }
        1.0
    };
    tile_set_numeric_trait(cx, created, TRAIT_USER0, mode)?;
    let time = sleep_wait_time_text(cx.world);
    tile_set_string_trait(cx, field(8), TRAIT_STRING, &time)?;
    let date = map_date(cx)?;
    tile_set_string_trait(cx, field(9), TRAIT_STRING, &date)?;
    cx.world.resource_mut::<SleepWaitStatics>().byte_b3b729 = 0;
    cx.host.play_ui_sound(0xB);
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(true)
}

/// IDA: 005D6F3D..005D6FD4: the time line of 0057B420, at the main thread's 24-bit precision (each `fsub`/`fmul` rounds
/// to a float). The hour's __ftol2 and the minutes' are kept as their low signed bytes (`movsx eax,al`, the byte at
/// var_24+3), the shown hour's as well (`movsx eax,cl`). A null day name prints "(null)", as the CRT's `%s` does.
/// Completion: 100%.
#[must_use]
pub fn sleep_wait_time_text(world: &World) -> Vec<u8> {
    let hour = game_hour(world);
    let hours = i32::from(x87::ftol(hour) as i8);
    let hours_float = hours as f32;
    let minutes = i32::from(x87::ftol((hour - hours_float) * 60.0) as i8);
    // 005D6F74 fld1; fcomp: 1.0 above the hour gives 12; 005D6F8D fcom: 12.0 below it subtracts 12.
    let shown = if 1.0 > hours_float {
        12
    } else if 12.0 < hours_float {
        i32::from(x87::ftol(hours_float - 12.0) as i8)
    } else {
        i32::from(x87::ftol(hours_float) as i8)
    };
    let suffix = if 12.0 <= hours_float { "pm" } else { "am" };
    let mut text = time_globals_day_name(world).unwrap_or_else(|| b"(null)".to_vec());
    text.extend_from_slice(format!(" {shown}:{minutes:02} {suffix}").as_bytes());
    text
}
