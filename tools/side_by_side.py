#!/usr/bin/env python3
"""The M7 gate's table: G1 to G6 on one commit (G4 by tools/boundary_audit.py) (docs/migration-plan.md, "The gate").

    python3 tools/virtual_display.py start ${XDG_RUNTIME_DIR}/ob-gate
    python3 tools/virtual_display.py exec ${XDG_RUNTIME_DIR}/ob-gate -- python3 tools/side_by_side.py run
    python3 tools/virtual_display.py exec ${XDG_RUNTIME_DIR}/ob-gate -- python3 tools/side_by_side.py mutate
    python3 tools/virtual_display.py exec ${XDG_RUNTIME_DIR}/ob-gate -- python3 tools/side_by_side.py g5 --cpp-captures DIR
    python3 tools/side_by_side.py report
    python3 tools/virtual_display.py stop ${XDG_RUNTIME_DIR}/ob-gate

`run` executes every test binary of the workspace, one at a time, with OBLIVION_ROW_LEDGER set, and the real
binary for the G3 lines. `mutate` changes rows in a scratch copy of fixtures/ and sees whether any test notices:
that is the only count here of rows no test compares, since a tracker's "read" doesn't say a value was compared.
`report` prints what the last `run`, `mutate` and `g5` left in <target>/side_by_side, and never runs anything but
the G6 source scan and git.

No number below is a constant that restates an expected answer. The only hand-written data is TOLERANCES: which G3
fields may differ and why. The values come from the runs, the logs and the evidence.

Keep strict closure (rows closed out of total) and bounded coverage (a sample) apart. This tool prints no percentage.
Host tests earn no validation bonus on the 110% scale.
"""
import argparse
import concurrent.futures as futures
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "fixtures"
TARGET = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
OUT = TARGET / "side_by_side"

# G3: the fields of a `Phase-NN:` line that may differ from the C++ logs, each with its reason. Anything else must
# match exactly. `oracle` says what the original recorded, read from the evidence when a function here can.
# The user's decision, 2026-11-06: G5 passes for now, with the logo open (docs/open-items.md).
G5_DECISION = "PASS by the user's decision, 2026-11-06, against the original's own frames, with the logo open (docs/open-items.md)"
TOLERANCES = {
    "Bink sound": "P06-RUN-001 and P06-RUN-002 record eax=1 at 0040EADB (the port's 1). The C++ x64 host's FFmpeg answers 0, so the C++ is the one that differs.",
    "editor IDs": "The C++'s own count: it logs 7,939 and 7,938, where P12-RUN-001 and the port give 7,896.",
    "item scripts not executed": "12d's leveled rolls: the time-seeded CRT at 0047DF80 picks the items, so x64 and x86 differ on their own record. Checked against a spread of seeds below.",
    "doors not registered": "The C++ counts every 0x18 door reference. The exe's 00680090 gates at 006800B4 and 004D7630 count fewer. No run counted this: a reading of the exe.",
    "change records": "12d's and 14c's rolls: x64 and x86 differ on their own record. Checked against a spread of seeds below.",
    "view": "The C++ host's own LoadingView text racing its movie thread (x64 pending, x86 adopted). Not an original value at all.",
}

# A test whose job is to hold a row against the run's records says nothing about the port.
EVIDENCE_TESTS = re.compile(r"match_their_evidence|the_rows_are_the_raw_records|rows_are_verbatim")
# A panic at one of these places is the row against its evidence, not against the port.
EVIDENCE_SITE = re.compile(r"record\(|records|record_hex|recorded_|evidence|not in the", re.IGNORECASE)
STRUCTURAL = re.compile(r"no row|missing expected|no .* row|`None`|ParseIntError|called `Result::unwrap|out of range|not in the run|unknown|no record")


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True).stdout.strip()


def need_display():
    if not os.environ.get("OBLIVION_VIRTUAL_DISPLAY_STATE"):
        sys.exit("run this through tools/virtual_display.py exec: tests stay on the virtual display")


def environment(**extra):
    env = os.environ.copy()
    # As .cargo/config.toml's `../OblivionRecomp/Data`, relative to the repository.
    env.setdefault("OBLIVION_DATA_DIR", str(ROOT.parent / "OblivionRecomp/Data"))
    env.update(extra)
    return env


def test_binaries():
    """Every test executable of the workspace, from cargo itself, as (name, path, package folder)."""
    env = environment()
    env["CARGO_TARGET_DIR"] = str(TARGET)
    done = subprocess.run(
        ["cargo", "test", "--workspace", "--no-run", "--message-format=json"],
        cwd=ROOT, env=env, capture_output=True, text=True)
    if done.returncode != 0:
        sys.exit(done.stderr[-4000:])
    found = []
    for line in done.stdout.splitlines():
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if message.get("reason") == "compiler-artifact" and message.get("executable") and message["profile"]["test"]:
            target = message["target"]
            folder = str(Path(message["manifest_path"]).parent)
            found.append((f"{Path(folder).name}:{target['name']}", message["executable"], folder))
    return found


def read_rows(name):
    """A rows file's lines, as `fixtures::rows` reads them: Latin-1, split on line ends."""
    text = (FIXTURES / "rows" / f"{name}.rows").read_bytes().decode("latin-1")
    return text.splitlines()


def row_files():
    return sorted(path.stem for path in (FIXTURES / "rows").glob("*.rows"))


def is_data_row(line):
    return bool(line) and not line.startswith("#")


def phase_of(name):
    match = re.match(r"phase(\d+)([a-z]?)", name)
    if not match:
        # Past the roadmap: `newgame`.
        return name
    return f"{match.group(1)}{match.group(2)}"


TEST_LINE = re.compile(r"^test (\S+)(?: - should panic)? \.\.\. (\w+)", re.M)


def parse_ledger(path, binary):
    """Ledger lines, with the binary added to the test's name."""
    entries = []
    if not path.exists():
        return entries
    for line in path.read_text(errors="replace").splitlines():
        fields = line.split("\t")
        entries.append({"kind": fields[0], "fields": fields[1:-1], "test": fields[-1], "binary": binary})
    return entries


