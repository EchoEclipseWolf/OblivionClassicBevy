//! The script interpreter, for what startup reaches: 004FBE00, 00517330, 00517640, 00516830, 005169A0's statements, the expression evaluator 004F3620 with 004F3320 and 004FAB20, 004FAE80's parameters, 004FA9C0, the event list functions, 21 block predicates and 4 functions. A census of the original from boot through the first idle frame reached no generic command; the gameplay loop's commands since go through `script_commands::run_command`, where an unported opcode, like an unreached block or function, is a stop, by return.
//!
//! Both callers of 005169A0 pass `a10 = 0`, so the compile-time arms (`a10 != 0`, `a8 != 0` in 004F3620 and 004FAB20) aren't here. The runner's singleton 00B36208 isn't kept: with no command, nothing nests.
//!
//! x87: the main thread's control word is 0x7F, 24 bits, so each float result of 004F3620 rounds to 24 bits. `f64` then `f32` could round twice where the exe rounds once, a recorded limit.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::logging::{Level, emit};
use oblivion_core::x87;

use crate::tes::actor::TesObjectRefr;
use crate::tes::ai_conditions::{bound_object, item_count};
use crate::tes::base_extra_list::{ExtraData, action_flag};
use crate::tes::form::FormIdentity;
use crate::tes::form_hierarchy::RttiClass;
use crate::tes::forms_dialogue::TesQuest;
use crate::tes::forms_init::{QuestDelayGlobals, cast, casts_to, form_type, lookup};
use crate::tes::forms_script::{
    Script, ScriptEvent, ScriptEventList, ScriptGlobals, create_script_event_list,
};
use crate::tes::forms_world::TesObjectCell;
use crate::tes::global::TESGlobal;
use crate::tes::interface_manager::{InterfaceManager, is_menu_mode};
use crate::tes::inventory::{ensure_container_changes, put_container_changes, reference_container};
use crate::tes::player::PlayerCharacter;
use crate::tes::player::the_player;
use crate::tes::tes::TesGlobals;
use crate::tes::tile::TileEngine;

// IDA: the opcodes 005169A0 and 00516830 name.
const OP_BEGIN: u16 = 16;
const OP_SET: u16 = 21;
const OP_IF: u16 = 22;
const OP_ELSE: u16 = 23;
const OP_ELSEIF: u16 = 24;
const OP_ENDIF: u16 = 25;
const OP_REFERENCE: u16 = 28;
const OP_RETURN: u16 = 30;

/// IDA: N/A the host's counts, set against the census of the original.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ScriptVmCounts {
    /// 00517330's entries.
    pub statement_runs: u32,
    /// 00517640's entries.
    pub block_only_runs: u32,
    /// 00516AF3, by the table's row.
    pub block_tests: [u32; 31],
    /// 004FADD5.
    pub function_calls: u32,
    /// Each 005172C6: the opcode, the reference and script IDs, as the capture logs them.
    pub commands: Vec<(u32, u32, u32)>,
    /// IDA: N/A the main thread's CRT (`_holdrand`, draws) as each command began, with a host `CrtProbe`.
    pub command_crt: Vec<(u32, u64)>,
}

/// IDA: 00B09E24 and 00B361C8 (005072B0's), and 004FA9C0's cache: 00B361B0 is `ScriptGlobals`, 00B09E1C the index, 00B361B4 the list, 00B361B8 the answer.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct ScriptVmGlobals {
    pub button_b09e24: i8,
    pub scripts_enabled_b09e25: u8,
    pub button_reference_b361c8: u32,
    pub cached_index_b09e1c: u32,
    pub cached_owner_b361b4: Option<Entity>,
    /// IDA: N/A the references prepended to TES +0x88 by 00441670, which 004402F0 drains.
    pub queued_references_88: Vec<Entity>,
    /// IDA: 00B33E9C, as `run_grid_scripts_frame` was given it.
    pub frame_seconds_b33e9c: f32,
    /// 00B361CC, which Disable fills.
    pub disabled_b361cc: Vec<Entity>,
}

impl Default for ScriptVmGlobals {
    /// IDA: N/A the image's values, read at 00B09E24 and 00B361C8.
    fn default() -> Self {
        Self {
            button_b09e24: -1,
            scripts_enabled_b09e25: 1,
            button_reference_b361c8: u32::MAX,
            cached_index_b09e1c: 0,
            cached_owner_b361b4: None,
            queued_references_88: Vec::new(),
            frame_seconds_b33e9c: 0.0,
            disabled_b361cc: Vec::new(),
        }
    }
}

// IDA: 004F9FA0 Script_GetScriptsEnabled. Completion: the image byte and its current value.
pub(crate) fn scripts_enabled(world: &World) -> bool {
    world
        .get_resource::<ScriptVmGlobals>()
        .is_none_or(|state| state.scripts_enabled_b09e25 != 0)
}

// IDA: the runner 00517810 hands 00517330, of which +0x00 the container and +0x04 the reference's base have no reader among the reached arms: +0x08 the events, +0x20 the depth, +0x24 the flags by depth, +0x18 and +0x1C a failed expression's kind and code. `owner` is N/A: whose list `events` is, since the host takes it out of its extra or quest for the run.
pub(in crate::tes) struct Runner<'a> {
    pub(in crate::tes) events_08: &'a mut ScriptEventList,
    owner: Option<Entity>,
    depth_20: usize,
    flags_24: [u32; 32],
    /// +0xA1, which the run answers.
    pub(in crate::tes) moved_a1: bool,
}

// IDA: N/A a list a variable is read from or stored to.
#[derive(Clone, Copy)]
enum EventTarget {
    Run,
    Reference(Entity),
    Quest(Entity),
}

// IDA: 004F3620's value: the tag at +0x00 of a stack slot and the eight bytes at +0x08, kept as bits, since an int writes only the low dword of its local.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Value {
    is_float: bool,
    bits: u64,
}

impl Value {
    // IDA: N/A the slot's low dword, as a signed int.
    fn int(self) -> i32 {
        self.bits as u32 as i32
    }

    // IDA: N/A the slot as a double.
    fn float(self) -> f64 {
        f64::from_bits(self.bits)
    }

    // IDA: N/A `fild` of an int slot, `fld qword` of a float's.
    fn number(self) -> f64 {
        if self.is_float {
            self.float()
        } else {
            f64::from(self.int())
        }
    }
}

