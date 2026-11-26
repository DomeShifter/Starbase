#!/usr/bin/env python3
"""Docs comment column with real input (needs ~/.claude-style tool starbase_ui on the path or in STARBASE_UI_DIR).
Args: BASE_URL --browser chromium|firefox [--size 1600x900] [--shots DIR]
2-page doc, pages side by side, comment on page 1: no comment card overlaps page 2 (test hook rects); the
Comments pane lists it and a real click on its card makes it the shown (highlighted) comment; single-page view
still shows cards. Uses the StarDocs test hook fields pages / balloons / paneCards / shownComment."""
import sys, os, json, time
sys.path.insert(0, os.environ.get("STARBASE_UI_DIR", os.path.expanduser("~/.claude/tools")))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import starbase_ui as U
args = sys.argv[1:]
if "--remote" not in args:
    sys.exit(U.ship_and_run(__file__, args, rd="/tmp/dcc-in", pyname="p.py", shots_to=(args[args.index("--shots") + 1] if "--shots" in args else None)))
base = next(a for a in args if a.startswith("http")); br = args[args.index("--browser") + 1] if "--browser" in args else "chromium"
W, H = map(int, (args[args.index("--size") + 1] if "--size" in args else "1600x900").split("x"))
tag = f"{br} {W}x{H}"
fails = []
def check(n, ok, d=""):
    print(("PASS " if ok else "FAIL ") + f"[{tag}] {n}" + ("" if ok else f" ({str(d)[:400]})"), flush=True)
    if not ok: fails.append(n)
def hit(a, b):  # [x, y, w, h] rects intersect (more than a hairline)
    return a[0] < b[0] + b[2] - 1 and b[0] < a[0] + a[2] - 1 and a[1] < b[1] + b[3] - 1 and b[1] < a[1] + a[3] - 1
with U.Session(br, base=base, outdir="shots") as s:
    # The window size under test: a real W x H window (Chromium: no emulated viewport, so the screen maths stays true).
    s.ctx.close()
    if br == "chromium":
        s.b.close(); s.b = s.pw.chromium.launch(headless=False, args=[f"--window-size={W},{H + 87}", "--window-position=0,0"])
        s.ctx = s.b.new_context(no_viewport=True)
    else:
        s.ctx = s.b.new_context(viewport={"width": W, "height": H})
    s.pg = s.ctx.new_page()
    s.sign_in("ada"); s.open("docs", path="/docs/?new=1&test"); time.sleep(3)
    print("viewport", s.pg.evaluate("[innerWidth, innerHeight]"), flush=True)
    call = lambda e: s._call("stardocs." + e)
    call('type("Hello brave world, the hall is booked.")'); s.key("ctrl+Return"); call('type("Second page text")')
    s.hook("caret.docStart"); s.key("shift+End")
    s.hook("review.newComment", text="Is the hall free?"); time.sleep(1)
    s.key("Escape"); time.sleep(0.5)
    s.hook("view.multiplePages"); time.sleep(2)
    st = s.snapshot(); pages, cards = st.get("pages") or [], st.get("balloons") or []
    print("view", json.dumps(st.get("view"))[:300], "\npages", pages, "\nballoons", cards, flush=True)
    check("two pages side by side", len(pages) >= 2 and abs(pages[1][1] - pages[0][1]) < 5 and pages[1][0] > pages[0][0], pages)
    check("a card shows", bool(cards), cards)
    check("no card overlaps page 2", all(not hit(c["rect"], p) for c in cards for p in pages[1:]), (pages, cards))
    check("card sits right of page 1", bool(cards and pages) and all(c["rect"][0] >= pages[0][0] + pages[0][2] - 1 for c in cards), (pages, cards))
    s.shot(f"side-by-side-{br}-{W}")
    cid = cards[0]["id"] if cards else None
    # Comments pane (far right): lists the comment; clicking its card highlights the linked text.
    s.hook("view.commentsPane", value=True); time.sleep(2)
    st = s.snapshot(); pages = st.get("pages") or pages
    if len(pages) > 1: s.click_xy(pages[1][0] + pages[1][2] / 2, pages[1][1] + 120)  # caret onto page 2
    else: s.click_xy(pages[0][0] + pages[0][2] / 2, pages[0][1] + pages[0][3] - 60)
    time.sleep(1)
    st = s.snapshot(); pc = st.get("paneCards") or []
    print("paneCards", pc, "shown", st.get("shownComment"), "pages", st.get("pages"), flush=True)
    check("pane lists the comment", any(c["id"] == cid for c in pc), pc)
    check("click away un-highlights", st.get("shownComment") is None, st.get("shownComment"))
    mine = [c for c in pc if c["id"] == cid]
    if mine:
        r = mine[0]["rect"]; s.click_xy(r[0] + r[2] / 2, r[1] + 14); time.sleep(1.2)
        st = s.snapshot(); check("pane card click highlights its text", st.get("shownComment") == cid, st.get("shownComment"))
    s.shot(f"pane-{br}-{W}")
    s.hook("view.commentsPane", value=False); time.sleep(1)
    s.hook("view.zoom", value=100); time.sleep(2)
    st = s.snapshot(); pages, cards = st.get("pages") or [], st.get("balloons") or []
    print("single view", json.dumps(st.get("view"))[:300], "\npages", pages, "\nballoons", cards, flush=True)
    check("single-page view: pages stacked", len(pages) < 2 or pages[1][1] > pages[0][1] + pages[0][3] - 1, pages)
    check("single-page view: cards still show", bool(cards), cards)
print("RESULT", tag, ("FAIL " + ", ".join(fails)) if fails else "ALL PASS", flush=True)
sys.exit(1 if fails else 0)
