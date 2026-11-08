//! NiGeometryData, NiTriShapeData and NiTriShape: the geometry a mesh emitter samples and a scene node holds.
//!
//! D3D9 stops here: no vertex or index buffer exists, and these arrays are host vectors, not renderer data.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

use super::av_object::{
    AvObjectLinkStatus, AvObjectPrefix, NiAvParent, commit_av_object_links, parse_av_object_prefix,
    plan_av_object_links,
};
use super::mesh_skin_owners::{MeshSkinInstance, MeshSkinResolvedOwner};
use super::mesh_tri_strips::{MeshTriStripsData, resolve_mesh_tri_strips_data_link};
use super::object_net::{NiObjectNet, ObjectNetPending};
use super::particle_mesh_emitter::{
    ParticleMeshEmitter, ParticleMeshGeometryView, ParticleMeshSkinLinkView,
    ParticleMeshSkinPartitionView, ParticleMeshTopology,
};
use super::scene_node::NiAvWorld;
use super::screen_elements::compute_bound_from_vertices;
use super::stream_allocation_groups::{
    StreamAllocationGroup, StreamAllocationGroups, StreamGroupArray,
};
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};
use super::transform::{NiTransform, resolve_scene_relative_transform};

const TRUNCATED: &str = "truncated mesh geometry payload";

/// IDA: 00B27504 word_B27504 (u16, image value 1): 00728670 stores it at +0x0A, then increments it, 16-bit wrap.
/// Completion: 100%.
/// TODO: [ ] Earlier phases' 00728670 callers (15 calls before Phase 08 at runtime, P08-RUN-001: the sun data got
/// 0x10) are not routed through it.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryDataIdCounter(pub u16);

impl Default for GeometryDataIdCounter {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self(1)
    }
}

/// IDA: 00728728 mov dx, word_B27504; 0072872F +0x0A = dx; 00728733 add 1. Completion: 100%.
pub fn allocate_geometry_data_id(world: &mut World) -> u16 {
    let mut counter = world.get_resource_or_insert_with(GeometryDataIdCounter::default);
    let id = counter.0;
    counter.0 = id.wrapping_add(1);
    id
}

/// IDA: 00728770 initializes NiGeometryData inherited by 0071FD30. Completion: 100% of the initialized cold scalars;
/// host-zero bounds stand in for the original's uninitialized +0x0C until LoadBinary overwrites it.
/// TODO: [ ] Allocation-group aliasing and reference counts. `geometry_group_arrays` hold the raw claims of 00729450;
/// the typed vectors below are host storage, not live aliases of them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshGeometryData {
    pub allocation_group_index: u32,
    /// IDA: N/A the group's index when 00712550 resolved one, in place of the original pointer.
    pub allocation_group: Option<u32>,
    pub geometry_group_arrays: [StreamGroupArray; 4],
    pub vertex_count_08: u16,
    /// IDA: 00728670 stores word_B27504 and increments it; the 00728770 stream constructor leaves it unset (host 0).
    pub data_id_0a: u16,
    pub bounds_0c: [f32; 4],
    pub positions_1c: Vec<[f32; 3]>,
    pub normals_20: Vec<[f32; 3]>,
    pub colors_24: Vec<[f32; 4]>,
    pub texture_coordinates_28: Vec<[f32; 2]>,
    pub geometry_flags_2c: u16,
    pub dirty_flags_2e: u16,
    pub keep_flags_30: u8,
    pub compress_flags_31: u8,
    pub positions_present_raw: u8,
    pub normals_present_raw: u8,
    pub colors_present_raw: u8,
    pub deferred_geometry_index: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeshGeometryDataParseResult {
    pub data: MeshGeometryData,
    pub bytes_consumed: usize,
}

/// IDA: N/A the resolved group for one more claim; the index was checked when the payload named it.
pub(super) fn claim_group<'g>(
    groups: &'g mut Option<&mut StreamAllocationGroups>,
    index: u32,
) -> Option<&'g mut StreamAllocationGroup> {
    groups
        .as_deref_mut()
        .and_then(|groups| groups.resolve_mut(index).ok().flatten())
}

/// IDA: 00729450 advances the group before the array read, including a zero-width claim when its presence branch is
/// taken, then reads N floats per entry. Completion: 100% of the host bounds, claim and read order.
/// TODO: [ ] The original's mutable pointer alias and unchecked overflow.
fn float_vectors<const N: usize>(
    reader: &mut ByteReader<'_>,
    count: usize,
    group: Option<&mut StreamAllocationGroup>,
    group_array: &mut StreamGroupArray,
) -> Result<Vec<[f32; N]>, StreamError> {
    if count > reader.remaining() / (N * 4) {
        return Err(StreamError("truncated mesh geometry float array"));
    }
    let width = count * N * 4;
    if let Some(group) = group {
        let storage = group.allocate(width as u32)?;
        let mut peek = *reader;
        group
            .bytes_mut(storage)
            .copy_from_slice(peek.take(width, TRUNCATED)?);
        *group_array = storage;
    }
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        let mut value = [0.0; N];
        for component in &mut value {
            *component = reader.f32(TRUNCATED)?;
        }
        result.push(value);
    }
    Ok(result)
}

// IDA: N/A a helper, with no address of its own.
fn words(reader: &mut ByteReader<'_>, count: usize) -> Result<Vec<u16>, StreamError> {
    if count > reader.remaining() / 2 {
        return Err(StreamError("truncated mesh triangle words"));
    }
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        result.push(reader.u16(TRUNCATED)?);
    }
    Ok(result)
}

