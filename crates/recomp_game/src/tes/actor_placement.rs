//! The placement 005FA320 Actor_RebuildHighProcess asks for before it builds the HighProcess: 005EB400, which with no package location to keep puts an actor on its cell's path grid (005E2E20 through 0067D820 and 004E5730), and 005EB370, which tests the position (005EB150).
//! Evidence: `evidence/gameplay-run002-2026-11-07/` run-ad, every call and return inside these functions for the Imperial Prison's 11 actors; run-ac for which actor took which. The x87 runs at 24 bits (control word 0x7F), so each `fld`/`fstp` step is an f32 operation.

use bevy_ecs::prelude::*;
use recomp_core::reader::StreamError;
use recomp_core::x87;

use super::actor::{Actor, MobileObject, TesObjectRefr};
use super::actor_process::{HighProcess, MiddleHighProcess};
use super::ai_package::{actor_is_dead, package_at_location, package_data, parent_cell};
use super::form_components::FormRef;
use super::forms_init::form_type;
use super::forms_world::{
    PathGridPoint, TesObjectCell, TesPathGrid, cell_is_interior, cell_subspace_at,
};
use super::game_settings::GameSettingCollection;

/// IDA: the path cost settings 005EC1F0 and 0067EDE0 read: fPathPreferredPointBonus, fPathInvalidMovementTypePenalty, fPathNPCWadingPenalty, fPathWaterExitPenalty and fPathSpaceExitPenalty.
const PATH_PREFERRED_POINT_BONUS: u32 = 0x00B3_A400;
const PATH_INVALID_MOVEMENT_TYPE_PENALTY: u32 = 0x00B3_A408;
const PATH_WATER_EXIT_PENALTY: u32 = 0x00B3_A420;
const PATH_SPACE_EXIT_PENALTY: u32 = 0x00B3_A428;

// IDA: N/A a static GameSetting by address.
fn setting(world: &World, address: u32) -> Result<f32, StreamError> {
    world
        .get_resource::<GameSettingCollection>()
        .and_then(|settings| settings.raw_at(address))
        .map(f32::from_bits)
        .ok_or(StreamError(
            "005EC1F0: a path setting with no static GameSetting",
        ))
}

/// IDA: N/A how many 0067D820 runs built and dropped a 004E7DF0 PathingLocation: its constructor's 00B36080 count and two debug spheres (0047FD30) on the first, which the host doesn't draw.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct PathingLocationCounts {
    pub built: u32,
    /// IDA: N/A each 005E2E20 that found a point with a connection: the actor, the point and the connection, as indexes of the grid's array.
    pub placements: Vec<(Entity, usize, usize)>,
}

// IDA: N/A the actor's process, +0x58.
fn process(world: &World, actor: Entity) -> Option<Entity> {
    world.get::<MobileObject>(actor).and_then(|m| m.process_58)
}

/// IDA: 005E3350: none of vt+0x1A0 005E17E0 (Paralysis, actor value 0x30, nonzero), vt+0x198 Actor_IsDead(0), vt+0x19C 005E04F0 (a process's vt+0x2E4: Low 0, MiddleHigh +0x11C), +0xB0 of 5 or 3, vt+0x18C 005E0EC0 (a process's vt+0x36C: Low 0, MiddleHigh the sleep state). Completion: 100% of a Low process or none; a MiddleHigh one stops.
/// # Errors
/// The actor value's, or a MiddleHigh process.
pub fn actor_can_be_placed(world: &mut World, actor: Entity) -> Result<bool, StreamError> {
    if super::ai_process::actor_get_actor_value(world, actor, 0x30)? != 0 {
        return Ok(false);
    }
    if actor_is_dead(world, actor, false) {
        return Ok(false);
    }
    if let Some(process) = process(world, actor)
        && world.get::<MiddleHighProcess>(process).is_some()
    {
        return Err(StreamError("005E337D: 005E04F0 on a MiddleHigh process"));
    }
    let state = world.get::<Actor>(actor).expect("an actor").dead_state_b0;
    Ok(state != 5 && state != 3)
}

