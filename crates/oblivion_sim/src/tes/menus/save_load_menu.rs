//! SaveLoadMenu, the base the Loadgame and Save menus share: its wheel (005D2F90) and its slider drag (005D2D80).

use oblivion_core::error::GameError;
use oblivion_core::x87;

use crate::tes::interface_manager::InterfaceManager;
use crate::tes::interface_scene::tile_cumulative_position;
use crate::tes::menu_slots::menu_handle_mouseout;
use crate::tes::tile::{InterfaceCx, MenuClass, MenuId, TileId};
use crate::tes::tile_values::*;

/// IDA: 005D2FCC..005D2FDF: wheel / -120 by `imul 0x77777777`: (hi - wheel) `sar` 6, plus that value's sign bit (`shr eax, 1Fh`), which is what rounds toward zero. Completion: 100%.
#[must_use]
pub fn wheel_notches(wheel: i32) -> i32 {
    let high = ((2_004_318_071_i64 * i64::from(wheel)) >> 32) as i32;
    let shifted = high.wrapping_sub(wheel) >> 6;
    shifted + i32::from(shifted < 0)
}

/// IDA: 005D2F90, vt+0x28 of the Loadgame and Save menus (005821F0's wheel on the active tile): the marker's (ID 4) user9 = wheel / -120 by the exe's multiply, then 0; when the pane's (ID 9) user7 moved, vt+0x14 (5D3038: 005AE220 hides the focus box) and 0057D730(0), whose false argument only clears +0x98 and +0x9C (57D78A): the next frame's pick finds the row. Completion: 100%.
/// TODO: [ ] the SaveMenu's fields (a stop).
/// # Errors
/// A SaveMenu, or a stop in a trait write.
pub fn slider_base_wheel(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    tile: TileId,
    wheel: i32,
) -> Result<(), GameError> {
    let MenuClass::Load(load) = &cx.engine.menu(menu).class else {
        return Err(GameError::unclassified(
            0x005D2F90,
            "005D2F90: the wheel on a SaveMenu",
        ));
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

/// IDA: 005D2D80, vt+0x20 of the Loadgame and Save menus (005821F0 while the left button is held): the slider base, on the marker (ID 4) only. The list's scroll is user5's expression: -1e9, then the target, then 0, with `slider_target`. Completion: 100%.
/// TODO: [ ] the SaveMenu's fields (a stop).
/// # Errors
/// A SaveMenu, or a stop in a trait write.
pub fn slider_base_held(
    cx: &mut InterfaceCx,
    menu: MenuId,
    id: i32,
    _tile: TileId,
) -> Result<(), GameError> {
    if id != 4 {
        return Ok(());
    }
    let MenuClass::Load(load) = &cx.engine.menu(menu).class else {
        return Err(GameError::unclassified(
            0x005D2D80,
            "005D2D80: the slider base on a SaveMenu",
        ));
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
    let virtual_height = crate::tes::interface_manager::ui_virtual_height(cx.settings);
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
}
