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

The AppImage and the tar bundle no system libraries. Desktops already have what they need, but a
bare system or container must add `libxkbcommon-x11-0` (Debian/Ubuntu; the X11 keyboard library),
or the programs don't start under X11. The Flatpak gets it from its runtime.

## Files

| File | What it is |
|---|---|
| `linux/io.github.DomeShifter.Starbase.desktop` | Starbase Drive menu entry. Also the AppImage's root desktop file. |
| `linux/io.github.DomeShifter.Starbase.Sheets.desktop` | Starbase Sheets menu entry, with the spreadsheet MIME types. |
| `linux/io.github.DomeShifter.Starbase.Docs.desktop` | Starbase Docs menu entry, with the document MIME types. |
| `linux/io.github.DomeShifter.Starbase.mime.xml` | shared-mime-info globs for the spreadsheet types, copied from GridCraft. They only matter on minimal systems; document types use the system's definitions, as in WordCraft. |
| `linux/io.github.DomeShifter.Starbase.metainfo.xml.in` | AppStream metadata for the whole package (software centres, Flathub). One component; Flathub allows one launchable, so it names Starbase Drive (the other two entries are still exported). Releases are written out by hand from `CHANGELOG.md`. Screenshots: `linux/screenshots/`, linked by commit. |
| `linux/icons.sh <dest>` | Builds the hicolor icon theme from the apps' own SVGs (nothing generated is committed). It pads non-square icons to a square and renders PNGs from 16 to 512 px with `rsvg-convert` or `resvg` when one of them is installed. |
| `linux/AppRun` | AppImage entry point. It picks the program: no arguments starts the Starbase Drive home; `stardocs …`/`starsheets …` start that program; otherwise the first file's extension decides (Word/ODT/RTF go to Starbase Docs, everything else to Starbase Sheets). A symlink named `stardocs` to the AppImage also works. |
| `linux/appimage.sh` | Builds `starbase-<version>-linux-<arch>.AppImage` (+ `.zsync`) from two built executables: `--starsheets <path> --stardocs <path> [--version v]`. It uses appimagetool 1.9.1 and the type2 runtime 20251108, both pinned by SHA-256 (same versions and sums as WordCraft). The embedded update information is `gh-releases-zsync\|DomeShifter\|Starbase\|latest\|starbase-*-linux-<arch>.AppImage.zsync`. |
| `linux/flatpak/io.github.DomeShifter.Starbase.yml` | Flatpak manifest: org.freedesktop.Platform 26.08 plus the rust-stable SDK extension, and three modules (starsheets, stardocs, shared data). The header explains how to build it. |
| `linux/flatpak/make-repo.sh <bundle> <site> <url> <key> <key.asc>` | Turns the `.flatpak` bundle into our own signed Flatpak repository (a static site): the app on branch `stable`, commit and summary signed with the release key, static deltas, plus `starbase.flatpakref`, `starbase.flatpakrepo`, the public key and an `index.html`. It then checks the result as a client that trusts only that key would. See "Our own Flatpak repository" below. |
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

Flatpak builds run offline, so each Cargo workspace needs its own vendored source list,
`cargo-sources-<workspace>.json`, made by `flatpak-cargo-generator.py` from its Cargo.lock. They
are not committed: `.github/workflows/flatpak.yml` regenerates them on every run (generator pinned
by commit and SHA-256), so they never go stale after a lockfile change. Flathub cannot run the
generator, so a Flathub submission commits them in its own manifest repository. Two helper
scripts in `linux/flatpak/` make the generated files work for our workspaces:

| Script | Why |
|---|---|
| `vendor-config.py` | The generator writes one source replacement per git repository; StarDocs uses craft-libs at five revisions. It rewrites the cargo config with one entry per revision. Run after the generator. |
| `prune-patches.py` | Runs inside the Flatpak build. Our `[patch.crates-io]` blocks name fork crates we never build, which the vendored sources do not contain; it drops those entries (and their `[[patch.unused]]` lockfile blocks). |

The commands for a local build are in the manifest's header.

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
- **CI builds the real Flatpak** (`.github/workflows/flatpak.yml`, artifact `starbase-flatpak`,
  about 31 MB): Flathub's own builder (`org.flatpak.Builder`) on org.freedesktop.Platform 26.08,
  offline from pinned sources. Then, on a clean runner, the bundle is installed from the file
  (runtime from Flathub) and checked: `--version` of both programs, Starbase Drive, a CSV in
  Starbase Sheets and Starbase Docs each staying up for 15 s under Xvfb (software Vulkan/GL from
  the runtime), and the sandbox creating, writing and reading `~/Documents/Starbase` while seeing
  nothing else in `~/Documents`.
- `appstreamcli validate` on the built metainfo passes. `flatpak-builder-lint` (manifest,
  builddir and repo) runs in CI with the screenshots mirrored as Flathub does it, and any lint
  error fails the job. Results are in the job summary.
- Flathub submission status, policy blockers and remaining steps:
  [`linux/flathub-submission.md`](linux/flathub-submission.md).

## Still missing

