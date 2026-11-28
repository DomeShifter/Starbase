#!/usr/bin/env python3
"""Verify that the Photos switch keeps its position across workspaces."""

import argparse
from pathlib import Path
import sys

sys.path.insert(0, str(Path.home() / ".claude/tools"))
from starbase_ui import Session, ship_and_run


def check(browser, testn):
    with Session(browser, testn=testn) as s:
        s.sign_in()
        s.open("drive")
        targets = lambda: s.snapshot("targets")
        assert "drive:photo_mode" not in targets(), "Photos offered in a folder without images"
        s.click_text("Photos")
        assert s.wait_text("drive:photo_mode")
        before = targets()
        point = before["drive:photo_mode"]
        s.pg.mouse.click(*point)
        assert s.wait_text("photo-exit")
        after = targets()
        for a, b in (
            (point, after["photo-exit"]),
            (before["account:avatar"], after["account:avatar"]),
            (before["workspace:search"], after["workspace:search"]),
        ):
            assert max(abs(x-y) for x, y in zip(a, b)) < 2, (a, b)
        s.pg.mouse.click(*point)
        assert s.wait_text("drive:photo_mode"), "The same pointer position did not return to Drive"
        assert "photo-exit" not in targets()
        print(f"PASS {browser}: photo-only switch, aligned search and account, same-pointer return to Drive", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--testn", type=int, required=True)
    parser.add_argument("--remote", action="store_true")
    args = parser.parse_args()
    if not args.remote:
        sys.exit(ship_and_run(__file__, sys.argv[1:], "/tmp/starbase-drive-photos-layout-input", env="set -o pipefail &&"))
    for browser in ("chromium", "firefox"):
        check(browser, args.testn)
