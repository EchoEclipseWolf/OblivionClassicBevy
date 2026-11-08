//! Phase 14c: the condition walk of the AI pass: 0056A510 TESCondition_Evaluate, 0056AC50 TESConditionItem_Evaluate, 0056A3B0 and the handlers of the 00B0C8C0 table that package and idle conditions reach. The table is `condition_tables`, generated from the exe.
//!
//! A handler is an arm of a `match` on the table's handler address. Where a form type would make the original read through a wrong pointer, the host stops: the exe has no test there. A handler with no arm stops with its address.
//! Evidence: objdump of each address named here; P14-RUN-004's ACT rows.

use bevy_ecs::prelude::*;
use recomp_core::crt;
use recomp_core::reader::StreamError;

use super::actor::{Actor, Character, Creature, MobileObject, TesObjectRefr};
use super::actor_process::{BaseProcess, LOW_PROCESS_VTABLE, LowProcess, MiddleHighProcess};
use super::ai_package::{
    FLT_MAX, actor_base_form, actor_is_dead, game_day_of_week, game_hour, parent_cell,
    procedure_at, reference_position, reference_world,
};
use super::ai_process::{AiArms, AiProcessGlobals, actor_get_actor_value_float, base_actor_value};
use super::base_extra_list::{EXTRA_LOCK, EXTRA_TELEPORT, ExtraData};
use super::condition_tables::CONDITION_FUNCTIONS;
use super::default_forms::DefaultFormStatics;
use super::form::{FormIdentity, OriginalVtable};
use super::form_arms::editor_id_of;
use super::form_components::{FormRef, TesActorBaseData};
use super::forms_ai::{TesCondition, TesConditionItem, TesPackage};
use super::forms_dialogue::TesQuest;
use super::forms_init::{effective_level, form_type};
use super::forms_misc::TesFaction;
use super::forms_world::{TesObjectCell, cell_is_interior};
use super::global::TESGlobal;
use super::inventory::{ensure_container_changes, put_container_changes, reference_container};
use super::player::{PlayerCharacter, PlayerGlobals};
use super::sky::SkyGlobals;
use super::tes::TesGlobals;

/// IDA: the sticky caches of the handlers, never cleared in this window. 4F5DB0, under 0040D4D0, clears them with no run in between here. The store orders are the handlers'.
#[derive(Debug, Default)]
pub struct ConditionCaches {
    /// 00B3618C, 00B36190, 00B36194: 004F6D70.
    pub in_cell_subject: Option<Entity>,
    pub in_cell_cell: Option<Entity>,
    pub in_cell_result: f32,
    /// 00B3619C, 00B36198, 00B361A0: 004F70C0.
    pub in_faction_subject: Option<Entity>,
    pub in_faction_faction: Option<Entity>,
    pub in_faction_result: f32,
}

/// IDA: N/A a form's +0x08.
#[must_use]
pub fn form_flags(world: &World, form: Entity) -> u32 {
    world.get::<FormIdentity>(form).expect("a form").flags_08
}

// IDA: vt+0x190 IsActor.
fn is_actor(world: &World, form: Entity) -> bool {
    world.get::<Actor>(form).is_some()
}

/// IDA: 00B333C4 g_thePlayer.
#[must_use]
pub fn the_player(world: &World) -> Option<Entity> {
    world
        .get_resource::<PlayerGlobals>()
        .and_then(|globals| globals.the_player_b333c4)
}

// IDA: N/A a helper, with no address of its own.
fn player_data(world: &World) -> Option<&PlayerCharacter> {
    the_player(world).and_then(|player| world.get::<PlayerCharacter>(player))
}

/// IDA: N/A an actor's process +0x58.
#[must_use]
pub fn process_of(world: &World, actor: Entity) -> Option<Entity> {
    world
        .get::<MobileObject>(actor)
        .and_then(|mobile| mobile.process_58)
}

/// IDA: N/A a process slot this slice ports for the LowProcess only: 'where' is the call that takes another vtable.
pub fn require_low(
    world: &World,
    process: Entity,
    where_: &'static str,
) -> Result<(), StreamError> {
    match world.get::<OriginalVtable>(process) {
        Some(OriginalVtable(LOW_PROCESS_VTABLE)) => Ok(()),
        _ => Err(StreamError(where_)),
    }
}

/// IDA: vt+0x18C 005E0EC0: process vt+0x36C, 0 without a process. Low and MiddleLow 006F7070 answer 0; MiddleHigh and High 0064B090 the +0x11D byte. Completion: 100%.
#[must_use]
pub fn actor_sitting_state(world: &World, actor: Entity) -> u8 {
    process_of(world, actor)
        .and_then(|process| world.get::<MiddleHighProcess>(process))
        .map_or(0, |middle| middle.byte_11d)
}

// IDA: process vt+0x184 and vt+0x180 on MiddleHigh and High, 0064B040 and 0064B020: +0xC0 when nonzero, with +0xCC. Nothing the port runs sets +0xC0, so a nonzero one is a host stop rather than a silent +0x08. Low and MiddleLow read +0x08 and +0x04 only.
pub fn stop_on_process_package_override(world: &World, process: Entity) -> Result<(), StreamError> {
    if world
        .get::<MiddleHighProcess>(process)
        .is_some_and(|middle| middle.current_package_c0 != 0)
    {
        return Err(StreamError(
            "0064B040: a process's +0xC0 package overrides +0x08",
        ));
    }
    Ok(())
}

// IDA: 00467510 TESActorBaseData_GetFactionRank(faction, isPlayer): the first entry naming the faction, with the player skipping a faction with +0x34 bit 3; -1 with none. Completion: 100%.
fn faction_rank(
    world: &World,
    data: &TesActorBaseData,
    faction: Entity,
    is_player: bool,
) -> Result<i32, StreamError> {
    for entry in &data.factions_18 {
        let FormRef::Resolved(listed) = entry.faction_00 else {
            if entry.faction_00.is_empty() {
                continue;
            }
            return Err(StreamError("00467530: an unresolved faction"));
        };
        if is_player
            && listed
                .and_then(|listed| world.get::<TesFaction>(listed))
                .is_some_and(|data| data.flags_34 & 8 != 0)
        {
            continue;
        }
        if listed == Some(faction) {
            return Ok(i32::from(entry.rank_04));
        }
    }
    Ok(-1)
}