1. **Mounted AppImage and child windows.** When Starbase Drive opens a file it starts another
   process from inside the AppImage. With FUSE the image is unmounted when the first process
   exits, so a window started from it could lose its files. Not seen yet; check on a desktop.
2. (Done: the Starbase Drive icon is a source of the Flatpak data module.)
3. **Package version.** Starbase has no version of its own: the workspaces are 0.4.0 and 0.18.1.
   `appimage.sh` takes `--version`, `$STARBASE_VERSION` or the latest git tag. The Flatpak
   manifest writes `0.0.0-dev` until the release job substitutes the tag.
4. **Flatpak sources for Flathub.** A `git` source pinned to a release tag and commit in place
   of the `dir` sources, and the generated `cargo-sources-*.json` committed in the Flathub
   repository. The engine commits follow `upstream.pin` (`linux/flatpak/pin-engines.sh`; CI
   warns when the committed manifest is behind). The fonts and word list that
   `starsheets/scripts/fetch-*.py` download are web-only; if a native build ever embeds them they
   become `file` sources (URL + sha256).
5. (Done: metainfo screenshots, the 0.1.0 release notes, branding colours, one launchable.) Each
   new release needs its own `<release>` entry in the metainfo.
6. **Local-mode scope** is James's decision (plan, decision 1). The manifest assumes
   `~/Documents/Starbase`; if he chooses all of Documents, it becomes `--filesystem=xdg-documents`.
7. **Release job (P5):** the AppImage is signed and attached, and the Flatpak goes to our own
   signed repository (above). deb/rpm (nfpm) come later.
8. AppImage desktop integration tools (Gear Lever, AppImageLauncher) install only the root
   (Starbase Drive) entry. The Starbase Sheets and Starbase Docs entries and their file associations work fully
   only in the Flatpak (and the later deb/rpm).

## Our own Flatpak repository

Flathub is not an option for now (`linux/flathub-submission.md`), so Starbase publishes its own
Flatpak repository on GitHub Pages: <https://domeshifter.github.io/Starbase/>. Users install with

```sh
flatpak install --user --from https://domeshifter.github.io/Starbase/starbase.flatpakref
```

and get updates with `flatpak update`; the runtime (org.freedesktop.Platform) comes from Flathub.
`starbase.flatpakrepo` adds just the remote (`flatpak remote-add --user starbase <url>`).

How it is made (`.github/workflows/release.yml`):

| Job | What it does |
|---|---|
| `flatpak` | Calls `flatpak.yml`: the build, Flathub's linter and the smoke test. |
| `flatpak-repo` | Imports the release key into a throwaway keyring (secrets `STARBASE_GPG_PRIVATE_KEY` / `STARBASE_GPG_PASSPHRASE`; the passphrase is preset in gpg-agent, since flatpak cannot ask for it), runs `make-repo.sh` (`flatpak build-commit-from --gpg-sign`, `build-update-repo --gpg-sign --generate-static-deltas`) and uploads the site as a Pages artifact. |
| `flatpak-repo-publish` | `actions/deploy-pages`: on a `vX.Y.Z` tag once every other release job (signing included) is green, or when Release is run by hand with "publish_flatpak_repo" ticked. |
| `flatpak-repo-check` | On a clean runner, installs from the published `.flatpakref` and runs both programs' `--version`. |

Why deploy-pages and not a `gh-pages` branch: an OSTree repository is thousands of binary
objects; committing them to git on every release would grow the repository for ever. Each
release instead **adds to the published repository**: `make-repo.sh` fetches the current version
from Pages (`ostree pull --mirror`, signature checked against our key), commits the new one on
top of it, and keeps the newest two versions with a static delta between them, so `flatpak
update` downloads only what changed. If Pages answers anything but the repository or a 404, the
job stops instead of replacing it. Pull requests never build or publish it.

Limits (GitHub Pages): 1 GB per site, soft 100 GB/month bandwidth, 10-minute deployments. Two
versions of a ~31 MB bundle plus deltas stay far below that. OSTree's object files
(`objects/xx/….commit`, `.dirtree`, `.filez`) are not dotfiles, so the Pages artifact keeps
them; deploy-pages publishes the files as they are (no Jekyll).

One-time setup in the repository settings (an admin does this once):
1. Settings > Pages > Build and deployment > Source: **GitHub Actions**.
2. Settings > Environments > `github-pages` > Deployment branches and tags: keep `main` and
   **add a tag rule `v*`** (otherwise the tag run's deploy is refused).

## Checking a download

Every Linux download on a release is signed with the Starbase release key
(`packaging/linux/starbase-release-signing.asc`, fingerprint
`7704 529F 91C0 B0CE C578  9097 F9BF 4A64 BA9E 1715`):

```sh
gpg --import starbase-release-signing.asc
gpg --verify SHA256SUMS.asc SHA256SUMS      # "Good signature from Starbase release signing"
sha256sum --check --ignore-missing SHA256SUMS
```

Each file also has its own `.asc` signature (`gpg --verify Starbase-x86_64.AppImage.asc`).
