#!/usr/bin/env python3
"""One port per original function, filed with its owning class.

`where` answers "is this already ported, and where does it go?" before a function is written. `check` is the gate:
it fails when an original function gains a second port in another file, when two production functions have the same
body, when a function headed `Class_Action` is outside its class's homes (`tools/placement_homes.json`) and isn't a
reviewed exception there, and when a class has no home rule. `report` prints the current picture and `prune` drops
entries that no longer exist. A new entry is added by hand, with its reason, after review (AGENTS.md, "One port per
original function").
"""
import argparse
import bisect
import csv
import hashlib
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

from rust_source import token_spans

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "tools/placement_baseline.json"
HOMES = ROOT / "tools/placement_homes.json"
EXPORT = ROOT / "tools/port_ledger"
# The heading annotation: `IDA: 0040EE7A WinMain_PlacePlayer(...)`, or an arm's `IDA: 00690B6B..00690D9C, ...`.
HEADING = re.compile(r"IDA:\s*`?([0-9A-Fa-f]{8})\b`?(?:\.\.[0-9A-Fa-f]{8})?[\s,:]*`?([A-Za-z_?][\w:?@$]*)?")
UNNAMED = re.compile(r"^(sub_|nullsub_|j_|loc_|unknown_|\?|$)")
NAMED = re.compile(r"^(?:[A-Za-z]\w*(?:_|::)\w+|[A-Z]\w{5,})$")  # not the prose word after an address
CLASS_NAME = re.compile(r"^([A-Za-z][A-Za-z0-9]*)(?:_|::)[A-Za-z0-9_:]+$")
BODY_TOKENS = 40  # the smallest body the identical-body check compares


class Export:
    """The IDA function export the port ledger keeps: entries, sizes and split chunks."""

    def __init__(self, directory=EXPORT):
        self.names, self.sizes, self.chunks = {}, {}, []
        functions = directory / "ida_functions.csv"
        if functions.exists():
            with functions.open(newline="") as stream:
                for row in csv.DictReader(stream):
                    entry = int(row["entry"], 16)
                    self.names[entry] = row["name"]
                    self.sizes[entry] = max(int(row["size"]), 1)
        chunks = directory / "ida_split_chunks.csv"
        if chunks.exists():
            with chunks.open(newline="") as stream:
                self.chunks = sorted((int(r["start"], 16), int(r["end_exclusive"], 16), int(r["entry"], 16))
                                     for r in csv.DictReader(stream))
        self.entries = sorted(self.names)
        self.chunk_starts = [c[0] for c in self.chunks]

    def owner(self, address):
        """The entry of the function holding `address`, or the address itself when the export doesn't know it."""
        i = bisect.bisect_right(self.chunk_starts, address) - 1
        if i >= 0 and self.chunks[i][0] <= address < self.chunks[i][1]:
            return self.chunks[i][2]
        i = bisect.bisect_right(self.entries, address) - 1
        if i >= 0 and address < self.entries[i] + self.sizes[self.entries[i]]:
            return self.entries[i]
        return address


def owner_class(export_name, heading_name):
    """`Class` of a `Class_Action` name. The export wins; the heading's own name serves while the export says sub_."""
    for name in (export_name, heading_name):
        if name and not UNNAMED.match(name):
            match = CLASS_NAME.match(name)
            return match[1] if match else None
    return None


