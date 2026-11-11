//! An NPC's FaceGen head inputs: the coefficients (`005221C0`, `00552880`, `00552990`, `00552530`) and the face
//! description `0052CD50` fills for `00555A80`'s part loop. Evidence: `evidence/ida-names-gameplay-2026-11-07/facegen-*`;
//! the map is in `docs/target-a-plan-2026-11-07.md` ("FaceGen (background agent) working notes").

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use crate::bs::string::BsStringT;
use crate::tes::actor_values::base_actor_value;
use crate::tes::facegen::{FaceGenMatrix, FaceGenSingleton, resize_matrix_data};
use crate::tes::forms_actors::{TesNpc, TesRace};
use crate::tes::forms_actors::{base_race, npc_sex};
use crate::tes::tile::InterfaceCx;

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

/// IDA: 00552530 FgMatrix_Add(this, out, other): a shape mismatch asserts (matrixVT.hpp line 359, a warning); out takes
/// this's shape (00552220) and out[i] = other[i] + this[i] (f32 adds) over this's elements. Completion: 100% but the
/// mismatch, whose read past `other` is the CRT's invalid-parameter handler: a stop.
fn add_matrices(this: &FaceGenMatrix, other: &FaceGenMatrix) -> Result<FaceGenMatrix, GameError> {
    if this.rows != other.rows || this.cols != other.cols || other.data.len() < this.data.len() {
        return Err(GameError::unclassified(
            0x00552530,
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
) -> Result<(), GameError> {
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
) -> Result<[FaceGenMatrix; 4], GameError> {
    let mut out: [FaceGenMatrix; 4] = Default::default();
    shape_coefficients(&mut out);
    let data = world.get::<TesNpc>(npc).ok_or(GameError::unclassified(
        0x005221C0,
        "005221C0: a base that isn't an NPC",
    ))?;
    match base_race(world, npc)? {
        Some(race) => {
            let race_data = world.get::<TesRace>(race).ok_or(GameError::unclassified(
                0x005221D1,
                "005221D1: a race that isn't a TESRace",
            ))?;
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
                .ok_or(GameError::unclassified(
                    0x005538D9,
                    "005538D9: g_faceGen is null, and 00553550 isn't run from here",
                ))?
                .coefficients
                .clone();
            copy_facegen(&defaults, &mut out);
        }
    }
    Ok(out)
}

/// IDA: N/A a part's TESModel or TESTexture path, as 00555A80 reads it (+0x04, its length by 00449190).
#[must_use]
pub fn path_text(path: &BsStringT) -> Vec<u8> {
    let length = crate::tes::form_components::path_length(path);
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
) -> Result<FaceDescription, GameError> {
    let mut description = FaceDescription {
        coefficients_00: npc_face_gen_coefficients(world, npc)?,
        ..FaceDescription::default()
    };
    let data = world.get::<TesNpc>(npc).ok_or(GameError::unclassified(
        0x0052CD50,
        "0052CD50: a base that isn't an NPC",
    ))?;
    let race = base_race(world, npc)?.ok_or(GameError::unclassified(
        0x0052CD50,
        "0052CD50: an NPC with no race",
    ))?;
    let race_data = world.get::<TesRace>(race).ok_or(GameError::unclassified(
        0x0052CD50,
        "0052CD50: a race that isn't a TESRace",
    ))?;
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
                return Err(GameError::unclassified(
                    0x0052D084,
                    "0052D084: the data handler's EyeDefault.dds search",
                ));
            }
        }
    }
    Ok(description)
}

// IDA: N/A g_faceGen's FanControls block (+0xC8), which 00553B30 and 00555A00 reach after 00553550 (a null g_faceGen is a
// stop here: the head build made it).
/// IDA: 005551C0's age (00555293..005552CE): 006EDD30 FanControls_GetControlValue(g_faceGen +0xC8, 0, 0, 0, coefficients)
/// stored to a float, `_floor` of it as a double, `__ftol2_sse`, its low byte. Completion: 100%.
/// # Errors
/// As `fan_control_value`, or a null g_faceGen.
pub fn face_gen_age_byte(
    world: &World,
    coefficients: &[FaceGenMatrix; 4],
) -> Result<u8, GameError> {
    let controls = fan_controls(world)?;
    let mut warnings = Vec::new();
    let value = fan_control_value(controls, 0, 0, 0, coefficients, &mut warnings)?;
    Ok(oblivion_core::x87::ftol(f64::from(value).floor() as f32) as u8)
}

