//! NiPSysMeshEmitter, with the geometry and skin views it samples from.
//!
//! The views are explicit resolved input, as in C++; `mesh_geometry` fills them from the geometry classes, and
//! `link_particle_mesh_emitter` links the slots.
//! x87: every float operation here is a 24-bit site (main thread after CreateDevice, control word 007F, P04-RUN-002): plain
//! `f32` in the instructions' grouping. Qword constants hold exact values, so `(f64::from(x) op C) as f32` is one rounding.

#![allow(
    clippy::if_same_then_else,
    reason = "0074AE30, 0074B3C0, 0074B7A0 and 0074BCD0 swap x87 operands by component; each order is kept"
)]

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::mesh_geometry::{
    MeshTriShapeLinkStatus, MeshTriShapePayload, NiGeometry, mark_mesh_emitter_keep_flags,
    mesh_skin_link_view,
};
use super::particle_emitter::{ParticleEmitterPending, parse_emitter_scalars};
use super::registration::SqrtMantissaTable;
use super::stream_objects::{NULL_REFERENCE, NiClass, StreamObjectTable};
use super::transform::{
    NiTransform, compose_scene_transform, invert_scene_transform_checked, normalize_vector,
    rotate_vector, transform_point,
};

const TRUNCATED: &str = "truncated particle mesh emitter payload";

/// IDA: float bits 0x3EAAAAAB, the third that 0074BCD0 stores.
const THIRD_FACTOR: f32 = f32::from_bits(0x3EAA_AAAB);

/// IDA: 004BFAA0 requires the table published by 00725860 only for a nonzero stored squared length. Completion: 100% of
/// the checked initialization boundary.
/// TODO: [ ] Verify runtime float bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticleMeshNormalizeResult {
    Normalized,
    MissingTable,
}

/// IDA: 004BFAA0 stores y*y+x*x+z*z as f32, selects one of 256 mantissas built by 007072E0, reconstructs the sqrt
/// exponent, stores the reciprocal as f32, then multiplies x/y/z in that order. Completion: 100% of the bit, branch and
/// store sequence for an existing table. Precision: 24-bit per step on the main thread after CreateDevice.
pub fn normalize_particle_mesh_averaged_normal(
    normal: &mut [f32; 3],
    table: Option<&SqrtMantissaTable>,
) -> ParticleMeshNormalizeResult {
    let squared = normal[1] * normal[1] + normal[0] * normal[0] + normal[2] * normal[2];
    let mut reciprocal = 0.0;
    if squared != 0.0 {
        let Some(table) = table else {
            return ParticleMeshNormalizeResult::MissingTable;
        };
        let bits = squared.to_bits();
        let exponent = (bits >> 23) as i32 - 127;
        let half_exponent = if exponent < 0 {
            -((-exponent + 1) / 2)
        } else {
            exponent / 2
        };
        let index = ((bits & 0x007F_FFFF) | if exponent & 1 != 0 { 0x0080_0000 } else { 0 }) >> 16;
        let sqrt_bits = table.0[index as usize] | (((half_exponent + 127) as u32) << 23);
        let approximate_sqrt = f32::from_bits(sqrt_bits);
        reciprocal = 1.0 / approximate_sqrt;
    }
    normal[0] *= reciprocal;
    normal[1] *= reciprocal;
    normal[2] *= reciprocal;
    ParticleMeshNormalizeResult::Normalized
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParticleMeshEmitterPending {
    pub base: ParticleEmitterPending,
    pub mesh_indices: Vec<u32>,
    pub initial_velocity_type: u32,
    pub emission_type: u32,
    pub emission_axis: [f32; 3],
    pub bytes_consumed: usize,
}

impl Default for ParticleMeshEmitterPending {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            base: ParticleEmitterPending::default(),
            mesh_indices: Vec::new(),
            initial_velocity_type: 0,
            emission_type: 0,
            emission_axis: [1.0, 0.0, 0.0],
            bytes_consumed: 0,
        }
    }
}

/// IDA: 0074BCD0 / 0074AE30 ask NiTriBasedGeomData +0x5C / +0x60 for the triangle count and each triple; NiLines is the
/// only observed fallback. Completion: 100% of the explicit kind.
/// TODO: [ ] Bind NiTriShapeData / NiTriStripsData virtual index providers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParticleMeshTopology {
    #[default]
    Other,
    Triangles,
    Lines,
}

/// IDA: 0074A390 and xOBSE NiGeometry.h agree on NiSkinPartition::Partition size 0x2C: pointer fields +0x04..+0x18 and
/// u16 counts +0x1C..+0x24. Completion: 100% of the explicit resolved view, without pointer ownership.
/// TODO: [ ] Partition materialization and bone-transform accumulation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleMeshSkinPartitionView<'a> {
    pub bones: &'a [u16],
    pub weights: &'a [f32],
    pub vertex_map: &'a [u16],
    pub bone_indices: &'a [u8],
    pub strip_data: &'a [u16],
    pub strip_lengths: &'a [u16],
    pub vertex_count: u16,
    pub triangle_count: u16,
    pub bone_count: u16,
    pub strip_count: u16,
    pub weights_per_vertex: u16,
    pub raw_tail_word: u32,
}

/// IDA: 0074C910 creates a 0xC NiSkinnedEmitterData cache object for every nonnull linked geometry. Its +0x08 ref is the
/// skin partition at NiSkinInstance +0x0C, or null pending 0074A2D0's lazy construction. The C++ owners are entities, and
/// `mesh_geometry::mesh_skin_link_view` reads them. Completion: 100% of the explicit source metadata.
/// TODO: [ ] NiRefObject lifetimes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParticleMeshSkinLinkView {
    pub geometry_present: bool,
    pub skin_instance_present: bool,
    pub skin_data_present: bool,
    pub geometry_data_present: bool,
    pub geometry_keep_flags: u8,
    pub partition_owner: Option<Entity>,
    /// IDA: N/A transient typed source identity for 0074C910's pointer read.
    /// TODO: [ ] Bind the raw NiGeometry lifetime.
    pub skin_instance_owner: Option<Entity>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParticleMeshSkinCacheEntry {
    pub cache_requested: bool,
    pub skin_instance_present: bool,
    pub needs_lazy_partition: bool,
    pub lazy_partition_source_present: bool,
    pub geometry_keep_flags_after: Option<u8>,
    pub partition_owner: Option<Entity>,
    /// IDA: N/A a host identity guard; 0074C910's cache keeps the partition at +0x08, not this skin pointer.
    /// TODO: [ ] Bind original object epochs.
    pub skin_instance_owner: Option<Entity>,
}

/// IDA: 0074A390 reads NiSkinInstance +0x08 data, +0x10 root parent, +0x14 bone objects; NiSkinData +0x0C transform and
/// +0x44 bone transforms; NiSkinPartition +0x08 / +0x0C supplies the partition array. Completion: 100% of the explicit
/// recovered field contracts with resolved transform inputs.
/// TODO: [ ] Bind live NiSkinInstance, NiSkinData and NiAVObject lifetimes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleMeshSkinResolvedView<'a> {
    pub skin_data_transform: NiTransform,
    pub root_parent_world: NiTransform,
    pub bone_world: &'a [NiTransform],
    pub skin_bone_transform: &'a [NiTransform],
    pub partitions: &'a [ParticleMeshSkinPartitionView<'a>],
}

/// IDA: 0074C910 creates one cache record after inserting a nonnull geometry and before its +0x30 keep-flag OR with 0x33.
/// Completion: 100% of the scoped metadata for one current source snapshot.
/// TODO: [ ] Bind the original's 0xC-byte cache object and refcounts.
fn make_mesh_skin_cache_entry(link: &ParticleMeshSkinLinkView) -> ParticleMeshSkinCacheEntry {
    let mut entry = ParticleMeshSkinCacheEntry::default();
    if link.geometry_present {
        entry.cache_requested = true;
        entry.skin_instance_present = link.skin_instance_present;
        if link.skin_instance_present {
            entry.partition_owner = link.partition_owner;
            entry.skin_instance_owner = link.skin_instance_owner;
        }
        entry.needs_lazy_partition = link.skin_instance_present && link.partition_owner.is_none();
        entry.lazy_partition_source_present = link.skin_instance_present && link.skin_data_present;
        if link.geometry_data_present {
            entry.geometry_keep_flags_after = Some(link.geometry_keep_flags | 0x33);
        }
    }
    entry
}

/// IDA: 0074C910 requests one 0xC NiSkinnedEmitterData object for each nonnull linked geometry, copies the current
/// NiSkinInstance +0x0C partition ref when present, then ORs NiGeometryData +0x30 keep flags with 0x33. Completion: 100%
/// of the checked metadata and slot order; no refcount or heap effects.
/// TODO: [ ] Bind concrete geometry and skin owners before applying side effects.
#[must_use]
pub fn prepare_particle_mesh_skin_cache(
    links: &[ParticleMeshSkinLinkView],
) -> Vec<ParticleMeshSkinCacheEntry> {
    links.iter().map(make_mesh_skin_cache_entry).collect()
}

