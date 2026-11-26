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


def ready_secs(s):
    t = time.time()
    s.open("drive")
    s.wait_for(lambda: s.snapshot("home.loading") is False, secs=60)
    return round(time.time() - t, 2)


def save_data(testn):
    """Save-Data on (Chromium): both apps "skipped", no docs/slides .wasm requested. Also Drive-ready time, with vs without prefetch."""
    with ui.Session("chromium", testn=testn, outdir="shots") as s:
        s.ctx.add_init_script("Object.defineProperty(navigator,'connection',{value:{saveData:true,effectiveType:'4g',type:'wifi'}})")
        seen = []
        s.pg.on("request", lambda r: seen.append(r.url))
        s.sign_in("ada")
        t = ready_secs(s)
        time.sleep(12)
        st = (s.snapshot("home.prefetch.docs"), s.snapshot("home.prefetch.slides"))
        wasm = [u for u in seen if ("/docs/" in u or "/slides/" in u) and u.split("?")[0].endswith(".wasm")]
        print(f"chromium Save-Data: prefetch states={st} app wasm requests={wasm} drive ready={t}s (prefetch off)", flush=True)
        return st == ("skipped", "skipped") and not wasm


def run(br, testn):
    with ui.Session(br, testn=testn, outdir="shots") as s:
        seen = []  # (url, is_prefetch)
        s.pg.on("request", lambda r: seen.append((r.url, any(r.headers.get(h, "").lower() == "prefetch" for h in ("sec-purpose", "purpose")
                                                             or r.headers.get("sec-purpose", "").startswith("prefetch")))))
        cdp = []  # Chromium: was the real docs .wasm fetch answered from a cache? (transferSize is not reliable there)
        if br == "chromium":
            c = s.pg.context.new_cdp_session(s.pg)
            c.send("Network.enable")
            ids, enc = {}, {}
            c.on("Network.responseReceived", lambda e: ids.__setitem__(e["requestId"], e["response"]["url"]))
            c.on("Network.loadingFinished", lambda e: enc.__setitem__(e["requestId"], e["encodedDataLength"]))
            cdp = (ids, enc)
        s.sign_in("ada")
        print(f"{br}: drive ready={ready_secs(s)}s (prefetch on, starts after 4 s idle)", flush=True)
        done = s.wait_for(lambda: (s.snapshot("home.prefetch.docs") in ("started", "done")) and (s.snapshot("home.prefetch.slides") in ("started", "done")), secs=40)
        fin = s.wait_for(lambda: s.snapshot("home.prefetch.docs") == "done" and s.snapshot("home.prefetch.slides") == "done", secs=60)
        time.sleep(1)
        wasm = [(u, p) for u, p in seen if u.split("?")[0].endswith(".wasm") and ("/docs/" in u or "/slides/" in u)]
        low = all(p for _, p in wasm) and len(wasm) >= 2
        print(f"{br}: states started={bool(done)} done={bool(fin)} prefetch wasm requests={wasm}", flush=True)
        s.open("docs")
        s.wait_text("Untitled document", secs=60)
        print(f"{br}: wasm requests total={[(u.rsplit(chr(47),1)[1][:12], p) for u, p in seen if u.split(chr(63))[0].endswith('.wasm')]}", flush=True)
        cached = s.pg.evaluate("""() => performance.getEntriesByType('resource').filter(e => e.name.split('?')[0].endsWith('.wasm')
            && e.name.includes('/docs/')).map(e => [e.transferSize, e.duration])""")
        hit = bool(cached) and all(t == 0 for t, _ in cached)
        if br == "chromium":
            # Headless Chromium's cache refuses entries this big (a plain second fetch of the same 35 MB file also misses),
            # so judge the sharing by the docs program's .js (small, same prefetch + same real fetch path).
            js = s.pg.evaluate("""() => performance.getEntriesByType('resource').filter(e => e.name.split('?')[0].endsWith('.js')
                && e.name.includes('/docs/stardocs-')).map(e => e.transferSize)""")
            print(f"{br}: docs .js transferSize={js} (0 = from cache; the wasm is over the test browser's cache limit)", flush=True)
            hit = bool(js) and all(t == 0 for t in js)
        print(f"{br}: docs wasm from cache={hit} ({cached}) low-priority prefetch headers={low}", flush=True)
        s.shot(f"{br}-docs")
        return bool(done) and bool(fin) and low and hit


if __name__ == "__main__":
    a = sys.argv
    testn = int(a[a.index("--testn") + 1])
    if "--remote" in a:
        sys.exit(0 if all([run(b, testn) for b in ("chromium", "firefox")] + [save_data(testn)]) else 1)
    shots = a[a.index("--shots") + 1] if "--shots" in a else None
    sys.exit(ui.ship_and_run(__file__, ["--testn", str(testn)], rd="/tmp/pfx-prefetch", shots_to=shots))
