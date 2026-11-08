//! Phase 12b: the environment forms, as in the C++'s forms_environment: TESWeather, TESClimate (with the weather list), TESLoadScreen, TESObjectANIO, TESWaterForm and TESEffectShader, plus the parts of 0044AC20 that create their default forms. The textures and models of WTHR, CLMT and EFSH are members, not component bases: 00466E80 finds no slots for them, and their reset pair doesn't touch them.
//!
//! Evidence: objdump of each address named here; P12-RUN-001's SAMPLE, AC20 and DATA rows.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::data_handler::{
    DataHandlerGlobals, DataHandlerLists, FormsById, TES_DESCRIPTION_VTABLE, TES_TEXTURE_VTABLE,
    TesDescription, TesTexture, construct_tes_form, reset_description, set_form_id,
};
use super::data_load::initialize_form_from_current_record;
use super::form::{FormIdentity, OriginalVtable};
use super::form_arms::{
    FormClass, FormLoadCx, default_editor_id, no_init_item, reset_14_components,
    reset_18_components,
};
use super::form_components::{
    ComponentSlot, DataChunkTails, FormRef, TES_MODEL_VTABLE, TesAttackDamageForm, TesModel,
    load_data_chunk, load_model, load_texture,
};
use super::forms_misc::{clear_altered, load_model_arm, set_initialized};
use super::forms_script::map_file_form_id;
use super::tes_file::BsStringT;

/// IDA: the form types of this family.
pub const FORM_TYPE_WEATHER: u8 = 0x2D;
pub const FORM_TYPE_CLIMATE: u8 = 0x2E;
pub const FORM_TYPE_LOAD_SCREEN: u8 = 0x3F;
pub const FORM_TYPE_ANIO: u8 = 0x41;
pub const FORM_TYPE_WATER: u8 = 0x42;
pub const FORM_TYPE_EFFECT_SHADER: u8 = 0x43;

/// IDA: the most-derived vtables, and each class's secondary vtables, from the constructors named below.
pub const TES_WEATHER_VTABLE: u32 = 0x00A4_7FEC;
pub const TES_CLIMATE_VTABLE: u32 = 0x00A4_5C9C;
pub const TES_LOAD_SCREEN_VTABLE: u32 = 0x00A4_997C;
pub const TES_LOAD_SCREEN_TEXTURE_VTABLE: u32 = 0x00A4_9960;
pub const TES_LOAD_SCREEN_DESCRIPTION_VTABLE: u32 = 0x00A4_9948;
pub const TES_ANIO_VTABLE: u32 = 0x00A4_3DFC;
pub const TES_ANIO_MODEL_VTABLE: u32 = 0x00A4_3DD8;
pub const TES_WATER_VTABLE: u32 = 0x00A4_7F0C;
pub const TES_WATER_DAMAGE_VTABLE: u32 = 0x00A4_7EF4;
pub const TES_EFFECT_SHADER_VTABLE: u32 = 0x00A4_19EC;

// IDA: the chunks this family's LoadForms dispatch on.
const EDID: u32 = 0x4449_4445;
const DATA: u32 = 0x4154_4144;
const DESC: u32 = 0x4353_4544;
const ICON: u32 = 0x4E4F_4349;
const ICO2: u32 = 0x324F_4349;
const MODL: u32 = 0x4C44_4F4D;
const MODB: u32 = 0x4244_4F4D;
const MODT: u32 = 0x5444_4F4D;
const ANAM: u32 = 0x4D41_4E41;
const CNAM: u32 = 0x4D41_4E43;
const DNAM: u32 = 0x4D41_4E44;
const FNAM: u32 = 0x4D41_4E46;
const GNAM: u32 = 0x4D41_4E47;
const HNAM: u32 = 0x4D41_4E48;
const LNAM: u32 = 0x4D41_4E4C;
const MNAM: u32 = 0x4D41_4E4D;
const SNAM: u32 = 0x4D41_4E53;
const TNAM: u32 = 0x4D41_4E54;
const NAM0: u32 = 0x304D_414E;
const WLST: u32 = 0x5453_4C57;

// IDA: N/A a store to +0x04.
fn set_form_type(world: &mut World, form: Entity, form_type: u8) {
    world
        .get_mut::<FormIdentity>(form)
        .expect("a form")
        .form_type_04 = form_type;
}

// IDA: 0046FFD0 TESTexture_Constructor as a member.
fn new_texture() -> TesTexture {
    TesTexture {
        original_vtable: TES_TEXTURE_VTABLE,
        path_04: BsStringT::default(),
    }
}

// IDA: 0046D7E0 TESModel_Constructor as a member.
fn new_model() -> TesModel {
    TesModel {
        original_vtable: TES_MODEL_VTABLE,
        ..TesModel::default()
    }
}

// IDA: alloca(+0x254), 00450C20(size 0) and 004028D0(field, text, 0), which DESC, MNAM and RDMP all inline. Completion: 100%.
pub(super) fn load_string(
    text: &mut BsStringT,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    let read = cx.file.read_text(0)?;
    text.set(Some(&read), 0);
    Ok(())
}

// ---- the weather list ----

/// IDA: an 8-byte {TESWeather*, u32 chance} block of a TESWeatherList.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeatherEntry {
    pub weather: Entity,
    pub chance: u32,
}

