#!/usr/bin/env python3
"""The same run on both sides: a memory snapshot of the original against the port's dump from that run's seed.

ledger_compare.py SNAPSHOT_FOLDER PORT_DUMP
PORT_DUMP is what `dump_for_the_ledger_compare` writes. See docs/original-debug.md.
"""
import collections
import sys

from snapshot import Snapshot

FORMS_MAP = 0xB0613C  # NiTPointerMap header: vtable, hash size, buckets, count.
SLG = 0xB33B00
REFERENCE_TYPES = (0x31, 0x32, 0x33)


def walk_map(snap, header):
    size, buckets = snap.u32(header + 4), snap.u32(header + 8)
    for index in range(size):
        node = snap.u32(buckets + index * 4)
        while node:
            yield index, snap.u32(node + 4), snap.u32(node + 8)
            node = snap.u32(node)


def original(snap):
    forms, refs = {}, {}
    for _, key, value in walk_map(snap, FORMS_MAP):
        kind = snap.u8(value + 4)
        flags = snap.u32(value + 8)
        forms[key] = (kind, flags)
        if kind in REFERENCE_TYPES:
            base = snap.u32(value + 0x1C)
            parent = snap.u32(value + 0x40)
            extras = []
            node = snap.u32(value + 0x48)
            while node and len(extras) < 64:
                extras.append(snap.u8(node + 4))
                node = snap.u32(node + 8)
            words = lambda at, n: ",".join("%08X" % snap.u32(value + at + i * 4) for i in range(n))
            refs[key] = (
                snap.u32(base + 0xC) if base else 0,
                snap.u32(parent + 0xC) if parent else 0,
                words(0x2C, 3),
                words(0x20, 3),
                "%08X" % snap.u32(value + 0x38),
                extras,
            )
    changes = {}
    game = snap.u32(SLG)
    for _, key, value in walk_map(snap, snap.u32(game)):
        changes[key] = (snap.u32(value), snap.u32(value + 4))
    return forms, refs, changes


def port(path):
    forms, refs, changes = {}, {}, {}
    for line in open(path):
        part = line.split()
        if part[0] == "FORM":
            forms[int(part[1], 16)] = (int(part[2], 16), int(part[3], 16))
        elif part[0] == "REF":
            extras = [int(x, 16) for x in part[7].split(",")] if len(part) > 7 and part[7] else []
            refs[int(part[1], 16)] = (int(part[2], 16), int(part[3], 16), part[4], part[5], part[6], extras)
        elif part[0] == "CHG":
            changes[int(part[1], 16)] = (int(part[2], 16), int(part[3], 16))
    return forms, refs, changes


def report(name, ours, theirs, show=12):
    only_original = sorted(set(theirs) - set(ours))
    only_port = sorted(set(ours) - set(theirs))
    print(f"{name}: original {len(theirs)}, port {len(ours)}, only original {len(only_original)}, only port {len(only_port)}")
    for key in only_original[:show]:
        print("  original only %X %s" % (key, str(theirs[key])[:110]))
    for key in only_port[:show]:
        print("  port only     %X %s" % (key, str(ours[key])[:110]))


def main():
    snap = Snapshot(sys.argv[1])
    o_forms, o_refs, o_changes = original(snap)
    p_forms, p_refs, p_changes = port(sys.argv[2])
    report("forms", p_forms, o_forms)
    both = set(o_forms) & set(p_forms)
    kind = [k for k in both if o_forms[k][0] != p_forms[k][0]]
    flags = [k for k in both if o_forms[k][1] != p_forms[k][1]]
    print(f"forms in both {len(both)}: type differs {len(kind)}, flags differ {len(flags)}")
    bits = collections.Counter(o_forms[k][1] ^ p_forms[k][1] for k in flags)
    print("  flag xors", [("%X" % x, n) for x, n in bits.most_common(14)])
    by_type = collections.Counter((o_forms[k][0], "%X" % (o_forms[k][1] ^ p_forms[k][1])) for k in flags)
    print("  by type and xor", [("%X" % t, x, n) for (t, x), n in by_type.most_common(14)])
    for k in sorted(flags)[:10]:
        print("  %X type %X original %X port %X" % (k, o_forms[k][0], o_forms[k][1], p_forms[k][1]))
    report("references", p_refs, o_refs)
    names = ["base", "parent", "pos", "rot", "scale", "extras"]
    for index, name in enumerate(names):
        keys = [k for k in set(o_refs) & set(p_refs) if o_refs[k][index] != p_refs[k][index]]
        print(f"references {name} differ: {len(keys)}")
        for k in sorted(keys)[:6]:
            print("  %X type %X original %s port %s" % (k, o_forms[k][0], str(o_refs[k][index])[:60], str(p_refs[k][index])[:60]))
    report("changes", p_changes, o_changes)
    keys = [k for k in set(o_changes) & set(p_changes) if o_changes[k] != p_changes[k]]
    print(f"changes differ: {len(keys)}")
    xors = collections.Counter(o_changes[k][0] ^ p_changes[k][0] for k in keys)
    print("  xors", [("%X" % x, n) for x, n in xors.most_common(10)])
    for k in sorted(keys)[:10]:
        print("  %X original %s port %s" % (k, o_changes[k], p_changes[k]))


if __name__ == "__main__":
    main()
