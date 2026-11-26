#!/usr/bin/env python3
"""Idle repaint check: real screen redraws per second while nobody touches Drive, Sheets, Docs or Slides.
Counts WebGL clear() calls (one per painted frame) over 5 s of idle, SEPARATELY from requestAnimationFrame calls:
eframe's web loop asks for an animation frame every display refresh (~60/s) but only paints when egui asked for
a repaint, so rAF calls are not repaints (an earlier measurement added both into one counter).
Fails if any app paints more than 5 times a second. Chromium and Firefox.
Needs the shared click library starbase_ui.py (STARBASE_UI_DIR, default ~/.claude/tools).
    idle-repaint.py --testn 20 [--apps drive,sheets,docs,slides] [--secs 5]
"""
import argparse, json, os, sys, time
sys.path.insert(0, os.environ.get("STARBASE_UI_DIR", os.path.expanduser("~/.claude/tools")))
from starbase_ui import Session  # noqa: E402

COUNT = (
    "window.__clr=0;window.__raf=0;"
    "for(const C of [window.WebGL2RenderingContext,window.WebGLRenderingContext]){if(!C)continue;"
    "const o=C.prototype.clear;C.prototype.clear=function(...a){window.__clr++;return o.apply(this,a)}};"
    "const _r=window.requestAnimationFrame.bind(window);window.requestAnimationFrame=(f)=>{window.__raf++;return _r(f)};"
)
LIMIT = 5.0


def measure(s, secs):
    s.pg.mouse.move(5, 795)  # park the pointer over nothing that reacts to hover
    time.sleep(3)  # let start-up work (fonts, file list, thumbnails) settle
    a = s.pg.evaluate("[window.__clr, window.__raf]")
    time.sleep(secs)
    b = s.pg.evaluate("[window.__clr, window.__raf]")
    return round((b[0] - a[0]) / secs, 1), round((b[1] - a[1]) / secs, 1)


def run(testn, apps, secs):
    out, fails = {}, 0
    for br in ("chromium", "firefox"):
        for app in apps:
            tag = f"{br}-{app}"
            try:
                with Session(br, testn=testn, outdir="shots") as s:
                    s.ctx.add_init_script(COUNT)
                    s.sign_in("ada")
                    s.open(app)
                    time.sleep(5)
                    paints, rafs = measure(s, secs)
                    ok = paints <= LIMIT
                    fails += not ok
                    out[tag] = {"paintsPerSec": paints, "animationFramesPerSec": rafs, "ok": ok}
            except Exception as e:  # one app failing must not hide the others
                fails += 1
                out[tag] = {"error": str(e)[:200]}
            print(tag, json.dumps(out[tag]), flush=True)
    print("RESULT", "PASS" if not fails else f"FAIL ({fails})")
    return 1 if fails else 0


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--testn", required=True)
    ap.add_argument("--apps", default="drive,sheets,docs,slides")
    ap.add_argument("--secs", type=float, default=5.0)
    a = ap.parse_args()
    sys.exit(run(a.testn, a.apps.split(","), a.secs))
