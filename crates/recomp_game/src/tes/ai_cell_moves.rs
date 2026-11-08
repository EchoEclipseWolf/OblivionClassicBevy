//! Phase 14c: the cell moves of the AI pass, outside a load: 004DD4B0 TESObjectREFR_MoveToCell with 004D35D0, 004CECD0 and the worldspace's persistent cell, and the door callbacks 00645A30 and 00645AF0 with their ownership and lock tests. `forms_references` keeps the load's arms of 004D35D0 and 004CECD0.
//!
//! Evidence: objdump of each address named here; P14-RUN-004.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::actor::{Actor, Character, MobileObject, TesObjectRefr, actor_process_level};
use super::actor_process::actor_process_manager_refresh;
use super::ai_conditions::{base_faction_rank, form_flags, teleport_door, the_player};
use super::ai_package::{
    actor_is_dead, cell_world, compare_length, parent_cell, reference_position, reference_world,
};
use super::ai_process::AiProcessGlobals;
use super::base_extra_list::{
    EXTRA_GLOBAL, EXTRA_LIGHT, EXTRA_LOCK, EXTRA_OWNER, EXTRA_RANK, EXTRA_TELEPORT, ExtraData,
    ExtraDataList,
};
use super::character_controller::actor_character_controller;
use super::data_handler::DataHandler;
use super::form::FormIdentity;
use super::forms_init::form_type;
use super::forms_items::set_change_flags;
use super::forms_references::{
    reference_is_persistent, save_load_active, save_load_flags, set_parent_cell,
};
use super::forms_world::{
    CellRegionLocks, TesObjectCell, TesWorldSpace, cell_is_interior, world_add_reference,
    world_remove_reference,
};
use super::global::TESGlobal;
use super::player::PlayerCharacter;
use super::tes::GameThreadLocals;
use super::tes_grid::{TesGridBoundaries, TesGridGlobals, cell_load_priority};

/// IDA: 00B0614C the altered-form NiTPrimitiveArray: +0x04 data, +0x0C size, +0x10 count.
#[derive(Resource, Debug, Default)]
pub struct AlteredForms {
    pub data: Vec<Option<Entity>>,
    pub count_10: u32,
}

/// IDA: 0046C640 TESForm_SetAltered(1): a form without bit 1 or 0x4000 not in the array joins it (00449070: the first empty slot, else appended), then bit 1. Completion: 100% of the argument 1.
pub fn set_altered(world: &mut World, form: Entity) {
    let flags = form_flags(world, form);
    if flags & 2 == 0 && flags & 0x4000 == 0 {
        let mut array = world.get_resource_or_init::<AlteredForms>();
        if !array.data.contains(&Some(form)) {
            match array.data.iter().position(Option::is_none) {
                Some(slot) => array.data[slot] = Some(form),
                None => array.data.push(Some(form)),
            }
            array.count_10 += 1;
        }
    }
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .flags_08 |= 2;
}

/// IDA: 004CCBF0 TESObjectCELL vt+0x90(1): 0046C640; an altered exterior cell outside a load passes it to its world. Completion: 100%.
pub fn cell_set_altered(world: &mut World, cell: Entity) {
    set_altered(world, cell);
    let flags = form_flags(world, cell);
    let data = world.get::<TesObjectCell>(cell).expect("a cell");
    let space = data.world_50;
    if flags & 0x4000 == 0
        && flags & 2 != 0
        && data.flags_24 & 1 == 0
        && world.resource::<GameThreadLocals>().byte_184 == 0
        && let Some(space) = space
    {
        set_altered(world, space);
    }
}

// IDA: 004D80C0(0): a type 0x30 extra's block needs its pointer and that's +0x00 nonnull to go on. `ExtraData` holds it as a dword: nonzero stops.
fn refresh_light(world: &World, reference: Entity) -> Result<(), StreamError> {
    if form_flags(world, reference) & (0x20 | 0x800) != 0 {
        return Ok(());
    }
    let list = &world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44;
    if list
        .get(EXTRA_LIGHT)
        .is_some_and(|entry| !matches!(entry.data, ExtraData::Dword(0)))
    {
        return Err(StreamError("004D80F7: the ExtraLight shader update"));
    }
    Ok(())
}

