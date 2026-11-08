//! Phase 12d: the world InitItems: 004CD960 cell, 004DF820 reference, 004F2C10 worldspace, with 00426270 the extra lists, 004EF290 the subspace pass and 004F28B0 the DistantLOD read. What the references build in their inventories is in `inventory`.
//!
//! Recorded host boundaries: 00517330, the script interpreter 004FBE00 reaches, and 00680090's pathing node map, each counted (`InventoryGlobals`). Every "not found" warning keeps its text; no shipped record reaches one (P12-RUN-005), so they stand on the exe alone. Evidence: objdump of each address named here.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::actor::{Actor, MobileObject, TesObjectRefr};
use super::actor_process::{actor_process_manager_remove, process_level};
use super::base_extra_list::{BaseExtraEntry, ExtraData, ExtraDataList, new_extra_entry};
use super::data_handler::{DataHandler, DataHandlerLists};
use super::data_load::{DataLoadGlobals, DataLoadServices};
use super::default_forms::DefaultFormStatics;
use super::form::FormIdentity;
use super::form_components::{
    FormRef, TesActorBaseData, TesFullName, TesHealthForm, TesScriptableForm,
};
use super::form_hierarchy::RttiClass;
use super::forms_environment::WarningCounter;
use super::forms_init::{
    cast, casts_to, create_script_event_list, form_id, form_type, init_item_of, initialized,
    lookup, map_id, map_through_owner, name_of, warn,
};
use super::forms_items::{TesSubSpace, set_change_flags};
use super::forms_misc::set_initialized;
use super::forms_script::last_mod_reference;
use super::forms_world::{
    CellRegionLocks, PointerMap, TesObjectCell, TesWorldSpace, cell_grid_x, cell_grid_y,
};
use super::inventory::{
    InventoryGlobals, ensure_container_changes, reference_container, resolve_leveled_items,
    run_container_item_scripts, run_item_scripts, run_script,
};
use super::player::{PlayerCharacter, PlayerGlobals};
use super::tes::TesGlobals;
use super::tes::load_form_change;
use recomp_core::pointer_list::PointerList;

// IDA: the extra types 00426270's table 00426758 sends somewhere, by byte.
const EXTRA_WATER_TYPE: u8 = 0x05;
const EXTRA_CLIMATE: u8 = 0x0C;
const EXTRA_PACKAGE_START: u8 = 0x1E;
const EXTRA_OWNERSHIP: u8 = 0x27;
const EXTRA_GLOBAL: u8 = 0x28;
const EXTRA_LOCK: u8 = 0x31;
const EXTRA_TELEPORT: u8 = 0x32;
const EXTRA_ENABLE_PARENT: u8 = 0x3F;
const EXTRA_ENABLE_CHILDREN: u8 = 0x40;
const EXTRA_RANDOM_TELEPORT: u8 = 0x43;
const EXTRA_MERCHANT: u8 = 0x44;
const EXTRA_POISON: u8 = 0x48;
const EXTRA_NORTH_ROTATION: u8 = 0x4C;
const EXTRA_X_MARKER: u8 = 0x4D;
const EXTRA_HORSE: u8 = 0x58;
const EXTRA_RANK: u8 = 0x29;
const EXTRA_STARTING_POSITION: u8 = 0x14;

// IDA: g_saveLoadGame +0x18.
fn save_load_flags(world: &World) -> Result<u32, StreamError> {
    world
        .get_resource::<TesGlobals>()
        .and_then(|globals| globals.save_load_game.as_ref())
        .map(|game| game.fields_14[1])
        .ok_or(StreamError("0045A50A: g_saveLoadGame is null"))
}

// IDA: `_finite` 00983FFD and `_isnan` 00984012 of three floats, as 004DF87E and 0042B7FF test them.
fn all_finite(values: &[f32]) -> bool {
    values.iter().all(|value| value.is_finite())
}

// IDA: 004D74D0: a reference whose vt+0x170 base is a type 0x24 with +0x104 of 4.
fn is_horse(world: &World, reference: Entity) -> bool {
    world
        .get::<TesObjectRefr>(reference)
        .and_then(|reference| reference.base_form_1c)
        .filter(|&base| form_type(world, base) == 0x24)
        .and_then(|base| world.get::<super::forms_actors::TesCreature>(base))
        .is_some_and(|creature| creature.data_104[0] == 4)
}

// IDA: 00420260 the parent ExtraEnableStateParent holds: raw before its reference's InitItem.
fn enable_parent_slot(list: &ExtraDataList) -> Option<(u32, Option<Option<Entity>>)> {
    let entry = list.get(EXTRA_ENABLE_PARENT)?;
    match entry.data {
        ExtraData::EnableStateParent { parent_0c, .. } => Some((parent_0c, entry.resolved_0c)),
        _ => None,
    }
}

// IDA: 00424A70 ExtraEnableStateChildren's add on the parent: a new 0x40 entry when there's none (0042A600), then a prepend unless the list holds the child.
fn add_enable_child(world: &mut World, parent: Entity, child: Entity) {
    let mut refr = world.get_mut::<TesObjectRefr>(parent).expect("a reference");
    if refr.extra_data_44.get(EXTRA_ENABLE_CHILDREN).is_none() {
        refr.extra_data_44.add(new_extra_entry(
            EXTRA_ENABLE_CHILDREN,
            ExtraData::EnableStateChildren(Vec::new()),
        ));
    }
    let entry = refr
        .extra_data_44
        .get_mut(EXTRA_ENABLE_CHILDREN)
        .expect("just added");
    if let ExtraData::EnableStateChildren(children) = &mut entry.data
        && !children.contains(&child)
    {
        children.insert(0, child);
    }
}

// IDA: 004D8AF0 for a door: an interior parent answers the 004BC4A0 subspace at the position, else the cell. Otherwise 004D6670 (+0x40, else the persistent cell's extra), then 004C9CF0's world, null for an interior, and 004F0600's subspace, else the world. A door's base is never a subspace, so vt+0x170's type gate passes. Completion: 100%.
fn pathing_door_world(world: &World, door: Entity) -> Option<Entity> {
    let reference = world.get::<TesObjectRefr>(door)?;
    let position = reference.position_2c;
    if let Some(cell) = reference.parent_cell_40
        && super::forms_world::cell_is_interior(world, cell)
    {
        return super::forms_world::cell_subspace_at(world, cell, position).or(Some(cell));
    }
    let cell = reference
        .parent_cell_40
        .or_else(|| reference.extra_data_44.persistent_cell())?;
    if super::forms_world::cell_is_interior(world, cell) {
        return None;
    }
    let space = world.get::<TesObjectCell>(cell)?.world_50?;
    super::forms_world::world_subspace_at(world, space, position).or(Some(space))
}

