//! A reference's 3D load through the model loader: 00438060 ModelLoader_QueueReference, the QueuedReference family's virtuals (`QueuedReference_QueueModelChild`, `QueuedReference_OnChildFinished`, the clone thread's run and the main thread's finish), and `QueuedFile_NotifyParent` from a finished model child.
//! Evidence: `evidence/gameplay-run002-2026-11-07/` run-ab (464 of the prison's 467 references queued at priority 1) and run-af (the drain). Host substitution (D9): the clone thread and the finish queue run on the main thread, in queue order (`bs::reference_tasks`).

use bevy_ecs::prelude::*;
use recomp_core::crt::stricmp;
use recomp_core::reader::StreamError;

use super::actor::{Actor, TesObjectRefr};
use super::default_forms::DefaultFormStatics;
use super::form::FormIdentity;
use super::form_components::TesModel;
use super::forms_init::{form_id, form_type};
use crate::bs::archive::ArchiveAccess;
use crate::bs::model_loader::{IoTaskId, ModelRequest, load_model};
use crate::bs::reference_tasks::{QueuedReference, QueuedReferenceClass, ReferenceTaskId};
use crate::bs::task_manager::TaskGlobals;

// IDA: N/A a TESModel's path, up to its terminator.
pub(crate) fn model_path_of(world: &World, form: Entity) -> Option<Vec<u8>> {
    model_path(world, form)
}

// IDA: N/A a form's TESModel path, to its terminator.
fn model_path(world: &World, form: Entity) -> Option<Vec<u8>> {
    let model = world.get::<TesModel>(form)?;
    let bytes = model.path_04.data_00.as_ref()?;
    Some(
        bytes
            .split(|&byte| byte == 0)
            .next()
            .unwrap_or_default()
            .to_vec(),
    )
}

/// IDA: 004B2A00 TESBoundObject_IsMarker, the bound objects' vt+0xF4 (is a marker): an activator (0x12) whose model is `EditorMarker.NIF` (0046D3F0, `_stricmp`); the prison marker door (0x18); a light (0x1A) with no model path (0046DA90); the door, travel, temple, divine, north, map, horse, X and XMarkerHeading statics (0x1C); a subspace (0x29); otherwise a form ID from 100 to 119. Completion: 100% of the TESModel arm; 004694A0's reference arm isn't reached from a base.
#[must_use]
pub fn bound_object_is_marker(world: &World, base: Entity) -> bool {
    let statics = world.resource::<DefaultFormStatics>();
    let is = |slot: Option<Entity>| slot == Some(base);
    let default_object = || form_id(world, base).wrapping_sub(100) <= 0x13;
    match form_type(world, base) {
        0x12 => {
            let path = model_path(world, base).unwrap_or_default();
            stricmp(&path, b"EditorMarker.NIF") == 0 || default_object()
        }
        0x18 => is(statics.prison_marker_b35ebc) || default_object(),
        0x1A => model_path(world, base).is_none_or(|path| path.is_empty()) || default_object(),
        0x1C => {
            [
                statics.door_marker_b35ea4,
                statics.travel_marker_b35eb4,
                statics.temple_marker_b35ec0,
                statics.divine_marker_b35ec4,
                statics.north_marker_b35eb8,
                statics.map_marker_b35ea8,
                statics.horse_marker_b35ed4,
                statics.x_marker_b35eac,
                statics.x_marker_heading_b35eb0,
            ]
            .into_iter()
            .any(is)
                || default_object()
        }
        0x29 => true,
        _ => default_object(),
    }
}

/// IDA: N/A what the queue reaches outside the world: the archives for the model child's file entry.
pub struct ReferenceLoadServices<'a, 'b> {
    pub archives: Option<&'a mut (dyn ArchiveAccess + 'b)>,
}

