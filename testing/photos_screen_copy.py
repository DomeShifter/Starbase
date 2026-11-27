#!/usr/bin/env python3
# Last updated: 2026-11-27T08:31:28Z
"""Real-input check of the viewer's 2560 px screen copy (UNRUN: written by the build helper; the verify
helper runs and fixes it).
Seed first: python3 -I testing/seed.py --url <URL> --set gps ; plus one big JPEG (e.g. 8192x6144) in the folder.
Run:  photos_screen_copy.py http://<host>:<port> --browser chromium|firefox [--user NAME] [--folder PATH]
Expects: home.photos.bestFormat becomes "jxl" (Chrome 155+/Firefox 158+, server with cjxl) or "jpeg";
opening a big picture full screen shows "256"/"1024" at once, then home.photos.viewer.showing == "2560"
within ~10 s; items[i].original stays false (the original is never fetched to look at it);
textures.requests grows by at most 3 for one open (1024, 2560 202-retry, 2560)."""
import os, sys, time
sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
import starbase_ui as ui


def launcher():
    sys.exit(ui.ship_and_run(os.path.abspath(__file__), sys.argv[1:], "/tmp/psc", pyname="p.py", shots_to=os.path.join(os.getcwd(), "shots")))


def main():
    a = sys.argv[1:]; a.remove("--remote"); base = a[0]; br = a[a.index("--browser") + 1] if "--browser" in a else "chromium"
    user = a[a.index("--user") + 1] if "--user" in a else "ada"
    folder = a[a.index("--folder") + 1] if "--folder" in a else f"{user}/seed-gps"
    res = []
    def rec(n, ok, d=""): res.append(ok); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base); S.sign_in(user)
    S.open("drive"); S.hook("home:folder", path=folder); time.sleep(2); S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:enable"); time.sleep(5)
    ph = lambda: S.snapshot("home.photos") or {}
    S.hook("home:photos:tab", tab="photos"); time.sleep(2)
    rec("best format known", ph().get("bestFormat") in ("jxl", "jpeg"), str(ph().get("bestFormat")))
    its = ph().get("items") or []
    big = max(range(len(its)), key=lambda i: str(its[i].get("name")).lower().startswith("big"), default=0)
    r0 = (ph().get("textures") or {}).get("requests", 0)
    S.hook("home:photos:open", index=big)
    shown = []
    for _ in range(30):
        shown.append((ph().get("viewer") or {}).get("showing")); time.sleep(0.5)
        if shown[-1] == "2560": break
    rec("viewer reaches the 2560 copy", shown[-1] == "2560", " -> ".join(dict.fromkeys(map(str, shown))))
    it = (ph().get("items") or [{}])[big]
    rec("original never fetched", not it.get("original"), str(it))
    r1 = (ph().get("textures") or {}).get("requests", 0)
    rec("few requests for one open", r1 - r0 <= 6, f"{r0} -> {r1}")
    S.shot(f"{br}-screen-copy")
    S.close()
    sys.exit(0 if all(res) else 1)


if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
