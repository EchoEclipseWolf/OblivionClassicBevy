//! Phase 11 (D2, D6): the actor processes and the ActorProcessManager the PlayerCharacter constructor chain reaches. Actor_ctor 005E15E0 allocates a LowProcess (FormHeap 0x90, 006438F0) and adds the
//! actor to the manager's level-3 list (00673A90); PlayerCharacter_ctor 0066A740 builds a HighProcess (FormHeap 0x2EC, 00628EE0 over MiddleHigh 0064B400, MiddleLow 00658690, Low and Base), copies the
//! old process into it (vt+4 0060CDB0 -> 00643520), removes the actor from the level-3 list (00674550), deletes the LowProcess (vt+0 0064ACD0 -> 00648CB0), stores the HighProcess at +0x58 and removes
//! it again with level 0 (High).
//!
//! A process is an entity with one capability component per class level (`BaseProcess`, `LowProcess`, `MiddleLowProcess`, `MiddleHighProcess`, `HighProcess`) and its most-derived vtable in
//! `OriginalVtable`, so the RTTI casts of 0060CDB0 are component tests. `g_actorProcessManager` 00B3BD00 is a CRT static (cinit 009FE5F0 -> 0067AF60 before WinMain, carried from Phase 00); nothing
//! drives the CRT statics yet, so whoever builds the world calls `construct_actor_process_manager`. Uninitialized FormHeap bytes are host zero (recorded substitution, as in Phase 09); the one copied
//! value (LowProcess +0x88 moved into the HighProcess by 00643520) comes through `ProcessInputs`.
//! Evidence: evidence/phase11-window-2026-10-06/disasm-chain.txt, disasm-reached.txt, disasm-tlslock.txt; P11-RUN-001.

use bevy_ecs::prelude::*;
use oblivion_core::crt::{MsvcRand, RandomGlobals, random_next};
use oblivion_core::error::GameError;
use oblivion_core::pointer_list::PointerList;
use oblivion_core::x87;

use super::actor::{Actor, MobileObject, TesObjectRefr, actor_process_level};
use super::player::{PlayerGlobals, player_is_sleeping};
use crate::bs::task_manager::LockFreeMap;
use crate::tes::form::{FormIdentity, OriginalVtable};
use crate::tes::forms_world::{cell_attached, cell_has_type_0d};
use crate::tes::game_settings::GameSettingCollection;
use crate::tes::tes::TesGlobals;

/// IDA: vtables stored by the process constructors (??_7*Process@@6B@).
pub const BASE_PROCESS_VTABLE: u32 = 0x00A6_F464;
pub const LOW_PROCESS_VTABLE: u32 = 0x00A7_20A4;
pub const MIDDLE_LOW_PROCESS_VTABLE: u32 = 0x00A7_2D64;
pub const MIDDLE_HIGH_PROCESS_VTABLE: u32 = 0x00A7_2684;
pub const HIGH_PROCESS_VTABLE: u32 = 0x00A7_1814;
/// IDA: ??_7?$LockFreeMap@PAVActor@@V?$NiPointer@VLipTask@@@@@@6B@ (00677DD0).
pub const LOCK_FREE_MAP_ACTOR_LIP_TASK_VTABLE: u32 = 0x00A7_2044;

/// IDA: ActorValueEntry (8 bytes, FormHeap): +0x00 u8 actor-value code, +0x01..+0x03 unset, +0x04 float modifier.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActorValueEntry {
    pub code_00: u8,
    pub value_04: f32,
}

/// IDA: ActorValueIndexedEntries (0x48 FormHeap block, 18 entry pointers; 0065C010 reads +0x00..+0x44). Not created in the Phase 11 window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActorValueIndexedEntries {
    pub entries: [Option<ActorValueEntry>; 18],
}

/// IDA: ActorValues (0x14; Actor +0x88, LowProcess +0x70, MiddleLowProcess +0x94): +0x00/+0x04 the BSSimpleList<ActorValueEntry*> head (here in list order), +0x08 magicka entry (code 9), +0x0C
/// fatigue entry (code 10), +0x10 the indexed block.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActorValues {
    pub list_00: Vec<ActorValueEntry>,
    pub magicka_08: Option<ActorValueEntry>,
    pub fatigue_0c: Option<ActorValueEntry>,
    pub indexed_10: Option<Box<ActorValueIndexedEntries>>,
}

// IDA: 0065BE10 ActorValues_Constructor: +0x00 = +0x04 = 0; FormHeap(8) {code 10, 0.0} -> +0x0C; FormHeap(8) {code 9, 0.0} -> +0x08; +0x10 = 0 (a failed second allocation stores 0 into +0x08 and +0x10). The
// +0x01..+0x03 entry bytes stay unset. Completion: 100%.
#[must_use]
pub fn construct_actor_values() -> ActorValues {
    ActorValues {
        list_00: Vec::new(),
        fatigue_0c: Some(ActorValueEntry {
            code_00: 10,
            value_04: 0.0,
        }),
        magicka_08: Some(ActorValueEntry {
            code_00: 9,
            value_04: 0.0,
        }),
        indexed_10: None,
    }
}

// IDA: 0065C6A0 ActorValues_Clear: release list entries from the head, then the indexed block.
// Completion: both containers are owned by Rust; the permanent magicka and fatigue entries survive.
fn clear_actor_values(values: &mut ActorValues) {
    values.list_00.clear();
    values.indexed_10 = None;
}

// IDA: 0065C010 ActorValues_GetEntry and 0065C710 ActorValues_SetIndexedEntry use the same 18 slots.
const fn indexed_actor_value_slot(code: u8) -> Option<usize> {
    Some(match code {
        8 => 0,
        11 => 1,
        40 => 2,
        48 => 3,
        36 => 4,
        49 => 5,
        6 => 6,
        56 => 7,
        46 => 8,
        47 => 9,
        41 => 10,
        33 => 11,
        26 => 12,
        5 => 13,
        7 => 14,
        0 => 15,
        4 => 16,
        13 => 17,
        _ => return None,
    })
}

// IDA: 0065C010 ActorValues_GetEntry. Completion: the two permanent entries, indexed block and linked list.
fn actor_value_entry(values: &ActorValues, code: u8) -> Option<&ActorValueEntry> {
    match code {
        9 => values.magicka_08.as_ref(),
        10 => values.fatigue_0c.as_ref(),
        _ => match indexed_actor_value_slot(code) {
            Some(slot) => values.indexed_10.as_ref()?.entries[slot].as_ref(),
            None => values.list_00.iter().find(|entry| entry.code_00 == code),
        },
    }
}

/// IDA: 0065CB80 ActorValues_GetModifier. Completion: the entry value or zero.
#[must_use]
pub fn actor_values_get(values: &ActorValues, code: u8) -> f32 {
    actor_value_entry(values, code).map_or(0.0, |entry| entry.value_04)
}

// IDA: 0065C8F0 ActorValues_InsertEntry. A copy always inserts, including zero-valued list entries.
// Completion: all code arms, with host-owned storage in place of FormHeap allocation.
fn insert_actor_value_entry(
    values: &mut ActorValues,
    entry: ActorValueEntry,
) -> Result<(), GameError> {
    match entry.code_00 {
        9 | 10 => {
            let permanent = if entry.code_00 == 9 {
                &mut values.magicka_08
            } else {
                &mut values.fatigue_0c
            };
            permanent
                .as_mut()
                .ok_or_else(|| {
                    GameError::unclassified(
                        0x0065C8F0,
                        "inserting into a missing permanent ActorValues entry",
                    )
                })?
                .value_04 = entry.value_04;
        }
        code => {
            if let Some(slot) = indexed_actor_value_slot(code) {
                values
                    .indexed_10
                    .get_or_insert_with(Default::default)
                    .entries[slot] = Some(entry);
            } else {
                values.list_00.insert(0, entry);
            }
        }
    }
    Ok(())
}

// IDA: 0065CB00 ActorValues_SetModifier, 0065C9B0 ActorValues_ClearEntry, 0065C8F0 ActorValues_InsertEntry.
// Completion: existing entries keep their place; clearing the last indexed slot frees its block.
// Host substitution: Vec and Box own the list nodes and the 0x48-byte indexed block.
pub fn actor_values_set_modifier(
    values: &mut ActorValues,
    code: u8,
    value: f32,
) -> Result<(), GameError> {
    if actor_value_entry(values, code).is_none() {
        if value != 0.0 {
            return insert_actor_value_entry(
                values,
                ActorValueEntry {
                    code_00: code,
                    value_04: value,
                },
            );
        }
    } else if matches!(code, 9 | 10) {
        let permanent = if code == 9 {
            &mut values.magicka_08
        } else {
            &mut values.fatigue_0c
        };
        permanent.as_mut().unwrap().value_04 = if value == 0.0 { 0.0 } else { value };
    } else if let Some(slot) = indexed_actor_value_slot(code) {
        let block = values.indexed_10.as_mut().unwrap();
        if value == 0.0 {
            block.entries[slot] = None;
            if block.entries.iter().all(Option::is_none) {
                values.indexed_10 = None;
            }
        } else {
            block.entries[slot].as_mut().unwrap().value_04 = value;
        }
    } else if let Some(index) = values.list_00.iter().position(|item| item.code_00 == code) {
        if value == 0.0 {
            values.list_00.remove(index);
        } else {
            values.list_00[index].value_04 = value;
        }
    }
    Ok(())
}

/// IDA: 0065CA60 ActorValues_AddModifier with allowPositive = 1.
pub fn actor_values_add_modifier(
    values: &mut ActorValues,
    code: u8,
    delta: f32,
) -> Result<(), GameError> {
    add_actor_value_modifier(values, code, delta, true)
}

// IDA: 0065CA60 ActorValues_AddModifier. Only an existing entry is clamped when allowPositive is clear.
pub(super) fn add_actor_value_modifier(
    values: &mut ActorValues,
    code: u8,
    delta: f32,
    allow_positive: bool,
) -> Result<(), GameError> {
    let value = if let Some(entry) = actor_value_entry(values, code) {
        let sum = entry.value_04 + delta;
        if !allow_positive && sum > 0.0 {
            0.0
        } else {
            sum
        }
    } else {
        delta
    };
    actor_values_set_modifier(values, code, value)
}

// IDA: 0065CD10 ActorValues_CopyFrom. Clear, copy fatigue then magicka, then the
// indexed slots in ascending offset order and the linked entries from the source head.
// Completion: successful allocations, including the reversed destination list from prepending.
pub fn copy_actor_values(
    destination: &mut ActorValues,
    source: Option<&ActorValues>,
) -> Result<(), GameError> {
    // 0065CD14 0065C6A0 on this.
    clear_actor_values(destination);
    // 0065CD1D a null source returns.
    let Some(source) = source else {
        return Ok(());
    };
    // 0065CD25..0065CD43 10 with the source fatigue value (0.0 without an entry).
    actor_values_set_modifier(
        destination,
        10,
        source.fatigue_0c.map_or(0.0, |entry| entry.value_04),
    )?;
    // 0065CD48..0065CD66 9 with the source magicka value.
    actor_values_set_modifier(
        destination,
        9,
        source.magicka_08.map_or(0.0, |entry| entry.value_04),
    )?;
    if let Some(indexed) = &source.indexed_10 {
        for &entry in indexed.entries.iter().flatten() {
            insert_actor_value_entry(destination, entry)?;
        }
    }
    for &entry in &source.list_00 {
        insert_actor_value_entry(destination, entry)?;
    }
    Ok(())
}

/// IDA: 0060CD90 BaseProcess_ctor (vtable A6F464): the head every process shares. The most-derived vtable is `OriginalVtable`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BaseProcess {
    pub edit_pack_procedure_type_04: u32,
    /// +0x08 the current package.
    pub editor_package_08: Option<Entity>,
}

/// IDA: 006438F0 LowProcess_ctor (0x90).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct LowProcess {
    /// +0x0C (-1.0).
    pub field_0c: f32,
    pub field_10: f32,
    /// +0x14 (-1.0; vt+0x28 00629250 reads it).
    pub cur_hour_14: f32,
    pub cur_packed_date_18: i32,
    /// +0x1C (vt+0x390 copy).
    pub byte_1c: u8,
    /// +0x1D (vt+0xC0 copy).
    pub byte_1d: u8,
    /// +0x1E (vt+0x148 copy).
    pub byte_1e: u8,
    pub byte_1f: u8,
    pub byte_20: u8,
    pub used_item_24: u32,
    pub actor_update_delay_seconds_28: f32,
    /// +0x2C (00643890 vt+0xD0: the follow target).
    pub field_2c: Option<Entity>,
    /// +0x30 (00649190: the package target reference).
    pub field_30: Option<Entity>,
    /// +0x34 (owned object, null here).
    pub pathing_34: u32,
    pub field_38: u32,
    /// +0x3C/+0x40 (list head).
    pub list_3c: [u32; 2],
    /// +0x44 (FormHeap block, moved by 00643520).
    pub block_44: u32,
    pub block_48: u32,
    pub list_4c: [u32; 2],
    pub list_54: [u32; 2],
    pub list_5c: [u32; 2],
    /// +0x64 (owned object, null here).
    pub field_64: u32,
    pub field_68: u32,
    pub field_6c: u32,
    /// +0x70..+0x83.
    pub damage_modifiers_70: ActorValues,
    pub byte_84: u8,
    /// +0x88: never stored by LowProcess_ctor.
    pub unset_88: u32,
    pub float_8c: f32,
}

