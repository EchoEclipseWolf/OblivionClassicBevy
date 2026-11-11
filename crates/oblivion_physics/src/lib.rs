//! IDA: N/A the host's physics: Avian 0.8 behind the game's physics seam (`oblivion_sim::bs::physics_seam`), a recorded host
//! substitution for Havok (docs/target-a-plan-2026-11-07.md, "Havok: Avian behind a seam"). Each Havok world is its own
//! headless Bevy app with Avian's plugins, stepped synchronously when the game calls the seam, never on Bevy's fixed clock.
//! Units are Havok's (game units times 0.1428767), z up, and gravity is the world cinfo's. Bodies enter as Avian static or
//! kinematic colliders; the character proxy is a shape moved with Avian's `MoveAndSlide`, gravity integrated here.
//!
//! What differs from Havok, recorded in docs/open-items.md: no MOPP (a triangle mesh instead), no convex radius on hulls,
//! Havok's layer matrix (008A83C0's tables) isn't applied (a character collides with every non-phantom body outside its
//! system group), and no dynamic bodies move (clutter and ragdolls come after Target A). Positions are compared with the
//! captures within a tolerance, not bit for bit; no rand draw comes from here.

use std::time::Duration;

use avian3d::prelude::*;
use bevy::app::App;
use bevy::ecs::system::SystemState;
use bevy::prelude::{ChildOf, Entity, MinimalPlugins, Quat, Transform, Vec3};
use bevy::time::TimeUpdateStrategy;
use oblivion_sim::bs::physics_seam::{
    BodyDesc, BodyMotion, BodyPose, CharacterDesc, CharacterMove, CharacterMoveResult,
    PhysicsBackend, PhysicsBodyId, PhysicsCharacterId, PhysicsTransform, PhysicsWorldId, ShapeDesc,
    WorldDesc,
};

/// IDA: N/A a substep short enough to leave static bodies where they are, run so the collider trees take new bodies before
/// a query (Avian builds them in its broadphase).
const REFRESH_STEP: f64 = 1.0e-6;

/// IDA: N/A a surface normal at least this upright supports the character (cos 45 degrees; Havok's controller uses the
/// listener's slope, +0x210, which goal 2's state logic owns).
const SUPPORT_NORMAL_Z: f32 = std::f32::consts::FRAC_1_SQRT_2;

// IDA: N/A one Havok world.
struct AvianWorld {
    app: App,
    gravity: Vec3,
    /// The colliders in add order: their entity, filter and seam body.
    bodies: Vec<(Entity, u32, u32)>,
    /// One root per assigned body ID. Removed IDs stay vacant.
    roots: Vec<Option<Entity>>,
    /// The largest convex radius of the meshes added (Avian's meshes have none: the character's skin takes it).
    mesh_radius: f32,
    dirty: bool,
}

// IDA: N/A one character proxy.
struct AvianCharacter {
    shape: Collider,
    position: Vec3,
    world: Option<usize>,
    filter: u32,
    keep_distance: f32,
    vertical_velocity: f32,
}

/// IDA: N/A the Avian backend.
#[derive(Default)]
pub struct AvianPhysics {
    worlds: Vec<AvianWorld>,
    characters: Vec<AvianCharacter>,
}

impl AvianPhysics {
    /// IDA: N/A a new backend with no worlds.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// IDA: N/A how many seam bodies a world holds (for tests).
    #[must_use]
    pub fn body_count(&self, world: PhysicsWorldId) -> u32 {
        self.worlds[world.0 as usize].roots.len() as u32
    }
}

// IDA: N/A the seam's vector as Avian's.
fn vec(value: [f32; 3]) -> Vec3 {
    Vec3::from_array(value)
}

// IDA: N/A a row-major rotation as a quaternion.
fn quat(rotation: &[f32; 9]) -> Quat {
    let r = rotation;
    Quat::from_mat3(&bevy::math::Mat3::from_cols(
        Vec3::new(r[0], r[3], r[6]),
        Vec3::new(r[1], r[4], r[7]),
        Vec3::new(r[2], r[5], r[8]),
    ))
    .normalize()
}

