//! Phase 12b: the misc forms, as in the C++'s forms_misc. TESClass is the first, the stub proving the family path: construct, reset pair, LoadForm, list arm, no editor ID. Append each class to `CLASSES` as it's ported.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::data_handler::{
    DataHandlerGlobals, TES_DESCRIPTION_VTABLE, TES_TEXTURE_VTABLE, TesDescription, TesTexture,
    construct_tes_form, reset_description,
};
use super::data_load::initialize_form_from_current_record;
use super::form::{FormIdentity, OriginalVtable};
use super::form_arms::{
    FormClass, FormLoadCx, default_editor_id, no_init_item, reset_14_components,
    reset_18_components,
};
use super::form_components::{
    ComponentSlot, DataChunkTails, FormRef, TES_FULL_NAME_VTABLE, TES_MODEL_VTABLE,
    TES_REACTION_FORM_VTABLE, TES_SOUND_FILE_VTABLE, TesFullName, TesModel, TesObjectLinks,
    TesReaction, TesReactionForm, TesSoundFile, load_data_chunk, load_description, load_full_name,
    load_model, load_texture, reset_form_components,
};
use super::tes_file::BsStringT;

/// IDA: the form type of TESClass.
pub const FORM_TYPE_CLASS: u8 = 0x05;
/// IDA: 0051C5E7..0051C5FA TESClass's four vtables: the form, +0x18 TESFullName, +0x24 TESDescription, +0x2C TESTexture.
pub const TES_CLASS_VTABLE: u32 = 0x00A5_2C6C;
pub const TES_CLASS_FULL_NAME_VTABLE: u32 = 0x00A5_2C54;
pub const TES_CLASS_DESCRIPTION_VTABLE: u32 = 0x00A5_2C3C;
pub const TES_CLASS_TEXTURE_VTABLE: u32 = 0x00A5_2C20;

// IDA: the chunks 0051C250 dispatches on.
const DATA: u32 = 0x4154_4144;
const DESC: u32 = 0x4353_4544;
const EDID: u32 = 0x4449_4445;
const FULL: u32 = 0x4C4C_5546;
const ICON: u32 = 0x4E4F_4349;

/// IDA: 0051C580 TESClass (0x6C): TESFullName +0x18, TESDescription +0x24, TESTexture +0x2C (`TesFullName`, `TesDescription`, `TesTexture` on the same entity), and the 0x34 bytes of DATA at +0x38.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesClass {
    pub attributes_38: [u32; 2],
    pub specialization_40: u32,
    pub major_skills_44: [u32; 7],
    pub flags_60: u32,
    pub services_64: u32,
    pub trainer_skill_68: u8,
    pub trainer_level_69: u8,
    pub unused_6a: u16,
}

impl TesClass {
    /// IDA: N/A the 0x34 bytes at +0x38.
    #[must_use]
    pub fn data_38_bytes(&self) -> [u8; 0x34] {
        let mut bytes = [0_u8; 0x34];
        let mut put =
            |at: usize, value: u32| bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        put(0x00, self.attributes_38[0]);
        put(0x04, self.attributes_38[1]);
        put(0x08, self.specialization_40);
        for (index, skill) in self.major_skills_44.iter().enumerate() {
            put(0x0C + 4 * index, *skill);
        }
        put(0x28, self.flags_60);
        put(0x2C, self.services_64);
        bytes[0x30] = self.trainer_skill_68;
        bytes[0x31] = self.trainer_level_69;
        bytes[0x32..0x34].copy_from_slice(&self.unused_6a.to_le_bytes());
        bytes
    }

    /// IDA: N/A the same bytes back into the fields.
    pub fn set_data_38_bytes(&mut self, bytes: &[u8; 0x34]) {
        let word =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
        self.attributes_38 = [word(0x00), word(0x04)];
        self.specialization_40 = word(0x08);
        for (index, skill) in self.major_skills_44.iter_mut().enumerate() {
            *skill = word(0x0C + 4 * index);
        }
        self.flags_60 = word(0x28);
        self.services_64 = word(0x2C);
        self.trainer_skill_68 = bytes[0x30];
        self.trainer_level_69 = bytes[0x31];
        self.unused_6a = u16::from_le_bytes([bytes[0x32], bytes[0x33]]);
    }
}

// IDA: 00565BE0 ActorValue_GroupIndexToCode(group, index): `movsx` of the index plus the group's base, 0 past group 6. Only group 2's arm, base 0x0C, is reached here. Completion: 100% of that arm.
const fn actor_value_group_2_code(index: i8) -> i32 {
    index as i32 + 0x0C
}

