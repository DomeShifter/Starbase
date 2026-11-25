"""Round-trip every deck of the corpus through deckcraft-cli and compare before/after.

For each deck: deckcraft-cli convert deck -> rt1.pptx -> rt2.pptx; a python-pptx inventory diff of the
original vs rt1 (what was lost or changed) and of rt1 vs rt2 (a second save must change nothing);
LibreOffice (headless) PDF renders of the original and rt1 (page counts + rough per-page pixel
difference); DeckCraft's own render of both. The results are compared with baseline.json: only a
change for the worse fails the run, because some losses are known (see SOURCES.md).

usage: python run.py --cli PATH --decks DIR --work DIR [--baseline baseline.json] [--results OUT.json]
                     [--summary FILE] [--write-baseline]
"""

import argparse
import glob
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time

from PIL import Image, ImageChops, ImageStat

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import inventory  # noqa: E402

PX_TOLERANCE = 0.3  # percentage points a page's pixel difference may grow before it counts as worse


def sh(cmd, timeout=180):
    t0 = time.monotonic()
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return r.returncode, (r.stdout + r.stderr).strip(), int((time.monotonic() - t0) * 1000)
    except subprocess.TimeoutExpired:
        return -1, f"timed out after {timeout} s", int((time.monotonic() - t0) * 1000)


def soffice(args, timeout):
    prof = pathlib.Path(os.environ.get("RUNNER_TEMP", "/tmp")) / "lo-profile"
    return sh(["soffice", f"-env:UserInstallation=file://{prof}", "--headless", "--norestore", *args], timeout)


def pdf_pages(pdf):
    rc, out, _ = sh(["pdfinfo", str(pdf)], 60)
    for line in out.splitlines():
        if line.startswith("Pages:"):
            return int(line.split()[1])
    return None


def pixdiff(a_files, b_files):
    out = []
    for x, y in zip(a_files, b_files):
        ia, ib = Image.open(x).convert("RGB"), Image.open(y).convert("RGB")
        if ia.size != ib.size:
            ib = ib.resize(ia.size)
        out.append(round(sum(ImageStat.Stat(ImageChops.difference(ia, ib)).mean) / 3 / 255 * 100, 2))
    return out


def render_lo(pptx_dir, pdf_dir, names):
    """PDF of every deck in pptx_dir with LibreOffice: one batch call, then one call per missing file."""
    pdf_dir.mkdir(parents=True, exist_ok=True)
    files = [str(pptx_dir / f"{n}.pptx") for n in names if (pptx_dir / f"{n}.pptx").exists()]
    if files:
        soffice(["--convert-to", "pdf", "--outdir", str(pdf_dir), *files], 1800)
    for n in names:
        if (pptx_dir / f"{n}.pptx").exists() and not (pdf_dir / f"{n}.pdf").exists():
            soffice(["--convert-to", "pdf", "--outdir", str(pdf_dir), str(pptx_dir / f"{n}.pptx")], 240)


def load_sources():
    src = {}
    m = json.loads((HERE / "fetched.json").read_text())
    for d in m["decks"]:
        src[d["id"]] = {"source": f"{m['repos'][d['repo']]['github']} ({m['repos'][d['repo']]['licence']})", "features": d["features"]}
    return src


def check_deck(name, cli, work, info):
    orig = work / "orig" / f"{name}.pptx"
    rt1, rt2 = work / "rt1" / f"{name}.pptx", work / "rt2" / f"{name}.pptx"
    r = {"source": info.get("source", "generated (make_decks.py)"), "features": info.get("features", "")}
    rc, out, ms = sh([cli, "convert", str(orig), str(rt1)], 300)
    r["convert"] = {"ok": rc == 0 and rt1.exists(), "ms": ms, "messages": [l for l in out.splitlines() if l.strip() and not l.endswith(".pptx")][:8]}
    if r["convert"]["ok"]:
        rc, out, _ = sh([cli, "convert", str(rt1), str(rt2)], 300)
        r["reconvert_ok"] = rc == 0 and rt2.exists()
    else:
        r["reconvert_ok"] = False
    try:
        a = inventory.inv(str(orig))
    except Exception as e:
        a = {"error": f"inventory failed: {type(e).__name__}: {e}", "parts": {}}
    r["slides"] = len(a.get("slides", []))
    r["diffs"], r["rt2_diffs"] = [], []
    if r["convert"]["ok"]:
        try:
            b = inventory.inv(str(rt1))
            r["diffs"] = inventory.diff(a, b)
            if r["reconvert_ok"]:
                r["rt2_diffs"] = inventory.diff(b, inventory.inv(str(rt2)))
        except Exception as e:
            r["diffs"] = [f"inventory of the saved file failed: {type(e).__name__}: {str(e)[:120]}"]
    # DeckCraft's own render of the original and of rt1
    pages = []
    for tag, f in (("o", orig), ("r", rt1)):
        d = work / "dc" / f"{name}-{tag}"
        if f.exists():
            sh([cli, "render", str(f), "--all", "--scale", "0.5", str(d)], 300)
        pages.append(sorted(glob.glob(str(d / "*.png"))))
    r["dc_pages"] = [len(pages[0]), len(pages[1])]
    r["dc_px"] = pixdiff(pages[0], pages[1])
    return r


