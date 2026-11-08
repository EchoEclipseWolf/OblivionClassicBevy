//! `00542F20`, the Sky's update, as `004424D0` calls it from the frame outside menu mode (`0040DEA1`), for its state: the game hour, the weather pick and transition (`005422F0`), the moons' phase day and the flags. Its graphics (colors, sun, clouds, stars, the moons' and the precipitation's updates, the HDR blend, the background color) are counted boundaries, the sky renderer being M6's.
//!
//! Evidence: `evidence/gameplay-run002-2026-11-07/` (run-w): the 59 weather picks at frame 62, through `004EECE0`.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;

use super::ai_package::game_hour;
use super::data_handler::DataHandlerLists;
use super::forms_environment::{TesClimate, TesWeather, WeatherEntry};
use super::forms_init::{form_id, lookup};
use super::forms_references::{save_load_active, save_load_flags};
use super::forms_region::{RegionDataKind, TesRegion, TesRegionData};
use super::game_settings::GameSettingCollection;
use super::player::PlayerCharacter;
use super::sky::{Sky, SkyGlobals};

// IDA: 00A3F398 and 00A2F928: 1/255 and 1.0 as doubles.
const ONE_255TH: f64 = 0.003_921_568_859_368_563;
const ONE: f64 = 1.0;

/// IDA: N/A the counts of the update's graphics calls, which the port doesn't make: 00540850 (the HDR blend), 0053D6C0 (the precipitation's release), 005418F0, 00541DD0, 0053FF90, 00542590, the parts' vt+0x0C, 0053F4C0 and 00498060.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct SkyUpdateBoundaries {
    pub calls: std::collections::BTreeMap<u32, u32>,
}

// IDA: N/A a helper, with no address of its own.
fn count(world: &mut World, address: u32) {
    *world
        .get_resource_or_init::<SkyUpdateBoundaries>()
        .calls
        .entry(address)
        .or_default() += 1;
}

// IDA: 00402C10 TimeGlobals_GetTruncatedDaysPassedOrOne: +0x10 or 1.0, truncated.
fn days_passed(world: &World) -> u32 {
    let value = world
        .get_resource::<super::default_forms::TimeGlobals>()
        .and_then(|globals| globals.globals_b332e0[4])
        .and_then(|global| world.get::<super::global::TESGlobal>(global))
        .map_or(1.0, |global| f32::from_bits(global.value_bits_24));
    recomp_core::x87::ftol(value) as u32
}

// IDA: a static GameSetting's float, by its address: 00B36628 fWeatherTransMax, 00B36630 fWeatherTransMin, 00B36638 fWeatherTransAccel.
fn setting(world: &World, address: u32) -> Result<f32, StreamError> {
    world
        .get_resource::<GameSettingCollection>()
        .and_then(|settings| settings.raw_at(address))
        .map(f32::from_bits)
        .ok_or(StreamError(
            "00542F20: a weather setting with no static GameSetting",
        ))
}

/// IDA: 004EECE0 TESWeatherList_PickWeather: the chances summed; none answers null. 0047DF80(0), unsigned modulo the sum, then the first entry whose span holds it. Running out of entries answers the last one read. Completion: 100%.
/// # Errors
/// The draw's.
pub fn pick_weather(
    entries: &[WeatherEntry],
    draw: &mut dyn FnMut() -> Result<i32, StreamError>,
) -> Result<Option<Entity>, StreamError> {
    let total = entries
        .iter()
        .fold(0_u32, |sum, entry| sum.wrapping_add(entry.chance));
    if total == 0 {
        return Ok(None);
    }
    let roll = (draw()? as u32) % total;
    let mut start = 0_u32;
    let mut end = u32::MAX;
    let mut last = None;
    for entry in entries {
        end = end.wrapping_add(entry.chance);
        last = Some(entry.weather);
        if start <= roll && roll <= end {
            return Ok(last);
        }
        start = start.wrapping_add(entry.chance);
    }
    Ok(last)
}

