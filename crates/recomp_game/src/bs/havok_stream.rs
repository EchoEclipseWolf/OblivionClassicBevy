//! The stream half of the three `bhk*` classes the preload fire models hold: bhkSPCollisionObject, bhkSimpleShapePhantom and
//! bhkConvexVerticesShape, one of each in 8 of `00443550`'s models. Their blocks read and link in original order, so a tree
//! builds and the stream stays in step.
//!
//! Host substitution (D2): the Havok SDK runtime isn't ported, so these hold what the stream gave them. The original keeps a
//! heap cinfo at +0x0C until vt+0x70 turns it into a hk object during the link; here the cinfo stays, and no hk object,
//! `00897670`'s `008BC750` or `0089D6C0`'s vt+0x70 runs. Nothing reads them until a physics phase.
//! Evidence: linux-capture-2026-11-06/run-d-models; P15-RUN-001's trees of these models.

use bevy_ecs::prelude::*;
use recomp_core::reader::{ByteReader, StreamError};

/// IDA: 00712B.. the floor of 007008A0's modern prefix: from here it reads nothing.
const MODERN_FLOOR: u32 = 0x0A01_0072;
/// IDA: 008BD2D0: bhkSimpleShapePhantom's vt+0x68, the bytes 0089D670 reads into its cinfo.
const PHANTOM_CINFO_BYTES: usize = 96;
/// IDA: 00564C90: bhkConvexVerticesShape's vt+0x68.
const CONVEX_CINFO_BYTES: usize = 32;
/// IDA: 008A25C0: the highest material +0x10 keeps, plus one.
const MATERIAL_COUNT: u32 = 31;
/// IDA: 008C8A30's `+0x04 = flt_B2EFC4` over what the stream read, the shape's radius.
const CONVEX_RADIUS_BITS: u32 = 0x3DCC_CCCD;

const TRUNCATED: &str = "truncated bhk block";

/// IDA: bhkSPCollisionObject (0x14): NiCollisionObject +0x08 the target, +0x0C the flags, +0x10 the phantom.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BhkSpCollisionObject {
    pub target_08: Option<Entity>,
    pub flags_0c: u16,
    pub body_10: Option<Entity>,
}

/// IDA: 0088ED20 bhkBlendCollisionObject_Create (0x28, 00897600 over bhkCollisionObject, vtable A9643C): +0x14 0.0, +0x18 1.0,
/// +0x1C 8, +0x20 and +0x24 0.0; the shared bhkSPCollisionObject part rides on `BhkSpCollisionObject`. The floats are raw bits.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BhkBlendCollisionObject {
    pub float_14: u32,
    pub float_18: u32,
    pub field_1c: u32,
    pub float_20: u32,
    pub float_24: u32,
}

impl Default for BhkBlendCollisionObject {
    /// IDA: N/A 0088ED20's stores.
    fn default() -> Self {
        Self {
            float_14: 0,
            float_18: 1.0_f32.to_bits(),
            field_1c: 8,
            float_20: 0,
            float_24: 0,
        }
    }
}

/// IDA: 00BA7A1C, the count of bhkBlendCollisionObjects built: both constructors (0088ED20's inline one and 0088EB60) raise it.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlendCollisionStatics {
    pub count_ba7a1c: u32,
}

/// IDA: 0088E7C0 bhkBlendCollisionObjectAddRotation's constructor (0x4C, vtable A96384): 0088EB60's bhkBlendCollisionObject, then
/// g_niMatrix3Identity at +0x28, raw bits.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BhkBlendCollisionAddRotation {
    pub rotation_28: [u32; 9],
}

impl Default for BhkBlendCollisionAddRotation {
    /// IDA: N/A the identity rotation.
    fn default() -> Self {
        let one = 1.0_f32.to_bits();
        Self {
            rotation_28: [one, 0, 0, 0, one, 0, 0, 0, one],
        }
    }
}

/// IDA: 0088F2D0 bhkBlendCollisionObject_LoadBinary: bhkCollisionObject_LoadBinary, then u32s into +0x14 and +0x18; a file
/// user version below 8 reads two more words and drops them. Completion: 100%.
/// # Errors
/// A truncated payload or an old block.
pub fn parse_blend_collision_payload(
    bytes: &[u8],
    version: u32,
    file_user_version: u32,
) -> Result<(BhkSpCollisionPayload, [u32; 2], usize), StreamError> {
    let base = parse_sp_collision_payload(bytes, version)?;
    let mut reader = ByteReader::new(&bytes[base.bytes_consumed..]);
    let floats = [reader.u32(TRUNCATED)?, reader.u32(TRUNCATED)?];
    if file_user_version < 8 {
        reader.u32(TRUNCATED)?;
        reader.u32(TRUNCATED)?;
    }
    Ok((base, floats, base.bytes_consumed + reader.position()))
}

