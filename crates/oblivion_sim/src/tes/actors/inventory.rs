//! Phase 12d: what a reference's InitItem builds in its inventory: 00485E00 the container changes, 00488420 the leveled items through 0046CDE0 and 0046CC70, 0048E060 the scripted items through 004FBE00.
//!
//! The script interpreter under 004FBE00 is `script_vm`. No row sees any of this: the extra list sits past the 0x40 bytes a sample holds, so it stands on the exe alone. Evidence: objdump of each address named here.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::actor::{Actor, MobileObject, TesObjectRefr};
use super::actor_process::HighProcess;
use super::inventory_entries::entry_is_worn;
use crate::tes::base_extra_list::{
    EXTRA_CONTAINER_CHANGES, EXTRA_LEVELED_ITEM, ExtraData, ExtraDataList, attach_and_run,
    lists_differ, new_extra_entry, set_extra_count, stack_count,
};
use crate::tes::data_load::DataLoadServices;
use crate::tes::form::FormIdentity;
use crate::tes::form_components::{
    FormRef, TesActorBaseData, TesContainer, TesContainerEntry, TesLeveledList, TesScriptableForm,
    calculate_items, entry_item, item_weight,
};
use crate::tes::form_hierarchy::RttiClass;
use crate::tes::forms_init::{
    cast, casts_to, effective_level, form_type, lookup, record_form_change,
};
use crate::tes::forms_references::{
    construct_reference, form_flags, set_parent_cell, set_reference_base_form,
    set_reference_position,
};
use crate::tes::forms_script::{mark_temporary, run_script};

/// IDA: 00484420 an inventory entry (0x0C): +0x00 the extend data lists (null until one comes), +0x04 the count, +0x08 the form.
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryEntry {
    pub lists_00: Option<Vec<ExtraDataList>>,
    pub count_04: i32,
    pub form_08: Entity,
}

/// IDA: 00485D50 ExtraContainerChanges' block (0x10): +0x00 the entry list, +0x04 the owner, +0x08 and +0x0C -1.0.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerChanges {
    pub entries_00: Vec<InventoryEntry>,
    pub owner_04: Entity,
    pub float_08: f32,
    pub float_0c: f32,
}

/// IDA: 00B34428, the temporary reference 00485D50 makes once, and the host's counts of its boundaries.
#[derive(Resource, Debug, Default)]
pub struct InventoryGlobals {
    pub temporary_reference_b34428: Option<Entity>,
    /// IDA: N/A the calls of 00517810 that 004FBE00 ran past its size test.
    pub script_runs_not_executed: u32,
    /// IDA: N/A the doors 00680090 would have given the node map.
    pub pathing_doors_not_registered: u32,
    /// IDA: N/A the 00485E00 creations.
    pub container_changes_created: u32,
    /// IDA: N/A the 0048E740 calls.
    pub container_script_passes: u32,
    /// IDA: N/A 0048E740's 004FBE00 calls.
    pub container_item_script_runs: u32,
    /// IDA: N/A 004D7190's 004FBE00 calls.
    pub reference_script_runs: u32,
}

// IDA: 00A30634, -1.0.
const MINUS_ONE: f32 = -1.0;

// IDA: 004D9A70 then 0046B590: a temporary reference. Its TESForm constructor runs with TLS +0x184 clear past 0044F95A, so it takes a dynamic ID and 0046B2C0 gives it back.
fn construct_temporary_reference(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_reference(world)?;
    mark_temporary(world, form)?;
    Ok(form)
}

/// IDA: 004D6D40 the base container: vt+0x170's CONT +0x24, or NPC_ and CREA +0x44, or null. Completion: 100%.
#[must_use]
pub fn reference_container(world: &World, reference: Entity) -> Option<Entity> {
    let base = world.get::<TesObjectRefr>(reference)?.base_form_1c?;
    matches!(form_type(world, base), 0x17 | 0x23 | 0x24).then_some(base)
}

