#!/usr/bin/env bash
# Smoke test of the any-file Drive against a running test server with the test user ada
# (password adapassword). Through the API: upload any type (as_is) → list → preview page headers
# → raw bytes (type, nosniff, sandbox CSP, byte range) → download; HTML and SVG never shown;
# text escaped. With FF_SHOT=path/to/ff-shot.py also in the browser (the &test hook, app/TESTING.md):
# the Drive lists a picture as kind "image" and opening it goes to its preview; plus screenshots
# of a picture and a text preview, light and dark. Prints PASS/FAIL lines; exit 1 on any FAIL.
#   scripts/files-smoke.sh BASE_URL
set -uo pipefail
BASE=${1:?usage: files-smoke.sh BASE_URL}
W=$(mktemp -d)
CJ=$W/cj
fail=0
ok() { if [ "$1" = 0 ]; then echo "PASS $2"; else echo "FAIL $2"; fail=1; fi; }
curl -s -c "$CJ" -o /dev/null -H 'content-type: application/json' -d '{"username":"ada","password":"adapassword"}' "$BASE/api/login"
# A real 1×1 PNG, a markdown note with a script in it, an HTML page and an SVG with script.
printf '\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\rIDATx\x9cc\xf8\xcf\xc0\xf0\x1f\x00\x05\x00\x01\xff\x89\x99=\x1d\x00\x00\x00\x00IEND\xaeB`\x82' > "$W/dot.png"
printf '# Smoke\n<script>alert(1)</script>\n' > "$W/note.md"
printf '<script>alert(1)</script>' > "$W/page.html"
printf '<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"/>' > "$W/pic.svg"
up() { curl -s -b "$CJ" --data-binary @"$W/$1" "$BASE/api/files?name=$2&as_is=1" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("id",""))'; }
tag=$(date +%s)
png=$(up dot.png "smoke-$tag.png"); md=$(up note.md "smoke-$tag.md"); html=$(up page.html "smoke-$tag.html"); svg=$(up pic.svg "smoke-$tag.svg")
[ -n "$png" ] && [ -n "$md" ] && [ -n "$html" ] && [ -n "$svg" ]; ok $? "upload any type"
curl -s -b "$CJ" "$BASE/api/files" | grep -q "smoke-$tag.png"; ok $? "listed"
h() { curl -s -b "$CJ" -D - -o "$W/body" "$@" | tr -d '\r'; }
hp=$(h "$BASE/api/files/$png/preview")
echo "$hp" | grep -qi '^content-security-policy: sandbox' && ! echo "$hp" | grep -qi 'allow-scripts' && grep -q '<img src="raw' "$W/body" && ! grep -q '<script' "$W/body"; ok $? "picture preview: sandboxed, script-free page with <img>"
hr=$(h -H 'Range: bytes=0-7' "$BASE/api/files/$png/raw")
echo "$hr" | grep -q '^HTTP/1.1 206' && echo "$hr" | grep -qi '^content-type: image/png' && echo "$hr" | grep -qi '^x-content-type-options: nosniff' \
  && echo "$hr" | grep -qi '^content-security-policy: sandbox' && cmp -s <(head -c 8 "$W/dot.png") "$W/body"; ok $? "raw: image/png, nosniff, sandbox, 206 range"
hd=$(h "$BASE/api/files/$png")
echo "$hd" | grep -qi '^content-disposition: attachment' && echo "$hd" | grep -qi '^x-content-type-options: nosniff' && cmp -s "$W/dot.png" "$W/body"; ok $? "download: attachment, nosniff, same bytes"
refused=0
for id in $html $svg $md; do [ "$(curl -s -b "$CJ" -o /dev/null -w '%{http_code}' "$BASE/api/files/$id/raw")" = 415 ] || refused=1; done
ok $refused "HTML, SVG and text never served raw (415)"
h "$BASE/api/files/$html/preview" > /dev/null; grep -q 'data-kind="none"' "$W/body" && ! grep -q 'alert' "$W/body"; ok $? "HTML preview page: download only"
h "$BASE/api/files/$svg/preview" > /dev/null; grep -q 'data-kind="none"' "$W/body" && ! grep -q 'alert' "$W/body"; ok $? "SVG preview page: download only"
h "$BASE/api/files/$md/preview" > /dev/null; grep -q '&lt;script&gt;alert(1)&lt;/script&gt;' "$W/body"; ok $? "text preview escaped"
if [ -n "${FF_SHOT:-}" ]; then
  LOGIN="js:fetch('/api/login',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({username:'ada',password:'adapassword'})}).then(r=>'login '+r.status)"
  CHECK="js:starsheets.wait(starsheets.state()).then(s=>(s=JSON.parse(s).ok.home.files.find(f=>f.id=='$png'),s&&s.kind=='image'?'PASS Drive lists it as image':'FAIL Drive kind '+JSON.stringify(s)))"
  OPENS="js:starsheets.wait(starsheets.run('home:opens',{id:'$png'})).then(r=>(r=JSON.parse(r).ok,r&&r.opens=='preview'&&r.url.endsWith('/preview')?'PASS opens as preview':'FAIL opens '+JSON.stringify(r)))"
  res=$("$FF_SHOT" "$BASE/" "$W/drive.png" --wait 3 --steps "$LOGIN;js:(location.href='/?test','home');sleep:12;$CHECK;$OPENS" 2>&1)
  echo "$res" | grep -o '"PASS[^"]*"\|"FAIL[^"]*"' | tr -d '"'
  echo "$res" | grep -q '"PASS opens' && echo "$res" | grep -q '"PASS Drive' || fail=1
  for id in $png $md; do
    "$FF_SHOT" "$BASE/" "$W/preview-$id-light.png" --size 1000x600 --wait 2 --steps "$LOGIN;js:(location.href='/api/files/$id/preview','go');sleep:3" > /dev/null 2>&1
    FF_DARK=1 "$FF_SHOT" "$BASE/" "$W/preview-$id-dark.png" --size 1000x600 --wait 2 --steps "$LOGIN;js:(location.href='/api/files/$id/preview','go');sleep:3" > /dev/null 2>&1
  done
  echo "screenshots: $W/drive.png $W/preview-*.png"
fi
exit $fail
