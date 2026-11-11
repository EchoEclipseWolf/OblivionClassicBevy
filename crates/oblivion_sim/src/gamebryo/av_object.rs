//! NiAVObject's links and flags. Component offsets name the original object.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;

use super::mesh_geometry::NiGeometry;
use super::object_net::{
    NiObjectNet, ObjectNetPending, object_net_extras_supported, parse_object_net_payload,
    resolve_object_net_extra_links,
};
use super::scene_node::{SceneNodePayload, child_slots};
use super::scene_state::property_type;
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable, check_link_index};
use super::transform::NiTransform;
use crate::gamebryo::scene_state::{
    ScenePropertyDefaults, refresh_effect_state, refresh_property_state,
};
use crate::gamebryo::scene_update::{SceneUpdateRuntime, SceneUpdateStatus, update_scene_object};

/// IDA: 007081B0's gates: properties AddHead at >= 04010008 (AddTail below), collision +0xA8 at >= 05000013.
pub const AV_PROPERTY_HEAD_INSERT_VERSION: u32 = 0x0401_0008;
pub const AV_COLLISION_LINK_VERSION: u32 = 0x0500_0013;

/// IDA: the +0x98 NiTPointerList<NiProperty*>, head first. A null stream reference is still a node.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct NiAvProperties(pub Vec<Option<Entity>>);

/// IDA: NiAVObject +0xA8, 007082FA: the collision object.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct NiAvCollision(pub Entity);

/// IDA: 007074B0 NiAVObject_SetParent's raw +0x1C parent.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NiAvParent(pub Option<Entity>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvObjectLinkStatus {
    Linked,
    /// A referenced owner class is not bound in the host.
    UnsupportedOwner,
    /// IDA: 007082FA assigns +0xA8. The collision classes register in a later phase (0088B070).
    LaterPhaseCollision,
}

/// IDA: 00707F00's modern NiAVObject prefix, shared by NiNode (00709EE0) and NiGeometry (007234B0).
#[derive(Clone, Debug, PartialEq)]
pub struct AvObjectPrefix {
    pub net: ObjectNetPending,
    pub raw_av_flags: u16,
    pub effective_av_flags: u16,
    pub local_transform: NiTransform,
    pub property_indices: Vec<u32>,
    pub collision_index: u32,
}

/// IDA: 00712AE0 reads a u32 count, then exactly that many u32 deferred references; a count past the stream is the
/// caller's `list_truncated`. Completion: 100% of the raw order.
/// TODO: [ ] NiStream's deferred-reference queue side effects.
pub fn read_av_reference_list(
    reader: &mut ByteReader<'_>,
    truncated: &'static str,
    list_truncated: GameError,
) -> Result<Vec<u32>, GameError> {
    let count = reader.u32(truncated)?;
    if count as usize > reader.remaining() / 4 {
        return Err(list_truncated);
    }
    let mut result = Vec::with_capacity(count as usize);
    for _ in 0..count {
        result.push(reader.u32(truncated)?);
    }
    Ok(result)
}

