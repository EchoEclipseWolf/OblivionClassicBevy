//! Phase 12b: the actor forms. TESRace (RACE 0x318), the TESActorBase part (0051E560), TESNPC (NPC_ 0x200) and TESCreature (CREA 0x140), with the creature sounds and the FaceGen matrices as a load writes them. Evidence: objdump of every address named here, evidence/phase12-window-2026-10-09/fam7-*.txt. Phase 11's `actor_process::ActorValues` sits at +0xD0.
//!
//! InitItem is 12d: RACE 0052BA30, NPC_ 00523F40 and CREA 0051CF10 are `no_init_item` until then. What the C++ grew later in these files (Phase 13c, Phase 14) isn't here.

use bevy_ecs::prelude::*;
use oblivion_core::crt;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::form::{FormIdentity, OriginalVtable};
use super::form_arms::{
    FormClass, FormLoadCx, default_editor_id, editor_id_of, reset_14_components,
    reset_18_components,
};
use super::form_components::{
    ComponentSlot, DataChunkTails, FormRef, TES_ATTRIBUTES_VTABLE, TES_MODEL_VTABLE,
    TesActorBaseData, TesAiForm, TesAnimation, TesAttackDamageForm, TesAttributes, TesContainer,
    TesFullName, TesHealthForm, TesModel, TesModelList, TesModelTextures, TesObjectLinks,
    TesRaceForm, TesReaction, TesReactionForm, TesScriptableForm, TesSpellList, load_data_chunk,
    load_description, load_full_name, load_model, load_texture, resolve_texture_hashes,
};
use super::forms_items::set_change_flags;
use super::forms_magic::{save_load_flags, with_component};
use super::forms_misc::set_initialized;
use super::forms_script::{latin1, load_script_reference};
use crate::bs::string::BsStringT;
use crate::tes::actor::{MobileObject, TesObjectRefr, actor_is_dead};
use crate::tes::actor_enable::{
    TYPE_ARMOR, TYPE_CREATURE, TYPE_LIGHT, TYPE_WEAPON, low_process_only,
};
use crate::tes::actor_process::{ActorValues, MiddleHighProcess, construct_actor_values};
use crate::tes::base_extra_list::EXTRA_LEVELED_ITEM;
use crate::tes::biped_attach::{SkinTextureDeferrals, SkinTextureRequest};
use crate::tes::data_handler::{
    DataHandlerGlobals, DataHandlerLists, FormsById, TES_DESCRIPTION_VTABLE, TES_TEXTURE_VTABLE,
    TesDescription, TesTexture,
};
use crate::tes::data_load::initialize_form_from_current_record;
use crate::tes::equipment::{
    base_entries, best_ammo, best_armor, best_clothing, best_weapon, enchantment_of, equip_item,
    form_worn, unequip_worn_stacks,
};
use crate::tes::face_gen_head::{copy_facegen, facegen_coefficients};
use crate::tes::facegen::{FaceGenMatrix, resize_matrix_data};
use crate::tes::form::{biped_mask, construct_tes_form};
use crate::tes::form_components::{covers, reset_description};
use crate::tes::forms_init::form_type;
use crate::tes::forms_items::weapon_type;
use crate::tes::inventory::{ensure_container_changes, put_container_changes, reference_container};
use crate::tes::player::PlayerGlobals;
use crate::tes::save_game::{SaveWriter, form_id};
use crate::tes::tes::GameThreadLocals;
use crate::tes::tes_file::{c_bytes, map_file_form_id};

/// IDA: the form types of this family.
pub const FORM_TYPE_RACE: u8 = 0x09;
pub const FORM_TYPE_NPC: u8 = 0x23;
pub const FORM_TYPE_CREATURE: u8 = 0x24;
// IDA: the types the casts of 0052DB07, 0052DBD7 and 005199F5 test: TESHair, TESEyes, TESSound.
const FORM_TYPE_HAIR: u8 = 0x07;
const FORM_TYPE_EYES: u8 = 0x08;
const FORM_TYPE_SOUND: u8 = 0x0A;

/// IDA: 0052D100: the form, TESFullName +0x18, TESDescription +0x24, TESSpellList +0x2C, TESReactionForm +0x40.
pub const RACE_VTABLES: [u32; 5] = [
    0x00A5_48FC,
    0x00A5_48E4,
    0x00A5_48CC,
    0x00A5_48AC,
    0x00A5_4898,
];
/// IDA: 00527220: the form, then +0x24, +0x44, +0x54, +0x68, +0x80, +0x88, +0x94, +0xA0, +0xAC, +0xC4, and TESRaceForm +0xE4.
pub const NPC_VTABLES: [u32; 12] = [
    0x00A5_3DD4,
    0x00A5_3D6C,
    0x00A5_3D58,
    0x00A5_3D38,
    0x00A5_3D14,
    0x00A5_3CFC,
    0x00A5_3CDC,
    0x00A5_3CC8,
    0x00A5_3CB4,
    0x00A5_3C94,
    0x00A5_3C80,
    0x00A5_3C6C,
];
/// IDA: 0051EB80: the form, then +0x24, +0x44, +0x54, +0x68, +0x80, +0x88, +0x94, +0xA0, +0xAC, +0xC4, and TESAttackDamageForm +0xE4, TESModelList +0xEC.
pub const CREATURE_VTABLES: [u32; 13] = [
    0x00A5_324C,
    0x00A5_31E4,
    0x00A5_31CC,
    0x00A5_31AC,
    0x00A5_3188,
    0x00A5_3170,
    0x00A5_3150,
    0x00A5_313C,
    0x00A5_3128,
    0x00A5_3108,
    0x00A5_30F4,
    0x00A5_30DC,
    0x00A5_30C8,
];
/// IDA: the NiTArray vtable 0052D261 and 00527318 store.
pub const FACE_GEN_UNDO_ARRAY_VTABLE: u32 = 0x00A5_3950;

// IDA: the chunks 0052D800, 00527E40 and 0051DD00 compare.
const EDID: u32 = 0x4449_4445;
const FULL: u32 = 0x4C4C_5546;
const DESC: u32 = 0x4353_4544;
const DATA: u32 = 0x4154_4144;
const ICON: u32 = 0x4E4F_4349;
const SCRI: u32 = 0x4952_4353;
const SPLO: u32 = 0x4F4C_5053;
const SNAM: u32 = 0x4D41_4E53;
const CNTO: u32 = 0x4F54_4E43;
const ACBS: u32 = 0x5342_4341;
const AIDT: u32 = 0x5444_4941;
const PKID: u32 = 0x4449_4B50;
const KFFZ: u32 = 0x5A46_464B;
const NIFZ: u32 = 0x5A46_494E;
const NIFT: u32 = 0x5446_494E;
const ATTR: u32 = 0x5254_5441;
const XNAM: u32 = 0x4D41_4E58;
const VNAM: u32 = 0x4D41_4E56;
const PNAM: u32 = 0x4D41_4E50;
const UNAM: u32 = 0x4D41_4E55;
const DNAM: u32 = 0x4D41_4E44;
const FNAM: u32 = 0x4D41_4E46;
const MNAM: u32 = 0x4D41_4E4D;
const HNAM: u32 = 0x4D41_4E48;
const ENAM: u32 = 0x4D41_4E45;
const CNAM: u32 = 0x4D41_4E43;
const INAM: u32 = 0x4D41_4E49;
const LNAM: u32 = 0x4D41_4E4C;
const RNAM: u32 = 0x4D41_4E52;
const ZNAM: u32 = 0x4D41_4E5A;
const TNAM: u32 = 0x4D41_4E54;
const WNAM: u32 = 0x4D41_4E57;
const BNAM: u32 = 0x4D41_4E42;
const NAM0: u32 = 0x304D_414E;
const NAM1: u32 = 0x314D_414E;
const NAM2: u32 = 0x324D_414E;
const NAM3: u32 = 0x334D_414E;
const NAM9: u32 = 0x394D_414E;
const HCLR: u32 = 0x524C_4348;
const INDX: u32 = 0x5844_4E49;
const FGGS: u32 = 0x5347_4746;
const FGGA: u32 = 0x4147_4746;
const FGTS: u32 = 0x5354_4746;
const CSDT: u32 = 0x5444_5343;
const CSDI: u32 = 0x4944_5343;
const CSDF: u32 = 0x4644_5343;
const CSDC: u32 = 0x4344_5343;
const CSCR: u32 = 0x5243_5343;
const MODB: u32 = 0x4244_4F4D;
const MODL: u32 = 0x4C44_4F4D;
const MODT: u32 = 0x5444_4F4D;

// IDA: 00B10CCC, the nine face models 0052B840 stores (bytes read from the exe).
const FACE_MODELS: [&[u8]; 9] = [
    b"Characters\\Imperial\\HeadHuman.nif",
    b"Characters\\Imperial\\EarsHuman.nif",
    b"Characters\\Imperial\\EarsHuman.nif",
    b"Characters\\Imperial\\MouthHuman.nif",
    b"Characters\\Imperial\\TeethLowerHuman.nif",
    b"Characters\\Imperial\\TeethUpperHuman.nif",
    b"Characters\\Imperial\\TongueHuman.nif",
    b"Characters\\Imperial\\EyeLeftHuman.nif",
    b"Characters\\Imperial\\EyeRightHuman.nif",
];
// IDA: 00B10CF0, the nine face textures (bytes read from the exe).
const FACE_TEXTURES: [Option<&[u8]>; 9] = [
    Some(b"Characters\\Imperial\\HeadHuman.dds"),
    Some(b"Characters\\Imperial\\EarsHuman.dds"),
    Some(b"Characters\\Imperial\\EarsHuman.dds"),
    Some(b"Characters\\Imperial\\MouthHuman.dds"),
    Some(b"Characters\\Imperial\\TeethLowerHuman.dds"),
    Some(b"Characters\\Imperial\\TeethUpperHuman.dds"),
    Some(b"Characters\\Imperial\\TongueHuman.dds"),
    None,
    None,
];
// IDA: 00B10D14, the ten body textures (bytes read from the exe).
const BODY_TEXTURES: [Option<&[u8]>; 10] = [
    Some(b"Characters\\Imperial\\Male\\UpperBodyMale.dds"),
    Some(b"Characters\\Imperial\\Male\\LegMale.dds"),
    Some(b"Characters\\Imperial\\Male\\HandMale.dds"),
    Some(b"Characters\\Imperial\\Male\\HandMale.dds"),
    None,
    Some(b"Characters\\Imperial\\Female\\UpperBodyFemale.dds"),
    Some(b"Characters\\Imperial\\Female\\LegFemale.dds"),
    Some(b"Characters\\Imperial\\Female\\HandFemale.dds"),
    Some(b"Characters\\Imperial\\Female\\HandFemale.dds"),
    None,
];

/// IDA: the NiTArray<FaceGenUndo*> (0x10) both classes hold: vtable, +0x04 data, +0x08 u16 capacity, +0x0A u16 size, +0x0C u16 count, +0x0E u16 grow 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FaceGenUndoArray {
    pub original_vtable: u32,
    pub data_04: u32,
    pub capacity_08: u16,
    pub size_0a: u16,
    pub count_0c: u16,
    pub grow_0e: u16,
}

// IDA: N/A a helper, with no address of its own.
const fn construct_undo_array() -> FaceGenUndoArray {
    FaceGenUndoArray {
        original_vtable: FACE_GEN_UNDO_ARRAY_VTABLE,
        data_04: 0,
        capacity_08: 0,
        size_0a: 0,
        count_0c: 0,
        grow_0e: 1,
    }
}

/// IDA: 0052D100 TESRace (0x318), with `TesFullName` +0x18, `TesDescription` +0x24, `TesSpellList` +0x2C and `TesReactionForm` +0x40 on the same entity: DATA +0x50 (0x24); the male and female TESAttributes +0x74, +0x80, members rather than bases, which 00466E80's cast doesn't find; +0x8C/+0x90 the hairs in list order; +0x94, +0x98 the default hairs (DNAM, raw); +0x9C byte hair color (CNAM); +0xA0, +0xA4 the FaceGen clamps (PNAM, UNAM); +0xA8/+0xAC the eyes; the body models +0xB0 (2), the face models +0xE0 (9), the face textures +0x1B8 (9), the body textures +0x224 (2 x 5), FaceGen +0x29C (4), +0x2FC u16 (SNAM), +0x300, +0x304 the voices (VNAM, raw), the undo array +0x308.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct TesRace {
    pub data_50: [u8; 0x24],
    pub attributes_74: TesAttributes,
    pub attributes_80: TesAttributes,
    pub hairs_8c: Vec<Entity>,
    pub default_hairs_94: [FormRef; 2],
    pub hair_color_9c: u8,
    pub facegen_main_a0: f32,
    pub facegen_face_a4: f32,
    pub eyes_a8: Vec<Entity>,
    pub body_models_b0: [TesModel; 2],
    pub face_models_e0: [TesModel; 9],
    pub face_textures_1b8: [TesTexture; 9],
    pub body_textures_224: [TesTexture; 10],
    pub facegen_29c: [FaceGenMatrix; 4],
    pub snam_2fc: u16,
    pub voices_300: [FormRef; 2],
    pub undo_308: FaceGenUndoArray,
}

