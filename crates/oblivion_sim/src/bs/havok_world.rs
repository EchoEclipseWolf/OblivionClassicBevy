//! Build 3 (goal 1): the bhkWorld and the bhk calls that put a reference's collision into it, Bethesda side, with the Havok
//! SDK objects behind the physics seam (`physics_seam`, a recorded host substitution). `0088AEB0`'s bhkWorld record, the
//! listeners `004D4A20` adds (`004CD320`, `004CD2D0`), the CellMopp (`00533380`), `0088C740` (vt+0x94) and `0088CC80`
//! (vt+0x90), `NiAVObject_VisitCollisionObjects` (`0088A7D0`'s walk) with `004DE010` / `0088A870`, `00889C20`'s system
//! group, `0088D070`. The bodies' shapes go to the seam in Havok's units, converted from the loaded bhk data
//! (`havok_stream`).

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::havok::HavokFrameGlobals;
use super::havok_stream::{
    BhkBlendCollisionObject, BhkConvexVerticesShape, BhkHostScale, BhkRigidBody, BhkShape,
    BhkSimpleShapePhantom, BhkSpCollisionObject,
};
use super::physics_seam::{
    BodyDesc, BodyMotion, PhysicsBodyId, PhysicsTransform, PhysicsWorldId, ShapeDesc, WorldDesc,
};
use crate::bs::character_controller::HAVOK_SCALE;
use crate::gamebryo::av_object::NiAvCollision;
use crate::gamebryo::scene_node::{NiAvWorld, NiNodeChildren, SceneNodePayload};
use crate::gamebryo::stream_objects::NiClass;

/// IDA: ??_7bhkWorld@@6B@, ??_7TESTrapListener@@6B@, ??_7TESWaterListener@@6B@, ??_7TESArrowTargetListener@@6B@,
/// ??_7CellMopp@@6B@, ??_7ExtraHavok@@6B@.
pub const BHK_WORLD_VTABLE: u32 = 0x00A9_5CF4;
pub const TES_TRAP_LISTENER_VTABLE: u32 = 0x00A5_6350;
pub const TES_WATER_LISTENER_VTABLE: u32 = 0x00A5_6384;
pub const TES_ARROW_TARGET_LISTENER_VTABLE: u32 = 0x00A5_62F4;
pub const CELL_MOPP_VTABLE: u32 = 0x00A5_6114;
pub const EXTRA_HAVOK_VTABLE: u32 = 0x00A3_4DD4;

/// IDA: a listener `004D4A20` hands the world, as its constructor leaves it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WorldListener {
    /// `00536E10` TESTrapListener (FormHeap 0x20): `00536060(0)`'s bhkEntityListener, then +0x18 = +0x1C = 0.
    Trap,
    /// `00537E40` TESWaterListener(height) (FormHeap 0x2C): +0x14 1.0, +0x20 1, +0x18 a FormHeap(4) float of the height
    /// times 0.1428767, +0x24 = +0x28 = 0.0.
    Water { height_18: f32 },
    /// `005360F0` TESArrowTargetListener (FormHeap 0x14): `00536060(0)`, its vtable.
    ArrowTarget,
}

/// IDA: `0088AEB0` bhkWorld_Constructor(cinfo) in a 16-aligned 0x90 FormHeap block, Bethesda side: +0x0C and +0x10 0,
/// +0x14 the visual debugger `0088B680` builds, +0x18 / +0x19 / +0x1A bytes, +0x24 the listener `004CD320` keeps, +0x60 the
/// hkArray of listeners (`004CD320` and `004CD2D0` append). The hkpWorld (`00889A70` with cinfo +0xA0) is the seam's world.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct BhkWorld {
    pub original_vtable: u32,
    pub reference_count_04: u32,
    /// +0x14 hkVisualDebugger (`0088B680(1)`; bHavokDebug 0 leaves none).
    pub visual_debugger_14: bool,
    /// +0x18: `0088C740` and `0088CC80` set it when they add.
    pub byte_18: u8,
    /// +0x19: (00B33A34 == 0).
    pub byte_19: u8,
    /// +0x1A: only vt+0x98 (`0088D2B0`) writes it; the constructor's 0 here.
    pub byte_1a: u8,
    /// +0x24.
    pub listener_24: Option<usize>,
    /// +0x60, in append order.
    pub listeners_60: Vec<WorldListener>,
    /// The cinfo's values, as the seam got them.
    pub desc: WorldDesc,
    /// IDA: N/A the seam's world (`None` without a backend).
    pub physics: Option<PhysicsWorldId>,
}

/// IDA: `00533380` CellMopp_Constructor (FormHeap 0x18): NiRefObject, its vtable, +0x08 an NiTArray<NiPointer<bhkRigidBody>>
/// (vtable, +0x0C data, +0x10..+0x14 sizes), +0x2C 1, and 00B36588 raised.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct CellMopp {
    pub original_vtable: u32,
    pub bodies_08: Vec<Entity>,
    pub field_2c: u32,
}

/// IDA: the counters the world code raises: 00BA7900 (`0088AEB0`) and 00B36588 (`00533380`).
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HavokWorldCounters {
    pub worlds_ba7900: u32,
    pub cell_mopps_b36588: u32,
}

/// IDA: a body's hk object in a world (`vt+0x58` answers it): the bhkWorld and the seam's body.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BhkInWorld {
    pub world: Entity,
    pub physics: Option<PhysicsBodyId>,
}

