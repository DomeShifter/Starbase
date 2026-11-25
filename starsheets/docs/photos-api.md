# Photos API (server)

The server keeps an index of the pictures in the folders where Photos is turned on, and makes
two thumbnails of each, in the background. This page is for whoever builds the Photos grid on top
of it. Code: `crates/server/src/photos/`. All routes need a signed-in person (as every `/api/files`
route).

## Turn Photos on for a folder (opt-in)

Nothing is indexed until someone turns Photos on for a folder. That covers the folder and every
folder inside it. The grid's "View as photos" switch calls this the first time.

| Call | Who may | Answer |
|---|---|---|
| `POST /api/photos/enable?folder=PATH` (or `folder_id=ID`) | anyone who may edit the folder | `{"folder", "enabled": true}`; indexing starts |
| `POST /api/photos/disable?folder=PATH` | the folder's owner (or the server's) | `{"folder", "enabled": false}`. The folder's pictures leave the index and their thumbnails are deleted. `enabled` stays `true` when a folder above it still has Photos on. |

## List the pictures in a folder

`GET /api/photos?folder=PATH` or `GET /api/photos?folder_id=ID`

| Parameter | Meaning |
|---|---|
| `folder` | The folder's path inside the storage folder, as `/api/files` lists it (`ann/Holiday 2024`). `""` = the top (only when sign-in is off, or for the owner). |
| `folder_id` | The folder's id from `/api/files` (`folder_ids`), instead of `folder`. |
| `sub=1` | Include every folder inside it too. |
| `sort` | `taken` (default: the EXIF date, or the file's modified time when there is none), `name` or `modified`. |
| `desc=1` | Newest (or Z) first. |

Who may: anyone who may view the folder (their own, or shared with them). Otherwise 403. A folder
that isn't there, or is reached through a symbolic link: 404. With Photos off for the folder, the
answer is `"enabled": false` and no photos. `can_enable` says whether this person may turn it on, and
`can_disable` whether they may turn it off.

Answer:

```json
{
  "folder": "ann/Holiday 2024", "sub": false, "role": "owner", "waiting": 12,
  "enabled": true, "can_enable": true, "can_disable": true,
  "photos": [
    { "id": "3f9a1c2b", "pid": 41, "name": "IMG_0001.jpg", "folder": "ann/Holiday 2024",
      "media": "image/jpeg", "size": 3811234, "modified": 1721000000,
      "state": "ready", "error": null, "v": "9b1e0c4d2a7f3e10",
      "taken": "2024-07-14T18:30:05+01:00", "width": 3000, "height": 4000, "orientation": 6,
      "camera": "Canon EOS R6", "gps": { "lat": 51.51, "lon": -0.12, "alt": null } }
  ]
}
```

| Field | Meaning |
|---|---|
| `id` | The file id used everywhere else (`/api/files/ID`, `/preview`, `/raw`). |
| `pid` | The index's own number for this picture. Stays the same when the file is renamed or moved. Later jobs (faces, tags, albums) will use it. |
| `state` | `pending` (not read yet), `ready`, `failed` (`error` says why), `unsupported` (HEIC, RAW, AVIF, TIFF: listed, no thumbnails yet). |
| `v` | The picture's content tag (only when `ready`). Put it in the thumbnail address. It changes when the picture changes. |
| `taken` | When it was taken (EXIF). Local time as the camera wrote it, `+HH:MM` when the camera saved its time zone. `null` when unknown. |
| `width`, `height` | As shown, with orientation applied (a portrait photo stored sideways is taller than wide). Only when `ready`. |
| `gps` | From EXIF. Given only to people who may edit the folder through their own account or a share that names them, never through a link. Everyone else gets `null`. (Whoever can download the original still gets the location inside it; taking it out for share links is a later job.) |
| `waiting` | How many pictures on the server are still waiting to be read. Ask again later to see more `ready`. |

Opening a folder also rescans it (at most every 5 s; with `sub=1` in the background, at most once a
minute), so new pictures show up without waiting for the regular rescan.

## Get a thumbnail

`GET /api/photos/thumb/ID/SIZE?v=V`

- `SIZE` is `256` (the grid) or `1024` (the viewer). The longest side is at most that; small pictures are never enlarged.
- JPEG (`image/jpeg`), or PNG when the picture has transparency.
- EXIF orientation is already applied.
- Who may: anyone who may view the file (403 otherwise).

| Answer | Meaning |
|---|---|
| 200 | The thumbnail. |
| 202 | Not ready yet (`{"state":"pending","retry":2}`, header `Retry-After: 2`). The server moved this picture to the front of the queue and waited up to 3 s. Ask again. |
| 304 | Unchanged (`If-None-Match` with the `ETag`). |
| 400 | A size other than 256 or 1024. |
| 404 | Not a picture, `{"state":"failed","error":…}`, `{"state":"unsupported"}`, or `{"state":"off"}` (Photos isn't on for its folder). Show the file type's icon. |

Caching: with `?v=` equal to the listing's `v`, the answer is `Cache-Control: private,
max-age=31536000, immutable`, because that address names exactly these bytes. Without it, the answer is `private, no-cache`
with an `ETag`. So always use `v` in the grid. Headers as for previews: `X-Content-Type-Options: nosniff`,
the sandbox `Content-Security-Policy`, `Content-Disposition: inline`.

## Status

`GET /api/photos/status` is for the server's owner only (403 for anyone else; anyone with sign-in off):

```json
{ "queued": 120, "working": 2, "scanning": false, "folders": ["ann/Photos"],
  "pictures": { "pending": 120, "ready": 4810, "failed": 3, "unsupported": 41 },
  "made_since_start": 900, "workers": 2, "child_memory_mb": 1536, "isolation": "process",
  "watching": "rescans only (network storage)", "network_storage": true, "rescan_seconds": 600,
  "cache": { "folder": "/cache/photos", "bytes": 52428800, "cap": 21474836480 },
  "last_rescan": { "started": 1764061200, "seconds": 4.2, "files": 4974 } }
```

## Edits

A picture's edits are kept next to it in an XMP sidecar, `photo.jpg.xmp` (the name darktable,
digiKam and Lightroom use). The picture itself is never changed. The sidecar holds Adobe's
`crs:` fields (so Lightroom and darktable see roughly the same edits), the picture's stored size
and final orientation in `tiff:` fields, and Starbase's own exact copy as JSON in `starbase:Edits`.
The exact mapping (crop corners, orientation) is in docs/photos-edit.md. Anything else in the sidecar (other apps' namespaces, darktable's
history, keywords) is kept as it is. The edits' JSON is the `starbase-photo-edit` crate's `Edits`
(every field optional; unknown fields are ignored; out-of-range values are refused).

`GET /api/photos/edits/ID` (anyone who may view the picture):

```json
{"edits": {"v": 1, "exposure": 0.5, "crop": {"l": 0.1, "t": 0, "r": 0.9, "b": 1}, ...},
 "etag": "3f2a…", "writable": true, "reason": null,
 "foreign": ["darktable"], "source": "starbase", "thumb_v": "9c41…"}
```

- `etag`: `none` when there is no sidecar; send it back in `If-Match` to save.
- Edits saved by a newer Starbase (`starbase:EditsVersion` other than 1) are shown as Adobe's
  fields only and never changed (`writable: false`).
- `writable: false` with a plain-language `reason` when this person can't save here: they may only
  view, the picture's kind can't be edited yet (RAW, HEIC), the folder can't be written on the
  server, or the sidecar is a link or isn't XMP Starbase can read.
- `foreign`: other editors whose edits are in the sidecar: `darktable` (a darktable history with
  steps beyond the automatic ones it adds on import) or `crs` (Adobe Camera Raw fields Starbase
  doesn't write, or Lightroom's edits without ours).
  Starbase keeps their data but doesn't show their edits.
- `source`: `starbase` (our JSON), `crs` (imported from Adobe's fields, approximately) or `none`.
- `thumb_v`: the picture's `v` for thumbnails; it changes whenever the edits change.

Who may save: whoever may edit the picture file, including someone it was shared with on its own
(not its folder): the sidecar is part of the picture, so editing the picture includes it.

`PUT /api/photos/edits/ID` with `If-Match: <etag>` and the edits as the body (JSON, at most 16 KB):
saves them. `If-Match: *` saves over whatever is there, but only when a sidecar exists. Answers `{"etag", "thumb_v"}`. Errors: 428 without `If-Match`; 400 for invalid edits;
413 for a body over 16 KB; 403 for someone who may only view, or a folder that can't be written;
409 with the current `etag` when the sidecar changed since the etag that was sent (someone else saved
first: read again, then save). Edits that change nothing take Starbase's part out of the sidecar.

`DELETE /api/photos/edits/ID` with `If-Match`: resets the picture. Starbase's fields are taken out;
the sidecar is deleted only when nothing else is left in it. The reply's `removed` says whose edits
went: `starbase`, `crs` (edits Lightroom wrote: the app should warn before, since GET said
`source: "crs"`) or `none`.

The sidecar goes with its picture: renaming, moving, copying, deleting (to the bin) and restoring
the picture do the same to `photo.jpg.xmp`. In the bin it isn't an item of its own.

Safety: the sidecar is read only if it is a plain file of at most 256 KB (never through a link), with
at most 256 attributes on an element and 20 000 in all, and parsed without DTDs or entities, outside
the server's lock (a slow file can't hold up anyone else). It is written to a temporary file in the
same folder (keeping the old file's permissions) and renamed over the old one, only if it is still
the sidecar that was read: two saves can't both win, and another app's save in the meantime is a
409, never lost.

## Export ("Save a copy")

`POST /api/photos/export/ID`, body (every field optional, at most 16 KB):

```json
{"format": "jpeg", "quality": 90, "max_side": null, "strip_location": true, "edits": {}}
```

Makes a new file next to the picture with its saved edits (or, when `edits` is sent, with those:
same shape and checks as `PUT` edits, 400 if invalid; the picture's sidecar is never changed, so
"Save a copy" leaves the original as it was): `photo (edited).jpg` (or `.png`),
numbered when that name is taken (`photo (edited) (2).jpg`). It never replaces a file: it is
saved like an upload. Answers 201 with `{"id", "name", "folder", "path"}`. Who may: anyone who may
add files to the picture's folder (403 otherwise; 403 also for a picture they can't see). 400 for
bad settings, 409 for a kind of picture Starbase can't edit yet (RAW, HEIC), 422 with the reason
when the picture can't be exported (unreadable, too big).

- `format`: `jpeg` (transparent parts become white) or `png`.
- `max_side`: the longest side at most this (16 to 30000; never enlarged); `null` = full size.
- `strip_location` (on unless set to `false`): no GPS position in the new file.

The picture is turned upright (its EXIF orientation), rendered with the edits and encoded in the
same locked-down child process as thumbnails, one export at a time. Metadata is rebuilt, not
copied: only camera, lens, exposure settings, dates, artist and copyright from the EXIF, orientation
set to 1, and the GPS position only when `strip_location` is `false`. Nothing else comes along:
no XMP (where phones keep a second copy of the location), no MakerNotes, no embedded thumbnail. A
JPEG keeps the original's colour profile; a PNG has no metadata.

Thumbnails show the edits too: the picture is turned upright first, then rendered with them, in
the same child process and limits. A cropped picture is decoded bigger (up to 4096 pixels), so
its thumbnails stay sharp.

## Settings (environment)

| Setting | Default | Meaning |
|---|---|---|
| `STARBASE_PHOTOS_WORKERS` | 2 | Pictures read at once. Each one runs at low CPU priority and idle disk priority. |
| `STARBASE_PHOTOS_CHILD_MB` | 1536 (at least 512) | The memory limit of each child process that decodes a picture. |
| `STARBASE_PHOTOS_RESCAN` | 600 on a network share, 3600 on a local disk | Seconds between full rescans (at least 30). |
| `STARBASE_PHOTOS_CACHE` | `/cache/photos` when a volume is mounted at `/cache`; else, if the storage folder is on a network share, a local per-user folder (`$XDG_CACHE_HOME` or `~/.cache`, else `/var/cache`, then `starbase/photos/<id>`); else `<storage>/.starsheets/photos-cache` (the log says which and why) | Where the index (`index.sqlite`) and thumbnails go. Always a local disk: SQLite's locking does not work on NAS shares. |
| `STARBASE_PHOTOS_CACHE_MB` | 20480 | The most the thumbnails may take. Past it, the least recently shown ones go first. |

Workers × child memory is kept within half of the server's memory limit (the container's
`memory.max`). The child's memory is lowered first (to 512 MB), then the number of workers (to 1).
The log says what was chosen.

The cache is never inside anyone's folder and is left out of backups. Deleting it is safe: every
picture is read again. The same goes for a damaged index: it is moved aside
(`index.sqlite.broken-<time>`) and rebuilt. Thumbnails of deleted pictures, or of folders where
Photos was turned off, are deleted straight away.

## Photos and NAS storage

SQLite cannot lock files on network shares (CIFS/SMB, NFS, FUSE, 9p), so the index and thumbnails
never go on one. The server checks the storage folder's file system and picks a local folder by
itself (see `STARBASE_PHOTOS_CACHE` above); the log line `Photos: index and thumbnails in ...`
says where and why. In Docker, keep the index across container rebuilds with the `./photos-cache:/cache`
volume that `docker/compose.yml` already has. If the index still can't open, the owner sees
"Photos needs a local disk for its index..." and everyone else sees "Photos isn't set up on this
server yet".

## Finding changes

One scanner thread does all scanning, and it never holds the index's lock for more than 500 files
at a time. On a local disk, inotify watches the folders where Photos is on, without following
symbolic links. On a network share (CIFS, NFS, FUSE) there is no inotify: it can't see other
computers' changes, and it would have to walk the whole tree over the network. Instead there is a
stat-only rescan every `STARBASE_PHOTOS_RESCAN` seconds, plus a rescan of a folder when it is opened.

## How pictures are read safely

The server reads each picture once, in pieces, only to hash it. Then it gives the opened file
itself (as stdin) to a child process (`starsheets-server photo-worker`), which decodes it. Before
reading anything, the child gives up what it doesn't need:

- a memory limit (`STARBASE_PHOTOS_CHILD_MB`), 30 s of CPU, no core files, no files written, at most 8 open files, and no new privileges;
- with Landlock (Linux 5.13 and later), no file system access at all. On older kernels, or where a container's seccomp blocks it, the other limits still apply.

The child is stopped after 60 s. The format is taken from
the file's first bytes, not its name. Only JPEG, PNG, WebP and GIF (first frame) are decoded. Width
and height are checked from the header before decoding: at most 30000 pixels a side, 50 million
pixels (150 million for JPEG, which is decoded scaled down). A crash is one `failed` picture, never
the server.

## Albums and Places

Albums are virtual (one small JSON file per person next to the server's other settings, holding file ids). An album only shows a picture to someone who may see it. Code: `crates/server/src/photos/albums.rs`. All routes need a signed-in person.

| Call | Answer |
|---|---|
| `GET /api/photos/albums` | `{"albums": [{"id", "name", "count", "cover": {"id", "v"}}]}` (`count` = pictures I can see) |
| `POST /api/photos/albums` `{"name", "files"?}` | `{"id", "name", "count"}` |
| `GET /api/photos/albums/ID` | `{"id", "name", "photos": [as the folder listing], "waiting"}`, newest first |
| `POST /api/photos/albums/ID/rename` `{"name"}`, `/add` `{"files"}`, `/remove` `{"files"}`, `/delete` | `{"id", "name", "count"}` or `{"deleted": true}`; the pictures stay where they are |
| `GET /api/photos/places` | `{"places": [{"name", "count", "cover"}]}`: pictures with a GPS position I may know (same rule as `gps` above), grouped by the nearest town of 25,000+ people within 80 km (bundled GeoNames list, CC BY 4.0, no network), else by position rounded to 0.1 degree (`52.1°N, 0.2°E`) |
| `GET /api/photos/places?place=NAME` | `{"place", "photos", "waiting"}` |
