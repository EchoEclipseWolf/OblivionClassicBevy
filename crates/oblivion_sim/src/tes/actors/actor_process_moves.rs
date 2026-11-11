//! The moves between process levels: 005EBB30 Actor_MoveToMiddleLowProcess (Actor vt+0x1AC), which 004CCA60
//! TESObjectCELL_ChangeProcessMiddleLowCount runs on each level-3 actor of a cell that gains an ExtraProcessMiddleLow, and the
//! helpers it calls: 00674850, 005E4B00, 006765A0 with 00631C80.
//!
//! The ten run-ah records (on the clone thread, under `004E43BF` <- `0043AAD7` <- `00430DEB`) are cell 22FF6's four creatures
//! and ImperialDungeon02's six NPCs, each a LowProcess at level 3 with no current package of a combat type, a dead state of
//! 0, a base without flag 0x400 and +0xCA already 1. The port runs them on the main thread (D9), so each move takes the
//! original's thread from its caller for 0045A500's save-load bit: the clone thread for 004CCA60's calls, the main thread for
//! 005F23E0's.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::actor::{Actor, MobileObject, TesObjectRefr};
use super::actor_process::{
    ActorProcessGlobals, BaseProcess, HIGH_PROCESS_VTABLE, HighProcess, LOW_PROCESS_VTABLE,
    LowProcess, MIDDLE_LOW_PROCESS_VTABLE, actor_process_manager_add, actor_process_manager_remove,
    construct_middle_low_process, copy_process_from, delete_low_process, process_level,
};
use crate::tes::form::{FormIdentity, OriginalVtable};

/// IDA: N/A the actors 004CCA60 handed to vt+0x1AC (005EBB30), in call order. run-ah calls it ten times, in this order:
/// cell 22FF6's `387D9`, `387DA`, `387DB`, `491FC`, then ImperialDungeon02's `176FF`, `17708`, `1772E`, `1770C`, `17752`,
/// `17754`. The commands test compares the record with that list.
#[derive(Resource, Default, Debug)]
pub struct MiddleLowPromotions {
    pub actors: Vec<Entity>,
}

// IDA: 0045A500 TESSaveLoadGame_TestCurrentThreadActiveFlag: g_saveLoadGame +0x18 bit 0 on the main thread, bit 0x40000 on
// another (`clone_thread`: the caller says which thread the original runs it on, since D9 runs everything on the main one).
// A null g_saveLoadGame faults, a host stop.
fn save_load_active_on(world: &World, clone_thread: bool) -> Result<bool, GameError> {
    let flags = crate::tes::forms_references::save_load_flags(world)?;
    Ok(flags & if clone_thread { 0x4_0000 } else { 1 } != 0)
}

// IDA: N/A the process's +0x08 package (vt+0x184 on Low and MiddleLow: 00452A60) and its type byte +0x20.
fn current_package_type(world: &World, process: Entity) -> Option<(Entity, i8)> {
    let package = world.get::<BaseProcess>(process)?.editor_package_08?;
    Some((
        package,
        crate::tes::ai_package::package_data(world, package).type_20,
    ))
}

// IDA: 00674850 ActorProcessManager_RemoveNearPlayerActor(actor) (thiscall on g_actorProcessManager): 004466B0 removes the first
// match from the +0x60 list. Completion: 100%.
fn manager_remove_near_player(world: &mut World, actor: Entity) {
    world
        .resource_mut::<ActorProcessGlobals>()
        .manager
        .near_player_60
        .remove_first_matching(actor);
}

// IDA: 006765A0 ActorProcessManager_RemoveFromDetectionLists(actor): each actor of the High list (+0x68, from its head, ending at
// a null item) whose vt+0x190 answers true (00977C50 on a Character and a Creature) has its process's vt+0x1BC(actor) run:
// 00631C80 HighProcess_RemoveDetectedActor walks +0x18C's list for an entry whose +0x00 is the actor, removes it (004466B0) and
// frees it. The port's detection lists never hold an entry, so a set one is a stop. Completion: 100% of empty lists.
// TODO: [ ] 00631C80's removal, when a port fills +0x18C.
fn manager_remove_from_detection_lists(world: &World, actor: Entity) -> Result<(), GameError> {
    let _ = actor;
    // `items()` skips null items where 006765A0 stops at the first; the port's lists hold none.
    for other in world
        .resource::<ActorProcessGlobals>()
        .manager
        .high_68
        .list
        .items()
    {
        if world.get::<Actor>(other).is_none() {
            continue;
        }
        let Some(process) = world.get::<MobileObject>(other).and_then(|m| m.process_58) else {
            return Err(GameError::unclassified(
                0x006765C6,
                "006765C6: a null process faults",
            ));
        };
        if world.get::<OriginalVtable>(process).map(|vtable| vtable.0) != Some(HIGH_PROCESS_VTABLE)
        {
            return Err(GameError::unclassified(
                0x006765C6,
                "006765C6: vt+0x1BC on a process other than a HighProcess",
            ));
        }
        let detection = world
            .get::<HighProcess>(process)
            .expect("a HighProcess")
            .detection_list_18c;
        if detection.is_some_and(|head| head[0] != 0) {
            return Err(GameError::unclassified(
                0x00631C80,
                "00631C80: a detection list with entries",
            ));
        }
    }
    Ok(())
}

