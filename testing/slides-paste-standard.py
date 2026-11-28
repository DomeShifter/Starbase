#!/usr/bin/env python3
"""Real-input check of the Paste standard in Slides (crate starbase-clipboard).

Runs on the build box (xvfb + Playwright, ships itself via starbase_ui.ship_and_run), in Chromium AND Firefox.

1. Own copy: a slide with a text box "own"; select the box, Ctrl+C. Right-click the slide background > Paste (the
   engine's slide menu: Cut/Copy/Paste at the top) pastes it straight away: no prompt (ownCopyCurrent true,
   pastePromptShown false), the slide gets a second shape.
2. Blur (second browser window, "outside" on the system clipboard with xclip; Firefox: dispatched blur event):
   ownCopyCurrent false. Right-click > Paste: the "Press Ctrl+V now" card (pastePromptShown true, target "paste prompt").
   Ctrl+V: card gone and a shape with "outside" appears.
3. Again blur + clipboard, Paste, tick "Don't explain again", Ctrl+V; a third time the card has no reason line.
4. The card's close button closes it without pasting.

UNRUN: written by the build helper, adjust the slide helpers below (how a slide's shapes/text are read) to the real
test hook if they differ. Usage: python3 -I testing/slides-paste-standard.py [--testn 27] [--browsers chromium,firefox]
Needs `TESTN=<n> starsheets-testserver.sh up` (slides) first.
"""

import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
from starbase_ui import Session, ship_and_run  # noqa: E402

DONT = "paste prompt: don't explain again"


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
    xfocus("Nightly" if s.br == "firefox" else "Untitled presentation")
    time.sleep(1.0)


def slide_texts(s):
    """The text of every shape on the current slide (via the hook's shape list)."""
    r = s.hook("shape.list") or {}
    shapes = r.get("shapes", r) if isinstance(r, dict) else r
    return [str(x.get("text", "")) for x in (shapes or []) if isinstance(x, dict)]


def right_click_paste(s):
    """Right-click an empty part of the slide; the engine menu lists Cut, Copy, Paste first."""
    s.right_click_xy(600, 420)
    s.shot("menu")
    s.click_text("Paste")


def run(br, testn, out):
    with Session(br, testn=testn, outdir=out) as s:
        s.base = f"http://192.168.178.100:{3100 + testn}"  # plain http: the prompt is for it
        s.sign_in("ada")
        s.open("slides")
        if br == "chromium":
            s.ctx.new_cdp_session(s.pg).send("Emulation.setFocusEmulationEnabled", {"enabled": False})
        s.hook("insert.textbox", text="own")
        s.key("ctrl+c")
        assert s.snapshot("ownCopyCurrent"), "own copy not remembered after Ctrl+C"
        n0 = len(slide_texts(s))
        right_click_paste(s)
        time.sleep(1.0)
        assert not s.snapshot("pastePromptShown"), "prompt shown although the copy was our own"
        assert len(slide_texts(s)) == n0 + 1, "own copy not pasted"
        print(f"PASS [{br}] own copy pastes directly, no prompt", flush=True)

        blur_and_clip(s, "outside")
        assert not s.snapshot("ownCopyCurrent"), "own copy kept after blur"
        right_click_paste(s)
        assert s.wait_for(lambda: s.snapshot("pastePromptShown")), "paste prompt did not show"
        assert "paste prompt" in s.snapshot("targets")
        s.key("ctrl+v")
        assert s.wait_for(lambda: not s.snapshot("pastePromptShown")), "prompt did not close on Ctrl+V"
        assert any("outside" in t for t in slide_texts(s)), slide_texts(s)
        print(f"PASS [{br}] blur + outside clipboard: prompt, Ctrl+V pastes 'outside'", flush=True)

        blur_and_clip(s, "outside2")
        right_click_paste(s)
        assert s.wait_for(lambda: s.snapshot("pastePromptShown"))
        s.click_text(DONT)
        time.sleep(1.0)
        s.key("ctrl+v")
        assert s.wait_for(lambda: not s.snapshot("pastePromptShown"))
        blur_and_clip(s, "outside3")
        right_click_paste(s)
        assert s.wait_for(lambda: s.snapshot("pastePromptShown"))
        assert DONT not in s.snapshot("targets"), "reason line still shown"
        print(f"PASS [{br}] don't explain again", flush=True)

        n1 = len(slide_texts(s))
        s.click_text("paste prompt: close")
        assert s.wait_for(lambda: not s.snapshot("pastePromptShown")), "close did not close"
        assert len(slide_texts(s)) == n1, "close pasted"
        print(f"PASS [{br}] close button closes without pasting", flush=True)
    return True


def main():
    a = sys.argv[1:]
    if "--remote" not in a:
        return ship_and_run(os.path.abspath(__file__), a, "/tmp/slides_paste_standard", pyname="slides_paste_standard.py",
                            shots_to=os.path.join(os.getcwd(), "shots", "slides-paste-standard"))
    testn = int(a[a.index("--testn") + 1]) if "--testn" in a else 27
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
