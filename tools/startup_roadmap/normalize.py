"""Normalize the preserved, read-only IDA call/CFG captures. No IDA connection."""
import argparse
import ast
import hashlib
import json
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("evidence", type=Path)
args = parser.parse_args()
functions = {}
sources = []
for path in sorted(args.evidence.glob("capture*.json")):
    raw = path.read_bytes()
    capture = json.loads(raw.decode("utf-8"))
    for function in ast.literal_eval(capture["stdout"]):
        assert "error" not in function, function
        entry = function["entry"]
        assert entry not in functions, hex(entry)
        function["calls"].sort(key=lambda call: call["site"])
        functions[entry] = function
    sources.append({"file": path.name, "sha256": hashlib.sha256(raw).hexdigest()})
manifest = {
    "binary_sha256": "0244dd598004ca2f379635e3bf5c583e8b3ced852f86d884767911fb6c48788b",
    "capture_date": "2026-09-24",
    "root": 0x98787C,
    "startup_idle_announcement_callsite": 0x40F0E5,
    "uncaptured_idle_continuation": 0x40F16A,
    "scope": "PE CRT entry through pre-menu startup, MainMenu/alternate branch, idle announcement and first message pump; uncaptured continuation begins at 0040F16A. Expanded dependencies include alternate paths. Not a complete transitive engine call graph.",
    "sources": sources,
    "functions": list(functions.values()),
}
output = Path(__file__).with_name("manifest.json")
temporary = output.with_suffix(".tmp")
temporary.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
temporary.replace(output)
print(f"Captured {len(functions)} function bodies, {sum(len(f['calls']) for f in functions.values())} call sites")
