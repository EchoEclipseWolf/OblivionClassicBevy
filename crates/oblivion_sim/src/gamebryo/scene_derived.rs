//! NiBillboardNode and NiDirectionalLight: the two scene classes derived from NiNode and NiDynamicEffect.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;

use super::av_object::{
    AvObjectLinkStatus, AvObjectPrefix, NiAvParent, NiAvProperties, commit_av_object_links,
    parse_av_object_prefix, plan_av_object_links, read_av_reference_list,
};
use super::object_net::{NiObjectNet, ObjectNetPending};
use super::scene_node::{
    NiAvWorld, NiEffectAffectedNodes, NiNodeEffects, SceneNodeLinkStatus, SceneNodePayload,
    link_scene_node, parse_scene_node_payload, spawn_scene_node,
};
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};
use super::transform::NiTransform;

const TRUNCATED: &str = "truncated derived scene payload";
const LIST_TRUNCATED: &str = "truncated derived scene reference list";

/// IDA: 00708F90's gate: stream node references are AddHead-ed into +0xCC at >= 0A00010E.
const LIGHT_NODE_LIST_VERSION: u32 = 0x0A00_010E;

#[derive(Clone, Debug, PartialEq)]
pub struct BillboardNodePending {
    pub node: SceneNodePayload,
    pub raw_mode_flags: u16,
    pub effective_mode_flags: u16,
    pub bytes_consumed: usize,
}

impl Default for BillboardNodePending {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            node: SceneNodePayload::default(),
            raw_mode_flags: 9,
            effective_mode_flags: 9,
            bytes_consumed: 0,
        }
    }
}

// IDA: 00722510 inherited NiNode load, then raw +0xDC u16 and OR8.
// Completion: 100% of the modern field order and resulting flags.
// TODO: [ ] Port legacy stream branches and callback failures.
pub fn parse_billboard_node_payload(
    bytes: &[u8],
    version: u32,
) -> Result<BillboardNodePending, GameError> {
    if version < 0x0A01_0072 {
        return Err(GameError::unclassified(
            0,
            "legacy NiBillboardNode payload unsupported",
        ));
    }
    let node = parse_scene_node_payload(bytes, version)?;
    let mut reader = ByteReader::new(&bytes[node.bytes_consumed..]);
    let raw_mode_flags = reader.u16(TRUNCATED)?;
    let effective_mode_flags = raw_mode_flags | 0x0008;
    let bytes_consumed = node.bytes_consumed + reader.position();
    Ok(BillboardNodePending {
        node,
        raw_mode_flags,
        effective_mode_flags,
        bytes_consumed,
    })
}

/// IDA: 00722490 allocates 0xE4, constructs NiNode, sets +0xE0 to zero, the vtable (`NI_BILLBOARD_NODE_VTABLE`), then
/// +0xDC to 9. Completion: 100% of the cold values.
/// TODO: [ ] Camera-facing updates (the consumers of +0xDC).
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct BillboardNode {
    pub raw_mode_flags_dc: u16,
    pub effective_mode_flags_dc: u16,
    pub unknown_e0: f32,
}

impl Default for BillboardNode {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            raw_mode_flags_dc: 9,
            effective_mode_flags_dc: 9,
            unknown_e0: 0.0,
        }
    }
}

// IDA: 00722490's cold NiBillboardNode: a NiNode whose class tag is NiBillboardNode, plus the +0xDC / +0xE0 fields.
// Completion: 100% of the cold values.
pub fn spawn_billboard_node(world: &mut World) -> Entity {
    let node = spawn_scene_node(world, SceneNodePayload::default());
    world
        .entity_mut(node)
        .insert((NiClass::BillboardNode, BillboardNode::default()));
    node
}

// IDA: 00722510 stores inherited NiNode fields before the +0xDC flags.
// Completion: 100% of the host publication after a successful parse.
// TODO: [ ] Model original partial writes on callback failure.
pub fn apply_billboard_node_payload(
    world: &mut World,
    target: Entity,
    pending: BillboardNodePending,
) {
    world.entity_mut(target).insert(pending.node);
    let mut billboard = world
        .get_mut::<BillboardNode>(target)
        .expect("a NiBillboardNode has its +0xDC fields");
    billboard.raw_mode_flags_dc = pending.raw_mode_flags;
    billboard.effective_mode_flags_dc = pending.effective_mode_flags;
}

