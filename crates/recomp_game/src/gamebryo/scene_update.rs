//! The NiAVObject update passes: 00707370 Update, 007073A0 UpdateSelected, vtable +0x4C UpdateControllers.
//!
//! Virtual dispatch on +0x60 / +0x64 / +0x68 / +0x74 / +0x78 becomes a match on `NiClass`. A controller's vtable +0x54
//! Update goes through `SceneUpdateRuntime`; `SceneControllers` is the real one, and tests inject their own.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::av_object::{NiAvParent, NiAvProperties};
use super::controller_sequence::update_controller_manager;
use super::mesh_geometry::{MeshTriShapePayload, NiGeometry, geometry_data_base};
use super::object_net::NiObjectNet;
use super::particle_controller_schedule::{
    ParticleControllerScheduleContext, ParticleControllerScheduleResult,
    ParticleControllerScheduleStatus, ParticleControllerScheduler,
};
use super::particle_data::ParticleData;
use super::particle_dynamics::merge_ni_sphere;
use super::particle_system::ParticleSystem;
use super::scene_derived::{BillboardNode, DirectionalLight, PointLight};
use super::scene_node::{NiAvWorld, NiNodeChildren, SceneNodePayload, update_world_transform};
use super::stream_objects::NiClass;
use super::texture::{
    TextureController, update_flip_controller, update_texture_transform_controller,
};
use super::transform::{
    NiTransform, compose_scene_transform, invert_scene_transform, rotate_vector,
};
use super::transform_controller::{
    ControllerTimeMap, NodeController, NodeControllerUpdateStatus,
    update_multi_target_transform_controller, update_transform_controller, update_vis_controller,
};
use crate::bs::scene_graphs::{SceneCamera, update_camera_world_to_clip};

type Status = SceneUpdateStatus;

/// IDA: N/A how a controller's Update went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneControllerStep {
    /// Skipped, no interpolator, interpolator failed or applied: the original outcomes. `next` is the +0x34 link, read
    /// after the Update returns.
    Completed {
        next: Option<Entity>,
        status: NodeControllerUpdateStatus,
    },
    /// A bound controller's Update returned a host-unsupported status.
    Failed { status: NodeControllerUpdateStatus },
    /// A controller whose vtable +0x54 Update is not ported.
    Unsupported,
}

/// IDA: N/A the host's vtable +0x54 dispatch for the 00707370 / 007073A0 passes. It stands for the C++ port's
/// `SceneUpdateRuntime`, whose `particles` member carries the one 007155A0 time cache. An implementor routes every
/// controller through that one `ParticleControllerTimeCache`.
pub trait SceneUpdateRuntime {
    /// IDA: N/A 00591360's texture, for a runtime whose owner holds the renderer and the palette: the outer `None` says this runtime can't, and the seam's DDS header answers. The file's size. Completion: 100%.
    fn tile_texture_size(&mut self, _world: &mut World, _path: &str) -> Option<Option<[u32; 2]>> {
        None
    }

    /// IDA: vtable +0x54 Update(time) of `controller`.
    fn update_controller(
        &mut self,
        world: &mut World,
        controller: Entity,
        time: f32,
    ) -> Result<SceneControllerStep, StreamError>;

    /// IDA: 0047C930's chain on a NiParticleSystem: 00753BE0 / 00750260 per +0x54. With no services, the scheduler reports
    /// the service it lacks, as the C++ does with an empty context.
    fn run_particle_controllers(
        &mut self,
        world: &mut World,
        head: Option<Entity>,
        system: Entity,
        time: f32,
    ) -> Result<ParticleControllerScheduleResult, StreamError> {
        ParticleControllerScheduler::run(
            world,
            head,
            system,
            time,
            &mut ParticleControllerScheduleContext::default(),
        )
    }
}

// IDA: 0047C930's +0x54 for NiTransformController 006C3C40, NiVisController 006D4470, NiMultiTargetTransformController
// 006CF9D0 and NiControllerManager 006C4200, and 00700A90's for NiFlipController 006D1960 and
// NiTextureTransformController 006D6CE0. Skipped, no interpolator, interpolator failed and applied are original
// outcomes; the unsupported statuses are host stops. Completion: 100% of the dispatch; next is read after Update.
/// # Errors
/// A controller with no host time state.
pub fn update_node_controller(
    world: &mut World,
    controller: Entity,
    time: f32,
) -> Result<SceneControllerStep, StreamError> {
    use NodeControllerUpdateStatus as Update;
    let Some(&class) = world.get::<NiClass>(controller) else {
        return Ok(SceneControllerStep::Unsupported);
    };
    let status = match class {
        NiClass::TransformController => {
            update_transform_controller(world, controller, time, ControllerTimeMap::SharedCache)?
        }
        NiClass::VisController => {
            update_vis_controller(world, controller, time, ControllerTimeMap::SharedCache)?
        }
        NiClass::MultiTargetTransformController => {
            update_multi_target_transform_controller(world, controller, time)?
        }
        NiClass::ControllerManager => update_controller_manager(world, controller, time),
        // NiBSBoneLODController's vt+0x54 (A7D3D4+0x54) is the shared no-op 0068F970.
        NiClass::BoneLodController => Update::Skipped,
        NiClass::BhkBlendController => {
            super::bone_lod::update_bhk_blend_controller(world, controller, time)?;
            Update::Applied
        }
        NiClass::FlipController => {
            update_flip_controller(world, controller, time, ControllerTimeMap::SharedCache)?
        }
        NiClass::TextureTransformController => update_texture_transform_controller(
            world,
            controller,
            time,
            ControllerTimeMap::SharedCache,
        )?,
        _ => return Ok(SceneControllerStep::Unsupported),
    };
    if !matches!(
        status,
        Update::Skipped | Update::NoInterpolator | Update::InterpolatorFailed | Update::Applied
    ) {
        return Ok(SceneControllerStep::Failed { status });
    }
    let next = if class.is_texture_controller() {
        world
            .get::<TextureController>(controller)
            .ok_or(StreamError("texture controller update target missing"))?
            .time
            .next_34
    } else {
        world
            .get::<NodeController>(controller)
            .ok_or(StreamError("node controller update target missing"))?
            .time
            .next_34
    };
    Ok(SceneControllerStep::Completed { next, status })
}

/// IDA: N/A the node controllers of `update_node_controller` with no particle services, for an update outside the frame
/// (a reference's load): a particle system's chain reports the missing service.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeControllersOnly;

impl SceneUpdateRuntime for NodeControllersOnly {
    /// IDA: N/A `NodeControllersOnly`'s side of `SceneUpdateRuntime`.
    fn update_controller(
        &mut self,
        world: &mut World,
        controller: Entity,
        time: f32,
    ) -> Result<SceneControllerStep, StreamError> {
        update_node_controller(world, controller, time)
    }
}

/// IDA: N/A no controller classes bound: every nonnull controller head is a host stop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NoSceneControllers;

impl SceneUpdateRuntime for NoSceneControllers {
    /// IDA: N/A `NoSceneControllers`'s side of `SceneUpdateRuntime`.
    fn update_controller(
        &mut self,
        _: &mut World,
        _: Entity,
        _: f32,
    ) -> Result<SceneControllerStep, StreamError> {
        Ok(SceneControllerStep::Unsupported)
    }
}

/// IDA: N/A the C++ `SceneUpdateRuntime`: `particles` is the schedule's services, and the one 007155A0 cache is the
/// world's `ParticleControllerTimeCache`, which `ControllerTimeMap::SharedCache` and the scheduler both use.
pub struct SceneControllers<'r, 'a> {
    pub particles: &'r mut ParticleControllerScheduleContext<'a>,
    /// N/A 00591360, for the frames that hold the renderer.
    pub tile_textures: Option<&'r mut dyn TileTextureLoader>,
}

/// IDA: N/A 00591360's calls, from whoever holds the renderer.
pub trait TileTextureLoader {
    /// IDA: 00591360: the palette, FindFile, 00701E00(path, {6, 3, 2}, 1), and the palette's add unless the path holds "oading\load_". The texture's width and height. Completion: 100%.
    fn load(&mut self, world: &mut World, path: &str) -> Option<[u32; 2]>;
}

impl SceneUpdateRuntime for SceneControllers<'_, '_> {
    /// IDA: N/A `SceneControllers`'s side of `SceneUpdateRuntime`.
    fn tile_texture_size(&mut self, world: &mut World, path: &str) -> Option<Option<[u32; 2]>> {
        self.tile_textures
            .as_mut()
            .map(|loader| loader.load(world, path))
    }

    // IDA: 0047C930's +0x54 for NiTransformController 006C3C40, NiVisController 006D4470, NiMultiTargetTransformController
    // 006CF9D0 and NiControllerManager 006C4200, and 00700A90's for NiFlipController 006D1960 and
    // NiTextureTransformController 006D6CE0. Skipped, no interpolator, interpolator failed and applied are original
    // outcomes; the unsupported statuses are host stops. Completion: 100% of the dispatch; next is read after Update.
    // TODO: [ ] A nested update from a scheduler service: `run` holds the cache resource out of the world meanwhile.
    fn update_controller(
        &mut self,
        world: &mut World,
        controller: Entity,
        time: f32,
    ) -> Result<SceneControllerStep, StreamError> {
        update_node_controller(world, controller, time)
    }

    /// IDA: N/A `SceneControllers`'s side of `SceneUpdateRuntime`.
    fn run_particle_controllers(
        &mut self,
        world: &mut World,
        head: Option<Entity>,
        system: Entity,
        time: f32,
    ) -> Result<ParticleControllerScheduleResult, StreamError> {
        ParticleControllerScheduler::run(world, head, system, time, self.particles)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneUpdateStatus {
    Updated,
    /// A controller whose vtable +0x54 Update is not ported (a particle controller on an NiProperty).
    UnsupportedController,
    /// A bound controller Update returned a host-unsupported status.
    ControllerFailed,
    /// ParticleControllerScheduler::run stopped with an unsupported status.
    ParticleScheduleFailed,
    /// A stored +0x1C parent whose entity has gone.
    MissingParent,
    /// 0072BB30 skinned geometry bound.
    UnsupportedSkinnedBound,
    /// The original dereferences a null NiGeometry +0xB4 data pointer.
    UnsupportedMissingData,
    /// A nonnull +0xA8 collision owner (bhk classes register in a later phase).
    UnsupportedCollision,
    /// 0070A280 reads NiNode +0xCC, which only 0070A9C0 writes; the NiNode constructor leaves it uninitialized, so the
    /// host stops instead.
    UnsupportedRigidBound,
    /// IDA: N/A host stop: an entity outside the NiAVObject classes the C++ `SceneChildOwner` binds.
    UnsupportedClass,
}

/// IDA: N/A the C++ `SceneUpdateResult`. `controller_status` is the last Update's status on a chain, and
/// `particle_status` the schedule's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneUpdateResult {
    pub status: SceneUpdateStatus,
    pub controller_status: Option<NodeControllerUpdateStatus>,
    pub particle_status: Option<ParticleControllerScheduleStatus>,
}

impl SceneUpdateResult {
    pub const UPDATED: Self = Self::stopped(Status::Updated);