def command_run(args):
    need_display()
    OUT.mkdir(parents=True, exist_ok=True)
    started = time.time()
    results = {"commit": git("rev-parse", "HEAD"), "dirty": bool(git("status", "--porcelain")),
               "binaries": {}, "tests": [], "ledger": []}
    ledger_dir = OUT / "ledger"
    shutil.rmtree(ledger_dir, ignore_errors=True)
    ledger_dir.mkdir()
    binaries = test_binaries()
    # Hard links, so a later rebuild doesn't change the binaries a mutation pass runs.
    held = OUT / "bin"
    shutil.rmtree(held, ignore_errors=True)
    held.mkdir()
    linked = []
    for name, path, folder in binaries:
        copy = held / name.replace(":", "_")
        os.link(path, copy)
        linked.append((name, str(copy), folder))
    (OUT / "binaries.json").write_text(json.dumps(linked, indent=1))
    for name, path, folder in binaries:
        ledger = ledger_dir / (name.replace(":", "_") + ".tsv")
        begun = time.time()
        # Binaries run one at a time: the plan records a flake when phases ran in parallel.
        done = subprocess.run([path], cwd=folder, capture_output=True, text=True, errors="replace",
                              env=environment(OBLIVION_ROW_LEDGER=str(ledger)))
        stdout = done.stdout
        (ledger_dir / (name.replace(":", "_") + ".log")).write_text(stdout + done.stderr)
        tests = TEST_LINE.findall(stdout)
        results["binaries"][name] = {"exit": done.returncode, "seconds": round(time.time() - begun, 1)}
        for test, status in tests:
            results["tests"].append({"binary": name, "test": test, "status": status})
        results["ledger"].extend(parse_ledger(ledger, name))
        passed = sum(1 for _, status in tests if status == "ok")
        print(f"{name}: {passed} of {len(tests)} ok, exit {done.returncode}, {time.time() - begun:.1f} s", flush=True)
    results["g3"] = run_g3()
    audit = subprocess.run([sys.executable, str(ROOT / "tools/boundary_audit.py"), "--run-tests", "--check"], cwd=ROOT,
                           env=environment(), capture_output=True, text=True, errors="replace")
    results["g4"] = {"exit": audit.returncode, "stdout": audit.stdout[-2000:]}
    print("G4 audit: exit", audit.returncode, flush=True)
    results["seconds"] = round(time.time() - started)
    (OUT / "run.json").write_text(json.dumps(results))
    print(f"wrote {OUT / 'run.json'}")


# ---- G3 ----


def app_binary():
    env = environment()
    env["CARGO_TARGET_DIR"] = str(TARGET)
    done = subprocess.run(["cargo", "build", "--bin", "OblivionRecompBevy", "--message-format=json"],
                          cwd=ROOT, env=env, capture_output=True, text=True)
    for line in done.stdout.splitlines():
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if message.get("reason") == "compiler-artifact" and message.get("executable"):
            return message["executable"]
    sys.exit("no app binary: " + done.stderr[-2000:])


def phase_lines(text):
    return [line[len("[INFO] "):].rstrip() for line in text.splitlines()
            if line.startswith("[INFO] Phase-")]


def run_app(binary, seed, ini=None):
    done = subprocess.run(
        [binary, "--ini", str(ini or FIXTURES / "config/Oblivion.ini"), "--seed", str(seed),
         "--exit-after-frames", "5", "--no-audio-device"],
        cwd=ROOT, env=environment(), capture_output=True, text=True, errors="replace", timeout=600)
    return {"exit": done.returncode, "lines": phase_lines(done.stdout),
            "host_stops": re.findall(r"(\d+) host stops", done.stdout)}


def baseline_lines(log, test="app_startup"):
    text = (FIXTURES / "cpp-baseline" / log).read_bytes().decode("latin-1")
    start = re.search(rf"Testing: {test}\r?\n", text).start()
    body = text[start:]
    body = body[:body.index("<end of output>")]
    return [line[len("[INFO] "):].rstrip() for line in body.splitlines() if line.startswith("[INFO] Phase-")]


def baseline_status(log):
    """Each ctest test of a log and whether it passed."""
    text = (FIXTURES / "cpp-baseline" / log).read_bytes().decode("latin-1")
    found = {}
    pieces = re.split(r"(?m)^\d+/\d+ Testing: ", text)[1:]
    for piece in pieces:
        name = piece.split("\n", 1)[0].strip()
        status = re.search(r"Test (Passed|Failed|Not Run)", piece)
        found[name] = status.group(1) if status else "no result in the log"
    return found


def fields_of(line):
    """A line's `name value` fields, for naming what differs: the words before each number or word after a comma."""
    return [part.strip() for part in line.split(", ")]


def tolerated(line):
    # The C++ leaves 11 models queued (8 bhk, 3 NIF 10.2.0.0). The original builds them, and so does the port.
    if line.startswith("Phase-13e:"):
        return "Phase-13e: ~", [("Phase-13e", line.split(": ", 1)[1])]
    out = line
    used = []
    for key in TOLERANCES:
        match = re.search(re.escape(key) + r" ([^, ]+)", out)
        if match:
            used.append((key, match.group(1)))
            out = out[:match.start(1)] + "~" + out[match.end(1):]
    return out, used


def compare_g3(ours, log_lines):
    """Every line in order: exact, tolerated with the fields named, or different."""
    rows = []
    for index in range(max(len(ours), len(log_lines))):
        mine = ours[index] if index < len(ours) else "<missing>"
        theirs = log_lines[index] if index < len(log_lines) else "<missing>"
        mine_t, mine_used = tolerated(mine)
        theirs_t, theirs_used = tolerated(theirs)
        moved = [(key, value, dict(theirs_used).get(key)) for key, value in mine_used
                 if dict(theirs_used).get(key) != value]
        rows.append({"line": mine, "cpp": theirs, "exact": mine == theirs, "match": mine_t == theirs_t,
                     "tolerated": moved})
    return rows


def run_g3():
    binary = app_binary()
    seeds = {}
    first = run_app(binary, 1)
    print("G3 seed 1:", first["exit"], flush=True)
    logs = {log: baseline_lines(log) for log in ["LastTest-x64.log", "LastTest-x86.log"]}
    # Which fields a seed really moves: a tolerance that says "rolls" has to move with the rolls.
    for seed in [1, 2, 3, 7, 0x60000000, 0x60000001, 0x12345678, 0x7FFFFFFF]:
        seeds[seed] = run_app(binary, seed)["lines"]
    return {"lines": first["lines"], "exit": first["exit"], "host_stops": first["host_stops"],
            "by_log": {log: compare_g3(first["lines"], lines) for log, lines in logs.items()},
            "seeds": {str(seed): lines for seed, lines in seeds.items()}}


def recorded_editor_ids():
    """P12-RUN-001's `M 714 4067CC FINAL edids=...`, from the evidence."""
    path = FIXTURES / "evidence/phase12-run001-2026-10-09.records.txt"
    for line in path.read_bytes().decode("latin-1").splitlines():
        if " 4067CC FINAL " in line and "edids=" in line:
            return int(re.search(r"edids=([0-9A-Fa-f]+)", line).group(1), 16), line
    return None, None