/// IDA: N/A the Avian collider for a seam shape. A degenerate hull or mesh gives `None`.
#[must_use]
pub fn collider(shape: &ShapeDesc) -> Option<Collider> {
    Some(match shape {
        ShapeDesc::Box {
            half_extents,
            radius,
        } => Collider::round_cuboid(
            2.0 * half_extents[0],
            2.0 * half_extents[1],
            2.0 * half_extents[2],
            *radius,
        ),
        ShapeDesc::Sphere { radius } => Collider::sphere(*radius),
        ShapeDesc::Capsule {
            point_a,
            point_b,
            radius,
        } => Collider::capsule_endpoints(*radius, vec(*point_a), vec(*point_b)),
        ShapeDesc::ConvexHull { points, .. } => {
            let points: Vec<Vec3> = points.iter().map(|point| vec(*point)).collect();
            Collider::convex_hull(&points)?
        }
        ShapeDesc::TriMesh {
            vertices,
            triangles,
            ..
        } => {
            if triangles.is_empty() {
                return None;
            }
            Collider::trimesh(
                vertices.iter().map(|vertex| vec(*vertex)).collect(),
                triangles.clone(),
            )
        }
        ShapeDesc::Compound(_) => {
            let mut leaves = Vec::new();
            flatten(shape, PhysicsTransform::default(), &mut leaves);
            let parts: Vec<(Position, Rotation, Collider)> = leaves
                .iter()
                .filter(|(_, child)| !matches!(child, ShapeDesc::TriMesh { .. }))
                .filter_map(|(transform, child)| {
                    Some((
                        Position(vec(transform.translation)),
                        Rotation(quat(&transform.rotation)),
                        collider(child)?,
                    ))
                })
                .collect();
            if parts.is_empty() {
                return None;
            }
            Collider::compound(parts)
        }
    })
}

/// IDA: N/A a shape's leaves under `transform`, compounds opened and their transforms composed in.
pub fn flatten(
    shape: &ShapeDesc,
    transform: PhysicsTransform,
    out: &mut Vec<(PhysicsTransform, ShapeDesc)>,
) {
    match shape {
        ShapeDesc::Compound(children) => {
            for (local, child) in children {
                flatten(child, transform.compose(local), out);
            }
        }
        leaf => out.push((transform, leaf.clone())),
    }
}

// IDA: N/A a world's app: Avian on Bevy's minimal plugins, the physics schedule run in `Update` with a manual clock.
fn world_app(gravity: Vec3) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::transform::TransformPlugin,
        bevy::asset::AssetPlugin::default(),
        bevy::mesh::MeshPlugin,
        PhysicsPlugins::new(bevy::app::Update),
    ))
    .insert_resource(Gravity(gravity))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        REFRESH_STEP,
    )));
    app.finish();
    app.cleanup();
    app
}

impl AvianWorld {
    // IDA: N/A one update of `seconds`.
    fn run(&mut self, seconds: f64) {
        self.app
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                seconds,
            )));
        self.app.update();
        self.dirty = false;
    }

    // IDA: N/A the bodies of a nonzero system group, which a character or a ray of that group passes through.
    fn group_bodies(&self, group: u32) -> Vec<Entity> {
        self.bodies
            .iter()
            .filter(|(_, filter, _)| group != 0 && filter >> 16 == group)
            .map(|(entity, _, _)| *entity)
            .collect()
    }

    // IDA: N/A the collider trees made current before a query.
    fn refresh(&mut self) {
        if self.dirty {
            // The first update only starts the clock; the second steps.
            self.run(REFRESH_STEP);
            self.run(REFRESH_STEP);
        }
    }
}

impl PhysicsBackend for AvianPhysics {
    fn create_world(&mut self, desc: &WorldDesc) -> PhysicsWorldId {
        let gravity = vec(desc.gravity);
        self.worlds.push(AvianWorld {
            app: world_app(gravity),
            gravity,
            bodies: Vec::new(),
            roots: Vec::new(),
            mesh_radius: 0.0,
            dirty: true,
        });
        PhysicsWorldId(self.worlds.len() as u32 - 1)
    }

