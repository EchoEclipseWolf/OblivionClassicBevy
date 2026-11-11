//! A BoundItemEffect's equip of its associated form: 00690AF0 BoundItemEffect_EquipAssociatedItem and the equipped arm of
//! 006918D0 BoundItemEffect_UpdateEffect. The item operations they call are the actor's (`actor_inventory`) and the
//! container's (`inventory`).
//!
//! The original's process entries (+0xE4 and the rest) hold the changes' own stack pointers, so a lock set through the
//! process lands on the container's stack. The port's `EquippedEntry` holds no stacks: the stack is found again as the
//! entry's first worn one (00485FA0's walk), which is the stack 00489C30 made worn and gave the process. Host substitution.
//! Evidence: process-manager-2026-11-10/prison-effects (seed 6AF306F7), the eighth original run.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::actor::{MobileObject, TesObjectRefr};
use super::actor_process::MiddleHighProcess;
use super::inventory::{InventoryEntry, ensure_container_changes, put_container_changes};
use super::magic_cast::ActiveEffect;
use crate::tes::base_extra_list::{EXTRA_WORN, ExtraDataList};
use crate::tes::forms_init::form_type;
use crate::tes::magic_cast::effect_item;
use crate::tes::player::the_player;

const TYPE_ARMOR: u8 = 0x14;
const TYPE_WEAPON: u8 = 0x21;

/// IDA: 00690AF0 BoundItemEffect_EquipAssociatedItem(effect). The target's reference (+0x20, MagicTarget_GetReference); the
/// associated form (+0x38) cast to TESObjectWEAP, then TESObjectARMO; neither dispels (ActiveEffect_Dispel, a stop).
///
/// A weapon: an equipped worn weapon (process vt+0xEC(1)) is displaced, a stop. vt+0x114 (005FBEC0) adds the weapon with no
/// stack; unless the equipped weapon is now the bound one, 005FAEA0(weapon, 1, null, 1, 1). With a worn weapon entry its first
/// stack takes 0041F370(1) and 0041F2F0, else +0x88 = 1. Then vt+0x300(1) (0064AD20, +0x114) and vt+0x3D4 (00629490, +0x140).
///
/// An armor: the sex is the NPC base's (0 otherwise). For each slot 0..15 the armor covers, 00486790's selection: its form's
/// biped model (00469330) loads now once (+0x87, read before the loop), its stack takes 004FBF90's OnUnequip (8) on the
/// stack and on the actor for the form, and 0041F6A0(0) (0x1B off); an emptied stack leaves the selection's list; the
/// selection is copied (004844A0) into +0x40[slot] and freed. Then a new list with 0041F2F0's 0x50, vt+0x114 with it, and
/// 005FAEA0(armor, 1, null, 1, 0); 00485FA0(armor, 1) on the changes: found takes 0041F370(1), else +0x88 = 1.
/// Completion: no displaced weapon; slot stacks without scripts.
/// # Errors
/// The stops above, or one of the inventory or equip calls'.
pub fn equip_associated_item(world: &mut World, entity: Entity) -> Result<(), GameError> {
    let effect = world.get::<ActiveEffect>(entity).expect("an effect");
    let reference = effect.target_20.expect("a bound effect's target");
    let associated = effect.associated_38.ok_or_else(|| {
        GameError::stop(0x00690B4E, "dispel a bound effect with no associated form")
    })?;
    match form_type(world, associated) {
        TYPE_WEAPON => equip_bound_weapon(world, entity, reference, associated),
        TYPE_ARMOR => equip_bound_armor(world, entity, reference, associated),
        _ => Err(GameError::stop(
            0x00690E8A,
            "dispel a bound effect whose form is no weapon or armor",
        )),
    }
}

// IDA: 00690B6B..00690D9C, the weapon arm of 00690AF0.
fn equip_bound_weapon(
    world: &mut World,
    entity: Entity,
    reference: Entity,
    weapon: Entity,
) -> Result<(), GameError> {
    if super::actor_process::actor_worn_weapon(world, reference).is_some() {
        return Err(GameError::stop(
            0x00690B8C,
            "displace the equipped weapon for a bound one",
        ));
    }
    super::actor_inventory::actor_add_inventory_item(world, reference, weapon, None, 1)?;
    if super::actor_process::actor_worn_weapon(world, reference) != Some(weapon) {
        super::actor_inventory::use_inventory_item(world, reference, weapon, 1, true, true)?;
    }
    let process = world
        .get::<MobileObject>(reference)
        .and_then(|mobile| mobile.process_58)
        .ok_or(GameError::stop(
            0x00690D3D,
            "a bound weapon's target with no process",
        ))?;
    let worn = super::actor_process::actor_worn_weapon(world, reference);
    let mut locked = false;
    if let Some(form) = worn {
        let mut changes = ensure_container_changes(world, reference)?;
        if let Some((at, index)) =
            super::inventory::find_worn_stack_for_form(&mut changes, form, false)
        {
            let list = &mut changes.entries_00[at].lists_00.as_mut().expect("found")[index];
            list.set_equip_locked(true);
            list.ensure_bound_armor();
            locked = true;
        }
        put_container_changes(world, reference, changes);
    }
    if !locked {
        world
            .get_mut::<ActiveEffect>(entity)
            .unwrap()
            .pending_equip_lock_88 = 1;
    }
    let Some(mut middle) = world.get_mut::<MiddleHighProcess>(process) else {
        return super::actor_enable::low_process_only(
            world,
            process,
            "00690D7E: a Low process's bound weapon",
        );
    };
    // 0064AD20 MiddleHighProcess_SetWeaponDrawRequested, 00629490 MiddleHighProcess_SetBoundWeapon.
    middle.weapon_draw_requested_114 = 1;
    middle.bound_weapon_140 = Some(weapon);
    Ok(())
}

