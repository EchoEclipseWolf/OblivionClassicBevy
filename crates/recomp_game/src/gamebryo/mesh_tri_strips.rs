//! NiTriStripsData, and the NiTriStrips link that shares NiTriShape's payload.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::av_object::{AvObjectLinkStatus, commit_av_object_links, plan_av_object_links};
use super::mesh_geometry::{
    MeshGeometryData, MeshTriShapeLinkStatus, MeshTriShapePayload, NiGeometry, claim_group,
    parse_mesh_geometry_data_payload, parse_mesh_tri_shape_payload,
};
use super::stream_allocation_groups::{
    StreamAllocationGroup, StreamAllocationGroups, StreamGroupArray,
};
use super::stream_objects::{NULL_REFERENCE, StreamObjectTable};

const TRUNCATED_BYTE: &str = "truncated NiTriStripsData byte";
const TRUNCATED_WORD: &str = "truncated NiTriStripsData word";

/// IDA: 00719DC0's outputs: count, lengths, points and the zero-extended WORD point count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MeshTriStripTopologyView<'a> {
    pub strip_count: u16,
    pub strip_lengths: &'a [u16],
    pub strip_points: &'a [u16],
    pub point_word_count: u32,
}

/// IDA: 00719E00 creates the 0x50 NiTriStripsData through 00732DD0. Completion: 100% of the initialized strip-count,
/// length and point cold fields; the inherited bounds and the +0x42 / +0x46 padding stay uninitialized in the original,
/// unlike these host defaults.
/// TODO: [ ] FormHeap and group aliases, and NiRefObject lifetime.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct MeshTriStripsData {
    pub base: MeshGeometryData,
    pub triangle_count_40: u16,
    pub strip_count_44: u16,
    pub strip_lengths_48: Vec<u16>,
    pub strip_points_present_raw: u8,
    pub strip_points_4c: Vec<u16>,
    /// IDA: 0071A18D / 0071A23F retain the ordered group claims for lengths and points.
    /// TODO: [ ] Replace the separate typed vectors with live aliases.
    pub strip_group_arrays: [StreamGroupArray; 2],
}

impl MeshTriStripsData {
    /// IDA: 0071A221 / 0071A224 and 00719DE5 / 00719DE8 wrap both additions in a WORD. Completion: 100%.
    #[must_use]
    fn point_count_word(&self) -> u16 {
        (u32::from(self.triangle_count_40) + 2 * u32::from(self.strip_count_44)) as u16
    }

    /// IDA: 00719D40 (a void virtual, in xOBSE) consumes the low WORD of its u32 index, subtracts each strip's WORD(length-2)
    /// and swaps the first two outputs on odd within-strip indices. Invalid arrays or indices are a host error. Completion:
    /// 100% of the raw topology selection.
    /// TODO: [ ] The original's unchecked pointer and partial-output behaviour.
    pub fn triangle_indices(&self, triangle_index_low16: u32) -> Result<[u16; 3], StreamError> {
        let mut local_index = triangle_index_low16 as u16;
        let mut strip_index = 0u16;
        let mut point_offset = 0usize;
        loop {
            let Some(&length) = self.strip_lengths_48.get(usize::from(strip_index)) else {
                return Err(StreamError("NiTriStrips triangle outside strip lengths"));
            };
            let strip_triangles = length.wrapping_sub(2);
            if local_index < strip_triangles {
                break;
            }
            local_index = local_index.wrapping_sub(strip_triangles);
            point_offset += usize::from(length);
            strip_index = strip_index.wrapping_add(1);
        }
        let first = point_offset + usize::from(local_index);
        let points = &self.strip_points_4c;
        if first > points.len() || points.len() - first < 3 {
            return Err(StreamError("NiTriStrips triangle outside point backing"));
        }
        let (a, b) = if local_index & 1 != 0 {
            (points[first + 1], points[first])
        } else {
            (points[first], points[first + 1])
        };
        Ok([a, b, points[first + 2]])
    }

