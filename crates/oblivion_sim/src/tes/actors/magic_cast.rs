//! A caster's cast of an ability or a disease on itself: 0069BE00 / 0069BE40 into 0069AF30 MagicCaster_CastSpell's
//! self-range arm, 0068EA50's ActiveEffect factory (ValueModifierEffect, BoundItemEffect), and the target's add (005EE3A0,
//! 006A2CF0, 006A27F0) into its HighProcess's active-effect list.
//!
//! The actor holds its MagicCaster part at +0x5C and its MagicTarget part at +0x68; both reach the process at +0x58. The
//! process's list (vt+0x298 00629350, *(+0x174)) is `ActiveEffectList` on the process entity, beside the MiddleHigh part's
//! `effect_list_174` head block. Evidence: run-ah (the 9 casts), run-af's after_move (`after_move_effects.rows`).

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::actor_process::MiddleHighProcess;
use crate::tes::actors::bound_item::apply_bound_ability;
use crate::tes::forms_magic::{
    EffectItemList, EffectSetting, MagicFormData, effect_items_differ, effect_school,
};

/// IDA: 00A76D34 ValueModifierEffect, 00A75344 BoundItemEffect: the two classes 0068EA50 makes here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveEffectClass {
    ValueModifier,
    BoundItem,
}

/// IDA: ActiveEffect (0068D7A0, 0x38) and its two derived classes: ValueModifierEffect (006A82F0, 0x3C: +0x38 the actor value)
/// and BoundItemEffect (0068FC60, 0x8C, over AssociatedItemEffect 0068F2B0: +0x38 the associated form, +0x3C..+0x80 the
/// bound item's entries, +0x84..+0x88 bytes). +0x13 is left unset by the ctor (FormHeap bytes), so the port has no field.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ActiveEffect {
    pub class: ActiveEffectClass,
    /// +0x04 the elapsed time.
    pub elapsed_04: f32,
    /// +0x08 the magic item.
    pub item_08: Entity,
    /// +0x0C the effect item: its index in the item's `EffectItemList`.
    pub effect_0c: usize,
    /// +0x10, set after the first application in ActiveEffect_Update.
    pub applied_10: u8,
    /// +0x11, set on dispel, expiration or the target's death.
    pub terminated_11: u8,
    /// +0x12, set after the removal virtual.
    pub removed_12: u8,
    /// +0x14 flags: 0x14 from 0069AF30's last argument, 0x0E for an effect other than the item's highest-cost one.
    pub flags_14: u32,
    /// +0x18 the magnitude (`fild` of 00413320's), 1.0 for a no-magnitude ValueModifierEffect.
    pub magnitude_18: f32,
    /// +0x1C the duration (`fild` of 004133C0's).
    pub duration_1c: f32,
    /// +0x20 the target's MagicTarget part: its actor.
    pub target_20: Option<Entity>,
    /// +0x24 the caster's MagicCaster part: its actor.
    pub caster_24: Entity,
    /// +0x28 the item's type (the item's vt+0x18).
    pub item_type_28: u32,
    pub looping_sound_handle_2c: u32,
    /// +0x30 0068EA50's last argument.
    pub field_30: u32,
    pub hit_effects_34: u32,
    /// +0x38 a ValueModifierEffect's actor value, as a word; a BoundItemEffect's associated form is `associated_38`.
    pub actor_value_38: u32,
    pub associated_38: Option<Entity>,
    /// +0x3C and +0x40..+0x7C own inventory entries for displaced equipment.
    pub displaced_weapon_entry_3c: Option<Box<super::inventory::InventoryEntry>>,
    pub displaced_armor_entries_40: [Option<Box<super::inventory::InventoryEntry>>; 16],
    pub equip_delay_80: f32,
    pub equipped_84: u8,
    /// +0x85 is cleared by the constructor. Its later use is unresolved.
    pub field_85: u8,
    pub displaced_weapon_model_loaded_86: u8,
    pub displaced_armor_model_loaded_87: u8,
    pub pending_equip_lock_88: u8,
}

