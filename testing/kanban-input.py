#!/usr/bin/env python3
"""Verify the Kanban board with real browser input on a disposable CT105 server."""

import argparse
from pathlib import Path
import sys
from uuid import uuid4

sys.path.insert(0, str(Path.home() / ".claude/tools"))
from starbase_ui import Session, ship_and_run


def check(browser, testn):
    with Session(browser, testn=testn) as s:
        s.sign_in()
        s.open("drive")
        name = f"Board {uuid4().hex[:6]}"
        s.click_text("New")
        s.click_text("New Kanban board")
        s.click_text("kanban:board-name")
        s.pg.keyboard.type(name)
        s.click_text("kanban:create")
        s.click_text("kanban:add:0")
        s.click_text("kanban:card-title")
        s.pg.keyboard.type("Review launch")
        s.click_text("kanban:apply-card")
        assert s.wait_text("kanban:card:0:0")
        start = s.snapshot("targets")["kanban:card:0:0"]
        end = s.snapshot("targets")["kanban:drop:1"]
        s.pg.mouse.move(*start)
        s.pg.mouse.down()
        s.pg.mouse.move(*end, steps=18)
        s.pg.mouse.up()
        assert s.wait_text("kanban:card:1:0"), "Card did not move into In progress"
        s.click_text("kanban:save")
        api = s.base + "/api/files"
        row = next(row for row in s.pg.request.get(api).json()["files"] if row["name"] == f"{name}.kanban")
        assert s.wait_for(lambda: bool(s.pg.request.get(f"{api}/{row['id']}").json()["columns"][1]["cards"]))
        assert s.pg.request.get(f"{api}/{row['id']}").json()["columns"][1]["cards"][0]["title"] == "Review launch"
        s.click_text("kanban:back")
        assert s.wait_text(f"item:{name}.kanban")
        s.click_text(f"item:{name}.kanban")
        s.key("Return")
        assert s.wait_text("kanban:card:1:0")
        print(f"PASS {browser}: created board in Drive, added and dragged a card, saved and reopened", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--testn", type=int, required=True)
    parser.add_argument("--browser", choices=("chromium", "firefox"))
    parser.add_argument("--remote", action="store_true")
    args = parser.parse_args()
    if not args.remote:
        sys.exit(ship_and_run(__file__, sys.argv[1:], "/tmp/starbase-kanban-input", env="set -o pipefail &&"))
    for browser in (args.browser,) if args.browser else ("chromium", "firefox"):
        check(browser, args.testn)
