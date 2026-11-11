//! The script commands and functions a new game reaches before the player's first `MoveTo`: `005172C6`'s targets and `004FADD5`'s, in `004FCA30`'s table at `00B0C8C0`. Each row's parameter types are `+0x14`'s, and `+0x10` is whether it needs a reference. A command nothing has reached is a stop.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::logging::{Level, emit};
use oblivion_core::x87;

use super::script_vm::{
    Runner, ScriptVmCounts, ScriptVmGlobals, parse_parameters, script_run_forced,
};
use crate::tes::actor::TesObjectRefr;
use crate::tes::actor::actor_sitting_state;
use crate::tes::actor::{Actor, DispositionEntry, set_death_state};
use crate::tes::actor_enable::{disable_actor_process, enable_actor_reset, enable_actor_update};
use crate::tes::actor_process::actor_values_add_modifier;
use crate::tes::ai_cell_moves::set_3d_null;
use crate::tes::ai_conditions::run_script_handler;
use crate::tes::ai_process::actor_get_actor_value_float;
use crate::tes::base_extra_list::{
    EXTRA_ENABLE_STATE_PARENT, EXTRA_GHOST, ExtraData, new_extra_entry,
};
use crate::tes::form::FormIdentity;
use crate::tes::form::set_altered;
use crate::tes::form_components::TesActorBaseData;
use crate::tes::form_components::{FormRef, TesSpellList, base_faction_rank};
use crate::tes::forms_dialogue::TesQuest;
use crate::tes::forms_init::{form_type, lookup};
use crate::tes::forms_items::set_change_flags;
use crate::tes::forms_misc::TesFaction;
use crate::tes::forms_references::actor_base_form;
use crate::tes::forms_references::form_flags;
use crate::tes::forms_references::parent_cell;
use crate::tes::forms_references::reference_cell;
use crate::tes::forms_world::TesObjectCell;
use crate::tes::forms_world::cell_set_altered;
use crate::tes::player::{
    PlayerCharacter, add_topic, set_in_chargen, stop_on_active_quest, the_player,
};

// IDA: N/A a row of the table: +0x10, and +0x14's types.
fn command_row(op: u16) -> Option<(bool, &'static [u32])> {
    Some(match op {
        4125 => (true, &[7]),
        4129 | 4130 | 4395 => (true, &[]),
        4150 | 4151 => (false, &[14]),
        4153 => (false, &[14, 23]),
        4179 => (true, &[6, 1]),
        4184 => (false, &[13]),
        4254 => (true, &[4, 2, 2, 2]),
        4300 | 4332 | 4433 => (true, &[1]),
        4327 | 4337 => (false, &[1]),
        4366 => (true, &[5, 1]),
        _ => return None,
    })
}

