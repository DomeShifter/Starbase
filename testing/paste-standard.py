#!/usr/bin/env python3
"""Real-input check of the Paste standard in Sheets (crate starbase-clipboard).

Runs on the build box (xvfb + Playwright, ships itself via starbase_ui.ship_and_run), in Chromium AND Firefox.

1. Own copy: A1 = "own", Ctrl+C. Right-click B2 > Paste pastes it straight away: no prompt, B2 = "own"
   (snapshot ownCopyCurrent true, pastePromptShown false).
2. Blur: another tab comes to the front (the page is hidden), "outside" goes on the system clipboard with
   xclip, the sheet tab comes back: ownCopyCurrent false.
3. Right-click C3 > Paste: the "Press Ctrl+V now" card shows (pastePromptShown true, target "paste prompt").
   Ctrl+V: C3 = "outside", prompt gone.
4. Again blur + clipboard, Paste: the card shows with its reason line; tick "Don't explain again"; Ctrl+V.
   A third time: the card shows without the reason line (targets lack "paste prompt: don't explain again").
5. The card's close button closes it without pasting.

Usage (from the repo root, anywhere):  python3 -I testing/paste-standard.py [--testn 36] [--browsers chromium,firefox]
Needs `TESTN=<n> starsheets-testserver.sh up` first. Cells are located by the grid geometry (col 100 px wide from
x=44, rows 21 px high from y=165 at 100% zoom); the menu rows by the page's right-click menu layout.
"""

import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
from starbase_ui import Session, ship_and_run  # noqa: E402

DONT = "paste prompt: don't explain again"


def cell_xy(a1):
    col = ord(a1[0]) - 65
    row = int(a1[1:]) - 1
    return 44 + 100 * col + 50, 165 + 21 * row + 10


def val(s, a1):
    for c, t in s.snapshot("cells") or []:
        if c == a1:
            return t
    # `cells` is the selection only: read the cell itself
    s.click_xy(*cell_xy(a1))
    return dict((c, t) for c, t in (s.snapshot("cells") or [])).get(a1)


def right_click_paste(s, a1):
    x, y = cell_xy(a1)
    s.right_click_xy(x, y)
    s.shot(f"menu-{a1}")
    # The menu is taller than the window, so it sits at the top (rows Cut 83, Copy 115, Paste 147) and 4 px
    # right of the pointer; it has no hook targets.
    s.click_xy(x + 60, 147)


def xfocus(name):
    for _ in range(5):
        ids = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", name], capture_output=True, text=True).stdout.split()
        if ids:
            break
        time.sleep(1.0)
    if not ids:
        allw = subprocess.run("for w in $(xdotool search --onlyvisible --name .); do xdotool getwindowname $w; done",
                              shell=True, capture_output=True, text=True).stdout.split("\n")
        raise AssertionError(f"no X window named {name!r}; windows: {allw[:8]}")
    subprocess.run(["xdotool", "windowfocus", ids[-1]], check=True)


def blur_and_clip(s, text):
    """A real second browser window takes the X focus (Playwright pages fake focus, so a second tab would not blur)."""
    if not getattr(s, "other", None):
        s.other = s.pw.chromium.launch(headless=False, args=["--window-size=300,200", "--window-position=1300,0"])
        pg = s.other.new_page()
        pg.set_content("<title>paste-other-window</title><body>other</body>")
        time.sleep(1.0)
    xfocus("paste-other-window")
    time.sleep(1.0)
    subprocess.run(["xclip", "-selection", "clipboard"], input=text.encode(), check=True)
    time.sleep(0.5)
    if s.br == "firefox":  # Playwright's Firefox fakes page focus and ignores the X focus: send the blur event itself
        s.pg.evaluate("window.dispatchEvent(new Event('blur'))")
        time.sleep(0.5)
    s.pg.bring_to_front()
    xfocus("Nightly" if s.br == "firefox" else "Untitled spreadsheet")
    time.sleep(1.0)


