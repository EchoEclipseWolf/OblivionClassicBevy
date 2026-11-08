//! 00714390's object passes: every LoadBinary in stream order, the root list, then every LinkObject.
//!
//! A stream object is an entity, so a pass arm reads and writes components. Nothing here iterates a query: the
//! table's order is the original's order. A failed load despawns everything it spawned, except what the texture palette
//! holds (`despawn_table`), so no partial graph is published. That is a host rule; the original's cancellation and
//! partial publication stay open.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::super::bs::havok_stream::{
    BhkConstraint, BhkConvexVerticesShape, BhkRigidBody, BhkRigidBodyPending, BhkShape,
    BhkShapeKind, BhkShapePending, BhkSimpleShapePhantom, BhkSpCollisionObject,
    BhkSpCollisionPayload, ConstraintKind, collision_target_flags, parse_constraint_payload,
    parse_convex_vertices_payload, parse_phantom_payload, parse_rigid_body_payload,
    parse_shape_payload, parse_sp_collision_payload,
};
use super::super::bs::nif_objects::{
    load_bs_parent_velocity, load_bsx_flags, parse_bs_parent_velocity_payload,
    parse_bsx_flags_payload,
};
use super::av_object_palette::{
    AvObjectPaletteLinkStatus, apply_av_object_palette_payload, parse_av_object_palette_payload,
};
use super::color_animation::ColorKeyData;
use super::controller_sequence::{
    ControllerManagerPayload, ControllerSequencePayload, link_controller_manager,
    link_controller_sequence, load_controller_manager, load_controller_sequence,
    parse_controller_manager_payload, parse_controller_sequence_payload,
};
use super::factory_payload::{
    FactoryPayloadStatus, FactoryPendingPayload, load_factory_payload_into_owner,
};
use super::interpolator_stream::{
    load_bool_interpolator_payload, load_bool_timeline_interpolator_payload,
    load_float_interpolator_payload, load_transform_interpolator_payload,
};
use super::mesh_geometry::{MeshTriShapeLinkStatus, link_mesh_geometry};
use super::mesh_skin_owners::{MeshSkinLinkStatus, link_mesh_skin_instance};
use super::object_net::{
    AlphaPropertyPending, parse_alpha_property_payload, resolve_alpha_property_links,
};
use super::particle_blend_interpolator::{
    StreamLoadStatus, load_particle_blend_bool, load_particle_blend_float,
    load_particle_blend_transform, parse_particle_blend_bool_payload,
    parse_particle_blend_float_payload, parse_particle_blend_transform_payload,
};
use super::particle_color::{
    ParticleColorModifier, ParticleColorPending, link_particle_color, parse_particle_color_payload,
    resolve_particle_color_links,
};
use super::particle_controller::{
    ControllerLinkStatus, ParticleEmitterControllerPayload, ParticleTimeControllerPayload,
    link_particle_emitter_controller, link_particle_time_controller,
    load_particle_emitter_controller, parse_particle_emitter_controller_payload,
};
use super::particle_data::ParticleData;
use super::particle_drag::{
    ParticleDragModifier, ParticleDragPending, link_particle_drag, parse_particle_drag_payload,
    resolve_particle_drag_link,
};
use super::particle_dynamics::{
    ParticleAgeDeathModifier, ParticleAgeDeathPending, ParticleBoundUpdateModifier,
    ParticleGrowFadeModifier, ParticlePositionModifier, link_particle_age_death,
    parse_particle_age_death_payload, parse_particle_bound_update_payload,
    parse_particle_grow_fade_payload, parse_particle_position_payload,
    resolve_particle_age_death_link, resolve_particle_bound_update_link,
    resolve_particle_grow_fade_link, resolve_particle_position_link,
};
use super::particle_emitter::{
    ParticleBoxEmitter, ParticleCylinderEmitter, ParticleCylinderEmitterPending,
    ParticleEmitterPending, ParticleSphereEmitter, ParticleSphereEmitterPending,
    parse_particle_box_emitter_payload, parse_particle_cylinder_emitter_payload,
    parse_particle_sphere_emitter_payload, resolve_particle_box_emitter_link,
    resolve_particle_cylinder_emitter_link, resolve_particle_sphere_emitter_link,
};
use super::particle_gravity::{
    ParticleGravityLoadResult, ParticleGravityModifier, link_particle_gravity,
    parse_particle_gravity_payload,
};
use super::particle_mesh_emitter::{ParticleMeshEmitterPending, link_particle_mesh_emitter};
use super::particle_modifier_value_controller::{
    ParticleModifierValueControllerPayload, link_particle_modifier_value_controller,
    load_particle_modifier_value_controller, parse_particle_modifier_value_controller_payload,
};
use super::particle_rotation::{
    ParticleRotationLoadResult, ParticleRotationModifier, link_particle_rotation,
    parse_particle_rotation_payload,
};
use super::particle_spawn::{
    ParticleSpawnModifier, parse_particle_spawn_payload, resolve_particle_spawn_link,
};
use super::particle_system::{
    ParticleSystem, ParticleSystemLinkStatus, ParticleSystemPayload, parse_particle_system_payload,
    resolve_particle_system_links,
};
use super::path_interpolator::{
    PathInterpolator, PathInterpolatorPayload, PathLinkStatus, link_path_interpolator,
    parse_path_interpolator_payload,
};
use super::properties::{
    MaterialPropertyValues, PropertyPending, StencilProperty, VertexColorProperty, ZBufferProperty,
    parse_material_property_payload, parse_stencil_property_payload,
    parse_vertex_color_property_payload, parse_z_buffer_property_payload,
    resolve_material_property_links, resolve_property_links,
};
use super::raw_reference::raw_object_reference;
use super::registration::NiFactoryRegistry;
use super::scene_derived::{
    BillboardNodePending, DirectionalLightLinkStatus, link_billboard_node, link_directional_light,
};
use super::scene_node::{SceneNodeLinkStatus, SceneNodePayload, link_scene_node};
use super::stream_allocation_groups::{StreamAllocationGroups, read_stream_allocation_groups};
use super::stream_objects::{
    NULL_REFERENCE, NiClass, StreamObjectEntry, StreamObjectError, StreamObjectTable,
    read_stream_object_table,
};
use super::stream_roots::{StreamRoots, read_stream_roots};
use super::texture::{
    FlipControllerPayload, SourceTexture, SourceTextureLoadServices, SourceTextureLoadStatus,
    SourceTexturePayload, TextureTransformControllerPayload, TexturingProperty,
    TexturingPropertyPayload, link_flip_controller, link_source_texture,
    link_texture_transform_controller, link_texturing_property, load_flip_controller,
    load_texture_transform_controller, parse_flip_controller_payload, parse_source_texture_payload,
    parse_texture_transform_controller_payload, parse_texturing_property_payload,
    resolve_texture_controller_head,
};
use super::transform_controller::{
    MultiTargetTransformControllerPayload, SingleInterpControllerPayload, link_node_controller,
    load_multi_target_transform_controller, load_transform_controller, load_vis_controller,
    parse_multi_target_transform_controller_payload, parse_single_interp_controller_payload,
    publish_node_controller_head,
};

/// IDA: 00714390's lowest version the bound payload loaders accept.
const MODERN_PAYLOAD_FLOOR: u32 = 0x0A01_0072;
/// IDA: 10.1.0.106, the cobwebs' and SkyDome01's version: 007008A0 reads a group word from 05000006 to below 0A010072.
const LEGACY_GROUP_VERSION: u32 = 0x0A01_006A;

/// IDA: 007008A0 NiObject_LoadBinary at 05000006 <= version < 0A010072: a u32 allocation group (NiStream_GetAllocationGroup),
/// then vt+0x48: NiGeometryData_SetActiveAllocationGroup (00728660) for geometry data, NiObject_PostLoadNoOp for the rest.
/// The exe's other version gates between 0A01006A and 0A010072 belong to the controller, interpolator, sequence and
/// skin classes (0A01006B..0A010071), so a class outside them reads at 10.1.0.106 as at 0A010072 past that word; the
/// controllers and interpolators with gates at or below 10.1.0.106 (00715F40's 0A01006D, 006D0550's 0A010068..0A01006D,
/// 006CB990's 0A01006E) read with the real version (`legacy_payload_version`). Geometry
/// data keeps the word: 00729450 reads its own group at 0A010072 at the same place, with the same effect. Returns the bytes to
/// skip. Completion: 100% for the classes checked; any other class at 10.1.0.106 is a host stop.
// IDA: N/A the version a 10.1.0.106 object's payload parser takes: 10.1.0.106 itself for the classes whose LoadBinary chain
// gates on it, 0A010072 for the rest.
fn legacy_payload_version(world: &World, entity: Entity) -> u32 {
    match world.get::<NiClass>(entity) {
        Some(
            NiClass::TransformController
            | NiClass::TransformInterpolator
            | NiClass::BoneLodController,
        ) => LEGACY_GROUP_VERSION,
        _ => MODERN_PAYLOAD_FLOOR,
    }
}

