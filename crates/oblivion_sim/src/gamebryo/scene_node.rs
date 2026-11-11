//! NiNode as an entity: its stream payload, world state, child slots and effect list.

use std::collections::HashSet;

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::av_object::{
    AV_PROPERTY_HEAD_INSERT_VERSION, AvObjectLinkStatus, AvObjectPrefix, NiAvParent,
    av_has_property_controller, av_properties, commit_av_object_links, parse_av_object_prefix,
    plan_av_object_links, read_av_reference_list, set_leaf_selective_update_flags,
};
use super::camera::SceneCamera;
use super::controller_time::is_time_controller;
use super::mesh_geometry::{NiGeometry, set_geometry_selective_update_flags};
use super::object_net::{NiObjectNet, ObjectNetPending, controller_head, get_controller};
use super::particle_system::ParticleSystem;
use super::scene_derived::DirectionalLight;
use super::stream_objects::check_link_index;
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};
use super::transform::{NiTransform, compose_scene_transform};

/// IDA: N/A the error of this file's `stream_objects::check_link_index` calls (the host's guard before 007124A0 / 00752CB0's reads).
const NODE_REFERENCE_OUT_OF_RANGE: GameError =
    GameError::unclassified(0, "NiNode deferred reference out of range");

/// IDA: 0070B780 NiNode_Constructor(0) on a new entity: NiAVObject_Constructor's 0x000E flags, identity transforms, no
/// children or effects. Completion: 100% of the cold state.
pub fn spawn_cold_node(world: &mut World) -> Entity {
    world
        .spawn((
            NiClass::Node,
            SceneNodePayload::default(),
            NiObjectNet::default(),
            NiAvWorld::default(),
            NiAvParent::default(),
            NiNodeChildren {
                slots_ac: Vec::new(),
                links_resolved: true,
            },
            NiNodeEffects::default(),
        ))
        .id()
}

/// IDA: 0070B780 NiNode and 00722490 NiBillboardNode vtable identities.
pub const NI_NODE_VTABLE: u32 = 0x00A7_E38C;
pub const NI_BILLBOARD_NODE_VTABLE: u32 = 0x00A4_5134;

/// IDA: 00707F00 and 00709EE0's deferred state. `stream_version` is NiStream +0xD8, read by the link gates.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct SceneNodePayload {
    pub net: ObjectNetPending,
    pub raw_av_flags: u16,
    pub effective_av_flags: u16,
    pub local_transform: NiTransform,
    pub property_indices: Vec<u32>,
    pub collision_index: u32,
    pub child_indices: Vec<u32>,
    pub effect_indices: Vec<u32>,
    pub stream_version: u32,
    pub bytes_consumed: usize,
}

impl Default for SceneNodePayload {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            net: ObjectNetPending::default(),
            raw_av_flags: 0x000E,
            effective_av_flags: 0x000E,
            local_transform: NiTransform::default(),
            property_indices: Vec::new(),
            collision_index: NULL_REFERENCE,
            child_indices: Vec::new(),
            effect_indices: Vec::new(),
            stream_version: 0x1400_0004,
            bytes_consumed: 0,
        }
    }
}

/// IDA: NiAVObject's world state: +0x64 transform and the bound. The combined bound +0xCC stays unset until
/// its update routine runs.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct NiAvWorld {
    pub transform: NiTransform,
    pub bound: [f32; 4],
    pub combined_bound: Option<[f32; 4]>,
}

/// IDA: 0070B530 / 0070B250 the +0xAC child slots. Order and null slots are observable, so this is an
/// explicit array rather than Bevy `Children`.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct NiNodeChildren {
    pub slots_ac: Vec<Option<Entity>>,
    /// IDA: N/A a fresh graph has none; the host rejects a relink, whose detach is not ported.
    pub links_resolved: bool,
}

/// IDA: 0070B530 the +0xBC NiTPointerList<NiDynamicEffect*>, head first, without refcounts.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct NiNodeEffects(pub Vec<Entity>);

/// IDA: an effect's +0xC0 list of affected nodes, kept by 00708E40.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct NiEffectAffectedNodes(pub Vec<Entity>);

