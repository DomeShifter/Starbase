#!/usr/bin/env python3
"""Real-input check of StarDocs keyboard shortcuts (real X key presses through xdotool).

Types text, selects the last word with Ctrl+Shift+Left, presses Ctrl+B, Ctrl+I, Ctrl+U and
Ctrl+E, reads the formatting back through the test hook, then checks Ctrl+Z undoes a step.
Run on the build box inside xvfb-run, with starbase_ui.py next to it:
    python3 docs-shortcuts-input.py <chromium|firefox> <test server number>
"""
import json
import subprocess
import sys
import time

from starbase_ui import Session


def state(s, cmd):
    try:
        return s.hook(cmd)
    except AssertionError as ex:
        return {"error": str(ex)[:120]}


def main():
    args = [a for a in sys.argv[1:] if a != "--remote"]
    browser, testn = args[0], int(args[1])
    with Session(browser, testn=testn) as s:
        s.sign_in("ada")
        s.open("docs")
        time.sleep(3)
        s.click_xy(640, 420)
        subprocess.run(["xdotool", "type", "--delay", "60", "plain words"], check=True)
        time.sleep(1)
        s.key("ctrl+shift+Left")
        for k in ["ctrl+b", "ctrl+i", "ctrl+u", "ctrl+e"]:
            s.key(k)
        f = state(s, "format.state")
        p = state(s, "para.state")
        s.shot(f"{browser}-after")
        s.key("ctrl+z")
        p2 = state(s, "para.state")
        s.key("ctrl+z")
        f2 = state(s, "format.state")
        res = {
            "browser": browser,
            "bold": f.get("bold"), "italic": f.get("italic"), "underline": f.get("underline"),
            "align": p.get("align") if isinstance(p, dict) else p,
            "align_after_undo": p2.get("align") if isinstance(p2, dict) else p2,
            "underline_after_2nd_undo": f2.get("underline"),
            "still_on_page": s.pg.url.startswith(s.base),
        }
        print("RESULT " + json.dumps(res), flush=True)


if __name__ == "__main__":
    main()