/// IDA: 005E4B00 Actor_RemoveAbilityAndEnchantmentEffects. Each spell of the base's TESSpellList (+0x58) whose MagicItem
/// (+0x18) vt+0x18 type is 4 (ability) or 1 (disease) goes to 006A1F70(+0x68, item, 0, 0), restarting the walk when 0052B4A0's
/// count changes; then vt+0x268's birthsign spells (+0x3C; 006F7070 null on a Character and a Creature) and an NPC's race spells
/// (+0xE8 +0x30) the same way. 00485E00 then gets or makes the container changes, and each of the ten slots at 00B1489C (0..8,
/// 13) goes through 00486790 (slot, 0): an enchanted worn item (an RTTI TESEnchantableForm with +0x04) reaches
/// MagicTarget_RemoveEffectsFromItem(+0x68, form, 0), and the selection is freed (00484470, FormHeap_Free). Both effect
/// walks start at the MagicTarget's vt+8 (005E5220: the process's vt+0x298), which a LowProcess and a MiddleLowProcess answer
/// null (006F7070), so they remove nothing. Completion: 100% for an actor with no process, a LowProcess or a MiddleLowProcess;
/// another process (an effect list) is a stop.
/// TODO: [ ] 006A1F70's and 00661A10's removals over a MiddleHigh or High process's effect list (the magic port).
/// # Errors
/// The stop above, or 00486790's.
pub fn actor_remove_ability_and_enchantment_effects(
    world: &mut World,
    actor: Entity,
) -> Result<(), GameError> {
    if let Some(process) = world.get::<MobileObject>(actor).and_then(|m| m.process_58)
        && !matches!(
            world.get::<OriginalVtable>(process).map(|vtable| vtable.0),
            Some(LOW_PROCESS_VTABLE | MIDDLE_LOW_PROCESS_VTABLE)
        )
    {
        return Err(GameError::unclassified(
            0x005E4BAE,
            "005E4BAE: 006A1F70 over a process's effect list",
        ));
    }
    // 005E4D29..005E4D59 00485E00(this, the base's +0x44 container): the changes block, made when missing.
    let changes = super::inventory::ensure_container_changes(world, actor)?;
    super::inventory::put_container_changes(world, actor, changes);
    // 005E4D60..005E4DBE the ten slots.
    for slot in [0, 1, 2, 3, 4, 5, 6, 7, 8, 13] {
        crate::tes::equipment::equipped_in_slot(world, actor, slot)?;
    }
    Ok(())
}

