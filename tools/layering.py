#!/usr/bin/env python3
"""Report production Rust source dependencies and check reviewed upward imports.

Run "graph" to inspect layer counts, or "graph --upward" for a candidate baseline.
"check" compares file / defining-module pairs. Review removals as well as additions.
This is a source graph: macro bodies are read as written, without macro expansion.

The gate ranks the layers: oblivion_core, oblivion_pathing, gamebryo, bs, the subject folders of tes/ together,
then tes/winmain. An edge is upward when it climbs a rank. The folders inside tes/ share one rank (user,
2026-11-11): the original has no order among them, and a function is filed with its owning class
(tools/placement.py) whichever tes/ folder that is. "graph" still prints every folder pair.
"""
import argparse
from collections import Counter, defaultdict
from dataclasses import dataclass, field
import json
from pathlib import Path
import sys

from rust_source import tokens

ROOT = Path(__file__).resolve().parents[1]
CRATES = ("oblivion_core", "oblivion_pathing", "oblivion_sim")
LAYERS = (
    "oblivion_core", "oblivion_pathing", "gamebryo", "bs", "tes/settings", "tes/forms",
    "tes/data", "tes/os", "tes/world", "tes/facegen", "tes/actors", "tes/ai",
    "tes/script", "tes/interface", "tes/menus", "tes/winmain",
)
BASELINE = ROOT / "tools/layering_baseline.json"


def rank(name):
    """The gate's rank of a layer: the tes/ subject folders share one, below tes/winmain."""
    if name.startswith("tes/"):
        return 5 if name == "tes/winmain" else 4
    return ("oblivion_core", "oblivion_pathing", "gamebryo", "bs").index(name)

PAIRS = {"{": "}", "[": "]", "(": ")"}


def use_tree(words, index, prefix=()):
    """Flatten a Rust use tree, retaining self imports, globs and local names."""
    parts = list(prefix)
    leaves = []
    while True:
        name = words[index]
        index += 1
        if name == "::":
            continue
        if name == "{":
            while words[index] != "}":
                branch, index = use_tree(words, index, parts)
                leaves.extend(branch)
                if words[index] == ",":
                    index += 1
            return leaves, index + 1
        if not (name == "self" and parts):
            parts.append(name)
        if words[index] != "::":
            break
    alias = parts[-1]
    if words[index] == "as":
        alias = words[index + 1]
        index += 2
    return [(tuple(parts), alias)], index


@dataclass(eq=False)
class Scope:
    module: tuple
    parent: 'Scope | None' = None
    bindings: dict = field(default_factory=dict)
    globs: list = field(default_factory=list)
    exports: set = field(default_factory=set)
    export_globs: list = field(default_factory=list)