/// IDA: 005E1E90: the base through vt+0x170 when vt+0x190 (an actor) answers true, then 00519CC0, 00519C70 and 00519CE0, which only a creature (type 0x24) can fail: an NPC answers false at the first. Completion: 100% of an NPC; a creature stops.
/// # Errors
/// A creature.
pub fn actor_placement_exempt(world: &World, actor: Entity) -> Result<bool, StreamError> {
    let base = world
        .get::<TesObjectRefr>(actor)
        .expect("a reference")
        .base_form_1c;
    if base.is_some_and(|base| form_type(world, base) == 0x24) {
        return Err(StreamError("005E1EBB: 00519CC0 on a creature base"));
    }
    Ok(false)
}

/// IDA: 005EB400(actor): 005E3350 false or 005E1E90 true answers false. With a process's package (+0x08) at its location (00566DC0(actor, 0, -1.0)): type 6 whose 00566D00 reference's base is XMarkerHeading answers false; a location of type 3 whose 00566DB0 radius is 0 answers false. Otherwise the position (vt+0x174), and with a cell 005E2E20(position, cell, 0, 0, 0); 004D8A30 stores it. Answers true. Completion: 100% of the arms run-ad shows.
/// # Errors
/// A stop below.
pub fn actor_place_on_path_grid(world: &mut World, actor: Entity) -> Result<bool, StreamError> {
    if !actor_can_be_placed(world, actor)? || actor_placement_exempt(world, actor)? {
        return Ok(false);
    }
    if let Some(process) = process(world, actor)
        && let Some(package) = world
            .get::<super::actor_process::BaseProcess>(process)
            .and_then(|base| base.editor_package_08)
        && package_at_location(world, package, actor, false, -1.0)?
    {
        let data = package_data(world, package);
        let location = data.location_24;
        if data.type_20 == 6
            && let Some(reference) = package_location_reference(location, actor)?
        {
            let base = world
                .get::<TesObjectRefr>(reference)
                .expect("a reference")
                .base_form_1c;
            if base.is_some()
                && base
                    == world
                        .resource::<super::default_forms::DefaultFormStatics>()
                        .x_marker_heading_b35eb0
            {
                return Ok(false);
            }
        }
        if location.is_some_and(|location| location.type_00 == 3 && location_radius(location) == 0)
        {
            return Ok(false);
        }
    }
    let mut position = super::ai_package::reference_position(world, actor)?;
    if let Some(cell) = parent_cell(world, actor) {
        position = place_on_path_grid(world, actor, position, cell)?;
    }
    super::forms_references::set_reference_position(world, actor, position)?;
    Ok(true)
}

/// IDA: 00566D00(actor) on a package's location: null or type 2 answers `actor`; 0 its reference; 3 `actor`; 4 and 5 read the actor's process, a stop here; others null. Completion: 100% of types 0..3.
fn package_location_reference(
    location: Option<super::forms_ai::PackageLocation>,
    actor: Entity,
) -> Result<Option<Entity>, StreamError> {
    let Some(location) = location else {
        return Ok(Some(actor));
    };
    match location.type_00 {
        0 => Ok(match location.object_08 {
            FormRef::Resolved(reference) => reference,
            FormRef::Raw(_) => None,
        }),
        2 | 3 => Ok(Some(actor)),
        4 | 5 => Err(StreamError("00566D59: a location in the actor's process")),
        _ => Ok(None),
    }
}

/// IDA: 005697A0 PackageLocation_GetRadius: types 0xFF and 1 answer 0, the rest +0x04. Completion: 100%.
fn location_radius(location: super::forms_ai::PackageLocation) -> u32 {
    if location.type_00 == -1 || location.type_00 == 1 {
        0
    } else {
        location.radius_04
    }
}