/// IDA: 00438060 ModelLoader_QueueReference(reference, priority): a save loading without +0x18 bit 1 returns; a marker base (vt+0xF4) returns unless it's a light; a sound (0x0A) returns; an actor with a combat controller gets 00612DE0(13) (the port starts no combat yet: none). A reference already in the +0x08 map only has a different priority set (vt+0x1C), a stop here. Otherwise the class by the base (`QueuedReferenceClass`), 004BCB70's reference, the map's insert and the task's vt+0x20 (`queue_reference_task`). Completion: 100% but the requeue.
/// # Errors
/// A stop in the task's queue.
pub fn model_loader_queue_reference(
    world: &mut World,
    reference: Entity,
    priority: u8,
    services: &mut ReferenceLoadServices<'_, '_>,
) -> Result<(), StreamError> {
    if super::forms_references::save_load_active(world)?
        && super::forms_references::save_load_flags(world)? & 2 == 0
    {
        return Ok(());
    }
    let Some(base) = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
    else {
        return Err(StreamError("0043807D: a reference with no base"));
    };
    let base_type = form_type(world, base);
    if bound_object_is_marker(world, base) && base_type != 0x1A {
        return Ok(());
    }
    if base_type == 0x0A {
        return Ok(());
    }
    let queued = world
        .resource::<TaskGlobals>()
        .model_loader_b33a1c
        .as_ref()
        .ok_or(StreamError("004380E6: a null g_modelLoader"))?
        .references
        .map_08
        .contains_key(&reference);
    if queued {
        return Err(StreamError("00438101: a reference queued again (vt+0x1C)"));
    }
    let class = if Some(reference) == super::ai_conditions::the_player(world) {
        QueuedReferenceClass::Player
    } else {
        match base_type {
            0x1E => QueuedReferenceClass::Tree,
            0x23 => QueuedReferenceClass::Character,
            0x24 => QueuedReferenceClass::Creature,
            _ => QueuedReferenceClass::Reference,
        }
    };
    let mut tasks = world.resource_mut::<TaskGlobals>();
    tasks.counters.live_tasks_b33a20 = tasks.counters.live_tasks_b33a20.wrapping_add(1);
    let loader = tasks.model_loader_b33a1c.as_mut().expect("tested above");
    let id = loader.references.insert(QueuedReference {
        class,
        references_08: 1,
        state_0c: 0,
        key_10: u64::from(priority) << 16,
        children_1c: Vec::new(),
        children_finished_1c_10: 0,
        reference_20: reference,
        child_24: None,
        model_28: None,
        clone_2c: None,
    });
    loader.references.map_08.insert(reference, id);
    queue_reference_task(world, id, services)
}

