//! GameplayMenu (1020): 005A3310 GameplayMenu_ctor's fields, 005A3380 SetField while
//! `Data\Menus\Options\gameplay_menu.xml` is built, and 005A3540, which builds it: the difficulty slider and the six
//! on/off buttons. Its caller (the Options menu's click 005BD880) and its click (005A3860, which writes the settings
//! back), mouseover (005A3B90, 005A3BB0) and vt+0x20 (005A3460) aren't ported.

use recomp_core::reader::StreamError;

use super::ai_conditions::the_player;
use super::game_settings::GameSettingCollection;
use super::interface_frame::{menu_destroy, menu_enable};
use super::interface_manager::{InterfaceManager, menu_register_tile, menu_update_cursor_depth};
use super::player::PlayerCharacter;
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::{
    TRAIT_DEPTH, TRAIT_STACKING, TRAIT_STRING, TRAIT_USER1, TRAIT_USER2, TRAIT_USER3, TRAIT_USER4,
    TRAIT_USER5, tile_numeric_trait, tile_set_numeric_trait, tile_set_string_trait,
};
use super::tile_xml::tile_read_xml;

/// IDA: MenuID_Gameplay.
pub const MENU_ID_GAMEPLAY: u32 = 1020;
const GAMEPLAY_XML: &str = "Data\\Menus\\Options\\gameplay_menu.xml";
/// IDA: 00B37B70 fDifficultyMinValue, 00B37B78 fDifficultyMaxValue (GMSTs).
const F_DIFFICULTY_MIN_B37B70: u32 = 0x00B3_7B70;
const F_DIFFICULTY_MAX_B37B78: u32 = 0x00B3_7B78;
/// IDA: 00B38DA0 sOnButtonText, 00B38DA8 sOffButtonText.
const S_ON_B38DA0: u32 = 0x00B3_8DA0;
const S_OFF_B38DA8: u32 = 0x00B3_8DA8;

/// IDA: 005A3310 GameplayMenu_ctor (via 00587D20's 1020 case, FormHeap 0x4C): Menu_Constructor, the nine tile pointers
/// +0x28..+0x48 null, vtable. Completion: 100%.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameplayMenu {
    /// +0x28.. fields 1..8 and 10: `gameplay_difficulty_slider`, its marker (2, by the slider's user6), the general and
    /// dialogue subtitles, crosshair, rest, wait and travel save buttons (3..8), `gameplay_return_button` (10).
    /// `gameplay_defaults_button` (9) has no slot.
    pub tiles_28: [Option<TileId>; 9],
}

/// IDA: 005A3380's ids, in +0x28..+0x48 order.
const FIELD_IDS: [i32; 9] = [1, 2, 3, 4, 5, 6, 7, 8, 10];

impl GameplayMenu {
    /// IDA: 005A3380 GameplayMenu_SetField: ids 1..8 to +0x28..+0x44, 10 to +0x48. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        if let Some(slot) = FIELD_IDS.iter().position(|&known| known == id) {
            self.tiles_28[slot] = Some(tile);
        }
    }

    /// IDA: 005A3340 GameplayMenu_HasRequiredTiles: all nine nonnull. Completion: 100%.
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

// IDA: N/A the menu's GameplayMenu part.
fn gameplay<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a GameplayMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::Gameplay(gameplay) => gameplay,
        _ => unreachable!("a GameplayMenu"),
    }
}

/// IDA: 005A3540 GameplayMenu_Create: an existing 1020 menu deleted (TileMenu vt+0, ported as `map_menu_create` does it);
/// the depth (00588890 → 00584980); Tile_ReadXML(gameplay_menu.xml) under the root; its containing menu, else false; a
/// menu that isn't 1020 is deleted when it has a tile, and false; 00584880 registers the tile; without the nine tiles
/// (005A3340) the warning and false (the menu stays). A stacking type 6006 or 102 takes the depth. Field 1: user1
/// fDifficultyMinValue, user2 fDifficultyMaxValue, user3 (max - min) / 100.0 and user4 (max - min) * 0.25 (each at the
/// main thread's 24 bits and stored as a float), user5 the player's +0x734 then 0. Then the string of fields 4, 3, 5, 6,
/// 7, 8 is sOnButtonText or sOffButtonText by the low byte of bDialogueSubtitles, bGeneralSubtitles, bCrossHair,
/// bSaveOnRest, bSaveOnWait and bSaveOnTravel. Menu_Enable(0), true. Completion: 100%.
/// # Errors
/// No player at 005A36D6, where the original reads null's +0x734; a stop in the XML or the menu calls.
pub fn gameplay_menu_create(cx: &mut InterfaceCx) -> Result<bool, StreamError> {
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_GAMEPLAY)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, GAMEPLAY_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(false);
    };
    if cx.engine.menu(menu).id() != MENU_ID_GAMEPLAY {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(false);
    }
    menu_register_tile(cx, menu, Some(created))?;
    if !gameplay(cx, menu).has_required_tiles() {
        cx.host.warning(
            "Gameplay Menu Creation Failed... Are your menu and art resources up to date?",
        );
        return Ok(false);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let fields = gameplay(cx, menu).clone();
    let field = |id| fields.field(id).expect("the nine tiles");
    let (min, max, on, off) = {
        let settings = cx.world.get_resource::<GameSettingCollection>();
        let float = |address| {
            settings
                .and_then(|settings| settings.raw_at(address))
                .map_or(0.0, f32::from_bits)
        };
        let text = |address| {
            settings
                .and_then(|settings| settings.text_at(address))
                .unwrap_or_default()
        };
        (
            float(F_DIFFICULTY_MIN_B37B70),
            float(F_DIFFICULTY_MAX_B37B78),
            text(S_ON_B38DA0),
            text(S_OFF_B38DA8),
        )
    };
    let slider = field(1);
    let range = max - min;
    tile_set_numeric_trait(cx, slider, TRAIT_USER1, min)?;
    tile_set_numeric_trait(cx, slider, TRAIT_USER2, max)?;
    tile_set_numeric_trait(cx, slider, TRAIT_USER3, range / 100.0)?;
    tile_set_numeric_trait(cx, slider, TRAIT_USER4, range * 0.25)?;
    let difficulty = the_player(cx.world)
        .and_then(|player| cx.world.get::<PlayerCharacter>(player))
        .map(|player| player.game_difficulty_level_734)
        .ok_or(StreamError(
            "005A36D6: no player, and the original reads its +0x734",
        ))?;
    tile_set_numeric_trait(cx, slider, TRAIT_USER5, difficulty)?;
    tile_set_numeric_trait(cx, slider, TRAIT_USER5, 0.0)?;
    let settings = cx.settings;
    for (id, value) in [
        (4, settings.dialogue_subtitles_b13200),
        (3, settings.general_subtitles_b13208),
        (5, settings.cross_hair_b13210),
        (6, settings.save_on_rest_b13218),
        (7, settings.save_on_wait_b13220),
        (8, settings.save_on_travel_b13228),
    ] {
        let text = if value { &on } else { &off };
        tile_set_string_trait(cx, field(id), TRAIT_STRING, text)?;
    }
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(true)
}