    /// IDA: N/A a status with no controller or particle status.
    #[must_use]
    pub const fn stopped(status: SceneUpdateStatus) -> Self {
        Self {
            status,
            controller_status: None,
            particle_status: None,
        }
    }
}

impl From<Status> for SceneUpdateResult {
    /// IDA: N/A Rust's `From`.
    fn from(status: Status) -> Self {
        Self::stopped(status)
    }
}

const UPDATED: SceneUpdateResult = SceneUpdateResult::UPDATED;

// IDA: 0072A0A0 NiBound equality: radius first, then x, y, z, all `==`. NaN compares unequal.
// Completion: 100% of the comparison order.
fn same_bound(left: &[f32; 4], right: &[f32; 4]) -> bool {
    right[3] == left[3] && right[0] == left[0] && right[1] == left[1] && right[2] == left[2]
}

// IDA: N/A the +0x98 list of an object, head first. An object not linked yet has none.
fn local_properties(world: &World, object: Entity) -> Vec<Option<Entity>> {
    world
        .get::<NiAvProperties>(object)
        .map(|properties| properties.0.clone())
        .unwrap_or_default()
}

// IDA: N/A a nonnull +0x1C whose entity has lost its world state stands for the C++ expired owner.
fn parent_alive(world: &World, parent: Entity) -> bool {
    world.get::<NiAvWorld>(parent).is_some()
}

// IDA: 00700A90 NiProperty vtable +0x50 and 0047C930's second half: for (c = +0x0C; c; c = c->next) c->vtable +0x54
// Update(time). The C++ port has one loop per owner family; the seam's dispatch makes those one loop here.
// Completion: 100% of the chain order; next is read after each Update returns. The Update bodies are the seam.
fn update_controller_chain(
    world: &mut World,
    head: Option<Entity>,
    time: f32,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneUpdateResult, StreamError> {
    let mut result = UPDATED;
    let mut cursor = head;
    while let Some(controller) = cursor {
        match runtime.update_controller(world, controller, time)? {
            SceneControllerStep::Completed { next, status } => {
                result.controller_status = Some(status);
                cursor = next;
            }
            SceneControllerStep::Failed { status } => {
                result.controller_status = Some(status);
                result.status = Status::ControllerFailed;
                return Ok(result);
            }
            SceneControllerStep::Unsupported => {
                result.status = Status::UnsupportedController;
                return Ok(result);
            }
        }
    }
    Ok(result)
}

// IDA: 0047C930 first half (updateProperties true): walk the +0x98 list from the +0x9C head; a nonnull property with a
// nonnull +0x0C controller runs its vtable +0x50 (00700A90). Completion: 100% of the order and null skipping.
// TODO: [ ] Bind property controllers beyond NiTexturingProperty; their +0x50 dispatch is not ported, as in the C++.
fn update_property_controllers(
    world: &mut World,
    properties: &[Option<Entity>],
    time: f32,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneUpdateResult, StreamError> {
    for &property in properties.iter().flatten() {
        let Some(head) = world
            .get::<NiObjectNet>(property)
            .and_then(|net| net.controller_0c)
        else {
            continue;
        };
        if world.get::<NiClass>(property) != Some(&NiClass::TexturingProperty) {
            return Ok(Status::UnsupportedController.into());
        }
        let step = update_controller_chain(world, Some(head), time, runtime)?;
        if step.status != Status::Updated {
            return Ok(step);
        }
    }
    Ok(UPDATED)
}

// IDA: 0047C930 NiAVObject::UpdateObjectControllers(time, updateProperties) for a NiNode-family owner.
// Completion: 100% of the properties-then-controllers order.
fn update_node_object_controllers(
    world: &mut World,
    node: Entity,
    time: f32,
    update_properties: bool,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneUpdateResult, StreamError> {
    if update_properties {
        let list = local_properties(world, node);
        let properties = update_property_controllers(world, &list, time, runtime)?;
        if properties.status != Status::Updated {
            return Ok(properties);
        }
    }
    let head = world
        .get::<NiObjectNet>(node)
        .and_then(|net| net.controller_0c);
    update_controller_chain(world, head, time, runtime)
}

// IDA: 0047C930 for a NiParticleSystem: properties, then the controller chain through the ordered particle scheduler
// (00753BE0 / 00750260 per +0x54). Completion: 100% of the chain order.
fn update_particle_object_controllers(
    world: &mut World,
    system: Entity,
    time: f32,
    update_properties: bool,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneUpdateResult, StreamError> {
    if update_properties {
        let list = local_properties(world, system);
        let properties = update_property_controllers(world, &list, time, runtime)?;
        if properties.status != Status::Updated {
            return Ok(properties);
        }
    }
    let head = world
        .get::<NiObjectNet>(system)
        .and_then(|net| net.controller_0c);
    if head.is_none() {
        return Ok(UPDATED);
    }
    let scheduled = runtime.run_particle_controllers(world, head, system, time)?;
    let mut result = UPDATED;
    result.particle_status = Some(scheduled.status);
    if scheduled.status != ParticleControllerScheduleStatus::Completed {
        result.status = Status::ParticleScheduleFailed;
    }
    Ok(result)
}

// IDA: 0070C120 NiAVObject +0x74 UpdateWorldData for leaf owners: compose the parent +0x64 world with the local +0x30
// transform, or copy local when +0x1C is null; collision is null for every linked owner (later phase).
// Completion: 100% of the branch and the 0x34-byte publication.
// TODO: [ ] Bind the collision vtable +0x50 callback (later phase).
fn update_leaf_world_data(world: &mut World, leaf: Entity, local: &NiTransform) -> Status {
    let parent = world.get::<NiAvParent>(leaf).and_then(|parent| parent.0);
    let transform = match parent {
        Some(parent) => {
            let Some(source) = world.get::<NiAvWorld>(parent) else {
                return Status::MissingParent;
            };
            compose_scene_transform(&source.transform, local)
        }
        None => *local,
    };
    world
        .get_mut::<NiAvWorld>(leaf)
        .expect("a NiAVObject has world state")
        .transform = transform;
    Status::Updated
}

// IDA: 00897510 bhkNiCollisionObject vt+0x50 UpdateWorldData, for bhkCollisionObject and bhkSPCollisionObject: with 00BA7A8C
// (the sync mode, 0 outside 0047C990's window, where it is 3) not 3, a +0x0C bit 6 runs vt+0x6C and vt+0x68 and clears the
// bit; otherwise bit 0 asks vt+0x74 (0089E9C0) and runs vt+0x68 or vt+0x64. Host substitution (D2): those virtuals move
// the hk rigid body to the node or the node to the body (0089EAE0, 0089EA70, 0089EE00), and with no hk object each does
// nothing, so the bit's clear is what remains. Completion: 100% with no hk object.
fn collision_update_world_data(world: &mut World, collision: Entity) {
    if let Some(mut object) =
        world.get_mut::<crate::bs::havok_stream::BhkSpCollisionObject>(collision)
        && object.flags_0c & 0x40 != 0
    {
        object.flags_0c &= 0xFFBF;
    }
}

// IDA: 00722AA0 NiGeometry +0x78 UpdateWorldBound: copy the old +0x20 bound, transform the +0xB4 data model bound (+0x0C)
// by +0x64 (0072A820), then, only while flag bit 0x80 is set, keep it when 0072A0A0 reports the new bound equal to the
// old one and clear it otherwise. Completion: 100% of the order.
fn update_geometry_world_bound(
    model_bound: Option<&[f32; 4]>,
    skinned: bool,
    transform: &NiTransform,
    world_bound: &mut [f32; 4],
    flags: &mut u16,
) -> Status {
    let previous = *world_bound;
    if skinned {
        return Status::UnsupportedSkinnedBound;
    }
    let Some(model_bound) = model_bound else {
        return Status::UnsupportedMissingData;
    };
    *world_bound = transform_scene_bound(model_bound, transform);
    if *flags & 0x0080 != 0 {
        if same_bound(world_bound, &previous) {
            *flags |= 0x0080;
        } else {
            *flags &= 0xFF7F;
        }
    }
    Status::Updated
}

// IDA: 00749310 NiParticleSystem +0x74's tail, after 0070C120: when +0xC0 (in world space) is set, +0x88 translate =
// B3F9A8 (zero-initialized BSS NiPoint3; no static writer, 407 readers) and +0x64 rotation = g_niMatrix3Identity. World
// scale is untouched. Completion: 100% of the stores.
pub fn particle_world_data_in_world_space(transform: &mut NiTransform) {
    transform.translation = [0.0, 0.0, 0.0];
    transform.rotation = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
}

// IDA: 00749550 NiParticleSystem +0x78's tail, after 00722AA0: when +0xC0 is set the bound is the +0xB4 data model bound
// (+0x0C..+0x18) with center and radius scaled by the +0x94 world scale (x, y, z stored, then radius).
// Precision: one float product per store. Completion: 100% of the order.
#[must_use]
pub fn particle_world_bound_in_world_space(model: &[f32; 4], scale: f32) -> [f32; 4] {
    let x = model[0] * scale;
    let y = model[1] * scale;
    let z = scale * model[2];
    let radius = model[3] * scale;
    [x, y, z, radius]
}

// IDA: 00749310 NiParticleSystem +0x74: 0070C120, then the world-space tail. Completion: 100% of the order.
fn update_particle_world_data(world: &mut World, system: Entity) -> Status {
    let state = world
        .get::<ParticleSystem>(system)
        .expect("a NiParticleSystem has its component");
    let local = state.local_transform_30;
    let in_world_space = state.in_world_space_c0;
    let status = update_leaf_world_data(world, system, &local);
    if status != Status::Updated {
        return status;
    }
    if in_world_space {
        particle_world_data_in_world_space(
            &mut world
                .get_mut::<NiAvWorld>(system)
                .expect("just stored")
                .transform,
        );
    }
    Status::Updated
}

// IDA: 00749550 NiParticleSystem +0x78: 00722AA0 over the live bound and flags, then the world-space tail. A NiParticleSystem
// has no +0xB8 skin: it links only with a null one. Completion: 100% of the order.
fn update_particle_world_bound(world: &mut World, system: Entity) -> Status {
    let state = world
        .get::<ParticleSystem>(system)
        .expect("a NiParticleSystem has its component");
    let model = state
        .data_b4
        .and_then(|data| world.get::<ParticleData>(data))
        .map(|data| data.bounds);
    let in_world_space = state.in_world_space_c0;
    let mut flags = state.effective_av_flags_18;
    let av = world
        .get::<NiAvWorld>(system)
        .expect("a NiAVObject has world state");
    let transform = av.transform;
    let mut bound = av.bound;
    let status =
        update_geometry_world_bound(model.as_ref(), false, &transform, &mut bound, &mut flags);
    if status == Status::Updated
        && in_world_space
        && let Some(model) = &model
    {
        bound = particle_world_bound_in_world_space(model, transform.scale);
    }
    world
        .get_mut::<NiAvWorld>(system)
        .expect("checked above")
        .bound = bound;
    world
        .get_mut::<ParticleSystem>(system)
        .expect("checked above")
        .effective_av_flags_18 = flags;
    status
}

// IDA: 007197C0 NiDirectionalLight +0x74: 0070C120, then +0x108 / +0x10C / +0x110 = world rotation entries 0, 3, 6 (first
// column), then +++0xB8. Completion: 100% of the store order.
fn update_light_world_data(world: &mut World, light: Entity) -> Status {
    let local = world
        .get::<DirectionalLight>(light)
        .expect("a NiDirectionalLight has its component")
        .payload
        .av
        .local_transform;
    let status = update_leaf_world_data(world, light, &local);
    if status != Status::Updated {
        return status;
    }
    let rotation = world
        .get::<NiAvWorld>(light)
        .expect("just stored")
        .transform
        .rotation;
    let mut state = world
        .get_mut::<DirectionalLight>(light)
        .expect("checked above");
    state.direction_108 = [rotation[0], rotation[3], rotation[6]];
    state.unknown_b8 = state.unknown_b8.wrapping_add(1);
    Status::Updated
}

// IDA: 00725410 NiPointLight +0x74: 0070C120, then ++(+0xB8); +0x78 is 0060D0A0, a bare return. Completion: 100%.
fn update_point_light_world_data(world: &mut World, light: Entity) -> Status {
    let local = world
        .get::<PointLight>(light)
        .expect("a NiPointLight has its component")
        .local_transform;
    let status = update_leaf_world_data(world, light, &local);
    if status != Status::Updated {
        return status;
    }
    let mut state = world.get_mut::<PointLight>(light).expect("checked above");
    state.revision_b8 = state.revision_b8.wrapping_add(1);
    Status::Updated
}

// IDA: 00722340 NiBillboardNode +0x78: when the +0xB8 child count is nonzero, grow the bound to be centered on the world
// translation: radius += |center - translate| ((dy*dy + dx*dx) + dz*dz, sqrt), center = translate. Precision: 24-bit per
// step on the main thread after CreateDevice; __CIsqrt keeps the precision bits. Completion: 100% of the
// order (x, y stored, z, then radius). It does not merge the children: only the downward pass runs 0070A360 first, so a
// repeated upward pass adds nothing unless the translation moved.
fn update_billboard_world_bound(world: &mut World, node: Entity) {
    let nonnull = world
        .get::<NiNodeChildren>(node)
        .expect("a NiNode has child slots")
        .slots_ac
        .iter()
        .flatten()
        .count();
    if nonnull == 0 {
        return;
    }
    let mut state = world
        .get_mut::<NiAvWorld>(node)
        .expect("a NiNode has world state");
    let translate = state.transform.translation;
    let bound = state.bound;
    let dx = bound[0] - translate[0];
    let dy = bound[1] - translate[1];
    let dz = bound[2] - translate[2];
    let squared = dy * dy + dx * dx + dz * dz;
    let distance = squared.sqrt();
    let radius = distance + bound[3];
    state.bound = [translate[0], translate[1], translate[2], radius];
}

// IDA: N/A host accessor for a child's +0x18 AV flags word. `None` for a class with no bound flags field.
// Completion: 100% of the typed live field per bound class.
fn child_flags(world: &World, child: Entity) -> Option<u16> {
    match *world.get::<NiClass>(child)? {
        NiClass::Node | NiClass::BillboardNode => world
            .get::<SceneNodePayload>(child)
            .map(|payload| payload.effective_av_flags),
        NiClass::TriShape | NiClass::TriStrips => world
            .get::<MeshTriShapePayload>(child)
            .map(|payload| payload.effective_av_flags),
        NiClass::DirectionalLight => world
            .get::<DirectionalLight>(child)
            .map(|light| light.payload.av.effective_av_flags),
        NiClass::PointLight => world
            .get::<PointLight>(child)
            .map(|light| light.av_flags_18),
        NiClass::Camera => world
            .get::<SceneCamera>(child)
            .map(|camera| camera.av_flags_18),
        NiClass::ParticleSystem => world
            .get::<ParticleSystem>(child)
            .map(|system| system.effective_av_flags_18),
        _ => None,
    }
}

// IDA: 0070A0D0 / 0070A190 inline child bound merge: nonzero child radius is copied when the node radius is zero,
// otherwise merged by 0072A6B0. Completion: 100% of the branch order.
fn merge_child_bound(world: &mut World, node: Entity, child: Entity) -> Result<(), StreamError> {
    let Some(bound) = world.get::<NiAvWorld>(child).map(|state| state.bound) else {
        return Ok(());
    };
    if bound[3] == 0.0 {
        return Ok(());
    }
    let mut state = world
        .get_mut::<NiAvWorld>(node)
        .expect("a NiNode has world state");
    if state.bound[3] == 0.0 {
        state.bound = bound;
        Ok(())
    } else {
        merge_ni_sphere(&mut state.bound, &bound)
    }
}

// IDA: 0070A360 NiNode +0x78 UpdateWorldBound: radius = 0, then each nonnull child with a nonzero radius is copied (node
// radius zero) or merged (0072A6B0). Completion: 100% of the order.
pub fn update_node_world_bound(world: &mut World, node: Entity) -> Result<(), StreamError> {
    world
        .get_mut::<NiAvWorld>(node)
        .expect("a NiNode has world state")
        .bound[3] = 0.0;
    let children = child_slots(world, node);
    for child in children.into_iter().flatten() {
        merge_child_bound(world, node, child)?;
    }
    Ok(())
}

// IDA: N/A a snapshot of the +0xAC slots. The passes do not change them while walking.
fn child_slots(world: &World, node: Entity) -> Vec<Option<Entity>> {
    world
        .get::<NiNodeChildren>(node)
        .expect("a NiNode has child slots")
        .slots_ac
        .clone()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pass {
    Downward,
    Selected,
    Rigid,
}

// IDA: 0070A0D0 NiNode +0x60 / 0070A190 +0x64 / 0070A280 +0x68.
// downward: 0047C930(time, 1) when updating, +0x74, radius 0, each nonnull child +0x60 then the inline bound merge.
// selected: 0047C930(time, flag 0x08), +0x74 only under flag 0x04, radius 0, children with flag 0x02 take +0x68 (flag
// 0x10) or +0x64, then the merge. rigid: 0047C930(time, 0x08), under 0x04 +0x74 then bound = 0072A820(+0xCC, world),
// children with 0x02 take +0x68 with no merge. Completion: 100% of the per-pass order.
// TODO: [ ] Bind the collision callback inside +0x74 (later phase).
fn update_node_pass(
    world: &mut World,
    node: Entity,
    time: f32,
    update_controllers: bool,
    pass: Pass,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneUpdateResult, StreamError> {
    let flags = world
        .get::<SceneNodePayload>(node)
        .expect("a NiNode has a payload")
        .effective_av_flags;
    let properties = pass == Pass::Downward || flags & 0x0008 != 0;
    if pass != Pass::Downward || update_controllers {
        let controllers = update_node_object_controllers(world, node, time, properties, runtime)?;
        if controllers.status != Status::Updated {
            return Ok(controllers);
        }
    }
    let world_data = pass == Pass::Downward || flags & 0x0004 != 0;
    if world_data {
        // 0070C120 reads the raw parent; a parent the host no longer holds stops first.
        let parent = world.get::<NiAvParent>(node).and_then(|parent| parent.0);
        if parent.is_some_and(|parent| !parent_alive(world, parent)) {
            return Ok(Status::MissingParent.into());
        }
        let collision = world
            .get::<super::av_object::NiAvCollision>(node)
            .map(|collision| collision.0)
            .filter(|&collision| {
                world
                    .get::<crate::bs::havok_stream::BhkSpCollisionObject>(collision)
                    .is_some()
            });
        let updated = match collision {
            Some(collision) => update_world_transform(
                world,
                node,
                Some(&mut |world: &mut World| collision_update_world_data(world, collision)),
            )?,
            None => update_world_transform(world, node, None)?,
        };
        if !updated {
            return Ok(Status::UnsupportedCollision.into());
        }
    }
    let children = child_slots(world, node);
    if pass == Pass::Rigid {
        if world_data {
            let state = world.get::<NiAvWorld>(node).expect("just stored");
            let Some(combined) = state.combined_bound else {
                return Ok(Status::UnsupportedRigidBound.into());
            };
            let bound = transform_scene_bound(&combined, &state.transform);
            world.get_mut::<NiAvWorld>(node).expect("just stored").bound = bound;
        }
        for child in children.into_iter().flatten() {
            let Some(child_word) = child_flags(world, child) else {
                return Ok(Status::UnsupportedClass.into());
            };
            if child_word & 0x0002 != 0 {
                let result = update_pass(world, child, time, false, Pass::Rigid, runtime)?;
                if result.status != Status::Updated {
                    return Ok(result);
                }
            }
        }
        return Ok(UPDATED);
    }
    world
        .get_mut::<NiAvWorld>(node)
        .expect("a NiNode has world state")
        .bound[3] = 0.0;
    for child in children.into_iter().flatten() {
        if pass == Pass::Downward {
            let result = update_pass(world, child, time, update_controllers, pass, runtime)?;
            if result.status != Status::Updated {
                return Ok(result);
            }
        } else {
            let Some(child_word) = child_flags(world, child) else {
                return Ok(Status::UnsupportedClass.into());
            };
            if child_word & 0x0002 != 0 {
                let next = if child_word & 0x0010 != 0 {
                    Pass::Rigid
                } else {
                    Pass::Selected
                };
                let result = update_pass(world, child, time, false, next, runtime)?;
                if result.status != Status::Updated {
                    return Ok(result);
                }
            }
        }
        merge_child_bound(world, node, child)?;
    }
    Ok(UPDATED)
}

// IDA: 007219C0 / 007219F0 / 00721A20 NiBillboardNode +0x60 / +0x64 / +0x68: the downward pass first sets +0xDC bit 0x08;
// every pass stores the time to +0xE0, runs the NiNode pass, then +0x78 (00722340). Completion: 100% of the store and
// call order.
fn update_billboard_pass(
    world: &mut World,
    node: Entity,
    time: f32,
    update_controllers: bool,
    pass: Pass,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneUpdateResult, StreamError> {
    let mut billboard = world
        .get_mut::<BillboardNode>(node)
        .expect("a NiBillboardNode has its +0xDC fields");
    if pass == Pass::Downward {
        billboard.effective_mode_flags_dc |= 0x0008;
    }
    billboard.unknown_e0 = time;
    let result = update_node_pass(world, node, time, update_controllers, pass, runtime)?;
    if result.status != Status::Updated {
        return Ok(result);
    }
    update_billboard_world_bound(world, node);
    Ok(UPDATED)
}

// IDA: 00707580 leaf +0x60 (0047C930(time, 1) when updating, +0x74, +0x78) and 007075B0 leaf +0x64 / +0x68 (0047C930(time,
// flag 0x08), then +0x74 and +0x78 only under flag 0x04). Geometry +0x78 is 00722AA0; the light +0x78 is a nullsub
// (0060D0A0). `controllers` is the class's 0047C930, given updateProperties. Completion: 100% of the order.
fn update_leaf_pass(
    world: &mut World,
    flags: u16,
    update_controllers: bool,
    pass: Pass,
    controllers: impl FnOnce(&mut World, bool) -> Result<SceneUpdateResult, StreamError>,
    world_data: impl FnOnce(&mut World) -> Status,
    world_bound: impl FnOnce(&mut World) -> Status,
) -> Result<SceneUpdateResult, StreamError> {
    let run_controllers = pass != Pass::Downward || update_controllers;
    let properties = pass == Pass::Downward || flags & 0x0008 != 0;
    if run_controllers {
        let result = controllers(world, properties)?;
        if result.status != Status::Updated {
            return Ok(result);
        }
    }
    if pass != Pass::Downward && flags & 0x0004 == 0 {
        return Ok(UPDATED);
    }
    let status = world_data(world);
    if status != Status::Updated {
        return Ok(status.into());
    }
    let status = world_bound(world);
    if status != Status::Updated {
        return Ok(status.into());
    }
    Ok(UPDATED)
}

// IDA: 0047C930 for a leaf that links only with a null +0x0C head (geometry, lights and cameras): just the property half.
fn leaf_property_controllers(
    world: &mut World,
    leaf: Entity,
    properties: bool,
    time: f32,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneUpdateResult, StreamError> {
    if !properties {
        return Ok(UPDATED);
    }
    let list = local_properties(world, leaf);
    update_property_controllers(world, &list, time, runtime)
}

// IDA: 0072BB30 NiSkinInstance_UpdateModelBound(out): the +0x08 data's bone 0 sphere (+0x44 records, 0x4C apart, sphere at
// +0x34) through 0072A820 by bone 0's (+0x14) world transform, then each later bone's merged in (0072A6B0); the result
// through 0072A820 by 0053D7A0(data +0x0C, 00718A80(the +0x10 root parent's world transform)). `None` for a skin the host
// can't read (a missing data, root or bone, or no bones: the original reads past them). Completion: 100% of the order.
// TODO: [ ] The IO thread's 53-bit precision: the model loader's finalize runs it there, and the helpers round to 24 bits.
fn skin_instance_bound(world: &World, skin: Entity) -> Result<Option<[f32; 4]>, StreamError> {
    use super::mesh_skin_owners::{MeshSkinData, MeshSkinInstance};
    let Some(instance) = world.get::<MeshSkinInstance>(skin) else {
        return Ok(None);
    };
    let Some(data) = instance
        .data_08
        .and_then(|data| world.get::<MeshSkinData>(data))
    else {
        return Ok(None);
    };
    let Some(root) = instance
        .root_parent_owner_10
        .and_then(|root| world.get::<NiAvWorld>(root))
    else {
        return Ok(None);
    };
    let bone_world = |index: usize| {
        instance
            .bone_owners_14
            .get(index)
            .copied()
            .flatten()
            .and_then(|bone| world.get::<NiAvWorld>(bone))
            .map(|state| state.transform)
    };
    let (Some(first), Some(first_world)) = (data.bones.first(), bone_world(0)) else {
        return Ok(None);
    };
    let mut bound = transform_scene_bound(&first.bounding_sphere, &first_world);
    for (index, bone) in data.bones.iter().enumerate().skip(1) {
        let Some(transform) = bone_world(index) else {
            return Ok(None);
        };
        merge_ni_sphere(
            &mut bound,
            &transform_scene_bound(&bone.bounding_sphere, &transform),
        )?;
    }
    let to_skin =
        compose_scene_transform(&data.transform_0c, &invert_scene_transform(&root.transform));
    Ok(Some(transform_scene_bound(&bound, &to_skin)))
}

// IDA: 00722AA0 for a NiTriShape or NiTriStrips, over the entity's components. NiTriStrips links only with a null skin
// instance in the host, so only NiTriShape can report one. Completion: 100% of the reads.
fn update_entity_geometry_world_bound(world: &mut World, shape: Entity) -> Status {
    let geometry = *world
        .get::<NiGeometry>(shape)
        .expect("a NiGeometry has its links");
    let model = match geometry.skin_b8 {
        Some(skin) => match skin_instance_bound(world, skin) {
            Ok(Some(bound)) => Some(bound),
            Ok(None) => return Status::UnsupportedSkinnedBound,
            Err(_) => return Status::UnsupportedSkinnedBound,
        },
        None => geometry
            .data_b4
            .and_then(|data| geometry_data_base(world, data))
            .map(|base| base.bounds_0c),
    };
    let state = world
        .get::<NiAvWorld>(shape)
        .expect("a NiAVObject has world state");
    let transform = state.transform;
    let mut bound = state.bound;
    let mut flags = world
        .get::<MeshTriShapePayload>(shape)
        .expect("a NiGeometry has a payload")
        .effective_av_flags;
    let status =
        update_geometry_world_bound(model.as_ref(), false, &transform, &mut bound, &mut flags);
    world
        .get_mut::<NiAvWorld>(shape)
        .expect("checked above")
        .bound = bound;
    world
        .get_mut::<MeshTriShapePayload>(shape)
        .expect("checked above")
        .effective_av_flags = flags;
    status
}

// IDA: vtable +0x60 / +0x64 / +0x68 dispatch over the bound NiAVObject classes.
// Completion: 100% of the per-class slot targets.
fn update_pass(
    world: &mut World,
    object: Entity,
    time: f32,
    update_controllers: bool,
    pass: Pass,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneUpdateResult, StreamError> {
    let Some(&class) = world.get::<NiClass>(object) else {
        return Ok(Status::UnsupportedClass.into());
    };
    match class {
        NiClass::Node => update_node_pass(world, object, time, update_controllers, pass, runtime),
        NiClass::BillboardNode => {
            update_billboard_pass(world, object, time, update_controllers, pass, runtime)
        }
        NiClass::TriShape | NiClass::TriStrips => {
            let payload = world
                .get::<MeshTriShapePayload>(object)
                .expect("a NiGeometry has a payload");
            let flags = payload.effective_av_flags;
            let local = payload.local_transform;
            update_leaf_pass(
                world,
                flags,
                update_controllers,
                pass,
                |world, properties| {
                    leaf_property_controllers(world, object, properties, time, runtime)
                },
                |world| update_leaf_world_data(world, object, &local),
                |world| update_entity_geometry_world_bound(world, object),
            )
        }
        NiClass::ParticleSystem => {
            // 007492B0 (+0x60) runs +0x74 before 00707580; 007492E0 (+0x64 / +0x68) runs +0x74 under flag 0x04 before 007075B0.
            let flags = world
                .get::<ParticleSystem>(object)
                .expect("a NiParticleSystem has its component")
                .effective_av_flags_18;
            if pass == Pass::Downward || flags & 0x0004 != 0 {
                let status = update_particle_world_data(world, object);
                if status != Status::Updated {
                    return Ok(status.into());
                }
            }
            update_leaf_pass(
                world,
                flags,
                update_controllers,
                pass,
                |world, properties| {
                    update_particle_object_controllers(world, object, time, properties, runtime)
                },
                |world| update_particle_world_data(world, object),
                |world| update_particle_world_bound(world, object),
            )
        }
        NiClass::Camera => {
            // NiCamera A7E4C4: +0x60 00707580, +0x64 / +0x68 007075B0, +0x74 0070D430 (0070C120 then 0070CC90), +0x78
            // 0070CC70 (bound center = world translation x, y, z; the radius is untouched).
            let camera = world
                .get::<SceneCamera>(object)
                .expect("a NiCamera has its component");
            let flags = camera.av_flags_18;
            let local = camera.local_transform_30;
            update_leaf_pass(
                world,
                flags,
                update_controllers,
                pass,
                |world, properties| {
                    leaf_property_controllers(world, object, properties, time, runtime)
                },
                |world| {
                    let status = update_leaf_world_data(world, object, &local);
                    if status != Status::Updated {
                        return status;
                    }
                    let transform = world
                        .get::<NiAvWorld>(object)
                        .expect("just stored")
                        .transform;
                    update_camera_world_to_clip(
                        &mut world.get_mut::<SceneCamera>(object).expect("checked above"),
                        &transform,
                    );
                    Status::Updated
                },
                |world| {
                    let mut state = world.get_mut::<NiAvWorld>(object).expect("just stored");
                    state.bound[0] = state.transform.translation[0];
                    state.bound[1] = state.transform.translation[1];
                    state.bound[2] = state.transform.translation[2];
                    Status::Updated
                },
            )
        }
        NiClass::DirectionalLight => {
            let flags = world
                .get::<DirectionalLight>(object)
                .expect("a NiDirectionalLight has its component")
                .payload
                .av
                .effective_av_flags;
            update_leaf_pass(
                world,
                flags,
                update_controllers,
                pass,
                |world, properties| {
                    leaf_property_controllers(world, object, properties, time, runtime)
                },
                |world| update_light_world_data(world, object),
                |_| Status::Updated,
            )
        }
        NiClass::PointLight => {
            let flags = world
                .get::<PointLight>(object)
                .expect("a NiPointLight has its component")
                .av_flags_18;
            update_leaf_pass(
                world,
                flags,
                update_controllers,
                pass,
                |world, properties| {
                    leaf_property_controllers(world, object, properties, time, runtime)
                },
                |world| update_point_light_world_data(world, object),
                |_| Status::Updated,
            )
        }
        _ => Ok(Status::UnsupportedClass.into()),
    }
}

// IDA: 00709EC0 NiNode +0x94 UpdateUpwardPass: read +0x1C, run +0x78 (0070A360, or 00722340 for NiBillboardNode), then
// recurse into the parent. Completion: 100% of the order (the parent is read before the bound update).
fn update_upward_pass(
    world: &mut World,
    start: Option<Entity>,
) -> Result<SceneUpdateResult, StreamError> {
    let mut current = start;
    while let Some(node) = current {
        let parent = world.get::<NiAvParent>(node).and_then(|parent| parent.0);
        if world.get::<NiClass>(node) == Some(&NiClass::BillboardNode) {
            update_billboard_world_bound(world, node);
        } else {
            update_node_world_bound(world, node)?;
        }
        if parent.is_some_and(|parent| !parent_alive(world, parent)) {
            return Ok(Status::MissingParent.into());
        }
        current = parent;
    }
    Ok(UPDATED)
}

// IDA: 0072A820 NiBound transform: center = 007101F0(R, c) * s + t (x/y stored, then z), radius = r * s.
// Precision: 24-bit per step on the main thread after CreateDevice. Completion: 100% of the operand and store order.
#[must_use]
pub fn transform_scene_bound(bound: &[f32; 4], transform: &NiTransform) -> [f32; 4] {
    let rotated = rotate_vector(&transform.rotation, &[bound[0], bound[1], bound[2]]);
    let scale = transform.scale;
    let sx = rotated[0] * scale;
    let sy = rotated[1] * scale;
    let sz = scale * rotated[2];
    let x = transform.translation[0] + sx;
    let y = transform.translation[1] + sy;
    let z = transform.translation[2] + sz;
    let radius = bound[3] * scale;
    [x, y, z, radius]
}

// IDA: 0070A9C0 NiNode vtable +0x50: recurse into each nonnull child's +0x50 (a nullsub for leaf NiAVObject classes), then
// store +0xCC = 0072A820(world bound +0x20, 00718A80 inverse of the +0x64 world transform).
// Completion: 100% of the order. TODO: [ ] Bind its original callers.
pub fn update_scene_node_model_bound(world: &mut World, node: Entity) {
    for child in child_slots(world, node).into_iter().flatten() {
        if world
            .get::<NiClass>(child)
            .is_some_and(|class| class.is_node())
        {
            update_scene_node_model_bound(world, child);
        }
    }
    let mut state = world
        .get_mut::<NiAvWorld>(node)
        .expect("a NiNode has world state");
    state.combined_bound = Some(transform_scene_bound(
        &state.bound,
        &invert_scene_transform(&state.transform),
    ));
}

// IDA: N/A the +0x1C read after a downward pass, then the parent's upward pass.
fn run_upward_from(world: &mut World, object: Entity) -> Result<SceneUpdateResult, StreamError> {
    let parent = world.get::<NiAvParent>(object).and_then(|parent| parent.0);
    if parent.is_some_and(|parent| !parent_alive(world, parent)) {
        return Ok(Status::MissingParent.into());
    }
    update_upward_pass(world, parent)
}

// IDA: 00707370 NiAVObject::Update(time, updateControllers): vtable +0x60 UpdateDownwardPass, then, when +0x1C is nonnull,
// the parent's +0x94 UpdateUpwardPass (00709EC0 NiNode: +0x78 bound, then its own parent).
// Completion: 100% of the dispatch for the bound NiAVObject classes.
// TODO: [ ] Bind camera-dependent culling consumers of the result.
pub fn update_scene_object(
    world: &mut World,
    object: Entity,
    time: f32,
    update_controllers: bool,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneUpdateResult, StreamError> {
    let result = update_pass(
        world,
        object,
        time,
        update_controllers,
        Pass::Downward,
        runtime,
    )?;
    if result.status != Status::Updated {
        return Ok(result);
    }
    run_upward_from(world, object)
}

// IDA: 007073A0 NiAVObject::UpdateSelected(time): vtable +0x64 UpdateSelectedDownwardPass, then the parent's +0x94
// UpdateUpwardPass. Completion: 100% of the dispatch.
// TODO: [ ] Bind the BS callers that select it.
pub fn update_scene_object_selected(
    world: &mut World,
    object: Entity,
    time: f32,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneUpdateResult, StreamError> {
    let result = update_pass(world, object, time, false, Pass::Selected, runtime)?;
    if result.status != Status::Updated {
        return Ok(result);
    }
    run_upward_from(world, object)
}

// IDA: vtable +0x4C UpdateControllers: NiNode 0070A310 runs 0047C930(time, 1) then recurses into each nonnull child slot;
// NiAVObject 007075F0 runs 0047C930(time, 1) only. Completion: 100% of the recursion order.
// TODO: [ ] Bind the original callers (animation / time managers).
pub fn update_scene_object_controllers(
    world: &mut World,
    object: Entity,
    time: f32,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneUpdateResult, StreamError> {
    let Some(&class) = world.get::<NiClass>(object) else {
        return Ok(Status::UnsupportedClass.into());
    };
    match class {
        NiClass::Node | NiClass::BillboardNode => {
            let own = update_node_object_controllers(world, object, time, true, runtime)?;
            if own.status != Status::Updated {
                return Ok(own);
            }
            for child in child_slots(world, object).into_iter().flatten() {
                let result = update_scene_object_controllers(world, child, time, runtime)?;
                if result.status != Status::Updated {
                    return Ok(result);
                }
            }
            Ok(UPDATED)
        }
        NiClass::ParticleSystem => {
            update_particle_object_controllers(world, object, time, true, runtime)
        }
        NiClass::TriShape
        | NiClass::TriStrips
        | NiClass::DirectionalLight
        | NiClass::PointLight
        | NiClass::Camera => leaf_property_controllers(world, object, true, time, runtime),
        _ => Ok(Status::UnsupportedClass.into()),
    }
}

/// IDA: 0047C990(root, time, accum) with 00BA7A8C = 3 around it: each controller in the root's chain but a
/// NiMultiTargetTransformController runs its vt+0x54 Update(time); the root's vt+0x74 (0070C120); with an accum root, its
/// parent's vt+0x74 unless the parent is the root, 0047C930(accum, time, 1) and the accum root's vt+0x74. Completion: 100% of the
/// order; 00BA7A8C's mode isn't kept.
/// # Errors
/// As the controllers' updates.
pub fn update_root_and_accum(
    world: &mut World,
    root: Entity,
    time: f32,
    accum: Option<Entity>,
    runtime: &mut dyn SceneUpdateRuntime,
) -> Result<SceneUpdateResult, StreamError> {
    let mut cursor = world
        .get::<NiObjectNet>(root)
        .and_then(|net| net.controller_0c);
    while let Some(controller) = cursor {
        if world.get::<NiClass>(controller) == Some(&NiClass::MultiTargetTransformController) {
            cursor = world
                .get::<super::transform_controller::NodeController>(controller)
                .and_then(|state| state.time.next_34);
            continue;
        }
        match runtime.update_controller(world, controller, time)? {
            SceneControllerStep::Completed { next, .. } => cursor = next,
            SceneControllerStep::Failed { status } => {
                return Ok(SceneUpdateResult {
                    status: Status::ControllerFailed,
                    controller_status: Some(status),
                    particle_status: None,
                });
            }
            SceneControllerStep::Unsupported => return Ok(Status::UnsupportedController.into()),
        }
    }
    // 0070C120 with the +0xA8 collision's world data, as `update_node_pass`'s.
    let world_data = |world: &mut World, node: Entity| -> Result<bool, StreamError> {
        let collision = world
            .get::<super::av_object::NiAvCollision>(node)
            .map(|collision| collision.0)
            .filter(|&collision| {
                world
                    .get::<crate::bs::havok_stream::BhkSpCollisionObject>(collision)
                    .is_some()
            });
        match collision {
            Some(collision) => update_world_transform(
                world,
                node,
                Some(&mut |world: &mut World| collision_update_world_data(world, collision)),
            ),
            None => update_world_transform(world, node, None),
        }
    };
    if !world_data(world, root)? {
        return Ok(Status::UnsupportedCollision.into());
    }
    if let Some(accum) = accum {
        let parent = world.get::<NiAvParent>(accum).and_then(|parent| parent.0);
        if let Some(parent) = parent
            && parent != root
            && !world_data(world, parent)?
        {
            return Ok(Status::UnsupportedCollision.into());
        }
        let controllers = update_node_object_controllers(world, accum, time, true, runtime)?;
        if controllers.status != Status::Updated {
            return Ok(controllers);
        }
        if !world_data(world, accum)? {
            return Ok(Status::UnsupportedCollision.into());
        }
    }
    Ok(UPDATED)
}

#[cfg(test)]
mod tests {
    use super::super::av_object::{av_has_property_controller, set_leaf_selective_update_flags};
    use super::super::controller_sequence::ControllerManager;
    use super::super::controller_time::ParticleControllerTimeCache;
    use super::super::extra_data::StringExtraData;
    use super::super::mesh_geometry::{MeshGeometryData, MeshTriShapeData, spawn_mesh_geometry};
    use super::super::object_net::{
        NiExtraDataName, ObjectNetPending, object_net_extras_supported,
        resolve_object_net_extra_links,
    };
    use super::super::particle_controller::ParticleUpdateController;
    use super::super::particle_system::spawn_particle_system;
    use super::super::properties::{VertexColorProperty, ZBufferProperty};
    use super::super::scene_derived::{spawn_billboard_node, spawn_directional_light};
    use super::super::scene_node::{
        NiEffectAffectedNodes, NiNodeEffects, SceneNodeLinkStatus, link_scene_node,
        spawn_scene_node,
    };
    use super::super::stream_objects::{NULL_REFERENCE, StreamObjectTable};
    use super::super::texture::{
        FlipController, TextureTransformController, TexturingProperty, spawn_texture_controller,
    };
    use super::super::transform_controller::{
        MultiTargetTransformController, TransformController, VisController, spawn_node_controller,
    };
    use super::*;

    fn translated(x: f32, y: f32, z: f32, scale: f32) -> NiTransform {
        NiTransform {
            rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            translation: [x, y, z],
            scale,
        }
    }

    struct LinkFixture {
        parent: Entity,
        child: Entity,
        shape: Entity,
        zbuffer: Entity,
        vertex: Entity,
        light: Entity,
        table: StreamObjectTable,
    }

    /// IDA: N/A host table: 0 parent, 1 child node, 2 child shape, 3 ZBuffer, 4 VertexColor, 5 named string extra, 6
    /// directional light.
    fn link_fixture(world: &mut World, version: u32) -> LinkFixture {
        let child = spawn_scene_node(world, SceneNodePayload::default());
        let shape = spawn_mesh_geometry(world, NiClass::TriShape, MeshTriShapePayload::default());
        let zbuffer = world
            .spawn((
                NiClass::ZBufferProperty,
                NiObjectNet::default(),
                ZBufferProperty::default(),
            ))
            .id();
        let vertex = world
            .spawn((
                NiClass::VertexColorProperty,
                NiObjectNet::default(),
                VertexColorProperty::default(),
            ))
            .id();
        let extra = world
            .spawn((
                NiClass::StringExtraData,
                NiExtraDataName(Some(b"Prn".to_vec())),
                StringExtraData {
                    value: Some(b"Weapon".to_vec()),
                },
            ))
            .id();
        let light = spawn_directional_light(world);
        let net = ObjectNetPending {
            extra_indices: vec![5],
            ..ObjectNetPending::default()
        };
        let parent = spawn_scene_node(
            world,
            SceneNodePayload {
                net,
                stream_version: version,
                property_indices: vec![3, NULL_REFERENCE, 4],
                child_indices: vec![1, NULL_REFERENCE, 2],
                effect_indices: vec![6],
                ..SceneNodePayload::default()
            },
        );
        let mut table = StreamObjectTable::default();
        table.push("NiNode", 0x0070_BA70, parent);
        table.push("NiNode", 0x0070_BA70, child);
        table.push("NiTriShape", 0x0071_75C0, shape);
        table.push("NiZBufferProperty", 0x0070_6CB0, zbuffer);
        table.push("NiVertexColorProperty", 0x0070_6530, vertex);
        table.push("NiStringExtraData", 0x0071_6B20, extra);
        table.push("NiDirectionalLight", 0x0071_97F0, light);
        LinkFixture {
            parent,
            child,
            shape,
            zbuffer,
            vertex,
            light,
            table,
        }
    }

    fn link(world: &mut World, fixture: &LinkFixture) -> SceneNodeLinkStatus {
        link_scene_node(
            world,
            fixture.parent,
            &[Some(fixture.child), None, Some(fixture.shape)],
            &[Some(fixture.light)],
            &fixture.table,
        )
        .unwrap()
    }

    /// Ported from `verify_mixed_link_and_property_order`: 007081B0 AddHead (>= 04010008) reverses the raw property
    /// order and keeps null nodes; 0070B530 stores parents before slots and links effects through 00708E40 both ways.
    #[test]
    fn mixed_link_and_property_order() {
        let mut world = World::new();
        let modern = link_fixture(&mut world, 0x1400_0004);
        assert_eq!(link(&mut world, &modern), SceneNodeLinkStatus::Linked);
        assert_eq!(
            world.get::<NiAvProperties>(modern.parent).unwrap().0,
            [Some(modern.vertex), None, Some(modern.zbuffer)],
            "modern property list must AddHead and keep null nodes"
        );
        assert_eq!(
            world
                .get::<NiObjectNet>(modern.parent)
                .unwrap()
                .extra_data
                .len(),
            1
        );
        assert_eq!(
            world.get::<NiNodeChildren>(modern.parent).unwrap().slots_ac,
            [Some(modern.child), None, Some(modern.shape)]
        );
        assert_eq!(
            world.get::<NiAvParent>(modern.child).unwrap().0,
            Some(modern.parent)
        );
        assert_eq!(
            world.get::<NiAvParent>(modern.shape).unwrap().0,
            Some(modern.parent),
            "SetParent reaches a leaf child too"
        );
        assert_eq!(
            world.get::<NiNodeEffects>(modern.parent).unwrap().0,
            [modern.light]
        );
        assert_eq!(
            world.get::<NiEffectAffectedNodes>(modern.light).unwrap().0,
            [modern.parent]
        );

        let legacy = link_fixture(&mut world, 0x0400_0000);
        assert_eq!(link(&mut world, &legacy), SceneNodeLinkStatus::Linked);
        let properties = &world.get::<NiAvProperties>(legacy.parent).unwrap().0;
        assert_eq!(
            properties[0],
            Some(legacy.zbuffer),
            "AddTail keeps raw order"
        );
        assert_eq!(properties[2], Some(legacy.vertex));
    }

    /// Ported from `verify_link_rejections_leave_node_fresh`: 007082DE's collision gate and 0070B250 / 007074B0's detach
    /// (not ported) both stop before any store.
    #[test]
    fn link_rejections_leave_the_node_fresh() {
        let mut world = World::new();
        let collision = link_fixture(&mut world, 0x1400_0004);
        world
            .get_mut::<SceneNodePayload>(collision.parent)
            .unwrap()
            .collision_index = 4;
        assert_eq!(
            link(&mut world, &collision),
            SceneNodeLinkStatus::UnsupportedOwner
        );
        assert!(
            world
                .get::<NiNodeChildren>(collision.parent)
                .unwrap()
                .slots_ac
                .is_empty()
        );
        assert!(world.get::<NiAvProperties>(collision.parent).is_none());
        assert_eq!(world.get::<NiAvParent>(collision.child).unwrap().0, None);

        let duplicate = link_fixture(&mut world, 0x1400_0004);
        world
            .get_mut::<SceneNodePayload>(duplicate.parent)
            .unwrap()
            .child_indices = vec![1, NULL_REFERENCE, 1];
        let status = link_scene_node(
            &mut world,
            duplicate.parent,
            &[Some(duplicate.child), None, Some(duplicate.child)],
            &[Some(duplicate.light)],
            &duplicate.table,
        )
        .unwrap();
        assert_eq!(status, SceneNodeLinkStatus::UnsupportedOwner);
        assert!(
            world
                .get::<NiNodeChildren>(duplicate.parent)
                .unwrap()
                .slots_ac
                .is_empty()
        );
        assert_eq!(world.get::<NiAvParent>(duplicate.child).unwrap().0, None);
    }

    /// Ported from `verify_leaf_selective_flags`: 00707D80's leaf flags and 007077A0's property controllers.
    #[test]
    fn leaf_selective_flags() {
        let (mut flags, mut selective, mut rigid) = (0x0006u16, true, false);
        set_leaf_selective_update_flags(
            &mut flags,
            &mut selective,
            false,
            &mut rigid,
            false,
            false,
            false,
        );
        assert_eq!(flags, 0x0010);
        assert!(!selective && rigid);
        flags = 0;
        set_leaf_selective_update_flags(
            &mut flags,
            &mut selective,
            false,
            &mut rigid,
            true,
            true,
            true,
        );
        assert_eq!(flags, 0x001E);
        assert!(selective && !rigid);

        let mut world = World::new();
        let texturing = world
            .spawn((
                NiClass::TexturingProperty,
                NiObjectNet::default(),
                TexturingProperty::default(),
            ))
            .id();
        let properties = [None, Some(texturing)];
        assert!(!av_has_property_controller(&world, &properties));
        // A spawned entity stands in for the NiFlipController head; only its presence is read.
        let head = world.spawn_empty().id();
        world
            .get_mut::<NiObjectNet>(texturing)
            .unwrap()
            .controller_0c = Some(head);
        assert!(av_has_property_controller(&world, &properties));
    }

    /// The linked slot 0070B250 would leave, as the C++ fixture sets it directly.
    fn attach(world: &mut World, parent: Entity, child: Entity) {
        world
            .get_mut::<NiNodeChildren>(parent)
            .unwrap()
            .slots_ac
            .push(Some(child));
        world.entity_mut(child).insert(NiAvParent(Some(parent)));
    }

    fn unit_shape(world: &mut World, local: NiTransform) -> Entity {
        let data = world
            .spawn((
                NiClass::TriShapeData,
                MeshTriShapeData {
                    base: MeshGeometryData {
                        bounds_0c: [0.0, 0.0, 0.0, 1.0],
                        ..MeshGeometryData::default()
                    },
                    ..MeshTriShapeData::default()
                },
            ))
            .id();
        let shape = spawn_mesh_geometry(
            world,
            NiClass::TriShape,
            MeshTriShapePayload {
                local_transform: local,
                ..MeshTriShapePayload::default()
            },
        );
        world.get_mut::<NiGeometry>(shape).unwrap().data_b4 = Some(data);
        shape
    }

    fn set_node_local(world: &mut World, node: Entity, local: NiTransform) {
        world
            .get_mut::<SceneNodePayload>(node)
            .unwrap()
            .local_transform = local;
    }

    /// Ported from `verify_update_pass`: 00707370 -> 0070A0D0: parent +0x74, children +0x60, inline bound merge;
    /// 00722AA0's geometry bound and its 0x80 equality flag; 00709EC0's upward pass.
    #[test]
    fn update_pass_reaches_the_leaf() {
        let mut world = World::new();
        let mut runtime = NoSceneControllers;
        let parent = spawn_scene_node(&mut world, SceneNodePayload::default());
        let shape = unit_shape(&mut world, translated(1.0, 0.0, 0.0, 1.0));
        set_node_local(&mut world, parent, translated(10.0, 0.0, 0.0, 2.0));
        attach(&mut world, parent, shape);

        let flags = |world: &World| {
            world
                .get::<MeshTriShapePayload>(shape)
                .unwrap()
                .effective_av_flags
        };
        let result = update_scene_object(&mut world, parent, 1.0, true, &mut runtime).unwrap();
        assert_eq!(result.status, Status::Updated);
        let leaf = *world.get::<NiAvWorld>(shape).unwrap();
        assert_eq!(leaf.transform.translation, [12.0, 0.0, 0.0]);
        assert_eq!(leaf.transform.scale, 2.0);
        assert_eq!(leaf.bound, [12.0, 0.0, 0.0, 2.0]);
        assert_eq!(world.get::<NiAvWorld>(parent).unwrap().bound, leaf.bound);

        world
            .get_mut::<MeshTriShapePayload>(shape)
            .unwrap()
            .effective_av_flags |= 0x0080;
        let result = update_scene_object(&mut world, parent, 2.0, true, &mut runtime).unwrap();
        assert_eq!(result.status, Status::Updated);
        assert_ne!(
            flags(&world) & 0x0080,
            0,
            "an unchanged bound keeps geometry flag 0x80"
        );
        set_node_local(&mut world, parent, translated(20.0, 0.0, 0.0, 2.0));
        let result = update_scene_object(&mut world, parent, 3.0, true, &mut runtime).unwrap();
        assert_eq!(result.status, Status::Updated);
        assert_eq!(
            flags(&world) & 0x0080,
            0,
            "a changed bound clears geometry flag 0x80"
        );
        update_scene_object(&mut world, parent, 4.0, true, &mut runtime).unwrap();
        assert_eq!(
            flags(&world) & 0x0080,
            0,
            "flag 0x80 stays clear once cleared"
        );

        // 0070A280 needs +0xCC, written only by 0070A9C0.
        let result = update_scene_object_selected(&mut world, parent, 5.0, &mut runtime).unwrap();
        assert_eq!(
            result.status,
            Status::Updated,
            "a selected pass without selective flags still runs the node controllers"
        );
    }

    /// Ported from `verify_billboard_and_light_update`: 007219C0's billboard downward pass, 00722340's bound recentring,
    /// and 007197C0's light direction and +0xB8 increment.
    #[test]
    fn billboard_and_light_update() {
        let mut world = World::new();
        let mut runtime = NoSceneControllers;
        let billboard = spawn_billboard_node(&mut world);
        let shape = unit_shape(&mut world, translated(3.0, 4.0, 0.0, 1.0));
        set_node_local(&mut world, billboard, translated(0.0, 0.0, 0.0, 1.0));
        attach(&mut world, billboard, shape);

        let light = spawn_directional_light(&mut world);
        {
            let mut state = world.get_mut::<DirectionalLight>(light).unwrap();
            state.payload.av.local_transform = NiTransform {
                rotation: [0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0],
                ..translated(0.0, 0.0, 0.0, 1.0)
            };
        }
        attach(&mut world, billboard, light);
        let revision = world.get::<DirectionalLight>(light).unwrap().unknown_b8;

        let result = update_scene_object(&mut world, billboard, 7.5, true, &mut runtime).unwrap();
        assert_eq!(result.status, Status::Updated);
        let state = *world.get::<BillboardNode>(billboard).unwrap();
        assert_eq!(state.unknown_e0, 7.5);
        assert_ne!(state.effective_mode_flags_dc & 0x0008, 0);
        assert_eq!(
            world.get::<NiAvWorld>(billboard).unwrap().bound,
            [0.0, 0.0, 0.0, 6.0],
            "the bound recentres on the translation and grows by the offset"
        );
        let state = world.get::<DirectionalLight>(light).unwrap();
        assert_eq!(state.direction_108, [0.0, 0.0, 1.0]);
        assert_eq!(state.unknown_b8, revision + 1);
    }

    /// The BSXFlags half of `verify_bsx_flags` that needs no NiNif payload: 006FF8A0's bare ED%03d name.
    #[test]
    fn unnamed_bsx_flags_take_the_bare_generated_name() {
        let mut world = World::new();
        let unnamed = world
            .spawn((NiClass::BsxFlagsExtraData, NiExtraDataName::default()))
            .id();
        let mut table = StreamObjectTable::default();
        table.push("BSXFlags", 0x006F_A900, unnamed);
        let pending = ObjectNetPending {
            extra_indices: vec![0],
            ..ObjectNetPending::default()
        };
        assert!(object_net_extras_supported(&world, &pending, &table));
        let state = resolve_object_net_extra_links(&mut world, &pending, &table).unwrap();
        assert_eq!(state.extra_data.len(), 1);
        assert_eq!(
            world.get::<NiExtraDataName>(unnamed).unwrap().0.as_deref(),
            Some(&b"ED000"[..])
        );
    }

    /// A recording stand-in for the controller branches: the next link comes from the fixture's map.
    struct RecordingRuntime {
        ran: Vec<Entity>,
        next: Vec<(Entity, Option<Entity>)>,
        stop_at: Option<(Entity, SceneControllerStep)>,
    }

    impl SceneUpdateRuntime for RecordingRuntime {
        fn update_controller(
            &mut self,
            _: &mut World,
            controller: Entity,
            _: f32,
        ) -> Result<SceneControllerStep, StreamError> {
            self.ran.push(controller);
            if let Some((entity, step)) = self.stop_at
                && entity == controller
            {
                return Ok(step);
            }
            let next = self
                .next
                .iter()
                .find(|entry| entry.0 == controller)
                .and_then(|entry| entry.1);
            Ok(SceneControllerStep::Completed {
                next,
                status: NodeControllerUpdateStatus::Applied,
            })
        }
    }

    /// 0047C930's order on the seam: properties, then the object's own chain, following +0x34 links.
    #[test]
    fn controllers_run_properties_first_and_stop_at_the_seam() {
        let mut world = World::new();
        let node = spawn_scene_node(&mut world, SceneNodePayload::default());
        let property_head = world.spawn_empty().id();
        let texturing = world
            .spawn((
                NiClass::TexturingProperty,
                NiObjectNet {
                    controller_0c: Some(property_head),
                    ..NiObjectNet::default()
                },
            ))
            .id();
        world
            .entity_mut(node)
            .insert(NiAvProperties(vec![None, Some(texturing)]));
        let first = world.spawn_empty().id();
        let second = world.spawn_empty().id();
        world.get_mut::<NiObjectNet>(node).unwrap().controller_0c = Some(first);
        let mut runtime = RecordingRuntime {
            ran: Vec::new(),
            next: vec![(first, Some(second))],
            stop_at: None,
        };
        let result = update_scene_object(&mut world, node, 0.0, true, &mut runtime).unwrap();
        assert_eq!(result.status, Status::Updated);
        assert_eq!(runtime.ran, [property_head, first, second]);

        // updateControllers false skips the whole of 0047C930 in the downward pass.
        runtime.ran.clear();
        update_scene_object(&mut world, node, 0.0, false, &mut runtime).unwrap();
        assert!(runtime.ran.is_empty());

        // Selected: 0047C930 always, but the property half only under flag 0x08, which the cold 0x000E has.
        world
            .get_mut::<SceneNodePayload>(node)
            .unwrap()
            .effective_av_flags = 0x0006;
        update_scene_object_selected(&mut world, node, 0.0, &mut runtime).unwrap();
        assert_eq!(runtime.ran, [first, second]);

        runtime.ran.clear();
        runtime.stop_at = Some((
            first,
            SceneControllerStep::Failed {
                status: NodeControllerUpdateStatus::UnsupportedSequenceUpdate,
            },
        ));
        let result = update_scene_object(&mut world, node, 0.0, true, &mut runtime).unwrap();
        assert_eq!(result.status, Status::ControllerFailed);
        assert_eq!(runtime.ran, [property_head, first]);

        let result = update_scene_object(&mut world, node, 0.0, true, &mut NoSceneControllers);
        assert_eq!(result.unwrap().status, Status::UnsupportedController);

        // A controller on another property class has no +0x50 dispatch.
        world.entity_mut(texturing).insert(NiClass::ZBufferProperty);
        let result = update_scene_object_controllers(&mut world, node, 0.0, &mut runtime).unwrap();
        assert_eq!(result.status, Status::UnsupportedController);
    }

    /// 0070A310 recurses after its own chain; a leaf runs only its property list.
    #[test]
    fn update_controllers_recurses_through_children() {
        let mut world = World::new();
        let node = spawn_scene_node(&mut world, SceneNodePayload::default());
        let shape = unit_shape(&mut world, NiTransform::default());
        attach(&mut world, node, shape);
        let node_head = world.spawn_empty().id();
        world.get_mut::<NiObjectNet>(node).unwrap().controller_0c = Some(node_head);
        let property_head = world.spawn_empty().id();
        let texturing = world
            .spawn((
                NiClass::TexturingProperty,
                NiObjectNet {
                    controller_0c: Some(property_head),
                    ..NiObjectNet::default()
                },
            ))
            .id();
        world
            .entity_mut(shape)
            .insert(NiAvProperties(vec![Some(texturing)]));
        let mut runtime = RecordingRuntime {
            ran: Vec::new(),
            next: Vec::new(),
            stop_at: None,
        };
        let result = update_scene_object_controllers(&mut world, node, 0.0, &mut runtime).unwrap();
        assert_eq!(result.status, Status::Updated);
        assert_eq!(runtime.ran, [node_head, property_head]);
    }

    /// 007075B0 and the selected / rigid NiNode passes; 0070A9C0's +0xCC; 00709EC0's upward walk.
    #[test]
    fn selected_rigid_and_upward_passes() {
        let mut world = World::new();
        let mut runtime = NoSceneControllers;
        let root = spawn_scene_node(&mut world, SceneNodePayload::default());
        let middle = spawn_scene_node(&mut world, SceneNodePayload::default());
        let shape = unit_shape(&mut world, translated(1.0, 0.0, 0.0, 1.0));
        attach(&mut world, root, middle);
        attach(&mut world, middle, shape);
        set_node_local(&mut world, middle, translated(0.0, 5.0, 0.0, 1.0));
        update_scene_object(&mut world, root, 0.0, true, &mut runtime).unwrap();
        assert_eq!(
            world.get::<NiAvWorld>(root).unwrap().bound,
            [1.0, 5.0, 0.0, 1.0]
        );

        // A rigid child without +0xCC stops with the host's rigid-bound status.
        for (node, flags) in [(root, 0x0004u16), (middle, 0x0016)] {
            world
                .get_mut::<SceneNodePayload>(node)
                .unwrap()
                .effective_av_flags = flags;
        }
        let result = update_scene_object_selected(&mut world, root, 0.0, &mut runtime).unwrap();
        assert_eq!(result.status, Status::UnsupportedRigidBound);

        update_scene_node_model_bound(&mut world, root);
        assert_eq!(
            world.get::<NiAvWorld>(middle).unwrap().combined_bound,
            Some([1.0, 0.0, 0.0, 1.0]),
            "the world bound in model space"
        );
        set_node_local(&mut world, middle, translated(0.0, 7.0, 0.0, 1.0));
        // Without 0x02 (the cold 0x000E has it), the rigid pass leaves the leaf where it was.
        world
            .get_mut::<MeshTriShapePayload>(shape)
            .unwrap()
            .effective_av_flags = 0;
        let before = world.get::<NiAvWorld>(shape).unwrap().transform;
        let result = update_scene_object_selected(&mut world, root, 0.0, &mut runtime).unwrap();
        assert_eq!(result.status, Status::Updated);
        assert_eq!(world.get::<NiAvWorld>(shape).unwrap().transform, before);
        assert_eq!(
            world.get::<NiAvWorld>(middle).unwrap().bound,
            [1.0, 7.0, 0.0, 1.0],
            "0072A820 over +0xCC"
        );
        assert_eq!(
            world.get::<NiAvWorld>(root).unwrap().bound,
            [1.0, 7.0, 0.0, 1.0],
            "the merge after a rigid child"
        );

        // Updating a leaf walks up: each ancestor recomputes its bound from its children.
        set_node_local(&mut world, middle, translated(0.0, 0.0, 0.0, 1.0));
        world
            .get_mut::<MeshTriShapePayload>(shape)
            .unwrap()
            .local_transform = translated(2.0, 0.0, 0.0, 1.0);
        let result = update_scene_object(&mut world, shape, 0.0, true, &mut runtime).unwrap();
        assert_eq!(result.status, Status::Updated);
        assert_eq!(
            world.get::<NiAvWorld>(shape).unwrap().bound,
            [2.0, 7.0, 0.0, 1.0],
            "the leaf composes with its parent's stored world, not the new local"
        );
        assert_eq!(
            world.get::<NiAvWorld>(root).unwrap().bound,
            [2.0, 7.0, 0.0, 1.0]
        );
    }

    #[test]
    fn host_stops_report_their_cause() {
        let mut world = World::new();
        let mut runtime = NoSceneControllers;

        // Null +0xB4 data, then a skinned shape: the bound stays where it was.
        let shape = spawn_mesh_geometry(
            &mut world,
            NiClass::TriShape,
            MeshTriShapePayload::default(),
        );
        let update = |world: &mut World, object, runtime: &mut NoSceneControllers| {
            update_scene_object(world, object, 0.0, true, runtime)
                .unwrap()
                .status
        };
        assert_eq!(
            update(&mut world, shape, &mut runtime),
            Status::UnsupportedMissingData
        );
        let skin = world.spawn_empty().id();
        world.get_mut::<NiGeometry>(shape).unwrap().skin_b8 = Some(skin);
        assert_eq!(
            update(&mut world, shape, &mut runtime),
            Status::UnsupportedSkinnedBound
        );
        assert_eq!(world.get::<NiAvWorld>(shape).unwrap().bound, [0.0; 4]);

        // A parent that has gone stops the pass; a collision owner stops after the store.
        let orphan = unit_shape(&mut world, NiTransform::default());
        let gone = spawn_scene_node(&mut world, SceneNodePayload::default());
        world.entity_mut(orphan).insert(NiAvParent(Some(gone)));
        world.despawn(gone);
        assert_eq!(
            update(&mut world, orphan, &mut runtime),
            Status::MissingParent
        );
        let collided = spawn_scene_node(
            &mut world,
            SceneNodePayload {
                collision_index: 0,
                ..SceneNodePayload::default()
            },
        );
        assert_eq!(
            update(&mut world, collided, &mut runtime),
            Status::UnsupportedCollision
        );

        // 00749550 reads 00722AA0's data pointer, which the original dereferences unchecked.
        let particles = spawn_particle_system(&mut world);
        assert_eq!(
            update(&mut world, particles, &mut runtime),
            Status::UnsupportedMissingData
        );
        let node = spawn_scene_node(&mut world, SceneNodePayload::default());
        attach(&mut world, node, particles);
        assert_eq!(
            update(&mut world, node, &mut runtime),
            Status::UnsupportedMissingData
        );
        let data = world.spawn(NiClass::FloatKeyData).id();
        assert_eq!(
            update(&mut world, data, &mut runtime),
            Status::UnsupportedClass
        );
    }

    fn particle_system_with_data(world: &mut World, local: NiTransform) -> Entity {
        let system = spawn_particle_system(world);
        let data = world
            .spawn((
                NiClass::ParticleData,
                ParticleData {
                    bounds: [1.0, 2.0, 3.0, 4.0],
                    ..ParticleData::default()
                },
            ))
            .id();
        let mut state = world.get_mut::<ParticleSystem>(system).unwrap();
        state.data_b4 = Some(data);
        state.local_transform_30 = local;
        system
    }

    /// 007492B0 / 00749310 / 00749550 in world space: the world translation and rotation reset, the bound is the model bound
    /// scaled by the composed world scale, and the node above merges it.
    #[test]
    fn particle_system_in_world_space() {
        let mut world = World::new();
        let parent = spawn_scene_node(
            &mut world,
            SceneNodePayload {
                local_transform: translated(10.0, 0.0, 0.0, 2.0),
                ..SceneNodePayload::default()
            },
        );
        let system = particle_system_with_data(&mut world, translated(1.0, 2.0, 3.0, 2.0));
        attach(&mut world, parent, system);
        let result =
            update_scene_object(&mut world, parent, 1.0, true, &mut NoSceneControllers).unwrap();
        assert_eq!(result.status, Status::Updated);
        let state = world.get::<NiAvWorld>(system).unwrap();
        assert_eq!(state.transform, translated(0.0, 0.0, 0.0, 4.0));
        assert_eq!(state.bound, [4.0, 8.0, 12.0, 16.0]);
        assert_eq!(world.get::<NiAvWorld>(parent).unwrap().bound, state.bound);
    }

    /// 00722AA0's branch: out of world space a NiParticleSystem bound is an ordinary geometry bound, with the 0x80 rule.
    #[test]
    fn particle_system_in_local_space() {
        let mut world = World::new();
        let parent = spawn_scene_node(
            &mut world,
            SceneNodePayload {
                local_transform: translated(10.0, 0.0, 0.0, 2.0),
                ..SceneNodePayload::default()
            },
        );
        let system = particle_system_with_data(&mut world, translated(1.0, 2.0, 3.0, 2.0));
        attach(&mut world, parent, system);
        {
            let mut state = world.get_mut::<ParticleSystem>(system).unwrap();
            state.in_world_space_c0 = false;
            state.effective_av_flags_18 = 0x0080;
        }
        let run = |world: &mut World| {
            update_scene_object(world, parent, 1.0, true, &mut NoSceneControllers).unwrap();
            world
                .get::<ParticleSystem>(system)
                .unwrap()
                .effective_av_flags_18
        };
        assert_eq!(run(&mut world), 0, "a changed bound clears 0x80");
        let state = world.get::<NiAvWorld>(system).unwrap();
        assert_eq!(state.transform, translated(12.0, 4.0, 6.0, 4.0));
        assert_eq!(state.bound, [16.0, 12.0, 18.0, 16.0]);
        world
            .get_mut::<ParticleSystem>(system)
            .unwrap()
            .effective_av_flags_18 = 0x0080;
        assert_eq!(run(&mut world), 0x0080, "an unchanged bound keeps 0x80");
    }

    /// 007492E0 runs +0x74 only under flag 0x04, and 007075B0 then runs +0x78 only under it too.
    #[test]
    fn particle_system_selected_pass_follows_its_flags() {
        let mut world = World::new();
        let system = particle_system_with_data(&mut world, translated(1.0, 2.0, 3.0, 2.0));
        let mut runtime = NoSceneControllers;
        world
            .get_mut::<ParticleSystem>(system)
            .unwrap()
            .in_world_space_c0 = false;
        let before = *world.get::<NiAvWorld>(system).unwrap();
        let result = update_scene_object_selected(&mut world, system, 1.0, &mut runtime).unwrap();
        assert_eq!(result.status, Status::Updated);
        assert_eq!(world.get::<NiAvWorld>(system), Some(&before));
        world
            .get_mut::<ParticleSystem>(system)
            .unwrap()
            .effective_av_flags_18 = 0x0004;
        update_scene_object_selected(&mut world, system, 1.0, &mut runtime).unwrap();
        let state = world.get::<NiAvWorld>(system).unwrap();
        assert_eq!(state.transform, translated(1.0, 2.0, 3.0, 2.0));
        assert_eq!(state.bound, [3.0, 6.0, 9.0, 8.0]);

        // A node's selected pass reads the child's own flags word.
        let parent = spawn_scene_node(&mut world, SceneNodePayload::default());
        attach(&mut world, parent, system);
        world
            .get_mut::<ParticleSystem>(system)
            .unwrap()
            .effective_av_flags_18 = 0x0002;
        let result = update_scene_object_selected(&mut world, parent, 1.0, &mut runtime).unwrap();
        assert_eq!(result.status, Status::Updated);
    }

    /// 0047C930 on a NiParticleSystem: its chain goes through the scheduler, and the result carries the schedule's status.
    #[test]
    fn particle_system_controllers_run_through_the_scheduler() {
        let mut world = World::new();
        let system = particle_system_with_data(&mut world, translated(0.0, 0.0, 0.0, 1.0));
        let mut controller = ParticleUpdateController::default();
        controller.time.target_30 = Some(system);
        controller.time.target_linked = true;
        let head = world
            .spawn((NiClass::ParticleUpdateController, controller))
            .id();
        world.get_mut::<NiObjectNet>(system).unwrap().controller_0c = Some(head);

        // With no services, as an empty C++ context.
        let result =
            update_scene_object_controllers(&mut world, system, 1.0, &mut NoSceneControllers)
                .unwrap();
        assert_eq!(result.status, Status::ParticleScheduleFailed);
        assert_eq!(
            result.particle_status,
            Some(ParticleControllerScheduleStatus::UnsupportedSystemUpdateService)
        );

        let mut updates = Vec::new();
        let mut update_system = |_: &mut World, target: Entity, time: f32| {
            updates.push((target, time));
        };
        let mut particles = ParticleControllerScheduleContext {
            update_system: Some(&mut update_system),
            ..ParticleControllerScheduleContext::default()
        };
        let mut runtime = SceneControllers {
            particles: &mut particles,
            tile_textures: None,
        };
        let result =
            update_scene_object_controllers(&mut world, system, 2.0, &mut runtime).unwrap();
        assert_eq!(result.status, Status::Updated);
        assert_eq!(
            result.particle_status,
            Some(ParticleControllerScheduleStatus::Completed)
        );
        // The downward pass with updateControllers false runs no chain; true does, between the two +0x74 calls.
        let result = update_scene_object(&mut world, system, 3.0, false, &mut runtime).unwrap();
        assert_eq!(result.status, Status::Updated);
        let scheduled = update_scene_object(&mut world, system, 3.0, true, &mut runtime).unwrap();
        assert_eq!(scheduled.status, Status::Updated);
        assert_eq!(updates, [(system, 2.0), (system, 3.0)]);
    }

    /// The real +0x54 dispatch: the node classes follow +0x34 after Update, each sampling the one shared cache, and a host
    /// stop reports the Update's status.
    #[test]
    fn scene_controllers_dispatch_the_node_chain() {
        let mut world = World::new();
        let node = spawn_scene_node(&mut world, SceneNodePayload::default());
        let transform = spawn_node_controller(
            &mut world,
            NiClass::TransformController,
            TransformController::default(),
        );
        let vis =
            spawn_node_controller(&mut world, NiClass::VisController, VisController::default());
        let multi = spawn_node_controller(
            &mut world,
            NiClass::MultiTargetTransformController,
            MultiTargetTransformController::default(),
        );
        let manager = spawn_node_controller(
            &mut world,
            NiClass::ControllerManager,
            ControllerManager::default(),
        );
        let time_of = |world: &mut World, entity, next| {
            world
                .get_mut::<NodeController>(entity)
                .unwrap()
                .time
                .next_34 = next;
        };
        time_of(&mut world, transform, Some(vis));
        time_of(&mut world, vis, Some(multi));
        time_of(&mut world, multi, Some(manager));
        world.get_mut::<NiObjectNet>(node).unwrap().controller_0c = Some(transform);

        let mut particles = ParticleControllerScheduleContext::default();
        let mut runtime = SceneControllers {
            particles: &mut particles,
            tile_textures: None,
        };
        let result = update_scene_object_controllers(&mut world, node, 2.0, &mut runtime).unwrap();
        assert_eq!(result.status, Status::Updated);
        for controller in [transform, vis] {
            let time = &world.get::<NodeController>(controller).unwrap().time;
            assert_eq!(time.fields.last_time, 2.0, "006C36B0's gate ran");
        }
        assert!(world.contains_resource::<ParticleControllerTimeCache>());

        // An expired active sequence is 006C4200's host stop, after the earlier controllers ran.
        let gone = world.spawn_empty().id();
        world
            .get_mut::<ControllerManager>(manager)
            .unwrap()
            .active_sequences_4c
            .push(gone);
        let result = update_scene_object_controllers(&mut world, node, 3.0, &mut runtime).unwrap();
        assert_eq!(result.status, Status::ControllerFailed);
        assert_eq!(
            result.controller_status,
            Some(NodeControllerUpdateStatus::UnsupportedSequenceUpdate)
        );

        // A class with no +0x54 here stops the chain.
        let data = world.spawn(NiClass::FloatKeyData).id();
        world.get_mut::<NiObjectNet>(node).unwrap().controller_0c = Some(data);
        let result = update_scene_object_controllers(&mut world, node, 4.0, &mut runtime).unwrap();
        assert_eq!(result.status, Status::UnsupportedController);
    }

    /// 00700A90 on a NiTexturingProperty: NiFlipController, then NiTextureTransformController.
    #[test]
    fn scene_controllers_dispatch_the_texture_chain() {
        let mut world = World::new();
        let node = spawn_scene_node(&mut world, SceneNodePayload::default());
        let flip = spawn_texture_controller(
            &mut world,
            NiClass::FlipController,
            FlipController::default(),
        );
        let transform = spawn_texture_controller(
            &mut world,
            NiClass::TextureTransformController,
            TextureTransformController::default(),
        );
        world
            .get_mut::<TextureController>(flip)
            .unwrap()
            .time
            .next_34 = Some(transform);
        let texturing = world
            .spawn((
                NiClass::TexturingProperty,
                NiObjectNet {
                    controller_0c: Some(flip),
                    ..NiObjectNet::default()
                },
            ))
            .id();
        world
            .entity_mut(node)
            .insert(NiAvProperties(vec![Some(texturing)]));
        let mut particles = ParticleControllerScheduleContext::default();
        let mut runtime = SceneControllers {
            particles: &mut particles,
            tile_textures: None,
        };
        let result = update_scene_object_controllers(&mut world, node, 2.0, &mut runtime).unwrap();
        assert_eq!(result.status, Status::Updated);
        for controller in [flip, transform] {
            let time = &world.get::<TextureController>(controller).unwrap().time;
            assert_eq!(time.fields.last_time, 2.0);
        }
    }

    #[test]
    fn particle_world_space_tails() {
        let mut transform = translated(1.0, 2.0, 3.0, 4.0);
        transform.rotation = [0.0; 9];
        particle_world_data_in_world_space(&mut transform);
        assert_eq!(transform, translated(0.0, 0.0, 0.0, 4.0), "scale untouched");
        assert_eq!(
            particle_world_bound_in_world_space(&[1.0, 2.0, 3.0, 4.0], 2.0),
            [2.0, 4.0, 6.0, 8.0]
        );
    }

    #[test]
    fn bound_transform_and_equality() {
        let transform = NiTransform {
            rotation: [0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            translation: [10.0, 20.0, 30.0],
            scale: 2.0,
        };
        assert_eq!(
            transform_scene_bound(&[1.0, 2.0, 3.0, 4.0], &transform),
            [16.0, 22.0, 34.0, 8.0]
        );
        let nan = [0.0, 0.0, 0.0, f32::NAN];
        assert!(!same_bound(&nan, &nan), "NaN compares unequal");
        assert!(same_bound(&[-0.0, 0.0, 0.0, 1.0], &[0.0, 0.0, 0.0, 1.0]));
    }
}
