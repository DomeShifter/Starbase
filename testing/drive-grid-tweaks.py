"""Real-input check of two Drive grid tweaks (needs ~/.claude/tools/starbase_ui.py).
 1. [Grid | v] has no Sort by rows (target "Date modified" absent) (Drive's own Sort menu orders the grid).
 2. Turning "Show date modified" off makes the card shorter (snapshot home.card.h), on restores it.
 3. A search that finds text inside a file keeps its snippet line with dates off (card height stays: it has a second line).
 4. Drive's main Sort menu reorders the grid (snapshot home.files order changes).
Needs: testing/seed.py --set docs and --set types, both with --folder grid (in this order), run on the server first.
Run: python3 testing/drive-grid-tweaks.py --testn 42 [--shots DIR]   (ships itself to the build box; Chromium + Firefox)."""
import os
import sys

sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
sys.path.insert(0, ".")
import starbase_ui as ui  # noqa: E402


def run(br, testn):
    def OPEN():
        s.open("drive")
        s.hook("home:folder", path="ada/grid (2)")  # seeded: seed.py --set docs --folder grid, then --set types --folder grid (lands in "grid (2)")

    global s
    with ui.Session(br, testn=testn, outdir="shots") as s:
        s.sign_in("ada")
        OPEN()
        s.click_text("Grid")
        h_on = s.snapshot("home.card.h") or 0
        s.click_text("Grid options")
        no_sort = "Date modified" not in (s.snapshot().get("targets") or {})  # a Sort-by row; the menu keeps "Show date modified"
        s.click_text("Show date modified")
        s.key("Escape")
        OPEN()
        h_off = s.snapshot("home.card.h") or 0
        s.shot(f"{br}-grid-nodates")
        s.click_text("Grid options")
        s.click_text("Show date modified")
        s.key("Escape")
        OPEN()
        h_back = s.snapshot("home.card.h") or 0
        names = lambda: [f["name"] for f in (s.snapshot("home.files") or [])]
        # dates off again: a content search must still show its snippet line, so the card keeps its height
        s.click_text("Grid options")
        s.click_text("Show date modified")
        s.key("Escape")
        OPEN()
        h_off2 = s.snapshot("home.card.h") or 0
        s.hook("home:search", text="lazy dog")
        s.wait_for(lambda: s.snapshot("home.found"), secs=30)
        found = s.snapshot("home.found") or []
        h_snip = s.snapshot("home.card.h") or 0
        s.shot(f"{br}-grid-nodates-snippet")
        s.hook("home:search", text="")
        s.wait_for(lambda: not (s.snapshot("home.search") or ""), secs=10)
        before = names()
        s.click_text("Sort")
        s.click_text("Size")
        s.key("Escape")
        OPEN()
        s.click_text("Grid")
        s.wait_for(lambda: names(), secs=20)
        s.shot(f"{br}-grid-sorted")
        after = names()
        ok_sort = len(before) > 1 and before != after
        ok_snip = bool(found) and h_snip == h_on and h_off2 < h_on
        ok = no_sort and h_off < h_on and h_back == h_on and ok_snip and ok_sort
        print(f"{br}: no Sort by in grid menu={no_sort} height on={h_on} off={h_off} back={h_back}; off+search: no-search={h_off2} snippet-height={h_snip} found={len(found)}; sort changed order={ok_sort} {before[:3]} -> {after[:3]}", flush=True)
        return ok


if __name__ == "__main__":
    a = sys.argv
    testn = int(a[a.index("--testn") + 1])
    if "--remote" in a:
        sys.exit(0 if all([run(b, testn) for b in ("chromium", "firefox")]) else 1)
    shots = a[a.index("--shots") + 1] if "--shots" in a else None
    sys.exit(ui.ship_and_run(__file__, ["--testn", str(testn)], rd="/tmp/dgt-grid", shots_to=shots))
