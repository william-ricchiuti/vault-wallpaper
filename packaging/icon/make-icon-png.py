#!/usr/bin/env python3
"""Draw the placeholder app icon (a small node graph on a dark rounded square).

Standard library only, so it runs on a fresh Mac. Writes a 1024x1024 RGBA PNG.
Usage: make-icon-png.py <out.png>
TODO: replace with a designed icon before the first public release.
"""
import math
import struct
import sys
import zlib

N = 1024
BG = (22, 24, 38)
EDGE = (120, 110, 200)
NODE = (170, 150, 255)
HUB = (235, 225, 255)

NODES = [(512, 512, 70), (300, 330, 38), (730, 300, 44), (760, 690, 40),
         (330, 730, 46), (520, 200, 30), (190, 520, 28), (860, 480, 30)]
EDGES = [(0, 1), (0, 2), (0, 3), (0, 4), (1, 5), (2, 5), (1, 6), (4, 6), (2, 7), (3, 7)]


def inside_rounded(x, y, r=200, pad=40):
    lo, hi = pad + r, N - pad - r
    cx = min(max(x, lo), hi)
    cy = min(max(y, lo), hi)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def seg_dist(px, py, ax, ay, bx, by):
    dx, dy = bx - ax, by - ay
    t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)))
    return math.hypot(px - ax - t * dx, py - ay - t * dy)


def pixel(x, y):
    if not inside_rounded(x, y):
        return (0, 0, 0, 0)
    for i, (nx, ny, r) in enumerate(NODES):
        if (x - nx) ** 2 + (y - ny) ** 2 <= r * r:
            return (*(HUB if i == 0 else NODE), 255)
    for a, b in EDGES:
        ax, ay, _ = NODES[a]
        bx, by, _ = NODES[b]
        if seg_dist(x, y, ax, ay, bx, by) <= 7:
            return (*EDGE, 255)
    return (*BG, 255)


def main(out):
    rows = bytearray()
    for y in range(N):
        rows.append(0)
        for x in range(N):
            rows.extend(pixel(x, y))

    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        return c + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", N, N, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(rows), 9))
    png += chunk(b"IEND", b"")
    with open(out, "wb") as f:
        f.write(png)


if __name__ == "__main__":
    main(sys.argv[1])
