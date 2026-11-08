//! Phase 14c: TESPackage for the AI pass: the time globals, 005672A0 the procedure type, the location getters 00566940, 00566A40, 00566B30, 00566DC0 at-location and 00684B30, and package selection 00569020 and 005692E0 with the schedule tests.
//!
//! x87: the main thread is at 24 bits here, so each `f32` step is an fadd, fsub or fmul in the exe's grouping; a `fild` is exact and rounds at the next operation. Evidence: objdump of each address named here; P14-RUN-004.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::actor::{Actor, Character, MobileObject, TesObjectRefr};
use super::actor_process::{BaseProcess, HighProcess, LowProcess, MiddleHighProcess};
use super::ai_conditions::{
    actor_sitting_state, evaluate_conditions, form_flags, process_of, require_low,
};
use super::ai_process::{AiArms, AiProcessGlobals};
use super::base_extra_list::{EXTRA_PACKAGE_START, EXTRA_RUN_ONCE_PACKS, ExtraData};
use super::character_controller::actor_character_controller;
use super::condition_tables::{PROCEDURE_BLOCK, PROCEDURE_ROWS};
use super::default_forms::{DefaultFormStatics, TimeGlobals};
use super::form::FormIdentity;
use super::form_components::{FormRef, TesAiForm, TesBipedModelForm, TesModel};
use super::forms_ai::{PackageLocation, PackageTarget, TesPackage};
use super::forms_init::form_type;
use super::forms_world::{TesObjectCell, cell_grid_x, cell_grid_y, cell_is_interior};
use super::game_settings::GameSettingCollection;
use super::global::TESGlobal;

/// IDA: 00A32048, FLT_MAX.
pub const FLT_MAX: f32 = f32::from_bits(0x7F7F_FFFF);