/// IDA: N/A a call `0088D070` makes on each collision object (its vt+0x70 SetMotionType(6, 0)), deferred: the bodies' hk
/// motion state and bhkBlendCollisionObject's `0088F880` stores (+0x0C bits, +0x14 / +0x18) wait for the ragdoll work.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct MotionTypeDeferrals {
    pub calls: Vec<(Entity, u32)>,
}

/// IDA: the 0x1C-byte context `NiAVObject_VisitCollisionObjects` passes its callback: +0x00 the world, +0x04 the recurse
/// byte, +0x08 (zero: each node's flags `& 0xFFE9 | 6`), +0x0C, +0x10 the system group, +0x14 the count, +0x18.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CollisionVisit {
    pub world_00: Option<Entity>,
    pub recurse_04: bool,
    pub field_08: u32,
    pub first_0c: u32,
    pub group_10: u32,
    pub count_14: u32,
    pub mark_18: u32,
}

// IDA: N/A the NiAVObject +0x18 flags of a node or a geometry.
fn av_flags_mut(world: &mut World, node: Entity) -> Option<Mut<'_, u16>> {
    if world.get::<SceneNodePayload>(node).is_some() {
        return world
            .get_mut::<SceneNodePayload>(node)
            .map(|payload| payload.map_unchanged(|payload| &mut payload.effective_av_flags));
    }
    world
        .get_mut::<crate::gamebryo::mesh_geometry::MeshTriShapePayload>(node)
        .map(|payload| payload.map_unchanged(|payload| &mut payload.effective_av_flags))
}

/// IDA: `NiAVObject_GetBhkNiCollisionObject`: +0xA8 cast to bhkNiCollisionObject, here a collision the host links.
#[must_use]
pub fn collision_object(world: &World, node: Entity) -> Option<Entity> {
    world
        .get::<NiAvCollision>(node)
        .map(|collision| collision.0)
        .filter(|&collision| world.get::<BhkSpCollisionObject>(collision).is_some())
}

/// IDA: `0088A7D0` NiAVObject_VisitCollisionObjects(object, context, callback): a null object returns; +0x08 zero sets the
/// object's flags `& 0xFFE9 | 6`; its collision object, if any, goes to the callback; with +0x04 a node's children (vt+8)
/// in slot order, null slots included (they return at once). Completion: 100%.
/// # Errors
/// A stop in the callback.
pub fn visit_collision_objects(
    world: &mut World,
    object: Option<Entity>,
    context: &mut CollisionVisit,
    callback: &mut dyn FnMut(&mut World, Entity, &mut CollisionVisit) -> Result<(), GameError>,
) -> Result<(), GameError> {
    let Some(object) = object else {
        return Ok(());
    };
    let collision = collision_object(world, object);
    if context.field_08 == 0
        && let Some(mut flags) = av_flags_mut(world, object)
    {
        *flags = *flags & 0xFFE9 | 6;
    }
    if let Some(collision) = collision {
        callback(world, collision, context)?;
    }
    if context.recurse_04 {
        let children = world
            .get::<NiNodeChildren>(object)
            .map(|children| children.slots_ac.clone())
            .unwrap_or_default();
        for child in children {
            visit_collision_objects(world, child, context, callback)?;
        }
    }
    Ok(())
}

/// IDA: the body a collision object holds (+0x10) and whether it is a rigid body, for the hk filter (+0x08's +0x30).
fn body_filter(world: &World, body: Entity) -> u32 {
    body_filter_info(world, body)
}

/// IDA: a body's hk collision filter info (the hkCollidable's +0x1C), as the loaded cinfo holds it: layer in bits 0..5,
/// system group in 16..31. 0 for an entity that is neither a rigid body nor a phantom. Completion: 100%.
#[must_use]
pub fn body_filter_info(world: &World, body: Entity) -> u32 {
    let cinfo = world
        .get::<BhkRigidBody>(body)
        .map(|body| body.cinfo_0c.as_slice())
        .or_else(|| {
            world
                .get::<BhkSimpleShapePhantom>(body)
                .map(|phantom| phantom.cinfo_0c.as_slice())
        });
    cinfo.map_or(0, |cinfo| {
        u32::from_le_bytes([cinfo[0], cinfo[1], cinfo[2], cinfo[3]])
    })
}

// IDA: N/A the store behind `body_filter` (the host keeps the hk filter in the loaded cinfo, which builds it: D2).
fn set_body_filter(world: &mut World, body: Entity, filter: u32) {
    let bytes = filter.to_le_bytes();
    if let Some(mut rigid) = world.get_mut::<BhkRigidBody>(body) {
        rigid.cinfo_0c[..4].copy_from_slice(&bytes);
    } else if let Some(mut phantom) = world.get_mut::<BhkSimpleShapePhantom>(body) {
        phantom.cinfo_0c[..4].copy_from_slice(&bytes);
    }
}

/// IDA: `0089F4D0` bhkWorldObject_SetSystemGroup(group) on a loaded body: the hk filter becomes (group << 16) | its low
/// word, then vt+0x80 (the world's update of it: the seam takes the filter when the body enters). Completion: 100%.
fn set_body_group(world: &mut World, body: Entity, group: u32) {
    let filter = body_filter(world, body);
    set_body_filter(world, body, (group << 16) | (filter & 0xFFFF));
}