/// IDA: 00485E00: the 0x1A entry's block, or a new 00485D50 added through 0041EC10; out of the list for the caller, which `put_container_changes` ends. Completion: 100%.
pub fn ensure_container_changes(
    world: &mut World,
    reference: Entity,
) -> Result<Box<ContainerChanges>, GameError> {
    let mut refr = world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference");
    if let Some(entry) = refr.extra_data_44.get_mut(EXTRA_CONTAINER_CHANGES) {
        let held = std::mem::replace(&mut entry.data, ExtraData::Dword(0));
        let ExtraData::ContainerChanges(changes) = held else {
            return Err(GameError::unclassified(
                0x0041E6F0,
                "0041E6F0: an extra 0x1A with no block",
            ));
        };
        return Ok(changes);
    }
    // 00485D5D: FormHeap(8), the list; 00B34428 once.
    world
        .get_resource_or_init::<InventoryGlobals>()
        .container_changes_created += 1;
    if world
        .resource::<InventoryGlobals>()
        .temporary_reference_b34428
        .is_none()
    {
        let temporary = construct_temporary_reference(world)?;
        world
            .resource_mut::<InventoryGlobals>()
            .temporary_reference_b34428 = Some(temporary);
    }
    world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44
        .add(new_extra_entry(
            EXTRA_CONTAINER_CHANGES,
            ExtraData::Dword(0),
        ));
    Ok(Box::new(ContainerChanges {
        entries_00: Vec::new(),
        owner_04: reference,
        float_08: MINUS_ONE,
        float_0c: MINUS_ONE,
    }))
}

/// IDA: N/A the block back in its entry.
pub fn put_container_changes(world: &mut World, reference: Entity, changes: Box<ContainerChanges>) {
    let mut refr = world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference");
    refr.extra_data_44
        .get_mut(EXTRA_CONTAINER_CHANGES)
        .expect("ensure_container_changes added it")
        .data = ExtraData::ContainerChanges(changes);
}

// IDA: 00487E40 ContainerChanges_GetTotalWeight: +0x08 unless it's -1.0 (00487E6A..00487E78: `fcomp`, `test ah, 44h`,
// `jp`, so a NaN answers +0x08 too). Then the owner's base container (004D6D40; its +0x04 bit 0 isn't tested here):
// each item that isn't persistent (vt+0x78 004D7030, flag 0x400; an unresolved item faults), 00470620's weight with
// -1.0 as 0.0, times the count, or with a change entry for the form the count plus its delta (00487F45 `add`, which
// wraps; a zero sum adds nothing); then the changes' entries the container doesn't list (00469920: bit 0 and listed)
// and that aren't persistent, 00470620's weight unclamped, worn armor on an actor owner (00487FE0..00488005) a stop,
// and `count > 0` times the weight (004880A1 `fild`, `fmul`, `fadd`). Each `fimul`/`fmul` and `fadd` rounds to a float
// at 24 bits. +0x08 = the total (004880C7); an actor owner's process vt+0x290 (004880CC..004880F1): HighProcess
// 006298C0 sets +0x294 to -1.0, the others' 0060D0A0 is a bare return. Completion: 100% past the worn-armor arm.
// TODO: [ ] 00487FF2 the worn armor reduction (005F23B0's skill levels, the B374D8 / B374E0 / B374E8 settings).
pub fn container_changes_total_weight(
    world: &mut World,
    changes: &mut ContainerChanges,
) -> Result<f32, GameError> {
    if changes.float_08 != MINUS_ONE {
        return Ok(changes.float_08);
    }
    let mut total = 0.0_f32;
    let container = reference_container(world, changes.owner_04)
        .and_then(|base| world.get::<TesContainer>(base))
        .cloned();
    let find_change = |item: Entity| {
        changes
            .entries_00
            .iter()
            .find(|entry| entry.form_08 == item)
    };
    if let Some(container) = &container {
        for entry in &container.entries_08 {
            let item = entry_item(entry.item)?;
            if form_flags(world, item) & 0x400 != 0 {
                continue;
            }
            let mut weight = item_weight(world, item)?;
            if weight == MINUS_ONE {
                weight = 0.0;
            }
            let count = match find_change(item) {
                Some(change) => {
                    let count = entry.count.wrapping_add(change.count_04);
                    if count == 0 {
                        continue;
                    }
                    count
                }
                None => entry.count,
            };
            total += (f64::from(weight) * f64::from(count)) as f32;
        }
    }
    let actor_owner = world.get::<Actor>(changes.owner_04).is_some();
    for entry in &changes.entries_00 {
        let item = entry.form_08;
        let in_base = container.as_ref().is_some_and(|container| {
            container.flags_04 & 1 != 0
                && container
                    .entries_08
                    .iter()
                    .any(|base| matches!(base.item, FormRef::Resolved(Some(form)) if form == item))
        });
        if in_base || form_flags(world, item) & 0x400 != 0 {
            continue;
        }
        let weight = item_weight(world, item)?;
        if actor_owner && form_type(world, item) == 0x14 && entry_is_worn(entry, false) {
            return Err(GameError::unclassified(
                0x00487FF2,
                "00487FF2: the worn armor weight",
            ));
        }
        if entry.count_04 > 0 {
            total += (f64::from(entry.count_04) * f64::from(weight)) as f32;
        }
    }
    changes.float_08 = total;
    if actor_owner
        && let Some(process) = world
            .get::<MobileObject>(changes.owner_04)
            .and_then(|mobile| mobile.process_58)
        && let Some(mut high) = world.get_mut::<HighProcess>(process)
    {
        high.cached_encumbrance_294 = MINUS_ONE;
    }
    Ok(total)
}

