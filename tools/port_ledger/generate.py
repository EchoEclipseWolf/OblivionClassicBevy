#!/usr/bin/env python3
"""Generate the Oblivion source-site ledger, or verify the checked-in reports with --check."""

import argparse
import csv
import hashlib
import io
import json
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass, fields
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.dont_write_bytecode = True
sys.path.insert(0, str(HERE.parent))
import boundary_audit as audit
import layering
from rust_source import token_spans

OUTPUT = audit.REPO / "docs/port-ledger"
ADDRESS = re.compile(r"(?<![\w+])(?:0x)?(0[0-9A-Fa-f]{3}_?[0-9A-Fa-f]{4})(?!\w)")
SIGNATURE = re.compile(r"[({;\[]")
FN = re.compile(r"\bfn\s+(\w+)\b")
LABEL = re.compile(r"^//[/!]?\s*IDA:\s*(.*)")
TODO = re.compile(r"\bTODO:\s*\[ \]")
ERROR = re.compile(r"\bGameError\s*::\s*(stop|original|unclassified|stream|from_legacy_message)\s*\(")
COUNTER = re.compile(r"\b(count_idle_call|count_face|count|boundaries|counted_boundaries\s*\.\s*push)\s*\(")
# Non-executable PE section virtual ranges, from the pinned 1.2.0416 evidence image.
DATA = [(0xA28000, 0xB018D2), (0xB02000, 0xBABC1C), (0xBAC000, 0xBAC1B9), (0xBAD000, 0xBAE66C)]


def addresses(text):
    return [m[1].replace("_", "").upper() for m in ADDRESS.finditer(text)
            if audit.in_image(m[1].replace("_", ""))]


class Functions:
    def __init__(self, entries, split):
        self.entries = entries
        self.pages = defaultdict(list)
        self.ranges = {}
        for address, (_, size) in entries.items():
            start = int(address, 16)
            chunks = split.get(address, [(start, start + size)])
            if sum(b - a for a, b in chunks) != size:
                raise ValueError(f"Chunk size mismatch at {address}")
            self.ranges[address] = chunks
            for a, b in chunks:
                for page in range(a >> 12, ((b - 1) >> 12) + 1):
                    self.pages[page].append((a, b, address))

    def owners(self, address):
        if not address:
            return []
        value = int(address, 16)
        return sorted({ea for a, b, ea in self.pages[value >> 12] if a <= value < b})

    def bytes(self, entries):
        """Union length counts a tail shared by several functions only once."""
        last = total = 0
        for start, end in sorted(chunk for ea in set(entries) for chunk in self.ranges[ea]):
            total += max(0, end - max(last, start))
            last = max(last, end)
        return total


def load_export():
    metadata = json.loads((HERE / "export.json").read_text())
    for file, expected in metadata["sha256"].items():
        if hashlib.sha256((HERE / file).read_bytes()).hexdigest() != expected:
            raise ValueError(f"IDA export hash mismatch for {file}")
    with (HERE / "ida_functions.csv").open() as src:
        entries = {row["entry"]: (row["name"], int(row["size"])) for row in csv.DictReader(src)}
    split = defaultdict(list)
    with (HERE / "ida_split_chunks.csv").open() as src:
        for row in csv.DictReader(src):
            split[row["entry"]].append((int(row["start"], 16), int(row["end_exclusive"], 16)))
    return Functions(entries, split), metadata


@dataclass
class Row:
    address: str
    file: str
    line: int
    kind: str
    name: str
    enclosing_definition: str = ""
    enclosing_entry: str = ""
    assessment: str = ""
    todo_count: int = 0
    evidence: str = ""
    audit_review: str = ""
    address_in_ida_functions: str = ""


def normalize(name):
    return name.replace("_", "").lower()


def assessment(doc, todos):
    matches = list(re.finditer(r"\bCompletion:\s*(\d{1,3})%([^.;\n]*)", doc, re.I))
    if len(matches) != 1:
        return "unassessed"
    match = matches[0]
    if int(match[1]) < 100 or todos or re.search(r"\b(except|but|excluding)\b", match[2], re.I):
        return "partial"
    if int(match[1]) != 100:
        return "unassessed"
    if match[2].strip(" )("):
        return "complete within stated scope"
    return "100% annotation, scope unspecified"


