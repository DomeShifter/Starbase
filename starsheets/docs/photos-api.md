# Photos API (server)

The server keeps an index of every picture in the storage folder and makes two thumbnails of
each, in the background. This page is for whoever builds the Photos grid on top of it.
Code: `crates/server/src/photos/`. All routes need a signed-in person (as every `/api/files` route)
and answer `GET` only.

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
that isn't there, or is reached through a symbolic link: 404.

Answer:

```json
{
  "folder": "ann/Holiday 2024", "sub": false, "role": "owner", "waiting": 12,
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
| `gps` | From EXIF, or `null`. Anyone who may view the picture can also download it with this location in it. Taking the location out for share links is a later job. |
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
| 404 | Not a picture, `{"state":"failed","error":…}`, or `{"state":"unsupported"}`. Show the file type's icon. |

Caching: with `?v=` equal to the listing's `v`, the answer is `Cache-Control: private,
max-age=31536000, immutable`, because that address names exactly these bytes. Without it, the answer is `private, no-cache`
with an `ETag`. So always use `v` in the grid. Headers as for previews: `X-Content-Type-Options: nosniff`,
the sandbox `Content-Security-Policy`, `Content-Disposition: inline`.

## Status

`GET /api/photos/status` gives `{ "queued": 120, "working": 2, "scanning": false }`. For the owner (or with
sign-in off), it also gives counts for the whole server:

```json
{ "pictures": { "pending": 120, "ready": 4810, "failed": 3, "unsupported": 41 },
  "made_since_start": 900, "workers": 2, "isolation": "process", "watching": "inotify",
  "network_storage": true, "rescan_seconds": 600,
  "last_rescan": { "started": 1764061200, "seconds": 4.2, "files": 4974 } }
```

## Settings (environment)

| Setting | Default | Meaning |
|---|---|---|
| `STARBASE_PHOTOS_WORKERS` | a quarter of the CPU cores, 1 to 4 | Pictures read at once. Each one runs at low CPU priority and idle disk priority. |
| `STARBASE_PHOTOS_RESCAN` | 600 on a network share, 3600 on a local disk | Seconds between full rescans (at least 30). |
| `STARBASE_PHOTOS_CACHE` | `<storage>/.starsheets/photos-cache` | Where the index (`index.sqlite`) and thumbnails go. Set it to a local disk when the storage folder is on a NAS share. |

The cache is never inside anyone's folder and is left out of backups. Deleting it is safe: every
picture is read again.

## How pictures are read safely

Each picture is read by a child process (`starsheets-server photo-worker`). It has a memory limit
(1.5 GB address space), a CPU-time limit (30 s), and is stopped after 60 s. The format is taken from
the file's first bytes, not its name. Only JPEG, PNG, WebP and GIF (first frame) are decoded. Width
and height are checked from the header before decoding: at most 30000 pixels a side, 50 million
pixels (150 million for JPEG, which is decoded scaled down). A crash is one `failed` picture, never
the server.
