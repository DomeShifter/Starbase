#!/usr/bin/env python3
"""Real-input check of the save-state icon (to-do 3 row 38) in Sheets and Docs, Chromium + Firefox.
Types a change with xdotool, records the saveIcon states seen (expects unsaved/saving, then saved), hovers the
icon (the card target must appear), clicks it (version history must open). Screenshots: floppy + card per app and
browser. Needs the shared click library starbase_ui.py (STARBASE_UI_DIR, default ~/.claude/tools).
    save-icon-input.py --testn 32 [--apps sheets,docs]
"""
import os, subprocess, sys, time

sys.path.insert(0, os.environ.get("STARBASE_UI_DIR", os.path.expanduser("~/.claude/tools")))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from starbase_ui import Session, ship_and_run  # noqa: E402


def run(testn, apps):
    fails = 0
    for br in ("chromium", "firefox"):
        for app in apps:
            tag = f"{br}-{app}"
            try:
                with Session(br, testn=testn, outdir="shots") as s:
                    s.sign_in("ada"); s.open(app); time.sleep(3)
                    if app == "sheets": s.click_xy(400, 330)
                    else: s.click_xy(640, 420)
                    subprocess.run(["xdotool", "type", "--delay", "60", "hello save icon"])
                    if app == "sheets": s.key("Return", settle=0.1)
                    seen, shot = [], False
                    end = time.time() + 60
                    while time.time() < end:
                        k = s.snapshot("saveIcon")
                        if k and (not seen or seen[-1] != k): seen.append(k)
                        if k in ("unsaved", "saving") and not shot: s.shot(f"{tag}-floppy"); shot = True
                        if k == "saved" and len(seen) > 1: break
                        time.sleep(0.15)
                    ok_states = any(k in ("unsaved", "saving") for k in seen) and seen[-1:] == ["saved"]
                    x, y = s._locate("save-icon"); s._at(x, y); time.sleep(2.0)
                    card = bool((s.snapshot().get("targets") or {}).get("save-icon-card"))
                    s.shot(f"{tag}-card")
                    s.click_xy(x, y, settle=3)
                    st = s.snapshot()
                    hist = bool(st.get("history")) if app == "sheets" else "true" in str(st).lower().split("versionhistory")[-1][:12]
                    ok = ok_states and card and hist
                    print(f"{'PASS' if ok else 'FAIL'} [{tag}] states={seen} card={card} history={hist}", flush=True)
                    fails += not ok
            except Exception as e:
                fails += 1; print(f"FAIL [{tag}] {type(e).__name__}: {str(e)[:200]}", flush=True)
    return fails


if __name__ == "__main__":
    a = sys.argv[1:]
    testn = int(a[a.index("--testn") + 1])
    apps = a[a.index("--apps") + 1].split(",") if "--apps" in a else ["sheets", "docs"]
    if "--remote" not in a:
        out = os.environ.get("SHOTS", "/tmp/save-icon-shots")
        sys.exit(ship_and_run(os.path.abspath(__file__), a, "/tmp/saveicon", pyname="save_icon_input.py", shots_to=out))
    sys.exit(1 if run(testn, apps) else 0)
