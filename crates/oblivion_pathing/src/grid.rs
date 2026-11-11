//! The path grid's points and the queries over them: 004E7DF0's 0x2C point with its 0067EDC0 search node, the
//! accessors 0067EC40..0067EDA0, the bucket key 004E5320, the z parity 004E8040 / 004E8060, the reset 004E55A0 and the
//! 3x3 nearest-point search 004E5730.

use oblivion_core::error::GameError;
use oblivion_core::pointer_list::PointerList;
use oblivion_core::x87;

use crate::cost::{ActorPathFacts, location_point_cost};

/// IDA: 0067EC90..0067ED80's bits of +0x10.
pub const NODE_OPENED: u8 = 0x01;
pub const NODE_CLOSED: u8 = 0x02;
pub const POINT_UNDER_WATER: u8 = 0x08;
pub const POINT_SWIMMING: u8 = 0x10;
pub const POINT_DISABLED: u8 = 0x20;
pub const POINT_IN_SUBSPACE: u8 = 0x40;
/// IDA: 0067EDA0 `and byte ptr [ecx+10h], 68h`: the bits a search reset keeps.
const NODE_RESET_KEEPS: u8 = 0x68;

/// IDA: 004E7DF0 TESPathGridPoint_Constructor (0x2C), on 0067EDC0 AStarNode_Constructor: +0x00 f, +0x04 g, +0x08 h,
/// +0x0C the parent (an index of the grid's array here), +0x10 the flag byte (the `NODE_` and `POINT_` bits), +0x14 the
/// position, +0x20 the connections, as indexes of the grid's array, +0x28 a word nothing in the loader sets. Bytes
/// +0x11..+0x13 are never written (heap bytes). Completion: 100% of the layout.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PathGridPoint {
    pub total_cost_00: f32,
    pub cost_04: f32,
    pub heuristic_08: f32,
    pub parent_0c: Option<usize>,
    pub flags_10: u8,
    pub position_14: [f32; 3],
    pub connections_20: PointerList<usize>,
    pub field_28: u32,
}

impl PathGridPoint {
    /// IDA: 0067EC50 AStarNode_UpdateTotalCost: `fld [ecx+8]; fadd [ecx+4]; fstp [ecx]`, h + g at 24 bits. Completion: 100%.
    pub fn update_total_cost(&mut self) {
        self.total_cost_00 = self.heuristic_08 + self.cost_04;
    }

    /// IDA: 0067ECC0 AStarNode_IsOpened: bit 0. Completion: 100%.
    #[must_use]
    pub const fn is_opened(&self) -> bool {
        self.flags_10 & NODE_OPENED != 0
    }

    /// IDA: 0067ECD0 AStarNode_SetOpened. Completion: 100%.
    pub const fn set_opened(&mut self, set: bool) {
        self.set_flag(NODE_OPENED, set);
    }

    /// IDA: 0067EC90 AStarNode_IsClosed: bit 1. Completion: 100%.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.flags_10 & NODE_CLOSED != 0
    }

    /// IDA: 0067ECA0 AStarNode_SetClosed. Completion: 100%.
    pub const fn set_closed(&mut self, set: bool) {
        self.set_flag(NODE_CLOSED, set);
    }

    /// IDA: 0067ECF0 AStarNode_IsUnderWater: bit 3. Completion: 100%.
    #[must_use]
    pub const fn is_under_water(&self) -> bool {
        self.flags_10 & POINT_UNDER_WATER != 0
    }

    /// IDA: 0067ED20 AStarNode_IsSwimming: bit 4. Completion: 100%.
    #[must_use]
    pub const fn is_swimming(&self) -> bool {
        self.flags_10 & POINT_SWIMMING != 0
    }

    /// IDA: 0067ED70 AStarNode_IsDisabled: bit 5. Completion: 100%.
    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        self.flags_10 & POINT_DISABLED != 0
    }

    /// IDA: 0067ED00 / 0067ED30 / 0067ED50 / 0067ED80: `or` the bit, or `and` its complement. Completion: 100%.
    pub const fn set_flag(&mut self, bit: u8, set: bool) {
        if set {
            self.flags_10 |= bit;
        } else {
            self.flags_10 &= !bit;
        }
    }

    /// IDA: 0067EDA0 AStarNode_ResetSearchState: f, g and h 0, the parent null, the flag byte `and 68h` (water, disabled
    /// and subspace stay). Completion: 100%.
    pub const fn reset_search_state(&mut self) {
        self.flags_10 &= NODE_RESET_KEEPS;
        self.total_cost_00 = 0.0;
        self.parent_0c = None;
        self.cost_04 = 0.0;
        self.heuristic_08 = 0.0;
    }

    /// IDA: 004E8040 TESPathGridPoint_GetZParityFlag: bit 0 of z's `fistp` (round to nearest). Completion: 100%.
    #[must_use]
    pub fn z_parity(&self) -> bool {
        x87::fistp(self.position_14[2]) & 1 != 0
    }

    /// IDA: 004E8060 TESPathGridPoint_SetZParityFlag: z `fistp`, bit 0 set or cleared, `fild` back into the float.
    /// Completion: 100%.
    pub fn set_z_parity(&mut self, set: bool) {
        let whole = x87::fistp(self.position_14[2]);
        self.position_14[2] = if set { whole | 1 } else { whole & !1 } as f32;
    }
}