// IDA: the entries of the base container, out of its component.
fn container_entries(world: &World, base: Entity) -> Vec<TesContainerEntry> {
    world
        .get::<TesContainer>(base)
        .map_or_else(Vec::new, |container| container.entries_08.clone())
}

// IDA: 0047DF80(0).
pub(crate) fn random(services: &mut DataLoadServices<'_>) -> Result<i32, GameError> {
    services.host.random_next().ok_or(GameError::unclassified(
        0x0047DF9D,
        "0047DF9D: 0047DF80 with no random source",
    ))
}

// IDA: 0041FF60 ExtraLeveledItem's index, or -1.
fn leveled_index(list: &ExtraDataList) -> u32 {
    match list.get(EXTRA_LEVELED_ITEM).map(|entry| &entry.data) {
        Some(ExtraData::LeveledItem { index_0c, .. }) => *index_0c,
        _ => u32::MAX,
    }
}

// IDA: 0048F7C0 ExtraContainerChanges_AddItem(form, list, count), as 004699C0 reaches it. +0x08 = -1.0; an empty list is dropped; a zero count is 1. With no entry of the form: 00484420, the list prepended, 0048AE30 appends it (0067B1E0), with vt+0x40(0x8000000) first. With one: a negative count stops; the list adds to a matching stack, or prepends; then the count takes it, 00469CA0's base count deciding a negative's replacement. An actor owner's process vt+0xF4 answers null on Low and MiddleLow; 0x27-owned and worn arms need extras this list hasn't.
pub(crate) fn add_item(
    world: &mut World,
    changes: &mut ContainerChanges,
    base: Entity,
    form: Entity,
    list: ExtraDataList,
    count: i32,
) -> Result<(), GameError> {
    changes.float_08 = MINUS_ONE;
    let list = (!list.entries.is_empty()).then_some(list);
    let count = if count == 0 { 1 } else { count };
    let Some(at) = changes
        .entries_00
        .iter()
        .position(|entry| entry.form_08 == form)
    else {
        crate::tes::forms_items::set_change_flags(world, changes.owner_04, 0x800_0000)?;
        changes.entries_00.push(InventoryEntry {
            lists_00: list.map(|list| vec![list]),
            count_04: count,
            form_08: form,
        });
        return Ok(());
    };
    if changes.entries_00[at].count_04 < 0 {
        return Err(GameError::unclassified(
            0x0048F9A8,
            "0048F9A8: a removed inventory entry",
        ));
    }
    if let Some(list) = list {
        let lists = changes.entries_00[at].lists_00.get_or_insert_with(Vec::new);
        let mut matched = false;
        for existing in lists.iter_mut() {
            if lists_differ(&list, existing)? {
                continue;
            }
            let total = (count as u16).wrapping_add(stack_count(existing) as u16);
            set_extra_count(existing, total);
            if existing.entries.is_empty() {
                return Err(GameError::unclassified(
                    0x0048FA3B,
                    "0048FA3B: an emptied stack",
                ));
            }
            matched = true;
            break;
        }
        if !matched {
            lists.insert(0, list);
        }
    }
    // 0048FB31 00469CA0: the base container's count of the form.
    let base_count = container_entries(world, base)
        .iter()
        .find(|entry| entry.item == FormRef::Resolved(Some(form)))
        .map_or(0, |entry| entry.count);
    let entry = &mut changes.entries_00[at];
    if entry.count_04 >= 0 || base_count > 0 {
        entry.count_04 = entry.count_04.wrapping_add(count);
    } else {
        entry.count_04 = count;
    }
    // 0048FB5A: an entry left with an empty list and no count goes.
    if entry.count_04 == 0 && entry.lists_00.as_ref().is_some_and(Vec::is_empty) {
        changes.entries_00.remove(at);
    }
    Ok(())
}

