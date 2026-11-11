//! Oblivion's path grid and path search, ported from the exe (pathing agent, 2026-11-08).
//!
//! # Boundary
//! This crate depends only on `oblivion_core`: no `bevy_ecs`, no `World`, no `Entity`. It holds the pathing data that is
//! pure (the 0x2C `TESPathGridPoint` with its embedded A* node, the bucket key, the search object 0067EB60 fills) and
//! the algorithms over it (the 3x3 nearest-point search, the point costs, the A* and its open list). Everything that
//! reads a form, a cell, an actor, a process, a game setting or Havok stays in `oblivion_sim` as thin glue: it answers
//! the [`search::PathSearchHost`] questions and builds an [`cost::ActorPathFacts`] before a search. `oblivion_sim`
//! depends on this crate (`TesPathGrid`'s points are [`grid::PathGridPoint`]s), never the other way round. The Phase 06
//! singleton constructors (`oblivion_sim::tes::pathing`) stay where they are, because they are `Resource`s.
//!
//! # The system in the exe (1.2.0416)
//! Addresses are IDA's; names in brackets are the ones the database carries after the pathing batches.
//!
//! **Data.**
//! - `TESPathGrid` (PGRD, 0x54, constructor 004E6070, load 004E74E0 / 004E65B0): +0x20 its cell, +0x24 a
//!   `NiTArray<TESPathGridPoint*>` (vtable A47590), +0x28 PGRI's inter-cell points, +0x30 the DATA point count (u16),
//!   +0x34 `NiTPointerMap<TESObjectREFR*, BSSimpleList<TESPathGridPoint*>*>` (PGRL: points linked to a reference),
//!   +0x44 `NiTPointerMap<u32, BSSimpleList<TESPathGridPoint*>*>` keyed by 004E5320 (x and y `fistp` then `sar 9`).
//! - `TESPathGridPoint` (0x2C, 004E7DF0 [TESPathGridPoint_Constructor]), whose base 0067EDC0 [AStarNode_Constructor]
//!   is the search node: +0x00 f, +0x04 g, +0x08 h, +0x0C parent, +0x10 the flag byte (bit 0 opened, bit 1 closed,
//!   bit 3 under water, bit 4 swimming, bit 5 disabled, bit 6 in a subspace), +0x14 the position, +0x20 the
//!   connections (a `BSSimpleList`), +0x28 zero. The accessors are 0067EC40..0067EDA0. The same 0x2C shape is the
//!   "PathingLocation" 0067D820 builds on its stack for a free position.
//! - Bit 0 of the z coordinate's `fistp` (004E8040 / 004E8060) marks a preferred point.
//!
//! **Grid queries.**
//! - 0067D820 [PathGrid_FindNearestPointForReference](position, reference, keep_five, exclude): an interior reference
//!   asks its cell's grid through 004E5730 with a location whose flags carry the water (bit 3) and subspace (bit 6)
//!   tests; an exterior one asks up to four grids of the loaded worldspace around the position, weighting a point the
//!   keep-five arm didn't confirm by 100, or with the position outside the loaded grid the nearest point of every
//!   loaded cell (004E55E0).
//! - 004E5730 [TESPathGrid_FindNearestPoint]: the 3x3 bucket walk, each enabled point with a connection costed by
//!   0067EDE0 with its z parity cleared; the cheapest, first on a tie.
//! - 0067EDE0 [PathingLocation_GetPointCost] = 005EC1F0 [Actor_GetPathPointCost] + the water / space exit penalties,
//!   plus the distance when positive.
//!
//! **Search.** 0067EB60 [PathGridSearch_Build](this 0x2C, from, to, reference, near_range): +0x00 from, +0x0C to,
//! +0x18 a byte its callers set (its zero asks 0067D820 for the keep-five arm), +0x1C the start point, +0x20 the end
//! point, +0x24 the found (or lowest-f) point, +0x28 the reference. It resets the loaded grids' node state (0067D7B0,
//! 004E55A0, 0067EDA0), answers true for equal ends, asks 0067E160 [PathGridSearch_FindEndPoints] whether a grid search
//! is needed (it isn't for a straight walk 006859A0 allows), then 0067E8D0 [PathGridSearch_Run], the A*: an
//! `AStarNodeList` open list (vtable A748D4) prepended by 0067EFE0 and popped by 0067F030's first-lowest-f scan from the
//! head; the heuristic 0067EEC0 is twice the straight distance; costs are 0067EDE0 with the popped point as the
//! location. 0067E3D0 turns the parent chain into a path's node list (0068C280 and the node setters 0068CA30..0068CB10).
//! Its callers are the PathHigh / PathMiddleHigh builders (00689300, 006895D0, 0068B440), 00510C70 (the console's
//! `CalcPathToPoint`), 00617590, 0064BBC0 and 00684F40.
//!
//! **Paths.** `PathLow` (0x14, vtable A74EDC, type 0: the inter-cell route over door and world nodes), `PathMiddleHigh`
//! (vtable A74F3C, type 1) and `PathHigh` (vtable A74CA4, type 2: the in-cell route over the grid; vt+0x18 00688120 is
//! its per-frame follow). A process keeps its path at +0x34 (the processes' vt+0x40C). 005E0710 lets a PathHigh veto a
//! connection it marked bad (00683C70).
//!
//! **The world-node graph.** 00B3BE00 [g_pathingWorldNodeMap] (0067FCF0): `NiTPointerMap<TESForm*, NiTPointerMap<
//! TESForm*, BSSimpleList<AStarWorldNode*>*>*>` of 0x14 `AStarWorldNode`s, one per linked door pair (00680090,
//! 00680A00, 0067FD70), listed at 00B3BE18; 00680110 searches it with an `AStarWorldNodeList` (vtable A749A4) for
//! `PathLow` (0068AB20). `oblivion_sim::tes::pathing::PathingDoorNodes` holds which doors get a node.
//!
//! **The background thread.** 00B3BF80 [g_pathBuilder] (00683030 / 00682EF0): a `BackgroundLoader` with three
//! `NiTPointerMap<Actor*, PathingData*>` of 37 buckets, +0x44 the request in hand. Its thread runs 0042FAD0
//! [BackgroundLoaderThread_Run]: wait +0x28, wait +0x40, the owner's vt+4 (006824F0 [PathBuilder_ProcessCurrentRequest]
//! -> 00682450 [PathBuilder_BuildLowPath]: a PathLow made on demand at the request's +0x08, 0068B030
//! [PathLow_BuildPathToPosition], state +0x20 = 2), release +0x40 and +0x34. The queue: 006836E0
//! [PathBuilder_QueueRequest] files a 0x28 `PathingData` (00682580: +0x00 the actor, +0x04 its 006830B0 value, +0x08
//! the PathLow, +0x0C the target interior, +0x10 the target world, +0x14 the position, +0x20 the state, +0x24 the result
//! byte) in +0x10 for a process-level-0 actor, else +0x20, or builds a same-cell request at once into +0x30; 00683420
//! [PathBuilder_Update] (from ActorProcessManager_Update) files the finished +0x40 (00682950) and 006829C0
//! [PathBuilder_StartNextRequest] hands the next to the thread (+0x40 and +0x44); 006826D0 [PathBuilder_CancelRequest],
//! 00682820 [PathBuilder_TakeBuiltPath], 006825C0 [PathBuilder_HasRequest] and 00682A90
//! [PathBuilder_PrintQueueDebugText] read it. The maps are keyed by Actor pointers, so their bucket order (which
//! 006829C0's "first entry" depends on) is the original heap's: a port needs a recorded host substitution.
//!
//! # What is ported here
//! The point and its node (`grid`), the bucket key, the z parity, the node reset, the 3x3 nearest-point search with its
//! exclusion list (`grid`), the two costs (`cost`), and 0067EB60 / 0067E160 / 0067E8D0 with the open list and the
//! heuristic (`search`). Each unported arm is a `GameError` at its address; `docs/open-items.md`'s "Pathing (pathing
//! agent, 2026-11-08)" lists them.

pub mod cost;
pub mod grid;
pub mod search;

pub use cost::{ActorPathFacts, actor_point_cost, location_point_cost};
pub use grid::{NearestPointQuery, PathGridPoint, path_grid_key};
pub use search::{PathGridSearch, PathSearchHost};