/// IDA: 004EEDD0 TESWeatherList_Load(list, file, form) (__cdecl). Count = +0x254 >> 3; 0 returns with the list as it was. 005B1D70 frees the list. A failed 00450C20 warns with the form's ID and carries on over FormHeap bytes (host zero). Each {ID, chance} maps through 0046BB20 with this file, then casts to TESWeather: a prepend (00446CB0), or a warning with the mapped ID. Then 005B27A0 with 004EEB10. Completion: 100%.
pub fn load_weather_list(
    world: &World,
    list: &mut Vec<WeatherEntry>,
    form_id: u32,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<(), StreamError> {
    let size = cx.file.chunk_size()?;
    let count = size >> 3;
    if count == 0 {
        return Ok(());
    }
    list.clear();
    let wanted = count.wrapping_mul(8);
    let mut buffer = vec![0_u8; wanted as usize];
    if !cx.file.read_chunk(&mut buffer, wanted)? {
        cx.file.services.warn(format!(
            "Error getting TESWeatherList chunk for form ID: {}",
            form_id as i32
        ));
    }
    let file = Some(cx.file.id);
    for entry in buffer.as_chunks::<8>().0 {
        let raw = u32::from_le_bytes(entry[..4].try_into().expect("four bytes"));
        let chance = u32::from_le_bytes(entry[4..].try_into().expect("four bytes"));
        let id = map_file_form_id(world, cx, raw, file)?;
        // 0046B250, then the __RTDynamicCast to TESWeather (b05400).
        let weather = world.resource::<FormsById>().lookup(id).filter(|&form| {
            world
                .get::<FormIdentity>(form)
                .is_some_and(|identity| identity.form_type_04 == FORM_TYPE_WEATHER)
        });
        match weather {
            Some(weather) => list.insert(0, WeatherEntry { weather, chance }),
            None => cx.file.services.warn(format!(
                "Error while loading weather list...cannot identify TESWeather from form ID: ({id:08X})"
            )),
        }
    }
    sort_weather_list(list);
    Ok(())
}

// IDA: 004EEB10 the weather-list order, unsigned: the higher chance first (-1), then 1 or 0. Completion: 100%.
const fn compare_weather(left: &WeatherEntry, right: &WeatherEntry) -> i32 {
    if left.chance > right.chance {
        -1
    } else if left.chance < right.chance {
        1
    } else {
        0
    }
}

/// IDA: 005B27A0 with 00506F00 the shell sort: the gap goes 3h + 1 from 1 while it is at most (n - 1) / 9 (signed), each pass an insertion sort by that gap, stopping at a compare of 0 or more. Not a stable sort, so `sort_by` can't stand in. The rebuild of the nodes is the `Vec`'s order. Completion: 100%.
pub fn sort_weather_list(items: &mut [WeatherEntry]) {
    let last = items.len() as i32 - 1;
    let mut gap = 1_i32;
    while gap <= last / 9 {
        gap = 3 * gap + 1;
    }
    while gap > 0 {
        let mut index = gap;
        while index <= last {
            let held = items[index as usize];
            let mut slot = index;
            while slot >= gap {
                let other = items[(slot - gap) as usize];
                if compare_weather(&held, &other) >= 0 {
                    break;
                }
                items[slot as usize] = other;
                slot -= gap;
            }
            items[slot as usize] = held;
            index += 1;
        }
        gap /= 3;
    }
}

// ---- TESWeather ----

/// IDA: 004EE7C0 TESWeather (0x148): two TESTextures +0x18 (DNAM, CNAM), a TESModel +0x30, DATA +0x48 (15 bytes; +0x57 is never stored), FNAM +0x58, NAM0 +0x68, the SNAM list +0x108, HNAM +0x110.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct TesWeather {
    pub textures_18: [TesTexture; 2],
    pub model_30: TesModel,
    pub data_48: [u8; 15],
    pub fog_58: [u8; 0x10],
    pub colors_68: [u8; 0xA0],
    /// An 8-byte {sound ID, type} block per SNAM, with the ID mapped.
    pub sounds_108: Vec<[u32; 2]>,
    pub hdr_110: [u8; 0x38],
}

impl TesWeather {
    // IDA: the 004EDED0 and 004EE847 stores: NAM0, DATA, FNAM and HNAM cleared; +0x57 not.
    const fn clear_blocks(&mut self) {
        self.colors_68 = [0; 0xA0];
        self.data_48 = [0; 15];
        self.fog_58 = [0; 0x10];
        self.hdr_110 = [0; 0x38];
    }
}

// IDA: 004EDED0 TESWeather vt+0x14: the blocks, then a jump to 0046C2E0, which finds no slots: the textures and the model stay. Completion: 100%.
fn reset_weather(world: &mut World, form: Entity) -> Result<(), StreamError> {
    world
        .get_mut::<TesWeather>(form)
        .expect("a weather")
        .clear_blocks();
    reset_14_components(world, form)
}

// IDA: 004EE8A0 TESWeather vt+0x18: 004EE770 frees the SNAM blocks and nodes; 0046C2F0. Completion: 100%.
fn destroy_weather(world: &mut World, form: Entity) -> Result<(), StreamError> {
    world
        .get_mut::<TesWeather>(form)
        .expect("a weather")
        .sounds_108
        .clear();
    reset_18_components(world, form)
}

// IDA: 004EE7C0 TESWeather_Constructor: 0046C460, the vtable, 00984BCF over the two textures, 0046D7E0, +0x108 = +0x10C = 0, +0x04, the blocks, 0046C2E0. Completion: 100% of the store order.
fn construct_weather(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        OriginalVtable(TES_WEATHER_VTABLE),
        TesWeather {
            textures_18: [new_texture(), new_texture()],
            model_30: new_model(),
            data_48: [0; 15],
            fog_58: [0; 0x10],
            colors_68: [0; 0xA0],
            sounds_108: Vec::new(),
            hdr_110: [0; 0x38],
        },
    ));
    set_form_type(world, form, FORM_TYPE_WEATHER);
    reset_14_components(world, form)?;
    Ok(form)
}