/// IDA: 00488420 the leveled items of the base container. The level is the lower of ID 7's 004677F0 and the owner's, when the owner's base casts to TESActorBase. Each entry whose form is type 0x2B and without a stack holding its index (004883D0) calculates (0046CDE0, the count's absolute value) and adds through 004699C0: for each result a new list with 0041FE90's index and 004238A0's count, 0048F7C0, and with any, the owner's vt+0x48(0x8000000). The index counts TESLevItem entries only: 0048855A is on their path. Completion: 100%.
pub fn resolve_leveled_items(
    world: &mut World,
    reference: Entity,
    changes: &mut ContainerChanges,
    services: &mut DataLoadServices<'_>,
) -> Result<(), GameError> {
    let Some(base) = reference_container(world, reference) else {
        return Ok(());
    };
    let player_base = cast(world, lookup(world, 7), RttiClass::TESNPC).ok_or(
        GameError::unclassified(0x00488481, "00488481: no TESNPC at ID 7 faults"),
    )?;
    let mut level = effective_level(
        world,
        world.get::<TesActorBaseData>(player_base).expect("an NPC"),
    )? as u16;
    if casts_to(world, base, RttiClass::TESActorBase) {
        let own = effective_level(
            world,
            world.get::<TesActorBaseData>(base).expect("an actor"),
        )?;
        // 004884C2 `cmp ax, di`, signed.
        if own < level as i16 {
            level = own as u16;
        }
    }
    let mut index = 0_u32;
    for entry in container_entries(world, base) {
        let FormRef::Resolved(Some(item)) = entry.item else {
            continue;
        };
        if form_type(world, item) != 0x2B {
            continue;
        }
        let held = changes.entries_00.iter().any(|inventory| {
            inventory
                .lists_00
                .iter()
                .flatten()
                .any(|list| leveled_index(list) == index)
        });
        if !held {
            let mut results = Vec::new();
            let list = world.get::<TesLeveledList>(item).expect("a TESLevItem");
            calculate_items(
                world,
                list,
                level,
                entry.count.unsigned_abs() as u16,
                &mut results,
                services,
            )?;
            let any = !results.is_empty();
            for (count, form) in results {
                let mut stack = ExtraDataList::default();
                stack.add(new_extra_entry(
                    EXTRA_LEVELED_ITEM,
                    ExtraData::LeveledItem {
                        index_0c: index,
                        byte_10: 0,
                    },
                ));
                set_extra_count(&mut stack, count as u16);
                add_item(world, changes, base, form, stack, count)?;
            }
            if any {
                record_form_change(world, reference, 0x800_0000)?;
            }
        }
        // 0048855A.
        index += 1;
    }
    Ok(())
}

