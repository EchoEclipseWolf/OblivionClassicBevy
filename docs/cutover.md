# Cutover: applied 2026-11-06

The record of what was done, on the user's word, from `docs/migration-plan.md` M7. C++ commit `6fe9ffc`, tag `cpp-frozen-2026-11-06`. The line-ending noise was left alone, and the steps read as they did when planned. The notes' originals are in `~/oblivion-capture/cutover-backup/`. Each step says whether it can be undone. The gate read PASS for G1 to G6 at `c194704`; see the limits in `docs/open-items.md`.

## Decisions for the user

1. **Go ahead with steps 1 to 6?** Step 1 is the only one that reaches the server.
2. **The C++ working tree's line endings.** `git status` shows 580 modified files, from the Linux checkout: `.gitattributes` and the rest differ in CRLF only. Ignoring line endings, the one real change is 4 lines of `CLAUDE.md`, which step 1 commits. Recommended: leave the noise alone, since a frozen tree needs no cleanup. `git checkout .` would clear it but rewrites 580 files on the Echo drive.
3. **"Read-only" can't be enforced on this filesystem.** The repository is on `ntfs3`, so `chmod` won't hold. Recommended: a tag and a notice at the top of its `AGENTS.md`. The server could refuse pushes, but that's the user's own setting on `10.0.0.4`, so it isn't in these steps.

## Steps

### 1. Freeze the C++ repository (a tag and a commit, pushed)

Tag `cpp-frozen-2026-11-06` at the new commit. **Undo:** a pushed commit stays in history unless force-pushed over; a tag can be deleted. Commit only `AGENTS.md` and `CLAUDE.md`, and write their blobs with the repository's own line endings, since `git add` would commit CRLF:

```bash
cd ../OblivionRecomp
```

Build each file from `git show HEAD:<file>` plus the text below, `git hash-object -w`, `git update-index --cacheinfo`, then `git commit` with nothing else staged. Then put the same text in the working file with its CRLF, so the tree agrees.

**`AGENTS.md`: insert at the top, under the title.**

> **Frozen 2026-11-06, tag `cpp-frozen-2026-11-06`.** This repository is a read-only reference. All work continues in `../OblivionRecompBevy`, including Phase 17. It ported phases 00 to 16 and can't be rebuilt on this machine, which has no MSVC. Its comments and values aren't an authority: the Bevy port found errors in them against the exe, listed in that repository's `docs/open-items.md`. Everything below describes the project as it stood at the freeze.

**`CLAUDE.md`: replace item 2 of "Session start" and add the notice.** The 4-line uncommitted change is folded in.

> **Frozen 2026-11-06.** Work in `../OblivionRecompBevy`, and read its `CLAUDE.md` instead.
>
> 2. The last handoff from here: `Implementation-Handoff-Phase16-Claude-2026-10-10.md`. Phase 16 is ported. Phase 17's handoff, `Implementation-Handoff-Phase17-Claude-2026-11-06.md`, is for the Bevy repository. Phases 06 to 16's format and lint item was superseded by that repository's `cargo fmt` and `cargo clippy` (decision D4), and this repository's own tools never ran on them.

### 2. The notes: `Startup-Roadmap.md` (plain edits; the folder isn't under git, so copy it first)

**Undo:** restore the copy. Raw evidence isn't touched.

- **Top of the file, above "Coding resumed":** a "Current" block.

  > **Current (2026-11-06): the work is in `OblivionRecompBevy`.** Phases 00 to 16 are ported there and passed its gate on host tests. Next is Phase 17: [handoff](Implementation-Handoff-Phase17-Claude-2026-11-06.md). The C++ repository is frozen at `cpp-frozen-2026-11-06`. Status paragraphs below this block describe the C++ port as it stood, and "Current" inside them is historical.

