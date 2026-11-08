#!/usr/bin/env python3
"""The rows of Phase 21's window, from a memory snapshot at 0040F0E0.

phase21_rows.py SNAPSHOT OUT.rows
What `the_port_matches_the_phase_21_rows` reads: `phase20_rows.py`'s, with the movie player and the music after the media. Floats are
their bits. g_renderer's +0x208, one higher for 0040F09A and 0040F09F, counts the host's frames too, so it's no row.
"""
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

    def walk(node):
        items = []
        while node:
            items.append(s.u32(node + 8))
            node = s.u32(node)
        return items

    manager = s.u32(0xB3A6E0)
    rows.append(f"MANAGER status={s.u8(manager + 8)} stack={bits(manager + 0xE0, 10)} mods={s.u32(manager + 0x118)} "
                f"priority={s.u32(manager + 0x8C)} geometry={s.u8(manager + 0x7C)} active={s.u32(manager + 0x98):X},{s.u32(manager + 0x9C):X}")
    table, count = s.u32(0xB13974), int.from_bytes(s.read(0xB1397A, 2), "little")
    rows.append("MENUS " + " ".join(str(1001 + i) for i in range(count) if s.u32(table + 4 * i)))
    rows.append(f"MEDIA backdrop={s.u8(0xB3B408)} constructed={s.u8(0xB3B409)} title={s.u8(0xB3C0EC)}")
    movie = s.u32(0xB33428)
    rows.append(f"MOVIE state={s.u32(movie + 0x20)} scale={bits(movie + 0x14)} x={bits(movie + 0x18)} y={bits(movie + 0x1C)} "
                f"thread={int(s.u32(0xB33434) != 0)} waiter={s.u8(0xB33425)} cached={int(s.u32(0xB33440) != 0)} "
                f"valid={s.u8(0xB33431)} paused={int(s.u32(0xB3342C) != 0)}")
    sound = s.u32(s.u32(0xB33398) + 0x24)
    rows.append(f"MUSIC flags={s.u32(sound + 0xDC):X} type={s.u32(sound + 0xB0)}")
    saves = s.u32(s.u32(0xB33B00) + 0x6C)
    rows.append(f"SAVES file={s.u32(saves):X} next={s.u32(saves + 4):X}")
    menu = s.u32(0xB3B40C)

    def tile_name(address):
        tile = s.u32(address)
        return text(tile + 8) if tile else "null"

    rows.append("MENU " + " ".join(f"{tile_name(menu + offset)}" for offset in (0x28, 0x30, 0x34, 0x38, 0x3C, 0x40, 0x44)))
    rows.append(f"MENUSTATE mode={s.u32(menu + 0x14):X} state={s.u32(menu + 0x24)} unk4c={s.u8(menu + 0x4C)} pending={s.u8(menu + 0x4D)}")

    def tile(address, depth):
        kind = KIND.get(s.u32(address), f"{s.u32(address):X}")
        children = walk(s.u32(address + 0x34))
        scene = s.u32(address + 0x24)
        rows.append(f"TILE {depth} {kind} {text(address + 8)} flags={s.u32(address + 0x2C):X} children={len(children)}")
        if scene:
            rows.append(f"NODE flags={s.u32(scene + 0x18) & 0xFFFF:04X} pos={bits(scene + 0x54, 3)} scale={bits(scene + 0x60)} "
                        f"nodechildren={int.from_bytes(s.read(scene + 0xB6, 2), 'little')}")
        for value in walk(s.u32(address + 0x18)):
            if value:
                rows.append(f"VAL {s.u32(value + 0x18) & 0xFFFF} {s.u8(value + 0x1A)} {bits(value + 4)} {text(value + 8)}")
        for child in children:
            tile(child, depth + 1)

    tile(s.u32(menu + 4), 0)
    with open(sys.argv[2], "w", encoding="latin-1", newline="\n") as out:
        out.write("\n".join(rows) + "\n")
    print(len(rows), "rows")


if __name__ == "__main__":
    main()
