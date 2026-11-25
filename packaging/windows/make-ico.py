#!/usr/bin/env python3
"""Packs PNG icons into a Windows .ico (PNG-compressed entries, Windows Vista and later).

    make-ico.py OUT.ico IN-16.png IN-24.png ... IN-256.png

The PNGs come from packaging/linux/icons.sh (rendered from the apps' own SVGs), so the
artwork stays in one place. Sizes are read from each PNG's header; anything over 256 px is
skipped (an .ico entry is at most 256 px).
"""

import struct
import sys


def png_size(data: bytes) -> tuple[int, int]:
    if data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
        raise SystemExit("not a PNG")
    return struct.unpack(">II", data[16:24])


def main() -> None:
    out, *inputs = sys.argv[1:]
    if not inputs:
        raise SystemExit(__doc__)
    images = []
    for path in inputs:
        data = open(path, "rb").read()
        w, h = png_size(data)
        if w <= 256 and h <= 256:
            images.append((w, h, data))
    images.sort()
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, blobs = b"", b""
    for w, h, data in images:
        # Width and height 0 mean 256; colour count 0, planes 1, 32 bits per pixel.
        entries += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(data), offset + len(blobs))
        blobs += data
    with open(out, "wb") as f:
        f.write(header + entries + blobs)
    print(f"{out}: {', '.join(f'{w}x{h}' for w, h, _ in images)}")


if __name__ == "__main__":
    main()
