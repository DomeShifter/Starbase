#!/usr/bin/env python3
# Last updated: 2026-11-27T09:00:00Z
"""Big test photos for photos_screen_copy.py (needs Pillow): bigland.jpg 8192x6144 and bigrot.jpg
6000x4000 stored with EXIF Orientation 6 (shows as 4000x6000 portrait). Both have a red top-left
block, green top-right, blue bottom-left, grey bottom-right + a diagonal, so rotation and colours
are visible. Run: python3 photos_screen_copy_data.py --url U --password P [--user ada] [--folder ada/seed-gps]"""
import argparse, os, sys, tempfile, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from seed import Client, q
from PIL import Image, ImageDraw


def make(path, w, h, orient):
    im = Image.new("RGB", (w, h), (235, 235, 235)); d = ImageDraw.Draw(im)
    d.rectangle([0, 0, w // 4, h // 4], fill=(220, 20, 20)); d.rectangle([3 * w // 4, 0, w, h // 4], fill=(20, 180, 20))
    d.rectangle([0, 3 * h // 4, w // 4, h], fill=(20, 20, 220)); d.rectangle([3 * w // 4, 3 * h // 4, w, h], fill=(120, 120, 120))
    for i in range(0, w, w // 64): d.line([i, 0, w - i, h], fill=(0, 0, 0), width=max(2, w // 2000))
    d.text((w // 3, h // 2), "TOP-LEFT IS RED", fill=(0, 0, 0))
    ex = Image.Exif(); ex[0x0112] = orient
    im.save(path, "JPEG", quality=88, exif=ex)


def main():
    ap = argparse.ArgumentParser(); ap.add_argument("--url"); ap.add_argument("--user", default="ada")
    ap.add_argument("--password"); ap.add_argument("--folder", default="ada/seed-gps"); a = ap.parse_args()
    c = Client(a.url); c.call("POST", "api/login", {"username": a.user, "password": a.password})
    with tempfile.TemporaryDirectory() as tmp:
        for n, w, h, o in (("bigland.jpg", 8192, 6144, 1), ("bigrot.jpg", 6000, 4000, 6)):
            p = os.path.join(tmp, n); make(p, w, h, o)
            with open(p, "rb") as f:
                c.call("POST", "api/files?" + q(folder=a.folder, name=n, as_is=1), f.read(), "application/octet-stream")
    t0 = time.time()
    while time.time() - t0 < 180:
        rows = {p["name"]: p for p in c.call("GET", "api/photos?" + q(folder=a.folder, sub=1)).get("photos", [])}
        if all(rows.get(n, {}).get("state") == "ready" for n in ("bigland.jpg", "bigrot.jpg")): print("big photos ready"); return
        time.sleep(1)
    sys.exit("FAIL: big photos not indexed")


if __name__ == "__main__":
    main()
