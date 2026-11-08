#!/usr/bin/env python3
"""The rows of Phase 18's window, from a memory snapshot at 0040F069.

phase18_rows.py SNAPSHOT OUT.rows
What `the_port_matches_the_phase_18_rows` reads: the player's stores, the costs 00413810 cached, the HUD's icon trait, the base's
change flags, the two last stores, and the initial state's bytes. The art setting's +0xA0 races with the IO thread, so it isn't here.
"""
import sys
from snapshot import Snapshot


def main():
    s = Snapshot(sys.argv[1])
    rows = []
    player = s.u32(0xB333C4)
    base = s.u32(player + 0x1C)
    save = s.u32(0xB33B00)
    item = s.u32(player + 0x624)
    # 0040F03A: the item is spell + 0x18.
    rows.append(f"PLAYER item={s.u32(item - 0x18 + 0xC):08X} book={s.u32(player + 0x628):08X} tk={s.u8(player + 0x6E6)}")
    node = base + 0x58
    while node:
        spell = s.u32(node)
        if spell:
            list_node = spell + 0x24 + 4
            costs = []
            while list_node:
                effect = s.u32(list_node)
                if effect:
                    costs.append(f"{s.u32(effect + 0x20):08X}")
                list_node = s.u32(list_node + 4)
            rows.append(f"SPELL {s.u32(spell + 0xC):08X} costs={' '.join(costs)}")
        node = s.u32(node + 4)
    table = s.u32(0xB13974)
    menu = s.u32(s.u32(table + 4 * 3) + 0x44)
    tile = s.u32(menu + 0x3C)
    value_node = s.u32(tile + 0x18)
    icon = ""
    while value_node:
        value = s.u32(value_node + 8)
        if s.u32(value + 0x18) & 0xFFFF == 0xFE6:
            data, length = s.u32(value + 8), int.from_bytes(s.read(value + 0xC, 2), "little")
            icon = s.read(data, length).decode("latin-1") if data and length else ""
        value_node = s.u32(value_node)
    rows.append(f"HUDICON {icon}")
    changes = s.u32(save)
    size, buckets = s.u32(changes + 4), s.u32(changes + 8)

    def change_flags(form):
        key = s.u32(form + 0xC)
        chain = s.u32(buckets + 4 * (key % size))
        while chain:
            if s.u32(chain + 4) == key:
                return s.u32(s.u32(chain + 8))
            chain = s.u32(chain)
        return 0

    rows.append(f"CHANGES base={change_flags(base):08X} player={change_flags(player):08X}")
    rows.append(f"SAVE 7D={s.u8(save + 0x7D)} 14={s.u32(save + 0x14):08X}")
    rows.append(f"OS3 {s.u8(s.u32(0xB33398) + 3)}")
    rows.append(f"LIMIT {s.u32(0xB04360):08X}")
    # 00453500's allocation is size + 4; 006610F0's size went by in the run, and the heap's block isn't read.
    buffer = s.u32(player + 0x70C)
    data = s.read(buffer, 0x9F4)
    for start in range(0, len(data), 32):
        rows.append(f"STATE {start:04X} {data[start:start + 32].hex().upper()}")
    with open(sys.argv[2], "w", newline="\n") as out:
        out.write("".join(row + "\n" for row in rows))


main()