// IDA: 004EDF20 TESWeather vt+0x1C LoadForm. The record must be WTHR (else 0, by the low byte); 00451210; 0046AB80(0). The chunks: DATA of exactly 15 bytes, or of 12 with +0x54 = 0xFFFF and +0x56 = 0xFF, any other size skipped; NAM0 of exactly 0xA0, any other skipped; MODB and MODL 0046DB00 on the member (MODT isn't dispatched); EDID (alloca, size 0x200, vt+0xD8 006FDFF0 keeps nothing); CNAM 004700E0 on +0x24; the 004EE130 byte table from DNAM: DNAM on +0x18, FNAM 0x10 bytes, HNAM 0x38 bytes, SNAM {0, 0}, 8 bytes, 0046BB20 on the first, appended (0067B1E0). Returns 1. Completion: 100%.
fn load_weather_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_WEATHER {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            DATA => {
                let size = cx.file.chunk_size()?;
                let mut weather = world.get_mut::<TesWeather>(form).expect("a weather");
                if size == 0x0F {
                    cx.file.read_chunk(&mut weather.data_48, 0x0F)?;
                } else if size == 0x0C {
                    cx.file.read_chunk(&mut weather.data_48[..0x0C], 0x0C)?;
                    weather.data_48[0x0C..].fill(0xFF);
                }
            }
            NAM0 => {
                if cx.file.chunk_size()? == 0xA0 {
                    let mut weather = world.get_mut::<TesWeather>(form).expect("a weather");
                    cx.file.read_chunk(&mut weather.colors_68, 0xA0)?;
                }
            }
            MODB | MODL => {
                let mut model = std::mem::take(
                    &mut world
                        .get_mut::<TesWeather>(form)
                        .expect("a weather")
                        .model_30,
                );
                let result = load_model(world, &mut model, None, cx);
                world
                    .get_mut::<TesWeather>(form)
                    .expect("a weather")
                    .model_30 = model;
                result?;
            }
            EDID => {
                cx.file.read_text(0x200)?;
            }
            CNAM => load_texture(
                &mut world
                    .get_mut::<TesWeather>(form)
                    .expect("a weather")
                    .textures_18[1],
                cx,
            )?,
            DNAM => load_texture(
                &mut world
                    .get_mut::<TesWeather>(form)
                    .expect("a weather")
                    .textures_18[0],
                cx,
            )?,
            FNAM => {
                let mut weather = world.get_mut::<TesWeather>(form).expect("a weather");
                cx.file.read_chunk(&mut weather.fog_58, 0x10)?;
            }
            HNAM => {
                let mut weather = world.get_mut::<TesWeather>(form).expect("a weather");
                cx.file.read_chunk(&mut weather.hdr_110, 0x38)?;
            }
            SNAM => {
                let mut bytes = [0_u8; 8];
                cx.file.read_chunk(&mut bytes, 8)?;
                let raw = u32::from_le_bytes(bytes[..4].try_into().expect("four bytes"));
                let kind = u32::from_le_bytes(bytes[4..].try_into().expect("four bytes"));
                let id = map_file_form_id(world, cx, raw, Some(cx.file.id))?;
                world
                    .get_mut::<TesWeather>(form)
                    .expect("a weather")
                    .sounds_108
                    .push([id, kind]);
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

// ---- TESClimate ----

/// IDA: 004BEC50 TESClimate (0x58): a TESModel +0x18, the weather list +0x30, two TESTextures +0x38 (FNAM sun, GNAM glare), TNAM +0x50 (6 bytes; +0x56 and +0x57 are never stored).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct TesClimate {
    pub model_18: TesModel,
    pub weathers_30: Vec<WeatherEntry>,
    pub textures_38: [TesTexture; 2],
    pub timing_50: [u8; 6],
}

/// IDA: 00B34D88, which 004BEDD1 and 004A596C save and compare to learn whether a weather list warned. All 18 references in the exe are saves, clears, compares and restores of it. 004A7A60 goes through 004941A0 with level 1 to vt+4 of the 00B34D90 object (00494650, vtable A3DA08), and that slot is 004943A0, `xor eax,eax; ret 4`. So nothing increments it and the climate and region warnings below can't fire. The C++ counted its warnings there.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WarningCounter {
    pub count_b34d88: i32,
}

// IDA: 004BEC30 and 004BECC2: +0x50 = 0, +0x54 = 0 (u16), then +0x55 = (+0x55 & 0xC3) | 3. Completion: 100%.
const fn clear_climate_timing(timing: &mut [u8; 6]) {
    *timing = [0; 6];
    timing[5] = (timing[5] & 0xC3) | 3;
}

// IDA: 004BEC30 TESClimate vt+0x14: no component pass. Completion: 100%.
fn reset_climate(world: &mut World, form: Entity) -> Result<(), StreamError> {
    clear_climate_timing(
        &mut world
            .get_mut::<TesClimate>(form)
            .expect("a climate")
            .timing_50,
    );
    Ok(())
}

// IDA: 004BEC50 TESClimate_Constructor: 0046C460, the vtable, 0046D7E0, 004EED50, 00984BCF over the textures, +0x04, the timing. No component pass. Completion: 100% of the store order.
fn construct_climate(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    let mut climate = TesClimate {
        model_18: new_model(),
        textures_38: [new_texture(), new_texture()],
        ..TesClimate::default()
    };
    clear_climate_timing(&mut climate.timing_50);
    world
        .entity_mut(form)
        .insert((OriginalVtable(TES_CLIMATE_VTABLE), climate));
    set_form_type(world, form, FORM_TYPE_CLIMATE);
    Ok(form)
}

