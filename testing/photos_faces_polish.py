#!/usr/bin/env python3
# Last updated: 2026-11-26T21:21:29Z
"""Real-input check of PR photos-faces-polish (People: sharp faces, rename from the list, new people in new scans).
  photos_faces_polish.py http://<host>:<port> --browser chromium|firefox --pics DIR_WITH_JPGS [--user NAME]
Pictures are named <person><n>.jpg (aldrin1.jpg ...), >= 3 people with >= 3 pictures each. The LAST person
(alphabetically) is held back and uploaded after the first scan: it must come up as a NEW unnamed group.
Server must run with --features ml (models present or downloadable) and the user must own it. Shots in ./shots."""
import json, os, subprocess, sys, time, glob, urllib.parse
sys.path.insert(0, os.path.expanduser("~/.claude/tools")); sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import starbase_ui as ui
RD = "/tmp/photos-faces-polish"

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
    who = lambda f: f.rstrip(".jpg").rstrip("0123456789")
    files = sorted(glob.glob("*.jpg")); late = sorted({who(f) for f in files})[-1]
    first, later = [f for f in files if who(f) != late], [f for f in files if who(f) == late]
    folder = pg.request.post(f"{S.base}/api/folders?path={S.user}&name=Faces%20{br}{int(time.time())}").json()["path"]
    up = lambda fs: [pg.request.post(f"{S.base}/api/files?name={q(f)}&as_is=1&folder={q(folder)}", data=open(f, "rb").read()) for f in fs]
    up(first)
    S.open("drive", path=f"/?test&dir={q(folder)}"); time.sleep(2)
    S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:enable"); time.sleep(8)
    S.hook("home:photos:tab", tab="people"); time.sleep(2)
    pe = lambda: S.snapshot("home.photos.people") or {}
    tg = lambda: S.snapshot("targets") or {}
    if pe().get("can_download"):
        S.click_text("Download face models"); S.wait_for(lambda: not pe().get("missing"), secs=400)
    if "Turn on" in tg(): S.click_text("Turn on")
    S.wait_for(lambda: pe().get("on") is True, secs=30)
    n_people = len({who(f) for f in first})
    S.wait_for(lambda: len(pe().get("named", [])) + len(pe().get("groups", [])) >= n_people, secs=420, every=5)
    time.sleep(5); st = pe(); S.shot(f"{br}-1-people")
    # 1. Sharp faces: every card has a face id, and its picture is a 256 px square.
    cards = st.get("named", []) + st.get("groups", [])
    sizes = []
    for c in cards:
        r = pg.request.get(f"{S.base}/api/photos/people/face/{c['face']}/{c['cover']}")
        sizes.append(r.status if r.status != 200 else len(r.body()))
    rec("face pictures served (256 px JPEG)", cards and all(c.get("face") for c in cards) and all(s > 1000 for s in sizes), str(sizes))
    # 2. Rename from the list: click the name, type, Enter.
    S.click_text("people:name:0"); time.sleep(1)
    rec("name box opens in place", pe().get("editing"), json.dumps(pe().get("editing")))
    subprocess.run(["xdotool", "key", "ctrl+a"]); subprocess.run(["xdotool", "type", "--delay", "40", "Sally Ride"]); subprocess.run(["xdotool", "key", "Return"])
    rec("Enter saves the name", S.wait_for(lambda: any(c["name"] == "Sally Ride" for c in pe().get("named", [])), secs=20), json.dumps(pe().get("named")))
    rec("the person page did not open", not S.snapshot("home.photos.people.faces"))
    S.shot(f"{br}-2-renamed")
    k = [c["name"] for c in pe()["named"]].index("Sally Ride")
    S.click_text(f"people:name:{k}"); time.sleep(1); subprocess.run(["xdotool", "type", "--delay", "40", "Wrong"]); subprocess.run(["xdotool", "key", "Escape"]); time.sleep(3)
    rec("Esc cancels", any(c["name"] == "Sally Ride" for c in pe().get("named", [])) and not pe().get("editing"), json.dumps(pe().get("named")))
    if pe().get("groups"):
        g = len(pe()["named"]); S.click_text(f"people:name:{g}"); time.sleep(1)
        subprocess.run(["xdotool", "type", "--delay", "40", "Mae Jemison"]); S.click_text("People"); time.sleep(1)
        rec("clicking elsewhere saves", S.wait_for(lambda: any(c["name"] == "Mae Jemison" for c in pe().get("named", [])), secs=20), json.dumps(pe().get("named")))
    # 3. New people in new scans: the held-back person comes up as a new group.
    before = {c["id"] for c in pe().get("named", []) + pe().get("groups", [])}
    named_counts = {c["id"]: c["count"] for c in pe().get("named", [])}
    up(later)
    new = lambda: [c for c in pe().get("groups", []) if c["id"] not in before and c["count"] >= 2]
    rec(f"new person '{late}' found as a new group", S.wait_for(lambda: new(), secs=420, every=5), json.dumps(pe().get("groups")))
    grew = {i: c["count"] for c in pe().get("named", []) for i in [c["id"]] if c["count"] != named_counts.get(i, c["count"])}
    rec("named people did not swallow them", not grew, json.dumps(grew))
    S.shot(f"{br}-3-new-person")
    print("RESULT", "ALL PASS" if all(res) else f"{res.count(False)} FAIL", flush=True)
    sys.exit(0 if all(res) else 1)

if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
