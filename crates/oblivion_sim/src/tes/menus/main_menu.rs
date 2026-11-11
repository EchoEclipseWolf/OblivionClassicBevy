//! MainMenu (1044): its creation, the New Game confirmation, the intro fade and the Continue button's refresh.
use oblivion_core::error::GameError;

use super::*;
use crate::gamebryo::scene_node::SceneNodePayload;
use crate::gamebryo::scene_update::{SceneUpdateStatus, update_scene_object};
use crate::tes::form_hierarchy::RttiClass;
use crate::tes::forms_dialogue::TesQuest;
use crate::tes::forms_init::{cast, form_type, lookup, record_form_change};
use crate::tes::interface_frame::*;
use crate::tes::interface_manager::{
    InterfaceManager, advance_priority, menu_register_tile, menu_update_cursor_depth,
};
use crate::tes::interface_player_scene::{
    interface_close_all_menus_if_menu_mode, interface_close_inventory_menus_and_restore_player,
    interface_update_reticle_if_player_has_3d,
};
use crate::tes::interface_scene::tile_scene_node;
use crate::tes::movie_player::{MoviePlayer, play_menu_movie};
use crate::tes::os_globals::{OsGlobalsData, lock_shared};
use crate::tes::os_sound_music::{MUSIC_TYPE_SPECIAL, MusicDevice, MusicSettings};
use crate::tes::player::PlayerCharacter;
use crate::tes::player::the_player;
use crate::tes::tes::TesGlobals;
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use crate::tes::tile_values::*;
use crate::tes::tile_xml::tile_read_xml;
use bevy_ecs::prelude::*;

/// IDA: N/A a menu's Tile*'s class by the menu table. Completion: 100%.
pub(super) fn main_menu_of(cx: &InterfaceCx) -> Option<MenuId> {
    let tile = cx.engine.tile_menu_by_id(1044)?;
    cx.engine.containing_menu(tile)
}

/// IDA: N/A host helper. Completion: 100%.
pub(super) fn with_main<R>(
    cx: &mut InterfaceCx,
    menu: MenuId,
    body: impl FnOnce(&mut MainMenu) -> R,
) -> Option<R> {
    match &mut cx.engine.menu_mut(menu).class {
        MenuClass::Main(main) => Some(body(main)),
        _ => None,
    }
}

