//! `005CE570` RaceSexMenu vt+0x2C, the race menu's frame, as `005821F0` InterfaceManager_UpdateFrame calls it for each menu
//! under the root (`0058399D`): the player's animation data, the menu's pages, the player's movement and animation update
//! (`00603CA0`, whose head tracking and FaceGen timers are run-an's per-frame draws) and the camera at the face.
//!
//! Kept apart from `race_sex_menu` (the menu's creation), which another slice owns.
//!
//! `005FCAB0`, the actor's animation-group pick, runs its player arms (`actor_anim_play::actor_update_animation_groups`; D25
//! was its count until 2026-11-09); its weapon, bow, spell and movement arms are stops. `005CDEF0`, the pages' changes, runs
//! (`race_sex_menu::race_sex_menu_apply_page_changes`).

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use crate::tes::interface_frame::menu_enable;
use crate::tes::interface_manager::menu_is_visible;
use crate::tes::player::the_player;
use crate::tes::tile::{InterfaceCx, MenuId};
use crate::tes::tile_values::{TRAIT_VISIBLE, tile_numeric_trait};

/// IDA: 005CE570 RaceSexMenu vt+0x2C. Neither a MessageMenu (1001) shown nor the menu's tile visible other than 1.0: 005CE5A5
/// Menu_Enable(0) and return. Otherwise, with +0x8D0 clear (the ctor's 0; only 005CC200 clears it again) and 00B3B5D8 clear,
/// the player's vt+0x164 (`player_anim_data`): when set, `005FCAB0`(player, 1.0, 1.0) (`actor_update_animation_groups`) and 005CE60B
/// ActorAnimData_Update(player, frame time, -1.0); 005CE613 `005CDEF0`; 005CE624
/// Actor_UpdateMovementAndAnimation(player, 0.0); 005CE64B RaceSexMenu_UpdateCamera(1.0, +0x8A0 / 5.0) (+0x8A0 is the ctor's
/// 0.0 until vt+0x20 `005C3BE0`'s held drag on the player, id 2, writes it); then `0057D2F0` on +0x8EC, the name field's MenuTextInputState:
/// while enabled (`005CC200`'s click on the name tile, id 10), `0057DDE0` blinks the caret and `005C30C0` stores the text.
/// Completion: 100% of the order.
/// TODO: [ ] +0x8D0 as the gate of the animation arm here.
/// # Errors
/// A stop in a callee.
pub fn race_sex_menu_update_frame(
    cx: &mut InterfaceCx,
    menu: MenuId,
    frame_time: f32,
) -> Result<(), GameError> {
    let tile = cx.engine.menu(menu).tile_04;
    let shown = menu_is_visible(cx, 1001)
        || tile.is_some_and(|tile| tile_numeric_trait(cx, tile, TRAIT_VISIBLE) != 1.0);
    if !shown {
        return menu_enable(cx, menu, false, frame_time);
    }
    let player = the_player(cx.world).ok_or(GameError::unclassified(
        0x005CE5BF,
        "005CE5BF: a null g_thePlayer",
    ))?;
    let camera_byte = cx
        .world
        .get_resource::<super::race_sex_menu::RaceSexMenuStatics>()
        .map_or(0, |statics| statics.camera_byte_b3b5d8);
    if camera_byte == 0
        && let Some(anim) = crate::tes::actor_movement_update::player_anim_data(cx.world, player)
    {
        crate::tes::actor_anim_play::actor_update_animation_groups(cx, player)?;
        crate::tes::actor_animation::anim_data_update_idle(cx.world, anim, player, frame_time)?;
    }
    super::race_sex_menu::race_sex_menu_apply_page_changes(cx, menu)?;
    crate::tes::actor_movement_update::player_update_movement_and_animation(cx.world, player, 0.0)?;
    // 005CE629: +0x8A0 over 5.0 (dbl 00A3F3F0), a float.
    let turn = super::race_sex_menu::race_sex(cx, menu).camera_turn_8a0 / 5.0;
    super::race_sex_menu::race_sex_menu_update_camera(cx, menu, turn)?;
    // 005CE650: the name field, while enabled, blinks its caret (0057DDE0) and stores its text (005C30C0).
    if super::race_sex_menu::race_sex(cx, menu)
        .name_input_8ec
        .enabled_00
        != 0
    {
        let ticks = crate::tes::window::time_info_ticks_10(cx.world).ok_or(
            GameError::unclassified(0x0057DDE0, "0057DDE0: no host g_timeInfo"),
        )?;
        super::race_sex_menu::race_sex_mut(cx, menu)
            .name_input_8ec
            .update_caret_blink(ticks);
        super::race_sex_menu::race_sex_menu_store_name(cx, menu)?;
    }
    Ok(())
}
