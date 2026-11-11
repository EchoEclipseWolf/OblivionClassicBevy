//! BSFaceGenModel's geometry morph: `00558840` BSFaceGenModel_ApplyCoefficients over a cloned head part, with the
//! BSFaceGenBaseMorphExtraData (`0055C6E0`) that holds its unmorphed vertices.
//! Evidence: `evidence/ida-names-gameplay-2026-11-07/facegen-*`; checked against run-af's `after_move` heads
//! (`fixtures/rows/facegen_nodes.rows`).
//!
//! `00558840` runs on the IO thread for QueuedHead and QueuedHelmet (`00435300`, `0043EB80`), whose x87 control word is
//! 0x27F (53-bit mantissa): each `fild`/`fmul`/`fadd` chain rounds to f64 and `fstp` once to f32. Checked: this rounding gives
//! the nine NPCs' 45 non-ear parts bit for bit (`tests/face_gen_heads.rs`); 24-bit rounding (plain f32) doesn't.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::face_gen_files::EgmData;
use crate::tes::facegen::FaceGenMatrix;

/// IDA: BSFaceGenBaseMorphExtraData (0x18, vtable 00A64C6C, `0055C6E0`): an NiExtraData with +0x0C the vertices (the
/// geometry data's at the clone, then the TRI's added vertices), +0x10 their count (vt+0x58(0)), +0x14 the geometry data's
/// count (vt+0x58(1)). vt+0x54 returns +0x0C.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceGenBaseMorph {
    pub vertices_0c: Vec<[f32; 3]>,
    pub total_10: u32,
    pub data_count_14: u32,
}

/// IDA: 0055C6E0 BSFaceGenBaseMorphExtraData(geometry, added, added count): with the geometry's data locked for reading
/// (00728AB0(1), 00728B60), +0x14 = +0x10 = the data's vertex count (+0x08), plus the added count when there are added
/// vertices; FormHeap(12 × +0x10); the data's positions copied, then the added vertices after them; 00728B20. No data or
/// no positions: all zero. Completion: 100% of the host form (no lock or allocator).
#[must_use]
pub fn base_morph(positions: Option<&[[f32; 3]]>, added: &[[f32; 3]]) -> FaceGenBaseMorph {
    let Some(positions) = positions else {
        return FaceGenBaseMorph::default();
    };
    let count = positions.len() as u32;
    let mut vertices = positions.to_vec();
    vertices.extend_from_slice(added);
    FaceGenBaseMorph {
        total_10: vertices.len() as u32,
        vertices_0c: vertices,
        data_count_14: count,
    }
}

/// IDA: N/A what `00558840` returned and which warning it gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// It returned 1: the positions are morphed.
    Applied,
    /// It returned 0 with the named check (a warning text when it gave one).
    Rejected(&'static str),
}

// IDA: N/A one `fild` delta × the weight, `fadd` the vertex, `fstp`: 53-bit rounding then f32 on the IO thread; at the main
// thread's 24-bit precision each step rounds to a float.
fn morph_component(delta: i16, weight: f32, value: f32, precision24: bool) -> f32 {
    if precision24 {
        f32::from(delta) * weight + value
    } else {
        (f64::from(delta) * f64::from(weight) + f64::from(value)) as f32
    }
}

