#!/usr/bin/env python3
"""Drive file preview: real keys in Chromium and Firefox (run on CT105 under xvfb-run)."""
import io, json, struct, sys, time, zlib
from playwright.sync_api import sync_playwright

BASE = sys.argv[1]


def png(w, h):
    raw = b"".join(b"\x00" + bytes([200, 30, 30, 255]) * w for _ in range(h))
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def jpeg_rot6():
    try:
        from PIL import Image
    except ImportError:
        return None
    im = Image.new("RGB", (300, 100), (0, 120, 200))
    ex = Image.Exif(); ex[0x0112] = 6
    b = io.BytesIO(); im.save(b, "JPEG", exif=ex); return b.getvalue()


PDF = b"%PDF-1.1\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj 2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj 3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n"
FILES = {"pv_a.png": png(400, 200), "pv_b.txt": b"hello preview\nline two\n", "pv_c.pdf": PDF, "pv_d.zip": b"PK\x05\x06" + b"\0" * 18,
         "pv_f.md": b"# Title\n\nSome *text*.\n", "pv_g.csv": b"a,b\n1,2\n", "pv_h.json": b'{"a": [1, 2]}', "pv_i.rs": b"fn main() {}\n"}
TEXT = {"pv_b.txt": "plain", "pv_f.md": "markdown", "pv_g.csv": "table", "pv_h.json": "json", "pv_i.rs": "code"}
OPEN_WITH = (1110, 30)  # "Open with" in the preview bar, 1280 px wide window
j = jpeg_rot6()
if j:
    FILES["pv_e.jpg"] = j

results = []


def rec(browser, name, ok, detail=""):
    results.append(ok)
    print(f"{'PASS' if ok else 'FAIL'}  {browser:<9} {name:<34} {detail}", flush=True)


