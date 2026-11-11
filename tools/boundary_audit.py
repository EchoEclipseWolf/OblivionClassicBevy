#!/usr/bin/env python3
"""G4 boundary audit: the roadmap's carried lists and the C++ stops and counters against the Rust.

    python3 tools/boundary_audit.py [--check] [--run-tests]

Reads, never writes: the notes' Startup-Roadmap.md and the C++ repository (`src/`, `include/`).
Writes `docs/boundary-audit.md`. `--check` alone writes nothing and exits 1 if the table would change, or
if anything is unreviewed `missing` or a counted boundary differs without a recorded reason. `--run-tests` runs
`phase15_initial_area` and rewrites `tools/boundary_audit_counts.json` first, even with `--check`; run it inside a
memory-capped scope (`AGENTS.md`, Build and test).

Each address gets one class from what the Rust code does, not from what its comments say:

  counted       `.count(0x...)`, or `field += 1` on a field whose `IDA: N/A` doc names the address
  stop          a string literal naming the address in an `Err` position (or a panic, marked)
  ported        an `IDA:` comment leading the address, directly above a `fn` (or inside its range),
                or a comment at the start of a line of body code naming the site
  substitution  an `IDA: N/A` label or a comment recording a host substitution, with no body
  missing       none of these. A comment alone lands here, marked `comment only`.

The hand decisions live in `tools/boundary_audit_review.json`, as groups of addresses with a class,
a decision and the automatic class they were made against; if that changes, the row says `STALE` and
has to be looked at again. Scaffold data and generated tables are not evidence.
"""
import argparse
import bisect
import json
import os
import re
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ECHO = Path(os.environ.get("OBLIVION_ECHO", "/run/media/natfoth/Echo"))
ROADMAP = ECHO / "OblivionBotInfos/Recomp/StartupLoading/Startup-Roadmap.md"
CPP = ECHO / "OblivionRecomp"
REVIEW = REPO / "tools/boundary_audit_review.json"
OUTPUT = REPO / "docs/boundary-audit.md"

# Generated metadata: an address here says nothing about what the port does.
NOT_EVIDENCE = re.compile(r"(scaffold_data|_table|_tables|table_data)\.rs$|_tests\.rs$|/tests/|/benches/")
ADDRESS = re.compile(r"(?<![0-9A-Za-z_+])0[0-9A-Fa-f]{7}(?![0-9A-Za-z_])")
HEX_LITERAL = re.compile(r"0x(0[0-9A-Fa-f_]{7,9})U?\b")
# The address space the carried lists use; other 8-digit tokens are values (form IDs, hashes).
CODE_LOW, IMAGE_HIGH = 0x00401000, 0x00C00000


def in_image(text):
    return CODE_LOW <= int(text, 16) < IMAGE_HIGH


def lex(text):
    """Comments and strings out of Rust or C++. Returns code (same length, comments and string
    bodies blanked), comments [(start, text)] and strings [(start, text)]."""
    code = list(text)
    comments, strings = [], []
    index, size = 0, len(text)
    while index < size:
        char = text[index]
        pair = text[index : index + 2]
        if pair == "//":
            end = text.find("\n", index)
            end = size if end < 0 else end
            comments.append((index, text[index:end]))
            code[index:end] = " " * (end - index)
            index = end
        elif pair == "/*":
            end = text.find("*/", index + 2)
            end = size if end < 0 else end + 2
            comments.append((index, text[index:end]))
            code[index:end] = [c if c == "\n" else " " for c in text[index:end]]
            index = end
        elif char == '"' or (
            char == "r"
            and re.match(r'r#*"', text[index : index + 6])
            and not (index and (text[index - 1].isalnum() or text[index - 1] == "_"))
        ):
            if char == '"':
                end = index + 1
                while end < size and text[end] != '"':
                    end += 2 if text[end] == "\\" else 1
                body_start, close = index + 1, end + 1
            else:
                hashes = re.match(r'r(#*)"', text[index:]).group(1)
                body_start = index + 2 + len(hashes)
                end = text.find('"' + hashes, body_start)
                end = size if end < 0 else end
                close = end + 1 + len(hashes)
            strings.append((index, text[body_start:end]))
            code[body_start:end] = [c if c == "\n" else " " for c in text[body_start:end]]
            index = close
        elif char == "'" and re.match(r"'(\\.[^']*|[^\\'])'", text[index : index + 12]):
            index += re.match(r"'(\\.[^']*|[^\\'])'", text[index : index + 12]).end()
        else:
            index += 1
    return "".join(code), comments, strings


def expand_shorthand(token):
    """`0077C450/3D0/4D0` and `0058F5D3..` style lists out of one backticked token."""
    found = []
    last = None
    for part in re.split(r"\s*/\s*", token):
        part = part.strip()
        if re.fullmatch(r"[0-9A-Fa-f]{8}", part):
            last = part.upper()
            found.append(last)
        elif re.fullmatch(r"[0-9A-Fa-f]{6}", part) and any(c.isdigit() for c in part):
            found.append(("00" + part).upper())
            last = found[-1]
        elif last and re.fullmatch(r"[0-9A-Fa-f]{2,5}", part) and re.search(r"\d", part):
            found.append((last[: 8 - len(part)] + part).upper())
    return found


