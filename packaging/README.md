# Packaging

Desktop packaging for Starbase: one package, two programs, three menu entries
(docs/research/desktop-plan.md, Phase 2). Linux comes first, as an AppImage and a Flatpak;
deb/rpm, Windows and macOS come later.

| Menu entry | App ID (desktop file, icon) | Runs | Opens |
|---|---|---|---|
| Starbase Drive (the home screen) | `io.github.DomeShifter.Starbase` | `starsheets --home` | nothing by double-click |
| Starbase Sheets | `io.github.DomeShifter.Starbase.Sheets` | `starsheets %F` | .xlsx .xlsm .xlsb .ods .csv .tsv |
| Starbase Docs | `io.github.DomeShifter.Starbase.Docs` | `stardocs %F` | .docx .docm .dotx .dotm .doc .odt .rtf |

The package's ID is `io.github.DomeShifter.Starbase`. Flathub requires `io.github.<user>.<repo>`
for apps hosted on GitHub, with at least 4 components, a lowercase domain part, and a repository
that the ID points to (github.com/DomeShifter/Starbase). Flatpak exports any desktop file and
icon whose name starts with the app ID, which is why the other two entries are
`….Starbase.Sheets` and `….Starbase.Docs`.

## Files

| File | What it is |
|---|---|
| `linux/io.github.DomeShifter.Starbase.desktop` | Starbase Drive menu entry. Also the AppImage's root desktop file. |
| `linux/io.github.DomeShifter.Starbase.Sheets.desktop` | Starbase Sheets menu entry, with the spreadsheet MIME types. |
| `linux/io.github.DomeShifter.Starbase.Docs.desktop` | Starbase Docs menu entry, with the document MIME types. |
| `linux/io.github.DomeShifter.Starbase.mime.xml` | shared-mime-info globs for the spreadsheet types, copied from GridCraft. They only matter on minimal systems; document types use the system's definitions, as in WordCraft. |
| `linux/io.github.DomeShifter.Starbase.metainfo.xml.in` | AppStream metadata for the whole package (software centres, Flathub). One component that lists all three menu entries as launchables. The scripts substitute `@VERSION@` and `@DATE@`. |
| `linux/icons.sh <dest>` | Builds the hicolor icon theme from the apps' own SVGs (nothing generated is committed). It pads non-square icons to a square and renders PNGs from 16 to 512 px with `rsvg-convert` or `resvg` when one of them is installed. |
| `linux/AppRun` | AppImage entry point. It picks the program: no arguments starts the Starbase Drive home; `stardocs …`/`starsheets …` start that program; otherwise the first file's extension decides (Word/ODT/RTF go to Starbase Docs, everything else to Starbase Sheets). A symlink named `stardocs` to the AppImage also works. |
| `linux/appimage.sh` | Builds `starbase-<version>-linux-<arch>.AppImage` (+ `.zsync`) from two built executables: `--starsheets <path> --stardocs <path> [--version v]`. It uses appimagetool 1.9.1 and the type2 runtime 20251108, both pinned by SHA-256 (same versions and sums as WordCraft). The embedded update information is `gh-releases-zsync\|DomeShifter\|Starbase\|latest\|starbase-*-linux-<arch>.AppImage.zsync`. |
| `linux/flatpak/io.github.DomeShifter.Starbase.yml` | Flatpak manifest: org.freedesktop.Platform 26.08 plus the rust-stable SDK extension, and three modules (starsheets, stardocs, shared data). The header explains how to build it. |
| `linux/flatpak/pin-engines.sh [--check]` | Copies the engine commits from `starsheets/upstream.pin` and `stardocs/upstream.pin` into the manifest. CI runs it before each Flatpak build. |

Adapted from WordCraft's `packaging/linux` (storytold/wordcraft v0.18.1) and GridCraft's
(storytold/gridcraft v0.19.1), both MIT OR Apache-2.0.

### Flatpak permissions

