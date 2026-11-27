#!/usr/bin/env python3
"""Looks + toolbar actions for the Photos frame. Run on the branch server (NEW) and on main's server (--base: shots only).
  photos_design_frame_shots.py http://<host>:<port> --browser chromium|firefox --pics DIR_ON_BOX [--base]
Shots: toolbar, nav, selection, selection+More, Albums (empty), view menu. NEW also clicks Upload (file chooser) and Scan now."""
import glob, os, sys, time, urllib.parse
sys.path.insert(0, os.path.expanduser("~/.claude/tools")); sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import starbase_ui as ui
RD = "/tmp/photos-design-frame-shots"
q = lambda x: urllib.parse.quote(x, safe="")


def launcher():
    sys.exit(ui.ship_and_run(os.path.abspath(__file__), sys.argv[1:], RD, pyname="p.py", shots_to=os.path.join(os.getcwd(), "shots")))


def main():
    a = sys.argv[1:]; a.remove("--remote"); base = a[0]; old = "--base" in a
    arg = lambda k, d=None: a[a.index(k) + 1] if k in a else d
    br, pics = arg("--browser", "chromium"), arg("--pics")
    res = []
    def rec(n, ok, d=""): res.append(bool(ok)); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base); pg = S.pg; S.sign_in("ada")
    tg = lambda: S.snapshot("targets") or {}
    tag = f"{br}-{'base' if old else 'new'}"
    folder = pg.request.post(f"{S.base}/api/folders?path=ada&name=Shots%20{q(tag)}%20{int(time.time())}").json()["path"]
    for f in sorted(glob.glob(os.path.join(pics, "*.jpg")))[:3]:
        pg.request.post(f"{S.base}/api/files?name={q(os.path.basename(f))}&as_is=1&folder={q(folder)}", data=open(f, "rb").read())
    S.open("drive", path=f"/?test&dir={q(folder)}"); time.sleep(2)
    S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:enable"); time.sleep(6)
    S.wait_for(lambda: len(S.snapshot("home.photos.items") or []) >= 3, secs=60, every=2)
    S.shot(f"{tag}-a-toolbar")
    S.click_xy(700, 400); S.key("Control+a"); time.sleep(1)
    S.shot(f"{tag}-b-selection")
    if not old:
        S.click_text("photos:sel:more"); time.sleep(1); S.shot(f"{tag}-c-selection-more"); S.key("Escape")
    S.key("Escape")
    S.click_text("Albums"); time.sleep(2); S.shot(f"{tag}-d-albums-empty")
    S.click_text("Photos"); time.sleep(1)
    if not old:
        n0 = len(S.snapshot("home.photos.items") or [])
        S.upload(sorted(glob.glob(os.path.join(pics, "*.jpg")))[3:4] or sorted(glob.glob(os.path.join(pics, "*.jpg")))[:1], target="photos:upload")
        S.click_text("photos:scan-now"); time.sleep(3)
        S.wait_for(lambda: len(S.snapshot("home.photos.items") or []) >= n0 + 1, secs=60, every=2)
        rec("Upload (toolbar chooser) then Scan now: the picture is in Photos", len(S.snapshot("home.photos.items") or []) >= n0 + 1)
        S.click_text("photos:view-menu"); time.sleep(1); S.shot(f"{tag}-f-view-menu"); S.key("Escape")
        S.click_xy(1000, 650); S.key("Control+a"); time.sleep(1); S.click_text("photos:sel:more"); time.sleep(1)
        S.shot(f"{tag}-g-light-selection-more")
    sys.exit(0 if all(res) else 1)


if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