/// IDA: 00707F00's NiAVObject prefix, then 00709EE0's child and effect lists, which clears +0x18 bit
/// 0x40. Completion: 100% of the >= 0A000100 widths and order (00709EE0 has no gate of its own).
pub fn parse_scene_node_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<SceneNodePayload, GameError> {
    const TRUNCATED: &str = "truncated NiNode stream payload";
    const LIST_TRUNCATED: &str = "truncated NiNode deferred list";
    if stream_version < 0x0A00_0100 {
        return Err(GameError::unclassified(
            0,
            "legacy NiNode payload unsupported",
        ));
    }
    let (prefix, mut reader) =
        parse_av_object_prefix(bytes, stream_version, TRUNCATED, LIST_TRUNCATED)?;
    let AvObjectPrefix {
        net,
        raw_av_flags,
        mut effective_av_flags,
        local_transform,
        property_indices,
        collision_index,
    } = prefix;
    let child_indices = read_av_reference_list(
        &mut reader,
        TRUNCATED,
        GameError::from_legacy_message(LIST_TRUNCATED),
    )?;
    let effect_indices = read_av_reference_list(
        &mut reader,
        TRUNCATED,
        GameError::from_legacy_message(LIST_TRUNCATED),
    )?;
    effective_av_flags &= !0x40;
    let bytes_consumed = net.bytes_consumed + reader.position();
    Ok(SceneNodePayload {
        net,
        raw_av_flags,
        effective_av_flags,
        local_transform,
        property_indices,
        collision_index,
        child_indices,
        effect_indices,
        stream_version,
        bytes_consumed,
    })
}

/// IDA: 0070BA70's cold NiNode, on an existing or new entity.
pub fn spawn_scene_node(world: &mut World, payload: SceneNodePayload) -> Entity {
    world
        .spawn((
            NiClass::Node,
            payload,
            NiObjectNet::default(),
            NiAvWorld::default(),
            NiAvParent::default(),
            NiNodeChildren::default(),
            NiNodeEffects::default(),
        ))
        .id()
}

/// IDA: N/A 0070C120 (`update_object_world_transform`) on a NiNode: its +0x30, +0x1C and collision reference from the
/// node's own components.
/// # Errors
/// As `update_object_world_transform`.
pub fn update_world_transform(
    world: &mut World,
    node: Entity,
    update_collision: Option<&mut dyn FnMut(&mut World)>,
) -> Result<bool, GameError> {
    let payload = world
        .get::<SceneNodePayload>(node)
        .expect("a NiNode has a payload");
    let local = payload.local_transform;
    let collision_index = payload.collision_index;
    let parent = world
        .get::<NiAvParent>(node)
        .expect("a NiNode has a parent slot")
        .0;
    update_object_world_transform(
        world,
        node,
        parent,
        &local,
        collision_index,
        update_collision,
    )
}

/// IDA: 0070C120 NiAVObject +0x74 UpdateWorldData: compose parent +0x64 and this +0x30 into +0x64, or copy +0x30 without
/// a parent. The optional collision vtable +0x50 runs only after that store. Returns false when a collision reference has
/// no callback. `parent` (+0x1C) and `local` (+0x30) are read by the caller from its class's components.
/// Completion: 100% of the branch and store order.
/// TODO: [ ] x87 rounding, and the concrete collision vtable.
/// # Errors
/// A parent with no world state.
pub fn update_object_world_transform(
    world: &mut World,
    object: Entity,
    parent: Option<Entity>,
    local: &NiTransform,
    collision_index: u32,
    update_collision: Option<&mut dyn FnMut(&mut World)>,
) -> Result<bool, GameError> {
    let transform = match parent {
        Some(parent) => {
            let source = world
                .get::<NiAvWorld>(parent)
                .ok_or(GameError::unclassified(
                    0,
                    "NiNode parent world transform unavailable",
                ))?;
            compose_scene_transform(&source.transform, local)
        }
        None => *local,
    };
    world
        .get_mut::<NiAvWorld>(object)
        .expect("a NiNode has world state")
        .transform = transform;
    if collision_index != NULL_REFERENCE {
        let Some(update) = update_collision else {
            return Ok(false);
        };
        update(world);
    }
    Ok(true)
}

/// IDA: 0070B530's outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneNodeLinkStatus {
    Linked,
    UnsupportedOwner,
}

