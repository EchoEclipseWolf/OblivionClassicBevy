//! Phase 14c: TESPackage for the AI pass: LowProcess's time stamps (the time globals are in `forms/global.rs`), 005672A0 the procedure type, the location getters 00566940, 00566A40, 00566B30, 00566DC0 at-location and 00684B30, and package selection 00569020 and 005692E0 with the schedule tests.
//!
//! x87: the main thread is at 24 bits here, so each `f32` step is an fadd, fsub or fmul in the exe's grouping; a `fild` is exact and rounds at the next operation. Evidence: objdump of each address named here; P14-RUN-004.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_core::x87;

use super::ai_process::AiProcessGlobals;
use super::condition_tables::{PROCEDURE_BLOCK, PROCEDURE_ROWS};
#[cfg(test)]
use crate::tes::actor::actor_height;
use crate::tes::actor::actor_sitting_state;
use crate::tes::actor::{
    Actor, Character, MobileObject, TesObjectRefr, actor_within_radius, process_of,
};
use crate::tes::actor_process::{BaseProcess, HighProcess, LowProcess, MiddleHighProcess};
use crate::tes::base_extra_list::{ExtraData, package_start_entry};
use crate::tes::default_forms::DefaultFormStatics;
use crate::tes::form::FormIdentity;
use crate::tes::form_components::{FormRef, form_bound_radius};
use crate::tes::forms_ai::{PackageLocation, PackageTarget, TesPackage};
use crate::tes::forms_init::form_type;
use crate::tes::forms_references::{parent_cell, reference_position, reference_world};
use crate::tes::forms_world::{
    cell_first_reference, cell_grid_x, cell_grid_y, cell_is_interior, cell_world,
};
use crate::tes::game_settings::GameSettingCollection;
use crate::tes::global::game_day_of_week;

/// IDA: 00A32048, FLT_MAX.
pub const FLT_MAX: f32 = f32::from_bits(0x7F7F_FFFF);

// ---- LowProcess's time stamps, from the time globals (`forms/global.rs`) ----

// ---- package data ----

/// IDA: N/A a package's component.
#[must_use]
pub fn package_data(world: &World, package: Entity) -> &TesPackage {
    world.get::<TesPackage>(package).expect("a package")
}

// IDA: N/A a helper, with no address of its own.
pub(crate) fn resolved(slot: FormRef, where_: &'static str) -> Result<Option<Entity>, GameError> {
    match slot {
        FormRef::Resolved(form) => Ok(form),
        FormRef::Raw(0) => Ok(None),
        FormRef::Raw(_) => Err(GameError::from_legacy_message(where_)),
    }
}

/// IDA: 00569820-style: a location's +0x08 once InitItem has run.
pub fn location_object(location: &PackageLocation) -> Result<Option<Entity>, GameError> {
    resolved(
        location.object_08,
        "00569820: an unresolved location object",
    )
}

/// IDA: 005697E0 the +0x08 of a type 0 location, else 0.
pub fn location_reference(
    world: &World,
    location: &PackageLocation,
) -> Result<Option<Entity>, GameError> {
    if location.type_00 != 0 {
        return Ok(None);
    }
    Ok(location_object(location)?.filter(|&form| world.get::<TesObjectRefr>(form).is_some()))
}

// IDA: 00569800 PackageLocation_GetCell.
fn location_cell(location: &PackageLocation) -> Result<Option<Entity>, GameError> {
    if location.type_00 != 1 {
        return Ok(None);
    }
    location_object(location)
}

// IDA: N/A no address of its own; its comments cite 00569E60.
fn target_value(target: &PackageTarget) -> Result<Option<Entity>, GameError> {
    resolved(target.value_04, "00569E60: an unresolved package target")
}

/// IDA: 00569E60 the +0x04 of a type 0 target.
pub fn target_reference(
    world: &World,
    target: &PackageTarget,
) -> Result<Option<Entity>, GameError> {
    if target.type_00 != 0 {
        return Ok(None);
    }
    Ok(target_value(target)?.filter(|&form| world.get::<TesObjectRefr>(form).is_some()))
}

/// IDA: 00569E70 the +0x04 of a type 1 target.
pub fn target_object(target: &PackageTarget) -> Result<Option<Entity>, GameError> {
    if target.type_00 != 1 {
        return Ok(None);
    }
    target_value(target)
}

