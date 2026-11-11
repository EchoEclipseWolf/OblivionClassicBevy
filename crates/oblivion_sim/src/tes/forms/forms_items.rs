//! Phase 12b: the bound-object items of the 00447DF0 factory: ACTI, APPA, ARMO, BOOK, CLOT, CONT, DOOR, INGR, LIGH, MISC, STAT, GRAS, TREE, FLOR, FURN, WEAP, AMMO, LVLC, SLGM, KEYM, ALCH, SBSP, SGST and LVLI, with the effect item lists INGR, ALCH and SGST load, and this family's arms of 0044AC20. A form is an entity with the shared components of `form_components` and one component of its own; a class's RTTI casts are the presence of a component.
//!
//! Evidence: objdump of each address named here; P12-RUN-001 and P12-RUN-003 rows. InitItem bodies are 12d's: each class here keeps `no_init_item` with a TODO in `CLASSES`.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;

use super::form::{FORM_FLAG_PARTIAL, FormIdentity, OriginalVtable};
use super::form_arms::{
    FormClass, FormFn, FormLoadCx, default_editor_id, editor_id_of, form_class, no_init_item,
    reset_14_components, reset_18_components,
};
use super::form_components::{
    ComponentSlot, DataChunkTails, FormRef, TES_ENCHANTABLE_FORM_VTABLE, TesAttackDamageForm,
    TesBipedModelForm, TesContainer, TesEnchantableForm, TesFullName, TesHealthForm,
    TesLeveledList, TesModel, TesObjectLinks, TesProduceForm, TesQualityForm, TesScriptableForm,
    TesUsesForm, TesValueForm, TesWeightForm, append_tes_object, load_biped_chunk, load_data_chunk,
    load_description, load_full_name, load_texture, reset_form_components,
};
use super::form_hierarchy::RttiClass;
use super::forms_init::cast;
use super::forms_magic::{EffectItemList, load_efid_chunk};
use super::forms_misc::{TesSound, clear_altered, load_model_arm, set_initialized};
use super::forms_script::{latin1, load_script_reference};
use crate::bs::string::BsStringT;
use crate::tes::data_handler::{
    DataHandler, DataHandlerGlobals, DataHandlerLists, EditorIdMap, FormsById,
    TES_DESCRIPTION_VTABLE, TesDescription, TesTexture,
};
use crate::tes::data_load::initialize_form_from_current_record;
use crate::tes::form::{construct_tes_form, set_form_id};
use crate::tes::form_components::reset_description;
use crate::tes::tes::{GameThreadLocals, TesGlobals};
use crate::tes::tes_file::{map_file_form_id, read_id, read_word};

// IDA: the chunks these loaders compare.
const ANAM: u32 = 0x4D41_4E41;
const BNAM: u32 = 0x4D41_4E42;
const CNAM: u32 = 0x4D41_4E43;
const CNTO: u32 = 0x4F54_4E43;
const DATA: u32 = 0x4154_4144;
const DESC: u32 = 0x4353_4544;
const DMTL: u32 = 0x4C54_4D44;
const DNAM: u32 = 0x4D41_4E44;
const EDID: u32 = 0x4449_4445;
const EFID: u32 = 0x4449_4645;
const ENAM: u32 = 0x4D41_4E45;
const ENIT: u32 = 0x5449_4E45;
const FNAM: u32 = 0x4D41_4E46;
const FULL: u32 = 0x4C4C_5546;
const ICON: u32 = 0x4E4F_4349;
const LVLD: u32 = 0x444C_564C;
const LVLF: u32 = 0x464C_564C;
const LVLO: u32 = 0x4F4C_564C;
const MNAM: u32 = 0x4D41_4E4D;
const MODB: u32 = 0x4244_4F4D;
const MODL: u32 = 0x4C44_4F4D;
const MODT: u32 = 0x5444_4F4D;
const PFIG: u32 = 0x4749_4650;
const PFPC: u32 = 0x4350_4650;
const QNAM: u32 = 0x4D41_4E51;
const SCRI: u32 = 0x4952_4353;
const SLCP: u32 = 0x5043_4C53;
const SNAM: u32 = 0x4D41_4E53;
const SOUL: u32 = 0x4C55_4F53;
const TNAM: u32 = 0x4D41_4E54;

// ---- the arms every class shares ----

// IDA: N/A +0x04.
fn set_form_type(world: &mut World, form: Entity, form_type: u8) {
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = form_type;
}

// IDA: 004B2FF0 TESBoundObject / 004B31D0 TESBoundAnimObject / 004B31F0 TESBoundTreeObject: 0046C460, +0x1C = +0x20 = +0x18 = 0, the vtable, which a class's own store overwrites before anything reads it. The class's vtable is what's stored here.
fn construct_bound(world: &mut World, vtable: u32) -> Result<Entity, GameError> {
    let form = construct_tes_form(world)?;
    world
        .entity_mut(form)
        .insert((TesObjectLinks::default(), OriginalVtable(vtable)));
    Ok(form)
}

// IDA: the inline TESFullName, with the class's vtable.
fn full_name(vtable: u32) -> TesFullName {
    TesFullName {
        original_vtable: vtable,
        name_04: BsStringT::default(),
    }
}

// IDA: 0046D7E0 TESModel, with the class's vtable.
fn model(vtable: u32) -> TesModel {
    TesModel {
        original_vtable: vtable,
        ..TesModel::default()
    }
}

// IDA: 0046FFD0 TESTexture with TESIcon's store, with the class's vtable.
fn icon(vtable: u32) -> TesTexture {
    TesTexture {
        original_vtable: vtable,
        path_04: BsStringT::default(),
    }
}

// IDA: 0046EF60 TESScriptableForm, with the class's vtable.
fn scriptable(vtable: u32) -> TesScriptableForm {
    TesScriptableForm {
        original_vtable: vtable,
        script_04: FormRef::Raw(0),
        resolved_08: 0,
    }
}

// IDA: 00470470 TESValueForm.
fn value(vtable: u32) -> TesValueForm {
    TesValueForm {
        original_vtable: vtable,
        value_04: 0,
    }
}

// IDA: 004705A0 TESWeightForm.
fn weight(vtable: u32) -> TesWeightForm {
    TesWeightForm {
        original_vtable: vtable,
        weight_04: 0.0,
    }
}

// IDA: 0046CA50 TESHealthForm.
fn health(vtable: u32) -> TesHealthForm {
    TesHealthForm {
        original_vtable: vtable,
        health_04: 0,
    }
}

// IDA: 00468A60 TESAttackDamageForm.
fn damage(vtable: u32) -> TesAttackDamageForm {
    TesAttackDamageForm {
        original_vtable: vtable,
        damage_04: 0,
    }
}

// IDA: 0046A790 TESEnchantableForm: +0x04 = 0, +0x08 = 0, +0x0C = 4.
fn enchantable(vtable: u32) -> TesEnchantableForm {
    debug_assert_ne!(vtable, TES_ENCHANTABLE_FORM_VTABLE);
    TesEnchantableForm {
        original_vtable: vtable,
        enchantment_04: FormRef::Raw(0),
        charge_08: 0,
        cast_type_0c: 4,
    }
}

// IDA: 004690D0 TESBipedModelForm: the parts' own vtables (TESModel A3C24C, TESIcon A320A4) stay, since the class only replaces +0x00's. The slot mask's reset comes through vt+0.
fn biped(vtable: u32) -> TesBipedModelForm {
    let part = TesModel {
        original_vtable: super::form_components::TES_MODEL_VTABLE,
        ..TesModel::default()
    };
    let part_icon = TesTexture {
        original_vtable: super::form_components::TES_ICON_VTABLE,
        path_04: BsStringT::default(),
    };
    TesBipedModelForm {
        original_vtable: vtable,
        slot_mask_04: 0,
        flags_06: 0,
        unset_07: 0,
        biped_models_08: [part.clone(), part.clone()],
        world_models_38: [part.clone(), part],
        icons_68: [part_icon.clone(), part_icon],
    }
}

/// IDA: 0046ABF0 TESForm vt+0x40 SetChangeFlags(flags) -> 0045B670(g_saveLoadGame, form, flags), in the exe's order: TLS +0x185 clear returns, TLS +0x184 set returns, DataHandler +0xCD4 set returns (a null DataHandler faults), form flag 0x4000 returns; then `TesSaveLoadGame::set_change_flags`. WinMain 0040EB8B sets +0x185, so from there this records. Completion: 100%.
/// # Errors
/// A null DataHandler or g_saveLoadGame past the TLS gates, which fault in the original.
pub fn set_change_flags(world: &mut World, form: Entity, flags: u32) -> Result<(), GameError> {
    let locals = world.resource::<GameThreadLocals>();
    if locals.track_changes_185 == 0 || locals.byte_184 != 0 {
        return Ok(());
    }
    let handler = world
        .get_resource::<DataHandler>()
        .ok_or(GameError::unclassified(
            0x0045B69A,
            "0045B69A: a null DataHandler faults",
        ))?;
    if handler.bytes_cd0[4] != 0 {
        return Ok(());
    }
    let identity = *world.get::<FormIdentity>(form).expect("a form");
    if identity.flags_08 & FORM_FLAG_PARTIAL != 0 {
        return Ok(());
    }
    world
        .get_resource_mut::<TesGlobals>()
        .and_then(|globals| globals.into_inner().save_load_game.as_mut())
        .ok_or(GameError::unclassified(
            0x0045B6B4,
            "0045B6B4: 0045A500 on a null g_saveLoadGame",
        ))?
        .set_change_flags(identity.form_id_0c, flags);
    Ok(())
}

/// IDA: 0046AC10 TESForm vt+0x44 ClearChangeFlags(flags) -> 004533D0 on g_saveLoadGame: a temporary form returns, otherwise a tail call of `TesSaveLoadGame::clear_change_flags`, with no TLS gate, unlike vt+0x40. Completion: 100%.
/// # Errors
/// A null g_saveLoadGame, or a release of a record's block.
pub fn clear_change_flags(world: &mut World, form: Entity, flags: u32) -> Result<(), GameError> {
    let identity = *world.get::<FormIdentity>(form).expect("a form");
    if identity.flags_08 & FORM_FLAG_PARTIAL != 0 {
        return Ok(());
    }
    world
        .get_resource_mut::<TesGlobals>()
        .and_then(|globals| globals.into_inner().save_load_game.as_mut())
        .ok_or(GameError::unclassified(
            0x004533DF,
            "004533DF: 00452D60 on a null g_saveLoadGame",
        ))?
        .clear_change_flags(identity.form_id_0c, flags)
}

// IDA: 0046BDA0 as a class calls it: the casts are the components the form has. The tails work on copies, since the original reads them through the same form, and go back after the call. TESAttributes (an actor's) has no class here. Completion: 100%.
fn load_form_data(
    world: &mut World,
    form: Entity,
    destination: &mut [u8],
    size: u16,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    let mut uses = world.get::<TesUsesForm>(form).map(|c| c.uses_04);
    let mut item_value = world.get::<TesValueForm>(form).map(|c| c.value_04);
    let mut item_health = world.get::<TesHealthForm>(form).map(|c| c.health_04);
    let mut item_weight = world.get::<TesWeightForm>(form).map(|c| c.weight_04);
    let mut item_quality = world.get::<TesQualityForm>(form).map(|c| c.quality_04);
    let mut item_damage = world.get::<TesAttackDamageForm>(form).map(|c| c.damage_04);
    let mut change = |flags: u32| set_change_flags(world, form, flags);
    let has_value = item_value.is_some();
    load_data_chunk(
        destination,
        size,
        DataChunkTails {
            uses: uses.as_mut(),
            value: item_value.as_mut(),
            value_change: has_value.then_some(&mut change as _),
            health: item_health.as_mut(),
            weight: item_weight.as_mut(),
            quality: item_quality.as_mut(),
            attack_damage: item_damage.as_mut(),
            attributes: None,
        },
        cx,
    )?;
    if let Some(stored) = uses {
        world.get_mut::<TesUsesForm>(form).expect("present").uses_04 = stored;
    }
    if let Some(stored) = item_value {
        world
            .get_mut::<TesValueForm>(form)
            .expect("present")
            .value_04 = stored;
    }
    if let Some(stored) = item_health {
        world
            .get_mut::<TesHealthForm>(form)
            .expect("present")
            .health_04 = stored;
    }
    if let Some(stored) = item_weight {
        world
            .get_mut::<TesWeightForm>(form)
            .expect("present")
            .weight_04 = stored;
    }
    if let Some(stored) = item_quality {
        world
            .get_mut::<TesQualityForm>(form)
            .expect("present")
            .quality_04 = stored;
    }
    if let Some(stored) = item_damage {
        world
            .get_mut::<TesAttackDamageForm>(form)
            .expect("present")
            .damage_04 = stored;
    }
    Ok(())
}

// IDA: the prologue most item loaders share: `cmp al, type` (else 0), 00451210, 0046AB80(0).
fn begin_load(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
    form_type: u8,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != form_type {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    Ok(true)
}

// IDA: the prologue with no record-type test: ACTI, FLOR, FURN and MISC.
fn begin_load_untested(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), GameError> {
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    Ok(())
}

type ChunkArm = for<'a, 'b, 'c> fn(
    &mut World,
    Entity,
    &'c mut FormLoadCx<'a, 'b>,
    u32,
) -> Result<(), GameError>;

// IDA: the loop of most loaders: the chunk type, and while nonzero the arm, 0044FEA0 (a false return ends it) and the next type.
fn each_chunk(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
    arm: ChunkArm,
) -> Result<(), GameError> {
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        arm(world, form, cx, chunk)?;
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    Ok(())
}

// IDA: the loop of the leveled lists: a do-while on 0044FEA0 with no test of the chunk type.
fn every_chunk(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
    arm: ChunkArm,
) -> Result<(), GameError> {
    loop {
        let chunk = cx.file.chunk_type()?;
        arm(world, form, cx, chunk)?;
        if !cx.file.advance_chunk()? {
            return Ok(());
        }
    }
}

