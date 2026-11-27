#!/usr/bin/env python3
"""Seed and check posters on a disposable Docker test server with FFmpeg."""

import argparse
import io
import subprocess
import tempfile
import time
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request

from PIL import Image

from seed import Client, q


VIDEOS = (
    ("post-a.mp4", "red", "libx264"),
    ("post-b.mov", "lime", "libx264"),
    ("post-c.webm", "blue", "libvpx-vp9"),
    ("post-d.ogv", "yellow", "libtheora"),
)


def clip(name, colour, codec, tmp):
    path = Path(tmp) / name
    subprocess.run(
        ["ffmpeg", "-y", "-nostdin", "-v", "error", "-f", "lavfi", "-i",
         f"color={colour}:s=320x180:d=1", "-c:v", codec, "-threads", "1", path],
        check=True, timeout=10,
    )
    return path.read_bytes()


def get(client, path, status=200, etag=None):
    headers = {"If-None-Match": etag} if etag else {}
    try:
        with client.op.open(Request(client.base + path, headers=headers), timeout=15) as response:
            assert response.status == status, (path, status, response.status)
            return response.read(), response.headers
    except HTTPError as error:
        assert error.code == status, (path, status, error.code, error.read()[:200])


def poster(client, path, colour):
    body, headers = get(client, path)
    assert headers["Content-Type"] == "image/png", (path, headers)
    assert headers["Cache-Control"].startswith("private"), (path, headers)
    with Image.open(io.BytesIO(body)) as image:
        assert image.size == (256, 144), (path, image.size)
        rgb = image.convert("RGB").getpixel((128, 72))
    expected = {"red": (255, 0, 0), "lime": (0, 255, 0), "blue": (0, 0, 255), "yellow": (255, 255, 0)}
    assert max(abs(a - b) for a, b in zip(rgb, expected[colour])) < 15, (path, colour, rgb)
    return body, headers["ETag"]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--url", required=True, help="Disposable TESTN server")
    args = parser.parse_args()
    ada, bob = Client(args.url), Client(args.url)
    for user, password, client in (("ada", "adapassword", ada), ("bob", "bobspassword", bob)):
        client.call("POST", "api/login", {"username": user, "password": password})
    folder = ada.call("POST", "api/folders?" + q(name="Video Posters"))["path"]
    with tempfile.TemporaryDirectory() as tmp:
        files = {
            name: (clip(name, colour, codec, tmp), colour) for name, colour, codec in VIDEOS
        }
        files["post-z.mp4"] = (b"broken media", None)
        ids = {
            name: ada.call("POST", "api/files?" + q(folder=folder, name=name, as_is=1), data, "application/octet-stream")["id"]
            for name, (data, _) in files.items()
        }
        for file_id in ids.values():
            get(ada, f"api/photos/thumb/{file_id}/256", 404)
        ada.call("POST", "api/photos/enable?" + q(folder=folder))
        for attempt in range(30):
            listing = ada.call("GET", "api/photos?" + q(folder=folder))
            if {row["name"] for row in listing["photos"]} == set(files):
                assert all(row["state"] == "unsupported" for row in listing["photos"]), listing
                break
            time.sleep(1)
        else:
            raise AssertionError(f"Video fixtures did not appear in Photos: {listing}")

        for name, (data, colour) in files.items():
            file_id = ids[name]
            drive, photos = f"api/files/{file_id}/thumb", f"api/photos/thumb/{file_id}/256"
            if colour:
                drive_bytes, drive_etag = poster(ada, drive, colour)
                photo_bytes, photo_etag = poster(ada, photos, colour)
                assert drive_bytes == photo_bytes
                get(ada, drive, 304, drive_etag)
                get(ada, photos, 304, photo_etag)
                get(ada, f"api/photos/thumb/{file_id}/1024", 404)
                assert get(ada, f"api/files/{file_id}/raw")[0] == data
            else:
                get(ada, drive, 404)
                get(ada, photos, 404)
            get(bob, drive, 403)
            get(bob, photos, 403)

        file_id = ids["post-a.mp4"]
        drive, photos = f"api/files/{file_id}/thumb", f"api/photos/thumb/{file_id}/256"
        _, before = poster(ada, drive, "red")
        updated = clip("replacement.mp4", "blue", "libx264", tmp)
        ada.call("PUT", f"api/files/{file_id}", updated, "application/octet-stream")
        _, after = poster(ada, photos, "blue")
        assert poster(ada, drive, "blue")[1] != before and after
        ada.call("PUT", f"api/files/{file_id}", files["post-a.mp4"][0], "application/octet-stream")
        poster(ada, drive, "red")
    print("PASS four formats, shared cache, conditional requests, replacement, originals, corrupt fallback and access checks.")


if __name__ == "__main__":
    main()