def carried_lists(text):
    """Every "Carried from Phase NN" paragraph and "Later-phase dependencies carried" list, with its
    bullets. A section's address list stays in the order the notes give it."""
    sections = []
    lines = text.splitlines()
    index = 0
    while index < len(lines):
        line = lines[index]
        heading = re.match(r"(Carried from Phases? ([0-9–-]+)|Later-phase dependencies carried from Phases? ([0-9–-]+))", line)
        if not heading:
            index += 1
            continue
        phase = heading.group(2) or heading.group(3)
        start = index
        body = []
        while index < len(lines):
            current = lines[index]
            if not current.strip():
                # A list resumes after its blank line only with a bullet.
                if index + 1 < len(lines) and lines[index + 1].startswith("- "):
                    index += 1
                    continue
                break
            if index > start and current.startswith("**"):
                break
            body.append(current)
            index += 1
        sections.append((f"P{phase}", start + 1, "\n".join(body)))
    return sections


def addresses_in_notes(body):
    found = []
    for token in re.findall(r"`([^`]+)`", body):
        head = token.split()[0] if token.split() else ""
        for address in expand_shorthand(head.strip(",;:")):
            found.append(address)
        for later in token.split()[1:]:
            if re.fullmatch(r"[0-9A-Fa-f]{8}", later.strip(",;:.")):
                found.append(later.strip(",;:.").upper())
        range_match = re.fullmatch(r"([0-9A-Fa-f]{8})\.\.([0-9A-Fa-f]{8})", token.strip())
        if range_match:
            found.extend(a.upper() for a in range_match.groups())
    return [a for a in found if in_image(a)]


class Source:
    def __init__(self, path, relative):
        self.relative = relative
        self.text = path.read_text(encoding="utf-8", errors="replace")
        self.code, self.comments, self.strings = lex(self.text)
        self.line_starts = [0] + [m.end() for m in re.finditer("\n", self.text)]

    def line(self, offset):
        return bisect.bisect_right(self.line_starts, offset)

    def where(self, offset):
        return f"{self.relative}:{self.line(offset)}"

    def statement(self, offset, limit=400):
        """The code from the previous statement end to this offset."""
        start = max(0, offset - limit)
        window = self.code[start:offset]
        cut = max(window.rfind(";"), window.rfind("{"), window.rfind("}"))
        return window[cut + 1 :]


def walk(root, suffixes):
    for directory, _, files in os.walk(root):
        for name in sorted(files):
            if name.endswith(suffixes):
                yield Path(directory) / name


def rust_sources():
    sources = []
    for base in ("crates", "src"):
        for path in walk(REPO / base, (".rs",)):
            relative = str(path.relative_to(REPO))
            if NOT_EVIDENCE.search("/" + relative):
                continue
            source = Source(path, relative)
            # Test modules are not the port.
            for match in re.finditer(r"#\[cfg\(test\)\]\s*(pub\s+)?mod\s+\w+\s*\{", source.code):
                depth, position = 1, match.end()
                while position < len(source.code) and depth:
                    depth += {"{": 1, "}": -1}.get(source.code[position], 0)
                    position += 1
                source.comments = [c for c in source.comments if not match.start() <= c[0] < position]
                source.strings = [s for s in source.strings if not match.start() <= s[0] < position]
                source.code = (
                    source.code[: match.start()]
                    + re.sub(r"[^\n]", " ", source.code[match.start() : position])
                    + source.code[position:]
                )
            sources.append(source)
    return sources


def cpp_sources():
    sources = []
    for base in ("src", "include"):
        for path in walk(CPP / base, (".cpp", ".hpp", ".inc", ".h")):
            sources.append(Source(path, str(path.relative_to(CPP))))
    return sources


LEAD_ADDRESS = re.compile(r"^\W*(?:0x)?([0-9A-Fa-f]{8})(?:\.\.([0-9A-Fa-f]{8}))?")


def norm_hex(token):
    return token.replace("_", "").upper()


