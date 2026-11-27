#!/usr/bin/env python3
# Last updated: 2026-11-27T08:00:00Z
"""Real-input check of the viewer's Info panel details (PR photos-info-details).
Run:  photos_info_input.py http://<host>:<port> --browser chromium|firefox [--user ada --password adapassword] [--light]
Uploads (api, as_is=1) one photo with rich EXIF (camera, lens, GPS + altitude, date with offset, copyright, artist,
description, unique id) and one with none, then checks in the viewer's Info panel: rows in order, empty optional rows
hidden, edits of Title/Caption/Labels/Copyright kept after a reload, original bytes unchanged (sha256), the offline mini
map shows land and water and opens Places, and the text uses house::BODY (checked by the unit test)."""
import base64, hashlib, http.cookiejar, json, os, struct, sys, time, urllib.parse, urllib.request
sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
import starbase_ui as ui

BASE_JPEG = base64.b64decode("/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAoHBwgHBgoICAgLCgoLDhgQDg0NDh0VFhEYIx8lJCIfIiEmKzcvJik0KSEiMEExNDk7Pj4+JS5ESUM8SDc9Pjv/2wBDAQoLCw4NDhwQEBw7KCIoOzs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozs7Ozv/wAARCABAAGADASIAAhEBAxEB/8QAFgABAQEAAAAAAAAAAAAAAAAAAAMG/8QAFRABAQAAAAAAAAAAAAAAAAAAABL/xAAXAQEBAQEAAAAAAAAAAAAAAAAABQYH/8QAFREBAQAAAAAAAAAAAAAAAAAAABT/2gAMAwEAAhEDEQA/AMpRSVFJszrFCtFJUUTFCtFJUUTFCtFJUUTFCtFJUUTFCtFJUUTFCVFJUU0UyDQrRSVFExQrRSVFExQrRSVFExQrRSVFExQrRSVFExQlRSdFNHMg0KUUnRRMUKUUnRRMUKUUnRRMUKUUnRRMUKUUnRRMUJUUlRTRzINCtFJUUTFCtFJUUTFCtFJUUTFCtFJUUTFCtFJUUTFD/9k=")


def launcher():
    sys.exit(ui.ship_and_run(os.path.abspath(__file__), sys.argv[1:], "/tmp/pii", pyname="p.py", shots_to=os.path.expanduser("~/.cache/starbase-shots/ip")))


def _ifd(entries, start):
    entries = sorted(entries); table = 2 + 12 * len(entries) + 4
    out, tail = struct.pack("<H", len(entries)), b""
    for tag, typ, count, data in entries:
        if len(data) <= 4: out += struct.pack("<HHI", tag, typ, count) + data.ljust(4, b"\0")
        else:
            out += struct.pack("<HHII", tag, typ, count, start + table + len(tail)); tail += data + (b"\0" if len(data) % 2 else b"")
    return out + b"\0\0\0\0" + tail


def _a(tag, s): b = s.encode() + b"\0"; return (tag, 2, len(b), b)
def _r(tag, n, d): return (tag, 5, 1, struct.pack("<2I", n, d))
def _s(tag, v): return (tag, 3, 1, struct.pack("<H", v))
def _dms(tag, deg):
    d = abs(deg); w = int(d); m = int((d - w) * 60); s = round(((d - w) * 60 - m) * 60 * 10000)
    return (tag, 5, 3, struct.pack("<6I", w, 1, m, 1, s, 10000))


def rich_jpeg():
    ifd0 = [_a(0x10E, "Sunset over the harbour"), _a(0x10F, "Acme"), _a(0x110, "Pixelator 9"), _a(0x13B, "Ada Example"), _a(0x8298, "(c) Ada Example")]
    exif = [_r(0x829A, 1, 250), _r(0x829D, 18, 10), _s(0x8827, 400), _a(0x9003, "2025:06:14 19:05:33"), _a(0x9011, "+01:00"),
            _r(0x920A, 69, 10), _s(0xA405, 26), _a(0xA420, "UID-0123456789"), _a(0xA434, "Acme 24mm f/1.8")]
    gps = [_a(1, "N"), _dms(2, 51.5074), _a(3, "W"), _dms(4, 0.1278), (5, 1, 1, b"\0"), _r(6, 35, 1)]
    ptr = lambda t: (t, 4, 1, b"\0\0\0\0")
    pos = 8 + len(_ifd(ifd0 + [ptr(0x8769), ptr(0x8825)], 8))
    ifd0 += [(0x8769, 4, 1, struct.pack("<I", pos))]; e = _ifd(exif, pos)
    ifd0 += [(0x8825, 4, 1, struct.pack("<I", pos + len(e)))]
    tiff = b"Exif\0\0II*\0" + struct.pack("<I", 8) + _ifd(ifd0, 8) + e + _ifd(gps, pos + len(e))
    app1 = b"\xff\xe1" + struct.pack(">H", len(tiff) + 2) + tiff
    return BASE_JPEG[:2] + app1 + BASE_JPEG[2:]


