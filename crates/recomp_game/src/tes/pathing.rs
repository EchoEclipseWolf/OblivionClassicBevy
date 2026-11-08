//! Phase 06 rows 06-D-2/06-D-3 (D5): WinMain 0040EB0C/0040EB11, the pathing singletons 00B3BE00 (0067FCF0) and 00B3BF80
//! (00683030) as typed values with the original sizes, vtables (evidence only), bucket counts, semaphores and critical
//! section. No thread starts: the closure has no CreateThread. Runtime evidence: P06-RUN-001.
//!
//! Host substitution: the semaphores and the critical section are host handles from `PathingPlatform`. Nothing waits on
//! them until a thread starts.
//! TODO: [ ] AI/pathing phases (carried): the 9 readers of 00B3BE00 and the 22 of 00B3BF80 (including
//!       ActorProcessManager_Update), the map entries, the BackgroundLoader thread start and the teardown.

use bevy_ecs::prelude::*;

// IDA: the vtables the constructors store (evidence only): the NiTMapBase and NiTPointerMap vtables of the two map
// instantiations, then BSThread.
const WORLD_NODE_MAP_BASE_VTABLE: u32 = 0x00A7_48F4;
const WORLD_NODE_MAP_VTABLE: u32 = 0x00A7_4964;
const ACTOR_MAP_BASE_VTABLE: u32 = 0x00A7_4B78;
const ACTOR_MAP_VTABLE: u32 = 0x00A7_4BEC;
const BS_THREAD_VTABLE: u32 = 0x00A3_D0F0;
const BACKGROUND_LOADER_THREAD_VTABLE: u32 = 0x00A3_631C;
const BACKGROUND_LOADER_VTABLE: u32 = 0x00A3_6328;
const PATH_BUILDER_VTABLE: u32 = 0x00A7_4C0C;
/// IDA: 0067FD34 push 0BFh.
const WORLD_NODE_MAP_BUCKETS: u32 = 191;
/// IDA: 00682F2A mov eax, 25h.
const ACTOR_MAP_BUCKETS: u32 = 37;

/// IDA: N/A a host semaphore or critical section. Never compared with evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathingHandle(pub u32);

/// IDA: N/A the seam for FormHeap_Allocate (false is a null return), CreateSemaphoreA and InitializeCriticalSection.
pub trait PathingPlatform {
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn allocate(&mut self, bytes: u32) -> bool;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn create_semaphore(&mut self, initial: i32, maximum: i32) -> PathingHandle;
    /// IDA: N/A a seam the port calls; an address, if any, is at the call.
    fn initialize_critical_section(&mut self) -> PathingHandle;
}

/// IDA: N/A the seam as a record: `default_pathing_platform` in the C++, and its test double. Every request succeeds except
/// a size in `failed_allocations`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordedPathingPlatform {
    pub failed_allocations: Vec<u32>,
    pub allocations: Vec<u32>,
    pub semaphores: Vec<(i32, i32)>,
    pub critical_sections: u32,
    next_handle: u32,
}

impl RecordedPathingPlatform {
    // IDA: N/A a helper, with no address of its own.
    fn handle(&mut self) -> PathingHandle {
        self.next_handle += 1;
        PathingHandle(self.next_handle)
    }
}

impl PathingPlatform for RecordedPathingPlatform {
    /// IDA: N/A `RecordedPathingPlatform`'s side of `PathingPlatform`.
    fn allocate(&mut self, bytes: u32) -> bool {
        self.allocations.push(bytes);
        !self.failed_allocations.contains(&bytes)
    }
    /// IDA: N/A `RecordedPathingPlatform`'s side of `PathingPlatform`.
    fn create_semaphore(&mut self, initial: i32, maximum: i32) -> PathingHandle {
        self.semaphores.push((initial, maximum));
        self.handle()
    }
    /// IDA: N/A `RecordedPathingPlatform`'s side of `PathingPlatform`.
    fn initialize_critical_section(&mut self) -> PathingHandle {
        self.critical_sections += 1;
        self.handle()
    }
}

/// IDA: the NiTPointerMap shape 0067FB70/00682EF0 build inline: vtable (the NiTMapBase vtable while the buckets are
/// allocated, then the NiTPointerMap one), +4 bucket count, +8 FormHeap(4 * count) zeroed (no null check), +0xC count 0.
/// Entries are carried.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathingPointerMap {
    pub original_vtable_00: u32,
    pub bucket_count_04: u32,
    /// +0x08: 32-bit entry pointers.
    pub buckets_08: Vec<u32>,
    pub count_0c: u32,
}