    fn set_broadphase_center(&mut self, _world: PhysicsWorldId, _center: [f32; 3]) {
        // Avian's broadphase has no bounds to centre.
    }

    fn add_body(&mut self, world: PhysicsWorldId, body: &BodyDesc) -> PhysicsBodyId {
        let target = &mut self.worlds[world.0 as usize];
        let rigid = match body.motion {
            BodyMotion::Fixed => RigidBody::Static,
            BodyMotion::Keyframed => RigidBody::Kinematic,
        };
        let root = target
            .app
            .world_mut()
            .spawn((
                rigid,
                Position(vec(body.transform.translation)),
                Rotation(quat(&body.transform.rotation)),
            ))
            .id();
        let body_id = PhysicsBodyId(target.roots.len() as u32);
        target.roots.push(Some(root));
        // Parry cannot nest a mesh in a compound. Avian attaches each leaf collider
        // to one rigid body so both pose reads and ray hits identify the same body.
        let compound = matches!(body.shape, ShapeDesc::Compound(_));
        let mut leaves = Vec::new();
        flatten(&body.shape, PhysicsTransform::default(), &mut leaves);
        for (local, shape) in leaves {
            if let ShapeDesc::TriMesh { radius, .. } = &shape {
                target.mesh_radius = target.mesh_radius.max(*radius);
            }
            let Some(shape) = collider(&shape) else {
                continue;
            };
            let mut entity = if compound {
                target.app.world_mut().spawn((
                    ChildOf(root),
                    Transform::from_translation(vec(local.translation))
                        .with_rotation(quat(&local.rotation)),
                ))
            } else {
                target.app.world_mut().entity_mut(root)
            };
            entity.insert(shape);
            if body.phantom {
                entity.insert(Sensor);
            }
            target
                .bodies
                .push((entity.id(), body.filter_info, body_id.0));
        }
        target.dirty = true;
        body_id
    }

    fn body_pose(&self, world: PhysicsWorldId, body: PhysicsBodyId) -> Option<BodyPose> {
        let target = self.worlds.get(world.0 as usize)?;
        let root = target.roots.get(body.0 as usize).copied().flatten()?;
        let world = target.app.world();
        Some(BodyPose {
            position: world.get::<Position>(root)?.0.to_array(),
            rotation_xyzw: world.get::<Rotation>(root)?.0.to_array(),
        })
    }

    fn remove_body(&mut self, world: PhysicsWorldId, body: PhysicsBodyId) {
        let target = &mut self.worlds[world.0 as usize];
        if let Some(root) = target.roots.get_mut(body.0 as usize).and_then(Option::take) {
            target.bodies.retain(|&(_, _, id)| id != body.0);
            target.app.world_mut().despawn(root);
            target.dirty = true;
        }
    }

    fn step(&mut self, world: PhysicsWorldId, substeps: u32, substep_time: f32) {
        let target = &mut self.worlds[world.0 as usize];
        for _ in 0..substeps {
            target.run(f64::from(substep_time));
        }
    }

    fn create_character(&mut self, desc: &CharacterDesc) -> PhysicsCharacterId {
        let shape = collider(&desc.shape).unwrap_or_else(|| Collider::sphere(0.01));
        self.characters.push(AvianCharacter {
            shape,
            position: vec(desc.position),
            world: None,
            filter: desc.filter_info,
            keep_distance: desc.keep_distance,
            vertical_velocity: 0.0,
        });
        PhysicsCharacterId(self.characters.len() as u32 - 1)
    }

    fn set_character_world(
        &mut self,
        character: PhysicsCharacterId,
        world: Option<PhysicsWorldId>,
    ) {
        let target = &mut self.characters[character.0 as usize];
        target.world = world.map(|world| world.0 as usize);
        target.vertical_velocity = 0.0;
    }