/// IDA: 005169A0's `default:` with `a10 = 0`: a row needing a reference warns and answers 0 with none. `+0x25` sets the runner's `+0xA1` before the call, and the command's own answer is dropped, so a failed parse still answers 1 through `005172D3`. Completion: 100% of the rows `command_row` names; another row is a stop.
/// # Errors
/// A row or an arm nothing has reached.
#[allow(clippy::too_many_arguments)]
pub(in crate::tes) fn run_command(
    world: &mut World,
    runner: &mut Runner,
    script: Entity,
    data: &[u8],
    op: u16,
    current: Option<Entity>,
    operands: usize,
) -> Result<bool, GameError> {
    let Some((needs_reference, types)) = command_row(op) else {
        emit(Level::Error, &format!("005172C6: command {op}"));
        return Err(GameError::unclassified(
            0x005172C6,
            "005172C6: a generic script command",
        ));
    };
    if needs_reference && current.is_none() {
        emit(
            Level::Warning,
            &format!("Function {op} requires a reference."),
        );
        return Ok(false);
    }
    let id_of = |world: &World, form: Option<Entity>| {
        form.and_then(|form| world.get::<FormIdentity>(form))
            .map_or(0, |identity| identity.form_id_0c)
    };
    let log = (
        u32::from(op),
        id_of(world, current),
        id_of(world, Some(script)),
    );
    let crt = crate::tes::form_arms::crt_probe(world);
    let mut counts = world.get_resource_or_init::<ScriptVmCounts>();
    counts.commands.push(log);
    if let Some(crt) = crt {
        counts.command_crt.push(crt);
    }
    // 0051729E, the row's +0x25.
    if op == 4254 {
        runner.moved_a1 = true;
    }
    let mut cursor = operands;
    let Some(values) = parse_parameters(world, runner, script, data, &mut cursor, types)? else {
        return Ok(true);
    };
    match op {
        4184 => add_topic(world, values[0])?,
        4179 => mod_disposition(world, current, values[0], values[1])?,
        4366 => force_actor_value(world, current, values[0], values[1])?,
        4129 => enable(world, runner, current)?,
        4130 => disable(world, runner, current),
        4150 => quest_set_running(world, values[0], true)?,
        4151 => quest_set_running(world, values[0], false)?,
        4153 => set_stage(world, values[0], values[1] as u8)?,
        4300 => set_form_flag(world, current, values[0] != 0, 0x2000, true)?,
        4433 => {
            // 00506750: -1 is no argument. 0046AB20 then vt+0x40(1).
            if values[0] != u32::MAX {
                set_form_flag(world, current, values[0] != 0, 0x10_0000, true)?;
            }
        }
        4327 => set_in_chargen(world, values[0] != 0),
        4395 => disable_linked_path_points(world, current)?,
        4254 => {
            let player = the_player(world);
            if current == player {
                if let (Some(player), Some(target)) = (player, lookup(world, values[0])) {
                    let offset = [1, 2, 3]
                        .map(|at| values.get(at).map_or(0.0, |&bits| f32::from_bits(bits)));
                    crate::tes::player_move::move_player_to(world, player, target, offset)?;
                }
                return Ok(true);
            }
            // 00512870: 00512540 with a reference and a target; neither null here, since 004FAE80 fails a null target.
            if let (Some(reference), Some(target)) = (current, lookup(world, values[0])) {
                // 00512873: the offsets start at 0.0, and a script may give none.
                let offset =
                    [1, 2, 3].map(|at| values.get(at).map_or(0.0, |&bits| f32::from_bits(bits)));
                crate::tes::move_to::move_reference_to(world, reference, target, offset)?;
            }
        }
        4125 => remove_spell(world, current, values[0])?,
        4332 => set_ghost(world, current, values[0] as i32),
        4337 => set_unconscious(world, current, values[0] != 0)?,
        _ => unreachable!("every row has an arm"),
    }
    Ok(true)
}

// IDA: 00510B90 RemoveSpell(spell) on an actor: vt+0x2E0, the player's 006675B0: 005F3CC0, and when that removed it, the selected item cleared if it was the spell. 005F3CC0: the base's spell list (vt+0x170 +0x54, 0046F5F0) holding it is removed (0046FEF0) with the base's vt+0x40(0x20), a stop; an ability or a disease (the +0x18 part's vt+0x18, +0x34: 4 or 1) with an active effect on the magic target (0066B0B0, +0x1E4) is a stop; a lesser power (2) on the +0x9C list (005E7B50) is a stop. 00579B60's "Spell '%s' not found in %s" prints only with the console open (+0x31) or 00B3B908, which only a console command sets. Completion: 100% for the player and a spell it doesn't hold.
fn remove_spell(world: &mut World, actor: Option<Entity>, spell: u32) -> Result<(), GameError> {
    let Some(actor) = actor.filter(|&actor| world.get::<Actor>(actor).is_some()) else {
        return Ok(());
    };
    if the_player(world) != Some(actor) {
        return Err(GameError::unclassified(
            0x00510C06,
            "00510C06: a non-player's vt+0x2E0",
        ));
    }
    let Some(spell) = lookup(world, spell) else {
        return Ok(());
    };
    player_remove_spell(world, actor, spell).map(|_| ())
}