/// IDA: *(HighProcess +0x174), the BSSimpleList of ActiveEffect* that process vt+0x298 (00629350) answers, in its order.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct ActiveEffectList(pub Vec<Entity>);

/// IDA: the codes 0068DA10 MagicEffect_RegisterCreatorByCode registers into 00B15A0C from the static initializers at
/// 009FEABA..009FEDEA, as each push's code.
const REGISTERED_CODES: [u32; 23] = [
    0x4D4C_4143,
    0x4C4D_4843,
    0x5243_4F43,
    0x5548_4F43,
    0x4B52_4144,
    0x4F4D_4544,
    0x5443_5444,
    0x5241_4944,
    0x4557_4944,
    0x4C50_5344,
    0x5A4E_5246,
    0x4956_4E49,
    0x5448_474C,
    0x4B43_4F4C,
    0x4559_454E,
    0x4E45_504F,
    0x4152_4150,
    0x4E41_4552,
    0x5052_5453,
    0x4744_5553,
    0x454C_4554,
    0x4E52_5554,
    0x504D_4156,
];

/// IDA: 'SEFF', the script effect's code.
const CODE_SCRIPT_EFFECT: u32 = 0x4646_4553;

// IDA: N/A a spell's effect item, and its setting's flags.
pub(crate) fn effect_item(
    world: &World,
    item: Entity,
    index: usize,
) -> crate::tes::forms_magic::EffectItem {
    world
        .get::<EffectItemList>(item)
        .expect("a magic item")
        .items_04[index]
        .clone()
}

// IDA: N/A an effect item's setting's +0x58 flags; a null setting faults.
pub(crate) fn setting_flags(world: &World, setting: Option<Entity>) -> Result<u32, GameError> {
    let setting = setting.ok_or(GameError::unclassified(
        0x0068D7D5,
        "0068D7D5: a null effect setting faults",
    ))?;
    Ok(world
        .get::<EffectSetting>(setting)
        .expect("an MGEF")
        .data_58
        .flags_58)
}

// IDA: N/A a magic item's type, the item's vt+0x18 (+0x34 of a SpellItem's SPIT block).
fn item_type(world: &World, item: Entity) -> Result<u32, GameError> {
    world
        .get::<MagicFormData>(item)
        .map(|data| data.type_34)
        .ok_or(GameError::unclassified(
            0x0069BE0B,
            "0069BE0B: a magic item with no type block",
        ))
}

/// IDA: 0068D7A0 ActiveEffect_Constructor(caster, item, effect): +0x08 the item, +0x28 its type, +0x04 = 0.0, +0x0C the
/// effect, bytes +0x10..+0x12 = 0, +0x18 = `fild` 00413320 (0 for a no-magnitude setting, flag 0x100), +0x24 the caster,
/// +0x20 = 0, +0x1C = `fild` 004133C0 (0 for a no-duration setting, flag 0x80), +0x2C/+0x30/+0x34/+0x14 = 0. Completion: 100%.
fn construct_active_effect(
    world: &World,
    class: ActiveEffectClass,
    caster: Entity,
    item: Entity,
    index: usize,
) -> Result<ActiveEffect, GameError> {
    let effect = effect_item(world, item, index);
    let flags = setting_flags(world, effect.setting_1c)?;
    let magnitude = if flags & 0x100 != 0 {
        0
    } else {
        effect.data_00[1] as i32
    };
    let duration = if flags & 0x80 != 0 {
        0
    } else {
        effect.data_00[3] as i32
    };
    Ok(ActiveEffect {
        class,
        elapsed_04: 0.0,
        item_08: item,
        effect_0c: index,
        applied_10: 0,
        terminated_11: 0,
        removed_12: 0,
        flags_14: 0,
        magnitude_18: magnitude as f32,
        duration_1c: duration as f32,
        target_20: None,
        caster_24: caster,
        item_type_28: item_type(world, item)?,
        looping_sound_handle_2c: 0,
        field_30: 0,
        hit_effects_34: 0,
        actor_value_38: 0,
        associated_38: None,
        displaced_weapon_entry_3c: None,
        displaced_armor_entries_40: Default::default(),
        equip_delay_80: 0.0,
        equipped_84: 0,
        field_85: 0,
        displaced_weapon_model_loaded_86: 0,
        displaced_armor_model_loaded_87: 0,
        pending_equip_lock_88: 0,
    })
}

