//! Phase 12b: the magic forms. EffectSetting (MGEF 0xA8) with the pre-WinMain EffectSettingCollection and 0044DCF0's MGEF arm; the effect items of EnchantmentItem (ENCH 0x44) and SpellItem (SPEL 0x44); BirthSign (BSGN 0x4C) and TESLevSpell (LVSP 0x34). IngredientItem, AlchemyItem and TESSigilStone are in `forms_items`, on `EffectItemList` and `load_efid_chunk`. Evidence: objdump of every address named here, evidence/phase12-window-2026-10-09/fam3-magic*.txt.
//!
//! InitItem is 12d: MGEF 00415960, BSGN 00519500 and LVSP 004B02D0 are `no_init_item` until then. ENCH's and SPEL's vt+0x6C is 0060D0A0.

use bevy_ecs::component::Mutable;
use bevy_ecs::prelude::*;
use oblivion_core::crt;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::effect_settings_table::BUILT_IN_EFFECTS;
use super::form::{FormIdentity, OriginalVtable};
use super::form_arms::{
    FormClass, FormLoadCx, default_editor_id, editor_id_of, no_init_item, reset_14_components,
    reset_18_components,
};
use super::form_components::{
    ComponentSlot, FormRef, TesFullName, TesLeveledList, TesModel, TesObjectLinks, TesSpellList,
    load_description, load_full_name, load_model, load_texture,
};
use super::forms_misc::{clear_altered, set_initialized};
use super::forms_script::latin1;
use crate::bs::string::BsStringT;
use crate::tes::data_handler::{
    DataHandlerGlobals, DataHandlerLists, FormsById, TesDescription, TesTexture,
};
use crate::tes::data_load::{
    DataLoadServices, construct_effect_setting, effect_setting_arm,
    initialize_form_from_current_record,
};
use crate::tes::form::{construct_tes_form, set_form_id_release};
use crate::tes::form_components::reset_description;
use crate::tes::forms_init::init_item_of;
use crate::tes::magic_cast::setting_flags;
use crate::tes::magic_selection::{
    EFFECT_FLAGS_SUMMON, MagicItemModelRequest, art_path, art_state, associated_counter,
    associated_settings, base_effect_cost, caster_skill_cost, effect_flags,
};
use crate::tes::player::player_get_actor_value;
use crate::tes::tes::{GameThreadLocals, TesGlobals};
use crate::tes::tes_file::map_file_form_id;

/// IDA: the form types of this family.
pub const FORM_TYPE_EFFECT_SETTING: u8 = 0x0C;
pub const FORM_TYPE_ENCHANTMENT: u8 = 0x0F;
pub const FORM_TYPE_SPELL: u8 = 0x10;
pub const FORM_TYPE_BIRTH_SIGN: u8 = 0x11;
pub const FORM_TYPE_LEV_SPELL: u8 = 0x40;

/// IDA: 00416430 and the stores after it: the form, TESModel +0x18, TESDescription +0x30, TESFullName +0x38, TESIcon +0x44.
pub const EFFECT_SETTING_VTABLES: [u32; 5] = [
    0x00A3_2B14,
    0x00A3_2AF0,
    0x00A3_2AD8,
    0x00A3_2AC4,
    0x00A3_2AA8,
];
/// IDA: 00419570: the form, TESFullName +0x18, EffectItemList +0x24.
pub const ENCHANTMENT_VTABLES: [u32; 3] = [0x00A3_3C84, 0x00A3_3C3C, 0x00A3_3C28];
/// IDA: 0041D420: the form, TESFullName +0x18, EffectItemList +0x24.
pub const SPELL_VTABLES: [u32; 3] = [0x00A3_4C3C, 0x00A3_4BF4, 0x00A3_4BE4];
/// IDA: 005195F0: the form, TESFullName +0x18, TESTexture +0x24, TESDescription +0x30, TESSpellList +0x38.
pub const BIRTH_SIGN_VTABLES: [u32; 5] = [
    0x00A5_2524,
    0x00A5_250C,
    0x00A5_24F0,
    0x00A5_24D8,
    0x00A5_24B8,
];
/// IDA: 004B0400: the form, TESLeveledList +0x24.
pub const LEV_SPELL_VTABLES: [u32; 2] = [0x00A4_2F1C, 0x00A4_2F04];
/// IDA: 00414A00 stores this before the class's own.
pub const EFFECT_ITEM_LIST_VTABLE: u32 = 0x00A3_282C;
/// IDA: 'SEFF', the scripted effect's code.
pub const SCRIPT_EFFECT_CODE: u32 = 0x4646_4553;

// IDA: the chunks of 00416100, 0041B2D0, 00413130, 005193E0 and 004B04A0.
pub(crate) const EDID: u32 = 0x4449_4445;
const FULL: u32 = 0x4C4C_5546;
const DESC: u32 = 0x4353_4544;
const ICON: u32 = 0x4E4F_4349;
const DATA: u32 = 0x4154_4144;
const ESCE: u32 = 0x4543_5345;
const EFID: u32 = 0x4449_4645;
const EFIT: u32 = 0x5449_4645;
const SCIT: u32 = 0x5449_4353;
const ENIT: u32 = 0x5449_4E45;
const SPIT: u32 = 0x5449_5053;
const SPLO: u32 = 0x4F4C_5053;
const LVLD: u32 = 0x444C_564C;
const LVLF: u32 = 0x464C_564C;
const LVLO: u32 = 0x4F4C_564C;
const MODB: u32 = 0x4244_4F4D;
const MODL: u32 = 0x4C44_4F4D;
const MODT: u32 = 0x5444_4F4D;

// IDA: 00B03418..00B034BF, the 0x15 (00B034C0) {mask, from the record} pairs of 00416100; a pair with 0 keeps the setting's bit. Bytes read from the exe.
const MERGED_FLAGS: [(u32, bool); 0x15] = [
    (0x0000_0001, false),
    (0x0000_0002, false),
    (0x0000_0004, false),
    (0x0000_0008, false),
    (0x0000_0010, false),
    (0x0000_0020, false),
    (0x0000_0040, false),
    (0x0000_0080, false),
    (0x0000_0100, false),
    (0x0000_0200, false),
    (0x0000_0400, true),
    (0x0000_0800, true),
    (0x0000_1000, true),
    (0x0000_2000, true),
    (0x0000_8000, false),
    (0x0001_0000, false),
    (0x0002_0000, false),
    (0x0004_0000, false),
    (0x0008_0000, false),
    (0x0010_0000, false),
    (0x0800_0000, true),
];
// IDA: 00B034C4..00B034DB, the 6 (00B034DC) masks 00416100 takes from the record. Bytes read from the exe.
const RECORD_FLAGS: [u32; 6] = [
    0x0020_0000,
    0x0040_0000,
    0x0080_0000,
    0x0100_0000,
    0x0200_0000,
    0x0400_0000,
];

/// IDA: one 00417220 call's arguments: (code, name, school, cost, associated, flags, resist, count, codes...), as `tools/effect_settings_table/generate.py` read them. `call` is the address of the call.
#[derive(Clone, Copy, Debug)]
pub struct BuiltInEffect {
    pub call: u32,
    pub code: u32,
    pub name: &'static [u8],
    pub school: u32,
    pub cost_bits: u32,
    pub associated: u32,
    pub flags: u32,
    pub resist: u32,
    pub counters: &'static [u32],
}

/// IDA: EffectSetting's DATA block, +0x58..+0x97, which 00416100 clears and reads whole. +0x70 and +0x78..+0x8C are raw form IDs until 00415960 (12d). The fields keep the bytes a record gave them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EffectSettingData {
    pub flags_58: u32,
    pub base_cost_5c: f32,
    pub associated_60: u32,
    pub school_64: u32,
    pub resist_68: u32,
    pub counter_count_6c: u16,
    pub unset_6e: u16,
    pub light_70: FormRef,
    pub projectile_speed_74: f32,
    pub shaders_and_sounds_78: [FormRef; 6],
    pub enchant_factor_90: f32,
    pub barter_factor_94: f32,
}

impl EffectSettingData {
    // IDA: N/A a raw slot's word.
    fn raw(slot: FormRef) -> u32 {
        match slot {
            FormRef::Raw(id) => id,
            FormRef::Resolved(_) => 0,
        }
    }

    /// IDA: N/A the 0x40 bytes at +0x58.
    #[must_use]
    pub fn bytes(&self) -> [u8; 0x40] {
        let mut bytes = [0_u8; 0x40];
        let mut put =
            |at: usize, value: u32| bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        put(0x00, self.flags_58);
        put(0x04, self.base_cost_5c.to_bits());
        put(0x08, self.associated_60);
        put(0x0C, self.school_64);
        put(0x10, self.resist_68);
        put(
            0x14,
            u32::from(self.counter_count_6c) | (u32::from(self.unset_6e) << 16),
        );
        put(0x18, Self::raw(self.light_70));
        put(0x1C, self.projectile_speed_74.to_bits());
        for (index, slot) in self.shaders_and_sounds_78.iter().enumerate() {
            put(0x20 + 4 * index, Self::raw(*slot));
        }
        put(0x38, self.enchant_factor_90.to_bits());
        put(0x3C, self.barter_factor_94.to_bits());
        bytes
    }