def scan(path, source, export):
    """Each production function of one file: its name, heading annotation and body."""
    lines = source.split("\n")
    starts = [0]
    for text in lines:
        starts.append(starts[-1] + len(text) + 1)
    words = list(token_spans(source))
    found, stack, depth, i = [], [], 0, 0
    test_attribute = False  # a `#[cfg(test)]` or `#[test]` waiting for its item
    pending = None

    def heading(line):
        block, at = [], line - 2
        while at >= 0:
            text = lines[at].strip()
            if text.endswith("]") and not text.startswith(("//", "#")):
                # The last line of an attribute rustfmt broke over several lines: take it back to its `#[`.
                top = at
                while top > 0 and at - top < 16 and not lines[top].strip().startswith("#["):
                    top -= 1
                    if not lines[top].strip() or lines[top].strip().endswith((";", "}")):
                        break  # an item ended in between: this `]` closes something else
                if not lines[top].strip().startswith("#["):
                    break
                block.extend(lines[n].strip() for n in range(at, top - 1, -1))
                at = top - 1
                continue
            if not text.startswith(("//", "#[", "#![")):
                break
            block.append(text)
            at -= 1
        return "\n".join(reversed(block))

    while i < len(words):
        word, start, _ = words[i]
        ahead = [w[0] for w in words[i:i + 7]]
        if ahead[:2] == ["#", "["] and (ahead[2:6] == ["cfg", "(", "test", ")"] or ahead[2:4] == ["test", "]"]):
            test_attribute = True
        if test_attribute and (word in ("use", "struct", "enum", "type", "static", "union", "macro_rules")
                               or (word == "const" and i + 1 < len(words) and words[i + 1][0] != "fn")):
            test_attribute = False  # the attribute was that item's, not the next function's
        if word in ("impl", "mod", "trait") and pending is None:
            j = i + 1
            while j < len(words) and words[j][0] not in ("{", ";"):
                j += 1
            if j < len(words) and words[j][0] == "{":
                label = " ".join(w[0] for w in words[i + 1:j])
                if word == "impl":
                    label = re.sub(r"<.*", "", label.split(" for ")[-1]).split(" where ")[0].replace(" ", "")
                pending = (word, label, test_attribute)
                test_attribute = False
                i = j
                continue
            test_attribute = False
        if word == "fn" and i + 1 < len(words) and re.fullmatch(r"[A-Za-z_]\w*", words[i + 1][0]):
            name, j, nest = words[i + 1][0], i + 2, 0
            while j < len(words):
                token = words[j][0]
                nest += token in "([" and len(token) == 1
                nest -= token in ")]" and len(token) == 1
                if not nest and token in ("{", ";"):
                    break
                j += 1
            is_test, test_attribute = test_attribute, False
            if j < len(words) and words[j][0] == "{":
                close, level = j, 0
                while close < len(words):
                    level += words[close][0] == "{"
                    level -= words[close][0] == "}"
                    if not level:
                        break
                    close += 1
                line = bisect.bisect_right(starts, start)
                comment = heading(line)
                body = [w[0] for w in words[j:close + 1]]
                nested = any(kind == "fn" for kind, _, _, _ in stack)
                in_test = is_test or "#[test]" in comment or "cfg(test)" in comment or any(t for _, _, _, t in stack)
                if not in_test and not nested:
                    address = heading_name = None
                    for text in comment.split("\n"):
                        match = HEADING.search(text)
                        if match:
                            address = int(match[1], 16)
                            heading_name = match[2] if match[2] and NAMED.match(match[2]) else None
                            break
                    entry = export.owner(address) if address is not None else None
                    export_name = export.names.get(entry, "") if entry is not None else ""
                    owner = next((label for kind, label, _, _ in reversed(stack) if kind in ("impl", "trait")), "")
                    found.append({
                        "file": path, "line": line, "name": f"{owner}::{name}" if owner else name,
                        "entry": entry, "ida": export_name if not UNNAMED.match(export_name) else (heading_name or ""),
                        "class": owner_class(export_name, heading_name) if entry is not None else None,
                        "tokens": len(body), "hash": hashlib.sha1(" ".join(body).encode()).hexdigest(),
                    })
                stack.append(("fn", name, depth, in_test))
                depth += 1
                i = j + 1
                continue
            i = j
            continue
        if word == "{":
            if pending:
                stack.append((pending[0], pending[1], depth, pending[2]))
                pending = None
            depth += 1
        elif word == "}":
            depth -= 1
            if stack and stack[-1][2] == depth:
                stack.pop()
        i += 1
    return found