/// IDA: 0068EA50 ActiveEffect_Create(caster, item, effect, a4), the caster's vt+0x40 (00699090). 'SEFF' is a ScriptEffect
/// (006A4420), a stop; a code registered in 00B15A0C runs its creator, a stop; the code tree's own classes (0068CD90,
/// 00692940, 00692980, 006A4870) are stops. Otherwise by the setting's flags: 0x40000 a 0x64 class (006A51D0), a stop;
/// +0x5A bit 0 (0x10000) or 0x20000 a BoundItemEffect (0068FC60: 0068F2B0's +0x38 = 0046B250 of the setting's +0x60, the
/// entries and bytes zeroed); any of 0x1180000 a ValueModifierEffect (006A82F0: +0x38 = the setting's +0x60 when +0x5B bit
/// 0 (0x1000000), else the effect's +0x14; +0x18 = 1.0 with flag 0x100); none throws exUnregisteredActiveEffect, a stop.
/// Then +0x30 = a4. Completion: 100% of the two classes.
/// # Errors
/// The stops above.
pub fn create_active_effect(
    world: &World,
    caster: Entity,
    item: Entity,
    index: usize,
    a4: u32,
) -> Result<ActiveEffect, GameError> {
    let effect = effect_item(world, item, index);
    let setting = effect.setting_1c.ok_or(GameError::unclassified(
        0x0068EA5E,
        "0068EA5E: a null effect setting faults",
    ))?;
    let data = world.get::<EffectSetting>(setting).expect("an MGEF");
    let code = data.code_98;
    if code == CODE_SCRIPT_EFFECT {
        return Err(GameError::unclassified(
            0x0068EA6E,
            "0068EA6E: 006A4420, a ScriptEffect",
        ));
    }
    if REGISTERED_CODES.contains(&code) {
        return Err(GameError::unclassified(
            0x0068EAEF,
            "0068EAEF: a creator registered in 00B15A0C",
        ));
    }
    let signed = code as i32;
    let own_class = matches!(
        signed,
        1_347_633_729
            | 1_413_562_945
            | 1_263_747_649
            | 1_095_123_521
            | 1_162_363_457
            | 1_330_664_771
            | 1_229_215_043
            | 1_095_783_747
            | 1_213_420_102
            | 1_213_417_798
            | 1_213_417_804
            | 1_145_849_939
    );
    if own_class {
        return Err(GameError::unclassified(
            0x0068EB10,
            "0068EB10: one of 0068EA50's code-tree classes (0068CD90, 00692940, 00692980, 006A4870)",
        ));
    }
    let flags = data.data_58.flags_58;
    let associated = data.data_58.associated_60;
    let mut active = if flags & 0x40000 != 0 {
        return Err(GameError::unclassified(
            0x0068EC3D,
            "0068EC3D: 006A51D0, the 0x64 effect class",
        ));
    } else if flags & 0x10000 != 0 || flags & 0x20000 != 0 {
        let mut active =
            construct_active_effect(world, ActiveEffectClass::BoundItem, caster, item, index)?;
        // 0068F2C9 0046B250(setting +0x60).
        active.associated_38 = world
            .resource::<crate::tes::data_handler::FormsById>()
            .lookup(associated);
        active
    } else if flags & 0x0118_0000 == 0 {
        return Err(GameError::unclassified(
            0x0068EC9C,
            "0068EC9C: \"Could not Activate unregistered effect!\" (exUnregisteredActiveEffect)",
        ));
    } else {
        let mut active =
            construct_active_effect(world, ActiveEffectClass::ValueModifier, caster, item, index)?;
        active.actor_value_38 = if flags & 0x0100_0000 != 0 {
            associated
        } else {
            effect.data_00[5]
        };
        if flags & 0x100 != 0 {
            active.magnitude_18 = 1.0;
        }
        active
    };
    active.field_30 = a4;
    Ok(active)
}