/// IDA: the player's vt+0x2E0 (006675B0) on a spell: 005F3CC0 Actor_RemoveSpell, as `remove_spell` reads it (a base-list spell,
/// an ability or disease with active effects, and a lesser power on the +0x9C list are stops); a spell none of those reach is
/// not removed and answers 0, so 006675B0 leaves the selected item. Completion: 100% of that arm.
/// # Errors
/// The stops above.
pub(crate) fn player_remove_spell(
    world: &mut World,
    actor: Entity,
    spell: Entity,
) -> Result<bool, GameError> {
    let held = world
        .get::<TesObjectRefr>(actor)
        .and_then(|refr| refr.base_form_1c)
        .and_then(|base| world.get::<TesSpellList>(base))
        .is_some_and(|list| list.spells_04.contains(&FormRef::Resolved(Some(spell))));
    if held {
        return Err(GameError::unclassified(
            0x005F3D2B,
            "005F3D2B: removing a base spell",
        ));
    }
    let kind = world
        .get::<crate::tes::forms_magic::MagicFormData>(spell)
        .map_or(0, |data| data.type_34);
    let player = world.get::<PlayerCharacter>(actor).expect("the player");
    if matches!(kind, 4 | 1) && player.active_effects_1e4.is_some_and(|list| list != [0; 2]) {
        return Err(GameError::unclassified(
            0x005F3D8B,
            "005F3D8B: an active effect list to search",
        ));
    }
    if kind == 2
        && world
            .get::<Actor>(actor)
            .expect("an actor")
            .greater_power_list_9c
            != [0; 2]
    {
        return Err(GameError::unclassified(
            0x005F3DB7,
            "005F3DB7: the +0x9C power list",
        ));
    }
    Ok(false)
}

// IDA: 00505A30 SetGhost(value) on an actor: 005E8EC0, 0041F150(value > 0) on +0x44: set, an ExtraGhost (00429FD0) added when the 0x25 bit is clear; clear, the 0x25 extra removed when set. 00B361AC's "SetGhost >> %d" is a console print the reached runs don't make. Completion: 100%.
fn set_ghost(world: &mut World, actor: Option<Entity>, value: i32) {
    let Some(actor) = actor.filter(|&actor| world.get::<Actor>(actor).is_some()) else {
        return;
    };
    let list = &mut world
        .get_mut::<TesObjectRefr>(actor)
        .expect("a reference")
        .into_inner()
        .extra_data_44;
    let held = list.get(EXTRA_GHOST).is_some();
    if value > 0 {
        if !held {
            list.add(new_extra_entry(EXTRA_GHOST, ExtraData::Marker));
        }
    } else if held {
        list.remove_by_type(EXTRA_GHOST);
    }
}

// IDA: 00505AC0 SetUnconscious(value) on an actor: set, 005E8E30(1), Actor_SetDeathState(3), then 005E02B0, the process's vt+0x194(actor), nothing on a LowProcess; clear, 005E8E30(0), which sets state 0 only from 3. Completion: 100% with no process or a LowProcess.
fn set_unconscious(world: &mut World, actor: Option<Entity>, value: bool) -> Result<(), GameError> {
    let Some(actor) = actor.filter(|&actor| world.get::<Actor>(actor).is_some()) else {
        return Ok(());
    };
    if value {
        set_death_state(world, actor, 3)?;
        if let Some(process) = crate::tes::actor::process_of(world, actor)
            && world
                .get::<crate::tes::form::OriginalVtable>(process)
                .map(|vtable| vtable.0)
                != Some(crate::tes::actor_process::LOW_PROCESS_VTABLE)
        {
            return Err(GameError::unclassified(
                0x005E02C4,
                "005E02C4: vt+0x194 on a process other than a LowProcess",
            ));
        }
    } else if world.get::<Actor>(actor).expect("an actor").dead_state_b0 == 3 {
        set_death_state(world, actor, 0)?;
    }
    Ok(())
}

// IDA: 00503A50 ModDisposition(reference, delta): `fild` of the int, then 005E2070 addDisposition: vt+0x40(0x8000), and a first entry for that reference prepends {truncated delta, reference}. 004F4E20's answer goes to the unread result and `00B36180..00B36188`, which 004F5DB0 clears each frame, so its vt+0x224 isn't run.
// TODO: [ ] A second change for the same reference, which reads vt+0x224 GetDisposition.
fn mod_disposition(
    world: &mut World,
    actor: Option<Entity>,
    other: u32,
    delta: u32,
) -> Result<(), GameError> {
    let Some(actor) = actor.filter(|&form| world.get::<Actor>(form).is_some()) else {
        return Ok(());
    };
    let Some(other) = lookup(world, other) else {
        return Ok(());
    };
    set_change_flags(world, actor, 0x8000)?;
    let mut data = world.get_mut::<Actor>(actor).expect("an actor");
    if data
        .disposition_list
        .iter()
        .any(|entry| entry.reference == other)
    {
        return Err(GameError::unclassified(
            0x005E2070,
            "005E2070: a second change, through GetDisposition",
        ));
    }
    data.disposition_list.insert(
        0,
        DispositionEntry {
            modifier: delta as i32,
            reference: other,
        },
    );
    Ok(())
}

