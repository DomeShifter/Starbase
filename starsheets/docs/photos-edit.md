# Photo edits: edit model and XMP sidecar (spec)

This is the official spec for the Photos image editor. It is the lead's frozen contract (v1) plus
the exact sidecar mapping, the merge rules, and what other photo tools do with our sidecars,
measured in an interop spike (`scripts/photo-sidecar-interop.sh`, results at the end).

Edits are non-destructive: the photo file is never changed. Edits live in an XMP sidecar next to
it, `photo.jpg.xmp`. "Save a copy" (export) writes a new file.

Code: crate `crates/photo-edit` (edit model + render), `crates/server/src/photos/` (sidecar +
routes), the app's editor panel. API calls: `docs/photos-api.md`, section "Edits".

## 1. Edit model (JSON)

```json
{ "v": 1, "rot90": 0, "flip_h": false, "angle": 0.0,
  "crop": {"l": 0.0, "t": 0.0, "r": 1.0, "b": 1.0}, "aspect": null,
  "exposure": 0.0, "contrast": 0, "highlights": 0, "shadows": 0,
  "saturation": 0, "vibrance": 0, "temperature": 0, "tint": 0, "filter": null }
```

| Field | Range | Meaning |
|---|---|---|
| `rot90` | 0..=3 | Quarter turns clockwise, applied after the EXIF orientation |
| `flip_h` | bool | Mirror left-right, after `rot90` |
| `angle` | -45.0..=45.0 degrees | Straighten, after `rot90`/flip; the image is auto-cropped to the largest upright rectangle inside |
| `crop` | fractions, `l<r`, `t<b`, each side ≥ 0.01 | Of the straightened, auto-cropped image |
| `aspect` | `null`, `free`, `original`, `1:1`, `4:3`, `3:2`, `16:9`, `4:5` | UI hint only, never changes pixels |
| `exposure` | -5.0..=5.0 EV | Linear-light gain |
| `contrast`, `highlights`, `shadows`, `saturation`, `vibrance`, `temperature`, `tint` | -100..=100 | Lightroom-like sliders, "approximate" |
| `filter` | `null` or `{"name": "bw"/"warm"/"cool"/"vivid"/"fade"/"sepia", "amount": 0.0..=1.0}` | Look presets |

Every field is optional on input; missing = no change. Decoding goes through a checked
constructor (`#[serde(try_from)]`): out-of-range values, NaN and infinity are refused.

Order of operations (fixed): EXIF orientation → `rot90`/flip → straighten → crop → tone (exposure,
contrast, highlights/shadows) → colour (temperature/tint, saturation/vibrance) → filter.

## 2. The sidecar file

Name: `<photo file name>.xmp` (`IMG_1.jpg` → `IMG_1.jpg.xmp`). Never `IMG_1.xmp`: that name
belongs to Lightroom and collides when a RAW and a JPEG share a base name.

We write three groups of fields into one `rdf:Description`:

| Group | Namespace | Why |
|---|---|---|
| Our own | `starbase` = `https://github.com/DomeShifter/Starbase/ns/photo-edit/1.0/` | The exact edits. **Wins on read.** |
| Camera Raw | `crs` = `http://ns.adobe.com/camera-raw-settings/1.0/` | The shared language other tools understand (approximate) |
| TIFF | `tiff` = `http://ns.adobe.com/tiff/1.0/` | Needed to make sense of the `crs:` crop (see 2.2) |

### 2.1 Field mapping

All numbers are written as plain decimals with a `.` (never rationals like `1/2`: darktable
reads them as `1`, test `x-rational`). Booleans are `True` / `False`.

