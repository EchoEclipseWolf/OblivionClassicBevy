//! NiSkinData and NiSkinInstance, and the resolved skin a mesh emitter samples from.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::reader::ByteReader;

use super::mesh_geometry::typed_reference;
use super::mesh_skin_partition::{MeshSkinPartition, MeshSkinPartitionRecord};
use super::particle_mesh_emitter::{ParticleMeshSkinPartitionView, ParticleMeshSkinResolvedView};
use super::scene_node::NiAvWorld;
use super::stream_objects::check_link_index;
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};
use super::transform::NiTransform;

/// IDA: N/A the error of this file's `stream_objects::check_link_index` calls (the host's guard before 007124A0 / 00752CB0's reads).
const MESH_SKIN_INDEX_OUT_OF_RANGE: GameError =
    GameError::unclassified(0, "mesh skin object index out of range");

const TRUNCATED: &str = "truncated mesh skin payload";

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeshSkinBoneWeight {
    pub vertex_index: u16,
    pub weight: f32,
}

/// IDA: 0072EF90's cold 0x4C NiSkinData::Bone; 0072F420 loads transform, sphere, u16 weight count and conditionally u16 /
/// f32 pairs. 0072EF90 initializes only the +0x44 pointer and +0x48 count; an identity transform and zero sphere stand in
/// for the uninitialized bytes. Completion: 100% of the loaded payload.
/// TODO: [ ] The +0x4A cold bytes, the 8-byte Weight padding and FormHeap identity.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshSkinBone {
    pub transform: NiTransform,
    pub bounding_sphere: [f32; 4],
    pub weights_44: Vec<MeshSkinBoneWeight>,
    pub weight_count_raw_48: u16,
}

/// IDA: 0072F210 allocates the 0x48 NiSkinData, clearing only the +0x08 partition and +0x44 bones pointers. Modern
/// 0072F420 leaves +0x08 null, and reads the +0x0C transform, +0x40 bone count and +0x44 bone records. An identity
/// transform and zero count stand in for the uninitialized bytes. Completion: 100% of the bounded loaded fields.
/// TODO: [ ] The pre-0A010065 deferred partition and legacy weight paths.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct MeshSkinData {
    pub transform_0c: NiTransform,
    pub weights_present_raw: u8,
    pub bones: Vec<MeshSkinBone>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeshSkinDataParseResult {
    pub owner: MeshSkinData,
    pub bytes_consumed: usize,
}

/// IDA: 00718B70 calls 00711B90 (nine f32 matrix entries), 00709430 (three f32 translation entries), then reads one f32
/// scale. This is not the NiAVObject order. Completion: 100% of the byte order and raw bits.
/// TODO: [ ] NiStream's short-read status.
fn read_transform(reader: &mut ByteReader<'_>) -> Result<NiTransform, GameError> {
    let mut result = NiTransform::default();
    for component in &mut result.rotation {
        *component = reader.f32(TRUNCATED)?;
    }
    for component in &mut result.translation {
        *component = reader.f32(TRUNCATED)?;
    }
    result.scale = reader.f32(TRUNCATED)?;
    Ok(result)
}

