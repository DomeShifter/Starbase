#!/usr/bin/env python3
"""Real-input check: the Photos frame (one toolbar + View menu, left list of views and tools, selection bar, Info).
  photos_design_frame_input.py http://<host>:<port> --browser chromium|firefox --pics DIR_ON_BOX
Expects (desktop window, light and dark):
 1. Top bar: Upload (photos:upload), View (photos:view-menu), Scan now (photos:scan-now), Slideshow; no second bar of buttons over the grid.
 2. View opens one menu with SHOW / TYPE / SORT / THEME (photos:view:show|type|sort|theme); "Oldest first" sets newestFirst false.
 3. Left list: views Photos, Albums, People, Places, Timeline, Moments; tool Duplicates; no More or Locked row.
 3b. Clicking Timeline, Moments, Duplicates, People, Places opens each; no People strip above the grid.
 4. Ctrl+A: the selection bar shows Add to album, Favourite, Download, Move to bin (photos:sel:*) and More (photos:sel:more).
 5. More > Info opens the viewer with its Info panel (viewer:info-panel), not Drive's Details."""
import glob, os, sys, time, urllib.parse
sys.path.insert(0, os.path.expanduser("~/.claude/tools")); sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import starbase_ui as ui
RD = "/tmp/photos-design-frame"
q = lambda x: urllib.parse.quote(x, safe="")


def launcher():
    d = os.path.join(os.getcwd(), "shots")
    sys.exit(ui.ship_and_run(os.path.abspath(__file__), sys.argv[1:], RD, pyname="p.py", shots_to=d))


def main():
    a = sys.argv[1:]; a.remove("--remote"); base = a[0]
    arg = lambda k, d=None: a[a.index(k) + 1] if k in a else d
    br, pics = arg("--browser", "chromium"), arg("--pics")
    res = []
    def rec(n, ok, d=""): res.append(bool(ok)); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base); pg = S.pg; S.sign_in("ada")
    tg = lambda: S.snapshot("targets") or {}
    tag = f"{br}-{int(time.time())}"
    folder = pg.request.post(f"{S.base}/api/folders?path=ada&name=Frame%20{q(tag)}").json()["path"]
    for f in sorted(glob.glob(os.path.join(pics, "*.jpg")))[:3]:
        pg.request.post(f"{S.base}/api/files?name={q(os.path.basename(f))}&as_is=1&folder={q(folder)}", data=open(f, "rb").read())
    S.open("drive", path=f"/?test&dir={q(folder)}"); time.sleep(2)
    S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:enable"); time.sleep(6)
    S.wait_for(lambda: len(S.snapshot("home.photos.items") or []) >= 1, secs=60, every=2)
    t = tg()
    rec("toolbar has Upload, View, Scan now", all(k in t for k in ["photos:upload", "photos:view-menu", "photos:scan-now"]), sorted(t)[:40])
    rec("old pills gone", not any(k in t for k in ["photo-type", "photo-set", "photo-theme"]))
    S.shot(f"{tag}-1-toolbar")
    S.click_text("photos:view-menu"); time.sleep(1)
    t = tg()
    rec("View menu sections", all(f"photos:view:{s}" in t for s in ["show", "type", "sort", "theme"]), [k for k in t if k.startswith("photos:view")])
    S.shot(f"{tag}-2-view-menu")
    S.click_text("Oldest first"); time.sleep(2)
    rec("Oldest first", S.snapshot("home.photos.newestFirst") is False)
    S.key("Escape")
    rec("left list: views and tools", all(k in tg() for k in ["Photos", "Albums", "People", "Places", "Timeline", "Moments", "Duplicates"]) and not any(k in tg() for k in ["photos:nav:more", "Locked"]))
    rec("People strip is gone", not any(k.startswith("people:strip") or k in ("people:see-all", "pill-See all") for k in tg()))
    S.shot(f"{tag}-3-nav-more")
    for row in ["Timeline", "Moments", "Duplicates", "People", "Places"]:
        S.click_text(row); time.sleep(2)
        tab = "events" if row == "Moments" else row.lower()
        rec(f"{row} opens with its row highlighted", S.snapshot("home.photos.albums.tab") == tab, S.snapshot("home.photos.albums.tab"))
        rec(f"{row}: no People heading / See all above the grid", not any(k.startswith("people:strip") or k in ("people:see-all", "pill-See all") for k in tg()))
    S.click_text("Photos"); time.sleep(1)
    rec("Photos row highlighted", S.snapshot("home.photos.albums.tab") == "photos")
    S.shot(f"{tag}-3b-photos-main")
    S.click_xy(700, 400); S.key("Control+a"); time.sleep(1)
    t = tg()
    rec("selection bar: four + More", all(f"photos:sel:{k}" in t for k in ["add-to-album", "favourite", "download", "move-to-bin", "more"]))
    S.shot(f"{tag}-4-selection")
    S.click_text("photos:sel:more"); time.sleep(1); S.click_text("Info"); time.sleep(2)
    rec("Info opens the viewer panel", "viewer:info-panel" in tg())
    S.shot(f"{tag}-5-info")
    sys.exit(0 if all(res) else 1)


if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