class Sources:
    def __init__(self, root):
        self.root = root
        self.modules = {}
        self.file_layers = {}
        self.scopes = {}
        self.references = {}
        self.declarations = defaultdict(set)

    def read(self, module, path):
        self.file_layers[path] = layer(module)
        stream = list(tokens((self.root / path).read_text()))
        words = [w for w, _ in stream]
        ends = {}
        starts = {}
        stack = []
        for i, word in enumerate(words):
            if word in PAIRS:
                stack.append(i)
            elif word in PAIRS.values():
                start = stack.pop()
                if PAIRS[words[start]] != word:
                    raise ValueError(f"{path}:{stream[i][1]}: unbalanced Rust token tree")
                ends[start] = i
                starts[i] = start

        def scan(scope, start, stop):
            i = start
            path_attr = None
            while i < stop:
                word = words[i]
                line = stream[i][1]
                if word == "#" and words[i + 1] in ("!", "["):
                    inner = words[i + 1] == "!"
                    opening = i + 2 if inner else i + 1
                    if words[opening] != "[":
                        i += 1
                        continue
                    attr = words[opening + 1:ends[opening]]
                    i = ends[opening] + 1
                    if attr[:2] == ["path", "="]:
                        path_attr = json.loads(attr[2])
                    if attr == ["test"] or test_only(attr):
                        if inner:
                            return
                        i = after_item(words, ends, i)
                        path_attr = None
                    continue
                previous = i - 1
                if previous >= 0 and words[previous] == ")":
                    previous = starts[previous] - 1
                exported = scope.parent is None and previous >= 0 and words[previous] == "pub"
                if word == "mod" and words[i + 2] in (";", "{"):
                    name = words[i + 1]
                    child = scope.module + (name,)
                    self.declarations[scope.module].add(name)
                    if exported:
                        scope.exports.add(name)
                    if words[i + 2] == "{":
                        self.modules[child] = path
                        sub = self.scopes[child] = Scope(child)
                        scan(sub, i + 3, ends[i + 2])
                        i = ends[i + 2] + 1
                    else:
                        folder = Path("crates", child[0], "src", *child[1:-1])
                        if path_attr:
                            target = path.parent / path_attr
                        else:
                            target = folder / (name + ".rs")
                            if not (self.root / target).is_file():
                                target = folder / name / "mod.rs"
                        self.read(child, target)
                        i += 3
                    path_attr = None
                    continue
                if word == "use" and words[i + 1] != "<":  # use<T> is an impl Trait capture bound
                    imports, end = use_tree(words, i + 1)
                    if words[end] != ";":
                        raise ValueError(f"{path}:{line}: unsupported use tree")
                    for parts, alias in imports:
                        self.references.setdefault((path, scope, parts), line)
                        if alias == "*":
                            scope.globs.append(parts[:-1])
                            if exported:
                                scope.export_globs.append(parts[:-1])
                        elif alias != "_":
                            scope.bindings[alias] = parts
                            if exported:
                                scope.exports.add(alias)
                    i = end + 1
                    continue
                if word in ("struct", "enum", "type", "fn", "trait", "const", "static", "union"):
                    if scope.parent is None and i + 1 < stop:
                        self.declarations[scope.module].add(words[i + 1])
                if word == "{":
                    scan(Scope(scope.module, scope), i + 1, ends[i])
                    i = ends[i] + 1
                    continue
                if word.isidentifier() and words[i + 1:i + 2] == ["::"]:
                    parts = [word]
                    while words[i + 1:i + 2] == ["::"] and words[i + 2].isidentifier():
                        i += 2
                        parts.append(words[i])
                    self.references.setdefault((path, scope, tuple(parts)), line)
                i += 1

        self.modules[module] = path
        self.scopes[module] = Scope(module)
        scan(self.scopes[module], 0, len(words))


def test_only(attr):
    """Whether cfg is necessarily false in a production build (other flags unknown)."""
    def value(parts):
        if parts == ["test"]:
            return False
        if len(parts) < 3 or parts[1] != "(" or parts[-1] != ")":
            return None
        args, start, depth = [], 2, 0
        for i in range(2, len(parts)):
            if depth == 0 and parts[i] in (",", ")"):
                if i > start:
                    args.append(value(parts[start:i]))
                start = i + 1
            depth += (parts[i] == "(") - (parts[i] == ")")
        if parts[0] == "not" and len(args) == 1 and args[0] is not None:
            return not args[0]
        if parts[0] == "all":
            return False if False in args else (True if None not in args else None)
        if parts[0] == "any":
            return True if True in args else (False if None not in args else None)
        return None

    return attr[:2] == ["cfg", "("] and value(attr[2:-1]) is False


def after_item(words, ends, i):
    while words[i] == "#":
        i = ends[i + 1] + 1
    # Braces in grouped imports, const initializers or type expressions are
    # part of the item. Skip all the way to the terminating semicolon.
    if words[i] == "pub":
        i += 1
        if words[i] == "(":
            i = ends[i] + 1
    semicolon = words[i] in ("use", "static", "type") or (
        words[i] == "const" and words[i + 1] not in ("fn", "unsafe", "extern")
    )
    while words[i] != ";":
        if words[i] == "{" and not semicolon:
            return ends[i] + 1
        i = ends.get(i, i) + 1
    return i + 1


def layer(module):
    if module[0] != "oblivion_sim":
        return module[0]
    if len(module) > 2 and module[1] == "tes":
        return "tes/" + module[2]
    if len(module) > 1 and module[1] != "tes":
        return module[1]
    return None  # crate root and tes/mod.rs only assemble the layers