/// IDA: 00658690 MiddleLowProcess_ctor (0xA8): Low, vtable A72D64, ActorValues at +0x94.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct MiddleLowProcess {
    /// +0x90 the game hour (`__ftol2`) of the last package evaluation: vt+0x18 006588C0 MiddleLowProcess_EvaluatePackage
    /// skips an unforced one with a package in the same hour and stores it after 00649340. MiddleLowProcess_ctor leaves it
    /// unset (host 0); HighProcess_ctor stores -1 (00628F44).
    pub evaluated_hour_90: i32,
    /// +0x94..+0xA7.
    pub max_modifiers_94: ActorValues,
}

/// IDA: the 0x0C InventoryEntry a process holds for an equipped item (00484420 by 0048A0BC): +0x00 its stacks (the worn one, from 00489C30), +0x04 the count, +0x08 the form. `worn` is InventoryEntry_IsWorn(entry, 0): its first stack holds 0x1B or 0x1C.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EquippedEntry {
    pub form_08: Entity,
    pub count_04: i32,
    pub worn: bool,
}

/// IDA: one 0x10 detection state of a High process's +0x18C list, as 00631BE0 HighProcess_SetDetected writes it: +0x00 the
/// target, +0x04 the state (0, or 3 for a positive level), +0x08 seen (the line of sight), +0x0C the level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetectionState {
    pub target_00: Entity,
    pub state_04: u32,
    pub seen_08: u8,
    pub level_0c: i32,
}

/// IDA: the entries of a High process's +0x18C detection list in list order (00631BE0 prepends a new one; 00631C50
/// HighProcess_GetDetectionState, vt+0x3B0, answers the first whose target matches). `HighProcess::detection_list_18c`
/// keeps the list head's own words.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct DetectionList(pub Vec<DetectionState>);

/// IDA: MiddleHighProcess +0xFC..+0x110 as 006541A0 stores them: each node by name under the actor's animation node (the anim
/// data's +0x04, else the reference's 3D; 006F94A0: a NiControllerManager's object palette first, else GetObjectByName). The
/// player's own (with the first-person data, 0065D750) go to 00B3BA84..00B3BA98 instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponNodes {
    /// +0xFC "Weapon".
    pub weapon_fc: Option<Entity>,
    /// +0x100 "Torch".
    pub torch_100: Option<Entity>,
    /// +0x104 "Bip01 L ForearmTwist".
    pub forearm_twist_104: Option<Entity>,
    /// +0x108 the sheathed weapon's node: "BackWeapon" for weapon types 1 and 3..5, else "SideWeapon" (00B06550[4 or 5]).
    pub sheath_108: Option<Entity>,
    /// +0x10C "Quiver".
    pub quiver_10c: Option<Entity>,
    /// +0x110 "ArrowBone" (vt+0x114 0064F5E0 with animation data and a weapon; 006541A0 and 00658550 zero it).
    pub arrow_bone_110: Option<Entity>,
}

/// IDA: 0064B400 MiddleHighProcess_ctor (0x18C).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct MiddleHighProcess {
    pub fields_a8: [u32; 4],
    pub float_b8: f32,
    pub torch_evaluation_timer_bc: f32,
    pub current_package_c0: Option<Entity>,
    pub float_c4: f32,
    /// +0xC8 (1).
    pub byte_c8: u8,
    pub procedure_type_cc: u32,
    /// +0xD0 (HighProcess stores 1).
    pub byte_d0: u8,
    pub field_e0: u32,
    /// +0xE4 the equipped weapon's entry (vt+0xFC / vt+0x104 00658550).
    pub weapon_e4: Option<EquippedEntry>,
    /// +0xE8 the equipped light's (vt+0xF0 0064B270 / vt+0x108 0064AE90).
    pub light_e8: Option<EquippedEntry>,
    /// +0xEC the equipped ammo's (vt+0xF4 0064B2A0 / vt+0x10C 0064AED0).
    pub ammo_ec: Option<EquippedEntry>,
    /// +0xF0 the equipped shield's (vt+0xF8 0064B2D0 / vt+0x110 0064AF10).
    pub shield_f0: Option<EquippedEntry>,
    /// +0xF4 a bow is the weapon (00658550; vt+0x138 006293C0).
    pub bow_f4: u8,
    /// +0xF5 a staff is the weapon (00658550; vt+0x13C 006293D0).
    pub staff_f5: u8,
    /// +0xF8 the last best-weapon search's rating (-1.0 from the ctor): process vt+0x328 00629620
    /// MiddleHighProcess_SetWeaponRating stores it after 0060E7C0's 0048BDA0, vt+0x324 00629610 reads it.
    pub weapon_rating_f8: f32,
    /// +0xFC..+0x110 the weapon's bone nodes (006541A0; zeroed by 00658550 with no animation data).
    pub weapon_nodes_fc: WeaponNodes,
    pub weapon_draw_requested_114: u8,
    pub weapon_drawn_115: u8,
    /// +0x118 NiPointer<bhkCharacterController> (vt+0x18C 006348E0 / vt+0x190 00634910); `tes::character_controller` keeps its +0x04 count.
    pub char_controller_118: Option<Entity>,
    /// +0x11C (vt+0x2E4 0064B080 reads it).
    pub knocked_state_11c: u8,
    /// +0x11D (vt+0x36C 0064B090 reads it).
    pub sleep_state_11d: u8,
    pub furniture_reference_120: Option<Entity>,
    /// +0x124 (0x7F).
    pub furniture_marker_index_124: u8,
    /// +0x128..+0x130 (g_niPoint3Zero dwords).
    pub furniture_marker_position_128: [u32; 3],
    /// +0x134 (006FAEE0).
    pub word_134: u16,
    pub byte_136: u8,
    /// +0x138 (0xFFFF).
    pub word_138: u16,
    pub field_13c: u32,
    pub bound_weapon_140: Option<Entity>,
    /// +0x144 the current cast item: vt+0x2A8 (00629360) reads it, vt+0x2AC (00629370) stores it (the actor's MagicCaster vt+0x30/0x34).
    pub current_magic_item_144: u32,
    pub field_148: u32,
    pub cast_animation_update_flag_14c: u8,
    pub field_150: u32,
    /// +0x154 (1.0).
    pub actor_alpha_154: f32,
    pub float_158: f32,
    pub face_animation_data_15c: Option<Entity>,
    pub byte_160: u8,
    pub byte_161: u8,
    pub field_164: u32,
    pub bytes_168: [u8; 4],
    /// +0x16C an equipment 3D update pending: vt+0x31C (006295E0) stores it, from an equip (0048A113) and around 005268D0; 0063CDC0 rebuilds the actor's equipment 3D while it's set and the actor has a node, then clears it.
    pub equipment_update_pending_16c: u8,
    pub full_update_pending_16d: u8,
    pub field_170: u32,
    /// +0x174 effect list head block (FormHeap 8 {0, 0}).
    pub effect_list_174: Option<[u32; 2]>,
    pub field_178: u32,
    /// +0x17C the ActorAnimData (0064FFF0 sets it; `actor_animation`).
    pub anim_data_17c: Option<Entity>,
    pub byte_180: u8,
    /// +0x184 the skin's shade property (NiPointer, vt+0x3A8 006349D0: 005EA1A0's find under the skin attachment node).
    pub skin_property_184: Option<Entity>,
    /// +0x188 the 3D's BSBound (its "BBX" extra), stored by vt+0x470 (`00629590`) from Character_Load3D (`00659F30`).
    pub bound_188: Option<Entity>,
}

/// IDA: 00628EE0 HighProcess_ctor (0x2EC).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct HighProcess {
    /// +0x18C detection list head block (FormHeap 8 {0, 0}).
    pub detection_list_18c: Option<[u32; 2]>,
    pub field_190: u32,
    pub field_194: u32,
    pub float_198: f32,
    /// +0x19C (GMST fIdleChatterCommentTimer).
    pub float_19c: f32,
    pub byte_1a0: u8,
    pub field_1a4: u32,
    pub float_1a8: f32,
    pub package_evaluation_timer_1ac: f32,
    pub float_1b0: f32,
    pub float_1b4: f32,
    pub float_1b8: f32,
    pub fields_1bc: [u32; 4],
    pub field_1cc: u32,
    pub byte_1d0: u8,
    pub skipped_high_update_beyond_fade_distance_1d1: u8,
    pub field_1d4: u32,
    pub float_1d8: f32,
    pub float_1dc: f32,
    pub movement_flags_timer_1e0: f32,
    pub timed_movement_flags_1e4: u8,
    pub float_1e8: f32,
    pub bone_lod_controller_1ec: Option<Entity>,
    pub last_bone_lod_1f0: i32,
    /// +0x1F4 (0xFFFF).
    pub current_action_1f4: u16,
    pub field_1f8: u32,
    pub movement_flags_1fc: u16,
    pub field_200: u32,
    pub detection_timer_204: f32,
    /// +0x208 (0xFFFF).
    pub word_208: u16,
    /// +0x20C..+0x214 (g_niPoint3Zero dwords).
    pub point_20c: [u32; 3],
    pub field_218: u32,
    pub float_21c: f32,
    pub fields_220: [u32; 2],
    pub dialogue_state_byte_228: u8,
    pub death_state_timer_22c: f32,
    /// +0x230 (Random_Next % 5000 * 0.001 + 10).
    pub idle_animation_timer_230: f32,
    pub float_234: f32,
    /// +0x238 (20.0).
    pub swim_breath_238: f32,
    /// +0x23C (1).
    pub byte_23c: u8,
    pub float_240: f32,
    pub byte_244: u8,
    pub float_248: f32,
    pub dialogue_movement_flag_24c: u8,
    pub field_250: u32,
    pub unset_254: u32,
    pub field_258: u32,
    pub byte_25c: u8,
    pub byte_25d: u8,
    pub float_260: f32,
    pub float_264: f32,
    /// +0x268 (NiPointer, null).
    pub field_268: u32,
    /// +0x26C (1.0).
    pub float_26c: f32,
    pub float_270: f32,
    pub field_274: u32,
    pub byte_278: u8,
    /// +0x27C..+0x284 (g_niPoint3Zero dwords).
    pub point_27c: [u32; 3],
    pub wading_water_288: u32,
    pub float_28c: f32,
    pub byte_290: u8,
    /// +0x294 (-1.0).
    pub cached_encumbrance_294: f32,
    /// +0x298 caches the integer result for actor value 48. A negative value requests a fresh read.
    pub cached_paralysis_298: i32,
    pub try_restore_with_spell_29c: i32,
    pub try_restore_with_item_2a0: i32,
    pub field_2a4: u32,
    pub byte_2a8: u8,
    pub byte_2a9: u8,
    pub float_2ac: f32,
    pub float_2b0: f32,
    pub field_2b4: u32,
    pub byte_2b8: u8,
    pub byte_2b9: u8,
    /// +0x2BC (1).
    pub field_2bc: u32,
    pub float_2c0: f32,
    pub field_2c4: u32,
    pub action_targets_2c8: [u32; 5],
    pub action_active_2dc: [u8; 5],
    pub field_2e4: u32,
    pub byte_2e8: u8,
}

/// IDA: N/A a node of a BSSimpleList<Actor*>: the arena slot stands in for the node pointer, and slot 0 is the embedded head, so the tail (+0x08) and the cursors can point at a node.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActorListNode(usize);

impl ActorListNode {
    /// IDA: N/A the embedded head.
    pub const HEAD: Self = Self(0);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ActorNodeSlot {
    item: Option<Entity>,
    next: Option<usize>,
}

/// IDA: BSSimpleList<Actor*> with an embedded head node (+0x00 item, +0x04 next). The tail and cursors hold nodes, which `oblivion_core::pointer_list::PointerList` does not expose. Completion: 100%.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActorList {
    nodes: Vec<Option<ActorNodeSlot>>,
}

impl Default for ActorList {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            nodes: vec![Some(ActorNodeSlot::default())],
        }
    }
}

impl ActorList {
    // IDA: N/A an accessor.
    fn slot(&self, node: ActorListNode) -> &ActorNodeSlot {
        self.nodes[node.0].as_ref().expect("a live node")
    }

    // IDA: N/A a helper, with no address of its own.
    fn slot_mut(&mut self, node: ActorListNode) -> &mut ActorNodeSlot {
        self.nodes[node.0].as_mut().expect("a live node")
    }

    // IDA: N/A a helper, with no address of its own.
    fn allocate(&mut self, slot: ActorNodeSlot) -> usize {
        self.nodes.push(Some(slot));
        self.nodes.len() - 1
    }

    /// IDA: N/A a node's +0x00.
    #[must_use]
    pub fn item(&self, node: ActorListNode) -> Option<Entity> {
        self.slot(node).item
    }

    /// IDA: N/A a node's +0x04.
    #[must_use]
    pub fn next(&self, node: ActorListNode) -> Option<ActorListNode> {
        self.slot(node).next.map(ActorListNode)
    }

    /// IDA: N/A the items in list order, without null nodes.
    #[must_use]
    pub fn items(&self) -> Vec<Entity> {
        self.ordered_items().collect()
    }

    /// IDA: N/A follow the embedded head and each next link.
    pub(super) fn ordered_items(&self) -> impl Iterator<Item = Entity> {
        std::iter::successors(Some(ActorListNode::HEAD), |&node| self.next(node))
            .filter_map(|node| self.item(node))
    }

    /// IDA: 00446CB0 PointerList_PrependNonNull(item) on the head: an empty head takes the item; otherwise a new node takes the head's item and next and the head takes the item with next = the new node.
    pub fn prepend_non_null(&mut self, item: Entity) {
        let head = *self.slot(ActorListNode::HEAD);
        if head.item.is_none() {
            self.slot_mut(ActorListNode::HEAD).item = Some(item);
            return;
        }
        let node = self.allocate(head);
        let head = self.slot_mut(ActorListNode::HEAD);
        head.item = Some(item);
        head.next = Some(node);
    }

