#!/usr/bin/env python3
# Last updated: 2026-11-27T07:30:00Z
"""Real-input check: Photo Mode is always dark, whatever the system theme; Drive stays light.
Seed first: python3 -I testing/seed.py --url <URL> --password adapassword --set gps
Run: photos_always_dark_input.py http://<host>:<port> --browser chromium|firefox [--theme light]
Shots (<browser>-dk-N-*): drive, grid, view-menu (Show/Type/Sort, no Theme), people, viewer, info, back to Drive."""
import os, sys, time
sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
import starbase_ui as ui


def launcher():
    sys.exit(ui.ship_and_run(os.path.abspath(__file__), sys.argv[1:], "/tmp/pdk", pyname="p.py", shots_to=os.path.expanduser("~/.cache/starbase-shots/dk")))


def main():
    a = sys.argv[1:]; a.remove("--remote"); base = a[0]; br = a[a.index("--browser") + 1] if "--browser" in a else "chromium"
    th = a[a.index("--theme") + 1] if "--theme" in a else None
    p = f"{br}{'-' + th if th else ''}-dk"
    res = []
    def rec(n, ok, d=""): res.append(ok); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base, **({"theme": th} if th else {})); S.sign_in("ada")
    S.open("drive"); S.hook("home:folder", path="ada/seed-gps"); time.sleep(2)
    S.shot(f"{p}-0-drive")
    S.hook("home:photos", on=True); time.sleep(2.5); S.hook("home:photos:enable"); time.sleep(5)
    S.hook("home:photos:tab", tab="photos"); time.sleep(3)
    items = (S.snapshot("home.photos") or {}).get("items") or []
    rec("photos in grid", len(items) >= 10, f"count={len(items)}")
    S.shot(f"{p}-1-grid")
    S.click_text("photos:view-menu"); time.sleep(1.5); S.shot(f"{p}-2-view-menu")
    S.pg.keyboard.press("Escape"); time.sleep(1)
    S.hook("home:photos:tab", tab="people"); time.sleep(3); S.shot(f"{p}-3-people")
    S.hook("home:photos:tab", tab="photos"); time.sleep(2)
    S.hook("home:photos:open", index=0); time.sleep(3); S.shot(f"{p}-4-viewer")
    if not (S.snapshot("home.photos.viewer") or {}).get("info"): S.click_text("viewer:info"); time.sleep(3)
    rec("info open", bool((S.snapshot("home.photos.viewer") or {}).get("info")))
    S.shot(f"{p}-5-info")
    S.pg.keyboard.press("Escape"); time.sleep(1)
    for _ in range(3):
        if "photo-exit" not in (S.snapshot("targets") or {}): break
        S.click_text("photo-exit"); time.sleep(2.5)
    rec("left Photo Mode", "photo-exit" not in (S.snapshot("targets") or {}))
    S.shot(f"{p}-6-back")
    print("RESULT", "PASS" if all(res) else "FAIL"); sys.exit(0 if all(res) else 1)


if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
