//! BSFadeNode: 00434B40 Model_ConvertRootToFadeNode, the 004A12E0 constructor that moves a model root's contents under the
//! fade node, and the fade distances QueuedModel_Finalize's a7 arm sets (004A02A0, 004A01B0).
//! Host form: the class tag stays `NiClass::Node`, every NiNode site accepts a fade node as the original's vtable does, and
//! the `BsFadeNode` component stands for the subclass and its +0xDC..+0xEC fields.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::havok_stream::BhkSpCollisionObject;
use super::scene_graphs::{AttachChildStatus, attach_scene_child};
use crate::gamebryo::av_object::{NiAvCollision, NiAvProperties};
use crate::gamebryo::mesh_geometry::NiGeometry;
use crate::gamebryo::mesh_skin_owners::MeshSkinInstance;
use crate::gamebryo::object_net::{AlphaProperty, NiExtraDataName, NiObjectNet};
use crate::gamebryo::scene_node::{NiAvWorld, NiNodeChildren, NiNodeEffects, SceneNodePayload};
use crate::gamebryo::scene_state::attach_object_property;
use crate::gamebryo::scene_update::{
    NoSceneControllers, SceneUpdateStatus, update_node_world_bound, update_scene_node_model_bound,
    update_scene_object,
};
use crate::gamebryo::stream_objects::{NULL_REFERENCE, NiClass};
use crate::gamebryo::transform::NiTransform;

/// IDA: BSFadeNode (0xF0, vtable `BSFadeNode::vftable`): the NiNode (0xDC), then +0xDC a byte, +0xE0 the near distance squared,
/// +0xE4 the far distance squared, +0xE8 a float, +0xEC the LOD stage byte.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct BsFadeNode {
    pub byte_dc: u8,
    pub near_squared_e0: f32,
    pub far_squared_e4: f32,
    pub float_e8: f32,
    pub stage_ec: u8,
}

/// IDA: the INI statics the a7 arm reads live: 00B05148 fLODBoundRadiusMult:LOD, 00B05150 fLODFadeOutPercent:LOD, 00B07280
/// iFadeNodeMinNearDistance:LOD. The defaults are the exe's image values, for a world with no INI; `begin_load_selected_files`
/// publishes the live ones.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct FadeNodeSettings {
    pub bound_radius_mult_b05148: f32,
    pub fade_out_percent_b05150: f32,
    pub min_near_distance_b07280: i32,
}

impl Default for FadeNodeSettings {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            bound_radius_mult_b05148: f32::from_bits(0x4040_0000),
            fade_out_percent_b05150: f32::from_bits(0x3F66_6666),
            min_near_distance_b07280: 0x190,
        }
    }
}

/// IDA: 00B35290, the NiAlphaProperty 004A07E0 creates once for every fade node.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FadeNodeGlobals {
    pub alpha_property_b35290: Option<Entity>,
}

/// IDA: 004A07E0 BSFadeNode_InitFadeState: the first call creates the 0x1C NiAlphaProperty (NiObjectNET, flags 0xEC, test
/// reference 0) into 00B35290 and sets its flag bit 0; every call stores +0xEC = 0, +0xE0 = 0.0, +0xDC = 1, +0xE4 = 0.0,
/// +0xE8 = 1.0. Completion: 100% of the stores; the refcount swap on 00B35290 has no host form.
pub fn fade_node_init_fade_state(world: &mut World, node: Entity) {
    if world
        .get_resource::<FadeNodeGlobals>()
        .and_then(|globals| globals.alpha_property_b35290)
        .is_none()
    {
        let alpha = world
            .spawn((
                NiClass::AlphaProperty,
                NiObjectNet::default(),
                AlphaProperty {
                    flags_18: 0x00EC | 1,
                    alpha_test_ref_1a: 0,
                },
            ))
            .id();
        world
            .get_resource_or_init::<FadeNodeGlobals>()
            .alpha_property_b35290 = Some(alpha);
    }
    world.entity_mut(node).insert(BsFadeNode {
        byte_dc: 1,
        near_squared_e0: 0.0,
        far_squared_e4: 0.0,
        float_e8: 1.0,
        stage_ec: 0,
    });
}