/// IDA: 00558840 BSFaceGenModel_ApplyCoefficients(model, coefficients, geometry, scale, offset), the arms the heads and
/// helmets reach (`offset` null, the EGM loaded): the geometry's BSFaceGenBaseMorphExtraData (`005508A0`) gives the
/// vertices (vt+0x54) and their count (vt+0x58(0)); without one the data's positions and count are used (and 00B39D84 = 1);
/// no base and no positions returns 0. Each EGM group g (symmetric, asymmetric) with more morphs than coefficient[g]'s rows
/// warns "FaceGen - Tried to apply a coordinate that did not match the EGM basis." and returns 0; any morph with fewer
/// vertices than the count warns "FaceGen - EGM basis did not match the provided model data." and returns 0. Then for
/// each group and morph i, weight = coefficient[g][i × cols] × scale × morph.scale (stored as a float); a nonzero weight
/// adds weight × delta to each vertex: with a base, the base's vertices (all `total`), the data's first `data count`
/// positions copied from them; without, the data's positions. Returns 1. The model's +0x14 spin flag and the data's +0x2E
/// bit 0 (with positions) and 00728B20 are the caller's. `precision24`: the caller's thread runs x87 at 24 bits (the main
/// thread), else 53 (the IO thread). Completion: 100% of these arms; the offset arms are a stop.
/// # Errors
/// The offset arms (`offset` nonnull), which no head or helmet passes.
#[allow(
    clippy::too_many_arguments,
    reason = "00558840's arguments and the caller's x87 precision"
)]
pub fn apply_coefficients(
    egm: &EgmData,
    coefficients: &[FaceGenMatrix],
    scale: f32,
    base: Option<&mut FaceGenBaseMorph>,
    positions: &mut [[f32; 3]],
    offset: Option<&[[f32; 3]]>,
    warnings: &mut Vec<String>,
    precision24: bool,
) -> Result<ApplyOutcome, GameError> {
    if offset.is_some() {
        return Err(GameError::unclassified(
            0x00558840,
            "00558840: the offset arms (a nonnull a5)",
        ));
    }
    let count = match &base {
        Some(base) if !base.vertices_0c.is_empty() => base.total_10,
        _ => {
            if positions.is_empty() {
                return Ok(ApplyOutcome::Rejected("00558840: no base and no positions"));
            }
            positions.len() as u32
        }
    };
    let groups = [&egm.symmetric_04, &egm.asymmetric_14];
    for (group, morphs) in groups.iter().enumerate() {
        let rows = coefficients.get(group).map_or(0, |matrix| matrix.rows);
        if !morphs.is_empty() && (rows as usize) < morphs.len() {
            const TEXT: &str =
                "FaceGen - Tried to apply a coordinate that did not match the EGM basis.";
            warnings.push(TEXT.to_owned());
            return Ok(ApplyOutcome::Rejected(TEXT));
        }
        for morph in morphs.iter() {
            if morph.deltas_04.len() < count as usize {
                const TEXT: &str = "FaceGen - EGM basis did not match the provided model data.";
                warnings.push(TEXT.to_owned());
                return Ok(ApplyOutcome::Rejected(TEXT));
            }
        }
    }
    let mut base = base.filter(|base| !base.vertices_0c.is_empty());
    for (group, morphs) in groups.iter().enumerate() {
        let matrix = &coefficients[group];
        for (index, morph) in morphs.iter().enumerate() {
            let coefficient = matrix
                .data
                .get(index * matrix.cols as usize)
                .copied()
                .ok_or(GameError::unclassified(
                    0x00558840,
                    "00558840: a coefficient past the vector (_invalid_parameter_noinfo)",
                ))?;
            // fld coefficient; fmul scale; fmul morph scale; fstp.
            let weight = if precision24 {
                coefficient * scale * morph.scale_00
            } else {
                (f64::from(coefficient) * f64::from(scale) * f64::from(morph.scale_00)) as f32
            };
            if weight == 0.0 {
                continue;
            }
            match base.as_deref_mut() {
                Some(base) => {
                    let data_count = positions.len().min(base.vertices_0c.len());
                    for (vertex, delta) in base
                        .vertices_0c
                        .iter_mut()
                        .zip(&morph.deltas_04)
                        .take(count as usize)
                    {
                        for axis in 0..3 {
                            vertex[axis] =
                                morph_component(delta[axis], weight, vertex[axis], precision24);
                        }
                    }
                    positions[..data_count].copy_from_slice(&base.vertices_0c[..data_count]);
                }
                None => {
                    for (vertex, delta) in positions.iter_mut().zip(&morph.deltas_04) {
                        for axis in 0..3 {
                            vertex[axis] =
                                morph_component(delta[axis], weight, vertex[axis], precision24);
                        }
                    }
                }
            }
        }
    }
    Ok(ApplyOutcome::Applied)
}

