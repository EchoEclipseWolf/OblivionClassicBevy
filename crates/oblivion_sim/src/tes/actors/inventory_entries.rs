//! A reference's inventory as the interface walks it: 004D8950 the entry count and 004D88F0 an entry by index, over
//! ExtraContainerChanges' block (0048D6C0, 00486A40) merged with the base container. An entry made here is a new
//! InventoryEntry whose extra lists are the block's own (shared pointers in the original, clones here); the caller frees
//! it (00484470). The extra-list tests are the ones those two read.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;
use oblivion_core::x87::round_to;

use super::inventory::{
    ContainerChanges, InventoryEntry, ensure_container_changes, put_container_changes,
    reference_container,
};
use super::player::PlayerCharacter;
use crate::tes::base_extra_list::{
    EXTRA_COUNT, EXTRA_OWNER, default_for_container, extra_form, leveled, owned_default,
    stack_count, worn,
};
use crate::tes::form::persistent;
use crate::tes::form_components::{
    FormRef, TesBipedModelForm, TesContainer, TesContainerEntry, TesEnchantableForm, TesFullName,
    TesHealthForm, TesQualityForm, TesUsesForm, TesValueForm, TesWeightForm, container_item_count,
    container_lists_form,
};
use crate::tes::forms_init::form_type;

/// IDA: the form type 00486A40 skips in the base container (`cmp byte [eax+4], 2Bh`) and 0048D6C0 casts away: TESLevItem.
const FORM_TYPE_LEVELED_ITEM: u8 = 0x2B;

// IDA: 00484660: the lists that aren't default for a container (0041FD70(0)); a list of one extra with a stack above 1
// loses its ExtraCount (0041F620), leaves the entry and is deleted, and the count starts over.
fn entry_non_default_lists(entry: &mut InventoryEntry) -> i32 {
    let Some(lists) = entry.lists_00.as_mut() else {
        return 0;
    };
    let mut count = 0;
    let mut at = 0;
    while at < lists.len() {
        if !default_for_container(&lists[at], false) {
            count += 1;
        }
        if lists[at].entries.len() == 1 && stack_count(&lists[at]) > 1 {
            lists[at].remove_by_type(EXTRA_COUNT);
            lists.remove(at);
            at = 0;
            count = 0;
            continue;
        }
        at += 1;
    }
    count
}

// IDA: 004845D0: the stack counts of the lists that are neither default (0041FD70(0)) nor worn (0041DF10(0)).
pub(crate) fn entry_loose_count(entry: &InventoryEntry) -> i32 {
    entry
        .lists_00
        .iter()
        .flatten()
        .filter(|list| !default_for_container(list, false) && !worn(list, false))
        .map(stack_count)
        .sum()
}

/// IDA: 00484E80(left): a list that's worn (0041DF10).
#[must_use]
pub fn entry_is_worn(entry: &InventoryEntry, left_only: bool) -> bool {
    entry
        .lists_00
        .iter()
        .flatten()
        .any(|list| worn(list, left_only))
}

// IDA: 004846D0: a list with 0041DEF0.
fn entry_has_leveled(entry: &InventoryEntry) -> bool {
    entry.lists_00.iter().flatten().any(leveled)
}

// IDA: 00484700: a list with an owner (0041E7B0) that's worn.
fn entry_owned_worn(entry: &InventoryEntry) -> Result<bool, GameError> {
    for list in entry.lists_00.iter().flatten() {
        if extra_form(list, EXTRA_OWNER)?.is_some() && worn(list, false) {
            return Ok(true);
        }
    }
    Ok(false)
}

// IDA: 00484780: the stack counts of the 0041FCB0 lists.
fn entry_owned_default_count(entry: &InventoryEntry) -> i32 {
    entry
        .lists_00
        .iter()
        .flatten()
        .filter(|list| owned_default(list))
        .map(stack_count)
        .sum()
}

// IDA: 004847C0: a 0041FCB0 list.
fn entry_has_owned_default(entry: &InventoryEntry) -> bool {
    entry.lists_00.iter().flatten().any(owned_default)
}

// IDA: 00484EC0(left): the first worn list decides: default (0041FD70(0)) with no owner; none answers true.
fn entry_worn_is_plain(entry: &InventoryEntry, left_only: bool) -> Result<bool, GameError> {
    match entry
        .lists_00
        .iter()
        .flatten()
        .find(|list| worn(list, left_only))
    {
        Some(list) => {
            Ok(default_for_container(list, false) && extra_form(list, EXTRA_OWNER)?.is_none())
        }
        None => Ok(true),
    }
}