// IDA: the EDID arm of a class with vt+0xD8 006FDFF0: alloca, size 0x200, nothing kept.
fn edid_arm(cx: &mut FormLoadCx<'_, '_>) -> Result<(), GameError> {
    cx.file.read_text(0x200)?;
    Ok(())
}

// IDA: 0046C7A0 on the form's one TESFullName.
fn full_arm(world: &mut World, form: Entity, cx: &mut FormLoadCx<'_, '_>) -> Result<(), GameError> {
    load_full_name(
        &mut world
            .get_mut::<TesFullName>(form)
            .expect("the class's component")
            .name_04,
        cx,
    )
}

// IDA: 004700E0 on the form's one TESTexture.
fn icon_arm(world: &mut World, form: Entity, cx: &mut FormLoadCx<'_, '_>) -> Result<(), GameError> {
    load_texture(
        &mut world
            .get_mut::<TesTexture>(form)
            .expect("the class's component"),
        cx,
    )
}

// IDA: the ENAM arm of a TESEnchantableForm: 004510E0 into a zeroed local, +0x04.
fn enam_arm(world: &mut World, form: Entity, cx: &mut FormLoadCx<'_, '_>) -> Result<(), GameError> {
    let id = read_id(cx)?;
    world
        .get_mut::<TesEnchantableForm>(form)
        .expect("the class's component")
        .enchantment_04 = FormRef::Raw(id);
    Ok(())
}

// IDA: the ANAM arm of a TESEnchantableForm: 004510F0 into a zeroed local, the word to +0x08.
fn anam_arm(world: &mut World, form: Entity, cx: &mut FormLoadCx<'_, '_>) -> Result<(), GameError> {
    let charge = read_word(cx)?;
    world
        .get_mut::<TesEnchantableForm>(form)
        .expect("the class's component")
        .charge_08 = charge;
    Ok(())
}

// ---- the slots of 00466E80, ascending ----

const ACTIVATOR_SLOTS: &[ComponentSlot] = &[
    ComponentSlot::FullName,
    ComponentSlot::Model,
    ComponentSlot::Scriptable,
];
const FLORA_SLOTS: &[ComponentSlot] = &[
    ComponentSlot::FullName,
    ComponentSlot::Model,
    ComponentSlot::Produce,
    ComponentSlot::Scriptable,
];
const APPARATUS_SLOTS: &[ComponentSlot] = &[
    ComponentSlot::FullName,
    ComponentSlot::Texture,
    ComponentSlot::Model,
    ComponentSlot::Quality,
    ComponentSlot::Scriptable,
    ComponentSlot::Value,
    ComponentSlot::Weight,
];
const ARMOR_SLOTS: &[ComponentSlot] = &[
    ComponentSlot::BipedModel,
    ComponentSlot::Enchantable,
    ComponentSlot::FullName,
    ComponentSlot::Health,
    ComponentSlot::Scriptable,
    ComponentSlot::Value,
    ComponentSlot::Weight,
];
const CLOTHING_SLOTS: &[ComponentSlot] = &[
    ComponentSlot::BipedModel,
    ComponentSlot::Enchantable,
    ComponentSlot::FullName,
    ComponentSlot::Scriptable,
    ComponentSlot::Value,
    ComponentSlot::Weight,
];
const BOOK_SLOTS: &[ComponentSlot] = &[
    ComponentSlot::Description,
    ComponentSlot::Enchantable,
    ComponentSlot::FullName,
    ComponentSlot::Texture,
    ComponentSlot::Model,
    ComponentSlot::Scriptable,
    ComponentSlot::Value,
    ComponentSlot::Weight,
];
const CONTAINER_SLOTS: &[ComponentSlot] = &[
    ComponentSlot::Container,
    ComponentSlot::FullName,
    ComponentSlot::Model,
    ComponentSlot::Scriptable,
    ComponentSlot::Weight,
];
const LIGHT_SLOTS: &[ComponentSlot] = &[
    ComponentSlot::FullName,
    ComponentSlot::Texture,
    ComponentSlot::Model,
    ComponentSlot::Scriptable,
    ComponentSlot::Value,
    ComponentSlot::Weight,
];
const MISC_SLOTS: &[ComponentSlot] = LIGHT_SLOTS;
const SIGIL_SLOTS: &[ComponentSlot] = &[
    ComponentSlot::FullName,
    ComponentSlot::Texture,
    ComponentSlot::Model,
    ComponentSlot::Scriptable,
    ComponentSlot::Uses,
    ComponentSlot::Value,
    ComponentSlot::Weight,
];
const MODEL_ONLY_SLOTS: &[ComponentSlot] = &[ComponentSlot::Model];
const TREE_SLOTS: &[ComponentSlot] = &[ComponentSlot::Texture, ComponentSlot::Model];
const WEAPON_SLOTS: &[ComponentSlot] = &[
    ComponentSlot::AttackDamage,
    ComponentSlot::Enchantable,
    ComponentSlot::FullName,
    ComponentSlot::Health,
    ComponentSlot::Texture,
    ComponentSlot::Model,
    ComponentSlot::Scriptable,
    ComponentSlot::Value,
    ComponentSlot::Weight,
];
const AMMO_SLOTS: &[ComponentSlot] = &[
    ComponentSlot::AttackDamage,
    ComponentSlot::Enchantable,
    ComponentSlot::FullName,
    ComponentSlot::Texture,
    ComponentSlot::Model,
    ComponentSlot::Value,
    ComponentSlot::Weight,
];
const LEV_ITEM_SLOTS: &[ComponentSlot] = &[ComponentSlot::LeveledList];
const LEV_CREATURE_SLOTS: &[ComponentSlot] =
    &[ComponentSlot::LeveledList, ComponentSlot::Scriptable];
const MAGIC_OBJECT_SLOTS: &[ComponentSlot] = &[
    ComponentSlot::FullName,
    ComponentSlot::Texture,
    ComponentSlot::Model,
    ComponentSlot::Scriptable,
    ComponentSlot::Weight,
];

// ---- ACTI, FLOR, FURN ----

/// IDA: 004B40A0 TESObjectACTI (0x58): TESBoundAnimObject, TESFullName +0x24, TESModel +0x30, TESScriptableForm +0x48, +0x54 the SNAM sound (raw ID). TESFlora and TESFurniture build on it.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesActivator {
    pub sound_54: FormRef,
}

/// IDA: 004AE660 TESFurniture (0x5C): TESObjectACTI, +0x58 the MNAM marker flags.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesFurniture {
    pub marker_flags_58: u32,
}

// IDA: the ACTI part of 004B40A0, with the most-derived vtables: FLOR's and FURN's run it, then replace the vtables, before anything reads them. The 0046C2E0 inside it runs on the ACTI object at that point, over ACTI's three slots.
fn construct_activator_part(
    world: &mut World,
    vtables: [u32; 4],
    form_type: u8,
) -> Result<Entity, GameError> {
    let form = construct_bound(world, vtables[0])?;
    world.entity_mut(form).insert((
        full_name(vtables[1]),
        model(vtables[2]),
        scriptable(vtables[3]),
        TesActivator::default(),
    ));
    set_form_type(world, form, form_type);
    reset_form_components(world, form, ACTIVATOR_SLOTS)?;
    set_initialized(world, form, true);
    Ok(form)
}

// IDA: 004B40A0 TESObjectACTI_Constructor: 004B31D0, +0x24 inline, 0046D7E0, 0046EF60, the four vtables, +0x04 = 0x12, +0x54 = 0, 0046C2E0, 0046AB80(1). Completion: 100% of the store order.
fn construct_activator(world: &mut World) -> Result<Entity, GameError> {
    construct_activator_part(
        world,
        [0x00A4_3CB4, 0x00A4_3CA0, 0x00A4_3C80, 0x00A4_3C6C],
        0x12,
    )
}

// IDA: 004ADFC0 TESFlora_Constructor: 0046E0B0 on its +0x00 part, 004B40A0 on +0x0C (the form), the five vtables, type 0x1F, 0046AB80(1). The produce part sits 0x0C before the form, so a dump of the form never holds it. Completion: 100% of the store order.
fn construct_flora(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_activator_part(
        world,
        [0x00A4_1B1C, 0x00A4_1B04, 0x00A4_1AE4, 0x00A4_1AD0],
        0x1F,
    )?;
    world.entity_mut(form).insert(TesProduceForm {
        original_vtable: 0x00A4_1C40,
        ingredient_04: FormRef::Raw(0),
        chances_08: [0; 4],
    });
    set_initialized(world, form, true);
    Ok(form)
}

// IDA: 004AE660 TESFurniture_Constructor: 004B40A0, the four vtables, +0x04 = 0x20, +0x58 = 0, 004B3C90, 0046AB80(1). Completion: 100% of the store order.
fn construct_furniture(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_activator_part(
        world,
        [0x00A4_1D1C, 0x00A4_1D08, 0x00A4_1CE8, 0x00A4_1CD4],
        0x20,
    )?;
    world.entity_mut(form).insert(TesFurniture::default());
    reset_activator(world, form)?;
    set_initialized(world, form, true);
    Ok(form)
}

// IDA: 004B3C90 vt+0x14 of ACTI and FLOR: +0x54 = 0, a jump to 0046C2E0. Completion: 100%.
fn reset_activator(world: &mut World, form: Entity) -> Result<(), GameError> {
    world
        .get_mut::<TesActivator>(form)
        .expect("an activator")
        .sound_54 = FormRef::Raw(0);
    reset_14_components(world, form)
}

// IDA: 004AE530 TESFurniture vt+0x14: +0x58 = 0, a jump to 004B3C90. Completion: 100%.
fn reset_furniture(world: &mut World, form: Entity) -> Result<(), GameError> {
    world
        .get_mut::<TesFurniture>(form)
        .expect("furniture")
        .marker_flags_58 = 0;
    reset_activator(world, form)
}

// IDA: the chunk arms ACTI, FLOR and FURN share, with no SNAM in the last two.
fn activator_common_arm(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
    chunk: u32,
) -> Result<(), GameError> {
    match chunk {
        MODL | MODB | MODT => load_model_arm(world, form, cx),
        FULL => full_arm(world, form, cx),
        EDID => edid_arm(cx),
        SCRI => load_script_reference(world, form, cx),
        _ => Ok(()),
    }
}

// IDA: 004B3DD0 TESObjectACTI vt+0x1C LoadForm, with no record-type test: 00451210, 0046AB80(0), the chunks while nonzero: MODL, MODB, MODT +0x30; EDID; SCRI; FULL +0x24; SNAM into a zeroed local, then +0x54. Returns 1. Completion: 100%.
fn load_activator_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    begin_load_untested(world, form, cx)?;
    each_chunk(world, form, cx, |world, form, cx, chunk| {
        if chunk == SNAM {
            let id = read_id(cx)?;
            world
                .get_mut::<TesActivator>(form)
                .expect("an activator")
                .sound_54 = FormRef::Raw(id);
            return Ok(());
        }
        activator_common_arm(world, form, cx, chunk)
    })?;
    Ok(true)
}

// IDA: 004AE0A0 TESFlora vt+0x1C LoadForm, with no record-type test: 00451210, 0046AB80(0), the chunks while nonzero: the common arms but SNAM; PFPC 4 bytes into a local, then the dword to the produce part's +0x08; PFIG into a zeroed local, then its +0x04. A short PFPC leaves stack bytes in the local; the host has zero. Returns 1. Completion: 100%.
fn load_flora_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    begin_load_untested(world, form, cx)?;
    each_chunk(world, form, cx, |world, form, cx, chunk| {
        match chunk {
            PFPC => {
                let mut local = [0_u8; 4];
                cx.file.read_chunk(&mut local, 4)?;
                world
                    .get_mut::<TesProduceForm>(form)
                    .expect("flora")
                    .chances_08 = local;
            }
            PFIG => {
                let id = read_id(cx)?;
                world
                    .get_mut::<TesProduceForm>(form)
                    .expect("flora")
                    .ingredient_04 = FormRef::Raw(id);
            }
            _ => activator_common_arm(world, form, cx, chunk)?,
        }
        Ok(())
    })?;
    Ok(true)
}

// IDA: 004AE700 TESFurniture vt+0x1C LoadForm, with no record-type test: 00451210, 0046AB80(0), the chunks while nonzero: the common arms but SNAM; MNAM 004510E0 straight into +0x58. Returns 1. Completion: 100%.
fn load_furniture_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    begin_load_untested(world, form, cx)?;
    each_chunk(world, form, cx, |world, form, cx, chunk| {
        if chunk == MNAM {
            let current = world
                .get::<TesFurniture>(form)
                .expect("furniture")
                .marker_flags_58;
            let (flags, _) = cx.file.read_dword(current)?;
            world
                .get_mut::<TesFurniture>(form)
                .expect("furniture")
                .marker_flags_58 = flags;
            return Ok(());
        }
        activator_common_arm(world, form, cx, chunk)
    })?;
    Ok(true)
}

// ---- APPA ----

/// IDA: 004B4910 TESObjectAPPA (0x7C): TESFullName +0x24, TESModel +0x30, TESIcon +0x48, TESScriptableForm +0x54, TESValueForm +0x60, TESWeightForm +0x68, TESQualityForm +0x70, +0x78 byte type.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesApparatus {
    pub type_78: u8,
}

// IDA: 004B4910 TESObjectAPPA_Constructor: 004B2FF0, the seven components in offset order, the eight vtables, +0x04 = 0x13, +0x78 = 0, 0046C2E0. Completion: 100% of the store order.
fn construct_apparatus(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_3FB4)?;
    world.entity_mut(form).insert((
        full_name(0x00A4_3FA0),
        model(0x00A4_3F80),
        icon(0x00A4_3F64),
        scriptable(0x00A4_3F50),
        value(0x00A4_3F30),
        weight(0x00A4_3F1C),
        TesQualityForm {
            original_vtable: 0x00A4_3F08,
            quality_04: 0.0,
        },
        TesApparatus::default(),
    ));
    set_form_type(world, form, 0x13);
    reset_form_components(world, form, APPARATUS_SLOTS)?;
    Ok(form)
}