/// IDA: the BSFaceGenModel's morph data (+0x1C of its 0x24 data): `0055B000` BSFaceGenMorphDataHead (0x18) or `0055C3F0`
/// BSFaceGenMorphDataHair (0xC), each over the TRI it was built from (the animation's expressions and modifiers, Build 3).
#[derive(Clone, Debug, PartialEq)]
pub enum FaceGenMorphData {
    Head(super::face_gen_files::TriData),
    Hair(super::face_gen_files::TriData),
}

/// IDA: a BSFaceGenModel (0x1C, vtable 00A64A98, `005564A0`) with its 0x24 data (`00556900`) folded in: data +0x00 the EGM
/// path, +0x08 the EGM (`00558770`, read on first use), +0x0C the NIF root, +0x10 its first geometry child, +0x14/+0x18 the
/// TRI's added vertices, +0x1C the morph data, +0x20 the BSFaceGenModelExtraData (no host form); the model's +0x0C the EGT
/// path (`00556A00`), +0x14 and +0x18 the EGM and EGT spin flags (the walk is single-threaded here). `loaded` is +0x08 nonnull.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceGenModel {
    pub loaded: bool,
    pub egm_path_00: Vec<u8>,
    pub egm_08: Option<EgmData>,
    pub root_0c: Option<Entity>,
    pub geometry_10: Option<Entity>,
    pub added_vertices_14: Vec<[f32; 3]>,
    pub morph_data_1c: Option<FaceGenMorphData>,
    pub egt_path_0c: Option<Vec<u8>>,
    /// The EGT holder's +0x08 (`00557C80`, read by 00557E60 on first use; kept when the read fails).
    pub egt_08: Option<super::face_gen_files::EgtData>,
}

/// IDA: g_faceGen +0xDAC, the BSFaceGenModelMap (0x20, `00551E80`, vtable 00A64820): a BSTCaseInsensitiveStringMap of
/// refcounted entries (vtable 00A647D8: the model and its last-use tick), +0x18 uiFaceGenMaxEGMDataSize (00B120EC), +0x1C
/// uiFaceGenMaxEGTDataSize (00B120F4). Keys are lower-cased here (the map's compare is case-insensitive); nothing reads the
/// entries in map order but `0054FE70`'s purge, whose order isn't observable.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct BsFaceGenModelMap {
    pub models: Vec<FaceGenModel>,
    pub entries: std::collections::BTreeMap<Vec<u8>, usize>,
    pub max_egm_size_18: u32,
    pub max_egt_size_1c: u32,
}

impl BsFaceGenModelMap {
    // IDA: N/A the case-insensitive key.
    fn key(path: &[u8]) -> Vec<u8> {
        path.to_ascii_lowercase()
    }

    /// IDA: 005515B0 BSFaceGenModelMap_Find(path, out): under 00B39C00, the entry's model (its tick set by GetTickCount,
    /// 005506B0's trim) or none. Completion: 100% but the tick (no host clock here) and the trim (a counted boundary: the
    /// 64 MB limits are never reached by the prison's models).
    #[must_use]
    pub fn find(&self, path: &[u8]) -> Option<usize> {
        self.entries.get(&Self::key(path)).copied()
    }

    /// IDA: 00551450 BSFaceGenModelMap_Add(path, model): an old entry under the key removed, a new 0x10 entry with the model
    /// set. Completion: 100% but the tick, as `find`.
    pub fn add(&mut self, path: &[u8], model: usize) {
        self.entries.insert(Self::key(path), model);
    }
}

