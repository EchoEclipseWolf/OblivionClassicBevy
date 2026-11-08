//! Phase 11 (D1): the reference/actor base chain of PlayerCharacter_ctor 0066A740: Character_ctor 0060E4F0 -> Actor_ctor 005E15E0 -> MobileObject_ctor 00659990 -> TESObjectREFR_ctor 004D9A70 ->
//! TESForm_Constructor 0046C460 (`tes::data_handler`; its DataHandler arm runs for the first time here).
//!
//! A form is an entity. Each class level is a capability component named after the class, holding that level's fields with their offsets in the names. The four vtable identities per level sit where the
//! object has them: main in `OriginalVtable`, the TESChildCell +0x18 in `TesObjectRefr`, MagicCaster +0x5C and MagicTarget +0x68 in `Actor`. Bytes the constructors do not store are host zero (uninitialized
//! FormHeap bytes in the original, recorded substitution). Evidence: evidence/phase11-window-2026-10-06/disasm-chain.txt; P11-RUN-001.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

pub use super::base_extra_list::ExtraDataList;

use super::actor_process::{
    ActorValues, MiddleHighProcess, ProcessInputs, actor_process_manager_add,
    construct_actor_values, construct_low_process, process_level,
};
use super::data_handler::construct_tes_form;
use super::form::{FormIdentity, OriginalVtable};
use super::forms_references::set_reference_rotation_z;
use super::tes::TesGlobals;

/// IDA: vtables of the chain (??_7<Class>@@6B@, _0 the TESChildCell part at +0x18, _1 the MagicCaster part at +0x5C, _2 the MagicTarget part at +0x68).
pub const TES_CHILD_CELL_VTABLE: u32 = 0x00A4_5E54;
pub const TES_OBJECT_REFR_VTABLE: u32 = 0x00A4_6C44;
pub const TES_OBJECT_REFR_CHILD_CELL_VTABLE: u32 = 0x00A4_6C38;
pub const MOBILE_OBJECT_VTABLE: u32 = 0x00A7_32FC;
pub const MOBILE_OBJECT_CHILD_CELL_VTABLE: u32 = 0x00A7_32F4;
pub const MAGIC_CASTER_VTABLE: u32 = 0x00A7_5DEC;
pub const MAGIC_TARGET_VTABLE: u32 = 0x00A7_67BC;
pub const EXTRA_DATA_LIST_VTABLE: u32 = 0x00A3_50F8;
pub const ACTOR_VTABLES: [u32; 4] = [0x00A6_E074, 0x00A6_E06C, 0x00A6_E024, 0x00A6_DFF4];
pub const CHARACTER_VTABLES: [u32; 4] = [0x00A6_FC9C, 0x00A6_FC94, 0x00A6_FC4C, 0x00A6_FC1C];
pub const CREATURE_VTABLES: [u32; 4] = [0x00A7_10F4, 0x00A7_10EC, 0x00A7_10A4, 0x00A7_1074];
/// IDA: form types (00B05E00 table).
pub const FORM_TYPE_REFERENCE: u8 = 0x31;
pub const FORM_TYPE_CHARACTER: u8 = 0x32;
pub const FORM_TYPE_CREATURE: u8 = 0x33;
/// IDA: TESForm flags the chain sets or clears: 0046AB80 clears 8, 005E167F sets 0x200000, 0066B06D sets 0x400.
pub const FORM_FLAG_INITIALIZED: u32 = 0x8;
pub const FORM_FLAG_ACTOR: u32 = 0x0020_0000;
pub const FORM_FLAG_PLAYER_INIT: u32 = 0x400;
/// IDA: 00B3F9A8 g_niPoint3Zero as raw dwords ({0, 0, 0} after its cinit).
pub const NI_POINT3_ZERO: [u32; 3] = [0; 3];