/// IDA: 004354A0 QueuedReference_Queue, the QueuedReference family's vt+0x20: a reference flagged 0x8000 (004D6FD0) with no node while the loading menu shows runs vt+0x30 (00437D40), a stop here; then vt+0x2C (the model child), +0x0C = 5, vt+0x28 (`reference_task_child_finished`). QueuedCharacter's vt+0x2C is `character_3d::queue_character_children` (0043DC00); QueuedCreature's (0043DD80) and QueuedPlayer's (0043DE00) are Build 2's. Completion: 100% of QueuedReference and QueuedTree.
fn queue_reference_task(
    world: &mut World,
    id: ReferenceTaskId,
    services: &mut ReferenceLoadServices<'_, '_>,
) -> Result<(), StreamError> {
    let task = reference_task(world, id)?.clone();
    let reference = task.reference_20;
    if world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08
        & 0x8000
        != 0
        && world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .loaded_3d_3c
            == 0
    {
        return Err(StreamError(
            "004354C4: vt+0x30 (00437D40) for a reference flagged 0x8000",
        ));
    }
    match task.class {
        QueuedReferenceClass::Reference | QueuedReferenceClass::Tree => {
            queue_model_child(world, id, services)?;
        }
        QueuedReferenceClass::Character => {
            let base = world
                .get::<TesObjectRefr>(reference)
                .expect("a reference")
                .base_form_1c
                .expect("tested at the queue");
            if let Err(stop) =
                super::character_3d::queue_character_children(world, id, reference, base, services)
            {
                // Build 2: an arm not ported yet; the task stays queued with no child.
                world
                    .get_resource_or_init::<super::character_3d::CharacterTaskStops>()
                    .stops
                    .push((reference, stop.0));
                world
                    .get_resource_or_init::<ReferenceLoadBoundaries>()
                    .actor_tasks += 1;
                return Ok(());
            }
            queue_model_child(world, id, services)?;
        }
        QueuedReferenceClass::MagicItem => {
            return Err(StreamError(
                "004354A0: a QueuedMagicItem queued as a reference",
            ));
        }
        QueuedReferenceClass::Creature => {
            let base = world
                .get::<TesObjectRefr>(reference)
                .expect("a reference")
                .base_form_1c
                .expect("tested at the queue");
            if let Err(stop) =
                super::character_3d::queue_creature_children(world, id, reference, base, services)
            {
                // Build 2: an arm not ported yet; the task stays queued with no child.
                world
                    .get_resource_or_init::<super::character_3d::CharacterTaskStops>()
                    .stops
                    .push((reference, stop.0));
                world
                    .get_resource_or_init::<ReferenceLoadBoundaries>()
                    .actor_tasks += 1;
                return Ok(());
            }
            queue_model_child(world, id, services)?;
        }
        QueuedReferenceClass::Player => {
            let base = world
                .get::<TesObjectRefr>(reference)
                .expect("a reference")
                .base_form_1c
                .expect("tested at the queue");
            if let Err(stop) =
                super::character_3d::queue_player_children(world, id, reference, base, services)
            {
                // Build 2: an arm not ported yet; the task stays queued with no child.
                world
                    .get_resource_or_init::<super::character_3d::CharacterTaskStops>()
                    .stops
                    .push((reference, stop.0));
                world
                    .get_resource_or_init::<ReferenceLoadBoundaries>()
                    .actor_tasks += 1;
                return Ok(());
            }
            queue_model_child(world, id, services)?;
        }
    }
    reference_task_mut(world, id)?.state_0c = 5;
    reference_task_child_finished(world, id)
}

/// IDA: N/A what the reference loads leave to later builds: the actor classes' tasks, queued with no child (a QueuedCharacter's stop is in `CharacterTaskStops`).
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ReferenceLoadBoundaries {
    pub actor_tasks: u32,
}

// IDA: N/A a reference task by ID.
fn reference_task(world: &World, id: ReferenceTaskId) -> Result<&QueuedReference, StreamError> {
    world
        .resource::<TaskGlobals>()
        .model_loader_b33a1c
        .as_ref()
        .and_then(|loader| loader.references.get(id))
        .ok_or(StreamError("a released QueuedReference"))
}

// IDA: N/A a helper, with no address of its own.
fn reference_task_mut(
    world: &mut World,
    id: ReferenceTaskId,
) -> Result<Mut<'_, QueuedReference>, StreamError> {
    let tasks = world.resource_mut::<TaskGlobals>();
    if tasks
        .model_loader_b33a1c
        .as_ref()
        .and_then(|loader| loader.references.get(id))
        .is_none()
    {
        return Err(StreamError("a released QueuedReference"));
    }
    Ok(tasks.map_unchanged(|tasks| {
        tasks
            .model_loader_b33a1c
            .as_mut()
            .and_then(|loader| loader.references.get_mut(id))
            .expect("tested above")
    }))
}

/// IDA: `QueuedReference_QueueModelChild` (0043C9B0), vt+0x2C: the base's model (`TESBoundObject_GetModelForReference`: its TESModel, or a biped form's model by the owner's sex, a stop here); one with a path goes through `ModelLoader_QueueModelForReference` (0043B990): `ModelLoader_LoadTESModel`(model, the task's priority, this task, 004A2A30's stage or 6 for a reference flagged 0x8000, base null or vt[1]+0x18 false, 1, 0), into +0x24. With no child, the model map's entry into +0x28. Completion: 100% of a TESModel.
fn queue_model_child(
    world: &mut World,
    id: ReferenceTaskId,
    services: &mut ReferenceLoadServices<'_, '_>,
) -> Result<(), StreamError> {
    let task = reference_task(world, id)?.clone();
    let reference = task.reference_20;
    let base = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
        .expect("tested at the queue");
    let child = queue_model_for_reference(world, id, base, reference, services)?;
    reference_task_mut(world, id)?.child_24 = child;
    Ok(())
}