// IDA: 004B44F0 TESObjectAPPA vt+0x14: +0x78 = 0, a jump to 0046C2E0. Completion: 100%.
fn reset_apparatus(world: &mut World, form: Entity) -> Result<(), GameError> {
    world
        .get_mut::<TesApparatus>(form)
        .expect("an apparatus")
        .type_78 = 0;
    reset_14_components(world, form)
}

// IDA: 004B45C0 TESObjectAPPA vt+0x1C LoadForm: `cmp al, 0x13`, 00451210, 0046AB80(0), the chunks while nonzero: DATA 0046BDA0(+0x78, 1), whose tails are value, weight and quality; EDID; SCRI; MODL, MODB, MODT +0x30; FULL +0x24; ICON +0x48. Returns 1. Completion: 100%.
fn load_apparatus_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if !begin_load(world, form, cx, 0x13)? {
        return Ok(false);
    }
    each_chunk(world, form, cx, |world, form, cx, chunk| match chunk {
        DATA => {
            let mut bytes = [world
                .get::<TesApparatus>(form)
                .expect("an apparatus")
                .type_78];
            load_form_data(world, form, &mut bytes, 1, cx)?;
            world
                .get_mut::<TesApparatus>(form)
                .expect("an apparatus")
                .type_78 = bytes[0];
            Ok(())
        }
        EDID => edid_arm(cx),
        SCRI => load_script_reference(world, form, cx),
        MODL | MODB | MODT => load_model_arm(world, form, cx),
        FULL => full_arm(world, form, cx),
        ICON => icon_arm(world, form, cx),
        _ => Ok(()),
    })?;
    Ok(true)
}

// ---- ARMO, CLOT ----

/// IDA: 004B5140 TESObjectARMO (0xE8): TESFullName +0x24, TESScriptableForm +0x30, TESEnchantableForm +0x3C, TESValueForm +0x4C, TESWeightForm +0x54, TESHealthForm +0x5C, TESBipedModelForm +0x64, +0xE4 word armor rating.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesArmor {
    pub armor_rating_e4: u16,
}

/// IDA: 004B6150 TESObjectCLOT (0xDC), a tag with no fields of its own: TESFullName +0x24, TESScriptableForm +0x30, TESEnchantableForm +0x3C, TESValueForm +0x4C, TESWeightForm +0x54, TESBipedModelForm +0x5C.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesClothing;

// IDA: 004B5140 TESObjectARMO_Constructor: 004B2FF0, the components, the eight vtables, +0x04 = 0x14, +0xE4 = 0, 0046C2E0, +0x48 = 3. Completion: 100% of the store order.
fn construct_armor(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_41BC)?;
    world.entity_mut(form).insert((
        full_name(0x00A4_41A4),
        scriptable(0x00A4_4190),
        enchantable(0x00A4_4178),
        value(0x00A4_4158),
        weight(0x00A4_4144),
        health(0x00A4_412C),
        biped(0x00A4_4118),
        TesArmor::default(),
    ));
    set_form_type(world, form, 0x14);
    reset_form_components(world, form, ARMOR_SLOTS)?;
    world
        .get_mut::<TesEnchantableForm>(form)
        .expect("armor")
        .cast_type_0c = 3;
    Ok(form)
}

// IDA: 004B4C00 TESObjectARMO vt+0x14: +0xE4 = 0, 0046C2E0, +0x48 = 3. Completion: 100%.
fn reset_armor(world: &mut World, form: Entity) -> Result<(), GameError> {
    world
        .get_mut::<TesArmor>(form)
        .expect("armor")
        .armor_rating_e4 = 0;
    reset_14_components(world, form)?;
    world
        .get_mut::<TesEnchantableForm>(form)
        .expect("armor")
        .cast_type_0c = 3;
    Ok(())
}

// IDA: 004B6150 TESObjectCLOT_Constructor: 004B2FF0, the components, the seven vtables, +0x04 = 0x16, 0046C2E0, +0x48 = 3. Completion: 100% of the store order.
fn construct_clothing(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_4644)?;
    world.entity_mut(form).insert((
        full_name(0x00A4_4630),
        scriptable(0x00A4_461C),
        enchantable(0x00A4_4604),
        value(0x00A4_45E4),
        weight(0x00A4_45D0),
        biped(0x00A4_45BC),
        TesClothing,
    ));
    set_form_type(world, form, 0x16);
    reset_form_components(world, form, CLOTHING_SLOTS)?;
    world
        .get_mut::<TesEnchantableForm>(form)
        .expect("clothing")
        .cast_type_0c = 3;
    Ok(form)
}

// IDA: 004B5C40 TESObjectCLOT vt+0x14: 0046C2E0, +0x48 = 3. Completion: 100%.
fn reset_clothing(world: &mut World, form: Entity) -> Result<(), GameError> {
    reset_14_components(world, form)?;
    world
        .get_mut::<TesEnchantableForm>(form)
        .expect("clothing")
        .cast_type_0c = 3;
    Ok(())
}

// IDA: the arms 004B4D90 and 004B5DB0 share: SCRI +0x30 (0046F020), EDID, FULL +0x24, ENAM, ANAM, and every biped chunk through `load_biped_chunk`, whose offsets were read from both. 0046D940's cross cast from a TESModel member (not a base) to TESForm is null, so the owner is `None`.
fn wearable_arm(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
    chunk: u32,
) -> Result<(), GameError> {
    match chunk {
        SCRI => load_script_reference(world, form, cx),
        EDID => edid_arm(cx),
        FULL => full_arm(world, form, cx),
        ENAM => enam_arm(world, form, cx),
        ANAM => anam_arm(world, form, cx),
        _ => {
            let mut held = std::mem::take(
                &mut *world
                    .get_mut::<TesBipedModelForm>(form)
                    .expect("a biped form"),
            );
            let result = load_biped_chunk(world, &mut held, chunk, None, cx);
            *world
                .get_mut::<TesBipedModelForm>(form)
                .expect("a biped form") = held;
            result.map(|_| ())
        }
    }
}

// IDA: 004B4D90 TESObjectARMO vt+0x1C LoadForm: `cmp al, 0x14`, 00451210, 0046AB80(0), the chunks while nonzero: DATA 0046BDA0(+0xE4, 2), whose tails are value, health and weight; the wearable arms. Returns 1. Completion: 100%.
fn load_armor_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if !begin_load(world, form, cx, 0x14)? {
        return Ok(false);
    }
    each_chunk(world, form, cx, |world, form, cx, chunk| {
        if chunk == DATA {
            let mut bytes = world
                .get::<TesArmor>(form)
                .expect("armor")
                .armor_rating_e4
                .to_le_bytes();
            load_form_data(world, form, &mut bytes, 2, cx)?;
            world
                .get_mut::<TesArmor>(form)
                .expect("armor")
                .armor_rating_e4 = u16::from_le_bytes(bytes);
            return Ok(());
        }
        wearable_arm(world, form, cx, chunk)
    })?;
    Ok(true)
}

// IDA: 004B5DB0 TESObjectCLOT vt+0x1C LoadForm: `cmp al, 0x16`, 00451210, 0046AB80(0), the chunks while nonzero: DATA 0046BDA0(0, 0), so only value and weight; the wearable arms. Returns 1. Completion: 100%.
fn load_clothing_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if !begin_load(world, form, cx, 0x16)? {
        return Ok(false);
    }
    each_chunk(world, form, cx, |world, form, cx, chunk| {
        if chunk == DATA {
            return load_form_data(world, form, &mut [], 0, cx);
        }
        wearable_arm(world, form, cx, chunk)
    })?;
    Ok(true)
}

// ---- BOOK ----

/// IDA: 004B59F0 TESObjectBOOK (0x8C): TESFullName +0x24, TESModel +0x30, TESIcon +0x48, TESScriptableForm +0x54, TESEnchantableForm +0x60, TESValueForm +0x70, TESWeightForm +0x78, TESDescription +0x80, +0x88 byte flags, +0x89 byte teaches.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesBook {
    pub flags_88: u8,
    pub teaches_89: u8,
}

// IDA: 004B59F0 TESObjectBOOK_Constructor: 004B2FF0, the components, 0046A400 on +0x80, the nine vtables, +0x04 = 0x15, word +0x88 = 0, +0x89 = 0xFF, 0046C2E0, +0x6C = 0. Completion: 100% of the store order.
fn construct_book(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_4454)?;
    let mut description = TesDescription {
        original_vtable: TES_DESCRIPTION_VTABLE,
        offset_04: 0,
    };
    reset_description(
        &mut description,
        &mut world.resource_mut::<DataHandlerGlobals>(),
    );
    description.original_vtable = 0x00A4_438C;
    world.entity_mut(form).insert((
        full_name(0x00A4_4440),
        model(0x00A4_4420),
        icon(0x00A4_4404),
        scriptable(0x00A4_43F0),
        enchantable(0x00A4_43D8),
        value(0x00A4_43B8),
        weight(0x00A4_43A4),
        description,
        TesBook {
            flags_88: 0,
            teaches_89: 0xFF,
        },
    ));
    set_form_type(world, form, 0x15);
    reset_form_components(world, form, BOOK_SLOTS)?;
    world
        .get_mut::<TesEnchantableForm>(form)
        .expect("a book")
        .cast_type_0c = 0;
    Ok(form)
}

// IDA: 004B53B0 TESObjectBOOK vt+0x14: word +0x88 = 0, +0x89 = 0xFF, 0046C2E0, +0x6C = 0. Completion: 100%.
fn reset_book(world: &mut World, form: Entity) -> Result<(), GameError> {
    *world.get_mut::<TesBook>(form).expect("a book") = TesBook {
        flags_88: 0,
        teaches_89: 0xFF,
    };
    reset_14_components(world, form)?;
    world
        .get_mut::<TesEnchantableForm>(form)
        .expect("a book")
        .cast_type_0c = 0;
    Ok(())
}

// IDA: 004B5440 TESObjectBOOK vt+0x1C LoadForm: `cmp al, 0x15`, 00451210, 0046AB80(0), the chunks while nonzero: DATA 0046BDA0(+0x88, 2), whose tails are value and weight; DESC 0046A390 on +0x80; EDID; SCRI; MODL, MODB, MODT +0x30; FULL +0x24; ANAM; ENAM; ICON +0x48. Returns 1. Completion: 100%.
fn load_book_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if !begin_load(world, form, cx, 0x15)? {
        return Ok(false);
    }
    each_chunk(world, form, cx, |world, form, cx, chunk| match chunk {
        DATA => {
            let book = *world.get::<TesBook>(form).expect("a book");
            let mut bytes = [book.flags_88, book.teaches_89];
            load_form_data(world, form, &mut bytes, 2, cx)?;
            *world.get_mut::<TesBook>(form).expect("a book") = TesBook {
                flags_88: bytes[0],
                teaches_89: bytes[1],
            };
            Ok(())
        }
        DESC => load_description(
            &mut world.get_mut::<TesDescription>(form).expect("a book"),
            cx,
        ),
        EDID => edid_arm(cx),
        SCRI => load_script_reference(world, form, cx),
        MODL | MODB | MODT => load_model_arm(world, form, cx),
        FULL => full_arm(world, form, cx),
        ANAM => anam_arm(world, form, cx),
        ENAM => enam_arm(world, form, cx),
        ICON => icon_arm(world, form, cx),
        _ => Ok(()),
    })?;
    Ok(true)
}

// ---- CONT ----

/// IDA: 004B6BE0 TESObjectCONT (0x7C): TESContainer +0x24, TESFullName +0x34, TESModel +0x40, TESScriptableForm +0x58, TESWeightForm +0x64, +0x6C unset, +0x70 / +0x74 the SNAM / QNAM sounds (raw IDs), +0x78 byte flags.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesContainerObject {
    pub open_sound_70: FormRef,
    pub close_sound_74: FormRef,
    pub flags_78: u8,
}

// IDA: 004B6BE0 TESObjectCONT_Constructor: 004B31D0, 00469690 on +0x24, +0x34 inline, 0046D7E0, 0046EF60, 004705A0, the six vtables, +0x04 = 0x17, +0x78 = +0x70 = +0x74 = 0, 0046C2E0. Completion: 100% of the store order.
fn construct_container(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_4824)?;
    world.entity_mut(form).insert((
        TesContainer {
            original_vtable: 0x00A4_4810,
            flags_04: 0,
            entries_08: Vec::new(),
        },
        full_name(0x00A4_47FC),
        model(0x00A4_47DC),
        scriptable(0x00A4_47C8),
        weight(0x00A4_47B4),
        TesContainerObject::default(),
    ));
    set_form_type(world, form, 0x17);
    reset_form_components(world, form, CONTAINER_SLOTS)?;
    Ok(form)
}

// IDA: 004B6250 TESObjectCONT vt+0x14: +0x78 = 0, +0x70 = +0x74 = 0, a jump to 0046C2E0. Completion: 100%.
fn reset_container(world: &mut World, form: Entity) -> Result<(), GameError> {
    *world
        .get_mut::<TesContainerObject>(form)
        .expect("a container") = TesContainerObject::default();
    reset_14_components(world, form)
}

