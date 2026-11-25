<p align="center">
  <img src="starsheets/app/stardrive.svg" alt="Starbase Drive" width="72" height="72">&nbsp;&nbsp;
  <img src="starsheets/app/favicon.svg" alt="Starbase Sheets" width="72" height="72">&nbsp;&nbsp;
  <img src="stardocs/app/favicon.svg" alt="Starbase Docs" width="72" height="72">
</p>

<h1 align="center">Starbase</h1>

<p align="center">
  <b>A self-hosted, open-source alternative to Google Workspace.</b><br>
  Drive, Sheets and Docs (Slides coming), with your files kept as ordinary <code>.xlsx</code> and
  <code>.docx</code> on your own disk.
</p>

<p align="center">
  <a href="https://github.com/DomeShifter/Starbase/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/DomeShifter/Starbase"></a>
  <a href="https://github.com/DomeShifter/Starbase/actions/workflows/starsheets.yml"><img alt="CI" src="https://github.com/DomeShifter/Starbase/actions/workflows/starsheets.yml/badge.svg"></a>
  <a href="#licence"><img alt="Licence: MIT or Apache-2.0" src="https://img.shields.io/badge/licence-MIT%20%2F%20Apache--2.0-blue"></a>
</p>

> [!NOTE]
> **Early preview (v0.1.0).** Starbase works and we use it every day, but it is young: expect
> rough edges, keep your own backups (the built-in backups help), and please
> [tell us what breaks](https://github.com/DomeShifter/Starbase/issues). Starbase Sheets is the
> most mature part; Starbase Docs and Starbase Drive are newer.

<p align="center">
  <img src="docs/images/drive-home.png" alt="Starbase Drive: folders and files with previews" width="800">
</p>

## Why Starbase

Google Workspace is wonderfully convenient, until you want your files back, want to work offline,
or would rather not hand everything to someone else's servers. Starbase aims for the same
friendly, familiar feel, on a machine you control.

| | |
|---|---|
| **Your files stay yours** | Every spreadsheet is a real `.xlsx` and every document a real `.docx`, in a normal folder. Open them in Excel, Word or LibreOffice any time. No database to export from, no lock-in. |
| **Any disk, any server** | One Docker image runs the whole suite on a home server, a NAS (TrueNAS, Unraid), a Proxmox box or a small cloud machine. |
| **Work together** | Live co-editing in Starbase Sheets: several people in the same spreadsheet, seeing each other's changes as they happen. |
| **Share like you're used to** | Share files and folders with people as editor or viewer, or by link. |
| **Version history** | Go back to any earlier version, name the important ones, restore without losing anything. |
| **Backups built in** | Scheduled, encrypted backups (restic format) with restore from the app. |
| **Sign in your way** | Starbase's own accounts, or single sign-on through your own login service (OIDC: Authelia, Authentik, Keycloak and others). |
| **Desktop too (in progress)** | Native Linux programs that work offline on local files, or connect to your Starbase server. |

## What works today

Starbase **v0.1.0** (November 2026) is the first public release. In one image you get:

**Starbase Drive**, the home screen: folders (with colours), file previews, search, recent,
starred, shared with me and a bin.

**Starbase Sheets**, a spreadsheet that feels like Google Sheets: Sheets-style menus, toolbar and
keyboard shortcuts, formulas, charts, conditional formatting, drop-downs (including multiple
selections and chips), filters and filter views, comments, and live editing with other people.

<p align="center">
  <img src="docs/images/sheets-budget.png" alt="Starbase Sheets: a budget with formulas and a chart" width="800">
</p>

**Starbase Docs**, a word processor that feels like Google Docs: paragraph styles, lists, tables,
pictures (with crop and text wrapping), comments, version history and your own default styles.

<p align="center">
  <img src="docs/images/docs-newsletter.png" alt="Starbase Docs: a newsletter with headings, a list and a table" width="800">
</p>

**Around them:** accounts and sharing, version history for spreadsheets and documents, scheduled
encrypted backups with restore, OIDC sign-in, and install guides for Docker, Proxmox, TrueNAS
SCALE and Unraid.

The full list is in the [changelog](CHANGELOG.md), with more detail for Sheets in
[its own changelog](starsheets/CHANGELOG.md).

## Install

One Docker image runs everything: `ghcr.io/domeshifter/starbase` (Sheets and Drive at `/`, Docs
at `/docs/`, your files in `/data`, backups in `/backups`). On a Linux machine with Docker (as
root, leave out `sudo`):

```sh
mkdir -p ~/starbase/data ~/starbase/backups
sudo chown 1000:1000 ~/starbase/data ~/starbase/backups
docker run -d --name starbase --restart unless-stopped -p 8080:8080 \
  --memory 1536m --memory-swap 1536m \
  -v ~/starbase/data:/data -v ~/starbase/backups:/backups \
  ghcr.io/domeshifter/starbase:latest
docker logs starbase   # shows the setup code for the first (owner's) account
```

Then open `http://<your server>:8080` and create the owner's account with that code.

Step-by-step guides: [Docker and Compose](docs/install/docker.md) ·
[Proxmox](docs/install/proxmox.md) · [TrueNAS SCALE](docs/install/truenas.md) ·
[Unraid](docs/install/unraid.md). Backups, restoring, time zones and rebuilding a lost server are
in the [Docker guide](docs/install/docker.md); single sign-on is in
[docs/oidc.md](starsheets/docs/oidc.md).

**Desktop programs:** not released yet. A Linux AppImage and a Flatpak are built and tested on
every change, and will be attached to a release once they are ready; Windows and Mac come later.

## Roadmap highlights

Here is where Starbase is heading. Nothing below has a date: it lands when it's good enough.

| What | Status |
|---|---|
| **Linux desktop app** (AppImage, Flatpak, then Flathub): work offline on local files, or connect to your server | In progress |
| **Live co-editing in Starbase Docs**, as Sheets already has | Planned |
| **Starbase Slides**, presentations built on DeckCraft, with `.pptx` files | In progress (early groundwork: a `.pptx` round-trip check runs in CI) |
| **Windows**, then **Mac** desktop apps | Planned |
| **Any file in Drive**: PDFs, pictures and everything else, not just spreadsheets and documents | Planned |
| **Photos**, with private face grouping that runs entirely on your own machine and is off unless you turn it on | Planned |
| **"Open in Starbase"** from your computer's file manager | Planned |

### Coming next

The next big step is the **Linux desktop release**: one package with Starbase Drive, Sheets and
Docs in your app menu, opening `.xlsx` and `.docx` files with a double-click, working without a
server, and able to connect to yours when you want to share. After that, Flathub.

## Built on

Starbase stands on the shoulders of the excellent open-source engines by
[storytold](https://github.com/storytold):

| Starbase app | Engine | Folder here |
|---|---|---|
| Starbase Sheets and Starbase Drive | [GridCraft](https://github.com/storytold/gridcraft) | `starsheets/` |
| Starbase Docs | [WordCraft](https://github.com/storytold/wordcraft) | `stardocs/` |
| Starbase Slides (coming) | [DeckCraft](https://github.com/storytold/deckcraft) | `slides/` |

We keep our engine changes on forks (DomeShifter/gridcraft branch `starsheets`, DomeShifter/wordcraft
branch `stardocs`) and offer useful fixes back upstream. The apps were called StarSheets, StarDocs
and StarDrive until November 2026, which is why some folders and programs still use those names.

## Contributing

Bug reports, ideas and pull requests are all welcome. Please read
[CONTRIBUTING.md](starsheets/CONTRIBUTING.md) and the
[code of conduct](starsheets/CODE_OF_CONDUCT.md) first.

Every push builds both apps on GitHub Actions (workflows `StarSheets` and `StarDocs`); a version
tag (`vX.Y.Z`) runs `Release`, which builds the Docker image and drafts the release notes from the
[changelog](CHANGELOG.md). To build Starbase Docs yourself:

```sh
cd stardocs
scripts/fetch-upstream.sh          # WordCraft at the pinned commit + generated Cargo.toml
cd app && trunk build --release    # site in stardocs/dist (needs about 9 GB of memory)
```

## Security and privacy

- Found a security problem? Please report it privately, as described in
  [SECURITY.md](starsheets/SECURITY.md), rather than in a public issue.
- What Starbase stores and sends: [docs/privacy.md](docs/privacy.md).
- How release downloads are signed: [docs/code-signing-policy.md](docs/code-signing-policy.md).

## Licence

Starbase is free and open source, under your choice of the [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE) licence. See [NOTICE](NOTICE) for the engines' licences and credits.