def field_value(lines, key):
    for line in lines:
        match = re.search(re.escape(key) + r" ([^, ]+)", line)
        if match:
            return match.group(1)
    return None


# ---- Mutation ----


VALUE = re.compile(r"^-?(0[xX])?[0-9A-Fa-f]+[Uu]?$|^-?\d+\.\d+(e[-+]?\d+)?$")
TOKEN = re.compile(r"[^\s|,:;=/{}()\[\]\"']+")


def tokens_of(line):
    """A row's value tokens, as (start, end, field name). The first token (the kind or the run) never counts; `seq=` neither: it names a record."""
    found = []
    if "|" in line and not line.startswith("{"):
        # Column rows: past the block prefix, the run and the kind, each column is a value; a last column with a space or `key=value` is the record it came from.
        columns = line.split("|")
        skip = 3 if columns[0] in {"SCENE", "SHD", "TARGETS", "MOVIE", "WINDOW"} else 2
        if columns[0] in {"MODEL", "NODE"}:
            skip = 1
        # These rows have no name column: the first past the kind is a value.
        if columns[0] == "MOVIE" and columns[2] in {"PATHMAP", "PATHBUILDER", "PATHTHREAD", "SOUND", "SEQUENCE_RET", "TILEMENU"}:
            skip = 2
        at = 0
        for index, column in enumerate(columns):
            is_needle = index == len(columns) - 1 and (" " in column or "=" in column)
            # The first column past the kind names the row: a lookup key, not a value.
            if index > skip and column and not is_needle:
                found.append((at, at + len(column), f"{index}"))
            at += len(column) + 1
        if found:
            return found
    words = list(TOKEN.finditer(line))
    for index, match in enumerate(words):
        if index == 0 or not VALUE.match(match.group()):
            continue
        # A DATA row's type, ID and offset name it. A wrong name makes it a dump behind no pointer, which is declared.
        if words[0].group() == "DATA" and index <= 3:
            continue
        before = line[:match.start()]
        # A comma list under one key (`proc=a,b,c`) counts under that key.
        key = re.search(r"([A-Za-z_][\w.]*)=[^ =]*$", before)
        name = key.group(1) if key else f"{index}"
        # `seq` names a record and `n` a walk's index. The first half of `pointer/id` is the run's heap: the test reads the ID.
        if name in {"seq", "n"} or line[match.end():match.end() + 1] == "/":
            continue
        # Don't take a key's name for a value (`f1f4=..`, `b24=..`).
        if line[match.end():match.end() + 1] == "=":
            continue
        found.append((match.start(), match.end(), name))
    if not found and len(words) > 1:
        last = words[-1]
        found.append((last.start(), last.end(), f"{len(words) - 1}"))
    return found


def change_char(char):
    if char.isdigit():
        return "1" if char == "0" else "0"
    if char in "abcdefABCDEF":
        return {"a": "b", "A": "B"}.get(char, "a" if char.islower() else "A")
    if char.isalpha():
        return "a" if char.lower() != "a" else "b"
    return None