// IDA: 00501890 ForceActorValue(av, value): value minus vt+0x288, a float, through vt+0x29C 005E29F0: 0065CA60 on +0x88, then vt+0x40(0x200000). 00B361AC's console arm isn't reached. Completion: 100% of the codes 005E29F0 has no more arms for; code 10 and 8 falling, and 18 and 27, are stops.
fn force_actor_value(
    world: &mut World,
    actor: Option<Entity>,
    code: u32,
    value: u32,
) -> Result<(), GameError> {
    let Some(actor) = actor.filter(|&form| world.get::<Actor>(form).is_some()) else {
        return Ok(());
    };
    let current = actor_get_actor_value_float(world, actor, code)?;
    let delta = x87::stored_float(f64::from(value as i32) - f64::from(current));
    if (code == 10 || code == 8) && delta < 0.0 {
        return Err(GameError::unclassified(
            0x005E2A12,
            "005E2A12: fatigue or health falling",
        ));
    }
    if code == 18 || code == 27 {
        return Err(GameError::unclassified(
            0x005E2A77,
            "005E2A77: a skill that refreshes the container",
        ));
    }
    actor_values_add_modifier(
        &mut world
            .get_mut::<Actor>(actor)
            .expect("an actor")
            .av_modifiers_88,
        code as u8,
        delta,
    )?;
    set_change_flags(world, actor, 0x20_0000)
}

// IDA: 0050A150 Enable: a null reference only clears the event list's +0x04 bit 0. A reference with an enable state parent warns and does nothing; else 004FA540 takes it off 00B361CC, and with +0x08 bit 11 004DD850 enables it. Then the bit.
fn enable(
    world: &mut World,
    runner: &mut Runner,
    reference: Option<Entity>,
) -> Result<(), GameError> {
    if let Some(reference) = reference {
        if has_enable_parent(world, reference) {
            emit(
                Level::Warning,
                "Enable is being called on a reference that has an enable state parent.",
            );
            return Ok(());
        }
        world
            .get_resource_or_init::<ScriptVmGlobals>()
            .disabled_b361cc
            .retain(|&entry| entry != reference);
        if form_flags(world, reference) & 0x800 != 0 {
            enable_reference(world, reference)?;
        }
    }
    runner.events_08.flags_04 &= !1;
    Ok(())
}

// IDA: vt+0x90(1) 004D6DB0: unless +0x4000, the cell the TESChildCell's vt+0 (004DDCB0, `reference_cell`) answers takes vt+0x90(1) (`cell_set_altered`), then 0046C640. For a persistent reference with no +0x40 that's its 0x11 extra's cell.
fn reference_set_altered(world: &mut World, reference: Entity) -> Result<(), GameError> {
    if form_flags(world, reference) & 0x4000 == 0
        && let Some(cell) = reference_cell(world, reference)?
    {
        cell_set_altered(world, cell);
    }
    set_altered(world, reference);
    Ok(())
}

// IDA: the 0x40 extra's children: bit 0 of a child's 0x3F flags inverts it, so it takes the other arm.
fn enable_state_children(world: &World, reference: Entity) -> Vec<(Entity, bool)> {
    let refr = world.get::<TesObjectRefr>(reference).expect("a reference");
    let Some(ExtraData::EnableStateChildren(children)) =
        refr.extra_data_44.get(0x40).map(|entry| &entry.data)
    else {
        return Vec::new();
    };
    children
        .iter()
        .map(|&child| {
            let inverted = matches!(
                world
                    .get::<TesObjectRefr>(child)
                    .and_then(|refr| refr.extra_data_44.get(EXTRA_ENABLE_STATE_PARENT))
                    .map(|entry| &entry.data),
                Some(ExtraData::EnableStateParent { flags_10, .. }) if flags_10 & 1 != 0
            );
            (child, inverted)
        })
        .collect()
}

