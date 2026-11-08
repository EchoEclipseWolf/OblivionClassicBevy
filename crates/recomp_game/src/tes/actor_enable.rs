//! The arms `004DD850` takes for a disabled actor: `005E5EC0`'s inventory reset (`004DDB00`) and auto-equip (`005227A0`, then `005F3140` and `00489C30` per item), the LowProcess and its add, `005F23E0`, and the first scheduled package pass (`0064A380`). A new game's command 30, `Enable` on `90028F5`, is the first to get here.
//!
//! Evidence: `records-run-u-status1-cover.txt` lines 1004 to 1326 in `evidence/gameplay-run001-2026-11-07/`, and the actor in `run-v-moveto`'s snapshot.

use std::path::Path;
use std::rc::Rc;

use bevy_ecs::prelude::*;
use recomp_core::logging::{Level, emit};
use recomp_core::reader::StreamError;

use super::actor::{Actor, Character, MobileObject, TesObjectRefr, actor_process_level};
use super::actor_process::{
    BaseProcess, LOW_PROCESS_VTABLE, LowProcess, MiddleHighProcess, ProcessInputs,
    actor_desired_level, actor_process_manager_add, actor_process_manager_remove,
    construct_low_process, delete_low_process,
};
use super::ai_cell_moves::set_3d_null;
use super::ai_conditions::{ScriptArms, ScriptRandom, the_player};
use super::ai_package::{actor_is_dead, game_hour, package_data, script_event_or};
use super::ai_process::{process_evaluate_package, process_run_scheduled};
use super::base_extra_list::{
    EXTRA_CONTAINER_CHANGES, EXTRA_COUNT, EXTRA_LEVELED_ITEM, EXTRA_WORN, ExtraData, ExtraDataList,
    new_extra_entry,
};
use super::data_load::DataLoadServices;
use super::form::OriginalVtable;
use super::form_arms::DataLoadHost;
use super::form_components::{
    FormRef, TesBipedModelForm, TesContainer, TesEnchantableForm, TesValueForm,
};
use super::forms_init::form_type;
use super::forms_items::set_change_flags;
use super::forms_references::{NoRandom, save_load_active};
use super::inventory::{
    ContainerChanges, InventoryEntry, ensure_container_changes, put_container_changes,
    reference_container, resolve_leveled_items, run_item_scripts,
};
use super::tes::GameThreadLocals;

// IDA: the form types the equip code casts to: TESObjectARMO, BOOK, TESObjectCLOT, TESObjectLIGH, TESObjectWEAP, TESAmmo, TESNPC, TESCreature.
const TYPE_ARMOR: u8 = 0x14;
const TYPE_CLOTHING: u8 = 0x16;
const TYPE_LIGHT: u8 = 0x1A;
const TYPE_WEAPON: u8 = 0x21;
const TYPE_NPC: u8 = 0x23;
const TYPE_CREATURE: u8 = 0x24;

// IDA: 00A350F8, ExtraDataList's vtable (00422EE0).
const EXTRA_DATA_LIST_VTABLE: u32 = 0x00A3_50F8;

// IDA: 00A30634, -1.0.
const MINUS_ONE: f32 = -1.0;

// IDA: N/A the load's seams as a script command's call of 00488420 sees them: 0047DF80 through the host's draw (`ScriptRandom`'s second), and warnings to the host log.
struct ScriptLoadHost(Option<Rc<dyn Fn() -> i32>>);

impl DataLoadHost for ScriptLoadHost {
    /// IDA: N/A `ScriptLoadHost`'s side of `DataLoadHost`.
    fn random_next(&mut self) -> Option<i32> {
        self.0.as_ref().map(|draw| draw())
    }
}

// IDA: the base container's entries, out of its component: (count, form).
fn base_entries(world: &World, reference: Entity) -> Vec<(i32, Entity)> {
    reference_container(world, reference)
        .and_then(|base| world.get::<TesContainer>(base))
        .map_or_else(Vec::new, |container| {
            container
                .entries_08
                .iter()
                .filter_map(|entry| match entry.item {
                    FormRef::Resolved(Some(form)) => Some((entry.count, form)),
                    _ => None,
                })
                .collect()
        })
}

// IDA: 004691B0: the biped part of an ARMO (+0x64) or a CLOT (+0x5C), else null; its +0x04 is the slot mask.
pub(super) fn biped_mask(world: &World, form: Entity) -> Option<u16> {
    matches!(form_type(world, form), TYPE_ARMOR | TYPE_CLOTHING)
        .then(|| world.get::<TesBipedModelForm>(form))
        .flatten()
        .map(|biped| biped.slot_mask_04)
}

/// IDA: 004691B0 then 00468FF0(biped, slot, 0): a form's biped covers the slot. Completion: 100%.
pub(super) fn biped_covers(world: &World, form: Entity, slot: u32) -> bool {
    biped_mask(world, form).is_some_and(|mask| covers(mask, slot))
}

// IDA: 00468FF0(biped, slot, 0): the word at +0x04 has bit `slot`. The decompiler shows a byte; the instruction is `movzx eax, word ptr [eax+4]` at 0046900C.
fn covers(mask: u16, slot: u32) -> bool {
    u32::from(mask) & (1 << slot) != 0
}

/// IDA: 004DD850's arms for an actor between 0046ABA0(0) and the cell test: vt+0x184 005E5EC0(1), then with a null +0x58 006438F0 LowProcess_ctor (FormHeap 0x90) and 00673A90(actor, 3, 0, 0, 0). Completion: 100% of the arms a disabled NPC with no process, or with a LowProcess above level 0, reaches; the rest are stops.
/// # Errors
/// A stop in 005E5EC0, or a null g_saveLoadGame.
pub(super) fn enable_actor_reset(world: &mut World, actor: Entity) -> Result<(), StreamError> {
    reset_actor(world, actor)?;
    if world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58
        .is_none()
    {
        let mut random = NoRandom;
        let inputs = ProcessInputs {
            random: &mut random,
            idle_chatter_comment_timer: 0,
            low_process_unset_88: 0,
            save_load_active: save_load_active(world)?,
        };
        let process = construct_low_process(world, &inputs);
        world
            .get_mut::<MobileObject>(actor)
            .expect("an actor")
            .process_58 = Some(process);
        actor_process_manager_add(world, actor, 3, false, false)?;
    }
    Ok(())
}

/// IDA: 004DD850's arms for an actor outside an attached cell, after the cell test: vt+0x1C4 005F23E0, then with a process vt+0x14(actor, 1) (0064A380, `process_run_scheduled`) and 00601B80, and with a level above 0 vt+0x1C0(0.0) 00605770 Actor_Update. 005F23E0 returns 1 when the process's level is 0065AD00's; a change is a stop. Completion: 100%, with the stops of each.
/// # Errors
/// A level change, or a stop in 0064A380, 00601B80 or 00605770.
pub(super) fn enable_actor_update(world: &mut World, actor: Entity) -> Result<(), StreamError> {
    let Some(process) = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58
    else {
        return Ok(());
    };
    if actor_process_level(world, actor) != actor_desired_level(world, actor)? {
        return Err(StreamError("005F2408: a process level change"));
    }
    process_run_scheduled(world, process, actor, true, &mut ScriptArms)?;
    reevaluate_package(world, actor)?;
    if actor_process_level(world, actor) != 0 {
        super::actor_update::actor_update(world, actor, 0.0)?;
    }
    Ok(())
}

/// IDA: 004E4690's arm for an actor with a process, after 0046ABA0(1): vt+0x190 is 1 on a Character, so the player's +0x1E0 is cleared when it's this actor (a set one is a stop, since the port keeps only whether it's set); 005E4B00 and 006A2160 remove the abilities' and enchantments' effects from the magic target at +0x68, whose list a LowProcess's vt+0x298 (006F7070) answers null, so nothing; 00B3B7D0, which only 006078E0 raises and nothing reached runs, is 0, so no 00607B90. Then 00674550 with the process's level; a combat package (00567770) reaches 005EAE70, a stop; a LowProcess answers vt+0x378 null; the deleting destructor (0064ACD0) and +0x58 = null. Completion: 100% for a LowProcess outside combat.
/// # Errors
/// A stop named above, or the destructor's.
pub(super) fn disable_actor_process(world: &mut World, actor: Entity) -> Result<(), StreamError> {
    let Some(process) = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58
    else {
        return Ok(());
    };
    if let Some(player) = the_player(world)
        && world
            .get::<super::player::PlayerCharacter>(player)
            .is_some_and(|data| data.last_ridden_horse_1e0 != 0)
    {
        return Err(StreamError("004E4701: the player's +0x1E0"));
    }
    low_process_only(world, process, "005E4B11: a process with magic effects")?;
    let level = actor_process_level(world, actor);
    let active = save_load_active(world)?;
    actor_process_manager_remove(world, actor, level, active);
    let Some(process) = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58
    else {
        return Ok(());
    };
    if current_package(world, process).is_some_and(|package| {
        super::ai_package::package_is_combat_like(package_data(world, package))
    }) {
        return Err(StreamError("004E476F: 005EAE70 for a combat package"));
    }
    delete_low_process(world, process)?;
    world
        .get_mut::<MobileObject>(actor)
        .expect("an actor")
        .process_58 = None;
    Ok(())
}

// IDA: N/A the current package, +0x08.
fn current_package(world: &World, process: Entity) -> Option<Entity> {
    world
        .get::<BaseProcess>(process)
        .expect("a process")
        .editor_package_08
}

// IDA: 00601B80(actor) on a LowProcess (vtable A720A4, whose slots name each call). vt+0x184 (00452A60, +0x08) of type 0x12 returns, as does Actor_IsDead(1). A current package of type 0x17 or 0x1E returns; 0x18 asks the actor's vt+0x388; any other takes vt+0xB8 and the door callbacks, a stop. vt+0x248 (0051AAD0, +0x20) set is a stop. 005EAE70 with no package: vt+0x4C8(4) (004943A0, 0), vt+0x4A8 and vt+0x480 do nothing, so 006116D0 only stores 1.0 at the actor's +0x100 unless a save is active; vt+0x174 is 0, vt+0x178 and vt+0x194 do nothing, vt+0x36C is 0, never 9. Then vt+0x3C0 (006479C0), vt+0x394(0), vt+0x18(actor, 1) (00649340), and 004FBF90(null, +0x44, 0x800) when a package came. With no package before, no re-equip. vt+0x194 does nothing; vt+0x18C's 005E0EC0 asks vt+0x36C, 0. A nonzero level stores the game hour less 1.0 (vt+0x1C 00629240, a float store). A target's +0x08 goes to +0x38 (vt+0x100 006292C0). Completion: 100% for a LowProcess with no package before.
pub(super) fn reevaluate_package(world: &mut World, actor: Entity) -> Result<(), StreamError> {
    let process = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58
        .ok_or(StreamError("00601B86: no process"))?;
    if world.get::<OriginalVtable>(process).map(|vtable| vtable.0) != Some(LOW_PROCESS_VTABLE) {
        return Err(StreamError("00601B91: a process other than a LowProcess"));
    }
    if current_package(world, process)
        .is_some_and(|package| package_data(world, package).type_20 == 0x12)
    {
        return Ok(());
    }
    if actor_is_dead(world, actor, true) {
        return Ok(());
    }
    if let Some(package) = current_package(world, process) {
        return Err(match package_data(world, package).type_20 {
            0x17 | 0x1E => return Ok(()),
            0x18 => StreamError("00601BF8: vt+0x388 for a type 0x18 package"),
            _ => StreamError("00601C19: the current package's vt+0xB8"),
        });
    }
    if world
        .get::<LowProcess>(process)
        .expect("a LowProcess")
        .byte_20
        != 0
    {
        return Err(StreamError("00601CDB: vt+0xCC's follower list"));
    }
    // 005EAE70, 006116D0's arm with no package.
    if !save_load_active(world)? {
        world
            .get_mut::<Actor>(actor)
            .expect("an actor")
            .head_track_target_scan_timer_100 = 1.0;
    }
    clear_package_state(world, process)?;
    world
        .get_mut::<LowProcess>(process)
        .expect("a LowProcess")
        .byte_1c = 0;
    process_evaluate_package(world, process, actor, true, &mut ScriptArms)?;
    if current_package(world, process).is_some() {
        script_event_or(world, actor, 0x800, None);
    }
    if actor_process_level(world, actor) != 0 {
        let hour = game_hour(world) - 1.0;
        world
            .get_mut::<LowProcess>(process)
            .expect("a LowProcess")
            .cur_hour_14 = hour;
    }
    if let Some(package) = current_package(world, process)
        && let Some(target) = package_data(world, package).target_28
    {
        world
            .get_mut::<LowProcess>(process)
            .expect("a LowProcess")
            .field_38 = target.count_08;
    }
    Ok(())
}