/// IDA: 00467510 on a base form, from outside this file.
pub fn base_faction_rank(
    world: &World,
    base: Entity,
    faction: Entity,
    is_player: bool,
) -> Result<i32, StreamError> {
    match world.get::<TesActorBaseData>(base) {
        Some(data) => faction_rank(world, data, faction, is_player),
        None => Ok(-1),
    }
}

// IDA: vt+0xA4 IsBoundObject, 977C50 in the vtables of TESSound (0x0A), 0x12..=0x2B and TESLevSpell (0x40). An RTTI scan confirms it for all of them, 0x1F included. Formerly: the class of 0x1F wasn't confirmed by its vtable; it's in with its range. The C++'s range is 0x13..=0x2B. Completion: 100%, with that note.
pub(super) const fn bound_object(form_type: u8) -> bool {
    matches!(form_type, 0x0A | 0x12..=0x2B | 0x40)
}

/// IDA: 0041E690 the 0x31 extra's block (bytes: +0x08 the flags), or null; a null-block entry can't arise here, since `load_lock` keeps one. Completion: 100%.
#[must_use]
pub fn lock_block(world: &World, reference: Entity) -> Option<&[u8; 12]> {
    let list = &world.get::<TesObjectRefr>(reference)?.extra_data_44;
    match &list.get(EXTRA_LOCK)?.data {
        ExtraData::Lock(block) => Some(block),
        _ => None,
    }
}

/// IDA: 0041E6B0 the 0x32 extra's door, with 0x32 holding the linked reference.
#[must_use]
pub fn teleport_door(world: &World, reference: Entity) -> Option<Entity> {
    let entry = world
        .get::<TesObjectRefr>(reference)?
        .extra_data_44
        .get(EXTRA_TELEPORT)?;
    entry.resolved_0c.flatten()
}

/// IDA: 004D7740 TESObjectREFR_GetLockData: the reference's lock; without one the linked door's. Completion: 100%.
#[must_use]
pub fn reference_lock(world: &World, reference: Entity) -> Option<&[u8; 12]> {
    if let Some(block) = lock_block(world, reference) {
        return Some(block);
    }
    lock_block(world, teleport_door(world, reference)?)
}

/// IDA: 004D7E30: the point minus the position per axis, each a float store; `(x*x + y*y) + z*z`; `__CIsqrt`, which keeps the precision bits, so the root rounds to float. Completion: 100%.
#[must_use]
pub fn distance_to(position: [f32; 3], point: [f32; 3]) -> f32 {
    let dx = point[0] - position[0];
    let dy = point[1] - position[1];
    let dz = point[2] - position[2];
    ((dx * dx + dy * dy) + dz * dz).sqrt()
}

/// IDA: 004D7E90 TESObjectREFR_GetDistance(other, includeDisabled): FLT_MAX for a disabled (without the flag) or deleted other, different interiors, or different or null exterior worlds; otherwise 004D7E30 to the other's vt+0x174. Completion: 100%.
pub fn reference_distance(
    world: &World,
    this: Entity,
    other: Entity,
    include_disabled: bool,
) -> Result<f32, StreamError> {
    let flags = form_flags(world, other);
    if (flags & 0x800 != 0 && !include_disabled) || flags & 0x20 != 0 {
        return Ok(FLT_MAX);
    }
    let other_cell = parent_cell(world, other);
    let this_cell = parent_cell(world, this);
    if let (Some(other_cell), Some(this_cell)) = (other_cell, this_cell)
        && (cell_is_interior(world, other_cell) || cell_is_interior(world, this_cell))
    {
        if this_cell != other_cell {
            return Ok(FLT_MAX);
        }
    } else {
        let other_world = reference_world(world, other);
        let this_world = reference_world(world, this);
        if this_world != other_world || this_world.is_none() {
            return Ok(FLT_MAX);
        }
    }
    let position = world.get::<TesObjectRefr>(this).expect("a ref").position_2c;
    Ok(distance_to(position, reference_position(world, other)?))
}

/// IDA: 004869C0 ContainerChanges_GetItemCount(item): |00469CA0 the base container's first entry of the item| plus the changes entry's +0x04; with a changes entry and both zero, 1. The owner has a container on every path here: 004F48F0 tests it. Completion: 100%.
pub(super) fn item_count(world: &World, reference: Entity, item: Entity) -> i32 {
    let mut result = 0_i32;
    if let Some(container) = reference_container(world, reference)
        .and_then(|base| world.get::<super::form_components::TesContainer>(base))
    {
        for entry in &container.entries_08 {
            if matches!(entry.item, FormRef::Resolved(Some(form)) if form == item) {
                result = entry.count;
                break;
            }
        }
    }
    result = result.wrapping_abs();
    let refr = world.get::<TesObjectRefr>(reference).expect("a reference");
    let Some(entry) = refr
        .extra_data_44
        .get(super::base_extra_list::EXTRA_CONTAINER_CHANGES)
    else {
        return result;
    };
    let ExtraData::ContainerChanges(changes) = &entry.data else {
        return result;
    };
    for change in &changes.entries_00 {
        if change.form_08 == item {
            if result != 0 || change.count_04 != 0 {
                return result.wrapping_add(change.count_04);
            }
            return 1;
        }
    }
    result
}

// IDA: N/A a second form parameter: +0x10 after 0056B2E0.
fn second_form(item: &TesConditionItem) -> Option<Entity> {
    item.parameters_resolved[1].flatten()
}

// IDA: `lea eax,[form+4]`, 49..=51: a reference's form types, as the handlers test them.
fn is_reference_type(world: &World, form: Entity) -> bool {
    form_type(world, form).wrapping_sub(49) <= 2
}

// IDA: the +0x24 value of a global.
fn global_value(world: &World, form: Entity) -> Option<f32> {
    world
        .get::<TESGlobal>(form)
        .map(|global| f32::from_bits(global.value_bits_24))
}

