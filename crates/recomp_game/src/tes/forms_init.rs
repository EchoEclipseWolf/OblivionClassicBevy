//! Phase 12d: vt+0x6C InitItem of the form classes, with the component resolvers each reaches: 0046F020 scripts, 0046E6B0 reactions, 0046FC10 spell lists, 0046A880 enchantments, 00469E90 containers, 0046E220 produce, 0046E4F0 races, 004676B0 actor base data, 00569540 AI packages, 0046D070 leveled lists and 0056A480 conditions. The world classes are in `forms_init_world`, the passes that call all of these in `post_load`.
//!
//! A slot holds a raw form ID until its InitItem, as `FormRef::Raw`, then `FormRef::Resolved`, by __RTDynamicCast through `form_hierarchy`. Every unresolved-reference warning keeps its text; none fires over the shipped data (P12-RUN-005), so they stand on the exe alone.
//!
//! The x87 precision of NPC auto-calculation is the main thread's 24 bits (P04-RUN-002): plain `f32` in the exe's grouping, and a `fmul qword` is the exact product rounded once, as `(f64::from(x) * C) as f32`. Evidence: objdump of each address named here.

use bevy_ecs::prelude::*;
use recomp_core::crt;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::actor_process::actor_values_set_modifier;
use super::condition_tables::{CONDITION_FUNCTIONS, CONDITION_PARAMETER_KINDS};
use super::data_handler::{DataHandler, DataHandlerLists, FormsById, TesSkill};
use super::data_load::{DataLoadGlobals, DataLoadServices};
use super::form::FormIdentity;
use super::form_arms::{editor_id_of, form_class};
use super::form_components::{
    FormRef, TesActorBaseData, TesAiForm, TesAttributes, TesBipedModelForm, TesContainer,
    TesEnchantableForm, TesHealthForm, TesLeveledList, TesObjectLinks, TesProduceForm, TesRaceForm,
    TesReactionForm, TesScriptableForm, TesSpellList,
};
use super::form_hierarchy::{RttiClass, form_type_casts_to};
use super::forms_actors::{ActorBaseValues, CreatureSoundsSlot, TesCreature, TesNpc, TesRace};
use super::forms_ai::{TesCondition, TesConditionItem, TesIdleForm, TesPackage, describe};
use super::forms_dialogue::{TesQuest, TesTopic, TesTopicInfo};
use super::forms_environment::{TesLoadScreen, TesObjectAnio, TesWaterForm, WarningCounter};
use super::forms_items::{
    TesActivator, TesBook, TesContainerObject, TesDoor, TesFurniture, TesLevCreature, TesLight,
    TesWeapon, set_change_flags,
};
use super::forms_magic::{EffectItemList, EffectSetting, MagicFormData};
use super::forms_misc::{TesClass, TesLandTexture, set_initialized};
use super::forms_region::TesRegion;
use super::forms_script::{
    Script, ScriptEventList, ScriptEventVariable, last_mod_reference, latin1, map_form_id_in,
    resolve_script_with,
};
use super::forms_world::TesWorldSpace;
use super::game_settings::GameSettingCollection;
use super::player::PlayerGlobals;
use super::tes_file::{BsStringT, TesFileId};

// IDA: the form flags these read.
const FLAG_INITIALIZED: u32 = 8;
const FLAG_TEMPORARY: u32 = 0x4000;
const FLAG_NO_HEALTH: u32 = 0x8_0000;
/// IDA: the "VAMP" effect code 005239C0 compares.
const VAMP: u32 = 0x504D_4156;

// ---- what every resolver shares ----

/// IDA: __RTDynamicCast(form, 0, TESForm, class, 0) on a form that exists. Completion: 100%.
#[must_use]
pub fn casts_to(world: &World, form: Entity, class: RttiClass) -> bool {
    form_type_casts_to(form_type(world, form), class)
}

/// IDA: __RTDynamicCast with a null form going through: 0 stays 0. Completion: 100%.
#[must_use]
pub fn cast(world: &World, form: Option<Entity>, class: RttiClass) -> Option<Entity> {
    form.filter(|&form| casts_to(world, form, class))
}

/// IDA: 0046B250 LookupFormByID. Completion: 100%.
#[must_use]
pub fn lookup(world: &World, id: u32) -> Option<Entity> {
    world.resource::<FormsById>().lookup(id)
}

/// IDA: N/A the type byte, +0x04.
#[must_use]
pub fn form_type(world: &World, form: Entity) -> u8 {
    world
        .get::<FormIdentity>(form)
        .expect("a form")
        .form_type_04
}

/// IDA: N/A the form ID, +0x0C.
#[must_use]
pub fn form_id(world: &World, form: Entity) -> u32 {
    world.get::<FormIdentity>(form).expect("a form").form_id_0c
}

/// IDA: the InitItem gate, +0x08 bit 3: `mov eax,[esi+8]; shr eax,3; test al,1`. Completion: 100%.
#[must_use]
pub fn initialized(world: &World, form: Entity) -> bool {
    world.get::<FormIdentity>(form).expect("a form").flags_08 & FLAG_INITIALIZED != 0
}

/// IDA: 0046BB20 over the world's files. Completion: 100% of `map_form_id_in`'s arms.
pub fn map_id(world: &World, id: u32, file: Option<TesFileId>) -> Result<u32, StreamError> {
    let files = &world.resource::<DataHandler>().file_store;
    map_form_id_in(world, files, id, file)
}

/// IDA: 0046B680(-1) then 0046BB20, the shared head of a resolver. Completion: 100%.
pub fn map_through_owner(world: &World, id: u32, owner: Entity) -> Result<u32, StreamError> {
    map_id(world, id, last_mod_reference(world, owner))
}

/// IDA: 004A7A60 DebugMessage_Warning, with g_disableWarningMessages at the call (D4: the host log). Completion: 100%.
pub fn warn(world: &World, services: &mut DataLoadServices<'_>, text: String) {
    let flag = world
        .resource::<DataLoadGlobals>()
        .disable_warning_messages_b06b18;
    (services.warning)(text, flag != 0);
}

/// IDA: vt+0xD4 as a warning's `%s`. Completion: 100% of the ported classes.
#[must_use]
pub fn name_of(world: &World, form: Entity) -> String {
    latin1(&editor_id_of(world, form))
}

// IDA: N/A the raw word of a slot a resolver is about to map. A slot holding a pointer can't reach here: the InitItem gate is the 0x8 flag, and only an InitItem stores a pointer.
fn raw_of(slot: FormRef) -> Result<u32, StreamError> {
    match slot {
        FormRef::Raw(id) => Ok(id),
        FormRef::Resolved(_) => Err(StreamError(
            "InitItem: a slot resolved twice maps a pointer",
        )),
    }
}

/// IDA: 0065C620 PointerList_RemoveFirstMatchingItem(prev node, mapped id), the removal every resolver makes when a previous node exists, then `node = prev->next`. The search starts at the previous node, and an ID that maps to something else matches nothing, so the walk meets the same node again and warns forever in the original. Host stop. A zero argument returns at once. Completion: 100%.
fn remove_after_previous(
    list: &mut Vec<FormRef>,
    previous: usize,
    mapped: u32,
) -> Result<(), StreamError> {
    let at = (mapped != 0)
        .then(|| {
            list.iter()
                .skip(previous)
                .position(|slot| *slot == FormRef::Raw(mapped))
        })
        .flatten();
    let Some(at) = at else {
        return Err(StreamError(
            "0065C620: a missing form whose ID maps to another: the original's walk does not end",
        ));
    };
    list.remove(previous + at);
    Ok(())
}

// ---- the component resolvers ----

/// IDA: 0046F020 as an InitItem reaches it. Completion: 100%.
pub fn init_script(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let mut sink = Vec::new();
    resolve_script_with(world, form, &map_id, &mut |text| sink.push(text))?;
    for text in sink {
        warn(world, services, text);
    }
    Ok(())
}

/// IDA: 0046E6B0 TESReactionForm_InitItem(owner): each block's +0x00 through the owner's last file and 0046B250, no cast, stored. A miss warns a3c42c and frees the node and the block: the next node is pulled in, or the head item cleared. The owner is never null at a reached call. Completion: 100%.
pub fn init_reactions(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let file = last_mod_reference(world, form);
    let mut reactions = std::mem::take(
        &mut world
            .get_mut::<TesReactionForm>(form)
            .expect("the class has reactions")
            .reactions_04,
    );
    let mut index = 0;
    while index < reactions.len() {
        let mapped = map_id(world, raw_of(reactions[index].faction)?, file)?;
        match lookup(world, mapped) {
            Some(found) => {
                reactions[index].faction = FormRef::Resolved(Some(found));
                index += 1;
            }
            None => {
                let text = format!(
                    "Form ({mapped:08X}) not found for reaction of pForm ({:08X}) '{}'.\r\n",
                    form_id(world, form),
                    name_of(world, form)
                );
                warn(world, services, text);
                reactions.remove(index);
            }
        }
    }
    world
        .get_mut::<TesReactionForm>(form)
        .expect("the class has reactions")
        .reactions_04 = reactions;
    Ok(())
}