// IDA: 004BECF0 TESClimate vt+0x1C LoadForm. The record must be CLMT; 00451210; 0046AB80(0). The chunks: MODL alone of the model's (0046DB00); EDID; FNAM and GNAM 004700E0 on +0x38 and +0x44 (the `lea edx,[eax+eax*2+0x183C153C]` scale); WLST with g_disableWarningMessages saved and set around 004EEDD0, and a warning if 00B34D88 grew (`jle` over it), with vt+0xD4's name; TNAM 6 bytes. Returns 1. Completion: 100%.
fn load_climate_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_CLIMATE {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            MODL => {
                let mut model = std::mem::take(
                    &mut world
                        .get_mut::<TesClimate>(form)
                        .expect("a climate")
                        .model_18,
                );
                let result = load_model(world, &mut model, None, cx);
                world
                    .get_mut::<TesClimate>(form)
                    .expect("a climate")
                    .model_18 = model;
                result?;
            }
            EDID => {
                cx.file.read_text(0x200)?;
            }
            FNAM | GNAM => load_texture(
                &mut world
                    .get_mut::<TesClimate>(form)
                    .expect("a climate")
                    .textures_38[usize::from(chunk == GNAM)],
                cx,
            )?,
            WLST => {
                let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
                let mut list = std::mem::take(
                    &mut world
                        .get_mut::<TesClimate>(form)
                        .expect("a climate")
                        .weathers_30,
                );
                let result = with_weather_list_warnings(world, cx, |world, cx| {
                    load_weather_list(world, &mut list, id, cx)
                });
                world
                    .get_mut::<TesClimate>(form)
                    .expect("a climate")
                    .weathers_30 = list;
                if result? {
                    cx.file.services.warn(
                        "Warnings were encountered while loading weather list chunk from climate "
                            .to_owned(),
                    );
                }
            }
            TNAM => {
                let mut climate = world.get_mut::<TesClimate>(form).expect("a climate");
                cx.file.read_chunk(&mut climate.timing_50, 6)?;
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

/// IDA: 004BEDD1..004BEE04 and 004A596C..004A5998: 00B34D88 saved, g_disableWarningMessages saved and set to 1, the list, the flag back. Answers whether the count grew, compared signed. Completion: 100%.
pub fn with_weather_list_warnings(
    world: &mut World,
    cx: &mut FormLoadCx<'_, '_>,
    call: impl FnOnce(&World, &mut FormLoadCx<'_, '_>) -> Result<(), StreamError>,
) -> Result<bool, StreamError> {
    let before = world.get_resource_or_init::<WarningCounter>().count_b34d88;
    let saved = cx.file.services.disable_warning_messages_b06b18;
    cx.file.services.disable_warning_messages_b06b18 = 1;
    let result = call(world, cx);
    cx.file.services.disable_warning_messages_b06b18 = saved;
    result?;
    Ok(world.resource::<WarningCounter>().count_b34d88 > before)
}

// ---- TESLoadScreen ----

/// IDA: 004F9A40 TESLoadScreen (0x3C): TESTexture +0x18 and TESDescription +0x24 (`TesTexture`, `TesDescription` on the same entity), the LNAM list +0x2C (0x0C blocks of raw words until InitItem), and the +0x34 BSStringT that DESC fills.
#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct TesLoadScreen {
    pub locations_2c: Vec<[u32; 3]>,
    pub text_34: BsStringT,
}

// IDA: 004F9AE0 TESLoadScreen vt+0x18: 004F99C0 frees the LNAM blocks and nodes; 0046C2F0. Completion: 100%.
fn destroy_load_screen(world: &mut World, form: Entity) -> Result<(), StreamError> {
    world
        .get_mut::<TesLoadScreen>(form)
        .expect("a load screen")
        .locations_2c
        .clear();
    reset_18_components(world, form)
}

// IDA: 004F9A40 TESLoadScreen_Constructor: 0046C460, 0046FFD0 on +0x18, 0046A400 on +0x24, the three vtables, +0x2C.. = 0, +0x34 clear, +0x04, 0046AB80(1). No component pass. Completion: 100% of the store order.
fn construct_load_screen(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    let mut description = TesDescription {
        original_vtable: TES_DESCRIPTION_VTABLE,
        offset_04: 0,
    };
    reset_description(
        &mut description,
        &mut world.resource_mut::<DataHandlerGlobals>(),
    );
    description.original_vtable = TES_LOAD_SCREEN_DESCRIPTION_VTABLE;
    world.entity_mut(form).insert((
        OriginalVtable(TES_LOAD_SCREEN_VTABLE),
        TesTexture {
            original_vtable: TES_LOAD_SCREEN_TEXTURE_VTABLE,
            path_04: BsStringT::default(),
        },
        description,
        TesLoadScreen::default(),
    ));
    set_form_type(world, form, FORM_TYPE_LOAD_SCREEN);
    set_initialized(world, form, true);
    Ok(form)
}

// IDA: 004F9700 TESLoadScreen vt+0x1C LoadForm, with no record type test. 00451210; 0046AB80(0). The chunks: FNAM warns "File %s contains old data for loadscreen %s (%08X)." and reads nothing; DESC goes to +0x34 (alloca, size 0, 004028D0), never to the TESDescription; EDID; ICON 004700E0 on +0x18; LNAM FormHeap(0x0C), 0x0C bytes (a short chunk leaves heap bytes: host zero), prepended (inlined 00446CB0). Returns 1. Completion: 100%.
fn load_load_screen_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            FNAM => {
                let name = cx.file.file()?.name_1c.clone();
                let id = world.get::<FormIdentity>(form).expect("a form").form_id_0c;
                cx.file.services.warn(format!(
                    "File {name} contains old data for loadscreen  ({id:08X})."
                ));
            }
            DESC => load_string(
                &mut world
                    .get_mut::<TesLoadScreen>(form)
                    .expect("a load screen")
                    .text_34,
                cx,
            )?,
            EDID => {
                cx.file.read_text(0x200)?;
            }
            ICON => load_texture(
                &mut world.get_mut::<TesTexture>(form).expect("a load screen"),
                cx,
            )?,
            LNAM => {
                let mut bytes = [0_u8; 0x0C];
                cx.file.read_chunk(&mut bytes, 0x0C)?;
                let word = |at: usize| {
                    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
                };
                let block = [word(0), word(4), word(8)];
                world
                    .get_mut::<TesLoadScreen>(form)
                    .expect("a load screen")
                    .locations_2c
                    .insert(0, block);
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

// ---- TESObjectANIO ----

/// IDA: 004B4290 TESObjectANIO (0x34): TESModel +0x18 (`TesModel` on the same entity), and +0x30 the idle: DATA's raw form ID until InitItem.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TesObjectAnio {
    pub idle_30: FormRef,
}

// IDA: 004B4290 TESObjectANIO_Constructor: 0046C460, 0046D7E0, the two vtables, +0x04, +0x30 = 0. No component pass. Completion: 100% of the store order.
fn construct_anio(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    world.entity_mut(form).insert((
        OriginalVtable(TES_ANIO_VTABLE),
        TesModel {
            original_vtable: TES_ANIO_MODEL_VTABLE,
            ..TesModel::default()
        },
        TesObjectAnio::default(),
    ));
    set_form_type(world, form, FORM_TYPE_ANIO);
    Ok(form)
}

// IDA: 004B43A0 TESObjectANIO vt+0x1C LoadForm. The record must be ANIO; 00451210, with no 0046AB80. The chunks: DATA 0046BDA0(local, 4), no tail, since no cast succeeds on this class; EDID; MODB, MODL and MODT 0046DB00. Returns 1. Completion: 100%.
fn load_anio_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_ANIO {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            DATA => {
                let mut bytes = [0_u8; 4];
                load_data_chunk(&mut bytes, 4, DataChunkTails::default(), cx)?;
                world
                    .get_mut::<TesObjectAnio>(form)
                    .expect("an ANIO")
                    .idle_30 = FormRef::Raw(u32::from_le_bytes(bytes));
            }
            EDID => {
                cx.file.read_text(0x200)?;
            }
            MODB | MODL | MODT => load_model_arm(world, form, cx)?,
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    Ok(true)
}

// ---- TESWaterForm ----

/// IDA: 004EDE40 TESWaterForm (0xAC): TESAttackDamageForm +0x18 and TESTexture +0x20 (`TesAttackDamageForm`, `TesTexture` on the same entity), +0x2C opacity (ANAM), +0x2D flags (FNAM), +0x30 BSStringT material (MNAM), +0x38 the sound (SNAM, raw until InitItem), +0x3C the 0x64-byte DATA block, +0xA0 GNAM's three raw IDs.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct TesWaterForm {
    pub opacity_2c: u8,
    pub flags_2d: u8,
    pub material_30: BsStringT,
    pub sound_38: FormRef,
    pub data_3c: [u8; 0x64],
    pub related_a0: [FormRef; 3],
    /// IDA: N/A how many bytes of +0x3C the file has written: 004ED580 never stores +0x75..+0x77 of the form, so until then they are FormHeap bytes.
    pub data_read: u16,
}