    /// IDA: 00719DC0 computes +0x40 + 2*+0x44 with a WORD add, then zero-extends it. Completion: 100% of the values.
    /// TODO: [ ] The original's four-output-pointer virtual ABI.
    #[must_use]
    pub fn strip_data(&self) -> MeshTriStripTopologyView<'_> {
        MeshTriStripTopologyView {
            strip_count: self.strip_count_44,
            strip_lengths: &self.strip_lengths_48,
            strip_points: &self.strip_points_4c,
            point_word_count: u32::from(self.point_count_word()),
        }
    }

    /// IDA: 00719F20 adds 2*(stripCount + WORD(tris+2*stripCount)) to the inherited array byte size. Completion: 100% of
    /// the topology addend.
    /// TODO: [ ] The full inherited 007285E0 array-size operation.
    #[must_use]
    pub fn topology_byte_size(&self) -> u32 {
        2 * (u32::from(self.strip_count_44) + u32::from(self.point_count_word()))
    }
}

/// IDA: 0071A18D / 0071A23F claim once before the raw reader call, even for a zero wrapped point count; a null group uses
/// host vectors only. Completion: 100% of the read count and order.
/// TODO: [ ] The original's group and FormHeap aliases.
fn strip_words(
    reader: &mut ByteReader<'_>,
    count: u16,
    group: Option<&mut StreamAllocationGroup>,
    group_array: &mut StreamGroupArray,
) -> Result<Vec<u16>, StreamError> {
    if usize::from(count) > reader.remaining() / 2 {
        return Err(StreamError("truncated NiTriStripsData word array"));
    }
    if let Some(group) = group {
        let width = 2 * u32::from(count);
        let storage = group.allocate(width)?;
        let mut peek = *reader;
        group
            .bytes_mut(storage)
            .copy_from_slice(peek.take(width as usize, TRUNCATED_WORD)?);
        *group_array = storage;
    }
    let mut result = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        result.push(reader.u16(TRUNCATED_WORD)?);
    }
    Ok(result)
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeshTriStripsDataParseResult {
    pub data: MeshTriStripsData,
    pub bytes_consumed: usize,
}

/// IDA: 0071A130 -> 00732E70 -> 00729450, modern >= 0A030005: geometry, triangle count, strip count; only with a nonzero
/// strip count, the lengths, the presence byte (the shared floor is later than its 0A000111 gate), then WORD-wrapped
/// points. No length-sum validation changes the read extent. Completion: 100% of the conditional widths and order.
/// TODO: [ ] Live typed group aliases, old versions, and the original's short-read mutation timing.
pub fn parse_mesh_tri_strips_data_payload(
    bytes: &[u8],
    stream_version: u32,
    mut groups: Option<&mut StreamAllocationGroups>,
) -> Result<MeshTriStripsDataParseResult, StreamError> {
    let prefix = parse_mesh_geometry_data_payload(bytes, stream_version, groups.as_deref_mut())?;
    let mut reader = ByteReader::new(&bytes[prefix.bytes_consumed..]);
    let index = prefix.data.allocation_group_index;
    let mut data = MeshTriStripsData {
        base: prefix.data,
        ..MeshTriStripsData::default()
    };
    data.triangle_count_40 = reader.u16(TRUNCATED_WORD)?;
    data.strip_count_44 = reader.u16(TRUNCATED_WORD)?;
    if data.strip_count_44 != 0 {
        data.strip_lengths_48 = strip_words(
            &mut reader,
            data.strip_count_44,
            claim_group(&mut groups, index),
            &mut data.strip_group_arrays[0],
        )?;
        data.strip_points_present_raw = reader.u8(TRUNCATED_BYTE)?;
        if data.strip_points_present_raw != 0 {
            data.strip_points_4c = strip_words(
                &mut reader,
                data.point_count_word(),
                claim_group(&mut groups, index),
                &mut data.strip_group_arrays[1],
            )?;
        }
    }
    Ok(MeshTriStripsDataParseResult {
        data,
        bytes_consumed: prefix.bytes_consumed + reader.position(),
    })
}

/// IDA: 00719AA0 is shared by TriShape and TriStrips and forwards the NiTriBasedGeom payload unchanged; there is no class
/// suffix. Completion: 100% of the shared modern decoder.
/// TODO: [ ] Shared legacy and shader object behaviour.
pub fn parse_mesh_tri_strips_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<MeshTriShapePayload, StreamError> {
    parse_mesh_tri_shape_payload(bytes, stream_version)
}