/// IDA: 0043B990 ModelLoader_QueueModelForReference(out, form, the task's priority, task, reference): `model_for_reference`'s
/// model, `ModelLoader_LoadTESModel` at 004A2A30's stage (6 for a reference flagged 0x8000), a7 the reference's base not a marker,
/// a8 1, a9 0; a cached model's child, finished already, notifies at once (004374A0). Completion: 100% of a TESModel or biped
/// form's model.
/// # Errors
/// As `load_model` and `queued_file_notify_parent`.
pub(crate) fn queue_model_for_reference(
    world: &mut World,
    id: ReferenceTaskId,
    form: Entity,
    reference: Entity,
    services: &mut ReferenceLoadServices<'_, '_>,
) -> Result<Option<IoTaskId>, StreamError> {
    let task = reference_task(world, id)?.clone();
    let base = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
        .expect("tested at the queue");
    let Some(path) = model_for_reference(world, form, reference)? else {
        return Ok(None);
    };
    if path.is_empty() {
        return Ok(None);
    }
    let stage = if world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08
        & 0x8000
        != 0
    {
        6
    } else {
        model_stage(form_type(world, form))
    };
    let request = ModelRequest {
        priority: task.priority(),
        parent: Some(id),
        lod: stage,
        a7: !bound_object_is_marker(world, base),
        a8: true,
        a9: false,
    };
    let path = crate::bs::model_loader::fixup_model_path(&path);
    let mut tasks = world.resource_mut::<TaskGlobals>();
    let child = match services.archives.as_deref_mut() {
        Some(archives) => {
            let mut out = None;
            archives.with_archives(&mut |manager| {
                out = Some(load_model(&mut tasks, manager, &path, request));
            });
            out.expect("with_archives runs its body")?
        }
        None => load_model(&mut tasks, None, &path, request)?,
    };
    if let Some(child) = child {
        // A cached model's child is finished already: 004374A0 notifies at once.
        let finished = world
            .resource::<TaskGlobals>()
            .io_manager_b33a10
            .as_ref()
            .and_then(|io| io.tasks.store[child.0].as_ref())
            .is_some_and(|task| task.state_0c == 5);
        if finished {
            queued_file_notify_parent(world, child)?;
        }
    }
    Ok(child)
}

/// IDA: `TESBoundObject_GetModelForReference` (00435580): the base's TESModel; otherwise a biped form's world model by the reference's effective owner's sex (004DB6B0, an NPC's 00519D20, else 0) through 00469240: +0x38 + 0x18 * sex, the male one when a female path is empty. Neither: none. Completion: 100%.
pub(crate) fn model_for_reference(
    world: &World,
    base: Entity,
    reference: Entity,
) -> Result<Option<Vec<u8>>, StreamError> {
    if world.get::<TesModel>(base).is_some() {
        return Ok(model_path(world, base));
    }
    let Some(biped) = world.get::<super::form_components::TesBipedModelForm>(base) else {
        return Ok(None);
    };
    let mut sex = 0;
    if let Some(owner) = super::ai_cell_moves::effective_owner(world, reference)?
        && form_type(world, owner) == 0x23
    {
        sex = super::hud_main_menu::npc_sex(world, owner).unwrap_or(0);
    }
    let path_of = |model: &TesModel| {
        model
            .path_04
            .data_00
            .as_ref()
            .map(|bytes| {
                bytes
                    .split(|&byte| byte == 0)
                    .next()
                    .unwrap_or_default()
                    .to_vec()
            })
            .unwrap_or_default()
    };
    if sex == 1 && path_of(&biped.world_models_38[1]).is_empty() {
        sex = 0;
    }
    Ok(Some(path_of(&biped.world_models_38[sex.min(1)])))
}

/// IDA: 004A2A30 TESForm_GetIOStage: the IO stage by a base's type: 2 for the small-item types (0x13..0x16, 0x19, 0x1B, 0x21, 0x22, 0x26..0x28, 0x2A), 3 for NPCs and creatures, otherwise 1. Completion: 100%.
#[must_use]
pub const fn model_stage(base_type: u8) -> u32 {
    match base_type {
        0x13..=0x16 | 0x19 | 0x1B | 0x21 | 0x22 | 0x26..=0x28 | 0x2A => 2,
        0x23 | 0x24 => 3,
        _ => 1,
    }
}

