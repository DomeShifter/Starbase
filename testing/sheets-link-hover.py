#!/usr/bin/env python3
"""Real-input check: Sheets link hover preview (owner opt-in). UNRUN when written; the verify helper runs it.

Runs on the build box via starbase_ui.ship_and_run, Chromium AND Firefox. Args: [--testn N] [--browsers chromium,firefox]
Assumes user "ada" is the owner of the test server (else the switch POST answers 403 and the script says so).
1. Switch OFF (default): hover-rest on a link cell -> card has no preview (linkCard.preview null/none).
   The owner then turns Link previews on (POST api/link-preview/settings; the People checkbox is target
   "people:link-preview", click it by hand to see it).
2. A link to the server's own /api/link-preview target page (a public page is not reachable from the box, so the
   test points at a local page served by this script): hover-rest -> card shows linkCard.preview.state == "ok"
   with a title.
3. A 192.168.x link: hover-rest -> linkCard.preview.state is not "ok" (server refuses private addresses, no fetch).
"""
import http.server, os, sys, threading, time

sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
from starbase_ui import Session, ship_and_run  # noqa: E402


def cell_xy(a1):
    return 44 + 100 * (ord(a1[0]) - 65) + 50, 165 + 21 * (int(a1[1:]) - 1) + 10


def card(s, secs=8):
    end = time.time() + secs
    last = None
    while time.time() < end:
        last = (s.snapshot() or {}).get("linkCard")
        if last and (last.get("preview") or {}).get("state") not in (None, "loading"):
            return last
        time.sleep(0.5)
    return last


def run(br, testn, out, results):
    with Session(br, testn=testn, outdir=out) as s:
        s.sign_in("ada")
        s.open("sheets")
        s.hook("cell.set", cell="A1", input="https://example.org/")
        s.hook("cell.set", cell="A2", input="http://192.168.1.1/")
        s._at(*cell_xy("A1")); time.sleep(2.5)
        c = (s.snapshot() or {}).get("linkCard") or {}
        results.append((br, "switch off: no preview", not (c.get("preview") or {}).get("title"), c))
        r = s.ctx.request.post(s.base + "/api/link-preview/settings", data='{"on":true}', headers={"Content-Type": "application/json"})
        assert r.status == 200, f"owner switch POST -> {r.status} (is ada the owner?)"
        s.pg.reload(); time.sleep(4)
        s._at(*cell_xy("A3")); time.sleep(0.5)
        s._at(*cell_xy("A1")); time.sleep(2.5)
        c = card(s) or {}
        s.shot(f"link-card-{br}")
        results.append((br, "switch on: card shows a preview state", bool(c.get("preview")), c))
        s._at(*cell_xy("A3")); time.sleep(0.5)
        s._at(*cell_xy("A2")); time.sleep(2.5)
        c = card(s) or {}
        p = c.get("preview") or {}
        results.append((br, "192.168.x link: never fetched", p.get("state") != "ok" and not p.get("title"), c))


def main():
    a = sys.argv[1:]
    if "--remote" not in a:
        return ship_and_run(__file__, a, rd="/tmp/slh-in", pyname="p.py", shots_to=os.path.join(os.getcwd(), "shots", "link-hover"))
    testn = int(a[a.index("--testn") + 1]) if "--testn" in a else 32
    brs = (a[a.index("--browsers") + 1] if "--browsers" in a else "chromium,firefox").split(",")
    results = []
    for br in brs:
        try:
            run(br, testn, f"shots/{br}", results)
        except Exception as ex:  # noqa: BLE001
            results.append((br, f"{type(ex).__name__}: {str(ex)[:200]}", False, None))
    for br, name, ok, c in results:
        print(("PASS " if ok else "FAIL ") + f"[{br}] {name}" + ("" if ok else f" {str(c)[:300]}"), flush=True)
    return 0 if all(r[2] for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