/// IDA: 005B6040 MainMenu_Create. An existing menu is deleted, then 0057CC00, 00579770 and 00578CF0(3). The NiAlphaAccumulator goes to the renderer: not ported, since the host render has none, a recorded difference. The null test leaves `continue` out, as the exe does. `version` is GetFileVersionInfoA's ProductVersion, the host's constant, a recorded substitution checked against the evidence file's bytes. Completion: 100% of the order.
#[allow(clippy::too_many_lines)]
pub fn main_menu_create(
    cx: &mut InterfaceCx,
    frame_time: f32,
) -> Result<Option<TileId>, GameError> {
    if let Some(menu) = main_menu_of(cx) {
        menu_destroy(cx, menu)?;
    }
    interface_close_inventory_menus_and_restore_player(cx)?;
    interface_close_all_menus_if_menu_mode(cx)?;
    interface_update_reticle_if_player_has_3d(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let depth = menu_update_cursor_depth(cx)?;
    let Some(created) = tile_read_xml(cx, root, "Data\\Menus\\Options\\main_menu.xml")? else {
        return Ok(None);
    };
    let Some(menu) = cx.engine.containing_menu(created) else {
        return Ok(Some(created));
    };
    if cx.engine.menu(menu).id() != 1044 {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(Some(created));
    }
    menu_register_tile(cx, menu, Some(created))?;
    let fields = with_main(cx, menu, |main| main.clone()).expect("the ID said so");
    // Every button but continue, and the backdrop.
    let (Some(new_game), Some(_), Some(_), Some(_), Some(_), Some(backdrop)) = (
        fields.new_game_button_30,
        fields.load_button_34,
        fields.options_button_38,
        fields.credits_button_3c,
        fields.exit_button_40,
        fields.backdrop_44,
    ) else {
        cx.host
            .warning("Main Menu Creation Failed... Are your menu and art resources up to date?");
        return Ok(None);
    };
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    let version = tile_resolve_source(cx, Some(created), b"version");
    if let Some(version) = version {
        tile_set_string_trait(cx, version, TRAIT_STRING, b"1.2.0416")?;
    }
    let visible = if cx.engine.main_menu_statics.movie_b3b408 {
        1.0
    } else {
        2.0
    };
    tile_set_numeric_trait(cx, backdrop, TRAIT_VISIBLE, visible)?;
    tile_update_scene_graph(cx, created, false)?;
    // `fidiv` and the compare with 0.75 at 24 bits: an aspect other than 4:3 moves and scales the backdrop's node.
    let aspect = cx.settings.renderer_height as f32 / cx.settings.renderer_width as f32;
    if aspect != 0.75
        && let Some(node) = tile_scene_node(cx, backdrop)
    {
        let mut payload = cx
            .world
            .get_mut::<SceneNodePayload>(node)
            .expect("a tile node has a payload");
        let transform = &mut payload.local_transform;
        transform.translation[0] -= 214.0;
        transform.translation[2] += 160.0;
        transform.scale = 1.333_333_4;
        let status = update_scene_object(cx.world, node, 0.0, false, cx.runtime)?;
        if status.status != SceneUpdateStatus::Updated {
            return Err(GameError::unclassified(
                0x005B6040,
                "005B6040: NiAVObject_Update reported an unsupported state",
            ));
        }
    }
    // 005B6342 0045D450, then a first list entry with a file.
    let saves = cx.host.save_file_list();
    let continue_button = fields.continue_button_28;
    if saves.is_empty() {
        if let Some(button) = continue_button {
            tile_set_numeric_trait(cx, button, TRAIT_VISIBLE, 1.0)?;
        }
        let priority = advance_priority(cx);
        tile_set_numeric_trait(cx, new_game, TRAIT_XDEFAULT, priority as f32)?;
    } else {
        if let Some(button) = continue_button {
            tile_set_numeric_trait(cx, button, TRAIT_VISIBLE, 2.0)?;
            // 005966F0(1) through Tile_SetUnsignedTrait: fild of the unsigned value.
            let priority = advance_priority(cx);
            tile_set_numeric_trait(cx, button, TRAIT_XDEFAULT, priority as f32)?;
        }
        tile_set_numeric_trait(cx, new_game, TRAIT_XDEFAULT, 1.0)?;
    }
    // zoom / 100 (24 bits), |scale| on the node, 1.0 / signed in a double, then x and y times that, each rounded on store.
    if let Some(version) = version {
        let zoom = tile_numeric_trait(cx, version, TRAIT_ZOOM);
        let scale = zoom / 100.0;
        if let Some(node) = tile_scene_node(cx, version) {
            cx.world
                .get_mut::<SceneNodePayload>(node)
                .expect("a tile node has a payload")
                .local_transform
                .scale = scale.abs();
        }
        let x = version_position(tile_numeric_trait(cx, version, TRAIT_X), scale);
        tile_set_numeric_trait(cx, version, TRAIT_X, x)?;
        let y = version_position(tile_numeric_trait(cx, version, TRAIT_Y), scale);
        tile_set_numeric_trait(cx, version, TRAIT_Y, y)?;
    }
    tile_update_scene_graph(cx, created, false)?;
    menu_enable(cx, menu, false, frame_time)?;
    Ok(Some(created))
}

/// IDA: 005B642B..005B6447: the version text's x or y, over the node's scale. Completion: 100%.
#[must_use]
pub fn version_position(position: f32, scale: f32) -> f32 {
    let inverse = 1.0f32 / scale;
    position * inverse
}

/// IDA: 005B5DF0 MainMenu_NewGameConfirmationCallback past its button: sCharGenQuest's form, type 0x3B and a TESQuest, or a warning. 004431F0 on form 0x3C, which is current already on this path, so a different one is a stop. 005B5960 MainMenu_Close: destroyOnClose 2.0 and 00584740. The music. 00B3A6D3 = 1 has no reader in the exe, so it isn't kept. Player +0x7F8 = 1. 00529820(1): the quest's +0x3C bit 0, then vt+0x40(4). Completion: 100% on the default path.
/// # Errors
/// Another world space, or a stop below.
pub fn main_menu_new_game(cx: &mut InterfaceCx) -> Result<(), GameError> {
    let id = cx.host.chargen_quest_id();
    let quest = lookup(cx.world, id).filter(|&form| {
        form_type(cx.world, form) == 0x3B && cx.world.get::<TesQuest>(form).is_some()
    });
    let Some(quest) = quest else {
        cx.host.warning(&format!(
            "Unable to find character generation quest '{id:08X}'."
        ));
        return Ok(());
    };
    let space = cast(cx.world, lookup(cx.world, 0x3C), RttiClass::TESWorldSpace);
    let current = cx
        .world
        .resource::<TesGlobals>()
        .tes
        .as_ref()
        .and_then(|tes| tes.current_world_space_74);
    if space.is_some() && space != current {
        return Err(GameError::unclassified(
            0x005B5E9F,
            "005B5E9F: a new game in another world space",
        ));
    }
    if let Some(menu) = main_menu_of(cx) {
        let tile = cx.engine.menu(menu).tile_04.ok_or(GameError::unclassified(
            0x005B5977,
            "005B5977: a MainMenu with no tile",
        ))?;
        tile_set_numeric_trait(cx, tile, TRAIT_DESTROY_ON_CLOSE, 2.0)?;
        let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
        menu_begin_fade_out(cx, menu, frame_time)?;
    }
    cx.host.new_game_music(cx.world);
    let player = the_player(cx.world).ok_or(GameError::unclassified(
        0x005B5EF1,
        "005B5EF1: a null g_thePlayer",
    ))?;
    cx.world
        .get_mut::<PlayerCharacter>(player)
        .ok_or(GameError::unclassified(
            0x005B5EF1,
            "005B5EF1: g_thePlayer is no PlayerCharacter",
        ))?
        .new_game_intro_pending_7f8 = 1;
    cx.world
        .get_mut::<TesQuest>(quest)
        .expect("tested above")
        .flags_3c |= 1;
    record_form_change(cx.world, quest, 4)
}

/// IDA: 005B5C90 MainMenu_BeginIntroFadeAndBackdrop: user3 = 255, then a transition to 0 over user4 seconds; `backdrop_movie` gives 00B3B408; the backdrop's visible 1 with the movie, 2 without. Returns 00B3B408. Completion: 100%.
pub fn main_menu_begin_intro(
    cx: &mut InterfaceCx,
    backdrop_movie: impl FnOnce() -> bool,
) -> Result<bool, GameError> {
    let menu = main_menu_of(cx).ok_or(GameError::unclassified(
        0x005B5C9B,
        "005B5C9B: a null 00B3B40C faults",
    ))?;
    let tile = cx.engine.menu(menu).tile_04.ok_or(GameError::unclassified(
        0x005B5C9B,
        "005B5C9B: a menu with no tile faults",
    ))?;
    tile_set_numeric_trait(cx, tile, TRAIT_USER3, 255.0)?;
    let seconds = tile_numeric_trait(cx, tile, TRAIT_USER4);
    tile_transition_numeric_trait_to(cx, tile, TRAIT_USER3, 255.0, 0.0, seconds);
    let movie = backdrop_movie();
    cx.engine.main_menu_statics.movie_b3b408 = movie;
    let backdrop = with_main(cx, menu, |main| main.backdrop_44).flatten();
    if let Some(backdrop) = backdrop {
        tile_set_numeric_trait(cx, backdrop, TRAIT_VISIBLE, if movie { 1.0 } else { 2.0 })?;
    }
    Ok(movie)
}

/// IDA: 005B5B70(anySaves): a change of `continue`'s visible, the priorities with it. Completion: 100%.
pub(super) fn main_menu_refresh_continue(cx: &mut InterfaceCx, any: bool) -> Result<(), GameError> {
    let Some(menu) = main_menu_of(cx) else {
        return Ok(());
    };
    let fields = with_main(cx, menu, |main| main.clone()).expect("the slot's class");
    let (Some(button), Some(new_game)) = (fields.continue_button_28, fields.new_game_button_30)
    else {
        return Ok(());
    };
    let visible = tile_numeric_trait(cx, button, TRAIT_VISIBLE);
    if any {
        if visible == 2.0 {
            return Ok(());
        }
        tile_set_numeric_trait(cx, button, TRAIT_VISIBLE, 2.0)?;
        let priority = advance_priority(cx);
        tile_set_numeric_trait(cx, button, TRAIT_XDEFAULT, priority as f32)?;
        tile_set_numeric_trait(cx, new_game, TRAIT_XDEFAULT, 1.0)?;
    } else {
        if visible == 1.0 {
            return Ok(());
        }
        tile_set_numeric_trait(cx, button, TRAIT_VISIBLE, 1.0)?;
        let priority = advance_priority(cx);
        tile_set_numeric_trait(cx, new_game, TRAIT_XDEFAULT, priority as f32)?;
    }
    Ok(())
}

/// IDA: MainMenu (0x50): 005B5F20 MainMenu_ctor's fields. Completion: 100% of the fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MainMenu {
    /// +0x28.
    pub continue_button_28: Option<TileId>,
    /// +0x30.
    pub new_game_button_30: Option<TileId>,
    /// +0x34.
    pub load_button_34: Option<TileId>,
    /// +0x38.
    pub options_button_38: Option<TileId>,
    /// +0x3C.
    pub credits_button_3c: Option<TileId>,
    /// +0x40.
    pub exit_button_40: Option<TileId>,
    /// +0x44.
    pub backdrop_44: Option<TileId>,
    /// +0x4C.
    pub unk_4c: u8,
    /// +0x4D: a confirmation is up, so 005B64C0 ignores clicks.
    pub confirmation_pending_4d: u8,
}