fn fan_controls(world: &World) -> Result<&crate::tes::facegen::FaceGenFanControls, GameError> {
    world
        .get_resource::<FaceGenSingleton>()
        .and_then(|singleton| singleton.0.as_deref())
        .map(|state| &state.fan_controls)
        .ok_or(GameError::unclassified(
            0x00553B37,
            "00553B37: g_faceGen is null (00553550)",
        ))
}

/// IDA: 006EDD30 FanControls_GetControlValue(group, control, basis, coefficients) (`retn 0x10`): group >= 5, control >= 2
/// and basis >= 2 assert (FanControls.cpp 187..189); an unloaded block (+0x00) answers 0.0; else the control's row
/// (+0x25C + 128 group + 64 control + 32 basis) times coefficients[2 basis] (005523C0), its first element, added to the
/// control's +0x1C value (`fld`, `fadd`, a float). Completion: 100%, f32 for the main thread's 24-bit x87.
/// # Errors
/// An index past the block (the original asserts and reads past it), or an empty product (`_invalid_parameter_noinfo`).
pub fn fan_control_value(
    controls: &crate::tes::facegen::FaceGenFanControls,
    group: usize,
    control: usize,
    basis: usize,
    coefficients: &[FaceGenMatrix; 4],
    warnings: &mut Vec<String>,
) -> Result<f32, GameError> {
    if !controls.loaded {
        return Ok(0.0);
    }
    let element = controls
        .group_controls
        .get(group)
        .and_then(|group| group.get(control))
        .and_then(|control| control.get(basis))
        .ok_or(GameError::unclassified(
            0x006EDD5A,
            "006EDD5A: a FanControls index past the block",
        ))?;
    let product =
        crate::tes::facegen::multiplied(&element.coefficients, &coefficients[2 * basis], warnings);
    let first = *product.data.first().ok_or(GameError::unclassified(
        0x006EDDD4,
        "006EDDD4: an empty FanControls product",
    ))?;
    Ok(element.value + first)
}

/// IDA: 00553B30 FaceGen_GetAgeGenderControl(coefficients, control, basis): 00553550, then 006EDD30(group 0). Completion:
/// 100%.
/// # Errors
/// As `fan_control_value`, or a null g_faceGen.
pub fn face_gen_age_gender_control(
    world: &World,
    coefficients: &[FaceGenMatrix; 4],
    control: usize,
    basis: usize,
) -> Result<f32, GameError> {
    let mut warnings = Vec::new();
    fan_control_value(
        fan_controls(world)?,
        0,
        control,
        basis,
        coefficients,
        &mut warnings,
    )
}

/// IDA: 006EDE10 FanControls_SetControlValues(group, basis, values, coefficients) (`retn 0x10`): group >= 5 asserts
/// (FanControls.cpp 217); an unloaded block returns. Each of the two values clamped to [15.0, 65.0] and [-4.0, 4.0]
/// (`low > v` takes low, then `high < v` takes high), less its current 006EDD30 value (floats); the weights are the
/// group's 2x2 inverse Gram matrix (+0xC34 + 16 (basis + 2 group)) times them: w0 = m0 d0 + m1 d1, w1 = d0 m2 + d1 m3;
/// then coefficients[2 basis] += transpose(control k's row) x w_k (00552730, 00552310, 00551DA0) for k 0 and 1.
/// Completion: 100%, f32 per step.
/// # Errors
/// As `fan_control_value`, or a row whose shape isn't the coefficients' (00551DA0's assert and read past the end).
pub fn fan_set_control_values(
    controls: &crate::tes::facegen::FaceGenFanControls,
    group: usize,
    basis: usize,
    values: [f32; 2],
    coefficients: &mut [FaceGenMatrix; 4],
    warnings: &mut Vec<String>,
) -> Result<(), GameError> {
    if group >= 5 {
        return Err(GameError::unclassified(
            0x006EDE4C,
            "006EDE4C: FanControls.cpp 217, a group past 5",
        ));
    }
    if !controls.loaded {
        return Ok(());
    }
    let low = [15.0_f32, -4.0];
    let high = [65.0_f32, 4.0];
    let mut differences = [0.0_f32; 2];
    for k in 0..2 {
        let mut value = values[k];
        if low[k] > value {
            value = low[k];
        }
        if high[k] < value {
            value = high[k];
        }
        let current = fan_control_value(controls, group, k, basis, coefficients, warnings)?;
        differences[k] = value - current;
    }
    let m = controls.inverse_gram[group][basis];
    let weights = [
        m[0] * differences[0] + m[1] * differences[1],
        differences[0] * m[2] + differences[1] * m[3],
    ];
    for (k, weight) in weights.into_iter().enumerate() {
        let row = &controls.group_controls[group][k][basis].coefficients;
        let step = crate::tes::facegen::scaled(&crate::tes::facegen::transposed(row), weight);
        let target = &mut coefficients[2 * basis];
        if target.rows != step.rows
            || target.cols != step.cols
            || target.data.len() < step.data.len()
        {
            return Err(GameError::unclassified(
                0x00551DA0,
                "00551DA0: FaceGen matrices of different shapes (matrixVT.hpp line 585)",
            ));
        }
        // 00551DA0's operand order: the added term first (commutative in f32, kept as written).
        for (value, add) in target.data.iter_mut().zip(&step.data) {
            *value += *add;
        }
    }
    Ok(())
}