def snake(name):
    """`HUDMainMenu` -> `hud_main_menu`, as the menus' files are named."""
    return re.sub(r"(?<=[a-z0-9])(?=[A-Z])|(?<=[A-Z])(?=[A-Z][a-z])", "_", name).lower()


class Homes:
    """`tools/placement_homes.json`: the files and folders each owner class may be filed in, and the exceptions."""

    def __init__(self, path=HOMES):
        data = json.loads(path.read_text()) if path.exists() else {"rules": [], "exceptions": {}}
        self.rules = [(re.compile(rule["classes"]), rule["homes"], rule.get("why", "")) for rule in data["rules"]]
        self.exceptions = data.get("exceptions", {})
        self.known = {}

    def of(self, name):
        """The homes of a class with `{snake}` filled in, or None when no rule names it."""
        if name not in self.known:
            self.known[name] = next(([home.replace("{snake}", snake(name)) for home in homes]
                                     for pattern, homes, _ in self.rules if pattern.search(name)), None)
        return self.known[name]

    def holds(self, file, name):
        homes = self.of(name)
        return homes is not None and any(home == "*" or file == home or (home.endswith("/") and file.startswith(home))
                                         for home in homes)


def misfiled(functions, homes):
    """(functions outside their class's homes, classes no rule names)."""
    outside, unnamed = [], set()
    for f in functions:
        if not f["class"]:
            continue
        if homes.of(f["class"]) is None:
            unnamed.add(f["class"])
        elif not homes.holds(f["file"], f["class"]):
            outside.append(f)
    return outside, unnamed


def production_files(root):
    for base in ("crates", "src"):
        for path in sorted((root / base).rglob("*.rs")):
            parts = path.relative_to(root).parts
            if "target" in parts or "tests" in parts or "examples" in parts or "benches" in parts:
                continue
            if parts[:2] == ("crates", "oblivion_testkit"):
                continue  # the test kit's captured scaffold replays ports; it doesn't own any
            yield path


def survey(root=ROOT, export=None):
    export = export or Export(root / "tools/port_ledger")
    functions = []
    for path in production_files(root):
        functions += scan(str(path.relative_to(root)), path.read_text(), export)
    # An arm headed only by its address range takes the name its function has on another definition.
    named = {f["entry"]: f for f in functions if f["entry"] is not None and f["ida"]}
    for f in functions:
        if f["entry"] in named and not f["ida"]:
            f["ida"], f["class"] = named[f["entry"]]["ida"], named[f["entry"]]["class"]
    return functions


def picture(functions):
    """The three things the baseline records: owner classes per file, cross-file ports, identical bodies."""
    classes, ports, bodies = defaultdict(set), defaultdict(list), defaultdict(list)
    for f in functions:
        if f["class"]:
            classes[f["file"]].add(f["class"])
        if f["entry"] is not None:
            ports[f"{f['entry']:08X}"].append(f)
        if f["tokens"] >= BODY_TOKENS:
            bodies[f["hash"]].append(f)
    ports = {entry: members for entry, members in ports.items() if len({m["file"] for m in members}) > 1}
    bodies = [members for members in bodies.values() if len(members) > 1]
    return classes, ports, bodies


def identity(f):
    return f"{f['file']}::{f['name']}"


def site(f):
    return f"{f['file']}:{f['line']}: {f['name']}"


def load_baseline(path=BASELINE):
    if not path.exists():
        return {"ports": {}, "bodies": []}
    return json.loads(path.read_text())