/// IDA: 005EBB30 Actor_MoveToMiddleLowProcess, Actor vt+0x1AC (Character and Creature), always answering 1. 00674850; a process at
/// level 2 returns. Level 0 with a shader accumulator runs 007AD1E0, and vt+0x184's package of type 0x12 asks vt+0x1CC (false on
/// a LowProcess, 0069D990) before 005EAE70. Then 005E4B00, and 005E4FC0 at level 0 or 1; 006A2160 on +0x68 (null on a
/// LowProcess); a dead state of 1 (vt+0x1F8 005E33E0) runs 005E9E70, the process's vt+0x23C and TESObjectREFR_RunScripts,
/// otherwise a dead actor at level 0 gets vt+0x9C(1). 00674550 at the old level. With g_dataHandler +0xCD4 clear: 006765A0
/// unless a save loads, and a package of type 5 clears the process's +0x04. A dead state of 6 is set to 0 (Actor_SetDeathState)
/// with the process's vt+0xA0(0.0). The process's vt+0x388 (false on a LowProcess) is kept. FormHeap(0xA8) 00658690; a combat
/// package (00567770) takes the follower and combat arms. The new process's vt+4 0060CDB0 from the old, the old one's vt+0(1),
/// +0x58 = the new, 00673A90(actor, 2, 1, 0, 0). With g_dataHandler set and +0xCD4 clear: vt+0x334(1) 005E6110 (with
/// MiddleLow's vt+0x36C 0, a package of type 12 or 13) takes the follower arm; otherwise vt+0x78 (004D9B10: the base's flag
/// 0x400) with +0xCA set, or 005E3220 (a package of type 1 without flag 0x800, or 0x1F) or 005E30A0 (type 2) with the
/// process's +0x2C the player, sets the hour (vt+0x1C: the game hour less 0.5, as float) and moves the actor to the front of
/// the level-2 list (00674550, then 00673A90(actor, 2, 0, 0, 0)). Last, +0xCA = 1. Nothing here draws.
/// Completion: 100% of the arms for an actor with a LowProcess outside a combat package, dead state 0 or 2, not following the
/// player; the others stop.
/// TODO: [ ] The old process at level 0 or 1 (007AD1E0, 005E4FC0, a MiddleHigh or High old process's slots).
/// TODO: [ ] A dead state of 1 (005E9E70, TESObjectREFR_RunScripts) or 6 (Actor_SetDeathState(0), vt+0xA0).
/// TODO: [ ] The combat arms (005EAE70, 00628140, vt+0x148/vt+0x228) and the follower arm (00680EE0, 006162D0, the load-door
/// follower count +0x110).
/// TODO: [ ] 005E30A0's distance test for an escort package whose +0x2C is the player.
/// `clone_thread` is the original's thread for 0045A500 (004CCA60's calls run on the clone thread).
/// # Errors
/// A stop named above, or one of 005E4B00's, 0060CDB0's, the destructor's or 00673A90's.
pub fn actor_move_to_middle_low_process(
    world: &mut World,
    actor: Entity,
    clone_thread: bool,
) -> Result<(), GameError> {
    // 005EBB59 00674850.
    manager_remove_near_player(world, actor);
    // 005EBB64..005EBB77 level 2 returns.
    let process = world.get::<MobileObject>(actor).and_then(|m| m.process_58);
    if process.is_some_and(|process| process_level(world, process) == 2) {
        return Ok(());
    }
    // 005EBBD6 reads the process's vtable with no null test.
    let Some(old) = process else {
        return Err(GameError::unclassified(
            0x005EBBD9,
            "005EBBD9: a null process faults",
        ));
    };
    let level = process_level(world, old);
    if level != 3 {
        return Err(GameError::unclassified(
            0x005EBB8B,
            "005EBB8B: moving a High or MiddleHigh process to MiddleLow",
        ));
    }
    // 005EBBA3..005EBBD1 a type 0x12 package asks vt+0x1CC: 0069D990 false on a LowProcess, so no 005EAE70.
    // 005EBBE4 005E4B00; 005EBBE9..005EBBF4 005E4FC0 only at level 0 or 1.
    actor_remove_ability_and_enchantment_effects(world, actor)?;
    // 005EBBF9 006A2160 on +0x68: vt+8 005E5220 reads the process's vt+0x298, 006F7070 null on a LowProcess.
    // 005EBC01..005EBC58 vt+0x1F8 005E33E0 (dead state 1); otherwise vt+0x198(0) needs level 0 for vt+0x9C.
    if world.get::<Actor>(actor).expect("an actor").dead_state_b0 == 1 {
        return Err(GameError::unclassified(
            0x005EBC13,
            "005EBC13: 005E9E70 and RunScripts for dead state 1",
        ));
    }
    // 005EBC5A..005EBC61 00674550 at the old level.
    let loading = save_load_active_on(world, clone_thread)?;
    actor_process_manager_remove(world, actor, level, loading);
    // 005EBC66 g_dataHandler +0xCD4, read with no null test.
    let cd4 = world
        .get_resource::<crate::tes::data_handler::DataHandler>()
        .ok_or(GameError::unclassified(
            0x005EBC6B,
            "005EBC6B: a null g_dataHandler faults",
        ))?
        .bytes_cd0[4];
    if cd4 == 0 {
        // 005EBC73..005EBC88 006765A0 unless a save loads.
        if !loading {
            manager_remove_from_detection_lists(world, actor)?;
        }
        // 005EBC8D..005EBCA1 a package of type 5 clears the process's +0x04.
        if let Some(process) = world.get::<MobileObject>(actor).and_then(|m| m.process_58)
            && current_package_type(world, process).is_some_and(|(_, kind)| kind == 5)
        {
            world
                .get_mut::<BaseProcess>(process)
                .expect("a process")
                .edit_pack_procedure_type_04 = 0;
        }
    }
    // 005EBCA4..005EBCC6 dead state 6.
    if world.get::<Actor>(actor).expect("an actor").dead_state_b0 == 6 {
        return Err(GameError::unclassified(
            0x005EBCB0,
            "005EBCB0: Actor_SetDeathState(0) for dead state 6",
        ));
    }
    // 005EBCC8..005EBCDF vt+0x388: 0069D990 false on a LowProcess (00674550's release may have left none: false too).
    let current = world.get::<MobileObject>(actor).and_then(|m| m.process_58);
    // 005EBCE3..005EBD09 FormHeap(0xA8) MiddleLowProcess_ctor.
    let new = construct_middle_low_process(world);
    // 005EBD0B..005EBD37 a combat package (00567770).
    if current
        .and_then(|process| current_package_type(world, process))
        .is_some_and(|(package, _)| {
            crate::tes::ai_package::package_is_combat_like(crate::tes::ai_package::package_data(
                world, package,
            ))
        })
    {
        return Err(GameError::unclassified(
            0x005EBD3D,
            "005EBD3D: the combat arms for a combat package",
        ));
    }
    // 005EBE3B..005EBE46 new vt+4 0060CDB0(+0x58).
    copy_process_from(world, new, current)?;
    // 005EBE48..005EBE55 the old one's vt+0(1): 0064ACD0 on a LowProcess.
    if let Some(old) = current {
        delete_low_process(world, old)?;
    }
    // 005EBE65 +0x58 = the new; 005EBE68 00673A90(actor, 2, 1, 0, 0).
    world
        .get_mut::<MobileObject>(actor)
        .expect("an actor")
        .process_58 = Some(new);
    actor_process_manager_add(world, actor, 2, true, false)?;
    // 005EBE6D..005EBEB8 the combat arm's byte is 0 here and vt+0x388 was false: nothing.
    // 005EBEBA..005EBECE g_dataHandler null or +0xCD4 set returns.
    if world
        .get_resource::<crate::tes::data_handler::DataHandler>()
        .is_none_or(|data| data.bytes_cd0[4] != 0)
    {
        return Ok(());
    }
    // 005EBEE8..005EBEFA vt+0x334(1) 005E6110: vt+0x36C on a MiddleLowProcess is 0, so the package's type decides.
    let package = current_package_type(world, new);
    if package.is_some_and(|(_, kind)| matches!(kind, 12 | 13)) {
        return Err(GameError::unclassified(
            0x005EBF00,
            "005EBF00: the follower arm for a type 12 or 13 package",
        ));
    }
    // 005EC0CC..005EC109 the reschedule test: vt+0x78 004D9B10 with +0xCA set, or 005E3220 / 005E30A0 with +0x2C the player.
    let reschedule = reschedule_test(world, actor, new)?;
    if reschedule {
        // 005EC10B..005EC132 vt+0x1C 00629240 (+0x14): the game hour less 0.5 (dbl_A2FAA0), stored as float.
        let hour = (f64::from(crate::tes::global::game_hour(world)) - 0.5) as f32;
        world
            .get_mut::<LowProcess>(new)
            .expect("a process")
            .cur_hour_14 = hour;
        // 005EC134..005EC14F 00674550(actor, 2), then 00673A90(actor, 2, 0, 0, 0): to the list's front.
        let loading = save_load_active_on(world, clone_thread)?;
        actor_process_manager_remove(world, actor, 2, loading);
        actor_process_manager_add(world, actor, 2, false, false)?;
    }
    // 005EC154 +0xCA = 1.
    world
        .get_mut::<Actor>(actor)
        .expect("an actor")
        .allow_quest_process_reschedule_ca = 1;
    Ok(())
}