def lo_compare(name, work, r):
    pdf_o, pdf_r = work / "pdf-orig" / f"{name}.pdf", work / "pdf-rt1" / f"{name}.pdf"
    r["lo_pages"] = [pdf_pages(pdf_o) if pdf_o.exists() else None, pdf_pages(pdf_r) if pdf_r.exists() else None]
    imgs = []
    for tag, pdf in (("o", pdf_o), ("r", pdf_r)):
        prefix = work / "png" / f"{name}-{tag}"
        prefix.parent.mkdir(exist_ok=True)
        if pdf.exists():
            sh(["pdftoppm", "-r", "40", "-png", str(pdf), str(prefix)], 300)
        imgs.append(sorted(glob.glob(f"{prefix}-*.png")))
    r["lo_px"] = pixdiff(imgs[0], imgs[1])


def compare(cur, base):
    """(regressions, improvements) of one deck's results against its baseline entry."""
    reg, imp = [], []
    if base is None:
        return reg, imp
    for key, label in (("convert", "open + save"), ("reconvert_ok", "open + save of the saved file")):
        was = base[key]["ok"] if key == "convert" else base[key]
        now = cur[key]["ok"] if key == "convert" else cur[key]
        if was and not now:
            reg.append(f"{label} now fails")
        elif now and not was:
            imp.append(f"{label} now works")
    for key, label in (("diffs", "new loss"), ("rt2_diffs", "second save changes")):
        new = [d for d in cur[key] if d not in set(base[key])]
        gone = [d for d in base[key] if d not in set(cur[key])]
        reg += [f"{label}: {d}" for d in new]
        imp += [f"fixed: {d}" for d in gone]
    for key, label in (("lo_pages", "LibreOffice pages"), ("dc_pages", "DeckCraft render pages")):
        bo, br = base.get(key, [None, None])
        co, cr = cur.get(key, [None, None])
        if bo == br and bo is not None and co != cr:
            reg.append(f"{label}: original {co}, saved {cr} (baseline: equal)")
        if br is not None and cr is None:
            reg.append(f"{label}: the saved file no longer renders")
    for key, label in (("lo_px", "LibreOffice"), ("dc_px", "DeckCraft")):
        for i, (c, b) in enumerate(zip(cur.get(key, []), base.get(key, []))):
            if c > b + PX_TOLERANCE:
                reg.append(f"{label} render, page {i + 1}: {c}% different from the original (baseline {b}%)")
            elif c + PX_TOLERANCE < b:
                imp.append(f"{label} render, page {i + 1}: {c}% (baseline {b}%)")
    return reg, imp


def fmt_px(v):
    if not v:
        return "–"
    m = max(v)
    changed = sum(1 for x in v if x > 0)
    return f"{m:.2f}% ({changed}/{len(v)} p.)" if m else "0"