def mutate_token(text):
    """The same length and alphabet, so a parser still reads it. A long dump changes at its start, middle and end, as a compare may cover only part."""
    suffix = ""
    if re.match(r"^-?0[xX][0-9A-Fa-f]+[Uu]$", text):
        text, suffix = text[:-1], text[-1]
    characters = list(text)
    places = [len(text) - 1]
    if len(text) > 16:
        places = [0, len(text) // 2, len(text) - 1]
    changed = False
    for place in places:
        at = place
        while at >= 0 and change_char(characters[at]) is None:
            at -= 1
        if at >= 0:
            characters[at] = change_char(characters[at])
            changed = True
    return "".join(characters) + suffix if changed else None


def mutate_line(line, spans):
    out = line
    for start, end, _ in sorted(spans, reverse=True):
        token = mutate_token(out[start:end])
        if token is not None:
            out = out[:start] + token + out[end:]
    return out


def group_key(line):
    words = TOKEN.findall(line)
    if not words:
        return "?"
    head = words[0]
    if re.match(r"P\d\d-RUN-\d+", head) or head in {"SCENE", "SHD", "TARGETS", "MOVIE", "WINDOW"}:
        return " ".join(words[:3])
    if re.match(r"RUN-|R\d", words[1] if len(words) > 1 else ""):
        return " ".join(words[:3])
    # A positional kind's second word, when it reads as a tag and not a value.
    if len(words) > 1 and not VALUE.match(words[1]):
        return " ".join(words[:2])
    return head


def plan_rows(per_group, whole_under):
    """Which rows the row pass changes: all of a small file; `per_group` evenly spaced from each group of a large one."""
    plan = []
    for name in row_files():
        lines = read_rows(name)
        indexes = [index for index, line in enumerate(lines) if is_data_row(line)]
        if len(indexes) <= whole_under:
            plan.extend((name, index) for index in indexes)
            continue
        groups = {}
        for index in indexes:
            groups.setdefault(group_key(lines[index]), []).append(index)
        for members in groups.values():
            if len(members) <= per_group:
                plan.extend((name, index) for index in members)
            else:
                step = (len(members) - 1) / (per_group - 1) if per_group > 1 else 1
                plan.extend((name, members[round(step * at)]) for at in range(per_group))
    return plan


def plan_fields(per_group):
    """The field pass: `per_group` rows of each group, each value on its own."""
    plan = []
    for name in row_files():
        lines = read_rows(name)
        groups = {}
        for index, line in enumerate(lines):
            if is_data_row(line):
                groups.setdefault(group_key(line), []).append(index)
        for members in groups.values():
            step = max(1, len(members) // per_group)
            for index in members[::step][:per_group]:
                for span in tokens_of(lines[index]):
                    plan.append((name, index, span))
    return plan


def edit_evidence(folder, phase, original, mutated, spans):
    """The same change in the run's records, so a check of a row against its evidence still passes and what's behind it gets its turn."""
    pairs = [(original[start:end], mutate_token(original[start:end])) for start, end, _ in spans
             if end - start >= 6]
    # A row that is a record verbatim, after its run ID.
    verbatim = re.sub(r"^P\d\d-RUN-\d+ ", "", original)
    changed = re.sub(r"^P\d\d-RUN-\d+ ", "", mutated)
    evidence = folder / "evidence"
    evidence.mkdir()
    wrote = False
    for path in (FIXTURES / "evidence").iterdir():
        target = evidence / path.name
        if not path.name.startswith(f"phase{phase[:2]}"):
            target.symlink_to(path)
            continue
        text = path.read_bytes().decode("latin-1")
        edited = text.replace(verbatim, changed) if verbatim != original else text
        for old, new in pairs:
            if new:
                edited = edited.replace(old, new)
        if edited == text:
            target.symlink_to(path)
        else:
            target.write_bytes(edited.encode("latin-1"))
            wrote = True
    return wrote


def scratch_fixtures(folder, name, lines, evidence_edit=None):
    folder.mkdir(parents=True)
    wrote = False
    if evidence_edit:
        wrote = edit_evidence(folder, phase_of(name), *evidence_edit)
    for entry in FIXTURES.iterdir():
        if entry.name != "rows" and not (evidence_edit and entry.name == "evidence"):
            (folder / entry.name).symlink_to(entry)
    rows = folder / "rows"
    rows.mkdir()
    for path in (FIXTURES / "rows").iterdir():
        if path.stem != name:
            (rows / path.name).symlink_to(path)
    (rows / f"{name}.rows").write_bytes(("\n".join(lines) + "\n").encode("latin-1"))
    return wrote


PANIC = re.compile(r"panicked at ([^:\n]+):(\d+):\d+:\n(.*)")


def classify_failure(output):
    match = PANIC.search(output)
    if not match:
        return "failed", None, output[-300:]
    path, line, message = match.group(1), int(match.group(2)), match.group(3)
    location = f"{path}:{line}"
    source = ROOT / path
    window = ""
    if source.exists():
        text = source.read_text(errors="replace").splitlines()
        window = "\n".join(text[max(0, line - 3):line + 2])
    if EVIDENCE_SITE.search(window) or "not in the" in message:
        return "evidence-only", location, message[:200]
    if STRUCTURAL.search(message):
        return "structural", location, message[:200]
    return "compared", location, message[:200]


class Sweep:
    def __init__(self):
        run = json.loads((OUT / "run.json").read_text())
        self.binaries = {name: (path, folder) for name, path, folder in json.loads((OUT / "binaries.json").read_text())}
        self.loaders = {}
        for entry in run["ledger"]:
            if entry["kind"] == "LOAD" and not EVIDENCE_TESTS.search(entry["test"]):
                self.loaders.setdefault(entry["fields"][0], {}).setdefault((entry["binary"], entry["test"]), 0)
        passed = {(test["binary"], test["test"]) for test in run["tests"] if test["status"] == "ok"}
        for name in self.loaders:
            self.loaders[name] = {key: 0 for key in self.loaders[name] if key in passed}

    def run_one(self, name, lines, tag, evidence_edit=None):
        """The file's tests one by one, the one that last caught something first, until one fails."""
        with tempfile.TemporaryDirectory(prefix="sbs") as scratch:
            scratch = Path(scratch)
            wrote = scratch_fixtures(scratch / "fixtures", name, lines, evidence_edit)
            if evidence_edit and not wrote:
                # Nothing in the records changed, so failing again says nothing new.
                return {"status": "evidence-only", "test": "", "location": None, "no_overlay": True}
            temp = scratch / "tmp"
            temp.mkdir()
            order = sorted(self.loaders.get(name, {}), key=lambda key: -self.loaders[name][key])
            for binary, test in order:
                path, folder = self.binaries[binary]
                try:
                    done = subprocess.run(
                        [path, "--exact", test], cwd=folder, capture_output=True, text=True, errors="replace",
                        timeout=900,
                        env=environment(OBLIVION_FIXTURES_ROOT=str(scratch / "fixtures"), TMPDIR=str(temp)))
                except subprocess.TimeoutExpired:
                    return {"status": "timeout", "test": test}
                if done.returncode != 0:
                    self.loaders[name][(binary, test)] += 1
                    status, location, message = classify_failure(done.stdout + done.stderr)
                    return {"status": status, "test": test, "binary": binary, "location": location, "message": message}
        return {"status": "uncaught", "tests": len(order)}


def command_mutate(args):
    need_display()
    OUT.mkdir(parents=True, exist_ok=True)
    sweep = Sweep()
    path = OUT / f"mutate-{args.mode}.jsonl"
    done = set()
    if path.exists() and not args.fresh:
        for line in path.read_text().splitlines():
            entry = json.loads(line)
            done.add((entry["file"], entry["line"], entry.get("field", "")))
    else:
        path.write_text("")
    contents = {name: read_rows(name) for name in row_files()}
    jobs = []
    if args.mode == "rows":
        for name, index in plan_rows(args.per_group, args.whole_under):
            if args.files and name not in args.files:
                continue
            if (name, index, "") not in done:
                jobs.append((name, index, None))
    else:
        for name, index, span in plan_fields(args.fields_per_group):
            if args.files and name not in args.files:
                continue
            if (name, index, span[2]) not in done:
                jobs.append((name, index, span))
    print(f"{len(jobs)} mutants to run ({len(done)} done before)", flush=True)

    def one(job):
        name, index, span = job
        lines = list(contents[name])
        original = lines[index]
        spans = [span] if span else tokens_of(original)
        lines[index] = mutate_line(original, spans)
        if lines[index] == original:
            return {"file": name, "line": index, "field": span[2] if span else "", "status": "unchanged"}
        result = sweep.run_one(name, lines, index)
        if result["status"] == "evidence-only":
            # Past the row's own check: the same change in the records. Failing at the same place now isn't the evidence's doing.
            again = sweep.run_one(name, lines, index, (original, lines[index], spans))
            if again.get("no_overlay"):
                again = {**result, "status": "evidence-only"}
            elif again["status"] == "evidence-only" and again.get("location") == result.get("location"):
                again["status"] = "compared"
            elif again["status"] == "uncaught":
                again = {**result, "status": "evidence-only"}
            result = again
        if result["status"] == "uncaught" and not span and len(spans) > 1:
            # Changing a row's key with its value can land on another row that happens to agree. One value at a time decides.
            for single in spans:
                retry = list(contents[name])
                retry[index] = mutate_line(original, [single])
                if retry[index] == original:
                    continue
                attempt = sweep.run_one(name, retry, index)
                if attempt["status"] != "uncaught":
                    result = {**attempt, "only_field": single[2]}
                    break
        result.update({"file": name, "line": index, "field": span[2] if span else "", "row": original[:160]})
        return result

    count = 0
    started = time.time()
    with open(path, "a") as output, futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for result in pool.map(one, jobs):
            output.write(json.dumps(result) + "\n")
            output.flush()
            count += 1
            if count % 50 == 0:
                print(f"{count} of {len(jobs)}, {time.time() - started:.0f} s", flush=True)
    print(f"wrote {path}")


# ---- G5 ----


def read_image(path, out_rgba):
    """Any image as raw RGBA through ffmpeg, so this tool needs no imaging library."""
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i", str(path),
                    "-f", "rawvideo", "-pix_fmt", "rgba", str(out_rgba)], check=True)
    probe = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height",
                            "-of", "csv=p=0", str(path)], capture_output=True, text=True, check=True)
    width, height = probe.stdout.strip().split(",")[:2]
    return int(width), int(height), Path(out_rgba).read_bytes()