/// IDA: 00B2E660, the body part pairs 0088ECD0 reads: pair 0 has no writer and stays 1.0, 1.0 (the image); pairs 1.. are
/// 0053A720's sHead:DEFAULT and on (`BlendSettingsState`), the image's 1.0 where the transfer set nothing.
#[must_use]
pub fn blend_part_pair(
    blend: Option<&crate::tes::blend_settings::BlendSettingsState>,
    part: u32,
) -> [f32; 2] {
    if part == 0 {
        return [1.0, 1.0];
    }
    let address = 0x00B2_E660 + 8 * part;
    let get = |address: u32| {
        blend
            .and_then(|state| state.values.get(&address).copied())
            .unwrap_or(1.0)
    };
    [get(address), get(address + 4)]
}

/// IDA: 008BEFC0 bhkMalleableConstraint (0x10, bhkConstraint; cinfo hkMalleableConstraintCinfo, 008BF490): the wrapped
/// constraint's type (008E8040's u32, 008E7E60's cinfo class), its own priority, and 008BF230's two floats at cinfo +0x10 and
/// +0x14 (tau, then strength). The wrapped data sits in the shared `BhkConstraint`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BhkMalleableConstraint {
    pub wrapped_type: u32,
    pub wrapped_priority: u32,
    pub tau_10: u32,
    pub strength_14: u32,
}

/// IDA: 008BF130 bhkMalleableConstraint_LoadBinary at user version 6 or more: bhkConstraint_LoadBinary, whose cinfo vt+4
/// (008BF230) reads bhkConstraintCinfo_LoadBase (the entity list, the priority), then 008E8040: a u32 type, 008E7E60's cinfo for
/// it and that cinfo's vt+4 (its own entity list and priority, then its data: a ragdoll, 7, through 008C0CB0, a zero last float
/// becoming 10.0; a limited hinge, 2, through 008B30D0); then the two floats. Other wrapped types are a stop. Returns the outer
/// constraint, the malleable part, the outer entities and the bytes read; the wrapped entities are read and resolved (008BFAA0)
/// but kept nowhere. Completion: 100% of the ragdoll and limited hinge.
/// # Errors
/// A truncated payload, an old block, or another wrapped type.
pub fn parse_malleable_constraint_payload(
    bytes: &[u8],
    version: u32,
    file_user_version: u32,
) -> Result<(BhkConstraint, BhkMalleableConstraint, Vec<u32>, usize), StreamError> {
    if version < MODERN_FLOOR || file_user_version < 6 {
        return Err(StreamError("008BF13B: an old bhkMalleableConstraint"));
    }
    let mut reader = ByteReader::new(bytes);
    let entities = read_reference_list(&mut reader)?;
    let priority_08 = reader.u32(TRUNCATED)?;
    let wrapped_type = reader.u32(TRUNCATED)?;
    let _wrapped_entities = read_reference_list(&mut reader)?;
    let wrapped_priority = reader.u32(TRUNCATED)?;
    let (size, ragdoll) = match wrapped_type {
        7 => (48 + 48 + 24, true),
        2 => (64 + 48 + 12, false),
        _ => {
            return Err(StreamError(
                "008E7E60: a malleable constraint wrapping another type",
            ));
        }
    };
    let mut data = reader.take(size, TRUNCATED)?.to_vec();
    if ragdoll && f32::from_bits(word_at(&data, size - 4)) == 0.0 {
        set_word(&mut data, size - 4, 10.0_f32.to_bits());
    }
    let tau_10 = reader.u32(TRUNCATED)?;
    let strength_14 = reader.u32(TRUNCATED)?;
    Ok((
        BhkConstraint {
            priority_08,
            data,
            entity_a: None,
            entity_b: None,
        },
        BhkMalleableConstraint {
            wrapped_type,
            wrapped_priority,
            tau_10,
            strength_14,
        },
        entities,
        reader.position(),
    ))
}