// IDA: 00484F50 the 0x12 extra's event list, and whether it has one.
fn script_variable(
    world: &mut World,
    parameter: Option<Entity>,
    index: u32,
) -> Result<f64, StreamError> {
    let Some(parameter) = parameter else {
        return Ok(0.0);
    };
    let kind = form_type(world, parameter);
    if kind < 0x31 {
        return Ok(0.0);
    }
    let events = if kind <= 0x33 {
        let list = &world
            .get::<TesObjectRefr>(parameter)
            .expect("a reference")
            .extra_data_44;
        if list
            .get(super::base_extra_list::EXTRA_REFERENCE_POINTER)
            .is_some()
        {
            return Err(StreamError(
                "004F6745: GetScriptVariable through an ExtraReferencePointer",
            ));
        }
        match list.get(0x12).map(|entry| &entry.data) {
            Some(ExtraData::Script { events_10, .. }) => events_10.as_ref(),
            _ => None,
        }
    } else if kind == 0x3B {
        world
            .get::<TesQuest>(parameter)
            .ok_or(StreamError("004F676B: a type 0x3B form that is no quest"))?
            .events_58
            .as_ref()
    } else {
        return Ok(0.0);
    };
    let Some(events) = events else {
        return Ok(0.0);
    };
    // 004FA110: the walk, and a miss warns (4FA179) and answers 0.0. The 00B361BC, 00B09E20, 00B361C0 cache can only tell if a list is reallocated at the same address.
    if let Some(variable) = events
        .variables_0c
        .iter()
        .find(|variable| variable.id_00 == index)
    {
        return Ok(f64::from_bits(variable.value_08));
    }
    let text = format!("Variable ID {index:08X} not found");
    world.resource_mut::<AiProcessGlobals>().warnings.push(text);
    Ok(0.0)
}

// IDA: 00542EA0 and the Sky's weathers: only null ones are ported.
fn sky_weathers_null(world: &World, where_: &'static str) -> Result<(), StreamError> {
    let sky = world
        .get_resource::<SkyGlobals>()
        .and_then(|globals| globals.singleton_b365c4.as_ref())
        .ok_or(StreamError("00542EA0: Sky_GetSingleton creating the sky"))?;
    if sky.first_weather_10 != 0 || sky.weather_14 != 0 {
        return Err(StreamError(where_));
    }
    Ok(())
}

