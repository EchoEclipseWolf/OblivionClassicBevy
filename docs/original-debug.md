# Running and debugging the original on Linux

Set up and proven on 2026-11-06. This replaces "It isn't set up yet" in earlier notes. It runs with no user action, on the virtual display, driven from a Claude session.

| Piece | Value |
| --- | --- |
| Exe | `OblivionBot/Oblivion.exe`, SHA-256 `0244dd59…`, run as `G:\OblivionBot\Oblivion.exe` |
| Prefix | `~/.wine-oblivion-capture`, from `tools/original_debug/setup_prefix.sh`: pinned `Oblivion.ini` (`d243541f…`), `BlendSettings.ini`, `Plugins.txt`, no saves. Wine 11.20 |
| Display | `tools/virtual_display.py`. At 1920x1080 the game's client area comes out 1920x1060; `OBLIVION_VIRTUAL_WIDTH=2000 OBLIVION_VIRTUAL_HEIGHT=1200` gives a true 1920x1080 |
| Debugger | IDA 9.5 `idat` through the ida MCP, remote Win32, `tools/original_debug/dbgsrv.sh` on `127.0.0.1:23947` |
| Database | A **copy** at `~/oblivion-capture/Oblivion.exe.i64`, so the real `.i64` is never locked. It's never saved |
| Logging debugger | A **private copy** of x32dbg at `~/oblivion-capture/x64dbg`, with `tools/original_debug/logfile_plugin.c` built into its `plugins`. Never the shared `Echo/x64dbg`: a plugin doesn't belong there, and x32dbg writes its `.ini` and `db` next to itself |

## Steps

```bash
tools/original_debug/setup_prefix.sh
mkdir -p ~/oblivion-capture
cp --reflink=auto ../OblivionBot/Oblivion.exe.i64 ~/oblivion-capture/
python3 tools/virtual_display.py start ${XDG_RUNTIME_DIR}/ob-g1
python3 tools/virtual_display.py exec ${XDG_RUNTIME_DIR}/ob-g1 -- tools/original_debug/dbgsrv.sh
```

Then, in the ida MCP, `open_database` on the copy and:

```python
import ida_dbg
ida_dbg.load_debugger("win32", 1)
d = r"G:\OblivionBot"
ida_dbg.set_process_options(d + r"\Oblivion.exe", "", d, "127.0.0.1", "", 23947)
ida_dbg.del_bpt(0x5DBBD0)   # the copy's saved breakpoint
```

Add breakpoints, then `ida_dbg.start_process(None, None, None)`, and pump with `wait_for_next_event`. `tools/original_debug/capture_a.py` is a worked example. End with `ida_dbg.exit_process()`, and check that no `Oblivion.exe` is left.

## The same run on both sides

A snapshot's seed lets the port run the same rolls, so the two sides can compare value for value, with no tolerance. On a virtual display:

```bash
LEDGER_SEED=6AEDD89C LEDGER_DUMP=$HOME/oblivion-capture/port-b1.txt cargo test -p recomp_startup --test winmain_driver dump_for_the_ledger -- --ignored
```

Then, from `tools/original_debug`, with `~/oblivion-capture/run-b1-snap/` holding the snapshots:

```bash
python3 ledger_compare.py ~/oblivion-capture/run-b1-snap/snapshot-phase16_end ~/oblivion-capture/port-b1.txt
```

```bash
python3 ledger_lists.py phase16_end ~/oblivion-capture/port-b1.txt
```

```bash
python3 field_compare.py ~/oblivion-capture/run-b1-snap/snapshot-phase16_end ~/oblivion-capture/port-b1.txt
```

The first takes forms, references and change records. The second takes lists, editor IDs, extras, cells, worlds and door nodes. The third takes every field whose name ends in its offset; `_b33a2c` is absolute. A resource's `GLOB` line or a singleton's `HEAP` line in the dump gives the base. Results and limits are in `docs/open-items.md`. `the_port_still_matches_the_same_seed_snapshot` hashes the port's side, so it runs with no snapshot; when it fails, compare again and only then change its hashes.

## Rows from a snapshot

`capture_a.py`'s checkpoints include `0040EFF3` (`phase17_end`) and `005B6093` (`menu_closed`, past `0057CC00`, `00579770` and `00578CF0`). Then, from `tools/original_debug`:

```bash
python3 phase17_rows.py ~/oblivion-capture/run-h-phase17/snapshot-phase17_end OUT.rows
```

