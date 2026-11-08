//! NiSkinPartition: the bone-limited vertex and strip batches a skinned mesh draws and a mesh emitter samples.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::particle_mesh_emitter::ParticleMeshSkinPartitionView;

const TRUNCATED: &str = "truncated NiSkinPartition payload";

/// IDA: 0072C420 constructs each 0x2C Partition with six null pointer DWORDs +0x04..+0x18, the +0x22 strip count zero and
/// the +0x28 tail zero. The other count WORDs are not initialized by the original; these host zeros stand in until
/// LoadBinary overwrites them. Completion: 100% of the initialized cold fields and the loaded representation.
/// TODO: [ ] Array allocator identity and the original's destructor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshSkinPartitionRecord {
    pub bones_04: Vec<u16>,
    pub weights_08: Vec<f32>,
    pub vertex_map_0c: Vec<u16>,
    pub bone_indices_10: Vec<u8>,
    pub strip_data_14: Vec<u16>,
    pub strip_lengths_18: Vec<u16>,
    pub vertex_count_1c: u16,
    pub triangle_count_1e: u16,
    pub bone_count_20: u16,
    pub strip_count_22: u16,
    pub weights_per_vertex_24: u16,
    pub raw_tail_word_28: u32,
    pub vertex_map_present_raw: u8,
    pub weights_present_raw: u8,
    pub strip_data_present_raw: u8,
    pub bone_indices_present_raw: u8,
}

impl MeshSkinPartitionRecord {
    /// IDA: 0074A390 consumes Partition's six pointers at +0x04..+0x18, the five u16 counts at +0x1C..+0x24 and the raw
    /// +0x28 word, in this order. Completion: 100% of the typed view.
    /// TODO: [ ] The original's bone and world transform ownership.
    #[must_use]
    pub fn view(&self) -> ParticleMeshSkinPartitionView<'_> {
        ParticleMeshSkinPartitionView {
            bones: &self.bones_04,
            weights: &self.weights_08,
            vertex_map: &self.vertex_map_0c,
            bone_indices: &self.bone_indices_10,
            strip_data: &self.strip_data_14,
            strip_lengths: &self.strip_lengths_18,
            vertex_count: self.vertex_count_1c,
            triangle_count: self.triangle_count_1e,
            bone_count: self.bone_count_20,
            strip_count: self.strip_count_22,
            weights_per_vertex: self.weights_per_vertex_24,
            raw_tail_word: self.raw_tail_word_28,
        }
    }
}

/// IDA: 0072CAA0 allocates the 0x10 NiSkinPartition; 0072E010 loads the +0x08 u32 count and the array of 0x2C records at
/// +0x0C. Completion: 100% of the cold and loaded owner.
/// TODO: [ ] The original's array constructor, destructor and FormHeap failures.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct MeshSkinPartition {
    pub partitions: Vec<MeshSkinPartitionRecord>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeshSkinPartitionParseResult {
    pub owner: MeshSkinPartition,
    pub bytes_consumed: usize,
}

// IDA: N/A a helper, with no address of its own.
fn read_words(reader: &mut ByteReader<'_>, count: usize) -> Result<Vec<u16>, StreamError> {
    if count > reader.remaining() / 2 {
        return Err(StreamError("truncated NiSkinPartition u16 array"));
    }
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        result.push(reader.u16(TRUNCATED)?);
    }
    Ok(result)
}

// IDA: N/A a helper, with no address of its own.
fn read_floats(reader: &mut ByteReader<'_>, count: usize) -> Result<Vec<f32>, StreamError> {
    if count > reader.remaining() / 4 {
        return Err(StreamError("truncated NiSkinPartition weight array"));
    }
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        result.push(reader.f32(TRUNCATED)?);
    }
    Ok(result)
}

