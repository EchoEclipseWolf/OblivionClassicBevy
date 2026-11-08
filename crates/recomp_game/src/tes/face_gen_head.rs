//! An NPC's FaceGen head inputs: the coefficients (`005221C0`, `00552880`, `00552990`, `00552530`) and the face
//! description `0052CD50` fills for `00555A80`'s part loop. Evidence: `evidence/ida-names-gameplay-2026-11-07/facegen-*`;
//! the map is in `docs/target-a-plan-2026-11-07.md` ("FaceGen (background agent) working notes").

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::ai_process::base_actor_value;
use super::facegen::{FaceGenMatrix, FaceGenSingleton};
use super::forms_actors::{TesNpc, TesRace};
use super::hud_main_menu::{base_race, npc_sex};
use super::tes_file::BsStringT;

/// IDA: 00B10CA8, the nine part names 0052CD50 stores in the description's +0x94 array, by part index.
pub const PART_NAMES_B10CA8: [&str; 9] = [
    "FaceGenFace",
    "FaceGenEars",
    "FaceGenEars",
    "FaceGenMouth",
    "FaceGenTeethLower",
    "FaceGenTeethUpper",
    "FaceGenTongue",
    "FaceGenEyeLeft",
    "FaceGenEyeRight",
];

/// IDA: 00552880 FaceGen_ShapeCoefficients(out): out[0] = 50 x 1, out[1] = 30 x 1, out[2] = 50 x 1, each resized
/// (00527160) with zeros; out[3] isn't touched (the caller's FgMatrix constructor left it 0 x 0). Completion: 100%.
pub fn shape_coefficients(out: &mut [FaceGenMatrix; 4]) {
    for (matrix, rows) in out.iter_mut().zip([50_u32, 30, 50]) {
        matrix.rows = rows;
        matrix.cols = 1;
        matrix.data.resize(rows as usize, 0.0);
    }
}

// IDA: 005528F0's per-matrix copy: the source's shape and bytes (005520E0) when it has rows and columns, else 0 x 0.
fn copy_matrix(source: &FaceGenMatrix, target: &mut FaceGenMatrix) {
    if source.rows != 0 && source.cols != 0 {
        target.rows = source.rows;
        target.cols = source.cols;
        target
            .data
            .resize((source.rows * source.cols) as usize, 0.0);
        target.data.copy_from_slice(&source.data);
    } else {
        target.rows = 0;
        target.cols = 0;
        target.data.clear();
    }
}

/// IDA: 00552530 FgMatrix_Add(this, out, other): a shape mismatch asserts (matrixVT.hpp line 359, a warning); out takes
/// this's shape (00552220) and out[i] = other[i] + this[i] (f32 adds) over this's elements. Completion: 100% but the
/// mismatch, whose read past `other` is the CRT's invalid-parameter handler: a stop.
fn add_matrices(this: &FaceGenMatrix, other: &FaceGenMatrix) -> Result<FaceGenMatrix, StreamError> {
    if this.rows != other.rows || this.cols != other.cols || other.data.len() < this.data.len() {
        return Err(StreamError(
            "00552530: FaceGen matrices of different shapes (matrixVT.hpp line 359)",
        ));
    }
    Ok(FaceGenMatrix {
        rows: this.rows,
        cols: this.cols,
        data: this
            .data
            .iter()
            .zip(&other.data)
            .map(|(&mine, &theirs)| theirs + mine)
            .collect(),
    })
}

/// IDA: 00552990 FaceGen_CombineCoefficients(race, npc, out, a4, limit), the arm with both sources, a4 0 and limit 0.0:
/// for each of the four matrices, a race matrix with rows and columns gives out its shape and npc + race (00552530); any
/// other gives out 0 x 0. With a null race or npc the other is copied (005528F0). Completion: 100% of the arm the heads
/// take; a4 (a copy of the asymmetric pair) and a positive limit (00552990's RMS clamp) are not reached.
/// # Errors
/// A shape mismatch, as `add_matrices`.
pub fn combine_coefficients(
    race: &[FaceGenMatrix; 4],
    npc: &[FaceGenMatrix; 4],
    out: &mut [FaceGenMatrix; 4],
) -> Result<(), StreamError> {
    for k in 0..4 {
        if race[k].rows != 0 && race[k].cols != 0 {
            out[k] = add_matrices(&race[k], &npc[k])?;
        } else {
            out[k] = FaceGenMatrix::default();
        }
    }
    Ok(())
}