/// IDA: 00897B90's payload: 00711C90's target reference, the u16, then the phantom's reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BhkSpCollisionPayload {
    pub target_index: u32,
    pub flags: u16,
    pub body_index: u32,
    pub bytes_consumed: usize,
}

/// IDA: 00897B90 after 007008A0. Completion: 100% at the modern floor.
pub fn parse_sp_collision_payload(
    bytes: &[u8],
    version: u32,
) -> Result<BhkSpCollisionPayload, StreamError> {
    if version < MODERN_FLOOR {
        return Err(StreamError("legacy bhk block unsupported"));
    }
    let mut reader = ByteReader::new(bytes);
    let target_index = reader.u32(TRUNCATED)?;
    let flags = reader.u16(TRUNCATED)?;
    let body_index = reader.u32(TRUNCATED)?;
    Ok(BhkSpCollisionPayload {
        target_index,
        flags,
        body_index,
        bytes_consumed: reader.position(),
    })
}

/// IDA: 00897810's store on the target, word +0x18. Completion: 100%.
#[must_use]
pub const fn collision_target_flags(flags: u16) -> u16 {
    flags & 0xFFEF | 6
}

/// IDA: bhkSimpleShapePhantom (0x14): the cinfo of 008BD5A0, as read. `shape` is 0089F350's store at cinfo +0x04.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct BhkSimpleShapePhantom {
    pub cinfo_0c: Vec<u8>,
    pub shape: Option<Entity>,
}

/// IDA: 0089F8E0's payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BhkPhantomPayload {
    pub shape_index: u32,
    pub cinfo: Vec<u8>,
    pub bytes_consumed: usize,
}

/// IDA: 0089F8E0 bhkWorldObject_LoadBinary: the shape's reference, 0089D650 (007008A0, then vt+0x6C's raw cinfo of `cinfo_bytes`),
/// the cinfo's array at +0x0C emptied (+0x0C and +0x10 zero, +0x14 its bit 30 kept and bit 31 set), then the first word's
/// two fixes. Completion: 100% at the modern floor. `file_user_version` is NiStream +0x04.
fn parse_world_object(
    reader: &mut ByteReader<'_>,
    file_user_version: u32,
    cinfo_bytes: usize,
) -> Result<(u32, Vec<u8>), StreamError> {
    let shape_index = reader.u32(TRUNCATED)?;
    let mut cinfo = reader.take(cinfo_bytes, TRUNCATED)?.to_vec();
    let capacity = word_at(&cinfo, 0x14);
    set_word(&mut cinfo, 0x0C, 0);
    set_word(&mut cinfo, 0x10, 0);
    set_word(&mut cinfo, 0x14, capacity & 0x4000_0000 | 0x8000_0000);
    let mut word = word_at(&cinfo, 0);
    if file_user_version < 9 && word & 0x20 != 0 {
        word = word & 0xFFFF_7FDF | 0x8000;
    }
    if word & 0x3F == 8 {
        word &= !0x4000;
    }
    set_word(&mut cinfo, 0, word);
    Ok((shape_index, cinfo))
}

// IDA: N/A a little-endian word of a raw cinfo.
fn word_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

// IDA: N/A a store into a raw cinfo.
fn set_word(bytes: &mut [u8], offset: usize, word: u32) {
    bytes[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
}

/// IDA: 0089F8E0 for bhkSimpleShapePhantom (vt+0x68 008BD2D0, 96 bytes). Completion: 100% at the modern floor.
pub fn parse_phantom_payload(
    bytes: &[u8],
    version: u32,
    file_user_version: u32,
) -> Result<BhkPhantomPayload, StreamError> {
    if version < MODERN_FLOOR {
        return Err(StreamError("legacy bhk block unsupported"));
    }
    let mut reader = ByteReader::new(bytes);
    let (shape_index, cinfo) =
        parse_world_object(&mut reader, file_user_version, PHANTOM_CINFO_BYTES)?;
    Ok(BhkPhantomPayload {
        shape_index,
        cinfo,
        bytes_consumed: reader.position(),
    })
}

/// IDA: bhkConvexVerticesShape (0x14): +0x10 the material, then the cinfo of 008C8D40: +0x04 the radius, +0x08 and +0x14 the
/// two arrays of 16-byte vectors, kept as the raw words.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct BhkConvexVerticesShape {
    pub material_10: u32,
    pub cinfo_0c: Vec<u8>,
    pub radius_bits: u32,
    pub vertices: Vec<[u32; 4]>,
    pub normals: Vec<[u32; 4]>,
}