/// IDA: dword_B3B3FC[3]: 00B3B408 the movie byte. 00B3B409 (set by the ctor) and 00B3B40C (the live MainMenu) have no reader here.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MainMenuStatics {
    pub movie_b3b408: bool,
}

/// IDA: N/A what a click asks of a phase that isn't here: 005B5D60 and 005AE190's LoadGame, 005B5DF0's new game. A host stop by return.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameStartRequest {
    NewGame,
    LoadSave(SaveFile),
}

/// IDA: 005B5F20 MainMenu_ctor's fields. Its movie and music are the host's `main_menu_ctor_media`, just before this. Completion: 100% of the fields.
#[must_use]
pub fn main_menu_constructed() -> MainMenu {
    MainMenu::default()
}

/// IDA: 005B5870 MainMenu_Destructor before its 00585330: 00B3B40C = 0, which the port reads through the menu's tile slot that 00585330 clears; the media (`main_menu_destructor_media`); 00459400 TESSaveLoadGame_ClearSaveFileList, whose list the host reads afresh at each use (`save_file_list`), so nothing is held to clear; InterfaceManager +0x94 = 1, which 0057E150 reads. Completion: 100% of the state the port holds.
pub(in crate::tes) fn main_menu_destructor(cx: &mut InterfaceCx) {
    cx.host.main_menu_destructor_media(cx.world);
    cx.world
        .resource_mut::<crate::tes::interface_manager::InterfaceManager>()
        .big_four_rebuild_pending_094 = 1;
}