fn legacy_group_prefix(world: &World, entity: Entity) -> Result<usize, Stop> {
    match world.get::<NiClass>(entity) {
        Some(NiClass::TriShapeData | NiClass::TriStripsData) => Ok(0),
        Some(
            NiClass::Node
            | NiClass::BillboardNode
            | NiClass::TriShape
            | NiClass::TriStrips
            | NiClass::AlphaProperty
            | NiClass::ZBufferProperty
            | NiClass::VertexColorProperty
            | NiClass::StencilProperty
            | NiClass::SpecularProperty
            | NiClass::MaterialProperty
            | NiClass::TexturingProperty
            | NiClass::SourceTexture
            | NiClass::StringExtraData
            | NiClass::BinaryExtraData
            | NiClass::BsxFlagsExtraData
            | NiClass::BsBound
            | NiClass::TransformController
            | NiClass::TransformInterpolator
            | NiClass::BoneLodController,
        ) => Ok(4),
        _ => Err(unsupported_target()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamGraphErrorCode {
    TypeTable,
    UnsupportedPayloadVersion,
    UnsupportedPayloadTarget,
    OwnerMismatch,
    UnsupportedLinkTarget,
    UnsupportedLinkService,
    Cancelled,
    UnavailablePauseService,
    /// A C++ `invalid_argument`, with its message.
    Stream(StreamError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamGraphError {
    pub code: StreamGraphErrorCode,
    pub table_error: Option<Box<StreamObjectError>>,
    pub object_index: Option<usize>,
    pub payload_byte_offset: usize,
    pub original_constructor: Option<u32>,
    /// IDA: N/A a host absolute cursor within a contiguous 00714390 body.
    pub body_byte_offset: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamGraph {
    pub table: StreamObjectTable,
    pub payload_bytes_consumed: usize,
    pub allocation_groups: StreamAllocationGroups,
    pub roots: StreamRoots,
    pub body_bytes_consumed: usize,
    /// IDA: N/A the textures the palette holds, whatever becomes of this graph: 00712980's shared replacements, and
    /// objects 007023E0 registered. The C++ palette's `shared_ptr` keeps them after the graph is dropped.
    pub palette_textures: Vec<Entity>,
}

/// IDA: the services 00714390's object passes consult. `draw_rand` is 0074FE30's CRT rand, called only for
/// flags & 6 == 0 and phase == 0. `source_textures` is what 007023E0 consults during LoadBinary.
/// `stream_auxiliary_04` is NiStream +0x04, the 00712C00 auxiliary u32 that 006C8820 reads.
#[derive(Default)]
pub struct StreamGraphLinkRuntime<'a> {
    pub draw_rand: Option<&'a mut (dyn FnMut() -> i32 + 'a)>,
    /// IDA: N/A `draw_rand` stands for the IO thread's draw, under its 027F control word.
    pub draw_on_io_thread: bool,
    pub source_textures: Option<&'a mut (dyn SourceTextureLoadServices + 'a)>,
    pub stream_auxiliary_04: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamGraphProgressDecision {
    Proceed,
    Cancelled,
    UnavailablePauseService,
}

/// IDA: 00714390 checks +0x260 before each +0x1C LoadBinary, pauses on state 3 before LoadBinary and LinkObject, and
/// increments +0x268 / +0x26C after each completed virtual call.
/// TODO: [ ] 007135C0 cancellation and 00748810 wait return effects.
pub trait StreamGraphProgress {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn before_load(&mut self, _index: usize) -> StreamGraphProgressDecision {
        StreamGraphProgressDecision::Proceed
    }
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn after_load(&mut self, _index: usize, _entry: &StreamObjectEntry) {}
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn before_link(&mut self, _index: usize) -> StreamGraphProgressDecision {
        StreamGraphProgressDecision::Proceed
    }
    /// IDA: N/A no address of its own; its comments cite 00897810.
    fn after_link(&mut self, _index: usize) {}
}

struct NoProgress;
impl StreamGraphProgress for NoProgress {}

/// What LinkObject still needs, in the table's order.
enum Deferred {
    BoneLod(super::bone_lod::BoneLodPayload),
    BlendCollision(crate::bs::havok_stream::BhkSpCollisionPayload),
    Alpha(AlphaPropertyPending),
    ZBuffer(PropertyPending<ZBufferProperty>),
    VertexColor(PropertyPending<VertexColorProperty>),
    Stencil(PropertyPending<StencilProperty>),
    Specular(PropertyPending<super::properties::SpecularProperty>),
    MaterialController(super::material_controllers::MaterialControllerPayload),
    GeomMorpher(super::morph::GeomMorpherPayload),
    Material(PropertyPending<MaterialPropertyValues>),
    Color(ParticleColorPending),
    /// 00897810: the target, its flags, then the phantom.
    SpCollision(BhkSpCollisionPayload),
    /// 0089F350: the shape.
    Phantom(u32),
    /// 008A50E0: the shape, then the constraint list.
    RigidBody(BhkRigidBodyPending),
    /// The shapes' LinkObjects (008C7C20, 008B0320, 008C9710, 008A15C0, 008A2600).
    Shape(BhkShapePending),
    /// 008A0320: the entity list's count and two entities.
    Constraint(Vec<u32>),
    /// 00752CB0 consumes the base target; the load arm stored the scalars and checked its range.
    BaseTarget,
    AgeDeath(ParticleAgeDeathPending),
    Drag(ParticleDragPending),
    Rotation(ParticleRotationLoadResult),
    Gravity(ParticleGravityLoadResult),
    BoxEmitter(ParticleEmitterPending),
    SphereEmitter(ParticleSphereEmitterPending),
    CylinderEmitter(ParticleCylinderEmitterPending),
    MeshEmitter(ParticleMeshEmitterPending),
    System(Box<ParticleSystemPayload>),
    TimeController(ParticleTimeControllerPayload),
    EmitterController(ParticleEmitterControllerPayload),
    ModifierValueController(ParticleModifierValueControllerPayload),
    Geometry,
    SkinInstance,
    Node(Box<SceneNodePayload>),
    Billboard(Box<BillboardNodePending>),
    Light,
    NodeController(ParticleTimeControllerPayload),
    MultiTarget(MultiTargetTransformControllerPayload),
    Sequence(ControllerSequencePayload),
    Manager(ControllerManagerPayload),
    SourceTexture(SourceTexturePayload),
    Texturing(Box<TexturingPropertyPayload>),
    Flip(FlipControllerPayload),
    TextureTransform(TextureTransformControllerPayload),
    Path(PathInterpolatorPayload),
}

/// How an arm stops: a C++ throw, or a returned graph error code.
enum Stop {
    Stream(StreamError),
    Code(StreamGraphErrorCode),
}

impl From<StreamError> for Stop {
    /// IDA: N/A Rust's `From`.
    fn from(error: StreamError) -> Self {
        Self::Stream(error)
    }
}

// IDA: N/A a helper, with no address of its own.
const fn unsupported_target() -> Stop {
    Stop::Code(StreamGraphErrorCode::UnsupportedLinkTarget)
}

// IDA: N/A a helper, with no address of its own.
fn component_mut<'w, T: Component<Mutability = bevy_ecs::component::Mutable>>(
    world: &'w mut World,
    entity: Entity,
) -> Result<Mut<'w, T>, Stop> {
    world
        .get_mut::<T>(entity)
        .ok_or(Stop::Code(StreamGraphErrorCode::OwnerMismatch))
}

// IDA: N/A a helper, with no address of its own.
const fn require(loaded: bool) -> Result<(), Stop> {
    if loaded {
        Ok(())
    } else {
        Err(unsupported_target())
    }
}

// IDA: N/A a helper, with no address of its own.
const fn require_loaded(status: StreamLoadStatus) -> Result<(), Stop> {
    match status {
        StreamLoadStatus::Loaded => Ok(()),
        StreamLoadStatus::UnsupportedLinkTarget => Err(unsupported_target()),
    }
}

/// IDA: 00713FF0's owners followed by one LoadBinary. Returns the bytes read and what the link pass still needs.
#[allow(clippy::too_many_lines)]
#[allow(
    clippy::too_many_arguments,
    reason = "00713FF0's per-object inputs, and the host's record of what the palette holds"
)]
fn load_object(
    world: &mut World,
    table: &mut StreamObjectTable,
    index: usize,
    bytes: &[u8],
    version: u32,
    groups: Option<&mut StreamAllocationGroups>,
    runtime: &mut StreamGraphLinkRuntime<'_>,
    palette_textures: &mut Vec<Entity>,
) -> Result<(usize, Option<Deferred>), Stop> {
    let entity = table.objects[index].entity;
    let constructor = table.objects[index].original_constructor;
    let count = table.objects.len();
    let done = |consumed, deferred| Ok((consumed, Some(deferred)));
    match constructor {
        0x0071_85B0 => {
            let pending = parse_alpha_property_payload(bytes, version)?;
            done(pending.bytes_consumed, Deferred::Alpha(pending))
        }
        0x0070_6CB0 => {
            let pending = parse_z_buffer_property_payload(bytes, version)?;
            done(pending.bytes_consumed, Deferred::ZBuffer(pending))
        }
        0x0070_6530 => {
            let pending = parse_vertex_color_property_payload(bytes, version)?;
            done(pending.bytes_consumed, Deferred::VertexColor(pending))
        }
        0x0071_8F80 => {
            let flags = component_mut::<StencilProperty>(world, entity)?.flags_18;
            let pending = parse_stencil_property_payload(bytes, version, flags)?;
            done(pending.bytes_consumed, Deferred::Stencil(pending))
        }
        // 006D2250 and 006DEDD0 (`material_controllers`).
        0x006D_23D0 | 0x006D_EF40 => {
            let Some(pending) = super::material_controllers::load_material_controller(
                world, entity, bytes, version, table,
            )?
            else {
                return Err(unsupported_target());
            };
            done(
                pending.bytes_consumed,
                Deferred::MaterialController(pending),
            )
        }
        // 006D0D30 and 006DE410 (`morph`).
        0x006D_1520 => {
            let Some(auxiliary) = runtime.stream_auxiliary_04 else {
                return Err(unsupported_target());
            };
            let pending = super::morph::load_geom_morpher_controller(
                world, entity, bytes, version, auxiliary,
            )?;
            done(pending.bytes_consumed, Deferred::GeomMorpher(pending))
        }
        0x006D_DF90 => {
            let Some(auxiliary) = runtime.stream_auxiliary_04 else {
                return Err(unsupported_target());
            };
            let (data, consumed) =
                super::morph::parse_morph_data_payload(bytes, version, auxiliary)?;
            *component_mut::<super::morph::MorphData>(world, entity)? = data;
            Ok((consumed, None))
        }
        // 006DA7E0: the data reference is immediate.
        0x006D_A740 => {
            let Some(consumed) = super::material_controllers::load_point3_interpolator(
                world, entity, bytes, version, table,
            )?
            else {
                return Err(unsupported_target());
            };
            Ok((consumed, None))
        }
        // 006EAD00.
        0x006E_A8C0 => {
            let pending =
                super::particle_blend_interpolator::parse_particle_blend_point3_payload(bytes)?;
            require(super::material_controllers::load_blend_point3_interpolator(
                world,
                entity,
                &pending,
                table,
                [f32::from_bits(super::material_controllers::INVALID_POINT_BITS); 3],
            )?)?;
            Ok((pending.bytes_consumed, None))
        }
        // 007414A0.
        0x0073_D670 => {
            let pending = super::properties::parse_specular_property_payload(bytes, version)?;
            done(pending.bytes_consumed, Deferred::Specular(pending))
        }
        0x0070_9710 => {
            let pending = parse_material_property_payload(bytes, version)?;
            done(pending.bytes_consumed, Deferred::Material(pending))
        }
        // LoadBinary stores scalars, name and order before any LinkObject: a forward system attachment must already
        // see the loaded metadata. References stay deferred, so these load with a null index.
        0x0075_A360 => {
            let pending = parse_particle_color_payload(bytes, version)?;
            let mut fields = pending.clone();
            fields.data_index = NULL_REFERENCE;
            let mut owner = component_mut::<ParticleColorModifier>(world, entity)?;
            resolve_particle_color_links(&mut owner, &fields, None, count)?;
            done(pending.bytes_consumed, Deferred::Color(pending))
        }
        0x0075_BD90 => {
            let pending = parse_particle_age_death_payload(bytes, version)?;
            let mut fields = pending.clone();
            fields.spawn_index = NULL_REFERENCE;
            let mut owner = component_mut::<ParticleAgeDeathModifier>(world, entity)?;
            resolve_particle_age_death_link(&mut owner, &fields, count, None)?;
            done(pending.bytes_consumed, Deferred::AgeDeath(pending))
        }
        0x0074_FDE0 => {
            let pending = parse_particle_emitter_controller_payload(bytes, version)?;
            require_loaded(load_particle_emitter_controller(
                world, entity, &pending, table,
            )?)?;
            done(pending.bytes_consumed, Deferred::EmitterController(pending))
        }
        0x0075_6730 | 0x0075_7170 | 0x0075_7F40 => {
            let pending = parse_particle_modifier_value_controller_payload(bytes, version)?;
            require_loaded(load_particle_modifier_value_controller(
                world, entity, &pending, table,
            )?)?;
            done(
                pending.bytes_consumed,
                Deferred::ModifierValueController(pending),
            )
        }
        // 006CDC10 reads each capacity slot through 006CD680 before its single +0x18 reference.
        0x006E_B4C0 => {
            let pending = parse_particle_blend_bool_payload(bytes)?;
            require_loaded(load_particle_blend_bool(world, entity, &pending, table)?)?;
            Ok((pending.bytes_consumed, None))
        }
        0x006D_24E0 => {
            let pending = parse_particle_blend_float_payload(bytes)?;
            require_loaded(load_particle_blend_float(world, entity, &pending, table)?)?;
            Ok((pending.bytes_consumed, None))
        }
        0x006C_BCE0 => {
            let pending = parse_particle_blend_transform_payload(bytes, version)?;
            require_loaded(load_particle_blend_transform(
                world, entity, &pending, table,
            )?)?;
            Ok((pending.bytes_consumed, None))
        }
        0x0074_9FE0 => {
            let pending = parse_particle_system_payload(bytes, version)?;
            component_mut::<ParticleSystem>(world, entity)?.apply_loaded_payload(&pending)?;
            done(pending.bytes_consumed, Deferred::System(Box::new(pending)))
        }
        0x0074_CFE0 => {
            let pending = parse_particle_sphere_emitter_payload(bytes, version)?;
            let mut owner = component_mut::<ParticleSphereEmitter>(world, entity)?;
            resolve_particle_sphere_emitter_link(&mut owner, &pending, count)?;
            done(pending.bytes_consumed, Deferred::SphereEmitter(pending))
        }
        0x0074_CC70 => {
            let pending = parse_particle_cylinder_emitter_payload(bytes, version)?;
            let mut owner = component_mut::<ParticleCylinderEmitter>(world, entity)?;
            resolve_particle_cylinder_emitter_link(&mut owner, &pending, count)?;
            done(pending.bytes_consumed, Deferred::CylinderEmitter(pending))
        }
        0x0074_D2D0 => {
            let pending = parse_particle_box_emitter_payload(bytes, version)?;
            let mut owner = component_mut::<ParticleBoxEmitter>(world, entity)?;
            resolve_particle_box_emitter_link(&mut owner, &pending, count)?;
            done(pending.bytes_consumed, Deferred::BoxEmitter(pending))
        }
        0x0075_0BA0 => {
            let pending = parse_particle_gravity_payload(bytes, version)?;
            *component_mut::<ParticleGravityModifier>(world, entity)? = pending.modifier.clone();
            done(pending.bytes_consumed, Deferred::Gravity(pending))
        }
        0x0074_E1C0 => {
            let pending = parse_particle_rotation_payload(bytes, version)?;
            *component_mut::<ParticleRotationModifier>(world, entity)? = pending.modifier.clone();
            done(pending.bytes_consumed, Deferred::Rotation(pending))
        }
        0x0075_1D10 => {
            let pending = parse_particle_spawn_payload(bytes, version)?;
            let mut owner = component_mut::<ParticleSpawnModifier>(world, entity)?;
            resolve_particle_spawn_link(&mut owner, &pending, count)?;
            done(pending.bytes_consumed, Deferred::BaseTarget)
        }
        0x0075_8960 => {
            let pending = parse_particle_drag_payload(bytes, version)?;
            let mut owner = component_mut::<ParticleDragModifier>(world, entity)?;
            resolve_particle_drag_link(&mut owner, &pending, count)?;
            done(pending.bytes_consumed, Deferred::Drag(pending))
        }
        0x0075_A9E0 => {
            let pending = parse_particle_bound_update_payload(bytes, version)?;
            let mut owner = component_mut::<ParticleBoundUpdateModifier>(world, entity)?;
            resolve_particle_bound_update_link(&mut owner, &pending, count)?;
            done(pending.bytes_consumed, Deferred::BaseTarget)
        }
        0x0075_5690 => {
            let pending = parse_particle_position_payload(bytes, version)?;
            let mut owner = component_mut::<ParticlePositionModifier>(world, entity)?;
            resolve_particle_position_link(&mut owner, &pending, count)?;
            done(pending.bytes_consumed, Deferred::BaseTarget)
        }
        0x0075_6DA0 => {
            let pending = parse_particle_grow_fade_payload(bytes, version)?;
            let mut owner = component_mut::<ParticleGrowFadeModifier>(world, entity)?;
            resolve_particle_grow_fade_link(&mut owner, &pending, count)?;
            done(pending.bytes_consumed, Deferred::BaseTarget)
        }
        // 006FA790 -> 00730A50: name then u32; link 006FAEC0 -> 00721550 reads nothing at >= 0500000B.
        0x006F_A900 => {
            let pending = parse_bsx_flags_payload(bytes, version)?;
            load_bsx_flags(world, entity, &pending)?;
            Ok((pending.bytes_consumed, None))
        }
        // 00897B90, 0089F8E0 and 008C8A30, load only (`bs::havok_stream`). NiStream +0x04 is the file's second user version.
        0x008B_71B0 | 0x0089_E960 => {
            let pending = parse_sp_collision_payload(bytes, version)?;
            component_mut::<BhkSpCollisionObject>(world, entity)?.flags_0c = pending.flags;
            done(pending.bytes_consumed, Deferred::SpCollision(pending))
        }
        0x008A_F070 => {
            let Some(auxiliary) = runtime.stream_auxiliary_04 else {
                return Err(unsupported_target());
            };
            let pending = parse_phantom_payload(bytes, version, auxiliary)?;
            component_mut::<BhkSimpleShapePhantom>(world, entity)?.cinfo_0c = pending.cinfo.clone();
            done(
                pending.bytes_consumed,
                Deferred::Phantom(pending.shape_index),
            )
        }
        // 008A44F0, both rigid body classes.
        0x008A_41F0 | 0x008B_97A0 => {
            let Some(auxiliary) = runtime.stream_auxiliary_04 else {
                return Err(unsupported_target());
            };
            let (body, pending, consumed) = parse_rigid_body_payload(bytes, version, auxiliary)?;
            *component_mut::<BhkRigidBody>(world, entity)? = body;
            done(consumed, Deferred::RigidBody(pending))
        }
        0x008C_62B0 | 0x008C_33C0 | 0x008B_7E10 | 0x008B_6780 | 0x008C_93F0 | 0x008A_0EF0
        | 0x008A_1B80 | 0x008A_F380 => {
            let Some(auxiliary) = runtime.stream_auxiliary_04 else {
                return Err(unsupported_target());
            };
            let kind = match constructor {
                0x008C_62B0 => BhkShapeKind::NiTriStrips,
                0x008C_33C0 => BhkShapeKind::MoppBvTree,
                0x008B_7E10 => BhkShapeKind::Box,
                0x008B_6780 => BhkShapeKind::Capsule,
                0x008A_F380 => BhkShapeKind::Sphere,
                0x008C_93F0 => BhkShapeKind::ConvexTransform,
                0x008A_1B80 => BhkShapeKind::Transform,
                _ => BhkShapeKind::List,
            };
            let (shape, pending, consumed) = parse_shape_payload(bytes, version, auxiliary, kind)?;
            *component_mut::<BhkShape>(world, entity)? = shape;
            done(consumed, Deferred::Shape(pending))
        }
        // 0088F2D0 bhkBlendCollisionObject_LoadBinary; its link is the collision object's, then 0088ECD0's flag and pair.
        0x0088_ED20 => {
            let Some(auxiliary) = runtime.stream_auxiliary_04 else {
                return Err(unsupported_target());
            };
            let (pending, floats, consumed) =
                crate::bs::havok_stream::parse_blend_collision_payload(bytes, version, auxiliary)?;
            component_mut::<BhkSpCollisionObject>(world, entity)?.flags_0c = pending.flags;
            let mut blend =
                component_mut::<crate::bs::havok_stream::BhkBlendCollisionObject>(world, entity)?;
            blend.float_14 = floats[0];
            blend.float_18 = floats[1];
            done(consumed, Deferred::BlendCollision(pending))
        }
        // 008BF130 bhkMalleableConstraint_LoadBinary.
        0x008B_EFC0 => {
            let Some(auxiliary) = runtime.stream_auxiliary_04 else {
                return Err(unsupported_target());
            };
            let (constraint, malleable, entities, consumed) =
                crate::bs::havok_stream::parse_malleable_constraint_payload(
                    bytes, version, auxiliary,
                )?;
            *component_mut::<BhkConstraint>(world, entity)? = constraint;
            *component_mut::<crate::bs::havok_stream::BhkMalleableConstraint>(world, entity)? =
                malleable;
            done(consumed, Deferred::Constraint(entities))
        }
        // 008A0940, the three constraint classes.
        0x008C_09E0 | 0x008B_2BE0 | 0x008C_17B0 => {
            let Some(auxiliary) = runtime.stream_auxiliary_04 else {
                return Err(unsupported_target());
            };
            let (constraint, entities, consumed) = parse_constraint_payload(
                bytes,
                version,
                auxiliary,
                match constructor {
                    0x008C_09E0 => ConstraintKind::Ragdoll,
                    0x008B_2BE0 => ConstraintKind::LimitedHinge,
                    _ => ConstraintKind::Prismatic,
                },
            )?;
            *component_mut::<BhkConstraint>(world, entity)? = constraint;
            done(consumed, Deferred::Constraint(entities))
        }
        // 008A2600 -> 0089D6C0 reads no reference.
        0x008C_8900 => {
            let (shape, consumed) = parse_convex_vertices_payload(bytes, version)?;
            *component_mut::<BhkConvexVerticesShape>(world, entity)? = shape;
            Ok((consumed, None))
        }
        // 006FF310 publishes the base and damping during LoadBinary; the 00752CB0 link consumes the target.
        0x006F_F000 => {
            let pending = parse_bs_parent_velocity_payload(bytes, version)?;
            load_bs_parent_velocity(world, entity, &pending, table)?;
            done(pending.bytes_consumed, Deferred::BaseTarget)
        }
        0x006D_2DD0 => Ok((
            load_float_interpolator_payload(world, table, index, bytes, version)?,
            None,
        )),
        0x006E_8370 => Ok((
            load_bool_interpolator_payload(world, table, index, bytes, version)?,
            None,
        )),
        0x006E_7E70 => Ok((
            load_bool_timeline_interpolator_payload(world, table, index, bytes, version)?,
            None,
        )),
        0x006D_66E0 => Ok((
            load_transform_interpolator_payload(world, table, index, bytes, version)?,
            None,
        )),
        0x006E_5BF0 => Ok((
            super::bspline::load_comp_transform_payload(world, table, entity, bytes, version)?,
            None,
        )),
        0x006E_6220 => Ok((
            super::bspline::load_comp_float_payload(world, table, entity, bytes)?,
            None,
        )),
        0x006E_71E0 => Ok((
            super::bspline::load_data_payload(world, entity, bytes, version)?,
            None,
        )),
        0x006E_7990 => Ok((
            super::bspline::load_basis_payload(world, entity, bytes)?,
            None,
        )),
        0x0070_21F0 => {
            let pending = parse_source_texture_payload(bytes, version)?;
            let Some(services) = runtime.source_textures.as_mut() else {
                return Err(Stop::Code(StreamGraphErrorCode::UnsupportedLinkService));
            };
            let loaded = component_mut::<SourceTexture>(world, entity)?.load(
                entity,
                &pending,
                Some(&mut **services),
            );
            if loaded.status != SourceTextureLoadStatus::Loaded {
                return Err(Stop::Code(StreamGraphErrorCode::UnsupportedLinkService));
            }
            if loaded.registered {
                palette_textures.push(entity);
            }
            // 00712980 stream +0x24 replaces the slot at the +0x268 cursor; 00702765 releases this duplicate.
            if let Some(replacement) = loaded.replacement {
                table.objects[index].entity = replacement;
                palette_textures.push(replacement);
                world.despawn(entity);
            }
            done(pending.bytes_consumed, Deferred::SourceTexture(pending))
        }
        0x0070_4A60 => {
            let flags = component_mut::<TexturingProperty>(world, entity)?.flags_18;
            let pending = parse_texturing_property_payload(bytes, version, flags)?;
            component_mut::<TexturingProperty>(world, entity)?.apply_loaded_payload(&pending);
            done(
                pending.bytes_consumed,
                Deferred::Texturing(Box::new(pending)),
            )
        }
        0x006D_21D0 => {
            let pending = parse_flip_controller_payload(bytes, version)?;
            require(load_flip_controller(world, entity, &pending, table)?)?;
            done(pending.bytes_consumed, Deferred::Flip(pending))
        }
        0x006D_7320 => {
            let pending = parse_texture_transform_controller_payload(bytes, version)?;
            require(load_texture_transform_controller(
                world, entity, &pending, table,
            )?)?;
            done(pending.bytes_consumed, Deferred::TextureTransform(pending))
        }
        0x006D_0260 => {
            let pending = parse_multi_target_transform_controller_payload(bytes, version)?;
            require(load_multi_target_transform_controller(
                world, entity, &pending, table,
            )?)?;
            done(pending.bytes_consumed, Deferred::MultiTarget(pending))
        }
        0x006C_7840 => {
            let Some(auxiliary) = runtime.stream_auxiliary_04 else {
                return Err(Stop::Code(StreamGraphErrorCode::UnsupportedLinkService));
            };
            let pending = parse_controller_sequence_payload(bytes, version, auxiliary)?;
            require(load_controller_sequence(world, entity, &pending, table)?)?;
            done(pending.bytes_consumed, Deferred::Sequence(pending))
        }
        0x006C_5BB0 => {
            let pending = parse_controller_manager_payload(bytes, version)?;
            require(load_controller_manager(world, entity, &pending, table)?)?;
            done(pending.bytes_consumed, Deferred::Manager(pending))
        }
        0x006D_C5E0 => {
            let pending = parse_path_interpolator_payload(bytes, version)?;
            component_mut::<PathInterpolator>(world, entity)?.apply_loaded_fields(&pending);
            done(pending.bytes_consumed, Deferred::Path(pending))
        }
        0x006C_3F00 => {
            let pending = parse_single_interp_controller_payload(bytes, version)?;
            require(load_transform_controller(world, entity, &pending, table)?)?;
            done(pending.bytes_consumed, single_interp_deferred(&pending))
        }
        // 008AB7B0 bhkBlendController_LoadBinary.
        0x008A_A920 => {
            let pending = super::bone_lod::parse_bhk_blend_payload(bytes, version)?;
            super::bone_lod::load_bhk_blend_controller(world, entity, &pending)?;
            done(
                pending.bytes_consumed,
                Deferred::NodeController(pending.time),
            )
        }
        // 006FDC30 NiBSBoneLODController_LoadBinary.
        0x006F_D6E0 => {
            let pending = super::bone_lod::parse_bone_lod_payload(bytes, version)?;
            super::bone_lod::load_bone_lod_controller(world, entity, &pending)?;
            done(pending.bytes_consumed, Deferred::BoneLod(pending))
        }
        0x006D_4620 => {
            let pending = parse_single_interp_controller_payload(bytes, version)?;
            require(load_vis_controller(world, entity, &pending, table)?)?;
            done(pending.bytes_consumed, single_interp_deferred(&pending))
        }
        // 007166B0 uses immediate references, so nothing is deferred.
        0x0071_6A40 => {
            let pending = parse_av_object_palette_payload(bytes, version)?;
            match apply_av_object_palette_payload(world, entity, &pending, table)? {
                AvObjectPaletteLinkStatus::Linked => Ok((pending.bytes_consumed, None)),
                AvObjectPaletteLinkStatus::UnsupportedTarget => Err(unsupported_target()),
            }
        }
        _ => {
            let loaded = load_factory_payload_into_owner(
                world,
                constructor,
                entity,
                bytes,
                version,
                groups,
            )?;
            match loaded.status {
                FactoryPayloadStatus::Decoded => {}
                FactoryPayloadStatus::OwnerMismatch => {
                    return Err(Stop::Code(StreamGraphErrorCode::OwnerMismatch));
                }
                FactoryPayloadStatus::UnsupportedVersion => {
                    return Err(Stop::Code(StreamGraphErrorCode::UnsupportedPayloadVersion));
                }
                FactoryPayloadStatus::UnsupportedTarget | FactoryPayloadStatus::Unregistered => {
                    return Err(Stop::Code(StreamGraphErrorCode::UnsupportedPayloadTarget));
                }
            }
            let deferred = match loaded.pending {
                // The immediate-reference classes have their own arms above, so the shared decoder never gets them here.
                FactoryPendingPayload::None
                | FactoryPendingPayload::BlendBool(_)
                | FactoryPendingPayload::BlendFloat(_) => None,
                FactoryPendingPayload::TimeController(pending) => {
                    Some(Deferred::TimeController(pending))
                }
                FactoryPendingPayload::EmitterController(pending) => {
                    Some(Deferred::EmitterController(pending))
                }
                FactoryPendingPayload::ModifierValueController(pending) => {
                    Some(Deferred::ModifierValueController(pending))
                }
                FactoryPendingPayload::MeshEmitter(pending) => Some(Deferred::MeshEmitter(pending)),
                FactoryPendingPayload::Geometry(_) => Some(Deferred::Geometry),
                FactoryPendingPayload::SkinInstance(_) => Some(Deferred::SkinInstance),
                FactoryPendingPayload::Node(pending) => Some(Deferred::Node(Box::new(pending))),
                FactoryPendingPayload::Billboard(pending) => {
                    Some(Deferred::Billboard(Box::new(pending)))
                }
                FactoryPendingPayload::Light(_) => Some(Deferred::Light),
            };
            Ok((loaded.bytes_consumed, deferred))
        }
    }
}