/// IDA: 00707F00's prefix from 0A000100: NiObjectNET, the u16 flags, translation, rotation, scale, the property list
/// and the collision reference. Then below 0A000102 the raw flags go to stream+300 and the flags keep only 0x3F, below
/// 14000004 bit 0x40 is cleared, and always (f & FFE1) | 000E. The reader is left at the class's own suffix. The error
/// texts are the caller's, which name its class. Completion: 100% of the >= 0A000100 widths and order.
/// TODO: [ ] The < 05000013 collision-data branch; stream+300 for the NiNode subclasses that read it (007247D0,
/// 00722510 NiBillboardNode_LoadBinary); below 0A000106 a collision object's vt call (a host stop here).
pub fn parse_av_object_prefix<'a>(
    bytes: &'a [u8],
    stream_version: u32,
    truncated: &'static str,
    list_truncated: &'static str,
) -> Result<(AvObjectPrefix, ByteReader<'a>), GameError> {
    let net = parse_object_net_payload(bytes, stream_version)?;
    let mut reader = ByteReader::new(&bytes[net.bytes_consumed..]);
    if stream_version < 0x0A00_0100 {
        return Err(GameError::unclassified(
            0,
            "legacy NiAVObject payload unsupported",
        ));
    }
    let raw_av_flags = reader.u16(truncated)?;
    let mut local_transform = NiTransform::default();
    for component in &mut local_transform.translation {
        *component = reader.f32(truncated)?;
    }
    for component in &mut local_transform.rotation {
        *component = reader.f32(truncated)?;
    }
    local_transform.scale = reader.f32(truncated)?;
    let property_indices = read_av_reference_list(
        &mut reader,
        truncated,
        GameError::from_legacy_message(list_truncated),
    )?;
    let collision_index = reader.u32(truncated)?;
    let mut effective_av_flags = raw_av_flags;
    if stream_version < 0x0A00_0102 {
        effective_av_flags &= 0x3F;
    }
    if stream_version < 0x0A00_0106 && collision_index != NULL_REFERENCE {
        return Err(GameError::unclassified(
            0,
            "NiAVObject collision object below 0A000106 unsupported",
        ));
    }
    if stream_version < 0x1400_0004 {
        effective_av_flags &= !0x40;
    }
    effective_av_flags = (effective_av_flags & 0xFFE1) | 0x000E;
    Ok((
        AvObjectPrefix {
            net,
            raw_av_flags,
            effective_av_flags,
            local_transform,
            property_indices,
            collision_index,
        },
        reader,
    ))
}

/// IDA: N/A the staged result of the link preflight: properties in their final list order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvObjectLinkPlan {
    pub status: AvObjectLinkStatus,
    pub properties: Vec<Option<Entity>>,
    /// +0xA8.
    pub collision: Option<Entity>,
}

/// IDA: 007081B0's preflight: 00700050's extra owners, each +0x98 reference, then +0xA8. Modern streams
/// AddHead (reversing raw order), older ones AddTail; null references stay as null nodes. Nothing is
/// changed, so a later sibling failure cannot leave partial state. Completion: 100% of the order.
/// TODO: [ ] Refcount timing.
pub fn plan_av_object_links(
    world: &World,
    net: &ObjectNetPending,
    properties: &[u32],
    collision_index: u32,
    table: &StreamObjectTable,
    stream_version: u32,
) -> Result<AvObjectLinkPlan, GameError> {
    let count = table.objects.len();
    for &index in &net.extra_indices {
        check_link_index(
            index,
            count,
            GameError::from_legacy_message("NiAVObject extra reference out of range"),
        )?;
    }
    check_link_index(
        net.controller_index,
        count,
        GameError::from_legacy_message("NiAVObject controller reference out of range"),
    )?;
    for &index in properties {
        check_link_index(
            index,
            count,
            GameError::from_legacy_message("NiAVObject property reference out of range"),
        )?;
    }
    check_link_index(
        collision_index,
        count,
        GameError::from_legacy_message("NiAVObject collision reference out of range"),
    )?;

    let refused = |status| AvObjectLinkPlan {
        status,
        properties: Vec::new(),
        collision: None,
    };
    if !object_net_extras_supported(world, net, table) {
        return Ok(refused(AvObjectLinkStatus::UnsupportedOwner));
    }
    let head_insert = stream_version >= AV_PROPERTY_HEAD_INSERT_VERSION;
    let mut planned = Vec::with_capacity(properties.len());
    for &index in properties {
        let mut owner = None;
        if index != NULL_REFERENCE {
            let entity = table.objects[index as usize].entity;
            if !world
                .get::<NiClass>(entity)
                .is_some_and(|class| class.is_property())
            {
                return Ok(refused(AvObjectLinkStatus::UnsupportedOwner));
            }
            owner = Some(entity);
        }
        if head_insert {
            planned.insert(0, owner);
        } else {
            planned.push(owner);
        }
    }
    // 007082DE: the collision reference is only consumed at >= 05000013.
    let mut collision = None;
    if stream_version >= AV_COLLISION_LINK_VERSION && collision_index != NULL_REFERENCE {
        let entity = table.objects[collision_index as usize].entity;
        // The other collision classes aren't bound (`bs::havok_stream`).
        if !matches!(
            world.get::<NiClass>(entity),
            Some(
                NiClass::BhkSpCollisionObject
                    | NiClass::BhkCollisionObject
                    | NiClass::BhkBlendCollisionObject
            )
        ) {
            return Ok(refused(AvObjectLinkStatus::LaterPhaseCollision));
        }
        collision = Some(entity);
    }
    Ok(AvObjectLinkPlan {
        status: AvObjectLinkStatus::Linked,
        properties: planned,
        collision,
    })
}