class RustEvidence:
    def __init__(self, sources):
        self.counted = defaultdict(list)  # address -> [(where, how)]
        self.stops = defaultdict(list)
        self.panics = defaultdict(list)
        self.bodies = defaultdict(list)
        self.ranges = []  # (low, high, where)
        self.inline = defaultdict(list)
        self.substitutions = defaultdict(list)
        self.comments = defaultdict(list)
        self.code_refs = defaultdict(list)
        self.global_fields = defaultdict(list)
        self.all_stop_literals = []
        self.all_panic_literals = []
        self.all_counters = []
        for source in sources:
            self.scan(source)

    def scan(self, source):
        self.scan_counts(source)
        self.scan_strings(source)
        self.scan_comments(source)
        self.scan_code_refs(source)
        self.scan_named_counters(source)

    def scan_counts(self, source):
        for match in re.finditer(r"\b(?:count|boundaries|counted_boundaries\.push)\s*\(([^;]*?)\)\s*;", source.code):
            for literal in HEX_LITERAL.findall(match.group(1)):
                address = norm_hex(literal)
                if len(address) == 8 and in_image(address):
                    self.counted[address].append((source.where(match.start()), "count"))
                    self.all_counters.append((address, source.where(match.start()), "count"))

    def scan_named_counters(self, source):
        # `/// IDA: N/A ... 00517810 ...` above `pub name: u32`, with `name += 1` anywhere in the port.
        fields = {}
        for start, comment in source.comments:
            if "IDA: N/A" not in comment:
                continue
            end = source.text.find("\n", start)
            rest = source.code[end : end + 200]
            field = re.match(r"\s*(?:#\[[^\n]*\]\s*)*(?:pub(?:\([a-z]+\))?\s+)?(\w+)\s*:\s*(?:u32|u64|usize|i32)\s*,", rest)
            if field:
                addresses = [a.upper() for a in ADDRESS.findall(comment) if in_image(a)]
                fields[field.group(1)] = (addresses, comment.strip(), source.where(start))
        self.named = getattr(self, "named", {})
        self.named.update(fields)

    def finish_named_counters(self, sources):
        increments = defaultdict(list)
        for source in sources:
            for match in re.finditer(r"\.?\b(\w+)\s*\+=\s*1\b", source.code):
                increments[match.group(1)].append(source.where(match.start()))
        self.named_counters = {}
        for name, (addresses, comment, where) in getattr(self, "named", {}).items():
            if name in increments:
                self.named_counters[name] = (addresses, comment, increments[name])
                for address in addresses:
                    self.counted[address].append((increments[name][0], f"field {name}"))
                    self.all_counters.append((address, increments[name][0], f"field {name}"))

    def scan_strings(self, source):
        for start, body in source.strings:
            found = [a.upper() for a in ADDRESS.findall(body) if in_image(a)]
            if not found:
                continue
            context = source.statement(start)
            # `#[expect(..., reason = "...")]`: a lint's reason, not code.
            if re.search(r"\breason\s*=\s*$", context):
                continue
            if re.search(r"\b(println!|eprintln!|emit\(|warn\w*\(|info!|log\w*\(|warning|startup_log|phase_line)", context):
                continue
            # In port code an assert or an expect naming a site is a loud stop, by panic.
            if re.search(r"(\.expect\(|panic!|unreachable!|unimplemented!|todo!|assert\w*!)", context):
                for address in found:
                    self.panics[address].append((source.where(start), body))
                if re.match(r"[0-9A-Fa-f]{8}", body):
                    self.all_panic_literals.append((found[0], source.where(start), body))
                continue
            # A bare address in a literal is a name in a table (shader constants), not a stop.
            if not re.search(r"[A-Za-z]{2}", ADDRESS.sub("", body)):
                continue
            leads = bool(re.match(r"[0-9A-Fa-f]{8}", body))
            if re.search(r"\b(Err|GameError|Error|ok_or|ok_or_else|map_err|no_\w+|stop\w*|fail\w*|unsupported\w*)\b", context) or leads:
                for address in found:
                    self.stops[address].append((source.where(start), body))
                if leads:
                    self.all_stop_literals.append((found[0], source.where(start), body))

    def scan_comments(self, source):
        for start, comment in source.comments:
            found = [a.upper() for a in ADDRESS.findall(comment) if in_image(a)]
            if not found:
                continue
            where = source.where(start)
            stripped = re.sub(r"^\s*(//[/!]?|/\*+)\s*", "", comment)
            label = re.match(r"(?:TODO: \[ \] )?IDA:\s*", stripped)
            line_end = source.text.find("\n", start)
            line_begin = source.text.rfind("\n", 0, start) + 1
            trailing = bool(source.code[line_begin:start].strip())
            after = source.code[line_end : line_end + 800] if line_end >= 0 else ""
            before_fn = re.match(
                r"(?:\s*(?:#\[[^\n]*\]|//[^\n]*)?\s*\n)*?\s*(?:#\[[^\n]*\]\s*)*(?:pub(?:\([a-z]+\))?\s+)?(?:const\s+)?(?:unsafe\s+)?(?:async\s+)?fn\s",
                source.text[line_end : line_end + 1200] if line_end >= 0 else "",
            )
            is_na = bool(re.match(r"IDA:\s*N/A", stripped))
            if stripped.startswith("TODO"):
                self.comments[found[0]].append((where, "todo"))
                for address in found:
                    self.comments[address].append((where, "todo"))
                continue
            if label and not is_na:
                rest = stripped[label.end() :]
                # The lead: addresses before the first word that is not a joiner.
                lead = []
                for token in re.finditer(r"(?:0x)?([0-9A-Fa-f]{8})(?:\.\.([0-9A-Fa-f]{8}))?|([^\s,/;:()]+)", rest):
                    if token.group(1):
                        lead.append((token.group(1).upper(), (token.group(2) or "").upper()))
                    elif token.group(3).lower() in ("and", "then", "or", "plus", "the", "with"):
                        continue
                    else:
                        break
                if lead and before_fn:
                    for low, high in lead:
                        self.bodies[low].append(where)
                        if high and in_image(high) and in_image(low):
                            self.ranges.append((int(low, 16), int(high, 16), where))
                    for address in found:
                        if not any(address == l for l, _ in lead):
                            self.comments[address].append((where, "named in another fn's IDA line"))
                    continue
                if lead and re.match(r"\s*(?:#\[[^\n]*\]\s*)*(?:pub(?:\([a-z]+\))?\s+)?\w+\s*:|\s*(?:pub\s+)?(?:const|static)\s", after):
                    # A field or constant carrying an address: data.
                    for low, _ in lead:
                        self.global_fields[low].append(where)
                    continue
            if is_na:
                for address in found:
                    self.substitutions[address].append((where, stripped[:140]))
                continue
            if re.search(r"(recorded )?(host )?substitut", comment, re.I):
                for address in found:
                    self.substitutions[address].append((where, stripped[:140]))
            # A comment at a line of code, or trailing one, names the site it ports, unless it says
            # the code doesn't.
            negative = re.search(
                r"\b(counted|boundary|not ported|isn't ported|host stop|stops?\b|not run|never|unported|TODO|not reached|skipp|aren't|isn't)\b",
                comment,
                re.I,
            )
            lead = LEAD_ADDRESS.match(stripped)
            next_line = source.code[line_end + 1 : source.text.find("\n", line_end + 1)] if line_end >= 0 else ""
            if lead and not negative and (trailing or next_line.strip()) and not re.match(r"\s*(pub\s|fn\s|#\[|struct\s|impl\s)", next_line):
                first = lead.group(1).upper()
                self.inline[first].append(where)
                if lead.group(2):
                    high = lead.group(2).upper()
                    self.inline[high].append(where)
                    if in_image(first) and in_image(high) and int(high, 16) - int(first, 16) < 0x4000:
                        self.ranges.append((int(first, 16), int(high, 16), where))
                for address in found:
                    if address not in (first, (lead.group(2) or "").upper()):
                        self.comments[address].append((where, "comment"))
            else:
                for address in found:
                    self.comments[address].append((where, "boundary comment" if negative else "comment"))

    def scan_code_refs(self, source):
        for match in HEX_LITERAL.finditer(source.code):
            address = norm_hex(match.group(1))
            if len(address) == 8 and in_image(address):
                self.code_refs[address].append(source.where(match.start()))
        for match in re.finditer(r"\b\w+_([0-9a-f]{6})\b", source.code):
            digits = match.group(1).upper()
            if re.search(r"\d", digits) and int(digits, 16) >= 0x00A00000:
                self.global_fields["00" + digits].append(source.where(match.start()))

    def same_stop_nearby(self, address, message):
        """A Rust stop within the same few instructions saying the same thing: the same stop, whose
        address the Rust moved. Returns (address, where, message)."""
        words = set(re.findall(r"[A-Za-z0-9+]{3,}", message.lower())) - {address.lower()}
        best = None
        for other, where, body in self.all_stop_literals + self.all_panic_literals:
            if abs(int(other, 16) - int(address, 16)) > 0x60:
                continue
            theirs = set(re.findall(r"[A-Za-z0-9+]{3,}", body.lower())) - {other.lower()}
            shared = len(words & theirs) / max(1, min(len(words), len(theirs)))
            if shared >= 0.5 and (best is None or shared > best[0]):
                best = (shared, other, where, body)
        return best[1:] if best else None

    def classify(self, address, cpp_message=None):
        """(class, evidence). A body found for an address also lists any stops in its arms."""
        value = int(address, 16)
        evidence = []
        if self.counted.get(address):
            sites = self.counted[address]
            evidence.append("; ".join(sorted({f"{w} ({h})" for w, h in sites})[:3]))
            return "counted", evidence
        if self.stops.get(address):
            where, body = self.stops[address][0]
            evidence.append(f'{where} "{body[:90]}"')
            if self.bodies.get(address):
                evidence.append(f"body {self.bodies[address][0]}")
            return "stop", evidence
        if self.panics.get(address):
            where, body = self.panics[address][0]
            evidence.append(f'panic {where} "{body[:90]}"')
            return "stop", evidence
        if cpp_message:
            moved = self.same_stop_nearby(address, cpp_message)
            if moved:
                evidence.append(f'moved to {moved[0]}: {moved[1]} "{moved[2][:90]}"')
                return "stop", evidence
        if self.bodies.get(address):
            evidence.append(f"body {self.bodies[address][0]}")
            return "ported", evidence
        if self.global_fields.get(address):
            evidence.append(f"field {self.global_fields[address][0]}")
            return "ported", evidence
        if self.inline.get(address):
            evidence.append(f"site {self.inline[address][0]}")
            return "ported", evidence
        for low, high, where in self.ranges:
            if low <= value <= high:
                evidence.append(f"in range {low:08X}..{high:08X} {where}")
                return "ported", evidence
        if self.substitutions.get(address):
            where, text = self.substitutions[address][0]
            evidence.append(f"{where} {text[:90]}")
            return "substitution", evidence
        if self.code_refs.get(address):
            evidence.append(f"constant {self.code_refs[address][0]}")
        if self.comments.get(address):
            where, how = self.comments[address][0]
            evidence.append(f"{how} only: {where}")
        return "missing", evidence


