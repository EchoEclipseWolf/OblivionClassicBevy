#!/usr/bin/env python3
"""The rows of Phase 17's window, from a memory snapshot at 0040EFF3.

phase17_rows.py SNAPSHOT OUT.rows
What `the_port_matches_the_phase_17_rows` reads: the manager, world child 0, the player scene and its light, the HUD's fields, and
every tile under the HUD with its traits in list order. Floats are their bits. See docs/original-debug.md.
"""
import struct
import sys
from snapshot import Snapshot

KIND = {0xA69138: "rect", 0xA69110: "image", 0xA6AEC4: "text", 0xA6AEEC: "model", 0xA6AF14: "menu"}


def main():
    s = Snapshot(sys.argv[1])
    rows = []

    def bits(address, count=1):
        return " ".join(f"{s.u32(address + 4 * i):08X}" for i in range(count))

    def text(address):
        data, length = s.u32(address), int.from_bytes(s.read(address + 4, 2), "little")
        return (s.read(data, length) or b"").decode("latin-1").replace("\n", "\\n") if data and length else ""

    def name(node):
        pointer = s.u32(node + 8)
        return s.text(pointer) if pointer else ""

    def scene(tag, node):
        rows.append(f"{tag} name={name(node)} flags={s.u32(node + 0x18) & 0xFFFF:04X} rot={bits(node + 0x30, 9)} pos={bits(node + 0x54, 3)} "
                    f"scale={bits(node + 0x60)} world={bits(node + 0x88, 3)}")

    def walk(node):
        items = []
        while node:
            items.append(s.u32(node + 8))
            node = s.u32(node)
        return items

    manager = s.u32(0xB3A6E0)
    graph = s.u32(0xB333CC)
    rows.append(f"MANAGER status={s.u8(manager + 8)} stack={bits(manager + 0xE0, 10)}")
    scene("CHILD0", s.u32(s.u32(graph + 0xB0)))
    scene("SCENE60", s.u32(manager + 0x60))
    scene("SCENE64", s.u32(manager + 0x64))
    scene("LIGHT", s.u32(manager + 0x18))
    light = s.u32(manager + 0x18)
    rows.append(f"LIGHTDATA revision={s.u32(light + 0xB8)} ambient={bits(light + 0xE0, 3)} diffuse={bits(light + 0xEC, 3)} direction={bits(light + 0x108, 3)}")
    shadow = s.u32(manager + 0x64)
    rows.append(f"SHADOW b11c={s.u8(shadow + 0x11C)} slot1={'same' if s.u32(0xB42F54 + 4) == shadow else 'other'} parent={'menu3d' if s.u32(shadow + 0x1C) == s.u32(manager + 4) else 'other'}"
                f" children={len([c for c in (s.u32(s.u32(shadow + 0xB0) + 4 * i) for i in range(int.from_bytes(s.read(shadow + 0xB6, 2), 'little'))) if c])}")
    table, count = s.u32(0xB13974), int.from_bytes(s.read(0xB1397A, 2), "little")
    rows.append("MENUS " + " ".join(str(1001 + i) for i in range(count) if s.u32(table + 4 * i)))
    root = s.u32(manager + 0x68)
    values = {s.u32(v + 0x18) & 0xFFFF: v for v in walk(s.u32(root + 0x18))}
    rows.append(f"ROOT user0={bits(values[4014] + 4)}")
    hud = s.u32(table + 4 * 3)
    menu = s.u32(hud + 0x44)
    rows.append(f"HUD id={s.u32(menu + 0x20)} state={s.u32(menu + 0x24)} depth18={s.u32(menu + 0x18)} mode={s.u32(menu + 0x14)} "
                f"zoom70={bits(menu + 0x70)} pointer={bits(menu + 0x68, 2)} b8c={s.u32(menu + 0x8C)}")
    names = []
    for offset in range(0x28, 0x5C, 4):
        tile = s.u32(menu + offset)
        names.append(f"{offset:02X}={text(tile + 8)}" if tile else f"{offset:02X}=null")
    rows.append("SLOTS " + " ".join(names))

    def tile(address, depth):
        kind = KIND.get(s.u32(address), f"{s.u32(address):X}")
        children = walk(s.u32(address + 0x34))
        node = s.u32(address + 0x24)
        rows.append(f"TILE {depth} {kind} {text(address + 8)} flags={s.u32(address + 0x2C):X} children={len(children)}")
        if node:
            rows.append(f"NODE flags={s.u32(node + 0x18) & 0xFFFF:04X} pos={bits(node + 0x54, 3)} scale={bits(node + 0x60)} "
                        f"nodechildren={int.from_bytes(s.read(node + 0xB6, 2), 'little')}")
        for value in walk(s.u32(address + 0x18)):
            if value:
                number = s.u8(value + 0x1A)
                rows.append(f"VAL {s.u32(value + 0x18) & 0xFFFF} {number} {bits(value + 4)} {text(value + 8)}")
        for child in children:
            tile(child, depth + 1)

    tile(hud, 0)
    with open(sys.argv[2], "w", encoding="latin-1") as out:
        out.write("\n".join(rows) + "\n")
    print(len(rows), "rows")


if __name__ == "__main__":
    main()