/// IDA: 00729450 NiGeometryData modern LoadBinary, before 00732E70 NiTriBasedGeomData's triangle count. Positions,
/// normals, colors and UVs claim from the selected group in that order. Completion: 100% of the supported read widths,
/// order and ending cursor.
/// TODO: [ ] Group allocator aliases, the old < 0A030005 branches and the original's failures.
pub fn parse_mesh_geometry_data_payload(
    bytes: &[u8],
    stream_version: u32,
    mut groups: Option<&mut StreamAllocationGroups>,
) -> Result<MeshGeometryDataParseResult, StreamError> {
    // 00729450 reads the same at 0A020000: its version gates are 0A000002, 0A000110, 0A010072 and 0A030007.
    if stream_version < 0x0A01_0072 {
        return Err(StreamError("legacy NiGeometryData payload unsupported"));
    }
    let mut reader = ByteReader::new(bytes);
    let mut data = MeshGeometryData {
        allocation_group_index: reader.u32(TRUNCATED)?,
        ..MeshGeometryData::default()
    };
    let index = data.allocation_group_index;
    if let Some(groups) = groups.as_deref() {
        data.allocation_group = groups.resolve(index)?.map(|_| index);
    }
    data.vertex_count_08 = reader.u16(TRUNCATED)?;
    let vertices = usize::from(data.vertex_count_08);
    data.keep_flags_30 = reader.u8(TRUNCATED)?;
    data.compress_flags_31 = reader.u8(TRUNCATED)?;
    data.positions_present_raw = reader.u8(TRUNCATED)?;
    if data.positions_present_raw != 0 {
        data.positions_1c = float_vectors::<3>(
            &mut reader,
            vertices,
            claim_group(&mut groups, index),
            &mut data.geometry_group_arrays[0],
        )?;
    }
    data.geometry_flags_2c = reader.u16(TRUNCATED)?;
    let normal_count = vertices
        * if data.geometry_flags_2c & 0xF000 != 0 {
            3
        } else {
            1
        };
    data.normals_present_raw = reader.u8(TRUNCATED)?;
    if data.normals_present_raw != 0 {
        data.normals_20 = float_vectors::<3>(
            &mut reader,
            normal_count,
            claim_group(&mut groups, index),
            &mut data.geometry_group_arrays[1],
        )?;
    }
    for component in &mut data.bounds_0c {
        *component = reader.f32(TRUNCATED)?;
    }
    data.colors_present_raw = reader.u8(TRUNCATED)?;
    if data.colors_present_raw != 0 {
        data.colors_24 = float_vectors::<4>(
            &mut reader,
            vertices,
            claim_group(&mut groups, index),
            &mut data.geometry_group_arrays[2],
        )?;
    }
    let uv_sets = usize::from(data.geometry_flags_2c & 0x3F);
    if uv_sets != 0 {
        data.texture_coordinates_28 = float_vectors::<2>(
            &mut reader,
            vertices * uv_sets,
            claim_group(&mut groups, index),
            &mut data.geometry_group_arrays[3],
        )?;
    }
    data.dirty_flags_2e = reader.u16(TRUNCATED)?;
    if stream_version >= 0x0A03_0007 {
        data.deferred_geometry_index = Some(reader.u32(TRUNCATED)?);
    }
    Ok(MeshGeometryDataParseResult {
        data,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 0071FD30 NiTriShapeData extends 00728770 NiGeometryData. Completion: 100% of the supported inherited fields and
/// the triangle payload.
/// TODO: [ ] The original's pointer and refcount layout.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct MeshTriShapeData {
    pub base: MeshGeometryData,
    pub triangle_count_40: u16,
    pub triangle_list_length_44: u32,
    pub triangle_list_present_raw: u8,
    pub triangle_list_48: Vec<u16>,
    pub segment_count_50: u16,
    pub segments_54: Vec<Vec<u16>>,
}

impl MeshTriShapeData {
    /// IDA: 0071FCC0 GetTriIndices reads three u16 values at 6*index from the +0x48 list, irrespective of segmented
    /// backing. Completion: 100% of the bounded view of materialized entries.
    /// TODO: [ ] The original's unchecked invalid-index behaviour.
    pub fn triangle_indices(&self, index: u16) -> Result<[u16; 3], StreamError> {
        let first = usize::from(index) * 3;
        if first > self.triangle_list_48.len() || self.triangle_list_48.len() - first < 3 {
            return Err(StreamError("mesh triangle index outside materialized list"));
        }
        Ok([
            self.triangle_list_48[first],
            self.triangle_list_48[first + 1],
            self.triangle_list_48[first + 2],
        ])
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeshTriShapeDataParseResult {
    pub data: MeshTriShapeData,
    pub bytes_consumed: usize,
}

/// IDA: 00729450 -> 00732E70 -> 0071FEC0, modern >= 0A030005. 0071FEC0 defaults the list flag to one before 0A000111;
/// the supported floor is later, so exactly one raw byte follows the length. Completion: 100% of the raw widths and
/// order, including the +0x44 length and the +0x50 segmented u16 payloads.
/// TODO: [ ] The legacy geometry and triangle branches, and allocator failures.
pub fn parse_mesh_tri_shape_data_payload(
    bytes: &[u8],
    stream_version: u32,
    groups: Option<&mut StreamAllocationGroups>,
) -> Result<MeshTriShapeDataParseResult, StreamError> {
    let prefix = parse_mesh_geometry_data_payload(bytes, stream_version, groups)?;
    let mut reader = ByteReader::new(&bytes[prefix.bytes_consumed..]);
    let mut data = MeshTriShapeData {
        base: prefix.data,
        ..MeshTriShapeData::default()
    };
    data.triangle_count_40 = reader.u16(TRUNCATED)?;
    data.triangle_list_length_44 = reader.u32(TRUNCATED)?;
    data.triangle_list_present_raw = reader.u8(TRUNCATED)?;
    if data.triangle_list_present_raw != 0 {
        data.triangle_list_48 = words(&mut reader, data.triangle_list_length_44 as usize)?;
    }
    data.segment_count_50 = reader.u16(TRUNCATED)?;
    if usize::from(data.segment_count_50) > reader.remaining() / 2 {
        return Err(StreamError("truncated mesh segment lengths"));
    }
    data.segments_54.reserve(usize::from(data.segment_count_50));
    for _ in 0..data.segment_count_50 {
        let count = reader.u16(TRUNCATED)?;
        data.segments_54
            .push(words(&mut reader, usize::from(count))?);
    }
    Ok(MeshTriShapeDataParseResult {
        data,
        bytes_consumed: prefix.bytes_consumed + reader.position(),
    })
}

/// IDA: 00707F00 and 007234B0's deferred state, shared by NiTriShape and NiTriStrips (00719AA0 forwards both).
/// Cold flags 0x000E: 00722810 calls 00708450, and 00707F00 later replaces the stream bits.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct MeshTriShapePayload {
    pub net: ObjectNetPending,
    pub raw_av_flags: u16,
    pub effective_av_flags: u16,
    pub local_transform: NiTransform,
    pub property_indices: Vec<u32>,
    pub collision_index: u32,
    pub geometry_data_index: u32,
    pub skin_index: u32,
    pub shader_flag_raw: u8,
    pub shader_name: Option<Vec<u8>>,
    pub shader_field_c: u32,
    pub bytes_consumed: usize,
}

impl Default for MeshTriShapePayload {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            net: ObjectNetPending::default(),
            raw_av_flags: 0x000E,
            effective_av_flags: 0x000E,
            local_transform: NiTransform::default(),
            property_indices: Vec::new(),
            collision_index: NULL_REFERENCE,
            geometry_data_index: NULL_REFERENCE,
            skin_index: NULL_REFERENCE,
            shader_flag_raw: 0,
            shader_name: None,
            shader_field_c: 0,
            bytes_consumed: 0,
        }
    }
}

/// IDA: 00707F00 -> 007234B0 (+0x1C through 00719AA0 / 007178F0). The shader name and word are read; no shader owner is
/// fabricated from them. Completion: 100% of the >= 0A010072 raw widths and order.
/// TODO: [ ] The legacy AV branches, and instantiating NiShader at 007234B0 (optional body 007386E0).
pub fn parse_mesh_tri_shape_payload(
    bytes: &[u8],
    stream_version: u32,
) -> Result<MeshTriShapePayload, StreamError> {
    if stream_version < 0x0A01_0072 {
        return Err(StreamError("legacy NiTriShape payload unsupported"));
    }
    let (prefix, mut reader) = parse_av_object_prefix(
        bytes,
        stream_version,
        TRUNCATED,
        "truncated mesh property reference list",
    )?;
    let AvObjectPrefix {
        net,
        raw_av_flags,
        effective_av_flags,
        local_transform,
        property_indices,
        collision_index,
    } = prefix;
    let geometry_data_index = reader.u32(TRUNCATED)?;
    let skin_index = reader.u32(TRUNCATED)?;
    let shader_flag_raw = reader.u8(TRUNCATED)?;
    let mut shader_name = None;
    let mut shader_field_c = 0;
    if shader_flag_raw != 0 {
        // IDA: 00713620 a signed-length owned string: a nonpositive length is null.
        let length = reader.u32(TRUNCATED)? as i32;
        if length > 0 {
            shader_name = Some(reader.take(length as usize, TRUNCATED)?.to_vec());
        }
        shader_field_c = reader.u32(TRUNCATED)?;
    }
    let bytes_consumed = net.bytes_consumed + reader.position();
    Ok(MeshTriShapePayload {
        net,
        raw_av_flags,
        effective_av_flags,
        local_transform,
        property_indices,
        collision_index,
        geometry_data_index,
        skin_index,
        shader_flag_raw,
        shader_name,
        shader_field_c,
        bytes_consumed,
    })
}

/// IDA: 007227B0 NiGeometry's links: +0xB4 data and +0xB8 skin (00723350), +0xAC NiPropertyState (00722B30), +0xB0
/// NiDynamicEffectState (00722C00). The +0xBC NiD3DShader (004EC910) is `bs::sky_shader::NiGeometryShader`, since shaders
/// are not entities.
/// TODO: [ ] The original refcounts; the host NiTriStrips never has a +0xB8 skin.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NiGeometry {
    pub property_state_ac: Option<Entity>,
    pub effect_state_b0: Option<Entity>,
    pub data_b4: Option<Entity>,
    pub skin_b8: Option<Entity>,
    pub links_resolved: bool,
}

