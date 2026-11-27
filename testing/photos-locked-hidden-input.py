#!/usr/bin/env python3
"""Check that Locked controls are absent from the regular browser build."""

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path.home() / ".claude/tools"))
from starbase_ui import Session, ship_and_run


def run(browser, testn):
    with Session(browser, testn=testn, outdir="shots") as s:
        s.sign_in("ada")
        s.open("drive", path="/?test&dir=ada/Photos")
        targets = lambda: s.snapshot("targets") or {}
        assert not s.snapshot("home.lockedEntry") and "drive:locked" not in targets()
        s.click_text("drive:photo_mode")
        assert s.wait_for(lambda: s.snapshot("home.photos.shown") and "Duplicates" in targets(), secs=15)
        assert "Locked" not in targets()
        s.click_text("photos:tile:0", button=3)
        assert s.wait_for(lambda: "Info" in targets(), secs=8)
        assert "Move to Locked folder…" not in targets()
        s.click_text("Info")
        assert s.wait_for(lambda: "viewer:info-panel" in targets(), secs=8)
        assert "viewer:info:lock" not in targets()
        print(f"PASS {browser}: Locked sidebar, menu and Info controls are hidden", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--testn", type=int, required=True)
    parser.add_argument("--remote", action="store_true")
    args = parser.parse_args()
    if not args.remote:
        sys.exit(ship_and_run(__file__, sys.argv[1:], "/tmp/codex-locked-hidden"))
    for browser in ("chromium", "firefox"):
        run(browser, args.testn)
