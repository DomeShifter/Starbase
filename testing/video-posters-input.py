#!/usr/bin/env python3
"""Check rendered video thumbnails in Drive and Photos in Chromium and Firefox.

On the disposable test server, seed ada/Video Posters with solid-colour clips:
post-a.mp4 (red), post-b.mov (green), post-c.webm (blue), and post-d.ogv
(yellow). Add a corrupt post-z.mp4 and enable Photos for that folder.
Run: python3 testing/video-posters-input.py --testn 755
"""

import argparse
import base64
import sys
from pathlib import Path
from urllib.parse import quote, urlsplit

sys.path.insert(0, str(Path.home() / ".claude/tools"))
from starbase_ui import Session, ship_and_run

FOLDER = "ada/Video Posters"
COLOURS = {"post-a.mp4": "red", "post-b.mov": "green", "post-c.webm": "blue", "post-d.ogv": "yellow"}
BROKEN = "post-z.mp4"
NAMES = {*COLOURS, BROKEN}
BROWSERS = ("chromium", "firefox")


def pixels(page, points):
    """Sample a small square inside each card in the actual browser screenshot."""
    png = base64.b64encode(page.screenshot(scale="css")).decode()
    return page.evaluate(
        """async ({png, points}) => {
            const bytes = Uint8Array.from(atob(png), c => c.charCodeAt(0));
            const image = await createImageBitmap(new Blob([bytes], {type: 'image/png'}));
            const canvas = document.createElement('canvas');
            canvas.width = image.width;
            canvas.height = image.height;
            const ctx = canvas.getContext('2d');
            ctx.drawImage(image, 0, 0);
            return Object.fromEntries(Object.entries(points).map(([name, [x, y]]) => {
                const data = ctx.getImageData(Math.round(x) - 8, Math.round(y) - 8, 16, 16).data;
                const totals = [0, 0, 0];
                for (let i = 0; i < data.length; i += 4) {
                    for (let c = 0; c < 3; c++) totals[c] += data[i + c];
                }
                return [name, totals.map(total => Math.round(total / 256))];
            }));
        }""",
        {"png": png, "points": points},
    )


def colour(rgb):
    red, green, blue = rgb
    if min(red, green) > 100 and min(red, green) > blue + 60 and abs(red - green) < 65:
        return "yellow"
    for i, name in enumerate(("red", "green", "blue")):
        if rgb[i] > 95 and all(rgb[i] > other + 60 for j, other in enumerate(rgb) if j != i):
            return name
    return "neutral"


def verify_pixels(s, browser, view, points):
    samples = pixels(s.pg, points)
    expected = {**COLOURS, BROKEN: "neutral"}
    actual = {name: colour(rgb) for name, rgb in samples.items()}
    assert actual == expected, (browser, view, actual, samples)
    print(f"PASS {browser} {view}: {actual}", flush=True)


def run(browser, testn):
    with Session(browser, testn=testn) as s:
        s.sign_in("ada")
        responses = {}
        s.pg.on("response", lambda r: responses.update({urlsplit(r.url).path: r.status}))
        s.open("drive", path=f"/?test&dir={quote(FOLDER, safe='')}")
        s.click_text("Grid")

        def drive_ready():
            cards = {f["name"]: f for f in s.snapshot("home.files") or [] if f["name"] in NAMES}
            if cards.keys() == NAMES and all(
                card.get("rect") and card.get("thumb") == ("none" if name == BROKEN else "ready")
                for name, card in cards.items()
            ):
                return cards

        cards = s.wait_for(drive_ready, secs=60)
        assert cards, (browser, "Drive cards did not reach ready/none", s.snapshot("home.files"))
        assert responses.get(f"/api/files/{cards[BROKEN]['id']}/thumb") == 404, "Drive did not try the corrupt fixture"
        picture_height = s.snapshot("home.card.pic")
        points = {
            name: ((card["rect"][0] + card["rect"][2]) / 2, card["rect"][1] + picture_height / 2)
            for name, card in cards.items()
        }
        verify_pixels(s, browser, "Drive", points)

        s.click_text("drive:photo_mode")

        def photos_ready():
            state = s.snapshot()
            home = state.get("home") or {}
            items = (home.get("photos") or {}).get("items") or []
            targets = state.get("targets") or {}
            points = {}
            for i, item in enumerate(items):
                name = item["name"]
                if name not in NAMES or item.get("state") != "unsupported":
                    continue
                if name == BROKEN and responses.get(f"/api/photos/thumb/{item['id']}/256") != 404:
                    continue
                if item["thumb"] == (name != BROKEN) and (at := targets.get(f"photos:tile:{i}")):
                    # The play badge covers the centre of every video tile.
                    points[name] = (at[0], at[1] - 42)
            return points if points.keys() == NAMES else None

        points = s.wait_for(photos_ready, secs=60)
        assert points, (browser, "Photos tiles did not reach ready/fallback", s.snapshot("home.photos.items"))
        verify_pixels(s, browser, "Photos", points)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--testn", type=int, required=True)
    parser.add_argument("--browser", choices=BROWSERS)
    parser.add_argument("--remote", action="store_true")
    args = parser.parse_args()
    if not args.remote:
        sys.exit(ship_and_run(__file__, sys.argv[1:], "/tmp/codex-video-posters-input", env="set -o pipefail;"))
    for browser in (args.browser,) if args.browser else BROWSERS:
        run(browser, args.testn)
