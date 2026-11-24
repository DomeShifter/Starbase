# Starbase

An open-source office suite you host yourself: **StarDrive** (files), **StarSheets** (spreadsheets)
and **StarDocs** (documents), familiar to anyone who uses Google Workspace, with your files kept as
ordinary `.xlsx` and `.docx` on your own disk.

Status: early preview. StarSheets is usable; StarDocs and the home screen are newer and rougher.

| Part | Folder | Engine |
|---|---|---|
| StarDocs | `stardocs/` | [WordCraft](https://github.com/storytold/wordcraft), our fork's `stardocs` branch |
| StarSheets + the home screen (StarDrive) + the file server | `starsheets/` (history from [DomeShifter/StarSheets](https://github.com/DomeShifter/StarSheets), which moved here) | [GridCraft](https://github.com/storytold/gridcraft), our fork's `starsheets` branch |

## Install

One Docker image runs the whole suite: `ghcr.io/domeshifter/starbase` (StarSheets and the home
screen at `/`, StarDocs at `/docs/`, your files in `/data`).

```sh
docker run -d --name starbase --restart unless-stopped -p 8080:8080 -v starbase:/data ghcr.io/domeshifter/starbase:latest
docker logs starbase   # the setup code for the first (owner's) account
```

Step-by-step guides: [Docker and Compose](docs/install/docker.md) ·
[Proxmox](docs/install/proxmox.md) · [TrueNAS SCALE](docs/install/truenas.md) ·
[Unraid](docs/install/unraid.md). Changes: [CHANGELOG.md](CHANGELOG.md).

## Building

Every push builds both apps on GitHub Actions (workflows `StarSheets` and `StarDocs`); the built
sites are kept as the `starsheets-web` and `stardocs-web` artifacts. A suite version tag
(`vX.Y.Z`) runs the `Release` workflow: both apps are rebuilt and tested, put in one image
(`docker/Dockerfile`), pushed to `ghcr.io/domeshifter/starbase` and a draft release is made from
[CHANGELOG.md](CHANGELOG.md). Running `Release` by hand builds the image without publishing it.

## Building StarDocs (web)
```
cd stardocs
scripts/fetch-upstream.sh          # WordCraft at the pinned commit + generated Cargo.toml
cd app && trunk build --release    # site in stardocs/dist
```
The release build needs about 9 GB of memory.

Licence: MIT OR Apache-2.0 (see NOTICE).