/// IDA: 00569E80 the +0x04 of a type 2 target, as a value.
#[must_use]
pub fn target_object_type(target: &PackageTarget) -> u32 {
    if target.type_00 != 2 {
        return 0;
    }
    match target.value_04 {
        FormRef::Raw(value) => value,
        FormRef::Resolved(_) => 0,
    }
}

/// IDA: 00567770 the table at 00567798.
#[must_use]
pub const fn package_is_combat_like(package: &TesPackage) -> bool {
    matches!(
        package.type_20 as u8,
        0x0C..=0x14 | 0x16..=0x18 | 0x1B | 0x1C | 0x1E | 0x20
    )
}

/// IDA: `[B152B0 + type * 4][index]`, unbounded in the exe: a row read past its 0x2C goes on into what follows it in memory, and `PROCEDURE_BLOCK` keeps that. Only a read outside the block stops. +0x18 of -1 reads the block's last dword as a pointer, which the host stops on. Completion: 100%.
pub fn procedure_at(world: &World, package: Entity, index: u32) -> Result<u32, GameError> {
    let row = package_data(world, package).field_18 as usize;
    let start = *PROCEDURE_ROWS.get(row).ok_or(GameError::unclassified(
        0x00649459,
        "00649459: +0x18 outside the 00B152B0 table",
    ))?;
    PROCEDURE_BLOCK
        .get(usize::from(start) + index as usize)
        .copied()
        .ok_or(GameError::unclassified(
            0x00649460,
            "00649460: a procedure index past the block",
        ))
}

/// IDA: 005672A0 TESPackage_InitProcedureType: +0x18 from the type, target and location. Its tables 0056764C, 005676D0, 005676EC and 0056770C read from the exe. Type 0 with a target whose three getters are all null leaves +0x18 alone (00567329, where the C++ writes -1). -1 warns and returns (00567606, where the C++ stops), and the package stays selectable. Completion: 100%.
pub fn package_init_procedure(world: &mut World, package: Entity) -> Result<(), GameError> {
    let data = package_data(world, package);
    let target = data.target_28;
    let location = data.location_24;
    let kind = data.type_20 as u8;
    let any_target = |world: &World, target: &PackageTarget| -> Result<bool, GameError> {
        Ok(target_reference(world, target)?.is_some()
            || target_object(target)?.is_some()
            || target_object_type(target) != 0)
    };
    let set: Option<i32> = match kind {
        0 => match &target {
            None => Some(i32::from(
                location.is_some_and(|location| location.radius() != 0),
            )),
            Some(target) => {
                if !any_target(world, target)? {
                    None
                } else {
                    Some(match target.type_00 {
                        0 => {
                            let reference = target_reference(world, target)?;
                            let base = reference
                                .and_then(|form| world.get::<TesObjectRefr>(form))
                                .and_then(|data| data.base_form_1c)
                                .ok_or(GameError::unclassified(
                                    0x005672FF,
                                    "005672FF: a target reference with no base",
                                ))?;
                            match form_type(world, base) {
                                18 | 23 | 24 | 28 | 30 | 31 | 32 | 36 => 2,
                                35 => 22,
                                _ => 3,
                            }
                        }
                        1 => {
                            let object = target_object(target)?.expect("tested above");
                            if form_type(world, object) == 35 {
                                22
                            } else {
                                26
                            }
                        }
                        2 => match target_object_type(target) {
                            1 | 7 | 11 | 12 | 16 => 2,
                            15 => 22,
                            _ => 26,
                        },
                        _ => 3,
                    })
                }
            }
        },
        1 => Some(match &target {
            Some(target) if any_target(world, target)? => 7,
            _ => -1,
        }),
        2 => Some(match &target {
            Some(target) if any_target(world, target)? => {
                let actor_target = match target.type_00 {
                    0 => {
                        let base = target_reference(world, target)?
                            .and_then(|form| world.get::<TesObjectRefr>(form))
                            .and_then(|data| data.base_form_1c)
                            .ok_or(GameError::unclassified(
                                0x005673F5,
                                "005673F5: a target reference with no base",
                            ))?;
                        matches!(form_type(world, base), 35 | 36)
                    }
                    1 => {
                        let object = target_object(target)?.expect("tested above");
                        matches!(form_type(world, object), 35 | 36)
                    }
                    2 => matches!(target_object_type(target), 15 | 16),
                    _ => false,
                };
                if actor_target { 8 } else { 9 }
            }
            _ => -1,
        }),
        3 => Some(5),
        4 => Some(4),
        5 => Some(1),
        6 => Some(0),
        7 => Some(29),
        8 => Some(30),
        9 => Some(32),
        0x0A => Some(35),
        0x0B => Some(34),
        0x0C => Some(12),
        0x0E => Some(13),
        0x0F => Some(11),
        0x11 => Some(20),
        0x12 => Some(10),
        0x13 => Some(15),
        0x15 => Some(21),
        0x16 => Some(23),
        0x17 => Some(24),
        0x18 => Some(25),
        0x19 => Some(27),
        0x1A => Some(28),
        0x1C => Some(33),
        0x1D => Some(36),
        0x1E => Some(37),
        0x1F => Some(38),
        0x20 => Some(39),
        _ => Some(-1),
    };
    if let Some(value) = set {
        world
            .get_mut::<TesPackage>(package)
            .expect("a package")
            .field_18 = value as u32;
    }
    if package_data(world, package).field_18 == 0xFFFF_FFFF {
        let id = world
            .get::<FormIdentity>(package)
            .expect("a form")
            .form_id_0c;
        world
            .resource_mut::<AiProcessGlobals>()
            .warnings
            .push(format!(
                "Package '{id:08X}' is not a valid package because it is missing necessary Target Or Location"
            ));
    }
    Ok(())
}