/// IDA: 005B64C0 MainMenu_HandleClick: ignored while +0x4D is set. Continue, New, Load, Options, Credits, Exit. 2: +0x4D = 1, the prompt with B38CF8 "Yes" and B38D00 "No". 3: 0057DE50(1), then the prompt, with no +0x4D (the C++ sets it). 4, 5, 6: vt+0x14, then 005AEA60, 005BD680, 0059D680. 7: vt+0x14, +0x4D, the prompt. 005BD680 and 0059D680 are later phases' menus, which stop by a warning here. Completion: 100% of the switch.
pub fn main_menu_handle_click(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: Option<TileId>,
) -> Result<(), GameError> {
    if with_main(cx, menu, |main| main.confirmation_pending_4d) != Some(0) {
        return Ok(());
    }
    match id {
        2 => {
            with_main(cx, menu, |main| main.confirmation_pending_4d = 1);
            let text = text_at_or_empty(cx.world, 0x00B3_8770);
            let (yes, no) = yes_no(cx);
            show_message(
                cx,
                &text,
                Some(MessageCallback::MainMenuContinue),
                1,
                &[yes, no],
            )?;
        }
        3 => {
            cx.host.play_ui_sound(1);
            let text = text_at_or_empty(cx.world, 0x00B3_8778);
            let (yes, no) = yes_no(cx);
            show_message(
                cx,
                &text,
                Some(MessageCallback::MainMenuNewGame),
                1,
                &[yes, no],
            )?;
        }
        4 => {
            menu_handle_mouseout(cx, menu, id, tile)?;
            load_menu_create(cx, false)?;
        }
        5 | 6 => {
            menu_handle_mouseout(cx, menu, id, tile)?;
            cx.host.warning(if id == 5 {
                "005BD680: the Options menu is carried to its phase"
            } else {
                "0059D680: the Credits menu is carried to its phase"
            });
        }
        7 => {
            menu_handle_mouseout(cx, menu, id, tile)?;
            with_main(cx, menu, |main| main.confirmation_pending_4d = 1);
            let text = text_at_or_empty(cx.world, 0x00B3_8C50);
            let exit = text_at_or_empty(cx.world, 0x00B3_8C58);
            let cancel = text_at_or_empty(cx.world, 0x00B3_8D08);
            show_message(
                cx,
                &text,
                Some(MessageCallback::MainMenuExit),
                1,
                &[exit, cancel],
            )?;
        }
        _ => {}
    }
    Ok(())
}