/// IDA: N/A what `00559330`'s NIF load and the FaceGen files' opens reach from the walk: BSStream_LoadFromPath (006F9980 through
/// the walk's stream services) and BSFile_OpenCore(path, 0, 0x2800, 1) (the archives; there are no loose meshes).
pub struct FaceGenFileServices<'a> {
    pub load_nif:
        &'a mut dyn FnMut(&mut World, &str) -> Result<crate::bs::stream::BsStreamLoad, GameError>,
    pub open: &'a mut dyn FnMut(&str) -> Option<crate::bs::file::OpenedFile>,
    /// FileFinder_FindFile(path, 0, 0, -1)'s answer (every archive type, 0xFFFF), for `00551A00`.
    pub exists: &'a mut dyn FnMut(&str) -> bool,
    pub warnings: &'a mut Vec<String>,
    pub last_error: &'a mut crate::tes::facegen::FaceGenLastError,
    /// The calling thread's x87 precision: 24 bits on the main thread (Direct3D's), 53 on the IO thread.
    pub precision24: bool,
}

/// IDA: 00557E60 BSFaceGenModel_GenerateTexture(model, coefficients, slot, 0) (thiscall model) with its EGT's load: no EGT
/// holder (+0x0C) answers None; without `00551960`'s map entry, an unloaded EGT with an empty path answers None, else
/// `00557C80` reads it (`006EFA20`; the object is kept whatever the read answers); then the sum and `0054DA40`
/// (`face_gen_texture::generate_face_gen_texture`). The slot's grey default (`00523D80`) is the caller's. Completion: 100%
/// of the texture; the spin flag (+0x18) and the map lock have no host form.
pub fn model_generate_face_texture(
    model: &mut FaceGenModel,
    coefficients: &[FaceGenMatrix; 4],
    services: &mut FaceGenFileServices<'_>,
) -> Option<super::face_gen_texture::FaceGenTextureImage> {
    let path = model.egt_path_0c.clone()?;
    if model.egt_08.is_none() {
        if path.is_empty() {
            return None;
        }
        let mut egt = super::face_gen_files::EgtData::default();
        let text = String::from_utf8_lossy(&path).into_owned();
        let mut opened = (services.open)(&text);
        let _ = super::face_gen_files::read_egt(
            &mut egt,
            opened.as_mut(),
            &path,
            services.warnings,
            services.last_error,
        );
        model.egt_08 = Some(egt);
    }
    let egt = model.egt_08.as_ref().expect("read above");
    super::face_gen_texture::generate_face_gen_texture(egt, coefficients, services.warnings)
}

/// IDA: N/A a geometry's data positions (+0xB4's +0x1C), for a NiTriShape or NiTriStrips.
#[must_use]
pub fn geometry_positions(world: &World, geometry: Entity) -> Option<Vec<[f32; 3]>> {
    use crate::gamebryo::mesh_geometry::{MeshTriShapeData, NiGeometry};
    use crate::gamebryo::mesh_tri_strips::MeshTriStripsData;
    let data = world.get::<NiGeometry>(geometry)?.data_b4?;
    if let Some(shape) = world.get::<MeshTriShapeData>(data) {
        return Some(shape.base.positions_1c.clone());
    }
    world
        .get::<MeshTriStripsData>(data)
        .map(|strips| strips.base.positions_1c.clone())
}

