# Changelog

Changes to Starbase as a whole that people running it would notice, newest first. A suite
version (tag `vX.Y.Z`) is one Docker image, `ghcr.io/domeshifter/starbase`, holding Starbase Sheets,
Starbase Docs, Starbase Drive and the file server; the release notes are this file's section for that version.
Details of each app: [Starbase Sheets' changelog](starsheets/CHANGELOG.md).

Versions follow [semantic versioning](https://semver.org): while we're below 1.0, the middle
number goes up for new features and the last one for fixes.

## [Unreleased]

### Added
- Linux downloads on a release are signed with the Starbase release key: `SHA256SUMS`, `SHA256SUMS.asc` and a `.asc` per file (key: `packaging/linux/starbase-release-signing.asc`; how to check: `packaging/README.md`). The AppImage is now attached to the release too.
- Starbase Docs: selecting a picture swaps the toolbar's text tools for the picture's, as in
  Google Docs: crop, image options, replace, reset, and the five ways of wrapping text.
- Starbase Docs: **my default styles**, as in Google Docs (Format › Paragraph styles › Options,
  and Options in the style box): save this document's paragraph styles as your defaults, use
  them in another document, or reset to Docs' own. New documents start with them. They are
  kept in your browser (in the desktop program, until it closes).
- Starbase Docs: **File › Make a copy**: "Copy of <name>" beside the document (or in your own
  folder when you can't add files to its folder), opened in a new tab; in the desktop program,
  beside the file on this computer, or the server's copy.
- Sharing: a **Commenter** role, for people and for "anyone with the link", as in Google Docs:
  they can read a document, add comments and replies, and change, resolve or delete their own
  comments, but not change the document. The server never keeps what a commenter's program sends:
  it writes only their comment changes into its own copy of the document. In Starbase Sheets a
  commenter is a viewer for now.
- Starbase Docs: File › Download in every format (Word, OpenDocument, Rich Text, PDF, plain
  text, web page, Markdown), with a Save dialog in the desktop program.

### Fixed
- Starbase Sheets on the desktop: a save to the server that failed is tried again by itself, even
  while the window is in the background (2 s later, then waiting longer each time, at most 30 s),
  and the save status shows the result straight away. It used to wait until the window was
  raised again.
- Starbase Docs: the desktop program's saving and server messages beside the document's name
  showed their tooltip twice.

## [0.1.0] - 2026-11-25

The first public release: an **early preview**. It works and we use it every day, but expect rough
edges; keep your own backups (Starbase's built-in backups help) and please report what breaks.

Highlights: Google-Sheets-style spreadsheets, Google-Docs-style documents and a Drive-style home
screen in one Docker image; live editing for spreadsheets; sharing; version history; scheduled
encrypted backups (restic format) with restore; sign-in with your own login service (OIDC:
Authelia, Authentik, Keycloak); install guides for Docker, Proxmox, TrueNAS and Unraid.

### Changed
- Renamed: the suite is **Starbase**, and its apps are **Starbase Drive** (the home screen),
  **Starbase Sheets** and **Starbase Docs** (were StarDrive, StarSheets and StarDocs). Settings
  are now `STARBASE_*`; the old `STARSHEETS_*` names still work (the new one wins if both are set).

### Added
- Starbase Docs: **version history** like Google Docs (File › Version history, Ctrl+Alt+Shift+H):
  earlier versions with who saved them, named versions, show one read-only, restore it (a new
  version, nothing deleted) or make a copy. Editors only. The server now keeps every version from
  the last day, one an hour for a week, one a day after that, up to `KEEP` (now 100 by default);
  named versions are always kept. Starbase Sheets uses the same history.
- One Docker image for the whole suite, `ghcr.io/domeshifter/starbase`: StarSheets and the home
  screen at `/`, StarDocs at `/docs/`, your files as ordinary `.xlsx` and `.docx` in `/data`.
- Install guides for Docker, Proxmox, TrueNAS SCALE and Unraid ([docs/install](docs/install/)).
