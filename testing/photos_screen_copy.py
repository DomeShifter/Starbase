#!/usr/bin/env python3
# Last updated: 2026-11-27T08:31:28Z
"""Real-input check of the viewer's 2560 px screen copy (verified 2026-11-27).
Seed first: seed.py --set gps (needs --password adapassword, venv with Pillow), then photos_screen_copy_data.py
(adds bigland.jpg 8192x6144 and bigrot.jpg, EXIF-rotated). Prints per big photo: copy format + bytes, ms until
showing == "2560", and every /raw request seen on the network (must be none).
Run:  photos_screen_copy.py http://<host>:<port> --browser chromium|firefox [--user NAME] [--folder PATH]
Expects: home.photos.bestFormat becomes "jxl" (Chrome 155+/Firefox 158+, server with cjxl) or "jpeg";
opening a big picture full screen shows "256"/"1024" at once, then home.photos.viewer.showing == "2560"
within ~10 s; items[i].original stays false (the original is never fetched to look at it);
textures.requests grows by at most 3 for one open (1024, 2560 202-retry, 2560)."""
import os, sys, time
sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
import starbase_ui as ui


def launcher():
    sys.exit(ui.ship_and_run(os.path.abspath(__file__), sys.argv[1:], "/tmp/psc", pyname="p.py", shots_to=os.path.expanduser("~/.cache/starbase-shots/sc")))


def main():
    a = sys.argv[1:]; a.remove("--remote"); base = a[0]; br = a[a.index("--browser") + 1] if "--browser" in a else "chromium"
    user = a[a.index("--user") + 1] if "--user" in a else "ada"
    folder = a[a.index("--folder") + 1] if "--folder" in a else f"{user}/seed-gps"
    res = []
    def rec(n, ok, d=""): res.append(ok); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base); S.sign_in(user)
    # A HiDPI screen (x2): at 1x a 1280 px window never needs more than the 1024 copy, so no 2560 is asked for.
    S.pg = S.b.new_context(viewport={"width": 1280, "height": 800}, device_scale_factor=2, storage_state=S.ctx.storage_state()).new_page()
    S.open("drive"); S.hook("home:folder", path=folder); time.sleep(2); S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:enable"); time.sleep(5)
    ph = lambda: S.snapshot("home.photos") or {}
    S.hook("home:photos:tab", tab="photos"); time.sleep(2)
    rec("best format known", ph().get("bestFormat") in ("jxl", "jpeg"), str(ph().get("bestFormat")))
    its = ph().get("items") or []
    reqs = []
    S.pg.on("response", lambda r: reqs.append((r.url, r.status, r.headers.get("content-type", ""), int(r.headers.get("content-length") or 0))) if "/api/" in r.url else None)
    for name in ("bigland.jpg", "bigrot.jpg"):
        big = next((i for i, x in enumerate(its) if x.get("name") == name), None)
        rec(f"{name} listed", big is not None)
        if big is None: continue
        if name != "bigland.jpg":  # a fresh page: Escape does not always close the viewer
            S.pg.reload(); time.sleep(4); S.open("drive"); S.hook("home:folder", path=folder); time.sleep(2); S.hook("home:photos:tab", tab="photos"); time.sleep(3)
            its = ph().get("items") or []; big = next(i for i, x in enumerate(its) if x.get("name") == name)
        reqs.clear(); r0 = (ph().get("textures") or {}).get("requests", 0)
        t0 = time.time(); S.hook("home:photos:open", index=big)
        shown = []
        for _ in range(120):
            shown.append((ph().get("viewer") or {}).get("showing")); S.pg.mouse.move(600 + len(shown) % 7, 400); (S.pg.keyboard.press("+") if len(shown) in (8, 11, 14, 17) else None); time.sleep(0.25)
            if shown[-1] == "2560": break
        ms = int((time.time() - t0) * 1000)
        rec(f"{name}: viewer reaches the 2560 copy", shown[-1] == "2560", " -> ".join(dict.fromkeys(map(str, shown))) + f" in {ms} ms")
        time.sleep(1.5)
        c2560 = [q for q in reqs if "/thumb/" in q[0] and q[0].split("?")[0].endswith("/2560") and q[1] == 200]
        rec(f"{name}: 2560 copy bytes", bool(c2560), str([(q[2], q[3], q[0].split("fmt=")[-1] if "fmt=" in q[0] else "") for q in c2560]))
        rec(f"{name}: no /raw or original request", not [q for q in reqs if "/raw" in q[0] or "/thumb/" not in q[0] and "/api/files" in q[0]], str([q[0][-60:] for q in reqs])[:300])
        rec(f"{name}: original flag stays false", not (ph().get("items") or [{}])[big].get("original"))
        r1 = (ph().get("textures") or {}).get("requests", 0)
        rec(f"{name}: few requests for one open", r1 - r0 <= 6, f"{r0} -> {r1}")
        S.shot(f"{br}-{name.split('.')[0]}-viewer")
        S.hook("home:photos:close") if False else S.pg.keyboard.press("Escape"); time.sleep(1)
    S.close()
    sys.exit(0 if all(res) else 1)


if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
