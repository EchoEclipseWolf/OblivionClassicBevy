//! 004D5BD0's reference loop (004D5C2E..004D5D3F) for a cell attached while the game runs, and the actor arm it takes: 005FA320 Actor_RebuildHighProcess and 00674E10.
//! Evidence: `evidence/gameplay-run002-2026-11-07/`: run-ab recorded the loop on the Imperial Prison (`1FBB9`), run-ac every call and return inside 005FA320 for its 11 actors, run-ad those inside 005EB400, 005EB370, 005E2E20 and the path grid's 004E5730.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::actor::{Actor, MobileObject, TesObjectRefr};
use super::form::FormIdentity;
use super::form_components::FormRef;
use super::forms_world::TesObjectCell;

/// IDA: N/A what the loop reaches that the port doesn't run: 00438060 ModelLoader_QueueReference with the cell's priority, the IO manager's queue, which the host's drain doesn't fill yet.
pub type QueueReferenceCall<'f> = dyn FnMut(&mut World, Entity, i32) + 'f;

/// IDA: 004D5C2E..004D5D3F, 004D5BD0's loop over the cell's +0x48 list under 00496EA0's region lock (the host has no other thread in the cell's list). The walk reads each node's next before its body, so the list is taken first. Per reference: unless a save is loading, 004F9EC0 and 004FBF90(+0x44, 0x1000); flag 0x800 or 0x20 skips the rest; an actor, unless a save is loading without +0x18 bit 4, gets vt+0x1A4 (005FA320) and 00674E10; then 00438060 with `priority`, and vt+0x154 non-null runs 004CEAD0, a stop here. Completion: 100% of the order; 00438060 is the caller's.
/// TODO: [ ] 004CEAD0, for a reference whose 3D is already loaded.
/// # Errors
/// A stop in the actor arm or in a script.
pub fn finish_attach_reference_loop(
    world: &mut World,
    cell: Entity,
    priority: i32,
    queue: &mut QueueReferenceCall<'_>,
) -> Result<(), StreamError> {
    let references: Vec<Entity> = world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .items()
        .collect();
    for reference in references {
        let loading = super::forms_references::save_load_active(world)?;
        if !loading {
            // 004D5C7A 004F9EC0; 004D5C86 004FBF90(reference, +0x44, 0x1000).
            super::ai_process::run_pending_script(world, reference)?;
            super::ai_package::script_event_or(world, reference, 0x1000, None);
        }
        let flags = world
            .get::<FormIdentity>(reference)
            .expect("a form")
            .flags_08;
        if flags & 0x800 != 0 || flags & 0x20 != 0 {
            continue;
        }
        // 004D5CAA vt+0x190 IsActor; 004D5CBA a save loading needs +0x18 bit 4.
        if world.get::<Actor>(reference).is_some() {
            if loading {
                return Err(StreamError("004D5CC9: the actor arm while a save loads"));
            }
            // 004D5CF7 vt+0x1A4; 004D5D09 00674E10.
            actor_rebuild_high_process(world, reference)?;
            process_manager_drop_queued(world, reference);
        }
        queue(world, reference, priority);
        if world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .loaded_3d_3c
            != 0
        {
            return Err(StreamError("004D5D32: 004CEAD0 on a loaded reference"));
        }
    }
    Ok(())
}

/// IDA: 00674E10 on g_actorProcessManager (thiscall): 00B3BCF8, 00674A20's current entry, is cleared when it is `actor` (nothing in the port sets it), then 004466B0 removes the first match from +0x58. Completion: 100%.
pub fn process_manager_drop_queued(world: &mut World, actor: Entity) {
    world
        .resource_mut::<super::actor_process::ActorProcessGlobals>()
        .manager
        .queued_58
        .remove_first_matching(actor);
}

