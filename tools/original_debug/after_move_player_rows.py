#!/usr/bin/env python3
"""The player's 3D block from run-af's `after_move` snapshot (005172C8, after command 64 returns).

after_move_player_rows.py SNAPSHOT OUT.rows
Rows: `PNODE <field> <name> <+0x18 flags>` for +0x3C, +0x5D0 and the globals 00B3BB0C, 00B3BB10, 00B3BB14 (00667BE0's
Camera01, the third person's Camera01 and Bip); `PBYTE 588 <+0x588>`, `PBITS 5D4 <+0x5D4 bits>`, `PSET 5C8|5CC <0|1>` and
`PBYTE 16C <the process's +0x16C>`, hex. `the_port_runs_the_new_game_commands` reads these rows.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ledger_compare import FORMS_MAP, walk_map  # noqa: E402
from snapshot import Snapshot  # noqa: E402

PLAYER = 0x14


def node_row(snap, field, node):
    if not node:
        return f"PNODE {field} - -"
    name = snap.u32(node + 8)
    text = snap.text(name) if name else "-"
    flags = snap.u32(node + 0x18) & 0xFFFF
    return f"PNODE {field} {text} {flags:X}"


def main():
    snap = Snapshot(sys.argv[1])
    forms = {key: value for _, key, value in walk_map(snap, FORMS_MAP)}
    player = forms[PLAYER]
    lines = [
        node_row(snap, "3C", snap.u32(player + 0x3C)),
        node_row(snap, "5D0", snap.u32(player + 0x5D0)),
        node_row(snap, "B3BB0C", snap.u32(0xB3BB0C)),
        node_row(snap, "B3BB10", snap.u32(0xB3BB10)),
        node_row(snap, "B3BB14", snap.u32(0xB3BB14)),
        f"PBYTE 588 {snap.u8(player + 0x588):X}",
        f"PBITS 5D4 {snap.u32(player + 0x5D4):08X}",
        f"PSET 5C8 {1 if snap.u32(player + 0x5C8) else 0}",
        f"PSET 5CC {1 if snap.u32(player + 0x5CC) else 0}",
        f"PBYTE 16C {snap.u8(snap.u32(player + 0x58) + 0x16C):X}",
    ]
    Path(sys.argv[2]).write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