impl Default for TesRace {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self {
            data_50: [0; 0x24],
            attributes_74: TesAttributes::default(),
            attributes_80: TesAttributes::default(),
            hairs_8c: Vec::new(),
            default_hairs_94: [FormRef::default(); 2],
            hair_color_9c: 0,
            facegen_main_a0: 0.0,
            facegen_face_a4: 0.0,
            eyes_a8: Vec::new(),
            body_models_b0: Default::default(),
            face_models_e0: Default::default(),
            face_textures_1b8: Default::default(),
            body_textures_224: Default::default(),
            facegen_29c: Default::default(),
            snam_2fc: 0,
            voices_300: [FormRef::default(); 2],
            undo_308: FaceGenUndoArray::default(),
        }
    }
}

/// IDA: 00527220 TESNPC (0x200): the TESActorBase part and `TesRaceForm` +0xE4 on the same entity; the 21 skills +0xEC; +0x104 class (CNAM, raw); FaceGen +0x108 (4, FGGS, FGGA, FGTS) and +0x168 (4); +0x1C8 hair; +0x1CC hair length; +0x1D0 eyes; +0x1D4 the FaceGen head's BSFaceGenNiNodeBiped and +0x1D8 its BSFaceGenNiNodeSkinned (00528A10 stores both, 00523220 releases them), +0x1DC an NiRefObject pointer whose role is open (005255A0 releases it), null while loading; +0x1E0 u16 (FNAM, 0xFF); +0x1E4 combat style (raw); +0x1E8 hair color; +0x1EC the record offset; the undo array +0x1F0.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesNpc {
    pub skills_ec: [u8; 21],
    pub class_104: FormRef,
    pub facegen_108: [FaceGenMatrix; 4],
    pub facegen_168: [FaceGenMatrix; 4],
    pub hair_1c8: Option<Entity>,
    pub hair_length_1cc: f32,
    pub eyes_1d0: Option<Entity>,
    pub face_nodes_1d4: [u32; 3],
    pub fnam_1e0: u16,
    pub combat_style_1e4: FormRef,
    pub hair_color_1e8: u32,
    pub record_offset_1ec: u32,
    pub undo_1f0: FaceGenUndoArray,
}

/// IDA: an 8-byte block of a creature sound list (FormHeap): +0x00 TESSound*, +0x04 u8 chance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CreatureSoundEntry {
    pub sound_00: Option<Entity>,
    pub chance_04: u8,
}

/// IDA: 005196D0 the creature sounds (0x28): ten list pointers, by sound type, each list in 00519880's chance order. `None` is a null pointer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CreatureSounds {
    pub lists: [Option<Vec<CreatureSoundEntry>>; 10],
}

/// IDA: TESCreature +0x100: with +0x28 bit 8 the sound block, otherwise the CSCR form ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CreatureSoundsSlot {
    Reference(FormRef),
    Sounds(Box<CreatureSounds>),
}

impl Default for CreatureSoundsSlot {
    /// IDA: N/A Rust's `Default`.
    fn default() -> Self {
        Self::Reference(FormRef::Raw(0))
    }
}

/// IDA: 0051EB80 TESCreature (0x140): the TESActorBase part, `TesAttackDamageForm` +0xE4 and `TesModelList` +0xEC on the same entity; +0x100 the sounds or the CSCR ID; DATA +0x104 (type, combat, magic, stealth, u16 soul); +0x10A reach (RNAM, 0x20); +0x10C turning speed (TNAM); +0x110 foot weight (WNAM, 3.0); +0x114 scale (BNAM, 1.0); +0x118 combat style (ZNAM, raw); the TESModel +0x11C (NAM0) and TESTexture +0x134 (NAM1), members rather than bases.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesCreature {
    pub sounds_100: CreatureSoundsSlot,
    pub data_104: [u8; 4],
    pub soul_108: u16,
    pub reach_10a: u8,
    pub turning_10c: f32,
    pub foot_weight_110: f32,
    pub scale_114: f32,
    pub combat_style_118: FormRef,
    pub blood_11c: TesModel,
    pub decal_134: TesTexture,
}

/// IDA: TESActorBase +0xD0, 0065BE10's ActorValues.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ActorBaseValues(pub ActorValues);

// IDA: the FGGS / FGGA / FGTS arm of 0052D800 and 00527E40: rows = size >> 2, one column, 00527160; a zeroed alloca of the size, 00450C20(size); then each float through `fld`/`fstp` into row i. Completion: 100%.
fn load_facegen_matrix(
    matrix: &mut FaceGenMatrix,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let size = cx.file.chunk_size()?;
    let rows = size >> 2;
    matrix.rows = rows;
    matrix.cols = 1;
    resize_matrix_data(matrix, rows);
    let mut buffer = vec![0_u8; size as usize];
    cx.file.read_chunk(&mut buffer, size)?;
    for (slot, word) in matrix.data.iter_mut().zip(buffer.as_chunks::<4>().0) {
        *slot = x87::fld_fstp_copy(u32::from_le_bytes(*word));
    }
    Ok(())
}

// IDA: 00468810 / 00468860 and 0046DD20 / 0046DD70 under 004688D0 and 0046DFE0: a block of the chunk's size, read whole, a first byte of 0 ending it; each name not on the list by _stricmp is copied (FormHeap) and appended (0067B1E0); the next name starts past the NUL, and an empty one ends it. The host's two bytes of padding stand for the heap past the block. Completion: 100%.
fn load_name_list(names: &mut Vec<Vec<u8>>, cx: &mut FormLoadCx<'_, '_>) -> Result<(), GameError> {
    let size = cx.file.chunk_size()? as usize;
    let mut buffer = vec![0_u8; size + 2];
    cx.file.read_chunk(&mut buffer, 0)?;
    let mut at = 0;
    while buffer[at] != 0 {
        let name = c_bytes(&buffer[at..]);
        if !names.iter().any(|item| crt::stricmp(item, name) == 0) {
            names.push(name.to_vec());
        }
        at += name.len() + 1;
    }
    Ok(())
}

// IDA: 0046DE60 TESModelList's NIFT: 0046DE10 frees the lists; an alloca of the chunk's size, read whole; +0x0C = the first dword, FormHeap(4 * it); for each, the names' next node (a null node faults at 0046DEE8), an 8-byte block, a count byte, the 0046D610 hashes with the RTTI cast's owner, and the offset moves past them. A block past the chunk reads the stack in the original: a host stop. Completion: 100%.
// TODO: [ ] 0046D722's "on UNKNOWN Model" warning, for a first texture list with no name at all: `resolve_texture_hashes` takes "" there.
fn load_model_list_textures(
    world: &World,
    list: &mut TesModelList,
    owner: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    list.textures_10 = None;
    list.count_0c = 0;
    let size = cx.file.chunk_size()?;
    let mut buffer = vec![0_u8; (size as usize).max(4)];
    cx.file.read_chunk(&mut buffer, size)?;
    let count = u32::from_le_bytes(buffer[..4].try_into().expect("four bytes"));
    list.count_0c = count;
    let mut textures = Vec::new();
    let mut at = 4_usize;
    for index in 0..count as usize {
        if index > 0 && index >= list.names_04.len() {
            return Err(GameError::unclassified(
                0x0046DEE8,
                "0046DEE8: NIFT names a null list node",
            ));
        }
        let path = list.names_04.get(index).cloned().unwrap_or_default();
        let entries = usize::from(*buffer.get(at).ok_or(GameError::unclassified(
            0x0046DF13,
            "0046DF13: a NIFT block past the chunk",
        ))?);
        at += 1;
        let data = buffer
            .get(at..at + entries * 0x18)
            .ok_or(GameError::unclassified(
                0x0046DF4C,
                "0046DF4C: a NIFT block past the chunk",
            ))?;
        let (count_00, entries_04) =
            resolve_texture_hashes(world, data, entries as u32, &path, Some(owner), cx);
        textures.push(TesModelTextures {
            count_00,
            entries_04,
        });
        at += entries * 0x18;
    }
    list.textures_10 = Some(textures);
    Ok(())
}

// IDA: 0045A500 on the main thread (the C++'s reading of it too): g_saveLoadGame +0x18 bit 0.
fn save_load_active(world: &World) -> bool {
    save_load_flags(world) & 1 != 0
}

// IDA: the AI setters of the constructors, each with vt+0x10(0x100): 00468210 +0x07, 00468180 +0x04, 004681B0 +0x05, 004681E0 +0x06, in that order.
fn construct_ai_bytes(world: &mut World, form: Entity, first: u8) -> Result<(), GameError> {
    let mut ai = world.get_mut::<TesAiForm>(form).expect("just built");
    ai.ai_04[3] = 0x32;
    ai.ai_04[0] = first;
    ai.ai_04[1] = 0x32;
    ai.ai_04[2] = 0x32;
    set_change_flags(world, form, 0x100)
}

// IDA: 0051E560 TESActorBase_Constructor: 004B31D0, 00467320, 00469690, 0046F2C0, 004684A0, 0046CA50, 00468BB0, 00468720, +0xA0 TESFullName inline, 0046D7E0, 0046EF60, the class's vtables over TESActorBase's, 0065BE10 on +0xD0. `vtables` is the class's list. Completion: 100% of the store order.
fn construct_actor_base(world: &mut World, vtables: &[u32]) -> Result<Entity, GameError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        OriginalVtable(vtables[0]),
        TesObjectLinks::default(),
        TesActorBaseData {
            original_vtable: vtables[1],
            magicka_08: 0x32,
            fatigue_0a: 0x32,
            level_0e: 1,
            ..TesActorBaseData::default()
        },
        TesContainer {
            original_vtable: vtables[2],
            ..TesContainer::default()
        },
        TesSpellList {
            original_vtable: vtables[3],
            ..TesSpellList::default()
        },
        TesAiForm {
            original_vtable: vtables[4],
            ..TesAiForm::default()
        },
        TesHealthForm {
            original_vtable: vtables[5],
            health_04: 0,
        },
        TesAttributes::new(vtables[6]),
        TesAnimation {
            original_vtable: vtables[7],
            ..TesAnimation::default()
        },
        TesFullName {
            original_vtable: vtables[8],
            name_04: BsStringT::default(),
        },
        TesModel {
            original_vtable: vtables[9],
            ..TesModel::default()
        },
        TesScriptableForm {
            original_vtable: vtables[10],
            ..TesScriptableForm::default()
        },
        ActorBaseValues(construct_actor_values()),
    ));
    Ok(form)
}