class Resolver:
    def __init__(self, sources):
        self.src = sources

    def member(self, module, name, seen):
        """Follow explicit re-exports and glob imports to the defining module."""
        key = (module, name)
        if key in seen:
            return None
        seen = seen | {key}
        if name in self.src.declarations[module]:
            return module + (name,)
        scope = self.src.scopes.get(module)
        if scope:
            if name in scope.bindings:
                return self.resolve(scope, scope.bindings[name], seen)
            for glob in scope.globs:
                target = self.resolve(scope, glob, seen)
                if target in self.src.modules:
                    found = self.member(target, name, seen)
                    if found:
                        return found
        return None

    def resolve(self, scope, parts, seen=frozenset()):
        head, *rest = parts
        if head == "crate":
            target = scope.module[:1]
        elif head in CRATES:
            target = (head,)
        elif head in ("super", "self"):
            target = scope.module[:-1] if head == "super" else scope.module
        else:
            target = None
            block = scope
            while block.parent is not None:
                key = (block, head)
                if head in block.bindings and key not in seen:
                    target = self.resolve(block, block.bindings[head], seen | {key})
                if target:
                    break
                for glob in block.globs:
                    if key not in seen:
                        module = self.resolve(block, glob, seen | {key})
                        if module in self.src.modules:
                            target = self.member(module, head, seen)
                    if target:
                        break
                if target:
                    break
                block = block.parent
            if not target:
                target = self.member(scope.module, head, seen)
        if not target:
            return None
        for name in rest:
            if name == "super":
                target = target[:-1]
            elif name != "*":
                target = self.member(target, name, seen) or target + (name,)
        return target

    def glob_targets(self, module, seen=frozenset()):
        """Import the exported names of a facade, including chained pub use globs."""
        if module not in self.src.scopes or module in seen:
            return
        scope = self.src.scopes[module]
        for name in scope.exports:
            target = self.member(module, name, frozenset())
            if target:
                yield target
        for glob in scope.export_globs:
            target = self.resolve(scope, glob)
            yield from self.glob_targets(target, seen | {module})

    def graph(self):
        edges = {}
        for (path, scope, parts), line in self.src.references.items():
            target = self.resolve(scope, parts)
            if not target:
                continue  # another crate, or a local type / associated function
            targets = [target]
            if parts[-1] == "*":
                targets.extend(self.glob_targets(target))
            for target in targets:
                while target and target not in self.src.modules:
                    target = target[:-1]
                if not target or self.src.modules[target] == path:
                    continue
                source_layer = self.src.file_layers[path]
                target_layer = self.src.file_layers[self.src.modules[target]]
                if source_layer and target_layer:
                    edge = (str(path), "::".join(target))
                    edges.setdefault(edge, (source_layer, target_layer, line))
        return edges


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    graph = commands.add_parser("graph", help="show each layer pair and its edge count")
    graph.add_argument("--upward", action="store_true", help="print upward edges as baseline JSON")
    commands.add_parser("check", help="compare current upward edges to the reviewed baseline")
    args = parser.parse_args()
    sources = Sources(ROOT)
    for crate in CRATES:
        sources.read((crate,), Path("crates", crate, "src/lib.rs"))
    edges = Resolver(sources).graph()
    upward = {(p, t): line for (p, t), (a, b, line) in edges.items()
              if rank(a) < rank(b)}

    if args.command == "graph":
        if args.upward:
            grouped = defaultdict(list)
            for source, target in sorted(upward):
                grouped[source].append(target)
            print(json.dumps(grouped, indent=2))
        else:
            pairs = Counter((a, b) for a, b, _ in edges.values())
            for a in LAYERS:
                for b in LAYERS:
                    if pairs[a, b]:
                        print(f"{a:18} -> {b:18} {pairs[a, b]:4}")
        return 0

    baseline = json.loads(BASELINE.read_text())
    expected = {(p, t) for p, targets in baseline.items() for t in targets}
    added = upward.keys() - expected
    removed = expected - upward.keys()
    for p, t in sorted(added):
        print(f"{p}:{upward[p, t]}: new upward dependency on {t}")
    for p, t in sorted(removed):
        print(f"{p}: removed upward dependency on {t}; review and shrink the baseline")
    print(f"{len(upward)} upward dependencies; {len(added)} added, {len(removed)} removed")
    return int(bool(added or removed))


if __name__ == "__main__":
    sys.exit(main())
