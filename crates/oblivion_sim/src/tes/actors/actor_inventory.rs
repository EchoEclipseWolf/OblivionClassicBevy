//! The actor's own item operations: 005FBEC0 Actor_AddInventoryItem (the Character's vt+0x114), 005FAEA0
//! Actor_UseInventoryItem, and 0060E7C0 Character_FindBestWeaponAndEquipAmmo (vt+0x2B8). The container walks they make are
//! in `inventory`; the equip itself (005F3140) is `actor_enable::equip_item`.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::actor::{MobileObject, TesObjectRefr, invalidate_armor_rating_cache};
use super::actor_process::MiddleHighProcess;
use super::inventory::{
    container_item_count, ensure_container_changes, put_container_changes,
    reference_add_inventory_item,
};
use crate::tes::base_extra_list::{EXTRA_REFERENCE_POINTER, ExtraDataList};
use crate::tes::forms_init::form_type;
use crate::tes::player::the_player;

const TYPE_ARMOR: u8 = 0x14;
const TYPE_LIGHT: u8 = 0x1A;
const TYPE_WEAPON: u8 = 0x21;
const TYPE_AMMO: u8 = 0x22;
const TYPE_NPC: u8 = 0x23;

/// IDA: 0060E7C0 Character_FindBestWeaponAndEquipAmmo(character, -1) (the actor's vt+0x2B8): with a changes block, 0048BDA0
/// on it and TESObjectREFR_GetActorBaseForm(0) with its rating out (0.0 with no block), the rating into the process's
/// vt+0x328 (00629620 MiddleHighProcess_SetWeaponRating, +0xF8); for a non-player whose choice is a bow (+0x90 5) and with
/// no equipped ammo, the best ammo's use (a stop). Answers the selection's form. Completion: a melee choice.
fn find_best_weapon_and_equip_ammo(
    world: &mut World,
    actor: Entity,
) -> Result<Option<Entity>, GameError> {
    let has_changes = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .extra_data_44
        .get(crate::tes::base_extra_list::EXTRA_CONTAINER_CHANGES)
        .is_some();
    let mut rating = 0.0_f32;
    let mut best = None;
    if has_changes {
        let base = world
            .get::<TesObjectRefr>(actor)
            .expect("an actor")
            .base_form_1c
            .ok_or(GameError::stop(0x0060E7F0, "an actor with no base"))?;
        let changes = ensure_container_changes(world, actor)?;
        let found = crate::tes::equipment::best_weapon_rated(world, &changes, base, actor, false);
        put_container_changes(world, actor, changes);
        let (chosen, rated) = found?;
        best = chosen.map(|(form, _)| form);
        rating = rated;
    }
    if let Some(process) = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
    {
        match world.get_mut::<MiddleHighProcess>(process) {
            Some(mut middle) => middle.weapon_rating_f8 = rating,
            None => {
                super::actor_enable::low_process_only(
                    world,
                    process,
                    "0060E81D: a Low process's vt+0x328",
                )?;
            }
        }
    }
    if let Some(weapon) = best
        && the_player(world) != Some(actor)
        && crate::tes::forms_items::weapon_type_of(world, weapon) == 5
    {
        return Err(GameError::stop(
            0x0060E85C,
            "a bow's ammo for the best weapon",
        ));
    }
    Ok(best)
}

/// IDA: 005FBEC0 Actor_AddInventoryItem(actor, item, stack, count), the Character's vt+0x114: a stack owned by the player's
/// base goes back to the player (the player only); a stack's reference pointer (0x22) is a stop; then 004D8720. An NPC
/// given a weapon runs vt+0x2B8(-1); a selection for a non-player while the player's +0x124 is clear is used
/// (005FAEA0(form, 1, null, 1, 0)); then a combat package (vt+0x334(1)) with a weapon draws it, a stop. The selection is freed.
/// Completion: a non-player outside a combat package, with no reference stack.
/// # Errors
/// The stops above, or one of the use's.
pub fn actor_add_inventory_item(
    world: &mut World,
    actor: Entity,
    item: Entity,
    stack: Option<ExtraDataList>,
    count: i32,
) -> Result<(), GameError> {
    let player = the_player(world);
    if let Some(list) = &stack {
        if player == Some(actor) {
            return Err(GameError::stop(0x005FBEE0, "the player's own stack"));
        }
        if list.get(EXTRA_REFERENCE_POINTER).is_some() {
            return Err(GameError::stop(
                0x005FBF02,
                "a stack with a reference pointer",
            ));
        }
    }
    reference_add_inventory_item(world, actor, item, stack, count)?;
    let base = world
        .get::<TesObjectRefr>(actor)
        .and_then(|reference| reference.base_form_1c);
    if base.is_none_or(|base| form_type(world, base) != TYPE_NPC)
        || form_type(world, item) != TYPE_WEAPON
    {
        return Ok(());
    }
    let Some(best) = find_best_weapon_and_equip_ammo(world, actor)? else {
        return Ok(());
    };
    let player_124 = player
        .and_then(|player| world.get::<super::player::PlayerCharacter>(player))
        .map_or(0, |data| data.byte_124);
    if Some(actor) != player && player_124 == 0 {
        use_inventory_item(world, actor, best, 1, true, false)?;
        if crate::tes::actor_package::actor_in_combat_package(world, actor, true)? {
            return Err(GameError::stop(
                0x005FC028,
                "drawing the added weapon in a combat package",
            ));
        }
    }
    Ok(())
}