# The map under the band of buttons and logo: the movie alone, in both pictures.
MOVIE_ONLY = {"movie area": (700, 480, 1640, 800)}


def compare_images(ours, cpp, diff_path, boxes=MOVIE_ONLY):
    with tempfile.TemporaryDirectory() as scratch:
        width, height, mine = read_image(ours, f"{scratch}/a.rgba")
        cpp_width, cpp_height, theirs = read_image(cpp, f"{scratch}/b.rgba")
    result = {"ours": str(ours), "cpp": str(cpp), "ours_size": [width, height], "cpp_size": [cpp_width, cpp_height]}
    if (width, height) != (cpp_width, cpp_height):
        result["error"] = "sizes differ"
        return result

    def stats(region):
        (left, top, right, bottom) = region
        pixels = over2 = over16 = over64 = 0
        total = peak = 0
        for y in range(top, bottom):
            row = y * width * 4
            for x in range(left, right):
                at = row + x * 4
                worst = max(abs(mine[at] - theirs[at]), abs(mine[at + 1] - theirs[at + 1]), abs(mine[at + 2] - theirs[at + 2]))
                pixels += 1
                total += worst
                peak = max(peak, worst)
                over2 += worst > 2
                over16 += worst > 16
                over64 += worst > 64
        return {"pixels": pixels, "mean_worst_channel": round(total / pixels, 3), "peak": peak,
                "over_2": over2, "over_16": over16, "over_64": over64}

    result["whole"] = stats((0, 0, width, height))
    for name, region in (boxes or {}).items():
        result[name] = stats(region)
    picture = bytearray(width * height * 3)
    for index in range(width * height):
        at = index * 4
        worst = max(abs(mine[at] - theirs[at]), abs(mine[at + 1] - theirs[at + 1]), abs(mine[at + 2] - theirs[at + 2]))
        value = min(255, worst * 4)
        picture[index * 3:index * 3 + 3] = bytes((value, value, value))
    raw = f"{diff_path}.rgb"
    Path(raw).write_bytes(bytes(picture))
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24",
                    "-s", f"{width}x{height}", "-i", raw, str(diff_path)], check=True)
    os.unlink(raw)
    return result


CAPTURES = [
    ("MainMenu, movie frame 288", ["--capture-frame", "288", "--menu-capture"], "main-menu-after-topbottom-version-frame288.bmp"),
    ("MainMenu, movie frame 301", ["--capture-frame", "301", "--menu-capture"], "main-menu-after-topbottom-version-frame301.bmp"),
    ("MainMenu, frame 288 against the C++'s 301", ["--capture-frame", "288", "--menu-capture"], "main-menu-after-topbottom-version-frame301.bmp"),
    ("Prompt: Continue", ["--capture-frame", "288", "--message-capture", "continue"], "message-continue-1920x1080.bmp"),
    ("Prompt: New", ["--capture-frame", "288", "--message-capture", "new"], "message-new-1920x1080.bmp"),
    ("Prompt: Exit", ["--capture-frame", "288", "--message-capture", "exit"], "message-exit-1920x1080.bmp"),
    ("LoadingMenu, held", ["--loading-capture"], "loading-1920.bmp"),
]


def command_g5(args):
    need_display()
    folder = OUT / "g5"
    shutil.rmtree(folder, ignore_errors=True)
    folder.mkdir(parents=True)
    binary = app_binary()
    results = {"commit": git("rev-parse", "HEAD"), "dirty": bool(git("status", "--porcelain")), "captures": []}
    cpp = Path(args.cpp_captures)
    # The C++ captures had a save, so Continue shows and the row is the wide layout. A scratch folder with one short save: never the user's.
    scratch = folder / "config"
    shutil.copytree(FIXTURES / "config", scratch)
    (scratch / "Saves").mkdir()
    (scratch / "Saves" / "Save 1.ess").write_bytes(b"TES4SAVEGAME")
    for name, flags, reference in CAPTURES:
        stem = re.sub(r"\W+", "-", name).strip("-").lower()
        ours = folder / f"{stem}.png"
        # No --seed: it fixes GetTickCount, and the menu's timed fade would never settle.
        command = [binary, "--ini", str(scratch / "Oblivion.ini"), "--no-audio-device"]
        if flags[-1] == "--loading-capture":
            command += ["--loading-capture", str(ours), str(args.loading_steps)]
        else:
            command += flags + [str(ours)]
        done = subprocess.run(command, cwd=ROOT, env=environment(), capture_output=True, text=True, errors="replace", timeout=600)
        entry = {"name": name, "reference": reference, "exit": done.returncode,
                 "said": [line for line in done.stdout.splitlines() if "capturing" in line or "capture" in line][:3]}
        if ours.exists() and done.returncode == 0:
            entry.update(compare_images(ours, cpp / reference, folder / f"{stem}-diff.png"))
            entry["movie area"] = entry.get("movie area")
        results["captures"].append(entry)
        print(json.dumps(entry)[:400], flush=True)
    (OUT / "g5.json").write_text(json.dumps(results, indent=1))


# ---- G6 ----


FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:default\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+\"[^\"]*\"\s+)?fn\s+(\w+)")


def fn_shape(lines, index):
    """What holds a `fn`: a trait impl, a trait's declaration, an inherent impl, nothing."""
    indent = len(lines[index]) - len(lines[index].lstrip())
    if indent == 0:
        return "free"
    for at in range(index - 1, -1, -1):
        line = lines[at]
        if line.strip() and len(line) - len(line.lstrip()) < indent:
            if re.match(r"\s*(unsafe )?impl.* for ", line):
                return "trait impl"
            if re.match(r"\s*(unsafe )?impl", line):
                return "inherent impl"
            if re.match(r"\s*(pub(\([^)]*\))? )?(unsafe )?trait", line):
                return "trait declaration"
            return "nested"
    return "free"