/// IDA: 005B5D60's first half, for each callback: 00578D70 reads and clears +0xB0. Continue (005B5D60), New (005B5DF0), Exit (005B59B0), Load's confirm (005AE190). What a Yes asks of a later phase goes to `start_game` and the menu stays, a host stop by return. Completion: 100% of the branches on this side of LoadGame.
pub fn run_message_callback(
    cx: &mut InterfaceCx,
    callback: MessageCallback,
) -> Result<(), GameError> {
    let button = {
        let mut manager = cx.world.resource_mut::<InterfaceManager>();
        let button = manager.msg_box_button_pressed_0b0;
        manager.msg_box_button_pressed_0b0 = 0xFF;
        button
    };
    match callback {
        MessageCallback::RaceSexReset => {
            return crate::tes::race_sex_menu::race_sex_menu_reset_confirm(cx, button);
        }
        MessageCallback::RaceSexRandom => {
            return crate::tes::race_sex_menu::race_sex_menu_random_confirm(cx, button);
        }
        MessageCallback::RaceSexDone => {
            return crate::tes::race_sex_menu::race_sex_menu_done_confirm(cx, button);
        }
        MessageCallback::MainMenuContinue => {
            let Some(menu) = main_menu_of(cx) else {
                return Ok(());
            };
            with_main(cx, menu, |main| main.confirmation_pending_4d = 1);
            if button == 1 {
                let saves = cx.host.save_file_list();
                if let Some(first) = saves.first() {
                    cx.host
                        .start_game(GameStartRequest::LoadSave(first.clone()));
                } else {
                    let text = crate::tes::game_settings::text_at_or_empty(cx.world, 0x00B3_86D8);
                    // 005B5DC3: dword ptr 00B38CF0, sOk.
                    let ok = crate::tes::game_settings::text_at_or_empty(cx.world, 0x00B3_8CF0);
                    show_message(cx, &text, None, 0, &[ok])?;
                }
            }
            with_main(cx, menu, |main| main.confirmation_pending_4d = 0);
        }
        MessageCallback::MainMenuNewGame => {
            if main_menu_of(cx).is_some() && button == 1 {
                main_menu_new_game(cx)?;
            }
        }
        MessageCallback::MainMenuExit => {
            let menu = main_menu_of(cx);
            if let Some(menu) = menu {
                with_main(cx, menu, |main| main.confirmation_pending_4d = 1);
            }
            if button == 1 {
                if menu.is_some() {
                    // The menu root's and the cursor's nodes culled, and one more 0057F170.
                    let (root, cursor) = {
                        let manager = cx.world.resource::<InterfaceManager>();
                        (manager.menu_root_068, manager.cursor_01c)
                    };
                    for tile in [root, cursor].into_iter().flatten() {
                        if let Some(node) = tile_scene_node(cx, tile) {
                            cx.world
                                .get_mut::<SceneNodePayload>(node)
                                .expect("a tile node has a payload")
                                .effective_av_flags |= 1;
                        }
                    }
                }
                cx.host.quit_game();
            } else if let Some(menu) = menu {
                with_main(cx, menu, |main| main.confirmation_pending_4d = 0);
            }
        }
        MessageCallback::LoadgameConfirm => {
            let Some(tile) = cx.engine.tile_menu_by_id(1038) else {
                return Ok(());
            };
            let Some(menu) = cx.engine.containing_menu(tile) else {
                return Ok(());
            };
            let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class else {
                return Ok(());
            };
            load.confirmation_pending_64 = 0;
            let chosen = load
                .chosen_save_4c
                .and_then(|index| load.saves_54.get(index).cloned());
            if button != 2
                && let Some(save) = chosen
            {
                cx.host.start_game(GameStartRequest::LoadSave(save));
            }
        }
    }
    Ok(())
}