/// IDA: 008E83B0: a u32 count, then that many 16-byte vectors.
fn read_vectors(reader: &mut ByteReader<'_>) -> Result<Vec<[u32; 4]>, StreamError> {
    let count = reader.u32(TRUNCATED)? as usize;
    let mut vectors = Vec::new();
    for _ in 0..count {
        vectors.push([
            reader.u32(TRUNCATED)?,
            reader.u32(TRUNCATED)?,
            reader.u32(TRUNCATED)?,
            reader.u32(TRUNCATED)?,
        ]);
    }
    Ok(vectors)
}

/// IDA: 008C8A30: 008A25C0 (0089D650 with no reference; a first word at 31 or above becomes 0, moves to +0x10, and clears),
/// the radius, then the two arrays. Completion: 100% at the modern floor.
pub fn parse_convex_vertices_payload(
    bytes: &[u8],
    version: u32,
) -> Result<(BhkConvexVerticesShape, usize), StreamError> {
    if version < MODERN_FLOOR {
        return Err(StreamError("legacy bhk block unsupported"));
    }
    let mut reader = ByteReader::new(bytes);
    let mut cinfo = reader.take(CONVEX_CINFO_BYTES, TRUNCATED)?.to_vec();
    let word = u32::from_le_bytes([cinfo[0], cinfo[1], cinfo[2], cinfo[3]]);
    let material_10 = if word >= MATERIAL_COUNT { 0 } else { word };
    cinfo[..4].fill(0);
    let vertices = read_vectors(&mut reader)?;
    let normals = read_vectors(&mut reader)?;
    let consumed = reader.position();
    Ok((
        BhkConvexVerticesShape {
            material_10,
            cinfo_0c: cinfo,
            radius_bits: CONVEX_RADIUS_BITS,
            vertices,
            normals,
        },
        consumed,
    ))
}

/// IDA: N/A host substitution (D2) for a clone at a process scale other than 1: the original scales the rigid body's cinfo
/// (`008A2D60`) and clones its shape scaled (each shape's vt+0x18 and vt+0x80 with the process); with no hk objects the host
/// copies the data unchanged and records the scale here, for the physics seam to apply.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct BhkHostScale(pub f32);

/// IDA: bhkRigidBody / bhkRigidBodyT's vt+0x68 (00533350): the cinfo's 224 bytes.
const RIGID_BODY_CINFO_BYTES: usize = 224;

/// IDA: bhkRigidBody and bhkRigidBodyT (00A5605C, 00A980A4): the cinfo of 008A5980 as read, `shape` 0089F350's store at
/// cinfo +0x04, +0x10 the constraints and actions 008A46C0 kept, +0x18 the u32 008A44F0 reads last.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct BhkRigidBody {
    pub cinfo_0c: Vec<u8>,
    pub shape: Option<Entity>,
    pub constraints_10: Vec<Entity>,
    pub field_18: u32,
}

/// IDA: what 008A44F0 leaves for 008A50E0: the shape's and the constraint list's references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BhkRigidBodyPending {
    pub shape_index: u32,
    pub constraint_indices: Vec<u32>,
}

/// IDA: 008A44F0 bhkRigidBody_LoadBinary: 0089F8E0 with the 224-byte cinfo (vt+0x6C 008A5A20: 0089D670 at NiStream +0x04 6
/// or more); a cinfo +0xC4 above 250.0 (`dbl_A529C0`, unordered keeps it) becomes 250.0; a mass +0xB0 equal to 0.0 with
/// 00535AC0's inverse mass 0.0 (no hk object yet) and a layer (word 0 & 0x3F) other than 2 sets the motion byte +0xD0 to 7;
/// then the reference list (NiStream_ReadDeferredReferenceList) and the u32 at +0x18. Completion: 100% at user version 6
/// and above; below it is the "old bhkRigidBody" re-export arm, a stop.
pub fn parse_rigid_body_payload(
    bytes: &[u8],
    version: u32,
    file_user_version: u32,
) -> Result<(BhkRigidBody, BhkRigidBodyPending, usize), StreamError> {
    if version < MODERN_FLOOR || file_user_version < 6 {
        return Err(StreamError("008A5A3A: an old bhkRigidBody"));
    }
    let mut reader = ByteReader::new(bytes);
    let (shape_index, mut cinfo) =
        parse_world_object(&mut reader, file_user_version, RIGID_BODY_CINFO_BYTES)?;
    let damping = f32::from_bits(word_at(&cinfo, 0xC4));
    if f64::from(damping) > 250.0 {
        set_word(&mut cinfo, 0xC4, 250.0_f32.to_bits());
    }
    let mass = f32::from_bits(word_at(&cinfo, 0xB0));
    if mass == 0.0 && word_at(&cinfo, 0) & 0x3F != 2 {
        cinfo[0xD0] = 7;
    }
    let constraint_indices = read_reference_list(&mut reader)?;
    let field_18 = reader.u32(TRUNCATED)?;
    Ok((
        BhkRigidBody {
            cinfo_0c: cinfo,
            shape: None,
            constraints_10: Vec::new(),
            field_18,
        },
        BhkRigidBodyPending {
            shape_index,
            constraint_indices,
        },
        reader.position(),
    ))
}