/// IDA: 00415130 EffectItemList_HasFlag(flag): over a nonempty list, an item whose setting (+0x1C) has the flag and not bit 22 ends the walk with 1; a null setting faults in the original (host stop). The walk starts with al = 0 and stops once al is set. Completion: 100%.
pub fn effects_have_flag(
    world: &World,
    effects: &EffectItemList,
    flag: u32,
) -> Result<bool, StreamError> {
    for item in &effects.items_04 {
        let setting = item.setting_1c.ok_or(StreamError(
            "0041515B: an effect item with no setting faults",
        ))?;
        let flags = world
            .get::<EffectSetting>(setting)
            .expect("an effect setting")
            .data_58
            .flags_58;
        if flags & flag != 0 && flags & 0x40_0000 == 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// IDA: 004190C0 EnchantmentItem_IsValidFor(owner), with the jump table 004191D4 over the bytes 004191EC for types 0x14..=0x22: ARMO: not 00415130(0x20000), cast type 3. BOOK: +0x88 bit 0, cast type 0. CLOT: TESBipedModelForm's 00469050, or not 00415130(0x20000), cast type 3. WEAP: a type byte +0x90 of 4 wants cast type 1, otherwise 2. AMMO: cast type 2. Anything else, or a form that isn't a TESObject, 0. Completion: 100%.
pub fn enchantment_valid_for(
    world: &World,
    enchantment: Entity,
    owner: Entity,
) -> Result<bool, StreamError> {
    if !casts_to(world, owner, RttiClass::TESObject) {
        return Ok(false);
    }
    let cast_type = world
        .get::<MagicFormData>(enchantment)
        .expect("an enchantment")
        .type_34;
    let effects = world
        .get::<EffectItemList>(enchantment)
        .expect("an enchantment");
    Ok(match form_type(world, owner) {
        0x14 => !effects_have_flag(world, effects, 0x2_0000)? && cast_type == 3,
        0x15 => {
            let book = world.get::<TesBook>(owner).expect("a book");
            book.flags_88 & 1 != 0 && cast_type == 0
        }
        0x16 => {
            let Some(biped) = world.get::<TesBipedModelForm>(owner) else {
                return Ok(false);
            };
            // 00469050: bit 6, 7 or 8 of the slot mask.
            let jewelry = biped.slot_mask_04 & 0x1C0 != 0;
            if !jewelry && effects_have_flag(world, effects, 0x2_0000)? {
                return Ok(false);
            }
            cast_type == 3
        }
        0x21 => {
            let weapon = world.get::<TesWeapon>(owner).expect("a weapon");
            if weapon.data_90[0] == 4 {
                cast_type == 1
            } else {
                cast_type == 2
            }
        }
        0x22 => cast_type == 2,
        _ => false,
    })
}

/// IDA: 0046A880 TESEnchantableForm_InitItem(owner): a zero slot returns. The ID maps, 0046B250 and the EnchantmentItem cast go in the slot. A null warns a3ba30 or a3b9f4 by 00404E50. A form that 004190C0 refuses warns a3b968 and the slot clears. Completion: 100%.
pub fn init_enchantment(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let slot = world
        .get::<TesEnchantableForm>(form)
        .expect("the class is enchantable")
        .enchantment_04;
    if slot.is_empty() {
        return Ok(());
    }
    let mapped = map_through_owner(world, raw_of(slot)?, form)?;
    let found = cast(world, lookup(world, mapped), RttiClass::EnchantmentItem);
    world
        .get_mut::<TesEnchantableForm>(form)
        .expect("the class is enchantable")
        .enchantment_04 = FormRef::Resolved(found);
    let Some(found) = found else {
        let name = name_of(world, form);
        let text = if name.is_empty() {
            format!(
                "Unable to find enchanting ({mapped:08X}) on owner object ({:08X}).",
                form_id(world, form)
            )
        } else {
            format!("Unable to find enchanting ({mapped:08X}) on owner object \"{name}\".")
        };
        warn(world, services, text);
        return Ok(());
    };
    if !enchantment_valid_for(world, found, form)? {
        let text = format!(
            "Enchanting ({:08X}) '{}' has invalid cast type or magic effect for object ({:08X}) '{}'.",
            form_id(world, found),
            name_of(world, found),
            form_id(world, form),
            name_of(world, form)
        );
        warn(world, services, text);
        world
            .get_mut::<TesEnchantableForm>(form)
            .expect("the class is enchantable")
            .enchantment_04 = FormRef::Resolved(None);
    }
    Ok(())
}

/// IDA: 005E02E0(player, 0): vt+0x170 the base form, when vt+0x190 answers true, which the player's does. A null g_thePlayer faults in the original (host stop). Completion: 100% of the arm a load takes.
pub fn player_base(world: &World) -> Result<Option<Entity>, StreamError> {
    let player = world
        .get_resource::<PlayerGlobals>()
        .and_then(|globals| globals.the_player_b333c4)
        .ok_or(StreamError("005E02EE: a null g_thePlayer faults"))?;
    Ok(world
        .get::<super::actor::TesObjectRefr>(player)
        .expect("the player is a reference")
        .base_form_1c)
}

// IDA: 0065F6A0 the player's spell: for a type 0, 4, 2 or 3 (vt+0x18 of +0x18 reads +0x34), a nonempty list's every setting takes 0x200000; a null item is skipped.
fn mark_player_spell(world: &mut World, spell: Entity) {
    let cast_type = world.get::<MagicFormData>(spell).expect("a spell").type_34;
    if !matches!(cast_type, 0 | 2 | 3 | 4) {
        return;
    }
    let settings: Vec<Entity> = world
        .get::<EffectItemList>(spell)
        .expect("a spell")
        .items_04
        .iter()
        .filter_map(|item| item.setting_1c)
        .collect();
    for setting in settings {
        world
            .get_mut::<EffectSetting>(setting)
            .expect("an effect setting")
            .data_58
            .flags_58 |= 0x20_0000;
    }
}

// IDA: 0046F3D0 TESSpellList_AddLeveledSpell(form): a null returns. When this list is the player's base's +0x54 it returns; the +0x0C list's item test; a prepend (00446CB0).
fn add_leveled_spell(world: &mut World, form: Entity, leveled: Entity) -> Result<(), StreamError> {
    if player_base(world)? == Some(form) {
        return Ok(());
    }
    let mut list = world.get_mut::<TesSpellList>(form).expect("a spell list");
    let slot = FormRef::Resolved(Some(leveled));
    if !list.leveled_0c.contains(&slot) {
        list.leveled_0c.insert(0, slot);
    }
    Ok(())
}

/// IDA: 0046FC10 TESSpellList_InitItem(owner). Each slot maps and looks up. A miss unlinks the node and warns a3c600 or a3c5c8 (by the same test as 00404E50). A hit that casts to SpellItem stays, and when the list's form casts to TESActorBase and is the player's base, 0065F6A0 runs. A hit that isn't unlinks the node, and a TESLevSpell goes to 0046F3D0. Completion: 100%, with the list's own form as the cast from TESSpellList.
pub fn init_spells(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let file = last_mod_reference(world, form);
    let mut spells = std::mem::take(
        &mut world
            .get_mut::<TesSpellList>(form)
            .expect("the class has spells")
            .spells_04,
    );
    let mut index = 0;
    let mut previous: Option<usize> = None;
    let mut result = Ok(());
    while index < spells.len() {
        let step = (|| -> Result<(), StreamError> {
            let mapped = map_id(world, raw_of(spells[index])?, file)?;
            let Some(found) = lookup(world, mapped) else {
                unlink_spell(&mut spells, &mut index, previous, mapped)?;
                let name = name_of(world, form);
                let text = if name.is_empty() {
                    format!(
                        "Unable to find spell ({mapped:08X}) for owner object ({:08X}).",
                        form_id(world, form)
                    )
                } else {
                    format!("Unable to find spell ({mapped:08X}) for owner object \"{name}\".")
                };
                warn(world, services, text);
                return Ok(());
            };
            if casts_to(world, found, RttiClass::SpellItem) {
                spells[index] = FormRef::Resolved(Some(found));
                if casts_to(world, form, RttiClass::TESActorBase)
                    && player_base(world)? == Some(form)
                {
                    mark_player_spell(world, found);
                }
                previous = Some(index);
                index += 1;
                return Ok(());
            }
            let leveled = casts_to(world, found, RttiClass::TESLevSpell);
            unlink_spell(&mut spells, &mut index, previous, mapped)?;
            if leveled {
                add_leveled_spell(world, form, found)?;
            }
            Ok(())
        })();
        if step.is_err() {
            result = step;
            break;
        }
    }
    world
        .get_mut::<TesSpellList>(form)
        .expect("the class has spells")
        .spells_04 = spells;
    result
}

// IDA: the unlink of 0046FC85 and 0046FDCB: a last node with a previous one goes through 0065C620 with the mapped ID, which leaves the walk at previous's next; another pulls the next node in; a head with no next clears its item.
fn unlink_spell(
    spells: &mut Vec<FormRef>,
    index: &mut usize,
    previous: Option<usize>,
    mapped: u32,
) -> Result<(), StreamError> {
    if *index + 1 == spells.len()
        && let Some(previous) = previous
    {
        remove_after_previous(spells, previous, mapped)?;
        *index = previous + 1;
        return Ok(());
    }
    spells.remove(*index);
    Ok(())
}

/// IDA: 00469E90 TESContainer_InitItem(owner): with +0x04 bit 0 clear, each entry through 004695D0 (the owner's last file, 0046B250, the TESBoundObject cast, stored; a miss warns a3b834 or a3b7f4). An entry left with a null item leaves the list and is freed: 0065C620 with the entry, in the original's list of entries, finds it. +0x04 |= 1. Completion: 100%.
pub fn init_container(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if world
        .get::<TesContainer>(form)
        .expect("the class has a container")
        .flags_04
        & 1
        != 0
    {
        return Ok(());
    }
    let file = last_mod_reference(world, form);
    let mut entries = std::mem::take(
        &mut world
            .get_mut::<TesContainer>(form)
            .expect("the class has a container")
            .entries_08,
    );
    let mut index = 0;
    let mut result = Ok(());
    while index < entries.len() {
        let mapped = match raw_of(entries[index].item).and_then(|raw| map_id(world, raw, file)) {
            Ok(mapped) => mapped,
            Err(error) => {
                result = Err(error);
                break;
            }
        };
        let found = cast(world, lookup(world, mapped), RttiClass::TESBoundObject);
        entries[index].item = FormRef::Resolved(found);
        if found.is_some() {
            index += 1;
            continue;
        }
        let name = name_of(world, form);
        let text = if name.is_empty() {
            format!(
                "Unable to find container object ({mapped:08X}) on owner object ({:08X}).",
                form_id(world, form)
            )
        } else {
            format!("Unable to find container object ({mapped:08X}) on owner object \"{name}\".")
        };
        warn(world, services, text);
        entries.remove(index);
    }
    let mut container = world
        .get_mut::<TesContainer>(form)
        .expect("the class has a container");
    container.entries_08 = entries;
    container.flags_04 |= 1;
    result
}

/// IDA: 0046D070 TESLeveledList_InitItem(owner): each entry's +0x04 maps and looks up, no cast, stored. A miss pulls the next node in (or clears the head item), frees the entry, and warns a3c118 or a3c0d0. Completion: 100%.
pub fn init_leveled(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let file = last_mod_reference(world, form);
    let mut entries = std::mem::take(
        &mut world
            .get_mut::<TesLeveledList>(form)
            .expect("the class has a leveled list")
            .entries_04,
    );
    let mut index = 0;
    let mut result = Ok(());
    while index < entries.len() {
        let mapped = match raw_of(entries[index].form_04).and_then(|raw| map_id(world, raw, file)) {
            Ok(mapped) => mapped,
            Err(error) => {
                result = Err(error);
                break;
            }
        };
        if let Some(found) = lookup(world, mapped) {
            entries[index].form_04 = FormRef::Resolved(Some(found));
            index += 1;
            continue;
        }
        entries.remove(index);
        let name = name_of(world, form);
        let text = if name.is_empty() {
            format!(
                "Unable to find Leveled Object Form ({mapped:08X}) for owner object ({:08X}).",
                form_id(world, form)
            )
        } else {
            format!(
                "Unable to find Leveled Object Form ({mapped:08X}) for owner object \"{name}\"."
            )
        };
        warn(world, services, text);
    }
    world
        .get_mut::<TesLeveledList>(form)
        .expect("the class has a leveled list")
        .entries_04 = entries;
    result
}

/// IDA: 004676B0 TESActorBaseData_InitItem(owner): each block's +0x00: nonzero maps and casts to TESFaction, stored; a zero or a miss unlinks the node, with no warning and the block leaked. Then a nonzero +0x14 maps and casts to TESLevItem, stored; a miss warns a3b6b4. Completion: 100%.
pub fn init_actor_base_data(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let file = last_mod_reference(world, form);
    let mut data = world
        .get::<TesActorBaseData>(form)
        .expect("an actor base")
        .clone();
    let mut index = 0;
    while index < data.factions_18.len() {
        let raw = raw_of(data.factions_18[index].faction_00)?;
        let found = if raw == 0 {
            None
        } else {
            let mapped = map_id(world, raw, file)?;
            cast(world, lookup(world, mapped), RttiClass::TESFaction)
        };
        if found.is_some() {
            data.factions_18[index].faction_00 = FormRef::Resolved(found);
            index += 1;
        } else {
            data.factions_18.remove(index);
        }
    }
    if !data.death_item_14.is_empty() {
        let mapped = map_id(world, raw_of(data.death_item_14)?, file)?;
        let found = cast(world, lookup(world, mapped), RttiClass::TESLevItem);
        data.death_item_14 = FormRef::Resolved(found);
        if found.is_none() {
            let text = format!(
                "Unable to find death item ({mapped:08X}) on ActorBase ({:08X}) '{}'.",
                form_id(world, form),
                name_of(world, form)
            );
            warn(world, services, text);
        }
    }
    *world
        .get_mut::<TesActorBaseData>(form)
        .expect("an actor base") = data;
    Ok(())
}

/// IDA: 00468590, a gate on 00B05DC0, then 00569540 on +0x10: each slot maps and casts to TESPackage. A hit is stored. A miss warns a67604 with vt+0x74, then unlinks: 0065C620 with the mapped ID when a previous node exists, otherwise the next node is pulled in, or the head item cleared. Completion: 100%.
pub fn init_packages(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if world.resource::<DataLoadGlobals>().skip_ai_packages_b05dc0 != 0 {
        return Ok(());
    }
    let file = last_mod_reference(world, form);
    let mut packages = std::mem::take(
        &mut world
            .get_mut::<TesAiForm>(form)
            .expect("an actor base")
            .packages_10,
    );
    let mut index = 0;
    let mut previous: Option<usize> = None;
    let mut result = Ok(());
    while index < packages.len() {
        let mapped = match raw_of(packages[index]).and_then(|raw| map_id(world, raw, file)) {
            Ok(mapped) => mapped,
            Err(error) => {
                result = Err(error);
                break;
            }
        };
        if let Some(found) = cast(world, lookup(world, mapped), RttiClass::TESPackage) {
            packages[index] = FormRef::Resolved(Some(found));
            previous = Some(index);
            index += 1;
            continue;
        }
        let text = format!(
            "Could not find Package ({mapped:08X}) for {}.",
            describe(world, form)
        );
        warn(world, services, text);
        if let Some(before) = previous {
            if let Err(error) = remove_after_previous(&mut packages, before, mapped) {
                result = Err(error);
                break;
            }
            index = before + 1;
        } else {
            packages.remove(index);
        }
    }
    world
        .get_mut::<TesAiForm>(form)
        .expect("an actor base")
        .packages_10 = packages;
    result
}

/// IDA: 0046E4F0 TESRaceForm_InitItem(owner): a zero slot returns; the ID maps and casts to TESRace, stored; a null warns a3c3e4 or a3c3b0. Completion: 100%.
pub fn init_race_form(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let slot = world.get::<TesRaceForm>(form).expect("an NPC").race_04;
    if slot.is_empty() {
        return Ok(());
    }
    let mapped = map_through_owner(world, raw_of(slot)?, form)?;
    let found = cast(world, lookup(world, mapped), RttiClass::TESRace);
    world.get_mut::<TesRaceForm>(form).expect("an NPC").race_04 = FormRef::Resolved(found);
    if found.is_none() {
        let name = name_of(world, form);
        let text = if name.is_empty() {
            format!(
                "Unable to find race ({mapped:08X}) on owner object ({:08X}).",
                form_id(world, form)
            )
        } else {
            format!("Unable to find race ({mapped:08X}) on owner object \"{name}\".")
        };
        warn(world, services, text);
    }
    Ok(())
}

/// IDA: 0046E220 TESProduceForm_InitItem(owner): a zero slot returns; the ID maps and casts to IngredientItem, stored; a null warns a3c320 or a3c2e4. Completion: 100%.
pub fn init_produce(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let slot = world
        .get::<TesProduceForm>(form)
        .expect("a flora")
        .ingredient_04;
    if slot.is_empty() {
        return Ok(());
    }
    let mapped = map_through_owner(world, raw_of(slot)?, form)?;
    let found = cast(world, lookup(world, mapped), RttiClass::IngredientItem);
    world
        .get_mut::<TesProduceForm>(form)
        .expect("a flora")
        .ingredient_04 = FormRef::Resolved(found);
    if found.is_none() {
        let name = name_of(world, form);
        let text = if name.is_empty() {
            format!(
                "Unable to find ingredient ({mapped:08X}) on owner object ({:08X}).",
                form_id(world, form)
            )
        } else {
            format!("Unable to find ingredient ({mapped:08X}) on owner object \"{name}\".")
        };
        warn(world, services, text);
    }
    Ok(())
}

// ---- conditions ----

// IDA: 0056B220 the script of parameter `index`: its kind must hold a form; the form's TESScriptableForm +0x04, or a reference's vt+0x170's. Null otherwise.
fn condition_parameter_script(
    world: &World,
    item: &TesConditionItem,
    index: usize,
) -> Result<Option<Entity>, StreamError> {
    let function = item.function_08();
    let Some(row) = CONDITION_FUNCTIONS.get(usize::from(function)) else {
        return Ok(None);
    };
    if index >= usize::from(row.parameter_count_12) {
        return Ok(None);
    }
    let kind = row.parameter_kinds_14[index] as usize;
    if CONDITION_PARAMETER_KINDS[kind].is_form_05 == 0 {
        return Ok(None);
    }
    let form = item.parameters_resolved[index].flatten();
    let scriptable = |form: Entity| {
        world
            .get::<TesScriptableForm>(form)
            .map(|component| component.script_04)
    };
    let slot = form.and_then(|form| {
        scriptable(form).or_else(|| {
            casts_to(world, form, RttiClass::TESObjectREFR)
                .then(|| {
                    world
                        .get::<super::actor::TesObjectRefr>(form)
                        .and_then(|reference| reference.base_form_1c)
                })
                .flatten()
                .and_then(scriptable)
        })
    });
    match slot {
        None | Some(FormRef::Raw(0) | FormRef::Resolved(None)) => Ok(None),
        Some(FormRef::Resolved(Some(script))) => Ok(Some(script)),
        Some(FormRef::Raw(_)) => Err(StreamError(
            "0056B283: a script slot still raw, which the original reads as a pointer",
        )),
    }
}

// IDA: 004FA890(index) on a script: the count of +0x48 blocks with +0x10 set; an index at or past it is a variable index, reduced by it, among those with +0x10 clear; otherwise among those set. Null when none.
fn script_variable_by_index(
    script: &Script,
    index: u32,
) -> Option<&super::forms_script::ScriptVariable> {
    let references = script
        .variables_48
        .iter()
        .filter(|variable| variable.data_00[0x10] != 0)
        .count() as u32;
    let reduced = index >= references;
    let wanted = if reduced { index - references } else { index };
    let mut count = 0;
    for variable in &script.variables_48 {
        if (variable.data_00[0x10] != 0) == reduced {
            continue;
        }
        if count == wanted {
            return Some(variable);
        }
        count += 1;
    }
    None
}

// IDA: 0056B2E0 TESConditionItem_ResolveParams(owner, &item+8, item+0x14). For each of the function's parameters: a form kind (00B0A54D): a nonzero word maps, looks up, is stored, and a miss warns a68080 with vt+0x74; kind 0x0B casts to MagicItem and warns a68028. Kind 0x16 with a previous parameter: the previous's script (0056B220) checks the word, by 004FA890 with +0x14 set, warning a67fa0, or by 004FA840 with it clear, warning a67f48 (and a49b10 inside it). A form kind in the third slot walks past the item in the original: a host stop.
fn resolve_condition_parameters(
    world: &mut World,
    form: Entity,
    item: &mut TesConditionItem,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let function = item.function_08();
    let Some(row) = CONDITION_FUNCTIONS.get(usize::from(function)) else {
        return Ok(());
    };
    let file = last_mod_reference(world, form);
    let compiled_flag = item.data_00[0x14];
    for index in 0..usize::from(row.parameter_count_12) {
        let kind = row.parameter_kinds_14[index];
        let info = &CONDITION_PARAMETER_KINDS[kind as usize];
        if info.is_form_05 != 0 {
            if index >= 2 {
                return Err(StreamError(
                    "0056B3B6: a form parameter past the condition item",
                ));
            }
            let raw = item.parameter(index);
            if raw == 0 {
                continue;
            }
            let mapped = map_id(world, raw, file)?;
            let found = lookup(world, mapped);
            item.parameters_resolved[index] = Some(found);
            let Some(found) = found else {
                let text = format!(
                    "Unable to find Function Info TESForm ({mapped:08X}) in TESConditionItem Parameter Init for {}.",
                    describe(world, form)
                );
                warn(world, services, text);
                continue;
            };
            if info.kind_00 == 0x0B {
                let magic = cast(world, Some(found), RttiClass::MagicItem);
                item.parameters_resolved[index] = Some(magic);
                if magic.is_none() {
                    let text = format!(
                        "Unable to find Function Info MagicItem in TESConditionItem Parameter Init for {}.",
                        describe(world, form)
                    );
                    warn(world, services, text);
                }
            }
            continue;
        }
        if kind != 0x16 || index == 0 || index >= 2 {
            continue;
        }
        let Some(script) = condition_parameter_script(world, item, index - 1)? else {
            continue;
        };
        let word = item.parameter(index);
        let body = world.get::<Script>(script).expect("a script");
        if compiled_flag != 0 {
            // 0056B51C..0056B5A4.
            if script_variable_by_index(body, word).is_none() {
                let text = format!(
                    "TESConditionItem Parameter for {} contains unconverted script variable data -- unable to change because script '{}' is uncompiled.",
                    describe(world, form),
                    name_of(world, script)
                );
                warn(world, services, text);
            }
        } else {
            // 0056B5B4..0056B63F: 004FA840 finds the block whose first dword is the word; a miss warns inside it, then here.
            // 004FA880 returns the block's +0x18 and 0056B5DF tests that, so a found variable with no name warns too.
            let found = body
                .variables_48
                .iter()
                .find(|variable| variable.data_00[..4] == word.to_le_bytes());
            if found.is_none() {
                let text = format!(
                    "Trying to access local variable {word} in script '{}' -- variable not found.\r\n",
                    name_of(world, script)
                );
                warn(world, services, text);
            }
            if !found.is_some_and(|variable| variable.name_18.data_00.is_some()) {
                let text = format!(
                    "Unable to find variableID {word} on script '{}' in TESConditionItem Parameter Init for {}.",
                    name_of(world, script),
                    describe(world, form)
                );
                warn(world, services, text);
            }
        }
    }
    Ok(())
}

// IDA: 00531200 TESTopicInfo_GetResponseText(&out, all), reached only by warnings no shipped record raises. Its 00530C40 reads the info's TRDT and NAM1 chunks back from the file through +0x34.
// TODO: [ ] 00530C40, whose text this leaves empty in those warnings' `%s`.
fn topic_info_text(_world: &World, _info: Entity) -> String {
    String::new()
}

// IDA: 0052F690 the topic of an info (the first on +0x7C whose 0052F5D0(+0x0C, 0) is the info) and 0052F570 the entry's quest (the first entry array holding it, +0x00).
fn topic_and_quest_of(world: &World, info: Entity) -> (Option<Entity>, Option<Entity>) {
    let id = form_id(world, info);
    let topics: Vec<Entity> = world
        .resource::<super::data_handler::DataHandlerLists>()
        .topics_7c
        .items()
        .collect();
    for topic in topics {
        let body = world.get::<TesTopic>(topic).expect("a topic");
        for entry in &body.entries_28 {
            let found = entry
                .infos_04
                .data_04
                .iter()
                .flatten()
                .any(|&held| held == info || form_id(world, held) == id);
            if found {
                let quest = match entry.quest_00 {
                    FormRef::Resolved(quest) => quest,
                    FormRef::Raw(_) => None,
                };
                return (Some(topic), quest);
            }
        }
    }
    (None, None)
}

// IDA: the owner text 0056AE8F..0056AF83 and 0056B010..0056B11A build: an INFO names its topic and quest with a5589c, anything else a67f34 with the table's type name and vt+0xD4.
fn condition_owner_text(world: &World, owner: Entity) -> String {
    let form_type = form_type(world, owner);
    if form_type == 0x3A {
        let (topic, quest) = topic_and_quest_of(world, owner);
        let (topic_id, topic_name) = topic.map_or((0, String::new()), |topic| {
            (form_id(world, topic), name_of(world, topic))
        });
        let (quest_id, quest_name) = quest.map_or((0, String::new()), |quest| {
            (form_id(world, quest), name_of(world, quest))
        });
        return format!(
            "TopicInfo {:08X} \"{}\" in Quest {quest_name} ({quest_id:08x}) Topic {topic_name} ({topic_id:08x})",
            form_id(world, owner),
            topic_info_text(world, owner)
        );
    }
    let name = super::tes_file::FORM_TYPE_TABLE
        .get(usize::from(form_type))
        .map_or("", |(_, name)| *name);
    format!(
        "Form {name} {} ({:08X})",
        name_of(world, owner),
        form_id(world, owner)
    )
}

/// IDA: 0056AE20 TESConditionItem_InitItem(owner): 0056B2E0; a function past the table reads past it in the original (host stop). A zero +0x20 warns a67ed8 with the owner text, the owner's last file's name (+0x1C of 0046B680's answer; a null file faults there, a host stop) and the function name. Then with +0x00 bit 2 and a nonzero +0x04, the comparison maps, casts to TESGlobal and is stored, a null warning a67e88 with the mapped ID and the owner text. Completion: 100%.
pub fn init_condition_item(
    world: &mut World,
    form: Entity,
    item: &mut TesConditionItem,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    resolve_condition_parameters(world, form, item, services)?;
    let row = CONDITION_FUNCTIONS
        .get(usize::from(item.function_08()))
        .ok_or(StreamError("0056AE82: a condition function past the table"))?;
    if row.handler_20 == 0 {
        let owner = condition_owner_text(world, form);
        let file = last_mod_reference(world, form)
            .and_then(|file| world.resource::<DataHandler>().file_store.get(file))
            .map(|file| file.name_1c.clone())
            .ok_or(StreamError("0056AF9E: an owner with no file faults"))?;
        let text = format!(
            "{owner} in file {file} contains bad condition item data. Function \"{}\" is not a condition function.",
            row.name
        );
        warn(world, services, text);
    }
    if item.data_00[0] & 4 != 0 {
        let raw = u32::from_le_bytes(item.data_00[4..8].try_into().expect("four bytes"));
        if raw != 0 {
            let mapped = map_through_owner(world, raw, form)?;
            let found = cast(world, lookup(world, mapped), RttiClass::TESGlobal);
            item.comparison_resolved = Some(found);
            if found.is_none() {
                let text = format!(
                    "Unable to find Value TESGlobal ({mapped:08X}) in TESConditionItem Init for form:\n\n{}.",
                    condition_owner_text(world, form)
                );
                warn(world, services, text);
            }
        }
    }
    Ok(())
}

/// IDA: 0056A480 TESCondition_InitItems(owner) -> 0056AE20 per item. Completion: 100%.
pub fn init_conditions(
    world: &mut World,
    form: Entity,
    condition: &mut TesCondition,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    for item in &mut condition.items {
        init_condition_item(world, form, item, services)?;
    }
    Ok(())
}

// ---- 004A9B70, the flag alone ----

/// IDA: 004A9B70, vt+0x6C of TESGrass, TESObjectTREE, TESCombatStyle and TESEffectShader: without the flag, 0046AB80(1). Completion: 100%.
pub fn init_flag_only(
    world: &mut World,
    form: Entity,
    _services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if !initialized(world, form) {
        set_initialized(world, form, true);
    }
    Ok(())
}

// ---- the list classes ----

/// IDA: 0051F290 TESFaction vt+0x6C. Completion: 100%.
pub fn init_faction(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_reactions(world, form, services)?;
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 00519500 BirthSign vt+0x6C. Completion: 100%.
pub fn init_birth_sign(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_spells(world, form, services)?;
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004C8FB0 TESLandTexture vt+0x6C: each +0x2C slot maps and casts to TESGrass. A hit is stored. A miss warns a464f0, then unlinks: 0065C620 with the raw slot word (`[esp+0xc]`, which 0046BB20 has mapped) when a previous node exists, otherwise the next is pulled in, or the head item cleared. Completion: 100%.
pub fn init_land_texture(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    let mut grasses = std::mem::take(
        &mut world
            .get_mut::<TesLandTexture>(form)
            .expect("a land texture")
            .grasses_2c,
    );
    let mut index = 0;
    let mut previous: Option<usize> = None;
    while index < grasses.len() {
        let mapped = map_through_owner(world, raw_of(grasses[index])?, form)?;
        if let Some(found) = cast(world, lookup(world, mapped), RttiClass::TESGrass) {
            grasses[index] = FormRef::Resolved(Some(found));
            previous = Some(index);
            index += 1;
            continue;
        }
        let text = format!(
            "Could not find Grass ({mapped:08X}) for LandTexture '{}' ({:08X}).",
            name_of(world, form),
            form_id(world, form)
        );
        warn(world, services, text);
        if let Some(before) = previous {
            remove_after_previous(&mut grasses, before, mapped)?;
            index = before + 1;
        } else {
            grasses.remove(index);
        }
    }
    world
        .get_mut::<TesLandTexture>(form)
        .expect("a land texture")
        .grasses_2c = grasses;
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004F9890 TESLoadScreen vt+0x6C: over a nonempty list, each block's first two words, when nonzero, map in place through the last file; no lookup. Completion: 100%.
pub fn init_load_screen(
    world: &mut World,
    form: Entity,
    _services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    let mut locations = std::mem::take(
        &mut world
            .get_mut::<TesLoadScreen>(form)
            .expect("a load screen")
            .locations_2c,
    );
    for block in &mut locations {
        if block[0] != 0 {
            block[0] = map_through_owner(world, block[0], form)?;
        }
        if block[1] != 0 {
            block[1] = map_through_owner(world, block[1], form)?;
        }
    }
    world
        .get_mut::<TesLoadScreen>(form)
        .expect("a load screen")
        .locations_2c = locations;
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004B4320 TESObjectANIO vt+0x6C: a nonzero +0x30 maps; 0055E000 finds it: the TESIdleForm cast goes in the slot; a miss stores 0. No warning. Completion: 100%.
pub fn init_object_anio(
    world: &mut World,
    form: Entity,
    _services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    let slot = world.get::<TesObjectAnio>(form).expect("an ANIO").idle_30;
    if !slot.is_empty() {
        let mapped = map_through_owner(world, raw_of(slot)?, form)?;
        let found = cast(world, lookup(world, mapped), RttiClass::TESIdleForm);
        world
            .get_mut::<TesObjectAnio>(form)
            .expect("an ANIO")
            .idle_30 = FormRef::Resolved(found);
    }
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004ED830 TESWaterForm vt+0x6C: a nonzero +0x38 maps and casts to TESSound (stored; a null warns a47eb8), then each of the three +0xA0 slots casts to TESWaterForm (stored; a null warns a47e7c). Completion: 100%.
pub fn init_water(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    let body = world.get::<TesWaterForm>(form).expect("a water");
    let (sound, related) = (body.sound_38, body.related_a0);
    if !sound.is_empty() {
        let mapped = map_through_owner(world, raw_of(sound)?, form)?;
        let found = cast(world, lookup(world, mapped), RttiClass::TESSound);
        world
            .get_mut::<TesWaterForm>(form)
            .expect("a water")
            .sound_38 = FormRef::Resolved(found);
        if found.is_none() {
            let text = format!(
                "Could not find sound ({mapped:08X}) for water type '{}' ({:08X}).",
                name_of(world, form),
                form_id(world, form)
            );
            warn(world, services, text);
        }
    }
    for (index, slot) in related.into_iter().enumerate() {
        if slot.is_empty() {
            continue;
        }
        let mapped = map_through_owner(world, raw_of(slot)?, form)?;
        let found = cast(world, lookup(world, mapped), RttiClass::TESWaterForm);
        world
            .get_mut::<TesWaterForm>(form)
            .expect("a water")
            .related_a0[index] = FormRef::Resolved(found);
        if found.is_none() {
            let text = format!(
                "Could not find WaterForm ({mapped:08X}) for water type '{}' ({:08X}).",
                name_of(world, form),
                form_id(world, form)
            );
            warn(world, services, text);
        }
    }
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004A3430 TESRegion vt+0x6C: +0x20 maps and casts to TESWorldSpace. Only a hit is stored, so a miss keeps the raw word. With +0x08 bit 6 and DataHandler +0xCD7 clear, 004EF170(1) sets the world's +0x5C bit 3. Completion: 100%.
pub fn init_region(
    world: &mut World,
    form: Entity,
    _services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    let slot = world
        .get::<TesRegion>(form)
        .expect("a region")
        .worldspace_20;
    let mapped = map_through_owner(world, raw_of(slot)?, form)?;
    if let Some(found) = cast(world, lookup(world, mapped), RttiClass::TESWorldSpace) {
        world
            .get_mut::<TesRegion>(form)
            .expect("a region")
            .worldspace_20 = FormRef::Resolved(Some(found));
        let flags = world.get::<FormIdentity>(form).expect("a form").flags_08;
        if flags & 0x40 != 0 && world.resource::<DataHandler>().loading_cd7 == 0 {
            world
                .get_mut::<TesWorldSpace>(found)
                .expect("a world")
                .flags_5c |= 8;
        }
    }
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 00520490 TESIdleForm vt+0x6C. Completion: 100%.
pub fn init_idle(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    let mut conditions = std::mem::take(
        &mut world
            .get_mut::<TesIdleForm>(form)
            .expect("an idle")
            .conditions_30,
    );
    let result = init_conditions(world, form, &mut conditions, services);
    world
        .get_mut::<TesIdleForm>(form)
        .expect("an idle")
        .conditions_30 = conditions;
    result?;
    set_initialized(world, form, true);
    Ok(())
}

// IDA: the warning of a package object or target, by the owner's name.
fn package_warning(world: &World, form: Entity, what: &str, mapped: u32) -> String {
    let name = name_of(world, form);
    if name.is_empty() {
        format!(
            "Unable to find Package {what} ({mapped:08X}) on owner object ({:08X}).",
            form_id(world, form)
        )
    } else {
        format!("Unable to find Package {what} ({mapped:08X}) on owner object \"{name}\".")
    }
}

// IDA: 00569B20 PackageLocation_InitItem(owner): type 5 returns. +0x08 maps through a local. Types 0 and 1 cast to TESObjectREFR and TESObjectCELL, 4 takes the lookup; each stores what it found, with a null warning first and the store a 0. Other types leave the word: the mapped local goes nowhere.
fn init_package_location(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let Some(location) = world
        .get::<TesPackage>(form)
        .expect("a package")
        .location_24
    else {
        return Ok(());
    };
    if location.type_00 == 5 {
        return Ok(());
    }
    let mapped = map_through_owner(world, raw_of(location.object_08)?, form)?;
    let (found, what) = match location.type_00 {
        0 => (
            cast(world, lookup(world, mapped), RttiClass::TESObjectREFR),
            "Location Reference",
        ),
        1 => (
            cast(world, lookup(world, mapped), RttiClass::TESObjectCELL),
            "Location Cell",
        ),
        4 => (lookup(world, mapped), "Location Object"),
        _ => return Ok(()),
    };
    if found.is_none() {
        warn(world, services, package_warning(world, form, what, mapped));
    }
    world
        .get_mut::<TesPackage>(form)
        .expect("a package")
        .location_24
        .as_mut()
        .expect("read above")
        .object_08 = FormRef::Resolved(found);
    Ok(())
}

// IDA: 0056A140 PackageTarget_InitItem(owner): +0x04 maps through a local. Type 0 casts to TESObjectREFR, type 1 takes the lookup; the slot takes the result and a null warns. Other types leave the word.
fn init_package_target(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let Some(target) = world.get::<TesPackage>(form).expect("a package").target_28 else {
        return Ok(());
    };
    let mapped = map_through_owner(world, raw_of(target.value_04)?, form)?;
    let (found, what) = match target.type_00 {
        0 => (
            cast(world, lookup(world, mapped), RttiClass::TESObjectREFR),
            "Target Reference",
        ),
        1 => (lookup(world, mapped), "Target Object"),
        _ => return Ok(()),
    };
    world
        .get_mut::<TesPackage>(form)
        .expect("a package")
        .target_28
        .as_mut()
        .expect("read above")
        .value_04 = FormRef::Resolved(found);
    if found.is_none() {
        warn(world, services, package_warning(world, form, what, mapped));
    }
    Ok(())
}

/// IDA: 005667F0 TESPackage vt+0x6C: 00569B20 with a location, 0056A140 with a target, 0056A480 on +0x34. Completion: 100%.
pub fn init_package(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_package_location(world, form, services)?;
    init_package_target(world, form, services)?;
    let mut conditions = std::mem::take(
        &mut world
            .get_mut::<TesPackage>(form)
            .expect("a package")
            .conditions_34,
    );
    let result = init_conditions(world, form, &mut conditions, services);
    world
        .get_mut::<TesPackage>(form)
        .expect("a package")
        .conditions_34 = conditions;
    result?;
    set_initialized(world, form, true);
    Ok(())
}

// IDA: one of 0052BA30's four slots: nonzero maps, casts, stored; a null warns `text`.
fn init_race_slot(
    world: &mut World,
    form: Entity,
    slot: FormRef,
    class: RttiClass,
    text: &str,
    services: &mut DataLoadServices<'_>,
) -> Result<FormRef, StreamError> {
    if slot.is_empty() {
        return Ok(slot);
    }
    let mapped = map_through_owner(world, raw_of(slot)?, form)?;
    let found = cast(world, lookup(world, mapped), class);
    if found.is_none() {
        let text = format!(
            "{text} ({mapped:08X}) for race '{}' ({:08X}).",
            name_of(world, form),
            form_id(world, form)
        );
        warn(world, services, text);
    }
    Ok(FormRef::Resolved(found))
}

/// IDA: 0052BA30 TESRace vt+0x6C: +0x300, +0x304 (TESRace), +0x94, +0x98 (TESHair), 0046FC10, 0046E6B0, then 0052B6A0 once by 00B3630C. Completion: 100% of the form's stores.
pub fn init_race(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    let body = world.get::<TesRace>(form).expect("a race");
    let (voices, hairs) = (body.voices_300, body.default_hairs_94);
    let male_voice = init_race_slot(
        world,
        form,
        voices[0],
        RttiClass::TESRace,
        "Could not find male voice race",
        services,
    )?;
    world.get_mut::<TesRace>(form).expect("a race").voices_300[0] = male_voice;
    let female_voice = init_race_slot(
        world,
        form,
        voices[1],
        RttiClass::TESRace,
        "Couldnot find female voice race",
        services,
    )?;
    world.get_mut::<TesRace>(form).expect("a race").voices_300[1] = female_voice;
    let male_hair = init_race_slot(
        world,
        form,
        hairs[0],
        RttiClass::TESHair,
        "Could not find male default hair",
        services,
    )?;
    world
        .get_mut::<TesRace>(form)
        .expect("a race")
        .default_hairs_94[0] = male_hair;
    let female_hair = init_race_slot(
        world,
        form,
        hairs[1],
        RttiClass::TESHair,
        "Could not find female default hair",
        services,
    )?;
    world
        .get_mut::<TesRace>(form)
        .expect("a race")
        .default_hairs_94[1] = female_hair;
    init_spells(world, form, services)?;
    init_reactions(world, form, services)?;
    init_race_statics(world);
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 00B3630C, 0052B6A0's once byte, and the thirteen static TESModels it fills, by the +0x04 path of each: 00B36380, 00B36398, 00B363B0, 00B363C8, 00B363F8, 00B36410, 00B36428, 00B36440, 00B36470, 00B36488, 00B364A0, 00B364B8, 00B364D0.
#[derive(Resource, Debug, Default)]
pub struct RaceStatics {
    pub set_b3630c: bool,
    pub model_paths: Vec<BsStringT>,
}

// IDA: 0052B6A0 once: each 46E080 (004028D0(+0x04, text, 0)) takes the text of a string GameSetting, 00B36310 to 00B36370, every 8 bytes (9E3D0A.. their constructors: sRaceMaleUpperBodyModel and on).
fn init_race_statics(world: &mut World) {
    if world.get_resource_or_init::<RaceStatics>().set_b3630c {
        return;
    }
    let mut paths = Vec::with_capacity(13);
    for index in 0..13 {
        let text = world
            .get_resource::<GameSettingCollection>()
            .and_then(|settings| settings.text_at(0x00B3_6310 + 8 * index));
        let mut path = BsStringT::default();
        path.set(text.as_deref(), 0);
        paths.push(path);
    }
    let mut statics = world.resource_mut::<RaceStatics>();
    statics.model_paths = paths;
    statics.set_b3630c = true;
}

// ---- scripts and quests ----

/// IDA: the quest delay table of 004FBB60: 00B361D4..00B361F3, eight halvings of 00B09E28, and the use counter 00B361F4.
#[derive(Resource, Debug)]
pub struct QuestDelayGlobals {
    pub delays_b361d4: [f32; 8],
    pub counter_b361f4: u32,
    /// 00B09E28 fQuestScriptDelayTime:MAIN: 5.0 in the image, bytes 00B09E28.
    pub quest_script_delay_time_b09e28: f32,
}

impl Default for QuestDelayGlobals {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            delays_b361d4: [0.0; 8],
            counter_b361f4: 0,
            quest_script_delay_time_b09e28: f32::from_bits(0x40A0_0000),
        }
    }
}

/// IDA: 004FBB60 Script_InitItem(owner). The flag returns; a null owner returns. Each reference: a nonzero +0x08 maps and looks up, stored; a zero tests +0x0C. A zero answer unlinks the node (0065C620 with the node's block when a previous exists, otherwise the next is pulled in or the head item cleared), warns a49f48 for a form that wasn't found or a49ef0 otherwise, and clears the compiled size +0x20. A count that differs from +0x1C with a nonzero +0x20 warns a49e90 and clears it too. A quest script (+0x28 set) with 00B09E28 above 0.0 takes a delay: a zero counter first fills the table; a variable named fQuestDelayTime (004FAA90, _stricmp) gives +0x38 = 0.0; a zero low byte gives the setting; otherwise the table entries of the set bits add onto +0x38. Each counts. The flag is set. Completion: 100%.
pub fn init_script_form(
    world: &mut World,
    form: Entity,
    owner: Option<Entity>,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    let Some(owner) = owner else {
        return Ok(());
    };
    let file = last_mod_reference(world, owner);
    let mut references = std::mem::take(
        &mut world
            .get_mut::<Script>(form)
            .expect("a script")
            .references_40,
    );
    let mut count = 0_u32;
    let mut index = 0;
    while index < references.len() {
        let raw = references[index].form_08;
        let mapped = if raw != 0 {
            map_id(world, raw, file)?
        } else {
            0
        };
        let good = if raw != 0 {
            let found = lookup(world, mapped);
            references[index].resolved_08 = Some(found);
            found.is_some()
        } else {
            references[index].variable_0c != 0
        };
        if good {
            index += 1;
        } else {
            // The block unlinks by itself either way: 0065C620 compares the block pointer.
            let (name, id) = (name_of(world, form), form_id(world, form));
            references.remove(index);
            let text = if raw != 0 {
                format!(
                    "Could not find referenced object ({mapped:08X}) on Script ({id:08X}) '{name}'.  Script will not be executed."
                )
            } else {
                format!(
                    "Referenced object {count} on Script ({id:08X}) '{name}' is invalid.  Script will not be executed."
                )
            };
            warn(world, services, text);
            world.get_mut::<Script>(form).expect("a script").header_18[8..12].fill(0);
        }
        count += 1;
    }
    let (expected, compiled, is_quest) = {
        let mut script = world.get_mut::<Script>(form).expect("a script");
        script.references_40 = references;
        (
            u32::from_le_bytes(script.header_18[4..8].try_into().expect("four bytes")),
            u32::from_le_bytes(script.header_18[8..12].try_into().expect("four bytes")),
            script.header_18[0x10] != 0,
        )
    };
    if count != expected && compiled != 0 {
        let text = format!(
            "Referenced object list on Script ({:08X}) '{}' is corrupt.  Expected {expected} objects, found {count}.",
            form_id(world, form),
            name_of(world, form)
        );
        warn(world, services, text);
        world.get_mut::<Script>(form).expect("a script").header_18[8..12].fill(0);
    }
    if is_quest {
        apply_quest_delay(world, form);
    }
    set_initialized(world, form, true);
    Ok(())
}

// IDA: 004FBCB0..004FBD8C, at 24 bits: `fmul` by the qword 0.5 of A2FAA0 is exact, and each halving is stored as a float before the next. +0x38 is `floats_34[1]`. `fcom` with `test ah, 0x41`: a NaN or a value at or below 0.0 skips.
fn apply_quest_delay(world: &mut World, form: Entity) {
    world.init_resource::<QuestDelayGlobals>();
    let mut globals = world.resource_mut::<QuestDelayGlobals>();
    let setting = globals.quest_script_delay_time_b09e28;
    if setting.is_nan() || setting <= 0.0 {
        return;
    }
    if globals.counter_b361f4 == 0 {
        let mut value = setting;
        for slot in &mut globals.delays_b361d4 {
            value = (f64::from(value) * 0.5) as f32;
            *slot = value;
        }
    }
    let low = globals.counter_b361f4 & 0xFF;
    let delays = globals.delays_b361d4;
    globals.counter_b361f4 = globals.counter_b361f4.wrapping_add(1);
    let mut script = world.get_mut::<Script>(form).expect("a script");
    let named = script.variables_48.iter().any(|variable| {
        variable
            .name_18
            .data_00
            .as_deref()
            .is_some_and(|name| crt::stricmp(name, b"fQuestDelayTime") == 0)
    });
    if named {
        script.floats_34[1] = 0.0;
    } else if low == 0 {
        script.floats_34[1] = setting;
    } else {
        let mut bits = low;
        let mut at = 0;
        while bits != 0 {
            if bits & 1 != 0 {
                script.floats_34[1] += delays[at];
            }
            at += 1;
            bits >>= 1;
        }
    }
}

/// IDA: 004FBDB0 Script vt+0x6C: 004FBB60 with the stack word `push ecx` leaves as its owner: `this`. Completion: 100%.
pub fn init_script_item(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    init_script_form(world, form, Some(form), services)
}

/// IDA: 004FBDC0 ScriptEventList_Create(script): FormHeap(0x14): +0x04, +0x08, +0x0C, +0x00 and +0x10 clear; 004FA910's list of 0x18 copies of each +0x48 block, each its +0x10 byte, +0x00 dword and +0x08 qword, in list order; +0x00 the script; +0x0C the list. Completion: 100%.
#[must_use]
pub fn create_script_event_list(world: &World, script: Entity) -> ScriptEventList {
    let body = world.get::<Script>(script).expect("a script");
    let variables = body
        .variables_48
        .iter()
        .map(|variable| ScriptEventVariable {
            id_00: u32::from_le_bytes(variable.data_00[..4].try_into().expect("four bytes")),
            // 004FA963 `fld qword`, `fstp qword`, which quiets an sNaN.
            value_08: x87::fld_fstp_copy_f64(u64::from_le_bytes(
                variable.data_00[8..16].try_into().expect("eight bytes"),
            )),
            byte_10: variable.data_00[0x10],
        })
        .collect();
    ScriptEventList {
        flags_04: 0,
        script_00: script,
        events_08: None,
        variables_0c: variables,
    }
}

/// IDA: vt+0x48 0046AC30 -> 0045B700(g_saveLoadGame, form, flags): a temporary form returns. `TesSaveLoadGame::record_form_change` chooses the map. Completion: 100%.
/// # Errors
/// A null g_saveLoadGame.
pub fn record_form_change(world: &mut World, form: Entity, flags: u32) -> Result<(), StreamError> {
    let identity = *world.get::<FormIdentity>(form).expect("a form");
    if identity.flags_08 & FLAG_TEMPORARY != 0 {
        return Ok(());
    }
    world
        .resource_mut::<super::tes::TesGlobals>()
        .save_load_game
        .as_mut()
        .ok_or(StreamError("0045B70E: g_saveLoadGame is null"))?
        .record_form_change(identity.form_id_0c, flags);
    Ok(())
}

/// IDA: 00529860 TESQuest vt+0x6C: 0046F020 on +0x18; a nonzero script gets a ScriptEventList in +0x58 and vt+0x48(0x8000000); with 00B10CA0, +0x3C bit 0 and vt+0x40(4). 0056A480 on +0x50; each stage (0052B160: each item, 0052B0C0); each target (0052B340). Completion: 100%.
pub fn init_quest(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_script(world, form, services)?;
    let script = world
        .get::<TesScriptableForm>(form)
        .expect("a quest")
        .script_04;
    if let FormRef::Resolved(Some(script)) = script {
        let events = create_script_event_list(world, script);
        world.get_mut::<TesQuest>(form).expect("a quest").events_58 = Some(events);
        record_form_change(world, form, 0x800_0000)?;
        if world.resource::<DataLoadGlobals>().quest_flag_b10ca0 != 0 {
            world.get_mut::<TesQuest>(form).expect("a quest").flags_3c |= 1;
            set_change_flags(world, form, 4)?;
        }
    }
    let mut conditions = std::mem::take(
        &mut world
            .get_mut::<TesQuest>(form)
            .expect("a quest")
            .conditions_50,
    );
    let result = init_conditions(world, form, &mut conditions, services);
    world
        .get_mut::<TesQuest>(form)
        .expect("a quest")
        .conditions_50 = conditions;
    result?;
    let mut stages =
        std::mem::take(&mut world.get_mut::<TesQuest>(form).expect("a quest").stages_40);
    let result = init_quest_stages(world, form, &mut stages, services);
    world.get_mut::<TesQuest>(form).expect("a quest").stages_40 = stages;
    result?;
    let mut targets =
        std::mem::take(&mut world.get_mut::<TesQuest>(form).expect("a quest").targets_48);
    let result = init_quest_targets(world, form, &mut targets, services);
    world.get_mut::<TesQuest>(form).expect("a quest").targets_48 = targets;
    result?;
    set_initialized(world, form, true);
    Ok(())
}

// IDA: 0052B160 per stage, 0052B0C0(quest, stage) per item: 0056A480 on +0x04; with g_disableWarningMessages set and 00B34D88 cleared, both restored after, 004FBB60(quest) on the embedded script; a nonzero count warns a54338, which 00B34D88 can't reach (`WarningCounter`). +0x68 = the quest's TESQuest cast.
fn init_quest_stages(
    world: &mut World,
    form: Entity,
    stages: &mut [super::forms_dialogue::QuestStageGroup],
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    for stage in stages {
        let index = stage.index_00;
        for item in &mut stage.items_04 {
            init_conditions(world, form, &mut item.conditions_04, services)?;
            let saved_flag = world
                .resource::<DataLoadGlobals>()
                .disable_warning_messages_b06b18;
            let saved_count = world.get_resource_or_init::<WarningCounter>().count_b34d88;
            world
                .resource_mut::<DataLoadGlobals>()
                .disable_warning_messages_b06b18 = 1;
            world.resource_mut::<WarningCounter>().count_b34d88 = 0;
            let result = init_script_form(world, item.script_0c, Some(form), services);
            let count = world.resource::<WarningCounter>().count_b34d88;
            world
                .resource_mut::<DataLoadGlobals>()
                .disable_warning_messages_b06b18 = saved_flag;
            world.resource_mut::<WarningCounter>().count_b34d88 = saved_count;
            result?;
            if count != 0 {
                let text = format!(
                    "Errors were encountered during InitItem for result script on quest stage {index}, quest '{}' ({:08X})\n\nSee Warnings file for more information.",
                    name_of(world, form),
                    form_id(world, form)
                );
                warn(world, services, text);
            }
            item.quest_68 = FormRef::Resolved(Some(form));
        }
    }
    Ok(())
}

// IDA: 0052B340 QuestTarget_InitItem(quest): a nonzero +0x0C maps and casts to TESObjectREFR, stored; a null warns a543f0. A zero warns a543c0. 0056A480 on +0x04.
fn init_quest_targets(
    world: &mut World,
    form: Entity,
    targets: &mut [super::forms_dialogue::QuestTarget],
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    for target in targets {
        if target.target_0c.is_empty() {
            let text = format!(
                "No reference on target for quest ({:08X}) '{}'.",
                form_id(world, form),
                name_of(world, form)
            );
            warn(world, services, text);
        } else {
            let mapped = map_through_owner(world, raw_of(target.target_0c)?, form)?;
            let found = cast(world, lookup(world, mapped), RttiClass::TESObjectREFR);
            target.target_0c = FormRef::Resolved(found);
            if found.is_none() {
                let text = format!(
                    "Could not find target reference ({mapped:08X}) on quest ({:08X}) '{}'.",
                    form_id(world, form),
                    name_of(world, form)
                );
                warn(world, services, text);
            }
        }
        init_conditions(world, form, &mut target.conditions_04, services)?;
    }
    Ok(())
}

/// IDA: 0052EE30 TESTopic vt+0x6C: each entry, unless +0x20 is set: a nonzero +0x00 is 0046B250 and the TESQuest cast, with no mapping since 005301E0's QSTI mapped it (the C++ maps again); a nonzero +0x1C maps and casts into +0x1C; otherwise the entry's non-deleted infos are counted and a positive count warns a55480. +0x20 = 1. Then 005A56F0 compacts the array, and each info's vt+0x6C runs, then its +0x20 takes the index. Completion: 100%.
pub fn init_topic(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    let count = world
        .get::<TesTopic>(form)
        .expect("a topic")
        .entries_28
        .len();
    for index in 0..count {
        let entry = world.get::<TesTopic>(form).expect("a topic").entries_28[index].clone();
        if entry.resolved_20 == 0 {
            if !entry.quest_00.is_empty() {
                let found = cast(
                    world,
                    lookup(world, raw_of(entry.quest_00)?),
                    RttiClass::TESQuest,
                );
                world.get_mut::<TesTopic>(form).expect("a topic").entries_28[index].quest_00 =
                    FormRef::Resolved(found);
            } else if !entry.quest_1c.is_empty() {
                let mapped = map_through_owner(world, raw_of(entry.quest_1c)?, form)?;
                let found = cast(world, lookup(world, mapped), RttiClass::TESQuest);
                world.get_mut::<TesTopic>(form).expect("a topic").entries_28[index].quest_1c =
                    FormRef::Resolved(found);
            } else {
                let undeleted = entry
                    .infos_04
                    .data_04
                    .iter()
                    .take(entry.infos_04.count_10 as usize)
                    .flatten()
                    .filter(|&&info| {
                        world.get::<FormIdentity>(info).expect("a form").flags_08 & 0x20 == 0
                    })
                    .count();
                if undeleted > 0 {
                    let text = format!(
                        "No Quest Reference on Topic \"{}\" ({undeleted} non-deleted infos attached to this quest).",
                        name_of(world, form)
                    );
                    warn(world, services, text);
                }
            }
            world.get_mut::<TesTopic>(form).expect("a topic").entries_28[index].resolved_20 = 1;
        }
        let infos = {
            let mut topic = world.get_mut::<TesTopic>(form).expect("a topic");
            let array = &mut topic.entries_28[index].infos_04;
            array.compact();
            array.data_04.clone()
        };
        for (at, info) in infos.into_iter().enumerate() {
            if let Some(info) = info {
                init_topic_info(world, info, services)?;
                world
                    .get_mut::<TesTopicInfo>(info)
                    .expect("an info")
                    .index_20 = at as u16;
            }
        }
    }
    set_initialized(world, form, true);
    Ok(())
}

// IDA: one list of 005318F0 or 005314D0: each ID maps and casts to TESTopic. A hit is stored. A miss unlinks (0065C620 with the mapped ID when a previous node exists, otherwise the next is pulled in, or the head item cleared), and `warn` runs.
fn init_topic_list(
    world: &mut World,
    form: Entity,
    list: &mut Vec<FormRef>,
    services: &mut DataLoadServices<'_>,
    miss: &dyn Fn(&World, u32) -> String,
) -> Result<(), StreamError> {
    let mut index = 0;
    let mut previous: Option<usize> = None;
    while index < list.len() {
        let mapped = map_through_owner(world, raw_of(list[index])?, form)?;
        if let Some(found) = cast(world, lookup(world, mapped), RttiClass::TESTopic) {
            list[index] = FormRef::Resolved(Some(found));
            previous = Some(index);
            index += 1;
            continue;
        }
        // 005319B1, 0053159E, 00531725 test the next node first: one pulls in over this node, and only a last node with a previous one goes through 0065C620.
        if index + 1 < list.len() {
            list.remove(index);
        } else if let Some(before) = previous {
            remove_after_previous(list, before, mapped)?;
            index = before + 1;
        } else {
            list.remove(index);
        }
        let text = miss(world, mapped);
        warn(world, services, text);
    }
    Ok(())
}

// IDA: 00531A28..00531A92: a5589c through `condition_owner_text` with both a topic and a quest, otherwise a558f8.
fn topic_info_owner_text(world: &World, form: Entity) -> String {
    let (topic, quest) = topic_and_quest_of(world, form);
    if topic.is_some() && quest.is_some() {
        return condition_owner_text(world, form);
    }
    format!(
        "TopicInfo {:08X} \"{}\"",
        form_id(world, form),
        topic_info_text(world, form)
    )
}

/// IDA: 005318F0 TESTopicInfo vt+0x6C: +0x28's list (warning a558d4), 0056A480 on +0x18, with a +0x30 block 005314D0: its +0x08 list (a55858), then +0x00 (a55810). Completion: 100%.
pub fn init_topic_info(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    let mut names = std::mem::take(
        &mut world
            .get_mut::<TesTopicInfo>(form)
            .expect("an info")
            .names_28,
    );
    let result = init_topic_list(world, form, &mut names, services, &|world, id| {
        format!(
            "Unable to find topic ({id:08X}) for {}",
            topic_info_owner_text(world, form)
        )
    });
    world
        .get_mut::<TesTopicInfo>(form)
        .expect("an info")
        .names_28 = names;
    result?;
    let mut conditions = std::mem::take(
        &mut world
            .get_mut::<TesTopicInfo>(form)
            .expect("an info")
            .conditions_18,
    );
    let result = init_conditions(world, form, &mut conditions, services);
    world
        .get_mut::<TesTopicInfo>(form)
        .expect("an info")
        .conditions_18 = conditions;
    result?;
    if let Some(mut choices) = world
        .get_mut::<TesTopicInfo>(form)
        .expect("an info")
        .choices_30
        .take()
    {
        let result = init_topic_list(
            world,
            form,
            &mut choices.choices_08,
            services,
            &|world, id| {
                format!(
                    "Unable to find topic ({id:08X}) for conversation data 'Link To' on {}",
                    condition_owner_text(world, form)
                )
            },
        )
        .and_then(|()| {
            init_topic_list(
                world,
                form,
                &mut choices.link_from_00,
                services,
                &|world, id| {
                    format!(
                        "Unable to find topic ({id:08X}) for conversation data 'Link From' on {}",
                        condition_owner_text(world, form)
                    )
                },
            )
        });
        world
            .get_mut::<TesTopicInfo>(form)
            .expect("an info")
            .choices_30 = Some(choices);
        result?;
    }
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 00415960 EffectSetting vt+0x6C: with +0x58 in 0x70000 and clear of 0x180000, +0x60 maps in place. +0x70 maps and casts to TESObjectLIGH; +0x80, +0x84, +0x88, +0x8C to TESSound; +0x78, +0x7C to TESEffectShader; each nonzero, stored with no warning. Completion: 100%.
pub fn init_effect_setting(
    world: &mut World,
    form: Entity,
    _services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    let data = world.get::<EffectSetting>(form).expect("a setting").data_58;
    if data.flags_58 & 0x7_0000 != 0 && data.flags_58 & 0x18_0000 == 0 {
        let mapped = map_through_owner(world, data.associated_60, form)?;
        world
            .get_mut::<EffectSetting>(form)
            .expect("a setting")
            .data_58
            .associated_60 = mapped;
    }
    let resolve =
        |world: &World, slot: FormRef, class: RttiClass| -> Result<FormRef, StreamError> {
            if slot.is_empty() {
                return Ok(slot);
            }
            let mapped = map_through_owner(world, raw_of(slot)?, form)?;
            Ok(FormRef::Resolved(cast(world, lookup(world, mapped), class)))
        };
    let light = resolve(world, data.light_70, RttiClass::TESObjectLIGH)?;
    world
        .get_mut::<EffectSetting>(form)
        .expect("a setting")
        .data_58
        .light_70 = light;
    for index in [2, 3, 4, 5, 0, 1] {
        let class = if index >= 2 {
            RttiClass::TESSound
        } else {
            RttiClass::TESEffectShader
        };
        let slot = resolve(world, data.shaders_and_sounds_78[index], class)?;
        world
            .get_mut::<EffectSetting>(form)
            .expect("a setting")
            .data_58
            .shaders_and_sounds_78[index] = slot;
    }
    set_initialized(world, form, true);
    Ok(())
}

// ---- the TESObject chain ----

/// IDA: 004B3D30 TESObjectACTI vt+0x6C: 0046F020, then +0x54 maps and casts to TESSound, stored, a null warning a43c2c. Completion: 100%.
pub fn init_activator(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_script(world, form, services)?;
    let slot = world
        .get::<TesActivator>(form)
        .expect("an activator")
        .sound_54;
    if !slot.is_empty() {
        let mapped = map_through_owner(world, raw_of(slot)?, form)?;
        let found = cast(world, lookup(world, mapped), RttiClass::TESSound);
        world
            .get_mut::<TesActivator>(form)
            .expect("an activator")
            .sound_54 = FormRef::Resolved(found);
        if found.is_none() {
            let text = format!(
                "Could not find open sound ({mapped:08X}) for activator '{}' ({:08X}).",
                name_of(world, form),
                form_id(world, form)
            );
            warn(world, services, text);
        }
    }
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004AE070 TESFlora vt+0x6C: 004B3D30, then 0046E220. Completion: 100%.
pub fn init_flora(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_activator(world, form, services)?;
    init_produce(world, form, services)?;
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004AEC30 TESFurniture vt+0x6C, 004AE830: a temporary form skips. A zero +0x58 warns a41e74 or, when 006FB0A0 of vt+0x114 finds no model, a41eac, then runs vt+0xF0, which searches the model's markers: a host stop past the warning. A nonzero +0x58 with neither top bit warns a41e3c. Completion: 100% but vt+0xF0.
/// TODO: [ ] 004AE893 vt+0xF0, with the NIF loaded (no shipped furniture reaches it).
pub fn init_furniture(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    let flags = world.get::<FormIdentity>(form).expect("a form").flags_08;
    if flags & FLAG_TEMPORARY == 0 {
        let markers = world
            .get::<TesFurniture>(form)
            .expect("a furniture")
            .marker_flags_58;
        if markers == 0 {
            return Err(StreamError(
                "004AE844: a furniture with no marker flags loads its model",
            ));
        }
        if markers & 0xC000_0000 == 0 {
            let text = format!(
                "Furniture '{}' is not marked for sitting or sleeping.",
                name_of(world, form)
            );
            warn(world, services, text);
        }
    }
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004B4590 vt+0x6C of APPA, MISC, KEYM, SLGM and SGST: 0046F020 on the class's scriptable part. Completion: 100%.
pub fn init_scriptable_object(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_script(world, form, services)?;
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004B4C20 TESObjectARMO vt+0x6C and 004B5C60 TESObjectCLOT's: a zero slot mask warns (a440d4 or a44578), 0046F020, 0046A880. Completion: 100%.
pub fn init_armor(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    init_biped_object(world, form, "Armor", services)
}

/// IDA: 004B5C60 TESObjectCLOT vt+0x6C. Completion: 100%.
pub fn init_clothing(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    init_biped_object(world, form, "Clothing", services)
}

// IDA: N/A a helper, with no address of its own.
fn init_biped_object(
    world: &mut World,
    form: Entity,
    what: &str,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    if world
        .get::<TesBipedModelForm>(form)
        .expect("a biped object")
        .slot_mask_04
        == 0
    {
        let text = format!(
            "{what} '{}' needs to have biped slots selected in the editor.",
            name_of(world, form)
        );
        warn(world, services, text);
    }
    init_script(world, form, services)?;
    init_enchantment(world, form, services)?;
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004B53E0 TESObjectBOOK vt+0x6C: 0046F020, 0046A880, then a +0x89 that isn't 0xFF is 00565BE0(2, it): the byte sign-extended + 12, and outside 12..=33 it becomes 0xFF. Completion: 100%.
pub fn init_book(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_script(world, form, services)?;
    init_enchantment(world, form, services)?;
    let mut book = world.get_mut::<TesBook>(form).expect("a book");
    if book.teaches_89 != 0xFF {
        let code = i32::from(book.teaches_89 as i8) + 12;
        if !(12..=33).contains(&code) {
            book.teaches_89 = 0xFF;
        }
    }
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004B6290 TESObjectCONT vt+0x6C: 0046F020, 00469E90, then +0x70 and +0x74 map and cast to TESSound, stored; a null warns a44764 for both. Completion: 100%.
pub fn init_container_object(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_script(world, form, services)?;
    init_container(world, form, services)?;
    let body = *world.get::<TesContainerObject>(form).expect("a container");
    for (index, slot) in [body.open_sound_70, body.close_sound_74]
        .into_iter()
        .enumerate()
    {
        if slot.is_empty() {
            continue;
        }
        let mapped = map_through_owner(world, raw_of(slot)?, form)?;
        let found = FormRef::Resolved(cast(world, lookup(world, mapped), RttiClass::TESSound));
        {
            let mut body = world
                .get_mut::<TesContainerObject>(form)
                .expect("a container");
            if index == 0 {
                body.open_sound_70 = found;
            } else {
                body.close_sound_74 = found;
            }
        }
        if found == FormRef::Resolved(None) {
            let text = format!(
                "Could not find open sound ({mapped:08X}) for container '{}' ({:08X}).",
                name_of(world, form),
                form_id(world, form)
            );
            warn(world, services, text);
        }
    }
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004B6DB0 TESObjectDOOR vt+0x6C: 0046F020, then each +0x68 slot maps and looks up, no cast. A hit is stored. A miss warns a44984 and unlinks: 0065C620 with the mapped ID when a previous node exists, otherwise the next is pulled in, or the head item cleared. Completion: 100%.
pub fn init_door(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_script(world, form, services)?;
    let mut teleports =
        std::mem::take(&mut world.get_mut::<TesDoor>(form).expect("a door").teleports_68);
    let mut index = 0;
    let mut previous: Option<usize> = None;
    let mut result = Ok(());
    while index < teleports.len() {
        let mapped =
            match raw_of(teleports[index]).and_then(|raw| map_through_owner(world, raw, form)) {
                Ok(mapped) => mapped,
                Err(error) => {
                    result = Err(error);
                    break;
                }
            };
        if let Some(found) = lookup(world, mapped) {
            teleports[index] = FormRef::Resolved(Some(found));
            previous = Some(index);
            index += 1;
            continue;
        }
        let text = format!(
            "Could not find RandomTeleport ({mapped:08X}) for Door '{}' ({:08X}).",
            name_of(world, form),
            form_id(world, form)
        );
        warn(world, services, text);
        if let Some(before) = previous {
            if let Err(error) = remove_after_previous(&mut teleports, before, mapped) {
                result = Err(error);
                break;
            }
            index = before + 1;
        } else {
            teleports.remove(index);
        }
    }
    world.get_mut::<TesDoor>(form).expect("a door").teleports_68 = teleports;
    result?;
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004129E0 vt+0x6C of INGR and ALCH: 0046F020 on +0x64; the flag stays clear. Completion: 100%.
pub fn init_magic_object(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_script(world, form, services)
}

/// IDA: 004B0C70 TESObjectLIGH vt+0x6C: 0046F020, then +0x8C maps and casts to TESSound, stored, a null warning a430d0. Completion: 100%.
pub fn init_light(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_script(world, form, services)?;
    let slot = world.get::<TesLight>(form).expect("a light").sound_8c;
    if !slot.is_empty() {
        let mapped = map_through_owner(world, raw_of(slot)?, form)?;
        let found = cast(world, lookup(world, mapped), RttiClass::TESSound);
        world.get_mut::<TesLight>(form).expect("a light").sound_8c = FormRef::Resolved(found);
        if found.is_none() {
            let text = format!(
                "Unable to find sound ({mapped:08X}) on object '{}'.",
                name_of(world, form)
            );
            warn(world, services, text);
        }
    }
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004BB120 TESObjectWEAP vt+0x6C: 0046F020, 0046A880. Completion: 100%.
pub fn init_weapon(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_script(world, form, services)?;
    init_enchantment(world, form, services)?;
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004A9000 TESAmmo vt+0x6C: 0046A880. Completion: 100%.
pub fn init_ammo(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_enchantment(world, form, services)?;
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004B02D0 vt+0x6C of TESLevItem and TESLevSpell: 0046D070. Completion: 100%.
pub fn init_leveled_form(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_leveled(world, form, services)?;
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 004AF950 TESLevCreature vt+0x6C: a nonzero +0x40 maps and casts to TESActorBase, stored, a null warning a42d40; 0046D070; 0046F020; then each entry whose form casts to TESActorBase with +0x28 bit 9 clear warns a42cc0. Completion: 100%.
pub fn init_leveled_creature(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    let slot = world
        .get::<TesLevCreature>(form)
        .expect("an LVLC")
        .template_40;
    if !slot.is_empty() {
        let mapped = map_through_owner(world, raw_of(slot)?, form)?;
        let found = cast(world, lookup(world, mapped), RttiClass::TESActorBase);
        world
            .get_mut::<TesLevCreature>(form)
            .expect("an LVLC")
            .template_40 = FormRef::Resolved(found);
        if found.is_none() {
            let text = format!(
                "Unable to find template actorbase ({mapped:08X}) on leveled creature {} ({:08X}).",
                name_of(world, form),
                form_id(world, form)
            );
            warn(world, services, text);
        }
    }
    init_leveled(world, form, services)?;
    init_script(world, form, services)?;
    let entries: Vec<FormRef> = world
        .get::<TesLeveledList>(form)
        .expect("an LVLC")
        .entries_04
        .iter()
        .map(|entry| entry.form_04)
        .collect();
    for entry in entries {
        let FormRef::Resolved(Some(actor)) = entry else {
            continue;
        };
        if !casts_to(world, actor, RttiClass::TESActorBase) {
            continue;
        }
        let flags = world
            .get::<TesActorBaseData>(actor)
            .expect("an actor base")
            .flags_04;
        if flags & 0x200 == 0 {
            let text = format!(
                "LeveledCreature '{}' ({:08X}) contains actors that process in low.\r\nRemove actor '{}' ({:08X}) or change it to not run in low.",
                name_of(world, form),
                form_id(world, form),
                name_of(world, actor),
                form_id(world, actor)
            );
            warn(world, services, text);
        }
    }
    set_initialized(world, form, true);
    Ok(())
}

// ---- actors ----

// IDA: a static GameSetting's raw word, as `fld d [address]` or `mov ecx,[address]` read it.
fn setting_raw(world: &World, address: u32) -> Result<u32, StreamError> {
    world
        .get_resource::<GameSettingCollection>()
        .and_then(|settings| settings.raw_at(address))
        .ok_or(StreamError(
            "InitItem: no static GameSetting at the address",
        ))
}

// IDA: N/A a helper, with no address of its own.
fn setting_float(world: &World, address: u32) -> Result<f32, StreamError> {
    setting_raw(world, address).map(f32::from_bits)
}

/// IDA: 004677F0 TESActorBaseData_GetEffectiveLevel: with +0x04 bit 7 clear, the level word. Otherwise the player's base's level word (vt+0x170's +0x24, with no cast) adds; a nonzero minimum above it answers the minimum; a nonzero maximum below it answers the maximum; below 1 answers 1. Completion: 100%.
pub fn effective_level(world: &World, data: &TesActorBaseData) -> Result<i16, StreamError> {
    if data.flags_04 & 0x80 == 0 {
        return Ok(data.level_0e as i16);
    }
    let base = player_base(world)?;
    let mut level = data.level_0e;
    if let Some(base) = base.and_then(|base| world.get::<TesActorBaseData>(base)) {
        level = level.wrapping_add(base.level_0e);
    }
    let level = level as i16;
    if data.min_10 != 0 && i32::from(level) < i32::from(data.min_10) {
        return Ok(data.min_10 as i16);
    }
    if data.max_12 != 0 && i32::from(level) > i32::from(data.max_12) {
        return Ok(data.max_12 as i16);
    }
    Ok(if level >= 1 { level } else { 1 })
}

// IDA: 00548020(a, endurance), the player's health: `fild` the second argument, `fmul` the float, 009828C0.
fn player_health(endurance: i32, multiplier: f32) -> i32 {
    x87::ftol(endurance as f32 * multiplier)
}

// IDA: 00547F80 the NPC health formula(strength, endurance, level - 1, endurance is primary, specialization), at 24 bits.
fn npc_health(
    world: &World,
    strength: i32,
    endurance: i32,
    level: i32,
    primary: bool,
    specialization: u32,
) -> Result<i32, StreamError> {
    let attribute = x87::ftol((strength + endurance) as f32 * setting_float(world, 0x00B3_7BE8)?);
    let mut per_level = setting_raw(world, 0x00B3_7BE0)? as i32;
    if primary {
        per_level = per_level.wrapping_add(1);
    }
    match specialization {
        0 => per_level = per_level.wrapping_add(1),
        1 => per_level = per_level.wrapping_sub(1),
        _ => {}
    }
    let total = attribute.wrapping_add(per_level.wrapping_mul(level));
    let threshold = setting_raw(world, 0x00B3_7BD8)? as i32;
    let mut scale = 1.0_f32;
    if level < threshold {
        // 00547FDE `fild`, `fidiv`, stored as a float; then 00547FF0..00548008: base + fraction * (1 - base), each step rounded.
        let fraction = (f64::from(level) / f64::from(threshold)) as f32;
        let base = setting_float(world, 0x00B3_7BD0)?;
        let part = fraction * (1.0 - base);
        scale = base + part;
    }
    Ok(x87::ftol(total as f32 * scale))
}

// IDA: `fistp` under the round-to-nearest control word of the main thread.
fn fistp(value: f32) -> i32 {
    x87::fistp(value)
}

// IDA: 005222D0 TESNPC_AutoCalcStats(0). The attributes, each from the race (+0x80 for a female, +0x74 otherwise), then, unless the player's class is the 00B37D00 setting's, the class's primaries add 00B362E4 or 00B362EC, and each skill of the attribute adds the level for a major skill and the level * 0.2 otherwise. Code 6 only for the player. 100 is the cap. The skills from the level, major or not, the specialization and the race's bonuses. Health, magicka and fatigue, then the class's services and trainer data. Completion: 100%.
fn auto_calc(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let data = world.get::<TesActorBaseData>(form).expect("an NPC").clone();
    let level_minus_one = i32::from(effective_level(world, &data)?) - 1;
    let (FormRef::Resolved(Some(race)), FormRef::Resolved(Some(class))) = (
        world.get::<TesRaceForm>(form).expect("an NPC").race_04,
        world.get::<TesNpc>(form).expect("an NPC").class_104,
    ) else {
        return Ok(());
    };
    let identity = *world.get::<FormIdentity>(form).expect("a form");
    let player = identity.form_id_0c == 7;
    let class_data = *world.get::<TesClass>(class).expect("a class");
    let use_class = !(player && setting_raw(world, 0x00B3_7D00)? == form_id(world, class));
    let race_body = world.get::<TesRace>(race).expect("a race").clone();
    let skills_by_index: Vec<(u32, u32)> = world
        .resource::<DataHandler>()
        .skills_d8
        .iter()
        .map(|&skill| {
            let skill = world.get::<TesSkill>(skill).expect("a skill");
            (skill.field_30, skill.specialization_34)
        })
        .collect();
    let level_float = level_minus_one as f32;
    let mut values = world.get::<TesAttributes>(form).expect("an NPC").values_04;
    for attribute in 0..8_u32 {
        if attribute == 6 && !player {
            continue;
        }
        let source = if data.flags_04 & 1 != 0 {
            race_body.attributes_80.values_04
        } else {
            race_body.attributes_74.values_04
        };
        let mut value = f32::from(source[attribute as usize]);
        if use_class {
            if attribute == class_data.attributes_38[0] {
                value += setting_float(world, 0x00B3_62E4)?;
            } else if attribute == class_data.attributes_38[1] {
                value += setting_float(world, 0x00B3_62EC)?;
            }
            for (skill, &(governing, _)) in skills_by_index.iter().enumerate() {
                if governing != attribute {
                    continue;
                }
                let code = skill as u32 + 12;
                // 00522401..00522438: a matching code's 00565C50 and 00446AF0 are this skill's +0x30 again, which equals the attribute here.
                let major = class_data.major_skills_44.contains(&code);
                let mut add = level_float;
                if !major {
                    // `fmul qword`: the double of 0.2f at A38538, rounded once.
                    add = (f64::from(add) * f64::from(f32::from_bits(0x3E4C_CCCD))) as f32;
                }
                value += add;
            }
        }
        if value > 100.0 {
            value = 100.0;
        }
        // 00468AF0: 00565C50(0, code) is the code.
        values[attribute as usize] = fistp(value) as u8;
        // 00468B26 vt+0x40(8).
        set_change_flags(world, form, 8)?;
    }
    world
        .get_mut::<TesAttributes>(form)
        .expect("an NPC")
        .values_04 = values;
    let mut skills = world.get::<TesNpc>(form).expect("an NPC").skills_ec;
    for skill in 0..21_u32 {
        let code = skill + 12;
        let major = use_class && class_data.major_skills_44.contains(&code);
        let mut value = if major {
            level_float + 25.0
        } else {
            // A2FC80 the double of 0.1f.
            let product = (f64::from(level_float) * f64::from(f32::from_bits(0x3DCC_CCCD))) as f32;
            product + 5.0
        };
        if use_class && skills_by_index[skill as usize].1 == class_data.specialization_40 {
            let bonus = value + 5.0;
            let half = level_float * 0.5;
            value = bonus + half;
        }
        for pair in 0..7 {
            let bonus_code = i32::from(race_body.data_50[2 * pair] as i8);
            if bonus_code == code as i32 {
                value += f32::from(race_body.data_50[2 * pair + 1] as i8);
            }
        }
        if value > 100.0 {
            value = 100.0;
        }
        skills[skill as usize] = fistp(value) as u8;
    }
    world.get_mut::<TesNpc>(form).expect("an NPC").skills_ec = skills;
    set_change_flags(world, form, 0x200)?;
    let primary = class_data.attributes_38[0] == 5 || class_data.attributes_38[1] == 5;
    let endurance = i32::from(values[5]);
    let strength = i32::from(values[0]);
    let health = if player {
        player_health(endurance, setting_float(world, 0x00B3_7720)?)
    } else {
        npc_health(
            world,
            strength,
            endurance,
            level_minus_one,
            primary,
            class_data.specialization_40,
        )?
    };
    // 00519C50: +0x84, 0046ABF0(4).
    world
        .get_mut::<TesHealthForm>(form)
        .expect("an NPC")
        .health_04 = health as u32;
    set_change_flags(world, form, 4)?;
    // 005482B0(intelligence, 0): `fild`, `fmul` the float, `fiadd`, 009828C0, at 24 bits.
    let intelligence = i32::from(values[1]);
    let magicka =
        x87::ftol(intelligence as f32 * setting_float(world, 0x00B3_7718)? + intelligence as f32);
    // 005479D0 the plain sum.
    let fatigue = i32::from(values[2]) + i32::from(values[3]) + endurance + strength;
    // 00467290 and 004672B0: each store then the component's vt+0x50(0x10), 004672F0 down to the form's vt+0x40. The earlier Rust made neither call (the C++ does), which didn't show while TLS +0x185 was clear; P14-RUN-001's 21C has this 0x10.
    world
        .get_mut::<TesActorBaseData>(form)
        .expect("an NPC")
        .magicka_08 = magicka as u16;
    set_change_flags(world, form, 0x10)?;
    world
        .get_mut::<TesActorBaseData>(form)
        .expect("an NPC")
        .fatigue_0a = fatigue as u16;
    set_change_flags(world, form, 0x10)?;
    let mut ai = world.get_mut::<TesAiForm>(form).expect("an NPC");
    ai.services_08 = class_data.services_64;
    // 004684C0: a code outside 12..=32 stores nothing.
    let trainer = u32::from(class_data.trainer_skill_68) + 12;
    if trainer.wrapping_sub(12) <= 0x14 {
        ai.trainer_skill_0c = (trainer - 12) as u8;
    }
    ai.trainer_level_0d = class_data.trainer_level_69;
    Ok(())
}

// IDA: 005239C0's walk of one spell list: each spell of type 4's "VAMP" effects add 00413320 (0 when the setting has bit 8, otherwise the magnitude), `fild`, `fadd`, stored as a float.
fn add_vampirism(world: &World, spells: &[FormRef], sum: &mut f32) -> Result<(), StreamError> {
    for slot in spells {
        let FormRef::Resolved(Some(spell)) = *slot else {
            continue;
        };
        let Some(data) = world.get::<MagicFormData>(spell) else {
            continue;
        };
        if data.type_34 != 4 {
            continue;
        }
        let effects = world.get::<EffectItemList>(spell).expect("a spell");
        for item in &effects.items_04 {
            if item.data_00[0] != VAMP {
                continue;
            }
            let flags = item
                .setting_1c
                .and_then(|setting| world.get::<EffectSetting>(setting))
                .ok_or(StreamError(
                    "00413323: an effect item with no setting faults",
                ))?
                .data_58
                .flags_58;
            let magnitude = if flags & 0x100 != 0 {
                0
            } else {
                item.data_00[1] as i32
            };
            *sum += magnitude as f32;
        }
    }
    Ok(())
}

// IDA: 005239C0 the actor value 0x45 of an NPC other than 7: the NPC's list, then its race's; 009828C0 of the sum; vt+0x134 00523310(0x45, it), past both its ranges: 00519F50 past its table: `fild`, 0065CB00(0x45, float) on +0xD0, 0046ABF0(0x10000000).
fn set_vampirism(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let mut sum = 0.0_f32;
    let spells = world
        .get::<TesSpellList>(form)
        .expect("an NPC")
        .spells_04
        .clone();
    add_vampirism(world, &spells, &mut sum)?;
    if let FormRef::Resolved(Some(race)) = world.get::<TesRaceForm>(form).expect("an NPC").race_04 {
        let spells = world
            .get::<TesSpellList>(race)
            .expect("a race")
            .spells_04
            .clone();
        add_vampirism(world, &spells, &mut sum)?;
    }
    let value = x87::ftol(sum);
    let mut values = world.get_mut::<ActorBaseValues>(form).expect("an NPC");
    actor_values_set_modifier(&mut values.0, 0x45, value as f32)?;
    set_change_flags(world, form, 0x1000_0000)
}

// IDA: the combat style both actor InitItems share: vt+0x120 reads the slot, a zero skips; it maps and casts to TESCombatStyle; a hit goes through vt+0x124 (the store, then 0046ABF0(0x400)); a null warns and leaves the slot.
fn init_combat_style(
    world: &mut World,
    form: Entity,
    slot: FormRef,
    what: &str,
    services: &mut DataLoadServices<'_>,
) -> Result<Option<FormRef>, StreamError> {
    if slot.is_empty() {
        return Ok(None);
    }
    let mapped = map_through_owner(world, raw_of(slot)?, form)?;
    let Some(found) = cast(world, lookup(world, mapped), RttiClass::TESCombatStyle) else {
        let text = format!(
            "Combat Style ({mapped:08X}) not found in InitItem for {what} ({:08X}) '{}'.",
            form_id(world, form),
            name_of(world, form)
        );
        warn(world, services, text);
        return Ok(None);
    };
    set_change_flags(world, form, 0x400)?;
    Ok(Some(FormRef::Resolved(Some(found))))
}

// IDA: 0046AB40(value): +0x08 bit 19.
fn set_no_health_flag(world: &mut World, form: Entity) {
    let health = world
        .get::<TesHealthForm>(form)
        .expect("an actor base")
        .health_04;
    let mut identity = world.get_mut::<FormIdentity>(form).expect("a form");
    if health == 0 {
        identity.flags_08 |= FLAG_NO_HEALTH;
    } else {
        identity.flags_08 &= !FLAG_NO_HEALTH;
    }
}

/// IDA: 00447300 DataHandler_RefreshMarkedNPCActorValues (WinMain 0040EBAA): +0x00's object list from 004B2460's first, each next through 004B2360 read after the call; `__RTDynamicCast` to TESNPC, and +0x28 bit 7 runs 005222D0(0). After 0040EB8B its vt+0x40 calls record, which is P14-RUN-001's +1,679 change entries. Answers the visited NPCs in order. Completion: 100%.
/// # Errors
/// A stop in 005222D0.
pub fn refresh_pc_level_npcs(world: &mut World) -> Result<Vec<Entity>, StreamError> {
    let mut visited = Vec::new();
    let mut object = world.resource::<DataHandlerLists>().objects_00.first_04;
    while let Some(form) = object {
        if casts_to(world, form, RttiClass::TESNPC)
            && world
                .get::<TesActorBaseData>(form)
                .expect("an NPC")
                .flags_04
                & 0x80
                != 0
        {
            auto_calc(world, form)?;
            visited.push(form);
        }
        object = world
            .get::<TesObjectLinks>(form)
            .expect("the list holds TESObjects")
            .next_20;
    }
    Ok(visited)
}

/// IDA: 00523F40 TESNPC vt+0x6C: 0046F020, 00469E90, 00468590, 0046E4F0, 004676B0, 0046FC10; a nonzero +0x104 maps and casts to TESClass, stored with no warning; vt+0xE4 00519D40 (+0x28 bit 4 of a type 0x23) runs 005222D0(0); the combat style; 0046AB40 by the health; for ID 7, 004672B0(0), 00519C50(0), 00467290(0), otherwise 005239C0. Completion: 100%.
pub fn init_npc(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_script(world, form, services)?;
    init_container(world, form, services)?;
    init_packages(world, form, services)?;
    init_race_form(world, form, services)?;
    init_actor_base_data(world, form, services)?;
    init_spells(world, form, services)?;
    let slot = world.get::<TesNpc>(form).expect("an NPC").class_104;
    if !slot.is_empty() {
        let mapped = map_through_owner(world, raw_of(slot)?, form)?;
        let found = cast(world, lookup(world, mapped), RttiClass::TESClass);
        world.get_mut::<TesNpc>(form).expect("an NPC").class_104 = FormRef::Resolved(found);
    }
    if form_type(world, form) == 0x23
        && world
            .get::<TesActorBaseData>(form)
            .expect("an NPC")
            .flags_04
            & 0x10
            != 0
    {
        auto_calc(world, form)?;
    }
    let style = world.get::<TesNpc>(form).expect("an NPC").combat_style_1e4;
    if let Some(style) = init_combat_style(world, form, style, "NPC", services)? {
        world
            .get_mut::<TesNpc>(form)
            .expect("an NPC")
            .combat_style_1e4 = style;
    }
    set_no_health_flag(world, form);
    if form_id(world, form) == 7 {
        world
            .get_mut::<TesActorBaseData>(form)
            .expect("an NPC")
            .fatigue_0a = 0;
        set_change_flags(world, form, 0x10)?;
        world
            .get_mut::<TesHealthForm>(form)
            .expect("an NPC")
            .health_04 = 0;
        set_change_flags(world, form, 4)?;
        world
            .get_mut::<TesActorBaseData>(form)
            .expect("an NPC")
            .magicka_08 = 0;
        set_change_flags(world, form, 0x10)?;
    } else {
        set_vampirism(world, form)?;
    }
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: 0051CF10 TESCreature vt+0x6C: 0046F020, 00469E90, 0046FC10, 00468590, 004676B0; +0x28 bit 0 clears 0x40070; with +0x28 bit 8 clear, a nonzero +0x100 maps and casts to TESCreature, stored, a null warning a52de0; the combat style; a +0x108 past 6 becomes 3; 0046AB40 by the health. Completion: 100%.
pub fn init_creature(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    init_script(world, form, services)?;
    init_container(world, form, services)?;
    init_spells(world, form, services)?;
    init_packages(world, form, services)?;
    init_actor_base_data(world, form, services)?;
    {
        let mut data = world.get_mut::<TesActorBaseData>(form).expect("a creature");
        if data.flags_04 & 1 != 0 {
            data.flags_04 &= 0xFFFB_FF8F;
        }
    }
    let flags = world
        .get::<TesActorBaseData>(form)
        .expect("a creature")
        .flags_04;
    if flags & 0x100 == 0 {
        let slot = world
            .get::<TesCreature>(form)
            .expect("a creature")
            .sounds_100
            .clone();
        match slot {
            CreatureSoundsSlot::Reference(slot) if !slot.is_empty() => {
                let mapped = map_through_owner(world, raw_of(slot)?, form)?;
                let found = cast(world, lookup(world, mapped), RttiClass::TESCreature);
                world
                    .get_mut::<TesCreature>(form)
                    .expect("a creature")
                    .sounds_100 = CreatureSoundsSlot::Reference(FormRef::Resolved(found));
                if found.is_none() {
                    let text = format!(
                        "Sound Creature ({mapped:08X}) not found in InitItem for Creature ({:08X}) '{}'.",
                        form_id(world, form),
                        name_of(world, form)
                    );
                    warn(world, services, text);
                }
            }
            CreatureSoundsSlot::Reference(_) => {}
            CreatureSoundsSlot::Sounds(_) => {
                return Err(StreamError(
                    "0051CF6D: a creature with a sound block and +0x28 bit 8 clear reads it as an ID",
                ));
            }
        }
    }
    let style = world
        .get::<TesCreature>(form)
        .expect("a creature")
        .combat_style_118;
    if let Some(style) = init_combat_style(world, form, style, "Creature", services)? {
        world
            .get_mut::<TesCreature>(form)
            .expect("a creature")
            .combat_style_118 = style;
    }
    {
        let mut creature = world.get_mut::<TesCreature>(form).expect("a creature");
        if creature.soul_108 > 6 {
            creature.soul_108 = 3;
        }
    }
    set_no_health_flag(world, form);
    set_initialized(world, form, true);
    Ok(())
}

/// IDA: vt+0x6C of any form: the class's slot. Completion: 100% of the ported classes.
pub fn init_item_of(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let class = form_class(u32::from(form_type(world, form))).ok_or(StreamError(
        "a vt+0x6C call on a form whose class isn't ported",
    ))?;
    (class.init_item)(world, form, services)
}
