//! The physics seam (Build 3, decided 2026-11-07): Havok is a recorded host substitution. The game keeps Havok's callers and
//! their Bethesda-side stores and calls this seam where the original makes an SDK call: the world's creation (`004D4A20`),
//! a body entering it (`0088A870`'s vt+0x5C), the step (`0088C440`), the character proxy's creation, world change and
//! position (`00895060`, `00891560`, `00891440`), and the controller's move (vt+0x80 under `0065AF30`). Everything crossing
//! it is plain data in Havok's units (game units times 0.1428767, z up), so this crate stays free of the host's physics
//! engine; the host (`oblivion_physics`, Avian) implements [`PhysicsBackend`] and installs it as the non-send
//! [`PhysicsSeam`].
//!
//! Without a backend the Bethesda-side stores still run and the seam records no hk object, as the D2 substitution did before
//! Build 3; only a call that needs an answer (the controller's move) is a host stop. No rand draw comes from Havok.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

/// IDA: N/A a world the backend made, by the order of its creation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PhysicsWorldId(pub u32);

/// IDA: N/A a body the backend made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PhysicsBodyId(pub u32);

/// IDA: N/A a character proxy the backend made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PhysicsCharacterId(pub u32);

/// IDA: N/A the values `004D4A20` puts in the bhkWorldCinfo (`0088A4F0`, then its stores) for the hkpWorld.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldDesc {
    /// `0088A4F0` / `004D4AA3` gravity (0, 0, -73.575, 0), Havok units per second squared.
    pub gravity: [f32; 3],
    /// `008A9460(3000.0)`: the broadphase from -1500 to 1500 on each axis.
    pub broadphase_size: f32,
    /// `0088D260`'s centre, Havok units (`004CB7F0`'s centroid times the scale).
    pub broadphase_center: [f32; 3],
    /// `00B2E2E8`, the fixed substep the step uses (`min(fMaxTime, 1.0)`).
    pub max_substep: f32,
}

/// IDA: N/A a rigid transform in Havok units: a row-major rotation and a translation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicsTransform {
    pub rotation: [f32; 9],
    pub translation: [f32; 3],
}

impl Default for PhysicsTransform {
    /// IDA: N/A the identity.
    fn default() -> Self {
        Self {
            rotation: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            translation: [0.0; 3],
        }
    }
}

impl PhysicsTransform {
    /// IDA: N/A `self * other` (other's frame inside self's).
    #[must_use]
    pub fn compose(&self, other: &Self) -> Self {
        let a = &self.rotation;
        let b = &other.rotation;
        let mut rotation = [0.0; 9];
        for row in 0..3 {
            for column in 0..3 {
                rotation[row * 3 + column] = a[row * 3] * b[column]
                    + a[row * 3 + 1] * b[3 + column]
                    + a[row * 3 + 2] * b[6 + column];
            }
        }
        Self {
            rotation,
            translation: self.apply(other.translation),
        }
    }

    /// IDA: N/A a point through the transform.
    #[must_use]
    pub fn apply(&self, point: [f32; 3]) -> [f32; 3] {
        let r = &self.rotation;
        std::array::from_fn(|row| {
            r[row * 3] * point[0]
                + r[row * 3 + 1] * point[1]
                + r[row * 3 + 2] * point[2]
                + self.translation[row]
        })
    }
}

/// IDA: N/A the SDK body's position in Havok units and rotation in Havok's xyzw order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyPose {
    pub position: [f32; 3],
    pub rotation_xyzw: [f32; 4],
}

/// IDA: N/A a collision shape as the bhk shapes describe it, in Havok units. `radius` is the convex radius Havok keeps around
/// a box or hull.
#[derive(Clone, Debug, PartialEq)]
pub enum ShapeDesc {
    /// bhkBoxShape: the half extents.
    Box { half_extents: [f32; 3], radius: f32 },
    /// bhkSphereShape.
    Sphere { radius: f32 },
    /// bhkCapsuleShape: the two segment ends.
    Capsule {
        point_a: [f32; 3],
        point_b: [f32; 3],
        radius: f32,
    },
    /// bhkConvexVerticesShape: the hull's points.
    ConvexHull { points: Vec<[f32; 3]>, radius: f32 },
    /// bhkNiTriStripsShape under a bhkMoppBvTreeShape (the MOPP code itself is Havok's accelerator, not taken): triangles.
    TriMesh {
        vertices: Vec<[f32; 3]>,
        triangles: Vec<[u32; 3]>,
        /// The mesh's convex radius (the shape cinfo's +0x04).
        radius: f32,
    },
    /// bhkListShape, bhkTransformShape and bhkConvexTransformShape.
    Compound(Vec<(PhysicsTransform, ShapeDesc)>),
}

/// IDA: N/A how a body moves: the cinfo's motion system (+0xD0). Target A adds every body fixed or keyframed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyMotion {
    /// Motion system 7 (MO_SYS_FIXED), and the host's stand-in for the others until clutter dynamics come (after Target A).
    Fixed,
    /// A body that follows its node (the actors' ragdoll bones, motion 6).
    Keyframed,
}