    /// IDA: 0067B1E0 PointerList_AppendNonNull(node, item): walk to the last node; an empty last node takes the item, otherwise a new node is appended.
    pub fn append_non_null(&mut self, from: ActorListNode, item: Entity) {
        let mut last = from;
        while let Some(next) = self.next(last) {
            last = next;
        }
        if self.item(last).is_none() {
            self.slot_mut(last).item = Some(item);
        } else {
            let node = self.allocate(ActorNodeSlot {
                item: Some(item),
                next: None,
            });
            self.slot_mut(last).next = Some(node);
        }
    }

    /// IDA: 0065C620 PointerList_RemoveFirstMatchingItem(node, item), from `from`: a match at `from` pulls the next node's item/next in (freeing that node) or clears the item; a later match is unlinked and freed.
    pub fn remove_first_matching(&mut self, from: ActorListNode, item: Entity) {
        let first = *self.slot(from);
        if first.next.is_none() && first.item.is_none() {
            return;
        }
        let mut previous = from;
        let mut current = from;
        while self.item(current) != Some(item) {
            previous = current;
            let Some(next) = self.next(current) else {
                return;
            };
            current = next;
        }
        if current == from {
            self.pull_next_in(from);
        } else {
            self.slot_mut(previous).next = self.slot(current).next;
            self.nodes[current.0] = None;
        }
    }

    // IDA: the head removal both 0065C620 and 0067B3D8 write: pull the next node in (FormHeap_Free) or clear the item.
    fn pull_next_in(&mut self, node: ActorListNode) {
        if let Some(next) = self.next(node) {
            let pulled = *self.slot(next);
            *self.slot_mut(node) = pulled;
            self.nodes[next.0] = None;
        } else {
            self.slot_mut(node).item = None;
        }
    }
}

/// IDA: one ActorProcessManager actor list (0x0C): the BSSimpleList<Actor*> head (+0x00 item, +0x04 next) and +0x08 the tail node (0067B240 points it at the head).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActorProcessList {
    pub list: ActorList,
    pub tail_08: ActorListNode,
}

/// IDA: N/A actor membership in the high list at manager +0x68.
#[derive(Component)]
pub struct ProcessLevelHigh;

/// IDA: N/A actor membership in the middle-high list at manager +0x00.
#[derive(Component)]
pub struct ProcessLevelMiddleHigh;

/// IDA: N/A actor membership in the middle-low list at manager +0x0C.
#[derive(Component)]
pub struct ProcessLevelMiddleLow;

/// IDA: N/A actor membership in the low list at manager +0x18.
#[derive(Component)]
pub struct ProcessLevelLow;

/// IDA: N/A which manager list a cursor's node belongs to, and the 00673A90 / 00674550 level switch (0 High +0x68, 1 +0x00, 2 +0x0C, 3 +0x18).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActorListId {
    MiddleHigh00,
    MiddleLow0c,
    Low18,
    High68,
}

impl ActorListId {
    /// IDA: N/A keep the actor index at the list insertion site.
    fn mark(self, world: &mut World, actor: Entity) {
        let mut entity = world.entity_mut(actor);
        match self {
            Self::High68 => entity.insert(ProcessLevelHigh),
            Self::MiddleHigh00 => entity.insert(ProcessLevelMiddleHigh),
            Self::MiddleLow0c => entity.insert(ProcessLevelMiddleLow),
            Self::Low18 => entity.insert(ProcessLevelLow),
        };
    }

    /// IDA: N/A drop the marker when the last occurrence leaves this list.
    fn unmark(self, world: &mut World, actor: Entity) {
        let mut entity = world.entity_mut(actor);
        match self {
            Self::High68 => entity.remove::<ProcessLevelHigh>(),
            Self::MiddleHigh00 => entity.remove::<ProcessLevelMiddleHigh>(),
            Self::MiddleLow0c => entity.remove::<ProcessLevelMiddleLow>(),
            Self::Low18 => entity.remove::<ProcessLevelLow>(),
        };
    }

    /// IDA: N/A the 00673A90 / 00674550 switch: other levels have no list.
    #[must_use]
    pub const fn for_level(level: i32) -> Option<Self> {
        match level {
            0 => Some(Self::High68),
            1 => Some(Self::MiddleHigh00),
            2 => Some(Self::MiddleLow0c),
            3 => Some(Self::Low18),
            _ => None,
        }
    }
}

/// IDA: a list cursor (00B3BD74 / 00B3BD78): a node pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActorListCursor {
    pub list: ActorListId,
    pub node: ActorListNode,
}

/// IDA: 0067AF60 ActorProcessManager_Constructor (00B3BD00, 0xB0; xOBSE GameProcess.h ActorProcessManager). Lists by process level; fields the constructor does not store stay zero (BSS).
#[derive(Debug)]
pub struct ActorProcessManager {
    /// Level 1.
    pub middle_high_00: ActorProcessList,
    /// Level 2.
    pub middle_low_0c: ActorProcessList,
    /// Level 3.
    pub low_18: ActorProcessList,
    /// +0x24, read and advanced before the three lower-level passes of 00678510.
    pub lower_level_update_accumulator_24: f32,
    pub field_28: u32,
    /// +0x40 NiPointer pair (00532DF0(0)).
    pub pointer_40: [u32; 2],
    pub pointer_48: [u32; 2],
    /// +0x50..+0x64.
    pub fields_50: [u32; 6],
    /// IDA: +0x58 the list 006748B0 prepends to, held apart from `fields_50`, which keeps the constructor's zeros for the Phase 11 image.
    pub queued_58: PointerList<Entity>,
    /// IDA: +0x60 the High actors near the player (00677A90 ActorProcessManager_CollectHighActorsNearPlayer prepends those
    /// within 00B36CD8 of it; 00674850 removes one). Held apart from `fields_50` as `queued_58` is. Nothing the port runs fills it.
    pub near_player_60: PointerList<Entity>,
    /// Level 0.
    pub high_68: ActorProcessList,
    /// 00B3BD74.
    pub cursor_74: Option<ActorListCursor>,
    /// 00B3BD78 (BSS).
    pub cursor_78: Option<ActorListCursor>,
    /// +0x7C..+0x97: 00677DD0 LockFreeMap<Actor*, NiPointer<LipTask>>(2, 37, 12) (the Phase 09 family).
    pub lip_tasks_7c: LockFreeMap,
    pub byte_98: u8,
    pub byte_99: u8,
    pub byte_9a: u8,
    pub field_9c: u32,
    /// The four process toggles checked by 00678510, in the static's byte order.
    pub high_processing_enabled_a0: u8,
    pub low_processing_enabled_a1: u8,
    pub middle_high_processing_enabled_a2: u8,
    pub middle_low_processing_enabled_a3: u8,
    /// Cmd_ToggleAiSchedules_Execute and Actor_Update.
    pub editor_schedules_enabled_a4: u8,
    /// Cmd_ShowDialogSubtitles_Execute.
    pub show_dialog_subtitles_a5: u8,
    /// Cleared in the constructor. Its runtime role is unresolved.
    pub flag_a6: u8,
    /// 00677F05 stores the non-null High actor count before visiting the list.
    pub high_actor_count_a8: u32,
    /// +0xAC GMST fRemoveExcessComplexDeadTime.
    pub remove_excess_dead_time_ac: f32,
}

impl ActorProcessManager {
    /// IDA: N/A a list by id.
    #[must_use]
    pub const fn list(&self, id: ActorListId) -> &ActorProcessList {
        match id {
            ActorListId::MiddleHigh00 => &self.middle_high_00,
            ActorListId::MiddleLow0c => &self.middle_low_0c,
            ActorListId::Low18 => &self.low_18,
            ActorListId::High68 => &self.high_68,
        }
    }

    /// IDA: N/A a list by id.
    pub const fn list_mut(&mut self, id: ActorListId) -> &mut ActorProcessList {
        match id {
            ActorListId::MiddleHigh00 => &mut self.middle_high_00,
            ActorListId::MiddleLow0c => &mut self.middle_low_0c,
            ActorListId::Low18 => &mut self.low_18,
            ActorListId::High68 => &mut self.high_68,
        }
    }

    // IDA: N/A a cursor's node item (a null cursor has none).
    fn cursor_item(&self, cursor: Option<ActorListCursor>) -> Option<Entity> {
        cursor.and_then(|cursor| self.list(cursor.list).list.item(cursor.node))
    }
}

/// IDA: the process-side globals Phase 11 writes or reads.
#[derive(Resource, Debug)]
pub struct ActorProcessGlobals {
    /// 00B3BD00.
    pub manager: ActorProcessManager,
    /// 00B3B928 (00628F4A clears it).
    pub high_process_b3b928: u32,
    /// 0040DC1A resets the pick count before each normal manager update.
    pub detection_picks_this_frame_b333bc: i32,
    /// 00677ECD clears the per-frame destination gate used by HighProcess_UpdateWanderProcedure.
    pub wander_destination_claimed_this_frame_b3b935: u8,
    /// 00B3BCF0, the process time 00673B00 reads (BSS, 0.0).
    /// Updated at 00678556 on a manager frame.
    pub time_seconds_b3bcf0: f32,
    /// 00B3F9A0 two pi (00A09BD0) read by 006FAEE0.
    pub two_pi_b3f9a0: f32,
    /// 00B069C3 Random_Next's first-use flag, and `oblivion_core::crt` holds its logic.
    pub random: RandomGlobals,
}

// IDA: 0067AF60 on the static (cinit 009FE5F0, then atexit 00A25DA0). Reads the GMST fRemoveExcessComplexDeadTime (00B37D58) from `GameSettingCollection`, its image value 2.5 when the collection is absent.
// Completion: 100% of the order.
pub fn construct_actor_process_manager(world: &mut World) {
    // 0067B009..0067B064 +0xAC = GMST.
    let remove_excess_dead_time_ac = world
        .get_resource::<GameSettingCollection>()
        .and_then(|settings| settings.find("fRemoveExcessComplexDeadTime"))
        .map_or(f32::from_bits(0x4020_0000), |setting| {
            f32::from_bits(setting.raw)
        });
    // Reinstalling the static in a reused host world also replaces its membership index.
    if let Some(old) = world.remove_resource::<ActorProcessGlobals>() {
        for id in [
            ActorListId::High68,
            ActorListId::MiddleHigh00,
            ActorListId::MiddleLow0c,
            ActorListId::Low18,
        ] {
            for actor in old.manager.list(id).list.ordered_items() {
                if world.get_entity(actor).is_ok() {
                    id.unmark(world, actor);
                }
            }
        }
    }
    world.insert_resource(ActorProcessGlobals {
        manager: ActorProcessManager {
            // 0067AF8B/0067AF99/0067AFA6 0067B240 on +0x00, +0x0C, +0x18: {0, 0, self}.
            middle_high_00: ActorProcessList::default(),
            middle_low_0c: ActorProcessList::default(),
            low_18: ActorProcessList::default(),
            lower_level_update_accumulator_24: 0.0,
            field_28: 0,
            // 0067AFAB..0067AFD2 00532DF0(0) on +0x40 and +0x48: {0, 0}.
            pointer_40: [0; 2],
            pointer_48: [0; 2],
            // 0067AFD7..0067AFEE +0x50..+0x64 = 0.
            fields_50: [0; 6],
            queued_58: PointerList::default(),
            near_player_60: PointerList::default(),
            // 0067AFF1 0067B240 on +0x68.
            high_68: ActorProcessList::default(),
            cursor_74: None,
            cursor_78: None,
            // 0067B004 00677DD0(2, 37, 12) on +0x7C: the LockFreeMap stores (vtable, +0x18, +0x08 = 37, the cleared buckets, +0x04 storage for 2 entries, +0x10 = 12, LockFreeTlsLock(2)).
            lip_tasks_7c: LockFreeMap::new(LOCK_FREE_MAP_ACTOR_LIP_TASK_VTABLE, 2, 37, 12),
            // 0067B009..0067B064 the flags, +0x74 = 0, +0x28 = 0, +0xAC = GMST and +0xA8 = 0; +0xA5/+0xA6 are stored twice.
            byte_98: 1,
            byte_99: 1,
            byte_9a: 0,
            field_9c: 0,
            high_processing_enabled_a0: 1,
            low_processing_enabled_a1: 1,
            middle_high_processing_enabled_a2: 1,
            middle_low_processing_enabled_a3: 1,
            editor_schedules_enabled_a4: 1,
            show_dialog_subtitles_a5: 0,
            flag_a6: 0,
            high_actor_count_a8: 0,
            remove_excess_dead_time_ac,
        },
        high_process_b3b928: 0,
        detection_picks_this_frame_b333bc: 0,
        wander_destination_claimed_this_frame_b3b935: 0,
        time_seconds_b3bcf0: 0.0,
        two_pi_b3f9a0: f32::from_bits(0x40C9_0FDB),
        random: RandomGlobals::default(),
    });
}