// IDA: N/A the actor's process and its MiddleHigh part's list; none, or no head block, is vt+0x298's null.
fn effect_list_process(world: &World, actor: Entity) -> Option<Entity> {
    let process = crate::tes::actor::process_of(world, actor)?;
    world
        .get::<MiddleHighProcess>(process)
        .and_then(|middle| middle.effect_list_174)
        .map(|_| process)
}

/// IDA: 006A25E0, PointerList_InsertSorted's compare over two ActiveEffects: each one's `"%c%.30s%.1f%.1f"` of a school
/// letter (0 C, 1 E, 2 A, 3 D, 4 F, 5 B, else Z), 004139F0's name (the script's, else 004134C0's copy of the setting's),
/// the magnitude (1000.0 with flag 0x100) and the duration (1000.0 with flag 0x80), compared by `_mbsicmp`. Completion:
/// 100%, `_mbsicmp` as the "C" locale's `_stricmp`.
/// TODO: [ ] `_mbsicmp`'s case fold against `_stricmp`'s for bytes between 'Z' and 'a' (no name here has one).
fn sort_key(world: &World, active: &ActiveEffect) -> Result<Vec<u8>, GameError> {
    let effect = effect_item(world, active.item_08, active.effect_0c);
    let flags = setting_flags(world, effect.setting_1c)?;
    let duration = if flags & 0x80 != 0 {
        1000.0
    } else {
        active.duration_1c
    };
    let magnitude = if flags & 0x100 != 0 {
        1000.0
    } else {
        active.magnitude_18
    };
    let letter = match effect_school(world, &effect) {
        0 => b'C',
        1 => b'E',
        2 => b'A',
        3 => b'D',
        4 => b'F',
        5 => b'B',
        _ => b'Z',
    };
    let name = match &effect.script_18 {
        Some(script) => script.name_08.data_00.clone().unwrap_or_default(),
        None => crate::tes::forms_magic::setting_name_copy(
            world,
            effect.setting_1c.expect("checked above"),
        )
        .unwrap_or_default(),
    };
    let name: Vec<u8> = name
        .into_iter()
        .take_while(|&byte| byte != 0)
        .take(30)
        .collect();
    let mut key = vec![letter];
    key.extend_from_slice(&name);
    key.extend_from_slice(
        format!("{:.1}{:.1}", f64::from(magnitude), f64::from(duration)).as_bytes(),
    );
    Ok(key)
}