/// IDA: 004D9A70 TESObjectREFR_Constructor (0x58).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesObjectRefr {
    /// +0x18 TESChildCell vtable.
    pub child_cell_vtable_18: u32,
    pub base_form_1c: Option<Entity>,
    /// +0x20..+0x28.
    pub rotation_20: [f32; 3],
    /// +0x2C..+0x34.
    pub position_2c: [f32; 3],
    pub scale_38: f32,
    /// +0x3C NiNode* (null; 004D6BF0 releases it).
    pub loaded_3d_3c: u32,
    /// +0x40 TESObjectCELL* (004D8AA0 sets it).
    pub parent_cell_40: Option<Entity>,
    /// +0x44..+0x57.
    pub extra_data_44: ExtraDataList,
}

/// IDA: 00659990 MobileObject_Constructor (0x5C): TESObjectREFR and the two vtables; +0x58 is the process (BaseProcess*) Actor_ctor stores.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MobileObject {
    pub process_58: Option<Entity>,
}

/// IDA: 00699140 MagicCaster_Constructor (0x0C at Actor +0x5C): vtable A75DEC, +0x08 = +0x04 = 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MagicCaster {
    pub original_vtable: u32,
    pub magic_node_04: u32,
    pub state_08: u32,
}

/// IDA: 006A1D50 MagicTarget_Constructor (8 at Actor +0x68): vtable A767BC, +0x04 byte = 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MagicTarget {
    pub original_vtable: u32,
    pub byte_04: u8,
}

/// IDA: an entry of Actor +0xA4, the 8-byte block 005E2070 makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispositionEntry {
    pub modifier: i32,
    pub reference: Entity,
}

/// IDA: 005E15E0 Actor_Constructor (0x104; Actor_mbrs).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Actor {
    pub magic_caster_5c: MagicCaster,
    pub magic_target_68: MagicTarget,
    /// +0x70 (7).
    pub expression_override_70: u32,
    pub expression_override_strength_74: f32,
    /// +0x78 (1).
    pub processing_enabled_78: u8,
    pub death_killer_7c: u32,
    pub murder_candidate_80: u8,
    /// +0x84 (not stored).
    pub unset_check_dead_body_timer_84: f32,
    /// +0x88..+0x9B.
    pub av_modifiers_88: ActorValues,
    pub greater_power_list_9c: [u32; 2],
    pub disposition_modifiers_a4: [u32; 2],
    /// +0xA4's entries, as a list.
    pub disposition_list: Vec<DispositionEntry>,
    pub combat_target_reevaluation_timer_ac: f32,
    pub dead_state_b0: u32,
    pub reserved_b4: [u32; 2],
    /// +0xBC (-1.0).
    pub last_process_update_time_bc: f32,
    pub deferred_death_processing_c0: u8,
    pub render_visibility_flags_c4: u32,
    pub force_run_c8: u8,
    pub force_sneak_c9: u8,
    /// +0xCA (1).
    pub allow_quest_process_reschedule_ca: u8,
    pub pending_movement_blocked_ref_cc: u32,
    pub leveled_creature_template_d0: u32,
    pub horse_or_rider_d4: u32,
    /// +0xD8 (not stored).
    pub unset_in_water_d8: u8,
    pub head_track_target_hold_timer_dc: f32,
    /// +0xE0 (1).
    pub head_track_visibility_active_e0: u8,
    pub pending_dialogue_target_e4: u32,
    /// +0xE8..+0xF0 (g_niPoint3Zero dwords).
    pub starting_position_e8: [u32; 3],
    pub starting_z_rotation_f4: f32,
    /// +0xF8 (005EA790: the cell or world).
    pub starting_location_f8: Option<Entity>,
    pub death_effects_processed_fc: u8,
    /// +0xFD (1).
    pub merchant_respawn_day_processed_fd: u8,
    pub head_track_target_scan_timer_100: f32,
}

/// IDA: 0060E4F0 Character_Constructor (0x10C; Character_mbrs).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct Character {
    /// +0x104 the skin info (`character_3d::SkinInfo`, 00478730's 0x154 object).
    pub skin_info_104: Option<Entity>,
    /// +0x108 (-1.0).
    pub armor_rating_cache_108: f32,
}