/// IDA: 007081E0's extras and names, then the property inserts, on a `Linked` plan.
pub fn commit_av_object_links(
    world: &mut World,
    owner: Entity,
    net: &ObjectNetPending,
    plan: AvObjectLinkPlan,
    table: &StreamObjectTable,
) -> Result<(), GameError> {
    assert_eq!(
        plan.status,
        AvObjectLinkStatus::Linked,
        "the NiAVObject link plan is not linkable"
    );
    let resolved = resolve_object_net_extra_links(world, net, table)?;
    let controller_0c = world
        .get::<NiObjectNet>(owner)
        .and_then(|existing| existing.controller_0c);
    world.entity_mut(owner).insert((
        NiObjectNet {
            controller_0c,
            ..resolved
        },
        NiAvProperties(plan.properties),
    ));
    if let Some(collision) = plan.collision {
        world.entity_mut(owner).insert(NiAvCollision(collision));
    }
    Ok(())
}

/// IDA: N/A the +0x98 property list of an object, head first; an object with no list has none.
#[must_use]
pub(crate) fn av_properties(world: &World, owner: Entity) -> &[Option<Entity>] {
    world
        .get::<NiAvProperties>(owner)
        .map_or(&[], |properties| properties.0.as_slice())
}

/// IDA: 007077A0 NiAVObject_HasPropertyController: from the head, the first nonnull property with a
/// +0x0C controller head. Completion: 100% of the order.
#[must_use]
pub fn av_has_property_controller(world: &World, properties: &[Option<Entity>]) -> bool {
    properties.iter().flatten().any(|&property| {
        world
            .get::<NiObjectNet>(property)
            .is_some_and(|net| net.controller_0c.is_some())
    })
}

/// IDA: 00707D80 NiAVObject_SetSelectiveUpdateFlags on a leaf: rigid = !transform_controller; transforms |=
/// it; a property controller forces selective and sets 0x18; otherwise 0x08 clears and 0x10 sets. Without
/// transforms, a property controller or a +0x0C controller, selective and 0x02 / 0x04 clear. Completion:
/// 100% of the branch and store order.
pub fn set_leaf_selective_update_flags(
    flags: &mut u16,
    selective: &mut bool,
    mut transforms: bool,
    rigid: &mut bool,
    transform_controller: bool,
    has_controller: bool,
    property_controller: bool,
) {
    *rigid = !transform_controller;
    if !transforms {
        transforms = transform_controller;
    }
    if !property_controller && !transforms && !has_controller {
        *selective = false;
        *flags &= !0x0002;
        *flags &= !0x0004;
    } else {
        *selective = true;
        *flags |= 0x0002;
        if transforms {
            *flags |= 0x0004;
        } else {
            *flags &= !0x0004;
        }
    }
    if property_controller {
        *flags |= 0x0018;
    } else {
        *flags &= !0x0008;
        *flags |= 0x0010;
    }
}

// IDA: 007073F0 NiAVObject_GetObjectByName: the `strcmp` of NiAVObject vt+0x58: a null name argument or object name is null. Completion: 100%.
pub(crate) fn object_named(world: &World, object: Entity, name: &[u8]) -> bool {
    world
        .get::<NiObjectNet>(object)
        .and_then(|net| net.name.as_deref())
        .is_some_and(|own| own == name)
}