/// IDA: 005FAEA0 Actor_UseInventoryItem(actor, item, count, null, immediate, override_lock) for a non-player, with no stack:
/// the changes (00485E00) and 004869C0's count above 0 decide; the health is TESForm_GetHealth's. A weapon (with no horse):
/// the count is at most 1; a health above 0.0 runs 005F3140 Actor_EquipItem; then the player's no-charge byte (+0x7F9)
/// clears and the armor rating cache (+0x108) is -1.0. An ARMO: slot 13 with no health is the player's message; otherwise
/// the cache, then 005E3DE0 Actor_CanEquipEnchantedItem (true with no enchantment), 005F3140, and 005E48D0 Actor_CastEquippedItemEnchantment (
/// a stop). Both end at LABEL_114: no repair hammer or stone arm for a non-player, then with no save active 004FBF90(actor,
/// null, 2) and 004FBF90(item, the actor's list, 2). Completion: a non-player's unenchanted weapon or non-shield armor.
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "the x87 compare: a NaN takes the negated arm"
)]
pub fn use_inventory_item(
    world: &mut World,
    actor: Entity,
    item: Entity,
    count: i32,
    _immediate: bool,
    _override_lock: bool,
) -> Result<(), GameError> {
    if the_player(world) == Some(actor) {
        return Err(GameError::stop(
            0x005FAEA0,
            "the player's use of an inventory item",
        ));
    }
    let kind = form_type(world, item);
    if kind == TYPE_AMMO {
        return Err(GameError::stop(0x005FAEC0, "using ammo"));
    }
    let changes = ensure_container_changes(world, actor)?;
    let held = container_item_count(world, changes.owner_04, &changes.entries_00, item);
    put_container_changes(world, actor, changes);
    if held <= 0 {
        return Ok(());
    }
    let _ = count;
    let health = super::equipment::stack_health(world, item, None);
    match kind {
        TYPE_WEAPON => {
            if world
                .get::<TesObjectRefr>(actor)
                .expect("an actor")
                .extra_data_44
                .get(crate::tes::base_extra_list::EXTRA_HORSE)
                .is_some()
            {
                return Err(GameError::stop(0x005FB2F9, "a weapon used on horseback"));
            }
            if health > 0.0 {
                super::equipment::equip_item(world, actor, item)?;
            }
            if let Some(player) = the_player(world)
                && let Some(mut data) = world.get_mut::<super::player::PlayerCharacter>(player)
            {
                data.no_charge_message_shown_7f9 = 0;
            }
            invalidate_armor_rating_cache(world, actor);
        }
        TYPE_ARMOR => {
            if crate::tes::form::biped_covers(world, item, 13) && !(health > 0.0) {
                return Ok(());
            }
            invalidate_armor_rating_cache(world, actor);
            if super::equipment::enchantment_of(world, item).is_some() {
                return Err(GameError::stop(
                    0x005FB0B4,
                    "an enchanted item's equip gate (005E3DE0)",
                ));
            }
            super::equipment::equip_item(world, actor, item)?;
        }
        TYPE_LIGHT => return Err(GameError::stop(0x005FB0D0, "using a light")),
        _ => {
            return Err(GameError::stop(
                0x005FAF8F,
                "using an item other than a weapon or armor",
            ));
        }
    }
    if !crate::tes::forms_references::save_load_active(world)? {
        crate::tes::ai_package::script_event_or(world, actor, 2, Some(item));
    }
    Ok(())
}
