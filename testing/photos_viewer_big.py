#!/usr/bin/env python3
# Last updated: 2026-11-27T06:50:00Z
"""Real-input check: the big-picture (8192x6144, 50 MP, over the 96 MB texture budget) viewer must not flicker.
Launch: photos_viewer_big.py http://<host>:<port> --browser chromium|firefox --pics DIR [--user ada]
DIR holds big1.jpg, big2.jpg (8192x6144) and jemison1.jpg (a portrait). Opens big1 from the grid and watches 10 s:
textures.requests (hook counter, this build only) and the network image requests stay flat. Then open/close,
next/prev between big1 and big2. Prints counts; PASS needs requests flat and texture bytes within the budget."""
import glob, os, sys, time, urllib.parse
sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
import starbase_ui as ui

def launcher():
    a = sys.argv[1:]; pics = a[a.index("--pics") + 1]; rest = [x for x in a if x not in ("--pics", pics)]
    extra = {os.path.basename(f): open(f, "rb").read() for f in sorted(glob.glob(pics + "/*.jpg"))}
    sys.exit(ui.ship_and_run(os.path.abspath(__file__), rest, "/tmp/pbig", pyname="p.py", extra=extra,
                             shots_to=os.path.expanduser("~/.cache/starbase-shots/vw")))

def main():
    a = sys.argv[1:]; a.remove("--remote"); base = a[0]; br = a[a.index("--browser") + 1] if "--browser" in a else "chromium"
    user = a[a.index("--user") + 1] if "--user" in a else "ada"
    res = []
    def rec(n, ok, d=""): res.append(ok); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base); pg = S.pg; S.sign_in(user)
    q = lambda x: urllib.parse.quote(x, safe="")
    folder = pg.request.post(f"{S.base}/api/folders?path={q(S.user)}&name=Big%20{br}{int(time.time())}").json()["path"]
    for f in ("big1.jpg", "big2.jpg"):
        r = pg.request.post(f"{S.base}/api/files?name={q(f)}&as_is=1&folder={q(folder)}", data=open(f, "rb").read(), timeout=300000)
        print("upload", f, r.status, flush=True)
    S.open("drive", path=f"/?test&dir={q(folder)}"); time.sleep(2)
    S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:enable")
    items = lambda: S.snapshot("home.photos.items") or []
    S.wait_for(lambda: len(items()) >= 2 and all(i.get("state", "ready") == "ready" for i in items()), secs=240, every=3)
    print("items", len(items()), flush=True)
    tx = lambda: S.snapshot("home.photos.textures") or {}
    reqs = []
    pg.on("request", lambda r: reqs.append(r.url) if "api/photos/" in r.url and "/thumb/" in r.url or "/original" in r.url or "/orig" in r.url else None)
    S.hook("home:photos:tab", tab="photos"); S.hook("home:photos:open", index=0); time.sleep(4)
    r0, n0 = tx().get("requests"), len(reqs); time.sleep(10); r1, n1 = tx().get("requests"), len(reqs)
    S.shot(f"{br}-big-open")
    print("textures", tx(), flush=True)
    rec("open from grid: counter flat 10 s", r0 is not None and r1 == r0, f"hook {r0}->{r1}; network {n0}->{n1}")
    rec("network image requests flat 10 s", n1 == n0, f"{n0}->{n1}")
    rec("texture bytes within budget", tx().get("bytes", 0) <= tx().get("budget", 1) * 1.01, f"{tx().get('bytes')}/{tx().get('budget')}")
    for i in range(3):  # open/close, next/prev between big pictures
        S.key("Escape"); time.sleep(1.5)
        S.hook("home:photos:open", index=i % 2); time.sleep(4)
        for k in ("ArrowRight", "ArrowLeft"): S.key(k); time.sleep(3)
    S.key("Escape"); time.sleep(3)
    t = tx(); print("after churn", t, flush=True)
    rec("closed: textures freed", t.get("asking", 0) == 0 and t.get("bytes", 0) <= t.get("budget", 1) * 0.5, str(t))
    S.hook("home:photos:open", index=0); time.sleep(3); a0 = tx().get("requests"); time.sleep(8)
    rec("reopen: flat again", tx().get("requests") == a0, f"{a0}->{tx().get('requests')}")
    S.close(); sys.exit(0 if all(res) else 1)

if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