/// IDA: 0074A0A0 composes the inverse target world with the mesh world before transforming a sampled point and its
/// normal-derived velocity. Explicit resolved input, not an overlay of NiGeometry or NiPSysData.
/// TODO: [ ] Bind actual NiGeometry, NiAVObject and skin owners and derived data.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParticleMeshGeometryView<'g> {
    pub vertex_count: u16,
    pub vertices: &'g [[f32; 3]],
    pub normals: &'g [[f32; 3]],
    pub topology: ParticleMeshTopology,
    pub triangles: &'g [[u16; 3]],
    pub mesh_to_target: NiTransform,
    pub skinned: bool,
    pub skin: Option<ParticleMeshSkinResolvedView<'g>>,
}

/// IDA: 0074C770 picks a list slot through CRT rand before inspecting mesh geometry; 0074AD30 draws a second raw rand
/// modulo vertex_count. Completion: 100% of the explicit ordered boundary.
/// TODO: [ ] Bind the CRT's state and the NiGeometry object table.
pub trait ParticleMeshEmitterRuntime<'g> {
    /// IDA: 009859DD rand returns 0..32767.
    fn draw_rand15(&mut self) -> u16;
    /// IDA: 0074C770 dereferences the chosen NiGeometry pointer after the first draw; `None` is a null pointer.
    /// TODO: [ ] Retain concrete linked NiGeometry and NiSkinInstance ownership.
    fn geometry_at(&mut self, slot: usize) -> Option<ParticleMeshGeometryView<'g>>;
    /// IDA: 00725860 publishes the 256-entry table consumed by 004BFAA0; `None` is reported as missing.
    /// TODO: [ ] Bind the process singleton's lifetime and x87 runtime validation.
    fn sqrt_mantissas(&self) -> Option<&SqrtMantissaTable>;
}

/// IDA: 0074BB20 allocates the 0x84-byte NiPSysMeshEmitter through 0074ACC0: empty geometry and skinned arrays, mode
/// words zero and axis (1,0,0). Completion: 100% of the cold values, modern payload and bounded generation for explicit
/// geometry and skin views.
/// TODO: [ ] Bind original skin and geometry owners.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct ParticleMeshEmitter {
    /// IDA: 0074F240 +0x18..+0x4C, and 00752DC0's +0x08 / +0x0C / +0x14.
    pub state: ParticleEmitterPending,
    /// IDA: 0074BB40 / 00712AE0 keep the raw deferred reference list for stream evidence; null-resolved entries do not
    /// become linked slots.
    pub raw_mesh_indices: Vec<u32>,
    /// IDA: 0074CA80 +0x54 / +0x5C occupied geometry array after 0074C910's nonnull inserts, in linked-slot order, as
    /// stream indexes for evidence. `linked_geometry_54` holds the same slots as entities.
    pub mesh_indices: Vec<u32>,
    /// IDA: 0074C5D0's published NiGeometry slots, when `link_particle_mesh_emitter` linked them; empty on the no-world
    /// presence path. TODO: [ ] Refcount semantics.
    pub linked_geometry_54: Vec<Entity>,
    /// IDA: 0074C910 stores one cache slot per linked mesh.
    pub skin_cache: Vec<ParticleMeshSkinCacheEntry>,
    next_raw_link_slot: usize,
    link_open: bool,
    pub initial_velocity_type_70: u32,
    pub emission_type_74: u32,
    pub emission_axis_78: [f32; 3],
}

impl Default for ParticleMeshEmitter {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            state: ParticleEmitterPending::default(),
            raw_mesh_indices: Vec::new(),
            mesh_indices: Vec::new(),
            linked_geometry_54: Vec::new(),
            skin_cache: Vec::new(),
            next_raw_link_slot: 0,
            link_open: false,
            initial_velocity_type_70: 0,
            emission_type_74: 0,
            emission_axis_78: [1.0, 0.0, 0.0],
        }
    }
}

/// IDA: 0074C770 / 0074AD30 consume raw CRT rand results; the host rejects values outside 0..32767 before using them.
/// Completion: 100% of the checked draw.
/// TODO: [ ] Bind the CRT's state.
fn draw_rand15<'g>(runtime: &mut dyn ParticleMeshEmitterRuntime<'g>) -> Result<u16, GameError> {
    let value = runtime.draw_rand15();
    if value > 32767 {
        return Err(GameError::unclassified(
            0,
            "particle mesh emitter random value outside rand15 domain",
        ));
    }
    Ok(value)
}

/// IDA: the 0053D460 fraction: a raw draw over 32767.0, stored as f32 where the caller stores it.
fn draw_fraction<'g>(runtime: &mut dyn ParticleMeshEmitterRuntime<'g>) -> Result<f32, GameError> {
    Ok((f64::from(draw_rand15(runtime)?) / 32767.0) as f32)
}

/// IDA: 0074A390 reads the u16 vertex map and bone map, optional u8 per-vertex bone indices, and a four-float weight
/// stride; each influence composes skinData * inverse(rootWorld) * boneWorld * skinBone, scales the rotation before point
/// multiplication, accumulates weighted position and normal, then calls 0043F350 on the final normal. Completion: 100% of
/// the bounded index, weight, transform and store order for resolved views.
/// TODO: [ ] Bind live bone object owners.
fn skin_vertex_weighted(
    geometry: &ParticleMeshGeometryView<'_>,
    skin: &ParticleMeshSkinResolvedView<'_>,
    partition: &ParticleMeshSkinPartitionView<'_>,
    local_vertex: u16,
) -> Result<([f32; 3], [f32; 3]), GameError> {
    if local_vertex >= partition.vertex_count
        || usize::from(local_vertex) >= partition.vertex_map.len()
    {
        return Err(GameError::unclassified(
            0,
            "mesh skin vertex map extent invalid",
        ));
    }
    if partition.weights_per_vertex > 4 || partition.bones.len() < usize::from(partition.bone_count)
    {
        return Err(GameError::unclassified(
            0,
            "mesh skin influence extent invalid",
        ));
    }
    let source_vertex = partition.vertex_map[usize::from(local_vertex)];
    if source_vertex >= geometry.vertex_count
        || usize::from(source_vertex) >= geometry.vertices.len()
    {
        return Err(GameError::unclassified(
            0,
            "mesh skin source vertex invalid",
        ));
    }
    let p = geometry.vertices[usize::from(source_vertex)];
    let n = if geometry.normals.is_empty() {
        [1.0, 0.0, 0.0]
    } else {
        geometry.normals[usize::from(source_vertex)]
    };
    // 00718A80, as the C++ port guards it, for the 0074A390 bone chain.
    let inverse_root = invert_scene_transform_checked(
        &skin.root_parent_world,
        GameError::unclassified(0, "mesh skin root scale invalid"),
    )?;
    let skin_to_root = compose_scene_transform(&skin.skin_data_transform, &inverse_root);
    let mut position = [0.0f32; 3];
    let mut normal = [0.0f32; 3];
    let stride = usize::from(local_vertex) * 4;
    if stride + usize::from(partition.weights_per_vertex) > partition.weights.len() {
        return Err(GameError::unclassified(
            0,
            "mesh skin weight extent invalid",
        ));
    }
    for influence in 0..usize::from(partition.weights_per_vertex) {
        let mut bone_slot = influence;
        if !partition.bone_indices.is_empty() {
            let offset =
                usize::from(local_vertex) * usize::from(partition.weights_per_vertex) + influence;
            if offset >= partition.bone_indices.len() {
                return Err(GameError::unclassified(
                    0,
                    "mesh skin bone index extent invalid",
                ));
            }
            bone_slot = usize::from(partition.bone_indices[offset]);
        }
        if bone_slot >= partition.bones.len() {
            return Err(GameError::unclassified(0, "mesh skin bone slot invalid"));
        }
        let bone_index = usize::from(partition.bones[bone_slot]);
        if bone_index >= skin.bone_world.len() || bone_index >= skin.skin_bone_transform.len() {
            return Err(GameError::unclassified(
                0,
                "mesh skin bone transform missing",
            ));
        }
        let with_world = compose_scene_transform(&skin_to_root, &skin.bone_world[bone_index]);
        let transform = compose_scene_transform(&with_world, &skin.skin_bone_transform[bone_index]);
        let m = transform.rotation.map(|value| value * transform.scale);
        let r = &transform.rotation;
        let t = &transform.translation;
        let bone_position = [
            // 0074A54D..0074A589: (m1*p1 + m0*p0) + m2*p2, then the translation.
            (m[1] * p[1] + m[0] * p[0] + m[2] * p[2] + t[0]),
            m[4] * p[1] + m[3] * p[0] + m[5] * p[2] + t[1],
            p[0] * m[6] + p[1] * m[7] + p[2] * m[8] + t[2],
        ];
        let bone_normal = [
            // 0074A5E1..0074A616: (r1*n1 + r0*n0) + r2*n2.
            (r[1] * n[1] + r[0] * n[0] + r[2] * n[2]),
            r[4] * n[1] + r[3] * n[0] + r[5] * n[2],
            n[0] * r[6] + n[1] * r[7] + n[2] * r[8],
        ];
        let weight = partition.weights[stride + influence];
        let weighted_px = bone_position[0] * weight;
        let weighted_py = bone_position[1] * weight;
        let weighted_pz = bone_position[2] * weight;
        position[0] += weighted_px;
        position[1] += weighted_py;
        position[2] += weighted_pz;
        let weighted_nx = bone_normal[0] * weight;
        let weighted_ny = bone_normal[1] * weight;
        let weighted_nz = weight * bone_normal[2];
        normal[0] += weighted_nx;
        normal[1] += weighted_ny;
        normal[2] += weighted_nz;
    }
    normalize_vector(&mut normal);
    Ok((position, normal))
}

