#!/usr/bin/env python3
"""Real-input check of Photos to-do 3 rows 42 + 43, Chromium + Firefox.
42: rotate in the editor, Save a copy, then leave the original: NO "exiting without saving" question
    (state: photo_edit.dirty false, .unsaved true, .copied true, .ask empty), and another change after
    the copy asks again (dirty true).
43: no raw "no such file" text anywhere in Photos' error line (home.photos error is empty/friendly).
Seeds with the repo kit first:  python3 -I testing/seed.py --url <ready URL> --set types
    photos-bugs-42-43.py --testn 43
"""
import json, os, subprocess, sys, time

sys.path.insert(0, os.environ.get("STARBASE_UI_DIR", os.path.expanduser("~/.claude/tools")))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from starbase_ui import Session, ship_and_run  # noqa: E402


def find(d, key):
    if isinstance(d, dict):
        if key in d: return d[key]
        for v in d.values():
            r = find(v, key)
            if r is not None: return r
    elif isinstance(d, list):
        for v in d:
            r = find(v, key)
            if r is not None: return r
    return None


FOLDER = "ada/seed-types"


def delete_on_server(base, name):
    """Row 43 trigger: a picture deleted on the server while Photos still lists it."""
    import http.cookiejar, urllib.request
    op = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
    op.open(urllib.request.Request(base + "api/login", json.dumps({"username": "ada", "password": "adapassword"}).encode(), {"Content-Type": "application/json"}))
    items = json.load(op.open(base + "api/photos?folder=" + FOLDER))["photos"]
    pid = next(i["id"] for i in items if i["name"] == name)
    op.open(urllib.request.Request(base + "api/files/" + pid, method="DELETE"))


def stale(testn):
    """Row 43: Photos open, a picture vanishes on the server, then the viewer / editor / copy touch it.
    The error line (Drive's `error`, Photos' `photos.error`, the editor's) must never read 'no such file'."""
    fails = 0
    base = f"http://192.168.178.100:{3100 + testn}/"
    for br in ("chromium", "firefox"):
        try:
            with Session(br, testn=testn, outdir="shots") as s:
                s.sign_in("ada"); s.open("drive", tab="photos"); time.sleep(2)
                s.hook("home:folder", path=FOLDER); time.sleep(2)
                s.hook("home:photos:enable"); time.sleep(6)
                seen = []

                def look(what):
                    st = s.snapshot()
                    pe = find(st, "photo_edit") or {}
                    e = [str(st.get("error")), str((st.get("photos") or {}).get("error")), str(pe.get("error"))]
                    seen.append(" | ".join(e)); print(f"  [{br}] {what}: errors={e}", flush=True)
                look("before")
                gone = "type.webp" if br == "chromium" else "type.gif"; delete_on_server(base, gone)
                idx = [i["name"] for i in (s.snapshot().get("photos") or {}).get("items", [])].index(gone)
                for act in ("favourite", "download", "delete"):
                    s.hook("home:photos:menu", index=idx, act=act); time.sleep(3); look("menu " + act + " on vanished")
                s.hook("home:photos:open", index=idx); time.sleep(3); look("viewer on vanished")
                s.hook("home:photos:edit", open=True); time.sleep(4); look("editor on vanished")
                s.hook("home:photos:edit:rotate"); s.hook("home:photos:edit:copy"); time.sleep(4); look("copy of vanished")
                s.shot(f"{br}-stale-topleft")
                ok = not any("no such file" in x for x in seen)
                print(f"{'PASS' if ok else 'FAIL'} [{br}] stale picture: no raw 'no such file'", flush=True)
                fails += not ok
        except Exception as e:
            fails += 1; print(f"FAIL [{br}] {type(e).__name__}: {str(e)[:300]}", flush=True)
    return fails


def run(testn):
    fails = 0
    for br in ("chromium", "firefox"):
        try:
            with Session(br, testn=testn, outdir="shots") as s:
                s.sign_in("ada"); s.open("drive", tab="photos"); time.sleep(2)
                s.hook("home:folder", path=FOLDER); time.sleep(2)
                s.hook("home:photos:enable"); time.sleep(6)
                ph = find(s.snapshot(), "photos")
                print(f"  [{br}] photos state: { {k: (v if not isinstance(v, (list, dict)) else len(v)) for k, v in (ph or {}).items()} }", flush=True)
                s.hook("home:photos:open", index=0); time.sleep(3)
                s.hook("home:photos:edit", open=True); time.sleep(3)
                s.hook("home:photos:edit:rotate"); time.sleep(0.5)
                before = find(s.snapshot(), "photo_edit") or {}
                s.hook("home:photos:edit:copy"); time.sleep(4)
                st = find(s.snapshot(), "photo_edit") or {}
                s.shot(f"{br}-after-copy")
                print(f"  [{br}] after copy: photo_edit={ {k: v for k, v in st.items() if k != 'rows'} }", flush=True)
                s.hook("home:photos:edit:cancel"); time.sleep(1)
                after = find(s.snapshot(), "photo_edit") or {}
                clean = (before.get("dirty") is True and st.get("copied") is True and st.get("dirty") is False
                         and st.get("unsaved") is True and not after.get("ask"))
                s.shot(f"{br}-left")
                # A change made AFTER the copy must ask again.
                s.hook("home:photos:open", index=0); time.sleep(3)
                s.hook("home:photos:edit", open=True); time.sleep(3)
                s.hook("home:photos:edit:rotate"); time.sleep(0.5)
                s.hook("home:photos:edit:copy"); time.sleep(4)
                s.hook("home:photos:edit:rotate"); time.sleep(0.5)
                again = find(s.snapshot(), "photo_edit") or {}
                s.hook("home:photos:edit:cancel"); time.sleep(1)
                asked = (find(s.snapshot(), "photo_edit") or {}).get("ask")
                clean = clean and again.get("dirty") is True and bool(asked)
                print(f"  [{br}] change after copy: dirty={again.get('dirty')} asks={bool(asked)}", flush=True)
                shot_txt = str(s.snapshot())
                no_raw = "no such file" not in shot_txt
                ok = clean and no_raw
                print(f"{'PASS' if ok else 'FAIL'} [{br}] before.dirty={before.get('dirty')} copied={st.get('copied')} "
                      f"dirty={st.get('dirty')} unsaved={st.get('unsaved')} ask_after_cancel={after.get('ask')} no_raw_no_such_file={no_raw}", flush=True)
                fails += not ok
        except Exception as e:
            fails += 1; print(f"FAIL [{br}] {type(e).__name__}: {str(e)[:200]}", flush=True)
    return fails


if __name__ == "__main__":
    a = sys.argv[1:]
    testn = int(a[a.index("--testn") + 1])
    if "--remote" not in a:
        out = os.environ.get("SHOTS", "/tmp/photos-bugs-shots")
        sys.exit(ship_and_run(os.path.abspath(__file__), a, "/tmp/photosbugs", pyname="photos_bugs_input.py", shots_to=out))
    sys.exit(1 if (stale(testn) if "--stale" in a else run(testn)) else 0)
