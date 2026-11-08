#!/usr/bin/env python3
"""The prison NPCs' skin info slots (Character +0x104), from run-af's `after_move` snapshot.

skin_slots_rows.py SNAPSHOT OUT.rows
SNAPSHOT is `snapshot-after_move` (005172C8, after command 64 returns). One row per filled slot of each NPC (base type 0x23)
in cell 1FBB9's +0x48 list, in list order: `SKIN <ref id> <slot> <form id> <model path>`, hex but the path. The skin
info's slots are 16 bytes from +0x4C: +0x00 the form, +0x04 its TESModel (+0x04 the BSStringT's text). An empty
path is `-`. `the_port_runs_the_new_game_commands` reads these rows.
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
        if not base or snap.u8(base + 4) != 0x23:
            continue
        skin = snap.u32(reference + 0x104)
        ref_id = snap.u32(reference + 0x0C)
        if not skin:
            lines.append(f"SKIN {ref_id:X} none")
            continue
        for slot in range(16):
            entry = skin + 0x4C + 16 * slot
            form = snap.u32(entry)
            if not form:
                continue
            model = snap.u32(entry + 4)
            text = snap.u32(model + 4) if model else 0
            path = snap.text(text) if text else ""
            lines.append(f"SKIN {ref_id:X} {slot:X} {snap.u32(form + 0x0C):X} {path or '-'}")
    Path(sys.argv[2]).write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