/// IDA: 0074A0A0 mode 1 writes three centered (2*rand/32767)-1 components, normalizes them, draws one speed fraction and
/// scales the vector. Completion: 100% of the four-draw and component store order for an explicit target space.
/// TODO: [ ] Validate CRT rand externally.
fn random_mesh_velocity<'g>(
    base: &ParticleEmitterPending,
    runtime: &mut dyn ParticleMeshEmitterRuntime<'g>,
) -> Result<[f32; 3], GameError> {
    let mut velocity = [0.0; 3];
    for component in &mut velocity {
        // 0074A1E3: fild; fadd st,st (exact); fdiv qword rounds to a float; fsub qword rounds again.
        let raw = f64::from(draw_rand15(runtime)?);
        *component = ((raw + raw) / 32767.0) as f32 - 1.0;
    }
    normalize_vector(&mut velocity);
    let speed_fraction = draw_fraction(runtime)?;
    let speed = (speed_fraction - 0.5) * base.speed_variation + base.speed;
    velocity[0] *= speed;
    velocity[1] *= speed;
    velocity[2] *= speed;
    Ok(velocity)
}

/// IDA: 0074A0A0 mode 2 rotates the +0x78 axis into target coordinates, draws one speed fraction, normalizes the rotated
/// axis and scales it. Completion: 100% of the one-draw and component store order.
/// TODO: [ ] Bind exact mesh and target NiAVObject frame owners.
fn directed_mesh_velocity<'g>(
    base: &ParticleEmitterPending,
    axis: &[f32; 3],
    relative: &NiTransform,
    runtime: &mut dyn ParticleMeshEmitterRuntime<'g>,
) -> Result<[f32; 3], GameError> {
    let mut direction = rotate_vector(&relative.rotation, axis);
    let speed_fraction = draw_fraction(runtime)?;
    let speed = (speed_fraction - 0.5) * base.speed_variation + base.speed;
    normalize_vector(&mut direction);
    direction[0] *= speed;
    direction[1] *= speed;
    direction[2] *= speed;
    Ok(direction)
}

/// IDA: 0074BCD0 / 0074AE30 draw a triangle index once, then advance and wrap through at most six degenerate triples
/// without drawing again. After six failures the original still uses the final triple. Completion: 100% of the bounded
/// index, draw and retry order on a resolved triangle provider.
/// TODO: [ ] Bind virtual GetNumTris / GetTriIndices to NiTriBasedGeomData.
fn choose_triangle<'g>(
    geometry: &ParticleMeshGeometryView<'_>,
    runtime: &mut dyn ParticleMeshEmitterRuntime<'g>,
) -> Result<[u16; 3], GameError> {
    let raw = draw_rand15(runtime)?;
    let count = geometry.triangles.len();
    if count == 0 {
        return Err(GameError::unclassified(
            0,
            "mesh emitter triangle count is zero",
        ));
    }
    let mut index = usize::from(raw) % count;
    let mut triangle = [0; 3];
    for _ in 0..6 {
        triangle = geometry.triangles[index];
        if triangle[0] != triangle[1] && triangle[0] != triangle[2] && triangle[1] != triangle[2] {
            break;
        }
        index += 1;
        if index >= count {
            index = 0;
        }
    }
    Ok(triangle)
}

/// IDA: 0074AD30 / 0074BCD0 / 0074AE30 use the incoming velocity's length before replacing its direction with the
/// selected or averaged normal. Completion: 100% of the squared-length component and f32 sqrt store order.
fn incoming_speed(velocity: &[f32; 3]) -> f32 {
    let squared = velocity[1] * velocity[1] + velocity[0] * velocity[0] + velocity[2] * velocity[2];
    squared.sqrt()
}

/// IDA: the three stores that scale a normal by the incoming speed.
fn store_scaled_normal(velocity: &mut [f32; 3], normal: &[f32; 3], speed: f32) {
    velocity[0] = normal[0] * speed;
    velocity[1] = normal[1] * speed;
    velocity[2] = speed * normal[2];
}

/// IDA: 0074B220, 0074B3C0 and 0074B7A0 each select a partition with one stored f32 rand fraction, clamped at count-1.
fn choose_partition<'s, 'g>(
    skin: &'s ParticleMeshSkinResolvedView<'_>,
    runtime: &mut dyn ParticleMeshEmitterRuntime<'g>,
) -> Result<&'s ParticleMeshSkinPartitionView<'s>, GameError> {
    if skin.partitions.is_empty() {
        return Err(GameError::unclassified(
            0,
            "mesh skin partition count is zero",
        ));
    }
    let partition_fraction = draw_fraction(runtime)?;
    let count = skin.partitions.len();
    // 0074B285: fimul rounds to a float at 24-bit before the truncating fistp.
    let mut partition_index = (f64::from(partition_fraction) * count as f64) as f32 as usize;
    if partition_index >= count - 1 {
        partition_index = count - 1;
    }
    Ok(&skin.partitions[partition_index])
}

/// IDA: 0074B220 requires tri-based skinned geometry and a cached partition, draws one f32 fraction for partition
/// selection, then a second for local vertex selection. 0074A390 supplies the weighted position and normal; mode 0 uses
/// 00404C90 / 0047DA10 to scale that normal to the incoming speed before 0074A0A0's relative-frame step. Completion: 100%
/// of the bounded draw, index and store order for a resolved skin.
/// TODO: [ ] Port 0074A2D0's lazy partition construction and live owners.
fn sample_skinned_vertex<'g>(
    geometry: &ParticleMeshGeometryView<'_>,
    initial_velocity_type: u32,
    position: &mut [f32; 3],
    velocity: &mut [f32; 3],
    runtime: &mut dyn ParticleMeshEmitterRuntime<'g>,
) -> Result<bool, GameError> {
    if geometry.topology != ParticleMeshTopology::Triangles {
        return Ok(false);
    }
    let skin = geometry
        .skin
        .as_ref()
        .ok_or(GameError::unclassified(0, "mesh skin owner unresolved"))?;
    let partition = choose_partition(skin, runtime)?;
    let vertex_fraction = draw_fraction(runtime)?;
    if partition.vertex_count == 0 {
        return Err(GameError::unclassified(0, "mesh skin vertex count is zero"));
    }
    // 0074B2F8: fild; fmul rounds to a float at 24-bit before the fistp.
    let vertex = x87::truncated_low16(f64::from(
        (f64::from(i32::from(partition.vertex_count) - 1) * f64::from(vertex_fraction)) as f32,
    ));
    if geometry.vertices.is_empty() {
        return Ok(false);
    }
    let (weighted_position, weighted_normal) =
        skin_vertex_weighted(geometry, skin, partition, vertex)?;
    *position = weighted_position;
    if initial_velocity_type == 0 && !geometry.normals.is_empty() {
        let speed = incoming_speed(velocity);
        store_scaled_normal(velocity, &weighted_normal, speed);
    }
    Ok(true)
}

/// IDA: the 0053D460 fraction (stored to a float there) selects a triangle through a fimul that rounds to a float at
/// 24-bit, clamped at count-1; a strip partition's offset is the index
/// itself, a list's three times it. Returns the triple from `strip_data`.
fn choose_skinned_triangle<'g>(
    partition: &ParticleMeshSkinPartitionView<'_>,
    has_vertices: bool,
    runtime: &mut dyn ParticleMeshEmitterRuntime<'g>,
) -> Result<Option<[u16; 3]>, GameError> {
    let triangle_fraction = f64::from((f64::from(draw_rand15(runtime)?) / 32767.0) as f32);
    let mut triangle_index =
        (triangle_fraction * f64::from(partition.triangle_count)) as f32 as usize;
    if triangle_index >= usize::from(partition.triangle_count) - 1 {
        triangle_index = usize::from(partition.triangle_count) - 1;
    }
    if !has_vertices {
        return Ok(None);
    }
    let offset = if partition.strip_count != 0 {
        triangle_index
    } else {
        3 * triangle_index
    };
    if offset + 2 >= partition.strip_data.len() {
        return Err(GameError::unclassified(
            0,
            "mesh skin triangle index extent invalid",
        ));
    }
    Ok(Some([
        partition.strip_data[offset],
        partition.strip_data[offset + 1],
        partition.strip_data[offset + 2],
    ]))
}

/// IDA: 004BFAA0, reported through the runtime's table, then the speed scaling of 0074B3C0 / 0074B7A0.
fn align_velocity_to_average<'g>(
    average_normal: &mut [f32; 3],
    velocity: &mut [f32; 3],
    runtime: &dyn ParticleMeshEmitterRuntime<'g>,
) -> Result<(), GameError> {
    if normalize_particle_mesh_averaged_normal(average_normal, runtime.sqrt_mantissas())
        == ParticleMeshNormalizeResult::MissingTable
    {
        return Err(GameError::unclassified(
            0,
            "particle mesh normal lookup table unavailable",
        ));
    }
    let speed = incoming_speed(velocity);
    store_scaled_normal(velocity, average_normal, speed);
    Ok(())
}

