"""Deterministic test data for Starbase testers (stdlib + Pillow only; run with `python3 -I`).

    from fixtures import photos, docs
    photos(dir, set="gps")     # -> list of file names written
    docs(dir)                  # -> ["two-page.docx"]

Every set is idempotent: same names, same bytes; a rerun overwrites with identical files.

Photo sets (JPEG unless stated; EXIF is hand-built TIFF bytes, no piexif):
  dates   24 photos spread over 18 months (2025-01-15 onwards), EXIF DateTimeOriginal.
          Names dates-01.jpg .. dates-24.jpg.
  gps     20 photos with real EXIF GPS in 6 countries (GB, FR, JP, US, BR, AU). The first
          three (gps-01..03) are within ~100 m in one city, so a stack/place group forms.
          All also carry DateTimeOriginal. Names gps-01.jpg .. gps-20.jpg.
  shapes  shape-portrait.jpg (300x400), shape-landscape.jpg (400x300), shape-pano.jpg
          (900x300, 3:1), shape-rot6.jpg (stored 400x300 with EXIF orientation 6, so it
          displays 300x400).
  types   type.png, type.gif, type.webp, and type.mp4 only when ffmpeg is on PATH.

After upload, Photos indexes in the background: a picture is usable only when its `state`
is "ready" (seed.py waits for that). "pending" just means not indexed yet; it is not a bug.
"""
import datetime
import os
import random
import shutil
import struct
import subprocess
import zipfile

from PIL import Image, ImageDraw

SETS = ("dates", "gps", "shapes", "types")

# (name, lat, lon): the first three are one London street.
PLACES = [
    ("london-a", 51.5007, -0.1246), ("london-b", 51.5010, -0.1240), ("london-c", 51.5005, -0.1250),
    ("york", 53.9599, -1.0873), ("paris", 48.8584, 2.2945), ("lyon", 45.7640, 4.8357),
    ("nice", 43.7102, 7.2620), ("tokyo", 35.6586, 139.7454), ("kyoto", 35.0116, 135.7681),
    ("osaka", 34.6937, 135.5023), ("nyc", 40.6892, -74.0445), ("sf", 37.8199, -122.4783),
    ("chicago", 41.8827, -87.6233), ("rio", -22.9519, -43.2105), ("sao-paulo", -23.5505, -46.6333),
    ("salvador", -12.9777, -38.5016), ("sydney", -33.8568, 151.2153), ("melbourne", -37.8136, 144.9631),
    ("brisbane", -27.4698, 153.0251), ("perth", -31.9523, 115.8613),
]


def _ifd(entries, start):
    """Little-endian IFD at TIFF offset `start`; values over 4 bytes go after the table."""
    entries = sorted(entries)
    table = 2 + 12 * len(entries) + 4
    out, tail = struct.pack("<H", len(entries)), b""
    for tag, typ, count, data in entries:
        if len(data) <= 4:
            out += struct.pack("<HHI", tag, typ, count) + data.ljust(4, b"\0")
        else:
            out += struct.pack("<HHII", tag, typ, count, start + table + len(tail))
            tail += data + (b"\0" if len(data) % 2 else b"")
    return out + b"\0\0\0\0" + tail


def _a(tag, s):
    b = s.encode() + b"\0"
    return (tag, 2, len(b), b)


def _rat3(tag, deg):
    d = abs(deg)
    whole = int(d)
    m = int((d - whole) * 60)
    s = round(((d - whole) * 60 - m) * 60 * 10000)
    return (tag, 5, 3, struct.pack("<6I", whole, 1, m, 1, s, 10000))


def exif_bytes(taken=None, gps=None, orientation=None):
    """APP1 payload (`Exif\\0\\0` + TIFF). `taken` datetime, `gps` (lat, lon), `orientation` 1-8."""
    ifd0 = []
    if orientation:
        ifd0.append((0x112, 3, 1, struct.pack("<H", orientation)))
    exif_ifd = [_a(0x9003, taken.strftime("%Y:%m:%d %H:%M:%S"))] if taken else None
    gps_ifd = None
    if gps:
        lat, lon = gps
        gps_ifd = [_a(1, "N" if lat >= 0 else "S"), _rat3(2, lat), _a(3, "E" if lon >= 0 else "W"), _rat3(4, lon)]
    ptr = (0, 4, 1, b"\0\0\0\0")
    pointers = ([(0x8769,) + ptr[1:]] if exif_ifd else []) + ([(0x8825,) + ptr[1:]] if gps_ifd else [])
    pos = 8 + len(_ifd(ifd0 + pointers, 8))  # pointer entries have a fixed size
    blobs = b""
    if exif_ifd:
        ifd0.append((0x8769, 4, 1, struct.pack("<I", pos)))
        b = _ifd(exif_ifd, pos)
        blobs, pos = blobs + b, pos + len(b)
    if gps_ifd:
        ifd0.append((0x8825, 4, 1, struct.pack("<I", pos)))
        blobs += _ifd(gps_ifd, pos)
    return b"Exif\0\0" + b"II*\0" + struct.pack("<I", 8) + _ifd(ifd0, 8) + blobs