/// IDA: 007175C0 / 00719A40 the cold 0xC0 NiTriShape or NiTriStrips through 007226C0 / 00722810. The NiObjectNET base,
/// parent and world state are shared with NiNode; the +0x98 property list arrives with the link.
pub fn spawn_mesh_geometry(
    world: &mut World,
    class: NiClass,
    payload: MeshTriShapePayload,
) -> Entity {
    world
        .spawn((
            class,
            payload,
            NiObjectNet::default(),
            NiAvWorld::default(),
            NiAvParent::default(),
            NiGeometry::default(),
        ))
        .id()
}

/// IDA: 00723350's outcome. An unsupported inherited owner publishes no partial link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshTriShapeLinkStatus {
    Linked,
    UnsupportedOwner,
}

// IDA: N/A a helper, with no address of its own.
fn check_owner_presence(
    payload: &MeshTriShapePayload,
    data: Option<Entity>,
    skin: Option<Entity>,
) -> Result<(), StreamError> {
    if data.is_some() != (payload.geometry_data_index != NULL_REFERENCE) {
        return Err(StreamError("NiTriShape geometry reference/owner mismatch"));
    }
    if skin.is_some() != (payload.skin_index != NULL_REFERENCE) {
        return Err(StreamError("NiTriShape skin reference/owner mismatch"));
    }
    Ok(())
}

/// IDA: 00719AB0 -> 00717900 -> 00723350 without a stream table: refuses a skin reference, then the bounded store below.
/// Completion: 100% of the typed data association after its guards.
/// TODO: [ ] Retire once every caller passes the table.
pub fn resolve_mesh_tri_shape_data_link(
    world: &mut World,
    shape: Entity,
    data: Option<Entity>,
) -> Result<MeshTriShapeLinkStatus, StreamError> {
    let payload = world
        .get::<MeshTriShapePayload>(shape)
        .expect("a NiTriShape has a payload");
    if payload.skin_index != NULL_REFERENCE {
        return Ok(MeshTriShapeLinkStatus::UnsupportedOwner);
    }
    resolve_mesh_tri_shape_links_without_table(world, shape, data, None)
}

/// IDA: 00723350 stores the resolved geometry data +0xB4 before the skin instance +0xB8. Without a table there are no
/// inherited owners, so any extra, controller, collision, shader or property reference is unsupported. Completion: 100%
/// of the store order for the bounded AV prefix.
/// TODO: [ ] The original's refcounts and other NiGeometryData subclasses.
pub fn resolve_mesh_tri_shape_links_without_table(
    world: &mut World,
    shape: Entity,
    data: Option<Entity>,
    skin: Option<Entity>,
) -> Result<MeshTriShapeLinkStatus, StreamError> {
    let payload = world
        .get::<MeshTriShapePayload>(shape)
        .expect("a NiTriShape has a payload");
    check_owner_presence(payload, data, skin)?;
    if !payload.net.extra_indices.is_empty()
        || payload.net.controller_index != NULL_REFERENCE
        || payload.collision_index != NULL_REFERENCE
        || payload.shader_flag_raw != 0
        || payload
            .property_indices
            .iter()
            .any(|&index| index != NULL_REFERENCE)
    {
        return Ok(MeshTriShapeLinkStatus::UnsupportedOwner);
    }
    let mut links = world
        .get_mut::<NiGeometry>(shape)
        .expect("a NiTriShape has links");
    links.data_b4 = data;
    links.skin_b8 = skin;
    links.links_resolved = true;
    Ok(MeshTriShapeLinkStatus::Linked)
}