/// IDA: `00889C20`(body, context): the hk filter's system group (+0x30 >> 16). A context group (+0x10) is stored on the body
/// and answers 1. Otherwise a zero group takes 00BA7EB0[layer] (the 008A83C0 tables), or a new one from
/// g_bhkCollisionGroupCounter (00B2EB3C, 16 bits, 0 becomes 10), and is stored; the group goes to +0x10. Completion: 100%.
fn assign_system_group(world: &mut World, body: Entity, context: &mut CollisionVisit) -> bool {
    let filter = body_filter(world, body);
    let mut group = filter >> 16;
    if context.group_10 != 0 {
        set_body_group(world, body, context.group_10);
        return true;
    }
    let mut changed = false;
    if group == 0 {
        group = world
            .get_resource::<super::havok::HavokGlobals>()
            .map_or(0, |globals| {
                globals.layer_tables[0x40 + (filter & 0x3F) as usize]
            });
        if group == 0 {
            let mut globals =
                world.resource_mut::<crate::bs::character_controller::CharacterControllerGlobals>();
            group = globals.collision_group_b2eb3c.wrapping_add(1) & 0xFFFF;
            if group == 0 {
                group = 10;
            }
            globals.collision_group_b2eb3c = group;
        }
        changed = true;
    }
    context.group_10 = group;
    if changed {
        set_body_group(world, body, group);
    }
    changed
}

/// IDA: `004DE010` (00B2E300's callback): bPreventHavokAddAll (00B09870) marks the target `& 0xFFE9 | 0x10` and clears
/// +0x04; bPreventHavokAddClutter (00B09868) does so for a clutter reference (a stop: no read of 00469520 here); otherwise
/// `0088A870`. Both settings are 0 in the pinned INI. Completion: 100% with both clear.
/// # Errors
/// Either setting set.
pub fn add_collision_callback(
    world: &mut World,
    collision: Entity,
    context: &mut CollisionVisit,
) -> Result<(), GameError> {
    let ini = |world: &World, address: u32| {
        world
            .get_resource::<crate::tes::settings::SettingCollections>()
            .and_then(|collections| {
                collections
                    .ini
                    .find_address(address)
                    .map(|setting| setting.raw)
            })
            .unwrap_or(0)
            & 0xFF
    };
    if ini(world, 0x00B0_9870) != 0 || ini(world, 0x00B0_9868) != 0 {
        return Err(GameError::unclassified(
            0x004DE01D,
            "004DE01D: bPreventHavokAddAll or bPreventHavokAddClutter set",
        ));
    }
    add_collision_to_world(world, collision, context)
}

/// IDA: `0088A870`(collision, context): with a body (+0x10): +0x18 sets or clears the collision's 0x40; a
/// bhkBlendCollisionObject's 0x100 set when +0x14 is 0, cleared otherwise; +0x0C bit 0 sets 4 then 8, otherwise clears 4;
/// 00B2E2D8 (1) runs `00889C20`; a body whose world (vt+0x58) isn't the context's enters it (vt+0x5C: the seam); the target's
/// vt+0x74 (NiAVObject_UpdateWorldTransform, its collision's vt+0x50 clearing 0x40); world +0x1A runs vt+0x7C(1) (a stop:
/// never set here). Then, body or not, +0x0C bit 0 clears and +0x14 counts. Completion: 100%.
/// # Errors
/// World +0x1A set, or a shape the seam can't take.
pub fn add_collision_to_world(
    world: &mut World,
    collision: Entity,
    context: &mut CollisionVisit,
) -> Result<(), GameError> {
    let body = world
        .get::<BhkSpCollisionObject>(collision)
        .and_then(|object| object.body_10);
    if let Some(body) = body {
        {
            let mut object = world
                .get_mut::<BhkSpCollisionObject>(collision)
                .expect("checked");
            if context.mark_18 != 0 {
                object.flags_0c |= 0x40;
            } else {
                object.flags_0c &= 0xFFBF;
            }
        }
        if world.get::<BhkBlendCollisionObject>(collision).is_some() {
            let mut object = world
                .get_mut::<BhkSpCollisionObject>(collision)
                .expect("checked");
            if context.count_14 == 0 {
                object.flags_0c |= 0x100;
            } else {
                object.flags_0c &= 0xFEFF;
            }
        }
        {
            let mut object = world
                .get_mut::<BhkSpCollisionObject>(collision)
                .expect("checked");
            if context.first_0c & 1 != 0 {
                object.flags_0c |= 4;
                object.flags_0c |= 8;
            } else {
                object.flags_0c &= 0xFFFB;
            }
        }
        // 0088A8DA 00B2E2D8, 1 in the image's .data.
        assign_system_group(world, body, context);
        let current = world.get::<BhkInWorld>(body).map(|held| held.world);
        if current != context.world_00 {
            if let Some(target) = context.world_00 {
                enter_world(world, collision, body, target)?;
            } else {
                world.entity_mut(body).remove::<BhkInWorld>();
            }
        }
        let target = world
            .get::<BhkSpCollisionObject>(collision)
            .and_then(|object| object.target_08);
        if let Some(target) = target {
            if world.get::<SceneNodePayload>(target).is_some() {
                crate::gamebryo::scene_node::update_world_transform(
                    world,
                    target,
                    Some(&mut |world: &mut World| {
                        crate::bs::havok_stream::collision_update_world_data(world, collision)
                    }),
                )?;
            } else {
                crate::bs::havok_stream::collision_update_world_data(world, collision);
            }
        }
        if let Some(bhk) = context.world_00
            && world
                .get::<BhkWorld>(bhk)
                .is_some_and(|data| data.byte_1a != 0)
        {
            return Err(GameError::unclassified(
                0x0088A94E,
                "0088A94E: the collision's vt+0x7C(1)",
            ));
        }
    }
    context.first_0c &= !1;
    context.count_14 += 1;
    Ok(())
}