| Edit field | Sidecar field | Written as | Notes |
|---|---|---|---|
| whole edit | `starbase:Edits` | the edit JSON, compact, ≤ 16 KB | Read first |
| version | `starbase:EditsVersion` | `1` | Unknown version → treat the sidecar as `crs:` only, and never overwrite `starbase:Edits` with a lower version |
| — | `crs:ProcessVersion` | `11.0` | Tells readers the `…2012` scale applies |
| — | `tiff:ImageWidth`, `tiff:ImageLength` | stored pixel width/height (before any orientation) | **Required**: darktable's crop import divides by them; missing → garbage crop (test `x-crop-nowh`: 60×40 instead of 300×200). Immich also reads them as the photo's size, so they must be the real stored size |
| `rot90`, `flip_h` (+ EXIF) | `tiff:Orientation` | 1..8 = EXIF orientation of the file, then `rot90`, then `flip_h` (2.3) | **Always** written, even when it equals the file's own value |
| `crop`, `angle` | `crs:HasCrop`, `crs:CropLeft/Top/Right/Bottom`, `crs:CropAngle` | 6 decimals; see 2.2 | `HasCrop="False"` and no Crop* fields when crop is full and angle is 0 |
| `exposure` | `crs:Exposure2012` | `+1.00` (sign, 2 decimals) | Imported by darktable |
| `contrast` | `crs:Contrast2012` | `+50` (sign, integer) | darktable ignores |
| `highlights` | `crs:Highlights2012` | integer | darktable ignores |
| `shadows` | `crs:Shadows2012` | integer | darktable ignores |
| `saturation` | `crs:Saturation` | integer | darktable ignores |
| `vibrance` | `crs:Vibrance` | integer | darktable ignores |
| `temperature` | `crs:IncrementalTemperature` | integer | Lightroom's relative white-balance slider for non-RAW files; darktable ignores |
| `tint` | `crs:IncrementalTint` | integer | darktable ignores |
| `filter` `bw` with amount ≥ 0.5 | `crs:ConvertToGrayscale` | `True` | The only filter with a `crs:` equivalent; other filters exist only in `starbase:Edits` |

Fields at their default (0, false) are not written (and are removed if a previous save wrote them).

### 2.2 Crop coordinates (the hard part)

`crs:` describes the crop in the **stored** image (pixels as they are in the file, before any
orientation), as fractions 0..1 of the stored width and height. `CropLeft/CropTop` is one corner
of the crop and `CropRight/CropBottom` the opposite corner; the crop rectangle itself is upright
in a frame turned by `CropAngle` degrees about its centre. So with an angle, the four numbers are
two diagonal corners, not an axis-aligned box. darktable's importer works this way, and our
spike reproduces its numbers (test `x-angle`: 0.1/0.1/0.9/0.9 with angle 10 on a 600×400 photo
gives 528×231 px = 480·cos10°+320·sin10° by 320·cos10°−480·sin10°).

Writing (edit → `crs:`), with `W×H` = stored size and `O` = the final `tiff:Orientation`:

1. Compute the crop in pixels of the displayed (oriented, `rot90`/flipped) frame: its centre
   `c`, width `w`, height `h`, using the same straighten + auto-crop maths as the render crate
   (`crop` is a fraction of the auto-cropped rectangle, not of the whole image).
2. Map `c` back to the stored frame with the inverse of `O`. If `O` swaps axes (5..8), swap `w`
   and `h`. If `O` mirrors (2, 4, 5, 7), the stored-frame angle is `-angle`, otherwise `angle`
   (darktable does the same flip of the sign).
3. Corners: `TL = c + R(a)·(−w/2, −h/2)`, `BR = c + R(a)·(+w/2, +h/2)` (R = rotation by the
   stored-frame angle `a`). Write `CropLeft = TL.x/W`, `CropTop = TL.y/H`, `CropRight = BR.x/W`,
   `CropBottom = BR.y/H`, `CropAngle = a`.

Reading (`crs:` → edit, only when there is no valid `starbase:Edits`) is the inverse: centre =
midpoint of the corners, `w = dx·cos a + dy·sin a`, `h = dy·cos a − dx·sin a` (in pixels), then
map into the displayed frame with `O` and express as fractions of the auto-cropped rectangle;
clamp into range. Test: the `angle.xmp` fixture must give 528×231 px ±1.

The sign of the angle: our `angle` and `crs:CropAngle` use the same sign. Whether positive means
clockwise in Lightroom itself is **not verified** (no Lightroom here); darktable stores the
negated value internally. Pinned by the fixture round-trip, not by a Lightroom test.

### 2.3 Orientation (no double rotation)

Rule: `tiff:Orientation` in our sidecar is always the **final** orientation = the file's EXIF
orientation (1 when missing), then `rot90` quarter turns clockwise, then `flip_h`. This is what
Lightroom does when a user rotates: it changes the orientation, never the pixels.

Why it must always be written and always be right (spike results):

- darktable compares the sidecar orientation with the file's: if they differ it adds its own
  rotation (test `x-tiff6`: file orientation 1 + sidecar 6 → output turned 90°). Correct when it
  reflects a user rotation; a double rotation if we wrote the wrong value.