// IDA: 004B63A0 TESObjectCONT vt+0x1C LoadForm: `cmp al, 0x17`, 00451210, 0046AB80(0), the chunks while nonzero: DATA 0046BDA0(+0x78, 1), whose tail is weight; EDID; SCRI +0x58; MODL, MODB, MODT +0x40; FULL +0x34; QNAM +0x74; SNAM +0x70; CNTO 8 bytes into a zeroed local, 00469500(0) clearing the container's bit 0, 00469750 with {item, count}. Returns 1. Completion: 100%.
fn load_container_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if !begin_load(world, form, cx, 0x17)? {
        return Ok(false);
    }
    each_chunk(world, form, cx, |world, form, cx, chunk| {
        match chunk {
            DATA => {
                let mut bytes = [world
                    .get::<TesContainerObject>(form)
                    .expect("a container")
                    .flags_78];
                load_form_data(world, form, &mut bytes, 1, cx)?;
                world
                    .get_mut::<TesContainerObject>(form)
                    .expect("a container")
                    .flags_78 = bytes[0];
            }
            EDID => edid_arm(cx)?,
            SCRI => load_script_reference(world, form, cx)?,
            MODL | MODB | MODT => load_model_arm(world, form, cx)?,
            FULL => full_arm(world, form, cx)?,
            QNAM => {
                let id = read_id(cx)?;
                world
                    .get_mut::<TesContainerObject>(form)
                    .expect("a container")
                    .close_sound_74 = FormRef::Raw(id);
            }
            SNAM => {
                let id = read_id(cx)?;
                world
                    .get_mut::<TesContainerObject>(form)
                    .expect("a container")
                    .open_sound_70 = FormRef::Raw(id);
            }
            CNTO => {
                let mut local = [0_u8; 8];
                cx.file.read_chunk(&mut local, 8)?;
                let mut container = world.get_mut::<TesContainer>(form).expect("a container");
                container.flags_04 &= 0xFE;
                container.add_entry(
                    u32::from_le_bytes(local[..4].try_into().expect("four bytes")),
                    i32::from_le_bytes(local[4..].try_into().expect("four bytes")),
                );
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(true)
}

// ---- DOOR ----

/// IDA: 004B8550 TESObjectDOOR (0x70): TESFullName +0x24, TESModel +0x30, TESScriptableForm +0x48, +0x54 unset, +0x58 / +0x5C / +0x60 the SNAM / ANAM / BNAM TESSound*, resolved while loading, +0x64 byte flags, +0x68/+0x6C the TNAM list of raw IDs.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesDoor {
    pub open_sound_58: Option<Entity>,
    pub close_sound_5c: Option<Entity>,
    pub loop_sound_60: Option<Entity>,
    pub flags_64: u8,
    pub teleports_68: Vec<FormRef>,
}

// IDA: 004B8550 TESObjectDOOR_Constructor: 004B31D0, +0x24 inline, 0046D7E0, 0046EF60, the four vtables, +0x68 = +0x6C = 0, +0x04 = 0x18, +0x58..+0x64 = 0, 0046C2E0. Completion: 100% of the store order.
fn construct_door(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_4A54)?;
    world.entity_mut(form).insert((
        full_name(0x00A4_4A3C),
        model(0x00A4_4A1C),
        scriptable(0x00A4_4A08),
        TesDoor::default(),
    ));
    set_form_type(world, form, 0x18);
    reset_form_components(world, form, ACTIVATOR_SLOTS)?;
    Ok(form)
}

// IDA: 004B6CD0 TESObjectDOOR vt+0x14: +0x58, +0x5C, +0x60 = 0, +0x64 = 0, a jump to 0046C2E0. The TNAM list stays. Completion: 100%.
fn reset_door(world: &mut World, form: Entity) -> Result<(), GameError> {
    let mut door = world.get_mut::<TesDoor>(form).expect("a door");
    door.open_sound_58 = None;
    door.close_sound_5c = None;
    door.loop_sound_60 = None;
    door.flags_64 = 0;
    reset_14_components(world, form)
}

// IDA: 004B7A20 TESObjectDOOR vt+0x18: the nodes after the head freed, +0x68 = 0, a jump to 0046C2F0. Completion: 100%.
fn destroy_door(world: &mut World, form: Entity) -> Result<(), GameError> {
    world
        .get_mut::<TesDoor>(form)
        .expect("a door")
        .teleports_68
        .clear();
    reset_18_components(world, form)
}

// IDA: the DOOR sound arm: 004510E0 into a zeroed local, 0046BB20(&id, the current file), 0046B250, the TESSound cast. A miss is a silent null.
fn door_sound(world: &World, cx: &mut FormLoadCx<'_, '_>) -> Result<Option<Entity>, GameError> {
    let raw = read_id(cx)?;
    let id = map_file_form_id(world, cx, raw, Some(cx.file.id))?;
    Ok(world
        .resource::<FormsById>()
        .lookup(id)
        .filter(|&found| world.get::<TesSound>(found).is_some()))
}

// IDA: 004B6EA0 TESObjectDOOR vt+0x1C LoadForm: `cmp al, 0x18`, 00451210, 0046AB80(0), the chunks while nonzero: ANAM +0x5C, BNAM +0x60, SNAM +0x58 by `door_sound`; FNAM 1 byte into +0x64; TNAM into a zeroed local, 00446CB0 prepending the raw ID (a zero one adds nothing); SCRI +0x48; EDID; MODL, MODB, MODT +0x30; FULL +0x24. Returns 1. Completion: 100%.
fn load_door_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if !begin_load(world, form, cx, 0x18)? {
        return Ok(false);
    }
    each_chunk(world, form, cx, |world, form, cx, chunk| {
        match chunk {
            ANAM => {
                let sound = door_sound(world, cx)?;
                world
                    .get_mut::<TesDoor>(form)
                    .expect("a door")
                    .close_sound_5c = sound;
            }
            BNAM => {
                let sound = door_sound(world, cx)?;
                world
                    .get_mut::<TesDoor>(form)
                    .expect("a door")
                    .loop_sound_60 = sound;
            }
            SNAM => {
                let sound = door_sound(world, cx)?;
                world
                    .get_mut::<TesDoor>(form)
                    .expect("a door")
                    .open_sound_58 = sound;
            }
            FNAM => {
                let mut byte = [world.get::<TesDoor>(form).expect("a door").flags_64];
                cx.file.read_chunk(&mut byte, 1)?;
                world.get_mut::<TesDoor>(form).expect("a door").flags_64 = byte[0];
            }
            TNAM => {
                let id = read_id(cx)?;
                if id != 0 {
                    world
                        .get_mut::<TesDoor>(form)
                        .expect("a door")
                        .teleports_68
                        .insert(0, FormRef::Raw(id));
                }
            }
            SCRI => load_script_reference(world, form, cx)?,
            EDID => edid_arm(cx)?,
            MODL | MODB | MODT => load_model_arm(world, form, cx)?,
            FULL => full_arm(world, form, cx)?,
            _ => {}
        }
        Ok(())
    })?;
    Ok(true)
}

// ---- LIGH ----

/// IDA: 004B1F90 TESObjectLIGH (0x90): TESFullName +0x24, TESModel +0x30, TESIcon +0x48, TESScriptableForm +0x54, TESWeightForm +0x60, TESValueForm +0x68, the 0x18 bytes of DATA +0x70, +0x88 float fade (FNAM), +0x8C the SNAM sound (raw ID).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct TesLight {
    pub time_70: i32,
    pub radius_74: u32,
    pub color_78: u32,
    pub flags_7c: u32,
    pub falloff_80: f32,
    pub fov_84: f32,
    pub fade_88: f32,
    pub sound_8c: FormRef,
}

impl TesLight {
    /// IDA: N/A the 0x18 bytes at +0x70.
    #[must_use]
    pub fn data_70_bytes(&self) -> [u8; 0x18] {
        let mut bytes = [0_u8; 0x18];
        bytes[0..4].copy_from_slice(&self.time_70.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.radius_74.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.color_78.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.flags_7c.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.falloff_80.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.fov_84.to_le_bytes());
        bytes
    }

    // IDA: N/A the same bytes back.
    fn set_data_70_bytes(&mut self, bytes: &[u8; 0x18]) {
        let word =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
        self.time_70 = word(0) as i32;
        self.radius_74 = word(4);
        self.color_78 = word(8);
        self.flags_7c = word(12);
        self.falloff_80 = f32::from_bits(word(16));
        self.fov_84 = f32::from_bits(word(20));
    }

    // IDA: 004B0C30 / 004B2056 the stores: +0x8C, +0x70..+0x80 = 0, +0x88 = 1.0, +0x84 = A430CC, 90.0 by the bytes.
    fn reset(&mut self) {
        *self = Self {
            fade_88: 1.0,
            fov_84: 90.0,
            ..Self::default()
        };
    }
}

// IDA: 004B1F90 TESObjectLIGH_Constructor: 004B31D0, the components, the seven vtables, +0x04 = 0x1A, the 004B0C30 stores, 0046C2E0. Completion: 100% of the store order.
fn construct_light(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_319C)?;
    let mut light = TesLight::default();
    light.reset();
    world.entity_mut(form).insert((
        full_name(0x00A4_3184),
        model(0x00A4_3164),
        icon(0x00A4_3148),
        scriptable(0x00A4_3134),
        weight(0x00A4_3120),
        value(0x00A4_3100),
        light,
    ));
    set_form_type(world, form, 0x1A);
    reset_form_components(world, form, LIGHT_SLOTS)?;
    Ok(form)
}

// IDA: 004B0C30 TESObjectLIGH vt+0x14: the stores, a jump to 0046C2E0. Completion: 100%.
fn reset_light(world: &mut World, form: Entity) -> Result<(), GameError> {
    world.get_mut::<TesLight>(form).expect("a light").reset();
    reset_14_components(world, form)
}

// IDA: 004B0D10 TESObjectLIGH vt+0x1C LoadForm: `cmp al, 0x1A`, 00451210, 0046AB80(0), +0x84 = +0x80 = 0.0, the chunks while nonzero: DATA 0046BDA0(+0x70, 0x18), whose tails are value and weight, then the test at 004B0EA0: `fcom` with 0.0 and `test ah, 0x44; jp` store only on equal, so -0.0 takes it and a NaN doesn't: +0x84 = 90.0, then +0x80 = 1.0; EDID; SCRI +0x54; FULL +0x24; FNAM 004510E0 straight into +0x88; SNAM +0x8C; ICON +0x48; MODL, MODB, MODT +0x30. Returns 1. Completion: 100%.
fn load_light_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if !begin_load(world, form, cx, 0x1A)? {
        return Ok(false);
    }
    {
        let mut light = world.get_mut::<TesLight>(form).expect("a light");
        light.fov_84 = 0.0;
        light.falloff_80 = 0.0;
    }
    each_chunk(world, form, cx, |world, form, cx, chunk| {
        match chunk {
            DATA => {
                let mut bytes = world
                    .get::<TesLight>(form)
                    .expect("a light")
                    .data_70_bytes();
                load_form_data(world, form, &mut bytes, 0x18, cx)?;
                let mut light = world.get_mut::<TesLight>(form).expect("a light");
                light.set_data_70_bytes(&bytes);
                if light.fov_84 == 0.0 {
                    light.fov_84 = 90.0;
                }
                if light.falloff_80 == 0.0 {
                    light.falloff_80 = 1.0;
                }
            }
            EDID => edid_arm(cx)?,
            SCRI => load_script_reference(world, form, cx)?,
            FULL => full_arm(world, form, cx)?,
            FNAM => {
                let current = world.get::<TesLight>(form).expect("a light").fade_88;
                let (bits, _) = cx.file.read_dword(current.to_bits())?;
                world.get_mut::<TesLight>(form).expect("a light").fade_88 = f32::from_bits(bits);
            }
            SNAM => {
                let id = read_id(cx)?;
                world.get_mut::<TesLight>(form).expect("a light").sound_8c = FormRef::Raw(id);
            }
            ICON => icon_arm(world, form, cx)?,
            MODL | MODB | MODT => load_model_arm(world, form, cx)?,
            _ => {}
        }
        Ok(())
    })?;
    Ok(true)
}

// ---- MISC, KEYM, SLGM, SGST ----

// IDA: 004B9860 TESObjectMISC_Constructor: 004B2FF0, the components, the seven vtables, +0x04 = 0x1B, 0046AB80(1), with no 0046C2E0. KEYM, SLGM and SGST run it, then replace the vtables and the type; here the most-derived ones go in at once. Completion: 100% of the store order.
fn construct_misc_part(
    world: &mut World,
    vtables: [u32; 7],
    form_type: u8,
) -> Result<Entity, GameError> {
    let form = construct_bound(world, vtables[0])?;
    world.entity_mut(form).insert((
        full_name(vtables[1]),
        model(vtables[2]),
        icon(vtables[3]),
        scriptable(vtables[4]),
        value(vtables[5]),
        weight(vtables[6]),
    ));
    set_form_type(world, form, form_type);
    set_initialized(world, form, true);
    Ok(form)
}

// IDA: N/A a helper, with no address of its own.
fn construct_misc(world: &mut World) -> Result<Entity, GameError> {
    construct_misc_part(
        world,
        [
            0x00A4_4CA4,
            0x00A4_4C8C,
            0x00A4_4C6C,
            0x00A4_4C50,
            0x00A4_4C3C,
            0x00A4_4C1C,
            0x00A4_4C08,
        ],
        0x1B,
    )
}

// IDA: 004AF4E0 TESKey_Constructor: 004B9860, the seven vtables, +0x04 = 0x27, 0046AB80(1). Completion: 100% of the store order.
fn construct_key(world: &mut World) -> Result<Entity, GameError> {
    construct_misc_part(
        world,
        [
            0x00A4_2A4C,
            0x00A4_2A34,
            0x00A4_2A14,
            0x00A4_29F8,
            0x00A4_29E4,
            0x00A4_29C4,
            0x00A4_29B0,
        ],
        0x27,
    )
}

// IDA: the arms 004B9600, 004BBDC0 and 004BBA30 share: DATA 0046BDA0(0, 0), whose tails are their value and weight (with SGST's uses first); EDID; SCRI +0x54; MODL, MODB, MODT +0x30; FULL +0x24; ICON +0x48.
fn misc_arm(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
    chunk: u32,
) -> Result<(), GameError> {
    match chunk {
        DATA => load_form_data(world, form, &mut [], 0, cx),
        EDID => edid_arm(cx),
        SCRI => load_script_reference(world, form, cx),
        MODL | MODB | MODT => load_model_arm(world, form, cx),
        FULL => full_arm(world, form, cx),
        ICON => icon_arm(world, form, cx),
        _ => Ok(()),
    }
}