/// IDA: 007124A0's list form, NiStream_ReadDeferredReferenceList: a u32 count, then that many references.
fn read_reference_list(reader: &mut ByteReader<'_>) -> Result<Vec<u32>, StreamError> {
    let count = reader.u32(TRUNCATED)?;
    let mut indices = Vec::new();
    for _ in 0..count {
        indices.push(reader.u32(TRUNCATED)?);
    }
    Ok(indices)
}

/// IDA: 008A25C0 bhkShape_LoadCinfoAndMaterial: 0089D650 (vt+0x6C's raw cinfo), then a first word at 31 or above reads as 0,
/// moves to +0x10, and clears. Completion: 100%.
fn read_shape_cinfo(
    reader: &mut ByteReader<'_>,
    size: usize,
) -> Result<(u32, Vec<u8>), StreamError> {
    let mut cinfo = reader.take(size, TRUNCATED)?.to_vec();
    let word = word_at(&cinfo, 0);
    let material = if word >= MATERIAL_COUNT { 0 } else { word };
    set_word(&mut cinfo, 0, 0);
    Ok((material, cinfo))
}

/// IDA: the shape classes the prison's models hold, as the stream gives them: +0x10 the material and the raw cinfo, `shapes`
/// the linked references (bhkNiTriStripsShape's NiTriStripsData entries, null ones kept; bhkMoppBvTreeShape's,
/// bhkTransformShape's and bhkConvexTransformShape's child; bhkListShape's nonnull children), `filters` the collision filters
/// (bhkNiTriStripsShape's per data, bhkListShape's +0x10 array), `mopp` bhkMoppBvTreeShape's hkMoppCode (its +0x10 origin
/// and scale words, then the code bytes).
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct BhkShape {
    pub material_10: u32,
    pub cinfo_0c: Vec<u8>,
    pub shapes: Vec<Option<Entity>>,
    pub filters: Vec<u32>,
    pub mopp: Option<([u32; 4], Vec<u8>)>,
}

/// IDA: which shape LoadBinary a constructor reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BhkShapeKind {
    /// bhkBoxShape: 008B6A30 (008AEAB0, 008A25C0), vt+0x68 00564C90, 32 bytes.
    Box,
    /// bhkCapsuleShape: 008B6A30, vt+0x68 008BDBE0, 48 bytes.
    Capsule,
    /// bhkSphereShape: 008AEAB0 bhkSphereRepShape_LoadBinary (bhkShape_LoadBinary), vt+0x68 00419750, 8 bytes.
    Sphere,
    /// bhkNiTriStripsShape: 008C74F0, vt+0x68 008BDBE0, 48 bytes (16 fewer below user version 4, 008C6560).
    NiTriStrips,
    /// bhkMoppBvTreeShape: 008C3640, vt+0x68 008C4830, 16 bytes.
    MoppBvTree,
    /// bhkConvexTransformShape: 008C96D0, vt+0x68 00563B80, 80 bytes.
    ConvexTransform,
    /// bhkListShape: 008A1090, vt+0x68 008904B0, 28 bytes.
    List,
    /// bhkTransformShape: 008A1DB0 (bhkMoppBvTreeShape's base load), vt+0x68 00563B80, 80 bytes; link 008A1DF0.
    Transform,
}

/// IDA: the references a shape's LinkObject resolves, in read order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BhkShapePending {
    pub kind: BhkShapeKind,
    pub indices: Vec<u32>,
}