// IDA: 0051C1B0 TESClass_Reset, vt+0x14: memset(+0x38, 0, 0x34); each major skill from 00565BE0(2, i), stored when code - 0x0C <= 0x14 unsigned; +0x38 = 0; +0x3C = 1; then a jump to 0046C2E0. Completion: 100% of the store order.
fn reset_class(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let mut class = world
        .get_mut::<TesClass>(form)
        .expect("the class's component");
    *class = TesClass::default();
    for (index, skill) in class.major_skills_44.iter_mut().enumerate() {
        let code = actor_value_group_2_code(index as i8);
        if (code.wrapping_sub(0x0C) as u32) <= 0x14 {
            *skill = code as u32;
        }
    }
    class.attributes_38 = [0, 1];
    reset_14_components(world, form)
}

// IDA: 0051C030 TESClass_IsValid: the two attributes differ and are <= 7 unsigned, and the seven major skills are pairwise distinct. Completion: 100%.
fn class_is_valid(class: &TesClass) -> bool {
    let [first, second] = class.attributes_38;
    if first == second || first > 7 || second > 7 {
        return false;
    }
    for (index, skill) in class.major_skills_44.iter().enumerate() {
        if class.major_skills_44[index + 1..].contains(skill) {
            return false;
        }
    }
    true
}

// IDA: 0051C210 TESClass vt+0x1C LoadForm. The record must be CLAS (else 0); 00451210; the chunks: DESC 0046A390 on +0x24; DATA 0046BDA0(+0x38, 0x34), no tail, since no cast succeeds on this class; EDID (alloca, size 0x200, vt+0xD8 006FDFF0 keeps nothing); FULL 0046C7A0 on +0x18; ICON 004700E0 on +0x2C. An invalid class warns "Class %s contains invalid data. Make sure all attributes and skills
// are unique." with vt+0xD4, an empty name here. Returns 1. Completion: 100%.
fn load_class_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if u32::from(FORM_TYPE_CLASS) != cx.file.record_form_type()? {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            DESC => load_description(
                &mut world
                    .get_mut::<TesDescription>(form)
                    .expect("the class's component"),
                cx,
            )?,
            DATA => {
                let mut class = world
                    .get_mut::<TesClass>(form)
                    .expect("the class's component");
                let mut bytes = class.data_38_bytes();
                load_data_chunk(&mut bytes, 0x34, DataChunkTails::default(), cx)?;
                class.set_data_38_bytes(&bytes);
            }
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
            ICON => load_texture(
                &mut world
                    .get_mut::<TesTexture>(form)
                    .expect("the class's component"),
                cx,
            )?,
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    if !class_is_valid(world.get::<TesClass>(form).expect("the class's component")) {
        cx.file.services.warn(
            "Class  contains invalid data. Make sure all attributes and skills are unique."
                .to_owned(),
        );
    }
    Ok(true)
}

// IDA: 0051C580 TESClass_Constructor: 0046C460; +0x18 TESFullName inline (A322A0, +0x1C = 0, +0x20 = +0x22 = 0); 0046A400 on +0x24; 0046FFD0 on +0x2C; the four class vtables; +0x04 = 5; 0051C1B0. Completion: 100% of the store order.
fn construct_class(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        TesFullName {
            original_vtable: TES_FULL_NAME_VTABLE,
            name_04: BsStringT::default(),
        },
        TesTexture {
            original_vtable: TES_TEXTURE_VTABLE,
            path_04: BsStringT::default(),
        },
    ));
    let mut description = TesDescription {
        original_vtable: TES_DESCRIPTION_VTABLE,
        offset_04: 0,
    };
    reset_description(
        &mut description,
        &mut world.resource_mut::<DataHandlerGlobals>(),
    );
    description.original_vtable = TES_CLASS_DESCRIPTION_VTABLE;
    world.entity_mut(form).insert((
        description,
        TesClass::default(),
        OriginalVtable(TES_CLASS_VTABLE),
    ));
    world
        .get_mut::<TesFullName>(form)
        .expect("just inserted")
        .original_vtable = TES_CLASS_FULL_NAME_VTABLE;
    world
        .get_mut::<TesTexture>(form)
        .expect("just inserted")
        .original_vtable = TES_CLASS_TEXTURE_VTABLE;
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_CLASS;
    reset_class(world, form)?;
    Ok(form)
}

