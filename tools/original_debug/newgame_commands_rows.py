#!/usr/bin/env python3
"""The script commands of a new game, and their effects, up to the player's first MoveTo.

newgame_commands_rows.py RECORDS SNAPSHOT OUT.rows
RECORDS is what the capture logged after `CP new_game`, each 005172C6 with its target, reference and script. SNAPSHOT is from 00512870, on the player. `the_port_runs_the_new_game_commands` reads these rows.
"""
import re
import sys
from snapshot import Snapshot
import ledger_compare as L

# 004FCA30's table: a command's handler and its opcode.
OPCODES = {0x503B60: 4184, 0x503A50: 4179, 0x501890: 4366, 0x50A150: 4129, 0x50A240: 4130, 0x502D80: 4151, 0x502D30: 4150,
           0x502CB0: 4153, 0x505680: 4300, 0x505AC0: 4337, 0x510B90: 4125, 0x505A30: 4332, 0x512870: 4254, 0x5032D0: 4327,
           0x506750: 4433, 0x5062D0: 4395}


def inline_list(s, head):
    items = []
    while head:
        data, head = s.u32(head), s.u32(head + 4)
        if data:
            items.append(data)
    return items


def main():
    rows = []
    started = False
    for line in open(sys.argv[1], errors="replace"):
        started = started or "CP new_game" in line
        match = re.search(r"CMD ([0-9A-F]+) ref=([0-9A-F]+) script=([0-9A-F]+)", line)
        if started and match:
            handler, reference, script = (int(group, 16) for group in match.groups())
            rows.append(f"CMD {OPCODES[handler]} {reference:X} {script:X}")
    s = Snapshot(sys.argv[2])
    forms = {k: v for _, k, v in L.walk_map(s, L.FORMS_MAP)}
    player = s.u32(0xB333C4)
    rows.append("TOPICS " + " ".join(f"{s.u32(topic + 0xC):X}" for topic in inline_list(s, player + 0x5E4)))
    rows.append(f"CHARGEN {s.u8(player + 0x6E5)}")
    for identity in (0x280DA, 0x280E6):
        actor = forms[identity]
        entries = inline_list(s, actor + 0xA4)
        rows.append(f"DISPOSITION {identity:X} " + " ".join(f"{s.u32(entry):08X},{s.u32(s.u32(entry + 4) + 0xC):X}" for entry in entries))
        values = inline_list(s, actor + 0x88)
        rows.append(f"AV {identity:X} " + " ".join(f"{s.u8(entry)},{s.u32(entry + 4):08X}" for entry in values))
    # Command 30's actor, and the one command 31 enables as its marker's child: their extra types in link order; its changes entries in list order (form, count, each stack's extra types in link order); its process's vtable and current package; and the 0x1E package start (location, the position's and +0x1C's bits), or `none`.
    for identity in (0x90028F5, 0x90013E8, 0x35CAD):
        actor = forms[identity]
        entries, kinds, start = [], [], "none"
        node = s.u32(actor + 0x48)
        while node:
            kinds.append(f"{s.u8(node + 4):X}")
            if s.u8(node + 4) == 0x1E:
                start = f"{s.u32(s.u32(node + 0xC) + 0xC):X} " + " ".join(f"{s.u32(node + o):08X}" for o in (0x10, 0x14, 0x18, 0x1C))
            if s.u8(node + 4) == 0x1A:
                for entry in inline_list(s, s.u32(s.u32(node + 0xC))):
                    stacks = []
                    for stack in inline_list(s, s.u32(entry)) if s.u32(entry) else []:
                        types, extra = [], s.u32(stack + 4)
                        while extra:
                            types.append(f"{s.u8(extra + 4):X}")
                            extra = s.u32(extra + 8)
                        stacks.append("+".join(types))
                    entries.append(f"{s.u32(s.u32(entry + 8) + 0xC):X},{s.u32(entry + 4)},{'/'.join(stacks)}")
            node = s.u32(node + 8)
        rows.append(f"EXTRAS {identity:X} " + " ".join(kinds))
        rows.append(f"INVENTORY {identity:X} " + " ".join(entries))
        process = s.u32(actor + 0x58)
        rows.append(f"PROCESS {identity:X} {s.u32(process) if process else 0:X}")
        package = s.u32(process + 8) if process else 0
        rows.append(f"PACKAGE {identity:X} {s.u32(package + 0xC) if package else 0:X}")
        rows.append(f"PACKAGESTART {identity:X} {start}")
        # What Actor_Update leaves and its later frames keep: the rotation's bits, +0xFD, +0x100's bits, and the process's procedure index and +0x84.
        rotation = " ".join(f"{s.u32(actor + o):08X}" for o in (0x20, 0x24, 0x28))
        rows.append(f"ACTORSTATE {identity:X} {rotation} {s.u8(actor + 0xFD)} {s.u32(actor + 0x100):08X} {s.u32(process + 4)} {s.u8(process + 0x84)}")
    # Commands 45, 46 and 54's MoveTos: the parent cell, the position's and the rotation's bits.
    for identity in (0x8565D, 0x2FEAC, 0x35CAD):
        ref = forms[identity]
        cell = s.u32(ref + 0x40)
        rows.append(f"PLACE {identity:X} {s.u32(cell + 0xC) if cell else 0:X} " + " ".join(f"{s.u32(ref + o):08X}" for o in (0x2C, 0x30, 0x34, 0x20, 0x24, 0x28)))
    # Commands 48 to 52's SetUnconscious: the death state +0xB0.
    for identity in (0x1ECC4, 0x1ECC1, 0x1ECC3, 0x1ECC2, 0x1ECC0):
        rows.append(f"DEADSTATE {identity:X} {s.u32(forms[identity] + 0xB0)}")
    # The priests frame 88's drain disables, with command 33's 90013E6: their processes are freed.
    for identity in (0xBC6A, 0xBC6B, 0xBC6C):
        process = s.u32(forms[identity] + 0x58)
        rows.append(f"PROCESS {identity:X} {s.u32(process) if process else 0:X}")
    for identity, form in sorted(forms.items()):
        kind = s.u8(form + 4)
        if kind == 0x3B:
            done = [s.u8(group) for group in inline_list(s, form + 0x40) if s.u8(group + 1)]
            rows.append(f"QUEST {identity:X} {s.u8(form + 0x3C):X} {s.u8(form + 0x5C)} " + ",".join(map(str, done)))
        elif 49 <= kind <= 51:
            flags = s.u32(form + 8)
            if flags & 0x800:
                rows.append(f"DISABLED {identity:X}")
    with open(sys.argv[3], "w") as out:
        out.write("\n".join(rows) + "\n")
    print(len(rows), "rows")


if __name__ == "__main__":
    main()
