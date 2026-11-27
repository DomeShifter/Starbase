#!/usr/bin/env python3
"""Open an unindexed Drive image with the common Photos viewer."""

import argparse
import io
import sys
from pathlib import Path
from urllib.parse import quote

sys.path.insert(0, str(Path.home() / ".claude/tools"))
from starbase_ui import Session, ship_and_run


def run(browser, testn):
    from PIL import Image

    with Session(browser, testn=testn, outdir="shots") as s:
        s.sign_in("ada")
        base = s.base
        requests = s.pg.request
        folder = requests.post(f"{base}/api/folders?name=Viewer%20{browser}").json()["path"]
        buf = io.BytesIO()
        Image.new("RGB", (300, 180), "#3155c0").save(buf, "PNG")
        body = buf.getvalue()
        file_id = requests.post(
            f"{base}/api/files?folder={quote(folder)}&name=Landscape.png&as_is=1", data=body,
        ).json()["id"]
        assert not requests.get(f"{base}/api/photos?folder={quote(folder)}").json()["enabled"]
        urls = []
        s.pg.on("request", lambda request: urls.append(request.url))
        s.open("drive", path=f"/?test&dir={quote(folder)}")
        s.click_text("item:Landscape.png")
        s.key("Return")
        assert s.wait_for(lambda: s.snapshot("home.photos.viewer.showing") == "1024", secs=20)
        assert s.snapshot("home.photos.viewer.id") == file_id
        s.click_text("viewer:info")
        assert s.wait_for(lambda: s.snapshot("home.photos.viewer.info_detail.writable"), secs=10)
        assert not any(f"/api/photos/faces/{file_id}" in url for url in urls)
        s.click_text("viewer:rotate_right")
        assert s.wait_for(lambda: requests.get(f"{base}/api/photos/edits/{file_id}").json()["edits"].get("rot90"), secs=10)
        rotated = requests.get(f"{base}/api/files/{file_id}/picture")
        assert rotated.ok and Image.open(io.BytesIO(rotated.body())).size == (180, 300)
        assert requests.get(f"{base}/api/files/{file_id}").body() == body
        s.shot("drive-photo-viewer-" + browser)
        s.key("Escape")
        s.key("Escape")
        assert s.wait_for(lambda: s.snapshot("home.photos.viewer") is None, secs=10)
        print(f"PASS {browser}: unindexed Drive image opened shared Info and rotated without changing original bytes", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--testn", type=int, required=True)
    parser.add_argument("--remote", action="store_true")
    args = parser.parse_args()
    if not args.remote:
        sys.exit(ship_and_run(__file__, sys.argv[1:], "/tmp/codex-drive-photo-viewer"))
    for browser in ("chromium", "firefox"):
        run(browser, args.testn)