// IDA: the TESActorBase chunk arms 00527E40 and 0051DD00 share. Returns whether the chunk was one of them. Order within an arm is the store order. Completion: 100%.
fn load_actor_base_chunk(
    world: &mut World,
    form: Entity,
    chunk: u32,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    match chunk {
        // Alloca, size 0x200; vt+0xD8 006FDFF0 keeps nothing.
        EDID => {
            cx.file.read_text(0x200)?;
        }
        FULL => load_full_name(
            &mut world
                .get_mut::<TesFullName>(form)
                .expect("the class's component")
                .name_04,
            cx,
        )?,
        SCRI => load_script_reference(world, form, cx)?,
        // 0046F420 on +0x54.
        SPLO => {
            let (id, _) = cx.file.read_dword(0)?;
            let loading = world.resource::<GameThreadLocals>().byte_184 != 0;
            world
                .get_mut::<TesSpellList>(form)
                .expect("the class's component")
                .add_spell_while_loading(id, loading)?;
        }
        // Zeroed locals, 00450C20(8), 004675E0(faction, the rank's low byte).
        SNAM => {
            let mut bytes = [0_u8; 8];
            cx.file.read_chunk(&mut bytes, 8)?;
            let faction = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            if faction != 0 {
                set_change_flags(world, form, 0x40)?;
                world
                    .get_mut::<TesActorBaseData>(form)
                    .expect("the class's component")
                    .set_faction_rank(faction, bytes[4] as i8);
            }
        }
        // Zeroed locals, 00450C20(8), 00469500(0), 00469750 on {item, count}.
        CNTO => {
            let mut bytes = [0_u8; 8];
            cx.file.read_chunk(&mut bytes, 8)?;
            let mut container = world
                .get_mut::<TesContainer>(form)
                .expect("the class's component");
            container.flags_04 &= 0xFE;
            container.add_entry(
                u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
                i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            );
        }
        // 00450C20(+0x28, 0x10), straight over the fields: a short chunk leaves the rest. The C++ zeroes them.
        ACBS => {
            let mut base = world
                .get_mut::<TesActorBaseData>(form)
                .expect("the class's component");
            let mut bytes = base.acbs_bytes();
            cx.file.read_chunk(&mut bytes, 0x10)?;
            base.set_acbs_bytes(&bytes);
        }
        // Zeroed locals, 00450C20(0x0C), 00468280.
        AIDT => {
            let mut bytes = [0_u8; 0x0C];
            cx.file.read_chunk(&mut bytes, 0x0C)?;
            world
                .get_mut::<TesAiForm>(form)
                .expect("the class's component")
                .set_data(&bytes);
            set_change_flags(world, form, 0x100)?;
        }
        // A zeroed local, 004510E0, 00468380 -> 0067B1E0, which ignores a null.
        PKID => {
            let (id, _) = cx.file.read_dword(0)?;
            if id != 0 {
                world
                    .get_mut::<TesAiForm>(form)
                    .expect("the class's component")
                    .packages_10
                    .push(FormRef::Raw(id));
            }
        }
        KFFZ => {
            with_component(world, form, |_, animation: &mut TesAnimation| {
                load_name_list(&mut animation.names_04, cx)
            })?;
        }
        // 0046DB00 on +0xAC.
        MODB | MODL | MODT => {
            with_component(world, form, |world, model: &mut TesModel| {
                load_model(world, model, Some(form), cx)
            })?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

// IDA: the hair and eyes arms of 00527E40: an uninitialized local, 004510E0, 0046BB20 with this file, 004471A0 (0 finds nothing) and the type's cast.
fn find_form_of_type(
    world: &World,
    form_type: u8,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(u32, Option<Entity>), GameError> {
    let (raw, _) = cx.file.read_dword(0)?;
    let id = map_file_form_id(world, cx, raw, Some(cx.file.id))?;
    let found = (id != 0)
        .then(|| world.resource::<FormsById>().lookup(id))
        .flatten()
        .filter(|&form| {
            world
                .get::<FormIdentity>(form)
                .expect("a form")
                .form_type_04
                == form_type
        });
    Ok((id, found))
}

// IDA: the LNAM arm 005281CE..00528225: `fcom` with 0.0, where only an ordered less-than takes 0.0, then `fcom` of 1.0 with it, where only an ordered greater takes 1.0; so a NaN stays, through `fld`/`fstp`, and -0.0 stays. The C++ makes a NaN 0.0. Completion: 100%.
#[allow(
    clippy::manual_clamp,
    reason = "005281E5 and 00528207: the original's two `fcom`s, in their order, which a NaN falls through"
)]
fn clamp_hair_length(bits: u32) -> f32 {
    let value = x87::fld_fstp_copy(bits);
    if value < 0.0 {
        0.0
    } else if value > 1.0 {
        1.0
    } else {
        value
    }
}

// IDA: 00527E40 TESNPC vt+0x1C LoadForm. `cmp al, 0x23` (else 0); +0x1EC = file +0x25C; 00451210; 0046AB80(0); the chunks to 0 or a failed advance; then form ID 7 with a g_thePlayer whose vt+0x170 is null: 006C5420 stores this as its +0x1C. Returns 1. Completion: 100%.
fn load_npc_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_NPC {
        return Ok(false);
    }
    let offset = cx.file.record_offset()?;
    world
        .get_mut::<TesNpc>(form)
        .expect("the class's component")
        .record_offset_1ec = offset;
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    let mut face_warned = false;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        if !load_actor_base_chunk(world, form, chunk, cx)? {
            load_npc_chunk(world, form, chunk, &mut face_warned, cx)?;
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    // 005284DA..005284FF.
    if world.get::<FormIdentity>(form).expect("a form").form_id_0c == 7 {
        let player = world
            .get_resource::<PlayerGlobals>()
            .and_then(|globals| globals.the_player_b333c4);
        if let Some(player) = player
            && let Some(mut reference) = world.get_mut::<TesObjectRefr>(player)
            && reference.base_form_1c.is_none()
        {
            reference.base_form_1c = Some(form);
        }
    }
    Ok(true)
}

// IDA: the arms of 00527E40 past the TESActorBase ones, by its compares and the byte table 0052853C over 0052851C. Completion: 100%.
fn load_npc_chunk(
    world: &mut World,
    form: Entity,
    chunk: u32,
    face_warned: &mut bool,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    match chunk {
        // 00527F35..00527FE5: 0046BFD0 is 12, TESHealthForm and TESAttributes.
        DATA => {
            let size = cx.file.chunk_size()?.wrapping_sub(12);
            if size == 0x17 {
                let mut local = [0_u8; 0x17];
                load_npc_data(world, form, &mut local, cx)?;
                // 00527F6E..00527F9E: past the first byte.
                world
                    .get_mut::<TesNpc>(form)
                    .expect("the class's component")
                    .skills_ec
                    .copy_from_slice(&local[1..0x16]);
            } else if size == 0x15 {
                let mut skills = world
                    .get::<TesNpc>(form)
                    .expect("the class's component")
                    .skills_ec;
                load_npc_data(world, form, &mut skills, cx)?;
                world
                    .get_mut::<TesNpc>(form)
                    .expect("the class's component")
                    .skills_ec = skills;
            } else {
                let name = latin1(&editor_id_of(world, form));
                let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
                let file = cx.file.file()?.name_1c.clone();
                cx.file.services.warn(format!(
                    "Unrecognized data format for NPC '{name}' ({id:08X}) in file '{file}'."
                ));
            }
        }
        // 00527EDE: the first warns.
        NAM0 | NAM1 | NAM2 | NAM3 => {
            if !*face_warned {
                let name = latin1(&editor_id_of(world, form));
                let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
                let file = cx.file.file()?.name_1c.clone();
                cx.file.services.warn(format!(
                    "Found face texture for NPC '{name}' ({id:08X}) in file '{file}'"
                ));
                *face_warned = true;
            }
        }
        // 0052835A, with no zeroed local: an empty chunk leaves +0x1E8.
        NAM9 | HCLR => {
            let mut npc = world
                .get_mut::<TesNpc>(form)
                .expect("the class's component");
            let (bits, _) = cx.file.read_dword(npc.hair_color_1e8)?;
            npc.hair_color_1e8 = bits;
        }
        // 005283E1: index 0x0B, 0x0C, 0x0D past the form, then 005528F0 to +0x168.
        FGGS | FGGA | FGTS => {
            let index = match chunk {
                FGGS => 0,
                FGGA => 1,
                _ => 2,
            };
            with_component(world, form, |_, npc: &mut TesNpc| {
                load_facegen_matrix(&mut npc.facegen_108[index], cx)?;
                let TesNpc {
                    facegen_108,
                    facegen_168,
                    ..
                } = npc;
                copy_facegen(facegen_108, facegen_168);
                Ok::<(), GameError>(())
            })?;
        }
        // 005280B4.
        CNAM => {
            let (id, _) = cx.file.read_dword(0)?;
            world
                .get_mut::<TesNpc>(form)
                .expect("the class's component")
                .class_104 = FormRef::Raw(id);
        }
        // 005282BC: an uninitialized local; vt+0x124 00521B40: +0x1E4, then 0046ABF0(0x400).
        ZNAM => {
            let (id, _) = cx.file.read_dword(0)?;
            world
                .get_mut::<TesNpc>(form)
                .expect("the class's component")
                .combat_style_1e4 = FormRef::Raw(id);
            set_change_flags(world, form, 0x400)?;
        }
        // 0052822A.
        ENAM => {
            let (id, found) = find_form_of_type(world, FORM_TYPE_EYES, cx)?;
            if found.is_some() {
                world
                    .get_mut::<TesNpc>(form)
                    .expect("the class's component")
                    .eyes_1d0 = found;
            } else {
                let name = latin1(&editor_id_of(world, form));
                let own = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
                cx.file.services.warn(format!(
                    "Could not find eyes ({id:08X}) on NPC '{name}' ({own:08X})."
                ));
            }
        }
        // 0052829C: only with 2 bytes or more, 004510F0.
        FNAM => {
            if cx.file.chunk_size()? >= 2 {
                let mut npc = world
                    .get_mut::<TesNpc>(form)
                    .expect("the class's component");
                let mut bytes = npc.fnam_1e0.to_le_bytes();
                cx.file.read_chunk(&mut bytes, 2)?;
                npc.fnam_1e0 = u16::from_le_bytes(bytes);
            }
        }
        // 0052815C.
        HNAM => {
            let (id, found) = find_form_of_type(world, FORM_TYPE_HAIR, cx)?;
            if found.is_some() {
                world
                    .get_mut::<TesNpc>(form)
                    .expect("the class's component")
                    .hair_1c8 = found;
            } else {
                let name = latin1(&editor_id_of(world, form));
                let own = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
                cx.file.services.warn(format!(
                    "Could not find hair ({id:08X}) on NPC '{name}' ({own:08X})."
                ));
            }
        }
        // 00528127: a zeroed local; +0x38, the base data's +0x14.
        INAM => {
            let (id, _) = cx.file.read_dword(0)?;
            world
                .get_mut::<TesActorBaseData>(form)
                .expect("the class's component")
                .death_item_14 = FormRef::Raw(id);
        }
        // 005281CE, with an uninitialized local: the host's is 0.
        LNAM => {
            let (bits, _) = cx.file.read_dword(0)?;
            world
                .get_mut::<TesNpc>(form)
                .expect("the class's component")
                .hair_length_1cc = clamp_hair_length(bits);
        }
        // 00528140: a zeroed local; +0xE8, TESRaceForm's +0x04.
        RNAM => {
            let (id, _) = cx.file.read_dword(0)?;
            world
                .get_mut::<TesRaceForm>(form)
                .expect("the class's component")
                .race_04 = FormRef::Raw(id);
        }
        _ => {}
    }
    Ok(())
}

// IDA: 0046BDA0 for TESNPC: `destination` of the skills' size, with the TESHealthForm and TESAttributes tails.
fn load_npc_data(
    world: &mut World,
    form: Entity,
    destination: &mut [u8],
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let mut health = world
        .get::<TesHealthForm>(form)
        .expect("the class's component")
        .health_04;
    let mut attributes = world
        .get::<TesAttributes>(form)
        .expect("the class's component")
        .values_04;
    load_data_chunk(
        destination,
        destination.len() as u16,
        DataChunkTails {
            health: Some(&mut health),
            attributes: Some(&mut attributes),
            ..DataChunkTails::default()
        },
        cx,
    )?;
    world
        .get_mut::<TesHealthForm>(form)
        .expect("the class's component")
        .health_04 = health;
    world
        .get_mut::<TesAttributes>(form)
        .expect("the class's component")
        .values_04 = attributes;
    Ok(())
}

// IDA: 005255A0 TESNPC vt+0x14: the skills and +0x100 = 0; +0x1CC = 0.0; +0x1C8 = +0x1D0 = 0; with 0045A500 clear the +0x1D8, +0x1D4 and +0x1DC NiRefObject pointers go null; +0x1E0 = 0xFF; +0x104 = +0x1E4 = 0; +0x1E8 = 0x19324B; vt+0x128(0x45) nonzero picks +0x168 for 005528F0, zero +0x108 (the C++ always takes +0x108); each of the eight matrices with rows zeroes its floats (memset); the skills = 5; 0046C2E0; +0x28 &= ~0x200 with vt+0x50(0x10); +0x84 = 0x32. Completion: 100%.
fn reset_npc(world: &mut World, form: Entity) -> Result<(), GameError> {
    let source = facegen_coefficients(world)?;
    let save_load = save_load_active(world);
    // 005232D0 -> 00519D90(0x45): past 0x27, 0065CB80 on +0xD0 through 009828C0. Code 0x45 is one of the list's (0065C010).
    let selector = world
        .get::<ActorBaseValues>(form)
        .expect("the class's component")
        .0
        .list_00
        .iter()
        .find(|entry| entry.code_00 == 0x45)
        .map_or(0, |entry| x87::ftol(entry.value_04));
    {
        let mut npc = world
            .get_mut::<TesNpc>(form)
            .expect("the class's component");
        npc.skills_ec = [0; 21];
        npc.hair_length_1cc = 0.0;
        npc.hair_1c8 = None;
        npc.eyes_1d0 = None;
        if !save_load {
            npc.face_nodes_1d4 = [0; 3];
        }
        npc.fnam_1e0 = 0xFF;
        npc.class_104 = FormRef::Raw(0);
        npc.combat_style_1e4 = FormRef::Raw(0);
        npc.hair_color_1e8 = 0x0019_324B;
        let TesNpc {
            facegen_108,
            facegen_168,
            ..
        } = &mut *npc;
        copy_facegen(
            &source,
            if selector != 0 {
                &mut *facegen_168
            } else {
                &mut *facegen_108
            },
        );
        for matrix in facegen_108.iter_mut().chain(facegen_168.iter_mut()) {
            if matrix.rows != 0 {
                matrix.data.fill(0.0);
            }
        }
        npc.skills_ec = [5; 21];
    }
    reset_14_components(world, form)?;
    world
        .get_mut::<TesActorBaseData>(form)
        .expect("the class's component")
        .flags_04 &= !0x200;
    set_change_flags(world, form, 0x10)?;
    world
        .get_mut::<TesHealthForm>(form)
        .expect("the class's component")
        .health_04 = 0x32;
    Ok(())
}

// IDA: 00521DA0 TESNPC vt+0x18: with 0045A500 clear the same three pointers go null; +0x1E0 = 0xFF; 0046C2F0. Completion: 100%.
fn destroy_npc(world: &mut World, form: Entity) -> Result<(), GameError> {
    let save_load = save_load_active(world);
    let mut npc = world
        .get_mut::<TesNpc>(form)
        .expect("the class's component");
    if !save_load {
        npc.face_nodes_1d4 = [0; 3];
    }
    npc.fnam_1e0 = 0xFF;
    reset_18_components(world, form)
}

// IDA: 00527220 TESNPC_Constructor: 0051E560; 0046E450 on +0xE4; the vtables; the two FaceGen arrays; +0x1D4..+0x1DC = 0; the undo array; +0x04 = 0x23; 005255A0; the four AI setters with 0x32, 5, 0x32, 0x32; +0x1EC = 0. Completion: 100% of the store order.
pub(crate) fn construct_npc(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_actor_base(world, &NPC_VTABLES)?;
    world.entity_mut(form).insert((
        TesRaceForm {
            original_vtable: NPC_VTABLES[11],
            race_04: FormRef::Raw(0),
        },
        TesNpc {
            undo_1f0: construct_undo_array(),
            ..TesNpc::default()
        },
    ));
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_NPC;
    reset_npc(world, form)?;
    construct_ai_bytes(world, form, 5)?;
    world
        .get_mut::<TesNpc>(form)
        .expect("just built")
        .record_offset_1ec = 0;
    Ok(form)
}

// IDA: 0051CDE0: without +0x28 bit 8 or with a null +0x100, the bit is set with vt+0x50(0x10), and a zeroed 0x28 block (005196D0) goes to +0x100. Completion: 100%.
fn ensure_sounds(world: &mut World, form: Entity) -> Result<(), GameError> {
    let flagged = world
        .get::<TesActorBaseData>(form)
        .expect("the class's component")
        .flags_04
        & 0x100
        != 0;
    let has_block = matches!(
        world
            .get::<TesCreature>(form)
            .expect("the class's component")
            .sounds_100,
        CreatureSoundsSlot::Sounds(_)
    );
    if !flagged || !has_block {
        world
            .get_mut::<TesActorBaseData>(form)
            .expect("the class's component")
            .flags_04 |= 0x100;
        set_change_flags(world, form, 0x10)?;
        world
            .get_mut::<TesCreature>(form)
            .expect("the class's component")
            .sounds_100 = CreatureSoundsSlot::Sounds(Box::default());
    }
    Ok(())
}

// IDA: 00519880 CreatureSounds_Add(entry, type): a type past 9 returns, unsigned, leaking the block; a null list pointer gets a new list; the walk stops at an empty head, or the first entry with a greater chance, before which 00446CB0 puts this one; otherwise 0067B1E0 appends it. Completion: 100%.
fn add_creature_sound(sounds: &mut CreatureSounds, entry: CreatureSoundEntry, sound_type: u32) {
    if sound_type > 9 {
        return;
    }
    let list = sounds.lists[sound_type as usize].get_or_insert_with(Vec::new);
    match list
        .iter()
        .position(|item| item.chance_04 > entry.chance_04)
    {
        Some(index) => list.insert(index, entry),
        None => list.push(entry),
    }
}

// IDA: 00517ED0 TESSound_FindByFile(name): the +0x6C list, the first whose +0x28 (or `Default`) matches by _stricmp. Completion: 100%.
fn find_sound_by_file(world: &World, name: &[u8]) -> Option<Entity> {
    let sounds = world.resource::<DataHandlerLists>();
    sounds.sounds_6c.items().find(|&sound| {
        let path = world
            .get::<super::form_components::TesSoundFile>(sound)
            .and_then(|file| file.path_04.data_00.as_deref())
            .unwrap_or_default();
        crt::stricmp(path, name) == 0
    })
}

// IDA: 00519960 CreatureSounds_LoadChunk(file, type, creature). CSDI: a zeroed local, 004510E0, FormHeap(8); a nonzero ID through 0046BB20 with this file, 0046B250 and the TESSound cast, a null warning; a zero ID a null sound. CSDF: 00450C20(a 0x104 local, 0x104), FormHeap(8), 00517ED0. Else 0. Then 0044FEA0; the next chunk must be CSDC, or the block is freed and it returns 0; 00450C20(+0x04, 1); 00519880; 1. The block's chance is unset for an empty CSDC, and 0 here. Completion: 100%.
fn load_creature_sound(
    world: &World,
    form: Entity,
    sound_type: u32,
    sounds: &mut CreatureSounds,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    let chunk = cx.file.chunk_type()?;
    let mut entry = CreatureSoundEntry::default();
    if chunk == CSDI {
        let (raw, _) = cx.file.read_dword(0)?;
        if raw != 0 {
            let id = map_file_form_id(world, cx, raw, Some(cx.file.id))?;
            entry.sound_00 = world.resource::<FormsById>().lookup(id).filter(|&sound| {
                world
                    .get::<FormIdentity>(sound)
                    .expect("a form")
                    .form_type_04
                    == FORM_TYPE_SOUND
            });
            if entry.sound_00.is_none() {
                let name = latin1(&editor_id_of(world, form));
                let own = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
                cx.file.services.warn(format!(
                    "Unable to find CreatureSound ID ({id:08X}) for creature '{name}' ({own:08X})."
                ));
            }
        }
    } else if chunk == CSDF {
        let name = cx.file.read_text(0x104)?;
        entry.sound_00 = find_sound_by_file(world, &name);
    } else {
        return Ok(false);
    }
    cx.file.advance_chunk()?;
    if cx.file.chunk_type()? != CSDC {
        return Ok(false);
    }
    let mut chance = [0_u8; 1];
    cx.file.read_chunk(&mut chance, 1)?;
    entry.chance_04 = chance[0];
    add_creature_sound(sounds, entry, sound_type);
    Ok(true)
}

// IDA: 0051CD60 vt of CSCR: with +0x28 bit 8, a nonzero ID frees the block (00519B40), clears the bit with vt+0x50(0x10) and stores it, and a zero ID does nothing; without the bit, the ID is stored. Completion: 100%.
fn set_sound_creature(world: &mut World, form: Entity, id: u32) -> Result<(), GameError> {
    let flagged = world
        .get::<TesActorBaseData>(form)
        .expect("the class's component")
        .flags_04
        & 0x100
        != 0;
    if flagged {
        if id != 0 {
            world
                .get_mut::<TesActorBaseData>(form)
                .expect("the class's component")
                .flags_04 &= !0x100;
            set_change_flags(world, form, 0x10)?;
            world
                .get_mut::<TesCreature>(form)
                .expect("the class's component")
                .sounds_100 = CreatureSoundsSlot::Reference(FormRef::Raw(id));
        }
    } else {
        world
            .get_mut::<TesCreature>(form)
            .expect("the class's component")
            .sounds_100 = CreatureSoundsSlot::Reference(FormRef::Raw(id));
    }
    Ok(())
}

// IDA: 0051DD00 TESCreature vt+0x1C LoadForm. `cmp al, 0x24` (else 0); 00451210; 0046AB80(0); the sound type local = -1; the chunks. CSDI or CSDF: 0051CDE0, 00519960; on a 0 the current chunk is read again with no advance. Returns 1. Completion: 100%.
fn load_creature_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_CREATURE {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    let mut sound_type = 0xFFFF_FFFF_u32;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        if chunk == CSDI || chunk == CSDF {
            ensure_sounds(world, form)?;
            let loaded = with_component(world, form, |world, creature: &mut TesCreature| {
                let CreatureSoundsSlot::Sounds(sounds) = &mut creature.sounds_100 else {
                    return Err(GameError::unclassified(
                        0x0051DEF9,
                        "0051DEF9: 00519960 with a null block faults",
                    ));
                };
                load_creature_sound(world, form, sound_type, sounds, cx)
            })?;
            if !loaded {
                chunk = cx.file.chunk_type()?;
                continue;
            }
        } else if !load_actor_base_chunk(world, form, chunk, cx)? {
            load_creature_chunk(world, form, chunk, &mut sound_type, cx)?;
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    Ok(true)
}

// IDA: the arms of 0051DD00 past the TESActorBase ones, by its compares and the table 0051E1B8 (SNAM 0051DFAC, TNAM 0051DFE9, WNAM 0051DFD6, ZNAM 0051DFFC). Completion: 100%.
fn load_creature_chunk(
    world: &mut World,
    form: Entity,
    chunk: u32,
    sound_type: &mut u32,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    match chunk {
        // 0051DF71.
        RNAM => {
            let mut creature = world
                .get_mut::<TesCreature>(form)
                .expect("the class's component");
            let mut byte = [creature.reach_10a];
            cx.file.read_chunk(&mut byte, 1)?;
            creature.reach_10a = byte[0];
        }
        // 0051DDE8, for a nonzero size: vt+0x18 0046E080 on +0x11C.
        NAM0 => {
            if cx.file.chunk_size()? != 0 {
                let text = cx.file.read_text(0)?;
                world
                    .get_mut::<TesCreature>(form)
                    .expect("the class's component")
                    .blood_11c
                    .path_04
                    .set(Some(&text), 0);
            }
        }
        // 0051DDB4, for a nonzero size: 004028D0 on +0x138.
        NAM1 => {
            if cx.file.chunk_size()? != 0 {
                let text = cx.file.read_text(0)?;
                world
                    .get_mut::<TesCreature>(form)
                    .expect("the class's component")
                    .decal_134
                    .path_04
                    .set(Some(&text), 0);
            }
        }
        // 0051DD9E: 0046BDA0(+0x104, 6), with the TESHealthForm, TESAttackDamageForm and TESAttributes tails.
        DATA => {
            let creature = world
                .get::<TesCreature>(form)
                .expect("the class's component");
            let mut bytes = [0_u8; 6];
            bytes[..4].copy_from_slice(&creature.data_104);
            bytes[4..].copy_from_slice(&creature.soul_108.to_le_bytes());
            let mut health = world
                .get::<TesHealthForm>(form)
                .expect("the class's component")
                .health_04;
            let mut damage = world
                .get::<TesAttackDamageForm>(form)
                .expect("the class's component")
                .damage_04;
            let mut attributes = world
                .get::<TesAttributes>(form)
                .expect("the class's component")
                .values_04;
            load_data_chunk(
                &mut bytes,
                6,
                DataChunkTails {
                    health: Some(&mut health),
                    attack_damage: Some(&mut damage),
                    attributes: Some(&mut attributes),
                    ..DataChunkTails::default()
                },
                cx,
            )?;
            let mut creature = world
                .get_mut::<TesCreature>(form)
                .expect("the class's component");
            creature.data_104.copy_from_slice(&bytes[..4]);
            creature.soul_108 = u16::from_le_bytes([bytes[4], bytes[5]]);
            world
                .get_mut::<TesHealthForm>(form)
                .expect("the class's component")
                .health_04 = health;
            world
                .get_mut::<TesAttackDamageForm>(form)
                .expect("the class's component")
                .damage_04 = damage;
            world
                .get_mut::<TesAttributes>(form)
                .expect("the class's component")
                .values_04 = attributes;
        }
        // 0051DF45: a zeroed local; +0x38.
        INAM => {
            let (id, _) = cx.file.read_dword(0)?;
            world
                .get_mut::<TesActorBaseData>(form)
                .expect("the class's component")
                .death_item_14 = FormRef::Raw(id);
        }
        // 0051DF5E 004510E0 onto +0x114, which an empty chunk leaves.
        BNAM => {
            let mut creature = world
                .get_mut::<TesCreature>(form)
                .expect("the class's component");
            let (bits, _) = cx.file.read_dword(creature.scale_114.to_bits())?;
            creature.scale_114 = f32::from_bits(bits);
        }
        // 0051DFE9 onto +0x10C.
        TNAM => {
            let mut creature = world
                .get_mut::<TesCreature>(form)
                .expect("the class's component");
            let (bits, _) = cx.file.read_dword(creature.turning_10c.to_bits())?;
            creature.turning_10c = f32::from_bits(bits);
        }
        // 0051DFD6 onto +0x110.
        WNAM => {
            let mut creature = world
                .get_mut::<TesCreature>(form)
                .expect("the class's component");
            let (bits, _) = cx.file.read_dword(creature.foot_weight_110.to_bits())?;
            creature.foot_weight_110 = f32::from_bits(bits);
        }
        // 0051DFFC: an uninitialized local; vt+0x124 0051C6C0: +0x118, then 0046ABF0(0x400).
        ZNAM => {
            let (id, _) = cx.file.read_dword(0)?;
            world
                .get_mut::<TesCreature>(form)
                .expect("the class's component")
                .combat_style_118 = FormRef::Raw(id);
            set_change_flags(world, form, 0x400)?;
        }
        // 0051E067: a zeroed local; 0051CD60.
        CSCR => {
            let (id, _) = cx.file.read_dword(0)?;
            set_sound_creature(world, form, id)?;
        }
        // 0051E125: an uninitialized local, which an empty chunk leaves: 0 here.
        CSDT => {
            let (value, _) = cx.file.read_dword(0)?;
            *sound_type = value;
        }
        // 0046DFE0 on +0xEC.
        NIFZ => {
            with_component(world, form, |_, list: &mut TesModelList| {
                load_name_list(&mut list.names_04, cx)
            })?;
        }
        NIFT => {
            with_component(world, form, |world, list: &mut TesModelList| {
                load_model_list_textures(world, list, form, cx)
            })?;
        }
        _ => {}
    }
    Ok(())
}

// IDA: 0051C7B0 TESCreature vt+0x14: +0x10C = 0.0; +0x110 = 3.0 (00A46C30); byte +0x104 = 0; +0x100 = 0, with no free; +0x118 = 0; +0x114 = 1.0; word +0x108 = 3; +0x105..+0x107 = 0x32; +0x10A = 0x20; vt+0 of +0x11C and +0x134; 0046C2E0; +0x28 |= 0x40 with vt+0x50(0x10); +0x28 |= 0x200 with vt+0x50(0x10); +0x84 = 0x32. Completion: 100%.
fn reset_creature(world: &mut World, form: Entity) -> Result<(), GameError> {
    {
        let mut creature = world
            .get_mut::<TesCreature>(form)
            .expect("the class's component");
        creature.turning_10c = 0.0;
        creature.foot_weight_110 = 3.0;
        creature.data_104[0] = 0;
        creature.sounds_100 = CreatureSoundsSlot::Reference(FormRef::Raw(0));
        creature.combat_style_118 = FormRef::Raw(0);
        creature.scale_114 = 1.0;
        creature.soul_108 = 3;
        creature.data_104[1] = 0x32;
        creature.data_104[2] = 0x32;
        creature.data_104[3] = 0x32;
        creature.reach_10a = 0x20;
        creature.blood_11c.init();
        creature.decal_134.path_04 = BsStringT::default();
    }
    reset_14_components(world, form)?;
    world
        .get_mut::<TesActorBaseData>(form)
        .expect("the class's component")
        .flags_04 |= 0x40;
    set_change_flags(world, form, 0x10)?;
    world
        .get_mut::<TesActorBaseData>(form)
        .expect("the class's component")
        .flags_04 |= 0x200;
    set_change_flags(world, form, 0x10)?;
    world
        .get_mut::<TesHealthForm>(form)
        .expect("the class's component")
        .health_04 = 0x32;
    Ok(())
}

// IDA: 0051C860 TESCreature vt+0x18: with +0x28 bit 8, a nonzero +0x100 goes through 00519B40 and the free, and +0x100 = 0; vt+4 of +0x11C and +0x134; 0046C2F0. Without the bit, +0x100 keeps a CSCR ID. Completion: 100%.
fn destroy_creature(world: &mut World, form: Entity) -> Result<(), GameError> {
    let flagged = world
        .get::<TesActorBaseData>(form)
        .expect("the class's component")
        .flags_04
        & 0x100
        != 0;
    let mut creature = world
        .get_mut::<TesCreature>(form)
        .expect("the class's component");
    if flagged {
        creature.sounds_100 = CreatureSoundsSlot::Reference(FormRef::Raw(0));
    }
    creature.blood_11c.destroy();
    reset_18_components(world, form)
}

// IDA: 0051EB80 TESCreature_Constructor: 0051E560; 00468A60 on +0xE4; 0046DBC0 on +0xEC; the vtables; 0046D7E0 on +0x11C; 0046FFD0 on +0x134; +0x04 = 0x24; 0051C7B0; the four AI setters with 0x32, 0x46, 0x32, 0x32; 00468AF0(6, 0x32): attribute 6 with vt+0x40(8); +0x28 |= 0x4000 with vt+0x50(0x10); with TLS +0x184 clear, 004675E0(00B36298, 0), a host stop. Completion: 100% of the arm a load takes.
// TODO: [ ] 0051ECDA, for a creature built outside a load, once a caller needs it (00B36298 is 0044AC20's).
fn construct_creature(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_actor_base(world, &CREATURE_VTABLES)?;
    world.entity_mut(form).insert((
        TesAttackDamageForm {
            original_vtable: CREATURE_VTABLES[11],
            damage_04: 0,
        },
        TesModelList {
            original_vtable: CREATURE_VTABLES[12],
            ..TesModelList::default()
        },
        TesCreature {
            blood_11c: TesModel {
                original_vtable: TES_MODEL_VTABLE,
                ..TesModel::default()
            },
            decal_134: TesTexture {
                original_vtable: TES_TEXTURE_VTABLE,
                path_04: BsStringT::default(),
            },
            ..TesCreature::default()
        },
    ));
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_CREATURE;
    reset_creature(world, form)?;
    construct_ai_bytes(world, form, 0x46)?;
    world
        .get_mut::<TesAttributes>(form)
        .expect("just built")
        .values_04[6] = 0x32;
    set_change_flags(world, form, 0x8)?;
    world
        .get_mut::<TesActorBaseData>(form)
        .expect("just built")
        .flags_04 |= 0x4000;
    set_change_flags(world, form, 0x10)?;
    if world.resource::<GameThreadLocals>().byte_184 == 0 {
        return Err(GameError::unclassified(
            0x0051ECDA,
            "0051ECDA: a TESCreature outside a load joins the 00B36298 faction",
        ));
    }
    Ok(form)
}

// IDA: the hair and eyes arms of 0052D800, one each: a size that isn't a multiple of 4, or 0, does nothing; FormHeap, 00450C20(size); each ID through 0046BB20, 004471A0 and the cast; 0052B550 / 0052B610 append it unless a listed form has the same ID, the walk ending at a null item; a null warns. Completion: 100%.
fn load_race_form_list(
    world: &World,
    list: &mut Vec<Entity>,
    form: Entity,
    form_type: u8,
    what: &str,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let size = cx.file.chunk_size()?;
    if size & 3 != 0 || size >> 2 == 0 {
        return Ok(());
    }
    let mut buffer = vec![0_u8; size as usize];
    cx.file.read_chunk(&mut buffer, size)?;
    for word in buffer.as_chunks::<4>().0 {
        let raw = u32::from_le_bytes(*word);
        let id = map_file_form_id(world, cx, raw, Some(cx.file.id))?;
        let found = (id != 0)
            .then(|| world.resource::<FormsById>().lookup(id))
            .flatten()
            .filter(|&entry| {
                world
                    .get::<FormIdentity>(entry)
                    .expect("a form")
                    .form_type_04
                    == form_type
            });
        if let Some(found) = found {
            let found_id = world.get::<FormIdentity>(found).expect("a form").form_id_0c;
            let present = list.iter().any(|&entry| {
                world.get::<FormIdentity>(entry).expect("a form").form_id_0c == found_id
            });
            if !present {
                list.push(found);
            }
        } else {
            let name = latin1(&editor_id_of(world, form));
            let own = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
            cx.file.services.warn(format!(
                "Could not find {what} ({id:08X}) for race '{name}' ({own:08X})."
            ));
        }
    }
    Ok(())
}

// IDA: 0052D800 TESRace vt+0x1C LoadForm. The record must be RACE (else 0); 00451210; the locals [ebp-0x1C] sex = 0, [ebp-0x18] index = 0, [ebp-5] face = 0; the chunks, by the compares and the table 0052DEAC over 0052DE80. Returns 1. Completion: 100%.
fn load_race_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? != u32::from(FORM_TYPE_RACE) {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    let mut sex = 0_usize;
    let mut index = 0_u32;
    let mut face = false;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            // Alloca, size 0x200; vt+0xD8 006FDFF0 keeps nothing.
            EDID => {
                cx.file.read_text(0x200)?;
            }
            FULL => load_full_name(
                &mut world
                    .get_mut::<TesFullName>(form)
                    .expect("the class's component")
                    .name_04,
                cx,
            )?,
            DESC => load_description(
                &mut world
                    .get_mut::<TesDescription>(form)
                    .expect("the class's component"),
                cx,
            )?,
            // 0046BDA0(+0x50, 0x24): no tail, as TESRace's attributes aren't bases.
            DATA => {
                let mut race = world
                    .get_mut::<TesRace>(form)
                    .expect("the class's component");
                load_data_chunk(&mut race.data_50, 0x24, DataChunkTails::default(), cx)?;
            }
            // 0052D955: 00450C20(+0x9C, 1).
            CNAM => {
                let mut race = world
                    .get_mut::<TesRace>(form)
                    .expect("the class's component");
                let mut byte = [race.hair_color_9c];
                cx.file.read_chunk(&mut byte, 1)?;
                race.hair_color_9c = byte[0];
            }
            NAM0 => face = true,
            NAM1 => face = false,
            MNAM => sex = 0,
            FNAM => sex = 1,
            // 0052D999: FormHeap(8), 00450C20(8), 0067B1E0 on +0x44. The block is unset past a short chunk, and 0 here.
            XNAM => {
                let mut bytes = [0_u8; 8];
                cx.file.read_chunk(&mut bytes, 8)?;
                world
                    .get_mut::<TesReactionForm>(form)
                    .expect("the class's component")
                    .reactions_04
                    .push(TesReaction {
                        faction: FormRef::Raw(u32::from_le_bytes([
                            bytes[0], bytes[1], bytes[2], bytes[3],
                        ])),
                        modifier: i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
                    });
            }
            // 0052D9BD: size 4 clears both; size 8 reads them through locals; another does nothing.
            VNAM => {
                let size = cx.file.chunk_size()?;
                if size == 4 {
                    world
                        .get_mut::<TesRace>(form)
                        .expect("the class's component")
                        .voices_300 = [FormRef::Raw(0); 2];
                } else if size == 8 {
                    let mut bytes = [0_u8; 8];
                    cx.file.read_chunk(&mut bytes, 8)?;
                    world
                        .get_mut::<TesRace>(form)
                        .expect("the class's component")
                        .voices_300 = [
                        FormRef::Raw(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
                        FormRef::Raw(u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]])),
                    ];
                }
            }
            // 0052DA07: 004510E0 onto +0xA0.
            PNAM => {
                let mut race = world
                    .get_mut::<TesRace>(form)
                    .expect("the class's component");
                let (bits, _) = cx.file.read_dword(race.facegen_main_a0.to_bits())?;
                race.facegen_main_a0 = f32::from_bits(bits);
            }
            // 0052DA1A: 004510E0 onto +0xA4.
            UNAM => {
                let mut race = world
                    .get_mut::<TesRace>(form)
                    .expect("the class's component");
                let (bits, _) = cx.file.read_dword(race.facegen_face_a4.to_bits())?;
                race.facegen_face_a4 = f32::from_bits(bits);
            }
            // 0052DA2D: 00450C20(8) to locals, unset for a short chunk and 0 here.
            DNAM => {
                let mut bytes = [0_u8; 8];
                cx.file.read_chunk(&mut bytes, 8)?;
                world
                    .get_mut::<TesRace>(form)
                    .expect("the class's component")
                    .default_hairs_94 = [
                    FormRef::Raw(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
                    FormRef::Raw(u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]])),
                ];
            }
            // 0052DA69: only with 2 bytes or more, 004510F0 onto +0x2FC.
            SNAM => {
                if cx.file.chunk_size()? >= 2 {
                    let mut race = world
                        .get_mut::<TesRace>(form)
                        .expect("the class's component");
                    let mut bytes = race.snam_2fc.to_le_bytes();
                    cx.file.read_chunk(&mut bytes, 2)?;
                    race.snam_2fc = u16::from_le_bytes(bytes);
                }
            }
            // 0052DC26: a zeroed local, 004510E0, 0046F420 on +0x2C.
            SPLO => {
                let (id, _) = cx.file.read_dword(0)?;
                let loading = world.resource::<GameThreadLocals>().byte_184 != 0;
                world
                    .get_mut::<TesSpellList>(form)
                    .expect("the class's component")
                    .add_spell_while_loading(id, loading)?;
            }
            // 0052DC6F: an alloca of 0x10, 00450C20(0x10), 00468CA0 on +0x74 and +0x80.
            ATTR => {
                let mut bytes = [0_u8; 0x10];
                cx.file.read_chunk(&mut bytes, 0x10)?;
                let mut race = world
                    .get_mut::<TesRace>(form)
                    .expect("the class's component");
                race.attributes_74.values_04.copy_from_slice(&bytes[..8]);
                race.attributes_80.values_04.copy_from_slice(&bytes[8..]);
            }
            // 0052DA89.
            HNAM => {
                with_component(world, form, |world, race: &mut TesRace| {
                    load_race_form_list(world, &mut race.hairs_8c, form, FORM_TYPE_HAIR, "hair", cx)
                })?;
            }
            // 0052DB56.
            ENAM => {
                with_component(world, form, |world, race: &mut TesRace| {
                    load_race_form_list(world, &mut race.eyes_a8, form, FORM_TYPE_EYES, "eyes", cx)
                })?;
            }
            // 0052DC6B, 0052D88E, 0052DCA2: +0x29C + 0x18 * index.
            FGGS | FGGA | FGTS => {
                let matrix = match chunk {
                    FGGS => 0,
                    FGGA => 1,
                    _ => 2,
                };
                with_component(world, form, |_, race: &mut TesRace| {
                    load_facegen_matrix(&mut race.facegen_29c[matrix], cx)
                })?;
            }
            // 0052DE36: 004510E0 into the index local; with `face`, vt+0 of that face model and texture, with no bound test: a host stop past 8.
            INDX => {
                let (value, _) = cx.file.read_dword(index)?;
                index = value;
                if face {
                    let mut race = world
                        .get_mut::<TesRace>(form)
                        .expect("the class's component");
                    let slot = index as usize;
                    if slot >= 9 {
                        return Err(GameError::unclassified(
                            0x0052DE51,
                            "0052DE51: a face INDX past 8 overruns the parts",
                        ));
                    }
                    race.face_models_e0[slot].init();
                    race.face_textures_1b8[slot].path_04 = BsStringT::default();
                }
            }
            // 0052DD8C: 0046FF00 (ICON) and 0046D3D0 (MODB, MODL, MODT) by the open section. Each member TESModel's RTTI cross cast to TESForm gives null at 0046D940, so a missing texture warns "on UNKNOWN form" (the C++ passes the race).
            ICON => {
                let mut race = world
                    .get_mut::<TesRace>(form)
                    .expect("the class's component");
                let slot = if face {
                    index as usize
                } else {
                    5 * sex + index as usize
                };
                let texture = if face {
                    race.face_textures_1b8.get_mut(slot)
                } else {
                    race.body_textures_224.get_mut(slot)
                }
                .ok_or(GameError::unclassified(
                    0x0052DDA5,
                    "0052DDA5: an ICON index overruns the parts",
                ))?;
                load_texture(texture, cx)?;
            }
            MODB | MODL | MODT => {
                with_component(world, form, |world, race: &mut TesRace| {
                    let model = if face {
                        race.face_models_e0.get_mut(index as usize).ok_or(
                            GameError::unclassified(
                                0x0052DDCB,
                                "0052DDCB: a face model past index 8",
                            ),
                        )?
                    } else {
                        &mut race.body_models_b0[sex]
                    };
                    load_model(world, model, None, cx)
                })?;
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    Ok(true)
}