// IDA: the scriptable part's script of a form: the TESScriptableForm cast's +0x04.
fn script_of(world: &World, form: Entity) -> Option<Entity> {
    match world.get::<TesScriptableForm>(form)?.script_04 {
        FormRef::Resolved(script) => script,
        FormRef::Raw(_) => None,
    }
}

// IDA: N/A a stack's event list, out for a run: 0041F7D0.
fn take_stack_events(
    list: &mut ExtraDataList,
) -> Option<crate::tes::forms_script::ScriptEventList> {
    match &mut list.get_mut(0x12)?.data {
        ExtraData::Script { events_10, .. } => events_10.take(),
        _ => None,
    }
}

// IDA: N/A the list back after the run.
fn put_stack_events(
    list: &mut ExtraDataList,
    events: Option<crate::tes::forms_script::ScriptEventList>,
) {
    if let Some(entry) = list.get_mut(0x12)
        && let ExtraData::Script { events_10, .. } = &mut entry.data
    {
        *events_10 = events;
    }
}

// IDA: the shape of 0048E14B..0048E1A4: a temporary reference, 0041E920 the stack's events, 004FBE00, vt+0x10(1).
pub(crate) fn run_with_temporary(
    world: &mut World,
    script: Entity,
    list: &mut ExtraDataList,
) -> Result<(), GameError> {
    let temporary = construct_temporary_reference(world)?;
    let mut events = take_stack_events(list);
    let result = run_script(
        world,
        script,
        Some(temporary),
        events.as_mut(),
        Some(temporary),
    )
    .map(|_| ());
    put_stack_events(list, events);
    world.despawn(temporary);
    result
}

// IDA: a stack's 0041E900: whether its 0x12 entry holds a script.
fn stack_has_script(list: &ExtraDataList) -> bool {
    matches!(
        list.get(0x12).map(|entry| &entry.data),
        Some(ExtraData::Script { .. })
    )
}

// IDA: the part 0048E1F4..0048E3CE and 0048E4A9..0048E692 share: each stack with no script takes one and runs, counting down; then that many new stacks, each prepended, with a count of 1 (which stores nothing), the script and a run.
fn fill_entry(
    world: &mut World,
    entry: &mut InventoryEntry,
    script: Entity,
    count: i32,
) -> Result<(), GameError> {
    let lists = entry.lists_00.get_or_insert_with(Vec::new);
    let mut count = count;
    for list in lists.iter_mut() {
        if !stack_has_script(list) {
            attach_and_run(world, list, script)?;
            count -= 1;
        }
    }
    // 0048E2C5, 0048E585 `test ebx,ebx; jbe` test only zero, so a negative count would loop about 2^32 times.
    if count < 0 {
        return Err(GameError::unclassified(
            0x0048E2C5,
            "0048E2C5: a negative stack count",
        ));
    }
    for _ in 0..count {
        let mut list = ExtraDataList::default();
        set_extra_count(&mut list, 1);
        attach_and_run(world, &mut list, script)?;
        lists.insert(0, list);
    }
    Ok(())
}