// IDA: 004ED580 the DATA defaults. Each float constant is read from the exe where the original `fld`s it (the values are also in AC20RAW 42 18), and +0x38 is a byte store, leaving +0x39..+0x3B. Completion: 100%.
fn default_water_data(data: &mut [u8; 0x64]) {
    const WORDS: [(usize, u32); 24] = [
        (0x00, 0x3DCC_CCCD), // 00A2FAAC
        (0x04, 0x42B4_0000), // 00A430CC
        (0x08, 0x3F00_0000), // 00A3D65C
        (0x0C, 0x3F80_0000), // fld1
        (0x10, 0x4248_0000), // 00A3D8F0
        (0x14, 0x3F00_0000), // the +0x08 value, still on the stack
        (0x18, 0x3CCC_CCCD), // 00A47E78
        (0x1C, 0),
        (0x20, 0),
        (0x24, 0x46D9_999A), // 00A47E74
        (0x28, 0x4820_0000), // 00A3F4F0
        (0x2C, 0x0080_8000),
        (0x30, 0x0019_0000),
        (0x34, 0x00FF_FFFF),
        (0x3C, 0x3DCC_CCCD), // the +0x00 value, still on the stack
        (0x40, 0x3F19_999A), // 00A3F424
        (0x44, 0x3F7C_28F6), // 00A47E70
        (0x48, 0x4000_0000), // 00A379B4
        (0x4C, 0x3C23_D70A), // 00A34BA0
        (0x50, 0x3ECC_CCCD), // 00A47E6C
        (0x54, 0x3F19_999A), // the +0x40 value, after `fxch`
        (0x58, 0x3F7C_28F6), // the +0x44 value
        (0x5C, 0x4120_0000), // 00A31C80
        (0x60, 0x3D4C_CCCD), // 00A43328
    ];
    for (at, word) in WORDS {
        data[at..at + 4].copy_from_slice(&word.to_le_bytes());
    }
    data[0x38] = 0x32;
}

// IDA: 004EDDE0 TESWaterForm vt+0x14: +0x24 path freed and cleared, +0x2C = 0x4B, +0x2D = 0, +0x38 = 0, +0x30 freed and cleared, 004ED580, 0046C2E0 (vt+0 of TESAttackDamageForm and TESTexture), then the three +0xA0 words. Completion: 100% of the store order.
fn reset_water(world: &mut World, form: Entity) -> Result<(), StreamError> {
    world.get_mut::<TesTexture>(form).expect("a water").path_04 = BsStringT::default();
    {
        let mut water = world.get_mut::<TesWaterForm>(form).expect("a water");
        water.opacity_2c = 0x4B;
        water.flags_2d = 0;
        water.sound_38 = FormRef::Raw(0);
        water.material_30 = BsStringT::default();
        default_water_data(&mut water.data_3c);
    }
    reset_14_components(world, form)?;
    world
        .get_mut::<TesWaterForm>(form)
        .expect("a water")
        .related_a0 = [FormRef::Raw(0); 3];
    Ok(())
}

// IDA: 004EDE40 TESWaterForm_Constructor: 0046C460, 00468A60 on +0x18, the two vtables, 0046FFD0 on +0x20, +0x30 clear, 004ED580, +0x04, 004EDDE0. Completion: 100% of the store order.
fn construct_water(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    let mut water = TesWaterForm {
        opacity_2c: 0,
        flags_2d: 0,
        material_30: BsStringT::default(),
        sound_38: FormRef::Raw(0),
        data_3c: [0; 0x64],
        related_a0: [FormRef::Raw(0); 3],
        data_read: 0,
    };
    default_water_data(&mut water.data_3c);
    world.entity_mut(form).insert((
        OriginalVtable(TES_WATER_VTABLE),
        TesAttackDamageForm {
            original_vtable: TES_WATER_DAMAGE_VTABLE,
            damage_04: 0,
        },
        new_texture(),
        water,
    ));
    set_form_type(world, form, FORM_TYPE_WATER);
    reset_water(world, form)?;
    Ok(form)
}