// IDA: 004B9600 vt+0x1C of MISC and KEYM, with no record-type test: 00451210, 0046AB80(0), the chunks while nonzero. Returns 1. Completion: 100%.
fn load_misc_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    begin_load_untested(world, form, cx)?;
    each_chunk(world, form, cx, misc_arm)?;
    Ok(true)
}

/// IDA: 004BBD10 TESSoulGem (0x74): TESObjectMISC, +0x70 byte soul (SOUL), +0x71 byte capacity (SLCP).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesSoulGem {
    pub soul_70: u8,
    pub capacity_71: u8,
}

// IDA: 004BBD10 TESSoulGem_Constructor: 004B9860, the seven vtables, +0x04 = 0x26, +0x70 = +0x71 = 0, 0046C2E0. Completion: 100% of the store order.
fn construct_soul_gem(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_misc_part(
        world,
        [
            0x00A4_56F4,
            0x00A4_56DC,
            0x00A4_56BC,
            0x00A4_56A0,
            0x00A4_568C,
            0x00A4_566C,
            0x00A4_5658,
        ],
        0x26,
    )?;
    world.entity_mut(form).insert(TesSoulGem::default());
    reset_form_components(world, form, MISC_SLOTS)?;
    Ok(form)
}

// IDA: 004BBC70 TESSoulGem vt+0x14: +0x70 = +0x71 = 0, a jump to 0046C2E0. Completion: 100%.
fn reset_soul_gem(world: &mut World, form: Entity) -> Result<(), GameError> {
    *world.get_mut::<TesSoulGem>(form).expect("a soul gem") = TesSoulGem::default();
    reset_14_components(world, form)
}

// IDA: 004BBDC0 TESSoulGem vt+0x1C LoadForm: `cmp al, 0x26`, 00451210, 0046AB80(0), the chunks while nonzero: `misc_arm`; SOUL 1 byte into +0x70 and SLCP 1 byte into +0x71, each only for a chunk of exactly 1 byte. Returns 1. Completion: 100%.
fn load_soul_gem_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if !begin_load(world, form, cx, 0x26)? {
        return Ok(false);
    }
    each_chunk(world, form, cx, |world, form, cx, chunk| {
        match chunk {
            SOUL => {
                if cx.file.chunk_size()? == 1 {
                    let mut byte = [0_u8];
                    cx.file.read_chunk(&mut byte, 1)?;
                    world
                        .get_mut::<TesSoulGem>(form)
                        .expect("a soul gem")
                        .soul_70 = byte[0];
                }
            }
            SLCP => {
                if cx.file.chunk_size()? == 1 {
                    let mut byte = [0_u8];
                    cx.file.read_chunk(&mut byte, 1)?;
                    world
                        .get_mut::<TesSoulGem>(form)
                        .expect("a soul gem")
                        .capacity_71 = byte[0];
                }
            }
            _ => misc_arm(world, form, cx, chunk)?,
        }
        Ok(())
    })?;
    Ok(true)
}

// ---- STAT ----

/// IDA: 004B9AA0 TESObjectSTAT (0x3C), a tag with no fields of its own: TESModel +0x24.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesStatic;

// IDA: 004B9AA0 TESObjectSTAT_Constructor: 004B2FF0, 0046D7E0 on +0x24, the two vtables, +0x04 = 0x1C, with no 0046C2E0. Completion: 100% of the store order.
fn construct_static(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_4DEC)?;
    world
        .entity_mut(form)
        .insert((model(0x00A4_4DC8), TesStatic));
    set_form_type(world, form, 0x1C);
    Ok(form)
}

// IDA: 004B9B30 TESObjectSTAT vt+0x1C LoadForm: `cmp al, 0x1C`, 00451210, no 0046AB80, the chunks while nonzero: EDID; MODL, MODB, MODT +0x24; DMTL 004B3460, which no shipped STAT has (scanned: Data's eleven plugins), so a host stop. Returns 1. Completion: 100% but DMTL.
// TODO: [ ] 004B9BE2 DMTL: 004B3460 (the +0x24 hash list and the 004B3400 map).
fn load_static_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != 0x1C {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    each_chunk(world, form, cx, |world, form, cx, chunk| match chunk {
        EDID => edid_arm(cx),
        MODL | MODB | MODT => load_model_arm(world, form, cx),
        DMTL => Err(GameError::unclassified(
            0x004B9BE2,
            "004B9BE2: 004B3460, the STAT DMTL chunk",
        )),
        _ => Ok(()),
    })?;
    Ok(true)
}

// ---- GRAS ----

/// IDA: 004AF040 TESGrass (0x5C): TESModel +0x24, the 0x20 bytes of DATA at +0x3C.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct TesGrass {
    pub density_3c: u8,
    pub min_slope_3d: u8,
    pub max_slope_3e: u8,
    pub unset_3f: u8,
    pub water_distance_40: u16,
    pub unset_42: u16,
    pub water_type_44: u32,
    pub position_range_48: f32,
    pub height_range_4c: f32,
    pub color_range_50: f32,
    pub wave_period_54: f32,
    pub flags_58: u8,
    pub unset_59: [u8; 3],
}

impl TesGrass {
    /// IDA: N/A the 0x20 bytes at +0x3C.
    #[must_use]
    pub fn data_3c_bytes(&self) -> [u8; 0x20] {
        let mut bytes = [0_u8; 0x20];
        bytes[0] = self.density_3c;
        bytes[1] = self.min_slope_3d;
        bytes[2] = self.max_slope_3e;
        bytes[3] = self.unset_3f;
        bytes[4..6].copy_from_slice(&self.water_distance_40.to_le_bytes());
        bytes[6..8].copy_from_slice(&self.unset_42.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.water_type_44.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.position_range_48.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.height_range_4c.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.color_range_50.to_le_bytes());
        bytes[24..28].copy_from_slice(&self.wave_period_54.to_le_bytes());
        bytes[28] = self.flags_58;
        bytes[29..32].copy_from_slice(&self.unset_59);
        bytes
    }

    // IDA: N/A the same bytes back.
    fn set_data_3c_bytes(&mut self, bytes: &[u8; 0x20]) {
        let word =
            |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
        self.density_3c = bytes[0];
        self.min_slope_3d = bytes[1];
        self.max_slope_3e = bytes[2];
        self.unset_3f = bytes[3];
        self.water_distance_40 = u16::from_le_bytes([bytes[4], bytes[5]]);
        self.unset_42 = u16::from_le_bytes([bytes[6], bytes[7]]);
        self.water_type_44 = word(8);
        self.position_range_48 = f32::from_bits(word(12));
        self.height_range_4c = f32::from_bits(word(16));
        self.color_range_50 = f32::from_bits(word(20));
        self.wave_period_54 = f32::from_bits(word(24));
        self.flags_58 = bytes[28];
        self.unset_59 = [bytes[29], bytes[30], bytes[31]];
    }

    // IDA: 004AEDC0's stores: +0x48 = A427E4 32.0, +0x3C = 0x1E, +0x3D = 0, +0x4C = A3D9A4 0.2, +0x3E = 0x5A, word +0x40 = 0, +0x50 = A3D65C 0.5, +0x44 = 0, +0x58 = 0, +0x54 = A31C80 10.0 (bytes read from the exe). +0x3F, +0x42 and +0x59.. aren't stored.
    fn reset(&mut self) {
        self.position_range_48 = 32.0;
        self.density_3c = 0x1E;
        self.min_slope_3d = 0;
        self.height_range_4c = f32::from_bits(0x3E4C_CCCD);
        self.max_slope_3e = 0x5A;
        self.water_distance_40 = 0;
        self.color_range_50 = 0.5;
        self.water_type_44 = 0;
        self.flags_58 = 0;
        self.wave_period_54 = 10.0;
    }
}

// IDA: 004AF040 TESGrass_Constructor: 004B2FF0, 0046D7E0 on +0x24, the two vtables, +0x04 = 0x1D, the 004AEDC0 stores, 0046C2E0. Completion: 100% of the store order.
fn construct_grass(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_280C)?;
    let mut grass = TesGrass::default();
    grass.reset();
    world.entity_mut(form).insert((model(0x00A4_27EC), grass));
    set_form_type(world, form, 0x1D);
    reset_form_components(world, form, MODEL_ONLY_SLOTS)?;
    Ok(form)
}

// IDA: 004AEDC0 TESGrass vt+0x14: the stores, a jump to 0046C2E0. Completion: 100%.
fn reset_grass(world: &mut World, form: Entity) -> Result<(), GameError> {
    world.get_mut::<TesGrass>(form).expect("grass").reset();
    reset_14_components(world, form)
}

// IDA: 004AF2A0 TESGrass vt+0x1C LoadForm: `cmp al, 0x1D`; the eight dwords of +0x3C cleared before 00451210; 0046AB80(0); the chunks while nonzero: DATA 0046BDA0(+0x3C, 0x20); EDID; MODL, MODB, MODT +0x24. Returns 1. Completion: 100%.
fn load_grass_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != 0x1D {
        return Ok(false);
    }
    *world.get_mut::<TesGrass>(form).expect("grass") = TesGrass::default();
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    each_chunk(world, form, cx, |world, form, cx, chunk| match chunk {
        DATA => {
            let mut bytes = world.get::<TesGrass>(form).expect("grass").data_3c_bytes();
            load_form_data(world, form, &mut bytes, 0x20, cx)?;
            world
                .get_mut::<TesGrass>(form)
                .expect("grass")
                .set_data_3c_bytes(&bytes);
            Ok(())
        }
        EDID => edid_arm(cx),
        MODL | MODB | MODT => load_model_arm(world, form, cx),
        _ => Ok(()),
    })?;
    Ok(true)
}

// ---- TREE ----

/// IDA: NiTArray<uint> (0x10): vtable, +0x04 data (null with no capacity), +0x08 word capacity, +0x0A word size (first free), +0x0C word occupied, +0x0E word grow by.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NiTArrayU32 {
    pub original_vtable: u32,
    pub data_04: Option<Vec<u32>>,
    pub capacity_08: u16,
    pub size_0a: u16,
    pub occupied_0c: u16,
    pub grow_by_0e: u16,
}

impl NiTArrayU32 {
    // IDA: 0065DD90: each of the +0x0A slots cleared, +0x0C = +0x0A = 0. Completion: 100%.
    fn clear(&mut self) {
        if let Some(data) = &mut self.data_04 {
            for slot in data.iter_mut().take(usize::from(self.size_0a)) {
                *slot = 0;
            }
        }
        self.occupied_0c = 0;
        self.size_0a = 0;
    }

    // IDA: 004E4A10 NiTArray_SetSize(capacity): equal returns. Below +0x0A, the slots past it are cleared with +0x0C counted down and +0x0A takes it. +0x08 = capacity; zero leaves a null array; otherwise a new FormHeap block holds the old slots, then zeros. Completion: 100%.
    fn set_size(&mut self, capacity: u32) {
        if capacity == u32::from(self.capacity_08) {
            return;
        }
        if capacity < u32::from(self.size_0a) {
            let data = self.data_04.as_mut().expect("a size has an array");
            for slot in &mut data[capacity as usize..usize::from(self.size_0a)] {
                if *slot != 0 {
                    *slot = 0;
                    self.occupied_0c = self.occupied_0c.wrapping_sub(1);
                }
            }
            self.size_0a = capacity as u16;
        }
        self.capacity_08 = capacity as u16;
        if capacity == 0 {
            self.data_04 = None;
            return;
        }
        let mut data = vec![0_u32; usize::from(capacity as u16)];
        if let Some(old) = &self.data_04 {
            data[..usize::from(self.size_0a)].copy_from_slice(&old[..usize::from(self.size_0a)]);
        }
        self.data_04 = Some(data);
    }

    // IDA: 0042BB90 NiTArray_AddAtIndex(index, &item): past the size, +0x0A = index + 1 and a nonzero item counts; within it, a nonzero item over a zero slot counts, and a zero item over a nonzero slot counts down; the item is stored in every arm. Completion: 100%.
    fn add_at_index(&mut self, index: u32, item: u32) {
        let at = index as usize;
        let data = self.data_04.as_mut().expect("the caller sized the array");
        if index >= u32::from(self.size_0a) {
            self.size_0a = (index + 1) as u16;
            if item != 0 {
                self.occupied_0c = self.occupied_0c.wrapping_add(1);
            }
        } else if item != 0 {
            if data[at] == 0 {
                self.occupied_0c = self.occupied_0c.wrapping_add(1);
            }
        } else if data[at] != 0 {
            self.occupied_0c = self.occupied_0c.wrapping_sub(1);
        }
        data[at] = item;
    }
}

/// IDA: 00B3FC80 / 00B3FC84, the x and y words the TREE and TESWorldSpace constructors copy. P12-RUN-004 (the C++'s) read both as 0 at 004067C7. Absent is that 0.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TreeGlobals {
    pub x_b3fc80: u32,
    pub y_b3fc84: u32,
}

/// IDA: 004B9DA0 TESObjectTREE (0x80): TESBoundTreeObject, TESModel +0x24, TESIcon +0x3C, NiTArray<uint> +0x48 (SNAM), CNAM +0x58 (0x20 bytes, of which the constructor leaves +0x6C unset: host zero), BNAM +0x78 (8 bytes).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesTree {
    pub seeds_48: NiTArrayU32,
    pub parameters_58: [u32; 8],
    pub billboard_78: [u32; 2],
}

