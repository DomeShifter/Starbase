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
                # a CSV whose first column is leading-zero text-numbers: no green corner on any of them
                csv = b"code,qty,zip\n00123,1,'00123\n007,2,'007\n0420,3,'0420\n"
                r = s.pg.request.post(f"{s.base}/api/files?name=ntw-zip.csv&as_is=1", data=csv,
                                      headers={"content-type": "application/octet-stream"})
                assert r.status < 300, f"upload csv: {r.status} {r.text()[:200]}"
                s.open("sheets", path=f"/?file={r.json()['id']}&test"); time.sleep(4)
                bad = []
                for y, x in [(196, 75), (216, 75), (236, 75), (196, 203), (216, 203), (236, 203)]:  # data cells of col A (0-prefixed) and C (apostrophe text)
                    s.click_xy(x, y)
                    c = s.snapshot().get("cell") or {}
                    print(f"  [{br}] x={x} y={y} input={c.get('input')!r} isText={c.get('isText')} markers={c.get('numberTextMarkers')}", flush=True)
                    if c.get("numberTextMarkers") != 0 or c.get("input") in ("", None): bad.append((x, y))
                s.shot(f"{br}-csv-leading-zero")
                ok2 = not bad
                print(f"{'PASS' if ok2 else 'FAIL'} [{br}] csv leading-zero column has no markers", flush=True)
                fails += not ok2
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