# Reviewed forwarding methods take locks, supply a host handle or adapt the return type.
# Their matching names alone would double-count the implementation they call.
FORWARDERS = {
    ("crates/oblivion_sim/src/tes/winmain/winmain_host.rs", "open_core", "00430EF0"),
    ("crates/oblivion_sim/src/tes/winmain/winmain_host.rs", "open_as_bs_file", "00431130"),
    ("crates/oblivion_sim/src/tes/winmain/winmain_host.rs", "find_file", "00431020"),
    ("crates/oblivion_sim/src/tes/winmain/winmain_host.rs", "random_next", "0047DF80"),
    ("crates/oblivion_sim/src/tes/os/os_globals.rs", "pause_game_sounds", "006A9B40"),
    ("crates/oblivion_sim/src/tes/os/os_globals.rs", "resume_game_sounds", "006A9C00"),
}


def declaration(functions, address, label, name):
    if label.startswith("N/A"):
        return "host helper"
    if not address:
        return "unresolved annotation"
    if not re.match(r"(?:0x)?0[\da-fA-F]{3}_?[\da-fA-F]{4}\b", label):
        return "citation"
    if address not in functions.entries:
        if functions.owners(address):
            return "instruction site"
        if any(a <= int(address, 16) < b for a, b in DATA):
            return "data reference"
        return "unresolved site"
    ida = functions.entries[address][0]
    lead = re.match(r"(?:0x)?0[\da-fA-F]{3}_?[\da-fA-F]{4}\b[ \t]+`?([\w:]+)", label)
    if ida.startswith(("sub_", "nullsub_", "j_")):
        return "ambiguous definition"
    action = ida.rsplit("_", 1)[-1]
    # The Rust function independently identifies either the full symbol or its named action.
    # For an action alone the attached leading annotation must name the class-qualified symbol.
    # A conflicting symbol in the leading annotation makes the identity unresolved.
    if lead and "_" in lead[1] and normalize(lead[1].rstrip(":")) != normalize(ida):
        return "ambiguous definition"
    if normalize(name) == normalize(ida) or (
        lead and lead[1].rstrip(":") == ida and normalize(name) == normalize(action)
    ):
        return "definition"
    return "ambiguous definition"


def delimiters(text, code):
    stack, closes = [], {}
    for token, start, _ in token_spans(text):
        if code[start].isspace():
            continue
        if token in ("(", "{", "["):
            stack.append((token, start))
        elif token in (")", "}", "]"):
            opening, pos = stack.pop()
            if opening != {")": "(", "}": "{", "]": "["}[token]:
                raise ValueError(f"Unbalanced source delimiter at {start}")
            closes[pos] = start
    return closes


