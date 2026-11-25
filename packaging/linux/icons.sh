#!/usr/bin/env bash
# Stage the three Starbase menu icons as a hicolor icon theme, from the apps' own SVGs, so the
# artwork lives in one place (the app folders) and nothing generated is committed here.
#
#   packaging/linux/icons.sh <dest>     e.g. <AppDir>/usr/share/icons  or  /app/share/icons
#
# Writes <dest>/hicolor/scalable/apps/<id>.svg and, when rsvg-convert or resvg is on PATH,
# <dest>/hicolor/<n>x<n>/apps/<id>.png for n in 16 24 32 48 64 128 256 512.
# Icons that are not square (the Starbase Docs logo is 48x64) are centred on a square canvas first,
# because icon themes scale every icon to a square.
#
# Starbase Drive's icon is starsheets/app/stardrive.svg, which is being designed on the `icons`
# branch. Until that is merged, the Starbase Sheets icon stands in and a warning is printed.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEST="${1:?usage: icons.sh <dest dir, e.g. AppDir/usr/share/icons>}"
ID=io.github.DomeShifter.Starbase

SHEETS_SVG="$ROOT/starsheets/app/favicon.svg"
DOCS_SVG="$ROOT/stardocs/app/icons/stardocs_logo.svg"
DRIVE_SVG="$ROOT/starsheets/app/stardrive.svg"
if [[ ! -f "$DRIVE_SVG" ]]; then
  echo "warning: $DRIVE_SVG not found (icons branch not merged yet); using the Starbase Sheets icon for Starbase Drive" >&2
  DRIVE_SVG="$SHEETS_SVG"
fi

RENDER=""
if command -v rsvg-convert >/dev/null; then
  RENDER=rsvg
elif command -v resvg >/dev/null; then
  RENDER=resvg
else
  echo "warning: neither rsvg-convert nor resvg found; staging scalable SVG icons only" >&2
fi

# Copy an SVG, padding its viewBox to a square (centred) when it is not one.
square_svg() {
  python3 -I - "$1" "$2" <<'PY'
import re, sys
src, dst = sys.argv[1], sys.argv[2]
svg = open(src, encoding="utf-8").read()
m = re.search(r'<svg\b[^>]*>', svg)
tag = m.group(0)
vb = re.search(r'viewBox="([^"]+)"', tag)
x, y, w, h = (float(v) for v in vb.group(1).replace(",", " ").split())
if w != h:
    s = max(w, h)
    x -= (s - w) / 2
    y -= (s - h) / 2
    new = re.sub(r'viewBox="[^"]+"', f'viewBox="{x:g} {y:g} {s:g} {s:g}"', tag)
    new = re.sub(r'\s(width|height)="[^"]*"', "", new)
    svg = svg.replace(tag, new, 1)
open(dst, "w", encoding="utf-8").write(svg)
PY
}

stage() {
  local svg="$1" name="$2" size out
  out="$DEST/hicolor/scalable/apps/$name.svg"
  mkdir -p "$(dirname "$out")"
  square_svg "$svg" "$out"
  [[ -n "$RENDER" ]] || return 0
  for size in 16 24 32 48 64 128 256 512; do
    mkdir -p "$DEST/hicolor/${size}x${size}/apps"
    if [[ "$RENDER" = rsvg ]]; then
      rsvg-convert -w "$size" -h "$size" -o "$DEST/hicolor/${size}x${size}/apps/$name.png" "$out"
    else
      resvg -w "$size" -h "$size" "$out" "$DEST/hicolor/${size}x${size}/apps/$name.png"
    fi
  done
  return 0
}

stage "$DRIVE_SVG" "$ID"
stage "$SHEETS_SVG" "$ID.Sheets"
stage "$DOCS_SVG" "$ID.Docs"
echo "icons staged in $DEST/hicolor"