/// IDA: 005FA320 Actor_RebuildHighProcess, vt+0x1A4, for an actor with a LowProcess or none, as run-ac shows the prison's 11. A process at level 0 returns at once (005FAA4C). The old process's vt+0x18 (00649340, the package evaluation, forced) unless a save loads; vt+0x334 005E6110(1); the old package's target reference; 005E6CD0(0); the level, 3 with no process, and 00674550 removes the actor from its list. A rotation x of FLT_MAX is set to 0 (004D84E0) and goes to 005EB400; so does an actor at the origin, or one at level 2 or 3 with no save loading; 005EB400 false, or any other actor, takes 005EB370. FormHeap(0x2EC) 00628EE0 (its draw), vt+4 0060CDB0 from the old process, the old one's vt+0(1), +0x58 = the new; vt+0x210 (+0x1F) true would run vt+0x300(1). 00673A90(actor, 0, 0, 0, 0); 00634CB0; vt+0x5C. With container changes and a level other than 1 the equipment block (`rebuild_equipment`). vt+0x4C; 005E6E00; the combat arm (005EAE70) with vt+0x334's answer and no combat controller; the death arm; vt+0x178(0); the sleeping arm needs a node; with +0xC0 the death animation arm, a stop; then 005E7230(null, 0); a current package of type 6 gets vt+0x17C(0); a living actor at level 2 or 3 gets 005EDA20(0); vt+0x80's arm; and vt+0x154's lighting entry. Completion: the arms run-ac shows, in its order; the rest stop.
/// # Errors
/// A stop.
#[allow(
    clippy::too_many_lines,
    reason = "one function in the exe, in its order"
)]
pub fn actor_rebuild_high_process(world: &mut World, actor: Entity) -> Result<(), StreamError> {
    use super::actor_process::{BaseProcess, LowProcess, MiddleHighProcess, process_level};
    let old = world.get::<MobileObject>(actor).and_then(|m| m.process_58);
    if old.is_some_and(|p| process_level(world, p) == 0) {
        return Ok(());
    }
    let loading = super::forms_references::save_load_active(world)?;
    // 005FA36C..005FA386 old vt+0x18(actor, 1): 00649340 on a LowProcess.
    if let Some(old) = old
        && !loading
    {
        super::ai_process::process_evaluate_package(
            world,
            old,
            actor,
            true,
            &mut super::ai_conditions::ScriptArms,
        )?;
    }
    // 005FA394 vt+0x334(1) 005E6110: kept in a byte for 005FA606.
    let combat = actor_in_combat_package(world, actor, true)?;
    // 005FA39A..005FA3B8 the old process's package target's reference (00569E60), for 005EAE70's arm.
    let current = world.get::<MobileObject>(actor).and_then(|m| m.process_58);
    let _target = current
        .and_then(|p| {
            world
                .get::<BaseProcess>(p)
                .and_then(|b| b.editor_package_08)
        })
        .and_then(|package| super::ai_package::package_data(world, package).target_28)
        .and_then(|target| match (target.type_00, target.value_04) {
            (0, super::form_components::FormRef::Resolved(reference)) => reference,
            _ => None,
        });
    // 005FA3C2 005E6CD0(0): its answer goes unused.
    actor_takes_package_procedure(world, actor, false)?;
    // 005FA3D0..005FA3F7 the level, and 00674550.
    let mut level = 3;
    if let Some(old) = current {
        level = process_level(world, old);
        super::actor_process::actor_process_manager_remove(world, actor, level, loading);
    }
    // 005FA3FB..005FA476 the placement.
    let data = world.get::<TesObjectRefr>(actor).expect("an actor");
    let (rotation_x, position) = (data.rotation_20[0], data.position_2c);
    let try_path_grid = if rotation_x == f32::MAX {
        return Err(StreamError(
            "005FA413: 004D84E0 for a rotation x of FLT_MAX",
        ));
    } else {
        position == [0.0; 3] || (!loading && (2..=3).contains(&level))
    };
    if !(try_path_grid && super::actor_placement::actor_place_on_path_grid(world, actor)?) {
        super::actor_placement::actor_place_if_blocked(world, actor)?;
    }
    // 005FA47B..005FA4C7 the HighProcess (its draw), 0060CDB0 from the old one, which is deleted.
    let random = world
        .get_non_send::<super::ai_conditions::ScriptRandom>()
        .ok_or(StreamError("0062901F: Random_Next with no host CRT"))?
        .0
        .clone();
    let drawn = random(world);
    let timer = world
        .get_resource::<super::game_settings::GameSettingCollection>()
        .and_then(|settings| settings.raw_at(0x00B3_6940))
        .ok_or(StreamError("00629106: fIdleChatterCommentTimer"))?;
    let mut source = DrawnRandom(Some(drawn));
    let high = super::actor_process::construct_high_process(
        world,
        &mut super::actor_process::ProcessInputs {
            random: &mut source,
            idle_chatter_comment_timer: timer,
            // N/A FormHeap's unset bytes: 0060CDB0 copies the old process's over them; with none they stay unknown.
            low_process_unset_88: 0,
            save_load_active: loading,
        },
    );
    super::actor_process::copy_process_from(world, high, current)?;
    if let Some(old) = current {
        super::actor_process::delete_low_process(world, old)?;
    }
    world
        .get_mut::<MobileObject>(actor)
        .expect("an actor")
        .process_58 = Some(high);
    if world.get::<LowProcess>(high).expect("a process").byte_1f != 0 {
        return Err(StreamError("005FA4EB: vt+0x300(1) on a HighProcess"));
    }
    // 005FA4FB..005FA50F 00673A90 at level 0; 00634CB0 (+0x1EC, then the node's Bip01, which needs a node); vt+0x5C 00629540 (+0x15C = 0).
    super::actor_process::actor_process_manager_add(world, actor, 0, false, false)?;
    world
        .get_mut::<super::actor_process::HighProcess>(high)
        .expect("a process")
        .field_1ec = 0;
    if world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .loaded_3d_3c
        != 0
    {
        return Err(StreamError("005FA503: 00634CB0's Bip01 with a node"));
    }
    world
        .get_mut::<MiddleHighProcess>(high)
        .expect("a process")
        .field_15c = 0;
    // 005FA514 the equipment block.
    if world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .extra_data_44
        .get(super::base_extra_list::EXTRA_CONTAINER_CHANGES)
        .is_some()
        && level != 1
    {
        rebuild_equipment(world, actor, high)?;
    }
    // 005FA5FD vt+0x4C 00628600: with +0x2C, its position (vt+0x174) to +0xD4.
    if world
        .get::<LowProcess>(high)
        .expect("a process")
        .field_2c
        .is_some()
    {
        return Err(StreamError("005FA5FD: 00628600 with a +0x2C reference"));
    }
    // 005FA601 005E6E00.
    super::ai_process::package_arrival(world, actor)?;
    // 005FA606..005FA65D the combat arm.
    if combat {
        return Err(StreamError(
            "005FA61F: 005EAE70 for an actor in a combat package",
        ));
    }
    // 005FA663..005FA6DC the death arm: a creature with a node asks vt+0x278 (true for every class here).
    let deferred = world
        .get::<Actor>(actor)
        .expect("an actor")
        .deferred_death_processing_c0;
    if deferred != 0
        || (super::ai_package::actor_is_dead(world, actor, false)
            && !has_extra(world, actor, super::base_extra_list::EXTRA_RAG_DOLL)
            && !has_extra(world, actor, 0x4B))
    {
        return Err(StreamError("005FA6D0: 005E6680(0) for a dead actor"));
    }
    // 005FA6E8 vt+0x178(0) 0065B5C0: no controller returns.
    if super::character_controller::actor_character_controller(world, actor).is_some() {
        return Err(StreamError(
            "005FA6E8: 0065B5C0 with a character controller",
        ));
    }
    // 005FA6EE..005FA93C: the sleeping arm and 005EE1B0 need a node; +0xC0 is 0.
    // 005FA941 005E7230(null, 0).
    refresh_equipped_visuals(world, actor)?;
    // 005FA947..005FA96F a current package of type 6 gets vt+0x17C(0).
    let package = world
        .get::<BaseProcess>(high)
        .and_then(|b| b.editor_package_08);
    if package.is_some_and(|package| super::ai_package::package_data(world, package).type_20 == 6) {
        set_procedure_index(world, high, 0)?;
    }
    // 005FA975..005FA991 a living actor at level 2 or 3: 005EDA20(0).
    if !super::ai_package::actor_is_dead(world, actor, false) && (level == 3 || level == 2) {
        apply_base_spells(world, actor)?;
    }
    // 005FA996..005FAA1C vt+0x80 (00651AA0's +0x20 byte here, 0051AAD0).
    if world.get::<LowProcess>(high).expect("a process").byte_20 != 0 {
        return Err(StreamError("005FA9A8: vt+0x80's follower arm"));
    }
    // 005FAA28 vt+0x154 null: no lighting entry.
    Ok(())
}

