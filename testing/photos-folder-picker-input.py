#!/usr/bin/env python3
"""Verify Photos folder browsing and root-relative input on a disposable CT105 clone."""

import argparse
import sys
from pathlib import Path
from urllib.parse import urlencode

sys.path.insert(0, str(Path.home() / ".claude/tools"))
from starbase_ui import Session, ship_and_run


def check(browser, testn):
    with Session(browser, testn=testn, outdir="shots") as s:
        s.sign_in("ada")
        s.open("drive")
        api = s.base + "/api/library-roots"
        name = f"Picker {browser}"
        folder = s.pg.request.post(s.base + "/api/folders?" + urlencode({"name": name})).json()["path"]
        child = s.pg.request.post(s.base + "/api/folders?" + urlencode({"path": folder, "name": "Nested"})).json()["path"]
        s.click_text("account:avatar", button=3)
        s.click_text("tab:Photos")
        s.click_text("settings:photos-location")
        s.click_text("settings:photos-folder-choice")
        s.click_text("settings:photos-folder")
        s.pg.keyboard.press("Control+a")
        s.pg.keyboard.type("/" + name)
        s.click_text("settings:save-library")
        assert s.wait_for(lambda: s.pg.request.get(api).json()["selected"]["photos"] == {"folder": name}, secs=15)
        s.click_text("settings:photos-browse")
        assert s.wait_text(f"settings:photos-browse:{name}")
        s.click_text(f"settings:photos-browse:{name}")
        assert s.wait_text("settings:photos-browse:Nested")
        s.click_text("settings:photos-browse:Nested")
        assert s.wait_for(lambda: "settings:photos-browse:Nested" not in (s.snapshot("targets") or {}))
        s.click_text("settings:photos-browse-up")
        assert s.wait_text("settings:photos-browse:Nested")
        s.click_text("settings:photos-browse:Nested")
        assert s.wait_for(lambda: "settings:photos-browse:Nested" not in (s.snapshot("targets") or {}))
        s.click_text("settings:photos-browse-use")
        s.click_text("settings:save-library")
        assert s.wait_for(lambda: s.pg.request.get(api).json()["roots"]["photos"] == child, secs=15)
        s.key("Escape")
        s.click_text("drive:photo_mode")
        assert s.wait_for(lambda: s.snapshot("home.photos.folder") == child, secs=15)
        print(f"PASS {browser}: leading slash saves, nested browse goes up and down, and Photos opens the chosen folder", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--testn", type=int, required=True)
    parser.add_argument("--remote", action="store_true")
    args = parser.parse_args()
    if not args.remote:
        sys.exit(ship_and_run(__file__, sys.argv[1:], "/tmp/codex54-picker-input", env="set -o pipefail &&"))
    for browser in ("chromium", "firefox"):
        check(browser, args.testn)