    /// IDA: N/A the same bytes back into the fields.
    pub fn set_bytes(&mut self, bytes: &[u8; 0x40]) {
        let word =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
        self.flags_58 = word(0x00);
        self.base_cost_5c = f32::from_bits(word(0x04));
        self.associated_60 = word(0x08);
        self.school_64 = word(0x0C);
        self.resist_68 = word(0x10);
        self.counter_count_6c = word(0x14) as u16;
        self.unset_6e = (word(0x14) >> 16) as u16;
        self.light_70 = FormRef::Raw(word(0x18));
        self.projectile_speed_74 = f32::from_bits(word(0x1C));
        for (index, slot) in self.shaders_and_sounds_78.iter_mut().enumerate() {
            *slot = FormRef::Raw(word(0x20 + 4 * index));
        }
        self.enchant_factor_90 = f32::from_bits(word(0x38));
        self.barter_factor_94 = f32::from_bits(word(0x3C));
    }
}

/// IDA: 00416430 EffectSetting (0xA8): `TesModel` +0x18, `TesDescription` +0x30, `TesFullName` +0x38, `TesTexture` +0x44 (TESIcon) on the same entity; +0x50/+0x54; the DATA block +0x58; +0x98 the code; +0x9C the counter-effect codes (FormHeap, ESCE; `Some(empty)` is a pointer 00416286 freed and left, with a count of 0); +0xA0/+0xA4.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct EffectSetting {
    pub field_50: u32,
    pub field_54: u32,
    pub data_58: EffectSettingData,
    pub code_98: u32,
    pub counters_9c: Option<Vec<u32>>,
    /// +0xA0 the art model's counter: below 0 queued (00415EB0 EffectSetting_MarkArtQueued, one step per queue), above 0 loaded (00415EE0 EffectSetting_MarkArtLoaded's absolute value), each further use one step from 0 (004157A0).
    pub art_counter_a0: u32,
    /// +0xA4 the associated item's counter (a bound weapon or armor, or a summon), the same way: 00415E50 queues, 00415740 steps, 00415780 / 00415790 test loaded / queued.
    pub associated_counter_a4: u32,
}

/// IDA: 00418DB0 EffectSettingCollection at 00B33508: a NiTMap<EffectID, EffectSetting*> with +0x04 = 0x25 buckets (vt+4 006A9060: key % size), each a chain with the newest at the head, and the settings it built, in call order. Pre-WinMain, cinit slot 151: built by the host when the loader starts, a recorded timing substitution, since nothing reads it earlier.
#[derive(Resource, Clone, Debug)]
pub struct EffectSettingCollection {
    pub buckets_08: Vec<Vec<(u32, Entity)>>,
    pub count_0c: u32,
    pub built_in: Vec<Entity>,
}

impl Default for EffectSettingCollection {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            buckets_08: vec![Vec::new(); 0x25],
            count_0c: 0,
            built_in: Vec::new(),
        }
    }
}

impl EffectSettingCollection {
    /// IDA: 00416870 -> 0055E000 the lookup (null when absent). Completion: 100%.
    #[must_use]
    pub fn lookup(&self, code: u32) -> Option<Entity> {
        self.buckets_08[(code % 0x25) as usize]
            .iter()
            .find(|(key, _)| *key == code)
            .map(|(_, form)| *form)
    }

    /// IDA: 00452570 NiTMapUInt32UInt8_SetAt: a key in the chain takes the new value; a new node goes to the chain's head and +0x0C counts. Completion: 100%.
    pub fn set_at(&mut self, code: u32, form: Entity) {
        let bucket = &mut self.buckets_08[(code % 0x25) as usize];
        if let Some(entry) = bucket.iter_mut().find(|(key, _)| *key == code) {
            entry.1 = form;
            return;
        }
        bucket.insert(0, (code, form));
        self.count_0c += 1;
    }
}

// IDA: N/A a form's component, out of the world for a call that needs the world too (a text warning, a form lookup), and back on every exit.
pub(in crate::tes) fn with_component<T, R>(
    world: &mut World,
    form: Entity,
    call: impl FnOnce(&mut World, &mut T) -> R,
) -> R
where
    T: Component<Mutability = Mutable> + Default,
{
    let mut value = std::mem::take(
        &mut *world
            .get_mut::<T>(form)
            .expect("a class's component is there"),
    );
    let result = call(world, &mut value);
    *world
        .get_mut::<T>(form)
        .expect("a class's component is there") = value;
    result
}

// IDA: g_saveLoadGame +0x18. The original dereferences 00B33B00 with no test; it is set by this point (P11-RUN-001), and a world without one reads as clear.
pub(in crate::tes) fn save_load_flags(world: &World) -> u32 {
    world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .map_or(0, |game| game.fields_14[1])
}

// IDA: the "set when different" test of 00416580 and 00413960: _stricmp of two non-null strings, otherwise 2 * (new == null) - 1, which is nonzero. Completion: 100%.
fn set_if_different(text: &mut BsStringT, name: Option<&[u8]>) {
    let differs = match (text.data_00.as_deref(), name) {
        (Some(current), Some(name)) => crt::stricmp(current, name) != 0,
        _ => true,
    };
    if differs {
        text.set(name, 0);
    }
}

// IDA: 00416430 EffectSetting_Constructor: 0046C460; 0046D7E0 on +0x18; 0046A400 on +0x30; +0x38 TESFullName inline; 0046FFD0 on +0x44, then its vtable A320A4; +0x5C = 0.0; +0x74 = 1.0; the five vtables; +0x98 = -1; +0x58 = +0x60 = 0; +0x64 = 6; +0x68 = 0x48; +0x6C = 0; +0x70 and +0x78..+0x8C = 0; +0x90 and +0x94 from the GameSetting floats 00B33704 and 00B3370C (`fld`/`fstp`); +0x9C = +0xA0 = +0xA4 = +0x50 = +0x54 = 0; +0x04 = 0x0C. Completion: 100% of the store order.
pub(crate) fn construct_effect_setting_with(
    world: &mut World,
    factor_90_bits: u32,
    factor_94_bits: u32,
) -> Result<Entity, GameError> {
    let form = construct_tes_form(world)?;
    let mut description = TesDescription {
        original_vtable: EFFECT_SETTING_VTABLES[2],
        offset_04: 0,
    };
    reset_description(
        &mut description,
        &mut world.resource_mut::<DataHandlerGlobals>(),
    );
    world.entity_mut(form).insert((
        OriginalVtable(EFFECT_SETTING_VTABLES[0]),
        TesModel {
            original_vtable: EFFECT_SETTING_VTABLES[1],
            ..TesModel::default()
        },
        description,
        TesFullName {
            original_vtable: EFFECT_SETTING_VTABLES[3],
            name_04: BsStringT::default(),
        },
        TesTexture {
            original_vtable: EFFECT_SETTING_VTABLES[4],
            path_04: BsStringT::default(),
        },
        EffectSetting {
            code_98: 0xFFFF_FFFF,
            data_58: EffectSettingData {
                school_64: 6,
                resist_68: 0x48,
                projectile_speed_74: 1.0,
                enchant_factor_90: x87::fld_fstp_copy(factor_90_bits),
                barter_factor_94: x87::fld_fstp_copy(factor_94_bits),
                ..EffectSettingData::default()
            },
            ..EffectSetting::default()
        },
    ));
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_EFFECT_SETTING;
    Ok(form)
}

// IDA: 00416580 EffectSetting_SetName(name) (into the TESFullName +0x3C). 004134C0's copy of the current name is what the test reads; it is freed either way. Completion: 100%.
fn set_effect_name(name: &mut TesFullName, text: &[u8]) {
    let mut copy = BsStringT::default();
    copy.set(name.name_04.data_00.as_deref(), 0);
    if match copy.data_00.as_deref() {
        Some(current) => crt::stricmp(current, text) != 0,
        None => true,
    } {
        name.name_04.set(Some(text), 0);
    }
}

// IDA: 00415D40 EffectSetting_SetCounters(count, codes): a nonzero +0x9C is freed and cleared; `test bx, bx; jbe` so only 0 stops, the count being unsigned; FormHeap(4 * count); +0x6C = count; the codes. Completion: 100%.
fn set_counters(setting: &mut EffectSetting, codes: &[u32]) {
    setting.counters_9c = None;
    if codes.is_empty() {
        return;
    }
    setting.counters_9c = Some(codes.to_vec());
    setting.data_58.counter_count_6c = codes.len() as u16;
}

/// IDA: 00418DB0 EffectSettingCollection_Constructor (0x25 buckets) -> 00417420's 161 calls of 00417220 RegisterEffect, in order: FormHeap(0xA8) 00416430; +0x58 flags; the "does not allow any range" throw, which no row takes; the ASSOCIATED test at 0041732A, which can't fire, comparing one bit with > 1; +0x98; 00416580; +0x5C through `fld`/`fstp`; +0x64, +0x60, +0x68; 00415D40 for a nonzero count; 00452570. Before WinMain 0046C569 sees a null DataHandler; here the TLS byte holds that arm shut for the call. The two GameSetting floats are zero, before cinit 201 and 202. Completion: 100% of the table and store order.
pub fn construct_effect_setting_collection(world: &mut World) -> Result<(), GameError> {
    let saved = world.resource::<GameThreadLocals>().byte_184;
    world.resource_mut::<GameThreadLocals>().byte_184 = 1;
    let mut collection = EffectSettingCollection::default();
    for effect in &BUILT_IN_EFFECTS {
        let form = construct_effect_setting_with(world, 0, 0)?;
        let mut entity = world.entity_mut(form);
        let mut setting = entity.get_mut::<EffectSetting>().expect("just built");
        setting.data_58.flags_58 = effect.flags;
        // 0041727F..004172A8.
        if effect.code != SCRIPT_EFFECT_CODE
            && effect.flags & 0x80_0000 == 0
            && effect.flags & 0x70 == 0
        {
            return Err(GameError::unclassified(
                0x004172AA,
                "004172AA: Registered EffectSetting does not allow any range!",
            ));
        }
        setting.code_98 = effect.code;
        set_effect_name(
            &mut entity.get_mut::<TesFullName>().expect("just built"),
            effect.name,
        );
        let mut setting = entity.get_mut::<EffectSetting>().expect("just built");
        setting.data_58.base_cost_5c = x87::fld_fstp_copy(effect.cost_bits);
        setting.data_58.associated_60 = effect.associated;
        setting.data_58.school_64 = effect.school;
        setting.data_58.resist_68 = effect.resist;
        set_counters(&mut setting, effect.counters);
        collection.set_at(effect.code, form);
        collection.built_in.push(form);
    }
    world.insert_resource(collection);
    world.resource_mut::<GameThreadLocals>().byte_184 = saved;
    Ok(())
}