// IDA: the 0056AC50 handler arm (subject, parameter, item +0x10, &double), by the table's +0x20. Every handler here returns 1 in al, so the compare always follows; the only false ones in the table are 004F7F50 (a null `[B333A0+0x5C]`) and 004F57A0 (a null subject, which the flag refuses), with no arm here. Completion: 42 of the 166 handlers, all that startup data reaches, each read in full; the others stop.
// TODO: [ ] PACK conditions no arm takes: 030 GetGold 004F4A00, 084 GetPCInFaction 004F71C0, 0B4 GetDetectionLevel 004F5650, 0B6 GetEquipped 004F4980, 0F9 GetPCFame 004F5960. They are in shipped PACK records but no startup run reached them. Idle-only handlers: 31 more, later than this window.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per handler, as the table has them"
)]
pub(super) fn run_handler(
    world: &mut World,
    index: u16,
    subject: Option<Entity>,
    parameter: Option<Entity>,
    item: &TesConditionItem,
    arms: &mut dyn AiArms,
) -> Result<f64, StreamError> {
    world.resource_mut::<AiProcessGlobals>().condition_calls[usize::from(index)] += 1;
    let raw_0c = item.parameter(0);
    let raw_10 = item.parameter(1);
    let actor = subject.filter(|&form| is_actor(world, form));
    let reference = subject.filter(|&form| world.get::<TesObjectRefr>(form).is_some());
    let player = the_player(world);
    Ok(match CONDITION_FUNCTIONS[usize::from(index)].handler_20 {
        // 004F4B70 GetStage: 00529750, +0x5C.
        0x004F_4B70 => {
            let Some(parameter) = parameter else {
                return Ok(0.0);
            };
            f64::from(
                world
                    .get::<TesQuest>(parameter)
                    .ok_or(StreamError("004F4B70: GetStage on a non-quest"))?
                    .byte_5c,
            )
        }
        // 004F67A0 GetQuestRunning: +0x3C bit 0.
        0x004F_67A0 => {
            let Some(parameter) = parameter else {
                return Ok(0.0);
            };
            let quest = world
                .get::<TesQuest>(parameter)
                .ok_or(StreamError("004F67A0: GetQuestRunning on a non-quest"))?;
            if quest.flags_3c & 1 != 0 { 1.0 } else { 0.0 }
        }
        // 004F70C0 GetInFaction, with its cache: the hit test runs before the subject's, and a null or non-actor subject stores nothing.
        0x004F_70C0 => {
            let faction = parameter.filter(|&form| form_type(world, form) == 6);
            let globals = world.resource::<AiProcessGlobals>();
            if subject == globals.caches.in_faction_subject
                && faction == globals.caches.in_faction_faction
            {
                return Ok(f64::from(globals.caches.in_faction_result));
            }
            let Some(actor) = actor else {
                return Ok(0.0);
            };
            let mut base = actor_base_form(world, actor, true)?;
            if base
                .and_then(|base| world.get::<TesActorBaseData>(base))
                .is_some_and(|data| data.factions_18.is_empty())
            {
                base = actor_base_form(world, actor, false)?;
            }
            let mut result = 0.0;
            if let (Some(base), Some(faction)) = (base, faction)
                && base_faction_rank(world, base, faction, Some(actor) == player)? != -1
            {
                result = 1.0;
            }
            let caches = &mut world.resource_mut::<AiProcessGlobals>().caches;
            caches.in_faction_subject = subject;
            caches.in_faction_result = result as f32;
            caches.in_faction_faction = faction;
            result
        }
        // 004F4EF0 GetRandomPercent: 0047DF80(0), signed % 100.
        0x004F_4EF0 => {
            let id = subject.map_or(0, |form| {
                world.get::<FormIdentity>(form).expect("a form").form_id_0c
            });
            let value = arms.random_next(world)?;
            let mut globals = world.resource_mut::<AiProcessGlobals>();
            globals.random_calls += 1;
            globals.random_subjects.push(id);
            f64::from(value % 100)
        }
        // 004F4CE0 GetIsID: the parameter must be bound (vt+0xA4), which the C++ doesn't test; the subject's vt+0x170 equals it.
        0x004F_4CE0 => {
            let (Some(subject), Some(parameter)) = (subject, parameter) else {
                return Ok(0.0);
            };
            if !bound_object(form_type(world, parameter)) {
                return Ok(0.0);
            }
            let data = world
                .get::<TesObjectRefr>(subject)
                .ok_or(StreamError("004F4CE0: GetIsID on a non-reference subject"))?;
            if data.base_form_1c == Some(parameter) {
                1.0
            } else {
                0.0
            }
        }
        // 004F6710 GetScriptVariable and GetQuestVariable.
        0x004F_6710 => script_variable(world, parameter, raw_10)?,
        // 004F43C0 GetDistance.
        0x004F_43C0 => {
            let other = parameter.filter(|&form| world.get::<TesObjectRefr>(form).is_some());
            let (Some(this), Some(other)) = (reference, other) else {
                return Ok(0.0);
            };
            f64::from(reference_distance(world, this, other, true)?)
        }
        // 004F4760 GetLocked: 004D7740 and bit 0 of the block's +0x08. The C++ stops on any subject with no lock of its own.
        0x004F_4760 => {
            let Some(reference) = reference else {
                return Ok(0.0);
            };
            match reference_lock(world, reference) {
                Some(block) if block[8] & 1 != 0 => 1.0,
                _ => 0.0,
            }
        }
        // 004F5010 GetDeadCount: 00440F70 on TES +0x8C.
        0x004F_5010 => {
            if parameter.is_none() {
                return Ok(0.0);
            }
            let tes = world
                .get_resource::<TesGlobals>()
                .and_then(|globals| globals.tes.as_ref())
                .ok_or(StreamError("004F5010: GetDeadCount without g_tes"))?;
            if tes.actor_base_death_counts_8c != [0; 2] {
                return Err(StreamError("00440F70: a nonempty actor death-count list"));
            }
            0.0
        }
        // 004F69D0 IsRaining and 004F68F0 IsPleasant: null weathers answer 0.
        0x004F_69D0 => {
            sky_weathers_null(world, "004F5ED0: IsRaining with a weather")?;
            0.0
        }
        0x004F_68F0 => {
            sky_weathers_null(world, "004F68F0: IsPleasant with a weather")?;
            0.0
        }
        // 004F48F0 GetItemCount: a bound parameter, the subject's container and its created changes, |004869C0|.
        0x004F_48F0 => {
            let (Some(parameter), Some(reference)) = (parameter, reference) else {
                return Ok(0.0);
            };
            if !bound_object(form_type(world, parameter))
                || reference_container(world, reference).is_none()
            {
                return Ok(0.0);
            }
            let changes = ensure_container_changes(world, reference)?;
            put_container_changes(world, reference, changes);
            f64::from(item_count(world, reference, parameter).wrapping_abs())
        }
        // 004F5D20 IsPlayersLastRiddenHorse: the player's +0x1E0, raw and null here.
        0x004F_5D20 => {
            if actor.is_none() {
                return Ok(0.0);
            }
            let Some(data) = player_data(world) else {
                return Ok(0.0);
            };
            if data.last_ridden_horse_1e0 != 0 {
                return Err(StreamError("004F5D3E: a last ridden horse"));
            }
            0.0
        }
        // 004F5E60 IsPlayerMovingIntoNewSpace and 004F87F0 GetPlayerInSEWorld.
        0x004F_5E60 => {
            let data = player_data(world).ok_or(StreamError("004F5E60: without g_thePlayer"))?;
            f64::from(u8::from(data.is_moving_into_new_space_12c != 0))
        }
        0x004F_87F0 => {
            let data = player_data(world).ok_or(StreamError("004F87F0: without g_thePlayer"))?;
            f64::from(u8::from(data.is_in_se_world_116 != 0))
        }
        // 004F4C80 GetAttacked: vt+0x390: 005E58C0's process vt+0x398 (Low 0069D990: 0), or the player's 0065DAD0: byte +0x620.
        0x004F_4C80 => {
            let Some(actor) = actor else {
                return Ok(0.0);
            };
            if Some(actor) == player {
                return Ok(f64::from(player_data(world).expect("the player").byte_620));
            }
            let process =
                process_of(world, actor).ok_or(StreamError("005E58C5: a null process faults"))?;
            require_low(world, process, "005E58C5: another process's vt+0x398")?;
            0.0
        }
        // 004F5060 GetIsAlerted: 005E0E30, process vt+0x210 00629300: +0x1F.
        0x004F_5060 => {
            let process = actor.and_then(|actor| process_of(world, actor));
            process
                .and_then(|process| world.get::<LowProcess>(process))
                .map_or(0.0, |low| f64::from(low.byte_1f))
        }
        // 004F4B10 GetTalkedToPC: an actor subject, or with no subject the parameter; vt+0x7C 00412980: bit 6.
        0x004F_4B10 => {
            let form = if subject.is_some() { actor } else { parameter };
            form.map_or(0.0, |form| {
                f64::from(u8::from(form_flags(world, form) & 0x40 != 0))
            })
        }
        // 004F6060 GetActorValue: a disabled actor asks its base (vt+0x12C 0051E790: fild of vt+0x128), else vt+0x288.
        0x004F_6060 => {
            let Some(actor) = actor else {
                return Ok(0.0);
            };
            if form_flags(world, actor) & 0x800 != 0 {
                let base = actor_base_form(world, actor, false)?
                    .ok_or(StreamError("004F6089: no base faults"))?;
                return Ok(f64::from(base_actor_value(world, base, raw_0c)?));
            }
            f64::from(actor_get_actor_value_float(world, actor, raw_0c)?)
        }
        // 004F64E0 GetSitting: vt+0x380 nonnull gives 10.0, otherwise vt+0x27C (Character 1, Player 2, Creature 0) zero tests the base's +0x104 == 4; then the 004F659C table adds by vt+0x18C's state: 1, 2, 2, 3, 4 for 1..=5.
        0x004F_64E0 => {
            let Some(actor) = actor else {
                return Ok(0.0);
            };
            let mut result = 0.0;
            if world.get::<Character>(actor).is_some() {
                if world
                    .get::<Actor>(actor)
                    .expect("an actor")
                    .horse_or_rider_d4
                    != 0
                {
                    result = 10.0;
                }
            } else if world.get::<Creature>(actor).is_some()
                && world
                    .get::<TesObjectRefr>(actor)
                    .expect("an actor")
                    .base_form_1c
                    .and_then(|base| world.get::<super::forms_actors::TesCreature>(base))
                    .is_some_and(|creature| creature.data_104[0] == 4)
            {
                result = 10.0;
            }
            result
                + match actor_sitting_state(world, actor) {
                    1 => 1.0,
                    2 | 3 => 2.0,
                    4 => 3.0,
                    5 => 4.0,
                    _ => 0.0,
                }
        }
        // 004F8F30 IsInCombat: the player's 006605A0(0), or vt+0x334(1) 005E6110: a current package of type 0x0C or 0x0D.
        0x004F_8F30 => {
            let Some(actor) = actor else {
                return Ok(0.0);
            };
            if Some(actor) == player {
                if player_data(world).expect("the player").combat_opponents_5ac != 0 {
                    return Err(StreamError("006605A0: the player's combat opponents"));
                }
                return Ok(0.0);
            }
            // 005E6110(1): with no mount, a nonzero process vt+0x36C (`0064B090`, +0x11D) answers false before the type is read.
            if actor_sitting_state(world, actor) != 0 {
                return Ok(0.0);
            }
            let package = process_of(world, actor)
                .and_then(|process| world.get::<BaseProcess>(process))
                .and_then(|base| base.editor_package_08);
            match package.and_then(|package| world.get::<TesPackage>(package)) {
                Some(package) if package.type_20 == 12 || package.type_20 == 13 => 1.0,
                _ => 0.0,
            }
        }
        // 004F8B90 IsCellOwner(cell, owner): 004CA970 the cell's ownership. The exe has no class test on the parameter.
        0x004F_8B90 => {
            let cell = parameter
                .and_then(|form| world.get::<TesObjectCell>(form))
                .ok_or(StreamError("004F8B90: IsCellOwner without a cell"))?;
            let owner = cell
                .extra_28
                .get(0x27)
                .and_then(|entry| entry.resolved_0c.flatten());
            if owner == second_form(item) { 1.0 } else { 0.0 }
        }
        // 004F6400 GetDetected: the target's process vt+0x3AC is pure on Low (0060D080); the player's 00655FE0 with a null +0x184 computes nothing. 005F6540 with no subject process answers 0; Low vt+0x3B0 0; a target with no vt+0x154 node gives -1, no detection.
        0x004F_6400 => {
            let (Some(actor), Some(target)) =
                (actor, parameter.filter(|&form| is_actor(world, form)))
            else {
                return Ok(0.0);
            };
            let target_process =
                process_of(world, target).ok_or(StreamError("004F64AA: a null process faults"))?;
            if Some(target) == player {
                let data = player_data(world).expect("the player");
                if data.combat_opponents_5ac != 0 {
                    return Err(StreamError("006605A0: the player's combat opponents"));
                }
                if world
                    .get::<MiddleHighProcess>(target_process)
                    .is_some_and(|middle| middle.skin_property_184 != 0)
                {
                    return Err(StreamError("00655FE0: with a +0x184 object"));
                }
            } else {
                require_low(
                    world,
                    target_process,
                    "004F64AA: another process's vt+0x3AC",
                )?;
            }
            let Some(process) = process_of(world, actor) else {
                return Ok(0.0);
            };
            require_low(world, process, "005F6540: another process's vt+0x3B0")?;
            if world
                .get::<TesObjectRefr>(target)
                .expect("an actor")
                .loaded_3d_3c
                != 0
            {
                return Err(StreamError("005F65A2: the detection of a target with 3D"));
            }
            if Some(target) == player
                && player_data(world)
                    .expect("the player")
                    .first_person_node_5d0
                    .is_some()
            {
                return Err(StreamError("006600F0: the player's first-person node"));
            }
            0.0
        }
        // 004F5730 IsSpellTarget: 006A1DF0, MagicTarget vt+8: the player's 0066B0B0, +0x1E4; a Character's 005E5220, process vt+0x298, Low 006F7070: 0.
        0x004F_5730 => {
            let (Some(actor), Some(_)) = (actor, parameter) else {
                return Ok(0.0);
            };
            if let Some(player) = world.get::<PlayerCharacter>(actor) {
                if player.active_effects_1e4.is_some_and(|list| list != [0; 2]) {
                    return Err(StreamError("006A1E04: the player's active effects"));
                }
                return Ok(0.0);
            }
            let Some(process) = process_of(world, actor) else {
                return Ok(0.0);
            };
            require_low(world, process, "005E5220: another process's vt+0x298")?;
            0.0
        }
        // 004F9120 GetLineOfSight: with no vt+0x154 node, 0, before any store.
        0x004F_9120 => {
            let target = parameter.filter(|&form| world.get::<TesObjectRefr>(form).is_some());
            let (Some(_), Some(target)) = (actor, target) else {
                return Ok(0.0);
            };
            if world
                .get::<TesObjectRefr>(target)
                .expect("a reference")
                .loaded_3d_3c
                != 0
            {
                return Err(StreamError(
                    "005F2820: the line of sight to a target with 3D",
                ));
            }
            if Some(target) == player
                && player_data(world)
                    .expect("the player")
                    .first_person_node_5d0
                    .is_some()
            {
                return Err(StreamError("006600F0: the player's first-person node"));
            }
            0.0
        }
        // 004F46E0 GetCurrentTime.
        0x004F_46E0 => f64::from(game_hour(world)),
        // 004F6CF0 GetInSameCell.
        0x004F_6CF0 => {
            let other = parameter.filter(|&form| is_reference_type(world, form));
            let mine = reference.and_then(|form| parent_cell(world, form));
            let theirs = other.and_then(|form| parent_cell(world, form));
            if mine.is_some() && mine == theirs {
                1.0
            } else {
                0.0
            }
        }
        // 004F60E0 GetDisabled: the 0x800 flag or 004FA560's 00B361CC list, which nothing fills before Phase 21's script runs. A null subject never arrives: row 023 requires one.
        0x004F_60E0 => {
            let subject = subject.ok_or(StreamError("004F60E0: without a subject"))?;
            let queued = world
                .get_resource::<super::script_vm::ScriptVmGlobals>()
                .is_some_and(|vm| vm.disabled_b361cc.contains(&subject));
            f64::from(u8::from(form_flags(world, subject) & 0x800 != 0 || queued))
        }
        // 004F4890 GetDead: vt+0x198(1).
        0x004F_4890 => match actor {
            Some(actor) if actor_is_dead(world, actor, true) => 1.0,
            _ => 0.0,
        },
        // 004F4BC0 GetStageDone(quest, stage): 00529B30, the group's +0x01.
        0x004F_4BC0 => {
            let Some(parameter) = parameter else {
                return Ok(0.0);
            };
            let quest = world
                .get::<TesQuest>(parameter)
                .ok_or(StreamError("004F4BC0: GetStageDone on a non-quest"))?;
            match quest
                .stages_40
                .iter()
                .find(|group| group.index_00 == raw_10 as u8)
            {
                Some(group) if group.flags_01 != 0 => 1.0,
                _ => 0.0,
            }
        }
        // 004F6D70 GetInCell: _strnicmp over the parameter's length, with the cache stored as 00B3618C, 00B36194, 00B36190.
        0x004F_6D70 => {
            let cell = parameter.filter(|&form| form_type(world, form) == 0x30);
            let second = second_form(item).filter(|&form| is_reference_type(world, form));
            let Some(who) = second.or(subject) else {
                return Ok(0.0);
            };
            let globals = world.resource::<AiProcessGlobals>();
            if Some(who) == globals.caches.in_cell_subject && cell == globals.caches.in_cell_cell {
                return Ok(f64::from(globals.caches.in_cell_result));
            }
            let mut result = 0.0;
            if let (Some(current), Some(cell)) = (parent_cell(world, who), cell) {
                let name = editor_id_of(world, cell);
                if crt::strnicmp(&editor_id_of(world, current), &name, name.len()) == 0 {
                    result = 1.0;
                }
            }
            let caches = &mut world.resource_mut::<AiProcessGlobals>().caches;
            caches.in_cell_subject = Some(who);
            caches.in_cell_result = result as f32;
            caches.in_cell_cell = cell;
            result
        }
        // 004F7330 GetGlobalValue: -1.0 unless a global.
        0x004F_7330 => parameter
            .filter(|&form| form_type(world, form) == 4)
            .and_then(|form| global_value(world, form))
            .map_or(-1.0, f64::from),
        // 004F4F50 GetLevel: 004677F0, movzx: the C++'s int16 is wrong above 0x7FFF.
        0x004F_4F50 => {
            let Some(actor) = actor else {
                return Ok(0.0);
            };
            let base = world
                .get::<TesObjectRefr>(actor)
                .expect("an actor")
                .base_form_1c
                .and_then(|base| world.get::<TesActorBaseData>(base))
                .ok_or(StreamError("004F4F50: GetLevel without a base"))?;
            f64::from(effective_level(world, base)? as u16)
        }
        // 004F7220 GetIsReference.
        0x004F_7220 => {
            let other = parameter.filter(|&form| is_reference_type(world, form));
            if subject.is_some() && subject == other {
                1.0
            } else {
                0.0
            }
        }
        // 004F7C00 GetCurrentAIProcedure: the code of 00B152B0[+0x18][+0x04] through the table at 004F7E30.
        0x004F_7C00 => {
            const CODES: [i8; 0x2D] = [
                0, 7, 1, 2, 8, 10, 11, 12, 14, 13, 16, 18, 4, 3, 19, 15, 5, 20, 9, -1, 6, -1, 21,
                22, 23, 24, 25, 26, -1, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, -1,
                17,
            ];
            let Some(process) = actor.and_then(|actor| process_of(world, actor)) else {
                return Ok(0.0);
            };
            stop_on_process_package_override(world, process)?;
            let base = *world.get::<BaseProcess>(process).expect("a process");
            let Some(package) = base.editor_package_08 else {
                return Ok(0.0);
            };
            let code = procedure_at(world, package, base.edit_pack_procedure_type_04)?;
            CODES
                .get(code as usize)
                .map_or(-1.0, |&code| f64::from(code))
        }
        // 004F7FA0 GetIsCurrentPackage: the process's package, or with flag 0x800 a nonnull 0x1F extra's.
        0x004F_7FA0 => {
            let package = parameter.filter(|&form| form_type(world, form) == 61);
            let (Some(actor), Some(package)) = (actor, package) else {
                return Ok(0.0);
            };
            let mut current = process_of(world, actor)
                .and_then(|process| world.get::<BaseProcess>(process))
                .and_then(|base| base.editor_package_08);
            if current
                .and_then(|current| world.get::<TesPackage>(current))
                .is_some_and(|data| data.flags_1c & 0x800 != 0)
                && let Some(entry) = world
                    .get::<TesObjectRefr>(actor)
                    .expect("an actor")
                    .extra_data_44
                    .get(0x1F)
                && let Some(form) = entry.resolved_0c.flatten()
            {
                current = Some(form);
            }
            if current == Some(package) { 1.0 } else { 0.0 }
        }
        // 004F5570 GetDayOfWeek.
        0x004F_5570 => f64::from(game_day_of_week(world)),
        // 004F55C0 IsPlayerInJail: 0065DA50, +0x608 above 0.
        0x004F_55C0 => f64::from(u8::from(
            player_data(world).is_some_and(|data| data.prison_days_remaining_608 > 0),
        )),
        // 004F80E0 GetPCExpelled: bit 3 of +0x34.
        0x004F_80E0 => f64::from(u8::from(
            parameter
                .and_then(|form| world.get::<TesFaction>(form))
                .is_some_and(|faction| faction.flags_34 & 8 != 0),
        )),
        // 004F9010 IsInInterior.
        0x004F_9010 => {
            let cell = reference.and_then(|form| parent_cell(world, form));
            f64::from(u8::from(
                cell.is_some_and(|cell| cell_is_interior(world, cell)),
            ))
        }
        // 004F6E50 GetInWorldspace: the +0x10 reference or the subject.
        0x004F_6E50 => {
            let space = parameter.filter(|&form| form_type(world, form) == 0x35);
            let second = second_form(item).filter(|&form| is_reference_type(world, form));
            let who = second
                .or(subject)
                .filter(|&form| world.get::<TesObjectRefr>(form).is_some());
            let (Some(space), Some(who)) = (space, who) else {
                return Ok(0.0);
            };
            if reference_world(world, who) == Some(space) {
                1.0
            } else {
                0.0
            }
        }
        // 004F4570 MenuMode: 0 asks 00578F60; an ID asks 0057C140, which only a menu can answer.
        0x004F_4570 => {
            if raw_0c != 0 {
                return Err(StreamError("004F458A: MenuMode with a menu ID"));
            }
            f64::from(u8::from(arms.menu_mode(world)?))
        }
        _ => {
            return Err(StreamError(
                "0056ACD2: a condition handler with no arm: see the table's row",
            ));
        }
    })
}