/// IDA: 004A0760 BSFadeNode_SetSkinRootParents(node, fade): for each nonnull child below +0xB6, a node (vt+8) recurses, and a
/// geometry (vt+0xC) with a +0xB8 skin instance gets its +0x10 root parent = `fade`. Completion: 100%.
pub fn fade_node_set_skin_root_parents(world: &mut World, node: Entity, fade: Entity) {
    let children = world
        .get::<NiNodeChildren>(node)
        .map(|children| children.slots_ac.clone())
        .unwrap_or_default();
    for child in children.into_iter().flatten() {
        if world.get::<NiNodeChildren>(child).is_some() {
            fade_node_set_skin_root_parents(world, child, fade);
        } else if let Some(skin) = world
            .get::<NiGeometry>(child)
            .and_then(|geometry| geometry.skin_b8)
            && let Some(mut instance) = world.get_mut::<MeshSkinInstance>(skin)
        {
            instance.root_parent_owner_10 = Some(fade);
        }
    }
}

// IDA: 00435CE0 NiAVObject_SetCollisionObject(object, collision): a changed +0xA8 drops the old reference and takes the new;
// then a collision object whose +0x08 target is not `object` gets vt+0x4C SetSceneObject(object). Completion: 100% for the
// collision classes the host links (bhkSPCollisionObject); the payload's index travels with the object it names.
pub(crate) fn set_collision_object(
    world: &mut World,
    object: Entity,
    collision: Option<Entity>,
    index: u32,
) -> Result<(), StreamError> {
    match collision {
        Some(collision) => {
            world.entity_mut(object).insert(NiAvCollision(collision));
            let mut target =
                world
                    .get_mut::<BhkSpCollisionObject>(collision)
                    .ok_or(StreamError(
                        "00435D1F: SetSceneObject on a collision class the host does not link",
                    ))?;
            if target.target_08 != Some(object) {
                target.target_08 = Some(object);
            }
        }
        None => {
            world.entity_mut(object).remove::<NiAvCollision>();
        }
    }
    if let Some(mut payload) = world.get_mut::<SceneNodePayload>(object) {
        payload.collision_index = index;
    }
    Ok(())
}