// IDA: 004A2E90 TESRegion_PickWeather: +0x24 = 0; with a data list, its first type 3 (004A4460), which is a TESRegionDataWeather, picks through 004EECE0 on its +0x08.
fn region_pick_weather(
    world: &mut World,
    region: Entity,
    draw: &mut dyn FnMut() -> Result<i32, StreamError>,
) -> Result<(), StreamError> {
    let entries = {
        let data = &world.get::<TesRegion>(region).expect("a region").data_18;
        data.items
            .iter()
            .filter_map(|&item| world.get::<TesRegionData>(item))
            .find(|item| item.kind.type_number() == 3)
            .map(|item| match &item.kind {
                RegionDataKind::Weather { weathers_08 } => weathers_08.clone(),
                _ => Vec::new(),
            })
    };
    world
        .get_mut::<TesRegion>(region)
        .expect("a region")
        .field_24 = 0;
    if let Some(entries) = entries {
        let picked = pick_weather(&entries, draw)?;
        let id = picked.map_or(0, |weather| form_id(world, weather));
        world
            .get_mut::<TesRegion>(region)
            .expect("a region")
            .field_24 = id;
    }
    Ok(())
}

// IDA: N/A the Sky out of its resource for an update, and back.
fn take_sky(world: &mut World) -> Option<Box<Sky>> {
    world
        .get_resource_mut::<SkyGlobals>()?
        .singleton_b365c4
        .take()
}

// IDA: N/A the Sky back in its resource.
fn put_sky(world: &mut World, sky: Box<Sky>) {
    world.resource_mut::<SkyGlobals>().singleton_b365c4 = Some(sky);
}

