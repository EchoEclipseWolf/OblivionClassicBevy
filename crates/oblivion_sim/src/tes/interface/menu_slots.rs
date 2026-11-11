//! IDA: N/A the menus' virtual slots, as the exe calls them: each menu class's vtable (`Menu_vtbl`, 15 slots, generated into
//! `menu_slots_table`) keyed by its GetID value, and one dispatch per slot. A slot is one of three things:
//! - the base `Menu` vtable's function (a nullsub, or `006FE080` / `00725DA0`, which return false): a faithful no-op;
//! - a class's own function with a port: the port, found by the function's address, so a body two classes share (the slider
//!   base `005D2D80`, the wheel `005D2F90`) is ported once;
//! - a class's own function with no port yet: counted in `MenuSlotBoundaries` by menu, slot and address, and the base's
//!   result given, so an unported handler shows in the counts instead of vanishing.
//!
//! Handlers are typed per slot (the registries below), so one registered under another slot's signature doesn't compile;
//! `the_registered_handlers_sit_in_their_slots` checks that each registered address is that slot's function in some class.
//! Porting a menu's input means adding its functions here.
//!
//! The slots no menu class overrides (+0x18, +0x1C, +0x24) and the ones the menus' creation and teardown call (+0x00,
//! +0x34 GetID) aren't dispatched here.

use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::menu_slots_table::{BASE_MENU_SLOTS, MENU_SLOT_ROWS};
use super::tile::{InterfaceCx, MenuId, TileId};

/// IDA: N/A one menu class: its GetID value, its name in the exe's RTTI, its vtable and the vtable's 15 slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuSlotRow {
    pub id: u32,
    pub class: &'static str,
    pub vtable: u32,
    pub slots: [u32; 15],
}

/// IDA: the `Menu_vtbl` slots the interface dispatches, by index (offset / 4); names from the IDB and xOBSE.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MenuSlot {
    /// +0x04 SetField(id, tile), from `0058B2F0`'s id arm while the XML builds the menu.
    SetField = 1,
    /// +0x08 the press on the active tile (`005821F0`).
    Press = 2,
    /// +0x0C HandleClick(id, tile).
    Click = 3,
    /// +0x10 HandleMouseover(id, tile).
    Mouseover = 4,
    /// +0x14 HandleMouseout(id, tile).
    Mouseout = 5,
    /// +0x20 the left button held on the active tile.
    Held = 8,
    /// +0x28 the wheel on the active tile.
    Wheel = 10,
    /// +0x2C the frame, from `0058399D`'s walk of the menu root's children.
    Frame = 11,
    /// +0x30 HandleKeyboardInput(key) -> bool, for the top stacked menu before a key becomes a navigation action.
    Keyboard = 12,
    /// +0x38 the navigation action (`00580BA0`) -> bool, for the topmost visible menu before the interface's own walk.
    Navigation = 14,
}

impl MenuSlot {
    /// IDA: N/A the slot's byte offset in the vtable.
    #[must_use]
    pub const fn offset(self) -> u32 {
        self as u32 * 4
    }
}

/// IDA: N/A the menu class row for a GetID value.
#[must_use]
pub fn menu_slot_row(id: u32) -> Option<&'static MenuSlotRow> {
    MENU_SLOT_ROWS.iter().find(|row| row.id == id)
}

/// IDA: N/A what a menu's slot holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotTarget {
    /// The base `Menu` vtable's function.
    Base(u32),
    /// The class's own function.
    Own(u32),
}

/// IDA: N/A the function in slot `slot` of the class whose GetID is `id`. A menu ID with no class row (none in the exe's
/// 46) is the base's.
#[must_use]
pub fn slot_target(id: u32, slot: MenuSlot) -> SlotTarget {
    let index = slot as usize;
    match menu_slot_row(id) {
        Some(row) if row.slots[index] != BASE_MENU_SLOTS[index] => {
            SlotTarget::Own(row.slots[index])
        }
        _ => SlotTarget::Base(BASE_MENU_SLOTS[index]),
    }
}

/// IDA: N/A the menus' own slot functions reached with no port: (menu ID, slot offset, function) and how many times.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct MenuSlotBoundaries {
    pub calls: BTreeMap<(u32, u32, u32), u32>,
}