// IDA: N/A a little-endian u16 of the compiled data.
fn u16_at(data: &[u8], at: usize) -> Result<u16, GameError> {
    data.get(at..at + 2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .ok_or(GameError::unclassified(
            0x00516830,
            "00516830: a read past the compiled data",
        ))
}

// IDA: N/A a little-endian u32 of the compiled data.
fn u32_at(data: &[u8], at: usize) -> Result<u32, GameError> {
    data.get(at..at + 4)
        .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .ok_or(GameError::unclassified(
            0x00516830,
            "00516830: a read past the compiled data",
        ))
}

/// IDA: N/A a seam for a test window that stands before 0040EB18's manager, where the real run has the LoadingMenu up, so 00578F60 answers true.
#[derive(Resource, Clone, Copy, Debug)]
pub struct ScriptMenuModeAnswer(pub bool);

// IDA: N/A how a script reaches 0057C140 (`interface_manager::is_menu_input_active`): through the World's TileEngine. With
// none (a script run inside the interface), a manager with a cursor and a root, past which 0057C140 reads the root's
// user0, is an error; no manager, cursor or root answers false first, as 0057C140 does.
fn is_menu_input_active(world: &World, id: u32) -> Result<bool, GameError> {
    let Some(engine) = world.get_resource::<TileEngine>() else {
        let reads_root = world
            .get_resource::<InterfaceManager>()
            .is_some_and(|manager| manager.cursor_01c.is_some() && manager.menu_root_068.is_some());
        if reads_root {
            return Err(GameError::unclassified(
                0x0057C140,
                "0057C140: a script run inside the interface",
            ));
        }
        return Ok(false);
    };
    Ok(crate::tes::interface_manager::is_menu_input_active(
        world, engine, id,
    ))
}

// IDA: N/A the SCHR header's compiled size, +0x20.
fn compiled_size(world: &World, script: Entity) -> u32 {
    let body = world.get::<Script>(script).expect("a script");
    u32::from_le_bytes(body.header_18[8..12].try_into().expect("four bytes"))
}

// IDA: 00517372..00517374 and friends `a3[8] = 0`: a failed script stops running.
fn zero_compiled_size(world: &mut World, script: Entity) {
    world.get_mut::<Script>(script).expect("a script").header_18[8..12].fill(0);
}

// IDA: N/A a form's +0x08 bit 3, whether a script's load has finished.
fn is_initialized(world: &World, script: Entity) -> bool {
    world.get::<FormIdentity>(script).expect("a form").flags_08 & 8 != 0
}

// IDA: 00484F50: the 0x12 extra's event list, taken out for a run.
fn take_reference_events(world: &mut World, reference: Entity) -> Option<ScriptEventList> {
    let mut refr = world.get_mut::<TesObjectRefr>(reference)?;
    match &mut refr.extra_data_44.get_mut(0x12)?.data {
        ExtraData::Script { events_10, .. } => events_10.take(),
        _ => None,
    }
}

// IDA: N/A its return.
fn put_reference_events(world: &mut World, reference: Entity, events: Option<ScriptEventList>) {
    if let Some(mut refr) = world.get_mut::<TesObjectRefr>(reference)
        && let Some(entry) = refr.extra_data_44.get_mut(0x12)
        && let ExtraData::Script { events_10, .. } = &mut entry.data
    {
        *events_10 = events;
    }
}

/// IDA: 0052ADF0's 004FBE00(item +0x0C, player, the player's list, 0, 1), having zeroed +0x28. Its `cl` skips the size test. The list is taken out of the 0x12 extra for the run, and none makes a local one.
/// # Errors
/// A stop below.
pub fn script_run_forced(
    world: &mut World,
    script: Entity,
    reference: Entity,
) -> Result<(), GameError> {
    world.get_mut::<Script>(script).expect("a script").header_18[0x10] = 0;
    if compiled_size(world, script) == 0 {
        return Ok(());
    }
    let mut events = take_reference_events(world, reference);
    let result = run_statements(
        world,
        script,
        Some(reference),
        events.as_mut(),
        Some(reference),
    );
    put_reference_events(world, reference, events);
    clear_variable_references(world, script);
    result.map(|_| ())
}

/// IDA: 004FBE00 Script_Run(script, reference, events, 0, 0): a compiled size at or below 4 answers 0. The quest arm, `+0x28` set: while `+0x38` is above 0 it counts down by 00B33E9C, `+0x3C` counts up, and it answers 0 while `+0x38` stays above 0. After the run, with 00B09E28 above 0, `+0x38` gains `fQuestDelayTime` from the events if that's above 0, else 00B09E28, and `+0x3C` clears. Then 004FB430. `owner` is N/A: whose list `events` is. Completion: 100%.
/// # Errors
/// A stop below.
pub fn script_run(
    world: &mut World,
    script: Entity,
    reference: Option<Entity>,
    mut events: Option<&mut ScriptEventList>,
    owner: Option<Entity>,
) -> Result<bool, GameError> {
    // 004FBE47: answers 0.
    if compiled_size(world, script) <= 4 {
        return Ok(false);
    }
    let quest = world.get::<Script>(script).expect("a script").header_18[0x10] != 0;
    if quest {
        let frame = world
            .get_resource_or_init::<ScriptVmGlobals>()
            .frame_seconds_b33e9c;
        let mut body = world.get_mut::<Script>(script).expect("a script");
        if 0.0 < body.floats_34[1] {
            body.floats_34[1] -= frame;
            body.floats_34[2] += frame;
            if 0.0 < body.floats_34[1] {
                return Ok(false);
            }
        }
    }
    let answer = run_statements(world, script, reference, events.as_deref_mut(), owner)?;
    if quest {
        let delay = world
            .get_resource::<QuestDelayGlobals>()
            .map_or(f32::from_bits(0x40A0_0000), |globals| {
                globals.quest_script_delay_time_b09e28
            });
        if 0.0 < delay {
            let body = world.get::<Script>(script).expect("a script");
            // 004FAA90: _stricmp over +0x48's names, the first match's ID.
            let variable = body
                .variables_48
                .iter()
                .find(|variable| {
                    variable
                        .name_18
                        .data_00
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(b"fQuestDelayTime"))
                })
                .map(|variable| {
                    u32::from_le_bytes(variable.data_00[..4].try_into().expect("four bytes"))
                });
            let own = variable.and_then(|id| {
                let list = events.as_deref()?;
                let found = list.variables_0c.iter().find(|entry| entry.id_00 == id)?;
                Some(f64::from_bits(found.value_08) as f32)
            });
            let mut body = world.get_mut::<Script>(script).expect("a script");
            body.floats_34[1] += match own {
                Some(value) if value > 0.0 => value,
                _ => delay,
            };
            body.floats_34[2] = 0.0;
        }
    }
    clear_variable_references(world, script);
    Ok(answer)
}

// IDA: 004FB430 Script_ClearVariableReferences: +0x08 of each reference with a nonzero +0x0C. Completion: 100%.
fn clear_variable_references(world: &mut World, script: Entity) {
    let mut body = world.get_mut::<Script>(script).expect("a script");
    for reference in &mut body.references_40 {
        if reference.variable_0c != 0 {
            reference.form_08 = 0;
            reference.resolved_08 = None;
        }
    }
}

// IDA: N/A the compiled bytes for a run.
fn script_data(world: &World, script: Entity) -> Result<Vec<u8>, GameError> {
    world
        .get::<Script>(script)
        .expect("a script")
        .data_30
        .clone()
        .ok_or(GameError::unclassified(
            0x00516830,
            "00516830: compiled size with no data",
        ))
}