/// IDA: 0048E060 the scripted items. For each base container entry whose item has a script and a positive count: an entry whose stacks hold one (00484F20) runs the script per stack; otherwise the stacks fill, a new entry (00484420 with count 0) going through 0048AE30 after. Then each changes entry the same way, with no new entry. Completion: 100%.
pub fn run_item_scripts(
    world: &mut World,
    reference: Entity,
    changes: &mut ContainerChanges,
) -> Result<(), GameError> {
    if let Some(base) = reference_container(world, reference) {
        for base_entry in container_entries(world, base) {
            let FormRef::Resolved(Some(item)) = base_entry.item else {
                continue;
            };
            let count = base_entry.count.wrapping_abs();
            let Some(script) = script_of(world, item) else {
                continue;
            };
            if count <= 0 {
                continue;
            }
            let at = changes
                .entries_00
                .iter()
                .position(|entry| entry.form_08 == item);
            if let Some(at) = at {
                let scripted = changes.entries_00[at]
                    .lists_00
                    .iter()
                    .flatten()
                    .any(stack_has_script);
                if scripted {
                    for list in changes.entries_00[at].lists_00.iter_mut().flatten() {
                        run_with_temporary(world, script, list)?;
                    }
                    continue;
                }
                let mut entry = changes.entries_00[at].clone();
                fill_entry(world, &mut entry, script, count)?;
                changes.entries_00[at] = entry;
            } else {
                let mut entry = InventoryEntry {
                    lists_00: None,
                    count_04: 0,
                    form_08: item,
                };
                fill_entry(world, &mut entry, script, count)?;
                // 0048E3E6 0048AE30(entry, 1).
                changes.float_08 = MINUS_ONE;
                crate::tes::forms_items::set_change_flags(world, changes.owner_04, 0x800_0000)?;
                changes.entries_00.push(entry);
            }
        }
    }
    let mut at = 0;
    while at < changes.entries_00.len() {
        let form = changes.entries_00[at].form_08;
        let count = changes.entries_00[at].count_04.wrapping_abs();
        if let Some(script) = script_of(world, form)
            && count > 0
        {
            let scripted = changes.entries_00[at]
                .lists_00
                .iter()
                .flatten()
                .any(stack_has_script);
            if scripted {
                for list in changes.entries_00[at].lists_00.iter_mut().flatten() {
                    run_with_temporary(world, script, list)?;
                }
            } else {
                let mut entry = changes.entries_00[at].clone();
                fill_entry(world, &mut entry, script, count)?;
                changes.entries_00[at] = entry;
            }
        }
        at += 1;
    }
    Ok(())
}

