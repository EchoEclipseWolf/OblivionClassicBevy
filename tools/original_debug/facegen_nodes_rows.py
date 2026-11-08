#!/usr/bin/env python3
"""The prison NPCs' FaceGen heads, from run-af's `after_move` snapshot.

facegen_nodes_rows.py SNAPSHOT OUT.rows
SNAPSHOT is `snapshot-after_move` (005172C8, after command 64 returns). For each NPC reference (base type 0x23) in cell
1FBB9's +0x48 list, in list order:

  FGNPC <ref> <base> <race> <hair> <eyes> <+0x1CC hair length bits> <+0x1E8 hair colour> <+0x1E0>
  FGCOEF <base> <npc108|npc168|race29c> <k> <rows> <cols> <float bits...>   one per FgMatrix (0x18: +0 rows, +4 cols,
                                                                             +0x0C the vector's first, +0x10 its last)
  FGPART <race> <part> <model path> <texture path>                        the race's +0xE0 + 0x18 i TESModel and +0x1B8 +
                                                                             0x0C i TESTexture (+0x04 the path), `-` empty
  FGHAIR <hair> <model path>                                                 TESHair +0x24 TESModel
  FGNODE <ref> <biped|skinned> <index> <name> <class> <vertex count> <v0 bits x3> <vlast bits x3> <fnv1a of all positions>

TESNPC +0x1D4 is the BSFaceGenNiNodeBiped, +0x1D8 the BSFaceGenNiNodeSkinned (00528A10). A node's children are its NiTArray
at +0xAC (+0xB0 data, +0xB6 the used count); a geometry's data is +0xB4, whose +0x08 is the vertex count and +0x1C the
positions. The test `facegen_heads_match_after_move` reads these rows. Hex throughout.
"""
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ledger_compare import FORMS_MAP, walk_map  # noqa: E402
from snapshot import Snapshot  # noqa: E402

PRISON = 0x1FBB9
CLASSES = {0xA7ED5C: "NiTriShape", 0xA7F27C: "NiTriStrips"}


def fnv1a(data):
    value = 0x811C9DC5
    for byte in data:
        value = ((value ^ byte) * 0x01000193) & 0xFFFFFFFF
    return value


def main():
    snap = Snapshot(sys.argv[1])
    forms = {key: value for _, key, value in walk_map(snap, FORMS_MAP)}
    cell = forms[PRISON]

    def form_id(address):
        return snap.u32(address + 0x0C) if address else 0

    def name(address):
        text = snap.u32(address + 8)
        return snap.text(text) if text else "-"

    def matrices(base, offset, label, lines):
        for k in range(4):
            matrix = base + offset + 0x18 * k
            rows, cols = snap.u32(matrix), snap.u32(matrix + 4)
            first, last = snap.u32(matrix + 0x0C), snap.u32(matrix + 0x10)
            count = (last - first) // 4 if first else 0
            words = [snap.u32(first + 4 * i) for i in range(count)]
            lines.append(f"FGCOEF {form_id(base):X} {label} {k} {rows:X} {cols:X} " + " ".join(f"{w:X}" for w in words))

    def path(address):
        text = snap.u32(address + 4)
        return (snap.text(text) if text else "") or "-"

    lines = []
    seen = set()
    races = set()
    hairs = set()
    for reference in snap.walk_list(cell + 0x48):
        base = snap.u32(reference + 0x1C)
        if not base or snap.u8(base + 4) != 0x23:
            continue
        ref_id = form_id(reference)
        race = snap.u32(base + 0xE8)
        lines.append(
            f"FGNPC {ref_id:X} {form_id(base):X} {form_id(race):X} {form_id(snap.u32(base + 0x1C8)):X} "
            f"{form_id(snap.u32(base + 0x1D0)):X} {snap.u32(base + 0x1CC):X} {snap.u32(base + 0x1E8):X} "
            f"{int.from_bytes(snap.read(base + 0x1E0, 2), 'little'):X}"
        )
        if race not in races:
            races.add(race)
            for part in range(9):
                lines.append(f"FGPART {form_id(race):X} {part} {path(race + 0xE0 + 0x18 * part)} {path(race + 0x1B8 + 0x0C * part)}")
        hair = snap.u32(base + 0x1C8)
        if hair and hair not in hairs:
            hairs.add(hair)
            lines.append(f"FGHAIR {form_id(hair):X} {path(hair + 0x24)}")
        if base not in seen:
            seen.add(base)
            matrices(base, 0x108, "npc108", lines)
            matrices(base, 0x168, "npc168", lines)
            matrices(race, 0x29C, "race29c", lines)
        for label, offset in (("biped", 0x1D4), ("skinned", 0x1D8)):
            node = snap.u32(base + offset)
            if not node:
                lines.append(f"FGNODE {ref_id:X} {label} none")
                continue
            data = snap.u32(node + 0xB0)
            used = int.from_bytes(snap.read(node + 0xB6, 2), "little")
            for index in range(used):
                child = snap.u32(data + 4 * index)
                if not child:
                    continue
                geometry = snap.u32(child + 0xB4)
                count = int.from_bytes(snap.read(geometry + 8, 2), "little")
                positions = snap.u32(geometry + 0x1C)
                raw = snap.read(positions, 12 * count) if positions and count else b""
                words = struct.unpack(f"<{3 * count}I", raw) if raw else ()
                first = " ".join(f"{w:X}" for w in words[:3]) or "- - -"
                final = " ".join(f"{w:X}" for w in words[-3:]) or "- - -"
                kind = CLASSES.get(snap.u32(child), f"{snap.u32(child):X}")
                lines.append(
                    f"FGNODE {ref_id:X} {label} {index:X} {name(child)} {kind} {count:X} {first} {final} {fnv1a(raw):X}"
                )
    Path(sys.argv[2]).write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
