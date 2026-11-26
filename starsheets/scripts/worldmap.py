#!/usr/bin/env python3
"""Build the Photos world map (app/assets/worldmap.bin) from Natural Earth GeoJSON.

Natural Earth (https://www.naturalearthdata.com) is in the public domain
(https://www.naturalearthdata.com/about/terms-of-use/). The GeoJSON copies come from
https://github.com/nvkelso/natural-earth-vector (geojson/). Usage:

    python3 worldmap.py <dir with the ne_*.geojson files> <out.bin>

Format "SBMAP1" (integers are unsigned LEB128 varints, coordinate deltas zigzag):
    magic b"SBMAP1", u8 layer count, then per layer:
      u8 kind (0 land, 1 lake, 2 border, 3 label), u8 band (0 world view, 1 close up),
      varint item count, then per item:
        kind 0-2: varint point count, then x,y zigzag deltas (Web Mercator, 0..1 scaled by 2^20)
        kind 3:   x, y (varint), u8 rank, varint name length, UTF-8 name
Polygons keep only their outer ring: holes (the Caspian...) are drawn by the lake layer on top.
"""
import json
import math
import sys

Q = 1 << 20
MAXLAT = 85.05112878


def merc(lon, lat):
    lat = max(-MAXLAT, min(MAXLAT, lat))
    x = (lon + 180.0) / 360.0
    s = math.sin(math.radians(lat))
    y = 0.5 - math.log((1 + s) / (1 - s)) / (4 * math.pi)
    return (min(Q - 1, max(0, round(x * Q))), min(Q - 1, max(0, round(y * Q))))


def dp(pts, tol):
    """Douglas-Peucker, iterative, on quantized points; tol in quantized units."""
    if len(pts) < 3:
        return pts
    keep = [False] * len(pts)
    keep[0] = keep[-1] = True
    stack = [(0, len(pts) - 1)]
    t2 = tol * tol
    while stack:
        a, b = stack.pop()
        (ax, ay), (bx, by) = pts[a], pts[b]
        dx, dy = bx - ax, by - ay
        l2 = dx * dx + dy * dy
        best, bi = -1.0, -1
        for i in range(a + 1, b):
            px, py = pts[i]
            if l2 == 0:
                d = (px - ax) ** 2 + (py - ay) ** 2
            else:
                t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / l2))
                d = (px - ax - t * dx) ** 2 + (py - ay - t * dy) ** 2
            if d > best:
                best, bi = d, i
        if best > t2:
            keep[bi] = True
            stack += [(a, bi), (bi, b)]
    out = [p for p, k in zip(pts, keep) if k]
    return [p for i, p in enumerate(out) if i == 0 or p != out[i - 1]]


def varint(n):
    b = bytearray()
    while True:
        c = n & 0x7F
        n >>= 7
        b.append(c | (0x80 if n else 0))
        if not n:
            return bytes(b)


def zz(n):
    return (n << 1) ^ (n >> 63)


def parts(geom):
    t, c = geom["type"], geom["coordinates"]
    if t == "Polygon":
        return [c[0]]
    if t == "MultiPolygon":
        return [p[0] for p in c]
    if t == "LineString":
        return [c]
    if t == "MultiLineString":
        return c
    return []


def layer(path, kind, band, tol, closed):
    feats = json.load(open(path))["features"]
    items = []
    for f in feats:
        for ring in parts(f["geometry"]):
            pts = dp([merc(p[0], p[1]) for p in ring], tol)
            if len(pts) >= (4 if closed else 2):
                items.append(pts)
    out = bytearray([kind, band]) + varint(len(items))
    n = 0
    for pts in items:
        out += varint(len(pts))
        px = py = 0
        for x, y in pts:
            out += varint(zz(x - px)) + varint(zz(y - py))
            px, py = x, y
        n += len(pts)
    print(f"{path}: {len(items)} items, {n} points, {len(out)} bytes", file=sys.stderr)
    return out


def labels(path, band):
    feats = json.load(open(path))["features"]
    out = bytearray([3, band]) + varint(len(feats))
    for f in feats:
        p = f["properties"]
        x, y = merc(*f["geometry"]["coordinates"][:2])
        name = (p.get("name") or p.get("NAME") or "").encode()[:60]
        rank = int(p.get("scalerank", p.get("SCALERANK", 9)) or 0)
        out += varint(x) + varint(y) + bytes([min(255, rank)]) + varint(len(name)) + name
    return out


def main():
    d, dst = sys.argv[1], sys.argv[2]
    # Tolerances: about half a pixel at the widest zoom each band is drawn at
    # (band 0 up to a 2048 px wide world, band 1 up to 8192 px; past that the 1:50m data is the limit).
    t0, t1 = Q / 2048 * 0.6, Q / 8192 * 0.7
    ls = [
        layer(f"{d}/ne_110m_land.geojson", 0, 0, t0, True),
        layer(f"{d}/ne_110m_lakes.geojson", 1, 0, t0, True),
        layer(f"{d}/ne_110m_admin_0_boundary_lines_land.geojson", 2, 0, t0, False),
        layer(f"{d}/ne_50m_land.geojson", 0, 1, t1, True),
        layer(f"{d}/ne_50m_lakes.geojson", 1, 1, t1, True),
        layer(f"{d}/ne_50m_admin_0_boundary_lines_land.geojson", 2, 1, t1, False),
        labels(f"{d}/ne_110m_populated_places_simple.geojson", 0),
    ]
    blob = b"SBMAP1" + bytes([len(ls)]) + b"".join(ls)
    open(dst, "wb").write(blob)
    print(f"{dst}: {len(blob)} bytes", file=sys.stderr)


main()