// IDA: N/A a helper, with no address of its own.
const fn single_interp_deferred(pending: &SingleInterpControllerPayload) -> Deferred {
    Deferred::NodeController(pending.time)
}

/// IDA: 0070B530's child and effect lists from the table: raw slot order, null slots kept. A child or effect of an
/// unbound class stops with `None`.
/// TODO: [ ] Other NiDynamicEffect classes in the effect list.
#[allow(clippy::type_complexity)]
fn resolve_scene_node_slots(
    world: &World,
    payload: &SceneNodePayload,
    table: &StreamObjectTable,
) -> Result<Option<(Vec<Option<Entity>>, Vec<Option<Entity>>)>, StreamError> {
    let mut children = Vec::with_capacity(payload.child_indices.len());
    for &reference in &payload.child_indices {
        if reference == NULL_REFERENCE {
            children.push(None);
            continue;
        }
        let entry = table
            .objects
            .get(reference as usize)
            .ok_or(StreamError("NiNode child index out of range"))?;
        // IDA: 0070B250 SetChildAt accepts any NiAVObject*; these are the host's bound classes.
        let bound = matches!(
            world.get::<NiClass>(entry.entity),
            Some(
                NiClass::Node
                    | NiClass::BillboardNode
                    | NiClass::TriShape
                    | NiClass::TriStrips
                    | NiClass::ParticleSystem
                    | NiClass::DirectionalLight
            )
        );
        if !bound {
            return Ok(None);
        }
        children.push(Some(entry.entity));
    }
    let mut effects = Vec::with_capacity(payload.effect_indices.len());
    for &reference in &payload.effect_indices {
        if reference == NULL_REFERENCE {
            effects.push(None);
            continue;
        }
        let entry = table
            .objects
            .get(reference as usize)
            .ok_or(StreamError("NiNode effect index out of range"))?;
        if world.get::<NiClass>(entry.entity) != Some(&NiClass::DirectionalLight) {
            return Ok(None);
        }
        effects.push(Some(entry.entity));
    }
    Ok(Some((children, effects)))
}

