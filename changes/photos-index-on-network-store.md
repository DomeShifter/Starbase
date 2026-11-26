### Fixed
- Photos now works when the storage folder is on a network share (NAS): the index and thumbnails go to a local folder by themselves, and the Docker compose file has a `./photos-cache:/cache` volume. If the index still cannot open, the message says why.