- darktable reads the crop in **stored** coordinates only when the sidecar carries the
  orientation (test `x6-tiff6`: red = stored top-left, correct). Without it the same numbers are
  read in **displayed** coordinates (tests `x6-crop`, `x6-crop-dwh`: blue = wrong corner).
- Immich lets sidecar fields override the file's (`metadata.service.ts`: `{...mediaTags,
  ...sidecarTags}`), so its thumbnails follow our `tiff:Orientation`. Good for `rot90`, and a
  wrongly written value would turn the photo there too.

Our own renderer ignores `tiff:Orientation` from the sidecar; it uses the file's EXIF orientation
plus the edit's `rot90`/`flip_h`.

### 2.4 Merge rules (server)

- **Read**: if `starbase:Edits` parses and `starbase:EditsVersion` = 1 → those are the edits. Else
  if any `crs:` crop/tone field is present → import it approximately (2.1–2.3), `foreign` gets
  `"crs"`. Else identity edits.
- **Write**: parse the existing sidecar, change only the fields we own (table 2.1: `starbase:*`,
  the `crs:` fields listed, `tiff:Orientation`, `tiff:ImageWidth`, `tiff:ImageLength`), keep every
  other field and namespace exactly (darktable history, ratings, tags, GPS, faces). Write to a
  temporary file in the same folder and rename over the old one. No sidecar yet → create one with
  only our fields.
- **Reset** (`DELETE`): remove our fields; if nothing else is left, delete the file.
- Never write `photo.xmp`, never touch the photo file.
- **darktable**: `foreign` gets `"darktable"` when `darktable:history` contains an operation
  other than darktable's automatic defaults for a JPEG (`colorin`, `colorout`, `gamma`, and
  `flip` with `multi_name="auto"`). Reason: darktable rewrites the sidecar as soon as it merely
  imports the photo (test `jx-ours`), adding a history of only those defaults; that is not a
  user edit and must not raise a warning. Any other operation means someone edited the photo in
  darktable: the UI warns before the first save that darktable keeps its own edits and will not
  see ours.

## 3. What other tools do with our sidecar

### darktable 5.0.1

- **`darktable-cli` never applies `crs:` and never changes the sidecar**, whichever name it has,
  passed explicitly or found by itself, with or without `--apply-custom-presets false` (tests
  `jx-crop`, `x-crop`, `x-crop-np`, `jx-crop-arg`: output identical to no sidecar).
- **GUI, our `photo.jpg.xmp`**: darktable treats it as its own sidecar. On import it rewrites the
  file through exiv2, **keeps all our `crs:`, `tiff:` and `starbase:` fields**, and adds
  `darktable:` history plus `xmp:Rating="1"`, `exif:DateTimeOriginal`, `xmpMM:DerivedFrom` and
  the tags `darktable|format|jpg` (`dc:subject`, `lr:hierarchicalSubject`). It applies **none** of
  our edits: the Lightroom import only reads `photo.xmp` (`src/develop/lightroom.c`,
  `dt_get_lightroom_xmp` swaps the extension for `.xmp`/`.XMP`). Fixture
  `after-darktable-import.xmp` is such a rewritten file.
  Side effect: Immich and other tools then see rating 1 and the darktable tags.
- **GUI, a `photo.xmp`** (Lightroom name, no `stEvt:softwareAgent` needed): imported once, on the
  first open in the darkroom: crop + angle and `Exposure2012` only (also, not used by us:
  `Blacks2012`, vignette, grain, clarity, tone curve, per-colour HSL, split toning, spots,
  rating, label, tags, GPS). `photo.xmp` is left untouched; darktable writes its own
  `photo.jpg.xmp`.
- So an "Open in darktable" action that should carry our crop and exposure over must, before
  launching darktable: write a `photo.xmp` copy of our `crs:`+`tiff:` fields, only when no other
  file shares the base name (RAW+JPEG pair) and darktable has not seen the photo before (no
  `darktable:history` in `photo.jpg.xmp`). Not part of v1.
- darktable only re-reads a changed sidecar for a photo already in its library when its "look
  for updated XMP files on startup" setting is on.

### Immich (from official docs and source only, not installed)

- Reads `photo.jpg.xmp` first, `photo.xmp` as a fallback
  ([docs](https://docs.immich.app/features/xmp-sidecars); source
  [`metadata.service.ts`](https://github.com/immich-app/immich/blob/7b27863b13d7ec52340e87637d71a39aa8d7efc0/server/src/services/metadata.service.ts)
  lines 546–551).
- Uses only: description, date, GPS, rating, tags. "All other fields (e.g. … Lightroom edits)
  remain in the `.xmp` file and are not searchable." It does **not** apply `crs:` edits; its own
  editor keeps edits in its database ([editing docs](https://docs.immich.app/features/editing)).
- Sidecar values override the file's (`{ ...mediaTags, ...sidecarTags }`, line 624), including
  `Orientation`, `ImageWidth`, `ImageHeight` (lines 557–570, 367–369): see 2.3.
- Its "Sidecar Write" job writes through exiftool, merging (`exiftool.write`, `writeTags` in
  [`metadata.repository.ts`](https://github.com/immich-app/immich/blob/7b27863b13d7ec52340e87637d71a39aa8d7efc0/server/src/repositories/metadata.repository.ts)).
  exiftool 13.25 keeps our `starbase:` fields on such a write without any namespace
  registration (spike `readers`), so Immich will not erase our edits.

### exiv2 0.28.5 (digiKam's metadata library) and exiftool 13.25

Both read every field, show `starbase:` under its own group (`Xmp.starbase.*`,
`XMP-starbase`) with no configuration, and keep it when they write another field. All
`crs:` crop fields read as reals. digiKam itself was not run; it does not apply `crs:` develop
settings (its own edits are versioned files) — **unverified**, from its documentation.

## 4. Interop results

Spike: `scripts/photo-sidecar-interop.sh`, darktable 5.0.1 / exiftool 13.25 / exiv2 0.28.5 on
Debian stable, 2026-11-25. Test photo 600×400 with four coloured quarters; "red" = stored
top-left quarter.

| Tool × field | Result | Evidence |
|---|---|---|
| darktable-cli × anything in any sidecar | ignored | all `auto`/`nopre`/`xmparg` cases = baseline |
| darktable GUI × our `photo.jpg.xmp` | ignored, file rewritten, our fields kept | `jx-ours` |
| darktable GUI × `photo.xmp` crop | works (needs `tiff:ImageWidth/Length`) | `x-crop` 300×200 red, `x-crop-r` green, `x-crop-nowh` broken |
| darktable GUI × crop + `CropAngle` | works, corner model of 2.2 | `x-angle` 528×231 |
| darktable GUI × crop with EXIF orientation 6 | works only with `tiff:Orientation` in the sidecar | `x6-tiff6` vs `x6-crop` |
| darktable GUI × sidecar orientation ≠ file | rotates (correct for a user rotation) | `x-tiff6` 400×600 |
| darktable GUI × `Exposure2012` | works (approximate) | `x-exp` brighter |
| darktable GUI × Contrast/Highlights/Shadows/Saturation/Vibrance/Temperature/Tint/Grayscale | ignored | `x-tone` = `x-exp` |
| darktable GUI × crop as rationals | ignored | `x-rational` 600×400 |
| Immich × crs edits | ignored (docs + source) | links above |
| Immich × `tiff:Orientation`, `tiff:ImageWidth/Length` | used, overrides the file | source lines 557–624 |
| Immich / exiftool write × `starbase:` | kept | spike `readers` |
| exiv2 (digiKam) × `starbase:` + `crs:` | read and kept | spike `readers` |

Fixtures (`crates/photo-edit/tests/fixtures/sidecars/`), all for a 600×400 stored photo:

| File | What a reader must get |
|---|---|
| `crop-top-left.xmp` | `starbase:Edits` wins: crop l0 t0 r0.5 b0.5 |
| `angle.xmp` | `crs:` only: crop of 528×231 px ±1, angle 10 |
| `rot6-crop-stored-top-left.xmp` | `crs:` only, file has EXIF orientation 6: the stored top-left quarter (displayed top-right), 200×300 px |
| `tone.xmp` | `crs:` only: exposure 1.0, contrast 50, highlights −50, shadows 50, saturation −100, vibrance −100, temperature 40, tint 20, filter bw 1.0 |
| `after-darktable-import.xmp` | real darktable 5.0.1 rewrite of our sidecar: `starbase:Edits` still wins; `foreign` is empty (defaults only) |

## 5. Open questions

1. Positive `crs:CropAngle` = clockwise or anticlockwise in Lightroom itself? Needs a real
   Lightroom sidecar with a known straighten.
2. Lightroom Classic embeds XMP into JPEGs instead of reading `photo.jpg.xmp`; whether it reads
   our sidecar at all is untested.