/// IDA: 004E5320 TESPathGrid_GetBucketKey: each coordinate `fistp` then `sar 9`; (value + 0x7FFF) above 0xFFFD unsigned
/// answers 0; x << 16 | y's low word. Completion: 100%.
#[must_use]
pub fn path_grid_key(position: [f32; 3]) -> u32 {
    let x = x87::fistp(position[0]) >> 9;
    let y = x87::fistp(position[1]) >> 9;
    bucket_key(x, y)
}

// IDA: 004E5320's and 004E5730's packing of a bucket. Completion: 100%.
const fn bucket_key(x: i32, y: i32) -> u32 {
    if (x.wrapping_add(0x7FFF) as u32) > 0xFFFD || (y.wrapping_add(0x7FFF) as u32) > 0xFFFD {
        return 0;
    }
    ((x as u32) << 16) | (y as u16) as u32
}

/// IDA: 004E55A0 TESPathGrid_ResetSearchState: with a +0x24 array, 0067EDA0 on each nonnull point below +0x30 (the
/// DATA count, as a word). A point index past the array is the original's read beyond it, a host stop. Completion: 100%.
/// # Errors
/// The stop.
pub fn reset_grid_search_state(
    points: Option<&mut [PathGridPoint]>,
    point_count_30: u16,
) -> Result<(), GameError> {
    let Some(points) = points else {
        return Ok(());
    };
    for index in 0..usize::from(point_count_30) {
        points
            .get_mut(index)
            .ok_or(GameError::unclassified(
                0x004E55B8,
                "004E55B8: a point index past the +0x24 array",
            ))?
            .reset_search_state();
    }
    Ok(())
}

/// IDA: 004E5730's inputs: the location (a 0067D820 PathingLocation or a grid point: its +0x10 flags and +0x14
/// position), `keep_five` (the third argument) and the `exclude` list (the fourth, as indexes).
#[derive(Clone, Copy, Debug)]
pub struct NearestPointQuery<'a> {
    pub location_flags: u8,
    pub location_position: [f32; 3],
    pub keep_five: bool,
    pub exclude: Option<&'a [usize]>,
}

/// IDA: 004E5170 TESPathGrid_SetLinkedPointsEnabled(grid, reference, enable)'s grid side: `linked` is the reference's
/// +0x34 list (PGRL), found by `NiTPointerMap_Lookup`; from the head, each point's disabled bit (0067ED80) becomes
/// `!enable`, until a node with neither item nor next. Answers whether any point was set: the caller then, for a
/// disable, marks the grid's cell (+0x20) with vt+0x40 TESForm_SetChangeFlags(0x1000000) (004E51CD). A missing entry
/// answers false. Completion: 100% of the grid side.
pub fn set_linked_points_enabled(
    points: &mut [PathGridPoint],
    linked: Option<&PointerList<usize>>,
    enable: bool,
) -> bool {
    let mut any = false;
    for point in linked.into_iter().flat_map(PointerList::items) {
        points[point].set_flag(POINT_DISABLED, !enable);
        any = true;
    }
    any
}

