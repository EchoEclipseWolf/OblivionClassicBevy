//! The path grid search: 0067D760 PathGridSearch_Constructor, 0067EB60 PathGridSearch_Build, 0067E160
//! PathGridSearch_FindEndPoints, 0067E8D0 PathGridSearch_Run (the A*), its open list (`AStarNodeList`, vtable A748D4:
//! 0067EFE0 and 0067F030) and the heuristic 0067EEC0. The x87 runs at 24 bits, so each `fstp dword` step is an f32
//! operation; comparisons against double constants are exact in f64.

use oblivion_core::error::GameError;

use crate::cost::{ActorPathFacts, location_point_cost};
use crate::grid::PathGridPoint;

/// IDA: 00B15450..00B15458, the unset position (FLT_MAX, FLT_MAX, FLT_MAX) in .data; nothing writes it.
pub const UNSET_POSITION: [f32; 3] = [f32::MAX; 3];

/// IDA: the 0x2C object 0067D760 PathGridSearch_Constructor builds: +0x00 from and +0x0C to (both 00B15450), +0x18 1,
/// +0x1C the start point, +0x20 the end point, +0x24 the found point, +0x28 the reference (here only whether it was
/// stored; the reference is the host's). Points are indexes of the host's grid.
#[derive(Clone, Debug, PartialEq)]
pub struct PathGridSearch {
    pub from_00: [f32; 3],
    pub to_0c: [f32; 3],
    /// +0x18: 0067D820's third argument is this byte's zero (so 0 asks for the keep-five arm). 0067D760 stores 1 and
    /// 0068B440 keeps it; other callers' stores aren't read yet.
    pub endpoints_without_keep_five_18: u8,
    pub start_point_1c: Option<usize>,
    pub end_point_20: Option<usize>,
    pub found_point_24: Option<usize>,
    pub reference_stored_28: bool,
}

impl Default for PathGridSearch {
    /// IDA: 0067D760 PathGridSearch_Constructor, in store order. Completion: 100%.
    fn default() -> Self {
        Self {
            from_00: UNSET_POSITION,
            to_0c: UNSET_POSITION,
            start_point_1c: None,
            end_point_20: None,
            found_point_24: None,
            reference_stored_28: false,
            endpoints_without_keep_five_18: 1,
        }
    }
}

/// IDA: N/A what the search asks outside the grid, for its reference (the actor). `oblivion_sim` answers it.
pub trait PathSearchHost {
    /// IDA: 0067D7B0 PathGrid_ResetLoadedSearchState: TES +0x34's grid (004E55A0), or every loaded exterior cell's.
    /// # Errors
    /// The host's stops.
    fn reset_loaded_grids(&mut self) -> Result<(), GameError>;
    /// IDA: 004D8B90 TESObjectREFR_IsInInterior on the reference.
    fn reference_in_interior(&mut self) -> bool;
    /// IDA: 0043F840 TES_IsPositionInLoadedGrid: true with TES +0x34, otherwise the position's 4096-unit cell inside
    /// the loaded grid.
    fn position_in_loaded_grid(&mut self, position: [f32; 3]) -> bool;
    /// IDA: 0067D820 PathGrid_FindNearestPointForReference(position, reference, keep_five, null), as an index of `grid`.
    /// # Errors
    /// The host's stops.
    fn nearest_point(
        &mut self,
        position: [f32; 3],
        keep_five: bool,
    ) -> Result<Option<usize>, GameError>;
    /// IDA: 005E0660 Actor_GetScaledBoundHeight on the reference.
    /// # Errors
    /// The host's stops.
    fn reference_height(&mut self) -> Result<f32, GameError>;
    /// IDA: 006859A0 (a Havok cast between the two positions): true when the straight walk is clear.
    /// # Errors
    /// The host's stops.
    fn straight_walk_clear(&mut self, from: [f32; 3], to: [f32; 3]) -> Result<bool, GameError>;
    /// IDA: 005E0710 Actor_IsPathConnectionVetoed(reference, from, to): the reference's process (+0x58) path (+0x34,
    /// the processes' vt+0x40C), when it's a PathHigh (vt+4 answers 2), through 00683C70.
    /// # Errors
    /// The host's stops.
    fn connection_vetoed(&mut self, from: usize, to: usize) -> Result<bool, GameError>;
    /// IDA: N/A the grid the point indexes name: the reference's cell's +0x44 grid's +0x24 array.
    fn grid(&mut self) -> &mut [PathGridPoint];
    /// IDA: N/A 005EC1F0's and 0067EDE0's inputs for the reference.
    fn facts(&self) -> ActorPathFacts;
}

