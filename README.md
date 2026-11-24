# Starbase

An open-source office suite you host yourself: **StarDrive** (files), **StarSheets** (spreadsheets)
and **StarDocs** (documents), familiar to anyone who uses Google Workspace, with your files kept as
ordinary `.xlsx` and `.docx` on your own disk.

Status: early prototype, private while it settles.

| Part | Folder | Engine |
|---|---|---|
| StarDocs | `stardocs/` | [WordCraft](https://github.com/storytold/wordcraft), our fork's `stardocs` branch |
| StarSheets | (moving in from DomeShifter/StarSheets) | [GridCraft](https://github.com/storytold/gridcraft) |

## Building StarDocs (web)
```
cd stardocs
scripts/fetch-upstream.sh          # WordCraft at the pinned commit + generated Cargo.toml
cd app && trunk build --release    # site in stardocs/dist
```
The release build needs about 9 GB of memory.

Licence: MIT OR Apache-2.0 (see NOTICE).