/// IDA: 0056A3B0 the compare (op, result, value): 0 ==, 1 !=, 2 >, 3 >=, 4 <, 5 <= of the result against the value, and past 5 true. Both are floats. A NaN passes ops 1 to 5, by the `test ah` patterns: 0056A3E8 `test ah,1; jne`, 0056A3FC `test ah,41h; jne`, 0056A410 `test ah,41h; jp`, 0056A424 `test ah,5; jnp false`. The C++'s plain compares fail them. Completion: 100%.
#[must_use]
#[expect(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "0056A3B0's NaN arms: a NaN must pass ops 2 to 5"
)]
pub fn compare_condition(op: u32, result: f32, value: f32) -> bool {
    match op {
        0 => result == value,
        1 => result != value,
        2 => !(result <= value),
        3 => !(result < value),
        4 => !(result >= value),
        5 => !(result > value),
        _ => true,
    }
}

// IDA: 0056B1D0: whether parameter 0 of a function is a reference kind (4 or 6).
fn parameter_is_reference(index: u16) -> bool {
    CONDITION_FUNCTIONS
        .get(usize::from(index))
        .and_then(|row| row.parameter_kinds_14.first())
        .is_some_and(|&kind| kind == 4 || kind == 6)
}

// IDA: 0056AC50 TESConditionItem_Evaluate(subject, target, &out): bit 1 swaps; a null +0x0C takes the target when 0056B1D0 says so; a null handler or a missing required subject is false with no compare; the handler; the value is the global's +0x24 with bit 2 (no class test, a null one 0.0) or the +0x04 float; the 0056A3B0 compare; index 0x4C with op 2 or 3 and false sets the out byte. Completion: 100%.
fn evaluate_item(
    world: &mut World,
    item: &TesConditionItem,
    subject: Option<Entity>,
    target: Option<Entity>,
    out: &mut bool,
    arms: &mut dyn AiArms,
) -> Result<bool, StreamError> {
    *out = false;
    let flags = item.data_00[0];
    let (first, second) = if flags & 2 != 0 {
        (target, subject)
    } else {
        (subject, target)
    };
    let index = item.function_08();
    let null_parameter = match item.parameters_resolved[0] {
        Some(form) => form.is_none(),
        None => item.parameter(0) == 0,
    };
    let parameter = if null_parameter {
        if parameter_is_reference(index) {
            second
        } else {
            None
        }
    } else {
        item.parameters_resolved[0].flatten()
    };
    let row = CONDITION_FUNCTIONS
        .get(usize::from(index))
        .ok_or(StreamError("0056AC9A: past the 00B0C8C0 table"))?;
    if row.handler_20 == 0 || (row.requires_subject_10 != 0 && first.is_none()) {
        return Ok(false);
    }
    let result = run_handler(world, index, first, parameter, item, arms)?;
    let raw_value = u32::from_le_bytes(item.data_00[4..8].try_into().expect("four bytes"));
    let value = if flags & 4 != 0 {
        match item.comparison_resolved {
            Some(Some(global)) => global_value(world, global)
                .ok_or(StreamError("0056ACE5: a condition global of another class"))?,
            Some(None) => 0.0,
            None if raw_value == 0 => 0.0,
            None => return Err(StreamError("0056ACE2: an unresolved condition global")),
        }
    } else {
        f32::from_bits(raw_value)
    };
    // 0056AD0A the double rounds to a float.
    let result = result as f32;
    let op = u32::from(flags >> 5);
    if compare_condition(op, result, value) {
        return Ok(true);
    }
    if index == 0x4C && (op == 2 || op == 3) {
        *out = true;
    }
    Ok(false)
}