/// IDA: 0072D860 reads five u16 counts, bones (unconditionally, even zero-length), the optional map and weights, the strip
/// lengths (or triangles*3 words without strips) and optional indices, then the optional u8 bone indices. The +0x28 tail
/// is not loaded. Completion: 100% of the >= 0A000112 presence, order and raw counts.
/// TODO: [ ] The older implicit-array branch and FormHeap failure effects.
fn read_partition(reader: &mut ByteReader<'_>) -> Result<MeshSkinPartitionRecord, StreamError> {
    let mut result = MeshSkinPartitionRecord {
        vertex_count_1c: reader.u16(TRUNCATED)?,
        triangle_count_1e: reader.u16(TRUNCATED)?,
        bone_count_20: reader.u16(TRUNCATED)?,
        strip_count_22: reader.u16(TRUNCATED)?,
        weights_per_vertex_24: reader.u16(TRUNCATED)?,
        ..MeshSkinPartitionRecord::default()
    };
    result.bones_04 = read_words(reader, usize::from(result.bone_count_20))?;
    result.vertex_map_present_raw = reader.u8(TRUNCATED)?;
    if result.vertex_map_present_raw != 0 {
        result.vertex_map_0c = read_words(reader, usize::from(result.vertex_count_1c))?;
    }
    let weighted_vertices =
        usize::from(result.vertex_count_1c) * usize::from(result.weights_per_vertex_24);
    result.weights_present_raw = reader.u8(TRUNCATED)?;
    if result.weights_present_raw != 0 {
        result.weights_08 = read_floats(reader, weighted_vertices)?;
    }
    let mut strip_words = 0usize;
    if result.strip_count_22 != 0 {
        result.strip_lengths_18 = read_words(reader, usize::from(result.strip_count_22))?;
        for &length in &result.strip_lengths_18 {
            strip_words += usize::from(length);
        }
    } else {
        strip_words = usize::from(result.triangle_count_1e) * 3;
    }
    result.strip_data_present_raw = reader.u8(TRUNCATED)?;
    if result.strip_data_present_raw != 0 {
        result.strip_data_14 = read_words(reader, strip_words)?;
    }
    result.bone_indices_present_raw = reader.u8(TRUNCATED)?;
    if result.bone_indices_present_raw != 0 {
        result.bone_indices_10 = reader.take(weighted_vertices, TRUNCATED)?.to_vec();
    }
    Ok(result)
}

/// IDA: 0072E010 -> 0072D860, modern >= 0A010072: a u32 count and one 0x2C record each. Each record takes at least ten
/// count bytes and four presence bytes. The parse completes before the owner is published. Completion: 100% of the
/// widths, order, version gates, raw presence bytes and empty arrays.
/// TODO: [ ] The older implicit-array branches and callback allocation errors.
pub fn parse_mesh_skin_partition_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<MeshSkinPartitionParseResult, StreamError> {
    if stream_version < 0x0A01_0072 {
        return Err(StreamError("legacy NiSkinPartition payload unsupported"));
    }
    let mut reader = ByteReader::new(bytes);
    let count = reader.u32(TRUNCATED)?;
    if count as usize > reader.remaining() / 14 {
        return Err(StreamError("truncated NiSkinPartition record array"));
    }
    let mut owner = MeshSkinPartition::default();
    owner.partitions.reserve(count as usize);
    for _ in 0..count {
        owner.partitions.push(read_partition(&mut reader)?);
    }
    Ok(MeshSkinPartitionParseResult {
        owner,
        bytes_consumed: reader.position(),
    })
}

/// IDA: NiSkinPartition vtable 00A80384 +0x20 -> 006E7260 -> 0068F970: an empty LinkObject. Completion: 100%.
/// TODO: [ ] NiSkinInstance's separate deferred reference to this owner is in its module.
pub const fn link_mesh_skin_partition(_owner: &mut MeshSkinPartition) {}

/// IDA: 0074A390 reads live array pointers and counts from a retained Partition: the owner entity and a record index.
/// Completion: 100% of the checked host handle.
/// TODO: [ ] The original's intrusive reference counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshSkinPartitionOwnedView {
    pub owner: Entity,
    pub partition_index: usize,
}