def scan(source, functions):
    code, text = source.code, source.text
    lines = text.splitlines()
    closes = delimiters(text, code)
    attribute_starts = {end: start - 1 for start, end in closes.items() if code[max(0,start-1):start+1] == "#["}
    # Each comment keeps its actual offset. This excludes comments masked from cfg(test) modules.
    by_line = {source.line(pos): (pos, c) for pos, c in source.comments}
    owners, annotations, rows = {}, set(), []
    traits = []
    for trait in re.finditer(r"\btrait\s+\w+[^;{]*\{", code):
        start = trait.end() - 1
        if start in closes:
            traits.append((start, closes[start]))
    for m in FN.finditer(code):
        # Skip parameter lists and array return types (their semicolons are inside brackets).
        pos = m.end()
        while body := SIGNATURE.search(code, pos):
            opening = body.start()
            if code[opening] in "{;":
                break
            pos = closes.get(opening, opening) + 1
        if not body:
            continue
        is_trait = any(start < m.start() < end for start, end in traits)
        if code[opening] == ";" and not is_trait:
            continue
        if code[opening] == "{" and opening not in closes:
            continue
        line = source.line(m.start())
        first = line
        # Rust attributes can span lines. Walk backward over whole balanced attribute brackets.
        while first > 1:
            prev = first - 1
            if prev in by_line and lines[prev-1].lstrip().startswith("//") and not lines[prev-1].lstrip().startswith("//!"):
                first = prev
                continue
            end = source.line_starts[prev-1] + len(lines[prev-1].rstrip()) - 1
            if end not in attribute_starts:
                break
            first = source.line(attribute_starts[end])
        block = [(n, by_line[n][1]) for n in range(first, line) if n in by_line]
        labels = [(n, LABEL.match(c)[1]) for n, c in block if LABEL.match(c)]
        name = m[1]
        entry = ""
        if labels:
            at, label = labels[0]
            found = addresses(label)
            address = found[0] if found and not label.startswith("N/A") else ""
            kind = declaration(functions, address, label, name)
            if address and is_trait:
                kind = "trait contract"
            elif (source.relative, name, address) in FORWARDERS:
                kind = "host adapter"
            doc = " ".join(re.sub(r"^//[/!]?\s*", "", c) for _, c in block)
            todos = len(TODO.findall(doc))
            if kind == "definition":
                entry = address
            rows.append(Row(address, source.relative, at, kind, name, name, entry,
                            assessment(doc, todos) if kind == "definition" else "unassessed",
                            todos, doc))
            annotations.add((at, address))
        for n in range(first, source.line(closes.get(opening, opening)) + 1):
            owners[n] = name, entry

    def add(line, address, kind, name, evidence):
        owner, entry = owners.get(line, ("", ""))
        rows.append(Row(address, source.relative, line, kind, name, owner, entry, todo_count=int(kind == "TODO"), evidence=evidence))

    for pos, comment in source.comments:
        line = source.line(pos)
        label = LABEL.match(comment)
        found = addresses(comment)
        if TODO.search(comment):
            for address in dict.fromkeys(found or [""]):
                add(line, address, "TODO", "TODO", comment.strip())
        if label and not found and (line, "") not in annotations:
            add(line, "", "host annotation" if label[1].startswith("N/A") else "unresolved annotation", "", comment.strip())
        for index, address in enumerate(found):
            if index == 0 and (line, address) in annotations:
                continue
            kind = "citation"
            if any(a <= int(address, 16) < b for a, b in DATA):
                kind = "data reference"
            elif not label and index == 0 and re.match(r"^//[/!]?\s*(?:0x)?" + address + r"\b", comment, re.I) and functions.owners(address) and address not in functions.entries:
                kind = "instruction site"
            add(line, address, kind, functions.entries.get(address, ("", 0))[0], comment.strip())

    for m in ERROR.finditer(code):
        end = closes.get(m.end()-1)
        if end is None:
            continue
        arg = text[m.end():end].strip()
        if m[1] in {"stop", "original", "unclassified"}:
            token = arg.split(",", 1)[0].strip()
            address = next(iter(addresses(token)), "")
        else:
            prefix = re.match(r'"([0-9a-fA-F]{8})(?=[: "])', arg)
            address = prefix[1].upper() if prefix else ""
        kind = "unclassified" if m[1] == "from_legacy_message" else m[1]
        add(source.line(m.start()), address, f"error {kind}", m[1], " ".join(arg.split()))
    for m in COUNTER.finditer(code):
        end = closes.get(m.end()-1)
        if end is None or re.search(r"\bfn\s*$", code[max(0, m.start()-12):m.start()]):
            continue
        args = code[m.end():end].strip()
        if not args:
            continue
        found = addresses(args)
        if not found and re.fullmatch(r"world\s*,\s*address", args):
            loop = re.search(r"for\s+address\s+in\s*\[([^\]]+)\]\s*\{\s*$", code[max(0,m.start()-600):m.start()])
            if loop:
                found = addresses(loop[1])
        if not found and m[1] == "count" and not re.match(r"world\s*,", args):
            continue
        for address in dict.fromkeys(found or [""]):
            add(source.line(m.start()), address, "counted call", m[1], " ".join(args.split()))
    return rows, owners


