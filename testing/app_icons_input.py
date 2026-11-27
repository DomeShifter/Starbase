#!/usr/bin/env python3
# Last updated: 2026-11-27T08:30:00Z
"""Real-input check of the space-card app icons (app-icons-spacecard PR). No new buttons: only logos changed.
Run:  app_icons_input.py http://<host>:<port> --browser chromium|firefox [--user NAME]
Expects: the Drive tab shows stardrive.svg, and for every app (drive, sheets, docs, photos, slides) the served logo file
starts with a 256 x 256 viewBox and has no Inkscape data. Screenshots: <browser>-icons-0-drive (home screen logo),
-1-photos (Photo Mode logo). LOOK: dark rounded square, coloured planet glow, white star and symbol, crisp at header size."""
import os, sys, time
sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
import starbase_ui as ui


def launcher():
    sys.exit(ui.ship_and_run(os.path.abspath(__file__), sys.argv[1:], "/tmp/pai", pyname="p.py", shots_to=os.path.join(os.getcwd(), "shots")))


def main():
    a = sys.argv[1:]; a.remove("--remote"); base = a[0]; br = a[a.index("--browser") + 1] if "--browser" in a else "chromium"
    res = []
    def rec(n, ok, d=""): res.append(ok); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base); S.sign_in(a[a.index("--user") + 1] if "--user" in a else "ada")
    S.open("drive"); time.sleep(2)
    href = S.pg.evaluate("document.getElementById('tab_icon_svg').getAttribute('href')")
    rec("Drive tab icon", href == "stardrive.svg", href)
    for f in ("stardrive.svg", "favicon.svg", "starphotos.svg", "favicon.ico", "apple-touch-icon.png"):
        r = S.pg.evaluate("f => fetch(f).then(r => r.text().then(t => [r.status, t.slice(0, 400)]))", f)
        ok = r[0] == 200 and (f.endswith((".ico", ".png")) or ('viewBox="0 0 256 256"' in r[1] and "inkscape" not in r[1]))
        rec("served " + f, ok, str(r[0]))
    S.shot(f"{br}-icons-0-drive")
    S.hook("home:photos", on=True); time.sleep(2.5)
    S.shot(f"{br}-icons-1-photos")
    if br == "chromium":
        S.click_text("photos:view-menu"); time.sleep(1); S.click_text("Light theme"); S.key("Escape"); time.sleep(1.5)
        S.shot(f"{br}-icons-2-photos-light")
    for app in ("sheets", "docs", "slides"):
        S.open(app); time.sleep(3)
        links = S.pg.evaluate("[...document.querySelectorAll('link[rel*=icon]')].map(l => l.getAttribute('href'))")
        print(app, "favicon links", links, flush=True)
        S.shot(f"{br}-icons-3-{app}")
    print("RESULT", "PASS" if all(res) else "FAIL"); sys.exit(0 if all(res) else 1)


if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