/// IDA: 004A12E0 BSFadeNode_Constructor(root): NiNode(0), the BSFadeNode vtable, the root's name. A root with no +0x0C
/// controller and no NiBillboardNode (00B3FD4C) in its RTTI chain: an identity +0x30 rotation (0070FF20, exact float
/// compares) moves the root's collision object (00435CE0 twice); otherwise a new NiNode takes the root through vt+0x84
/// AttachChild(root, 0) and the root is renamed "FadeNode Rot". A controller or a billboard: the same wrapper, renamed
/// "FadeNode Anim". Then the source's (the root, or the wrapper) children AttachChild(child, 1) to the fade node in slot
/// order; 004A0760; the root's properties head first AttachProperty (AddHead) then RemoveProperty; its dynamic effects; the
/// source's +0x54 translate, +0x30 rotate and |+0x60| scale; under 00B3F600 the root's extra data, capacity then index 0
/// until empty; 004A07E0; NiNode_UpdateWorldBound and UpdateModelBound. Completion: 100% but the dynamic-effect move, a stop.
/// Host substitutions: the emptied root and the wrapper stay unreferenced entities; the extra data list is already sorted
/// and named by its link (006FF8A0, 006FF570), so moving it in order is what the re-insert yields.
pub fn fade_node_construct(world: &mut World, root: Entity) -> Result<Entity, StreamError> {
    let name = world
        .get::<NiObjectNet>(root)
        .ok_or(StreamError("004A1300: a model root without NiObjectNET"))?
        .name
        .clone();
    let fade = spawn_node(world, NiTransform::default());
    world.get_mut::<NiObjectNet>(fade).expect("spawned").name = name;
    let controller = world
        .get::<NiObjectNet>(root)
        .is_some_and(|net| net.controller_0c.is_some());
    let billboard = world.get::<NiClass>(root) == Some(&NiClass::BillboardNode);
    let root_transform = world
        .get::<SceneNodePayload>(root)
        .ok_or(StreamError("004A1300: a model root that is not a NiNode"))?
        .local_transform;
    let source = if !controller
        && !billboard
        && root_transform.rotation == NiTransform::default().rotation
    {
        // 004A1376 00435CE0(fade, root +0xA8), then 00435CE0(root, null).
        let collision = world
            .get::<NiAvCollision>(root)
            .map(|collision| collision.0);
        let index = world
            .get::<SceneNodePayload>(root)
            .map_or(NULL_REFERENCE, |payload| payload.collision_index);
        set_collision_object(world, fade, collision, index)?;
        set_collision_object(world, root, None, NULL_REFERENCE)?;
        root
    } else {
        let wrapper = spawn_node(world, NiTransform::default());
        if attach_scene_child(world, wrapper, Some(root), false) != AttachChildStatus::Attached {
            return Err(StreamError("004A13A9: the wrapper's AttachChild failed"));
        }
        let rename: &[u8] = if controller || billboard {
            b"FadeNode Anim"
        } else {
            b"FadeNode Rot"
        };
        world.get_mut::<NiObjectNet>(root).expect("checked").name = Some(rename.to_vec());
        wrapper
    };
    // 004A13F0 the source's children, slot order; each AttachChild detaches it from the source first.
    let children = world
        .get::<NiNodeChildren>(source)
        .map(|children| children.slots_ac.clone())
        .unwrap_or_default();
    for child in children.into_iter().flatten() {
        if attach_scene_child(world, fade, Some(child), true) != AttachChildStatus::Attached {
            return Err(StreamError("004A1410: AttachChild to the fade node failed"));
        }
    }
    fade_node_set_skin_root_parents(world, fade, fade);
    // 004A1430 the root's properties: the head AttachProperty on the fade node, then RemoveProperty on the root.
    while let Some(property) = world
        .get::<NiAvProperties>(root)
        .and_then(|properties| properties.0.first().copied())
    {
        attach_object_property(world, fade, property)?;
        let mut properties = world.get_mut::<NiAvProperties>(root).expect("checked");
        if let Some(position) = properties.0.iter().position(|&other| other == property) {
            properties.0.remove(position);
        }
    }
    if world
        .get::<NiNodeEffects>(root)
        .is_some_and(|effects| !effects.0.is_empty())
    {
        return Err(StreamError(
            "004A1460: a model root with dynamic effects (00708E40, 0070B930)",
        ));
    }
    // 004A1488 the source's transform, scale through fabs.
    let mut transform = world
        .get::<SceneNodePayload>(source)
        .expect("a NiNode")
        .local_transform;
    transform.scale = transform.scale.abs();
    world
        .get_mut::<SceneNodePayload>(fade)
        .expect("spawned")
        .local_transform = transform;
    // 004A14C8..004A1530 the extra data, under 00B3F600.
    let extra = std::mem::take(
        &mut world
            .get_mut::<NiObjectNet>(root)
            .expect("checked")
            .extra_data,
    );
    world
        .get_mut::<NiObjectNet>(fade)
        .expect("spawned")
        .extra_data = extra;
    fade_node_init_fade_state(world, fade);
    // 004A1543 NiNode_UpdateWorldBound, 004A154A NiNode_UpdateModelBound.
    update_node_world_bound(world, fade)?;
    update_scene_node_model_bound(world, fade);
    Ok(fade)
}

// IDA: N/A a NiNode(0) on a new entity (`spawn_cold_node`) with a local transform.
fn spawn_node(world: &mut World, transform: NiTransform) -> Entity {
    let node = crate::gamebryo::scene_node::spawn_cold_node(world);
    world
        .get_mut::<SceneNodePayload>(node)
        .expect("spawned")
        .local_transform = transform;
    node
}

/// IDA: 00434B40 Model_ConvertRootToFadeNode: a null +0x08 returns null; a root whose vt+0x98 IsFadeNode answers returns it;
/// otherwise a 0xF0 block through 004A12E0 replaces +0x08 (NiRefObjectPtr_Assign). Completion: 100%.
pub fn model_convert_root_to_fade_node(
    world: &mut World,
    root: Option<Entity>,
) -> Result<Option<Entity>, StreamError> {
    let Some(root) = root else {
        return Ok(None);
    };
    if world.get::<BsFadeNode>(root).is_some() {
        return Ok(Some(root));
    }
    fade_node_construct(world, root).map(Some)
}

