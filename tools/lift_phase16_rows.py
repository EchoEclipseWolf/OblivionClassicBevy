#!/usr/bin/env python3
"""Lifts fixtures/rows/phase16.rows from the three raw runs in fixtures/evidence.

Phase 16 has no imported .inc: the C++ test held its values by hand. Each row is
`KIND RUN TAG seq=N key=value ...`; every key=value is copied verbatim from record N, which the test
`the_rows_are_the_raw_records` proves. BB (one-shot block) records aren't lifted: no host state holds them.
"""
from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / "fixtures"
RUNS = {
    "P16-RUN-001": "phase16-run001-2026-10-10",
    "P16-RUN-002": "phase16-run002-2026-10-10",
    "P16-RUN-003": "phase16-run003-2026-10-10",
}
KINDS = {
    "WIN": ["rect", "rotation"],
    "TES": ["t30", "t34", "t74", "t20", "grids", "centre", "coc"],
    "PLAYER": ["id", "flags", "rot", "pos", "scale", "parent", "b3c", "procvt", "dead", "extra", "raw"],
    "PLAYERX": ["p588", "p590", "p5d0", "p6e8", "p71e", "p730"],
    "CTRL": ["f1f4", "f320"],
    "SL": ["m0"],
    "IO": ["raw", "qraw", "c24", "c28raw"],
    "TL": ["b184", "b185", "b186"],
    "CELL": ["id", "type", "flags", "b24", "b25", "b26", "ws", "data", "land", "pgrd"],
    "REFS": ["cell", "count"],
}
REF_TAGS = ("insert", "end_cell_full", "prepare")
REF_KEYS = ["n", "ref", "id", "type", "flags", "base", "btype", "pos", "rot", "scale", "parent", "extra"]
VALUES = {
    "cee90": ["id", "posv", "rotv"],
    "door": ["pos", "rot"],
    "chosen": ["id", "type", "base", "btype", "pos", "rot", "cands"],
    "after_choice": ["pos", "rot", "cands", "door"],
    "final": ["pos"],
    "final_lift": ["pos"],
    "land_call": ["id", "pos"],
    "height_entry": ["pos"],
    "tri": ["block"],
    "heights": ["height", "water", "z"],
    "zrot": ["arg", "dead"],
    "set_rz": ["arg", "ret"],
    "setpos": ["pos"],
    "ctrl": ["f1f4", "f320"],
    "insert_entry": ["id", "flags", "b24", "b26"],
    "cell_regions": ["id"],
    "model_queue": ["a1id", "a2", "ret"],
    "d_iom": ["eax"],
    "d_total": ["ebx", "ecx", "edx"],
    "d_progress": ["n", "ebx", "s0", "s1"],
    "d_loop": ["n", "eax"],
    "d_end": ["n", "eax"],
}


def row_of(run, line):
    parts = line.split(" ")
    seq, kind = parts[0], parts[1]
    fields = {}
    for token in parts[2:]:
        if "=" in token:
            key, value = token.split("=", 1)
            fields.setdefault(key, value)
    keep = None
    tag = parts[2]
    if kind == "V":
        keep = VALUES[tag]
    elif kind in KINDS:
        keep = KINDS[kind]
    elif kind == "REF" and tag in REF_TAGS:
        keep = REF_KEYS
    elif kind == "WB":
        return f"WB {run} {parts[3]} seq={seq} at={tag}"
    elif kind == "M" and tag == "END":
        return f"END {run} {parts[3]} seq={seq}"
    if keep is None:
        return None
    return f"{kind} {run} {tag} seq={seq} " + " ".join(f"{key}={fields[key]}" for key in keep)


def lift():
    rows = []
    for run, name in RUNS.items():
        text = (OUT / "evidence" / f"{name}.records.txt").read_text(errors="replace")
        for line in text.splitlines():
            row = row_of(run, line)
            if row:
                rows.append(row)
    target = OUT / "rows" / "phase16.rows"
    target.write_text("\n".join(rows) + "\n", newline="\n")
    return target


if __name__ == "__main__":
    print(lift())