/// IDA: 0074B3C0 selects a partition with one stored f32 rand fraction, falls back to 0074B220 when its triangle count is
/// zero, otherwise selects one triangle using the unrounded 0053D460 fraction, samples three weighted skin vertices, and
/// divides their position and normal sums through 004BF9B0. Mode 0 normalizes the averaged normal through 004BFAA0; mode
/// 3 consumes two further draws and applies the original's barycentric float stores. The C++ operand swaps by component
/// stay. Completion: 100% of the bounded branch, draw, index and store order on explicit skin views.
/// TODO: [ ] Bind the partition allocator.
fn sample_skinned_face<'g>(
    geometry: &ParticleMeshGeometryView<'_>,
    initial_velocity_type: u32,
    emission_type: u32,
    position: &mut [f32; 3],
    velocity: &mut [f32; 3],
    runtime: &mut dyn ParticleMeshEmitterRuntime<'g>,
) -> Result<bool, GameError> {
    if geometry.topology != ParticleMeshTopology::Triangles {
        return Ok(false);
    }
    let skin = geometry
        .skin
        .as_ref()
        .ok_or(GameError::unclassified(0, "mesh skin owner unresolved"))?;
    let partition = choose_partition(skin, runtime)?;
    if partition.triangle_count == 0 {
        return sample_skinned_vertex(geometry, initial_velocity_type, position, velocity, runtime);
    }
    let Some(vertex_indices) =
        choose_skinned_triangle(partition, !geometry.vertices.is_empty(), runtime)?
    else {
        return Ok(false);
    };
    let (first_position, first_normal) =
        skin_vertex_weighted(geometry, skin, partition, vertex_indices[0])?;
    let (second_position, second_normal) =
        skin_vertex_weighted(geometry, skin, partition, vertex_indices[1])?;
    let (third_position, third_normal) =
        skin_vertex_weighted(geometry, skin, partition, vertex_indices[2])?;
    let reciprocal_three = (1.0f64 / 3.0) as f32;
    for component in 0..3 {
        let pair = if component == 2 {
            first_position[component] + second_position[component]
        } else {
            second_position[component] + first_position[component]
        };
        let sum = if component == 0 {
            pair + third_position[component]
        } else {
            third_position[component] + pair
        };
        position[component] = if component == 2 {
            reciprocal_three * sum
        } else {
            sum * reciprocal_three
        };
    }
    if initial_velocity_type == 0 {
        let mut average_normal = [0.0; 3];
        for component in 0..3 {
            let pair = if component == 0 {
                second_normal[component] + first_normal[component]
            } else {
                first_normal[component] + second_normal[component]
            };
            let sum = if component == 0 {
                pair + third_normal[component]
            } else {
                third_normal[component] + pair
            };
            average_normal[component] = if component == 2 {
                reciprocal_three * sum
            } else {
                sum * reciprocal_three
            };
        }
        align_velocity_to_average(&mut average_normal, velocity, runtime)?;
    }
    if emission_type == 3 {
        let mut first_difference = [0.0f32; 3];
        let mut second_difference = [0.0f32; 3];
        for component in 0..3 {
            first_difference[component] = second_position[component] - first_position[component];
            second_difference[component] = third_position[component] - first_position[component];
        }
        let first_fraction = draw_fraction(runtime)?;
        for value in &mut first_difference {
            *value *= first_fraction;
        }
        // 0074B6FB..0074B70D: the stored 0053D460 quotient times (1 - first), each rounded.
        let second_fraction =
            (f64::from(draw_rand15(runtime)?) / 32767.0) as f32 * (1.0 - first_fraction);
        for value in &mut second_difference {
            *value *= second_fraction;
        }
        for component in 0..3 {
            let first_sum = first_difference[component] + first_position[component];
            position[component] = first_sum + second_difference[component];
        }
    }
    Ok(true)
}

/// IDA: 0074B7A0 selects a partition with one stored f32 rand fraction, falls back to 0074B220 for zero triangles,
/// otherwise selects a triangle through the unrounded 0053D460 fraction and then one of three edges with raw rand%3. Two
/// 0074A390 weighted vertices provide the endpoint pair; 004BF9B0 stores the midpoint and average normal, and mode 4 draws
/// one more fraction for the exact endpoint interpolation stores. The C++ operand swaps by component stay.
/// Completion: 100% of the bounded branch, draw, index and store order on resolved skin views.
/// TODO: [ ] Bind lazy partition allocation and original bone owners.
fn sample_skinned_edge<'g>(
    geometry: &ParticleMeshGeometryView<'_>,
    initial_velocity_type: u32,
    emission_type: u32,
    position: &mut [f32; 3],
    velocity: &mut [f32; 3],
    runtime: &mut dyn ParticleMeshEmitterRuntime<'g>,
) -> Result<bool, GameError> {
    if geometry.topology != ParticleMeshTopology::Triangles {
        return Ok(false);
    }
    let skin = geometry
        .skin
        .as_ref()
        .ok_or(GameError::unclassified(0, "mesh skin owner unresolved"))?;
    let partition = choose_partition(skin, runtime)?;
    if partition.triangle_count == 0 {
        return sample_skinned_vertex(geometry, initial_velocity_type, position, velocity, runtime);
    }
    let Some(triangle) =
        choose_skinned_triangle(partition, !geometry.vertices.is_empty(), runtime)?
    else {
        return Ok(false);
    };
    let edge = draw_rand15(runtime)? % 3;
    let endpoints = match edge {
        1 => [triangle[1], triangle[2]],
        2 => [triangle[0], triangle[2]],
        _ => [triangle[0], triangle[1]],
    };
    let (first_position, first_normal) =
        skin_vertex_weighted(geometry, skin, partition, endpoints[0])?;
    let (second_position, second_normal) =
        skin_vertex_weighted(geometry, skin, partition, endpoints[1])?;
    let reciprocal_two = (1.0f64 / 2.0) as f32;
    for component in 0..3 {
        let sum = if component == 0 {
            second_position[component] + first_position[component]
        } else {
            first_position[component] + second_position[component]
        };
        position[component] = if component == 2 {
            reciprocal_two * sum
        } else {
            sum * reciprocal_two
        };
    }
    if initial_velocity_type == 0 {
        let mut average_normal = [0.0; 3];
        for component in 0..3 {
            let sum = if component == 0 {
                second_normal[component] + first_normal[component]
            } else {
                first_normal[component] + second_normal[component]
            };
            average_normal[component] = if component == 2 {
                reciprocal_two * sum
            } else {
                sum * reciprocal_two
            };
        }
        align_velocity_to_average(&mut average_normal, velocity, runtime)?;
    }
    if emission_type == 4 {
        let mut difference = [0.0f32; 3];
        for component in 0..3 {
            difference[component] = second_position[component] - first_position[component];
        }
        let fraction = draw_fraction(runtime)?;
        for (component, value) in difference.iter_mut().enumerate() {
            *value = if component == 2 {
                fraction * (*value)
            } else {
                (*value) * fraction
            };
        }
        for component in 0..3 {
            position[component] = difference[component] + first_position[component];
        }
    }
    Ok(true)
}

/// IDA: 0074AE30 averages two normals with f32 0.5 stores; 0074BCD0 averages three with f32 0x3EAAAAAB stores, then both
/// call 004BFAA0 before scaling by the incoming speed. Completion: 100% of the explicit normalization boundary.
/// TODO: [ ] The BSS sqrt-mantissa lookup initializer's real owner.
fn align_to_averaged_normal<'g>(
    geometry: &ParticleMeshGeometryView<'_>,
    indices: &[u16],
    velocity: &mut [f32; 3],
    runtime: &dyn ParticleMeshEmitterRuntime<'g>,
) -> Result<(), GameError> {
    if geometry.normals.is_empty() {
        return Ok(());
    }
    let normal_at =
        |slot: usize, component: usize| geometry.normals[usize::from(indices[slot])][component];
    let mut normal = [0.0; 3];
    let factor = if indices.len() == 2 {
        0.5
    } else {
        THIRD_FACTOR
    };
    for (component, value) in normal.iter_mut().enumerate() {
        let reverse_pair = component == 0;
        let pair = (normal_at(usize::from(reverse_pair), component))
            + (normal_at(usize::from(!reverse_pair), component));
        let sum = if indices.len() == 2 {
            pair
        } else if component == 0 {
            pair + normal_at(2, component)
        } else {
            normal_at(2, component) + pair
        };
        *value = sum * factor;
    }
    align_velocity_to_average(&mut normal, velocity, runtime)
}

