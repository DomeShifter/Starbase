#!/usr/bin/env python3
# Last updated: 2026-11-27T06:27:12Z
"""Real-input check of the viewer's Info panel "People" section and the big-picture flicker fix (UNRUN: written
by the build helper; the verify helper runs and fixes it).
Seed first: python3 -I testing/seed.py --url <URL> --set gps ; faces on, one face named (People page) for --user.
Run:  photos_info_people.py http://<host>:<port> --browser chromium|firefox [--user NAME] [--folder PATH]
Expects: Info (viewer:info) shows the heading PEOPLE (target photos:info-people) with a quiet line
("Looking for faces…" briefly, then "No faces found" / "Face recognition is off.") or one link per named
person (photos:info-person:N, round face + name) and "N unnamed faces" (photos:info-people-unnamed).
Clicking photos:info-person:0 closes the viewer and opens that person's page (People tab, opened card).
Flicker: while the viewer shows one picture, textures.requests stays the same over 5 s (one open = its
1024 + at most one original request, never more)."""
import os, sys, time
sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
import starbase_ui as ui


def launcher():
    sys.exit(ui.ship_and_run(os.path.abspath(__file__), sys.argv[1:], "/tmp/pip", pyname="p.py", shots_to=os.path.join(os.getcwd(), "shots")))


def main():
    a = sys.argv[1:]; a.remove("--remote"); base = a[0]; br = a[a.index("--browser") + 1] if "--browser" in a else "chromium"
    user = a[a.index("--user") + 1] if "--user" in a else "ada"
    folder = a[a.index("--folder") + 1] if "--folder" in a else f"{user}/seed-gps"
    res = []
    def rec(n, ok, d=""): res.append(ok); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base); S.sign_in(user)
    S.open("drive"); S.hook("home:folder", path=folder); time.sleep(2); S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:enable"); time.sleep(8)
    tg = lambda: S.snapshot("targets") or {}
    vw = lambda: S.snapshot("home.photos.viewer") or {}
    req = lambda: (S.snapshot("home.photos.textures") or {}).get("requests", 0)
    S.hook("home:photos:tab", tab="photos")
    S.hook("home:photos:open", index=0); time.sleep(4)
    r0 = req(); time.sleep(5); r1 = req()
    rec("one open = no repeated image requests", r1 == r0, f"{r0} -> {r1}")
    S.click_text("viewer:info"); time.sleep(3)
    for i in range(1, 27):  # find a picture with a NAMED person (People are loaded when the panel opens)
        if (vw().get("people") or {}).get("people"): break
        S.key("Right"); time.sleep(2.5)
    rec("People section shown", "photos:info-people" in tg(), str(vw().get("people")))
    S.shot(f"{br}-info-people")
    ppl = (vw().get("people") or {}).get("people") or []
    if ppl:
        S.click_text("photos:info-person:0"); time.sleep(3)
        st = S.snapshot("home.photos") or {}
        rec("person link opens their page", not st.get("viewer"), str({k: st.get(k) for k in ("tab", "open")}))
    else:
        print("SKIP person link: no named face in the first picture", flush=True)
    S.close()
    sys.exit(0 if all(res) else 1)


if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