/// IDA: 00625100 Creature_Constructor (0x108): Actor and a byte at +0x104.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Creature {
    pub byte_104: u8,
}

// IDA: 004D9A70 TESObjectREFR_Constructor: TESForm_Constructor, +0x18 TESChildCell then REFR vtables, +0x3C = 0, 00422EE0 on +0x44, +0x04 = 0x31, 004D6BF0 (+0x38 = 1.0; +0x3C released unless g_saveLoadGame +0x18 has
// bit 2; 0046B090 for type 0x31 is a no-op case), then +0x1C..+0x34 and +0x40 = 0. Completion: 100% of the store order; the 004D6C13 release stops.
#[expect(
    clippy::field_reassign_with_default,
    reason = "004D9A70's stores stay one statement each, in original store order"
)]
pub fn construct_reference(world: &mut World) -> Result<Entity, StreamError> {
    // 004D9A98 TESForm_Constructor (the DataHandler arm runs once g_dataHandler is published).
    let form = construct_tes_form(world)?;
    // 004D9A9D..004D9AAA +0x18 TESChildCell, +0x00 / +0x18 the TESObjectREFR vtables.
    let mut reference = TesObjectRefr::default();
    reference.child_cell_vtable_18 = TES_CHILD_CELL_VTABLE;
    world
        .get_mut::<OriginalVtable>(form)
        .expect("just spawned")
        .0 = TES_OBJECT_REFR_VTABLE;
    reference.child_cell_vtable_18 = TES_OBJECT_REFR_CHILD_CELL_VTABLE;
    // 004D9AB9 +0x3C = 0; 004D9AC8 00422EE0 on +0x44.
    reference.loaded_3d_3c = 0;
    reference.extra_data_44.entries.clear();
    reference.extra_data_44.presence_08 = [0; 12];
    reference.extra_data_44.original_vtable = EXTRA_DATA_LIST_VTABLE;
    // 004D9AD4 +0x04 = 0x31 (REFR).
    world
        .get_mut::<FormIdentity>(form)
        .expect("just spawned")
        .form_type_04 = FORM_TYPE_REFERENCE;
    // 004D9AD8 004D6BF0: +0x38 = 1.0; unless g_saveLoadGame +0x18 has bit 2 a nonzero +0x3C is released and cleared (null here); then j_TESForm_ResetComponents (0046B090: type 0x31 is in the no-op case group 13,
    // 49..52, 54, 61). A world with no g_saveLoadGame reads as flags 0.
    reference.scale_38 = 1.0;
    let save_load_flags_18 = world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .map_or(0, |game| game.fields_14[1]);
    let keep_3d = save_load_flags_18 >> 2 & 1 != 0;
    if !keep_3d && reference.loaded_3d_3c != 0 {
        return Err(StreamError(
            "004D6C13: releasing a loaded 3D is not reached",
        ));
    }
    // 004D9ADD..004D9AF4 +0x1C..+0x34 and +0x40 = 0.
    reference.base_form_1c = None;
    reference.rotation_20 = [0.0; 3];
    reference.position_2c = [0.0; 3];
    reference.parent_cell_40 = None;
    world.entity_mut(form).insert(reference);
    Ok(form)
}