// IDA: N/A a helper, with no address of its own.
fn count_unported(cx: &mut InterfaceCx, id: u32, slot: MenuSlot, address: u32) {
    *cx.world
        .get_resource_or_init::<MenuSlotBoundaries>()
        .calls
        .entry((id, slot.offset(), address))
        .or_default() += 1;
}

// IDA: N/A a helper, with no address of its own: the class's vt+0x34 GetID, which names the vtable the exe calls through.
fn menu_id(cx: &InterfaceCx, menu: MenuId) -> u32 {
    cx.engine.menu(menu).id()
}

/// IDA: N/A a SetField port.
pub type SetFieldFn = fn(&mut InterfaceCx, MenuId, i32, TileId);
/// IDA: N/A a HandleClick, HandleMouseover or HandleMouseout port: (menu, id, tile).
pub type TileEventFn = fn(&mut InterfaceCx, MenuId, i32, Option<TileId>) -> Result<(), GameError>;
/// IDA: N/A a press or held port: (menu, id, active tile).
pub type PointerFn = fn(&mut InterfaceCx, MenuId, i32, TileId) -> Result<(), GameError>;
/// IDA: N/A a wheel port: (menu, id, active tile, wheel delta).
pub type WheelFn = fn(&mut InterfaceCx, MenuId, i32, TileId, i32) -> Result<(), GameError>;
/// IDA: N/A a frame port: (menu, frame time).
pub type FrameFn = fn(&mut InterfaceCx, MenuId, f32) -> Result<(), GameError>;
/// IDA: N/A a HandleKeyboardInput port: (menu, the translated key) -> handled.
pub type KeyboardFn = fn(&mut InterfaceCx, MenuId, u32) -> Result<bool, GameError>;
/// IDA: N/A a navigation port: (menu, action, 1.0) -> handled.
pub type NavigationFn = fn(&mut InterfaceCx, MenuId, i32, f32) -> Result<bool, GameError>;

// IDA: N/A the SetField ports. Each class's arm of `menus::set_field_by_class` is its own SetField; `005D2C60`
// (SaveLoadMenu_SetField) serves both the Loadgame and the Save menu.
fn set_field_port(address: u32) -> Option<SetFieldFn> {
    match address {
        0x005B_5800 | 0x005B_C510 | 0x005D_9BE0 | 0x005C_0BB0 | 0x005B_6670 | 0x005B_13C0
        | 0x005A_9B40 | 0x005C_26D0 | 0x005D_2C60 | 0x005A_55F0 | 0x005A_4700 | 0x005A_8DC0
        | 0x005D_68D0 | 0x005B_D990 | 0x005B_D5B0 | 0x005C_0530 | 0x0059_5190 | 0x005A_3380
        | 0x0058_79A0 | 0x005B_3D80 | 0x0059_6440 | 0x005C_E730 | 0x0058_7AE0 => {
            Some(crate::tes::menus::set_field_by_class)
        }
        _ => None,
    }
}

// IDA: N/A the HandleClick ports.
fn click_port(address: u32) -> Option<TileEventFn> {
    match address {
        0x005B_64C0 => Some(crate::tes::menus::main_menu_handle_click),
        0x005B_CDD0 => Some(crate::tes::menus::message_menu_handle_click),
        0x005A_EED0 => Some(crate::tes::menus::load_menu_handle_click),
        0x005A_68C0 => Some(crate::tes::hud_main_menu::hud_handle_click),
        0x005C_C200 => Some(crate::tes::race_sex_menu::race_sex_menu_handle_click),
        _ => None,
    }
}

// IDA: N/A the HandleMouseover ports.
fn mouseover_port(address: u32) -> Option<TileEventFn> {
    match address {
        0x005B_D9E0 => Some(crate::tes::tile::menu_play_focus_sound),
        0x005B_C770 => Some(crate::tes::menus::message_menu_mouseover),
        0x005A_E560 => Some(crate::tes::menus::load_menu_mouseover),
        _ => None,
    }
}

// IDA: N/A the HandleMouseout ports.
fn mouseout_port(address: u32) -> Option<TileEventFn> {
    match address {
        0x005B_C890 => Some(crate::tes::menus::message_menu_mouseout),
        0x005A_E220 => Some(crate::tes::menus::load_menu_mouseout),
        _ => None,
    }
}