// IDA: 0052B840 TESRace vt+0x14: nine dwords at +0x50 = 0, then every second byte of the first seven pairs = 0xFF; +0x68, +0x60, +0x6C, +0x64 = 1.0; +0x300, +0xA0, +0x304, +0xA4, +0x94, +0x98 = 0, byte +0x9C = 0; 005538D0 and 005528F0 to +0x29C; 00552880: 50, 30, 50 rows of one column, the kept floats resized; +0x2FC = 0; the nine face parts: model vt+0, vt+0x18 with 00B10CCC[i]; texture vt+0, 004028D0 with 00B10CF0[i]; then for each sex five body textures (00B10D14) and the body model's vt+0; a jump to 0046C2E0. Completion: 100%.
fn reset_race(world: &mut World, form: Entity) -> Result<(), GameError> {
    let source = facegen_coefficients(world)?;
    {
        let mut race = world
            .get_mut::<TesRace>(form)
            .expect("the class's component");
        race.data_50 = [0; 0x24];
        for pair in 0..7 {
            race.data_50[2 * pair] = 0xFF;
        }
        let one = 1.0_f32.to_le_bytes();
        for offset in [0x68, 0x60, 0x6C, 0x64] {
            race.data_50[offset - 0x50..offset - 0x50 + 4].copy_from_slice(&one);
        }
        race.voices_300[0] = FormRef::Raw(0);
        race.facegen_main_a0 = 0.0;
        race.voices_300[1] = FormRef::Raw(0);
        race.facegen_face_a4 = 0.0;
        race.default_hairs_94 = [FormRef::Raw(0); 2];
        race.hair_color_9c = 0;
        copy_facegen(&source, &mut race.facegen_29c);
        for (matrix, rows) in race.facegen_29c.iter_mut().zip([0x32, 0x1E, 0x32]) {
            matrix.rows = rows;
            matrix.cols = 1;
            resize_matrix_data(matrix, rows);
        }
        race.snam_2fc = 0;
        for part in 0..9 {
            race.face_models_e0[part].init();
            race.face_models_e0[part]
                .path_04
                .set(Some(FACE_MODELS[part]), 0);
            race.face_textures_1b8[part].path_04 = BsStringT::default();
            race.face_textures_1b8[part]
                .path_04
                .set(FACE_TEXTURES[part], 0);
        }
        for sex in 0..2 {
            for part in 0..5 {
                let slot = 5 * sex + part;
                race.body_textures_224[slot].path_04 = BsStringT::default();
                race.body_textures_224[slot]
                    .path_04
                    .set(BODY_TEXTURES[slot], 0);
            }
            race.body_models_b0[sex].init();
        }
    }
    reset_14_components(world, form)
}

