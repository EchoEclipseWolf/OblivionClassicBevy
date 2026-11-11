//! Phase 11 (D1): the reference/actor base chain of PlayerCharacter_ctor 0066A740: Character_ctor 0060E4F0 -> Actor_ctor 005E15E0 -> MobileObject_ctor 00659990 -> TESObjectREFR_ctor 004D9A70 ->
//! TESForm_Constructor 0046C460 (`tes::data_handler`; its DataHandler arm runs for the first time here).
//!
//! A form is an entity. Each class level is a capability component named after the class, holding that level's fields with their offsets in the names. The four vtable identities per level sit where the
//! object has them: main in `OriginalVtable`, the TESChildCell +0x18 in `TesObjectRefr`, MagicCaster +0x5C and MagicTarget +0x68 in `Actor`. Bytes the constructors do not store are host zero (uninitialized
//! FormHeap bytes in the original, recorded substitution). Evidence: evidence/phase11-window-2026-10-06/disasm-chain.txt; P11-RUN-001.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

pub use crate::tes::base_extra_list::ExtraDataList;

use super::actor_process::{
    ActorValues, MiddleHighProcess, ProcessInputs, actor_process_manager_add,
    construct_actor_values, construct_low_process, process_level,
};
use crate::bs::character_controller::{BhkCharacterController, HAVOK_SCALE};
use crate::tes::actor_process::process_actor_value;
use crate::tes::actors::actor_process_high::write_high_process;
use crate::tes::ai_cell_moves::list_of;
use crate::tes::ai_package::compare_length;
use crate::tes::ai_process::{actor_get_actor_value_float, fold_creature_code, no_process_value};
use crate::tes::base_extra_list::{
    EXTRA_TELEPORT, extras_write_nothing, mark_teleport_cells, teleport_door,
};
use crate::tes::cell_reference_loop::{CastBoundaries, DrawnRandom, base_and_race_abilities};
use crate::tes::character_controller::actor_character_controller;
use crate::tes::data_load::DataLoadGlobals;
use crate::tes::equipment::find_equipped_in_slot;
use crate::tes::form::{FormIdentity, OriginalVtable};
use crate::tes::form_components::TesActorBaseData;
use crate::tes::forms_actors::{base_race, npc_sex, race_scale};
use crate::tes::forms_init::form_type;
use crate::tes::forms_items::set_change_flags;
use crate::tes::forms_references::{
    construct_reference, parent_cell, reference_owned_by, reference_position, reference_world,
    set_reference_locked, set_reference_rotation_z,
};
use crate::tes::forms_world::cell_is_interior;
use crate::tes::player::{refresh_equipped_visuals, the_player};
use crate::tes::save_game::{ProcessView, SaveWriter, empty_list, null_id};
use crate::tes::stats_menu::player;
use crate::tes::tes_grid::cell_water_height;
use crate::tes::tile::InterfaceCx;

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
    /// +0xD8 the actor stood in water on its last movement update: 00603CA0 stores Actor_IsSubmergedAtHeightFraction(0.01)
    /// at 00604565.., after testing it against the new answer (the entered-water splash).
    pub in_water_d8: u8,
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

