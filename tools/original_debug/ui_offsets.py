#!/usr/bin/env python3
"""Where each piece of a screen sits, port against an original frame, when the movie under them differs.

ui_offsets.py PORT.png ORIGINAL.png [ORIGINAL2.png ...]
For each box: the shift of the port's box, in whole pixels, that best matches the original, by the sum of absolute differences
over the box's dark pixels (the UI), which the movie doesn't move. With several original frames, the median of each pixel first
takes most of the movie out. See docs/original-debug.md.
"""
import subprocess
import sys

BOXES = {
    "version": (8, 1054, 90, 1076),
    "logo title": (750, 295, 1175, 340),
    "logo rule": (540, 343, 1385, 354),
    "logo word": (545, 358, 1378, 476),
    "New": (540, 835, 626, 872),
    "Load": (694, 835, 790, 872),
    "Options": (858, 835, 1002, 875),
    "Credits": (1068, 835, 1208, 872),
    "Exit": (1274, 835, 1360, 872),
    "top bar": (0, 118, 1920, 190),
    "bottom bar": (0, 885, 1920, 955),
}
REACH = 4


def load(path):
    raw = subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-i", path, "-f", "rawvideo", "-pix_fmt", "gray", "-"],
                         check=True, capture_output=True).stdout
    assert len(raw) == 1920 * 1080, path
    return raw


def main():
    port = load(sys.argv[1])
    frames = [load(path) for path in sys.argv[2:]]
    if len(frames) == 1:
        original = frames[0]
    else:
        original = bytes(sorted(values)[len(values) // 2] for values in zip(*frames))
    for name, (left, top, right, bottom) in BOXES.items():
        scores = []
        for dy in range(-REACH, REACH + 1):
            for dx in range(-REACH, REACH + 1):
                total = 0
                for y in range(top, bottom):
                    a = port[y * 1920 + left:y * 1920 + right]
                    b = original[(y + dy) * 1920 + left + dx:(y + dy) * 1920 + right + dx]
                    total += sum(abs(p - q) for p, q in zip(a, b))
                scores.append((total / ((right - left) * (bottom - top)), dx, dy))
        scores.sort()
        here = next(score for score in scores if score[1] == 0 and score[2] == 0)
        print(f"{name:12} original is port + ({scores[0][1]:+d},{scores[0][2]:+d}), mean {scores[0][0]:.2f}; at (0,0) {here[0]:.2f}; next {scores[1][1:]} {scores[1][0]:.2f}")


if __name__ == "__main__":
    main()
