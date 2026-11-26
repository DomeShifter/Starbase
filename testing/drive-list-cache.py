#!/usr/bin/env python3
"""Real-input check: the Drive file list / storage total are cached per person, never stale.

No UI change, so this drives the same HTTP calls the Drive makes (stdlib only, runs anywhere):
    python3 -I testing/drive-list-cache.py <base-url> <user> <password> [<second-user> <password>]

Steps (each prints PASS/FAIL, exit 1 on any FAIL):
 1. GET /api/files x10 and GET /api/usage x10: prints the timings (first = miss, rest = hits;
    a hit should be under 50 ms).
 2. Upload a file: the very next list shows it; /api/usage counts it.
 3. Rename it: the very next list shows the new name.
 4. Second user (if given): their list never contains the first user's private file.
 5. Delete it: gone from the next list.
"""
import json, sys, time, urllib.request, urllib.parse

base = sys.argv[1].rstrip("/")
fails = 0


def call(method, path, cookie="", body=None):
    req = urllib.request.Request(base + path, data=body, method=method, headers={"Cookie": cookie} if cookie else {})
    t = time.time()
    with urllib.request.urlopen(req, timeout=60) as r:
        data = r.read()
        return r.status, data, (time.time() - t) * 1000, r.headers


def login(user, password):
    req = urllib.request.Request(
        base + "/api/login", data=json.dumps({"username": user, "password": password}).encode(), method="POST",
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        cookies = r.headers.get_all("Set-Cookie") or []
    return "; ".join(c.split(";")[0] for c in cookies)


def check(ok, what):
    global fails
    print(("PASS " if ok else "FAIL ") + what)
    fails += 0 if ok else 1


def names(cookie):
    return [f["name"] for f in json.loads(call("GET", "/api/files", cookie)[1])["files"]]


me = login(sys.argv[2], sys.argv[3])
for path in ("/api/files", "/api/usage"):
    ms = [call("GET", path, me)[2] for _ in range(10)]
    print(f"{path}: first {ms[0]:.0f} ms, hits " + " ".join(f"{m:.0f}" for m in ms[1:]) + f" (median hit {sorted(ms[1:])[4]:.0f} ms)")
    check(sorted(ms[1:])[4] < 50, f"{path} median hit under 50 ms")
used0 = json.loads(call("GET", "/api/usage", me)[1])["files"]
name = f"listcache-{int(time.time())}"
q = urllib.parse.urlencode({"name": name + ".xlsx", "as_is": 1})
st, data, _, _ = call("POST", "/api/files?" + q, me, b"cache test")
fid = json.loads(data)["id"]
check(name + ".xlsx" in names(me), "upload shows in the very next list")
check(json.loads(call("GET", "/api/usage", me)[1])["files"] == used0 + 1, "upload counted in the very next usage")
call("POST", f"/api/files/{fid}/rename?" + urllib.parse.urlencode({"name": name + "-renamed"}), me)
now = names(me)
check(name + "-renamed.xlsx" in now and name + ".xlsx" not in now, "rename shows in the very next list")
if len(sys.argv) >= 6:
    other = login(sys.argv[4], sys.argv[5])
    names(other)
    check(not any(n.startswith(name) for n in names(other)), "second user never sees the private file")
call("DELETE", f"/api/files/{fid}", me)
check(not any(n.startswith(name) for n in names(me)), "delete shows in the very next list")
sys.exit(1 if fails else 0)
