"""Real-input check of two Drive grid tweaks (needs ~/.claude/tools/starbase_ui.py).
 1. [Grid | v] has no Sort by rows (target "Date modified" absent) (Drive's own Sort menu orders the grid).
 2. Turning "Show date modified" off makes the card shorter (snapshot home.card.h), on restores it.
Run: python3 testing/drive-grid-tweaks.py --testn 42 [--shots DIR]   (ships itself to the build box; Chromium + Firefox)."""
import os
import sys

sys.path.insert(0, os.path.expanduser("~/.claude/tools"))
sys.path.insert(0, ".")
import starbase_ui as ui  # noqa: E402


def run(br, testn):
    with ui.Session(br, testn=testn, outdir="shots") as s:
        s.sign_in("ada")
        s.open("drive")
        s.click_text("Grid")
        h_on = s.snapshot("home.card.h") or 0
        s.click_text("Grid options")
        no_sort = "Date modified" not in (s.snapshot().get("targets") or {})  # a Sort-by row; the menu keeps "Show date modified"
        s.click_text("Show date modified")
        s.key("Escape")
        s.open("drive")
        h_off = s.snapshot("home.card.h") or 0
        s.shot(f"{br}-grid-nodates")
        s.click_text("Grid options")
        s.click_text("Show date modified")
        s.key("Escape")
        s.open("drive")
        h_back = s.snapshot("home.card.h") or 0
        ok = no_sort and h_off < h_on and h_back == h_on
        print(f"{br}: no Sort by in grid menu={no_sort} height on={h_on} off={h_off} back={h_back}", flush=True)
        return ok


if __name__ == "__main__":
    a = sys.argv
    testn = int(a[a.index("--testn") + 1])
    if "--remote" in a:
        sys.exit(0 if all([run(b, testn) for b in ("chromium", "firefox")]) else 1)
    shots = a[a.index("--shots") + 1] if "--shots" in a else None
    sys.exit(ui.ship_and_run(__file__, ["--testn", str(testn)], rd="/tmp/dgt-grid", shots_to=shots))