/// IDA: 0072F420 modern NiSkinData::LoadBinary: 00718B70's root transform, a u32 bone count, one raw weight flag, then the
/// 0x4C records: transform, 00716EA0 sphere, u16 weight count and optional u16 / f32 pairs. A weightless record still takes
/// 70 bytes. All or nothing here. Completion: 100% of the modern raw widths and order.
/// TODO: [ ] The 04010009 tail and the pre-0A010065 deferred partition reference.
pub fn parse_mesh_skin_data_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<MeshSkinDataParseResult, GameError> {
    if stream_version < 0x0A01_0072 {
        return Err(GameError::unclassified(
            0,
            "legacy NiSkinData payload unsupported",
        ));
    }
    let mut reader = ByteReader::new(bytes);
    let mut owner = MeshSkinData {
        transform_0c: read_transform(&mut reader)?,
        ..MeshSkinData::default()
    };
    let bone_count = reader.u32(TRUNCATED)?;
    owner.weights_present_raw = reader.u8(TRUNCATED)?;
    if bone_count as usize > reader.remaining() / 70 {
        return Err(GameError::stream("truncated NiSkinData bone records"));
    }
    owner.bones.reserve(bone_count as usize);
    for _ in 0..bone_count {
        let mut bone = MeshSkinBone {
            transform: read_transform(&mut reader)?,
            ..MeshSkinBone::default()
        };
        for component in &mut bone.bounding_sphere {
            *component = reader.f32(TRUNCATED)?;
        }
        bone.weight_count_raw_48 = reader.u16(TRUNCATED)?;
        if owner.weights_present_raw != 0 {
            if usize::from(bone.weight_count_raw_48) > reader.remaining() / 6 {
                return Err(GameError::stream("truncated NiSkinData bone weights"));
            }
            bone.weights_44
                .reserve(usize::from(bone.weight_count_raw_48));
            for _ in 0..bone.weight_count_raw_48 {
                let vertex_index = reader.u16(TRUNCATED)?;
                let weight = reader.f32(TRUNCATED)?;
                bone.weights_44.push(MeshSkinBoneWeight {
                    vertex_index,
                    weight,
                });
            }
        }
        owner.bones.push(bone);
    }
    Ok(MeshSkinDataParseResult {
        owner,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 0072F290 only resolves the legacy +0x08 partition reference for versions [04000001, 0A010065). Completion: 100% of
/// the modern no-op branch.
/// TODO: [ ] The legacy NiRefObjectPtr assignment when that format is ported.
pub const fn link_mesh_skin_data_modern(_data: &mut MeshSkinData) {}

/// IDA: 0072BAA0's queued references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshSkinInstancePayload {
    pub data_index: u32,
    pub partition_index: u32,
    pub root_parent_index: u32,
    pub bone_indices: Vec<u32>,
    pub bytes_consumed: usize,
}

impl Default for MeshSkinInstancePayload {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            data_index: NULL_REFERENCE,
            partition_index: NULL_REFERENCE,
            root_parent_index: NULL_REFERENCE,
            bone_indices: Vec::new(),
            bytes_consumed: 0,
        }
    }
}

/// IDA: 0072BD10 allocates the 0x2C NiSkinInstance: +0x18 serial -1; +0x08..+0x14 and +0x1C..+0x28 cold null or zero.
/// +0x10 root and +0x14 bones are raw pointers with no reference, stored as entities. Completion: 100% of the cold state
/// and the link identities.
/// TODO: [ ] The original's NiRefObjectPtr refcounts.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct MeshSkinInstance {
    pub pending: MeshSkinInstancePayload,
    pub data_08: Option<Entity>,
    pub partition_0c: Option<Entity>,
    pub root_parent_owner_10: Option<Entity>,
    pub bone_owners_14: Vec<Option<Entity>>,
    pub serial_18: i32,
    pub cold_tail_words_1c: [u32; 4],
    pub typed_links_bound: bool,
    pub scene_links_bound: bool,
}

impl Default for MeshSkinInstance {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            pending: MeshSkinInstancePayload::default(),
            data_08: None,
            partition_0c: None,
            root_parent_owner_10: None,
            bone_owners_14: Vec::new(),
            serial_18: -1,
            cold_tail_words_1c: [0; 4],
            typed_links_bound: false,
            scene_links_bound: false,
        }
    }
}