// IDA: 0052B990 TESRace vt+0x18: 0052B5A0 and 0052B660 free the hair and eye nodes; each face model's vt+4, with the texture's nullsub_2; for each sex, the textures' nullsub_2 and the body model's vt+4; a jump to 0046C2F0. Completion: 100%.
fn destroy_race(world: &mut World, form: Entity) -> Result<(), GameError> {
    let mut race = world
        .get_mut::<TesRace>(form)
        .expect("the class's component");
    race.hairs_8c.clear();
    race.eyes_a8.clear();
    for model in &mut race.face_models_e0 {
        model.destroy();
    }
    for model in &mut race.body_models_b0 {
        model.destroy();
    }
    reset_18_components(world, form)
}

// IDA: 0052D100 TESRace_Constructor: 0046C460; +0x18 TESFullName inline; 0046A400 on +0x24; 0046F2C0 on +0x2C; 0046E5E0 on +0x40; the five vtables; 00468BB0 on +0x74 and +0x80; +0x8C, +0x90, +0xA8, +0xAC = 0; the arrays by `eh vector constructor` 00984BCF (0046D7E0 x 2, x 9; 0046FFD0 x 9, x 10; 0043EB30 x 4); the undo array; +0x04 = 9; 0052B840. Completion: 100% of the store order.
pub(crate) fn construct_race(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_tes_form(world)?;
    let mut description = TesDescription {
        original_vtable: TES_DESCRIPTION_VTABLE,
        offset_04: 0,
    };
    reset_description(
        &mut description,
        &mut world.resource_mut::<DataHandlerGlobals>(),
    );
    description.original_vtable = RACE_VTABLES[2];
    let model = TesModel {
        original_vtable: TES_MODEL_VTABLE,
        ..TesModel::default()
    };
    let texture = TesTexture {
        original_vtable: TES_TEXTURE_VTABLE,
        path_04: BsStringT::default(),
    };
    world.entity_mut(form).insert((
        OriginalVtable(RACE_VTABLES[0]),
        TesFullName {
            original_vtable: RACE_VTABLES[1],
            name_04: BsStringT::default(),
        },
        description,
        TesSpellList {
            original_vtable: RACE_VTABLES[3],
            ..TesSpellList::default()
        },
        TesReactionForm {
            original_vtable: RACE_VTABLES[4],
            reactions_04: Vec::new(),
            group_type_0c: 6,
        },
        TesRace {
            attributes_74: TesAttributes::new(TES_ATTRIBUTES_VTABLE),
            attributes_80: TesAttributes::new(TES_ATTRIBUTES_VTABLE),
            body_models_b0: std::array::from_fn(|_| model.clone()),
            face_models_e0: std::array::from_fn(|_| model.clone()),
            face_textures_1b8: std::array::from_fn(|_| texture.clone()),
            body_textures_224: std::array::from_fn(|_| texture.clone()),
            undo_308: construct_undo_array(),
            ..TesRace::default()
        },
    ));
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_RACE;
    reset_race(world, form)?;
    Ok(form)
}

