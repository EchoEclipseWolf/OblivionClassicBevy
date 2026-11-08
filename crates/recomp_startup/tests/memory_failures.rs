//! The C++ `startup_memory_failures`. Its fixture replaces `operator new` for the test executable and fails an ordered
//! sequence of allocations; a `#[global_allocator]` does the same here. Rust's infallible allocations abort on null, so
//! the sequence only reaches the sites that reserve with `try_reserve`, which are the sites with a null branch to test.
//!
//! The counters are per thread, like the C++ `thread_local`s, so the test harness's own threads are never affected.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::Path;

use bevy_ecs::prelude::*;
use recomp_game::bs::archive::{ArchiveLoadStatus, ArchiveManager, ArchiveSettings};
use recomp_game::bs::file_finder::{BsFileCallbacks, FileFinder};
use recomp_game::bs::texture_palette::BsTexturePalette;
use recomp_game::gamebryo::mesh_geometry::{MeshTriShapePayload, NiGeometry, spawn_mesh_geometry};
use recomp_game::gamebryo::object_net::{AlphaProperty, NiObjectNet};
use recomp_game::gamebryo::registration::{
    StartupCallback, StartupCallbackError, StartupCallbackScheduler, StartupCallbackScope,
    register_known_crt_callbacks,
};
use recomp_game::gamebryo::scene_state::{
    ScenePropertyDefaults, ScenePropertyState, attach_object_property,
    construct_scene_property_state, refresh_property_state, update_properties_downward,
};
use recomp_game::gamebryo::screen_element_palette::{
    SCREEN_ELEMENT_COUNT, UiScreenElementPalette, initialize_screen_element_palette,
    shutdown_screen_element_palette,
};
use recomp_game::gamebryo::screen_elements::ScreenElementsQuad;
use recomp_game::gamebryo::stream_objects::NiClass;
use recomp_startup::winmain_memory::{
    SoundCollisionListener, StartupAllocationSite, StartupMemoryState,
};

thread_local! {
    static INJECT_FAILURE: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS_BEFORE_FAILURE: Cell<usize> = const { Cell::new(0) };
    static SUCCESSES_BETWEEN_FAILURES: Cell<usize> = const { Cell::new(0) };
    static FAILURES_REMAINING: Cell<usize> = const { Cell::new(0) };
    static ALLOCATION_TO_WATCH: Cell<usize> = const { Cell::new(0) };
    static WATCHED_ALLOCATION_RELEASED: Cell<bool> = const { Cell::new(false) };
    static WATCHED_ALLOCATION_RELEASES: Cell<usize> = const { Cell::new(0) };
    static FAIL_ONLY_SIZE: Cell<usize> = const { Cell::new(0) };
}

/// IDA: N/A isolated host allocator fault fixture. Completion: 100% of the bounded fault sequence.
fn fail_after(successful_allocations: usize, failures: usize) {
    ALLOCATIONS_BEFORE_FAILURE.set(successful_allocations);
    SUCCESSES_BETWEEN_FAILURES.set(successful_allocations);
    FAILURES_REMAINING.set(failures);
    ALLOCATION_TO_WATCH.set(0);
    WATCHED_ALLOCATION_RELEASED.set(false);
    WATCHED_ALLOCATION_RELEASES.set(0);
    FAIL_ONLY_SIZE.set(0);
    INJECT_FAILURE.set(true);
}

/// IDA: N/A the fixture for a site behind other allocations, which the C++ fixture's first-allocation count reaches
/// directly: here the FileFinder lookup comes first in `open_archive`, so the fault selects the site by its size.
fn fail_size_once(bytes: usize) {
    fail_after(0, 1);
    FAIL_ONLY_SIZE.set(bytes);
}

fn inject_failure() -> bool {
    INJECT_FAILURE.get()
}

struct FaultAllocator;