// IDA: N/A the press ports.
fn press_port(address: u32) -> Option<PointerFn> {
    match address {
        0x005A_E140 => Some(crate::tes::menus::load_menu_press),
        0x005A_5890 => Some(crate::tes::hud_main_menu::hud_handle_press),
        0x005C_2790 => Some(crate::tes::race_sex_menu::race_sex_menu_handle_press),
        _ => None,
    }
}

// IDA: N/A the held ports.
fn held_port(address: u32) -> Option<PointerFn> {
    match address {
        0x005D_2D80 => Some(crate::tes::menus::slider_base_held),
        0x005A_6940 => Some(crate::tes::hud_main_menu::hud_handle_held),
        0x005C_3BE0 => Some(crate::tes::race_sex_menu::race_sex_menu_handle_held),
        _ => None,
    }
}

// IDA: N/A the wheel ports.
fn wheel_port(address: u32) -> Option<WheelFn> {
    match address {
        0x005D_2F90 => Some(crate::tes::menus::slider_base_wheel),
        _ => None,
    }
}

// IDA: N/A the frame ports.
fn frame_port(address: u32) -> Option<FrameFn> {
    match address {
        0x005A_6DE0 => Some(crate::tes::hud_main_menu::hud_main_frame),
        0x005A_5540 => Some(crate::tes::hud_info_menu::hud_info_frame),
        0x005A_93B0 => Some(crate::tes::hud_subtitle_menu::hud_subtitle_frame),
        0x005A_A1B0 => Some(crate::tes::inventory_menu::inventory_frame),
        0x005B_70F0 => Some(crate::tes::map_menu::map_frame),
        0x005B_1410 => Some(crate::tes::magic_menu::magic_frame),
        0x005D_A180 => Some(crate::tes::stats_menu::stats_frame),
        0x005C_E570 => Some(crate::tes::race_sex_menu_frame::race_sex_menu_update_frame),
        _ => None,
    }
}

// IDA: N/A the HandleKeyboardInput ports (none yet: the seven classes with their own are counted).
fn keyboard_port(address: u32) -> Option<KeyboardFn> {
    match address {
        0x005C_3DD0 => Some(crate::tes::race_sex_menu::race_sex_menu_handle_keyboard),
        _ => None,
    }
}

// IDA: N/A the navigation ports.
fn navigation_port(address: u32) -> Option<NavigationFn> {
    match address {
        0x005A_EDF0 => Some(crate::tes::menus::load_menu_handle_action),
        _ => None,
    }
}

/// IDA: vt+0x04 SetField(id, tile), as `0058B2F0`'s id arm calls it. Completion: the dispatch; the ports are the registry's.
pub fn menu_set_field(cx: &mut InterfaceCx, menu: MenuId, id: i32, tile: TileId) {
    let menu_id = menu_id(cx, menu);
    if let SlotTarget::Own(address) = slot_target(menu_id, MenuSlot::SetField) {
        match set_field_port(address) {
            Some(port) => port(cx, menu, id, tile),
            None => count_unported(cx, menu_id, MenuSlot::SetField, address),
        }
    }
}

// IDA: N/A the dispatch shared by the three tile-event slots.
fn dispatch_tile_event(
    cx: &mut InterfaceCx,
    menu: MenuId,
    slot: MenuSlot,
    registry: fn(u32) -> Option<TileEventFn>,
    id: i32,
    tile: Option<TileId>,
) -> Result<(), GameError> {
    let menu_id = menu_id(cx, menu);
    if let SlotTarget::Own(address) = slot_target(menu_id, slot) {
        match registry(address) {
            Some(port) => return port(cx, menu, id, tile),
            None => count_unported(cx, menu_id, slot, address),
        }
    }
    Ok(())
}

/// IDA: vt+0x0C HandleClick(id, tile). Completion: the dispatch.
/// # Errors
/// A stop in the class's port.
pub fn menu_handle_click(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: Option<TileId>,
) -> Result<(), GameError> {
    dispatch_tile_event(cx, menu, MenuSlot::Click, click_port, id, tile)
}