def offline_map_colours(page, centre):
    """The London fixture should show land and water around the pin at regional zoom."""
    if not centre:
        return [0, 0]
    x, y = centre
    top = max(0, int(y - 30))
    bottom = min(page.viewport_size["height"], int(y + 30))
    png = page.screenshot(clip={"x": int(x - 120), "y": top, "width": 240, "height": bottom - top})
    return page.evaluate("""async (png) => {
        const bytes = Uint8Array.from(atob(png), c => c.charCodeAt(0));
        const image = await createImageBitmap(new Blob([bytes], {type: 'image/png'}));
        const canvas = document.createElement('canvas');
        canvas.width = image.width;
        canvas.height = image.height;
        const ctx = canvas.getContext('2d');
        ctx.drawImage(image, 0, 0);
        const pixels = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
        const colours = [[0x2b, 0x30, 0x37], [0x17, 0x1a, 0x1f]];
        const counts = [0, 0];
        for (let i = 0; i < pixels.length; i += 4) {
            colours.forEach((rgb, k) => {
                if (rgb.every((v, c) => Math.abs(pixels[i + c] - v) <= 2)) counts[k]++;
            });
        }
        return counts;
    }""", base64.b64encode(png).decode())


class Api:
    def __init__(self, base, user, pw):
        self.base = base.rstrip("/") + "/"; self.op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
        self.call("POST", "api/login", {"username": user, "password": pw})
    def call(self, m, path, body=None, raw=False):
        data = body if isinstance(body, bytes) else (json.dumps(body).encode() if body is not None else None)
        rq = urllib.request.Request(self.base + path, data=data, method=m)
        if data is not None: rq.add_header("Content-Type", "application/octet-stream" if isinstance(body, bytes) else "application/json")
        with self.op.open(rq, timeout=60) as r: b = r.read()
        return b if raw else (json.loads(b) if b else {})