// SAFETY: every path forwards to `System` with the caller's layout, or returns null, which `GlobalAlloc` allows.
unsafe impl GlobalAlloc for FaultAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // The thread's accessors are const-initialized with no destructor, so they neither allocate nor fail in
        // teardown.
        let only = FAIL_ONLY_SIZE.get();
        if INJECT_FAILURE.get()
            && ALLOCATIONS_BEFORE_FAILURE.get() == 0
            && (only == 0 || only == layout.size())
        {
            let remaining = FAILURES_REMAINING.get() - 1;
            FAILURES_REMAINING.set(remaining);
            INJECT_FAILURE.set(remaining != 0);
            ALLOCATIONS_BEFORE_FAILURE.set(SUCCESSES_BETWEEN_FAILURES.get());
            return std::ptr::null_mut();
        }
        // SAFETY: the caller's layout, unchanged.
        let result = unsafe { System.alloc(layout) };
        if INJECT_FAILURE.get() && ALLOCATIONS_BEFORE_FAILURE.get() != 0 {
            ALLOCATIONS_BEFORE_FAILURE.set(ALLOCATIONS_BEFORE_FAILURE.get() - 1);
            ALLOCATION_TO_WATCH.set(result as usize);
        }
        result
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if !ptr.is_null() && ptr as usize == ALLOCATION_TO_WATCH.get() {
            WATCHED_ALLOCATION_RELEASED.set(true);
            WATCHED_ALLOCATION_RELEASES.set(WATCHED_ALLOCATION_RELEASES.get() + 1);
        }
        // SAFETY: `ptr` came from `System.alloc` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: FaultAllocator = FaultAllocator;

/// IDA: 0040E647..0040E6B8 branch comparison. Completion: 100% of the host failure scenarios below.
/// TODO: [ ] Record original-runtime failed-allocation and teardown observations.
#[test]
fn startup_allocation_branches() {
    // IDA: 0040E658 SoundCollisionListener vtable A2FE0C and slot identities.
    assert_eq!(SoundCollisionListener::ORIGINAL_VTABLE, 0x00A2_FE0C);
    assert_eq!(SoundCollisionListener::ORIGINAL_SLOTS[0], 0x006B_4630);
    assert_eq!(SoundCollisionListener::ORIGINAL_SLOTS[4], 0x0040_5AA0);

    let path_text = "a".repeat(512);
    let path = Path::new(&path_text);
    let mut world = World::new();
    let mut callbacks = BsFileCallbacks::default();
    let mut memory = StartupMemoryState::default();
    memory.install_out_of_memory_policy();
    assert!(memory.allocate_listener());
    fail_after(0, 1);
    assert!(
        memory.allocate_listener() && !inject_failure() && memory.listener_allocated(),
        "the listener recovers after a first-attempt storage failure"
    );
    fail_after(0, 2);
    assert!(
        !memory.allocate_listener() && !inject_failure() && memory.listener_allocated(),
        "a persistent listener failure keeps the prior global owner"
    );
    memory.release_listener();
    assert!(!memory.listener_allocated());

    assert!(memory.allocate_texture_palette(&mut world));
    fail_after(0, 1);
    assert!(
        memory.allocate_texture_palette(&mut world)
            && !inject_failure()
            && world.get_resource::<BsTexturePalette>().is_some(),
        "the palette recovers after a first-attempt storage failure"
    );
    fail_after(0, 2);
    assert!(
        !memory.allocate_texture_palette(&mut world)
            && !inject_failure()
            && world.get_resource::<BsTexturePalette>().is_none(),
        "a persistent palette failure drops its old reference"
    );
    StartupMemoryState::release_texture_palette(&mut world);

    fail_after(0, 1);
    let recovered_storage = memory.allocate_file_finder(path, &mut callbacks);
    assert!(recovered_storage.is_some() && !inject_failure());
    fail_after(0, 2);
    let no_storage = memory.allocate_file_finder(path, &mut callbacks);
    assert!(no_storage.is_none() && !inject_failure());

    // The first allocation is the storage; the next copies the long path.
    fail_after(1, 1);
    let recovered_construction = memory.allocate_file_finder(path, &mut callbacks);
    assert!(
        recovered_construction.is_some()
            && !inject_failure()
            && ALLOCATION_TO_WATCH.get() != 0
            && WATCHED_ALLOCATION_RELEASED.get(),
        "a path-copy failure leaks no storage and blocks no retry"
    );
    fail_after(1, 2);
    let failed_construction = memory.allocate_file_finder(path, &mut callbacks);
    assert!(
        failed_construction.is_none()
            && !inject_failure()
            && WATCHED_ALLOCATION_RELEASES.get() == 2,
        "persistent constructor failures leak no object storage"
    );
    assert_eq!(memory.failed_allocations(), 4);
    assert_eq!(memory.recovery_attempts(), 8);
    assert_eq!(memory.recovered_allocations(), 4);

    let mut persistent = StartupMemoryState::failing_at(StartupAllocationSite::Listener);
    persistent.install_out_of_memory_policy();
    assert!(!persistent.allocate_listener());
    assert_eq!(persistent.failed_allocations(), 1);
    assert_eq!(persistent.recovery_attempts(), 1);
    assert_eq!(persistent.recovered_allocations(), 0);

    let recovered = memory.allocate_file_finder(path, &mut callbacks);
    assert!(
        recovered.is_some()
            && memory.allocate_listener()
            && memory.allocate_texture_palette(&mut world),
        "an allocation failure poisons no later construction"
    );
}