/// IDA: 00719AB0 -> 00717900 -> 00723350 with the inherited 007081B0 link published before the +0xB4 data store; preflight
/// precedes every store. Completion: 100% of the inherited and data publication order.
/// TODO: [ ] Skin (unreached in the startup trace) and shader owners.
pub fn resolve_mesh_tri_strips_data_link(
    world: &mut World,
    strips: Entity,
    data: Option<Entity>,
    table: &StreamObjectTable,
    stream_version: u32,
) -> Result<MeshTriShapeLinkStatus, StreamError> {
    let payload = world
        .get::<MeshTriShapePayload>(strips)
        .expect("a NiTriStrips has a payload")
        .clone();
    if data.is_some() != (payload.geometry_data_index != NULL_REFERENCE) {
        return Err(StreamError("NiTriStrips data reference/owner mismatch"));
    }
    let resolved = world
        .get::<NiGeometry>(strips)
        .expect("a NiTriStrips has links")
        .links_resolved;
    // 00700050's +0x0C head: the geometry controller the host binds, NiGeomMorpherController.
    let Some(controller) = super::mesh_geometry::typed_reference(
        world,
        table,
        payload.net.controller_index,
        super::stream_objects::NiClass::GeomMorpherController,
        "geometry controller out of range",
    )?
    else {
        return Ok(MeshTriShapeLinkStatus::UnsupportedOwner);
    };
    if resolved || payload.skin_index != NULL_REFERENCE || payload.shader_flag_raw != 0 {
        return Ok(MeshTriShapeLinkStatus::UnsupportedOwner);
    }
    let plan = plan_av_object_links(
        world,
        &payload.net,
        &payload.property_indices,
        payload.collision_index,
        table,
        stream_version,
    )?;
    if plan.status != AvObjectLinkStatus::Linked {
        return Ok(MeshTriShapeLinkStatus::UnsupportedOwner);
    }
    commit_av_object_links(world, strips, &payload.net, plan, table)?;
    if let Some(mut net) = world.get_mut::<super::object_net::NiObjectNet>(strips) {
        net.controller_0c = controller;
    }
    let mut links = world
        .get_mut::<NiGeometry>(strips)
        .expect("a NiTriStrips has links");
    links.data_b4 = data;
    links.links_resolved = true;
    Ok(MeshTriShapeLinkStatus::Linked)
}

#[cfg(test)]
mod tests {
    use super::super::mesh_geometry::{link_mesh_geometry, spawn_mesh_geometry};
    use super::super::stream_allocation_groups::read_stream_allocation_groups;
    use super::super::stream_objects::NiClass;
    use super::*;

    fn data(lengths: Vec<u16>, points: Vec<u16>, triangles: u16) -> MeshTriStripsData {
        MeshTriStripsData {
            triangle_count_40: triangles,
            strip_count_44: lengths.len() as u16,
            strip_lengths_48: lengths,
            strip_points_4c: points,
            ..MeshTriStripsData::default()
        }
    }

    /// The geometry prefix with no arrays: group 1, no vertices, flags zero, zero bounds.
    fn prefix() -> Vec<u8> {
        let mut bytes = 1u32.to_le_bytes().to_vec();
        bytes.extend([0; 7]); // vertices, keep, compress, positions, flags
        bytes.push(0); // normals
        bytes.extend([0; 16]); // bounds
        bytes.push(0); // colors
        bytes.extend([0, 0]); // dirty
        bytes.extend(NULL_REFERENCE.to_le_bytes());
        bytes
    }

    /// No C++ test covers this directly: 00719D40's walk, reversal and failures.
    #[test]
    fn triangles_walk_strips_and_reverse_odd_indices() {
        let strips = data(vec![4, 3], vec![10, 11, 12, 13, 20, 21, 22], 3);
        assert_eq!(strips.triangle_indices(0).unwrap(), [10, 11, 12]);
        assert_eq!(strips.triangle_indices(1).unwrap(), [12, 11, 13]);
        assert_eq!(strips.triangle_indices(2).unwrap(), [20, 21, 22]);
        assert_eq!(
            strips.triangle_indices(0x0001_0000).unwrap(),
            [10, 11, 12],
            "only the low WORD counts"
        );
        assert!(strips.triangle_indices(3).is_err());
        let short = data(vec![4], vec![1, 2, 3], 2);
        assert!(short.triangle_indices(1).is_err());
    }

