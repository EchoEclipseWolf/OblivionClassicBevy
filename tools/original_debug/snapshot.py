#!/usr/bin/env python3
"""Reads an original-game memory snapshot written by capture_a.py's `snapshot` set. See docs/original-debug.md."""
import bisect
import json
from pathlib import Path


class Snapshot:
    def __init__(self, folder):
        self.folder = Path(folder)
        self.index = json.loads((self.folder / "index.json").read_text())
        self.regions = sorted(self.index["regions"], key=lambda r: r["start"])
        self.starts = [r["start"] for r in self.regions]
        self.cache = {}

    def _region(self, address):
        i = bisect.bisect_right(self.starts, address) - 1
        if i < 0 or address >= self.regions[i]["end"]:
            return None
        return self.regions[i]

    def read(self, address, size):
        region = self._region(address)
        if region is None or address + size > region["end"]:
            return None
        start = region["start"]
        if start not in self.cache:
            self.cache[start] = (self.folder / f"{start:08X}.bin").read_bytes()
        return self.cache[start][address - start:address - start + size]

    def u32(self, address):
        data = self.read(address, 4)
        return int.from_bytes(data, "little") if data else None

    def u8(self, address):
        data = self.read(address, 1)
        return data[0] if data else None

    def text(self, address, limit=256):
        data = self.read(address, limit) or b""
        return data.split(b"\0")[0].decode("latin-1")

    def walk_list(self, node, limit=1_000_000):
        """A { item, next } list from its first node: the items."""
        items = []
        while node and len(items) < limit:
            item, following = self.u32(node), self.u32(node + 4)
            if item is None:
                break
            if item:
                items.append(item)
            node = following
        return items
