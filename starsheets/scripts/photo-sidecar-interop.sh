#!/usr/bin/env bash
# Sidecar interop spike for the photo editor (docs/photos-edit.md, "Interop results"). Re-runnable.
# Hand-writes XMP sidecars (crs: crop/angle/tone + our starbase: namespace) for two small test JPEGs
# (one plain, one with EXIF Orientation=6), then checks what darktable-cli applies, whether darktable
# rewrites the sidecar, and whether exiv2 (a stand-in for digiKam, which uses libexiv2) and exiftool
# read our fields. Prints one result line per check, plus a montage of every darktable output.
#
# Run it in a THROWAWAY container on any Docker host (needs network for apt; nothing else is touched):
#   docker run --rm -v "$PWD/out:/out" -v "$PWD/starsheets/scripts:/s:ro" debian:stable \
#     bash /s/photo-sidecar-interop.sh /out
# Takes ~8 min (the GUI cases open real darktable under Xvfb, one at a time).
# Results: /out/results.tsv, /out/montage.png, and every sidecar before/after in /out/case-*/.
set -uo pipefail
OUT=${1:-/out}
mkdir -p "$OUT"
if ! command -v darktable-cli >/dev/null || ! command -v xdotool >/dev/null; then
  apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
    darktable libimage-exiftool-perl exiv2 imagemagick xvfb xauth xdotool fonts-dejavu-core bsdextrautils >"$OUT/apt.log" 2>&1 || { echo "apt failed"; exit 1; }
fi
IM=convert; command -v magick >/dev/null && IM=magick
RES="$OUT/results.tsv"
echo -e "case\trun\tout_wh\tmean_rgb\tsat\tsidecar_after\tnotes" >"$RES"
echo "darktable $(darktable-cli --version 2>/dev/null | head -1 | awk '{print $2}'), exiftool $(exiftool -ver), $(exiv2 --version | head -1)" >"$OUT/versions.txt"

# Test image: 600x400, four mid-tone quadrants (stored pixels): TL red, TR green, BL blue, BR yellow,
# so a crop shows WHICH part of the stored image was kept and a rotation shows up in the dimensions.
W0=$OUT/base
mkdir -p "$W0"
$IM -size 300x200 xc:'rgb(190,70,70)' -size 300x200 xc:'rgb(70,170,70)' +append \
  \( -size 300x200 xc:'rgb(70,90,190)' -size 300x200 xc:'rgb(190,180,60)' +append \) -append \
  -quality 95 "$W0/plain.jpg"
cp "$W0/plain.jpg" "$W0/rot6.jpg"
exiftool -q -overwrite_original -Orientation#=6 "$W0/rot6.jpg"

SB='https://github.com/DomeShifter/Starbase/ns/photo-edit/1.0/'
# sidecar FILE [crs attributes...] — writes an XMP packet the way our server will (no softwareAgent,
# no darktable:history), with starbase:Edits holding the JSON.
sidecar() {
  local f=$1; shift
  {
    echo '<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>'
    echo '<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
    echo ' <rdf:Description rdf:about=""'
    echo '  xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"'
    echo '  xmlns:tiff="http://ns.adobe.com/tiff/1.0/"'
    echo "  xmlns:starbase=\"$SB\""
    echo '  crs:ProcessVersion="11.0"'
    for a in "$@"; do echo "  $a"; done
    echo '  starbase:EditsVersion="1"'
    echo "  starbase:Edits='{\"v\":1}'/>"
    echo '</rdf:RDF></x:xmpmeta>'
    echo '<?xpacket end="w"?>'
  } >"$f"
}
CROP_TL='crs:HasCrop="True" crs:CropLeft="0" crs:CropTop="0" crs:CropRight="0.5" crs:CropBottom="0.5" crs:CropAngle="0"'
TONE='crs:Exposure2012="+1.00" crs:Contrast2012="+50" crs:Saturation="-100" crs:ConvertToGrayscale="True" crs:Vibrance="-100" crs:Highlights2012="-50" crs:Shadows2012="+50" crs:IncrementalTemperature="+40" crs:IncrementalTint="+20"'

measure() { # file -> "WxH<TAB>r,g,b<TAB>sat"
  local wh rgb sat
  wh=$(identify -format '%wx%h' "$1" 2>/dev/null)
  rgb=$($IM "$1" -auto-orient -resize 1x1\! -format '%[fx:int(255*r)],%[fx:int(255*g)],%[fx:int(255*b)]' info: 2>/dev/null)
  sat=$($IM "$1" -colorspace HSL -channel G -separate -format '%[fx:mean]' info: 2>/dev/null | cut -c1-5)
  printf '%s\t%s\t%s' "$wh" "$rgb" "$sat"
}