// IDA: 004B9DA0 TESObjectTREE_Constructor: 004B31F0, 0046D7E0 on +0x24, 0046FFD0 on +0x3C, the four vtables, the array's +0x08 = 0, +0x0E = 1, +0x0A = +0x0C = 0, +0x04 = 0; +0x58 = A45128 2.5, +0x04 = 0x1E, +0x5C = A31E2C 5.0, +0x60 = A44F70 85.0, +0x64 = A3D65C 0.5, +0x68 = A41724 0.7, +0x70 = +0x74 = 1.0, +0x78 and +0x7C the two globals, with no 0046C2E0 (constants' bytes read from the exe). Completion: 100% of the store order.
fn construct_tree(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_4FB4)?;
    let globals = world
        .get_resource::<TreeGlobals>()
        .copied()
        .unwrap_or_default();
    world.entity_mut(form).insert((
        model(0x00A4_4F94),
        icon(0x00A4_4F78),
        TesTree {
            seeds_48: NiTArrayU32 {
                original_vtable: 0x00A4_4F6C,
                grow_by_0e: 1,
                ..NiTArrayU32::default()
            },
            parameters_58: [
                2.5_f32.to_bits(),
                5.0_f32.to_bits(),
                85.0_f32.to_bits(),
                0.5_f32.to_bits(),
                0x3F33_3333,
                0,
                1.0_f32.to_bits(),
                1.0_f32.to_bits(),
            ],
            billboard_78: [globals.x_b3fc80, globals.y_b3fc84],
        },
    ));
    set_form_type(world, form, 0x1E);
    Ok(form)
}

// IDA: 004BAD20 TESObjectTREE vt+0x1C LoadForm: `cmp al, 0x1E`, 00451210, no 0046AB80, the chunks while nonzero: CNAM 0x20 bytes into +0x58 and BNAM 8 into +0x78, each only for a chunk of exactly that size; EDID; MODL, MODB, MODT +0x24; ICON +0x3C; SNAM, for a nonzero size that's a multiple of 4: a cleared FormHeap block read whole, 0065DD90, then each nonzero dword appended (past the capacity, 004E4A10(+0x0A + grow), then 0042BB90), and the block freed. Returns 1. Completion: 100%.
fn load_tree_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != 0x1E {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    each_chunk(world, form, cx, |world, form, cx, chunk| {
        match chunk {
            CNAM => {
                if cx.file.chunk_size()? == 0x20 {
                    let mut bytes = [0_u8; 0x20];
                    cx.file.read_chunk(&mut bytes, 0x20)?;
                    let mut tree = world.get_mut::<TesTree>(form).expect("a tree");
                    for (slot, word) in tree.parameters_58.iter_mut().zip(bytes.as_chunks::<4>().0)
                    {
                        *slot = u32::from_le_bytes(*word);
                    }
                }
            }
            BNAM => {
                if cx.file.chunk_size()? == 8 {
                    let mut bytes = [0_u8; 8];
                    cx.file.read_chunk(&mut bytes, 8)?;
                    let mut tree = world.get_mut::<TesTree>(form).expect("a tree");
                    for (slot, word) in tree.billboard_78.iter_mut().zip(bytes.as_chunks::<4>().0) {
                        *slot = u32::from_le_bytes(*word);
                    }
                }
            }
            EDID => edid_arm(cx)?,
            MODL | MODB | MODT => load_model_arm(world, form, cx)?,
            ICON => icon_arm(world, form, cx)?,
            SNAM => {
                let size = cx.file.chunk_size()?;
                if size != 0 && size & 3 == 0 {
                    let mut bytes = vec![0_u8; size as usize];
                    cx.file.read_chunk(&mut bytes, size)?;
                    let mut tree = world.get_mut::<TesTree>(form).expect("a tree");
                    let seeds = &mut tree.seeds_48;
                    seeds.clear();
                    for word in bytes.as_chunks::<4>().0 {
                        let seed = u32::from_le_bytes(*word);
                        if seed == 0 {
                            continue;
                        }
                        let index = u32::from(seeds.size_0a);
                        if index >= u32::from(seeds.capacity_08) {
                            seeds.set_size(index + u32::from(seeds.grow_by_0e));
                        }
                        seeds.add_at_index(index, seed);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    Ok(true)
}

// ---- WEAP ----

/// IDA: 004BB610 TESObjectWEAP (0xA0): TESFullName +0x24, TESModel +0x30, TESIcon +0x48, TESScriptableForm +0x54, TESEnchantableForm +0x60, TESValueForm +0x70, TESWeightForm +0x78, TESHealthForm +0x80, TESAttackDamageForm +0x88, the 0x10 bytes of DATA at +0x90.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesWeapon {
    pub data_90: [u8; 0x10],
}

// IDA: 004BB610 TESObjectWEAP_Constructor: 004B2FF0, the components, the ten vtables, +0x04 = 0x21, +0x90..+0x9C = 0, 0046C2E0, +0x6C = 2. Completion: 100% of the store order.
fn construct_weapon(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_5354)?;
    world.entity_mut(form).insert((
        full_name(0x00A4_533C),
        model(0x00A4_531C),
        icon(0x00A4_5300),
        scriptable(0x00A4_52EC),
        enchantable(0x00A4_52D4),
        value(0x00A4_52B4),
        weight(0x00A4_52A0),
        health(0x00A4_5288),
        damage(0x00A4_5270),
        TesWeapon::default(),
    ));
    set_form_type(world, form, 0x21);
    reset_form_components(world, form, WEAPON_SLOTS)?;
    world
        .get_mut::<TesEnchantableForm>(form)
        .expect("a weapon")
        .cast_type_0c = 2;
    Ok(form)
}

// IDA: 004BB0F0 TESObjectWEAP vt+0x14: +0x90..+0x9C = 0, 0046C2E0, +0x6C = 2. Completion: 100%.
fn reset_weapon(world: &mut World, form: Entity) -> Result<(), GameError> {
    world.get_mut::<TesWeapon>(form).expect("a weapon").data_90 = [0; 0x10];
    reset_14_components(world, form)?;
    world
        .get_mut::<TesEnchantableForm>(form)
        .expect("a weapon")
        .cast_type_0c = 2;
    Ok(())
}

// IDA: 004BB150 TESObjectWEAP vt+0x1C LoadForm: `cmp al, 0x21`, 00451210, 0046AB80(0), the chunks while nonzero: DATA 0046BDA0(+0x90, 0x10), whose tails are value, health, weight and attack damage; EDID; SCRI +0x54; MODL, MODB, MODT +0x30; FULL +0x24; ANAM; ENAM; ICON +0x48. Then `cmp byte +0x90, 6; jl`: a type of 6 or more, signed, warns A4523C with vt+0xD4 and the ID, and clears the byte. Returns 1. Completion: 100%.
fn load_weapon_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if !begin_load(world, form, cx, 0x21)? {
        return Ok(false);
    }
    each_chunk(world, form, cx, |world, form, cx, chunk| match chunk {
        DATA => {
            let mut bytes = world.get::<TesWeapon>(form).expect("a weapon").data_90;
            load_form_data(world, form, &mut bytes, 0x10, cx)?;
            world.get_mut::<TesWeapon>(form).expect("a weapon").data_90 = bytes;
            Ok(())
        }
        EDID => edid_arm(cx),
        SCRI => load_script_reference(world, form, cx),
        MODL | MODB | MODT => load_model_arm(world, form, cx),
        FULL => full_arm(world, form, cx),
        ANAM => anam_arm(world, form, cx),
        ENAM => enam_arm(world, form, cx),
        ICON => icon_arm(world, form, cx),
        _ => Ok(()),
    })?;
    if world.get::<TesWeapon>(form).expect("a weapon").data_90[0] as i8 >= 6 {
        let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
        let name = latin1(&editor_id_of(world, form));
        cx.file.services.warn(format!(
            "Clearing invalid type on weapon '{name}' ({id:08X})."
        ));
        world.get_mut::<TesWeapon>(form).expect("a weapon").data_90[0] = 0;
    }
    Ok(true)
}

// ---- AMMO ----

/// IDA: 004A9430 TESAmmo (0x84): TESFullName +0x24, TESModel +0x30, TESIcon +0x48, TESEnchantableForm +0x54, TESValueForm +0x64, TESWeightForm +0x6C, TESAttackDamageForm +0x74, the 8 bytes of DATA at +0x7C.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesAmmo {
    pub data_7c: [u8; 8],
}

// IDA: 004A9430 TESAmmo_Constructor: 004B2FF0, the components, the eight vtables, +0x04 = 0x22, +0x7C = +0x80 = 0, 0046C2E0, +0x60 = 2. Completion: 100% of the store order.
fn construct_ammo(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_068C)?;
    world.entity_mut(form).insert((
        full_name(0x00A4_0678),
        model(0x00A4_0658),
        icon(0x00A4_063C),
        enchantable(0x00A4_0624),
        value(0x00A4_0604),
        weight(0x00A4_05F0),
        damage(0x00A4_05D8),
        TesAmmo::default(),
    ));
    set_form_type(world, form, 0x22);
    reset_form_components(world, form, AMMO_SLOTS)?;
    world
        .get_mut::<TesEnchantableForm>(form)
        .expect("ammo")
        .cast_type_0c = 2;
    Ok(form)
}

// IDA: 004A8FE0 TESAmmo vt+0x14: +0x7C = +0x80 = 0, 0046C2E0, +0x60 = 2. Completion: 100%.
fn reset_ammo(world: &mut World, form: Entity) -> Result<(), GameError> {
    world.get_mut::<TesAmmo>(form).expect("ammo").data_7c = [0; 8];
    reset_14_components(world, form)?;
    world
        .get_mut::<TesEnchantableForm>(form)
        .expect("ammo")
        .cast_type_0c = 2;
    Ok(())
}

// IDA: 004A9030 TESAmmo vt+0x1C LoadForm: `cmp al, 0x22`, 00451210, 0046AB80(0), the chunks while nonzero: DATA 0046BDA0(+0x7C, 8), whose tails are value, weight and attack damage; EDID; MODL, MODB, MODT +0x30; FULL +0x24; ANAM; ENAM; ICON +0x48. No SCRI. Returns 1. Completion: 100%.
fn load_ammo_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if !begin_load(world, form, cx, 0x22)? {
        return Ok(false);
    }
    each_chunk(world, form, cx, |world, form, cx, chunk| match chunk {
        DATA => {
            let mut bytes = world.get::<TesAmmo>(form).expect("ammo").data_7c;
            load_form_data(world, form, &mut bytes, 8, cx)?;
            world.get_mut::<TesAmmo>(form).expect("ammo").data_7c = bytes;
            Ok(())
        }
        EDID => edid_arm(cx),
        MODL | MODB | MODT => load_model_arm(world, form, cx),
        FULL => full_arm(world, form, cx),
        ANAM => anam_arm(world, form, cx),
        ENAM => enam_arm(world, form, cx),
        ICON => icon_arm(world, form, cx),
        _ => Ok(()),
    })?;
    Ok(true)
}

// ---- SBSP ----

/// IDA: 004BCA00 TESSubSpace (0x30): TESBoundObject, +0x24/+0x26/+0x28 word extents, +0x2C float radius.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct TesSubSpace {
    pub extents_24: [u16; 3],
    pub radius_2c: f32,
}

// IDA: the constants 004BC531..004BC55C store: 0x190, 0x190, 0xC8 and __CIsqrt(A45A50, 90000.0 by the bytes), 300.0.
const DEFAULT_SUB_SPACE: TesSubSpace = TesSubSpace {
    extents_24: [0x190, 0x190, 0xC8],
    radius_2c: 300.0,
};

// IDA: 004BCA00 TESSubSpace_Constructor: 004B2FF0, the vtable, +0x04 = 0x29, the stores, 0046C2E0. Completion: 100% of the store order.
fn construct_sub_space(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_592C)?;
    world.entity_mut(form).insert(DEFAULT_SUB_SPACE);
    set_form_type(world, form, 0x29);
    reset_form_components(world, form, &[])?;
    Ok(form)
}

// IDA: 004BC530 TESSubSpace vt+0x14: the stores, a jump to 0046C2E0. Completion: 100%.
fn reset_sub_space(world: &mut World, form: Entity) -> Result<(), GameError> {
    *world.get_mut::<TesSubSpace>(form).expect("a subspace") = DEFAULT_SUB_SPACE;
    reset_14_components(world, form)
}

// IDA: 004BC68F..004BC6F0 at 24 bits: `fild` each word, `fmul` A2FAA0 0.5 into a stored float, then aa, bb, their sum, cc, the sum, each rounded to float; `fstp` and `fld` the float; __CIsqrt. Completion: 100%.
fn sub_space_radius(extents: [u16; 3]) -> f32 {
    let half = |extent: u16| (f64::from(extent) * 0.5) as f32;
    let (a, b, c) = (half(extents[0]), half(extents[1]), half(extents[2]));
    let sum = a * a + b * b;
    let total = sum + c * c;
    total.sqrt()
}