/// IDA: see the words here, which name the addresses this relates to.
/// 00700050 stores the controller head first, then the node links.
fn link_node_arm(
    world: &mut World,
    entity: Entity,
    payload: &SceneNodePayload,
    table: &StreamObjectTable,
    billboard: bool,
) -> Result<(), Stop> {
    let Some((children, effects)) = resolve_scene_node_slots(world, payload, table)? else {
        return Err(unsupported_target());
    };
    let head = payload.net.controller_index;
    if head != NULL_REFERENCE && head as usize >= table.objects.len() {
        return Err(Stop::Stream(StreamError(if billboard {
            "NiBillboardNode controller out of range"
        } else {
            "NiNode controller index out of range"
        })));
    }
    require(publish_node_controller_head(
        world,
        entity,
        payload.net.controller_index,
        table,
    )?)?;
    let status = if billboard {
        link_billboard_node(world, entity, &children, &effects, table)?
    } else {
        link_scene_node(world, entity, &children, &effects, table)?
    };
    require(status == SceneNodeLinkStatus::Linked)
}

// IDA: N/A a helper, with no address of its own.
fn typed_reference(
    world: &World,
    table: &StreamObjectTable,
    reference: u32,
    class: NiClass,
    out_of_range: &'static str,
) -> Result<Option<Entity>, Stop> {
    if reference == NULL_REFERENCE {
        return Ok(None);
    }
    let entry = table
        .objects
        .get(reference as usize)
        .ok_or(Stop::Stream(StreamError(out_of_range)))?;
    if world.get::<NiClass>(entry.entity) != Some(&class) {
        return Err(unsupported_target());
    }
    Ok(Some(entry.entity))
}

