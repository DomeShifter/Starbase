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
 5. Share with the second user: shows in their very next list; unshare: gone from their very next list.
 6. Env LISTCACHE_EXT_CMD (a shell command that puts a file called $NAME into the first user's
    folder on the server's disk, outside the app): it appears in the list within 6 s.
 7. Delete it: gone from the next list.
 Also prints a before/after table: "miss" = a unique query (cache bypassed, the old speed),
 "hit" = the plain request, 10 each, median and p90.
"""
import json, os, subprocess, sys, time, urllib.request, urllib.parse

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
def p90(v):
    v = sorted(v)
    return v[int(len(v) * 0.9) - 1]


for path in ("/api/files", "/api/usage"):
    miss = [call("GET", f"{path}?nocache={i}{time.time()}", me)[2] for i in range(10)]
    hit = [call("GET", path, me)[2] for _ in range(11)][1:]
    print(f"TABLE {path}: before(miss) median {sorted(miss)[5]:.0f} ms p90 {p90(miss):.0f} ms | after(hit) median {sorted(hit)[5]:.0f} ms p90 {p90(hit):.0f} ms")
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
    jh = {"Content-Type": "application/json", "Cookie": me}
    share = lambda people: urllib.request.urlopen(urllib.request.Request(base + f"/api/files/{fid}/sharing", data=json.dumps({"people": people}).encode(), method="POST", headers=jh), timeout=30).read()
    share({sys.argv[4]: "view"})
    sh = json.loads(call("GET", "/api/files", other)[1])
    check(any(f["name"].startswith(name) for f in sh["files"]) or name in json.dumps(sh), "shared file shows in the second user's very next list")
    share({sys.argv[4]: "remove"})
    check(name not in json.dumps(json.loads(call("GET", "/api/files", other)[1])), "after unshare the second user's very next list has no trace of it")
ext = os.environ.get("LISTCACHE_EXT_CMD")
if ext:
    names(me)
    ename = f"listcache-ext-{int(time.time())}.txt"
    subprocess.run(ext, shell=True, check=True, env={**os.environ, "NAME": ename})
    t0 = time.time()
    while ename not in names(me) and time.time() - t0 < 10:
        time.sleep(0.5)
    check(ename in names(me), f"file added on disk outside the app appears (took {time.time() - t0:.1f} s, limit 6 s)")
    check(time.time() - t0 <= 6, "within 6 s")
call("DELETE", f"/api/files/{fid}", me)
check(not any(n.startswith(name) for n in names(me)), "delete shows in the very next list")
sys.exit(1 if fails else 0)