/// IDA: 006A27F0 MagicTarget_AddEffect(target, caster, item, effect) for an ability or a disease: 004FBF90's 0x2000 event on
/// the target's extra list; 'SEFF' isn't one here; an ability or a disease has magnitude 1.0 (no resistance, vt+0x08). The
/// player's god mode is for the player only. The effect's vt+0x34: a ValueModifierEffect needs a living target (006A81F0:
/// 006A1D70, vt+0x198(0)), a BoundItemEffect a Character (006A8AA0). 0068D9F0 (+0x30's type 20 or 22) is false here, so
/// 006A1F70(item, caster, effect) dispels an equal effect from the same item and caster (none here: a stop). The effect's
/// vt+4 copies it (006A8350 / 00690730 through vt+0x2C: 0068D8A0's fields and +0x38; a BoundItemEffect's entries, none
/// set); +0x20 = the target; 0068E170(1.0) does nothing; with no list (vt+0x08) the copy is deleted; else
/// PointerList_InsertSorted with 006A25E0, then vt+0x10 (005EE510). Answers whether it was added. Completion: 100% of
/// those arms; the resistance, the dispel and 005EE510's arms are stops.
fn magic_target_add_effect(
    world: &mut World,
    target: Entity,
    active: &ActiveEffect,
) -> Result<bool, GameError> {
    // 006A2828 004FBF90(setting, target's list, 0x2000): an empty event list.
    let setting = effect_item(world, active.item_08, active.effect_0c).setting_1c;
    crate::tes::ai_package::script_event_or(world, target, 0x2000, setting);
    if !matches!(active.item_type_28, 1 | 4) {
        return Err(GameError::unclassified(
            0x006A28A4,
            "006A28A4: a resistible item (the target's vt+0x08)",
        ));
    }
    let valid = match active.class {
        ActiveEffectClass::ValueModifier => !crate::tes::actor::actor_is_dead(world, target, false),
        ActiveEffectClass::BoundItem => world.get::<super::actor::Character>(target).is_some(),
    };
    if !valid {
        return Ok(false);
    }
    let Some(process) = effect_list_process(world, target) else {
        // 006A2ABE: no list, so the copy is deleted.
        return Ok(false);
    };
    // 006A2A6B 006A1F70(item, caster, effect).
    let list = world
        .get::<ActiveEffectList>(process)
        .map(|list| list.0.clone())
        .unwrap_or_default();
    let new_item = effect_item(world, active.item_08, active.effect_0c);
    for &entry in &list {
        let held = world
            .get::<ActiveEffect>(entry)
            .expect("an active effect")
            .clone();
        if held.terminated_11 == 0
            && held.item_08 == active.item_08
            && held.caster_24 == active.caster_24
            && held.field_30 == active.field_30
        {
            let held_item = effect_item(world, held.item_08, held.effect_0c);
            if !effect_items_differ(world, &held_item, &new_item)? {
                return Err(GameError::unclassified(
                    0x006A1FDE,
                    "006A1FDE: 0068EA10, dispelling an equal effect",
                ));
            }
        }
    }
    // 006A2A9C the copy, its target and 0068E170(1.0).
    let mut copy = active.clone();
    copy.target_20 = Some(target);
    let key = sort_key(world, &copy)?;
    let entity = world.spawn(copy).id();
    // 006A2AD6 PointerList_InsertSorted: before the first entry the compare answers <= 0 for.
    let mut at = list.len();
    for (index, &entry) in list.iter().enumerate() {
        let held = world
            .get::<ActiveEffect>(entry)
            .expect("an active effect")
            .clone();
        if oblivion_core::crt::stricmp(&key, &sort_key(world, &held)?) <= 0 {
            at = index;
            break;
        }
    }
    let mut list = list;
    list.insert(at, entity);
    world.entity_mut(process).insert(ActiveEffectList(list));
    target_effect_added(world, target, entity)
}

// IDA: 0041B950 EffectCode_GetShieldOrResistanceMask.
fn shield_or_resistance_mask(code: u32) -> u8 {
    match &code.to_le_bytes() {
        b"SHLD" | b"REDG" | b"RSNW" => 8,
        b"FISH" => 1,
        b"FRSH" => 2,
        b"LISH" => 4,
        _ => 0,
    }
}

// IDA: 006A86F0 ValueModifierEffect_ApplyEffect, 006A84D0 ValueModifierEffect_ModifyTargetActorValue,
// 005E2800 Actor_ModifyMaxActorValue, 00628C00 HighProcess_ModifyMaxActorValue and 00658850 MiddleLowProcess_ModifyMaxActorValue.
// Completion: a non-hostile, recoverable positive modifier on a non-player actor.
fn apply_recoverable_modifier(
    world: &mut World,
    entity: Entity,
    target: Entity,
    flags: u32,
) -> Result<(), GameError> {
    let effect = world.get::<ActiveEffect>(entity).expect("an effect");
    if flags & 4 != 0 || flags & 2 == 0 || effect.magnitude_18 <= 0.0 {
        return Err(GameError::stop(
            0x006A86F0,
            "detrimental, nonrecoverable or nonpositive actor-value effect",
        ));
    }
    let code = effect.actor_value_38;
    let delta = effect.magnitude_18;
    if let Some(process) = crate::tes::actor::process_of(world, target) {
        if let Some(mut middle) = world.get_mut::<super::actor_process::MiddleLowProcess>(process) {
            super::actor_process::actor_values_add_modifier(
                &mut middle.max_modifiers_94,
                code as u8,
                delta,
            )?;
        }
        if let Some(mut high) = world.get_mut::<super::actor_process::HighProcess>(process) {
            match code {
                11 => high.cached_encumbrance_294 = -1.0,
                48 => high.cached_paralysis_298 = -1,
                _ => {}
            }
        }
    }
    crate::tes::forms_items::set_change_flags(world, target, 0x100000)?;
    if matches!(code, 18 | 27) {
        return Err(GameError::stop(
            0x005E28CB,
            "refresh the target's equipment after an armor skill change",
        ));
    }
    Ok(())
}

