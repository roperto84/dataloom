#!/usr/bin/env python3
"""Render the Dataloom app icon as a 1024x1024 PNG.

Pure standard library on purpose: the build machine has no Pillow.  The icon
matches `widgets::draw_logo` in the app: an indigo-to-sky rounded tile, a white
database cylinder, and a mint spark.
"""

import math
import os
import struct
import zlib

SIZE = 1024
PAD = 56.0
SIDE = SIZE - PAD * 2.0
RADIUS = SIDE * 0.225

BRAND_TOP = (99, 102, 241)      # #6366F1
BRAND_BOTTOM = (14, 165, 233)   # #0EA5E9
GLYPH = (248, 250, 252)         # #F8FAFC
SPARK = (52, 211, 153)          # #34D399
BAND = (67, 56, 202)            # #4338CA


def clamp(value, low=0.0, high=1.0):
    return low if value < low else high if value > high else value


def mix(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def rounded_rect_sdf(x, y, cx, cy, hx, hy, r):
    qx = abs(x - cx) - (hx - r)
    qy = abs(y - cy) - (hy - r)
    ax = max(qx, 0.0)
    ay = max(qy, 0.0)
    return math.hypot(ax, ay) + min(max(qx, qy), 0.0) - r


def ellipse_sdf(x, y, cx, cy, rx, ry):
    scale = min(rx, ry)
    return (math.hypot((x - cx) / rx, (y - cy) / ry) - 1.0) * scale


def rect_sdf(x, y, cx, cy, hx, hy):
    dx = abs(x - cx) - hx
    dy = abs(y - cy) - hy
    ax = max(dx, 0.0)
    ay = max(dy, 0.0)
    return math.hypot(ax, ay) + min(max(dx, dy), 0.0)


def coverage(distance):
    return clamp(0.5 - distance)


def over(dst, src_rgb, src_alpha):
    if src_alpha <= 0.0:
        return dst
    a = src_alpha + dst[3] * (1.0 - src_alpha)
    if a <= 0.0:
        return (0.0, 0.0, 0.0, 0.0)
    rgb = tuple(
        (src_rgb[i] * src_alpha + dst[i] * dst[3] * (1.0 - src_alpha)) / a
        for i in range(3)
    )
    return (rgb[0], rgb[1], rgb[2], a)


def point_in_triangle(px, py, a, b, c):
    def sign(p1, p2, p3):
        return (p1[0] - p3[0]) * (p2[1] - p3[1]) - (p2[0] - p3[0]) * (p1[1] - p3[1])

    d1 = sign((px, py), a, b)
    d2 = sign((px, py), b, c)
    d3 = sign((px, py), c, a)
    has_neg = d1 < 0 or d2 < 0 or d3 < 0
    has_pos = d1 > 0 or d2 > 0 or d3 > 0
    return not (has_neg and has_pos)


# --- Geometry shared by the app's draw_logo (all lengths relative to SIDE) ---
CX = SIZE / 2.0
CY = SIZE / 2.0
RX = SIDE * 0.208
RY = SIDE * 0.0855
TOP_Y = CY - SIDE * 0.134
BOTTOM_Y = CY + SIDE * 0.134
BAND_YS = (TOP_Y + (BOTTOM_Y - TOP_Y) * 0.42, TOP_Y + (BOTTOM_Y - TOP_Y) * 0.72)
BAND_WIDTH = SIDE * 0.014

SPARK_CX = PAD + SIDE * 0.835
SPARK_CY = PAD + SIDE * 0.825
SPARK_OUTER = SIDE * 0.105
SPARK_INNER = SIDE * 0.036


def spark_triangles():
    triangles = []
    for k in range(4):
        angle = math.radians(-90 + 90 * k)
        tip = (
            SPARK_CX + math.cos(angle) * SPARK_OUTER,
            SPARK_CY + math.sin(angle) * SPARK_OUTER,
        )
        left = (
            SPARK_CX + math.cos(angle + math.pi / 4) * SPARK_INNER,
            SPARK_CY + math.sin(angle + math.pi / 4) * SPARK_INNER,
        )
        right = (
            SPARK_CX + math.cos(angle - math.pi / 4) * SPARK_INNER,
            SPARK_CY + math.sin(angle - math.pi / 4) * SPARK_INNER,
        )
        triangles.append((left, tip, right))
    return triangles


SPARK_TRIS = spark_triangles()


def spark_coverage(x, y):
    # 2x2 supersampling inside the spark's bounding box only.
    if math.hypot(x - SPARK_CX, y - SPARK_CY) <= SPARK_INNER * 1.15:
        return 1.0
    hits = 0.0
    for dx, dy in ((0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)):
        px, py = x + dx, y + dy
        if any(point_in_triangle(px, py, *tri) for tri in SPARK_TRIS):
            hits += 0.25
    return hits


def render():
    pixels = bytearray()
    for y in range(SIZE):
        row = bytearray()
        fy = y + 0.5
        for x in range(SIZE):
            fx = x + 0.5
            dst = (0.0, 0.0, 0.0, 0.0)

            tile = coverage(
                rounded_rect_sdf(fx, fy, CX, CY, SIDE / 2.0, SIDE / 2.0, RADIUS)
            )
            if tile <= 0.0:
                row.extend((0, 0, 0, 0))
                continue

            t = clamp(((fx - PAD) + (fy - PAD)) / (2.0 * SIDE))
            base = mix(BRAND_TOP, BRAND_BOTTOM, t)
            # Soft top-left light so the flat gradient feels like a surface.
            glow = clamp(1.0 - math.hypot(fx - PAD, fy - PAD) / (SIDE * 0.85))
            base = tuple(round(base[i] + (255 - base[i]) * 0.10 * glow) for i in range(3))
            dst = (base[0], base[1], base[2], tile)

            # Soft shadow beneath the cylinder.
            shadow = coverage(
                ellipse_sdf(fx, fy - SIDE * 0.012, CX, BOTTOM_Y, RX * 1.02, RY * 1.10)
            )
            dst = over(dst, (15, 23, 42), shadow * 0.18 * tile)

            body_sdf = min(
                ellipse_sdf(fx, fy, CX, TOP_Y, RX, RY),
                ellipse_sdf(fx, fy, CX, BOTTOM_Y, RX, RY),
                rect_sdf(
                    fx,
                    fy,
                    CX,
                    (TOP_Y + BOTTOM_Y) / 2.0,
                    RX,
                    (BOTTOM_Y - TOP_Y) / 2.0,
                ),
            )
            body = min(coverage(body_sdf), tile)
            dst = over(dst, GLYPH, body)

            for band_y in BAND_YS:
                ring = coverage(
                    abs(ellipse_sdf(fx, fy, CX, band_y, RX, RY)) - BAND_WIDTH / 2.0
                )
                dst = over(dst, BAND, ring * 0.42 * body)

            rim = coverage(
                abs(ellipse_sdf(fx, fy, CX, TOP_Y, RX, RY)) - SIDE * 0.008
            )
            dst = over(dst, BAND, rim * 0.30 * body)

            spark = spark_coverage(fx, fy)
            dst = over(dst, SPARK, spark * tile)

            row.extend(
                (
                    round(clamp(dst[0], 0, 255)),
                    round(clamp(dst[1], 0, 255)),
                    round(clamp(dst[2], 0, 255)),
                    round(clamp(dst[3]) * 255),
                )
            )
        pixels.extend(row)
    return bytes(pixels)


def write_png(path, width, height, rgba):
    def chunk(kind, data):
        payload = kind + data
        return (
            struct.pack(">I", len(data))
            + payload
            + struct.pack(">I", zlib.crc32(payload) & 0xFFFFFFFF)
        )

    stride = width * 4
    raw = bytearray()
    for y in range(height):
        raw.append(0)
        raw.extend(rgba[y * stride : (y + 1) * stride])

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as handle:
        handle.write(png)


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    assets = os.path.join(root, "assets")
    os.makedirs(assets, exist_ok=True)
    rgba = render()
    write_png(os.path.join(assets, "icon.png"), SIZE, SIZE, rgba)
    print(f"wrote {os.path.join(assets, 'icon.png')}")


if __name__ == "__main__":
    main()
