# Google Sheets reference: what Starbase Sheets emulates

> **Last reviewed:** 2026-11-21 · **Sources:** Google's keyboard-shortcut help page (support.google.com/docs/answer/181110, read in full 2026-11-21), Google Workspace Updates blog and "What's new in Google Workspace" (2026 entries), press coverage of the 2023–2025 Sheets UI refreshes. Menu and toolbar contents marked *(from memory)* are recalled, not checked against a live screen. Verify those against a screenshot before building them.

This is the target we copy for **behaviour and layout**, not for branding. Starbase Sheets never uses Google's name, logo, product icons or exact trademarked artwork. Icons come from an openly licensed set (Material Symbols, Apache-2.0).

Status column used throughout: **P1** = phase 1 (look and feel), **P2** = phase 2 (files, autosave, history), **P3** = sharing, **P4** = live co-editing, **Later**, **No** (deliberately out of scope).

---

## 1. Screen layout (top to bottom)

| # | Area | What Sheets shows | Starbase Sheets | Notes |
|---|---|---|---|---|
| 1 | Title row | App icon (links home) · editable file name · star · "move" · cloud status icon ("Saved to Drive") · right: version history clock, comments, Meet, **Share** button (filled, rounded), account avatar | P1 (name, status), P2 (history), P3 (Share) | No star/move/Meet. The status icon becomes "Saved" / "Saving…" in P2 |
| 2 | Menu bar | File · Edit · View · Insert · Format · Data · Tools · Extensions · Help (+ Accessibility when screen reader on) | P1 | Sits under the file name, same row block as the title. Icons beside menu items (added 2023) |
| 3 | Toolbar | One rounded "pill" row spanning the window, light tinted background (Material 3). Contents in §3 | P1 | Overflow collapses into a "⋮" more button on narrow windows |
| 4 | Formula bar | Name box (cell ref / named range dropdown) · "fx" · formula input (expandable with ▾ / Ctrl+Up/Down) | P1 | Reuse GridCraft's formula bar; restyle only |
| 5 | Grid | Light grey headers, thin light grid lines, **blue** selection border + light blue fill, small square fill handle at bottom-right, frozen-row/column drag bars at the top-left corner | P1 | Font default **Arial 10** |
| 6 | Bottom bar | **+** (add sheet) · **≡** (all sheets list) · sheet tabs (coloured underline when tab colour set; tab menu on right-click / ▾) · right side: **Explore** button and the quick-sum box (Sum / Average / Count… of the selection, clickable to pick which) | P1 (tabs, quick sum), No (Explore) | No status bar like Excel's: the quick-sum box is the only status |
| 7 | Side panels | Open on the right instead of dialogs: chart editor, conditional formatting, data validation, pivot table editor, named ranges, protected ranges, comments, version history, Explore, Gemini | P1 (structure), per feature after | The biggest UX difference from Excel's dialogs |

Visual tokens (approximate, from memory; match against screenshots in P1):

