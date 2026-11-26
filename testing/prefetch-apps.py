"""Real-input check of Drive's quiet prefetch of the Docs and Slides programs (needs ~/.claude/tools/starbase_ui.py).
Run: python3 testing/prefetch-apps.py --testn 44 [--shots DIR]   (ships itself to the build box; Chromium + Firefox).
Checks: after Drive settles, snapshot prefetch.docs/slides reach "started"/"done"; the .wasm of both apps was requested
as a prefetch (Sec-Purpose/Purpose: prefetch header = low priority); opening a doc then loads its .wasm from cache
(resource timing transferSize 0 or a cache hit)."""
import os
import sys
import time

sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
sys.path.insert(0, ".")
import starbase_ui as ui  # noqa: E402


def run(br, testn):
    with ui.Session(br, testn=testn, outdir="shots") as s:
        seen = []  # (url, is_prefetch)
        s.pg.on("request", lambda r: seen.append((r.url, any(r.headers.get(h, "").lower() == "prefetch" for h in ("sec-purpose", "purpose")
                                                             or r.headers.get("sec-purpose", "").startswith("prefetch")))))
        s.sign_in("ada")
        s.open("drive")
        done = s.wait_for(lambda: (s.snapshot("prefetch.docs") in ("started", "done")) and (s.snapshot("prefetch.slides") in ("started", "done")), secs=40)
        fin = s.wait_for(lambda: s.snapshot("prefetch.docs") == "done" and s.snapshot("prefetch.slides") == "done", secs=60)
        time.sleep(1)
        wasm = [(u, p) for u, p in seen if u.split("?")[0].endswith(".wasm") and ("/docs/" in u or "/slides/" in u)]
        low = all(p for _, p in wasm) and len(wasm) >= 2
        print(f"{br}: states started={bool(done)} done={bool(fin)} prefetch wasm requests={wasm}", flush=True)
        s.open("docs")
        s.wait_text("Untitled document", secs=60)
        cached = s.pg.evaluate("""() => performance.getEntriesByType('resource').filter(e => e.name.split('?')[0].endsWith('.wasm')
            && e.name.includes('/docs/')).map(e => [e.transferSize, e.duration])""")
        hit = bool(cached) and all(t == 0 for t, _ in cached)
        print(f"{br}: docs wasm from cache={hit} ({cached}) low-priority prefetch headers={low}", flush=True)
        s.shot(f"{br}-docs")
        return bool(done) and bool(fin) and low and hit


if __name__ == "__main__":
    a = sys.argv
    testn = int(a[a.index("--testn") + 1])
    if "--remote" in a:
        sys.exit(0 if all([run(b, testn) for b in ("chromium", "firefox")]) else 1)
    shots = a[a.index("--shots") + 1] if "--shots" in a else None
    sys.exit(ui.ship_and_run(__file__, ["--testn", str(testn)], rd="/tmp/pfx-prefetch", shots_to=shots))
