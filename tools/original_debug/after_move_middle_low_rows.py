#!/usr/bin/env python3
"""The actors 005EBB30 moved to a MiddleLowProcess, from run-af's `after_move` snapshot (005172C8, after command 64 returns).

after_move_middle_low_rows.py SNAPSHOT OUT.rows
Rows, hex: `MLLIST <ids>`, g_actorProcessManager's (00B3BD00) level-2 list (+0x0C) in list order; then per actor of it
`MLACTOR <id> VT <process vtable> LEVEL <2 if in +0x0C, 3 if in +0x18, 0 if in +0x68, else -> CA <+0xCA>
PKG <the process's +0x08 package's form ID or -> F0C <+0x0C bits> F14 <+0x14 bits> D18 <+0x18> CHG <1 with the 0x1A
container changes extra (BaseExtraList +0x08 presence bits), else 0>`. run-ab's `fa_ret` has CHG 1 only for 1772E and 17754
before the drain, and `drain_ret` all ten: 005E4B00's 00485E00 makes the others'.
`the_port_runs_the_new_game_commands` reads these rows.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ledger_compare import FORMS_MAP, walk_map  # noqa: E402
from snapshot import Snapshot  # noqa: E402

MANAGER = 0xB3BD00


def main():
    snap = Snapshot(sys.argv[1])
    lists = {level: snap.walk_list(MANAGER + offset) for level, offset in ((0, 0x68), (1, 0x00), (2, 0x0C), (3, 0x18))}
    middle_low = lists[2]
    lines = ["MLLIST " + " ".join(f"{snap.u32(actor + 0x0C):X}" for actor in middle_low)]
    for actor in middle_low:
        form_id = snap.u32(actor + 0x0C)
        process = snap.u32(actor + 0x58)
        level = next((str(level) for level, items in lists.items() if actor in items), "-")
        package = snap.u32(process + 0x08)
        package_id = f"{snap.u32(package + 0x0C):X}" if package else "-"
        lines.append(
            f"MLACTOR {form_id:X} VT {snap.u32(process):X} LEVEL {level} CA {snap.u8(actor + 0xCA):X} PKG {package_id}"
            f" F0C {snap.u32(process + 0x0C):08X} F14 {snap.u32(process + 0x14):08X} D18 {snap.u32(process + 0x18):X}"
            f" CHG {(snap.u8(actor + 0x44 + 8 + 3) >> 2) & 1}"
        )
    Path(sys.argv[2]).write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
