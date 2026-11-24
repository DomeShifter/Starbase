# Starbase

An open-source office suite you host yourself: **StarDrive** (files), **StarSheets** (spreadsheets)
and **StarDocs** (documents), familiar to anyone who uses Google Workspace, with your files kept as
ordinary `.xlsx` and `.docx` on your own disk.

Status: early preview. StarSheets is usable; StarDocs and the home screen are newer and rougher.

| Part | Folder | Engine |
|---|---|---|
| StarDocs | `stardocs/` | [WordCraft](https://github.com/storytold/wordcraft), our fork's `stardocs` branch |
| StarSheets + the home screen (StarDrive) + the file server | `starsheets/` (history from [DomeShifter/StarSheets](https://github.com/DomeShifter/StarSheets), which moved here) | [GridCraft](https://github.com/storytold/gridcraft), our fork's `starsheets` branch |

Every push builds both apps on GitHub Actions (workflows `StarSheets` and `StarDocs`); the built
sites are kept as the `starsheets-web` and `stardocs-web` artifacts. StarSheets' own README
(`starsheets/README.md`) explains running it with Docker.

## Building StarDocs (web)
```
cd stardocs
scripts/fetch-upstream.sh          # WordCraft at the pinned commit + generated Cargo.toml
cd app && trunk build --release    # site in stardocs/dist
```
The release build needs about 9 GB of memory.

Licence: MIT OR Apache-2.0 (see NOTICE).