/// IDA: 004A02A0 BSFadeNode_SetFadeDistances(near, far): iFadeNodeMinNearDistance (00B07280, `fild` then a float) above
/// `near` raises it to that value and `far` to at least 1.25 times it; +0xE0 = near * near, +0xE4 = far * far.
/// Completion: 100%, float stores of double products.
pub fn fade_node_set_fade_distances(
    fade: &mut BsFadeNode,
    mut near: f32,
    mut far: f32,
    min_near_distance: i32,
) {
    let minimum = min_near_distance as f32;
    if minimum > near {
        near = minimum;
        let raised = f64::from(minimum) * 1.25;
        if f64::from(far) < raised {
            far = raised as f32;
        }
    }
    fade.near_squared_e0 = (f64::from(near) * f64::from(near)) as f32;
    fade.far_squared_e4 = (f64::from(far) * f64::from(far)) as f32;
}

/// IDA: 004A01B0 BSFadeNode_SetStage(stage): +0xEC unless it holds 1. Completion: 100%.
pub fn fade_node_set_stage(fade: &mut BsFadeNode, stage: u8) {
    if fade.stage_ec != 1 && fade.stage_ec != stage {
        fade.stage_ec = stage;
    }
}

/// IDA: 0043966D..00439718 the a7 arm's fade distances from the radius: far = uGridsToLoad << 12 (an unsigned `fild`: 2^32 added when bit 19 of the setting, the shifted sign, is set) times 0.5
/// less (radius + 2048), unless the radius is nonzero and fLODBoundRadiusMult times it is smaller; near = far times
/// fLODFadeOutPercent. The IO thread's 53-bit x87: each store rounds to a float. Completion: 100%.
pub fn fade_distances(
    radius: f32,
    grids_to_load: u32,
    bound_radius_mult: f32,
    fade_out_percent: f32,
) -> Result<(f32, f32), StreamError> {
    let r = f64::from(radius);
    let mut grids = f64::from((grids_to_load << 12) as i32);
    if grids_to_load & 0x80000 != 0 {
        grids += 4_294_967_296.0;
    }
    let mut far = (grids * 0.5 - (r + 2048.0)) as f32;
    let scaled = f64::from(bound_radius_mult) * r;
    if r != 0.0 && f64::from(far) > scaled {
        far = scaled as f32;
    }
    let near = (f64::from(far) * f64::from(fade_out_percent)) as f32;
    Ok((near, far))
}

/// IDA: 0043961E..00439718 QueuedModel_Finalize's a7 arm with a LOD: 00434B40 on the Model, NiAVObject_Update(0, 0), the fade
/// distances, 004A02A0 and 004A01B0(+0x30). Returns the fade node. Completion: 100% but the stops above.
pub fn finalize_fade_root(
    world: &mut World,
    root: Option<Entity>,
    stage: u8,
    grids_to_load: u32,
    settings: FadeNodeSettings,
) -> Result<Option<Entity>, StreamError> {
    let Some(fade) = model_convert_root_to_fade_node(world, root)? else {
        return Err(StreamError("00439626: a null model root through 00434B40"));
    };
    let result = update_scene_object(world, fade, 0.0, false, &mut NoSceneControllers)?;
    if result.status != SceneUpdateStatus::Updated {
        return Err(StreamError(match result.status {
            SceneUpdateStatus::UnsupportedCollision => {
                "00439630: the fade node's update reached a collision object"
            }
            SceneUpdateStatus::UnsupportedSkinnedBound => {
                "00439630: the fade node's update reached a skinned bound"
            }
            SceneUpdateStatus::UnsupportedController
            | SceneUpdateStatus::ControllerFailed
            | SceneUpdateStatus::ParticleScheduleFailed => {
                "00439630: the fade node's update reached a controller"
            }
            SceneUpdateStatus::UnsupportedClass => {
                "00439630: the fade node's update reached a class the host does not bind"
            }
            _ => "00439630: NiAVObject_Update stopped on the fade node",
        }));
    }
    // 0043963A: below 0.001, half 004DC4B0's z on the Model's +0x08, the fade node now.
    let mut radius = world.get::<NiAvWorld>(fade).expect("a NiNode").bound[3];
    if radius < 0.001 {
        radius = (f64::from(bound_extra_top(world, fade)?[2]) * 0.5) as f32;
    }
    let (near, far) = fade_distances(
        radius,
        grids_to_load,
        settings.bound_radius_mult_b05148,
        settings.fade_out_percent_b05150,
    )?;
    let mut node = world.get_mut::<BsFadeNode>(fade).expect("constructed");
    fade_node_set_fade_distances(&mut node, near, far, settings.min_near_distance_b07280);
    fade_node_set_stage(&mut node, stage);
    Ok(Some(fade))
}

