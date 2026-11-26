# Shared test kit

Deterministic test data plus a seeder, so testers stop re-inventing both. Needs Python 3 and Pillow;
no server address is stored here.

    python3 -I testing/seed.py --url $URL --user USER --password PASS --set gps

Sets: `dates` (24 photos over 18 months), `gps` (20 photos, 6 countries, 3 in one city), `shapes`
(portrait, landscape, 3:1 pano, EXIF orientation 6), `types` (png/gif/webp, mp4 if ffmpeg), `docs`
(2-page .docx). `--folder NAME` picks the folder (default `seed-<set>`); `--url/--user/--password` can
come from `STARBASE_URL/STARBASE_USER/STARBASE_PASSWORD`.

The seeder signs in, creates the folder, uploads with `as_is=1` (without it the server renames
non-spreadsheets to `*.xlsx`), enables Photos and waits until every file is indexed. A picture
whose state is `pending` is not indexed yet, not a bug; the seeder fails naming files that are
still pending after `--timeout` (default 120 s).

Use the generators directly: `sys.path.insert(0, "testing"); from fixtures import photos, docs`.