/// IDA: vt+0x10 HandleMouseover(id, tile). Completion: the dispatch.
/// # Errors
/// A stop in the class's port.
pub fn menu_handle_mouseover(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: Option<TileId>,
) -> Result<(), GameError> {
    dispatch_tile_event(cx, menu, MenuSlot::Mouseover, mouseover_port, id, tile)
}

/// IDA: vt+0x14 HandleMouseout(id, tile). Completion: the dispatch.
/// # Errors
/// A stop in the class's port.
pub fn menu_handle_mouseout(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: Option<TileId>,
) -> Result<(), GameError> {
    dispatch_tile_event(cx, menu, MenuSlot::Mouseout, mouseout_port, id, tile)
}

// IDA: N/A the dispatch shared by the press and held slots.
fn dispatch_pointer(
    cx: &mut InterfaceCx,
    menu: MenuId,
    slot: MenuSlot,
    registry: fn(u32) -> Option<PointerFn>,
    id: i32,
    tile: TileId,
) -> Result<(), GameError> {
    let menu_id = menu_id(cx, menu);
    if let SlotTarget::Own(address) = slot_target(menu_id, slot) {
        match registry(address) {
            Some(port) => return port(cx, menu, id, tile),
            None => count_unported(cx, menu_id, slot, address),
        }
    }
    Ok(())
}

/// IDA: vt+0x08, `005821F0`'s press on the active tile. Completion: the dispatch.
/// # Errors
/// A stop in the class's port.
pub fn menu_handle_press(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: TileId,
) -> Result<(), GameError> {
    dispatch_pointer(cx, menu, MenuSlot::Press, press_port, id, tile)
}

/// IDA: vt+0x20, `005821F0` while the left button is held on the active tile. Completion: the dispatch.
/// # Errors
/// A stop in the class's port.
pub fn menu_handle_held(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: TileId,
) -> Result<(), GameError> {
    dispatch_pointer(cx, menu, MenuSlot::Held, held_port, id, tile)
}

/// IDA: vt+0x28, `005821F0`'s wheel on the active tile. Completion: the dispatch.
/// # Errors
/// A stop in the class's port.
pub fn menu_handle_wheel(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: TileId,
    wheel: i32,
) -> Result<(), GameError> {
    let menu_id = menu_id(cx, menu);
    if let SlotTarget::Own(address) = slot_target(menu_id, MenuSlot::Wheel) {
        match wheel_port(address) {
            Some(port) => return port(cx, menu, id, tile, wheel),
            None => count_unported(cx, menu_id, MenuSlot::Wheel, address),
        }
    }
    Ok(())
}

/// IDA: vt+0x2C, the menu's frame, as `0058399D` calls it for each child of the menu root. Completion: the dispatch.
/// # Errors
/// A stop in the class's port.
pub fn menu_update_frame(
    cx: &mut InterfaceCx,
    menu: MenuId,
    frame_time: f32,
) -> Result<(), GameError> {
    let menu_id = menu_id(cx, menu);
    if let SlotTarget::Own(address) = slot_target(menu_id, MenuSlot::Frame) {
        match frame_port(address) {
            Some(port) => return port(cx, menu, frame_time),
            None => count_unported(cx, menu_id, MenuSlot::Frame, address),
        }
    }
    Ok(())
}

/// IDA: vt+0x30 HandleKeyboardInput(key), as `005821F0` asks the top stacked menu before a key becomes a navigation action;
/// the base `006FE080` returns false. Completion: the dispatch.
/// # Errors
/// A stop in the class's port.
pub fn menu_handle_keyboard(
    cx: &mut InterfaceCx,
    menu: MenuId,
    key: u32,
) -> Result<bool, GameError> {
    let menu_id = menu_id(cx, menu);
    if let SlotTarget::Own(address) = slot_target(menu_id, MenuSlot::Keyboard) {
        match keyboard_port(address) {
            Some(port) => return port(cx, menu, key),
            None => count_unported(cx, menu_id, MenuSlot::Keyboard, address),
        }
    }
    Ok(false)
}

