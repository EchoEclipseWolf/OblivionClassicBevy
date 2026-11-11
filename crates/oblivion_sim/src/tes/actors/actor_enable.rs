//! The arms `004DD850` takes for a disabled actor: `005E5EC0`'s inventory reset (`004DDB00`) and auto-equip (`005227A0`, then `005F3140` and `00489C30` per item), the LowProcess and its add, `005F23E0`, and the first scheduled package pass (`0064A380`). A new game's command 30, `Enable` on `90028F5`, is the first to get here.
//!
//! Evidence: `records-run-u-status1-cover.txt` lines 1004 to 1326 in `evidence/gameplay-run001-2026-11-07/`, and the actor in `run-v-moveto`'s snapshot.

use std::path::Path;
use std::rc::Rc;

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::logging::{Level, emit};

use super::actor::{Actor, Character, MobileObject, TesObjectRefr, actor_process_level};
use super::actor_process::{
    BaseProcess, LOW_PROCESS_VTABLE, LowProcess, MiddleHighProcess, ProcessInputs,
    actor_process_manager_add, actor_process_manager_remove, construct_low_process,
    delete_low_process,
};
use super::equipment::{changes_unequip_item, enchantment_of, update_skin_property_after_unequip};
use super::inventory::{
    ContainerChanges, ensure_container_changes, put_container_changes, reference_container,
    resolve_leveled_items, run_item_scripts,
};
use crate::gamebryo::object_net::object_by_name_with_palette;
use crate::tes::actor::actor_is_dead;
use crate::tes::actor_process::{process_evaluate_package, process_run_scheduled};
use crate::tes::ai_cell_moves::set_3d_null;
use crate::tes::ai_conditions::ScriptArms;
use crate::tes::ai_package::{package_data, script_event_or};
use crate::tes::base_extra_list::{EXTRA_CONTAINER_CHANGES, EXTRA_LEVELED_ITEM, ExtraDataList};
use crate::tes::data_load::DataLoadServices;
use crate::tes::form::OriginalVtable;
use crate::tes::form_arms::DataLoadHost;
use crate::tes::form_arms::ScriptRandom;
use crate::tes::forms_actors::{auto_equip, creature_auto_equip};
use crate::tes::forms_init::form_type;
use crate::tes::forms_items::set_change_flags;
use crate::tes::forms_references::{NoRandom, save_load_active};
use crate::tes::global::game_hour;
use crate::tes::player::the_player;

// IDA: the form types the equip code casts to: TESObjectARMO, BOOK, TESObjectCLOT, TESObjectLIGH, TESObjectWEAP, TESAmmo, TESNPC, TESCreature.
pub(crate) const TYPE_ARMOR: u8 = 0x14;
pub(crate) const TYPE_CLOTHING: u8 = 0x16;
pub(crate) const TYPE_LIGHT: u8 = 0x1A;
pub(crate) const TYPE_WEAPON: u8 = 0x21;
const TYPE_NPC: u8 = 0x23;
pub(crate) const TYPE_CREATURE: u8 = 0x24;

// IDA: 00A350F8, ExtraDataList's vtable (00422EE0).
pub(super) const EXTRA_DATA_LIST_VTABLE: u32 = 0x00A3_50F8;

// IDA: 00A30634, -1.0.
pub(super) const MINUS_ONE: f32 = -1.0;

// IDA: N/A the load's seams as a script command's call of 00488420 sees them: 0047DF80 through the host's draw (`ScriptRandom`'s second), and warnings to the host log.
struct ScriptLoadHost(Option<Rc<dyn Fn() -> i32>>);

impl DataLoadHost for ScriptLoadHost {
    /// IDA: N/A `ScriptLoadHost`'s side of `DataLoadHost`.
    fn random_next(&mut self) -> Option<i32> {
        self.0.as_ref().map(|draw| draw())
    }
}