/// IDA: N/A the C++ port's cycle guard around 0070B250's parent store. A cycle cannot leak here, but the host
/// keeps rejecting what it rejected, because the original's detach is not ported.
fn creates_cycle(world: &World, parent: Entity, child: Entity) -> bool {
    let mut pending = vec![child];
    let mut visited = HashSet::new();
    while let Some(current) = pending.pop() {
        if current == parent {
            return true;
        }
        if !visited.insert(current) {
            continue;
        }
        if let Some(children) = world.get::<NiNodeChildren>(current) {
            pending.extend(children.slots_ac.iter().flatten().copied());
        }
    }
    false
}

/// IDA: 0070B120 NiNode vt+0x88 DetachChild(out, child): walks the slots below +0xB6 for the first holding `child`. A hit clears the child's +0x1C and runs 006D7F60 RemoveAt: the slot goes null, +0xB8 drops by one, and +0xB6 drops by one only for the last slot, so an inner slot stays as a null. Returns the child, or null on a miss. Completion: 100%; the host has no +0xB8 and no counts.
pub fn detach_child(world: &mut World, node: Entity, child: Entity) -> Option<Entity> {
    let mut children = world.get_mut::<NiNodeChildren>(node)?;
    let index = children
        .slots_ac
        .iter()
        .position(|&slot| slot == Some(child))?;
    children.slots_ac[index] = None;
    if index + 1 == children.slots_ac.len() {
        children.slots_ac.pop();
    }
    world.entity_mut(child).insert(NiAvParent(None));
    Some(child)
}

/// IDA: 0070B250 NiNode vt+0x90 SetChildAt(out, index, child): an index at or past +0xB6: 007074B0 (a parent's vt+0x88 detach, then +0x1C = this), the growth 00523B10 and SetAt 004B34E0, returning null. A lower index: the old slot's +0x1C goes null, 007074B0, SetAt, returning the old child. Completion: 100%; 00523B10's capacity has no host form, the array grows to `index + 1` with null slots.
pub fn set_child_at(
    world: &mut World,
    node: Entity,
    index: usize,
    child: Option<Entity>,
) -> Option<Entity> {
    let old = world
        .get::<NiNodeChildren>(node)
        .expect("a NiNode has child slots")
        .slots_ac
        .get(index)
        .copied()
        .flatten();
    if let Some(old) = old {
        world.entity_mut(old).insert(NiAvParent(None));
    }
    if let Some(child) = child {
        // 007074B0.
        let parent = world.get::<NiAvParent>(child).and_then(|parent| parent.0);
        if let Some(parent) = parent {
            detach_child(world, parent, child);
        }
        world.entity_mut(child).insert(NiAvParent(Some(node)));
    }
    let mut children = world
        .get_mut::<NiNodeChildren>(node)
        .expect("a NiNode has child slots");
    if index >= children.slots_ac.len() {
        children.slots_ac.resize(index + 1, None);
    }
    children.slots_ac[index] = child;
    old
}