// IDA: the lookup of 00416870 on a world that must hold 00B33508.
fn find_setting(world: &World, code: u32) -> Result<Option<Entity>, GameError> {
    Ok(world
        .get_resource::<EffectSettingCollection>()
        .ok_or(GameError::unclassified(
            0x00416870,
            "00416870: the EffectSettingCollection is required (it is built before WinMain)",
        ))?
        .lookup(code))
}

// IDA: 00416100 EffectSetting vt+0x1C LoadForm. The record must be MGEF (else 0); 00451210; the chunks to 0 or a failed advance. DATA: +0x60, +0x58 and +0x58's bit 24 are kept; 0x40 bytes cleared; +0x74 = 1.0; 00450C20(+0x58, 0x40); the masks; a kept bit 24 is put back with the old +0x60; +0x58 bit 21 cleared. ESCE: a nonzero +0x9C is freed (and left); a nonzero count: `movsx`, FormHeap(4 * count), 00450C20(+0x9C, size). EDID (alloca, size 0x200, vt+0xD8 006FDFF0 keeps nothing). DESC, FULL, ICON, and MODB/MODL/MODT to 0046DB00. Returns 1. Completion: 100%.
fn load_effect_setting_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? != u32::from(FORM_TYPE_EFFECT_SETTING) {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            DATA => {
                let mut setting = world.get_mut::<EffectSetting>(form).expect("an MGEF");
                let old_flags = setting.data_58.flags_58;
                let old_associated = setting.data_58.associated_60;
                let old_bit_24 = old_flags & 0x100_0000 != 0;
                let mut data = EffectSettingData {
                    projectile_speed_74: 1.0,
                    ..EffectSettingData::default()
                };
                let mut bytes = data.bytes();
                cx.file.read_chunk(&mut bytes, 0x40)?;
                data.set_bytes(&bytes);
                let record = data.flags_58;
                let mut merged = old_flags;
                for (mask, from_record) in MERGED_FLAGS {
                    if from_record {
                        if record & mask != 0 {
                            merged |= mask;
                        } else {
                            merged &= !mask;
                        }
                    }
                }
                for mask in RECORD_FLAGS {
                    if record & mask != 0 {
                        merged |= mask;
                    } else {
                        merged &= !mask;
                    }
                }
                data.flags_58 = merged;
                if old_bit_24 {
                    data.flags_58 |= 0x100_0000;
                    data.associated_60 = old_associated;
                }
                data.flags_58 &= !0x20_0000;
                setting.data_58 = data;
            }
            ESCE => {
                let mut setting = world.get_mut::<EffectSetting>(form).expect("an MGEF");
                if let Some(counters) = setting.counters_9c.as_mut() {
                    counters.clear();
                }
                let count = setting.data_58.counter_count_6c;
                if count != 0 {
                    // 004162A8 `movsx`: a count past 0x7FFF is a negative size to FormHeap. The C++ extends it unsigned.
                    if (count as i16) < 0 {
                        return Err(GameError::unclassified(
                            0x004162A8,
                            "004162A8: a counter count past 0x7FFF is a negative FormHeap size",
                        ));
                    }
                    let size = u32::from(count) * 4;
                    let mut bytes = vec![0_u8; size as usize];
                    cx.file.read_chunk(&mut bytes, size)?;
                    setting.counters_9c = Some(
                        bytes
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .map(|word| u32::from_le_bytes(*word))
                            .collect(),
                    );
                }
            }
            DESC => load_description(
                &mut world
                    .get_mut::<TesDescription>(form)
                    .expect("the class's component"),
                cx,
            )?,
            EDID => {
                cx.file.read_text(0x200)?;
            }
            FULL => load_full_name(
                &mut world
                    .get_mut::<TesFullName>(form)
                    .expect("the class's component")
                    .name_04,
                cx,
            )?,
            ICON => load_texture(
                &mut world
                    .get_mut::<TesTexture>(form)
                    .expect("the class's component"),
                cx,
            )?,
            MODB | MODL | MODT => {
                with_component(world, form, |world, model: &mut TesModel| {
                    load_model(world, model, Some(form), cx)
                })?;
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    Ok(true)
}

// IDA: 004168A0 EffectSetting_FindByName(text): under 4 characters null; the key is the first four, each through `movsx`, ORed in, so a byte past 0x7F sets every bit above it (the C++ copies the four bytes); 0055E000. Completion: 100%.
pub(crate) fn find_setting_by_name(
    world: &World,
    text: &[u8],
) -> Result<Option<Entity>, GameError> {
    if text.len() < 4 {
        return Ok(None);
    }
    find_setting(world, effect_code_of_name(text))
}

// IDA: 004168C5..004168E5 the key.
fn effect_code_of_name(text: &[u8]) -> u32 {
    let byte = |at: usize| i32::from(text[at] as i8) as u32;
    (((((byte(3) << 8) | byte(2)) << 8) | byte(1)) << 8) | byte(0)
}

/// IDA: ScriptEffectInfo (0x18): +0x00 the script (a mapped form ID), +0x04 school, +0x08 BSStringT name, +0x10 the visual effect code, +0x14 u8 hostile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptEffectInfo {
    pub script_00: u32,
    pub school_04: u32,
    pub name_08: BsStringT,
    pub visual_10: u32,
    pub hostile_14: u8,
}

/// IDA: EffectItem (0x24): +0x00 code, +0x04 magnitude, +0x08 area, +0x0C duration, +0x10 range, +0x14 actor value, +0x18 ScriptEffectInfo*, +0x1C EffectSetting*, +0x20 float cost.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectItem {
    pub data_00: [u32; 6],
    pub script_18: Option<Box<ScriptEffectInfo>>,
    pub setting_1c: Option<Entity>,
    pub cost_20: f32,
}

/// IDA: 00414A00 EffectItemList (0x10): the class's vtable, +0x04/+0x08 the BSSimpleList<EffectItem*>, +0x0C the hostile count, which 00414C70 doesn't clear.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct EffectItemList {
    pub original_vtable: u32,
    pub items_04: Vec<EffectItem>,
    pub hostile_count_0c: u32,
}

impl EffectItemList {
    /// IDA: 00414C70 EffectItemList_Free: 00413BB0 and FormHeap_Free of each item, and the nodes. Completion: 100%.
    pub fn free(&mut self) {
        self.items_04.clear();
    }
}

// IDA: the copy 004134C0 -> 00413490 makes of a setting's +0x3C: null when the name is null or empty.
pub(crate) fn setting_name_copy(world: &World, setting: Entity) -> Option<Vec<u8>> {
    let mut copy = BsStringT::default();
    copy.set(
        world
            .get::<TesFullName>(setting)
            .expect("an MGEF")
            .name_04
            .data_00
            .as_deref(),
        0,
    );
    copy.data_00
}

/// IDA: 00414790 EffectItem_Constructor(setting): +0x1C; +0x00 = +0x98; +0x08, +0x0C, +0x04 = 0; +0x14 = a setting flag in 0x1180000 ? its +0x60 : 0x48; +0x20 = -1.0. For 'SEFF': FormHeap(0x18) with the name cleared, the 004134C0 copy through 00413960, +0x04 = 4, +0x00 = 0, +0x10 = 0, byte +0x14 = 0; otherwise +0x18 = 0. +0x10 is the first of bits 4, 5, 6 of +0x58, or a throw. Completion: 100%.
pub fn construct_effect_item(world: &World, setting: Entity) -> Result<EffectItem, GameError> {
    let source = world.get::<EffectSetting>(setting).expect("an MGEF");
    let flags = source.data_58.flags_58;
    let mut item = EffectItem {
        data_00: [source.code_98, 0, 0, 0, 0, 0],
        script_18: None,
        setting_1c: Some(setting),
        cost_20: -1.0,
    };
    item.data_00[5] = if flags & 0x118_0000 != 0 {
        source.data_58.associated_60
    } else {
        0x48
    };
    if source.code_98 == SCRIPT_EFFECT_CODE {
        let mut script = Box::<ScriptEffectInfo>::default();
        let name = setting_name_copy(world, setting);
        set_if_different(&mut script.name_08, name.as_deref());
        script.school_04 = 4;
        item.script_18 = Some(script);
    }
    item.data_00[4] = if flags & 0x10 != 0 {
        0
    } else if flags & 0x20 != 0 {
        1
    } else if flags & 0x40 != 0 {
        2
    } else {
        return Err(GameError::unclassified(
            0x004148BD,
            "004148BD: EffectID does not allow any Range setting!",
        ));
    };
    Ok(item)
}

