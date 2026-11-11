#!/usr/bin/env python3
"""The Imperial Prison's path grid (cell 1FBB9's +0x44 TESPathGrid) from run-af's `after_move` snapshot.

pathing_prison_rows.py SNAPSHOT OUT.rows
SNAPSHOT is `snapshot-after_move` (005172C8, after command 64 returns), or run-x's `snapshot-frame_after_move` (the next
0040F26A, after command 65 DisableLinkedPathPoints ran on 3E31C). Rows, hex (pathing agent, 2026-11-08):
  GRID <+0x30 point count> <+0x24 array +0x0C count> <+0x34 map count> <+0x44 map count> <+0x28 PGRI entries>
  POINT <index> <+0x00 f> <+0x04 g> <+0x08 h> <+0x0C parent index or -> <+0x10 flags> <+0x14 x> <+0x18 y> <+0x1C z> <+0x28> <connections as indexes, in list order>
  BUCKET <+0x44 bucket> <key> <points as indexes, in list order>
  LINKED <reference form ID> <points as indexes, in list order>   (each +0x34 entry, in bucket and chain order; PGRL)
Floats are their raw u32 bits. Bytes +0x11..+0x13 are heap bytes the constructor never writes, so they are left out.
`oblivion_pathing`'s tests and `the_port_runs_the_new_game_commands` read these rows.
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
    grid = snap.u32(forms[PRISON] + 0x44)
    array = snap.u32(grid + 0x24)
    count = snap.u32(array + 0x0C) & 0xFFFF
    data = snap.u32(array + 4)
    pointers = [snap.u32(data + 4 * i) for i in range(count)]
    index = {pointer: i for i, pointer in enumerate(pointers)}
    inter = len(snap.walk_list(grid + 0x28))
    lines = [
        f"GRID {snap.u32(grid + 0x30) & 0xFFFF:X} {count:X} {snap.u32(grid + 0x34 + 0x0C):X} "
        f"{snap.u32(grid + 0x44 + 0x0C):X} {inter:X}"
    ]
    for i, point in enumerate(pointers):
        words = [snap.u32(point + at) for at in (0x00, 0x04, 0x08)]
        parent = snap.u32(point + 0x0C)
        parent_text = f"{index[parent]:X}" if parent else "-"
        position = [snap.u32(point + at) for at in (0x14, 0x18, 0x1C)]
        connections = [f"{index[c]:X}" for c in snap.walk_list(point + 0x20)]
        lines.append(
            "POINT "
            + " ".join(
                [f"{i:X}"]
                + [f"{w:08X}" for w in words]
                + [parent_text, f"{snap.u8(point + 0x10):X}"]
                + [f"{w:08X}" for w in position]
                + [f"{snap.u32(point + 0x28):X}"]
                + connections
            )
        )
    for bucket, key, value in walk_map(snap, grid + 0x44):
        listed = [f"{index[p]:X}" for p in snap.walk_list(value)]
        lines.append(" ".join(["BUCKET", f"{bucket:X}", f"{key:08X}"] + listed))
    for _, reference, value in walk_map(snap, grid + 0x34):
        listed = [f"{index[p]:X}" for p in snap.walk_list(value)]
        lines.append(" ".join(["LINKED", f"{snap.u32(reference + 0x0C):X}"] + listed))
    Path(sys.argv[2]).write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
