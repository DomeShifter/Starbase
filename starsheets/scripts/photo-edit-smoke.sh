#!/usr/bin/env bash
# Smoke test of the photo editor (app/src/home/photo_edit.rs) in a real browser, against a running
# test server with the test user ada (adapassword). Uploads 3 pictures into a new folder, opens the
# viewer and then the editor through the test hook (app/TESTING.md, `home:photos:edit…`): opens on
# the picture → a slider changes the edits and makes it dirty → the preview renders (time printed)
# → undo/redo → each tab (screenshots light and dark) → filter + crop tools → Cancel with changes
# asks first → Esc-free close. Save is checked against what the server says (writable or why not).
# Prints PASS/FAIL lines; exit 1 on any FAIL.
#   FF_SHOT=path/to/ff-shot.py scripts/photo-edit-smoke.sh BASE_URL [SHOT_DIR]
set -uo pipefail
BASE=${1:?usage: photo-edit-smoke.sh BASE_URL [SHOT_DIR]}
FF=${FF_SHOT:?set FF_SHOT to ff-shot.py}
W=$(mktemp -d)
OUT=${2:-$W}
mkdir -p "$OUT"
fail=0
curl -s -c "$W/cj" -o /dev/null -H 'content-type: application/json' -d '{"username":"ada","password":"adapassword"}' "$BASE/api/login"
enc() { python3 -c 'import sys,urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "$1"; }
folder=$(curl -s -b "$W/cj" -X POST "$BASE/api/folders?path=ada&name=$(enc "Edit smoke $(date +%s)")" | python3 -c 'import json,sys; print(json.load(sys.stdin)["path"])')
# 3 colourful PNGs (a sky-to-ground gradient with a sun), made without an image library.
python3 - "$W" <<'EOF'
import struct, sys, zlib
def px(x, y, w, h, i):
    if (x - w * 0.7) ** 2 + (y - h * 0.3) ** 2 < (h * 0.12) ** 2:
        return (250, 210, 80)
    if y < h * 0.6:
        t = y / (h * 0.6)
        return (int(90 + 100 * t), int(150 + 60 * t), 235)
    t = (y - h * 0.6) / (h * 0.4)
    return (int(60 + 40 * i), int(140 - 60 * t), int(60 - 30 * t))
def png(path, w, h, i):
    raw = b"".join(b"\0" + bytes(v for x in range(w) for v in px(x, y, w, h, i)) for y in range(h))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 1)) + chunk(b"IEND", b""))
for i, (w, h) in enumerate([(1600, 1066), (1000, 1400), (1400, 1400)]):
    png(f"{sys.argv[1]}/pic{i}.png", w, h, i)
EOF
for i in 0 1 2; do
  curl -s -b "$W/cj" -o /dev/null --data-binary @"$W/pic$i.png" "$BASE/api/files?name=pic$i.png&as_is=1&folder=$(enc "$folder")"