    fn set_character_position(&mut self, character: PhysicsCharacterId, position: [f32; 3]) {
        self.characters[character.0 as usize].position = vec(position);
    }

    fn character_position(&self, character: PhysicsCharacterId) -> [f32; 3] {
        self.characters[character.0 as usize].position.to_array()
    }

    fn cast_ray(
        &mut self,
        world: PhysicsWorldId,
        origin: [f32; 3],
        direction: [f32; 3],
        max_distance: f32,
        group: u32,
        max_hits: u32,
    ) -> Vec<(f32, PhysicsBodyId)> {
        let target = &mut self.worlds[world.0 as usize];
        target.refresh();
        let Ok(direction) = bevy::math::Dir3::new(vec(direction)) else {
            return Vec::new();
        };
        let excluded = target.group_bodies(group);
        let mut system: SystemState<SpatialQuery> = SystemState::new(target.app.world_mut());
        let query = system
            .get(target.app.world())
            .expect("Avian's spatial query resources");
        let filter = SpatialQueryFilter::default().with_excluded_entities(excluded);
        let mut hits: Vec<(f32, PhysicsBodyId)> = query
            .ray_hits(
                vec(origin),
                direction,
                max_distance,
                max_hits,
                true,
                &filter,
            )
            .iter()
            .filter_map(|hit| {
                target
                    .bodies
                    .iter()
                    .find(|body| body.0 == hit.entity)
                    .map(|body| (hit.distance, PhysicsBodyId(body.2)))
            })
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        hits
    }

    fn move_character(
        &mut self,
        character: PhysicsCharacterId,
        movement: &CharacterMove,
    ) -> CharacterMoveResult {
        let state = &self.characters[character.0 as usize];
        let Some(index) = state.world else {
            // Outside a world the proxy moves freely.
            let position = state.position + vec(movement.velocity) * movement.time;
            self.characters[character.0 as usize].position = position;
            return CharacterMoveResult {
                position: position.to_array(),
                supported: false,
                vertical_velocity: 0.0,
            };
        };
        let world = &mut self.worlds[index];
        world.refresh();
        let excluded = world.group_bodies(state.filter >> 16);
        let vertical = if movement.gravity {
            state.vertical_velocity + world.gravity.z * movement.time
        } else {
            0.0
        };
        let velocity = vec(movement.velocity) + Vec3::new(0.0, 0.0, vertical);
        let shape = state.shape.clone();
        let start = state.position;
        let mut system: SystemState<MoveAndSlide> = SystemState::new(world.app.world_mut());
        let slide = system
            .get(world.app.world())
            .expect("Avian's spatial query resources");
        let filter = SpatialQueryFilter::default().with_excluded_entities(excluded);
        let mut supported = false;
        let standing = movement.velocity[0] == 0.0 && movement.velocity[1] == 0.0;
        // Havok's proxy holds its keep distance from every surface; a mesh's convex radius widens it.
        let config = MoveAndSlideConfig {
            skin_width: state.keep_distance + world.mesh_radius,
            ..MoveAndSlideConfig::default()
        };
        let output = slide.move_and_slide(
            &shape,
            start,
            Quat::IDENTITY,
            velocity,
            Duration::from_secs_f32(movement.time.max(0.0)),
            &config,
            &filter,
            |hit| {
                if hit.normal.z >= SUPPORT_NORMAL_Z {
                    supported = true;
                    // Standing on a supporting surface: no slide down it (the controller's on-ground state).
                    if standing {
                        return MoveAndSlideHitResponse::Abort;
                    }
                }
                MoveAndSlideHitResponse::Accept
            },
        );
        let state = &mut self.characters[character.0 as usize];
        state.position = output.position;
        state.vertical_velocity = if supported && vertical <= 0.0 {
            0.0
        } else {
            output.projected_velocity.z
        };
        CharacterMoveResult {
            position: output.position.to_array(),
            supported,
            vertical_velocity: state.vertical_velocity,
        }
    }
}