/// IDA: 00559330 BSFaceGenModel_LoadModelMesh(model, egm, nif, tri, hair): only for a model with no data (+0x08) and an EGM
/// path. The data, its BSFaceGenModelExtraData and the EGM path; the NIF through BSStream_LoadFromPath: a failure warns
/// "Failed to load '%s' in BSFaceGenModel::LoadModelMesh." and returns 0 (the data stays); a count other than one warns
/// "Bad object count in '%s' in BSFaceGenModel::LoadModelMesh." and returns 0. The root (top object 0) and its first child
/// whose vt+0x10 is nonnull (a NiTriShape or NiTriStrips) into +0x0C and +0x10. With a TRI path, `006F4B50` into a 0xA0 TRI;
/// a read that succeeds with `00556800`'s base count equal to the geometry data's vertex count makes the morph data (hair or
/// head); a head with more TRI vertices than the data's keeps the extras (`00556800`'s count on) at +0x14/+0x18. The data's
/// +0x2E takes 0x8000 with morph data, else keeps its low 12 bits. Returns 1. Completion: the data, the files and the flags;
/// the mesh's own surgery (its texturing property (6) detached, 006FFFD0's controller and 006FFC60's extra data released, a
/// skin instance's +0x10 = the geometry, NiAVObject +0x18 |= 2) is deferred with the head nodes (D12), since only the clones
/// would show it.
/// # Errors
/// A stream loader's error.
pub fn load_model_mesh(
    world: &mut World,
    model: &mut FaceGenModel,
    egm: &[u8],
    nif: Option<&str>,
    tri: Option<&str>,
    hair: bool,
    services: &mut FaceGenFileServices<'_>,
) -> Result<bool, GameError> {
    if model.loaded || egm.is_empty() {
        return Ok(true);
    }
    model.loaded = true;
    model.egm_path_00 = egm.to_vec();
    if let Some(nif) = nif {
        let load = (services.load_nif)(world, nif)?;
        if load.status != crate::bs::stream::BsStreamLoadStatus::Loaded {
            services.warnings.push(format!(
                "Failed to load '{nif}' in BSFaceGenModel::LoadModelMesh."
            ));
            return Ok(false);
        }
        if load.root_count() != 1 {
            services.warnings.push(format!(
                "Bad object count in '{nif}' in BSFaceGenModel::LoadModelMesh."
            ));
            return Ok(false);
        }
        model.root_0c = load.root(0);
        if let Some(root) = model.root_0c {
            let children = world
                .get::<crate::gamebryo::scene_node::NiNodeChildren>(root)
                .map(|children| children.slots_ac.clone())
                .unwrap_or_default();
            model.geometry_10 = children.into_iter().flatten().find(|&child| {
                world
                    .get::<crate::gamebryo::stream_objects::NiClass>(child)
                    .is_some_and(|class| {
                        matches!(
                            class,
                            crate::gamebryo::stream_objects::NiClass::TriShape
                                | crate::gamebryo::stream_objects::NiClass::TriStrips
                        )
                    })
            });
        }
        // 005595AB..00559607: the geometry's texturing property (type 6) detached, its controller released, its extra data
        // removed, a skin instance's +0x10 root parent set to the geometry itself (so each clone gets its own instance in
        // 0072BFA0), and +0x18 |= 2.
        if let Some(geometry) = model.geometry_10 {
            let properties = world
                .get::<crate::gamebryo::av_object::NiAvProperties>(geometry)
                .map(|list| list.0.clone())
                .unwrap_or_default();
            for (index, property) in properties.iter().enumerate() {
                if crate::gamebryo::scene_state::property_type(world, *property)? == 6 {
                    world
                        .get_mut::<crate::gamebryo::av_object::NiAvProperties>(geometry)
                        .expect("read above")
                        .0
                        .remove(index);
                    break;
                }
            }
            if let Some(mut net) =
                world.get_mut::<crate::gamebryo::object_net::NiObjectNet>(geometry)
            {
                net.controller_0c = None;
                net.extra_data.clear();
            }
            if let Some(skin) = world
                .get::<crate::gamebryo::mesh_geometry::NiGeometry>(geometry)
                .and_then(|links| links.skin_b8)
                && let Some(mut instance) =
                    world.get_mut::<crate::gamebryo::mesh_skin_owners::MeshSkinInstance>(skin)
            {
                instance.root_parent_owner_10 = Some(geometry);
            }
            if let Some(mut payload) =
                world.get_mut::<crate::gamebryo::mesh_geometry::MeshTriShapePayload>(geometry)
            {
                payload.effective_av_flags |= 2;
            }
        }
    }
    if let Some(tri) = tri {
        let mut data = super::face_gen_files::TriData::default();
        let mut opened = (services.open)(tri);
        let read = super::face_gen_files::read_tri(
            &mut data,
            opened.as_mut(),
            tri.as_bytes(),
            services.warnings,
            services.last_error,
        );
        let count = model
            .geometry_10
            .and_then(|geometry| geometry_positions(world, geometry))
            .map(|positions| positions.len() as u32);
        if read && count == Some(data.base_vertex_count()) {
            let count = count.expect("tested");
            if hair {
                model.morph_data_1c = Some(FaceGenMorphData::Hair(data));
            } else {
                let base = data.base_vertex_count() as usize;
                if data.vertices_00.len() as u32 > count {
                    let extra = data.vertices_00.len() - count as usize;
                    model.added_vertices_14 = data.vertices_00[base..base + extra].to_vec();
                }
                model.morph_data_1c = Some(FaceGenMorphData::Head(data));
            }
        }
    }
    if let Some(geometry) = model.geometry_10 {
        let morph = model.morph_data_1c.is_some();
        // The keep flags (+0x2E) of the geometry's data; a geometry with no host data stores nothing.
        crate::gamebryo::mesh_geometry::update_geometry_data_flags_2e(world, geometry, |flags| {
            if morph {
                flags & 0xFFF | 0x8000
            } else {
                flags & 0xFFF
            }
        });
    }
    Ok(true)
}