# runcase NAME IMAGE SIDECAR_NAME(photo.jpg.xmp|photo.xmp|none) DT_MODE NOTES CRS...
#   DT_MODE: auto  = darktable-cli IN OUT (darktable finds sidecars itself), library + presets on
#            nopre = same with --apply-custom-presets false (in-memory library)
#            xmparg= darktable-cli IN SIDECAR OUT (sidecar passed explicitly)
runcase() {
  local name=$1 img=$2 scname=$3 mode=$4 notes=$5; shift 5
  local d="$OUT/case-$name"; rm -rf "$d"; mkdir -p "$d/cfg"
  cp "$W0/$img" "$d/photo.jpg"
  local sc=""
  if [ "$scname" != none ]; then
    sc="$d/$( [ "$scname" = photo.xmp ] && echo photo.xmp || echo photo.jpg.xmp )"
    sidecar "$sc" "$@"; cp "$sc" "$d/sidecar.before"
  fi
  local args=("$d/photo.jpg")
  [ "$mode" = xmparg ] && args+=("$sc")
  args+=("$d/out.jpg")
  [ "$mode" = nopre ] && args+=(--apply-custom-presets false)
  darktable-cli "${args[@]}" --core --configdir "$d/cfg" --conf plugins/lighttable/export/force_lcms2=FALSE >"$d/dt.log" 2>&1
  local after="none"
  if [ -n "$sc" ]; then
    if cmp -s "$sc" "$d/sidecar.before"; then after=unchanged
    else after="REWRITTEN"; grep -q 'darktable:history' "$sc" && after+="+dt-history"
      grep -q 'starbase:Edits' "$sc" && after+="+starbase-kept" || after+="+starbase-LOST"
      grep -q 'crs:CropTop' "$sc" && after+="+crs-kept" || after+="+crs-LOST"; fi
  fi
  # darktable may also create its own photo.jpg.xmp when we used photo.xmp
  [ "$scname" = photo.xmp ] && [ -f "$d/photo.jpg.xmp" ] && after+=";dt-created-photo.jpg.xmp" && cp "$d/photo.jpg.xmp" "$d/dt-created.xmp"
  if [ -f "$d/out.jpg" ]; then
    echo -e "$name\t$mode\t$(measure "$d/out.jpg")\t$after\t$notes" >>"$RES"
    $IM "$d/out.jpg" -auto-orient -resize 200x200 -background white -gravity center -extent 210x210 \
      -gravity south -pointsize 11 -annotate +0+0 "$name" "$d/tile.png"
  else
    echo -e "$name\t$mode\tNO-OUTPUT\t-\t-\t$after\t$notes ($(tail -1 "$d/dt.log"))" >>"$RES"
  fi
}

runcase base plain.jpg none auto "no sidecar: baseline"
runcase base6 rot6.jpg none auto "no sidecar, EXIF orientation 6: baseline"
runcase jx-crop plain.jpg photo.jpg.xmp auto "photo.jpg.xmp, crop to stored TL (red)" $CROP_TL
runcase x-crop plain.jpg photo.xmp auto "photo.xmp (LR name), crop to stored TL (red)" $CROP_TL
runcase x-crop-np plain.jpg photo.xmp nopre "photo.xmp, presets off" $CROP_TL
runcase jx-crop-arg plain.jpg photo.jpg.xmp xmparg "photo.jpg.xmp passed as argument" $CROP_TL

# The Lightroom import only runs when darktable's DARKROOM loads an image for the first time
# (src/develop/develop.c dt_dev_read_history_ext → dt_lightroom_import), never in darktable-cli.
# So: open each case in the real GUI under Xvfb (`darktable FILE` opens it in the darkroom), wait
# until darktable has written its own history to photo.jpg.xmp, quit with Ctrl+Q, then render that
# history with darktable-cli and measure.
# guicase NAME IMAGE SIDECAR_NAME NOTES CRS...
Xvfb :9 -screen 0 1600x1000x24 >/dev/null 2>&1 &
XVFB=$!
sleep 2
guicase() {
  local name=$1 img=$2 scname=$3 notes=$4; shift 4
  local d="$OUT/gui-$name"; rm -rf "$d"; mkdir -p "$d/cfg"
  cp "$W0/$img" "$d/photo.jpg"
  local sc="$d/$scname"
  sidecar "$sc" "$@"; cp "$sc" "$d/sidecar.before"
  DISPLAY=:9 darktable --configdir "$d/cfg" --library "$d/lib.db" "$d/photo.jpg" >"$d/gui.log" 2>&1 &
  local dp=$! i
  for i in $(seq 1 90); do sleep 1; grep -q 'darktable:history_end' "$d/photo.jpg.xmp" 2>/dev/null && break; done
  sleep 4
  DISPLAY=:9 xdotool key ctrl+q
  for i in $(seq 1 15); do sleep 1; kill -0 $dp 2>/dev/null || break; done
  kill -9 $dp 2>/dev/null; wait $dp 2>/dev/null  # Ctrl+Q is sometimes swallowed under Xvfb
  local ops after="photo.jpg.xmp:" lr=""
  grep -q 'invalid crop data' "$d/gui.log" && lr=" [log: invalid crop data]"
  ops=$(grep -o 'darktable:operation="[a-z]*"' "$d/photo.jpg.xmp" 2>/dev/null | cut -d'"' -f2 | sort -u | tr '\n' ' ')
  grep -q 'starbase:Edits' "$d/photo.jpg.xmp" 2>/dev/null && after+="starbase-kept," || after+="no-starbase,"
  grep -q 'CropTop' "$d/photo.jpg.xmp" 2>/dev/null && after+="crs-kept," || after+="no-crs,"
  if [ "$scname" = photo.xmp ]; then
    if cmp -s "$sc" "$d/sidecar.before"; then after+="photo.xmp-unchanged"; else after+="photo.xmp-CHANGED"; fi
  fi
  notes+=$lr
  darktable-cli "$d/photo.jpg" "$d/photo.jpg.xmp" "$d/out.jpg" --core --configdir "$d/cfg-cli" >"$d/cli.log" 2>&1
  if [ -f "$d/out.jpg" ]; then
    echo -e "$name\tgui\t$(measure "$d/out.jpg")\t$after\t$notes; history: $ops" >>"$RES"
    $IM "$d/out.jpg" -auto-orient -resize 200x200 -background white -gravity center -extent 210x210 \
      -gravity south -pointsize 11 -annotate +0+0 "$name" "$d/tile.png"
  else
    echo -e "$name\tgui\tNO-OUTPUT\t-\t-\t$after\t$notes; history: $ops" >>"$RES"
  fi
}
WH='tiff:ImageWidth="600" tiff:ImageLength="400"'
guicase jx-ours plain.jpg photo.jpg.xmp "our photo.jpg.xmp: crop TL + tone" $CROP_TL $TONE $WH
guicase x-crop-nowh plain.jpg photo.xmp "photo.xmp crop TL, no tiff:ImageWidth/Length" $CROP_TL
guicase x-crop plain.jpg photo.xmp "photo.xmp crop TL (expect red 300x200)" $CROP_TL $WH
guicase x-crop-r plain.jpg photo.xmp "photo.xmp crop 0.5..1 x 0..0.5 (expect green)" \
  'crs:HasCrop="True" crs:CropLeft="0.5" crs:CropTop="0" crs:CropRight="1" crs:CropBottom="0.5"' $WH