/// IDA: 00555A00 FaceGen_SetAgeGenderControl(coefficients, control, basis, value): the two group-0 values for `basis`,
/// `value` at `control` and 006EDD30's current one at the other, then 006EDE10(0, basis, values, coefficients). Completion:
/// 100%.
/// # Errors
/// As `fan_set_control_values`, or a null g_faceGen.
pub fn face_gen_set_age_gender_control(
    world: &World,
    coefficients: &mut [FaceGenMatrix; 4],
    control: usize,
    basis: usize,
    value: f32,
) -> Result<(), GameError> {
    let controls = fan_controls(world)?;
    let mut warnings = Vec::new();
    let mut values = [0.0_f32; 2];
    for (index, slot) in values.iter_mut().enumerate() {
        *slot = if index == control {
            value
        } else {
            fan_control_value(controls, 0, index, basis, coefficients, &mut warnings)?
        };
    }
    fan_set_control_values(controls, 0, basis, values, coefficients, &mut warnings)
}

/// IDA: 00552C10 FaceGen_CoefficientsDifference(from, to, out): with all three, each of the four matrices: a `from` with
/// rows and columns gives out its shape (zeros, 00527160) and then `to` - `from` (00552630, assigned through 005520E0);
/// any other gives out 0 x 0. Completion: 100%.
pub fn coefficients_difference(
    from: &[FaceGenMatrix; 4],
    to: &[FaceGenMatrix; 4],
    out: &mut [FaceGenMatrix; 4],
) {
    let mut warnings = Vec::new();
    for k in 0..4 {
        if from[k].rows != 0 && from[k].cols != 0 {
            out[k] = crate::tes::facegen::subtracted(&to[k], &from[k], &mut warnings);
        } else {
            out[k] = FaceGenMatrix::default();
        }
    }
}

// IDA: N/A g_faceGen's linear controls (+0x88), which 005538F0 and 005539E0 reach after 00553550 (a null g_faceGen is a stop:
// 00553550 isn't run from here).
fn linear_controls(
    world: &World,
) -> Result<&[Vec<crate::tes::facegen::FaceGenLinearControl>; 4], GameError> {
    world
        .get_resource::<FaceGenSingleton>()
        .and_then(|singleton| singleton.0.as_ref())
        .map(|state| &state.linear_controls)
        .ok_or(GameError::unclassified(
            0x0055390E,
            "0055390E: g_faceGen is null (00553550)",
        ))
}