// IDA: the chunks the other classes here add.
const CNAM: u32 = 0x4D41_4E43;
const FNAM: u32 = 0x4D41_4E46;
const GNAM: u32 = 0x4D41_4E47;
const HNAM: u32 = 0x4D41_4E48;
const INAM: u32 = 0x4D41_4E49;
const MNAM: u32 = 0x4D41_4E4D;
const MODB: u32 = 0x4244_4F4D;
const MODL: u32 = 0x4C44_4F4D;
const MODT: u32 = 0x5444_4F4D;
const RNAM: u32 = 0x4D41_4E52;
const SNAM: u32 = 0x4D41_4E53;
const SNDD: u32 = 0x4444_4E53;
const SNDX: u32 = 0x5844_4E53;
const XNAM: u32 = 0x4D41_4E58;

// IDA: the form types and vtables, read from each constructor.
pub const FORM_TYPE_FACTION: u8 = 0x06;
pub const FORM_TYPE_HAIR: u8 = 0x07;
pub const FORM_TYPE_EYES: u8 = 0x08;
pub const FORM_TYPE_SOUND: u8 = 0x0A;
pub const FORM_TYPE_LAND_TEXTURE: u8 = 0x0E;
pub const TES_FACTION_VTABLE: u32 = 0x00A5_3524;
pub const TES_HAIR_VTABLE: u32 = 0x00A5_3654;
pub const TES_EYES_VTABLE: u32 = 0x00A5_3414;
pub const TES_SOUND_VTABLE: u32 = 0x00A5_219C;
pub const TES_LAND_TEXTURE_VTABLE: u32 = 0x00A4_656C;

const FACTION_SLOTS: &[ComponentSlot] = &[ComponentSlot::FullName, ComponentSlot::Reaction];
const HAIR_SLOTS: &[ComponentSlot] = &[
    ComponentSlot::FullName,
    ComponentSlot::Texture,
    ComponentSlot::Model,
];
const EYES_SLOTS: &[ComponentSlot] = &[ComponentSlot::FullName, ComponentSlot::Texture];
const SOUND_SLOTS: &[ComponentSlot] = &[ComponentSlot::SoundFile];
const LAND_TEXTURE_SLOTS: &[ComponentSlot] = &[ComponentSlot::Texture];

// IDA: the inline TESFullName of these classes, with the class's secondary vtable.
fn full_name(vtable: u32) -> TesFullName {
    debug_assert_ne!(vtable, TES_FULL_NAME_VTABLE);
    TesFullName {
        original_vtable: vtable,
        name_04: BsStringT::default(),
    }
}

// IDA: 0046FFD0 TESTexture, with the class's secondary vtable.
fn texture(vtable: u32) -> TesTexture {
    debug_assert_ne!(vtable, TES_TEXTURE_VTABLE);
    TesTexture {
        original_vtable: vtable,
        path_04: BsStringT::default(),
    }
}

// IDA: 0046D7E0 TESModel, with the class's secondary vtable.
fn model(vtable: u32) -> TesModel {
    debug_assert_ne!(vtable, TES_MODEL_VTABLE);
    TesModel {
        original_vtable: vtable,
        ..TesModel::default()
    }
}

// IDA: N/A +0x04.
fn set_form_type(world: &mut World, form: Entity, form_type: u8) {
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = form_type;
}

/// IDA: 0046AB80 TESForm_SetInitializedFlag(set): +0x08 bit 3. Completion: 100%.
pub fn set_initialized(world: &mut World, form: Entity, set: bool) {
    let mut identity = world.get_mut::<FormIdentity>(form).expect("a form");
    if set {
        identity.flags_08 |= 8;
    } else {
        identity.flags_08 &= !8;
    }
}

/// IDA: 0046C640 TESForm vt+0x90(0) (004129B0 jumps here): with the altered bit (+0x08 bit 1) set, the 00B06150 walk, a host stop; otherwise a clear of a clear bit. Completion: 100% of the arm reached.
/// TODO: [ ] 0046C69D the 00B06150 array, with vt+0x90(1).
pub fn clear_altered(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let mut identity = world.get_mut::<FormIdentity>(form).expect("a form");
    if identity.flags_08 & 2 != 0 {
        return Err(StreamError("0046C69D: the 00B06150 altered-form array"));
    }
    identity.flags_08 &= !2;
    Ok(())
}

// IDA: N/A a MODL / MODB / MODT arm: 0046DB00 on the form's one TESModel, which is out of the world for the call.
pub(super) fn load_model_arm(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    let mut held = std::mem::take(&mut *world.get_mut::<TesModel>(form).expect("a TESModel"));
    let result = load_model(world, &mut held, Some(form), cx);
    *world.get_mut::<TesModel>(form).expect("a TESModel") = held;
    result
}