/// IDA: 0048E740(changes, reference): the scripted stacks of a container, run on the 00B34428 temporary reference. A null reference returns 0. The entry count is read twice, then a null parent cell (006ECC80) returns 0. The temporary takes the cell (vt+0x194) and the position (004D8A30). Per entry, until a null item: the stack count, then per stack until a null: 0041E900's script; none, or not initialized, goes to the next stack with nothing compared. A form of type 0x0D: 004D7620 (the stack's copyable extras), 006C5420 the entry's form, 0041E920 the events, 004FBE00(script, temporary, events, reference, 0), the entry recount, and only if it's unchanged the stack recount; 0041F130(0) and 0041E3D0(1) on the temporary. Then a stack count that differs from the first ends the entry's walk, and an entry count that differs ends it too. After a walk, a changed entry count recounts and goes back to the list's head (0048E935), so the next entry is the head's next. Last, vt+0x194(0) and a null base. Returns whether a run answered nonzero. The boundary can't change either count, so those arms only run with an interpreter. Completion: 100% of the control flow; the temporary's extras are the gap below.
/// TODO: [ ] 004D7620 (00428920, 004282A0) on the temporary: a transient copy, whose absence only the 0x13 byte's start could show; the rest was written while the interpreter was a boundary. RUN-002's after dump holds presence bits 0x12, 0x13, 0x2A, 0x36 left stale by it, which the host doesn't have, and without an interpreter it couldn't match them (0x13 comes from script commands).
/// # Errors
/// A stop in 004FBE00, vt+0x194, or 004D8A30.
pub fn run_container_item_scripts(
    world: &mut World,
    changes: &mut ContainerChanges,
    reference: Option<Entity>,
) -> Result<bool, GameError> {
    world
        .get_resource_or_init::<InventoryGlobals>()
        .container_script_passes += 1;
    let Some(reference) = reference else {
        return Ok(false);
    };
    let (cell, position) = {
        let refr = world.get::<TesObjectRefr>(reference).expect("a reference");
        (refr.parent_cell_40, refr.position_2c)
    };
    let mut first_count = changes.entries_00.len();
    let mut last_count = first_count;
    let Some(cell) = cell else {
        return Ok(false);
    };
    let temporary = world
        .resource::<InventoryGlobals>()
        .temporary_reference_b34428
        .ok_or(GameError::unclassified(
            0x0048E7A8,
            "0048E7A8: a null 00B34428",
        ))?;
    set_parent_cell(world, temporary, Some(cell))?;
    set_reference_position(world, temporary, position)?;
    let mut answered = false;
    let mut at = 0;
    while at < changes.entries_00.len() {
        let form = changes.entries_00[at].form_08;
        let stacks = changes.entries_00[at].lists_00.as_ref().map_or(0, Vec::len);
        let mut stacks_after = 0;
        for stack_at in 0..stacks {
            let stack = &mut changes.entries_00[at]
                .lists_00
                .as_mut()
                .expect("counted above")[stack_at];
            let script = match stack.get(0x12).map(|extra| &extra.data) {
                Some(ExtraData::Script { script_0c, .. }) => Some(*script_0c),
                _ => None,
            };
            let Some(script) = script else { continue };
            let identity = *world.get::<FormIdentity>(script).expect("a form");
            if identity.flags_08 & 8 == 0 {
                continue;
            }
            if identity.form_type_04 == 0x0D {
                set_reference_base_form(world, temporary, Some(form));
                world
                    .resource_mut::<InventoryGlobals>()
                    .container_item_script_runs += 1;
                let mut events = take_stack_events(stack);
                let result = run_script(
                    world,
                    script,
                    Some(temporary),
                    events.as_mut(),
                    Some(temporary),
                );
                put_stack_events(stack, events);
                // 0041E3D0: the temporary's copy of the stack's extras goes after each run.
                world
                    .get_mut::<TesObjectRefr>(temporary)
                    .expect("a reference")
                    .extra_data_44
                    .remove_all();
                if result? {
                    answered = true;
                }
                last_count = changes.entries_00.len();
                if first_count == last_count {
                    stacks_after = changes.entries_00[at].lists_00.as_ref().map_or(0, Vec::len);
                }
            }
            if stacks_after != stacks || first_count != last_count {
                break;
            }
        }
        if first_count != last_count {
            first_count = changes.entries_00.len();
            last_count = first_count;
            at = 0;
        }
        at += 1;
    }
    set_parent_cell(world, temporary, None)?;
    set_reference_base_form(world, temporary, None);
    Ok(answered)
}

/// IDA: 00485FA0 ContainerChanges_FindWornStackForForm(changes, form, take_locked): the form's entry's first stack worn
/// (0041DF10(0)); with `take_locked`, an earlier unworn stack holding 0x47 takes ExtraWorn (0041F200(1, 0)) and answers.
/// Answers the entry and stack indexes. Completion: 100%.
pub(crate) fn find_worn_stack_for_form(
    changes: &mut ContainerChanges,
    form: Entity,
    take_locked: bool,
) -> Option<(usize, usize)> {
    let at = changes
        .entries_00
        .iter()
        .position(|entry| entry.form_08 == form)?;
    let lists = changes.entries_00[at].lists_00.as_mut()?;
    for (index, list) in lists.iter_mut().enumerate() {
        if list.is_worn() {
            return Some((at, index));
        }
        if list.present(crate::tes::base_extra_list::EXTRA_CANNOT_WEAR) && take_locked {
            // 0041F200(1, 0): 0x1B unless its bit is set.
            list.add(new_extra_entry(
                crate::tes::base_extra_list::EXTRA_WORN,
                ExtraData::Marker,
            ));
            return Some((at, index));
        }
    }
    None
}

