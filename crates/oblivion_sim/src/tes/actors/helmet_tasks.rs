//! QueuedHelmet, an actor's slot-1 child (a helmet or a hood): `004781D0` SkinInfo_QueueHelmet, `0043BAF0` QueuedHelmet_Create
//! and `0043C8C0` its Queue (slot 1's model as its QueuedModel child), the walk's `00437B40` Run (the helmet's FaceGen model,
//! `00478A40`) and `0043EB80` CheckFinished (the clone, `004792F0`, fitted to the NPC's face), and the main thread's
//! `00437B60` Finish (`0047B7D0`, the clone onto the skeleton). The walk runs on one thread here (D9).

use std::collections::HashMap;

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::character_3d::{FaceGenSettings, SkinInfo, skin_model_path};
use crate::bs::archive::ArchiveAccess;
use crate::bs::face_gen_model::{
    BsFaceGenModelMap, FaceGenFileServices, FaceGenModel, FaceGenModelFiles,
    apply_helmet_coefficients, load_face_gen_model,
};
use crate::bs::model_loader::{
    IoTaskId, ModelBuildServices, ModelRequest, QueuedModel, TaskParent, fixup_model_path,
    load_model, queue_io_task, task_mut,
};
use crate::bs::reference_tasks::ReferenceTaskId;
use crate::bs::task_manager::TaskGlobals;
use crate::gamebryo::mesh_geometry::{MeshTriShapePayload, NiGeometry};
use crate::gamebryo::scene_node::NiNodeChildren;
use crate::gamebryo::transform::y_rotation;

/// IDA: QueuedHelmet's vtable (0x38 task, 0043BAF0): vt+4 00437B40 Run, vt+8 00435260 (vt+0x28), vt+0x14 00437B60 Finish,
/// vt+0x20 0043C8C0 Queue, vt+0x24 00436E70 QueuedFile_OnChildFinished, vt+0x28 0043EB80 CheckFinished.
pub const QUEUED_HELMET_VTABLE: u32 = 0x00A3_6D2C;

/// IDA: a QueuedHelmet's own fields (`0043BAF0`): +0x20 the skin info, +0x24 slot 1's QueuedModel (`0043C8C0`), +0x28 the
/// helmet's FaceGen model (`00437B40`; an index into `BsFaceGenModelMap::models`), +0x2C the clone (`0043EB80`), +0x30 the
/// owner (the skin's +0x150), the helmet map's key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedHelmet {
    pub skin_20: Entity,
    pub model_task_24: Option<IoTaskId>,
    pub face_gen_model_28: Option<usize>,
    pub clone_2c: Option<Entity>,
    pub owner_30: Entity,
}

/// IDA: N/A the QueuedHelmet tasks' own fields, by task (the walk's store holds their BSTask part); g_modelLoader's +0x10 helmet
/// map (owner to task: vt+0xC adds, refusing a key it holds; vt+0x10 removes); each actor task's +0x3C (`0043DC00`); and the
/// helmets whose vt+0x28 a child's notification reached outside the walk's own step (host form: the walk runs them after the
/// step, D9). Looked up only, never iterated for order but `pending_checks`, which keeps the notifications' order.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct HelmetTasks {
    pub helmets: HashMap<IoTaskId, QueuedHelmet>,
    pub helmet_map_10: HashMap<Entity, IoTaskId>,
    pub helmet_of_actor_3c: HashMap<ReferenceTaskId, IoTaskId>,
    pub pending_checks: Vec<IoTaskId>,
}

