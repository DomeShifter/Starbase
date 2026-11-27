#!/usr/bin/env python3
# Last updated: 2026-11-27T07:30:14Z
"""Real-input check of PR photos-ignore-faces (People: Ignore / Not a face, every way back).
  photos_ignore_faces_input.py http://<host>:<port> --browser chromium|firefox --pics DIR_WITH_JPGS [--user NAME]
Pictures as for photos_faces_polish.py (<person><n>.jpg, >= 2 people with >= 2 pictures each). Server runs with
--features ml, the user owns it. Everything is clicked with the mouse (right-click menus, buttons); the API is used
only to upload, turn faces on and read the result. Shots in ./shots (light theme)."""
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
    S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:tab", tab="people")
    pe = lambda: S.snapshot("home.photos.people") or {}
    tg = lambda: S.snapshot("targets") or {}
    S.wait_for(lambda: len(pe().get("groups", [])) + len(pe().get("named", [])) >= 2, secs=900, every=5)
    mid = lambda t: ((t[0] + t[2]) / 2, (t[1] + t[3]) / 2) if isinstance(t, list) else (t["x"] + t["w"] / 2, t["y"] + t["h"] / 2)
    def click(name, right=False):
        S.wait_for(lambda: name in tg(), secs=20); x, y = mid(tg()[name])
        (S.right_click_xy if right else S.click_xy)(x, y)
    n0 = len(pe()["groups"]) + len(pe()["named"])
    # 1. Right-click a group tile -> Ignore group: it leaves the cards, "Ignored (N)" appears.
    click("person:0", right=True); S.shot("1-card-menu"); click("people:ctx-ignore")
    S.wait_for(lambda: pe().get("ignored"), secs=20)
    rec("ignore group", len(pe()["groups"]) + len(pe()["named"]) == n0 - 1 and "people:ignored" in tg(), pe().get("ignored_count")); S.shot("2-ignored")
    # 2. Stop ignoring: back as an unnamed group.
    click("people:unignore:0"); S.wait_for(lambda: not pe().get("ignored"), secs=20)
    rec("stop ignoring", len(pe()["groups"]) + len(pe()["named"]) >= n0 - 1 and not pe()["ignored"])
    # 3. Right-click a group -> Not a face; then "It is a face".
    click("person:0", right=True); click("people:ctx-notface"); S.wait_for(lambda: pe().get("notfaces"), secs=20)
    rec("not a face", "people:notfaces" in tg(), pe().get("notfaces_count")); S.shot("3-notfaces")
    click("people:isface:0"); S.wait_for(lambda: not pe().get("notfaces"), secs=20); rec("it is a face", not pe()["notfaces"])
    # 4. A face on a group page: right-click -> Ignore face; group page More -> Not a face (closes the page).
    click("person:0"); S.wait_for(lambda: pe().get("faces", 0) > 0, secs=20)
    click("face:0", right=True); S.shot("4-face-menu"); click("people:ctx-ignore"); time.sleep(2)
    click("people:more"); click("people:ctx-notface"); time.sleep(2)
    rec("face ignore + More not a face", pe().get("ignored") and pe().get("notfaces"), (pe().get("ignored"), pe().get("notfaces")))
    # 5. Ignored -> Group again; Not faces -> Assign to... the first person/group.
    click("people:regroup:0"); time.sleep(3); rec("group again", not pe().get("ignored"))
    k = len(pe().get("ignored", []))
    click(f"people:assign:{k}"); S.wait_for(lambda: pe().get("assigning") is not None, secs=10); click("people:assign-to:0")
    S.wait_for(lambda: not pe().get("notfaces"), secs=20); rec("assign to", not pe()["notfaces"]); S.shot("5-end")
    print("RESULT", "PASS" if all(res) else "FAIL", f"{sum(res)}/{len(res)}"); sys.exit(0 if all(res) else 1)

if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