// IDA: N/A 005EB520's and 005EBB30's shared tail with g_dataHandler set and +0xCD4 clear, after vt+0x334(1) answered false: the
// reschedule test (vt+0x78 004D9B10's base flag 0x400 with +0xCA set, or 005E3220 or 005E30A0 with the process's +0x2C the
// player). True moves the actor to the front of its level's list, +0x14 set to the game hour less 0.5 (005EBB30) or 1.0
// (005EB520). Completion: 100% but 005E30A0's distance test, a stop.
fn reschedule_test(world: &World, actor: Entity, process: Entity) -> Result<bool, GameError> {
    let base_persistent = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .base_form_1c
        .is_some_and(|base| world.get::<FormIdentity>(base).expect("a form").flags_08 & 0x400 != 0);
    let ca = world
        .get::<Actor>(actor)
        .expect("an actor")
        .allow_quest_process_reschedule_ca;
    if base_persistent && ca != 0 {
        return Ok(true);
    }
    let package = current_package_type(world, process);
    let player = world
        .get_resource::<super::player::PlayerGlobals>()
        .and_then(|globals| globals.the_player_b333c4);
    let follows_player = player.is_some()
        && world
            .get::<LowProcess>(process)
            .expect("a process")
            .field_2c
            == player;
    // 005E3220: a package of type 1 without flag 0x800 (TESPackage_TestFlag800), or type 0x1F.
    let type_1_or_1f = package.is_some_and(|(package, kind)| {
        (kind == 1 && crate::tes::ai_package::package_data(world, package).flags_1c & 0x800 == 0)
            || kind == 0x1F
    });
    // 005E30A0: false unless a type 2 package's +0x2C is the player, which needs its distance test.
    if !type_1_or_1f && package.is_some_and(|(_, kind)| kind == 2) && follows_player {
        return Err(GameError::unclassified(
            0x005E30A0,
            "005E30A0: an escort package's distance test",
        ));
    }
    Ok(type_1_or_1f && follows_player)
}

