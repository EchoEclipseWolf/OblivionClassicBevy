//! The glue between an actor and `oblivion_pathing`'s grid search (0067EB60): the host questions 0067E160 and 0067E8D0
//! ask about the actor, its cell and its process. The search itself, the open list and the costs are in the crate.
//! Nothing in the port calls this yet: its callers (the PathHigh / PathMiddleHigh builders 00689300, 006895D0,
//! 0068B440 and the console's 00510C70) aren't ported.

use bevy_ecs::prelude::*;
use oblivion_core::error::GameError;
use oblivion_pathing::{ActorPathFacts, PathGridPoint, PathGridSearch, PathSearchHost};

use super::tes::TesGlobals;
use crate::tes::actor::MobileObject;
use crate::tes::actor_placement::{PathingLocationCounts, actor_path_facts};
use crate::tes::actor_process::LowProcess;
use crate::tes::forms_references::parent_cell;
use crate::tes::forms_world::{TesObjectCell, TesPathGrid, cell_is_interior, cell_subspace_at};

/// IDA: N/A `PathSearchHost` for an actor in an interior: the reference's cell's grid is the one the points index.
struct ActorSearchHost<'w> {
    world: &'w mut World,
    actor: Entity,
    grid: Entity,
    facts: ActorPathFacts,
}

impl PathSearchHost for ActorSearchHost<'_> {
    /// IDA: 0067D7B0 PathGrid_ResetLoadedSearchState: TES +0x34's grid through 004E55A0 (none answers at once); without
    /// +0x34 the loaded exterior cells, a stop. Completion: 100% of the interior arm.
    fn reset_loaded_grids(&mut self) -> Result<(), GameError> {
        let interior = self
            .world
            .resource::<TesGlobals>()
            .tes
            .as_ref()
            .and_then(|tes| tes.current_interior_cell_34);
        let Some(interior) = interior else {
            return Err(GameError::unclassified(
                0x0067D7CC,
                "0067D7CC: 0067D7B0's loaded exterior cells",
            ));
        };
        let Some(grid) = self
            .world
            .get::<TesObjectCell>(interior)
            .expect("a cell")
            .path_grid_44
        else {
            return Ok(());
        };
        let data = self
            .world
            .get_mut::<TesPathGrid>(grid)
            .expect("a grid")
            .into_inner();
        oblivion_pathing::grid::reset_grid_search_state(
            data.points_24.as_deref_mut(),
            data.point_count_30,
        )
    }

    /// IDA: 004D8B90 TESObjectREFR_IsInInterior: with a parent cell (+0x40) its interior bit. Without one (the
    /// TESChildCell's cell) isn't reached: `actor_path_grid_search` stops first. Completion: 100% with a parent cell.
    fn reference_in_interior(&mut self) -> bool {
        parent_cell(self.world, self.actor).is_some_and(|cell| cell_is_interior(self.world, cell))
    }

    /// IDA: 0043F840 TES_IsPositionInLoadedGrid: TES +0x34 answers true; the exterior grid test isn't reached, since
    /// 0067E160 asks only after 004D8B90, which an interior answers. Completion: 100% of an interior.
    fn position_in_loaded_grid(&mut self, _: [f32; 3]) -> bool {
        self.world
            .resource::<TesGlobals>()
            .tes
            .as_ref()
            .is_some_and(|tes| tes.current_interior_cell_34.is_some())
    }

    /// IDA: 0067D820 PathGrid_FindNearestPointForReference(position, reference, keep_five, null): the interior arm in
    /// `actor_placement`, which stops on an exterior; keep-five is a stop there. Completion: 100% of the interior arm with
    /// keep-five false.
    fn nearest_point(
        &mut self,
        position: [f32; 3],
        keep_five: bool,
    ) -> Result<Option<usize>, GameError> {
        if keep_five {
            return Err(GameError::unclassified(
                0x004E58E4,
                "004E58E4: 004E5730's keep-five arm",
            ));
        }
        nearest_point(self.world, position, self.actor, self.grid)
    }

    /// IDA: 005E0660 Actor_GetScaledBoundHeight, shared with the package radius test.
    fn reference_height(&mut self) -> Result<f32, GameError> {
        crate::tes::actor::actor_height(self.world, self.actor)
    }

    /// IDA: 006859A0: a set 00B3C089 answers true; otherwise Havok's cast (00446A10), a stop. Completion: 0% of the cast.
    /// TODO: [ ] 006859A0's cast through the host's physics.
    fn straight_walk_clear(&mut self, _: [f32; 3], _: [f32; 3]) -> Result<bool, GameError> {
        Err(GameError::unclassified(
            0x006859A0,
            "006859A0: Havok's straight-walk cast",
        ))
    }

    /// IDA: 005E0710 Actor_IsPathConnectionVetoed: no process (+0x58), or a null path (+0x34, vt+0x40C), answers false; a
    /// path is a stop (00683C70 for a PathHigh). Completion: 100% of a process with no path.
    /// TODO: [ ] 00683C70 for a PathHigh.
    fn connection_vetoed(&mut self, _: usize, _: usize) -> Result<bool, GameError> {
        let Some(process) = self
            .world
            .get::<MobileObject>(self.actor)
            .and_then(|mobile| mobile.process_58)
        else {
            return Ok(false);
        };
        let low = self
            .world
            .get::<LowProcess>(process)
            .ok_or(GameError::unclassified(
                0x005E0733,
                "005E0733: vt+0x40C on a process other than a LowProcess",
            ))?;
        if low.pathing_34 != 0 {
            return Err(GameError::unclassified(
                0x005E074C,
                "005E074C: a process's path (00683C70)",
            ));
        }
        Ok(false)
    }

    // IDA: N/A the grid's +0x24 array.
    fn grid(&mut self) -> &mut [PathGridPoint] {
        self.world
            .get_mut::<TesPathGrid>(self.grid)
            .expect("a grid")
            .into_inner()
            .points_24
            .as_deref_mut()
            .unwrap_or_default()
    }

    // IDA: N/A the facts gathered before the search.
    fn facts(&self) -> ActorPathFacts {
        self.facts
    }
}