/// IDA: 0051F570 a TESFaction rank (FormHeap 0x1C): +0x00 male title, +0x08 female title, +0x10 the insignia TESTexture (vtable A3C650).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TesFactionRank {
    pub male_00: BsStringT,
    pub female_08: BsStringT,
    pub insignia_10: TesTexture,
}

/// IDA: 0051F820 TESFaction (0x44): TESFullName +0x18, TESReactionForm +0x24, +0x34 byte flags, +0x38 float crime gold multiplier 1.0, +0x3C/+0x40 the rank list.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesFaction {
    pub flags_34: u8,
    pub crime_gold_multiplier_38: f32,
    pub ranks_3c: Vec<TesFactionRank>,
}

// IDA: 0051F820 TESFaction_Constructor: 0046C460; +0x18 inline; 0046E5E0 on +0x24; the three vtables; +0x3C = +0x40 = 0; +0x34 = 0; +0x38 = 1.0; 0046C2E0 while +0x04 is still 0; +0x04 = 6. Completion: 100% of the store order.
fn construct_faction(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        full_name(0x00A5_3510),
        TesReactionForm {
            original_vtable: 0x00A5_34FC,
            reactions_04: Vec::new(),
            group_type_0c: 6,
        },
        OriginalVtable(TES_FACTION_VTABLE),
        TesFaction {
            flags_34: 0,
            crime_gold_multiplier_38: 1.0,
            ranks_3c: Vec::new(),
        },
    ));
    debug_assert_ne!(TES_REACTION_FORM_VTABLE, 0x00A5_34FC);
    reset_form_components(world, form, FACTION_SLOTS)?;
    set_form_type(world, form, FORM_TYPE_FACTION);
    Ok(form)
}

// IDA: 0051F090 TESFaction vt+0x14: +0x34 = 0, +0x38 = 1.0, a jump to 0046C2E0. Completion: 100%.
fn reset_faction(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let mut faction = world.get_mut::<TesFaction>(form).expect("a faction");
    faction.flags_34 = 0;
    faction.crime_gold_multiplier_38 = 1.0;
    reset_14_components(world, form)
}

// IDA: 0051FCC0 TESFaction vt+0x18: 0051FB00 (0051F2D0 and the block for every rank, then each node), a jump to 0046C2F0. Completion: 100%.
fn destroy_faction(world: &mut World, form: Entity) -> Result<(), StreamError> {
    world
        .get_mut::<TesFaction>(form)
        .expect("a faction")
        .ranks_3c
        .clear();
    reset_18_components(world, form)
}

// IDA: 0051F8C0 TESFaction vt+0x1C LoadForm, with no record-type test. 00451210; then a do-while on 0044FEA0 with no test of the chunk type. A rank local ([ebp-0x14]) starts null. EDID (size 0x200, vt+0xD8 006FDFF0); DATA 0046BDA0(+0x34, 1); FULL 0046C7A0 on +0x18; CNAM 004510E0 straight into +0x38; RNAM (no read: FormHeap(0x1C), 0051F570, 0067B1E0 onto +0x3C, and it's the local); XNAM (FormHeap(8), 8 bytes read in place, 0067B1E0 onto +0x28); with a rank, FNAM its +0x08, MNAM its +0x00, INAM its +0x14 (the jump table at 0051FAD4 over 0051FAE8, read from the exe). Returns 1. Completion: 100%.
fn load_faction_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    initialize_form_from_current_record(world, form, cx)?;
    let mut rank: Option<usize> = None;
    loop {
        match cx.file.chunk_type()? {
            EDID => {
                cx.file.read_text(0x200)?;
            }
            DATA => {
                let mut bytes = [world.get::<TesFaction>(form).expect("a faction").flags_34];
                load_data_chunk(&mut bytes, 1, DataChunkTails::default(), cx)?;
                world
                    .get_mut::<TesFaction>(form)
                    .expect("a faction")
                    .flags_34 = bytes[0];
            }
            FULL => load_full_name(
                &mut world
                    .get_mut::<TesFullName>(form)
                    .expect("the class's component")
                    .name_04,
                cx,
            )?,
            CNAM => {
                let current = world
                    .get::<TesFaction>(form)
                    .expect("a faction")
                    .crime_gold_multiplier_38;
                let (bits, _) = cx.file.read_dword(current.to_bits())?;
                world
                    .get_mut::<TesFaction>(form)
                    .expect("a faction")
                    .crime_gold_multiplier_38 = f32::from_bits(bits);
            }
            RNAM => {
                let mut faction = world.get_mut::<TesFaction>(form).expect("a faction");
                faction.ranks_3c.push(TesFactionRank {
                    insignia_10: TesTexture {
                        original_vtable: TES_TEXTURE_VTABLE,
                        path_04: BsStringT::default(),
                    },
                    ..TesFactionRank::default()
                });
                rank = Some(faction.ranks_3c.len() - 1);
            }
            XNAM => {
                let mut bytes = [0_u8; 8];
                cx.file.read_chunk(&mut bytes, 8)?;
                world
                    .get_mut::<TesReactionForm>(form)
                    .expect("the class's component")
                    .reactions_04
                    .push(TesReaction {
                        faction: FormRef::Raw(u32::from_le_bytes(
                            bytes[..4].try_into().expect("four bytes"),
                        )),
                        modifier: i32::from_le_bytes(bytes[4..].try_into().expect("four bytes")),
                    });
            }
            chunk @ (FNAM | MNAM | INAM) => {
                if let Some(index) = rank {
                    let text = cx.file.read_text(0)?;
                    let mut faction = world.get_mut::<TesFaction>(form).expect("a faction");
                    let block = &mut faction.ranks_3c[index];
                    let target = match chunk {
                        FNAM => &mut block.female_08,
                        MNAM => &mut block.male_00,
                        _ => &mut block.insignia_10.path_04,
                    };
                    target.set(Some(&text), 0);
                }
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            return Ok(true);
        }
    }
}