/// IDA: `QueuedFile_NotifyParent` (00436F30) on a finished model child: a parent, state 4 or more and every child of its own finished: the parent's vt+0x24 (00436E70: its finished count up, then vt+0x28) and the link dropped. Completion: 100% for a reference task's model child.
/// # Errors
/// A stop in the parent's vt+0x28.
pub fn queued_file_notify_parent(world: &mut World, child: IoTaskId) -> Result<(), StreamError> {
    let parent = {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let Some(task) = tasks
            .io_manager_b33a10
            .as_mut()
            .and_then(|io| io.tasks.store[child.0].as_mut())
        else {
            return Err(StreamError("00436F30: a released QueuedModel"));
        };
        if task.state_0c < 4 || task.children_1c != 0 {
            return Ok(());
        }
        let Some(parent) = task.parent_18.take() else {
            return Ok(());
        };
        parent
    };
    {
        let mut task = reference_task_mut(world, parent)?;
        task.children_finished_1c_10 += 1;
        task.references_08 -= 1;
    }
    reference_task_child_finished(world, parent)
}

/// IDA: `QueuedReference_OnChildFinished` (0043CAE0), vt+0x28: state 4 or more, every child finished and not 6: the child's Model into +0x28 when +0x28 is null; with a Model, the reference not an actor (vt+0x190), its cell attached, 00441E90 true and no node, the clone thread (`QueuedReference_QueueOnCloneThread`); otherwise the finish queue (004BD750, 0043A5F0). Completion: 100%.
/// # Errors
/// A released task.
pub fn reference_task_child_finished(
    world: &mut World,
    id: ReferenceTaskId,
) -> Result<(), StreamError> {
    let task = reference_task(world, id)?.clone();
    if task.class == QueuedReferenceClass::MagicItem {
        return super::magic_selection::magic_item_task_children_finished(world, id);
    }
    if task.state_0c < 4 || !task.children_done() || task.state_0c == 6 {
        return Ok(());
    }
    let mut model = task.model_28;
    if model.is_none()
        && let Some(child) = task.child_24
    {
        model = world
            .resource::<TaskGlobals>()
            .io_manager_b33a10
            .as_ref()
            .and_then(|io| io.tasks.store[child.0].as_ref())
            .and_then(|child| child.model_28);
        reference_task_mut(world, id)?.model_28 = model;
    }
    let reference = task.reference_20;
    let to_clone = model.is_some()
        && world.get::<Actor>(reference).is_none()
        && super::ai_package::parent_cell(world, reference)
            .is_some_and(|cell| super::ai_cell_moves::cell_attached(world, cell, false))
        && reference_takes_clone(world, reference)
        && world
            .get::<TesObjectRefr>(reference)
            .expect("a reference")
            .loaded_3d_3c
            == 0;
    let mut tasks = world.resource_mut::<TaskGlobals>();
    let queues = &mut tasks
        .model_loader_b33a1c
        .as_mut()
        .ok_or(StreamError("0043CB4A: a null g_modelLoader"))?
        .references;
    if to_clone {
        queues.clone_queue.push_back(id);
    } else {
        queues.finish_queue.push_back(id);
    }
    Ok(())
}

/// IDA: 00441E90 TES_ReferenceUsesCloneThread: a live reference (no 0x20, no 0x800) whose base is a light, an activator, or not a marker (vt+0xF4). Completion: 100%.
#[must_use]
pub fn reference_takes_clone(world: &World, reference: Entity) -> bool {
    let flags = world
        .get::<FormIdentity>(reference)
        .expect("a form")
        .flags_08;
    if flags & 0x20 != 0 || flags & 0x800 != 0 {
        return false;
    }
    let Some(base) = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .base_form_1c
    else {
        return true;
    };
    matches!(form_type(world, base), 0x1A | 0x12) || !bound_object_is_marker(world, base)
}
