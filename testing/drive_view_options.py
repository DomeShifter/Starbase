"""Real-input check of Drive's [List | ▾] / [Grid | ▾] view options (needs ~/.claude/tools/starbase_ui.py).
Run: python3 testing/drive_view_options.py --testn 31 [--shots DIR]   (ships itself to the build box; Chromium + Firefox)."""
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
sys.path.insert(0, ".")
import starbase_ui as ui  # noqa: E402


def run(br, testn):
    with ui.Session(br, testn=testn, outdir="shots") as s:
        s.sign_in("ada")
        s.open("drive")
        s.click_text("List")
        s.click_text("List options")
        s.click_text("Size")
        s.key("Escape")
        s.open("drive")
        cols = s.snapshot("home.view_opts.cols") or ""
        ok1 = "-size" in cols
        x, y = s._locate("List column edge Last modified")
        s._at(x, y)
        subprocess.run(["xdotool", "mousedown", "1"])
        time.sleep(0.3)
        for i in range(1, 9):
            s._at(x - 10 * i, y)
        subprocess.run(["xdotool", "mouseup", "1"])
        time.sleep(1)
        s.open("drive")
        cols = s.snapshot("home.view_opts.cols") or ""
        ok2 = "modified=" in cols and float(cols.split("modified=")[1].split(",")[0]) > 200
        s.shot(f"{br}-list")
        s.click_text("Grid")
        s.click_text("Grid options")
        s.click_text("Large")
        s.click_text("Show previews")
        s.key("Escape")
        s.open("drive")
        grid = s.snapshot("home.view_opts.grid")
        lst = s.snapshot("home.list")
        ok3 = grid == "012" and lst is False
        s.shot(f"{br}-grid")
        print(f"{br}: hide Size+reload={ok1} drag wider+reload={ok2} ({cols}) Large+previews off+reload={ok3} ({grid})", flush=True)
        return ok1 and ok2 and ok3


if __name__ == "__main__":
    a = sys.argv
    testn = int(a[a.index("--testn") + 1])
    if "--remote" in a:
        sys.exit(0 if all([run(b, testn) for b in ("chromium", "firefox")]) else 1)
    shots = a[a.index("--shots") + 1] if "--shots" in a else None
    sys.exit(ui.ship_and_run(__file__, ["--testn", str(testn)], rd="/tmp/dv-viewopts", shots_to=shots))