// IDA: 004DD850, for a reference with +0x08 bit 11: vt+0x90(1), vt+0x40(0x40000000), 0046ABA0(0). An actor takes `enable_actor_reset` here. A cell of level 6 is a stop. An actor then takes `enable_actor_update`. 00665260, then the 0x40 children: the inverted one disabled, the rest enabled. Completion: 100% outside an attached cell, with the actor arms' stops.
fn enable_reference(world: &mut World, reference: Entity) -> Result<(), GameError> {
    if form_flags(world, reference) & 0x800 == 0 {
        return Ok(());
    }
    reference_set_altered(world, reference)?;
    set_change_flags(world, reference, 0x4000_0000)?;
    world
        .get_mut::<FormIdentity>(reference)
        .expect("a form")
        .flags_08 &= !0x800;
    set_change_flags(world, reference, 0x4000_0001)?;
    let actor = world.get::<Actor>(reference).is_some();
    if actor {
        enable_actor_reset(world, reference)?;
    }
    if parent_cell(world, reference)
        .and_then(|cell| world.get::<TesObjectCell>(cell))
        .is_some_and(|cell| cell.process_level_26 == 6)
    {
        return Err(GameError::unclassified(
            0x004DD92F,
            "004DD92F: enabling a reference in an attached cell",
        ));
    }
    if actor {
        enable_actor_update(world, reference)?;
    }
    stop_on_active_quest(world)?;
    for (child, inverted) in enable_state_children(world, reference) {
        if inverted {
            disable_reference(world, child)?;
        } else {
            enable_reference(world, child)?;
        }
    }
    Ok(())
}

/// IDA: 004E4690, for a reference without +0x08 bit 11: 004E1F80, whose Havok world a cell not attached lacks, vt+0x90(1), vt+0x40(0x40000000), 0046ABA0(1), for an actor with a process `disable_actor_process`, vt+0x150(0), which a reference with no 3D passes and an actor takes as Character_Set3D(null), 00665260, then the 0x40 children, the inverted one enabled. 00420480's 0x42 list and 004D9310(0) are stops when they'd act: a SOUN base's sound by form ID (006ACA40), or an ACTI, DOOR or LIGH base's 0x5B handle. Completion: 100% outside an attached cell, for a non-actor or an actor on a LowProcess.
/// # Errors
/// An actor, an attached cell, or a loaded reference.
pub(in crate::tes) fn disable_reference(
    world: &mut World,
    reference: Entity,
) -> Result<(), GameError> {
    if form_flags(world, reference) & 0x800 != 0 {
        return Ok(());
    }
    let actor = world.get::<Actor>(reference).is_some();
    if parent_cell(world, reference)
        .and_then(|cell| world.get::<TesObjectCell>(cell))
        .is_some_and(|cell| cell.process_level_26 == 6)
    {
        return Err(GameError::unclassified(
            0x004E1F80,
            "004E1F80: disabling a reference in an attached cell",
        ));
    }
    reference_set_altered(world, reference)?;
    set_change_flags(world, reference, 0x4000_0000)?;
    world
        .get_mut::<FormIdentity>(reference)
        .expect("a form")
        .flags_08 |= 0x800;
    set_change_flags(world, reference, 0x4000_0001)?;
    if actor {
        disable_actor_process(world, reference)?;
        // 004E47CC vt+0x150(0): Character 0060E430.
        set_3d_null(world, reference)?;
    } else if world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .loaded_3d_3c
        != 0
    {
        return Err(GameError::unclassified(
            0x004E47CC,
            "004E47CC: vt+0x150(0) with a loaded 3D",
        ));
    }
    stop_on_active_quest(world)?;
    for (child, inverted) in enable_state_children(world, reference) {
        if inverted {
            enable_reference(world, child)?;
        } else {
            disable_reference(world, child)?;
        }
    }
    let refr = world.get::<TesObjectRefr>(reference).expect("a reference");
    if refr.extra_data_44.get(0x42).is_some() {
        return Err(GameError::unclassified(
            0x004E4820,
            "004E4820: the 0x42 list's references",
        ));
    }
    let base = refr.base_form_1c.map_or(0, |base| form_type(world, base));
    if base == 0x0A
        || (matches!(base, 0x12 | 0x18 | 0x1A) && refr.extra_data_44.get(0x5B).is_some())
    {
        return Err(GameError::unclassified(
            0x004E4899,
            "004E4899: a reference's sound stop (004D9310)",
        ));
    }
    Ok(())
}

