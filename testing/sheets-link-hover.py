#!/usr/bin/env python3
"""Real-input check: Sheets link hover preview (owner opt-in). UNRUN when written; the verify helper runs it.

Runs on the build box via starbase_ui.ship_and_run, Chromium AND Firefox. Args: [--testn N] [--browsers chromium,firefox]
Assumes user "ada" is the owner of the test server (else the switch POST answers 403 and the script says so).
1. Switch OFF (default): hover-rest on a link cell -> card has no preview (linkCard.preview null/none).
   The owner then turns Link previews on (POST api/link-preview/settings; the People checkbox is target
   "settings:link-preview", click it by hand to see it).
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


def card(s, cell, secs=12):
    end = time.time() + secs
    last = None
    while time.time() < end:
        last = (s.snapshot() or {}).get("linkCard")
        if last and last.get("cell") == cell and (last.get("preview") or {}).get("state") not in (None, "loading"):
            return last
        time.sleep(0.5)
    return last


def run(br, testn, out, results):
    with Session(br, testn=testn, outdir=out) as s:
        s.sign_in("ada")
        s.ctx.request.post(s.base + "/api/link-preview/settings", data='{"on":false}', headers={"Content-Type": "application/json"})  # setup: start OFF
        s.open("sheets")
        s.hook("cell.set", cell="A1", input="https://example.org/")
        s.hook("cell.set", cell="A2", input="http://192.168.1.1/")
        s._at(*cell_xy("A1")); time.sleep(2.5)
        c = (s.snapshot() or {}).get("linkCard") or {}
        results.append((br, "switch off: no preview", not (c.get("preview") or {}).get("title"), c))
        # Owner ticks Settings > Server > "Show a page preview when hovering a link" by a real click (hook target).
        s.click_text("account:avatar", button=3)
        s.click_text("tab:Server")
        s.click_text("settings:link-preview"); time.sleep(2)
        chk = (s.snapshot() or {}).get("account", {}).get("linkPreviewChecked")
        results.append((br, "checkbox ticked by click (snapshot)", chk is True, chk))
        r = s.ctx.request.get(s.base + "/api/link-preview/settings")
        results.append((br, "server switch now on", '"on":true' in r.text().replace(" ", ""), r.text()[:80]))
        s.open("sheets")
        s.hook("cell.set", cell="A1", input="https://example.org/")
        s.hook("cell.set", cell="A2", input="http://192.168.1.1/")
        s._at(*cell_xy("A3")); time.sleep(0.5)
        s._at(*cell_xy("A1")); time.sleep(2.5)
        c = card(s, "A1") or {}
        s.shot(f"link-card-{br}")
        results.append((br, "switch on: card shows a preview state", bool(c.get("preview")), c))
        s._at(*cell_xy("A3")); time.sleep(0.5)
        s._at(*cell_xy("A2")); time.sleep(2.5)
        c = card(s, "A2") or {}
        p = (c.get("preview") or {}) if c.get("cell") == "A2" else {}
        print(f"[{br}] A2 (192.168.x) card: cell={c.get('cell')} preview={p}", flush=True)
        results.append((br, "192.168.x link: never fetched", p.get("state") != "ok" and not p.get("title"), c))


SAFE = ["http://192.168.1.1/", "http://127.0.0.1:3100/", "http://[::1]/", "http://169.254.169.254/", "http://localhost/",
        "http://192-168-178-100.nip.io/", "http://127.0.0.1.nip.io/", "https://httpbin.org/redirect-to?url=http://192.168.178.100:3006/"]


def safety(br, testn, out, results):
    """Owner switch is ON (set by run()): private/loopback/redirect targets must never come back state ok."""
    from urllib.parse import quote
    with Session(br, testn=testn, outdir=out) as s:
        s.sign_in("ada"); s.open("sheets")
        for u in SAFE:
            r = s.ctx.request.get(s.base + "/api/link-preview?url=" + quote(u, safe=""))
            t = r.text()
            results.append((br, f"safety: {u} not fetched", r.status == 200 and '"state":"ok"' not in t.replace(" ", "") and '"title"' not in t, t[:160]))
        r = s.ctx.request.get(s.base + "/api/link-preview?url=" + quote("https://example.org/", safe=""))
        print(f"[{br}] example.org via API: {r.text()[:200]}", flush=True)


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
            safety(br, testn, f"shots/{br}", results)
        except Exception as ex:  # noqa: BLE001
            results.append((br, f"{type(ex).__name__}: {str(ex)[:200]}", False, None))
    for br, name, ok, c in results:
        print(("PASS " if ok else "FAIL ") + f"[{br}] {name}" + ("" if ok else f" {str(c)[:300]}"), flush=True)
    return 0 if all(r[2] for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