/// IDA: 00719AB0 -> 00717900 -> 00723350 after the inherited 007081B0 link: extras (00700050), the +0x98 property list and
/// +0xA8 collision are published before the +0xB4 data and +0xB8 skin stores; every owner is preflighted before the first
/// store. A controller head other than NiGeomMorpherController, or a shader body, stays unsupported. Completion: 100% of the
/// order.
/// TODO: [ ] The other geometry controller heads and the optional shader body.
pub fn resolve_mesh_tri_shape_links(
    world: &mut World,
    shape: Entity,
    data: Option<Entity>,
    skin: Option<Entity>,
    table: &StreamObjectTable,
    stream_version: u32,
) -> Result<MeshTriShapeLinkStatus, StreamError> {
    let payload = world
        .get::<MeshTriShapePayload>(shape)
        .expect("a NiTriShape has a payload")
        .clone();
    check_owner_presence(&payload, data, skin)?;
    let resolved = world
        .get::<NiGeometry>(shape)
        .expect("a NiTriShape has links")
        .links_resolved;
    // 00700050's +0x0C head: the geometry controller the host binds, NiGeomMorpherController.
    let Some(controller) = typed_reference(
        world,
        table,
        payload.net.controller_index,
        NiClass::GeomMorpherController,
        "geometry controller out of range",
    )?
    else {
        return Ok(MeshTriShapeLinkStatus::UnsupportedOwner);
    };
    if resolved || payload.shader_flag_raw != 0 {
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
    commit_av_object_links(world, shape, &payload.net, plan, table)?;
    if let Some(mut net) = world.get_mut::<super::object_net::NiObjectNet>(shape) {
        net.controller_0c = controller;
    }
    let mut links = world
        .get_mut::<NiGeometry>(shape)
        .expect("a NiTriShape has links");
    links.data_b4 = data;
    links.skin_b8 = skin;
    links.links_resolved = true;
    Ok(MeshTriShapeLinkStatus::Linked)
}

/// IDA: N/A the C++ stream graph's typed lookup: `None` is a bound object of another class, the graph's unsupported owner.
pub(super) fn typed_reference(
    world: &World,
    table: &StreamObjectTable,
    index: u32,
    class: NiClass,
    out_of_range: &'static str,
) -> Result<Option<Option<Entity>>, StreamError> {
    if index == NULL_REFERENCE {
        return Ok(Some(None));
    }
    let entry = table
        .objects
        .get(index as usize)
        .ok_or(StreamError(out_of_range))?;
    Ok((world.get::<NiClass>(entry.entity) == Some(&class)).then_some(Some(entry.entity)))
}

/// IDA: 00719AB0's link as the stream graph runs it: the +0xB4 and +0xB8 references resolved through the table with a
/// class check, then the class's own link. NiTriStrips takes NiTriStripsData and no skin. Completion: 100% of the checks.
pub fn link_mesh_geometry(
    world: &mut World,
    geometry: Entity,
    table: &StreamObjectTable,
    stream_version: u32,
) -> Result<MeshTriShapeLinkStatus, StreamError> {
    let class = world.get::<NiClass>(geometry).copied();
    let payload = world
        .get::<MeshTriShapePayload>(geometry)
        .ok_or(StreamError("geometry payload owner mismatch"))?;
    let (data_index, skin_index) = (payload.geometry_data_index, payload.skin_index);
    match class {
        Some(NiClass::TriStrips) => {
            let Some(data) = typed_reference(
                world,
                table,
                data_index,
                NiClass::TriStripsData,
                "NiTriStrips data index out of range",
            )?
            else {
                return Ok(MeshTriShapeLinkStatus::UnsupportedOwner);
            };
            resolve_mesh_tri_strips_data_link(world, geometry, data, table, stream_version)
        }
        Some(NiClass::TriShape) => {
            let Some(data) = typed_reference(
                world,
                table,
                data_index,
                NiClass::TriShapeData,
                "NiTriShape geometry index out of range",
            )?
            else {
                return Ok(MeshTriShapeLinkStatus::UnsupportedOwner);
            };
            let Some(skin) = typed_reference(
                world,
                table,
                skin_index,
                NiClass::SkinInstance,
                "NiTriShape skin index out of range",
            )?
            else {
                return Ok(MeshTriShapeLinkStatus::UnsupportedOwner);
            };
            resolve_mesh_tri_shape_links(world, geometry, data, skin, table, stream_version)
        }
        _ => Err(StreamError("geometry payload owner mismatch")),
    }
}

/// IDA: N/A the NiGeometryData base of a data entity, whichever subclass holds it.
#[must_use]
pub fn geometry_data_base(world: &World, data: Entity) -> Option<&MeshGeometryData> {
    world
        .get::<MeshTriShapeData>(data)
        .map(|owner| &owner.base)
        .or_else(|| {
            world
                .get::<MeshTriStripsData>(data)
                .map(|owner| &owner.base)
        })
}

/// IDA: 0074C910 reads the geometry's current +0xB4 data pointer and +0xB8 skin now. A forward cold shape is retained with
/// null data, and later shape links do not change this snapshot. Completion: 100% of the observed fields.
#[must_use]
pub fn mesh_skin_link_view(world: &World, geometry: Entity) -> ParticleMeshSkinLinkView {
    let links = world
        .get::<NiGeometry>(geometry)
        .copied()
        .unwrap_or_default();
    let data = links
        .data_b4
        .and_then(|data| geometry_data_base(world, data));
    let instance = links
        .skin_b8
        .and_then(|skin| world.get::<MeshSkinInstance>(skin));
    ParticleMeshSkinLinkView {
        geometry_present: true,
        skin_instance_present: links.skin_b8.is_some(),
        skin_data_present: instance.is_some_and(|instance| instance.data_08.is_some()),
        geometry_data_present: links.data_b4.is_some(),
        geometry_keep_flags: data.map_or(0, |data| data.keep_flags_30),
        partition_owner: instance.and_then(|instance| instance.partition_0c),
        skin_instance_owner: links.skin_b8,
    }
}

/// IDA: 0074C910's write to NiGeometryData +0x30, after this geometry and its cache are published and before the next
/// slot. Completion: 100%.
pub fn mark_mesh_emitter_keep_flags(world: &mut World, data: Entity) {
    if let Some(mut owner) = world.get_mut::<MeshTriShapeData>(data) {
        owner.base.keep_flags_30 |= 0x33;
    } else if let Some(mut owner) = world.get_mut::<MeshTriStripsData>(data) {
        owner.base.keep_flags_30 |= 0x33;
    }
}

/// IDA: 0074BCD0 / 0074AD30 consume NiTriShapeData +0x08, +0x1C / +0x20 and 0071FCC0's triples. The arrays stay on the
/// data entity; the triples and a resolved skin are held here. Completion: 100% of the view's inputs.
/// TODO: [ ] The original's scene world epoch.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshTriShapeEmitterView {
    pub data: Entity,
    pub triangles: Vec<[u16; 3]>,
    pub mesh_to_target: NiTransform,
    pub skin_owner: Option<MeshSkinResolvedOwner>,
}