// IDA: 005E15E0 Actor_Constructor (after 00659990 MobileObject_Constructor). The LowProcess and the level-3 add (00673A90) run here. Completion: 100% of the store order.
pub fn construct_actor(world: &mut World, inputs: &ProcessInputs<'_>) -> Result<Entity, GameError> {
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
) -> Result<Entity, GameError> {
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
    actor.in_water_d8 = 0;
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
) -> Result<Entity, GameError> {
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
) -> Result<Entity, GameError> {
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
) -> Result<Entity, GameError> {
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
) -> Result<(), GameError> {
    let process = world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58);
    if let Some(process) = process
        && let Some(middle) = world.get::<MiddleHighProcess>(process)
    {
        if middle.knocked_state_11c != 0 {
            return Ok(());
        }
        if matches!(middle.sleep_state_11d, 4 | 5 | 9 | 10)
            && middle.furniture_reference_120.is_some()
        {
            return Err(GameError::unclassified(
                0x0065A49F,
                "0065A49F: the +0x120 arm of a sleep state",
            ));
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
) -> Result<(), GameError> {
    let dead_state = world.get::<Actor>(actor).expect("an actor").dead_state_b0;
    if dead_state == 0 || dead_state == 4 {
        actor_set_z_rotation_checked(world, actor, x87::fld_fstp_copy(radians.to_bits()))?;
    }
    Ok(())
}

/// IDA: the Character's vt InvalidateArmorRatingCache: +0x108 = -1.0. Completion: 100%.
pub fn invalidate_armor_rating_cache(world: &mut World, actor: Entity) {
    if let Some(mut character) = world.get_mut::<Character>(actor) {
        character.armor_rating_cache_108 = -1.0;
    }
}

/// IDA: N/A an actor's process +0x58.
#[must_use]
pub fn process_of(world: &World, actor: Entity) -> Option<Entity> {
    world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
}

/// IDA: 005E33B0 Actor_IsDead(excludeState6). Completion: 100%.
#[must_use]
pub fn actor_is_dead(world: &World, actor: Entity, exclude_state_6: bool) -> bool {
    let state = world.get::<Actor>(actor).expect("an actor").dead_state_b0;
    state == 1 || state == 2 || (!exclude_state_6 && state == 6)
}

// IDA: 0065ADD0 Actor_GetBoundBottom / 0065AE80 Actor_GetBoundTop. A null +0x188 delegates
// to 004DC360 TESObjectREFR_GetBoundBottom / 004DC410 TESObjectREFR_GetBoundTop on raw +0x3C.
pub(in crate::tes) fn actor_bound_corner(
    world: &mut World,
    actor: Entity,
    top: bool,
) -> Result<[f32; 3], GameError> {
    use crate::bs::fade_node::BoundExtraTopStatics;
    let cached = process_of(world, actor)
        .and_then(|process| world.get::<MiddleHighProcess>(process))
        .and_then(|process| process.bound_188);
    let bound = match cached {
        Some(bound) => Some(bound),
        None => match world.get::<crate::tes::reference_3d::Reference3D>(actor) {
            Some(node) => crate::gamebryo::object_net::find_extra_data(world, node.0, b"BBX")?,
            None => None,
        },
    };
    let Some(bound) = bound else {
        return Ok([0.0; 3]);
    };
    let corner = world
        .get::<crate::gamebryo::extra_data::BsBound>(bound)
        .ok_or(GameError::unclassified(
            0x005E0660,
            "an actor's BBX extra is not a BSBound",
        ))?
        .corner(top);
    let mut statics = world.get_resource_or_init::<BoundExtraTopStatics>();
    match (cached.is_some(), top) {
        (true, true) => statics.process_top_b3bab8 = corner,
        (true, false) => statics.process_bottom_b3baac = corner,
        (false, true) => statics.top_b35ef4 = corner,
        (false, false) => statics.bottom_b35ee8 = corner,
    }
    Ok(corner)
}

// IDA: 005E0660 Actor_GetScaledBoundHeight. Read the two virtuals separately, then the actor's scale.
pub(in crate::tes) fn actor_height(world: &mut World, actor: Entity) -> Result<f32, GameError> {
    let top = actor_bound_corner(world, actor, true)?;
    let bottom = actor_bound_corner(world, actor, false)?;
    let height = top[2] - bottom[2];
    Ok(height * crate::tes::actor_animation::actor_scale(world, actor)?)
}

/// IDA: vt+0x18C 005E0EC0: process vt+0x36C, 0 without a process. Low and MiddleLow 006F7070 answer 0; MiddleHigh and High 0064B090 the +0x11D byte. Completion: 100%.
#[must_use]
pub fn actor_sitting_state(world: &World, actor: Entity) -> u8 {
    process_of(world, actor)
        .and_then(|process| world.get::<MiddleHighProcess>(process))
        .map_or(0, |middle| middle.sleep_state_11d)
}

// IDA: 00645AF0 the unlock callback and 00645A30 the lock one: a living actor and a door (0x18) reference; the rank flag for a teleport door whose destination is in a world (0042B470), or an actor outside an interior; an owned door with a lock takes the lock or unlock, and the cells on both sides are flagged or cleared with the opposite value. Both answer 0.
pub(crate) fn door_lock_callback(
    world: &mut World,
    reference: Entity,
    actor: Entity,
    lock: bool,
) -> Result<(), GameError> {
    if actor_is_dead(world, actor, false) {
        return Ok(());
    }
    let base = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c;
    if base.is_none_or(|base| form_type(world, base) != 0x18) {
        return Ok(());
    }
    let mut faction_ok = false;
    let linked = teleport_door(world, reference);
    let teleport = list_of(world, reference).get(EXTRA_TELEPORT).is_some();
    if teleport {
        let cell = parent_cell(world, actor);
        if linked.is_some_and(|linked| reference_world(world, linked).is_some())
            || cell.is_none_or(|cell| !cell_is_interior(world, cell))
        {
            faction_ok = true;
        }
    }
    if reference_owned_by(world, reference, actor, faction_ok)?
        && crate::tes::forms_references::reference_lock(world, reference).is_some()
    {
        set_reference_locked(world, reference, lock)?;
        if teleport {
            mark_teleport_cells(world, linked, reference, !lock)?;
        }
    }
    Ok(())
}

/// IDA: 00684B30 Actor_IsWithinRadius. A nonzero hkCharacterContext state clears the vertical
/// delta. Otherwise the actor's scaled bound (128 when zero) and ±30 determine the vertical window.
/// A widened radius includes at least 32 game units, or the controller's converted capsule radius.
pub fn actor_within_radius(
    world: &mut World,
    actor: Entity,
    point: [f32; 3],
    radius: f32,
    widen: bool,
) -> Result<bool, GameError> {
    let position = reference_position(world, actor)?;
    let mut delta = [
        position[0] - point[0],
        position[1] - point[1],
        position[2] - point[2],
    ];
    use crate::bs::character_controller::BhkCharacterController;
    use crate::bs::character_controller::controller_capsule_radius;
    let controller = actor_character_controller(world, actor);
    let ignore_height = controller.is_some_and(|c| {
        world
            .get::<BhkCharacterController>(c)
            .expect("a character controller")
            .context_state_1ec
            != 0
    });
    if ignore_height {
        delta[2] = 0.0;
    } else {
        let mut height = actor_height(world, actor)?;
        if height == 0.0 {
            height = 128.0;
        }
        if position[2] - 30.0 < point[2] && (position[2] + height) + 30.0 > point[2] {
            delta[2] = 0.0;
        }
    }
    let mut limit = radius;
    if widen {
        let mut width = 32.0;
        if let Some(c) = controller {
            width = controller_capsule_radius(world, c) * 6.999_040_126_800_537_f64 as f32;
            if world.get::<BhkCharacterController>(c).unwrap().flags_1f4 & 1 != 0 {
                width += width;
            }
            if width < 32.0 {
                width = 32.0;
            }
        }
        limit = width + radius;
    }
    Ok(compare_length(delta, limit) <= 0)
}

// IDA: 005F19A7: `fv = vt+0x288(40)`; `f32(fv / 10.0)`; exactly 0 (-0.0 too) is 1.0, a NaN stays.
pub(crate) fn magicka_multiplier(world: &mut World, actor: Entity) -> Result<f32, GameError> {
    let multiplier = (f64::from(actor_get_actor_value_float(world, actor, 40)?) / 10.0) as f32;
    Ok(if multiplier == 0.0 { 1.0 } else { multiplier })
}

/// IDA: 005F1970 Actor vt+0x284 GetActorValue (Creature 006253C0 folds first). AV 9: `ftol(f32(fild(ftol(00643340)) * multiplier))`: the sum is an int before the multiply (005F19EB), where the C++ keeps its fraction. The others `ftol` the sum. The player is 0065E030's (`player::player_get_actor_value`). Completion: 100%.
pub fn actor_get_actor_value(
    world: &mut World,
    actor: Entity,
    actor_value: u32,
) -> Result<i32, GameError> {
    // PlayerCharacter's vt+0x284 is 0065E030.
    if Some(actor) == the_player(world) {
        return crate::tes::player::player_get_actor_value(world, actor, actor_value);
    }
    let code = fold_creature_code(world, actor, actor_value);
    let Some(process) = process_of(world, actor) else {
        return no_process_value(world, actor, code);
    };
    if code == 9 {
        let multiplier = magicka_multiplier(world, actor)?;
        let value = process_actor_value(world, actor, process, 9)?;
        return Ok(x87::ftol((f64::from(value) * f64::from(multiplier)) as f32));
    }
    process_actor_value(world, actor, process, code)
}

// IDA: 005E7EF0 at version 125 with flags 4: 00659C30 (process type byte 0, 004E2D60, the process), then the actor's own fields.
pub(crate) fn write_actor(
    out: &mut SaveWriter,
    world: &World,
    player: Entity,
    process: &ProcessView<'_>,
) -> Result<(), GameError> {
    let actor = world.get::<Actor>(player).expect("an actor");
    // 00659C48: process vt+0x08, 006F7070 answers 0 for a high process.
    out.u8(0);
    extras_write_nothing(world, player)?;
    out.u16(0);
    write_high_process(out, world, process)?;
    out.f32(actor.last_process_update_time_bc);
    out.u8(actor.force_run_c8);
    out.u8(actor.force_sneak_c9);
    out.u8(actor.processing_enabled_78);
    empty_list(actor.greater_power_list_9c, "005E80DF: a greater power")?;
    out.u16(0);
    out.u32(null_id(
        actor.death_killer_7c,
        "005E814E: the death killer",
    )?);
    out.u32(null_id(
        actor.leveled_creature_template_d0,
        "005E817A: the template",
    )?);
    out.u8(actor.murder_candidate_80);
    out.u32(null_id(
        actor.pending_movement_blocked_ref_cc,
        "005E81D8: the blocked ref",
    )?);
    out.u32(null_id(
        actor.pending_dialogue_target_e4,
        "005E820B: the dialogue target",
    )?);
    out.f32(actor.unset_check_dead_body_timer_84);
    out.f32(actor.combat_target_reevaluation_timer_ac);
    out.u8(actor.allow_quest_process_reschedule_ca);
    out.u8(actor.in_water_d8);
    out.f32(actor.head_track_target_hold_timer_dc);
    out.f32(actor.head_track_target_scan_timer_100);
    out.u8(actor.death_effects_processed_fc);
    out.u8(actor.deferred_death_processing_c0);
    Ok(())
}

// IDA: 005E6680 Actor_SetDeathState(state), for the arms a load takes. With 00B14E98 set, an essential base (+0x28 bit 1) turns 1 or 2 into 6. State 2: a process's vt+0xE8 is null on Low and MiddleLow; 0x1A extra data would run 004876C0: a host stop. A nonzero state on a level-0 process goes to 00628630: a host stop. A change marks vt+0x40(0x40); +0xB0 takes it.
pub(crate) fn set_death_state(
    world: &mut World,
    actor: Entity,
    state: u32,
) -> Result<(), GameError> {
    let mut state = state;
    if world
        .resource::<DataLoadGlobals>()
        .essential_take_no_damage_b14e98
        != 0
    {
        let essential = world
            .get::<TesObjectRefr>(actor)
            .expect("an actor")
            .base_form_1c
            .and_then(|base| world.get::<TesActorBaseData>(base))
            .is_some_and(|data| data.flags_04 & 2 != 0);
        if essential && (state == 2 || state == 1) {
            state = 6;
        }
    }
    let process = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58;
    if state == 2
        && world
            .get::<TesObjectRefr>(actor)
            .expect("an actor")
            .extra_data_44
            .get(crate::tes::forms::base_extra_list::EXTRA_CONTAINER_CHANGES)
            .is_some()
    {
        return Err(GameError::unclassified(
            0x005E670C,
            "005E670C: 004876C0 with container changes",
        ));
    }
    if state != 0 && process.is_some_and(|process| process_level(world, process) == 0) {
        return Err(GameError::unclassified(
            0x005E6753,
            "005E6753: 00628630 on a high process at death",
        ));
    }
    if world.get::<Actor>(actor).expect("an actor").dead_state_b0 != state {
        set_change_flags(world, actor, 0x40)?;
    }
    world
        .get_mut::<Actor>(actor)
        .expect("an actor")
        .dead_state_b0 = state;
    Ok(())
}

// IDA: 0065A330 Actor_SetParentCell, whose first store is on the controller: an unchanged cell returns; for a nonnull cell with a controller (0065A2C0), +0x318 = `fmul` the qword 0.1428767293691635 on 004CACE0's float, one rounding; then 004D8AA0: for a nonnull, changed cell vt+0x40(4); +0x40 = cell; vt+0x190 with a process runs its vt+0x500: 0068F970, a bare `retn`, on Low and MiddleLow; 00651EF0 on the others, which returns with a null or empty +0x170 and is a host stop past that. A form with no Actor goes straight to 004D8AA0. Completion: 100%, with 00651EF0's list a stop.
// TODO: [ ] 00651F10 the process's +0x170 list.
pub(crate) fn actor_or_reference_set_parent_cell(
    world: &mut World,
    reference: Entity,
    cell: Option<Entity>,
) -> Result<(), GameError> {
    let parent = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .parent_cell_40;
    let process = world
        .get::<MobileObject>(reference)
        .and_then(|mobile| mobile.process_58);
    let actor = world.get::<Actor>(reference).is_some();
    if actor {
        if cell == parent {
            return Ok(());
        }
        if let Some(cell) = cell
            && let Some(controller) = actor_character_controller(world, reference)
        {
            let water = cell_water_height(world, cell);
            world
                .get_mut::<BhkCharacterController>(controller)
                .expect("the process's controller")
                .water_height_318 = (f64::from(water) * f64::from(HAVOK_SCALE)) as f32;
        }
    }
    if cell.is_some() && parent != cell {
        set_change_flags(world, reference, 4)?;
    }
    world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference")
        .parent_cell_40 = cell;
    if actor
        && let Some(process) = process
        && let Some(middle) = world.get::<MiddleHighProcess>(process)
        && middle.field_170 != 0
    {
        return Err(GameError::unclassified(
            0x00651F10,
            "00651F10: 00651EF0, a process's +0x170 list",
        ));
    }
    Ok(())
}

/// IDA: 00611D00 Actor_GetHeightScale, vt+0xEC: 004D7260's +0x38, times the race's height at +0x60 + 4 * sex when the base has a race. 004D7260's creature arm doesn't apply to the player's base. Completion: 100% for an NPC base.
/// # Errors
/// A base that isn't an NPC.
pub fn actor_height_scale(world: &World, actor: Entity) -> Result<f32, GameError> {
    let refr = world.get::<TesObjectRefr>(actor).expect("a reference");
    let scale = refr.scale_38;
    let Some(base) = refr.base_form_1c else {
        return Ok(scale);
    };
    let Some(race) = base_race(world, base)? else {
        return Ok(scale);
    };
    let Some(sex) = npc_sex(world, base) else {
        return Ok(0.0 * scale);
    };
    Ok(race_scale(world, race, 0x60 + 4 * sex) * scale)
}

// IDA: 005F1910 Actor_FloorCalculatedActorValue, `fild` to a float.
pub(crate) fn calculated(cx: &InterfaceCx, code: u32) -> Result<f32, GameError> {
    let player = player(cx)?;
    Ok(crate::tes::player::floor_calculated_actor_value(cx.world, player, code)? as f32)
}

/// IDA: 005FA320 Actor_RebuildHighProcess, vt+0x1A4, for an actor with a LowProcess or none, as run-ac shows the prison's 11, or a MiddleLowProcess (005F23E0's level-0 arm; its vt+0x18 006588C0 stores +0x90 and 0060CDB0 takes the MiddleLow arm 00658760). A process at level 0 returns at once (005FAA4C). The old process's vt+0x18 (00649340, the package evaluation, forced) unless a save loads; vt+0x334 005E6110(1); the old package's target reference; 005E6CD0(0); the level, 3 with no process, and 00674550 removes the actor from its list. A rotation x of FLT_MAX is set to 0 (004D84E0) and goes to 005EB400; so does an actor at the origin, or one at level 2 or 3 with no save loading; 005EB400 false, or any other actor, takes 005EB370. FormHeap(0x2EC) 00628EE0 (its draw), vt+4 0060CDB0 from the old process, the old one's vt+0(1), +0x58 = the new; vt+0x210 (+0x1F) true would run vt+0x300(1). 00673A90(actor, 0, 0, 0, 0); 00634CB0; vt+0x5C. With container changes and a level other than 1 the equipment block (`rebuild_equipment`). vt+0x4C; 005E6E00; the combat arm (005EAE70) with vt+0x334's answer and no combat controller; the death arm; vt+0x178(0); the sleeping arm needs a node; with +0xC0 the death animation arm, a stop; then 005E7230(null, 0); a current package of type 6 gets vt+0x17C(0); a living actor at level 2 or 3 gets 005EDA20(0); vt+0x80's arm; and vt+0x154's lighting entry. Completion: the arms run-ac shows, in its order; the rest stop.
/// # Errors
/// A stop.
#[allow(
    clippy::too_many_lines,
    reason = "one function in the exe, in its order"
)]
pub fn actor_rebuild_high_process(world: &mut World, actor: Entity) -> Result<(), GameError> {
    use crate::tes::actor_process::{BaseProcess, LowProcess, MiddleHighProcess, process_level};
    let old = world.get::<MobileObject>(actor).and_then(|m| m.process_58);
    if old.is_some_and(|p| process_level(world, p) == 0) {
        return Ok(());
    }
    let loading = crate::tes::forms_references::save_load_active(world)?;
    // 005FA36C..005FA386 old vt+0x18(actor, 1): 00649340 on a LowProcess, 006588C0 on a MiddleLowProcess.
    if let Some(old) = old
        && !loading
    {
        // 006588C0 on a MiddleLowProcess: the game hour first (forced, so no same-hour test), +0x90 = its `__ftol2` after.
        let hour = crate::tes::global::game_hour(world);
        crate::tes::actor_process::process_evaluate_package(
            world,
            old,
            actor,
            true,
            &mut crate::tes::ai_conditions::ScriptArms,
        )?;
        if world
            .get::<crate::tes::form::OriginalVtable>(old)
            .map(|vtable| vtable.0)
            == Some(crate::tes::actor_process::MIDDLE_LOW_PROCESS_VTABLE)
            && let Some(mut middle_low) =
                world.get_mut::<crate::tes::actor_process::MiddleLowProcess>(old)
        {
            middle_low.evaluated_hour_90 = oblivion_core::x87::ftol(hour);
        }
    }
    // 005FA394 vt+0x334(1) 005E6110: kept in a byte for 005FA606.
    let combat = crate::tes::actor_package::actor_in_combat_package(world, actor, true)?;
    // 005FA39A..005FA3B8 the old process's package target's reference (00569E60), for 005EAE70's arm.
    let current = world.get::<MobileObject>(actor).and_then(|m| m.process_58);
    let _target = current
        .and_then(|p| {
            world
                .get::<BaseProcess>(p)
                .and_then(|b| b.editor_package_08)
        })
        .and_then(|package| crate::tes::ai_package::package_data(world, package).target_28)
        .and_then(|target| match (target.type_00, target.value_04) {
            (0, crate::tes::form_components::FormRef::Resolved(reference)) => reference,
            _ => None,
        });
    // 005FA3C2 005E6CD0(0): its answer goes unused.
    crate::tes::actor_package::actor_takes_package_procedure(world, actor, false)?;
    // 005FA3D0..005FA3F7 the level, and 00674550.
    let mut level = 3;
    if let Some(old) = current {
        level = process_level(world, old);
        crate::tes::actor_process::actor_process_manager_remove(world, actor, level, loading);
    }
    // 005FA3FB..005FA476 the placement.
    let data = world.get::<TesObjectRefr>(actor).expect("an actor");
    let (rotation_x, position) = (data.rotation_20[0], data.position_2c);
    let try_path_grid = if rotation_x == f32::MAX {
        return Err(GameError::unclassified(
            0x005FA413,
            "005FA413: 004D84E0 for a rotation x of FLT_MAX",
        ));
    } else {
        position == [0.0; 3] || (!loading && (2..=3).contains(&level))
    };
    if !(try_path_grid && crate::tes::actor_placement::actor_place_on_path_grid(world, actor)?) {
        crate::tes::actor_placement::actor_place_if_blocked(world, actor)?;
    }
    // 005FA47B..005FA4C7 the HighProcess (its draw), 0060CDB0 from the old one, which is deleted.
    let random = world
        .get_non_send::<crate::tes::form_arms::ScriptRandom>()
        .ok_or(GameError::unclassified(
            0x0062901F,
            "0062901F: Random_Next with no host CRT",
        ))?
        .0
        .clone();
    let drawn = random(world);
    let timer = world
        .get_resource::<crate::tes::game_settings::GameSettingCollection>()
        .and_then(|settings| settings.raw_at(0x00B3_6940))
        .ok_or(GameError::unclassified(
            0x00629106,
            "00629106: fIdleChatterCommentTimer",
        ))?;
    let mut source = DrawnRandom(Some(drawn));
    let high = crate::tes::actor_process::construct_high_process(
        world,
        &mut crate::tes::actor_process::ProcessInputs {
            random: &mut source,
            idle_chatter_comment_timer: timer,
            // N/A FormHeap's unset bytes: 0060CDB0 copies the old process's over them; with none they stay unknown.
            low_process_unset_88: 0,
            save_load_active: loading,
        },
    );
    crate::tes::actor_process::copy_process_from(world, high, current)?;
    if let Some(old) = current {
        crate::tes::actor_process::delete_low_process(world, old)?;
    }
    world
        .get_mut::<MobileObject>(actor)
        .expect("an actor")
        .process_58 = Some(high);
    if world.get::<LowProcess>(high).expect("a process").byte_1f != 0 {
        return Err(GameError::unclassified(
            0x005FA4EB,
            "005FA4EB: vt+0x300(1) on a HighProcess",
        ));
    }
    // 005FA4FB..005FA50F 00673A90 at level 0; 00634CB0 (+0x1EC, then the node's Bip01, which needs a node); vt+0x5C 00629540 (+0x15C = 0).
    crate::tes::actor_process::actor_process_manager_add(world, actor, 0, false, false)?;
    crate::tes::actors::actor_process_high::find_bone_lod_controller(world, high, actor);
    world
        .get_mut::<MiddleHighProcess>(high)
        .expect("a process")
        .face_animation_data_15c = None;
    // 005FA514 the equipment block.
    if world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .extra_data_44
        .get(crate::tes::base_extra_list::EXTRA_CONTAINER_CHANGES)
        .is_some()
        && level != 1
    {
        rebuild_equipment(world, actor, high)?;
    }
    // 005FA5FD vt+0x4C 00628600: with +0x2C, its position (vt+0x174) to +0xD4.
    if world
        .get::<LowProcess>(high)
        .expect("a process")
        .field_2c
        .is_some()
    {
        return Err(GameError::unclassified(
            0x005FA5FD,
            "005FA5FD: 00628600 with a +0x2C reference",
        ));
    }
    // 005FA601 005E6E00.
    crate::tes::actor_package::package_arrival(world, actor)?;
    // 005FA606..005FA65D the combat arm.
    if combat {
        return Err(GameError::unclassified(
            0x005FA61F,
            "005FA61F: 005EAE70 for an actor in a combat package",
        ));
    }
    // 005FA663..005FA6DC the death arm: a creature with a node asks vt+0x278 (true for every class here).
    let deferred = world
        .get::<Actor>(actor)
        .expect("an actor")
        .deferred_death_processing_c0;
    if deferred != 0
        || (actor_is_dead(world, actor, false)
            && !has_extra(world, actor, crate::tes::base_extra_list::EXTRA_RAG_DOLL)
            && !has_extra(world, actor, 0x4B))
    {
        return Err(GameError::unclassified(
            0x005FA6D0,
            "005FA6D0: 005E6680(0) for a dead actor",
        ));
    }
    // 005FA6E8 vt+0x178(0) 0065B5C0: no controller returns.
    if crate::tes::character_controller::actor_character_controller(world, actor).is_some() {
        return Err(GameError::unclassified(
            0x005FA6E8,
            "005FA6E8: 0065B5C0 with a character controller",
        ));
    }
    // 005FA6EE..005FA93C: the sleeping arm and 005EE1B0 need a node; +0xC0 is 0.
    // 005FA941 005E7230(null, 0).
    refresh_equipped_visuals(world, actor)?;
    // 005FA947..005FA96F a current package of type 6 gets vt+0x17C(0).
    let package = world
        .get::<BaseProcess>(high)
        .and_then(|b| b.editor_package_08);
    if package
        .is_some_and(|package| crate::tes::ai_package::package_data(world, package).type_20 == 6)
    {
        set_procedure_index(world, high, 0)?;
    }
    // 005FA975..005FA991 a living actor at level 2 or 3: 005EDA20(0).
    if !actor_is_dead(world, actor, false) && (level == 3 || level == 2) {
        apply_base_spells(world, actor)?;
    }
    // 005FA996..005FAA1C vt+0x80 (00651AA0's +0x20 byte here, 0051AAD0).
    if world.get::<LowProcess>(high).expect("a process").byte_20 != 0 {
        return Err(GameError::unclassified(
            0x005FA9A8,
            "005FA9A8: vt+0x80's follower arm",
        ));
    }
    // 005FAA28 vt+0x154 null: no lighting entry.
    Ok(())
}