def check(functions, baseline, homes=None):
    homes = homes or Homes()
    classes, ports, bodies = picture(functions)
    problems, stale = [], []
    outside, unnamed = misfiled(functions, homes)
    for name in sorted(unnamed):
        first = min((f for f in functions if f["class"] == name), key=lambda f: (f["file"], f["line"]))
        problems.append(f"{site(first)}: owner class {name} ({first['ida']}) has no home. Add a rule for it to "
                        "tools/placement_homes.json and file the function there.")
    excepted = set()
    for f in sorted(outside, key=lambda f: (f["file"], f["line"])):
        if identity(f) in homes.exceptions:
            excepted.add(identity(f))
            continue
        problems.append(f"{site(f)}: {f['ida']} belongs to {f['class']}, whose home is "
                        f"{', '.join(homes.of(f['class']))}. File it there and call it, or record the reviewed "
                        "exception in tools/placement_homes.json.")
    for name in sorted(set(homes.exceptions) - excepted):
        stale.append(f"home exception {name} is gone or at home (remove it from tools/placement_homes.json)")
    for entry in sorted(ports):
        known = set(baseline["ports"].get(entry, {}).get("members", ()))
        members = ports[entry]
        for f in members:
            if identity(f) not in known:
                others = "; ".join(site(m) for m in members if m is not f)
                problems.append(f"{site(f)}: {entry} {f['ida']} is already ported at {others}. Call that port; "
                                "a second definition needs a reviewed baseline entry with its reason.")
    for entry, record in sorted(baseline["ports"].items()):
        current = {identity(f) for f in ports.get(entry, ())}
        for name in sorted(set(record["members"]) - current):
            stale.append(f"{entry}: {name} is gone")
    known_bodies = {frozenset(group) for group in baseline["bodies"]}
    for members in sorted(bodies, key=lambda m: site(m[0])):
        group = frozenset(identity(f) for f in members)
        if not any(group <= known for known in known_bodies):
            problems.append("identical bodies: " + "; ".join(site(f) for f in members)
                            + ". Keep one and call it, or record the reviewed exception.")
    current_bodies = {frozenset(identity(f) for f in members) for members in bodies}
    for group in sorted(known_bodies - current_bodies, key=sorted):
        if not any(group <= current for current in current_bodies):
            stale.append("identical bodies no longer present: " + "; ".join(sorted(group)))
    return problems, stale, (len(classes), len(ports), len(bodies))


def snapshot(functions, baseline):
    """The current picture as a baseline, keeping each reviewed reason."""
    classes, ports, bodies = picture(functions)
    return {
        "ports": {entry: {"ida": members[0]["ida"],
                          "members": sorted(identity(f) for f in members),
                          "reason": baseline["ports"].get(entry, {}).get("reason", "")}
                  for entry, members in sorted(ports.items())},
        "bodies": sorted(sorted(identity(f) for f in members) for members in bodies),
    }


def prune(functions, baseline):
    """Drop what no longer exists; never add."""
    current = snapshot(functions, baseline)
    ports = {}
    for entry, record in baseline["ports"].items():
        members = sorted(set(record["members"]) & set(current["ports"].get(entry, {}).get("members", ())))
        if len({m.split("::")[0] for m in members}) > 1:
            ports[entry] = {**record, "members": members}
    bodies = [group for group in baseline["bodies"] if any(set(group) <= set(now) for now in current["bodies"])]
    return {"ports": dict(sorted(ports.items())), "bodies": sorted(bodies)}


def where(functions, export, query):
    """Every production function whose heading names this address or this IDA name."""
    hits = []
    if re.fullmatch(r"(0x)?[0-9A-Fa-f]{6,8}", query):
        entry = export.owner(int(query, 16))
        hits = [f for f in functions if f["entry"] == entry]
        label = f"{entry:08X} {export.names.get(entry, '(not in the export)')}"
    else:
        needle = query.lower()
        entries = {e for e, name in export.names.items() if needle in name.lower()}
        hits = [f for f in functions if f["entry"] in entries or needle in f["ida"].lower()
                or needle in f["name"].lower()]
        label = f"names containing {query!r}"
    return label, sorted(hits, key=lambda f: (f["file"], f["line"]))