/// IDA: this family's classes, each with its InitItem still to come.
/// TODO: [ ] 12d: 0052BA30 (RACE), 00523F40 (NPC_) and 0051CF10 (CREA), read by the vt+0x6C passes.
pub const CLASSES: &[FormClass] = &[
    FormClass {
        form_type: FORM_TYPE_RACE,
        vtable: RACE_VTABLES[0],
        construct: construct_race,
        load_form: load_race_form,
        reset_14: reset_race,
        reset_18: destroy_race,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_race,
        slots: &[
            ComponentSlot::Description,
            ComponentSlot::FullName,
            ComponentSlot::SpellList,
            ComponentSlot::Reaction,
        ],
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_NPC,
        vtable: NPC_VTABLES[0],
        construct: construct_npc,
        load_form: load_npc_form,
        reset_14: reset_npc,
        reset_18: destroy_npc,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_npc,
        slots: &[
            ComponentSlot::AiForm,
            ComponentSlot::Animation,
            ComponentSlot::ActorBaseData,
            ComponentSlot::Attributes,
            ComponentSlot::Container,
            ComponentSlot::FullName,
            ComponentSlot::Health,
            ComponentSlot::Model,
            ComponentSlot::RaceForm,
            ComponentSlot::Scriptable,
            ComponentSlot::SpellList,
        ],
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_CREATURE,
        vtable: CREATURE_VTABLES[0],
        construct: construct_creature,
        load_form: load_creature_form,
        reset_14: reset_creature,
        reset_18: destroy_creature,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_creature,
        slots: &[
            ComponentSlot::AiForm,
            ComponentSlot::Animation,
            ComponentSlot::ActorBaseData,
            ComponentSlot::AttackDamage,
            ComponentSlot::Attributes,
            ComponentSlot::Container,
            ComponentSlot::FullName,
            ComponentSlot::Health,
            ComponentSlot::Model,
            ComponentSlot::Scriptable,
            ComponentSlot::SpellList,
            ComponentSlot::ModelList,
        ],
        special_arm: None,
    },
];

/// IDA: +0xE8, TESRaceForm's +0x04 in a TESNPC. Completion: 100%.
/// # Errors
/// A base that isn't an NPC, an arm no player reaches.
pub fn base_race(world: &World, base: Entity) -> Result<Option<Entity>, GameError> {
    if world
        .get::<FormIdentity>(base)
        .expect("a form")
        .form_type_04
        != 0x23
    {
        return Err(GameError::unclassified(
            0x004D7260,
            "004D7260: a creature base's scale",
        ));
    }
    Ok(
        match world.get::<TesRaceForm>(base).map(|part| part.race_04) {
            Some(FormRef::Resolved(race)) => race,
            _ => None,
        },
    )
}

/// IDA: 00519D20 TESActorBase_GetNPCSexOrMinusOne: an NPC's flags bit 0, and -1 for anything else. Completion: 100%.
#[must_use]
pub fn npc_sex(world: &World, base: Entity) -> Option<usize> {
    (world
        .get::<FormIdentity>(base)
        .expect("a form")
        .form_type_04
        == 0x23)
        .then(|| {
            (world
                .get::<TesActorBaseData>(base)
                .expect("an NPC has its data")
                .flags_04
                & 1) as usize
        })
}

/// IDA: a float of TESRace's +0x50 block, by its offset from the object's start. Completion: 100%.
#[must_use]
pub fn race_scale(world: &World, race: Entity, offset: usize) -> f32 {
    let data = &world.get::<TesRace>(race).expect("a race").data_50;
    f32::from_le_bytes(
        data[offset - 0x50..offset - 0x50 + 4]
            .try_into()
            .expect("four bytes"),
    )
}

/// IDA: 0052D2C0 TESRace_GetBodyTexture(race, out texture, out FaceGen texture, npc, slot): the part is 4 for the tail (15),
/// `slot - 2` for 2..6, else none (answers 0); the sex is the NPC's (`00519D20`), 0 with none; the race's body texture
/// +0x224 + 0x0C (5 sex + part) with an empty path answers 0; otherwise "Textures\%s" of it through TES_CreateTextureImage
/// (`00442890`, quiet 0, which makes a texture even for a file that isn't found) and, with bFaceGenTexturing, the FaceGen
/// texture (`SkinTextureDeferrals`). Answers the base texture's path. Completion: 100% of the base texture; the FaceGen
/// texture is a host deferral counted as built (D13).
// TODO: [ ] the FaceGen body texture (00557E60) and its palette entry.
pub(crate) fn race_body_texture(
    world: &mut World,
    race: Entity,
    npc: Option<Entity>,
    slot: usize,
    geometry: Entity,
) -> Option<Vec<u8>> {
    let part = if slot == 15 { 4 } else { slot.wrapping_sub(2) };
    if part > 4 {
        return None;
    }
    let sex = npc.and_then(|npc| npc_sex(world, npc)).unwrap_or(0);
    let texture = &world.get::<TesRace>(race)?.body_textures_224[5 * sex + part];
    if crate::tes::form_components::path_length(&texture.path_04) == 0 {
        return None;
    }
    let mut path = b"Textures\\".to_vec();
    path.extend_from_slice(&crate::tes::face_gen_head::path_text(&texture.path_04));
    let face_texturing = world
        .get_resource_or_init::<crate::tes::face_gen_tasks::FaceGenHeadSettings>()
        .face_texturing_b10d3c;
    if face_texturing {
        // 0052D5BA: 00B3630C clear leaves no static model, and 0052D2C0 answers 0 (LABEL_40).
        let statics = world.get_resource_or_init::<crate::tes::forms_init::RaceStatics>();
        if !statics.set_b3630c {
            return None;
        }
        let (format, index) = match (slot, sex) {
            (2, 1) => ("ModUpperBodyFemale", 11),
            (2, _) => ("ModUpperBodyMale", 10),
            _ => ("ModBody", 12),
        };
        let model = statics
            .model_paths
            .get(index)
            .map(crate::tes::face_gen_head::path_text)
            .unwrap_or_default();
        let form_id = npc.map_or(0, |npc| {
            world
                .get::<crate::tes::form::FormIdentity>(npc)
                .map_or(0, |identity| identity.form_id_0c)
        });
        let palette_name = format!("{form_id:08X}{format}").into_bytes();
        // 005500C0: the text up to its last '.', then ".egt"; with no '.' there is no model, and 0052D2C0 answers 0.
        let mut meshes = b"Meshes\\".to_vec();
        meshes.extend_from_slice(&model);
        let dot = meshes.iter().rposition(|&byte| byte == b'.')?;
        let mut egt_path = meshes[..dot].to_vec();
        egt_path.extend_from_slice(b".egt");
        world
            .get_resource_or_init::<SkinTextureDeferrals>()
            .parts
            .push(SkinTextureRequest {
                geometry,
                slot,
                palette_name,
                egt_path,
            });
    }
    Some(path)
}