/// IDA: 0074AE30 chooses one of three edges after the triangle retry; NiLines instead uses a single fraction to choose
/// adjacent vertex indices. Mode 4 draws one further fraction after the midpoint and normal stores. Completion: 100% of
/// the bounded branch and RNG order for a resolved topology.
/// TODO: [ ] Verify NiLines count arithmetic externally.
fn sample_edge<'g>(
    geometry: &ParticleMeshGeometryView<'_>,
    initial_velocity_type: u32,
    emission_type: u32,
    position: &mut [f32; 3],
    velocity: &mut [f32; 3],
    runtime: &mut dyn ParticleMeshEmitterRuntime<'g>,
) -> Result<bool, GameError> {
    let endpoints: [u16; 2] = if geometry.topology == ParticleMeshTopology::Triangles {
        let triangle = choose_triangle(geometry, runtime)?;
        let edge = draw_rand15(runtime)? % 3;
        match edge {
            1 => [triangle[1], triangle[2]],
            2 => [triangle[0], triangle[2]],
            _ => [triangle[0], triangle[1]],
        }
    } else if geometry.topology == ParticleMeshTopology::Lines {
        let fraction = draw_fraction(runtime)?;
        if geometry.vertex_count < 2 {
            return Err(GameError::unclassified(
                0,
                "mesh line emitter needs two vertices",
            ));
        }
        let first = x87::truncated_low16(f64::from(
            (f64::from(i32::from(geometry.vertex_count) - 1) * f64::from(fraction)) as f32,
        ));
        [first, first.wrapping_add(1)]
    } else {
        return Ok(false);
    };
    if endpoints[0] >= geometry.vertex_count || endpoints[1] >= geometry.vertex_count {
        return Err(GameError::unclassified(
            0,
            "mesh edge index outside vertex extent",
        ));
    }
    if geometry.vertices.is_empty() {
        return Ok(false);
    }
    let first = geometry.vertices[usize::from(endpoints[0])];
    let second = geometry.vertices[usize::from(endpoints[1])];
    for component in 0..3 {
        let sum = if component == 0 {
            second[component] + first[component]
        } else {
            first[component] + second[component]
        };
        position[component] = sum * 0.5;
    }
    if initial_velocity_type == 0 && !geometry.normals.is_empty() {
        align_to_averaged_normal(geometry, &endpoints, velocity, runtime)?;
    }
    if emission_type == 4 {
        let fraction = draw_fraction(runtime)?;
        for component in 0..3 {
            let difference = second[component] - first[component];
            let scaled = difference * fraction;
            position[component] = first[component] + scaled;
        }
    }
    Ok(true)
}

/// IDA: 0074BCD0 uses NiTriBasedGeomData or falls through to 0074AE30 for NiLines. Mode 3 draws two barycentric fractions
/// after the center and normal stores. Completion: 100% of the bounded branch and RNG order for a resolved topology.
fn sample_face<'g>(
    geometry: &ParticleMeshGeometryView<'_>,
    initial_velocity_type: u32,
    emission_type: u32,
    position: &mut [f32; 3],
    velocity: &mut [f32; 3],
    runtime: &mut dyn ParticleMeshEmitterRuntime<'g>,
) -> Result<bool, GameError> {
    if geometry.topology == ParticleMeshTopology::Lines {
        return sample_edge(
            geometry,
            initial_velocity_type,
            emission_type,
            position,
            velocity,
            runtime,
        );
    }
    if geometry.topology != ParticleMeshTopology::Triangles {
        return Ok(false);
    }
    let triangle = choose_triangle(geometry, runtime)?;
    if triangle.iter().any(|&index| index >= geometry.vertex_count) {
        return Err(GameError::unclassified(
            0,
            "mesh face index outside vertex extent",
        ));
    }
    if geometry.vertices.is_empty() {
        return Ok(false);
    }
    let first = geometry.vertices[usize::from(triangle[0])];
    let second = geometry.vertices[usize::from(triangle[1])];
    let third = geometry.vertices[usize::from(triangle[2])];
    for component in 0..3 {
        let pair = if component == 2 {
            first[component] + second[component]
        } else {
            second[component] + first[component]
        };
        let sum = if component == 0 {
            pair + third[component]
        } else {
            third[component] + pair
        };
        position[component] = sum * THIRD_FACTOR;
    }
    if initial_velocity_type == 0 && !geometry.normals.is_empty() {
        align_to_averaged_normal(geometry, &triangle, velocity, runtime)?;
    }
    if emission_type == 3 {
        let first_fraction = draw_fraction(runtime)?;
        let second_fraction = draw_fraction(runtime)?;
        let remaining = 1.0 - first_fraction;
        let weighted_second = second_fraction * remaining;
        for component in 0..3 {
            let first_difference = second[component] - first[component];
            let second_difference = third[component] - first[component];
            let first_scaled = first_difference * first_fraction;
            let second_scaled = second_difference * weighted_second;
            let first_sum = first[component] + first_scaled;
            position[component] = first_sum + second_scaled;
        }
    }
    Ok(true)
}

impl ParticleMeshEmitter {
    /// IDA: 0074BB40 LoadBinary stores the inherited scalars and name, and the mode and axis, before 0074CA80 resolves
    /// mesh refs. Completion: 100% of the supported publication.
    /// TODO: [ ] Match the original's list and refcount allocation failures.
    pub fn apply_loaded_payload(&mut self, pending: &ParticleMeshEmitterPending) {
        self.state = pending.base.clone();
        self.raw_mesh_indices.clone_from(&pending.mesh_indices);
        self.initial_velocity_type_70 = pending.initial_velocity_type;
        self.emission_type_74 = pending.emission_type;
        self.emission_axis_78 = pending.emission_axis;
    }

    /// IDA: 0074C770 void __thiscall (NiPSysMeshEmitter*, NiVector3*, NiVector3*) reads the +0x5C mesh count, draws once,
    /// chooses floor(f32 draw fraction * count), clamps count to count-1, checks the chosen mesh, then routes the raw
    /// emission mode. Unskinned and resolved skinned leaves draw in their saved order, then 0074A0A0 applies an explicit
    /// mesh-to-target transform. The bool reports sampled outputs; the original's virtual ignores the leaf's AL.
    /// Completion: 100% of the bounded geometry and mode-specific draw order.
    /// TODO: [ ] Lazy skin partition construction and original world owners.
    pub fn generate<'g>(
        &self,
        position: &mut [f32; 3],
        velocity: &mut [f32; 3],
        runtime: &mut dyn ParticleMeshEmitterRuntime<'g>,
    ) -> Result<bool, GameError> {
        let count = self.mesh_indices.len();
        if count == 0 {
            return Ok(false);
        }
        let fraction = draw_fraction(runtime)?;
        // 0074C7BB: fmulp rounds to a float at 24-bit before the truncating fistp.
        let mut chosen = (f64::from(fraction) * count as f64) as f32 as usize;
        if chosen == count {
            chosen = count - 1;
        }
        if chosen >= count {
            return Ok(false);
        }
        let Some(geometry) = runtime.geometry_at(chosen) else {
            return Ok(false);
        };
        let vertex_count = usize::from(geometry.vertex_count);
        if !geometry.vertices.is_empty() && geometry.vertices.len() < vertex_count {
            return Err(GameError::unclassified(
                0,
                "mesh emitter vertex extent invalid",
            ));
        }
        if !geometry.normals.is_empty() && geometry.normals.len() < vertex_count {
            return Err(GameError::unclassified(
                0,
                "mesh emitter normal extent invalid",
            ));
        }
        let mut prepared_position = [0.0; 3];
        let mut prepared_velocity = *velocity;
        let (velocity_type, emission) = (self.initial_velocity_type_70, self.emission_type_74);
        let sampled = if geometry.skinned {
            match emission {
                1 | 3 => sample_skinned_face(
                    &geometry,
                    velocity_type,
                    emission,
                    &mut prepared_position,
                    &mut prepared_velocity,
                    runtime,
                )?,
                2 | 4 => sample_skinned_edge(
                    &geometry,
                    velocity_type,
                    emission,
                    &mut prepared_position,
                    &mut prepared_velocity,
                    runtime,
                )?,
                _ => sample_skinned_vertex(
                    &geometry,
                    velocity_type,
                    &mut prepared_position,
                    &mut prepared_velocity,
                    runtime,
                )?,
            }
        } else {
            match emission {
                1 | 3 => sample_face(
                    &geometry,
                    velocity_type,
                    emission,
                    &mut prepared_position,
                    &mut prepared_velocity,
                    runtime,
                )?,
                2 | 4 => sample_edge(
                    &geometry,
                    velocity_type,
                    emission,
                    &mut prepared_position,
                    &mut prepared_velocity,
                    runtime,
                )?,
                _ => {
                    let raw_vertex = draw_rand15(runtime)?;
                    if geometry.vertex_count == 0 {
                        return Err(GameError::unclassified(
                            0,
                            "mesh emitter vertex count is zero",
                        ));
                    }
                    let vertex = usize::from(raw_vertex % geometry.vertex_count);
                    if geometry.vertices.is_empty() {
                        return Ok(false);
                    }
                    prepared_position = geometry.vertices[vertex];
                    if velocity_type == 0 && !geometry.normals.is_empty() {
                        let speed = incoming_speed(&prepared_velocity);
                        store_scaled_normal(
                            &mut prepared_velocity,
                            &geometry.normals[vertex],
                            speed,
                        );
                    }
                    true
                }
            }
        };
        if !sampled {
            return Ok(false);
        }
        prepared_position = transform_point(&geometry.mesh_to_target, &prepared_position);
        if velocity_type == 0 {
            prepared_velocity =
                rotate_vector(&geometry.mesh_to_target.rotation, &prepared_velocity);
        } else if velocity_type == 1 {
            prepared_velocity = random_mesh_velocity(&self.state, runtime)?;
        } else if velocity_type == 2 {
            prepared_velocity = directed_mesh_velocity(
                &self.state,
                &self.emission_axis_78,
                &geometry.mesh_to_target,
                runtime,
            )?;
        }
        *position = prepared_position;
        *velocity = prepared_velocity;
        Ok(true)
    }
}