// IDA: N/A a helper, with no address of its own.
fn controller_status(status: ControllerLinkStatus) -> Result<(), Stop> {
    match status {
        ControllerLinkStatus::Linked => Ok(()),
        ControllerLinkStatus::UnsupportedOwner => Err(unsupported_target()),
        ControllerLinkStatus::UnsupportedService => {
            Err(Stop::Code(StreamGraphErrorCode::UnsupportedLinkService))
        }
    }
}

/// IDA: 00700050 / 0075A470 link dispatch, one LinkObject.
#[allow(clippy::too_many_lines)]
fn link_object(
    world: &mut World,
    table: &StreamObjectTable,
    index: usize,
    deferred: &Deferred,
    version: u32,
    runtime: &mut StreamGraphLinkRuntime<'_>,
) -> Result<(), Stop> {
    let entity = table.objects[index].entity;
    match deferred {
        Deferred::Alpha(pending) => resolve_alpha_property_links(world, entity, pending, table)?,
        Deferred::ZBuffer(pending) => resolve_property_links(world, entity, pending, table)?,
        Deferred::VertexColor(pending) => resolve_property_links(world, entity, pending, table)?,
        Deferred::Stencil(pending) => resolve_property_links(world, entity, pending, table)?,
        Deferred::GeomMorpher(pending) => require(super::morph::link_geom_morpher_controller(
            world, entity, pending, table,
        )?)?,
        Deferred::MaterialController(pending) => require(
            super::material_controllers::link_material_controller(world, entity, pending, table)?,
        )?,
        // 00741340 NiAlphaProperty_LinkObject, shared: 00700050.
        Deferred::Specular(pending) => resolve_property_links(world, entity, pending, table)?,
        Deferred::Material(pending) => {
            resolve_material_property_links(world, entity, pending, table)?
        }
        Deferred::Color(pending) => {
            link_particle_color::<ColorKeyData>(world, entity, pending, table)?
        }
        Deferred::BaseTarget => {}
        Deferred::SpCollision(pending) => {
            let target = raw_object_reference(table, pending.target_index)?.0;
            let body = raw_object_reference(table, pending.body_index)?.0;
            let mut object = component_mut::<BhkSpCollisionObject>(world, entity)?;
            object.target_08 = target;
            object.body_10 = body;
            if let Some(target) = target {
                if let Some(mut node) = world.get_mut::<SceneNodePayload>(target) {
                    node.effective_av_flags = collision_target_flags(node.effective_av_flags);
                } else if let Some(mut mesh) =
                    world.get_mut::<super::mesh_geometry::MeshTriShapePayload>(target)
                {
                    mesh.effective_av_flags = collision_target_flags(mesh.effective_av_flags);
                } else {
                    return Err(unsupported_target());
                }
            }
        }
        Deferred::BlendCollision(pending) => {
            let target = raw_object_reference(table, pending.target_index)?.0;
            let body = raw_object_reference(table, pending.body_index)?.0;
            {
                let mut object = component_mut::<BhkSpCollisionObject>(world, entity)?;
                object.target_08 = target;
                object.body_10 = body;
            }
            if let Some(target) = target {
                if let Some(mut node) = world.get_mut::<SceneNodePayload>(target) {
                    node.effective_av_flags = collision_target_flags(node.effective_av_flags);
                } else if let Some(mut mesh) =
                    world.get_mut::<super::mesh_geometry::MeshTriShapePayload>(target)
                {
                    mesh.effective_av_flags = collision_target_flags(mesh.effective_av_flags);
                } else {
                    return Err(unsupported_target());
                }
            }
            // 0088ECD0: +0x0C |= 8, then the body part's pair (00B2E660 by the body's filter bits 8..12) into +0x14 and +0x18. Host
            // substitution (D2): the original reads the hk body's filter; the port reads the loaded cinfo's word 0, which builds it.
            component_mut::<BhkSpCollisionObject>(world, entity)?.flags_0c |= 8;
            let part = body
                .and_then(|body| world.get::<BhkRigidBody>(body))
                .map_or(0, |body| {
                    (u32::from_le_bytes(body.cinfo_0c[..4].try_into().expect("a cinfo")) >> 8)
                        & 0x1F
                });
            let pair = crate::bs::havok_stream::blend_part_pair(
                world.get_resource::<crate::tes::blend_settings::BlendSettingsState>(),
                part,
            );
            let mut blend =
                component_mut::<crate::bs::havok_stream::BhkBlendCollisionObject>(world, entity)?;
            blend.float_14 = pair[0].to_bits();
            blend.float_18 = pair[1].to_bits();
        }
        Deferred::Phantom(shape_index) => {
            let shape = raw_object_reference(table, *shape_index)?.0;
            component_mut::<BhkSimpleShapePhantom>(world, entity)?.shape = shape;
        }
        // 008A50E0: 0089F350's shape, then each nonnull list entry whose RTTI chain holds bhkConstraint (00BA7D50) or
        // bhkAction (00BA7D04) goes in through 008A4070, the newest at the +0x10 head.
        Deferred::RigidBody(pending) => {
            let shape = raw_object_reference(table, pending.shape_index)?.0;
            let mut kept = Vec::new();
            for &index in &pending.constraint_indices {
                if let Some(other) = raw_object_reference(table, index)?.0
                    && matches!(
                        world.get::<NiClass>(other),
                        Some(
                            NiClass::BhkRagdollConstraint
                                | NiClass::BhkLimitedHingeConstraint
                                | NiClass::BhkPrismaticConstraint
                                | NiClass::BhkMalleableConstraint
                        )
                    )
                {
                    kept.insert(0, other);
                }
            }
            let mut body = component_mut::<BhkRigidBody>(world, entity)?;
            body.shape = shape;
            body.constraints_10 = kept;
        }
        Deferred::Shape(pending) => {
            let mut shapes = Vec::new();
            for &index in &pending.indices {
                let other = raw_object_reference(table, index)?.0;
                // 008A15C0 keeps nonnull children only.
                if pending.kind != BhkShapeKind::List || other.is_some() {
                    shapes.push(other);
                }
            }
            component_mut::<BhkShape>(world, entity)?.shapes = shapes;
        }
        // 008A0320: the list's count, then two references; +0x0C takes the first and +0x10 the second only when the first is
        // nonnull. A count other than two would leave the stream's references out of step, a host stop.
        Deferred::Constraint(indices) => {
            if indices.len() != 2 {
                return Err(unsupported_target());
            }
            let first = raw_object_reference(table, indices[0])?.0;
            let second = raw_object_reference(table, indices[1])?.0;
            let mut constraint = component_mut::<BhkConstraint>(world, entity)?;
            constraint.entity_a = first;
            if first.is_some() {
                constraint.entity_b = second;
            }
        }
        Deferred::AgeDeath(pending) => link_particle_age_death(world, entity, pending, table)?,
        Deferred::Drag(pending) => link_particle_drag(world, entity, pending, table)?,
        Deferred::Rotation(pending) => link_particle_rotation(world, entity, pending, table)?,
        Deferred::Gravity(pending) => link_particle_gravity(world, entity, pending, table)?,
        // 00753030: 00752CB0 consumes the base target, then +0x50 takes the next reference as a raw pointer.
        Deferred::BoxEmitter(pending) => {
            let reference = raw_object_reference(table, pending.emitter_object_index)?;
            component_mut::<ParticleBoxEmitter>(world, entity)?.emitter_object_50 = reference;
        }
        Deferred::SphereEmitter(pending) => {
            let reference = raw_object_reference(table, pending.base.emitter_object_index)?;
            component_mut::<ParticleSphereEmitter>(world, entity)?.emitter_object_50 = reference;
        }
        Deferred::CylinderEmitter(pending) => {
            let reference = raw_object_reference(table, pending.base.emitter_object_index)?;
            component_mut::<ParticleCylinderEmitter>(world, entity)?.emitter_object_50 = reference;
        }
        Deferred::MeshEmitter(pending) => {
            require(
                link_particle_mesh_emitter(world, entity, pending, table)?
                    == MeshTriShapeLinkStatus::Linked,
            )?;
        }
        Deferred::System(pending) => {
            require(
                resolve_particle_system_links(world, entity, pending, table)?
                    == ParticleSystemLinkStatus::Linked,
            )?;
        }
        Deferred::TimeController(pending) => {
            controller_status(link_particle_time_controller(
                world, entity, pending, table, version,
            )?)?;
        }
        Deferred::EmitterController(pending) => {
            controller_status(link_particle_emitter_controller(
                world,
                entity,
                pending,
                table,
                version,
                runtime.draw_rand.as_deref_mut(),
                runtime.draw_on_io_thread,
            )?)?;
        }
        Deferred::ModifierValueController(pending) => {
            controller_status(link_particle_modifier_value_controller(
                world, entity, pending, table, version,
            )?)?;
        }
        Deferred::Geometry => {
            require(
                link_mesh_geometry(world, entity, table, version)?
                    == MeshTriShapeLinkStatus::Linked,
            )?;
        }
        Deferred::SkinInstance => {
            require(link_mesh_skin_instance(world, entity, table)? == MeshSkinLinkStatus::Linked)?;
        }
        Deferred::Node(payload) => link_node_arm(world, entity, payload, table, false)?,
        Deferred::Billboard(pending) => link_node_arm(world, entity, &pending.node, table, true)?,
        Deferred::Light => {
            if link_directional_light(world, entity, table, table.objects.len())?
                == DirectionalLightLinkStatus::UnsupportedOwner
            {
                return Err(unsupported_target());
            }
        }
        // 006FD750: 00715E70's next and target, then the groups' nodes.
        Deferred::BoneLod(pending) => {
            require(link_node_controller(
                world,
                entity,
                &pending.time,
                table,
                version,
            )?)?;
            super::bone_lod::link_bone_lod_groups(world, entity, pending, table)?;
        }
        // 006C3DB0 / 006D44F0 -> 00715E70: next controller, then the raw target.
        Deferred::NodeController(time) => {
            require(link_node_controller(world, entity, time, table, version)?)?;
        }
        // A79984+20 is 006CE1C0 -> 00715E70 with no derived links.
        Deferred::MultiTarget(pending) => {
            require(link_node_controller(
                world,
                entity,
                &pending.time,
                table,
                version,
            )?)?;
        }
        Deferred::Sequence(pending) => {
            require(link_controller_sequence(world, entity, pending, table)?)?
        }
        Deferred::Manager(pending) => require(link_controller_manager(
            world, entity, pending, table, version,
        )?)?,
        Deferred::SourceTexture(pending) => {
            require(link_source_texture(world, entity, pending, table)?)?
        }
        Deferred::Texturing(pending) => {
            let Some(head) =
                resolve_texture_controller_head(world, pending.net.controller_index, table)?
            else {
                return Err(unsupported_target());
            };
            require(link_texturing_property(
                world, entity, pending, table, head,
            )?)?;
        }
        Deferred::Flip(pending) => require(link_flip_controller(
            world, entity, pending, table, version,
        )?)?,
        Deferred::TextureTransform(pending) => {
            require(link_texture_transform_controller(
                world,
                entity,
                &pending.base.time,
                table,
                version,
            )?)?;
        }
        // 006DC000: 007124A0 path NiPosData (+18), then percent NiFloatData (+1C).
        Deferred::Path(pending) => {
            let path = typed_reference(
                world,
                table,
                pending.path_data_index,
                NiClass::PositionKeyData,
                "path interpolator data out of range",
            )?;
            let percent = typed_reference(
                world,
                table,
                pending.percent_data_index,
                NiClass::FloatKeyData,
                "path interpolator percent out of range",
            )?;
            require(
                link_path_interpolator(world, entity, path, percent) == PathLinkStatus::Linked,
            )?;
        }
    }
    Ok(())
}