// IDA: N/A whether the reference's extra list holds an entry of the type with its data (0041FFA0's +0x0C, 004212C0's +0x10, each set by its loader).
fn has_extra(world: &World, actor: Entity, extra_type: u8) -> bool {
    world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .extra_data_44
        .get(extra_type)
        .is_some()
}

/// IDA: 005FA52C..005FA5F1, the equipment block: 00486790 for the weapon (9), quiver (12), shield (13) and torch (14) slots into vt+0x104 (00658550), vt+0x10C (0064AED0, +0xEC), vt+0x110 (0064AF10, +0xF0) and vt+0x108 (0064AE90, +0xE8); the weapon again for 00612A90 into vt+0x328 (+0xF8); vt+0xF0(1). Completion: 100% with nothing worn in those slots; a worn item stops.
/// TODO: [ ] 00486790 with a worn item, and the setters that keep it.
fn rebuild_equipment(world: &mut World, actor: Entity, process: Entity) -> Result<(), GameError> {
    for slot in [9, 12, 13, 14, 9] {
        if find_equipped_in_slot(world, actor, slot)? {
            return Err(GameError::unclassified(
                0x005FA539,
                "005FA539: an item worn in a rebuilt actor's slot",
            ));
        }
    }
    // 00658550(null, 0): +0xE4 is already null; +0x17C null clears +0xFC..+0x110; the player's arm clears 00B3BA84..00B3BA98.
    let mut middle = world
        .get_mut::<crate::tes::actor_process::MiddleHighProcess>(process)
        .expect("a process");
    if middle.weapon_e4.is_some() || middle.anim_data_17c.is_some() {
        return Err(GameError::unclassified(
            0x005FA548,
            "005FA548: 00658550 on a process with a weapon or animation data",
        ));
    }
    middle.weapon_nodes_fc = crate::tes::actor_process::WeaponNodes::default();
    // 0064AED0, 0064AF10, 0064AE90 store null over null; 00612A90(null) is 0.0 into +0xF8.
    middle.ammo_ec = None;
    middle.shield_f0 = None;
    middle.light_e8 = None;
    middle.weapon_rating_f8 = 0.0;
    Ok(())
}