def summary(results, outcome, meta, wrote_baseline, has_baseline):
    n = len(results)
    regs = sum(1 for o in outcome.values() if o[0])
    lines = [f"## Slides .pptx round trip: {n} decks",
             "",
             f"DeckCraft `{meta['deckcraft']}`, {meta['libreoffice']}. Original → `deckcraft-cli convert` → saved (rt1) → saved again (rt2).",
             ""]
    if wrote_baseline:
        lines += ["**Baseline mode:** nothing is judged; `results.json` (artifact) is the new baseline.", ""]
    elif not has_baseline:
        lines += ["**No baseline yet:** nothing is judged. Commit this run's `results.json` as `slides/corpus/baseline.json`.", ""]
    else:
        lines += [f"**{'No regressions' if not regs else f'{regs} deck(s) got worse than the baseline'}.**", ""]
    lines += ["| Deck | Slides | Save | 2nd save same | Inventory changes | LO pages (orig/saved) | LO pixel diff (max) | DeckCraft render diff (max) | Status |",
              "|---|---|---|---|---|---|---|---|---|"]
    for name, r in sorted(results.items()):
        reg, imp = outcome[name]
        status = "❌ worse" if reg else ("⬆ better" if imp else ("known losses" if r["diffs"] else "ok"))
        if not has_baseline or wrote_baseline:
            status = "–"
        lo = r.get("lo_pages", [None, None])
        lines.append(f"| {name} | {r['slides']} | {'ok' if r['convert']['ok'] else 'FAILED'} | "
                     f"{'yes' if r['reconvert_ok'] and not r['rt2_diffs'] else 'NO'} | {len(r['diffs'])} | "
                     f"{lo[0]}/{lo[1]} | {fmt_px(r.get('lo_px'))} | {fmt_px(r.get('dc_px'))} | {status} |")
    worse = {k: v[0] for k, v in outcome.items() if v[0]}
    if worse:
        lines += ["", "### Worse than the baseline", ""]
        for k, v in sorted(worse.items()):
            lines += [f"- **{k}**"] + [f"  - `{x}`" for x in v[:20]]
    better = {k: v[1] for k, v in outcome.items() if v[1]}
    if better:
        lines += ["", "### Better than the baseline (refresh baseline.json to lock these in)", ""]
        for k, v in sorted(better.items()):
            lines += [f"- **{k}**"] + [f"  - `{x}`" for x in v[:10]]
    lines += ["", "<details><summary>All inventory changes (original → saved)</summary>", ""]
    for name, r in sorted(results.items()):
        if r["diffs"]:
            lines += [f"**{name}** ({r['features']})", ""] + [f"- `{d}`" for d in r["diffs"][:40]] + \
                     ([f"- … {len(r['diffs']) - 40} more"] if len(r["diffs"]) > 40 else []) + [""]
    lines += ["</details>", ""]
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", required=True)
    ap.add_argument("--decks", required=True, help="folder with the corpus decks (*.pptx)")
    ap.add_argument("--work", required=True)
    ap.add_argument("--baseline", default=str(HERE / "baseline.json"))
    ap.add_argument("--results", default="results.json")
    ap.add_argument("--summary")
    ap.add_argument("--write-baseline", action="store_true")
    ap.add_argument("--deckcraft", default=os.environ.get("DECKCRAFT_TAG", "?") + " @ " + os.environ.get("DECKCRAFT_COMMIT", "?")[:12])
    args = ap.parse_args()

    work = pathlib.Path(args.work)
    shutil.rmtree(work, ignore_errors=True)
    for d in ("orig", "rt1", "rt2", "dc", "png"):
        (work / d).mkdir(parents=True)
    names = sorted(pathlib.Path(f).stem for f in glob.glob(os.path.join(args.decks, "*.pptx")))
    for n in names:
        shutil.copy(os.path.join(args.decks, f"{n}.pptx"), work / "orig" / f"{n}.pptx")
    sources = load_sources()
    results = {}
    for n in names:
        t0 = time.monotonic()
        results[n] = check_deck(n, args.cli, work, sources.get(n, {}))
        print(f"{n}: convert {'ok' if results[n]['convert']['ok'] else 'FAILED'}, {len(results[n]['diffs'])} changes, "
              f"{time.monotonic() - t0:.1f} s", flush=True)
    render_lo(work / "orig", work / "pdf-orig", names)
    render_lo(work / "rt1", work / "pdf-rt1", names)
    for n in names:
        lo_compare(n, work, results[n])
    _, lo_ver, _ = soffice(["--version"], 60)

    base_path = pathlib.Path(args.baseline)
    has_baseline = base_path.exists() and not args.write_baseline
    base = json.loads(base_path.read_text())["decks"] if has_baseline else {}
    outcome = {}
    for n, r in results.items():
        reg, imp = compare(r, base.get(n)) if has_baseline else ([], [])
        if has_baseline and n not in base:
            imp.append("new deck: not in the baseline yet")
        outcome[n] = (reg, imp)

    meta = {"deckcraft": args.deckcraft, "libreoffice": lo_ver.splitlines()[0] if lo_ver else "LibreOffice ?"}
    out = {"meta": meta, "decks": results}
    pathlib.Path(args.results).write_text(json.dumps(out, indent=1, ensure_ascii=False, sort_keys=True) + "\n")
    text = summary(results, outcome, meta, args.write_baseline, has_baseline)
    if args.summary:
        with open(args.summary, "a") as f:
            f.write(text)
    print(text)
    gone = [n for n in base if n not in results]
    if gone:
        print(f"note: in the baseline but not in the corpus: {', '.join(gone)}")
    sys.exit(1 if any(o[0] for o in outcome.values()) else 0)


if __name__ == "__main__":
    main()
