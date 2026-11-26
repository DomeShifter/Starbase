#!/usr/bin/env python3
"""Real-input check: Drive's "Locked" entry and Photo Mode's one-time Locked folder note.

Seed first (pictures in a folder with Photos on):
    python3 -I testing/seed.py --url <ready: URL> --set gps        # prints the folder, e.g. 'ada/seed-gps'
Run from a workstation:
    python3 testing/drive-locked-folder.py <base-url> [chromium|firefox ...] [--folder ada/seed-gps]
(ships itself + ~/.claude/tools/starbase_ui.py to the build box and runs there under Xvfb).

Each browser starts with clean local storage, so the note is new there. Steps, all real clicks:
 1. Drive, Recent (no folder shown): the left list has "Locked" (target drive:locked).
 2. Click it: Photo Mode opens at the Locked tab (home.photos.shown, albums.tab locked, asks for the password).
 3. Unlock with the password typed in the field: unlocked.
 4. Open the seeded folder in Photo Mode, Photos tab: the note shows (photos.lockedHint.shown).
 5. Click "Got it" (target photos:hint-locked:ok): gone, dismissed; after a reload it stays gone.
 6. Open Locked again, click the back arrow: Photo Mode is off again.
"""
import os, sys, time

sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import starbase_ui
from starbase_ui import Session


def tab(s):
    return (s.snapshot("home.photos.albums.tab") or "").lower()


def run(browser, base, folder):
    with Session(browser, base=base, outdir=f"shots/{browser}") as s:
        s.sign_in("ada").open("drive")
        ok = True
        # 1. Recent: no folder on screen, the entry is there.
        s.click_text("Recent")
        assert s.wait_for(lambda: s.snapshot("home.view") == "recent", secs=20), "Recent did not open"
        assert s.snapshot("home.lockedEntry") is True, "lockedEntry false"
        assert s.wait_text("drive:locked", secs=10), "no drive:locked target"
        # 2. Click it.
        s.click_text("drive:locked", settle=2)
        at = s.wait_for(lambda: s.snapshot("home.photos.shown") and tab(s) == "locked", secs=30)
        s.shot("1-locked-tab")
        print(f"{browser}: Locked entry -> Photo Mode at Locked: {bool(at)} view={s.snapshot('home.view')}")
        ok &= bool(at)
        # 3. Password (typed: click the field first).
        # The password field has no target yet: click it where the Locked tab draws it.
        s.click_xy(320, 178)
        for ch in starbase_ui.USERS.get("ada", "adapassword"):
            s.key(ch, settle=0.05)
        s.key("Return", settle=2)
        unlocked = s.wait_for(lambda: s.snapshot("home.photos.locked.unlocked"), secs=20)
        print(f"{browser}: unlocked={bool(unlocked)} error={s.snapshot('home.photos.locked.error')}")
        ok &= bool(unlocked)
        # 4. The seeded folder, Photos tab: the note.
        s.open("drive", path=f"/?test&dir={folder}")
        s.click_text("Photos", settle=2)
        shown = s.wait_for(lambda: s.snapshot("home.photos.lockedHint.shown"), secs=60)
        s.shot("2-hint")
        print(f"{browser}: hint shown={bool(shown)}")
        ok &= bool(shown)
        # 5. Got it.
        if shown:
            s.click_text("photos:hint-locked:ok", settle=2)
            gone = s.wait_for(lambda: s.snapshot("home.photos.lockedHint") == {"shown": False, "dismissed": True}, secs=10)
            s.open("drive", path=f"/?test&dir={folder}")
            time.sleep(5)
            still = s.snapshot("home.photos.lockedHint")
            s.shot("3-after-reload")
            print(f"{browser}: Got it -> gone={bool(gone)}; after reload {still}")
            ok &= bool(gone) and still == {"shown": False, "dismissed": True}
        # 6. Leaving Locked: the back arrow returns to Drive with Photo Mode off (not stuck on).
        s.open("drive")
        if s.snapshot("home.photos.shown"):  # Photo Mode is still on from step 4: leave it first
            s.click_xy(34, 32, settle=2)
        s.click_text("Recent")
        s.wait_for(lambda: s.snapshot("home.view") == "recent", secs=20)
        s.click_text("drive:locked", settle=2)
        s.wait_for(lambda: s.snapshot("home.photos.shown"), secs=30)
        s.click_xy(34, 32, settle=2)
        off = s.wait_for(lambda: s.snapshot("home.photos.shown") is False, secs=10)
        s.shot("4-after-back")
        print(f"{browser}: back arrow -> Photo Mode off: {bool(off)} view={s.snapshot('home.view')}")
        ok &= bool(off)
        return ok


if __name__ == "__main__":
    a = sys.argv[1:]
    if "--remote" not in a:
        sys.exit(starbase_ui.ship_and_run(__file__, a, rd="/tmp/locked-folder", pyname="t.py",
                                          shots_to=os.environ.get("SHOTS", "/tmp/locked-folder-shots")))
    a.remove("--remote")
    folder = "ada/seed-gps"
    if "--folder" in a:
        i = a.index("--folder"); folder = a[i + 1]; del a[i:i + 2]
    base, browsers = a[0], a[1:] or ["chromium", "firefox"]
    ok = True
    for b in browsers:
        try:
            r = run(b, base, folder)
        except Exception as e:
            print(f"{b}: ERROR {e}"); r = False
        print(f"{b}: {'PASS' if r else 'FAIL'}"); ok &= r
    sys.exit(0 if ok else 1)