// IDA: 00413130 EffectItem_Load(file, spell name). The next chunk must be EFIT (else 0): six dwords cleared, 00450C20(0x18); +0x1C from the new code; an unknown code warns "Unknown EffectSetting '%d' ..." and returns 1. A setting flag in 0x1180000 with bit 24 takes its +0x60. Past 'SEFF', 1. The next chunk must be SCIT (else 0): a null +0x18 is built; 0x10 bytes to zeroed locals: +0x04 the second, +0x00 the first, +0x10 the third, byte +0x14 the fourth's low byte; 0046BB20 on +0x00; a +0x10 naming no setting clears. The next chunk, if FULL, sets the name; any other stays current, and the caller's advance passes over it. Returns 1. Completion: 100%, with FormHeap failing not modelled.
fn load_effect_item(
    world: &World,
    item: &mut EffectItem,
    spell: Option<&[u8]>,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    cx.file.advance_chunk()?;
    if cx.file.chunk_type()? != EFIT {
        return Ok(false);
    }
    item.data_00 = [0; 6];
    let mut bytes = [0_u8; 0x18];
    cx.file.read_chunk(&mut bytes, 0x18)?;
    for (word, chunk) in item.data_00.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *word = u32::from_le_bytes(*chunk);
    }
    item.setting_1c = find_setting(world, item.data_00[0])?;
    let Some(setting) = item.setting_1c else {
        let name = spell.map_or_else(|| "{unknown}".to_owned(), latin1);
        cx.file.services.warn(format!(
            "Unknown EffectSetting '{}' encountered when loading EffectItem in spell '{name}'",
            item.data_00[0] as i32
        ));
        return Ok(true);
    };
    let source = world.get::<EffectSetting>(setting).expect("an MGEF");
    let flags = source.data_58.flags_58;
    if flags & 0x118_0000 != 0 && flags & 0x100_0000 != 0 {
        item.data_00[5] = source.data_58.associated_60;
    }
    if source.code_98 != SCRIPT_EFFECT_CODE {
        return Ok(true);
    }
    cx.file.advance_chunk()?;
    if cx.file.chunk_type()? != SCIT {
        return Ok(false);
    }
    let mut locals = [0_u8; 0x10];
    cx.file.read_chunk(&mut locals, 0x10)?;
    let word = |at: usize| u32::from_le_bytes(locals[at..at + 4].try_into().expect("four bytes"));
    let id = map_file_form_id(world, cx, word(0), Some(cx.file.id))?;
    let visual = word(8);
    let visual = if visual != 0 && find_setting(world, visual)?.is_none() {
        0
    } else {
        visual
    };
    {
        let script = item.script_18.get_or_insert_with(Box::default);
        script.school_04 = word(4);
        script.script_00 = id;
        script.visual_10 = visual;
        script.hostile_14 = locals[0x0C];
    }
    cx.file.advance_chunk()?;
    if cx.file.chunk_type()? == FULL {
        let text = cx.file.read_text(0)?;
        item.script_18
            .as_mut()
            .expect("set above")
            .name_08
            .set(Some(&text), 0);
    }
    Ok(true)
}

// IDA: 00413470 EffectItem_IsHostile: +0x18's byte +0x14, or bit 0 of the setting's byte +0x58. A null setting faults. Completion: 100%.
fn effect_item_hostile(world: &World, item: &EffectItem) -> Result<bool, GameError> {
    if let Some(script) = &item.script_18 {
        return Ok(script.hostile_14 != 0);
    }
    let setting = item.setting_1c.ok_or(GameError::unclassified(
        0x00413470,
        "00413470: an EffectItem without a setting faults",
    ))?;
    Ok(world
        .get::<EffectSetting>(setting)
        .expect("an MGEF")
        .data_58
        .flags_58
        & 1
        != 0)
}

/// IDA: 004154B0 EffectItemList_LoadEffect(file, spell name), the EFID chunk of the list on `owner`. The code (004510E0, -1 for an empty chunk) must name a setting, or the warnings: each has a TESForm cast, which succeeds for every class holding this list, so "(%08X)" is the arm that runs. FormHeap(0x24) 00414790 from that setting; 00413130; on success 0067B1E0 and +0x0C for a hostile item; on failure the warning, with the EFID's setting's name, then 00413BB0 and the free. A null copy of the name prints "(null)", as vsprintf does. Completion: 100%, with FormHeap failing not modelled.
pub fn load_efid_chunk(
    world: &mut World,
    owner: Entity,
    spell: Option<&[u8]>,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let (code, _) = cx.file.read_dword(0xFFFF_FFFF)?;
    let Some(setting) = find_setting(world, code)? else {
        let name = latin1(&editor_id_of(world, owner));
        let id = world.get::<FormIdentity>(owner).expect("a form").form_id_0c;
        cx.file.services.warn(format!(
            "Unable to find EffectSetting {} in spell '{name}' ({id:08X}).",
            code as i32
        ));
        return Ok(());
    };
    let mut item = construct_effect_item(world, setting)?;
    if load_effect_item(world, &mut item, spell, cx)? {
        let hostile = effect_item_hostile(world, &item)?;
        let mut list = world
            .get_mut::<EffectItemList>(owner)
            .expect("the class's component");
        list.items_04.push(item);
        if hostile {
            list.hostile_count_0c += 1;
        }
        return Ok(());
    }
    let name =
        setting_name_copy(world, setting).map_or_else(|| "(null)".to_owned(), |text| latin1(&text));
    let editor = latin1(&editor_id_of(world, owner));
    let id = world.get::<FormIdentity>(owner).expect("a form").form_id_0c;
    cx.file.services.warn(format!(
        "Unable to load EffectItem '{name}' in spell '{editor}' ({id:08X})."
    ));
    Ok(())
}

// IDA: 0041B340..0041B38A: vt+0xD4 when non-null and non-empty, else the TESFullName's +0x04 or `Default`.
fn spell_name(world: &World, form: Entity) -> Vec<u8> {
    let editor = editor_id_of(world, form);
    if !editor.is_empty() {
        return editor;
    }
    world
        .get::<TesFullName>(form)
        .expect("the class's component")
        .name_04
        .data_00
        .clone()
        .unwrap_or_default()
}

/// IDA: 0041B2D0 MagicItem LoadForm, with no record test: 00451210; each chunk: EDID (alloca, size 0x200, vt+0xD8 006FDFF0 keeps nothing), EFID 004154B0, FULL 0046C7A0; then, for every chunk, vt+0x3C (`on_chunk`, 0060CF60 `ret 8` for ENCH and SPEL), and the chunk's type, read at the top of the pass, against vt+0x24 (`data_type`): `read_data` does the memset of vt+0x2C at vt+0x28 and the read. A form with no effect warns "Magic Item (%08X) %s has no effects defined.". Returns 1. Completion: 100%.
pub fn load_magic_item(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
    data_type: u32,
    on_chunk: &mut dyn FnMut(&mut World, &mut FormLoadCx<'_, '_>, u32) -> Result<(), GameError>,
    read_data: &mut dyn FnMut(&mut World, &mut FormLoadCx<'_, '_>) -> Result<(), GameError>,
) -> Result<bool, GameError> {
    initialize_form_from_current_record(world, form, cx)?;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            EDID => {
                cx.file.read_text(0x200)?;
            }
            EFID => {
                let name = spell_name(world, form);
                load_efid_chunk(world, form, Some(&name), cx)?;
            }
            FULL => load_full_name(
                &mut world
                    .get_mut::<TesFullName>(form)
                    .expect("the class's component")
                    .name_04,
                cx,
            )?,
            _ => {}
        }
        on_chunk(world, cx, chunk)?;
        if chunk == data_type {
            read_data(world, cx)?;
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    if world
        .get::<EffectItemList>(form)
        .expect("the class's component")
        .items_04
        .is_empty()
    {
        let identity = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
        let name = world
            .get::<TesFullName>(form)
            .expect("the class's component")
            .name_04
            .data_00
            .clone()
            .unwrap_or_default();
        cx.file.services.warn(format!(
            "Magic Item ({identity:08X}) {} has no effects defined.",
            latin1(&name)
        ));
    }
    Ok(true)
}

/// IDA: the ENIT / SPIT block (0x10 at +0x34) of EnchantmentItem and SpellItem: the vt+0x28 of their +0x18 part, 00419000, is `lea eax, [ecx+0x1C]`, and vt+0x2C, 008C4830, is 0x10.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MagicFormData {
    pub type_34: u32,
    pub value_38: u32,
    pub value_3c: u32,
    pub flags_40: u8,
    pub unset_41: [u8; 3],
}

impl MagicFormData {
    /// IDA: N/A the 0x10 bytes at +0x34.
    #[must_use]
    pub fn bytes(&self) -> [u8; 0x10] {
        let mut bytes = [0_u8; 0x10];
        bytes[..4].copy_from_slice(&self.type_34.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.value_38.to_le_bytes());
        bytes[8..0x0C].copy_from_slice(&self.value_3c.to_le_bytes());
        bytes[0x0C] = self.flags_40;
        bytes[0x0D..].copy_from_slice(&self.unset_41);
        bytes
    }

