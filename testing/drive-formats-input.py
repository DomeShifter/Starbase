#!/usr/bin/env python3
"""Real-input check: Drive shows BMP, TIFF and ICO pictures (tiles and viewer).
Makes a 120x80 picture (red left half, blue right) as .bmp, .tif and .ico in plain Python (no Pillow needed on the
build box), uploads them with as_is=1, then in the browser: the Drive grid tiles get thumbnails (hook
home.files[].thumb == "ready") and the viewer (hook home:preview) shows each one as an image.
Needs the shared click library starbase_ui.py (~/.claude/tools) and a test server (TESTN=n starsheets-testserver.sh up).
Usage: drive-formats-input.py --testn 34 [--browser chromium|firefox] [--shots DIR]
"""
import json, os, struct, sys, time, zlib

W, H = 120, 80


def px(x, y):
    return (220, 30, 30) if x < W // 2 else (30, 30, 220)


def bmp():
    row = (W * 3 + 3) & ~3
    data = b""
    for y in range(H - 1, -1, -1):
        r = b"".join(bytes((b, g, rr)) for rr, g, b in (px(x, y) for x in range(W)))
        data += r + b"\0" * (row - len(r))
    head = struct.pack("<IiiHHIIiiII", 40, W, H, 1, 24, 0, len(data), 2835, 2835, 0, 0)
    return b"BM" + struct.pack("<IHHI", 54 + len(data), 0, 0, 54) + head + data


def tif():
    pix = b"".join(bytes(px(x, y)) for y in range(H) for x in range(W))
    bps_at = 8 + len(pix)
    ifd_at = bps_at + 8
    entries = [(256, 3, 1, W), (257, 3, 1, H), (258, 3, 3, bps_at), (259, 3, 1, 1), (262, 3, 1, 2),
               (273, 4, 1, 8), (277, 3, 1, 3), (278, 3, 1, H), (279, 4, 1, len(pix)), (284, 3, 1, 1)]
    out = b"II*\0" + struct.pack("<I", ifd_at) + pix + struct.pack("<HHHH", 8, 8, 8, 0) + struct.pack("<H", len(entries))
    for tag, typ, n, val in entries:
        out += struct.pack("<HHI", tag, typ, n) + (struct.pack("<HH", val, 0) if typ == 3 and n == 1 else struct.pack("<I", val))
    return out + b"\0\0\0\0"


def ico():
    raw = b"".join(b"\0" + b"".join(bytes(px(x, y) + (255,)) for x in range(W)) for y in range(H))

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))

    png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
    return struct.pack("<HHH", 0, 1, 1) + struct.pack("<BBBBHHII", W, H, 0, 0, 1, 32, len(png), 22) + png


def remote(argv):
    from starbase_ui import Session
    br = argv[argv.index("--browser") + 1] if "--browser" in argv else "chromium"
    testn = int(argv[argv.index("--testn") + 1])
    files = {"fmt-red-blue.bmp": bmp(), "fmt-red-blue.tif": tif(), "fmt-red-blue.ico": ico()}
    ok = True
    with Session(br, testn=testn) as s:
        s.sign_in("ada")
        s.open("drive")
        names = {}
        for n, b in files.items():
            r = s.pg.request.post(f"{s.base}/api/files?name={n}&as_is=1", data=b,
                                  headers={"content-type": "application/octet-stream"})
            assert r.status < 300, f"upload {n}: {r.status} {r.text()[:200]}"
            names[r.json()["id"]] = n
        s.hook("home:refresh")
        time.sleep(3)

        def thumbs():
            fs = s.snapshot("home.files") or []
            return {f["name"]: f.get("thumb") for f in fs if f["id"] in names}

        try:
            s.wait_for(lambda: len(thumbs()) == 3 and all(v == "ready" for v in thumbs().values()), secs=90)
        except Exception as e:
            print("wait:", e)
        t = thumbs()
        print(br, "tiles", json.dumps(t))
        ok &= len(t) == 3 and all(v == "ready" for v in t.values())
        s.shot(f"{br}-grid")
        for fid, n in names.items():
            s.hook("home:preview", id=fid)
            try:
                s.wait_for(lambda: str((s.snapshot("home.preview") or {}).get("content", "")).startswith(("image", "none")), secs=60)
            except Exception as e:
                print("wait:", e)
            c = (s.snapshot("home.preview") or {}).get("content")
            print(br, "viewer", n, c)
            ok &= str(c).startswith("image")
            if n.endswith(".tif"):
                s.shot(f"{br}-viewer-tif")
            s.hook("home:preview:close")
            time.sleep(1)
    print(br, "PASS" if ok else "FAIL")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    if "--remote" in sys.argv:
        remote(sys.argv)
    sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
    import starbase_ui
    a = sys.argv[1:]
    shots = a[a.index("--shots") + 1] if "--shots" in a else None
    if shots:
        i = a.index("--shots")
        a = a[:i] + a[i + 2:]
    sys.exit(starbase_ui.ship_and_run(__file__, a, rd="/tmp/drive-formats", pyname="p.py", shots_to=shots))