// IDA: 00517330 ScriptRunner_Run, with the 00517810 around it: an uninitialized script answers 0. Null events make a local list, freed after. `+0x29` stores the block's arguments, unreached. Each instruction is 00516830, then 005169A0, and opcode 30 ends the run; a failed 00516830 zeroes the compiled size. Unless a dispatch failed, 004FA0E0 clears each event's mask. The answer is +0xA1, which only a generic command sets. Completion: 100% of the reached arms.
fn run_statements(
    world: &mut World,
    script: Entity,
    reference: Option<Entity>,
    events: Option<&mut ScriptEventList>,
    owner: Option<Entity>,
) -> Result<bool, GameError> {
    world
        .get_resource_or_init::<ScriptVmCounts>()
        .statement_runs += 1;
    if !is_initialized(world, script) {
        return Ok(false);
    }
    let mut local;
    let events_08 = match events {
        Some(events) => events,
        None => {
            local = create_script_event_list(world, script);
            &mut local
        }
    };
    if world.get::<Script>(script).expect("a script").header_18[0x11] != 0 {
        return Err(GameError::unclassified(
            0x00517474,
            "00517474: a script with block arguments",
        ));
    }
    let mut runner = Runner {
        events_08,
        owner,
        depth_20: 0,
        flags_24: [0; 32],
        moved_a1: false,
    };
    let data = script_data(world, script)?;
    let mut position = 0usize;
    let mut skip = 0u32;
    let mut current: Option<Entity> = None;
    let mut line = 0u32;
    let mut failed = false;
    while compiled_size(world, script) != 0 {
        line += 1;
        let Some((op, operands, length)) = next_instruction(
            world,
            &mut runner,
            script,
            &data,
            &mut position,
            &mut current,
            skip != 0,
        )?
        else {
            zero_compiled_size(world, script);
            break;
        };
        let mut length = length;
        if skip != 0 {
            skip -= 1;
        } else {
            if op == OP_RETURN {
                break;
            }
            if current.is_none() {
                current = reference;
            }
            if !dispatch(
                world,
                &mut runner,
                script,
                &data,
                op,
                current,
                &mut length,
                operands,
                &mut skip,
                line,
                false,
            )? {
                failed = true;
                break;
            }
        }
        position = operands + length as usize;
        if position >= compiled_size(world, script) as usize {
            break;
        }
    }
    if !failed {
        // 004FA0E0.
        for event in runner.events_08.events_08.iter_mut().flatten() {
            event.mask_04 = 0;
        }
    }
    // 005175F0.
    Ok(runner.moved_a1)
}

/// IDA: 00517950, with 00517640 in it, from 004F9EC0: an uninitialized script answers 0, and the loop is 00517330's with `a11 = 1`, so only Begin blocks dispatch and each body is skipped. So the predicates' own effects are all it has. Completion: 100%.
/// # Errors
/// A stop below.
pub fn run_blocks_only(
    world: &mut World,
    script: Entity,
    reference: Option<Entity>,
    events: &mut ScriptEventList,
    owner: Option<Entity>,
) -> Result<(), GameError> {
    world
        .get_resource_or_init::<ScriptVmCounts>()
        .block_only_runs += 1;
    if !is_initialized(world, script) {
        return Ok(());
    }
    let mut runner = Runner {
        events_08: events,
        owner,
        depth_20: 0,
        flags_24: [0; 32],
        moved_a1: false,
    };
    let data = script_data(world, script)?;
    let mut position = 0usize;
    let mut skip = 0u32;
    let mut current: Option<Entity> = None;
    let mut line = 0u32;
    while compiled_size(world, script) != 0 {
        line += 1;
        let Some((op, operands, length)) = next_instruction(
            world,
            &mut runner,
            script,
            &data,
            &mut position,
            &mut current,
            skip != 0,
        )?
        else {
            zero_compiled_size(world, script);
            return Ok(());
        };
        let mut length = length;
        if skip != 0 {
            skip -= 1;
        } else {
            if current.is_none() {
                current = reference;
            }
            if !dispatch(
                world,
                &mut runner,
                script,
                &data,
                op,
                current,
                &mut length,
                operands,
                &mut skip,
                line,
                true,
            )? {
                return Ok(());
            }
        }
        position = operands + length as usize;
        if position >= compiled_size(world, script) as usize {
            return Ok(());
        }
    }
    Ok(())
}

// IDA: 00516830: the opcode, and for 28 a reference index through 004FA9C0 unless skipping, cast to TESObjectREFR, then the opcode; then the length. Answers the opcode, the operands' offset and the length; `None` for a missing reference, where the exe warns through 005167A0. `current` keeps its value when 28 finds nothing, as the exe's does. Completion: 100%.
fn next_instruction(
    world: &mut World,
    runner: &mut Runner,
    script: Entity,
    data: &[u8],
    position: &mut usize,
    current: &mut Option<Entity>,
    skipping: bool,
) -> Result<Option<(u16, usize, u32)>, GameError> {
    let mut op = u16_at(data, *position)?;
    *position += 2;
    if op != OP_REFERENCE {
        *current = None;
    } else {
        let index = u16_at(data, *position)?;
        *position += 2;
        let mut variable = false;
        if !skipping && let Some(at) = resolve_reference(world, runner, script, u32::from(index)) {
            let entry = world.get::<Script>(script).expect("a script").references_40[at].clone();
            variable = entry.variable_0c != 0;
            if let Some(form) = entry.resolved_08.flatten() {
                *current = cast(world, Some(form), RttiClass::TESObjectREFR);
            }
        }
        op = u16_at(data, *position)?;
        *position += 2;
        if !skipping && current.is_none() && !variable {
            emit(
                Level::Warning,
                "Unknown reference object for a function. Execution of the script stopped.",
            );
            return Ok(None);
        }
    }
    let length = u32::from(u16_at(data, *position)?);
    *position += 2;
    Ok(Some((op, *position, length)))
}

// IDA: 004FA9C0: an index of 0 or past +0x1C is null. A hit on 00B361B0, 00B09E1C and 00B361B4 answers 00B361B8. Else the index-th of +0x40; a nonzero +0x0C with events reads that variable, and a nonzero low dword goes through 0046B250 into +0x08. Answers the index in `references_40`. Completion: 100%.
fn resolve_reference(
    world: &mut World,
    runner: &Runner,
    script: Entity,
    index: u32,
) -> Option<usize> {
    let body = world.get::<Script>(script).expect("a script");
    let count = u32::from_le_bytes(body.header_18[4..8].try_into().expect("four bytes"));
    if index == 0 || index > count {
        return None;
    }
    let at = index as usize - 1;
    let variable = body.references_40.get(at)?.variable_0c;
    let hit = world
        .get_resource::<ScriptGlobals>()
        .is_some_and(|globals| globals.current_b361b0 == Some(script))
        && {
            let vm = world.get_resource_or_init::<ScriptVmGlobals>();
            vm.cached_index_b09e1c == index && vm.cached_owner_b361b4 == runner.owner
        };
    if hit {
        return Some(at);
    }
    if variable != 0 {
        let value = read_variable(world, runner, EventTarget::Run, variable);
        let id = value.to_bits() as u32;
        if id != 0 {
            let form = lookup(world, id);
            let mut body = world.get_mut::<Script>(script).expect("a script");
            body.references_40[at].form_08 = id;
            body.references_40[at].resolved_08 = Some(form);
        }
    }
    world.get_resource_or_init::<ScriptGlobals>().current_b361b0 = Some(script);
    let mut vm = world.get_resource_or_init::<ScriptVmGlobals>();
    vm.cached_index_b09e1c = index;
    vm.cached_owner_b361b4 = runner.owner;
    Some(at)
}

// IDA: N/A `0041F7D0`'s list of a reference, or a quest's +0x58.
fn target_list(world: &World, target: EventTarget) -> Option<&ScriptEventList> {
    match target {
        EventTarget::Run => None,
        EventTarget::Reference(reference) => {
            match &world
                .get::<TesObjectRefr>(reference)?
                .extra_data_44
                .get(0x12)?
                .data
            {
                ExtraData::Script { events_10, .. } => events_10.as_ref(),
                _ => None,
            }
        }
        EventTarget::Quest(quest) => world.get::<TesQuest>(quest)?.events_58.as_ref(),
    }
}

// IDA: N/A whether a target is the list the run has taken out.
fn is_run_list(runner: &Runner, target: EventTarget) -> bool {
    match target {
        EventTarget::Run => true,
        EventTarget::Reference(form) | EventTarget::Quest(form) => runner.owner == Some(form),
    }
}