- **Line 16's and lines 912 to 935's "Current slice" and "Current 03-..." entries:** leave them. They're history, and the block above says so. Rewriting 25 links gains nothing.
- **"Bevy gate (M7)" section:** its opening says the gate doesn't pass, and its table has gone stale. Change the commit to the gate's, the opening to "all six read PASS, with these limits", G4 to "Pass", and phase 12's "edid count differs" to "0 failed". Replace the phase table's row-pass counts with the last report's.
- **Phase map rows 06 to 16 and the status paragraphs at lines 75, 161, 252, 361, 496, 614, 674, 716, 764, 794, 809:** "sign-off waits on format/lint" becomes the D4 text in step 3. 24 lines mention it; the historical ones in Phase 06's sign-off stay.
- **Row 17:** start `0040EF24`, "first-person" corrected, and the chain `00578CD0` → `0057E510` → `0057D480` → `005A6B00`.

### 3. Close D4's sign-off items (in that roadmap)

> **D4 (2026-11-06): the C++ format and lint item on phases 06 to 16 is superseded, not run.** `cargo fmt --check` and `cargo clippy --workspace --all-targets -D warnings` pass on the Rust port of these phases at the gate's commit. This says nothing about the C++ tree, which was never formatted or linted at this point.

**The phases' other sign-off items stay as they are.** Closing D4 doesn't sign a phase off in the strict sense, and "ported" stays the word. **For the user to decide:** whether phases 06 to 16 count as signed off now. This document doesn't assume so.

### 4. The notes: `Living-Conversion-Changes.md` and `LIVE-Checkpoint.md`

**Undo:** restore the copies. Both have been stale since 2026-10-06, by the user's decision to keep docs minimal, so each gets one short block at the top saying the work moved and pointing at the roadmap. No rows are added.

### 5. The ledger's writer moves here (overwrites one notes file)

```bash
cd ../OblivionRecompBevy
python3 tools/startup_roadmap/generate.py --notes-at-cutover ../OblivionBotInfos/Recomp/StartupLoading
```

**Undo:** restore a copy of `Startup-Call-Ledger.md`, taken first. **Dry-run on a scratch copy, 2026-11-06:** same 47 bodies, 1,224 calls and 1,605 blocks; 395 lines change, with the header's note. The file also went from CRLF to the generator's LF. One line changes the code's path from `src/app/original_startup_scaffold.cpp` to `crates/recomp_startup/src/scaffold_data.rs`, and most of the rest take `pending` to `implemented_behavior`.

**A caveat to write into the ledger's header, since the generator doesn't:** `implemented_behavior` comes from `dispositions.json`, which records the **C++** port (`tools/startup_roadmap/README.md`). What Rust has is `RUST_PORTED_THROUGH`. Without that note, the regenerated ledger reads as more of a claim than it is. Also rerun `cargo fmt` if `scaffold_data.rs` regenerates differently; the dry run didn't compare it.

### 6. This repository

**Undo:** an ordinary commit.

- `AGENTS.md`: "Do not start Phase 17 before then" becomes "the gate passed on 2026-11-06, with the limits in `docs/open-items.md`"; "migration source and side-by-side baseline" becomes "frozen reference at `cpp-frozen-2026-11-06`".
- `CLAUDE.md` item 2: current state and the Phase 17 handoff, as its "keep this item current" rule asks. Item 4 points at the new handoff.
- `tools/startup_roadmap/README.md`: the ledger's writer is here now, and drop "do not pass it before the cutover".
- `docs/migration-plan.md` M7: done, with the tag. `docs/open-items.md`: remove the "Cutover (M7)" and "Phase 17 needs..." lines, and carry what's left.
- This file: delete it, or mark it applied.

### Not part of the cutover

Starting Phase 17 itself, which waits on the user's word. The memory note that says the C++ repository is the migration source gets a line about the freeze.

## What the gate's PASS doesn't settle

Copied here so that the decision is made with it in view. Details are in `docs/open-items.md`.

- **G1** samples, and 27 rows and 262 changed values are excused with a reason rather than checked. Those reasons are a reading of what each value is.
- **G5** passes by the user's decision. The logo's colour is unexplained. The LoadingMenu, prompts, Load list and cursor weren't compared against the original's own frames.
- **G6**'s labels are from one scripted pass and aren't a review.
- **The same-seed compare** is one seed, under Wine, with no saves, no audio and no input. No input has reached the game under the virtual display, so the menu's Exit, New and Continue paths have no original run behind them.
- **IDB sync:** names and comments owed to the database haven't been written. It has only been read since the migration began.
