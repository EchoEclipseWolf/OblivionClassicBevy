#!/usr/bin/env python3
"""The rows of Phase 19's window, from a memory snapshot at 0040F08D.

phase19_rows.py SNAPSHOT OUT.rows
What `the_port_matches_the_phase_19_rows` reads: the manager, the menu table, the masters' buffer size, the reticle's globals, the models
with their reference counts but for the two Tile3D ones (a counted boundary), and every tile under the reticle. Floats are their bits.
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
    rows.append(f"MANAGER status={s.u8(manager + 8)} stack={bits(manager + 0xE0, 10)} mods={s.u32(manager + 0x118)}")
    table, count = s.u32(0xB13974), int.from_bytes(s.read(0xB1397A, 2), "little")
    rows.append("MENUS " + " ".join(str(1001 + i) for i in range(count) if s.u32(table + 4 * i)))
    node = s.u32(0xB33A98) + 0x8C8
    index = 0
    while node:
        file = s.u32(node)
        if not file:
            break
        rows.append(f"FILE {index} master={s.u8(file + 0x3DC) & 1} buffer={s.u32(file + 0x228):08X}")
        node = s.u32(node + 4)
        index += 1

    def tile_name(address):
        return text(s.u32(address) + 8) if s.u32(address) else "null"

    rows.append(f"RETICLE crosshair={tile_name(0xB3B350)} detect={tile_name(0xB3B358)} sneak={tile_name(0xB3B354)} "
                f"seconds={bits(0xB140BC, 3)} alpha={bits(0xB140B8)} sneaking={s.u8(0xB3B361)}")
    model_map = s.u32(s.u32(0xB33A1C))
    size, buckets = s.u32(model_map + 8), s.u32(model_map + 12)
    models = []
    for bucket in range(size):
        entry = s.u32(buckets + 4 * bucket) & 0xFFFFFFFE
        while entry:
            model = s.u32(entry + 4)
            models.append((s.text(s.u32(model)), s.u32(model + 4) & 0xFFFF))
            entry = s.u32(entry + 8) & 0xFFFFFFFE
    for path, references in sorted(models, key=lambda model: model[0].lower()):
        if not path.lower().startswith("meshes\\menus\\"):
            rows.append(f"MODEL {path} references={references}")

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

    tile(s.u32(manager + 0x80), 0)
    with open(sys.argv[2], "w", encoding="latin-1", newline="\n") as out:
        out.write("\n".join(rows) + "\n")
    print(len(rows), "rows")


if __name__ == "__main__":
    main()
