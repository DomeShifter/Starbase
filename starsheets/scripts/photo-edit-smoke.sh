#!/usr/bin/env bash
# Smoke test of picture edits (docs/photos-api.md, "Edits") against a running test server with the
# test users ada (adapassword, the owner) and bob (bobspassword). Through the API: upload a picture
# to a new folder of ada's, turn Photos on → GET edits (none, writable) → PUT without If-Match 428,
# bad edits 400, too big 413 → PUT saves (new etag, new thumb_v, listing's v follows) → a second save
# from the old etag 409 → the thumbnail under the new v is made → export ("Save a copy": a new
# JPEG of the edited size, a second one numbered, bob 403) → bob 403 → DELETE resets (v back to
# the original). Prints PASS/FAIL lines; exit 1 on any FAIL. Needs python3 with no extra modules.
#   scripts/photo-edit-smoke.sh BASE_URL
set -uo pipefail
BASE=${1:?usage: photo-edit-smoke.sh BASE_URL}
W=$(mktemp -d)
fail=0
ok() { if [ "$1" = 0 ]; then echo "PASS $2"; else echo "FAIL $2"; fail=1; fi; }
login() { curl -s -c "$W/$1.cj" -o /dev/null -H 'content-type: application/json' -d "{\"username\":\"$1\",\"password\":\"$2\"}" "$BASE/api/login"; }
login ada adapassword
login bob bobspassword
json() { python3 -c "import json,sys; d=json.load(sys.stdin); print(eval(sys.argv[1]))" "$1"; }
python3 - "$W" <<'EOF'
import struct, sys, zlib
w, h = 320, 200
raw = b"".join(b"\0" + bytes(v for x in range(w) for v in (x * 255 // w, y * 255 // h, 128)) for y in range(h))
chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
open(sys.argv[1] + "/pic.png", "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
EOF
tag=$(date +%s)
enc() { python3 -c 'import sys,urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "$1"; }
folder=$(curl -s -b "$W/ada.cj" -X POST "$BASE/api/folders?path=ada&name=$(enc "Edits smoke $tag")" | json 'd.get("path","")')
id=$(curl -s -b "$W/ada.cj" --data-binary @"$W/pic.png" "$BASE/api/files?name=pic.png&as_is=1&folder=$(enc "$folder")" | json 'd.get("id","")')
[ -n "$folder" ] && [ -n "$id" ]; ok $? "a new folder with a picture"
curl -s -b "$W/ada.cj" -o /dev/null -X POST "$BASE/api/photos/enable?folder=$(enc "$folder")"
listed_v() { curl -s -b "$W/ada.cj" "$BASE/api/photos?folder=$(enc "$folder")" | json '(d["photos"] or [{}])[0].get("v")'; }
for _ in $(seq 1 50); do v0=$(listed_v); [ "$v0" != None ] && break; sleep 0.2; done
[ "$v0" != None ]; ok $? "picture read (v=$v0)"
E="$BASE/api/photos/edits/$id"
g=$(curl -s -b "$W/ada.cj" "$E")
[ "$(echo "$g" | json '(d["etag"], d["writable"], d["foreign"], d["edits"]["exposure"], d["thumb_v"])')" = "('none', True, [], 0.0, '$v0')" ]; ok $? "GET: no edits yet, writable"
code() { curl -s -o "$W/out" -w '%{http_code}' -b "$W/$1.cj" "${@:2}"; }
[ "$(code ada -X PUT -d '{"exposure":1}' "$E")" = 428 ]; ok $? "PUT without If-Match: 428"
[ "$(code ada -X PUT -H 'If-Match: none' -d '{"exposure":99}' "$E")" = 400 ]; ok $? "edits out of range: 400"
python3 -c 'print("{\"aspect\":\"" + "x" * 17000 + "\"}")' > "$W/big.json"
[ "$(code ada -X PUT -H 'If-Match: none' --data-binary @"$W/big.json" "$E")" = 413 ]; ok $? "over 16 KB: 413"
[ "$(code ada -X PUT -H 'If-Match: none' -d '{"exposure":0.5,"crop":{"l":0.1,"t":0,"r":0.9,"b":1}}' "$E")" = 200 ]; ok $? "PUT saves"
etag=$(json 'd["etag"]' < "$W/out"); v1=$(json 'd["thumb_v"]' < "$W/out")
[ "$etag" != none ] && [ "$v1" != "$v0" ] && [ "$v1" != None ]; ok $? "new etag ($etag) and thumb_v ($v1)"
[ "$(listed_v)" = "$v1" ]; ok $? "the listing's v follows"
[ "$(code ada -X PUT -H 'If-Match: none' -d '{"contrast":5}' "$E")" = 409 ] && [ "$(json 'd["etag"]' < "$W/out")" = "$etag" ]; ok $? "a save from an old etag: 409 with the current etag"
[ "$(curl -s -b "$W/ada.cj" "$E" | json '(d["edits"]["exposure"], d["source"])')" = "(0.5, 'starbase')" ]; ok $? "GET reads the saved edits"
for _ in $(seq 1 50); do t=$(code ada "$BASE/api/photos/thumb/$id/256?v=$v1"); [ "$t" = 200 ] && break; sleep 0.2; done
[ "$t" = 200 ]; ok $? "thumbnail under the new v"
X="$BASE/api/photos/export/$id"
[ "$(code ada -X POST -d '{}' "$X")" = 201 ] && [ "$(json 'd["name"]' < "$W/out")" = "pic (edited).jpg" ]; ok $? "export: a new file"
newid=$(json 'd["id"]' < "$W/out")
curl -s -b "$W/ada.cj" -o "$W/ex.jpg" "$BASE/api/files/$newid"
size=$(python3 - "$W/ex.jpg" <<'PY'
import struct, sys
b = open(sys.argv[1], "rb").read(); i = 2
while i + 9 < len(b) and b[i] == 0xFF:
    m, n = b[i + 1], struct.unpack(">H", b[i + 2:i + 4])[0]
    if m in (0xC0, 0xC1, 0xC2):
        h, w = struct.unpack(">HH", b[i + 5:i + 9]); print(f"{w}x{h}"); break
    i += 2 + n
PY
)
[ "$size" = 256x200 ]; ok $? "export has the edits (crop 0.1-0.9 of 320 wide: $size)"
[ "$(code ada -X POST -d '{"format":"png","max_side":100}' "$X")" = 201 ] && [ "$(code ada -X POST "$X")" = 201 ] && [ "$(json 'd["name"]' < "$W/out")" = "pic (edited) (2).jpg" ]; ok $? "never over a file: numbered"
[ "$(code bob -X POST "$X")" = 403 ]; ok $? "export by someone else: 403"
[ "$(code bob "$E")" = 403 ] && [ "$(code bob -X PUT -H "If-Match: $etag" -d '{}' "$E")" = 403 ]; ok $? "someone else: 403"
[ "$(code ada -X DELETE -H "If-Match: $etag" "$E")" = 200 ] && [ "$(json '(d["etag"], d["thumb_v"])' < "$W/out")" = "('none', '$v0')" ]; ok $? "DELETE resets: sidecar gone, v back"
rm -rf "$W"
exit $fail
