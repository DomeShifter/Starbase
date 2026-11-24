# StarSheets: notes for AI assistants

- Goal and phases: README.md, ROADMAP.md. Target behaviour: docs/sheets-reference.md. Choices: docs/decisions.md (add a line for every new one).
- **Files stay .xlsx** (docs/decisions.md 2026-11-23): never add a StarSheets-only file format. Data Excel can't hold goes in the StarSheets part inside the .xlsx; caches are derived and disposable.
- **Never edit `upstream/`.** A change GridCraft must make is a commit on the fork's `starsheets` branch (github.com/DomeShifter/gridcraft), subject `<crate>: <what> (StarSheets NNNN)`; then bump `commit` in `upstream.pin` and add a row in docs/upstream.md. Offer upstream where useful.
- **House style:** build every dialog, menu, card and panel from `app/src/house.rs` (docs/house-style.md); no one-off sizes or colours.
- Menus and toolbar are data tables in `app/src/shell.rs`; only use command ids that exist (the test checks).
- This repo is **public**: no hostnames, IP addresses, credentials or details of anyone's home server. Deployment specifics live outside the repo.
- Keep docs short and in plain words; update ROADMAP.md checkboxes as work lands.
- Licence headers: files adapted from GridCraft say so in their first lines.
- **No warnings:** run `cargo fmt -p starsheets -p starsheets-server -p starsheets-sync` (not `--all`: that reformats GridCraft in upstream/) and keep clippy clean (CI's "Format and lint" job fails on any warning). Fix rather than `#[allow]`; an allow says why above it.
