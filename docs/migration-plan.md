# Migration plan: C++ phases 00–16 to Bevy, then Phase 17

Written 2026-11-04. Status: M0 done (see [M0 results](#m0-results)); M1 and M2 code and tests done, on host tests only (see [M1 progress](#m1-progress) and [M2 progress](#m2-progress)). Notes paths below are relative to
`../OblivionBotInfos/Recomp/StartupLoading/`; "C++" paths are relative to
`../OblivionRecomp/`.

## The goal in one paragraph

Move everything the C++ repository does into this Bevy project, then prove the Bevy build reaches the
same point in startup with the same state. "The same point" has an exact definition (see
[The gate](#the-gate)): the end of Phase 16, WinMain `0040EF23`, or `0040EF54` on the default no-cell path,
with every earlier phase's recorded values matching. Only then does Phase 17 start, here.

## Decisions (user, 2026-11-04)

| # | Question | Decision | Consequence |
| --- | --- | --- | --- |
| D1 | What "drop Vulkan" meant | Drop the D3D9/Win32 host. The C++ repository has no Vulkan code. | wgpu itself uses Vulkan on this machine, which is fine. |
| D2 | C++ baseline | The Oct 10 logs and recorded rows, plus an attempt at running the prebuilt tests under Wine ([M0.4](#m0-foundations-before-any-game-code)). | The gate must not depend on the Wine attempt working. |
| D3 | How much ECS during migration | **ECS-native from the start**, as in Evil Genius. | See [Architecture](#architecture). Order and identity are kept explicitly rather than by keeping the C++ object shape. |
| D4 | The C++ format and lint item on phases 06–16 | Replaced by `cargo fmt` and `cargo clippy` at the gate. | Recorded in the roadmap as superseding the C++ item at cutover. |
| D5 | Virtual display | KWin's built-in virtual framebuffer, the same as Evil Genius. See [Testing](#testing-on-the-virtual-display). | `kwin_wayland` is already installed, so nothing new is needed. |
| D6 | Trig last-bit differences (`f64::sin`, `tan` use the platform library) | Record them as known, and port exact results only where a row disagrees. **Extended (user, 2026-11-05):** every trig and CRT math call goes through full precision and rounds once at the store, read from the disassembly with no row needed, since `fsin`, `fsincos` and `__CIsin` ignore the 24-bit rounding the arithmetic around them obeys. Last-bit differences from the host library against the original stay recorded. | `sqrt`, `fmod` and float parsing round correctly on both, so the exposure is trig (`__CItan` at Phase 05 and later). |
| D7 | Platforms | Native 64-bit, Linux first. Avoid Linux-only code so Windows stays possible. No x86 build, and no in-process original-engine calls. | `src/interop` was never implemented, so nothing is lost. |

## What exists today

| Item | Amount | Migration treatment |
| --- | --- | --- |
| Ported game code `src/{tes,gamebryo,bs}` | 31,886 / 27,065 / 10,850 lines | Port, in phase order |
| Startup sequence `src/startup` | 13,237 lines, of which `scaffold.cpp` is 11,560 generated | Port the WinMain blocks; generate the scaffold (see M1) |
| Host `src/{app,ui,render,audio,video,assets,core}` | About 10,700 lines | Replace with Bevy where it is a host, port where it is logic (XML, tiles, BSA, saves) |
| Tests | 75 CTests, 31,396 lines, plus 11 `.inc` files with 13,941 rows (2.98 MB) | Port per phase; rows become fixtures |
| Engine headers `include/oblivion/engine/{classes,virtuals}` | 378 generated | **Do not port.** Nothing in `src/` includes them. Keep them as a layout reference; generate Rust size checks only if needed |
| Research and evidence | 88 MB in the notes; 40-odd `phaseNN-runNNN` folders | Stays where it is, and stays the single record |
| IDA database | `OblivionBot/Oblivion.exe.i64` | Stays where it is, kept in sync |

Status per `Startup-Roadmap.md`: phases 00–05 are signed off; 06–16 are ported and wait only on format and
lint; 17–22 are not started. The last C++ test logs (Oct 10, x86 and x64) show 75 of 75 passing.

## The oracle: three sources, one direction

Keep these separate. They answer different questions.

1. **The original executable, through captured evidence, is the authority.** `tests/startup/phase*_expected.inc`
   and `evidence/phase*-run*/records.txt` hold values read from the running original.
2. **The C++ port is a comparison peer, not an authority.** It passes the same rows. If Bevy and C++ disagree, the
   rows decide, and a row-free disagreement goes back to IDA.
3. **The Oct 10 logs are the C++ end state on record.** The `[INFO] Phase-NN:` lines appear in `LastTest.log`.

Because both implementations are tested against the same rows, a Bevy pass plus the recorded C++ pass means that
they agree wherever rows exist. That is the core of the side-by-side check, and it needs no running C++.

### Known nondeterminism

The Oct 10 logs differ between x86 and x64 on these values, so a plain log diff will never be clean:

| Value | x64 | x86 |
| --- | --- | --- |
| Phase-12 editor IDs | 7939 | 7938 |
| Phase-12 item scripts not executed | 1338 | 1335 |
| Phase-14 change records | 5485 | 5486 |

The likely cause is the time-seeded `srand(_time64)` at `0047DF80` feeding leveled-list rolls; the notes record that
this variance follows CRT `rand` rolls for container changes and change-map keys. The handoffs do not explain the
editor ID difference, so M4 must confirm it rather than assume it. The notes also carry unresolved gaps: P12-RUN-001 records
7,896 editor IDs where the Oct 10 logs show 793x, and interior cells are 1,916 in Phase 12's notes and 1,914 in Phase 13's.

The comparison handles this in two ways:

- The test fixture injects the fixed values the C++ fixture uses (`get_tick_count` 0x1000, `time64` 0x60000000), and the
  Rust `rand` is the same LCG, so test runs are reproducible.
- The app-level log diff uses a declared tolerance list, with each tolerated field and its reason in the comparison tool.
  Anything not listed must match exactly. `startup_phase13` also failed once in a parallel x64 run and passed on rerun,
  so the gate runs phases serially.

## The gate

Phase 17 may start when **all** of these hold on the same commit.

**G1. Row parity.** Every row in every imported fixture matches, for phases 04–16, byte for byte under the C++ tests'
normalization (`<M+0C>`, `<P>`, `<blk>`). No row is deleted or relaxed without a recorded reason.

**G2. Phase 16 end state.** Bit for bit on the default path, per P16-RUN-001: `Rect {2048, 2048, 128}`, rotation 0, one new
change record, controller `+0x320` and `+0x1F4`, exit at `0040EF54`. The `XPAichan01` (RUN-002) and `X 4 / Y 7` (RUN-003) paths
also match their rows, including the land height 530.77 and the RUN-002 percent sequence.

**G3. Log parity.** The Bevy app emits the same `Phase-NN:` lines as `main.cpp`, in the same order, and these match the
baseline apart from the declared tolerances:

```text
Phase-12: files compiled 11, records 114808 of 1277335, forms 103510, ...
Phase-13: models queued 26, blood decals 3, player base NPC_ 0x7, fatigue 150, health 80, ...
Phase-13e: model tasks run 15 (models built 7, cached 8, failed 0), queued 11 (8 Havok, 3 NIF 10.2.0.0), ...
Phase-14: worldspace Tamriel, idle roots 45, archives 17, change records ~, tracked actors 1998, autocalc NPCs 1679, ...
Phase-15: path no settings, cell 0, Rect {2048, 2048, 128}, tail skipped
Phase-16: cell 0, Rect {2048, 2048, 128}, rotation z 0, 0040EF54
```

**G4. Boundary parity.** Every host substitution, host stop and counted boundary in the roadmap's carried lists exists in Rust
with the same address and the same count. A counted boundary that goes missing silently is a failure, because the notes record
that boundaries can own end-state fields (TES `+0x30`, P15-RUN-009).

**G5. Visible parity, bounded.** KWin's title bar clamps a 1920x1080 window to 1052 on the 1080-pixel virtual display, so M6 needs `decorations: false` or a taller display before this comparison can be made. The LoadingMenu and MainMenu still come up with working menu input, music and movie. Compare
against the C++ captures in `out/captures` and `--menu-capture` frame 288 at 1920x1080. This proves what the C++ tests prove for
those screens, not rendering fidelity. It runs on the virtual display (D5).

**G6. Quality.** `cargo fmt --check` and `cargo clippy` are clean, and each host-only item is labelled.

Passing the gate is not an original-validation bonus on the 110% scale, and it does not close the phases' own carried work.

## Architecture

### Workspace

```text
OblivionRecompBevy/          Bevy binary: App, plugins, host services
  crates/recomp_core/        CRT and x87 behaviour, logging, original-address types
  crates/recomp_game/        gamebryo/, bs/, tes/ as modules: components, resources, systems
  crates/recomp_startup/     WinMain blocks, Startup_* routines, scaffold
  crates/recomp_host/        Bevy plugins: window, render, UI, audio, video, input
  fixtures/                  imported rows and pinned config, each with provenance
  tools/                     fixture import, side-by-side comparison
```

The three engine layers stay modules because the C++ includes form cycles (gamebryo↔bs, gamebryo→tes, bs→tes). Splitting them
into crates would force an untangling during a parity port. `recomp_game` uses `bevy_ecs` but no Bevy rendering, audio or
windowing, which is what lets phase tests run without a window.

### ECS-native mapping (D3)

Game objects are entities from the first port. Inheritance becomes capability components. The C++ object shape is evidence, not
a design to keep.

| C++ today (measured) | Bevy | Fidelity obligation |
| --- | --- | --- |
| Forms and base objects (103,510 at Phase 12) | One entity each: identity component (form ID, type code, flags) plus capability components such as `TESFullName`, `TESModel`, `TESContainer` | Component names follow the original base classes. Comment original object offsets on fields |
| References, actors, cells, worldspaces | Entities with `TESObjectREFR`, `Actor`, `TESObjectCELL` state components; the player is an entity with the `0x804` chain split across components | The row printers rebuild the original view, so the components must retain every asserted field |
| `NiObject` family (376 classes in `gamebryo`) | One entity per object; typed link components | `NiNode` child slots are an explicit slot-array component, not Bevy `Children`, because order and empty slots are observable. Bevy transforms are a host projection only |
| Pointers and `shared_ptr` (1,131 lines), `unique_ptr` (367) | Typed `Entity` links in components | `Entity` is transient: never saved, printed or compared. Where a refcount is observable it stays a field, and the release at zero despawns |
| Original IDs | `FormIdIndex` resource from form ID to `Entity`, updated at spawn and despawn | Same lookup results, including failed lookups |
| Original lists and maps (`BSSimpleList`, `NiTMap`, `NiTPointerMap`, TES object lists, cell reference lists) | Hand-written ordered index resources or components holding entities | Bucket counts and chain order are original data (401 GMST buckets). **Never use query iteration or `HashMap` where order is observable** |
| About 36 `*Globals` structs and 20 `x_globals()` accessors | One typed `Resource` per struct | No giant game resource |
| About 35 `*Services` structs of `std::function` hooks | Resources holding traits or closures; Bevy host versions added by plugins, test versions by tests | Keeps the seams the phase tests rely on, including `bind_real_arms` and the recording arms |
| 146 classes with virtuals; 213 `dynamic_cast` lines | Dispatch on the type code, or on which components exist | Preserve `__RTDynamicCast` outcomes, including failures. Keep a type → original vtable address table, because rows assert values such as `vt=00A479D4` |
| `thread_local GameThreadLocals` (`byte_184/185/186`) | `thread_local!`, as in C++ | Worker threads read their own copy |
| 66 `reinterpret_cast`, 198 code `+0x` sites | Safe byte parsing with explicit little-endian reads | Layouts were never byte-exact in C++ either |

Write this mapping per class family into `docs/ecs-mapping.md` as each family is ported: original class and size, components, field offsets, and why each list is ordered as it is.

### Keeping original order in ECS

This is the main technical risk of D3, so these rules are fixed before any port:

- **Ordered code runs in exclusive systems on `&mut World`.** Original stores happen immediately, so ordered code does not use `Commands`, whose deferred flush would change store order and what later code reads.
- **No parallel scheduling between phases.** Each phase block is an exclusive system in an explicit chain. Nothing else touches game state while one runs.
- **Row output is independent of Bevy's storage.** Row printers walk the ordered indexes and never a query, and they print form IDs, type codes and original addresses, never `Entity` values.
- **A `PhaseOutcome` replaces `main.cpp`'s nesting** (brace depth 14, 7 nested `try`). A host stop still falls through to MainMenu as it does today. Phase output lives in resources, so it outlives the phase.
- **Long blocks are resumable.** Phase 12 and the drains must keep the LoadingMenu rendering, so they are written as steps at their original loop boundaries. The exclusive system runs steps until a time budget is spent, then returns so that Bevy can draw a frame. Game-visible order does not change; only frame boundaries do. Tests run the steps straight through.
- **Worker threads cannot hold `World`.** The task manager and movie threads stay real `std::thread`s with their semaphores and exchange data through queues. State they read concurrently (P07's movie-thread reads of `00B3A6E0` and the TileMenu table) goes in small shared atomics, not components. The C++ port drains the IO thread's tasks on the main thread as a recorded substitution, and that stays until a later phase needs more.

**M0.6 proves these rules first:** one small vertical slice from Phase 11 or 12 (a form type with capability components, its ordered list, a Ni node with slots, and matching fixture rows) before the mass port. If the slice shows a rule failing, stop and bring it back to the user rather than quietly working around it.

### Testing on the virtual display

D5 uses the Evil Genius procedure (`EvilGeniusBevy/docs/testing.md`). All automated test processes run inside it, including those with no window:

1. Create a private mode-700 runtime directory. Unset `DISPLAY` and `WAYLAND_DISPLAY`, and set `QT_QPA_PLATFORM=offscreen`.
2. With that directory as `XDG_RUNTIME_DIR`, start `dbus-run-session -- kwin_wayland --virtual --no-lockscreen --no-global-shortcuts --no-kactivities --socket <unique-name> --width 1920 --height 1080`.
3. Run tests with `DISPLAY` unset and `WAYLAND_DISPLAY=<unique-name>`. Stop the compositor when finished.

Never fall back to the physical display; report the tests as blocked instead. In M0.1, adapt `tools/virtual_display.py` from `CreeperWorld3Bevy` or `DragonsteadBevy` so that this is one command. Test input stays virtual, with windows unfocused and no cursor movement, as in Evil Genius. 1920x1080 matches the size P07 was captured at.

### Host replacements

| C++ host | Bevy replacement | Parity consequence |
| --- | --- | --- |
| Win32 window, class, icon, eight-case procedure | `bevy_winit` window | Substitution. Port the window logic that affects game state (`iSize W/H`, focus, single-instance test) |
| D3D9 device, caps, device sequence (P04) | wgpu via Bevy | **The caps drive game state.** P05 picks render path 2+ and `shaderpackage013.sdp` (653 entries) from them. Supply a pinned caps fixture from the run, not from wgpu |
| D3D9 shader creation from `.sdp` SM1–3 bytecode | Parse and retain as data; no GPU creation | wgpu cannot take D3D9 bytecode. The 27 slots, pass tables and 415 / 54 pass counts remain testable as data. Real shaders are rewritten as WGSL when a later phase draws with them |
| 13 render targets (43,661,464 bytes) | Same records and byte accounting as data | GPU targets are created only when something draws |
| DirectInput8 | Bevy input feeding a port of the buffered event logic | Keep OS-global state and the poll and discard sequence |
| XAudio2, Media Foundation, DirectSound globals | `bevy_audio` with the `mp3` and `wav` features | Keep six buses, the voice cap and the DirectSound-shaped state as data |
| Bink (x86 `binkw32.dll`; x64 FFmpeg) | FFmpeg path, as in the C++ x64 build. `/usr/bin/ffmpeg` has the Bink decoders | `binkw32` is x86-only, so this is already a recorded substitution. Keep the real movie thread and order |
| D3D9 quads for tiles and fonts | Bevy UI or 2D, with the 960-unit virtual frame | The Tile tree, XML build and expressions are ported logic. Only projection changes |
| Havok SDK calls | Counted records exactly as C++ | Carried, unchanged |
| `SHGetKnownFolderPath`, CRT, Win32 files | Configured paths; case-insensitive lookup with backslash handling | New work specific to Linux. See M1 |

## Milestones

Each milestone ends with its phases' tests passing, in phase order. Later phases stay provisional until earlier ones pass.

### M0: foundations (before any game code)

**M0.1 Workspace.** Done for the profiles (see `Cargo.toml`). Create the crates when M0.3 needs them, and add the virtual display tool.

**M0.2 Fixture import.** `tools/import_fixtures.py` turns each `.inc` into a plain text file under `fixtures/`, with the source path
and SHA-256 in a manifest, and fails on any drift. `evidence/` is not in the C++ git repository, so the manifest also records each
`records.txt` hash. Phases 04, 05, 06 and 16 have no `.inc`; their values are written by hand in `phase0{4,5,6}_tests.cpp` and
`phase16_tests.cpp`, so lift those into fixtures, checked against `records.txt`.

**M0.3 `recomp_core` primitives.** These came from the MSVC CRT or are copied per file in C++, so build them once and test them first:

- MSVC `rand`/`srand`: `holdrand = holdrand * 214013 + 2531011`, return `(holdrand >> 16) & 0x7FFF`. No implementation exists in the C++
  tree. Check it against the `RAND` rows in `phase14_expected.inc` and the 157 draws of P14-RUN-004.
- x87 helpers, one copy of each: `x87_float_copy`, `x87_float_store`, `ftol`, `fistp`, `x87_truncated_low16`, `unsigned_to_x87`,
  `truncate_to_int32`, plus `fmod` with float 2π for `0065A450`. Match the C++ `double` emulation, which already passes rows on both
  architectures. Rust's `f32` and `f64` are SSE on x86-64, like the C++ x64 build. No `long double` is involved.
- C-locale `_stricmp`; `fgets` text mode (0x200 limit, Ctrl-Z, lone CR); `sscanf` and `sprintf "%f"` float behaviour; `atol`.
- Original hashes (`bsa_name_hash`, `bsa_rolling_hash`, `havok_pointer_hash`), FNV-1a, and the logging levels.
- The three pointer-keyed maps in `tes/forms_world` and the `004A80A0` pointer-key hash: check whether iteration order is ever observed.

**M0.4 C++ baseline.** Copy the Oct 10 `LastTest.log` files into `fixtures/cpp-baseline/` with their hashes, and reference `out/captures` by path (161 MB, so not copied). Then try one live run (user-gated):
`recomp_startup_phase10_tests.exe` under `WINEPREFIX=~/.wine-oblivion`, with `Z:\` Data paths. The blockers on record are:

- `MSVCP140D.dll` and `VCRUNTIME140D.dll` are not on this machine: the `Microsoft Visual Studio` folder on the old Windows drive is empty. (`ucrtbased.dll` exists in the
  Windows Kits.) They are debug runtimes, so they must come from a machine with Visual Studio.
- D3D9 tests under Wine would run on wined3d, so a difference there says nothing about the port.
- An app run such as `--startup-probe` reads the prefix's own INI (`46286e75…`). Reproducing the Windows log means placing the pinned INI (M0.5) in the prefix for the run and
  restoring yours byte for byte afterwards, the same rule the earlier debug runs followed.

If the Wine run works, it gives a second live data point and allows running C++ with the same fixed inputs. If not, nothing in the gate depends on it (D2).

**M0.5 Pin config.** The evidence used the Windows files, not the Wine prefix:

| File | Hash | Source |
| --- | --- | --- |
| `Oblivion.ini` | `d243541f…` | `<windows-profile>/OneDrive/Documents/My Games/Oblivion/Oblivion.ini` (matches the hash in the notes) |
| `Plugins.txt` | `f2e65b7c…` | `<windows-profile>/AppData/Local/Oblivion/Plugins.txt` (211 lines, matching P10's "11 of 211"; hash not stated in the notes) |

The Wine prefix's `Oblivion.ini` hashes differently (`46286e75…`), so a test reading it would silently take different paths. Copy the pinned files to
`fixtures/config/` read-only, and add `iSize` and other machine-dependent values from the runs (P07 is 1920x1080). Tests read only fixtures.

**M0.6 ECS slice.** The vertical slice described under [Keeping original order in ECS](#keeping-original-order-in-ecs). It needs M0.2, M0.3 and M0.5.

### M0 results

Done 2026-11-04. These are host tests, so they earn no validation bonus, and no phase is closed by them.

| Item | Result |
| --- | --- |
| M0.1 | Workspace crates `recomp_core` and `recomp_game`; `tools/virtual_display.py` (KWin) runs all tests. `recomp_startup` and `recomp_host` wait for M1 and M2 |
| M0.2 | `tools/import_fixtures.py`, with `--check`: 11 `.inc` files, 9 raw runs for phases 04–06 and 16, and hashes for all 42 run records. `phase07`'s `.inc` is a C++ table, so it is kept verbatim for M3. **Not done:** the hand-written values in `phase0{4,5,6}_tests.cpp` and `phase16_tests.cpp` are lifted when those tests port (M2, M5); the raw runs they cite are already here |
| M0.3 | `recomp_core`: 15 tests. `rand` reproduces all 157 draws of P14-RUN-004 in one sequence. The BSA hash matches every folder and file record in `Oblivion - Misc.bsa`. Logging moved to M1, with `logging_settings` |
| M0.3 pointer maps | `road_store` and `CellRegionLocks` are only looked up, never iterated, so address order cannot show. `004A80A0`'s pointer-key hash has no port yet: check it in M4 |
| M0.4 | Oct 10 logs archived. **Wine run failed:** exit 53, `MSVCP140D.dll`, `VCRUNTIME140D.dll` and `ucrtbased.dll` missing from the prefix. The x86 `ucrtbased.dll` is in the old drive's `Windows Kits/10/bin/x86/ucrt/`; the other two need a machine with Visual Studio (`VC/Redist/MSVC/<ver>/debug_nonredist/x86/`). The gate does not depend on it |
| M0.5 | `Oblivion.ini` (`d243541f…`) and `Plugins.txt` (`f2e65b7c…`) pinned read-only in `fixtures/config/`. Nothing reads them yet |
| M0.6 | Passed, below |

**The ECS slice (TESGlobal).** All 95 forms load from the 11 shipped files as entities and match the original's rows: the 6 `AC20RAW` defaults, `COUNT 04` (count, FNV), `LIST 74` (count, FNV and order), and 5 `SAMPLE` and 5 `POST` byte rows, except for heap garbage and raw pointers. Also verified:

- The rows are unchanged when loading one record per step (the resumable rule).
- They are unchanged when noise entities shift every `Entity` and the query order.
- They pass through a real Bevy `Schedule` of chained exclusive systems.

Measured at opt-level 1: 103,510 six-component form entities spawn and index in 66 ms, the ordered walk takes 1.3 ms, and all lookups 3 ms. That is the ECS overhead only, not load time, since no parsing or real components were involved. Check the real number in M4.

**What the slice does not prove:**

- Node slots have only unit tests, with no recorded row yet. `gamebryo_scene_node` and P13e's `NODE` rows check them in M1.
- Worker threads were not exercised.
- A single simple form type says little about the components of `TESNPC` or `TESObjectREFR`. Revisit the rules at the first of those.
- The slice's file walk reads only the GLOB group, and M4 replaces it.
- `FormsByEditorId`'s case-insensitivity is assumed, because no slice row looks up a name. Check it against `00B06164` in M4, with its bucket order.
- P11's first draw `6C3F` fits a seed 11 seconds before the recorded `_time64`, the only fit within 40 seconds. That value was read at `0066AEF2`, and the seed call at `0047DFA6` was not recorded, so this is a fit and not a record. M3's Phase 11 test needs a `time64` sequence, or injects `rand` directly as C++ does.

The slice caught one real mistake on its first run: the master bit (`00447050`) and mod list rules (`0046B6CC`) had been skipped, and the `SAMPLE` rows failed until they were ported. That is the row check doing what it is for.

### M1: phases 00–03, the base layer

Paths and INI settings, FileFinder, BSA v103 and archives, zlib, containers with original bucket order, factories and the Ni object layer (the 58 traced factories, NIF streams,
properties, controllers, particles, sequences). The 2,304 GMST defaults wait for M3, where Phase 07 reads them.

- Host-owned CRT entry and `_cinit` classification carry over as dispositions.
- Port the scaffold generator to emit Rust from the same `manifest.json` and `dispositions.json`, with both copied here. The generator also writes
  `Startup-Call-Ledger.md`: only one repository may own that write, and at cutover that becomes this one.
- Directory enumeration: `FindFirstFile` and `directory_iterator` sites (7 in `bs`, `tes`) are used for plugin scans and archive lookups. Sort or pin their order wherever it is observable, because Linux `readdir` order differs from NTFS on Windows.
- Tests: the 41 `gamebryo_*` tests, `bs_archive`, `tes_settings`, `core_containers`, `startup_scaffold`, `startup_memory_failures`, `logging_settings`.

### M1 progress

Host tests only: no validation bonus, and no phase is closed by them.

| Batch | State | Checked by |
| --- | --- | --- |
| Logging, Win32 profile rules, INI settings (635 static settings), blend settings, path setup | Ported | `logging_settings` and `tes_settings` ported; the pinned INI loads through the WinMain order |
| `NiFile`, `BSFile`, `ArchiveFile`, `CompressedArchiveFile`, BSA archives, the manager, FileFinder, the open and exists routes | Ported | `bs_archive` ported, including P03-RUN-006 on the shipped data: 17 archives in the original's order with their type masks, every Meshes record hash, packed lookups, and a real texture inflated |
| The game's `BSSimpleList` as `PointerList` | Ported | Its own tests; the `LIST 74` rows through the DataHandler |
| `gamebryo` base: transforms, `NiStringPalette`, search paths, the stream-object table, `NiObjectNET`, `NiAlphaProperty`, `NiAVObject` links, `NiNode` | Ported | `gamebryo_string_palette`, `gamebryo_search_path`, `gamebryo_object_net`, `gamebryo_scene_node` ported (minus their factory checks, which wait for the factory). No recorded rows cover these yet |
| Texture palette (`bs_archive`'s remaining part) | Ported | Its `bs_archive` checks |
| Extra data, properties, the factory registry and the stream object table | Ported | `gamebryo_extra_data`, `gamebryo_properties`, `gamebryo_stream_objects` ported, plus the factory checks left over from earlier batches |
| Animation keys and interpolators, geometry and skin, textures, controllers and sequences, particles | Ported | Their `gamebryo_*` tests, ported with expected values unchanged; dropped checks are `shared_ptr` lifetime only |
| Scene derived classes, property and effect state, scene update, BS scene graphs | Ported | `gamebryo_scene_graph`. Controllers and particle systems are connected. 00742060, `update_effects_downward` on a particle system, stays an error, as in the C++ |
| The stream graph, post-load, root callbacks, whole-file loader | Ported | `gamebryo_stream_graph`, `gamebryo_stream_finalize`, `gamebryo_factory_creation`, `gamebryo_factory_payload` ported, and `stream_graph_real` (below) |
| Scaffold generator (`manifest.json`, `dispositions.json`) | Ported | `verify_manifest` ported. A clean run of the C++ generator matches every dependency and call record. The generator writes the ledger only with `--notes-at-cutover` |
| Registration (scheduler, singleton defaults, collision dispatch, shader creators as data), `winmain_memory`, `UIScreenElementPalette` | Ported | Module tests and `startup_memory_failures`. Shader creator load methods and device binding are M2's |
| The rest of `startup_scaffold` (paths, path limits, non-ASCII paths, assets) | Ported | `scaffold_paths`, `scaffold_assets`. `verify_existing_window_branch` needs a real window and goes with Phase 04 in M2 |
| Directory order | Audited | Archive scans go through the sorted `host_path::find_files`. `tes_file`'s `*.esm` then `*.esp` plugin scan is unported and its order is observable, so it must use `find_files` when ported |

**First real check of the Ni code.** `stream_graph_real` loads the seven P13e models from the shipped Meshes archive through the whole file pipeline and compares every recorded `NODE` row: class, name, flags, child count, properties in order, controller and extra count. All 86 match, with no object the rows do not list and no flag difference. The rows were captured after 00436880 Model_Init and geometry prepare, which M1 does not have, so three differences are declared and counted in the test: the root's `BASE <path>` name (7), 004809A0's `EditorMarker` strip (2), and the ShadowLight boundary's `BSShaderPPLightingProperty` (1). The row's +0xBC shader field and `MODEL` reference counts are not compared, because they come from the model loader. Exact rows wait for that loader.

Only these rows check the Ni classes against the original so far. They check tree shape, not field values such as transforms or particle data. Every other M1 check compares with the C++ port, which earns no validation bonus.

The user authorized finishing M1 without stopping at each batch (2026-11-04). Class families are ported in parallel worktrees to `docs/porting.md`, then merged and checked on `master`.

No WinMain driver runs any of this yet: M1 has the owners and their tests, and the blocks that call them in order come with their phases.

Questions about the C++, settled on 2026-11-05 by static reads of the IDB, the disk exe and P08-RUN-002 (no debug run, no IDB edits). Each one is original behaviour read from code, except the material ID, which has a recorded value:

| Question | Original | Result here |
| --- | --- | --- |
| `add_interp_info` and blending | 006CC5C0 stores the item +0 pointer, and 006EB5E0 and 006CD0F0 skip on that pointer. There is no stream index at runtime, and flag 1 reads item +0x14 rather than rejecting | **C++ port defect, fixed here.** Sampling tests the source, and the bool and float many paths accept flag 1, so two manager-controlled sequences now blend. The dirty path with more than two items is still unported |
| `recompute_two_weights` 0 then 1 | 006CCACA and 006CCACD: two `fstp [esi+8]`, both on the second item | Original quirk. Kept |
| Billboard bound in the upward pass | The vtable holds 00722340 at +0x78 and 00709EC0 at +0x94, with no merge. 007219C0 runs the NiNode pass first | Matches the original. It is not a repeated growth: the center becomes the translation, so a repeat adds nothing unless the translation moved |
| Default material ID | Disk and image value 1; `InterlockedIncrement` returns the new value. P08-RUN-002 enters the constructor at 1 for 007098F4 and at 2 for the palette's first | The code is right: default 2, palette 3..0x13. The IDB comment at 007AB4C1 and the C++ comment saying 1 and 2..0x12 are wrong, and are left for the IDB sync |
| Palette 00707370 Update | 007AB564 calls it before the slot store; 00707580 reaches 0070C120 and 00722AA0 | **C++ port omission, fixed here.** The world bound is the model bound. The C++ test's zero bound expected value was changed for this reason, and no row covers it |

One place follows the executable instead of the C++, because the order is observable: 007AB1D0's property creation and attach order. No C++ expected value depends on it.

Decisions made while porting:

- **A failed load publishes nothing.** The graph despawns the table's entities on failure, except textures the palette still holds. This is a host rule: the original's cancellation and partial publication remain a recorded TODO.
- **Dispositions record the C++ port.** `dispositions.json` is copied unchanged, so `implemented_behavior` means the C++ has the body. `recomp_startup::scaffold::RUST_PORTED_THROUGH` states what this repository has.
- **Worktrees never share a `CARGO_TARGET_DIR`.** Cargo hashes workspace crates by their path relative to the workspace root, so sharing links another worktree's code with no error. It showed up as a fixture path baked in from a removed worktree.

- **`core_containers` is dropped.** `include/oblivion/containers/` is a host library derived from xOBSE, and only its own test includes it. No game code uses it. Its `LockFreeMap` and `LockFreeQueue` are abstract stubs with no recovered behaviour. The game's real `BSSimpleList` is in `tes/tes_file.hpp` and is ported; the original's other containers are ported where their owner needs them, keeping bucket order.
- **Stream objects are entities with a `NiClass` tag**, in place of the C++ `FactoryObject` variant of `shared_ptr`s. Links are `Entity` values checked against that tag, so the same wrong-type rejections apply. The C++ cycle guard is kept even though a cycle could not leak here, because the host still rejects whatever would need the unported detach.
- `FileFinder` and the archive manager are resources, not singletons, so the C++ singleton-lifetime checks are gone. The 00B27E4C / 00B27E50 slots are `BsFileCallbacks`.
- The buffer limit 00B04360 lives on the manager and is shared with its archives, so that parallel tests cannot race on a global.
- New Linux work: `recomp_core::profile` ports `GetPrivateProfile*`, and `recomp_core::host_path` ports backslash paths, case-insensitive lookup and NTFS's sorted `FindFirstFile` order. Their rules follow Wine's documented behaviour, not a Windows run; duplicate INI sections and `0x` integers are unverified, and shipped files use neither.
- The registry collection's open always fails on Linux, so `sBasePath` keeps its default. If the evidence machine's HKLM key held another value, later readers of `sBasePath` could differ; check when one is ported.
- **Registry:** `NiFactoryRegistry::create` spawns a class's cold components by original constructor address, or reports `Unsupported`. `NiFloatData` is tag-only until the animation classes arrive. A factory failure midway through a table despawns what it spawned.
- **`MaterialIdCounter`** is a resource rather than a C++ atomic. No port so far constructs materials off the main thread; revisit if a worker thread ever does.
- `plan_archive_initialization`, the C++ host's menu shortcut, is not ported. M6 uses the real manager.

### M2: phases 04–06, the host boundary

The hardest milestone conceptually, because these phases are mostly D3D9, DirectInput, DirectSound and Bink calls.

- **Comparison here is data-level, by necessity.** Port the state and accounting (P04 FaceGen tables and counts, P05's 16 effect slots, 27 shader classes, 13 render targets,
  shader package parsing and pass tables, cameras, cube FNVs). Device calls become recorded substitutions.
- Keep the store traces as data: they are what `phase04_tests.cpp:385` and `phase05_tests.cpp` assert.
- Bring up the Bevy window, a real movie thread with intro stubs (P06), and the pathing owners.
- Pinned caps fixture as above. Note the open C++ item: the unpatched-IDB versus disk-exe background-run question at `0040616E/004061AF` and `0040F16E`.
- x87 note: P04 records control word `027F` at `0040E865` and `007F` after device creation; the movie thread stays `027F`. Thread-dependent precision becomes a choice of helper at each site.

### M2 progress

Host tests only: no validation bonus, and no phase is closed by them.

| Batch | State | Checked by |
| --- | --- | --- |
| Phase 04 renderer: `dx9_system`, `renderer`, render path, RendererInfo, render state sequence, source and cube textures | Ported | The 13 `phase04_tests` checks. **Against the original:** pinned caps reproduce `RendererInfo-run001.txt` byte for byte, all 76 caps dwords and 44 state rows match the renderer dump at 0049851E, and the recorded device calls match |
| Phase 04 OS globals, window procedure, FaceGen, WinMain window and renderer blocks | Ported | `verify_window`, `verify_os_globals`, the FaceGen checks, `verify_existing_window_branch`. **Against the original:** FaceGen's real data matches P04-RUN-002's row, and construction matches P04-RUN-001's trace |

| Phase 05 shader system: package, 27 slots, passes, constant maps, shader manager | Ported | The 12 `phase05_tests` shader checks. **Against the original:** slots, 415 passes, 653 entries, and the five pool headers at 007BA279, which depend on every acquire and release in order |
| Phase 05 render targets, effect textures, normalization cube | Ported | The 10 target and texture checks. **Against the original:** the 13 targets and 43,661,464 bytes, cube FNVs, the DDS reader's first real comparison |
| Phase 05 scene graphs and the joined window | Ported | The scene checks, plus `phase05_window_matches_the_runs`: all of `0040E89D..0040EAC1` as one sequence on pinned caps and shipped data. Its call order comes from the disassembly; the runs' probes only show that the sequence increases |
| Phase 06 input polling, movie player and thread, pathing | Ported | 16 of 18 `phase06_tests` checks; `verify_playback` is its data part only. **Against the original:** path builder dumps, tile set headers, movie object bytes. Input, activation and thread lifecycle are compared with the C++ only. Some row checks read back fixture values, so they are weak |
| Bevy window (`recomp_host`) | Opens at the INI's size on the virtual display, and exits cleanly | `window_smoke`. It draws nothing yet. KWin's title bar clamps a 1920x1080 client to 1052 on a 1080 screen, so the smoke test uses 1280x720 and the pinned size is checked as the request |

The real movie thread is a `std::thread` over a Bink, device and platform seam, with scripted frames. **No WinMain driver or app runs any of this yet**, so G3 is untouched: each phase window runs in tests only.

For M6: `verify_splash` (needs a host image loader), decoded playback frames and `verify_local_stub` (need a real decoder). The wiring notes are in the Phase 06 section of `docs/ecs-mapping.md`, or the list below:

- The movie thread and main thread must share one OS platform object and one thread ID numbering.
- `HostMoviePlatform.in_scene` is set by the renderer's owner, the `QueuedMovieDevice` receiver is drained on the main thread, and `movie_player_shutdown` runs at exit.

Phase 05 values the window test does not reach: renderer data from +0x60 on for the file slots, NiTArray capacity and grow, `ctrl=007F`, `packageBuild`, `flags0@B46988`, part of the L30PASS0 dump, and the globals that change after 007BA279 (some have end checks only).

**Unported static constructor:** `00A10EB0` calls `007B7250` before WinMain, which zeroes `00B42EC0..00B42F2C` and presets `B42EA7`, `B42E85`, `B42F40`, `B42F3F`, `B42E90`, `B42D70`, `B42E95` and `B42E87`. Neither repository has it, nor does the call ledger. `B42EA7` is cleared before its other readers. The other presets have not been checked for readers, so check them in the phase that reads them. Phase 00's constructor audit missed this one, so that audit may have other gaps.

Decisions made while porting:

- `TextureManager` lives in `bs::render_targets`; 007BA0F0 owns it through `ShaderManagerGlobals`, not as a resource.
- `Shared<T>` locks replace the original's unsynchronized input and TimeInfo globals, with lock order input then TimeInfo.
- D3DX answers are data from the file through the DDS reader, and shapes startup doesn't reach are host stops. `depth: 1` and `image_file_format: 4` are D3DX conventions, labelled as such.
- `AudioPlugin` stays off on the virtual display unless a test names its own `ALSA_CONFIG_PATH` (changed in M6), so that tests do not open the machine's audio device. `recomp_host::render_exit` holds an exit until pipelines drain, because exiting earlier crashes in the NVIDIA driver (status 139).

Other C++ address corrections: `texture_loading.cpp`'s comments name the wrong D3DX call sites (the calls are 00760EF5, 007610D3, 007610F9, 0076111E). These IDA names are owed in the sync.

Caps are in `fixtures/config/device-caps.txt`. D3DCAPS9 itself is fully recorded (the renderer dump in attempt 3), and five other seam lines are labelled assumed; none feeds a RendererInfo field. Fullscreen, the ATI probe and a successful NVCPL load have no test.

Where the Rust follows the executable and the C++ does not, each checked against the disk bytes:

- `00403960` joystick default bindings: every C++ index is 4 too low, and the +0x1BD5 store is missing.
- `kDepthStencilCandidates`: 00B29944 reads 70 first and 80 last; the C++ swaps them.
- 0076BEF0's 16-bit depth arm returns 0x49 or 0x50 (80), not the C++'s 70: `sbb eax,eax; and eax,7; add eax,49h` at 0076C291.

Expected values changed, each with its reason:

- `verify_queries` no longer expects `binding(2, 5) == 0`: that value pins the joystick bug above. The joystick press now uses the bound button, and the expected counts 2 and 1 are unchanged.
- The palette's world bound (M1 table above): 007AB564 runs 00707370.

A `MAIN` control-word row was removed from `phase06.rows` because nothing in the port can read it; P04-RUN-002's value is in its evidence file. `crt::gcvt` is written from documented MSVC behaviour and has no test or run behind it; it only reaches macro text on the compile path, which isn't reached while the package holds every program.

Kept on purpose: `0076A54D`'s `(100h)/(100w) != 75` truncates to 0, so the flag is always set in landscape.

**Settled (user, 2026-11-05):** the disk exe NOPs the timer pause calls at 0040616E and 004061AF and makes 0040F16E a plain jump. That is the user's own background-run patch, so it isn't part of the original's behaviour. The ported window procedure and its tests stay on the IDB's unpatched path, and the Bevy host stays running in the background by its own policy.

### M3: phases 07–11, UI, sky, TES construction, plugins, player

InterfaceManager, fonts, tile engine and LoadingMenu (P07, every tile value); Sky (P08); TES, DataHandler, two real task threads and water (P09); plugin selection (P10); the `0x804` player
and processes (P11). Rows: `phase07/08/09/10/11_expected.inc`. Also confirm the Phase 07 tile binary layout decision and the D6 atomic reads, which fixed a real data race.

### M3 progress

Host tests only: no validation bonus, and no phase is closed by them. **No WinMain driver or app runs any of this yet.**

| Batch | State | Checked by |
| --- | --- | --- |
| GMST store (Phase 07 D7) | Ported, with the table generated by `tools/game_settings_table/generate.py` | Counts, lookups, the 29 control settings and the duplicate rule, all compared with the C++ only |
| Phase 07 InterfaceManager, fonts, tile engine, LoadingMenu, movie gates | Ported | `phase07_window` (13 tests). **Against the original:** every tile row, value, flag and child count from the real XML, re-derived from P07-RUN-001's records; manager bytes (`+0x74` = `448B02BD` with plain sin and cos), the five fonts, faders, the LoadingMenu dump, hit counts, node records, material colours and both scene graphs. **C++ only:** the other aspect sizes and the safe-zone split, the action table, the menu mode stack, `progress`, the ribbon, text wrap and every failure path |
| Phase 08 Sky prelude | Ported | `phase08_sky` (2 tests) and 15 unit tests. **Against the original:** all 55 rows of P08-RUN-001, from the shipped NIFs, and P08-RUN-002's sun, glare and cloud geometry. **C++ only:** the 7 `007BDAC0` calls, the store order and the five failure arms |
| Phase 09 TES, Havok state, grids, task threads, DataHandler, plugin headers, water, BSTreeManager, world update | Ported | `phase09_tes` (9 tests). **Against the original:** all 491 rows of P09-RUN-001 over real `Data`, with every row read and corruption shown to fail, and 8 lines from its records. Counters compare as advances only, because the harness enters the window at lower values (3/2/5 against 0x12/6/0x21); the advances match. **C++ only:** grid normalization, the tint arm, Havok arms, allocation failure, file-list fixtures, water without a shader, the low render path, tree-manager arms and the store and thread-call traces |
| Phase 10 plugin selection | Ported | `phase10_plugins` (11 tests). **Against the original:** all 51 rows of P10-RUN-001 from the run's own INI and its full 211-line Plugins.txt, plus the 24-byte line buffers of its 11 matches. Three rows only hold constants (`locale_changed`, `root_tail`, `mode`), and the line count of 211 is the handoff's, checked against the pinned file. **C++ only:** every missing-file, fallback and line-rule arm |
| Phase 11 player, processes, character controller | Ported | `phase11_player` (16 tests). **Against the original:** all 43 rows of P11-RUN-001, including the player, process and controller images; the C++ read fewer of them. Havok SDK offsets are `??` in the controller image. **C++ only:** the manager arms, form arms, allocation failures and FOV at other angles |
| Phases 08 to 11 as one sequence | `game_data_window` | `00406390..004067B9` on one World with the real DataHandler and file list, which each phase test stands in for. It matches P10-RUN-001's file list and flags, and P11-RUN-001's form count and next form ID. Phase 07 isn't joined yet; it needs M6's host fixture |

Settled for Phase 07:

- **Tile layout:** the binary `Tile` layout stays a recorded substitution, since `phase07.rows` needs only names, kinds, trait IDs and raw values. Tiles are plain-data arenas in `TileEngine`, not entities: nothing queries them by component, and each observable order is an explicit list.
- **D6 still holds:** the sequence is the constructor, `publish_interface_manager` at 005821C7, then the initializer. The test shows all 10 font reads see no manager and `strings.xml`'s read sees it. This runs on one thread through the host seam. The real movie thread reads presence only, and its own test covers it. A missing `InterfaceMovieReads` is now an `Err`, because creating one would give the driver an unshared copy.

Where the Rust follows the executable and the C++ does not:

- `Menu +0x24` starts at 4, not 0: `c7 40 24 04 00 00 00` at 00584659, and P07-RUN-001's S 132 dump agrees.
- `00591C50` calls all of `Menu_RegisterTile`: `e8 d6 2b ff ff` at 00591CA5 (the two byte reads above are confirmed on the disk exe). The C++ stores only `+0x04`. The shipped XML can't show the difference.
- `005AD0D0` reads the `[LoadingBar]` weights and the slide delay. The C++ hardcodes them. This comes from the decompile, and no run covers it.

Phase 09's real threads only block on `+0x14` during the window, so no work or arithmetic runs on them. `BsTaskThread::stop_and_join` resumes, releases and joins; the original never joins, so that part is the host's. The handoff's 45 bhk registrations is wrong: the run and the C++ both have 43. Phase 09 found no C++ error against the exe.

### M3 review against the executable

The rows only cover what a run recorded, so after porting (user, 2026-11-05: errors here cause issues further down the line) independent reviewers read the disassembly against the Rust, aimed at unrecorded paths. Each finding was checked against the bytes before fixing. The coverage lists are in each reviewer's report; not every function in scope was read, and `0058DBD0`, `0058D2F0` and `005744E0` were read once.

**x87 precision (the widest finding).** The main thread runs at 24 bits after CreateDevice, so the original rounds every add, multiply and divide to float. The C++ and the early Rust kept `f64` and rounded once at the store. About 60 functions changed to per-step `f32` in the exe's grouping, across transforms, bounds, animation, interpolators and particles. No row covers composed transforms, so no row could catch this or confirm it; **the premise rests on P04-RUN-002's control word and the disassembly.** No row failed after the change. Trig results stay full precision (`fsin`, `__CIsin`), rounded when stored. **The Rust now differs from the C++ in the last bit for rotated objects, so the C++ is not a peer on those values, and the gate compares with rows.** Mixed-thread code has `TODO`s, and M4 must check any worker thread that reaches these helpers, since worker control words are unrecorded. **First evidence (13e):** P15-RUN-001's raw composed transforms for `health_bar01.nif`'s `r.0` and two lights match the 24-bit form bit for bit, where the 53-bit form misses 2 elements of each. **Still the first suspect if a later row of a rotated object fails.** If such a row fails, suspect this premise first and bring it back to the user; don't adjust the phase port to fit.** The move to full-precision trig at `007B70E0` and the particle sites also came from the disassembly, not a row disagreement; the user accepted it on 2026-11-05 and D6 now says so.

Grouping errors found by that audit: 00758B75 (drag dot), 0074A54D and 0074A5E1 (skin position and normal), and 0053D460's fraction, which the exe rounds. The two skin fixes have no dedicated test.

Where the Rust now follows the executable and the C++ does not:

| Address | Difference | Reached at startup? |
| --- | --- | --- |
| `0058CC60` | The limit is 00B13BC0 = 10000, not 4098, so only IDs below 4001 are rejected | Not on shipped XML (logged IDs: 4014..4019, 4042, 4043, 10025, 10184) |
| `005AD980` | The move-bar limits come from settings, the timer is float per tick, the clock wraps, and no stop on pending | Not wired to a driver |
| `00584980` | `float(ftol(deepest))` before the floor of 10 | Yes; differs only on a fractional depth |
| `005898F0` | Tests the top tile only | Differs only for a nested menu |
| `005AD780`, `005ADB40` | AdvanceSlide stores nothing on an empty list; 005ADB40 ported with its gates | With the driver |
| `00575B40`, `0067B1E0` | Signed glyph index; a zero line width takes no entry | Rare text only |
| `0058D1C0` | Parent store order; a released parent skips the count and removal | Order yes, no visible effect |
| `00591C20` | The menu builder finds no material on the node, so no revision bump or alpha 0 | Yes; no row covers it |
| `0058E959`, `0058B800`, `0058EE59`, `0058B61F`, `005ADEC0`, `005AD0D0`, actions 2023 and 2024 | Flag bit, repeated argument, `" "` filename, NaN zoom, exact `fild`, 24-bit products, double logs | Mostly unreached |
| `0065C9B0`, `006FAEE0`, `007B70E0` | A zero modifier on another code stops; NaN leaves the loop; full-precision trig with the reference taken as an unrounded double | Only `007B70E0`; 75 degrees gives the same bits |
| `008B9C1B`, `008BA092` | Phantom count 2 (the run's `02` is now compared) and state manager count starting at 1 | Yes; nothing reads them yet |
| `006F7030`, `007B7FE6` | NiShadeProperty answers -1; NiScreenElements is geometry | Not on the sky NIFs |
| `00B3F580`, `00B35C14` | A dword store; the live `fSeenDataUpdateRadius` setting, not a copy | Yes; same values here, but the copy could have drifted after GMST loads |
| `0049E652`, `0043F395` | `fistp qword`'s low dword; NaN takes the zero arm | Not on the default settings |

A Rust-only error: `0044FCFE` compared raw MAST bytes with a UTF-8 name, now `original_bytes`.

Reported by the reviewers as matching the exe: file ordering, flag bits, header offsets, all 69 form types, all 21 skills, the 43 bhk registrations, all 0x60 layer dwords, the filter constants and radii that no run recorded, the 223 tile tokens and all 26 `0058BEE0` actions. Reviewers also found these function groups partly unread: the Havok SDK bodies (declared substitutions), `0070B120`, `00731620..00731D80`, `00732D70`, `00722690` and `007227B0` in the sky reach; `0065BE70`, `0065BCE0`, `00658760` and the `00895190` arms not taken in the player reach.

Not settled: `%0.f` at 00592390 uses Rust's ties-to-even, and the CRT path wasn't read; no Phase 07 tile formats a number there. Host `ln` and `log10` last bits against the CRT's SSE2 paths. `00986000` selects an SSE2 path on a runtime flag, unestablished. x87 intermediates can leave float's exponent range where `f32` can't, so very large or denormal values could differ.

Some new comments say "the C++ differs" from grep at the site only; those at 005898F0, the section sum, 005AD780 and 005ADB40 weren't confirmed against the C++ body.

Open from Phases 07 to 11:

- **Not wired for the driver (M6):** `construct_actor_process_manager` before WinMain, `InterfaceServices.player_present` from `the_player_b333c4`, the tile engine's `random_next` sharing `ActorProcessGlobals.random` (00B069C3), and D4's `Phase-10:` and `Phase-11:` log lines.
- `0044F3D0` is the first reader of the Loaded bit. Later phases should re-scan the static constructor presets; the nearest hits are `00406AC6` and `00406B29`, outside the Phase 11 closure.
- Phase 11's rand draw `6C3F` is injected: the seed is a fit, not a record.

- **The first driver will hit `00432860`'s worker pause on the shader-off arm**, as a host stop: the main thread isn't `+0x30`, and the test fixture answers with the current thread, as the C++ test does.
- `WaterTextureServices` has only a test fixture. A driver must bind `BsTexturePalette`, FileFinder and `00701E00`. Readers of Phase 09's globals (`SceneGraphInputs.tes_present`, `004814C0`, the LoadingMenu's `data_handler_present`) aren't switched over.
- `DataHandlerLists` models only `globals_74`, so `B dh lists=zero` proves nothing for the other 22. M1's bounded `TesFile` sits beside the field-exact `TesFileData` until Phase 12 folds them.
- File attributes are synthesized (0x20 or 0x10) and FILETIME comes from mtime; the rows match bit for bit. Compressed TES4 headers and the resolved-master arm of `00450080` are host stops.

- **Static constructor presets:** nothing Phases 08 and 09 reach reads them (direct-address operands only). `B42E90` enters unwritten, with 14 reader functions including `007BD3D0`, `007C9230`, `007C9F30` and `7DCC0B` in the WaterShader class, so whichever later phase reaches one must port the preset. It's a Phase 00 gap until then.
- `direct_image_info` returns `None` for `cursor.dds` and the ribbon DDS, and the gate hasn't been found. The Phase 07 test reads the DDS header itself, as the C++ harness does. M6 will hit this when it loads menu textures.
- The Phase 07 window is tested at 1920x1080 only, and the 1264x681 layout goes with the host projection to M6. Three objects show an empty name in the run's dump while the C++ names them, so that check skips empty names. The glyph fixup bytes, the full-screen cursor branch and `005AD980`'s 2500/100/10 constants aren't verified.
- The sky test's SkyShader slot is a stub with the right vtable, and real slot 10 from Phase 05 hasn't been joined to this window. After a successful load the emptied NIF root stays as an unparented entity, since refcounts aren't modelled.
- `fixtures/rows/phase07_window.rows` and `phase08_run002.rows` are hand-lifted and not pinned to the `.inc` files. They're checked at test time against pinned evidence.

### M4: phases 12–13, the data load

`0044F3D0`: 11 files, 103,510 forms. This is the largest and most valuable milestone, since all later phases sit on it. Rows: `phase12` (250 + 49 + 301 samples and 23 list FNVs), `phase13`, `phase13e`.
Check the load's run time under opt-level 1 early. The C++ Debug load took tens of seconds (`app_startup` allows 600 s). If opt-level 1 is too slow for routine tests, give the parsers a per-package override and record it.

### M4 progress

Host tests only: no validation bonus, and no phase is closed by them. **No WinMain driver or app runs any of this yet.**

| Batch | State | Checked by |
| --- | --- | --- |
| 12a file pass: validation, compiled file array, record loop as a resumable cursor, `0044DCF0`'s frame, TES4, GMST and SKIL arms; `form_components`, compressed records, `EditorIdMap`, form type map | Ported | `phase12_load` (20 tests). **Against the original:** compiled order, flags, HEDR words, `+0x8C0`, `+0x8D0`, `00B33A90`, `00B33A94`, the first percent and 109 of the 2,996 rows. **C++ or disassembly only:** every synthetic arm (missing master, newer version, wrong type, partial form, compressed failures, TOFT). Every one of the 58 form map rows checked against exe bytes |
| 12b base objects: items, misc, script (A); magic, actors (B); AI, environment, region (C) | Ported | `phase12_load`, `phase12_magic_actors`. **Against the original:** 2,532 of 2,996 rows after the merge, with comparisons of the first bytes behind pointers beyond what the C++ test made. **Exe or file only:** synthetic chunk arms, and the region sound and grass lists, which were checked against a scan of the shipped files with no row behind them. POST and PLIST rows wait for 12d |
| 12c world, references, extra lists, dialogue | Ported | `phase12_world` (21 tests), `phase12_dialogue` (11 tests). **Against the original:** COUNT, LIST, SAMPLE and AC20 rows, bringing the total to 2,723 of 2,996. These types have no DATA rows, so nothing behind a world form's pointer is compared; see `docs/open-items.md`. The player sample is against a harness stand-in until 12d |
| 12d `0044AC20`, every InitItem, post-load tail, conditions | Ported | `phase12_load`, `phase12_post_load`. **Against the original:** every line of `phase12.rows` (2,996), including 307 POST rows (23,880 bytes exact, 2,976 pointers by null-ness) and PLIST except the junk `PLIST BC`; RUN-006's 56 `004F1AC0` calls. **Exe only:** warning arms, inventory and the teleport and DistantLOD arms. The tail takes 1.21 s and the file pass 0.29 s at opt-level 1 |
| 13a to 13d preload, player base and checks, scripts and sound, FPS clamp | Ported | `game_data_window` runs `00406390..00406944` as one sequence with the real load, plus `phase13_tail` (13 tests). **Against the original:** 3,062 of 3,064 rows, including 26 tasks field by field, all 3,010 scripts and exact `PASSES container=967`, `script_runs=4153`; `CR` order for 160 shared creations. **Not compared exactly:** roll-dependent `create` and `item_runs`, since the seed is a harness value. No Phase 13 row holds a rotated transform. The whole test takes 1.67 s |
| 13e model drain | Ported, on the main thread as a recorded substitution (the plan's rule), resumable per task | `stream_graph_real`, `game_data_window`. **Against the original:** every `phase13e.rows` line (7 `MODEL`, 86 `NODE`, with one declared difference at `0080B190`), `MODELMAP` counts and every entry's bucket and key from the walk, all of `phase13.rows`. The `Phase-13e:` line matches `fixtures/cpp-baseline` exactly. 15 tasks run and 11 stay gated. 6.86 ms |

**Everything still owed is in `docs/open-items.md`**, including every value no run has checked.

The file pass over all 11 files with every form arm skipped takes 0.065 s at opt-level 1 on a warm cache, and 0.29 to 0.32 s with every class in, so no per-package override is needed. The run visits 114,808 of the 1,277,335 records (8%), because 0044F0C0 skips the master's exterior groups; that's the original's behaviour. Re-measure when 12d is in: `--ignored --nocapture measure_the_file_pass_with_every_arm_skipped`.

Where the Rust follows the executable and the C++ does not, in 12a: `0046BDA0`'s TESQualityForm tail goes through `fistp` and `fild`; `004504B0` treats a NaN version as not newer; `0046DB98` and `0046BF1F` copy through x87, which quiets an sNaN; `0044F2EE`'s `fild` keeps the counter exact; several warning texts were cut short (`00A38E18`, `00A38EB0`, `00A38E78`, `00A38DE0`, `00A38EF8`); `0052EAF0` and `0046C640` clear flag 2 where the C++ stops; `00442617` clears `00B4310C`.

Where the Rust follows the executable and the C++ does not, in 12b. None changes a shipped result, since all rows passed:

- **Family A:** STAT's chunk is `DMTL` at `004B9BB0`, not `DOTL`; a DOOR TNAM of 0 adds nothing at `00446CB6`; FLOR PFPC stores the whole dword at `004AE11D`; TREE `+0x6C` stays unset at `004B9E59`; SBSP DNAM goes through `fistp`; a short BMDT keeps its bytes.
- **Family B:** the MGEF constructor reads two GameSettings at `00416512`, `00416520`; NaN stays at `005281FE`; `00527F47`'s 23-byte NPC DATA and `00519A31`'s CSDF exist; `005255A0` picks `+0x168` or `+0x108`; a short ACBS leaves the rest of the fields; `004168C5`, `004162A8` sign-extend; `0041558D` warns with the EFID's setting and `00415593` prints "(null)"; `00416286` leaves a freed pointer with count 0; `0045B670` is a no-op when `+0x185` is clear or `+0x184` is set; `0046D940` gives RACE's member models a null owner.
- **Family C:** `004ABC9E` keeps a NaN CSTD value; `004A6B2B`'s segment test has NaN arms; `00B34D88` can't fire, since all 18 references read, save or clear it and `004943A0` returns 0 (static evidence only); PLDT and PTDT out of range copy nothing where the C++ throws; a short CTDA keeps `0056A9C0`'s default; `00435830` has a first arm the C++ misses, returning a reversed `SpecialIdleAnims` list.

The effect settings table has no errors: all 161 rows match the pushes.

Open from 12b:

- **`DataLoadHost::find_matching_files` (00431970) is a real substitution**, defaulting to an empty list. Each idle root's `+0x20` is null in tests where the original has a list, and no row reads it. 00520FA0 in 12d reads it, so the driver has to implement it.
- **No valid row covers the `+0xBC` region list's order:** `PLIST BC` is junk, as the run walked the wrong head. It rests on the C++ and the `0044DCA8` byte table.
- `IdleAnimRoots` is insertion-ordered where the original has 37 buckets; 00520FA0 walks the buckets, so 12d must settle it. `TesWaterForm.related_a0` is `[u32; 3]`, and POST 42 shows pointers.
- TESFlora's produce part has no row. The APPA samples may not tell `0046BDA0`'s exe form from the C++'s; a synthetic test is the only sure discriminator.
- `BOUND` hashes `+0x00` across all families and is still unread.

The shared `x87::u32_to_x87_float` rounds to float first, which differs from `fild` above 2^24; `record_percent` avoids it, and other callers haven't been checked.

### M4 close

**M4 is ported on host tests.** `00406390..00406944`, all of `00406360` after the prelude's scene graphs, runs as one sequence on one World with the real load, then the model drain. Every line of `phase12.rows` (2,996), `phase13.rows` (3,064) and `phase13e.rows` (93) is read. Read but not fully compared: `PLIST BC` (the run walked the wrong head), roll-dependent `create` and `item_runs`, and one `NODE` row's `0080B190` difference. The whole load test takes about 1.7 s at opt-level 1, so no per-package override is needed.

Five independent reviewers then read the exe against the Rust, including the helpers every family reused. They found nine defects, all fixed. The porting agents had already fixed many more C++ errors while reading the exe, listed in `docs/open-items.md`, section 6 and earlier in this section. One the startup can reach depending on rolls: `0041E4D0` merges two ExtraLeveledItem stacks whatever their index. It didn't change the host's roll counts. `docs/open-items.md` holds everything still owed.

**What this doesn't show:** no driver or app runs any of it, so G3 is only the `Phase-13e:` line so far. No original run was made. 11 of 26 model tasks stay gated. Much of what the load computes has no row behind it, listed with the capture that would check it in `docs/open-items.md`, section 1.

### M5: phases 14–16, world setup and placement

`phase14` is the largest fixture (1.66 MB: all 1,998 actors, 157 draws replayed, 5,487 change entries with invariants), then `phase15` and `phase15d` (25 cells, 1,609 references; interior 639), then `phase16`
(no `.inc`; lifted in M0.2). The 47 condition functions and 176 functions of 14c live here. Port `bind_real_arms` and the recording arms, so that all of the Phase 15 INI paths remain testable.

### M5 progress

Host tests only: no validation bonus, and no phase is closed by them. **No WinMain driver or app runs any of this yet.**

| Batch | State | Checked by |
| --- | --- | --- |
| 14 world setup, archives, change records, NPC values, the AI package pass and 42 condition handlers | Ported, one sequence | `phase14_world_setup`, `phase14_ai`. **Against the original:** every line of `phase14.rows` (4,667): all 3,996 `ACT` rows with 21 `proc` words where the C++ compared 5, RAND from the stream, `CHG` exact apart from 70 or 71 roll-moved keys; RUN-001's raw records (44 and 45 idle roots, TES bytes at three points, 36 LOD entries, archive headers, 12 NPC images); RUN-005's 158 draw subjects. Several exe corrections pass reverted too, so no row sees them; `docs/open-items.md` lists them. The window test takes 2.4 s |

| 15 initial area, plugin re-read, master cell load, exterior grid, interior enter and exit | Ported, with `RealInitialAreaArms` and recording arms; every INI path runs on a fresh game | `phase15_initial_area` (10 tests). **Against the original:** all 1,798 + 647 rows and RUN-001, 002, 003, 004, 005, 007, 009's records; 86 `NO` objects' transforms and bounds; the `Phase-15:` line matches the baseline. All cell 3D is a counted boundary, so no reference has a world yet. **Exe only:** `docs/open-items.md` |

| 16 player placement | Ported | `phase16_place_player` (8 tests). **Against the original:** all 1,681 rows lifted to `phase16.rows` (`tools/lift_phase16_rows.py`), each proven present in its raw record, with some keys read but not compared. **G2:** the three paths match on the host tests: `Rect {2048, 2048, 128}`, one new change record by delta, controller `+0x320`, `+0x1F4`, the land height 530.77 and RUN-002's percents (with scripted counts). **Not absolute:** change counts differ on every path, and player position, `b3c`, `p5d0` are set aside at `end_cell` as boundary-owned. The `Phase-16:` line matches the baseline as a test assertion only |

### M5 close

**M5 is ported on host tests.** `00406390..0040EF23`, WinMain through placing the player, runs as one sequence on one World on a fresh game per path. Every line of `phase14.rows` (4,667), `phase15.rows` (1,798), `phase15d.rows` (647) and `phase16.rows` (1,681) is read, though some keys are read and not compared. **G2 matches on the host tests for all three paths, with these limits:** one new change record holds as a delta only, since absolute counts differ on every path; player position, `b3c` and `p5d0` on the cell paths are set aside as boundary-owned; RUN-002's percents use scripted counts. G3 holds for the `Phase-13e:`, `Phase-15:` and `Phase-16:` lines as test assertions; **no driver or app prints or runs any of it**, and all cell 3D is a counted boundary.

Four independent reviewers then read the exe against the Rust, including the 176 AI functions no one had second-checked, and found eight distinct defects, all fixed or made into stops; a scan they prompted found two more settings. None changes an outcome on the shipped startup paths, as far as those reviews read. **A pattern to keep checking:** settings held as detached fields the load never set (`00B09E28`, `00B14E98`, `00B05DC0`, `00B10CA0`). `00B14E98`'s image value is 1, and the port held 0.

The x87 premise has run evidence from a recorded composition (`health_bar01.nif`, 4 worlds, 2 discriminating) and one percent at `00434108`, and nothing against it. A rotated reference's world transform still hasn't been computed anywhere, because cell 3D is a boundary. Everything still owed is in `docs/open-items.md`.

### M6: the host comes together

The real Bevy app runs the whole sequence: LoadingMenu during the load, then MainMenu with movie, music, input, Load list and prompts, emitting the G3 log lines. Host tests (`ui_main_menu`, `ui_loading_menu`, `audio_assets`, the
`app_*` probes) port or get replaced, one by one, on a list. `d3d9_*` tests are retired as a recorded substitution. Capture commands reappear as Bevy screenshot flags.

**Progress:**

- **Driver: done.** `recomp_startup::winmain` runs `0040DF80..0040EF54` in order as an exclusive chain, with resumable load and drain, `PhaseOutcome`, one rand source (`--seed`), one finder, archives and palette, real Phase 05 slots. G3 passes on both C++ logs, in process and on the binary, tolerating Bink sound (P06-RUN-001/002 record 1, C++ prints 0), x86's `adopted`, roll-moved counts, doors (C++ 5256 counts every `0x18` reference; `006800B4` gates it to 4567), and editor IDs. **That last one is a real miss against P12-RUN-001 and is open:** `docs/open-items.md`, "The 25 editor IDs". No stop on the default path, `sStartingCell` or coordinates path over 40 seeds each. `WinMain` read in full; 847 tests.

- **Movies, music, raw input: done on host tests.** `FfmpegBink` plays real frames on the real movie thread; `winmain_menu_media` runs `005B5F20`'s media, `0040F092`'s wait polled, title music, logo and backdrop past `Block::Finished`, in exe order; `MusicDevice` over `bevy_audio`; `feed_raw_input`. Nothing has been heard through a real device. C++ errors fixed from bytes: Bink sound (`00410324`), hard-coded title music and loop (`005B5B2F`, `006AA1F4`, `006AD950`, `006AD9AF`), the logo never played (`0040F0BE`), no menu-owned movie state (`004105F6`).

- **Screens: done on host tests, with gaps.** The real `TileEngine` builds and draws the LoadingMenu, MainMenu, Load list and prompts through Bevy, with the real movie underneath and scripted input. G5's comparison ran against the C++ captures and isn't a pixel match: 1 to 2 px offsets only the original can settle, and the menu numbers are contaminated by a different movie frame. The LoadingMenu never appears in a normal run, because the whole load runs while the intro thread runs and `00579260` skips the draw, as the exe does.

- **Review: done.** Two reviewers read the driver, media and screens against the exe. 5 defects in the driver and media plus 7 follow-ups, 15 groups in the screens, all fixed. The ones that reach past M6: `005AD440` draws 191 `Random_Next` during the load, moving every later roll; `00B06704` is the INI's `fGlobalTimeMultiplier`; `006AE860`'s gates; a presented picture lost when its movie closed; one wheel notch scrolled 3 rows; Escape was never read. The builder's "100%" claims were the weak spot: the 559 addresses in the M6 files are all heads, but several functions at 100% weren't.

**Host tests, one by one:**

| C++ test | Rust replacement, or why it's retired |
| --- | --- |
| audio_assets: tes4title probe | `real_assets_decode_with_their_formats`, `title_music_decodes_whole`, `title_music_matches_ffmpeg` |
| audio_assets: focus wav, voice mp3 | `real_assets_decode_with_their_formats` |
| audio_assets: UI ids 1, 2, focus variant, unknown silent | `ui_ids_map_to_their_assets` |
| audio_assets: fade boundaries | `fade_midpoint_and_end_boundary` |
| audio_assets: output gain | `mute_focus_and_bus_gain` |
| audio_assets: missing BSA lookup | `a_missing_archived_file_is_a_failed_lookup` (asserts the open fails, not the probe) |
| audio_assets: truncated WAV | `a_truncated_wav_fails_without_a_panic` |
| `--audio-probe` | `the_app_plays_the_menu_s_movie_and_music` |
| phase06 `verify_splash` | `a_splash_decodes_draws_and_waits` |
| `verify_local_stub` | `the_intro_sequence_plays_on_the_real_thread`, `a_missing_decoder_is_a_null_open` |
| decoded playback | `map_loop_plays_as_the_menu_owns_it`, `map_loop_wraps_after_its_last_frame` |
| app_startup | `tests/app_driver.rs` |
| app_startup_cancel | **Not ported:** needs the window's close handling |
| app_startup_failure | `a_missing_data_folder_*`, `a_missing_data_folder_exits_1_and_says_why` |
| app_menu_probe, app_menu_render, app_message_input, app_load_input | Capture flags (`--menu-capture`, `--menu-capture-selected`, `--message-capture`, `--load-capture`, `--loading-capture`, `--intro-capture`) and `tests/screens.rs` |
| d3d9_smoke, d3d9_shader_pipeline, d3d9_texture_upload, shader_probe | Retired, recorded substitution: no D3D9 device exists |
| ui_loading_menu: `progress_tests` | `loading_menu::tests::progress` |
| ui_loading_menu: `StartupWork` | Retired: the host's job tracker; the driver's blocks replace it |
| ui_loading_menu: XML geometry, string, ribbon at user3 50 | `the_loading_menu_lays_out` |
| ui_loading_menu: both layouts | `loading-1264.png` capture only; **no test at 1264** |
| ui_loading_menu: missing assets rejected | **Gap**, with the missing `Data` exit |
| ui_loading_menu: asset export | Retired: a tool, not a check |
| ui_main_menu: `load_menu_test` | `every_save_gets_a_row`, `the_load_list_has_a_row_per_save`. Scroll-offset checks retired: the C++ virtualized six rows, the exe makes all and clips |
| ui_main_menu: corrupt, truncated, no-image saves | `save_files::tests::the_list_keeps_what_it_can` |
| ui_main_menu: `still_matches` | Retired: host-only, the exe doesn't re-check |
| ui_main_menu: `synthetic_bsa_test`, `compressed_asset_test` | `recomp_game` archive tests, not compared check by check |
| ui_main_menu: `save_index_test` | `the_newest_comes_first`, `captions_follow_the_file_name` |
| ui_main_menu: `tile_trait_test`, DOM `real_asset_and_tree_test` | Retired with the DOM |
| ui_main_menu: `font_and_scene_test` | `the_draw_list_holds_the_screen`, weaker: no per-glyph checks |
| ui_main_menu: `movie_asset_test` | Left to the media tests; **not compared** |
| ui_main_menu: `menu_test` | `the_menu_lays_out_from_its_assets`, `no_saves_narrow_the_layout`, `no_saves_hide_continue`, `saves_show_continue_and_the_version`. **Gap:** backdrop transform |
| ui_main_menu: `failure_test` (malformed XML) | **Gap** |
| ui_main_menu: `recreation_test` | `a_second_create_replaces_the_menu` |
| ui_main_menu: `input_and_actions_test` | `the_pointer_clicks_a_button`, `new_game_prompt_says_yes_and_no`, `exit_asks_and_quits`. Partial: no key-repeat or focus-lost checks |
| ui_main_menu: `message_menu_test` | The prompt tests. Partial: no wrap or button-width checks |

**M6 close, on host tests:** 968 passed, 0 failed, 3 ignored; fmt and clippy clean; 93 fixtures match. G3 passes with tolerances listed in the driver note. G5's comparison ran and isn't a pixel match; G4 isn't checked; G1 to G6 run together in M7. Nothing was checked against a new run of the original, nothing was heard, and no real input or real save has gone through it.

**The user's own run (2026-11-05), real display, real data, own INI:** menu, looping movie and music, right volume, button sounds, keyboard, Escape on the Load menu, and real saves with right data and pictures all work. This is by eye and ear on the host; it isn't an original-validation result.

### M7: run the gate, cut over

Run G1–G6 with `tools/side_by_side.py`, which prints a table per phase: oracle rows matched, C++ recorded result, Bevy result, tolerated differences. Publish the result in `Startup-Roadmap.md`.

Then, on your go-ahead: freeze the C++ repository as read-only reference, change the roadmap's "Current" sections and the C++ `AGENTS.md`/`CLAUDE.md` to point here, close D4's sign-off items, and write the Phase 17 handoff.

**Cut over on 2026-11-06, on the user's go-ahead.** The C++ repository is frozen at `cpp-frozen-2026-11-06` (`6fe9ffc`). It can't be made read-only on `ntfs3`, so that's a tag and a notice. The roadmap and its D4 items point here, this repository writes the call ledger, and the Phase 17 handoff is written. D4 closes the format and lint item only: phases 06 to 16 stay "ported", not signed off.

**Gate result (2026-11-06, commit `9408ca5`), host tests 970 passed, 0 failed, 3 ignored: the gate does not pass.** G2 and G3 pass, G3 with named tolerances. G1 fails on the editor ID count and on rows no test compares. G4's verdict was undercut by a first run of the original, which reached seven addresses it accepted as unreached; those are decided now (`docs/open-items.md`), and its 203 reached stops aren't sorted. G5 isn't a pixel match; the LoadingMenu's absence from a normal run is fixed. G6 fails on labels (1,059 source functions), with fmt and clippy clean. `tools/side_by_side.py` prints the table; the roadmap has it. The cutover followed the same day. Phase 17 waits on the user.

**Later the same day, on host tests only:** G1 passes, with 984 tests, the editor ID count fixed and the unnoticed rows and values compared or declared (`docs/open-items.md`). G2, G3 and G4 pass. G5 passes by the user's decision, against the original's own frames, with the logo open, and G6 passes with labels from one scripted pass (`docs/open-items.md`). **So all six read PASS on one commit, which isn't the same as finished:** the mutation passes sample, declared rows are excused rather than checked, G5's logo is open, and nothing here earns a validation bonus. The cutover followed the same day. Phase 17 waits on the user.

### Phase 17 and onward

WinMain `0040EF24..0040EFEE`, through `00578CD0` → `0057D480` → `005A6B00`: first-person node work on valid cells, player script and 3D state, scene-child transform and update, `hud_main_menu.xml` with sections 50 → 5 → 100.
Past `0040EFF3`, before the menu (traced 2026-11-06, `docs/open-items.md`): `0040EFFA..0040F04A` selects the starting spell, which queues `restoration.NIF`; `0040F050` PlayerCharacter_SaveInitialState; `0040F05A`, `0040F05E`; and `0040F078..0040F088` closes the LoadingMenu, through `00404C60` and the purge. The no-cell jump `0040EF08` → `0040EF54` skips `0040EF24..0040EF53`. It needs fresh original runs, which need the Linux debug setup below. Carried items that are likely to come due: the player's region refresh, `00438060` and the 3D load, the drain's `00433A40` / `00433590` bodies, and the `00517330` script interpreter.

## Linux debug setup

Done, 2026-11-06: `docs/original-debug.md`.

## Risks

| Risk | Why it is credible | Mitigation |
| --- | --- | --- |
| Scale | About 70,000 lines of game code with order-sensitive logic | Port per phase with rows from the first day, so drift appears at once |
| ECS changes order or behaviour (D3) | About 900 C++ classes get split into components; 1,500 smart-pointer lines become entity links; destruction order may be observable | The fixed rules above, the M0.6 slice first, and rows from day one. Keep original store and release order at recorded sites |
| ECS slows the load | 103,510 form entities with many component types; every link becomes a lookup; our code is at opt-level 1 | Measure in the M0.6 slice and early in M4. If needed, raise optimization for specific crates and record it. Use spawn batches only where order stays intact |
| Hidden dependence on MSVC or Windows | The audit found no in-tree LCG, and `rand`, CRT parsing and `FindFirstFile` order all come from outside the tree | M0.3 primitives first; directory order handled in M1 |
| Trig last-bit drift | Rust uses the platform math library | D6 |
| Bevy version churn | 0.20.1 is pinned by `Cargo.lock` | Stay on it through the gate; upgrade afterwards as a separate change |
| Debug-build file size on ntfs3 | A sibling project hit the volume's limit | Profiles already use line tables and no dep debug info |
| C++ can't run again | No MSVC here; debug DLLs absent | Baseline archived in M0.4; oracle rows don't need it |
| Stale notes | The checkpoint is stale since Oct 6; `docs/startup-loading.md` predates the ports; some roadmap links are dead | Trust order: IDA and `records.txt`, then the roadmap, then older notes |
