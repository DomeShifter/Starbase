#!/usr/bin/env python3
"""Verify Kanban autosave, card controls, drag and the archive on CT105."""

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
        name = f"Workflow {uuid4().hex[:6]}"
        s.click_text("New")
        s.click_text("New Kanban board")
        s.click_text("kanban:board-name")
        s.pg.keyboard.type(name)
        s.click_text("kanban:create")
        assert s.wait_text("kanban:save:saved")
        root = s.base + "/api/files"
        row = next(row for row in s.pg.request.get(root).json()["files"] if row["name"] == name + ".kanban")
        api = root + "/" + row["id"]

        def board():
            return s.pg.request.get(api).json()

        def card(title):
            s.click_text("kanban:add:0")
            s.click_text("kanban:card-title")
            s.pg.keyboard.type(title)
            s.click_text("kanban:apply-card")
            assert s.wait_text("kanban:save:saved")

        s.click_text("kanban:numbering")
        assert s.wait_for(lambda: board()["numbering"])
        card("First")
        card("Second")
        assert s.wait_for(lambda: len(board()["columns"][0]["cards"]) == 2)
        assert [c["number"] for c in board()["columns"][0]["cards"]] == [1, 2]
        positions = s.snapshot("targets")
        first, second = positions["kanban:card:0:0"], positions["kanban:card:0:1"]
        assert abs(second[1] - first[1]) < 10 and second[0] > first[0], (first, second)
        assert positions["kanban:expand:0:0"][0] > first[0]

        s.pg.mouse.dblclick(*first)
        assert s.wait_text("kanban:inline-title")
        assert "kanban:apply-card" not in s.snapshot("targets")
        s.pg.keyboard.press("Control+a")
        s.pg.keyboard.type("Renamed")
        s.pg.keyboard.press("Enter")
        assert s.wait_for(lambda: board()["columns"][0]["cards"][0]["title"] == "Renamed")
        assert s.wait_text("kanban:save:saved")
        positions = s.snapshot("targets")
        s.pg.mouse.move(*positions["kanban:card:0:0"])
        s.pg.mouse.down()
        s.pg.mouse.move(*positions["kanban:drop:1"], steps=18)
        s.pg.mouse.up()
        assert s.wait_for(lambda: board()["columns"][1]["cards"][0]["title"] == "Renamed")
        assert s.wait_text("kanban:save:saved")

        s.click_text("kanban:expand:1:0")
        s.click_text("kanban:archive-card")
        assert s.wait_for(lambda: board()["archive"][0]["number"] == 1)
        s.click_text("kanban:archive")
        s.click_text("kanban:delete:0")
        s.click_text("kanban:confirm-delete")
        assert s.wait_for(lambda: board()["archive"] == [])
        s.click_text("kanban:archive")
        card("Third")
        assert s.wait_for(lambda: [c["number"] for c in board()["columns"][0]["cards"]] == [2, 3])
        assert s.pg.request.get(api + "/versions").json() == []
        s.click_text("kanban:back")
        s.click_text(f"item:{name}.kanban")
        s.key("Return")
        assert s.wait_text("kanban:card:0:1")
        print(f"PASS {browser}: auto save, layout, rename, whole-column drag, archive, numbering and reopen", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--testn", type=int, required=True)
    parser.add_argument("--browser", choices=("chromium", "firefox"))
    parser.add_argument("--remote", action="store_true")
    args = parser.parse_args()
    if not args.remote:
        sys.exit(ship_and_run(__file__, sys.argv[1:], "/tmp/kanban-workflow-input", env="set -o pipefail &&"))
    for browser in (args.browser,) if args.browser else ("chromium", "firefox"):
        check(browser, args.testn)