// IDA: 006479C0 LowProcess vt+0x3C0(actor): vt+0xBC(0) (007790C0, +0x1D) and vt+0x394(0) (+0x1C), +0x2C and +0x30 null, the +0x44 and +0x48 blocks out of the +0x3C list and freed, the rest of the list freed, the +0x50 chain freed and +0x4C = 0. A block or a list holding anything is a stop. Completion: 100% of the empty lists.
fn clear_package_state(world: &mut World, process: Entity) -> Result<(), StreamError> {
    let mut low = world.get_mut::<LowProcess>(process).expect("a LowProcess");
    low.byte_1d = 0;
    low.byte_1c = 0;
    low.field_2c = None;
    low.field_30 = None;
    if low.block_44 != 0 || low.block_48 != 0 || low.list_3c != [0; 2] || low.list_4c[1] != 0 {
        return Err(StreamError("006479EA: a LowProcess's package lists"));
    }
    low.list_4c[0] = 0;
    Ok(())
}

// IDA: 005E5EC0 Character vt+0x184(1). A process of level 0 takes vt+0x31C(1) unless a save is active, a stop; otherwise Character_Set3D(null) (0060E430): with no 3D, 004E0F80's 00439DC0 (`set_3d_null`). Then 004DDB00(1). The base: TESNPC 005227A0(base, actor, a, b, 0, 1), TESCreature 0051E240(actor, a, b, 1), where `a` and `b` are 1 unless the process's current package (vt+0x184, 00452A60's +0x08) has flag 0x100000 or 0x200000, a stop. Completion: 100% for no process or a LowProcess above level 0.
fn reset_actor(world: &mut World, actor: Entity) -> Result<(), StreamError> {
    let process = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58;
    if let Some(process) = process {
        low_process_only(
            world,
            process,
            "005E5ED4: a process other than a LowProcess",
        )?;
        if actor_process_level(world, actor) == 0 {
            return Err(StreamError("005E5EF2: a level 0 process's vt+0x31C"));
        }
    }
    if !save_load_active(world)? {
        set_3d_null(world, actor)?;
    }
    reset_inventory(world, actor)?;
    let base = world
        .get::<TesObjectRefr>(actor)
        .expect("a reference")
        .base_form_1c
        .ok_or(StreamError("005E5F22: an actor with no base"))?;
    if let Some(process) = process
        && current_package(world, process)
            .is_some_and(|package| package_data(world, package).flags_1c & 0x30_0000 != 0)
    {
        return Err(StreamError("005E5F65: a package that keeps the outfit"));
    }
    match form_type(world, base) {
        TYPE_NPC => auto_equip(world, actor, true, true, true),
        TYPE_CREATURE => creature_auto_equip(world, actor, true, true, true),
        _ => Ok(()),
    }
}

// IDA: N/A the process vtable test the arms below share: a LowProcess (A720A4) answers each slot they call with a constant or does nothing; another class is a stop.
pub(crate) fn low_process_only(
    world: &World,
    process: Entity,
    stop: &'static str,
) -> Result<(), StreamError> {
    if world.get::<OriginalVtable>(process).map(|vtable| vtable.0) == Some(LOW_PROCESS_VTABLE) {
        Ok(())
    } else {
        Err(StreamError(stop))
    }
}

// IDA: 00486030 ExtraContainerChanges_UnequipWornStacks(keep_locked): in each entry every stack worn (0041DF10(0): 0x1B or
// 0x1C) that isn't equip-locked (0041DF40, when `keep_locked`) loses 0x1B (0041F6A0(0)) and its count extra (004238A0(0)); a
// stack left with no extras leaves the list and is destroyed. An entry left with a list of no stacks and a count of 0 leaves
// the changes (004845A0, the list freed). Completion: 100%.
fn unequip_worn_stacks(changes: &mut ContainerChanges, keep_locked: bool) {
    for entry in &mut changes.entries_00 {
        if let Some(lists) = entry.lists_00.as_mut() {
            lists.retain_mut(|list| {
                if !super::inventory_entries::worn(list, false)
                    || (keep_locked && list.present(EXTRA_LOCKED_EQUIP))
                {
                    return true;
                }
                list.remove_if_found(EXTRA_WORN);
                list.remove_if_found(EXTRA_COUNT);
                !list.entries.is_empty()
            });
        }
    }
    changes.entries_00.retain(|entry| {
        !(entry.count_04 == 0 && entry.lists_00.as_ref().is_some_and(Vec::is_empty))
    });
}

// IDA: 0048DF80 on the changes: the owner's vt+0x40(0x8000000), then in each entry every stack holding a 0x36 extra (0041E850) loses it (00424790) and its count (004238A0(0)), the entry's count drops by 1 whatever the stack held, and the stack leaves the list (PointerList_RemoveFirstMatchingItem, 0048DFDD) and is destroyed. An entry left with a list but no stacks and a count of 0 goes, and the walk restarts at the head. Completion: 100%.
fn drop_leveled_stacks(changes: &mut ContainerChanges) {
    let mut at = 0;
    while at < changes.entries_00.len() {
        let entry = &mut changes.entries_00[at];
        if let Some(lists) = entry.lists_00.as_mut() {
            let before = lists.len();
            lists.retain(|list| list.get(EXTRA_LEVELED_ITEM).is_none());
            entry.count_04 = entry.count_04.wrapping_sub((before - lists.len()) as i32);
        }
        if entry.lists_00.as_ref().is_some_and(Vec::is_empty) && entry.count_04 == 0 {
            changes.entries_00.remove(at);
            at = 0;
        } else {
            at += 1;
        }
    }
}

// IDA: 004DDB00(1). With a base container the reset always runs, since its argument is 1: 00485E00, then 00487B60's answer (it reads each item's vt+0x78) decides nothing. 0048DF80 (`drop_leveled_stacks`). vt+0x190 is 1 on a Character, so 005E9690 asks a process for its four equipped items: on a LowProcess, vt+0x104 (00725DA0) and vt+0x10C, 0x108 and 0x110 (006FE080) answer 0. 00488420, whose rolls draw through the host's CRT, and 0048E060; an empty list goes (0041F5F0). Then 00420480's 0x42 list. Completion: 100% for no process or a LowProcess.
fn reset_inventory(world: &mut World, actor: Entity) -> Result<(), StreamError> {
    if reference_container(world, actor).is_none() {
        return Ok(());
    }
    let mut changes = ensure_container_changes(world, actor)?;
    let flagged = set_change_flags(world, actor, 0x0800_0000);
    drop_leveled_stacks(&mut changes);
    put_container_changes(world, actor, changes);
    flagged?;
    if let Some(process) = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58
    {
        low_process_only(world, process, "005E96A6: a process's equipped items")?;
    }
    let mut host = ScriptLoadHost(
        world
            .get_non_send::<ScriptRandom>()
            .map(|random| random.1.clone()),
    );
    let mut warning = |text: String, _: bool| emit(Level::Warning, &text);
    let mut services = DataLoadServices {
        working_directory: Path::new(""),
        warning: &mut warning,
        host: &mut host,
    };
    let mut changes = ensure_container_changes(world, actor)?;
    let result = resolve_leveled_items(world, actor, &mut changes, &mut services)
        .and_then(|()| run_item_scripts(world, actor, &mut changes));
    let empty = changes.entries_00.is_empty();
    put_container_changes(world, actor, changes);
    result?;
    let list = &mut world
        .get_mut::<TesObjectRefr>(actor)
        .expect("a reference")
        .into_inner()
        .extra_data_44;
    if empty {
        list.remove_by_type(EXTRA_CONTAINER_CHANGES);
    }
    if list.get(0x42).is_some() {
        return Err(StreamError("004DDBB5: the 0x42 list's references"));
    }
    Ok(())
}

// IDA: 0041E7B0 BaseExtraList_GetOwner: the 0x27 extra's form (as 00426270 resolved it), else null.
fn list_owner(list: &ExtraDataList) -> Result<Option<Entity>, StreamError> {
    let Some(entry) = list.get(super::base_extra_list::EXTRA_OWNER) else {
        return Ok(None);
    };
    entry.resolved_0c.ok_or(StreamError(
        "0041E7B0: an ownership extra whose form isn't resolved",
    ))
}

// IDA: the health 0048C870 rates a stack by: its 0x2B extra's float (0041E810), else 0046CA20 TESHealthForm vt+0x10 (+0x04), loaded with `fild` and 2^32 added to a negative (an unsigned), stored a float.
fn stack_health(world: &World, form: Entity, list: Option<&ExtraDataList>) -> f32 {
    if let Some(entry) = list.and_then(|list| list.get(super::base_extra_list::EXTRA_HEALTH))
        && let ExtraData::Float(bits) = entry.data
    {
        return f32::from_bits(bits);
    }
    let health = world
        .get::<super::form_components::TesHealthForm>(form)
        .map_or(0, |health| health.health_04);
    f64::from(health) as f32
}

// IDA: 00547370 CalculateArmorRatingFromFactors(base, skill, luck, condition): `ftol` of the luck and the skill into 00547B90; the
// rating fArmorRatingBase (00B36E98) + adjusted / 100.0 * (fArmorRatingMax (00B36EA0) - base, stored a float) times the base
// rating, stored a float; its floor (`ftol`, less 1 when that's above the value), and 1.0 when that floor is 1.0 or less; times
// (fArmorRatingConditionMult (00B36EB0) * condition + fArmorRatingConditionBase (00B36EA8), stored a float), stored a float.
fn armor_rating(world: &World, base_rating: u16, skill: f32, luck: f32, condition: f32) -> f32 {
    let adjusted = super::magic_menu::luck_adjusted_skill(world, skill as i32, luck as i32);
    let rating_base = setting_float(world, 0x00B3_6E98);
    let range = setting_float(world, 0x00B3_6EA0) - rating_base;
    let rating = ((f64::from(rating_base) + f64::from(adjusted) / 100.0 * f64::from(range))
        * f64::from(base_rating)) as f32;
    let floor = |value: f32| {
        let truncated = value as i32 as f32;
        if value - truncated < 0.0 {
            truncated - 1.0
        } else {
            truncated
        }
    };
    let floored = floor(rating);
    let rating = if floored <= 1.0 { 1.0 } else { floored };
    let factor = (f64::from(setting_float(world, 0x00B3_6EB0)) * f64::from(condition)
        + f64::from(setting_float(world, 0x00B3_6EA8))) as f32;
    factor * rating
}

// IDA: a float setting's bits at its address.
fn setting_float(world: &World, address: u32) -> f32 {
    f32::from_bits(
        world
            .resource::<super::game_settings::GameSettingCollection>()
            .raw_at(address)
            .unwrap_or(0),
    )
}

// IDA: 0048C870's rating of one stack (or of a form with no stack): the stack's health, then with a health above 0.0 (when
// `test_health`) the armor's +0xE4 / 100.0 truncated to a word, the base's vt+0x12C (0051E790: vt+0x128 TESNPC_GetBaseActorValue
// as a double) of luck (7) and of the armor's skill (004B4C80: Heavy Armor 18 when +0x6A bit 7 is set, else Light Armor 27).
fn rate_armor(
    world: &World,
    base: Entity,
    armor: Entity,
    list: Option<&ExtraDataList>,
    test_health: bool,
) -> Result<Option<f32>, StreamError> {
    let condition = stack_health(world, armor, list);
    // 0048CAC5 `test ah, 41h`: at or below 0.0, or a NaN, skips the stack.
    if test_health && condition.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return Ok(None);
    }
    let rating = world
        .get::<super::forms_items::TesArmor>(armor)
        .map_or(0, |armor| armor.armor_rating_e4);
    let base_rating = ((f64::from(rating) / 100.0) as f32) as i32 as u16;
    let luck = super::ai_process::base_actor_value(world, base, 7)? as f32;
    let heavy = world
        .get::<TesBipedModelForm>(armor)
        .is_some_and(|biped| biped.flags_06 & 0x80 != 0);
    let skill =
        super::ai_process::base_actor_value(world, base, if heavy { 18 } else { 27 })? as f32;
    Ok(Some(armor_rating(
        world,
        base_rating,
        skill,
        luck,
        condition,
    )))
}

