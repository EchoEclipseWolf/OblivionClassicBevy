//! Phase 18: PlayerCharacter_SaveInitialState (006632A0), the player's save written to a FormHeap buffer before a game starts.
//!
//! The chain is the save game's own: 00661540 -> 005E7EF0 -> 00659C30 -> 004E2D60, then the process's 006324F0 -> 00651140 -> 00658CB0 -> 006471A0 -> 0060D1F0. Format version 125 (`g_saveLoadGame` +0x7C) takes every `>=` gate up to 0x7B, and skips `0045A170`'s block headers and 0x7E's arm; `byte_B05BAC` is 0, so there's no logging. An address's other arms are stops: a flag other than 4, or a list with an item. The only reader, 00663340 RestoreInitialState, isn't ported.
//!
//! Evidence: disassembly; `run-i-phase18/initial-state.bin`, byte for byte, in `the_port_matches_the_phase_18_rows`, but for bytes the exe never stores: the process's +0x88 and +0xD8..+0xDF, the controller's +0x2E0..+0x2EF and +0x31C, the actor's +0x84 and +0xD8, and the player's +0x5A8. They're heap bytes, the same in two runs, and the port writes its zeros. Fields the port has no slot for write literal zeros: +0xD4..+0xDC, +0x2E0, +0x31C.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::actor::{Actor, MobileObject, TesObjectRefr};
use super::actor_process::{
    BaseProcess, HighProcess, LowProcess, MiddleHighProcess, MiddleLowProcess,
};
use super::character_controller::BhkCharacterController;
use super::form::FormIdentity;
use super::form_components::{FormRef, TesFullName, TesRaceForm};
use super::forms_actors::TesNpc;
use super::forms_magic::{EffectSetting, EffectSettingCollection};
use super::game_settings::GameSettingCollection;
use super::hud_main_menu::npc_sex;
use super::player::{PlayerCharacter, PlayerGlobals};
use super::tes::TesGlobals;

// IDA: the format 0045B3CE stores at +0x7C.
const SAVE_FORMAT_VERSION: u8 = 125;
// IDA: 00421A00 and 00421340's types that write nothing with flags 4: 0x1A, the player's only extra at 0040F050.
const EXTRA_WRITES_NOTHING: u8 = 0x1A;
const SETTING_FLAG_SAVED: u32 = 0x20_0000;

// IDA: 0045B9A0 and 0045F7A0: +0x14 past each write. +0x7D is 0 through this call, so 0045F7A0 stores raw IDs and not 0045E0D0's.
#[derive(Default)]
struct SaveWriter {
    bytes: Vec<u8>,
}

impl SaveWriter {
    // IDA: N/A a helper over `Vec`.
    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    // IDA: N/A a helper over `Vec`.
    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    // IDA: N/A a helper over `Vec`.
    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    // IDA: N/A a helper over `Vec`.
    fn f32(&mut self, value: f32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    // IDA: N/A a helper over `Vec`.
    fn words(&mut self, values: &[u32]) {
        for value in values {
            self.u32(*value);
        }
    }

    // IDA: N/A a helper over `Vec`.
    fn floats(&mut self, values: &[f32]) {
        for value in values {
            self.f32(*value);
        }
    }
}

// IDA: a form slot a first start holds null, written as its ID of 0. Anything else is an arm no run reached.
fn null_id(word: u32, address: &'static str) -> Result<u32, StreamError> {
    if word == 0 {
        Ok(0)
    } else {
        Err(StreamError(address))
    }
}

// IDA: a `{item, next}` list head a first start holds empty.
fn empty_list(head: [u32; 2], address: &'static str) -> Result<(), StreamError> {
    if head == [0, 0] {
        Ok(())
    } else {
        Err(StreamError(address))
    }
}

// IDA: `*(form + 0x0C)`, or 0 for a null.
fn form_id(world: &World, form: Option<Entity>) -> u32 {
    form.and_then(|form| world.get::<FormIdentity>(form))
        .map_or(0, |identity| identity.form_id_0c)
}

// IDA: N/A the process's five class levels, read together.
struct ProcessView<'a> {
    base: &'a BaseProcess,
    low: &'a LowProcess,
    middle_low: &'a MiddleLowProcess,
    middle_high: &'a MiddleHighProcess,
    high: &'a HighProcess,
}