/// IDA: A6CB58, the 12 bytes 005B5AD4..005B5AE5 copy to the stack: "Data\Music\".
const MUSIC_FOLDER: &[u8] = b"Data\\Music\\";

/// IDA: 005B5AC0 MainMenu_StartTitleMusic. 005B5AD4..005B5B2D "Data\Music\" + sMainMenuMusic on the stack, with no length check; 005B5B2F 00B3C0EC = 1; 005B5B36 g_osGlobals->sound; a non-null one: 006AB160(8, path, 0), then 006AB420 when it returned true. Completion: 100%; a name past the 0x108-byte stack buffer overruns it in the original and only makes a longer path here.
pub fn main_menu_start_title_music(
    world: &mut World,
    music_name: &[u8],
    settings: &MusicSettings,
    device: &mut dyn MusicDevice,
) -> bool {
    let mut path = MUSIC_FOLDER.to_vec();
    path.extend_from_slice(music_name);
    let mut os = world.resource_mut::<OsGlobalsData>();
    let Some(sound) = os.sound_24.as_mut() else {
        return false;
    };
    sound.main_menu_music_b3c0ec = 1;
    let loaded = sound.load_music_track(MUSIC_TYPE_SPECIAL, Some(&path), false, settings, device);
    if loaded {
        sound.start_music_playback(settings, device);
    }
    loaded
}

/// IDA: 005B5C90 MainMenu_BeginIntroFadeAndBackdrop's movie: 005B5CE4..005B5CF7 00B3B408 = current's +0x20 != 0, or 0 without one; while 0, 005B5D07 00410C40(sMainMenuMovie, 1) sets it. Completion: 100% of the movie; the tile traits around it (0FB1, 0FB2, 0FA1) are the screens', and 00B3B408 tells them which of 005B5D18's and 005B5D3A's values applies.
pub fn main_menu_begin_backdrop_movie(player: &MoviePlayer, movie_name: &str) -> bool {
    let current = lock_shared(&player.globals.current_b33428).clone();
    let mut backdrop = current.is_some_and(|movie| movie.state() != 0);
    if !backdrop {
        backdrop = play_menu_movie(player, movie_name, true);
    }
    backdrop
}

#[cfg(test)]
mod tests {
    use super::*;

    // 005B642B `fld1; fdivrp` rounds at 24 bits, and 005B6438's `fmul` of the stored value rounds again: the version's 10 at zoom 75 is 0x41555556.
    #[test]
    fn the_version_divides_at_24_bits() {
        assert_eq!(version_position(10.0, 0.75).to_bits(), 0x4155_5556);
    }
}