/// IDA: a body's vt+0x5C(world): bhkRigidBody's `008A48C0` / the phantom's: the hk object enters the world. The hk side is
/// the seam's body, placed at its target's world transform (a bhkRigidBodyT composes its cinfo +0x30 translation and +0x40
/// rotation), its shape converted. Completion: 100% of the Bethesda side (the membership).
fn enter_world(
    world: &mut World,
    collision: Entity,
    body: Entity,
    target_world: Entity,
) -> Result<(), GameError> {
    let physics_world = world
        .get::<BhkWorld>(target_world)
        .ok_or(GameError::unclassified(
            0x0088A91E,
            "0088A91E: vt+0x5C with no bhkWorld",
        ))?
        .physics;
    let desc = body_desc(world, collision, body)?;
    let physics = super::physics_seam::seam_add_body(world, physics_world, &desc);
    world
        .get_resource_or_init::<super::physics_seam::PhysicsSeamLog>()
        .body_entities
        .push(body);
    world
        .entity_mut(body)
        .remove::<super::ragdoll::UnattachedBodyPose>()
        .insert(BhkInWorld {
            world: target_world,
            physics,
        });
    Ok(())
}

// IDA: N/A a word of a raw cinfo.
fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

// IDA: N/A a float of a raw cinfo.
fn float(bytes: &[u8], offset: usize) -> f32 {
    f32::from_bits(word(bytes, offset))
}

// IDA: N/A three floats of a raw cinfo.
fn vector(bytes: &[u8], offset: usize) -> [f32; 3] {
    [
        float(bytes, offset),
        float(bytes, offset + 4),
        float(bytes, offset + 8),
    ]
}

/// IDA: N/A the row-major matrix of a unit quaternion (x, y, z, w), as hkRotation builds it.
#[must_use]
pub fn quaternion_matrix(q: [f32; 4]) -> [f32; 9] {
    let [x, y, z, w] = q;
    [
        1.0 - 2.0 * (y * y + z * z),
        2.0 * (x * y - z * w),
        2.0 * (x * z + y * w),
        2.0 * (x * y + z * w),
        1.0 - 2.0 * (x * x + z * z),
        2.0 * (y * z - x * w),
        2.0 * (x * z - y * w),
        2.0 * (y * z + x * w),
        1.0 - 2.0 * (x * x + y * y),
    ]
}

/// IDA: 0043F3E0 HavokVector_ToGameUnits. Three single-precision products.
pub fn to_game_units(vector: [f32; 3]) -> [f32; 3] {
    vector.map(|value| value * 6.999_04_f32)
}

/// IDA: N/A a node's world transform in Havok units, scale left out (Havok bodies don't scale; a scaled clone's shapes carry
/// `BhkHostScale`).
fn node_physics_transform(world: &World, node: Entity) -> PhysicsTransform {
    let transform = world
        .get::<NiAvWorld>(node)
        .map(|state| state.transform)
        .unwrap_or_default();
    PhysicsTransform {
        rotation: transform.rotation,
        translation: transform.translation.map(|value| value * HAVOK_SCALE),
    }
}

/// IDA: N/A the seam's description of a body: its shape, its place and its filter. Static bodies (motion system 7) and the
/// rest are all fixed for Target A (clutter dynamics come later); a keyframed one (motion 6) follows its node.
fn body_desc(world: &World, collision: Entity, body: Entity) -> Result<BodyDesc, GameError> {
    let target = world
        .get::<BhkSpCollisionObject>(collision)
        .and_then(|object| object.target_08);
    let node = target.map_or_else(PhysicsTransform::default, |target| {
        node_physics_transform(world, target)
    });
    let scale = world.get::<BhkHostScale>(body).map_or(1.0, |scale| scale.0);
    let (shape, transform, motion, phantom) = if let Some(rigid) = world.get::<BhkRigidBody>(body) {
        let shape = rigid.shape.ok_or(GameError::unclassified(
            0x008A48C0,
            "008A48C0: a rigid body with no shape",
        ))?;
        let cinfo = &rigid.cinfo_0c;
        let transform = if world.get::<NiClass>(body) == Some(&NiClass::BhkRigidBodyT) {
            let local = PhysicsTransform {
                rotation: quaternion_matrix([
                    float(cinfo, 0x40),
                    float(cinfo, 0x44),
                    float(cinfo, 0x48),
                    float(cinfo, 0x4C),
                ]),
                translation: vector(cinfo, 0x30).map(|value| value * scale),
            };
            node.compose(&local)
        } else {
            node
        };
        let motion = if cinfo[0xD0] == 6 {
            BodyMotion::Keyframed
        } else {
            BodyMotion::Fixed
        };
        (shape, transform, motion, false)
    } else if let Some(phantom) = world.get::<BhkSimpleShapePhantom>(body) {
        let shape = phantom.shape.ok_or(GameError::unclassified(
            0x008AF1A0,
            "008AF1A0: a phantom with no shape",
        ))?;
        (shape, node, BodyMotion::Fixed, true)
    } else {
        return Err(GameError::unclassified(
            0x0088A91E,
            "0088A91E: vt+0x5C on a body class the host lacks",
        ));
    };
    Ok(BodyDesc {
        shape: shape_desc(world, shape, scale)?,
        transform: world
            .get::<super::ragdoll::UnattachedBodyPose>(body)
            .map_or(transform, |pose| pose.0),
        motion,
        filter_info: body_filter(world, body),
        phantom,
    })
}