/// IDA: 004DD850's arms for an actor between 0046ABA0(0) and the cell test: vt+0x184 005E5EC0(1), then with a null +0x58 006438F0 LowProcess_ctor (FormHeap 0x90) and 00673A90(actor, 3, 0, 0, 0). Completion: 100% of the arms a disabled NPC with no process, or with a LowProcess above level 0, reaches; the rest are stops.
/// # Errors
/// A stop in 005E5EC0, or a null g_saveLoadGame.
pub(in crate::tes) fn enable_actor_reset(
    world: &mut World,
    actor: Entity,
) -> Result<(), GameError> {
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

/// IDA: 004DD850's arms for an actor outside an attached cell, after the cell test: vt+0x1C4 005F23E0, then with a process vt+0x14(actor, 1) (0064A380, `process_run_scheduled`) and 00601B80, and with a level above 0 vt+0x1C0(0.0) 00605770 Actor_Update. 005F23E0 (`actor_process_moves::actor_sync_process_level`) moves a process whose level isn't 0065AD00's, and +0x58 is read again after it. Completion: 100%, with the stops of each.
/// # Errors
/// A stop in 005F23E0, 0064A380, 00601B80 or 00605770.
pub(in crate::tes) fn enable_actor_update(
    world: &mut World,
    actor: Entity,
) -> Result<(), GameError> {
    if world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58
        .is_none()
    {
        return Ok(());
    }
    // 004DDA4E vt+0x1C4 005F23E0; 004DDA68 +0x58 read again.
    super::actor_process_moves::actor_sync_process_level(world, actor)?;
    let Some(process) = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58
    else {
        return Ok(());
    };
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
pub(in crate::tes) fn disable_actor_process(
    world: &mut World,
    actor: Entity,
) -> Result<(), GameError> {
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
        return Err(GameError::unclassified(
            0x004E4701,
            "004E4701: the player's +0x1E0",
        ));
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
        crate::tes::ai_package::package_is_combat_like(package_data(world, package))
    }) {
        return Err(GameError::unclassified(
            0x004E476F,
            "004E476F: 005EAE70 for a combat package",
        ));
    }
    delete_low_process(world, process)?;
    world
        .get_mut::<MobileObject>(actor)
        .expect("an actor")
        .process_58 = None;
    Ok(())
}

// IDA: N/A the current package, +0x08.
pub(super) fn current_package(world: &World, process: Entity) -> Option<Entity> {
    world
        .get::<BaseProcess>(process)
        .expect("a process")
        .editor_package_08
}

