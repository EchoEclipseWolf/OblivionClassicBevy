#!/usr/bin/env python3
"""Import original-run evidence and the C++ baseline into fixtures/ with provenance.

  import_fixtures.py           write fixtures/ and fixtures/manifest.json
  import_fixtures.py --check   verify fixtures and their sources against the manifest

Sources are read-only. `.inc` files become plain rows, one per line, exactly the strings the
C++ tests compare against. Hashes ignore CR, because the C++ working tree has CRLF noise.
"""
import ast
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import lift_phase16_rows

ROOT = Path(__file__).resolve().parent.parent
ECHO = ROOT.parent
CPP = ECHO / "OblivionRecomp"
NOTES = ECHO / "OblivionBotInfos/Recomp/StartupLoading"
WINDOWS_PROFILE = Path(os.environ.get("OBLIVION_WINDOWS_PROFILE", "../windows-profile"))
OUT = ROOT / "fixtures"
INC_DIR = CPP / "tests/startup"

# Phases with no .inc: their values sit in the C++ tests, so keep the raw run for porting them.
EVIDENCE = [
    "phase04-run001-2026-10-05", "phase04-run002-2026-10-06",
    "phase05-run001-2026-10-06", "phase05-run002-2026-10-06",
    "phase06-run001-2026-10-06", "phase06-run002-2026-10-06",
    "phase08-run002-2026-10-06", "phase09-run001-2026-10-06", "phase10-run001-2026-10-06",
    "phase12-run001-2026-10-09", "phase13-run001-2026-10-09", "phase13-run002-2026-10-09", "phase13-run003-2026-10-09",
    "phase14-run001-2026-10-09",
    "phase14-run005-2026-10-09",
    "phase15-run001-2026-10-10", "phase15-run002-2026-10-10", "phase15-run003-2026-10-10", "phase15-run004-2026-10-10",
    "phase15-run004b-2026-10-10", "phase15-run005-2026-10-10", "phase15-run007-2026-10-10", "phase15-run009-2026-10-10",
    "phase16-run001-2026-10-10", "phase16-run002-2026-10-10", "phase16-run003-2026-10-10",
]
# Files beside a run's records.txt that a phase test compares with: output name, then the path under evidence/. A text mode
# file keeps its CR LF bytes in the fixture, since they are what the test compares.
EXTRA_EVIDENCE = {
    "phase04-run001-attempt3-2026-10-05.records.txt": "phase04-run001-2026-10-05/records-attempt3.txt",
    "phase04-run001-2026-10-05.RendererInfo.txt": "phase04-run001-2026-10-05/RendererInfo-run001.txt",
    # The P07-RUN-001 dump behind phase07.rows and phase07_window.rows; its folder holds no records.txt.
    "phase07-run001-2026-10-06.records.txt": "phase07-window-2026-10-06/run001-records.txt",
    # The memory snapshots behind the SCENE rows of phase05.rows.
    "phase05-run002-2026-10-06.snap-camera-world.hex": "phase05-run002-2026-10-06/snap-camera-world.hex",
    "phase05-run002-2026-10-06.snap-camera-menu.hex": "phase05-run002-2026-10-06/snap-camera-menu.hex",
    "phase05-run002-2026-10-06.snap-camera-menu3d.hex": "phase05-run002-2026-10-06/snap-camera-menu3d.hex",
    "phase05-run002-2026-10-06.snap-shadowNode.hex": "phase05-run002-2026-10-06/snap-shadowNode.hex",
    "phase05-run002-2026-10-06.snap-accumulator-head.hex": "phase05-run002-2026-10-06/snap-accumulator-head.hex",
    "phase05-run002-2026-10-06.snap-accumulator-tail.hex": "phase05-run002-2026-10-06/snap-accumulator-tail.hex",
    "phase05-run002-2026-10-06.snap-globals-B42C00.hex": "phase05-run002-2026-10-06/snap-globals-B42C00.hex",
    "phase05-run001-2026-10-06.snap-scene-world.hex": "phase05-run001-2026-10-06/snap-scene-world.hex",
    "phase05-run001-2026-10-06.snap-scene-menu.hex": "phase05-run001-2026-10-06/snap-scene-menu.hex",
    "phase05-run001-2026-10-06.snap-scene-menu3d.hex": "phase05-run001-2026-10-06/snap-scene-menu3d.hex",
    "phase05-run001-2026-10-06.snap-scene-sky.hex": "phase05-run001-2026-10-06/snap-scene-sky.hex",
    "phase05-run001-2026-10-06.snap-scene-weather.hex": "phase05-run001-2026-10-06/snap-scene-weather.hex",
    "phase05-run001-2026-10-06.snap-scene-lodRoot.hex": "phase05-run001-2026-10-06/snap-scene-lodRoot.hex",
    "phase05-run001-2026-10-06.snap-scene-lodTrees.hex": "phase05-run001-2026-10-06/snap-scene-lodTrees.hex",
    "phase05-run001-2026-10-06.snap-scene-objectLOD.hex": "phase05-run001-2026-10-06/snap-scene-objectLOD.hex",
    "phase05-run001-2026-10-06.snap-scene-magicProjectile.hex": "phase05-run001-2026-10-06/snap-scene-magicProjectile.hex",
    "phase05-run001-2026-10-06.snap-scene-fog.hex": "phase05-run001-2026-10-06/snap-scene-fog.hex",
    "phase05-run001-2026-10-06.snap-scene-wireframe.hex": "phase05-run001-2026-10-06/snap-scene-wireframe.hex",
    "phase05-run001-2026-10-06.snap-slot01-shader.hex": "phase05-run001-2026-10-06/snap-slot01-shader.hex",
    "phase05-run001-2026-10-06.snap-slot02-shader.hex": "phase05-run001-2026-10-06/snap-slot02-shader.hex",
    "phase05-run001-2026-10-06.snap-slot03-shader.hex": "phase05-run001-2026-10-06/snap-slot03-shader.hex",
    "phase05-run001-2026-10-06.snap-slot04-shader.hex": "phase05-run001-2026-10-06/snap-slot04-shader.hex",
    "phase05-run001-2026-10-06.snap-slot05-shader.hex": "phase05-run001-2026-10-06/snap-slot05-shader.hex",
    "phase05-run001-2026-10-06.snap-slot06-shader.hex": "phase05-run001-2026-10-06/snap-slot06-shader.hex",
    "phase05-run001-2026-10-06.snap-slot07-shader.hex": "phase05-run001-2026-10-06/snap-slot07-shader.hex",
    "phase05-run001-2026-10-06.snap-slot08-shader.hex": "phase05-run001-2026-10-06/snap-slot08-shader.hex",
    "phase05-run001-2026-10-06.snap-slot09-shader.hex": "phase05-run001-2026-10-06/snap-slot09-shader.hex",
    "phase05-run001-2026-10-06.snap-slot10-shader.hex": "phase05-run001-2026-10-06/snap-slot10-shader.hex",
    "phase05-run001-2026-10-06.snap-slot11-shader.hex": "phase05-run001-2026-10-06/snap-slot11-shader.hex",
    "phase05-run001-2026-10-06.snap-slot12-shader.hex": "phase05-run001-2026-10-06/snap-slot12-shader.hex",
    "phase05-run001-2026-10-06.snap-slot13-shader.hex": "phase05-run001-2026-10-06/snap-slot13-shader.hex",
    "phase05-run001-2026-10-06.snap-slot14-shader.hex": "phase05-run001-2026-10-06/snap-slot14-shader.hex",
    "phase05-run001-2026-10-06.snap-slot15-shader.hex": "phase05-run001-2026-10-06/snap-slot15-shader.hex",
    "phase05-run001-2026-10-06.snap-slot16-shader.hex": "phase05-run001-2026-10-06/snap-slot16-shader.hex",
    "phase05-run001-2026-10-06.snap-slot17-shader.hex": "phase05-run001-2026-10-06/snap-slot17-shader.hex",
    "phase05-run001-2026-10-06.snap-slot18-shader.hex": "phase05-run001-2026-10-06/snap-slot18-shader.hex",
    "phase05-run001-2026-10-06.snap-slot19-shader.hex": "phase05-run001-2026-10-06/snap-slot19-shader.hex",
    "phase05-run001-2026-10-06.snap-slot20-shader.hex": "phase05-run001-2026-10-06/snap-slot20-shader.hex",
    "phase05-run001-2026-10-06.snap-slot21-shader.hex": "phase05-run001-2026-10-06/snap-slot21-shader.hex",
    "phase05-run001-2026-10-06.snap-slot22-shader.hex": "phase05-run001-2026-10-06/snap-slot22-shader.hex",
    "phase05-run001-2026-10-06.snap-slot23-shader.hex": "phase05-run001-2026-10-06/snap-slot23-shader.hex",
    "phase05-run001-2026-10-06.snap-slot24-shader.hex": "phase05-run001-2026-10-06/snap-slot24-shader.hex",
    "phase05-run001-2026-10-06.snap-slot25-shader.hex": "phase05-run001-2026-10-06/snap-slot25-shader.hex",
    "phase05-run001-2026-10-06.snap-slot26-shader.hex": "phase05-run001-2026-10-06/snap-slot26-shader.hex",
    "phase05-run001-2026-10-06.snap-slot27-shader.hex": "phase05-run001-2026-10-06/snap-slot27-shader.hex",
}
BASELINE = {
    "LastTest-x64.log": CPP / "out/build/vs2026-x64/Testing/Temporary/LastTest.log",
    "LastTest-x86.log": CPP / "out/build/vs2026-x86/Testing/Temporary/LastTest.log",
}
# The files the evidence runs used, not the Wine prefix's copies.
CONFIG = {
    "Oblivion.ini": (WINDOWS_PROFILE / "OneDrive/Documents/My Games/Oblivion/Oblivion.ini",
                     "d243541f17453f0f86db767633b96690ba6f94ee4e501c2b636e69e3a06c0fe3"),
    "Plugins.txt": (WINDOWS_PROFILE / "AppData/Local/Oblivion/Plugins.txt",
                    "f2e65b7cad007c1e686517ac4eec6238139df0d0fa50cb8dd21955639c3141d6"),
    "BlendSettings.ini": (WINDOWS_PROFILE / "OneDrive/Documents/My Games/Oblivion/BlendSettings.ini",
                          "4fa5e2568cf1e3b5eff585c345605d96ffa8af1be5f6ffc97163439c6db576b7"),
    "BlendSettings-version.ini": (WINDOWS_PROFILE / "AppData/Local/VirtualStore/Windows/BlendSettings.ini",
                                  "db2774c2861e151e06e4bc00088cf5cb12c1e721b657d2e2408a496f57c7566a"),
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def text_sha(path):
    return sha(path.read_bytes().replace(b"\r\n", b"\n"))


def is_table(inc):
    """phase07's .inc is a C++ initializer table, not string rows: keep its lines verbatim."""
    for line in inc.read_text(encoding="latin-1").splitlines():
        line = line.strip()
        if line and not line.startswith("//"):
            return line.startswith("{")
    return False


def rows_of(inc):
    if is_table(inc):
        return [line.strip() for line in inc.read_text(encoding="latin-1").splitlines()
                if line.strip() and not line.strip().startswith("//")]
    rows = []
    pending = []
    for number, line in enumerate(inc.read_text(encoding="latin-1").splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("//"):
            continue
        if not line.startswith('"') or not line.endswith(('"', '",')):
            raise SystemExit(f"{inc}:{number}: not a C string row")
        # A row may be adjacent literals across lines; the comma ends it.
        pending.append(line.removesuffix(","))
        if line.endswith(","):
            # .inc rows are ASCII C literals with only \\" and \\\\ escapes.
            value = ast.literal_eval(" ".join(pending))
            pending = []
            if "\n" in value or "\r" in value:
                raise SystemExit(f"{inc}:{number}: a row holds a line break")
            rows.append(value)
    if pending:
        raise SystemExit(f"{inc}: an unterminated row")
    return rows


def build():
    manifest = {"cpp_commit": subprocess.check_output(
        ["git", "-C", str(CPP), "rev-parse", "HEAD"], text=True).strip(), "files": {}}

    def record(out, source, source_hash):
        manifest["files"][str(out.relative_to(OUT))] = {
            "source": str(source), "source_sha256": source_hash,
            "sha256": sha(out.read_bytes())}

    for inc in sorted(INC_DIR.glob("phase*_expected.inc")):
        out = OUT / "rows" / (inc.name.replace("_expected.inc", "") + ".rows")
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_bytes(("\n".join(rows_of(inc)) + "\n").encode("latin-1"))
        record(out, inc, text_sha(inc))
    for run in EVIDENCE:
        source = NOTES / "evidence" / run / "records.txt"
        out = OUT / "evidence" / (run + ".records.txt")
        shutil.copyfile(source, out)
        record(out, source, text_sha(source))
    # Phase 16 has no .inc: its rows are lifted from the runs just copied.
    source = NOTES / "evidence" / "phase16-run002-2026-10-10" / "records.txt"
    record(lift_phase16_rows.lift(), source, text_sha(source))
    # Phase 17's rows come from a memory snapshot, by tools/original_debug/phase17_rows.py, not from a records.txt.
    source = NOTES / "evidence" / "phase17-run001-2026-11-06" / "phase17.rows"
    out = OUT / "rows" / "phase17.rows"
    shutil.copyfile(source, out)
    record(out, source, text_sha(source))
    # Phase 18's likewise, by tools/original_debug/phase18_rows.py.
    source = NOTES / "evidence" / "phase18-run001-2026-11-06" / "phase18.rows"
    out = OUT / "rows" / "phase18.rows"
    shutil.copyfile(source, out)
    record(out, source, text_sha(source))
    # Phase 19's likewise, by tools/original_debug/phase19_rows.py.
    source = NOTES / "evidence" / "phase19-run001-2026-11-07" / "phase19.rows"
    out = OUT / "rows" / "phase19.rows"
    shutil.copyfile(source, out)
    record(out, source, text_sha(source))
    # Phase 20's likewise, by tools/original_debug/phase20_rows.py.
    source = NOTES / "evidence" / "phase20-run001-2026-11-07" / "phase20.rows"
    out = OUT / "rows" / "phase20.rows"
    shutil.copyfile(source, out)
    record(out, source, text_sha(source))
    # Phase 21's likewise, by tools/original_debug/phase21_rows.py.
    source = NOTES / "evidence" / "phase21-run001-2026-11-07" / "phase21.rows"
    out = OUT / "rows" / "phase21.rows"
    shutil.copyfile(source, out)
    record(out, source, text_sha(source))
    # Phase 22's likewise, by tools/original_debug/phase22_rows.py.
    source = NOTES / "evidence" / "phase22-run001-2026-11-07" / "phase22.rows"
    out = OUT / "rows" / "phase22.rows"
    shutil.copyfile(source, out)
    record(out, source, text_sha(source))
    # The New Game confirmation's likewise, by tools/original_debug/newgame_rows.py.
    source = NOTES / "evidence" / "gameplay-run001-2026-11-07" / "newgame.rows"
    out = OUT / "rows" / "newgame.rows"
    shutil.copyfile(source, out)
    record(out, source, text_sha(source))
    # The new game's commands likewise, by tools/original_debug/newgame_commands_rows.py.
    source = NOTES / "evidence" / "gameplay-run001-2026-11-07" / "newgame_commands.rows"
    out = OUT / "rows" / "newgame_commands.rows"
    shutil.copyfile(source, out)
    record(out, source, text_sha(source))
    # The prison's references with 3D in run-af's after_move, by tools/original_debug/after_move_nodes_rows.py.
    source = NOTES / "evidence" / "gameplay-run002-2026-11-07" / "after_move_nodes.rows"
    out = OUT / "rows" / "after_move_nodes.rows"
    shutil.copyfile(source, out)
    record(out, source, text_sha(source))
    # The prison NPCs' skin info slots in run-af's after_move, by tools/original_debug/skin_slots_rows.py.
    source = NOTES / "evidence" / "gameplay-run002-2026-11-07" / "skin_slots.rows"
    out = OUT / "rows" / "skin_slots.rows"
    shutil.copyfile(source, out)
    record(out, source, text_sha(source))
    # The prison NPCs' FaceGen heads in run-af's after_move, by tools/original_debug/facegen_nodes_rows.py (FaceGen agent).
    source = NOTES / "evidence" / "gameplay-run002-2026-11-07" / "facegen_nodes.rows"
    out = OUT / "rows" / "facegen_nodes.rows"
    shutil.copyfile(source, out)
    record(out, source, text_sha(source))
    for name, relative in EXTRA_EVIDENCE.items():
        source = NOTES / "evidence" / relative
        out = OUT / "evidence" / name
        shutil.copyfile(source, out)
        record(out, source, text_sha(source))
    for name, source in BASELINE.items():
        out = OUT / "cpp-baseline" / name
        shutil.copyfile(source, out)
        record(out, source, text_sha(source))
    for name, (source, expected) in CONFIG.items():
        data = source.read_bytes()
        if sha(data) != expected:
            raise SystemExit(f"{source}: hash {sha(data)}, expected {expected}")
        out = OUT / "config" / name
        if out.exists():
            out.chmod(0o644)
        out.write_bytes(data)
        out.chmod(0o444)
        record(out, source, expected)
    # Every run's raw records stay in the notes; pin their hashes so drift is visible.
    manifest["all_run_records"] = {
        str(p.relative_to(NOTES)): text_sha(p)
        for p in sorted(NOTES.glob("evidence/phase*-run*/records.txt"))}
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=1, sort_keys=True) + "\n")
    print(f"imported {len(manifest['files'])} files, "
          f"pinned {len(manifest['all_run_records'])} run records")


def check():
    manifest = json.loads((OUT / "manifest.json").read_text())
    bad = []
    for name, entry in manifest["files"].items():
        path = OUT / name
        if not path.exists() or sha(path.read_bytes()) != entry["sha256"]:
            bad.append(f"fixture changed: {name}")
        source = Path(entry["source"])
        if source.exists():
            actual = sha(source.read_bytes()) if name.startswith("config/") else text_sha(source)
            if actual != entry["source_sha256"]:
                bad.append(f"source changed: {source}")
        else:
            bad.append(f"source missing: {source}")
    for name, value in manifest["all_run_records"].items():
        path = NOTES / name
        if not path.exists() or text_sha(path) != value:
            bad.append(f"run records changed: {name}")
    print("\n".join(bad) or f"ok: {len(manifest['files'])} fixtures match")
    return 1 if bad else 0


if __name__ == "__main__":
    if "--check" in sys.argv:
        sys.exit(check())
    build()