// IDA: 007416F0 delegates to 0070B530 NiNode_LinkObject.
// Completion: 100% delegation to the shared NiNode linker.
// TODO: [ ] Bind billboard update consumers.
pub fn link_billboard_node(
    world: &mut World,
    node: Entity,
    resolved_children: &[Option<Entity>],
    resolved_effects: &[Option<Entity>],
    table: &StreamObjectTable,
) -> Result<SceneNodeLinkStatus, GameError> {
    link_scene_node(world, node, resolved_children, resolved_effects, table)
}

/// IDA: 00B259FC lpAddend__0, image value 1; its sole reference is the 007090C0 InterlockedIncrement. Host
/// substitution: the exclusive `&mut World` borrow replaces the interlocked ordering, and 32-bit wrap is kept.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DynamicEffectUidCounter(pub u32);

impl Default for DynamicEffectUidCounter {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self(1)
    }
}

// IDA: 007090C0 `push offset lpAddend__0; call InterlockedIncrement; mov [esi+0B0h], eax`. The incremented value,
// shared by every NiDynamicEffect subclass: the first call returns 2. Completion: 100%.
// TODO: [ ] Call from future NiLight/NiTextureEffect owners as they are bound.
pub fn allocate_dynamic_effect_uid(world: &mut World) -> u32 {
    let mut counter = world.get_resource_or_insert_with(DynamicEffectUidCounter::default);
    counter.0 = counter.0.wrapping_add(1);
    counter.0
}

/// IDA: 00707F00's prefix with where it ended. `parse_av_object_prefix` holds the shared read.
#[derive(Clone, Debug, PartialEq)]
pub struct DerivedAvPayload {
    pub net: ObjectNetPending,
    pub raw_av_flags: u16,
    pub effective_av_flags: u16,
    pub local_transform: NiTransform,
    pub property_indices: Vec<u32>,
    pub collision_index: u32,
    pub bytes_consumed: usize,
}

impl Default for DerivedAvPayload {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            net: ObjectNetPending::default(),
            raw_av_flags: 0x000E,
            effective_av_flags: 0x000E,
            local_transform: NiTransform::default(),
            property_indices: Vec::new(),
            collision_index: NULL_REFERENCE,
            bytes_consumed: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DirectionalLightPending {
    pub av: DerivedAvPayload,
    pub enabled: bool,
    pub affected_node_indices: Vec<u32>,
    pub dimmer: f32,
    pub ambient: [f32; 3],
    pub diffuse: [f32; 3],
    pub specular: [f32; 3],
    /// IDA: NiStream +0xD8 version for the 007081B0 / 00708F90 gates; set by the parse (N/A modern 20.0.0.4 default).
    pub stream_version: u32,
    pub bytes_consumed: usize,
}

impl Default for DirectionalLightPending {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            av: DerivedAvPayload::default(),
            enabled: true,
            affected_node_indices: Vec::new(),
            dimmer: 1.0,
            ambient: [1.0; 3],
            diffuse: [1.0; 3],
            specular: [1.0; 3],
            stream_version: 0x1400_0004,
            bytes_consumed: 0,
        }
    }
}