/// IDA: N/A a strip list's triangles in NiTriStripsData's order: each strip alternates its winding, and a degenerate
/// triangle (a repeated index) is left out, as hkMeshShape's strip subparts do.
fn strip_triangles(lengths: &[u16], points: &[u16], base: u32, out: &mut Vec<[u32; 3]>) {
    let mut at = 0;
    for &length in lengths {
        let strip = &points[at..at + usize::from(length)];
        for index in 2..strip.len() {
            let (a, b, c) = (strip[index - 2], strip[index - 1], strip[index]);
            if a == b || b == c || a == c {
                continue;
            }
            let triangle = if index % 2 == 0 { [a, b, c] } else { [a, c, b] };
            out.push(triangle.map(|vertex| base + u32::from(vertex)));
        }
        at += usize::from(length);
    }
}

/// IDA: N/A the seam's shape for a bhk shape, from the cinfo layouts the stream read (`havok_stream::parse_shape_payload`):
/// bhkSphereShape +0x04 the radius; bhkBoxShape +0x04 the radius, +0x10 the half extents; bhkCapsuleShape +0x04 the radius,
/// +0x10 and +0x20 the points; bhkConvexVerticesShape its vertices and 0.1 radius (`008C8A30`); bhkMoppBvTreeShape its
/// child (the MOPP code is Havok's own accelerator); bhkNiTriStripsShape its NiTriStripsData (game units) times
/// 0.1428767 and the cinfo's +0x20 scale; bhkTransformShape and bhkConvexTransformShape their child under the cinfo's
/// hkTransform (+0x10 three rotation columns, +0x40 the translation); bhkListShape its children. Any other class is a stop.
/// `scale` is the clone's `BhkHostScale`.
/// # Errors
/// A shape class not converted.
pub fn shape_desc(world: &World, shape: Entity, scale: f32) -> Result<ShapeDesc, GameError> {
    let class = world.get::<NiClass>(shape).copied();
    if let Some(convex) = world.get::<BhkConvexVerticesShape>(shape) {
        return Ok(ShapeDesc::ConvexHull {
            points: convex
                .vertices
                .iter()
                .map(|vertex| [0, 1, 2].map(|at| f32::from_bits(vertex[at]) * scale))
                .collect(),
            radius: f32::from_bits(convex.radius_bits) * scale,
        });
    }
    let data = world.get::<BhkShape>(shape).ok_or(GameError::unclassified(
        0,
        "a bhk shape class the seam doesn't convert",
    ))?;
    let cinfo = &data.cinfo_0c;
    let child = |index: usize| {
        data.shapes
            .get(index)
            .copied()
            .flatten()
            .ok_or(GameError::unclassified(0, "a wrapper shape with no child"))
    };
    Ok(match class {
        Some(NiClass::BhkSphereShape) => ShapeDesc::Sphere {
            radius: float(cinfo, 4) * scale,
        },
        Some(NiClass::BhkBoxShape) => ShapeDesc::Box {
            half_extents: vector(cinfo, 0x10).map(|value| value * scale),
            radius: float(cinfo, 4) * scale,
        },
        Some(NiClass::BhkCapsuleShape) => ShapeDesc::Capsule {
            point_a: vector(cinfo, 0x10).map(|value| value * scale),
            point_b: vector(cinfo, 0x20).map(|value| value * scale),
            radius: float(cinfo, 4) * scale,
        },
        Some(NiClass::BhkMoppBvTreeShape) => shape_desc(world, child(0)?, scale)?,
        Some(NiClass::BhkNiTriStripsShape) => {
            let factor = vector(cinfo, 0x20).map(|value| value * HAVOK_SCALE * scale);
            let mut vertices = Vec::new();
            let mut triangles = Vec::new();
            for data in data.shapes.iter().flatten() {
                let strips = world
                    .get::<crate::gamebryo::mesh_tri_strips::MeshTriStripsData>(*data)
                    .ok_or(GameError::unclassified(
                        0x008C6560,
                        "008C6560: a bhkNiTriStripsShape entry that isn't NiTriStripsData",
                    ))?;
                let base = vertices.len() as u32;
                vertices.extend(
                    strips
                        .base
                        .positions_1c
                        .iter()
                        .map(|point| std::array::from_fn(|at| point[at] * factor[at])),
                );
                strip_triangles(
                    &strips.strip_lengths_48,
                    &strips.strip_points_4c,
                    base,
                    &mut triangles,
                );
            }
            ShapeDesc::TriMesh {
                vertices,
                triangles,
                radius: float(cinfo, 4) * scale,
            }
        }
        Some(NiClass::BhkTransformShape | NiClass::BhkConvexTransformShape) => {
            let columns = [
                vector(cinfo, 0x10),
                vector(cinfo, 0x20),
                vector(cinfo, 0x30),
            ];
            let rotation = std::array::from_fn(|at| columns[at % 3][at / 3]);
            let transform = PhysicsTransform {
                rotation,
                translation: vector(cinfo, 0x40).map(|value| value * scale),
            };
            ShapeDesc::Compound(vec![(transform, shape_desc(world, child(0)?, scale)?)])
        }
        Some(NiClass::BhkListShape) => ShapeDesc::Compound(
            data.shapes
                .iter()
                .flatten()
                .map(|&child| {
                    Ok((
                        PhysicsTransform::default(),
                        shape_desc(world, child, scale)?,
                    ))
                })
                .collect::<Result<_, GameError>>()?,
        ),
        _ => {
            return Err(GameError::unclassified(
                0,
                "a bhk shape class the seam doesn't convert (host stop)",
            ));
        }
    })
}