// ---- references ----

// ---- the location getters ----

// IDA: N/A no address of its own; its comments cite 0041E9B0.
fn start_location_form(world: &World, reference: Entity) -> Result<Option<Entity>, GameError> {
    let Some(entry) = package_start_entry(world, reference) else {
        return Ok(None);
    };
    match (entry.resolved_0c, &entry.data) {
        (Some(form), _) => Ok(form),
        (None, ExtraData::PackageStart { target_0c: 0, .. }) => Ok(None),
        _ => Err(GameError::unclassified(
            0x0041E9B0,
            "0041E9B0: an unresolved ExtraPackageStartLocation",
        )),
    }
}

// IDA: the location type 4 and 5 arm: the actor's process's current package is this one; then the process's +0x30 (vt+0x3B8 00780DE0), or with none the actor.
fn package_process_reference(world: &World, package: Entity, actor: Entity) -> Option<Entity> {
    let process = process_of(world, actor)?;
    if world.get::<BaseProcess>(process)?.editor_package_08 != Some(package) {
        return None;
    }
    world
        .get::<LowProcess>(process)
        .and_then(|low| low.field_30)
        .or(Some(actor))
}

/// IDA: 00566940 TESPackage_GetLocationWorldspace(actor); its table at 00566A28. Completion: 100%.
pub fn package_location_world(
    world: &World,
    package: Entity,
    actor: Entity,
) -> Result<Option<Entity>, GameError> {
    let location = package_data(world, package).location_24;
    let Some(location) = location.filter(|location| location.type_00 != 2) else {
        // 004D79B0.
        return Ok(
            start_location_form(world, actor)?.filter(|&form| form_type(world, form) == 0x35)
        );
    };
    Ok(match location.type_00 {
        1 => cell_world(world, location_cell(&location)?),
        0 => location_reference(world, &location)?.and_then(|form| reference_world(world, form)),
        3 => world
            .get::<Actor>(actor)
            .expect("an actor")
            .starting_location_f8
            .filter(|&form| form_type(world, form) == 0x35),
        4 | 5 => package_process_reference(world, package, actor)
            .and_then(|form| reference_world(world, form)),
        _ => None,
    })
}

/// IDA: 00566A40 TESPackage_GetLocationCell(actor). Completion: 100%.
pub fn package_location_cell(
    world: &World,
    package: Entity,
    actor: Entity,
) -> Result<Option<Entity>, GameError> {
    let location = package_data(world, package).location_24;
    let Some(location) = location.filter(|location| location.type_00 != 2) else {
        // 004D79D0.
        return Ok(
            start_location_form(world, actor)?.filter(|&form| form_type(world, form) == 0x30)
        );
    };
    Ok(match location.type_00 {
        1 => location_cell(&location)?,
        0 => location_reference(world, &location)?.and_then(|form| parent_cell(world, form)),
        3 => world
            .get::<Actor>(actor)
            .expect("an actor")
            .starting_location_f8
            .filter(|&form| form_type(world, form) == 0x30 && cell_is_interior(world, form)),
        4 | 5 => package_process_reference(world, package, actor)
            .and_then(|form| parent_cell(world, form)),
        _ => None,
    })
}