// IDA: 00690DA3..00690E85, the armor arm of 00690AF0.
fn equip_bound_armor(
    world: &mut World,
    entity: Entity,
    reference: Entity,
    armor: Entity,
) -> Result<(), GameError> {
    let sex = world
        .get::<TesObjectRefr>(reference)
        .and_then(|refr| refr.base_form_1c)
        .and_then(|base| crate::tes::forms_actors::npc_sex(world, base))
        .unwrap_or(0);
    let model_loaded = world
        .get::<ActiveEffect>(entity)
        .unwrap()
        .displaced_armor_model_loaded_87;
    for slot in 0..16_u32 {
        if !crate::tes::form::biped_covers(world, armor, slot) {
            continue;
        }
        let has_changes = world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44
            .get(crate::tes::base_extra_list::EXTRA_CONTAINER_CHANGES)
            .is_some();
        if !has_changes {
            continue;
        }
        let changes = ensure_container_changes(world, reference)?;
        let found = crate::tes::equipment::equipped_stack_in_slot(world, &changes, slot);
        put_container_changes(world, reference, changes);
        let Some((at, index)) = found? else {
            continue;
        };
        let changes = ensure_container_changes(world, reference)?;
        let form = changes.entries_00[at].form_08;
        put_container_changes(world, reference, changes);
        if model_loaded == 0
            && world
                .get::<crate::tes::form_components::TesBipedModelForm>(form)
                .is_some()
        {
            let path = crate::tes::form_components::armor_biped_model_path(world, form, sex);
            crate::bs::model_loader::load_model_now_dropped(world, &path, 0x00690E05)?;
            world
                .get_mut::<ActiveEffect>(entity)
                .unwrap()
                .displaced_armor_model_loaded_87 = 1;
        }
        let changes = ensure_container_changes(world, reference)?;
        // IDA: N/A host stop at 00690E1B, a call of 004FBF90 on the stack whose form and event arguments haven't been
        // read: a stack with a script (0x12) stops here; one without answers 0 and changes nothing.
        let script = changes.entries_00[at]
            .lists_00
            .as_ref()
            .map(|lists| &lists[index])
            .is_some_and(|list| list.get(0x12).is_some());
        put_container_changes(world, reference, changes);
        if script {
            return Err(GameError::stop(
                0x00690E1B,
                "a script event on an inventory stack's own script",
            ));
        }
        crate::tes::ai_package::script_event_or(world, reference, 8, Some(form));
        let mut changes = ensure_container_changes(world, reference)?;
        let list = &mut changes.entries_00[at].lists_00.as_mut().expect("selected")[index];
        // 0041F6A0 ExtraDataList_ClearWorn(0): 0x1B off.
        list.remove_by_type(EXTRA_WORN);
        let emptied = list.entries.is_empty();
        let count = crate::tes::base_extra_list::stack_count(list);
        // 004844A0 copies the selection: its list holds the stack unless it emptied (00690E4B
        // PointerList_RemoveFirstMatchingItem on the selection's own list).
        let copy = InventoryEntry {
            lists_00: Some(if emptied {
                Vec::new()
            } else {
                vec![list.clone()]
            }),
            count_04: count,
            form_08: form,
        };
        put_container_changes(world, reference, changes);
        world
            .get_mut::<ActiveEffect>(entity)
            .unwrap()
            .displaced_armor_entries_40[slot as usize] = Some(Box::new(copy));
    }
    let mut stack = ExtraDataList::constructed();
    stack.ensure_bound_armor();
    super::actor_inventory::actor_add_inventory_item(world, reference, armor, Some(stack), 1)?;
    super::actor_inventory::use_inventory_item(world, reference, armor, 1, true, false)?;
    let mut changes = ensure_container_changes(world, reference)?;
    let found = super::inventory::find_worn_stack_for_form(&mut changes, armor, true);
    if let Some((at, index)) = found {
        changes.entries_00[at].lists_00.as_mut().expect("found")[index].set_equip_locked(true);
    }
    put_container_changes(world, reference, changes);
    if found.is_none() {
        world
            .get_mut::<ActiveEffect>(entity)
            .unwrap()
            .pending_equip_lock_88 = 1;
    }
    Ok(())
}