/// IDA: N/A a 0047DF80 draw made just before 00628EE0 runs, handed to it: nothing between them draws, and the first-use flag is already clear by the time a game runs.
struct DrawnRandom(Option<i32>);

impl super::actor_process::RandomSource for DrawnRandom {
    /// IDA: N/A `DrawnRandom`'s side of `RandomSource`.
    fn time64(&mut self) -> i64 {
        0
    }
    /// IDA: N/A `DrawnRandom`'s side of `RandomSource`.
    fn srand(&mut self, _seed: u32) {}
    /// IDA: N/A `DrawnRandom`'s side of `RandomSource`.
    fn rand(&mut self) -> i32 {
        self.0.take().expect("one draw for 00628EE0")
    }
}

// IDA: N/A whether the reference's extra list holds an entry of the type with its data (0041FFA0's +0x0C, 004212C0's +0x10, each set by its loader).
fn has_extra(world: &World, actor: Entity, extra_type: u8) -> bool {
    world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .extra_data_44
        .get(extra_type)
        .is_some()
}

/// IDA: 005E6110(require_rider_or_not_sleeping): with a process, when vt+0x380 (Character_GetMountedHorse) is set, or the argument is 0, or the process's vt+0x36C (sleep state; a LowProcess's 0) is 0, the process's +0x08 package of type 12 or 13 answers true. Completion: 100% of a LowProcess or none.
fn actor_in_combat_package(
    world: &World,
    actor: Entity,
    require: bool,
) -> Result<bool, StreamError> {
    let Some(process) = world.get::<MobileObject>(actor).and_then(|m| m.process_58) else {
        return Ok(false);
    };
    if world
        .get::<super::actor_process::MiddleHighProcess>(process)
        .is_some()
    {
        return Err(StreamError("005E6129: 005E6110 on a MiddleHigh process"));
    }
    let _ = require;
    let package = world
        .get::<super::actor_process::BaseProcess>(process)
        .and_then(|b| b.editor_package_08);
    Ok(package.is_some_and(|package| {
        matches!(
            super::ai_package::package_data(world, package).type_20,
            12 | 13
        )
    }))
}

