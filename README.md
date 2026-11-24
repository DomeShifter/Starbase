# Starbase

An open-source office suite you host yourself: **Starbase Drive** (files), **Starbase Sheets**
(spreadsheets) and **Starbase Docs** (documents), familiar to anyone who uses Google Workspace,
with your files kept as ordinary `.xlsx` and `.docx` on your own disk.

Status: early preview. Starbase Sheets is usable; Starbase Docs and Starbase Drive are newer and
rougher. (Parts of the code, the sign-in screen and the settings still use the older names StarSheets,
StarDocs and StarDrive.)

| Part | Folder | Engine |
|---|---|---|
| StarDocs | `stardocs/` | [WordCraft](https://github.com/storytold/wordcraft), our fork's `stardocs` branch |
| StarSheets + the home screen (StarDrive) + the file server | `starsheets/` (history from [DomeShifter/StarSheets](https://github.com/DomeShifter/StarSheets), which moved here) | [GridCraft](https://github.com/storytold/gridcraft), our fork's `starsheets` branch |

## Install

One Docker image runs the whole suite: `ghcr.io/domeshifter/starbase` (Starbase Sheets and Drive
at `/`, Starbase Docs at `/docs/`, your files in `/data`, backups in `/backups`). On a Linux
machine with Docker (as root, leave out `sudo`):

```sh
mkdir -p ~/starbase/data ~/starbase/backups
sudo chown 1000:1000 ~/starbase/data ~/starbase/backups
docker run -d --name starbase --restart unless-stopped -p 8080:8080 \
  --memory 1536m --memory-swap 1536m \
  -v ~/starbase/data:/data -v ~/starbase/backups:/backups \
  ghcr.io/domeshifter/starbase:latest
docker logs starbase   # the setup code for the first (owner's) account
```

Then open `http://<your server>:8080`. Backups, restoring, time zone, rebuilding a lost server
and signing in with your own login service are in the [Docker guide](docs/install/docker.md).

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