def home_line(homes, export, query):
    """Where a function of this address or `Class_Action` name is filed, for `where`."""
    name = query
    if re.fullmatch(r"(0x)?[0-9A-Fa-f]{6,8}", query):
        name = export.names.get(export.owner(int(query, 16)), "")
    owner = owner_class(name, None) or (query if homes.of(query) else None)
    if not owner:
        return None
    found = homes.of(owner)
    return f"{owner}'s home: {', '.join(found)}" if found else f"{owner} has no home rule yet: add one to {HOMES.name}"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("check", help="compare the sources to the reviewed baseline")
    report = commands.add_parser("report", help="print owner classes per file and every cross-file port")
    report.add_argument("--baseline", action="store_true", help="print the current picture as baseline JSON")
    report.add_argument("--file", help="list one file's functions with what each ports (a path suffix)")
    report.add_argument("--owed", action="store_true", help="list the entries still marked OWED")
    report.add_argument("--exceptions", action="store_true", help="list the reviewed home exceptions with reasons")
    commands.add_parser("prune", help="drop baseline entries that no longer exist")
    lookup = commands.add_parser("where", help="find the existing port of an address or an IDA/Rust name")
    lookup.add_argument("query")
    args = parser.parse_args(argv)
    export = Export()
    functions = survey(ROOT, export)
    baseline = load_baseline()
    homes = Homes()

    if args.command == "where":
        label, hits = where(functions, export, args.query)
        print(label)
        line = home_line(homes, export, args.query)
        if line:
            print(f"  {line}")
        for f in hits:
            print(f"  {site(f)}  [{f['entry']:08X} {f['ida']}]" if f["entry"] is not None else f"  {site(f)}  [IDA: N/A]")
        if not hits:
            print("  no production function is headed by it; grep the IDA comments before porting it:")
            print(f"  grep -rn '{args.query}' crates src docs/port-ledger/ledger.csv")
        return 0
    if args.command == "report":
        if args.baseline:
            print(json.dumps(snapshot(functions, baseline), indent=1))
            return 0
        if args.owed:
            owed = {e: r for e, r in baseline["ports"].items() if r.get("reason", "").startswith("OWED")}
            for entry, record in sorted(owed.items()):
                print(f"{entry} {record['ida']}: {record['reason']}")
                for member in record["members"]:
                    print(f"  {member}")
            print(f"{len(owed)} of {len(baseline['ports'])} recorded cross-file ports are owed a fix")
            misfiles = {n: r for n, r in homes.exceptions.items() if r.startswith("OWED")}
            for name, reason in sorted(misfiles.items()):
                print(f"{name}: {reason}")
            print(f"{len(misfiles)} of {len(homes.exceptions)} functions recorded outside their class's home are owed a move")
            return 0
        if args.exceptions:
            for name, reason in sorted(homes.exceptions.items()):
                print(f"{name}: {reason}")
            print(f"{len(homes.exceptions)} functions are recorded outside their class's home")
            return 0
        if args.file:
            for f in sorted((f for f in functions if f["file"].endswith(args.file)), key=lambda f: (f["file"], f["line"])):
                origin = f"{f['entry']:08X} {f['ida']}" if f["entry"] is not None else "IDA: N/A"
                print(f"{site(f)}  [{origin}]")
            return 0
        classes, ports, bodies = picture(functions)
        for file in sorted(classes):
            print(f"{file}: {', '.join(sorted(classes[file]))}")
        for entry in sorted(ports):
            print(f"{entry} {ports[entry][0]['ida']}")
            for f in ports[entry]:
                print(f"  {site(f)}")
        for members in bodies:
            print("identical: " + "; ".join(site(f) for f in members))
        print(f"{len(classes)} files with owner classes, {len(ports)} cross-file ports, {len(bodies)} identical bodies")
        return 0
    if args.command == "prune":
        BASELINE.write_text(json.dumps(prune(functions, baseline), indent=1) + "\n")
        return 0
    problems, stale, (files, ports, bodies) = check(functions, baseline, homes)
    for line in problems:
        print(line)
    for line in stale:
        print(f"stale entry: {line}" + ("" if line.startswith("home exception") else " (run `python3 tools/placement.py prune`)"))
    print(f"{files} files with owner classes, {ports} cross-file ports, {bodies} identical bodies; "
          f"{len(problems)} new, {len(stale)} stale")
    return int(bool(problems))


if __name__ == "__main__":
    sys.exit(main())