/// IDA: 005227A0(base, actor, allocation, weapon, 0, keep): TLS +0x185 saved, and cleared unless `keep` (the last argument), so the equips below record no change flags (0045B670's gate); restored at the end. vt+0x190 is 1 on a Character. A LowProcess answers vt+0x138 (0069D990) 0 and vt+0x10C, 0x110, 0x108 and 0x104 do nothing; a MiddleHigh or High process's vt+0x138 (006293C0) is the bow byte +0xF4, whose set arm (004DC5B0) is a stop, and the four store null over +0xEC, +0xF0, +0xE8 (0064AED0, 0064AF10, 0064AE90) and +0xE4 (00658550: +0xFC..+0x110 zeroed with +0xE4 null), each held selection's free a stop. 00486030(1) unequips stacks with 0x1B or 0x1C (0041DF10(0)), a stop; others stay. The armor searches 0048C870 (slots 2, 3, 5, 4, 1, 0, 13, 6, 7, 8), the weapon's 0048BDA0 and the torch's 0048B660 only read without an ARMO, a WEAP or a LIGH in the base container or the changes. 0048D110 picks the clothing per slot. Then in order: slots 8, 7, 6; 2; 3 unless 2's item covers 3; 4, 5, 1 and 0 unless an item equipped above covers them; 13. Each is vt+0x108 005F3140(form, 1, null, 0), after 004D8880, which is passed the selection, not its form, here, so it never finds the item worn. With a process, 005EDA20(1) last: each of 00B1489C's ten slots' worn item (00486790) with an enchantment (005E3DE0) is applied, which `equip_item`'s stop on an enchantment and the worn-stack stop above leave none of. Completion: 100% of those arms.
/// # Errors
/// An arm not ported, by its site.
pub(crate) fn auto_equip(
    world: &mut World,
    actor: Entity,
    allocation: bool,
    weapon: bool,
    keep_tls_185: bool,
) -> Result<(), GameError> {
    let saved = world.resource::<GameThreadLocals>().track_changes_185;
    if !keep_tls_185 {
        world.resource_mut::<GameThreadLocals>().track_changes_185 = 0;
    }
    let result = auto_equip_body(world, actor, allocation, weapon);
    world.resource_mut::<GameThreadLocals>().track_changes_185 = saved;
    result
}

// IDA: N/A 005227A0 between its TLS +0x185 save and restore.
fn auto_equip_body(
    world: &mut World,
    actor: Entity,
    allocation: bool,
    weapon: bool,
) -> Result<(), GameError> {
    let process = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58;
    if let Some(process) = process
        && let Some(middle) = world.get::<MiddleHighProcess>(process)
    {
        if middle.bow_f4 != 0 {
            return Err(GameError::unclassified(
                0x005227F8,
                "005227F8: 004DC5B0 for a bow (+0xF4)",
            ));
        }
        if middle.weapon_e4.is_some()
            || middle.light_e8.is_some()
            || middle.ammo_ec.is_some()
            || middle.shield_f0.is_some()
        {
            return Err(GameError::unclassified(
                0x0064AED0,
                "0064AED0: freeing a process's held selection",
            ));
        }
    } else if let Some(process) = process {
        low_process_only(world, process, "005227F8: a process's vt+0x138")?;
    }
    let changes = ensure_container_changes(world, actor)?;
    if let Some(process) = process
        && let Some(mut middle) = world.get_mut::<MiddleHighProcess>(process)
    {
        // 0052281F..0052284F: vt+0x10C, 0x110, 0x108 and 0x104 with null; 00658550 with +0xE4 null zeroes +0xFC..+0x110.
        middle.weapon_nodes_fc = crate::tes::actors::actor_process::WeaponNodes::default();
    }
    // 00522820 00486030(1).
    let mut changes = changes;
    unequip_worn_stacks(&mut changes, true);
    // 004867DD: a leveled stack in an entry of negative count is 004853B0's, which 00486790 runs first.
    let negative = changes.entries_00.iter().any(|entry| {
        entry.count_04 < 0
            && entry
                .lists_00
                .iter()
                .flatten()
                .any(|list| list.get(EXTRA_LEVELED_ITEM).is_some())
    });
    let mut forms: Vec<Entity> = changes
        .entries_00
        .iter()
        .map(|entry| entry.form_08)
        .collect();
    put_container_changes(world, actor, changes);
    if negative {
        return Err(GameError::unclassified(
            0x004867E8,
            "004867E8: 004853B0 on a removed leveled entry",
        ));
    }
    forms.extend(base_entries(world, actor).into_iter().map(|(_, form)| form));
    let holds = |kind: u8| forms.iter().any(|&form| form_type(world, form) == kind);
    let (holds_weapon, holds_light) = (holds(TYPE_WEAPON), holds(TYPE_LIGHT));
    // 00522888..00522935: with the allocation flag, 0048C870 for slots 2, 3, 5, 4, 1, 0, 13, 6, 7, 8 (a5 is 0 here).
    let base = reference_container(world, actor).ok_or(GameError::unclassified(
        0x005227A0,
        "005227A0: an actor with no base",
    ))?;
    let mut armor: [Option<Entity>; 16] = [None; 16];
    if allocation {
        let changes = ensure_container_changes(world, actor)?;
        let mut found = Ok(());
        for slot in [2, 3, 5, 4, 1, 0, 13, 6, 7, 8] {
            match best_armor(world, &changes, base, actor, slot, true) {
                Ok(pick) => armor[slot as usize] = pick.map(|(form, _)| form),
                Err(error) => {
                    found = Err(error);
                    break;
                }
            }
        }
        put_container_changes(world, actor, changes);
        found?;
    }
    // 00522939: with the weapon flag, 0048BDA0 (its rating out isn't used).
    let _ = holds_weapon;
    let mut weapon_pick = None;
    if weapon {
        let changes = ensure_container_changes(world, actor)?;
        let found = best_weapon(world, &changes, base, actor, true);
        put_container_changes(world, actor, changes);
        weapon_pick = found?.map(|(form, _)| form);
        if weapon_pick.is_some_and(|form| weapon_type(world, form) == 5) {
            return Err(GameError::unclassified(
                0x00522988,
                "00522988: a bow's arrows (0048B9C0)",
            ));
        }
    }
    if holds_light {
        return Err(GameError::unclassified(
            0x0048B660,
            "0048B660: a light in the base container",
        ));
    }
    // 00522A08..00522B70: 0048D110 for each slot with no armor.
    let changes = ensure_container_changes(world, actor)?;
    let mut picks: [Option<Entity>; 16] = armor;
    let mut found = Ok(());
    for slot in [2, 3, 5, 4, 1, 0, 13, 8, 6, 7] {
        if picks[slot as usize].is_none() {
            match best_clothing(world, &changes, base, actor, slot) {
                Ok(pick) => picks[slot as usize] = pick,
                Err(error) => {
                    found = Err(error);
                    break;
                }
            }
        }
    }
    put_container_changes(world, actor, changes);
    found?;
    let pick = |slot: u32| picks[slot as usize];
    let slots = [
        pick(2),
        pick(3),
        pick(5),
        pick(4),
        pick(1),
        pick(0),
        pick(13),
        pick(8),
        pick(6),
        pick(7),
    ];
    let [
        upper,
        lower_3,
        lower_5,
        slot_4,
        slot_1,
        slot_0,
        slot_13,
        slot_8,
        slot_6,
        slot_7,
    ] = slots;
    for item in [slot_8, slot_7, slot_6].into_iter().flatten() {
        equip_item(world, actor, item)?;
    }
    let mut worn: Vec<u16> = Vec::new();
    let covered = |worn: &[u16], slot: u32| worn.iter().any(|&mask| covers(mask, slot));
    let equip_unless = |world: &mut World, item: Option<Entity>, worn: &mut Vec<u16>, slot: u32| {
        let Some(item) = item else {
            return Ok(());
        };
        if covered(worn, slot) {
            return Ok(());
        }
        equip_item(world, actor, item)?;
        if let Some(mask) = biped_mask(world, item) {
            worn.push(mask);
        }
        Ok::<(), GameError>(())
    };
    // 00522BF9..00522CC2: slot 2, and slot 3 only when 2's item is absent or doesn't cover 3.
    equip_unless(world, upper, &mut worn, 2)?;
    equip_unless(world, lower_3, &mut worn, 3)?;
    for (item, slot) in [(slot_4, 4), (lower_5, 5), (slot_1, 1), (slot_0, 0)] {
        equip_unless(world, item, &mut worn, slot)?;
    }
    if let Some(item) = slot_13 {
        equip_item(world, actor, item)?;
    }
    // 00522D6A: the weapon, with its selection's count (00405110), after 004D8880.
    if let Some(item) = weapon_pick {
        equip_item(world, actor, item)?;
    }
    Ok(())
}

// IDA: 0051D4C0 TESActorBase_CanEquipForm(base, form): 00519D00 TESActorBase_CanUseWeaponAndShield (an NPC, or a creature with +0x28 flag 4) and then an ARMO over slot 13, a LIGH, a WEAP
// or an AMMO.
pub(crate) fn creature_can_equip(world: &World, base: Entity, form: Entity) -> bool {
    let allowed = form_type(world, base) != TYPE_CREATURE
        || world
            .get::<crate::tes::form_components::TesActorBaseData>(base)
            .is_some_and(|data| data.flags_04 & 4 != 0);
    allowed
        && match form_type(world, form) {
            TYPE_ARMOR => biped_mask(world, form).is_some_and(|mask| covers(mask, 13)),
            TYPE_LIGHT | TYPE_WEAPON | 0x22 => true,
            _ => false,
        }
}

/// IDA: 0051E240 TESCreature_EquipBestItems(base, actor, allocation, weapon, keep): TLS +0x185 saved and cleared unless `keep`, restored
/// at the end. Without the weapon flag, a process's vt+0x138 (the bow byte; a LowProcess's 0069D990 answers 0) reaches 004DC5B0,
/// a stop. 00486030(1) unequips the worn stacks (a stop, as 005227A0's), the process's vt+0x10C, 0x110, 0x108 and 0x104 store
/// null. With the allocation flag 0048C870 picks the slot-13 armor, with the weapon flag 0048BDA0 the weapon and, for a bow,
/// 0048B9C0 the arrows, each with the worn checks on. The shield and then the weapon are equipped through vt+0x108 (005F3140,
/// `equip_item`) when 0051D4C0 allows the form (a creature without +0x28 flag 4, weapon and shield, takes neither; an ARMO
/// only over slot 13) and 004D8880 finds no worn stack of it; the arrows' equip with the selection's count is a stop. Then a
/// process's vt+0x124(0) (the sheath node +0x108; a LowProcess's 004943A0 answers 0) with Creature vt+0x27C (0) takes
/// vt+0x300(1) (+0x114) and, dead, vt+0x308(1) (+0x115), both nothing on a LowProcess. Last, with a process, 005EDA20(1): an equipped enchanted item is a
/// stop. Completion: 100% of those arms.
/// # Errors
/// An arm not ported, by its site.
pub(crate) fn creature_auto_equip(
    world: &mut World,
    actor: Entity,
    allocation: bool,
    weapon: bool,
    keep_tls_185: bool,
) -> Result<(), GameError> {
    let saved = world.resource::<GameThreadLocals>().track_changes_185;
    if !keep_tls_185 {
        world.resource_mut::<GameThreadLocals>().track_changes_185 = 0;
    }
    let result = creature_auto_equip_body(world, actor, allocation, weapon);
    world.resource_mut::<GameThreadLocals>().track_changes_185 = saved;
    result
}

// IDA: N/A 0051E240 between its TLS +0x185 save and restore.
fn creature_auto_equip_body(
    world: &mut World,
    actor: Entity,
    allocation: bool,
    weapon: bool,
) -> Result<(), GameError> {
    let process = world
        .get::<MobileObject>(actor)
        .expect("an actor")
        .process_58;
    if let Some(process) = process {
        if let Some(middle) = world.get::<MiddleHighProcess>(process) {
            if !weapon && middle.bow_f4 != 0 {
                return Err(GameError::unclassified(
                    0x0051E297,
                    "0051E297: 004DC5B0 for a bow (+0xF4)",
                ));
            }
            if middle.weapon_e4.is_some()
                || middle.light_e8.is_some()
                || middle.ammo_ec.is_some()
                || middle.shield_f0.is_some()
            {
                return Err(GameError::unclassified(
                    0x0064AED0,
                    "0064AED0: freeing a process's held selection",
                ));
            }
        } else {
            low_process_only(world, process, "0051E2D6: a process's vt+0x10C..0x104")?;
        }
    }
    // 0051E2C1 00486030(1).
    let mut changes = ensure_container_changes(world, actor)?;
    unequip_worn_stacks(&mut changes, true);
    put_container_changes(world, actor, changes);
    if let Some(process) = process
        && let Some(mut middle) = world.get_mut::<MiddleHighProcess>(process)
    {
        middle.weapon_nodes_fc = crate::tes::actors::actor_process::WeaponNodes::default();
    }
    let base = reference_container(world, actor).ok_or(GameError::unclassified(
        0x0051E240,
        "0051E240: an actor with no base",
    ))?;
    let mut shield = None;
    if allocation {
        let changes = ensure_container_changes(world, actor)?;
        let found = best_armor(world, &changes, base, actor, 13, true);
        put_container_changes(world, actor, changes);
        shield = found?.map(|(form, _)| form);
    }
    let mut weapon_pick = None;
    let mut ammo = None;
    if weapon {
        let changes = ensure_container_changes(world, actor)?;
        let found = best_weapon(world, &changes, base, actor, true);
        let found = found.and_then(|pick| {
            let pick = pick.map(|(form, _)| form);
            let arrows = match pick {
                Some(form) if weapon_type(world, form) == 5 => {
                    best_ammo(world, &changes, actor, true)?
                }
                _ => None,
            };
            Ok((pick, arrows))
        });
        put_container_changes(world, actor, changes);
        (weapon_pick, ammo) = found?;
    }
    if let Some(form) = shield
        && creature_can_equip(world, base, form)
        && !form_worn(world, actor, form)?
    {
        equip_item(world, actor, form)?;
    }
    if let Some(form) = weapon_pick
        && creature_can_equip(world, base, form)
        && !form_worn(world, actor, form)?
    {
        equip_item(world, actor, form)?;
        if let Some(arrows) = ammo
            && !form_worn(world, actor, arrows)?
        {
            return Err(GameError::unclassified(
                0x0051E3F5,
                "0051E3F5: the arrows' equip with the selection's count",
            ));
        }
        // 0051E41E: vt+0x124(0) (MiddleHigh 006508D0: +0x108, the sheath node; a LowProcess's 004943A0: 0) and Creature vt+0x27C
        // (0); with neither, vt+0x300(1) (0064AD20: +0x114) and, dead, vt+0x308(1) (0064AD40: +0x115). A LowProcess's two do
        // nothing.
        if let Some(process) = process {
            let dead = actor_is_dead(world, actor, false);
            if let Some(mut middle) = world.get_mut::<MiddleHighProcess>(process) {
                if middle.weapon_nodes_fc.sheath_108.is_none() {
                    middle.weapon_draw_requested_114 = 1;
                    if dead {
                        middle.weapon_drawn_115 = 1;
                    }
                }
            } else {
                low_process_only(
                    world,
                    process,
                    "0051E41E: a process's vt+0x124 and vt+0x300",
                )?;
            }
        }
    }
    if process.is_some() {
        let changes = ensure_container_changes(world, actor)?;
        let enchanted = changes.entries_00.iter().any(|entry| {
            enchantment_of(world, entry.form_08).is_some()
                && entry
                    .lists_00
                    .iter()
                    .flatten()
                    .any(|list| crate::tes::base_extra_list::worn(list, false))
        });
        put_container_changes(world, actor, changes);
        if enchanted {
            return Err(GameError::unclassified(
                0x005EDA9B,
                "005EDA9B: an equipped item's enchantment (005E3DE0)",
            ));
        }
    }
    Ok(())
}

