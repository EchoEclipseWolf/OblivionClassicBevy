#!/usr/bin/env python3
"""The prison's references and whether each has 3D, from run-af's `after_move` snapshot.

after_move_nodes_rows.py SNAPSHOT OUT.rows
SNAPSHOT is `snapshot-after_move` (005172C8, after command 64 returns). One row per reference in cell 1FBB9's +0x48 list,
in list order: `NODE <ref id> <base type> <1 if +0x3C is set> <+0x08 flags>`, hex. `the_port_runs_the_new_game_commands`
reads these rows.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ledger_compare import FORMS_MAP, walk_map  # noqa: E402
from snapshot import Snapshot  # noqa: E402

PRISON = 0x1FBB9


def main():
    snap = Snapshot(sys.argv[1])
    forms = {key: value for _, key, value in walk_map(snap, FORMS_MAP)}
    cell = forms[PRISON]
    lines = []
    for reference in snap.walk_list(cell + 0x48):
        base = snap.u32(reference + 0x1C)
        kind = snap.u8(base + 4) if base else 0
        node = 1 if snap.u32(reference + 0x3C) else 0
        lines.append(f"NODE {snap.u32(reference + 0x0C):X} {kind:X} {node} {snap.u32(reference + 8):X}")
    Path(sys.argv[2]).write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