// IDA: 004BC570 TESSubSpace vt+0x1C LoadForm: `cmp al, 0x29`, 00451210, 0046AB80(0), the chunks while nonzero: EDID; DNAM 0x0C bytes into a local, each float `fistp` under chop, the low word stored. Then +0x2C from the extents, always. A short DNAM truncates stack bytes in the original; the host has zero. Returns 1. Completion: 100%.
fn load_sub_space_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if !begin_load(world, form, cx, 0x29)? {
        return Ok(false);
    }
    each_chunk(world, form, cx, |world, form, cx, chunk| {
        match chunk {
            EDID => edid_arm(cx)?,
            DNAM => {
                let mut bytes = [0_u8; 0x0C];
                cx.file.read_chunk(&mut bytes, 0x0C)?;
                let mut space = world.get_mut::<TesSubSpace>(form).expect("a subspace");
                for (extent, word) in space.extents_24.iter_mut().zip(bytes.as_chunks::<4>().0) {
                    let size = f32::from_le_bytes(*word);
                    *extent = oblivion_core::x87::ftol(size) as u16;
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    let mut space = world.get_mut::<TesSubSpace>(form).expect("a subspace");
    space.radius_2c = sub_space_radius(space.extents_24);
    Ok(true)
}

// ---- LVLI, LVLC ----

/// IDA: 004B00B0 TESLevItem (0x34), a tag with no fields of its own: TESLeveledList +0x24.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesLevItem;

/// IDA: 004AF620 TESLevCreature (0x44): TESBoundAnimObject, TESLeveledList +0x24, TESScriptableForm +0x34, +0x40 the TNAM template (raw ID).
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesLevCreature {
    pub template_40: FormRef,
}

// IDA: the arms 004B0160 and 004AF730 share: LVLD, read with size 0 into a local byte (only the low byte is cleared; a longer chunk runs up the stack in the original, and the host reads the first byte): bit 7 through 0046CB10, the rest 0046CAF0 into +0x0C. LVLF, size 0 into +0x0D, past which is another field: a host stop. LVLO 0x0C bytes over {0, 0, word 1}, then 0046CBF0(level, count, form).
fn leveled_arm(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
    chunk: u32,
) -> Result<bool, GameError> {
    match chunk {
        LVLD => {
            let size = cx.file.chunk_size()? as usize;
            let mut local = vec![0_u8; size.max(1)];
            cx.file.read_chunk(&mut local, 0)?;
            let mut list = world
                .get_mut::<TesLeveledList>(form)
                .expect("a leveled list");
            if local[0] & 0x80 != 0 {
                list.flags_0d |= 1;
            } else {
                list.flags_0d &= 0xFE;
            }
            list.chance_none_0c = local[0] & 0x7F;
        }
        LVLF => {
            if cx.file.chunk_size()? > 1 {
                return Err(GameError::unclassified(
                    0,
                    "LVLF past one byte overruns TESLeveledList +0x0D",
                ));
            }
            let mut list = world
                .get_mut::<TesLeveledList>(form)
                .expect("a leveled list");
            let mut byte = [list.flags_0d];
            cx.file.read_chunk(&mut byte, 0)?;
            list.flags_0d = byte[0];
        }
        LVLO => {
            let mut local = [0_u8; 0x0C];
            local[8] = 1;
            cx.file.read_chunk(&mut local, 0x0C)?;
            let word =
                |at: usize| u32::from_le_bytes(local[at..at + 4].try_into().expect("four bytes"));
            world
                .get_mut::<TesLeveledList>(form)
                .expect("a leveled list")
                .add_entry(word(0) as u16, word(8) as i16, word(4));
        }
        _ => return Ok(false),
    }
    Ok(true)
}

// IDA: 004B00B0 TESLevItem_Constructor: 004B2FF0, 0046CB80 on +0x24, the two vtables, +0x04 = 0x2B, 0046C2E0. Completion: 100% of the store order.
fn construct_lev_item(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_2DC4)?;
    world.entity_mut(form).insert((
        TesLeveledList {
            original_vtable: 0x00A4_2DA8,
            ..TesLeveledList::default()
        },
        TesLevItem,
    ));
    set_form_type(world, form, 0x2B);
    reset_form_components(world, form, LEV_ITEM_SLOTS)?;
    Ok(form)
}

// IDA: 004B0160 TESLevItem vt+0x1C LoadForm: `cmp al, 0x2B`, 00451210, no 0046AB80 yet, a do-while on 0044FEA0 with no test of the chunk type: `leveled_arm`; EDID; DATA 0046BDA0 into a zeroed byte local with size 1, then 0046CB30(its != 0) on +0x0D bit 1. Then 0046AB80(0). Returns 1. Completion: 100%.
fn load_lev_item_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != 0x2B {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    every_chunk(world, form, cx, |world, form, cx, chunk| {
        if leveled_arm(world, form, cx, chunk)? {
            return Ok(());
        }
        match chunk {
            EDID => edid_arm(cx)?,
            DATA => {
                let mut local = [0_u8];
                load_form_data(world, form, &mut local, 1, cx)?;
                let mut list = world
                    .get_mut::<TesLeveledList>(form)
                    .expect("a leveled list");
                if local[0] != 0 {
                    list.flags_0d |= 2;
                } else {
                    list.flags_0d &= 0xFD;
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    set_initialized(world, form, false);
    Ok(true)
}

// IDA: 004AF620 TESLevCreature_Constructor: 004B31D0, 0046CB80 on +0x24, 0046EF60 on +0x34, the three vtables, +0x04 = 0x25, +0x40 = 0, 0046C2E0, 0046AB80(1). Completion: 100% of the store order.
fn construct_lev_creature(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_bound(world, 0x00A4_2B9C)?;
    world.entity_mut(form).insert((
        TesLeveledList {
            original_vtable: 0x00A4_2B84,
            ..TesLeveledList::default()
        },
        scriptable(0x00A4_2B70),
        TesLevCreature::default(),
    ));
    set_form_type(world, form, 0x25);
    reset_form_components(world, form, LEV_CREATURE_SLOTS)?;
    set_initialized(world, form, true);
    Ok(form)
}

// IDA: 004AF610 TESLevCreature vt+0x14: +0x40 = 0, a jump to 0046C2E0. Completion: 100%.
fn reset_lev_creature(world: &mut World, form: Entity) -> Result<(), GameError> {
    world
        .get_mut::<TesLevCreature>(form)
        .expect("a list")
        .template_40 = FormRef::Raw(0);
    reset_14_components(world, form)
}

// IDA: 004AF730 TESLevCreature vt+0x1C LoadForm: `cmp al, 0x25`, 00451210, a do-while on 0044FEA0 with no test of the chunk type: `leveled_arm`; EDID; SCRI +0x34; TNAM into a zeroed local, +0x40. Then 0046AB80(0). No DATA arm. Returns 1. Completion: 100%.
fn load_lev_creature_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if cx.file.record_form_type()? as u8 != 0x25 {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    every_chunk(world, form, cx, |world, form, cx, chunk| {
        if leveled_arm(world, form, cx, chunk)? {
            return Ok(());
        }
        match chunk {
            EDID => edid_arm(cx)?,
            SCRI => load_script_reference(world, form, cx)?,
            TNAM => {
                let id = read_id(cx)?;
                world
                    .get_mut::<TesLevCreature>(form)
                    .expect("a list")
                    .template_40 = FormRef::Raw(id);
            }
            _ => {}
        }
        Ok(())
    })?;
    set_initialized(world, form, false);
    Ok(true)
}

// ---- effect items, for INGR, ALCH and SGST ----

// ---- INGR, ALCH ----

/// IDA: 00412B50 MagicItemObject with 004198F0 IngredientItem or 00412BE0 AlchemyItem (0x80): TESBoundObject, TESFullName +0x24, EffectItemList +0x30, TESModel +0x40, TESIcon +0x58, TESScriptableForm +0x64, TESWeightForm +0x70, ENIT +0x78: s32 value -1, byte flags +0x7C, of which the constructors store only those, so +0x7D.. stay unset until a read.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesMagicObject {
    pub value_78: i32,
    pub flags_7c: u8,
    pub rest_7d: Option<[u8; 3]>,
}

// IDA: the shared stores of 004198F0 and 00412BE0: 00412B50 (004B2FF0, 00412AE0: TESFullName, 00414A00), 0046D7E0 on +0x40, 0046FFD0 on +0x58, 0046EF60 on +0x64, 004705A0 on +0x70, the seven vtables, the type, +0x78 = -1, +0x7C = 0, 0046C2E0. Completion: 100% of the store order.
fn construct_magic_object(
    world: &mut World,
    vtables: [u32; 7],
    form_type: u8,
) -> Result<Entity, GameError> {
    let form = construct_bound(world, vtables[0])?;
    world.entity_mut(form).insert((
        full_name(vtables[1]),
        EffectItemList {
            original_vtable: vtables[2],
            ..EffectItemList::default()
        },
        model(vtables[3]),
        icon(vtables[4]),
        scriptable(vtables[5]),
        weight(vtables[6]),
        TesMagicObject {
            value_78: -1,
            flags_7c: 0,
            rest_7d: None,
        },
    ));
    set_form_type(world, form, form_type);
    reset_form_components(world, form, MAGIC_OBJECT_SLOTS)?;
    Ok(form)
}

// IDA: N/A a helper, with no address of its own.
fn construct_ingredient(world: &mut World) -> Result<Entity, GameError> {
    construct_magic_object(
        world,
        [
            0x00A3_3F5C,
            0x00A3_3F14,
            0x00A3_3F00,
            0x00A3_3EE0,
            0x00A3_3EC4,
            0x00A3_3EB0,
            0x00A3_3E9C,
        ],
        0x19,
    )
}

// IDA: N/A a helper, with no address of its own.
fn construct_alchemy(world: &mut World) -> Result<Entity, GameError> {
    construct_magic_object(
        world,
        [
            0x00A3_217C,
            0x00A3_2134,
            0x00A3_2124,
            0x00A3_2104,
            0x00A3_20E8,
            0x00A3_20D4,
            0x00A3_20C0,
        ],
        0x28,
    )
}

// IDA: 004195F0 vt+0x14 of INGR and ALCH: +0x78 = -1, +0x7C = 0, a jump to 0046C2E0. +0x7D.. stay. Completion: 100%.
fn reset_magic_object(world: &mut World, form: Entity) -> Result<(), GameError> {
    let mut object = world
        .get_mut::<TesMagicObject>(form)
        .expect("a magic object");
    object.value_78 = -1;
    object.flags_7c = 0;
    reset_14_components(world, form)
}

// IDA: 00412860 vt+0x18 of INGR and ALCH: 00414C70 on +0x30, which frees the items and nodes and leaves +0x0C, a jump to 0046C2F0. Completion: 100%.
fn destroy_magic_object(world: &mut World, form: Entity) -> Result<(), GameError> {
    world
        .get_mut::<EffectItemList>(form)
        .expect("an effect list")
        .items_04
        .clear();
    reset_18_components(world, form)
}

// IDA: N/A a null name's text: A2F7EC `Default`, empty.
fn full_name_text(world: &World, form: Entity) -> Vec<u8> {
    world
        .get::<TesFullName>(form)
        .expect("the class's component")
        .name_04
        .data_00
        .clone()
        .unwrap_or_default()
}

// IDA: 0041B5B0 vt+0x1C of INGR and ALCH, with no record-type test and no 0046AB80: 00451210, the chunks while nonzero. First EDID; EFID, naming the spell by vt+0xD4 when nonempty, else +0x28 or `Default`; FULL +0x24. Then, for every chunk, the part's vt+0x3C 00419760(file, chunk): DATA 0046BDA0(0, 0) on the form, whose tail is weight; SCRI +0x64; ICON +0x58; MODL, MODB, MODT +0x40. Then vt+0x24 00419730 ENIT: a match clears vt+0x28 00419740 (+0x78) for vt+0x2C 00419750 (8) bytes and reads them. No items warns A34270 with the ID and +0x28 or `Default`. Returns 1. Completion: 100%.
fn load_magic_object_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    initialize_form_from_current_record(world, form, cx)?;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            EDID => edid_arm(cx)?,
            EFID => {
                let editor = editor_id_of(world, form);
                let spell = if editor.is_empty() {
                    full_name_text(world, form)
                } else {
                    editor
                };
                load_efid_chunk(world, form, Some(&spell), cx)?;
            }
            FULL => full_arm(world, form, cx)?,
            DATA => load_form_data(world, form, &mut [], 0, cx)?,
            SCRI => load_script_reference(world, form, cx)?,
            ICON => icon_arm(world, form, cx)?,
            MODL | MODB | MODT => load_model_arm(world, form, cx)?,
            ENIT => {
                let mut bytes = [0_u8; 8];
                cx.file.read_chunk(&mut bytes, 8)?;
                *world
                    .get_mut::<TesMagicObject>(form)
                    .expect("a magic object") = TesMagicObject {
                    value_78: i32::from_le_bytes(bytes[..4].try_into().expect("four bytes")),
                    flags_7c: bytes[4],
                    rest_7d: Some([bytes[5], bytes[6], bytes[7]]),
                };
            }
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    if world
        .get::<EffectItemList>(form)
        .expect("an effect list")
        .items_04
        .is_empty()
    {
        let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
        let name = latin1(&full_name_text(world, form));
        cx.file.services.warn(format!(
            "Magic Item ({id:08X}) {name} has no effects defined."
        ));
    }
    Ok(true)
}

// ---- SGST ----

// IDA: 004BB8A0 TESSigilStone_Constructor: 004B9860, 00470330 on +0x70, 00414A00 on +0x78, the nine vtables, +0x04 = 0x2A, 0046C2E0. Completion: 100% of the store order.
fn construct_sigil_stone(world: &mut World) -> Result<Entity, GameError> {
    let form = construct_misc_part(
        world,
        [
            0x00A4_5534,
            0x00A4_5520,
            0x00A4_5500,
            0x00A4_54E4,
            0x00A4_54D0,
            0x00A4_54B0,
            0x00A4_549C,
        ],
        0x2A,
    )?;
    world.entity_mut(form).insert((
        TesUsesForm {
            original_vtable: 0x00A4_5488,
            uses_04: 0,
        },
        EffectItemList {
            original_vtable: 0x00A4_5478,
            ..EffectItemList::default()
        },
    ));
    reset_form_components(world, form, SIGIL_SLOTS)?;
    Ok(form)
}

// IDA: 004BB790 TESSigilStone vt+0x18: 00414C70 on +0x78, a jump to 0046C2F0. Completion: 100%.
fn destroy_sigil_stone(world: &mut World, form: Entity) -> Result<(), GameError> {
    world
        .get_mut::<EffectItemList>(form)
        .expect("an effect list")
        .items_04
        .clear();
    reset_18_components(world, form)
}

// IDA: 004BBA30 TESSigilStone vt+0x1C LoadForm: `cmp al, 0x2A`, 00451210, 0046AB80(0), the chunks while nonzero: `misc_arm`, whose DATA tails are uses, value and weight; EFID 004154B0 with +0x28 or `Default` as the spell. Returns 1. Completion: 100%.
fn load_sigil_stone_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
    if !begin_load(world, form, cx, 0x2A)? {
        return Ok(false);
    }
    each_chunk(world, form, cx, |world, form, cx, chunk| {
        if chunk == EFID {
            let spell = full_name_text(world, form);
            return load_efid_chunk(world, form, Some(&spell), cx);
        }
        misc_arm(world, form, cx, chunk)
    })?;
    Ok(true)
}

// ---- this family's arms of 0044AC20 ----

// IDA: N/A a class's 00447DF0 arm.
pub(crate) fn construct_default(world: &mut World, form_type: u8) -> Result<Entity, GameError> {
    let class = form_class(u32::from(form_type)).expect("this family's class");
    (class.construct)(world)
}

// IDA: the arm of a bound default form: 55E000 and the cast; a miss runs FormHeap and the constructor, 0046C300(id, 1), vt+0xD8 (006FDFF0 keeps nothing), the caller's stores, 004B2B80, vt+0x90(0).
pub fn default_object(
    world: &mut World,
    form_type: u8,
    class: RttiClass,
    id: u32,
    model_path: Option<&[u8]>,
) -> Result<Option<Entity>, GameError> {
    if let Some(found) = cast(world, world.resource::<FormsById>().lookup(id), class) {
        return Ok(Some(found));
    }
    let form = construct_default(world, form_type)?;
    set_form_id(world, form, id);
    if let Some(path) = model_path {
        world
            .get_mut::<TesModel>(form)
            .expect("a TESModel")
            .path_04
            .set(Some(path), 0);
    }
    append_tes_object(world, form);
    clear_altered(world, form)?;
    Ok(Some(form))
}

// IDA: a CLOT default: 004B6150, then 00469090(sex, path) the world models, 004690B0(sex, path) the biped models, 00469020(bit, set) the slot bits; 004B2B80, vt+0x90(0). Row-backed for each form by AC20RAW.
pub fn default_clothing(
    world: &mut World,
    id: u32,
    piece: &str,
    slots: &[(u32, bool)],
) -> Result<Option<Entity>, GameError> {
    if let Some(found) = world.resource::<FormsById>().lookup(id) {
        return Ok(Some(found));
    }
    let form = construct_default(world, 0x16)?;
    set_form_id(world, form, id);
    {
        let mut part = world.get_mut::<TesBipedModelForm>(form).expect("clothing");
        let path = |sex: &str, tail: &str| {
            format!("Clothes\\LowerClass\\05\\{sex}\\{piece}{tail}.NIF").into_bytes()
        };
        part.world_models_38[0]
            .path_04
            .set(Some(&path("M", "_gnd")), 0);
        part.world_models_38[1]
            .path_04
            .set(Some(&path("F", "_gnd")), 0);
        part.biped_models_08[0].path_04.set(Some(&path("M", "")), 0);
        part.biped_models_08[1].path_04.set(Some(&path("F", "")), 0);
        for &(bit, set) in slots {
            let mask = 1_u16 << bit;
            if set {
                part.slot_mask_04 |= mask;
            } else {
                part.slot_mask_04 &= !mask;
            }
        }
    }
    append_tes_object(world, form);
    clear_altered(world, form)?;
    Ok(Some(form))
}

// IDA: a sound of 0041BB80 or 00518000: 004471A0 and the cast; a miss runs FormHeap(0x44), 00517F40, 0046C300(id, 1), vt+0xD8 00517EA0, vt+0x90(0), 00446CB0 onto +0x6C, and for 00518000 the editor-ID map.
pub fn default_sound(
    world: &mut World,
    id: u32,
    name: &[u8],
    mapped: bool,
) -> Result<Entity, GameError> {
    if let Some(found) = world.resource::<FormsById>().lookup(id)
        && world.get::<TesSound>(found).is_some()
    {
        return Ok(found);
    }
    let form = construct_default(world, 0x0A)?;
    set_form_id(world, form, id);
    world
        .get_mut::<TesSound>(form)
        .expect("a sound")
        .editor_id_30
        .set(Some(name), 0);
    clear_altered(world, form)?;
    world
        .resource_mut::<DataHandlerLists>()
        .sounds_6c
        .prepend_non_null(form);
    if mapped {
        world
            .resource_mut::<EditorIdMap>()
            .insert_or_assign(name, form);
    }
    Ok(form)
}

// IDA: the tables of 0041BB9B and 00518000, read from the pushes of each.
pub const MAGIC_SOUNDS: [(u32, &[u8]); 18] = [
    (0x12C, b"MagicFailureSoundAlteration"),
    (0x12D, b"MagicFailureSoundConjuration"),
    (0x12E, b"MagicFailureSoundDestruction"),
    (0x12F, b"MagicFailureSoundIllusion"),
    (0x130, b"MagicFailureSoundMysticism"),
    (0x131, b"MagicFailureSoundRestoration"),
    (0x138, b"MagicEnchantDrawSoundAlteration"),
    (0x139, b"MagicEnchantDrawSoundConjuration"),
    (0x13A, b"MagicEnchantDrawSoundDestruction"),
    (0x13B, b"MagicEnchantDrawSoundIllusion"),
    (0x13C, b"MagicEnchantDrawSoundMysticism"),
    (0x13D, b"MagicEnchantDrawSoundRestoration"),
    (0x13E, b"MagicEnchantHitSoundAlteration"),
    (0x13F, b"MagicEnchantHitSoundConjuration"),
    (0x140, b"MagicEnchantHitSoundDestruction"),
    (0x141, b"MagicEnchantHitSoundIllusion"),
    (0x142, b"MagicEnchantHitSoundMysticism"),
    (0x143, b"MagicEnchantHitSoundRestoration"),
];
pub const FOOTSTEP_SOUNDS: [(u32, &[u8]); 25] = [
    (0x212, b"FootSoundDirt"),
    (0x22B, b"FSTMetal"),
    (0x213, b"FootSoundGrass"),
    (0x214, b"FootSoundStone"),
    (0x215, b"FootSoundWater"),
    (0x216, b"FootSoundWood"),
    (0x21F, b"FSTSnow"),
    (0x217, b"FootSoundHeavyArmor"),
    (0x218, b"FootSoundLightArmor"),
    (0x229, b"FSTArmorHeavySneak"),
    (0x228, b"FSTArmorLightSneak"),
    (0x219, b"FootSoundEarthLand"),
    (0x21A, b"FootSoundGrassLand"),
    (0x21B, b"FootSoundMetalLand"),
    (0x21C, b"FootSoundStoneLand"),
    (0x21D, b"FootSoundWaterLand"),
    (0x21E, b"FootSoundWoodLand"),
    (0x220, b"FSTSnowLand"),
    (0x221, b"FSTEarthSneak"),
    (0x222, b"FSTGrassSneak"),
    (0x223, b"FSTMetalSneak"),
    (0x225, b"FSTStoneSneak"),
    (0x226, b"FSTWaterSneak"),
    (0x227, b"FSTWoodSneak"),
    (0x224, b"FSTSnowSneak"),
];

// ---- the table ----

// IDA: vt+0x6C per type, as the map's `init_item_6c` says.
const fn item_init(form_type: u8) -> super::form_arms::InitItemFn {
    use super::forms_init as init;
    match form_type {
        0x12 => init::init_activator,
        0x13 | 0x1B | 0x26 | 0x27 | 0x2A => init::init_scriptable_object,
        0x14 => init::init_armor,
        0x15 => init::init_book,
        0x16 => init::init_clothing,
        0x17 => init::init_container_object,
        0x18 => init::init_door,
        0x19 | 0x28 => init::init_magic_object,
        0x1A => init::init_light,
        0x1D | 0x1E => init::init_flag_only,
        0x1F => init::init_flora,
        0x20 => init::init_furniture,
        0x21 => init::init_weapon,
        0x22 => init::init_ammo,
        0x25 => init::init_leveled_creature,
        0x2B => init::init_leveled_form,
        _ => no_init_item,
    }
}

// IDA: N/A a row of `CLASSES` with this family's common fields. vt+0xD4 of all of these is 004129A0.
const fn item_class(
    form_type: u8,
    vtable: u32,
    construct: super::form_arms::ConstructFn,
    load_form: super::form_arms::LoadFormFn,
    reset_14: FormFn,
    reset_18: FormFn,
    slots: &'static [ComponentSlot],
) -> FormClass {
    FormClass {
        form_type,
        vtable,
        construct,
        load_form,
        reset_14,
        reset_18,
        editor_id: default_editor_id,
        init_item: item_init(form_type),
        slots,
        special_arm: None,
    }
}

/// IDA: this family's classes, from the exe's type map. A type absent here takes `AbsentArm`.
// Every vt+0x6C but STAT's and SBSP's (0060D0A0) is: 004B3D30 ACTI, 004B4590 APPA MISC KEYM SLGM SGST, 004B4C20 ARMO, 004B53E0 BOOK, 004B5C60 CLOT, 004B6290 CONT, 004B6DB0 DOOR, 004129E0 INGR ALCH, 004B0C70 LIGH, 004A9B70 GRAS TREE, 004AE070 FLOR, 004AEC30 FURN, 004BB120 WEAP, 004A9000 AMMO, 004AF950 LVLC, 004B02D0 LVLI.
pub const CLASSES: &[FormClass] = &[
    item_class(
        0x12,
        0x00A4_3CB4,
        construct_activator,
        load_activator_form,
        reset_activator,
        reset_18_components,
        ACTIVATOR_SLOTS,
    ),
    item_class(
        0x13,
        0x00A4_3FB4,
        construct_apparatus,
        load_apparatus_form,
        reset_apparatus,
        reset_18_components,
        APPARATUS_SLOTS,
    ),
    item_class(
        0x14,
        0x00A4_41BC,
        construct_armor,
        load_armor_form,
        reset_armor,
        reset_18_components,
        ARMOR_SLOTS,
    ),
    item_class(
        0x15,
        0x00A4_4454,
        construct_book,
        load_book_form,
        reset_book,
        reset_18_components,
        BOOK_SLOTS,
    ),
    item_class(
        0x16,
        0x00A4_4644,
        construct_clothing,
        load_clothing_form,
        reset_clothing,
        reset_18_components,
        CLOTHING_SLOTS,
    ),
    item_class(
        0x17,
        0x00A4_4824,
        construct_container,
        load_container_form,
        reset_container,
        reset_18_components,
        CONTAINER_SLOTS,
    ),
    item_class(
        0x18,
        0x00A4_4A54,
        construct_door,
        load_door_form,
        reset_door,
        destroy_door,
        ACTIVATOR_SLOTS,
    ),
    item_class(
        0x19,
        0x00A3_3F5C,
        construct_ingredient,
        load_magic_object_form,
        reset_magic_object,
        destroy_magic_object,
        MAGIC_OBJECT_SLOTS,
    ),
    item_class(
        0x1A,
        0x00A4_319C,
        construct_light,
        load_light_form,
        reset_light,
        reset_18_components,
        LIGHT_SLOTS,
    ),
    item_class(
        0x1B,
        0x00A4_4CA4,
        construct_misc,
        load_misc_form,
        reset_14_components,
        reset_18_components,
        MISC_SLOTS,
    ),
    item_class(
        0x1C,
        0x00A4_4DEC,
        construct_static,
        load_static_form,
        reset_14_components,
        reset_18_components,
        MODEL_ONLY_SLOTS,
    ),
    item_class(
        0x1D,
        0x00A4_280C,
        construct_grass,
        load_grass_form,
        reset_grass,
        reset_18_components,
        MODEL_ONLY_SLOTS,
    ),
    item_class(
        0x1E,
        0x00A4_4FB4,
        construct_tree,
        load_tree_form,
        reset_14_components,
        reset_18_components,
        TREE_SLOTS,
    ),
    item_class(
        0x1F,
        0x00A4_1B1C,
        construct_flora,
        load_flora_form,
        reset_activator,
        reset_18_components,
        FLORA_SLOTS,
    ),
    item_class(
        0x20,
        0x00A4_1D1C,
        construct_furniture,
        load_furniture_form,
        reset_furniture,
        reset_18_components,
        ACTIVATOR_SLOTS,
    ),
    item_class(
        0x21,
        0x00A4_5354,
        construct_weapon,
        load_weapon_form,
        reset_weapon,
        reset_18_components,
        WEAPON_SLOTS,
    ),
    item_class(
        0x22,
        0x00A4_068C,
        construct_ammo,
        load_ammo_form,
        reset_ammo,
        reset_18_components,
        AMMO_SLOTS,
    ),
    item_class(
        0x25,
        0x00A4_2B9C,
        construct_lev_creature,
        load_lev_creature_form,
        reset_lev_creature,
        reset_18_components,
        LEV_CREATURE_SLOTS,
    ),
    item_class(
        0x26,
        0x00A4_56F4,
        construct_soul_gem,
        load_soul_gem_form,
        reset_soul_gem,
        reset_18_components,
        MISC_SLOTS,
    ),
    item_class(
        0x27,
        0x00A4_2A4C,
        construct_key,
        load_misc_form,
        reset_14_components,
        reset_18_components,
        MISC_SLOTS,
    ),
    item_class(
        0x28,
        0x00A3_217C,
        construct_alchemy,
        load_magic_object_form,
        reset_magic_object,
        destroy_magic_object,
        MAGIC_OBJECT_SLOTS,
    ),
    item_class(
        0x29,
        0x00A4_592C,
        construct_sub_space,
        load_sub_space_form,
        reset_sub_space,
        reset_18_components,
        &[],
    ),
    item_class(
        0x2A,
        0x00A4_5534,
        construct_sigil_stone,
        load_sigil_stone_form,
        reset_14_components,
        destroy_sigil_stone,
        SIGIL_SLOTS,
    ),
    item_class(
        0x2B,
        0x00A4_2DC4,
        construct_lev_item,
        load_lev_item_form,
        reset_14_components,
        reset_18_components,
        LEV_ITEM_SLOTS,
    ),
];

// IDA: TESObjectWEAP +0x90, the weapon type (0 blade one hand .. 4 staff, 5 bow).
pub(crate) fn weapon_type_of(world: &World, weapon: Entity) -> u8 {
    weapon_type(world, weapon)
}

// IDA: TESObjectWEAP +0x90.
pub(in crate::tes) fn weapon_type(world: &World, weapon: Entity) -> u8 {
    world
        .get::<crate::tes::forms_items::TesWeapon>(weapon)
        .map_or(0, |weapon| weapon.data_90[0])
}