/// IDA: 0067D760 PathGridSearch_Constructor then 0067EB60 PathGridSearch_Build(from, to, actor, near_range) for an
/// actor in an interior cell with a path grid. With no parent cell, or one with no grid, a stop: 0067D820 would answer
/// null for both ends, but 0067D7B0 and 004D8B90 differ there. Completion: 100% of what the host answers.
/// # Errors
/// The host's stops.
pub fn actor_path_grid_search(
    world: &mut World,
    actor: Entity,
    from: [f32; 3],
    to: [f32; 3],
    near_range: bool,
) -> Result<(bool, PathGridSearch), GameError> {
    let grid = parent_cell(world, actor)
        .and_then(|cell| {
            world
                .get::<TesObjectCell>(cell)
                .expect("a cell")
                .path_grid_44
        })
        .ok_or(GameError::unclassified(
            0x0067E1F6,
            "0067E1F6: a search for an actor with no cell grid",
        ))?;
    let facts = crate::tes::actor_placement::actor_path_facts(world, actor);
    let mut host = ActorSearchHost {
        world,
        actor,
        grid,
        facts,
    };
    let mut search = PathGridSearch::default();
    let built = search.build(from, to, &mut host, near_range)?;
    Ok((built, search))
}

/// IDA: 004E5170 TESPathGrid_SetLinkedPointsEnabled(grid, reference, enable): the reference's +0x34 points through
/// `oblivion_pathing::grid::set_linked_points_enabled`; a disable that set any point marks the grid's cell (+0x20) with
/// vt+0x40 TESForm_SetChangeFlags(0x1000000) (004E51CD). Answers whether any point was set. Completion: 100%.
/// # Errors
/// 0046ABF0's.
pub fn path_grid_set_linked_points_enabled(
    world: &mut World,
    grid: Entity,
    reference: Entity,
    enable: bool,
) -> Result<bool, GameError> {
    let data = world
        .get_mut::<TesPathGrid>(grid)
        .expect("a grid")
        .into_inner();
    let key = crate::tes::forms_world::path_grid_reference_key(reference);
    let linked = data.references_34.get(key);
    let any = match data.points_24.as_deref_mut() {
        Some(points) => oblivion_pathing::grid::set_linked_points_enabled(points, linked, enable),
        // The port's points are the array's indexes, and 004E65B0's PGRL arm lists only points the array holds.
        None if linked.is_some_and(|list| !list.is_empty()) => {
            return Err(GameError::unclassified(
                0x004E51AD,
                "004E51AD: a linked point with no +0x24 array",
            ));
        }
        None => false,
    };
    if !enable && any {
        let cell = data.cell_20.ok_or(GameError::unclassified(
            0x004E51CD,
            "004E51CD: a path grid with no cell",
        ))?;
        crate::tes::forms_items::set_change_flags(world, cell, 0x0100_0000)?;
    }
    Ok(any)
}