impl MeshSkinPartitionOwnedView {
    /// IDA: 0074A390 reads a retained NiSkinPartition::Partition record. Completion: 100%.
    pub fn view<'w>(
        &self,
        world: &'w World,
    ) -> Result<ParticleMeshSkinPartitionView<'w>, StreamError> {
        world
            .get::<MeshSkinPartition>(self.owner)
            .and_then(|owner| owner.partitions.get(self.partition_index))
            .map(MeshSkinPartitionRecord::view)
            .ok_or(StreamError("NiSkinPartition view index out of range"))
    }
}

/// IDA: N/A a checked wrapper around 0074A390's borrowed Partition fields. Completion: 100%.
pub fn make_mesh_skin_partition_view(
    world: &World,
    owner: Entity,
    partition_index: usize,
) -> Result<MeshSkinPartitionOwnedView, StreamError> {
    let found = world
        .get::<MeshSkinPartition>(owner)
        .is_some_and(|partition| partition_index < partition.partitions.len());
    if !found {
        return Err(StreamError("NiSkinPartition owner/index unresolved"));
    }
    Ok(MeshSkinPartitionOwnedView {
        owner,
        partition_index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from `gamebryo_mesh_owners` `verify_partition_optional_arrays_and_lifetime`. The `owner.reset()` before
    /// the retained view tests a `shared_ptr` lifetime and has no meaning here.
    #[test]
    fn partition_optional_arrays() {
        let mut bytes = 1u32.to_le_bytes().to_vec();
        for count in [1u16, 0, 1, 0, 2] {
            bytes.extend(count.to_le_bytes());
        }
        bytes.extend(4u16.to_le_bytes()); // bone map
        bytes.push(2); // raw noncanonical true vertex map
        bytes.extend(7u16.to_le_bytes());
        bytes.push(1); // weights present
        bytes.extend(0x3F00_0000u32.to_le_bytes());
        bytes.extend(0x3F00_0000u32.to_le_bytes());
        bytes.push(0); // strip data absent
        bytes.push(1); // bone indices present
        bytes.push(0);
        bytes.push(0);
        let extent = bytes.len();
        bytes.push(0xAB);
        let parsed = parse_mesh_skin_partition_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(parsed.bytes_consumed, extent);
        assert_eq!(parsed.owner.partitions.len(), 1);
        assert_eq!(parsed.owner.partitions[0].vertex_map_present_raw, 2);
        assert!(parsed.owner.partitions[0].strip_data_14.is_empty());
        assert_eq!(parsed.owner.partitions[0].view().weights.len(), 2);

        let mut world = World::new();
        let owner = world.spawn(parsed.owner).id();
        let retained = make_mesh_skin_partition_view(&world, owner, 0).unwrap();
        let view = retained.view(&world).unwrap();
        assert_eq!(view.bones[0], 4);
        assert_eq!(view.vertex_map[0], 7);
        assert_eq!(view.bone_indices.len(), 2);
        assert!(make_mesh_skin_partition_view(&world, owner, 1).is_err());
        assert!(
            parse_mesh_skin_partition_payload(&bytes[..extent - 1], 0x1400_0004).is_err(),
            "truncated SkinPartition optional index array accepted"
        );

        let mut absent = 1u32.to_le_bytes().to_vec();
        for count in [1u16, 1, 0, 0, 0] {
            absent.extend(count.to_le_bytes());
        }
        absent.extend([0; 4]);
        let parsed = parse_mesh_skin_partition_payload(&absent, 0x1400_0004).unwrap();
        let record = &parsed.owner.partitions[0];
        assert_eq!(parsed.bytes_consumed, absent.len());
        assert_eq!(record.vertex_count_1c, 1);
        assert_eq!(record.triangle_count_1e, 1);
        assert!(record.vertex_map_0c.is_empty());
        assert!(record.weights_08.is_empty());
        assert!(record.strip_data_14.is_empty());
        assert!(record.bone_indices_10.is_empty());
    }
}
