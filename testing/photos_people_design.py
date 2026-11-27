#!/usr/bin/env python3
# Last updated: 2026-11-26T23:54:14Z
"""Real-input check of PR photos-design-people (People design: strip heading, cards, one Face recognition
menu, off page as an empty state, person page with an editable title and a More menu).
  photos_people_design.py http://<host>:<port> --browser chromium|firefox --pics DIR_WITH_JPGS [--user NAME]
Pictures: faces of >= 2 people, >= 2 pictures each. Server with --features ml (models present or downloadable);
the user must own the server. Shots in ./shots (look at: off page, People page, open menu, strip, person page)."""
import json, os, subprocess, sys, time, glob, urllib.parse
sys.path.insert(0, os.path.expanduser("~/.claude/tools")); sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import starbase_ui as ui
RD = "/tmp/photos-people-design"
NAME = "Ada Design"

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
    key = lambda k: subprocess.run(["xdotool", "key", k])
    folder = pg.request.post(f"{S.base}/api/folders?path={S.user}&name=Design%20{br}{int(time.time())}").json()["path"]
    for f in sorted(glob.glob("*.jpg")):
        pg.request.post(f"{S.base}/api/files?name={q(f)}&as_is=1&folder={q(folder)}", data=open(f, "rb").read())
    st0 = lambda: pg.request.get(f"{S.base}/api/photos/faces").json()
    if st0().get("missing"):  # the download itself isn't what this checks
        pg.request.post(f"{S.base}/api/photos/faces/models", data="{}")
        S.wait_for(lambda: not st0().get("missing"), secs=600, every=5)
    pg.request.post(f"{S.base}/api/photos/faces", data=json.dumps({"on": False}), headers={"Content-Type": "application/json"})
    S.open("drive", path=f"/?test&dir={q(folder)}"); time.sleep(2)
    S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:enable"); time.sleep(8)
    S.hook("home:photos:tab", tab="people"); time.sleep(3)
    pe = lambda: S.snapshot("home.photos.people") or {}
    tg = lambda: S.snapshot("targets") or {}
    # 1. Off: one empty state with one main button; the menu button says Off.
    t = tg(); S.shot(f"{br}-1-off")
    rec("off page = empty state with Turn on", "empty:Turn on face recognition" in t, str([k for k in t if k.startswith(("empty:", "people:"))]))
    rec("menu button shown when off", "people:fr-menu" in t)
    S.click_text("empty:Turn on face recognition")
    rec("turned on by the button", S.wait_for(lambda: pe().get("on") is True, secs=30))
    S.wait_for(lambda: (pe().get("progress") or {}).get("done") == (pe().get("progress") or {}).get("all") and pe().get("named", []) + pe().get("groups", []), secs=600, every=5)
    time.sleep(3); t = tg(); S.shot(f"{br}-2-people")
    rec("header: menu + Scan now", "people:fr-menu" in t and "photos:scan-now" in t, str([k for k in t if k.startswith(("people:", "photos:scan"))]))
    rec("cards + inline name", "person:0" in t and "people:name:0" in t)
    if pe().get("groups"):
        rec("Unnamed groups heading", "people:groups-heading" in t)
    # 2. The one menu: turn off, privacy note.
    S.click_text("people:fr-menu"); time.sleep(1); t = tg(); S.shot(f"{br}-3-menu")
    rec("menu: Turn off + privacy note", "Turn off face recognition" in t and "people:fr-note" in t)
    key("Escape"); time.sleep(1)
    # 3. Strip on Photos: heading row, See all, faces.
    S.hook("home:photos:tab", tab="photos"); time.sleep(3); t = tg(); S.shot(f"{br}-4-strip")
    rec("strip heading + See all + face", all(k in t for k in ("people:strip-heading", "people:see-all", "pill-See all", "people-strip:0")))
    S.click_text("people:see-all"); time.sleep(2)
    rec("See all opens People", "people:fr-menu" in tg())
    # 4. Person page: click the title, type, Enter saves; More has Merge.
    k = len(pe().get("named", []))
    S.click_text(f"person:{k if pe().get('groups') else 0}"); time.sleep(3)
    t = tg(); rec("person page: back, title, More", all(x in t for x in ("people:back", "people:title", "people:more")))
    S.click_text("people:title"); time.sleep(1)
    rec("title turns into a name box", pe().get("editing") is not None)
    subprocess.run(["xdotool", "key", "ctrl+a"]); subprocess.run(["xdotool", "type", "--delay", "40", NAME]); key("Return")
    rec("rename from the title", S.wait_for(lambda: any(n["name"] == NAME for n in pe().get("named", [])), secs=20), json.dumps(pe().get("named")))
    S.shot(f"{br}-5-person")
    S.click_text("people:more"); time.sleep(1)
    rec("More menu: Merge with…", "Merge with…" in tg())
    S.click_text("Merge with…"); time.sleep(1)
    rec("merge picker shown", pe().get("merging") is True)
    S.shot(f"{br}-6-merge")
    # 5. Left list: Events has its icon; Albums title spacing (look at the shots).
    S.hook("home:photos:tab", tab="albums"); time.sleep(2); S.shot(f"{br}-7-albums")
    print("RESULT", "ALL PASS" if all(res) else f"{res.count(False)} FAIL", flush=True)
    sys.exit(0 if all(res) else 1)

if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