// IDA: 005422F0 Sky_UpdateWeather. With a climate (+0x0C): when +0xFC bit 0 is set, +0x18 is null, or the hours since +0xD4 (24 added when +0xD4 is past +0xD0) pass (255 - climate +0x54) * 22 / 255 + 1 with no +0x14, +0x18 takes the climate's pick, or form 0x15E, and each region of DataHandler +0xBC picks (004A2E90). The weather wanted is +0x1C; or with none, the player's region's +0x24 unless +0xDC is 2, else +0x18; a reload with +0x10 set and save flag bit 4 clear keeps +0x1C. A new one, with no +0x14 or +0xFC bit 4, moves +0x10 to +0x14 (bit 4: +0x14 = 0 and the precipitation released, 0053D6C0), takes +0x10, copies +0xD0 to +0xD4 unless a save is active, blends (00540850) and sets bit 0; otherwise bit 0 clears. Then +0xD8, the transition: 0.0 with no +0x10, 1.0 with no +0x14, else the hours over fWeatherTransMin + (Max - Min) * (+0x10's +0x4B / 255), stored as a float; bit 3 eases it by fWeatherTransAccel. With +0x14, 00540850 again. Past 1.0, bit 3 clears, +0xD8 = 1.0, +0xF4 = 0.0 and +0x14 = 0. Completion: 100%, with 00540850 and 0053D6C0 counted.
fn update_weather(
    world: &mut World,
    sky: &mut Sky,
    draw: &mut dyn FnMut() -> Result<i32, StreamError>,
) -> Result<(), StreamError> {
    let Some(climate) = lookup(world, sky.first_climate_0c).filter(|_| sky.first_climate_0c != 0)
    else {
        return Ok(());
    };
    let pick = if sky.dword_fc & 1 != 0 || sky.weather_18 == 0 {
        true
    } else {
        let extra = if f64::from(sky.float_d4) > f64::from(sky.float_d0) {
            24.0
        } else {
            0.0
        };
        let elapsed = extra + f64::from(sky.float_d0) - f64::from(sky.float_d4);
        let length = world
            .get::<TesClimate>(climate)
            .map_or(0, |data| data.timing_50[4]);
        let limit = f64::from((255 - i32::from(length)) * 22) * ONE_255TH + ONE;
        // 00542364 `fcompp; test ah, 5; jp`: the pick when the limit is below.
        limit < elapsed && sky.weather_14 == 0
    };
    if pick {
        let entries = world
            .get::<TesClimate>(climate)
            .map(|data| data.weathers_30.clone())
            .unwrap_or_default();
        let picked = pick_weather(&entries, draw)?;
        sky.weather_18 = match picked {
            Some(weather) => form_id(world, weather),
            // 0054238D: form 0x15E through the TESWeather cast.
            None => lookup(world, 0x15E)
                .filter(|&form| world.get::<TesWeather>(form).is_some())
                .map_or(0, |form| form_id(world, form)),
        };
        let regions: Vec<Entity> = world
            .get_resource::<DataHandlerLists>()
            .map(|lists| lists.regions_bc.items().collect())
            .unwrap_or_default();
        for region in regions {
            region_pick_weather(world, region, draw)?;
        }
    }
    let active = save_load_active(world)?;
    let keep_override = sky.first_weather_10 != 0 && active && save_load_flags(world)? & 0x10 == 0;
    let wanted = if keep_override || sky.weather_override_1c != 0 {
        sky.weather_override_1c
    } else {
        let region = super::ai_conditions::the_player(world)
            .and_then(|player| world.get::<PlayerCharacter>(player))
            .and_then(|player| player.region_6e8)
            .and_then(|region| world.get::<TesRegion>(region))
            .map_or(0, |region| region.field_24);
        if region != 0 && sky.dword_dc != 2 {
            region
        } else {
            sky.weather_18
        }
    };
    if wanted != 0
        && (sky.weather_14 == 0 || sky.dword_fc & 0x10 != 0)
        && wanted != sky.first_weather_10
    {
        if sky.dword_fc & 0x10 != 0 {
            sky.weather_14 = 0;
            if sky.precipitation_38.is_some() {
                count(world, 0x0053_D6C0);
            }
        } else {
            sky.weather_14 = sky.first_weather_10;
        }
        sky.first_weather_10 = wanted;
        if !active {
            sky.float_d4 = sky.float_d0;
        }
        count(world, 0x0054_0850);
        sky.dword_fc |= 1;
    } else {
        sky.dword_fc &= !1;
    }
    if sky.first_weather_10 == 0 {
        sky.float_d8 = 0.0;
    } else if sky.weather_14 == 0 {
        sky.float_d8 = 1.0;
    } else {
        let extra = if f64::from(sky.float_d4) > f64::from(sky.float_d0) {
            24.0
        } else {
            0.0
        };
        let elapsed = extra + f64::from(sky.float_d0) - f64::from(sky.float_d4);
        let speed = lookup(world, sky.first_weather_10)
            .and_then(|weather| world.get::<TesWeather>(weather))
            .map_or(0, |weather| weather.data_48[3]);
        let max = f64::from(setting(world, 0x00B3_6628)?);
        let min = f64::from(setting(world, 0x00B3_6630)?);
        // 00542509: the span is stored as a float before the divide.
        let span =
            recomp_core::x87::stored_float(min + (max - min) * (f64::from(speed) * ONE_255TH));
        sky.float_d8 = recomp_core::x87::stored_float(elapsed / f64::from(span));
    }
    if sky.dword_fc & 8 != 0 {
        let accel = f64::from(setting(world, 0x00B3_6638)?);
        sky.float_d8 = recomp_core::x87::stored_float(
            (f64::from(sky.float_d8) - f64::from(sky.float_f4)) * (accel + ONE)
                + f64::from(sky.float_f4),
        );
    }
    if sky.weather_14 != 0 {
        count(world, 0x0054_0850);
    }
    // 00542556 `fcom; test ah, 5; jp`: past 1.0 ends the transition.
    if 1.0 < sky.float_d8 {
        sky.dword_fc &= !8;
        sky.float_d8 = 1.0;
        sky.weather_14 = 0;
        sky.float_f4 = 0.0;
    }
    Ok(())
}

