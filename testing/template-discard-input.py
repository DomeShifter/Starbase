#!/usr/bin/env python3
"""Real-input check: a file made from a template and closed untouched is removed; edited, it is kept.

Run from a workstation: python3 testing/template-discard-input.py <base-url> [chromium|firefox ...]
(ships itself + ~/.claude/tools/starbase_ui.py to the build box and runs there under Xvfb).
"""
import os, sys, time

sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import starbase_ui
from starbase_ui import Session

TEMPLATE = "To-do tracker"


def exists(s, fid):
    return s.ctx.request.get(f"{s.base}/api/files/{fid}").status == 200


def body(s, fid):
    return s.ctx.request.get(f"{s.base}/api/files/{fid}").body()


def from_template(s, home):
    """Drive: New -> From a template... -> the card, all real clicks; returns the new tab and file id."""
    s.pg = home; s.hookname = "starsheets"
    s.click_text("New"); s.click_text("From a template…")
    with s.ctx.expect_page(timeout=60000) as pi:
        s.click_text(f"template:{TEMPLATE}")
    tab = pi.value
    tab.wait_for_function("window.starsheets", timeout=180000)
    s.pg = tab
    st = s.wait_for(lambda: (lambda d: d if d.get("screen") == "sheet" and d.get("fileId") else None)(s.snapshot()), secs=90)
    assert st, f"new tab never showed the sheet: {s.snapshot().get('screen')}"
    return tab, st["fileId"]


def run(browser, base):
    with Session(browser, base=base, outdir=f"shots/{browser}") as s:
        s.sign_in("ada"); s.open("drive"); home = s.pg
        # 1. untouched: Ctrl+W closes the tab -> the file is gone
        tab, fid = from_template(s, home)
        assert exists(s, fid), "new file not on the server"
        s.shot("untouched-open")
        s.key("ctrl+w", settle=3)
        gone = s.wait_for(lambda: not exists(s, fid), secs=15)
        print(f"{browser}: untouched {fid} -> {'removed' if gone else 'STILL THERE'}")
        # 2. edited: type into a cell, wait for the save, Ctrl+W -> the file stays
        tab, fid2 = from_template(s, home)
        before = body(s, fid2)
        s.click_xy(640, 450); s.key("Z"); s.key("Return", settle=2)
        saved = s.wait_for(lambda: body(s, fid2) != before, secs=30)
        s.shot("edited-open")
        s.key("ctrl+w", settle=3); time.sleep(5)
        kept = exists(s, fid2)
        print(f"{browser}: edited {fid2} saved={bool(saved)} -> {'kept' if kept else 'REMOVED'}")
        s.pg = home; s.shot("drive-after")
        return bool(gone) and bool(saved) and kept


if __name__ == "__main__":
    a = sys.argv[1:]
    if "--remote" not in a:
        sys.exit(starbase_ui.ship_and_run(__file__, a, rd="/tmp/tpl-discard", pyname="t.py",
                                          shots_to=os.environ.get("SHOTS", "/tmp/tpl-discard-shots")))
    a.remove("--remote")
    base, browsers = a[0], a[1:] or ["chromium", "firefox"]
    ok = True
    for b in browsers:
        try:
            r = run(b, base)
        except Exception as e:
            print(f"{b}: ERROR {e}"); r = False
        print(f"{b}: {'PASS' if r else 'FAIL'}"); ok &= r
    sys.exit(0 if ok else 1)
