# Flathub submission: status and remaining steps

Where Starbase stands against Flathub's rules, checked on 2026-11-25 against
[Requirements](https://docs.flathub.org/docs/for-app-authors/requirements),
[Submission](https://docs.flathub.org/docs/for-app-authors/submission),
[MetaInfo guidelines](https://docs.flathub.org/docs/for-app-authors/metainfo-guidelines) and
[Quality guidelines](https://docs.flathub.org/docs/for-app-authors/metainfo-guidelines/quality-guidelines).
Nothing has been submitted.

## Read this first: two policy blockers

| Blocker | What Flathub says | What it means for us |
|---|---|---|
| **Generative AI policy** | Submitters must disclose AI-generated code, documentation and packaging, with the parts and rough extent. "Flathub manifests must not contain AI-generated or AI-assisted content", and disclosure does not exempt them. AI tools must not open or automate submission PRs, or write their descriptions, commit messages, review comments or replies. Reviewers may reject on the extent of generated material. | Much of Starbase, including `packaging/`, was written with an AI assistant. So: the manifest in the Flathub PR must be written by a person (this repository's manifest can be a reference, not a copy); the PR text, reviewer note and every reply must be written by a person; the PR must disclose AI use. That is why this file holds facts to write from, **not** a ready-made PR text. Rejection is possible even with all of that done. |
| **Development history** | "Applications that have only existed for a very short period of time will generally not be accepted." They look for sustained commit history, real-world use and tagged releases. | The repository started on 2026-11-21 and v0.1.0 was tagged on 2026-11-25. Waiting for a few more tagged releases and some months of history makes acceptance far more likely. |

Recommendation: keep the Flatpak building in CI (it already does), publish the `.flatpak` bundle
with GitHub releases meanwhile, and submit later, with the manifest and PR written by hand.

## Requirement checklist

| Requirement | Status |
|---|---|
| App ID `io.github.DomeShifter.Starbase` (4 parts, lowercase domain, points at the repo) | OK |
| Runtime on Flathub, current: org.freedesktop.Platform 26.08 + rust-stable extension | OK; re-check the newest version on the day of submission |
| No network at build time; crates from generated `cargo-sources-*.json` | OK in CI (generated from the lockfiles on every run). For Flathub they are generated once per release and committed in the Flathub repo |
| Engines (GridCraft, WordCraft forks) as `git` sources pinned to commits | OK |
| App source as a `git` source with `tag` + `commit` | **To do, by hand** in the Flathub manifest (this repo's manifest uses `dir` sources for CI). It must be a tag that contains the new metainfo: **not v0.1.0** (see "Release to submit" below) |
| Licence files installed to `/app/share/licenses/<app id>` | OK (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`); metainfo says `MIT OR Apache-2.0` |
| Desktop files, icons (SVG + PNG up to 512 px) | OK; `desktop-file-validate` clean |
| MetaInfo: `developer` with id, one `launchable`, homepage/bugtracker/help/vcs-browser/contribute URLs, OARS, releases with notes, branding colours | OK (this branch) |
| Screenshots, linked by commit, with captions | OK (this branch): Drive, Sheets, Docs, light theme, made-up content |
| `flatpak-builder-lint` manifest / builddir / repo | Runs in `.github/workflows/flatpak.yml` with screenshots mirrored as Flathub does; any error fails the job |
| Architectures: Flathub builds x86_64 and aarch64 | **Open.** CI builds x86_64 only. Either add an aarch64 build to CI, or add `flathub.json` with `{"only-arches": ["x86_64"]}` to the Flathub repo |
| English UI, no web wrapper, works without a special desktop | OK |

## Quality notes (recommended, not required)

| Guideline | Status |
|---|---|
| Name under 15 characters | "Starbase" |
| Summary 10 to 35 characters, no full stop | "Edit spreadsheets and documents" (31) |
| Screenshots: window capture on Linux with title bar and shadow, 1000x700 or smaller | Ours are Linux browser captures of the web app at 1200x675 (1.5x), without window decorations. Better: capture the desktop windows (Flatpak build) on a GNOME desktop with the window-capture tool, then replace the three files and re-pin the links |
| Brand colours: light and dark, colourful, contrast with the icon | Light `#9ec5f8`, dark `#1e3f73` (blue, against the orange Drive icon) |
| Release notes for every release | 0.1.0 written; add one `<release>` per new release, newest first |

## Facts for the reviewer note (to be written in your own words)

Starbase is an office suite: **Starbase Drive** (a home screen listing files and folders),
**Starbase Sheets** (spreadsheets) and **Starbase Docs** (documents). It ships as one Flatpak with
two programs (`starsheets`, `stardocs`).

**Why three desktop files.** Each app has its own menu entry and file associations, like
LibreOffice's: `io.github.DomeShifter.Starbase.desktop` (Drive, `starsheets --home`, the metainfo
launchable), `….Starbase.Sheets.desktop` (spreadsheet MIME types) and `….Starbase.Docs.desktop`
(document MIME types). All names start with the app ID, so Flatpak exports them. A fourth,
hidden desktop file for the `starbase-open` handler is planned (marked spot in the manifest).

| Permission | Why |
|---|---|
| `--socket=wayland`, `--socket=fallback-x11` | Display; X11 only when no Wayland compositor runs |
| `--share=ipc` | X11 shared memory, needed by the X11 fallback |
| `--device=dri` | GPU rendering (wgpu: Vulkan, or GL as a fallback) |
| `--share=network` | "Connect to a server": the user's own Starbase server, for shared files and live editing. No telemetry, no update checks |
| `--filesystem=xdg-documents/Starbase:create` | Local mode works in one folder, `~/Documents/Starbase`. Every other file goes through the file-chooser and document portals |
| Not requested | Home or host filesystem, `org.freedesktop.Flatpak`, printing (until the apps can print). The server sign-in token goes through the Secret portal |

CI checks the filesystem scope: the sandbox can create and write `~/Documents/Starbase` and sees
nothing else in `~/Documents` (smoke job in `flatpak.yml`).

**How Starbase differs from GridCraft and WordCraft** (reviewers may ask, since the engines are
forks): it combines both into one suite with a shared home screen, a self-hosted server for
sharing and live editing, version history and sign-in; changes to the engines are kept on the
forks' `starsheets` and `stardocs` branches, pinned by commit.

**AI disclosure**: list which parts were written with an AI assistant (application code,
documentation, packaging) and roughly how much, as the policy asks.

## Release to submit

v0.1.0 (commit `f88ea7314be886259f3b3f35f10150ce5e56d0e4`) does **not** contain the Flathub
metainfo (screenshots, one launchable, release notes). Tag a new release after this branch is
merged (for example v0.1.1, with its own `<release>` entry in the metainfo and `CHANGELOG.md`),
and use that tag and its commit in the Flathub manifest.

The screenshot links point at the commit that added the images. Merge this branch with a merge
commit (not squash) so that commit stays in `main`'s history, or re-pin the links to the release
tag once it exists.

## Remaining steps

1. Decide whether to submit now, given the two blockers above (recommended: wait).
2. Optional: replace the screenshots with desktop-window captures (see Quality notes).
3. Merge this branch; add the `starbase-open` files at the marked spot in the manifest when that
   work lands; tag the release to submit; check the Flatpak CI run on the tag is green
   (lint included).
4. Decide on aarch64: add a CI build, or plan an `only-arches` `flathub.json`.
5. By hand: write the Flathub manifest (git source with the tag and commit, the engine `git`
   sources, the two modules and the data module), generate `cargo-sources-starsheets.json` and
   `cargo-sources-stardocs.json` for that tag with `flatpak-cargo-generator.py` and
   `vendor-config.py` (commands in the manifest header), build it with `org.flatpak.Builder` and
   run `flatpak-builder-lint` on the manifest and the repo.
6. By hand: fork `flathub/flathub` (all branches), branch from `new-pr`, add the manifest and the
   two JSON files, open the PR against **`new-pr`** titled `Add io.github.DomeShifter.Starbase`,
   keep the PR template, write the reviewer note and AI disclosure yourself, and answer reviews
   yourself. After review, comment `bot, build` to start a test build.
7. After acceptance: verify the app on Flathub (io.github IDs are verified through GitHub).