/// IDA: 005E6CD0(keep): a current package (vt+0x184; a LowProcess's +0x08) of type 16 answers true for anyone but the player, whose +0x08 is released; otherwise with `keep` 0 the +0x08 package's type 16, 10, or 13 with procedure 1 answers true. Completion: 100% of a LowProcess or none; the player's arm stops.
fn actor_takes_package_procedure(
    world: &World,
    actor: Entity,
    keep: bool,
) -> Result<bool, StreamError> {
    let Some(process) = world.get::<MobileObject>(actor).and_then(|m| m.process_58) else {
        return Ok(false);
    };
    let base = *world
        .get::<super::actor_process::BaseProcess>(process)
        .expect("a process");
    let Some(package) = base.editor_package_08 else {
        return Ok(false);
    };
    let package_type = super::ai_package::package_data(world, package).type_20;
    if package_type == 16 {
        if Some(actor) == super::ai_conditions::the_player(world) {
            return Err(StreamError("005E6D02: the player's type 16 package"));
        }
        return Ok(true);
    }
    if keep {
        return Ok(false);
    }
    Ok(package_type == 10 || (package_type == 13 && base.edit_pack_procedure_type_04 == 1))
}

/// IDA: 005FA52C..005FA5F1, the equipment block: 00486790 for the weapon (9), quiver (12), shield (13) and torch (14) slots into vt+0x104 (00658550), vt+0x10C (0064AED0, +0xEC), vt+0x110 (0064AF10, +0xF0) and vt+0x108 (0064AE90, +0xE8); the weapon again for 00612A90 into vt+0x328 (+0xF8); vt+0xF0(1). Completion: 100% with nothing worn in those slots; a worn item stops.
/// TODO: [ ] 00486790 with a worn item, and the setters that keep it.
fn rebuild_equipment(world: &mut World, actor: Entity, process: Entity) -> Result<(), StreamError> {
    for slot in [9, 12, 13, 14, 9] {
        if find_equipped_in_slot(world, actor, slot)? {
            return Err(StreamError(
                "005FA539: an item worn in a rebuilt actor's slot",
            ));
        }
    }
    // 00658550(null, 0): +0xE4 is already null; +0x17C null clears +0xFC..+0x110; the player's arm clears 00B3BA84..00B3BA98.
    let mut middle = world
        .get_mut::<super::actor_process::MiddleHighProcess>(process)
        .expect("a process");
    if middle.weapon_e4.is_some() || middle.anim_data_17c.is_some() {
        return Err(StreamError(
            "005FA548: 00658550 on a process with a weapon or animation data",
        ));
    }
    middle.weapon_nodes_fc = super::actor_process::WeaponNodes::default();
    // 0064AED0, 0064AF10, 0064AE90 store null over null; 00612A90(null) is 0.0 into +0xF8.
    middle.ammo_ec = None;
    middle.shield_f0 = None;
    middle.light_e8 = None;
    middle.float_f8 = 0.0;
    Ok(())
}