// IDA: 005E15E0 Actor_Constructor (after 00659990 MobileObject_Constructor). The LowProcess and the level-3 add (00673A90) run here. Completion: 100% of the store order.
pub fn construct_actor(
    world: &mut World,
    inputs: &ProcessInputs<'_>,
) -> Result<Entity, StreamError> {
    // 005E160D 00659990 MobileObject_Constructor: 004D9A70, then the two vtables.
    let form = construct_reference(world)?;
    world
        .get_mut::<OriginalVtable>(form)
        .expect("just spawned")
        .0 = MOBILE_OBJECT_VTABLE;
    world
        .get_mut::<TesObjectRefr>(form)
        .expect("just inserted")
        .child_cell_vtable_18 = MOBILE_OBJECT_CHILD_CELL_VTABLE;
    world.entity_mut(form).insert(MobileObject::default());
    let mut actor = Actor::default();
    // 005E161D 00699140 on +0x5C: vtable, +0x08 = 0, +0x04 = 0.
    actor.magic_caster_5c.original_vtable = MAGIC_CASTER_VTABLE;
    actor.magic_caster_5c.state_08 = 0;
    actor.magic_caster_5c.magic_node_04 = 0;
    // 005E162C 006A1D50 on +0x68: vtable, +0x04 byte = 0.
    actor.magic_target_68.original_vtable = MAGIC_TARGET_VTABLE;
    actor.magic_target_68.byte_04 = 0;
    // 005E163C..005E164F the four Actor vtables.
    world
        .get_mut::<OriginalVtable>(form)
        .expect("just spawned")
        .0 = ACTOR_VTABLES[0];
    world
        .get_mut::<TesObjectRefr>(form)
        .expect("just inserted")
        .child_cell_vtable_18 = ACTOR_VTABLES[1];
    actor.magic_caster_5c.original_vtable = ACTOR_VTABLES[2];
    actor.magic_target_68.original_vtable = ACTOR_VTABLES[3];
    // 005E1656 0065BE10 on +0x88.
    actor.av_modifiers_88 = construct_actor_values();
    // 005E165B..005E1679 +0x9C, +0xA0, +0xA4, +0xA8, +0xB4, +0xB8 = 0.
    actor.greater_power_list_9c = [0; 2];
    actor.disposition_modifiers_a4 = [0; 2];
    actor.reserved_b4 = [0; 2];
    // 005E167F flags |= 0x200000.
    world
        .get_mut::<FormIdentity>(form)
        .expect("just spawned")
        .flags_08 |= FORM_FLAG_ACTOR;
    // 005E1686..005E16C0 FormHeap(0x90) LowProcess_ctor -> +0x58 (the store follows the pushes of the 00673A90 arguments).
    let process = construct_low_process(world, inputs);
    world
        .get_mut::<MobileObject>(form)
        .expect("just inserted")
        .process_58 = Some(process);
    // 005E16C3 00673A90(g_actorProcessManager, this, 3, 0, 0, 0).
    actor_process_manager_add(world, form, 3, false, false)?;
    // 005E16C8..005E177F in store order.
    actor.last_process_update_time_bc = -1.0;
    actor.dead_state_b0 = 0;
    actor.deferred_death_processing_c0 = 0;
    actor.expression_override_strength_74 = 0.0;
    actor.expression_override_70 = 7;
    actor.combat_target_reevaluation_timer_ac = 0.0;
    actor.processing_enabled_78 = 1;
    actor.head_track_target_hold_timer_dc = 0.0;
    actor.murder_candidate_80 = 0;
    actor.head_track_target_scan_timer_100 = 0.0;
    actor.force_sneak_c9 = 0;
    actor.starting_z_rotation_f4 = 0.0;
    actor.force_run_c8 = 0;
    actor.death_killer_7c = 0;
    actor.pending_dialogue_target_e4 = 0;
    actor.leveled_creature_template_d0 = 0;
    actor.horse_or_rider_d4 = 0;
    actor.allow_quest_process_reschedule_ca = 1;
    actor.pending_movement_blocked_ref_cc = 0;
    actor.head_track_visibility_active_e0 = 1;
    actor.render_visibility_flags_c4 = 0;
    actor.starting_location_f8 = None;
    actor.starting_position_e8 = NI_POINT3_ZERO;
    actor.death_effects_processed_fc = 0;
    actor.merchant_respawn_day_processed_fd = 1;
    world.entity_mut(form).insert(actor);
    Ok(form)
}