// IDA: 0053C5E0: with climate +0x55's low six bits as the phase length, days passed modulo eight lengths, over the length, against the static 00B365BC: a change stores it and answers 1.
fn moon_phase_changed(world: &mut World, sky: &Sky) -> bool {
    let Some(climate) = lookup(world, sky.first_climate_0c).filter(|_| sky.first_climate_0c != 0)
    else {
        return false;
    };
    let length = u32::from(
        world
            .get::<TesClimate>(climate)
            .map_or(0, |data| data.timing_50[5]),
    ) & 0x3F;
    if length == 0 {
        return false;
    }
    let phase = days_passed(world) % (length * 8) / length;
    let mut globals = world.get_resource_or_init::<SkyUpdateGlobals>();
    if phase == globals.moon_phase_b365bc {
        return false;
    }
    globals.moon_phase_b365bc = phase;
    true
}

/// IDA: 00B365BC, the moon phase 0053C5E0 last saw (BSS).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SkyUpdateGlobals {
    pub moon_phase_b365bc: u32,
}

/// IDA: 00542F20 Sky_Update(time) on the Sky, from 004424D0. A save active without save flag bit 10 returns. +0xD0 = the game hour. 005422F0. A save active sets +0xFC bit 0. With +0xFC bit 5 and a day passed since +0xEC, or a save active: a moon phase change (0053C5E0) marks each moon's +0x70, when clear, 1 (2 with a save active), and +0xEC = the days. A flash (+0xE4 above 0.0) is a stop. Last, +0xFC bits 0 and 1 clear. Completion: 100% of the state named; the graphics calls are counted.
/// TODO: [ ] 00542F63..00542FC1 +0xFC bit 2 from the camera against the player's cell's water, and 00542FC8..00543002 the root's and the sun's node flag 1 by +0xDC: both need the scene graph, M6. 005431D1 00498060 the background color.
/// # Errors
/// The draw's, a missing setting, or a flash.
pub fn sky_update(
    world: &mut World,
    draw: &mut dyn FnMut() -> Result<i32, StreamError>,
) -> Result<(), StreamError> {
    let Some(mut sky) = take_sky(world) else {
        return Ok(());
    };
    let result = sky_update_with(world, &mut sky, draw);
    put_sky(world, sky);
    result
}

// IDA: N/A `sky_update`'s body, with the Sky out of its resource.
fn sky_update_with(
    world: &mut World,
    sky: &mut Sky,
    draw: &mut dyn FnMut() -> Result<i32, StreamError>,
) -> Result<(), StreamError> {
    let active = save_load_active(world)?;
    if active && save_load_flags(world)? & 0x400 == 0 {
        return Ok(());
    }
    sky.float_d0 = game_hour(world);
    update_weather(world, sky, draw)?;
    if save_load_active(world)? {
        sky.dword_fc |= 1;
    }
    for address in [0x0054_18F0, 0x0054_1DD0, 0x0053_FF90, 0x0054_2590] {
        count(world, address);
    }
    let days = days_passed(world);
    let active = save_load_active(world)?;
    if (sky.dword_fc & 0x20 != 0 && sky.dword_ec < days) || active {
        if moon_phase_changed(world, sky) {
            let mark = if active { 2 } else { 1 };
            for moon in [sky.masser_30.as_mut(), sky.secunda_34.as_mut()]
                .into_iter()
                .flatten()
            {
                if moon.byte_70 == 0 {
                    moon.byte_70 = mark;
                }
            }
        }
        sky.dword_ec = days;
    }
    if sky.float_e4 > 0.0 {
        return Err(StreamError("00543185: a lightning flash's fade"));
    }
    count(world, 0x0049_8060);
    sky.dword_fc &= !3;
    Ok(())
}