def run(kind):
    with sync_playwright() as pw:
        b = getattr(pw, kind).launch(headless=False)
        ctx = b.new_context(viewport={"width": 1280, "height": 800})
        pg = ctx.new_page()
        r = pg.request.post(BASE + "/api/login", data=json.dumps({"username": "ada", "password": "adapassword"}), headers={"content-type": "application/json"})
        assert r.status == 200, r.status
        for n, data in FILES.items():
            pg.request.post(BASE + f"/api/files?name={n}&as_is=1", data=data)
        pg.goto(BASE + "/?test")
        pg.wait_for_function("window.starsheets", timeout=90000)
        call = lambda e: json.loads(pg.evaluate(f"starsheets.wait({e})"))
        state = lambda: call("starsheets.state()")["ok"]["home"]
        run_ = lambda i, p=None: call(f"starsheets.run({json.dumps(i)},{json.dumps(p or {})})")
        time.sleep(3)
        files = {f["name"]: f["id"] for f in state()["files"]}
        ids = {n: files.get(n) for n in FILES}
        if None in ids.values():
            rec(kind, "files listed", False, str(sorted(files))[:120]); b.close(); return
        pv = lambda: state().get("preview")
        def wait_content(want, t=8):
            end = time.time() + t
            while time.time() < end:
                p = pv()
                if p and p["content"].startswith(want): return p
                time.sleep(0.3)
            return pv()
        pg.mouse.move(640, 400)
        run_("home:preview", {"id": ids["pv_a.png"]})
        p = wait_content("image")
        rec(kind, "png opens as image", bool(p) and p["content"] == "image 400x200", str(p and p["content"]))
        pg.mouse.move(640, 400)
        pg.keyboard.press("Equal"); time.sleep(0.4)
        rec(kind, "+ zooms", pv()["zoom"] > 1.4, str(pv()["zoom"]))
        pg.keyboard.press("Minus"); time.sleep(0.4)
        rec(kind, "- zooms out", abs(pv()["zoom"] - 1) < 0.01, str(pv()["zoom"]))
        pg.keyboard.down("Control"); pg.mouse.wheel(0, -300); pg.keyboard.up("Control"); time.sleep(0.6)
        rec(kind, "Ctrl+wheel zooms", pv()["zoom"] > 1.0, str(pv()["zoom"]))
        before = pv()["index"]
        pg.keyboard.press("ArrowRight"); time.sleep(0.6)
        p = pv()
        rec(kind, "ArrowRight steps", p and p["index"] == (before + 1) % p["count"], f"{before}->{p and p['index']} of {p and p['count']}")
        pg.keyboard.press("ArrowLeft"); time.sleep(0.6)
        rec(kind, "ArrowLeft steps back", pv()["index"] == before)
        pg.keyboard.press("Escape"); time.sleep(0.5)
        rec(kind, "Esc closes", pv() is None)
        for n, want in TEXT.items():
            run_("home:preview", {"id": ids[n]})
            p = wait_content(f"text {want} ")
            rec(kind, f"{n} -> text {want}", bool(p) and p["id"] == ids[n] and p["content"] == f"text {want} {len(FILES[n])}", str(p and p["content"]))
        run_("home:preview:close"); time.sleep(0.5)
        # Real mouse: double click a tile, then right-click > Preview (2nd menu row, rows 32 px).
        def tile(n):
            r = next(f for f in state()["files"] if f["name"] == n).get("rect")
            return ((r[0] + r[2]) / 2, (r[1] + r[3]) / 2) if r else None
        xy = tile("pv_b.txt")
        if xy:
            pg.mouse.dblclick(*xy); p = wait_content("text")
            rec(kind, "double-click opens preview", bool(p) and p["id"] == ids["pv_b.txt"], str(p and p["content"]))
            pg.mouse.move(640, 400); pg.keyboard.press("Escape"); time.sleep(0.5)
            xy = tile("pv_g.csv")
            pg.mouse.click(*xy, button="right"); time.sleep(0.6)
            pg.mouse.click(xy[0] + 40, xy[1] + 52); p = wait_content("text")
            rec(kind, "right-click > Preview opens", bool(p) and p["id"] == ids["pv_g.csv"], str(p and p["content"]))
            pg.mouse.move(640, 400); pg.keyboard.press("Escape"); time.sleep(0.5)
            rec(kind, "Esc closes (after mouse open)", pv() is None)
        else:
            rec(kind, "tile rects in hook", False, "no rect")
        run_("home:preview", {"id": ids["pv_c.pdf"]})
        time.sleep(1.5)
        p = pv(); n = pg.evaluate("document.querySelectorAll('iframe[data-starbase-preview]').length")
        rec(kind, "pdf -> iframe over content", p["element"] == "iframe" and n == 1, f"{p['element']} n={n}")
        loaded = pg.evaluate("(() => { const f = document.querySelector('iframe[data-starbase-preview]'); try { return f.contentDocument ? 'same-origin doc' : 'no doc'; } catch(e) { return 'blocked'; } })()")
        rec(kind, "pdf frame not refused", True, loaded)
        # Open with menu over the PDF: the frame hides so the menu shows.
        frame_vis = "getComputedStyle(document.querySelector('iframe[data-starbase-preview]')).visibility"
        pg.mouse.click(*OPEN_WITH); time.sleep(0.6)
        vis = pg.evaluate(frame_vis)
        rec(kind, "Open with menu over pdf", vis == "hidden", f"frame {vis}")
        pg.mouse.click(*OPEN_WITH); time.sleep(0.6)
        vis = pg.evaluate(frame_vis)
        rec(kind, "menu closed -> pdf frame back", vis == "visible", vis)
        # Click into the PDF (the keyboard goes to the frame), then Esc without moving back.
        pg.mouse.click(640, 450); time.sleep(0.8)
        foc = pg.evaluate("document.activeElement && document.activeElement.tagName")
        pg.keyboard.press("Escape"); time.sleep(0.8)
        rec(kind, "Esc closes after click into pdf", pv() is None, f"focus={foc}")
        run_("home:preview", {"id": ids["pv_c.pdf"]}); time.sleep(1)
        run_("home:preview", {"id": ids["pv_d.zip"]})
        time.sleep(0.8)
        n = pg.evaluate("document.querySelectorAll('[data-starbase-preview]').length")
        rec(kind, "zip -> no preview; element removed", pv()["content"] == "none" and n == 0, f"n={n}")
        if "pv_e.jpg" in ids:
            run_("home:preview", {"id": ids["pv_e.jpg"]})
            p = wait_content("image")
            rec(kind, "EXIF Orientation=6 applied", p["content"] == "image 100x300", p["content"])
        pg.mouse.move(640, 400); pg.keyboard.press("Escape"); time.sleep(0.4)
        rec(kind, "Esc closes (zip/jpg)", pv() is None)
        b.close()


for k in sys.argv[2:] or ["chromium", "firefox"]:
    try:
        run(k)
    except Exception as e:
        rec(k, "ERROR", False, f"{type(e).__name__}: {str(e)[:200]}")
print(f"{sum(results)}/{len(results)} passed")
sys.exit(0 if all(results) else 1)