/// IDA: the shape LoadBinarys at the modern floor and user version 6 or more: bhkBoxShape and bhkCapsuleShape the cinfo
/// alone; bhkNiTriStripsShape (008C74F0's arm for NiStream +0x04 2 or more) the cinfo, the data reference list, a u32 count
/// and that many filters (a filter with bit 5 below user version 9 becomes `& 0xFFFF7FDF | 0x8000`); bhkMoppBvTreeShape
/// (008A1DB0: the child reference, the cinfo, cinfo +0x04 = 0; 008E84B0: a u32 size, 16 bytes of origin and scale, the
/// code; NiStream +0x04 2 or more keeps it at cinfo +0x08); bhkTransformShape the child reference, the cinfo, cinfo +0x04 = 0; bhkConvexTransformShape the
/// child reference, the cinfo, cinfo +0x08 = 0; bhkListShape the child reference list, the cinfo, then 008E85E0's u32 count and filters into cinfo +0x10.
/// Completion: 100% at user version 6 and above; the older arms ("old bhkMeshShape", "old bhkMoppBvTreeShape") stop.
pub fn parse_shape_payload(
    bytes: &[u8],
    version: u32,
    file_user_version: u32,
    kind: BhkShapeKind,
) -> Result<(BhkShape, BhkShapePending, usize), StreamError> {
    if version < MODERN_FLOOR || file_user_version < 6 {
        return Err(StreamError("an old bhk shape"));
    }
    let mut reader = ByteReader::new(bytes);
    let mut shape = BhkShape::default();
    let mut indices = Vec::new();
    match kind {
        BhkShapeKind::Box => {
            (shape.material_10, shape.cinfo_0c) = read_shape_cinfo(&mut reader, 32)?
        }
        BhkShapeKind::Capsule => {
            (shape.material_10, shape.cinfo_0c) = read_shape_cinfo(&mut reader, 48)?;
        }
        BhkShapeKind::Sphere => {
            (shape.material_10, shape.cinfo_0c) = read_shape_cinfo(&mut reader, 8)?;
        }
        BhkShapeKind::NiTriStrips => {
            (shape.material_10, shape.cinfo_0c) = read_shape_cinfo(&mut reader, 48)?;
            indices = read_reference_list(&mut reader)?;
            let count = reader.u32(TRUNCATED)? as i32;
            for _ in 0..count.max(0) {
                let mut filter = reader.u32(TRUNCATED)?;
                if file_user_version < 9 && filter & 0x20 != 0 {
                    filter = filter & 0xFFFF_7FDF | 0x8000;
                }
                shape.filters.push(filter);
            }
        }
        BhkShapeKind::MoppBvTree => {
            indices.push(reader.u32(TRUNCATED)?);
            (shape.material_10, shape.cinfo_0c) = read_shape_cinfo(&mut reader, 16)?;
            set_word(&mut shape.cinfo_0c, 4, 0);
            let size = reader.u32(TRUNCATED)? as usize;
            let origin = [
                reader.u32(TRUNCATED)?,
                reader.u32(TRUNCATED)?,
                reader.u32(TRUNCATED)?,
                reader.u32(TRUNCATED)?,
            ];
            let code = reader.take(size, TRUNCATED)?.to_vec();
            shape.mopp = Some((origin, code));
        }
        BhkShapeKind::Transform => {
            indices.push(reader.u32(TRUNCATED)?);
            (shape.material_10, shape.cinfo_0c) = read_shape_cinfo(&mut reader, 80)?;
            set_word(&mut shape.cinfo_0c, 4, 0);
        }
        BhkShapeKind::ConvexTransform => {
            indices.push(reader.u32(TRUNCATED)?);
            (shape.material_10, shape.cinfo_0c) = read_shape_cinfo(&mut reader, 80)?;
            set_word(&mut shape.cinfo_0c, 8, 0);
        }
        BhkShapeKind::List => {
            indices = read_reference_list(&mut reader)?;
            (shape.material_10, shape.cinfo_0c) = read_shape_cinfo(&mut reader, 28)?;
            let count = reader.u32(TRUNCATED)? as i32;
            for _ in 0..count.max(0) {
                shape.filters.push(reader.u32(TRUNCATED)?);
            }
        }
    }
    Ok((shape, BhkShapePending { kind, indices }, reader.position()))
}