/// IDA: 0048C870 ContainerChanges_FindBestArmorForSlot(changes, base, slot, 1): a worn stack in the slot (00486790) is a stop
/// (its 0x47 test, 0041DF40). Then the base container's entries (TESObjectARMO casts, with their changes entry): with an entry,
/// its stacks owned by someone but `base` count against the total; a form whose total count is above 0 (or whose base count is
/// negative) and covers the slot is rated per stack owned by nobody or `base` (with a health above 0.0), or with no stacks as
/// a whole (health untested); then the changes entries that are armor owned by nobody or `base`, with a nonzero count, not
/// listed by a base container flagged 1 (00469920), covering the slot and with a count of 0 or more, likewise. The first rating
/// strictly above the best (from -FLT_MAX) wins; the answer is its form and stack. Completion: 100% but the worn stack.
/// # Errors
/// As above, or an ownership extra not resolved.
pub(crate) fn best_armor(
    world: &World,
    changes: &ContainerChanges,
    base: Entity,
    actor: Entity,
    slot: u32,
    check_worn: bool,
) -> Result<Option<(Entity, Option<usize>)>, StreamError> {
    if check_worn && super::cell_reference_loop::equipped_in_slot(world, actor, slot)?.is_some() {
        return Err(StreamError("0048C8AA: a worn stack's 0x47 test (0041DF40)"));
    }
    let mut best = -f32::MAX;
    let mut chosen: Option<(Entity, Option<usize>)> = None;
    let mut consider = |rating: Option<f32>, choice: (Entity, Option<usize>)| {
        if let Some(rating) = rating
            && best < rating
        {
            best = rating;
            chosen = Some(choice);
        }
    };
    let entry_of = |form: Entity| {
        changes
            .entries_00
            .iter()
            .find(|entry| entry.form_08 == form)
    };
    for (count, form) in base_entries(world, actor) {
        let armor = (form_type(world, form) == TYPE_ARMOR).then_some(form);
        let entry = armor.and_then(entry_of);
        let total_ok = match entry {
            None => true,
            Some(entry) => {
                let lists = entry.lists_00.as_deref().unwrap_or_default();
                let first_owner = match lists.first() {
                    Some(list) => list_owner(list)?,
                    None => None,
                };
                let mut owned = 0;
                if first_owner != Some(base) && entry.count_04 > 0 {
                    for list in lists {
                        if list_owner(list)?.is_some() {
                            owned += 1;
                        }
                    }
                }
                (first_owner.is_none() || owned < count + entry.count_04)
                    && (count + entry.count_04 > 0 || count < 0)
            }
        };
        let Some(armor) = armor.filter(|_| total_ok) else {
            continue;
        };
        if !biped_covers(world, armor, slot) {
            continue;
        }
        let lists = entry
            .and_then(|entry| entry.lists_00.as_deref())
            .unwrap_or_default();
        if lists.is_empty() {
            consider(rate_armor(world, base, armor, None, false)?, (armor, None));
            continue;
        }
        for (index, list) in lists.iter().enumerate() {
            let owner = list_owner(list)?;
            if owner.is_none() || owner == Some(base) {
                consider(
                    rate_armor(world, base, armor, Some(list), true)?,
                    (armor, Some(index)),
                );
            }
        }
    }
    let flagged = reference_container(world, actor)
        .and_then(|container| world.get::<TesContainer>(container))
        .filter(|container| container.flags_04 & 1 != 0);
    for entry in &changes.entries_00 {
        let armor = entry.form_08;
        if form_type(world, armor) != TYPE_ARMOR {
            continue;
        }
        let lists = entry.lists_00.as_deref().unwrap_or_default();
        let owner = match lists.first() {
            Some(list) => list_owner(list)?,
            None => None,
        };
        if owner.is_some() && owner != Some(base) {
            continue;
        }
        let listed = flagged.is_some_and(|container| {
            container
                .entries_08
                .iter()
                .any(|item| item.item == FormRef::Resolved(Some(armor)))
        });
        if entry.count_04 == 0 || listed || !biped_covers(world, armor, slot) || entry.count_04 < 0
        {
            continue;
        }
        if lists.is_empty() {
            consider(rate_armor(world, base, armor, None, false)?, (armor, None));
            continue;
        }
        for (index, list) in lists.iter().enumerate() {
            let owner = list_owner(list)?;
            if owner.is_none() || owner == Some(base) {
                consider(
                    rate_armor(world, base, armor, Some(list), true)?,
                    (armor, Some(index)),
                );
            }
        }
    }
    Ok(chosen)
}

// IDA: 00547070 the weapon damage rating(skill, luck, strength, fatigue, damage, health, multiplier, no_fatigue): the
// luck-adjusted skill (00547B90); damage * 00B36DD8, stored a float; 00B36DF8 * health + 00B36DF0, stored; adjusted * 0.01 *
// 00B36DE8 + 00B36DE0, stored; min(strength, 100) * 0.01 * 00B36E08 + 00B36E00, stored; the fatigue factor (1.0, or
// 00B37530 - (1 - fatigue) * 00B37538); their product stored, then times the multiplier, stored.
#[expect(clippy::too_many_arguments, reason = "00547070's eight arguments")]
fn weapon_damage_rating(
    world: &World,
    skill: i32,
    luck: i32,
    strength: i32,
    fatigue: f32,
    damage: i32,
    health: f32,
    multiplier: f32,
    no_fatigue: bool,
) -> f32 {
    let adjusted = super::magic_menu::luck_adjusted_skill(world, skill, luck);
    let set = |address| f64::from(setting_float(world, address));
    let damage = (f64::from(damage) * set(0x00B3_6DD8)) as f32;
    let health = (set(0x00B3_6DF8) * f64::from(health) + set(0x00B3_6DF0)) as f32;
    let hundredth = f64::from(0.01_f32);
    let skill = (f64::from(adjusted) * hundredth * set(0x00B3_6DE8) + set(0x00B3_6DE0)) as f32;
    let strength =
        (hundredth * f64::from(strength.min(100)) * set(0x00B3_6E08) + set(0x00B3_6E00)) as f32;
    let fatigue = if no_fatigue {
        1.0
    } else {
        (set(0x00B3_7530) - (1.0 - f64::from(fatigue)) * set(0x00B3_7538)) as f32
    };
    let product = (f64::from(health)
        * f64::from(damage)
        * f64::from(skill)
        * f64::from(strength)
        * f64::from(fatigue)) as f32;
    (f64::from(product) * f64::from(multiplier)) as f32
}

// IDA: 00547140(weapon, enchantment + 0x18, health, skill, luck, strength, fatigue, damage): 0 for a weapon with no health
// (0046CA20); the damage (+0x88 vt+0x10) unless `damage` isn't -1.0 (`ftol`); 00547070(.., 1.0, 0); 00B36D00 times it, plus an
// enchantment's effects (a count above 0) through its vt[0](0) times 00B36D20, a stop here; each step stored a float.
#[expect(clippy::too_many_arguments, reason = "00547140's eight arguments")]
fn weapon_rating(
    world: &World,
    weapon: Entity,
    enchantment: Option<Entity>,
    health: f32,
    skill: i32,
    luck: i32,
    strength: i32,
    fatigue: f32,
    damage: f32,
) -> Result<f32, StreamError> {
    if world
        .get::<super::form_components::TesHealthForm>(weapon)
        .is_none_or(|health| health.health_04 == 0)
    {
        return Ok(0.0);
    }
    let mut points = i32::from(attack_damage(world, weapon));
    if damage != -1.0 {
        points = damage as i32;
    }
    let base = weapon_damage_rating(
        world, skill, luck, strength, fatigue, points, health, 1.0, false,
    );
    if enchantment.is_some() {
        return Err(StreamError(
            "005471E6: an enchanted weapon's rating (its effect cost)",
        ));
    }
    let scaled = (f64::from(setting_float(world, 0x00B3_6D00)) * f64::from(base)) as f32;
    Ok(scaled + 0.0)
}

// IDA: +0x88 TESAttackDamageForm vt+0x10, the word at +0x04.
fn attack_damage(world: &World, weapon: Entity) -> u16 {
    world
        .get::<super::form_components::TesAttackDamageForm>(weapon)
        .map_or(0, |form| form.damage_04)
}

// IDA: TESObjectWEAP +0x90, the weapon type (0 blade one hand .. 4 staff, 5 bow).
pub(crate) fn weapon_type_of(world: &World, weapon: Entity) -> u8 {
    weapon_type(world, weapon)
}

// IDA: TESObjectWEAP +0x90.
fn weapon_type(world: &World, weapon: Entity) -> u8 {
    world
        .get::<super::forms_items::TesWeapon>(weapon)
        .map_or(0, |weapon| weapon.data_90[0])
}

// IDA: TESEnchantableForm +0x04, the enchantment.
fn enchantment_of(world: &World, form: Entity) -> Option<Entity> {
    match world
        .get::<TesEnchantableForm>(form)
        .map(|form| form.enchantment_04)
    {
        Some(FormRef::Resolved(found)) => found,
        _ => None,
    }
}

// IDA: 0051A120 Actor_GetEquipmentRating's TESAmmo arm: the ammo's damage (+0x74 TESAttackDamageForm vt+0x10), through
// 005479A0 UInt16ToDouble.
fn ammo_rating(world: &World, ammo: Entity) -> f32 {
    f64::from(attack_damage(world, ammo)) as f32
}

/// IDA: 0048B9C0 ContainerChanges_FindBestAmmo(changes, base, check): with `check` a worn quiver stack (00486790(10)) is a stop
/// (its 0x47 test). The base container's TESAmmo, ownership counted as 0048C870's (the base the owner), rated by 0051A120 (the
/// damage); then the changes entries' ammo with no negative leveled stack, owned by nobody or the base, with a count, not
/// listed by a flagged base container. The best of each (strictly above, from -FLT_MAX across both) and, when they differ,
/// the changes' one if its rating is above the base's. Answers that form (the selection's stack and count aren't kept here).
/// Completion: 100% but the worn stack.
/// # Errors
/// As above, or an ownership extra not resolved.
pub(crate) fn best_ammo(
    world: &World,
    changes: &ContainerChanges,
    actor: Entity,
    check_worn: bool,
) -> Result<Option<Entity>, StreamError> {
    if check_worn && super::cell_reference_loop::equipped_in_slot(world, actor, 10)?.is_some() {
        return Err(StreamError("0048B9F8: a worn quiver stack's 0x47 test"));
    }
    let base = reference_container(world, actor);
    let mut best = -f32::MAX;
    let mut from_base: Option<Entity> = None;
    let entry_of = |form: Entity| {
        changes
            .entries_00
            .iter()
            .find(|entry| entry.form_08 == form)
    };
    for (count, form) in base_entries(world, actor) {
        if form_type(world, form) != 0x22 {
            continue;
        }
        if let Some(entry) = entry_of(form) {
            let lists = entry.lists_00.as_deref().unwrap_or_default();
            let first_owner = match lists.first() {
                Some(list) => list_owner(list)?,
                None => None,
            };
            let mut owned = 0;
            if first_owner != base && entry.count_04 > 0 {
                for list in lists {
                    if list_owner(list)?.is_some() {
                        owned += 1;
                    }
                }
            }
            let total_ok = (first_owner.is_none() || owned < count + entry.count_04)
                && (count + entry.count_04 > 0 || count < 0);
            if !total_ok {
                continue;
            }
        }
        let rating = ammo_rating(world, form);
        if best < rating {
            best = rating;
            from_base = Some(form);
        }
    }
    let flagged = base
        .and_then(|container| world.get::<TesContainer>(container))
        .filter(|container| container.flags_04 & 1 != 0);
    let mut from_changes: Option<Entity> = None;
    for entry in &changes.entries_00 {
        let ammo = entry.form_08;
        if form_type(world, ammo) != 0x22 {
            continue;
        }
        let lists = entry.lists_00.as_deref().unwrap_or_default();
        if lists
            .iter()
            .any(|list| list.get(EXTRA_LEVELED_ITEM).is_some())
            && entry.count_04 < 0
        {
            continue;
        }
        let owner = match lists.first() {
            Some(list) => list_owner(list)?,
            None => None,
        };
        if owner.is_some() && owner != base {
            continue;
        }
        let listed = flagged.is_some_and(|container| {
            container
                .entries_08
                .iter()
                .any(|item| item.item == FormRef::Resolved(Some(ammo)))
        });
        if entry.count_04 == 0 || listed {
            continue;
        }
        let rating = ammo_rating(world, ammo);
        if best < rating {
            best = rating;
            from_changes = Some(ammo);
        }
    }
    let mut chosen = from_base;
    if let Some(changed) = from_changes
        && Some(changed) != from_base
    {
        let base_rating = from_base.map_or(0.0, |form| ammo_rating(world, form));
        if base_rating < ammo_rating(world, changed) {
            chosen = Some(changed);
        }
    }
    Ok(chosen)
}