/// IDA: `0088C740` bhkWorld vt+0x94 (root, 3D): both nonnull and the 3D's BSX (`006FA970`) with bit 2, else 0. The root's
/// collision object (`NiAVObject_GetBhkNiCollisionObject`): with a body and its vt+0x74 0 (bhkCollisionObject's
/// `BodyIsKeyframedOrAbsent`; a bhkSPCollisionObject answers 1), +0x18 = 1 and the answer 1; a root other than the 3D moves
/// the collision object onto the 3D (`move_collision_to_root`). Then the 3D's collision objects are visited with
/// `004DE010` and the context {world, recurse, 0, had-a-collision, 0, 0, 1}; 00BA7908 (BSS, no writer) skips
/// `0088BBB0`. Completion: 100% of the arms the prison reaches.
/// # Errors
/// The move arm, or a stop under the visit.
pub fn bhk_world_add_reference_3d(
    world: &mut World,
    bhk_world: Entity,
    root: Option<Entity>,
    node: Option<Entity>,
) -> Result<bool, GameError> {
    let (Some(root), Some(node)) = (root, node) else {
        return Ok(false);
    };
    if !crate::tes::reference_3d::bsx_flag(world, node, 2) {
        return Ok(false);
    }
    let mut answer = false;
    let collision = collision_object(world, root);
    if let Some(collision) = collision
        && world
            .get::<BhkSpCollisionObject>(collision)
            .and_then(|object| object.body_10)
            .is_some()
        && !collision_keyframed_or_absent(world, collision)
    {
        world
            .get_mut::<BhkWorld>(bhk_world)
            .expect("a bhkWorld")
            .byte_18 = 1;
        answer = true;
        if node != root {
            move_collision_to_root(world, collision, root, node)?;
        }
    }
    let mut context = CollisionVisit {
        world_00: Some(bhk_world),
        recurse_04: true,
        field_08: 0,
        first_0c: u32::from(collision.is_some()),
        group_10: 0,
        count_14: 0,
        mark_18: 1,
    };
    visit_collision_objects(world, Some(node), &mut context, &mut add_collision_callback)?;
    Ok(answer)
}

/// IDA: a collision object's vt+0x74: bhkSPCollisionObject's `ReturnTrue_SharedStub`; bhkCollisionObject's `0089E9C0`
/// bhkCollisionObject_BodyIsKeyframedOrAbsent: no body false; a body with no hk object true; else its motion type 6 or 7
/// (the host reads the cinfo's +0xD0 motion system, which builds it: D2). Completion: 100% of the classes the host links.
fn collision_keyframed_or_absent(world: &World, collision: Entity) -> bool {
    if world.get::<NiClass>(collision) == Some(&NiClass::BhkSpCollisionObject) {
        return true;
    }
    let Some(body) = world
        .get::<BhkSpCollisionObject>(collision)
        .and_then(|object| object.body_10)
    else {
        return false;
    };
    world
        .get::<BhkRigidBody>(body)
        .is_none_or(|body| matches!(body.cinfo_0c[0xD0], 6 | 7))
}

/// IDA: `0088CC80` bhkWorld vt+0x90 (3D, recurse, keyframed, group, force): a null 3D answers 0; without `force` the BSX bit
/// 2; +0x18 = 1; 00BA7908 (no writer) skips `0088ABB0`'s visit; then `004DE010`'s visit with {world, recurse, 0,
/// keyframed && 3D +0xA8, group, 0, 1}. Answers 1. Completion: 100%.
/// # Errors
/// A stop under the visit.
pub fn bhk_world_add_objects(
    world: &mut World,
    bhk_world: Entity,
    node: Option<Entity>,
    recurse: bool,
    keyframed: bool,
    group: u32,
    force: bool,
) -> Result<bool, GameError> {
    let Some(node) = node else {
        return Ok(false);
    };
    if !force && !crate::tes::reference_3d::bsx_flag(world, node, 2) {
        return Ok(false);
    }
    world
        .get_mut::<BhkWorld>(bhk_world)
        .expect("a bhkWorld")
        .byte_18 = 1;
    let mut context = CollisionVisit {
        world_00: Some(bhk_world),
        recurse_04: recurse,
        field_08: 0,
        first_0c: u32::from(keyframed && collision_object(world, node).is_some()),
        group_10: group,
        count_14: 0,
        mark_18: 1,
    };
    visit_collision_objects(world, Some(node), &mut context, &mut add_collision_callback)?;
    Ok(true)
}

/// IDA: `0088D070`(3D, motion, recurse, force): a null 3D, or no force and no BSX bit 2, answers 0; otherwise the visit
/// with {0, recurse, 5, motion} and `00889D20` (each collision object's vt+0x70(motion, 0)), answering 1. Host deferral:
/// each vt+0x70 is recorded in `MotionTypeDeferrals`, not run (the hk motion state and `0088F880`'s blend stores).
/// Completion: 100% of the walk.
/// # Errors
/// None today.
pub fn set_motion_type(
    world: &mut World,
    node: Option<Entity>,
    motion: u32,
    recurse: bool,
    force: bool,
) -> Result<bool, GameError> {
    let Some(node) = node else {
        return Ok(false);
    };
    if !force && !crate::tes::reference_3d::bsx_flag(world, node, 2) {
        return Ok(false);
    }
    let mut context = CollisionVisit {
        world_00: None,
        recurse_04: recurse,
        field_08: 5,
        first_0c: motion,
        ..CollisionVisit::default()
    };
    visit_collision_objects(
        world,
        Some(node),
        &mut context,
        &mut |world, collision, context| {
            world
                .get_resource_or_init::<MotionTypeDeferrals>()
                .calls
                .push((collision, context.first_0c));
            Ok(())
        },
    )?;
    Ok(true)
}