// IDA: 0050A240 Disable: a null, disabled or +0x4000 reference only sets the event list's bit. With an enable state parent it warns and does nothing; else 004FBB30 prepends it to 00B361CC once, for 004FA580 at the end of 004402F0. Then the bit.
fn disable(world: &mut World, runner: &mut Runner, reference: Option<Entity>) {
    if let Some(reference) = reference
        && form_flags(world, reference) & 0x4800 == 0
    {
        if has_enable_parent(world, reference) {
            emit(
                Level::Warning,
                "Disable is being called on a reference that has an enable state parent.",
            );
            return;
        }
        let mut vm = world.get_resource_or_init::<ScriptVmGlobals>();
        if !vm.disabled_b361cc.contains(&reference) {
            vm.disabled_b361cc.insert(0, reference);
        }
    }
    runner.events_08.flags_04 |= 1;
}

// IDA: 004D7990, 00420260: the 0x3F extra's parent.
fn has_enable_parent(world: &World, reference: Entity) -> bool {
    world
        .get::<TesObjectRefr>(reference)
        .is_some_and(|refr| refr.extra_data_44.get(EXTRA_ENABLE_STATE_PARENT).is_some())
}

// IDA: 00529820 through 00502D30 and 00502D80: +0x3C bit 0, then vt+0x40(4).
fn quest_set_running(world: &mut World, id: u32, running: bool) -> Result<(), GameError> {
    let Some(quest) = lookup(world, id) else {
        return Ok(());
    };
    let mut data = world
        .get_mut::<TesQuest>(quest)
        .ok_or(GameError::unclassified(0x00529820, "00529820: not a quest"))?;
    if running {
        data.flags_3c |= 1;
    } else {
        data.flags_3c &= !1;
    }
    set_change_flags(world, quest, 4)
}

// IDA: 00529AC0 SetStage(stage): the first group of that index, else nothing. With +0x3C bit 1 clear or bit 3 set, bit 0 and vt+0x40(4). A higher stage goes to +0x5C. 0052B080, then vt+0x40(0x10000000). Completion: 100%.
fn set_stage(world: &mut World, id: u32, stage: u8) -> Result<(), GameError> {
    let Some(quest) = lookup(world, id) else {
        return Ok(());
    };
    let group = world
        .get::<TesQuest>(quest)
        .ok_or(GameError::unclassified(0x00529AC0, "00529AC0: not a quest"))?
        .stages_40
        .iter()
        .position(|group| group.index_00 == stage);
    let Some(group) = group else {
        return Ok(());
    };
    let flags = world.get::<TesQuest>(quest).expect("a quest").flags_3c;
    if flags & 2 == 0 || flags & 8 != 0 {
        world.get_mut::<TesQuest>(quest).expect("a quest").flags_3c |= 1;
        set_change_flags(world, quest, 4)?;
    }
    let mut data = world.get_mut::<TesQuest>(quest).expect("a quest");
    if stage > data.byte_5c {
        data.byte_5c = stage;
    }
    resolve_stage_group(world, quest, group)?;
    set_change_flags(world, quest, 0x1000_0000)
}

// IDA: 0052B080(group, quest): an unset +0x01, or a quest with +0x3C bit 3: set +0x01, then 0052ADF0 on each item. Completion: 100%.
fn resolve_stage_group(world: &mut World, quest: Entity, group: usize) -> Result<(), GameError> {
    let data = world.get::<TesQuest>(quest).expect("a quest");
    if data.stages_40[group].flags_01 != 0 && data.flags_3c & 8 == 0 {
        return Ok(());
    }
    let count = data.stages_40[group].items_04.len();
    world.get_mut::<TesQuest>(quest).expect("a quest").stages_40[group].flags_01 = 1;
    for item in 0..count {
        resolve_stage_item(world, quest, group, item)?;
    }
    Ok(())
}