/// IDA: 0048BDA0 ContainerChanges_FindBestWeapon(changes, base, &rating, -1, 1) (`__userpurge`, read from the disassembly): a
/// worn weapon stack (00486790(9)) is a stop (its 0x47 test). The base's combat style (vt+0x120, +0x1E4, else 00B35788's
/// default) +0x68 bit 0x40 (prefers ranged) adds 3000.0 to a bow's or a staff's damage; luck (vt+0x12C(7)) and strength
/// (vt+0x128(0), an int as a float); the fatigue is 1.0 (the base container cast to Actor is null). The base container's
/// weapons (a staff only with its enchantment's effects): with their changes entry the ownership counts (as 0048C870's), the
/// best ammo (`0048B9C0`, a bow needs one), the damage, the skill (vt+0x12C of 00B086A0's AV by type); per stack owned or not
/// with a health above 0.0 the health fraction (stack / form, stored a float) and 00547140; with no stacks 00547140 at the last
/// fraction (1.0 to start). Then the changes entries' weapons owned by nobody or the base, with a count, not listed by a flagged
/// base container, not a negative leveled stack, a bow with ammo: per stack 00547140 with damage -1.0, with no stacks 00547070
/// directly (no 00B36D00 factor). The first rating strictly above the best wins. The rating out is 00546C60: 00B36D00 times it.
/// Completion: 100% with no enchanted weapon and no ammo (both stops).
/// # Errors
/// As above, or an ownership extra not resolved.
pub(crate) fn best_weapon(
    world: &World,
    changes: &ContainerChanges,
    base: Entity,
    actor: Entity,
    check_worn: bool,
) -> Result<Option<(Entity, Option<usize>)>, StreamError> {
    if check_worn && super::cell_reference_loop::equipped_in_slot(world, actor, 9)?.is_some() {
        return Err(StreamError(
            "0048BE08: a worn weapon stack's 0x47 test (0041DF40)",
        ));
    }
    let prefers_ranged = || -> Result<bool, StreamError> {
        let style = match world
            .get::<super::forms_actors::TesNpc>(base)
            .map(|npc| npc.combat_style_1e4)
        {
            Some(FormRef::Resolved(Some(style))) => style,
            _ => {
                return Err(StreamError(
                    "004A98C0: the default combat style's flags (00B35788)",
                ));
            }
        };
        Ok(world
            .get::<super::forms_ai::TesCombatStyle>(style)
            .is_some_and(|style| style.data_18[0x50] & 0x40 != 0))
    };
    let luck = super::ai_process::base_actor_value(world, base, 7)? as f32;
    let strength = super::ai_process::base_actor_value(world, base, 0)? as f32;
    let fatigue = 1.0_f32;
    const SKILLS: [u32; 6] = [14, 14, 16, 16, 16, 28];
    let skill_of = |weapon: Entity| -> Result<f32, StreamError> {
        let kind = usize::from(weapon_type(world, weapon));
        let code = *SKILLS
            .get(kind)
            .ok_or(StreamError("004BB060: a weapon type past 00B086A0"))?;
        Ok(super::ai_process::base_actor_value(world, base, code)? as f32)
    };
    let mut best = -f32::MAX;
    let mut chosen: Option<(Entity, Option<usize>)> = None;
    let mut fraction = 1.0_f32;
    let entry_of = |form: Entity| {
        changes
            .entries_00
            .iter()
            .find(|entry| entry.form_08 == form)
    };
    for (count, form) in base_entries(world, actor) {
        if form_type(world, form) != TYPE_WEAPON {
            continue;
        }
        let weapon = form;
        let kind = weapon_type(world, weapon);
        if kind == 4 {
            return Err(StreamError(
                "0048BF34: a staff's enchantment effects (004149F0)",
            ));
        }
        let entry = entry_of(weapon);
        let ammo = best_ammo(world, changes, actor, check_worn)?;
        if let Some(entry) = entry {
            let lists = entry.lists_00.as_deref().unwrap_or_default();
            let first_owner = match lists.first() {
                Some(list) => list_owner(list)?,
                None => None,
            };
            let mut owned = 0;
            if first_owner != Some(base) && entry.count_04 > 0 {
                for list in lists {
                    if list_owner(list)?.is_some() {
                        owned += 1;
                    }
                }
            }
            if first_owner.is_some() && owned >= entry.count_04 + count {
                continue;
            }
            if entry.count_04 + count <= 0 && count >= 0 {
                continue;
            }
        }
        if kind == 5 && ammo.is_none() {
            continue;
        }
        let mut damage = i32::from(attack_damage(world, weapon));
        // 0048C0AC: a bow adds its ammo's damage (+0x74 vt+0x10, a word).
        if let Some(ammo) = ammo.filter(|_| kind == 5) {
            damage += i32::from(attack_damage(world, ammo));
        }
        if (kind == 5 || kind == 4) && prefers_ranged()? {
            damage = (f64::from(damage) + 3000.0) as i32;
        }
        let enchantment = enchantment_of(world, weapon);
        let skill = skill_of(weapon)?;
        let rate = |fraction: f32| {
            weapon_rating(
                world,
                weapon,
                enchantment,
                fraction,
                skill as i32,
                luck as i32,
                strength as i32,
                fatigue,
                damage as f32,
            )
        };
        let lists = entry
            .and_then(|entry| entry.lists_00.as_deref())
            .unwrap_or_default();
        if lists.is_empty() {
            let rating = rate(fraction)?;
            if best < rating {
                best = rating;
                chosen = Some((weapon, None));
            }
            continue;
        }
        if enchantment.is_some() {
            return Err(StreamError(
                "0048C146: an enchanted weapon's charge test (004849C0)",
            ));
        }
        for (index, list) in lists.iter().enumerate() {
            let health = stack_health(world, weapon, Some(list));
            if health.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
                continue;
            }
            fraction = (f64::from(health) / f64::from(stack_health(world, weapon, None))) as f32;
            let rating = rate(fraction)?;
            if best < rating {
                best = rating;
                chosen = Some((weapon, Some(index)));
            }
        }
    }
    let flagged = reference_container(world, actor)
        .and_then(|container| world.get::<TesContainer>(container))
        .filter(|container| container.flags_04 & 1 != 0);
    for entry in &changes.entries_00 {
        let weapon = entry.form_08;
        if form_type(world, weapon) != TYPE_WEAPON {
            continue;
        }
        let lists = entry.lists_00.as_deref().unwrap_or_default();
        let owner = match lists.first() {
            Some(list) => list_owner(list)?,
            None => None,
        };
        if owner.is_some() && owner != Some(base) {
            continue;
        }
        if entry.count_04 == 0 {
            continue;
        }
        let listed = flagged.is_some_and(|container| {
            container
                .entries_08
                .iter()
                .any(|item| item.item == FormRef::Resolved(Some(weapon)))
        });
        if listed {
            continue;
        }
        let leveled = lists
            .iter()
            .any(|list| list.get(EXTRA_LEVELED_ITEM).is_some());
        if leveled && entry.count_04 < 0 {
            continue;
        }
        let kind = weapon_type(world, weapon);
        if kind == 5 && best_ammo(world, changes, actor, check_worn)?.is_none() {
            continue;
        }
        let skill = skill_of(weapon)?;
        let mut damage = i32::from(attack_damage(world, weapon));
        if (kind == 5 || kind == 4) && prefers_ranged()? {
            damage = (f64::from(damage) + 3000.0) as i32;
        }
        if lists.is_empty() {
            let rating = weapon_damage_rating(
                world,
                skill as i32,
                luck as i32,
                strength as i32,
                fatigue,
                damage,
                fraction,
                1.0,
                false,
            );
            if best < rating {
                best = rating;
                chosen = Some((weapon, None));
            }
            continue;
        }
        let enchantment = enchantment_of(world, weapon);
        for (index, list) in lists.iter().enumerate() {
            let health = stack_health(world, weapon, Some(list));
            if health.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
                continue;
            }
            fraction = (f64::from(health) / f64::from(stack_health(world, weapon, None))) as f32;
            let rating = weapon_rating(
                world,
                weapon,
                enchantment,
                fraction,
                skill as i32,
                luck as i32,
                strength as i32,
                fatigue,
                -1.0,
            )?;
            if best < rating {
                best = rating;
                chosen = Some((weapon, Some(index)));
            }
        }
    }
    Ok(chosen)
}

// IDA: 00470520 TESValueForm vt+0x10 (+0x04), as an unsigned through 005479B0 (its double stored a float).
fn clothing_value(world: &World, form: Entity) -> f32 {
    let value = world
        .get::<TesValueForm>(form)
        .map_or(-1, |value| value.value_04);
    f64::from(value as u32) as f32
}

/// IDA: 0048D110 ContainerChanges_FindBestClothingForSlot(changes, base, slot, 1), 0048C870's shape over TESObjectCLOT with the
/// value (00470520, 005479B0) as the rating and no health test: a worn stack in the slot is a stop (its 0x47 test); the base
/// container's clothing, ownership counted as 0048C870's, per stack owned by anyone (0048D110 doesn't test the owner of a
/// base entry's stack) or as a whole; then the changes entries owned by nobody or the base, with a count, not listed by a
/// flagged base container, covering the slot, and with no leveled stack or a count of 0 or more, per stack or as a whole.
/// The first value strictly above the best (from -FLT_MAX) wins. Completion: 100% but the worn stack.
/// # Errors
/// As above, or an ownership extra not resolved.
fn best_clothing(
    world: &World,
    changes: &ContainerChanges,
    base: Entity,
    actor: Entity,
    slot: u32,
) -> Result<Option<Entity>, StreamError> {
    if super::cell_reference_loop::equipped_in_slot(world, actor, slot)?.is_some() {
        return Err(StreamError("0048D147: a worn stack's 0x47 test (0041DF40)"));
    }
    let mut best = -f32::MAX;
    let mut chosen = None;
    let mut consider = |value: f32, form: Entity| {
        if best < value {
            best = value;
            chosen = Some(form);
        }
    };
    let entry_of = |form: Entity| {
        changes
            .entries_00
            .iter()
            .find(|entry| entry.form_08 == form)
    };
    for (count, form) in base_entries(world, actor) {
        let clothing = (form_type(world, form) == TYPE_CLOTHING).then_some(form);
        let entry = clothing.and_then(entry_of);
        if let Some(entry) = entry {
            let lists = entry.lists_00.as_deref().unwrap_or_default();
            let first_owner = match lists.first() {
                Some(list) => list_owner(list)?,
                None => None,
            };
            let mut owned = 0;
            if first_owner != Some(base) && entry.count_04 > 0 {
                for list in lists {
                    if list_owner(list)?.is_some() {
                        owned += 1;
                    }
                }
            }
            let total_ok = (first_owner.is_none() || owned < count + entry.count_04)
                && (count + entry.count_04 > 0 || count < 0);
            if !total_ok {
                continue;
            }
        }
        let Some(clothing) = clothing else {
            continue;
        };
        if !biped_covers(world, clothing, slot) {
            continue;
        }
        let lists = entry
            .and_then(|entry| entry.lists_00.as_deref())
            .unwrap_or_default();
        if lists.is_empty() {
            consider(clothing_value(world, clothing), clothing);
        } else {
            for _ in lists {
                consider(clothing_value(world, clothing), clothing);
            }
        }
    }
    let flagged = reference_container(world, actor)
        .and_then(|container| world.get::<TesContainer>(container))
        .filter(|container| container.flags_04 & 1 != 0);
    for entry in &changes.entries_00 {
        let clothing = entry.form_08;
        if form_type(world, clothing) != TYPE_CLOTHING {
            continue;
        }
        let lists = entry.lists_00.as_deref().unwrap_or_default();
        let owner = match lists.first() {
            Some(list) => list_owner(list)?,
            None => None,
        };
        if owner.is_some() && owner != Some(base) {
            continue;
        }
        if entry.count_04 == 0 {
            continue;
        }
        let listed = flagged.is_some_and(|container| {
            container
                .entries_08
                .iter()
                .any(|item| item.item == FormRef::Resolved(Some(clothing)))
        });
        let leveled = lists
            .iter()
            .any(|list| list.get(EXTRA_LEVELED_ITEM).is_some());
        if listed || !biped_covers(world, clothing, slot) || (leveled && entry.count_04 < 0) {
            continue;
        }
        if lists.is_empty() {
            consider(clothing_value(world, clothing), clothing);
        } else {
            for _ in lists {
                consider(clothing_value(world, clothing), clothing);
            }
        }
    }
    Ok(chosen)
}