/// IDA: 005538F0 FaceGen_GetLinearControl(coefficients, a2, a3, index) (cdecl): kind = a3 + 2 x a2; with a control list of
/// that kind and `index` below its length, the control's row (`0054F6C0`) times the kind's coefficient vector
/// (`005523C0`, f32 sums), its first element; otherwise 0.0. Completion: 100%.
/// # Errors
/// A null g_faceGen, a kind past the four lists, or an empty product (`_invalid_parameter_noinfo`).
pub fn linear_control_value(
    world: &World,
    coefficients: &[FaceGenMatrix; 4],
    kind: usize,
    index: u32,
) -> Result<f32, GameError> {
    let lists = linear_controls(world)?;
    let list = lists.get(kind).ok_or(GameError::unclassified(
        0x0055397D,
        "0055397D: a linear control kind past the four",
    ))?;
    let Some(control) = list.get(index as usize) else {
        return Ok(0.0);
    };
    let vector = coefficients.get(kind).ok_or(GameError::unclassified(
        0x0055398B,
        "0055398B: a coefficient vector past the four",
    ))?;
    let mut warnings = Vec::new();
    let product = crate::tes::facegen::multiplied(&control.coefficients, vector, &mut warnings);
    product.data.first().copied().ok_or(GameError::unclassified(
        0x005539A3,
        "005539A3: an empty linear control product",
    ))
}

/// IDA: 005539E0 FaceGen_SetLinearControl(coefficients, a2, a3, index, value) (cdecl): kind = a3 + 2 x a2; with `index`
/// below that kind's list: d = value - `005538F0` (stored as a float), the control's row transposed (`00552730`) and scaled
/// by d (`00552310`, f32 products), added into the kind's vector in place (`00551DA0`, element by element, to the shorter
/// end). Completion: 100%.
/// # Errors
/// As `linear_control_value`.
#[allow(clippy::assign_op_pattern)] // 00551DA0's operand order: the scaled term first.
pub fn set_linear_control(
    world: &World,
    coefficients: &mut [FaceGenMatrix; 4],
    kind: usize,
    index: u32,
    value: f32,
) -> Result<(), GameError> {
    let lists = linear_controls(world)?;
    let Some(control) = lists
        .get(kind)
        .ok_or(GameError::unclassified(
            0x00553AAA,
            "00553AAA: a linear control kind past the four",
        ))?
        .get(index as usize)
    else {
        return Ok(());
    };
    let current = linear_control_value(world, coefficients, kind, index)?;
    let delta = value - current;
    let vector = coefficients.get_mut(kind).ok_or(GameError::unclassified(
        0x00553AE4,
        "00553AE4: a coefficient vector past the four",
    ))?;
    for (target, &row) in vector.data.iter_mut().zip(&control.coefficients.data) {
        let scaled = row * delta;
        *target = scaled + *target;
    }
    Ok(())
}

/// IDA: 00B3F058 FaceGen's random first-use byte: 006F6A10 seeds the CRT from `_time64` while it's 0.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct FaceGenRandomGlobals {
    pub seeded_b3f058: u8,
}

/// IDA: 006F6A10 FaceGen_RandomUnit: while 00B3F058 is 0, `srand(_time64(0))` and the byte = 1; then four `rand()`s,
/// v = rand + v x 32768.0 (dbl 00A47A50), and v x 8.673617379884035e-19 (dbl 00A7CDD8, 2^-60). The main thread runs the
/// x87 at 24-bit precision, so each sum rounds to a float (the products by powers of two are exact). Completion: 100%.
pub fn face_gen_random_unit(
    globals: &mut FaceGenRandomGlobals,
    crt: &mut dyn crate::tes::actor_process::RandomSource,
) -> f32 {
    if globals.seeded_b3f058 == 0 {
        let time = crt.time64();
        crt.srand(time as u32);
        globals.seeded_b3f058 = 1;
    }
    let mut value = 0.0_f32;
    for _ in 0..4 {
        value = crt.rand() as f32 + value * 32768.0;
    }
    value * f32::from_bits(0x2180_0000)
}

/// IDA: 006F6A80 FaceGen_RandomGaussian: the polar Box-Muller. u = 2 x U - 1.0 and w = 2 x U - 1.0 from two 006F6A10s,
/// s = w x w + u x u, again while s > 1.0 or s == 0.0; then `__CIlog(s)` x -2.0 (dbl 00A3F400) / s, `__CIsqrt` and x w.
/// At 24-bit precision each step rounds to a float: `__CIlog` takes its SSE2 path (`__CIlog_pentium4`, the exception
/// masks being the default) and returns a double, which the multiply rounds; `__CIsqrt`'s `__load_CW` keeps the precision
/// bits, so its `fsqrt` is a float's. Completion: 100%; the SSE2 log is the host's `f64::ln`, which could differ in the
/// last place of a double (seen only where the float's rounding sits on a half).
pub fn face_gen_random_gaussian(
    globals: &mut FaceGenRandomGlobals,
    crt: &mut dyn crate::tes::actor_process::RandomSource,
) -> f32 {
    loop {
        let first = face_gen_random_unit(globals, crt);
        let u = first + first - 1.0;
        let second = face_gen_random_unit(globals, crt);
        let w = second + second - 1.0;
        let s = w * w + u * u;
        if s > 1.0 || s == 0.0 {
            continue;
        }
        let log = (f64::from(s).ln() * -2.0) as f32;
        return (log / s).sqrt() * w;
    }
}