/// A fixed array, not a `Vec`: these cleanups run while an allocation failure is armed, as in the C++.
#[derive(Resource, Default)]
struct Order {
    entries: [i32; 3],
    next: usize,
}

fn nothing(_: &mut World) -> Result<(), StartupCallbackError> {
    Ok(())
}

fn record<const N: i32>(world: &mut World) -> Result<(), StartupCallbackError> {
    let mut order = world.resource_mut::<Order>();
    let next = order.next;
    order.entries[next] = N;
    order.next += 1;
    Ok(())
}

/// A failure in the public pending-ID report must occur only after all resource cleanups, in forward pair order then
/// singleton.
#[test]
fn pending_report_failure_comes_after_every_cleanup() {
    let mut world = World::new();
    world.init_resource::<Order>();
    let mut scheduler = StartupCallbackScheduler::default();
    scheduler.set_singleton(StartupCallback::new(1, 2, nothing, record::<3>).pending(true, false));
    scheduler
        .register_pair(StartupCallback::new(3, 4, nothing, record::<1>).pending(true, false))
        .unwrap();
    scheduler
        .register_pair(StartupCallback::new(5, 6, nothing, record::<2>).pending(true, false))
        .unwrap();
    scheduler.run(&mut world).unwrap();
    fail_after(0, 1);
    assert_eq!(
        scheduler.shutdown(&mut world),
        Err(StartupCallbackError("bad allocation"))
    );
    assert!(!inject_failure());
    assert_eq!(world.resource::<Order>().entries, [1, 2, 3]);
    assert!(
        scheduler.shutdown(&mut world).unwrap().is_empty(),
        "a failed report does not run the cleanups twice"
    );
    assert_eq!(world.resource::<Order>().next, 3);
}

/// The scope's exit allocates nothing, so an exhausted allocator interrupts no callback release.
#[test]
fn scope_exit_needs_no_allocation() {
    let mut world = World::new();
    world.init_resource::<Order>();
    {
        let mut scheduler = StartupCallbackScheduler::default();
        scheduler
            .register_pair(StartupCallback::new(1, 2, nothing, record::<1>).pending(true, false))
            .unwrap();
        scheduler
            .register_pair(StartupCallback::new(3, 4, nothing, record::<2>).pending(true, false))
            .unwrap();
        let mut scoped = StartupCallbackScope::new(&mut world, scheduler);
        scoped.run().unwrap();
        fail_after(0, 1);
    }
    let still_armed = inject_failure();
    INJECT_FAILURE.set(false);
    assert!(still_armed, "the scope's exit made no allocation at all");
    assert_eq!(world.resource::<Order>().next, 2);
}