/// IDA: 0074BB40 calls 0074F240 (NiPSysEmitter, not VolumeEmitter), then 00712AE0 reads a u32 mesh count and that many u32
/// deferred references, followed by +0x70 / +0x74 raw mode words and +0x78 / +0x7C / +0x80 axis floats. It does not read
/// VolumeEmitter's +0x50 emitter-object link. Completion: 100% of the bounded modern byte order.
/// TODO: [ ] The original's list-allocation errors and legacy stream behaviour.
pub fn parse_particle_mesh_emitter_payload(
    bytes: &[u8],
    version: u32,
) -> Result<ParticleMeshEmitterPending, GameError> {
    let (mut base, mut reader) = parse_emitter_scalars(bytes, version, TRUNCATED)?;
    base.bytes_consumed = reader.position();
    let count = reader.u32(TRUNCATED)?;
    if count > u32::from(u16::MAX) || count as usize > reader.remaining() / 4 {
        return Err(GameError::unclassified(
            0,
            "particle mesh emitter list count out of range",
        ));
    }
    let mut mesh_indices = Vec::with_capacity(count as usize);
    for _ in 0..count {
        mesh_indices.push(reader.u32(TRUNCATED)?);
    }
    let initial_velocity_type = reader.u32(TRUNCATED)?;
    let emission_type = reader.u32(TRUNCATED)?;
    let mut emission_axis = [0.0; 3];
    for component in &mut emission_axis {
        *component = reader.f32(TRUNCATED)?;
    }
    Ok(ParticleMeshEmitterPending {
        base,
        mesh_indices,
        initial_velocity_type,
        emission_type,
        emission_axis,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 0074CA80 (+0x20) resolves raw deferred references in list order and calls 0074C910, which skips a null geometry
/// before inserting a linked slot. Presence bytes correspond to raw list entries; duplicates stay. Completion: 100% of the
/// checked null and slot order, staged; the original's refcounts remain open.
/// TODO: [ ] Bind the NiStream object table and 0074C910's side effects.
pub fn resolve_particle_mesh_emitter_link_with_presence(
    target: &mut ParticleMeshEmitter,
    pending: &ParticleMeshEmitterPending,
    object_count: usize,
    resolved_present: &[u8],
) -> Result<(), GameError> {
    if resolved_present.len() != pending.mesh_indices.len() {
        return Err(GameError::unclassified(
            0,
            "particle mesh resolved list length mismatch",
        ));
    }
    if pending.base.base.target_index != NULL_REFERENCE
        && pending.base.base.target_index as usize >= object_count
    {
        return Err(GameError::unclassified(
            0,
            "particle mesh emitter target link out of range",
        ));
    }
    let mut prepared_linked = Vec::with_capacity(pending.mesh_indices.len());
    for (slot, &index) in pending.mesh_indices.iter().enumerate() {
        if index != NULL_REFERENCE && index as usize >= object_count {
            return Err(GameError::unclassified(
                0,
                "particle mesh emitter geometry link out of range",
            ));
        }
        if resolved_present[slot] > 1 || (index == NULL_REFERENCE && resolved_present[slot] != 0) {
            return Err(GameError::unclassified(
                0,
                "particle mesh emitter resolved presence invalid",
            ));
        }
        if resolved_present[slot] != 0 {
            prepared_linked.push(index);
        }
    }
    target.state = pending.base.clone();
    target.raw_mesh_indices.clone_from(&pending.mesh_indices);
    target.mesh_indices = prepared_linked;
    target.linked_geometry_54.clear();
    target.skin_cache.clear();
    target.next_raw_link_slot = 0;
    target.link_open = false;
    target.initial_velocity_type_70 = pending.initial_velocity_type;
    target.emission_type_74 = pending.emission_type;
    target.emission_axis_78 = pending.emission_axis;
    Ok(())
}

/// IDA: 0074CA80 consumes every deferred reference, while 0074C910 ignores the raw null sentinel. This convenience path
/// has no type resolver. Completion: 100% of the sentinel compaction only.
/// TODO: [ ] Supply actual stream presence.
pub fn resolve_particle_mesh_emitter_link(
    target: &mut ParticleMeshEmitter,
    pending: &ParticleMeshEmitterPending,
    object_count: usize,
) -> Result<(), GameError> {
    let present: Vec<u8> = pending
        .mesh_indices
        .iter()
        .map(|&index| u8::from(index != NULL_REFERENCE))
        .collect();
    resolve_particle_mesh_emitter_link_with_presence(target, pending, object_count, &present)
}

/// IDA: 0074CA80 calls 0074C910 for each nonnull resolved mesh in array order. The host verifies slot and presence
/// agreement, builds all metadata first, then publishes the cache at once. Completion: 100% of the checked metadata
/// publication; NiRefObject side effects remain open.
/// TODO: [ ] Bind actual NiGeometry and NiSkinInstance graph owners and flags.
pub fn resolve_particle_mesh_emitter_skin_cache(
    target: &mut ParticleMeshEmitter,
    links: &[ParticleMeshSkinLinkView],
) -> Result<(), GameError> {
    if links.len() != target.mesh_indices.len() {
        return Err(GameError::unclassified(
            0,
            "mesh emitter skin cache slot count mismatch",
        ));
    }
    for link in links {
        if !link.geometry_present {
            return Err(GameError::unclassified(
                0,
                "mesh emitter skin cache geometry mismatch",
            ));
        }
        if !link.skin_instance_present && (link.skin_data_present || link.partition_owner.is_some())
        {
            return Err(GameError::unclassified(
                0,
                "mesh emitter skin metadata without instance",
            ));
        }
    }
    target.skin_cache = prepare_particle_mesh_skin_cache(links);
    Ok(())
}

/// IDA: N/A the host checks that precede any store in a fresh link.
fn check_mesh_emitter_link(
    target: &ParticleMeshEmitter,
    pending: &ParticleMeshEmitterPending,
    object_count: usize,
) -> Result<(), GameError> {
    if target.link_open {
        return Err(GameError::unclassified(
            0,
            "particle mesh emitter link already open",
        ));
    }
    if pending.base.base.target_index != NULL_REFERENCE
        && pending.base.base.target_index as usize >= object_count
    {
        return Err(GameError::unclassified(
            0,
            "particle mesh emitter target link out of range",
        ));
    }
    for &index in &pending.mesh_indices {
        if index != NULL_REFERENCE && index as usize >= object_count {
            return Err(GameError::unclassified(
                0,
                "particle mesh emitter geometry link out of range",
            ));
        }
    }
    Ok(())
}

/// IDA: 0074CA80's modifier link precedes the deferred list's preallocation, then 0074C910 is called once for each raw
/// slot. Completion: 100% of the capacity and cursor setup for a fresh typed link; allocator and refcounts are open.
/// TODO: [ ] NiTArray growth and malformed stream partial effects.
pub fn begin_particle_mesh_emitter_link(
    target: &mut ParticleMeshEmitter,
    pending: &ParticleMeshEmitterPending,
    object_count: usize,
) -> Result<(), GameError> {
    check_mesh_emitter_link(target, pending, object_count)?;
    target.state = pending.base.clone();
    target.raw_mesh_indices.clone_from(&pending.mesh_indices);
    target.initial_velocity_type_70 = pending.initial_velocity_type;
    target.emission_type_74 = pending.emission_type;
    target.emission_axis_78 = pending.emission_axis;
    target.mesh_indices.clear();
    target.linked_geometry_54.clear();
    target.skin_cache.clear();
    target.mesh_indices.reserve(target.raw_mesh_indices.len());
    target.skin_cache.reserve(target.raw_mesh_indices.len());
    target.next_raw_link_slot = 0;
    target.link_open = true;
    Ok(())
}

/// IDA: 0074C910 skips a null geometry, otherwise 0074C5D0 publishes the compacted geometry slot before 0074C6A0 publishes
/// its cache entry; the caller may then write NiGeometryData +0x30 before advancing slots. `geometry` is the resolved
/// slot, or `None` for a null one. Completion: 100% of the per-slot publication order.
/// TODO: [ ] Bind NiSkinnedEmitterData's 0xC heap object and intrusive refcounts.
pub fn append_particle_mesh_emitter_link_slot(
    target: &mut ParticleMeshEmitter,
    geometry: Option<Entity>,
    link: &ParticleMeshSkinLinkView,
) -> Result<(), GameError> {
    let resolved_present = geometry.is_some();
    if !target.link_open || target.next_raw_link_slot >= target.raw_mesh_indices.len() {
        return Err(GameError::unclassified(
            0,
            "particle mesh emitter raw link slot unavailable",
        ));
    }
    let index = target.raw_mesh_indices[target.next_raw_link_slot];
    if link.geometry_present != resolved_present
        || (index == NULL_REFERENCE && resolved_present)
        || (!link.skin_instance_present
            && (link.skin_data_present
                || link.partition_owner.is_some()
                || link.skin_instance_owner.is_some()))
    {
        return Err(GameError::unclassified(
            0,
            "particle mesh emitter link slot mismatch",
        ));
    }
    if let Some(geometry) = geometry {
        let cache = make_mesh_skin_cache_entry(link);
        target.mesh_indices.push(index);
        target.linked_geometry_54.push(geometry);
        target.skin_cache.push(cache);
    }
    target.next_raw_link_slot += 1;
    Ok(())
}

/// IDA: 0074C1E0 NiPSysMeshEmitter's CreateClone after 0074EE00 NiPSysEmitter_CopyMembers (the emitter fields, which the
/// host copy holds): 0074A8C0 gives the clone's +0x50 geometry array the original's count as its size and no items, so
/// its geometries and their skin caches start empty; +0x70, +0x74 and +0x78..+0x80 are copied. Completion: 100%.
pub fn clone_mesh_emitter_members(clone: &mut ParticleMeshEmitter) {
    clone.linked_geometry_54.clear();
    clone.skin_cache.clear();
    clone.link_open = false;
}

/// IDA: 0074C910 NiPSysMeshEmitter_AddGeometry as 0074CA00's ProcessClone calls it: a geometry goes into the first empty
/// slot of +0x50 and its 0xC NiSkinnedEmitterData (with the skin instance's +0x0C partition) into +0x60; the caller then
/// ORs the data's +0x30 keep flags with 0x33. Completion: 100% of the stores the host keeps.
pub fn add_cloned_mesh_geometry(
    target: &mut ParticleMeshEmitter,
    geometry: Entity,
    link: &ParticleMeshSkinLinkView,
) {
    target.linked_geometry_54.push(geometry);
    target.skin_cache.push(make_mesh_skin_cache_entry(link));
}

/// IDA: 0074CA80 exits after the final deferred slot. A host guard for complete ordered slot consumption, with no original
/// tail effect. Completion: 100% of the scoped check.
/// TODO: [ ] Bind the NiStream deferred-list cursor.
pub fn finish_particle_mesh_emitter_link(
    target: &mut ParticleMeshEmitter,
) -> Result<(), GameError> {
    if !target.link_open || target.next_raw_link_slot != target.raw_mesh_indices.len() {
        return Err(GameError::unclassified(
            0,
            "particle mesh emitter link slots incomplete",
        ));
    }
    target.link_open = false;
    Ok(())
}

/// IDA: 0074CA80 / 0074C910 as the stream graph runs them on real geometry: for each raw slot in list order, a null slot
/// advances; a nonnull one is retained even as a forward cold NiTriShape or NiTriStrips, observing its current +0xB4 and
/// +0xB8 now; 0074C5D0 and 0074C6A0 publish, then the data's +0x30 keep flags take |= 0x33 before the next slot. Range
/// errors come first, as in `begin`; the class check then runs before any store, where the C++ graph fails midway and
/// drops the whole graph. The host NiTriStrips has no +0xB8 owner, so a skinned one stops. Completion: 100% of the slot and
/// store order.
/// TODO: [ ] NiSkinnedEmitterData's heap object and refcounts. 0074A2D0's cache fill and 0072ED50's lazy partition are not
/// ported in C++ either: `needs_lazy_partition` records the case and nothing fills it.
pub fn link_particle_mesh_emitter(
    world: &mut World,
    emitter: Entity,
    pending: &ParticleMeshEmitterPending,
    table: &StreamObjectTable,
) -> Result<MeshTriShapeLinkStatus, GameError> {
    const CHECKED: &str = "checked above";
    let target = world
        .get::<ParticleMeshEmitter>(emitter)
        .ok_or(GameError::unclassified(
            0,
            "particle mesh emitter owner mismatch",
        ))?;
    check_mesh_emitter_link(target, pending, table.objects.len())?;
    let mut slots = Vec::with_capacity(pending.mesh_indices.len());
    for &index in &pending.mesh_indices {
        if index == NULL_REFERENCE {
            slots.push(None);
            continue;
        }
        let geometry = table.objects[index as usize].entity;
        let supported = match world.get::<NiClass>(geometry) {
            Some(NiClass::TriShape) => true,
            Some(NiClass::TriStrips) => world
                .get::<MeshTriShapePayload>(geometry)
                .is_some_and(|payload| payload.skin_index == NULL_REFERENCE),
            _ => false,
        };
        if !supported {
            return Ok(MeshTriShapeLinkStatus::UnsupportedOwner);
        }
        slots.push(Some(geometry));
    }
    begin_particle_mesh_emitter_link(
        &mut world
            .get_mut::<ParticleMeshEmitter>(emitter)
            .expect(CHECKED),
        pending,
        table.objects.len(),
    )?;
    for geometry in slots {
        let link = geometry.map_or_else(ParticleMeshSkinLinkView::default, |geometry| {
            mesh_skin_link_view(world, geometry)
        });
        let data = geometry
            .and_then(|geometry| world.get::<NiGeometry>(geometry))
            .and_then(|links| links.data_b4);
        append_particle_mesh_emitter_link_slot(
            &mut world
                .get_mut::<ParticleMeshEmitter>(emitter)
                .expect(CHECKED),
            geometry,
            &link,
        )?;
        if let Some(data) = data {
            mark_mesh_emitter_keep_flags(world, data);
        }
    }
    finish_particle_mesh_emitter_link(
        &mut world
            .get_mut::<ParticleMeshEmitter>(emitter)
            .expect(CHECKED),
    )?;
    Ok(MeshTriShapeLinkStatus::Linked)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
    }

    fn payload() -> ParticleMeshEmitterPending {
        let mut bytes = Vec::new();
        word(&mut bytes, 4);
        bytes.extend(b"Mesh");
        word(&mut bytes, 3); // order
        word(&mut bytes, u32::MAX); // null target
        bytes.push(1); // active
        for _ in 0..14 {
            word(&mut bytes, 0); // emitter scalar fields, including modern radius variation
        }
        word(&mut bytes, 4); // raw mesh references
        word(&mut bytes, 0);
        word(&mut bytes, u32::MAX);
        word(&mut bytes, 1);
        word(&mut bytes, 0);
        word(&mut bytes, 7); // raw velocity mode
        word(&mut bytes, 3); // face-point mode
        word(&mut bytes, 0x3F80_0000);
        word(&mut bytes, 0x8000_0000);
        word(&mut bytes, 0);
        let size = bytes.len();
        bytes.push(0xA5);
        let parsed = parse_particle_mesh_emitter_payload(&bytes, 0x1400_0004).unwrap();
        assert_eq!(parsed.bytes_consumed, size);
        assert_eq!(parsed.base.base.name.as_deref(), Some(&b"Mesh"[..]));
        assert_eq!(parsed.base.base.order, 3);
        assert_eq!(parsed.mesh_indices.len(), 4);
        assert_eq!(parsed.initial_velocity_type, 7);
        assert_eq!(parsed.emission_type, 3);
        assert_eq!(parsed.emission_axis[0], 1.0);
        assert_eq!(parsed.emission_axis[1].to_bits(), 0x8000_0000);
        assert!(parse_particle_mesh_emitter_payload(&bytes[..size - 1], 0x1400_0004).is_err());
        parsed
    }

    #[derive(Default)]
    struct Runtime<'g> {
        draws: Vec<u16>,
        next: usize,
        geometry: Option<ParticleMeshGeometryView<'g>>,
        table: Option<SqrtMantissaTable>,
    }

    impl<'g> ParticleMeshEmitterRuntime<'g> for Runtime<'g> {
        fn draw_rand15(&mut self) -> u16 {
            assert!(
                self.next < self.draws.len(),
                "mesh path consumed an unexpected RNG draw"
            );
            self.next += 1;
            self.draws[self.next - 1]
        }

        fn geometry_at(&mut self, _slot: usize) -> Option<ParticleMeshGeometryView<'g>> {
            self.geometry
        }

        fn sqrt_mantissas(&self) -> Option<&SqrtMantissaTable> {
            self.table.as_ref()
        }
    }

    #[test]
    fn link_and_normalizer() {
        let mut emitter = ParticleMeshEmitter::default();
        assert!(emitter.mesh_indices.is_empty());
        assert!(emitter.raw_mesh_indices.is_empty());
        assert_eq!(emitter.emission_axis_78, [1.0, 0.0, 0.0]);
        let pending = payload();
        let present = [1, 0, 0, 1];
        resolve_particle_mesh_emitter_link_with_presence(&mut emitter, &pending, 2, &present)
            .unwrap();
        assert_eq!(emitter.raw_mesh_indices, pending.mesh_indices);
        assert_eq!(emitter.mesh_indices, [0, 0]);
        assert_eq!(emitter.emission_type_74, 3);
        let mut invalid = present;
        invalid[1] = 1;
        assert!(
            resolve_particle_mesh_emitter_link_with_presence(&mut emitter, &pending, 2, &invalid)
                .is_err()
        );
        assert_eq!(emitter.mesh_indices, [0, 0]);
        let mut zero = [0.0; 3];
        assert_eq!(
            normalize_particle_mesh_averaged_normal(&mut zero, None),
            ParticleMeshNormalizeResult::Normalized
        );
        assert_eq!(zero, [0.0; 3]);
        let mut nonzero = [1.0, 0.0, 0.0];
        assert_eq!(
            normalize_particle_mesh_averaged_normal(&mut nonzero, None),
            ParticleMeshNormalizeResult::MissingTable
        );
        assert_eq!(nonzero, [1.0, 0.0, 0.0]);
        let table = SqrtMantissaTable::default();
        assert_eq!(
            normalize_particle_mesh_averaged_normal(&mut nonzero, Some(&table)),
            ParticleMeshNormalizeResult::Normalized
        );
        assert_eq!(nonzero[0], 1.0);
    }

    fn pending_with(velocity_type: u32, emission: u32) -> ParticleMeshEmitterPending {
        ParticleMeshEmitterPending {
            mesh_indices: vec![0],
            initial_velocity_type: velocity_type,
            emission_type: emission,
            ..ParticleMeshEmitterPending::default()
        }
    }

    fn linked(velocity_type: u32, emission: u32) -> ParticleMeshEmitter {
        let mut emitter = ParticleMeshEmitter::default();
        resolve_particle_mesh_emitter_link(&mut emitter, &pending_with(velocity_type, emission), 1)
            .unwrap();
        emitter
    }

    #[test]
    fn unskinned_draws_and_missing_arrays() {
        let empty = ParticleMeshEmitter::default();
        let mut no_draw = Runtime::default();
        let mut untouched_position = [9.0; 3];
        let mut untouched_velocity = [1.0, 2.0, 3.0];
        assert!(
            !empty
                .generate(
                    &mut untouched_position,
                    &mut untouched_velocity,
                    &mut no_draw
                )
                .unwrap()
        );
        assert_eq!(no_draw.next, 0);
        assert_eq!(untouched_position, [9.0; 3]);
        let emitter = linked(7, 0);
        let vertices = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
        let geometry = ParticleMeshGeometryView {
            vertex_count: 2,
            vertices: &vertices,
            mesh_to_target: NiTransform {
                translation: [10.0, 0.0, 0.0],
                ..NiTransform::default()
            },
            ..ParticleMeshGeometryView::default()
        };
        let mut runtime = Runtime {
            geometry: Some(geometry),
            draws: vec![0, 1],
            ..Runtime::default()
        };
        let mut position = [0.0; 3];
        let mut velocity = [0.0, 2.0, 0.0];
        assert!(
            emitter
                .generate(&mut position, &mut velocity, &mut runtime)
                .unwrap()
        );
        assert_eq!(runtime.next, 2);
        assert_eq!(position, [14.0, 5.0, 6.0]);
        assert_eq!(velocity, [0.0, 2.0, 0.0]);
        runtime.next = 0;
        runtime.geometry.as_mut().unwrap().vertices = &[];
        position = [9.0; 3];
        assert!(
            !emitter
                .generate(&mut position, &mut velocity, &mut runtime)
                .unwrap()
        );
        assert_eq!(runtime.next, 2);
        assert_eq!(position, [9.0; 3]);
        runtime.next = 0;
        runtime.geometry = None;
        assert!(
            !emitter
                .generate(&mut position, &mut velocity, &mut runtime)
                .unwrap()
        );
        assert_eq!(runtime.next, 1);
    }

    #[test]
    fn line_edge_and_triangle_retry() {
        let vertices = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
        let geometry = ParticleMeshGeometryView {
            vertex_count: 3,
            vertices: &vertices,
            topology: ParticleMeshTopology::Lines,
            ..ParticleMeshGeometryView::default()
        };
        let emitter = linked(7, 2);
        let mut runtime = Runtime {
            geometry: Some(geometry),
            draws: vec![0, 0],
            ..Runtime::default()
        };
        let mut position = [0.0; 3];
        let mut velocity = [0.0, 1.0, 0.0];
        assert!(
            emitter
                .generate(&mut position, &mut velocity, &mut runtime)
                .unwrap()
        );
        assert_eq!(runtime.next, 2);
        assert_eq!(position, [1.0, 0.0, 0.0]);
        let emitter = linked(7, 4);
        runtime.draws = vec![0, 0, 32767];
        runtime.next = 0;
        assert!(
            emitter
                .generate(&mut position, &mut velocity, &mut runtime)
                .unwrap()
        );
        assert_eq!(runtime.next, 3);
        assert_eq!(position, vertices[1]);
        let triangles = [[0, 0, 0], [0, 1, 2]];
        {
            let view = runtime.geometry.as_mut().unwrap();
            view.topology = ParticleMeshTopology::Triangles;
            view.triangles = &triangles;
        }
        let emitter = linked(7, 1);
        runtime.draws = vec![0, 0];
        runtime.next = 0;
        assert!(
            emitter
                .generate(&mut position, &mut velocity, &mut runtime)
                .unwrap()
        );
        assert_eq!(runtime.next, 2);
    }

    #[test]
    fn face_and_weighted_skin() {
        let vertices = [[0.0, 0.0, 0.0], [3.0, 0.0, 0.0], [0.0, 3.0, 0.0]];
        let triangles = [[0, 1, 2]];
        let geometry = ParticleMeshGeometryView {
            vertex_count: 3,
            vertices: &vertices,
            topology: ParticleMeshTopology::Triangles,
            triangles: &triangles,
            ..ParticleMeshGeometryView::default()
        };
        let emitter = linked(7, 3);
        let mut runtime = Runtime {
            geometry: Some(geometry),
            draws: vec![0, 0, 0, 0],
            ..Runtime::default()
        };
        let mut position = [0.0; 3];
        let mut velocity = [1.0, 0.0, 0.0];
        assert!(
            emitter
                .generate(&mut position, &mut velocity, &mut runtime)
                .unwrap()
        );
        assert_eq!(runtime.next, 4);
        assert_eq!(position, vertices[0]);

        let skin_vertices = [[2.0, 0.0, 0.0]];
        let normals = [[1.0, 0.0, 0.0]];
        let bones = [0u16];
        let weights = [1.0, 0.0, 0.0, 0.0];
        let vertex_map = [0u16];
        let partition = ParticleMeshSkinPartitionView {
            bones: &bones,
            weights: &weights,
            vertex_map: &vertex_map,
            vertex_count: 1,
            bone_count: 1,
            weights_per_vertex: 1,
            ..ParticleMeshSkinPartitionView::default()
        };
        let partitions = [partition];
        let bone_frames = [NiTransform::default()];
        let skin = ParticleMeshSkinResolvedView {
            bone_world: &bone_frames,
            skin_bone_transform: &bone_frames,
            partitions: &partitions,
            ..ParticleMeshSkinResolvedView::default()
        };
        let skinned = ParticleMeshGeometryView {
            vertex_count: 1,
            vertices: &skin_vertices,
            normals: &normals,
            skinned: true,
            skin: Some(skin),
            ..geometry
        };
        let emitter = linked(0, 0);
        runtime.geometry = Some(skinned);
        runtime.draws = vec![0, 0, 0];
        runtime.next = 0;
        velocity = [0.0, 2.0, 0.0];
        assert!(
            emitter
                .generate(&mut position, &mut velocity, &mut runtime)
                .unwrap()
        );
        assert_eq!(runtime.next, 3);
        assert_eq!(position, skin_vertices[0]);
        assert_eq!(velocity, [2.0, 0.0, 0.0]);
        runtime.geometry.as_mut().unwrap().vertices = &[];
        runtime.next = 0;
        assert!(
            !emitter
                .generate(&mut position, &mut velocity, &mut runtime)
                .unwrap()
        );
        assert_eq!(runtime.next, 3);
    }

    /// Beyond the C++ test: the stream graph's ordered link path and 0074C910's metadata.
    #[test]
    fn ordered_link_slots_and_cache() {
        let mut world = World::new();
        let partition_owner = world.spawn_empty().id();
        let mut emitter = ParticleMeshEmitter::default();
        let pending = ParticleMeshEmitterPending {
            mesh_indices: vec![0, NULL_REFERENCE],
            ..ParticleMeshEmitterPending::default()
        };
        assert!(finish_particle_mesh_emitter_link(&mut emitter).is_err());
        begin_particle_mesh_emitter_link(&mut emitter, &pending, 1).unwrap();
        assert!(begin_particle_mesh_emitter_link(&mut emitter, &pending, 1).is_err());
        let link = ParticleMeshSkinLinkView {
            geometry_present: true,
            skin_instance_present: true,
            skin_data_present: true,
            geometry_data_present: true,
            geometry_keep_flags: 0x40,
            partition_owner: Some(partition_owner),
            ..ParticleMeshSkinLinkView::default()
        };
        let geometry = world.spawn_empty().id();
        append_particle_mesh_emitter_link_slot(&mut emitter, Some(geometry), &link).unwrap();
        assert!(finish_particle_mesh_emitter_link(&mut emitter).is_err());
        append_particle_mesh_emitter_link_slot(
            &mut emitter,
            None,
            &ParticleMeshSkinLinkView::default(),
        )
        .unwrap();
        finish_particle_mesh_emitter_link(&mut emitter).unwrap();
        assert_eq!(emitter.mesh_indices, [0]);
        assert_eq!(emitter.linked_geometry_54, [geometry]);
        let cache = &emitter.skin_cache[0];
        assert!(cache.cache_requested);
        assert!(!cache.needs_lazy_partition);
        assert!(cache.lazy_partition_source_present);
        assert_eq!(cache.geometry_keep_flags_after, Some(0x73));
        assert_eq!(cache.partition_owner, Some(partition_owner));
        let lazy = ParticleMeshSkinLinkView {
            partition_owner: None,
            ..link
        };
        resolve_particle_mesh_emitter_skin_cache(&mut emitter, &[lazy]).unwrap();
        assert!(emitter.skin_cache[0].needs_lazy_partition);
        assert!(resolve_particle_mesh_emitter_skin_cache(&mut emitter, &[]).is_err());
    }
}