/// IDA: 00553B70 FaceGen_RandomizeCoefficients(scale, from, out) (cdecl): with both, g_faceGen (00553550 when null, a
/// stop here), then 006EE010(scale, from, out): for each of the four matrices, out takes from's rows and columns, is
/// resized with zeros, and each element, row by row, is from + 006F6A80 x scale x 1.5 (dbl 00A563D0), in floats.
/// Completion: 100%.
/// # Errors
/// A null g_faceGen, or no host CRT.
pub fn face_gen_randomize_coefficients(
    world: &mut World,
    scale: f32,
    from: &[FaceGenMatrix; 4],
    out: &mut [FaceGenMatrix; 4],
) -> Result<(), GameError> {
    if world
        .get_resource::<FaceGenSingleton>()
        .and_then(|singleton| singleton.0.as_ref())
        .is_none()
    {
        return Err(GameError::unclassified(
            0x00553B83,
            "00553B83: g_faceGen is null, and 00553550 isn't run from here",
        ));
    }
    let crt = world
        .get_non_send::<crate::tes::actor_process::MainThreadRandom>()
        .map(|random| random.0.clone())
        .ok_or(GameError::unclassified(
            0x006F6A10,
            "006F6A10: no host CRT for _rand",
        ))?;
    let mut crt = crt.borrow_mut();
    let mut globals = *world.get_resource_or_init::<FaceGenRandomGlobals>();
    for (source, target) in from.iter().zip(out.iter_mut()) {
        target.rows = source.rows;
        target.cols = source.cols;
        target.data.clear();
        target
            .data
            .resize(source.rows.wrapping_mul(source.cols) as usize, 0.0);
        for row in 0..source.rows {
            for col in 0..source.cols {
                let gaussian = face_gen_random_gaussian(&mut globals, &mut *crt);
                let index = (col + row * source.cols) as usize;
                target.data[index] = source.data[index] + gaussian * scale * 1.5;
            }
        }
    }
    *world.resource_mut::<FaceGenRandomGlobals>() = globals;
    Ok(())
}

/// IDA: 00551990 FaceGen_CoefficientsDiffer(a, b) (cdecl): both null answers 1 and one null 0; with both, 1 when any of the
/// four pairs isn't 00550330-equal (the same rows and columns and, both non-zero, the same bytes), else 0. Completion: 100%
/// for two present sets, the only form the port passes.
#[must_use]
pub fn coefficients_differ(a: &[FaceGenMatrix; 4], b: &[FaceGenMatrix; 4]) -> bool {
    a.iter().zip(b.iter()).any(|(a, b)| {
        a.rows != b.rows
            || a.cols != b.cols
            || (a.rows != 0
                && a.cols != 0
                && a.data
                    .iter()
                    .zip(&b.data)
                    .any(|(x, y)| x.to_bits() != y.to_bits()))
    })
}

// IDA: 005528F0(source, target): each of the four matrices takes the source's shape, and 005520E0 its bytes (memcpy), when the source has both rows and columns; else 0 x 0. Completion: 100%.
pub(crate) fn copy_facegen(source: &[FaceGenMatrix; 4], target: &mut [FaceGenMatrix; 4]) {
    for (from, to) in source.iter().zip(target.iter_mut()) {
        if from.rows != 0 && from.cols != 0 {
            to.rows = from.rows;
            to.cols = from.cols;
            resize_matrix_data(to, from.rows.wrapping_mul(from.cols));
            to.data.copy_from_slice(&from.data);
        } else {
            to.rows = 0;
            to.cols = 0;
            resize_matrix_data(to, 0);
        }
    }
}