def run(br, testn, out):
    with Session(br, testn=testn, outdir=out) as s:
        # Session forwards through localhost (a secure context, where the browser can read the clipboard). The prompt
        # is for plain http, so go to the test server by its address instead.
        s.base = f"http://192.168.178.100:{3100 + testn}"
        s.sign_in("ada")
        s.open("sheets")
        if br == "chromium":  # Playwright fakes page focus; switch that off so a real X focus change blurs the page
            s.ctx.new_cdp_session(s.pg).send("Emulation.setFocusEmulationEnabled", {"enabled": False})
        s.hook("cell.set", cell="A1", input="own")
        s.click_xy(*cell_xy("A1"))
        s.key("ctrl+c")
        assert s.snapshot("ownCopyCurrent"), "own copy not remembered after Ctrl+C"

        # happy path: no blur, Paste goes straight in
        right_click_paste(s, "B2")
        time.sleep(1.0)
        assert not s.snapshot("pastePromptShown"), "prompt shown although the copy was our own"
        assert val(s, "B2") == "own", f"B2 = {val(s, 'B2')!r}"
        print(f"PASS [{br}] own copy pastes directly, no prompt", flush=True)

        # outside clipboard: prompt
        blur_and_clip(s, "outside")
        assert not s.snapshot("ownCopyCurrent"), "own copy kept after blur"
        right_click_paste(s, "C3")
        assert s.wait_for(lambda: s.snapshot("pastePromptShown")), "paste prompt did not show"
        tg = s.snapshot("targets")
        assert "paste prompt" in tg and DONT in tg, f"card targets: {list(tg)}"
        s.shot("prompt")
        s.key("ctrl+v")
        assert s.wait_for(lambda: not s.snapshot("pastePromptShown")), "prompt did not close on Ctrl+V"
        assert val(s, "C3") == "outside", f"C3 = {val(s, 'C3')!r}"
        print(f"PASS [{br}] blur + outside clipboard: prompt, Ctrl+V pastes 'outside'", flush=True)

        # don't explain again
        blur_and_clip(s, "outside2")
        right_click_paste(s, "D4")
        assert s.wait_for(lambda: s.snapshot("pastePromptShown")), "second prompt did not show"
        s.click_text(DONT)
        time.sleep(1.0)
        s.key("ctrl+v")
        assert s.wait_for(lambda: not s.snapshot("pastePromptShown")), "second prompt did not close"
        assert val(s, "D4") == "outside2", f"D4 = {val(s, 'D4')!r}"
        blur_and_clip(s, "outside3")
        right_click_paste(s, "E5")
        assert s.wait_for(lambda: s.snapshot("pastePromptShown")), "third prompt did not show"
        assert DONT not in s.snapshot("targets"), "reason line still shown after Don't explain again"
        s.shot("prompt-short")
        print(f"PASS [{br}] don't explain again: third prompt has no reason line", flush=True)

        # close button
        s.click_text("paste prompt: close")
        assert s.wait_for(lambda: not s.snapshot("pastePromptShown")), "close button did not close the prompt"
        assert val(s, "E5") in (None, ""), f"E5 = {val(s, 'E5')!r} after close"
        print(f"PASS [{br}] close button closes the prompt without pasting", flush=True)
    return True


def main():
    a = sys.argv[1:]
    if "--remote" not in a:
        return ship_and_run(os.path.abspath(__file__), a, "/tmp/paste_standard", pyname="paste_standard.py",
                            shots_to=os.path.join(os.getcwd(), "shots", "paste-standard"))
    testn = int(a[a.index("--testn") + 1]) if "--testn" in a else 36
    brs = (a[a.index("--browsers") + 1] if "--browsers" in a else "chromium,firefox").split(",")
    bad = 0
    for br in brs:
        try:
            run(br, testn, f"shots/{br}")
        except Exception as ex:  # noqa: BLE001
            bad += 1
            print(f"FAIL [{br}] {type(ex).__name__}: {str(ex)[:300]}", flush=True)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