/// IDA: 005221C0 TESNPC_GetFaceGenCoefficients(npc, out): 00552880(out); with a race (+0xE8), 00552990(race +0x29C, the
/// NPC's +0x168 when vt+0x128(69), TESNPC_GetBaseActorValue(vampirism), is nonzero, else +0x108, out, 0, 0.0); with none,
/// 005528F0 copies g_faceGen's +0x08 (005538D0). Completion: 100%.
/// # Errors
/// A base that isn't an NPC, a null g_faceGen (00553550 isn't run from here), or 00552990's.
pub fn npc_face_gen_coefficients(
    world: &World,
    npc: Entity,
) -> Result<[FaceGenMatrix; 4], StreamError> {
    let mut out: [FaceGenMatrix; 4] = Default::default();
    shape_coefficients(&mut out);
    let data = world
        .get::<TesNpc>(npc)
        .ok_or(StreamError("005221C0: a base that isn't an NPC"))?;
    match base_race(world, npc)? {
        Some(race) => {
            let race_data = world
                .get::<TesRace>(race)
                .ok_or(StreamError("005221D1: a race that isn't a TESRace"))?;
            let vampire = base_actor_value(world, npc, 69)? != 0;
            let own = if vampire {
                &data.facegen_168
            } else {
                &data.facegen_108
            };
            combine_coefficients(&race_data.facegen_29c, own, &mut out)?;
        }
        None => {
            let defaults = world
                .get_resource::<FaceGenSingleton>()
                .and_then(|singleton| singleton.0.as_ref())
                .ok_or(StreamError(
                    "005538D9: g_faceGen is null, and 00553550 isn't run from here",
                ))?
                .coefficients
                .clone();
            for (source, target) in defaults.iter().zip(out.iter_mut()) {
                copy_matrix(source, target);
            }
        }
    }
    Ok(out)
}

/// IDA: N/A a part's TESModel or TESTexture path, as 00555A80 reads it (+0x04, its length by 00449190).
#[must_use]
pub fn path_text(path: &BsStringT) -> Vec<u8> {
    let length = super::character_3d::path_length(path);
    path.data_00
        .as_deref()
        .map(|bytes| bytes[..length.min(bytes.len())].to_vec())
        .unwrap_or_default()
}

/// IDA: the 0xC4 face description (00527C90, 00526CE0) 0052CD50 fills: +0x00 the four coefficient matrices, +0x60 the
/// hair, +0x64 the hair colour, +0x68 the hair length, +0x6C the eyes, +0x70 the sex (-1 for none), +0x74 the nine part
/// models and +0x84 their textures (here their paths, from the race's TESModel +0xE0 + 0x18 i and TESTexture +0x1B8 +
/// 0x0C i), +0x94 the names (00B10CA8), +0xA4 the NPC's own face textures (only with bFaceGenTexturing: not built here),
/// +0xB4 bFaceGenTexturing (00B10D3C), +0xB8 and +0xBC the race's eye models (+0x188, +0x1A0, the parts 7 and 8), +0xC0
/// 00B120B0, which skips 00555A80's `_50.NIF` head when nonzero (no writer; image value 0xFFFFFFFF).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceDescription {
    pub coefficients_00: [FaceGenMatrix; 4],
    pub hair_60: Option<Entity>,
    pub hair_color_64: u32,
    pub hair_length_68: f32,
    pub eyes_6c: Option<Entity>,
    pub sex_70: i32,
    pub model_paths_74: [Vec<u8>; 9],
    pub texture_paths_84: [Vec<u8>; 9],
    pub face_texturing_b4: bool,
    pub skip_lod_head_c0: u32,
}

