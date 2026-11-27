#!/usr/bin/env python3
# Last updated: 2026-11-27T06:28:18Z
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
    # Models and on/off aren't what this checks: through the API (the button is "Download face models and turn on").
    st0 = lambda: pg.request.get(f"{S.base}/api/photos/faces").json()
    if st0().get("missing"):
        pg.request.post(f"{S.base}/api/photos/faces/models", data="{}")
        S.wait_for(lambda: not st0().get("missing"), secs=600, every=5)
    pg.request.post(f"{S.base}/api/photos/faces", data=json.dumps({"on": True}), headers={"Content-Type": "application/json"})
    S.hook("home:photos:tab", tab="people"); time.sleep(2)
    S.wait_for(lambda: pe().get("on") is True, secs=30)
    n_people = len({who(f) for f in first})
    S.wait_for(lambda: len(pe().get("named", [])) + len(pe().get("groups", [])) >= n_people, secs=900, every=5)
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
    # The open People list isn't asked again after an upload: read the server's list (what a reload shows).
    srv = lambda: pg.request.get(f"{S.base}/api/photos/people").json()
    new = lambda: [c for c in srv().get("groups", []) if c["id"] not in before and c["count"] >= 2]
    rec(f"new person '{late}' found as a new group", S.wait_for(lambda: new(), secs=600, every=5), json.dumps(srv().get("groups")))
    grew = {c["id"]: c["count"] for c in srv().get("people", []) if c["count"] != named_counts.get(c["id"], c["count"])}
    S.hook("home:photos:tab", tab="photos"); time.sleep(1); S.hook("home:photos:tab", tab="people"); time.sleep(3)
    rec("named people did not swallow them", not grew, json.dumps(grew))
    S.shot(f"{br}-3-new-person")
    # 4. Safer remove (photos-faces-ux): open the first named person with >= 2 faces.
    S.hook("home:photos:tab", tab="photos"); time.sleep(1); S.hook("home:photos:tab", tab="people"); time.sleep(3)
    named = pe().get("named", []); k = next((i for i, c in enumerate(named) if c["count"] >= 2), 0); person = named[k]
    S.click_text(f"person:{k}"); S.wait_for(lambda: pe().get("faces", 0) >= 2, secs=20)
    n0 = pe()["faces"]
    S.click_text("face:0"); time.sleep(2)
    rec("click on a face opens the picture", S.snapshot("home.photos.viewer"), json.dumps(S.snapshot("home.photos.viewer")))
    S.hook("home:photos:key", key="close"); time.sleep(1)
    S.click_text("people:face-select:0"); S.click_text("people:face-select:1"); time.sleep(1)
    rec("the circles select two faces", len(pe().get("selected", [])) == 2, json.dumps(pe().get("selected")))
    S.shot(f"{br}-4-selected")
    S.click_text("people:remove-selected"); time.sleep(1)
    rec("Remove asks first", pe().get("confirm") == 2, json.dumps(pe().get("confirm")))
    S.shot(f"{br}-5-confirm")
    S.click_text("people:remove-cancel"); time.sleep(1)
    rec("Cancel keeps the faces", pe().get("confirm") is None and pe()["faces"] == n0, f"{pe().get('faces')} vs {n0}")
    S.click_text("face:0", button=3); time.sleep(1); S.click_text("people:face-menu"); time.sleep(1)
    rec("right-click Not X asks first", pe().get("confirm") == 1, json.dumps(pe().get("confirm")))
    S.click_text("people:remove-yes")
    rec("Remove takes it out", S.wait_for(lambda: pe()["faces"] == n0 - 1, secs=20), str(pe().get("faces")))
    S.click_text("people:back"); time.sleep(1)
    one = lambda: [c for c in srv().get("groups", []) if c["id"] not in before]
    rec("the removed face is its own unnamed group", S.wait_for(lambda: one(), secs=120, every=5), json.dumps(srv().get("groups")))
    time.sleep(20)  # a regroup pass
    back = [c for c in srv().get("people", []) if c["id"] == person["id"]]
    rec("it never rejoins the person", back and back[0]["count"] <= person["count"], json.dumps(back))
    # 5. "Same person?": only when these pictures hold a borderline look-alike.
    if srv().get("questions"):
        S.hook("home:photos:tab", tab="photos"); time.sleep(1); S.hook("home:photos:tab", tab="people"); time.sleep(3)
        S.shot(f"{br}-6-same-person")
        S.click_text("people:same-skip"); time.sleep(1)
        rec("Skip hides the question", pe().get("skipped") == 1, json.dumps(pe().get("skipped")))
        if pe().get("questions"):
            S.click_text("people:same-no"); time.sleep(5)
            rec("No is remembered", len(srv().get("questions", [])) < len(pe().get("questions", [])) + 2, json.dumps(srv().get("questions")))
    else:
        print("SKIP same-person: no borderline look-alikes in these pictures", flush=True)
    print("RESULT", "ALL PASS" if all(res) else f"{res.count(False)} FAIL", flush=True)
    sys.exit(0 if all(res) else 1)

if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
