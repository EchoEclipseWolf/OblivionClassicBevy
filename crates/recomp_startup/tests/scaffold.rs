//! Ported from the C++ `startup_scaffold` test's `verify_manifest`: evidence integrity, with no runtime bonus. The other
//! parts of that test are with what they check: registration and the palette in `recomp_game`'s `registration` and
//! `screen_element_palette` modules, the processor in `processor`, memory in `winmain_memory`, paths in
//! `scaffold_paths.rs`, assets in `scaffold_assets.rs`. The window branch belongs to the Phase 04 window work (M2).
//!
//! The expectations about implemented phases are about the C++ port's record, as `recomp_startup::scaffold` explains.

use std::collections::{BTreeMap, BTreeSet};

use recomp_startup::scaffold::{
    CallSite, Resolution as R, ScopeDisposition as D, dependencies, find, functions,
};

fn window(low: u32, high: u32) -> impl Iterator<Item = &'static CallSite> {
    functions()
        .iter()
        .flat_map(|function| function.calls)
        .filter(move |call| call.site >= low && call.site <= high)
}

const fn expected_resolution_for(disposition: D) -> R {
    match disposition {
        D::ObsoleteRuntime => R::ObsoleteRuntime,
        D::HostSupplied => R::HostSupplied,
        D::ImplementedBehavior => R::ImplementedLocal,
        D::Unresolved => R::UnresolvedIndirect,
        D::Pending => R::PendingBody,
    }
}

const fn settled(disposition: D) -> bool {
    matches!(
        disposition,
        D::HostSupplied | D::ImplementedBehavior | D::ObsoleteRuntime
    )
}

fn implemented(call: &CallSite) -> bool {
    call.disposition == D::ImplementedBehavior && call.stub.resolution == R::ImplementedLocal
}

fn resolutions() -> BTreeMap<u32, R> {
    dependencies()
        .iter()
        .map(|dependency| (dependency.identity, dependency.stub.resolution))
        .collect()
}

#[test]
fn the_graph_keeps_its_invariants() {
    let mut seen = BTreeSet::new();
    let mut expected = BTreeMap::new();
    for dependency in dependencies() {
        assert!(seen.insert(dependency.identity), "Duplicate dependency");
        assert_eq!(
            dependency.stub.identity, dependency.identity,
            "Stub identity mismatch"
        );
        let resolution = expected_resolution_for(dependency.disposition);
        assert_eq!(
            dependency.stub.resolution, resolution,
            "Scope disposition/stub mismatch"
        );
        expected.insert(dependency.identity, resolution);
        assert_eq!(
            dependency.body_captured,
            find(dependency.identity).is_some(),
            "Body capture flag mismatch"
        );
    }
    let mut sites = BTreeSet::new();
    let mut entries = BTreeSet::new();
    let mut indirect_count = 0;
    for function in functions() {
        assert!(entries.insert(function.entry), "Duplicate function");
        assert!(seen.contains(&function.entry), "Owner has no stub");
        let blocks: BTreeSet<u32> = function.blocks.iter().map(|block| block.start).collect();
        let mut previous = 0;
        for call in function.calls {
            assert!(call.site > previous, "Callsites not in instruction order");
            previous = call.site;
            assert!(sites.insert(call.site), "Duplicate callsite");
            let identity = if call.target == 0 {
                call.site
            } else {
                call.target
            };
            assert!(seen.contains(&identity), "Call has no dependency stub");
            assert_eq!(
                call.stub.identity, identity,
                "Call dispatch identity mismatch"
            );
            assert_eq!(
                call.stub.resolution, expected[&identity],
                "Call dispatch differs from dependency disposition"
            );
            indirect_count += usize::from(call.target == 0);
            let owners = function
                .blocks
                .iter()
                .filter(|block| block.start <= call.site && call.site < block.end)
                .count();
            assert_eq!(owners, 1, "Callsite needs exactly one basic block");
        }
        for block in function.blocks {
            assert!(block.start <= block.end, "Invalid block range");
            for successor in block.successors {
                assert!(
                    blocks.contains(successor) || seen.contains(successor),
                    "Lost branch/tail destination"
                );
            }
        }
    }
    assert_eq!(
        (entries.len(), sites.len(), indirect_count, seen.len()),
        (47, 1224, 123, 597),
        "Preserved 2026-09-24 capture changed; re-audit evidence before changing baseline"
    );
    assert!(find(0).is_none(), "Unknown function accepted");
}