// IDA: 00486790 as a yes or no.
fn find_equipped_in_slot(world: &World, actor: Entity, slot: u32) -> Result<bool, StreamError> {
    Ok(equipped_in_slot(world, actor, slot)?.is_some())
}

/// IDA: 00486790 ExtraContainerChangesData_FindEquippedInBipedSlot(slot, 0): per entry, a stack with 0x36 on an entry of negative count runs 004853B0, a stop; then the first stack with 0x1B or 0x1C (0041DF10(0)) whose form fits the slot: 9 a weapon (0x21), 12 an ammo (0x22), 14 a light (0x1A), 10 and 11 nothing, 6 and 7 a ring's biped (slot 6 or 7) worn on the right (6, no 0x1C) or the left (7, 0x1C), the rest the biped's mask. Answers the entry's form and whether that stack holds 0x1C (InventoryEntry_IsWorn(selection, 1)); no changes block answers none. Completion: 100% but 004853B0.
/// # Errors
/// 004853B0's arm.
pub(crate) fn equipped_in_slot(
    world: &World,
    actor: Entity,
    slot: u32,
) -> Result<Option<(Entity, bool)>, StreamError> {
    use super::base_extra_list::{EXTRA_LEVELED_ITEM, EXTRA_WORN, EXTRA_WORN_LEFT};
    let Some(super::base_extra_list::ExtraData::ContainerChanges(changes)) = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .extra_data_44
        .get(super::base_extra_list::EXTRA_CONTAINER_CHANGES)
        .map(|entry| &entry.data)
    else {
        return Ok(None);
    };
    for entry in &changes.entries_00 {
        let Some(lists) = &entry.lists_00 else {
            continue;
        };
        if entry.count_04 < 0
            && lists
                .iter()
                .any(|list| list.get(EXTRA_LEVELED_ITEM).is_some())
        {
            return Err(StreamError("004867F0: 004853B0 for a leveled stack"));
        }
        for list in lists {
            if list.get(EXTRA_WORN).is_none() && list.get(EXTRA_WORN_LEFT).is_none() {
                continue;
            }
            let left = list.get(EXTRA_WORN_LEFT).is_some();
            let form_type = super::forms_init::form_type(world, entry.form_08);
            let fits = match slot {
                6 | 7 => {
                    (super::actor_enable::biped_covers(world, entry.form_08, 7)
                        || super::actor_enable::biped_covers(world, entry.form_08, 6))
                        && (slot == 7) == left
                }
                9 => form_type == 0x21,
                10 | 11 => false,
                12 => form_type == 0x22,
                14 => form_type == 0x1A,
                _ => super::actor_enable::biped_covers(world, entry.form_08, slot),
            };
            if fits {
                return Ok(Some((entry.form_08, left)));
            }
        }
    }
    Ok(None)
}