/// IDA: 0070B530's child stage: the preflight, then 00523B10 SetSize and per raw slot 0070B250 (007074B0's parent
/// store before the slot store), then the effect loop: modern AddHead, legacy AddTail, and 00708E40 adds this node
/// to the effect's list when absent. Completion: 100% of the order on a fresh graph.
/// TODO: [ ] Detach and reparent, refcounts, and NiTArray growth.
fn link_stage(
    world: &mut World,
    node: Entity,
    resolved_children: &[Option<Entity>],
    resolved_effects: &[Option<Entity>],
    object_count: usize,
    publish_av: impl FnOnce(&mut World) -> Result<(), GameError>,
) -> Result<SceneNodeLinkStatus, GameError> {
    let payload = world
        .get::<SceneNodePayload>(node)
        .expect("a NiNode has a payload")
        .clone();
    if resolved_children.len() != payload.child_indices.len()
        || resolved_effects.len() != payload.effect_indices.len()
    {
        return Err(GameError::unclassified(
            0,
            "NiNode self/child/effect slot mismatch",
        ));
    }
    let children_state = world
        .get::<NiNodeChildren>(node)
        .expect("a NiNode has child slots");
    if children_state.links_resolved
        || !children_state.slots_ac.is_empty()
        || !world
            .get::<NiNodeEffects>(node)
            .expect("a NiNode has an effect list")
            .0
            .is_empty()
    {
        return Ok(SceneNodeLinkStatus::UnsupportedOwner);
    }
    // A non-null controller needs a head the graph has already bound (00700050 +0x0C).
    if payload.net.controller_index != NULL_REFERENCE
        && world
            .get::<NiObjectNet>(node)
            .is_none_or(|net| net.controller_0c.is_none())
    {
        return Ok(SceneNodeLinkStatus::UnsupportedOwner);
    }
    let mut seen = HashSet::new();
    for (&index, &child) in payload.child_indices.iter().zip(resolved_children) {
        check_link_index(index, object_count, NODE_REFERENCE_OUT_OF_RANGE)?;
        if child.is_some() != (index != NULL_REFERENCE) || child == Some(node) {
            return Err(GameError::unclassified(
                0,
                "NiNode child reference/owner mismatch",
            ));
        }
        let Some(child) = child else {
            continue;
        };
        if !world
            .get::<NiClass>(child)
            .is_some_and(|class| class.is_av_object())
        {
            return Ok(SceneNodeLinkStatus::UnsupportedOwner);
        }
        // A previous parent, or the same object twice, needs the unported detach.
        let has_parent = world
            .get::<NiAvParent>(child)
            .is_some_and(|parent| parent.0.is_some());
        if has_parent || !seen.insert(child) || creates_cycle(world, node, child) {
            return Ok(SceneNodeLinkStatus::UnsupportedOwner);
        }
    }
    for (&index, &effect) in payload.effect_indices.iter().zip(resolved_effects) {
        check_link_index(index, object_count, NODE_REFERENCE_OUT_OF_RANGE)?;
        if effect.is_some() != (index != NULL_REFERENCE) {
            return Err(GameError::unclassified(
                0,
                "NiNode effect reference/owner mismatch",
            ));
        }
    }

    publish_av(world)?;
    let mut slots = Vec::with_capacity(resolved_children.len());
    for &child in resolved_children {
        if let Some(child) = child {
            world.entity_mut(child).insert(NiAvParent(Some(node)));
        }
        slots.push(child);
    }
    let mut children = world
        .get_mut::<NiNodeChildren>(node)
        .expect("checked above");
    children.slots_ac = slots;
    let head_insert = payload.stream_version >= AV_PROPERTY_HEAD_INSERT_VERSION;
    for effect in resolved_effects.iter().flatten().copied() {
        let mut effects = world.get_mut::<NiNodeEffects>(node).expect("checked above");
        if head_insert {
            effects.0.insert(0, effect);
        } else {
            effects.0.push(effect);
        }
        if world.get::<NiEffectAffectedNodes>(effect).is_none() {
            world
                .entity_mut(effect)
                .insert(NiEffectAffectedNodes::default());
        }
        let mut affected = world
            .get_mut::<NiEffectAffectedNodes>(effect)
            .expect("just ensured");
        if !affected.0.contains(&node) {
            affected.0.insert(0, node);
        }
    }
    world
        .get_mut::<NiNodeChildren>(node)
        .expect("checked above")
        .links_resolved = true;
    Ok(SceneNodeLinkStatus::Linked)
}

/// IDA: 0070B530 NiNode_LinkObject in full: 007081B0 (extras, +0x98 properties, +0xA8), then children, then
/// effects. Every owner is preflighted before the first store, so an unsupported sibling leaves the node fresh.
/// The caller publishes the controller head first. Completion: 100% of the order for the bound classes.
pub fn link_scene_node(
    world: &mut World,
    node: Entity,
    resolved_children: &[Option<Entity>],
    resolved_effects: &[Option<Entity>],
    table: &StreamObjectTable,
) -> Result<SceneNodeLinkStatus, GameError> {
    let payload = world
        .get::<SceneNodePayload>(node)
        .expect("a NiNode has a payload")
        .clone();
    let plan = plan_av_object_links(
        world,
        &payload.net,
        &payload.property_indices,
        payload.collision_index,
        table,
        payload.stream_version,
    )?;
    if plan.status != AvObjectLinkStatus::Linked {
        return Ok(SceneNodeLinkStatus::UnsupportedOwner);
    }
    link_stage(
        world,
        node,
        resolved_children,
        resolved_effects,
        table.objects.len(),
        |world| commit_av_object_links(world, node, &payload.net, plan, table),
    )
}

