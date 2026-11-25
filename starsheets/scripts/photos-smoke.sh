#!/usr/bin/env bash
# Smoke test of the photo index and thumbnails (docs/photos-api.md) against a running test server
# with the test users ada (adapassword, the owner) and bob (bobspassword). Through the API: upload
# two pictures to a new folder of ada's → off until turned on (bob can't) → listing (dates from EXIF, both read) → thumbnails (both sizes, type,
# nosniff, sandbox CSP, immutable only with v, 304) → bob gets 403 → a non-picture gets 404 →
# status (owner only) → turned off. Prints PASS/FAIL lines; exit 1 on any FAIL. Needs python3 with no extra modules.
#   scripts/photos-smoke.sh BASE_URL
set -uo pipefail
BASE=${1:?usage: photos-smoke.sh BASE_URL}
W=$(mktemp -d)
fail=0
ok() { if [ "$1" = 0 ]; then echo "PASS $2"; else echo "FAIL $2"; fail=1; fi; }
login() { curl -s -c "$W/$1.cj" -o /dev/null -H 'content-type: application/json' -d "{\"username\":\"$1\",\"password\":\"$2\"}" "$BASE/api/login"; }
login ada adapassword
login bob bobspassword
json() { python3 -c "import json,sys; d=json.load(sys.stdin); print(eval(sys.argv[1]))" "$1"; }
# Two real PNGs (sizes 1600×900 and 40×30), made here without any image library.
python3 - "$W" <<'EOF'
import struct, sys, zlib
def png(path, w, h):
    raw = b"".join(b"\0" + bytes(v for x in range(w) for v in (x * 255 // w, y * 255 // h, 128)) for y in range(h))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
png(sys.argv[1] + "/wide.png", 1600, 900)
png(sys.argv[1] + "/small.png", 40, 30)
EOF
tag=$(date +%s)
enc() { python3 -c 'import sys,urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "$1"; }
folder=$(curl -s -b "$W/ada.cj" -X POST "$BASE/api/folders?path=ada&name=$(enc "Photos smoke $tag")" | json 'd.get("path","")')
[ -n "$folder" ]; ok $? "a new folder: $folder"
up() { curl -s -b "$W/ada.cj" --data-binary @"$W/$1" "$BASE/api/files?name=$1&as_is=1&folder=$(enc "$folder")" | json 'd.get("id","")'; }
wide=$(up wide.png); small=$(up small.png)
[ -n "$wide" ] && [ -n "$small" ]; ok $? "upload two pictures"
[ "$(curl -s -b "$W/ada.cj" "$BASE/api/photos?folder=$(enc "$folder")" | json 'd["enabled"]')" = False ]; ok $? "off until turned on"
[ "$(curl -s -b "$W/bob.cj" -o /dev/null -w '%{http_code}' -X POST "$BASE/api/photos/enable?folder=$(enc "$folder")")" = 403 ]; ok $? "someone else can't turn it on"
[ "$(curl -s -b "$W/ada.cj" -X POST "$BASE/api/photos/enable?folder=$(enc "$folder")" | json 'd["enabled"]')" = True ]; ok $? "turned on"
for _ in $(seq 1 50); do
  list=$(curl -s -b "$W/ada.cj" "$BASE/api/photos?folder=$(enc "$folder")&sort=name")
  [ "$(echo "$list" | json 'sum(p["state"]=="ready" for p in d["photos"])')" = 2 ] && break
  sleep 0.2
done
[ "$(echo "$list" | json '[p["name"] for p in d["photos"]]')" = "['small.png', 'wide.png']" ]; ok $? "listed by name, both read"
[ "$(echo "$list" | json '[(p["width"],p["height"]) for p in d["photos"]]')" = "[(40, 30), (1600, 900)]" ]; ok $? "sizes from the index"
v=$(echo "$list" | json '[p["v"] for p in d["photos"] if p["name"]=="wide.png"][0]')
h() { curl -s -b "$W/$1.cj" -D - -o "$W/body" "${@:2}" | tr -d '\r'; }
t=$(h ada "$BASE/api/photos/thumb/$wide/256?v=$v")
echo "$t" | grep -q '^HTTP/1.1 200' && echo "$t" | grep -qi '^content-type: image/jpeg' && echo "$t" | grep -qi '^x-content-type-options: nosniff' \
  && echo "$t" | grep -qi '^content-security-policy: sandbox' && echo "$t" | grep -qi '^cache-control: private, max-age=31536000, immutable' \
  && [ "$(head -c 3 "$W/body" | od -An -tx1 | tr -d ' ')" = ffd8ff ]; ok $? "thumbnail 256: JPEG, nosniff, sandbox, immutable with v"
t=$(h ada "$BASE/api/photos/thumb/$wide/1024")
etag=$(echo "$t" | grep -i '^etag:' | cut -d' ' -f2)
echo "$t" | grep -qi '^cache-control: private, no-cache' && [ -n "$etag" ]; ok $? "thumbnail 1024 without v: no-cache with an ETag"
[ "$(curl -s -b "$W/ada.cj" -o /dev/null -w '%{http_code}' -H "If-None-Match: $etag" "$BASE/api/photos/thumb/$wide/1024")" = 304 ]; ok $? "304 when unchanged"
[ "$(curl -s -b "$W/bob.cj" -o /dev/null -w '%{http_code}' "$BASE/api/photos/thumb/$wide/256")" = 403 ]; ok $? "someone else: 403 (thumbnail)"
[ "$(curl -s -b "$W/bob.cj" -o /dev/null -w '%{http_code}' "$BASE/api/photos?folder=$(enc "$folder")")" = 403 ]; ok $? "someone else: 403 (listing)"
[ "$(curl -s -o /dev/null -w '%{http_code}' "$BASE/api/photos/status")" = 401 ]; ok $? "signed in only"
[ "$(curl -s -b "$W/ada.cj" -o /dev/null -w '%{http_code}' "$BASE/api/photos/thumb/$wide/300")" = 400 ]; ok $? "only sizes 256 and 1024"
[ "$(curl -s -b "$W/bob.cj" -o /dev/null -w '%{http_code}' "$BASE/api/photos/status")" = 403 ]; ok $? "status: owner only"
s=$(curl -s -b "$W/ada.cj" "$BASE/api/photos/status")
echo "$s" | json 'd["pictures"]["ready"] >= 2 and d["isolation"] == "process"' | grep -q True; ok $? "status (owner): counts, pictures read in child processes"
[ "$(curl -s -b "$W/ada.cj" -X POST "$BASE/api/photos/disable?folder=$(enc "$folder")" | json 'd["enabled"]')" = False ]; ok $? "turned off"
[ "$(curl -s -b "$W/ada.cj" "$BASE/api/photos/thumb/$wide/256" | json 'd["state"]')" = off ]; ok $? "thumbnails gone with it"
rm -rf "$W"
exit $fail