// IDA: 004D78E0: an XMarker or XMarkerHeading reference's 0x4D extra's form.
fn marker_target(world: &World, form: Entity) -> Result<Option<Entity>, StreamError> {
    let Some(reference) = world.get::<TesObjectRefr>(form) else {
        return Ok(None);
    };
    let statics = world.resource::<DefaultFormStatics>();
    if reference.base_form_1c.is_none()
        || (reference.base_form_1c != statics.x_marker_b35eac
            && reference.base_form_1c != statics.x_marker_heading_b35eb0)
    {
        return Ok(None);
    }
    let Some(entry) = reference.extra_data_44.get(0x4D) else {
        return Ok(None);
    };
    match (entry.resolved_0c, &entry.data) {
        (Some(found), _) => Ok(found),
        (None, ExtraData::Dword(0)) => Ok(None),
        _ => Err(StreamError("00420D20: an unresolved XTarget")),
    }
}

/// IDA: N/A the main thread's CRT state as the host holds it: `_holdrand` and the draws made so far, for a test's count
/// of a window's draws (the script commands, the Big Four menus).
pub struct CrtProbe(pub std::rc::Rc<dyn Fn() -> (u32, u64)>);

/// IDA: N/A the probe's reading, when the host installed one.
#[must_use]
pub fn crt_probe(world: &World) -> Option<(u32, u64)> {
    world.get_non_send::<CrtProbe>().map(|probe| (probe.0)())
}