#[test]
fn the_entry_and_winmain_keep_their_order() {
    let expected = resolutions();
    let entry = find(0x98_787C).expect("PE entry missing");
    assert!(
        entry
            .blocks
            .iter()
            .flat_map(|block| block.successors)
            .any(|&successor| successor == 0x98_769C),
        "CRT tail transfer lost"
    );
    for (identity, resolution) in [
        (0x98_787C, R::ObsoleteRuntime),
        (0x99_7BE3, R::HostSupplied),
        (0x98_769C, R::ObsoleteRuntime),
        (0x98_1C6B, R::HostSupplied),
        (0x40_4940, R::HostSupplied),
    ] {
        assert_eq!(
            expected[&identity], resolution,
            "CRT boundary disposition lost"
        );
    }
    let main = find(0x40_DF80).expect("WinMain missing");
    let at = |site: u32| {
        main.calls
            .iter()
            .find(|call| call.site == site)
            .expect("a WinMain callsite")
    };
    for site in [
        0x40_DFCE, 0x40_E39B, 0x40_E8ED, 0x40_EA6E, 0x40_EAAC, 0x40_EAC1,
    ] {
        assert_eq!(
            at(site).disposition,
            D::ImplementedBehavior,
            "WinMain path, shader setting or menu order changed"
        );
    }
    for site in [0x40_E647, 0x40_E660, 0x40_E68B] {
        assert_eq!(
            at(site).disposition,
            D::HostSupplied,
            "WinMain startup allocations changed"
        );
    }
    assert_eq!(
        at(0x40_F088).target,
        0x44_0AF0,
        "WinMain menu order changed"
    );
    assert_eq!(
        at(0x40_F08D).target,
        0x5B_6040,
        "WinMain menu order changed"
    );
    assert_eq!(
        expected[&0x40_1F00],
        R::PendingBody,
        "00401F00 stays a shared pending dependency"
    );
    for identity in [0x74_7BA0, 0x6F_98F0] {
        assert_eq!(
            expected[&identity],
            R::ImplementedLocal,
            "WinMain local registration implementations lost"
        );
    }
    for identity in [
        0x40_E687, 0x7B_4870, 0x7B_4830, 0x40_6950, 0x40_3160, 0x43_16E0, 0x4A_24F0, 0x75_FA90,
        0x53_AC60, 0x42_F610,
    ] {
        assert_eq!(
            expected[&identity],
            R::ImplementedLocal,
            "Phase 03 FileFinder, palette, BlendSettings and archive targets changed"
        );
    }
}

#[test]
fn phases_00_to_05_report_nothing_pending() {
    let expected = resolutions();
    // Phases 00-02: no CRT entry callsite and no WinMain callsite before 0040E3D0 may report a pending stub.
    for function in functions() {
        let crt_entry = function.entry == 0x98_787C || function.entry == 0x98_769C;
        if !crt_entry && function.entry != 0x40_DF80 {
            continue;
        }
        for call in function.calls {
            if !crt_entry && call.site >= 0x40_E3D0 {
                continue;
            }
            assert!(
                settled(call.disposition),
                "A phase 00-02 callsite still reports pending"
            );
        }
    }
    // Phase 03: the callsite disposition decides; 00401F00 stays a shared pending dependency for later callers.
    for call in window(0x40_E3D0, 0x40_E75E) {
        assert!(
            matches!(call.disposition, D::HostSupplied | D::ImplementedBehavior),
            "A phase-03 window callsite still reports pending"
        );
    }
    for identity in [
        0x40_4A00, 0x57_2DC0, 0x40_52F0, 0x40_C280, 0x40_C1B0, 0x55_3550,
    ] {
        assert_eq!(
            expected[&identity],
            R::ImplementedLocal,
            "Phase 04 window targets changed"
        );
    }
    for call in window(0x40_E763, 0x40_E893) {
        assert!(
            settled(call.disposition),
            "A phase-04 window callsite still reports pending"
        );
    }
    let mut phase05_sites = 0;
    for call in window(0x40_E898, 0x40_EAC1) {
        phase05_sites += 1;
        assert!(
            settled(call.disposition),
            "A phase-05 window callsite still reports pending"
        );
        // The obsolete 0040E89D log call shares 00404EE0's stub with other callers.
        if call.disposition == D::ImplementedBehavior {
            assert_eq!(
                call.stub.resolution,
                R::ImplementedLocal,
                "A phase-05 window callsite still resolves as pending"
            );
        }
    }
    assert_eq!(phase05_sites, 5, "The phase-05 window holds five callsites");
}

