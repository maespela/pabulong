#!/usr/bin/env python3
"""Renders the Pabulong .ico files for Windows from the logo kit's mark.

The mark in the kit (01-svg/mark/*.svg, 05-app-icons/pabulong-app-icon-master.svg)
is pure geometry: a rounded stem, a ring and two arcs around the ring's
centre. This draws exactly that geometry with anti-aliasing, so the icons
match the kit without an SVG renderer. The wordmark is never drawn.

    python3 make-icons.py [out_dir]

Writes:
  pabulong.ico          app / installer icon: the kit's white tile with the
                        colour mark (16-256 px; the faded outer ring is
                        dropped at 24 px and below, like the kit's favicon)
  tray-light-on.ico     tray, light taskbar, processing on  (colour mark)
  tray-light-off.ico    tray, light taskbar, processing off (no rings, 60%)
  tray-dark-on.ico      tray, dark taskbar (reversed: white + lavender)
  tray-dark-off.ico
The off state mirrors MenuBarStatusMark on macOS (rings hidden, dimmed).
Standard library only.
"""

import math
import os
import struct
import sys
import zlib

NAVY = (0x0D, 0x2C, 0x52)
BLUE = (0x1A, 0x3C, 0xFF)
WHITE = (0xFF, 0xFF, 0xFF)
LAVENDER = (0xC6, 0xC9, 0xE3)

# Mark geometry in SVG user units (from the kit).
STEM = ((13.5, 17.5), (13.5, 54.5), 3.5)  # capsule: x=10 w=7 y=14 h=44 rx=3.5
CENTER = (25.5, 26.0)
RING = (12.0, 3.5)  # radius, half stroke
ARC_INNER = (20.0, 2.25)
ARC_OUTER = (27.0, 2.25)
ARC_HALF_ANGLE = math.radians(40.0)
MARK_VIEWBOX = (7.94, 4.33, 48.88, 55.73)


def dist_segment(p, a, b):
    ax, ay = a
    bx, by = b
    px, py = p
    dx, dy = bx - ax, by - ay
    t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)))
    return math.hypot(px - ax - t * dx, py - ay - t * dy)


def dist_arc(p, radius):
    cx, cy = CENTER
    x, y = p[0] - cx, p[1] - cy
    if abs(math.atan2(y, x)) <= ARC_HALF_ANGLE:
        return abs(math.hypot(x, y) - radius)
    ends = [(cx + radius * math.cos(s * ARC_HALF_ANGLE), cy + radius * math.sin(s * ARC_HALF_ANGLE)) for s in (-1, 1)]
    return min(math.hypot(p[0] - ex, p[1] - ey) for ex, ey in ends)


def shapes(palette, rings, outer_ring, outer_alpha):
    stem, ring_colour = palette
    out = [
        (lambda p: dist_segment(p, STEM[0], STEM[1]) - STEM[2], stem, 1.0),
        (lambda p: abs(math.hypot(p[0] - CENTER[0], p[1] - CENTER[1]) - RING[0]) - RING[1], stem, 1.0),
    ]
    if rings:
        out.append((lambda p: dist_arc(p, ARC_INNER[0]) - ARC_INNER[1], ring_colour, 1.0))
        if outer_ring:
            out.append((lambda p: dist_arc(p, ARC_OUTER[0]) - ARC_OUTER[1], ring_colour, outer_alpha))
    return out


def render(size, mark_box, palette, rings=True, outer_ring=True, outer_alpha=0.4, opacity=1.0, tile=None, tile_radius=0.0):
    """mark_box: (x, y, scale) mapping SVG units to pixels. tile: background colour."""
    ox, oy, scale = mark_box
    px_in_units = 1.0 / scale
    layers = shapes(palette, rings, outer_ring, outer_alpha)
    pixels = bytearray()
    for j in range(size):
        pixels.append(0)  # PNG filter: none
        for i in range(size):
            r = g = b = a = 0.0
            if tile is not None:
                # Rounded-square tile.
                half = size / 2.0
                qx = abs(i + 0.5 - half) - (half - tile_radius)
                qy = abs(j + 0.5 - half) - (half - tile_radius)
                d = math.hypot(max(qx, 0.0), max(qy, 0.0)) + min(max(qx, qy), 0.0) - tile_radius
                cov = max(0.0, min(1.0, 0.5 - d))
                r, g, b = (c / 255.0 for c in tile)
                a = cov
            u = ((i + 0.5) - ox) / scale
            v = ((j + 0.5) - oy) / scale
            for sdf, colour, alpha in layers:
                d = sdf((u, v))
                cov = max(0.0, min(1.0, 0.5 - d / px_in_units)) * alpha
                if cov <= 0.0:
                    continue
                cr, cg, cb = (c / 255.0 for c in colour)
                na = cov + a * (1.0 - cov)
                if na > 0:
                    r = (cr * cov + r * a * (1.0 - cov)) / na
                    g = (cg * cov + g * a * (1.0 - cov)) / na
                    b = (cb * cov + b * a * (1.0 - cov)) / na
                a = na
            a *= opacity
            pixels += bytes((round(r * 255), round(g * 255), round(b * 255), round(a * 255)))
    return png(size, size, bytes(pixels))


def png(w, h, raw):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def ico(images):
    """images: list of (size, png_bytes)."""
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries = b""
    data = b""
    for size, blob in images:
        dim = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset + len(data))
        data += blob
    return header + entries + data


def mark_fit(size, padding):
    """Centre the mark's viewBox in a square of `size` px with `padding` px margins."""
    vx, vy, vw, vh = MARK_VIEWBOX
    scale = (size - 2 * padding) / max(vw, vh)
    ox = (size - vw * scale) / 2.0 - vx * scale
    oy = (size - vh * scale) / 2.0 - vy * scale
    return ox, oy, scale


def app_mark(size):
    # The master tile: translate(190.793 192.555) scale(9.921462) in a 1024 tile.
    k = size / 1024.0
    return 190.793 * k, 192.555 * k, 9.921462 * k


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__))
    os.makedirs(out, exist_ok=True)

    app = []
    for size in (16, 20, 24, 32, 40, 48, 64, 96, 128, 256):
        app.append((size, render(size, app_mark(size), (NAVY, BLUE), outer_ring=size > 24, tile=WHITE, tile_radius=size * 0.18)))
    with open(os.path.join(out, "pabulong.ico"), "wb") as f:
        f.write(ico(app))

    tray_sizes = (16, 20, 24, 32, 40, 48)
    variants = {
        "tray-light-on": dict(palette=(NAVY, BLUE), outer_alpha=0.4),
        "tray-light-off": dict(palette=(NAVY, BLUE), rings=False, opacity=0.6),
        "tray-dark-on": dict(palette=(WHITE, LAVENDER), outer_alpha=0.45),
        "tray-dark-off": dict(palette=(WHITE, LAVENDER), rings=False, opacity=0.6),
    }
    for name, opts in variants.items():
        images = []
        for size in tray_sizes:
            images.append((size, render(size, mark_fit(size, max(1, size // 16)), outer_ring=size > 24, **opts)))
        with open(os.path.join(out, name + ".ico"), "wb") as f:
            f.write(ico(images))
    print("icons written to", out)


if __name__ == "__main__":
    main()
