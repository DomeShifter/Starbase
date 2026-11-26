#!/usr/bin/env python3
"""Real input for pre-compressed serving: / , /docs/ and /slides/ start in Chromium and Firefox (no broken wasm/font).
Usage: python3 -I testing/precompress-load-input.py --testn N   (site must be pre-compressed, scripts/precompress.sh)"""
import os, sys
sys.path.insert(0, os.environ.get("STARBASE_UI_DIR", os.path.expanduser("~/.claude/tools")))
from starbase_ui import Session, ship_and_run  # noqa: E402


def run(testn):
    fails = 0
    for br in ("chromium", "firefox"):
        for app in ("drive", "docs", "slides"):
            tag = f"{br}/{app}"
            try:
                with Session(br, phone=False, testn=testn, outdir="shots") as s:
                    s.sign_in("ada")
                    bad = []
                    s.pg.on("requestfailed", lambda r: bad.append(r.url[-40:]))
                    s.pg.on("response", lambda r: bad.append(f"{r.status} {r.url[-40:]}") if r.status >= 400 and "/api/" not in r.url else None)
                    s.open(app)
                    st = s.snapshot()
                    s.shot(f"{br}-{app}")
                    ok = bool(st) and not bad
                    print(f"{'PASS' if ok else 'FAIL'} [{tag}] hook={s.hookname} state={bool(st)} bad={bad[:3]}", flush=True)
                    fails += not ok
            except Exception as e:
                fails += 1; print(f"FAIL [{tag}] {type(e).__name__}: {str(e)[:200]}", flush=True)
    return fails


if __name__ == "__main__":
    a = sys.argv[1:]
    testn = int(a[a.index("--testn") + 1])
    if "--remote" not in a:
        sys.exit(ship_and_run(os.path.abspath(__file__), a, "/tmp/pcload", pyname="pc_load.py", shots_to=os.environ.get("SHOTS", "/tmp/pcload-shots")))
    sys.exit(1 if run(testn) else 0)