/// IDA: N/A 00559B50's four paths: the EGM (the map's key), the NIF, the TRI and the EGT, each possibly null.
#[derive(Clone, Copy, Debug)]
pub struct FaceGenModelFiles<'a> {
    pub egm: &'a [u8],
    pub nif: Option<&'a str>,
    pub tri: Option<&'a str>,
    pub egt: Option<&'a [u8]>,
}

/// IDA: 00559B50 BSFaceGenModel_Load(model, egm, nif, tri, egt, hair): 0 without bUseFaceGenHeads (00B120B4); else
/// `00559330` | `00556A00` (the EGT path stored when the model has none and one is given; true). Completion: 100%.
/// # Errors
/// As `load_model_mesh`.
pub fn load_face_gen_model(
    world: &mut World,
    model: &mut FaceGenModel,
    files: FaceGenModelFiles<'_>,
    hair: bool,
    use_face_gen_heads_b120b4: bool,
    services: &mut FaceGenFileServices<'_>,
) -> Result<bool, GameError> {
    if !use_face_gen_heads_b120b4 {
        return Ok(false);
    }
    let FaceGenModelFiles { egm, nif, tri, egt } = files;
    let loaded = load_model_mesh(world, model, egm, nif, tri, hair, services)?;
    if model.egt_path_0c.is_none()
        && let Some(egt) = egt
    {
        model.egt_path_0c = Some(egt.to_vec());
    }
    Ok(loaded | true)
}

/// IDA: N/A `create_morphed_positions`'s answer: the morphed positions and the clone's base morph.
pub type MorphedPositions = (Vec<[f32; 3]>, FaceGenBaseMorph);

/// IDA: 00559870 BSFaceGenModel_CreateMorphedGeometry(model, coefficients, out), the positions it gives: `00556A70` (no data
/// or no geometry: 0) clones the geometry and its data and adds a BSFaceGenBaseMorphExtraData (`0055C6E0`: the data's
/// positions and the TRI's extras), then `00558840`(model, coefficients, clone, 1.0, null); a 0 releases the clone. The EGM is
/// read on first use (`00558770` → `006F0A00`; the 0x24 object is kept even when the read fails). Answers the positions and
/// the clone's base morph after the morph. Completion: the positions;
/// the clone itself (its geometry, data, skin data and extra data) is deferred with the head nodes (D12).
/// # Errors
/// As `apply_coefficients`.
pub fn create_morphed_positions(
    world: &World,
    model: &mut FaceGenModel,
    coefficients: &[FaceGenMatrix],
    services: &mut FaceGenFileServices<'_>,
) -> Result<Option<MorphedPositions>, GameError> {
    let Some(geometry) = model.geometry_10 else {
        return Ok(None);
    };
    let Some(positions) = geometry_positions(world, geometry) else {
        return Ok(None);
    };
    if model.egm_08.is_none() {
        if model.egm_path_00.is_empty() {
            return Ok(None);
        }
        let mut egm = EgmData::default();
        let path = String::from_utf8_lossy(&model.egm_path_00).into_owned();
        let mut opened = (services.open)(&path);
        let _ = super::face_gen_files::read_egm(
            &mut egm,
            opened.as_mut(),
            &model.egm_path_00,
            services.warnings,
            services.last_error,
        );
        model.egm_08 = Some(egm);
    }
    let egm = model.egm_08.as_ref().expect("read above");
    let mut base = base_morph(Some(&positions), &model.added_vertices_14);
    let mut morphed = positions;
    match apply_coefficients(
        egm,
        coefficients,
        1.0,
        Some(&mut base),
        &mut morphed,
        None,
        services.warnings,
        services.precision24,
    )? {
        ApplyOutcome::Applied => Ok(Some((morphed, base))),
        ApplyOutcome::Rejected(_) => Ok(None),
    }
}

