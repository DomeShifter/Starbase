### Added
- Server: "Save a copy" of a picture with its edits (`POST /api/photos/export/ID`): a new file
  next to it, JPEG or PNG, optionally smaller. Location data is left out unless asked for; other
  metadata is rebuilt from a short list (camera, exposure, dates), never copied.
- Server: thumbnails show a picture's edits.