// IDA: vt+0x150(0): Character 0060E430, Creature 00625020, with no 3D: 004E0F80(null): 00439DC0 drops the reference's queued tasks; the host queues none (recorded substitution, while 00438060 stays counted). The player's 0065F950 is not ported.
pub(super) fn set_3d_null(world: &World, actor: Entity) -> Result<(), StreamError> {
    if world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .loaded_3d_3c
        != 0
    {
        return Err(StreamError("004E0FCD: Set3D(null) with loaded 3D"));
    }
    if world.get::<PlayerCharacter>(actor).is_some() {
        return Err(StreamError("0065F950: the player's Set3D"));
    }
    Ok(())
}

// IDA: vt+0x154 GetNiNode: TESObjectREFR and Creature 00422DE0 and the player's fallback: +0x3C. PlayerCharacter 006600F0: +0x5D0 when +0x588 is clear and it's nonnull, else 00612220, which is +0x3C for the player. Character 00612220: +0x3C, null when that node's +0x18 bit 0 is set: the placeholder has no node to read, and nothing sets it before M6.
fn reference_node(world: &World, reference: Entity) -> u32 {
    if let Some(player) = world.get::<PlayerCharacter>(reference)
        && player.view_flags_588[0] == 0
        && player.first_person_node_5d0.is_some()
    {
        // The port's +0x3C reads count presence; the node is an entity of its own.
        return 1;
    }
    world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .loaded_3d_3c
}

// IDA: 005FA320 Actor_RebuildHighProcess, vt+0x1A4: a process whose vt+8 is 0 returns 1 at 005FAA4C: the player's HighProcess. Otherwise a host stop. Completion: 100% of the first arm. The rebuild itself, for 004D5BD0's loop, is `cell_reference_loop::actor_rebuild_high_process`.
// TODO: [ ] 005FA367, the rebuild.
fn actor_rebuild_high_process(world: &World, actor: Entity) -> Result<(), StreamError> {
    if actor_process_level(world, actor) == 0 {
        return Ok(());
    }
    Err(StreamError("005FA367: the HighProcess rebuild"))
}

