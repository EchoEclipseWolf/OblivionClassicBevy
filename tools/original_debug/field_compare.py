#!/usr/bin/env python3
"""Every offset-named field of the port's components against the same run's memory snapshot.

field_compare.py SNAPSHOT_FOLDER PORT_DUMP [TYPE_FILTER]
PORT_DUMP holds `ENT entity id` and `COMP Type id {:?}` lines, from `dump_for_the_ledger_compare`. A field named
`flags_24` compares with the original at +0x24. A number is compared at the width that fits it, so a u8 and a u32 both pass
on the right low bytes, and that can hide a wrong high byte: each such pass prints as LOOSE, to be read against the field's
type. A field with no offset in its name isn't compared.
"""
import collections
import math
import re
import struct
import sys

from ledger_compare import FORMS_MAP, walk_map
from snapshot import Snapshot

TOKEN = re.compile(r'\s*("(?:[^"\\]|\\.)*"|[A-Za-z_][A-Za-z0-9_]*|-?\d+v\d+(?:#\d+)?|-?(?:\d+\.\d*(?:e-?\d+)?|\d+e-?\d+)|-?\d+|NaN|-?inf|[{}()\[\],:])')
OFFSET = re.compile(r"_([0-9a-f]{2,4})$")
ABSOLUTE = re.compile(r"_(b[0-9a-f]{5})$")
SKIP_TYPES = {"BsStringT", "PointerList", "PointerMap", "ExtraDataList", "ModReferences", "NiTPointerMapRecord"}


def tokens(text):
    position = 0
    while position < len(text):
        match = TOKEN.match(text, position)
        if not match:
            if text[position:].strip() == "":
                return
            raise ValueError(text[position:position + 40])
        yield match.group(1)
        position = match.end()


class Parser:
    def __init__(self, text):
        self.tokens = list(tokens(text))
        self.at = 0

    def peek(self):
        return self.tokens[self.at] if self.at < len(self.tokens) else None

    def take(self):
        self.at += 1
        return self.tokens[self.at - 1]

    def value(self):
        token = self.take()
        if token == "[":
            items = []
            while self.peek() != "]":
                items.append(self.value())
                if self.peek() == ",":
                    self.take()
            self.take()
            return ("list", items)
        if token == "(":
            items = []
            while self.peek() != ")":
                items.append(self.value())
                if self.peek() == ",":
                    self.take()
            self.take()
            return ("tuple", items)
        if token[0] == '"':
            return ("str", token)
        if re.fullmatch(r"-?\d+v\d+(#\d+)?", token):
            return ("entity", token.split("#")[0])
        if re.fullmatch(r"-?\d+", token):
            return ("int", int(token))
        if token in ("NaN", "inf", "-inf") or re.fullmatch(r"-?(\d+\.\d*(e-?\d+)?|\d+e-?\d+)", token):
            return ("float", float(token))
        if token in ("true", "false"):
            return ("bool", token == "true")
        if token == "None":
            return ("none", None)
        if self.peek() == "{":
            self.take()
            fields = []
            while self.peek() != "}":
                name = self.take()
                self.take()
                fields.append((name, self.value()))
                if self.peek() == ",":
                    self.take()
            self.take()
            return ("struct", token, fields)
        if self.peek() == "(":
            self.take()
            items = []
            while self.peek() != ")":
                items.append(self.value())
                if self.peek() == ",":
                    self.take()
            self.take()
            return ("wrap", token, items)
        return ("name", token)