`phase18_rows.py` does the same at `0040F069`, from `run-i-phase18/snapshot-phase18_end`. The set `window` adds first-hit call coverage over `ida_dbg.CAP_WINDOW` (`ida_dbg.CAP_WINDOWS` has Phases 18 to 22's), following tail jumps and indirect calls, and `CAP_SNAPS` limits which checkpoints snapshot. `phase19_rows.py` reads `run-k-phase19/snapshot-phase19_end`, `phase20_rows.py` reads `run-m-phase20/snapshot-phase20_end`, `phase21_rows.py` reads `run-o-phase21/snapshot-phase21_end`, and `phase22_rows.py` reads `run-q-phase22/snapshot-idle_frame_end`. A breakpoint on a shared function can be hit by the IO thread first, so a `COVER` line's thread matters.

The evidence copy is `evidence/phase17-run001-2026-11-06/phase17.rows` in the notes, and `tools/import_fixtures.py` takes it to `fixtures/rows/`. A run's seed goes with it: this one's is `6AEE4398`, in `records.txt`.

## A new game, with no input

`capture_newgame.py IDLE_FRAMES SECONDS`, under the private x32dbg, runs the script census to the menu, then at the IDLE_FRAMES-th `0040F26A` writes InterfaceManager `+0xB0 = 1` and calls `005B5DF0`, returning to `0040F26A`. It logs each frame's status and the player's cell after that. Under IDA the same three writes work from a stop. Confirm after a few frames, not the first: `00584740` does nothing until the menu is visible 2.

## Which tool

| Need | Tool | Cost |
| --- | --- | --- |
| Frequent sites on one thread: every draw, every insert | x32dbg logging breakpoints, `capture_c.py` | About 1.3 ms a hit: 100,000 lines in 140 s |
| Memory dumps, logic per stop, sites on several threads | IDA, `capture_a.py`, `snapshot.py` | About 100 ms a stop; 550 MB in 10 s |

x32dbg evaluates the condition and logs inside the debugger, with no trip through a controller. Under IDA the same run would be about 3 hours.

Start the private copy on the virtual display. `logfile.dp32` loads itself from `plugins`, and a second `plugload logfile` stops the logging: its second open of `C:\\x32log.txt` fails on the sharing mode and replaces the handle. In its `x32dbg.ini`, `[Exceptions] IgnoreRange=406D1388-406D1388` passes the thread-name exceptions.

- **`isRunning` reads false briefly while a conditional breakpoint runs.** Don't take that for a stop, or a `run` lands on a running game.
- **Polling over HTTP logs a line per request.** `capture_c.py` watches the file's size.
- **Log lines come in batches about 100 ms apart,** so ticks are that coarse, and only a batch's first line has one.
- **`{s:eax}` printed `???` on the insert probe,** though a constant address works. Join on the form ID instead.
- **`{s:…}` fails on most live addresses; `{mem;50@[esp+4]}` works** and logs the bytes as hex.
- **Breakpoints hit by two threads at once aren't safe here.** In a run with probes on both the main and IO threads, `006F9980` logged one call twice, `0043B420` logged 2 of 27 hits, and the debugger paused at `006F9981`. **So use x32dbg for sites one thread hits,** and check a count another way: the rand state against its distance, the door list against a snapshot. For sites both threads reach, use IDA, which logged all 27.
- **x32dbg restores a program's breakpoints from its database.** `capture_c.py` runs `bc` first.
- **No X, `SendInput` or posted-menu input reached x32dbg's menus,** and no input reaches the game.

## Traps

- **Wine links prefix folders to the real home.** See `setup_prefix.sh`.
- **Don't start audio servers in the virtual session.** `virtual_display.py` doesn't redirect `XDG_STATE_HOME`, and PipeWire there sees the real devices. A private WirePlumber wrote to the real `~/.local/state/wireplumber/` on 2026-11-06.
- **A breakpoint inside an instruction corrupts it.** `0040EBB8`, a label in the notes, is inside the `call` at `0040EBB4`; the int3 changed the call's target and the game crashed. `capture_a.py` refuses non-instruction addresses. `idc.GetDisasm` on a non-head shows the containing instruction, so it hides this.
- **`FS` reads 0 here**, so `dbg_get_thread_sreg_base` fails, and older scripts log `teb=0`. Find a TEB by content: a page where `+0x18` is the page and `+0x24` is the thread ID. Read a thread's `_holdrand` from the ptd in `eax` at `009859D5` (`_srand`) or `009859E2` (`_rand`), at `+0x14`.
- **Each stop costs about 100 ms, however few breakpoints are armed.** 10,000 hits is about 17 minutes, so a frequent site gets its own run.

## How far it reproduces the recorded runs

P16-RUN-001 replayed with its own unchanged script (`phase16-scope-2026-10-10/p16_run_lib.py`): same 52 records in the same order and the same end, `0040EF54`. Of 559 fields, 132 differ by heap pointer only. What's left:

| Field | Recorded | Here | Reading |
| --- | --- | --- | --- |
| `TL` teb, block, b185 | `28E000`, `E27EC0`, 1 | 0 | The `FS` trap. Read by content: `00 01 00`, as recorded |
| `WIN before` rotation, `PLAYER` raw and `procraw` words | | | Uninitialised bytes |
| Change count `m0 +0x0C` | `0x1568` | `0x156E` | Roll-dependent, by reasoning |
| `fnstsw` `eax` at `65A50B`, `65A530` | `...3B` | `...22` | x87 sticky flags; not explained, nothing found reading them |
| `IO raw +0x18` | `0x0A` | 0 | **Not explained.** The ledger lists this word as drifting |
| `IO raw +0x30` | `0x10120` | `0x1BC` | **Not explained** |

`RendererInfo.txt` equals P04-RUN-001's line for line except the GPU name and driver DLL, so the render path and shader package are the same. **Known differences from the recorded machine:** no audio device (this Wine has no 32-bit `winepulse.drv`, and no driver module loads), no saves, wined3d for D3D9, Linux file timing. Treat a value that could depend on those as Wine evidence only. A capture earns the original-validation bonus only with that stated.

`newgame_commands_rows.py RECORDS SNAPSHOT OUT.rows` reads a run with `005172C6` and `004FADD5` logged from the confirmation, stopped at `00512870` when `[esp+0xC]` is the player. **Start `dbgsrv.sh` through `virtual_display.py exec`:** run directly, the game opens on the real display. First-hit coverage begun after the confirmation misses what its first frames ran, such as `005A4840`.
