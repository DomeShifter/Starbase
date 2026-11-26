#!/bin/sh
# Pre-compress a built site folder: a .br (brotli -q 11) and a .gz (gzip -9) next to every
# text-like file, wasm and font. starsheets-server picks .br, else .gz, else the plain file.
# Usage: scripts/precompress.sh <site-dir>   (needs the `brotli` command)
set -eu
dir="${1:?usage: precompress.sh <site-dir>}"
command -v brotli >/dev/null || { sudo apt-get install -y -qq brotli >/dev/null 2>&1 || apt-get install -y -qq brotli >/dev/null; }
find "$dir" -type f \( -name '*.wasm' -o -name '*.js' -o -name '*.css' -o -name '*.html' -o -name '*.svg' -o -name '*.json' -o -name '*.txt' -o -name '*.ttf' -o -name '*.otf' \) \
  -exec sh -c 'for f; do gzip -9 -k -f "$f"; brotli -q 11 -k -f "$f"; done' sh {} +