class Compare:
    def __init__(self, snap):
        self.snap = snap
        self.pointers = {}
        for _, key, value in walk_map(snap, FORMS_MAP):
            self.pointers[value] = key
        self.entities = {}
        self.result = collections.defaultdict(lambda: [0, 0, None])
        self.loose = collections.Counter()
        self.loose_sample = {}
        self.tag_now = self.owner_now = None

    def record(self, name, good, sample):
        slot = self.result[name]
        slot[0] += 1
        if not good:
            slot[1] += 1
            slot[2] = slot[2] or sample

    def scalar(self, kind, value, address):
        word = self.snap.u32(address)
        if word is None:
            return None, "unmapped"
        if kind == "int":
            masked = value & 0xFFFFFFFF
            good = masked in (word, word & 0xFF, word & 0xFFFF) or (self.pointers.get(word) == value)
            # A port value that passes only by width: the original's word holds more. Kept apart, since a u8 field beside
            # other data looks just like a missed pointer.
            if good and masked != word and self.pointers.get(word) != value:
                self.loose[self.tag_now] += 1
                self.loose_sample.setdefault(self.tag_now, ("%X" % masked, "%X" % word, self.owner_now))
            return good, "%X" % word
        if kind == "bool":
            return ((word & 0xFF) != 0) == value or (word != 0) == value, "%X" % (word & 0xFF)
        original = struct.unpack("<f", struct.pack("<I", word))[0]
        if math.isnan(value):
            return math.isnan(original), "%f" % original
        try:
            return struct.pack("<f", value) == struct.pack("<I", word), "%r" % original
        except OverflowError:
            return False, "%r" % original

    def node(self, name, node, address, tag, owner):
        kind = node[0]
        if kind in ("int", "float", "bool"):
            self.tag_now, self.owner_now = tag, owner
            good, seen = self.scalar(kind, node[1], address)
            if good is not None:
                self.record(tag, good, (owner, hex(address), node[1], seen))
        elif kind == "entity":
            word = self.snap.u32(address)
            want = self.entities.get(node[1])
            if want is None:
                self.record(tag, bool(word), (owner, hex(address), "a non-form entity", word))
            else:
                self.record(tag, self.pointers.get(word) == want, (owner, hex(address), "%X" % want, "%X" % (self.pointers.get(word) or 0), word))
        elif kind == "none":
            word = self.snap.u32(address)
            if word is not None:
                self.record(tag, word == 0, (owner, hex(address), "None", "%X" % word))
        elif kind == "wrap" and node[1] == "Some" and len(node[2]) == 1:
            self.node(name, node[2][0], address, tag, owner)
        elif kind == "struct" and node[1] not in SKIP_TYPES:
            self.nested(node, address, tag, owner)
        elif kind == "list" and node[1] and all(item[0] in ("int", "float") for item in node[1]):
            before = self.result[tag][:]
            self.array(node[1], address, tag, owner)
            if self.result[tag][1] > before[1]:
                # Behind a pointer: the slot holds the array's address.
                pointer = self.snap.u32(address)
                self.result[tag] = before
                self.array(node[1], pointer if pointer else address, tag, owner)

    def nested(self, node, address, tag, owner):
        # A nested struct's names may be relative to it, absolute in the form, or relative to a pointer in its slot. The form
        # with the fewest differences is taken.
        parent = int(OFFSET.search(tag).group(1), 16) if OFFSET.search(tag) else 0
        pointer = self.snap.u32(address)
        best = None
        for base in (address, address - parent, pointer):
            if not base:
                continue
            saved = self.result, self.loose, self.loose_sample
            self.result = collections.defaultdict(lambda: [0, 0, None])
            self.loose, self.loose_sample = collections.Counter(), {}
            self.fields(node, base, tag, owner)
            attempt = self.result, self.loose, self.loose_sample
            self.result, self.loose, self.loose_sample = saved
            # A value passing only by width counts against a base, or a pointer's low byte picks the wrong one.
            score = sum(slot[1] for slot in attempt[0].values()), sum(attempt[1].values())
            if best is None or score < best[0]:
                best = (score, attempt)
        if best:
            for name, slot in best[1][0].items():
                total = self.result[name]
                total[0] += slot[0]
                total[1] += slot[1]
                total[2] = total[2] or slot[2]
            self.loose.update(best[1][1])
            for name, sample in best[1][2].items():
                self.loose_sample.setdefault(name, sample)

    def array(self, items, address, tag, owner):
        if items[0][0] == "float":
            width = 4
        else:
            width = None
        for candidate in ([width] if width else [1, 2, 4]):
            good = True
            for index, item in enumerate(items):
                raw = self.snap.read(address + index * candidate, candidate)
                if raw is None:
                    good = False
                    break
                if item[0] == "float":
                    if struct.pack("<f", item[1]) != raw and not (math.isnan(item[1]) and math.isnan(struct.unpack("<f", raw)[0])):
                        good = False
                        break
                elif int.from_bytes(raw, "little") != (item[1] & ((1 << (candidate * 8)) - 1)):
                    good = False
                    break
            if good:
                self.record(tag, True, None)
                return
        raw = self.snap.read(address, 16)
        self.record(tag, False, (owner, hex(address), str([item[1] for item in items[:6]]), raw.hex() if raw else None))

    def fields(self, node, base, type_name, owner):
        for field, value in node[2]:
            absolute = ABSOLUTE.search(field)
            if absolute:
                self.node(field, value, int(absolute.group(1), 16), type_name + "." + field, owner)
                continue
            match = OFFSET.search(field)
            if not match:
                continue
            self.node(field, value, base + int(match.group(1), 16), type_name + "." + field, owner)