// IDA: 0068E670 ActiveEffect_Update.
// Completion: the unexpired, non-player ability path for recoverable modifiers and the zero-step guard.
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "the x87 compare: a NaN takes the negated arm"
)]
fn update_effect(world: &mut World, entity: Entity, step: f32) -> Result<(), GameError> {
    let effect = world.get::<ActiveEffect>(entity).expect("an effect");
    if !(step > 0.0) || effect.terminated_11 != 0 {
        if effect.applied_10 != 0 && effect.terminated_11 != 0 && effect.removed_12 == 0 {
            return Err(GameError::stop(0x0068E9A8, "remove a terminated effect"));
        }
        return Ok(());
    }
    let target = effect.target_20.expect("a listed effect's target");
    if Some(target) == crate::tes::player::the_player(world) || effect.item_type_28 != 4 {
        return Err(GameError::stop(0x0068E670, "player or timed spell effect"));
    }
    let item = effect_item(world, effect.item_08, effect.effect_0c);
    let flags = setting_flags(world, item.setting_1c)?;
    if effect.applied_10 == 0 {
        // 0068E210 skips a type-4 item's hit sound even when the MGEF has a sound.
        let effect_flags = effect.flags_14;
        let magnitude = effect.magnitude_18;
        match effect.class {
            ActiveEffectClass::ValueModifier => {
                apply_recoverable_modifier(world, entity, target, flags)?
            }
            ActiveEffectClass::BoundItem => apply_bound_ability(world, entity)?,
        }
        if effect_flags & 1 == 0
            && flags & 0x08000000 == 0
            && shield_or_resistance_mask(item.data_00[0]) != 0
            && magnitude < 100.0
        {
            return Err(GameError::stop(
                0x0068E739,
                "attach a shield ability's hit effects",
            ));
        }
        let mut effect = world.get_mut::<ActiveEffect>(entity).unwrap();
        effect.applied_10 = 1;
        effect.elapsed_04 = 0.0;
    }
    let mut effect = world.get_mut::<ActiveEffect>(entity).unwrap();
    if effect.removed_12 == 0 {
        effect.elapsed_04 += step;
        if effect.class == ActiveEffectClass::BoundItem {
            // 006918D0 BoundItemEffect_UpdateEffect.
            if effect.equipped_84 != 0 {
                super::bound_item::update_equipped_bound_item(world, entity)?;
            } else if effect.equip_delay_80 <= effect.elapsed_04 {
                super::bound_item::equip_associated_item(world, entity)?;
                world.get_mut::<ActiveEffect>(entity).unwrap().equipped_84 = 1;
            }
            effect = world.get_mut::<ActiveEffect>(entity).unwrap();
        }
        if effect.class == ActiveEffectClass::ValueModifier && flags & 2 == 0 {
            return Err(GameError::stop(
                0x006A8800,
                "update a nonrecoverable modifier over time",
            ));
        }
        if crate::tes::actor::actor_is_dead(world, target, true) && flags & 0x10000000 == 0 {
            world.get_mut::<ActiveEffect>(entity).unwrap().terminated_11 = 1;
            return Err(GameError::stop(
                0x0068E9A8,
                "remove the dead target's effect",
            ));
        }
    }
    Ok(())
}