// IDA: N/A the entity's components, under `MobileObject` +0x58. The player's process is high.
fn process_view(world: &World, player: Entity) -> Result<ProcessView<'_>, StreamError> {
    let process = world
        .get::<MobileObject>(player)
        .and_then(|mobile| mobile.process_58)
        .ok_or(StreamError("00659C34: a player with no process"))?;
    Ok(ProcessView {
        base: world.get(process).expect("a process"),
        low: world.get(process).expect("a low process"),
        middle_low: world.get(process).expect("a middle low process"),
        middle_high: world.get(process).expect("a middle high process"),
        high: world
            .get(process)
            .ok_or(StreamError("00659C7B: a player without a high process"))?,
    })
}

// IDA: N/A the character controller, process vt+0x18C: 006348E0's +0x118.
fn controller<'a>(
    world: &'a World,
    process: &ProcessView<'_>,
) -> Option<&'a BhkCharacterController> {
    process
        .middle_high
        .char_controller_118
        .and_then(|entity| world.get(entity))
}

// IDA: 00421340 and 00421A00 on TESObjectREFR +0x44, with flags 4: the u16 count and nothing for type 0x1A. Each other type is an arm no run reached.
fn extras_write_nothing(world: &World, player: Entity) -> Result<(), StreamError> {
    let refr = world.get::<TesObjectRefr>(player).expect("a reference");
    for entry in &refr.extra_data_44.entries {
        if entry.type_04 != EXTRA_WRITES_NOTHING {
            return Err(StreamError("00421A00: an extra other than type 0x1A"));
        }
    }
    Ok(())
}

