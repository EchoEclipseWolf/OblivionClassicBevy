#!/usr/bin/env python3
"""The prison casters' active effects from run-af's `after_move` snapshot (005172C8, after command 64 returns).

after_move_effects_rows.py SNAPSHOT OUT.rows
One row per ActiveEffect in each caster's HighProcess list (process vt+0x298 00629350: the list at *(+0x174)), in list
order, casters in run-af's finish order: `EFFECT <caster> <class> <item id> <code> <+0x14 flags> <+0x18 bits> <+0x1C bits>
<+0x20> <+0x24> <+0x28> <+0x38>`, hex. +0x20 and +0x24 are written as `<actor id>+<offset>` (the actor's MagicTarget or
MagicCaster part). The ctor (0068D7A0) leaves +0x13 unset, so it isn't written. `the_port_runs_the_new_game_commands`
reads these rows.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ledger_compare import FORMS_MAP, walk_map  # noqa: E402
from snapshot import Snapshot  # noqa: E402

CASTERS = [0x1FC43, 0x32A15, 0x32A16, 0x32A17, 0x32A18, 0x14A2C, 0x14A30, 0x178C3, 0x178C4]
CLASSES = {0xA76D34: "ValueModifierEffect", 0xA75344: "BoundItemEffect"}


def main():
    snap = Snapshot(sys.argv[1])
    forms = {key: value for _, key, value in walk_map(snap, FORMS_MAP)}
    refs = {forms[i]: i for i in CASTERS}

    def part(pointer):
        for ref, form_id in refs.items():
            if 0 <= pointer - ref < 0x100:
                return f"{form_id:X}+{pointer - ref:X}"
        return f"{pointer:X}"

    lines = []
    for caster in CASTERS:
        process = snap.u32(forms[caster] + 0x58)
        node = snap.u32(process + 0x174)
        while node:
            effect = snap.u32(node)
            if effect:
                vtable = snap.u32(effect)
                item = snap.u32(effect + 8)
                effect_item = snap.u32(effect + 0xC)
                code = snap.read(effect_item, 4).decode("latin-1")
                # The item's form ID: +0x08 is the MagicItem part of a SpellItem, 0x18 into the form.
                item_id = snap.u32(item - 0x18 + 0xC)
                bound = snap.u32(effect + 0x38)
                extra = f"{snap.u32(bound + 0xC):X}" if vtable == 0xA75344 and bound else f"{bound:X}"
                lines.append(
                    f"EFFECT {caster:X} {CLASSES.get(vtable, hex(vtable))} {item_id:X} {code} {snap.u32(effect + 0x14):X} "
                    f"{snap.u32(effect + 0x18):08X} {snap.u32(effect + 0x1C):08X} {part(snap.u32(effect + 0x20))} "
                    f"{part(snap.u32(effect + 0x24))} {snap.u32(effect + 0x28):X} {extra}")
            node = snap.u32(node + 4)
    Path(sys.argv[2]).write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
