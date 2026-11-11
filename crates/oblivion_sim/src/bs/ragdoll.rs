//! The skeleton and rigid-body reads that drive an actor's ragdoll root.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::havok_stream::{BhkBlendCollisionObject, BhkSpCollisionObject};
use super::havok_world::{BhkInWorld, BhkWorld};
use super::physics_seam::PhysicsSeam;
use crate::gamebryo::av_object::NiAvCollision;
use crate::gamebryo::scene_node::{
    NiAvWorld, NiNodeChildren, SceneNodePayload, update_world_transform,
};
use crate::gamebryo::transform_controller::quaternion_to_rotation_matrix;

// IDA: 00497420 NiAVObject_GetBlendCollisionObject. Cast the +A8 object through its RTTI chain.
// Completion: 100% of the loaded collision classes.
pub(crate) fn blend_collision(world: &World, node: Entity) -> Option<Entity> {
    let object = world.get::<NiAvCollision>(node)?.0;
    world.get::<BhkBlendCollisionObject>(object).map(|_| object)
}

// IDA: 0088F200 NiNode_FindRagdollRootBlendCollision. +B6 includes holes and +B8 counts live elements.
// Completion: all search depths in their original order. A null slot the exe dereferences is a host stop.
fn root_blend_collision(
    world: &World,
    skeleton: Entity,
    first: Entity,
) -> Result<Option<Entity>, GameError> {
    if let Some(blend) = blend_collision(world, skeleton).or_else(|| blend_collision(world, first))
    {
        return Ok(Some(blend));
    }
    let children = world
        .get::<NiNodeChildren>(first)
        .ok_or(GameError::unclassified(
            0x0088F247,
            "a ragdoll root without node children",
        ))?;
    let slot = usize::from(children.slots_ac.iter().flatten().count() != 1);
    let Some(branch) = children.slots_ac.get(slot).copied().flatten() else {
        return Ok(None);
    };
    if let Some(blend) = blend_collision(world, branch) {
        return Ok(Some(blend));
    }
    if let Some(node) = world.get::<NiNodeChildren>(branch) {
        for child in &node.slots_ac {
            let child = child.ok_or(GameError::unclassified(
                0x0088F2A8,
                "a null ragdoll branch child",
            ))?;
            if let Some(blend) = blend_collision(world, child) {
                return Ok(Some(blend));
            }
        }
    }
    Ok(None)
}

/// IDA: 0088F700 NiNode_ApplyRagdollRootTransform. The in-world body supplies its SDK pose,
/// otherwise the first bone supplies +64. Only the translation and rotation are written to the root.
/// Completion: the original graph walk, pose stores and height gate. The physics backend supplies the SDK pose.
/// # Errors
/// A missing live body pose, invalid graph or unsupported collision update.
pub fn apply_root_transform(
    world: &mut World,
    root: Entity,
    preserve_height: bool,
) -> Result<[f32; 3], GameError> {
    let child_zero = |node| {
        world
            .get::<NiNodeChildren>(node)?
            .slots_ac
            .first()
            .copied()
            .flatten()
    };
    let Some(skeleton) = child_zero(root) else {
        return Ok([0.0; 3]);
    };
    let Some(bone) = child_zero(skeleton) else {
        return Ok([0.0; 3]);
    };
    let Some(blend) = root_blend_collision(world, skeleton, bone)? else {
        return Ok([0.0; 3]);
    };
    let zero_blend = f32::from_bits(
        world
            .get::<BhkBlendCollisionObject>(blend)
            .unwrap()
            .hierarchy_gain_14,
    ) == 0.0;
    if world.get::<NiAvCollision>(bone).is_none() && !zero_blend {
        return Ok([0.0; 3]);
    }

    let membership = world
        .get::<BhkSpCollisionObject>(blend)
        .and_then(|object| object.body_10)
        .and_then(|body| world.get::<BhkInWorld>(body));
    let (mut position, rotation) = if let Some(held) = membership {
        // 00607840 bhkWorldObject_HasHavokWorld, then 004D6900/004D6950. Both getters
        // read the same SDK motion state (+40 position, +80 quaternion).
        let pose = world
            .get::<BhkWorld>(held.world)
            .and_then(|w| w.physics)
            .zip(held.physics)
            .and_then(|(physics, body)| {
                world
                    .get_non_send::<PhysicsSeam>()?
                    .0
                    .body_pose(physics, body)
            })
            .ok_or(GameError::unclassified(
                0x0088F7B6,
                "an in-world ragdoll without a physics body pose",
            ))?;
        let [x, y, z, w] = pose.rotation_xyzw;
        (
            super::havok_world::to_game_units(pose.position),
            quaternion_to_rotation_matrix(&[w, x, y, z]),
        )
    } else {
        let transform = world
            .get::<NiAvWorld>(bone)
            .ok_or(GameError::unclassified(
                0x0088F7D7,
                "a ragdoll bone without a world transform",
            ))?
            .transform;
        (transform.translation, transform.rotation)
    };
    let mut node = world
        .get_mut::<SceneNodePayload>(root)
        .expect("the root node");
    let previous = node.local_transform.translation;
    if preserve_height || !zero_blend {
        position[2] = previous[2];
    }
    let displacement = std::array::from_fn(|axis| position[axis] - previous[axis]);
    node.local_transform.translation = position;
    node.local_transform.rotation = rotation;
    if preserve_height {
        let collision = super::havok_world::collision_object(world, root);
        let updated = match collision {
            Some(object) => update_world_transform(
                world,
                root,
                Some(&mut |world| {
                    crate::bs::havok_stream::collision_update_world_data(world, object)
                }),
            )?,
            None => update_world_transform(world, root, None)?,
        };
        if !updated {
            return Err(GameError::unclassified(
                0x0088F86E,
                "ragdoll root collision world update",
            ));
        }
    }
    Ok(displacement)
}