/// IDA: 005227A0(base, actor, allocation, weapon, 0, keep): TLS +0x185 saved, and cleared unless `keep` (the last argument), so the equips below record no change flags (0045B670's gate); restored at the end. vt+0x190 is 1 on a Character. A LowProcess answers vt+0x138 (0069D990) 0 and vt+0x10C, 0x110, 0x108 and 0x104 do nothing; a MiddleHigh or High process's vt+0x138 (006293C0) is the bow byte +0xF4, whose set arm (004DC5B0) is a stop, and the four store null over +0xEC, +0xF0, +0xE8 (0064AED0, 0064AF10, 0064AE90) and +0xE4 (00658550: +0xFC..+0x110 zeroed with +0xE4 null), each held selection's free a stop. 00486030(1) unequips stacks with 0x1B or 0x1C (0041DF10(0)), a stop; others stay. The armor searches 0048C870 (slots 2, 3, 5, 4, 1, 0, 13, 6, 7, 8), the weapon's 0048BDA0 and the torch's 0048B660 only read without an ARMO, a WEAP or a LIGH in the base container or the changes. 0048D110 picks the clothing per slot. Then in order: slots 8, 7, 6; 2; 3 unless 2's item covers 3; 4, 5, 1 and 0 unless an item equipped above covers them; 13. Each is vt+0x108 005F3140(form, 1, null, 0), after 004D8880, which is passed the selection, not its form, here, so it never finds the item worn. With a process, 005EDA20(1) last: each of 00B1489C's ten slots' worn item (00486790) with an enchantment (005E3DE0) is applied, which `equip_item`'s stop on an enchantment and the worn-stack stop above leave none of. Completion: 100% of those arms.
/// # Errors
/// An arm not ported, by its site.
pub(crate) fn auto_equip(
    world: &mut World,
    actor: Entity,
    allocation: bool,
    weapon: bool,
    keep_tls_185: bool,
) -> Result<(), StreamError> {
    let saved = world.resource::<GameThreadLocals>().track_changes_185;
    if !keep_tls_185 {
        world.resource_mut::<GameThreadLocals>().track_changes_185 = 0;
    }
    let result = auto_equip_body(world, actor, allocation, weapon);
    world.resource_mut::<GameThreadLocals>().track_changes_185 = saved;
    result
}

// IDA: N/A 005227A0 between its TLS +0x185 save and restore.
fn auto_equip_body(
    world: &mut World,
    actor: Entity,
    allocation: bool,
    weapon: bool,
) -> Result<(), StreamError> {
    let process = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58;
    if let Some(process) = process
        && let Some(middle) = world.get::<MiddleHighProcess>(process)
    {
        if middle.bow_f4 != 0 {
            return Err(StreamError("005227F8: 004DC5B0 for a bow (+0xF4)"));
        }
        if middle.weapon_e4.is_some()
            || middle.light_e8.is_some()
            || middle.ammo_ec.is_some()
            || middle.shield_f0.is_some()
        {
            return Err(StreamError("0064AED0: freeing a process's held selection"));
        }
    } else if let Some(process) = process {
        low_process_only(world, process, "005227F8: a process's vt+0x138")?;
    }
    let changes = ensure_container_changes(world, actor)?;
    if let Some(process) = process
        && let Some(mut middle) = world.get_mut::<MiddleHighProcess>(process)
    {
        // 0052281F..0052284F: vt+0x10C, 0x110, 0x108 and 0x104 with null; 00658550 with +0xE4 null zeroes +0xFC..+0x110.
        middle.weapon_nodes_fc = super::actor_process::WeaponNodes::default();
    }
    // 00522820 00486030(1).
    let mut changes = changes;
    unequip_worn_stacks(&mut changes, true);
    // 004867DD: a leveled stack in an entry of negative count is 004853B0's, which 00486790 runs first.
    let negative = changes.entries_00.iter().any(|entry| {
        entry.count_04 < 0
            && entry
                .lists_00
                .iter()
                .flatten()
                .any(|list| list.get(EXTRA_LEVELED_ITEM).is_some())
    });
    let mut forms: Vec<Entity> = changes
        .entries_00
        .iter()
        .map(|entry| entry.form_08)
        .collect();
    put_container_changes(world, actor, changes);
    if negative {
        return Err(StreamError("004867E8: 004853B0 on a removed leveled entry"));
    }
    forms.extend(base_entries(world, actor).into_iter().map(|(_, form)| form));
    let holds = |kind: u8| forms.iter().any(|&form| form_type(world, form) == kind);
    let (holds_weapon, holds_light) = (holds(TYPE_WEAPON), holds(TYPE_LIGHT));
    // 00522888..00522935: with the allocation flag, 0048C870 for slots 2, 3, 5, 4, 1, 0, 13, 6, 7, 8 (a5 is 0 here).
    let base =
        reference_container(world, actor).ok_or(StreamError("005227A0: an actor with no base"))?;
    let mut armor: [Option<Entity>; 16] = [None; 16];
    if allocation {
        let changes = ensure_container_changes(world, actor)?;
        let mut found = Ok(());
        for slot in [2, 3, 5, 4, 1, 0, 13, 6, 7, 8] {
            match best_armor(world, &changes, base, actor, slot, true) {
                Ok(pick) => armor[slot as usize] = pick.map(|(form, _)| form),
                Err(error) => {
                    found = Err(error);
                    break;
                }
            }
        }
        put_container_changes(world, actor, changes);
        found?;
    }
    // 00522939: with the weapon flag, 0048BDA0 (its rating out isn't used).
    let _ = holds_weapon;
    let mut weapon_pick = None;
    if weapon {
        let changes = ensure_container_changes(world, actor)?;
        let found = best_weapon(world, &changes, base, actor, true);
        put_container_changes(world, actor, changes);
        weapon_pick = found?.map(|(form, _)| form);
        if weapon_pick.is_some_and(|form| weapon_type(world, form) == 5) {
            return Err(StreamError("00522988: a bow's arrows (0048B9C0)"));
        }
    }
    if holds_light {
        return Err(StreamError("0048B660: a light in the base container"));
    }
    // 00522A08..00522B70: 0048D110 for each slot with no armor.
    let changes = ensure_container_changes(world, actor)?;
    let mut picks: [Option<Entity>; 16] = armor;
    let mut found = Ok(());
    for slot in [2, 3, 5, 4, 1, 0, 13, 8, 6, 7] {
        if picks[slot as usize].is_none() {
            match best_clothing(world, &changes, base, actor, slot) {
                Ok(pick) => picks[slot as usize] = pick,
                Err(error) => {
                    found = Err(error);
                    break;
                }
            }
        }
    }
    put_container_changes(world, actor, changes);
    found?;
    let pick = |slot: u32| picks[slot as usize];
    let slots = [
        pick(2),
        pick(3),
        pick(5),
        pick(4),
        pick(1),
        pick(0),
        pick(13),
        pick(8),
        pick(6),
        pick(7),
    ];
    let [
        upper,
        lower_3,
        lower_5,
        slot_4,
        slot_1,
        slot_0,
        slot_13,
        slot_8,
        slot_6,
        slot_7,
    ] = slots;
    for item in [slot_8, slot_7, slot_6].into_iter().flatten() {
        equip_item(world, actor, item)?;
    }
    let mut worn: Vec<u16> = Vec::new();
    let covered = |worn: &[u16], slot: u32| worn.iter().any(|&mask| covers(mask, slot));
    let equip_unless = |world: &mut World, item: Option<Entity>, worn: &mut Vec<u16>, slot: u32| {
        let Some(item) = item else {
            return Ok(());
        };
        if covered(worn, slot) {
            return Ok(());
        }
        equip_item(world, actor, item)?;
        if let Some(mask) = biped_mask(world, item) {
            worn.push(mask);
        }
        Ok::<(), StreamError>(())
    };
    // 00522BF9..00522CC2: slot 2, and slot 3 only when 2's item is absent or doesn't cover 3.
    equip_unless(world, upper, &mut worn, 2)?;
    equip_unless(world, lower_3, &mut worn, 3)?;
    for (item, slot) in [(slot_4, 4), (lower_5, 5), (slot_1, 1), (slot_0, 0)] {
        equip_unless(world, item, &mut worn, slot)?;
    }
    if let Some(item) = slot_13 {
        equip_item(world, actor, item)?;
    }
    // 00522D6A: the weapon, with its selection's count (00405110), after 004D8880.
    if let Some(item) = weapon_pick {
        equip_item(world, actor, item)?;
    }
    Ok(())
}

/// IDA: 004E1580 TESObjectREFR_UpdateBodyModels's creature arm (type 0x24) with a node, before the body models: the two flags
/// are 1 unless the actor's current package (Actor_GetCurrentPackage, +0x1C) has 0x100000 or 0x200000; in a load (+0x18 bit 2)
/// with change flag 0x8000000, 0051D460 instead, a stop; otherwise 0051E240(actor, a, b, 0). Completion: 100% of the auto-equip
/// arm.
/// # Errors
/// The load arm, or a stop in 0051E240.
pub(crate) fn creature_body_equip(world: &mut World, actor: Entity) -> Result<(), StreamError> {
    let process = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58;
    let flags = process
        .and_then(|process| current_package(world, process))
        .map_or(0, |package| package_data(world, package).flags_1c);
    if super::forms_references::save_load_flags(world)? & 2 != 0 {
        return Err(StreamError(
            "004E163D: the load arm's change flags (0051D460)",
        ));
    }
    creature_auto_equip(
        world,
        actor,
        flags & 0x10_0000 == 0,
        flags & 0x20_0000 == 0,
        false,
    )
}

/// IDA: 0051E240 TESCreature_EquipBestItems(base, actor, allocation, weapon, keep): TLS +0x185 saved and cleared unless `keep`, restored
/// at the end. Without the weapon flag, a process's vt+0x138 (the bow byte; a LowProcess's 0069D990 answers 0) reaches 004DC5B0,
/// a stop. 00486030(1) unequips the worn stacks (a stop, as 005227A0's), the process's vt+0x10C, 0x110, 0x108 and 0x104 store
/// null. With the allocation flag 0048C870 picks the slot-13 armor, with the weapon flag 0048BDA0 the weapon and, for a bow,
/// 0048B9C0 the arrows, each with the worn checks on. The shield and then the weapon are equipped through vt+0x108 (005F3140,
/// `equip_item`) when 0051D4C0 allows the form (a creature without +0x28 flag 4, weapon and shield, takes neither; an ARMO
/// only over slot 13) and 004D8880 finds no worn stack of it; the arrows' equip with the selection's count is a stop. Then a
/// process's vt+0x124(0) (the sheath node +0x108; a LowProcess's 004943A0 answers 0) with Creature vt+0x27C (0) takes
/// vt+0x300(1) (+0x114) and, dead, vt+0x308(1) (+0x115), both nothing on a LowProcess. Last, with a process, 005EDA20(1): an equipped enchanted item is a
/// stop. Completion: 100% of those arms.
/// # Errors
/// An arm not ported, by its site.
pub(crate) fn creature_auto_equip(
    world: &mut World,
    actor: Entity,
    allocation: bool,
    weapon: bool,
    keep_tls_185: bool,
) -> Result<(), StreamError> {
    let saved = world.resource::<GameThreadLocals>().track_changes_185;
    if !keep_tls_185 {
        world.resource_mut::<GameThreadLocals>().track_changes_185 = 0;
    }
    let result = creature_auto_equip_body(world, actor, allocation, weapon);
    world.resource_mut::<GameThreadLocals>().track_changes_185 = saved;
    result
}