impl MeshTriShapeEmitterView {
    /// IDA: N/A the partition views the skin refers to, built first because a view cannot hold its own storage. Empty
    /// without a skin. Completion: 100%.
    #[must_use]
    pub fn partition_views<'w>(&self, world: &'w World) -> Vec<ParticleMeshSkinPartitionView<'w>> {
        self.skin_owner
            .as_ref()
            .map_or_else(Vec::new, |owner| owner.partition_views(world))
    }

    /// IDA: 0074BCD0 uses three u16 indices per materialized triangle while 0074AD30 consumes the raw vertex and normal
    /// arrays. `partitions` comes from `partition_views`. Completion: 100%.
    /// TODO: [ ] Actual relative world frames and skinned source arrays.
    #[must_use]
    pub fn view<'a>(
        &'a self,
        world: &'a World,
        partitions: &'a [ParticleMeshSkinPartitionView<'a>],
    ) -> ParticleMeshGeometryView<'a> {
        let Some(data) = world.get::<MeshTriShapeData>(self.data) else {
            return ParticleMeshGeometryView::default();
        };
        ParticleMeshGeometryView {
            vertex_count: data.base.vertex_count_08,
            vertices: &data.base.positions_1c,
            normals: &data.base.normals_20,
            topology: ParticleMeshTopology::Triangles,
            triangles: &self.triangles,
            mesh_to_target: self.mesh_to_target,
            skinned: self.skin_owner.is_some(),
            skin: self
                .skin_owner
                .as_ref()
                .map(|owner| owner.view(world, partitions)),
        }
    }
}

/// IDA: 0071FCC0 reads 6*index from +0x48; 0074BCD0 consumes these in source order for NiTriShapeData. Completion: 100%
/// of the topology of a linked triangle list.
/// TODO: [ ] The original's zero-triangle mode, and NiTriStripsData separately.
fn materialize_emitter_view(
    world: &World,
    links: &NiGeometry,
    mesh_to_target: NiTransform,
) -> Result<MeshTriShapeEmitterView, StreamError> {
    const UNLINKED: &str = "NiTriShape geometry data is not linked";
    let data_entity = links.data_b4.ok_or(StreamError(UNLINKED))?;
    if !links.links_resolved {
        return Err(StreamError(UNLINKED));
    }
    let data = world
        .get::<MeshTriShapeData>(data_entity)
        .ok_or(StreamError(UNLINKED))?;
    let triangle_count = usize::from(data.triangle_count_40);
    if triangle_count > data.triangle_list_48.len() / 3 {
        return Err(StreamError(
            "NiTriShapeData triangle list is not materialized",
        ));
    }
    let mut triangles = Vec::with_capacity(triangle_count);
    for index in 0..data.triangle_count_40 {
        triangles.push(data.triangle_indices(index)?);
    }
    Ok(MeshTriShapeEmitterView {
        data: data_entity,
        triangles,
        mesh_to_target,
        skin_owner: None,
    })
}

/// IDA: 0074C770 / 0074BCD0 use the linked NiTriShapeData owner for emission. Completion: 100% of a non-skinned shape.
pub fn make_mesh_tri_shape_emitter_view(
    world: &World,
    shape: Entity,
    mesh_to_target: NiTransform,
) -> Result<MeshTriShapeEmitterView, StreamError> {
    let links = world
        .get::<NiGeometry>(shape)
        .copied()
        .ok_or(StreamError("NiTriShape geometry data is not linked"))?;
    if links.links_resolved && links.data_b4.is_some() && links.skin_b8.is_some() {
        return Err(StreamError(
            "NiTriShape skinned view requires cached skin owner",
        ));
    }
    materialize_emitter_view(world, &links, mesh_to_target)
}

/// IDA: 0074C910 captures NiSkinInstance and its partition at LinkObject; 0074BCD0 later consumes that partition with
/// live root and bone frames. Completion: 100% of the owner identity and triangle materialization.
/// TODO: [ ] The original's lazy 0072ED50 partition path and unchecked indices.
pub fn make_skinned_mesh_tri_shape_emitter_view(
    world: &World,
    shape: Entity,
    mesh_to_target: NiTransform,
    skin_owner: MeshSkinResolvedOwner,
) -> Result<MeshTriShapeEmitterView, StreamError> {
    let links = world
        .get::<NiGeometry>(shape)
        .copied()
        .ok_or(StreamError("NiTriShape cached skin owner mismatch"))?;
    if links.skin_b8 != Some(skin_owner.instance) {
        return Err(StreamError("NiTriShape cached skin owner mismatch"));
    }
    let mut result = materialize_emitter_view(world, &links, mesh_to_target)?;
    result.skin_owner = Some(skin_owner);
    Ok(result)
}

/// IDA: 0074C770 consumes a linked mesh slot, then 0074A0C3 / 0074A0DE snapshot the source and target world frames before
/// 00718A80 and 0053D7A0. The C++ `make_stream_mesh_emitter_view` takes explicit frames for those; here they are the
/// entities' `NiAvWorld`, so a stale update is the original's stale update. Only a NiTriShape source is viewable, as in
/// C++. Completion: 100% of the owner and slot checks.
/// TODO: [ ] 0074A2D0's cache fill and 0072ED50 lazy generation: a skinned slot with no cached partition is a host stop.
pub fn make_stream_mesh_emitter_view(
    world: &World,
    emitter: Entity,
    linked_mesh_slot: usize,
    table: &StreamObjectTable,
) -> Result<MeshTriShapeEmitterView, StreamError> {
    const UNRESOLVED: &str = "mesh emitter frame owners unresolved";
    let owner = world
        .get::<ParticleMeshEmitter>(emitter)
        .ok_or(StreamError("mesh emitter linked slot unavailable"))?;
    let source = *owner
        .linked_geometry_54
        .get(linked_mesh_slot)
        .ok_or(StreamError("mesh emitter linked slot unavailable"))?;
    let target = table
        .objects
        .get(owner.state.base.target_index as usize)
        .ok_or(StreamError("mesh emitter target index out of range"))?
        .entity;
    let links = world
        .get::<NiGeometry>(source)
        .copied()
        .ok_or(StreamError(UNRESOLVED))?;
    if world.get::<NiClass>(source) != Some(&NiClass::TriShape)
        || world.get::<NiClass>(target) != Some(&NiClass::ParticleSystem)
        || !links.links_resolved
        || links.data_b4.is_none()
    {
        return Err(StreamError(UNRESOLVED));
    }
    let source_world = world
        .get::<NiAvWorld>(source)
        .ok_or(StreamError(UNRESOLVED))?;
    let target_world = world
        .get::<NiAvWorld>(target)
        .ok_or(StreamError(UNRESOLVED))?;
    let relative =
        resolve_scene_relative_transform(&source_world.transform, &target_world.transform);
    let Some(skin) = links.skin_b8 else {
        return make_mesh_tri_shape_emitter_view(world, source, relative);
    };
    let cached = owner
        .skin_cache
        .get(linked_mesh_slot)
        .ok_or(StreamError("mesh emitter skin scene/cache unavailable"))?;
    if cached.skin_instance_owner != Some(skin) || cached.partition_owner.is_none() {
        return Err(StreamError(
            "mesh emitter cached skin partition unavailable",
        ));
    }
    let skin_owner = super::mesh_skin_owners::make_mesh_skin_resolved_owner_with_partition(
        world,
        Some(skin),
        cached.partition_owner,
    )?;
    make_skinned_mesh_tri_shape_emitter_view(world, source, relative, skin_owner)
}