guicase x-exp plain.jpg photo.xmp "photo.xmp Exposure2012 +1 only" 'crs:Exposure2012="+1.00"' $WH
guicase x-tone plain.jpg photo.xmp "photo.xmp all tone/colour fields" $TONE $WH
guicase x-angle plain.jpg photo.xmp "photo.xmp crop 0.1..0.9 + CropAngle 10" \
  'crs:HasCrop="True" crs:CropLeft="0.1" crs:CropTop="0.1" crs:CropRight="0.9" crs:CropBottom="0.9" crs:CropAngle="10"' $WH
guicase x6-crop rot6.jpg photo.xmp "EXIF orient 6, crop TL: red=stored coords, blue=displayed" $CROP_TL $WH
guicase x6-crop-dwh rot6.jpg photo.xmp "EXIF orient 6, crop TL, ImageWidth/Length = displayed 400x600" $CROP_TL \
  'tiff:ImageWidth="400" tiff:ImageLength="600"'
guicase x6-tiff6 rot6.jpg photo.xmp "EXIF orient 6 + tiff:Orientation=6 in sidecar" $CROP_TL $WH 'tiff:Orientation="6"'
guicase x-tiff6 plain.jpg photo.xmp "EXIF orient 1 + tiff:Orientation=6 in sidecar" $WH 'tiff:Orientation="6"'
guicase x-rational plain.jpg photo.xmp "crop as rationals 1/2" \
  'crs:HasCrop="True" crs:CropLeft="0/1" crs:CropTop="0/1" crs:CropRight="1/2" crs:CropBottom="1/2"' $WH
kill $XVFB 2>/dev/null

# Readers of our namespace: exiv2 (digiKam's library) and exiftool on the plain sidecar.
d="$OUT/case-jx-crop"
{
  echo "== exiv2 -px (digiKam reads via libexiv2) =="; exiv2 -px "$d/sidecar.before" 2>&1
  echo "== exiftool -XMP:all -G1 =="; exiftool -XMP:all -G1 -a "$d/sidecar.before" 2>&1
  echo "== exiftool on the JPEG (does it pick up photo.jpg.xmp? it should not) =="; exiftool -XMP-crs:all "$d/photo.jpg" 2>&1
  echo "== exiftool -n -CropTop types =="; exiftool -n -XMP-crs:CropTop -XMP-crs:CropRight "$d/sidecar.before"
  echo "== exiv2 modify round-trip keeps starbase =="; cp "$d/sidecar.before" "$OUT/rt.xmp"
  exiv2 -M'set Xmp.dc.title test' "$OUT/rt.xmp" 2>&1; grep -c 'starbase:Edits' "$OUT/rt.xmp"
  echo "== exiftool write round-trip keeps starbase (no config) =="; cp "$d/sidecar.before" "$OUT/rt2.xmp"
  exiftool -q -overwrite_original -XMP-dc:Title=test "$OUT/rt2.xmp" 2>&1; grep -c 'starbase:Edits' "$OUT/rt2.xmp"
} >"$OUT/readers.txt"

$IM "$OUT"/case-*/tile.png "$OUT"/gui-*/tile.png +append "$OUT/montage.png" 2>/dev/null
column -t -s $'\t' "$RES"
