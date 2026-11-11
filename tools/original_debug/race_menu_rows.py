#!/usr/bin/env python3
"""The race menu changing, step by step, from run-aq's records (`records-run-aq-racemenu.txt`, Build 5's C7).

race_menu_rows.py RECORDS OUT.rows
For each step of the first pass (from `start` to `reset`), the lines after its 10 frames, hex:
  NPC <step> <race> <hair> <eyes> <+0x1CC length bits> <+0x1E8 colour>
  FGCOEF <step> <npc108|npc168> <k> <rows> <cols> <float bits...>
  TILE <step> <pane> <child> <user0 bits>        the clicked tile's value (a child name's spaces as '_')
  FGNODE <step> <biped|skinned> <i> <name> <vertex count> <fnv1a of the positions>   the head nodes' parts
The first `start` dump is the race menu before any click. Steps after `reset` (the boxes' later clicks) are left out.
The replay tests in `winmain_driver` read these rows.
"""
import sys

FIRST_PASS_END = "reset"


def main():
    records, out = sys.argv[1], sys.argv[2]
    rows = []
    seen = set()
    current = None
    for line in open(records, encoding="ascii"):
        parts = line.split()
        if len(parts) < 3:
            continue
        kind, step = parts[1], parts[2]
        if kind == "NPC":
            if current == FIRST_PASS_END:
                break
            current = None if step in seen else step
            if current is None:
                continue
            seen.add(step)
            fields = dict(part.split("=", 1) for part in parts[3:8])
            rows.append(f"NPC {step} {fields['race']} {fields['hair']} {fields['eyes']} {fields['len']} {fields['col']}")
        elif kind == "FGCOEF" and step == current:
            rows.append("FGCOEF " + " ".join(parts[2:]))
        elif kind == "FGNODE" and step == current:
            rows.append("FGNODE " + " ".join(parts[2:]))
        elif kind == "TILE" and step not in seen:
            # TILE <step> <pane> <child words...> user0=<bits>, logged before its step's NPC line.
            value = parts[-1].split("=", 1)[1]
            rows.append(f"TILE {step} {parts[3]} {'_'.join(parts[4:-1])} {value}")
    open(out, "w", encoding="ascii").write("\n".join(rows) + "\n")


if __name__ == "__main__":
    main()