// IDA: 004D8880 TESObjectREFR_IsFormWorn(actor, form): with a base container, whether the changes' entry for the form holds a worn stack (00485FA0
// with 0: its equip-locked arm doesn't run).
fn form_worn(world: &mut World, actor: Entity, form: Entity) -> Result<bool, StreamError> {
    if reference_container(world, actor).is_none() {
        return Ok(false);
    }
    let changes = ensure_container_changes(world, actor)?;
    let found = changes
        .entries_00
        .iter()
        .find(|entry| entry.form_08 == form)
        .is_some_and(|entry| {
            entry
                .lists_00
                .iter()
                .flatten()
                .any(|list| super::inventory_entries::worn(list, false))
        });
    put_container_changes(world, actor, changes);
    Ok(found)
}

// IDA: 0051D4C0 TESActorBase_CanEquipForm(base, form): 00519D00 TESActorBase_CanUseWeaponAndShield (an NPC, or a creature with +0x28 flag 4) and then an ARMO over slot 13, a LIGH, a WEAP
// or an AMMO.
fn creature_can_equip(world: &World, base: Entity, form: Entity) -> bool {
    let allowed = form_type(world, base) != TYPE_CREATURE
        || world
            .get::<super::form_components::TesActorBaseData>(base)
            .is_some_and(|data| data.flags_04 & 4 != 0);
    allowed
        && match form_type(world, form) {
            TYPE_ARMOR => biped_mask(world, form).is_some_and(|mask| covers(mask, 13)),
            TYPE_LIGHT | TYPE_WEAPON | 0x22 => true,
            _ => false,
        }
}

// IDA: N/A 0051E240 between its TLS +0x185 save and restore.
fn creature_auto_equip_body(
    world: &mut World,
    actor: Entity,
    allocation: bool,
    weapon: bool,
) -> Result<(), StreamError> {
    let process = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58;
    if let Some(process) = process {
        if let Some(middle) = world.get::<MiddleHighProcess>(process) {
            if !weapon && middle.bow_f4 != 0 {
                return Err(StreamError("0051E297: 004DC5B0 for a bow (+0xF4)"));
            }
            if middle.weapon_e4.is_some()
                || middle.light_e8.is_some()
                || middle.ammo_ec.is_some()
                || middle.shield_f0.is_some()
            {
                return Err(StreamError("0064AED0: freeing a process's held selection"));
            }
        } else {
            low_process_only(world, process, "0051E2D6: a process's vt+0x10C..0x104")?;
        }
    }
    // 0051E2C1 00486030(1).
    let mut changes = ensure_container_changes(world, actor)?;
    unequip_worn_stacks(&mut changes, true);
    put_container_changes(world, actor, changes);
    if let Some(process) = process
        && let Some(mut middle) = world.get_mut::<MiddleHighProcess>(process)
    {
        middle.weapon_nodes_fc = super::actor_process::WeaponNodes::default();
    }
    let base =
        reference_container(world, actor).ok_or(StreamError("0051E240: an actor with no base"))?;
    let mut shield = None;
    if allocation {
        let changes = ensure_container_changes(world, actor)?;
        let found = best_armor(world, &changes, base, actor, 13, true);
        put_container_changes(world, actor, changes);
        shield = found?.map(|(form, _)| form);
    }
    let mut weapon_pick = None;
    let mut ammo = None;
    if weapon {
        let changes = ensure_container_changes(world, actor)?;
        let found = best_weapon(world, &changes, base, actor, true);
        let found = found.and_then(|pick| {
            let pick = pick.map(|(form, _)| form);
            let arrows = match pick {
                Some(form) if weapon_type(world, form) == 5 => {
                    best_ammo(world, &changes, actor, true)?
                }
                _ => None,
            };
            Ok((pick, arrows))
        });
        put_container_changes(world, actor, changes);
        (weapon_pick, ammo) = found?;
    }
    if let Some(form) = shield
        && creature_can_equip(world, base, form)
        && !form_worn(world, actor, form)?
    {
        equip_item(world, actor, form)?;
    }
    if let Some(form) = weapon_pick
        && creature_can_equip(world, base, form)
        && !form_worn(world, actor, form)?
    {
        equip_item(world, actor, form)?;
        if let Some(arrows) = ammo
            && !form_worn(world, actor, arrows)?
        {
            return Err(StreamError(
                "0051E3F5: the arrows' equip with the selection's count",
            ));
        }
        // 0051E41E: vt+0x124(0) (MiddleHigh 006508D0: +0x108, the sheath node; a LowProcess's 004943A0: 0) and Creature vt+0x27C
        // (0); with neither, vt+0x300(1) (0064AD20: +0x114) and, dead, vt+0x308(1) (0064AD40: +0x115). A LowProcess's two do
        // nothing.
        if let Some(process) = process {
            let dead = actor_is_dead(world, actor, false);
            if let Some(mut middle) = world.get_mut::<MiddleHighProcess>(process) {
                if middle.weapon_nodes_fc.sheath_108.is_none() {
                    middle.byte_114 = 1;
                    if dead {
                        middle.byte_115 = 1;
                    }
                }
            } else {
                low_process_only(
                    world,
                    process,
                    "0051E41E: a process's vt+0x124 and vt+0x300",
                )?;
            }
        }
    }
    if process.is_some() {
        let changes = ensure_container_changes(world, actor)?;
        let enchanted = changes.entries_00.iter().any(|entry| {
            enchantment_of(world, entry.form_08).is_some()
                && entry
                    .lists_00
                    .iter()
                    .flatten()
                    .any(|list| super::inventory_entries::worn(list, false))
        });
        put_container_changes(world, actor, changes);
        if enchanted {
            return Err(StreamError(
                "005EDA9B: an equipped item's enchantment (005E3DE0)",
            ));
        }
    }
    Ok(())
}

// IDA: 005F3422's table before 00489C30, for an actor with a process: an ARMO covering slot 13 unequips the process's shield
// (vt+0xF8(0)), a WEAP its weapon (vt+0xFC(0)), both stops here, and a WEAP with a worn light (vt+0xF0(1)) and a two-hand
// test reaches 004DCAB0, a stop; any other ARMO, and a CLOT, takes nothing. A LIGH or an AMMO is a stop.
fn process_equip_checks(
    world: &World,
    process: Entity,
    kind: u8,
    shield: bool,
) -> Result<(), StreamError> {
    let Some(middle) = world.get::<MiddleHighProcess>(process) else {
        return match kind {
            TYPE_CLOTHING | TYPE_ARMOR => Ok(()),
            _ => Err(StreamError("005F3422: a Low process's weapon or light arm")),
        };
    };
    match kind {
        TYPE_ARMOR if shield && middle.shield_f0.is_some() => {
            Err(StreamError("005F34A5: unequipping the process's shield"))
        }
        TYPE_WEAPON if middle.weapon_e4.is_some() => {
            Err(StreamError("005F3560: unequipping the process's weapon"))
        }
        TYPE_WEAPON if middle.light_e8.is_some_and(|light| light.worn) => Err(StreamError(
            "005F35B7: a weapon over a worn light (004DCAB0)",
        )),
        TYPE_LIGHT | 0x22 => Err(StreamError("005F3422: a light's or an ammo's arm")),
        _ => Ok(()),
    }
}

// IDA: 00489C30's process arm (0048A0BC) after the changes' equip, its stack v26 now worn: a CLOT ring or amulet (slots 6, 7,
// 8) is a stop; any other CLOT or ARMO takes vt+0x31C(1) (006295E0, +0x16C). An ARMO covering slot 13: with no worn shield
// (vt+0xF8(1)) a new entry of the form, the count and the stack goes to vt+0x110 (0064AF10, +0xF0); a WEAP: with no worn
// weapon (vt+0xFC(1)) a new entry to vt+0x104 (00658550: +0xE4, +0xF4 for a bow, +0xF5 for a staff; with +0x17C anim data
// 006541A0, a stop; otherwise +0xFC..+0x110 zeroed), and on true the anim data's +0x60 takes the weapon's +0x94 (no anim data
// here). Each then with +0x16C clear (vt+0x320) 004E1B40 / 004E1900, a stop. Last 005EA1A0 on the node: no 3D returns.
fn process_equip(
    world: &mut World,
    actor: Entity,
    process: Entity,
    item: Entity,
    kind: u8,
    shield: bool,
) -> Result<(), StreamError> {
    if kind == TYPE_CLOTHING
        && biped_mask(world, item)
            .is_some_and(|mask| [6, 7, 8].into_iter().any(|slot| covers(mask, slot)))
    {
        // 0048A0C9: a ring (slots 6, 7) or an amulet (8) takes 004DCE60 / 004DCF90 only with +0x16C clear (vt+0x320), a stop
        // here; it doesn't take vt+0x31C(1).
        let pending = world
            .get::<MiddleHighProcess>(process)
            .map(|middle| middle.equipment_update_pending_16c);
        return match pending {
            Some(0) => Err(StreamError(
                "0048A0D4: 004DCE60 / 004DCF90, a ring's or amulet's 3D",
            )),
            Some(_) => Ok(()),
            None => low_process_only(world, process, "0048A0C9: a Low process's ring or amulet"),
        };
    }
    let weapon_kind = (kind == TYPE_WEAPON).then(|| weapon_type(world, item));
    let player = the_player(world) == Some(actor);
    let Some(mut middle) = world.get_mut::<MiddleHighProcess>(process) else {
        return low_process_only(world, process, "0048A113: a process's vt+0x31C");
    };
    let entry = super::actor_process::EquippedEntry {
        form_08: item,
        count_04: 1,
        worn: true,
    };
    if shield {
        if middle.shield_f0.is_some_and(|held| held.worn) {
            return Err(StreamError("0048A15B: the worn shield entry's restack"));
        }
        middle.shield_f0 = Some(entry);
        if middle.equipment_update_pending_16c == 0 {
            return Err(StreamError("0048A1D0: 004E1B40, the shield's 3D"));
        }
    } else if let Some(weapon_kind) = weapon_kind {
        if middle.weapon_e4.is_some_and(|held| held.worn) {
            return Err(StreamError("0048A3A1: the worn weapon entry's restack"));
        }
        middle.weapon_e4 = Some(entry);
        middle.bow_f4 = u8::from(weapon_kind == 5);
        middle.staff_f5 = u8::from(weapon_kind == 4);
        if let Some(anim) = middle.anim_data_17c {
            // 006585A9: with animation data, 006541A0 on its manager and node, or (no node yet) 0 and the reference's 3D; the
            // player's first-person data (0065D750) is a stop here.
            if player {
                return Err(StreamError(
                    "006585E0: the player's first-person weapon nodes",
                ));
            }
            let item_type = Some(weapon_kind);
            let (manager, anim_node) = world
                .get::<super::actor_animation::ActorAnimData>(anim)
                .map_or((None, None), |data| (data.manager_98, data.node_04));
            let nodes = match anim_node {
                Some(node) => weapon_bone_nodes(world, manager, Some(node), item_type)?,
                None => {
                    let node = world
                        .get::<super::reference_3d::Reference3D>(actor)
                        .map(|loaded| loaded.0);
                    weapon_bone_nodes(world, None, node, item_type)?
                }
            };
            world
                .get_mut::<MiddleHighProcess>(process)
                .expect("a process")
                .weapon_nodes_fc = nodes;
            if let Some(weapon) = nodes.weapon_fc {
                detach_scabbard(world, weapon)?;
            }
            // 0048A4B4: as below, with +0x16C clear 004E1900. An actor with a node and no skin info (a creature) attaches the
            // weapon through 00479450: host deferral D15 (`CreatureBodyDeferrals`); one with a skin info (00479780) is a stop.
            let pending = world
                .get::<MiddleHighProcess>(process)
                .expect("a process")
                .equipment_update_pending_16c;
            if pending == 0 {
                let has_node = world
                    .get::<super::reference_3d::Reference3D>(actor)
                    .is_some();
                let has_skin = world
                    .get::<Character>(actor)
                    .is_some_and(|character| character.skin_info_104.is_some());
                if !has_node || has_skin {
                    return Err(StreamError("0048A4B4: 004E1900, the weapon's 3D"));
                }
                world
                    .get_resource_or_init::<super::reference_3d::CreatureBodyDeferrals>()
                    .weapons
                    .push((actor, item));
            }
            return Ok(());
        }
        middle.weapon_nodes_fc = super::actor_process::WeaponNodes::default();
        if player {
            return Err(StreamError("006585F0: the player's 00B3BA84..00B3BA98"));
        }
        if middle.equipment_update_pending_16c == 0 {
            return Err(StreamError("0048A4B4: 004E1900, the weapon's 3D"));
        }
    } else {
        // 0048A113: vt+0x31C(1), 006295E0.
        middle.equipment_update_pending_16c = 1;
    }
    Ok(())
}