/// IDA: N/A dropping a failed table. A texture the palette holds stays, as its C++ `shared_ptr` keeps it, with the
/// NiPixelData its +0x3C holds.
fn despawn_table(world: &mut World, table: &StreamObjectTable, palette_textures: &[Entity]) {
    let pixel_data: Vec<Entity> = palette_textures
        .iter()
        .filter_map(|&texture| world.get::<SourceTexture>(texture)?.app_pixel_data_3c)
        .collect();
    for entry in &table.objects {
        if !palette_textures.contains(&entry.entity)
            && !pixel_data.contains(&entry.entity)
            && world.get_entity(entry.entity).is_ok()
        {
            world.despawn(entry.entity);
        }
    }
}

/// IDA: N/A dropping a finished graph that a later pass failed on: the C++ graph's `shared_ptr` owners go out of scope.
pub fn despawn_stream_graph(world: &mut World, graph: &StreamGraph) {
    despawn_table(world, &graph.table, &graph.palette_textures);
}

struct GraphInput<'b> {
    /// The table's bytes, or the whole contiguous body.
    type_table: &'b [u8],
    payloads: &'b [u8],
    group_header: Option<&'b [u8]>,
    root_records: Option<&'b [u8]>,
    contiguous_body: bool,
    object_count: u32,
    version: u32,
}