#[test]
fn phases_06_to_11_keep_their_windows() {
    let phase06: Vec<(u32, u32)> = window(0x40_EAC6, 0x40_EB11)
        .map(|call| (call.site, call.target))
        .collect();
    assert_eq!(
        phase06,
        [
            (0x40_EAC8, 0x40_46A0),
            (0x40_EAD2, 0x47_D0F0),
            (0x40_EADB, 0x41_0310),
            (0x40_EAEA, 0x41_0E40),
            (0x40_EB0C, 0x67_FCF0),
            (0x40_EB11, 0x68_3030)
        ],
        "The phase-06 window callsites or targets changed"
    );
    assert!(
        window(0x40_EAC6, 0x40_EB11).all(implemented),
        "A phase-06 window callsite still reports pending"
    );

    let phase07: Vec<&CallSite> = window(0x40_EB16, 0x40_EB2B).collect();
    assert_eq!(phase07.len(), 1, "The phase-07 window holds one callsite");
    assert_eq!(
        (phase07[0].site, phase07[0].target),
        (0x40_EB18, 0x57_8CC0),
        "The phase-07 window callsite or target changed"
    );
    assert!(
        implemented(phase07[0]),
        "The phase-07 window callsite still reports pending"
    );

    assert!(
        window(0x40_EB2C, 0x40_EB2C).all(implemented),
        "0040EB2C -> 00406360 is implemented (Phases 08-13)"
    );
    let mut phase08_sites = 0;
    for call in window(0x40_6390, 0x40_65FF) {
        phase08_sites += 1;
        if call.site == 0x40_6390 || call.site == 0x40_65FF {
            assert_eq!(
                (call.target, call.disposition),
                (0x40_4EE0, D::ObsoleteRuntime),
                "The phase-08 log calls are obsolete_runtime"
            );
            continue;
        }
        assert!(
            implemented(call),
            "A phase-08 window callsite still reports pending"
        );
    }
    assert_eq!(phase08_sites, 33, "The phase-08 window holds 33 callsites");

    let mut phase09_sites = 0;
    for call in window(0x40_6604, 0x40_6686) {
        phase09_sites += 1;
        match call.site {
            0x40_6609 => assert_eq!(
                (call.target, call.disposition),
                (0x40_1F00, D::HostSupplied),
                "00406609 FormHeap_Allocate is host_supplied"
            ),
            0x40_664E | 0x40_665E => assert_eq!(
                (call.target, call.disposition),
                (0x40_4EE0, D::ObsoleteRuntime),
                "The phase-09 log calls are obsolete_runtime"
            ),
            _ => assert!(
                implemented(call),
                "A phase-09 window callsite still reports pending"
            ),
        }
    }
    assert_eq!(phase09_sites, 9, "The phase-09 window holds nine callsites");
    let tes_constructor = find(0x44_1970).expect("TES_Constructor");
    assert_eq!(
        tes_constructor.calls.len(),
        35,
        "TES_Constructor's captured body holds 35 callsites"
    );
    assert!(
        tes_constructor
            .calls
            .iter()
            .all(|call| matches!(call.disposition, D::ImplementedBehavior | D::HostSupplied)),
        "A TES_Constructor callsite still reports pending"
    );

    let phase10: Vec<&CallSite> = window(0x40_6687, 0x40_6773).collect();
    assert_eq!(phase10.len(), 5, "The phase-10 window holds five callsites");
    assert!(
        phase10.iter().all(|call| implemented(call)),
        "A phase-10 window callsite still reports pending"
    );
    let lookup = find(0x44_7C50).expect("00447C50");
    assert_eq!(lookup.calls.len(), 1);
    assert_eq!(
        lookup.calls[0].disposition,
        D::HostSupplied,
        "00447C50's _stricmp call is host_supplied"
    );
    assert_eq!(
        lookup.calls[0].stub.resolution,
        R::HostSupplied,
        "00447C50's _stricmp call is host_supplied"
    );

    let mut phase11_sites = 0;
    for call in window(0x40_6774, 0x40_67B9) {
        phase11_sites += 1;
        match call.site {
            0x40_6779 => assert_eq!(
                (call.target, call.disposition),
                (0x40_1F00, D::HostSupplied),
                "00406779 FormHeap_Allocate is host_supplied"
            ),
            0x40_67B5 => assert_eq!(
                (call.target, call.disposition),
                (0x40_4EE0, D::ObsoleteRuntime),
                "The phase-11 log call is obsolete_runtime"
            ),
            _ => assert!(
                implemented(call),
                "A phase-11 window callsite still reports pending"
            ),
        }
    }
    assert_eq!(phase11_sites, 4, "The phase-11 window holds four callsites");
}