/// IDA: 005E7230 PlayerCharacter_RefreshEquippedItemVisuals(null, 0): no process, or DataHandler +0xCD4, returns. With the process's light (+0xE8), shield (+0xF0) and weapon (+0xE4) all null, every arm reaches 005E7A06: the shield and torch lookups (00486790) are freed, and with no armor change vt+0x... isn't called. The menu-mode and sleep reads change nothing. Completion: 100% with nothing equipped; an equipped item stops.
/// TODO: [ ] 005E7230's arms with a weapon, a shield or a light.
fn refresh_equipped_visuals(world: &mut World, actor: Entity) -> Result<(), StreamError> {
    let Some(process) = world.get::<MobileObject>(actor).and_then(|m| m.process_58) else {
        return Ok(());
    };
    let Some(middle) = world.get::<super::actor_process::MiddleHighProcess>(process) else {
        return Err(StreamError("005E7262: 005E7230 on a Low process"));
    };
    if middle.weapon_e4.is_some() || middle.light_e8.is_some() || middle.shield_f0.is_some() {
        return Err(StreamError(
            "005E72C4: 005E7230 with an equipped weapon, shield or light",
        ));
    }
    Ok(())
}

/// IDA: 005E7230 PlayerCharacter_RefreshEquippedItemVisuals(changed, 0) for a WEAP or an ARMO covering slot 13, after
/// 005F3140's equip: no process, or DataHandler +0xCD4, returns. The process's light (+0xE8), shield (+0xF0) and weapon (+0xE4)
/// at vt+0xF0/0xF8/0xFC(0); the menu test is vt+0x304 (+0x115), vt+0x2FC (+0x114), then InterfaceManager_IsMenuMode; a weapon of
/// type 1, 3, 4 or 5 takes both hands. A WEAP: with both hands, a worn shield unequips (004853B0, 004DC8F0) and a worn light
/// with menu mode too, stops; otherwise with menu mode or no light: no weapon, or a light outside menu mode, ends; an unworn
/// shield of a living actor is shown (004E1B40), a stop; no light ends; the rest are stops. An ARMO in slot 13: the light arms
/// are stops; then a one-hand weapon, or no shield, ends; otherwise the shield unequips (004853B0, 004DC8F0(0)), a stop. The
/// shield and torch lookups (00486790) are only freed, and with no unequip the armor rating isn't invalidated. Completion: 100%
/// of the arms with no light; the unequips and the shield's 3D are stops.
pub(crate) fn refresh_equipped_visuals_for(
    world: &mut World,
    actor: Entity,
    changed: Entity,
) -> Result<(), StreamError> {
    let Some(process) = world.get::<MobileObject>(actor).and_then(|m| m.process_58) else {
        return Ok(());
    };
    if world
        .get_resource::<super::data_handler::DataHandler>()
        .is_some_and(|data| data.bytes_cd0[4] != 0)
    {
        return Ok(());
    }
    let Some(middle) = world.get::<super::actor_process::MiddleHighProcess>(process) else {
        return Err(StreamError("005E7262: 005E7230 on a Low process"));
    };
    let (light, shield, weapon) = (middle.light_e8, middle.shield_f0, middle.weapon_e4);
    let menu =
        middle.byte_115 != 0 || middle.byte_114 != 0 || super::script_vm::is_menu_mode(world);
    let both_hands = weapon.is_some_and(|weapon| {
        matches!(
            super::actor_enable::weapon_type_of(world, weapon.form_08),
            1 | 3 | 4 | 5
        )
    });
    let dead = super::ai_package::actor_is_dead(world, actor, false);
    match super::forms_init::form_type(world, changed) {
        0x21 => {
            if both_hands {
                if shield.is_some_and(|shield| shield.worn) {
                    return Err(StreamError(
                        "005E7AA7: a two-hand weapon unequips the worn shield",
                    ));
                }
                if light.is_some_and(|light| light.worn) {
                    return Err(StreamError("005E7ABF: a two-hand weapon over a worn light"));
                }
                return Ok(());
            }
            if menu || light.is_none() {
                if weapon.is_none() || (!menu && light.is_some()) {
                    return Ok(());
                }
                if shield.is_some_and(|shield| !shield.worn) && !dead {
                    // 005E7B1A: the changes' worn stack in slot 13 (00486790) goes back onto the shield's entry; 004E1B40 with
                    // no node returns; the armor rating is invalidated.
                    if equipped_in_slot(world, actor, 13)?.is_some() {
                        let mut middle = world
                            .get_mut::<super::actor_process::MiddleHighProcess>(process)
                            .expect("checked above");
                        if let Some(entry) = middle.shield_f0.as_mut() {
                            entry.worn = true;
                        }
                    }
                    no_node_or_stop(world, actor, "004E1B40: the shield's 3D")?;
                    invalidate_armor_rating(world, actor);
                }
                if light.is_none() {
                    return Ok(());
                }
                return Err(StreamError("005E7B40: a weapon's light arms"));
            }
            Err(StreamError(
                "005E7B83: a weapon with a light outside menu mode",
            ))
        }
        _ => {
            if light.is_some() {
                return Err(StreamError("005E7A33: a shield's light arms"));
            }
            if (weapon.is_some() && !both_hands) || shield.is_none() {
                return Ok(());
            }
            // 005E7A50: 004853B0(shield, 0, 0, 0) drops the worn stack from the process's entry (the changes keep it worn);
            // 004DC8F0(0) with no node returns; the armor rating is invalidated.
            {
                let mut middle = world
                    .get_mut::<super::actor_process::MiddleHighProcess>(process)
                    .expect("checked above");
                if let Some(entry) = middle.shield_f0.as_mut() {
                    entry.worn = false;
                }
            }
            no_node_or_stop(world, actor, "004DC8F0: the shield's 3D detached")?;
            invalidate_armor_rating(world, actor);
            Ok(())
        }
    }
}

