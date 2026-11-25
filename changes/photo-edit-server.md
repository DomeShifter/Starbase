### Added
- Server: picture edits are saved next to each picture in an XMP sidecar (`photo.jpg.xmp`), which
  darktable and Lightroom can read too; other apps' data in the sidecar is kept. New endpoints
  `GET/PUT/DELETE /api/photos/edits/ID` with etags, so two people saving at once can't overwrite
  each other (docs/photos-api.md, "Edits"). Thumbnails follow the edits.
- Server: a picture's sidecar now goes with it when it is renamed, moved, copied, deleted to the
  bin and restored, so edits are never lost and never passed to a later file of the same name.

### Fixed
- Server: hostile sidecars (very deep nesting, thousands of namespace declarations) are refused
  before parsing, instead of crashing or stalling the server.
- Server: moving, copying or restoring a picture never overwrites another file already named like
  its sidecar; a half-done move is undone instead of leaving the picture and its edits apart.