/// IDA: 005EB520 Actor_MoveToLowProcess, Actor vt+0x1A8 (Character and Creature), always answering 1. A process at level 3
/// returns. Level 0 with a shader accumulator runs 007AD1E0, and vt+0x184's package of type 0x12 asks vt+0x1CC (false on a
/// MiddleLowProcess) before 005EAE70. 00674850; a dead state of 6 is set to 0 with vt+0xA0(0.0); a package of type 0x12 whose
/// +0x2C isn't the player, or of type 0x0F for a non-guard while the player's cell is null or exterior, reaches 005EAE70.
/// 005E4B00, and 005E4FC0 at level 0 or 1; 006A2160 on +0x68; with g_dataHandler +0xCD4 clear and no save loading, 006765A0. A
/// dead state of 1 runs 005E9E70, vt+0x23C and RunScripts; otherwise a dead actor at level 0 gets vt+0x9C(1). 00674550 at the
/// old level; vt+0x388 (false on a MiddleLowProcess) kept; a combat package takes 005EAE70's arms. FormHeap(0x90)
/// LowProcess_ctor, the new process's vt+4 0060CDB0 from the old, the old one's vt+0(1), +0x58 = the new, its vt+0xB8(actor)
/// (00643740: g_pathBuilder's 006826D0 removal and +0x34 freed), 00673A90(actor, 3, 1, 0, 0). With g_dataHandler set and
/// +0xCD4 clear: vt+0x334(1) (a package of type 12 or 13; LowProcess vt+0x36C is 0) takes the follower arm; otherwise the
/// reschedule test sets +0x14 to the game hour less 1.0 (dbl_A2F928) and moves the actor to the front of the level-3 list.
/// +0xCA = 1. Nothing here draws.
/// Completion: 100% of the arms for an actor with a MiddleLowProcess outside packages of types 0x0F, 0x12 and the combat
/// types, dead state 0 or 2; the others stop. No caller in the port reaches it yet (005F23E0's level-3 arm).
/// TODO: [ ] An old High or MiddleHigh process (007AD1E0, 005E4FC0, their slots).
/// TODO: [ ] 005EAE70's arms (types 0x0F and 0x12, the combat types), dead states 1 and 6, and the follower arm.
/// TODO: [ ] g_pathBuilder's 006826D0 removal (host substitution: the port keeps no path builder entries per actor).
/// `clone_thread` is the original's thread for 0045A500.
/// # Errors
/// A stop named above, or one of 005E4B00's, 0060CDB0's, the destructor's or 00673A90's.
pub fn actor_move_to_low_process(
    world: &mut World,
    actor: Entity,
    clone_thread: bool,
) -> Result<(), GameError> {
    // 005EB54B..005EB55A level 3 returns.
    let process = world.get::<MobileObject>(actor).and_then(|m| m.process_58);
    if process.is_some_and(|process| process_level(world, process) == 3) {
        return Ok(());
    }
    let Some(old) = process else {
        return Err(GameError::unclassified(
            0x005EB61A,
            "005EB61A: a null process faults",
        ));
    };
    let level = process_level(world, old);
    if level != 2 {
        return Err(GameError::unclassified(
            0x005EB56C,
            "005EB56C: moving a High or MiddleHigh process to Low",
        ));
    }
    // 005EB58A..005EB5B5 a type 0x12 package asks vt+0x1CC: 0069D990 false on a MiddleLowProcess.
    // 005EB5C0 00674850.
    manager_remove_near_player(world, actor);
    // 005EB5C5..005EB5E8 dead state 6.
    if world.get::<Actor>(actor).expect("an actor").dead_state_b0 == 6 {
        return Err(GameError::unclassified(
            0x005EB5D2,
            "005EB5D2: Actor_SetDeathState(0) for dead state 6",
        ));
    }
    // 005EB5EA..005EB658 005EAE70 for a type 0x12 package not following the player, or a type 0x0F one.
    if let Some((_, kind)) = current_package_type(world, old)
        && (kind == 0x12 || kind == 0x0F)
    {
        return Err(GameError::unclassified(
            0x005EB611,
            "005EB611: 005EAE70 for a package of type 0x0F or 0x12",
        ));
    }
    // 005EB66B 005E4B00; 005EB670..005EB67B 005E4FC0 only at level 0 or 1; 005EB683 006A2160 (null on a MiddleLowProcess).
    actor_remove_ability_and_enchantment_effects(world, actor)?;
    // 005EB68E..005EB6AC 006765A0 with +0xCD4 clear and no save loading.
    let cd4 = world
        .get_resource::<crate::tes::data_handler::DataHandler>()
        .ok_or(GameError::unclassified(
            0x005EB68E,
            "005EB68E: a null g_dataHandler faults",
        ))?
        .bytes_cd0[4];
    let loading = save_load_active_on(world, clone_thread)?;
    if cd4 == 0 && !loading {
        manager_remove_from_detection_lists(world, actor)?;
    }
    // 005EB6B4..005EB709 dead state 1; a dead actor at level 0 only.
    if world.get::<Actor>(actor).expect("an actor").dead_state_b0 == 1 {
        return Err(GameError::unclassified(
            0x005EB6C3,
            "005EB6C3: 005E9E70 and RunScripts for dead state 1",
        ));
    }
    // 005EB712 00674550 at the old level.
    actor_process_manager_remove(world, actor, level, loading);
    // 005EB718..005EB72C vt+0x388: false on a MiddleLowProcess. 005EB735..005EB793 a combat package.
    let current = world.get::<MobileObject>(actor).and_then(|m| m.process_58);
    let Some(current) = current else {
        return Err(GameError::unclassified(
            0x005EB73C,
            "005EB73C: a null process faults",
        ));
    };
    if current_package_type(world, current).is_some_and(|(package, _)| {
        crate::tes::ai_package::package_is_combat_like(crate::tes::ai_package::package_data(
            world, package,
        ))
    }) {
        return Err(GameError::unclassified(
            0x005EB748,
            "005EB748: the combat arms for a combat package",
        ));
    }
    // 005EB79D..005EB7B3 FormHeap(0x90) LowProcess_ctor; 005EB7CD vt+4 0060CDB0; 005EB7DC the old one's vt+0(1).
    let new = super::actor_process::construct_low_process_for_copy(world);
    copy_process_from(world, new, Some(current))?;
    delete_low_process(world, current)?;
    // 005EB7DE +0x58 = the new; 005EB7EC vt+0xB8 00643740.
    world
        .get_mut::<MobileObject>(actor)
        .expect("an actor")
        .process_58 = Some(new);
    if world.get::<LowProcess>(new).expect("a process").pathing_34 != 0 {
        return Err(GameError::unclassified(
            0x00643762,
            "00643762: freeing a LowProcess's +0x34",
        ));
    }
    // 005EB7FC 00673A90(actor, 3, 1, 0, 0).
    actor_process_manager_add(world, actor, 3, true, false)?;
    // 005EB805..005EB84E g_dataHandler null or +0xCD4 set returns.
    if world
        .get_resource::<crate::tes::data_handler::DataHandler>()
        .is_none_or(|data| data.bytes_cd0[4] != 0)
    {
        return Ok(());
    }
    // 005EB873..005EB885 vt+0x334(1) 005E6110.
    if current_package_type(world, new).is_some_and(|(_, kind)| matches!(kind, 12 | 13)) {
        return Err(GameError::unclassified(
            0x005EB88B,
            "005EB88B: the follower arm for a type 12 or 13 package",
        ));
    }
    // 005EBA89..005EBB0E the reschedule test: +0x14 = the game hour less 1.0, then the list's front.
    if reschedule_test(world, actor, new)? {
        let hour = (f64::from(crate::tes::global::game_hour(world)) - 1.0) as f32;
        world
            .get_mut::<LowProcess>(new)
            .expect("a process")
            .cur_hour_14 = hour;
        actor_process_manager_remove(world, actor, 3, loading);
        actor_process_manager_add(world, actor, 3, false, false)?;
    }
    // 005EBB13 +0xCA = 1.
    world
        .get_mut::<Actor>(actor)
        .expect("an actor")
        .allow_quest_process_reschedule_ca = 1;
    Ok(())
}

