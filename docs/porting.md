# Porting a C++ module to this repository

Read `AGENTS.md` first, and `docs/ecs-mapping.md` for what has been mapped so far. The C++ source is at
`../OblivionRecomp` and is read-only. Good examples to copy the style of:
`crates/recomp_game/src/gamebryo/properties.rs`, `extra_data.rs` and `scene_node.rs`.

## The port is exact, not a redesign of behaviour

- Keep the original's control flow, store order, raw values, field widths and failure behaviour. Keep every
  `IDA:` address comment, and every `Completion` score and `TODO: [ ]` that still applies. Do not improve, simplify or
  "fix" arithmetic. If the C++ does `static_cast<float>(static_cast<double>(a) * b)`, the Rust does the same.
- Numeric helpers live in `recomp_core::x87`, `crt` and `hash`. Use them instead of local copies. `as` saturates in Rust
  where the C++ cast does not, so use `x87::ftol` and friends for those conversions.
- Float fields read from a stream keep their raw bits: `ByteReader::f32`. Text may be non-UTF-8: use `Vec<u8>`.
- Do not port host stubs that exist only for hand-built C++ fixtures unless a kept test needs them.
- Do not port Direct3D, DirectInput, DirectSound or Bink calls as calls. Follow "The host boundary" below.

## ECS mapping

- A stream object is an entity. Its class tag is a `NiClass` value, and its state is components named after the class.
  Each component field carries its original offset in its name where one is known (`flags_18`).
- A C++ `shared_ptr` or `weak_ptr` to another Ni object becomes an `Entity` or `Option<Entity>` in a component, and a typed
  check against `NiClass` where the C++ checked the variant. `Entity` is never saved, printed or compared with evidence.
- The C++ `*Pending` structs stay: parse functions are pure, taking `&[u8]` and the stream version and returning the pending
  value with `bytes_consumed`. Link functions take `&mut World`, publish nothing on failure, and mirror the C++ preflight.
- C++ `invalid_argument` becomes `Err(StreamError("same message"))`.
- Ordered code takes `&mut World` directly. No `Commands`, no query iteration where order is observable; keep explicit
  ordered `Vec`s or `PointerList`. No `HashMap` iteration where order can be seen.
- A pure algorithm over plain data stays a plain function with no ECS in it. That keeps the C++ test portable.
- Globals and singletons become a `Resource`, never a Rust `static`.
- `crates/recomp_game` may use `bevy_ecs`, never Bevy rendering, audio or windowing.

## The host boundary (M2 onward)

Phases 04–06 are mostly device, input, sound and movie calls. Nothing in `recomp_game` or `recomp_startup` touches wgpu, winit, an audio device or FFmpeg.

- **State is data.** Caps, render states, render target records, shader slots, pass tables, input buffers and the DirectSound-shaped state keep their original values, widths and order in plain structs and resources.
- **A device call is a trace entry.** Where the C++ issues a call in an order a test can see, append to an ordered `Vec` in a resource or return value, with the original address, the call and its arguments, as the C++ `store_trace`, `state_calls` and `trace` do. Keep each failure branch: results come from a trait or a closure resource that tests can script, in the same place as the C++ `*Platform` and `*Services` seams.
- **Keep Windows types as plain numbers.** Use `u32` for `HRESULT`, `D3DFORMAT`, `DWORD` and flags, with named constants of the original values. A handle is a host value (`Option<u32>` or a newtype), never printed in comparison with evidence.
- **Device caps come from `fixtures/config/device-caps.txt`** and nothing else. wgpu's limits never feed game state. See that file's header for which fields were recorded and which are assumed.
- **x87 precision is a choice per site.** The main thread runs with control word `027F` (53-bit) until `CreateDevice` and `007F` (24-bit) after (P04-RUN-002, `0049851E`); the movie thread stays `027F`. At a 24-bit site every `fadd`/`fsub`/`fmul`/`fdiv`/`fsqrt` rounds to float, so plain `f32` arithmetic in the instructions' exact grouping is right; order of additions matters. `fld` itself does not round. A qword constant is the exact value rounded once, so `(f64::from(x) * C) as f32` is right. `__CIsqrt` keeps the precision bits (00991B15), so at 24-bit it rounds to float. Trig and other CRT math (`fsin`, `fsincos`, `__CIsin`, `__CIcos`, `__CIacos`, `__CIexp`) go through `f64` and round once at the store (decision D6). At a 53-bit site use `f64` intermediates and `x87::stored_float`. Say which in the comment when the site is not obvious. Follow the disassembly, not the C++ port's arithmetic: its `f64` chains were wrong at 24-bit sites.
- **Lift hand-written expected values.** A value in a C++ phase test that came from an original run goes to `fixtures/rows/phaseNN.rows`, one row per line with the run ID, and is checked against the matching file in `fixtures/evidence/`. A value made up for a fixture stays in the test.
- `AdjustWindowRect`, `GetClientRect` and similar host calls whose result a run recorded take the recorded value through the seam.

Phase 06's checks that take a device in C++ port as data, with a scripted movie source: tile sets, play movie objects, sequence, continue test, thread, tile allocation failure. The texture they create is a trace entry. Splash drawing, decoded playback frames and the local-stub movie file go to M6; list them in your report.

## Layout

File by engine layer and subject: `crates/recomp_game/src/gamebryo/<subject>.rs`, keeping the C++ file's name. Add the module to
`gamebryo/mod.rs` (kept sorted). Add `NiClass` variants in `stream_objects.rs`, and a cold-object arm to `spawn_cold` in
`registration.rs` for each class with a factory constructor. Those three shared files will be merged with other work, so change
only the lines you need there.

## Tests

Port the C++ test file's assertions into a `#[cfg(test)] mod tests` in the module, or `crates/recomp_game/tests/<name>.rs` for
ones needing real data. Keep every check, with its expected values unchanged. If a check truly cannot apply (it tests a C++ variant,
`shared_ptr` lifetime or a removed singleton), drop just that check and list it in your report with the reason.

An expected value failing is a bug in the port until proven otherwise. Do not change an expected value to make a test pass. Compare
with the C++ and, if needed, report it as an open question.

Run tests inside the virtual display, with a state folder unique to your work:

```sh
python3 tools/virtual_display.py start ${XDG_RUNTIME_DIR}/oblivion-virtual-NAME
python3 tools/virtual_display.py exec ${XDG_RUNTIME_DIR}/oblivion-virtual-NAME -- cargo test -p recomp_game FILTER
python3 tools/virtual_display.py stop ${XDG_RUNTIME_DIR}/oblivion-virtual-NAME
rm -rf ${XDG_RUNTIME_DIR}/oblivion-virtual-NAME
```

Never share a `CARGO_TARGET_DIR` between worktrees. Cargo hashes workspace crates by their path relative to the workspace root, so
worktrees sharing one overwrite each other's crates and can link another worktree's code with no error. Each worktree keeps its own `target`.

Set `OBLIVION_DATA_DIR=../OblivionRecomp/Data` when a test reads game data. Name the test or module you run; `recomp_core`, `recomp_game` and `recomp_startup` are enough unless you work on the window. Before finishing, run
`cargo fmt --all` and make `cargo clippy -p recomp_game --all-targets` clean for your files.

## Reporting

End with: files ported and their line counts, tests ported and passing, each check dropped and why, each boundary left as a host stop or
TODO, any new class links to families not yet ported, and anything in the C++ that looked wrong. Do not edit `docs/migration-plan.md`,
`CLAUDE.md` or the notes; report what they should say.