/// IDA: `0088CDC0`(3D, recurse, force): a null 3D, or no force and no BSX bit 2, answers 0; otherwise the visit with {0,
/// recurse, 2} and `00889CF0` (each collision object's +0x0C |= 0x40), answering 1. Completion: 100%.
/// # Errors
/// None today.
pub fn mark_collision_for_sync(
    world: &mut World,
    node: Option<Entity>,
    recurse: bool,
    force: bool,
) -> Result<bool, GameError> {
    let Some(node) = node else {
        return Ok(false);
    };
    if !force && !crate::tes::reference_3d::bsx_flag(world, node, 2) {
        return Ok(false);
    }
    let mut context = CollisionVisit {
        world_00: None,
        recurse_04: recurse,
        field_08: 2,
        ..CollisionVisit::default()
    };
    visit_collision_objects(
        world,
        Some(node),
        &mut context,
        &mut |world, collision, _| {
            if let Some(mut object) = world.get_mut::<BhkSpCollisionObject>(collision) {
                object.flags_0c |= 0x40;
            }
            Ok(())
        },
    )?;
    Ok(true)
}

/// IDA: N/A a row-major rotation's unit quaternion (x, y, z, w), as `005398E0` builds it from an NiMatrix3.
#[must_use]
pub fn matrix_quaternion(m: [f32; 9]) -> [f32; 4] {
    let trace = m[0] + m[4] + m[8];
    if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [
            (m[7] - m[5]) / s,
            (m[2] - m[6]) / s,
            (m[3] - m[1]) / s,
            0.25 * s,
        ]
    } else if m[0] > m[4] && m[0] > m[8] {
        let s = (1.0 + m[0] - m[4] - m[8]).sqrt() * 2.0;
        [
            0.25 * s,
            (m[1] + m[3]) / s,
            (m[2] + m[6]) / s,
            (m[7] - m[5]) / s,
        ]
    } else if m[4] > m[8] {
        let s = (1.0 + m[4] - m[0] - m[8]).sqrt() * 2.0;
        [
            (m[1] + m[3]) / s,
            0.25 * s,
            (m[5] + m[7]) / s,
            (m[2] - m[6]) / s,
        ]
    } else {
        let s = (1.0 + m[8] - m[0] - m[4]).sqrt() * 2.0;
        [
            (m[2] + m[6]) / s,
            (m[5] + m[7]) / s,
            0.25 * s,
            (m[3] - m[1]) / s,
        ]
    }
}

// IDA: N/A a NiAVObject's local transform (+0x30 rotation, +0x54 translation, +0x60 scale), node or geometry.
fn local_transform_mut(
    world: &mut World,
    object: Entity,
) -> Option<Mut<'_, crate::gamebryo::transform::NiTransform>> {
    if world.get::<SceneNodePayload>(object).is_some() {
        return world
            .get_mut::<SceneNodePayload>(object)
            .map(|payload| payload.map_unchanged(|payload| &mut payload.local_transform));
    }
    world
        .get_mut::<crate::gamebryo::mesh_geometry::MeshTriShapePayload>(object)
        .map(|payload| payload.map_unchanged(|payload| &mut payload.local_transform))
}

// IDA: N/A a node's collision index, which travels with the collision object it names.
fn collision_index(world: &World, object: Entity) -> u32 {
    world
        .get::<SceneNodePayload>(object)
        .map(|payload| payload.collision_index)
        .or_else(|| {
            world
                .get::<crate::gamebryo::mesh_geometry::MeshTriShapePayload>(object)
                .map(|payload| payload.collision_index)
        })
        .unwrap_or(crate::gamebryo::stream_objects::NULL_REFERENCE)
}

