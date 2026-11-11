//! WinMain 0040E3D0..0040E6B8: the allocation and global-state prefix. Host allocation stands in for the original's
//! pools; the allocations that can fail keep their null branches.

use std::path::Path;
use std::path::PathBuf;

use crate::bs::file_finder::{BsFileCallbacks, FileFinder};
use crate::bs::texture_palette::BsTexturePalette;
use bevy_ecs::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedPoolRegistration {
    pub call_site: u32,
    pub block_size: u32,
    pub reserve_bytes: u32,
    pub name: &'static str,
}

// IDA: N/A a helper, with no address of its own.
const fn pool(
    call_site: u32,
    block_size: u32,
    reserve_bytes: u32,
    name: &'static str,
) -> FixedPoolRegistration {
    FixedPoolRegistration {
        call_site,
        block_size,
        reserve_bytes,
        name,
    }
}

/// IDA: 0040E3D0..0040E61B -> 00401F40, which aligns blocks and reserves and commits named pool pages. The host
/// allocator supplies generic storage instead; these exact arguments are evidence, not working pool storage.
const ORIGINAL_POOLS: [FixedPoolRegistration; 27] = [
    pool(0x0040_E3D0, 8, 0x00C0_0000, "Default Pool 8"),
    pool(0x0040_E3E6, 12, 0x0080_0000, "Default Pool 12"),
    pool(0x0040_E3FC, 16, 0x0040_0000, "Default Pool 16"),
    pool(0x0040_E412, 20, 0x0040_0000, "Default Pool 20"),
    pool(0x0040_E428, 24, 0x0080_0000, "Default Pool 24"),
    pool(0x0040_E43E, 28, 0x0040_0000, "Default Pool 28"),
    pool(0x0040_E454, 32, 0x0080_0000, "Default Pool 32"),
    pool(0x0040_E46A, 36, 0x0040_0000, "Default Pool 36"),
    pool(0x0040_E480, 40, 0x0040_0000, "Default Pool 40"),
    pool(0x0040_E496, 44, 0x0040_0000, "Default Pool 44"),
    pool(0x0040_E4AC, 56, 0x0200_0000, "Default Pool 56"),
    pool(0x0040_E4C2, 68, 0x0040_0000, "Default Pool 68"),
    pool(0x0040_E4D8, 72, 0x0080_0000, "Default Pool 72"),
    pool(0x0040_E4EE, 96, 0x0080_0000, "Default Pool 96"),
    pool(0x0040_E504, 100, 0x0080_0000, "Default Pool 100"),
    pool(0x0040_E51A, 48, 0x0020_0000, "Default Pool 48"),
    pool(0x0040_E530, 52, 0x0040_0000, "Default Pool 52"),
    pool(0x0040_E546, 64, 0x0040_0000, "Default Pool 64"),
    pool(0x0040_E55C, 80, 0x0040_0000, "Default Pool 80"),
    pool(0x0040_E572, 92, 0x0040_0000, "Default Pool 92"),
    pool(0x0040_E588, 108, 0x0040_0000, "Default Pool 108"),
    pool(0x0040_E59E, 120, 0x0040_0000, "Default Pool 120"),
    pool(0x0040_E5B7, 200, 0x0080_0000, "Default Pool 200"),
    pool(0x0040_E5D0, 216, 0x0040_0000, "NiGeometry Pool"),
    pool(0x0040_E5E9, 244, 0x0040_0000, "NiNode Pool"),
    pool(0x0040_E602, 256, 0x0080_0000, "Default Pool 256"),
    pool(0x0040_E61B, 264, 0x0040_0000, "BSFadeNode Pool"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupAllocationSite {
    Listener,
    FileFinder,
    TexturePalette,
}

/// IDA: SoundCollisionListener, an hkCollisionListener subclass, 4 bytes in the original: only the vtable pointer.
/// 0040E658 stores vtable A2FE0C: +0x00 006B4630 contact callback (impact sounds; needs Havok contacts), +0x04 006E7260
/// and +0x08/+0x0C 0068F970 no-op callbacks, +0x10 00405AA0 deleting destructor (clears 00BA7A00, restores the
/// hkCollisionListener vtable, frees when bit 0 is set). Its only consumer, 00889A70, registers it with the Havok world
/// during TES construction (a later phase; seq 35 in P03-RUN-005); the contact callback did not run before first idle
/// in that trace. Completion: 100% of the identity and lifetime; the callbacks belong to the TES and Havok phases.
/// TODO: [ ] Port 006B4630 with the Havok contact owners (later phase).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SoundCollisionListener;

impl SoundCollisionListener {
    pub const ORIGINAL_VTABLE: u32 = 0x00A2_FE0C;
    pub const ORIGINAL_SLOTS: [u32; 5] = [
        0x006B_4630,
        0x006E_7260,
        0x0068_F970,
        0x0068_F970,
        0x0040_5AA0,
    ];
}

/// IDA: N/A the FormHeap request at an allocation site, at its original size. Rust's own objects allocate their parts
/// infallibly and a zero-sized owner allocates nothing, so this reservation is what can fail and what the null branch
/// tests. Held across the construction, as the original's storage is. Completion: 100% of the host's branch.
fn form_heap_storage(bytes: usize) -> Option<Vec<u8>> {
    let mut storage = Vec::new();
    storage.try_reserve_exact(bytes).ok()?;
    Some(storage)
}

/// IDA: 0040E3D0..0040E6B8 WinMain allocation and global-state prefix. A scoped host allocation and order model;
/// original pool storage, callback passes, listener behavior and texture-map construction remain separate work. The
/// palette's 00B35300 reference and 00B3FAC8 raw alias are one thing here: the `BsTexturePalette` resource.
/// TODO: [ ] Connect recovered game cache-purge passes when their owners exist.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StartupMemoryState {
    fail_site: Option<StartupAllocationSite>,
    listener_ba7a00: Option<SoundCollisionListener>,
    pool_count: usize,
    failed_allocations: u32,
    recovery_attempts: u32,
    recovered_allocations: u32,
    recovery_installed: bool,
    object_callbacks_enabled_b3f958: bool,
}

impl StartupMemoryState {
    /// IDA: N/A host-only failure injection for branch verification. Completion: 100% of the selected site.
    /// TODO: [ ] Compare each injected branch with original runtime evidence.
    #[must_use]
    pub fn failing_at(site: StartupAllocationSite) -> Self {
        Self {
            fail_site: Some(site),
            ..Self::default()
        }
    }

    /// IDA: 0040E3D0..0040E61B -> 00401F40. Evidence of 27 original pool arguments, not an implementation of the pools.
    #[must_use]
    pub const fn fixed_pools() -> &'static [FixedPoolRegistration] {
        &ORIGINAL_POOLS
    }

    /// IDA: 0040E3D0..0040E61B -> 00401F40: the host records all 27 pool classes. Completion: 100% of the ordered
    /// metadata; generic allocation is host-owned.
    pub const fn register_fixed_pools(&mut self) {
        self.pool_count = ORIGINAL_POOLS.len();
    }

    /// IDA: 0040E620 stores 00405D60 in g_formHeap.outOfMemoryCallback. The three nullable host allocations recover and
    /// retry once; other required failures reach the startup error path. Completion: 100% of this standalone retry and
    /// terminal-error substitution, not original memory-pressure availability.
    /// TODO: [ ] Integrate recovery when later game-cache producers are supported.
    pub const fn install_out_of_memory_policy(&mut self) {
        self.recovery_installed = true;
    }

    /// IDA: 0040E62F writes g_objectLoadCallbacksEnabled=1 after 0040E62A. Completion: 100% of the startup transition.
    /// TODO: [ ] Later phases: the 00701C00 texture loads.
    pub const fn initializers_dispatched(&mut self) {
        self.object_callbacks_enabled_b3f958 = true;
    }

    /// IDA: 0040E647/0040E651/0040E653 allocate a 4-byte listener, assign 00BA7A00 and store vtable A2FE0C (0040E658),
    /// all skipped on failure: 0040E651 skips the global assignment, so an existing owner stays. Completion: 100% of
    /// the identity and nullable ownership.
    /// TODO: [ ] Port the contact callback with the TES and Havok phase.
    pub fn allocate_listener(&mut self) -> bool {
        for attempt in 0..2 {
            let storage = if self.should_fail(StartupAllocationSite::Listener) {
                None
            } else {
                form_heap_storage(4)
            };
            if storage.is_some() {
                self.listener_ba7a00 = Some(SoundCollisionListener);
                if attempt != 0 {
                    self.recovered_allocations += 1;
                }
                return true;
            }
            if attempt == 0 && self.recovery_installed {
                self.attempt_host_recovery();
            } else {
                break;
            }
        }
        self.record_failure();
        false
    }

    /// IDA: 0040F511 -> 004466F0 -> 008897A0 -> vtable +0x10 -> 00405AA0. TES teardown clears the global and deletes
    /// the listener before FileFinder. Completion: 100%.
    pub const fn release_listener(&mut self) {
        self.listener_ba7a00 = None;
    }

    /// IDA: 0040E660/0040E66A/0040E66E allocate the 0x14-byte FileFinder conditionally. Failure skips 004316E0, and the
    /// following g_FileFinder test skips AddSearchPath, so a failed attempt installs no callback. The second step of a
    /// host attempt copies the path, as the C++ port's by-value constructor argument did; its failure frees the storage
    /// and takes the same retry. The caller makes the result the `FileFinder` resource. Completion: 100% of the
    /// nullable host construction.
    /// TODO: [ ] Complete 03-D callback and packed-file behavior.
    pub fn allocate_file_finder(
        &mut self,
        data_parent: &Path,
        callbacks: &mut BsFileCallbacks,
    ) -> Option<FileFinder> {
        for attempt in 0..2 {
            let storage = if self.should_fail(StartupAllocationSite::FileFinder) {
                None
            } else {
                form_heap_storage(0x14)
            };
            let result = storage.and_then(|storage| {
                let mut copy = PathBuf::new();
                copy.try_reserve_exact(data_parent.as_os_str().len()).ok()?;
                copy.push(data_parent);
                let finder = FileFinder::new(copy, callbacks);
                drop(storage);
                Some(finder)
            });
            if result.is_some() {
                if attempt != 0 {
                    self.recovered_allocations += 1;
                }
                return result;
            }
            if attempt == 0 && self.recovery_installed {
                self.attempt_host_recovery();
            } else {
                break;
            }
        }
        self.record_failure();
        None
    }

    /// IDA: 0040E68B/0040E695/0040E6A5/0040E6AD/0040E6B8. The original passes null through NiRefObjectPtr_Assign when
    /// the 0x10 allocation fails, then mirrors the resulting reference to a raw alias: unlike the listener branch, a
    /// null result drops the old reference. The object is the 03-E BSTexturePalette (004A24F0, 1009/523 maps); its own
    /// map allocations are host-owned and abort on failure (the original stores null maps or crashes in memset).
    /// Completion: 100% of the nullable assignment.
    pub fn allocate_texture_palette(&mut self, world: &mut World) -> bool {
        let mut replacement = None;
        for attempt in 0..2 {
            let storage = if self.should_fail(StartupAllocationSite::TexturePalette) {
                None
            } else {
                form_heap_storage(0x10)
            };
            if storage.is_some() {
                replacement = Some(BsTexturePalette::default());
                if attempt != 0 {
                    self.recovered_allocations += 1;
                }
                break;
            }
            if attempt == 0 && self.recovery_installed {
                self.attempt_host_recovery();
            } else {
                break;
            }
        }
        world.remove_resource::<BsTexturePalette>();
        let allocated = replacement.is_some();
        if let Some(palette) = replacement {
            world.insert_resource(palette);
        } else {
            self.record_failure();
        }
        allocated
    }

    /// IDA: 0040F5EC..0040F612 drops the palette reference before callback cleanup. Releasing the last reference
    /// destroys both maps and their texture references. Completion: 100% of the host release; the original map
    /// destructor is 03-E.
    pub fn release_texture_palette(world: &mut World) {
        world.remove_resource::<BsTexturePalette>();
    }

    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn registered_pool_count(&self) -> usize {
        self.pool_count
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn recovery_policy_installed(&self) -> bool {
        self.recovery_installed
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn object_load_callbacks_enabled(&self) -> bool {
        self.object_callbacks_enabled_b3f958
    }
    /// IDA: 00441D1B TES_Constructor stores 00B3F958 = 1 unless bTintMipMaps (Phase 09; already 1 from 0040E62F on the
    /// default path). Completion: 100%.
    pub const fn enable_object_load_callbacks(&mut self) {
        self.object_callbacks_enabled_b3f958 = true;
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn listener(&self) -> Option<&SoundCollisionListener> {
        self.listener_ba7a00.as_ref()
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn listener_allocated(&self) -> bool {
        self.listener_ba7a00.is_some()
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn failed_allocations(&self) -> u32 {
        self.failed_allocations
    }
    /// IDA: N/A host-only bounded recovery and retry accounting, kept distinct from 00405D60's game-cache passes.
    #[must_use]
    pub const fn recovery_attempts(&self) -> u32 {
        self.recovery_attempts
    }
    /// IDA: N/A an accessor.
    #[must_use]
    pub const fn recovered_allocations(&self) -> u32 {
        self.recovered_allocations
    }

    /// IDA: N/A host-only deterministic failure injection. Completion: 100% of the selected site.
    fn should_fail(&self, site: StartupAllocationSite) -> bool {
        matches!(self.fail_site, Some(selected) if selected == site)
    }

    /// IDA: N/A host-only safe point before one retry. Host substitution: the C++ port calls `_heapmin` here, which has
    /// no portable Rust or Linux counterpart, so only the count remains. Neither one can purge live game caches or
    /// emulate any 00405D60 pass.
    /// TODO: [ ] Reconcile original 00405D60 when game-cache owners are ported.
    const fn attempt_host_recovery(&mut self) {
        self.recovery_attempts += 1;
    }

    /// IDA: N/A the original 00405D60 may run TES, model and texture purge passes; those owners do not exist at this
    /// startup boundary in the host. Counts one terminal failed logical allocation after the bounded retry. Completion:
    /// 100%, no allocation.
    const fn record_failure(&mut self) {
        self.failed_allocations += 1;
    }
}

#[cfg(test)]
mod tests {
    use crate::tes::winmain::winmain_memory::*;

    /// `verify_startup_memory` in the C++ `startup_scaffold`.
    #[test]
    fn ordering_null_branches_and_ownership() {
        let pools = StartupMemoryState::fixed_pools();
        assert_eq!(pools.len(), 27);
        assert_eq!(pools[0].call_site, 0x0040_E3D0);
        assert_eq!(pools[0].block_size, 8);
        assert_eq!(pools[0].reserve_bytes, 0x00C0_0000);
        assert_eq!(pools[10].call_site, 0x0040_E4AC);
        assert_eq!(pools[10].block_size, 56);
        assert_eq!(pools[10].reserve_bytes, 0x0200_0000);
        assert_eq!(pools[26].call_site, 0x0040_E61B);
        assert_eq!(pools[26].block_size, 264);
        assert_eq!(pools[26].reserve_bytes, 0x0040_0000);

        let mut world = World::new();
        let mut callbacks = BsFileCallbacks::default();
        let temp = std::env::temp_dir();
        let mut ordinary = StartupMemoryState::default();
        assert!(!ordinary.object_load_callbacks_enabled() && !ordinary.recovery_policy_installed());
        ordinary.register_fixed_pools();
        ordinary.install_out_of_memory_policy();
        assert_eq!(ordinary.registered_pool_count(), pools.len());
        assert!(ordinary.recovery_policy_installed() && !ordinary.object_load_callbacks_enabled());
        ordinary.initializers_dispatched();
        assert!(ordinary.object_load_callbacks_enabled());
        assert!(ordinary.allocate_listener());
        let finder = ordinary.allocate_file_finder(&temp, &mut callbacks);
        assert!(finder.is_some());
        assert_eq!(callbacks.open, BsFileCallbacks::FILE_FINDER_OPEN);
        assert!(ordinary.allocate_texture_palette(&mut world));
        // 0040E8D4 copies this reference into 00B42EBC (phase 05).
        assert!(world.get_resource::<BsTexturePalette>().is_some());
        ordinary.release_listener();
        assert!(!ordinary.listener_allocated());
        drop(finder);
        StartupMemoryState::release_texture_palette(&mut world);
        assert!(world.get_resource::<BsTexturePalette>().is_none());

        let mut no_listener = StartupMemoryState::failing_at(StartupAllocationSite::Listener);
        assert!(!no_listener.allocate_listener());
        assert!(!no_listener.listener_allocated());
        assert_eq!(no_listener.failed_allocations(), 1);

        let mut no_finder = StartupMemoryState::failing_at(StartupAllocationSite::FileFinder);
        let mut untouched = BsFileCallbacks::default();
        let before = untouched.open;
        assert!(
            no_finder
                .allocate_file_finder(&temp, &mut untouched)
                .is_none()
        );
        assert_eq!(no_finder.failed_allocations(), 1);
        assert_eq!(untouched.open, before, "a failed attempt skips 004316E0");

        let mut no_palette = StartupMemoryState::failing_at(StartupAllocationSite::TexturePalette);
        assert!(!no_palette.allocate_texture_palette(&mut world));
        assert!(world.get_resource::<BsTexturePalette>().is_none());
        assert_eq!(no_palette.failed_allocations(), 1);
    }
}