/// IDA: vt+0x38(action, 1.0), as `00580BA0` asks the topmost visible menu first; the base `00725DA0` returns false.
/// Completion: the dispatch.
/// # Errors
/// A stop in the class's port.
pub fn menu_handle_navigation(
    cx: &mut InterfaceCx,
    menu: MenuId,
    action: i32,
) -> Result<bool, GameError> {
    let menu_id = menu_id(cx, menu);
    if let SlotTarget::Own(address) = slot_target(menu_id, MenuSlot::Navigation) {
        match navigation_port(address) {
            Some(port) => return port(cx, menu, action, 1.0),
            None => count_unported(cx, menu_id, MenuSlot::Navigation, address),
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLOTS: [MenuSlot; 10] = [
        MenuSlot::SetField,
        MenuSlot::Press,
        MenuSlot::Click,
        MenuSlot::Mouseover,
        MenuSlot::Mouseout,
        MenuSlot::Held,
        MenuSlot::Wheel,
        MenuSlot::Frame,
        MenuSlot::Keyboard,
        MenuSlot::Navigation,
    ];

    // IDA: N/A whether the slot's registry has a port for `address`.
    fn registered(slot: MenuSlot, address: u32) -> bool {
        match slot {
            MenuSlot::SetField => set_field_port(address).is_some(),
            MenuSlot::Click => click_port(address).is_some(),
            MenuSlot::Mouseover => mouseover_port(address).is_some(),
            MenuSlot::Mouseout => mouseout_port(address).is_some(),
            MenuSlot::Press => press_port(address).is_some(),
            MenuSlot::Held => held_port(address).is_some(),
            MenuSlot::Wheel => wheel_port(address).is_some(),
            MenuSlot::Frame => frame_port(address).is_some(),
            MenuSlot::Keyboard => keyboard_port(address).is_some(),
            MenuSlot::Navigation => navigation_port(address).is_some(),
        }
    }

    // The table holds the 46 classes, one row per GetID value.
    #[test]
    fn the_table_has_one_row_per_menu_id() {
        assert_eq!(MENU_SLOT_ROWS.len(), 46);
        let mut ids: Vec<u32> = MENU_SLOT_ROWS.iter().map(|row| row.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 46);
        assert_eq!(
            menu_slot_row(1036).map(|row| row.class),
            Some("RaceSexMenu")
        );
    }

    // A port registered under a slot is that slot's function in at least one class, so a handler can't sit in the wrong
    // slot; and no port is registered for a base function, which the dispatch never asks for.
    #[test]
    fn the_registered_handlers_sit_in_their_slots() {
        let mut addresses: Vec<u32> = MENU_SLOT_ROWS.iter().flat_map(|row| row.slots).collect();
        addresses.extend(BASE_MENU_SLOTS);
        addresses.sort_unstable();
        addresses.dedup();
        for slot in SLOTS {
            let index = slot as usize;
            for &address in &addresses {
                if !registered(slot, address) {
                    continue;
                }
                assert_ne!(
                    address, BASE_MENU_SLOTS[index],
                    "{slot:?} {address:08X} is the base's"
                );
                assert!(
                    MENU_SLOT_ROWS.iter().any(|row| row.slots[index] == address),
                    "{slot:?} {address:08X} is no class's {slot:?}"
                );
            }
        }
    }

    // The race menu's own slots: press, click, held, wheel, frame and the keyboard (005C3DD0, the name field's typing) are
    // its own functions; mouseover is the base's.
    #[test]
    fn the_race_menu_slots() {
        assert_eq!(
            slot_target(1036, MenuSlot::Click),
            SlotTarget::Own(0x005C_C200)
        );
        assert_eq!(
            slot_target(1036, MenuSlot::Press),
            SlotTarget::Own(0x005C_2790)
        );
        assert_eq!(
            slot_target(1036, MenuSlot::Held),
            SlotTarget::Own(0x005C_3BE0)
        );
        assert_eq!(
            slot_target(1036, MenuSlot::Wheel),
            SlotTarget::Own(0x005C_2870)
        );
        assert_eq!(
            slot_target(1036, MenuSlot::Frame),
            SlotTarget::Own(0x005C_E570)
        );
        assert_eq!(
            slot_target(1036, MenuSlot::Mouseover),
            SlotTarget::Base(0x0060_CF60)
        );
        assert_eq!(
            slot_target(1036, MenuSlot::Keyboard),
            SlotTarget::Own(0x005C_3DD0)
        );
    }
}
