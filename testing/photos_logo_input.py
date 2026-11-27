#!/usr/bin/env python3
# Last updated: 2026-11-27T07:00:00Z
"""Real-input check of the Starbase Photos logo (photos-logo PR).
Seed first: python3 -I testing/seed.py --url <URL> --set gps   (into ada/seed-gps).
Run:  photos_logo_input.py http://<host>:<port> --browser chromium|firefox [--user NAME]
Expects: in the Drive the tab is "Starbase Drive" with stardrive.svg; Photo Mode (home:photos on) shows the rose-pink
Starbase Photos logo (target photo-logo, 32 x 32) before the "Photo Mode" title, and the tab becomes "Starbase Photos"
with starphotos.svg; the View menu "Light theme" row switches the theme (logo stays), "Back to Drive" restores the
Drive title and icon. Screenshots: <browser>-logo-0-drive, -1-dark, -2-light (look at the logo: rose square, white card, star)."""
import os, sys, time
sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
import starbase_ui as ui


def launcher():
    sys.exit(ui.ship_and_run(os.path.abspath(__file__), sys.argv[1:], "/tmp/plg", pyname="p.py", shots_to=os.path.join(os.getcwd(), "shots")))


def main():
    a = sys.argv[1:]; a.remove("--remote"); base = a[0]; br = a[a.index("--browser") + 1] if "--browser" in a else "chromium"
    res = []
    def rec(n, ok, d=""): res.append(ok); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base); S.sign_in(a[a.index("--user") + 1] if "--user" in a else "ada")
    S.open("drive"); S.hook("home:folder", path="ada/seed-gps"); time.sleep(2)
    tab = lambda: S.pg.evaluate("[document.title, document.getElementById('tab_icon_svg').getAttribute('href')]")
    tg = lambda: S.snapshot("targets") or {}
    rec("Drive tab", tab() == ["Starbase Drive", "stardrive.svg"], str(tab()))
    S.shot(f"{br}-logo-0-drive")
    S.hook("home:photos", on=True); time.sleep(2.5)
    rec("logo target in Photo Mode", "photo-logo" in tg(), str(tg().get("photo-logo")))
    rec("tab says Starbase Photos with its icon", tab() == ["Starbase Photos", "starphotos.svg"], str(tab()))
    S.shot(f"{br}-logo-1-dark")
    S.click_text("photos:view-menu"); time.sleep(1); S.click_text("Light theme"); S.key("Escape"); time.sleep(1.5)
    S.shot(f"{br}-logo-2-light")
    S.click_text("photo-exit"); time.sleep(2)
    rec("back to Drive tab", tab() == ["Starbase Drive", "stardrive.svg"], str(tab()))
    print("RESULT", "PASS" if all(res) else "FAIL"); sys.exit(0 if all(res) else 1)


if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