def main():
    a = sys.argv[1:]; a.remove("--remote"); base = a[0]; br = a[a.index("--browser") + 1] if "--browser" in a else "chromium"
    user = a[a.index("--user") + 1] if "--user" in a else "ada"; pw = a[a.index("--password") + 1] if "--password" in a else "adapassword"
    light = "--light" in a; tag = br + ("-light" if light else "")
    res = []
    def rec(n, ok, d=""): res.append(ok); print(("PASS " if ok else "FAIL ") + n, str(d)[:300], flush=True)
    api = Api(base, user, pw); folder = f"info-{br}-{int(time.time())}"
    folder = api.call("POST", "api/folders?" + urllib.parse.urlencode({"name": folder})).get("path") or folder
    files = {"rich.jpg": rich_jpeg(), "plain.jpg": BASE_JPEG}; ids = {}
    for n, b in files.items():
        ids[n] = api.call("POST", "api/files?" + urllib.parse.urlencode({"folder": folder, "name": n, "as_is": 1}), b)["id"]
    api.call("POST", "api/photos/enable?" + urllib.parse.urlencode({"folder": folder}))
    for _ in range(60):
        rows = {p["name"]: p for p in api.call("GET", "api/photos?" + urllib.parse.urlencode({"folder": folder, "sub": 1})).get("photos", [])}
        if len(rows) == 2 and all(p["state"] != "pending" for p in rows.values()): break
        time.sleep(1)
    sha = lambda n: hashlib.sha256(api.call("GET", f"api/files/{ids[n]}", raw=True)).hexdigest()
    before = {n: sha(n) for n in files}
    rec("original bytes served = uploaded", all(before[n] == hashlib.sha256(files[n]).hexdigest() for n in files), before)
    S = ui.Session(br, base=base); S.sign_in(user)
    S.open("drive"); S.hook("home:folder", path=folder); time.sleep(2); S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:enable"); time.sleep(5)
    S.hook("home:photos:tab", tab="photos")
    vw = lambda: S.snapshot("home.photos.viewer") or {}
    tg = lambda: S.snapshot("targets") or {}
    def open_pic(name):
        items = (S.snapshot("home.photos") or {}).get("items") or []
        i = [x.get("name") for x in items].index(name); S.hook("home:photos:open", index=i); time.sleep(3)
        if not vw().get("info"): S.click_text("viewer:info"); time.sleep(3)
    labels = lambda: [r[0] for r in vw().get("info_rows") or []]
    open_pic("rich.jpg"); time.sleep(2)
    rows = {r[0]: r[1] for r in vw().get("info_rows") or []}; print("ROWS", json.dumps(vw().get("info_rows")), flush=True)
    order = ["Name", "Size", "Taken", "Camera", "Exposure", "Lens", "Optics", "Place", "Position", "Altitude"]
    got = [l for l in labels() if l in order]
    rec("rich: rows present, in order", got == [l for l in order if l in got] and {"Camera", "Exposure", "Lens", "Optics", "Position", "Altitude", "Taken", "Time zone"} <= set(labels()), got)
    rec("rich: values", "Acme" in rows.get("Camera", "") and "ISO 400" in rows.get("Exposure", "") and "+01:00" in rows.get("Time zone", "") and "51.5074" in rows.get("Position", "") and "35 m" in rows.get("Altitude", ""), rows)
    S.shot(f"{tag}-info-rich")
    for f, text in (("title", "Harbour trip"), ("caption", "Evening light"), ("labels", "boats, sea"), ("copyright", "(c) Test 2026")):
        S.pg.mouse.move(1100, 500); S.pg.mouse.wheel(0, -5000 if f in ("title", "caption") else 5000); time.sleep(1.2)
        S.click_text(f"viewer:info:edit:{f}"); time.sleep(1); S.pg.keyboard.press("Control+a"); S.pg.keyboard.type(text, delay=20); time.sleep(0.5)
        if f != "caption": S.pg.keyboard.press("Enter")
        time.sleep(1.5)
    S.pg.mouse.move(1100, 500); S.pg.mouse.wheel(0, 3000); time.sleep(1.5); S.shot(f"{tag}-info-rich-bottom")
    t = tg(); rec("sections have targets (people/map)", "viewer:info:map" in t and "photos:info-people" in t, [k for k in t if "info" in k])
    S.shot(f"{tag}-info-edited")
    S.pg.reload(); time.sleep(6); S.hook("home:folder", path=folder); time.sleep(2); S.hook("home:photos", on=True); time.sleep(3); S.hook("home:photos:tab", tab="photos"); time.sleep(3)
    open_pic("rich.jpg"); time.sleep(2); after = " | ".join(str(r) for r in vw().get("info_rows") or []); print("AFTER", after, flush=True)
    rec("edits saved after reload", all(x in after for x in ("Harbour trip", "Evening light", "boats", "(c) Test 2026")), after)
    rec("original bytes unchanged after edits", sha("rich.jpg") == before["rich.jpg"] and sha("plain.jpg") == before["plain.jpg"], "")
    S.pg.mouse.move(1100, 500); S.pg.mouse.wheel(0, 3000); time.sleep(1.5)
    if S.snapshot("home.photos.map.tiles") is None:
        land, water = offline_map_colours(S.pg, tg().get("viewer:info:map"))
        rec("offline mini map shows land and water", land > 100 and water > 100, {"land": land, "water": water})
    S.click_text("viewer:info:map"); time.sleep(4); st = S.snapshot("home.photos") or {}
    m = (S.snapshot("home.photos.map") or {}); c = m.get("centre") or [0, 0]; print("MAP", m.get("centre"), m.get("zoom"), flush=True)
    import math; wx = (-0.1278 + 180) / 360; wy = 0.5 - math.log(math.tan(math.pi / 4 + math.radians(51.5074) / 2)) / (2 * math.pi)  # London in map units
    near = isinstance(c, list) and len(c) == 2 and abs(c[0] - wx) < 0.02 and abs(c[1] - wy) < 0.02
    rec("mini map click opens Places at that spot", not st.get("viewer") and m.get("pictures", 0) >= 1 and near, {"centre": c, "zoom": m.get("zoom")})
    S.hook("home:photos:tab", tab="photos"); time.sleep(2); open_pic("plain.jpg"); time.sleep(2)
    pl = labels(); print("PLAIN", json.dumps(vw().get("info_rows")), flush=True)
    rec("plain: empty optional rows hidden", not ({"Camera", "Lens", "Optics", "Position", "Altitude", "Exposure", "Place"} & set(pl)), pl)
    S.shot(f"{tag}-info-plain")
    S.close(); sys.exit(0 if all(res) else 1)


if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