/// IDA: 005E1880 Actor_Constructor(add), the constructor 004DB260 reaches (always with 1). Beside 005E15E0: the 00673A90 add runs only with `add`, it stores +0x84 = 0.0 and +0xD8 = 0, and it never stores +0x80 (FormHeap bytes in the original, host zero). Completion: 100% of the store order.
pub fn construct_actor_with_add(
    world: &mut World,
    inputs: &ProcessInputs<'_>,
    add: bool,
) -> Result<Entity, StreamError> {
    // 005E18AD 00659990 MobileObject_Constructor.
    let form = construct_reference(world)?;
    world
        .get_mut::<OriginalVtable>(form)
        .expect("just spawned")
        .0 = MOBILE_OBJECT_VTABLE;
    world
        .get_mut::<TesObjectRefr>(form)
        .expect("just inserted")
        .child_cell_vtable_18 = MOBILE_OBJECT_CHILD_CELL_VTABLE;
    world.entity_mut(form).insert(MobileObject::default());
    let mut actor = Actor::default();
    // 005E18BD 00699140 on +0x5C, 005E18CC 006A1D50 on +0x68.
    actor.magic_caster_5c.original_vtable = MAGIC_CASTER_VTABLE;
    actor.magic_target_68.original_vtable = MAGIC_TARGET_VTABLE;
    // 005E18DC..005E18EF the four Actor vtables.
    world
        .get_mut::<OriginalVtable>(form)
        .expect("just spawned")
        .0 = ACTOR_VTABLES[0];
    world
        .get_mut::<TesObjectRefr>(form)
        .expect("just inserted")
        .child_cell_vtable_18 = ACTOR_VTABLES[1];
    actor.magic_caster_5c.original_vtable = ACTOR_VTABLES[2];
    actor.magic_target_68.original_vtable = ACTOR_VTABLES[3];
    // 005E18F6 0065BE10 on +0x88.
    actor.av_modifiers_88 = construct_actor_values();
    // 005E18FB..005E1919 +0x9C, +0xA0, +0xA4, +0xA8, +0xB4, +0xB8 = 0.
    actor.greater_power_list_9c = [0; 2];
    actor.disposition_modifiers_a4 = [0; 2];
    actor.reserved_b4 = [0; 2];
    // 005E191F flags |= 0x200000.
    world
        .get_mut::<FormIdentity>(form)
        .expect("just spawned")
        .flags_08 |= FORM_FLAG_ACTOR;
    // 005E1926..005E1959 FormHeap(0x90) LowProcess_ctor -> +0x58.
    let process = construct_low_process(world, inputs);
    world
        .get_mut::<MobileObject>(form)
        .expect("just inserted")
        .process_58 = Some(process);
    // 005E1950..005E1969 00673A90(g_actorProcessManager, this, 3, 0, 0, 0), with the argument.
    if add {
        actor_process_manager_add(world, form, 3, false, false)?;
    }
    // 005E196E..005E1A2A in store order; the last two of +0xE4 and +0x84 follow the floats on the x87 stack.
    actor.pending_dialogue_target_e4 = 0;
    actor.last_process_update_time_bc = -1.0;
    actor.dead_state_b0 = 0;
    actor.deferred_death_processing_c0 = 0;
    actor.expression_override_strength_74 = 0.0;
    actor.expression_override_70 = 7;
    actor.combat_target_reevaluation_timer_ac = 0.0;
    actor.processing_enabled_78 = 1;
    actor.head_track_target_hold_timer_dc = 0.0;
    actor.force_sneak_c9 = 0;
    actor.head_track_target_scan_timer_100 = 0.0;
    actor.force_run_c8 = 0;
    actor.starting_z_rotation_f4 = 0.0;
    actor.death_killer_7c = 0;
    actor.leveled_creature_template_d0 = 0;
    actor.horse_or_rider_d4 = 0;
    actor.allow_quest_process_reschedule_ca = 1;
    actor.pending_movement_blocked_ref_cc = 0;
    actor.unset_in_water_d8 = 0;
    actor.head_track_visibility_active_e0 = 1;
    actor.render_visibility_flags_c4 = 0;
    actor.starting_location_f8 = None;
    actor.starting_position_e8 = NI_POINT3_ZERO;
    actor.unset_check_dead_body_timer_84 = 0.0;
    actor.pending_dialogue_target_e4 = 0;
    actor.death_effects_processed_fc = 0;
    actor.merchant_respawn_day_processed_fd = 1;
    world.entity_mut(form).insert(actor);
    Ok(form)
}