/// IDA: 005EB370(actor): 005E3350 true, then 005EB150 on the position (vt+0x174) with `test_controller` 1; a true answer takes 005E2E20 and vt[2]+0x14, a stop. Answers 005EB150's. Completion: 100% of the false arm.
/// # Errors
/// A stop below, or 005EB150 true.
pub fn actor_place_if_blocked(world: &mut World, actor: Entity) -> Result<bool, StreamError> {
    if !actor_can_be_placed(world, actor)? {
        return Ok(false);
    }
    let position = super::ai_package::reference_position(world, actor)?;
    if position_blocked(world, actor, position, true)? {
        return Err(StreamError(
            "005EB3B1: 005E2E20 and vt[2]+0x14 for a blocked actor",
        ));
    }
    Ok(false)
}

/// IDA: 005EB150(actor, position, test_controller): a cell (+0x40) at process level 5 or 6, else false. A non-finite or NaN component answers true. An interior reads 00441800(cell, 0, 2): with an entry whose +0x20 isn't zero and whose +0x2C radius is below the distance to it, true. Then with `test_controller`, no save-load bit 11 and a character controller, 008949C0 and 00895F00 decide, a stop; with none, false. An exterior's 0043FB80 height test is a stop. Completion: 100% of an interior with no controller.
/// TODO: [ ] 00441800's cell +0x54 block, which the port doesn't hold: run-ad's had a zero +0x20 for the prison, the same branch as none. The exterior and controller arms.
fn position_blocked(
    world: &mut World,
    actor: Entity,
    position: [f32; 3],
    test_controller: bool,
) -> Result<bool, StreamError> {
    let Some(cell) = parent_cell(world, actor) else {
        return Ok(false);
    };
    let level = world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .process_level_26 as i8;
    if level != 6 && level != 5 {
        return Ok(false);
    }
    if position.iter().any(|value| !value.is_finite()) {
        return Ok(true);
    }
    if !cell_is_interior(world, cell) {
        return Err(StreamError("005EB2B3: an exterior's 0043FB80 height test"));
    }
    if !test_controller || super::forms_references::save_load_flags(world)? & 0x800 != 0 {
        return Ok(false);
    }
    if super::character_controller::actor_character_controller(world, actor).is_some() {
        return Err(StreamError(
            "005EB33A: 008949C0 with a character controller",
        ));
    }
    Ok(false)
}

/// IDA: 005E2E20(position, cell, keep_height 0, match_height 0, flag 0) on the cell's path grid (004AF170, +0x44): 0067D820 finds the point; none, or a point with no connections, takes the actor's own position (vt+0x174); otherwise the first connection: the edge from the point to it normalized (0043F350), Random_Next(0) % 1000 / 1000.0 times its length, and the point plus the unit edge times that, at 24 bits. With no grid an interior keeps `position`; an exterior's height arm is a stop. Completion: 100% of an interior with flag 0 and both height bytes 0.
/// # Errors
/// The draw's, or a stop.
pub fn place_on_path_grid(
    world: &mut World,
    actor: Entity,
    position: [f32; 3],
    cell: Entity,
) -> Result<[f32; 3], StreamError> {
    let Some(grid) = world
        .get::<TesObjectCell>(cell)
        .expect("a cell")
        .path_grid_44
    else {
        if !cell_is_interior(world, cell) {
            return Err(StreamError(
                "005E2E94: an exterior's height with no path grid",
            ));
        }
        return Ok(position);
    };
    let own = |world: &World| super::ai_package::reference_position(world, actor);
    let Some(point) = nearest_point(world, position, actor, grid)? else {
        return own(world);
    };
    let points = grid_points(world, grid)?;
    let Some(next) = points[point].connections_20.items().next() else {
        return own(world);
    };
    let start = points[point].position_14;
    let end = points[next].position_14;
    let mut edge = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
    let length = super::forms_world::normalize_and_length_24_bit(&mut edge);
    let draw = world
        .get_non_send::<super::ai_conditions::ScriptRandom>()
        .ok_or(StreamError("005E2F27: Random_Next with no host CRT"))?
        .0
        .clone();
    let random = draw(world);
    world
        .get_resource_or_init::<PathingLocationCounts>()
        .placements
        .push((actor, point, next));
    // 005E2F2C `cdq; idiv 1000`: the signed remainder; `fild`, `fdiv` by 1000.0 and times the length, stored as a float.
    let along = (random % 1000) as f32 / 1000.0 * length;
    if along > length {
        return Ok(end);
    }
    Ok([
        start[0] + edge[0] * along,
        start[1] + edge[1] * along,
        start[2] + edge[2] * along,
    ])
}