// IDA: 00484F20: a list with a script (0x12).
fn entry_has_script(entry: &InventoryEntry) -> bool {
    entry
        .lists_00
        .iter()
        .flatten()
        .any(|list| list.get(0x12).is_some())
}

// IDA: N/A a base entry's form.
pub(crate) fn base_form(entry: &TesContainerEntry) -> Option<Entity> {
    match entry.item {
        FormRef::Resolved(form) => form,
        _ => None,
    }
}

// IDA: N/A the block's entry of a form, by index (the first match).
fn changes_entry_of(changes: &ContainerChanges, form: Entity) -> Option<usize> {
    changes
        .entries_00
        .iter()
        .position(|entry| entry.form_08 == form)
}

// IDA: 00484420 InventoryEntry(form, count): a new empty list head (FormHeap(8)), the count.
fn new_entry(form: Entity, count: i32) -> InventoryEntry {
    InventoryEntry {
        lists_00: Some(Vec::new()),
        count_04: count,
        form_08: form,
    }
}

/// IDA: 0048D6C0 ContainerChanges_CountEntries: over the owner's base container (no leveled item): an item with no block
/// entry is one; with one whose count and the base's sum above 0 (and the base not negative, or the entry above it), an
/// entry with no lists is one, else its non-default lists (00484660), one more for an owned default list (004847C0), and
/// one more when the loose count (004845D0) is under the sum and nothing's worn. Then each block entry with a count above
/// 0 that the container doesn't list (00469920): its non-default lists, one for the first owned default list, one when the
/// loose count is under its count. The answer is one more than the total (0048D8EE). Completion: 100% but the
/// unreachable negative-count arm of the second loop.
pub fn container_changes_count(world: &World, changes: &mut ContainerChanges) -> i32 {
    let container = reference_container(world, changes.owner_04)
        .and_then(|base| world.get::<TesContainer>(base))
        .cloned();
    let mut total = 0;
    if let Some(container) = &container {
        for item in &container.entries_08 {
            let Some(form) = base_form(item) else {
                break;
            };
            if form_type(world, form) == FORM_TYPE_LEVELED_ITEM {
                continue;
            }
            let base_count = item.count;
            let magnitude = base_count.abs();
            let Some(index) = changes_entry_of(changes, form) else {
                total += 1;
                continue;
            };
            let entry = &mut changes.entries_00[index];
            let count = entry.count_04;
            if count + magnitude > 0 && (base_count >= 0 || count > base_count) {
                if entry.lists_00.as_ref().is_none_or(Vec::is_empty) {
                    total += 1;
                    continue;
                }
                total += entry_non_default_lists(entry);
                if entry_has_owned_default(entry) {
                    total += 1;
                }
                if entry_loose_count(entry) < base_count + entry.count_04
                    && !entry_is_worn(entry, false)
                {
                    total += 1;
                }
            }
        }
    }
    for index in 0..changes.entries_00.len() {
        let entry = &mut changes.entries_00[index];
        if entry.count_04 <= 0 {
            continue;
        }
        if container
            .as_ref()
            .is_some_and(|container| container_lists_form(container, entry.form_08))
        {
            continue;
        }
        total += entry_non_default_lists(entry);
        if entry.lists_00.iter().flatten().any(owned_default) {
            total += 1;
        }
        if entry_loose_count(entry) < entry.count_04 {
            total += 1;
        }
    }
    total + 1
}

// IDA: 00486A40's copy of an entry's lists when it holds a script (00484F20) and a list: prepended, so in reverse.
fn copy_scripted_lists(target: &mut InventoryEntry, source: &InventoryEntry) {
    if entry_has_script(source)
        && source
            .lists_00
            .as_ref()
            .is_some_and(|lists| !lists.is_empty())
    {
        let lists = target.lists_00.get_or_insert_with(Vec::new);
        for list in source.lists_00.iter().flatten() {
            lists.insert(0, list.clone());
        }
    }
}