#[test]
fn phases_12_and_13_keep_their_windows() {
    let game_data = find(0x40_6360).expect("00406360");
    let in_body = |low: u32, high: u32| {
        game_data
            .calls
            .iter()
            .filter(move |call| call.site >= low && call.site <= high)
    };
    assert!(
        in_body(0x40_67C7, 0x40_67C7).all(implemented),
        "004067C7 DataHandler_LoadSelectedFiles is implemented (Phase 12)"
    );
    assert_eq!(in_body(0x40_67C7, 0x40_67C7).count(), 1);

    let load = find(0x44_F3D0).expect("0044F3D0");
    assert_eq!(load.calls.len(), 59, "0044F3D0 holds 59 body callsites");
    let mut logs = 0;
    for call in load.calls {
        if call.target == 0x4A_7A60 || call.target == 0x40_4EC0 {
            assert_eq!(
                call.disposition,
                D::ObsoleteRuntime,
                "A 0044F3D0 log callsite is obsolete_runtime"
            );
            logs += 1;
            continue;
        }
        assert_eq!(
            call.disposition,
            D::ImplementedBehavior,
            "A 0044F3D0 body callsite still reports pending"
        );
        if call.target != 0 {
            assert_eq!(
                call.stub.resolution,
                R::ImplementedLocal,
                "A 0044F3D0 direct callsite still resolves as pending"
            );
        }
    }
    assert_eq!(logs, 8, "0044F3D0 holds eight log callsites");

    assert!(
        in_body(0x40_67D2, 0x40_67D2).all(implemented),
        "004067D2 TES_PreloadDefaultAssets is implemented (Phase 13a)"
    );
    assert_eq!(in_body(0x40_67D2, 0x40_67D2).count(), 1);
    let preload = find(0x44_3550).expect("00443550");
    assert_eq!(preload.calls.len(), 71, "00443550 holds 71 body callsites");
    let mut crt = 0;
    for call in preload.calls {
        if call.target == 0x98_208B || call.target == 0x98_11E2 {
            assert_eq!(
                call.disposition,
                D::HostSupplied,
                "A 00443550 CRT callsite is host_supplied"
            );
            crt += 1;
            continue;
        }
        assert_eq!(
            call.disposition,
            D::ImplementedBehavior,
            "A 00443550 body callsite still reports pending"
        );
        if call.target == 0x43_B420 {
            assert_eq!(
                call.stub.resolution,
                R::ImplementedLocal,
                "A 0043B420 callsite still resolves as pending"
            );
        }
    }
    assert_eq!(
        crt, 6,
        "00443550 holds three sprintf and three cookie callsites"
    );

    let mut phase13b_sites = 0;
    for call in in_body(0x40_67DC, 0x40_6861) {
        phase13b_sites += 1;
        let expected = match call.site {
            0x40_67DC => D::ObsoleteRuntime,
            0x40_67FD => D::HostSupplied,
            _ => D::ImplementedBehavior,
        };
        assert_eq!(
            call.disposition, expected,
            "A phase-13b callsite has the wrong disposition"
        );
    }
    assert_eq!(phase13b_sites, 6, "The phase-13b block holds six callsites");
    let mut phase13c_sites = 0;
    for call in in_body(0x40_6866, 0x40_68CE) {
        phase13c_sites += 1;
        let expected = if call.target == 0x40_4EC0 {
            D::ObsoleteRuntime
        } else {
            D::ImplementedBehavior
        };
        assert_eq!(
            call.disposition, expected,
            "A phase-13c callsite has the wrong disposition"
        );
    }
    assert_eq!(
        phase13c_sites, 7,
        "The phase-13c block holds seven callsites"
    );
    let mut phase13d_sites = 0;
    for call in in_body(0x40_68D1, 0x40_6944) {
        phase13d_sites += 1;
        let expected = if call.target == 0x40_4EE0 {
            D::ObsoleteRuntime
        } else {
            D::ImplementedBehavior
        };
        assert_eq!(
            call.disposition, expected,
            "A phase-13d callsite has the wrong disposition"
        );
        if call.target == 0x44_7D80 || call.target == 0x6A_F850 {
            assert_eq!(
                call.stub.resolution,
                R::ImplementedLocal,
                "A phase-13d callsite still resolves as pending"
            );
        }
    }
    assert_eq!(
        phase13d_sites, 4,
        "The phase-13d block holds four callsites"
    );

    let owner = dependencies()
        .iter()
        .find(|dependency| dependency.identity == 0x40_6360)
        .expect("00406360");
    assert_eq!(
        owner.disposition,
        D::ImplementedBehavior,
        "00406360 Startup_InitializeGameData is implemented"
    );
    for call in game_data.calls {
        assert_ne!(
            call.disposition,
            D::Pending,
            "A 00406360 callsite still reports pending"
        );
        if (0x9A_9F54..=0x9A_9F72).contains(&call.site) {
            assert_eq!(
                call.disposition,
                D::HostSupplied,
                "A 00406360 unwind funclet callsite is host_supplied"
            );
        }
    }
}

