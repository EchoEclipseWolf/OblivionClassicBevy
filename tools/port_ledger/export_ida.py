"""Run export(db, directory) inside the live IDA session. Only the CSV outputs are written."""

import csv
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path

import ida_funcs
import ida_loader
import idautils


def export(db, directory):
    database = ida_loader.get_path(ida_loader.PATH_TYPE_IDB)
    if database != "/home/natfoth/Documents/OblivionIDA/Oblivion.exe.i64":
        raise ValueError(f"Expected the live Oblivion database, found {database}")
    functions, split_chunks = [], []
    for function in db.functions.get_all():
        entry = function.start_ea
        size = ida_funcs.calc_func_size(function)
        chunks = sorted(idautils.Chunks(entry))
        if sum(end - start for start, end in chunks) != size:
            raise ValueError(f"Inconsistent function size at {entry:08X}")
        functions.append((f"{entry:08X}", db.functions.get_name(function), size))
        if chunks != [(entry, entry + size)]:
            split_chunks.extend(
                (f"{entry:08X}", f"{start:08X}", f"{end:08X}") for start, end in chunks
            )
    directory = Path(directory)
    hashes = {}
    for filename, header, rows in (
        ("ida_functions.csv", ("entry", "name", "size"), functions),
        ("ida_split_chunks.csv", ("entry", "start", "end_exclusive"), split_chunks),
    ):
        path = directory / filename
        with path.open("w", newline="", encoding="utf-8") as output:
            writer = csv.writer(output, lineterminator="\n")
            writer.writerow(header)
            writer.writerows(sorted(rows))
        hashes[filename] = hashlib.sha256(path.read_bytes()).hexdigest()
    metadata = {"database": database, "export_date_utc": str(datetime.now(timezone.utc).date()), "sha256": hashes}
    (directory / "export.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    return len(functions), len(split_chunks)
