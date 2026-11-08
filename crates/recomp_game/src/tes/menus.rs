//! M6: the three menu classes the first screens need, MainMenu (1044), LoadgameMenu (1038) and MessageMenu (1001), with the fields their `SetField` slots fill while `Tile_ReadXML` builds them. The classes' creation and click paths follow in this file. Fields keep the exe's offsets from the object's start, where `MainMenu_mbrs` in the IDB is listed from `Menu_mbrs`' end.

use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::ai_conditions::the_player;
use super::form_hierarchy::RttiClass;
use super::forms_dialogue::TesQuest;
use super::forms_init::{cast, form_type, lookup, record_form_change};
use super::interface_frame::*;
use super::interface_manager::{
    InterfaceManager, interface_update_menu_root, menu_register_tile, menu_update_cursor_depth,
};
use super::interface_player_scene::{
    interface_close_all_menus_if_menu_mode, interface_close_inventory_menus_and_restore_player,
    interface_update_reticle_if_player_has_3d,
};
use super::interface_scene::{SavePreview, tile_cumulative_position, tile_scene_node};
use super::player::PlayerCharacter;
use super::save_files::SaveFile;
use super::tes::TesGlobals;
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use super::tile_values::*;
use super::tile_xml::{tile_build_template, tile_read_xml};
use crate::gamebryo::scene_node::SceneNodePayload;
use crate::gamebryo::scene_update::{SceneUpdateStatus, update_scene_object};

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

/// IDA: MessageMenu (0x64): 005BC450's fields. +0x28..+0x58 are the 13 tiles 005BC510 stores by ID; +0x60 the 0x5C callback's index base (005BC8B0 stores its third argument); +0x5C the callback. Completion: 100% of the fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MessageMenu {
    /// +0x28..+0x58, IDs 1..13.
    pub tiles_28: [Option<TileId>; 13],
    /// +0x5C.
    pub callback_5c: Option<MessageCallback>,
    /// +0x60.
    pub button_base_60: u8,
}

/// IDA: the callbacks 005BC8B0's second argument takes on the paths this port reaches: 005B5D60, 005B5DF0, 005B59B0 and 005AE190. Host: an enum for the function pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageCallback {
    MainMenuContinue,
    MainMenuNewGame,
    MainMenuExit,
    LoadgameConfirm,
}

/// IDA: LoadgameMenu (0x68): 005AE450's fields. 005D2C60 stores IDs 1..7 at +0x28, +0x2C, +0x34, +0x38, +0x3C, +0x40, +0x44 and ID 9 at +0x48 (+0x30 isn't an ID: 005AE560 keeps the row in focus there). +0x4C the save chosen, +0x50 005AE140's store, +0x54 the save list, +0x58 the row of a pending confirm, +0x5C the name 005AE6D0 compares, +0x64 the confirm byte. Completion: 100% of the fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadgameMenu {
    /// +0x28.
    pub field_id1_28: Option<TileId>,
    /// +0x2C: 005AE560's focus box.
    pub focus_box_2c: Option<TileId>,
    /// +0x30: the row in focus.
    pub focused_row_30: Option<TileId>,
    /// +0x34.
    pub field_id3_34: Option<TileId>,
    /// +0x38.
    pub field_id4_38: Option<TileId>,
    /// +0x3C.
    pub field_id5_3c: Option<TileId>,
    /// +0x40: 005AEED0 hides it on Return.
    pub field_id6_40: Option<TileId>,
    /// +0x44.
    pub field_id7_44: Option<TileId>,
    /// +0x48: the list pane 005AE6D0 fills.
    pub list_pane_48: Option<TileId>,
    /// +0x4C: which save, an index in the list.
    pub chosen_save_4c: Option<usize>,
    /// The list +0x54 holds: g_saveLoadGame's, kept with the class here.
    pub saves_54: Vec<SaveFile>,
    /// +0x50.
    pub scroll_50: f32,
    /// +0x58.
    pub confirm_row_58: Option<TileId>,
    /// +0x64: 005AEED0 ignores clicks while it's set.
    pub confirmation_pending_64: u8,
}

/// IDA: vt+4 SetField from 0058B2F0's id arm: 005B5800 (MainMenu), 005BC510 (MessageMenu), 005D2C60 (LoadgameMenu), nullsub_1 for LoadingMenu; the other ported classes' own SetField (each named at its `set_field`). Completion: 100% of the ported classes' slots.
pub fn menu_set_field(cx: &mut InterfaceCx, menu: MenuId, id: i32, tile: TileId) {
    match &mut cx.engine.menu_mut(menu).class {
        MenuClass::Main(main) => {
            let slot = match id {
                2 => &mut main.continue_button_28,
                3 => &mut main.new_game_button_30,
                4 => &mut main.load_button_34,
                5 => &mut main.options_button_38,
                6 => &mut main.credits_button_3c,
                7 => &mut main.exit_button_40,
                8 => &mut main.backdrop_44,
                _ => return,
            };
            *slot = Some(tile);
        }
        MenuClass::Message(message) => {
            if (1..=13).contains(&id) {
                message.tiles_28[id as usize - 1] = Some(tile);
            }
        }
        MenuClass::Stats(stats) => stats.set_field(id, tile),
        MenuClass::QuickKeys(quick_keys) => quick_keys.set_field(id, tile),
        MenuClass::Map(map) => map.set_field(id, tile),
        MenuClass::Magic(magic) => magic.set_field(id, tile),
        MenuClass::Inventory(inventory) => inventory.set_field(id, tile),
        MenuClass::Load(load) => {
            let slot = match id {
                1 => &mut load.field_id1_28,
                2 => &mut load.focus_box_2c,
                3 => &mut load.field_id3_34,
                4 => &mut load.field_id4_38,
                5 => &mut load.field_id5_3c,
                6 => &mut load.field_id6_40,
                7 => &mut load.field_id7_44,
                9 => &mut load.list_pane_48,
                _ => return,
            };
            *slot = Some(tile);
        }
        MenuClass::Hud(hud) => super::hud_main_menu::hud_set_field(hud, id, tile),
        MenuClass::HudInfo(info) => super::hud_main_menu::hud_info_set_field(info, id, tile),
        MenuClass::HudSubtitle(subtitle) => subtitle.set_field(id, tile),
        MenuClass::SleepWait(sleep_wait) => sleep_wait.set_field(id, tile),
        MenuClass::Pause(pause) => pause.set_field(id, tile),
        MenuClass::Options(options) => options.set_field(id, tile),
        MenuClass::Quantity(quantity) => quantity.set_field(id, tile),
        MenuClass::Audio(audio) => audio.set_field(id, tile),
        MenuClass::Gameplay(gameplay) => gameplay.set_field(id, tile),
        MenuClass::Controls(controls) => controls.set_field(id, tile),
        MenuClass::MagicPopup(popup) => popup.set_field(id, tile),
        MenuClass::Breath(breath) => breath.set_field(id, tile),
        MenuClass::Recharge(recharge) => recharge.set_field(id, tile),
        MenuClass::TextEdit(text_edit) => text_edit.set_field(id, tile),
        MenuClass::Save(save) => save.set_field(id, tile),
        MenuClass::Loading(_) | MenuClass::Unported(_) => {}
    }
}