/// IDA: 0047CEA0 BSThread (vtable 00A3D0F0): +4 FormHeap(0x80) critical section (0 when the allocation fails), +8..+0x14
/// zero, +0x18 byte zero.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BsThreadState {
    pub original_vtable_00: u32,
    pub critical_section_04: Option<PathingHandle>,
    pub unk_08: u32,
    pub unk_0c: u32,
    pub unk_10: u32,
    pub unk_14: u32,
    pub unk_18: u8,
}

/// IDA: N/A which BackgroundLoader a thread's +0x1C names. The only one built so far is the PathBuilder's base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundLoaderOwner {
    PathBuilderBase,
}

/// IDA: 0042FA50 BackgroundLoaderThread (FormHeap 0x44, vtable 00A3631C) on the BSThread base: owner +0x1C, then three
/// (initial, maximum, handle) semaphore triples at +0x20/+0x2C/+0x38, the last fixed at (1, 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundLoaderThread {
    pub thread_00: BsThreadState,
    pub owner_1c: BackgroundLoaderOwner,
    pub initial_a_20: i32,
    pub maximum_a_24: i32,
    pub semaphore_a_28: PathingHandle,
    pub initial_b_2c: i32,
    pub maximum_b_30: i32,
    pub semaphore_b_34: PathingHandle,
    pub initial_c_38: i32,
    pub maximum_c_3c: i32,
    pub semaphore_c_40: PathingHandle,
}

/// IDA: 0042FB60 BackgroundLoader(this, createThread, a1, a2, a3, a4) (vtable 00A36328): +4 byte 0 (bytes +5..+7
/// unwritten), +8 0, +0xC the FormHeap(0x44) thread when createThread (0 on allocation failure).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BackgroundLoader {
    pub original_vtable_00: u32,
    pub unk_04: u8,
    pub unk_08: u32,
    pub thread_0c: Option<Box<BackgroundLoaderThread>>,
}

/// IDA: 00682EF0 PathBuilder (FormHeap 0x48, vtable 00A74C0C): BackgroundLoader(1, 0, 1, 0, 1), three
/// NiTPointerMap<Actor*, PathingData*> (37 buckets) at +0x10/+0x20/+0x30, +0x40 = +0x44 = 0.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathBuilder {
    pub base_00: BackgroundLoader,
    pub actor_maps_10: [PathingPointerMap; 3],
    pub unk_40: u32,
    pub unk_44: u32,
}

/// IDA: 00B3BE00 g_pathingWorldNodeMap and 00B3BF80 g_pathBuilder.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct PathingSingletons {
    pub world_node_map_b3be00: Option<Box<PathingPointerMap>>,
    pub path_builder_b3bf80: Option<Box<PathBuilder>>,
}

// IDA: 0067FB70 / 00682F2A..00683004: base vtable, +4 count, +0xC 0, the zeroed FormHeap(4 * count) bucket array (no null
// check: the C++ port's terminal allocation path, 03-A), then the final vtable. Completion: 100%.
fn construct_pointer_map(base_vtable: u32, vtable: u32, buckets: u32) -> PathingPointerMap {
    let mut map = PathingPointerMap {
        original_vtable_00: base_vtable,
        bucket_count_04: buckets,
        count_0c: 0,
        buckets_08: vec![0; buckets as usize],
    };
    map.original_vtable_00 = vtable;
    map
}

// IDA: 0042FA50 BackgroundLoaderThread(owner, a1, a2, a3, a4) in store order. 0042FA54 0047CEA0 BSThread: vtable, +4..+0x14
// = 0, +0x18 = 0, then FormHeap(0x80) and 00401000 InitializeCriticalSection into +4. Then 0042FA59..0042FABA. Completion:
// 100%.
fn construct_background_loader_thread(
    owner: BackgroundLoaderOwner,
    initial_a: i32,
    maximum_a: i32,
    initial_b: i32,
    maximum_b: i32,
    platform: &mut dyn PathingPlatform,
) -> BackgroundLoaderThread {
    let critical_section = if platform.allocate(0x80) {
        Some(platform.initialize_critical_section())
    } else {
        None
    };
    let thread = BsThreadState {
        original_vtable_00: BS_THREAD_VTABLE,
        critical_section_04: critical_section,
        ..BsThreadState::default()
    };
    let semaphore_a = platform.create_semaphore(initial_a, maximum_a);
    let semaphore_b = platform.create_semaphore(initial_b, maximum_b);
    let semaphore_c = platform.create_semaphore(1, 1);
    let mut thread = BackgroundLoaderThread {
        thread_00: thread,
        owner_1c: owner,
        initial_a_20: initial_a,
        maximum_a_24: maximum_a,
        semaphore_a_28: semaphore_a,
        initial_b_2c: initial_b,
        maximum_b_30: maximum_b,
        semaphore_b_34: semaphore_b,
        initial_c_38: 1,
        maximum_c_3c: 1,
        semaphore_c_40: semaphore_c,
    };
    thread.thread_00.original_vtable_00 = BACKGROUND_LOADER_THREAD_VTABLE;
    thread
}

