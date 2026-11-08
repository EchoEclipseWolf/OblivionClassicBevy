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
use recomp_core::crt::{MsvcRand, RandomGlobals};
use recomp_core::pointer_list::PointerList;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::actor::{Actor, MobileObject, TesObjectRefr, actor_process_level};
use super::form::{FormIdentity, OriginalVtable};
use super::forms_world::{TesObjectCell, cell_has_type_0d};
use super::game_settings::GameSettingCollection;
use super::player::{PlayerCharacter, PlayerGlobals};
use super::tes::TesGlobals;
use crate::bs::task_manager::LockFreeMap;

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

// IDA: 0065C6A0 ActorValues_Clear: free each list item and pull the next node in until the head is empty; then 0065BCE0 on the +0x10 block and FormHeap_Free (none here). Completion: 100% of the reached arm.
// TODO: [ ] 0065BCE0 (the indexed block's entries) when a block exists (M4).
fn clear_actor_values(values: &mut ActorValues) -> Result<(), StreamError> {
    values.list_00.clear();
    if values.indexed_10.is_some() {
        return Err(StreamError(
            "0065C6EA: 0065BCE0 on an ActorValues block is not reached",
        ));
    }
    Ok(())
}

// IDA: the 18 codes 0065C010 / 0065C8F0 keep in the +0x10 indexed block.
const fn indexed_actor_value(code: u8) -> bool {
    matches!(
        code,
        0 | 4 | 5 | 6 | 7 | 8 | 11 | 13 | 26 | 33 | 36 | 40 | 41 | 46 | 47 | 48 | 49 | 56
    )
}

// IDA: 0065C010 ActorValues_GetEntry(code): 9 (+0x08), 10 (+0x0C), the indexed codes from the +0x10 block (null without one), the others by walking the list. Completion: 100%.
fn actor_value_entry(values: &mut ActorValues, code: u8) -> Option<&mut ActorValueEntry> {
    if code == 9 {
        return values.magicka_08.as_mut();
    }
    if code == 10 {
        return values.fatigue_0c.as_mut();
    }
    if indexed_actor_value(code) {
        // 0065C010 the block slot of each code (the +0x10 block offset / 4).
        let slot = match code {
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
            _ => 17, // 13
        };
        return values.indexed_10.as_mut()?.entries[slot].as_mut();
    }
    values.list_00.iter_mut().find(|item| item.code_00 == code)
}

/// IDA: 0065CB80 ActorValues_GetModifier(code): 0065C010's entry's +0x04, else 0.0. Completion: 100%.
#[must_use]
pub fn actor_values_get(values: &ActorValues, code: u8) -> f32 {
    if code == 9 {
        return values.magicka_08.map_or(0.0, |entry| entry.value_04);
    }
    if code == 10 {
        return values.fatigue_0c.map_or(0.0, |entry| entry.value_04);
    }
    if indexed_actor_value(code) {
        let slot = match code {
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
            _ => 17, // 13
        };
        return values
            .indexed_10
            .as_ref()
            .and_then(|block| block.entries[slot])
            .map_or(0.0, |entry| entry.value_04);
    }
    values
        .list_00
        .iter()
        .find(|item| item.code_00 == code)
        .map_or(0.0, |entry| entry.value_04)
}

// IDA: 0065CB00 ActorValues_SetModifier(code, value): with an entry, a zero value goes to 0065C9B0 (codes 9/10: the entry's value = 0.0) and another value is stored; without one, a nonzero value allocates
// {code, value} and inserts it (0065C8F0). Completion: 100% for codes 9/10 and the list prepend.
// TODO: [ ] 0065C8F0's indexed insert and the 0065BE70 / list-remove arms of 0065C9B0 (M4, Phase 14).
pub fn actor_values_set_modifier(
    values: &mut ActorValues,
    code: u8,
    value: f32,
) -> Result<(), StreamError> {
    if let Some(entry) = actor_value_entry(values, code) {
        if value == 0.0 && code != 9 && code != 10 {
            return Err(StreamError(
                "0065C9E9: a zero modifier on another code removes its entry (not ported)",
            ));
        }
        // 0065C9D1 / 0065C9DD fldz, or 0065CB72 fstp [entry+4].
        entry.value_04 = if value == 0.0 { 0.0 } else { value };
        return Ok(());
    }
    if value == 0.0 {
        return Ok(());
    }
    // 0065CB21 FormHeap(8) {code, value}, then 0065C8F0: a list code is prepended.
    if code == 9 || code == 10 || indexed_actor_value(code) {
        return Err(StreamError(
            "0065C8F0: inserting into the indexed ActorValues block is not modelled",
        ));
    }
    values.list_00.insert(
        0,
        ActorValueEntry {
            code_00: code,
            value_04: value,
        },
    );
    Ok(())
}

