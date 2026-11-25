# Changelog

Changes to Starbase as a whole that people running it would notice, newest first. A suite
version (tag `vX.Y.Z`) is one Docker image, `ghcr.io/domeshifter/starbase`, holding Starbase Sheets,
Starbase Docs, Starbase Drive and the file server; the release notes are this file's section for that version.
Details of each app: [Starbase Sheets' changelog](starsheets/CHANGELOG.md).

Versions follow [semantic versioning](https://semver.org): while we're below 1.0, the middle
number goes up for new features and the last one for fixes.

## [Unreleased]

### Changed
- Renamed: the suite is **Starbase**, and its apps are **Starbase Drive** (the home screen),
  **Starbase Sheets** and **Starbase Docs** (were StarDrive, StarSheets and StarDocs). Settings
  are now `STARBASE_*`; the old `STARSHEETS_*` names still work (the new one wins if both are set).

### Added
- One Docker image for the whole suite, `ghcr.io/domeshifter/starbase`: StarSheets and the home
  screen at `/`, StarDocs at `/docs/`, your files as ordinary `.xlsx` and `.docx` in `/data`.
- Install guides for Docker, Proxmox, TrueNAS SCALE and Unraid ([docs/install](docs/install/)).