// IDA: 00742270 -> 0071A6D0 calls 00708EB0 inherited load (00707F00, the enable byte, the node list), then the dimmer
// and three NiVector3 loads (+0xDC, +0xE0 / +0xEC / +0xF8). 00708EB0 reads the enable byte from 0A010066 (below it the
// 007090C0 constructor's 1 stays) and the node list from 0A00010E. Completion: 100% of the >= 0A000100 raw sequence.
// TODO: [ ] Bind original short-read and the < 04010000 effect list branch.
pub fn parse_directional_light_payload(
    bytes: &[u8],
    version: u32,
) -> Result<DirectionalLightPending, GameError> {
    if version < 0x0A00_0100 {
        return Err(GameError::unclassified(
            0,
            "legacy NiDirectionalLight payload unsupported",
        ));
    }
    let (prefix, mut reader) = parse_av_object_prefix(bytes, version, TRUNCATED, LIST_TRUNCATED)?;
    let AvObjectPrefix {
        net,
        raw_av_flags,
        effective_av_flags,
        local_transform,
        property_indices,
        collision_index,
    } = prefix;
    let net_bytes = net.bytes_consumed;
    let av = DerivedAvPayload {
        net,
        raw_av_flags,
        effective_av_flags,
        local_transform,
        property_indices,
        collision_index,
        bytes_consumed: net_bytes + reader.position(),
    };
    let enabled = version < 0x0A01_0066 || reader.u8(TRUNCATED)? != 0;
    let affected_node_indices = if version >= LIGHT_NODE_LIST_VERSION {
        read_av_reference_list(
            &mut reader,
            TRUNCATED,
            GameError::from_legacy_message(LIST_TRUNCATED),
        )?
    } else {
        Vec::new()
    };
    let dimmer = reader.f32(TRUNCATED)?;
    let read_vector = |reader: &mut ByteReader<'_>| -> Result<[f32; 3], GameError> {
        let mut vector = [0.0; 3];
        for component in &mut vector {
            *component = reader.f32(TRUNCATED)?;
        }
        Ok(vector)
    };
    let ambient = read_vector(&mut reader)?;
    let diffuse = read_vector(&mut reader)?;
    let specular = read_vector(&mut reader)?;
    Ok(DirectionalLightPending {
        av,
        enabled,
        affected_node_indices,
        dimmer,
        ambient,
        diffuse,
        specular,
        stream_version: version,
        bytes_consumed: net_bytes + reader.position(),
    })
}

/// IDA: 007197F0 / 00719760 construct NiDynamicEffect -> NiLight -> NiDirectionalLight. The other NiAVObject state sits
/// in `NiObjectNet`, `NiAvProperties`, `NiAvParent` and `NiAvWorld`; the +0xBC list is `NiEffectAffectedNodes`.
/// Completion: 100% of the light's cold values and the 007090C0 +0xB0 UID.
/// TODO: [ ] Rendering updates.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct DirectionalLight {
    pub payload: DirectionalLightPending,
    /// IDA: 007090C0 +0xCC NiTPointerList<NiNode*>, head first. 00708F90 (>= 0A00010E) AddHeads each nonnull stream
    /// reference with no mutual 00708E40 call. xOBSE calls this field unaffectedNodes while the NIF format calls the
    /// stream list Affected Nodes; its consumer semantics are unverified, hence the neutral name.
    pub stream_node_list_cc: Vec<Entity>,
    pub unknown_b4: u32,
    pub enabled: bool,
    pub unknown_b8: u32,
    pub uid_b0: u32,
    pub unknown_104: u32,
    pub direction_108: [f32; 3],
    pub links_resolved: bool,
}

// IDA: 007197F0's cold NiDirectionalLight. The +0xB0 UID is allocated last, matching 007090C0's store order.
// Completion: 100% of the cold values.
pub fn spawn_directional_light(world: &mut World) -> Entity {
    let uid_b0 = allocate_dynamic_effect_uid(world);
    world
        .spawn((
            NiClass::DirectionalLight,
            DirectionalLight {
                payload: DirectionalLightPending::default(),
                stream_node_list_cc: Vec::new(),
                unknown_b4: 0,
                enabled: true,
                unknown_b8: 1,
                uid_b0,
                unknown_104: 0,
                direction_108: [1.0, 0.0, 0.0],
                links_resolved: false,
            },
            NiObjectNet::default(),
            NiAvProperties::default(),
            NiAvParent::default(),
            NiAvWorld::default(),
            NiEffectAffectedNodes::default(),
        ))
        .id()
}