/// IDA: 00714390's load, root and link passes in that order. Completion: 100% of the bound classes' order.
/// TODO: [ ] 007135C0 cancellation and the original's partial publication.
fn load_graph(
    world: &mut World,
    registry: &NiFactoryRegistry,
    input: GraphInput<'_>,
    link_runtime: &mut StreamGraphLinkRuntime<'_>,
    progress: &mut dyn StreamGraphProgress,
) -> Result<StreamGraph, StreamGraphError> {
    let plain_error =
        |code, object_index, payload_byte_offset, original_constructor| StreamGraphError {
            code,
            table_error: None,
            object_index,
            payload_byte_offset,
            original_constructor,
            body_byte_offset: None,
        };
    let mut table = read_stream_object_table(
        world,
        input.type_table,
        input.object_count,
        input.version,
        registry,
    )
    .map_err(|error| StreamGraphError {
        table_error: Some(Box::new(error)),
        ..plain_error(StreamGraphErrorCode::TypeTable, None, 0, None)
    })?;
    let version = input.version;
    // 10.1.0.106 reads as 0A010072 past each object's 007008A0 group word (`legacy_group_prefix`).
    let payload_version = if version == LEGACY_GROUP_VERSION {
        MODERN_PAYLOAD_FLOOR
    } else {
        version
    };
    let mut palette_textures: Vec<Entity> = Vec::new();
    if payload_version < MODERN_PAYLOAD_FLOOR {
        despawn_table(world, &table, &palette_textures);
        return Err(plain_error(
            StreamGraphErrorCode::UnsupportedPayloadVersion,
            None,
            0,
            None,
        ));
    }
    let group_header = if input.contiguous_body {
        Some(&input.type_table[table.bytes_consumed..])
    } else {
        input.group_header
    };
    let mut groups = StreamAllocationGroups::default();
    if let Some(header) = group_header {
        match read_stream_allocation_groups(header, version) {
            Ok(read) => groups = read,
            Err(error) => {
                despawn_table(world, &table, &palette_textures);
                return Err(plain_error(
                    StreamGraphErrorCode::Stream(error),
                    None,
                    0,
                    None,
                ));
            }
        }
    }
    let has_groups = group_header.is_some();
    let payloads = if input.contiguous_body {
        &input.type_table[table.bytes_consumed + groups.bytes_consumed..]
    } else {
        input.payloads
    };
    let body_offset =
        |table: &StreamObjectTable, groups: &StreamAllocationGroups, position: usize| {
            input
                .contiguous_body
                .then_some(table.bytes_consumed + groups.bytes_consumed + position)
        };
    let stopped = |world: &mut World,
                   table: &StreamObjectTable,
                   palette_textures: &[Entity],
                   stop: Stop,
                   index: usize,
                   position: usize,
                   body: Option<usize>| {
        despawn_table(world, table, palette_textures);
        StreamGraphError {
            code: match stop {
                Stop::Stream(error) => StreamGraphErrorCode::Stream(error),
                Stop::Code(code) => code,
            },
            table_error: None,
            object_index: Some(index),
            payload_byte_offset: position,
            original_constructor: Some(table.objects[index].original_constructor),
            body_byte_offset: body,
        }
    };
    let decision_stop = |decision| {
        Stop::Code(if decision == StreamGraphProgressDecision::Cancelled {
            StreamGraphErrorCode::Cancelled
        } else {
            StreamGraphErrorCode::UnavailablePauseService
        })
    };

    let mut deferred_links: Vec<(usize, Deferred)> = Vec::new();
    let mut position = 0;
    for index in 0..table.objects.len() {
        let decision = progress.before_load(index);
        if decision != StreamGraphProgressDecision::Proceed {
            let body = body_offset(&table, &groups, position);
            return Err(stopped(
                world,
                &table,
                &palette_textures,
                decision_stop(decision),
                index,
                position,
                body,
            ));
        }
        let prefix = if version == LEGACY_GROUP_VERSION {
            match legacy_group_prefix(world, table.objects[index].entity) {
                Ok(prefix) => prefix,
                Err(stop) => {
                    let body = body_offset(&table, &groups, position);
                    return Err(stopped(
                        world,
                        &table,
                        &palette_textures,
                        stop,
                        index,
                        position,
                        body,
                    ));
                }
            }
        } else {
            0
        };
        let remaining = payloads.get(position + prefix..).unwrap_or_default();
        let group_view = if has_groups { Some(&mut groups) } else { None };
        let object_version = if version == LEGACY_GROUP_VERSION {
            legacy_payload_version(world, table.objects[index].entity)
        } else {
            payload_version
        };
        match load_object(
            world,
            &mut table,
            index,
            remaining,
            object_version,
            group_view,
            link_runtime,
            &mut palette_textures,
        ) {
            Ok((consumed, deferred)) => {
                if let Some(deferred) = deferred {
                    deferred_links.push((index, deferred));
                }
                position += prefix + consumed;
            }
            Err(stop) => {
                let body = body_offset(&table, &groups, position);
                return Err(stopped(
                    world,
                    &table,
                    &palette_textures,
                    stop,
                    index,
                    position,
                    body,
                ));
            }
        }
        progress.after_load(index, &table.objects[index]);
    }

    // IDA: 00714462 root virtual +48 precedes every LinkObject.
    let roots = if input.contiguous_body {
        read_stream_roots(&payloads[position..], &table)
    } else if let Some(records) = input.root_records {
        read_stream_roots(records, &table)
    } else {
        Ok(StreamRoots::default())
    };
    let roots = match roots {
        Ok(roots) => roots,
        Err(error) => {
            despawn_table(world, &table, &palette_textures);
            return Err(plain_error(
                StreamGraphErrorCode::Stream(error),
                None,
                position,
                None,
            ));
        }
    };

    // IDA: 00728F20 retains the additional geometry reference at +0x34. Its owner is not ported; do not publish a graph
    // that silently discards a non-null link.
    for index in 0..table.objects.len() {
        let link = world
            .get::<ParticleData>(table.objects[index].entity)
            .and_then(|data| data.deferred_geometry_link);
        let Some(link) = link else { continue };
        if link == NULL_REFERENCE {
            continue;
        }
        let stop = if link as usize >= table.objects.len() {
            Stop::Stream(StreamError(
                "particle additional geometry link out of range",
            ))
        } else {
            unsupported_target()
        };
        return Err(stopped(
            world,
            &table,
            &palette_textures,
            stop,
            index,
            position,
            None,
        ));
    }

    // IDA: 00714390 completes every LoadBinary before the +0x20 link pass.
    let mut deferred_cursor = 0;
    for index in 0..table.objects.len() {
        let decision = progress.before_link(index);
        if decision != StreamGraphProgressDecision::Proceed {
            let body = body_offset(&table, &groups, position + roots.bytes_consumed);
            return Err(stopped(
                world,
                &table,
                &palette_textures,
                decision_stop(decision),
                index,
                position,
                body,
            ));
        }
        if deferred_links
            .get(deferred_cursor)
            .is_some_and(|(at, _)| *at == index)
        {
            let deferred = &deferred_links[deferred_cursor].1;
            deferred_cursor += 1;
            if let Err(stop) = link_object(
                world,
                &table,
                index,
                deferred,
                payload_version,
                link_runtime,
            ) {
                return Err(stopped(
                    world,
                    &table,
                    &palette_textures,
                    stop,
                    index,
                    position,
                    None,
                ));
            }
        }
        progress.after_link(index);
    }
    let body_bytes_consumed = if input.contiguous_body {
        table.bytes_consumed + groups.bytes_consumed + position + roots.bytes_consumed
    } else {
        0
    };
    Ok(StreamGraph {
        table,
        payload_bytes_consumed: position,
        allocation_groups: groups,
        roots,
        body_bytes_consumed,
        palette_textures,
    })
}