/// IDA: 0065CA60(code, delta, 1): the entry's value plus the delta, and an entry reaching 0 clears; none takes a new one through 0065C8F0 unless the delta is 0. Completion: list codes; the indexed block is a stop.
/// # Errors
/// An entry reaching 0, or an indexed code.
pub fn actor_values_add_modifier(
    values: &mut ActorValues,
    code: u8,
    delta: f32,
) -> Result<(), StreamError> {
    if let Some(entry) = actor_value_entry(values, code) {
        entry.value_04 += delta;
        if entry.value_04 == 0.0 {
            return Err(StreamError("0065CA60: ActorValues_ClearEntry"));
        }
        return Ok(());
    }
    actor_values_set_modifier(values, code, delta)
}

// IDA: 0065CD10 ActorValues_CopyFrom(source): 0065C6A0 clears this; 0065CB00(10, source fatigue or 0.0) and 0065CB00(9, source magicka or 0.0); each nonzero indexed source entry is copied (FormHeap 8) and
// inserted by 0065C8F0; then each source list item likewise. Completion: 100% of the reached arms (an empty source list and no indexed block).
// TODO: [ ] The indexed block and list item copies (M4, Phase 14).
pub fn copy_actor_values(
    destination: &mut ActorValues,
    source: Option<&ActorValues>,
) -> Result<(), StreamError> {
    // 0065CD14 0065C6A0 on this.
    clear_actor_values(destination)?;
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
    // 0065CD6B the indexed block's 18 entries.
    if source.indexed_10.is_some() {
        return Err(StreamError(
            "0065CD76: copying an indexed ActorValues block is not reached",
        ));
    }
    // 0065D107..0065D137 each source list item (none in Phase 11).
    if !source.list_00.is_empty() {
        return Err(StreamError(
            "0065D10E: copying ActorValues list entries is not reached",
        ));
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
    pub float_28: f32,
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
    /// +0x90 (unset by MiddleLow; HighProcess stores -1).
    pub field_90: i32,
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
    pub float_bc: f32,
    pub current_package_c0: u32,
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
    /// +0xF8 (-1.0).
    pub float_f8: f32,
    /// +0xFC..+0x110 the weapon's bone nodes (006541A0; zeroed by 00658550 with no animation data).
    pub weapon_nodes_fc: WeaponNodes,
    pub byte_114: u8,
    pub byte_115: u8,
    /// +0x118 NiPointer<bhkCharacterController> (vt+0x18C 006348E0 / vt+0x190 00634910); `tes::character_controller` keeps its +0x04 count.
    pub char_controller_118: Option<Entity>,
    /// +0x11C (vt+0x2E4 0064B080 reads it).
    pub knocked_state_11c: u8,
    /// +0x11D (vt+0x36C 0064B090 reads it).
    pub byte_11d: u8,
    pub field_120: u32,
    /// +0x124 (0x7F).
    pub byte_124: u8,
    /// +0x128..+0x130 (g_niPoint3Zero dwords).
    pub point_128: [u32; 3],
    /// +0x134 (006FAEE0).
    pub word_134: u16,
    pub byte_136: u8,
    /// +0x138 (0xFFFF).
    pub word_138: u16,
    pub field_13c: u32,
    pub field_140: u32,
    pub queued_magic_item_144: u32,
    pub field_148: u32,
    pub byte_14c: u8,
    pub field_150: u32,
    /// +0x154 (1.0).
    pub actor_alpha_154: f32,
    pub float_158: f32,
    pub field_15c: u32,
    pub byte_160: u8,
    pub byte_161: u8,
    pub field_164: u32,
    pub bytes_168: [u8; 4],
    /// +0x16C an equipment 3D update pending: vt+0x31C (006295E0) stores it, from an equip (0048A113) and around 005268D0; 0063CDC0 rebuilds the actor's equipment 3D while it's set and the actor has a node, then clears it.
    pub equipment_update_pending_16c: u8,
    pub byte_16d: u8,
    pub field_170: u32,
    /// +0x174 effect list head block (FormHeap 8 {0, 0}).
    pub effect_list_174: Option<[u32; 2]>,
    pub field_178: u32,
    /// +0x17C the ActorAnimData (0064FFF0 sets it; `actor_animation`).
    pub anim_data_17c: Option<Entity>,
    pub byte_180: u8,
    /// +0x184 the skin's shade property (NiPointer, vt+0x3A8 006349D0: 005EA1A0's find under the skin attachment node); 0 for
    /// none, the only value the port stores.
    pub skin_property_184: u32,
    pub bounding_box_188: u32,
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
    pub float_1ac: f32,
    pub float_1b0: f32,
    pub float_1b4: f32,
    pub float_1b8: f32,
    pub fields_1bc: [u32; 4],
    pub field_1cc: u32,
    pub byte_1d0: u8,
    pub byte_1d1: u8,
    pub field_1d4: u32,
    pub float_1d8: f32,
    pub float_1dc: f32,
    pub float_1e0: f32,
    pub byte_1e4: u8,
    pub float_1e8: f32,
    pub field_1ec: u32,
    pub field_1f0: u32,
    /// +0x1F4 (0xFFFF).
    pub current_action_1f4: u16,
    pub field_1f8: u32,
    pub movement_flags_1fc: u16,
    pub field_200: u32,
    pub float_204: f32,
    /// +0x208 (0xFFFF).
    pub word_208: u16,
    /// +0x20C..+0x214 (g_niPoint3Zero dwords).
    pub point_20c: [u32; 3],
    pub field_218: u32,
    pub float_21c: f32,
    pub fields_220: [u32; 2],
    pub byte_228: u8,
    pub float_22c: f32,
    /// +0x230 (Random_Next % 5000 * 0.001 + 10).
    pub float_230: f32,
    pub float_234: f32,
    /// +0x238 (20.0).
    pub swim_breath_238: f32,
    /// +0x23C (1).
    pub byte_23c: u8,
    pub float_240: f32,
    pub byte_244: u8,
    pub float_248: f32,
    pub byte_24c: u8,
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
    pub field_288: u32,
    pub float_28c: f32,
    pub byte_290: u8,
    /// +0x294 (-1.0).
    pub float_294: f32,
    /// +0x298..+0x2A0 (-1).
    pub fields_298: [i32; 3],
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

/// IDA: BSSimpleList<Actor*> with an embedded head node (+0x00 item, +0x04 next). The tail and cursors hold nodes, which `recomp_core::pointer_list::PointerList` does not expose. Completion: 100%.
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
        let mut out = Vec::new();
        let mut node = Some(ActorListNode::HEAD);
        while let Some(current) = node {
            if let Some(item) = self.item(current) {
                out.push(item);
            }
            node = self.next(current);
        }
        out
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

/// IDA: N/A which manager list a cursor's node belongs to, and the 00673A90 / 00674550 level switch (0 High +0x68, 1 +0x00, 2 +0x0C, 3 +0x18).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActorListId {
    MiddleHigh00,
    Low0c,
    Low18,
    High68,
}

impl ActorListId {
    /// IDA: N/A the 00673A90 / 00674550 switch: other levels have no list.
    #[must_use]
    pub const fn for_level(level: i32) -> Option<Self> {
        match level {
            0 => Some(Self::High68),
            1 => Some(Self::MiddleHigh00),
            2 => Some(Self::Low0c),
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
    pub low_0c: ActorProcessList,
    /// Level 3.
    pub low_18: ActorProcessList,
    pub field_28: u32,
    /// +0x40 NiPointer pair (00532DF0(0)).
    pub pointer_40: [u32; 2],
    pub pointer_48: [u32; 2],
    /// +0x50..+0x64.
    pub fields_50: [u32; 6],
    /// IDA: +0x58 the list 006748B0 prepends to, held apart from `fields_50`, which keeps the constructor's zeros for the Phase 11 image.
    pub queued_58: PointerList<Entity>,
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
    /// +0xA0..+0xA6.
    pub bytes_a0: [u8; 7],
    pub field_a8: u32,
    /// +0xAC GMST fRemoveExcessComplexDeadTime.
    pub remove_excess_dead_time_ac: f32,
}

impl ActorProcessManager {
    /// IDA: N/A a list by id.
    #[must_use]
    pub const fn list(&self, id: ActorListId) -> &ActorProcessList {
        match id {
            ActorListId::MiddleHigh00 => &self.middle_high_00,
            ActorListId::Low0c => &self.low_0c,
            ActorListId::Low18 => &self.low_18,
            ActorListId::High68 => &self.high_68,
        }
    }

    /// IDA: N/A a list by id.
    pub const fn list_mut(&mut self, id: ActorListId) -> &mut ActorProcessList {
        match id {
            ActorListId::MiddleHigh00 => &mut self.middle_high_00,
            ActorListId::Low0c => &mut self.low_0c,
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
    /// 00B3BCF0, the process time 00673B00 reads (BSS, 0.0).
    /// TODO: [ ] the per-frame advance, in the process manager's frame, which isn't ported.
    pub time_seconds_b3bcf0: f32,
    /// 00B3F9A0 two pi (00A09BD0) read by 006FAEE0.
    pub two_pi_b3f9a0: f32,
    /// 00B069C3 Random_Next's first-use flag, and `recomp_core::crt` holds its logic.
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
    world.insert_resource(ActorProcessGlobals {
        manager: ActorProcessManager {
            // 0067AF8B/0067AF99/0067AFA6 0067B240 on +0x00, +0x0C, +0x18: {0, 0, self}.
            middle_high_00: ActorProcessList::default(),
            low_0c: ActorProcessList::default(),
            low_18: ActorProcessList::default(),
            field_28: 0,
            // 0067AFAB..0067AFD2 00532DF0(0) on +0x40 and +0x48: {0, 0}.
            pointer_40: [0; 2],
            pointer_48: [0; 2],
            // 0067AFD7..0067AFEE +0x50..+0x64 = 0.
            fields_50: [0; 6],
            queued_58: PointerList::default(),
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
            bytes_a0: [1, 1, 1, 1, 1, 0, 0],
            field_a8: 0,
            remove_excess_dead_time_ac,
        },
        high_process_b3b928: 0,
        time_seconds_b3bcf0: 0.0,
        two_pi_b3f9a0: f32::from_bits(0x40C9_0FDB),
        random: RandomGlobals::default(),
    });
}

/// IDA: N/A the host seam of 0047DF80 Random_Next: 0047DFA8 _time64(0), 0047DF89/0047DFAE _srand and 0047DF98/0047DFBD _rand. A test injects the run's draw, since P11-RUN-001's _time64 is recorded later than the seed it fits.
pub trait RandomSource {
    /// IDA: 0047DFA8 _time64(0).
    fn time64(&mut self) -> i64;
    /// IDA: _srand.
    fn srand(&mut self, seed: u32);
    /// IDA: _rand.
    fn rand(&mut self) -> i32;
}

/// IDA: N/A the default `RandomSource`: the `recomp_core::crt` LCG over a host clock.
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

// IDA: 0047DF80 Random_Next(seed) (__cdecl): a nonzero seed: srand(seed), 00B069C3 = 0; otherwise, while 00B069C3 is set, srand(_time64(0)) and 00B069C3 = 0; then rand(). `recomp_core::crt::random_next`
// over a seam, because a test injects the CRT's result. Completion: 100%.
pub fn random_next(seed: u32, globals: &mut RandomGlobals, source: &mut dyn RandomSource) -> i32 {
    if seed != 0 {
        source.srand(seed);
        globals.first_use_b069c3 = 0;
    } else if globals.first_use_b069c3 != 0 {
        let time = source.time64();
        source.srand(time as u32);
        globals.first_use_b069c3 = 0;
    }
    source.rand()
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
fn build_low_process(inputs: &ProcessInputs<'_>) -> (BaseProcess, LowProcess) {
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
    p.float_28 = 0.0;
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
    p.unset_88 = inputs.low_process_unset_88;
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
    p.skin_property_184 = 0;
    // 0064B46D..0064B4E3 in store order.
    p.float_f8 = -1.0;
    p.queued_magic_item_144 = 0;
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
    p.field_140 = 0;
    // 0064B4E9..0064B506 FormHeap(8) {0, 0} -> +0x174.
    p.effect_list_174 = Some([0; 2]);
    // 0064B50C..0064B542.
    low.pathing_34 = 0;
    p.current_package_c0 = 0;
    p.procedure_type_cc = 0;
    p.anim_data_17c = None;
    p.byte_114 = 0;
    p.byte_115 = 0;
    p.knocked_state_11c = 0;
    low.field_38 = 0;
    p.byte_11d = 0;
    p.field_120 = 0;
    p.byte_124 = 0x7F;
    // 0064B549 006FAEE0(0.0) on +0x128 (writes +0x134).
    p.word_134 = wrapped_angle_word(0.0, two_pi);
    // 0064B550..0064B575 +0x136 = 0, +0x128..+0x130 = g_niPoint3Zero, +0xB8 = 0.0, +0x180 = 0.
    p.byte_136 = 0;
    p.point_128 = [0; 3];
    p.float_b8 = 0.0;
    p.byte_180 = 0;
    // 0064B57B..0064B5A5 release +0x184 (null: skipped).
    // 0064B5AB..0064B62C in store order.
    p.float_bc = 0.0;
    p.field_e0 = 0;
    p.float_c4 = 0.0;
    p.field_148 = 0;
    p.byte_14c = 0;
    p.actor_alpha_154 = 1.0;
    p.field_150 = 0;
    p.field_15c = 0;
    p.field_164 = 0;
    p.float_158 = 0.0;
    p.byte_161 = 0;
    // 0064B5EF fstp [esi+88h].
    low.unset_88 = 0.0_f32.to_bits();
    p.byte_c8 = 1;
    p.bytes_168[0] = 0;
    p.bytes_168[1] = 0;
    p.byte_160 = 0;
    p.bounding_box_188 = 0;
    p.bytes_168[2] = 0;
    p.bytes_168[3] = 0;
    p.equipment_update_pending_16c = 0;
    p.byte_16d = 0;
    p.field_170 = 0;
    p
}

// IDA: 006438F0 LowProcess_ctor (FormHeap 0x90). Completion: 100% of the store order.
pub fn construct_low_process(world: &mut World, inputs: &ProcessInputs<'_>) -> Entity {
    let (base, low) = build_low_process(inputs);
    world
        .spawn((OriginalVtable(LOW_PROCESS_VTABLE), base, low))
        .id()
}

// IDA: 00628EE0 HighProcess_ctor (FormHeap 0x2EC; 00658690 and 0064B400 inside). x87: 24-bit, plain f32. Completion: 100% of the store order.
#[expect(
    clippy::field_reassign_with_default,
    reason = "00628EE0's stores stay one statement each, in original store order"
)]
pub fn construct_high_process(world: &mut World, inputs: &mut ProcessInputs<'_>) -> Entity {
    let (base, mut low) = build_low_process(inputs);
    // 00658690 MiddleLowProcess_ctor: vtable A72D64, 0065BE10 on +0x94.
    let mut middle_low = MiddleLowProcess {
        field_90: 0,
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
    middle_low.field_90 = -1;
    globals.high_process_b3b928 = 0;
    // 00628F50..00629019 in store order.
    p.fields_1bc = [0; 4];
    p.float_1d8 = 0.0;
    p.float_1dc = 0.0;
    p.movement_flags_1fc = 0;
    p.float_1ac = 0.0;
    p.field_200 = 0;
    p.float_204 = 0.0;
    p.word_208 = 0xFFFF;
    p.float_198 = 0.0;
    p.field_1a4 = 0;
    p.field_1ec = 0;
    p.field_1f0 = 0;
    p.current_action_1f4 = 0xFFFF;
    p.field_1f8 = 0;
    p.field_218 = 0;
    p.byte_1d0 = 0;
    p.fields_220 = [0; 2];
    p.point_20c[0] = 0;
    p.point_20c[1] = 0;
    p.float_21c = 0.0;
    p.float_22c = 0.0;
    p.point_20c[2] = 0;
    p.byte_228 = 0;
    p.byte_23c = 1;
    p.byte_244 = 0;
    p.field_250 = 0;
    // 0062901F Random_Next(0); 00629024..00629054 the remainder of 5000, then +0x230 = fild(r) * 0.001 + 10.0 at 24-bit precision (dbl_A30E40 is float 0.001 widened).
    let random = random_next(0, &mut globals.random, &mut *inputs.random);
    let remainder = random % 5000;
    p.byte_24c = 0;
    p.byte_25c = 0;
    p.field_1cc = 0;
    p.float_230 = remainder as f32 * 0.001 + 10.0;
    p.float_248 = 0.0;
    p.float_234 = 0.0;
    p.float_1e8 = 0.0;
    p.float_1b4 = 0.0;
    p.float_1b0 = 0.0;
    // 0062907A..00629095 FormHeap(8) {0, 0} -> +0x18C.
    p.detection_list_18c = Some([0; 2]);
    // 0062908F..00629120 in store order.
    p.swim_breath_238 = 20.0;
    p.byte_1e4 = 0;
    p.field_2bc = 1;
    p.float_1a8 = 0.0;
    p.field_2c4 = 0;
    p.float_1e0 = 0.0;
    p.byte_25d = 0;
    p.float_240 = 0.0;
    p.float_2c0 = 0.0;
    p.float_1b8 = 0.0;
    p.point_27c = [0; 3];
    p.float_260 = 0.0;
    // 00629106 [00B36940].
    p.float_19c = x87::float_copy(inputs.idle_chatter_comment_timer);
    p.field_288 = 0;
    p.float_264 = 0.0;
    p.float_26c = 1.0;
    p.float_270 = 0.0;
    // 0062912C..00629156 release +0x268 (null: skipped).
    // 0062915C..006291F6 in store order.
    p.float_28c = 0.0;
    p.field_274 = 0;
    p.byte_278 = 0;
    p.float_294 = -1.0;
    p.byte_290 = 0;
    p.fields_298 = [-1; 3];
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
    p.byte_1d1 = 0;
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

// IDA: vt+8 GetProcessLevel: LowProcess 0073FD50 returns 3, HighProcess 006F7070 0. MiddleLow / MiddleHigh processes are not built in Phase 11. Completion: 100% of the two built classes.
// TODO: [ ] The level of a standalone MiddleLow / MiddleHigh process, when one is built (M4).
#[must_use]
pub fn process_level(world: &World, process: Entity) -> i32 {
    match world.get::<OriginalVtable>(process) {
        Some(OriginalVtable(HIGH_PROCESS_VTABLE)) => 0,
        _ => 3,
    }
}

// IDA: vt+4 0060CDB0 BaseProcess_CopyFrom(old) on the new process: the RTTI casts of both, the High/MiddleHigh/MiddleLow arms (not reached from a LowProcess) and 00643520 for the Low part. Completion: 100% of
// the reached arms.
// TODO: [ ] The High, MiddleHigh (0064B650) and MiddleLow (00658760) copy arms (M4 or later).
pub fn copy_process_from(
    world: &mut World,
    process: Entity,
    old_process: Option<Entity>,
) -> Result<(), StreamError> {
    // 0060CDB9 a null old process returns.
    let Some(old_process) = old_process else {
        return Ok(());
    };
    // 0060CE81..0060CEBA the old High arm.
    if world.get::<HighProcess>(old_process).is_some() {
        return Err(StreamError(
            "0060CE87: copying from a HighProcess is not reached",
        ));
    }
    // 0060CEBC..0060CEF3 the new High arm: vt+0x15C (00629800) > 0.0 runs vt+0x178(0) and vt+0xD0(0); 006299F0 only with an old High process. 00629800 returns +0x248 (0.0 after the constructor): `fcomp
    // 0.0; test ah, 41h` continues only for an ordered value above zero.
    if world
        .get::<HighProcess>(process)
        .is_some_and(|high| high.float_248 > 0.0)
    {
        return Err(StreamError(
            "0060CEA0: vt+0x178 (0064AF90) / vt+0xD0 (0064AF50) are not reached",
        ));
    }
    // 0060CF01..0060CF0E MiddleHigh 0064B650 (both MiddleHigh).
    if world.get::<MiddleHighProcess>(process).is_some()
        && world.get::<MiddleHighProcess>(old_process).is_some()
    {
        return Err(StreamError(
            "0060CF0E: 0064B650 (MiddleHigh copy) is not reached",
        ));
    }
    // 0060CF13..0060CF26 MiddleLow 00658760 (both MiddleLow).
    if world.get::<MiddleLowProcess>(process).is_some()
        && world.get::<MiddleLowProcess>(old_process).is_some()
    {
        return Err(StreamError(
            "0060CF26: 00658760 (MiddleLow copy) is not reached",
        ));
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
    n.float_28 = o.float_28;
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
        return Err(StreamError(
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

// IDA: vt+0 0064ACD0 LowProcess deleting destructor (flag 1): 00648CB0 (the owned-object, FormHeap-block and list releases, 0065CCC0 on +0x70, 0060CDA0) then FormHeap_Free. A +0x08 package without flag 0x800 (005660A0) is only dropped, being the form's own. Completion: 100% of the reached
// arms; an owned +0x34 / +0x64 object or a flag 0x800 package stops.
// TODO: [ ] The release arms, when a process owns something (M4).
pub fn delete_low_process(world: &mut World, process: Entity) -> Result<(), StreamError> {
    let (Some(low), Some(base)) = (
        world.get::<LowProcess>(process),
        world.get::<BaseProcess>(process),
    ) else {
        return Err(StreamError(
            "0064ACD0: only a LowProcess deleting destructor is reached",
        ));
    };
    // 00648CE1 an owned +0x34 object (vt+0(1)).
    if low.pathing_34 != 0 {
        return Err(StreamError(
            "00648CE1: releasing LowProcess objects is not reached",
        ));
    }
    // 00648CFA a +0x08 package with flag 0x800: TESSaveLoadGame 0045C7A0 or its vt+0x10(1).
    if base.editor_package_08.is_some_and(|package| {
        world
            .get::<super::forms_ai::TesPackage>(package)
            .is_some_and(|package| package.flags_1c & 0x800 != 0)
    }) {
        return Err(StreamError("00648D01: freeing a flag 0x800 package"));
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
        return Err(StreamError(
            "00648D3A: releasing LowProcess blocks is not reached",
        ));
    }
    // 00648E10 0065CCC0 on +0x70: 0065C6A0, then the +0x08 and +0x0C entries freed. 00648E1F 0060CDA0 BaseProcess vtable; 0064ACDF FormHeap_Free.
    let mut low = world.get_mut::<LowProcess>(process).expect("checked above");
    clear_actor_values(&mut low.damage_modifiers_70)?;
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
) -> Result<(), StreamError> {
    // 00673A95..00673AB6 the level list; 00673ABE 00659A00.
    let Some(id) = ActorListId::for_level(level) else {
        return Ok(());
    };
    if actor_process_level(world, actor) != level {
        return Ok(());
    }
    // 0067B260 ActorProcessList_Add. 0067B266 g_thePlayer is never added.
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
        return Err(StreamError("0067B2CB: neighbour insertion is not reached"));
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
    Ok(())
}

// IDA: 0043F8C0 TES_IsCellInGridRange(cell, flag), which reads only the cell: a null cell, or a signed +0x26 outside 2..=6, is false; 5 and 6 are true; 2, 3 and 4 are true only with a clear flag (table 0043F8EC). Completion: 100%.
fn cell_level_test(world: &World, cell: Option<Entity>, flag: bool) -> bool {
    let Some(cell) = cell else {
        return false;
    };
    match world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .process_level_26 as i8
    {
        5 | 6 => true,
        2..=4 => !flag,
        _ => false,
    }
}

// IDA: 0065D550 on g_thePlayer: +0x590 above 0. A null player faults.
fn player_is_sleeping(world: &World) -> Result<bool, StreamError> {
    world
        .get_resource::<PlayerGlobals>()
        .and_then(|globals| globals.the_player_b333c4)
        .and_then(|player| world.get::<PlayerCharacter>(player))
        .map(|player| player.pc_sleep_hours_590 > 0)
        .ok_or(StreamError("0065D557: a null g_thePlayer faults"))
}

// IDA: 0065AD00 Actor_GetDesiredProcessLevel. With no active save and the player sleeping, the process's level, or -1 with none. An active save, a 3D, or 004354F0 ModelLoader_IsReferenceQueued (the model loader 00B33A1C's +0x08 map of reference tasks holds the actor) runs the grid tests: 0043F8C0(cell, 1) is 0 and (cell, 0) is 1. Otherwise a cell with the type 0x0D extra (004C9FB0), or 0043F8C0(cell, 0), is 2; else 3. Completion: 100% of the arms a load reaches.
pub(crate) fn actor_desired_level(world: &World, actor: Entity) -> Result<i32, StreamError> {
    let flags = world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .map(|game| game.fields_14[1])
        .ok_or(StreamError("0065AD09: a null g_saveLoadGame faults"))?;
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
        if cell_level_test(world, cell, true) {
            return Ok(0);
        }
        if cell_level_test(world, cell, false) {
            return Ok(1);
        }
    }
    if cell.is_some_and(|cell| cell_has_type_0d(world, cell)) || cell_level_test(world, cell, false)
    {
        return Ok(2);
    }
    Ok(3)
}

/// IDA: 006748B0 on g_actorProcessManager: an actor already in the +0x58 list, or with no process, returns. A death state (vt+0x198(0) 005E33B0: +0xB0 of 1, 2 or 6), a process level other than 0065AD00's, the process's +0x20 byte (vt+0x248 0051AAD0), flag 0x20 or a sleeping player sets flag 0x20000 (vt+0xA0(1) 0046AB00) and prepends the actor; otherwise it returns. Every SetDeleted(0) on an actor gets here. The C++ never runs it. Completion: 100% of the arms a load reaches.
pub fn actor_process_manager_refresh(world: &mut World, actor: Entity) -> Result<(), StreamError> {
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
    if !actor_process_list_remove(manager, id, actor, None) {
        return false;
    }
    // 0067B403..0067B422 flag 0x20 and no active save/load: 00659BC0 deletes the process: +0x58 vt+0(1) (the deleting destructor), then +0x58 = 0. Host: the Low / MiddleLow destructors free only their own members.
    let deleted = world
        .get::<FormIdentity>(actor)
        .is_some_and(|identity| identity.flags_08 & 0x20 != 0);
    if deleted && !save_load_active {
        let mut mobile = world.get_mut::<MobileObject>(actor);
        if let Some(process) = mobile.as_mut().and_then(|mobile| mobile.process_58.take()) {
            world.despawn(process);
        }
    }
    true
}

// IDA: 0067B320 ActorProcessList_Remove(actor, start) (__thiscall on the list): the High-list two-item and cursor clears, the search from `start` (null: the head), the unlink (a match at the head pulls the next
// node in). Returns whether the actor was found. `actor_process_manager_remove` runs the flag-0x20 process release of 0067B403. Completion: 100%.
pub fn actor_process_list_remove(
    manager: &mut ActorProcessManager,
    id: ActorListId,
    actor: Entity,
    start: Option<ActorListNode>,
) -> bool {
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
    if let Some(previous) = previous {
        // 0067B3C2..0067B3D3 RemoveFirstMatchingItem from the previous node; a removed tail moves the tail back.
        list.list.remove_first_matching(previous, actor);
        if node == list.tail_08 {
            list.tail_08 = previous;
        }
    } else {
        // 0067B3D8..0067B400 the head: pull the next node in (FormHeap_Free) or clear it; without a next node the tail is the head.
        list.list.pull_next_in(ActorListNode::HEAD);
        if list.list.next(ActorListNode::HEAD).is_none() {
            list.tail_08 = ActorListNode::HEAD;
        }
    }
    true
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
