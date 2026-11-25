# Starbase Sheets roadmap

> **Last updated:** 2026-11-23 · **Now:** Sheets polish (find bar, Clip, Google formulas saved Excel-safe, pictures in cells) released in 0.4.0; next: desktop apps

| Phase | Goal | Done when | Status |
|---|---|---|---|
| 0 Test build | Our app with a Sheets menu bar and toolbar instead of GridCraft's ribbon, Sheets colours | Opens and edits an xlsx in the browser with no ribbon; list of what can't be restyled | **in progress** |
| 1 Look and feel | Sheets menus, toolbar with icons, bottom bar (+, sheet list, tabs, quick sum), blue selection, side panels instead of dialogs, Sheets shortcuts | Side-by-side screenshots match closely | next |
| 2 Saving like Drive | Small server: file list home page, saving after every change (no Save button), version history | Close the tab, reopen, nothing lost; restore an older version | |
| 3 Live editing and sharing | The same file open in several tabs or by several people, all seeing each other's changes as they happen; accounts; sharing like Google (owner, can edit, can view, link) | Two browsers edit together and stay identical; a viewer can't edit | **done in 0.2.0** ([plan](docs/live-editing.md)); follow-ups below |
| 5 Extras | Sheets-only functions (QUERY, SPLIT…), phone layout | as needed | |

## Phase 0 checklist
- [x] Repository, licence, docs
- [x] GridCraft pinned (v0.19.1) and fetched by script; patch queue
- [x] Patch: switch to hide the ribbon (`Chrome.ribbon`)
- [x] Patch: theme override (`Tokens::install`)
- [x] Title row, menu bar and toolbar driven by data tables
- [x] Sheets palette
- [x] CI builds the web app (about 5 minutes per build)
- [x] Hosted on the home server
- [ ] Spike report: what works, what needs more patches (→ docs/decisions.md)