// IDA: 004DC8F0's and 004E1B40's first test: no node (+0x3C) returns; a node is a stop here.
fn no_node_or_stop(world: &World, actor: Entity, stop: &'static str) -> Result<(), StreamError> {
    if world
        .get::<super::reference_3d::Reference3D>(actor)
        .is_some()
    {
        return Err(StreamError(stop));
    }
    Ok(())
}

// IDA: 0060E260 Character_InvalidateArmorRatingCache (vt+0x2C0): +0x108 = -1.0.
fn invalidate_armor_rating(world: &mut World, actor: Entity) {
    if let Some(mut character) = world.get_mut::<super::actor::Character>(actor) {
        character.armor_rating_cache_108 = -1.0;
    }
}

/// IDA: vt+0x17C 0064F760(index) on a HighProcess: the current package (vt+0x184 0064B040: +0xC0, else +0x08); the index to +0xCC with a +0xC0 package, else +0x04; then with a package, an index (vt+0x180) at or past 00673980's sequence length for its procedure type (+0x18) is set again to the length less one. Completion: 100% with +0xC0 null.
fn set_procedure_index(world: &mut World, process: Entity, index: i32) -> Result<(), StreamError> {
    super::ai_conditions::stop_on_process_package_override(world, process)?;
    let mut base = world
        .get_mut::<super::actor_process::BaseProcess>(process)
        .expect("a process");
    base.edit_pack_procedure_type_04 = index as u32;
    let Some(package) = base.editor_package_08 else {
        return Ok(());
    };
    let length =
        procedure_sequence_length(super::ai_package::package_data(world, package).field_18);
    if index >= length {
        return set_procedure_index(world, process, length - 1);
    }
    Ok(())
}