/// IDA: N/A a menu's Tile*'s class by the menu table. Completion: 100%.
fn main_menu_of(cx: &InterfaceCx) -> Option<MenuId> {
    let tile = cx.engine.tile_menu_by_id(1044)?;
    cx.engine.containing_menu(tile)
}

/// IDA: N/A host helper. Completion: 100%.
fn with_main<R>(
    cx: &mut InterfaceCx,
    menu: MenuId,
    body: impl FnOnce(&mut MainMenu) -> R,
) -> Option<R> {
    match &mut cx.engine.menu_mut(menu).class {
        MenuClass::Main(main) => Some(body(main)),
        _ => None,
    }
}

/// IDA: 005B5F20 MainMenu_ctor's fields. Its movie and music are the host's `main_menu_ctor_media`, just before this. Completion: 100% of the fields.
#[must_use]
pub fn main_menu_constructed() -> MainMenu {
    MainMenu::default()
}

/// IDA: 005B6040 MainMenu_Create. An existing menu is deleted, then 0057CC00, 00579770 and 00578CF0(3). The NiAlphaAccumulator goes to the renderer: not ported, since the host render has none, a recorded difference. The null test leaves `continue` out, as the exe does. `version` is GetFileVersionInfoA's ProductVersion, the host's constant, a recorded substitution checked against the evidence file's bytes. Completion: 100% of the order.
#[allow(clippy::too_many_lines)]
pub fn main_menu_create(
    cx: &mut InterfaceCx,
    frame_time: f32,
) -> Result<Option<TileId>, StreamError> {
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
            return Err(StreamError(
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
pub fn main_menu_new_game(cx: &mut InterfaceCx) -> Result<(), StreamError> {
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
        return Err(StreamError("005B5E9F: a new game in another world space"));
    }
    if let Some(menu) = main_menu_of(cx) {
        let tile = cx
            .engine
            .menu(menu)
            .tile_04
            .ok_or(StreamError("005B5977: a MainMenu with no tile"))?;
        tile_set_numeric_trait(cx, tile, TRAIT_DESTROY_ON_CLOSE, 2.0)?;
        let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
        menu_begin_fade_out(cx, menu, frame_time)?;
    }
    cx.host.new_game_music(cx.world);
    let player = the_player(cx.world).ok_or(StreamError("005B5EF1: a null g_thePlayer"))?;
    cx.world
        .get_mut::<PlayerCharacter>(player)
        .ok_or(StreamError("005B5EF1: g_thePlayer is no PlayerCharacter"))?
        .new_game_intro_pending_7f8 = 1;
    cx.world
        .get_mut::<TesQuest>(quest)
        .expect("tested above")
        .flags_3c |= 1;
    record_form_change(cx.world, quest, 4)
}

/// IDA: 005B5870 MainMenu_Destructor before its 00585330: 00B3B40C = 0, which the port reads through the menu's tile slot that 00585330 clears; the media (`main_menu_destructor_media`); 00459400 TESSaveLoadGame_ClearSaveFileList, whose list the host reads afresh at each use (`save_file_list`), so nothing is held to clear; InterfaceManager +0x94 = 1, which 0057E150 reads. Completion: 100% of the state the port holds.
pub(super) fn main_menu_destructor(cx: &mut InterfaceCx) {
    cx.host.main_menu_destructor_media(cx.world);
    cx.world
        .resource_mut::<super::interface_manager::InterfaceManager>()
        .big_four_rebuild_pending_094 = 1;
}

/// IDA: 005966F0 InterfaceManager_AdvanceDefaultSelectionPriority: +0x8C + 1. Completion: 100%.
fn advance_priority(cx: &mut InterfaceCx) -> u32 {
    let mut manager = cx.world.resource_mut::<InterfaceManager>();
    manager.default_selection_priority_08c += 1;
    manager.default_selection_priority_08c
}

/// IDA: 005B5C90 MainMenu_BeginIntroFadeAndBackdrop: user3 = 255, then a transition to 0 over user4 seconds; `backdrop_movie` gives 00B3B408; the backdrop's visible 1 with the movie, 2 without. Returns 00B3B408. Completion: 100%.
pub fn main_menu_begin_intro(
    cx: &mut InterfaceCx,
    backdrop_movie: impl FnOnce() -> bool,
) -> Result<bool, StreamError> {
    let menu = main_menu_of(cx).ok_or(StreamError("005B5C9B: a null 00B3B40C faults"))?;
    let tile = cx
        .engine
        .menu(menu)
        .tile_04
        .ok_or(StreamError("005B5C9B: a menu with no tile faults"))?;
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

/// IDA: vt+0xC HandleClick: 005B64C0 (MainMenu), 005BCDD0 (MessageMenu), 005AEED0 (LoadgameMenu), nullsub for the rest. Completion: 100% of the three.
pub fn menu_handle_click(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: Option<TileId>,
) -> Result<(), StreamError> {
    match cx.engine.menu(menu).class {
        MenuClass::Main(_) => main_menu_handle_click(cx, menu, id, tile),
        MenuClass::Message(_) => message_menu_handle_click(cx, menu, id),
        MenuClass::Load(_) => load_menu_handle_click(cx, menu, id, tile),
        MenuClass::Hud(_) => super::hud_main_menu::hud_handle_click(cx, id),
        _ => Ok(()),
    }
}

/// IDA: vt+0x10 HandleMouseover: 005BD9E0 Menu_PlayFocusSound (MainMenu), 005BC770 (MessageMenu), 005AE560 (LoadgameMenu). Completion: 100%.
pub fn menu_handle_mouseover(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: Option<TileId>,
) -> Result<(), StreamError> {
    match cx.engine.menu(menu).class {
        MenuClass::Main(_) => {
            cx.host.play_ui_sound(4);
            Ok(())
        }
        MenuClass::Message(_) => message_menu_mouseover(cx, menu, tile),
        MenuClass::Load(_) => load_menu_mouseover(cx, menu, id, tile),
        _ => Ok(()),
    }
}

/// IDA: vt+0x14 HandleMouseout: nullsub_1 (MainMenu), 005BC890, 005AE220. Completion: 100%.
pub fn menu_handle_mouseout(
    cx: &mut InterfaceCx,
    menu: MenuId,
    _id: i32,
    _tile: Option<TileId>,
) -> Result<(), StreamError> {
    let hide = match &cx.engine.menu(menu).class {
        MenuClass::Message(message) => message.tiles_28[2],
        MenuClass::Load(load) => load.focus_box_2c,
        _ => None,
    };
    if let Some(hide) = hide {
        tile_set_numeric_trait(cx, hide, TRAIT_VISIBLE, 1.0)?;
    }
    Ok(())
}

/// IDA: 005B64C0 MainMenu_HandleClick: ignored while +0x4D is set. Continue, New, Load, Options, Credits, Exit. 2: +0x4D = 1, the prompt with B38CF8 "Yes" and B38D00 "No". 3: 0057DE50(1), then the prompt, with no +0x4D (the C++ sets it). 4, 5, 6: vt+0x14, then 005AEA60, 005BD680, 0059D680. 7: vt+0x14, +0x4D, the prompt. 005BD680 and 0059D680 are later phases' menus, which stop by a warning here. Completion: 100% of the switch.
pub fn main_menu_handle_click(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: Option<TileId>,
) -> Result<(), StreamError> {
    if with_main(cx, menu, |main| main.confirmation_pending_4d) != Some(0) {
        return Ok(());
    }
    match id {
        2 => {
            with_main(cx, menu, |main| main.confirmation_pending_4d = 1);
            let text = game_setting_text(cx, 0x00B3_8770);
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
            let text = game_setting_text(cx, 0x00B3_8778);
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
            let text = game_setting_text(cx, 0x00B3_8C50);
            let exit = game_setting_text(cx, 0x00B3_8C58);
            let cancel = game_setting_text(cx, 0x00B3_8D08);
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

/// IDA: the GMST string settings the prompts push, by the static Setting's address (00B38770 sContinueLastSave and so on), which is what the exe's operands are. The Phase 12 load may have replaced the text. Completion: 100%.
fn game_setting_text(cx: &InterfaceCx, address: u32) -> Vec<u8> {
    cx.world
        .get_resource::<super::game_settings::GameSettingCollection>()
        .and_then(|settings| settings.text_at(address))
        .unwrap_or_default()
}

/// IDA: dword_B38CF8 sYes and unk_B38D00 sNo, the settings in 005B64C0's pushes. Completion: 100%.
fn yes_no(cx: &InterfaceCx) -> (Vec<u8>, Vec<u8>) {
    (
        game_setting_text(cx, 0x00B3_8CF8),
        game_setting_text(cx, 0x00B3_8D00),
    )
}

/// IDA: 00579C10 ShowMessageBox (text, callback, base, buttons...): no manager, cursor or root returns false. The menu root's user0 2.0 takes 005BC8B0; anything else takes 005BCC00, which runs the same and then shows the menu at once (0057EA20(1.0), visible 2, destroyOnClose 2, state Visible). Completion: 100%.
pub fn show_message(
    cx: &mut InterfaceCx,
    text: &[u8],
    callback: Option<MessageCallback>,
    button_base: u8,
    buttons: &[Vec<u8>],
) -> Result<bool, StreamError> {
    let manager = cx.world.resource::<InterfaceManager>();
    let (Some(_), Some(root)) = (manager.cursor_01c, manager.menu_root_068) else {
        return Ok(false);
    };
    let in_game = tile_numeric_trait(cx, root, TRAIT_USER0) == 2.0;
    let created = message_menu_create(cx, text, callback, button_base, buttons)?;
    if !in_game && let Some(tile) = cx.engine.tile_menu_by_id(1001) {
        interface_apply_fade(cx, tile, 1.0)?;
        tile_set_numeric_trait(cx, tile, TRAIT_VISIBLE, 2.0)?;
        tile_set_numeric_trait(cx, tile, TRAIT_DESTROY_ON_CLOSE, 2.0)?;
        if let Some(menu) = cx.engine.containing_menu(tile) {
            cx.engine.menu_mut(menu).state_24 = MENU_STATE_VISIBLE;
        }
        tile_update_scene_graph(cx, tile, false)?;
    }
    Ok(created)
}

/// IDA: 005BC8B0: an old menu deleted; 00584980; 005903E0; Tile_ReadXML(message_menu.xml); the class test; 00584880; the +0x2C, +0x30, +0x28 test (IDs 2, 3, 1); the depth; +0xB0 = -1; +0x5C = callback; a null first button hides it, else its text; the rest of the buttons take IDs from 5; the message text, or sDefaultMessage; +0x60 = base; user0 = status != 1; 0057DE50(0xB); two 0058FBA0; 00585190(0); visible 2. Completion: 100% except 00572DF0(2) and 0040D160, a screen effect with no capture on this path, counted.
pub fn message_menu_create(
    cx: &mut InterfaceCx,
    text: &[u8],
    callback: Option<MessageCallback>,
    button_base: u8,
    buttons: &[Vec<u8>],
) -> Result<bool, StreamError> {
    if let Some(old) = cx.engine.tile_menu_by_id(1001)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    interface_update_menu_root(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let Some(created) = tile_read_xml(cx, root, "Data\\Menus\\message_menu.xml")? else {
        return Ok(false);
    };
    let Some(menu) = cx.engine.containing_menu(created) else {
        return Ok(false);
    };
    if cx.engine.menu(menu).id() != 1001 {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(false);
    }
    menu_register_tile(cx, menu, Some(created))?;
    let MenuClass::Message(fields) = cx.engine.menu(menu).class.clone() else {
        return Ok(false);
    };
    let tiles = fields.tiles_28;
    let (Some(message_text), Some(_), Some(_), first_button) =
        (tiles[1], tiles[2], tiles[0], tiles[3])
    else {
        // 005BC99D: DebugLog_Printf, where 005B61B6's MainMenu warns.
        cx.host.debug_log(
            "Message Menu Creation Failed... Are your menu and art resources up to date?",
        );
        return Ok(false);
    };
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    cx.world
        .resource_mut::<InterfaceManager>()
        .msg_box_button_pressed_0b0 = 0xFF;
    let mut labels = buttons.iter();
    let first = labels.next();
    if let Some(button) = first_button {
        match first {
            Some(label) if !label.is_empty() => {
                tile_set_string_trait(cx, button, TRAIT_STRING, label)?;
            }
            Some(_) => {
                // 005BCA1B: sOk.
                let ok = game_setting_text(cx, 0x00B3_8CF0);
                tile_set_string_trait(cx, button, TRAIT_STRING, &ok)?;
            }
            None => tile_set_numeric_trait(cx, button, TRAIT_VISIBLE, 1.0)?,
        }
    }
    let mut id = 4usize;
    for label in labels {
        if label.is_empty() {
            break;
        }
        id += 1;
        // 005BC5E0(id).
        match tiles.get(id - 1).copied().flatten() {
            Some(button) => {
                tile_set_string_trait(cx, button, TRAIT_STRING, label)?;
                tile_set_numeric_trait(cx, button, TRAIT_VISIBLE, 2.0)?;
            }
            None => cx.host.debug_log("Missing MessageMenu button"),
        }
    }
    // 005BCB24: sDefaultMessage.
    let shown = if text.is_empty() {
        game_setting_text(cx, 0x00B3_8E10)
    } else {
        text.to_vec()
    };
    tile_set_string_trait(cx, message_text, TRAIT_STRING, &shown)?;
    if let MenuClass::Message(message) = &mut cx.engine.menu_mut(menu).class {
        message.callback_5c = callback;
        message.button_base_60 = button_base;
    }
    let status = cx
        .world
        .resource::<InterfaceManager>()
        .menu_loading_status_008;
    tile_set_numeric_trait(cx, created, TRAIT_USER0, f32::from(u8::from(status != 1)))?;
    cx.host.play_ui_sound(0xB);
    tile_update_scene_graph(cx, created, false)?;
    tile_update_scene_graph(cx, created, false)?;
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    tile_set_numeric_trait(cx, created, TRAIT_VISIBLE, 2.0)?;
    Ok(true)
}

/// IDA: 005BCDD0 MessageMenu_HandleClick: ID 2 is 005BCCB0 (a scrolled text page, which no prompt here uses); from 4 on, 0057DE50(1), the four active fields cleared, +0xB0 = id + base - 4, 005BC6B0. Completion: 100% of the buttons.
fn message_menu_handle_click(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
) -> Result<(), StreamError> {
    if id < 4 {
        return Ok(());
    }
    cx.host.play_ui_sound(1);
    let base = match &cx.engine.menu(menu).class {
        MenuClass::Message(message) => message.button_base_60,
        _ => 0,
    };
    {
        let mut manager = cx.world.resource_mut::<InterfaceManager>();
        manager.active_tile_098 = None;
        manager.active_menu_09c = None;
        manager.unknown_0a0 = 0;
        manager.unknown_0a4 = 0;
        manager.msg_box_button_pressed_0b0 = (id + i32::from(base) - 4) as u8;
    }
    message_menu_close(cx)
}

/// IDA: 005BC6B0 MessageMenu_CloseWithSelection: destroyOnClose 2, then at once with no +0x60 node (the first-person HUD's) or a showing Load menu, which is every screen here. 005BC4A0 the destructor hands +0x5C to +0xB4. Completion: 100%.
pub fn message_menu_close(cx: &mut InterfaceCx) -> Result<(), StreamError> {
    let Some(tile) = cx.engine.tile_menu_by_id(1001) else {
        return Ok(());
    };
    let Some(menu) = cx.engine.containing_menu(tile) else {
        return Ok(());
    };
    tile_set_numeric_trait(cx, tile, TRAIT_DESTROY_ON_CLOSE, 2.0)?;
    let callback = match &cx.engine.menu(menu).class {
        MenuClass::Message(message) => message.callback_5c,
        _ => None,
    };
    cx.world
        .resource_mut::<InterfaceManager>()
        .message_callback_0b4 = callback;
    menu_destroy(cx, menu)
}

/// IDA: 005BC770: for a tile with an ID of 4 or more, the focus box (+0x30) takes depth = 00588D90(tile) - the depth of its ancestor below the root - 1, the tile's size and position, and shows; otherwise it hides. Completion: 100%.
fn message_menu_mouseover(
    cx: &mut InterfaceCx,
    menu: MenuId,
    tile: Option<TileId>,
) -> Result<(), StreamError> {
    let MenuClass::Message(message) = &cx.engine.menu(menu).class else {
        return Ok(());
    };
    let Some(focus) = message.tiles_28[2] else {
        return Ok(());
    };
    let target = tile.filter(|&tile| tile_numeric_trait(cx, tile, TRAIT_ID) >= 4.0);
    let Some(target) = target else {
        return tile_set_numeric_trait(cx, focus, TRAIT_VISIBLE, 1.0);
    };
    let depth = depth_to_locus(cx, target);
    let mut top = target;
    while let Some(parent) = cx.engine.tile(top).parent {
        if cx.engine.tile(parent).parent.is_none() {
            break;
        }
        top = parent;
    }
    let value =
        (f64::from(depth) - f64::from(tile_numeric_trait(cx, top, TRAIT_DEPTH)) - 1.0) as f32;
    tile_set_numeric_trait(cx, focus, TRAIT_DEPTH, value)?;
    for trait_id in [TRAIT_WIDTH, TRAIT_HEIGHT, TRAIT_X, TRAIT_Y] {
        let value = tile_numeric_trait(cx, target, trait_id);
        tile_set_numeric_trait(cx, focus, trait_id, value)?;
    }
    tile_set_numeric_trait(cx, focus, TRAIT_VISIBLE, 2.0)
}

/// IDA: 005B5D60's first half, for each callback: 00578D70 reads and clears +0xB0. Continue (005B5D60), New (005B5DF0), Exit (005B59B0), Load's confirm (005AE190). What a Yes asks of a later phase goes to `start_game` and the menu stays, a host stop by return. Completion: 100% of the branches on this side of LoadGame.
pub fn run_message_callback(
    cx: &mut InterfaceCx,
    callback: MessageCallback,
) -> Result<(), StreamError> {
    let button = {
        let mut manager = cx.world.resource_mut::<InterfaceManager>();
        let button = manager.msg_box_button_pressed_0b0;
        manager.msg_box_button_pressed_0b0 = 0xFF;
        button
    };
    match callback {
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
                    let text = game_setting_text(cx, 0x00B3_86D8);
                    // 005B5DC3: dword ptr 00B38CF0, sOk.
                    let ok = game_setting_text(cx, 0x00B3_8CF0);
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

/// IDA: 00588D90 Tile_GetDepthToLocusOrMenuRoot: the depth trait, plus the depth of each ancestor that has locus 2 or whose own parent is the menu root; the walk goes on to the root either way. Completion: 100%.
#[allow(clippy::assign_op_pattern)] // The operand order: the ancestor's depth first.
fn depth_to_locus(cx: &InterfaceCx, tile: TileId) -> f32 {
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let mut depth = tile_numeric_trait(cx, tile, TRAIT_DEPTH);
    let mut current = cx.engine.tile(tile).parent;
    while let Some(id) = current {
        let locus = find_value(cx, id, TRAIT_LOCUS)
            .is_some_and(|value| cx.engine.value(value).number_04 == 2.0);
        let parent = cx.engine.tile(id).parent;
        if locus || parent == root {
            depth = tile_numeric_trait(cx, id, TRAIT_DEPTH) + depth;
        }
        current = parent;
    }
    depth
}

/// IDA: 005AEA60 (hideReturn): an old Load menu deleted; 00584980; Tile_ReadXML(load_menu.xml); the class test; 00584880; the depth; the flag hides +0x28; 0045D450 unless the MainMenu is there, which keeps the list; 005AE6D0 for each save, or once with no file; 0058FBA0; 00585190(0); 0057F9F0(first row, mouseover, 0). Completion: 100% of the order. 0045E6A0 ConvertXboxSaves is carried: it scans for console saves, and its answer here is taken as false.
pub fn load_menu_create(
    cx: &mut InterfaceCx,
    hide_return: bool,
) -> Result<Option<TileId>, StreamError> {
    if let Some(old) = cx.engine.tile_menu_by_id(1038)
        && let Some(menu) = cx.engine.containing_menu(old)
    {
        menu_destroy(cx, menu)?;
    }
    let depth = menu_update_cursor_depth(cx)?;
    let root = cx.world.resource::<InterfaceManager>().menu_root_068;
    let Some(created) = tile_read_xml(cx, root, "Data\\Menus\\Options\\load_menu.xml")? else {
        return Ok(None);
    };
    let Some(menu) = cx.engine.containing_menu(created) else {
        return Ok(None);
    };
    if cx.engine.menu(menu).id() != 1038 {
        if cx.engine.menu(menu).tile_04.is_some() {
            menu_destroy(cx, menu)?;
        }
        return Ok(None);
    }
    menu_register_tile(cx, menu, Some(created))?;
    let stacking = tile_numeric_trait(cx, created, TRAIT_STACKING);
    if stacking == 6006.0 || stacking == 102.0 {
        tile_set_numeric_trait(cx, created, TRAIT_DEPTH, depth)?;
    }
    if hide_return {
        let first = match &cx.engine.menu(menu).class {
            MenuClass::Load(load) => load.field_id1_28,
            _ => None,
        };
        if let Some(first) = first {
            tile_set_numeric_trait(cx, first, TRAIT_VISIBLE, 1.0)?;
        }
    }
    let saves = cx.host.save_file_list();
    if let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class {
        load.saves_54.clone_from(&saves);
    }
    let mut first_row = None;
    if saves.is_empty() {
        first_row = load_menu_add_row(cx, menu, 0, None)?;
    }
    for (index, save) in saves.iter().enumerate() {
        let row = load_menu_add_row(cx, menu, index as i32, Some(save))?;
        if first_row.is_none() {
            first_row = row;
        }
    }
    tile_update_scene_graph(cx, created, false)?;
    let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
    menu_enable(cx, menu, false, frame_time)?;
    interface_dispatch_pointer_event(cx, first_row, TRAIT_MOUSEOVER)?;
    Ok(Some(created))
}

/// IDA: 005AE6D0 (buffer, index, file, count): a row from `save_game_template` in the list pane (+0x48). A file's caption is 00464060's "name\ntime" or "Save n - location\ntime"; no file gives the B38700 setting and hides the picture. user0 = index; the name with spaces as '_', 255 bytes; user3 = caption; id = index + 101; xdefault when it matches +0x5C. Completion: 100% of the order. `tile_build_template` says why the exe's "save_game_template" finds the XML's load_game_template.
fn load_menu_add_row(
    cx: &mut InterfaceCx,
    menu: MenuId,
    index: i32,
    save: Option<&SaveFile>,
) -> Result<Option<TileId>, StreamError> {
    let MenuClass::Load(load) = &cx.engine.menu(menu).class else {
        return Ok(None);
    };
    let (pane, picture, details) = (load.list_pane_48, load.field_id6_40, load.field_id7_44);
    let Some(pane) = pane else {
        return Ok(None);
    };
    let row = tile_build_template(cx, menu, pane, b"save_game_template")?;
    let caption = if let Some(save) = save {
        save.caption(
            &game_setting_text(cx, 0x00B3_86F0),
            &game_setting_text(cx, 0x00B3_8710),
            &game_setting_text(cx, 0x00B3_8718),
            &game_setting_text(cx, 0x00B3_86F8),
            &game_setting_text(cx, 0x00B3_8720),
        )
    } else {
        for hidden in [picture].into_iter().flatten() {
            tile_set_numeric_trait(cx, hidden, TRAIT_VISIBLE, 1.0)?;
        }
        if let Some(details) = details {
            tile_set_string_trait(cx, details, TRAIT_STRING, b"")?;
        }
        game_setting_text(cx, 0x00B3_8700)
    };
    let Some(row) = row else {
        return Ok(None);
    };
    tile_set_numeric_trait(cx, row, TRAIT_USER0, index as f32)?;
    let name: Vec<u8> = caption
        .iter()
        .take(255)
        .map(|&byte| if byte == b' ' { b'_' } else { byte })
        .collect();
    cx.engine.tile_mut(row).name_08 = name;
    tile_set_string_trait(cx, row, TRAIT_USER3, &caption)?;
    tile_set_numeric_trait(cx, row, TRAIT_ID, (index + 101) as f32)?;
    Ok(Some(row))
}

/// IDA: 005AE560 the row's hover: below ID 101 the focus box hides. From 101 on, 0057DE50(4), +0x30 = 0, the box's depth = 00588D90(tile) - user15, its size is the row's less 2, its place the row's cumulative place, it shows, +0x30 = the row, and 005AE240(user0). Completion: 100%.
fn load_menu_mouseover(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: Option<TileId>,
) -> Result<(), StreamError> {
    let Some(tile) = tile else {
        return Ok(());
    };
    let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class else {
        return Ok(());
    };
    let focus = load.focus_box_2c;
    if id < 101 {
        if let Some(focus) = focus {
            tile_set_numeric_trait(cx, focus, TRAIT_VISIBLE, 1.0)?;
        }
        return Ok(());
    }
    load.focused_row_30 = None;
    cx.host.play_ui_sound(4);
    let Some(focus) = focus else {
        return Ok(());
    };
    let depth = (f64::from(depth_to_locus(cx, tile))
        - f64::from(tile_numeric_trait(cx, focus, TRAIT_USER15))) as f32;
    tile_set_numeric_trait(cx, focus, TRAIT_DEPTH, depth)?;
    let width = tile_numeric_trait(cx, tile, TRAIT_WIDTH) - 2.0;
    tile_set_numeric_trait(cx, focus, TRAIT_WIDTH, width)?;
    let height = tile_numeric_trait(cx, tile, TRAIT_HEIGHT) - 2.0;
    tile_set_numeric_trait(cx, focus, TRAIT_HEIGHT, height)?;
    let [x, y] = tile_cumulative_position(cx, tile);
    tile_set_numeric_trait(cx, focus, TRAIT_X, x)?;
    tile_set_numeric_trait(cx, focus, TRAIT_Y, y)?;
    tile_set_numeric_trait(cx, focus, TRAIT_VISIBLE, 2.0)?;
    if let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class {
        load.focused_row_30 = Some(tile);
    }
    let index = x87::ftol(tile_numeric_trait(cx, tile, TRAIT_USER0));
    load_menu_show_details(cx, menu, index)
}

/// IDA: 005AE240(index): the picture's texture from 004610F0 (591A80 twice), the five lines "%s\n%s\n%s\n%s\n%s" in +0x44, the picture's user0 = the screenshot's height; a missing screenshot hides it and shows B38750. Index -1 hides. Completion: 100%.
fn load_menu_show_details(
    cx: &mut InterfaceCx,
    menu: MenuId,
    index: i32,
) -> Result<(), StreamError> {
    let MenuClass::Load(load) = &cx.engine.menu(menu).class else {
        return Ok(());
    };
    let (picture, details) = (load.field_id6_40, load.field_id7_44);
    let save = usize::try_from(index)
        .ok()
        .and_then(|index| load.saves_54.get(index).cloned());
    let (Some(picture), Some(details)) = (picture, details) else {
        return Ok(());
    };
    let Some(save) = save else {
        if index == -1 {
            tile_set_numeric_trait(cx, picture, TRAIT_VISIBLE, 1.0)?;
            tile_set_string_trait(cx, details, TRAIT_STRING, b"")?;
        }
        return Ok(());
    };
    image_set_preview(cx, picture, None)?;
    tile_set_numeric_trait(cx, picture, TRAIT_VISIBLE, 2.0)?;
    let text = save.detail_text(
        &game_setting_text(cx, 0x00B3_8720),
        &game_setting_text(cx, 0x00B3_8730),
        &game_setting_text(cx, 0x00B3_8728),
    );
    tile_set_string_trait(cx, details, TRAIT_STRING, &text)?;
    tile_set_numeric_trait(
        cx,
        picture,
        TRAIT_USER0,
        save.preview.as_ref().map_or(0, |preview| preview.height) as f32,
    )?;
    let preview = save.preview.clone();
    image_set_preview(cx, picture, preview.clone())?;
    if preview.is_none() {
        tile_set_numeric_trait(cx, picture, TRAIT_VISIBLE, 1.0)?;
        let text = game_setting_text(cx, 0x00B3_8750);
        tile_set_string_trait(cx, details, TRAIT_STRING, &text)?;
    }
    Ok(())
}

/// IDA: 00591A80 TileImage_SetTexture: filename = the empty text, +0x44 = the texture, flags |= 0x30. Completion: 100%.
fn image_set_preview(
    cx: &mut InterfaceCx,
    tile: TileId,
    preview: Option<std::sync::Arc<SavePreview>>,
) -> Result<(), StreamError> {
    tile_set_string_trait(cx, tile, TRAIT_FILENAME, b"")?;
    let tile = cx.engine.tile_mut(tile);
    tile.image_override_44 = preview;
    tile.flags_2c |= 0x30;
    Ok(())
}

/// IDA: 005AEED0: ignored while +0x64 is set. ID 1 is Return: the picture hides, 005AE080 (the picture cleared, destroyOnClose 2, 00584740), 005BDA20, 005B5B70(any saves), and the list stays while the MainMenu does. From 101: the row's user0 picks the save, kept in +0x4C; shift asks to delete (+0x58 the row; the delete is a later phase's), else the prompt, with B38780 when a game runs and B38768 otherwise; +0x64 = 1. Completion: 100% of what a main menu reaches. 005D2C60's slider is in `menu_handle_press`.
fn load_menu_handle_click(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: Option<TileId>,
) -> Result<(), StreamError> {
    let MenuClass::Load(load) = &cx.engine.menu(menu).class else {
        return Ok(());
    };
    if load.confirmation_pending_64 != 0 {
        return Ok(());
    }
    let picture = load.field_id6_40;
    let any_saves = !load.saves_54.is_empty();
    if id == 1 {
        if let Some(picture) = picture {
            tile_set_numeric_trait(cx, picture, TRAIT_VISIBLE, 1.0)?;
            image_set_preview(cx, picture, None)?;
        }
        if let Some(own) = cx.engine.menu(menu).tile_04 {
            tile_set_numeric_trait(cx, own, TRAIT_DESTROY_ON_CLOSE, 2.0)?;
        }
        let frame_time = cx.world.resource::<InterfaceManager>().frame_seconds_host;
        menu_begin_fade_out(cx, menu, frame_time)?;
        main_menu_refresh_continue(cx, any_saves)?;
    } else if id >= 101
        && let Some(row) = tile
    {
        let index = x87::ftol(tile_numeric_trait(cx, row, TRAIT_USER0));
        let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class else {
            return Ok(());
        };
        let Some(index) = usize::try_from(index)
            .ok()
            .filter(|&index| index < load.saves_54.len())
        else {
            return Ok(());
        };
        load.chosen_save_4c = Some(index);
        let shift = cx.world.resource::<InterfaceManager>().input_modifier_flags & 4 != 0;
        if shift {
            if let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class {
                load.confirm_row_58 = Some(row);
            }
            cx.host
                .warning("005AECA0: deleting a save is carried to its phase");
            return Ok(());
        }
        let text = game_setting_text(cx, 0x00B3_8768);
        let (yes, no) = yes_no(cx);
        show_message(
            cx,
            &text,
            Some(MessageCallback::LoadgameConfirm),
            1,
            &[yes, no],
        )?;
        if let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class {
            load.confirmation_pending_64 = 1;
        }
    }
    Ok(())
}

/// IDA: 005B5B70(anySaves): a change of `continue`'s visible, the priorities with it. Completion: 100%.
fn main_menu_refresh_continue(cx: &mut InterfaceCx, any: bool) -> Result<(), StreamError> {
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

/// IDA: 005AE155..005AE186 what 005AE140 stores at +0x50. Completion: 100%.
#[must_use]
pub fn load_press_scroll(virtual_height: f32, pointer: f32, marker_y: f32) -> f32 {
    let cursor = x87::ftol(virtual_height - (virtual_height * 0.5 + pointer));
    // 5AE181 `fisub`: the int isn't rounded first.
    (f64::from(marker_y) - f64::from(cursor)) as f32
}

/// IDA: vt+0x08 (005821F0's press on the active tile): 005AE140, +0x50 = cumulative y of ID 4 - ftol(h_v - (h_v / 2 + cursor z)), a float. Completion: 100%.
pub fn menu_handle_press(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    _tile: TileId,
) -> Result<(), StreamError> {
    if matches!(cx.engine.menu(menu).class, MenuClass::Hud(_)) {
        super::hud_main_menu::hud_handle_press(cx, menu, id);
        return Ok(());
    }
    let MenuClass::Load(load) = &cx.engine.menu(menu).class else {
        return Ok(());
    };
    let Some(marker) = load.field_id4_38 else {
        return Ok(());
    };
    let [_, y] = tile_cumulative_position(cx, marker);
    let pointer = cx.world.resource::<InterfaceManager>().cursor_state_020[2];
    let height = super::interface_manager::ui_virtual_height(cx.settings);
    let scroll = load_press_scroll(height, pointer, y);
    if let MenuClass::Load(load) = &mut cx.engine.menu_mut(menu).class {
        load.scroll_50 = scroll;
    }
    Ok(())
}

/// IDA: 005D2FCC..005D2FDF: wheel / -120 by `imul 0x77777777`: (hi - wheel) `sar` 6, plus that value's sign bit (`shr eax, 1Fh`), which is what rounds toward zero. Completion: 100%.
#[must_use]
pub fn wheel_notches(wheel: i32) -> i32 {
    let high = ((2_004_318_071_i64 * i64::from(wheel)) >> 32) as i32;
    let shifted = high.wrapping_sub(wheel) >> 6;
    shifted + i32::from(shifted < 0)
}

/// IDA: vt+0x28 (005821F0's wheel on the active tile): 005D2F90: the marker's (ID 4) user9 = wheel / -120 by the exe's multiply, then 0; when the pane's (ID 9) user7 moved, vt+0x14 (5D3038: 005AE220 hides the focus box) and 0057D730(0), whose false argument only clears +0x98 and +0x9C (57D78A): the next frame's pick finds the row. Completion: 100%.
pub fn menu_handle_wheel(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: TileId,
    wheel: i32,
) -> Result<(), StreamError> {
    let MenuClass::Load(load) = &cx.engine.menu(menu).class else {
        return Ok(());
    };
    let (Some(pane), Some(marker)) = (load.list_pane_48, load.field_id4_38) else {
        return Ok(());
    };
    let before = x87::ftol(tile_numeric_trait(cx, pane, 4021));
    let notches = wheel_notches(wheel);
    tile_set_numeric_trait(cx, marker, 4023, notches as f32)?;
    tile_set_numeric_trait(cx, marker, 4023, 0.0)?;
    // 5D300D `fild` of the ftol'd value, 5D3025 `fucompp` with the raw one.
    if before as f32 != tile_numeric_trait(cx, pane, 4021) {
        menu_handle_mouseout(cx, menu, id, Some(tile))?;
        let mut manager = cx.world.resource_mut::<InterfaceManager>();
        manager.active_menu_09c = None;
        manager.active_tile_098 = None;
    }
    Ok(())
}

/// IDA: 005D2DC7..005D2F57: the value 005D2D80 gives user5. Completion: 100%.
#[must_use]
pub fn slider_target(
    virtual_height: f32,
    pointer: f32,
    bar_y: f32,
    height: f32,
    minimum: f32,
    maximum: f32,
) -> f32 {
    // 5D2DE2 __ftol2_sse, then `fild`: the cursor is a whole number. 00593020's third read is the same one. All 24 bits.
    let cursor_y = x87::ftol(virtual_height - (virtual_height * 0.5 + pointer)) as f32;
    let raw = (cursor_y - bar_y) / height;
    // 5D2E14..5D2ED9: below 0 takes 0; 1 or more, or an unordered quotient, takes 1.
    let fraction = if raw < 0.0 {
        0.0
    } else if raw >= 1.0 || raw.is_nan() {
        1.0
    } else {
        raw
    };
    // 5D2F2C..5D2F57: min + (max - min) * fraction + 0.5, stored, __ftol2_sse, `fild`.
    x87::ftol((maximum - minimum) * fraction + minimum + 0.5) as f32
}

/// IDA: vt+0x20 (005821F0 while the left button is held): 005D2D80, the LoadgameMenu's slider base, on the marker (ID 4) only. The list's scroll is user5's expression: -1e9, then the target, then 0, with `slider_target`. Completion: 100%.
pub fn menu_handle_held(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    _tile: TileId,
) -> Result<(), StreamError> {
    if matches!(cx.engine.menu(menu).class, MenuClass::Hud(_)) {
        return super::hud_main_menu::hud_handle_held(cx, menu, id);
    }
    if id != 4 {
        return Ok(());
    }
    let MenuClass::Load(load) = &cx.engine.menu(menu).class else {
        return Ok(());
    };
    let Some(bar) = load.field_id3_34 else {
        return Ok(());
    };
    let maximum = tile_numeric_trait(cx, bar, TRAIT_USER2);
    let minimum = tile_numeric_trait(cx, bar, 4015);
    let pointer = {
        let manager = cx.world.resource::<InterfaceManager>();
        manager.cursor_state_020[2]
    };
    let virtual_height = super::interface_manager::ui_virtual_height(cx.settings);
    let [_, bar_y] = tile_cumulative_position(cx, bar);
    let height = tile_numeric_trait(cx, bar, TRAIT_HEIGHT);
    tile_set_numeric_trait(cx, bar, TRAIT_USER5, -1_000_000_000.0)?;
    let target = slider_target(virtual_height, pointer, bar_y, height, minimum, maximum);
    tile_set_numeric_trait(cx, bar, TRAIT_USER5, target)?;
    tile_set_numeric_trait(cx, bar, TRAIT_USER5, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamebryo::scene_update::NoSceneControllers;
    use crate::tes::interface_frame::MENU_STATE_VISIBLE;
    use crate::tes::interface_manager::{
        InterfaceBlock130, MenuModeChanged, interface_settings_from,
    };
    use crate::tes::settings::{SettingCollections, register_static_settings};
    use crate::tes::tile::{Menu, NullInterfaceHost, TileKind, with_interface_cx};
    use bevy_ecs::prelude::*;

    fn run(body: impl FnOnce(&mut InterfaceCx)) {
        let mut collections = SettingCollections::default();
        register_static_settings(&mut collections);
        let settings = interface_settings_from(&collections.ini, 1920, 1080, 8, Default::default());
        let mut world = World::new();
        world.insert_resource(InterfaceManager {
            block_130: Some(InterfaceBlock130 {
                unknown_0: 0,
                unknown_4: 0.0,
                unknown_8: 0.001,
                self_c: true,
                unknown_10: 0,
                nodes: Vec::new(),
                blend_progress: 0.0,
            }),
            ..InterfaceManager::default()
        });
        world.init_resource::<MenuModeChanged>();
        let mut host = NullInterfaceHost::default();
        with_interface_cx(
            &mut world,
            &settings,
            &mut host,
            &mut NoSceneControllers,
            body,
        );
    }

    // 005D2FCC: `imul`, `sub edx, ecx`, `sar edx, 6`, then `shr eax, 1Fh` of the shifted value: one notch toward the user is -1.
    #[test]
    fn a_wheel_notch_is_one_row() {
        assert_eq!(wheel_notches(120), -1);
        assert_eq!(wheel_notches(-120), 1);
        assert_eq!(wheel_notches(240), -2);
        assert_eq!(wheel_notches(-240), 2);
        assert_eq!(wheel_notches(0), 0);
    }

    // 005D2DE2 and 005D2F4E: the cursor and the target both go through __ftol2_sse, so the slider lands on whole rows.
    #[test]
    fn the_slider_lands_on_whole_values() {
        // cursor 1000 - (500 + 344.5) = 155.5, ftol 155; (155 - 100) / 200 = 0.275; 0 + 10 * 0.275 + 0.5 = 3.25, ftol 3.
        assert_eq!(slider_target(1000.0, 344.5, 100.0, 200.0, 0.0, 10.0), 3.0);
        // 5D2E76: at or past 1 takes 1; 5D2ED6: below 0 takes 0.
        assert_eq!(slider_target(1000.0, 0.0, 100.0, 200.0, 0.0, 10.0), 10.0);
        assert_eq!(slider_target(1000.0, 480.0, 100.0, 200.0, 0.0, 10.0), 0.0);
        // A bar of no height with the cursor on it: 0 / 0 is unordered, and 5D2E79 takes the 1.0.
        assert_eq!(slider_target(1000.0, 400.0, 100.0, 0.0, 0.0, 10.0), 10.0);
    }

    // 005AE17C..005AE186: `fisub` from the float y, stored as a float: only the cursor is truncated.
    #[test]
    fn the_press_keeps_the_markers_fraction() {
        assert_eq!(load_press_scroll(1000.0, 344.5, 100.25), 100.25 - 155.0);
    }

    // 005B642B `fld1; fdivrp` rounds at 24 bits, and 005B6438's `fmul` of the stored value rounds again: the version's 10 at zoom 75 is 0x41555556.
    #[test]
    fn the_version_divides_at_24_bits() {
        assert_eq!(version_position(10.0, 0.75).to_bits(), 0x4155_5556);
    }

    fn load_menu(cx: &mut InterfaceCx) -> (MenuId, TileId, TileId, TileId) {
        let root = tile_initialize(cx, TileKind::Rect, None, None, None).unwrap();
        let tile = tile_initialize(cx, TileKind::Menu, Some(root), None, None).unwrap();
        let pane = tile_initialize(cx, TileKind::Rect, Some(tile), None, None).unwrap();
        let marker = tile_initialize(cx, TileKind::Rect, Some(tile), None, None).unwrap();
        let focus = tile_initialize(cx, TileKind::Rect, Some(tile), None, None).unwrap();
        let menu = MenuId(cx.engine.menus.len() as u32);
        let mut owner = Menu::new(MenuClass::Load(LoadgameMenu {
            list_pane_48: Some(pane),
            field_id4_38: Some(marker),
            focus_box_2c: Some(focus),
            ..LoadgameMenu::default()
        }));
        owner.tile_04 = Some(tile);
        owner.state_24 = MENU_STATE_VISIBLE;
        cx.engine.menus.push(owner);
        cx.engine.tile_mut(tile).menu_44 = Some(menu);
        (menu, tile, pane, focus)
    }

    // 005D3025..005D3051: `fild` of the ftol'd value against the raw one, then vt+0x14 (005AE220 hides the box, where vt+0x10 shows it) and 0057D730(0), which clears +0x98 and +0x9C.
    #[test]
    fn a_wheel_that_moves_the_list_drops_the_focus() {
        run(|cx| {
            let (menu, tile, pane, focus) = load_menu(cx);
            tile_set_numeric_trait(cx, pane, 4021, 2.5).unwrap();
            tile_set_numeric_trait(cx, focus, TRAIT_VISIBLE, 2.0).unwrap();
            {
                let mut manager = cx.world.resource_mut::<InterfaceManager>();
                manager.active_tile_098 = Some(tile);
                manager.active_menu_09c = Some(menu);
            }
            menu_handle_wheel(cx, menu, 101, tile, 120).unwrap();
            assert_eq!(tile_numeric_trait(cx, focus, TRAIT_VISIBLE), 1.0);
            let manager = cx.world.resource::<InterfaceManager>();
            assert_eq!(manager.active_tile_098, None);
            assert_eq!(manager.active_menu_09c, None);
        });
    }
}