// IDA: 004ED6A0 TESWaterForm vt+0x1C LoadForm. The record must be WATR; 00451210; 0046AB80(0). The chunks: GNAM 0x0C bytes into +0xA0; ANAM and FNAM a byte each; DATA 0046BDA0(+0x3C, 0x64), where TESAttackDamageForm is the one cast that succeeds, taking a tail word; EDID; MNAM alloca, size 0, 004028D0(+0x30); SNAM 004510E0 over a zeroed local; TNAM 004700E0 on +0x20. Returns 1. Completion: 100%.
fn load_water_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_WATER {
        return Ok(false);
    }
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            GNAM => {
                let mut water = world.get_mut::<TesWaterForm>(form).expect("a water");
                let mut bytes = [0_u8; 0x0C];
                for (index, word) in water.related_a0.iter().enumerate() {
                    let FormRef::Raw(word) = *word else {
                        return Err(StreamError("004EE0A0: a GNAM over resolved slots"));
                    };
                    bytes[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
                }
                cx.file.read_chunk(&mut bytes, 0x0C)?;
                for (index, word) in water.related_a0.iter_mut().enumerate() {
                    *word = FormRef::Raw(u32::from_le_bytes(
                        bytes[index * 4..index * 4 + 4]
                            .try_into()
                            .expect("four bytes"),
                    ));
                }
            }
            ANAM => {
                let mut water = world.get_mut::<TesWaterForm>(form).expect("a water");
                let mut byte = [water.opacity_2c, 0];
                cx.file.read_chunk(&mut byte, 1)?;
                water.opacity_2c = byte[0];
            }
            FNAM => {
                let mut water = world.get_mut::<TesWaterForm>(form).expect("a water");
                let mut byte = [water.flags_2d, 0];
                cx.file.read_chunk(&mut byte, 1)?;
                water.flags_2d = byte[0];
            }
            DATA => {
                let mut damage = world
                    .get::<TesAttackDamageForm>(form)
                    .expect("a water")
                    .damage_04;
                let size = cx.file.chunk_size()?;
                {
                    let mut water = world.get_mut::<TesWaterForm>(form).expect("a water");
                    let tails = DataChunkTails {
                        attack_damage: Some(&mut damage),
                        ..DataChunkTails::default()
                    };
                    load_data_chunk(&mut water.data_3c, 0x64, tails, cx)?;
                    water.data_read = water.data_read.max((size as u16).min(0x64));
                }
                world
                    .get_mut::<TesAttackDamageForm>(form)
                    .expect("a water")
                    .damage_04 = damage;
            }
            EDID => {
                cx.file.read_text(0x200)?;
            }
            MNAM => load_string(
                &mut world
                    .get_mut::<TesWaterForm>(form)
                    .expect("a water")
                    .material_30,
                cx,
            )?,
            SNAM => {
                let (bits, _) = cx.file.read_dword(0)?;
                world
                    .get_mut::<TesWaterForm>(form)
                    .expect("a water")
                    .sound_38 = FormRef::Raw(bits);
            }
            TNAM => load_texture(&mut world.get_mut::<TesTexture>(form).expect("a water"), cx)?,
            _ => {}
        }
        if !cx.file.advance_chunk()? {
            break;
        }
        chunk = cx.file.chunk_type()?;
    }
    Ok(true)
}

// ---- TESEffectShader ----

/// IDA: 004ADC00 TESEffectShader (0x110): the 0xE0-byte DATA block +0x18, and TESTexture members +0xF8 (ICON) and +0x104 (ICO2).
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct TesEffectShader {
    pub data_18: [u8; 0xE0],
    pub texture_f8: TesTexture,
    pub texture_104: TesTexture,
    /// IDA: N/A how many bytes of +0x18 the file has written: 004AC9C0 stores byte 0 alone, so until then +0x19..+0x1B are FormHeap bytes.
    pub data_read: u16,
}