/// IDA: 00566B30 TESPackage_GetLocationPosition(out, actor). Every path is a copy of floats, or the grid `<<12`, `fild`, a float store. g_niPoint3Zero 00B3F9A8 for a miss. Completion: 100%.
pub fn package_location_position(
    world: &World,
    package: Entity,
    actor: Entity,
) -> Result<[f32; 3], GameError> {
    let zero = [0.0; 3];
    let location = package_data(world, package).location_24;
    let Some(location) = location.filter(|location| location.type_00 != 2) else {
        // 004D79F0: the 0x1E extra's position, else vt+0x174.
        if let Some(entry) = package_start_entry(world, actor)
            && let ExtraData::PackageStart { location_10, .. } = &entry.data
        {
            return Ok(location_10.map(f32::from_bits));
        }
        return reference_position(world, actor);
    };
    match location.type_00 {
        1 => {
            let Some(cell) = location_cell(&location)? else {
                return Ok(zero);
            };
            if let Some(first) = cell_first_reference(world, cell) {
                return reference_position(world, first);
            }
            if cell_is_interior(world, cell) {
                return Ok(zero);
            }
            Ok([
                (cell_grid_x(world, cell) << 12) as f32,
                (cell_grid_y(world, cell) << 12) as f32,
                0.0,
            ])
        }
        0 => {
            let Some(reference) = location_reference(world, &location)? else {
                return Ok(zero);
            };
            // 004D74D0: a creature of creature type 4 asks 00625290, which with no 3D gives vt+0x174.
            let data = world.get::<TesObjectRefr>(reference).expect("a reference");
            if data
                .base_form_1c
                .and_then(|base| world.get::<crate::tes::forms_actors::TesCreature>(base))
                .is_some_and(|creature| creature.data_104[0] == 4)
                && data.loaded_3d_3c != 0
            {
                return Err(GameError::unclassified(
                    0x00625290,
                    "00625290: the creature EntryPoint node",
                ));
            }
            reference_position(world, reference)
        }
        // vt+0xF4 00628E40: +0xE8.
        3 => Ok(world
            .get::<Actor>(actor)
            .expect("an actor")
            .starting_position_e8
            .map(f32::from_bits)),
        4 | 5 => match package_process_reference(world, package, actor) {
            Some(form) => reference_position(world, form),
            None => Ok(zero),
        },
        _ => Ok(zero),
    }
}

/// IDA: 0047F6F0 the length test: `(y*y + x*x) + z*z` against `r*r`: -1 only when the limit is above it, 1 only when below, otherwise 0, so a NaN is 0 (0047F722, 0047F733; the C++ gives -1). Completion: 100%.
#[must_use]
pub fn compare_length(delta: [f32; 3], radius: f32) -> i32 {
    let length = (delta[1] * delta[1] + delta[0] * delta[0]) + delta[2] * delta[2];
    let limit = radius * radius;
    if limit > length {
        -1
    } else if limit < length {
        1
    } else {
        0
    }
}

// IDA: 00B36B28 iAIDistanceRadiusMinLocation, default 0x64 (009E7BE0) with no collection entry.
fn default_location_radius(world: &World) -> i32 {
    world
        .get_resource::<GameSettingCollection>()
        .and_then(|settings| settings.raw_at(0x00B3_6B28))
        .map_or(0x64, |raw| raw as i32)
}

// IDA: 00566E65 a bound radius plus a float: `fild`, `fadd`, which rounds once, then 009828C0. The C++ rounds twice in one arm and not at all in the other, which shows only past 2^24.
fn bound_plus(bound: i32, extra: f32) -> i32 {
    x87::ftol((f64::from(bound) + f64::from(extra)) as f32)
}