/// IDA: 005200F0 TESHair (0x4C): TESFullName +0x18, TESModel +0x24, TESTexture +0x3C, +0x48 byte flags.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesHair {
    pub flags_48: u8,
}

// IDA: 005200F0 TESHair_Constructor: 0046C460, +0x18 inline, 0046D7E0 on +0x24, 0046FFD0 on +0x3C, the four vtables, +0x04 = 7, +0x48 = 0, 0046C2E0. Completion: 100% of the store order.
fn construct_hair(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        full_name(0x00A5_3640),
        model(0x00A5_3620),
        texture(0x00A5_3604),
        OriginalVtable(TES_HAIR_VTABLE),
        TesHair::default(),
    ));
    set_form_type(world, form, FORM_TYPE_HAIR);
    reset_form_components(world, form, HAIR_SLOTS)?;
    Ok(form)
}

// IDA: 0051FD90 TESHair vt+0x14: +0x48 = 0, a jump to 0046C2E0. Completion: 100%.
fn reset_hair(world: &mut World, form: Entity) -> Result<(), StreamError> {
    world.get_mut::<TesHair>(form).expect("a hair").flags_48 = 0;
    reset_14_components(world, form)
}

// IDA: 0051FEA0 TESHair vt+0x1C LoadForm: the record must be HAIR (`cmp eax, 7`, else 0); 00451210; the chunks while nonzero: EDID; DATA 0046BDA0(+0x48, 1); FULL +0x18; ICON +0x3C; MODL, MODB, MODT 0046DB00 on +0x24. Returns 1. Completion: 100%.
fn load_hair_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if u32::from(FORM_TYPE_HAIR) != cx.file.record_form_type()? {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            EDID => {
                cx.file.read_text(0x200)?;
            }
            DATA => {
                let mut bytes = [world.get::<TesHair>(form).expect("a hair").flags_48];
                load_data_chunk(&mut bytes, 1, DataChunkTails::default(), cx)?;
                world.get_mut::<TesHair>(form).expect("a hair").flags_48 = bytes[0];
            }
            FULL => load_full_name(
                &mut world
                    .get_mut::<TesFullName>(form)
                    .expect("the class's component")
                    .name_04,
                cx,
            )?,
            ICON => load_texture(
                &mut world
                    .get_mut::<TesTexture>(form)
                    .expect("the class's component"),
                cx,
            )?,
            MODL | MODB | MODT => load_model_arm(world, form, cx)?,
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    Ok(true)
}

/// IDA: 0051EFD0 TESEyes (0x34): TESFullName +0x18, TESTexture +0x24, +0x30 byte flags.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesEyes {
    pub flags_30: u8,
}

// IDA: 0051EFD0 TESEyes_Constructor: 0046C460, +0x18 inline, 0046FFD0 on +0x24, the three vtables, +0x04 = 8, +0x30 = 0, 0046C2E0. Completion: 100% of the store order.
fn construct_eyes(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        full_name(0x00A5_3400),
        texture(0x00A5_33E4),
        OriginalVtable(TES_EYES_VTABLE),
        TesEyes::default(),
    ));
    set_form_type(world, form, FORM_TYPE_EYES);
    reset_form_components(world, form, EYES_SLOTS)?;
    Ok(form)
}