/// IDA: 0060E540 Character_Constructor(add), 004DB2DB's arm: 005E1880, the four Character vtables, +0x04 = 0x32, +0x104 = 0. It leaves +0x108 unset (FormHeap bytes), which the C++ agrees with: 0060E4F0, the player's, stores -1.0 there. Completion: 100% of the store order.
pub fn construct_character_with_add(
    world: &mut World,
    inputs: &ProcessInputs<'_>,
    add: bool,
) -> Result<Entity, StreamError> {
    let form = construct_actor_with_add(world, inputs, add)?;
    world
        .get_mut::<OriginalVtable>(form)
        .expect("just spawned")
        .0 = CHARACTER_VTABLES[0];
    world
        .get_mut::<TesObjectRefr>(form)
        .expect("just inserted")
        .child_cell_vtable_18 = CHARACTER_VTABLES[1];
    let mut actor = world.get_mut::<Actor>(form).expect("just inserted");
    actor.magic_caster_5c.original_vtable = CHARACTER_VTABLES[2];
    actor.magic_target_68.original_vtable = CHARACTER_VTABLES[3];
    world
        .get_mut::<FormIdentity>(form)
        .expect("just spawned")
        .form_type_04 = FORM_TYPE_CHARACTER;
    world.entity_mut(form).insert(Character {
        skin_info_104: None,
        armor_rating_cache_108: 0.0,
    });
    Ok(form)
}

/// IDA: 00625100 Creature_Constructor(add), 004DB29E's arm: 005E1880, the four Creature vtables, +0x04 = 0x33, byte +0x104 = 0. Completion: 100% of the store order.
pub fn construct_creature_with_add(
    world: &mut World,
    inputs: &ProcessInputs<'_>,
    add: bool,
) -> Result<Entity, StreamError> {
    let form = construct_actor_with_add(world, inputs, add)?;
    world
        .get_mut::<OriginalVtable>(form)
        .expect("just spawned")
        .0 = CREATURE_VTABLES[0];
    world
        .get_mut::<TesObjectRefr>(form)
        .expect("just inserted")
        .child_cell_vtable_18 = CREATURE_VTABLES[1];
    let mut actor = world.get_mut::<Actor>(form).expect("just inserted");
    actor.magic_caster_5c.original_vtable = CREATURE_VTABLES[2];
    actor.magic_target_68.original_vtable = CREATURE_VTABLES[3];
    world
        .get_mut::<FormIdentity>(form)
        .expect("just spawned")
        .form_type_04 = FORM_TYPE_CREATURE;
    world.entity_mut(form).insert(Creature { byte_104: 0 });
    Ok(form)
}

// IDA: 0060E4F0 Character_Constructor: Actor_Constructor, +0x108 = -1.0, the four vtables, +0x04 = 0x32, +0x104 = 0. Completion: 100% of the store order.
pub fn construct_character(
    world: &mut World,
    inputs: &ProcessInputs<'_>,
) -> Result<Entity, StreamError> {
    // 0060E4F3 Actor_Constructor.
    let form = construct_actor(world, inputs)?;
    // 0060E4F8 +0x108 = -1.0 (flt_A30634).
    let armor_rating_cache_108 = -1.0;
    // 0060E504..0060E518 the four Character vtables.
    world
        .get_mut::<OriginalVtable>(form)
        .expect("just spawned")
        .0 = CHARACTER_VTABLES[0];
    world
        .get_mut::<TesObjectRefr>(form)
        .expect("just inserted")
        .child_cell_vtable_18 = CHARACTER_VTABLES[1];
    let mut actor = world.get_mut::<Actor>(form).expect("just inserted");
    actor.magic_caster_5c.original_vtable = CHARACTER_VTABLES[2];
    actor.magic_target_68.original_vtable = CHARACTER_VTABLES[3];
    // 0060E51F +0x04 = 0x32 (ACHR); 0060E523 +0x104 = 0.
    world
        .get_mut::<FormIdentity>(form)
        .expect("just spawned")
        .form_type_04 = FORM_TYPE_CHARACTER;
    world.entity_mut(form).insert(Character {
        skin_info_104: None,
        armor_rating_cache_108,
    });
    Ok(form)
}

