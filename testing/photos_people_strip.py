#!/usr/bin/env python3
# Last updated: 2026-11-26T20:00:00Z
"""Real-input check of the People strip, the one-step turn-on and the progress bar (not yet run).
Run on a test server whose folder has faces seeded (testing/seed.py --set people, server built with --features ml):
  photos_people_strip.py http://<host>:<port> --browser chromium|firefox [--user NAME]
Expects: Photos on in the folder; People tab says 'Turn on face recognition' or 'Download face models and turn on';
after one click the bar 'Finding faces' shows and finishes; back on the Photos tab the strip shows faces + 'See all';
a click on a strip face opens that person's page."""
import os, sys, time
sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
import starbase_ui as ui

def main():
    a = sys.argv[1:]; base = a[0]; br = a[a.index("--browser") + 1] if "--browser" in a else "chromium"
    res = []
    def rec(n, ok, d=""): res.append(ok); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base); S.sign_in(a[a.index("--user") + 1] if "--user" in a else "ada")
    S.open("drive"); S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:enable"); time.sleep(6)
    S.hook("home:photos:tab", tab="people"); time.sleep(2)
    pe = lambda: S.snapshot("home.photos.people") or {}
    tg = lambda: S.snapshot("targets") or {}
    S.shot(f"{br}-strip-0-off")
    label = next((k for k in tg() if k.startswith("pill-") and ("turn on" in k.lower())), None)
    rec("one turn-on button", label is not None, str(label))
    if label: S.click_text(label)
    rec("bar shown or already done", S.wait_for(lambda: "people:progress" in tg() or pe().get("on") is True, secs=60))
    S.shot(f"{br}-strip-1-bar")
    rec("scan finishes", S.wait_for(lambda: (pe().get("progress") or {}).get("done") == (pe().get("progress") or {}).get("all") and pe().get("named", []) + pe().get("groups", []), secs=420, every=5))
    S.hook("home:photos:tab", tab="photos"); time.sleep(3)
    S.shot(f"{br}-strip-2-photos")
    rec("strip faces + See all", "people-strip:0" in tg() and "pill-See all" in tg())
    S.click_text("people-strip:0"); time.sleep(3)
    rec("person page opened", "people:back" in tg())
    S.shot(f"{br}-strip-3-person")
    S.close(); sys.exit(0 if all(res) else 1)

if __name__ == "__main__": main()