// IDA: 0051ED20 TESEyes vt+0x14: +0x30 = 0, a jump to 0046C2E0. Completion: 100%.
fn reset_eyes(world: &mut World, form: Entity) -> Result<(), StreamError> {
    world.get_mut::<TesEyes>(form).expect("eyes").flags_30 = 0;
    reset_14_components(world, form)
}

// IDA: 0051ED90 TESEyes vt+0x1C LoadForm: the record must be EYES (`cmp eax, 8`, else 0); 00451210; the chunks while nonzero: EDID; DATA 0046BDA0(+0x30, 1); FULL +0x18; ICON +0x24. Returns 1. Completion: 100%.
fn load_eyes_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if u32::from(FORM_TYPE_EYES) != cx.file.record_form_type()? {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            EDID => {
                cx.file.read_text(0x200)?;
            }
            DATA => {
                let mut bytes = [world.get::<TesEyes>(form).expect("eyes").flags_30];
                load_data_chunk(&mut bytes, 1, DataChunkTails::default(), cx)?;
                world.get_mut::<TesEyes>(form).expect("eyes").flags_30 = bytes[0];
            }
            FULL => load_full_name(
                &mut world
                    .get_mut::<TesFullName>(form)
                    .expect("the class's component")
                    .name_04,
                cx,
            )?,
            ICON => load_texture(
                &mut world
                    .get_mut::<TesTexture>(form)
                    .expect("the class's component"),
                cx,
            )?,
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    Ok(true)
}

/// IDA: 00517F40 TESSound (0x44): TESBoundAnimObject (`TesObjectLinks`), TESSoundFile +0x24, +0x30 BSStringT editor ID, and the SNDX block +0x38..+0x43.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesSound {
    pub editor_id_30: BsStringT,
    pub min_attenuation_38: u8,
    pub max_attenuation_39: u8,
    pub frequency_adjustment_3a: u8,
    pub byte_3b: u8,
    pub flags_3c: u32,
    pub static_attenuation_40: u16,
    pub word_42: u16,
}

impl TesSound {
    /// IDA: N/A the 0x0C bytes at +0x38.
    #[must_use]
    pub fn data_38_bytes(&self) -> [u8; 0x0C] {
        let mut bytes = [0_u8; 0x0C];
        bytes[0] = self.min_attenuation_38;
        bytes[1] = self.max_attenuation_39;
        bytes[2] = self.frequency_adjustment_3a;
        bytes[3] = self.byte_3b;
        bytes[4..8].copy_from_slice(&self.flags_3c.to_le_bytes());
        bytes[8..10].copy_from_slice(&self.static_attenuation_40.to_le_bytes());
        bytes[10..12].copy_from_slice(&self.word_42.to_le_bytes());
        bytes
    }

    // IDA: N/A the same bytes back.
    fn set_data_38_bytes(&mut self, bytes: &[u8; 0x0C]) {
        self.min_attenuation_38 = bytes[0];
        self.max_attenuation_39 = bytes[1];
        self.frequency_adjustment_3a = bytes[2];
        self.byte_3b = bytes[3];
        self.flags_3c = u32::from_le_bytes(bytes[4..8].try_into().expect("four bytes"));
        self.static_attenuation_40 = u16::from_le_bytes([bytes[8], bytes[9]]);
        self.word_42 = u16::from_le_bytes([bytes[10], bytes[11]]);
    }

    // IDA: 00517C10's zero stores, 00517F6D's too: +0x3C, +0x39, +0x38, +0x3A, +0x40, +0x3B, +0x42.
    fn clear_data(&mut self) {
        self.set_data_38_bytes(&[0; 0x0C]);
    }
}

// IDA: 00517F40 TESSound_Constructor: 004B31D0, TESSoundFile inline +0x24, the two vtables, +0x30 clear, +0x04 = 0x0A, the 00517C10 stores, 0046C2E0. Completion: 100% of the store order.
fn construct_sound(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        TesObjectLinks::default(),
        TesSoundFile {
            original_vtable: 0x00A5_2180,
            path_04: BsStringT::default(),
        },
        OriginalVtable(TES_SOUND_VTABLE),
        TesSound::default(),
    ));
    debug_assert_ne!(TES_SOUND_FILE_VTABLE, 0x00A5_2180);
    set_form_type(world, form, FORM_TYPE_SOUND);
    reset_form_components(world, form, SOUND_SLOTS)?;
    Ok(form)
}

