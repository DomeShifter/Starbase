#!/usr/bin/env python3
"""Check right-click Settings and its tabs in desktop browsers on an isolated test instance."""

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path.home() / ".claude/tools"))
from starbase_ui import Session, ship_and_run


def check(browser, testn):
    with Session(browser, testn=testn, outdir="shots") as s:
        s.sign_in("ada")
        s.open("drive")

        def target(name):
            return (s.snapshot("targets") or {}).get(name)

        def settings():
            s.click_text("account:avatar", button=3)
            assert s.wait_for(lambda: target("settings:dark"), secs=6), "Right-click did not open Settings"

        def check_setting(key, value):
            assert s.pg.evaluate("(key) => localStorage.getItem(key)", key) == value

        def backup():
            response = s.pg.request.get(s.base + "/api/backup")
            assert response.ok
            return response.json()

        settings()
        s.click_text("settings:dark")
        check_setting("starbase_drive_dark", "1")
        s.shot(f"{browser}-dark-settings")
        s.open("drive")
        check_setting("starbase_drive_dark", "1")
        settings()
        s.click_text("settings:dark")
        check_setting("starbase_drive_dark", "0")
        assert s.wait_for(lambda: target("drive:photo_mode"), secs=6), "Photo Mode is not offered by default"

        s.click_text("tab:Photos")
        s.click_text("settings:photos-enabled")
        check_setting("starbase_photos_available", "0")
        assert s.wait_for(lambda: not target("drive:photo_mode"), secs=6), "The Photo Mode button is still visible"
        s.open("drive")
        assert not target("drive:photo_mode"), "Photo Mode returned after reload"
        settings()
        s.click_text("tab:Photos")
        s.click_text("settings:photos-enabled")
        check_setting("starbase_photos_available", "1")
        assert s.wait_for(lambda: target("drive:photo_mode"), secs=6), "Re-enabling did not restore Photo Mode"

        original_faces = s.pg.request.get(s.base + "/api/photos/faces").json()
        if original_faces.get("can_toggle"):
            original_on = original_faces["on"]
            s.click_text("settings:faces")
            assert s.wait_for(
                lambda: s.pg.request.get(s.base + "/api/photos/faces").json()["on"] != original_on,
                secs=8,
            )
            s.click_text("settings:faces")
            assert s.wait_for(
                lambda: s.pg.request.get(s.base + "/api/photos/faces").json()["on"] == original_on,
                secs=8,
            )

        s.click_text("tab:Backups")
        before = backup()
        assert before["enabled"] is True
        s.click_text("settings:backups-enabled")
        assert s.wait_for(lambda: backup()["enabled"] is False, secs=8), "Master Off was not saved"
        disabled_run = s.pg.request.post(s.base + "/api/backup/run")
        assert disabled_run.status == 400 and "turned off" in disabled_run.text()
        assert all(before[k] == backup()[k] for k in ("on", "hour", "to", "set_up"))
        s.click_text("settings:backups-enabled")
        assert s.wait_for(lambda: backup()["enabled"] is True, secs=8), "Master On was not saved"

        s.click_text("tab:Server")
        assert s.wait_for(lambda: target("settings:signup") and target("settings:link-preview"), secs=8)
        url = s.base + "/api/link-preview/settings"
        original_preview = s.pg.request.get(url).json()["on"]
        s.click_text("settings:link-preview")
        assert s.wait_for(lambda: s.pg.request.get(url).json()["on"] != original_preview, secs=8)
        s.click_text("settings:link-preview")
        assert s.wait_for(lambda: s.pg.request.get(url).json()["on"] == original_preview, secs=8)

        s.click_text("tab:Storage")
        assert s.wait_for(lambda: target("settings:storage"), secs=8)
        s.key("Escape")
        s.hook("account:storage")
        assert s.wait_for(lambda: target("settings:storage"), secs=8)

    with Session(browser, testn=testn, outdir="shots") as member:
        member.sign_in("bob")
        member.open("drive")
        member.click_text("account:avatar", button=3)
        assert member.wait_for(lambda: (member.snapshot("targets") or {}).get("settings:dark"), secs=6)
        assert all("tab:" + name not in member.snapshot("targets") for name in ("Backups", "Server", "Storage"))
    print(f"PASS {browser}: settings, persistent appearance and Photos visibility, owner backup switch", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--testn", type=int, required=True)
    parser.add_argument("--remote", action="store_true")
    args = parser.parse_args()
    if not args.remote:
        sys.exit(ship_and_run(__file__, sys.argv[1:], "/tmp/codex-web-settings",
                              shots_to=str(Path.home() / ".cache/starbase-shots/codex-web-settings")))
    for browser in ("chromium", "firefox"):
        check(browser, args.testn)