/// IDA: 0067EEC0 PathGridSearch_Heuristic(a, b): FLT_MAX with a null end, otherwise twice |a - b|: the components as
/// floats, `(dx * dx + dy * dy) + dz * dz` stored, `__CIsqrt` stored, `fadd st, st` stored. Completion: 100%.
#[must_use]
pub fn heuristic(a: [f32; 3], b: [f32; 3]) -> f32 {
    let delta = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let squared = (delta[0] * delta[0] + delta[1] * delta[1]) + delta[2] * delta[2];
    let length = squared.sqrt();
    length + length
}

/// IDA: an `AStarNodeList` (NiTPointerList<AStarNode*>, vtable A748D4), from the head: 0067EFE0 AStarNodeList_AddHead
/// links a new node at the head; 0067F030 AStarNodeList_PopLowestTotalCost walks from the head keeping the first node
/// whose +0x00 is strictly below the best so far (`fcompp; test ah, 41h`), starting at FLT_MAX, and unlinks it. A node
/// whose f is FLT_MAX or NaN is never popped. Completion: 100%.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AStarNodeList {
    /// From the head.
    pub nodes: Vec<usize>,
}

impl AStarNodeList {
    /// IDA: 0067EFE0 AStarNodeList_AddHead. Completion: 100%.
    pub fn add_head(&mut self, point: usize) {
        self.nodes.insert(0, point);
    }

    /// IDA: 0067F030 AStarNodeList_PopLowestTotalCost. Completion: 100%.
    pub fn pop_lowest_total_cost(&mut self, points: &[PathGridPoint]) -> Option<usize> {
        let mut best = f32::MAX;
        let mut found = None;
        for (at, &point) in self.nodes.iter().enumerate() {
            let total = points[point].total_cost_00;
            if best > total {
                best = total;
                found = Some(at);
            }
        }
        found.map(|at| self.nodes.remove(at))
    }
}

impl PathGridSearch {
    /// IDA: 0067EB60 PathGridSearch_Build(this, from, to, reference, near_range): from and to FLT_MAX, +0x1C..+0x28 0,
    /// 0067D7B0, then from and to stored; equal ends (`fucompp` each component) answer true; otherwise 0067E160's false
    /// answers true (no grid search: a straight walk, or no usable points) and its true answers 0067E8D0's. The
    /// reference is the host's (a null reference reaches 0067E160's null test, answering true). Completion: 100%.
    /// # Errors
    /// The host's stops.
    pub fn build(
        &mut self,
        from: [f32; 3],
        to: [f32; 3],
        host: &mut dyn PathSearchHost,
        near_range: bool,
    ) -> Result<bool, GameError> {
        self.from_00 = UNSET_POSITION;
        self.to_0c = UNSET_POSITION;
        self.start_point_1c = None;
        self.end_point_20 = None;
        self.found_point_24 = None;
        self.reference_stored_28 = false;
        host.reset_loaded_grids()?;
        self.from_00 = from;
        self.to_0c = to;
        if self.to_0c == self.from_00 {
            return Ok(true);
        }
        if self.find_end_points(host, near_range)? {
            return self.run(host);
        }
        Ok(true)
    }

    // IDA: 0067E160's tail, 0067E38D: +0x1C, +0x20 and +0x28 cleared, answering false. Completion: 100%.
    fn clear_end_points(&mut self) -> Result<bool, GameError> {
        self.start_point_1c = None;
        self.end_point_20 = None;
        self.reference_stored_28 = false;
        Ok(false)
    }

