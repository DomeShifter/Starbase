#!/usr/bin/env python3 -I
"""Real input for the transfer-size job: the server sends brotli, then gzip, then plain.
Usage: python3 -I testing/transfer-size-input.py --url http://HOST:PORT
Needs a site pre-compressed by scripts/precompress.sh. Checks every app's wasm and a font."""
import argparse, re, sys, urllib.error, urllib.request

ap = argparse.ArgumentParser()
ap.add_argument("--url", required=True)
a = ap.parse_args()
base = a.url.rstrip("/")


def get(path, enc, etag=None):
    h = {"Accept-Encoding": enc}
    if etag:
        h["If-None-Match"] = etag
    try:
        r = urllib.request.urlopen(urllib.request.Request(base + path, headers=h))
    except urllib.error.HTTPError as e:
        return e.code, e.headers, b""
    return r.status, r.headers, r.read()


fail = 0


def check(ok, msg):
    global fail
    print(("ok   " if ok else "FAIL ") + msg)
    fail += not ok


for app in ("/", "/docs/", "/slides/"):
    html = get(app, "identity")[2].decode()
    m = re.search(r'["\'(]([^"\'()]*_bg\.wasm)', html)
    if not m:
        check(False, f"{app}: no wasm link found")
        continue
    w = m.group(1)
    path = w if w.startswith("/") else app + w.lstrip("./")
    sizes = {}
    for enc, want in (("br", "br"), ("gzip", "gzip"), ("identity", None)):
        s, h, b = get(path, enc)
        sizes[enc] = len(b)
        check(s == 200 and h.get("Content-Encoding") == want and "Accept-Encoding" in (h.get("Vary") or ""),
              f"{path} {enc}: {s} {h.get('Content-Encoding')} {len(b)} bytes")
    check(sizes["br"] < sizes["gzip"] < sizes["identity"], f"{path}: br < gzip < plain {sizes}")

font = "/Roboto-Regular.ttf"
s, h, _ = get(font, "br")
check(s == 200 and h.get("Content-Type") == "font/ttf" and h.get("Content-Encoding") == "br",
      f"{font}: {s} {h.get('Content-Type')} {h.get('Content-Encoding')}")
tag = h.get("ETag")
check(bool(tag) and get(font, "br", tag)[0] == 304, f"{font}: ETag {tag} answers 304")
sys.exit(1 if fail else 0)