// IDA: 00558840 BSFaceGenModel_ApplyCoefficients(model, coefficients, geometry, 1.0, null) on a helmet geometry: its EGM read
// on first use (00558770), no BSFaceGenBaseMorphExtraData (the stream loader has no such class, so a helmet NIF has none), the
// data's positions morphed in place. Answers whether it returned 1.
pub(crate) fn apply_helmet_coefficients(
    world: &mut World,
    model: usize,
    coefficients: &[crate::tes::facegen::FaceGenMatrix],
    data: Entity,
    files: &mut FaceGenFileServices<'_>,
) -> Result<bool, GameError> {
    let mut held = world.resource::<BsFaceGenModelMap>().models[model].clone();
    if held.egm_08.is_none() {
        if held.egm_path_00.is_empty() {
            return Ok(false);
        }
        let mut egm = crate::bs::face_gen_files::EgmData::default();
        let path = String::from_utf8_lossy(&held.egm_path_00).into_owned();
        let mut opened = (files.open)(&path);
        let _ = crate::bs::face_gen_files::read_egm(
            &mut egm,
            opened.as_mut(),
            &held.egm_path_00,
            files.warnings,
            files.last_error,
        );
        held.egm_08 = Some(egm);
    }
    let egm = held.egm_08.clone().expect("read above");
    world.resource_mut::<BsFaceGenModelMap>().models[model] = held;
    let mut positions = if let Some(strips) =
        world.get::<crate::gamebryo::mesh_tri_strips::MeshTriStripsData>(data)
    {
        strips.base.positions_1c.clone()
    } else if let Some(shape) = world.get::<crate::gamebryo::mesh_geometry::MeshTriShapeData>(data)
    {
        shape.base.positions_1c.clone()
    } else {
        return Err(GameError::unclassified(
            0x00558840,
            "00558840: helmet data that is neither strips nor a shape",
        ));
    };
    let outcome = apply_coefficients(
        &egm,
        coefficients,
        1.0,
        None,
        &mut positions,
        None,
        files.warnings,
        false,
    )?;
    if outcome != ApplyOutcome::Applied {
        return Ok(false);
    }
    if let Some(mut strips) =
        world.get_mut::<crate::gamebryo::mesh_tri_strips::MeshTriStripsData>(data)
    {
        strips.base.positions_1c = positions;
    } else if let Some(mut shape) =
        world.get_mut::<crate::gamebryo::mesh_geometry::MeshTriShapeData>(data)
    {
        shape.base.positions_1c = positions;
    }
    Ok(true)
}

/// IDA: 0054FE70 BSFaceGenModelMap_UnloadAllEGMAndEGTData with g_faceGen's +0xDAC map: under 00B39C00 and the map's lock,
/// each entry's model through 00559BA0 (its data's +0x08 EGM freed and nulled, 005599B0 first) and 00559C40 (its +0x0C's +0x08
/// EGT likewise). The walk's order isn't observable. The port holds no EGT data (D13), so only the EGMs go. Completion: 100%
/// of the state the port holds.
pub(crate) fn unload_face_gen_data(world: &mut World) {
    if let Some(mut map) = world.get_resource_mut::<BsFaceGenModelMap>() {
        for model in &mut map.models {
            model.egm_08 = None;
            model.loaded = false;
        }
    }
}
