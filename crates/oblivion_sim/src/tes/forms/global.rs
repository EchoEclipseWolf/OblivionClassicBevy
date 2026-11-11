//! TESGlobal (form type 0x04, size 0x28), the M0.6 ECS slice, now through the real file pass: an entity built on `construct_tes_form`, whose class goes through `form_arms`.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::form::{FormIdentity, OriginalVtable};
use super::form_arms::{
    FormClass, FormLoadCx, no_init_item, reset_14_components, reset_18_components,
};
use crate::bs::string::BsStringT;
use crate::tes::data_load::initialize_form_from_current_record;
use crate::tes::default_forms::TimeGlobals;
use crate::tes::form::construct_tes_form;
use crate::tes::forms_init::lookup;
use crate::tes::tile::InterfaceCx;

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
pub fn construct_global(world: &mut World) -> Result<Entity, GameError> {
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

// IDA: 004F9470 TESGlobal vt+0x1C LoadForm. The record must be GLOB (else 0); 00451210; the chunks: EDID (alloca, size 0x200, vt+0xD8 004F9610: 004028D0(+0x18, text, 0)), FNAM (1 byte), FLTV (004510E0). The loop ends when 0044FEA0 or 004510B0 gives 0. Returns 1. Completion: 100%.
fn load_global_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, GameError> {
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

// IDA: N/A a helper, with no address of its own.
fn time_global(world: &World, index: usize) -> Option<f32> {
    let global = world.get_resource::<TimeGlobals>()?.globals_b332e0[index]?;
    Some(f32::from_bits(
        world.get::<TESGlobal>(global)?.value_bits_24,
    ))
}

/// IDA: 004029E0(frame seconds), 0040DB9F outside menu mode: +0x0C += +0x14 * frame / 3600, each a float at 24 bits. Completion: 100% below 24 hours; a new day is a stop.
/// # Errors
/// The hour passing 24.
pub fn time_globals_advance(world: &mut World, frame_seconds: f32) -> Result<(), GameError> {
    let Some(scale) = time_global(world, 5) else {
        return Ok(());
    };
    let hour = time_global(world, 3).unwrap_or(12.0);
    let next = scale * frame_seconds / 3600.0 + hour;
    if next > 24.0 {
        return Err(GameError::unclassified(
            0x00402A13,
            "00402A13: a game day passing",
        ));
    }
    let global = world.resource::<TimeGlobals>().globals_b332e0[3].ok_or(
        GameError::unclassified(0x004029E0, "004029E0: a null game hour faults"),
    )?;
    world
        .get_mut::<TESGlobal>(global)
        .expect("a global")
        .value_bits_24 = next.to_bits();
    Ok(())
}

/// IDA: 00402BD0 TimeGlobals_GetGameHour: +0x0C, or 12.0.
#[must_use]
pub fn game_hour(world: &World) -> f32 {
    time_global(world, 3).unwrap_or(12.0)
}

// IDA: 00402B40 the year: +0x00 or 427.0, `fistp qword` under chop, the low dword.
pub(in crate::tes) fn game_year(world: &World) -> i32 {
    x87::ftol64_low(time_global(world, 0).unwrap_or(427.0)) as i32
}

// IDA: 00402B80 the month: +0x04 through __ftol2, then `movsx eax,al` (00402B8A..00402B8F), or a literal 7. The C++ returns the whole int.
pub(in crate::tes) fn game_month(world: &World) -> i32 {
    time_global(world, 1).map_or(7, |value| i32::from(x87::ftol(value) as i8))
}

// IDA: 00402BA0 the day: +0x08 or 17.0, __ftol2, the whole eax; callers narrow it.
pub(in crate::tes) fn game_day(world: &World) -> i32 {
    x87::ftol(time_global(world, 2).unwrap_or(17.0))
}

/// IDA: 00402C40 the day of the week: +0x10 or 1.0 by `fistp qword` under chop, the low dword, unsigned % 7. The C++'s float to unsigned cast is undefined for a negative. Completion: 100%.
#[must_use]
pub fn game_day_of_week(world: &World) -> i32 {
    (x87::ftol64_low(time_global(world, 4).unwrap_or(1.0)) % 7) as i32
}

// IDA: 0047D2B0 the days in a month, from the table at 00B06710 (the C++'s comment says 00B06A00): `cmp eax,0Ch; jl`, else `xor ax,ax`. A negative index reads before the table, which only a broken +0x18 gives.
pub(in crate::tes) fn days_in_month(month: i32) -> Result<i32, GameError> {
    const DAYS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if month >= 12 {
        return Ok(0);
    }
    usize::try_from(month)
        .map(|month| DAYS[month])
        .map_err(|_| {
            GameError::unclassified(
                0x0047D2B8,
                "0047D2B8: a negative month reads before the table",
            )
        })
}

/// IDA: 00402BA0 for a caller that narrows it, with `movsx`.
#[must_use]
pub fn game_day_byte(world: &World) -> u8 {
    game_day(world) as u8
}

/// IDA: 00402860 TimeGlobals_Init: 0046B250 of 0x35..=0x3A, in order. Completion: 100%.
pub fn init_time_globals(world: &mut World) {
    let mut globals = [None; 6];
    for (index, slot) in globals.iter_mut().enumerate() {
        *slot = lookup(world, 0x35 + index as u32);
    }
    world.insert_resource(TimeGlobals {
        globals_b332e0: globals,
    });
}

// IDA: 00402E50 TimeGlobals_FormatDate: "%s %d, 3E%d" of the month's name (00B06FA4[month], the setting's text), the day
// (`movsx` of its __ftol2 byte) and the year (`fistp` under chop).
pub(crate) fn map_date(cx: &InterfaceCx) -> Result<Vec<u8>, GameError> {
    let month = game_month(cx.world);
    if !(0..=12).contains(&month) {
        return Err(GameError::unclassified(
            0x00402E50,
            "00402E50: a month outside 00B06FA4's table",
        ));
    }
    let name = cx
        .world
        .get_resource::<crate::tes::game_settings::GameSettingCollection>()
        .and_then(|settings| settings.text_at(0x00B3_50EC + 8 * month as u32))
        .unwrap_or_default();
    let day = i32::from(game_day(cx.world) as i8);
    let year = game_year(cx.world);
    let mut text = name;
    text.extend_from_slice(format!(" {day}, 3E{year}").as_bytes());
    Ok(text)
}

/// IDA: 00B3514C sDaySundas, the first of the seven settings 00B06FD4 points at (8 bytes apart).
const S_DAY_SUNDAS_B3514C: u32 = 0x00B3_514C;

/// IDA: 00402C90 TimeGlobals_GetDayName: GameDaysPassed (+0x10's +0x24) or 1.0, stored as a float, `fistp dword` under
/// chop, an unsigned `div` by 7, and 00B06FD4[day]'s setting text (a null entry is null; -1 and 7 give "Bad Day", which
/// an unsigned remainder never reaches). Completion: 100%.
#[must_use]
pub fn time_globals_day_name(world: &World) -> Option<Vec<u8>> {
    let days = world
        .get_resource::<crate::tes::default_forms::TimeGlobals>()
        .and_then(|globals| globals.globals_b332e0[4])
        .and_then(|global| world.get::<TESGlobal>(global))
        .map_or(1.0, |global| f32::from_bits(global.value_bits_24));
    let day = x87::ftol(days) as u32 % 7;
    world
        .get_resource::<crate::tes::game_settings::GameSettingCollection>()
        .and_then(|settings| settings.text_at(S_DAY_SUNDAS_B3514C + 8 * day))
}

// IDA: 00402C10 TimeGlobals_GetTruncatedDaysPassedOrOne: +0x10 or 1.0, truncated.
pub(crate) fn days_passed(world: &World) -> u32 {
    let value = world
        .get_resource::<crate::tes::default_forms::TimeGlobals>()
        .and_then(|globals| globals.globals_b332e0[4])
        .and_then(|global| world.get::<TESGlobal>(global))
        .map_or(1.0, |global| f32::from_bits(global.value_bits_24));
    oblivion_core::x87::ftol(value) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_month_table_ends_at_twelve() {
        assert_eq!(days_in_month(0).unwrap(), 31);
        assert_eq!(days_in_month(1).unwrap(), 28);
        assert_eq!(days_in_month(12).unwrap(), 0);
        assert!(days_in_month(-1).is_err());
    }
}