/// IDA: 005F23E0 Actor_SyncProcessLevel, Actor vt+0x1C4, on the main thread (004DD850, 0043F240). No process answers 0; a level equal to 0065AD00's answers 1. A
/// HighProcess whose vt+0x47C (006299B0: +0x2BC) is 6 is destroyed (vt+0x10(1)) and one of 5 goes to 004E4690, both answering
/// 0. Then by the desired level: 0 vt+0x1A4 (005FA320), 1 vt+0x1B0, 2 vt+0x1AC (005EBB30) unless 005F0750 releases a summon
/// (then vt+0x10(1)), 3 for a persistent reference (004DB4A0) vt+0x1A8 (005EB520) unless 005F0750, and for another the
/// release arm (005E4B00, 006A2160, 005EDA20(0), TESSaveLoadGame_UnloadForm) and vt+0x10(1). 005F0750 reads the process's
/// vt+0x3D0, null on Low and MiddleLow (006F7070), so it is false there. Completion: the arms to levels 0, 2 and 3 for a Low or
/// MiddleLow process, and to 0 for any; the others stop.
/// TODO: [ ] vt+0x1B0 (005F1B50, the move to a MiddleHighProcess), Destroy, 004E4690, and 005F0750 on a MiddleHigh or High process.
/// # Errors
/// A stop named above, or the move's.
pub fn actor_sync_process_level(world: &mut World, actor: Entity) -> Result<bool, GameError> {
    let Some(process) = world.get::<MobileObject>(actor).and_then(|m| m.process_58) else {
        return Ok(false);
    };
    let level = process_level(world, process);
    let desired = super::actor_process::actor_desired_level(world, actor)?;
    if level == desired {
        return Ok(true);
    }
    // 005F2408..005F2450 a HighProcess's +0x2BC of 5 or 6.
    if level == 0 {
        let state = world
            .get::<HighProcess>(process)
            .map_or(0, |high| high.field_2bc);
        if state == 5 || state == 6 {
            return Err(GameError::unclassified(
                0x005F242D,
                "005F242D: a HighProcess's +0x2BC of 5 or 6 (004E4690 or Destroy)",
            ));
        }
    }
    let low_or_middle_low = matches!(
        world.get::<OriginalVtable>(process).map(|vtable| vtable.0),
        Some(LOW_PROCESS_VTABLE | MIDDLE_LOW_PROCESS_VTABLE)
    );
    match desired {
        // 005F245E vt+0x1A4.
        0 => {
            crate::tes::actor::actor_rebuild_high_process(world, actor)?;
            Ok(true)
        }
        1 => Err(GameError::unclassified(
            0x005F2470,
            "005F2470: vt+0x1B0, the move to a MiddleHighProcess",
        )),
        // 005F2482..005F2497 005F0750, then vt+0x1AC.
        2 if low_or_middle_low => {
            actor_move_to_middle_low_process(world, actor, false)?;
            Ok(true)
        }
        // 005F249F..005F24CC 004DB4A0, 005F0750, then vt+0x1A8.
        3 if low_or_middle_low => {
            if !crate::tes::forms_references::reference_is_persistent(world, actor)? {
                return Err(GameError::unclassified(
                    0x005F24D4,
                    "005F24D4: releasing a non-persistent actor at level 3",
                ));
            }
            actor_move_to_low_process(world, actor, false)?;
            Ok(true)
        }
        2 | 3 => Err(GameError::unclassified(
            0x005F2484,
            "005F2484: 005F0750 on a MiddleHigh or High process",
        )),
        _ => Ok(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tes::actor_process::{
        MIDDLE_LOW_PROCESS_VTABLE, MiddleLowProcess, ProcessInputs, RandomSource,
        construct_actor_process_manager,
    };

    // IDA: N/A a CRT seam that never draws (LowProcess_ctor draws nothing).
    struct NoDraws;

    impl RandomSource for NoDraws {
        // IDA: N/A test seam.
        fn time64(&mut self) -> i64 {
            panic!("no draw")
        }
        // IDA: N/A test seam.
        fn srand(&mut self, _seed: u32) {
            panic!("no draw")
        }
        // IDA: N/A test seam.
        fn rand(&mut self) -> i32 {
            panic!("no draw")
        }
    }

    // IDA: N/A a world with what 005EBB30 and 005EB520 read.
    fn world_with_actors(count: usize) -> (World, Vec<Entity>) {
        let mut world = World::new();
        world.insert_resource(crate::tes::game_settings::GameSettingCollection::default());
        world.init_resource::<crate::tes::player::PlayerGlobals>();
        world.insert_resource(crate::tes::tes::GameThreadLocals::default());
        world.insert_resource(crate::tes::tes::TesGlobals {
            save_load_game: Some(crate::tes::save_load_game::TesSaveLoadGame::default()),
            ..crate::tes::tes::TesGlobals::default()
        });
        crate::tes::data_handler::construct_data_handler(&mut world).unwrap();
        construct_actor_process_manager(&mut world);
        let actors = (0..count)
            .map(|_| {
                let mut random = NoDraws;
                let inputs = ProcessInputs {
                    random: &mut random,
                    idle_chatter_comment_timer: 0,
                    low_process_unset_88: 0x1234,
                    save_load_active: false,
                };
                crate::tes::actor::construct_character(&mut world, &inputs).unwrap()
            })
            .collect();
        (world, actors)
    }

    // IDA: N/A the actor's process vtable.
    fn vtable(world: &World, actor: Entity) -> u32 {
        let process = world
            .get::<MobileObject>(actor)
            .unwrap()
            .process_58
            .unwrap();
        world.get::<OriginalVtable>(process).unwrap().0
    }

    // 005EBB30 moves each level-3 actor to a new MiddleLowProcess appended to the level-2 list (00673A90(2, 1)), copying the
    // Low part (00643520) and +0xCA = 1; a second call returns at level 2. 005EB520 then moves one back to a LowProcess,
    // appended to the level-3 list, with the MiddleLow +0x94 released.
    #[test]
    fn the_moves_to_middle_low_and_back() {
        use crate::tes::actor_process::{ProcessLevelLow, ProcessLevelMiddleLow};
        let (mut world, actors) = world_with_actors(2);
        let manager = |world: &World| {
            world
                .resource::<ActorProcessGlobals>()
                .manager
                .middle_low_0c
                .list
                .items()
        };
        for &actor in &actors {
            let process = world
                .get::<MobileObject>(actor)
                .unwrap()
                .process_58
                .unwrap();
            world.get_mut::<LowProcess>(process).unwrap().cur_hour_14 = 7.5;
            actor_move_to_middle_low_process(&mut world, actor, true).unwrap();
            assert_eq!(vtable(&world, actor), MIDDLE_LOW_PROCESS_VTABLE);
            assert!(world.get::<ProcessLevelMiddleLow>(actor).is_some());
            assert!(world.get::<ProcessLevelLow>(actor).is_none());
        }
        assert_eq!(manager(&world), actors, "appended in call order");
        let low_list = world
            .resource::<ActorProcessGlobals>()
            .manager
            .low_18
            .list
            .items();
        assert!(
            actors.iter().all(|actor| !low_list.contains(actor)),
            "off the level-3 list"
        );
        let process = world
            .get::<MobileObject>(actors[0])
            .unwrap()
            .process_58
            .unwrap();
        let low = world.get::<LowProcess>(process).unwrap();
        assert_eq!(low.cur_hour_14, 7.5, "00643520 copies +0x14");
        assert_eq!(low.unset_88, 0x1234, "00643520 copies +0x88");
        assert!(world.get::<MiddleLowProcess>(process).is_some());
        assert_eq!(
            world
                .get::<Actor>(actors[0])
                .unwrap()
                .allow_quest_process_reschedule_ca,
            1
        );
        // Level 2 returns at once.
        actor_move_to_middle_low_process(&mut world, actors[0], true).unwrap();
        assert_eq!(
            world.get::<MobileObject>(actors[0]).unwrap().process_58,
            Some(process)
        );
        // 005EB520 back to Low.
        actor_move_to_low_process(&mut world, actors[0], false).unwrap();
        assert_eq!(vtable(&world, actors[0]), LOW_PROCESS_VTABLE);
        assert!(world.get::<ProcessLevelLow>(actors[0]).is_some());
        assert!(world.get::<ProcessLevelMiddleLow>(actors[0]).is_none());
        assert!(
            world.get_entity(process).is_err(),
            "00658970 deleted the MiddleLowProcess"
        );
        assert_eq!(manager(&world), [actors[1]]);
        let low_list = world
            .resource::<ActorProcessGlobals>()
            .manager
            .low_18
            .list
            .items();
        assert_eq!(
            low_list.last(),
            Some(&actors[0]),
            "appended to the level-3 list"
        );
        // Level 3 returns at once.
        actor_move_to_low_process(&mut world, actors[0], false).unwrap();
    }

    // 005EBB30 with no process faults at 005EBBD9, a host stop; dead state 1 stops at 005EBC13.
    #[test]
    fn the_unported_arms_stop() {
        let (mut world, actors) = world_with_actors(1);
        world.get_mut::<MobileObject>(actors[0]).unwrap().process_58 = None;
        assert!(actor_move_to_middle_low_process(&mut world, actors[0], true).is_err());
        let (mut world, actors) = world_with_actors(1);
        world.get_mut::<Actor>(actors[0]).unwrap().dead_state_b0 = 1;
        assert!(actor_move_to_middle_low_process(&mut world, actors[0], true).is_err());
    }
}