`wayland` with an X11 fallback, `ipc`, `dri` (GPU), `network` (for "Connect to a server"), and
`xdg-documents/Starbase:create`, so local mode can work in `~/Documents/Starbase`. Files
elsewhere go through the file-chooser and document portals. Not granted: home or host access,
WordCraft's updater rights (Flathub does the updating), and printing (upstream's cups-client
module), until the native apps can print.

### Cargo sources for the Flatpak

Flatpak builds run offline, so each Cargo workspace needs its own vendored source list. Generate
them with `flatpak-cargo-generator.py`, once per lockfile:

```
python3 flatpak-cargo-generator.py starsheets/Cargo.lock -o packaging/linux/flatpak/cargo-sources-starsheets.json
python3 flatpak-cargo-generator.py stardocs/Cargo.lock   -o packaging/linux/flatpak/cargo-sources-stardocs.json
```

## Checked so far

- `desktop-file-validate`: all three desktop files are clean.
- `appstreamcli validate --pedantic`: passes. The one pedantic note is about the uppercase
  letters in the ID (`cid-contains-uppercase-letter`), which Flathub allows outside the domain.
- `appimage.sh` with stub executables built a working AppImage. The dispatcher sent each case to
  the right program, the update information was embedded, and the icons were staged.
- **CI builds the real AppImage** (`.github/workflows/desktop.yml`, artifact `starbase-appimage`):
  both programs built on ubuntu-22.04 (glibc 2.35), packed with `appimage.sh`, then smoke-tested
  on ubuntu-22.04 and the newest Ubuntu without FUSE (`APPIMAGE_EXTRACT_AND_RUN=1`): `--version`,
  the dispatcher, no missing libraries, and Starbase Drive, a CSV in Starbase Sheets and Starbase
  Docs each staying up for 15 s under Xvfb. Version: `X.Y.Z` from a `vX.Y.Z` tag, otherwise
  `0.0.0-dev+<short commit>`. Not signed yet (marked placeholder step).
- The Flatpak manifest has **not** been built: there are no native executables yet, and no
  `flatpak-builder` was available.

## Still missing

1. **Mounted AppImage and child windows.** When Starbase Drive opens a file it starts another
   process from inside the AppImage. With FUSE the image is unmounted when the first process
   exits, so a window started from it could lose its files. Not seen yet; check on a desktop.
2. **Starbase Drive icon in the Flatpak.** `starsheets/app/stardrive.svg` is merged and
   `icons.sh` uses it; add it as a source in the Flatpak data module.
3. **Package version.** Starbase has no version of its own: the workspaces are 0.4.0 and 0.18.1.
   `appimage.sh` takes `--version`, `$STARBASE_VERSION` or the latest git tag. The Flatpak
   manifest writes `0.0.0-dev` until the release job substitutes the tag.
4. **Flatpak sources.** The generated `cargo-sources-*.json` files, `file` sources (URL +
   sha256) for the fonts and word list that `starsheets/scripts/fetch-*.py` download if the
   native build embeds them, and, for Flathub, `git` sources pinned to a release in place of
   the `dir` sources. The engine fork commits in the manifest must be kept equal to each
   `upstream.pin`.
5. **Metainfo screenshots** (Flathub requires them), a release-notes entry per release, and a
   paragraph for Flathub reviewers on how Starbase differs from GridCraft and WordCraft.
6. **Local-mode scope** is James's decision (plan, decision 1). The manifest assumes
   `~/Documents/Starbase`; if he chooses all of Documents, it becomes `--filesystem=xdg-documents`.
7. **Release job (P5):** build both executables on ubuntu-22.04 / ubuntu-22.04-arm, run
   `appimage.sh`, build the `.flatpak` bundle, then publish SHA256SUMS and a GPG signature.
   deb/rpm (nfpm) come later.
8. AppImage desktop integration tools (Gear Lever, AppImageLauncher) install only the root
   (Starbase Drive) entry. The Starbase Sheets and Starbase Docs entries and their file associations work fully
   only in the Flatpak (and the later deb/rpm).