/// IDA: N/A how a script reaches 0047DF80: the CRT's draw, which the host installs. The second is the same draw for a seam with no World, such as 00488420's rolls under a script command, which reads the host's copy of 00B069C3.
#[derive(Clone)]
pub struct ScriptRandom(
    pub std::rc::Rc<dyn Fn(&mut World) -> i32>,
    pub std::rc::Rc<dyn Fn() -> i32>,
);

// IDA: N/A a script's arms: the host's draw, and 00578F60.
pub(super) struct ScriptArms;

impl AiArms for ScriptArms {
    /// IDA: N/A `ScriptArms`'s side of `AiArms`.
    fn random_next(&mut self, world: &mut World) -> Result<i32, StreamError> {
        let draw = world
            .get_non_send::<ScriptRandom>()
            .cloned()
            .ok_or(StreamError("0047DF80: a script's draw with no host"))?;
        Ok((draw.0)(world))
    }
    /// IDA: N/A `ScriptArms`'s side of `AiArms`.
    fn menu_mode(&mut self, world: &mut World) -> Result<bool, StreamError> {
        Ok(super::script_vm::script_is_menu_mode(world))
    }
}

/// IDA: a script function sharing a condition's evaluator: `index` is the row in both tables, and `second` a word parameter. `GetRandomPercent` reaches `ScriptRandom`. Completion: as `run_handler`.
/// # Errors
/// As `run_handler`.
pub fn run_script_handler(
    world: &mut World,
    index: u16,
    subject: Option<Entity>,
    parameter: Option<Entity>,
    second: u32,
) -> Result<f64, StreamError> {
    let mut item = TesConditionItem {
        data_00: [0; 0x18],
        comparison_resolved: None,
        parameters_resolved: [None, None],
    };
    item.data_00[8..10].copy_from_slice(&index.to_le_bytes());
    item.data_00[0x10..0x14].copy_from_slice(&second.to_le_bytes());
    run_handler(world, index, subject, parameter, &item, &mut ScriptArms)
}

