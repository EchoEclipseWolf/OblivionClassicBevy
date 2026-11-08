//! NiAVObject's links and flags. Component offsets name the original object.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::object_net::{
    NiObjectNet, ObjectNetPending, object_net_extras_supported, parse_object_net_payload,
    resolve_object_net_extra_links,
};
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};
use super::transform::NiTransform;

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

/// IDA: 00712AE0 reads a u32 count, then exactly that many u32 deferred references. Completion: 100% of the raw order.
/// TODO: [ ] NiStream's deferred-reference queue side effects.
pub fn read_av_reference_list(
    reader: &mut ByteReader<'_>,
    truncated: &'static str,
    list_truncated: &'static str,
) -> Result<Vec<u32>, StreamError> {
    let count = reader.u32(truncated)?;
    if count as usize > reader.remaining() / 4 {
        return Err(StreamError(list_truncated));
    }
    let mut result = Vec::with_capacity(count as usize);
    for _ in 0..count {
        result.push(reader.u32(truncated)?);
    }
    Ok(result)
}

/// IDA: 00707F00's modern prefix: NiObjectNET, the u16 flags (bit 0x40 cleared below 14000004, then (f & FFE1) | 000E),
/// translation, rotation, scale, the property list and the collision reference. The reader is left at the class's own
/// suffix. The error texts are the caller's, which name its class. Completion: 100% of the >= 0A010072 widths and order.
/// TODO: [ ] The legacy flag and collision remapping.
pub fn parse_av_object_prefix<'a>(
    bytes: &'a [u8],
    stream_version: u32,
    truncated: &'static str,
    list_truncated: &'static str,
) -> Result<(AvObjectPrefix, ByteReader<'a>), StreamError> {
    let net = parse_object_net_payload(bytes, stream_version)?;
    let mut reader = ByteReader::new(&bytes[net.bytes_consumed..]);
    let raw_av_flags = reader.u16(truncated)?;
    let mut effective_av_flags = raw_av_flags;
    if stream_version < 0x1400_0004 {
        effective_av_flags &= !0x40;
    }
    effective_av_flags = (effective_av_flags & 0xFFE1) | 0x000E;
    let mut local_transform = NiTransform::default();
    for component in &mut local_transform.translation {
        *component = reader.f32(truncated)?;
    }
    for component in &mut local_transform.rotation {
        *component = reader.f32(truncated)?;
    }
    local_transform.scale = reader.f32(truncated)?;
    let property_indices = read_av_reference_list(&mut reader, truncated, list_truncated)?;
    let collision_index = reader.u32(truncated)?;
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

// IDA: N/A a helper, with no address of its own.
fn validate_reference(
    index: u32,
    object_count: usize,
    message: &'static str,
) -> Result<(), StreamError> {
    if index != NULL_REFERENCE && index as usize >= object_count {
        return Err(StreamError(message));
    }
    Ok(())
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
) -> Result<AvObjectLinkPlan, StreamError> {
    let count = table.objects.len();
    for &index in &net.extra_indices {
        validate_reference(index, count, "NiAVObject extra reference out of range")?;
    }
    validate_reference(
        net.controller_index,
        count,
        "NiAVObject controller reference out of range",
    )?;
    for &index in properties {
        validate_reference(index, count, "NiAVObject property reference out of range")?;
    }
    validate_reference(
        collision_index,
        count,
        "NiAVObject collision reference out of range",
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
) -> Result<(), StreamError> {
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