/// IDA: 00486A40 ContainerChanges_GetEntryAt(index): the owner's base container first, then the block's own entries,
/// counting each the way 0048D6C0 does, and the index's entry made new (00484420, or a zeroed 0x0C block whose list is
/// made on demand): a base item's count merged with its block entry, or one of the entry's lists with its stack. Null when
/// the index is past the end. Completion: 100%.
/// # Errors
/// None yet; kept for the arms that touch the extra lists' setters.
#[allow(clippy::too_many_lines)]
pub fn container_changes_entry(
    world: &World,
    changes: &mut ContainerChanges,
    index: i32,
) -> Result<Option<InventoryEntry>, GameError> {
    let container = reference_container(world, changes.owner_04)
        .and_then(|base| world.get::<TesContainer>(base))
        .cloned();
    let mut seen = 0_i32;
    if let Some(container) = &container {
        for item in &container.entries_08 {
            let Some(form) = base_form(item) else {
                break;
            };
            if form_type(world, form) == FORM_TYPE_LEVELED_ITEM {
                continue;
            }
            let found = changes_entry_of(changes, form);
            let base_count = item.count;
            let magnitude = base_count.abs();
            if found.is_some() && magnitude <= 0 {
                continue;
            }
            if let Some(at) = found {
                let entry = &changes.entries_00[at];
                if base_count < 0
                    || entry.count_04 + base_count <= 0
                    || entry
                        .lists_00
                        .as_ref()
                        .and_then(|lists| lists.first())
                        .is_some_and(leveled)
                {
                    continue;
                }
            }
            let non_default = found.map(|at| entry_non_default_lists(&mut changes.entries_00[at]));
            if seen == index && non_default.is_none_or(|count| count == 0) {
                let mut made = InventoryEntry {
                    lists_00: None,
                    count_04: 0,
                    form_08: form,
                };
                made.count_04 = match found {
                    Some(at) => {
                        let entry = &mut changes.entries_00[at];
                        if base_count >= 0 {
                            entry.count_04 + base_count - entry_non_default_lists(entry)
                        } else {
                            entry.count_04 + magnitude
                        }
                    }
                    None => magnitude,
                };
                if let Some(at) = found {
                    copy_scripted_lists(&mut made, &changes.entries_00[at]);
                }
                return Ok(Some(made));
            }
            let Some(at) = found else {
                seen += 1;
                continue;
            };
            let lists_count = non_default.unwrap_or(0);
            if index < seen + lists_count {
                let entry = changes.entries_00[at].clone();
                for list in entry.lists_00.iter().flatten() {
                    if seen == index {
                        if default_for_container(list, false) {
                            seen -= 1;
                        } else {
                            let stack = stack_count(list).min(entry.count_04 + magnitude);
                            let mut made = new_entry(entry.form_08, stack);
                            made.lists_00
                                .get_or_insert_with(Vec::new)
                                .insert(0, list.clone());
                            return Ok(Some(made));
                        }
                    }
                    seen += 1;
                }
                continue;
            }
            seen += entry_non_default_lists(&mut changes.entries_00[at]);
            let entry = &changes.entries_00[at];
            let loose = entry_loose_count(entry);
            let shown = (loose != 0
                || !entry_is_worn(entry, false)
                || (entry.count_04 > 0 && form_type(world, entry.form_08) != 0x22))
                && loose < entry.count_04 + base_count
                && (!entry_has_leveled(entry) || entry.count_04 <= base_count);
            if shown {
                if seen == index {
                    let mut made = InventoryEntry {
                        lists_00: None,
                        count_04: 0,
                        form_08: form,
                    };
                    let entry = &mut changes.entries_00[at];
                    made.count_04 = entry.count_04 + base_count - entry_non_default_lists(entry);
                    copy_scripted_lists(&mut made, &changes.entries_00[at]);
                    return Ok(Some(made));
                }
                seen += 1;
            }
        }
    }
    // 00486AE8 (LABEL_105): the block's entries the container doesn't list.
    for at in 0..changes.entries_00.len() {
        let form = changes.entries_00[at].form_08;
        let mut item_count = 0;
        if let Some(container) = &container {
            item_count = container_item_count(container, form);
            if item_count < 0 {
                return Err(GameError::unclassified(
                    0x00486B45,
                    "00486B45: a negative base count with a worn list (004238A0) is carried",
                ));
            }
        }
        let entry = &changes.entries_00[at];
        if !(entry.count_04 > 0 || item_count < 0) {
            continue;
        }
        let unlisted = entry
            .lists_00
            .as_ref()
            .and_then(|lists| lists.first())
            .is_some_and(leveled)
            || item_count < 0
            || container
                .as_ref()
                .is_none_or(|container| !container_lists_form(container, form));
        if !unlisted {
            continue;
        }
        let has_list = entry
            .lists_00
            .as_ref()
            .is_some_and(|lists| !lists.is_empty());
        let non_default = entry_non_default_lists(&mut changes.entries_00[at]);
        let entry = changes.entries_00[at].clone();
        if has_list && index < seen + non_default {
            for list in entry.lists_00.iter().flatten() {
                let plain = default_for_container(list, false);
                if seen == index {
                    if !plain && stack_count(list) > 0 {
                        let mut made = new_entry(form, stack_count(list));
                        made.lists_00
                            .get_or_insert_with(Vec::new)
                            .insert(0, list.clone());
                        return Ok(Some(made));
                    }
                } else if !plain {
                    seen += 1;
                }
            }
            continue;
        }
        if entry.count_04 <= non_default {
            seen += non_default;
            continue;
        }
        let mut taken = 0;
        let mut owned_found = false;
        for _ in entry
            .lists_00
            .iter()
            .flatten()
            .filter(|list| owned_default(list))
            .take(1)
        {
            owned_found = true;
            if entry_is_worn(&entry, false) && !entry_owned_worn(&entry)? {
                seen += 1;
            }
            if seen == index {
                let mut made = new_entry(form, entry_owned_default_count(&entry));
                for list in entry.lists_00.iter().flatten() {
                    if owned_default(list) {
                        made.lists_00
                            .get_or_insert_with(Vec::new)
                            .push(list.clone());
                    }
                }
                return Ok(Some(made));
            }
            if entry_owned_default_count(&entry) < entry.count_04 + item_count {
                taken = entry_owned_default_count(&entry);
                if entry_owned_worn(&entry)? || !entry_is_worn(&entry, false) {
                    seen += 1;
                }
            }
        }
        if !owned_found && entry.count_04 > non_default && non_default > 0 {
            taken = entry
                .lists_00
                .iter()
                .flatten()
                .filter(|list| !default_for_container(list, false))
                .map(stack_count)
                .sum();
            seen += non_default;
            if entry_is_worn(&entry, false) {
                if !entry_worn_is_plain(&entry, false)? {
                    seen -= 1;
                }
            } else if entry_is_worn(&entry, true) && !entry_worn_is_plain(&entry, true)? {
                seen -= 1;
            }
        }
        if entry_is_worn(&entry, false)
            && entry.count_04 > 1
            && taken != entry.count_04
            && !entry_has_owned_default(&entry)
        {
            seen += 1;
        }
        if seen != index || entry.count_04 - taken <= 0 {
            seen += 1;
            continue;
        }
        let mut made = new_entry(form, entry.count_04 - taken);
        let owned = entry_has_owned_default(&entry);
        for list in entry.lists_00.iter().flatten() {
            if !worn(list, false)
                && default_for_container(list, true)
                && !(owned && owned_default(list))
            {
                made.lists_00
                    .get_or_insert_with(Vec::new)
                    .push(list.clone());
            }
        }
        return Ok(Some(made));
    }
    Ok(None)
}