/// IDA: 0071FB40 NiTriShapeData(vertexCount, positions, normals, colors, uvs, textureSets, dataFlags, triangleCount,
/// triangles) over 00732D70 (+0x40) over 00728670: +0x08 count, +0x1C positions, +0x20 normals, +0x24 colors, +0x28 uvs,
/// +0x2C..+0x34 = 0, +0x2C = dataFlags | (textureSets & 0x3F) | (+0x2C & 0xFC0), the 0072A0F0 bound when there are vertices
/// and positions, +0x0A = word_B27504++, +0x2E &= 0x0FFF; then +0x48 triangles, +0x50 = 0 and +0x44 = 3 * count. The arrays
/// are owned here (FormHeap blocks in the original). Completion: 100%.
#[allow(
    clippy::too_many_arguments,
    reason = "0071FB40's own nine constructor arguments, plus the world holding word_B27504"
)]
pub fn construct_tri_shape_data(
    world: &mut World,
    vertex_count: u16,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    texture_coordinates: Vec<[f32; 2]>,
    texture_sets: u8,
    data_flags: u16,
    triangle_count: u16,
    triangles: Vec<u16>,
) -> Entity {
    let mut data = MeshTriShapeData::default();
    // 00728699..007286D6.
    data.base.vertex_count_08 = vertex_count;
    data.base.positions_1c = positions;
    data.base.normals_20 = normals;
    data.base.colors_24 = colors;
    data.base.texture_coordinates_28 = texture_coordinates;
    // 007286F8.
    data.base.geometry_flags_2c =
        data_flags | u16::from(texture_sets & 0x3F) | (data.base.geometry_flags_2c & 0x0FC0);
    // 00728711: the bound only with vertices and positions.
    if vertex_count != 0 && !data.base.positions_1c.is_empty() {
        let (center, radius) =
            compute_bound_from_vertices(&data.base.positions_1c, usize::from(vertex_count));
        data.base.bounds_0c = [center[0], center[1], center[2], radius];
    }
    // 0072872F, 0072873B.
    data.base.data_id_0a = allocate_geometry_data_id(world);
    data.base.dirty_flags_2e &= 0x0FFF;
    // 00732DA8; 0071FB7D; 0071FB92.
    data.triangle_count_40 = triangle_count;
    data.triangle_list_present_raw = u8::from(!triangles.is_empty());
    data.triangle_list_48 = triangles;
    data.segment_count_50 = 0;
    data.triangle_list_length_44 = 3 * u32::from(triangle_count);
    world.spawn((NiClass::TriShapeData, data)).id()
}

/// IDA: 007174B0 NiTriShape(data...): the FormHeap(0x58) data above first, then 00722690 over 007227B0 over 00708450
/// (flags 0x000E, identity transforms, zero bound), +0xAC / +0xB0 null, +0xB4 = data, +0xB8 / +0xBC null. Completion: 100%.
pub fn construct_tri_shape(world: &mut World, data: Option<Entity>) -> Entity {
    let shape = spawn_mesh_geometry(world, NiClass::TriShape, MeshTriShapePayload::default());
    world
        .get_mut::<NiGeometry>(shape)
        .expect("just spawned")
        .data_b4 = data;
    shape
}

#[cfg(test)]
mod tests {
    use super::super::mesh_skin_owners::{
        MeshSkinData, MeshSkinInstancePayload, bind_mesh_skin_instance_data_partition,
        bind_mesh_skin_instance_scene_owners,
    };
    use super::super::mesh_skin_partition::{MeshSkinPartition, MeshSkinPartitionRecord};
    use super::super::particle_mesh_emitter::{
        ParticleMeshEmitterPending, link_particle_mesh_emitter,
    };
    use super::*;

    fn word(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend(value.to_le_bytes());
    }

    fn dword(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }

    /// The one-vertex data of `verify_tri_shape_data_and_owner`, with where its deferred reference and end lie.
    fn tri_shape_data_bytes() -> (Vec<u8>, usize, usize) {
        let mut bytes = Vec::new();
        dword(&mut bytes, 5); // allocation group
        word(&mut bytes, 1); // one vertex
        bytes.push(0xA5); // keep flags
        bytes.push(0x5A); // compress flags
        bytes.push(2); // raw noncanonical true positions
        for value in [0x3F80_0000, 0x4000_0000, 0x4040_0000] {
            dword(&mut bytes, value);
        }
        word(&mut bytes, 1); // one UV set
        bytes.push(0); // no normals
        for _ in 0..4 {
            dword(&mut bytes, 0);
        }
        bytes.push(0); // no colors
        dword(&mut bytes, 0x3F00_0000);
        dword(&mut bytes, 0x3E80_0000); // UV pair
        word(&mut bytes, 0x8000); // dirty flags
        let deferred_at = bytes.len();
        dword(&mut bytes, NULL_REFERENCE); // deferred geometry null
        word(&mut bytes, 1); // triangle count
        dword(&mut bytes, 3); // triangle list length, not a u16
        bytes.push(1);
        for value in [0, 0, 0] {
            word(&mut bytes, value);
        }
        word(&mut bytes, 0); // no segments
        let extent = bytes.len();
        bytes.push(0xAB);
        (bytes, deferred_at, extent)
    }