// IDA: 004FA110 ScriptEventList_GetVariable: the entry's double, or a warning and 0.0. Its cache 00B361BC, 00B361C0, 00B361C4 only avoids the walk, so it isn't kept. Completion: 100% of the value.
fn read_variable(world: &World, runner: &Runner, target: EventTarget, id: u32) -> f64 {
    let list = if is_run_list(runner, target) {
        Some(&*runner.events_08)
    } else {
        target_list(world, target)
    };
    let found = list.and_then(|list| {
        list.variables_0c
            .iter()
            .find(|variable| variable.id_00 == id)
    });
    match found {
        Some(variable) => f64::from_bits(variable.value_08),
        None => {
            emit(Level::Warning, &format!("Variable ID {id:08X} not found."));
            0.0
        }
    }
}

// IDA: 004FB630: the entry's double at +0x08, or a warning. Completion: 100% of the store.
fn write_variable(
    world: &mut World,
    runner: &mut Runner,
    target: EventTarget,
    id: u32,
    value: f64,
) {
    let list: Option<&mut ScriptEventList> = if is_run_list(runner, target) {
        Some(&mut *runner.events_08)
    } else {
        match target {
            EventTarget::Run => None,
            EventTarget::Reference(reference) => world
                .get_mut::<TesObjectRefr>(reference)
                .and_then(|refr| refr.into_inner().extra_data_44.get_mut(0x12))
                .and_then(|entry| match &mut entry.data {
                    ExtraData::Script { events_10, .. } => events_10.as_mut(),
                    _ => None,
                }),
            EventTarget::Quest(quest) => world
                .get_mut::<TesQuest>(quest)
                .and_then(|quest| quest.into_inner().events_58.as_mut()),
        }
    };
    let variable = list.and_then(|list| {
        list.variables_0c
            .iter_mut()
            .find(|variable| variable.id_00 == id)
    });
    match variable {
        Some(variable) => variable.value_08 = value.to_bits(),
        None => emit(
            Level::Warning,
            &format!("Trying to set variableID {id}: variable not found."),
        ),
    }
}

// IDA: 005169A0 with `a10 = 0`. Answers the exe's `al`. With `a11`, everything but a Begin returns at once. Completion: 100% of its statement arms; a generic command is a stop.
#[allow(clippy::too_many_arguments)]
fn dispatch(
    world: &mut World,
    runner: &mut Runner,
    script: Entity,
    data: &[u8],
    op: u16,
    current: Option<Entity>,
    length: &mut u32,
    operands: usize,
    skip: &mut u32,
    line: u32,
    a11: bool,
) -> Result<bool, GameError> {
    if op == OP_BEGIN {
        return begin_block(world, runner, script, data, current, length, operands, a11);
    }
    if a11 {
        return Ok(true);
    }
    match op {
        17 | 28 | 29 => Ok(true),
        OP_SET => set_statement(world, runner, script, data, current, operands, line),
        OP_IF => {
            if runner.flags_24[runner.depth_20] != 0 {
                runner.depth_20 += 1;
            }
            runner.flags_24[runner.depth_20] = 1;
            if_statement(world, runner, script, data, current, operands, skip, line)
        }
        OP_ELSEIF => if_statement(world, runner, script, data, current, operands, skip, line),
        OP_ELSE => {
            let count = u32::from(u16_at(data, operands)?);
            if runner.flags_24[runner.depth_20] & 2 == 0 {
                runner.flags_24[runner.depth_20] |= 2;
            } else {
                *skip += count;
            }
            Ok(true)
        }
        OP_ENDIF => {
            runner.flags_24[runner.depth_20] = 0;
            if runner.depth_20 != 0 {
                runner.depth_20 -= 1;
            }
            Ok(true)
        }
        _ => {
            super::script_commands::run_command(world, runner, script, data, op, current, operands)
        }
    }
}

// IDA: 005169A0's opcode 16: the type (past 0x1E returns), the block's length, a required reference with a warning, the row's handler, then 0.0 or `a11` adds the length, skipping the body. A failed handler warns and answers 0. Completion: 100%.
#[allow(clippy::too_many_arguments)]
fn begin_block(
    world: &mut World,
    runner: &mut Runner,
    script: Entity,
    data: &[u8],
    current: Option<Entity>,
    length: &mut u32,
    operands: usize,
    a11: bool,
) -> Result<bool, GameError> {
    let kind = u16_at(data, operands)?;
    if kind > 0x1E {
        return Ok(true);
    }
    let body = u32_at(data, operands + 2)?;
    let mut cursor = operands + 6;
    // 00B0AF58's byte: all but GameMode, MenuMode.
    if kind > 1 && current.is_none() {
        emit(Level::Warning, "Null for a required ref pointer.");
        return Ok(false);
    }
    world.get_resource_or_init::<ScriptVmCounts>().block_tests[usize::from(kind)] += 1;
    let Some(answer) = block_predicate(world, runner, script, data, &mut cursor, current, kind)?
    else {
        emit(Level::Warning, "Error Executing line.");
        return Ok(false);
    };
    if answer == 0.0 || a11 {
        *length = length.wrapping_add(body);
    }
    Ok(true)
}

// IDA: 00B0AF58's parameter types (+0x04 of each row, 12 bytes each), for the rows reached: 1 Integer, 4 ObjectReferenceID, 6 Actor, 21 ObjectID, 26 Container, 28 Crime Type, 29 Package, 31 Magic Effect.
fn block_parameters(kind: u16) -> Result<&'static [u32], GameError> {
    Ok(match kind {
        0 | 21 | 30 => &[],
        1 => &[1],
        2 | 8 | 10 | 25 => &[6],
        3..=6 => &[26],
        9 => &[21],
        13 => &[28, 6],
        15 | 16 | 20 => &[29],
        22 => &[31],
        24 | 26 | 27 => &[4],
        28 | 29 => &[21],
        _ => {
            return Err(GameError::unclassified(
                0x00516AF3,
                "00516AF3: a block type no startup run reaches",
            ));
        }
    })
}

// IDA: the row's handler. Each parses through 004FAE80, a failure answering `None`, zeroes the answer, and with a reference and events tests one mask through 004FB5F0, on the last parameter. 0 005022D0, 1 00502270 and 2 00503F90 differ. Completion: 100% of the reached rows.
fn block_predicate(
    world: &mut World,
    runner: &mut Runner,
    script: Entity,
    data: &[u8],
    cursor: &mut usize,
    current: Option<Entity>,
    kind: u16,
) -> Result<Option<f64>, GameError> {
    let types = block_parameters(kind)?;
    let Some(values) = parse_parameters(world, runner, script, data, cursor, types)? else {
        return Ok(None);
    };
    let last = values.last().copied().unwrap_or(0);
    let first = values.first().copied().unwrap_or(0);
    let mask: u32 = match kind {
        // 005022D0 GameMode.
        0 => return Ok(Some(f64::from(u8::from(!is_menu_mode(world))))),
        // 00502270, 004F4570 MenuMode: 0 asks 00578F60. An ID asks 0057C140.
        1 => {
            let answer = if first != 0 {
                is_menu_input_active(world, first)?
            } else {
                is_menu_mode(world)
            };
            return Ok(Some(f64::from(u8::from(answer))));
        }
        // 00503F90: 1.0 with action flag 2, then 004D82E0(ref, 1) and (ref, 2).
        2 => {
            let mut answer = 0.0;
            if let Some(reference) = current {
                let mut refr = world
                    .get_mut::<TesObjectRefr>(reference)
                    .expect("a reference");
                let extras = &mut refr.extra_data_44;
                if action_flag(extras, 2) {
                    answer = 1.0;
                }
                extras.clear_action_flags(1);
                extras.clear_action_flags(2);
            }
            return Ok(Some(answer));
        }
        3 => 1,
        4 | 28 => 2,
        5 | 29 => 8,
        6 => 4,
        8 => 0x80,
        9 => 0x100,
        10 => 0x10,
        // 00504341's jump table on the crime type: 0x10000, then 2..=6.
        13 => match first {
            2 => 0x2_0000,
            3 => 0x4_0000,
            4 => 0x8_0000,
            5 => 0x10_0000,
            6 => 0x20_0000,
            _ => 0x1_0000,
        },
        15 => 0x200,
        16 => 0x400,
        20 => 0x800,
        21 => 0x1000,
        22 => 0x2000,
        24 => 0x1000_0000,
        25 => 0x8000,
        26 => 0x2000_0000,
        27 => 0x4000_0000,
        30 => 0x8000_0000,
        _ => {
            return Err(GameError::unclassified(
                0x00516AF3,
                "00516AF3: a block type no startup run reaches",
            ));
        }
    };
    if current.is_none() {
        return Ok(Some(0.0));
    }
    Ok(Some(f64::from(u8::from(test_event(
        runner.events_08,
        last,
        mask,
    )))))
}