    /// No C++ test covers this directly: 00719DC0 / 00719F20's WORD wrap.
    #[test]
    fn point_count_wraps_in_a_word() {
        let wrapped = data(vec![0; 2], Vec::new(), 0xFFFE);
        let topology = wrapped.strip_data();
        assert_eq!(topology.point_word_count, 2, "0xFFFE + 4 wraps");
        assert_eq!(topology.strip_count, 2);
        assert_eq!(wrapped.topology_byte_size(), 8);
        assert_eq!(
            data(vec![4, 3], Vec::new(), 3).topology_byte_size(),
            2 * (2 + 7)
        );
    }

    /// No C++ test covers this directly: 0071A130's gates and group claims.
    #[test]
    fn payload_reads_points_only_behind_both_gates() {
        let mut bytes = prefix();
        let prefix_len = bytes.len();
        bytes.extend(1u16.to_le_bytes()); // triangles
        bytes.extend(1u16.to_le_bytes()); // strips
        bytes.extend(3u16.to_le_bytes()); // length
        bytes.push(2); // raw noncanonical present
        for value in [7u16, 8, 9] {
            bytes.extend(value.to_le_bytes());
        }
        let extent = bytes.len();
        bytes.push(0xAB);
        let mut header = 1u32.to_le_bytes().to_vec();
        header.extend(8u32.to_le_bytes());
        let mut groups = read_stream_allocation_groups(&header, 0x1400_0004).unwrap();
        let parsed =
            parse_mesh_tri_strips_data_payload(&bytes, 0x1400_0004, Some(&mut groups)).unwrap();
        assert_eq!(parsed.bytes_consumed, extent);
        assert_eq!(parsed.data.strip_lengths_48, [3]);
        assert_eq!(parsed.data.strip_points_present_raw, 2);
        assert_eq!(parsed.data.strip_points_4c, [7, 8, 9]);
        let group = groups.resolve(1).unwrap().unwrap();
        assert_eq!(group.allocation_count(), 2);
        assert_eq!(group.used_bytes(), 8);
        assert_eq!(
            group.bytes(parsed.data.strip_group_arrays[1]),
            [7, 0, 8, 0, 9, 0]
        );
        assert!(
            parse_mesh_tri_strips_data_payload(&bytes[..extent - 1], 0x1400_0004, None).is_err()
        );

        let mut empty = prefix();
        empty.extend(0u16.to_le_bytes());
        empty.extend(0u16.to_le_bytes());
        let parsed = parse_mesh_tri_strips_data_payload(&empty, 0x1400_0004, None).unwrap();
        assert_eq!(
            parsed.bytes_consumed,
            prefix_len + 4,
            "no presence byte without strips"
        );
    }

    /// No C++ test covers this directly: NiTriStrips takes its own data class and no skin.
    #[test]
    fn strips_link_their_own_data_class() {
        let mut world = World::new();
        let shape_data = world.spawn(NiClass::TriShapeData).id();
        let strip_data = world.spawn(NiClass::TriStripsData).id();
        let mut table = StreamObjectTable::default();
        table.push("NiTriShapeData", 0, shape_data);
        table.push("NiTriStripsData", 0, strip_data);
        let payload = |index, skin| MeshTriShapePayload {
            geometry_data_index: index,
            skin_index: skin,
            ..MeshTriShapePayload::default()
        };
        let good = spawn_mesh_geometry(&mut world, NiClass::TriStrips, payload(1, NULL_REFERENCE));
        assert_eq!(
            link_mesh_geometry(&mut world, good, &table, 0x1400_0004).unwrap(),
            MeshTriShapeLinkStatus::Linked
        );
        assert_eq!(
            world.get::<NiGeometry>(good).unwrap().data_b4,
            Some(strip_data)
        );
        let wrong = spawn_mesh_geometry(&mut world, NiClass::TriStrips, payload(0, NULL_REFERENCE));
        let skinned = spawn_mesh_geometry(&mut world, NiClass::TriStrips, payload(1, 0));
        for refused in [wrong, skinned] {
            assert_eq!(
                link_mesh_geometry(&mut world, refused, &table, 0x1400_0004).unwrap(),
                MeshTriShapeLinkStatus::UnsupportedOwner
            );
            assert_eq!(
                world.get::<NiGeometry>(refused).unwrap(),
                &NiGeometry::default()
            );
        }
    }
}