/// IDA: N/A one body entering a world: `0088A870`'s vt+0x5C(world) on a bhkRigidBody, or a phantom's.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyDesc {
    pub shape: ShapeDesc,
    pub transform: PhysicsTransform,
    pub motion: BodyMotion,
    /// The hk object's collision filter info: layer in bits 0..5, the system group in bits 16..31.
    pub filter_info: u32,
    /// A phantom takes no part in the controller's collisions.
    pub phantom: bool,
}

/// IDA: N/A the character proxy: Phase 11's +0x374 capsule (`008905E0`), relative to the proxy's position.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterDesc {
    pub shape: ShapeDesc,
    pub position: [f32; 3],
    /// The phantom's filter info (`0057E270`): bodies of the same system group don't collide with it.
    pub filter_info: u32,
    /// The hkCharacterProxy's keep distance (the cinfo's +0x4C, 0.5 from `00890C00`): the gap it holds from surfaces.
    pub keep_distance: f32,
}

/// IDA: N/A the controller's vt+0x80 step (`0065B44B` under `0065AF30`): the wanted velocity over `time`, Havok units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterMove {
    pub time: f32,
    pub velocity: [f32; 3],
    /// Gravity applies (the controller's in-air state); a flying or keyframed controller passes false.
    pub gravity: bool,
}

/// IDA: N/A what the move answers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterMoveResult {
    pub position: [f32; 3],
    /// The capsule stands on a surface after the move.
    pub supported: bool,
    /// The vertical velocity the backend carries into the next move.
    pub vertical_velocity: f32,
}

/// IDA: N/A the host's physics behind the seam (a recorded substitution for the Havok SDK).
pub trait PhysicsBackend {
    /// `004D4A20`'s `0088AEB0` with its cinfo, `0088D260`'s centre included.
    fn create_world(&mut self, desc: &WorldDesc) -> PhysicsWorldId;
    /// `0088D260`'s re-centre of the broadphase.
    fn set_broadphase_center(&mut self, world: PhysicsWorldId, center: [f32; 3]);
    /// A body's vt+0x5C(world).
    fn add_body(&mut self, world: PhysicsWorldId, body: &BodyDesc) -> PhysicsBodyId;
    /// `008A2ED0` / `008A2F10`: motion-state position and rotation of a live body.
    fn body_pose(&self, world: PhysicsWorldId, body: PhysicsBodyId) -> Option<BodyPose>;
    /// A body's vt+0x60: the body leaves `world` (`0088A970`, NiAVObject_RemoveCollisionFromWorld's callback).
    fn remove_body(&mut self, world: PhysicsWorldId, body: PhysicsBodyId);
    /// `0088C440`'s substeps: `substeps` steps of `substep_time` seconds.
    fn step(&mut self, world: PhysicsWorldId, substeps: u32, substep_time: f32);
    /// The hkCharacterProxy over its phantom, outside any world.
    fn create_character(&mut self, desc: &CharacterDesc) -> PhysicsCharacterId;
    /// `00895060`: the proxy's phantom leaves its old world and enters `world` (None: none).
    fn set_character_world(&mut self, character: PhysicsCharacterId, world: Option<PhysicsWorldId>);
    /// `00891560`'s hkCharacterProxy position store.
    fn set_character_position(&mut self, character: PhysicsCharacterId, position: [f32; 3]);
    /// `00891440`'s read.
    fn character_position(&self, character: PhysicsCharacterId) -> [f32; 3];
    /// A ray cast (the picks `0088B850` bhkWorld_PickObject makes): the hits' distances along the ray and their bodies,
    /// nearest first, up to `max_hits`, phantoms and bodies of `group` (nonzero) left out.
    fn cast_ray(
        &mut self,
        world: PhysicsWorldId,
        origin: [f32; 3],
        direction: [f32; 3],
        max_distance: f32,
        group: u32,
        max_hits: u32,
    ) -> Vec<(f32, PhysicsBodyId)>;
    /// The controller's vt+0x80 integrate: the proxy moved and slid through its world.
    fn move_character(
        &mut self,
        character: PhysicsCharacterId,
        movement: &CharacterMove,
    ) -> CharacterMoveResult;
}

/// IDA: N/A the installed backend. A host or test inserts it as a non-send resource before the game reaches a world.
pub struct PhysicsSeam(pub Box<dyn PhysicsBackend>);

/// IDA: N/A the seam's calls in order, kept with or without a backend, for the tests and the open items.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct PhysicsSeamLog {
    pub worlds: Vec<WorldDesc>,
    pub bodies: u32,
    /// The game's body entity of each add, in order (the seam's body ids with a backend).
    pub body_entities: Vec<Entity>,
    pub steps: Vec<(u32, f32)>,
    pub characters: u32,
    pub moves: u32,
    /// The bodies that left their world (vt+0x60).
    pub removed_bodies: u32,
}