// IDA: 00517C10 TESSound vt+0x14: the stores, a jump to 0046C2E0. +0x30 stays. Completion: 100%.
fn reset_sound(world: &mut World, form: Entity) -> Result<(), StreamError> {
    world
        .get_mut::<TesSound>(form)
        .expect("a sound")
        .clear_data();
    reset_14_components(world, form)
}

// IDA: 00517CB0 TESSound vt+0x1C LoadForm: the record must be SOUN (`cmp eax, 0xA`, else 0); 00451210; the chunks while nonzero: EDID (size 0x200, vt+0xD8 00517EA0: 004028D0(+0x30, text, 0)); SNDD 0046BDA0 into an 8-byte local, then +0x38, +0x39, +0x3A from its first bytes and +0x3C from its dword at 4, leaving +0x3B and +0x40; FNAM (the text, vt+0x10 of +0x24, 0046E080 by the bytes of A52190: 004028D0(+0x28, text, 0)); SNDX 0046BDA0(+0x38, 0x0C). Returns 1. A short SNDD leaves stack bytes in the local; the host has zero. Completion: 100%.
fn load_sound_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if u32::from(FORM_TYPE_SOUND) != cx.file.record_form_type()? {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            EDID => {
                let text = cx.file.read_text(0x200)?;
                world
                    .get_mut::<TesSound>(form)
                    .expect("a sound")
                    .editor_id_30
                    .set(Some(&text), 0);
            }
            SNDD => {
                let mut local = [0_u8; 8];
                load_data_chunk(&mut local, 8, DataChunkTails::default(), cx)?;
                let mut sound = world.get_mut::<TesSound>(form).expect("a sound");
                sound.min_attenuation_38 = local[0];
                sound.max_attenuation_39 = local[1];
                sound.frequency_adjustment_3a = local[2];
                sound.flags_3c = u32::from_le_bytes(local[4..].try_into().expect("four bytes"));
            }
            FNAM => {
                let text = cx.file.read_text(0)?;
                world
                    .get_mut::<TesSoundFile>(form)
                    .expect("a sound")
                    .path_04
                    .set(Some(&text), 0);
            }
            SNDX => {
                let mut bytes = world
                    .get::<TesSound>(form)
                    .expect("a sound")
                    .data_38_bytes();
                load_data_chunk(&mut bytes, 0x0C, DataChunkTails::default(), cx)?;
                world
                    .get_mut::<TesSound>(form)
                    .expect("a sound")
                    .set_data_38_bytes(&bytes);
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

// IDA: 00517EC0 TESSound vt+0xD4: +0x30, or `Default`. Completion: 100%.
fn sound_editor_id(world: &World, form: Entity) -> Vec<u8> {
    world
        .get::<TesSound>(form)
        .expect("a sound")
        .editor_id_30
        .data_00
        .clone()
        .unwrap_or_default()
}

/// IDA: 004C93D0 TESLandTexture (0x34): TESTexture +0x18, +0x24 NiPointer (null here), +0x28 HNAM's three bytes then SNAM's, +0x2C/+0x30 the GNAM list of raw grass form IDs (12d resolves them).
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesLandTexture {
    pub object_24: u32,
    pub havok_28: [u8; 4],
    pub grasses_2c: Vec<FormRef>,
}

// IDA: 004C93D0 TESLandTexture_Constructor: 0046C460; 0046FFD0 on +0x18; the two vtables; +0x24 = +0x2C = +0x30 = 0; +0x04 = 0x0E; 0046AB80(1); the null +0x24 test; +0x28 = 2, +0x29..+0x2B = 0x1E; 0046C2E0. Completion: 100% of the store order.
pub(crate) fn construct_land_texture(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        texture(0x00A4_6550),
        OriginalVtable(TES_LAND_TEXTURE_VTABLE),
        TesLandTexture {
            object_24: 0,
            havok_28: [2, 0x1E, 0x1E, 0x1E],
            grasses_2c: Vec::new(),
        },
    ));
    set_form_type(world, form, FORM_TYPE_LAND_TEXTURE);
    set_initialized(world, form, true);
    reset_form_components(world, form, LAND_TEXTURE_SLOTS)?;
    Ok(form)
}

