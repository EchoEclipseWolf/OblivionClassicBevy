//! Phase 18: WinMain 0040EFF3..0040F068. `ebp` is 0 at 0040EFF9, read in the run. The IO thread's half and the two last stores are the driver's.
//!
//! Evidence: disassembly; `run-i-phase18` and `run-j-phase18-cover`, the default path only, under Wine.

use crate::tes::actor::TesObjectRefr;
use crate::tes::form_components::TesSpellList;
use crate::tes::magic_selection::{
    MagicItemModelRequest, find_starting_spell, player_clear_active_book,
    player_set_selected_magic_item,
};
use crate::tes::player::{player_save_initial_state, the_player};
use crate::tes::tile::InterfaceCx;
use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

/// IDA: WinMain 0040EFF3..0040F045: 005E02E0(player, 0), the spell list at base +0x58 until 00415360 answers 0x19, 00664850(player, 0) and 00664700(player, spell + 0x18). No such spell goes on to 0040F04A. Returns what 0041A610 asks to queue. Completion: 100% of a first start.
/// # Errors
/// A null player or base, or an arm below.
pub fn select_starting_spell(
    cx: &mut InterfaceCx,
) -> Result<Option<MagicItemModelRequest>, GameError> {
    let player = the_player(cx.world).ok_or(GameError::unclassified(
        0x0040EFF3,
        "0040EFF3: a null g_thePlayer",
    ))?;
    let base = cx
        .world
        .get::<TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c)
        .ok_or(GameError::unclassified(
            0x005E0304,
            "005E0304: a null base form",
        ))?;
    let spells = cx
        .world
        .get::<TesSpellList>(base)
        .map_or(Vec::new(), |list| list.spells_04.clone());
    let Some(spell) = find_starting_spell(cx.world, &spells)? else {
        return Ok(None);
    };
    player_clear_active_book(cx.world, player)?;
    player_set_selected_magic_item(cx, player, spell)
}

/// IDA: 0040F04A..0040F050 PlayerCharacter_SaveInitialState on the player.
/// # Errors
/// A null player, or as `player_save_initial_state`.
pub fn save_player_initial_state(world: &mut World) -> Result<(), GameError> {
    let player = the_player(world).ok_or(GameError::unclassified(
        0x0040F04A,
        "0040F04A: a null g_thePlayer",
    ))?;
    player_save_initial_state(world, player)
}