// IDA: 005538D0: g_faceGen + 8. A null g_faceGen runs 00553550, which needs the file services and is Phase 04's: a host stop here.
pub(crate) fn facegen_coefficients(world: &World) -> Result<[FaceGenMatrix; 4], GameError> {
    Ok(world
        .get_resource::<FaceGenSingleton>()
        .and_then(|singleton| singleton.0.as_ref())
        .ok_or(GameError::unclassified(
            0x005538D9,
            "005538D9: g_faceGen is null, and 00553550 isn't run from here",
        ))?
        .coefficients
        .clone())
}

/// IDA: 00550790 FaceGen_FindModelExtraData(part) and 00558840 BSFaceGenModel_ApplyCoefficients(model, difference, part,
/// 1.0, null) on the main thread: a part with no BSFaceGenModelExtraData (`FaceGenPartMorph`) answers false. The model's
/// EGM, unloaded by 0054FE70 after the loading, is read again first (00551930 → 005516B0 → 00559190:
/// FaceGenEgm_ConstructFromFile), then the morph over the part's base morph at 24-bit precision writes its positions.
/// Answers whether the part has a model (the caller's second 00550980). Completion: 100%.
/// # Errors
/// No file from the host for an EGM to reload, or a stop in the morph.
pub(crate) fn apply_part_morph(
    cx: &mut InterfaceCx,
    geometry: Entity,
    difference: &[FaceGenMatrix; 4],
) -> Result<bool, GameError> {
    use crate::bs::face_gen_model::BsFaceGenModelMap;
    let Some(part) = cx
        .world
        .get::<crate::tes::face_gen_nodes::FaceGenPartMorph>(geometry)
        .cloned()
    else {
        return Ok(false);
    };
    let (egm_path, loaded) = {
        let map = cx.world.resource::<BsFaceGenModelMap>();
        let model = &map.models[part.model];
        (model.egm_path_00.clone(), model.egm_08.is_some())
    };
    if !loaded {
        if egm_path.is_empty() {
            return Ok(true);
        }
        let path = String::from_utf8_lossy(&egm_path).into_owned();
        let mut opened = cx.host.open_face_gen_file(&path);
        if opened.is_none() {
            return Err(GameError::unclassified(
                0x00559190,
                "00559190: an EGM to reload with no host files",
            ));
        }
        let mut egm = crate::bs::face_gen_files::EgmData::default();
        let mut warnings = Vec::new();
        let mut last_error = cx
            .world
            .remove_resource::<crate::tes::facegen::FaceGenLastError>()
            .unwrap_or_default();
        let _ = crate::bs::face_gen_files::read_egm(
            &mut egm,
            opened.as_mut(),
            &egm_path,
            &mut warnings,
            &mut last_error,
        );
        cx.world.insert_resource(last_error);
        for text in warnings {
            cx.host.warning(&text);
        }
        cx.world.resource_mut::<BsFaceGenModelMap>().models[part.model].egm_08 = Some(egm);
    }
    let egm = cx.world.resource::<BsFaceGenModelMap>().models[part.model]
        .egm_08
        .clone()
        .expect("loaded above");
    let Some(data) = cx
        .world
        .get::<crate::gamebryo::mesh_geometry::NiGeometry>(geometry)
        .and_then(|links| links.data_b4)
    else {
        return Ok(true);
    };
    let mut positions =
        crate::bs::face_gen_model::geometry_positions(cx.world, geometry).unwrap_or_default();
    let mut base = part.base;
    let mut warnings = Vec::new();
    let outcome = crate::bs::face_gen_model::apply_coefficients(
        &egm,
        difference,
        1.0,
        Some(&mut base),
        &mut positions,
        None,
        &mut warnings,
        true,
    )?;
    for text in warnings {
        cx.host.warning(&text);
    }
    if outcome == crate::bs::face_gen_model::ApplyOutcome::Applied {
        cx.world
            .get_mut::<crate::tes::face_gen_nodes::FaceGenPartMorph>(geometry)
            .expect("read above")
            .base = base;
        if let Some(mut strips) = cx
            .world
            .get_mut::<crate::gamebryo::mesh_tri_strips::MeshTriStripsData>(data)
        {
            strips.base.positions_1c = positions;
        } else if let Some(mut shape) = cx
            .world
            .get_mut::<crate::gamebryo::mesh_geometry::MeshTriShapeData>(data)
        {
            shape.base.positions_1c = positions;
        }
    }
    Ok(true)
}