#[test]
fn phases_14_to_16_keep_their_windows() {
    let winmain = find(0x40_DF80).expect("WinMain is in the scaffold");
    let in_body = |low: u32, high: u32| {
        winmain
            .calls
            .iter()
            .filter(move |call| call.site >= low && call.site <= high)
    };
    let mut phase14_sites = 0;
    for call in in_body(0x40_EB31, 0x40_EBB8) {
        phase14_sites += 1;
        if call.site == 0x40_EB92 || call.site == 0x40_EBB4 {
            assert_eq!(
                (call.target, call.disposition),
                (0x40_4EE0, D::ObsoleteRuntime),
                "The phase-14 log calls are obsolete_runtime"
            );
            continue;
        }
        assert!(
            implemented(call),
            "A phase-14 callsite still reports pending"
        );
    }
    assert_eq!(phase14_sites, 8, "The phase-14 block holds eight callsites");

    let (mut phase15_sites, mut warnings, mut crt) = (0, 0, 0);
    for call in in_body(0x40_EBB9, 0x40_EE79) {
        phase15_sites += 1;
        if call.target == 0x4A_7A60 {
            assert_eq!(
                call.disposition,
                D::ObsoleteRuntime,
                "The phase-15 warnings are obsolete_runtime"
            );
            warnings += 1;
            continue;
        }
        if call.target == 0x98_3479 || call.target == 0x98_262D {
            assert_eq!(
                call.disposition,
                D::HostSupplied,
                "The phase-15 atol / _stricmp sites are host_supplied"
            );
            assert_eq!(
                call.stub.resolution,
                R::HostSupplied,
                "The phase-15 atol / _stricmp sites are host_supplied"
            );
            crt += 1;
            continue;
        }
        assert!(
            implemented(call),
            "A phase-15 callsite still reports pending"
        );
    }
    assert_eq!(
        (phase15_sites, warnings, crt),
        (24, 4, 3),
        "The phase-15 block holds 24 callsites (4 warnings, 3 CRT)"
    );

    let mut phase16_sites = 0;
    for call in in_body(0x40_EE7A, 0x40_EF23) {
        phase16_sites += 1;
        if call.site == 0x40_EE7F {
            assert_eq!(
                call.disposition,
                D::ObsoleteRuntime,
                "The phase-16 log is obsolete_runtime"
            );
            continue;
        }
        assert!(
            implemented(call),
            "A phase-16 callsite still reports pending"
        );
    }
    assert_eq!(phase16_sites, 7, "The phase-16 block holds 7 callsites");
}
