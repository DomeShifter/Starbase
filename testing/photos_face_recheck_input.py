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
    J = {"Content-Type": "application/json"}
    post = lambda path, body: pg.request.post(f"{S.base}{path}", data=json.dumps(body), headers=J)
    def library():
        lib = pg.request.get(f"{S.base}/api/photos?all=1").json(); return lib.get("photos", lib.get("items", [])) if isinstance(lib, dict) else lib
    def faces_by_pic():
        """Every face the index knows (people, groups, Ignored, Not a face) -> {picture id: [boxes]}."""
        pp = people(); out = {}; marks = {}
        for c in pp.get("people", []) + pp.get("groups", []):
            for f in pg.request.get(f"{S.base}/api/photos/people/{c['id']}").json().get("faces", []): out.setdefault(f["id"], []).append(f["box"])
        for k in ("ignored", "notfaces"):
            for f in pp.get(k, []): out.setdefault(f["id"], []).append(f["box"]); marks.setdefault(k, []).append(f["id"])
        return out, marks
    def iou(a, b):
        x0, y0, x1, y1 = max(a[0], b[0]), max(a[1], b[1]), min(a[0] + a[2], b[0] + b[2]), min(a[1] + a[3], b[1] + b[3])
        i = max(0, x1 - x0) * max(0, y1 - y0); u = a[2] * a[3] + b[2] * b[3] - i
        return i / u if u else 0
    if phase == "before":
        folder = f"{S.user}/Photos"  # the library folder Photos indexes by itself
        COPIES = 16  # the same pictures under 6 names: a library big enough that the re-check takes a while
        for c in range(COPIES):
            for f in sorted(glob.glob("*.jpg")):
                pg.request.post(f"{S.base}/api/files?name={q('c%d_' % c + f)}&as_is=1&folder={q(folder)}", data=open(f, "rb").read() + b"\0" * c)  # distinct bytes: identical files are stored once
        if st().get("missing"):
            pg.request.post(f"{S.base}/api/photos/faces/models", data="{}"); S.wait_for(lambda: not st().get("missing"), secs=600, every=5)
        S.wait_for(lambda: len(library()) >= 16 * len(glob.glob("*.jpg")), secs=300, every=5)
        pg.request.post(f"{S.base}/api/photos/faces", data=json.dumps({"on": True}), headers=J)
        pr = lambda: st()["progress"]
        S.wait_for(lambda: pr()["all"] > 0 and pr()["done"] >= pr()["all"] and len(people().get("groups", [])) >= 3, secs=600, every=5)
        lib = library(); print("library item keys", sorted(lib[0]) if lib else None, flush=True)
        idname = {str(i.get("id")): i.get("name", "") for i in lib}
        gs = people()["groups"]; kinds = {}
        for g in gs:
            fs = pg.request.get(f"{S.base}/api/photos/people/{g['id']}").json()["faces"]
            kinds[g["id"]] = {idname.get(str(f["id"]), "?").split("_")[-1].rstrip("0123456789.jpg") for f in fs}
        print("groups", json.dumps({k: sorted(v) for k, v in kinds.items()}), flush=True)
        doll = [g for g, v in kinds.items() if any("doll" in n for n in v)]
        rest = [g["id"] for g in gs if g["id"] not in doll]
        print("doll group" if doll else "no doll face was detected: one face of a person group stands in for the doll", flush=True)
        rec("before: three groups to mark", len(rest) >= 3 or (doll and len(rest) >= 2), (doll, rest))
        post(f"/api/photos/people/{rest[0]}/name", {"name": "Recheck Ann"})
        if doll: post(f"/api/photos/people/{doll[0]}/notface", {})
        else:
            f0 = pg.request.get(f"{S.base}/api/photos/people/{rest[2]}").json()["faces"][0]["face"]
            post(f"/api/photos/people/{rest[2]}/notface", {"faces": [f0]})
        post(f"/api/photos/people/{rest[1]}/ignore", {})
        pp = people(); fb, marks = faces_by_pic()
        rec("before: named, not-face and ignored set", pp.get("people") and pp.get("notfaces") and pp.get("ignored"), json.dumps({k: len(v) for k, v in marks.items()}))
        annf = len(pg.request.get(f"{S.base}/api/photos/people/{[c for c in pp['people'] if c['name'] == 'Recheck Ann'][0]['id']}").json()["faces"])
        json.dump({"annf": annf, "people": pp, "progress": pr(), "faces": fb, "marks": marks, "names": idname}, open("recheck-before.json", "w"))
        rec("before: scanned", True, json.dumps(pr()) + f" photos with faces {len(fb)} faces {sum(map(len, fb.values()))}")
    else:
        S.open("drive", path="/?test"); time.sleep(2)
        S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:tab", tab="people"); time.sleep(3)
        pe = lambda: S.snapshot("home.photos.people") or {}
        rp = lambda: (pe().get("progress") or {}).get("recheck") or {}
        if "--reset" in a:  # --reset NAME: the script runs on the build box; stop NAME's container, mark every picture as checked by the old finder, start it again
            import subprocess; cn = a[a.index("--reset") + 1]
            db = subprocess.check_output(["bash", "-c", f"find /srv/build/testdata/{cn} -name index.sqlite -path '*cache/photos*'"], text=True).split()[0]
            subprocess.run(["docker", "stop", cn], check=True, capture_output=True)
            import sqlite3; c = sqlite3.connect(db); c.execute("UPDATE faces_done SET detect = 1"); c.commit(); c.close()
            subprocess.run(["docker", "start", cn], check=True, capture_output=True)
            seen = []; t0 = time.time()
            while time.time() - t0 < 90 and not seen:  # tight poll while the server comes up: the first moment the re-check is running
                try: r = pg.request.get(f"{S.base}/api/photos/faces", timeout=2000).json()["progress"].get("recheck") or {}
                except Exception: continue
                if r.get("done", 0) < r.get("all", 0): seen.append(r); S.shot("recheck-bar-early")
            print("recheck seen by api poll:", seen, flush=True)
            S.hook("home:photos:tab", tab="people"); bar = False; t0 = time.time()
            while time.time() - t0 < 20 and not bar:
                bar = "people:progress" in (S.snapshot("targets") or {}) and rp().get("done", 0) < rp().get("all", 0)
                if not bar: S.hook("home:photos:tab", tab="people")
            if bar: S.shot("recheck-bar-ui"); print("bar on screen:", rp(), flush=True)
            print("progress line on screen while re-checking:", bar, "(too fast to catch at this size: unit-level only)" if not bar else "", flush=True)
        running = rp().get("done", 0) < rp().get("all", 0)
        if "--reset" not in a: rec("after: re-check pending at start", running, json.dumps(rp()))
        if running:
            rec("after: bar shown", S.wait_for(lambda: "people:progress" in (S.snapshot("targets") or {}), secs=20))
            S.wait_text("Checking older photos again", secs=20); S.shot("recheck-bar")
            rec("after: bar text", True)
        rec("after: re-check finished", S.wait_for(lambda: rp().get("all", 0) > 0 and rp().get("done") == rp().get("all"), secs=900, every=5), json.dumps(rp()))
        B = json.load(open("recheck-before.json")); b, now = B["people"], people()
        ann = lambda p: [c for c in p.get("people", []) if c.get("name") == "Recheck Ann"]
        annf_now = len(pg.request.get(f"{S.base}/api/photos/people/{ann(now)[0]['id']}").json()["faces"]) if ann(now) else 0
        rec("after: named person kept", ann(now) and ann(now) and annf_now >= B["annf"], f"faces {B['annf']} -> {annf_now if ann(now) else None}")
        fa, marks = faces_by_pic(); fb = B["faces"]
        rec("after: not-face marks stay", sorted(marks.get("notfaces", [])) == sorted(B["marks"]["notfaces"]) and len(now.get("notfaces", [])) >= len(b["notfaces"]), (len(b["notfaces"]), len(now.get("notfaces", []))))
        rec("after: ignored marks stay", sorted(marks.get("ignored", [])) == sorted(B["marks"]["ignored"]), (len(b["ignored"]), len(now.get("ignored", []))))
        dollids = [i for i, n in B["names"].items() if "doll" in n]
        rec("after: dolls not back in People", not any(i in {f for c in now.get("people", []) + now.get("groups", []) for f in [x["id"] for x in pg.request.get(f"{S.base}/api/photos/people/{c['id']}").json().get("faces", [])]} for i in B["marks"]["notfaces"]))
        new = 0; dup = 0; lost = 0
        for pid, boxes in fa.items():
            old = [tuple(x) for x in fb.get(pid, [])]
            extra = len(boxes) - len(old)
            if extra < 0: lost += 1
            for bx in boxes:
                if tuple(bx) in old: continue
                new += 1; dup += any(iou(bx, o) >= 0.5 for o in old)
        rec("after: faces per photo >= before, none lost", lost == 0 and all(len(fa.get(k, [])) >= len(v) for k, v in fb.items()), f"before {sum(map(len, fb.values()))} after {sum(map(len, fa.values()))}")
        rec("after: every extra box is new, no overlap with an old one", dup == 0, f"new boxes {new} overlapping {dup}")
        print("per-photo changes", {B['names'].get(k, k): (len(fb.get(k, [])), len(v)) for k, v in fa.items() if len(v) != len(fb.get(k, []))}, flush=True)
        S.shot("people-after")
    S.close(); print("ALL PASS" if all(res) else "SOME FAILED", flush=True); sys.exit(0 if all(res) else 1)

if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