#[derive(Resource, Default)]
struct PartialOwner(bool);

fn take_partial_then_fail(world: &mut World) -> Result<(), StartupCallbackError> {
    world.resource_mut::<PartialOwner>().0 = true;
    Err(StartupCallbackError("bad allocation"))
}

fn release_partial(world: &mut World) -> Result<(), StartupCallbackError> {
    world.resource_mut::<PartialOwner>().0 = false;
    Ok(())
}

/// An entered callback owns its partial state even if its initializer fails.
#[test]
fn failing_initializer_keeps_its_cleanup() {
    let mut world = World::new();
    world.init_resource::<PartialOwner>();
    let init_failed;
    {
        let mut partial = StartupCallbackScheduler::default();
        partial
            .register_pair(StartupCallback::new(
                1,
                2,
                take_partial_then_fail,
                release_partial,
            ))
            .unwrap();
        let mut scope = StartupCallbackScope::new(&mut world, partial);
        init_failed = scope.run().is_err();
    }
    assert!(init_failed && !world.resource::<PartialOwner>().0);
}

fn failing_cleanup(_: &mut World) -> Result<(), StartupCallbackError> {
    Err(StartupCallbackError("bad allocation"))
}

/// A failed cleanup must not suppress later cleanup or leave a retry armed.
#[test]
fn failing_cleanup_suppresses_nothing() {
    let mut world = World::new();
    world.init_resource::<Order>();
    let mut scheduler = StartupCallbackScheduler::default();
    scheduler
        .register_pair(StartupCallback::new(1, 2, nothing, failing_cleanup))
        .unwrap();
    scheduler
        .register_pair(StartupCallback::new(3, 4, nothing, record::<1>))
        .unwrap();
    scheduler.run(&mut world).unwrap();
    assert_eq!(
        scheduler.shutdown(&mut world),
        Err(StartupCallbackError("bad allocation"))
    );
    assert_eq!(world.resource::<Order>().next, 1);
    assert!(scheduler.shutdown(&mut world).unwrap().is_empty());
    assert_eq!(world.resource::<Order>().next, 1);
}

/// IDA: 00707854 FormHeap(0x30) in 007077D0 under 00722B30. The list copy and entity bookkeeping come first and are not
/// sites, so the fault selects the state's storage by its size, as with the archive below. A failed clone leaves the
/// installed owner.
#[test]
fn failed_property_state_clone_keeps_the_installed_owner() {
    let mut world = World::new();
    let alpha = world
        .spawn((
            NiClass::AlphaProperty,
            NiObjectNet::default(),
            AlphaProperty::default(),
        ))
        .id();
    let geometry = spawn_mesh_geometry(
        &mut world,
        NiClass::TriShape,
        MeshTriShapePayload::default(),
    );
    attach_object_property(&mut world, geometry, Some(alpha)).unwrap();
    let inherited = construct_scene_property_state(&mut world, &ScenePropertyDefaults::default());
    update_properties_downward(&mut world, geometry, Some(inherited)).unwrap();
    let old_state = world.get::<NiGeometry>(geometry).unwrap().property_state_ac;
    assert!(old_state.is_some() && old_state != Some(inherited));

    fail_size_once(0x30);
    let clone_failed = update_properties_downward(&mut world, geometry, Some(inherited)).is_err();
    assert!(clone_failed && !inject_failure());
    assert_eq!(
        world.get::<NiGeometry>(geometry).unwrap().property_state_ac,
        old_state
    );
    assert_eq!(
        world
            .get::<ScenePropertyState>(old_state.unwrap())
            .unwrap()
            .slots[0],
        Some(alpha)
    );

    // The same failure under 00707610: its temporary default state does not stay behind.
    let states = world.query::<&ScenePropertyState>().iter(&world).count();
    fail_size_once(0x30);
    let refresh_failed =
        refresh_property_state(&mut world, geometry, &ScenePropertyDefaults::default()).is_err();
    assert!(refresh_failed && !inject_failure());
    assert_eq!(
        world.query::<&ScenePropertyState>().iter(&world).count(),
        states
    );
}

