#!/usr/bin/env python3
# Last updated: 2026-11-26T22:23:16Z
"""Real-input check of the photo viewer's dark bar and its Info panel (not yet run).
Seed first: python3 -I testing/seed.py --url <URL> --set gps   (pictures with camera EXIF + GPS).
Run:  photos_viewer_info.py http://<host>:<port> --browser chromium|firefox [--user NAME]
Expects: the viewer opens with a 56 px dark bar (target viewer:bar) whose icon buttons are 36 x 36
(viewer:close, viewer:info, viewer:download, ...); Info (viewer:info button) opens a 320 px panel
(viewer:info-panel) with the headings WHAT / WHEN AND WHERE / CAMERA / IN (empty ones left out), label left
and value right, a close button (viewer:info:close) and 'Move to Locked folder' as a dark pill
button (viewer:info:lock). The panel's close button shuts it; the I key opens it again.
Screenshots: <browser>-viewer-0-bar, -1-info, -2-info-closed (look at them: spacing, alignment)."""
import os, sys, time
sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
import starbase_ui as ui


def main():
    a = sys.argv[1:]; base = a[0]; br = a[a.index("--browser") + 1] if "--browser" in a else "chromium"
    res = []
    def rec(n, ok, d=""): res.append(ok); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base); S.sign_in(a[a.index("--user") + 1] if "--user" in a else "ada")
    S.open("drive"); S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:enable"); time.sleep(4)
    tg = lambda: S.snapshot("targets") or {}
    vw = lambda: S.snapshot("home.photos.viewer") or {}
    S.hook("home:photos:open", index=0); time.sleep(3)
    rec("viewer open with its bar", "viewer:bar" in tg(), str(sorted(k for k in tg() if k.startswith("viewer:"))))
    S.shot(f"{br}-viewer-0-bar")
    S.click_text("viewer:info"); time.sleep(1.5)
    rec("Info panel open", vw().get("info") is True and "viewer:info:lock" in tg())
    rows = dict(vw().get("info_rows") or [])
    rec("camera + place rows", "Camera" in rows and "Place" in rows, str(rows))
    S.shot(f"{br}-viewer-1-info")
    S.click_text("viewer:info:close"); time.sleep(1)
    rec("close button shuts the panel", vw().get("info") is False)
    S.shot(f"{br}-viewer-2-info-closed")
    S.key("i"); time.sleep(1)
    rec("I opens it again", vw().get("info") is True)
    S.close(); sys.exit(0 if all(res) else 1)


if __name__ == "__main__":
    main()
