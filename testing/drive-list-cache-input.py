#!/usr/bin/env python3
"""Real-input check for the Drive list cache (PR 'Drive list cache'): in a real browser the Drive lists,
a file uploaded with the real Upload button shows up at once (no manual refresh), and a deleted one is gone.

    python3 testing/drive-list-cache-input.py --testn 51 [--browser chromium|firefox] [--shots DIR]
(ships itself + ~/.claude/tools/starbase_ui.py to the build box, runs there under Xvfb; start the server with
 TESTN=51 starsheets-testserver.sh up first). Checks the hook snapshot (home.files) and saves a screenshot.
"""
import json, os, sys, time


def remote(argv):
    from starbase_ui import Session
    br = argv[argv.index("--browser") + 1] if "--browser" in argv else "chromium"
    testn = int(argv[argv.index("--testn") + 1])
    name = f"cachecheck-{int(time.time())}.txt"
    path = f"/tmp/{name}"
    open(path, "w").write("hello cache")
    ok = True
    with Session(br, testn=testn, outdir="shots") as s:
        s.sign_in("ada")
        s.open("drive")
        s.wait_for(lambda: len(s.snapshot("home.files") or []) > 0, secs=30)
        n0 = len(s.snapshot("home.files") or [])
        print(br, "lists", n0, "files; first load OK")
        s.shot(f"{br}-1-list")
        s.click_text("New")
        s.upload(path, target="Upload files…")
        got = s.wait_for(lambda: name in [f["name"] for f in (s.snapshot("home.files") or [])], secs=10)
        print(br, "uploaded file in list within 10 s, no refresh:", bool(got))
        ok &= bool(got)
        s.shot(f"{br}-2-uploaded")
        r = s.pg.evaluate("fetch('api/files').then(r=>r.json()).then(j=>j.files.find(f=>f.name==%s)?.id)" % json.dumps(name))
        d = s.pg.evaluate("id => fetch('api/files/'+id,{method:'DELETE'}).then(r=>r.status)", r)
        s.hook("home:refresh")
        gone = s.wait_for(lambda: name not in [f["name"] for f in (s.snapshot("home.files") or [])], secs=10)
        print(br, "delete status", d, "gone from list:", bool(gone))
        ok &= bool(gone)
    print(br, "PASS" if ok else "FAIL")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    if "--remote" in sys.argv:
        remote(sys.argv)
    sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
    import starbase_ui
    a = sys.argv[1:]
    shots = None
    if "--shots" in a:
        i = a.index("--shots"); shots = a[i + 1]; a = a[:i] + a[i + 2:]
    sys.exit(starbase_ui.ship_and_run(__file__, a, rd="/tmp/drive-list-cache", pyname="p.py", shots_to=shots))
