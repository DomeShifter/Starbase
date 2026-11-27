#!/usr/bin/env python3
# Last updated: 2026-11-27T11:05:49Z
"""Real-input check of PR photos-face-recheck (older photos are checked once more for missed faces).
  photos_face_recheck_input.py http://<host>:<port> --browser chromium|firefox --pics DIR_WITH_JPGS --phase before|after [--user NAME]
Server built with --features ml, the user owns it. Pictures as for photos_faces_polish.py (>= 2 people, >= 2 pictures
each), ideally one with two faces close together.
  before: uploads, turns faces on, waits for the scan, names the first group "Recheck Ann" and saves the state to
          ./recheck-before.json.
  Then by hand: stop the test server, run on its photo index (index.sqlite in the Photos cache folder)
          sqlite3 index.sqlite "UPDATE faces_done SET detect = 1"   and start the server again.
  after:  opens People and checks: the bar reads "Checking older photos again: x of y" (target people:progress)
          while it runs, the test hook's progress.recheck reaches done == all, "Recheck Ann" still exists with at least
          as many pictures, and no group or person disappeared. Shots in ./shots."""
import json, os, sys, time, glob, urllib.parse
sys.path.insert(0, os.path.expanduser("~/.claude/tools")); sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import starbase_ui as ui
RD = "/tmp/photos-face-recheck"

def launcher():
    a = sys.argv[1:]; pics = a[a.index("--pics") + 1]; rest = [x for x in a if x not in ("--pics", pics)]
    extra = {os.path.basename(f): open(f, "rb").read() for f in sorted(glob.glob(pics + "/*.jpg"))}
    sys.exit(ui.ship_and_run(os.path.abspath(__file__), rest, RD, pyname="p.py", extra=extra, shots_to=os.path.join(os.getcwd(), "shots")))

def main():
    a = sys.argv[1:]; a.remove("--remote"); base = a[0]; br = a[a.index("--browser") + 1] if "--browser" in a else "chromium"
    phase = a[a.index("--phase") + 1]
    res = []
    def rec(n, ok, d=""): res.append(bool(ok)); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base); pg = S.pg; S.sign_in(a[a.index("--user") + 1] if "--user" in a else "ada")
    q = lambda x: urllib.parse.quote(x, safe="")
    st = lambda: pg.request.get(f"{S.base}/api/photos/faces").json()
    people = lambda: pg.request.get(f"{S.base}/api/photos/people").json()
    if phase == "before":
        folder = pg.request.post(f"{S.base}/api/folders?path={S.user}&name=Recheck%20{br}{int(time.time())}").json()["path"]
        for f in sorted(glob.glob("*.jpg")):
            pg.request.post(f"{S.base}/api/files?name={q(f)}&as_is=1&folder={q(folder)}", data=open(f, "rb").read())
        if st().get("missing"):
            pg.request.post(f"{S.base}/api/photos/faces/models", data="{}"); S.wait_for(lambda: not st().get("missing"), secs=600, every=5)
        pg.request.post(f"{S.base}/api/photos/faces", data=json.dumps({"on": True}), headers={"Content-Type": "application/json"})
        pr = lambda: st()["progress"]
        S.wait_for(lambda: pr()["all"] > 0 and pr()["done"] >= pr()["all"] and people().get("groups"), secs=600, every=5)
        g = people()["groups"][0]["id"]
        pg.request.post(f"{S.base}/api/photos/people/{g}/name", data=json.dumps({"name": "Recheck Ann"}), headers={"Content-Type": "application/json"})
        json.dump({"people": people(), "progress": pr()}, open("recheck-before.json", "w"))
        rec("before: scanned and named", True, json.dumps(pr()))
    else:
        S.open("drive", path="/?test"); time.sleep(2)
        S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:tab", tab="people"); time.sleep(3)
        pe = lambda: S.snapshot("home.photos.people") or {}
        rp = lambda: (pe().get("progress") or {}).get("recheck") or {}
        running = rp().get("done", 0) < rp().get("all", 0)
        rec("after: re-check pending at start", running, json.dumps(rp()))
        if running:
            rec("after: bar shown", S.wait_for(lambda: "people:progress" in (S.snapshot("targets") or {}), secs=20))
            S.wait_text("Checking older photos again", secs=20); S.shot("recheck-bar")
            rec("after: bar text", True)
        rec("after: re-check finished", S.wait_for(lambda: rp().get("all", 0) > 0 and rp().get("done") == rp().get("all"), secs=900, every=5), json.dumps(rp()))
        b, now = json.load(open("recheck-before.json"))["people"], people()
        ann = lambda p: [c for c in p.get("named", []) if c.get("name") == "Recheck Ann"]
        rec("after: named person kept", ann(now) and ann(now)[0].get("count", 0) >= ann(b)[0].get("count", 0), json.dumps(ann(now)))
        rec("after: nothing removed", len(now.get("named", [])) + len(now.get("groups", [])) >= len(b.get("named", [])) + len(b.get("groups", [])))
        S.shot("people-after")
    S.close(); print("ALL PASS" if all(res) else "SOME FAILED", flush=True); sys.exit(0 if all(res) else 1)

if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
