#!/usr/bin/env python3
"""Real-input check: Ctrl+E centres (and Ctrl+L goes back) the paragraph in StarDocs (real X keys), read back via the test hook.

Run from a workstation: python3 testing/docs-center-input.py <base-url> [chromium|firefox ...]
(ships itself + starbase_ui.py to the build box and runs there under Xvfb).
"""
import json, os, subprocess, sys, time

sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
import starbase_ui
from starbase_ui import Session


def tryhook(s, cmd):
    try:
        return s.hook(cmd)
    except AssertionError as ex:
        return {"error": str(ex)[:100]}


def run(browser, base):
    with Session(browser, base=base, outdir=f"shots/{browser}") as s:
        s.sign_in("ada"); s.open("docs"); time.sleep(3)
        s.click_xy(640, 420)
        subprocess.run(["xdotool", "type", "--delay", "60", "centre me please"], check=True)
        time.sleep(1)
        snap0 = s.snapshot()
        s.key("ctrl+e", settle=1.5)
        snap = s.snapshot()
        s.shot("after-ctrl-e")
        before, after = snap0.get("align"), snap.get("align")
        print(f"{browser}: align before={before!r} after={after!r}")
        s.key("ctrl+l", settle=1.5)
        left = s.snapshot().get("align")
        print(f"{browser}: after Ctrl+L align={left!r}")
        return after == "center" and left == "left"


if __name__ == "__main__":
    a = sys.argv[1:]
    if "--remote" not in a:
        sys.exit(starbase_ui.ship_and_run(__file__, a, rd="/tmp/docs-center", pyname="t.py",
                                          shots_to=os.environ.get("SHOTS", "/tmp/docs-center-shots")))
    a.remove("--remote")
    base, browsers = a[0], a[1:] or ["chromium", "firefox"]
    ok = True
    for b in browsers:
        try:
            ok &= run(b, base)
        except Exception as e:
            print(f"{b}: ERROR {e}"); ok = False
    sys.exit(0 if ok else 1)
