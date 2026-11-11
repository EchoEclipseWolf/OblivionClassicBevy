//! AudioMenu (1017): 00595150 AudioMenu_ctor's fields, 00595190 SetField while `Data\Menus\Options\audio_menu.xml` is
//! built, and 00595380, which builds it with the five volume sliders from OSSoundGlobals. Its caller (the Options menu's
//! click 005BD880) and its click (00595790), mouseover (00595940) and per-frame (00595A20) virtuals aren't ported.

use oblivion_core::error::GameError;
use oblivion_core::x87;

use crate::tes::interface_frame::{menu_destroy, menu_enable};
use crate::tes::interface_manager::{
    InterfaceManager, menu_register_tile, menu_update_cursor_depth,
};
use crate::tes::os_globals::OsGlobalsData;
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use crate::tes::tile_values::{
    TRAIT_DEPTH, TRAIT_STACKING, TRAIT_USER1, TRAIT_USER2, TRAIT_USER4, TRAIT_USER5,
    tile_numeric_trait, tile_set_numeric_trait, tile_update_scene_graph,
};
use crate::tes::tile_xml::tile_read_xml;

/// IDA: MenuID_Audio.
pub const MENU_ID_AUDIO: u32 = 1017;
const AUDIO_XML: &str = "Data\\Menus\\Options\\audio_menu.xml";

/// IDA: 00595150 AudioMenu_ctor (via 00587D20's 1017 case, FormHeap 0x54): Menu_Constructor, the eleven tile pointers
/// +0x28..+0x50 null, vtable. Completion: 100%.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioMenu {
    /// +0x28.. fields 1..10 and 12: the master, voice, effect, foot and music sliders (1, 3, 5, 7, 9), each followed by
    /// its marker (2, 4, 6, 8, 10, by the slider's user6), and `audio_return_button` (12). `audio_defaults_button` (11)
    /// has no slot.
    pub tiles_28: [Option<TileId>; 11],
}

/// IDA: 00595190's ids, in +0x28..+0x50 order.
const FIELD_IDS: [i32; 11] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12];

impl AudioMenu {
    /// IDA: 00595190 AudioMenu_SetField: ids 1..10 to +0x28..+0x4C, 12 to +0x50. Completion: 100%.
    pub fn set_field(&mut self, id: i32, tile: TileId) {
        if let Some(slot) = FIELD_IDS.iter().position(|&known| known == id) {
            self.tiles_28[slot] = Some(tile);
        }
    }

    /// IDA: 00595300 AudioMenu_HasRequiredTiles: all eleven nonnull. Completion: 100%.
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

// IDA: N/A the menu's AudioMenu part.
fn audio<'a>(cx: &'a InterfaceCx, menu: MenuId) -> &'a AudioMenu {
    match &cx.engine.menu(menu).class {
        MenuClass::Audio(audio) => audio,
        _ => unreachable!("an AudioMenu"),
    }
}

/// IDA: 005954A5..: one slider: user1 0, user2 100, user4 25, user5 the volume's percent then 0. Completion: 100%.
fn set_slider(cx: &mut InterfaceCx, slider: TileId, percent: i32) -> Result<(), GameError> {
    tile_set_numeric_trait(cx, slider, TRAIT_USER1, 0.0)?;
    tile_set_numeric_trait(cx, slider, TRAIT_USER2, 100.0)?;
    tile_set_numeric_trait(cx, slider, TRAIT_USER4, 25.0)?;
    tile_set_numeric_trait(cx, slider, TRAIT_USER5, percent as f32)?;
    tile_set_numeric_trait(cx, slider, TRAIT_USER5, 0.0)
}

/// IDA: 00595380 AudioMenu_Create: an existing 1017 menu deleted (TileMenu vt+0, ported as `map_menu_create` does it);
/// the depth (00588890 → 00584980); Tile_ReadXML(audio_menu.xml) under the root; its containing menu, else false; a menu
/// that isn't 1017 is deleted when it has a tile, and false; Tile_UpdateSceneGraph(0) before 00584880 registers the
/// tile; without the eleven tiles (00595300) the warning and false (the menu stays). A stacking type 6006 or 102 takes
/// the depth. Then each slider from g_osGlobals' sound object (+0x24): its volume `fmul` 100.0 (a double, rounded at the
/// main thread's 24 bits) through __ftol2, in this order: master (+0xB8) on field 1, music (+0x2F8, 006A8E00 OSSoundGlobals_GetMusicVolume) on 9, foot
/// (+0xBC) on 7, voice (+0xC0) on 3, effects (+0xC4) on 5. Menu_Enable(0), true. Completion: 100%.
/// # Errors
/// No sound object at 00595499, where the original reads null's +0xB8; a stop in the XML or the menu calls.
pub fn audio_menu_create(cx: &mut InterfaceCx) -> Result<bool, GameError> {
    if let Some(old) = cx.engine.tile_menu_by_id(MENU_ID_AUDIO)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let created = tile_read_xml(cx, root, AUDIO_XML)?;
    let menu = created.and_then(|created| cx.engine.containing_menu(created));
    let (Some(created), Some(menu)) = (created, menu) else {
        return Ok(false);
    };
    if cx.engine.menu(menu).id() != MENU_ID_AUDIO {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(false);
    }
    tile_update_scene_graph(cx, created, false)?;
    menu_register_tile(cx, menu, Some(created))?;
    if !audio(cx, menu).has_required_tiles() {
        cx.host
            .warning("Audio Menu Creation Failed... Are your menu and art resources up to date?");
        return Ok(false);
    }
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let volumes = cx
        .world
        .get_resource::<OsGlobalsData>()
        .and_then(|globals| globals.sound_24.as_ref())
        .map(|sound| {
            [
                (1, sound.master_volume_b8),
                (9, sound.music_volume2_2f8),
                (7, sound.foot_volume_bc),
                (3, sound.voice_volume_c0),
                (5, sound.effects_volume_c4),
            ]
        })
        .ok_or(GameError::unclassified(
            0x00595499,
            "00595499: no OSSoundGlobals, and the original reads its +0xB8",
        ))?;
    let fields = audio(cx, menu).clone();
    for (id, volume) in volumes {
        let slider = fields.field(id).expect("the eleven tiles");
        set_slider(cx, slider, x87::ftol(volume * 100.0))?;
    }
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(true)
}