/// IDA: vt+0x17C 0064F760(index) on a HighProcess: the current package (vt+0x184 0064B040: +0xC0, else +0x08); the index to +0xCC with a +0xC0 package, else +0x04; then with a package, an index (vt+0x180) at or past 00673980's sequence length for its procedure type (+0x18) is set again to the length less one. Completion: 100% with +0xC0 null.
fn set_procedure_index(world: &mut World, process: Entity, index: i32) -> Result<(), GameError> {
    crate::tes::ai_conditions::stop_on_process_package_override(world, process)?;
    let mut base = world
        .get_mut::<crate::tes::actor_process::BaseProcess>(process)
        .expect("a process");
    base.edit_pack_procedure_type_04 = index as u32;
    let Some(package) = base.editor_package_08 else {
        return Ok(());
    };
    // 00673980 ProcedureType_GetSequenceLength(type): 2, 3, 4 or 5 by the table, -1 for any other.
    let length = crate::tes::ai_process::procedure_count(
        crate::tes::ai_package::package_data(world, package).field_18 as i32,
    );
    if index >= length {
        return set_procedure_index(world, process, length - 1);
    }
    Ok(())
}

/// IDA: 005EDA20(0): `base_and_race_abilities`' walk, each spell through the caster's vt+0 and vt+4, which cast it (an ability or a disease). Completion: 100% of the walk; the casts are `CastBoundaries`.
fn apply_base_spells(world: &mut World, actor: Entity) -> Result<(), GameError> {
    if Some(actor) == crate::tes::player::the_player(world) {
        return Err(GameError::unclassified(
            0x005EDA97,
            "005EDA97: the player's birth sign",
        ));
    }
    let spells = base_and_race_abilities(world, actor);
    for spell in spells {
        world
            .get_resource_or_init::<CastBoundaries>()
            .casts
            .push((actor, spell));
        // 005EDB54 vt+0(spell, 1) and vt+4(spell, +0x68, 1): 0069BE00 casts an ability, 0069BE40 a disease.
        crate::tes::magic_cast::cast_on_self(world, actor, spell, true)?;
    }
    Ok(())
}