// IDA: 0071A6D0 applies loaded +0xAC enable and +0xDC..+0x100 light values; +0x108 direction retains 00719760's cold
// value. Completion: 100% of the host publication after a successful parse.
// TODO: [ ] Bind original in-place callback failure state.
pub fn apply_directional_light_payload(
    world: &mut World,
    target: Entity,
    pending: DirectionalLightPending,
) {
    let mut light = world
        .get_mut::<DirectionalLight>(target)
        .expect("the target is a NiDirectionalLight");
    light.enabled = pending.enabled;
    light.payload = pending;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirectionalLightLinkStatus {
    Linked,
    UnsupportedOwner,
}

// IDA: 00742280 -> 0071A630 -> 00708F90: 007081B0's inherited link (extras, +0x98 properties, +0xA8 collision), then at
// >= 0A00010E each nonnull stream node reference is AddHead-ed into the +0xCC list with no mutual 00708E40 call.
// Completion: 100% of the preflight then publication order, for the null AV/effect domain plus typed ObjectNET owners.
// TODO: [ ] Light controller heads (NiLightColorController etc.) and the collision owner; nonnull heads stay unsupported.
pub fn link_directional_light(
    world: &mut World,
    target: Entity,
    table: &StreamObjectTable,
    object_count: usize,
) -> Result<DirectionalLightLinkStatus, GameError> {
    let light = world
        .get::<DirectionalLight>(target)
        .expect("the target is a NiDirectionalLight");
    let payload = light.payload.clone();
    let links_resolved = light.links_resolved;
    let av = &payload.av;
    let version = payload.stream_version;
    if object_count != table.objects.len() {
        return Err(GameError::unclassified(
            0,
            "directional light object count mismatch",
        ));
    }
    // IDA: N/A host guard around 00708F90's deferred reference resolution.
    for &index in &payload.affected_node_indices {
        if index != NULL_REFERENCE && index as usize >= object_count {
            return Err(GameError::unclassified(
                0,
                "derived scene reference out of range",
            ));
        }
    }
    if links_resolved || av.net.controller_index != NULL_REFERENCE {
        return Ok(DirectionalLightLinkStatus::UnsupportedOwner);
    }
    let plan = plan_av_object_links(
        world,
        &av.net,
        &av.property_indices,
        av.collision_index,
        table,
        version,
    )?;
    if plan.status != AvObjectLinkStatus::Linked {
        return Ok(DirectionalLightLinkStatus::UnsupportedOwner);
    }
    let mut listed = Vec::new();
    if version >= LIGHT_NODE_LIST_VERSION {
        for &index in &payload.affected_node_indices {
            if index == NULL_REFERENCE {
                continue;
            }
            let entity = table.objects[index as usize].entity;
            if !world
                .get::<NiClass>(entity)
                .is_some_and(|class| class.is_node())
            {
                return Ok(DirectionalLightLinkStatus::UnsupportedOwner);
            }
            listed.insert(0, entity);
        }
    }
    commit_av_object_links(world, target, &av.net, plan, table)?;
    let mut light = world
        .get_mut::<DirectionalLight>(target)
        .expect("checked above");
    light.stream_node_list_cc = listed;
    light.links_resolved = true;
    Ok(DirectionalLightLinkStatus::Linked)
}

// IDA: 00708E40 NiDynamicEffect_AttachAffectedNode(effect, node): when the node is not yet in the effect's +0xBC list (+0xC0
// head), AddHead it (a raw pointer, no refcount) and make the mirrored call, which AddHeads the effect to the node's +0xBC
// effect list. Shared by the Phase 07 menu lights and the Phase 08 sun light. Completion: 100%.
pub fn attach_affected_node(world: &mut World, light: Entity, node: Entity) {
    // 00708E43..00708E60: scan the effect's +0xC0 head for the node.
    if world.get::<NiEffectAffectedNodes>(light).is_none() {
        world
            .entity_mut(light)
            .insert(NiEffectAffectedNodes::default());
    }
    let mut affected = world
        .get_mut::<NiEffectAffectedNodes>(light)
        .expect("just ensured");
    if affected.0.contains(&node) {
        return;
    }
    // 00708E68..00708EA1 AddHead (raw pointer, no refcount), then the mirror call 00708E40(node, effect) on the node's +0xBC
    // effect list.
    affected.0.insert(0, node);
    let mut effects = world
        .get_mut::<NiNodeEffects>(node)
        .expect("a NiNode has an effect list");
    if !effects.0.contains(&light) {
        effects.0.insert(0, light);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dword(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }

    /// An empty named object's AV prefix: flags 0x0040, translation (2, 0, 0), identity rotation, scale 1.
    fn prefix_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        dword(&mut bytes, 0); // empty name
        dword(&mut bytes, 0); // no extras
        dword(&mut bytes, NULL_REFERENCE); // null controller
        bytes.extend(0x0040u16.to_le_bytes());
        dword(&mut bytes, 0x4000_0000);
        dword(&mut bytes, 0);
        dword(&mut bytes, 0);
        for index in 0..9 {
            dword(&mut bytes, if index % 4 == 0 { 0x3F80_0000 } else { 0 });
        }
        dword(&mut bytes, 0x3F80_0000);
        dword(&mut bytes, 0); // no properties
        dword(&mut bytes, NULL_REFERENCE); // null collision
        bytes
    }

    #[test]
    fn billboard_payload_sets_bit_3_after_the_node_load() {
        let mut bytes = prefix_bytes();
        dword(&mut bytes, 0); // no children
        dword(&mut bytes, 0); // no effects
        bytes.extend(0x0001u16.to_le_bytes());
        let parsed = parse_billboard_node_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(parsed.bytes_consumed, bytes.len());
        assert_eq!(parsed.raw_mode_flags, 1);
        assert_eq!(parsed.effective_mode_flags, 9);
        assert!(parse_billboard_node_payload(&bytes, 0x0A01_0071).is_err());
        assert!(parse_billboard_node_payload(&bytes[..bytes.len() - 1], 0x1400_0004).is_err());

        let mut world = World::new();
        let node = spawn_billboard_node(&mut world);
        assert_eq!(
            world.get::<NiClass>(node),
            Some(&NiClass::BillboardNode),
            "the billboard tag replaces the NiNode one"
        );
        assert_eq!(
            *world.get::<BillboardNode>(node).unwrap(),
            BillboardNode::default()
        );
        let local = parsed.node.local_transform;
        apply_billboard_node_payload(&mut world, node, parsed);
        assert_eq!(
            world
                .get::<SceneNodePayload>(node)
                .unwrap()
                .local_transform
                .translation,
            local.translation
        );
        assert_eq!(
            world.get::<BillboardNode>(node).unwrap().raw_mode_flags_dc,
            1
        );
        assert_eq!(
            world
                .get::<BillboardNode>(node)
                .unwrap()
                .effective_mode_flags_dc,
            9
        );
    }

    /// The two `spawn_cold` arms, at the C++ registration's constructors.
    #[test]
    fn factories_create_the_derived_classes() {
        use super::super::registration::{FactoryCreate, NiFactoryRegistry};
        let mut world = World::new();
        let mut registry = NiFactoryRegistry::default();
        for (name, constructor, class) in [
            ("NiBillboardNode", 0x0072_2490, NiClass::BillboardNode),
            ("NiDirectionalLight", 0x0071_97F0, NiClass::DirectionalLight),
        ] {
            registry.insert_or_assign(name, constructor);
            let FactoryCreate::Created { entity, .. } = registry.create(&mut world, name) else {
                panic!("{name} did not create");
            };
            assert_eq!(world.get::<NiClass>(entity), Some(&class));
        }
        assert_eq!(
            world.resource::<DynamicEffectUidCounter>().0,
            2,
            "the light's constructor took a UID"
        );
    }

    fn light_bytes(nodes: &[u32]) -> Vec<u8> {
        let mut bytes = prefix_bytes();
        bytes.push(0); // disabled
        dword(&mut bytes, nodes.len() as u32);
        for &node in nodes {
            dword(&mut bytes, node);
        }
        dword(&mut bytes, 0x3F00_0000); // dimmer 0.5
        for bits in [0x3F80_0000u32, 0x4000_0000, 0x4040_0000] {
            for _ in 0..3 {
                dword(&mut bytes, bits);
            }
        }
        bytes
    }

    #[test]
    fn light_payload_order_and_uid_sequence() {
        let bytes = light_bytes(&[NULL_REFERENCE]);
        let parsed = parse_directional_light_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(parsed.bytes_consumed, bytes.len());
        assert!(!parsed.enabled);
        assert_eq!(parsed.affected_node_indices, [NULL_REFERENCE]);
        assert_eq!(parsed.dimmer, 0.5);
        assert_eq!(parsed.ambient, [1.0; 3]);
        assert_eq!(parsed.diffuse, [2.0; 3]);
        assert_eq!(parsed.specular, [3.0; 3]);
        assert_eq!(parsed.av.effective_av_flags, 0x004E);
        assert_eq!(parsed.av.local_transform.translation[0], 2.0);
        assert!(parse_directional_light_payload(&bytes, 0x0A00_00FF).is_err());
        // 10.0.1.0 (the ears): no enable byte (the constructor's 1 stays) and no node list.
        let mut old = prefix_bytes();
        old.extend_from_slice(&bytes[prefix_bytes().len() + 1 + 4 + 4..]);
        let parsed_old = parse_directional_light_payload(&old, 0x0A00_0100).unwrap();
        assert_eq!(parsed_old.bytes_consumed, old.len());
        assert!(parsed_old.enabled);
        assert!(parsed_old.affected_node_indices.is_empty());
        assert_eq!(parsed_old.dimmer, 0.5);
        assert!(parse_directional_light_payload(&bytes[..bytes.len() - 1], 0x1400_0004).is_err());

        let mut world = World::new();
        let first = spawn_directional_light(&mut world);
        let second = spawn_directional_light(&mut world);
        let light = world.get::<DirectionalLight>(first).unwrap();
        assert_eq!(light.uid_b0, 2, "the image value 1, incremented");
        assert_eq!(light.direction_108, [1.0, 0.0, 0.0]);
        assert_eq!(light.unknown_b8, 1);
        assert_eq!(world.get::<DirectionalLight>(second).unwrap().uid_b0, 3);
        world.resource_mut::<DynamicEffectUidCounter>().0 = u32::MAX;
        assert_eq!(allocate_dynamic_effect_uid(&mut world), 0, "32-bit wrap");

        apply_directional_light_payload(&mut world, first, parsed);
        let light = world.get::<DirectionalLight>(first).unwrap();
        assert!(!light.enabled);
        assert_eq!(light.direction_108, [1.0, 0.0, 0.0], "+0x108 stays cold");
    }

    #[test]
    fn light_link_lists_nodes_head_first_without_the_mutual_attach() {
        let mut world = World::new();
        let first = spawn_scene_node(&mut world, SceneNodePayload::default());
        let second = spawn_billboard_node(&mut world);
        let data = world.spawn(NiClass::FloatKeyData).id();
        let mut table = StreamObjectTable::default();
        table.push("NiNode", 0, first);
        table.push("NiBillboardNode", 0, second);
        table.push("NiFloatData", 0, data);

        let light = spawn_directional_light(&mut world);
        let mut payload =
            parse_directional_light_payload(&light_bytes(&[0, NULL_REFERENCE, 1]), 0x1400_0004)
                .unwrap();
        apply_directional_light_payload(&mut world, light, payload.clone());
        assert_eq!(
            link_directional_light(&mut world, light, &table, 2).unwrap_err(),
            GameError::unclassified(0, "directional light object count mismatch")
        );
        assert_eq!(
            link_directional_light(&mut world, light, &table, 3).unwrap(),
            DirectionalLightLinkStatus::Linked
        );
        let linked = world.get::<DirectionalLight>(light).unwrap();
        assert_eq!(linked.stream_node_list_cc, [second, first]);
        assert!(linked.links_resolved);
        assert!(
            world
                .get::<super::super::scene_node::NiNodeEffects>(first)
                .unwrap()
                .0
                .is_empty(),
            "00708F90 makes no mutual 00708E40 call"
        );
        assert_eq!(
            link_directional_light(&mut world, light, &table, 3).unwrap(),
            DirectionalLightLinkStatus::UnsupportedOwner,
            "a relink is refused"
        );

        // A non-node reference, or a controller head, leaves the light unlinked.
        let other = spawn_directional_light(&mut world);
        payload.affected_node_indices = vec![2];
        apply_directional_light_payload(&mut world, other, payload.clone());
        assert_eq!(
            link_directional_light(&mut world, other, &table, 3).unwrap(),
            DirectionalLightLinkStatus::UnsupportedOwner
        );
        assert!(!world.get::<DirectionalLight>(other).unwrap().links_resolved);
        payload.affected_node_indices = vec![9];
        apply_directional_light_payload(&mut world, other, payload.clone());
        assert!(link_directional_light(&mut world, other, &table, 3).is_err());
        payload.affected_node_indices = vec![];
        payload.av.net.controller_index = 0;
        apply_directional_light_payload(&mut world, other, payload.clone());
        assert_eq!(
            link_directional_light(&mut world, other, &table, 3).unwrap(),
            DirectionalLightLinkStatus::UnsupportedOwner
        );

        // Below 0A00010E the stream list is not consumed.
        let old = spawn_directional_light(&mut world);
        payload.av.net.controller_index = NULL_REFERENCE;
        payload.affected_node_indices = vec![0];
        payload.stream_version = 0x0A00_010D;
        apply_directional_light_payload(&mut world, old, payload);
        assert_eq!(
            link_directional_light(&mut world, old, &table, 3).unwrap(),
            DirectionalLightLinkStatus::Linked
        );
        assert!(
            world
                .get::<DirectionalLight>(old)
                .unwrap()
                .stream_node_list_cc
                .is_empty()
        );
    }

    #[test]
    fn affected_nodes_attach_both_ways_once() {
        let mut world = World::new();
        let light = spawn_directional_light(&mut world);
        let first = spawn_scene_node(&mut world, SceneNodePayload::default());
        let second = spawn_scene_node(&mut world, SceneNodePayload::default());
        attach_affected_node(&mut world, light, first);
        attach_affected_node(&mut world, light, second);
        attach_affected_node(&mut world, light, first);
        assert_eq!(
            world.get::<NiEffectAffectedNodes>(light).unwrap().0,
            [second, first],
            "AddHead, once"
        );
        assert_eq!(world.get::<NiNodeEffects>(first).unwrap().0, [light]);
    }
}

/// IDA: NiPointLight (0x114, vtable A43044) as 004B1600 builds it: 0071A490 NiLight_Constructor over 007090C0
/// NiDynamicEffect (+0xAC enable 1, +0xB0 the UID, +0xB4 0, +0xB8 1) and 0070xxxx NiAVObject (+0x18 0x0E, identity); NiLight
/// +0xDC dimmer 1.0, +0xE0 ambient, +0xEC diffuse and +0xF8 specular 1.0, +0x104 0; then +0x108 = 0, +0x10C = 1.0,
/// +0x110 = 0 (constant, linear, quadratic attenuation). The other NiAVObject state is in `NiObjectNet`, `NiAvProperties`,
/// `NiAvParent` and `NiAvWorld`. Completion: 100% of the cold values.
/// TODO: [ ] Rendering: Bevy draws from `tes::light_3d::ShadowSceneDeferrals`.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct PointLight {
    pub av_flags_18: u16,
    pub local_transform: NiTransform,
    pub enabled_ac: bool,
    pub uid_b0: u32,
    pub unknown_b4: u32,
    /// +0xB8, counted up by each store block and each world update (00725410).
    pub revision_b8: u32,
    pub dimmer_dc: f32,
    pub ambient_e0: [f32; 3],
    pub diffuse_ec: [f32; 3],
    pub specular_f8: [f32; 3],
    pub unknown_104: u32,
    pub attenuation_108: [f32; 3],
}