// IDA: 006541A0 MiddleHighProcess_FindWeaponNodes(process, manager, node) for a non-player: the sheath index 4 (BackWeapon) for weapon types 1 and 3..5, else 5
// (SideWeapon). A manager looks each name up in its +0x7C object palette (vt+0x4C) alone; with none, 006F94A0 on the node. "Weapon" +0xFC, the sheath
// +0x108, "Torch" +0x100, "Bip01 L ForearmTwist" +0x104, "Quiver" +0x10C, +0x110 = 0. No node stores only +0x110.
fn weapon_bone_nodes(
    world: &World,
    manager: Option<Entity>,
    node: Option<Entity>,
    weapon_kind: Option<u8>,
) -> Result<super::actor_process::WeaponNodes, StreamError> {
    let mut nodes = super::actor_process::WeaponNodes::default();
    let sheath: &[u8] = match weapon_kind {
        Some(1 | 3..=5) => b"BackWeapon",
        _ => b"SideWeapon",
    };
    if manager.is_none() && node.is_none() {
        return Ok(nodes);
    }
    let find = |name: &[u8]| match (manager, node) {
        (Some(manager), _) => palette_object(world, manager, name),
        (None, Some(node)) => object_by_name_with_palette(world, node, name),
        (None, None) => None,
    };
    nodes.weapon_fc = find(b"Weapon");
    nodes.sheath_108 = find(sheath);
    nodes.torch_100 = find(b"Torch");
    nodes.forearm_twist_104 = find(b"Bip01 L ForearmTwist");
    nodes.quiver_10c = find(b"Quiver");
    Ok(nodes)
}

// IDA: a NiControllerManager's +0x7C NiDefaultAVObjectPalette vt+0x4C GetAVObject (006C5430); no palette answers null.
fn palette_object(world: &World, manager: Entity, name: &[u8]) -> Option<Entity> {
    world
        .get::<crate::gamebryo::controller_sequence::ControllerManager>(manager)
        .and_then(|manager| manager.object_palette_7c)
        .and_then(|palette| {
            world.get::<crate::gamebryo::av_object_palette::DefaultAvObjectPalette>(palette)
        })
        .and_then(|palette| palette.get_av_object(name))
}

/// IDA: 006F94A0 NiObjectNET_GetObjectByNameWithPalette(node, name): a NiControllerManager in the node's chain (00B3CAC0)
/// looks in its palette first; a miss, or no manager, is the node's GetObjectByName (vt+0x58). Completion: 100%.
pub(crate) fn object_by_name_with_palette(
    world: &World,
    node: Entity,
    name: &[u8],
) -> Option<Entity> {
    crate::gamebryo::controller_sequence::find_controller_manager(world, node)
        .and_then(|manager| palette_object(world, manager, name))
        .or_else(|| crate::bs::model_loader::object_by_name(world, node, name))
}

/// IDA: the process arms 004E3490's LABEL_94 runs for an actor whose process is level 0 or 1 (a MiddleHigh or High one):
/// vt+0xEC(1) (0064B240: +0xE4 when worn) into vt+0x104 (00658550: the same entry keeps its stack; with animation data,
/// 006541A0 on its manager and node), then vt+0x114 (0064F5E0: +0x110 the "ArrowBone" under the anim data's node with a weapon,
/// else null). A lower level, or no process, does nothing. Completion: 100% of a non-player with the same weapon entry.
/// # Errors
/// The player's arms, or a different weapon entry.
pub(crate) fn process_animation_nodes(
    world: &mut World,
    actor: Entity,
    anim: Option<Entity>,
) -> Result<(), StreamError> {
    let Some(process) = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
    else {
        return Ok(());
    };
    if super::actor_process::process_level(world, process) > 1 {
        return Ok(());
    }
    // 004E3EAC: the player also takes vt+0x114 with 0065D750(1)'s first-person anim data; with a weapon that sets 00B3BA98
    // (a stop here), with none +0x110 stays null. Its 00658550 with no weapon zeroes 00B3BA84..00B3BA98, already zero.
    if the_player(world) == Some(actor)
        && world
            .get::<MiddleHighProcess>(process)
            .is_some_and(|middle| middle.weapon_e4.is_some())
    {
        return Err(StreamError(
            "004E3EAC: the player's first-person weapon nodes (00B3BA84..00B3BA98)",
        ));
    }
    let middle = world
        .get::<MiddleHighProcess>(process)
        .cloned()
        .ok_or(StreamError(
            "004E3E78: a level 0 or 1 process with no MiddleHigh part",
        ))?;
    let worn = middle.weapon_e4.filter(|entry| entry.worn);
    if worn.is_none() && middle.weapon_e4.is_some() {
        return Err(StreamError(
            "00658563: replacing the process's weapon entry with null",
        ));
    }
    let data = anim.and_then(|anim| {
        world
            .get::<super::actor_animation::ActorAnimData>(anim)
            .map(|data| (data.manager_98, data.node_04))
    });
    // 00658550: with animation data and a weapon entry, 006541A0; otherwise +0xFC..+0x110 zeroed.
    let mut nodes = super::actor_process::WeaponNodes::default();
    if let (Some(entry), Some((manager, node))) = (worn, data) {
        let kind = Some(weapon_type(world, entry.form_08));
        nodes = if node.is_some() {
            weapon_bone_nodes(world, manager, node, kind)?
        } else {
            let root = world
                .get::<super::reference_3d::Reference3D>(actor)
                .map(|loaded| loaded.0);
            weapon_bone_nodes(world, None, root, kind)?
        };
        if let Some(weapon) = nodes.weapon_fc {
            detach_scabbard(world, weapon)?;
        }
    }
    nodes.arrow_bone_110 = match (data, middle.weapon_e4) {
        (Some((_, Some(node))), Some(_)) => object_by_name_with_palette(world, node, b"ArrowBone"),
        _ => None,
    };
    world
        .get_mut::<MiddleHighProcess>(process)
        .expect("read above")
        .weapon_nodes_fc = nodes;
    Ok(())
}

// IDA: 00480770 NiNode_DetachScabbard(node): the first child named "Scb" (00A3CE0C) is detached (vt+0x8C), a stop here; a
// "FadeNode " child restarts the walk on it as a NiNode (vt+0x08). Answers whether one was found. Completion: 100% with no "Scb".
fn detach_scabbard(world: &World, node: Entity) -> Result<(), StreamError> {
    let mut cursor = Some(node);
    'walk: while let Some(at) = cursor {
        let children = world
            .get::<crate::gamebryo::scene_node::NiNodeChildren>(at)
            .map(|children| children.slots_ac.clone())
            .unwrap_or_default();
        for child in children.into_iter().flatten() {
            let Some(name) = world
                .get::<crate::gamebryo::object_net::NiObjectNet>(child)
                .and_then(|net| net.name.clone())
            else {
                continue;
            };
            if recomp_core::crt::stricmp(&name, b"Scb") == 0 {
                return Err(StreamError("004807D3: detaching a weapon node's scabbard"));
            }
            if recomp_core::crt::strnicmp(&name, b"FadeNode ", 9) == 0 {
                cursor = world
                    .get::<crate::gamebryo::scene_node::NiNodeChildren>(child)
                    .map(|_| child);
                continue 'walk;
            }
        }
        break;
    }
    Ok(())
}

// IDA: 005EA1A0 Actor_UpdateSkinProperty(actor, node) with a process and a NiNode: the skin attachment node, 006F94A0(node, "SkinAttachment") (a
// NiControllerManager's palette is a stop), else the first NiNode child whose name doesn't start "BIP" (00A6E684), else the
// node; then 00480630 on it: from the last child back, a NiNode child whose first child has a type-4 property (00706DA0) of
// vt+0x54 5..10; none falls to 00480560's walk in child order (a NiNode recursed, a geometry's type-4 property tested). The
// result goes to the process's vt+0x3A8 (006349D0 on a MiddleHigh or High process: +0x184, counted; nothing on a Low or
// MiddleLow one). The property's vt+0x54 isn't ported: a type-4 property on a tested object is a stop. Completion: 100% of a
// tree with none (null stored).
pub(crate) fn find_skin_property(
    world: &mut World,
    process: Entity,
    node: Entity,
) -> Result<(), StreamError> {
    use crate::gamebryo::av_object::NiAvProperties;
    use crate::gamebryo::scene_node::NiNodeChildren;
    if world.get::<NiNodeChildren>(node).is_none() {
        return Ok(());
    }
    if crate::gamebryo::controller_sequence::find_controller_manager(world, node).is_some() {
        return Err(StreamError(
            "006F94BA: a NiControllerManager's object palette",
        ));
    }
    let children = |world: &World, at: Entity| -> Vec<Entity> {
        world
            .get::<NiNodeChildren>(at)
            .map(|children| children.slots_ac.iter().flatten().copied().collect())
            .unwrap_or_default()
    };
    let shaded = |world: &World, object: Entity| -> Result<bool, StreamError> {
        let properties = world
            .get::<NiAvProperties>(object)
            .map(|list| list.0.clone())
            .unwrap_or_default();
        for property in properties.into_iter().flatten() {
            if crate::gamebryo::scene_state::property_type(world, Some(property))? == 4 {
                return Ok(true);
            }
        }
        Ok(false)
    };
    let attachment = crate::bs::model_loader::object_by_name(world, node, b"SkinAttachment")
        .or_else(|| {
            children(world, node).into_iter().find(|&child| {
                world.get::<NiNodeChildren>(child).is_some()
                    && world
                        .get::<crate::gamebryo::object_net::NiObjectNet>(child)
                        .and_then(|net| net.name.as_deref())
                        .is_some_and(|name| recomp_core::crt::strnicmp(name, b"BIP", 3) != 0)
            })
        })
        .unwrap_or(node);
    for child in children(world, attachment).into_iter().rev() {
        if world.get::<NiNodeChildren>(child).is_some()
            && let Some(&first) = world
                .get::<NiNodeChildren>(child)
                .and_then(|children| children.slots_ac.first())
                .and_then(Option::as_ref)
            && shaded(world, first)?
        {
            return Err(StreamError("00480675: a shade property's vt+0x54"));
        }
    }
    let mut stack = vec![attachment];
    while let Some(at) = stack.pop() {
        for child in children(world, at).into_iter().rev() {
            if world.get::<NiNodeChildren>(child).is_some() {
                stack.push(child);
            } else if shaded(world, child)? {
                return Err(StreamError("004805C9: a shade property's vt+0x54"));
            }
        }
    }
    if let Some(mut middle) = world.get_mut::<MiddleHighProcess>(process) {
        middle.skin_property_184 = 0;
    }
    Ok(())
}