## Phase 1 checklist
- [x] Toolbar: Material icons, font and size boxes showing the active cell, zoom, number formats, colour palette, borders, merge, alignment, wrapping, rotation, functions
- [x] Bottom bar: + / sheet list / tabs / quick sum (patch 0003)
- [x] Roboto as the default font (patches 0005, 0006)
- [x] Enter edits, no selection animation, Backspace only clears (patch 0004)
- [ ] Cell interactions list, remaining rows (docs/sheets-reference.md §4b)
- [x] Sheets keyboard shortcuts that differ from Excel (patch 0011; Ctrl+Shift+F and Alt+1 in the app)
- [x] Ctrl+/ shortcut list, Alt+/ menu search, Ctrl+Shift+V paste values
- [ ] Still to do from §5: Alt+Shift+K sheet list, comment navigation keys
- [ ] Side panels instead of dialogs (conditional formatting, data validation, charts)
- [x] Default text size 10 pt
- [x] Default 21 px rows and 100 px columns for new sheets only (patch 0010; opened files keep their sizes)
- [x] Right-click menu, error/note markers, link card (patches 0007-0009)
- [x] Grid at Sheets' 1:1 size, Sheets header sizes, plain arrow cursor, flat menus/toolbar, unboxed formula bar (patch 0012)
- [x] Typed web/e-mail addresses become links; browser shortcuts that clash with Sheets ones (Ctrl+D, Ctrl+B…) are blocked; hand cursors for dragging cells (patch 0013)
- [x] Sheets-style menus: icons, 32 px rows, grey shortcuts, ▸ submenus, dividers, rounded panel with shadow (docs/sheets-menus.md); Sheets-blue links (patch 0014); link card under its cell
- [x] Menu contents matched to docs/sheets-menus.md where GridCraft has the command (Number, Rotation, Font size, Function, Zoom submenus; UK wording)
- [x] Right-click menus (cell, row and column headers, sheet tab + its ▾) and toolbar dropdowns (icon strips, number formats with examples) in the Sheets look (patch 0015)
- [x] Sheets menu batch 1 (docs/research/google-sheets-gap.md items 2-6, 9): Paste special sub-menu, Move column, Freeze 2 rows/columns, Sort sheet, Randomise range, Trim whitespace, Custom currency, Keyboard shortcuts, Search the menus
- [x] Sheets menu batch 2 (gap list items 7, 8, 11-13, what the engine allows): comment cards (Insert › Comment, Ctrl+Alt+M; reply, resolve, edit, delete), trendline type, degree, equation label and R² in the chart editor, Create/Remove filter
- [ ] Batch 2 leftovers that need GridCraft changes: Sheets-style pivot editor (GridCraft draws its own pane), comment dates and per-reply edit/delete, Comment on the right-click menu; commenter share role (server); sorting inside a filter view (the grid can't show rows in another order for one person)
- [ ] Still missing from Sheets menus: Clip wrapping, Hidden sheets, Full screen, Version history (phase 2), Edit › Move row up/down, Paste special › Conditional formatting only, Alternating colours (all need engine work)
- [x] Named ranges suggested while typing formulas (patch 0016); sheet tabs: ▾ opens the menu, drag to reorder
- [x] Conditional formatting side panel: single colour (19 Sheets conditions, style, presets) and colour scales with configurable points (docs/sheets-cf.md, patch 0017); tab colour bar inside the tab
- [x] Data validation side panel: Sheets' 25 criteria, drop-down items with colours, help text, warning or reject, arrow or plain (docs/sheets-dv.md, patch 0018)
- [x] Date picker when editing a cell with a date rule
- [x] Drop-down colours as in Sheets: light and dark chips, text in the partner colour
- [x] Phase 2 start: files on a server folder (server/files.py): autosave a moment after each change (new sheets once they have data), File › Open list, Make a copy, Move to bin, version history (last 6), free names with " (2)", a save over a newer copy becomes a "conflicted copy"
- [x] Filter menu like Sheets (patch 0019); sort/filter by colour and filter views (0093); later: filter by condition
- [x] Conditional formatting date choices (today, tomorrow, yesterday, past week/month/year; before/after one week/month/year ago)
- [x] Importing into a sheet that has data opens the file in a new tab
- [x] Fonts: 28 free fonts (look-alikes for Arial, Calibri, Cambria, Comic Sans, Courier New, Georgia, Impact, Times New Roman, Trebuchet, Verdana, plus Sheets' Google fonts), fetched on first use (patch 0021, app/webfonts.txt)
- [x] File window: folders, path bar, search, sort, new folder/spreadsheet here, rename, copy/cut/paste, drag to move, bin with restore, download
- [x] Saving on close: closing or hiding the tab saves at once; the status line says where and when it saved (hover) and opens version history (click)
- [x] A file opens in this tab only if the sheet here is blank; otherwise a new tab
- [x] Google Sheets formulas: `{…}` array literals with formulas and ranges, ARRAYFORMULA, Ctrl+Shift+Enter wraps in ARRAYFORMULA (patch 0022)
- [x] QUERY stage 1–3 (crates/sheets-functions): select, where (comparisons, contains, starts/ends with, matches, like, is null, and/or/not), group by with sum/avg/count/min/max, pivot, order by, limit, offset, label, format, date literals and functions; headers guessed
- [ ] QUERY: check every case against answers recorded from Google Sheets; dates shown as dates in results
- [x] Data validation: chip display style, "Allow multiple selections"
- [ ] Data validation later: hover text on invalid cells
- [ ] Dark mode palette
- [x] Plain-language explanation when hovering over an error cell

## Live editing checklist ([plan](docs/live-editing.md))
- [x] Step 0: Rust server (`crates/server`) replaces server/files.py, same API and same files on disk; checked against the old server request by request (`crates/server/tests/parity.py`)
- [x] Step 1: live sync between tabs (server runs the engine, saves the file; tabs catch up with each other); undo of your own latest change
- [ ] Step 1 follow-ups: keep changes made while offline across a reload; a log of unsaved changes on the server for a crash; selection follows rows inserted above it
- [x] Step 2: accounts (local, username and password; owner set up with a code from the log, owner adds people) and seeing other people's cursors and avatars
- [x] Step 3: undo that only undoes your own changes (cell by cell; refused when someone changed those cells since)
- [x] Step 4: share dialog (owner, editors, viewers; anyone signed in with the link) and view-only mode
- [x] Sign-up: people make their own account when the owner allows it (optional sign-up code)

## Files and folders checklist
- [x] Files changed outside Starbase Sheets (Excel, a file share) are never written over
- [x] Everyone's own folder; folder sharing; own bin (30 days); stars and recent
- [x] Preview pictures, drawn on the server
- [x] Preview pictures of documents: Starbase Docs sends one of page 1 after each save (server side; Starbase Docs side in progress)
- [x] Home screen with a folder tree, cards or list, drag to move, upload
- [x] A "Move to…" folder picker and a Details panel (owner, who changed it last, who has access)
- [x] Sort menu; copy a sheet to another spreadsheet; choose where files are kept
- [x] Backups (prototype): daily, encrypted, in restic's format to any folder the server can reach; keep 7 days / 4 weeks / 12 months; restore anyone's files (owner) or your own into a new folder
- [ ] Backups next: S3 / Backblaze B2 targets, restore everything onto a new server from the app, choose the keep rules

The full Sheets feature list with a phase per item is in [docs/sheets-reference.md](docs/sheets-reference.md).