def scan_labels():
    """Each `fn` in the Rust sources: does a comment directly above it, past attributes, carry `IDA:`?"""
    summary = {}
    missing = []
    files = list(ROOT.glob("src/**/*.rs")) + list(ROOT.glob("tests/**/*.rs")) + list(ROOT.glob("crates/**/*.rs"))
    for path in sorted(files):
        relative = path.relative_to(ROOT)
        if str(relative).startswith("target"):
            continue
        parts = relative.parts
        crate = parts[1] if parts[0] == "crates" else "app"
        lines = path.read_text(errors="replace").splitlines()
        # `mod tests` blocks and test targets count apart: the rule names ported and host-only items.
        test_from = next((index for index, line in enumerate(lines)
                          if re.match(r"\s*#\[cfg\(test\)\]", line)), len(lines))
        whole_test = "tests" in parts or path.name.endswith("_tests.rs")
        banner = any(line.startswith("//!") and "IDA:" in line for line in lines[:12])
        for index, line in enumerate(lines):
            match = FN.match(line)
            if not match:
                continue
            at = index - 1
            labelled = False
            is_test = whole_test or index > test_from
            test_attribute = False
            while at >= 0:
                text = lines[at].strip()
                if text.startswith("#[") or text.startswith("#!") or text.endswith(")]") or text == ")]" or text.startswith(('reason =', 'clippy::')) or text in {"unused_imports,", "dead_code,"} or re.fullmatch(r"[\w:, ]+,?", text) and lines[at].startswith(" " * 8):
                    test_attribute |= text.startswith("#[test")
                    at -= 1
                    continue
                if text.startswith("//"):
                    labelled |= "IDA:" in text
                    at -= 1
                    continue
                break
            if test_attribute:
                bucket = "#[test] fns"
            elif is_test:
                bucket = "test helpers"
            else:
                bucket = "source"
            key = (crate, bucket)
            entry = summary.setdefault(key, [0, 0, 0])
            entry[0] += 1
            if labelled:
                entry[1] += 1
            else:
                entry[2] += 1
                missing.append({"file": str(relative), "line": index + 1, "fn": match.group(1), "bucket": bucket,
                                "banner": banner, "shape": fn_shape(lines, index)})
    return summary, missing


# ---- The report ----


def load_jsonl(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines()]


def parse_ranges(text):
    found = set()
    for part in text.split(","):
        if part:
            first, last = part.split("-")
            found.update(range(int(first), int(last) + 1))
    return found


def relaxed_match(entry, row, field=""):
    """Does a ledger `RELAXED` name this row, or this field of it? `what` is the row's leading words, then `#field` or nothing."""
    what = entry["fields"][1]
    head, _, key = what.partition(" #")
    if not row.startswith(head):
        return False
    # `roll-moved` is a tolerance over a whole CHG row: any token of it.
    return not key or not field or key == field or key == "roll-moved"


def table(rows, headers):
    widths = [max(len(str(row[index])) for row in rows + [headers]) for index in range(len(headers))]
    print(" | ".join(str(header).ljust(width) for header, width in zip(headers, widths)))
    print("-+-".join("-" * width for width in widths))
    for row in rows:
        print(" | ".join(str(value).ljust(width) for value, width in zip(row, widths)))