/// IDA: 00566DC0 TESPackage_IsAtLocation(actor, exact, radius). The furniture reference and marker point are tested at both upper process levels.
/// TODO: [ ] 00566F26 the mounted entry point and 005671A4 the active combat-controller test.
pub fn package_at_location(
    world: &mut World,
    package: Entity,
    actor: Entity,
    exact: bool,
    radius_override: f32,
) -> Result<bool, GameError> {
    let location = package_data(world, package).location_24;
    let data = world.get::<Actor>(actor).expect("an actor");
    if location.is_some_and(|location| location.type_00 == 3) && data.starting_location_f8.is_none()
    {
        return Ok(true);
    }
    let space = package_location_world(world, package, actor)?;
    let mut cell = package_location_cell(world, package, actor)?;
    if cell.is_some_and(|cell| !cell_is_interior(world, cell)) {
        cell = None;
    }
    let actor_world = reference_world(world, actor);
    let mut actor_cell = parent_cell(world, actor);
    if actor_cell.is_some_and(|cell| cell_is_interior(world, cell)) {
        // 00566E77.
        if actor_cell != cell {
            return Ok(false);
        }
        if location.is_some_and(|location| location.type_00 == 1) {
            return Ok(true);
        }
    } else {
        actor_cell = None;
        if cell.is_some() || actor_world != space {
            return Ok(false);
        }
    }
    // 00566E65.
    let mut radius = location.map_or(0, |location| location.radius() as i32);
    if exact {
        radius = 0;
    } else if radius_override != -1.0 {
        radius = x87::ftol(radius_override);
    }
    let mut point = package_location_position(world, package, actor)?;
    let mut use_exact = false;
    let reference = match &location {
        Some(location) => location_reference(world, location)?,
        None => None,
    };
    let furniture = reference
        .and_then(|entity| world.get::<TesObjectRefr>(entity))
        .and_then(|refr| refr.base_form_1c)
        .is_some_and(|base| form_type(world, base) == 0x20);
    if radius == 0 {
        match (&location, reference) {
            (None, _) => radius = default_location_radius(world),
            // 00566F12, 00567152: type 3 alone takes 10 and exact; any other the setting, with the widening kept. The C++ makes them all 10 and exact.
            (Some(location), None) => {
                if location.type_00 == 3 {
                    use_exact = true;
                    radius = 10;
                } else {
                    radius = default_location_radius(world);
                }
            }
            (Some(_), Some(reference)) => {
                // 00566F1A vt+0x380, Character +0xD4, a raw word the host never sets.
                if world.get::<Character>(actor).is_some() && data.horse_or_rider_d4 != 0 {
                    return Err(GameError::unclassified(
                        0x00566F1A,
                        "00566F1A: a mounted actor",
                    ));
                }
                let base = world
                    .get::<TesObjectRefr>(reference)
                    .expect("a reference")
                    .base_form_1c;
                let base_type = base.map(|base| form_type(world, base));
                let statics = world.resource::<DefaultFormStatics>();
                if furniture
                    && let Some((process, middle)) = process_of(world, actor)
                        .and_then(|p| world.get::<MiddleHighProcess>(p).map(|m| (p, m)))
                {
                    // 00566F99 MiddleHighProcess_GetFurnitureMarkerPosition returns &process[0x128].
                    // 00566FA7 GetProcessLevel: only High takes this position.
                    if world.get::<HighProcess>(process).is_some() {
                        point = middle.furniture_marker_position_128.map(f32::from_bits);
                    }
                    radius = 20;
                    use_exact = true;
                } else if base.is_some()
                    && (base == statics.x_marker_heading_b35eb0 || base == statics.x_marker_b35eac)
                {
                    radius = 20;
                    use_exact = true;
                } else if world.get::<MobileObject>(reference).is_some()
                    && world.get::<Actor>(reference).is_some()
                {
                    // 0056702D vt+0x18C of 9 is 90, exact; no Low process has it.
                    if actor_sitting_state(world, reference) == 9 {
                        radius = 90;
                        use_exact = true;
                    } else {
                        let bound = x87::ftol(form_bound_radius(world, base));
                        radius = bound_plus(bound, 20.0);
                        if radius == 0 {
                            radius = default_location_radius(world);
                        }
                    }
                } else {
                    let base_type = base_type.ok_or(GameError::unclassified(
                        0x005670FD,
                        "005670FD: a base-less reference location faults",
                    ))?;
                    let extra = if matches!(base_type, 0x12 | 0x17 | 0x1C) {
                        0.0
                    } else {
                        20.0
                    };
                    let bound = x87::ftol(form_bound_radius(world, base));
                    radius = bound_plus(bound, extra);
                    if radius <= 0 {
                        radius = default_location_radius(world);
                    }
                }
            }
        }
    }
    // 0056707B the Sleep (5) height test in an interior. 005670CE: only above 100.0 fails, so a NaN passes.
    if package_data(world, package).type_20 as u8 == 5 && actor_cell.is_some() {
        let actor_z = reference_position(world, actor)?[2];
        let location_z = package_location_position(world, package, actor)?[2];
        if (actor_z - location_z).abs() > 100.0 {
            return Ok(false);
        }
    }
    // 00567172 vt+0x334(1) 005E6110, and vt+0x330 005E6190, which answers for type 0x0C only: 006163A0.
    if let Some(process) = process_of(world, actor)
        && let Some(current) = world
            .get::<BaseProcess>(process)
            .and_then(|base| base.editor_package_08)
        && package_data(world, current).type_20 == 12
    {
        return Err(GameError::unclassified(
            0x006163A0,
            "006163A0: a type 0x0C package at the location test",
        ));
    }
    if actor_within_radius(world, actor, point, radius as f32, !use_exact)? {
        return Ok(true);
    }
    // 005671D6..00567262. vt+0x380 returns the embedded marker position at both upper levels.
    if !furniture
        || !process_of(world, actor).is_some_and(|p| world.get::<MiddleHighProcess>(p).is_some())
    {
        return Ok(false);
    }
    let distance =
        crate::tes::forms_references::distance_to(reference_position(world, actor)?, point);
    let state = actor_sitting_state(world, actor);
    Ok((distance < 100.0 && state == 4) || state == 9)
}