/// IDA: 004D8950(reference, 0) TESObjectREFR_GetInventoryCount: no base container is -1; else 00485E00 (the block made
/// when missing) and 0048D6C0. Completion: 100% of the block arm.
/// # Errors
/// As `ensure_container_changes`.
pub fn reference_inventory_count(world: &mut World, reference: Entity) -> Result<i32, GameError> {
    if reference_container(world, reference).is_none() {
        return Ok(-1);
    }
    let mut changes = ensure_container_changes(world, reference)?;
    let count = container_changes_count(world, &mut changes);
    put_container_changes(world, reference, changes);
    Ok(count)
}

/// IDA: 004D88F0(reference, index, 0) TESObjectREFR_GetInventoryEntryAt: no base container is null; else 00485E00 and
/// 00486A40. Completion: 100% of the block arm.
/// # Errors
/// As `ensure_container_changes` and `container_changes_entry`.
pub fn reference_inventory_entry(
    world: &mut World,
    reference: Entity,
    index: i32,
) -> Result<Option<InventoryEntry>, GameError> {
    if reference_container(world, reference).is_none() {
        return Ok(None);
    }
    let mut changes = ensure_container_changes(world, reference)?;
    let entry = container_changes_entry(world, &mut changes, index);
    put_container_changes(world, reference, changes);
    entry
}