// IDA: 004FB5F0: the first entry with the ID answers its mask against `mask`; a miss goes to 004FB510, which makes the list if it's null and prepends {ID, 0}, and answers false. Completion: 100%.
fn test_event(events: &mut ScriptEventList, id: u32, mask: u32) -> bool {
    let list = events.events_08.get_or_insert_with(Vec::new);
    if let Some(event) = list.iter().find(|event| event.id_00 == id) {
        return event.mask_04 & mask != 0;
    }
    list.insert(
        0,
        ScriptEvent {
            id_00: id,
            mask_04: 0,
        },
    );
    false
}

// IDA: 004FAE80 in run mode: the count, a word, then each parameter by the row's type, as a form's ID or an int. No count of parameters answers an empty list; `None` is the exe's 0. A form parameter is `r`, an index through 004FA9C0, and a type test; a null form or a wrong type fails. Completion: 100% of the types `block_parameters` and 00502BE0 name.
pub(in crate::tes) fn parse_parameters(
    world: &mut World,
    runner: &mut Runner,
    script: Entity,
    data: &[u8],
    cursor: &mut usize,
    types: &[u32],
) -> Result<Option<Vec<u32>>, GameError> {
    if types.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let count = u16_at(data, *cursor)? as i16;
    *cursor += 2;
    let mut values = Vec::new();
    for at in 0..count.max(0) as usize {
        let Some(&kind) = types.get(at) else {
            return Err(GameError::unclassified(
                0x004FAE80,
                "004FAE80: more parameters than the row names",
            ));
        };
        match kind {
            // 004FB22B: 004FAB20, then __ftol2_sse. 23 shares the arm.
            1 | 23 => {
                let Some(value) = read_value(world, runner, script, data, cursor, None)? else {
                    return Ok(None);
                };
                values.push(x87::ftol64_low_f64(value));
            }
            // 2: 004FAB20, stored as a float, whose bits are kept here.
            2 => {
                let Some(value) = read_value(world, runner, script, data, cursor, None)? else {
                    return Ok(None);
                };
                values.push(x87::stored_float(value).to_bits());
            }
            // A word.
            5 | 28 => {
                values.push(i32::from(u16_at(data, *cursor)? as i16) as u32);
                *cursor += 2;
            }
            3 | 4 | 6 | 7 | 9 | 11 | 13 | 14 | 17 | 21 | 25 | 26 | 29 | 31 => {
                let tag = *data.get(*cursor).ok_or(GameError::unclassified(
                    0x004FAE80,
                    "004FAE80: a read past the compiled data",
                ))?;
                *cursor += 1;
                if tag != b'r' {
                    return Ok(None);
                }
                let index = u16_at(data, *cursor)?;
                *cursor += 2;
                let Some(entry) = resolve_reference(world, runner, script, u32::from(index)) else {
                    return Ok(None);
                };
                let form = world.get::<Script>(script).expect("a script").references_40[entry]
                    .resolved_08
                    .flatten();
                let Some(form) = form else {
                    return Ok(None);
                };
                let form_kind = form_type(world, form);
                let fits = match kind {
                    // 00469520's types, and vt+0xA8.
                    3 => bound_object(form_kind),
                    4 | 26 => (49..=51).contains(&form_kind),
                    6 => (50..=51).contains(&form_kind),
                    // A spell. The case's second arm, for type 21, isn't reached.
                    7 => casts_to(world, form, RttiClass::SpellItem),
                    9 => form_kind == 48,
                    11 => casts_to(world, form, RttiClass::MagicItem),
                    13 => form_kind == 57,
                    14 => form_kind == 59,
                    17 => form_kind == 6,
                    25 => (35..=36).contains(&form_kind),
                    21 => casts_to(world, form, RttiClass::TESBoundObject),
                    29 => form_kind == 61,
                    _ => casts_to(world, form, RttiClass::EffectSetting),
                };
                if !fits {
                    return Ok(None);
                }
                values.push(world.get::<FormIdentity>(form).expect("a form").form_id_0c);
            }
            _ => {
                return Err(GameError::unclassified(
                    0x004FAF50,
                    "004FAF50: a parameter type no startup run reaches",
                ));
            }
        }
    }
    Ok(Some(values))
}

// IDA: 005169A0's opcode 21: `G` a global, `r` another list's variable, else the run's. An `r` or `G` with no reference or no list returns. The expression's error warns and zeroes the compiled size. `f` stores the double, `l` and `s` its __ftol. A store to a reference outside a cell of level 6 is queued through 00441670. Completion: 100%.
fn set_statement(
    world: &mut World,
    runner: &mut Runner,
    script: Entity,
    data: &[u8],
    current: Option<Entity>,
    operands: usize,
    line: u32,
) -> Result<bool, GameError> {
    let mut at = operands;
    let mut kind = data[at];
    at += 1;
    let mut target = EventTarget::Run;
    let mut global = None;
    let mut queued_form = None;
    if kind == b'G' || kind == b'r' {
        let index = u16_at(data, at)?;
        at += 2;
        if kind == b'r' {
            kind = data[at];
            at += 1;
        }
        let Some(entry) = resolve_reference(world, runner, script, u32::from(index)) else {
            return Ok(true);
        };
        let form = world.get::<Script>(script).expect("a script").references_40[entry]
            .resolved_08
            .flatten();
        let Some(form) = form else {
            return Ok(true);
        };
        queued_form = Some(form);
        if kind == b'G' {
            global = Some(form);
        } else if casts_to(world, form, RttiClass::TESObjectREFR) {
            if target_list(world, EventTarget::Reference(form)).is_none()
                && runner.owner != Some(form)
            {
                return Ok(true);
            }
            target = EventTarget::Reference(form);
        } else if world.get::<TesQuest>(form).is_some() {
            if target_list(world, EventTarget::Quest(form)).is_none() && runner.owner != Some(form)
            {
                return Ok(true);
            }
            target = EventTarget::Quest(form);
        }
    }
    let mut variable = 0;
    if kind != b'G' {
        variable = u32::from(u16_at(data, at)? as i16 as u16);
        at += 2;
    }
    let size = u16_at(data, at)? as i16;
    at += 2;
    let (value, error) = evaluate(
        world,
        runner,
        script,
        data,
        at,
        size.max(0) as usize,
        current,
    )?;
    if error != 0 {
        emit(
            Level::Warning,
            &format!("Script line {line}: Set expression returned an error: {error}."),
        );
        zero_compiled_size(world, script);
        return Ok(true);
    }
    if kind == b'G' {
        if let Some(mut global) = global.and_then(|form| world.get_mut::<TESGlobal>(form)) {
            global.value_bits_24 = (value as f32).to_bits();
        }
        return Ok(true);
    }
    match kind {
        b'f' => write_variable(world, runner, target, variable, value),
        b'l' | b's' => {
            let stored = f64::from(x87::ftol64_low_f64(value) as i32);
            write_variable(world, runner, target, variable, stored);
        }
        _ => {}
    }
    let queued = queued_form.or(current);
    if let Some(reference) =
        queued.and_then(|form| cast(world, Some(form), RttiClass::TESObjectREFR))
    {
        let flags = world
            .get::<FormIdentity>(reference)
            .expect("a form")
            .flags_08;
        let cell = world
            .get::<TesObjectRefr>(reference)
            .and_then(|refr| refr.parent_cell_40);
        let level = cell
            .and_then(|cell| world.get::<TesObjectCell>(cell))
            .map(|cell| cell.process_level_26);
        if flags & 0x4000 == 0 && level != Some(6) {
            // 00441670: once each, at the head.
            let mut vm = world.get_resource_or_init::<ScriptVmGlobals>();
            if !vm.queued_references_88.contains(&reference) {
                vm.queued_references_88.insert(0, reference);
            }
            if let Some(tes) = world.resource_mut::<TesGlobals>().tes.as_mut() {
                tes.queued_script_references_88 = 1;
            }
        }
    }
    Ok(true)
}