done
echo "folder: $folder"
LOGIN="js:fetch('/api/login',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({username:'ada',password:'adapassword'})}).then(r=>'login '+r.status)"
GO="js:(location.href='/?test&dir=$(enc "$folder")','go');sleep:10"
ST="starsheets.wait(starsheets.state()).then(s=>JSON.parse(s).ok.home.photos.photo_edit)"
RUN() { echo "js:starsheets.wait(starsheets.run('$1',$2))"; }
CHECK() { echo "js:$ST.then(p=>($1)?'PASS $2':'FAIL $2 '+JSON.stringify(p))"; }
INFO() { echo "js:$ST.then(p=>'INFO $1 '+JSON.stringify($2))"; }
OPEN="$LOGIN;$GO;$(RUN home:photos '{on:true}');sleep:6;$(RUN home:photos:open '{index:0}');sleep:3;$(RUN home:photos:edit '{open:true}');sleep:4"
steps="$OPEN"
steps+=";$(CHECK 'p.open&&p.loaded&&p.source&&!p.dirty&&p.tab===\"adjust\"' 'opens on the picture, clean')"
steps+=";$(CHECK 'p.writable||p.whyNot' 'save state is known (writable, or the reason why not)')"
steps+=";$(INFO server '{writable:p.writable,whyNot:p.whyNot,foreign:p.foreign}')"
steps+=";$(RUN home:photos:edit:slider "{key:'exposure',value:0.8}");$(RUN home:photos:edit:slider "{key:'saturation',value:40}");sleep:2"
steps+=";$(CHECK 'p.dirty&&p.edits.exposure>0.7&&p.edits.saturation===40&&p.canUndo' 'sliders change the edits')"
steps+=";$(CHECK "p.preview==='full'&&p.renders>=1" 'preview re-rendered')"
steps+=";$(INFO 'render ms (1024 px, wasm)' 'p.renderMs')"
steps+=";shot:$OUT/adjust.png"
steps+=";$(RUN home:photos:edit:undo '{}');sleep:1;$(CHECK 'p.edits.saturation===0&&p.canRedo' 'undo')"
steps+=";$(RUN home:photos:edit:redo '{}');sleep:1;$(CHECK 'p.edits.saturation===40' 'redo')"
steps+=";$(RUN home:photos:edit:tab "{tab:'filters'}");$(RUN home:photos:edit:set "{filter:{name:'warm',amount:0.7}}");sleep:3"
steps+=";$(CHECK "p.tab==='filters'&&p.edits.filter&&p.edits.filter.name==='warm'" 'filter set')"
steps+=";shot:$OUT/filters.png"
steps+=";$(RUN home:photos:edit:tab "{tab:'crop'}");$(RUN home:photos:edit:aspect "{name:'1:1'}");$(RUN home:photos:edit:set '{angle:5}');sleep:2"
steps+=";$(CHECK "p.tab==='crop'&&p.edits.aspect==='1:1'&&p.edits.angle===5" 'crop tools')"
steps+=";shot:$OUT/crop.png"
steps+=";$(RUN home:photos:edit:compare '{on:true}');sleep:1;$(CHECK 'p.comparing' 'compare shows the original')"
steps+=";$(RUN home:photos:edit:compare '{on:false}');sleep:1"
steps+=";$(RUN home:photos:edit:save '{}');sleep:2"
steps+=";$(CHECK 'p.writable?(!p.dirty||p.ask):(p.dirty&&p.whyNot)' 'save: saved, asked, or refused with a reason')"
steps+=";$(INFO 'after save' '{dirty:p.dirty,ask:p.ask,error:p.error,message:p.message}')"
steps+=";$(RUN home:photos:edit:cancel '{}');sleep:1"
steps+=";$(CHECK 'p.open&&(p.ask||!p.dirty)' 'cancel with changes asks first')"
steps+=";$(RUN home:photos:edit:answer '{yes:true}');sleep:1"
steps+=";$(CHECK '!p.open' 'closed')"
res=$("$FF" "$BASE/" "$OUT/end.png" --size 1400x900 --wait 3 --steps "$steps" 2>&1)
echo "$res" | grep -o '"PASS[^"]*"\|"FAIL[^"]*\|"INFO[^"]*' | tr -d '"'
echo "$res" | grep -q '"FAIL' && fail=1
[ "$(echo "$res" | grep -c '"PASS')" -ge 13 ] || { echo "FAIL not every check ran"; echo "$res" | tail -5; fail=1; }
# Each tab with the system in dark mode.
for t in adjust filters crop; do
  FF_DARK=1 "$FF" "$BASE/" "$OUT/$t-dark.png" --size 1400x900 --wait 3 --steps "$OPEN;$(RUN home:photos:edit:tab "{tab:'$t'}");$(RUN home:photos:edit:set "{exposure:0.5,vibrance:30,filter:{name:'vivid',amount:0.6}}");sleep:3" > /dev/null 2>&1
done
echo "screenshots: $OUT/{adjust,filters,crop}{,-dark}.png"
exit $fail