/// IDA: bhkRagdollConstraint, bhkLimitedHingeConstraint and bhkPrismaticConstraint (0x10, bhkConstraint): the cinfo object
/// of vt+0x74 (008C1620, 008B34B0, 008C1CB0), read by its vt+4 (008C0CB0, 008B30D0, 008C1B00): 008A0C30's entity reference
/// list and +0x08 priority, then the data in read order: the ragdoll's two 48-byte blocks and six words (a zero last float
/// becomes 10.0), the hinge's 64 and 48 bytes and three words, the prismatic's 80 and 48 bytes and three words. `entity_a` and `entity_b` are 008A0320's stores at cinfo +0x0C and +0x10.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct BhkConstraint {
    pub priority_08: u32,
    pub data: Vec<u8>,
    pub entity_a: Option<Entity>,
    pub entity_b: Option<Entity>,
}

/// IDA: 008A0940 bhkConstraint_LoadBinary at user version 6 or more: 0089D650, whose vt+0x6C (008A02C0) runs the cinfo's vt+4.
/// Host substitution: 008C0CB0's normalization of the ragdoll's four axes (SSE `rsqrtss` with a Newton step) isn't applied;
/// the raw axes stay, as no hk constraint is built. Completion: 100% of the reads.
/// IDA: the cinfo vt+4 that [`parse_constraint_payload`] follows: 008C0CB0, 008B30D0 or 008C1B00. IDA: N/A as a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstraintKind {
    Ragdoll,
    LimitedHinge,
    Prismatic,
}