// IDA: 00707E40 NiAVObject_SetCollisionObject(null): +0xA8 released (the host's link and its stream index).
pub(crate) fn clear_collision(world: &mut World, object: Entity) {
    world.entity_mut(object).remove::<NiAvCollision>();
    if let Some(mut payload) = world.get_mut::<SceneNodePayload>(object) {
        payload.collision_index = crate::gamebryo::stream_objects::NULL_REFERENCE;
    }
}

// IDA: 00442F70 NiAVObject_DeepCopyGeometryData. Visit children from the last slot down.
// Completion: the geometry-data copy and vt+0x8C setter on the supported geometry classes.
// Host substitution: copying the loaded data component replaces the 00700790 stream round trip.
pub(crate) fn deep_copy_geometry_data(world: &mut World, object: Entity) {
    if let Some(geometry) = world.get::<NiGeometry>(object) {
        if let Some(data) = geometry.data_b4 {
            let copy = world.entity_mut(data).clone_and_spawn();
            world.get_mut::<NiGeometry>(object).unwrap().data_b4 = Some(copy);
        }
    } else {
        for child in child_slots(world, object).into_iter().rev().flatten() {
            deep_copy_geometry_data(world, child);
        }
    }
}

// IDA: N/A the entries of the +0x98 list, head first.
fn local_properties(world: &World, object: Entity) -> Vec<Option<Entity>> {
    world
        .get::<NiAvProperties>(object)
        .map(|list| list.0.clone())
        .unwrap_or_default()
}

// IDA: 00707530 NiAVObject_GetPropertyOfType(object, type): null for type >= 10 or an empty list; otherwise the first nonnull +0x98 entry from
// the +0x9C head whose vtable +0x4C type equals `type`. Completion: 100%.
pub fn find_property_of_type(
    world: &World,
    object: Entity,
    kind: u32,
) -> Result<Option<Entity>, GameError> {
    if kind >= 10 {
        return Ok(None);
    }
    for property in local_properties(world, object).into_iter().flatten() {
        if property_type(world, Some(property))? == kind {
            return Ok(Some(property));
        }
    }
    Ok(None)
}

// IDA: 00480820(object): a NiGeomMorpherController in the object's chain (NiObjectNET_GetController, 00B3CA90's RTTI), else a
// node's children in slot order, the first true answering. Completion: 100%.
pub(crate) fn has_geom_morpher(world: &World, object: Entity) -> bool {
    if crate::gamebryo::object_net::get_controller(
        world,
        object,
        crate::gamebryo::stream_objects::NiClass::GeomMorpherController,
    )
    .is_some()
    {
        return true;
    }
    world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(object)
        .is_some_and(|children| {
            children
                .slots_ac
                .iter()
                .flatten()
                .any(|&child| has_geom_morpher(world, child))
        })
}

/// IDA: 004DE3C0(object, value): with an object, each controller from +0x0C by +0x34 takes +0x10 (the phase) = value, then
/// the same for the chain of its texturing property (00707530 type 6), then with vt+8 (a node) each child slot recurses
/// (a null slot returns at once). Completion: 100%; a chain entry with no host time state is a stop.
pub(crate) fn set_controller_phases(
    world: &mut World,
    object: Option<Entity>,
    value: f32,
) -> Result<(), GameError> {
    use crate::gamebryo::controller_time::controller_next;
    use crate::gamebryo::particle_controller::controller_time_mut;
    let Some(object) = object else {
        return Ok(());
    };
    let mut owners = vec![object];
    if let Some(property) = crate::gamebryo::av_object::find_property_of_type(world, object, 6)? {
        owners.push(property);
    }
    for owner in owners {
        let mut cursor = world
            .get::<NiObjectNet>(owner)
            .and_then(|net| net.controller_0c);
        while let Some(controller) = cursor {
            controller_time_mut(world, controller)
                .ok_or(GameError::unclassified(
                    0x004DE3D8,
                    "004DE3D8: a controller with no host time state",
                ))?
                .fields
                .phase = value;
            cursor = controller_next(world, controller);
        }
    }
    let children = world
        .get::<crate::gamebryo::scene_node::NiNodeChildren>(object)
        .map(|children| children.slots_ac.clone());
    for child in children.unwrap_or_default() {
        set_controller_phases(world, child, value)?;
    }
    Ok(())
}