// IDA: 004AC9C0 the DATA defaults, walked through the x87 stack by hand: `fst` pairs of 0.0 and 1.0 with `fxch`es between them, and byte 0 stored alone. Bits equal AC20RAW 43 144. Completion: 100%.
fn default_shader_data(data: &mut [u8; 0xE0]) {
    const ONE: u32 = 0x3F80_0000;
    let mut word = |index: usize, value: u32| {
        data[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    };
    for index in [
        4, 5, 6, 7, 8, 9, 11, 12, 14, 15, 16, 17, 18, 19, 28, 29, 30, 34, 35, 36, 37, 38, 39, 40,
        41, 42, 45, 53,
    ] {
        word(index, 0);
    }
    for index in [10, 13, 20, 21, 22, 31, 32, 33, 43, 44, 46, 50, 51, 52, 55] {
        word(index, ONE);
    }
    word(1, 5);
    word(2, 1);
    word(3, 3);
    word(23, 6);
    word(24, 5);
    word(25, 1);
    word(26, 4);
    word(27, 6);
    word(47, 0x00FF_FFFF);
    word(48, 0x00FF_FFFF);
    word(49, 0x00FF_FFFF);
    word(54, 0x3F00_0000); // 00A3D65C
    data[0] = 0;
}

// IDA: 004ACFF0 TESEffectShader vt+0x14: 004AC9C0, then a jump to 0046C2E0, which finds no slots. Completion: 100%.
fn reset_shader(world: &mut World, form: Entity) -> Result<(), StreamError> {
    default_shader_data(
        &mut world
            .get_mut::<TesEffectShader>(form)
            .expect("a shader")
            .data_18,
    );
    reset_14_components(world, form)
}

// IDA: 004ADC00 TESEffectShader_Constructor: 0046C460, the vtable, 004AC9C0, the two 0046FFD0s, +0x04, 004AC9C0 again, 0046C2E0. Completion: 100% of the store order.
fn construct_shader(world: &mut World) -> Result<Entity, StreamError> {
    let form = construct_tes_form(world)?;
    let mut shader = TesEffectShader {
        data_18: [0; 0xE0],
        texture_f8: new_texture(),
        texture_104: new_texture(),
        data_read: 0,
    };
    default_shader_data(&mut shader.data_18);
    world
        .entity_mut(form)
        .insert((OriginalVtable(TES_EFFECT_SHADER_VTABLE), shader));
    set_form_type(world, form, FORM_TYPE_EFFECT_SHADER);
    reset_shader(world, form)?;
    Ok(form)
}

// IDA: 004AC840 TESEffectShader vt+0x1C LoadForm. The record must be EFSH; vt+0x18 (0046C2F0) first; 00451210; 0046AB80(0). The chunks: DATA 0046BDA0(+0x18, 0xE0), no tail; ICON and ICO2 004700E0 on the members; EDID. Returns 1. Completion: 100%.
fn load_shader_form(
    world: &mut World,
    form: Entity,
    cx: &mut FormLoadCx<'_, '_>,
) -> Result<bool, StreamError> {
    if cx.file.record_form_type()? as u8 != FORM_TYPE_EFFECT_SHADER {
        return Ok(false);
    }
    reset_18_components(world, form)?;
    initialize_form_from_current_record(world, form, cx)?;
    set_initialized(world, form, false);
    let mut chunk = cx.file.chunk_type()?;
    while chunk != 0 {
        match chunk {
            DATA => {
                let size = cx.file.chunk_size()?;
                let mut shader = world.get_mut::<TesEffectShader>(form).expect("a shader");
                load_data_chunk(&mut shader.data_18, 0xE0, DataChunkTails::default(), cx)?;
                shader.data_read = shader.data_read.max((size as u16).min(0xE0));
            }
            ICON => load_texture(
                &mut world
                    .get_mut::<TesEffectShader>(form)
                    .expect("a shader")
                    .texture_f8,
                cx,
            )?,
            ICO2 => load_texture(
                &mut world
                    .get_mut::<TesEffectShader>(form)
                    .expect("a shader")
                    .texture_104,
                cx,
            )?,
            EDID => {
                cx.file.read_text(0x200)?;
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

// ---- the default forms ----

/// IDA: 0044AE4C the DefaultWater arm of 0044AC20: an ID 0x18 that casts to TESWaterForm is kept (and answered); otherwise FormHeap(0xAC) 004EDE40, 0046C300, vt+0xD8 006FDFF0 (the name goes nowhere), 004EDDA0 "Water\water00.dds" on +0x24, the +0xA4 prepend, then vt+0x90(0). Completion: 100% of the arm, with its global for 12d.
pub fn create_default_water(world: &mut World) -> Result<Entity, StreamError> {
    if let Some(form) = world
        .resource::<FormsById>()
        .lookup(0x18)
        .filter(|&form| world.get::<TesWaterForm>(form).is_some())
    {
        return Ok(form);
    }
    let form = construct_water(world)?;
    set_form_id(world, form, 0x18);
    world
        .get_mut::<TesTexture>(form)
        .expect("just built")
        .path_04
        .set(Some(b"Water\\water00.dds"), 0);
    world
        .resource_mut::<DataHandlerLists>()
        .water_forms_a4
        .prepend_non_null(form);
    clear_altered(world, form)?;
    Ok(form)
}

// IDA: 004EE8C0 TESWeather_MakeDefault: the two paths, then the stores below. No store reads another, so the groups here keep the values, not the exe's interleaving. Bits equal AC20RAW 2D 15E. Completion: 100%.
fn make_default_weather(weather: &mut TesWeather) {
    // 004EE8C6..004EE8DD. +0x28 is the second texture's path.
    weather.textures_18[1]
        .path_04
        .set(Some(b"Sky\\CloudsCloudy.dds"), 0);
    weather.textures_18[0]
        .path_04
        .set(Some(b"Sky\\CloudsClear.dds"), 0);
    // 004EE8E2..004EE904: 00A47E74 at +0x60 and +0x58, 00A3F4F0 at +0x64 and +0x5C.
    const NEAR: [u8; 4] = 0x46D9_999A_u32.to_le_bytes();
    const FAR: [u8; 4] = 0x4820_0000_u32.to_le_bytes();
    weather.fog_58[8..12].copy_from_slice(&NEAR);
    weather.fog_58[0..4].copy_from_slice(&NEAR);
    weather.fog_58[12..16].copy_from_slice(&FAR);
    weather.fog_58[4..8].copy_from_slice(&FAR);
    // 004EE8ED..004EE913, 004EE9C4..004EE9CE. +0x54..+0x56 stay as they were.
    weather.data_48[0..3].fill(0x33);
    weather.data_48[3..6].fill(0xFF);
    weather.data_48[6] = 0;
    weather.data_48[7] = 0xFF;
    weather.data_48[8] = 0;
    weather.data_48[9] = 0xFF;
    weather.data_48[10] = 0x32;
    weather.data_48[11] = 0;
    // 004EE916..004EE9D7: twelve colors stored twice, at +0x68 and +0xD8; then +0x98..+0xC4 once.
    const TWICE: [u32; 12] = [
        0x0084_6C66,
        0x0071_533E,
        0x0084_6D66,
        0x0010_0A05,
        0x005F_6DB6,
        0x00D1_9A83,
        0x0057_82A8,
        0x0026_1B0F,
        0x005B_4B46,
        0x009B_9B9B,
        0x006C_5953,
        0x001C_140F,
    ];
    const ONCE: [u32; 12] = [
        0x0025_2212,
        0x0085_7065,
        0x0035_221A,
        0x0028_1C15,
        0x0079_A5FF,
        0x00CA_EDFF,
        0x0049_9AE2,
        0x0076_4D34,
        0x0077_9FF2,
        0x00BE_D9DE,
        0x009D_BDFF,
        0,
    ];
    for (index, color) in TWICE.iter().enumerate() {
        let bytes = color.to_le_bytes();
        weather.colors_68[index * 4..index * 4 + 4].copy_from_slice(&bytes);
        weather.colors_68[0x70 + index * 4..0x74 + index * 4].copy_from_slice(&bytes);
    }
    for (index, color) in ONCE.iter().enumerate() {
        weather.colors_68[0x30 + index * 4..0x34 + index * 4].copy_from_slice(&color.to_le_bytes());
    }
}

/// IDA: 0044CB9C the DefaultWeather arm of 0044AC20, whose lookup has no cast: an ID 0x15E that is present stays; otherwise FormHeap(0x148) 004EE7C0, 0046C300, vt+0xD8 "DefaultWeather" (goes nowhere), 004EE8C0, the inlined +0x1C prepend. Completion: 100% of the arm.
pub fn create_default_weather(world: &mut World) -> Result<(), StreamError> {
    if world.resource::<FormsById>().lookup(0x15E).is_some() {
        return Ok(());
    }
    let form = construct_weather(world)?;
    set_form_id(world, form, 0x15E);
    make_default_weather(&mut world.get_mut::<TesWeather>(form).expect("just built"));
    world
        .resource_mut::<DataHandlerLists>()
        .weathers_1c
        .prepend_non_null(form);
    Ok(())
}

/// IDA: 0044CC6F the DefaultClimate arm of 0044AC20: an ID 0x15F that is present stays; otherwise FormHeap(0x58) 004BEC50, 0046C300, vt+0xD8 "DefaultClimate" (goes nowhere), 004BEE70, the inlined +0x14 prepend. 004BEE70: the sun and glare paths, vt+0x18 "Sky\Stars.nif" on the model, the six TNAM bytes, the list emptied, and {0x15E as TESWeather, 100} prepended, or a warning (to `DataHandlerGlobals::warnings`, since this runs outside a file). Completion: 100% of the arm.
pub fn create_default_climate(world: &mut World) -> Result<(), StreamError> {
    if world.resource::<FormsById>().lookup(0x15F).is_some() {
        return Ok(());
    }
    let form = construct_climate(world)?;
    set_form_id(world, form, 0x15F);
    let weather = world
        .resource::<FormsById>()
        .lookup(0x15E)
        .filter(|&found| {
            world
                .get::<FormIdentity>(found)
                .is_some_and(|identity| identity.form_type_04 == FORM_TYPE_WEATHER)
        });
    {
        let mut climate = world.get_mut::<TesClimate>(form).expect("just built");
        climate.textures_38[0].path_04.set(Some(b"Sky\\Sun.dds"), 0);
        climate.textures_38[1]
            .path_04
            .set(Some(b"Sky\\SunGlare.dds"), 0);
        climate.model_18.path_04.set(Some(b"Sky\\Stars.nif"), 0);
        climate.timing_50 = [0x24, 0x2A, 0x6C, 0x72, 0, 0xC3];
        climate.weathers_30.clear();
        if let Some(weather) = weather {
            climate.weathers_30.insert(
                0,
                WeatherEntry {
                    weather,
                    chance: 100,
                },
            );
        }
    }
    if weather.is_none() {
        world.resource_mut::<DataHandlerGlobals>().warnings.push(
            "Unable to add default weather to default climate.  ( TESClimate::MakeDefault() )"
                .to_owned(),
        );
    }
    world
        .resource_mut::<DataHandlerLists>()
        .climates_14
        .prepend_non_null(form);
    Ok(())
}

/// IDA: 0041C898.. the three effect shaders of 0041BB80 (0x144 effectAbsorb, 0x145 effectReflect, 0x146 LifeDetected): an ID that casts to TESEffectShader stays; otherwise FormHeap(0x110) 004ADC00, 0046C300, vt+0xD8 (goes nowhere), vt+0x90(0), the +0xAC prepend. Completion: 100% of the arm.
pub fn create_default_effect_shaders(world: &mut World) -> Result<Vec<Entity>, StreamError> {
    let mut statics = Vec::with_capacity(3);
    for id in [0x144, 0x145, 0x146] {
        if let Some(present) = world
            .resource::<FormsById>()
            .lookup(id)
            .filter(|&form| world.get::<TesEffectShader>(form).is_some())
        {
            statics.push(present);
            continue;
        }
        let form = construct_shader(world)?;
        set_form_id(world, form, id);
        clear_altered(world, form)?;
        world
            .resource_mut::<DataHandlerLists>()
            .effect_shaders_ac
            .prepend_non_null(form);
        statics.push(form);
    }
    Ok(statics)
}

/// IDA: this family's classes. A type absent here takes `AbsentArm`.
pub const CLASSES: &[FormClass] = &[
    FormClass {
        form_type: FORM_TYPE_WEATHER,
        vtable: TES_WEATHER_VTABLE,
        construct: construct_weather,
        load_form: load_weather_form,
        reset_14: reset_weather,
        reset_18: destroy_weather,
        editor_id: default_editor_id,
        init_item: no_init_item,
        slots: &[],
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_CLIMATE,
        vtable: TES_CLIMATE_VTABLE,
        construct: construct_climate,
        load_form: load_climate_form,
        reset_14: reset_climate,
        reset_18: reset_18_components,
        editor_id: default_editor_id,
        init_item: no_init_item,
        slots: &[],
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_LOAD_SCREEN,
        vtable: TES_LOAD_SCREEN_VTABLE,
        construct: construct_load_screen,
        load_form: load_load_screen_form,
        reset_14: reset_14_components,
        reset_18: destroy_load_screen,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_load_screen,
        slots: &[ComponentSlot::Description, ComponentSlot::Texture],
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_ANIO,
        vtable: TES_ANIO_VTABLE,
        construct: construct_anio,
        load_form: load_anio_form,
        reset_14: reset_14_components,
        reset_18: reset_18_components,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_object_anio,
        slots: &[ComponentSlot::Model],
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_WATER,
        vtable: TES_WATER_VTABLE,
        construct: construct_water,
        load_form: load_water_form,
        reset_14: reset_water,
        reset_18: reset_18_components,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_water,
        slots: &[ComponentSlot::AttackDamage, ComponentSlot::Texture],
        special_arm: None,
    },
    FormClass {
        form_type: FORM_TYPE_EFFECT_SHADER,
        vtable: TES_EFFECT_SHADER_VTABLE,
        construct: construct_shader,
        load_form: load_shader_form,
        reset_14: reset_shader,
        reset_18: reset_18_components,
        editor_id: default_editor_id,
        init_item: super::forms_init::init_flag_only,
        slots: &[],
        special_arm: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_sort_matches_00506f00() {
        let mut world = World::new();
        // Twelve entries, so the gaps are 4 and 1. Equal chances end in the order these passes leave them, walked by hand from 00506F20..00506FBE, which a stable sort wouldn't give.
        let chances = [5, 9, 1, 9, 3, 7, 7, 2, 8, 0, 6, 4];
        let original: Vec<Entity> = chances.iter().map(|_| world.spawn_empty().id()).collect();
        let mut entries: Vec<WeatherEntry> = chances
            .iter()
            .zip(&original)
            .map(|(&chance, &weather)| WeatherEntry { weather, chance })
            .collect();
        sort_weather_list(&mut entries);
        let order: Vec<usize> = entries
            .iter()
            .map(|entry| {
                original
                    .iter()
                    .position(|&form| form == entry.weather)
                    .unwrap()
            })
            .collect();
        assert_eq!(order, [1, 3, 8, 6, 5, 10, 0, 11, 4, 7, 2, 9]);
        let mut empty: Vec<WeatherEntry> = Vec::new();
        sort_weather_list(&mut empty);
    }
}
