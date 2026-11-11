#!/usr/bin/env python3
"""The state Build 4's menu-mode frame leaves at the end of the move's frame, from run-x's `snapshot-frame_after_move`
(the first 0040F26A after command 64 returns: the rest of that 0040D800 frame has run).

frame_after_move_rows.py SNAPSHOT OUT.rows
Rows, hex (Build 4 agent, 2026-11-08):
  IFACE <InterfaceManager +0x08 status> <+0x09 transition byte> <+0xE0..+0x104 the ten menu modes>
  FADER <index> <+0x10 set: 0|1> <+0x08 active> <+0x0C seconds bits> <+0x14 alpha bits>   (00B12DC0 + 0x18 x index)
  HAVOK <00BA7914 substeps> <00BA790C bits> <00BA7910 bits> <00B2E2E0 bits> <00B2E2E4 bits> <00BA790A byte>
  FRAME <00B3341C byte> <00B075E8 bits> <00B075EC bits> <00B075F0 bits> <00B333B8 byte> <00B2D860 bits> <00B3BCF4>
  TIME <g_timeInfo +0x0C bits>
  PLAYER <+0x2C..+0x34 position bits> <+0x20..+0x28 rotation bits> <+0x588 byte>
  CTRL <controller +0x1F4> <+0x310 bits>
  CAMERA <the World SceneGraph camera's +0x64..+0x94 world transform, 13 words>
  ANIM <the third-person animation data's (process +0x17C) +0x00> <+0x94 bits> <+0x90 byte>
`the_move_frame_matches_frame_after_move` reads these rows.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ledger_compare import FORMS_MAP, walk_map  # noqa: E402
from snapshot import Snapshot  # noqa: E402

PLAYER = 0x14


def words(snap, base, count):
    return " ".join(f"{snap.u32(base + 4 * i):08X}" for i in range(count))


def main():
    snap = Snapshot(sys.argv[1])
    manager = snap.u32(0xB3A6E0)
    lines = [
        f"IFACE {snap.u8(manager + 8):X} {snap.u8(manager + 9):X} "
        + " ".join(f"{snap.u32(manager + 0xE0 + 4 * i):X}" for i in range(10))
    ]
    for index in range(3):
        base = 0xB12DC0 + 0x18 * index
        lines.append(
            f"FADER {index} {1 if snap.u32(base + 0x10) else 0} {snap.u8(base + 8):X} "
            f"{snap.u32(base + 0x0C):08X} {snap.u32(base + 0x14):08X}"
        )
    lines.append(
        f"HAVOK {snap.u32(0xBA7914):X} {snap.u32(0xBA790C):08X} {snap.u32(0xBA7910):08X} "
        f"{snap.u32(0xB2E2E0):08X} {snap.u32(0xB2E2E4):08X} {snap.u8(0xBA790A):X}"
    )
    lines.append(
        f"FRAME {snap.u8(0xB3341C):X} {snap.u32(0xB075E8):08X} {snap.u32(0xB075EC):08X} {snap.u32(0xB075F0):08X} "
        f"{snap.u8(0xB333B8):X} {snap.u32(0xB2D860):08X} {snap.u32(0xB3BCF4):X}"
    )
    lines.append(f"TIME {snap.u32(0xB33E90 + 0x0C):08X}")
    forms = {key: value for _, key, value in walk_map(snap, FORMS_MAP)}
    player = forms[PLAYER]
    lines.append(f"PLAYER {words(snap, player + 0x2C, 3)} {words(snap, player + 0x20, 3)} {snap.u8(player + 0x588):X}")
    process = snap.u32(player + 0x58)
    controller = snap.u32(process + 0x118)
    lines.append(f"CTRL {snap.u32(controller + 0x1F4):X} {snap.u32(controller + 0x310):08X}")
    camera = snap.u32(snap.u32(0xB333CC) + 0xDC)
    lines.append(f"CAMERA {words(snap, camera + 0x64, 13)}")
    anim = snap.u32(process + 0x17C)
    lines.append(f"ANIM {snap.u32(anim):X} {snap.u32(anim + 0x94):08X} {snap.u8(anim + 0x90):X}")
    Path(sys.argv[2]).write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