// IDA: 0052ADF0: an item already dated warns unless its quest has +0x3C bit 3. 0056A950 over +0x04 with the player. On a pass: +0x64 the date (0047D390), bit 0 of +0x00 finishes the quest (00529840, and 00660450 for the active quest), +0x61 logs it (006697A0). With 004F9FA0 and a compiled +0x20: the embedded script's +0x28 is zeroed and it runs on the player with the player's events, cl = 1.
// TODO: [ ] 006697A0 and the finishing arm, which no stage up to the first MoveTo reaches.
fn resolve_stage_item(
    world: &mut World,
    quest: Entity,
    group: usize,
    item: usize,
) -> Result<(), GameError> {
    let player = the_player(world).ok_or(GameError::unclassified(
        0x0052ADF0,
        "0052ADF0: a null g_thePlayer",
    ))?;
    let data = world.get::<TesQuest>(quest).expect("a quest");
    let entry = &data.stages_40[group].items_04[item];
    if entry.field_64 != 0 && data.flags_3c & 8 == 0 {
        emit(
            Level::Warning,
            "Trying to resolve a quest stage item that already has a log date.",
        );
        return Ok(());
    }
    let conditions = entry.conditions_04.clone();
    let (flags, log, script) = (entry.flags_00, entry.log_61, entry.script_0c);
    if !crate::tes::ai_conditions::evaluate_conditions_for_script(world, &conditions, player)? {
        return Ok(());
    }
    if flags & 1 != 0 {
        return Err(GameError::unclassified(
            0x00529840,
            "00529840: a stage that finishes its quest",
        ));
    }
    if log != 0 {
        return Err(GameError::unclassified(
            0x006697A0,
            "006697A0: a stage with a log entry",
        ));
    }
    // +0x64 points at a 4-byte date. Nothing reached reads the date, so this keeps only that it's set.
    world.get_mut::<TesQuest>(quest).expect("a quest").stages_40[group].items_04[item].field_64 = 1;
    script_run_forced(world, script, player)
}

// IDA: 0046AA50 and 0046AB20: a bit of +0x08. The first also takes vt+0x40(1), and 00506750 calls that itself.
fn set_form_flag(
    world: &mut World,
    form: Option<Entity>,
    on: bool,
    flag: u32,
    mark: bool,
) -> Result<(), GameError> {
    let Some(form) = form else {
        return Ok(());
    };
    let mut identity = world.get_mut::<FormIdentity>(form).expect("a form");
    if on {
        identity.flags_08 |= flag;
    } else {
        identity.flags_08 &= !flag;
    }
    if mark {
        set_change_flags(world, form, 1)?;
    }
    Ok(())
}

// IDA: 005062D0 Cmd_DisableLinkedPathPoints_Execute: with a calling reference ([esp+0Ch]), 004E51F0(reference, 0), the pathing glue's `set_linked_path_points_enabled`; answers 1. Completion: 100% of the handler; 004E51F0's exterior loop is a stop there.
fn disable_linked_path_points(world: &mut World, current: Option<Entity>) -> Result<(), GameError> {
    let Some(reference) = current else {
        return Ok(());
    };
    crate::tes::path_search::set_linked_path_points_enabled(world, reference, false)
}

// IDA: N/A a function's row: +0x10, +0x14's types.
fn function_row(op: u32) -> Option<(bool, &'static [u32])> {
    Some(match op {
        4097 => (true, &[4]),
        4101 | 4131 | 4142 | 4176 | 4239 | 4299 | 4385 | 4396 | 4423 => (true, &[]),
        4108 | 4114 | 4173 | 4267 | 4272 | 4347 | 4461 => (false, &[]),
        4155 => (false, &[14, 23]),
        4163 => (true, &[9]),
        4169 => (true, &[17]),
        4180 => (false, &[25]),
        4291 | 4293 => (false, &[17]),
        4319 => (true, &[11]),
        _ => return None,
    })
}