/// IDA: 006A2280 MagicTarget_UpdateEffects. Walk the process list at its caller, in stored order.
/// Completion: an actor with a loaded node and cell, including the script-only filter when processing is disabled.
/// # Errors
/// An active effect whose update or removal still needs a port.
pub fn update_target_effects(world: &mut World, actor: Entity, step: f32) -> Result<(), GameError> {
    let Some(process) = effect_list_process(world, actor) else {
        return Ok(());
    };
    if crate::tes::forms_init_world::reference_ni_node(world, actor).is_none()
        || !crate::tes::forms_references::parent_cell(world, actor)
            .and_then(|cell| world.get::<crate::tes::forms_world::TesObjectCell>(cell))
            .is_some_and(|cell| cell.scene_node_present_54 != 0)
    {
        return Ok(());
    }
    let script_only = world
        .get::<super::actor::Actor>(actor)
        .unwrap()
        .processing_enabled_78
        == 0
        && !crate::tes::forms_references::save_load_active(world)?;
    let mut index = 0;
    while let Some(&entity) = world
        .get::<ActiveEffectList>(process)
        .and_then(|effects| effects.0.get(index))
    {
        let effect = world.get::<ActiveEffect>(entity).expect("a listed effect");
        if !script_only
            || effect_item(world, effect.item_08, effect.effect_0c)
                .setting_1c
                .and_then(|e| world.get::<EffectSetting>(e))
                .is_some_and(|s| s.code_98 == CODE_SCRIPT_EFFECT)
        {
            update_effect(world, entity, step)?;
            if world.get::<ActiveEffect>(entity).unwrap().terminated_11 != 0 {
                return Err(GameError::stop(
                    0x006A2368,
                    "unlink the finished effect and notify its target",
                ));
            }
        }
        index += 1;
    }
    Ok(())
}

/// IDA: 005EE510, the actor's MagicTarget vt+0x10 after an add: with a process, 0041B950 and 0041B9A0 of the setting's
/// code (each nonzero runs the process's vt+0x44C/0x450), then for a hostile effect not on an ability, 005E70F0. Completion:
/// 100% of codes both answer 0 for and a non-hostile effect; the rest are stops.
fn target_effect_added(world: &World, target: Entity, entity: Entity) -> Result<bool, GameError> {
    let active = world.get::<ActiveEffect>(entity).expect("just added");
    let effect = effect_item(world, active.item_08, active.effect_0c);
    let code = effect.data_00[0] as i32;
    let first = shield_or_resistance_mask(code as u32) != 0;
    let second = matches!(code, 1_195_657_299 | 1_195_657_542 | 1_195_659_846);
    if first || second {
        return Err(GameError::unclassified(
            0x005EE53A,
            "005EE53A: the process's vt+0x44C/0x450 for 0041B950/0041B9A0's code",
        ));
    }
    let hostile = match &effect.script_18 {
        Some(script) => script.hostile_14 != 0,
        None => setting_flags(world, effect.setting_1c)? & 1 != 0,
    };
    if hostile && active.item_type_28 != 4 {
        return Err(GameError::unclassified(
            0x005EE5DC,
            "005EE5DC: 005E70F0 for a hostile effect",
        ));
    }
    let _ = target;
    Ok(true)
}

/// IDA: 005EE3A0, the MagicTarget's vt+0 (006992A0 reaches it from the caster's vt+0x28 with its own target part): the
/// extra list's presence bit 0x25 with an item that isn't an ability answers 0; 006A2CF0 (a self-range effect, or 'SEFF'
/// with the item's vt+0x1C, or the item's vt+0x20, goes to 006A27F0; other ranges are stops); 004149F0's arm needs an
/// item other than a disease or an ability. Completion: 100% of an ability or a disease on its caster.
fn magic_target_apply(
    world: &mut World,
    target: Entity,
    active: &ActiveEffect,
) -> Result<bool, GameError> {
    let present = world
        .get::<super::actor::TesObjectRefr>(target)
        .expect("an actor")
        .extra_data_44
        .present(0x25);
    if present && active.item_type_28 != 4 {
        return Ok(false);
    }
    let range = effect_item(world, active.item_08, active.effect_0c).data_00[4];
    if range != 0 {
        return Err(GameError::unclassified(
            0x006A2D2C,
            "006A2D2C: 006A2CF0 for a touch or target range",
        ));
    }
    let added = magic_target_add_effect(world, target, active)?;
    if !matches!(active.item_type_28, 1 | 4) {
        return Err(GameError::unclassified(
            0x005EE3E9,
            "005EE3E9: 004149F0's arm",
        ));
    }
    Ok(added)
}

