#!/usr/bin/env python3
"""Check the Sheets date picker with real input in desktop Chromium and Firefox.

Run against an isolated Starbase test server with --testn <number>.
"""
import argparse
import calendar
import subprocess
import sys
import time
from datetime import date

from pathlib import Path

sys.path.insert(0, str(Path.home() / ".claude/tools"))
from starbase_ui import Session, ship_and_run


def check(browser, testn):
    with Session(browser, testn=testn, outdir="shots") as s:
        s.sign_in("ada")
        s.open("sheets")
        s.click_xy(400, 330)
        validated = s.snapshot("active")
        s.hook("cell.set", cell=validated, input="2026-12-15")
        s.hook("data.validation", range=validated, type="date", operator="between", formula1="1", formula2="2958465")
        s.key("F2")

        def month(year, value):
            expected = {"year": year, "month": value}
            assert s.wait_for(lambda: s.snapshot("datePicker") == expected, secs=7), (expected, s.snapshot("datePicker"))

        month(2026, 12)
        s.click_text("date-picker:next")
        month(2027, 1)
        s.click_text("date-picker:previous")
        month(2026, 12)
        s.click_text("date-picker:month")
        s.click_text("date-picker:month-6")
        month(2026, 6)
        s.click_text("date-picker:year")
        s.click_text("date-picker:year-2027")
        month(2027, 6)
        s.click_text(f"date-picker:day-{calendar.monthrange(2027, 6)[0]}")
        assert s.snapshot("datePicker") is None
        assert s.snapshot("cell.number") == (date(2027, 6, 1) - date(1899, 12, 30)).days, s.snapshot("cell")

        s.click_xy(400, 355)
        formatted = s.snapshot("active")
        assert formatted != validated
        s.hook("cell.set", cell=formatted, input="2026-12-22")
        s.hook("home.numberFormat", code="@date")
        before = s.snapshot("cell")
        s._at(400, 355)
        subprocess.run(["xdotool", "click", "--repeat", "2", "--delay", "90", "1"], check=True)
        time.sleep(0.3)
        month(2026, 12)
        s.key("Escape")
        assert s.snapshot("datePicker") is None
        assert s.snapshot("cell") == before
        print(f"PASS {browser}: both rollover directions, independent menus, day selection, date-format double-click and cancel", flush=True)


p = argparse.ArgumentParser()
p.add_argument("--testn", type=int, required=True)
p.add_argument("--remote", action="store_true")
a = p.parse_args()
if not a.remote:
    sys.exit(ship_and_run(__file__, sys.argv[1:], "/tmp/codex-date-picker"))
for browser in ("chromium", "firefox"):
    check(browser, a.testn)