use crate::bs::character_controller::BhkCharacterController;
use crate::tes::actor::process_of;
use crate::tes::actor_package::package_arrival;
use crate::tes::actor_values::base_actor_value;
use crate::tes::ai_conditions::evaluate_conditions;
use crate::tes::ai_package::{
    location_reference, package_at_location, package_data, package_init_procedure,
    package_location_cell, package_location_position, package_location_world, procedure_at,
    script_event_or, target_object, target_object_type, target_reference,
};
use crate::tes::ai_process::{
    AiArms, base_of, low_mut, package_apply_target, package_target_is_actor, procedure_count,
};
use crate::tes::base_extra_list::run_once_add;
use crate::tes::data_handler_world::doors_near;
use crate::tes::form_components::{collect_scheduled_packages, select_from_list};
use crate::tes::forms_items::clear_change_flags;
use crate::tes::forms_references::{
    activate_reference, actor_package_list, form_flags, move_to_cell, record_start_location,
    reference_encumbrance, reference_world, set_reference_position, set_rotation_x,
};
use crate::tes::global::{game_day, game_day_byte, game_hour, game_month, game_year};
use crate::tes::player::the_player;
use crate::tes::save_game::{ProcessView, SaveWriter, empty_list, form_id, null_id};
pub use oblivion_core::crt::RandomSource;

/// IDA: N/A the main thread's CRT as the host installs it, for the callers that reach `_srand`, `_rand` and `_time64`
/// themselves rather than through 0047DF80 (FaceGen's 006F6A10).
#[derive(Clone)]
pub struct MainThreadRandom(pub std::rc::Rc<std::cell::RefCell<dyn RandomSource>>);

/// IDA: N/A the default `RandomSource`: the `oblivion_core::crt` LCG over a host clock.
pub struct CrtRandomSource<'a> {
    pub rand: MsvcRand,
    pub time64: &'a mut dyn FnMut() -> i64,
}

impl RandomSource for CrtRandomSource<'_> {
    /// IDA: N/A `CrtRandomSource`'s side of `RandomSource`.
    fn time64(&mut self) -> i64 {
        (self.time64)()
    }

    /// IDA: N/A `CrtRandomSource`'s side of `RandomSource`.
    fn srand(&mut self, seed: u32) {
        self.rand.srand(seed);
    }

    /// IDA: N/A `CrtRandomSource`'s side of `RandomSource`.
    fn rand(&mut self) -> i32 {
        self.rand.rand()
    }
}

/// IDA: N/A what the process code reads outside itself.
pub struct ProcessInputs<'a> {
    /// 0047DF80's CRT.
    pub random: &'a mut dyn RandomSource,
    /// IDA: 00B36940 GMST fIdleChatterCommentTimer (HighProcess +0x19C), raw bits.
    pub idle_chatter_comment_timer: u32,
    /// IDA: N/A test seam for the unset LowProcess +0x88 (FormHeap garbage in the original).
    pub low_process_unset_88: u32,
    /// IDA: 0045A500 TESSaveLoadGame_TestCurrentThreadActiveFlag(g_saveLoadGame) (0067B411).
    pub save_load_active: bool,
}

// IDA: 006438F0 LowProcess_ctor on an allocated block, without the vtable. Completion: 100% of the store order.
#[expect(
    clippy::field_reassign_with_default,
    reason = "006438F0's stores stay one statement each, in original store order"
)]
fn build_low_process(unset_88: u32) -> (BaseProcess, LowProcess) {
    // 00643919 BaseProcess_ctor: vtable A6F464. 00643920 vtable A720A4.
    let mut base = BaseProcess::default();
    let mut p = LowProcess::default();
    // 00643926..00643942 +0x3C, +0x40, +0x4C, +0x50, +0x54, +0x58, +0x5C, +0x60 = 0.
    p.list_3c = [0; 2];
    p.list_4c = [0; 2];
    p.list_54 = [0; 2];
    p.list_5c = [0; 2];
    // 00643945 0065BE10 on +0x70.
    p.damage_modifiers_70 = construct_actor_values();
    // 0064394A..0064399D in store order.
    p.cur_hour_14 = -1.0;
    base.editor_package_08 = None;
    p.field_0c = -1.0;
    p.pathing_34 = 0;
    base.edit_pack_procedure_type_04 = 0;
    p.field_10 = 0.0;
    p.field_2c = None;
    p.actor_update_delay_seconds_28 = 0.0;
    p.cur_packed_date_18 = 0;
    p.float_8c = 0.0;
    p.byte_1c = 0;
    p.byte_1d = 0;
    p.block_44 = 0;
    p.block_48 = 0;
    p.field_30 = None;
    p.byte_84 = 0;
    p.byte_1f = 0;
    p.byte_20 = 0;
    p.field_64 = 0;
    p.field_68 = 0;
    p.field_6c = 0;
    p.used_item_24 = 0;
    p.field_38 = 0;
    p.byte_1e = 0;
    // +0x88 is not stored: FormHeap bytes (host zero unless a test injects them).
    p.unset_88 = unset_88;
    (base, p)
}

// IDA: 006FAEE0 (on MiddleHigh +0x128): wrap the angle into [0, 00B3F9A0) by whole periods (float stores), then +0x0C (MiddleHigh +0x134) = (short)(angle * 1000.0) truncated (fistp with RC = chop).
// Completion: 100%.
fn wrapped_angle_word(angle: f32, period: f32) -> u16 {
    let mut value = angle;
    while value < 0.0 {
        value += period;
    }
    // 006FAF19 `test ah,41h; jnp` and 006FAF3C `test ah,1; je`: an unordered compare leaves both ways, so NaN doesn't loop (the C++ does).
    while value >= period {
        value -= period;
    }
    x87::ftol(value * 1000.0) as u16
}

// IDA: 0064B400 MiddleHighProcess_ctor, over the Low and MiddleLow state. Completion: 100% of the store order.
#[expect(
    clippy::field_reassign_with_default,
    reason = "0064B400's stores stay one statement each, in original store order"
)]
fn build_middle_high(low: &mut LowProcess, two_pi: f32) -> MiddleHighProcess {
    // 0064B431 vtable A72684; 0064B437..0064B449 +0xA8..+0xB4 = 0.
    let mut p = MiddleHighProcess::default();
    p.fields_a8 = [0; 4];
    // 0064B453 +0x118 = 0 (NiPointer).
    p.char_controller_118 = None;
    // 0064B459..0064B467 +0x134 word = 0, +0x136 = 0xFF, +0x184 = 0.
    p.word_134 = 0;
    p.byte_136 = 0xFF;
    p.skin_property_184 = None;
    // 0064B46D..0064B4E3 in store order.
    p.weapon_rating_f8 = -1.0;
    p.current_magic_item_144 = 0;
    p.field_178 = 0;
    p.weapon_e4 = None;
    p.light_e8 = None;
    p.ammo_ec = None;
    p.shield_f0 = None;
    p.weapon_nodes_fc = WeaponNodes::default();
    p.bow_f4 = 0;
    p.staff_f5 = 0;
    p.word_138 = 0xFFFF;
    p.field_13c = 0;
    p.bound_weapon_140 = None;
    // 0064B4E9..0064B506 FormHeap(8) {0, 0} -> +0x174.
    p.effect_list_174 = Some([0; 2]);
    // 0064B50C..0064B542.
    low.pathing_34 = 0;
    p.current_package_c0 = None;
    p.procedure_type_cc = 0;
    p.anim_data_17c = None;
    p.weapon_draw_requested_114 = 0;
    p.weapon_drawn_115 = 0;
    p.knocked_state_11c = 0;
    low.field_38 = 0;
    p.sleep_state_11d = 0;
    p.furniture_reference_120 = None;
    p.furniture_marker_index_124 = 0x7F;
    // 0064B549 006FAEE0(0.0) on +0x128 (writes +0x134).
    p.word_134 = wrapped_angle_word(0.0, two_pi);
    // 0064B550..0064B575 +0x136 = 0, +0x128..+0x130 = g_niPoint3Zero, +0xB8 = 0.0, +0x180 = 0.
    p.byte_136 = 0;
    p.furniture_marker_position_128 = [0; 3];
    p.float_b8 = 0.0;
    p.byte_180 = 0;
    // 0064B57B..0064B5A5 release +0x184 (null: skipped).
    // 0064B5AB..0064B62C in store order.
    p.torch_evaluation_timer_bc = 0.0;
    p.field_e0 = 0;
    p.float_c4 = 0.0;
    p.field_148 = 0;
    p.cast_animation_update_flag_14c = 0;
    p.actor_alpha_154 = 1.0;
    p.field_150 = 0;
    p.face_animation_data_15c = None;
    p.field_164 = 0;
    p.float_158 = 0.0;
    p.byte_161 = 0;
    // 0064B5EF fstp [esi+88h].
    low.unset_88 = 0.0_f32.to_bits();
    p.byte_c8 = 1;
    p.bytes_168[0] = 0;
    p.bytes_168[1] = 0;
    p.byte_160 = 0;
    p.bound_188 = None;
    p.bytes_168[2] = 0;
    p.bytes_168[3] = 0;
    p.equipment_update_pending_16c = 0;
    p.full_update_pending_16d = 0;
    p.field_170 = 0;
    p
}

// IDA: 006438F0 LowProcess_ctor (FormHeap 0x90). Completion: 100% of the store order.
pub fn construct_low_process(world: &mut World, inputs: &ProcessInputs<'_>) -> Entity {
    let (base, low) = build_low_process(inputs.low_process_unset_88);
    world
        .spawn((OriginalVtable(LOW_PROCESS_VTABLE), base, low))
        .id()
}

// IDA: 006438F0 LowProcess_ctor (FormHeap 0x90) for a caller whose 0060CDB0 copies an old process's +0x88 over the unset bytes,
// so no test seam for them. Completion: 100% of the store order.
pub fn construct_low_process_for_copy(world: &mut World) -> Entity {
    let (base, low) = build_low_process(0);
    world
        .spawn((OriginalVtable(LOW_PROCESS_VTABLE), base, low))
        .id()
}

// IDA: 00658690 MiddleLowProcess_ctor (FormHeap 0xA8): 006438F0 LowProcess_ctor, vtable A72D64, 0065BE10 ActorValues_ctor on +0x94.
// +0x88 and +0x90 are never stored: FormHeap bytes, host zero (the caller's 0060CDB0 copies the old process's +0x88 over it).
// Completion: 100% of the store order.
pub fn construct_middle_low_process(world: &mut World) -> Entity {
    let (base, low) = build_low_process(0);
    let middle_low = MiddleLowProcess {
        evaluated_hour_90: 0,
        max_modifiers_94: construct_actor_values(),
    };
    world
        .spawn((
            OriginalVtable(MIDDLE_LOW_PROCESS_VTABLE),
            base,
            low,
            middle_low,
        ))
        .id()
}

// IDA: 00628EE0 HighProcess_ctor (FormHeap 0x2EC; 00658690 and 0064B400 inside). x87: 24-bit, plain f32. Completion: 100% of the store order.
#[expect(
    clippy::field_reassign_with_default,
    reason = "00628EE0's stores stay one statement each, in original store order"
)]
pub fn construct_high_process(world: &mut World, inputs: &mut ProcessInputs<'_>) -> Entity {
    let (base, mut low) = build_low_process(inputs.low_process_unset_88);
    // 00658690 MiddleLowProcess_ctor: vtable A72D64, 0065BE10 on +0x94.
    let mut middle_low = MiddleLowProcess {
        evaluated_hour_90: 0,
        max_modifiers_94: construct_actor_values(),
    };
    let mut globals = world.resource_mut::<ActorProcessGlobals>();
    // 00628F0D MiddleHighProcess_ctor.
    let mut middle_high = build_middle_high(&mut low, globals.two_pi_b3f9a0);
    // 00628F14..00628F4A vtable A71814, +0x190/+0x194/+0x268 = 0, +0xD0 = 1, +0x1D4 = 0, +0x90 = -1, 00B3B928 = 0.
    let mut p = HighProcess::default();
    p.field_190 = 0;
    p.field_194 = 0;
    p.field_268 = 0;
    middle_high.byte_d0 = 1;
    p.field_1d4 = 0;
    middle_low.evaluated_hour_90 = -1;
    globals.high_process_b3b928 = 0;
    // 00628F50..00629019 in store order.
    p.fields_1bc = [0; 4];
    p.float_1d8 = 0.0;
    p.float_1dc = 0.0;
    p.movement_flags_1fc = 0;
    p.package_evaluation_timer_1ac = 0.0;
    p.field_200 = 0;
    p.detection_timer_204 = 0.0;
    p.word_208 = 0xFFFF;
    p.float_198 = 0.0;
    p.field_1a4 = 0;
    p.bone_lod_controller_1ec = None;
    p.last_bone_lod_1f0 = 0;
    p.current_action_1f4 = 0xFFFF;
    p.field_1f8 = 0;
    p.field_218 = 0;
    p.byte_1d0 = 0;
    p.fields_220 = [0; 2];
    p.point_20c[0] = 0;
    p.point_20c[1] = 0;
    p.float_21c = 0.0;
    p.death_state_timer_22c = 0.0;
    p.point_20c[2] = 0;
    p.dialogue_state_byte_228 = 0;
    p.byte_23c = 1;
    p.byte_244 = 0;
    p.field_250 = 0;
    // 0062901F Random_Next(0); 00629024..00629054 the remainder of 5000, then +0x230 = fild(r) * 0.001 + 10.0 at 24-bit precision (dbl_A30E40 is float 0.001 widened).
    let random = random_next(0, &mut globals.random, &mut *inputs.random);
    let remainder = random % 5000;
    p.dialogue_movement_flag_24c = 0;
    p.byte_25c = 0;
    p.field_1cc = 0;
    p.idle_animation_timer_230 = remainder as f32 * 0.001 + 10.0;
    p.float_248 = 0.0;
    p.float_234 = 0.0;
    p.float_1e8 = 0.0;
    p.float_1b4 = 0.0;
    p.float_1b0 = 0.0;
    // 0062907A..00629095 FormHeap(8) {0, 0} -> +0x18C.
    p.detection_list_18c = Some([0; 2]);
    // 0062908F..00629120 in store order.
    p.swim_breath_238 = 20.0;
    p.timed_movement_flags_1e4 = 0;
    p.field_2bc = 1;
    p.float_1a8 = 0.0;
    p.field_2c4 = 0;
    p.movement_flags_timer_1e0 = 0.0;
    p.byte_25d = 0;
    p.float_240 = 0.0;
    p.float_2c0 = 0.0;
    p.float_1b8 = 0.0;
    p.point_27c = [0; 3];
    p.float_260 = 0.0;
    // 00629106 [00B36940].
    p.float_19c = x87::float_copy(inputs.idle_chatter_comment_timer);
    p.wading_water_288 = 0;
    p.float_264 = 0.0;
    p.float_26c = 1.0;
    p.float_270 = 0.0;
    // 0062912C..00629156 release +0x268 (null: skipped).
    // 0062915C..006291F6 in store order.
    p.float_28c = 0.0;
    p.field_274 = 0;
    p.byte_278 = 0;
    p.cached_encumbrance_294 = -1.0;
    p.byte_290 = 0;
    p.cached_paralysis_298 = -1;
    p.try_restore_with_spell_29c = -1;
    p.try_restore_with_item_2a0 = -1;
    p.field_2a4 = 0;
    p.byte_2a8 = 0;
    p.byte_2a9 = 0;
    for index in 0..5 {
        p.action_targets_2c8[index] = 0;
        p.action_active_2dc[index] = 0;
    }
    p.float_2ac = 0.0;
    p.field_2e4 = 0;
    p.float_2b0 = 0.0;
    p.byte_2e8 = 0;
    p.byte_1a0 = 0;
    p.field_2b4 = 0;
    p.byte_2b8 = 0;
    p.field_258 = 0;
    p.skipped_high_update_beyond_fade_distance_1d1 = 0;
    p.byte_2b9 = 0;
    world
        .spawn((
            OriginalVtable(HIGH_PROCESS_VTABLE),
            base,
            low,
            middle_low,
            middle_high,
            p,
        ))
        .id()
}