/// IDA: 005F2E70 Actor_UnequipItem(form, 1, null, 0, 0, 1), with no stack list: nothing locked. An ARMO first takes vt
/// InvalidateArmorRatingCache (+0x108 = -1.0), then the ARMO and CLOT arm: 006A2110(form, 1) on the magic target (its active
/// effects from the item: none here, a stop otherwise); for the player with menu mode 006A2280(0.0) over the same empty list,
/// PlayerCharacter_RemovePendingEquipmentEnchantment (+0x1FC null; a list is a stop) and 005E99C0's unequip sound (none under
/// the LoadingMenu: the manager's stack top 1007, the root's user0 test aside; another top is a stop); then 005E4260: with the
/// changes and no save active, 004FBF90's OnUnequip on the null list (nothing) and on the actor's ExtraScript (0x12) event list
/// (a stop), 0048A540 (no entry for the form returns), and the form's enchantment's 0041A7E0 (a stop). a7 = 1 skips
/// PlayerCharacter_RefreshEquippedItemVisuals. Completion: 100% of an unstacked ARMO or CLOT with no effects or script.
fn unequip_item_unstacked(
    world: &mut World,
    actor: Entity,
    form: Entity,
) -> Result<(), StreamError> {
    let kind = form_type(world, form);
    if !matches!(kind, TYPE_ARMOR | TYPE_CLOTHING) {
        return Err(StreamError("005F2EC0: unequipping a light, weapon or ammo"));
    }
    if kind == TYPE_ARMOR
        && let Some(mut character) = world.get_mut::<Character>(actor)
    {
        character.armor_rating_cache_108 = -1.0;
    }
    let player = the_player(world) == Some(actor);
    if player {
        let data = world
            .get::<super::player::PlayerCharacter>(actor)
            .expect("the player");
        if data.active_effects_1e4.is_some_and(|list| list != [0; 2]) {
            return Err(StreamError(
                "006A2110: an active effect from the unequipped item",
            ));
        }
        if data.pending_enchanted_equipment_1fc != 0 {
            return Err(StreamError(
                "00662C8E: the player's pending equipment enchantments",
            ));
        }
        let top = world
            .get_resource::<super::interface_manager::InterfaceManager>()
            .map(|manager| {
                super::interface_frame::top_stacked_menu_mode(&manager.menu_mode_stack_0e0)
            });
        if top != Some(1007) {
            return Err(StreamError(
                "005E99D8: the unequip sound outside the LoadingMenu",
            ));
        }
    } else {
        return Err(StreamError("005F2EE6: a non-player's clothing unequip"));
    }
    let changes = world
        .get::<TesObjectRefr>(actor)
        .expect("a reference")
        .extra_data_44
        .get(EXTRA_CONTAINER_CHANGES)
        .is_some();
    if changes {
        if !super::forms_references::save_load_active(world)?
            && world
                .get::<TesObjectRefr>(actor)
                .expect("a reference")
                .extra_data_44
                .get(0x12)
                .is_some()
        {
            return Err(StreamError(
                "004FBF90: an OnUnequip event on the actor's script",
            ));
        }
        let held = ensure_container_changes(world, actor)?;
        let entry = held.entries_00.iter().any(|entry| entry.form_08 == form);
        put_container_changes(world, actor, held);
        if entry {
            return Err(StreamError("0048A540: the changes' unequip of an entry"));
        }
    }
    if enchantment_of(world, form).is_some() {
        return Err(StreamError(
            "005E42DE: releasing the item's enchantment (0041A7E0)",
        ));
    }
    Ok(())
}

// IDA: 005F3140 Actor vt+0x108(form, 1, null, 0): vt+0x168 (Character 004349A0, +0x104; the player's 006600D0(0), the same +0x104) and 004691B0's biped. A ring is a stop; with a +0x104, each skin slot holding a TESBipedModelForm over a slot the item covers takes 005F2E70 (`unequip_item_unstacked`). With a process, the 005F3422 table's ARMO, LIGH, WEAP and AMMO arms are stops; CLOT takes the default. Then 00489C30, whose process arm for CLOT (0048A0BC) is a stop for slots 6, 7 and 8 and otherwise vt+0x31C(1), nothing on a LowProcess, then 005EA1A0(+0x3C), which returns on no 3D. Then the visuals of an ARMO with slot 13, a LIGH or a WEAP (005E7230), a TESEnchantableForm's enchantment (0041A610), and the 0x51 extra are stops. Completion: 100% of the clothing arm, with no process or a LowProcess.
fn equip_item(world: &mut World, actor: Entity, item: Entity) -> Result<(), StreamError> {
    // 005F3169: the player's skin info is 006600D0 PlayerCharacter_GetSkinInfo(0), +0x104, the field vt+0x168 reads on a
    // Character; its later arms (005F3535's shield unequip, 005F37DB's telekinesis after an enchantment) follow stops below.
    let skin = world
        .get::<Character>(actor)
        .and_then(|character| character.skin_info_104);
    if let Some(mask) = biped_mask(world, item) {
        if covers(mask, 7) || covers(mask, 6) {
            return Err(StreamError("005F320F: a ring's hand"));
        }
        // 005F32F0..005F33DD: per slot the item covers, each skin slot whose form is a TESBipedModelForm covering it (and not of
        // type 9) takes 005F2E70(form, 1, the slot's equipped stack, 0, 0, 1) unless that stack is equip-locked; a race's body
        // part isn't a TESBipedModelForm, so it stays. The same form is unequipped too: the skin info keeps it after 00486030.
        if let Some(skin) = skin {
            for slot in (0..16).filter(|&slot| covers(mask, slot)) {
                let slots = world
                    .get::<super::character_3d::SkinInfo>(skin)
                    .expect("a skin info")
                    .slots_4c;
                for held in slots {
                    let Some(form) = held.form_00 else {
                        continue;
                    };
                    if form_type(world, form) == 9 || !biped_covers(world, form, slot) {
                        continue;
                    }
                    if super::cell_reference_loop::equipped_in_slot(world, actor, slot)?.is_some() {
                        return Err(StreamError(
                            "005F3381: a skin slot's form with an equipped stack",
                        ));
                    }
                    unequip_item_unstacked(world, actor, form)?;
                }
            }
        }
    }
    let process = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58;
    let kind = form_type(world, item);
    let shield = kind == TYPE_ARMOR && biped_mask(world, item).is_some_and(|mask| covers(mask, 13));
    if let Some(process) = process {
        process_equip_checks(world, process, kind, shield)?;
    }
    let mut changes = ensure_container_changes(world, actor)?;
    let result = equip_in_changes(world, &mut changes, item, 1);
    put_container_changes(world, actor, changes);
    result?;
    if let Some(process) = process {
        process_equip(world, actor, process, item, kind, shield)?;
        // 0048A4F0 005EA1A0(actor, +0x3C).
        let node = world
            .get::<super::reference_3d::Reference3D>(actor)
            .map(|loaded| loaded.0);
        if let Some(node) = node {
            find_skin_property(world, process, node)?;
        }
    }
    if kind == TYPE_LIGHT {
        return Err(StreamError("005F379C: a light's visuals (005E7230)"));
    }
    if kind == TYPE_WEAPON || shield {
        super::cell_reference_loop::refresh_equipped_visuals_for(world, actor, item)?;
    }
    if world
        .get::<TesEnchantableForm>(item)
        .is_some_and(|form| matches!(form.enchantment_04, FormRef::Resolved(Some(_))))
    {
        return Err(StreamError(
            "005F37CC: an enchantment's resources (0041A610)",
        ));
    }
    if world
        .get::<TesObjectRefr>(actor)
        .expect("a reference")
        .extra_data_44
        .get(0x51)
        .is_some()
    {
        return Err(StreamError("005F3804: the 0x51 extra's refraction"));
    }
    Ok(())
}

// IDA: 00489C30's arm for an item with a changes entry, with no stack given (n6a null), not left and not locked (a6, a7 0): the
// base container's count (TESContainer_GetItemCount), the entry's plus it, less the loose (004845D0) and default (00484620)
// stacks; a first stack that's leveled uses the base count's absolute value. Above 0, a new list with 0x1B goes to the entry's
// head (LABEL_84). Otherwise a worn entry returns at once (00489E57, before the process arm, a stop here), and the first stack
// takes 0x1B (0041F200(1, 0)) and loses its ExtraCount (004238A0(1)) unless it holds 0x47 (the locked equip, 0041DF40), then
// 0041F370(0) clears a 0x47 there. Completion: 100% of those arms; a count above 1 and a locked first stack are stops.
fn equip_existing_entry(
    world: &World,
    changes: &mut ContainerChanges,
    index: usize,
    item: Entity,
    count: i32,
) -> Result<(), StreamError> {
    use super::inventory_entries::{
        default_for_container, entry_loose_count, leveled, stack_count, worn,
    };
    if count > 1 {
        return Err(StreamError("0048A029: a worn stack's count"));
    }
    let base_count = reference_container(world, changes.owner_04)
        .and_then(|base| world.get::<TesContainer>(base))
        .and_then(|container| {
            container
                .entries_08
                .iter()
                .find(|entry| entry.item == FormRef::Resolved(Some(item)))
                .map(|entry| entry.count)
        })
        .unwrap_or(0);
    let entry = &mut changes.entries_00[index];
    let lists = entry.lists_00.as_deref().unwrap_or_default();
    let held = if lists.is_empty() {
        0
    } else {
        let defaults: i32 = lists
            .iter()
            .filter(|list| default_for_container(list, false))
            .map(stack_count)
            .sum();
        entry_loose_count(entry) + defaults
    };
    let mut free = entry.count_04 + base_count - held;
    if lists.first().is_some_and(leveled) {
        free = base_count.abs() - held;
    }
    let new_worn_list = || {
        let mut list = ExtraDataList {
            original_vtable: EXTRA_DATA_LIST_VTABLE,
            ..ExtraDataList::default()
        };
        list.add(new_extra_entry(EXTRA_WORN, ExtraData::Marker));
        list
    };
    if free > 0 {
        entry
            .lists_00
            .get_or_insert_with(Vec::new)
            .insert(0, new_worn_list());
        return Ok(());
    }
    if super::inventory_entries::entry_is_worn(entry, false) {
        return Err(StreamError(
            "00489E57: equipping a worn entry returns before the process arm",
        ));
    }
    let Some(list) = entry.lists_00.as_mut().and_then(|lists| lists.first_mut()) else {
        entry
            .lists_00
            .get_or_insert_with(Vec::new)
            .insert(0, new_worn_list());
        return Ok(());
    };
    if list.present(EXTRA_LOCKED_EQUIP) {
        return Err(StreamError("00489F6A: a locked (0x47) first stack"));
    }
    if !worn(list, false) || !list.present(EXTRA_WORN) {
        list.add(new_extra_entry(EXTRA_WORN, ExtraData::Marker));
    }
    if list.present(super::base_extra_list::EXTRA_COUNT) {
        list.remove_by_type(super::base_extra_list::EXTRA_COUNT);
    }
    Ok(())
}

// IDA: 0x47, the locked equip: 0041F370(lock) sets or clears it (Actor_EquipItem's last argument), 0041DF40 tests it (byte +0x10
// bit 7), and the auto-equip searches keep a worn stack that has it.
const EXTRA_LOCKED_EQUIP: u8 = 0x47;

// IDA: 00489C30(changes, form, count, actor, null, 0, 0) for an item with no changes entry, up to its process arm (0048A081), which `equip_item` takes: +0x08 and +0x0C = -1.0, the owner's vt+0x40(0x8000000), and for an ARMO or CLOT covering none of slots 6, 7, 8 and 13 vt+0x40(0x20000000). 00484420(form, 0) is appended (0067B1E0), and a new 00422EE0 list with 0041F200(1, 0)'s ExtraWorn (a count above 1 is 004238A0's) is prepended to its stacks. 0041F370(0) has no 0x47 to remove. Completion: 100% of that arm.
fn equip_in_changes(
    world: &mut World,
    changes: &mut ContainerChanges,
    item: Entity,
    count: i32,
) -> Result<(), StreamError> {
    changes.float_08 = MINUS_ONE;
    changes.float_0c = MINUS_ONE;
    let owner = changes.owner_04;
    set_change_flags(world, owner, 0x0800_0000)?;
    if let Some(mask) = biped_mask(world, item)
        && ![7, 6, 8, 13].into_iter().any(|slot| covers(mask, slot))
    {
        set_change_flags(world, owner, 0x2000_0000)?;
    }
    if let Some(index) = changes
        .entries_00
        .iter()
        .position(|entry| entry.form_08 == item)
    {
        return equip_existing_entry(world, changes, index, item, count);
    }
    if count > 1 {
        return Err(StreamError("0048A029: a worn stack's count"));
    }
    let mut list = ExtraDataList {
        original_vtable: EXTRA_DATA_LIST_VTABLE,
        ..ExtraDataList::default()
    };
    // 0041F200(1, 0): 0x1B unless its bit is set, which a new list's isn't.
    list.add(new_extra_entry(EXTRA_WORN, ExtraData::Marker));
    changes.entries_00.push(InventoryEntry {
        lists_00: Some(vec![list]),
        count_04: 0,
        form_08: item,
    });
    Ok(())
}
