#!/usr/bin/env bash
# Sets the GridCraft, WordCraft and DeckCraft `commit:` lines in the Flatpak manifest to the
# commits in starsheets/, stardocs/ and slides/upstream.pin, so the pin files stay the one place an engine
# bump is made. CI runs it before every Flatpak build and warns when the committed manifest was
# behind; run it by hand after a bump to keep the committed manifest current.
#
#   packaging/linux/flatpak/pin-engines.sh            update the manifest in place
#   packaging/linux/flatpak/pin-engines.sh --check    only report; exit 1 if it is behind
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
MANIFEST="$ROOT/packaging/linux/flatpak/io.github.DomeShifter.Starbase.yml"
check=false; [[ "${1:-}" == --check ]] && check=true
behind=0
for pair in starsheets:gridcraft stardocs:wordcraft slides:deckcraft; do
  ws="${pair%%:*}" engine="${pair##*:}"
  pin=$(sed -n 's/^commit = "\([0-9a-f]\{40\}\)"/\1/p' "$ROOT/$ws/upstream.pin")
  [[ -n "$pin" ]] || { echo "no commit in $ws/upstream.pin" >&2; exit 2; }
  # The commit line follows the fork's url line.
  now=$(sed -n "/url: https:\/\/github.com\/DomeShifter\/$engine.git/{n;s/^ *commit: \([0-9a-f]*\)$/\1/p}" "$MANIFEST")
  [[ -n "$now" ]] || { echo "no $engine git source in the manifest" >&2; exit 2; }
  if [[ "$now" == "$pin" ]]; then
    echo "$engine: $pin (matches $ws/upstream.pin)"
    continue
  fi
  behind=1
  echo "$engine: manifest had $now, $ws/upstream.pin has $pin"
  $check || sed -i "/url: https:\/\/github.com\/DomeShifter\/$engine.git/{n;s/commit: $now\$/commit: $pin/}" "$MANIFEST"
done
if $check && (( behind )); then exit 1; fi
exit 0