// IDA: vt+8 GetProcessLevel: LowProcess 0073FD50 returns 3, MiddleLowProcess 008CF6B0 (ReturnTwo_SharedVirtual) 2,
// MiddleHighProcess 0091FEA0 (ReturnOne_SharedVirtual) 1, HighProcess 006F7070 0. The port builds no other process class.
// Completion: 100%.
#[must_use]
pub fn process_level(world: &World, process: Entity) -> i32 {
    match world.get::<OriginalVtable>(process) {
        Some(OriginalVtable(HIGH_PROCESS_VTABLE)) => 0,
        Some(OriginalVtable(MIDDLE_HIGH_PROCESS_VTABLE)) => 1,
        Some(OriginalVtable(MIDDLE_LOW_PROCESS_VTABLE)) => 2,
        _ => 3,
    }
}

// IDA: 00655FE0 MiddleHighProcess_GetActorLightLevel.
// Completion: actor-local light list, interior probe and interior player scene sample.
// TODO: [ ] Exterior directional-light sampling.
pub fn actor_light_level(
    world: &mut World,
    process: Entity,
    actor: Entity,
    exclude_self: bool,
) -> Result<f32, GameError> {
    use crate::bs::scene_graphs::{SceneGraphGlobals, ShadowSceneLight, ShadowSceneNode};
    use crate::gamebryo::properties::LightingProperty;
    use crate::tes::base_extra_list::{EXTRA_LIGHT, ExtraData};
    use crate::tes::forms_references::{parent_cell, reference_position};
    use crate::tes::forms_world::cell_is_interior;
    let Some(property) = world
        .get::<MiddleHighProcess>(process)
        .and_then(|p| p.skin_property_184)
    else {
        return Ok(0.0);
    };
    let player = crate::tes::player::the_player(world) == Some(actor);
    let mut light = world
        .get_mut::<LightingProperty>(property)
        .expect("selected lighting property")
        .first_light();
    let excluded = exclude_self
        .then(|| {
            world
                .get::<TesObjectRefr>(actor)
                .unwrap()
                .extra_data_44
                .get(EXTRA_LIGHT)
        })
        .flatten()
        .and_then(|e| {
            if let ExtraData::LightData { light_00, .. } = e.data {
                Some(light_00)
            } else {
                None
            }
        });
    if player
        && light.is_none()
        && let Some(node) = world
            .get::<super::player::PlayerCharacter>(actor)
            .and_then(|p| p.first_person_node_5d0)
    {
        crate::bs::scene_lighting::update_geometry_lights(world, node)?;
        light = world
            .get_mut::<LightingProperty>(property)
            .unwrap()
            .first_light();
    }
    let index = world
        .get::<LightingProperty>(property)
        .unwrap()
        .scene_index_1c;
    let scene = world
        .resource::<SceneGraphGlobals>()
        .shadow_slots
        .get(usize::from(index))
        .copied()
        .flatten();
    let Some(scene) = scene else {
        if let Some(node) = world
            .get::<crate::tes::reference_3d::Reference3D>(actor)
            .map(|v| v.0)
        {
            super::actor_enable::find_skin_property(world, process, node)?;
        }
        return Ok(0.0);
    };
    let mut point = reference_position(world, actor)?;
    let top = crate::tes::actor::actor_bound_corner(world, actor, true)?;
    let bottom = crate::tes::actor::actor_bound_corner(world, actor, false)?;
    point[2] += ((top[2] - bottom[2]) - 1.0) * 0.5;
    let mut sum = 0.0;
    let mut ambient = 0.0;
    if player {
        // 007C6570 ShadowSceneNode_CalculateLightAtPoint traverses the scene's own list.
        let lights = world
            .get::<ShadowSceneNode>(scene)
            .unwrap()
            .light_list_e4
            .clone();
        for shadow in lights {
            sum += ShadowSceneLight::calculate_light_at_point(world, shadow, point, excluded)?;
        }
        let sun = world
            .get::<ShadowSceneNode>(scene)
            .unwrap()
            .light_118
            .as_ref()
            .and_then(|s| s.light_100)
            .and_then(|source| {
                world.get::<crate::gamebryo::scene_derived::DirectionalLight>(source)
            })
            .ok_or(GameError::unclassified(
                0x007C6570,
                "no directional light for the player light query",
            ))?;
        let [r, g, b] = sun.payload.ambient;
        ambient = r.max(g).max(b);
    } else {
        while let Some(shadow) = light {
            sum += ShadowSceneLight::calculate_light_at_point(world, shadow, point, excluded)?;
            light = world
                .get_mut::<LightingProperty>(property)
                .unwrap()
                .next_light();
        }
    }
    if parent_cell(world, actor).is_some_and(|cell| cell_is_interior(world, cell)) {
        sum += ambient;
    } else if world
        .get::<ShadowSceneNode>(scene)
        .unwrap()
        .light_118
        .is_some()
    {
        return Err(GameError::unclassified(
            0x0065623A,
            "exterior directional-light sample",
        ));
    }
    Ok((sum * 100.0).clamp(0.0, 100.0))
}

// IDA: vt+4 0060CDB0 BaseProcess_CopyFrom(old) on the new process: the RTTI casts of both, the High and MiddleHigh arms, the
// MiddleLow arm (00658760, both MiddleLow) and 00643520 for the Low part. Completion: 100% of the Low and MiddleLow arms; the
// High and MiddleHigh ones stop.
// TODO: [ ] The High (vt+0x15C/0x178/0xD0, 006299F0) and MiddleHigh (0064B650) copy arms.
pub fn copy_process_from(
    world: &mut World,
    process: Entity,
    old_process: Option<Entity>,
) -> Result<(), GameError> {
    // 0060CDB9 a null old process returns.
    let Some(old_process) = old_process else {
        return Ok(());
    };
    // 0060CE81..0060CEBA the old High arm.
    if world.get::<HighProcess>(old_process).is_some() {
        return Err(GameError::unclassified(
            0x0060CE87,
            "0060CE87: copying from a HighProcess is not reached",
        ));
    }
    // 0060CEBC..0060CEF3 the new High arm: vt+0x15C (00629800) > 0.0 runs vt+0x178(0) and vt+0xD0(0); 006299F0 only with an old High process. 00629800 returns +0x248 (0.0 after the constructor): `fcomp
    // 0.0; test ah, 41h` continues only for an ordered value above zero.
    if world
        .get::<HighProcess>(process)
        .is_some_and(|high| high.float_248 > 0.0)
    {
        return Err(GameError::unclassified(
            0x0060CEA0,
            "0060CEA0: vt+0x178 (0064AF90) / vt+0xD0 (0064AF50) are not reached",
        ));
    }
    // 0060CF01..0060CF0E MiddleHigh 0064B650 (both MiddleHigh).
    if world.get::<MiddleHighProcess>(process).is_some()
        && world.get::<MiddleHighProcess>(old_process).is_some()
    {
        return Err(GameError::unclassified(
            0x0060CF0E,
            "0060CF0E: 0064B650 (MiddleHigh copy) is not reached",
        ));
    }
    // 0060CF13..0060CF26 MiddleLow 00658760 MiddleLowProcess_CopyFrom(old) (both MiddleLow): +0x90, then 0065CD10 on +0x94.
    if let (Some(mut n), Some(o)) = (
        world.get::<MiddleLowProcess>(process).cloned(),
        world.get::<MiddleLowProcess>(old_process).cloned(),
    ) {
        n.evaluated_hour_90 = o.evaluated_hour_90;
        copy_actor_values(&mut n.max_modifiers_94, Some(&o.max_modifiers_94))?;
        world.entity_mut(process).insert(n);
    }
    // 0060CF2B..0060CF3A 00643520 LowProcess_CopyFrom(old).
    let (Some(mut n), Some(mut o), Some(mut new_base), Some(mut old_base)) = (
        world.get::<LowProcess>(process).cloned(),
        world.get::<LowProcess>(old_process).cloned(),
        world.get::<BaseProcess>(process).copied(),
        world.get::<BaseProcess>(old_process).copied(),
    ) else {
        return Ok(());
    };
    // 00643526..0064353D old vt+0x28 (00629250: +0x14) into new vt+0x1C (00629240: +0x14).
    n.cur_hour_14 = o.cur_hour_14;
    // 0064353F..0064354D +0x08 moved (old +0x08 = 0), +0x0C copied.
    new_base.editor_package_08 = old_base.editor_package_08;
    old_base.editor_package_08 = None;
    n.field_0c = o.field_0c;
    // 00643550..00643571 +0x1F; old vt+0xC0 (0076C780: byte +0x1D) -> +0x1D; vt+0x390 (007790B0: byte +0x1C) -> +0x1C.
    n.byte_1f = o.byte_1f;
    n.byte_1d = o.byte_1d;
    n.byte_1c = o.byte_1c;
    // 00643574..00643592 +0x44 moved; +0x84; +0x28; +0x24.
    n.block_44 = o.block_44;
    o.block_44 = 0;
    n.byte_84 = o.byte_84;
    n.actor_update_delay_seconds_28 = o.actor_update_delay_seconds_28;
    n.used_item_24 = o.used_item_24;
    // 00643595..006435A1 old vt+0x148 (00629270: byte +0x1E) -> +0x1E.
    n.byte_1e = o.byte_1e;
    // 006435A4..006435AE a nonzero +0x48 is moved.
    if o.block_48 != 0 {
        n.block_48 = o.block_48;
        o.block_48 = 0;
    }
    // 006435B1..006435EA old vt+0x40C (004EF1E0: +0x34 pathing) arm.
    if o.pathing_34 != 0 {
        return Err(GameError::unclassified(
            0x006435C1,
            "006435C1: copying a LowProcess pathing object is not reached",
        ));
    }
    // 006435EC..00643611 old vt+0x2C (00438DB0: +0x18) -> +0x18; +0x2C, +0x04, +0x38.
    n.cur_packed_date_18 = o.cur_packed_date_18;
    n.field_2c = o.field_2c;
    new_base.edit_pack_procedure_type_04 = old_base.edit_pack_procedure_type_04;
    n.field_38 = o.field_38;
    // 00643611 0065CD10 on +0x70 from the old +0x70.
    copy_actor_values(&mut n.damage_modifiers_70, Some(&o.damage_modifiers_70))?;
    // 00643616..00643628 +0x88 (the old process's unset bytes) and +0x8C copied.
    n.unset_88 = o.unset_88;
    n.float_8c = o.float_8c;
    world.entity_mut(process).insert((n, new_base));
    world.entity_mut(old_process).insert((o, old_base));
    Ok(())
}

