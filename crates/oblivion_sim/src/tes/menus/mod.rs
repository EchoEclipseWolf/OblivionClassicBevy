//! M6: the three menu classes the first screens need, MainMenu (1044), LoadgameMenu (1038) and MessageMenu (1001), with the fields their `SetField` slots fill while `Tile_ReadXML` builds them. The classes' creation and click paths follow in this file. Fields keep the exe's offsets from the object's start, where `MainMenu_mbrs` in the IDB is listed from `Menu_mbrs`' end.

pub mod audio_menu;
pub mod breath_menu;
pub mod controls_menu;
pub mod gameplay_menu;
pub mod hud_info_menu;
pub mod hud_main_menu;
pub mod hud_reticle;
pub mod hud_subtitle_menu;
pub mod inventory_menu;
pub mod loadgame_menu;
pub mod loading_menu;
pub mod magic_menu;
pub mod magic_popup_menu;
pub mod main_menu;
pub mod map_menu;
pub mod message_menu;
pub mod options_menu;
pub mod pause_menu;
pub mod quantity_menu;
pub mod quick_keys_menu;
pub mod race_sex_menu;
pub mod race_sex_menu_frame;
pub mod recharge_menu;
pub mod save_load_menu;
pub mod save_menu;
pub mod sleep_wait_menu;
pub mod stats_menu;
pub mod text_edit_menu;

pub use loadgame_menu::*;
pub use main_menu::*;
pub use message_menu::*;
pub use save_load_menu::*;

#[cfg(test)]
use super::interface_manager::InterfaceManager;
use super::save_files::SaveFile;
use super::tile::{InterfaceCx, MenuClass, MenuId, TileId, depth_to_locus};
use crate::tes::game_settings::text_at_or_empty;

pub use super::menu_slots::{
    menu_handle_click, menu_handle_held, menu_handle_mouseout, menu_handle_mouseover,
    menu_handle_press, menu_handle_wheel, menu_set_field,
};

/// IDA: the ported classes' vt+4 SetField, which `menu_slots` reaches by each class's address: 005B5800 (MainMenu),
/// 005BC510 (MessageMenu), 005D2C60 (SaveLoadMenu_SetField, the Loadgame and Save menus); the other ported classes' own
/// SetField (each named at its `set_field`). Completion: 100% of the ported classes' slots.
pub fn set_field_by_class(cx: &mut InterfaceCx, menu: MenuId, id: i32, tile: TileId) {
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
        MenuClass::RaceSex(race_sex) => race_sex.set_field(id, tile),
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
        MenuClass::HudInfo(info) => crate::tes::hud_info_menu::hud_info_set_field(info, id, tile),
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
    use crate::tes::tile_values::*;
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