// IDA: N/A the grid's +0x24 array.
fn grid_points(world: &World, grid: Entity) -> Result<&Vec<PathGridPoint>, StreamError> {
    world
        .get::<TesPathGrid>(grid)
        .expect("a path grid")
        .points_24
        .as_ref()
        .ok_or(StreamError("004E5730: a path grid with no points"))
}

/// IDA: 0067D820(position, reference, 0, 0)'s interior arm: 004D8B90 (the reference's cell is interior), the cell's grid, a 004E7DF0 PathingLocation at `position` whose flag byte takes bit 3 when the cell's water (004CACE0) is above the position's z and bit 6 when 004CBBB0 finds a subspace holding it; then 004E5730(location, reference, 0, null). An exterior is a stop. Completion: 100% of the interior arm.
fn nearest_point(
    world: &mut World,
    position: [f32; 3],
    actor: Entity,
    grid: Entity,
) -> Result<Option<usize>, StreamError> {
    let Some(cell) = parent_cell(world, actor) else {
        return Ok(None);
    };
    if !cell_is_interior(world, cell) {
        return Err(StreamError("0067D864: 0067D820's exterior arm"));
    }
    world.get_resource_or_init::<PathingLocationCounts>().built += 1;
    let mut flags = 0u8;
    if super::tes_grid::cell_water_height(world, cell) > position[2] {
        flags |= 8;
    }
    if cell_subspace_at(world, cell, position).is_some() {
        flags |= 0x40;
    }
    grid_nearest_point(world, grid, position, flags, actor)
}

/// IDA: 004E5730(location, reference, 0, null, &out) on a grid with points: x and y `fistp` (nearest) `sar 9`; for dx -1..1 and y - 1..y + 1 the key (y word | (x + dx) << 16), 0 when either is outside 0x7FFF +- 0x7FFE; each listed point not flagged 0x20 (0067ED70) with a connection, its z parity (004E8040) cleared for 0067EDE0 and put back (004E8060, `fild` to the float), the lowest cost wins, the first on a tie. Completion: 100% with the third argument 0.
fn grid_nearest_point(
    world: &mut World,
    grid: Entity,
    position: [f32; 3],
    location_flags: u8,
    actor: Entity,
) -> Result<Option<usize>, StreamError> {
    let base_x = x87::fistp(position[0]) >> 9;
    let base_y = x87::fistp(position[1]) >> 9;
    let mut best = f32::MAX;
    let mut found = None;
    for dx in -1..=1 {
        let x = dx + base_x;
        for y in base_y - 1..=base_y + 1 {
            let key = if (x.wrapping_add(0x7FFF) as u32) <= 0xFFFD
                && (y.wrapping_add(0x7FFF) as u32) <= 0xFFFD
            {
                ((x as u32) << 16) | u32::from(y as u16)
            } else {
                0
            };
            let listed: Vec<usize> = world
                .get::<TesPathGrid>(grid)
                .expect("a path grid")
                .points_44
                .get(key)
                .map(|list| list.items().collect())
                .unwrap_or_default();
            for point in listed {
                let data = grid_points(world, grid)?[point].clone();
                if data.flags_10 & 0x20 != 0 || data.connections_20.is_empty() {
                    continue;
                }
                let parity = x87::fistp(data.position_14[2]) & 1 != 0;
                set_z_parity(world, grid, point, false);
                let cost = path_cost(world, grid, position, location_flags, point, actor)?;
                set_z_parity(world, grid, point, parity);
                if best > cost {
                    best = cost;
                    found = Some(point);
                }
            }
        }
    }
    Ok(found)
}

