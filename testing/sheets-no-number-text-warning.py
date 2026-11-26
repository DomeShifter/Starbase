#!/usr/bin/env python3
"""Real-input check (to-do 3 row 44): a number stored as text gets NO green corner / warning button in Sheets.
Types '123 (text) into a cell with xdotool, selects it, expects snapshot cell.isText true, cell.number null,
cell.numberTextMarkers 0 and no "Error checking options" target; screenshot to look at the cell corner.
Chromium + Firefox. Needs starbase_ui.py (STARBASE_UI_DIR, default ~/.claude/tools).
    sheets-no-number-text-warning.py --testn 22
"""
import os, subprocess, sys, time

sys.path.insert(0, os.environ.get("STARBASE_UI_DIR", os.path.expanduser("~/.claude/tools")))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from starbase_ui import Session, ship_and_run  # noqa: E402


def run(testn):
    fails = 0
    for br in ("chromium", "firefox"):
        try:
            with Session(br, testn=testn, outdir="shots") as s:
                s.sign_in("ada"); s.open("sheets"); time.sleep(3)
                s.click_xy(400, 330)
                subprocess.run(["xdotool", "type", "--delay", "60", "'123"])
                s.key("Return", settle=0.3)
                s.key("Up", settle=0.5)  # back on the cell: the warning button would show beside it
                time.sleep(1)
                st = s.snapshot()
                cell = st.get("cell") or {}
                tg = st.get("targets") or {}
                s.shot(f"{br}-number-text")
                ok = cell.get("isText") is True and cell.get("number") is None and cell.get("numberTextMarkers") == 0 and not any("error-check" in k or "warning" in k for k in tg)
                print(f"{'PASS' if ok else 'FAIL'} [{br}] cell={cell.get('input')!r} isText={cell.get('isText')} number={cell.get('number')} markers={cell.get('numberTextMarkers')}", flush=True)
                fails += not ok
        except Exception as e:
            fails += 1; print(f"FAIL [{br}] {type(e).__name__}: {str(e)[:200]}", flush=True)
    return fails


if __name__ == "__main__":
    a = sys.argv[1:]
    testn = int(a[a.index("--testn") + 1])
    if "--remote" not in a:
        out = os.environ.get("SHOTS", "/tmp/ntw-shots")
        sys.exit(ship_and_run(os.path.abspath(__file__), a, "/tmp/ntw", pyname="ntw_input.py", shots_to=out))
    sys.exit(1 if run(testn) else 0)