/// IDA: 0052BC50 TESRace_GetFacePartModel(i) and 0052BD00 TESRace_GetFacePartTexture(i): past 8 null; the ears (1 the
/// male's, 2 the female's) with an empty path fall back to the other's (+0x110 for 1, +0xF8 for 2; textures +0x1D0, +0x1C4);
/// any other +0xE0 + 0x18 i (+0x1B8 + 0x0C i). Completion: 100%.
fn race_part_paths(race: &TesRace, part: usize) -> (Vec<u8>, Vec<u8>) {
    let model = |index: usize| path_text(&race.face_models_e0[index].path_04);
    let texture = |index: usize| path_text(&race.face_textures_1b8[index].path_04);
    let pick = |own: Vec<u8>, other: usize, get: &dyn Fn(usize) -> Vec<u8>| {
        if own.is_empty() { get(other) } else { own }
    };
    match part {
        1 => (pick(model(1), 2, &model), pick(texture(1), 2, &texture)),
        2 => (pick(model(2), 1, &model), pick(texture(2), 1, &texture)),
        _ => (model(part), texture(part)),
    }
}

/// IDA: 0052CD50 TESRace_FillFaceDescription(race, npc, description), the NPC arm: 005221C0, the NPC's hair +0x1C8, hair
/// length +0x1CC, hair colour +0x1E8, eyes +0x1D0 and sex (00519D20); then the nine parts (0052BC50, 0052BD00, 00B10CA8);
/// with bFaceGenTexturing (0052CF73), the NPC's own face textures (00524100: a master's NPC with vampirism 0 gets
/// `data\Textures\Faces\<master>\%08X_%i.dds` through TES_CreateTextureImage(.., 1, 1) for the face and its sex's ears,
/// 00523D80's default for the rest) aren't loaded here: they feed only 005551C0's face texture, deferred with it (D13); +0xB4, +0xC0, +0xB8, +0xBC; no hair: the
/// race's first hair (+0x8C); no eyes: the race's first eyes (+0xA8), else the data handler's eye list searched for
/// "Characters\Eyes\EyeDefault.dds" (a stop here). Completion: the arms the prison's NPCs reach.
/// # Errors
/// The arms named above, or a base that isn't an NPC with a race.
pub fn fill_face_description(
    world: &World,
    npc: Entity,
    face_texturing_b10d3c: bool,
    skip_lod_head_b120b0: u32,
) -> Result<FaceDescription, StreamError> {
    let mut description = FaceDescription {
        coefficients_00: npc_face_gen_coefficients(world, npc)?,
        ..FaceDescription::default()
    };
    let data = world
        .get::<TesNpc>(npc)
        .ok_or(StreamError("0052CD50: a base that isn't an NPC"))?;
    let race = base_race(world, npc)?.ok_or(StreamError("0052CD50: an NPC with no race"))?;
    let race_data = world
        .get::<TesRace>(race)
        .ok_or(StreamError("0052CD50: a race that isn't a TESRace"))?;
    description.hair_60 = data.hair_1c8;
    description.hair_length_68 = data.hair_length_1cc;
    description.hair_color_64 = data.hair_color_1e8;
    description.eyes_6c = data.eyes_1d0;
    description.sex_70 = npc_sex(world, npc).map_or(-1, |sex| sex as i32);
    for part in 0..9 {
        let (model, texture) = race_part_paths(race_data, part);
        description.model_paths_74[part] = model;
        description.texture_paths_84[part] = texture;
    }
    description.face_texturing_b4 = face_texturing_b10d3c;
    description.skip_lod_head_c0 = skip_lod_head_b120b0;
    if description.hair_60.is_none() {
        description.hair_60 = race_data.hairs_8c.first().copied();
    }
    if description.eyes_6c.is_none() {
        match race_data.eyes_a8.first() {
            Some(&eyes) => description.eyes_6c = Some(eyes),
            None => {
                return Err(StreamError(
                    "0052D084: the data handler's EyeDefault.dds search",
                ));
            }
        }
    }
    Ok(description)
}