pub fn parse_constraint_payload(
    bytes: &[u8],
    version: u32,
    file_user_version: u32,
    kind: ConstraintKind,
) -> Result<(BhkConstraint, Vec<u32>, usize), StreamError> {
    if version < MODERN_FLOOR || file_user_version < 6 {
        return Err(StreamError("008A0985: an old bhkConstraint"));
    }
    let mut reader = ByteReader::new(bytes);
    let entities = read_reference_list(&mut reader)?;
    let priority_08 = reader.u32(TRUNCATED)?;
    let ragdoll = kind == ConstraintKind::Ragdoll;
    let size = match kind {
        ConstraintKind::Ragdoll => 48 + 48 + 24,
        ConstraintKind::LimitedHinge => 64 + 48 + 12,
        ConstraintKind::Prismatic => 80 + 48 + 12,
    };
    let mut data = reader.take(size, TRUNCATED)?.to_vec();
    if ragdoll && f32::from_bits(word_at(&data, size - 4)) == 0.0 {
        set_word(&mut data, size - 4, 10.0_f32.to_bits());
    }
    Ok((
        BhkConstraint {
            priority_08,
            data,
            entity_a: None,
            entity_b: None,
        },
        entities,
        reader.position(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: u32 = 0x1400_0004;

    #[test]
    fn the_collision_object_reads_its_two_references_around_the_flags() {
        let mut bytes = 7_u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&0x81_u16.to_le_bytes());
        bytes.extend_from_slice(&9_u32.to_le_bytes());
        let payload = parse_sp_collision_payload(&bytes, VERSION).unwrap();
        assert_eq!(
            (payload.target_index, payload.flags, payload.body_index),
            (7, 0x81, 9)
        );
        assert_eq!(payload.bytes_consumed, 10);
    }

    // 00897829..00897836: bit 4 clears, bits 1 and 2 set, and the rest stay.
    #[test]
    fn the_collision_link_changes_three_flag_bits() {
        assert_eq!(collision_target_flags(0x0010), 0x0006);
        assert_eq!(collision_target_flags(0xFFFF), 0xFFEF);
        assert_eq!(collision_target_flags(0x0008), 0x000E);
    }

    #[test]
    fn the_phantom_fixes_the_first_word() {
        let mut bytes = 3_u32.to_le_bytes().to_vec();
        let mut cinfo = vec![0; PHANTOM_CINFO_BYTES];
        cinfo[..4].copy_from_slice(&0x4008_u32.to_le_bytes());
        bytes.extend_from_slice(&cinfo);
        let payload = parse_phantom_payload(&bytes, VERSION, 11).unwrap();
        assert_eq!(payload.shape_index, 3);
        assert_eq!(payload.bytes_consumed, 100);
        assert_eq!(payload.cinfo[..4], 8_u32.to_le_bytes());

        cinfo[..4].copy_from_slice(&0x21_u32.to_le_bytes());
        let mut bytes = 3_u32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&cinfo);
        let old = parse_phantom_payload(&bytes, VERSION, 8).unwrap();
        assert_eq!(old.cinfo[..4], 0x8001_u32.to_le_bytes());
    }

    #[test]
    fn the_shape_moves_its_material_and_reads_two_arrays() {
        let mut bytes = vec![0; CONVEX_CINFO_BYTES];
        bytes[..4].copy_from_slice(&31_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        for word in [1_u32, 2, 3, 4] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        let (shape, consumed) = parse_convex_vertices_payload(&bytes, VERSION).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(shape.material_10, 0, "31 and above read as 0");
        assert_eq!(shape.vertices, [[1, 2, 3, 4]]);
        assert!(shape.normals.is_empty());
        assert_eq!(f32::from_bits(shape.radius_bits), 0.1);
    }

    // 008A44F0: 250.0 caps +0xC4, a zero mass outside layer 2 sets the motion byte, then the list and +0x18.
    #[test]
    fn the_rigid_body_caps_and_reads_its_tail() {
        let mut bytes = 5_u32.to_le_bytes().to_vec();
        let mut cinfo = vec![0; RIGID_BODY_CINFO_BYTES];
        cinfo[..4].copy_from_slice(&1_u32.to_le_bytes());
        cinfo[0x14..0x18].copy_from_slice(&0x4000_0007_u32.to_le_bytes());
        cinfo[0xC4..0xC8].copy_from_slice(&300.0_f32.to_le_bytes());
        bytes.extend_from_slice(&cinfo);
        for word in [2_u32, 8, 9, 0x77] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        let (body, pending, consumed) = parse_rigid_body_payload(&bytes, VERSION, 11).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(pending.shape_index, 5);
        assert_eq!(pending.constraint_indices, [8, 9]);
        assert_eq!(body.field_18, 0x77);
        assert_eq!(word_at(&body.cinfo_0c, 0xC4), 250.0_f32.to_bits());
        assert_eq!(body.cinfo_0c[0xD0], 7);
        assert_eq!(word_at(&body.cinfo_0c, 0x14), 0xC000_0000);
        assert!(parse_rigid_body_payload(&bytes, VERSION, 5).is_err());
    }

    // 008C3640: the child, the cinfo with its material moved and +0x04 cleared, then 008E84B0's size, origin and code.
    #[test]
    fn the_mopp_shape_reads_its_code() {
        let mut bytes = 4_u32.to_le_bytes().to_vec();
        let mut cinfo = vec![0xAA; 16];
        cinfo[..4].copy_from_slice(&3_u32.to_le_bytes());
        bytes.extend_from_slice(&cinfo);
        bytes.extend_from_slice(&3_u32.to_le_bytes());
        for word in [1_u32, 2, 3, 4] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(&[9, 8, 7]);
        let (shape, pending, consumed) =
            parse_shape_payload(&bytes, VERSION, 11, BhkShapeKind::MoppBvTree).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(pending.indices, [4]);
        assert_eq!(shape.material_10, 3);
        assert_eq!(word_at(&shape.cinfo_0c, 0), 0);
        assert_eq!(word_at(&shape.cinfo_0c, 4), 0);
        assert_eq!(shape.mopp, Some(([1, 2, 3, 4], vec![9, 8, 7])));
    }

    // 008C0CB0: the entity list, the priority, 120 bytes, and a zero last float read as 10.0.
    #[test]
    fn the_ragdoll_reads_its_data() {
        let mut bytes = 2_u32.to_le_bytes().to_vec();
        for word in [1_u32, 2, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(&[0; 120]);
        let (constraint, entities, consumed) =
            parse_constraint_payload(&bytes, VERSION, 11, ConstraintKind::Ragdoll).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(entities, [1, 2]);
        assert_eq!(constraint.priority_08, 1);
        assert_eq!(word_at(&constraint.data, 116), 10.0_f32.to_bits());
        let hinge = [bytes[..16].to_vec(), vec![0; 124]].concat();
        let (constraint, _, consumed) =
            parse_constraint_payload(&hinge, VERSION, 11, ConstraintKind::LimitedHinge).unwrap();
        assert_eq!((consumed, constraint.data.len()), (hinge.len(), 124));
        // 008C1B00: 80 and 48 bytes and three words.
        let prismatic = [bytes[..16].to_vec(), vec![0; 140]].concat();
        let (constraint, _, consumed) =
            parse_constraint_payload(&prismatic, VERSION, 11, ConstraintKind::Prismatic).unwrap();
        assert_eq!((consumed, constraint.data.len()), (prismatic.len(), 140));
    }
}