/// IDA: 00680A00's 0x14 AStarWorldNode, less +0x00, the index from 006806B0: +0x04 the door, +0x08 its world, +0x0C the
/// linked door, +0x10 its world.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathingDoorNode {
    pub door_04: Entity,
    pub world_08: Option<Entity>,
    pub linked_0c: Entity,
    pub world_10: Option<Entity>,
}

/// IDA: 00B3BE00's entries and `listHead_` 00B3BE18, the door half of 00680090. `gated` is who reached 006800D3 and `nodes`
/// is in build order; the original list prepends. A hash map stands for the two NiTPointerMap levels, whose buckets no
/// reader here walks. Completion: 100% of which doors get a node; the 006806B0 index and the 00680AB0 grid links aren't
/// ported, and no reader is yet.
#[derive(Resource, Debug, Default)]
pub struct PathingDoorNodes {
    pub gated: Vec<Entity>,
    pub nodes: Vec<PathingDoorNode>,
    by_worlds: std::collections::HashMap<(Option<Entity>, Option<Entity>), Vec<usize>>,
    world_keys: std::collections::HashSet<Entity>,
}

impl PathingDoorNodes {
    /// IDA: see the words here, which name the addresses this relates to.
    /// The outer keys of 00B3BE00, as 0067FD70 adds them: each node's two ends. A null end can't reach it: 0067FD70 reads a
    /// missing key's type byte. Completion: 100% of the key set.
    #[must_use]
    pub const fn world_keys(&self) -> &std::collections::HashSet<Entity> {
        &self.world_keys
    }

    /// IDA: 0067F5D0's walk, with 00680410: a node between these worlds already holds the door at either end. Null worlds
    /// answer false, at 0067F60B and 0067F63B. Completion: 100%.
    #[must_use]
    pub fn has_door(
        &self,
        door: Entity,
        world: Option<Entity>,
        linked_world: Option<Entity>,
    ) -> bool {
        if world.is_none() || linked_world.is_none() {
            return false;
        }
        self.by_worlds
            .get(&(world, linked_world))
            .is_some_and(|nodes| {
                nodes.iter().any(|&index| {
                    self.nodes[index].door_04 == door || self.nodes[index].linked_0c == door
                })
            })
    }

    /// IDA: 006800DF..006800FD: 00680A00, 0067FD70's two directions (one when the worlds are the same), then the list.
    /// Completion: 100% of the entries.
    pub fn insert(&mut self, node: PathingDoorNode) {
        let index = self.nodes.len();
        self.nodes.push(node);
        self.world_keys.extend(node.world_08);
        self.world_keys.extend(node.world_10);
        self.by_worlds
            .entry((node.world_08, node.world_10))
            .or_default()
            .push(index);
        if node.world_08 != node.world_10 {
            self.by_worlds
                .entry((node.world_10, node.world_08))
                .or_default()
                .push(index);
        }
    }
}

// IDA: 0067FCF0 PathingWorldNodeMap_EnsureInitialized: when 00B3BE00 is null (0067FD11), FormHeap(0x10) and 0067FB70(191)
// (NiTPointerMap<TESForm*, NiTPointerMap<TESForm*, BSSimpleList<AStarWorldNode*>*>*>); a null allocation stores 0
// (0067FD1A..0067FD57). Completion: 100%.
pub fn ensure_pathing_world_node_map(
    singletons: &mut PathingSingletons,
    platform: &mut dyn PathingPlatform,
) {
    if singletons.world_node_map_b3be00.is_some() {
        return;
    }
    if !platform.allocate(0x10) {
        singletons.world_node_map_b3be00 = None;
        return;
    }
    singletons.world_node_map_b3be00 = Some(Box::new(construct_pointer_map(
        WORLD_NODE_MAP_BASE_VTABLE,
        WORLD_NODE_MAP_VTABLE,
        WORLD_NODE_MAP_BUCKETS,
    )));
}