// IDA: 00416A80 and 00416BA0's walk: EffectSettingCollection's buckets in order, each chain in order, the keys of settings with flag 0x200000.
fn saved_effect_settings(world: &World) -> Vec<u32> {
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

// IDA: N/A the base's full name, through `TesFullName` or empty, as 004DA2A0.
fn base_full_name(world: &World, base: Entity) -> Vec<u8> {
    world
        .get::<TesFullName>(base)
        .and_then(|name| name.name_04.data_00.clone())
        .unwrap_or_default()
}

// IDA: base +0x104, resolved; 0x00B37C88 is the setting 00662244 looks a class up by.
fn base_class(world: &World, base: Entity) -> Result<u32, StreamError> {
    let npc = world.get::<TesNpc>(base).expect("the player's base");
    let class = match npc.class_104 {
        FormRef::Raw(id) => id,
        FormRef::Resolved(form) => form_id(world, form),
    };
    let default = world
        .resource::<GameSettingCollection>()
        .raw_at(0x00B3_7C88)
        .unwrap_or(0);
    if class != 0 && class == default {
        return Err(StreamError(
            "00662276: 0051C3B0, the default class's record",
        ));
    }
    Ok(class)
}

// IDA: 00659BE0 and its callees' constants at version 125: 00632370, 00650F50, 00658BC0, 00647060, 0060D0E0 and 00651AD0.
fn process_save_size(world: &World, process: &ProcessView<'_>) -> u32 {
    let package = if process.base.editor_package_08.is_some() {
        4
    } else {
        0
    };
    let base_process = 4 + package + 8;
    let low = base_process + 11 + 1 + 1 + 4 + 8 + 4 + 8 + 4 + 1;
    let middle_low = low + 4;
    let middle_high = middle_low + 2 + 5 + 4 + 51 + 2 + 8 + 1 + 2 + 10 + 1 + 4;
    let controller_size = controller(world, process).map_or(0, |_| 12 + 44 + 12 + 8);
    middle_high + 54 + 4 + 45 + 5 + 5 + 14 + controller_size + 2 + 30 + 8 + 4 + 24
}

// IDA: 0060E360 -> 005E7D00 at version 125, with 00659BE0 and 004E2BF0 (2 for the extras' count).
fn actor_save_size(world: &World, process: &ProcessView<'_>) -> u32 {
    (2 + 1) + process_save_size(world, process) + 7 + 2 + 4 + 4 + 5 + 4 + 4 + 14 + 1 + 1
}

// IDA: 006610F0 PlayerCharacter_GetSaveSize at version 125, with flags 4 and empty lists.
fn player_save_size(
    world: &World,
    player: &PlayerCharacter,
    process: &ProcessView<'_>,
    base: Entity,
) -> Result<u32, StreamError> {
    let npc = world.get::<TesNpc>(base).expect("the player's base");
    let chargen = if player.chargen_skill_exp_5b0.is_some() {
        21
    } else {
        0
    };
    let bonuses = player
        .attribute_bonuses_5b4
        .as_ref()
        .map_or(0, |list| list.items.len()) as u32;
    let mut size = 876 + actor_save_size(world, process);
    size += 102 + 136 + 1 + 12 + 5 + (4 * chargen + 1) + 10 + 5 + 4;
    size += 44 + 4 + 4 + 4 + 4 + 2 + 4 + 2 + 2;
    // 98 takes in the known topics' count, and 2 is the quest items'.
    size += 2 + 84 + 4 + 8 * bonuses + 98 + 2;
    size += 4 + 4 * saved_effect_settings(world).len() as u32;
    // 00523440: the four matrices at +0x108, and 21.
    size += 21
        + npc
            .facegen_108
            .iter()
            .map(|m| 4 * m.rows * m.cols)
            .sum::<u32>();
    size += base_full_name(world, base).len() as u32 + 1 + 1;
    size += 4 + 4;
    Ok(size)
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
) -> Result<(), StreamError> {
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
    out.f32(low.float_28);
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
) -> Result<(), StreamError> {
    write_low_process(out, world, process)?;
    out.u32(process.middle_low.field_90 as u32);
    Ok(())
}

// IDA: 00651140 at version 125 with flags 4. 0x134's word goes out as milliseconds over 1000, at 24 bits (D6); 0x136 and 0x128..0x130 are copied through the stack.
fn write_middle_high_process(
    out: &mut SaveWriter,
    world: &World,
    process: &ProcessView<'_>,
) -> Result<(), StreamError> {
    write_middle_low_process(out, world, process)?;
    let m = process.middle_high;
    out.u8(m.byte_114);
    out.u8(m.byte_115);
    out.u8(m.byte_14c);
    out.f32(m.actor_alpha_154);
    out.f32(m.float_158);
    out.u32(null_id(m.field_13c, "006512FE: the +0x13C form's ID")?);
    out.u32(m.field_e0);
    out.u8(m.byte_180);
    // +0xD4..+0xDC: 12 bytes the port has no field for; zero in the run.
    out.words(&[0; 3]);
    out.f32(m.float_c4);
    out.u16(m.word_138);
    out.u8(m.byte_11d);
    out.u8(m.byte_124);
    out.u32(null_id(m.field_120, "006513AB: the +0x120 form's ID")?);
    out.f32(f32::from(m.word_134) / 1000.0);
    out.u8(m.byte_136);
    out.words(&m.point_128);
    out.u8(m.knocked_state_11c);
    out.u32(null_id(
        m.queued_magic_item_144,
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
    out.f32(m.float_bc);
    empty_list([m.fields_a8[0], m.fields_a8[1]], "0065156E: the +0xA8 list")?;
    out.u16(0);
    out.u8(m.bytes_168[3]);
    out.u32(null_id(m.field_148, "006515DF: the +0x148 item")?);
    Ok(())
}

// IDA: 00651B90: the controller's fields, with flag 0x800 of +0x1F4 an arm no run reached, and vt+0x190 true for the player.
fn write_controller(
    out: &mut SaveWriter,
    controller: &BhkCharacterController,
) -> Result<(), StreamError> {
    out.u32(controller.context_field_1ec);
    out.u32(controller.field_2a0);
    out.u32(controller.flags_1f4);
    if controller.flags_1f4 & 0x800 != 0 {
        return Err(StreamError("00651C50: 005E1500, flag 0x800's vector"));
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

// IDA: 006324F0 at version 125 with flags 4.
fn write_high_process(
    out: &mut SaveWriter,
    world: &World,
    process: &ProcessView<'_>,
) -> Result<(), StreamError> {
    write_middle_high_process(out, world, process)?;
    let h = process.high;
    out.u8(h.byte_228);
    out.u8(h.byte_23c);
    out.u8(h.byte_25c);
    out.u8(h.byte_25d);
    out.u16(h.movement_flags_1fc);
    out.u16(h.current_action_1f4);
    out.u16(h.word_208);
    out.u32(h.field_1f0);
    out.f32(h.float_1ac);
    out.f32(h.float_204);
    out.f32(h.float_21c);
    out.f32(h.float_22c);
    out.f32(h.float_230);
    out.f32(h.float_234);
    out.f32(h.float_248);
    out.f32(h.float_198);
    out.f32(h.float_1b0);
    out.f32(h.float_1b4);
    out.f32(h.float_19c);
    out.u32(h.field_1cc);
    out.u8(h.byte_244);
    out.words(&h.point_20c);
    out.u32(h.unset_254);
    out.u32(h.field_2bc);
    out.f32(h.float_2c0);
    out.f32(h.swim_breath_238);
    out.f32(h.float_1a8);
    out.f32(h.float_1b8);
    out.f32(h.float_260);
    out.u8(h.byte_278);
    out.u32(h.field_274);
    out.u8(h.byte_290);
    out.f32(h.float_28c);
    out.u32(null_id(h.field_218, "00632815: the +0x218 form's ID")?);
    out.u32(null_id(h.field_1a4, "00632836: the +0x1A4 form's ID")?);
    out.u32(null_id(h.field_2c4, "0063285D: the +0x2C4 form's ID")?);
    empty_list(
        h.detection_list_18c.unwrap_or_default(),
        "0063289E: a detection entry",
    )?;
    out.u16(0);
    let size: u16 = controller(world, process).map_or(0, |_| 76);
    out.u16(size);
    if let Some(controller) = controller(world, process) {
        write_controller(out, controller)?;
    }
    for index in 0..5 {
        out.u32(null_id(
            h.action_targets_2c8[index],
            "006329CB: an action target",
        )?);
        out.u8(h.action_active_2dc[index]);
    }
    out.u32(null_id(h.field_2e4, "00632A1B: the +0x2E4 form's ID")?);
    out.u8(h.byte_2e8);
    out.f32(h.float_2ac);
    out.f32(h.float_2b0);
    out.u32(null_id(h.field_258, "00632A96: the +0x258 form's ID")?);
    out.u8(h.byte_1d0);
    out.f32(h.float_1d8);
    out.f32(h.float_1dc);
    out.f32(h.float_1e0);
    out.u8(h.byte_2a8);
    out.u8(h.byte_1e4);
    out.f32(h.float_1e8);
    out.f32(h.float_240);
    out.u8(h.byte_2a8);
    Ok(())
}

// IDA: 005E7EF0 at version 125 with flags 4: 00659C30 (process type byte 0, 004E2D60, the process), then the actor's own fields.
fn write_actor(
    out: &mut SaveWriter,
    world: &World,
    player: Entity,
    process: &ProcessView<'_>,
) -> Result<(), StreamError> {
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
    out.u8(actor.unset_in_water_d8);
    out.f32(actor.head_track_target_hold_timer_dc);
    out.f32(actor.head_track_target_scan_timer_100);
    out.u8(actor.death_effects_processed_fc);
    out.u8(actor.deferred_death_processing_c0);
    Ok(())
}

// IDA: 00526230: the four matrices row by row, the race, the hair and eyes with the hair's length and colour, then whether the NPC is female.
fn write_npc_face(out: &mut SaveWriter, world: &World, base: Entity) -> Result<(), StreamError> {
    let npc = world.get::<TesNpc>(base).expect("the player's base");
    for matrix in &npc.facegen_108 {
        for row in 0..matrix.rows {
            for col in 0..matrix.cols {
                out.f32(matrix.data[(col + row * matrix.cols) as usize]);
            }
        }
    }
    let race = world.get::<TesRaceForm>(base).expect("an NPC's race");
    out.u32(match race.race_04 {
        FormRef::Raw(id) => id,
        FormRef::Resolved(form) => form_id(world, form),
    });
    out.u32(form_id(world, npc.hair_1c8));
    out.u32(form_id(world, npc.eyes_1d0));
    out.f32(npc.hair_length_1cc);
    out.u32(npc.hair_color_1e8);
    out.u8(u8::from(npc_sex(world, base) == Some(1)));
    Ok(())
}

// IDA: 00661540 PlayerCharacter_SaveGame at version 125 with flags 4.
fn write_player(
    out: &mut SaveWriter,
    world: &World,
    player_entity: Entity,
    process: &ProcessView<'_>,
    base: Entity,
) -> Result<(), StreamError> {
    let p = world
        .get::<PlayerCharacter>(player_entity)
        .expect("the player");
    let globals = world.resource::<PlayerGlobals>();
    out.floats(&p.max_av_modifiers_204.0);
    out.floats(&p.script_av_modifiers_324.0);
    out.floats(&p.damage_av_modifiers_450.0);
    out.f32(p.health_444);
    out.f32(p.magicka_448);
    out.f32(p.stamina_44c);
    write_actor(out, world, player_entity, process)?;
    out.u8(p.view_flags_588[0]);
    out.u8(p.view_flags_588[1]);
    out.u8(p.view_flags_588[3]);
    out.u8(p.view_flags_588[4]);
    out.u32(p.pc_sleep_hours_590 as u32);
    out.f32(p.athletics_run_use_timer_59c);
    out.f32(p.athletics_swim_use_timer_5a0);
    out.f32(p.sneak_use_timer_5a4);
    out.u8(p.unset_sneak_skill_use_actor_nearby_5a8);
    out.u8(p.dialogue_cooldown_active_600);
    out.f32(p.unset_dialogue_cooldown_elapsed_604);
    out.u32(p.prison_days_remaining_608 as u32);
    out.u32(p.field_60c);
    out.u8(p.jail_guard_pursuit_active_610);
    out.u8(p.is_a_murderer_611);
    out.u32(p.amount_sold_stolen_614 as u32);
    out.u8(p.byte_620);
    out.f32(p.z_rotation_offset_61c);
    out.words(&p.custom_marker_position_62c);
    out.f32(p.camera_eye_height_offset_5d4);
    out.f32(p.bow_draw_timer_640);
    out.u32(p.training_sessions_used_5bc);
    out.u32(p.field_648);
    out.u32(p.field_64c);
    out.u8(p.pc_sleep_hours_set_594);
    out.f32(p.camera_fov_598);
    out.u8(p.vampire_has_fed_6e4);
    out.u8(p.is_in_chargen_6e5);
    out.u8(p.telekinesis_targeting_enabled_6e6);
    out.u32(p.fame_6f4 as u32);
    out.u32(p.infamy_6f8 as u32);
    out.u32(p.infamy_bounty_accumulator_6fc as u32);
    out.u32(p.play_time_milliseconds_714);
    out.u8(p.byte_5c0);
    out.words(&p.misc_stats_658.0[..28]);
    out.words(&p.misc_stats_658.0[28..]);
    out.u8(p.guard_combat_active_738);
    out.u32(p.grab_mode_57c);
    out.f32(p.field_580);
    out.f32(p.grab_distance_584);
    out.u32(p.load_door_follower_move_count_110);
    out.u8(p.jail_sentence_active_200);
    match &p.chargen_skill_exp_5b0 {
        Some(experience) => {
            out.u8(21);
            out.floats(experience);
        }
        None => out.u8(0),
    }
    out.u8(p.view_flags_588[2]);
    out.u8(p.fast_travel_enabled_5a9);
    out.u32(p.initial_timestamp_low_718);
    out.u32(p.cached_sound_form_id_760);
    // 00B3BAEA and 00B3BAFC.
    out.u8(0);
    out.f32(globals.b3bafc);
    out.f32(globals.vanity_wheel_b3bb24);
    out.u32(null_id(
        p.dialogue_package_118,
        "00661B1E: the dialogue package",
    )?);
    out.u32(null_id(p.birthsign_644, "00661B41: the birthsign")?);
    // 00B3BAD0, null at 0040F050 in the run.
    out.u32(0);
    out.u32(form_id(world, p.active_magic_item_624));
    out.u32(null_id(
        p.casting_magic_item_1e8,
        "00661BD0: the casting item",
    )?);
    out.u32(null_id(
        p.casting_target_1ec,
        "00661BF5: the casting target",
    )?);
    out.u32(null_id(
        p.last_ridden_horse_1e0,
        "00661C0C: the last horse",
    )?);
    if p.region_6e8.is_some() {
        return Err(StreamError("00661C2F: the player's region"));
    }
    out.u32(0);
    out.u32(null_id(p.book_628, "00661C52: the book")?);
    out.u32(null_id(
        p.chargen_suggested_class_650,
        "00661C75: the suggested class",
    )?);
    out.u32(null_id(p.alchemy_item_6e0, "00661C98: the alchemy item")?);
    out.u32(null_id(
        p.grabbed_reference_578,
        "00661CFA: the grabbed reference",
    )?);
    // 00B3BAD4, null at 0040F050 in the run.
    out.u32(0);
    out.u32(null_id(
        p.last_activated_load_door_570,
        "00661D55: the load door",
    )?);
    out.u32(null_id(
        p.custom_marker_location_638,
        "00661D83: the marker's location",
    )?);
    if p.combat_opponents_5ac != 0 {
        return Err(StreamError("00661DC8: a combat opponent"));
    }
    out.u16(0);
    out.u32(0);
    if p.random_teleport_index_by_door_788.count_0c != 0 {
        return Err(StreamError("00661EBA: a random teleport entry"));
    }
    out.u16(0);
    // 00B3BB44's list, empty at 0040F050 in the run.
    out.u16(0);
    empty_list(
        p.active_effects_1e4.unwrap_or_default(),
        "0068DD30: a player's active effect",
    )?;
    out.u16(0);
    out.floats(&p.skill_exp_130);
    // 00661F8F counts a node with a block, whatever the block holds.
    let bonuses: &[[u32; 2]] = p
        .attribute_bonuses_5b4
        .as_ref()
        .map_or(&[], |list| &list.items);
    out.u32(bonuses.len() as u32);
    for bonus in bonuses {
        out.words(bonus);
    }
    out.u16(p.skill_advance_counts_5b8);
    out.u8(p.stealth_skill_advance_count_5ba);
    out.words(&p.skill_advances_188);
    out.u32(p.major_skill_advances_184 as u32);
    out.u8(p.can_level_up_1dc);
    out.u32(null_id(p.active_quest_5f4, "00662071: the active quest")?);
    empty_list(p.known_topics_5e4, "006620AE: a known topic")?;
    out.u16(0);
    empty_list(
        p.known_quest_stage_items_5ec,
        "00662150: a quest stage item",
    )?;
    out.u16(0);
    // 00416BA0.
    let settings = saved_effect_settings(world);
    out.u32(settings.len() as u32);
    out.words(&settings);
    write_npc_face(out, world, base)?;
    let name = base_full_name(world, base);
    out.u8((name.len() + 1) as u8);
    out.bytes.extend_from_slice(&name);
    out.u8(0);
    out.u32(base_class(world, base)?);
    out.u32(null_id(
        p.previous_chargen_class_654,
        "0066229C: the previous class",
    )?);
    Ok(())
}

/// IDA: 006632A0 PlayerCharacter_SaveInitialState: +0x70C set warns and returns. 004533F0 the player's change flags, `0x4` in the run; vt+0x4C 006610F0 for the size; 00453500 the buffer of size + 4 into +0x70C, 0046AC70 the flags, +0x7D = 0, vt+0x50 00661540, +0x7D = 1 and +0x14 = 0. The buffer is `PlayerCharacter::initial_state`. Completion: 100% of a first start at version 125 with flags 4.
/// # Errors
/// A flag or a list other than a first start's, or a size that disagrees with the bytes, which the original would overrun.
pub fn player_save_initial_state(world: &mut World, player: Entity) -> Result<(), StreamError> {
    {
        let data = world.get::<PlayerCharacter>(player).expect("the player");
        if data.initial_state.is_some() {
            return Err(StreamError("006632B2: the initial state already exists"));
        }
    }
    let id = world
        .get::<FormIdentity>(player)
        .expect("a form")
        .form_id_0c;
    let globals = world.resource::<TesGlobals>();
    let save = globals
        .save_load_game
        .as_ref()
        .ok_or(StreamError("006632C6: a null g_saveLoadGame faults"))?;
    if save.byte_7c != SAVE_FORMAT_VERSION {
        return Err(StreamError("0045A170: a format version other than 125"));
    }
    if save.field_04 != 0 {
        return Err(StreamError("004533FF: the save load's own map"));
    }
    let flags = save.changes_00.get(id).map_or(0, |record| record.flags_00);
    if flags != 4 {
        return Err(StreamError("00661540: player change flags other than 4"));
    }
    let base = world
        .get::<TesObjectRefr>(player)
        .and_then(|refr| refr.base_form_1c)
        .ok_or(StreamError("006613F3: a null base form faults"))?;
    let bytes = {
        let process = process_view(world, player)?;
        let data = world.get::<PlayerCharacter>(player).expect("the player");
        let size = player_save_size(world, data, &process, base)? & 0xFFFF;
        let mut out = SaveWriter::default();
        out.u32(flags);
        write_player(&mut out, world, player, &process, base)?;
        if out.bytes.len() as u32 != size + 4 {
            return Err(StreamError("006632E9: the save's size and bytes disagree"));
        }
        out.bytes
    };
    {
        let mut data = world
            .get_mut::<PlayerCharacter>(player)
            .expect("the player");
        data.initial_state = Some(bytes);
        data.initial_state_buffer_70c = 1;
    }
    let mut globals = world.resource_mut::<TesGlobals>();
    let save = globals.save_load_game.as_mut().expect("read above");
    save.byte_7d = 1;
    save.fields_14[0] = 0;
    Ok(())
}