// IDA: 004C8D80 TESLandTexture vt+0x14: the +0x24 release (null while nothing sets it), +0x28 = 2, +0x29..+0x2B = 0x1E, a jump to 0046C2E0. Completion: 100%.
fn reset_land_texture(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let mut land = world.get_mut::<TesLandTexture>(form).expect("a texture");
    land.object_24 = 0;
    land.havok_28 = [2, 0x1E, 0x1E, 0x1E];
    reset_14_components(world, form)
}

// IDA: 004C8DD0 TESLandTexture vt+0x18: the +0x24 release, the nodes after the head freed, +0x2C = 0, a jump to 0046C2F0. Completion: 100%.
fn destroy_land_texture(world: &mut World, form: Entity) -> Result<(), StreamError> {
    let mut land = world.get_mut::<TesLandTexture>(form).expect("a texture");
    land.object_24 = 0;
    land.grasses_2c.clear();
    reset_18_components(world, form)
}

// IDA: 004C90A0 TESLandTexture vt+0x1C LoadForm: the record must be LTEX (`cmp al, 0xE`, else 0); 00451210; a do-while on 0044FEA0 with no test of the chunk type: EDID; HNAM 3 bytes into +0x28; SNAM 1 byte into +0x2B; ICON and MODL 004700E0 on +0x18; GNAM 4 bytes into a zeroed local, nonzero 0067B1E0 onto +0x2C. Then 0046AB80(0). Returns 1. Completion: 100%.
fn load_land_texture_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_LAND_TEXTURE {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    loop {
        match cx.file.chunk_type()? {
            EDID => {
                cx.file.read_text(0x200)?;
            }
            HNAM => {
                let mut land = world.get_mut::<TesLandTexture>(form).expect("a texture");
                cx.file.read_chunk(&mut land.havok_28[..3], 3)?;
            }
            SNAM => {
                let mut land = world.get_mut::<TesLandTexture>(form).expect("a texture");
                cx.file.read_chunk(&mut land.havok_28[3..], 1)?;
            }
            ICON | MODL => load_texture(
                &mut world
                    .get_mut::<TesTexture>(form)
                    .expect("the class's component"),
                cx,
            )?,
            GNAM => {
                let (id, _) = cx.file.read_dword(0)?;
                if id != 0 {
                    world
                        .get_mut::<TesLandTexture>(form)
                        .expect("a texture")
                        .grasses_2c
                        .push(FormRef::Raw(id));
                }
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
    }
    set_initialized(world, form, false);
    Ok(true)
}

/// IDA: this family's classes. A type absent here takes `AbsentArm`.
/// TODO: [ ] vt+0x6C: FACT 0051F290 and LTEX 004C8FB0 are 12d's. HAIR, EYES and SOUN are 0060D0A0.
pub const CLASSES: &[FormClass] = &[
    FormClass {
        form_type: FORM_TYPE_CLASS,
        vtable: TES_CLASS_VTABLE,
        construct: construct_class,
        load_form: load_class_form,
        reset_14: reset_class,
        reset_18: reset_18_components,
        editor_id: default_editor_id,
        init_item: no_init_item,
        slots: &[
            ComponentSlot::Description,
            ComponentSlot::FullName,
            ComponentSlot::Texture,
        ],
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_FACTION,
        vtable: TES_FACTION_VTABLE,
        construct: construct_faction,
        load_form: load_faction_form,
        reset_14: reset_faction,
        reset_18: destroy_faction,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_faction,
        slots: FACTION_SLOTS,
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_HAIR,
        vtable: TES_HAIR_VTABLE,
        construct: construct_hair,
        load_form: load_hair_form,
        reset_14: reset_hair,
        reset_18: reset_18_components,
        editor_id: default_editor_id,
        init_item: no_init_item,
        slots: HAIR_SLOTS,
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_EYES,
        vtable: TES_EYES_VTABLE,
        construct: construct_eyes,
        load_form: load_eyes_form,
        reset_14: reset_eyes,
        reset_18: reset_18_components,
        editor_id: default_editor_id,
        init_item: no_init_item,
        slots: EYES_SLOTS,
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_SOUND,
        vtable: TES_SOUND_VTABLE,
        construct: construct_sound,
        load_form: load_sound_form,
        reset_14: reset_sound,
        reset_18: reset_18_components,
        editor_id: sound_editor_id,
        init_item: no_init_item,
        slots: SOUND_SLOTS,
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_LAND_TEXTURE,
        vtable: TES_LAND_TEXTURE_VTABLE,
        construct: construct_land_texture,
        load_form: load_land_texture_form,
        reset_14: reset_land_texture,
        reset_18: destroy_land_texture,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_land_texture,
        slots: LAND_TEXTURE_SLOTS,
        special_arm: None,
    },
];
