#!/usr/bin/env python3
"""Check that pressing Enter in Locked submits only the password form."""

import argparse
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path.home() / ".claude/tools"))
from starbase_ui import Session, USERS, ship_and_run


def run(browser, testn):
    with Session(browser, testn=testn, outdir="shots") as s:
        s.sign_in("ada")
        s.open("drive", path="/?test&dir=ada/Photos")
        s.click_text("item:Photo 1.jpg")
        s.click_text("drive:locked")
        assert s.wait_for(lambda: s.snapshot("home.photos.albums.tab") == "locked", secs=12)
        assert s.wait_for(lambda: s.snapshot("home.photos.locked.checked"), secs=12)
        if s.snapshot("home.photos.locked.unlocked"):
            s.hook("home:photos:locked:lock")
        s.click_text("photos:locked:password")
        subprocess.run(["xdotool", "type", USERS["ada"]], check=True)
        s.key("Return")
        assert s.wait_for(lambda: s.snapshot("home.photos.locked.unlocked"), secs=12)
        assert s.snapshot("home.photos.albums.tab") == "locked"
        assert s.snapshot("home.preview") is None and s.snapshot("home.photos.viewer") is None
        s.hook("home:photos:locked:lock")
        print(f"PASS {browser}: Enter unlocked the folder without opening the Drive selection", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--testn", type=int, required=True)
    parser.add_argument("--remote", action="store_true")
    args = parser.parse_args()
    if not args.remote:
        sys.exit(ship_and_run(__file__, sys.argv[1:], "/tmp/codex-locked-enter"))
    for browser in ("chromium", "firefox"):
        run(browser, args.testn)