    fn tri_shape_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        dword(&mut bytes, 0); // empty NET name
        dword(&mut bytes, 0); // extras count
        dword(&mut bytes, NULL_REFERENCE); // controller null
        word(&mut bytes, 0x0040); // raw AV flags
        for index in 0..3 {
            dword(&mut bytes, if index == 0 { 0x4000_0000 } else { 0 });
        }
        for index in 0..9 {
            dword(&mut bytes, if index % 4 == 0 { 0x3F80_0000 } else { 0 });
        }
        dword(&mut bytes, 0x3F80_0000);
        dword(&mut bytes, 0); // properties
        dword(&mut bytes, NULL_REFERENCE); // collision
        dword(&mut bytes, 1); // data
        dword(&mut bytes, NULL_REFERENCE); // skin
        bytes.push(0); // shader absent
        bytes
    }

    /// Ported from `gamebryo_mesh_owners` `verify_tri_shape_data_and_owner`.
    #[test]
    fn tri_shape_data_and_owner() {
        let (bytes, deferred_at, extent) = tri_shape_data_bytes();
        let parsed = parse_mesh_tri_shape_data_payload(&bytes, 0x1400_0004, None).unwrap();
        assert_eq!(parsed.bytes_consumed, extent);
        assert_eq!(parsed.data.base.allocation_group_index, 5);
        assert_eq!(parsed.data.base.keep_flags_30, 0xA5);
        assert_eq!(parsed.data.base.compress_flags_31, 0x5A);
        assert_eq!(parsed.data.base.positions_present_raw, 2);
        assert_eq!(parsed.data.base.positions_1c, [[1.0, 2.0, 3.0]]);
        assert_eq!(parsed.data.base.texture_coordinates_28[0], [0.5, 0.25]);
        assert_eq!(parsed.data.triangle_list_length_44, 3);
        assert_eq!(parsed.data.triangle_indices(0).unwrap(), [0, 0, 0]);
        assert!(
            parse_mesh_tri_shape_data_payload(&bytes[..extent - 1], 0x1400_0004, None).is_err(),
            "truncated TriShapeData segment tail accepted"
        );
        let mut older = bytes[..deferred_at].to_vec();
        older.extend(&bytes[deferred_at + 4..extent]);
        let old = parse_mesh_tri_shape_data_payload(&older, 0x0A03_0005, None).unwrap();
        assert_eq!(old.bytes_consumed, extent - 4);
        assert!(old.data.base.deferred_geometry_index.is_none());
        assert_eq!(old.data.triangle_count_40, 1);

        let shape_bytes = tri_shape_bytes();
        let payload = parse_mesh_tri_shape_payload(&shape_bytes, 0x1400_0004).unwrap();
        assert_eq!(payload.bytes_consumed, shape_bytes.len());
        assert_eq!(payload.raw_av_flags, 0x40);
        assert_eq!(payload.effective_av_flags, 0x4E);
        assert_eq!(payload.local_transform.translation[0], 2.0);
        assert_eq!(payload.geometry_data_index, 1);
        let old_shape = parse_mesh_tri_shape_payload(&shape_bytes, 0x0A01_0072).unwrap();
        assert_eq!(old_shape.raw_av_flags, 0x40);
        assert_eq!(old_shape.effective_av_flags, 0x0E);
        assert!(
            parse_mesh_tri_shape_payload(&shape_bytes[..shape_bytes.len() - 1], 0x1400_0004)
                .is_err(),
            "truncated TriShape shader-flag tail accepted"
        );

        let mut world = World::new();
        let shape = spawn_mesh_geometry(&mut world, NiClass::TriShape, payload);
        let data = world.spawn((NiClass::TriShapeData, parsed.data)).id();
        assert_eq!(
            resolve_mesh_tri_shape_data_link(&mut world, shape, Some(data)).unwrap(),
            MeshTriShapeLinkStatus::Linked
        );
        let view = make_mesh_tri_shape_emitter_view(&world, shape, NiTransform::default()).unwrap();
        assert_eq!(view.data, data);
        let partitions = view.partition_views(&world);
        let sampled = view.view(&world, &partitions);
        assert_eq!(sampled.vertex_count, 1);
        assert_eq!(sampled.triangles.len(), 1);
        assert_eq!(sampled.vertices[0][1], 2.0);
    }

    /// Beyond the C++ test: 00729450's group claims, in positions then UV order, with an exact byte copy.
    #[test]
    fn geometry_arrays_claim_their_group_in_order() {
        let (mut bytes, _, _) = tri_shape_data_bytes();
        bytes[0..4].copy_from_slice(&1u32.to_le_bytes());
        let mut header = 1u32.to_le_bytes().to_vec();
        header.extend(20u32.to_le_bytes());
        let mut groups = super::super::stream_allocation_groups::read_stream_allocation_groups(
            &header,
            0x1400_0004,
        )
        .unwrap();
        let parsed =
            parse_mesh_tri_shape_data_payload(&bytes, 0x1400_0004, Some(&mut groups)).unwrap();
        assert_eq!(parsed.data.base.allocation_group, Some(1));
        let group = groups.resolve(1).unwrap().unwrap();
        assert_eq!(group.used_bytes(), 20);
        assert_eq!(group.allocation_count(), 2);
        let arrays = parsed.data.base.geometry_group_arrays;
        assert_eq!(arrays[0].size, 12);
        assert_eq!(arrays[3].offset, 12);
        assert_eq!(group.bytes(arrays[3]), [0, 0, 0, 0x3F, 0, 0, 0x80, 0x3E]);
    }

    /// 0071FB40's constructor: the data ID counter, the flags and the bound.
    #[test]
    fn construction_takes_the_next_data_id() {
        let mut world = World::new();
        let make = |world: &mut World| {
            construct_tri_shape_data(
                world,
                2,
                vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
                Vec::new(),
                Vec::new(),
                Vec::new(),
                0x41,
                0x1000,
                1,
                vec![0, 1, 1],
            )
        };
        let (first, second) = (make(&mut world), make(&mut world));
        let data = world.get::<MeshTriShapeData>(first).unwrap();
        assert_eq!(data.base.data_id_0a, 1);
        assert_eq!(data.base.geometry_flags_2c, 0x1001);
        assert_eq!(data.base.bounds_0c, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(data.triangle_list_length_44, 3);
        assert_eq!(data.triangle_list_present_raw, 1);
        assert_eq!(
            world
                .get::<MeshTriShapeData>(second)
                .unwrap()
                .base
                .data_id_0a,
            2
        );
        world.insert_resource(GeometryDataIdCounter(0xFFFF));
        assert_eq!(allocate_geometry_data_id(&mut world), 0xFFFF);
        assert_eq!(allocate_geometry_data_id(&mut world), 0);
        let shape = construct_tri_shape(&mut world, Some(first));
        assert_eq!(world.get::<NiGeometry>(shape).unwrap().data_b4, Some(first));
        assert_eq!(
            world
                .get::<MeshTriShapePayload>(shape)
                .unwrap()
                .effective_av_flags,
            0x000E
        );
    }

    /// Beyond the C++ test: the graph's typed link and preflight, on the bound classes.
    #[test]
    fn world_link_checks_classes_and_publishes_in_order() {
        let mut world = World::new();
        let property = world.spawn(NiClass::AlphaProperty).id();
        let data = world
            .spawn((NiClass::TriShapeData, MeshTriShapeData::default()))
            .id();
        let wrong = world.spawn(NiClass::TriStripsData).id();
        let mut table = StreamObjectTable::default();
        table.push("NiAlphaProperty", 0, property);
        table.push("NiTriShapeData", 0, data);
        table.push("NiTriStripsData", 0, wrong);
        let payload = MeshTriShapePayload {
            property_indices: vec![0],
            geometry_data_index: 1,
            ..MeshTriShapePayload::default()
        };
        let shape = spawn_mesh_geometry(&mut world, NiClass::TriShape, payload.clone());
        assert_eq!(
            link_mesh_geometry(&mut world, shape, &table, 0x1400_0004).unwrap(),
            MeshTriShapeLinkStatus::Linked
        );
        assert_eq!(world.get::<NiGeometry>(shape).unwrap().data_b4, Some(data));
        assert_eq!(
            world
                .get::<super::super::av_object::NiAvProperties>(shape)
                .unwrap()
                .0,
            [Some(property)]
        );
        assert_eq!(
            link_mesh_geometry(&mut world, shape, &table, 0x1400_0004).unwrap(),
            MeshTriShapeLinkStatus::UnsupportedOwner,
            "a relink is refused"
        );
        let mistyped = spawn_mesh_geometry(
            &mut world,
            NiClass::TriShape,
            MeshTriShapePayload {
                geometry_data_index: 2,
                ..payload.clone()
            },
        );
        assert_eq!(
            link_mesh_geometry(&mut world, mistyped, &table, 0x1400_0004).unwrap(),
            MeshTriShapeLinkStatus::UnsupportedOwner
        );
        assert_eq!(
            world.get::<NiGeometry>(mistyped).unwrap(),
            &NiGeometry::default()
        );
        let far = spawn_mesh_geometry(
            &mut world,
            NiClass::TriShape,
            MeshTriShapePayload {
                geometry_data_index: 9,
                ..payload
            },
        );
        assert!(link_mesh_geometry(&mut world, far, &table, 0x1400_0004).is_err());
    }

    /// Beyond the C++ test: `make_stream_mesh_emitter_view` and 0074C910's snapshot on real geometry entities.
    #[test]
    fn mesh_emitter_links_real_geometry() {
        let mut world = World::new();
        let (bytes, _, _) = tri_shape_data_bytes();
        let parsed = parse_mesh_tri_shape_data_payload(&bytes, 0x1400_0004, None).unwrap();
        let data = world.spawn((NiClass::TriShapeData, parsed.data)).id();
        let system = world
            .spawn((NiClass::ParticleSystem, NiAvWorld::default()))
            .id();
        let bone = world.spawn((NiClass::Node, NiAvWorld::default())).id();
        let skin_data = world
            .spawn((
                NiClass::SkinData,
                MeshSkinData {
                    bones: vec![Default::default()],
                    ..MeshSkinData::default()
                },
            ))
            .id();
        let partition = world
            .spawn((
                NiClass::SkinPartition,
                MeshSkinPartition {
                    partitions: vec![MeshSkinPartitionRecord::default()],
                },
            ))
            .id();
        let mut instance = MeshSkinInstance {
            pending: MeshSkinInstancePayload {
                data_index: 3,
                partition_index: 4,
                root_parent_index: 5,
                bone_indices: vec![5],
                bytes_consumed: 0,
            },
            ..MeshSkinInstance::default()
        };
        bind_mesh_skin_instance_data_partition(&mut instance, Some(skin_data), Some(partition), 8)
            .unwrap();
        bind_mesh_skin_instance_scene_owners(&mut instance, Some(bone), vec![Some(bone)], 8)
            .unwrap();
        let skin = world.spawn((NiClass::SkinInstance, instance)).id();
        let plain_payload = MeshTriShapePayload {
            geometry_data_index: 0,
            ..MeshTriShapePayload::default()
        };
        let plain = spawn_mesh_geometry(&mut world, NiClass::TriShape, plain_payload.clone());
        let skinned = spawn_mesh_geometry(
            &mut world,
            NiClass::TriShape,
            MeshTriShapePayload {
                skin_index: 2,
                ..plain_payload
            },
        );
        let emitter = world.spawn(ParticleMeshEmitter::default()).id();
        let mut table = StreamObjectTable::default();
        for (name, entity) in [
            ("NiTriShapeData", data),
            ("NiPSysMeshEmitter", emitter),
            ("NiSkinInstance", skin),
            ("NiSkinData", skin_data),
            ("NiSkinPartition", partition),
            ("NiNode", bone),
            ("NiTriShape", plain),
            ("NiParticleSystem", system),
        ] {
            table.push(name, 0, entity);
        }
        table.push("NiTriShape", 0, skinned);
        let mut pending = ParticleMeshEmitterPending {
            mesh_indices: vec![0],
            ..ParticleMeshEmitterPending::default()
        };
        pending.base.base.target_index = 7;
        assert_eq!(
            link_particle_mesh_emitter(&mut world, emitter, &pending, &table).unwrap(),
            MeshTriShapeLinkStatus::UnsupportedOwner,
            "slot 0 is data, not a geometry"
        );
        assert!(
            world
                .get::<ParticleMeshEmitter>(emitter)
                .unwrap()
                .raw_mesh_indices
                .is_empty()
        );
        pending.mesh_indices = vec![6, NULL_REFERENCE, 8];
        for geometry in [plain, skinned] {
            assert_eq!(
                link_mesh_geometry(&mut world, geometry, &table, 0x1400_0004).unwrap(),
                MeshTriShapeLinkStatus::Linked
            );
        }
        assert_eq!(
            link_particle_mesh_emitter(&mut world, emitter, &pending, &table).unwrap(),
            MeshTriShapeLinkStatus::Linked
        );
        let owner = world.get::<ParticleMeshEmitter>(emitter).unwrap();
        assert_eq!(owner.mesh_indices, [6, 8]);
        assert_eq!(owner.linked_geometry_54, [plain, skinned]);
        assert_eq!(owner.skin_cache[1].partition_owner, Some(partition));
        assert_eq!(owner.skin_cache[0].geometry_keep_flags_after, Some(0xB7));
        assert_eq!(
            world
                .get::<MeshTriShapeData>(data)
                .unwrap()
                .base
                .keep_flags_30,
            0xB7,
            "0xA5 | 0x33, written after the first slot, seen by the second"
        );

        let view = make_stream_mesh_emitter_view(&world, emitter, 0, &table).unwrap();
        assert!(view.skin_owner.is_none());
        let partitions = view.partition_views(&world);
        assert_eq!(view.view(&world, &partitions).vertices[0], [1.0, 2.0, 3.0]);
        let view = make_stream_mesh_emitter_view(&world, emitter, 1, &table).unwrap();
        let partitions = view.partition_views(&world);
        assert_eq!(partitions.len(), 1);
        let sampled = view.view(&world, &partitions);
        assert!(sampled.skinned);
        assert_eq!(sampled.skin.unwrap().partitions.len(), 1);
        assert!(make_stream_mesh_emitter_view(&world, emitter, 2, &table).is_err());
    }
}