// IDA: 005169A0's opcodes 22 and 24: the skip count, the expression's size, and with a taken branch or a 0.0 the skip grows, else the branch is taken. An error warns and zeroes the compiled size. Completion: 100%.
#[allow(clippy::too_many_arguments)]
fn if_statement(
    world: &mut World,
    runner: &mut Runner,
    script: Entity,
    data: &[u8],
    current: Option<Entity>,
    operands: usize,
    skip: &mut u32,
    line: u32,
) -> Result<bool, GameError> {
    let count = u32::from(u16_at(data, operands)?);
    let size = u16_at(data, operands + 2)? as i16;
    let mut error = 0;
    if runner.flags_24[runner.depth_20] & 2 != 0 {
        *skip += count;
    } else {
        let (value, code) = evaluate(
            world,
            runner,
            script,
            data,
            operands + 4,
            size.max(0) as usize,
            current,
        )?;
        error = code;
        if value == 0.0 {
            *skip += count;
        } else {
            runner.flags_24[runner.depth_20] |= 2;
        }
    }
    if error != 0 {
        emit(
            Level::Warning,
            &format!("Script line {line}: failed to evaluate expression."),
        );
        zero_compiled_size(world, script);
    }
    Ok(true)
}

// IDA: 00B0A12D, the 16 operators.
const OPERATORS: [&[u8]; 16] = [
    b"(", b")", b"&&", b"||", b"<=", b"<", b">=", b">", b"==", b"!=", b"-", b"+", b"*", b"/", b"%",
    b"~",
];

// IDA: 004F3320: one token into `out`, answering its operator or 16. Bytes below 33, as signed, are skipped. `X` with an opcode in 4096..4465 copies its header and length; `s l f G Z` 3 bytes, ending the token; `r` 3, going on; `n` 5; `z` 9; a quote to the closing one; space or tab ends it. Else an operator at the start is the token, later it ends the token, and a partial match with none sets error 5. Completion: 100%.
fn next_token(input: &[u8], position: &mut usize, out: &mut Vec<u8>, error: &mut i32) -> u32 {
    out.clear();
    let peek = |at: usize| input.get(at).copied().unwrap_or(0);
    let mut at = *position;
    let mut first = true;
    while peek(at) != 0 && (peek(at) as i8) < 33 {
        at += 1;
    }
    let mut operator = 16;
    loop {
        let byte = peek(at);
        if byte == 0 {
            break;
        }
        let word = u16::from_le_bytes([peek(at + 1), peek(at + 2)]);
        let copy = |out: &mut Vec<u8>, count: usize, at: &mut usize| {
            for offset in 0..count {
                out.push(peek(*at + offset));
            }
            *at += count;
        };
        match byte {
            b'X' if (4096..4465).contains(&word) => {
                let size = usize::from(u16::from_le_bytes([peek(at + 3), peek(at + 4)])) + 5;
                copy(out, size, &mut at);
                break;
            }
            b's' | b'l' | b'f' | b'G' | b'Z' => {
                copy(out, 3, &mut at);
                break;
            }
            b'r' => {
                copy(out, 3, &mut at);
                first = false;
                continue;
            }
            b'n' => {
                copy(out, 5, &mut at);
                break;
            }
            b'z' => {
                copy(out, 9, &mut at);
                break;
            }
            b'"' => {
                out.push(b'"');
                at += 1;
                while peek(at) != 0 && peek(at) != b'"' {
                    out.push(peek(at));
                    at += 1;
                }
                if peek(at) == b'"' {
                    out.push(b'"');
                    at += 1;
                }
                break;
            }
            b' ' | b'\t' => break,
            _ => {}
        }
        let mut partial = false;
        let mut found = None;
        for (index, text) in OPERATORS.iter().enumerate() {
            let mut matched = 0;
            while matched < text.len() && peek(at + matched) == text[matched] {
                matched += 1;
                partial = true;
            }
            if matched == text.len() {
                found = Some(index as u32);
                break;
            }
        }
        if let Some(index) = found {
            if first {
                operator = index;
                let text = OPERATORS[index as usize];
                out.extend_from_slice(text);
                at += text.len();
            }
            break;
        }
        if partial {
            *error = 5;
        }
        out.push(byte);
        at += 1;
        first = false;
    }
    *position = at;
    operator
}

// IDA: 0047D550: optional `-`, then digits, not a lone `-`. Completion: 100%.
fn is_integer_text(text: &[u8]) -> bool {
    let Some((&head, rest)) = text.split_first() else {
        return false;
    };
    (head.is_ascii_digit() || head == b'-')
        && !(rest.is_empty() && head == b'-')
        && rest.iter().all(u8::is_ascii_digit)
}

// IDA: 0047D5B0: digits with at most one `.`, an optional leading `-`, not a lone `-` or `.`. Completion: 100%.
fn is_float_text(text: &[u8]) -> bool {
    let Some((&head, rest)) = text.split_first() else {
        return false;
    };
    if !(head.is_ascii_digit() || head == b'-' || head == b'.') {
        return false;
    }
    if rest.is_empty() && (head == b'-' || head == b'.') {
        return false;
    }
    let mut dot = head == b'.';
    for &byte in rest {
        if byte == b'.' {
            if dot {
                return false;
            }
            dot = true;
        } else if !byte.is_ascii_digit() {
            return false;
        }
    }
    true
}

// IDA: N/A the x87's 24-bit rounding of a float result.
fn round24(value: f64) -> f64 {
    f64::from(value as f32)
}