// IDA: 00601B80(actor) on a LowProcess (vtable A720A4, whose slots name each call). vt+0x184 (00452A60, +0x08) of type 0x12 returns, as does Actor_IsDead(1). A current package of type 0x17 or 0x1E returns; 0x18 asks the actor's vt+0x388; any other takes vt+0xB8 and the door callbacks, a stop. vt+0x248 (0051AAD0, +0x20) set is a stop. 005EAE70 with no package: vt+0x4C8(4) (004943A0, 0), vt+0x4A8 and vt+0x480 do nothing, so 006116D0 only stores 1.0 at the actor's +0x100 unless a save is active; vt+0x174 is 0, vt+0x178 and vt+0x194 do nothing, vt+0x36C is 0, never 9. Then vt+0x3C0 (006479C0), vt+0x394(0), vt+0x18(actor, 1) (00649340), and 004FBF90(null, +0x44, 0x800) when a package came. With no package before, no re-equip. vt+0x194 does nothing; vt+0x18C's 005E0EC0 asks vt+0x36C, 0. A nonzero level stores the game hour less 1.0 (vt+0x1C 00629240, a float store). A target's +0x08 goes to +0x38 (vt+0x100 006292C0). Completion: 100% for a LowProcess with no package before.
pub(in crate::tes) fn reevaluate_package(
    world: &mut World,
    actor: Entity,
) -> Result<(), GameError> {
    let process = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58
        .ok_or(GameError::unclassified(0x00601B86, "00601B86: no process"))?;
    if world.get::<OriginalVtable>(process).map(|vtable| vtable.0) != Some(LOW_PROCESS_VTABLE) {
        return Err(GameError::unclassified(
            0x00601B91,
            "00601B91: a process other than a LowProcess",
        ));
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
            0x18 => {
                GameError::unclassified(0x00601BF8, "00601BF8: vt+0x388 for a type 0x18 package")
            }
            _ => GameError::unclassified(0x00601C19, "00601C19: the current package's vt+0xB8"),
        });
    }
    if world
        .get::<LowProcess>(process)
        .expect("a LowProcess")
        .byte_20
        != 0
    {
        return Err(GameError::unclassified(
            0x00601CDB,
            "00601CDB: vt+0xCC's follower list",
        ));
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
fn clear_package_state(world: &mut World, process: Entity) -> Result<(), GameError> {
    let mut low = world.get_mut::<LowProcess>(process).expect("a LowProcess");
    low.byte_1d = 0;
    low.byte_1c = 0;
    low.field_2c = None;
    low.field_30 = None;
    if low.block_44 != 0 || low.block_48 != 0 || low.list_3c != [0; 2] || low.list_4c[1] != 0 {
        return Err(GameError::unclassified(
            0x006479EA,
            "006479EA: a LowProcess's package lists",
        ));
    }
    low.list_4c[0] = 0;
    Ok(())
}

// IDA: 005E5EC0 Character vt+0x184(1). A process of level 0 takes vt+0x31C(1) unless a save is active, a stop; otherwise Character_Set3D(null) (0060E430): with no 3D, 004E0F80's 00439DC0 (`set_3d_null`). Then 004DDB00(1). The base: TESNPC 005227A0(base, actor, a, b, 0, 1), TESCreature 0051E240(actor, a, b, 1), where `a` and `b` are 1 unless the process's current package (vt+0x184, 00452A60's +0x08) has flag 0x100000 or 0x200000, a stop. Completion: 100% for no process or a LowProcess above level 0.
fn reset_actor(world: &mut World, actor: Entity) -> Result<(), GameError> {
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
            return Err(GameError::unclassified(
                0x005E5EF2,
                "005E5EF2: a level 0 process's vt+0x31C",
            ));
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
        .ok_or(GameError::unclassified(
            0x005E5F22,
            "005E5F22: an actor with no base",
        ))?;
    if let Some(process) = process
        && current_package(world, process)
            .is_some_and(|package| package_data(world, package).flags_1c & 0x30_0000 != 0)
    {
        return Err(GameError::unclassified(
            0x005E5F65,
            "005E5F65: a package that keeps the outfit",
        ));
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
) -> Result<(), GameError> {
    if world.get::<OriginalVtable>(process).map(|vtable| vtable.0) == Some(LOW_PROCESS_VTABLE) {
        Ok(())
    } else {
        Err(GameError::from_legacy_message(stop))
    }
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
fn reset_inventory(world: &mut World, actor: Entity) -> Result<(), GameError> {
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
        return Err(GameError::unclassified(
            0x004DDBB5,
            "004DDBB5: the 0x42 list's references",
        ));
    }
    Ok(())
}

// IDA: N/A 0041E7B0 BaseExtraList_GetOwner (`base_extra_list::extra_form` of the 0x27 extra), with the equipment
// callers' own text for an unresolved form.
pub(super) fn list_owner(list: &ExtraDataList) -> Result<Option<Entity>, GameError> {
    crate::tes::base_extra_list::extra_form(list, crate::tes::base_extra_list::EXTRA_OWNER).map_err(
        |_| {
            GameError::unclassified(
                0x0041E7B0,
                "0041E7B0: an ownership extra whose form isn't resolved",
            )
        },
    )
}

// IDA: 00480675 / 004805C9 a type-4 property is eligible when its shader type is 5 through 10.
// Completion: 100% for the bound shader property classes.
fn skin_shade_property(world: &World, object: Entity) -> Result<Option<Entity>, GameError> {
    use crate::bs::sky_shader::shader_property_id;
    use crate::gamebryo::av_object::find_property_of_type;
    let Some(property) = find_property_of_type(world, object, 4)? else {
        return Ok(None);
    };
    Ok((5..=10)
        .contains(&shader_property_id(world, property)?)
        .then_some(property))
}

// IDA: 00480560 NiNode_FindSkinShadePropertyRecursive(node)
// Completion: 100% of the ordered child walk. Only the geometry arm tests a leaf.
fn find_skin_shade_property_recursive(
    world: &World,
    node: Entity,
) -> Result<Option<Entity>, GameError> {
    use crate::gamebryo::scene_node::NiNodeChildren;
    for &child in world
        .get::<NiNodeChildren>(node)
        .expect("a NiNode")
        .slots_ac
        .iter()
        .flatten()
    {
        let found = if world.get::<NiNodeChildren>(child).is_some() {
            find_skin_shade_property_recursive(world, child)?
        } else if crate::bs::sky_shader::is_geometry(world, child) {
            skin_shade_property(world, child)?
        } else {
            None
        };
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}

// IDA: 00480630 NiNode_FindSkinShadeProperty(node)
// Completion: 100%. The reverse walk tests slot zero of each child node, including holes.
fn find_skin_shade_property(world: &World, node: Entity) -> Result<Option<Entity>, GameError> {
    use crate::gamebryo::scene_node::NiNodeChildren;
    for &child in world
        .get::<NiNodeChildren>(node)
        .expect("a NiNode")
        .slots_ac
        .iter()
        .rev()
        .flatten()
    {
        if let Some(first) = world
            .get::<NiNodeChildren>(child)
            .and_then(|children| children.slots_ac.first().copied().flatten())
            && let Some(property) = skin_shade_property(world, first)?
        {
            return Ok(Some(property));
        }
    }
    find_skin_shade_property_recursive(world, node)
}

// IDA: 005EA1A0 Actor_UpdateSkinProperty(actor, node), then 006349D0 MiddleHighProcess_SetSkinProperty.
// Completion: the palette, named child and root paths and the process store. ECS owns shared property lifetimes.
pub(crate) fn find_skin_property(
    world: &mut World,
    process: Entity,
    node: Entity,
) -> Result<(), GameError> {
    use crate::gamebryo::{object_net::NiObjectNet, scene_node::NiNodeChildren};
    let Some(children) = world.get::<NiNodeChildren>(node) else {
        return Ok(());
    };
    let attachment = object_by_name_with_palette(world, node, b"SkinAttachment")
        .or_else(|| {
            children.slots_ac.iter().flatten().copied().find(|&child| {
                world
                    .get::<NiObjectNet>(child)
                    .and_then(|net| net.name.as_deref())
                    .is_some_and(|name| oblivion_core::crt::strnicmp(name, b"BIP", 3) != 0)
                    && world.get::<NiNodeChildren>(child).is_some()
            })
        })
        .unwrap_or(node);
    let property = find_skin_shade_property(world, attachment)?;
    if let Some(mut middle) = world.get_mut::<MiddleHighProcess>(process)
        && middle.skin_property_184 != property
    {
        middle.skin_property_184 = property;
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
pub(super) fn unequip_item_unstacked(
    world: &mut World,
    actor: Entity,
    form: Entity,
) -> Result<(), GameError> {
    let kind = form_type(world, form);
    if !matches!(kind, TYPE_ARMOR | TYPE_CLOTHING) {
        return Err(GameError::unclassified(
            0x005F2EC0,
            "005F2EC0: unequipping a light, weapon or ammo",
        ));
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
            return Err(GameError::unclassified(
                0x006A2110,
                "006A2110: an active effect from the unequipped item",
            ));
        }
        if data.pending_enchanted_equipment_1fc != 0 {
            return Err(GameError::unclassified(
                0x00662C8E,
                "00662C8E: the player's pending equipment enchantments",
            ));
        }
        let top = world
            .get_resource::<crate::tes::interface_manager::InterfaceManager>()
            .map(|manager| {
                crate::tes::interface_frame::top_stacked_menu_mode(&manager.menu_mode_stack_0e0)
            });
        if top != Some(1007) {
            return Err(GameError::unclassified(
                0x005E99D8,
                "005E99D8: the unequip sound outside the LoadingMenu",
            ));
        }
    } else {
        // 006A2110 MagicTarget_RemoveEffectsFromItem(form, 1) over the process's list: an unterminated effect whose
        // +0x30 is the item dispels. The port's +0x30 holds 0068EA50's argument, nonzero only for an item's effect.
        let process = world
            .get::<MobileObject>(actor)
            .and_then(|mobile| mobile.process_58);
        let from_item = process
            .and_then(|process| world.get::<super::magic_cast::ActiveEffectList>(process))
            .is_some_and(|list| {
                list.0.iter().any(|&effect| {
                    world
                        .get::<super::magic_cast::ActiveEffect>(effect)
                        .is_some_and(|effect| effect.terminated_11 == 0 && effect.field_30 != 0)
                })
            });
        if from_item {
            return Err(GameError::unclassified(
                0x006A2150,
                "006A2150: dispelling an active effect from the unequipped item",
            ));
        }
    }
    let changes = world
        .get::<TesObjectRefr>(actor)
        .expect("a reference")
        .extra_data_44
        .get(EXTRA_CONTAINER_CHANGES)
        .is_some();
    if changes {
        // 005E4260: with no save active, 004FBF90(actor, null, 8) changes nothing and 004FBF90(form, the actor's list, 8)
        // is OnUnequip on the actor's script.
        if !crate::tes::forms_references::save_load_active(world)? {
            script_event_or(world, actor, 8, Some(form));
        }
        let mut held = ensure_container_changes(world, actor)?;
        let result = changes_unequip_item(world, &mut held, form, actor);
        put_container_changes(world, actor, held);
        if result? {
            update_skin_property_after_unequip(world, actor)?;
        }
    }
    if enchantment_of(world, form).is_some() {
        return Err(GameError::unclassified(
            0x005E42DE,
            "005E42DE: releasing the item's enchantment (0041A7E0)",
        ));
    }
    Ok(())
}