fn palette_entries(world: &World) -> usize {
    world
        .resource::<UiScreenElementPalette>()
        .entries_b42cf8
        .iter()
        .flatten()
        .count()
}

/// IDA: 007AB1D0 host-only allocation failure on reinitialization: the prior 17 owners are gone before a failed retry, a
/// later one succeeds and shutdown clears it. The first site is the 0x1C vertex color, selected by size because the
/// release before it is entity bookkeeping. The later sizes check the same rollback from inside the loop, which the C++
/// states for "any throwing entry prefix" and tests only at the first. The 0x30 state copy is not among them: the quad's
/// 48-byte vertex array comes first and is infallible, so that site is checked on plain geometry above.
/// TODO: [ ] Validate original partial geometry failure under a safe runtime.
#[test]
fn failed_palette_reinitialization_keeps_no_owner() {
    let mut world = World::new();
    let mut known = StartupCallbackScheduler::default();
    register_known_crt_callbacks(&mut known).unwrap();
    known.run(&mut world).unwrap();
    initialize_screen_element_palette(&mut world).unwrap();
    assert_eq!(palette_entries(&world), SCREEN_ELEMENT_COUNT);

    for bytes in [0x1C, 0xC0, 0x70, 0x24, 0x5C] {
        fail_size_once(bytes);
        let failed = initialize_screen_element_palette(&mut world).is_err();
        assert!(failed && !inject_failure(), "site {bytes:#X}");
        assert_eq!(palette_entries(&world), 0, "site {bytes:#X}");
        assert_eq!(
            world.query::<&ScreenElementsQuad>().iter(&world).count(),
            0,
            "site {bytes:#X} left a geometry"
        );
        assert_eq!(
            world.query::<&ScenePropertyState>().iter(&world).count(),
            0,
            "site {bytes:#X} left a state"
        );
        initialize_screen_element_palette(&mut world).unwrap();
        assert_eq!(palette_entries(&world), SCREEN_ELEMENT_COUNT);
    }
    shutdown_screen_element_palette(&mut world);
    assert_eq!(palette_entries(&world), 0);
    known.shutdown(&mut world).unwrap();
}

/// IDA: 0042F538 FormHeap(0x280) failure -> 0042F57F OpenArchive returns null, and a later open is not poisoned.
#[test]
fn archive_allocation_failure_returns_null() {
    let root = std::env::temp_dir().join(format!(
        "oblivion_recomp_archive_oom_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Data")).unwrap();
    std::fs::write(root.join("Data").join("Empty.bsa"), b"").unwrap();
    let mut callbacks = BsFileCallbacks::default();
    let mut finder = FileFinder::new(&root, &mut callbacks);
    finder.add("Data");
    let mut manager = ArchiveManager::new(&root, ArchiveSettings::default());
    fail_size_once(0x280);
    let failed = manager.open_archive("Empty.bsa", 0, &finder);
    assert!(failed.is_none() && !inject_failure());
    assert!(manager.archives().is_empty());
    let retried = manager.open_archive("Empty.bsa", 0, &finder).unwrap();
    assert_eq!(
        manager.archives()[retried].status(),
        ArchiveLoadStatus::Loaded
    );
    drop(manager);
    std::fs::remove_dir_all(&root).unwrap();
}
