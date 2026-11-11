//! An actor's equipment: the best-item searches and ratings, the biped slot tests, auto-equip and the equip of one item.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::actor::{Character, MobileObject, TesObjectRefr, actor_is_dead};
use super::actor_enable::{
    EXTRA_DATA_LIST_VTABLE, MINUS_ONE, TYPE_ARMOR, TYPE_CLOTHING, TYPE_LIGHT, TYPE_WEAPON,
    current_package, find_skin_property, list_owner, low_process_only, unequip_item_unstacked,
};
use super::actor_process::MiddleHighProcess;
use super::equipment_3d::{attach_weapon_model, weapon_bone_nodes};
use super::inventory::{
    ContainerChanges, InventoryEntry, ensure_container_changes, put_container_changes,
    reference_container,
};
use crate::tes::ai_package::package_data;
use crate::tes::form::{biped_covers, biped_mask, clothing_value};
use crate::tes::forms_items::{set_change_flags, weapon_type};
use crate::tes::player::{player_get_actor_value_float, the_player};
// IDA: 0x47 ExtraCannotWear, the locked equip (`base_extra_list::EXTRA_CANNOT_WEAR`).
use crate::tes::base_extra_list::EXTRA_CANNOT_WEAR as EXTRA_LOCKED_EQUIP;
use crate::tes::base_extra_list::{
    EXTRA_CONTAINER_CHANGES, EXTRA_COUNT, EXTRA_LEVELED_ITEM, EXTRA_WORN, ExtraData, ExtraDataList,
    new_extra_entry,
};
use crate::tes::form_components::{
    FormRef, TesBipedModelForm, TesContainer, TesEnchantableForm, covers,
};
use crate::tes::forms_actors::creature_auto_equip;
use crate::tes::forms_init::form_type;
use crate::tes::game_settings::float_at_or_zero;

