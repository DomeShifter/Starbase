#!/usr/bin/env python3
"""Real-input check of the Paste standard in Sheets (crate starbase-clipboard).

1. Sign in, open a sheet, put "own" in A1, select A1, Ctrl+C: snapshot ownCopyCurrent = true.
2. Blur the window (focus another X window), put "outside" on the clipboard with xclip.
3. Back in the sheet: snapshot ownCopyCurrent = false.
4. Right-click B2 > Paste: on http the "paste prompt" card shows (pastePromptShown = true).
5. Ctrl+V: B2 holds "outside" and pastePromptShown = false.

Usage: python3 -I testing/paste-standard.py --url <test server> [--browser chromium|firefox]
Needs xdotool and xclip on the machine that runs the browser. Uses ~/.claude/tools/starbase_ui.py.
"""

import argparse
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
import starbase_ui as ui  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", required=True)
    ap.add_argument("--browser", default="chromium")
    a = ap.parse_args()
    s = ui.session(a.url, browser=a.browser)
    s.sign_in()
    s.open_app("sheets")
    s.new_file()
    s.hook("cell.set", {"cell": "A1", "input": "own"})
    s.click_cell("A1")
    s.key("ctrl+c")
    assert s.state()["ownCopyCurrent"], "own copy not remembered"

    # Something else gets the focus and copies.
    subprocess.run(["xdotool", "search", "--name", "xterm|Terminal", "windowactivate", "--sync"], check=False)
    subprocess.run(["xclip", "-selection", "clipboard"], input=b"outside", check=True)
    time.sleep(0.5)
    s.focus()
    assert not s.state()["ownCopyCurrent"], "own copy kept after blur"

    s.right_click_cell("B2")
    s.click_menu("Paste")
    s.wait_state(lambda st: st.get("pastePromptShown"), "paste prompt did not show")
    s.key("ctrl+v")
    s.wait_state(lambda st: not st.get("pastePromptShown"), "prompt did not close on Ctrl+V")
    got = s.hook("cell.get", {"cell": "B2"})
    assert "outside" in str(got), f"B2 = {got!r}"
    print("PASS paste standard", a.browser)
    return 0


if __name__ == "__main__":
    sys.exit(main())