// IDA: 004F3620 Expression_Eval in run mode: RPN over 004F3320's tokens. Operands are text numbers (atol, atof) or 004FAB20; operators 2..=15 pop. 3 is an underflow, 4 a zero divisor, 5 a bad end. Comparisons and && || give an int, arithmetic an int only when both are. Answers the value as a double and the error. `reg` and `tag` are the exe's locals, which keep their bytes between tokens. Completion: 100% except a float `%`, a stop.
fn evaluate(
    world: &mut World,
    runner: &mut Runner,
    script: Entity,
    data: &[u8],
    start: usize,
    size: usize,
    current: Option<Entity>,
) -> Result<(f64, i32), GameError> {
    if size == 0 {
        return Ok((0.0, 0));
    }
    let input = data
        .get(start..start + size)
        .ok_or(GameError::unclassified(
            0x004F3620,
            "004F3620: an expression past the compiled data",
        ))?;
    let mut error = 0;
    let mut stack: Vec<Value> = Vec::new();
    let mut reg = Value::default();
    let mut token = Vec::new();
    let mut position = 0;
    loop {
        let operator = next_token(input, &mut position, &mut token, &mut error);
        if token.is_empty() {
            break;
        }
        if !(2..=15).contains(&operator) {
            if is_integer_text(&token) {
                reg.is_float = false;
                let number: i64 = std::str::from_utf8(&token)
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0);
                reg.bits = (reg.bits & 0xFFFF_FFFF_0000_0000) | u64::from(number as i32 as u32);
            } else {
                reg.is_float = true;
                if is_float_text(&token) {
                    let number: f64 = std::str::from_utf8(&token)
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0.0);
                    reg.bits = number.to_bits();
                } else {
                    let mut cursor = 0;
                    match read_operand(
                        world,
                        runner,
                        script,
                        &token,
                        &mut cursor,
                        current,
                        &mut reg,
                    )? {
                        true => {}
                        false => return Ok((0.0, 5)),
                    }
                }
            }
            stack.push(reg);
            continue;
        }
        let Some(right) = stack.pop() else {
            return Ok((0.0, 3));
        };
        if operator == 15 {
            if right.is_float {
                reg.bits = (-right.float()).to_bits();
            } else {
                reg.bits = (reg.bits & 0xFFFF_FFFF_0000_0000)
                    | u64::from(right.int().wrapping_neg() as u32);
            }
            reg.is_float = right.is_float;
            stack.push(reg);
            continue;
        }
        let Some(left) = stack.pop() else {
            return Ok((0.0, 3));
        };
        let any_float = left.is_float || right.is_float;
        let as_int = |reg: &mut Value, value: i32| {
            reg.is_float = false;
            reg.bits = (reg.bits & 0xFFFF_FFFF_0000_0000) | u64::from(value as u32);
        };
        let (a, b) = (left.number(), right.number());
        match operator {
            2 => as_int(&mut reg, i32::from(a != 0.0 && b != 0.0)),
            3 => as_int(&mut reg, i32::from(a != 0.0 || b != 0.0)),
            4 => as_int(&mut reg, i32::from(a <= b)),
            5 => as_int(&mut reg, i32::from(a < b)),
            6 => as_int(&mut reg, i32::from(a >= b)),
            7 => as_int(&mut reg, i32::from(a > b)),
            8 => as_int(&mut reg, i32::from(a == b)),
            9 => as_int(&mut reg, i32::from(a != b)),
            10..=12 => {
                if any_float {
                    let value = match operator {
                        10 => a - b,
                        11 => a + b,
                        _ => a * b,
                    };
                    reg.is_float = true;
                    reg.bits = round24(value).to_bits();
                } else {
                    let (x, y) = (left.int(), right.int());
                    let value = match operator {
                        10 => x.wrapping_sub(y),
                        11 => x.wrapping_add(y),
                        _ => x.wrapping_mul(y),
                    };
                    as_int(&mut reg, value);
                }
            }
            _ => {
                if b == 0.0 {
                    return Ok((0.0, 4));
                }
                if any_float {
                    if operator == 14 {
                        return Err(GameError::unclassified(
                            0x004F3620,
                            "004F3620: a float % in a script",
                        ));
                    }
                    reg.is_float = true;
                    reg.bits = round24(a / b).to_bits();
                } else if operator == 13 {
                    as_int(&mut reg, left.int().wrapping_div(right.int()));
                } else {
                    as_int(&mut reg, left.int().wrapping_rem(right.int()));
                }
            }
        }
        stack.push(reg);
    }
    let Some(top) = stack.pop() else {
        return Ok((0.0, 5));
    };
    if !stack.is_empty() {
        return Ok((0.0, 5));
    }
    Ok((top.number(), error))
}

// IDA: N/A 004FAB20 into the evaluator's local, where `Z` writes only the low dword.
fn read_operand(
    world: &mut World,
    runner: &mut Runner,
    script: Entity,
    token: &[u8],
    cursor: &mut usize,
    current: Option<Entity>,
    reg: &mut Value,
) -> Result<bool, GameError> {
    if token.first() == Some(&b'Z') {
        let index = u16_at(token, 1)?;
        let Some(entry) = resolve_reference(world, runner, script, u32::from(index)) else {
            return Ok(false);
        };
        let form = world.get::<Script>(script).expect("a script").references_40[entry]
            .resolved_08
            .flatten();
        match form {
            Some(form) => {
                let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
                reg.bits = (reg.bits & 0xFFFF_FFFF_0000_0000) | u64::from(id);
            }
            None => {
                let variable =
                    world.get::<Script>(script).expect("a script").references_40[entry].variable_0c;
                if variable == 0 {
                    return Ok(false);
                }
                reg.bits = 0f64.to_bits();
            }
        }
        return Ok(true);
    }
    match read_value(world, runner, script, token, cursor, current)? {
        Some(value) => {
            reg.bits = value.to_bits();
            Ok(true)
        }
        None => Ok(false),
    }
}

// IDA: 004FAB20 in run mode: `G` a global's float, `r` then a function or another list's variable, `n` `fild`, `z` a double, `X` a function by 004FCA30's row, `s l f` the run's variable. `Z` is `read_operand`'s. `None` is the exe's 0. A reference with no form but a variable reads 0.0. Completion: 100% of the reached arms; 00B0xxxx's 0x22 stack arm is a stop.
fn read_value(
    world: &mut World,
    runner: &mut Runner,
    script: Entity,
    data: &[u8],
    cursor: &mut usize,
    current: Option<Entity>,
) -> Result<Option<f64>, GameError> {
    let mut kind = *data.get(*cursor).ok_or(GameError::unclassified(
        0x004FAB20,
        "004FAB20: a read past the compiled data",
    ))?;
    *cursor += 1;
    let mut target = EventTarget::Run;
    let mut reference = current;
    match kind {
        b'G' | b'r' => {
            let index = u16_at(data, *cursor)?;
            *cursor += 2;
            let Some(entry) = resolve_reference(world, runner, script, u32::from(index)) else {
                return Ok(None);
            };
            let row = world.get::<Script>(script).expect("a script").references_40[entry].clone();
            let form = row.resolved_08.flatten();
            let Some(form) = form else {
                return Ok(if row.variable_0c != 0 {
                    Some(0.0)
                } else {
                    None
                });
            };
            if kind == b'G' {
                let bits = world
                    .get::<TESGlobal>(form)
                    .map_or(0, |global| global.value_bits_24);
                return Ok(Some(f64::from(f32::from_bits(bits))));
            }
            kind = *data.get(*cursor).ok_or(GameError::unclassified(
                0x004FAB20,
                "004FAB20: a read past the compiled data",
            ))?;
            *cursor += 1;
            if kind == b'X' {
                reference = cast(world, Some(form), RttiClass::TESObjectREFR);
            } else {
                if world.get::<TesQuest>(form).is_some() {
                    target = EventTarget::Quest(form);
                } else if casts_to(world, form, RttiClass::TESObjectREFR) {
                    let stacked = world
                        .get::<TesObjectRefr>(form)
                        .is_some_and(|refr| refr.extra_data_44.get(0x22).is_some());
                    if stacked {
                        return Err(GameError::unclassified(
                            0x004FAB20,
                            "004FAB20: a variable of an inventory reference",
                        ));
                    }
                    target = EventTarget::Reference(form);
                }
                if !is_run_list(runner, target) && target_list(world, target).is_none() {
                    return Ok(None);
                }
            }
        }
        b'n' => {
            let value = u32_at(data, *cursor)? as i32;
            *cursor += 4;
            return Ok(Some(f64::from(value)));
        }
        b'z' => {
            let low = u32_at(data, *cursor)?;
            let high = u32_at(data, *cursor + 4)?;
            *cursor += 8;
            return Ok(Some(f64::from_bits(
                u64::from(low) | (u64::from(high) << 32),
            )));
        }
        _ => {}
    }
    if kind == b'X' {
        let opcode = u32::from(u16_at(data, *cursor)?);
        *cursor += 4;
        world
            .get_resource_or_init::<ScriptVmCounts>()
            .function_calls += 1;
        return call_function(world, runner, script, data, cursor, opcode, reference);
    }
    let variable = u16_at(data, *cursor)? as i16 as i32 as u32;
    *cursor += 2;
    if !matches!(kind, b'f' | b'l' | b's') {
        return Ok(None);
    }
    Ok(Some(read_variable(world, runner, target, variable)))
}

