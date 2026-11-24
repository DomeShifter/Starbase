#!/usr/bin/env python3
"""Downloads the fonts in app/webfonts.txt from Google Fonts as TTF (egui reads TTF/OTF, not
WOFF2) into app/webfonts/<slug>/{regular,bold,italic,bolditalic}.ttf, plus menu.ttf (only the
letters of the font's name, for the font menu). A style a family lacks is
skipped (the app then draws it from the regular face). Each font's licence (OFL or Apache) is
saved beside it as LICENSE.txt, and app/webfonts/NOTICE.txt lists every font, its licence and
where it came from; a font without a licence fails the build. Run before `trunk build`.
Usage: scripts/fetch-fonts.py [--check]   (--check: only report what each family has)"""
import os, re, sys, urllib.parse, urllib.request

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "app")
OUT = os.path.join(ROOT, "webfonts")
STYLES = {"regular": "0,400", "bold": "0,700", "italic": "1,400", "bolditalic": "1,700"}
UA = "Mozilla/5.0"  # without a modern browser's user agent the CSS API answers with TTF links


def slug(name):
    return re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.read()


def licence(family):
    """(licence text, where it came from) for a Google Fonts family."""
    s = family.replace(" ", "").lower()
    for path in (f"google/fonts/main/ofl/{s}/OFL.txt", f"google/fonts/main/apache/{s}/LICENSE.txt",
                 f"google/fonts/main/ufl/{s}/UFL.txt", f"googlefonts/{s}/main/OFL.txt"):
        try:
            return get(f"https://raw.githubusercontent.com/{path}").decode("utf-8"), f"https://github.com/{path.replace('/main/', '/blob/main/', 1)}"
        except Exception:
            continue
    return None, None


def main():
    check = "--check" in sys.argv
    failed = []
    notice = ["Fonts served with StarSheets", "",
              "Each folder holds one font family from Google Fonts and its licence (LICENSE.txt).",
              "The menu name is what StarSheets calls it; the family is the font actually used.", ""]
    for line in open(os.path.join(ROOT, "webfonts.txt"), encoding="utf-8"):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        menu, family = [x.strip() for x in line.split("|")]
        have = []
        for style, spec in STYLES.items():
            q = urllib.parse.quote(family) + ":ital,wght@" + spec
            try:
                css = get(f"https://fonts.googleapis.com/css2?family={q}").decode()
            except Exception:
                continue
            m = re.search(r"url\((https://[^)]+\.ttf)\)", css)
            if not m:
                continue
            have.append(style)
            if not check:
                folder = os.path.join(OUT, slug(menu))
                os.makedirs(folder, exist_ok=True)
                path = os.path.join(folder, style + ".ttf")
                if not os.path.exists(path):
                    open(path, "wb").write(get(m.group(1)))
        # The font menu shows each name in its own font, like Sheets: a tiny face holding only the
        # letters of the name (Google's `text=` subset), a few KB each.
        if not check and "regular" in have:
            path = os.path.join(OUT, slug(menu), "menu.ttf")
            if not os.path.exists(path):
                q = urllib.parse.quote(family) + "&text=" + urllib.parse.quote(menu)
                m = re.search(r"url\((https://[^)]+)\)", get(f"https://fonts.googleapis.com/css2?family={q}").decode())
                if m:
                    open(path, "wb").write(get(m.group(1)))
        text, source = licence(family)
        kind = "missing" if text is None else ("SIL Open Font License 1.1" if "Open Font License" in text else "Apache License 2.0" if "Apache License" in text else "Ubuntu Font Licence" if "Ubuntu Font" in text else "see LICENSE.txt")
        if text is None:
            failed.append(f"{menu} (no licence)")
        elif not check:
            open(os.path.join(OUT, slug(menu), "LICENSE.txt"), "w", encoding="utf-8").write(text)
        notice.append(f"{slug(menu)}/  {menu} = {family}  ·  {kind}  ·  {source}")
        print(f"{menu:18} {family:18} {kind:28} {' '.join(have) or 'NOTHING'}")
        if "regular" not in have:
            failed.append(menu)
    if not check:
        open(os.path.join(OUT, "NOTICE.txt"), "w", encoding="utf-8").write("\n".join(notice) + "\n")
    if failed:
        sys.exit(f"no regular face for: {', '.join(failed)}")


if __name__ == "__main__":
    main()