// ---- the time globals (00B332E0; 00402B40..00402C40) ----

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
pub fn time_globals_advance(world: &mut World, frame_seconds: f32) -> Result<(), StreamError> {
    let Some(scale) = time_global(world, 5) else {
        return Ok(());
    };
    let hour = time_global(world, 3).unwrap_or(12.0);
    let next = scale * frame_seconds / 3600.0 + hour;
    if next > 24.0 {
        return Err(StreamError("00402A13: a game day passing"));
    }
    let global = world.resource::<TimeGlobals>().globals_b332e0[3]
        .ok_or(StreamError("004029E0: a null game hour faults"))?;
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
pub(super) fn game_year(world: &World) -> i32 {
    x87::ftol64_low(time_global(world, 0).unwrap_or(427.0)) as i32
}

// IDA: 00402B80 the month: +0x04 through __ftol2, then `movsx eax,al` (00402B8A..00402B8F), or a literal 7. The C++ returns the whole int.
pub(super) fn game_month(world: &World) -> i32 {
    time_global(world, 1).map_or(7, |value| i32::from(x87::ftol(value) as i8))
}

// IDA: 00402BA0 the day: +0x08 or 17.0, __ftol2, the whole eax; callers narrow it.
pub(super) fn game_day(world: &World) -> i32 {
    x87::ftol(time_global(world, 2).unwrap_or(17.0))
}

/// IDA: 00402C40 the day of the week: +0x10 or 1.0 by `fistp qword` under chop, the low dword, unsigned % 7. The C++'s float to unsigned cast is undefined for a negative. Completion: 100%.
#[must_use]
pub fn game_day_of_week(world: &World) -> i32 {
    (x87::ftol64_low(time_global(world, 4).unwrap_or(1.0)) % 7) as i32
}

// IDA: 0047D2B0 the days in a month, from the table at 00B06710 (the C++'s comment says 00B06A00): `cmp eax,0Ch; jl`, else `xor ax,ax`. A negative index reads before the table, which only a broken +0x18 gives.
fn days_in_month(month: i32) -> Result<i32, StreamError> {
    const DAYS: [i32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if month >= 12 {
        return Ok(0);
    }
    usize::try_from(month)
        .map(|month| DAYS[month])
        .map_err(|_| StreamError("0047D2B8: a negative month reads before the table"))
}

/// IDA: 00643640 LowProcess vt+0x20: +0x14 the game hour; +0x18 `((year << 4 | month) << 9) | movsx(day)`. Completion: 100%.
#[must_use]
pub fn process_stamps(world: &World) -> (f32, i32) {
    let year = game_year(world) << 4;
    let packed = (game_month(world) | year) << 9;
    (game_hour(world), i32::from(game_day(world) as i8) | packed)
}

/// IDA: 00402BA0 for a caller that narrows it, with `movsx`.
#[must_use]
pub fn game_day_byte(world: &World) -> u8 {
    game_day(world) as u8
}

// ---- package data ----

/// IDA: N/A a package's component.
#[must_use]
pub fn package_data(world: &World, package: Entity) -> &TesPackage {
    world.get::<TesPackage>(package).expect("a package")
}

// IDA: N/A a helper, with no address of its own.
fn resolved(slot: FormRef, where_: &'static str) -> Result<Option<Entity>, StreamError> {
    match slot {
        FormRef::Resolved(form) => Ok(form),
        FormRef::Raw(0) => Ok(None),
        FormRef::Raw(_) => Err(StreamError(where_)),
    }
}

/// IDA: 00569820-style: a location's +0x08 once InitItem has run.
pub fn location_object(location: &PackageLocation) -> Result<Option<Entity>, StreamError> {
    resolved(
        location.object_08,
        "00569820: an unresolved location object",
    )
}

/// IDA: 005697E0 the +0x08 of a type 0 location, else 0.
pub fn location_reference(
    world: &World,
    location: &PackageLocation,
) -> Result<Option<Entity>, StreamError> {
    if location.type_00 != 0 {
        return Ok(None);
    }
    Ok(location_object(location)?.filter(|&form| world.get::<TesObjectRefr>(form).is_some()))
}

// IDA: 00569800 PackageLocation_GetCell.
fn location_cell(location: &PackageLocation) -> Result<Option<Entity>, StreamError> {
    if location.type_00 != 1 {
        return Ok(None);
    }
    location_object(location)
}

// IDA: N/A no address of its own; its comments cite 00569E60.
fn target_value(target: &PackageTarget) -> Result<Option<Entity>, StreamError> {
    resolved(target.value_04, "00569E60: an unresolved package target")
}

/// IDA: 00569E60 the +0x04 of a type 0 target.
pub fn target_reference(
    world: &World,
    target: &PackageTarget,
) -> Result<Option<Entity>, StreamError> {
    if target.type_00 != 0 {
        return Ok(None);
    }
    Ok(target_value(target)?.filter(|&form| world.get::<TesObjectRefr>(form).is_some()))
}

/// IDA: 00569E70 the +0x04 of a type 1 target.
pub fn target_object(target: &PackageTarget) -> Result<Option<Entity>, StreamError> {
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
pub fn procedure_at(world: &World, package: Entity, index: u32) -> Result<u32, StreamError> {
    let row = package_data(world, package).field_18 as usize;
    let start = *PROCEDURE_ROWS
        .get(row)
        .ok_or(StreamError("00649459: +0x18 outside the 00B152B0 table"))?;
    PROCEDURE_BLOCK
        .get(usize::from(start) + index as usize)
        .copied()
        .ok_or(StreamError("00649460: a procedure index past the block"))
}

/// IDA: 005672A0 TESPackage_InitProcedureType: +0x18 from the type, target and location. Its tables 0056764C, 005676D0, 005676EC and 0056770C read from the exe. Type 0 with a target whose three getters are all null leaves +0x18 alone (00567329, where the C++ writes -1). -1 warns and returns (00567606, where the C++ stops), and the package stays selectable. Completion: 100%.
pub fn package_init_procedure(world: &mut World, package: Entity) -> Result<(), StreamError> {
    let data = package_data(world, package);
    let target = data.target_28;
    let location = data.location_24;
    let kind = data.type_20 as u8;
    let any_target = |world: &World, target: &PackageTarget| -> Result<bool, StreamError> {
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
                                .ok_or(StreamError("005672FF: a target reference with no base"))?;
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
                            .ok_or(StreamError("005673F5: a target reference with no base"))?;
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

/// IDA: vt+0x174 Actor 005F10E0: a process with vt+0x2C0 bit 0x800 (High 006285A0: word +0x1FC; the others 0) takes the ring buffer, a host stop; otherwise +0x2C. REFR 004C8D00 +0x2C. Completion: 100%, past the ring buffer. The C++ stops on every non-Low process, so on the player.
pub fn reference_position(world: &World, reference: Entity) -> Result<[f32; 3], StreamError> {
    if let Some(process) = process_of(world, reference)
        && world
            .get::<HighProcess>(process)
            .is_some_and(|high| high.movement_flags_1fc & 0x800 != 0)
    {
        return Err(StreamError("005F10F1: the ring buffer of a moving process"));
    }
    Ok(world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .position_2c)
}

/// IDA: 006ECC80 +0x40.
#[must_use]
pub fn parent_cell(world: &World, reference: Entity) -> Option<Entity> {
    world.get::<TesObjectRefr>(reference)?.parent_cell_40
}

/// IDA: 004C9CF0 an exterior cell's +0x50.
#[must_use]
pub fn cell_world(world: &World, cell: Option<Entity>) -> Option<Entity> {
    let data = world.get::<TesObjectCell>(cell?)?;
    if data.flags_24 & 1 != 0 {
        return None;
    }
    data.world_50
}

/// IDA: 004D6670 TESObjectREFR_GetExteriorWorldspace: +0x40, else the TESChildCell's vt+0 004DDCB0, which is +0x40 or, for a persistent or 0x4000 reference, the 0x11 extra's. Completion: 100%.
#[must_use]
pub fn reference_world(world: &World, reference: Entity) -> Option<Entity> {
    if let Some(cell) = parent_cell(world, reference) {
        return cell_world(world, Some(cell));
    }
    let persistent = super::forms_references::reference_is_persistent(world, reference)
        .expect("004DB4A0: a null g_saveLoadGame or TESFile faults")
        || form_flags(world, reference) & 0x4000 != 0;
    if !persistent {
        return None;
    }
    let cell = world
        .get::<TesObjectRefr>(reference)
        .expect("a reference")
        .extra_data_44
        .persistent_cell();
    cell_world(world, cell)
}

/// IDA: 005E02E0(useTemplate): vt+0x128 00447550, the +0xD0 template, then vt+0x170 cast to an actor base. +0xD0 is a raw word the host never sets, so a nonzero one stops. Completion: 100%, past that.
pub fn actor_base_form(
    world: &World,
    actor: Entity,
    use_template: bool,
) -> Result<Option<Entity>, StreamError> {
    let data = world.get::<Actor>(actor).expect("an actor");
    if use_template && data.leveled_creature_template_d0 != 0 {
        return Err(StreamError("00447550: a leveled template (+0xD0)"));
    }
    Ok(world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .base_form_1c)
}

/// IDA: 005E33B0 Actor_IsDead(excludeState6). Completion: 100%.
#[must_use]
pub fn actor_is_dead(world: &World, actor: Entity, exclude_state_6: bool) -> bool {
    let state = world.get::<Actor>(actor).expect("an actor").dead_state_b0;
    state == 1 || state == 2 || (!exclude_state_6 && state == 6)
}

// ---- the location getters ----

// IDA: 0041E9B0's entry.
fn package_start_entry(
    world: &World,
    reference: Entity,
) -> Option<&super::base_extra_list::BaseExtraEntry> {
    world
        .get::<TesObjectRefr>(reference)?
        .extra_data_44
        .get(EXTRA_PACKAGE_START)
}

// IDA: N/A no address of its own; its comments cite 0041E9B0.
fn start_location_form(world: &World, reference: Entity) -> Result<Option<Entity>, StreamError> {
    let Some(entry) = package_start_entry(world, reference) else {
        return Ok(None);
    };
    match (entry.resolved_0c, &entry.data) {
        (Some(form), _) => Ok(form),
        (None, ExtraData::PackageStart { target_0c: 0, .. }) => Ok(None),
        _ => Err(StreamError(
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
) -> Result<Option<Entity>, StreamError> {
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
) -> Result<Option<Entity>, StreamError> {
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

/// IDA: 004CBA50 a cell's first reference: the +0x48 head item.
#[must_use]
pub fn cell_first_reference(world: &World, cell: Entity) -> Option<Entity> {
    world
        .get::<TesObjectCell>(cell)?
        .objects_48
        .items()
        .next()
        .filter(|&form| world.get::<TesObjectRefr>(form).is_some())
}

/// IDA: 00566B30 TESPackage_GetLocationPosition(out, actor). Every path is a copy of floats, or the grid `<<12`, `fild`, a float store. g_niPoint3Zero 00B3F9A8 for a miss. Completion: 100%.
pub fn package_location_position(
    world: &World,
    package: Entity,
    actor: Entity,
) -> Result<[f32; 3], StreamError> {
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
                .and_then(|base| world.get::<super::forms_actors::TesCreature>(base))
                .is_some_and(|creature| creature.data_104[0] == 4)
                && data.loaded_3d_3c != 0
            {
                return Err(StreamError("00625290: the creature EntryPoint node"));
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

// IDA: 005E0660 the actor's height: `(max z - min z) * scale`, both zero with no +0x3C (0065ADD0, 0065AE80 through 004DC360, 004DC410).
fn actor_height(world: &World, actor: Entity) -> Result<f32, StreamError> {
    if world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .loaded_3d_3c
        != 0
    {
        return Err(StreamError("004DC360: the bound of a loaded actor"));
    }
    Ok(0.0)
}

/// IDA: 00684B30 Actor_IsWithinRadius(position, radius, widen): the delta to the point; a controller-less actor with height h (0 is 128: 00684BBA) clears delta z when `az - 30 < pz` and `(az + h) + 30 > pz`, comparing the actor's z, not delta z as the C++ does (00684BD2 `fld [esp+1Ch]`); with `widen` the radius grows by 32. Completion: 100% of the controller-less arm.
/// TODO: [ ] 00684B96, 00684C3E the controller arms, with a controller on a non-Low process.
pub fn actor_within_radius(
    world: &World,
    actor: Entity,
    point: [f32; 3],
    radius: f32,
    widen: bool,
) -> Result<bool, StreamError> {
    let position = reference_position(world, actor)?;
    let mut delta = [
        position[0] - point[0],
        position[1] - point[1],
        position[2] - point[2],
    ];
    if actor_character_controller(world, actor).is_some() {
        return Err(StreamError("00684B96: the character-controller arm"));
    }
    let mut height = actor_height(world, actor)?;
    if height == 0.0 {
        height = 128.0;
    }
    let below = position[2] - 30.0;
    if below < point[2] {
        let above = 30.0 + (position[2] + height);
        if above > point[2] {
            delta[2] = 0.0;
        }
    }
    let limit = if widen { 32.0 + radius } else { radius };
    Ok(compare_length(delta, limit) <= 0)
}

// IDA: 0046D5C0 the bound radius of a form: a TESModel's +0x0C; otherwise 004694A0(form, the form as a reference): no TESBipedModelForm gives 0.0 (fldz), and with a null reference, the male world model's: a float at biped +0x44. Null is 0.0. The C++ stops past TESModel. Completion: 100% for a base form.
fn form_bound_radius(world: &World, form: Option<Entity>) -> f32 {
    let Some(form) = form else {
        return 0.0;
    };
    if let Some(model) = world.get::<TesModel>(form) {
        return model.bound_radius_0c;
    }
    world
        .get::<TesBipedModelForm>(form)
        .map_or(0.0, |biped| biped.world_models_38[0].bound_radius_0c)
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

/// IDA: 00566DC0 TESPackage_IsAtLocation(actor, exact, radius). Completion: 100% of what a LowProcess actor reaches, each arm read; the furniture and mount arms of other processes stop.
/// TODO: [ ] 00566F26 mounted on the location reference, 00566F8A a furniture process arm, 005671D6 the furniture fallback: High and MiddleHigh actors, past this window.
pub fn package_at_location(
    world: &World,
    package: Entity,
    actor: Entity,
    exact: bool,
    radius_override: f32,
) -> Result<bool, StreamError> {
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
    let point = package_location_position(world, package, actor)?;
    let mut use_exact = false;
    if radius == 0 {
        let reference = match &location {
            Some(location) => location_reference(world, location)?,
            None => None,
        };
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
                    return Err(StreamError("00566F1A: a mounted actor"));
                }
                let base = world
                    .get::<TesObjectRefr>(reference)
                    .expect("a reference")
                    .base_form_1c;
                let base_type = base.map(|base| form_type(world, base));
                if base_type == Some(0x20)
                    && let Some(process) = process_of(world, actor)
                {
                    require_low(world, process, "00566F8A: a process's furniture arm")?;
                }
                let statics = world.resource::<DefaultFormStatics>();
                if base.is_some()
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
                    let base_type = base_type.ok_or(StreamError(
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
        return Err(StreamError(
            "006163A0: a type 0x0C package at the location test",
        ));
    }
    let within = actor_within_radius(world, actor, point, radius as f32, !use_exact);
    // 005671D6: on a miss, a furniture location with a nonzero process vt+0x380 (`0064B0A0`, MiddleHigh and High) gets a second chance by distance and state. Not ported, so it stops rather than answering false.
    if within == Ok(false)
        && process_of(world, actor)
            .is_some_and(|process| world.get::<MiddleHighProcess>(process).is_some())
    {
        return Err(StreamError(
            "005671D6: the furniture fallback on a MiddleHigh or High process",
        ));
    }
    within
}

// ---- package selection ----

// IDA: 00568EB0 the schedule hour test (start, duration, hour): __ftol2 of the hour.
fn schedule_hour_matches(start: i32, duration: i32, hour: f32) -> bool {
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
fn schedule_day_matches(world: &World, day_field: u8, previous: bool) -> bool {
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

// IDA: 004200C0: the 0x21 extra's list has the package: a null item ends the walk.
fn run_once_contains(world: &World, actor: Entity, package: Entity) -> bool {
    let list = &world
        .get::<TesObjectRefr>(actor)
        .expect("an actor")
        .extra_data_44;
    match list.get(EXTRA_RUN_ONCE_PACKS).map(|entry| &entry.data) {
        Some(ExtraData::RunOncePacks(items)) => items.iter().any(|item| item.0 == package),
        _ => false,
    }
}

/// IDA: 00468670 the actor's package list: 005E02E0(1), or 005E02E0(0) when that's empty; null with no base. As a copy of the slots. Completion: 100%.
pub fn actor_package_list(
    world: &World,
    actor: Entity,
) -> Result<Option<Vec<FormRef>>, StreamError> {
    let Some(base) = actor_base_form(world, actor, true)? else {
        return Ok(None);
    };
    let mut list = world.get::<TesAiForm>(base).map(|ai| &ai.packages_10);
    if list.is_some_and(Vec::is_empty) {
        let Some(plain) = actor_base_form(world, actor, false)? else {
            return Ok(None);
        };
        list = world.get::<TesAiForm>(plain).map(|ai| &ai.packages_10);
    }
    Ok(list.cloned())
}

// IDA: N/A no address of its own; its comments cite 00569074.
fn package_of(world: &World, slot: FormRef) -> Result<Option<Entity>, StreamError> {
    let form = resolved(slot, "00569074: an unresolved AI package")?;
    match form {
        Some(form) if world.get::<TesPackage>(form).is_none() => Err(StreamError(
            "00569074: an AI package entry that is no TESPackage",
        )),
        form => Ok(form),
    }
}

/// IDA: 004FBF90(form, list, event) -> 004FB580: an event list at the 0x12 extra's +0x10 with entries at +0x08 takes the bits in two walks. `ScriptEventList` has no +0x08 until a script runs, which is the counted boundary, so here it's always empty and answers 0. Completion: 100% of the empty list.
/// TODO: [ ] 004FB580's two walks: the form's entry or the last empty one, then the first empty one too, when the interpreter fills +0x08 (Phase 21).
pub const fn script_event_or(_world: &World, _actor: Entity, _event: u32, _form: Option<Entity>) {}

// IDA: 00424D00 on a list: 00 extra 0x23's list loses the follower (0065C620), then 0045A500's query, whose answer isn't used.
fn follower_remove(world: &mut World, target: Entity, follower: Entity) {
    let mut data = world.get_mut::<TesObjectRefr>(target).expect("a reference");
    if let Some(entry) = data
        .extra_data_44
        .get_mut(super::base_extra_list::EXTRA_FOLLOWER)
        && let ExtraData::Followers(items) = &mut entry.data
        && let Some(at) = items.iter().position(|&item| item == follower)
    {
        items.remove(at);
    }
}

/// IDA: 00569020 TESAIForm_SelectPackage(actor) on the list: the current package, then the first whose flags, schedule and 0056A950 conditions pass. Answers the current one when none does. Completion: 100%.
pub fn select_from_list(
    world: &mut World,
    list: &[FormRef],
    actor: Entity,
    arms: &mut dyn AiArms,
) -> Result<Option<Entity>, StreamError> {
    // 005E0380: process vt+0x184 00452A60: +0x08, 0 without one.
    let process = process_of(world, actor);
    if let Some(process) = process {
        super::ai_conditions::stop_on_process_package_override(world, process)?;
    }
    let current = process
        .and_then(|process| world.get::<BaseProcess>(process))
        .and_then(|base| base.editor_package_08);
    for &slot in list {
        let Some(package) = package_of(world, slot)? else {
            return Ok(current);
        };
        let data = package_data(world, package);
        let flags = data.flags_1c;
        let schedule = data.schedule_2c;
        if (flags & 0x8000 != 0 || flags & 0x400 != 0)
            && (flags & 0x400 == 0 || run_once_contains(world, actor, package))
        {
            continue;
        }
        // 005690B4 00567280: with flag 4 and the current package, process vt+0xC0 (Low 0076C780: +0x1D) skips it. The C++ stops.
        if flags & 4 != 0 && current == Some(package) {
            let byte_1d = process
                .and_then(|process| world.get::<LowProcess>(process))
                .is_some_and(|low| low.byte_1d != 0);
            if byte_1d {
                continue;
            }
        }
        let month = schedule.data_00[0] as i8;
        let day_field = schedule.data_00[1];
        let date = schedule.data_00[2] as i8;
        let hour = schedule.data_00[3] as i8;
        let duration = schedule.duration_04 as i32;
        let mut yesterday_day = false;
        let mut yesterday_date = false;
        let mut yesterday_month = false;
        if i32::from(hour).wrapping_add(duration) > 24 {
            yesterday_day = schedule_day_matches(world, day_field, true);
            let mut day = game_day(world) - 1;
            if day < 0 {
                day = 30;
            }
            if i32::from(date) == day {
                yesterday_date = true;
            }
            let mut month_now = game_month(world) - 1;
            if month_now < 0 {
                month_now = 12;
            }
            if i32::from(month) == month_now {
                yesterday_month = true;
            }
        }
        if month != -1 && i32::from(month) != game_month(world) && !yesterday_month {
            continue;
        }
        if date != 0 && date != game_day(world) as i8 && !yesterday_date {
            continue;
        }
        if day_field as i8 != -1 && !schedule_day_matches(world, day_field, false) && !yesterday_day
        {
            continue;
        }
        if hour != -1 && !schedule_hour_matches(i32::from(hour), duration, game_hour(world)) {
            continue;
        }
        let target = match &package_data(world, package).target_28 {
            Some(target) => target_reference(world, target)?,
            None => None,
        };
        let conditions = package_data(world, package).conditions_34.clone();
        if !evaluate_conditions(world, &conditions, Some(actor), target, arms)? {
            continue;
        }
        // 00569217 the debug-selection log (00579440 is null at startup).
        if package_data(world, package).field_18 == 0xFFFF_FFFF {
            package_init_procedure(world, package)?;
        }
        if let Some(current) = current {
            if current == package {
                return Ok(Some(package));
            }
            // 00569276..005692A8 an old type 1: process vt+0xCC (00780E50: +0x2C) cast to an Actor loses the follower. The C++ stops on anything there.
            if package_data(world, current).type_20 == 1
                && let Some(followed) = process
                    .and_then(|process| world.get::<LowProcess>(process))
                    .and_then(|low| low.field_2c)
                && world.get::<Actor>(followed).is_some()
            {
                follower_remove(world, followed, actor);
            }
            // 005692B7 004FBF90(old, +0x44, 0x800).
            script_event_or(world, actor, 0x800, Some(current));
        }
        // 005692CD 004FBF90(new, +0x44, 0x200).
        script_event_or(world, actor, 0x200, Some(package));
        return Ok(Some(package));
    }
    Ok(current)
}

// IDA: 00567B80 the schedule order: -1, 0 or 1.
fn schedule_order(mine: [u8; 4], theirs: [u8; 4]) -> i32 {
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

/// IDA: 005692E0 TESAIForm_CollectScheduledPackages(actor, list, stamp, hour), through 005E0340 and 004686C0: from the process's last stamp (vt+0x34 day, vt+0x38 month) hour by hour to now, each package that's started and isn't current joins once (005672A0 on a -1), appended (0067B1E0); then the 00567B80 bubble sort. The stamp argument and vt+0x3C the year are read and not used. Completion: 100%.
#[expect(
    clippy::neg_cmp_op_on_partial_ord,
    reason = "005692E0's `fcompp; test ah,41h`: a NaN hour skips"
)]
pub fn collect_scheduled_packages(
    world: &mut World,
    list: &[FormRef],
    process: Entity,
    hour: f32,
) -> Result<Vec<Entity>, StreamError> {
    let stamp = world
        .get::<LowProcess>(process)
        .expect("a LowProcess")
        .cur_packed_date_18;
    let mut out: Vec<Entity> = Vec::new();
    let mut day = stamp & 0x1FF;
    let mut month = (stamp >> 9) & 0xF;
    let today = i32::from(game_day(world) as i8);
    let this_month = game_month(world);
    let mut stamp_hour = hour;
    super::ai_conditions::stop_on_process_package_override(world, process)?;
    if day != 0 {
        loop {
            let current = world
                .get::<BaseProcess>(process)
                .expect("a process")
                .editor_package_08;
            if list.is_empty() {
                break;
            }
            for &slot in list {
                let Some(package) = package_of(world, slot)? else {
                    break;
                };
                let schedule = package_data(world, package).schedule_2c.data_00;
                let m = schedule[0] as i8;
                if m != -1 && i32::from(m) > month {
                    continue;
                }
                let d = schedule[2] as i8;
                if d != 0 && i32::from(d) > day {
                    continue;
                }
                let w = schedule[1] as i8;
                if w != -1 && i32::from(w) > game_day_of_week(world) {
                    continue;
                }
                let h = schedule[3] as i8;
                if h != -1 && !(stamp_hour > f32::from(h)) {
                    continue;
                }
                if Some(package) == current || out.contains(&package) {
                    continue;
                }
                if package_data(world, package).field_18 == 0xFFFF_FFFF {
                    package_init_procedure(world, package)?;
                }
                out.push(package);
            }
            stamp_hour += 1.0;
            if 23.0 < stamp_hour {
                stamp_hour = 0.0;
                day += 1;
                let length = days_in_month(month)?;
                if day > length {
                    day -= length;
                    month += 1;
                    if month >= 12 {
                        month -= 12;
                    }
                }
            }
            if day == today
                && month == this_month
                && x87::ftol(stamp_hour) >= x87::ftol(game_hour(world))
            {
                break;
            }
            if day == 0 {
                break;
            }
        }
    }
    // 005694B0 the bubble sort by 00567B80. The inner walk starts at the outer item: a self compare is 0.
    let mut count = out.len();
    let mut swapped = true;
    let mut outer = 0;
    while outer < out.len() && count > 0 && swapped {
        swapped = false;
        for inner in outer..out.len() {
            let mine = package_data(world, out[outer]).schedule_2c.data_00;
            let theirs = package_data(world, out[inner]).schedule_2c.data_00;
            if schedule_order(mine, theirs) == 1 {
                out.swap(outer, inner);
                swapped = true;
            }
        }
        outer += 1;
        count -= 1;
    }
    Ok(out)
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
        assert!(actor_within_radius(&world, actor, [0.0, 0.0, 1000.0], 10.0, false).unwrap());
        // Above the clamp: az - 30 is past pz, so the delta stays.
        let actor = actor_at(&mut world, [0.0, 0.0, 1100.0]);
        assert!(!actor_within_radius(&world, actor, [0.0, 0.0, 1000.0], 10.0, false).unwrap());
        // Widening adds 32.
        let actor = actor_at(&mut world, [40.0, 0.0, 1000.0]);
        assert!(!actor_within_radius(&world, actor, [0.0, 0.0, 1000.0], 10.0, false).unwrap());
        assert!(actor_within_radius(&world, actor, [0.0, 0.0, 1000.0], 10.0, true).unwrap());
    }

    // 00566F12, 00567152: a radius 0 location with no reference takes the setting (default 0x64, widened by 32) unless it's type 3, which takes 10 with no widening. The C++ makes them all 10.
    #[test]
    fn a_location_with_no_reference_takes_the_setting() {
        let mut world = World::new();
        world.init_resource::<DefaultFormStatics>();
        world.insert_resource(super::super::tes::GameThreadLocals::default());
        world.insert_resource(super::super::tes::TesGlobals {
            save_load_game: Some(super::super::tes::TesSaveLoadGame::default()),
            ..super::super::tes::TesGlobals::default()
        });
        let identity = FormIdentity {
            form_type_04: 0x32,
            flags_08: 0,
            form_id_0c: 1,
        };
        let empty = super::super::form::ModReferences(Vec::new());
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
        assert!(package_at_location(&world, package, actor, false, -1.0).unwrap());
        // Type 3 with a starting location: +0xE8, 120 away, outside 10.
        let mut data = world.get_mut::<Actor>(actor).unwrap();
        data.starting_location_f8 = Some(other);
        data.starting_position_e8 = [120.0_f32.to_bits(), 0, 0];
        world.get_mut::<TesPackage>(package).unwrap().location_24 = Some(PackageLocation {
            type_00: 3,
            radius_04: 0,
            object_08: FormRef::Resolved(None),
        });
        assert!(!package_at_location(&world, package, actor, false, -1.0).unwrap());
    }

    #[test]
    fn a_nan_length_compares_equal() {
        assert_eq!(compare_length([f32::NAN, 0.0, 0.0], 1.0), 0);
        assert_eq!(compare_length([0.5, 0.0, 0.0], 1.0), -1);
        assert_eq!(compare_length([2.0, 0.0, 0.0], 1.0), 1);
        assert_eq!(compare_length([1.0, 0.0, 0.0], 1.0), 0);
    }

    #[test]
    fn the_month_table_ends_at_twelve() {
        assert_eq!(days_in_month(0).unwrap(), 31);
        assert_eq!(days_in_month(1).unwrap(), 28);
        assert_eq!(days_in_month(12).unwrap(), 0);
        assert!(days_in_month(-1).is_err());
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