// IDA: 00485150 InventoryEntry_GetCategory: types 0x13, 0x19, 0x28 4; 0x14, 0x16 2; 0x21, 0x22 1; else 8.
pub(in crate::tes) fn item_category(world: &World, form: Entity) -> i32 {
    match form_type(world, form) {
        0x13 | 0x19 | 0x28 => 4,
        0x14 | 0x16 => 2,
        0x21 | 0x22 => 1,
        _ => 8,
    }
}

// IDA: 00488DF0 InventoryEntry_GetName: the form's TESFullName, its empty or absent text the setting at 00B38D30, null
// data "".
pub(in crate::tes) fn item_name(world: &World, form: Entity) -> Vec<u8> {
    let name = world
        .get::<TesFullName>(form)
        .and_then(|name| name.name_04.data_00.clone())
        .map(|name| {
            let end = name
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(name.len());
            name[..end].to_vec()
        });
    match name {
        Some(name) if !name.is_empty() => name,
        _ => world
            .get_resource::<crate::tes::game_settings::GameSettingCollection>()
            .and_then(|settings| settings.text_at(0x00B3_8D30))
            .unwrap_or_default(),
    }
}

// IDA: 004854F0(player, 0, 1, 1, 0): an armor's or clothing's biped flag 0x40 (not playable, 00469080) refuses; a
// persistent form (vt+0x78) while the player's +0x200 jail flag is set refuses; with a5 set, the rest is skipped.
pub(in crate::tes) fn entry_usable(
    world: &World,
    entry: &InventoryEntry,
    player: Entity,
) -> Result<bool, GameError> {
    let form = entry.form_08;
    if matches!(form_type(world, form), 0x14 | 0x16)
        && world
            .get::<TesBipedModelForm>(form)
            .is_some_and(|biped| biped.flags_06 & 0x40 != 0)
    {
        return Ok(false);
    }
    if persistent(world, form)
        && world
            .get::<PlayerCharacter>(player)
            .is_some_and(|data| data.jail_sentence_active_200 != 0)
    {
        return Ok(false);
    }
    Ok(true)
}

// IDA: 004852B0 InventoryEntry_GetHealthPercent(entry, 1): a TESHealthForm's health percentage (not ported: it stops); a
// TESUsesForm that isn't a sigil stone, 00484970's uses (stops); else -1.0.
pub(in crate::tes) fn item_health_percent(
    world: &World,
    entry: &InventoryEntry,
) -> Result<f32, GameError> {
    if world.get::<TesHealthForm>(entry.form_08).is_some() {
        return Err(GameError::unclassified(
            0x004852B0,
            "004852B0: an item's health percentage (InventoryEntry_GetHealth) is carried",
        ));
    }
    if world.get::<TesUsesForm>(entry.form_08).is_some() && form_type(world, entry.form_08) != 0x2D
    {
        return Err(GameError::unclassified(
            0x004852B0,
            "004852B0: an item's uses (00484970) are carried",
        ));
    }
    Ok(-1.0)
}

// IDA: 0048F450 InventoryEntry_FormatStat(kind, ...): 0 the count ("%i" above 1, " " at or below, the K and M forms
// above 999 and 999999 stop); 1 the value (00488E50), 2 the weight (00485260), 3 the quality (004891C0), 4 the health
// (004852B0): below 0.0 (or NaN) null; between 0 and 1 rounded to tenths (004843B0) and "%.1f" if still between; else
// __ftol2 and "%d".
pub(in crate::tes) fn item_string(
    world: &World,
    entry: &InventoryEntry,
    kind: u32,
) -> Result<Option<Vec<u8>>, GameError> {
    let value = match kind {
        0 => {
            let count = entry.count_04;
            if count > 999 {
                return Err(GameError::unclassified(
                    0x0048F4BE,
                    "0048F4BE: a count above 999's K or M form is carried",
                ));
            }
            return Ok(Some(if count > 1 {
                count.to_string().into_bytes()
            } else {
                b" ".to_vec()
            }));
        }
        1 => item_value(world, entry)?,
        2 => item_weight(world, entry.form_08),
        3 => item_quality(world, entry)?,
        4 => item_health_percent(world, entry)?,
        _ => return Ok(None),
    };
    // 0048F5E7: `fcom` 0.0 with C0 set below it or unordered, so a NaN is null too.
    if value < 0.0 || value.is_nan() {
        return Ok(None);
    }
    let mut value = value;
    if value > 0.0 && value < 1.0 {
        value = round_to(value, 0.1);
    }
    if value > 0.0 && value < 1.0 {
        // 0048F646: "%.1f" of the double. Rust rounds a tie to even.
        return Ok(Some(format!("{:.1}", f64::from(value)).into_bytes()));
    }
    Ok(Some(x87::ftol(value).to_string().into_bytes()))
}

