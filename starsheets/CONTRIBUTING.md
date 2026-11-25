# Contributing

Thank you for being here. Starbase Sheets is a small project, and every bug report, idea and fix helps.

## Reporting a bug or suggesting an idea

Please [open an issue](https://github.com/DomeShifter/Starbase/issues/new/choose). For a bug, the most useful things are: what you
did, what you expected, what happened instead, and your browser or system. A small .xlsx file or a
screenshot helps a lot. If the same thing works differently in Google Sheets, that's worth saying:
matching Google Sheets is the goal.

## Changing the code

1. Read [docs/architecture.md](docs/architecture.md) and [docs/upstream.md](docs/upstream.md).
   Starbase Sheets is built on [GridCraft](https://github.com/storytold/gridcraft): we never edit
   GridCraft's copy in `upstream/` directly. Changes to it are commits on our fork's
   `starsheets` branch, each listed in docs/upstream.md.
2. Build:
   ```sh
   scripts/fetch-upstream.sh             # GridCraft at the pinned commit of our fork
   python3 scripts/fetch-fonts.py        # the web fonts (needed for a full build)
   cd app && trunk build --release       # the web app, in app/dist/
   cargo test -p starsheets -p starsheets-server -p starsheets-sync
   ```
   Before a pull request: `cargo fmt -p starsheets -p starsheets-server -p starsheets-sync`, and no clippy warnings
   (`cargo clippy -p starsheets --target wasm32-unknown-unknown`, and
   `cargo clippy -p starsheets-server -p starsheets-sync --all-targets`). CI checks both.
3. Keep pull requests small and about one thing. Say what it changes for someone using Starbase Sheets.
4. Menus and the toolbar are tables in `app/src/shell.rs`; a test checks that every entry names a
   real command.
5. Google-only functions (like QUERY) live in the fork's `crates/sheets-functions` (in `upstream/gridcraft` once fetched), with their own tests.

Where Google Sheets and Excel behave differently, Starbase Sheets follows Google Sheets, but we try not
to break opening and saving Excel files.

## How we talk to each other

Kindly. See the [code of conduct](CODE_OF_CONDUCT.md).