def add_calls(rows, sources, owners):
    """Resolve free calls through the same Rust module and re-export tree as the layering audit."""
    modules = layering.Sources(audit.REPO)
    for crate in layering.CRATES:
        modules.read((crate,), Path("crates", crate, "src/lib.rs"))
    resolver = layering.Resolver(modules)
    by_file = defaultdict(list)
    for module, path in modules.modules.items():
        by_file[str(path)].append(module)
    verified = defaultdict(set)
    for row in rows:
        for module in by_file.get(row.file, ()):
            if row.kind == "definition" and row.name in modules.declarations[module]:
                verified[module + (row.name,)].add(row.address)
    verified_names = {target[-1] for target in verified}
    call = re.compile(r"(?<![.\w:])((?:\b\w+\s*::\s*)*\b\w+)\s*\(")
    for source in sources:
        candidates = by_file.get(source.relative, ())
        if len(candidates) != 1:
            continue
        module = candidates[0]
        scope = modules.scopes[module]
        for match in call.finditer(source.code):
            path = tuple(re.split(r"\s*::\s*", match[1]))
            if path[-1] not in verified_names:
                continue
            # Absolute and relative module paths have unambiguous scope. A bare local can
            # shadow a free function, so leave it unresolved unless it has an explicit path.
            if path[0] not in {"self", "super", "crate", *layering.CRATES}:
                continue
            target = resolver.resolve(scope, path)
            entries = verified.get(target, ())
            if len(entries) == 1:
                line = source.line(match.start())
                owner, entry = owners[source.relative].get(line, ("", ""))
                rows.append(Row(next(iter(entries)), source.relative, line, "resolved call",
                                path[-1], owner, entry, evidence="::".join(path)))


def attach_audit(rows, sources, owners):
    review = audit.load_review()
    boundaries = audit.RustEvidence(sources)
    boundaries.finish_named_counters(sources)
    indexed = defaultdict(set)
    for row in rows:
        if row.kind == "counted call":
            indexed[row.address].add(f"{row.file}:{row.line}")
    for address, location, how in boundaries.all_counters:
        if how.startswith("field ") or location not in indexed[address]:
            file, line = location.rsplit(":", 1)
            owner, entry = owners.get(file, {}).get(int(line), ("", ""))
            rows.append(Row(address, file, int(line), "named counter" if how.startswith("field ") else "audit counter reference",
                            how, owner, entry, evidence="boundary_audit.py"))
    # Keep every named increment visible even when the independent audit lists only the first.
    for name, (entries, _, increments) in boundaries.named_counters.items():
        for location in increments[1:]:
            file, line = location.rsplit(":", 1)
            owner, entry = owners[file].get(int(line), ("", ""))
            for address in entries:
                rows.append(Row(address, file, int(line), "named counter", "field " + name, owner, entry))
    for table, kind in ((boundaries.stops, "audit diagnostic"), (boundaries.panics, "panic diagnostic")):
        for address, occurrences in table.items():
            for location, message in occurrences:
                file, line = location.rsplit(":", 1)
                owner, entry = owners[file].get(int(line), ("", ""))
                rows.append(Row(address, file, int(line), kind, "", owner, entry, evidence=message))

    # Hand decisions remain independent from the source-site classification. Include decisions whose
    # only evidence is outside the Rust source, without granting them a Rust definition.
    review_file = audit.REVIEW.relative_to(audit.REPO).as_posix()
    review_lines = audit.REVIEW.read_text().splitlines()
    locations = defaultdict(list)
    for number, line in enumerate(review_lines, 1):
        for address in addresses(line):
            locations[address].append(number)
    for address, item in review["items"].items():
        # The JSON lists each decision in the addresses array. A prose note may also cite the same address.
        token = re.compile(r'^\s*"' + address + r'"(?:,|\s*$)')
        at = next((n for n in locations[address] if token.match(review_lines[n - 1])), locations[address][0])
        rows.append(Row(address, review_file, at, "reviewed disposition",
                        item["class"], evidence=item["decision"] + " " + item["note"]))
    for row in rows:
        item = review["items"].get(row.address)
        if item:
            row.audit_review = item["class"] + " / " + item["decision"]
    return boundaries, review