/// IDA: 0070B530 without a stream table: NiNode-only children, with no extras, properties, collision or effects.
/// A host entry that shares the child stage. TODO: [ ] Retire once every caller passes the table.
pub fn link_scene_node_children(
    world: &mut World,
    node: Entity,
    resolved_children: &[Option<Entity>],
    object_count: usize,
) -> Result<SceneNodeLinkStatus, GameError> {
    let payload = world
        .get::<SceneNodePayload>(node)
        .expect("a NiNode has a payload")
        .clone();
    for &index in payload
        .property_indices
        .iter()
        .chain(&payload.effect_indices)
    {
        check_link_index(index, object_count, NODE_REFERENCE_OUT_OF_RANGE)?;
    }
    let nonnull = |indices: &[u32]| indices.iter().any(|&index| index != NULL_REFERENCE);
    if !payload.net.extra_indices.is_empty()
        || nonnull(&payload.property_indices)
        || payload.collision_index != NULL_REFERENCE
        || nonnull(&payload.effect_indices)
    {
        return Ok(SceneNodeLinkStatus::UnsupportedOwner);
    }
    let effects = vec![None; payload.effect_indices.len()];
    link_stage(
        world,
        node,
        resolved_children,
        &effects,
        object_count,
        |_| Ok(()),
    )
}

// IDA: 0070AC00 NiNode vt+0x58 GetObjectByName(name): 007073F0 on this object, then each nonnull slot below +0xB6 through its own vt+0x58, the first hit returned. Completion: 100%. Leaf classes run 007073F0 alone.
pub fn object_by_name(world: &World, object: Entity, name: &[u8]) -> Option<Entity> {
    if super::av_object::object_named(world, object, name) {
        return Some(object);
    }
    world
        .get::<NiNodeChildren>(object)?
        .slots_ac
        .iter()
        .flatten()
        .find_map(|&child| object_by_name(world, child, name))
}

// IDA: N/A the slots of a NiNode's child array (+0xB0 below +0xB6).
pub(crate) fn child_slots(world: &World, node: Entity) -> Vec<Option<Entity>> {
    world
        .get::<NiNodeChildren>(node)
        .map(|children| children.slots_ac.clone())
        .unwrap_or_default()
}

/// IDA: 00471B80(node): a NiNode's children in slot order, a NiNode recursed, a geometry (vt+0x0C) with a +0xB8 skin instance
/// answering true. Completion: 100%.
#[must_use]
pub fn has_skin(world: &World, node: Entity) -> bool {
    child_slots(world, node).into_iter().flatten().any(|child| {
        if world.get::<NiNodeChildren>(child).is_some() {
            has_skin(world, child)
        } else {
            world
                .get::<NiGeometry>(child)
                .is_some_and(|geometry| geometry.skin_b8.is_some())
        }
    })
}

/// IDA: 00480770(node), a weapon's in Load3D: the children in slot order; the first named `Scb` (A3CE0C, `_stricmp`) is
/// detached in place (vt+0x8C) and released, answering true; one named `FadeNode ` (`_strnicmp`, 9) becomes the node
/// searched from its slot 0 when it is a node (vt+8), else false. Completion: 100%.
pub(crate) fn detach_scabbard(world: &mut World, node: Entity) -> bool {
    use crate::gamebryo::scene_node::NiNodeChildren;
    let mut current = node;
    'walk: loop {
        let children = world
            .get::<NiNodeChildren>(current)
            .map(|children| children.slots_ac.clone())
            .unwrap_or_default();
        for (index, child) in children.into_iter().enumerate() {
            let Some(child) = child else { continue };
            let Some(name) = world
                .get::<NiObjectNet>(child)
                .and_then(|net| net.name.clone())
            else {
                continue;
            };
            if oblivion_core::crt::stricmp(&name, b"Scb") == 0 {
                crate::gamebryo::scene_node::set_child_at(world, current, index, None);
                return true;
            }
            if oblivion_core::crt::strnicmp(&name, b"FadeNode ", 9) == 0 {
                if world.get::<NiNodeChildren>(child).is_none() {
                    return false;
                }
                current = child;
                continue 'walk;
            }
        }
        return false;
    }
}

// IDA: 00405790 NiNode child slot `index` (below +0xB6, else null). Completion: 100%.
pub(crate) fn node_child(world: &World, node: Entity, index: usize) -> Option<Entity> {
    world
        .get::<NiNodeChildren>(node)
        .and_then(|children| children.slots_ac.get(index).copied().flatten())
}

/// IDA: N/A the stop of 0070AA20's walk at an object whose class has no host vtable +0x5C arm, or no host state.
const NO_SELECTIVE_UPDATE_ARM: GameError = GameError::unclassified(
    0x0070_AA20,
    "0070AA20: an object with no host SetSelectiveUpdateFlags arm",
);

