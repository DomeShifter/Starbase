"""Seed a Starbase server with a fixture set and wait until Photos has indexed it.

    python3 -I testing/seed.py --url $URL --set gps [--user U --password P] [--folder NAME]

--url/--user/--password also come from STARBASE_URL / STARBASE_USER / STARBASE_PASSWORD.
Sets: dates, gps, shapes, types, docs. Signs in (POST api/login), creates the folder
(POST api/folders), uploads each file (POST api/files?as_is=1, keeps the name), and for picture sets enables Photos
(POST api/photos/enable) and polls GET api/photos?folder=... until every file's state is
"ready" (or "unsupported"/"failed", which are reported). Fails naming the files still
pending after --timeout seconds. Prints the folder path and how long indexing took.
"""
import argparse
import http.cookiejar
import json
import os
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fixtures  # noqa: E402


class Client:
    def __init__(self, base):
        self.base = base.rstrip("/") + "/"
        self.op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

    def call(self, method, path, body=None, ctype=None):
        data = body if isinstance(body, bytes) else (json.dumps(body).encode() if body is not None else None)
        req = urllib.request.Request(self.base + path, data=data, method=method)
        if data is not None:
            req.add_header("Content-Type", ctype or "application/json")
        try:
            with self.op.open(req, timeout=60) as r:
                raw = r.read()
        except urllib.error.HTTPError as e:
            sys.exit(f"FAIL {method} {path}: HTTP {e.code} {e.read()[:200]!r}")
        try:
            return json.loads(raw) if raw else {}
        except ValueError:
            return {}


def q(**kw):
    return urllib.parse.urlencode(kw)


def main():
    env = os.environ.get
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default=env("STARBASE_URL"))
    ap.add_argument("--user", default=env("STARBASE_USER", "ada"))
    ap.add_argument("--password", default=env("STARBASE_PASSWORD"))
    ap.add_argument("--set", required=True, choices=list(fixtures.SETS) + ["docs"])
    ap.add_argument("--folder", help="folder to create (default: seed-<set>)")
    ap.add_argument("--timeout", type=int, default=120)
    a = ap.parse_args()
    if not a.url or a.password is None:
        sys.exit("need --url and --password (or STARBASE_URL / STARBASE_PASSWORD)")

    c = Client(a.url)
    c.call("POST", "api/login", {"username": a.user, "password": a.password})
    name = a.folder or f"seed-{a.set}"
    made = c.call("POST", "api/folders?" + q(name=name))
    folder = made.get("path") or name  # an existing folder may answer without a path

    with tempfile.TemporaryDirectory() as tmp:
        names = fixtures.docs(tmp) if a.set == "docs" else fixtures.photos(tmp, a.set)
        for n in names:
            with open(os.path.join(tmp, n), "rb") as f:
                c.call("POST", "api/files?" + q(folder=folder, name=n, as_is=1), f.read(), "application/octet-stream")
    print(f"uploaded {len(names)} files to {folder!r}")
    if a.set == "docs":
        return

    c.call("POST", "api/photos/enable?" + q(folder=folder))
    t0 = time.time()
    while True:
        listing = c.call("GET", "api/photos?" + q(folder=folder, sub=1))
        rows = {p["name"]: p for p in listing.get("photos", [])}
        wait = [n for n in names if n not in rows or rows[n]["state"] == "pending"]
        if not wait:
            break
        if time.time() - t0 > a.timeout:
            sys.exit(f"FAIL: still unindexed after {a.timeout}s: {', '.join(wait)}")
        time.sleep(1)
    bad = {n: (rows[n]["state"], rows[n].get("error")) for n in names if rows[n]["state"] != "ready"}
    print(f"indexed {len(names)} files in {time.time() - t0:.1f}s; folder={folder!r}")
    if bad:
        print("not ready:", bad)
    print("with taken:", sum(1 for n in names if rows[n].get("taken")),
          "with gps:", sum(1 for n in names if rows[n].get("gps")))


if __name__ == "__main__":
    main()