/// IDA: `0088C81D..0088CBEA`, `0088C740`'s arm for a collision object on a child of the 3D. A bhkRigidBodyT body is kept; a
/// bhkRigidBody becomes one: `bhkNiCollisionObject_SetBody(null)`, a new bhkRigidBodyT (`008B96D0`, its +0x30 the body's
/// vt+0x8C translation and +0x20 its vt+0x90 rotation), its hk transform zeroed (vt+0x94 00BA7A40, vt+0x98 00B2F080), then
/// `SetBody(T)`. A rigid body's local transform takes the child's (`005398E0`'s quaternion of +0x30, its +0x54 translation in
/// Havok units) in front: T = child * T. Then the 3D takes the collision object (`00435CE0`), the child loses it and its
/// flags become `& 0xFFE9 | 6`. Any other body (a phantom): the same move, and the child's local transform becomes the
/// identity. The host keeps a bhkRigidBodyT's local transform in its cinfo +0x30 / +0x40, where a loaded one has it.
/// Completion: 100% of the Bethesda side; the quaternion arithmetic is host f32 (the hk side is the seam's).
fn move_collision_to_root(
    world: &mut World,
    collision: Entity,
    child: Entity,
    root: Entity,
) -> Result<(), GameError> {
    let body = world
        .get::<BhkSpCollisionObject>(collision)
        .and_then(|object| object.body_10)
        .expect("checked by the caller");
    let index = collision_index(world, child);
    let local = local_transform_mut(world, child)
        .map(|transform| *transform)
        .unwrap_or_default();
    let rigid = world.get::<BhkRigidBody>(body).cloned();
    if let Some(mut rigid) = rigid {
        let body = if world.get::<NiClass>(body) == Some(&NiClass::BhkRigidBodyT) {
            body
        } else {
            // 0088C870 SetBody(null); 0088C89A 008B96D0; the old body's NiPointer released.
            let converted = rigid.clone();
            let scale = world.get::<BhkHostScale>(body).copied();
            let mut entity = world.spawn((converted, NiClass::BhkRigidBodyT));
            if let Some(scale) = scale {
                entity.insert(scale);
            }
            let new = entity.id();
            world
                .get_mut::<BhkSpCollisionObject>(collision)
                .expect("checked")
                .body_10 = Some(new);
            rigid = world.get::<BhkRigidBody>(new).expect("spawned").clone();
            new
        };
        let cinfo = &rigid.cinfo_0c;
        let old = PhysicsTransform {
            rotation: quaternion_matrix([
                float(cinfo, 0x40),
                float(cinfo, 0x44),
                float(cinfo, 0x48),
                float(cinfo, 0x4C),
            ]),
            translation: vector(cinfo, 0x30),
        };
        let front = PhysicsTransform {
            rotation: local.rotation,
            translation: local.translation.map(|value| value * HAVOK_SCALE),
        };
        let composed = front.compose(&old);
        let quaternion = matrix_quaternion(composed.rotation);
        let mut record = world.get_mut::<BhkRigidBody>(body).expect("a rigid body");
        for (at, value) in composed.translation.iter().enumerate() {
            record.cinfo_0c[0x30 + 4 * at..0x34 + 4 * at].copy_from_slice(&value.to_le_bytes());
        }
        for (at, value) in quaternion.iter().enumerate() {
            record.cinfo_0c[0x40 + 4 * at..0x44 + 4 * at].copy_from_slice(&value.to_le_bytes());
        }
    } else if let Some(mut transform) = local_transform_mut(world, child) {
        *transform = crate::gamebryo::transform::NiTransform {
            scale: transform.scale,
            ..crate::gamebryo::transform::NiTransform::default()
        };
    }
    super::fade_node::set_collision_object(world, root, Some(collision), index)?;
    super::fade_node::set_collision_object(
        world,
        child,
        None,
        crate::gamebryo::stream_objects::NULL_REFERENCE,
    )?;
    if let Some(mut flags) = av_flags_mut(world, child) {
        *flags = *flags & 0xFFE9 | 6;
    }
    Ok(())
}

/// IDA: `0088CD50` NiAVObject_RemoveCollisionFromWorld(object, recurse, force): a null object answers 0; without `force`, the
/// object's BSX bit 2 (`006FA970`) must be set, else 0; then the visit with {world 0, `recurse`, 1} (no flag rewrite) and
/// `00B2E304`'s callback `0088A970` (each body's vt+0x60: it leaves its world, `seam_remove_body`), answering `force`.
/// Completion: 100%.
/// # Errors
/// None today.
pub fn remove_collision_from_world(
    world: &mut World,
    object: Option<Entity>,
    recurse: bool,
    force: bool,
) -> Result<bool, GameError> {
    let Some(object) = object else {
        return Ok(false);
    };
    if !force && !crate::tes::reference_3d::bsx_flag(world, object, 2) {
        return Ok(false);
    }
    let mut context = CollisionVisit {
        world_00: None,
        recurse_04: recurse,
        field_08: 1,
        first_0c: 0,
        group_10: 0,
        count_14: 0,
        mark_18: 0,
    };
    visit_collision_objects(
        world,
        Some(object),
        &mut context,
        &mut |world, collision, _| {
            let body = world
                .get::<BhkSpCollisionObject>(collision)
                .and_then(|object| object.body_10);
            if let Some(body) = body
                && let Some(held) = world.get::<BhkInWorld>(body).copied()
            {
                let physics = world
                    .get::<BhkWorld>(held.world)
                    .and_then(|data| data.physics);
                super::physics_seam::seam_remove_body(world, physics, held.physics);
                world.entity_mut(body).remove::<BhkInWorld>();
            }
            Ok(())
        },
    )?;
    Ok(force)
}

/// IDA: `0088C440` bhkWorld vt+0x80, the world's update: the pick counters' warning (00BA790A clear: more than 00B2E2F4
/// picks warns; the port counts no picks) and their reset; +0x2C pending operations (`00889F20`, `0088A120`: the port queues
/// none); with +0x19, 00BA7914 substeps of 00BA790C (`00898B70`, the hkpWorld's step: the seam's), then `0088A660`, the
/// visual debugger's step and `0088A790` (counted); +0x4C's `0088A280` (none). Answers whether it stepped. Completion: 100%
/// of the Bethesda side.
/// # Errors
/// None today.
pub fn bhk_world_update(world: &mut World, bhk: Entity) -> Result<bool, GameError> {
    let frame = *world.get_resource_or_init::<HavokFrameGlobals>();
    let (enabled, physics) = {
        let record = world.get::<BhkWorld>(bhk).expect("a bhkWorld");
        (record.byte_19 != 0, record.physics)
    };
    if enabled && frame.substeps_ba7914 != 0 {
        crate::bs::physics_seam::seam_step(
            world,
            physics,
            frame.substeps_ba7914,
            frame.substep_time_ba790c,
        );
        return Ok(true);
    }
    Ok(false)
}