// IDA: 00683030 PathBuilder_EnsureInitialized: when 00B3BF80 is null (00683051), FormHeap(0x48) and 00682EF0; a null
// allocation stores 0 (0068305A..00683092). 00682F19..00682F23 0042FB60(this, 1, 0, 1, 0, 1); 0042FB9B..0042FBD1 FormHeap(0x44)
// and 0042FA50(this, 0, 1, 0, 1); 00682F39..00683011 the PathBuilder vtable, three 37-bucket maps, +0x40/+0x44. Completion:
// 100%.
pub fn ensure_path_builder(singletons: &mut PathingSingletons, platform: &mut dyn PathingPlatform) {
    if singletons.path_builder_b3bf80.is_some() {
        return;
    }
    if !platform.allocate(0x48) {
        singletons.path_builder_b3bf80 = None;
        return;
    }
    let mut base = BackgroundLoader {
        original_vtable_00: BACKGROUND_LOADER_VTABLE,
        unk_04: 0,
        unk_08: 0,
        thread_0c: None,
    };
    if platform.allocate(0x44) {
        base.thread_0c = Some(Box::new(construct_background_loader_thread(
            BackgroundLoaderOwner::PathBuilderBase,
            0,
            1,
            0,
            1,
            platform,
        )));
    }
    base.original_vtable_00 = PATH_BUILDER_VTABLE;
    let actor_map =
        || construct_pointer_map(ACTOR_MAP_BASE_VTABLE, ACTOR_MAP_VTABLE, ACTOR_MAP_BUCKETS);
    singletons.path_builder_b3bf80 = Some(Box::new(PathBuilder {
        base_00: base,
        actor_maps_10: [actor_map(), actor_map(), actor_map()],
        unk_40: 0,
        unk_44: 0,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use recomp_core::fixtures;

    /// This block's rows of `fixtures/rows/phase06.rows`, as `[run, kind, ..., needle]`. Other blocks are theirs.
    fn rows(kind: &str) -> Vec<Vec<String>> {
        fixtures::rows("phase06")
            .into_iter()
            .filter_map(|row| row.strip_prefix("MOVIE|").map(str::to_owned))
            .map(|row| row.split('|').map(str::to_owned).collect::<Vec<_>>())
            .filter(|row| row[1] == kind)
            .collect()
    }

    /// C++ verify_pathing, with the values P06-RUN-001 recorded: 0067FCF0, 00683030 and their constructors.
    #[test]
    fn verify_pathing() {
        let mut singletons = PathingSingletons::default();
        let mut platform = RecordedPathingPlatform::default();
        ensure_pathing_world_node_map(&mut singletons, &mut platform);
        {
            let map = singletons.world_node_map_b3be00.as_ref().unwrap();
            assert_eq!(map.original_vtable_00, 0x00A7_4964);
            assert_eq!(map.bucket_count_04, 191);
            assert_eq!(map.buckets_08.len(), 191);
            assert_eq!(map.count_0c, 0);
        }
        assert_eq!(platform.allocations, [0x10]);
        let before = singletons.world_node_map_b3be00.clone();
        ensure_pathing_world_node_map(&mut singletons, &mut platform);
        assert_eq!(
            singletons.world_node_map_b3be00, before,
            "0067FCF0 runs once"
        );
        assert_eq!(platform.allocations.len(), 1, "0067FCF0 runs once");

        ensure_path_builder(&mut singletons, &mut platform);
        {
            let builder = singletons.path_builder_b3bf80.as_ref().unwrap();
            assert_eq!(builder.base_00.original_vtable_00, 0x00A7_4C0C);
            assert_eq!(builder.base_00.unk_04, 0);
            assert_eq!(builder.base_00.unk_08, 0);
            assert!(builder.base_00.thread_0c.is_some());
            assert_eq!((builder.unk_40, builder.unk_44), (0, 0));
            for actor_map in &builder.actor_maps_10 {
                assert_eq!(actor_map.original_vtable_00, 0x00A7_4BEC);
                assert_eq!(actor_map.bucket_count_04, 37);
                assert_eq!(actor_map.buckets_08.len(), 37);
                assert_eq!(actor_map.count_0c, 0);
            }
            let thread = builder.base_00.thread_0c.as_ref().unwrap();
            assert_eq!(thread.thread_00.original_vtable_00, 0x00A3_631C);
            assert_eq!(thread.owner_1c, BackgroundLoaderOwner::PathBuilderBase);
            assert!(thread.thread_00.critical_section_04.is_some());
            assert_eq!(platform.critical_sections, 1);
            assert_eq!((thread.initial_a_20, thread.maximum_a_24), (0, 1));
            assert_eq!((thread.initial_b_2c, thread.maximum_b_30), (0, 1));
            assert_eq!((thread.initial_c_38, thread.maximum_c_3c), (1, 1));
            assert_eq!(platform.semaphores, [(0, 1), (0, 1), (1, 1)]);
        }
        assert_eq!(
            platform.allocations,
            [0x10, 0x48, 0x44, 0x80],
            "Allocation order must be 0x48, 0x44, 0x80 after the map"
        );
        let before = singletons.path_builder_b3bf80.clone();
        ensure_path_builder(&mut singletons, &mut platform);
        assert_eq!(singletons.path_builder_b3bf80, before, "00683030 runs once");
        assert_eq!(platform.allocations.len(), 4, "00683030 runs once");

        // Null allocations: 0x10/0x48 store 0; 0x44 leaves +0xC 0; 0x80 leaves +4 0.
        for failing in [0x10, 0x48, 0x44, 0x80] {
            let mut fresh = PathingSingletons::default();
            let mut platform = RecordedPathingPlatform {
                failed_allocations: vec![failing],
                ..RecordedPathingPlatform::default()
            };
            ensure_pathing_world_node_map(&mut fresh, &mut platform);
            ensure_path_builder(&mut fresh, &mut platform);
            assert_eq!(failing == 0x10, fresh.world_node_map_b3be00.is_none());
            assert_eq!(failing == 0x48, fresh.path_builder_b3bf80.is_none());
            if failing == 0x44 {
                let builder = fresh.path_builder_b3bf80.as_ref().unwrap();
                assert!(builder.base_00.thread_0c.is_none());
            }
            if failing == 0x80 {
                let builder = fresh.path_builder_b3bf80.as_ref().unwrap();
                let thread = builder.base_00.thread_0c.as_ref().unwrap();
                assert!(thread.thread_00.critical_section_04.is_none());
            }
        }
    }

    fn le(words: &[u32]) -> String {
        words
            .iter()
            .map(|word| {
                word.to_le_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            })
            .collect()
    }

    /// The words a run dumped, against the port: `row` is the dump, eight hex digits to a word. Pointers and heap bytes are
    /// the run's, so a row masks them with `*`.
    fn assert_dump(row: &str, words: &[u32]) {
        assert_eq!(row.len(), words.len() * 8, "{row}");
        for (index, word) in words.iter().enumerate() {
            let at = index * 8;
            let ours = le(&[*word]);
            for (wanted, got) in row[at..at + 8].chars().zip(ours.chars()) {
                assert!(
                    wanted == '*' || wanted == got,
                    "word {index} of {row}: {ours}"
                );
            }
        }
    }

    /// The runs' dumps of the map, the PathBuilder and its thread.
    #[test]
    fn pathing_dumps_match_run_001() {
        let mut singletons = PathingSingletons::default();
        let mut platform = RecordedPathingPlatform::default();
        ensure_pathing_world_node_map(&mut singletons, &mut platform);
        ensure_path_builder(&mut singletons, &mut platform);

        let map_rows = rows("PATHMAP");
        assert_eq!(map_rows.len(), 2, "both runs");
        for row in map_rows {
            let map = singletons.world_node_map_b3be00.as_ref().unwrap();
            assert_dump(
                &row[2],
                &[map.original_vtable_00, map.bucket_count_04, 0, map.count_0c],
            );
        }

        let builder = singletons.path_builder_b3bf80.as_ref().unwrap();
        let builder_rows = rows("PATHBUILDER");
        assert_eq!(builder_rows.len(), 2, "both runs");
        for row in builder_rows {
            let mut words = vec![
                builder.base_00.original_vtable_00,
                u32::from(builder.base_00.unk_04),
                builder.base_00.unk_08,
                0,
            ];
            for map in &builder.actor_maps_10 {
                words.extend([map.original_vtable_00, map.bucket_count_04, 0, map.count_0c]);
            }
            words.extend([builder.unk_40, builder.unk_44]);
            assert_dump(&row[2], &words);
        }

        let thread = builder.base_00.thread_0c.as_ref().unwrap();
        let thread_rows = rows("PATHTHREAD");
        assert_eq!(thread_rows.len(), 1);
        let state = &thread.thread_00;
        assert_dump(
            &thread_rows[0][2],
            &[
                state.original_vtable_00,
                0,
                state.unk_08,
                state.unk_0c,
                state.unk_10,
                state.unk_14,
                u32::from(state.unk_18),
                0,
                thread.initial_a_20 as u32,
                thread.maximum_a_24 as u32,
                0,
                thread.initial_b_2c as u32,
                thread.maximum_b_30 as u32,
                0,
                thread.initial_c_38 as u32,
                thread.maximum_c_3c as u32,
                0,
            ],
        );
    }
}
