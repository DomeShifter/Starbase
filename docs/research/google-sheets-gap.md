# Starbase Sheets vs Google Sheets: gap list

_Research only, 2026-11-24. Sources: `starsheets/app/src/shell.rs` (MENUS + TOOLBAR tables), the side panels in `app/src/*panel.rs`, `bottom.rs` (sheet tab menu), `README.md`, `ROADMAP.md`, `CHANGELOG.md` (to Unreleased after 0.4.0), `docs/sheets-reference.md`, `crates/server/src/{store,sharing}.rs`; GridCraft at the pinned commit `ad4b590` (fork branch `starsheets`, shallow-cloned for reading): every `cmd!(…)` in `crates/engine` (**386** commands; Starbase Sheets' menus and toolbar reach about **74** of them), the right-click menus in `crates/ui-egui/src/grid/menus.rs`, and every registered function in `crates/functions`, `crates/sheets-functions` and `crates/calc` (**521** names)._

**Key to "Starbase Sheets today":**
- **Has**: in a menu, the toolbar, a right-click menu or a side panel.
- **Partly**: there, but missing a part Sheets users expect (said in Notes).
- **Engine-only**: a GridCraft command or function exists, but no Starbase Sheets menu entry or panel.
- **Missing**: nothing in the engine either.

**Size:** S = a menu entry or small panel (≤1 day); M = new UI or a server piece (days); L = a new subsystem (weeks).

## 1. Functions: how many Google functions we have

Google's function list (support.google.com/docs/table/25273, downloaded and parsed) has **516** functions. GridCraft registers **469** of them by name (**91%**). By name only: a few behave differently from Google's (checked: `DATEVALUE` was fixed in Unreleased; Excel's `REGEXEXTRACT`/`REGEXREPLACE` take extra optional arguments but accept Google's two- and three-argument forms).

| Google category | We have | Missing |
|---|---|---|
| Financial, Logical, Database, Filter | all | none |
| Math | 83 / 84 | COUNTUNIQUE |
| Statistical | 134 / 136 | AVERAGE.WEIGHTED, MARGINOFERROR |
| Text | 38 / 41 | **SPLIT**, **JOIN**, **REGEXMATCH** |
| Array | 27 / 29 | FLATTEN, ARRAY_CONSTRAIN (LAMBDA, MAP, REDUCE, SCAN, BYROW/BYCOL, MAKEARRAY, LET, FILTER, SORT, SORTN, UNIQUE, SEQUENCE, VSTACK/HSTACK, TOCOL/TOROW, CHOOSECOLS/ROWS, TAKE/DROP, GROUPBY, PIVOTBY are all there) |
| Date | 25 / 26 | EPOCHTODATE |
| Info | 16 / 18 | ISEMAIL, ISDATE |
| Lookup | 16 / 17 | GETPIVOTDATA (only read from files, not calculated) |
| Engineering | 44 / 47 | IMCOTH, IMLOG, IMTANH |
| Google | 2 / 7 | **IMAGE**, **SPARKLINE**, GOOGLEFINANCE, GOOGLETRANSLATE, DETECTLANGUAGE (ARRAYFORMULA and QUERY are there) |
| Web | 2 / 8 | **IMPORTRANGE**, IMPORTHTML, IMPORTXML, IMPORTDATA, IMPORTFEED, ISURL |
| Parser | 1 / 6 | TO_TEXT, TO_DATE, TO_PERCENT, TO_DOLLARS, TO_PURE_NUMBER |
| Operator | 2 / 17 | ADD, MINUS, MULTIPLY, DIVIDE, POW, EQ, NE, GT, GTE, LT, LTE, ISBETWEEN, UMINUS, UPLUS, UNARY_PERCENT |
| AI | 0 / 1 | AI (Gemini) |

**Most-used Google functions we lack, in rough order of how often people type them:**

| Function | Use | Size | Notes |
|---|---|---|---|
| SPLIT | Split text on a delimiter into cells | S | Very common. TEXTSPLIT exists; SPLIT is a thin wrapper (Google's `split_by_each`, `remove_empty_text` flags) |
| JOIN | Join a range with a delimiter | S | Wrapper on TEXTJOIN |
| REGEXMATCH | TRUE/FALSE regex test | S | Wrapper on REGEXTEST |
| IMPORTRANGE | Pull a range from another spreadsheet | M | Between Starbase files only: needs the server to resolve a file id/URL, check the person's access, and recalc on change |
| SPARKLINE | In-cell mini chart from a formula | M | Engine has Excel sparklines (`insert.sparkline`) to draw with; needs a formula-driven renderer + Google's options map |
| IMAGE | Picture in a cell from a URL | S–M | Pictures in cells exist; needs a server fetch proxy (CORS) |
| COUNTUNIQUE | Count distinct values | S | `=COUNTA(UNIQUE(…))` equivalent |
| FLATTEN | Range to one column | S | TOCOL equivalent |
| GOOGLEFINANCE | Live and historic prices | – | Not building (Google data) |
| GOOGLETRANSLATE / DETECTLANGUAGE | Translation | – | Not building (cloud); maybe the local LLM later |
| IMPORTHTML / IMPORTXML / IMPORTDATA / IMPORTFEED | Scrape tables / XPath / CSV / RSS from the web | M | Needs the same server fetch proxy as IMAGE, plus refresh rules and a size cap |
| ISEMAIL, ISURL, ISDATE, TO_TEXT, EPOCHTODATE, AVERAGE.WEIGHTED, ARRAY_CONSTRAIN, operator functions (ADD, EQ…) | Small helpers | S (all together) | Each is a few lines; do as one batch so pasted Google formulas never show #NAME? |

## 2. Feature table

| Feature | Google Sheets: where | Starbase Sheets today | How common | Size | Notes (GridCraft command / file) |
|---|---|---|---|---|---|
| New / Open / Import / Make a copy / Move to bin / Details / Print | File | Has | High | – | `panel:new/open/copy/bin/details`, `file.open`, `file.print` |
| Download .xlsx / .pdf / .html / .csv | File › Download | Has | High | – | `file.saveAs`, `file.exportPdf/Html/Csv` |
| Download .tsv | File › Download | Engine-only | Low | S | `file.saveAs` takes `.tsv` |
| Download .ods | File › Download | Missing (engine reads ODS, can't write it) | Low–Med | M | `crates/xlsx/src/ods.rs` is read-only |
| Import xls, xlsm, xlsb, ods, tsv, txt | File › Import | Engine (check the upload accepts them) | Med | S | `file.open` reads all of these; Sheets' Import also offers "insert new sheet(s) / append / replace data at selected cell" (Missing, S–M) |
| Rename / star / move to folder | Title row | Has | High | – | Home screen + title |
| Share: owner / can edit / can view / link | Share | Has | High | – | `crates/server/src/sharing.rs` (`Role::View/Edit/Owner`) |
| **Commenter** role | Share dialog | Missing | Med | S–M | Needs comments first (below) |
| Email / email collaborators / publish to web / embed | File › Share / Email | Missing | Low | M | Skip, except maybe "Publish to web" as a public read-only link later |
| Version history: list, restore | File › Version history | Partly | High | – | Server keeps the last **6** versions (`KEEP`, `store.rs`) |
| Version history: named versions, see who changed what, highlight changes, more than 6 | File › Version history | Missing | Med–High | M | Store per-edit-session snapshots with author; "name current version"; a diff view of changed cells |
| Cell edit history (right-click › Show edit history) | Right-click | Missing | Med | M | The live server already sees every edit with its author; needs a per-cell log |
| Autosave, live co-editing, cursors | Built in | Has | High | – | `crates/server/src/live.rs` |
| Offline | File › Make available offline | Missing | Low | L | Home server; skip |
| Settings: locale, time zone, calculation (iterative, manual) | File › Settings | Partly | Med | S | Settings panel exists; check locale/time zone; `formulas.calculationOptions` (incl. iterative) is engine-only |
| Undo / redo / cut / copy / paste / paste special | Edit | Has | High | – | Paste special is GridCraft's Excel-style dialog; Sheets has a sub-menu (values only, format only, formula only, data validation only, conditional formatting only, transposed, column widths) |
| Paste special as a Sheets sub-menu | Edit › Paste special ▸ | Has (except Conditional formatting only: engine) | High | S | Same command with params; Ctrl+Shift+V = values is the most used |
| Move row / column up, down, left, right | Edit › Move | Partly (columns; rows need an engine command) | Med | S | `sheet.moveColumns`; no row move in the engine |
| Delete values / row / column / cells | Edit › Delete | Has | High | – | |
| Find and replace, Ctrl+F bar | Edit | Has | High | – | `findreplace.rs`, regex included |
| Show gridlines / formula bar / formulas | View › Show | Has | Med | – | |
| Show protected ranges | View › Show | Missing | Low | S | Draw the hatching Sheets shows |
| Freeze rows / columns (incl. 2 rows/cols) | View › Freeze | Has | High | S | Drag bars at the corner are "later" in the reference |
| Group / ungroup rows and columns | View › Group | Has | Med | – | `data.group/ungroup` |
| Hidden sheets list | View › Hidden sheets | Has (≡ list) | Med | – | `bottom.rs` |
| Zoom | View › Zoom | Has | Med | – | |
| Full screen | View › Full screen | Missing | Low | S | Browser fullscreen API |
| Compact controls (Ctrl+Shift+F) | ⌃ top right | Missing | Low | S | Hide menu bar |
| Insert cells / rows / columns / sheet | Insert | Has | High | – | Also right-click |
| Hide / unhide rows and columns | Right-click | Has | High | – | `grid/menus.rs` |
| Resize rows/columns (dialog, fit to data) | Right-click › Resize | Has | Med | – | |
| Sheet tab menu: delete, duplicate, copy to, rename, colour, protect, hide, move | Sheet tab | Has | High | – | `bottom.rs`; "View comments" missing (no comments) |
| Chart: line, area, column, bar (stacked, 100%), pie, doughnut, scatter, bubble, combo, histogram, radar, waterfall, treemap, candlestick, box plot | Insert › Chart | Has | High | – | `chartpanel.rs`, plus funnel and sunburst which Sheets lacks |
| Chart: stepped area, gauge, geo, org chart, timeline, table chart, scorecard | Chart editor › Chart type | Missing | Low–Med | M each | Scorecard (one big number with change) is the most used of these on dashboards; geo needs map data |
| Chart editor: trendlines, data labels, axis min/max, log scale, error bars, aggregate, "use row 1 as headers", switch rows/columns | Chart editor › Customise / Setup | Partly / Engine-only | Med | S–M | Panel has style, titles, slice colours, series colour, trendline options, legend, gridlines, vertical axis Min/Max (2026-11-25; no log scale). `chart.addElement`, `chart.switchRowColumn` exist |
| Pivot table + editor | Insert › Pivot table | Has (GridCraft's Excel-style field pane) | High | S–M | `ui-egui/src/pivot_pane.rs`, `pivot.*`. Restyle it as Sheets' side panel ("Rows / Columns / Values / Filters", "Suggested") |
| Pivot: calculated field, group by date/number, filter by condition, show as % of | Pivot editor | Engine-only | Med | S | `pivot.calculatedField`, `pivot.group`, `pivot.filter`, `pivot.valueSettings` |
| Image in cell / over cells | Insert › Image | Has | Med | – | |
| Drawing (shapes, text boxes, arrows) | Insert › Drawing | Engine-only | Low–Med | M | `insert.shape`, `insert.textBox`, `object.*`; Sheets uses a separate drawing canvas |
| Function sub-menu, all functions list | Insert › Function / Help › Function list | Has | High | – | |
| Formula help card (syntax, example; F1, Shift+F1) | While typing | Partly | High | S–M | Name suggestions with a short description exist; the full card with arguments highlighted is "later" in the reference |
| Formula result preview (F9 tooltip), error explanation | While typing / hover | Has | Med | – | `calcpreview.rs`, `errortip.rs` |
| Link (Ctrl+K), link card | Insert › Link | Has | High | – | `linkcard.rs`; "link to a range / sheet" choice maybe missing (S) |
| Tick box | Insert › Tick box | Has | High | – | `insert.checkbox` |
| Drop-down (data validation) with coloured chips | Insert › Drop-down | Has | High | – | `dvpanel.rs` |
| Drop-down: **multiple selections**, chip display style, data-range-driven colours | DV panel | Partly | Med–High | S | Multiple selections and Chip / Arrow / Plain text done (2026-11-25); colours from a data range missing |
| Emoji | Insert › Emoji | Missing | Low | S | Picker UI only |
| Smart chips: @person, @date, @file, place, finance, YouTube, ratings | Insert › Smart chips / type @ | Missing | Med | M | @date (date picker exists) and @person (server users) make sense; rating chip (★) is cheap; the rest are Google-tied |
| Note (Shift+F2) | Insert › Note | Has | Med | – | `notes.rs` |
| **Comments**: add (Ctrl+Alt+M), reply, resolve, edit, delete, comments panel | Insert › Comment; toolbar | Engine-only | High | M | `review.newComment`, `replyComment`, `resolveComment`, `deleteComment`, `next/previousComment` exist. Needs a Sheets-style thread card + side panel, author names from accounts, and to be live-synced through the server |
| Comments: @mention, assign, notifications, emoji reactions | Comment box | Missing | Med | M | Needs comments first + server notifications |
| Theme (fonts + colours for the whole file) | Format › Theme | Missing (engine has Excel themes) | Low–Med | S–M | Workbook theme exists in the file model; a picker of a few Sheets-like themes |
| Number formats (all of Sheets' list, custom number, custom date/time) | Format › Number | Has | High | – | `customnumber.rs`, `customdate.rs` |
| Custom currency (other symbols, e.g. $, €, ¥) | Format › Number › Custom currency | Has | Med | S | `currency.rs` |
| Text: bold, italic, underline, strikethrough | Format › Text | Has | High | – | |
| Alignment, wrapping (overflow, wrap, clip), rotation | Format | Has | High | – | |
| Font size, fonts | Toolbar | Has | High | – | 28 fonts |
| Merge cells | Format › Merge | Has | Med | – | |
| Borders, colour, line style | Toolbar | Has | High | – | |
| Convert to table (Ctrl+Alt+T) | Format › Convert to table | Partly | Med–High | M | Runs GridCraft's Excel table (`insert.table`, `table.*`). Sheets' **Tables** add column types (number, text, date, drop-down, tick box, chips), a column menu (Ctrl+Alt+\\), table references `Table[Column]` and Pre-built tables. Engine has structured refs and banded rows; column types and the Sheets look are missing |
| Pre-built tables (templates) | Insert › Pre-built tables | Missing | Low–Med | M | Needs a template gallery (shared with the home screen's "start a new spreadsheet" row) |
| Conditional formatting (single colour, colour scale, custom formula) | Format › Conditional formatting | Has | High | – | `cfpanel.rs` |
| Alternating colours | Format › Alternating colours | Engine-only (as table styles) | Med–High | S–M | `table.style`, `table.bandedRows`, `home.formatAsTable`; Sheets' panel = header colour + two band colours + footer, on a plain range. Skipped in batch 1: a table rewrites header cells and adds a header row when there is none, so plain-range banding needs an engine change |
| Clear formatting (Ctrl+\\) | Format | Has | High | – | |
| Paint format | Toolbar | Has | High | – | `edit.formatPainter` |
| Sort range A→Z / Z→A / advanced (several columns, header row) | Data › Sort range | Has | High | – | |
| **Sort sheet** (by column, whole sheet keeping a frozen header) | Data › Sort sheet | Has | High | S | `menuactions.rs` |
| Filter: by values, by condition, filter by cell value | Data › Create a filter | Has | High | – | `data.filter`, right-click "Filter by cell value" |
| Filter / sort by colour (fill, text, conditional formatting colours) | Filter menu | Has (fill and text; not conditional-format colours) | Med | – | On the roadmap ("later: sort/filter by colour, filter by condition") |
| **Filter views** (personal, named, shareable by URL; temporary views for viewers) | Data › Create filter view | Has (named, per person, saved in the file; no sort inside a view, no `?fvid=` link yet) | High (in shared sheets) | M | Biggest everyday gap for shared sheets: live editing makes one person's filter hide rows for everyone. Needs per-person view state on the server (not in the .xlsx) + URL `?fvid=` |
| Group by view | Data › Create group by view | Missing | Low–Med | M | New (2025-26) Tables feature; GROUPBY function exists |
| Slicer | Data › Add a slicer | Partly | Med | S–M | `insert.slicer` is GridCraft's Excel slicer (tables/pivots). Sheets' slicer works on any chart/pivot range with condition or values and "set current filters as default" |
| Protect sheets and ranges (warning / restrict, except cells) | Data › Protect | Partly | Med | M | `protectpanel.rs`. Sheets restricts to **named people**; ours protects for everyone (Excel-style) — needs the server to enforce per-person edit rights on ranges |
| Named ranges | Data › Named ranges | Has | Med | – | `namespanel.rs`, name box list |
| **Named functions** | Data › Named functions | Engine-only (partly) | Med (power users) | S–M | LAMBDA exists; a named LAMBDA in Name Manager may already work (unverified). Needs Sheets' panel (name, arguments, description, import from another file) |
| Randomise range | Data › Randomise range | Has | Low | S | Shuffles the selected rows' contents; formats stay |
| Column stats (counts, distribution chart, frequency) | Data › Column stats | Missing | Low–Med | S–M | `selection.stats` gives the numbers; needs the side panel |
| Data validation (all criteria, warning / reject, help text) | Data › Data validation | Has | High | – | `dvpanel.rs` |
| Data clean-up: remove duplicates | Data › Data clean-up | Has | Med | – | `data.removeDuplicates` |
| Data clean-up: trim whitespace, clean-up suggestions | Data › Data clean-up | Partly (trim) | Med | S | Suggestions (inconsistent spellings, number-as-text) are M |
| Split text to columns (auto-detect separator) | Data › Split text to columns | Has | Med | – | `data.textToColumns` |
| Data extraction (pull parts out of smart chips) | Data › Data extraction | Missing | Low | – | Needs chips; skip |
| Spelling | Tools › Spelling | Has | Med | – | Personal dictionary per browser (could move to the account) |
| **Autocomplete** (suggest the rest of a typed value from the column) | Tools › Suggestion controls | Check | High | S | Excel's AutoComplete may already be in GridCraft; check it's on |
| **Smart Fill** (Ctrl+Shift+Y, pattern suggestions) | Tools › Suggestion controls | Engine-only (as Flash Fill) | Med | S–M | `edit.flashFill` / `data.flashFill` (Excel's Ctrl+E); Sheets offers it as a grey suggestion to accept. Knowledge-Graph part not buildable |
| Formula suggestions / formula corrections | Tools › Suggestion controls | Missing | Low | M | AI-ish; skip or local LLM later |
| Calculation controls (recalc settings, iterative) | Tools › Calculation controls | Partly | Low | S | Only "Calculate now"; `formulas.calculationOptions` has the rest |
| Conditional notifications / notification settings (email on change) | Tools | Missing | Low–Med | M | Needs mail or Telegram-like hooks on the server |
| Accessibility checker | Tools › Accessibility | Has (checker) | Low | – | `review.checkAccessibility`; screen readers on an egui canvas: L |
| Create a form | Tools › Create a new form | Missing | Low | – | No forms product yet |
| Macros: record, run, shortcuts | Extensions › Macros | Engine-only | Low–Med | M | `macro.*`, `automate.*` (GridCraft has its own recorder + script runtime, Excel-flavoured) |
| Apps Script, add-ons, AppSheet | Extensions | Missing | Low (most users) | – | Not building |
| Search the menus (Alt+/) | Help / toolbar magnifier | Has | Med | S | `shortcuts.rs` |
| Keyboard shortcuts dialog (Ctrl+/) | Help | Has | Med | S | `shortcuts.rs` |
| "Enable compatible spreadsheet shortcuts" (Excel keys) | Shortcuts dialog | Engine-only | Low | S | GridCraft's Excel keymap exists |
| Comment shortcuts (Ctrl+Alt+M, Ctrl+Alt+Shift+A, R/J/K/E/U) | Keys | Missing | Med | S | After comments |
| Insert/delete menu keys (Ctrl+Alt+= / Ctrl+Alt+-), sheet menu (Alt+Shift+S), sheet list (Alt+Shift+K) | Keys | Partly | Low | S | Ctrl+Alt+= shown in menus; others to check |
| Timeline view | Insert › Timeline | Missing | Low | L | Project-tracking view; skip for now |
| Explore, Gemini side panel, "Help me organise", AI() function, Fill with Gemini | Explore / side panel | Missing | Med (marketing), Low (use) | – | Not building now; the local LLM could do a version later |
| Connected Sheets, Data connectors (BigQuery, Looker), Sheets canvas | Data › Data connectors | Missing | Low | – | Not building |
| Phone layout | – | Missing | Med | L | On the README "not here yet" list |

## 3. Top 15 to build next (value ÷ size)

**Cheap wins (S): the engine has it or it's a thin wrapper:**
1. **Missing Google functions, batch 1**: SPLIT, JOIN, REGEXMATCH, COUNTUNIQUE, FLATTEN, ISEMAIL, ISURL, ISDATE, TO_TEXT, EPOCHTODATE, AVERAGE.WEIGHTED, ARRAY_CONSTRAIN and the operator functions (ADD, EQ, …). Most are wrappers on functions we have; pasted Google formulas stop showing #NAME?. SPLIT alone is one of the most used text functions in Sheets.
2. **Paste special as Sheets' sub-menu** (values only = Ctrl+Shift+V, format only, formula only, transposed, column widths, DV only, CF only).
3. **Data › Sort sheet** sub-menu (the right-click already does it) and **View › Freeze 2 rows / 2 columns**.
4. **Help › Keyboard shortcuts (Ctrl+/)** and **Search the menus (Alt+/)**: both read the `MENUS` table.
5. **Alternating colours** side panel (`table.style`/banded rows on a plain range).
6. **Custom currency** formats + **Data clean-up › Trim whitespace**.
7. **Pivot editor details**: calculated field, group by date/number, filter by condition, "show as % of" (`pivot.*`), restyled as a Sheets side panel.
8. **Chart editor extras**: data labels, trendline, axis min/max, switch rows/columns (`chart.addElement`, `chart.switchRowColumn`).
9. **Edit › Move** (row/column up/down/left/right) and **Randomise range**.
10. **Smart Fill / autocomplete check**: make sure AutoComplete is on and offer Flash Fill (`data.flashFill`) as Sheets' grey suggestion with Ctrl+Shift+Y.

**Medium jobs (M), high value:**
11. **Comments**: thread cards, side panel, reply/resolve/delete, live-synced, with account names; then a **commenter** share role. The engine commands exist (`review.*Comment`). Sheets users reach for comments daily in shared files.
12. **Filter views**: personal filters that don't hide rows for everyone else, named and shareable by link. With live editing on, a plain filter is a nuisance in shared files.
13. **Drop-down: multiple selections + chip style**, and **filter/sort by colour**.
14. **IMPORTRANGE between Starbase files** (server resolves the file, checks the reader's access, pushes updates), then **IMAGE** + **SPARKLINE**.
15. **Version history upgrade**: more than 6 versions, named versions, who changed what (and later per-cell edit history), using the authors the live server already knows.

_Batch 2 (2026-11-25): 8 done as far as the engine goes (trendline options; axis min/max needs engine fields), 11 done without a side panel or commenter role, 12 only Create/Remove filter; 7, 12 (filter views) and 13 wait on GridCraft changes (starsheets/ROADMAP.md lists them)._

_After these: **Sheets-style Tables** (column types, column menu, Pre-built tables, group by view) and **per-person protected ranges** (server-enforced) are the next M–L jobs._

## 4. Deliberately NOT building (for now)

| Item | Why |
|---|---|
| GOOGLEFINANCE (live and historic prices) | Google's market data feed; no free equivalent with a licence to redistribute. Could later wrap a free API as our own function name |
| GOOGLETRANSLATE, DETECTLANGUAGE, AI(), Gemini side panel, Fill with Gemini, Explore, enhanced Smart Fill, formula corrections | Google cloud AI. Revisit with the local LLM (VM211) as its own project |
| IMPORTHTML / IMPORTXML / IMPORTFEED / IMPORTDATA (for now) | Need a server-side web fetcher with caching, limits and abuse protection; do after IMAGE proves the proxy |
| Connected Sheets, Data connectors (BigQuery, Looker), AppSheet, Sheets canvas | Google services |
| Apps Script, add-ons marketplace | Huge; GridCraft has its own macro/script runtime if we ever want scripting |
| Smart chips tied to Google (place/Maps, finance, YouTube, Drive file, calendar) and Data extraction | Google data; only @person, @date and rating chips make sense here |
| Create a form, Timeline view, Record video, Meet | Other products / low use |
| Email, email collaborators, conditional email notifications | Needs outgoing mail on the server; Share + link covers most of it |
| Offline mode | Home server; large job (PWA + sync) |
| Chart types: geo, org chart, annotated timeline | Low use; geo needs map boundary data |
| Excel-only GridCraft features (Solver, scenarios, goal seek, What-If data tables, Analysis ToolPak, form controls, ink drawing, Power Query) | Not in Google Sheets; leave them engine-only so the app stays Sheets-shaped |

## Sources

- Google Sheets function list (downloaded and parsed, 516 rows): https://support.google.com/docs/table/25273
- Chart types: https://support.google.com/docs/answer/190718
- Tables (column types, table references, group by views, pre-built tables): https://support.google.com/docs/answer/14239833
- Filters and filter views (by colour, temporary views): https://support.google.com/docs/answer/3540681
- Slicers: https://support.google.com/docs/answer/9245556
- Protected sheets and ranges: https://support.google.com/docs/answer/1218656
- Named functions: https://support.google.com/docs/answer/12504534
- Comments (mention, assign, resolve): https://support.google.com/docs/answer/65129
- Macros: https://support.google.com/docs/answer/7665004
- Smart Fill (search result snippet only, not opened): https://support.google.com/docs/answer/9914525
- Keyboard shortcuts and menus: already recorded in `starsheets/docs/sheets-reference.md` §5 (from https://support.google.com/docs/answer/181110, 2026-11-21) and `starsheets/docs/sheets-menus.md` (live Sheets screenshots, 2026-11-21).
- Download page https://support.google.com/docs/answer/49114 didn't list formats. The Download list (xlsx, ods, pdf, html, csv, tsv) and Import options are from `sheets-reference.md` and general knowledge; check against a live Sheets menu.
- "How common" is my judgement, not measured data.