// IDA: vt+0 0064ACD0 LowProcess deleting destructor (flag 1), and 00658970 MiddleLowProcess's (006586F0: 0065CCC0 on +0x94 first): 00648CB0 (the owned-object, FormHeap-block and list releases, 0065CCC0 on +0x70, 0060CDA0) then FormHeap_Free. A +0x08 package without flag 0x800 (005660A0) is only dropped, being the form's own. Completion: 100% of the reached
// arms; an owned +0x34 / +0x64 object or a flag 0x800 package stops.
// TODO: [ ] The release arms, when a process owns something (M4).
pub fn delete_low_process(world: &mut World, process: Entity) -> Result<(), GameError> {
    // vt+0 00658970 MiddleLowProcess deleting destructor: 006586F0 (vtable A72D64, 0065CCC0 on +0x94), then 00648CB0.
    if world.get::<OriginalVtable>(process).map(|vtable| vtable.0)
        == Some(MIDDLE_LOW_PROCESS_VTABLE)
    {
        let mut middle_low = world
            .get_mut::<MiddleLowProcess>(process)
            .expect("a MiddleLowProcess");
        clear_actor_values(&mut middle_low.max_modifiers_94);
    } else if world.get::<OriginalVtable>(process).map(|vtable| vtable.0)
        != Some(LOW_PROCESS_VTABLE)
    {
        return Err(GameError::unclassified(
            0x0064ACD0,
            "0064ACD0: only the Low and MiddleLow deleting destructors are reached",
        ));
    }
    let (Some(low), Some(base)) = (
        world.get::<LowProcess>(process),
        world.get::<BaseProcess>(process),
    ) else {
        return Err(GameError::unclassified(
            0x0064ACD0,
            "0064ACD0: only a LowProcess deleting destructor is reached",
        ));
    };
    // 00648CE1 an owned +0x34 object (vt+0(1)).
    if low.pathing_34 != 0 {
        return Err(GameError::unclassified(
            0x00648CE1,
            "00648CE1: releasing LowProcess objects is not reached",
        ));
    }
    // 00648CFA a +0x08 package with flag 0x800: TESSaveLoadGame 0045C7A0 or its vt+0x10(1).
    if base.editor_package_08.is_some_and(|package| {
        world
            .get::<crate::tes::forms_ai::TesPackage>(package)
            .is_some_and(|package| package.flags_1c & 0x800 != 0)
    }) {
        return Err(GameError::unclassified(
            0x00648D01,
            "00648D01: freeing a flag 0x800 package",
        ));
    }
    // 00648D3A..00648DF6 +0x08 = 0, the +0x44/+0x48 blocks and the +0x3C/+0x4C/+0x54/+0x5C lists (all empty here), +0x4C/+0x30/+0x5C cleared.
    if low.block_44 != 0
        || low.block_48 != 0
        || low.list_3c != [0; 2]
        || low.list_54 != [0; 2]
        || low.list_4c[1] != 0
        || low.list_5c[1] != 0
        || low.field_64 != 0
    {
        return Err(GameError::unclassified(
            0x00648D3A,
            "00648D3A: releasing LowProcess blocks is not reached",
        ));
    }
    // 00648E10 0065CCC0 on +0x70: 0065C6A0, then the +0x08 and +0x0C entries freed. 00648E1F 0060CDA0 BaseProcess vtable; 0064ACDF FormHeap_Free.
    let mut low = world.get_mut::<LowProcess>(process).expect("checked above");
    clear_actor_values(&mut low.damage_modifiers_70);
    world.despawn(process);
    Ok(())
}

// IDA: 00673A90 ActorProcessManager_AddActor(actor, level, append, afterNeighbor, neighbor): the level list; only when the actor's process level (00659A00) equals `level`, 0067B260 on it (g_thePlayer is never
// added). Completion: 100% of the reached arms; neighbour insertion stops.
// TODO: [ ] 0067B2CB's neighbour insertion (M4, Phase 14).
pub fn actor_process_manager_add(
    world: &mut World,
    actor: Entity,
    level: i32,
    append: bool,
    after_neighbor: bool,
) -> Result<(), GameError> {
    // 00673A95..00673AB6 the level list; 00673ABE 00659A00.
    let Some(id) = ActorListId::for_level(level) else {
        return Ok(());
    };
    if actor_process_level(world, actor) != level {
        return Ok(());
    }
    actor_process_list_add(world, id, actor, append, after_neighbor)
}

// IDA: 0067B260 ActorProcessList_Add. Completion: append and prepend, including the player exclusion.
// TODO: [ ] Insertion relative to a neighbour at 0067B2CB.
pub(super) fn actor_process_list_add(
    world: &mut World,
    id: ActorListId,
    actor: Entity,
    append: bool,
    after_neighbor: bool,
) -> Result<(), GameError> {
    // 0067B266 g_thePlayer is never added.
    if world
        .get_resource::<PlayerGlobals>()
        .is_some_and(|globals| globals.the_player_b333c4 == Some(actor))
    {
        return Ok(());
    }
    let mut globals = world.resource_mut::<ActorProcessGlobals>();
    let list = globals.manager.list_mut(id);
    if after_neighbor {
        // 0067B2CB..0067B2F8 insert at the neighbour (not reached).
        return Err(GameError::unclassified(
            0x0067B2CB,
            "0067B2CB: neighbour insertion is not reached",
        ));
    }
    if append {
        // 0067B2A7 AppendNonNull(tail, actor).
        let tail = list.tail_08;
        list.list.append_non_null(tail, actor);
    } else {
        // 0067B283 PrependNonNull(head, actor).
        list.list.prepend_non_null(actor);
    }
    // 0067B288..0067B2A3 the actor's process vt+0x28 (00629250: fld [+0x14]) and the tail item's process vt+0x28: read-only, the results are popped.
    // 0067B2FD..0067B307 advance the tail when it has a next node.
    if let Some(next) = list.list.next(list.tail_08) {
        list.tail_08 = next;
    }
    id.mark(world, actor);
    Ok(())
}

// IDA: 0065AD00 Actor_GetDesiredProcessLevel. With no active save and the player sleeping, the process's level, or -1 with none. An active save, a 3D, or 004354F0 ModelLoader_IsReferenceQueued (the model loader 00B33A1C's +0x08 map of reference tasks holds the actor) runs the grid tests: 0043F8C0(cell, 1) is 0 and (cell, 0) is 1. Otherwise a cell with the type 0x0D extra (004C9FB0), or 0043F8C0(cell, 0), is 2; else 3. Completion: 100% of the arms a load reaches.
pub(crate) fn actor_desired_level(world: &World, actor: Entity) -> Result<i32, GameError> {
    let flags = world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .map(|game| game.fields_14[1])
        .ok_or(GameError::unclassified(
            0x0065AD09,
            "0065AD09: a null g_saveLoadGame faults",
        ))?;
    let active = flags & 1 != 0;
    if !active && player_is_sleeping(world)? {
        return Ok(actor_process_level(world, actor));
    }
    let reference = world.get::<TesObjectRefr>(actor).expect("a reference");
    let cell = reference.parent_cell_40;
    // 004354F0 ModelLoader_IsReferenceQueued: the model loader's +0x08 map (its reference tasks) holds the actor.
    let queued = world
        .get_resource::<crate::bs::task_manager::TaskGlobals>()
        .and_then(|tasks| tasks.model_loader_b33a1c.as_ref())
        .is_some_and(|loader| loader.references.map_08.contains_key(&actor));
    if active || reference.loaded_3d_3c != 0 || queued {
        if cell_attached(world, cell, true) {
            return Ok(0);
        }
        if cell_attached(world, cell, false) {
            return Ok(1);
        }
    }
    if cell.is_some_and(|cell| cell_has_type_0d(world, cell)) || cell_attached(world, cell, false) {
        return Ok(2);
    }
    Ok(3)
}

/// IDA: 006748B0 on g_actorProcessManager: an actor already in the +0x58 list, or with no process, returns. A death state (vt+0x198(0) 005E33B0: +0xB0 of 1, 2 or 6), a process level other than 0065AD00's, the process's +0x20 byte (vt+0x248 0051AAD0), flag 0x20 or a sleeping player sets flag 0x20000 (vt+0xA0(1) 0046AB00) and prepends the actor; otherwise it returns. Every SetDeleted(0) on an actor gets here. The C++ never runs it. Completion: 100% of the arms a load reaches.
pub fn actor_process_manager_refresh(world: &mut World, actor: Entity) -> Result<(), GameError> {
    if world
        .resource::<ActorProcessGlobals>()
        .manager
        .queued_58
        .items()
        .any(|item| item == actor)
    {
        return Ok(());
    }
    let Some(process) = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
    else {
        return Ok(());
    };
    let level = process_level(world, process);
    let desired = actor_desired_level(world, actor)?;
    let dead = matches!(
        world.get::<Actor>(actor).expect("an actor").dead_state_b0,
        1 | 2 | 6
    );
    let byte_20 = world
        .get::<LowProcess>(process)
        .map_or(0, |low| low.byte_20);
    let deleted = world.get::<FormIdentity>(actor).expect("a form").flags_08 & 0x20 != 0;
    if !dead && level == desired && byte_20 == 0 && !deleted && !player_is_sleeping(world)? {
        return Ok(());
    }
    world
        .get_mut::<FormIdentity>(actor)
        .expect("a form")
        .flags_08 |= 0x0002_0000;
    world
        .resource_mut::<ActorProcessGlobals>()
        .manager
        .queued_58
        .prepend_non_null(actor);
    Ok(())
}

// IDA: 00674550 ActorProcessManager_RemoveActor(actor, level): 00579440 (its result unused, no stores), the level list, the 00B3BD74/00B3BD78 cursors cleared when they point at the actor, then 0067B320(list,
// actor, 0). Returns 0067B320's byte. `save_load_active` is 0045A500's answer at 0067B411. Completion: 100%.
pub fn actor_process_manager_remove(
    world: &mut World,
    actor: Entity,
    level: i32,
    save_load_active: bool,
) -> bool {
    let mut globals = world.resource_mut::<ActorProcessGlobals>();
    let manager = &mut globals.manager;
    // 0067457D..0067459E the cursors pointing at the actor are cleared.
    if manager.cursor_item(manager.cursor_74) == Some(actor) {
        manager.cursor_74 = None;
    }
    if manager.cursor_item(manager.cursor_78) == Some(actor) {
        manager.cursor_78 = None;
    }
    let Some(id) = ActorListId::for_level(level) else {
        return false;
    };
    // 0067B320 ActorProcessList_Remove(actor, 0).
    if !actor_process_list_remove(world, id, actor, None) {
        return false;
    }
    // 0067B403..0067B422 flag 0x20 and no active save/load: 00659BC0 deletes the process: +0x58 vt+0(1) (the deleting destructor), then +0x58 = 0. Host: the Low / MiddleLow destructors free only their own members.
    release_deleted_process(world, actor, save_load_active);
    true
}

// IDA: 0067B320 ActorProcessList_Remove(actor, start) (__thiscall on the list): the High-list two-item and cursor clears, the search from `start` (null: the head), the unlink (a match at the head pulls the next
// node in). Returns whether the actor was found. `actor_process_manager_remove` runs the flag-0x20 process release of 0067B403. Completion: 100%.
pub fn actor_process_list_remove(
    world: &mut World,
    id: ActorListId,
    actor: Entity,
    start: Option<ActorListNode>,
) -> bool {
    let mut globals = world.resource_mut::<ActorProcessGlobals>();
    let manager = &mut globals.manager;
    // 0067B329..0067B351 on the High list (+0x68) with exactly two items: 00B3BD74 = 0.
    if id == ActorListId::High68 && manager.high_68.list.items().len() == 2 {
        manager.cursor_74 = None;
    }
    if manager.cursor_item(manager.cursor_74) == Some(actor) {
        manager.cursor_74 = None;
    }
    if manager.cursor_item(manager.cursor_78) == Some(actor) {
        manager.cursor_78 = None;
    }
    let list = manager.list_mut(id);
    // 0067B38D..0067B3BC find the actor (previous node in ebp). 0067B389..0067B38B a nonnull start node replaces the head.
    let mut previous = None;
    let mut node = start.unwrap_or(ActorListNode::HEAD);
    loop {
        if list.list.next(node).is_none() && list.list.item(node).is_none() {
            return false;
        }
        if list.list.item(node) == Some(actor) {
            break;
        }
        previous = Some(node);
        let Some(next) = list.list.next(node) else {
            return false;
        };
        node = next;
    }
    let removed = if let Some(previous) = previous {
        // 0067B3C2..0067B3D3 RemoveFirstMatchingItem from the previous node; a removed tail moves the tail back.
        list.list.remove_first_matching(previous, actor);
        if node == list.tail_08 {
            list.tail_08 = previous;
        }
        Some(actor)
    } else {
        let head_item = list.list.item(ActorListNode::HEAD);
        // 0067B3D8..0067B400 the head: pull the next node in (FormHeap_Free) or clear it; without a next node the tail is the head.
        list.list.pull_next_in(ActorListNode::HEAD);
        if list.list.next(ActorListNode::HEAD).is_none() {
            list.tail_08 = ActorListNode::HEAD;
        }
        head_item
    };
    if let Some(removed) = removed
        && !list.list.ordered_items().any(|item| item == removed)
    {
        id.unmark(world, removed);
    }
    true
}

/// IDA: N/A check the four actor indexes after the frame's world transaction.
/// The original list remains the authority for traversal order.
#[cfg(debug_assertions)]
pub(crate) fn check_process_level_markers(world: &mut World) {
    use std::collections::HashSet;

    fn check<M: Component>(world: &mut World, id: ActorListId) {
        let mut members: HashSet<_> = world
            .resource::<ActorProcessGlobals>()
            .manager
            .list(id)
            .list
            .ordered_items()
            .collect();
        let mut marked = world.query_filtered::<Entity, With<M>>();
        assert!(
            marked.iter(world).all(|actor| members.remove(&actor)) && members.is_empty(),
            "process level marker differs from {id:?}"
        );
    }

    if world.contains_resource::<ActorProcessGlobals>() {
        check::<ProcessLevelHigh>(world, ActorListId::High68);
        check::<ProcessLevelMiddleHigh>(world, ActorListId::MiddleHigh00);
        check::<ProcessLevelMiddleLow>(world, ActorListId::MiddleLow0c);
        check::<ProcessLevelLow>(world, ActorListId::Low18);
    }
}