/// IDA: 0069BE00 (vt+0x2A0 0064AE50, an ability, type 4) and 0069BE40 (vt+0x2A4 0064AE70, a disease, type 1, with a
/// target): the caster's vt+0x34 (005E5420: the process's vt+0x2AC, +0x144) on the item, then 0069AF30(0.0, null, a3).
/// 0069AF30's self arm: no item returns; the caster's +0x08 = 4 (a byte of the actor's MagicCaster part the port doesn't
/// hold); the caster is the actor; type 7 (an ingredient) isn't one; no touch effect, so no target; vt+0x24's magic node
/// needs 3D (a stop with a node); 004152C0's highest-cost self effect; with spell flag 0x80 (+0x40) 0069AC50, a stop. Each
/// effect in order: vt+0x40 (0068EA50(caster, item, effect, null)); a3 sets flags 0x14; an ability or a disease sets no
/// magnitude or duration; a self effect other than the highest-cost one gets 0x0E, then vt+0x28 (006992A0 -> the actor's
/// MagicTarget vt+0 005EE3A0); the original is deleted. Then no area or art arm, the skill-use arm is for types 0 and 5,
/// and vt+0x34(0), vt+0x3C(0) (+0x178) and +0x08 = 0. Completion: 100% of a self-range ability or disease on an actor
/// with no 3D.
/// # Errors
/// The stops above, or one in the effects' creation and add.
pub fn cast_on_self(
    world: &mut World,
    actor: Entity,
    item: Entity,
    a3: bool,
) -> Result<(), GameError> {
    let item_type = item_type(world, item)?;
    if !matches!(item_type, 1 | 4) {
        return Ok(());
    }
    let process = crate::tes::actor::process_of(world, actor).ok_or(GameError::unclassified(
        0x005E51BB,
        "005E51BB: a caster with no process",
    ))?;
    // 0069BE2D vt+0x34 -> 00629370: +0x144 = the item.
    if world.get::<MiddleHighProcess>(process).is_none() {
        return Err(GameError::unclassified(
            0x0064AE50,
            "0064AE50: a caster with no MiddleHigh process",
        ));
    }
    let id = world
        .get::<crate::tes::form::FormIdentity>(item)
        .expect("a form")
        .form_id_0c;
    world
        .get_mut::<MiddleHighProcess>(process)
        .expect("checked above")
        .current_magic_item_144 = id;
    if world
        .get::<super::actor::TesObjectRefr>(actor)
        .expect("an actor")
        .loaded_3d_3c
        != 0
    {
        return Err(GameError::unclassified(
            0x0069AFE9,
            "0069AFE9: vt+0x24's magic node with 3D",
        ));
    }
    if world
        .get::<MagicFormData>(item)
        .is_some_and(|data| data.flags_40 & 0x80 != 0)
    {
        return Err(GameError::unclassified(
            0x0069B149,
            "0069B149: 0069AC50 for spell flag 0x80",
        ));
    }
    let highest = crate::tes::forms_magic::highest_cost_effect_in_range(world, item, 0)?;
    let count = world
        .get::<EffectItemList>(item)
        .expect("a magic item")
        .items_04
        .len();
    for index in 0..count {
        let mut active = create_active_effect(world, actor, item, index, 0)?;
        if a3 {
            active.flags_14 |= 0x14;
        }
        let range = effect_item(world, item, index).data_00[4];
        if range != 0 {
            return Err(GameError::unclassified(
                0x0069B3AE,
                "0069B3AE: a touch or target effect in 0069AF30",
            ));
        }
        if Some(index) != highest {
            active.flags_14 |= 0x0E;
        }
        // 0069B3E7 vt+0x28 -> 006992A0 -> 005EE3A0; its answer is v98, used only for types 0 and 5.
        magic_target_apply(world, actor, &active)?;
    }
    // 0069B8D0.. vt+0x34(0), vt+0x3C(0) -> 00629390 (+0x178), +0x08 = 0.
    let mut middle = world
        .get_mut::<MiddleHighProcess>(process)
        .expect("checked above");
    middle.current_magic_item_144 = 0;
    middle.field_178 = 0;
    Ok(())
}