/// IDA: 0056A950 TESCondition_EvaluateSubject(subject, 0), from a script.
/// # Errors
/// As `evaluate_conditions`.
pub fn evaluate_conditions_for_script(
    world: &mut World,
    condition: &TesCondition,
    subject: Entity,
) -> Result<bool, StreamError> {
    evaluate_conditions(world, condition, Some(subject), None, &mut ScriptArms)
}

/// IDA: 0056A510 TESCondition_Evaluate(subject, target, &result, chain) with a null chain, which is all 0056A950 passes (0005308BD's nonzero chain is dialogue's, Phase 21): the AND walk with OR runs. Returns the pass byte. Completion: 100% of the null chain.
pub fn evaluate_conditions(
    world: &mut World,
    condition: &TesCondition,
    subject: Option<Entity>,
    target: Option<Entity>,
    arms: &mut dyn AiArms,
) -> Result<bool, StreamError> {
    let mut pass = true;
    let mut in_or = false;
    let mut or_out = false;
    let mut or_fail = false;
    let mut any_fail = false;
    let mut last = false;
    let mut result_out = false;
    let mut target = target;
    if let Some(form) = target
        && let Some(marked) = marker_target(world, form)?
    {
        target = Some(marked);
    }
    for item in &condition.items {
        let mut out = false;
        let mut check = false;
        if in_or {
            // 0056A684.
            if !last {
                last = evaluate_item(world, item, subject, target, &mut out, arms)?;
                if or_out || out {
                    or_out = true;
                    if !out && !last {
                        or_fail = true;
                    }
                } else if !last {
                    or_fail = true;
                }
            }
            if item.data_00[0] & 1 == 0 {
                in_or = false;
                if !(pass && last) {
                    pass = false;
                    if or_out && !or_fail {
                        result_out = true;
                    } else {
                        any_fail = true;
                        check = true;
                    }
                }
            }
        } else {
            last = evaluate_item(world, item, subject, target, &mut out, arms)?;
            if item.data_00[0] & 1 != 0 {
                // 0056A651.
                in_or = true;
                or_fail = false;
                or_out = out;
                if !out && !last {
                    or_fail = true;
                }
            } else {
                pass = pass && last;
                if out {
                    result_out = true;
                } else if !last {
                    any_fail = true;
                }
            }
        }
        // 0056A5BC / 0056A5CA.
        if (check || (!pass && !in_or)) && !result_out {
            return Ok(pass);
        }
    }
    // 0056A607. The result byte isn't returned: 0056A950 discards it, and `any_fail` only clears it.
    let _ = any_fail;
    if !in_or {
        return Ok(pass);
    }
    Ok(pass && last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tes::ai_process::AiRecordedArms;

    // GetCurrentTime (0x12) with no time globals is 12.0: `value` decides each item. Flag bit 0 is OR with the next.
    fn item(value: f32, or: bool) -> TesConditionItem {
        let mut data = [0_u8; 0x18];
        data[0] = u8::from(or);
        data[4..8].copy_from_slice(&value.to_le_bytes());
        data[8] = 0x12;
        TesConditionItem {
            data_00: data,
            comparison_resolved: None,
            parameters_resolved: [None; 2],
        }
    }

    fn walk(items: Vec<TesConditionItem>) -> (bool, u32) {
        let mut world = World::new();
        world.init_resource::<AiProcessGlobals>();
        let result = evaluate_conditions(
            &mut world,
            &TesCondition { items },
            None,
            None,
            &mut AiRecordedArms::default(),
        )
        .unwrap();
        let calls = world.resource::<AiProcessGlobals>().condition_calls[0x12];
        (result, calls)
    }

    // 0056A510: an AND stops at the first failure; an OR run evaluates only until one passes, and fails only if none does.
    #[test]
    fn the_walk_is_and_over_or_runs() {
        assert_eq!(walk(vec![]), (true, 0));
        assert_eq!(walk(vec![item(12.0, false)]), (true, 1));
        assert_eq!(walk(vec![item(1.0, false), item(12.0, false)]), (false, 1));
        assert_eq!(walk(vec![item(12.0, true), item(1.0, false)]), (true, 1));
        assert_eq!(walk(vec![item(1.0, true), item(12.0, false)]), (true, 2));
        assert_eq!(walk(vec![item(1.0, true), item(2.0, false)]), (false, 2));
        assert_eq!(
            walk(vec![item(1.0, true), item(2.0, false), item(12.0, false)]),
            (false, 2)
        );
        // A failing OR at the very end of the list.
        assert_eq!(walk(vec![item(12.0, false), item(1.0, true)]), (false, 2));
    }

    // 00B0C8C0's rows the handlers name exist, so the `match` can't be dead on an address typo.
    #[test]
    fn the_condition_table_has_the_rows_the_arms_name() {
        for handler in [
            0x004F_4B70,
            0x004F_4EF0,
            0x004F_6D70,
            0x004F_7C00,
            0x004F_6E50,
            0x004F_4570,
        ] {
            assert!(
                CONDITION_FUNCTIONS
                    .iter()
                    .any(|row| row.handler_20 == handler),
                "{handler:X}"
            );
        }
    }

    #[test]
    fn a_nan_passes_all_but_equal() {
        assert!(!compare_condition(0, f32::NAN, 1.0));
        for op in 1..=5 {
            assert!(compare_condition(op, f32::NAN, 1.0), "{op}");
        }
        assert!(compare_condition(6, 0.0, 1.0));
        assert!(compare_condition(2, 2.0, 1.0));
        assert!(!compare_condition(2, 1.0, 1.0));
        assert!(compare_condition(3, 1.0, 1.0));
        assert!(compare_condition(4, 0.0, 1.0));
        assert!(!compare_condition(4, 1.0, 1.0));
        assert!(compare_condition(5, 1.0, 1.0));
    }
}
