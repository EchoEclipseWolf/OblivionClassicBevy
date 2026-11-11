#!/usr/bin/env python3
"""Migrate legacy StreamError call sites. Dry-run by default; pass --write to apply.

Structural definition and tuple-field reads are reviewed separately. An address prefix
identifies the site, never the kind. Only reviewed (source file, message) pairs
and direct byte-reader bounds checks override the unclassified default. String contents are preserved.
"""
import argparse
from collections import Counter
from pathlib import Path
import re

from rust_source import token_spans

ROOT = Path(__file__).resolve().parents[1]
SITE = re.compile(r'"([0-9A-Fa-f]{8})(?=[: "])')
READER = "crates/oblivion_core/src/reader.rs"
# actor_point_cost explicitly stops at each unported water arm. DataHandler_AddFormToList
# records the pending REFR arm; parse_object_net_payload bounds-checks the link array.
REVIEWED = {
    ("crates/oblivion_pathing/src/cost.rs", "005EC24F: 005E06C0 for a point under water"): ("stop", 0x005EC24F),
    ("crates/oblivion_pathing/src/cost.rs", "005EC33C: 00519C70 for a swimming point"): ("stop", 0x005EC33C),
    ("crates/oblivion_pathing/src/cost.rs", "005EC3A0: 005E3400 for a wading point"): ("stop", 0x005EC3A0),
    ("crates/oblivion_sim/src/tes/data/data_load.rs", "Phase-14 pending: 0044DB24 the REFR arm of 0044D950"): ("stop", 0x0044DB24),
    ("crates/oblivion_sim/src/gamebryo/object_net.rs", "truncated NiObjectNET links"): ("stream", 0),
}


def convert(path, source, counts, dynamic):
    if "StreamError" not in source:
        return source
    tokens = list(token_spans(source))
    changes = []
    for i, (word, start, end) in enumerate(tokens):
        if word != "StreamError":
            continue
        if path == READER and start < source.find("pub struct ByteReader"):
            continue  # the old tuple definition and its trait implementations
        if tokens[i + 1:i + 2] and tokens[i + 1][0] == "(":
            arg = tokens[i + 2][0]
            end = tokens[i + 1][2]
            if arg.startswith('"') and tokens[i + 3][0] in (",", ")"):
                site = SITE.match(arg)
                address = int(site[1], 16) if site else 0
                kind, address = REVIEWED.get((path, arg[1:-1]), ("unclassified", address))
                # The count exceeds the remaining input buffer at these byte-reader guards.
                if arg.startswith('"truncated ') and re.search(
                    r"if [^{}\n]*reader\.remaining\(\)[^{}\n]*\{\s*return Err\($",
                    source[max(0, start - 240):start],
                ):
                    kind = "stream"
                replacement = f"GameError::{kind}("
                if kind != "stream":
                    replacement += (f"0x{address:08X}" if address else "0") + ", "
            else:
                line = source.count("\n", 0, start) + 1
                dynamic.append((path, line, arg))
                if path == READER:
                    kind, replacement = "stream", "GameError::stream("
                elif path.endswith("/tes/data/data_load.rs") and arg == "Box":
                    kind, replacement = "stop", "GameError::stop(0x0044DE2E, "
                else:
                    kind, replacement = "dynamic", "GameError::from_legacy_message("
            counts[kind] += 1
        else:
            replacement = "GameError"
            counts["type references"] += 1
        if source[end:end + 1].isspace():
            replacement = replacement.rstrip()
        changes.append((start, end, replacement))
    for start, end, replacement in reversed(changes):
        source = source[:start] + replacement + source[end:]
    source = source.replace("oblivion_core::reader::GameError", "oblivion_core::error::GameError")
    return re.sub(
        r"(?m)^([ \t]*)use oblivion_core::reader::\{ByteReader, GameError\};",
        r"\1use oblivion_core::error::GameError;\n\1use oblivion_core::reader::ByteReader;",
        source,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--dry-run", action="store_true", help="inventory without writing (default)")
    mode.add_argument("--write", action="store_true", help="apply the source conversion")
    parser.add_argument("--dynamic", action="store_true", help="list all variable or computed messages")
    args = parser.parse_args()
    counts = Counter()
    dynamic = []
    for folder in ("crates", "src", "tests"):
        for path in sorted((ROOT / folder).rglob("*.rs")):
            source = path.read_text()
            updated = convert(path.relative_to(ROOT).as_posix(), source, counts, dynamic)
            if source != updated:
                counts["files"] += 1
                if args.write:
                    path.write_text(updated)
    if args.dynamic:
        for path, line, argument in dynamic:
            print(f"{path}:{line}: computed message ({argument})")
    if not counts:
        print("No legacy error sites to convert.")
    for kind, count in sorted(counts.items()):
        print(f"{kind}: {count}")


if __name__ == "__main__":
    main()
