//! TESGlobal (form type 0x04, size 0x28), the M0.6 ECS slice, now through the real file pass: an entity built on `construct_tes_form`, whose class goes through `form_arms`.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::data_handler::{
    DataHandlerLists, EditorIdMap, FormsById, construct_tes_form, set_form_id,
};
use super::data_load::initialize_form_from_current_record;
use super::form::{FormIdentity, OriginalVtable};
use super::form_arms::{
    FormClass, FormLoadCx, no_init_item, reset_14_components, reset_18_components,
};
use super::tes_file::BsStringT;

/// IDA: 00A4981C the TESGlobal vtable.
pub const TES_GLOBAL_VTABLE: u32 = 0x00A4_981C;
/// IDA: the form type.
pub const FORM_TYPE_GLOBAL: u8 = 0x04;

// IDA: the chunks 004F94B0 dispatches on.
const EDID: u32 = 0x4449_4445;
const FNAM: u32 = 0x4D41_4E46;
const FLTV: u32 = 0x5654_4C46;

/// IDA: 004F95E0 TESGlobal: +0x18 BSStringT editor ID, whose capacity outlives a shorter reload.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct EditorId(pub BsStringT);

/// IDA: 004F95E0 TESGlobal: +0x20 variable type, +0x24 value as raw float bits.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct TESGlobal {
    pub variable_type_20: u8,
    pub value_bits_24: u32,
}

/// IDA: 004F95E0 TESGlobal_Constructor: 0046C460, +0x18 clear, value 0, type 's', vtable, +0x04 = 4. Completion: 100%.
pub fn construct_global(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        OriginalVtable(TES_GLOBAL_VTABLE),
        EditorId::default(),
        TESGlobal {
            variable_type_20: b's',
            value_bits_24: 0,
        },
    ));
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = FORM_TYPE_GLOBAL;
    Ok(form)
}

/// IDA: 00402860's globals, in order: GameYear .. TimeScale. Each runs SetFormID, vt+0xD8, the +0x24
/// value, the +0x74 prepend and the editor-ID map. An ID already present is skipped.
/// Completion: 100% of the globals; the function's other form types arrive with them.
pub fn create_default_globals(world: &mut World) -> Result<(), StreamError> {
    const DEFAULTS: [(u32, &[u8], f32); 6] = [
        (0x35, b"GameYear", 427.0),
        (0x36, b"GameMonth", 7.0),
        (0x37, b"GameDay", 17.0),
        (0x38, b"GameHour", 12.0),
        (0x39, b"GameDaysPassed", 1.0),
        (0x3A, b"TimeScale", 30.0),
    ];
    for (id, name, value) in DEFAULTS {
        if world.resource::<FormsById>().lookup(id).is_some() {
            continue;
        }
        let form = construct_global(world)?;
        set_form_id(world, form, id);
        world
            .get_mut::<EditorId>(form)
            .expect("just built")
            .0
            .set(Some(name), 0);
        world
            .get_mut::<TESGlobal>(form)
            .expect("just built")
            .value_bits_24 = value.to_bits();
        world
            .resource_mut::<DataHandlerLists>()
            .globals_74
            .prepend_non_null(form);
        world
            .resource_mut::<EditorIdMap>()
            .insert_or_assign(name, form);
    }
    Ok(())
}

// IDA: 004F9470 TESGlobal vt+0x1C LoadForm. The record must be GLOB (else 0); 00451210; the chunks: EDID (alloca, size 0x200, vt+0xD8 004F9610: 004028D0(+0x18, text, 0)), FNAM (1 byte), FLTV (004510E0). The loop ends when 0044FEA0 or 004510B0 gives 0. Returns 1. Completion: 100%.
fn load_global_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if u32::from(FORM_TYPE_GLOBAL) != cx.file.record_form_type()? {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            EDID => {
                let text = cx.file.read_text(0x200)?;
                world
                    .get_mut::<EditorId>(form)
                    .expect("a global")
                    .0
                    .set(Some(&text), 0);
            }
            FNAM => {
                let mut global = world.get_mut::<TESGlobal>(form).expect("a global");
                let mut byte = [global.variable_type_20, 0];
                cx.file.read_chunk(&mut byte, 1)?;
                global.variable_type_20 = byte[0];
            }
            FLTV => {
                let mut global = world.get_mut::<TESGlobal>(form).expect("a global");
                let (bits, _) = cx.file.read_dword(global.value_bits_24)?;
                global.value_bits_24 = bits;
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

// IDA: 004F9630 vt+0xD4: +0x18, or `Default`. Completion: 100%.
fn global_editor_id(world: &World, form: Entity) -> Vec<u8> {
    world
        .get::<EditorId>(form)
        .expect("a global")
        .0
        .data_00
        .clone()
        .unwrap_or_default()
}

/// IDA: TESGlobal's class. It has no component bases, so 0046B090 and 0046B170 only run the form's nullsub_2 slot.
pub const CLASSES: &[FormClass] = &[FormClass {
    form_type: FORM_TYPE_GLOBAL,
    vtable: TES_GLOBAL_VTABLE,
    construct: construct_global,
    load_form: load_global_form,
    reset_14: reset_14_components,
    reset_18: reset_18_components,
    editor_id: global_editor_id,
    init_item: no_init_item,
    slots: &[],
    special_arm: None,
}];
