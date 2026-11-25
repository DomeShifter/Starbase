# Contributing to Starbase

Thank you for being here. How to build and change each app: [Starbase Sheets](starsheets/CONTRIBUTING.md),
Starbase Docs and Starbase Slides (the "Building" sections of [README.md](README.md)). This page has
the rules for the whole repository.

## Changelog

Don't edit [CHANGELOG.md](CHANGELOG.md) in a pull request. Add one file instead:
`changes/<branch-or-topic>.md`, with your entries under a heading, written for someone using Starbase:

```markdown
### Added
- Starbase Docs: tables can be sorted by a column (Table › Sort).

### Fixed
- Starbase Sheets: pasting a column of dates no longer shifts them by a day.
```

- Headings: `### Added`, `### Changed`, `### Fixed` (also `### Removed`, `### Security`). Each entry
  starts with `- `; longer entries continue on indented lines.
- One file per pull request (or per topic), named after the branch or topic. Nothing that only
  changes how Starbase is built or tested needs an entry.
- Why: two pull requests that both add a line to CHANGELOG.md conflict with each other; separate
  files never do.

At release time the fragments become the new version's section:
`changes/assemble.py --version X.Y.Z --write` (in the release-preparation pull request) moves them
into `## [X.Y.Z] - <date>` and deletes them; `changes/assemble.py --unreleased` shows what the next
release would say. The `Release` workflow (`vX.Y.Z` tag) runs the same script on its own copy, so
fragments that are still in `changes/` are in the release notes even if nobody ran it first.

## Continuous integration

Each app has its own workflow, started only when its folder changes; a newer push to the same
pull request cancels the older run. To keep pull requests quick, some checks run only on `main`
and on release tags, never on pull requests:

| Check | Pull request | `main` and `v*` tags |
|---|---|---|
| Tests, clippy (native and wasm), formatting | yes | yes |
| Web builds | without `wasm-opt` (bigger, same code) | with `wasm-opt` |
| Desktop programs | debug build, starts under a headless X server | release build (ubuntu-22.04), release tests, starts |
| AppImage and Flatpak packages | only when `packaging/**` or their workflow changes | always |

Build caches are saved only by `main` (GitHub keeps 10 GB per repository; pull-request caches
pushed the main ones out). A pull request starts from `main`'s cache, so its first run is as
quick as later ones. Before pushing, run the formatting, clippy and tests for what you changed
locally: CI confirms, it shouldn't be the place where problems are found.