/// IDA: 006918D0 BoundItemEffect_UpdateEffect's equipped arm (+0x84 set): for a weapon on a target with a process, vt+0x3D4
/// (00629490, +0x140) again; a pending lock (+0x88) with a worn weapon entry of the same form locks its first stack and adds
/// 0x50, clearing +0x88; then the player's arm (a dispel when vt+0x304 and vt+0x2FC are both 0). Otherwise a pending lock on
/// an armor takes 00485FA0(armor, 1)'s stack the same way. Completion: a non-player target.
/// # Errors
/// The player's arm.
pub fn update_equipped_bound_item(world: &mut World, entity: Entity) -> Result<(), GameError> {
    let effect = world.get::<ActiveEffect>(entity).expect("an effect");
    let reference = effect.target_20.expect("a bound effect's target");
    let associated = effect.associated_38;
    let pending = effect.pending_equip_lock_88;
    let weapon = associated.filter(|&form| form_type(world, form) == TYPE_WEAPON);
    let process = world
        .get::<MobileObject>(reference)
        .and_then(|mobile| mobile.process_58);
    if let (Some(process), Some(weapon)) = (process, weapon) {
        let Some(mut middle) = world.get_mut::<MiddleHighProcess>(process) else {
            return super::actor_enable::low_process_only(
                world,
                process,
                "00691920: a Low process's bound weapon",
            );
        };
        middle.bound_weapon_140 = Some(weapon);
        if pending != 0 && super::actor_process::actor_worn_weapon(world, reference) == Some(weapon)
        {
            let mut changes = ensure_container_changes(world, reference)?;
            if let Some((at, index)) =
                super::inventory::find_worn_stack_for_form(&mut changes, weapon, false)
            {
                world
                    .get_mut::<ActiveEffect>(entity)
                    .unwrap()
                    .pending_equip_lock_88 = 0;
                let list = &mut changes.entries_00[at].lists_00.as_mut().expect("found")[index];
                list.set_equip_locked(true);
                list.ensure_bound_armor();
            }
            put_container_changes(world, reference, changes);
        }
        if the_player(world) == Some(reference) {
            return Err(GameError::stop(
                0x0069198A,
                "the player's bound weapon state",
            ));
        }
        return Ok(());
    }
    if pending == 0 {
        return Ok(());
    }
    let Some(armor) = associated.filter(|&form| form_type(world, form) == TYPE_ARMOR) else {
        return Ok(());
    };
    let mut changes = ensure_container_changes(world, reference)?;
    if let Some((at, index)) = super::inventory::find_worn_stack_for_form(&mut changes, armor, true)
    {
        world
            .get_mut::<ActiveEffect>(entity)
            .unwrap()
            .pending_equip_lock_88 = 0;
        let list = &mut changes.entries_00[at].lists_00.as_mut().expect("found")[index];
        list.set_equip_locked(true);
        list.ensure_bound_armor();
    }
    put_container_changes(world, reference, changes);
    Ok(())
}

// IDA: 00690A00 BoundItemEffect_ApplyEffect. Read the art and range's highest-cost effect
// independently. The ability arm writes a zero equip delay even when the model load returned null.
// Completion: a bound ability with a target and associated form.
pub(crate) fn apply_bound_ability(world: &mut World, entity: Entity) -> Result<(), GameError> {
    use crate::bs::{
        model_loader::{SynchronousModelLoad, lookup_and_release_model},
        task_manager::TaskGlobals,
    };
    let effect = world.get::<ActiveEffect>(entity).unwrap();
    if effect.associated_38.is_none() || effect.target_20.is_none() {
        return Err(GameError::stop(
            0x00690ADA,
            "dispel a bound effect missing its associated form or target",
        ));
    }
    let spell = effect.item_08;
    let index = effect.effect_0c;
    let range = effect_item(world, spell, index).data_00[4];
    let art = crate::tes::forms_magic::magic_item_art_setting(world, spell)?
        .ok_or_else(|| GameError::stop(0x00690A60, "a null bound effect art setting"))?;
    let highest = crate::tes::forms_magic::highest_cost_effect_in_range(world, spell, range)?;
    if let Some(path) = crate::tes::actors::magic_selection::art_path(world, art) {
        let load = world
            .get_resource::<SynchronousModelLoad>()
            .ok_or_else(|| GameError::stop(0x00690A80, "bound effect art needs model services"))?
            .0;
        load(world, &path, false, 0, true)?;
        world
            .get_mut::<ActiveEffect>(entity)
            .unwrap()
            .equip_delay_80 = 0.0;
        if highest != Some(index) {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let loader = tasks
                .model_loader_b33a1c
                .as_mut()
                .expect("the model loader");
            // releaseEntry=0 never tests save flags.
            lookup_and_release_model(loader, &path, false, None)?;
        }
    }
    Ok(())
}