def cpp_stops_and_counts(sources):
    stops = defaultdict(list)
    counts = defaultdict(list)
    for source in sources:
        for start, body in source.strings:
            context = source.statement(start, 200)
            is_stop = re.search(r"\bthrow\b|\b(stop|set_host_stop|unreached|unreached_warning|require_hook)\s*\(", context)
            pending = re.match(r"Phase-\w+ pending: ([0-9A-Fa-f]{8})", body)
            lead = re.match(r"([0-9A-Fa-f]{8})\b", body)
            if is_stop and lead and in_image(lead.group(1)):
                stops[lead.group(1).upper()].append((source.where(start), body))
            elif pending and in_image(pending.group(1)):
                stops[pending.group(1).upper()].append((source.where(start), body))
        for match in re.finditer(r"\bcount\s*\(([^;]*?)\)\s*;", source.code):
            for literal in HEX_LITERAL.findall(match.group(1)):
                address = norm_hex(literal)
                if len(address) == 8 and in_image(address):
                    counts[address].append(source.where(match.start()))
    return stops, counts


EXE = ECHO / "OblivionBot/Oblivion.exe"
FUNCTIONS = ECHO / "OblivionBotInfos/Recomp/StartupLoading/evidence/phase00-cinit-2026-10-05/ida-functions.tsv"