def command_report(args):
    if not (OUT / "run.json").exists():
        sys.exit("nothing to report: `side_by_side.py run` first")
    run = json.loads((OUT / "run.json").read_text())
    head = git("rev-parse", "HEAD")
    print(f"commit {run['commit'][:10]}{' + uncommitted changes' if run['dirty'] else ''}"
          f"{'' if run['commit'] == head else f' (HEAD is now {head[:10]})'}")
    print("Host tests only: no validation bonus on the 110% scale. Strict closure and bounded coverage are kept apart; no percentages.")
    failed_tests = [test for test in run["tests"] if test["status"] == "FAILED"]
    failed_binaries = {name: info for name, info in run["binaries"].items() if info["exit"] != 0}
    counts = {}
    for test in run["tests"]:
        counts[test["status"]] = counts.get(test["status"], 0) + 1
    print(f"tests, all binaries: {counts}; binaries with a nonzero exit: {list(failed_binaries)}")

    ledger = run["ledger"]
    relaxed = [entry for entry in ledger if entry["kind"] == "RELAXED"]
    deferred = [entry for entry in ledger if entry["kind"] == "DEFERRED"]
    mutants_rows = load_jsonl(OUT / "mutate-rows.jsonl")
    mutants_fields = load_jsonl(OUT / "mutate-fields.jsonl")
    manifest = json.loads((FIXTURES / "manifest.json").read_text())
    status_x64 = baseline_status("LastTest-x64.log")
    status_x86 = baseline_status("LastTest-x86.log")
    by_test = {(test["binary"], test["test"]): test["status"] for test in run["tests"]}
    loaders = {}
    for entry in ledger:
        if entry["kind"] == "LOAD":
            loaders.setdefault(entry["fields"][0], set()).add((entry["binary"], entry["test"]))
    g3_x64 = run["g3"]["by_log"]["LastTest-x64.log"]

    # ---- G1 ----
    print("\n== G1: rows, phases 04 to 16 ==")
    print("rows: data lines of the file. tracker: lines a row tracker marked read, across all tests (- = the file has no tracker; 'read' doesn't prove a compare).")
    print("row pass: rows changed / a port compare noticed / only an evidence check or lookup noticed / nothing noticed. This is a sample where it says so.")
    print("relaxed: rows and fields a test declared set aside. no reason: those with an empty reason.")
    rows = []
    unread_rows = {}
    any_uncaught = []
    for name in row_files():
        lines = read_rows(name)
        data = [index for index, line in enumerate(lines) if is_data_row(line)]
        reads = [entry for entry in ledger if entry["kind"] == "READ" and entry["fields"][0] == name]
        read = set()
        for entry in reads:
            read |= parse_ranges(entry["fields"][2])
        tracker = "-" if not reads else f"{len(read & set(data))}"
        if reads:
            unread_rows[name] = [index for index in data if index not in read]
        mine = [entry for entry in mutants_rows if entry["file"] == name]
        tally = {}
        for entry in mine:
            tally[entry["status"]] = tally.get(entry["status"], 0) + 1
        uncaught = [entry for entry in mine if entry["status"] == "uncaught"]
        any_uncaught.extend(uncaught)
        relaxed_here = [entry for entry in relaxed if entry["fields"][0] == name]
        unique = {(entry["fields"][1], entry["fields"][3]) for entry in relaxed_here}
        no_reason = {what for what, reason in unique if not reason.strip()}
        tests = loaders.get(name, set())
        bad = [key for key in tests if by_test.get(key) == "FAILED"]
        sampled = "" if len(mine) == len(data) else " (sample)"
        rows.append([
            phase_of(name), name, len(data), "yes" if f"rows/{name}.rows" in manifest["files"] else "no",
            tracker,
            f"{len(mine)}{sampled} / {tally.get('compared', 0)} / {tally.get('evidence-only', 0) + tally.get('structural', 0)} / {len(uncaught)}" if mine else "not run",
            len(unique), len(no_reason),
            f"{len(tests)} ({len(bad)} failed)",
        ])
    table(rows, ["phase", "file", "rows", "pinned", "tracker", "row pass", "relaxed", "no reason", "tests loading it"])

    missed, recorded_line = recorded_editor_ids()
    port_ids = field_value(run["g3"]["lines"], "editor IDs")
    id_miss = str(missed) != port_ids
    print(f"\neditor IDs: `{recorded_line}` is {missed}; the port's Phase-12 line says {port_ids}: {'FAIL, a miss' if id_miss else 'match'}.")
    print("  x64 log:", field_value(baseline_lines("LastTest-x64.log"), "editor IDs"),
          " x86 log:", field_value(baseline_lines("LastTest-x86.log"), "editor IDs"))
    if deferred:
        print("deferred groups:", [(entry["fields"][0], entry["fields"][1], entry["fields"][2], entry["fields"][3]) for entry in deferred])
    else:
        print("deferred groups: none (every DEFERRED table is empty)")
    never = {name: indexes for name, indexes in unread_rows.items() if indexes}
    for name, indexes in never.items():
        print(f"tracker: {name}: {len(indexes)} lines no tracker read, e.g. {[read_rows(name)[index][:60] for index in indexes[:4]]}")

    print("\nRelaxed, with their reasons (from the tests' ledger):")
    grouped = {}
    for entry in relaxed:
        grouped.setdefault((entry["fields"][0], entry["fields"][3]), set()).add(entry["fields"][1])
    for (name, reason), members in sorted(grouped.items()):
        shown = sorted(members)
        print(f"  {name}: {len(members)} rows or fields, e.g. {shown[:3]}: {reason or '*** NO RECORDED REASON ***'}")

    print("\nRows nothing noticed changing (row pass), and whether a test declared them:")
    undeclared = []
    for entry in any_uncaught:
        row = read_rows(entry["file"])[entry["line"]]
        hits = [item for item in relaxed if item["fields"][0] == entry["file"] and relaxed_match(item, row)]
        reasons = {item["fields"][3] for item in hits if item["fields"][3].strip()}
        if not reasons:
            undeclared.append(entry)
        print(f"  {entry['file']}:{entry['line'] + 1}: {row[:90]}: {sorted(reasons)[0][:70] if reasons else '*** NOT DECLARED ***'}")

    field_uncaught = [entry for entry in mutants_fields if entry["status"] == "uncaught"]
    field_status = {}
    for entry in mutants_fields:
        field_status[entry["status"]] = field_status.get(entry["status"], 0) + 1
    print(f"\nField pass (bounded: a few rows of each group, one value at a time): {len(mutants_fields)} values changed: {field_status}")
    undeclared_fields = {}
    for entry in field_uncaught:
        row = read_rows(entry["file"])[entry["line"]]
        hits = [item for item in relaxed if item["fields"][0] == entry["file"] and relaxed_match(item, row, entry["field"])]
        reasons = {item["fields"][3] for item in hits if item["fields"][3].strip()}
        key = (entry["file"], group_key(row), entry["field"])
        if reasons:
            continue
        undeclared_fields.setdefault(key, 0)
        undeclared_fields[key] += 1
    if undeclared_fields:
        print("  Values nothing noticed changing, not declared:")
        for (name, group, field), count in sorted(undeclared_fields.items()):
            print(f"    {name}: {group}: {field} ({count})")

    g1_fail = bool(failed_tests) or id_miss or bool(undeclared) or bool(undeclared_fields) or any(not entry["fields"][3].strip() for entry in relaxed)
    print(f"\nG1: {'FAIL' if g1_fail else 'PASS'}"
          f" (tests failed {len(failed_tests)}, editor-ID miss {id_miss}, undeclared unnoticed rows {len(undeclared)}, undeclared unnoticed fields {len(undeclared_fields)})")

    # ---- G2 ----
    print("\n== G2: Phase 16's end state, three paths ==")
    want = {"run_001_the_default_path_whole": "P16-RUN-001 default", "run_002_an_interior_cell_whole": "P16-RUN-002 XPAichan01",
            "run_003_an_exterior_cell_whole": "P16-RUN-003 X 4 / Y 7",
            "the_rows_against_the_53_bit_forms": "rows tell 24-bit from 53-bit",
            "the_interior_path_reaches_the_cell_arm": "driver, interior line", "the_coordinates_path_reaches_the_cell_arm": "driver, exterior line",
            "the_driver_ends_where_the_harness_ends": "driver World against the harness and the rows, three paths"}
    g2_rows = []
    g2_ok = True
    for test, what in want.items():
        status = [value for (binary, name), value in by_test.items() if name == test]
        ok = status and all(value == "ok" for value in status)
        g2_ok &= bool(ok)
        g2_rows.append([test, what, ",".join(status) or "not found"])
    table(g2_rows, ["test", "what", "result"])
    print(f"G2: {'PASS' if g2_ok else 'FAIL'}")

    # ---- G3 ----
    print("\n== G3: the app's Phase-NN lines against fixtures/cpp-baseline ==")
    g3_ok = run["g3"]["exit"] == 0
    for log, lines in run["g3"]["by_log"].items():
        bad = [line for line in lines if not line["match"]]
        g3_ok &= not bad
        exact = sum(1 for line in lines if line["exact"])
        print(f"{log}: {len(lines)} lines, {exact} exact, {len(lines) - exact - len(bad)} match with tolerances, {len(bad)} differ")
        for line in bad:
            print(f"  ours: {line['line']}\n  C++ : {line['cpp']}")
    rows = []
    for index, line in enumerate(g3_x64):
        x86 = run["g3"]["by_log"]["LastTest-x86.log"][index]
        phase = line["line"].split(":")[0]
        rows.append([phase, "exact" if line["exact"] else "tolerated" if line["match"] else "DIFFERS",
                     "exact" if x86["exact"] else "tolerated" if x86["match"] else "DIFFERS",
                     "; ".join(f"{key}: ours {ours}, x64 {cpp}" for key, ours, cpp in line["tolerated"])])
    table(rows, ["line", "x64", "x86", "tolerated (x64)"])
    print("\nEach tolerance, its values across the seeds this run tried, and the logs:")
    seed_lines = run["g3"]["seeds"]
    for key, reason in TOLERANCES.items():
        values = {seed: field_value(lines, key) for seed, lines in seed_lines.items()}
        spread = sorted(set(values.values()), key=str)
        print(f"  {key}: seeds {spread}; x64 {field_value(baseline_lines('LastTest-x64.log'), key)}, x86 {field_value(baseline_lines('LastTest-x86.log'), key)}")
        print(f"    {reason}")
    moved = {}
    first = next(iter(seed_lines.values()), [])
    for lines in seed_lines.values():
        for index, line in enumerate(lines):
            if line != first[index]:
                mine = tolerated(line)[0]
                if mine != tolerated(first[index])[0]:
                    moved[index] = line
    print(f"  fields outside the tolerances that moved with a seed: {list(moved.values()) or 'none'}")
    print(f"G3: {'PASS, with the tolerances above' if g3_ok else 'FAIL'}")

    # ---- G4 ----
    print("\n== G4: boundaries (tools/boundary_audit.py --check; the audit's own run) ==")
    print(run["g4"]["stdout"].strip())
    print(f"G4: {'PASS' if run['g4']['exit'] == 0 else 'FAIL'} (`boundary_audit.py --run-tests --check`, exit {run['g4']['exit']}, in this run)")

    # ---- G5 ----
    print("\n== G5: pictures ==")
    print(f"G5: {G5_DECISION}")
    print("The C++ repo's out/captures, for comparison only:")
    if (OUT / "g5.json").exists():
        g5 = json.loads((OUT / "g5.json").read_text())
        print(f"commit {g5['commit'][:10]}{' + uncommitted changes' if g5['dirty'] else ''}")
        rows = []
        for entry in g5["captures"]:
            whole = entry.get("whole")
            area = entry.get("movie area")
            rows.append([entry["name"], entry["reference"], entry["exit"],
                         f"{whole['over_2']} / {whole['over_16']} / {whole['over_64']} of {whole['pixels']}" if whole else entry.get("error", "no picture"),
                         whole["mean_worst_channel"] if whole else "", whole["peak"] if whole else "",
                         f"{area['over_2']} of {area['pixels']}, peak {area['peak']}" if area else ""])
        table(rows, ["screen", "C++ capture", "exit", "pixels off by >2 / >16 / >64", "mean", "peak", "movie area off by >2"])
        print(f"pictures and differences in {OUT / 'g5'}")
    else:
        print("not run: `side_by_side.py g5 --cpp-captures DIR`")

    # ---- G6 ----
    print("\n== G6: format, lint, labels ==")
    env = environment()
    env["CARGO_TARGET_DIR"] = str(TARGET)
    fmt = subprocess.run(["cargo", "fmt", "--all", "--check"], cwd=ROOT, env=env, capture_output=True, text=True)
    print(f"cargo fmt --check: exit {fmt.returncode}")
    if args.clippy:
        clippy = subprocess.run(["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"],
                                cwd=ROOT, env=env, capture_output=True, text=True)
        print(f"cargo clippy --workspace --all-targets -D warnings: exit {clippy.returncode}")
        print(clippy.stderr[-1500:] if clippy.returncode else "", end="")
    else:
        clippy = None
        print("cargo clippy: not run here; pass --clippy")
    summary, missing = scan_labels()
    rows = [[crate, bucket, *values] for (crate, bucket), values in sorted(summary.items())]
    table(rows, ["crate", "where", "fns", "labelled", "no adjacent IDA:"])
    source_missing = [entry for entry in missing if entry["bucket"] == "source"]
    print(f"source fns with no adjacent IDA: label: {len(source_missing)}; test helpers {sum(1 for entry in missing if entry['bucket'] == 'test helpers')}; #[test] fns {sum(1 for entry in missing if entry['bucket'] == '#[test] fns')}")
    shapes = {}
    for entry in source_missing:
        shapes[entry["shape"]] = shapes.get(entry["shape"], 0) + 1
    print(f"  of the source ones, by what holds them: {shapes}; in a file with a `//! IDA:` banner: {sum(1 for entry in source_missing if entry['banner'])}")
    (OUT / "labels-missing.json").write_text(json.dumps(missing, indent=0))
    print(f"the list: {OUT / 'labels-missing.json'}")
    ok = fmt.returncode == 0 and (clippy is None or clippy.returncode == 0) and not source_missing
    print(f"G6: {'PASS' if ok else 'FAIL'}{' (clippy not run)' if clippy is None else ''}")

    # ---- The C++ column, per phase ----
    print("\n== Per phase: C++ on record, the port now ==")
    rows = []
    # Files past the roadmap, such as `newgame`, come after the numbered phases.
    phases = sorted({phase_of(name) for name in row_files()}, key=lambda text: (0, int(text[:2]), text[2:]) if text[:2].isdigit() else (1, 0, text))
    lines = run["g3"]["lines"]
    for phase in phases:
        test = f"startup_phase{phase[:2]}"
        mine = [line for line in lines if line.startswith(f"Phase-{phase}:")]
        x64 = [line for line in g3_x64 if line["line"].startswith(f"Phase-{phase}:")]
        files = [name for name in row_files() if phase_of(name) == phase]
        tests = set()
        for name in files:
            tests |= loaders.get(name, set())
        bad = sum(1 for key in tests if by_test.get(key) == "FAILED")
        relax = {(entry["fields"][1], entry["fields"][3]) for entry in relaxed if entry["fields"][0] in files}
        rows.append([phase, sum(len([1 for line in read_rows(name) if is_data_row(line)]) for name in files),
                     f"x64 {status_x64.get(test, '?')}, x86 {status_x86.get(test, '?')}",
                     f"{len(tests) - bad} of {len(tests)} tests ok",
                     "exact" if x64 and all(line["exact"] for line in x64) else "tolerated" if x64 and all(line["match"] for line in x64) else ("no line" if not x64 else "DIFFERS"),
                     len(relax)])
    table(rows, ["phase", "rows", "C++ recorded", "Bevy tests", "Bevy Phase line vs C++", "tolerated rows and fields"])


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("run")
    mutate = sub.add_parser("mutate")
    mutate.add_argument("--mode", choices=["rows", "fields"], default="rows")
    # A new-game mutant runs the whole game to the scripts' stop, 6.2 GB at peak (2026-11-07): 3 stay under the 24 GB cap.
    mutate.add_argument("--jobs", type=int, default=3)
    mutate.add_argument("--per-group", type=int, default=12)
    mutate.add_argument("--whole-under", type=int, default=700)
    mutate.add_argument("--fields-per-group", type=int, default=2)
    mutate.add_argument("--files", nargs="*")
    mutate.add_argument("--fresh", action="store_true")
    g5 = sub.add_parser("g5")
    g5.add_argument("--cpp-captures", required=True)
    g5.add_argument("--loading-steps", type=int, default=0)
    report = sub.add_parser("report")
    report.add_argument("--clippy", action="store_true")
    args = parser.parse_args()
    {"run": command_run, "mutate": command_mutate, "g5": command_g5, "report": command_report}[args.command](args)


if __name__ == "__main__":
    main()