/// IDA: 004E5730 TESPathGrid_FindNearestPoint(location, reference, keep_five, exclude, &confirmed) on a grid. A null
/// +0x24 array answers null. Location x and y `fistp` (nearest) `sar 9`; for dx -1..1 and y - 1..y + 1 the bucket key
/// (0 outside 0x7FFF +- 0x7FFE), looked up through `bucket`; each listed point not disabled (0067ED70), with a
/// connection and not in `exclude`, costs 0067EDE0 with its z parity cleared (004E8040 / 004E8060) and put back; the
/// lowest cost wins, the first on a tie. `confirmed` (the sixth argument) is 0 on this arm. Completion: 100% with
/// `keep_five` false.
/// TODO: [ ] the keep-five arm: 004E49B0's five cheapest within fPathPointReachDistance-like `00B3A448`, then 00687C30's
///       walk test on each (004E59CD), which can answer a better point and set `confirmed`.
/// # Errors
/// The costs', or the keep-five arm.
pub fn grid_nearest_point(
    points: Option<&mut [PathGridPoint]>,
    bucket: &dyn Fn(u32) -> Vec<usize>,
    query: NearestPointQuery<'_>,
    facts: &ActorPathFacts,
) -> Result<Option<usize>, GameError> {
    let Some(points) = points else {
        return Ok(None);
    };
    let base_x = x87::fistp(query.location_position[0]) >> 9;
    let base_y = x87::fistp(query.location_position[1]) >> 9;
    let mut best = f32::MAX;
    let mut found = None;
    for dx in -1..=1 {
        let x = dx + base_x;
        for y in base_y - 1..=base_y + 1 {
            for point in bucket(bucket_key(x, y)) {
                let data = &points[point];
                if data.is_disabled() || data.connections_20.is_empty() {
                    continue;
                }
                if query
                    .exclude
                    .is_some_and(|exclude| exclude.contains(&point))
                {
                    continue;
                }
                let parity = data.z_parity();
                points[point].set_z_parity(false);
                let cost = location_point_cost(
                    query.location_flags,
                    query.location_position,
                    points,
                    point,
                    facts,
                )?;
                points[point].set_z_parity(parity);
                if best > cost {
                    best = cost;
                    found = Some(point);
                }
                if query.keep_five {
                    return Err(GameError::unclassified(
                        0x004E58E4,
                        "004E58E4: 004E5730's keep-five arm",
                    ));
                }
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use oblivion_core::fixtures;

    /// IDA: N/A `fixtures/rows/pathing_prison.rows` (run-af's after_move, cell 1FBB9's grid) as points and buckets.
    pub(crate) struct PrisonGrid {
        pub header: Vec<u32>,
        pub points: Vec<PathGridPoint>,
        pub buckets: Vec<(u32, u32, Vec<usize>)>,
        /// The +0x34 entries: the reference's form ID and its points.
        pub linked: Vec<(u32, Vec<usize>)>,
    }

    pub(crate) fn prison_grid() -> PrisonGrid {
        prison_grid_rows("pathing_prison")
    }

    /// IDA: N/A one of the prison grid's row files: `pathing_prison` (run-af's after_move) or `pathing_prison_frame`
    /// (run-x's frame_after_move, after command 65).
    pub(crate) fn prison_grid_rows(name: &str) -> PrisonGrid {
        let rows = fixtures::rows(name);
        fixtures::ledger_read(name, &vec![true; rows.len()]);
        let hex = |text: &str| u32::from_str_radix(text, 16).unwrap();
        let mut grid = PrisonGrid {
            header: Vec::new(),
            points: Vec::new(),
            buckets: Vec::new(),
            linked: Vec::new(),
        };
        for row in &rows {
            let fields: Vec<&str> = row.split(' ').collect();
            match fields[0] {
                "GRID" => grid.header = fields[1..].iter().map(|f| hex(f)).collect(),
                "POINT" => {
                    assert_eq!(hex(fields[1]) as usize, grid.points.len());
                    let mut point = PathGridPoint {
                        total_cost_00: f32::from_bits(hex(fields[2])),
                        cost_04: f32::from_bits(hex(fields[3])),
                        heuristic_08: f32::from_bits(hex(fields[4])),
                        parent_0c: (fields[5] != "-").then(|| hex(fields[5]) as usize),
                        flags_10: hex(fields[6]) as u8,
                        position_14: [
                            f32::from_bits(hex(fields[7])),
                            f32::from_bits(hex(fields[8])),
                            f32::from_bits(hex(fields[9])),
                        ],
                        field_28: hex(fields[10]),
                        ..PathGridPoint::default()
                    };
                    // The rows list connections from the head; prepending them in reverse rebuilds that order.
                    for connection in fields[11..].iter().rev() {
                        point
                            .connections_20
                            .prepend_non_null(hex(connection) as usize);
                    }
                    grid.points.push(point);
                }
                "BUCKET" => grid.buckets.push((
                    hex(fields[1]),
                    hex(fields[2]),
                    fields[3..].iter().map(|f| hex(f) as usize).collect(),
                )),
                "LINKED" => grid.linked.push((
                    hex(fields[1]),
                    fields[2..].iter().map(|f| hex(f) as usize).collect(),
                )),
                other => panic!("{other}"),
            }
        }
        grid
    }

    /// The snapshot's points have no search state yet, and every point's bucket is the one 004E5320 names.
    #[test]
    fn prison_points_have_no_search_state_and_sit_in_their_buckets() {
        let grid = prison_grid();
        assert_eq!(grid.header, [0x11C, 0x11C, 1, 0x22, 0]);
        assert_eq!(grid.points.len(), 284);
        for (index, point) in grid.points.iter().enumerate() {
            let mut reset = point.clone();
            reset.reset_search_state();
            assert_eq!(&reset, point, "{index}: 0067EDA0 changes nothing yet");
            let key = path_grid_key(point.position_14);
            let (bucket, _, _) = grid
                .buckets
                .iter()
                .find(|(_, k, listed)| *k == key && listed.contains(&index))
                .unwrap_or_else(|| panic!("{index} is in bucket {key:08X}"));
            assert_eq!(*bucket, key % 37, "NiTPointerMap's key % 37");
        }
        let listed: usize = grid.buckets.iter().map(|(_, _, listed)| listed.len()).sum();
        assert_eq!(listed, 284);
    }

    /// 004E5170 for 3E31C, the prison's crumbling wall (command 65 in CGCrumbleWall01SCRIPT's OnLoad): run-x's grid
    /// before and after the frame that runs it differs only in the linked point's disabled bit.
    #[test]
    fn disabling_the_walls_linked_points_matches_run_x() {
        let mut before = prison_grid();
        let after = prison_grid_rows("pathing_prison_frame");
        assert_eq!(before.linked, [(0x3E31C, vec![0x97])]);
        assert_eq!(after.linked, before.linked);
        let mut list = PointerList::default();
        for &point in before.linked[0].1.iter().rev() {
            list.prepend_non_null(point);
        }
        assert!(set_linked_points_enabled(
            &mut before.points,
            Some(&list),
            false
        ));
        assert_eq!(before.points, after.points);
        assert!(!set_linked_points_enabled(&mut before.points, None, false));
        assert!(set_linked_points_enabled(
            &mut before.points,
            Some(&list),
            true
        ));
        assert_eq!(
            before.points[0x97].flags_10, 0,
            "EnableLinkedPathPoints clears it again"
        );
    }

    #[test]
    fn node_accessors_follow_the_bit_layout() {
        let mut point = PathGridPoint {
            flags_10: 0xFF,
            total_cost_00: 1.0,
            cost_04: 2.0,
            heuristic_08: 3.0,
            parent_0c: Some(4),
            ..PathGridPoint::default()
        };
        assert!(point.is_opened() && point.is_closed() && point.is_disabled());
        point.reset_search_state();
        assert_eq!(point.flags_10, 0x68);
        assert_eq!(
            (
                point.total_cost_00,
                point.cost_04,
                point.heuristic_08,
                point.parent_0c
            ),
            (0.0, 0.0, 0.0, None)
        );
        point.heuristic_08 = 0.5;
        point.cost_04 = 0.25;
        point.update_total_cost();
        assert_eq!(point.total_cost_00, 0.75);
        point.position_14[2] = -466.0;
        assert!(!point.z_parity());
        point.set_z_parity(true);
        assert_eq!(point.position_14[2], -465.0);
        assert_eq!(bucket_key(-0x8000, 0), 0);
        assert_eq!(bucket_key(0x7FFF, 0), 0);
        assert_eq!(bucket_key(-0x7FFF, 0), 0x8001_0000);
        assert_eq!(bucket_key(-1, 6), 0xFFFF_0006);
    }
}