/// IDA: 0067B403..0067B422 flag 0x20 and no active save/load: 00659BC0 deletes the process, as the tail of `actor_process_manager_remove`, for a caller that ran 0067B320 with a start node. Completion: 100%.
pub fn release_deleted_process(world: &mut World, actor: Entity, save_load_active: bool) {
    let deleted = world
        .get::<FormIdentity>(actor)
        .is_some_and(|identity| identity.flags_08 & 0x20 != 0);
    if deleted && !save_load_active {
        let mut mobile = world.get_mut::<MobileObject>(actor);
        if let Some(process) = mobile.as_mut().and_then(|mobile| mobile.process_58.take()) {
            world.despawn(process);
        }
    }
}

/// IDA: 0064B240 MiddleHighProcess_GetEquippedWeaponData(1) (process vt+0xEC) on an actor's process: +0xE4's form when it is
/// worn (InventoryEntry_IsWorn). A process without a MiddleHigh part answers none. Completion: 100%.
pub fn actor_worn_weapon(world: &World, reference: Entity) -> Option<Entity> {
    let process = world.get::<MobileObject>(reference)?.process_58?;
    world
        .get::<MiddleHighProcess>(process)?
        .weapon_e4
        .filter(|entry| entry.worn)
        .map(|entry| entry.form_08)
}

/// IDA: 00643640, the two values stored by LowProcess_UpdateTimeStamps.
#[must_use]
pub fn process_stamps(world: &World) -> (f32, i32) {
    (game_hour(world), process_packed_date(world))
}

/// IDA: 00643652..00643681 LowProcess_UpdateTimeStamps: +0x18 `((year << 4 | month) << 9) | movsx(day)`. Completion: 100%.
#[must_use]
pub(crate) fn process_packed_date(world: &World) -> i32 {
    let year = game_year(world) << 4;
    let packed = (game_month(world) | year) << 9;
    i32::from(game_day(world) as i8) | packed
}

// IDA: the shared sum of 00643340 and 006433E0: fild base + actor +0x88 + process +0x70, a float at each add; code 11 starts from 004D8FB0. `fild` is exact and rounds at the first fadd, so the C++'s int to float first differs past 2^24.
fn low_process_value(
    world: &mut World,
    actor: Entity,
    process: Entity,
    code: u32,
) -> Result<f32, GameError> {
    let a = actor_values_get(
        &world.get::<Actor>(actor).expect("an actor").av_modifiers_88,
        code as u8,
    );
    let b = world.get::<LowProcess>(process).map_or(0.0, |low| {
        actor_values_get(&low.damage_modifiers_70, code as u8)
    });
    if code == 11 {
        let encumbrance = reference_encumbrance(world, actor)?;
        return Ok((encumbrance + a) + b);
    }
    let base = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .base_form_1c
        .ok_or(GameError::unclassified(
            0x006433B7,
            "006433B7: vt+0x170 with no base faults",
        ))?;
    let value = base_actor_value(world, base, code)?;
    Ok(((f64::from(value) + f64::from(a)) as f32) + b)
}

// IDA: 00658790 MiddleLowProcess_GetActorValue and 00628940 HighProcess_GetActorValue.
// Completion: +0x94 is read before the Low virtual. The integer route truncates twice.
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "the x87 compare: a NaN takes the negated arm"
)]
pub(crate) fn process_actor_value(
    world: &mut World,
    actor: Entity,
    process: Entity,
    code: u32,
) -> Result<i32, GameError> {
    let high = world.get::<HighProcess>(process);
    if let Some(high) = high {
        match code {
            11 if !(high.cached_encumbrance_294 < 0.0) => {
                return Ok(x87::ftol(high.cached_encumbrance_294.floor()));
            }
            48 if high.cached_paralysis_298 >= 0 => return Ok(high.cached_paralysis_298),
            _ => {}
        }
    }
    let modifier = world
        .get::<MiddleLowProcess>(process)
        .map(|p| actor_values_get(&p.max_modifiers_94, code as u8));
    let low = x87::ftol(low_process_value(world, actor, process, code)?);
    let result = modifier.map_or(low, |m| x87::ftol((f64::from(low) + f64::from(m)) as f32));
    if let Some(mut high) = world.get_mut::<HighProcess>(process) {
        match code {
            11 => {
                high.cached_encumbrance_294 = result as f32;
                return Ok(x87::ftol(high.cached_encumbrance_294.floor()));
            }
            48 => high.cached_paralysis_298 = result,
            _ => {}
        }
    }
    Ok(result)
}

// IDA: 006587E0 MiddleLowProcess_GetActorValueFloat and 006289F0 HighProcess_GetActorValueFloat.
// Completion: the two High cache arms, including the integer virtual for paralysis.
#[allow(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "the x87 compare: a NaN takes the negated arm"
)]
pub(crate) fn process_actor_value_float(
    world: &mut World,
    actor: Entity,
    process: Entity,
    code: u32,
) -> Result<f32, GameError> {
    if let Some(high) = world.get::<HighProcess>(process) {
        if code == 48 {
            return Ok(process_actor_value(world, actor, process, code)? as f32);
        }
        if code == 11 && !(high.cached_encumbrance_294 < 0.0) {
            return Ok(high.cached_encumbrance_294);
        }
    }
    let modifier = world
        .get::<MiddleLowProcess>(process)
        .map(|p| actor_values_get(&p.max_modifiers_94, code as u8));
    let low = low_process_value(world, actor, process, code)?;
    let result = modifier.map_or(low, |m| low + m);
    if code == 11
        && let Some(mut high) = world.get_mut::<HighProcess>(process)
    {
        high.cached_encumbrance_294 = result;
    }
    Ok(result)
}

/// IDA: 00643640 LowProcess vt+0x20.
pub fn process_update_stamps(world: &mut World, process: Entity) {
    let hour = game_hour(world);
    let Some(mut low) = world.get_mut::<LowProcess>(process) else {
        return;
    };
    low.cur_hour_14 = hour;
    let date = process_packed_date(world);
    world
        .get_mut::<LowProcess>(process)
        .expect("a LowProcess")
        .cur_packed_date_18 = date;
}

/// IDA: 00648E40 LowProcess_SelectPackage(actor): 005E0330 (00468670 -> 00569020); a change from a nonnull package to none or one below 0xFF000000 clears the actor's change flags 0x30000; +0x08 the new one; a new one with no location or of type 2 records the start location. Completion: 100%.
fn process_select_package(
    world: &mut World,
    process: Entity,
    actor: Entity,
    arms: &mut dyn AiArms,
) -> Result<(), GameError> {
    let selected = match actor_package_list(world, actor)? {
        Some(list) => select_from_list(world, &list, actor, arms)?,
        None => None,
    };
    let old = base_of(world, process).editor_package_08;
    if old.is_some() && selected != old {
        let temporary = selected.is_some_and(|selected| {
            world
                .get::<FormIdentity>(selected)
                .expect("a form")
                .form_id_0c
                >= 0xFF00_0000
        });
        if !temporary {
            clear_change_flags(world, actor, 0x3_0000)?;
        }
    }
    world
        .get_mut::<BaseProcess>(process)
        .expect("a process")
        .editor_package_08 = selected;
    if let Some(selected) = selected {
        let location = package_data(world, selected).location_24;
        if location.is_none_or(|location| location.type_00 == 2) {
            record_start_location(world, actor)?;
        }
    }
    Ok(())
}

// IDA: 00648120 LowProcess vt+0x188(actor, step): +0x04 += step, floored at 0; past the sequence the last index, and an end procedure raises 0x400.
pub(crate) fn process_advance_procedure(
    world: &mut World,
    process: Entity,
    actor: Entity,
    step: i32,
) -> Result<(), GameError> {
    let mut base = base_of(world, process);
    let index = (base.edit_pack_procedure_type_04 as i32 + step).max(0);
    base.edit_pack_procedure_type_04 = index as u32;
    world.entity_mut(process).insert(base);
    let Some(package) = base.editor_package_08 else {
        return Ok(());
    };
    let count = procedure_count(package_data(world, package).field_18 as i32);
    if index >= count {
        if count < 1 {
            return Err(GameError::unclassified(
                0x0064816F,
                "0064816F: a -1 count reads before the row",
            ));
        }
        let last = (count - 1) as u32;
        world
            .get_mut::<BaseProcess>(process)
            .expect("a process")
            .edit_pack_procedure_type_04 = last;
        if procedure_at(world, package, last)? == 0x2C {
            script_event_or(world, actor, 0x400, Some(package));
        }
    }
    Ok(())
}

// IDA: 00649190 LowProcess vt+0x55C(actor): a type 0 location's reference into +0x30; 00569A10 `cmp byte,3; setg` is signed, so types up to 3 and any negative do nothing (the C++ casts to unsigned); above, without flag 4 the procedure advances. With flag 4 the 00646A80 search, a host stop.
// TODO: [ ] 006491ED the +0x4C candidates from 00446B90 and 00646A80: a flag 4 package with a type 4 or 5 location.
fn process_update_location_target(
    world: &mut World,
    process: Entity,
    actor: Entity,
) -> Result<(), GameError> {
    let Some(package) = base_of(world, process).editor_package_08 else {
        return Ok(());
    };
    let data = package_data(world, package);
    let flags = data.flags_1c;
    let Some(location) = data.location_24 else {
        return Ok(());
    };
    if location.type_00 == 0 {
        let reference = location_reference(world, &location)?;
        low_mut(world, process).field_30 = reference;
        return Ok(());
    }
    if location.type_00 <= 3 {
        return Ok(());
    }
    if flags & 4 == 0 {
        return process_advance_procedure(world, process, actor, 1);
    }
    Err(GameError::unclassified(
        0x006491ED,
        "006491ED: the 00646A80 candidate search",
    ))
}

/// IDA: 0064A380 LowProcess vt+0x14(reference, events): the scheduled packages since the last stamp: each whose conditions pass becomes current in turn and places the actor; the final cell move (004DD4B0) and +0x08 cleared on `this`; then +0x84 = 0 on the actor's current +0x58, re-read at 0064A878. With `events` 0x200 and 0x400 are raised before each store; 005ED860 passes 0. Completion: 100%, with `activate_reference`'s stops.
pub(crate) fn process_run_scheduled(
    world: &mut World,
    process: Entity,
    actor: Entity,
    events: bool,
    arms: &mut dyn AiArms,
) -> Result<(), GameError> {
    let current = base_of(world, process).editor_package_08;
    if let Some(current) = current
        && package_data(world, current).flags_1c & (4 | 0x4000 | 2) != 0
    {
        low_mut(world, process).byte_84 = 0;
        return Ok(());
    }
    let scheduled = match actor_package_list(world, actor)? {
        Some(list) => {
            let hour = game_hour(world);
            collect_scheduled_packages(world, &list, process, hour)?
        }
        None => Vec::new(),
    };
    let mut cell = None;
    let mut space = None;
    let mut rotate = false;
    for candidate in scheduled {
        let data = package_data(world, candidate);
        let flags = data.flags_1c;
        let kind = data.type_20;
        let location = data.location_24;
        let target = data.target_28;
        let conditions = data.conditions_34.clone();
        let target_ref = match &target {
            Some(target) => target_reference(world, target)?,
            None => None,
        };
        low_mut(world, process).byte_84 = 0;
        if !evaluate_conditions(world, &conditions, Some(actor), target_ref, arms)? {
            continue;
        }
        if events {
            let old = base_of(world, process).editor_package_08;
            script_event_or(world, actor, 0x200, Some(candidate));
            script_event_or(world, actor, 0x400, old);
        }
        world
            .get_mut::<BaseProcess>(process)
            .expect("a process")
            .editor_package_08 = Some(candidate);
        package_arrival(world, actor)?;
        process_update_location_target(world, process, actor)?;
        let mut place = |world: &mut World| -> Result<(), GameError> {
            let where_ = package_location_position(world, candidate, actor)?;
            set_reference_position(world, actor, where_)?;
            cell = package_location_cell(world, candidate, actor)?;
            space = package_location_world(world, candidate, actor)?;
            Ok(())
        };
        // 0064A894 by +0x20, unsigned: a negative type takes the default.
        match kind {
            5 | 6 => {
                place(world)?;
                rotate = location.is_some_and(|location| location.type_00 == 1);
            }
            0 => {
                if let Some(reference) =
                    target_ref.filter(|&form| form_flags(world, form) & 0x20 == 0)
                {
                    let position =
                        crate::tes::forms_references::reference_position(world, reference)?;
                    set_reference_position(world, actor, position)?;
                    rotate = true;
                    if world.get::<Actor>(reference).is_none()
                        && form_flags(world, reference) & (0x800 | 0x20) == 0
                    {
                        activate_reference(world, reference, actor)?;
                    }
                    cell = crate::tes::forms_references::parent_cell(world, reference);
                    space = reference_world(world, reference);
                } else {
                    place(world)?;
                    rotate = false;
                }
            }
            3 | 4 => {
                place(world)?;
                rotate = match &location {
                    Some(location) => location_reference(world, location)?.is_some(),
                    None => false,
                };
            }
            1 => {
                if let Some(target) = &target
                    && (target_ref.is_some()
                        || target_object(target)?.is_some()
                        || target_object_type(target) != 0)
                    && let Some(reference) = target_ref.filter(|&form| {
                        form_flags(world, form) & 0x20 == 0 && Some(form) != the_player(world)
                    })
                {
                    let position =
                        crate::tes::forms_references::reference_position(world, reference)?;
                    set_reference_position(world, actor, position)?;
                    cell = crate::tes::forms_references::parent_cell(world, reference);
                    space = reference_world(world, reference);
                    rotate = false;
                }
            }
            _ => {}
        }
        package_arrival(world, actor)?;
        if flags & 0x400 != 0 {
            let day = game_day_byte(world);
            run_once_add(world, actor, candidate, day);
        }
        if flags & 0x10 != 0 {
            doors_near(world, actor, true)?;
        } else if flags & 0x80 != 0 {
            doors_near(world, actor, false)?;
        }
    }
    if cell.is_some() || space.is_some() {
        if rotate {
            set_rotation_x(world, actor, crate::tes::ai::ai_package::FLT_MAX)?;
        }
        move_to_cell(world, actor, cell, space)?;
        world
            .get_mut::<BaseProcess>(process)
            .expect("a process")
            .editor_package_08 = None;
    }
    if let Some(current) = process_of(world, actor) {
        low_mut(world, current).byte_84 = 0;
    }
    Ok(())
}

