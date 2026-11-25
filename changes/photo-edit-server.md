### Added
- Server: picture edits are saved next to each picture in an XMP sidecar (`photo.jpg.xmp`), which
  darktable and Lightroom can read too; other apps' data in the sidecar is kept. New endpoints
  `GET/PUT/DELETE /api/photos/edits/ID` with etags, so two people saving at once can't overwrite
  each other (docs/photos-api.md, "Edits"). Thumbnails follow the edits.