def duplicates(rows):
    grouped = defaultdict(list)
    for row in rows:
        if row.kind == "definition":
            grouped[row.address].append(row)
    return {entry: sites for entry, sites in grouped.items() if len(sites) > 1}


def reviewed_duplicates(groups, path):
    decisions = {}
    for line in path.read_text().splitlines():
        if line and not line.startswith("#"):
            entry, reason = line.split(maxsplit=1)
            if entry in decisions:
                raise ValueError(f"Repeated duplicate decision for {entry}")
            decisions[entry] = reason
    if groups.keys() != decisions.keys():
        raise ValueError(f"Review duplicate entries: new {sorted(groups.keys()-decisions.keys())}, "
                         f"stale {sorted(decisions.keys()-groups.keys())}")
    return decisions


def summary(rows, functions, meta, audit_result, review, decisions):
    kinds = defaultdict(list)
    for row in rows:
        kinds[row.kind].append(row)
    definitions = {r.address for r in kinds["definition"]}
    cited = {r.address for r in rows if r.address and r.file.endswith(".rs")}
    entries = cited & functions.entries.keys()
    totals = Counter(r.assessment for r in kinds["definition"])
    lines = [
        "# Oblivion port ledger", "",
        f"The read-only IDA export from `{meta['database']}` on {meta['export_date_utc']} UTC contains "
        f"{len(functions.entries):,} function entries and {functions.bytes(functions.entries):,} distinct function bytes.",
        "", "## Source coverage", "",
        f"Production Rust names {len(cited):,} distinct image addresses. {len(entries):,} are exported function entries. "
        f"{len(definitions):,} entries have a matched Rust definition. Their full functions occupy "
        f"{functions.bytes(definitions):,} distinct bytes, including unported arms. The byte totals count shared tails once.",
        "", "A definition requires a leading entry address and agreement with the Rust function name. "
        "A class-qualified IDA label can also identify a Rust function with the same action name. "
        "Ambiguous matches, other addresses in an annotation, and references within a function earn no definition credit. "
        "An enclosing entry is recorded only for a matched Rust definition. The last CSV column lists every IDA chunk owner of the site address. "
        "Resolved calls are explicit module-qualified calls to matched free functions in the layering audit. "
        "Calls through methods, local bindings and dynamic dispatch require separate evidence.",
        "", "| Source assessment | Definition sites |", "| --- | ---: |",
    ]
    lines.extend(f"| {key} | {count:,} |" for key, count in sorted(totals.items()))
    lines += ["", "The assessment describes the annotated scope. A bare 100% leaves scope unspecified. "
              "The external Startup-Roadmap.md records strict closure of 16/16 rows for phases 00–02 and 32/32 for phase 03. Phases 17–22 run on the default path and remain unsigned. "
              "The Phase 22 capture compares rows at `0040F275` after one idle frame. Other runtime comparisons are recorded by capture and fields in the "
              "[open items](../open-items.md) and [Target A plan](../target-a-plan-2026-11-07.md). "
              "The ledger awards no validation points.", "",
              "| Kind | Sites | Distinct addresses |", "| --- | ---: | ---: |"]
    lines.extend(f"| {kind} | {len(sites):,} | {len({r.address for r in sites if r.address}):,} |"
                 for kind, sites in sorted(kinds.items()) if sites)
    lines += ["", "## Largest matched functions containing a stop or counted call", "",
              "Ranked by original function size. Counted calls use their enclosing matched Rust definition. Explicit stops in helpers can also identify a unique enclosing IDA function by instruction address. "
              "A counter argument can identify a callee. Unclassified errors and historical audit dispositions are excluded.",
              "", "| Entry | IDA function | Bytes | Explicit stops | Counted sites |", "| --- | --- | ---: | ---: | ---: |"]
    stops, counts = defaultdict(set), defaultdict(set)
    for row in rows:
        if row.kind == "error stop":
            # A host helper can carry a precise instruction address in the caller. Use its
            # unique IDA chunk owner when the stop address is internal rather than an entry.
            owner = row.enclosing_entry
            if not owner and row.address not in functions.entries:
                inside = functions.owners(row.address)
                if len(inside) == 1:
                    owner = inside[0]
            if owner:
                stops[owner].add((row.file, row.line))
        elif row.kind == "counted call" and row.enclosing_entry:
            counts[row.enclosing_entry].add((row.file, row.line))
    for ea in sorted(stops.keys() | counts.keys(), key=lambda ea: (-functions.entries[ea][1], ea))[:25]:
        name, size = functions.entries[ea]
        lines.append(f"| {ea} | `{name}` | {size:,} | {len(stops[ea])} | {len(counts[ea])} |")
    lines += ["", "## Boundary audit comparison", "",
              f"The independent scanner finds {sum(map(len, audit_result.stops.values())):,} diagnostic references, "
              f"{sum(map(len, audit_result.panics.values())):,} panic references and {len(audit_result.all_counters):,} counter references. "
              f"The CSV carries {len(review['items']):,} hand-reviewed address dispositions. The typed error rows retain "
              "unclassified and dynamically supplied messages. A legacy counter reference can start at an earlier match arm "
              "and include a following call. The CSV preserves those audit locations separately from the balanced call expressions.",
              "", "| Reviewed counter | Detected field increments | Recorded disposition |", "| --- | ---: | --- |"]
    for name, item in sorted(review["named_counters"].items()):
        sites = audit_result.named_counters.get(name, ([], "", []))[2]
        lines.append(f"| `{name}` | {len(sites)} | {item['verdict'].replace('|', '/')} |")
    possible = Counter(r.address for r in rows if r.kind in {"definition", "ambiguous definition"})
    ambiguous = {r.address for r in kinds["ambiguous definition"] if possible[r.address] > 1}
    lines += ["", "## Reviewed duplicate definitions", "",
              f"{len(ambiguous):,} repeated leading entries also have unresolved identity claims. "
              "The CSV retains each claim for review. Only matched definitions enter the following duplicate list.", ""]
    lines.extend(f"- `{ea}` {reason}" for ea, reason in sorted(decisions.items()))
    if not decisions:
        lines.append("No verified entry has multiple definition sites.")
    lines += ["", "Regenerate with `python3 tools/port_ledger/generate.py`. Use `--check` to verify both reports against "
              "the pinned export and current source. The [site CSV](ledger.csv) includes the source path and line for each row.", ""]
    return "\n".join(lines)