// IDA: 0089EAE0 bhkCollisionObject_UpdateBodyFromNode: the SDK pose written on the
// reset/sync arm can precede insertion in the world. The shape and motion still come
// from the body at insertion. Host substitution for the SDK's out-of-world motion state.
#[derive(Component, Clone, Copy)]
pub(super) struct UnattachedBodyPose(pub super::physics_seam::PhysicsTransform);

// IDA: 0088F880 bhkBlendCollisionObject_SetMotionType(1, null) with the sync bit set
// at 0049771C. Layer 8 scales each gain by zero and disables the bone's blend controller.
// TODO: [ ] Other supplied bodies, motion changes in a running world and dynamic SDK contacts.
fn set_saved_bone_motion(
    world: &mut World,
    blend: Entity,
    body: Entity,
    filter: u32,
) -> Result<(), GameError> {
    use crate::gamebryo::{
        object_net::NiObjectNet, stream_objects::NiClass, transform_controller::NodeController,
    };
    if world.get::<BhkInWorld>(body).is_some() {
        return Err(GameError::unclassified(
            0x0088F880,
            "restoring a saved ragdoll after the body entered its world",
        ));
    }
    let layer = filter & 0x3F;
    let gains = if layer == 8 {
        let pair = super::havok_stream::blend_part_pair(
            world.get_resource::<crate::tes::blend_settings::BlendSettingsState>(),
            (filter >> 8) & 31,
        );
        let mut object = world.get_mut::<BhkSpCollisionObject>(blend).unwrap();
        object.flags_0c &= !0x200;
        let mut next = object
            .target_08
            .and_then(|node| world.get::<NiObjectNet>(node)?.controller_0c);
        while let Some(controller) = next {
            let is_blend = world.get::<NiClass>(controller) == Some(&NiClass::BhkBlendController);
            let mut state =
                world
                    .get_mut::<NodeController>(controller)
                    .ok_or(GameError::unclassified(
                        0x0088F989,
                        "a ragdoll bone controller without time state",
                    ))?;
            if is_blend {
                state.time.fields.flags &= !8;
                break;
            }
            next = state.time.next_34;
        }
        if pair[0] <= 1.0 {
            pair.map(|gain| gain * 0.0)
        } else {
            [1.0; 2]
        }
    } else {
        return Err(GameError::unclassified(
            0x0088F99D,
            "saved ragdoll outside the biped collision layer",
        ));
    };
    let mut collision = world.get_mut::<BhkBlendCollisionObject>(blend).unwrap();
    collision.velocity_gain_18 = gains[1].to_bits();
    collision.hierarchy_gain_14 = gains[0].to_bits();
    Ok(())
}

// IDA: 00497640 RagDollData_ApplyNodePoses. A mismatching part skips that subtree;
// its caller still visits the remaining siblings. The index advances only on a matched body.
pub(crate) fn apply_node_poses(
    world: &mut World,
    node: Entity,
    poses: &[u8],
    index: &mut usize,
) -> Result<bool, GameError> {
    let Some(pose) = poses.get(*index * 28..).and_then(|tail| tail.get(..28)) else {
        return Ok(true);
    };
    let collision = blend_collision(world, node);
    let mut body_to_sync = None;
    if let Some(blend) = collision {
        let body = world
            .get::<BhkSpCollisionObject>(blend)
            .and_then(|sp| sp.body_10);
        let filter = body.map_or(0, |body| super::havok_world::body_filter_info(world, body));
        if u32::from(pose[0]) != (filter >> 8) & 31 {
            return Ok(false);
        }
        let float = |at: usize| f32::from_le_bytes(pose[at..at + 4].try_into().unwrap());
        let mut payload = world
            .get_mut::<SceneNodePayload>(node)
            .expect("a ragdoll bone");
        payload.local_transform.translation = [float(4), float(8), float(12)];
        payload.local_transform.rotation =
            crate::gamebryo::transform::matrix_from_euler_xyz(float(16), float(20), float(24));
        world
            .get_mut::<BhkSpCollisionObject>(blend)
            .unwrap()
            .flags_0c |= 0x40;
        if let Some(body) = body {
            set_saved_bone_motion(world, blend, body, filter)?;
            body_to_sync = Some(body);
        }
        *index += 1;
    }
    let attached = super::havok_world::collision_object(world, node);
    let mut on_collision = |world: &mut World| {
        let object = attached.expect("called only for a collision");
        if let Some(body) = body_to_sync {
            let t = world.get::<NiAvWorld>(node).unwrap().transform;
            world.entity_mut(body).insert(UnattachedBodyPose(
                super::physics_seam::PhysicsTransform {
                    translation: t
                        .translation
                        .map(|v| v * crate::bs::character_controller::HAVOK_SCALE),
                    rotation: t.rotation,
                },
            ));
        }
        crate::bs::havok_stream::collision_update_world_data(world, object);
    };
    let updated = if attached.is_some() {
        update_world_transform(world, node, Some(&mut on_collision))?
    } else {
        update_world_transform(world, node, None)?
    };
    if !updated {
        return Err(GameError::unclassified(
            0x0049773B,
            "unsupported saved ragdoll node collision",
        ));
    }
    let mut matched = true;
    let children = world.get::<NiNodeChildren>(node).unwrap().slots_ac.clone();
    for child in children {
        if *index * 28 == poses.len() {
            break;
        }
        if let Some(child) = child.filter(|c| world.get::<NiNodeChildren>(*c).is_some()) {
            matched &= apply_node_poses(world, child, poses, index)?;
        }
    }
    Ok(matched)
}
