#!/usr/bin/env python3
"""Pixel differences over the parts of a screen the movie doesn't touch.

static_compare.py PORT.png ORIGINAL.png [left,top,right,bottom ...]
Prints the mean and peak of the worst channel, and how many pixels pass 2, 16 and 64. See docs/original-debug.md.
"""
import subprocess
import sys

REGIONS = {"above the map": (0, 0, 1920, 186), "below the map": (0, 890, 1920, 1080), "version": (8, 1054, 90, 1076)}


def load(path):
    raw = subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-i", path, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"],
                         check=True, capture_output=True).stdout
    assert len(raw) == 1920 * 1080 * 3, path
    return raw


def main():
    a, b = load(sys.argv[1]), load(sys.argv[2])
    regions = dict(REGIONS)
    for text in sys.argv[3:]:
        regions[text] = tuple(int(part) for part in text.split(","))
    for name, (left, top, right, bottom) in regions.items():
        total = peak = over2 = over16 = over64 = 0
        for y in range(top, bottom):
            for x in range(left, right):
                at = (y * 1920 + x) * 3
                worst = max(abs(a[at] - b[at]), abs(a[at + 1] - b[at + 1]), abs(a[at + 2] - b[at + 2]))
                total += worst
                peak = max(peak, worst)
                over2 += worst > 2
                over16 += worst > 16
                over64 += worst > 64
        pixels = (right - left) * (bottom - top)
        print(f"{name:15} mean {total / pixels:.3f} peak {peak} over2 {over2} over16 {over16} over64 {over64} of {pixels}")


if __name__ == "__main__":
    main()