// ---- package selection ----

// IDA: 00568EB0 the schedule hour test (start, duration, hour): __ftol2 of the hour.
pub(crate) fn schedule_hour_matches(start: i32, duration: i32, hour: f32) -> bool {
    let now = x87::ftol(hour);
    if now == start {
        return true;
    }
    let end = start.wrapping_add(duration) as f32;
    if end <= 23.0 {
        if now <= start {
            return false;
        }
    } else if now <= start && (now as f32) >= end - 24.0 {
        return false;
    }
    now < start.wrapping_add(duration)
}

// IDA: 00567C00 the day-of-week test, by the table at 00567C74 under `movsx` and `ja`.
pub(crate) fn schedule_day_matches(world: &World, day_field: u8, previous: bool) -> bool {
    let mut day = game_day_of_week(world);
    if previous {
        day -= 1;
        if day < 0 {
            day = 6;
        }
    }
    match day_field as i8 {
        0..=6 => day == i32::from(day_field),
        7 => day != 0 && day != 6,
        8 => day == 0 || day == 6,
        9 => day == 1 || day == 3 || day == 5,
        10 => day == 2 || day == 4,
        _ => false,
    }
}

/// IDA: N/A 004FBF90 (`base_extra_list::extra_list_script_event_or`) on a reference's own list, with the form's ID (0
/// for none); a reference with no TESObjectREFR answers 0 as a null list does.
pub fn script_event_or(
    world: &mut World,
    reference: Entity,
    event: u32,
    form: Option<Entity>,
) -> bool {
    let id = form.map_or(0, |form| {
        world
            .get::<crate::tes::form::FormIdentity>(form)
            .expect("a form")
            .form_id_0c
    });
    let Some(mut data) = world.get_mut::<TesObjectRefr>(reference) else {
        return false;
    };
    crate::tes::base_extra_list::extra_list_script_event_or(&mut data.extra_data_44, id, event)
}