// IDA: 0070AA20 child loop: vtable +0x5C per nonnull slot. NiNode and NiBillboardNode use 0070AA20, geometry 00722F00, NiParticleSystem
// 00749360 (selective, rigid false, 0x02 | 0x0C set, 0x10 cleared; `transforms` and the controllers are ignored) and NiDirectionalLight and
// NiCamera (A7E4C4) the leaf 00707D80.
// Completion: 100% of the verified per-class dispatch for the bound child classes.
// TODO: [ ] Leaf controller-chain queries once leaf heads link.
fn set_child_selective_update_flags(
    world: &mut World,
    child: Entity,
    selective: &mut bool,
    transforms: bool,
    rigid: &mut bool,
) -> Result<(), GameError> {
    match world.get::<NiClass>(child).copied() {
        // NiBillboardNode vtable A45134 +0x5C is also 0070AA20.
        Some(NiClass::Node | NiClass::BillboardNode) => {
            set_selective_update_flags(world, child, selective, transforms, rigid)
        }
        Some(NiClass::TriShape) => {
            let skinned = world
                .get::<NiGeometry>(child)
                .is_some_and(|geometry| geometry.skin_b8.is_some());
            set_geometry_selective_update_flags(world, child, skinned, selective, transforms, rigid)
        }
        // NiTriStrips links only with a null +0xB8 skin instance.
        Some(NiClass::TriStrips) => {
            set_geometry_selective_update_flags(world, child, false, selective, transforms, rigid)
        }
        Some(NiClass::ParticleSystem) => {
            let mut system = world
                .get_mut::<ParticleSystem>(child)
                .ok_or(NO_SELECTIVE_UPDATE_ARM)?;
            *selective = true;
            *rigid = false;
            system.effective_av_flags_18 |= 0x0002;
            system.effective_av_flags_18 |= 0x000C;
            system.effective_av_flags_18 &= 0xFFEF;
            Ok(())
        }
        Some(NiClass::DirectionalLight) => {
            let property_controller =
                av_has_property_controller(world, av_properties(world, child));
            let mut light = world
                .get_mut::<DirectionalLight>(child)
                .ok_or(NO_SELECTIVE_UPDATE_ARM)?;
            set_leaf_selective_update_flags(
                &mut light.payload.av.effective_av_flags,
                selective,
                transforms,
                rigid,
                false,
                false,
                property_controller,
            );
            Ok(())
        }
        Some(NiClass::Camera) => {
            let property_controller =
                av_has_property_controller(world, av_properties(world, child));
            let mut camera = world
                .get_mut::<SceneCamera>(child)
                .ok_or(NO_SELECTIVE_UPDATE_ARM)?;
            set_leaf_selective_update_flags(
                &mut camera.av_flags_18,
                selective,
                transforms,
                rigid,
                false,
                false,
                property_controller,
            );
            Ok(())
        }
        _ => Err(NO_SELECTIVE_UPDATE_ARM),
    }
}