// IDA: the base container's entries, out of its component: (count, form).
pub(crate) fn base_entries(world: &World, reference: Entity) -> Vec<(i32, Entity)> {
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

// IDA: 005E43B0 Actor_FindTorchInventoryEntry, 0048B660 ExtraContainerChangesData_SelectTorch.
// Completion: no changes block or no LIGH candidate in either inventory list.
// TODO: [ ] Select a carried light by rating and return its worn stack.
pub(super) fn find_torch_inventory_entry(
    world: &World,
    actor: Entity,
) -> Result<Option<Entity>, GameError> {
    let Some(extra) = world
        .get::<TesObjectRefr>(actor)
        .and_then(|r| r.extra_data_44.get(EXTRA_CONTAINER_CHANGES))
    else {
        return Ok(None);
    };
    let ExtraData::ContainerChanges(changes) = &extra.data else {
        return Ok(None);
    };
    if base_entries(world, actor)
        .into_iter()
        .any(|(_, f)| form_type(world, f) == TYPE_LIGHT)
        || changes
            .entries_00
            .iter()
            .any(|e| form_type(world, e.form_08) == TYPE_LIGHT)
    {
        return Err(GameError::unclassified(
            0x0048B660,
            "select a carried torch by rating",
        ));
    }
    Ok(None)
}

// IDA: 00486030 ExtraContainerChanges_UnequipWornStacks(keep_locked): in each entry every stack worn (0041DF10(0): 0x1B or
// 0x1C) that isn't equip-locked (0041DF40, when `keep_locked`) loses 0x1B (0041F6A0(0)) and its count extra (004238A0(0)); a
// stack left with no extras leaves the list and is destroyed. An entry left with a list of no stacks and a count of 0 leaves
// the changes (004845A0, the list freed). Completion: 100%.
pub(crate) fn unequip_worn_stacks(changes: &mut ContainerChanges, keep_locked: bool) {
    for entry in &mut changes.entries_00 {
        if let Some(lists) = entry.lists_00.as_mut() {
            lists.retain_mut(|list| {
                if !crate::tes::base_extra_list::worn(list, false)
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

// IDA: the health 0048C870 rates a stack by: its 0x2B extra's float (0041E810), else 0046CA20 TESHealthForm vt+0x10 (+0x04), loaded with `fild` and 2^32 added to a negative (an unsigned), stored a float.
pub(crate) fn stack_health(world: &World, form: Entity, list: Option<&ExtraDataList>) -> f32 {
    if let Some(entry) = list.and_then(|list| list.get(crate::tes::base_extra_list::EXTRA_HEALTH))
        && let ExtraData::Float(bits) = entry.data
    {
        return f32::from_bits(bits);
    }
    let health = world
        .get::<crate::tes::form_components::TesHealthForm>(form)
        .map_or(0, |health| health.health_04);
    f64::from(health) as f32
}

// IDA: 00547140(weapon, enchantment + 0x18, health, skill, luck, strength, fatigue, damage): 0 for a weapon with no health
// (0046CA20); the damage (+0x88 vt+0x10) unless `damage` isn't -1.0 (`ftol`); 00547070(.., 1.0, 0); 00B36D00 times it, plus an
// enchantment's effects (a count above 0) through its vt[0](0) times 00B36D20, a stop here; each step stored a float.
#[expect(clippy::too_many_arguments, reason = "00547140's eight arguments")]
pub(super) fn weapon_rating(
    world: &World,
    weapon: Entity,
    enchantment: Option<Entity>,
    health: f32,
    skill: i32,
    luck: i32,
    strength: i32,
    fatigue: f32,
    damage: f32,
) -> Result<f32, GameError> {
    if world
        .get::<crate::tes::form_components::TesHealthForm>(weapon)
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
        return Err(GameError::unclassified(
            0x005471E6,
            "005471E6: an enchanted weapon's rating (its effect cost)",
        ));
    }
    let scaled = (f64::from(float_at_or_zero(world, 0x00B3_6D00)) * f64::from(base)) as f32;
    Ok(scaled + 0.0)
}

// IDA: +0x88 TESAttackDamageForm vt+0x10, the word at +0x04.
pub(super) fn attack_damage(world: &World, weapon: Entity) -> u16 {
    world
        .get::<crate::tes::form_components::TesAttackDamageForm>(weapon)
        .map_or(0, |form| form.damage_04)
}

// IDA: TESEnchantableForm +0x04, the enchantment.
pub(crate) fn enchantment_of(world: &World, form: Entity) -> Option<Entity> {
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
) -> Result<Option<Entity>, GameError> {
    if check_worn && equipped_in_slot(world, actor, 10)?.is_some() {
        return Err(GameError::unclassified(
            0x0048B9F8,
            "0048B9F8: a worn quiver stack's 0x47 test",
        ));
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

// IDA: 00546C60 Combat_ScaleWeaponRating(rating, unused, 0): fAIMeleeWeaponMult (00B36D00) times the rating, a float
// product of floats.
pub(super) fn combat_scale_weapon_rating(world: &World, rating: f32) -> f32 {
    float_at_or_zero(world, 0x00B3_6D00) * rating
}

/// IDA: 0048D110 ContainerChanges_FindBestClothingForSlot(changes, base, slot, 1), 0048C870's shape over TESObjectCLOT with the
/// value (00470520, 005479B0) as the rating and no health test: a worn stack in the slot is a stop (its 0x47 test); the base
/// container's clothing, ownership counted as 0048C870's, per stack owned by anyone (0048D110 doesn't test the owner of a
/// base entry's stack) or as a whole; then the changes entries owned by nobody or the base, with a count, not listed by a
/// flagged base container, covering the slot, and with no leveled stack or a count of 0 or more, per stack or as a whole.
/// The first value strictly above the best (from -FLT_MAX) wins. Completion: 100% but the worn stack.
/// # Errors
/// As above, or an ownership extra not resolved.
pub(crate) fn best_clothing(
    world: &World,
    changes: &ContainerChanges,
    base: Entity,
    actor: Entity,
    slot: u32,
) -> Result<Option<Entity>, GameError> {
    if equipped_in_slot(world, actor, slot)?.is_some() {
        return Err(GameError::unclassified(
            0x0048D147,
            "0048D147: a worn stack's 0x47 test (0041DF40)",
        ));
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

// IDA: 004D8880 TESObjectREFR_IsFormWorn(actor, form): with a base container, whether the changes' entry for the form holds a worn stack (00485FA0
// with 0: its equip-locked arm doesn't run).
pub(crate) fn form_worn(world: &mut World, actor: Entity, form: Entity) -> Result<bool, GameError> {
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
                .any(|list| crate::tes::base_extra_list::worn(list, false))
        });
    put_container_changes(world, actor, changes);
    Ok(found)
}

// IDA: 005F3422's table before 00489C30, for an actor with a process: an ARMO covering slot 13 unequips the process's shield
// (vt+0xF8(0)), a WEAP its weapon (vt+0xFC(0)), both stops here, and a WEAP with a worn light (vt+0xF0(1)) and a two-hand
// test reaches 004DCAB0, a stop; any other ARMO, and a CLOT, takes nothing. A LIGH or an AMMO is a stop.
fn process_equip_checks(
    world: &World,
    process: Entity,
    kind: u8,
    shield: bool,
) -> Result<(), GameError> {
    let Some(middle) = world.get::<MiddleHighProcess>(process) else {
        return match kind {
            TYPE_CLOTHING | TYPE_ARMOR => Ok(()),
            _ => Err(GameError::unclassified(
                0x005F3422,
                "005F3422: a Low process's weapon or light arm",
            )),
        };
    };
    match kind {
        TYPE_ARMOR if shield && middle.shield_f0.is_some() => Err(GameError::unclassified(
            0x005F34A5,
            "005F34A5: unequipping the process's shield",
        )),
        TYPE_WEAPON if middle.weapon_e4.is_some() => Err(GameError::unclassified(
            0x005F3560,
            "005F3560: unequipping the process's weapon",
        )),
        TYPE_WEAPON if middle.light_e8.is_some_and(|light| light.worn) => {
            Err(GameError::unclassified(
                0x005F35B7,
                "005F35B7: a weapon over a worn light (004DCAB0)",
            ))
        }
        TYPE_LIGHT | 0x22 => Err(GameError::unclassified(
            0x005F3422,
            "005F3422: a light's or an ammo's arm",
        )),
        _ => Ok(()),
    }
}

// IDA: 005F3140 Actor vt+0x108(form, 1, null, 0): vt+0x168 (Character 004349A0, +0x104; the player's 006600D0(0), the same +0x104) and 004691B0's biped. A ring is a stop; with a +0x104, each skin slot holding a TESBipedModelForm over a slot the item covers takes 005F2E70 (`unequip_item_unstacked`). With a process, the 005F3422 table's ARMO, LIGH, WEAP and AMMO arms are stops; CLOT takes the default. Then 00489C30, whose process arm for CLOT (0048A0BC) is a stop for slots 6, 7 and 8 and otherwise vt+0x31C(1), nothing on a LowProcess, then 005EA1A0(+0x3C), which returns on no 3D. Then the visuals of an ARMO with slot 13, a LIGH or a WEAP (005E7230), a TESEnchantableForm's enchantment (0041A610), and the 0x51 extra are stops. Completion: 100% of the clothing arm, with no process or a LowProcess.
pub(crate) fn equip_item(world: &mut World, actor: Entity, item: Entity) -> Result<(), GameError> {
    // 005F3169: the player's skin info is 006600D0 PlayerCharacter_GetSkinInfo(0), +0x104, the field vt+0x168 reads on a
    // Character; its later arms (005F3535's shield unequip, 005F37DB's telekinesis after an enchantment) follow stops below.
    let skin = world
        .get::<Character>(actor)
        .and_then(|character| character.skin_info_104);
    if let Some(mask) = biped_mask(world, item) {
        if covers(mask, 7) || covers(mask, 6) {
            return Err(GameError::unclassified(
                0x005F320F,
                "005F320F: a ring's hand",
            ));
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
                    if equipped_in_slot(world, actor, slot)?.is_some() {
                        return Err(GameError::unclassified(
                            0x005F3381,
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
            .get::<crate::tes::reference_3d::Reference3D>(actor)
            .map(|loaded| loaded.0);
        if let Some(node) = node {
            find_skin_property(world, process, node)?;
        }
    }
    if kind == TYPE_LIGHT {
        return Err(GameError::unclassified(
            0x005F379C,
            "005F379C: a light's visuals (005E7230)",
        ));
    }
    if kind == TYPE_WEAPON || shield {
        crate::tes::player::refresh_equipped_visuals_for(world, actor, item)?;
    }
    if world
        .get::<TesEnchantableForm>(item)
        .is_some_and(|form| matches!(form.enchantment_04, FormRef::Resolved(Some(_))))
    {
        return Err(GameError::unclassified(
            0x005F37CC,
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
        return Err(GameError::unclassified(
            0x005F3804,
            "005F3804: the 0x51 extra's refraction",
        ));
    }
    Ok(())
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
) -> Result<Option<f32>, GameError> {
    let condition = stack_health(world, armor, list);
    // 0048CAC5 `test ah, 41h`: at or below 0.0, or a NaN, skips the stack.
    if test_health && condition.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return Ok(None);
    }
    let rating = world
        .get::<crate::tes::forms_items::TesArmor>(armor)
        .map_or(0, |armor| armor.armor_rating_e4);
    let base_rating = ((f64::from(rating) / 100.0) as f32) as i32 as u16;
    let luck = crate::tes::actor_values::base_actor_value(world, base, 7)? as f32;
    let heavy = world
        .get::<TesBipedModelForm>(armor)
        .is_some_and(|biped| biped.flags_06 & 0x80 != 0);
    let skill =
        crate::tes::actor_values::base_actor_value(world, base, if heavy { 18 } else { 27 })?
            as f32;
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
) -> Result<Option<(Entity, Option<usize>)>, GameError> {
    if check_worn && equipped_in_slot(world, actor, slot)?.is_some() {
        return Err(GameError::unclassified(
            0x0048C8AA,
            "0048C8AA: a worn stack's 0x47 test (0041DF40)",
        ));
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
) -> Result<Option<(Entity, Option<usize>)>, GameError> {
    Ok(best_weapon_rated(world, changes, base, actor, check_worn)?.0)
}

/// IDA: N/A 0048BDA0's selection (the form and its stack's index) and its scaled rating.
pub(crate) type RatedWeapon = (Option<(Entity, Option<usize>)>, f32);

/// IDA: 0048BDA0 with its rating out: 0048C843..0048C85A store 00546C60 Combat_ScaleWeaponRating(best, 0, 0), that is
/// fAIMeleeWeaponMult times the best rating, whether or not a weapon was chosen (the best starts at -FLT_MAX).
/// # Errors
/// As `best_weapon`.
pub(crate) fn best_weapon_rated(
    world: &World,
    changes: &ContainerChanges,
    base: Entity,
    actor: Entity,
    check_worn: bool,
) -> Result<RatedWeapon, GameError> {
    if check_worn && equipped_in_slot(world, actor, 9)?.is_some() {
        return Err(GameError::unclassified(
            0x0048BE08,
            "0048BE08: a worn weapon stack's 0x47 test (0041DF40)",
        ));
    }
    let prefers_ranged = || -> Result<bool, GameError> {
        let style = match world
            .get::<crate::tes::forms_actors::TesNpc>(base)
            .map(|npc| npc.combat_style_1e4)
        {
            Some(FormRef::Resolved(Some(style))) => style,
            _ => {
                return Err(GameError::unclassified(
                    0x004A98C0,
                    "004A98C0: the default combat style's flags (00B35788)",
                ));
            }
        };
        Ok(world
            .get::<crate::tes::forms_ai::TesCombatStyle>(style)
            .is_some_and(|style| style.data_18[0x50] & 0x40 != 0))
    };
    let luck = crate::tes::actor_values::base_actor_value(world, base, 7)? as f32;
    let strength = crate::tes::actor_values::base_actor_value(world, base, 0)? as f32;
    let fatigue = 1.0_f32;
    const SKILLS: [u32; 6] = [14, 14, 16, 16, 16, 28];
    let skill_of = |weapon: Entity| -> Result<f32, GameError> {
        let kind = usize::from(weapon_type(world, weapon));
        let code = *SKILLS.get(kind).ok_or(GameError::unclassified(
            0x004BB060,
            "004BB060: a weapon type past 00B086A0",
        ))?;
        Ok(crate::tes::actor_values::base_actor_value(world, base, code)? as f32)
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
            return Err(GameError::unclassified(
                0x0048BF34,
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
            return Err(GameError::unclassified(
                0x0048C146,
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
    Ok((chosen, combat_scale_weapon_rating(world, best)))
}

/// IDA: 004E1580 TESObjectREFR_UpdateBodyModels's creature arm (type 0x24) with a node, before the body models: the two flags
/// are 1 unless the actor's current package (Actor_GetCurrentPackage, +0x1C) has 0x100000 or 0x200000; in a load (+0x18 bit 2)
/// with change flag 0x8000000, 0051D460 instead, a stop; otherwise 0051E240(actor, a, b, 0). Completion: 100% of the auto-equip
/// arm.
/// # Errors
/// The load arm, or a stop in 0051E240.
pub(crate) fn creature_body_equip(world: &mut World, actor: Entity) -> Result<(), GameError> {
    let process = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58;
    let flags = process
        .and_then(|process| current_package(world, process))
        .map_or(0, |package| package_data(world, package).flags_1c);
    if crate::tes::forms_references::save_load_flags(world)? & 2 != 0 {
        return Err(GameError::unclassified(
            0x004E163D,
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

// IDA: 00489C30's process arm (0048A0BC) after the changes' equip, its stack v26 now worn: a CLOT ring or amulet (slots 6, 7,
// 8) is a stop; any other CLOT or ARMO takes vt+0x31C(1) (006295E0, +0x16C). An ARMO covering slot 13: with no worn shield
// (vt+0xF8(1)) a new entry of the form, the count and the stack goes to vt+0x110 (0064AF10, +0xF0); a WEAP: with no worn
// weapon (vt+0xFC(1)) a new entry to vt+0x104 (00658550: +0xE4, +0xF4 for a bow, +0xF5 for a staff; with +0x17C anim data
// 006541A0, a stop; otherwise +0xFC..+0x110 zeroed), and on true the anim data's +0x60 takes the weapon's +0x94 (no anim data
// here). Each then with +0x16C clear (vt+0x320) 004E1B40 / 004E1900, a stop. Last 005EA1A0 on the node: no 3D returns.
pub(super) fn process_equip(
    world: &mut World,
    actor: Entity,
    process: Entity,
    item: Entity,
    kind: u8,
    shield: bool,
) -> Result<(), GameError> {
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
            Some(0) => Err(GameError::unclassified(
                0x0048A0D4,
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
            return Err(GameError::unclassified(
                0x0048A15B,
                "0048A15B: the worn shield entry's restack",
            ));
        }
        middle.shield_f0 = Some(entry);
        if middle.equipment_update_pending_16c == 0 {
            return Err(GameError::unclassified(
                0x0048A1D0,
                "0048A1D0: 004E1B40, the shield's 3D",
            ));
        }
    } else if let Some(weapon_kind) = weapon_kind {
        if middle.weapon_e4.is_some_and(|held| held.worn) {
            return Err(GameError::unclassified(
                0x0048A3A1,
                "0048A3A1: the worn weapon entry's restack",
            ));
        }
        middle.weapon_e4 = Some(entry);
        middle.bow_f4 = u8::from(weapon_kind == 5);
        middle.staff_f5 = u8::from(weapon_kind == 4);
        if let Some(anim) = middle.anim_data_17c {
            // 006585A9: with animation data, 006541A0 on its manager and node, or (no node yet) 0 and the reference's 3D; the
            // player's first-person data (0065D750) is a stop here.
            if player {
                return Err(GameError::unclassified(
                    0x006585E0,
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
                        .get::<crate::tes::reference_3d::Reference3D>(actor)
                        .map(|loaded| loaded.0);
                    weapon_bone_nodes(world, None, node, item_type)?
                }
            };
            world
                .get_mut::<MiddleHighProcess>(process)
                .expect("a process")
                .weapon_nodes_fc = nodes;
            if let Some(weapon) = nodes.weapon_fc {
                crate::gamebryo::scene_node::detach_scabbard(world, weapon);
            }
            // 0048A21D..0048A231: 00658550 answers 1, so TESObjectREFR_GetAnimData's +0xC0 takes the weapon's speed (+0x94).
            let speed = world
                .get::<crate::tes::forms_items::TesWeapon>(item)
                .map_or(0.0, |weapon| {
                    f32::from_le_bytes(weapon.data_90[4..8].try_into().expect("four bytes"))
                });
            if let Some(mut data) = world.get_mut::<super::actor_animation::ActorAnimData>(anim) {
                data.speed_c0 = speed;
            }
            // 0048A237: with +0x16C clear (vt+0x320), 004E1900.
            let pending = world
                .get::<MiddleHighProcess>(process)
                .expect("a process")
                .equipment_update_pending_16c;
            if pending == 0 {
                attach_weapon_model(world, actor, process, item)?;
            }
            return Ok(());
        }
        middle.weapon_nodes_fc = super::actor_process::WeaponNodes::default();
        if player {
            return Err(GameError::unclassified(
                0x006585F0,
                "006585F0: the player's 00B3BA84..00B3BA98",
            ));
        }
        if middle.equipment_update_pending_16c == 0 {
            attach_weapon_model(world, actor, process, item)?;
        }
    } else {
        // 0048A113: vt+0x31C(1), 006295E0.
        middle.equipment_update_pending_16c = 1;
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
) -> Result<(), GameError> {
    use super::inventory_entries::entry_loose_count;
    use crate::tes::base_extra_list::default_for_container;
    use crate::tes::base_extra_list::leveled;
    use crate::tes::base_extra_list::stack_count;
    use crate::tes::base_extra_list::worn;
    if count > 1 {
        return Err(GameError::unclassified(
            0x0048A029,
            "0048A029: a worn stack's count",
        ));
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
        return Err(GameError::unclassified(
            0x00489E57,
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
        return Err(GameError::unclassified(
            0x00489F6A,
            "00489F6A: a locked (0x47) first stack",
        ));
    }
    if !worn(list, false) || !list.present(EXTRA_WORN) {
        list.add(new_extra_entry(EXTRA_WORN, ExtraData::Marker));
    }
    if list.present(crate::tes::base_extra_list::EXTRA_COUNT) {
        list.remove_by_type(crate::tes::base_extra_list::EXTRA_COUNT);
    }
    Ok(())
}

// IDA: 00489C30(changes, form, count, actor, null, 0, 0) for an item with no changes entry, up to its process arm (0048A081), which `equip_item` takes: +0x08 and +0x0C = -1.0, the owner's vt+0x40(0x8000000), and for an ARMO or CLOT covering none of slots 6, 7, 8 and 13 vt+0x40(0x20000000). 00484420(form, 0) is appended (0067B1E0), and a new 00422EE0 list with 0041F200(1, 0)'s ExtraWorn (a count above 1 is 004238A0's) is prepended to its stacks. 0041F370(0) has no 0x47 to remove. Completion: 100% of that arm.
pub(super) fn equip_in_changes(
    world: &mut World,
    changes: &mut ContainerChanges,
    item: Entity,
    count: i32,
) -> Result<(), GameError> {
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
        return Err(GameError::unclassified(
            0x0048A029,
            "0048A029: a worn stack's count",
        ));
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

/// IDA: 0048A540 ExtraContainerChanges_UnequipItem(changes, &ok, form, 1, actor, null, 0, 0): +0x08 and +0x0C = -1.0; no entry
/// answers 1. A stack locked (0041DF40) on a living actor answers 1 at once; the first worn stack (0041DF10(0)) takes the
/// unequip arm, a stop here. With none: the owner's vt+0x40(0x8000000), and for an ARMO or CLOT covering none of slots 6, 7,
/// 8 and 13 vt+0x40(0x20000000). An entry with a list that is empty, whose count is 0 or the base container's, leaves the
/// changes; otherwise a negative count's list is cleared and 005EA1A0 runs. Answers whether that last arm ran.
/// Completion: an entry with no worn stack.
pub(super) fn changes_unequip_item(
    world: &mut World,
    changes: &mut ContainerChanges,
    form: Entity,
    actor: Entity,
) -> Result<bool, GameError> {
    changes.float_08 = MINUS_ONE;
    changes.float_0c = MINUS_ONE;
    let Some(at) = changes
        .entries_00
        .iter()
        .position(|entry| entry.form_08 == form)
    else {
        return Ok(false);
    };
    for list in changes.entries_00[at]
        .lists_00
        .as_deref()
        .unwrap_or_default()
    {
        if list.present(EXTRA_LOCKED_EQUIP) && !actor_is_dead(world, actor, false) {
            return Ok(false);
        }
        if list.is_worn() {
            return Err(GameError::unclassified(
                0x0048A5E0,
                "0048A5E0: the changes' unequip of a worn stack",
            ));
        }
    }
    let owner = changes.owner_04;
    set_change_flags(world, owner, 0x0800_0000)?;
    if let Some(mask) = biped_mask(world, form)
        && ![7, 6, 8, 13].into_iter().any(|slot| covers(mask, slot))
    {
        set_change_flags(world, owner, 0x2000_0000)?;
    }
    let base_count = reference_container(world, owner)
        .and_then(|base| world.get::<TesContainer>(base))
        .and_then(|container| {
            container
                .entries_08
                .iter()
                .find(|entry| entry.item == FormRef::Resolved(Some(form)))
                .map(|entry| entry.count)
        })
        .unwrap_or(0);
    let entry = &mut changes.entries_00[at];
    let keep = match &entry.lists_00 {
        None => true,
        Some(lists) => !lists.is_empty() || (entry.count_04 != 0 && entry.count_04 != base_count),
    };
    if !keep {
        changes.entries_00.remove(at);
        return Ok(false);
    }
    if entry.count_04 < 0
        && let Some(lists) = entry.lists_00.as_mut()
    {
        lists.clear();
    }
    Ok(true)
}

// IDA: 0048A6F8 005EA1A0 Actor_UpdateSkinProperty(actor, actor's node): no node returns.
pub(super) fn update_skin_property_after_unequip(
    world: &mut World,
    actor: Entity,
) -> Result<(), GameError> {
    let Some(node) = world
        .get::<crate::tes::reference_3d::Reference3D>(actor)
        .map(|loaded| loaded.0)
    else {
        return Ok(());
    };
    let process = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
        .ok_or(GameError::stop(
            0x005EA1A0,
            "a skin property update with no process",
        ))?;
    find_skin_property(world, process, node)
}

// IDA: 00547370 CalculateArmorRatingFromFactors(base, skill, luck, condition): `ftol` of the luck and the skill into 00547B90; the
// rating fArmorRatingBase (00B36E98) + adjusted / 100.0 * (fArmorRatingMax (00B36EA0) - base, stored a float) times the base
// rating, stored a float; its floor (00404F70's body inline at 005473D5 and again at 00547424), and 1.0 unless that floor is
// above 1.0 (00547413 `test ah, 5`, `jp`: equal, below and a NaN answer 1.0); times (fArmorRatingConditionMult (00B36EB0) *
// condition + fArmorRatingConditionBase (00B36EA8), stored a float), stored a float. Each x87 result is rounded to a float as
// it is made (the main thread's 24-bit precision), so the steps are `f32` operations in the instructions' order.
pub(super) fn armor_rating(
    world: &World,
    base_rating: u16,
    skill: f32,
    luck: f32,
    condition: f32,
) -> f32 {
    // 00547370..00547384.
    let luck = x87::ftol(luck);
    let adjusted = super::actor_values::luck_adjusted_skill(world, x87::ftol(skill), luck);
    let rating_base = float_at_or_zero(world, 0x00B3_6E98);
    // 005473A7 `fsubp`, stored at 005473AB.
    let range = float_at_or_zero(world, 0x00B3_6EA0) - rating_base;
    // 005473AF..005473C5 `fdiv` 100.0 (dbl_A309F0), `fmul`, `faddp`, `fild`, `fmulp`.
    let rating = (rating_base + adjusted / 100.0 * range) * f32::from(base_rating);
    let floored = x87::floor_by_ftol(rating);
    let rating = if floored > 1.0 { floored } else { 1.0 };
    // 0054746E..0054748A.
    let factor =
        float_at_or_zero(world, 0x00B3_6EB0) * condition + float_at_or_zero(world, 0x00B3_6EA8);
    factor * rating
}

// IDA: 00547F00 Combat_GetFatigueDamageMultiplier(fatigue) (cdecl): fFatigueBase (00B37530) - (1.0 - fatigue) * fFatigueMult
// (00B37538), stored a float. Completion: 100%.
pub(super) fn fatigue_damage_multiplier(world: &World, fatigue: f32) -> f32 {
    float_at_or_zero(world, 0x00B3_7530) - (1.0 - fatigue) * float_at_or_zero(world, 0x00B3_7538)
}

// IDA: 00547070 CalculateWeaponDamage(skill, luck, strength, fatigue, damage, health, multiplier, no_fatigue): the
// luck-adjusted skill (00547B90); `fild` damage * 00B36DD8, stored a float; 00B36DF8 * health + 00B36DF0, stored; adjusted *
// 0.01 (dbl_A3B150, the float 0.01 as a double) * 00B36DE8 + 00B36DE0, stored; 0.01 * min(strength, 100) (`fimul`) * 00B36E08
// + 00B36E00, stored; the fatigue factor (1.0, or 00547F00); health * damage * skill * strength * fatigue stored, then times
// the multiplier, stored. Each x87 result is rounded to a float as it is made (the main thread's 24-bit precision).
#[expect(clippy::too_many_arguments, reason = "00547070's eight arguments")]
pub(super) fn weapon_damage_rating(
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
    let adjusted = super::actor_values::luck_adjusted_skill(world, skill, luck);
    let set = |address| float_at_or_zero(world, address);
    let hundredth = 0.01_f32;
    // 00547086..0054709E, 005470E5: `fild` and `fimul` take the integer whole, so the product is rounded once: exact in an
    // `f64` for an integer below 2^29.
    let damage = (f64::from(damage) * f64::from(set(0x00B3_6DD8))) as f32;
    let health = set(0x00B3_6DF8) * health + set(0x00B3_6DF0);
    let skill = adjusted * hundredth * set(0x00B3_6DE8) + set(0x00B3_6DE0);
    let strength = (f64::from(hundredth) * f64::from(strength.min(100))) as f32 * set(0x00B3_6E08)
        + set(0x00B3_6E00);
    let fatigue = if no_fatigue {
        1.0
    } else {
        fatigue_damage_multiplier(world, fatigue)
    };
    let product = health * damage * skill * strength * fatigue;
    product * multiplier
}

/// IDA: 0060E580 Character_GetArmorRating (vt+0x348): the cache +0x108 recomputed when it's below 0.0 (or NaN) from the
/// equipped armor (none here: the cache becomes 0.0; any armor stops); the light-armor master arm (Light Armor's mastery 4,
/// 005F23B0) stops; plus DefendBonus (43, vt+0x288); capped at fMaxArmorRating (00B37D20) when that's above 0.
/// Completion: 100% with no armor.
/// # Errors
/// Armor in the inventory, the master arm, or an actor value that stops.
pub fn character_armor_rating(world: &mut World, player: Entity) -> Result<f32, GameError> {
    let cache = world
        .get::<Character>(player)
        .map_or(0.0, |data| data.armor_rating_cache_108);
    // 0060E580: neither above nor equal to 0.0 (below, or NaN) recomputes.
    if cache < 0.0 || cache.is_nan() {
        let forms = crate::tes::menus::magic_menu::inventory_forms(world, player);
        if forms.into_iter().any(|form| form_type(world, form) == 0x14) {
            return Err(GameError::unclassified(
                0x0060E580,
                "0060E580: equipped armor's rating is carried",
            ));
        }
        if let Some(mut data) = world.get_mut::<Character>(player) {
            data.armor_rating_cache_108 = 0.0;
        }
    }
    let rating = world
        .get::<Character>(player)
        .map_or(0.0, |data| data.armor_rating_cache_108);
    if crate::tes::player::actor_skill_mastery_level(world, player, 27)? == 4 {
        return Err(GameError::unclassified(
            0x0060E580,
            "0060E580: the light-armor master arm is carried",
        ));
    }
    let total = player_get_actor_value_float(world, player, 43)? + rating;
    let cap = float_at_or_zero(world, 0x00B3_7D20);
    Ok(if cap <= 0.0 || total < cap {
        total
    } else {
        cap
    })
}

// IDA: 00486790 as a yes or no.
pub(crate) fn find_equipped_in_slot(
    world: &World,
    actor: Entity,
    slot: u32,
) -> Result<bool, GameError> {
    Ok(equipped_in_slot(world, actor, slot)?.is_some())
}

/// IDA: 00486790 ExtraContainerChangesData_FindEquippedInBipedSlot(slot, 0): per entry, a stack with 0x36 on an entry of negative count runs 004853B0, a stop; then the first stack with 0x1B or 0x1C (0041DF10(0)) whose form fits the slot: 9 a weapon (0x21), 12 an ammo (0x22), 14 a light (0x1A), 10 and 11 nothing, 6 and 7 a ring's biped (slot 6 or 7) worn on the right (6, no 0x1C) or the left (7, 0x1C), the rest the biped's mask. Answers the actor's entry's form and whether that stack holds 0x1C (InventoryEntry_IsWorn(selection, 1)); no changes block answers none. Completion: 100% but 004853B0.
/// # Errors
/// 004853B0's arm.
pub(crate) fn equipped_in_slot(
    world: &World,
    actor: Entity,
    slot: u32,
) -> Result<Option<(Entity, bool)>, GameError> {
    let Some(crate::tes::base_extra_list::ExtraData::ContainerChanges(changes)) = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .extra_data_44
        .get(crate::tes::base_extra_list::EXTRA_CONTAINER_CHANGES)
        .map(|entry| &entry.data)
    else {
        return Ok(None);
    };
    Ok(
        equipped_stack_in_slot(world, changes, slot)?.map(|(at, index)| {
            let entry = &changes.entries_00[at];
            let left = entry.lists_00.as_ref().expect("matched")[index]
                .present(crate::tes::base_extra_list::EXTRA_WORN_LEFT);
            (entry.form_08, left)
        }),
    )
}

/// IDA: 00486790's walk on a changes block, answering the selection's place: the entry's and the matched stack's indexes
/// (the selection copies the form, that stack and its 0041E860 count). Completion: 100% but 004853B0.
/// # Errors
/// 004853B0's arm.
pub(crate) fn equipped_stack_in_slot(
    world: &World,
    changes: &crate::tes::inventory::ContainerChanges,
    slot: u32,
) -> Result<Option<(usize, usize)>, GameError> {
    use crate::tes::base_extra_list::{EXTRA_LEVELED_ITEM, EXTRA_WORN_LEFT};
    for (at, entry) in changes.entries_00.iter().enumerate() {
        let Some(lists) = &entry.lists_00 else {
            continue;
        };
        if entry.count_04 < 0
            && lists
                .iter()
                .any(|list| list.get(EXTRA_LEVELED_ITEM).is_some())
        {
            return Err(GameError::unclassified(
                0x004867F0,
                "004867F0: 004853B0 for a leveled stack",
            ));
        }
        for (index, list) in lists.iter().enumerate() {
            if !list.is_worn() {
                continue;
            }
            let left = list.present(EXTRA_WORN_LEFT);
            let form_type = crate::tes::forms_init::form_type(world, entry.form_08);
            let fits = match slot {
                6 | 7 => {
                    (crate::tes::form::biped_covers(world, entry.form_08, 7)
                        || crate::tes::form::biped_covers(world, entry.form_08, 6))
                        && (slot == 7) == left
                }
                9 => form_type == 0x21,
                10 | 11 => false,
                12 => form_type == 0x22,
                14 => form_type == 0x1A,
                _ => crate::tes::form::biped_covers(world, entry.form_08, slot),
            };
            if fits {
                return Ok(Some((at, index)));
            }
        }
    }
    Ok(None)
}

// IDA: 0060E260 Character_InvalidateArmorRatingCache (vt+0x2C0): +0x108 = -1.0.
pub(crate) fn invalidate_armor_rating(world: &mut World, actor: Entity) {
    if let Some(mut character) = world.get_mut::<crate::tes::actor::Character>(actor) {
        character.armor_rating_cache_108 = -1.0;
    }
}