// IDA: 00567B80 the schedule order: -1, 0 or 1.
pub(crate) fn schedule_order(mine: [u8; 4], theirs: [u8; 4]) -> i32 {
    let mut a = mine[0] as i8;
    let mut b = theirs[0] as i8;
    let mut same = (a == -1 && b == -1) || a == b;
    if same {
        a = mine[2] as i8;
        b = theirs[2] as i8;
        same = (a == 0 && b == -1) || a == b;
        if same {
            a = mine[1] as i8;
            b = theirs[1] as i8;
            same = (a == -1 && b == -1) || a == b;
        }
    }
    if same {
        let hour = mine[3] as i8;
        let other = theirs[3] as i8;
        if hour == other {
            return 0;
        }
        if hour == -1 {
            return 1;
        }
        if other == -1 {
            return -1;
        }
        return if hour < other { -1 } else { 1 };
    }
    if a == -1 {
        return 1;
    }
    if b == -1 {
        return -1;
    }
    if a < b { -1 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor_at(world: &mut World, position: [f32; 3]) -> Entity {
        world
            .spawn((
                TesObjectRefr {
                    position_2c: position,
                    ..TesObjectRefr::default()
                },
                Actor::default(),
            ))
            .id()
    }

    // 00684BD2 reads the actor's z, where the C++ reads delta z: 950 - 30 < 1000 and (950 + 128) + 30 > 1000 clear the 50 below it.
    #[test]
    fn the_height_clamp_compares_the_actor_z() {
        let mut world = World::new();
        let actor = actor_at(&mut world, [0.0, 0.0, 950.0]);
        assert!(actor_within_radius(&mut world, actor, [0.0, 0.0, 1000.0], 10.0, false).unwrap());
        // Above the clamp: az - 30 is past pz, so the delta stays.
        let actor = actor_at(&mut world, [0.0, 0.0, 1100.0]);
        assert!(!actor_within_radius(&mut world, actor, [0.0, 0.0, 1000.0], 10.0, false).unwrap());
        // Widening adds 32.
        let actor = actor_at(&mut world, [40.0, 0.0, 1000.0]);
        assert!(!actor_within_radius(&mut world, actor, [0.0, 0.0, 1000.0], 10.0, false).unwrap());
        assert!(actor_within_radius(&mut world, actor, [0.0, 0.0, 1000.0], 10.0, true).unwrap());
    }

    // The second High pass captured both a full-size capsule (26BE1) and an almost collapsed one (178C4).
    #[test]
    fn controller_state_and_capsule_radius_set_the_package_radius() {
        use crate::bs::character_controller::BHK_CAPSULE_SHAPE_VTABLE;
        use crate::bs::character_controller::BhkCharacterController;
        use crate::bs::character_controller::BhkShapeRecord;
        use crate::gamebryo::extra_data::BsBound;
        let mut world = World::new();
        let actor = actor_at(&mut world, [0.0; 3]);
        world.get_mut::<TesObjectRefr>(actor).unwrap().scale_38 = 1.0;
        let capsule = world
            .spawn(BhkShapeRecord {
                original_vtable: BHK_CAPSULE_SHAPE_VTABLE,
                havok_shape_08: true,
                havok_radius: f32::from_bits(0x40A5_082B),
                ..Default::default()
            })
            .id();
        let controller = world
            .spawn(BhkCharacterController {
                shapes_374: [Some(capsule), None],
                ..Default::default()
            })
            .id();
        let bound = world
            .spawn(BsBound {
                center_0c: [0, 0, 0x4280_0001],
                extents_18: [0, 0, 0x4280_0001],
            })
            .id();
        let process = world
            .spawn(MiddleHighProcess {
                char_controller_118: Some(controller),
                bound_188: Some(bound),
                ..Default::default()
            })
            .id();
        world.entity_mut(actor).insert(MobileObject {
            process_58: Some(process),
        });
        assert_eq!(
            actor_height(&mut world, actor).unwrap().to_bits(),
            0x4300_0001
        );
        assert!(actor_within_radius(&mut world, actor, [35.0, 0.0, 100.0], 0.0, true).unwrap());
        assert!(!actor_within_radius(&mut world, actor, [35.0, 0.0, 1000.0], 0.0, true).unwrap());
        world
            .get_mut::<BhkCharacterController>(controller)
            .unwrap()
            .context_state_1ec = 2;
        assert!(actor_within_radius(&mut world, actor, [35.0, 0.0, 1000.0], 0.0, true).unwrap());
        world
            .get_mut::<BhkCharacterController>(controller)
            .unwrap()
            .flags_1f4 |= 1;
        assert!(actor_within_radius(&mut world, actor, [70.0, 0.0, 1000.0], 0.0, true).unwrap());
        world
            .get_mut::<BhkShapeRecord>(capsule)
            .unwrap()
            .havok_radius = f32::from_bits(0x3ABB_4579);
        assert!(actor_within_radius(&mut world, actor, [32.0, 0.0, 1000.0], 0.0, true).unwrap());
        assert!(!actor_within_radius(&mut world, actor, [33.0, 0.0, 1000.0], 0.0, true).unwrap());
    }

    // 00566F12, 00567152: a radius 0 location with no reference takes the setting (default 0x64, widened by 32) unless it's type 3, which takes 10 with no widening. The C++ makes them all 10.
    #[test]
    fn a_location_with_no_reference_takes_the_setting() {
        let mut world = World::new();
        world.init_resource::<DefaultFormStatics>();
        world.insert_resource(crate::tes::tes::GameThreadLocals::default());
        world.insert_resource(crate::tes::tes::TesGlobals {
            save_load_game: Some(crate::tes::save_load_game::TesSaveLoadGame::default()),
            ..crate::tes::tes::TesGlobals::default()
        });
        let identity = FormIdentity {
            form_type_04: 0x32,
            flags_08: 0,
            form_id_0c: 1,
        };
        let empty = crate::tes::form::ModReferences(Vec::new());
        let actor = actor_at(&mut world, [0.0; 3]);
        world.entity_mut(actor).insert((identity, empty.clone()));
        let other = actor_at(&mut world, [120.0, 0.0, 0.0]);
        world.entity_mut(other).insert((identity, empty));
        let package = world
            .spawn(TesPackage {
                location_24: Some(PackageLocation {
                    type_00: 4,
                    radius_04: 0,
                    object_08: FormRef::Resolved(None),
                }),
                ..TesPackage::default()
            })
            .id();
        let process = world
            .spawn((
                BaseProcess {
                    edit_pack_procedure_type_04: 0,
                    editor_package_08: Some(package),
                },
                LowProcess {
                    field_30: Some(other),
                    ..LowProcess::default()
                },
            ))
            .id();
        world.entity_mut(actor).insert(MobileObject {
            process_58: Some(process),
        });
        // Type 4: the point is the process's +0x30, 120 away, inside 100 + 32.
        assert!(package_at_location(&mut world, package, actor, false, -1.0).unwrap());
        // Type 3 with a starting location: +0xE8, 120 away, outside 10.
        let mut data = world.get_mut::<Actor>(actor).unwrap();
        data.starting_location_f8 = Some(other);
        data.starting_position_e8 = [120.0_f32.to_bits(), 0, 0];
        world.get_mut::<TesPackage>(package).unwrap().location_24 = Some(PackageLocation {
            type_00: 3,
            radius_04: 0,
            object_08: FormRef::Resolved(None),
        });
        assert!(!package_at_location(&mut world, package, actor, false, -1.0).unwrap());
    }

    #[test]
    fn a_nan_length_compares_equal() {
        assert_eq!(compare_length([f32::NAN, 0.0, 0.0], 1.0), 0);
        assert_eq!(compare_length([0.5, 0.0, 0.0], 1.0), -1);
        assert_eq!(compare_length([2.0, 0.0, 0.0], 1.0), 1);
        assert_eq!(compare_length([1.0, 0.0, 0.0], 1.0), 0);
    }

    #[test]
    fn the_hour_test_follows_00568eb0() {
        assert!(schedule_hour_matches(8, 4, 8.5));
        assert!(schedule_hour_matches(8, 4, 11.0));
        assert!(!schedule_hour_matches(8, 4, 12.0));
        assert!(!schedule_hour_matches(8, 4, 7.0));
        // Across midnight.
        assert!(schedule_hour_matches(22, 4, 1.0));
        assert!(!schedule_hour_matches(22, 4, 2.0));
    }

    #[test]
    fn the_schedule_order_follows_00567b80() {
        assert_eq!(schedule_order([0xFF, 0xFF, 0, 8], [0xFF, 0xFF, 0, 8]), 0);
        assert_eq!(schedule_order([0xFF, 0xFF, 0, 8], [0xFF, 0xFF, 0, 9]), -1);
        assert_eq!(schedule_order([0xFF, 0xFF, 0, 0xFF], [0xFF, 0xFF, 0, 9]), 1);
        assert_eq!(schedule_order([1, 0xFF, 0, 8], [2, 0xFF, 0, 8]), -1);
    }
}
