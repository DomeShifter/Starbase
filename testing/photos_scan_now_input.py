#!/usr/bin/env python3
"""Real-input proof of Photos > Scan now. Launch from anywhere (runs on the build box):
  photos_scan_now_input.py http://<host>:<port> --browser chromium|firefox --data DATA_DIR --pics DIR_ON_BOX [--phone]
DATA_DIR: the test server's store on the build box (pictures are dropped there on disk, as on a NAS);
DIR_ON_BOX: portrait JPGs on the build box. Server must run with --features ml; ada must be the owner.
Checks: toolbar button → "Looking for new photos…" → (faces) → "Up to date — 3 new photos…", the new pictures in the
grid without a reload, faces in People; a second click within 30 s shows the wait message; the People page button;
--phone: the button inside the narrow menu (photo-menu)."""
import glob, json, os, shutil, sys, time, urllib.parse
sys.path.insert(0, os.path.expanduser("~/.claude/tools")); sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import starbase_ui as ui
RD = "/tmp/photos-scan-now"
q = lambda x: urllib.parse.quote(x, safe="")


def launcher():
    d = os.path.join(os.getcwd(), "shots")
    sys.exit(ui.ship_and_run(os.path.abspath(__file__), sys.argv[1:], RD, pyname="p.py", shots_to=d))


def main():
    a = sys.argv[1:]; a.remove("--remote"); base = a[0]
    arg = lambda k, d=None: a[a.index(k) + 1] if k in a else d
    br, data, pics, phone = arg("--browser", "chromium"), arg("--data"), arg("--pics"), "--phone" in a
    res = []
    def rec(n, ok, d=""): res.append(bool(ok)); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base, phone=phone); pg = S.pg; S.sign_in("ada")
    tag = f"{br}{'-phone' if phone else ''}{int(time.time())}"
    folder = pg.request.post(f"{S.base}/api/folders?path=ada&name=Scan%20{q(tag)}").json()["path"]
    first = sorted(glob.glob(pics + "/*.jpg"))
    pg.request.post(f"{S.base}/api/files?name=start.jpg&as_is=1&folder={q(folder)}", data=open(first[0], "rb").read())
    S.open("drive", path=f"/?test&dir={q(folder)}"); time.sleep(2)
    S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:enable"); time.sleep(6)
    pe = lambda: S.snapshot("home.photos.people") or {}
    sc = lambda: pe().get("scan") or {}
    items = lambda: len(S.snapshot("home.photos.items") or [])
    # Faces on (models fetched once per server).
    S.hook("home:photos:tab", tab="people"); time.sleep(3)
    tg = lambda: S.snapshot("targets") or {}
    for label in ("Download face models and turn on", "Turn on face recognition", "Download face models", "Turn on"):
        if pe().get("on") is not True and label in tg():
            S.click_text(label); time.sleep(2)
    S.wait_for(lambda: not pe().get("missing") and not (pe().get("download") or {}).get("busy"), secs=400, every=3)
    if pe().get("on") is not True and "Turn on face recognition" in tg():
        S.click_text("Turn on face recognition")
    rec("faces on", S.wait_for(lambda: pe().get("on") is True, secs=30), json.dumps(pe().get("note")))
    S.hook("home:photos:tab", tab="photos"); time.sleep(2)
    before = items()
    # Three new portraits dropped on disk, then Scan now straight away.
    for i, f in enumerate(first[1:4]):
        shutil.copy(f, os.path.join(data, folder, f"new{i}-{os.path.basename(f)}"))
    if phone:
        S.click_text("photo-menu"); time.sleep(1)
    S.click_text("photos:scan-now")
    seen = set()
    def watch():
        s = sc(); seen.add(s.get("said", ""))
        return s.get("running") is False and s.get("said")
    S.wait_for(watch, secs=240, every=0.3)
    said = sc().get("said", ""); print("SAID", sorted(seen), flush=True); S.shot(f"{tag}-1-done")
    rec("status went Looking → done", "Looking for new photos…" in seen and said.startswith(("Up to date", "No new photos")), said)
    rec("status counts the 3 new photos", "3 new photos" in said, said)
    rec("new photos in the grid without a reload", S.wait_for(lambda: items() >= before + 3, secs=30), f"{before}->{items()}")
    if phone:
        S.click_text("photo-menu"); time.sleep(1)
    S.click_text("photos:scan-now"); time.sleep(3)
    rec("second click within 30 s: wait message", "seconds" in (sc().get("said") or ""), sc().get("said"))
    S.shot(f"{tag}-2-wait")
    if not phone:
        time.sleep(31)
        S.hook("home:photos:tab", tab="people"); time.sleep(3)
        rec("People page has Scan now", "photos:scan-now" in (S.snapshot("targets") or {}))
        S.click_text("photos:scan-now")
        S.wait_for(watch, secs=240, every=0.5)
        rec("People-page scan finishes", (sc().get("said") or "").startswith(("Up to date", "No new photos")), sc().get("said"))
        ok = S.wait_for(lambda: sum(g.get("count", 0) for g in pe().get("groups", []) + pe().get("named", [])) >= 3, secs=120, every=3)
        rec("faces appear in People", ok, json.dumps({"groups": pe().get("groups")})[:300])
        S.shot(f"{tag}-3-people")
    sys.exit(0 if all(res) else 1)


if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