    /// IDA: 0067E160 PathGridSearch_FindEndPoints(this, reference, near_range). An unset from or to answers false. With
    /// an interior reference (004D8B90) or either end in the loaded grid (0043F840), 0067D820 at each end (keep-five on
    /// +0x18's zero), and the reference stored; no point at either end, or the same point, clears and answers false.
    /// Otherwise a = |from - start|^2, b = |to - end|^2, c = |from - to|^2 (hkVector4::lengthSquared3, stored): true with
    /// `near_range` and both a and b below 4194304.0 (A4CAE0), or both below 250000.0 (A748C8), or c above a + b, or the
    /// two points differing in the water bit (0067ECF0); with c at or above 1000000.0 (A47CA8), clears and false;
    /// otherwise 005E0660 times 0.5 (A2FAA0) raises both ends and 006859A0's clear walk clears and answers false, a
    /// blocked one answers true. Completion: 100%.
    /// # Errors
    /// The host's stops.
    pub fn find_end_points(
        &mut self,
        host: &mut dyn PathSearchHost,
        near_range: bool,
    ) -> Result<bool, GameError> {
        if self.from_00 == UNSET_POSITION {
            return Ok(false);
        }
        if self.to_0c == UNSET_POSITION {
            return Ok(false);
        }
        if !host.reference_in_interior()
            && !host.position_in_loaded_grid(self.from_00)
            && !host.position_in_loaded_grid(self.to_0c)
        {
            return self.clear_end_points();
        }
        let keep_five = self.endpoints_without_keep_five_18 == 0;
        self.start_point_1c = host.nearest_point(self.from_00, keep_five)?;
        let end = host.nearest_point(self.to_0c, keep_five)?;
        self.end_point_20 = end;
        self.reference_stored_28 = true;
        let (Some(start), Some(end)) = (self.start_point_1c, self.end_point_20) else {
            return self.clear_end_points();
        };
        if start == end {
            return self.clear_end_points();
        }
        let squared = |a: [f32; 3], b: [f32; 3]| {
            let delta = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
            (delta[0] * delta[0] + delta[1] * delta[1]) + delta[2] * delta[2]
        };
        let (start_water, end_water, start_at, end_at) = {
            let grid = host.grid();
            (
                grid[start].is_under_water(),
                grid[end].is_under_water(),
                grid[start].position_14,
                grid[end].position_14,
            )
        };
        let to_start = squared(self.from_00, start_at);
        let to_end = squared(self.to_0c, end_at);
        let between = squared(self.from_00, self.to_0c);
        if near_range && 4_194_304.0 > to_start && 4_194_304.0 > to_end {
            return Ok(true);
        }
        if 250_000.0 > to_start && 250_000.0 > to_end {
            return Ok(true);
        }
        if between > to_start + to_end {
            return Ok(true);
        }
        if end_water != start_water {
            return Ok(true);
        }
        // 0067E311 `fcomp; test ah, 5; jp`: only an ordered c below 1000000.0 goes on, so NaN clears.
        #[expect(
            clippy::neg_cmp_op_on_partial_ord,
            reason = "0067E319: an unordered compare clears the end points"
        )]
        let far = !(f64::from(between) < 1_000_000.0);
        if far {
            return self.clear_end_points();
        }
        let raise = (f64::from(host.reference_height()?) * 0.5) as f32;
        let from = [self.from_00[0], self.from_00[1], self.from_00[2] + raise];
        let to = [self.to_0c[0], self.to_0c[1], self.to_0c[2] + raise];
        if !host.straight_walk_clear(from, to)? {
            return Ok(true);
        }
        self.clear_end_points()
    }

    /// IDA: 0067E8D0 PathGridSearch_Run(this, reference): with both points, the start's g 0, h 0067EEC0(start, end),
    /// f, parent null, opened, and the open list's head. Each 0067F030 pop until one finds the end: the end popped is
    /// found (+0x24 the end); otherwise each connection from the head (+0x20), until found: a disabled one (0067ED70) or
    /// one 005E0710 vetoes is passed over; the end gets the popped point as parent and is found; any other costs
    /// g(popped) + 0067EDE0(popped, it), and when it was neither opened nor closed, or its g is above that, it is opened
    /// and listed if it wasn't (a closed one is updated but not listed again), takes g, h 0067EEC0(it, end), f and the
    /// parent, and becomes +0x24 when that is null or has a higher f. Then the popped point is closed. Answers found.
    /// Completion: 100%.
    /// # Errors
    /// The host's stops.
    pub fn run(&mut self, host: &mut dyn PathSearchHost) -> Result<bool, GameError> {
        let (Some(start), Some(end)) = (self.start_point_1c, self.end_point_20) else {
            return Ok(false);
        };
        let facts = host.facts();
        let mut found = false;
        let mut open = AStarNodeList::default();
        {
            let grid = host.grid();
            let end_at = grid[end].position_14;
            let point = &mut grid[start];
            point.cost_04 = 0.0;
            point.heuristic_08 = heuristic(point.position_14, end_at);
            point.update_total_cost();
            point.parent_0c = None;
            point.set_opened(true);
        }
        open.add_head(start);
        while let Some(popped) = open.pop_lowest_total_cost(host.grid()) {
            if found {
                break;
            }
            if popped == end {
                found = true;
                self.found_point_24 = Some(end);
            }
            let connections: Vec<usize> = host.grid()[popped].connections_20.items().collect();
            for next in connections {
                if found {
                    break;
                }
                if host.grid()[next].is_disabled() || host.connection_vetoed(popped, next)? {
                    continue;
                }
                if next == end {
                    found = true;
                    host.grid()[end].parent_0c = Some(popped);
                    self.found_point_24 = Some(end);
                    continue;
                }
                let grid = host.grid();
                let (location_flags, location_at, popped_cost) = (
                    grid[popped].flags_10,
                    grid[popped].position_14,
                    grid[popped].cost_04,
                );
                let step = location_point_cost(location_flags, location_at, grid, next, &facts)?;
                let cost = popped_cost + step;
                let point = &grid[next];
                // 0067EA62 `fcomp; test ah, 41h; jnp`: only a g below or equal to the new cost keeps the point, so an
                // unordered (NaN) compare relaxes it.
                #[expect(
                    clippy::neg_cmp_op_on_partial_ord,
                    reason = "0067EA6D: an unordered compare relaxes the point"
                )]
                let relax = (!point.is_opened() && !point.is_closed()) || !(point.cost_04 <= cost);
                if relax {
                    if !point.is_opened() {
                        grid[next].set_opened(true);
                        open.add_head(next);
                    }
                    let end_at = grid[end].position_14;
                    let point = &mut grid[next];
                    point.cost_04 = cost;
                    point.heuristic_08 = heuristic(point.position_14, end_at);
                    point.update_total_cost();
                    point.parent_0c = Some(popped);
                    let total = point.total_cost_00;
                    if self
                        .found_point_24
                        .is_none_or(|best| grid[best].total_cost_00 > total)
                    {
                        self.found_point_24 = Some(next);
                    }
                }
            }
            host.grid()[popped].set_closed(true);
        }
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::tests::prison_grid;

    /// A host over the prison's grid from run-af's after_move: an interior NPC with no process path, and settings
    /// chosen for the test (no run has the prison's GameSettings next to a search; see the capture request).
    struct PrisonHost {
        points: Vec<PathGridPoint>,
        buckets: Vec<(u32, u32, Vec<usize>)>,
        resets: u32,
        walk_clear: bool,
    }

    impl PathSearchHost for PrisonHost {
        fn reset_loaded_grids(&mut self) -> Result<(), GameError> {
            self.resets += 1;
            let count = self.points.len() as u16;
            crate::grid::reset_grid_search_state(Some(&mut self.points), count)
        }
        fn reference_in_interior(&mut self) -> bool {
            true
        }
        fn position_in_loaded_grid(&mut self, _: [f32; 3]) -> bool {
            true
        }
        fn nearest_point(
            &mut self,
            position: [f32; 3],
            keep_five: bool,
        ) -> Result<Option<usize>, GameError> {
            let buckets = self.buckets.clone();
            let lookup = move |key: u32| {
                buckets
                    .iter()
                    .find(|(_, k, _)| *k == key)
                    .map(|(_, _, listed)| listed.clone())
                    .unwrap_or_default()
            };
            let facts = self.facts();
            crate::grid::grid_nearest_point(
                Some(&mut self.points),
                &lookup,
                crate::grid::NearestPointQuery {
                    location_flags: 0,
                    location_position: position,
                    keep_five,
                    exclude: None,
                },
                &facts,
            )
        }
        fn reference_height(&mut self) -> Result<f32, GameError> {
            Ok(128.0)
        }
        fn straight_walk_clear(&mut self, _: [f32; 3], _: [f32; 3]) -> Result<bool, GameError> {
            Ok(self.walk_clear)
        }
        fn connection_vetoed(&mut self, _: usize, _: usize) -> Result<bool, GameError> {
            Ok(false)
        }
        fn grid(&mut self) -> &mut [PathGridPoint] {
            &mut self.points
        }
        fn facts(&self) -> ActorPathFacts {
            ActorPathFacts {
                is_creature: false,
                movement_type_invalid: Ok(false),
                preferred_point_bonus: Ok(0.25),
                invalid_movement_type_penalty: Ok(1000.0),
                water_exit_penalty: Ok(100.0),
                space_exit_penalty: Ok(50.0),
            }
        }
    }

    fn prison_host() -> PrisonHost {
        let grid = prison_grid();
        PrisonHost {
            points: grid.points,
            buckets: grid.buckets,
            resets: 0,
            walk_clear: false,
        }
    }

    #[test]
    fn the_open_list_pops_the_first_lowest_from_the_head() {
        let mut points = vec![PathGridPoint::default(); 4];
        for (point, total) in points.iter_mut().zip([3.0, 1.0, 1.0, f32::MAX]) {
            point.total_cost_00 = total;
        }
        let mut open = AStarNodeList::default();
        for point in 0..4 {
            open.add_head(point);
        }
        assert_eq!(open.nodes, [3, 2, 1, 0]);
        assert_eq!(
            open.pop_lowest_total_cost(&points),
            Some(2),
            "the tie's first from the head"
        );
        assert_eq!(open.pop_lowest_total_cost(&points), Some(1));
        assert_eq!(open.pop_lowest_total_cost(&points), Some(0));
        assert_eq!(
            open.pop_lowest_total_cost(&points),
            None,
            "FLT_MAX is never below FLT_MAX"
        );
        assert_eq!(heuristic([3.0, 4.0, 0.0], [0.0; 3]), 10.0);
    }

    /// The A* across the prison grid between two far points: a self-consistency check (no run has a finished search;
    /// see the capture request in docs/open-items.md).
    #[test]
    fn the_search_crosses_the_prison_grid() {
        let mut host = prison_host();
        let from = host.points[0].position_14;
        let to = host.points[283].position_14;
        let mut search = PathGridSearch::default();
        assert!(search.build(from, to, &mut host, false).unwrap());
        assert_eq!(host.resets, 1);
        let (start, end) = (search.start_point_1c.unwrap(), search.end_point_20.unwrap());
        assert_eq!(search.found_point_24, Some(end));
        // The parent chain runs from the end back to the start along connections, and every g on it adds up.
        let facts = host.facts();
        let mut at = end;
        let mut steps = 0;
        while at != start {
            let parent = host.points[at].parent_0c.expect("a parent");
            assert!(host.points[parent].connections_20.items().any(|c| c == at));
            if at != end {
                let mut points = host.points.clone();
                let (flags, position) = (points[parent].flags_10, points[parent].position_14);
                let step = location_point_cost(flags, position, &mut points, at, &facts).unwrap();
                assert_eq!(host.points[at].cost_04, host.points[parent].cost_04 + step);
                assert!(host.points[at].is_opened());
            }
            at = parent;
            steps += 1;
            assert!(steps < 284);
        }
        assert!(steps > 1);
        assert_eq!(host.points[start].cost_04, 0.0);
        assert!(host.points[start].is_closed());
        // A second build resets every node first.
        let before = host.points.clone();
        search.build(from, to, &mut host, false).unwrap();
        assert_eq!(host.points, before, "the same search again");
    }

    #[test]
    fn near_ends_need_no_grid_search() {
        let mut host = prison_host();
        let from = host.points[0].position_14;
        let to = host.points[1].position_14;
        let mut search = PathGridSearch::default();
        assert!(search.build(from, to, &mut host, false).unwrap());
        // Both ends sit on their points, so a and b are 0: below 250000, a search runs.
        assert_eq!(search.found_point_24, search.end_point_20);
        let mut search = PathGridSearch::default();
        assert!(search.build(from, from, &mut host, false).unwrap());
        assert_eq!(
            search.start_point_1c, None,
            "equal ends answer true at once"
        );
    }
}