    /// IDA: N/A the same bytes back into the fields.
    pub fn set_bytes(&mut self, bytes: &[u8; 0x10]) {
        let word =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
        self.type_34 = word(0);
        self.value_38 = word(4);
        self.value_3c = word(8);
        self.flags_40 = bytes[0x0C];
        self.unset_41.copy_from_slice(&bytes[0x0D..]);
    }
}

// IDA: 0041B3DA..0041B40E for ENCH and SPEL: memset 0x10, then 00450C20(+0x34, 0x10).
fn read_magic_form_data(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let mut bytes = [0_u8; 0x10];
    world
        .get_mut::<MagicFormData>(form)
        .expect("the class's component")
        .set_bytes(&bytes);
    cx.file.read_chunk(&mut bytes, 0x10)?;
    world
        .get_mut::<MagicFormData>(form)
        .expect("the class's component")
        .set_bytes(&bytes);
    Ok(())
}

// IDA: 00419730 ENIT, 0041D250 SPIT: the vt+0x24 of the +0x18 part.
fn load_magic_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    let data_type = if world
        .get::<FormIdentity>(form)
        .expect("a form")
        .form_type_04
        == FORM_TYPE_SPELL
    {
        SPIT
    } else {
        ENIT
    };
    load_magic_item(
        world,
        form,
        cx,
        data_type,
        &mut |_, _, _| Ok(()),
        &mut |world, cx| read_magic_form_data(world, form, cx),
    )
}

// IDA: 00418EF0 EnchantmentItem vt+0x14: +0x3C = +0x38 = -1, byte +0x40 = 0, +0x34 = 2, then 0046C2E0. Completion: 100%.
fn reset_enchantment(world: &mut World, form: Entity) -> Result<(), GameError> {
    let mut data = world
        .get_mut::<MagicFormData>(form)
        .expect("the class's component");
    data.value_3c = 0xFFFF_FFFF;
    data.value_38 = 0xFFFF_FFFF;
    data.flags_40 = 0;
    data.type_34 = 2;
    reset_14_components(world, form)
}

// IDA: 0041D1B0 SpellItem vt+0x14: +0x34 = 0, +0x38 = -1, +0x3C = 0, byte +0x40 = 0, then 0046C2E0. Completion: 100%.
fn reset_spell(world: &mut World, form: Entity) -> Result<(), GameError> {
    let mut data = world
        .get_mut::<MagicFormData>(form)
        .expect("the class's component");
    data.type_34 = 0;
    data.value_38 = 0xFFFF_FFFF;
    data.value_3c = 0;
    data.flags_40 = 0;
    reset_14_components(world, form)
}

// IDA: 00418F10 vt+0x18 of both: 00414C70 on +0x24, then 0046C2F0. Completion: 100%.
fn destroy_magic_form(world: &mut World, form: Entity) -> Result<(), GameError> {
    world
        .get_mut::<EffectItemList>(form)
        .expect("the class's component")
        .free();
    reset_18_components(world, form)
}

// IDA: 004194E0 under the class's constructor: 0046C460; 00412AE0 on +0x18: TESFullName inline and 00414A00 on +0x24; the three vtables.
fn construct_magic_form_base(world: &mut World, vtables: [u32; 3]) -> Result<Entity, GameError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        OriginalVtable(vtables[0]),
        TesFullName {
            original_vtable: vtables[1],
            name_04: BsStringT::default(),
        },
        EffectItemList {
            original_vtable: vtables[2],
            ..EffectItemList::default()
        },
        MagicFormData::default(),
    ));
    Ok(form)
}

// IDA: 00419570 EnchantmentItem_Constructor: 004194E0; byte +0x40 = 0; the vtables; +0x04 = 0x0F; +0x3C = +0x38 = -1; +0x34 = 2; 0046C2E0. Completion: 100% of the store order.
fn construct_enchantment(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_magic_form_base(world, ENCHANTMENT_VTABLES)?;
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_ENCHANTMENT;
    reset_enchantment(world, form)?;
    Ok(form)
}

// IDA: 0041D420 SpellItem_Constructor: 004194E0; the vtables; +0x04 = 0x10; +0x34 = 0; +0x38 = -1; +0x3C = 0; byte +0x40 = 0; 0046C2E0. Completion: 100% of the store order.
fn construct_spell(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_magic_form_base(world, SPELL_VTABLES)?;
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_SPELL;
    reset_spell(world, form)?;
    Ok(form)
}

// IDA: 00413410 EffectItem_SetRange(0), 004133E0 SetDuration, 00413340 SetMagnitude and 00413390 SetArea, as 0041D4A0 and 0041D590 call them. Each is refused by its setting flag (range: allowed by bit 4; the rest: refused by bits 7, 8, 9) and a negative value; the area also by a range of 0; each store sets the cost to -1.0. Completion: 100% of the arguments reached.
fn set_default_effect(item: &mut EffectItem, flags: u32, duration: i32, magnitude: i32, area: i32) {
    if flags & 0x10 != 0 {
        item.data_00[4] = 0;
        item.cost_20 = -1.0;
    }
    if flags & 0x80 == 0 && duration >= 0 {
        item.data_00[3] = duration as u32;
        item.cost_20 = -1.0;
    }
    if flags & 0x100 == 0 && magnitude >= 0 {
        item.data_00[1] = magnitude as u32;
        item.cost_20 = -1.0;
    }
    if flags & 0x200 == 0 && item.data_00[4] != 0 && area >= 0 {
        item.data_00[2] = area as u32;
        item.cost_20 = -1.0;
    }
}

// IDA: 0041D4A0 DefaultPlayerSpell ('REHE', magnitude 5, "Default Player Spell") and 0041D590 DefaultMarksmanParalyzeSpell ('PARA', duration 10, "Master Marksman Paralysis", +0x38 = 0, and vt+0x14(0) of the +0x18 part, 00418FE0: byte +0x40 |= 1): FormHeap(0x44) 0041D420; FormHeap(0x24) 00414790 from 00416870, faulting on a null; the setters; 00414B90 the append and hostile count; 004028D0(+0x1C); +0x34 = 0. Completion: 100%.
fn construct_default_spell(world: &mut World, marksman: bool) -> Result<Entity, GameError> {
    let form = construct_spell(world)?;
    let code = if marksman { 0x4152_4150 } else { 0x4548_4552 };
    let setting = find_setting(world, code)?.ok_or(GameError::unclassified(
        0x00414790,
        "00414790 on a null EffectSetting faults (0041D51E, 0041D60E)",
    ))?;
    let mut item = construct_effect_item(world, setting)?;
    let flags = world
        .get::<EffectSetting>(setting)
        .expect("an MGEF")
        .data_58
        .flags_58;
    if marksman {
        set_default_effect(&mut item, flags, 10, 0, 0);
    } else {
        set_default_effect(&mut item, flags, 0, 5, 0);
    }
    let hostile = effect_item_hostile(world, &item)?;
    {
        let mut list = world.get_mut::<EffectItemList>(form).expect("just built");
        list.items_04.push(item);
        if hostile {
            list.hostile_count_0c += 1;
        }
    }
    let name: &[u8] = if marksman {
        b"Master Marksman Paralysis"
    } else {
        b"Default Player Spell"
    };
    world
        .get_mut::<TesFullName>(form)
        .expect("just built")
        .name_04
        .set(Some(name), 0);
    let mut data = world.get_mut::<MagicFormData>(form).expect("just built");
    data.type_34 = 0;
    if marksman {
        data.value_38 = 0;
        data.flags_40 |= 1;
    }
    Ok(form)
}

/// IDA: 0041C77A..0041C884, in 0041BB80 under 0044AC20: for 0x136 and 0x137: g_formsByID through the SpellItem cast; else 0041D4A0 or 0041D590, 0046C300(ID, 1), vt+0xD8 006FDFF0, which keeps nothing, vt+0x90(0), and the +0x2C prepend. 00B335A8 and 00B335AC are returned to the caller. Completion: 100%, called by a test until 12d has 0044AC20.
pub fn create_default_spells(world: &mut World) -> Result<[Entity; 2], GameError> {
    let mut spells = [Entity::PLACEHOLDER; 2];
    for (slot, (id, marksman)) in spells.iter_mut().zip([(0x136, false), (0x137, true)]) {
        let found = world.resource::<FormsById>().lookup(id).filter(|&form| {
            world
                .get::<FormIdentity>(form)
                .expect("a form")
                .form_type_04
                == FORM_TYPE_SPELL
        });
        *slot = match found {
            Some(form) => form,
            None => {
                let form = construct_default_spell(world, marksman)?;
                set_form_id_release(world, form, id, true)?;
                clear_altered(world, form)?;
                world
                    .resource_mut::<DataHandlerLists>()
                    .spells_2c
                    .prepend_non_null(form);
                form
            }
        };
    }
    Ok(spells)
}

