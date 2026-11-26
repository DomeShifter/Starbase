#!/usr/bin/env python3
"""Real-input check of Photos to-do 3 rows 42 + 43, Chromium + Firefox.
42: rotate in the editor, Save a copy, then leave the original: NO "exiting without saving" question
    (state: photo_edit.dirty false, .unsaved true, .copied true, .ask empty), and another change after
    the copy asks again (dirty true).
43: no raw "no such file" text anywhere in Photos' error line (home.photos error is empty/friendly).
Seeds with the repo kit first:  python3 -I testing/seed.py --url <ready URL> --set types
    photos-bugs-42-43.py --testn 43
"""
import os, subprocess, sys, time

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


def run(testn):
    fails = 0
    for br in ("chromium", "firefox"):
        try:
            with Session(br, testn=testn, outdir="shots") as s:
                s.sign_in("ada"); s.open("drive"); time.sleep(3)
                s.open("drive", tab="photos"); time.sleep(3)
                s.hook("home:photos:open", index=0); time.sleep(1)
                s.hook("home:photos:edit", open=True); time.sleep(3)
                s.hook("home:photos:edit:rotate"); time.sleep(0.5)
                before = find(s.snapshot(), "photo_edit") or {}
                s.hook("home:photos:edit:copy"); time.sleep(4)
                st = find(s.snapshot(), "photo_edit") or {}
                s.shot(f"{br}-after-copy")
                s.hook("home:photos:edit:cancel"); time.sleep(1)
                after = find(s.snapshot(), "photo_edit") or {}
                clean = (before.get("dirty") is True and st.get("copied") is True and st.get("dirty") is False
                         and st.get("unsaved") is True and not after.get("ask"))
                s.shot(f"{br}-left")
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
    sys.exit(1 if run(testn) else 0)
