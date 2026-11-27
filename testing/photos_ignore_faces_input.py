#!/usr/bin/env python3
# Last updated: 2026-11-27T08:15:36Z
"""Real-input check of PR photos-ignore-faces (People: Ignore / Not a face, every way back).
  photos_ignore_faces_input.py http://<host>:<port> --browser chromium|firefox --pics DIR_WITH_JPGS [--user NAME]
Pictures as for photos_faces_polish.py (<person><n>.jpg, >= 2 people with >= 2 pictures each). Server runs with
--features ml, the user owns it. Everything is clicked with the mouse (right-click menus, buttons); the API is used
only to upload, turn faces on and read the result. Shots in ./shots."""
import json, os, sys, time, glob, urllib.parse
sys.path.insert(0, os.path.expanduser("~/.claude/tools")); sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import starbase_ui as ui
RD = "/tmp/photos-ignore-faces"

def launcher():
    a = sys.argv[1:]; pics = a[a.index("--pics") + 1]; rest = [x for x in a if x not in ("--pics", pics)]
    extra = {os.path.basename(f): open(f, "rb").read() for f in sorted(glob.glob(pics + "/*.jpg"))}
    sys.exit(ui.ship_and_run(os.path.abspath(__file__), rest, RD, pyname="p.py", extra=extra, shots_to=os.path.join(os.getcwd(), "shots")))

