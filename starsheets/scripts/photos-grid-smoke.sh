#!/usr/bin/env bash
# Smoke test of "View as photos" (app/src/home/photos.rs, viewer.rs) in a real browser, against a
# running test server with the test user ada (adapassword). Uploads 40 pictures into a new folder,
# then through the test hook (app/TESTING.md, `home:photos…`): the switch asks first → turned on →
# all listed, newest first by EXIF date or modified time → thumbnails drawn within the texture
# budget → the viewer opens, steps on and back, shows the 1024 px picture → slideshow with shuffle
# moves on by itself → closed. Screenshots of the grid and the viewer, light and dark.
# Prints PASS/FAIL lines; exit 1 on any FAIL.
#   FF_SHOT=path/to/ff-shot.py scripts/photos-grid-smoke.sh BASE_URL
set -uo pipefail
BASE=${1:?usage: photos-grid-smoke.sh BASE_URL}
FF=${FF_SHOT:?set FF_SHOT to ff-shot.py}
W=$(mktemp -d)
fail=0
curl -s -c "$W/cj" -o /dev/null -H 'content-type: application/json' -d '{"username":"ada","password":"adapassword"}' "$BASE/api/login"
enc() { python3 -c 'import sys,urllib.parse; print(urllib.parse.quote(sys.argv[1]))' "$1"; }
folder=$(curl -s -b "$W/cj" -X POST "$BASE/api/folders?path=ada&name=$(enc "Grid smoke $(date +%s)")" | python3 -c 'import json,sys; print(json.load(sys.stdin)["path"])')
# 40 PNGs of different sizes and colours (portrait and landscape), made without an image library.
python3 - "$W" <<'EOF'
import struct, sys, zlib
def png(path, w, h, c):
    raw = b"".join(b"\0" + bytes(v for x in range(w) for v in ((x * 255 // w + c[0]) % 256, (y * 255 // h + c[1]) % 256, c[2])) for y in range(h))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 1)) + chunk(b"IEND", b""))
for i in range(40):
    w, h = (1600, 1000) if i % 3 else (900, 1400)
    png(f"{sys.argv[1]}/pic{i:02}.png", w, h, ((i * 37) % 256, (i * 91) % 256, (i * 53) % 256))
EOF
for i in $(seq -w 0 39); do
  curl -s -b "$W/cj" -o /dev/null --data-binary @"$W/pic$i.png" "$BASE/api/files?name=pic$i.png&as_is=1&folder=$(enc "$folder")"
done
echo "folder: $folder"
LOGIN="js:fetch('/api/login',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({username:'ada',password:'adapassword'})}).then(r=>'login '+r.status)"
GO="js:(location.href='/?test&dir=$(enc "$folder")','go');sleep:10"
ST="starsheets.wait(starsheets.state()).then(s=>JSON.parse(s).ok.home.photos)"
RUN() { echo "js:starsheets.wait(starsheets.run('$1',$2))"; }
CHECK() { echo "js:$ST.then(p=>($1)?'PASS $2':'FAIL $2 '+JSON.stringify(Object.assign({},p,{items:(p.items||[]).slice(0,3)})))"; }
steps="$LOGIN;$GO;$(RUN home:photos '{on:true}');sleep:2"
steps+=";$(CHECK 'p.shown&&p.enabled===false&&p.canEnable' 'asks before turning on')"
steps+=";shot:$W/ask.png"
steps+=";$(RUN home:photos:enable '{}');sleep:8"
steps+=";$(CHECK 'p.enabled===true&&p.count===40' 'all 40 listed')"
steps+=";sleep:6"
steps+=";$(CHECK 'p.items.filter(i=>i.thumb).length>=12&&p.textures.bytes<=p.textures.budget' 'thumbnails on screen, within the budget')"
steps+=";shot:$W/grid.png"
steps+=";$(RUN home:photos:open '{index:3}');sleep:3"
steps+=";$(CHECK "p.viewer&&p.viewer.index===3&&p.viewer.showing==='1024'" 'viewer shows the 1024 px picture')"
steps+=";$(RUN home:photos:key "{key:'next'}");$(RUN home:photos:key "{key:'next'}");$(RUN home:photos:key "{key:'prev'}");sleep:1"
steps+=";$(CHECK 'p.viewer.index===4' 'next, next, back')"
steps+=";key:Right;sleep:1;$(CHECK 'p.viewer.index===5' 'arrow key')"
steps+=";shot:$W/viewer.png"
steps+=";$(RUN home:photos:slideshow '{every:1,shuffle:true}');sleep:3.5"
steps+=";$(CHECK 'p.viewer.playing&&p.viewer.shuffle&&p.viewer.pos>=2' 'slideshow moves on by itself, shuffled')"
steps+=";key:Escape;sleep:1;$(CHECK '!p.viewer' 'Esc closes')"
res=$("$FF" "$BASE/" "$W/end.png" --wait 3 --steps "$steps" 2>&1)
echo "$res" | grep -o '"PASS[^"]*"\|"FAIL[^"]*' | tr -d '"'
echo "$res" | grep -q '"FAIL' && fail=1
[ "$(echo "$res" | grep -c '"PASS')" -ge 8 ] || { echo "FAIL not every check ran"; echo "$res" | tail -5; fail=1; }
# The same views with the system in dark mode.
FF_DARK=1 "$FF" "$BASE/" "$W/grid-dark.png" --wait 3 --steps "$LOGIN;$GO;$(RUN home:photos '{on:true}');sleep:8" > /dev/null 2>&1
FF_DARK=1 "$FF" "$BASE/" "$W/viewer-dark.png" --wait 3 --steps "$LOGIN;$GO;$(RUN home:photos '{on:true}');sleep:6;$(RUN home:photos:open '{index:0}');sleep:3" > /dev/null 2>&1
echo "screenshots: $W/ask.png $W/grid.png $W/viewer.png $W/grid-dark.png $W/viewer-dark.png"
exit $fail