/// IDA: 00B35EF4..00B35EFC, the statics 004DC4B0 keeps its last sum in.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct BoundExtraTopStatics {
    pub top_b35ef4: [f32; 3],
}

/// IDA: 004DC4B0 NiAVObject_GetBoundExtraTop(out, object): a nonnull object's first extra named "BBX" (NiObjectNET_GetExtraData,
/// 00A7D2CC), read as a BSBound: its +0x18 extents into the 00B35EF4 statics, then each plus its +0x0C center (`fld`, `fadd`,
/// `fstp` to a float: one rounding of an exact sum, the same at any x87 precision), stored back and answered; otherwise
/// g_niPoint3Zero. Completion: 100%; a "BBX" extra that isn't a BSBound is a stop.
/// # Errors
/// A "BBX" extra of another class.
pub fn bound_extra_top(world: &mut World, object: Entity) -> Result<[f32; 3], StreamError> {
    let extras = world
        .get::<NiObjectNet>(object)
        .map(|net| net.extra_data.clone())
        .unwrap_or_default();
    let named_bbx = extras.into_iter().find(|&extra| {
        world
            .get::<NiExtraDataName>(extra)
            .is_some_and(|name| name.0.as_deref() == Some(b"BBX".as_slice()))
    });
    let Some(extra) = named_bbx else {
        return Ok([0.0; 3]);
    };
    let bound = *world
        .get::<crate::gamebryo::extra_data::BsBound>(extra)
        .ok_or(StreamError(
            "004DC4C6: a \"BBX\" extra that isn't a BSBound",
        ))?;
    let top: [f32; 3] = std::array::from_fn(|axis| {
        f32::from_bits(bound.center_0c[axis]) + f32::from_bits(bound.extents_18[axis])
    });
    world
        .get_resource_or_init::<BoundExtraTopStatics>()
        .top_b35ef4 = top;
    Ok(top)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 004A02A0: a near below 400 rises to 400 and the far to at least 500.
    #[test]
    fn fade_distances_respect_the_minimum_near() {
        let mut fade = BsFadeNode {
            byte_dc: 1,
            near_squared_e0: 0.0,
            far_squared_e4: 0.0,
            float_e8: 1.0,
            stage_ec: 0,
        };
        fade_node_set_fade_distances(&mut fade, 90.0, 100.0, 400);
        assert_eq!(fade.near_squared_e0, 160_000.0);
        assert_eq!(fade.far_squared_e4, 250_000.0);
        fade_node_set_stage(&mut fade, 6);
        assert_eq!(fade.stage_ec, 6);
        fade.stage_ec = 1;
        fade_node_set_stage(&mut fade, 6);
        assert_eq!(fade.stage_ec, 1);
    }

    // 0043962D: uGridsToLoad 5 gives 8192 less the radius, capped by three radii.
    #[test]
    fn fade_distances_cap_by_the_bound_radius() {
        let (near, far) = fade_distances(100.0, 5, 3.0, 0.9).unwrap();
        assert_eq!(far, 300.0);
        assert_eq!(near, (300.0f64 * f64::from(0.9f32)) as f32);
        let (_, far) = fade_distances(4000.0, 5, 3.0, 0.9).unwrap();
        assert_eq!(far, 4192.0);
    }
}