def main():
    a = sys.argv[1:]; a.remove("--remote"); base = a[0]; br = a[a.index("--browser") + 1] if "--browser" in a else "chromium"
    res = []
    def rec(n, ok, d=""): res.append(bool(ok)); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base); pg = S.pg; S.sign_in(a[a.index("--user") + 1] if "--user" in a else "ada")
    q = lambda x: urllib.parse.quote(x, safe="")
    folder = pg.request.post(f"{S.base}/api/folders?path={S.user}&name=Ignore%20{br}{int(time.time())}").json()["path"]
    for f in sorted(glob.glob("*.jpg")):
        pg.request.post(f"{S.base}/api/files?name={q(f)}&as_is=1&folder={q(folder)}", data=open(f, "rb").read())
    st = lambda: pg.request.get(f"{S.base}/api/photos/faces").json()
    if st().get("missing"):
        pg.request.post(f"{S.base}/api/photos/faces/models", data="{}"); S.wait_for(lambda: not st().get("missing"), secs=600, every=5)
    pg.request.post(f"{S.base}/api/photos/faces", data=json.dumps({"on": True}), headers={"Content-Type": "application/json"})
    S.open("drive", path=f"/?test&dir={q(folder)}"); time.sleep(2)
    S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:enable"); time.sleep(8); S.hook("home:photos:tab", tab="people")
    pe = lambda: S.snapshot("home.photos.people") or {}
    tg = lambda: S.snapshot("targets") or {}
    # People loads its cards only when opened: re-click the tab every ~20 s while waiting for faces.
    tick = [0]
    def have():
        tick[0] += 1
        if tick[0] % 4 == 0: S.hook("home:photos:tab", tab="people")
        return len(pe().get("groups", [])) + len(pe().get("named", [])) >= 2
    S.wait_for(have, secs=300, every=5)
    def click(name, right=False):
        if not S.wait_for(lambda: name in tg(), secs=20): print("NO TARGET", name, sorted(tg())[:40], flush=True)
        S.click_text(name, button=3 if right else 1)
    def undo_all(prefix, key):
        """Every tile of an Ignored / Not faces section has its own button: press them until the section is empty."""
        for _ in range(30):
            ts = sorted((k for k in tg() if k.startswith(prefix)), key=lambda k: int(k.rsplit(":", 1)[1]))
            if not ts: break
            click(ts[0]); time.sleep(2)
        S.wait_for(lambda: not pe().get(key), secs=20)
    cards = lambda: len(pe()["groups"]) + len(pe()["named"])
    pics = lambda: sorted(c.get("pictures", c.get("count", 0)) for c in pe()["groups"] + pe()["named"])
    n0 = len(pe()["groups"]) + len(pe()["named"])
    # 1. Right-click a group tile -> Ignore group: it leaves the cards, "Ignored (N)" appears.
    click("person:0", right=True); S.shot("1-card-menu"); click("people:ctx-ignore")
    S.wait_for(lambda: pe().get("ignored"), secs=20)
    rec("ignore group", len(pe()["groups"]) + len(pe()["named"]) == n0 - 1 and "people:ignored" in tg(), pe().get("ignored_count")); S.shot("2-ignored")
    # 2. Stop ignoring: back as an unnamed group.
    p0 = pics(); click("person:0", right=True) if False else None
    undo_all("people:unignore:", "ignored")
    rec("stop ignoring: cards back", cards() == n0 and not pe()["ignored"], (cards(), n0))
    # 3. Right-click a group -> Not a face; then "It is a face".
    click("person:0", right=True); click("people:ctx-notface"); S.wait_for(lambda: pe().get("notfaces"), secs=20)
    rec("not a face", "people:notfaces" in tg(), pe().get("notfaces_count")); S.shot("3-notfaces")
    undo_all("people:isface:", "notfaces"); rec("it is a face: cards back", cards() == n0 and not pe()["notfaces"], (cards(), n0))
    # 4. A face on a group page: right-click -> Ignore face; then More -> Not a face for the rest (the page drops the ignored face).
    click("person:0"); S.wait_for(lambda: pe().get("faces", 0) > 0, secs=20); nf0 = pe()["faces"]
    click("face:0", right=True); S.shot("4-face-menu"); click("people:ctx-ignore"); time.sleep(2)
    rec("page drops the ignored face", pe().get("faces") == nf0 - 1, (nf0, pe().get("faces")))
    click("people:more"); S.shot("4c-more"); click("people:ctx-notface"); time.sleep(3)
    rec("face ignore + More not a face", pe().get("ignored") and pe().get("notfaces"), (pe().get("ignored"), pe().get("notfaces")))
    S.shot("4d-sections")
    # 5. One tile per group; open the ignored tile and undo ONE face; then group the rest again.
    rec("one tile per group", pe()["ignored_tiles"] == [1] and len(pe()["notface_tiles"]) == 1, (pe()["ignored_tiles"], pe()["notface_tiles"]))
    click("people:ignored-tile:0"); S.wait_for(lambda: pe().get("marked_open"), secs=10)
    click("people:marked-undo:0"); S.wait_for(lambda: not pe().get("ignored"), secs=20)
    rec("per-face undo (ignored)", not pe()["ignored"] and cards() == n0 - 1, (cards(), n0))
    # 6. Not faces -> Assign to... the first person/group.
    click("people:nf-assign:0"); S.wait_for(lambda: pe().get("assigning") is not None, secs=10); S.shot("6-assign-picker"); click("people:assign-to:0")
    S.wait_for(lambda: not pe().get("notfaces"), secs=20); rec("assign not-face to", not pe()["notfaces"]); c1 = cards()
    # 7. Ignore a whole group, then Assign to... another one: the ignored group joins it.
    click("person:0", right=True); click("people:ctx-ignore"); S.wait_for(lambda: pe().get("ignored"), secs=20)
    click("people:assign:0"); S.wait_for(lambda: pe().get("assigning") is not None, secs=10); click("people:assign-to:0")
    S.wait_for(lambda: not pe().get("ignored"), secs=20); rec("assign ignored group to", not pe()["ignored"] and cards() == c1 - 1, (cards(), c1))
    # 8. Scan now must not bring back a not-face.
    click("person:0", right=True); click("people:ctx-notface"); S.wait_for(lambda: pe().get("notfaces"), secs=20); nfc = pe()["notfaces_count"]
    click("photos:scan-now"); time.sleep(15); S.hook("home:photos:tab", tab="people"); time.sleep(5)
    rec("scan now keeps the not-face", pe()["notfaces_count"] == nfc and not pe()["ignored"], (nfc, pe()["notfaces_count"])); S.shot("8-after-scan")
    # 9. An ignored face is not listed in the viewer's People (API behind the Info panel).
    click("person:0", right=True); click("people:ctx-ignore"); S.wait_for(lambda: pe().get("ignored"), secs=20)
    api = pg.request.get(f"{S.base}/api/photos/people").json(); ig = api["ignored"][0]; pic = ig.get("pic") or ig.get("thumb") or ig.get("id")
    info = pg.request.get(f"{S.base}/api/photos/faces/{pic}").json()
    rec("ignored face not in the photo's People", info.get("people") == [], (pic, info))
    n_in = len(info.get("people", [])) + info.get("unnamed", 0); rec("info counts exclude ignored", n_in == 0, info)
    print("RESULT", "PASS" if all(res) else "FAIL", f"{sum(res)}/{len(res)}"); sys.exit(0 if all(res) else 1)

if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