// IDA: 0043F8C0(cell, flag), which reads only the cell.
pub(crate) fn cell_attached(world: &World, cell: Entity, flag: bool) -> bool {
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

// IDA: 0065D550 on g_thePlayer.
fn player_is_sleeping(world: &World) -> bool {
    the_player(world)
        .and_then(|player| world.get::<PlayerCharacter>(player))
        .is_some_and(|player| player.pc_sleep_hours_590 > 0)
}

// IDA: N/A a helper, with no address of its own.
fn lock_cell(world: &mut World, cell: Entity) -> Result<(), StreamError> {
    world
        .get_resource_or_init::<CellRegionLocks>()
        .acquire(cell)
}

// IDA: N/A a helper, with no address of its own.
fn unlock_cell(world: &mut World, cell: Entity) {
    world.resource_mut::<CellRegionLocks>().release(cell);
}

/// IDA: 004CECD0 TESObjectCELL_RemoveReference, with TLS +0x184 clear: the removal under the cell's region lock; a persistent cell clears the 0x11 extra and the world's entry; otherwise vt+0x194(null), and a form neither temporary nor persistent makes the cell altered when DataHandler +0xCD4 is clear. Completion: 100%.
pub fn remove_reference_runtime(
    world: &mut World,
    cell: Entity,
    reference: Entity,
) -> Result<(), StreamError> {
    lock_cell(world, cell)?;
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .remove_first_matching(reference);
    unlock_cell(world, cell);
    if form_flags(world, cell) & 0x400 != 0 {
        world
            .get_mut::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44
            .set_persistent_cell(None);
        let data = world.get::<TesObjectCell>(cell).expect("a cell");
        if data.flags_24 & 1 == 0
            && let Some(space) = data.world_50
        {
            world_remove_reference(world, space, reference)?;
        }
        return Ok(());
    }
    set_parent_cell(world, reference, None)?;
    if form_flags(world, reference) & 0x4000 == 0
        && !reference_is_persistent(world, reference)?
        && world
            .get_resource::<DataHandler>()
            .is_some_and(|handler| handler.bytes_cd0[4] == 0)
    {
        cell_set_altered(world, cell);
    }
    Ok(())
}

/// IDA: 004D35D0 TESObjectCELL_InsertPlayerAndRefresh(reference), with TLS +0x184 clear: the persistent arm; otherwise the previous cell's removal, the insert under the lock with vt+0x194, the altered cell, and the attached arms. Phase 15 calls this too. Completion: 100% of the arms with no 3D; 00438060 is made when a caller lends the reference load's services (`queue`), and is a counted boundary otherwise (startup's calls), whose priority argument `TesGridBoundaries` keeps.
/// TODO: [ ] 004D377D an attached cell with 3D.
pub fn insert_reference_runtime(
    world: &mut World,
    cell: Entity,
    reference: Entity,
) -> Result<(), StreamError> {
    insert_reference_queued(world, cell, reference, None)
}

/// IDA: 004D35D0 as `insert_reference_runtime`, with the reference load's services for 004D3882's 00438060.
/// # Errors
/// As `insert_reference_runtime` and 00438060.
pub fn insert_reference_queued(
    world: &mut World,
    cell: Entity,
    reference: Entity,
    queue: Option<&mut super::reference_load::ReferenceLoadServices<'_, '_>>,
) -> Result<(), StreamError> {
    if world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
        .is_none()
    {
        return Ok(());
    }
    let loading = world.resource::<GameThreadLocals>().byte_184 != 0;
    if form_flags(world, cell) & 0x400 != 0 {
        // 004D35FD..004D3617.
        lock_cell(world, cell)?;
        world
            .get_mut::<TesObjectCell>(cell)
            .expect("a cell")
            .objects_48
            .prepend_non_null(reference);
        unlock_cell(world, cell);
        world
            .get_mut::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44
            .set_persistent_cell(Some(cell));
        let data = world.get::<TesObjectCell>(cell).expect("a cell");
        if data.flags_24 & 1 == 0
            && let Some(space) = data.world_50
        {
            world_add_reference(world, space, reference)?;
        }
        if !loading {
            cell_set_altered(world, cell);
        }
        return Ok(());
    }
    // 004D3667..004D3675.
    if let Some(previous) = parent_cell(world, reference) {
        remove_reference_runtime(world, previous, reference)?;
    }
    // 004D367A..004D36A1.
    lock_cell(world, cell)?;
    world
        .get_mut::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .prepend_non_null(reference);
    set_parent_cell(world, reference, Some(cell))?;
    unlock_cell(world, cell);
    let loaded = reference_node(world, reference) != 0;
    // 004D36A6..004D36E5: the cast's target is B05538, MobileObject.
    if form_flags(world, reference) & 0x800 == 0
        && loaded
        && world.get::<MobileObject>(reference).is_none()
    {
        return Err(StreamError("004D36DB: vt+0x148 with loaded 3D"));
    }
    if form_flags(world, reference) & 0x4000 == 0
        && !reference_is_persistent(world, reference)?
        && !loading
    {
        cell_set_altered(world, cell);
    }
    let flags = form_flags(world, reference);
    if !loading && flags & 0x4000 == 0 && flags & 0x800 == 0 {
        if cell_attached(world, cell, false) {
            if loaded {
                return Err(StreamError("004D377D: 004CEAD0 a loaded reference"));
            }
            let actor = world.get::<Actor>(reference).is_some();
            let level = world
                .get::<TesObjectCell>(cell)
                .expect("a cell")
                .process_level_26;
            let load = !actor || level == 6 || level == 5;
            if (actor && player_is_sleeping(world)) || !load {
                // 004D379D..004D37C0.
                if actor {
                    actor_process_manager_refresh(world, reference)?;
                }
                return refresh_light(world, reference);
            }
            // 004D3830..004D386A an actor, unless 0045A500 holds without +0x18 bit 4: vt+0x1A4.
            if actor && (!save_load_active(world)? || save_load_flags(world)? & 0x10 != 0) {
                actor_rebuild_high_process(world, reference)?;
            }
            // 004D386C..004D3882 00440C80(cell, null), 00438060.
            let grids = world
                .get_resource::<TesGridGlobals>()
                .and_then(|globals| globals.grids_to_load_b06a2c)
                .ok_or(StreamError(
                    "004D3875: 00440C80 before 0040EE7A publishes uGridsToLoad",
                ))?;
            let priority = cell_load_priority(world, cell, None, grids)?;
            // The priority is recorded as 004D5BD0's loop records its own; the call is made when the services are lent.
            world
                .get_resource_or_init::<TesGridBoundaries>()
                .queue_model(priority);
            if let Some(services) = queue {
                super::reference_load::model_loader_queue_reference(
                    world,
                    reference,
                    priority as u8,
                    services,
                )?;
            }
            // 004D3887.
            return refresh_light(world, reference);
        }
        // 004D3897.
        if world.get::<Actor>(reference).is_some()
            && !player_is_sleeping(world)
            && save_load_flags(world)? & 0x20 == 0
        {
            set_3d_null(world, reference)?;
        }
    }
    refresh_light(world, reference)
}

// IDA: 004F03A0 the world's persistent-cell insert. 004EF1F0 creates a missing cell: FormHeap 0x58, 004D3210, a host stop.
fn world_insert_persistent(
    world: &mut World,
    space: Entity,
    reference: Entity,
) -> Result<(), StreamError> {
    if form_flags(world, reference) & 0x4000 != 0 {
        return Ok(());
    }
    let cell = world
        .get::<TesWorldSpace>(space)
        .expect("a world")
        .persistent_cell_34
        .ok_or(StreamError("004EF1F0: creating a world's persistent cell"))?;
    insert_reference_runtime(world, cell, reference)
}

// IDA: 004F03D0 the world's persistent-cell removal.
fn world_remove_persistent(
    world: &mut World,
    space: Entity,
    reference: Entity,
) -> Result<(), StreamError> {
    match world
        .get::<TesWorldSpace>(space)
        .expect("a world")
        .persistent_cell_34
    {
        Some(cell) => remove_reference_runtime(world, cell, reference),
        None => Ok(()),
    }
}

// IDA: 004EFE80 TESWorldSpace_LookupCell(x, y): out of range warns (004EFED4) and answers null. The C++ stops.
fn world_lookup_cell(world: &mut World, space: Entity, x: i32, y: i32) -> Option<Entity> {
    if !(-32768..=0x7FFF).contains(&x) || !(-32768..=0x7FFF).contains(&y) {
        world
            .resource_mut::<AiProcessGlobals>()
            .warnings
            .push(super::data_handler_world::invalid_coordinate_warning());
        return None;
    }
    let key = (((x as i16 as i32) << 16) as u32) | u32::from(y as u16);
    world
        .get::<TesWorldSpace>(space)
        .expect("a world")
        .cells_30
        .get(key)
        .copied()
}

// IDA: vt+0x178(0) on an actor or a plain reference: Character 0065B5C0 returns at once with no controller; TESObjectREFR 004D66E0 hands vt+0x154's node to 00897A20, which returns on null.
fn refresh_node(world: &World, reference: Entity, where_: &'static str) -> Result<(), StreamError> {
    if world.get::<Actor>(reference).is_some() {
        return no_controller(world, reference, where_);
    }
    if world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .loaded_3d_3c
        != 0
    {
        return Err(StreamError(where_));
    }
    Ok(())
}

// IDA: vt+0x178(0) 0065B5C0: returns at once with no controller.
fn no_controller(world: &World, actor: Entity, where_: &'static str) -> Result<(), StreamError> {
    if actor_character_controller(world, actor).is_some() {
        return Err(StreamError(where_));
    }
    Ok(())
}

/// IDA: 004DD4B0 TESObjectREFR_MoveToCell(reference, cell, world) (cdecl), for an actor or a plain reference, which skips the mount, death, process and level arms and takes TESObjectREFR's vt+0x178 and vt+0x144. In exe order: an exterior cell is dropped; neither returns; the horse's process vt+0x474(1) is `ret 4` on Low and MiddleLow, so a mounted actor stops only past them; the old cell and world; a dead actor loses its 0x4B extra and flag 0x10000 (vt+0x9C(0)); then the interior or exterior arm and the tail. Stops that the C++ has at 004DD832 and 004DD707 aren't what the exe does. Completion: 100% with no controller and no 3D.
/// TODO: [ ] 004DD523 a mounted actor (+0xD4 is a raw word nothing sets), 004DD5E0 and 0065AB80 with a controller or 3D, 004DD7EE 00463A90.
pub fn move_to_cell(
    world: &mut World,
    actor: Entity,
    cell: Option<Entity>,
    space: Option<Entity>,
) -> Result<(), StreamError> {
    let cell = cell.filter(|&cell| cell_is_interior(world, cell));
    if cell.is_none() && space.is_none() {
        return Ok(());
    }
    world.resource_mut::<AiProcessGlobals>().cell_moves += 1;
    let mut refresh = false;
    let mut changed = false;
    if world.get::<Character>(actor).is_some()
        && world
            .get::<Actor>(actor)
            .expect("an actor")
            .horse_or_rider_d4
            != 0
    {
        return Err(StreamError("004DD523: a mounted actor's horse"));
    }
    let old_cell = parent_cell(world, actor);
    let old_world = reference_world(world, actor);
    let is_actor = world.get::<Actor>(actor).is_some();
    // vt+0x198(0): a plain reference's 004D7DD0 answers 0 past vt+0x190.
    if is_actor && actor_is_dead(world, actor, false) {
        world
            .get_mut::<TesObjectRefr>(actor)
            .expect("an actor")
            .extra_data_44
            .remove_by_type(0x4B);
        world
            .get_mut::<FormIdentity>(actor)
            .expect("a form")
            .flags_08 &= !0x1_0000;
        set_change_flags(world, actor, 1)?;
    }
    let mut new_cell = cell;
    if let Some(cell) = cell {
        if old_cell == Some(cell) {
            refresh_node(
                world,
                actor,
                "004DD5E0: an actor's controller staying in its cell",
            )?;
        } else {
            changed = true;
            if let Some(old_world) = old_world
                && reference_is_persistent(world, actor)?
            {
                world_remove_persistent(world, old_world, actor)?;
            }
            insert_reference_runtime(world, cell, actor)?;
        }
        if cell_attached(world, cell, true) {
            refresh = true;
        }
    } else {
        let space = space.expect("tested above");
        if old_cell.is_some_and(|cell| cell_is_interior(world, cell)) {
            changed = true;
        }
        if reference_is_persistent(world, actor)? && old_world != Some(space) {
            if let Some(old_world) = old_world {
                world_remove_persistent(world, old_world, actor)?;
            }
            world_insert_persistent(world, space, actor)?;
        }
        let position = reference_position(world, actor)?;
        let x = x87::fistp(position[0]);
        let y = x87::fistp(position[1]);
        new_cell = world_lookup_cell(world, space, x >> 12, y >> 12);
        if let Some(new) = new_cell {
            insert_reference_runtime(world, new, actor)?;
            if cell_attached(world, new, true) {
                if old_world == Some(space) {
                    let old_attached =
                        old_cell.is_some_and(|cell| cell_attached(world, cell, true));
                    if changed && old_attached {
                        refresh_node(world, actor, "004DD732: vt+0x178 with a controller")?;
                    } else if !old_attached
                        && world
                            .get::<TesObjectRefr>(actor)
                            .expect("an actor")
                            .loaded_3d_3c
                            != 0
                    {
                        refresh = true;
                    }
                } else {
                    refresh = true;
                    changed = true;
                    // 004DD6FD process vt+0x20, Low 00643640.
                    if let Some(process) = super::ai_conditions::process_of(world, actor) {
                        super::ai_process::process_update_stamps(world, process);
                    }
                }
            }
        } else {
            if let Some(old_cell) = old_cell {
                remove_reference_runtime(world, old_cell, actor)?;
            }
            if is_actor && !player_is_sleeping(world) && save_load_flags(world)? & 0x20 == 0 {
                set_3d_null(world, actor)?;
            }
        }
    }
    // 004DD7B0, vt+0x188.
    if is_actor && !refresh {
        actor_process_manager_refresh(world, actor)?;
        if new_cell.is_none()
            && desired_level_is_3(world, actor)?
            && !reference_is_persistent(world, actor)?
        {
            return Err(StreamError("004DD7EE: 00463A90 unloading a moved actor"));
        }
    }
    if changed && !is_actor {
        // vt+0x144 004E48B0: the node's 0088CD50, and the parent cell's Havok world's 005369B0, 00536D30 and 005374F0.
        let loaded = world
            .get::<TesObjectRefr>(actor)
            .expect("a reference")
            .loaded_3d_3c
            != 0;
        if loaded
            || parent_cell(world, actor)
                .is_some_and(|cell| super::tes_grid::cell_has_havok_world(world, cell))
        {
            return Err(StreamError(
                "004E48F9: a moved reference's node or Havok world",
            ));
        }
    } else if changed {
        // vt+0x144 0065AB80: vt+0x1C8 0065A380 with a controller, and the +0x3C Havok arm.
        no_controller(world, actor, "0065AB80: the controller of a moved actor")?;
        if world
            .get::<TesObjectRefr>(actor)
            .expect("an actor")
            .loaded_3d_3c
            != 0
        {
            return Err(StreamError("0065AB80: the 3D of a moved actor"));
        }
    }
    if refresh {
        refresh_node(world, actor, "004DD832: vt+0x178 after a refresh")?;
    }
    Ok(())
}

// IDA: 0065AD00 == 3 at 004DD7D9. 0065AD00 itself is in `actor_process`; with no new cell it comes down to 3 unless sleeping, active or attached, so the host asks through the same reads.
fn desired_level_is_3(world: &World, actor: Entity) -> Result<bool, StreamError> {
    Ok(super::actor_process::actor_desired_level(world, actor)? == 3)
}

// ---- 00645A30, 00645AF0 and what they read ----

// IDA: 0041E7B0, 0041E7D0: the form of an ownership or global extra, resolved at InitItem.
fn extra_form(list: &ExtraDataList, extra_type: u8) -> Result<Option<Entity>, StreamError> {
    let Some(entry) = list.get(extra_type) else {
        return Ok(None);
    };
    match (entry.resolved_0c, &entry.data) {
        (Some(form), _) => Ok(form),
        (None, ExtraData::Dword(0)) => Ok(None),
        _ => Err(StreamError("0041E7B0: an unresolved form extra")),
    }
}

// IDA: 0041E7F0 the 0x29 value, -1 with none.
fn extra_rank(list: &ExtraDataList) -> i32 {
    match list.get(EXTRA_RANK).map(|entry| &entry.data) {
        Some(ExtraData::Dword(value)) => *value as i32,
        _ => -1,
    }
}

// IDA: N/A a helper, with no address of its own.
fn list_of(world: &World, reference: Entity) -> &ExtraDataList {
    &world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44
}

// IDA: N/A a helper, with no address of its own.
fn cell_list(world: &World, cell: Entity) -> &ExtraDataList {
    &world.get::<TesObjectCell>(cell).expect("a cell").extra_28
}

// IDA: 004DB6B0 TESObjectREFR_GetEffectiveOwner: its own; a non-actor without one takes the linked door's, then, unless its base is of type 0x20, 0x18 or 0x12 (004DB738 `cmp byte [eax+4],12h`, where the C++ has 0x15), the cell's.
pub(super) fn effective_owner(
    world: &World,
    reference: Entity,
) -> Result<Option<Entity>, StreamError> {
    let owner = extra_form(list_of(world, reference), EXTRA_OWNER)?;
    if world.get::<Actor>(reference).is_some() || owner.is_some() {
        return Ok(owner);
    }
    if let Some(door) = teleport_door(world, reference)
        && let Some(linked) = extra_form(list_of(world, door), EXTRA_OWNER)?
    {
        return Ok(Some(linked));
    }
    let kind = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
        .map_or(0, |base| form_type(world, base));
    if !matches!(kind, 0x20 | 0x18 | 0x12)
        && let Some(cell) = parent_cell(world, reference)
    {
        return extra_form(cell_list(world, cell), EXTRA_OWNER);
    }
    Ok(None)
}

// IDA: 004DB830 the required rank: its own, the linked door's, the cell's; -1 is 0.
fn required_rank(world: &World, reference: Entity) -> i32 {
    let rank = extra_rank(list_of(world, reference));
    if rank != -1 {
        return rank;
    }
    if let Some(door) = teleport_door(world, reference) {
        let rank = extra_rank(list_of(world, door));
        if rank != -1 {
            return rank;
        }
    }
    let Some(cell) = parent_cell(world, reference) else {
        return 0;
    };
    let rank = extra_rank(cell_list(world, cell));
    if rank == -1 { 0 } else { rank }
}

// IDA: 004DB7D0 the ownership global: its own, the linked door's, the cell's.
fn ownership_global(world: &World, reference: Entity) -> Result<Option<Entity>, StreamError> {
    if let Some(global) = extra_form(list_of(world, reference), EXTRA_GLOBAL)? {
        return Ok(Some(global));
    }
    if let Some(door) = teleport_door(world, reference)
        && let Some(global) = extra_form(list_of(world, door), EXTRA_GLOBAL)?
    {
        return Ok(Some(global));
    }
    if let Some(cell) = parent_cell(world, reference) {
        return extra_form(cell_list(world, cell), EXTRA_GLOBAL);
    }
    Ok(None)
}

// IDA: 004DE770 TESObjectREFR_IsOwnedBy(actor, factionRanks). vt+0x128 00447550's +0xD0 is a raw word nothing sets, so it stops where the exe uses it as the own form.
fn reference_owned_by(
    world: &World,
    reference: Entity,
    actor: Entity,
    faction_ok: bool,
) -> Result<bool, StreamError> {
    let Some(owner) = effective_owner(world, reference)? else {
        return Ok(false);
    };
    let rank = required_rank(world, reference);
    let global = ownership_global(world, reference)?;
    if world
        .get::<Actor>(actor)
        .expect("an actor")
        .leveled_creature_template_d0
        != 0
    {
        return Err(StreamError("004DE7C3: a leveled template owner test"));
    }
    let own = world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .base_form_1c;
    let mut owned = true;
    if Some(owner) != own {
        let is_faction = form_type(world, owner) == 6;
        if !is_faction || !faction_ok {
            let value = global
                .and_then(|global| world.get::<TESGlobal>(global))
                .map(|global| f32::from_bits(global.value_bits_24));
            if value.is_none_or(|value| value == 0.0) {
                owned = false;
            }
        }
        if is_faction
            && let Some(base) = own.filter(|&own| matches!(form_type(world, own), 0x23 | 0x24))
            && base_faction_rank(world, base, owner, Some(actor) == the_player(world))? < rank
        {
            return Ok(false);
        }
    }
    Ok(owned)
}

// IDA: 004DBEA0 Unlock, 004DBE40 Lock: 004D77D0 follows the linked doors to the first reference with a lock; a byte `and`, `or` on its +0x08 bit 0; 004D9070 marks the reference's own lock, or else the linked door's, one level (vt+0x40(0x40)).
fn set_reference_locked(
    world: &mut World,
    reference: Entity,
    locked: bool,
) -> Result<(), StreamError> {
    let mut holder = Some(reference);
    while let Some(form) = holder {
        if list_of(world, form).get(EXTRA_LOCK).is_some() {
            break;
        }
        holder = teleport_door(world, form);
    }
    let Some(holder) = holder else {
        return Ok(());
    };
    if let Some(entry) = world
        .get_mut::<TesObjectRefr>(holder)
        .expect("a reference")
        .extra_data_44
        .get_mut(EXTRA_LOCK)
        && let ExtraData::Lock(block) = &mut entry.data
    {
        if locked {
            block[8] |= 1;
        } else {
            block[8] &= !1;
        }
    }
    let mut globals = world.resource_mut::<AiProcessGlobals>();
    if locked {
        globals.doors_locked += 1;
    } else {
        globals.doors_unlocked += 1;
    }
    if list_of(world, reference).get(EXTRA_LOCK).is_some() {
        return set_change_flags(world, reference, 0x40);
    }
    if let Some(door) = teleport_door(world, reference)
        && list_of(world, door).get(EXTRA_LOCK).is_some()
    {
        return set_change_flags(world, door, 0x40);
    }
    Ok(())
}

// IDA: 004C9860 the cell's +0x24 bit 6, then vt+0x40(8).
fn cell_set_flag_40(world: &mut World, cell: Entity, value: bool) -> Result<(), StreamError> {
    let mut data = world.get_mut::<TesObjectCell>(cell).expect("a cell");
    if value {
        data.flags_24 |= 0x40;
    } else {
        data.flags_24 &= !0x40;
    }
    set_change_flags(world, cell, 8)
}

// IDA: 0042B5F0 (teleport data, door, value): the owned cells of the linked door, and the door.
fn mark_teleport_cells(
    world: &mut World,
    linked: Option<Entity>,
    door: Entity,
    value: bool,
) -> Result<(), StreamError> {
    if let Some(cell) = linked.and_then(|linked| parent_cell(world, linked))
        && extra_form(cell_list(world, cell), EXTRA_OWNER)?.is_some()
    {
        cell_set_flag_40(world, cell, value)?;
    }
    if let Some(cell) = parent_cell(world, door)
        && extra_form(cell_list(world, cell), EXTRA_OWNER)?.is_some()
    {
        cell_set_flag_40(world, cell, value)?;
    }
    Ok(())
}

// IDA: 00645AF0 the unlock callback and 00645A30 the lock one: a living actor and a door (0x18) reference; the rank flag for a teleport door whose destination is in a world (0042B470), or an actor outside an interior; an owned door with a lock takes the lock or unlock, and the cells on both sides are flagged or cleared with the opposite value. Both answer 0.
fn door_lock_callback(
    world: &mut World,
    reference: Entity,
    actor: Entity,
    lock: bool,
) -> Result<(), StreamError> {
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
        && super::ai_conditions::reference_lock(world, reference).is_some()
    {
        set_reference_locked(world, reference, lock)?;
        if teleport {
            mark_teleport_cells(world, linked, reference, !lock)?;
        }
    }
    Ok(())
}

/// IDA: 00446B90 DataHandler_ForEachReferenceInRange(cell, position, 5000, position, 5000, callback, actor): an interior through 004CBC20, under the cell's region lock (which the C++ doesn't take), with +0x48's next read before the callback: a reference at or past the radius (00480520, each delta a float) is skipped. An exterior goes through 004F0750, a host stop. `lock` picks 00645A30 over 00645AF0. Completion: 100% of the interior arm.
/// TODO: [ ] 004F0750, the exterior search: no shipped package reached it in P14-RUN-004.
pub fn doors_near(world: &mut World, actor: Entity, lock: bool) -> Result<(), StreamError> {
    let Some(cell) = parent_cell(world, actor) else {
        return Ok(());
    };
    if !cell_is_interior(world, cell) {
        if cell_world(world, Some(cell)).is_some() {
            return Err(StreamError("004F0750: the exterior range search"));
        }
        return Ok(());
    }
    let center = reference_position(world, actor)?;
    lock_cell(world, cell)?;
    let items: Vec<Entity> = world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .items()
        .collect();
    for reference in items {
        let position = reference_position(world, reference)?;
        let delta = [
            position[0] - center[0],
            position[1] - center[1],
            position[2] - center[2],
        ];
        if compare_length(delta, 5000.0) >= 0 {
            continue;
        }
        door_lock_callback(world, reference, actor, lock)?;
    }
    unlock_cell(world, cell);
    Ok(())
}