// IDA: 005232D0 TESNPC vt+0x128 and 00519D90 under it. 5232D7 skills (av - 12 <= 0x14) go to +0xEC through 00565C50(2, av), `movsx`, `movzx` the byte; 5232FE 37..=39 answer 0; else 519D90 on the table at 00519F24: 0..=7 00468AD0's byte of the attribute at +0x88 + 4 + 00565C50(0, av) (av itself), `movzx`; 8 00519C10 (+0x80 vt+0x10: A53CFC's slot 004B2460, +0x04); 9 +0x24 vt+0x48 and 10 +0x24 vt+0x4C (`movzx ax`). Completion: 100% of the arms the checks reach; the others stop.
// 37..=39 answer 0, and 33 and up go to 00519D90 (005232FB).
// TODO: [ ] 11 (004698B0), when a caller reads it.
pub(crate) fn npc_base_actor_value(
    world: &World,
    npc: Entity,
    actor_value: u32,
) -> Result<i32, GameError> {
    match actor_value {
        0..=7 => Ok(i32::from(
            world
                .get::<TesAttributes>(npc)
                .ok_or(GameError::unclassified(
                    0x00519DAE,
                    "00519DAE: a base with no attributes",
                ))?
                .values_04[actor_value as usize],
        )),
        8 => Ok(world
            .get::<TesHealthForm>(npc)
            .ok_or(GameError::unclassified(
                0x00519C10,
                "00519C10: a base with no health",
            ))?
            .health_04 as i32),
        9 => Ok(i32::from(
            world
                .get::<TesActorBaseData>(npc)
                .ok_or(GameError::unclassified(
                    0x00519E69,
                    "00519E69: a base with no base data",
                ))?
                .magicka_08,
        )),
        10 => Ok(i32::from(
            world
                .get::<TesActorBaseData>(npc)
                .ok_or(GameError::unclassified(
                    0x00519E57,
                    "00519E57: a base with no base data",
                ))?
                .fatigue_0a,
        )),
        // 005232D7: +0xEC's byte at 00565C50(2, av) = av - 12, `movzx`.
        12..=32 => Ok(i32::from(
            world
                .get::<TesNpc>(npc)
                .ok_or(GameError::unclassified(
                    0x005232E5,
                    "005232E5: a skill of a base that isn't a TESNPC",
                ))?
                .skills_ec[(actor_value - 12) as usize],
        )),
        37..=39 => Ok(0),
        33.. => crate::tes::actor_values::base_form_actor_value(world, npc, actor_value),
        _ => Err(GameError::unclassified(
            0x005232D7,
            "005232D7: an actor value the checks don't read",
        )),
    }
}

// IDA: 00526230: the four matrices row by row, the race, the hair and eyes with the hair's length and colour, then whether the NPC is female.
pub(crate) fn write_npc_face(
    out: &mut SaveWriter,
    world: &World,
    base: Entity,
) -> Result<(), GameError> {
    let npc = world.get::<TesNpc>(base).expect("the player's base");
    for matrix in &npc.facegen_108 {
        for row in 0..matrix.rows {
            for col in 0..matrix.cols {
                out.f32(matrix.data[(col + row * matrix.cols) as usize]);
            }
        }
    }
    let race = world.get::<TesRaceForm>(base).expect("an NPC's race");
    out.u32(match race.race_04 {
        FormRef::Raw(id) => id,
        FormRef::Resolved(form) => form_id(world, form),
    });
    out.u32(form_id(world, npc.hair_1c8));
    out.u32(form_id(world, npc.eyes_1d0));
    out.f32(npc.hair_length_1cc);
    out.u32(npc.hair_color_1e8);
    out.u8(u8::from(npc_sex(world, base) == Some(1)));
    Ok(())
}

// IDA: 00521A10 TESNPC_GetFaceGenCoefficientsForVampirism: +0x168 with vampirism (vt+0x128(69)), else +0x108.
pub(crate) fn npc_own_coefficients(
    world: &World,
    npc: Entity,
) -> Result<[FaceGenMatrix; 4], GameError> {
    let vampire = crate::tes::actor_values::base_actor_value(world, npc, 69)? != 0;
    let data = world.get::<TesNpc>(npc).expect("an NPC");
    Ok(if vampire {
        data.facegen_168.clone()
    } else {
        data.facegen_108.clone()
    })
}

/// IDA: 00521E40 TESNPC_ResetFaceGenCoefficients(npc): the NPC's own coefficients (+0x168 with vampirism, vt+0x128(69), else
/// +0x108) take g_faceGen's defaults (`005538D0`, `005528F0`), then `00552880` reshapes them (50, 30, 50 rows, kept values);
/// the cached head data (+0x1DC) is released and cleared. Completion: 100%; the release has no host form.
/// # Errors
/// A null g_faceGen, or the vampirism value's read.
pub(crate) fn npc_reset_face(world: &mut World, npc: Entity) -> Result<(), GameError> {
    let defaults = crate::tes::face_gen_head::facegen_coefficients(world)?;
    let vampire = crate::tes::actor_values::base_actor_value(world, npc, 69)? != 0;
    let mut data = world.get_mut::<TesNpc>(npc).expect("an NPC");
    let target = if vampire {
        &mut data.facegen_168
    } else {
        &mut data.facegen_108
    };
    crate::tes::face_gen_head::copy_facegen(&defaults, target);
    crate::tes::face_gen_head::shape_coefficients(target);
    data.face_nodes_1d4[2] = 0;
    Ok(())
}

/// IDA: 00526F00 TESNPC_UndoFaceGen(npc): with entries in the undo array (+0x1F0, its size word +0x1FA), the last entry's two
/// sets go back into the race's +0x29C and the NPC's own coefficients (+0x168 with vampirism, else +0x108), the entry is
/// removed (00431F50), destroyed (00526E70) and freed, and the array compacted (00521BE0). Nothing in the exe pushes an entry
/// (the array's only writers are the TESNPC and TESRace ctors and dtors), so the size is always the ctor's 0 and it returns.
/// Completion: 100% of the reachable arm; a non-empty array is a stop, the port having no entries.
/// # Errors
/// A non-empty undo array.
pub(crate) fn npc_undo_face(world: &World, npc: Entity) -> Result<(), GameError> {
    let size = world.get::<TesNpc>(npc).expect("an NPC").undo_1f0.size_0a;
    if size != 0 {
        return Err(GameError::unclassified(
            0x00526F0C,
            "00526F0C: a FaceGen undo entry, which nothing in the exe pushes",
        ));
    }
    Ok(())
}

// IDA: N/A 0047DF80 Random_Next(0) through the host's main-thread CRT (the scripts' seam).
fn main_thread_random_next(world: &mut World) -> Result<i32, GameError> {
    let draw = world
        .get_non_send::<crate::tes::form_arms::ScriptRandom>()
        .map(|random| random.0.clone())
        .ok_or(GameError::unclassified(0x0047DF80, "0047DF80: no host CRT"))?;
    Ok(draw(world))
}

/// IDA: 005273A0 TESNPC_Randomize(npc, keep_age, keep_gender, keep_hair_length) (`retn 0Ch`): 00552880 shapes a
/// difference set; 00553B70(fNPCGeneticVariation 00B362D4, race +0x29C, generated). With A and B the generated set's age
/// controls (00553B30(0, 0) and (0, 1)), the age Y is A when kept, else Random_Next / 32767.0 x 50.0 + 15.0, and
/// T = B - (A - Y); each clamped to [15, 65], then 00555A00(0, 0, Y) and (0, 1, T). With s = 2.0 for a sex that isn't 0
/// (00520FF0, -1 included), else -2.0, G = (1, 0) - s and H = (1, 1) - s; Z is G when kept, else Random_Next / 32767.0 x
/// 4.0 minus 2.0 and W = H - (G - Z) (kept, W = H); each clamped to [-2, 2], then 00555A00(1, 0, s + Z) and (1, 1, s + W). Unless
/// kept, +0x1CC (hair length) = Random_Next / 32767.0. Then 00552C10(race, generated, difference); when 00551990 finds it
/// differs from the NPC's own (+0x168 with vampirism, else +0x108), 005528F0 copies it there and +0x1DC is released.
/// All at the main thread's 24-bit precision. Completion: 100%; the release has no host form.
/// # Errors
/// A null race or g_faceGen, no host CRT, or a stop in the controls.
pub fn npc_randomize_face(
    world: &mut World,
    npc: Entity,
    keep_age: bool,
    keep_gender: bool,
    keep_hair_length: bool,
) -> Result<(), GameError> {
    use crate::tes::face_gen_head::{
        coefficients_differ, coefficients_difference, face_gen_age_gender_control,
        face_gen_randomize_coefficients, face_gen_set_age_gender_control, shape_coefficients,
    };
    let mut difference: [FaceGenMatrix; 4] = Default::default();
    shape_coefficients(&mut difference);
    let race = base_race(world, npc)?.ok_or(GameError::unclassified(
        0x005273D8,
        "005273D8: a null race faults",
    ))?;
    let race_coefficients = world
        .get::<TesRace>(race)
        .expect("a race")
        .facegen_29c
        .clone();
    let scale = world
        .get_resource::<crate::tes::game_settings::GameSettingCollection>()
        .and_then(|settings| settings.find("fNPCGeneticVariation"))
        .map_or(0.0, |setting| f32::from_bits(setting.raw));
    let mut generated: [FaceGenMatrix; 4] = Default::default();
    face_gen_randomize_coefficients(world, scale, &race_coefficients, &mut generated)?;
    let shape_age = face_gen_age_gender_control(world, &generated, 0, 0)?;
    let texture_age = face_gen_age_gender_control(world, &generated, 0, 1)?;
    let age = if keep_age {
        shape_age
    } else {
        main_thread_random_next(world)? as f32 / 32767.0 * 50.0 + 15.0
    };
    let texture = texture_age - (shape_age - age);
    let age = age.clamp(15.0, 65.0);
    let texture = texture.clamp(15.0, 65.0);
    face_gen_set_age_gender_control(world, &mut generated, 0, 0, age)?;
    face_gen_set_age_gender_control(world, &mut generated, 0, 1, texture)?;
    let side = if npc_sex(world, npc).map_or(-1, |sex| sex as i32) != 0 {
        2.0_f32
    } else {
        -2.0
    };
    let shape_gender = face_gen_age_gender_control(world, &generated, 1, 0)? - side;
    let texture_gender = face_gen_age_gender_control(world, &generated, 1, 1)? - side;
    let (gender, texture) = if keep_gender {
        (shape_gender, texture_gender)
    } else {
        let gender = main_thread_random_next(world)? as f32 / 32767.0 * 4.0 - 2.0;
        (gender, texture_gender - (shape_gender - gender))
    };
    let gender = gender.clamp(-2.0, 2.0);
    let texture = texture.clamp(-2.0, 2.0);
    face_gen_set_age_gender_control(world, &mut generated, 1, 0, side + gender)?;
    face_gen_set_age_gender_control(world, &mut generated, 1, 1, side + texture)?;
    if !keep_hair_length {
        let length = main_thread_random_next(world)? as f32 / 32767.0;
        world
            .get_mut::<TesNpc>(npc)
            .expect("an NPC")
            .hair_length_1cc = length;
    }
    coefficients_difference(&race_coefficients, &generated, &mut difference);
    let vampire = crate::tes::actor_values::base_actor_value(world, npc, 69)? != 0;
    let mut data = world.get_mut::<TesNpc>(npc).expect("an NPC");
    let own = if vampire {
        &mut data.facegen_168
    } else {
        &mut data.facegen_108
    };
    if coefficients_differ(&difference, own) {
        crate::tes::face_gen_head::copy_facegen(&difference, own);
        data.face_nodes_1d4[2] = 0;
    }
    Ok(())
}
