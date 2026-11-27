#!/usr/bin/env python3
"""Check the shared browser appearance preference in Drive and the editors."""

import argparse
import base64
import sys
from pathlib import Path

sys.path.insert(0, str(Path.home() / ".claude/tools"))
from starbase_ui import Session, ship_and_run


def luminance(page, clip):
    png = page.screenshot(clip=clip)
    return page.evaluate(
        """async png => {
            const bytes = Uint8Array.from(atob(png), c => c.charCodeAt(0));
            const image = await createImageBitmap(new Blob([bytes], {type: 'image/png'}));
            const canvas = document.createElement('canvas');
            canvas.width = image.width;
            canvas.height = image.height;
            const ctx = canvas.getContext('2d');
            ctx.drawImage(image, 0, 0);
            const data = ctx.getImageData(0, 0, image.width, image.height).data;
            let sum = 0;
            for (let i = 0; i < data.length; i += 4) {
                sum += 0.2126 * data[i] + 0.7152 * data[i + 1] + 0.0722 * data[i + 2];
            }
            return sum / (data.length / 4);
        }""",
        base64.b64encode(png).decode(),
    )


def run(browser, testn):
    header = {"x": 244, "y": 3, "width": 175, "height": 20}
    paper = {"x": 610, "y": 335, "width": 100, "height": 80}
    with Session(browser, testn=testn, outdir="shots") as s:
        s.sign_in("ada")
        for dark in (True, False):
            s.open("drive")
            s.click_text("account:avatar", button=3)
            s.click_text("settings:dark")
            expected = "1" if dark else "0"
            assert s.pg.evaluate("localStorage.getItem('starbase_drive_dark')") == expected
            s.key("Escape")
            for app in ("drive", "sheets", "docs", "slides"):
                if app != "drive":
                    s.open(app)
                colour = luminance(s.pg, header)
                assert (colour < 105 if dark else colour > 185), (browser, app, dark, colour)
                if app in ("sheets", "docs", "slides"):
                    canvas = luminance(s.pg, paper)
                    assert canvas > 185, (browser, app, dark, canvas)
                s.shot(f"{browser}-{app}-{'dark' if dark else 'light'}")
                print(f"PASS {browser} {app} {'dark' if dark else 'light'} header={colour:.0f}", flush=True)
            s.open("drive")
            s.click_text("drive:photo_mode")
            assert s.wait_for(lambda: s.snapshot("home.photos.shown"), secs=10)
            colour = luminance(s.pg, header)
            assert colour < 105, (browser, "photos", dark, colour)
            s.shot(f"{browser}-photos-after-{'dark' if dark else 'light'}")
            s.hook("home:photos", on=False)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--testn", type=int, required=True)
    parser.add_argument("--remote", action="store_true")
    args = parser.parse_args()
    if not args.remote:
        sys.exit(ship_and_run(__file__, sys.argv[1:], "/tmp/codex-editor-appearance",
                              shots_to=str(Path.home() / ".cache/starbase-shots/codex-editor-appearance")))
    for browser in ("chromium", "firefox"):
        run(browser, args.testn)