/// IDA: 00673980 ProcedureType_GetSequenceLength(type): 2, 3, 4 or 5 by the table, -1 for any other. Completion: 100%.
const fn procedure_sequence_length(procedure_type: u32) -> i32 {
    match procedure_type {
        0 | 7 | 12 | 14 | 15 | 16 | 19 | 21 | 24 | 25 | 29 | 31 | 34..=39 => 2,
        1 | 2 | 3 | 6 | 9 | 13 | 17 | 18 | 20 | 23 => 3,
        4 | 5 | 10 | 22 | 26 | 28 | 30 | 32 => 4,
        8 | 11 | 27 | 33 => 5,
        _ => -1,
    }
}

/// IDA: N/A each 0069AF30 cast the port doesn't make: the caster's actor and the spell. 005EDA20 reaches it through the caster's vt+0 (005E51B0 -> process vt+0x2A0 0064AE50 -> 0069BE00, an ability, type 4) and vt+4 (005E51E0 -> vt+0x2A4 0064AE70 -> 0069BE40, a disease, type 1, with the actor's +0x68 target), each after the caster's vt+0x34 on the spell's +0x18 part.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct CastBoundaries {
    pub casts: Vec<(Entity, Entity)>,
}

/// IDA: 005EDA20(0): `base_and_race_abilities`' walk, each spell through the caster's vt+0 and vt+4, which cast it (an ability or a disease). Completion: 100% of the walk; the casts are `CastBoundaries`.
/// TODO: [ ] 0069AF30, the cast and the active effects it makes.
fn apply_base_spells(world: &mut World, actor: Entity) -> Result<(), StreamError> {
    if Some(actor) == super::ai_conditions::the_player(world) {
        return Err(StreamError("005EDA97: the player's birth sign"));
    }
    let spells = base_and_race_abilities(world, actor);
    for spell in spells {
        world
            .get_resource_or_init::<CastBoundaries>()
            .casts
            .push((actor, spell));
    }
    Ok(())
}

/// IDA: the walk 005EDA20(0) and 005E4DD0 share for a non-player: the base's spell list (+0x58) when vt+0x190 says an actor, then (a Character's birth sign, 006F7070, is null) an NPC's race's (+0xE8, its +0x30), in list order, keeping each spell whose type (the +0x18 part's vt+0x18: +0x34) is 4 (an ability) or 1 (a disease). Completion: 100% of a non-player.
#[must_use]
pub(crate) fn base_and_race_abilities(world: &World, actor: Entity) -> Vec<Entity> {
    use super::form_components::{TesRaceForm, TesSpellList};
    let base = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .base_form_1c;
    let spells_of = |world: &World, form: Option<Entity>| -> Vec<Entity> {
        form.and_then(|form| world.get::<TesSpellList>(form))
            .map(|list| {
                list.spells_04
                    .iter()
                    .filter_map(|spell| match spell {
                        FormRef::Resolved(spell) => *spell,
                        FormRef::Raw(_) => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut spells = spells_of(world, base);
    if base.is_some_and(|base| super::forms_init::form_type(world, base) == 0x23) {
        let race = base
            .and_then(|base| world.get::<TesRaceForm>(base))
            .and_then(|form| match form.race_04 {
                FormRef::Resolved(race) => race,
                FormRef::Raw(_) => None,
            });
        if race.is_some() {
            spells.extend(spells_of(world, race));
        }
    }
    spells.retain(|&spell| {
        let spell_type = world
            .get::<super::forms_magic::MagicFormData>(spell)
            .map_or(u32::MAX, |data| data.type_34);
        spell_type == 4 || spell_type == 1
    });
    spells
}