// IDA: 00659A00 Actor_GetProcessLevel: process +0x58 vt+8, or -1 without one. Completion: 100%.
#[must_use]
pub fn actor_process_level(world: &World, actor: Entity) -> i32 {
    world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
        .map_or(-1, |process| process_level(world, process))
}

/// IDA: dbl_A3D5B0: 6.2831854820251465, exactly the float 2*pi.
const TWO_PI_A3D5B0: f32 = 6.283_185_482_025_146_5_f64 as f32;

// IDA: 0065A450 Actor_SetZRotationChecked(radians). With a process: vt+0x19C 005E04F0, process vt+0x2E4: 0064B080's `movsx` of +0x11C on MiddleHigh and High, 006F7070's 0 on the others; nonzero returns with no store. Then process vt+0x36C (0064B090's `movzx` of +0x11D, or 0): minus 4, the 0065A54C bytes send 4, 5, 9, 10 to 0065A48E and 6, 7, 8 past it: vt+0x378 00629630 reads +0x120 (006F7070 on the others, where 0 goes on); a nonzero +0x120 goes to the RTTI cast and Actor vt+0x380 (0060E350 on the player: +0xD4), whose 0 returns with no store: a host stop. Then the wrap, 0065A4CA: `fldz`, `fld radians`, `fcom`: below 0 (0065A4E1), `fadd` the 2*pi, `fstp` float, then `__CIfmod` of that by 2*pi, `fstp` float, 004D8A10. Otherwise 2*pi `fcom` radians: below (0065A514): `__CIfmod` of radians by 2*pi. Otherwise, or on a NaN, the value as it came (0065A530). `fprem` is exact, so `x87::float_fmod`; the sum rounds once to float at 24 bits. Completion: 100% with the stop.
// TODO: [ ] 0065A49F, the +0x120 arm, when a run reaches it. RUN-001..003: +0x11D is 0.
fn actor_set_z_rotation_checked(
    world: &mut World,
    actor: Entity,
    radians: f32,
) -> Result<(), StreamError> {
    let process = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58);
    if let Some(process) = process
        && let Some(middle) = world.get::<MiddleHighProcess>(process)
    {
        if middle.knocked_state_11c != 0 {
            return Ok(());
        }
        if matches!(middle.byte_11d, 4 | 5 | 9 | 10) && middle.field_120 != 0 {
            return Err(StreamError("0065A49F: the +0x120 arm of a sleep state"));
        }
    }
    let wrapped = if radians < 0.0 {
        x87::float_fmod(radians + TWO_PI_A3D5B0, TWO_PI_A3D5B0)
    } else if TWO_PI_A3D5B0 < radians {
        x87::float_fmod(radians, TWO_PI_A3D5B0)
    } else {
        radians
    };
    set_reference_rotation_z(world, actor, wrapped)
}

/// IDA: 005E0EA0 Actor_SetZRotationIfProcessAllows(radians), vt+0x1E8 on Actor, Character, Creature and PlayerCharacter: +0xB0 is 0 or 4 runs 0065A450 (`fld`, `fstp` the argument: a float copy); otherwise nothing. Completion: 100%.
/// # Errors
/// A stop in 0065A450.
pub fn actor_set_z_rotation_if_process_allows(
    world: &mut World,
    actor: Entity,
    radians: f32,
) -> Result<(), StreamError> {
    let dead_state = world.get::<Actor>(actor).expect("an actor").dead_state_b0;
    if dead_state == 0 || dead_state == 4 {
        actor_set_z_rotation_checked(world, actor, x87::fld_fstp_copy(radians.to_bits()))?;
    }
    Ok(())
}
