//! IDA: N/A the Avian backend against the seam's contract: a world, a fixed floor, a capsule character falling onto it.
use oblivion_physics::AvianPhysics;
use oblivion_sim::bs::physics_seam::{
    BodyDesc, BodyMotion, CharacterDesc, CharacterMove, PhysicsBackend, PhysicsTransform,
    ShapeDesc, WorldDesc,
};

#[test]
fn a_capsule_falls_onto_a_floor_and_stays() {
    let mut physics = AvianPhysics::new();
    let world = physics.create_world(&WorldDesc {
        gravity: [0.0, 0.0, -73.575],
        broadphase_size: 3000.0,
        broadphase_center: [0.0; 3],
        max_substep: 1.0 / 60.0,
    });
    // A floor whose top is at z = 0.
    physics.add_body(
        world,
        &BodyDesc {
            shape: ShapeDesc::Box {
                half_extents: [50.0, 50.0, 1.0],
                radius: 0.0,
            },
            transform: PhysicsTransform {
                translation: [0.0, 0.0, -1.0],
                ..PhysicsTransform::default()
            },
            motion: BodyMotion::Fixed,
            filter_info: 1,
            phantom: false,
        },
    );
    let character = physics.create_character(&CharacterDesc {
        shape: ShapeDesc::Capsule {
            point_a: [0.0, 0.0, 6.25],
            point_b: [0.0, 0.0, -6.25],
            radius: 2.893,
        },
        position: [0.0, 0.0, 9.143 + 0.92],
        filter_info: 9 | (11 << 16),
        keep_distance: 0.0,
    });
    physics.set_character_world(character, Some(world));
    let movement = CharacterMove {
        time: 0.16,
        velocity: [0.0; 3],
        gravity: true,
    };
    let first = physics.move_character(character, &movement);
    assert!(first.supported, "lands within one step: {first:?}");
    let second = physics.move_character(character, &movement);
    assert!((first.position[2] - second.position[2]).abs() < 0.1 * 0.1428767);
    assert!(
        (second.position[2] - 9.143).abs() < 0.05,
        "rests on the floor: {second:?}"
    );
}

#[test]
fn compound_pose_and_ray_hits_share_the_body_until_removal() {
    let mut physics = AvianPhysics::new();
    let world = physics.create_world(&WorldDesc {
        gravity: [0.0; 3],
        broadphase_size: 3000.0,
        broadphase_center: [0.0; 3],
        max_substep: 1.0 / 60.0,
    });
    let body = physics.add_body(
        world,
        &BodyDesc {
            shape: ShapeDesc::Compound(vec![(
                PhysicsTransform {
                    translation: [5.0, 0.0, 0.0],
                    ..Default::default()
                },
                ShapeDesc::Sphere { radius: 1.0 },
            )]),
            transform: PhysicsTransform {
                rotation: [0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
                translation: [10.0, 20.0, 30.0],
            },
            motion: BodyMotion::Keyframed,
            filter_info: 7 << 16,
            phantom: false,
        },
    );
    let origin = [10.0, 25.0, 35.0];
    let down = [0.0, 0.0, -1.0];
    let hits = physics.cast_ray(world, origin, down, 10.0, 0, 4);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].1, body);
    assert!((hits[0].0 - 4.0).abs() < 1e-4);
    assert!(physics.cast_ray(world, origin, down, 10.0, 7, 4).is_empty());
    let pose = physics.body_pose(world, body).unwrap();
    assert_eq!(pose.position, [10.0, 20.0, 30.0]);
    assert!((pose.rotation_xyzw[2] * pose.rotation_xyzw[3] - 0.5).abs() < 1e-6);
    physics.remove_body(world, body);
    assert!(physics.body_pose(world, body).is_none());
    assert!(physics.cast_ray(world, origin, down, 10.0, 0, 4).is_empty());
}