/// IDA: 0047CA30 NiAVObject_UpdateMultiTargetControllers(time, accum) on the anim data's root: each controller of the root's
/// +0x0C chain (+0x34 next) whose RTTI is NiMultiTargetTransformController runs `0047C740`, Oblivion's copy of `006CF9D0`
/// (the flag 0x08 gate, the shared invalid seed, each target with +0x18 bit 0x02 sampled into its local translate, rotation
/// and scale), with the accum argument unused. Completion: 100%; the refcount pairs around each target are the host's.
/// # Errors
/// A sample the port's blend can't make.
pub fn anim_data_update_multi_targets(
    world: &mut World,
    root: Entity,
    time: f32,
) -> Result<(), GameError> {
    use crate::gamebryo::transform_controller::{NodeController, NodeControllerUpdateStatus};
    let mut next = world
        .get::<crate::gamebryo::object_net::NiObjectNet>(root)
        .and_then(|net| net.controller_0c);
    while let Some(controller) = next {
        if world
            .get::<crate::gamebryo::stream_objects::NiClass>(controller)
            .copied()
            == Some(crate::gamebryo::stream_objects::NiClass::MultiTargetTransformController)
        {
            let status =
                crate::gamebryo::transform_controller::update_multi_target_transform_controller(
                    world, controller, time,
                )?;
            if !matches!(
                status,
                NodeControllerUpdateStatus::Applied | NodeControllerUpdateStatus::Skipped
            ) {
                return Err(GameError::unclassified(
                    0x0047C823,
                    "0047C823: 0047C740's blend sample",
                ));
            }
        }
        next = world
            .get::<NodeController>(controller)
            .and_then(|node| node.time.next_34);
    }
    Ok(())
}

// IDA: 004A0E50 NiTPointerList remove of the first node holding the property (+0x98 list), used by 00708560 and 004A1220. Completion: 100% of
// the host list.
pub(crate) fn remove_property(world: &mut World, object: Entity, property: Entity) {
    if let Some(mut list) = world.get_mut::<NiAvProperties>(object)
        && let Some(index) = list.0.iter().position(|&entry| entry == Some(property))
    {
        list.0.remove(index);
    }
}

// IDA: 00708560 NiAVObject_DetachPropertyOfType(object, out, type): for type < 10, the first nonnull +0x98 entry of that type is removed (004A0E50)
// and returned; null otherwise. Completion: 100%. The caller's release is not modelled: properties have no count, and one can be shared.
pub(crate) fn detach_property_of_type(
    world: &mut World,
    object: Entity,
    kind: u32,
) -> Result<Option<Entity>, GameError> {
    let found = find_property_of_type(world, object, kind)?;
    if let Some(property) = found {
        remove_property(world, object, property);
    }
    Ok(found)
}

// IDA: 0040D0E0 NiAVObject_DetachPropertyOfType(type): the first property of that type leaves +0x98.
pub(crate) fn detach_first_property_of_type(
    world: &mut World,
    object: Entity,
    kind: u32,
) -> Result<(), GameError> {
    let properties = world
        .get::<NiAvProperties>(object)
        .map(|list| list.0.clone())
        .unwrap_or_default();
    for (index, property) in properties.iter().enumerate() {
        if property_type(world, *property)? == kind {
            world
                .get_mut::<NiAvProperties>(object)
                .expect("read above")
                .0
                .remove(index);
            return Ok(());
        }
    }
    Ok(())
}

// IDA: 0040D0C0 NiAVObject_GetPropertyOfType(type).
pub(crate) fn property_of_type(
    world: &World,
    object: Entity,
    kind: u32,
) -> Result<Option<Entity>, GameError> {
    let properties = world
        .get::<NiAvProperties>(object)
        .map(|list| list.0.clone())
        .unwrap_or_default();
    for property in properties.into_iter().flatten() {
        if property_type(world, Some(property))? == kind {
            return Ok(Some(property));
        }
    }
    Ok(None)
}