/// IDA: N/A the seam's create_world: `None` without a backend.
pub fn seam_create_world(world: &mut World, desc: &WorldDesc) -> Option<PhysicsWorldId> {
    world
        .get_resource_or_init::<PhysicsSeamLog>()
        .worlds
        .push(*desc);
    world
        .get_non_send_mut::<PhysicsSeam>()
        .map(|mut seam| seam.0.create_world(desc))
}

/// IDA: N/A the seam's add_body: `None` without a backend or a world.
pub fn seam_add_body(
    world: &mut World,
    physics: Option<PhysicsWorldId>,
    body: &BodyDesc,
) -> Option<PhysicsBodyId> {
    world.get_resource_or_init::<PhysicsSeamLog>().bodies += 1;
    let physics = physics?;
    world
        .get_non_send_mut::<PhysicsSeam>()
        .map(|mut seam| seam.0.add_body(physics, body))
}

/// IDA: N/A the seam's remove_body (a body's vt+0x60): nothing without a backend or a world.
pub fn seam_remove_body(
    world: &mut World,
    physics: Option<PhysicsWorldId>,
    body: Option<PhysicsBodyId>,
) {
    world
        .get_resource_or_init::<PhysicsSeamLog>()
        .removed_bodies += 1;
    if let (Some(physics), Some(body)) = (physics, body)
        && let Some(mut seam) = world.get_non_send_mut::<PhysicsSeam>()
    {
        seam.0.remove_body(physics, body);
    }
}

/// IDA: N/A the seam's step.
pub fn seam_step(
    world: &mut World,
    physics: Option<PhysicsWorldId>,
    substeps: u32,
    substep_time: f32,
) {
    world
        .get_resource_or_init::<PhysicsSeamLog>()
        .steps
        .push((substeps, substep_time));
    if let Some(physics) = physics
        && let Some(mut seam) = world.get_non_send_mut::<PhysicsSeam>()
    {
        seam.0.step(physics, substeps, substep_time);
    }
}

/// IDA: N/A the seam's character creation.
pub fn seam_create_character(
    world: &mut World,
    desc: &CharacterDesc,
) -> Option<PhysicsCharacterId> {
    world.get_resource_or_init::<PhysicsSeamLog>().characters += 1;
    world
        .get_non_send_mut::<PhysicsSeam>()
        .map(|mut seam| seam.0.create_character(desc))
}

/// IDA: N/A the seam's world change for a character.
pub fn seam_set_character_world(
    world: &mut World,
    character: Option<PhysicsCharacterId>,
    physics: Option<PhysicsWorldId>,
) {
    if let Some(character) = character
        && let Some(mut seam) = world.get_non_send_mut::<PhysicsSeam>()
    {
        seam.0.set_character_world(character, physics);
    }
}

/// IDA: N/A the seam's character position store.
pub fn seam_set_character_position(
    world: &mut World,
    character: Option<PhysicsCharacterId>,
    position: [f32; 3],
) {
    if let Some(character) = character
        && let Some(mut seam) = world.get_non_send_mut::<PhysicsSeam>()
    {
        seam.0.set_character_position(character, position);
    }
}

/// IDA: N/A the seam's character position read: a stop without a backend.
/// # Errors
/// No backend or no proxy.
pub fn seam_character_position(
    world: &World,
    character: Option<PhysicsCharacterId>,
) -> Result<[f32; 3], GameError> {
    let character = character.ok_or(GameError::unclassified(
        0x00891440,
        "00891440: the controller's proxy with no physics backend (host)",
    ))?;
    world
        .get_non_send::<PhysicsSeam>()
        .map(|seam| seam.0.character_position(character))
        .ok_or(GameError::unclassified(
            0x00891440,
            "00891440: the controller's proxy with no physics backend (host)",
        ))
}

/// IDA: N/A the seam's move: a stop without a backend.
/// # Errors
/// No backend or no proxy.
pub fn seam_move_character(
    world: &mut World,
    character: Option<PhysicsCharacterId>,
    movement: &CharacterMove,
) -> Result<CharacterMoveResult, GameError> {
    world.get_resource_or_init::<PhysicsSeamLog>().moves += 1;
    let character = character.ok_or(GameError::unclassified(
        0x0065B44B,
        "0065B44B: the controller's vt+0x80 step with no physics backend (host)",
    ))?;
    world
        .get_non_send_mut::<PhysicsSeam>()
        .map(|mut seam| seam.0.move_character(character, movement))
        .ok_or(GameError::unclassified(
            0x0065B44B,
            "0065B44B: the controller's vt+0x80 step with no physics backend (host)",
        ))
}

/// IDA: N/A the seam's ray cast: empty without a backend or a world.
pub fn seam_cast_ray(
    world: &mut World,
    physics: Option<PhysicsWorldId>,
    origin: [f32; 3],
    direction: [f32; 3],
    max_distance: f32,
    group: u32,
    max_hits: u32,
) -> Vec<(f32, PhysicsBodyId)> {
    let Some(physics) = physics else {
        return Vec::new();
    };
    world
        .get_non_send_mut::<PhysicsSeam>()
        .map(|mut seam| {
            seam.0
                .cast_ray(physics, origin, direction, max_distance, group, max_hits)
        })
        .unwrap_or_default()
}