def _image(w, h, seed, label=""):
    rnd = random.Random(seed)
    img = Image.new("RGB", (w, h), tuple(rnd.randrange(40, 220) for _ in range(3)))
    d = ImageDraw.Draw(img)
    for _ in range(6):
        x, y = rnd.randrange(w), rnd.randrange(h)
        d.ellipse((x, y, x + w // 4, y + h // 4), fill=tuple(rnd.randrange(256) for _ in range(3)))
    d.text((6, 6), label, fill=(255, 255, 255))
    return img


def _save(img, path, **kw):
    img.save(path, "JPEG", quality=85, **kw)
    return os.path.basename(path)


def photos(dir, set="dates"):
    """Write one fixture set into `dir` (created); returns the file names."""
    if set not in SETS:
        raise ValueError(f"unknown set {set!r}; one of {SETS}")
    os.makedirs(dir, exist_ok=True)
    out = []
    if set == "dates":
        for i in range(24):
            when = datetime.datetime(2025, 1, 15, 10, 30) + datetime.timedelta(days=int(i * 18 * 30.4 / 24), minutes=i)
            out.append(_save(_image(320, 240, i, f"d{i + 1}"), f"{dir}/dates-{i + 1:02d}.jpg", exif=exif_bytes(taken=when)))
    elif set == "gps":
        for i, (name, lat, lon) in enumerate(PLACES):
            when = datetime.datetime(2025, 6, 1, 9, 0) + datetime.timedelta(days=i * 9)
            out.append(_save(_image(320, 240, 100 + i, name), f"{dir}/gps-{i + 1:02d}.jpg", exif=exif_bytes(taken=when, gps=(lat, lon))))
    elif set == "shapes":
        when = datetime.datetime(2025, 8, 1, 12, 0)
        out.append(_save(_image(300, 400, 1, "portrait"), f"{dir}/shape-portrait.jpg", exif=exif_bytes(taken=when)))
        out.append(_save(_image(400, 300, 2, "landscape"), f"{dir}/shape-landscape.jpg", exif=exif_bytes(taken=when)))
        out.append(_save(_image(900, 300, 3, "pano 3:1"), f"{dir}/shape-pano.jpg", exif=exif_bytes(taken=when)))
        out.append(_save(_image(400, 300, 4, "rot6"), f"{dir}/shape-rot6.jpg", exif=exif_bytes(taken=when, orientation=6)))
    else:
        img = _image(200, 150, 7, "type")
        img.save(f"{dir}/type.png")
        img.save(f"{dir}/type.gif")
        img.save(f"{dir}/type.webp")
        out += ["type.png", "type.gif", "type.webp"]
        if shutil.which("ffmpeg"):
            r = subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=160x120:rate=10:duration=1",
                                "-pix_fmt", "yuv420p", f"{dir}/type.mp4"])
            if r.returncode == 0:
                out.append("type.mp4")
    return out


def docs(dir):
    """A 2-page .docx (a page break between two paragraphs of text)."""
    os.makedirs(dir, exist_ok=True)
    ns = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
    body = ('<w:p><w:r><w:t>Page one: the quick brown fox.</w:t></w:r></w:p>'
            '<w:p><w:r><w:br w:type="page"/></w:r></w:p>'
            '<w:p><w:r><w:t>Page two: jumps over the lazy dog.</w:t></w:r></w:p>')
    parts = {
        "[Content_Types].xml": '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
        "_rels/.rels": '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>',
        "word/document.xml": f'<?xml version="1.0" encoding="UTF-8"?><w:document {ns}><w:body>{body}<w:sectPr/></w:body></w:document>',
    }
    with zipfile.ZipFile(f"{dir}/two-page.docx", "w", zipfile.ZIP_DEFLATED) as z:
        for name, text in parts.items():
            z.writestr(zipfile.ZipInfo(name, (2025, 1, 1, 0, 0, 0)), text)
    return ["two-page.docx"]