def main():
    sys.setrecursionlimit(20000)
    snap = Snapshot(sys.argv[1])
    only = sys.argv[3] if len(sys.argv) > 3 else None
    compare = Compare(snap)
    by_id = {key: value for value, key in compare.pointers.items()}
    lines = open(sys.argv[2], errors="replace")
    comps = []
    for line in lines:
        if line.startswith("ENT "):
            part = line.split()
            compare.entities[part[1].split("#")[0]] = int(part[2], 16)
        elif line.startswith("COMP "):
            comps.append(line)
        elif line.startswith("PROC "):
            comps.append(line)
    types = collections.Counter()
    errors = 0
    for line in open(sys.argv[2], errors="replace"):
        part = line.rstrip("\n").split(" ", 2 if line.startswith("GLOB ") else 3)
        if part[0] == "GLOB":
            name, address, text = part[1], 0, part[2]
        elif part[0] == "HEAP":
            name, address, text = part[2], snap.u32(int(part[1], 16)), part[3]
        else:
            continue
        try:
            node = Parser(text).value()
        except (ValueError, IndexError, TypeError, RecursionError) as error:
            errors += 1
            print("parse error", name, repr(error)[:80])
            continue
        types[name] += 1
        if node[0] == "struct":
            compare.fields(node, address, name, name)
    for line in comps:
        _, type_name, owner, text = line.rstrip("\n").split(" ", 3)
        if only and only not in type_name:
            continue
        address = by_id.get(int(owner, 16))
        if address is None:
            continue
        if line.startswith("PROC "):
            address = snap.u32(address + 0x58)
        try:
            node = Parser(text).value()
        except (ValueError, IndexError, TypeError, RecursionError) as error:
            errors += 1
            if errors <= 3:
                print('parse error', type_name, owner, repr(error)[:100], text[:160])
            continue
        types[type_name] += 1
        if node[0] == "struct":
            compare.fields(node, address, type_name, owner)
    print("types", len(types), "instances", sum(types.values()), "parse errors", errors)
    print("passed only by width, the original's word holds more:", len(compare.loose))
    for name, count in compare.loose.most_common():
        print(f"LOOSE {name}: {count}; {compare.loose_sample[name]}")
    rows = sorted(compare.result.items(), key=lambda item: (item[1][1] == 0, item[0]))
    clean = sum(1 for _, slot in rows if slot[1] == 0)
    print("fields", len(rows), "clean", clean, "compares", sum(slot[0] for _, slot in rows))
    for name, slot in rows:
        if slot[1]:
            print(f"{name}: {slot[1]} of {slot[0]} differ; {slot[2]}")


if __name__ == "__main__":
    main()
