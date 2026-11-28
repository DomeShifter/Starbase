#!/usr/bin/env python3
"""Real-input check of the Paste standard in Docs (crate starbase-clipboard, stardocs/app/src/pasting.rs).

Runs on the build box (xvfb + Playwright, ships itself via starbase_ui.ship_and_run), in Chromium AND Firefox.
Needs `SLOT=<slot> TESTN=<n> starsheets-testserver.sh up` first.

1. Own copy: type "own", select it (Shift+Home), Ctrl+C. Click the end of the line, Edit menu > Paste: pastes at once,
   no prompt (snapshot ownCopyCurrent true, pastePromptShown false), text becomes "ownown".
2. Blur: another window takes the focus, "outside" goes on the system clipboard (xclip), the Docs tab returns:
   ownCopyCurrent false.
3. Edit > Paste: the "Press Ctrl+V now" card shows (pastePromptShown true, target "paste prompt"). Ctrl+V: the text
   gains "outside", the card is gone.
4. Blur + clipboard again, Paste, tick "Don't explain again", Ctrl+V; a third time the card has no reason line.
5. The card's close button closes it without pasting.

Usage:  python3 -I testing/docs-paste-standard.py [--testn 25] [--browsers chromium,firefox]
"""

import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
from starbase_ui import Session, ship_and_run  # noqa: E402

DONT = "paste prompt: don't explain again"


def text(s):
    return s.snapshot("text") or ""


def edit_paste(s):
    s.click_text("menu:Edit")
    time.sleep(0.8)
    s.click_text("Paste")
    time.sleep(1.0)


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
    xfocus("Nightly" if s.br == "firefox" else "Untitled document")
    time.sleep(1.0)


def run(br, testn, out):
    with Session(br, testn=testn, outdir=out) as s:
        # Plain http by address (not the localhost forward): the browser cannot read the clipboard there.
        s.base = f"http://192.168.178.100:{3100 + testn}"
        s.sign_in("ada")
        s.open("docs")
        time.sleep(3)
        if br == "chromium":  # Playwright fakes page focus; a real X focus change must blur the page
            s.ctx.new_cdp_session(s.pg).send("Emulation.setFocusEmulationEnabled", {"enabled": False})
        s.click_xy(640, 420)
        subprocess.run(["xdotool", "type", "--delay", "60", "own"], check=True)
        time.sleep(0.8)
        s.key("shift+Home")
        s.key("ctrl+c")
        assert s.snapshot("ownCopyCurrent"), "own copy not remembered after Ctrl+C"
        s.key("End")

        edit_paste(s)
        assert not s.snapshot("pastePromptShown"), "prompt shown although the copy was our own"
        assert text(s).strip() == "ownown", f"text = {text(s)!r}"
        print(f"PASS [{br}] own copy pastes directly, no prompt", flush=True)

        blur_and_clip(s, "outside")
        assert not s.snapshot("ownCopyCurrent"), "own copy kept after blur"
        s.click_xy(640, 420)
        s.key("End")
        edit_paste(s)
        assert s.wait_for(lambda: s.snapshot("pastePromptShown")), "paste prompt did not show"
        tg = s.snapshot("targets")
        assert "paste prompt" in tg and DONT in tg, f"card targets: {list(tg)}"
        s.shot("prompt")
        s.key("ctrl+v")
        assert s.wait_for(lambda: not s.snapshot("pastePromptShown")), "prompt did not close on Ctrl+V"
        assert "outside" in text(s), f"text = {text(s)!r}"
        print(f"PASS [{br}] blur + outside clipboard: prompt, Ctrl+V pastes 'outside'", flush=True)

        blur_and_clip(s, "second")
        s.click_xy(640, 420)
        s.key("End")
        edit_paste(s)
        assert s.wait_for(lambda: s.snapshot("pastePromptShown")), "second prompt did not show"
        s.click_text(DONT)
        time.sleep(1.0)
        s.key("ctrl+v")
        assert s.wait_for(lambda: not s.snapshot("pastePromptShown")), "second prompt did not close"
        assert "second" in text(s), f"text = {text(s)!r}"
        blur_and_clip(s, "third")
        s.click_xy(640, 420)
        s.key("End")
        edit_paste(s)
        assert s.wait_for(lambda: s.snapshot("pastePromptShown")), "third prompt did not show"
        assert DONT not in s.snapshot("targets"), "reason line still shown after Don't explain again"
        s.shot("prompt-short")
        print(f"PASS [{br}] don't explain again: third prompt has no reason line", flush=True)

        s.click_text("paste prompt: close")
        assert s.wait_for(lambda: not s.snapshot("pastePromptShown")), "close button did not close the prompt"
        assert "third" not in text(s), f"text = {text(s)!r} after close"
        print(f"PASS [{br}] close button closes the prompt without pasting", flush=True)
    return True


def main():
    a = sys.argv[1:]
    if "--remote" not in a:
        return ship_and_run(os.path.abspath(__file__), a, "/tmp/docs_paste_standard", pyname="docs_paste_standard.py",
                            shots_to=os.path.join(os.getcwd(), "shots", "docs-paste-standard"))
    testn = int(a[a.index("--testn") + 1]) if "--testn" in a else 25
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