/// IDA: 004FADD5's other targets, through `run_script_handler` where the condition table shares the evaluator: 4097 004F43C0, 4101 004F4760, 4114 004F46E0, 4131 004F60E0, 4142 004F4890, 4155 004F4BC0, 4163 004F6D70, 4173 004F4EF0, 4176 004F4F50, 4180 004F5010, 4239 004F7C00, 4267 004F55C0, 4319 004F5730, 4385 004F8F30, 4396 004F9010, 4461 004F87F0. 4108 004F6010 is the quest script's own elapsed time when it has one. Completion: 100% of the rows named here.
/// # Errors
/// A row or an arm nothing has reached.
pub(in crate::tes) fn run_function(
    world: &mut World,
    runner: &mut Runner,
    script: Entity,
    data: &[u8],
    cursor: &mut usize,
    op: u32,
    reference: Option<Entity>,
) -> Result<Option<f64>, GameError> {
    let Some((needs_reference, types)) = function_row(op) else {
        emit(Level::Error, &format!("004FADD5: function {op}"));
        return Err(GameError::unclassified(
            0x004FADD5,
            "004FADD5: a script function no startup run reaches",
        ));
    };
    if needs_reference && reference.is_none() {
        return Ok(None);
    }
    let Some(values) = parse_parameters(world, runner, script, data, cursor, types)? else {
        return Ok(None);
    };
    match op {
        4108 => {
            let body = world
                .get::<crate::tes::forms_script::Script>(script)
                .expect("a script");
            if body.header_18[0x10] != 0 && body.floats_34[2] > 0.0 {
                return Ok(Some(f64::from(body.floats_34[2])));
            }
            let frame = world
                .get_resource_or_init::<ScriptVmGlobals>()
                .frame_seconds_b33e9c;
            Ok(Some(f64::from(frame)))
        }
        // 004F60E0 with no reference reads the run's own list.
        4131 if reference.is_none() => Ok(Some(f64::from(runner.events_08.flags_04 & 1))),
        4272 => {
            let player = the_player(world).ok_or(GameError::unclassified(
                0x004F5600,
                "004F5600: a null g_thePlayer",
            ))?;
            Ok(Some(f64::from(u8::from(
                world
                    .get::<PlayerCharacter>(player)
                    .expect("the player")
                    .is_a_murderer_611
                    != 0,
            ))))
        }
        // 004F82C0 GetDestroyed: +0x08 bit 13.
        4299 => {
            Ok(Some(f64::from(u8::from(reference.is_some_and(|form| {
                form_flags(world, form) & 0x2000 != 0
            })))))
        }
        // 004F7280 GetFactionRank: -1 for a non-actor. A template's base with no factions gives way to the reference's own.
        4169 => {
            let Some(actor) = reference.filter(|&form| world.get::<Actor>(form).is_some()) else {
                return Ok(Some(-1.0));
            };
            let mut base = actor_base_form(world, actor, true)?;
            if base.is_some_and(|base| {
                world
                    .get::<TesActorBaseData>(base)
                    .is_some_and(|data| data.factions_18.is_empty())
            }) {
                base = actor_base_form(world, actor, false)?;
            }
            let faction = lookup(world, values[0]);
            let is_player = Some(actor) == the_player(world);
            match (base, faction) {
                (Some(base), Some(faction)) => Ok(Some(f64::from(base_faction_rank(
                    world, base, faction, is_player,
                )?))),
                _ => Ok(Some(-1.0)),
            }
        }
        // 004F8140 and 004F81A0: the faction's +0x34 bits 6 and 4.
        4291 | 4293 => {
            let bit = if op == 4291 { 0x40 } else { 0x10 };
            let set = lookup(world, values[0])
                .and_then(|faction| world.get::<TesFaction>(faction))
                .is_some_and(|faction| faction.flags_34 & bit != 0);
            Ok(Some(f64::from(u8::from(set))))
        }
        // 004F59B0, vt+0x208 0066B0A0: +0x6F8.
        4347 => {
            let player = the_player(world).ok_or(GameError::unclassified(
                0x004F59B0,
                "004F59B0: a null g_thePlayer",
            ))?;
            Ok(Some(f64::from(
                world
                    .get::<PlayerCharacter>(player)
                    .expect("the player")
                    .infamy_6f8,
            )))
        }
        // 004F5CB0: a sitting state of 4, or a +0x104 of 4 on a creature, with vt+0x380's +0xD4. vt+0x388 answers 0 on both vtables.
        4423 => {
            let Some(actor) = reference.filter(|&form| world.get::<Actor>(form).is_some()) else {
                return Ok(Some(0.0));
            };
            if actor_sitting_state(world, actor) == 4 {
                return Err(GameError::unclassified(
                    0x004F5CCC,
                    "004F5CCC: a rider's horse",
                ));
            }
            Ok(Some(0.0))
        }
        _ => {
            let form = types
                .first()
                .filter(|&&kind| kind != 1 && kind != 23)
                .and_then(|_| lookup(world, values[0]));
            let second = values.get(1).copied().unwrap_or(0);
            run_script_handler(world, (op - 4096) as u16, reference, form, second).map(Some)
        }
    }
}