// IDA: 004FADD5's targets, by 004FCA30's row: 4127 005072B0 GetButtonPressed, 4143 00502890 GetItemCount, 4154 00502BE0 GetStage, 4271 00505210 IsPCSleeping. The census, with about 37 idle iterations, reached no other. Completion: 100% of the four.
fn call_function(
    world: &mut World,
    runner: &mut Runner,
    script: Entity,
    data: &[u8],
    cursor: &mut usize,
    opcode: u32,
    reference: Option<Entity>,
) -> Result<Option<f64>, GameError> {
    match opcode {
        4127 => {
            // 005072B0: a null reference, or one with +0x08 bit 14, asks the script's own ID.
            let flagged = reference.filter(|&reference| {
                world
                    .get::<FormIdentity>(reference)
                    .expect("a form")
                    .flags_08
                    & 0x4000
                    == 0
            });
            let who = flagged.unwrap_or(script);
            let id = world.get::<FormIdentity>(who).expect("a form").form_id_0c;
            let mut vm = world.get_resource_or_init::<ScriptVmGlobals>();
            if vm.button_b09e24 > -1 && id == vm.button_reference_b361c8 {
                let answer = f64::from(vm.button_b09e24);
                vm.button_b09e24 = -1;
                vm.button_reference_b361c8 = 0;
                return Ok(Some(answer));
            }
            Ok(Some(-1.0))
        }
        4154 => {
            // 00502BE0: Quest, then 004F4B70, whose 00529750 reads +0x5C.
            let Some(values) = parse_parameters(world, runner, script, data, cursor, &[14])? else {
                return Ok(None);
            };
            let stage = values
                .first()
                .and_then(|&id| lookup(world, id))
                .and_then(|quest| {
                    world
                        .get::<TesQuest>(quest)
                        .map(|quest| f64::from(quest.byte_5c))
                });
            Ok(Some(stage.unwrap_or(0.0)))
        }
        4143 => {
            // +0x10: needs a reference. 00502890: ObjectID, then 004F48F0 GetItemCount: a bound parameter, the subject's container and its created changes, |004869C0|.
            let Some(reference) = reference else {
                return Ok(None);
            };
            let Some(values) = parse_parameters(world, runner, script, data, cursor, &[3])? else {
                return Ok(None);
            };
            let Some(item) = values.first().and_then(|&id| lookup(world, id)) else {
                return Ok(Some(0.0));
            };
            if !bound_object(form_type(world, item))
                || reference_container(world, reference).is_none()
            {
                return Ok(Some(0.0));
            }
            let changes = ensure_container_changes(world, reference)?;
            put_container_changes(world, reference, changes);
            Ok(Some(f64::from(
                item_count(world, reference, item).wrapping_abs(),
            )))
        }
        4271 => {
            // 00505210, 004F8050: player +0x594.
            let set = the_player(world)
                .and_then(|player| world.get::<PlayerCharacter>(player))
                .is_some_and(|player| player.pc_sleep_hours_set_594 != 0);
            Ok(Some(f64::from(u8::from(set))))
        }
        _ => super::script_commands::run_function(
            world, runner, script, data, cursor, opcode, reference,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tes::base_extra_list::ExtraDataList;

    // 004F3320's split of an RPN string: the operator comes alone, and an operand stops at it.
    #[test]
    fn tokens_split_on_operators() {
        let mut error = 0;
        let mut position = 0;
        let mut token = Vec::new();
        let input = b"12 3.5 + == ";
        assert_eq!(next_token(input, &mut position, &mut token, &mut error), 16);
        assert_eq!(token, b"12");
        assert_eq!(next_token(input, &mut position, &mut token, &mut error), 16);
        assert_eq!(token, b"3.5");
        assert_eq!(next_token(input, &mut position, &mut token, &mut error), 11);
        assert_eq!(token, b"+");
        assert_eq!(next_token(input, &mut position, &mut token, &mut error), 8);
        assert_eq!(token, b"==");
        assert_eq!(next_token(input, &mut position, &mut token, &mut error), 16);
        assert!(token.is_empty());
        assert_eq!(error, 0);
    }

    // 004F3320: `<=` is found before `<`, and a lone `&` is a partial match, error 5.
    #[test]
    fn a_partial_operator_is_error_five() {
        let mut error = 0;
        let mut position = 0;
        let mut token = Vec::new();
        assert_eq!(next_token(b"<=", &mut position, &mut token, &mut error), 4);
        assert_eq!(error, 0);
        position = 0;
        assert_eq!(next_token(b"&x", &mut position, &mut token, &mut error), 16);
        assert_eq!(error, 5);
    }

    // 0047D550 and 0047D5B0.
    #[test]
    fn number_text_is_as_the_exe_reads_it() {
        assert!(is_integer_text(b"-12"));
        assert!(!is_integer_text(b"-"));
        assert!(!is_integer_text(b"1.5"));
        assert!(is_float_text(b"1.5"));
        assert!(is_float_text(b".5"));
        assert!(!is_float_text(b"."));
        assert!(!is_float_text(b"1.2.3"));
    }

    // 004FB5F0: a miss prepends {ID, 0} and answers false, so +0x08 is no longer null, and a later hit reads the mask.
    #[test]
    fn a_missed_event_test_inserts_an_entry() {
        let mut world = World::new();
        let script = world.spawn_empty().id();
        let mut events = ScriptEventList {
            flags_04: 0,
            script_00: script,
            events_08: None,
            variables_0c: Vec::new(),
        };
        assert!(!test_event(&mut events, 7, 1));
        assert_eq!(
            events.events_08.as_ref().unwrap()[0],
            ScriptEvent {
                id_00: 7,
                mask_04: 0
            }
        );
        events.events_08.as_mut().unwrap()[0].mask_04 = 3;
        assert!(test_event(&mut events, 7, 2));
        assert!(!test_event(&mut events, 7, 4));
        assert!(!test_event(&mut events, 9, 1));
        assert_eq!(events.events_08.as_ref().unwrap()[0].id_00, 9);
    }

    // 0041F830: no entry reads the flag against 1.
    #[test]
    fn with_no_extra_only_flag_one_is_set() {
        let extras = ExtraDataList::default();
        assert!(action_flag(&extras, 1));
        assert!(!action_flag(&extras, 2));
    }
}