// IDA: 00488E50(entry, 0): a TESValueForm's value (`fild`), unless it's enchantable with an enchantment (00484340, stops);
// a magic item's cost (stops); else -1.0.
pub(in crate::tes) fn item_value(world: &World, entry: &InventoryEntry) -> Result<f32, GameError> {
    let form = entry.form_08;
    let Some(value) = world.get::<TesValueForm>(form) else {
        if world
            .get::<crate::tes::forms_magic::EffectItemList>(form)
            .is_some()
        {
            return Err(GameError::unclassified(
                0x00488E50,
                "00488E50: a magic item's value is carried",
            ));
        }
        return Ok(-1.0);
    };
    if world
        .get::<TesEnchantableForm>(form)
        .is_some_and(|enchantable| {
            !matches!(
                enchantable.enchantment_04,
                FormRef::Resolved(None) | FormRef::Raw(0)
            )
        })
    {
        return Err(GameError::unclassified(
            0x00488E50,
            "00488E50: an enchanted item's value is carried",
        ));
    }
    Ok(value.value_04 as f32)
}

// IDA: 00485260 / 00470620 Form_GetItemWeight: the TESWeightForm's weight, 0.0 for a persistent form; -1.0 with none.
pub(in crate::tes) fn item_weight(world: &World, form: Entity) -> f32 {
    match world.get::<TesWeightForm>(form) {
        Some(_) if persistent(world, form) => 0.0,
        Some(weight) => weight.weight_04,
        None => -1.0,
    }
}

// IDA: 004891C0 InventoryEntry_GetQuality: a TESQualityForm's quality; a weapon (category 1) or armor (0x14) stops;
// else -1.0.
pub(in crate::tes) fn item_quality(
    world: &World,
    entry: &InventoryEntry,
) -> Result<f32, GameError> {
    if let Some(quality) = world.get::<TesQualityForm>(entry.form_08) {
        return Ok(quality.quality_04);
    }
    if item_category(world, entry.form_08) == 1 || form_type(world, entry.form_08) == 0x14 {
        return Err(GameError::unclassified(
            0x004891C0,
            "004891C0: a weapon's or armor's rating is carried",
        ));
    }
    Ok(-1.0)
}

// IDA: 00485C00 InventoryEntry_GetStatusFlags: 1 another owner on the first list, 2 an enchantment, 4 poison (0x48), 8 a
// health of 0. Only the empty cases are ported; any of these stops.
pub(in crate::tes) fn item_flags(world: &World, entry: &InventoryEntry) -> Result<u32, GameError> {
    if entry
        .lists_00
        .as_ref()
        .and_then(|lists| lists.first())
        .is_some_and(|list| list.get(0x27).is_some() || list.get(0x48).is_some())
    {
        return Err(GameError::unclassified(
            0x00485C00,
            "00485C00: an owned or poisoned item's flags are carried",
        ));
    }
    let mut flags = 0;
    if world
        .get::<TesEnchantableForm>(entry.form_08)
        .is_some_and(|enchantable| {
            !matches!(
                enchantable.enchantment_04,
                FormRef::Resolved(None) | FormRef::Raw(0)
            )
        })
    {
        flags |= 2;
    }
    if world.get::<TesHealthForm>(entry.form_08).is_some() {
        return Err(GameError::unclassified(
            0x00485C00,
            "00485C00: an item's health flag is carried",
        ));
    }
    Ok(flags)
}

// IDA: 0048F6A0: 1 "Icons\icon_small_item_pickup.dds", 2 "...icon_enchanted_item.dds", 4 "...icon_poisoned_weapon.dds",
// 8 "...icon_broken_item.dds", another positive "Icons\icon_image_set_%d.dds"; 0 null.
pub(in crate::tes) fn flag_icon(flags: u32) -> Option<Vec<u8>> {
    let name = match flags {
        0 => return None,
        1 => "icon_small_item_pickup.dds".to_owned(),
        2 => "icon_enchanted_item.dds".to_owned(),
        4 => "icon_poisoned_weapon.dds".to_owned(),
        8 => "icon_broken_item.dds".to_owned(),
        other => format!("icon_image_set_{other}.dds"),
    };
    Some(format!("Icons\\{name}").into_bytes())
}