/// IDA: 0072BAA0 queues the u32 data, >= 0A010065 u32 partition, u32 root parent and 00712AE0's u32 count plus that many
/// u32 bone references. No owner is resolved during LoadBinary. Completion: 100% of the modern raw reference order.
/// TODO: [ ] The pre-0A010065 implicit partition and the legacy NiObject base.
pub fn parse_mesh_skin_instance_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<MeshSkinInstancePayload, GameError> {
    if stream_version < 0x0A01_0072 {
        return Err(GameError::unclassified(
            0,
            "legacy NiSkinInstance payload unsupported",
        ));
    }
    let mut reader = ByteReader::new(bytes);
    let data_index = reader.u32(TRUNCATED)?;
    let partition_index = reader.u32(TRUNCATED)?;
    let root_parent_index = reader.u32(TRUNCATED)?;
    let count = reader.u32(TRUNCATED)?;
    if count as usize > reader.remaining() / 4 {
        return Err(GameError::stream(
            "truncated NiSkinInstance bone references",
        ));
    }
    let mut bone_indices = Vec::with_capacity(count as usize);
    for _ in 0..count {
        bone_indices.push(reader.u32(TRUNCATED)?);
    }
    Ok(MeshSkinInstancePayload {
        data_index,
        partition_index,
        root_parent_index,
        bone_indices,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 0072BDB0 resolves and refcounts +0x08 data, then +0x0C partition. Root and bone references follow in the next
/// stage. Completion: 100% of the typed bind order and raw-index retention.
/// TODO: [ ] Intrusive refcount transitions.
pub fn bind_mesh_skin_instance_data_partition(
    instance: &mut MeshSkinInstance,
    data: Option<Entity>,
    partition: Option<Entity>,
    object_count: usize,
) -> Result<(), GameError> {
    let pending = &instance.pending;
    check_link_index(
        pending.data_index,
        object_count,
        MESH_SKIN_INDEX_OUT_OF_RANGE,
    )?;
    check_link_index(
        pending.partition_index,
        object_count,
        MESH_SKIN_INDEX_OUT_OF_RANGE,
    )?;
    check_link_index(
        pending.root_parent_index,
        object_count,
        MESH_SKIN_INDEX_OUT_OF_RANGE,
    )?;
    for &index in &pending.bone_indices {
        check_link_index(index, object_count, MESH_SKIN_INDEX_OUT_OF_RANGE)?;
    }
    if data.is_some() != (pending.data_index != NULL_REFERENCE)
        || partition.is_some() != (pending.partition_index != NULL_REFERENCE)
    {
        return Err(GameError::unclassified(
            0,
            "NiSkinInstance typed reference mismatch",
        ));
    }
    instance.data_08 = data;
    instance.partition_0c = partition;
    instance.typed_links_bound = true;
    Ok(())
}

/// IDA: 0072BDB0 stores the raw +0x10 root before allocating and writing each +0x14 bone pointer in deferred-list order.
/// Completion: 100% of the store order; the original's zero-length FormHeap allocation is open.
/// TODO: [ ] The FormHeap array.
pub fn bind_mesh_skin_instance_scene_owners(
    instance: &mut MeshSkinInstance,
    root: Option<Entity>,
    bones: Vec<Option<Entity>>,
    object_count: usize,
) -> Result<(), GameError> {
    let pending = &instance.pending;
    check_link_index(
        pending.root_parent_index,
        object_count,
        MESH_SKIN_INDEX_OUT_OF_RANGE,
    )?;
    if bones.len() != pending.bone_indices.len() {
        return Err(GameError::unclassified(
            0,
            "NiSkinInstance bone-owner slot count mismatch",
        ));
    }
    if root.is_some() != (pending.root_parent_index != NULL_REFERENCE) {
        return Err(GameError::unclassified(
            0,
            "NiSkinInstance root-owner identity mismatch",
        ));
    }
    for (&index, bone) in pending.bone_indices.iter().zip(&bones) {
        check_link_index(index, object_count, MESH_SKIN_INDEX_OUT_OF_RANGE)?;
        if bone.is_some() != (index != NULL_REFERENCE) {
            return Err(GameError::unclassified(
                0,
                "NiSkinInstance bone-owner identity mismatch",
            ));
        }
    }
    instance.root_parent_owner_10 = root;
    instance.bone_owners_14 = bones;
    instance.scene_links_bound = true;
    Ok(())
}

/// IDA: 0072BE70's and the graph's link outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshSkinLinkStatus {
    Linked,
    UnsupportedOwner,
}

/// IDA: N/A what the C++ graph's scene-owner lookup accepts: NiNode (not NiBillboardNode), NiTriShape (not NiTriStrips) and
/// NiParticleSystem. `None` is another class.
/// TODO: [ ] Other NiAVObject owners as their world fields are ported.
fn skin_scene_owner(
    world: &World,
    table: &StreamObjectTable,
    index: u32,
) -> Result<Option<Option<Entity>>, GameError> {
    if index == NULL_REFERENCE {
        return Ok(Some(None));
    }
    let entry = table
        .objects
        .get(index as usize)
        .ok_or(GameError::unclassified(
            0,
            "NiSkinInstance scene index out of range",
        ))?;
    let supported = matches!(
        world.get::<NiClass>(entry.entity),
        Some(NiClass::Node | NiClass::TriShape | NiClass::ParticleSystem)
    );
    Ok(supported.then_some(Some(entry.entity)))
}

/// IDA: 0072BDB0 as the stream graph runs it: +0x08 data then +0x0C partition by class, then the +0x10 root and +0x14 bones.
/// The stages run on a copy, so a later unsupported bone leaves the instance fresh. Completion: 100% of the order.
pub fn link_mesh_skin_instance(
    world: &mut World,
    entity: Entity,
    table: &StreamObjectTable,
) -> Result<MeshSkinLinkStatus, GameError> {
    let mut staged = world
        .get::<MeshSkinInstance>(entity)
        .ok_or(GameError::unclassified(0, "NiSkinInstance owner mismatch"))?
        .clone();
    let Some(data) = typed_reference(
        world,
        table,
        staged.pending.data_index,
        NiClass::SkinData,
        "NiSkinInstance data index out of range",
    )?
    else {
        return Ok(MeshSkinLinkStatus::UnsupportedOwner);
    };
    let Some(partition) = typed_reference(
        world,
        table,
        staged.pending.partition_index,
        NiClass::SkinPartition,
        "NiSkinInstance partition index out of range",
    )?
    else {
        return Ok(MeshSkinLinkStatus::UnsupportedOwner);
    };
    bind_mesh_skin_instance_data_partition(&mut staged, data, partition, table.objects.len())?;
    let Some(root) = skin_scene_owner(world, table, staged.pending.root_parent_index)? else {
        return Ok(MeshSkinLinkStatus::UnsupportedOwner);
    };
    let mut bones = Vec::with_capacity(staged.pending.bone_indices.len());
    for &index in &staged.pending.bone_indices {
        let Some(bone) = skin_scene_owner(world, table, index)? else {
            return Ok(MeshSkinLinkStatus::UnsupportedOwner);
        };
        bones.push(bone);
    }
    bind_mesh_skin_instance_scene_owners(&mut staged, root, bones, table.objects.len())?;
    *world
        .get_mut::<MeshSkinInstance>(entity)
        .expect("read above") = staged;
    Ok(MeshSkinLinkStatus::Linked)
}

/// IDA: 0074A390 consumes NiSkinData +0x0C and +0x44 transforms with NiSkinInstance +0x10 / +0x14 world transforms and the
/// partition's +0x0C array. The worlds are snapshots from `NiAvWorld` at resolve time, where the C++ port asks a provider.
/// Completion: 100% of the sources.
/// TODO: [ ] The original's live frame recomputation epoch.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshSkinResolvedOwner {
    pub instance: Entity,
    pub partition: Entity,
    pub root_parent_world: NiTransform,
    pub bone_world: Vec<NiTransform>,
    pub skin_bone_transform: Vec<NiTransform>,
}

impl MeshSkinResolvedOwner {
    /// IDA: N/A the views 0074A390 needs, one per partition record in array order. A view cannot hold its own storage, so
    /// the caller keeps this and passes it to `view`. Completion: 100%.
    #[must_use]
    pub fn partition_views<'w>(&self, world: &'w World) -> Vec<ParticleMeshSkinPartitionView<'w>> {
        world
            .get::<MeshSkinPartition>(self.partition)
            .map_or_else(Vec::new, |partition| {
                partition
                    .partitions
                    .iter()
                    .map(MeshSkinPartitionRecord::view)
                    .collect()
            })
    }

    /// IDA: 0074A390's source bundle; the skin transform is read live. A vanished instance or data gives the empty view.
    /// Completion: 100%.
    #[must_use]
    pub fn view<'a>(
        &'a self,
        world: &'a World,
        partitions: &'a [ParticleMeshSkinPartitionView<'a>],
    ) -> ParticleMeshSkinResolvedView<'a> {
        let data = world
            .get::<MeshSkinInstance>(self.instance)
            .and_then(|instance| instance.data_08)
            .and_then(|data| world.get::<MeshSkinData>(data));
        let Some(data) = data else {
            return ParticleMeshSkinResolvedView::default();
        };
        ParticleMeshSkinResolvedView {
            skin_data_transform: data.transform_0c,
            root_parent_world: self.root_parent_world,
            bone_world: &self.bone_world,
            skin_bone_transform: &self.skin_bone_transform,
            partitions,
        }
    }
}