| Token | Sheets look |
|---|---|
| Page background behind chrome | very light blue-grey (#F9FBFD) |
| Toolbar pill | light blue tint (#EDF2FA), fully rounded ends |
| Text | near-black #1F1F1F, secondary #444746 |
| Accent / selection | blue #1A73E8 (border), fill rgba(26,115,232,0.1) |
| Headers | #F8F9FA background, #5F6368 text, active header darker with blue tint |
| Grid lines | #E2E3E3 |
| Share button | filled light blue (#C2E7FF) with dark text, pill shape |
| UI font | Google Sans / Roboto look-alike → we use an open font (e.g. Inter or Roboto, Apache-2.0) |
| Cell default font | Arial 10 (we ship Arimo, metric-compatible, Apache-2.0) |

## 2. Menus *(from memory, verify)*

| Menu | Items (in order, separators as ·) | Phase |
|---|---|---|
| **File** | New · Open · Import · Make a copy · Share · Email · Download (xlsx, ods, pdf, html, csv, tsv) · Rename · Move · Add shortcut to Drive · Move to trash · Version history · Make available offline · Details · Settings (locale, time zone, calculation incl. **manual calculation** (new 2026-09), iterative calc) · Print | P1 shell, items as they become real (P2 for New/Open/Version history) |
| **Edit** | Undo · Redo · Cut · Copy · Paste · Paste special (values, format, formula, data validation, conditional formatting, transposed, column widths) · Move (row/column up/down) · Delete (values, row, column, cells shift…) · Find and replace | P1 |
| **View** | Show (gridlines, formula bar, formulas, protected ranges) · Freeze (rows/columns: none, 1, 2, up to current) · Group / Ungroup · Comments · Hidden sheets · Zoom · Full screen | P1 |
| **Insert** | Cells (shift right/down) · Rows (above/below) · Columns (left/right) · Sheet · Timeline · Chart · Pivot table · Image (in cell / over cells) · Drawing · Function · Link · Checkbox · Dropdown · Emoji · Smart chips · Comment · Note · Table (Ctrl+Alt+T) | P1 for cells/rows/columns/sheet/chart/function/link/checkbox/dropdown/note; Later for the rest |
| **Format** | Theme · Number (Automatic, Plain text, Number, Percent, Scientific, Accounting, Financial, Currency, Date, Time, Date time, Duration, Custom) · Text (bold, italic, underline, strikethrough) · Alignment · Wrapping (overflow, wrap, clip) · Rotation · Font size · Merge cells · Convert to table · Conditional formatting · Alternating colours · Clear formatting | P1 |
| **Data** | Sort sheet · Sort range · Create a filter · Create filter view · Add a slicer · Protect sheets and ranges · Named ranges · Named functions · Randomise range · Column stats · Data validation · Data cleanup (cleanup suggestions, remove duplicates, trim whitespace) · Split text to columns · Data extraction · Data connectors | P1 core (sort, filter, validation, named ranges, remove duplicates, trim, split); Later for filter views/slicers |
| **Tools** | Create a new form · Spelling · Suggestion controls · Notification settings · Accessibility | Mostly No |
| **Extensions** | Add-ons · Macros · Apps Script · AppSheet | No (GridCraft has its own macro system: maybe Later) |
| **Help** | Search the menus / Tool finder (Alt+/) · Help · Training · Updates · Keyboard shortcuts (Ctrl+/) · Function list | P1 (tool finder + shortcuts dialog are high value) |

Right-click (cell) menu, shortened in the 2023 refresh: Cut · Copy · Paste · Paste special ▸ · Insert row above/below · Insert column left/right · Delete row · Delete column · Delete cells ▸ · Create a filter · Filter by cell value · Show edit history · Conditional formatting · Data validation · Smart chips ▸ · View more cell actions ▸ (comment, note, link, define named range, protect range, get link to this cell…). **P1.**

Sheet tab menu: Delete · Duplicate · Copy to ▸ · Rename · Change colour · Protect sheet · Hide sheet · View comments · Move right/left. **P1.**

## 3. Toolbar (left to right) *(from memory, verify)*

| Group | Buttons |
|---|---|
| Search | Menu search / tool finder (magnifier) |
| History | Undo · Redo · Print · Paint format |
| View | Zoom % (dropdown) |
| Number | Format as currency (£ follows locale) · Percent · Decrease decimals · Increase decimals · "123" more formats ▾ |
| Font | Font family ▾ · Font size with − / + steppers |
| Text | Bold · Italic · Strikethrough · Text colour (A with bar) |
| Cell | Fill colour · Borders ▾ · Merge ▾ |
| Align | Horizontal ▾ · Vertical ▾ · Wrapping ▾ · Rotation ▾ |
| Insert | Link · Comment · Chart |
| Data | Filter ▾ (create filter / filter views) |
| Functions | Σ Functions ▾ (SUM, AVERAGE, COUNT, MAX, MIN, More functions) |
| Far right | "Hide the menus" ⌃ (compact controls, Ctrl+Shift+F) |

## 4. Interaction behaviours that make it "feel like Sheets"

| Behaviour | Sheets | Phase |
|---|---|---|
| Saving | Every change saved automatically; status icon in title row; **no Save button**; Ctrl+S just shows "saved" | P2 |
| Version history | File › Version history: list of autosaved versions by time and person, named versions, restore, "show changes" highlights | P2 |
| Home screen | Docs-style file list: template gallery row on top, recent files grid/list, search, owner filter | P2 |
| Sharing | Share dialog: people + General access (restricted / anyone with link), roles Viewer / Commenter / Editor | P3 |
| Live presence | Coloured cursors and selections of other editors, avatars in title row | P4 |
| Formula help | As you type `=SUM(` a help card appears (syntax, example, summary), F1 toggles full/compact; range colour-coding; formula result preview tooltip (F9) | P1 (mostly in GridCraft already: check) |
| Autocomplete | Function name dropdown; **smart fill** / autofill suggestions | P1 (functions), Later (smart fill) |
| Fill handle | Drag the dot to fill series; Ctrl+Enter fill range | P1 (GridCraft has it) |
| Quick sum | Selection shows Sum in bottom-right; click to choose Avg, Min, Max, Count, Count numbers | P1 |
| Checkboxes & dropdowns | Insert › Checkbox makes TRUE/FALSE cells with ticks; dropdown chips (coloured pills) from data validation | P1 (checkbox), P1/Later (chip styling) |
| Tables | Convert to table (Ctrl+Alt+T): typed columns, header styling, column menu (Ctrl+Alt+\\), table references | Later (GridCraft has Excel tables) |
| Paste values | Ctrl+Shift+V | P1 |
| Hyperlinks | Hover card with preview; Alt+Enter opens | P1 |
| Comments vs notes | Comments are threaded (side panel, @mentions, resolve); notes are simple yellow popups (Shift+F2) | P1 notes, P3 comments |
| Freeze | Drag the thick grey bars at the top-left corner | P1 |
| Explore / Gemini | AI analysis panels | No (maybe Later with the local LLM) |

## 4b. Cell interactions: Google Sheets vs Microsoft Excel

GridCraft behaves like Excel. Starbase Sheets switches each row below to the Sheets way. Status: **done**, **next** (planned soon), **later**. *(From memory unless the row says checked: verify each one in Sheets before building it.)*

| # | Situation | Google Sheets | Excel / GridCraft | Starbase Sheets |
|---|---|---|---|---|
| 1 | Press **Enter** on a selected cell | Opens the cell for editing (cursor at the end) | Moves down a row | **done** (patch 0004) |
| 2 | Enter while editing / Shift+Enter / Tab while editing | Saves, moves down / up / right | Same | same already |
| 3 | Selection moving between cells | Outline jumps instantly | Outline glides (animation) | **done** (animation off) |
| 4 | **Backspace** on a selected cell | Clears the contents only | Clears and opens the cell for editing (Mac Excel) | **done** (patch 0004) |
| 5 | Enter with a copied range still marked | Nothing special (Enter edits) | Pastes the copied range and ends copy mode | **done** (patch 0004) |
| 6 | Copied range outline | Static dashed blue outline | Animated "marching ants" | **done** (patch 0004) |
| 7 | Copy marker after pasting | Stays; you can paste again | Same for Ctrl+V; ends after Enter-paste | same already |
| 8 | Default font / size / row height / column width | Arial 10 · 21 px rows · 100 px columns | Calibri 11 · 20 px rows · 64 px columns | font **done** (Roboto 10, patch 0006); row/column sizes **done** for new sheets (patch 0010); opened files keep theirs |
| 9 | Single click on a link cell | Selects it and shows a link card (preview, copy, edit, remove); open from the card or Alt+Enter | Ctrl+click follows (or click follows) | **done** (patch 0009 + app/src/linkcard.rs) |
| 10 | Formula error in a cell | Red triangle top-right; hover shows the error explanation | Green triangle top-left + warning icon menu | triangle **done** (patch 0008); hover explanation **next** |
| 11 | Notes and comments markers | Note: small black triangle top-right; comment: orange-yellow triangle, opens in side thread | Red triangle (note), purple (comment) | colours **done** (patch 0008); comment side thread **later** |
| 12 | Selection colours | Blue outline, light blue fill, active cell has no fill; headers of selected rows/columns turn light blue | Green outline, grey fill | **done** (palette) |
| 13 | Fill handle | Small blue square at the bottom-right of the selection; drag to fill; double-click fills down | Same, green | **done** (palette) |
| 14 | Typing a formula | Help card under the cell (syntax, example, description; F1 toggles); live result preview tooltip | Function tooltip only | **later** |
| 15 | Function name autocomplete | Dropdown while typing `=SU…`; Tab or Enter accepts | Same | same already |
| 16 | Ctrl+Enter when not editing | Fills the typed value into every selected cell | Same | same already |
| 17 | Alt+Enter while editing | New line inside the cell (Ctrl+Enter too) | New line inside the cell | same already |
| 18 | Space on a checkbox cell | Toggles it | Toggles it | same already |
| 19 | Paste values | Ctrl+Shift+V | Ctrl+Alt+V dialog / Ctrl+Shift+V (new Excel) | check |
| 20 | Delete key | Clears contents, keeps formatting | Same | same already |
| 21 | Clicking the corner box | Selects everything | Same | same already |
| 22 | Dropdown (data validation) cells | Shown as coloured chips with a ▾ arrow; click the chip to pick | Arrow only when selected | **later** |
| 23 | Undo after typing | Undoes the whole edit | Same | same already |
| 24 | Double-click a column border | Fits to data ("Fit to data") | Autofit | same already |
| 25 | Right-click on a cell | Sheets' shorter menu (see §2) with "View more cell actions" | Excel's long menu | **done** (patch 0007; "View more cell actions" submenu later) |
| 26 | Dragging a selection's border | Moves the cells (hand cursor) | Same | same already |
| 27 | Empty selection drag past the edge | Scrolls smoothly | Same | same already |
| 28 | Freeze panes | Drag the thick grey bars at the top-left corner | View › Freeze | **later** |
| 29 | Status/mode indicator | None ("Ready/Enter/Edit" doesn't exist) | Status bar mode | **done** (status bar hidden) |
| 30 | Ctrl+arrow, Ctrl+Shift+arrow, Home, Ctrl+Home | Same as Excel | — | same already |

## 5. Keyboard shortcuts (PC, Google's official list, 2026-11-21)

Where GridCraft (Excel) already uses the same key, nothing to do. Rows marked **≠ Excel** need our keymap to override GridCraft's.

### Common
| Action | Shortcut | |
|---|---|---|
| Select column / row | Ctrl+Space / Shift+Space | |
| Select all | Ctrl+A / Ctrl+Shift+Space | |
| Undo / Redo | Ctrl+Z / Ctrl+Y, Ctrl+Shift+Z, F4 | F4 ≠ Excel (Excel: repeat / toggle $ while editing) |
| Find / Find and replace | Ctrl+F / Ctrl+H | |
| Fill range / down / right | Ctrl+Enter / Ctrl+D / Ctrl+R | |
| Save / Open / Print | Ctrl+S / Ctrl+O / Ctrl+P | Ctrl+S = no-op "all changes saved" in P2 |
| Copy / Cut / Paste / Paste values | Ctrl+C / X / V / Ctrl+Shift+V | |
| Keyboard shortcuts help | Ctrl+/ | ≠ Excel |
| Insert new sheet | Shift+F11 | |
| Compact controls (hide menus) | Ctrl+Shift+F | ≠ Excel (font dialog) |
| Tool finder (search menus) | Alt+/ | ≠ Excel |
| Rename sheet | Alt+1 | ≠ Excel |
| Convert range to table | Ctrl+Alt+T | ≠ Excel (Ctrl+T) |

### Format
| Action | Shortcut | |
|---|---|---|
| Bold / Italic / Underline | Ctrl+B / I / U | |
| Strikethrough | Alt+Shift+5 | ≠ Excel (Ctrl+5) |
| Align centre / left / right | Ctrl+Shift+E / L / R | ≠ Excel |
| Borders top / right / bottom / left / remove / outer | Alt+Shift+1 / 2 / 3 / 4 / 6 / 7 (outer also Ctrl+Shift+7) | ≠ Excel |
| Insert link | Ctrl+K | |
| Insert date / time / date and time | Ctrl+; / Ctrl+Shift+; / Ctrl+Alt+Shift+; | |
| Format decimal / time / date / currency / percent / exponent | Ctrl+Shift+1 … 6 | same digits as Excel, slightly different meanings |
| Clear formatting | Ctrl+\\ | ≠ Excel |

### Navigate
| Action | Shortcut | |
|---|---|---|
| Row start / end; sheet start / end | Home / End; Ctrl+Home / Ctrl+End | |
| Scroll to active cell | Ctrl+Backspace | |
| Next / previous sheet | Alt+Down / Alt+Up | ≠ Excel (Ctrl+PgDn/PgUp; keep both) |
| List of sheets | Alt+Shift+K | |
| Open hyperlink | Alt+Enter | ≠ Excel (Excel: new line inside a cell). Check in Sheets what inserts a line break while editing |
| Revision history | Ctrl+Alt+Shift+H | P2 |
| Quick sum focus | Alt+Shift+Q | |
| Filter drop-down on filtered cell | Ctrl+Alt+R | |
| Cycle focus between areas | Ctrl+Alt+. / Ctrl+Alt+, | |

### Notes and comments
| Action | Shortcut |
|---|---|
| Insert/edit note | Shift+F2 |
| Insert/edit comment | Ctrl+Alt+M |
| Comment thread | Ctrl+Alt+Shift+A |
| In a selected comment: reply / next / previous / resolve / exit | R / J / K / E / U |

### Menus
| Action | Shortcut |
|---|---|
| File / Edit / View / Insert / Format / Data / Tools / Help menu | Alt+F / E / V / I / O / D / T / H (Alt+Shift+… in other browsers) |
| Insert menu / delete menu for selected cells | Ctrl+Alt+= / Ctrl+Alt+- |
| Sheet menu | Alt+Shift+S |
| Context menu | Ctrl+Shift+\\ / Shift+F10 |
| Table column menu | Ctrl+Alt+\\ |

### Rows and columns
| Action | Shortcut |
|---|---|
| Insert rows above / columns left (rows/cols selected) | Ctrl+Alt+= |
| Insert rows below / columns right | Alt+I, R, B / Alt+I, C, O |
| Delete rows / columns | Ctrl+Alt+- (selected) / Alt+E, D, D / Alt+E, D, E |
| Hide / unhide row | Ctrl+Alt+9 / Ctrl+Shift+9 |
| Hide / unhide column | Ctrl+Alt+0 / Ctrl+Shift+0 |
| Group / ungroup | Alt+Shift+→ / Alt+Shift+← |
| Expand / collapse group | Alt+Shift+↓ / Alt+Shift+↑ |

### Formulas
| Action | Shortcut |
|---|---|
| Show all formulas | Ctrl+~ |
| Array formula | Ctrl+Shift+Enter (wraps in ARRAYFORMULA) |
| Collapse expanded array | Ctrl+E |
| Formula help show/hide; full/compact | Shift+F1; F1 |
| Absolute/relative reference | F4 (while editing) |
| Formula result preview | F9 |
| Resize formula bar | Ctrl+Up / Ctrl+Down |
| Range selection mode | F2 / Ctrl+E |

Sheets also offers **"Enable compatible spreadsheet shortcuts"** (Excel keys). Starbase Sheets: a setting with the same name that switches to GridCraft's existing Excel keymap.

## 6. Functions that Sheets has and Excel doesn't (later phases)

| Function | What | Phase |
|---|---|---|
| QUERY | SQL-like query over a range | Later (high value) |
| ARRAYFORMULA | applies a formula over ranges | P1 (map to dynamic arrays) |
| SPLIT, JOIN | text split/join | Later |
| GOOGLEFINANCE, GOOGLETRANSLATE, DETECTLANGUAGE | Google services | No |
| IMPORTRANGE | pull from another spreadsheet | P2+ (between Starbase Sheets files only) |
| IMPORTDATA, IMPORTHTML, IMPORTXML, IMPORTFEED, IMAGE | web fetch | Later (needs server proxy) |
| SPARKLINE | in-cell chart as a function | Later (GridCraft has Excel sparklines) |
| REGEXMATCH, REGEXEXTRACT, REGEXREPLACE | regex | Later |
| UNIQUE, FILTER, SORT, SORTN, FLATTEN | arrays (Excel has most; SORTN/FLATTEN Sheets-only) | Later |
| SHEET, SHEETS | sheet number / count (Sheets added 2026-02) | Later |

## 7. Sheets changes in 2026 (from Google's update posts)

| Date | Change | Starbase Sheets |
|---|---|---|
| 2026-02 | SHEET / SHEETS functions | Later |
| 2026-04 | Faster performance, doubled cell limits (now GA 2026-09) | GridCraft already handles 1,048,576 × 16,384 |
| 2026-04 → 10 | Gemini: build/edit sheets, text → tables, "Fill with Gemini", fix formula errors, cell-level citations | No (maybe Later via the local LLM) |
| 2026-06 → 07 | Import 3D bar, combo and multi-x scatter charts | follows GridCraft |
| 2026-08 | Excel tables import as Sheets tables; linked pivot tables | follows GridCraft |
| 2026-08 | "Sheets canvas": interactive mini-apps / publish canvases as links (11-09) | No |
| 2026-09 | **Manual calculation setting** (File › Settings › Calculation) | P1 setting (GridCraft has Excel's calc modes) |
| 2026-09 | Pivot: manual reorder + custom sort, better calculated-field editor; 10-26 compact layout | Later |
| 2026-09 | Custom web fonts in charts | Later |
| 2026-10 | Box-and-whisker charts; stock charts | follows GridCraft |

## 8. Deliberately not copied

Google branding and icons · Drive integration · Gemini / Explore (unless we add our own local AI later) · Apps Script / add-ons marketplace · Google Forms link · Meet · Smart chips tied to Google people/places · offline mode (we're LAN-hosted).
