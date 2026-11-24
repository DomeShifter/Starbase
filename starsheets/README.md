# StarSheets

A spreadsheet that looks and feels like Google Sheets, that you can run yourself.

![StarSheets showing a sales table with charts](docs/images/starsheets.png)

I wanted the spreadsheet I'm used to (Google Sheets' menus, shortcuts and side panels) without my
files living in someone else's cloud. StarSheets is my attempt at that: it runs in your browser,
your spreadsheets are ordinary `.xlsx` files in a folder you choose, and it's free and open source.

It stands on the shoulders of [GridCraft](https://github.com/storytold/gridcraft), a fast
spreadsheet engine written in Rust by the ArtCraft team. GridCraft does the hard parts
(formulas, files, the grid, charts); StarSheets puts a Google Sheets-style face on it and adds
the things Google Sheets users reach for.

> **Status: early preview.** It's usable and I use it myself, but there are rough edges and
> missing pieces. Please don't trust it with anything you can't afford to lose yet, and do
> [tell me what breaks](https://github.com/DomeShifter/Starbase/issues/new/choose).

## What you get

### Feels like Google Sheets
- **The same layout and habits**: menu bar, one-row toolbar, side panels, right-click menus,
  sheet tabs with ◀ ▶ when there are many, Google's sizes, fonts and keyboard shortcuts
  (Enter to edit, Ctrl+Enter for a new line in a cell, Ctrl+F to find, and so on).
- **Work together**: open a spreadsheet in several tabs or with other people and everyone sees
  every change as it happens, with each other's cursors. Share files and folders as "can edit"
  or "can view".
- **No Save button**: every change saves itself a moment later. A home screen with your folders
  and file previews, a bin, and version history to go back to an earlier copy.

### Formulas
- **Google Sheets formulas**, on top of GridCraft's hundreds of Excel ones:
  - `QUERY`: ask a range questions in a SQL-like language
    (`=QUERY(A1:D, "select A, sum(D) where B = 'Paid' group by A")`).
  - `ARRAYFORMULA`, `{…}` arrays (`={A2:A, B2:B}`), `SORTN`, and `SORT` with Google's
    TRUE/FALSE and several sort columns.
  - **Open-ended ranges** such as `A2:A` or `A3:C`, which run to the bottom of the sheet and keep
    up as you add rows.
  - Modern functions both apps share, such as `FILTER`, `UNIQUE`, `XLOOKUP`, `LET` and `LAMBDA`.
- **Formulas you can read**: while you type, each range has its own colour that matches the
  outline drawn round it on the sheet, and functions, numbers, text and errors are coloured like a
  code editor, the same in the cell and in the formula bar. Suggestions with a short description
  pop up as you type a function name.
- **Your Google formulas survive Excel**: formulas Excel can't read are saved the way Google Sheets
  saves them, so Excel shows their last results and StarSheets gets the formulas back unchanged.

### Tidy, checked data
- **Conditional formatting** in a Google-style side panel, previewed live as you choose: colour
  scales, "date is" rules (today, past week…), custom formulas and preset styles.
- **Data validation**: drop-down lists with coloured items, tick boxes, date pickers, number, text,
  email and URL rules, with a warning or a refusal for anything that doesn't fit.
- **Find and replace** (with regular expressions), **named ranges**, **protected sheets and
  ranges**, and **spelling suggestions** while you type.
- **Pictures in cells** as well as over them, charts with a colour for each pie slice, and
  Overflow, Wrap or Clip for long text.

### Your files stay yours
- Spreadsheets are **ordinary `.xlsx` files** in a folder you choose, readable by Excel and
  LibreOffice. What Excel has no place for (Google-style drop-down colours, Clip, protected-range
  settings) goes in a small extra part inside the same file that other programs ignore.
- **Excel files open well**, because GridCraft was built to match Excel.

What's not here (yet): comments with mentions, add-ons, desktop apps, a phone layout, and some
Google-only functions (`SPLIT`, `REGEXMATCH`, `IMPORTRANGE`, `GOOGLEFINANCE` and others). See the
[roadmap](ROADMAP.md).

## Try it with Docker

```sh
docker run -d --name starsheets --restart unless-stopped -p 8080:8080 -v starsheets:/data ghcr.io/domeshifter/starsheets:latest
docker logs starsheets   # shows the setup code
```

Then open <http://localhost:8080> and make your account with the setup code from the log: you
become the server's owner and can add other people (your avatar › People), or turn on sign-up so
they can make their own account, with a code you give out if you like. Everyone signs in with
a username and password kept on your server; there is no sign-in through other companies. Your
spreadsheets are kept as `.xlsx` files in the `starsheets` volume. There's a ready-made
[compose file](docker/compose.yml) too.

- Open the same spreadsheet in several tabs, or with other people, and everyone sees every change
  as it happens, with each other's cursors.
- On a private network you trust, `STARSHEETS_AUTH=off` turns sign-in off (everyone is a guest).
- Copy and paste work best over HTTPS (browsers limit the clipboard on plain `http://`), so a
  reverse proxy with a certificate is a good idea if you use it from other machines. Behind a
  proxy, or reachable from the internet, see the security settings in [docs/deploy.md](docs/deploy.md).

## Build it yourself

You need Rust (stable) with the `wasm32-unknown-unknown` target, [trunk](https://trunkrs.dev) and
Python 3.

```sh
scripts/fetch-upstream.sh          # GridCraft at the pinned release, with StarSheets' patches
python3 scripts/fetch-fonts.py     # the free fonts in the font menu
cd app && trunk build --release    # the app, in dist/
```

Serve `dist/` with any web server, or run the server (`cargo run --release -p starsheets-server`)
with `SITE=app/dist` to get the app and the file storage together. More in [docs/deploy.md](docs/deploy.md) and
[CONTRIBUTING.md](CONTRIBUTING.md).

## How it fits together

| Part | Where it comes from |
|---|---|
| Formula engine, xlsx/csv import and export, grid, charts | [GridCraft](https://github.com/storytold/gridcraft), one release plus our changes, kept on [our fork](https://github.com/DomeShifter/gridcraft/tree/starsheets) (listed in [docs/upstream.md](docs/upstream.md)) |
| Google Sheets look: title, menus, toolbar, side panels | `app/` |
| Google-only functions (QUERY) | `sheets-functions` crate in [our GridCraft fork](https://github.com/DomeShifter/gridcraft/tree/starsheets/crates/sheets-functions) |
| File storage, accounts, sharing and live editing | `crates/server` (Rust) |

Design choices and the reasons for them are in [docs/decisions.md](docs/decisions.md).

## Thanks

To the ArtCraft team for GridCraft, to the [egui](https://github.com/emilk/egui) project, and to
the designers of the fonts StarSheets uses (Roboto and the Google Fonts families listed in each
build's `webfonts/NOTICE.txt`, all under the SIL Open Font License) and of Google's Material Symbols icons.

## Licence

MIT or Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE),
[NOTICE](NOTICE)).

StarSheets is not affiliated with or endorsed by Google or the ArtCraft team. "Google Sheets" is a
trademark of Google LLC, mentioned only to describe the look StarSheets aims for.