// IDA: 00707610, 00707980 and NiAVObject_Update(0.0, 1) on one tree (0040640B..00406458). Completion: 100% of the order.
pub(crate) fn refresh_and_update(
    world: &mut World,
    object: Entity,
    defaults: &ScenePropertyDefaults,
    update: &mut dyn SceneUpdateRuntime,
    site: &'static str,
) -> Result<(), GameError> {
    refresh_property_state(world, object, defaults)?;
    refresh_effect_state(world, object)?;
    let updated = update_scene_object(world, object, 0.0, true, update)?;
    if updated.status != SceneUpdateStatus::Updated {
        return Err(GameError::from_legacy_message(site));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property(world: &mut World) -> Entity {
        world.spawn(NiClass::AlphaProperty).id()
    }

    #[test]
    fn property_order_follows_the_version_gate() {
        let mut world = World::new();
        let (a, b) = (property(&mut world), property(&mut world));
        let mut table = StreamObjectTable::default();
        table.push("NiAlphaProperty", 0, a);
        table.push("NiAlphaProperty", 0, b);
        let net = ObjectNetPending::default();
        let raw = [0, NULL_REFERENCE, 1];
        let modern =
            plan_av_object_links(&world, &net, &raw, NULL_REFERENCE, &table, 0x1400_0004).unwrap();
        assert_eq!(modern.status, AvObjectLinkStatus::Linked);
        assert_eq!(
            modern.properties,
            [Some(b), None, Some(a)],
            "AddHead reverses raw order"
        );
        let old = plan_av_object_links(&world, &net, &raw, 0, &table, 0x0400_0002).unwrap();
        assert_eq!(
            old.properties,
            [Some(a), None, Some(b)],
            "AddTail keeps it, and no collision is read"
        );
    }

    #[test]
    fn preflight_rejects_before_any_change() {
        let mut world = World::new();
        let data = world.spawn(NiClass::FloatKeyData).id();
        let mut table = StreamObjectTable::default();
        table.push("NiFloatData", 0, data);
        let net = ObjectNetPending::default();
        let wrong =
            plan_av_object_links(&world, &net, &[0], NULL_REFERENCE, &table, 0x1400_0004).unwrap();
        assert_eq!(wrong.status, AvObjectLinkStatus::UnsupportedOwner);
        let collision = plan_av_object_links(&world, &net, &[], 0, &table, 0x1400_0004).unwrap();
        assert_eq!(collision.status, AvObjectLinkStatus::LaterPhaseCollision);
        assert!(
            plan_av_object_links(&world, &net, &[5], NULL_REFERENCE, &table, 0x1400_0004).is_err()
        );
    }

    #[test]
    fn property_controller_scan_skips_null_nodes() {
        let mut world = World::new();
        let plain = world.spawn(NiObjectNet::default()).id();
        assert!(!av_has_property_controller(&world, &[None, Some(plain)]));
        let head = world.spawn_empty().id();
        let controlled = world
            .spawn(NiObjectNet {
                controller_0c: Some(head),
                ..NiObjectNet::default()
            })
            .id();
        assert!(av_has_property_controller(
            &world,
            &[None, Some(plain), Some(controlled)]
        ));
    }

    #[test]
    fn leaf_flags_follow_00707d80() {
        let (mut flags, mut selective, mut rigid) = (0x0008u16, true, false);
        set_leaf_selective_update_flags(
            &mut flags,
            &mut selective,
            false,
            &mut rigid,
            false,
            false,
            false,
        );
        assert_eq!((flags, selective, rigid), (0x0010, false, true));
        let (mut flags, mut selective, mut rigid) = (0u16, false, true);
        set_leaf_selective_update_flags(
            &mut flags,
            &mut selective,
            false,
            &mut rigid,
            true,
            false,
            false,
        );
        assert_eq!((flags, selective, rigid), (0x0016, true, false));
        let (mut flags, mut selective, mut rigid) = (0x0004u16, false, false);
        set_leaf_selective_update_flags(
            &mut flags,
            &mut selective,
            false,
            &mut rigid,
            false,
            true,
            true,
        );
        assert_eq!((flags, selective), (0x001A, true));
    }
}