/// IDA: N/A a pre-separated table and payload span, no group header. Host adapter for 00714390.
pub fn load_stream_graph_payloads(
    world: &mut World,
    registry: &NiFactoryRegistry,
    type_table: &[u8],
    payloads: &[u8],
    object_count: u32,
    version: u32,
    link_runtime: &mut StreamGraphLinkRuntime<'_>,
) -> Result<StreamGraph, StreamGraphError> {
    let input = GraphInput {
        type_table,
        payloads,
        group_header: None,
        root_records: None,
        contiguous_body: false,
        object_count,
        version,
    };
    load_graph(world, registry, input, link_runtime, &mut NoProgress)
}

/// IDA: 00714390 -> 00713030 before ordered LoadBinary and LinkObject.
#[allow(clippy::too_many_arguments)]
pub fn load_stream_graph_with_groups(
    world: &mut World,
    registry: &NiFactoryRegistry,
    type_table: &[u8],
    group_header: &[u8],
    payloads: &[u8],
    object_count: u32,
    version: u32,
    link_runtime: &mut StreamGraphLinkRuntime<'_>,
) -> Result<StreamGraph, StreamGraphError> {
    let input = GraphInput {
        type_table,
        payloads,
        group_header: Some(group_header),
        root_records: None,
        contiguous_body: false,
        object_count,
        version,
    };
    load_graph(world, registry, input, link_runtime, &mut NoProgress)
}

/// IDA: 00714390's ordered load, root and link subset.
#[allow(clippy::too_many_arguments)]
pub fn load_stream_graph_with_roots(
    world: &mut World,
    registry: &NiFactoryRegistry,
    type_table: &[u8],
    group_header: &[u8],
    payloads: &[u8],
    root_records: &[u8],
    object_count: u32,
    version: u32,
    link_runtime: &mut StreamGraphLinkRuntime<'_>,
) -> Result<StreamGraph, StreamGraphError> {
    let input = GraphInput {
        type_table,
        payloads,
        group_header: Some(group_header),
        root_records: Some(root_records),
        contiguous_body: false,
        object_count,
        version,
    };
    load_graph(world, registry, input, link_runtime, &mut NoProgress)
}

/// IDA: 00714390 reads one body in table, group, payload and root order, then runs LinkObject. Completion: 100% of the
/// successful modern body cursor.
/// TODO: [ ] Callback cancellation and the original's partial publication.
pub fn load_stream_graph_body(
    world: &mut World,
    registry: &NiFactoryRegistry,
    body: &[u8],
    object_count: u32,
    version: u32,
    link_runtime: &mut StreamGraphLinkRuntime<'_>,
    progress: Option<&mut dyn StreamGraphProgress>,
) -> Result<StreamGraph, StreamGraphError> {
    let input = GraphInput {
        type_table: body,
        payloads: &[],
        group_header: None,
        root_records: None,
        contiguous_body: true,
        object_count,
        version,
    };
    let mut none = NoProgress;
    load_graph(
        world,
        registry,
        input,
        link_runtime,
        progress.unwrap_or(&mut none),
    )
}