/// IDA: 00649340 LowProcess vt+0x18(reference, force). Completion: 100%, past the 0x800 arm 0064958F.
/// TODO: [ ] 0064959C 005EAE70, a flag 0x800 package ending; 006497C4 draining a nonempty +0x3C.
#[allow(
    clippy::too_many_lines,
    reason = "one function in the exe, in its order"
)]
pub(crate) fn process_evaluate_package(
    world: &mut World,
    process: Entity,
    actor: Entity,
    force: bool,
    arms: &mut dyn AiArms,
) -> Result<bool, GameError> {
    let hour = game_hour(world);
    let mut force = force;
    let mut old = base_of(world, process).editor_package_08;
    if !force && let Some(old) = old {
        // 005E6BA0 a type 0x0F; 00649377 vt+0x334(1) 005E6110: 0x0C or 0x0D. Both return false; the C++ stops on the second.
        let kind = package_data(world, old).type_20;
        if kind as u8 == 15 || kind == 12 || kind == 13 {
            return Ok(false);
        }
    }
    let mut result = false;
    if let Some(previous) = old {
        if package_data(world, previous).field_18 == 0xFFFF_FFFF {
            package_init_procedure(world, previous)?;
        }
        let data = package_data(world, previous);
        let flags = data.flags_1c;
        let kind = data.type_20 as u8;
        let schedule = data.schedule_2c;
        let duration = schedule.duration_04 as i32;
        let index = base_of(world, process).edit_pack_procedure_type_04;
        if duration != 0 && flags & 4 != 0 {
            let mut check = true;
            if kind != 5 && kind != 4 && kind != 3 && procedure_at(world, previous, index)? != 0x2C
            {
                check = false;
            }
            let mut done = false;
            if check {
                let start = i32::from(schedule.data_00[3] as i8);
                let mut end = if start == -1 {
                    x87::ftol(
                        world
                            .get::<LowProcess>(process)
                            .expect("a LowProcess")
                            .field_0c,
                    )
                    .wrapping_add(duration)
                } else {
                    start.wrapping_add(duration)
                };
                if end > 23 {
                    end -= 23;
                } else if start as f32 > hour {
                    result = true;
                }
                if x87::ftol(hour) >= end {
                    result = true;
                } else if !result && force {
                    force = false;
                }
                script_event_or(world, actor, 0x400, Some(previous));
                if flags & 0x400 != 0 {
                    let day = game_day_byte(world);
                    run_once_add(world, actor, previous, day);
                }
                done = result;
            }
            // 0064957D.
            if !done && (!force || flags & 0x4000 != 0) {
                return Ok(result);
            }
        } else if flags & 4 != 0 {
            if procedure_at(world, previous, index)? != 0x2C {
                return Ok(false);
            }
            result = true;
        } else if flags & 2 == 0 {
            result = true;
        } else {
            if !package_at_location(world, previous, actor, false, -1.0)?
                && procedure_at(world, previous, index)? != 0x2C
            {
                return Ok(result);
            }
            result = true;
        }
        // 0064958F.
        if flags & 0x800 != 0 {
            return Err(GameError::unclassified(
                0x0064959C,
                "0064959C: 005EAE70, a flag 0x800 package ending",
            ));
        }
        process_select_package(world, process, actor, arms)?;
        if base_of(world, process).editor_package_08 == Some(previous) {
            return Ok(false);
        }
        let player_cell = the_player(world)
            .and_then(|player| crate::tes::forms_references::parent_cell(world, player));
        if flags & 0x200 != 0
            && player_cell == crate::tes::forms_references::parent_cell(world, actor)
        {
            // vt+0x38C(1) on LowProcess 0068F970 is a no-op.
            world
                .get_mut::<BaseProcess>(process)
                .expect("a process")
                .editor_package_08 = Some(previous);
            return Ok(false);
        }
        low_mut(world, process).field_0c = hour;
        world
            .get_mut::<BaseProcess>(process)
            .expect("a process")
            .edit_pack_procedure_type_04 = 0;
    } else {
        process_select_package(world, process, actor, arms)?;
        old = base_of(world, process).editor_package_08;
        result = old.is_some();
        low_mut(world, process).field_0c = hour;
        world
            .get_mut::<BaseProcess>(process)
            .expect("a process")
            .edit_pack_procedure_type_04 = 0;
        let mut low = low_mut(world, process);
        low.field_2c = None;
        low.byte_1c = 0; // vt+0x394(0) 006292D0.
    }
    if !result {
        return Ok(result);
    }
    // 006493EC.
    let current = base_of(world, process).editor_package_08;
    if let Some(old_package) = old
        && current != old
    {
        let flags = package_data(world, old_package).flags_1c;
        if flags & 0x10 != 0 {
            doors_near(world, actor, true)?;
        } else if flags & 0x80 != 0 {
            doors_near(world, actor, false)?;
        }
    }
    let current = base_of(world, process).editor_package_08;
    if let Some(package) = current
        && current != old
    {
        let flags = package_data(world, package).flags_1c;
        if flags & 8 != 0 {
            doors_near(world, actor, true)?;
        } else if flags & 0x40 != 0 {
            doors_near(world, actor, false)?;
        }
    }
    let current = base_of(world, process).editor_package_08;
    if let Some(package) = current {
        // 00649702 005EAE10: with old == current, vt+0x334(1) false gives the zeroed +0x2C; a 0x0C or 0x0D goes to vt+0x338's chain.
        let kind = package_data(world, package).type_20;
        if old == current && (kind == 12 || kind == 13) {
            return Err(GameError::unclassified(
                0x00649702,
                "00649702: 005EAE10's 005E0AF0 chain",
            ));
        }
        if package_target_is_actor(world, package)? && kind != 0 {
            package_apply_target(world, package, actor)?;
        }
    }
    let current = base_of(world, process)
        .editor_package_08
        .ok_or(GameError::unclassified(
            0x00566DDD,
            "00566DDD: 00566DC0 on a null package faults",
        ))?;
    if package_at_location(world, current, actor, false, -1.0)? {
        package_arrival(world, actor)?;
    }
    low_mut(world, process).byte_84 = 0;
    record_start_location(world, actor)?;
    let mut low = low_mut(world, process);
    low.field_2c = None;
    // vt+0x174 0 skips vt+0x178; vt+0xBC(0) 007790C0 and vt+0x394(0) 006292D0.
    low.byte_1d = 0;
    low.byte_1c = 0;
    if low.list_3c != [0; 2] {
        return Err(GameError::unclassified(
            0x006497C4,
            "006497C4: freeing the +0x3C list",
        ));
    }
    Ok(result)
}

// IDA: 0060D1F0 at version 125 with flags 4.
fn write_base_process(out: &mut SaveWriter, world: &World, process: &ProcessView<'_>) {
    let id = form_id(world, process.base.editor_package_08);
    out.u32(id);
    if id != 0 {
        out.u32(process.base.edit_pack_procedure_type_04);
    }
    out.f32(process.low.field_0c);
    out.f32(process.low.field_10);
}

// IDA: 006471A0 at version 125 with flags 4.
fn write_low_process(
    out: &mut SaveWriter,
    world: &World,
    process: &ProcessView<'_>,
) -> Result<(), GameError> {
    write_base_process(out, world, process);
    let low = process.low;
    out.f32(low.cur_hour_14);
    out.u32(low.cur_packed_date_18 as u32);
    out.u8(low.byte_1c);
    out.u8(low.byte_1d);
    out.u8(low.byte_84);
    out.u8(low.byte_1f);
    out.u8(low.byte_20);
    out.u32(low.unset_88);
    out.f32(low.actor_update_delay_seconds_28);
    out.u32(null_id(low.used_item_24, "006472C9: a used item's ID")?);
    out.u32(low.field_38);
    let follow = if low.block_44 == 0 {
        form_id(world, low.field_2c)
    } else {
        0
    };
    out.u32(follow);
    out.u32(form_id(world, low.field_30));
    out.f32(low.float_8c);
    out.u8(low.byte_1e);
    Ok(())
}

// IDA: 00658CB0 at version 125 with flags 4.
fn write_middle_low_process(
    out: &mut SaveWriter,
    world: &World,
    process: &ProcessView<'_>,
) -> Result<(), GameError> {
    write_low_process(out, world, process)?;
    out.u32(process.middle_low.evaluated_hour_90 as u32);
    Ok(())
}

// IDA: 00651140 at version 125 with flags 4. 0x134's word goes out as milliseconds over 1000, at 24 bits (D6); 0x136 and 0x128..0x130 are copied through the stack.
pub(crate) fn write_middle_high_process(
    out: &mut SaveWriter,
    world: &World,
    process: &ProcessView<'_>,
) -> Result<(), GameError> {
    write_middle_low_process(out, world, process)?;
    let m = process.middle_high;
    out.u8(m.weapon_draw_requested_114);
    out.u8(m.weapon_drawn_115);
    out.u8(m.cast_animation_update_flag_14c);
    out.f32(m.actor_alpha_154);
    out.f32(m.float_158);
    out.u32(null_id(m.field_13c, "006512FE: the +0x13C form's ID")?);
    out.u32(m.field_e0);
    out.u8(m.byte_180);
    // +0xD4..+0xDC: 12 bytes the port has no field for; zero in the run.
    out.words(&[0; 3]);
    out.f32(m.float_c4);
    out.u16(m.word_138);
    out.u8(m.sleep_state_11d);
    out.u8(m.furniture_marker_index_124);
    out.u32(form_id(world, m.furniture_reference_120));
    out.f32(f32::from(m.word_134) / 1000.0);
    out.u8(m.byte_136);
    out.words(&m.furniture_marker_position_128);
    out.u8(m.knocked_state_11c);
    out.u32(null_id(
        m.current_magic_item_144,
        "00651475: the +0x144 item",
    )?);
    out.u32(null_id(m.field_178, "0065149E: the +0x178 target")?);
    empty_list(
        m.effect_list_174.unwrap_or_default(),
        "0068DD30: an active effect",
    )?;
    out.u16(0);
    out.u8(m.byte_c8);
    out.u8(m.bytes_168[0]);
    out.u8(m.bytes_168[1]);
    out.f32(m.float_b8);
    out.f32(m.torch_evaluation_timer_bc);
    empty_list([m.fields_a8[0], m.fields_a8[1]], "0065156E: the +0xA8 list")?;
    out.u16(0);
    out.u8(m.bytes_168[3]);
    out.u32(null_id(m.field_148, "006515DF: the +0x148 item")?);
    Ok(())
}

// IDA: 00651B90: the controller's fields, with flag 0x800 of +0x1F4 an arm no run reached, and vt+0x190 true for the player.
pub(crate) fn write_controller(
    out: &mut SaveWriter,
    controller: &BhkCharacterController,
) -> Result<(), GameError> {
    out.u32(controller.context_state_1ec);
    out.u32(controller.field_2a0);
    out.u32(controller.flags_1f4);
    if controller.flags_1f4 & 0x800 != 0 {
        return Err(GameError::unclassified(
            0x00651C50,
            "00651C50: 005E1500, flag 0x800's vector",
        ));
    }
    // +0x2E0: 16 bytes the port has no field for; zero in the run.
    out.words(&[0; 4]);
    out.words(&controller.zero_2f0);
    // 00651C96: +0x08's +0x10 through 0043F3E0. The port has no +0x08 object; the run holds zero here, and not the proxy's position.
    out.floats(&[0.0; 3]);
    // +0x31C, which the port has no field for; zero in the run.
    out.u32(0);
    out.f32(controller.floats_320[0]);
    out.f32(controller.floats_320[1]);
    out.f32(controller.floats_320[3]);
    out.f32(controller.floats_320[4]);
    Ok(())
}

/// IDA: 00651DD0 MiddleHighProcess_ResetWeaponNodeScales: unless 00B33D80 is set (`EquipmentRebuildFlag`), each of
/// +0xFC..+0x110 that is set takes a scale (+0x60) of |1.0|. Completion: 100%.
pub fn reset_weapon_node_scales(world: &mut World, process: Entity) {
    if world
        .get_resource::<crate::tes::biped_attach::EquipmentRebuildFlag>()
        .is_some_and(|flag| flag.active_b33d80)
    {
        return;
    }
    let Some(nodes) = world
        .get::<MiddleHighProcess>(process)
        .map(|middle| middle.weapon_nodes_fc)
    else {
        return;
    };
    for node in [
        nodes.weapon_fc,
        nodes.torch_100,
        nodes.forearm_twist_104,
        nodes.sheath_108,
        nodes.quiver_10c,
        nodes.arrow_bone_110,
    ]
    .into_iter()
    .flatten()
    {
        if let Some(mut payload) =
            world.get_mut::<crate::gamebryo::scene_node::SceneNodePayload>(node)
        {
            payload.local_transform.scale = 1.0;
        }
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::wrapped_angle_word;

    // 006FAF19 and 006FAF3C leave on an unordered compare, so NaN ends the loops.
    #[test]
    fn a_nan_angle_does_not_loop() {
        wrapped_angle_word(f32::NAN, 6.0);
        assert_eq!(wrapped_angle_word(7.0, 6.0), 1000);
    }
}