// IDA: 006800BE..006800FD, past the type and prison gates: a linked door, 0067F5D0, then the node.
fn register_pathing_door(world: &mut World, door: Entity) {
    let Some(linked) = super::ai_conditions::teleport_door(world, door) else {
        return;
    };
    let door_world = pathing_door_world(world, door);
    let linked_world = pathing_door_world(world, linked);
    let mut nodes = world.get_resource_or_init::<super::pathing::PathingDoorNodes>();
    nodes.gated.push(door);
    if !nodes.has_door(door, door_world, linked_world) {
        nodes.insert(super::pathing::PathingDoorNode {
            door_04: door,
            world_08: door_world,
            linked_0c: linked,
            world_10: linked_world,
        });
        // 0067FD70's NiTMapUInt32UInt8_SetAt on 00B3BE00, for each new key.
        let count = nodes.world_keys().len() as u32;
        if let Some(mut pathing) = world.get_resource_mut::<super::pathing::PathingSingletons>()
            && let Some(map) = pathing.world_node_map_b3be00.as_mut()
        {
            map.count_0c = count;
        }
    }
}

// IDA: 0042B700 the ExtraTeleport block's InitItem(owner): word 0 maps and casts to TESObjectREFR, stored; a null warns a35cac and answers 0. A base that isn't a door warns a35c60, clears the slot and answers 0. A position or rotation with a non-finite float warns (a35c20, a35be4) and takes (0, 0, 0) from g_niPoint3Zero. Answers 1.
fn init_teleport(
    world: &mut World,
    entry: &mut BaseExtraEntry,
    owner: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<bool, StreamError> {
    let ExtraData::Teleport(block) = &mut entry.data else {
        return Err(StreamError(
            "0042B700: an extra 0x32 with no teleport block",
        ));
    };
    let mapped = map_through_owner(world, block[0], owner)?;
    let door = cast(world, lookup(world, mapped), RttiClass::TESObjectREFR);
    entry.resolved_0c = Some(door);
    let Some(door) = door else {
        warn(
            world,
            services,
            format!("Could not find linked door ({mapped:08X}) in teleport data init."),
        );
        return Ok(false);
    };
    let base = world
        .get::<TesObjectRefr>(door)
        .expect("a reference")
        .base_form_1c
        .ok_or(StreamError("0042B77F: a linked door with no base faults"))?;
    if form_type(world, base) != 0x18 {
        let type_name = super::tes_file::FORM_TYPE_TABLE
            .get(usize::from(form_type(world, base)))
            .map_or("", |(_, name)| *name);
        let text = format!(
            "Linked door ({mapped:08X}) in teleport data points to invalid object ({type_name} {}({:08X})).",
            name_of(world, base),
            form_id(world, base)
        );
        warn(world, services, text);
        entry.resolved_0c = Some(None);
        return Ok(false);
    }
    let floats = |block: &[u32; 7], from: usize| {
        [
            f32::from_bits(block[from]),
            f32::from_bits(block[from + 1]),
            f32::from_bits(block[from + 2]),
        ]
    };
    if !all_finite(&floats(block, 1)) {
        block[1..4].fill(0);
        warn(
            world,
            services,
            "Corrupt location found in teleport data, setting to (0, 0, 0).".to_owned(),
        );
    }
    if !all_finite(&floats(block, 4)) {
        block[4..7].fill(0);
        warn(
            world,
            services,
            "Corrupt angle found in teleport data, setting to (0, 0, 0).".to_owned(),
        );
    }
    Ok(true)
}

// IDA: an entry's word at +0x0C, for the arms that map it.
fn entry_word(entry: &BaseExtraEntry) -> u32 {
    match &entry.data {
        ExtraData::Dword(word) | ExtraData::Float(word) => *word,
        ExtraData::PackageStart { target_0c, .. } => *target_0c,
        ExtraData::EnableStateParent { parent_0c, .. } => *parent_0c,
        _ => 0,
    }
}

// IDA: one arm of 00426270's switch that maps +0x0C through the owner's last file, casts and stores; a null warns the text and the entry goes (00422E20).
fn resolve_entry(
    world: &World,
    entry: &mut BaseExtraEntry,
    file: Option<super::tes_file::TesFileId>,
    class: Option<RttiClass>,
) -> Result<(u32, Option<Entity>), StreamError> {
    let mapped = map_id(world, entry_word(entry), file)?;
    let found = lookup(world, mapped);
    let found = class.map_or(found, |class| cast(world, found, class));
    entry.resolved_0c = Some(found);
    Ok((mapped, found))
}

/// IDA: 00426270 ExtraDataList_InitItem(owner), by the jump table 00426720 over bytes 00426758 for types 5..=0x58. Each entry's next is read before its arm, and a failed arm removes the entry. Completion: 100%. The critical section 00B33800 around it is a host no-op.
pub fn init_extra_list(
    world: &mut World,
    owner: Entity,
    list: &mut ExtraDataList,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let file = last_mod_reference(world, owner);
    let mut entries = std::mem::take(&mut list.entries);
    let mut kept = Vec::with_capacity(entries.len());
    let mut removed = Vec::new();
    for mut entry in entries.drain(..) {
        let keep = match entry.type_04 {
            EXTRA_LOCK => {
                // 004262CD: a zero key is left; a miss on TESKey warns.
                let ExtraData::Lock(block) = &entry.data else {
                    return Err(StreamError("004262D0: an extra 0x31 with no lock block"));
                };
                let key = u32::from_le_bytes(block[4..8].try_into().expect("four bytes"));
                if key == 0 {
                    true
                } else {
                    let mapped = map_id(world, key, file)?;
                    let found = cast(world, lookup(world, mapped), RttiClass::TESKey);
                    entry.resolved_0c = Some(found);
                    if found.is_none() {
                        warn(
                            world,
                            services,
                            format!(
                                "Unable to find key {mapped:08X} for lock data. Lock will be removed."
                            ),
                        );
                    }
                    found.is_some()
                }
            }
            EXTRA_TELEPORT => init_teleport(world, &mut entry, owner, services)?,
            EXTRA_OWNERSHIP
            | EXTRA_GLOBAL
            | EXTRA_POISON
            | EXTRA_CLIMATE
            | EXTRA_WATER_TYPE
            | EXTRA_PACKAGE_START
            | EXTRA_RANDOM_TELEPORT
            | EXTRA_MERCHANT
            | EXTRA_X_MARKER => {
                let (class, what): (Option<RttiClass>, fn(u32) -> String) = match entry.type_04 {
                    EXTRA_OWNERSHIP => (None, |id| {
                        format!(
                            "Unable to find ownership owner form {id:08X}. Ownership will be removed."
                        )
                    }),
                    EXTRA_GLOBAL => (Some(RttiClass::TESGlobal), |id| {
                        format!(
                            "Unable to find ownership condition global {id:08X}. Ownership will be removed."
                        )
                    }),
                    EXTRA_POISON => (Some(RttiClass::AlchemyItem), |id| {
                        format!("Unable to find poison {id:08X}. Poison data will be removed.")
                    }),
                    EXTRA_CLIMATE => (Some(RttiClass::TESClimate), |id| {
                        format!(
                            "Unable to find cell climate {id:08X}. Climate data will be removed."
                        )
                    }),
                    EXTRA_WATER_TYPE => (Some(RttiClass::TESWaterForm), |id| {
                        format!(
                            "Unable to find cell water type {id:08X}. Water data will be removed."
                        )
                    }),
                    EXTRA_PACKAGE_START => (None, |id| {
                        format!(
                            "Unable to find package start location cell {id:08X}. Package start location extra data will be removed."
                        )
                    }),
                    EXTRA_RANDOM_TELEPORT => (Some(RttiClass::TESObjectREFR), |id| {
                        format!(
                            "Unable to find random door teleport marker {id:08X}. Random door teleport marker data will be removed."
                        )
                    }),
                    EXTRA_MERCHANT => (Some(RttiClass::TESObjectREFR), |id| {
                        format!(
                            "Unable to find merchant container {id:08X}. Merchant container data will be removed."
                        )
                    }),
                    _ => (Some(RttiClass::TESObjectREFR), |id| {
                        format!(
                            "Unable to find XMarker target {id:08X}. XMarker target data will be removed."
                        )
                    }),
                };
                let (mapped, found) = resolve_entry(world, &mut entry, file, class)?;
                if found.is_none() {
                    warn(world, services, what(mapped));
                }
                found.is_some()
            }
            EXTRA_ENABLE_PARENT => {
                let (mapped, found) =
                    resolve_entry(world, &mut entry, file, Some(RttiClass::TESObjectREFR))?;
                match found {
                    None => {
                        warn(
                            world,
                            services,
                            format!(
                                "Unable to find enable state parent {mapped:08X}. Enable state parent data will be removed."
                            ),
                        );
                        false
                    }
                    Some(parent) => {
                        // 0042654E: the owner's REFR cast, then 004E0E40 reads the owner's own list, which holds this entry as stored; the list is out of the component here, so it goes back for the call.
                        let child =
                            casts_to(world, owner, RttiClass::TESObjectREFR).then_some(owner);
                        let good =
                            child.is_some_and(|child| chain_ok_with_entry(world, child, parent));
                        if good {
                            add_enable_child(world, parent, child.expect("tested"));
                        } else {
                            warn(
                                world,
                                services,
                                "Enable state parent loop detected. Parent removed.".to_owned(),
                            );
                        }
                        good
                    }
                }
            }
            EXTRA_HORSE => {
                // 0042663F: the REFR cast; a reference that isn't a horse clears the slot.
                let (mapped, found) =
                    resolve_entry(world, &mut entry, file, Some(RttiClass::TESObjectREFR))?;
                let found = found.filter(|&horse| is_horse(world, horse));
                entry.resolved_0c = Some(found);
                if found.is_none() {
                    warn(
                        world,
                        services,
                        format!(
                            "Unable to find travel horse {mapped:08X}. Travel horse data will be removed."
                        ),
                    );
                }
                found.is_some()
            }
            _ => true,
        };
        if keep {
            kept.push(entry);
        } else {
            removed.push(entry.type_04);
        }
    }
    list.entries = kept;
    for extra_type in removed {
        // 00422E20 clears the type's presence bit, the entry being gone already.
        list.remove_entry(extra_type);
    }
    Ok(())
}

// IDA: 004E0E40 the parent chain has no loop, as 0042656D runs it: the entry 00426270 is at has just taken `parent`, so the chain starts there. The child itself answers 0; a form seen before ends the walk with 1. An initialized form's next is its resolved word; another's goes through 0046B250 and the REFR cast of its raw word.
fn chain_ok_with_entry(world: &World, child: Entity, parent: Entity) -> bool {
    if parent == child {
        return false;
    }
    let mut seen = vec![parent];
    let slot = world
        .get::<TesObjectRefr>(parent)
        .and_then(|reference| enable_parent_slot(&reference.extra_data_44));
    let mut key = if initialized(world, parent) {
        slot.and_then(|(_, form)| form.flatten())
    } else {
        slot.and_then(|(raw, _)| cast(world, lookup(world, raw), RttiClass::TESObjectREFR))
    };
    while let Some(current) = key {
        if current == child {
            return false;
        }
        if seen.contains(&current) {
            break;
        }
        seen.push(current);
        let slot = world
            .get::<TesObjectRefr>(current)
            .and_then(|reference| enable_parent_slot(&reference.extra_data_44));
        key = if initialized(world, current) {
            slot.and_then(|(_, form)| form.flatten())
        } else {
            slot.and_then(|(raw, _)| cast(world, lookup(world, raw), RttiClass::TESObjectREFR))
        };
    }
    true
}

// IDA: the shape of 00420340, the 0x3F entry's +0x10 bit 0.
fn parent_opposite(list: &ExtraDataList) -> bool {
    matches!(
        list.get(EXTRA_ENABLE_PARENT).map(|entry| &entry.data),
        Some(ExtraData::EnableStateParent { flags_10, .. }) if flags_10 & 1 != 0
    )
}

// IDA: 0046ABA0(set): +0x08 bit 11, then vt+0x40(0x40000001).
fn set_disabled(world: &mut World, form: Entity, set: bool) -> Result<(), StreamError> {
    {
        let mut identity = world.get_mut::<FormIdentity>(form).expect("a form");
        if set {
            identity.flags_08 |= 0x800;
        } else {
            identity.flags_08 &= !0x800;
        }
    }
    set_change_flags(world, form, 0x4000_0001)
}

// IDA: 004DBF90: each 0x40 child follows this reference's bit 11, inverted by its own 00420340; one that differs takes it and recurses. A null item ends nothing here: the list holds none.
fn sync_enable_children(world: &mut World, reference: Entity) -> Result<(), StreamError> {
    let children = match world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44
        .get(EXTRA_ENABLE_CHILDREN)
        .map(|entry| &entry.data)
    {
        Some(ExtraData::EnableStateChildren(children)) => children.clone(),
        _ => return Ok(()),
    };
    for child in children {
        let mut state = world
            .get::<FormIdentity>(reference)
            .expect("a form")
            .flags_08
            & 0x800
            != 0;
        if parent_opposite(
            &world
                .get::<TesObjectRefr>(child)
                .expect("a reference")
                .extra_data_44,
        ) {
            state = !state;
        }
        let current = world.get::<FormIdentity>(child).expect("a form").flags_08 & 0x800 != 0;
        if current != state {
            set_disabled(world, child, state)?;
            sync_enable_children(world, child)?;
        }
    }
    Ok(())
}

// IDA: vt+0xF8 004DD1A0(g_niPoint3Zero) through 0041F7B0 and 0041F770: the 0x14 entry (a new 0042AC40 with no entry) takes vt+0x174's position and the rotation.
fn set_starting_position(world: &mut World, reference: Entity) {
    let mut refr = world
        .get_mut::<TesObjectRefr>(reference)
        .expect("a reference");
    let position = refr.position_2c.map(f32::to_bits);
    let rotation = refr.rotation_20.map(f32::to_bits);
    let data = ExtraData::StartingPosition {
        position_0c: position,
        rotation_18: rotation,
    };
    if let Some(entry) = refr.extra_data_44.get_mut(EXTRA_STARTING_POSITION) {
        entry.data = data;
    } else {
        refr.extra_data_44
            .add(new_extra_entry(EXTRA_STARTING_POSITION, data));
    }
}

// IDA: 005E6680 Actor_SetDeathState(state), for the arms a load takes. With 00B14E98 set, an essential base (+0x28 bit 1) turns 1 or 2 into 6. State 2: a process's vt+0xE8 is null on Low and MiddleLow; 0x1A extra data would run 004876C0: a host stop. A nonzero state on a level-0 process goes to 00628630: a host stop. A change marks vt+0x40(0x40); +0xB0 takes it.
pub(crate) fn set_death_state(
    world: &mut World,
    actor: Entity,
    state: u32,
) -> Result<(), StreamError> {
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
            .get(super::base_extra_list::EXTRA_CONTAINER_CHANGES)
            .is_some()
    {
        return Err(StreamError("005E670C: 004876C0 with container changes"));
    }
    if state != 0 && process.is_some_and(|process| process_level(world, process) == 0) {
        return Err(StreamError("005E6753: 00628630 on a high process at death"));
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

// IDA: 004D7DD0(0), past the actor test: not ID 7, and vt+0x170's TESHealthForm answers 0 (`fild`, with the unsigned fix, `fcomp` 0.0: at or below). A base with no health form answers 0, so it counts.
fn base_has_no_health(world: &World, reference: Entity) -> bool {
    if form_id(world, reference) == 7 {
        return false;
    }
    let base = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c;
    base.and_then(|base| world.get::<TesHealthForm>(base))
        .is_none_or(|health| health.health_04 == 0)
}

// IDA: 005EA790 the actor's starting data: +0xE8.. the position words, +0xF4 vt+0x1E0's rotation, +0xF8 the cell when 006ECC80's is interior, otherwise 004D6670: +0x40, or vt+0 of +0x18's persistent cell, through 004C9CF0, the exterior cell's world.
fn set_actor_starting_data(world: &mut World, actor: Entity) -> Result<(), StreamError> {
    let refr = world.get::<TesObjectRefr>(actor).expect("an actor");
    let position = refr.position_2c;
    let rotation = refr.rotation_20[2];
    let parent = refr.parent_cell_40;
    let cell_of = match parent {
        Some(_) => parent,
        None => super::forms_references::reference_cell(world, actor)?,
    };
    let interior = parent.and_then(|cell| {
        world
            .get::<TesObjectCell>(cell)
            .filter(|body| body.flags_24 & 1 != 0)
            .map(|_| cell)
    });
    let location = interior.or_else(|| {
        // 004D6670: +0x40, or with a null one vt+0 004DDCB0, which also answers null unless 004DB4A0 or flag 0x4000 (the extra alone isn't enough).
        cell_of
            .and_then(|cell| world.get::<TesObjectCell>(cell))
            .filter(|body| body.flags_24 & 1 == 0)
            .and_then(|body| body.world_50)
    });
    let mut data = world.get_mut::<Actor>(actor).expect("an actor");
    data.starting_position_e8 = position.map(f32::to_bits);
    data.starting_z_rotation_f4 = rotation;
    data.starting_location_f8 = location;
    Ok(())
}

// IDA: 0065FD20 on g_thePlayer: a base that is 00B35EBC with teleport data (004D7630), in no cell or an exterior one or an interior with +0x24 bit 7, joins the player's +0x704 list once by a prepend.
fn add_jail_marker(world: &mut World, reference: Entity) -> Result<(), StreamError> {
    let refr = world.get::<TesObjectRefr>(reference).expect("a reference");
    if refr.extra_data_44.get(EXTRA_TELEPORT).is_none() {
        return Ok(());
    }
    if let Some(cell) = refr.parent_cell_40 {
        let flags = world.get::<TesObjectCell>(cell).expect("a cell").flags_24;
        if flags & 1 != 0 && flags & 0x80 == 0 {
            return Ok(());
        }
    }
    let player = world
        .resource::<PlayerGlobals>()
        .the_player_b333c4
        .ok_or(StreamError("0065FD7A: a null g_thePlayer faults"))?;
    let mut body = world
        .get_mut::<PlayerCharacter>(player)
        .ok_or(StreamError("0065FD7A: g_thePlayer is no PlayerCharacter"))?;
    if !body.jail_markers_704.items().any(|item| item == reference) {
        body.jail_markers_704.prepend_non_null(reference);
    }
    Ok(())
}

/// IDA: 004D70E0 TESObjectREFR_InitializeScriptAnd3DState: a deleted reference (+0x08 bit 5) returns, and so does one whose 0x12 entry holds a script. Otherwise vt+0x170's TESScriptableForm script goes in through 0041F090 with 004FBDC0's event list (0041F130). With a base container, 00485E00 and 0048E060. Completion: 100%.
pub fn init_reference_script(world: &mut World, reference: Entity) -> Result<(), StreamError> {
    if world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08
        & 0x20
        != 0
    {
        return Ok(());
    }
    let refr = world.get::<TesObjectRefr>(reference).expect("a reference");
    let holds_script = matches!(
        refr.extra_data_44.get(0x12).map(|entry| &entry.data),
        Some(ExtraData::Script { .. })
    );
    if holds_script {
        return Ok(());
    }
    // 004D710A vt+0x128: 006F7070 answers null for a reference, 00447550 an actor's +0xD0, the leveled creature's base, which no load sets.
    if world
        .get::<Actor>(reference)
        .is_some_and(|actor| actor.leveled_creature_template_d0 != 0)
    {
        return Err(StreamError("004D7112: an actor with a +0xD0 template"));
    }
    let refr = world.get::<TesObjectRefr>(reference).expect("a reference");
    let base = refr.base_form_1c;
    let script = base
        .and_then(|base| world.get::<TesScriptableForm>(base))
        .and_then(|part| match part.script_04 {
            FormRef::Resolved(script) => script,
            FormRef::Raw(_) => None,
        });
    if let Some(script) = script {
        let events = create_script_event_list(world, script);
        world
            .get_mut::<TesObjectRefr>(reference)
            .expect("a reference")
            .extra_data_44
            .add(new_extra_entry(
                0x12,
                ExtraData::Script {
                    script_0c: script,
                    events_10: Some(events),
                },
            ));
    }
    if reference_container(world, reference).is_some() {
        let mut changes = ensure_container_changes(world, reference)?;
        let result = run_item_scripts(world, reference, &mut changes);
        super::inventory::put_container_changes(world, reference, changes);
        result?;
    }
    Ok(())
}

/// IDA: 004DF820 TESObjectREFR vt+0x6C, and ACHR's and ACRE's. Completion: 100% of the arms a load takes; `set_death_state` and the inventory stop elsewhere.
pub fn init_reference(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if initialized(world, form) {
        return Ok(());
    }
    // 004DF83D: g_disableWarningMessages and 00B34D88 saved, 1 and 0.
    let saved_flag = world
        .resource::<DataLoadGlobals>()
        .disable_warning_messages_b06b18;
    let saved_count = world.get_resource_or_init::<WarningCounter>().count_b34d88;
    world
        .resource_mut::<DataLoadGlobals>()
        .disable_warning_messages_b06b18 = 1;
    world.resource_mut::<WarningCounter>().count_b34d88 = 0;
    let base = world
        .get::<TesObjectRefr>(form)
        .expect("a reference")
        .base_form_1c;
    // 004DF865: a deleted base deletes the reference: vt+0x8C(1).
    if let Some(base) = base
        && world.get::<FormIdentity>(base).expect("a form").flags_08 & 0x20 != 0
    {
        if casts_to(world, form, RttiClass::Actor) {
            return Err(StreamError("0065AA60: vt+0x8C(1) on an actor at InitItem"));
        }
        world
            .get_mut::<FormIdentity>(form)
            .expect("a form")
            .flags_08 |= 0x20;
        set_change_flags(world, form, 0x4000_0001)?;
    }
    // 004DF87E..004DF9C5.
    let refr = world.get::<TesObjectRefr>(form).expect("a reference");
    let (position, rotation) = (refr.position_2c, refr.rotation_20);
    if !all_finite(&position) {
        warn(
            world,
            services,
            "Corrupt location found on reference, setting to (0, 0, 0).".to_owned(),
        );
        world
            .get_mut::<TesObjectRefr>(form)
            .expect("a reference")
            .position_2c = [0.0; 3];
    }
    if !all_finite(&rotation) {
        warn(
            world,
            services,
            "Corrupt angle found on reference, setting to (0, 0, 0).".to_owned(),
        );
        world
            .get_mut::<TesObjectRefr>(form)
            .expect("a reference")
            .rotation_20 = [0.0; 3];
    }
    // 004DF9CB 00426270.
    let mut list = std::mem::take(
        &mut world
            .get_mut::<TesObjectRefr>(form)
            .expect("a reference")
            .extra_data_44,
    );
    let result = init_extra_list(world, form, &mut list, services);
    world
        .get_mut::<TesObjectRefr>(form)
        .expect("a reference")
        .extra_data_44 = list;
    result?;
    // 004DF9D3..004DFA19.
    if let Some(base) = base
        && form_type(world, base) == 0x18
    {
        let prison = world
            .get_resource::<DefaultFormStatics>()
            .and_then(|statics| statics.prison_marker_b35ebc)
            == Some(base);
        // 00680090, a recorded boundary. The count applies three of its gates (00B3BE00 set, not the prison marker, a teleport block). 0042B410's linked door, 0067F5D0 and 00680A00 aren't evaluated, so it's an upper bound on the doors the node map takes.
        let singleton = world
            .get_resource::<super::pathing::PathingSingletons>()
            .is_some_and(|singletons| singletons.world_node_map_b3be00.is_some());
        let has_teleport = world
            .get::<TesObjectRefr>(form)
            .expect("a reference")
            .extra_data_44
            .get(EXTRA_TELEPORT)
            .is_some();
        if singleton && !prison && has_teleport {
            world
                .get_resource_or_init::<InventoryGlobals>()
                .pathing_doors_not_registered += 1;
            register_pathing_door(world, form);
        }
        if prison {
            add_jail_marker(world, form)?;
        }
    }
    let is_actor = casts_to(world, form, RttiClass::Actor);
    if is_actor {
        let player = world
            .get_resource::<PlayerGlobals>()
            .and_then(|globals| globals.the_player_b333c4);
        // 004DFA30 005E0260, vt+0x37C 005F0260: the player skips.
        if world
            .get::<Actor>(form)
            .expect("an actor")
            .starting_location_f8
            .is_none()
            && player != Some(form)
        {
            set_actor_starting_data(world, form)?;
        }
        // 004DFA47 005E04E0, 004DFA54 004D7DD0(0).
        if player != Some(form) && base_has_no_health(world, form) {
            set_death_state(world, form, 2)?;
            actor_process_manager_remove(world, form, 3, false);
        }
    } else {
        // 004DFA75: types 0x1C and 0x1E..=0x20 keep no starting position. A null base faults.
        let base = base.ok_or(StreamError("004DFA7F: a reference with no base faults"))?;
        let base_type = form_type(world, base);
        if base_type != 0x1C && !(0x1E..=0x20).contains(&base_type) {
            set_starting_position(world, form);
        }
    }
    // 004DFABD..004DFB44.
    let flags = save_load_flags(world)?;
    if flags & 0x800 == 0 || flags & 0x40 != 0 {
        let refr = world.get::<TesObjectRefr>(form).expect("a reference");
        let parent = enable_parent_slot(&refr.extra_data_44).and_then(|(_, found)| found.flatten());
        let opposite = parent_opposite(&refr.extra_data_44);
        if let Some(parent) = parent {
            // 004DFB03..004DFB44 0045A530(0) around the call, which restores the bit: the active bit stays clear at startup, or the save-load arm: a host stop.
            // 004DFAE9..004DFB44: 0045A500 saved; without +0x18 bit 6, 0045A530(0) around 0046ABA0, which 004F1630's and 004D4310's toggle reaches.
            let active = flags & 1 != 0;
            if flags & 0x40 == 0 {
                super::data_handler_world::set_save_load_thread_active(world, false)?;
            }
            let mut state =
                world.get::<FormIdentity>(parent).expect("a form").flags_08 & 0x800 != 0;
            if opposite {
                state = !state;
            }
            set_disabled(world, form, state)?;
            super::data_handler_world::set_save_load_thread_active(world, active)?;
        }
    }
    // 004DFB49 004DBF90, 004DFB54 the flag.
    sync_enable_children(world, form)?;
    set_initialized(world, form, true);
    // 004DFB59..004DFB6A: the count can't fire (`WarningCounter`), so the three arms that report it can't.
    let count = world.resource::<WarningCounter>().count_b34d88;
    world
        .resource_mut::<DataLoadGlobals>()
        .disable_warning_messages_b06b18 = saved_flag;
    world.resource_mut::<WarningCounter>().count_b34d88 = saved_count;
    if count != 0 {
        return Err(StreamError(
            "004DFB76: 00B34D88 counted a warning, which 004A7A60 never does",
        ));
    }
    init_reference_script(world, form)?;
    // 004DFCC0..004DFCF5.
    if reference_container(world, form).is_some() {
        let mut changes = ensure_container_changes(world, form)?;
        let result = resolve_leveled_items(world, form, &mut changes, services)
            .and_then(|()| run_item_scripts(world, form, &mut changes));
        let empty = changes.entries_00.is_empty();
        super::inventory::put_container_changes(world, form, changes);
        result?;
        if empty {
            // 0041F5F0.
            world
                .get_mut::<TesObjectRefr>(form)
                .expect("a reference")
                .extra_data_44
                .remove_by_type(super::base_extra_list::EXTRA_CONTAINER_CHANGES);
        }
    }
    // 004DFCFA.
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .flags_08 &= 0xFFDF_FFFF;
    Ok(())
}

// IDA: 00420C60 ExtraNorthRotation (0x4C): `fcomp` 0.0 with `jnp` on `test ah, 0x44`: equal removes (0041E160); otherwise set, or a new entry.
fn set_north_rotation(list: &mut ExtraDataList, value: f32) {
    if value == 0.0 {
        list.remove_by_type(EXTRA_NORTH_ROTATION);
        return;
    }
    if let Some(entry) = list.get_mut(EXTRA_NORTH_ROTATION) {
        entry.data = ExtraData::Float(value.to_bits());
        return;
    }
    list.add(new_extra_entry(
        EXTRA_NORTH_ROTATION,
        ExtraData::Float(value.to_bits()),
    ));
}

// IDA: 00464910 TESSaveLoadGame_ReadCellChanges(cell): 0045A500 was tested by the caller and again here. 004637C0 on the cell, whose al is the answer, which 004CDA22 drops. An interior looks its ID up in +0x08, an exterior its world's in +0x0C (a null world faults); a miss answers. A hit walks the list with 00463D70, a host stop: the maps are empty with no save. Completion: 100% of the arms with no save.
// TODO: [ ] 00464977, 004649E8 the two maps' hits, with save loading.
fn load_cell_change(world: &mut World, cell: Entity) -> Result<(), StreamError> {
    load_form_change(world, cell)?;
    let game = world
        .resource::<TesGlobals>()
        .save_load_game
        .as_ref()
        .ok_or(StreamError("00464919: a null g_saveLoadGame"))?;
    let data = world.get::<TesObjectCell>(cell).expect("a cell");
    let found = if data.flags_24 & 1 != 0 {
        game.interior_references_08.count_0c != 0
    } else {
        if data.world_50.is_none() {
            return Err(StreamError("004649CF: 004C9CF0 gives a null world"));
        }
        game.exterior_references_0c.count_0c != 0
    };
    if found {
        return Err(StreamError(
            "00464971: a saved cell's new references (00463D70)",
        ));
    }
    Ok(())
}

/// IDA: 004CD960 TESObjectCELL vt+0x6C. Each object's vt+0x6C unless flagged, then 004637C0 (`load_form_change`); an interior's north marker (00B35EB8) sets the rotation through 00420C60; an object that left the cell (a temporary cell keeps them) makes the walk re-read the node. 0045A500's arm is `load_cell_change`, then 0045FDA0 with g_saveLoadGame +0x1C, a host stop. Unless the flag is set, with g_disableWarningMessages set and 00B34D88 cleared around it: 00426270; an exterior's ownership warns a46888 and goes (0x27, 0x29, 0x28), and its name is freed; the flag. Completion: 100%, with the 00B34D88 reports unreachable.
pub fn init_cell(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let north = world
        .get_resource::<DefaultFormStatics>()
        .and_then(|statics| statics.north_marker_b35eb8);
    let temporary = world.get::<FormIdentity>(form).expect("a form").flags_08 & 0x400 != 0;
    let mut objects: Vec<Entity> = world
        .get::<TesObjectCell>(form)
        .expect("a cell")
        .objects_48
        .items()
        .collect();
    let mut index = 0;
    while index < objects.len() {
        let object = objects[index];
        if !initialized(world, object) {
            init_reference(world, object, services)?;
            // 004CD99A 004637C0(object), inside the same arm.
            load_form_change(world, object)?;
        }
        let cell_flags = world.get::<TesObjectCell>(form).expect("a cell").flags_24;
        let refr = world.get::<TesObjectRefr>(object).expect("a reference");
        if cell_flags & 1 != 0 && refr.base_form_1c == north {
            // 004CD9C0 `fmul qword` -1.0, stored as a float.
            #[expect(
                clippy::neg_multiply,
                reason = "004CD9C4 is `fmul` by the qword -1.0 at A3D360, not `fchs`"
            )]
            let value = (f64::from(refr.rotation_20[2]) * -1.0) as f32;
            let mut cell = world.get_mut::<TesObjectCell>(form).expect("a cell");
            set_north_rotation(&mut cell.extra_28, value);
        }
        let parent = world
            .get::<TesObjectRefr>(object)
            .expect("a reference")
            .parent_cell_40;
        if temporary || parent == Some(form) {
            index += 1;
        } else {
            objects = world
                .get::<TesObjectCell>(form)
                .expect("a cell")
                .objects_48
                .items()
                .collect();
        }
    }
    // 004CDA12 0045A500: the arm 004471D0, 004F1630 and 004D4310 open with their 0045A530 toggle.
    if save_load_flags(world)? & 1 != 0 {
        load_cell_change(world, form)?;
        let flags = save_load_flags(world)?;
        let field_1c = world
            .resource::<TesGlobals>()
            .save_load_game
            .as_ref()
            .map_or(0, |game| game.fields_14[2]);
        if flags & 0x100 == 0 && field_1c != 0 {
            return Err(StreamError("004CDA59: 0045FDA0 with g_saveLoadGame +0x1C"));
        }
    }
    if initialized(world, form) {
        return Ok(());
    }
    let saved_flag = world
        .resource::<DataLoadGlobals>()
        .disable_warning_messages_b06b18;
    let saved_count = world.get_resource_or_init::<WarningCounter>().count_b34d88;
    world
        .resource_mut::<DataLoadGlobals>()
        .disable_warning_messages_b06b18 = 1;
    world.resource_mut::<WarningCounter>().count_b34d88 = 0;
    let mut list = std::mem::take(
        &mut world
            .get_mut::<TesObjectCell>(form)
            .expect("a cell")
            .extra_28,
    );
    let result = init_extra_list(world, form, &mut list, services);
    world
        .get_mut::<TesObjectCell>(form)
        .expect("a cell")
        .extra_28 = list;
    result?;
    let interior = world.get::<TesObjectCell>(form).expect("a cell").flags_24 & 1 != 0;
    if !interior {
        let owned = world
            .get::<TesObjectCell>(form)
            .expect("a cell")
            .extra_28
            .get(EXTRA_OWNERSHIP)
            .is_some();
        if owned {
            // 004CDAD5..004CDB72.
            let cell_world = world.get::<TesObjectCell>(form).expect("a cell").world_50;
            let (world_id, world_name) = cell_world.map_or((0, "NONE".to_owned()), |found| {
                (form_id(world, found), name_of(world, found))
            });
            let text = format!(
                "Removing ownership data on exterior cell '{}' ({:08X}) at ( {}, {} ) in worldspace '{world_name}' ({world_id:08X}).",
                name_of(world, form),
                form_id(world, form),
                cell_grid_x(world, form),
                cell_grid_y(world, form),
            );
            warn(world, services, text);
            let mut cell = world.get_mut::<TesObjectCell>(form).expect("a cell");
            // 00423660(0), 004237E0(-1), 00423720(0): each removes only what 0041E210 finds, so a clear bit hides an entry and a set bit with no entry stays set.
            cell.extra_28.remove_if_found(EXTRA_OWNERSHIP);
            cell.extra_28.remove_if_found(EXTRA_RANK);
            cell.extra_28.remove_if_found(EXTRA_GLOBAL);
        }
        // 004CDB7D..004CDB92 the name freed.
        world.get_mut::<TesFullName>(form).expect("a cell").name_04 =
            super::tes_file::BsStringT::default();
    }
    set_initialized(world, form, true);
    let count = world.resource::<WarningCounter>().count_b34d88;
    world
        .resource_mut::<DataLoadGlobals>()
        .disable_warning_messages_b06b18 = saved_flag;
    world.resource_mut::<WarningCounter>().count_b34d88 = saved_count;
    if count != 0 {
        return Err(StreamError(
            "004CDBBD: 00B34D88 counted a warning, which 004A7A60 never does",
        ));
    }
    Ok(())
}

// IDA: one of 004F2C10's slots: nonzero maps and casts; a null warns by 00404E50's test; the slot takes the result.
fn init_world_slot(
    world: &mut World,
    form: Entity,
    slot: FormRef,
    class: RttiClass,
    what: &str,
    services: &mut DataLoadServices<'_>,
) -> Result<FormRef, StreamError> {
    let FormRef::Raw(raw) = slot else {
        return Err(StreamError("004F2C4D: a slot resolved twice"));
    };
    if raw == 0 {
        return Ok(slot);
    }
    let mapped = map_through_owner(world, raw, form)?;
    let found = cast(world, lookup(world, mapped), class);
    if found.is_none() {
        let name = name_of(world, form);
        let text = if name.is_empty() {
            format!(
                "Unable to find {what} ({mapped:08X}) on owner worldspace ({:08X}).",
                form_id(world, form)
            )
        } else {
            format!("Unable to find {what} ({mapped:08X}) on owner worldspace \"{name}\".")
        };
        warn(world, services, text);
    }
    Ok(FormRef::Resolved(found))
}

/// IDA: 004F2C10 TESWorldSpace vt+0x6C. Unless flagged: +0x58 (TESClimate), +0x80 (TESWaterForm), +0x7C (TESWorldSpace), the flag. Always: vt+0x6C of each cell of the +0x30 map, in bucket order; of +0x34; then the OFST arms; then 004F28B0. Completion: 100% but the rebuild arm.
/// TODO: [ ] 004F2EAA 00451920 with +0xA8 null in a master, which rebuilds OFST from the file (no shipped world reaches it).
/// TODO: [ ] 004F30C5 the 00B09DB8 arm, clear in the image, which loads every cell.
pub fn init_world_space(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    if !initialized(world, form) {
        let body = world.get::<TesWorldSpace>(form).expect("a world");
        let (climate, water, parent) = (body.climate_58, body.water_80, body.parent_7c);
        let found = init_world_slot(
            world,
            form,
            climate,
            RttiClass::TESClimate,
            "climate",
            services,
        )?;
        world
            .get_mut::<TesWorldSpace>(form)
            .expect("a world")
            .climate_58 = found;
        let found = init_world_slot(
            world,
            form,
            water,
            RttiClass::TESWaterForm,
            "water type",
            services,
        )?;
        world
            .get_mut::<TesWorldSpace>(form)
            .expect("a world")
            .water_80 = found;
        let found = init_world_slot(
            world,
            form,
            parent,
            RttiClass::TESWorldSpace,
            "landscape world",
            services,
        )?;
        world
            .get_mut::<TesWorldSpace>(form)
            .expect("a world")
            .parent_7c = found;
        set_initialized(world, form, true);
    }
    let body = world.get::<TesWorldSpace>(form).expect("a world");
    let cells: Vec<Entity> = body
        .cells_30
        .buckets_08
        .iter()
        .flatten()
        .map(|&(_, cell)| cell)
        .collect();
    let persistent = body.persistent_cell_34;
    let has_offsets = body.offsets_a8.is_some();
    for cell in cells {
        init_cell(world, cell, services)?;
    }
    if let Some(cell) = persistent {
        init_cell(world, cell, services)?;
    }
    if !has_offsets {
        // 004F2E7A 0046B680(0), 0044FAA0: the first file, and whether it's a master.
        let first = world
            .get::<super::form::ModReferences>(form)
            .and_then(|references| references.0.first().copied())
            .and_then(|index| {
                world
                    .resource::<DataHandler>()
                    .mods_by_id_8d4
                    .get(usize::from(index))
                    .copied()
                    .flatten()
            });
        if let Some(first) = first
            && world
                .resource::<DataHandler>()
                .file_store
                .get(first)
                .is_some_and(|file| file.is_master())
        {
            return Err(StreamError(
                "004F2EAA: 00451920, a master's world with no OFST",
            ));
        }
    }
    read_distant_lod(world, form, services)
}

// IDA: 004F28B0 the DistantLOD read: +0xD8 = 0; "Data\DistantLOD\%s.cmp" with vt+0xD4; 00431690(path, 0, 0x800); none frees the path and returns. 007C2A40 clears +0xC8. vt+0x1C the size, >> 2, less 1: a smaller file's count wraps, a host stop. With +0x24 (the file opened), each word: its low and high halves are the key, the same 32 bits, 004D6760 finds it or 00452570(key, 1) adds it; +0xD8 = 1; the last word goes in +0xDC. vt+0(1) closes it.
fn read_distant_lod(
    world: &mut World,
    form: Entity,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    world
        .get_mut::<TesWorldSpace>(form)
        .expect("a world")
        .lod_loaded_d8 = 0;
    let name = name_of(world, form);
    let path = format!("Data\\DistantLOD\\{name}.cmp");
    let Some(bytes) = services.host.read_data_file(&path) else {
        return Ok(());
    };
    if bytes.len() < 4 {
        return Err(StreamError("004F2951: a DistantLOD file under four bytes"));
    }
    let words = bytes.len() / 4;
    let word =
        |at: usize| u32::from_le_bytes(bytes[4 * at..4 * at + 4].try_into().expect("four bytes"));
    let mut body = world.get_mut::<TesWorldSpace>(form).expect("a world");
    body.map_c8 = PointerMap::new(body.map_c8.original_vtable, body.map_c8.buckets_04);
    for at in 0..words - 1 {
        let key = word(at);
        if body.map_c8.get(key).is_none() {
            body.map_c8.set_at(key, 1);
        }
    }
    body.lod_loaded_d8 = 1;
    body.field_dc = word(words - 1);
    Ok(())
}

/// IDA: 004EF290 per worldspace -> 004CB980(world) on +0x34: each object of the persistent cell whose vt+0x170 base is type 0x29 goes to 004F1AC0. 00496EA0 and 00496F50 are the cell's lock, a host no-op here. Completion: 100%.
pub fn world_subspace_pass(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let Some(cell) = world
        .get::<TesWorldSpace>(form)
        .expect("a world")
        .persistent_cell_34
    else {
        return Ok(());
    };
    let objects: Vec<Entity> = world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .items()
        .collect();
    for object in objects {
        let base = world
            .get::<TesObjectRefr>(object)
            .expect("a reference")
            .base_form_1c
            .ok_or(StreamError(
                "004CB9B2: a persistent object with no base faults",
            ))?;
        if form_type(world, base) == 0x29 {
            add_subspace_reference(world, form, object, base);
        }
    }
    Ok(())
}

// IDA: 004F1AC0: a null +0x60 gets 004F0DE0(0x25). The extent is 004A9730's radius times vt+0xEC's scale, stored as a float at 24 bits. Each edge is position -/+ extent, stored as a float, `fistp`, `sar 12`. Over x, then y: the key is x << 16 | y & 0xFFFF; a missing list is added, then the reference prepends.
fn add_subspace_reference(world: &mut World, form: Entity, reference: Entity, base: Entity) {
    let radius = world
        .get::<TesSubSpace>(base)
        .expect("a subspace")
        .radius_2c;
    let refr = world.get::<TesObjectRefr>(reference).expect("a reference");
    let extent = radius * refr.scale_38;
    let [x, y, _] = refr.position_2c;
    let x_min = recomp_core::x87::fistp(x - extent) >> 12;
    let y_min = recomp_core::x87::fistp(y - extent) >> 12;
    let x_max = recomp_core::x87::fistp(x + extent) >> 12;
    let y_max = recomp_core::x87::fistp(y + extent) >> 12;
    let mut body = world.get_mut::<TesWorldSpace>(form).expect("a world");
    let map = body
        .subspaces_60
        .get_or_insert_with(|| PointerMap::new(0x00A4_8228, 0x25));
    for grid_x in x_min..=x_max {
        for grid_y in y_min..=y_max {
            let key = ((grid_x as u16 as u32) << 16) | u32::from(grid_y as u16);
            if map.get(key).is_none() {
                map.set_at(key, PointerList::default());
            }
            map.get_mut(key)
                .expect("just added")
                .prepend_non_null(reference);
        }
    }
}

// ---- Phase 13d: the passes of 00447D80 ----

/// IDA: 004D7190 TESObjectREFR_RunScripts. A deleted reference (+0x08 bit 5) returns 0. vt+0x170's type (a null base faults) 0x17, 0x23 or 0x24: 004D6D40; with a container, 00485E00, then 0048E740, whose nonzero answer sets the result. Then 0041E900 on +0x44: with a script, 0041E920's events, 004FBE00(script, this, events, 0, 0); a nonzero answer returns 1 at once. Otherwise an event list runs vt+0x40(0x4000000). Completion: 100% of the control flow.
/// # Errors
/// A null base, or a stop below.
pub fn reference_run_scripts(world: &mut World, reference: Entity) -> Result<bool, StreamError> {
    let flags = world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08;
    if flags & 0x20 != 0 {
        return Ok(false);
    }
    let base = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
        .ok_or(StreamError("004D71AF: a reference with no base faults"))?;
    let mut answered = false;
    if matches!(form_type(world, base), 0x17 | 0x23 | 0x24)
        && reference_container(world, reference).is_some()
    {
        let mut changes = ensure_container_changes(world, reference)?;
        let result = run_container_item_scripts(world, &mut changes, Some(reference));
        super::inventory::put_container_changes(world, reference, changes);
        answered = result?;
    }
    let script = match world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44
        .get(0x12)
        .map(|entry| &entry.data)
    {
        Some(ExtraData::Script {
            script_0c,
            events_10,
        }) => Some((*script_0c, events_10.is_some())),
        _ => None,
    };
    if let Some((script, events)) = script {
        world
            .resource_mut::<InventoryGlobals>()
            .reference_script_runs += 1;
        // The list comes out of the extra for the run.
        let mut list = world
            .get_mut::<TesObjectRefr>(reference)
            .expect("a reference")
            .into_inner()
            .extra_data_44
            .get_mut(0x12)
            .and_then(|entry| match &mut entry.data {
                ExtraData::Script { events_10, .. } => events_10.take(),
                _ => None,
            });
        let result = run_script(
            world,
            script,
            Some(reference),
            list.as_mut(),
            Some(reference),
        );
        if let Some(entry) = world
            .get_mut::<TesObjectRefr>(reference)
            .expect("a reference")
            .into_inner()
            .extra_data_44
            .get_mut(0x12)
            && let ExtraData::Script { events_10, .. } = &mut entry.data
        {
            *events_10 = list;
        }
        if result? {
            return Ok(true);
        }
        if events {
            set_change_flags(world, reference, 0x400_0000)?;
        }
    }
    Ok(answered)
}

/// IDA: 004CB8C0(cell, all, skipActors), with 00B35C80's 00496EA0 and 00496F50 around it. The player's parent cell (006ECC80) is read first; each node's next is read before its item runs; a null item goes to the next node. With a2 = 0, vt+0x154 (3D) or +0x08 bit 11 decides, which stops. An actor with a3 skips. 004D7190 answering nonzero with a2 = 0, or the player's cell having changed, ends the walk with 1. Completion: 100% of the a2 = 1 path.
/// TODO: [ ] 004CB900 the a2 = 0 arm (vt+0x154).
/// # Errors
/// A lock another thread holds, or a stop below.
pub fn cell_run_reference_scripts(
    world: &mut World,
    cell: Entity,
    all: bool,
    skip_actors: bool,
) -> Result<bool, StreamError> {
    world
        .get_resource_or_init::<CellRegionLocks>()
        .acquire(cell)?;
    let player = world
        .get_resource::<PlayerGlobals>()
        .and_then(|globals| globals.the_player_b333c4)
        .ok_or(StreamError("004CB8DD: a null g_thePlayer"))?;
    let parent = |world: &World| {
        world
            .get::<TesObjectRefr>(player)
            .expect("the player")
            .parent_cell_40
    };
    let player_cell = parent(world);
    let references: Vec<Entity> = world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .objects_48
        .items()
        .collect();
    let mut stopped = false;
    for reference in references {
        if !all {
            return Err(StreamError("004CB900: the a2 = 0 arm of 004CB8C0"));
        }
        if skip_actors && casts_to(world, reference, RttiClass::Actor) {
            continue;
        }
        let answered = reference_run_scripts(world, reference)?;
        if (answered && !all) || player_cell != parent(world) {
            stopped = true;
            break;
        }
    }
    world.resource_mut::<CellRegionLocks>().release(cell);
    Ok(stopped)
}

/// IDA: 00447D80 on g_dataHandler: +0x64's vt+0x6C until a null item, then 00447CA0: +0xCC read once and tested signed, each nonzero +0xC4 slot's 004CB8C0(cell, 1, 1); +0x0C's worldspaces, a null item skipped: 004EF1E0's +0x34, nonzero, the same. Completion: 100%.
/// # Errors
/// A stop below.
pub fn init_scripts_and_cells(
    world: &mut World,
    services: &mut DataLoadServices<'_>,
) -> Result<(), StreamError> {
    let scripts: Vec<Entity> = world
        .resource::<DataHandlerLists>()
        .scripts_64
        .items()
        .collect();
    for script in scripts {
        init_item_of(world, script, services)?;
    }
    let array = &world.resource::<DataHandler>().cells_c0;
    let cells: Vec<Option<Entity>> = array
        .cells
        .iter()
        .take(array.size_0c.min(i32::MAX as u32) as usize)
        .copied()
        .collect();
    for cell in cells.into_iter().flatten() {
        cell_run_reference_scripts(world, cell, true, true)?;
    }
    let worlds: Vec<Entity> = world
        .resource::<DataHandlerLists>()
        .world_spaces_0c
        .items()
        .collect();
    for form in worlds {
        let cell = world
            .get::<TesWorldSpace>(form)
            .expect("a worldspace")
            .persistent_cell_34;
        if let Some(cell) = cell {
            cell_run_reference_scripts(world, cell, true, true)?;
        }
    }
    Ok(())
}