/// IDA: 004781D0 SkinInfo_QueueHelmet(skin, out, priority, parent): slot 1's model (+0x60) null or -1 gives a null out;
/// otherwise 0043BAF0 QueuedHelmet_Create(g_modelLoader, out, skin, priority, parent): FormHeap(0x38), IOTask_Constructor
/// (00B33A20 up, the priority in the key's byte 2), +0x18..+0x2C zeroed, +0x20 the skin, +0x30 the skin's owner; out and the
/// helmet map's vt+0xC(owner, task) take references; a key the map holds already frees the task and gives a null out;
/// otherwise QueuedFile_SetParent(parent) and vt+0x20 (`queue_helmet`). Answers out, whose reference the caller holds.
/// Completion: 100%.
/// # Errors
/// No ModelLoader or IOManager, a released parent, or a stop in the queue.
pub fn create_queued_helmet(
    world: &mut World,
    skin: Entity,
    priority: u8,
    parent: Option<ReferenceTaskId>,
    archives: Option<&mut (dyn ArchiveAccess + '_)>,
) -> Result<Option<IoTaskId>, GameError> {
    let info = world.get::<SkinInfo>(skin).expect("a skin info");
    let Some(model) = info.slots_4c[1].model_04 else {
        return Ok(None);
    };
    let owner = info.owner_150;
    let id = {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let TaskGlobals {
            io_manager_b33a10,
            counters,
            ..
        } = &mut *tasks;
        let io = io_manager_b33a10.as_mut().ok_or(GameError::unclassified(
            0x0043BAF0,
            "0043BAF0: a null g_IOManager",
        ))?;
        counters.live_tasks_b33a20 = counters.live_tasks_b33a20.wrapping_add(1);
        let id = IoTaskId(io.tasks.store.len());
        io.tasks.store.push(Some(QueuedModel {
            original_vtable: QUEUED_HELMET_VTABLE,
            // out's reference and the one passed to the map's vt+0xC.
            references_08: 2,
            state_0c: 0,
            key_10: u64::from(priority) << 16,
            parent_18: None,
            children_1c: 0,
            path_20: Vec::new(),
            entry_24: None,
            model_28: None,
            field_2c: 0,
            lod_30: 0,
            flags_34: 0,
            host_gate: None,
        }));
        id
    };
    let duplicate = world
        .get_resource_or_init::<HelmetTasks>()
        .helmet_map_10
        .contains_key(&owner);
    if duplicate {
        // 0043BBBA: the map refuses the key and drops its argument; out's release frees the task (00436670, 00B33A20 - 1).
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let TaskGlobals {
            io_manager_b33a10,
            counters,
            ..
        } = &mut *tasks;
        if let Some(io) = io_manager_b33a10.as_mut() {
            io.tasks.store[id.0] = None;
        }
        counters.live_tasks_b33a20 = counters.live_tasks_b33a20.wrapping_sub(1);
        return Ok(None);
    }
    let mut helmets = world.resource_mut::<HelmetTasks>();
    helmets.helmet_map_10.insert(owner, id);
    helmets.helmets.insert(
        id,
        QueuedHelmet {
            skin_20: skin,
            model_task_24: None,
            face_gen_model_28: None,
            clone_2c: None,
            owner_30: owner,
        },
    );
    if let Some(parent) = parent {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let TaskGlobals {
            io_manager_b33a10,
            model_loader_b33a1c,
            ..
        } = &mut *tasks;
        let io = io_manager_b33a10.as_mut().ok_or(GameError::unclassified(
            0x0043AC40,
            "0043AC40: a null g_IOManager",
        ))?;
        let loader = model_loader_b33a1c.as_mut().ok_or(GameError::unclassified(
            0x0043AC40,
            "0043AC40: a null g_modelLoader",
        ))?;
        crate::bs::model_loader::set_parent(io, loader, id, parent)?;
    }
    queue_helmet(
        world,
        id,
        skin_model_path(world, Some(model)),
        priority,
        archives,
    )?;
    Ok(Some(id))
}

/// IDA: 0043C8C0 QueuedHelmet_Queue (vt+0x20): ModelLoader_LoadTESModel(slot 1's model, the key's priority, this, 3, 0, 1, 0)
/// into +0x24 (its out reference moved there), then g_IOManager vt+0x3C on this task. A cached model's child is finished
/// already, and 004374A0 notifies this task at once. Completion: 100%.
// IDA: N/A `path` is slot 1's model path, read by the caller.
fn queue_helmet(
    world: &mut World,
    id: IoTaskId,
    path: Vec<u8>,
    priority: u8,
    archives: Option<&mut (dyn ArchiveAccess + '_)>,
) -> Result<(), GameError> {
    if path.is_empty() {
        return Err(GameError::unclassified(
            0x0043C916,
            "0043C916: a helmet model with an empty path",
        ));
    }
    let request = ModelRequest {
        priority,
        parent: Some(TaskParent::Io(id)),
        lod: 3,
        a7: false,
        a8: true,
        a9: false,
    };
    let path = fixup_model_path(&path);
    let child = {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        match archives {
            Some(archives) => {
                let mut out = None;
                archives.with_archives(&mut |manager| {
                    out = Some(load_model(&mut tasks, manager, &path, request));
                });
                out.expect("with_archives runs its body")?
            }
            None => load_model(&mut tasks, None, &path, request)?,
        }
    };
    world
        .resource_mut::<HelmetTasks>()
        .helmets
        .get_mut(&id)
        .expect("made above")
        .model_task_24 = child;
    if let Some(child) = child {
        let finished = world
            .resource::<TaskGlobals>()
            .io_manager_b33a10
            .as_ref()
            .and_then(|io| io.tasks.store[child.0].as_ref())
            .is_some_and(|task| task.state_0c == 5);
        if finished {
            crate::tes::reference_load::queued_file_notify_parent(world, child)?;
        }
    }
    let mut tasks = world.resource_mut::<TaskGlobals>();
    let TaskGlobals {
        io_manager_b33a10,
        counters,
        ..
    } = &mut *tasks;
    let io = io_manager_b33a10.as_mut().ok_or(GameError::unclassified(
        0x0043C995,
        "0043C995: a null g_IOManager",
    ))?;
    queue_io_task(io, counters, id)
}

/// IDA: 004BDDC0, a caller's release of its QueuedHelmet out: a null skips; +0x08 down, and at zero vt+0 (00439B40: 004392E0
/// releases the children list and the parent, then the helmet map's vt+0x10 on +0x30; 00436670's 00B33A20 - 1).
/// Completion: 100% of what's modelled.
/// # Errors
/// A released task.
pub fn release_helmet_reference(
    world: &mut World,
    helmet: Option<IoTaskId>,
) -> Result<(), GameError> {
    let Some(id) = helmet else {
        return Ok(());
    };
    let mut tasks = world.resource_mut::<TaskGlobals>();
    let task = task_mut(&mut tasks, id)?;
    task.references_08 -= 1;
    if task.references_08 != 0 {
        return Ok(());
    }
    Err(GameError::unclassified(
        0x00439B40,
        "00439B40: a QueuedHelmet's destructor (its children, parent and helmet map entry)",
    ))
}

/// IDA: N/A `00437B40` and `0043EB80` with the walk's services: FaceGen's files through the archives (BSFile_OpenCore(path, 0,
/// 0x2800, 1)), FaceGen's last error (00B3F068) and its warnings through DebugMessage_Warning, as `run_queued_head_in_walk`.
pub(crate) fn with_face_gen_files<T>(
    world: &mut World,
    services: &mut ModelBuildServices<'_>,
    body: impl FnOnce(&mut World, &mut FaceGenFileServices<'_>, &mut ModelBuildServices<'_>) -> T,
) -> T {
    let mut last_error = world
        .remove_resource::<crate::tes::facegen::FaceGenLastError>()
        .unwrap_or_default();
    let mut warnings = Vec::new();
    let shared = std::cell::RefCell::new(services.archives.take());
    let result = {
        let mut no_nif =
            |_: &mut World, _: &str| -> Result<crate::bs::stream::BsStreamLoad, GameError> {
                Err(GameError::unclassified(
                    0x00559330,
                    "00559330: a helmet's FaceGen model has no NIF",
                ))
            };
        let mut open = |path: &str| {
            let mut opened = None;
            if let Some(archives) = shared.borrow_mut().as_deref_mut() {
                archives.with_archives(&mut |manager| {
                    opened = manager.and_then(|manager| manager.open_packed(path, 0x2800, 1));
                });
            }
            opened
        };
        let mut exists = |path: &str| {
            let mut found = false;
            if let Some(archives) = shared.borrow_mut().as_deref_mut() {
                archives.with_archives(&mut |manager| {
                    found = manager
                        .is_some_and(|manager| manager.open_packed(path, 0, 0xFFFF).is_some());
                });
            }
            found
        };
        let mut files = FaceGenFileServices {
            load_nif: &mut no_nif,
            open: &mut open,
            exists: &mut exists,
            warnings: &mut warnings,
            last_error: &mut last_error,
            precision24: false,
        };
        body(world, &mut files, services)
    };
    services.archives = shared.into_inner();
    world.insert_resource(last_error);
    for text in warnings {
        let disabled = *services.disable_warning_messages_b06b18 != 0;
        (services.warning)(text.into_bytes(), disabled);
    }
    result
}

/// IDA: 00478A40 SkinInfo_GetHelmetFaceGenModel(skin): under 00B33E00 (no host form, D9), slot 1's model (+0x60, null gives
/// null): "Data\Meshes\" and its path (vt+0x14), FaceGen_MakeEgmPath (the text to its last '.', then ".egm"; no '.' gives
/// null), and 00553620 FaceGen_GetModel(egm, null, null, null, 1, 0): the model map's entry (loaded through 00559B50 when it
/// has no data), else a new BSFaceGenModel (005564A0) loaded and added, or null when the load fails. Completion: 100%.
fn helmet_face_gen_model(
    world: &mut World,
    skin: Entity,
    files: &mut FaceGenFileServices<'_>,
) -> Result<Option<usize>, GameError> {
    let Some(model) = world.get::<SkinInfo>(skin).expect("a skin info").slots_4c[1].model_04 else {
        return Ok(None);
    };
    let mut path = b"Data\\Meshes\\".to_vec();
    path.extend_from_slice(&skin_model_path(world, Some(model)));
    let Some(dot) = path.iter().rposition(|&byte| byte == b'.') else {
        return Ok(None);
    };
    let mut egm = path[..dot].to_vec();
    egm.extend_from_slice(b".egm");
    crate::tes::face_gen_tasks::ensure_model_map(world)?;
    let use_heads = world
        .get_resource_or_init::<FaceGenSettings>()
        .use_face_gen_heads_b120b4;
    let files_of = FaceGenModelFiles {
        egm: &egm,
        nif: None,
        tri: None,
        egt: None,
    };
    let found = world.resource::<BsFaceGenModelMap>().find(&egm);
    if let Some(index) = found {
        let mut held = world.resource::<BsFaceGenModelMap>().models[index].clone();
        if !held.loaded {
            load_face_gen_model(world, &mut held, files_of, false, use_heads, files)?;
        }
        world.resource_mut::<BsFaceGenModelMap>().models[index] = held;
        return Ok(Some(index));
    }
    let mut held = FaceGenModel::default();
    if !load_face_gen_model(world, &mut held, files_of, false, use_heads, files)? {
        return Ok(None);
    }
    let mut map = world.resource_mut::<BsFaceGenModelMap>();
    map.models.push(held);
    let index = map.models.len() - 1;
    map.add(&egm, index);
    Ok(Some(index))
}

/// IDA: 00437B40 QueuedHelmet_Run (vt+4): state 6 does nothing; otherwise +0x28 takes 00478A40's FaceGen model of +0x20.
/// Completion: 100%.
/// # Errors
/// A stop in the model's load.
pub fn run_queued_helmet_in_walk(
    world: &mut World,
    id: IoTaskId,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    let state = task_mut(&mut world.resource_mut::<TaskGlobals>(), id)?.state_0c;
    if state == 6 {
        return Ok(());
    }
    let skin = world
        .resource::<HelmetTasks>()
        .helmets
        .get(&id)
        .ok_or(GameError::unclassified(
            0x00437B40,
            "00437B40: a QueuedHelmet with no fields",
        ))?
        .skin_20;
    let model = with_face_gen_files(world, services, |world, files, _| {
        helmet_face_gen_model(world, skin, files)
    })?;
    world
        .resource_mut::<HelmetTasks>()
        .helmets
        .get_mut(&id)
        .expect("read above")
        .face_gen_model_28 = model;
    Ok(())
}

/// IDA: N/A the walk's step end: each helmet a child's notification queued (`helmet_child_finished`) runs `check_finished`, in
/// the notifications' order.
/// # Errors
/// As `check_finished`.
pub fn run_pending_helmet_checks(
    world: &mut World,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    loop {
        let next = world
            .get_resource_mut::<HelmetTasks>()
            .and_then(|mut helmets| {
                (!helmets.pending_checks.is_empty()).then(|| helmets.pending_checks.remove(0))
            });
        let Some(id) = next else {
            return Ok(());
        };
        check_finished(world, id, services)?;
    }
}

/// IDA: 0043EB80 QueuedHelmet_CheckFinished (vt+0x28): with state 4 or more, every child finished and not 6: for an owner
/// (+0x30) other than the player, a child (+0x24) whose Model (+0x28) has a root (+0x08) gives +0x2C `004792F0`'s clone. A
/// clone with no skinned geometry child (vt+0x0C with +0xB8) takes the FaceGen fit with +0x28's model (else 00478A40's): each
/// child with +0xB4 (a geometry's data), for an NPC owner's base (type 0x23): the data deep-copied (`00700790`) into the
/// geometry (vt+0x8C), 005221C0's coefficients, and under g_faceGen's lock (no host form) without bUseFaceGenHeads, or with
/// `00558840`(model, coefficients, geometry, 1.0, null) answering 1, the geometry's rotation (+0x30) times 0070FD80(-pi/2)
/// (007100A0). Then with no parent (+0x18) 004BD750 queues the task for the main thread (a stop here), and
/// QueuedFile_NotifyParent. Completion: 100% of a rigid helmet; a skinned one's fit is 0047B090's on the main thread.
/// # Errors
/// A helmet with no parent, a node child the fit would read as a geometry, or a stop in the clone or the fit.
pub fn check_finished(
    world: &mut World,
    id: IoTaskId,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    let (state, children, has_parent) = {
        let mut tasks = world.resource_mut::<TaskGlobals>();
        let task = task_mut(&mut tasks, id)?;
        (task.state_0c, task.children_1c, task.parent_18.is_some())
    };
    if state >= 4 && children == 0 && state != 6 {
        let helmet = world
            .resource::<HelmetTasks>()
            .helmets
            .get(&id)
            .cloned()
            .ok_or(GameError::unclassified(
                0x0043EB80,
                "0043EB80: a QueuedHelmet with no fields",
            ))?;
        if Some(helmet.owner_30) != crate::tes::player::the_player(world) {
            fit_helmet(world, id, &helmet, services)?;
        }
        if !has_parent {
            return Err(GameError::unclassified(
                0x0043EE7B,
                "0043EE7B: a QueuedHelmet with no parent (004BD750, the IOManager's task queue)",
            ));
        }
    }
    crate::tes::reference_load::queued_file_notify_parent(world, id)
}

// IDA: 0043EBF9..0043EE59, the clone and its FaceGen fit (see `check_finished`).
fn fit_helmet(
    world: &mut World,
    id: IoTaskId,
    helmet: &QueuedHelmet,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    let root = {
        let tasks = world.resource::<TaskGlobals>();
        let model = helmet
            .model_task_24
            .and_then(|child| {
                tasks
                    .io_manager_b33a10
                    .as_ref()
                    .and_then(|io| io.tasks.store[child.0].as_ref())
            })
            .and_then(|child| child.model_28);
        model.and_then(|model| {
            tasks
                .model_loader_b33a1c
                .as_ref()
                .and_then(|loader| loader.models.store[model.0].root_08)
        })
    };
    let Some(root) = root else {
        return Ok(());
    };
    let clone = super::biped_attach::clone_helmet(
        world,
        helmet.skin_20,
        helmet.face_gen_model_28,
        Some(root),
        services,
    )?;
    world
        .resource_mut::<HelmetTasks>()
        .helmets
        .get_mut(&id)
        .expect("read above")
        .clone_2c = clone;
    let Some(clone) = clone else {
        return Ok(());
    };
    let children = world
        .get::<NiNodeChildren>(clone)
        .map(|children| children.slots_ac.clone())
        .unwrap_or_default();
    let skinned = children.iter().flatten().any(|&child| {
        world
            .get::<NiGeometry>(child)
            .is_some_and(|geometry| geometry.skin_b8.is_some())
    });
    if skinned {
        return Ok(());
    }
    let skin = helmet.skin_20;
    with_face_gen_files(world, services, |world, files, _| {
        let model = match helmet.face_gen_model_28 {
            Some(model) => Some(model),
            None => helmet_face_gen_model(world, skin, files)?,
        };
        let Some(model) = model else {
            return Ok(());
        };
        for child in children.into_iter().flatten() {
            let Some(data) = world
                .get::<NiGeometry>(child)
                .and_then(|geometry| geometry.data_b4)
            else {
                if world
                    .get::<NiNodeChildren>(child)
                    .is_some_and(|children| !children.slots_ac.is_empty())
                {
                    return Err(GameError::unclassified(
                        0x0043ECF0,
                        "0043ECF0: a helmet's child node, whose +0xB4 the fit reads as geometry data",
                    ));
                }
                continue;
            };
            let base = world
                .get::<super::actor::TesObjectRefr>(helmet.owner_30)
                .and_then(|refr| refr.base_form_1c)
                .filter(|&base| crate::tes::forms_init::form_type(world, base) == 0x23);
            let Some(base) = base else {
                continue;
            };
            // 0043ED80 00700790: a deep copy of the data, taken by the geometry (vt+0x8C).
            let copy = world.entity_mut(data).clone_and_spawn();
            world
                .get_mut::<NiGeometry>(child)
                .expect("a geometry")
                .data_b4 = Some(copy);
            let coefficients = crate::tes::face_gen_head::npc_face_gen_coefficients(world, base)?;
            let use_heads = world
                .get_resource_or_init::<FaceGenSettings>()
                .use_face_gen_heads_b120b4;
            let applied = if use_heads {
                apply_helmet_coefficients(world, model, &coefficients, copy, files)?
            } else {
                true
            };
            if applied {
                let mut payload =
                    world
                        .get_mut::<MeshTriShapePayload>(child)
                        .ok_or(GameError::unclassified(
                            0x0043EE17,
                            "0043EE17: a helmet geometry with no transform",
                        ))?;
                payload.local_transform.rotation = crate::gamebryo::transform::multiply(
                    &payload.local_transform.rotation,
                    &y_rotation(-1.570_796_4),
                );
            }
        }
        Ok(())
    })
}

/// IDA: 00437B60 QueuedHelmet_Finish (vt+0x14), from 0043B090 QueuedCharacter_FinishOnMainThread after the head: with the
/// child's Model (+0x24's +0x28), not state 6 and a clone (+0x2C): the Model's +0x04 up and `0047B7D0`(skin, +0x28, +0x2C)
/// (`biped_attach::attach_helmet`); then the helmet map's vt+0x10 removes the owner (+0x30), dropping the map's reference.
/// Completion: 100%.
/// # Errors
/// A released helmet, or a stop in the attach.
pub fn queued_helmet_finish(
    world: &mut World,
    id: IoTaskId,
    services: &mut ModelBuildServices<'_>,
) -> Result<(), GameError> {
    let helmet = world
        .resource::<HelmetTasks>()
        .helmets
        .get(&id)
        .cloned()
        .ok_or(GameError::unclassified(
            0x00437B60,
            "00437B60: a QueuedHelmet with no fields",
        ))?;
    let state = task_mut(&mut world.resource_mut::<TaskGlobals>(), id)?.state_0c;
    let model = {
        let tasks = world.resource::<TaskGlobals>();
        helmet
            .model_task_24
            .and_then(|child| {
                tasks
                    .io_manager_b33a10
                    .as_ref()
                    .and_then(|io| io.tasks.store[child.0].as_ref())
            })
            .and_then(|child| child.model_28)
    };
    if let Some(model) = model
        && state != 6
        && helmet.clone_2c.is_some()
    {
        {
            let mut tasks = world.resource_mut::<TaskGlobals>();
            let held = &mut tasks
                .model_loader_b33a1c
                .as_mut()
                .ok_or(GameError::unclassified(
                    0x00437B7D,
                    "00437B7D: a null g_modelLoader",
                ))?
                .models
                .store[model.0];
            held.references_04 = held.references_04.wrapping_add(1);
        }
        super::biped_attach::attach_helmet(
            world,
            helmet.skin_20,
            helmet.face_gen_model_28,
            helmet.clone_2c,
            services,
        )?;
    }
    let removed = {
        let mut helmets = world.resource_mut::<HelmetTasks>();
        let held = helmets.helmet_map_10.get(&helmet.owner_30) == Some(&id);
        if held {
            helmets.helmet_map_10.remove(&helmet.owner_30);
        }
        held
    };
    if removed {
        task_mut(&mut world.resource_mut::<TaskGlobals>(), id)?.references_08 -= 1;
    }
    Ok(())
}