/// IDA: 0074A390's source requirements, with the instance's current +0x0C partition. Original 0072ED50 lazy generation and
/// world-transform production stay unresolved. Completion: 100% of the checked assembly for the non-lazy branch.
/// TODO: [ ] 0072ED50, only after its full caller and data contract.
pub fn make_mesh_skin_resolved_owner(
    world: &World,
    instance: Option<Entity>,
) -> Result<MeshSkinResolvedOwner, GameError> {
    let partition = instance
        .and_then(|instance| world.get::<MeshSkinInstance>(instance))
        .and_then(|instance| instance.partition_0c);
    make_mesh_skin_resolved_owner_with_partition(world, instance, partition)
}

/// IDA: 0074C910 caches the partition pointer current at MeshEmitter LinkObject; 0074A390 must consume that cache, not a
/// later instance +0x0C. Completion: 100% of the identity checks and the world reads.
/// TODO: [ ] 0074A2D0's separate cache fill and lazy path when reached.
pub fn make_mesh_skin_resolved_owner_with_partition(
    world: &World,
    instance: Option<Entity>,
    cached_partition: Option<Entity>,
) -> Result<MeshSkinResolvedOwner, GameError> {
    const UNRESOLVED: &str = "NiSkinInstance data/partition unresolved";
    let present =
        |owner: Option<Entity>| owner.is_some_and(|owner| world.get_entity(owner).is_ok());
    let instance_entity = instance.ok_or(GameError::from_legacy_message(UNRESOLVED))?;
    let owner = world
        .get::<MeshSkinInstance>(instance_entity)
        .ok_or(GameError::from_legacy_message(UNRESOLVED))?;
    let partition = cached_partition.ok_or(GameError::from_legacy_message(UNRESOLVED))?;
    if !owner.typed_links_bound
        || !owner.scene_links_bound
        || owner.data_08.is_none()
        || world.get::<MeshSkinPartition>(partition).is_none()
        || !present(owner.root_parent_owner_10)
    {
        return Err(GameError::from_legacy_message(UNRESOLVED));
    }
    let data = owner
        .data_08
        .and_then(|data| world.get::<MeshSkinData>(data))
        .ok_or(GameError::from_legacy_message(UNRESOLVED))?;
    if owner.pending.root_parent_index == NULL_REFERENCE {
        return Err(GameError::unclassified(
            0,
            "NiSkinInstance scene transforms unresolved",
        ));
    }
    let root_world = owner
        .root_parent_owner_10
        .and_then(|root| world.get::<NiAvWorld>(root));
    let Some(root_world) = root_world else {
        return Err(GameError::unclassified(
            0,
            "NiSkinInstance root/bone transform mismatch",
        ));
    };
    if owner.pending.bone_indices.len() != data.bones.len() {
        return Err(GameError::unclassified(
            0,
            "NiSkinInstance root/bone transform mismatch",
        ));
    }
    let mut bone_world = Vec::with_capacity(data.bones.len());
    let mut skin_bone_transform = Vec::with_capacity(data.bones.len());
    for (index, (&raw_index, bone)) in owner
        .pending
        .bone_indices
        .iter()
        .zip(&data.bones)
        .enumerate()
    {
        let bone_owner = owner.bone_owners_14[index];
        if raw_index == NULL_REFERENCE || !present(bone_owner) {
            return Err(GameError::unclassified(
                0,
                "null NiSkinInstance bone world transform",
            ));
        }
        let Some(world_state) = bone_owner.and_then(|bone| world.get::<NiAvWorld>(bone)) else {
            return Err(GameError::unclassified(
                0,
                "NiSkinInstance bone world transform unavailable",
            ));
        };
        bone_world.push(world_state.transform);
        skin_bone_transform.push(bone.transform);
    }
    Ok(MeshSkinResolvedOwner {
        instance: instance_entity,
        partition,
        root_parent_world: root_world.transform,
        bone_world,
        skin_bone_transform,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dword(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }

    fn transform(bytes: &mut Vec<u8>, translation_x: u32) {
        for index in 0..13 {
            dword(
                bytes,
                match index {
                    9 => translation_x,
                    0 | 4 | 8 | 12 => 0x3F80_0000,
                    _ => 0,
                },
            );
        }
    }

    fn at_x(x: f32) -> NiAvWorld {
        NiAvWorld {
            transform: NiTransform {
                translation: [x, 0.0, 0.0],
                ..NiTransform::default()
            },
            ..NiAvWorld::default()
        }
    }

    /// Ported from `gamebryo_mesh_owners` `verify_skin_data_instance_and_scene_boundary`. The missing-scene check is a
    /// root with no world, in place of the C++ empty provider. The resets before the final view test `shared_ptr`
    /// lifetimes and are dropped.
    #[test]
    fn skin_data_instance_and_scene_boundary() {
        let mut bytes = Vec::new();
        transform(&mut bytes, 0x4000_0000); // root skin transform translation x2
        dword(&mut bytes, 1); // one bone
        bytes.push(1); // weights present
        transform(&mut bytes, 0x4040_0000); // bone skin transform translation x3
        for _ in 0..4 {
            dword(&mut bytes, 0);
        }
        bytes.extend(1u16.to_le_bytes()); // one weight, deliberately u16
        bytes.extend(9u16.to_le_bytes()); // vertex index
        dword(&mut bytes, 0x3F00_0000); // weight 0.5
        let parsed = parse_mesh_skin_data_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(parsed.bytes_consumed, bytes.len());
        assert_eq!(parsed.owner.bones.len(), 1);
        assert_eq!(parsed.owner.transform_0c.translation[0], 2.0);
        assert_eq!(parsed.owner.bones[0].transform.translation[0], 3.0);
        assert_eq!(parsed.owner.bones[0].weights_44[0].vertex_index, 9);
        assert_eq!(parsed.owner.bones[0].weights_44[0].weight, 0.5);

        let mut instance_bytes = Vec::new();
        for value in [0, 1, 2, 1, 3] {
            dword(&mut instance_bytes, value);
        }
        let pending = parse_mesh_skin_instance_payload(&instance_bytes, 0x1400_0004).unwrap();
        assert_eq!(pending.bytes_consumed, 20);
        assert_eq!(pending.data_index, 0);
        assert_eq!(pending.partition_index, 1);
        assert_eq!(pending.root_parent_index, 2);
        assert_eq!(pending.bone_indices, [3]);

        let mut world = World::new();
        let data = world.spawn((NiClass::SkinData, parsed.owner)).id();
        let partition = world
            .spawn((
                NiClass::SkinPartition,
                MeshSkinPartition {
                    partitions: vec![MeshSkinPartitionRecord::default()],
                },
            ))
            .id();
        let root_node = world.spawn(NiClass::Node).id();
        let bone_node = world.spawn(NiClass::Node).id();
        let mut instance = MeshSkinInstance {
            pending,
            ..MeshSkinInstance::default()
        };
        bind_mesh_skin_instance_data_partition(&mut instance, Some(data), Some(partition), 4)
            .unwrap();
        bind_mesh_skin_instance_scene_owners(
            &mut instance,
            Some(root_node),
            vec![Some(bone_node)],
            4,
        )
        .unwrap();
        let instance = world.spawn((NiClass::SkinInstance, instance)).id();
        assert!(
            make_mesh_skin_resolved_owner(&world, Some(instance)).is_err(),
            "skin resolution fabricated missing scene transforms"
        );
        world.entity_mut(root_node).insert(at_x(4.0));
        assert!(
            make_mesh_skin_resolved_owner(&world, Some(instance)).is_err(),
            "skin resolution fabricated a missing bone transform"
        );
        world.entity_mut(bone_node).insert(at_x(5.0));
        let resolved = make_mesh_skin_resolved_owner(&world, Some(instance)).unwrap();
        let partitions = resolved.partition_views(&world);
        let view = resolved.view(&world, &partitions);
        assert_eq!(view.skin_data_transform.translation[0], 2.0);
        assert_eq!(view.skin_bone_transform[0].translation[0], 3.0);
        assert_eq!(view.root_parent_world.translation[0], 4.0);
        assert_eq!(view.bone_world[0].translation[0], 5.0);
        assert_eq!(view.partitions.len(), 1);
    }

    /// Beyond the C++ test: the graph's typed link, with an unsupported bone leaving the instance fresh.
    #[test]
    fn world_link_binds_or_leaves_the_instance_fresh() {
        let mut world = World::new();
        let data = world.spawn(NiClass::SkinData).id();
        let partition = world.spawn(NiClass::SkinPartition).id();
        let root = world.spawn(NiClass::Node).id();
        let strips = world.spawn(NiClass::TriStrips).id();
        let mut table = StreamObjectTable::default();
        for (name, entity) in [
            ("NiSkinData", data),
            ("NiSkinPartition", partition),
            ("NiNode", root),
            ("NiTriStrips", strips),
        ] {
            table.push(name, 0, entity);
        }
        let spawn = |world: &mut World, bones: Vec<u32>| {
            world
                .spawn((
                    NiClass::SkinInstance,
                    MeshSkinInstance {
                        pending: MeshSkinInstancePayload {
                            data_index: 0,
                            partition_index: 1,
                            root_parent_index: 2,
                            bone_indices: bones,
                            bytes_consumed: 0,
                        },
                        ..MeshSkinInstance::default()
                    },
                ))
                .id()
        };
        let good = spawn(&mut world, vec![2, NULL_REFERENCE]);
        assert_eq!(
            link_mesh_skin_instance(&mut world, good, &table).unwrap(),
            MeshSkinLinkStatus::Linked
        );
        let linked = world.get::<MeshSkinInstance>(good).unwrap();
        assert_eq!(linked.data_08, Some(data));
        assert_eq!(linked.partition_0c, Some(partition));
        assert_eq!(linked.root_parent_owner_10, Some(root));
        assert_eq!(linked.bone_owners_14, [Some(root), None]);
        assert_eq!(linked.serial_18, -1);

        let refused = spawn(&mut world, vec![3]);
        assert_eq!(
            link_mesh_skin_instance(&mut world, refused, &table).unwrap(),
            MeshSkinLinkStatus::UnsupportedOwner
        );
        let fresh = world.get::<MeshSkinInstance>(refused).unwrap();
        assert!(fresh.data_08.is_none() && !fresh.typed_links_bound);
        let far = spawn(&mut world, vec![9]);
        assert_eq!(
            link_mesh_skin_instance(&mut world, far, &table).unwrap_err(),
            GameError::unclassified(0, "mesh skin object index out of range")
        );
    }
}