// IDA: 0070AA20 NiNode::SetSelectiveUpdateFlags(bool& selective, bool transforms, bool& rigid), also used by NiBillboardNode. Children
// dispatch through vtable +0x5C; 007077A0 reads the node's +0x98 property list; 00707770 is any chain controller whose vtable +0x5C
// is true, and only NiTransformController (977C50) answers true among the bound classes.
// Completion: 100% of the verified flag order on the +0x18 live word.
// TODO: [ ] Property controller classes beyond NiTexturingProperty.
pub fn set_selective_update_flags(
    world: &mut World,
    node: Entity,
    selective: &mut bool,
    mut transforms: bool,
    rigid: &mut bool,
) -> Result<(), GameError> {
    let transform_controlled = get_controller(world, node, NiClass::TransformController).is_some();
    if !transforms {
        transforms = transform_controlled;
    }
    *selective = transforms;
    *rigid = true;
    let children = world
        .get::<NiNodeChildren>(node)
        .map_or_else(Vec::new, |children| children.slots_ac.clone());
    for child in children.into_iter().flatten() {
        let mut child_selective = false;
        let mut child_rigid = true;
        set_child_selective_update_flags(
            world,
            child,
            &mut child_selective,
            transforms,
            &mut child_rigid,
        )?;
        if child_selective {
            *selective = true;
        }
        if !child_rigid {
            *rigid = false;
        }
    }
    let property_controller = av_has_property_controller(world, av_properties(world, node));
    let has_head = is_time_controller(world, controller_head(world, node));
    let mut payload = world
        .get_mut::<SceneNodePayload>(node)
        .ok_or(NO_SELECTIVE_UPDATE_ARM)?;
    let flags = &mut payload.effective_av_flags;
    if property_controller || has_head {
        *selective = true;
        *flags |= 0x0002;
    } else if *selective {
        *flags |= 0x0002;
    } else {
        *flags &= 0xFFFD;
    }
    *flags = if transforms {
        *flags | 0x0004
    } else {
        *flags & 0xFFFB
    };
    *flags = if property_controller {
        *flags | 0x0008
    } else {
        *flags & 0xFFF7
    };
    *flags = if *rigid {
        *flags | 0x0010
    } else {
        *flags & 0xFFEF
    };
    if transform_controlled {
        *rigid = false;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut u32_field = |value: u32| bytes.extend(value.to_le_bytes());
        u32_field(0); // empty name
        u32_field(0); // no extras
        u32_field(NULL_REFERENCE); // null controller
        bytes.extend(0x0040u16.to_le_bytes()); // bit 0x40 is removed after the lists
        let mut u32_field = |value: u32| bytes.extend(value.to_le_bytes());
        for index in 0..3 {
            u32_field(if index == 0 { 0x4000_0000 } else { 0 }); // translation (2, 0, 0)
        }
        for index in 0..9 {
            u32_field(if index % 4 == 0 { 0x3F80_0000 } else { 0 });
        }
        u32_field(0x3F80_0000); // scale 1
        u32_field(0); // no properties
        u32_field(NULL_REFERENCE); // null collision
        u32_field(3); // three child slots, one null
        u32_field(1);
        u32_field(NULL_REFERENCE);
        u32_field(2);
        u32_field(1); // one null effect
        u32_field(NULL_REFERENCE);
        bytes
    }

    /// Ported from `gamebryo_scene_node`.
    #[test]
    fn payload_versions_and_truncation() {
        let bytes = node_bytes();
        // 10.0.1.0's 0x3F mask leaves the same 0x0E here.
        for version in [0x0A00_0100, 0x0A01_0072, 0x1400_0004] {
            let parsed = parse_scene_node_payload(&bytes, version).unwrap();
            assert_eq!(parsed.bytes_consumed, bytes.len());
            assert_eq!(parsed.raw_av_flags, 0x40);
            assert_eq!(parsed.effective_av_flags, 0x0E);
            assert_eq!(parsed.local_transform.translation[0], 2.0);
            assert_eq!(parsed.child_indices, [1, NULL_REFERENCE, 2]);
            assert_eq!(parsed.effect_indices, [NULL_REFERENCE]);
        }
        assert!(parse_scene_node_payload(&bytes, 0x0A00_00FF).is_err());
        assert!(parse_scene_node_payload(&bytes[..bytes.len() - 1], 0x1400_0004).is_err());
    }

    #[test]
    fn children_parents_and_the_world_callback() {
        let mut world = World::new();
        let parent = spawn_scene_node(
            &mut world,
            parse_scene_node_payload(&node_bytes(), 0x1400_0004).unwrap(),
        );
        let first = spawn_scene_node(&mut world, SceneNodePayload::default());
        let last = spawn_scene_node(&mut world, SceneNodePayload::default());
        let status =
            link_scene_node_children(&mut world, parent, &[Some(first), None, Some(last)], 3)
                .unwrap();
        assert_eq!(status, SceneNodeLinkStatus::Linked);
        assert_eq!(
            world.get::<NiNodeChildren>(parent).unwrap().slots_ac,
            [Some(first), None, Some(last)]
        );
        assert_eq!(world.get::<NiAvParent>(first).unwrap().0, Some(parent));
        assert_eq!(world.get::<NiAvParent>(last).unwrap().0, Some(parent));

        {
            let mut parent_world = world.get_mut::<NiAvWorld>(parent).unwrap();
            parent_world.transform.translation = [10.0, 20.0, 30.0];
            parent_world.transform.scale = 2.0;
        }
        {
            let mut payload = world.get_mut::<SceneNodePayload>(first).unwrap();
            payload.local_transform.translation = [1.0, 2.0, 3.0];
            payload.local_transform.scale = 3.0;
            payload.collision_index = 1;
        }
        assert!(
            !update_world_transform(&mut world, first, None).unwrap(),
            "no collision callback"
        );
        let stored = world.get::<NiAvWorld>(first).unwrap().transform;
        assert_eq!(
            stored.translation,
            [12.0, 24.0, 36.0],
            "the store precedes the missing callback"
        );
        assert_eq!(stored.scale, 6.0);
        let mut called = false;
        let mut callback = |world: &mut World| {
            called = true;
            assert_eq!(
                world.get::<NiAvWorld>(first).unwrap().transform.translation[0],
                12.0
            );
            assert_eq!(world.get::<NiAvWorld>(first).unwrap().transform.scale, 6.0);
        };
        assert!(update_world_transform(&mut world, first, Some(&mut callback)).unwrap());
        assert!(called);
        assert!(update_world_transform(&mut world, last, None).unwrap());
        assert_eq!(
            world.get::<NiAvWorld>(last).unwrap().transform.translation,
            [10.0, 20.0, 30.0]
        );
    }

    /// The full link: properties, effects and rejection, on the bound classes.
    #[test]
    fn full_link_publishes_in_order_or_not_at_all() {
        let mut world = World::new();
        let property = world.spawn(NiClass::AlphaProperty).id();
        let light = world.spawn(NiClass::DirectionalLight).id();
        let child = spawn_scene_node(&mut world, SceneNodePayload::default());
        let mut table = StreamObjectTable::default();
        table.push("NiAlphaProperty", 0, property);
        table.push("NiNode", 0, child);
        table.push("NiDirectionalLight", 0, light);
        let payload = SceneNodePayload {
            property_indices: vec![0],
            child_indices: vec![1],
            effect_indices: vec![2],
            ..SceneNodePayload::default()
        };
        let node = spawn_scene_node(&mut world, payload);
        let status =
            link_scene_node(&mut world, node, &[Some(child)], &[Some(light)], &table).unwrap();
        assert_eq!(status, SceneNodeLinkStatus::Linked);
        assert_eq!(
            world
                .get::<super::super::av_object::NiAvProperties>(node)
                .unwrap()
                .0,
            [Some(property)]
        );
        assert_eq!(world.get::<NiNodeEffects>(node).unwrap().0, [light]);
        assert_eq!(world.get::<NiEffectAffectedNodes>(light).unwrap().0, [node]);
        assert_eq!(
            link_scene_node(&mut world, node, &[Some(child)], &[Some(light)], &table).unwrap(),
            SceneNodeLinkStatus::UnsupportedOwner,
            "a relink needs the unported detach"
        );

        // A child with a parent leaves a second node untouched.
        let payload = SceneNodePayload {
            property_indices: vec![0],
            child_indices: vec![1],
            ..SceneNodePayload::default()
        };
        let other = spawn_scene_node(&mut world, payload);
        assert_eq!(
            link_scene_node(&mut world, other, &[Some(child)], &[], &table).unwrap(),
            SceneNodeLinkStatus::UnsupportedOwner
        );
        assert!(
            world
                .get::<super::super::av_object::NiAvProperties>(other)
                .is_none()
        );
        assert!(
            world
                .get::<NiNodeChildren>(other)
                .unwrap()
                .slots_ac
                .is_empty()
        );
    }

    #[test]
    fn transforms_compose_and_invert() {
        use super::super::transform::{invert_scene_transform, resolve_scene_relative_transform};
        let parent = NiTransform {
            translation: [10.0, 20.0, 30.0],
            scale: 2.0,
            ..NiTransform::default()
        };
        let local = NiTransform {
            translation: [1.0, 2.0, 3.0],
            scale: 3.0,
            ..NiTransform::default()
        };
        let composed = compose_scene_transform(&parent, &local);
        assert_eq!(composed.translation, [12.0, 24.0, 36.0]);
        assert_eq!(composed.scale, 6.0);
        let inverse = invert_scene_transform(&parent);
        assert_eq!(inverse.scale, 0.5);
        assert_eq!(inverse.translation, [-5.0, -10.0, -15.0]);
        let relative = resolve_scene_relative_transform(&composed, &parent);
        assert_eq!(relative.translation, [1.0, 2.0, 3.0]);
        assert_eq!(relative.scale, 3.0);
    }
}