def exe_calls():
    """Every direct `call` or tail `jmp` in the exe, read-only through objdump, and each function's
    extent from the IDA export the notes keep: {target: [(site, function start)]}, instruction
    starts. Empty if either file or objdump isn't there, and the section says so."""
    try:
        listing = subprocess.run(
            ["objdump", "-d", "--no-show-raw-insn", "-M", "intel", str(EXE)],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
        rows = [line.split("\t") for line in FUNCTIONS.read_text(encoding="utf-8", errors="replace").splitlines()]
    except (OSError, subprocess.CalledProcessError):
        return {}, set(), {}
    starts = sorted(int(row[0], 16) for row in rows if len(row) > 1)
    ends = {int(row[0], 16): int(row[1], 16) for row in rows if len(row) > 1}
    names = {int(row[0], 16): row[2] for row in rows if len(row) > 2}
    calls = defaultdict(list)
    instructions = set()
    for match in re.finditer(r"^ *([0-9a-f]+):\t(\S+)(?: +0x([0-9a-f]+))?", listing, re.M):
        site = int(match.group(1), 16)
        instructions.add(site)
        if match.group(3) and match.group(2) in ("call", "jmp"):
            index = bisect.bisect_right(starts, site) - 1
            start = starts[index] if index >= 0 else None
            if start is None or site >= ends[start]:
                continue
            target = int(match.group(3), 16)
            if match.group(2) == "jmp" and start <= target < ends[start]:
                continue
            calls[target].append((site, start))
    return calls, instructions, names


def rust_span(source, comment_offset):
    """The IDA comment block and the fn after it, as offsets."""
    position = source.code.find("{", comment_offset)
    if position < 0:
        return comment_offset, comment_offset
    depth, index = 0, position
    code = source.code
    while index < len(code):
        depth += {"{": 1, "}": -1}.get(code[index], 0)
        index += 1
        if depth == 0:
            break
    return comment_offset, index


def silent_passes(rust, sources, targets):
    """For each boundary, each direct exe caller inside a function the Rust ports: does that Rust
    function show the call? `none` is the case G4 exists to catch; a helper may hold it, so each
    such row is read by hand."""
    calls, instructions, names = exe_calls()
    by_file = {source.relative: source for source in sources}
    rows = []
    for target in sorted(targets):
        for site, function in calls.get(int(target, 16), []):
            bodies = rust.bodies.get(f"{function:08X}", [])
            for where in bodies[:1]:
                path, line = where.rsplit(":", 1)
                source = by_file[path]
                begin, end = rust_span(source, source.line_starts[int(line) - 1])
                literal = target[:4] + "_" + target[4:]
                code = source.code[begin:end]
                raw = source.text[begin:end]
                if re.search(rf"count\(|boundaries\(", code) and (literal in code or target in code):
                    how = "count"
                elif any(begin <= start < end and target in body for start, body in source.strings):
                    how = "stop"
                elif literal in code:
                    how = "constant"
                elif target in raw:
                    how = "comment"
                else:
                    how = "none"
                rows.append((target, f"{site:08X}", f"{function:08X}", names.get(function, ""), where, how))
    return rows


def open_items_section_four():
    text = (REPO / "docs/open-items.md").read_text(encoding="utf-8")
    section = re.search(r"^## 4\. .*?(?=^## 5\.)", text, re.S | re.M).group(0)
    return {a.upper() for a in ADDRESS.findall(section)}, {a.upper() for a in ADDRESS.findall(text)}


def load_review():
    review = {"groups": [], "named_counters": {}, "counted": {}, "paths": {}, "callers": {}}
    if REVIEW.exists():
        review.update(json.loads(REVIEW.read_text(encoding="utf-8")))
    items = {}
    for group in review["groups"]:
        for address in group["addresses"]:
            note = group.get("note", "")
            if isinstance(address, list):
                address, note = address[0], f"{note} {address[1]}".strip()
            items[address] = {"class": group["class"], "note": note, "auto": group["auto"], "decision": group["decision"]}
    review["items"] = items
    return review


C15_LINES = {"tail": "15c:", "exit": "15d exit:", "enter": "15d enter:"}
RUST_LINES = {"tail": "LAND (4, 7)", "exit": "RUN-003:", "enter": "RUN-002:"}
COUNTS_CACHE = REPO / "tools/boundary_audit_counts.json"


def cpp_path_counts():
    """The C++ log's counted boundaries per path (`startup_phase15`, on its own fixture)."""
    out = {}
    text = (REPO / "fixtures/cpp-baseline/LastTest-x86.log").read_text(encoding="utf-8", errors="replace")
    for name, prefix in C15_LINES.items():
        line = next((line for line in text.splitlines() if line.startswith(prefix)), "")
        out[name] = {a: int(n) for a, n in re.findall(r"\b([0-9A-F]{8}) x(\d+)", line)}
    return out


def run_rust_path_counts():
    """The Rust's on the same paths, from `phase15_initial_area` on the shipped data. Needs
    OBLIVION_DATA_DIR. The answer is kept in `tools/boundary_audit_counts.json`."""
    run = subprocess.run(
        ["cargo", "test", "-p", "oblivion_testkit", "--test", "phase15_initial_area", "--", "--nocapture", "--test-threads=1"],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=True,
    )
    out = {}
    for name, prefix in RUST_LINES.items():
        line = next(line for line in run.stdout.splitlines() if line.startswith(prefix))
        body = re.search(r"boundaries \{(.*)\}", line).group(1)
        out[name] = {f"{int(a, 16):08X}": int(n, 16) for a, n in re.findall(r"([0-9A-F]+): ([0-9A-F]+)", body)}
    COUNTS_CACHE.write_text(json.dumps(out, indent=1, sort_keys=True) + "\n", encoding="utf-8")
    return out


def build():
    notes = ROADMAP.read_text(encoding="utf-8")
    sections = carried_lists(notes)
    from_notes = defaultdict(list)
    for name, _, body in sections:
        for address in addresses_in_notes(body):
            if name not in from_notes[address]:
                from_notes[address].append(name)
    sources = rust_sources()
    rust = RustEvidence(sources)
    rust.finish_named_counters(sources)
    cpp = cpp_sources()
    cpp_stops, cpp_counts = cpp_stops_and_counts(cpp)
    rust.sources = sources
    return sections, from_notes, rust, cpp_stops, cpp_counts


def render(check_only=False):
    sections, from_notes, rust, cpp_stops, cpp_counts = build()
    review = load_review()
    items = review.get("items", {})
    addresses = sorted(set(from_notes) | set(cpp_stops) | set(cpp_counts))
    rows = []
    tally = Counter()
    auto_tally = Counter()
    decision_tally = Counter()
    stale = []
    for address in addresses:
        cpp_message = cpp_stops[address][0][1] if address in cpp_stops else None
        auto, evidence = rust.classify(address, cpp_message)
        auto_tally[auto] += 1
        decided = items.get(address)
        final, note = auto, ""
        if decided:
            final, note = decided["class"], f"{decided['decision']}: {decided.get('note', '')}"
            decision_tally[decided["decision"]] += 1
            if decided.get("auto") != auto:
                stale.append(address)
                note = f"STALE (reviewed against {decided.get('auto')}, now {auto}): " + note
        tally[final] += 1
        origin = []
        if from_notes.get(address):
            origin.append(",".join(from_notes[address]))
        if address in cpp_stops:
            origin.append(f"C++ stop x{len(cpp_stops[address])}")
        if address in cpp_counts:
            origin.append(f"C++ count x{len(cpp_counts[address])}")
        rows.append((address, final, auto, " ".join(origin), "; ".join(evidence), note))

    named = review.get("named_counters", {})
    counted_review = review.get("counted", {})
    differing = []
    counted_rows = []
    for address in sorted(set(cpp_counts) | {a for a, s in rust.counted.items() if any(h == "count" for _, h in s)}):
        cpp_sites = len(cpp_counts.get(address, []))
        rust_sites = sum(1 for _, how in rust.counted.get(address, []) if how == "count")
        note = counted_review.get(address, "")
        if cpp_sites != rust_sites and not note:
            differing.append(address)
        counted_rows.append((address, cpp_sites, rust_sites, note))

    unnamed_stops = []
    section_four, whole = open_items_section_four()
    for address, where, body in sorted(set(rust.all_stop_literals)):
        if address not in cpp_stops and address not in from_notes and address not in whole:
            unnamed_stops.append((address, where, body))
    unnamed_counters = sorted(
        {
            (address, where, how)
            for address, where, how in rust.all_counters
            if address not in cpp_counts and address not in from_notes and address not in whole
        }
    )
    decisions = review.get("items", {})
    unlisted = [a for a, f, *_ in rows if f == "missing" and a not in decisions]
    # A boundary's direct exe callers inside ported functions.
    carried_late = {a for a, names in from_notes.items() if any(n in ("P12", "P13", "P14", "P15", "P16") for n in names)}
    caller_rows = silent_passes(rust, rust.sources, set(rust.counted) | set(cpp_counts) | carried_late)
    callers_reviewed = review.get("callers", {})
    unreviewed_callers = [r for r in caller_rows if r[5] == "none" and r[0] not in callers_reviewed]
    _, instructions, _ = exe_calls()
    off_instruction = sorted(
        {
            (address, where, body)
            for address, where, body in rust.all_stop_literals + rust.all_panic_literals
            if instructions and int(address, 16) < 0x009A0000 and int(address, 16) not in instructions
        }
    )

    out = []
    out.append("# Boundary audit (G4)")
    out.append("")
    out.append(
        "**A snapshot, not current status:** the table as last generated. Its source-line references and classes go "
        "stale as the code changes; regenerate it before relying on any row, and don't hand-edit it."
    )
    out.append("")
    out.append(
        "Generated by `tools/boundary_audit.py`; hand decisions are in `tools/boundary_audit_review.json`. "
        "Sources: the roadmap's carried lists, the C++ stops (`throw` or `stop` whose message leads with an address, and "
        "`Phase-NN pending`) and `count(0x...)` sites. A class comes from what the Rust code does. A comment alone is `missing`."
    )
    out.append("")
    out.append("## Counts")
    out.append("")
    out.append("| Class | Automatic | After review |")
    out.append("| --- | ---: | ---: |")
    for name in ("ported", "stop", "counted", "substitution", "missing"):
        out.append(f"| {name} | {auto_tally[name]} | {tally[name]} |")
    out.append(f"| **total** | {sum(auto_tally.values())} | {sum(tally.values())} |")
    out.append("")
    out.append(f"Carried sections read: {len(sections)}. Reviewed by hand: {len(items)}. Stale decisions: {len(stale)}. Unreviewed `missing`: {len(unlisted)}.")
    if len(decision_tally):
        out.append("")
        out.append("Decisions on the rows the code alone calls `missing`:")
        out.append("")
        out.append("| Decision | Rows |")
        out.append("| --- | ---: |")
        for name, count in sorted(decision_tally.items()):
            out.append(f"| {name} | {count} |")
    out.append("")
    out.append("## Named counters")
    out.append("")
    out.append("Fields that count a boundary, with the counts on the shipped data (`named_counters` in the review file: C++ log, roadmap, Rust).")
    out.append("")
    out.append("| Field | Boundary | C++ log | Roadmap | Rust | Verdict |")
    out.append("| --- | --- | ---: | ---: | ---: | --- |")
    for name, entry in sorted(named.items()):
        out.append(
            f"| `{name}` | {entry.get('boundary','')} | {entry.get('cpp','')} | {entry.get('roadmap','')} | {entry.get('rust','')} | {entry.get('verdict','')} |"
        )
    out.append("")
    out.append("## `count(0x...)` sites, C++ against Rust")
    out.append("")
    out.append("| Address | C++ sites | Rust sites | Reason if they differ |")
    out.append("| --- | ---: | ---: | --- |")
    for address, cpp_sites, rust_sites, note in counted_rows:
        out.append(f"| `{address}` | {cpp_sites} | {rust_sites} | {note} |")
    out.append("")
    out.append("## Counted boundaries per path, C++ log against Rust")
    out.append("")
    out.append(
        "The C++ log's `15c`, `15d exit` and `15d enter` lines against `phase15_initial_area` (`--run-tests` refreshes the Rust side). Only the rows that differ; the C++ test runs on its own fixture."
    )
    out.append("")
    cpp_paths = cpp_path_counts()
    rust_paths = json.loads(COUNTS_CACHE.read_text(encoding="utf-8")) if COUNTS_CACHE.exists() else {}
    path_notes = review.get("paths", {})
    for name in C15_LINES:
        cpp_row, rust_row = cpp_paths.get(name, {}), rust_paths.get(name, {})
        shared = sum(1 for a in cpp_row if rust_row.get(a) == cpp_row[a])
        out.append(f"**{name}:** {len(cpp_row)} addresses in the C++, {len(rust_row)} in the Rust, {shared} the same.")
        out.append("")
        out.append("| Address | C++ | Rust | Reason |")
        out.append("| --- | ---: | ---: | --- |")
        for address in sorted(set(cpp_row) | set(rust_row)):
            if cpp_row.get(address) != rust_row.get(address):
                note = path_notes.get(address, "")
                if not note:
                    differing.append(f"{name}:{address}")
                out.append(f"| `{address}` | {cpp_row.get(address, '-')} | {rust_row.get(address, '-')} | {note} |")
        out.append("")
    out.append("## Direct callers of a boundary in a ported function, with no sign of the call")
    out.append("")
    out.append(
        f"{len(caller_rows)} direct exe calls of counted or late-carried addresses sit in functions the Rust ports: "
        + ", ".join(f"{k} {v}" for k, v in sorted(Counter(r[5] for r in caller_rows).items()))
        + ". `none` means that Rust function's own text doesn't show the call; each was read, since a helper or an earlier stop may hold it."
    )
    out.append("")
    out.append("| Target | Sites | In | Decision |")
    out.append("| --- | --- | --- | --- |")
    none_rows = defaultdict(list)
    for row in caller_rows:
        if row[5] == "none":
            none_rows[row[0]].append(row)
    for target, group in sorted(none_rows.items()):
        sites = " ".join(sorted(r[1] for r in group))
        where = ", ".join(sorted({f"{r[2]}" for r in group}))
        out.append(f"| `{target}` | {sites} | {where} | {callers_reviewed.get(target, 'UNREVIEWED')} |")
    out.append("")
    out.append("## Rust stops whose address isn't an instruction start")
    out.append("")
    out.append("A label, not behaviour: the message's address doesn't begin an instruction in the exe (each is inside the one the note means).")
    out.append("")
    out.append("| Address | Where | Message |")
    out.append("| --- | --- | --- |")
    for address, where, body in off_instruction:
        out.append(f"| `{address}` | {where} | {body[:100].replace('|', '/')} |")
    out.append("")
    out.append("## Rust stops no list names")
    out.append("")
    out.append(
        "An error literal leading with an address that the carried lists, the C++ stops and `docs/open-items.md` don't name."
    )
    out.append("")
    out.append("| Address | Where | Message |")
    out.append("| --- | --- | --- |")
    for address, where, body in unnamed_stops:
        out.append(f"| `{address}` | {where} | {body[:100].replace('|', '/')} |")
    out.append("")
    if unnamed_counters:
        out.append("## Rust counters no list names")
        out.append("")
        out.append("| Address | Where | How |")
        out.append("| --- | --- | --- |")
        for address, where, how in unnamed_counters:
            out.append(f"| `{address}` | {where} | {how} |")
        out.append("")
    out.append("## Every address")
    out.append("")
    out.append("`Auto` is the class the code gives; `Class` is after hand review.")
    out.append("")
    out.append("| Address | Class | Auto | From | Evidence | Note |")
    out.append("| --- | --- | --- | --- | --- | --- |")
    for address, final, auto, origin, evidence, note in rows:
        cell = evidence.replace("|", "/")
        out.append(f"| `{address}` | {final} | {auto} | {origin} | {cell} | {note.replace('|', '/')} |")
    out.append("")
    return "\n".join(out), unlisted, differing, stale, rows, unreviewed_callers


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="write nothing; fail on a change or an open item")
    parser.add_argument("--run-tests", action="store_true", help="refresh the Rust per-path counts first")
    parser.add_argument("--list", choices=["missing", "counted", "stop", "ported", "substitution"], help="print one automatic class")
    args = parser.parse_args()
    if args.run_tests:
        run_rust_path_counts()
    text, missing, differing, stale, rows, unreviewed = render()
    if args.list:
        for address, final, auto, origin, evidence, note in rows:
            if auto == args.list:
                print(address, final, origin, "|", evidence)
        return 0
    print(
        f"{len(rows)} addresses; unreviewed missing {len(missing)}; counted differing with no reason {len(differing)}; "
        f"stale {len(stale)}; unreviewed callers {len(unreviewed)}"
    )
    for item in differing:
        print("  differs:", item)
    if args.check:
        current = OUTPUT.read_text(encoding="utf-8") if OUTPUT.exists() else ""
        changed = current != text
        if changed:
            print("docs/boundary-audit.md is out of date")
        return 1 if changed or missing or differing or stale or unreviewed else 0
    OUTPUT.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
