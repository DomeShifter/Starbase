#!/usr/bin/env python3
"""Docs: View › Pages side by side (1 / 2 / Auto) with real input, via starbase_ui (on the build box).
default Auto = as before. Args: BASE_URL --browser chromium|firefox [--shots DIR]. Needs starbase_ui.py next to this file or on PYTHONPATH.
A 3-page document, at 2560x1080 and 1600x900: "2" in the bottom-right switch puts two
pages side by side; a click on page 2 and typing lands in page 2's text; Ctrl+End then typing reaches the end;
Auto shows as many as fit; zoom still works; the choice survives a reload (kept per browser)."""
import json, os, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
import starbase_ui as U  # noqa: E402

args = sys.argv[1:]
if "--remote" not in args:
    shots = args[args.index("--shots") + 1] if "--shots" in args else None
    sys.exit(U.ship_and_run(os.path.abspath(__file__), args, rd="/tmp/dpages-in", pyname="p.py", shots_to=shots))
base = next(a for a in args if a.startswith("http"))
br = args[args.index("--browser") + 1] if "--browser" in args else "chromium"
fails = []


def check(name, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + f"[{br}] {name}" + ("" if ok else f" ({str(detail)[:300]})"), flush=True)
    if not ok:
        fails.append(name)


with U.Session(br, base=base, outdir="shots") as s:
    s.sign_in("ada")
    for w, h in ((2560, 1080), (1600, 900)):
        tag = f"{w}x{h}"
        s.pg.set_viewport_size({"width": w, "height": h})
        s.open("docs", path="/docs/?new=1&test")
        time.sleep(2)
        call = lambda e: s._call("stardocs." + e)  # noqa: E731
        call('type("Alpha page one")'); s.key("ctrl+Return")
        call('type("Bravo page two")'); s.key("ctrl+Return")
        call('type("Charlie page three")'); time.sleep(1)
        st = s.snapshot("shell")
        if w == 2560:
            check(f"{tag} default is Auto (3 across at 100 %, as before)", st.get("pagesSideBySide") == "pagesAuto" and st.get("pageCols") == 3, st)
        # The bottom-right switch, by a real click on its "2".
        cr = st.get("canvasRect") or [0, 0, w, h]
        if w < 2000:
            s.hook("view.zoom", value=60); time.sleep(1)  # two pages only fit this width at 60 %
        for _try in range(3):
            # A real click needs the control on the physical screen (window stays 1300x900), so the wide run uses the hook.
            s.click_text("pages 2") if w < 2000 else s.hook("stardocs.pages2"); time.sleep(1.5)
            st = s.snapshot("shell")
            if st.get("pagesSideBySide") == "pages2":
                break
        check(f"{tag} 2 pages side by side", st.get("pagesSideBySide") == "pages2" and st.get("pageCols") == 2, {k: st.get(k) for k in ("pagesSideBySide", "pageCols", "canvasRect")})
        s.shot(f"{tag}-two")
        # Click in page 2 where the app says it was drawn (pageRects, screen points), then type.
        pr = st.get("pageRects") or []
        check(f"{tag} page 2 right of page 1 in one row", len(pr) >= 2 and pr[1][0] > pr[0][0] + pr[0][2] and abs(pr[1][1] - pr[0][1]) < 2, pr)
        x, y = pr[1][0] + pr[1][2] / 2, pr[1][1] + pr[1][3] * 0.3
        s.pg.mouse.click(x, y); time.sleep(0.8)
        s.pg.keyboard.type(" TWO", delay=40); time.sleep(1)
        text = s.snapshot("text") or ""
        check(f"{tag} typing on page 2 lands in page 2", "Bravo page two TWO" in text or "TWO" in text.split("Charlie")[0].split("Alpha")[-1], text)
        s.pg.keyboard.press("Control+End"); time.sleep(0.5)
        s.pg.keyboard.type("END", delay=40); time.sleep(1)
        text = s.snapshot("text") or ""
        check(f"{tag} Ctrl+End reaches the end", text.rstrip().endswith("threeEND"), text[-80:])
        s.pg.mouse.wheel(0, 4000); time.sleep(1)
        s.shot(f"{tag}-end")
        # Zoom still works with two pages.
        s.hook("view.zoom", value=60); time.sleep(1)
        st = s.snapshot()
        check(f"{tag} zoom 60 % keeps 2 across", abs((st.get("view") or {}).get("zoom", 0) - 0.6) < 0.02 and st["shell"].get("pageCols") == 2, {"zoom": (st.get("view") or {}).get("zoom"), "cols": st["shell"].get("pageCols")})
        s.hook("stardocs.pagesAuto"); time.sleep(1.5)
        st = s.snapshot("shell")
        check(f"{tag} Auto at 60 % shows all 3", st.get("pageCols") == 3, st)
        s.shot(f"{tag}-auto")
    s.hook("stardocs.pages1"); time.sleep(1)
    check("1 page: one per row", s.snapshot("shell.pageCols") == 1, s.snapshot("shell"))
    s.hook("stardocs.pages2"); time.sleep(1)
    # Remembered: reload, still 2 pages.
    s.open("docs", path="/docs/?new=1&test"); time.sleep(2)
    check("choice remembered after reload", s.snapshot("shell.pagesSideBySide") == "pages2", s.snapshot("shell"))
    s.hook("stardocs.pagesAuto"); time.sleep(1)
    print("shell", json.dumps(s.snapshot("shell"))[:300], flush=True)
sys.exit(1 if fails else 0)