/// IDA: 004E8060(set) on a point: z `fistp` (nearest), bit 0 set or cleared, `fild` back to the float. Completion: 100%.
fn set_z_parity(world: &mut World, grid: Entity, point: usize, set: bool) {
    let mut data = world.get_mut::<TesPathGrid>(grid).expect("a path grid");
    let z = &mut data.points_24.as_mut().expect("points")[point].position_14[2];
    let whole = x87::fistp(*z);
    *z = if set { whole | 1 } else { whole & !1 } as f32;
}

/// IDA: 0067EDE0(location, point, reference): 005EC1F0's cost, plus fPathWaterExitPenalty when the two flag bytes differ in bit 3 and fPathSpaceExitPenalty when they differ in bit 6; a cost above 0 adds hkVector4::length3 of location minus point (24 bits, 00404C90), otherwise 0. Completion: 100%.
fn path_cost(
    world: &mut World,
    grid: Entity,
    position: [f32; 3],
    location_flags: u8,
    point: usize,
    actor: Entity,
) -> Result<f32, StreamError> {
    let mut cost = point_cost(world, grid, point, actor)?;
    let data = &grid_points(world, grid)?[point];
    let (flags, at) = (data.flags_10, data.position_14);
    if (flags >> 3 ^ location_flags >> 3) & 1 != 0 {
        cost += setting(world, PATH_WATER_EXIT_PENALTY)?;
    }
    if (flags >> 6 ^ location_flags >> 6) & 1 != 0 {
        cost += setting(world, PATH_SPACE_EXIT_PENALTY)?;
    }
    if 0.0 >= cost {
        return Ok(0.0);
    }
    let delta = [
        position[0] - at[0],
        position[1] - at[1],
        position[2] - at[2],
    ];
    let length = ((delta[1] * delta[1] + delta[0] * delta[0]) + delta[2] * delta[2]).sqrt();
    Ok(length + cost)
}

/// IDA: 005EC1F0(reference, point): 1.0, less fPathPreferredPointBonus for a z-odd point when the base isn't a creature. A point with bit 0 or 1 reads its bits 4 and 3; otherwise bit 3 (under water) runs the swim test (005E06C0), a stop here, and 0067ED30 stores the answer in bit 4. With bit 4 the 00519C70 and 005EA640 arms, a stop; without, 005E1E90 adds fPathInvalidMovementTypePenalty, and bit 3 the wading penalty, a stop. Completion: 100% of a dry point for an NPC.
fn point_cost(
    world: &mut World,
    grid: Entity,
    point: usize,
    actor: Entity,
) -> Result<f32, StreamError> {
    let data = grid_points(world, grid)?[point].clone();
    let mut cost = 1.0f32;
    let base = world
        .get::<TesObjectRefr>(actor)
        .expect("a reference")
        .base_form_1c;
    if x87::fistp(data.position_14[2]) & 1 != 0
        && base.is_none_or(|base| form_type(world, base) != 0x24)
    {
        cost = 1.0 - setting(world, PATH_PREFERRED_POINT_BONUS)?;
    }
    let flags = data.flags_10;
    let (swimming, wading);
    if flags & 3 != 0 {
        swimming = flags & 0x10 != 0;
        wading = flags & 8 != 0;
    } else {
        if flags & 8 != 0 {
            return Err(StreamError("005EC24F: 005E06C0 for a point under water"));
        }
        swimming = false;
        wading = false;
        let mut grid_data = world.get_mut::<TesPathGrid>(grid).expect("a path grid");
        grid_data.points_24.as_mut().expect("points")[point].flags_10 &= !0x10;
    }
    if swimming {
        return Err(StreamError("005EC33C: 00519C70 for a swimming point"));
    }
    if actor_placement_exempt(world, actor)? {
        cost += setting(world, PATH_INVALID_MOVEMENT_TYPE_PENALTY)?;
    }
    if wading {
        return Err(StreamError("005EC3A0: 005E3400 for a wading point"));
    }
    Ok(cost)
}

/// IDA: N/A whether a process is a HighProcess, for 005FA320's tests.
#[must_use]
pub fn is_high(world: &World, process: Entity) -> bool {
    world.get::<HighProcess>(process).is_some()
}