/// IDA: 004869C0 ContainerChanges_GetItemCount(changes, form): |00469CA0 the base container's first entry of the item| (the
/// container of `owner`, the changes' +0x04), plus the changes entry's +0x04 (`entries` is the changes' +0x00 list); with a
/// changes entry and both zero, 1. Completion: 100%.
pub(crate) fn container_item_count(
    world: &World,
    owner: Entity,
    entries: &[InventoryEntry],
    form: Entity,
) -> i32 {
    let base = reference_container(world, owner)
        .and_then(|base| world.get::<crate::tes::form_components::TesContainer>(base))
        .and_then(|container| {
            container
                .entries_08
                .iter()
                .find(|entry| {
                    entry.item == crate::tes::form_components::FormRef::Resolved(Some(form))
                })
                .map(|entry| entry.count)
        })
        .unwrap_or(0)
        .wrapping_abs();
    match entries.iter().find(|entry| entry.form_08 == form) {
        Some(entry) if base == 0 && entry.count_04 == 0 => 1,
        Some(entry) => base.wrapping_add(entry.count_04),
        None => base,
    }
}

/// IDA: 004D8720 TESObjectREFR_AddInventoryItem(reference, item, stack, count): with a base container, 004FBF90(reference,
/// stack, 1) and 0048F7C0 ExtraContainerChanges_AddItem on the changes 00485E00 makes. 0048F7C0's actor arm: the owner's
/// process vt+0xF4(0) (the equipped ammo) of the same form adds the count to its stack, a stop here; a stack owned by the
/// player is the player's only. Completion: an actor whose equipped ammo isn't the item.
pub(crate) fn reference_add_inventory_item(
    world: &mut World,
    reference: Entity,
    item: Entity,
    mut stack: Option<ExtraDataList>,
    count: i32,
) -> Result<(), GameError> {
    let Some(base) = reference_container(world, reference) else {
        return Ok(());
    };
    // 004D8738: 004FBF90(reference, stack, 1), its answer unused.
    if let Some(stack) = stack.as_mut() {
        let id = world
            .get::<FormIdentity>(reference)
            .expect("a form")
            .form_id_0c;
        crate::tes::base_extra_list::extra_list_script_event_or(stack, id, 1);
    }
    let process = world
        .get::<super::actor::MobileObject>(reference)
        .and_then(|mobile| mobile.process_58);
    if let Some(process) = process
        && !crate::tes::forms_references::save_load_active(world)?
        && world
            .get::<super::actor_process::MiddleHighProcess>(process)
            .and_then(|middle| middle.ammo_ec)
            .is_some_and(|ammo| ammo.form_08 == item)
    {
        return Err(GameError::stop(
            0x0048F87A,
            "adding to the equipped ammo's stack",
        ));
    }
    let mut changes = ensure_container_changes(world, reference)?;
    let result = super::inventory::add_item(
        world,
        &mut changes,
        base,
        item,
        stack.unwrap_or_else(ExtraDataList::constructed),
        count,
    );
    put_container_changes(world, reference, changes);
    result
}

#[cfg(test)]
mod stack_tests {
    use super::*;
    use crate::tes::base_extra_list::{BaseExtraEntry, ExtraData};

    fn leveled(index_0c: u32) -> ExtraDataList {
        let mut list = ExtraDataList::default();
        list.add(BaseExtraEntry {
            original_vtable: 0x00A3_5938,
            type_04: EXTRA_LEVELED_ITEM,
            data: ExtraData::LeveledItem {
                index_0c,
                byte_10: 0,
            },
            resolved_0c: None,
        });
        list
    }

    // 00429140 calls 0041DF70, which compares only +0x04.
    #[test]
    fn two_leveled_items_never_differ_whatever_their_index() {
        assert!(!lists_differ(&leveled(1), &leveled(2)).unwrap());
        assert!(lists_differ(&ExtraDataList::default(), &leveled(2)).unwrap());
    }
}