/// IDA: 004E51F0 TESPathGrid_SetLinkedPointsEnabledInLoadedCells(reference, enable), for 005062D0 DisableLinkedPathPoints
/// (enable 0) and 005062B0 EnableLinkedPathPoints (1): with TES +0x34, its grid (004AF170) through 004E5170, or nothing
/// with none; without +0x34, each of the uGridsToLoad x uGridsToLoad grid cells (00482150, the count read once at
/// 004E5218) with a grid, a stop when any cell is attached. Completion: 100% of the interior arm.
/// TODO: [ ] the loaded exterior cells' loop (004E5230).
/// # Errors
/// 004E5170's, or the exterior loop.
pub fn set_linked_path_points_enabled(
    world: &mut World,
    reference: Entity,
    enable: bool,
) -> Result<(), GameError> {
    let tes = world
        .resource::<TesGlobals>()
        .tes
        .as_ref()
        .ok_or(GameError::unclassified(
            0x004E51F0,
            "004E51F0: a null g_tes",
        ))?;
    if let Some(interior) = tes.current_interior_cell_34 {
        if let Some(grid) = world
            .get::<TesObjectCell>(interior)
            .expect("a cell")
            .path_grid_44
        {
            path_grid_set_linked_points_enabled(world, grid, reference, enable)?;
        }
        return Ok(());
    }
    if tes
        .grid_cells_08
        .as_ref()
        .is_some_and(|cells| cells.grid_10.iter().any(|slot| slot.cell_00.is_some()))
    {
        return Err(GameError::unclassified(
            0x004E5230,
            "004E5230: 004E51F0's loaded exterior cells",
        ));
    }
    Ok(())
}

/// IDA: 0067D820(position, reference, 0, 0)'s interior arm: 004D8B90 (the reference's cell is interior), the cell's grid, a 004E7DF0 PathingLocation at `position` whose flag byte takes bit 3 when the cell's water (004CACE0) is above the position's z and bit 6 when 004CBBB0 finds a subspace holding it; then 004E5730(location, reference, 0, null). An exterior is a stop. Completion: 100% of the interior arm.
pub(crate) fn nearest_point(
    world: &mut World,
    position: [f32; 3],
    actor: Entity,
    grid: Entity,
) -> Result<Option<usize>, GameError> {
    let Some(cell) = parent_cell(world, actor) else {
        return Ok(None);
    };
    if !cell_is_interior(world, cell) {
        return Err(GameError::unclassified(
            0x0067D864,
            "0067D864: 0067D820's exterior arm",
        ));
    }
    world.get_resource_or_init::<PathingLocationCounts>().built += 1;
    let mut flags = 0u8;
    if crate::tes::tes_grid::cell_water_height(world, cell) > position[2] {
        flags |= 8;
    }
    if cell_subspace_at(world, cell, position).is_some() {
        flags |= 0x40;
    }
    grid_nearest_point(world, grid, position, flags, actor)
}

/// IDA: 004E5730(location, reference, 0, null, &out) on the grid's +0x24 points and +0x44 buckets: `oblivion_pathing::grid::grid_nearest_point`, with the location's flags and position and the actor's facts. Completion: 100% with the third argument 0.
fn grid_nearest_point(
    world: &mut World,
    grid: Entity,
    position: [f32; 3],
    location_flags: u8,
    actor: Entity,
) -> Result<Option<usize>, GameError> {
    let facts = actor_path_facts(world, actor);
    let mut data = world.get_mut::<TesPathGrid>(grid).expect("a path grid");
    let data = &mut *data;
    let buckets = &data.points_44;
    let bucket = |key: u32| -> Vec<usize> {
        buckets
            .get(key)
            .map(|list| list.items().collect())
            .unwrap_or_default()
    };
    oblivion_pathing::grid::grid_nearest_point(
        data.points_24.as_deref_mut(),
        &bucket,
        oblivion_pathing::NearestPointQuery {
            location_flags,
            location_position: position,
            keep_five: false,
            exclude: None,
        },
        &facts,
    )
}