// IDA: 005193E0 BirthSign vt+0x1C LoadForm, with no record test: 00451210; a pass per chunk until 0044FEA0 fails, with no test for chunk 0: EDID (alloca, size 0x200, vt+0xD8 keeps nothing), DESC, FULL, ICON, and SPLO (a zeroed local, 004510E0, 0046F420). 0046AB80(0). Returns 1. Completion: 100%.
fn load_birth_sign_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    initialize_form_from_current_record(world, form, cx)?;
    loop {
        match cx.file.chunk_type()? {
            EDID => {
                cx.file.read_text(0x200)?;
            }
            DESC => load_description(
                &mut world
                    .get_mut::<TesDescription>(form)
                    .expect("the class's component"),
                cx,
            )?,
            FULL => load_full_name(
                &mut world
                    .get_mut::<TesFullName>(form)
                    .expect("the class's component")
                    .name_04,
                cx,
            )?,
            ICON => load_texture(
                &mut world
                    .get_mut::<TesTexture>(form)
                    .expect("the class's component"),
                cx,
            )?,
            SPLO => {
                let (id, _) = cx.file.read_dword(0)?;
                let loading = world.resource::<GameThreadLocals>().byte_184 != 0;
                world
                    .get_mut::<TesSpellList>(form)
                    .expect("the class's component")
                    .add_spell_while_loading(id, loading)?;
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
    }
    set_initialized(world, form, false);
    Ok(true)
}

// IDA: 005195F0 BirthSign_Constructor: 0046C460; +0x18 TESFullName inline; 0046FFD0 on +0x24; 0046A400 on +0x30; 0046F2C0 on +0x38; the five vtables; +0x04 = 0x11; 0046AB80(1). Completion: 100% of the store order.
fn construct_birth_sign(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_tes_form(world)?;
    let mut description = TesDescription {
        original_vtable: BIRTH_SIGN_VTABLES[3],
        offset_04: 0,
    };
    reset_description(
        &mut description,
        &mut world.resource_mut::<DataHandlerGlobals>(),
    );
    world.entity_mut(form).insert((
        OriginalVtable(BIRTH_SIGN_VTABLES[0]),
        TesFullName {
            original_vtable: BIRTH_SIGN_VTABLES[1],
            name_04: BsStringT::default(),
        },
        TesTexture {
            original_vtable: BIRTH_SIGN_VTABLES[2],
            path_04: BsStringT::default(),
        },
        description,
        TesSpellList {
            original_vtable: BIRTH_SIGN_VTABLES[4],
            ..TesSpellList::default()
        },
    ));
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_BIRTH_SIGN;
    set_initialized(world, form, true);
    Ok(form)
}

// IDA: 004B04A0 TESLevSpell vt+0x1C LoadForm. `cmp al, 0x40` with the record's type (else 0); 00451210; a pass per chunk until 0044FEA0 fails: LVLD into a byte local, size 0: 0046CB10 from bit 7, 0046CAF0 with the low seven bits; LVLF, size 0, at +0x31; EDID (alloca, size 0x200, vt+0xD8 keeps nothing); LVLO: zeroed locals with the word +0x08 = 1, 00450C20(0x0C), 0046CBF0(+0x00, +0x08, +0x04). 0046AB80(0). Returns 1. A chunk past one byte runs on past the local or +0x31 in the original, and those are a host stop. Completion: 100%.
fn load_lev_spell_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_LEV_SPELL {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    loop {
        match cx.file.chunk_type()? {
            LVLD => {
                if cx.file.chunk_size()? > 1 {
                    return Err(GameError::unclassified(
                        0x004B0504,
                        "004B0504: an LVLD past one byte overruns the local",
                    ));
                }
                let mut value = [0_u8; 1];
                cx.file.read_chunk(&mut value, 0)?;
                let mut list = world
                    .get_mut::<TesLeveledList>(form)
                    .expect("the class's component");
                if value[0] & 0x80 != 0 {
                    list.flags_0d |= 1;
                } else {
                    list.flags_0d &= 0xFE;
                }
                list.chance_none_0c = value[0] & 0x7F;
            }
            LVLF => {
                if cx.file.chunk_size()? > 1 {
                    return Err(GameError::unclassified(
                        0x004B0561,
                        "004B0561: an LVLF past one byte overruns +0x31",
                    ));
                }
                let mut value = [world
                    .get::<TesLeveledList>(form)
                    .expect("the class's component")
                    .flags_0d];
                cx.file.read_chunk(&mut value, 0)?;
                world
                    .get_mut::<TesLeveledList>(form)
                    .expect("the class's component")
                    .flags_0d = value[0];
            }
            EDID => {
                cx.file.read_text(0x200)?;
            }
            LVLO => {
                let mut bytes = [0_u8; 0x0C];
                bytes[8] = 1;
                cx.file.read_chunk(&mut bytes, 0x0C)?;
                let level = u16::from_le_bytes([bytes[0], bytes[1]]);
                let form_id = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                let count = i16::from_le_bytes([bytes[8], bytes[9]]);
                world
                    .get_mut::<TesLeveledList>(form)
                    .expect("the class's component")
                    .add_entry(level, count, form_id);
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
    }
    set_initialized(world, form, false);
    Ok(true)
}

// IDA: 004B0400 TESLevSpell_Constructor: 004B2FF0 (0046C460, +0x1C = +0x20 = +0x18 = 0); 0046CB80 on +0x24; the two vtables; +0x04 = 0x40; 0046AB80(1). Completion: 100% of the store order.
fn construct_lev_spell(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        OriginalVtable(LEV_SPELL_VTABLES[0]),
        TesObjectLinks::default(),
        TesLeveledList {
            original_vtable: LEV_SPELL_VTABLES[1],
            ..TesLeveledList::default()
        },
    ));
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_LEV_SPELL;
    set_initialized(world, form, true);
    Ok(form)
}

/// IDA: this family's classes. MGEF, BSGN and LVSP have an InitItem: `no_init_item` here.
/// TODO: [ ] 12d: 00415960 (MGEF), 00519500 (BSGN) and 004B02D0 (LVSP), read by the vt+0x6C passes.
pub const CLASSES: &[FormClass] = &[
    FormClass {
        form_type: FORM_TYPE_EFFECT_SETTING,
        vtable: EFFECT_SETTING_VTABLES[0],
        construct: construct_effect_setting,
        load_form: load_effect_setting_form,
        reset_14: reset_14_components,
        reset_18: reset_18_components,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_effect_setting,
        slots: &[
            ComponentSlot::Description,
            ComponentSlot::FullName,
            ComponentSlot::Texture,
            ComponentSlot::Model,
        ],
        special_arm: Some(effect_setting_arm),
    },
    FormClass {
        form_type: FORM_TYPE_ENCHANTMENT,
        vtable: ENCHANTMENT_VTABLES[0],
        construct: construct_enchantment,
        load_form: load_magic_form,
        reset_14: reset_enchantment,
        reset_18: destroy_magic_form,
        editor_id: default_editor_id,
        init_item: no_init_item,
        slots: &[ComponentSlot::FullName],
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_SPELL,
        vtable: SPELL_VTABLES[0],
        construct: construct_spell,
        load_form: load_magic_form,
        reset_14: reset_spell,
        reset_18: destroy_magic_form,
        editor_id: default_editor_id,
        init_item: no_init_item,
        slots: &[ComponentSlot::FullName],
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_BIRTH_SIGN,
        vtable: BIRTH_SIGN_VTABLES[0],
        construct: construct_birth_sign,
        load_form: load_birth_sign_form,
        reset_14: reset_14_components,
        reset_18: reset_18_components,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_birth_sign,
        slots: &[
            ComponentSlot::Description,
            ComponentSlot::FullName,
            ComponentSlot::Texture,
            ComponentSlot::SpellList,
        ],
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_LEV_SPELL,
        vtable: LEV_SPELL_VTABLES[0],
        construct: construct_lev_spell,
        load_form: load_lev_spell_form,
        reset_14: reset_14_components,
        reset_18: reset_18_components,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_leveled_form,
        slots: &[ComponentSlot::LeveledList],
        special_arm: None,
    },
];

/// IDA: 00412F20 EffectItem_GetSchool: the script's +0x04, else the setting's +0x64. Completion: 100%.
pub(crate) fn effect_school(world: &World, effect: &EffectItem) -> u32 {
    match (&effect.script_18, effect.setting_1c) {
        (Some(script), _) => script.school_04,
        (None, Some(setting)) => {
            world
                .get::<EffectSetting>(setting)
                .expect("an MGEF")
                .data_58
                .school_64
        }
        (None, None) => 0,
    }
}

/// IDA: 00414190(other) on two effect items: different code, magnitude (0 with flag 0x100), area (0 with flag 0x200 or a
/// self range), duration (0 with flag 0x80), range, actor value, or 'SEFF'-ness answers 1 (different). Completion: those
/// tests; equal items go on to its script and hostility tests, a stop.
pub(crate) fn effect_items_differ(
    world: &World,
    a: &EffectItem,
    b: &EffectItem,
) -> Result<bool, GameError> {
    if a.data_00[0] != b.data_00[0] {
        return Ok(true);
    }
    let fa = setting_flags(world, a.setting_1c)?;
    let fb = setting_flags(world, b.setting_1c)?;
    let magnitude = |flags: u32, item: &EffectItem| {
        if flags & 0x100 != 0 {
            0
        } else {
            item.data_00[1]
        }
    };
    let area = |flags: u32, item: &EffectItem| {
        if flags & 0x200 != 0 || item.data_00[4] == 0 {
            0
        } else {
            item.data_00[2]
        }
    };
    let duration = |flags: u32, item: &EffectItem| {
        if flags & 0x80 != 0 {
            0
        } else {
            item.data_00[3]
        }
    };
    if magnitude(fb, b) != magnitude(fa, a)
        || area(fb, b) != area(fa, a)
        || duration(fb, b) != duration(fa, a)
        || b.data_00[4] != a.data_00[4]
        || b.data_00[5] != a.data_00[5]
    {
        return Ok(true);
    }
    Err(GameError::unclassified(
        0x004141F8,
        "004141F8: 00414190's script and hostility tests for equal effects",
    ))
}

// IDA: the flag of EffectSetting +0x58 that 004152C0, 00419B30, 00414FE0 and 00415030 skip.
const EFFECT_FLAG_NO_COST_PICK: u32 = 0x40_0000;

const EFFECT_CODE_SEFF: u32 = 0x4646_4553;

// IDA: 00413810: a cost below 0.0 is worked out and stored at the effect's +0x20. Flags 0x100, 0x80 and 0x200 of the setting drop magnitude, duration and area; a self range drops area too. The null setting faults.
pub(crate) fn effect_cost(
    world: &mut World,
    spell: Entity,
    index: usize,
) -> Result<f32, GameError> {
    let list = world.get::<EffectItemList>(spell).expect("a magic item");
    let item = &list.items_04[index];
    if item.cost_20 >= 0.0 {
        return Ok(item.cost_20);
    }
    let setting = item.setting_1c.ok_or(GameError::unclassified(
        0x00413820,
        "00413820: a null effect setting faults",
    ))?;
    let data = world
        .get::<EffectSetting>(setting)
        .expect("a setting")
        .data_58;
    let flags = data.flags_58;
    let [_, magnitude, area, duration, range, _] = item.data_00;
    let magnitude = if flags & 0x100 != 0 { 0 } else { magnitude };
    let duration = if flags & 0x80 != 0 { 0 } else { duration };
    let area = if flags & 0x200 != 0 || range == 0 {
        0
    } else {
        area
    };
    let cost = base_effect_cost(
        world,
        data.base_cost_5c,
        area,
        duration,
        magnitude,
        range == 2,
    );
    world
        .get_mut::<EffectItemList>(spell)
        .expect("a magic item")
        .items_04[index]
        .cost_20 = cost;
    Ok(cost)
}

// IDA: 00413890 EffectItem_GetMagickaCostForCaster with an actor: the cached cost (00413810); the effect's school (the
// script info's +4, else the setting's +0x64) as an actor value (0041B770: 0..5 to 20..25); Luck (7) and that skill
// through vt+0x284; 00548C00(cost, skill, luck); then floored as 00404F70 does (004138F4..0041392A, inlined) and at
// least 1.0 (00413932 `fcom`, `test ah, 41h`: at or below 1.0, or unordered, answers 1.0). A null caster (004138A4)
// goes straight to the floor. Completion: 100%.
pub(crate) fn effect_cost_for_caster(
    world: &mut World,
    spell: Entity,
    index: usize,
    caster: Option<Entity>,
) -> Result<f32, GameError> {
    let mut cost = effect_cost(world, spell, index)?;
    if let Some(caster) = caster {
        let item = &world
            .get::<EffectItemList>(spell)
            .expect("a magic item")
            .items_04[index];
        let school = match &item.script_18 {
            Some(script) => script.school_04,
            None => {
                let setting = item.setting_1c.ok_or(GameError::unclassified(
                    0x004138B5,
                    "004138B5: a null effect setting faults",
                ))?;
                world
                    .get::<EffectSetting>(setting)
                    .expect("a setting")
                    .data_58
                    .school_64
            }
        };
        if school > 5 {
            return Err(GameError::unclassified(
                0x004138CB,
                "004138CB: a school with no actor value is carried",
            ));
        }
        let luck = player_get_actor_value(world, caster, 7)?;
        let skill = player_get_actor_value(world, caster, 20 + school)?;
        cost = caster_skill_cost(world, cost, skill, luck);
    }
    let floor = oblivion_core::x87::floor_by_ftol(cost);
    // 00413932..00413939: `fcom` against 1.0, `test ah, 41h`, `jnz`: below, equal or unordered answers 1.0.
    Ok(if floor > 1.0 { floor } else { 1.0 })
}

/// IDA: 004152C0 EffectItemList_FindHighestCostEffect(list, 3, false): strictly highest, skipping the setting flag 0x400000. Each cost goes through __ftol2. Completion: 100% of the any-range, no-area call.
/// # Errors
/// A null effect setting.
pub fn highest_cost_effect(world: &mut World, spell: Entity) -> Result<Option<usize>, GameError> {
    highest_cost_effect_in_range(world, spell, 3)
}

/// IDA: 004152C0 EffectItemList_FindHighestCostEffect(list, range, false): an empty list answers null; each effect's 00413890 cost, strictly above the best so far, with the setting's 0x400000 clear and, unless `range` is 3 (any), its +0x10 range equal to `range`. Completion: 100% of the no-area calls.
/// # Errors
/// A null effect setting.
pub fn highest_cost_effect_in_range(
    world: &mut World,
    spell: Entity,
    range: u32,
) -> Result<Option<usize>, GameError> {
    let count = world
        .get::<EffectItemList>(spell)
        .expect("a magic item")
        .items_04
        .len();
    let mut best = -1_i32;
    let mut found = None;
    for index in 0..count {
        let cost = effect_cost_for_caster(world, spell, index, None)? as i32;
        let item = &world
            .get::<EffectItemList>(spell)
            .expect("a magic item")
            .items_04[index];
        let effect_range = item.data_00[4];
        let setting = item.setting_1c.ok_or(GameError::unclassified(
            0x00415303,
            "00415303: a null effect setting faults",
        ))?;
        let flags = world
            .get::<EffectSetting>(setting)
            .expect("a setting")
            .data_58
            .flags_58;
        if flags & EFFECT_FLAG_NO_COST_PICK == 0
            && cost > best
            && (range == 3 || effect_range == range)
        {
            found = Some(index);
            best = cost;
        }
    }
    Ok(found)
}

/// IDA: 00415360 MagicItem_GetHighestCostEffectSchoolActorValue: no effect answers -1; 00412F20, the script's school, else the setting's; 0041B770 maps 0..5 to 20..25 and anything else to -1. Completion: 100%.
/// # Errors
/// A null effect setting.
pub fn magic_item_school_actor_value(world: &mut World, spell: Entity) -> Result<i32, GameError> {
    let Some(index) = highest_cost_effect(world, spell)? else {
        return Ok(-1);
    };
    let item = &world
        .get::<EffectItemList>(spell)
        .expect("a magic item")
        .items_04[index];
    let school = match &item.script_18 {
        Some(script) => script.school_04,
        None => {
            let setting = item.setting_1c.ok_or(GameError::unclassified(
                0x00412F2B,
                "00412F2B: a null effect setting faults",
            ))?;
            world
                .get::<EffectSetting>(setting)
                .expect("a setting")
                .data_58
                .school_64
        }
    };
    Ok(if school <= 5 { school as i32 + 20 } else { -1 })
}

/// IDA: 00419B30 with 0: the art setting of a plain spell, by highest range, then highest cost, past the flag 0x400000. Types 5, 7 and 1 look up POSN and DISE. Completion: 100% of a spell with a type other than those.
/// # Errors
/// Those types, a script effect, or a null setting.
pub fn magic_item_art_setting(
    world: &mut World,
    spell: Entity,
) -> Result<Option<Entity>, GameError> {
    let kind = world
        .get::<MagicFormData>(spell)
        .map_or(0, |data| data.type_34);
    if matches!(kind, 1 | 5 | 7) {
        return Err(GameError::unclassified(
            0x00419B55,
            "00419B55: a poison or disease's art setting, 00414EB0 and the collection",
        ));
    }
    let count = world
        .get::<EffectItemList>(spell)
        .expect("a magic item")
        .items_04
        .len();
    let mut best_cost = -1_i32;
    let mut best_range = 0_i32;
    let mut found = None;
    for index in 0..count {
        let cost = effect_cost_for_caster(world, spell, index, None)? as i32;
        let item = &world
            .get::<EffectItemList>(spell)
            .expect("a magic item")
            .items_04[index];
        let range = item.data_00[4] as i32;
        let setting = item.setting_1c.ok_or(GameError::unclassified(
            0x00419BC1,
            "00419BC1: a null effect setting faults",
        ))?;
        let skipped = world
            .get::<EffectSetting>(setting)
            .expect("a setting")
            .data_58
            .flags_58
            & EFFECT_FLAG_NO_COST_PICK
            != 0;
        if !skipped && (range > best_range || (range == best_range && cost > best_cost)) {
            found = Some(index);
            best_cost = cost;
            best_range = range;
        }
    }
    let Some(index) = found else {
        return Ok(None);
    };
    let item = &world
        .get::<EffectItemList>(spell)
        .expect("a magic item")
        .items_04[index];
    if item.data_00[0] == EFFECT_CODE_SEFF {
        return Err(GameError::unclassified(
            0x00419C08,
            "00419C08: a script effect's art, 00412CB0",
        ));
    }
    Ok(item.setting_1c)
}

// IDA: 00419CF0 and 00419E50 on a spell none of whose effects has a summon flag, with the art setting's +0xA0 at 0: the first false, the second false, so 0041A6A7's arm. Any other state of +0xA0, or a summon effect, is an arm no run reached.
pub(crate) fn magic_item_load_request(
    world: &mut World,
    spell: Entity,
) -> Result<Option<MagicItemModelRequest>, GameError> {
    let Some(setting) = magic_item_art_setting(world, spell)? else {
        return Ok(None);
    };
    let list = world.get::<EffectItemList>(spell).expect("a magic item");
    for item in &list.items_04 {
        let flags = item
            .setting_1c
            .and_then(|effect| world.get::<EffectSetting>(effect))
            .map_or(0, |effect| effect.data_58.flags_58);
        if flags & EFFECT_FLAGS_SUMMON != 0 {
            return Err(GameError::unclassified(
                0x00419D5C,
                "00419D5C: an effect with a summon flag",
            ));
        }
    }
    let counter = world
        .get::<EffectSetting>(setting)
        .expect("a setting")
        .art_counter_a0;
    if counter != 0 {
        return Err(GameError::unclassified(
            0x00419D2C,
            "00419D2C: an art setting already queued or loaded",
        ));
    }
    let model = world.get::<TesModel>(setting).expect("a setting's model");
    let path = (model.path_04.length_04 != 0)
        .then(|| model.path_04.data_00.clone())
        .flatten();
    Ok(Some(MagicItemModelRequest {
        spell,
        setting,
        path,
    }))
}

/// IDA: 0041A610 MagicItem_QueueResources(item, 0). The art setting (00419B30). Loaded (00419CF0 MagicItem_IsResourcesLoaded: the art has no path or is loaded, +0xA0 above 0, and every associated item of an effect with 0x70000 is loaded, +0xA4 above 0); otherwise neither loaded nor queued (00419E50 false) takes 00438300 QueuedMagicItem_Start (`MagicItemModelRequest`, the caller's to start) and returns; otherwise queued. Loaded with an art path loads the art now (0041A6E5 ModelLoader_LoadModelNow), a stop; then 004157A0 steps +0xA0 away from 0. Each effect with 0x70000: loaded, its associated item's model loads now (0041A747), a stop; a summon (0x40000) neither loaded nor queued counts 00B33518, a stop; then 00415740 steps +0xA4 away from 0. Completion: 100% but the loaded arms and summons.
/// # Errors
/// A loaded arm's synchronous loads, a summon, or the art setting's stops.
pub fn magic_item_queue_resources(
    world: &mut World,
    spell: Entity,
) -> Result<Option<MagicItemModelRequest>, GameError> {
    let setting = magic_item_art_setting(world, spell)?;
    let associated = associated_settings(world, spell);
    let art = setting.map(|setting| art_state(world, setting));
    let loaded = art.is_none_or(|(path, counter)| path == 0 || counter > 0)
        && associated
            .iter()
            .all(|&effect| associated_counter(world, effect) > 0);
    if !loaded && !resources_queued_or_loaded(world, art, &associated) {
        let Some(setting) = setting else {
            return Ok(None);
        };
        return Ok(Some(MagicItemModelRequest {
            spell,
            setting,
            path: art_path(world, setting),
        }));
    }
    if let (Some(setting), Some((path, counter))) = (setting, art) {
        if loaded && path != 0 {
            return Err(GameError::unclassified(
                0x0041A6E5,
                "0041A6E5: loading a loaded art model now",
            ));
        }
        let next = if counter >= 0 {
            counter + 1
        } else {
            counter - 1
        };
        world
            .get_mut::<EffectSetting>(setting)
            .expect("a setting")
            .art_counter_a0 = next as u32;
    }
    for effect in associated {
        if loaded {
            return Err(GameError::unclassified(
                0x0041A747,
                "0041A747: loading a loaded associated item now",
            ));
        }
        let counter = associated_counter(world, effect);
        let flags = effect_flags(world, effect);
        if counter == 0 && flags & 0x4_0000 != 0 {
            return Err(GameError::unclassified(
                0x0041AAE5,
                "0041AAE5: a summon counted in 00B33518",
            ));
        }
        let next = if counter >= 0 {
            counter + 1
        } else {
            counter - 1
        };
        world
            .get_mut::<EffectSetting>(effect)
            .expect("a setting")
            .associated_counter_a4 = next as u32;
    }
    Ok(None)
}

// IDA: 00419E50 MagicItem_IsResourcesQueuedOrLoaded: an art with a path that isn't loaded clears the all-loaded flag and must be queued (+0xA0 below 0, 004157F0), else false; each effect with 0x70000 whose item isn't loaded (00415780) the same on +0xA4 (00415790); true when something was only queued.
pub(crate) fn resources_queued_or_loaded(
    world: &World,
    art: Option<(usize, i32)>,
    associated: &[Entity],
) -> bool {
    let mut all_loaded = true;
    if let Some((path, counter)) = art
        && path != 0
        && counter <= 0
    {
        all_loaded = false;
        if counter >= 0 {
            return false;
        }
    }
    for &effect in associated {
        let counter = associated_counter(world, effect);
        if counter <= 0 {
            all_loaded = false;
            if counter >= 0 {
                return false;
            }
        }
    }
    !all_loaded
}

/// IDA: 00416900 the 00B33508 map, in bucket order, each value's vt+0x6C. Completion: 100%.
pub fn init_effect_settings(
    world: &mut World,
    services: &mut DataLoadServices<'_>,
) -> Result<(), GameError> {
    let settings: Vec<Entity> = world
        .resource::<EffectSettingCollection>()
        .buckets_08
        .iter()
        .flatten()
        .map(|&(_, form)| form)
        .collect();
    for form in settings {
        init_item_of(world, form, services)?;
    }
    Ok(())
}

const SETTING_FLAG_SAVED: u32 = 0x20_0000;

// IDA: 00416A80 and 00416BA0's walk: EffectSettingCollection's buckets in order, each chain in order, the keys of settings with flag 0x200000.
pub(crate) fn saved_effect_settings(world: &World) -> Vec<u32> {
    let collection = world.resource::<EffectSettingCollection>();
    let mut keys = Vec::new();
    for chain in &collection.buckets_08 {
        for (key, setting) in chain {
            let flags = world
                .get::<EffectSetting>(*setting)
                .expect("a setting")
                .data_58
                .flags_58;
            if flags & SETTING_FLAG_SAVED != 0 {
                keys.push(*key);
            }
        }
    }
    keys
}

// IDA: 00414FE0 (range 2) and 00415030 (range 1): an effect of the range whose setting lacks 0x400000; an empty list is
// false.
pub(crate) fn has_range(world: &World, spell: Entity, range: u32) -> bool {
    let Some(list) = world.get::<EffectItemList>(spell) else {
        return false;
    };
    list.items_04.iter().any(|item| {
        item.data_00[4] == range
            && item
                .setting_1c
                .and_then(|setting| world.get::<EffectSetting>(setting))
                .is_some_and(|setting| setting.data_58.flags_58 & EFFECT_FLAG_NO_COST_PICK == 0)
    })
}

/// IDA: 0041D320, a spell's EffectItemList vt+0 (MagicItem_GetMagickaCost) with a caster: a power (2) or lesser power (3)
/// drops the caster; the +0x40 bit 0 (a set cost) takes +0x38 through 00548C00; else 004151C0: the effects past the
/// setting flag 0x400000 summed through 00413890, an empty list 0.0, the sum floored by 00404F70 when not negative, else
/// 0.0. Completion: 100% of the computed cost.
/// TODO: [ ] The set-cost arm (+0x40 bit 0).
/// # Errors
/// A set cost, a null effect setting, or an actor value that stops.
pub fn magic_item_cost_for_caster(
    world: &mut World,
    spell: Entity,
    caster: Option<Entity>,
) -> Result<f32, GameError> {
    let data = world
        .get::<MagicFormData>(spell)
        .copied()
        .unwrap_or_default();
    let caster = if matches!(data.type_34, 2 | 3) {
        None
    } else {
        caster
    };
    if data.flags_40 & 1 != 0 {
        return Err(GameError::unclassified(
            0x0041D34C,
            "0041D34C: a magic item's set cost is carried",
        ));
    }
    let count = world
        .get::<EffectItemList>(spell)
        .map_or(0, |list| list.items_04.len());
    if count == 0 {
        return Ok(0.0);
    }
    let mut sum = 0.0_f32;
    for index in 0..count {
        let setting = world
            .get::<EffectItemList>(spell)
            .expect("a magic item")
            .items_04[index]
            .setting_1c
            .ok_or(GameError::unclassified(
                0x004151EC,
                "004151EC: a null effect setting faults",
            ))?;
        let flags = world
            .get::<EffectSetting>(setting)
            .expect("a setting")
            .data_58
            .flags_58;
        if flags & EFFECT_FLAG_NO_COST_PICK == 0 {
            sum += effect_cost_for_caster(world, spell, index, caster)?;
        }
    }
    // 00415211..0041521D: `fcom` of 0.0 with the sum; only 0.0 above it gives 0.0, so a NaN is floored.
    Ok(if sum < 0.0 {
        0.0
    } else {
        x87::floor_by_ftol(sum)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Bytes at 004168C5: each `movsx`, so a byte past 0x7F floods the bits above it.
    #[test]
    fn a_name_past_0x7f_sign_extends_into_the_key() {
        assert_eq!(effect_code_of_name(b"REHE"), 0x4548_4552);
        assert_eq!(effect_code_of_name(b"\xC1EHE"), 0xFFFF_FFC1);
        assert_eq!(effect_code_of_name(b"RE\xC8E"), 0xFFC8_4552);
    }

    #[test]
    fn the_table_is_the_exes_size_and_each_code_is_in_its_own_chain() {
        assert_eq!(BUILT_IN_EFFECTS.len(), 161);
        let mut seen = std::collections::BTreeSet::new();
        assert!(
            BUILT_IN_EFFECTS
                .iter()
                .all(|effect| seen.insert(effect.code))
        );
    }
}