def sync_reports(directory, reports, check):
    if not check:
        directory.mkdir(parents=True, exist_ok=True)
    for filename, content in reports.items():
        path = directory / filename
        raw = content.encode("utf-8")
        if check:
            if not path.exists() or path.read_bytes() != raw:
                raise ValueError(f"Out of date: {path}")
        elif not path.exists() or path.read_bytes() != raw:
            path.write_bytes(raw)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    functions, metadata = load_export()
    sources = sorted(audit.rust_sources(), key=lambda source: source.relative)
    rows, owners = [], {}
    for source in sources:
        if source.relative.startswith("crates/oblivion_testkit/"):
            owners[source.relative] = {}
            continue
        sites, owners[source.relative] = scan(source, functions)
        rows.extend(sites)
    add_calls(rows, sources, owners)
    boundaries, review = attach_audit(rows, sources, owners)
    for row in rows:
        row.address_in_ida_functions = " ".join(functions.owners(row.address))
    groups = duplicates(rows)
    decisions = reviewed_duplicates(groups, HERE / "multiple_definitions_allow.txt")
    stream = io.StringIO(newline="")
    columns = [f.name for f in fields(Row)]
    writer = csv.DictWriter(stream, fieldnames=columns, lineterminator="\n")
    writer.writeheader()
    for row in sorted(rows, key=lambda r: (r.address, r.file, r.line, r.kind, r.name, r.evidence)):
        writer.writerow(vars(row))
    reports = {"ledger.csv": stream.getvalue(), "summary.md": summary(rows, functions, metadata, boundaries, review, decisions)}
    try:
        sync_reports(OUTPUT, reports, args.check)
    except ValueError as error:
        sys.exit(str(error))
    print(f"{len(rows):,} site records across {len(functions.entries):,} IDA functions")


if __name__ == "__main__":
    main()
