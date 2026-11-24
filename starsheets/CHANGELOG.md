# Changelog

Changes that people using StarSheets would notice, newest first. Versions follow
[semantic versioning](https://semver.org): while we're below 1.0, the middle number goes up for
new features and the last one for fixes.

## [Unreleased]

### Added
- Single sign-on through your own sign-in service (Authelia, Authentik, Keycloak and other
  OpenID Connect services): a "Sign in with SSO" button, accounts made or linked on first
  sign-in, owners from a group, passwords still available. Set up with `STARSHEETS_OIDC_*`
  (docs/oidc.md).

### Fixed
- Menu Paste (Edit › Paste, or right-click › Paste) pastes what you last copied in StarSheets,
  including text copied from inside a cell; Edit › Copy and Cut also reach the clipboard now.
  Something copied in another app (or before a reload) pastes with Ctrl+V, and the menu says so
  instead of "Nothing to paste".
- "There's already data here. Do you want to replace it?" (dragging cells onto others) now
  looks like the other StarSheets dialogs.
- `DATEVALUE` of a date (not text) gives that date, as in Google Sheets, instead of #VALUE!.
- Typing in a cell with a coloured font keeps that colour while editing.
- New folder: the name is selected so you can type straight away, the folder shows first
  until it is named, it lines up with the other folders, and pressing Enter to name it no
  longer opens it.
- No more dark flashes in the grid area while a spreadsheet opens or the page reloads.
- The formula bar and cells no longer blink off and on a few times while a spreadsheet opens:
  fonts arriving from the server used to blank them for a moment each.
- Big tabs: each edit is checked with the server in well under a millisecond (it re-read the
  whole tab before, about 90 ms on a million cells and more in the browser). The regular
  whole-file check is about four times faster.
- The server no longer lists every folder on the file share every few seconds for each open
  file, which could slow down opening and listing files for everyone.
- The browser does less work on every frame (fonts and settings are only checked when
  something changed).
- Big files open about twice as fast: the browser no longer gets the whole file a second time
  from the server after reading it (unless someone changed it in between).
- Undo and redo on big files no longer recalculate the whole file in every open tab.
- Saving a big file no longer holds up other people's edits or other files while it writes.
- When the server does send a whole big file to a tab, it sends about a ninth of the data
  (compressed): much quicker away from home.
- A big spreadsheet shows "Opening …" while it is read, instead of a still page.
- Fixed: a sheet with NOW() or RAND() could make some edits take the slow way to the server
  (the check compared the clock-driven result, which differs between the tab and the server).
- Fixed: after an undo (and some other changes), saving could drop parts of a sheet StarSheets
  keeps without showing them (for example Excel-only features).

### Added (for developers)
- No compiler or clippy warnings, code formatted with GridCraft's settings (`rustfmt.toml`); a CI
  job "Format and lint" keeps it that way. `.git-blame-ignore-revs` skips the formatting commit.
- The home screen's code is split by part (`app/src/home/`); the side panels share one shape
  (`app/src/panels.rs`).
- A speed check on a big 31-tab, 2-million-cell workbook (`cargo run --release -p
  starsheets-sync --example bench`), run by CI with its table in the run's summary.
- The example Docker Compose file caps the server's memory.
- Typing, Delete or Enter while a dialog without a text box is open (Settings, the merge warning,
  "Open link?") no longer changes the cells behind it.
- Excel files: rows of ordinary 11 pt text open at Excel's height again (they were 2 px too
  tall), and fonts in files without a theme keep their link to the theme when saved.

### Added
- Right-click inside a cell (or the formula bar) while typing: Cut, Copy, Paste, Delete and
  Select all for the text.
- Share works on a new, empty spreadsheet: it is saved first, then the share box opens.
- **Settings › Style** tab: choose the toolbar's background colour (shows straight away; Cancel
  puts it back). The first of the look-and-feel settings.

### Changed
- The "Saved" text beside the title is now a clock symbol left of Files. A dot shows when it is
  saving (blue) or something isn't saved (red); hover shows who edited the file last and when,
  and where it is saved; a click opens version history.
- Other people in the file now show to the right of Share, next to your own picture.
- The sheet tab arrows (◀ ▶) move a page of whole tabs at a time and slide smoothly: ▶ brings the
  last tab you could see to the front, ◀ does the same the other way.
- Drop-down cells: only a click on the ▾ opens the list. Dragging from the ▾ or the fill handle
  now fills, selects or moves the cells, and the ▾ shows a pointing-hand cursor.
- The name box list (left of fx) looks like the other menus, shows each named range's cells, and
  has "Manage named ranges". Protected ranges no longer appear in it.

## [0.4.0] - 2026-11-23

### Added
- **Ctrl+F find bar** (top right): finds on this sheet, "n of N", arrows or Enter/Shift+Enter
  through the matches, ⋮ for the full Find and replace (Ctrl+H), ✕ or Esc to close.
- **Enter on a drop-down cell** opens its list; arrow keys and Enter choose; "Edit rule" under
  the choices opens the rule.
- **Clip** text wrapping beside Overflow and Wrap (Format › Wrapping and the toolbar), as in
  Google Sheets; kept in StarSheets' section of the file, since Excel has no such setting.
- **StarSheets' own section in your files:** what Excel has no place for (which kind of rule a
  drop-down was made with, which colours belong to its items, protected-range settings) is kept
  inside the .xlsx in a part Excel and LibreOffice keep but don't show. Files stay ordinary .xlsx.
- **Google Sheets formulas:** `SORTN`, `SORT` with TRUE/FALSE for ascending/descending and several
  sort columns, and open-ended ranges such as `A3:C` (to the bottom of the sheet). Saved .xlsx
  files spell those ranges the way Excel reads them.
- **Data validation** in the cell right-click menu, under Conditional formatting.
- **◀ ▶ buttons** after the sheet tabs when they don't all fit; the bar under the tabs is now only a
  slim sign of where you are.
- **Border colour and line style** in the borders menu (thin, medium, thick, dashed, dotted,
  double), used by the next border you pick, as in Sheets.
- **Pictures in cells:** Insert › Image › "Insert image in cell" or "over cells" opens your file
  chooser straight away; pasting a picture (Ctrl+V) puts it inside the selected cell, fitted to it.
- **Spelling suggestions:** while typing in a cell, click a word underlined in red to see up to
  five corrections, add it to your own dictionary (kept in this browser), or ignore it.
- **Insert row below / column right** in the cell right-click menu and in Insert › Rows and
  Insert › Columns.
- **Chart editor › Pie slices:** every slice is listed with its own colour box, so you can change
  several without picking them one at a time (long lists show the first 12, then "Show all").
- **Activity log** for whoever runs the server: who opened and saved what, conflicts, changes made
  outside StarSheets and failed saves, one line each with the time, in
  `.starsheets/log.txt` next to your files (and in the server's output).

### Changed
- **Formulas are coloured like a code editor** (functions, numbers, TRUE/FALSE, errors,
  operators, as well as references and text), the same in the cell, in the formula bar while
  editing and when just looking.
- Range boxes in the data validation and conditional formatting panels accept sheet names,
  `$` signs and open ranges (`'My sheet'!$A$2:A`).
- **Pictures in cells from Google Sheets files** open as pictures in the cells (they follow the
  cell's alignment); floating pictures and charts slide under frozen rows and columns.
- **Google-style formulas** (`{…}`, `SORTN`, `ARRAYFORMULA`, `QUERY`, open ranges like `A3:C`)
  leave empty cells empty in their results, as Google Sheets does, instead of showing 0.
- The in-cell editor of a Wrap cell widens with the formula before wrapping.
- A drop-down rule's ranges are merged (`A2:B28`, not `A2,A2:A28,A2:B2,B2:B28`).
- The browser tab shows only the file's name; the sheet tabs use all the room up to the quick sum.
- The conditional formatting panel's live preview edits the rule in place (no more rules
  vanishing and coming back while editing).
- **Google Sheets formulas in .xlsx files open in Excel:** formulas Excel can't read (`{A1:B1;…}`
  with ranges, `SORTN`, `QUERY`, `ARRAYFORMULA`…) are saved the way Google Sheets saves them, so
  Excel shows their last results and StarSheets gets the formulas back unchanged.
- **Big files use far less memory:** the server sends a workbook as text instead of building a
  copy of it in memory first. A 2-million-cell file took 3.3 GB on the server when opened; now
  it takes about 360 MB, and undo on a big sheet no longer copies it the same way. (Open tabs on
  an older version reload once.)
- **Ctrl+D and Ctrl+R** copy drop-downs and other data validation along, like the fill handle.
- The **borders menu** stays open while you pick borders, colours and line styles; click away to
  close it.
- **Editing a long formula in a cell:** the box wraps at the right edge of the sheet and grows
  downwards instead of hiding the end of the formula.
- **Drop-down lists in cells** (data validation) show every choice, as tall as the window allows,
  then scroll with a visible bar; they have the house menu look.
- **Open a link** asks in a house-style box ("Open this link?") instead of GridCraft's.
- **Font size and zoom** lists have the house style; the font size's ▾ sits inside its box.
- **Long dialog titles** (a long file name in Share) end in "…" instead of running under the ✕.
- **Sign-in** is now a real web form, so password managers (Keeper, Bitwarden, the browser's own)
  can fill in your username and password.
- **Chart menu** (right-click a chart) has the same padding and icons as the other menus.
- **File › Open:** renaming now opens a small Rename dialog (as in Drive) instead of a box squeezed
  into the list, and the buttons above the list have the same padding as everywhere else.

### Fixed
- **Ctrl+Enter** (Cmd+Enter on a Mac) in a cell starts a new line, as in Google Sheets, instead
  of finishing the edit.
- Long wrapped text no longer leaves an empty line above it: rows were sized with letter widths
  that were too generous, so they could get one line too many. (A row made too tall by an older
  version fits itself the next time anything in it changes.)
- A formula being edited in a bold, italic or other-font cell shows in the formula bar's plain
  font, so it is as easy to read as in the formula bar.
- Delete on empty cells no longer adds an undo step.
- Editing a drop-down no longer makes conditional formatting rules vanish and come back, and a
  rule of your own on the same cells is no longer taken for a drop-down colour.
- Google Sheets files: array formulas (ARRAYFORMULA, QUERY…) work after import instead of
  showing #SPILL! (the cells they fill are no longer blocked by Google's placeholders).
- A picture in a cell follows the cell's alignment.
- Hovering a cell with both a note and an error shows both.
- Choosing an alignment a cell already has keeps it, from the toolbar, the menu or the keys
  (it used to switch back to the default).
- Deleting the tab you're on takes you to the tab on its left.
- Deleting a sheet asks first.
- Files open in a new browser tab from the file browser, as from Google Drive.
- The all-sheets list is only as wide as its names.
- Tick boxes are bigger in their cells.
- Dragging a column or row edge is one step for undo (Ctrl+Z put back only the last pixel).
- File previews show columns A to L.
- Menus and pop-ups open instantly (they used to fade in over a fifth of a second).
- The mouse's back and forward buttons in the file browser act at once every time (they could
  wait up to a second).
- A border drawn on an edge two cells share replaces the other cell's line there.
- Emptying a linked cell (Delete, Backspace or editing the text away) now removes its link and
  its blue underline too, as in Sheets.
- Drop-down lists are only as wide as their longest choice, with the choices on the left.
- Changing a column's width refits the rows of wrapped cells in it, so a row no longer stays two
  lines tall when its text fits on one.
- The border colour button no longer draws its colour bar over a second, black bar.
- Thick and medium borders meet properly at the corners.
- New app icon and favicon.
- **Print** works in Firefox: Firefox won't let a page print a PDF shown inside it, so the
  print-ready PDF opens in a new tab with the browser's print button (Chrome prints directly).
- Double-clicking a cell to edit it follows your computer's double-click speed, so slower
  touchpad double-taps work too.
- Undoing a merge that moved a value into the top-left cell now puts the value back where it was.
- Alt+Shift+5 (strikethrough) and the other Ctrl/Alt+Shift+number shortcuts work instead of
  typing a symbol into the cell.
- A doughnut chart whose range includes a column of text no longer shrinks: the text column used
  to get an empty ring of its own that squeezed the real one.
- Undo (Ctrl+Z) no longer jumps to the top of the sheet; you stay where you were working.
- Pressing Enter in a side-panel box (a chart's data range, find, a rule's formula, the font
  size…) no longer also starts editing a cell on the sheet.
- Chart colour schemes are named after the colours they really use in your file (they said
  "Grey", "Gold" and "Light blue" for green, light blue and purple).
- Text being typed in a cell was thrown away when the connection to the server dropped and came
  back (for example when the server restarted, or a tab had been open a long time), or when
  another tab changed the same sheet. It now stays, and is saved when you finish.
- Updating the server no longer reloads every open tab; tabs only reload when the way they talk
  to the server has changed, and a cell being typed in comes back after that reload.
- That reload could also save someone's latest changes as a "(conflicted copy)" even with the
  file open in only one tab. The message now explains what happened.

## [0.3.0] - 2026-11-22

### Added
- **Home screen:** StarSheets opens on your files instead of a blank sheet, like Google Sheets'
  home. A folder tree on the left (My files, Shared with me, Recent, Starred, Bin, and for the
  server's owner Everyone's files), and your folders and spreadsheets as cards with a **preview
  picture** of each, or as a list. New spreadsheet, new folder and Upload are under New; you can
  also drop spreadsheets from your computer onto the page. Right-click for Open, Share, Star,
  Details, Rename, Make a copy, Download, Copy/Cut/Paste, Move to… and Move to bin; drag things
  onto a folder, the tree or the path to move them.
- **Everyone has their own folder** on the server, and **folders can be shared**: everything in a
  shared folder is shared too, also what's added later. Moving something into another person's
  folder makes it theirs, so StarSheets asks first.
- **Details** (right-click): type, where it is, owner, size, when it changed and who changed it,
  when you opened it, and who has access.
- **Sort** menu: sort by name, date modified or size, either direction, with folders at the top
  or mixed in with the files.
- **Stars and Recent:** star files and folders to find them quickly; Recent lists what you opened.
- **Getting around:** Back, Forward and Up buttons above your files; the mouse's back and forward
  buttons go back and forward through the folders you visited (Up and Backspace go to the folder
  above) instead of leaving the page. A **Files** button next to Share in every spreadsheet opens
  your files at that spreadsheet's folder in a new tab; the StarSheets logo goes there too.
- **Folder colours:** right-click a folder › Folder colour, 14 colours to choose from. Only you
  see the colours you pick, as in Drive.
- **Move to…** (right-click): pick a folder in a small folder tree and move everything selected
  there. Select like in a file manager: click, Ctrl+click to add or remove one, Shift+click for a
  range, Ctrl+A for all. Right-click any empty space for New spreadsheet, New folder and Paste.
- **Copy a sheet to another spreadsheet:** right-click a sheet's tab › Copy to › New spreadsheet,
  or Existing spreadsheet… (pick it from a list with search). Formats come along; anyone with the
  other spreadsheet open sees the new tab straight away.
- **Choose where files are kept:** when setting up a new server, and later from your avatar ›
  Where files are kept (the server's owner). StarSheets moves everything there (files, versions,
  bin, accounts), or switches to StarSheets files already in that folder, and restarts itself.
- **Your own bin:** each person's deleted things go to their own bin, and are deleted for good
  after 30 days. Restoring a file brings back its earlier versions and who it was shared with.

### Changed
- **Files changed outside StarSheets are safe.** If a spreadsheet that's open in StarSheets is
  changed on disk (for example in Excel over a file share), StarSheets notices within a few
  seconds and shows that version. Anything typed here that wasn't saved yet goes into a copy next
  to it ("<name> (StarSheets copy <date>)"); the outside change is never written over.
- **A spreadsheet moved or renamed while it's open** (in another tab, or by someone else) shows its
  new name and place straight away. If it's moved to the bin, StarSheets says so: changes aren't
  saved until it's restored, then saving carries on by itself.
- When a server that had files from before gets its first accounts, those files move into the
  owner's own folder (and files someone else made into theirs) once, automatically. Nothing is
  renamed or lost.
- **Protected ranges with a warning work:** after the "Heads up!" warning, OK lets you edit (what
  you started typing is kept), and "Don't show this again for 5 minutes" can be ticked. The
  warning was drawn twice each frame, so the tick undid itself and the edit was dropped.
- Every menu now has the same look (16 px either side, 32 px rows, icons), and every one-line text
  field and search bar has its text centred top to bottom.

## [0.2.0] - 2026-11-22

### Added
- **Create an account:** the owner can let people make their own account from the sign-in box
  (People › Sign-up), with an optional sign-up code to give out. It's off unless the owner turns
  it on.
- **Share** a spreadsheet like in Google Sheets: add people as viewers or editors, or let anyone
  signed in to your server open it with the link. Viewers see live changes but can't edit. Other
  people's files only show up once they're shared with you.
- **Accounts:** everyone signs in with a username and password kept on your server (no sign-in
  through other companies). The first visit makes the owner's account with a setup code from the
  server's log; the owner adds other people. Your avatar (top right) changes your name, colour and
  password.
- **See who's here:** the avatars of everyone with the file open, and their selections on the
  grid in their colour, with their name.
- **Live editing between tabs:** open the same spreadsheet in several tabs (or browsers) and every
  change shows everywhere straight away. The server keeps the file and saves it; no more
  "conflicted copy" files. Undo undoes your own changes and keeps everyone else's; if someone
  has changed the same cells since, StarSheets says so instead. Needs a proxy that passes WebSockets
  (docs/deploy.md); without one the app saves the usual way.

### Changed
- Hovering over a cell with an error (#VALUE!, #DIV/0! …) explains it in plain words, and names
  the cell to fix first when the error comes from another cell.
- A checkbox cell toggles only when you click the box itself; the rest of the cell just selects.
- The formula suggestion list never covers the cell you're typing in (it opens above when there
  isn't room below); no suggestions inside quoted text, and quoted text shows in green.
- A formula with a bracket or quotation mark missing explains what's wrong, as Sheets does, and
  offers to enter it as text (Add "'") or to keep working on it.
- Formula problems in conditional formatting and data validation are explained in plain words.
- The rename box in File › Open uses the house style.
- People's colours no longer include blue, which is how your own selection looks.
- The server is now one Rust program (`starsheets-server`) instead of a Python script, the first
  step towards live editing ([docs/live-editing.md](docs/live-editing.md)). Same files, folders,
  versions and bin; nothing to move. Versions saved within the same second are now kept in the
  right order. The Docker image no longer contains Python.
- Messages (such as editing a protected cell) appear as StarSheets dialogs with plain wording.
- Pie and doughnut charts: each slice can have its own colour (chart editor › Customise › Pie
  slices), kept in the .xlsx file; colour schemes now colour every slice.
- Settings and the colour-scale points use the house drop-down.
- Dragging the fill handle from cells with data validation extends the rule to the new cells.
- File › Settings: "Display language" is replaced by **how numbers are written** (1,234.56,
  1.234,56, 1 234,56 or 1'234.56), set by the locale; cells show and accept that format.
- Enter accepts a highlighted function suggestion ("=lef" → "=LEFT(") and editing goes on.
- Tooltips in the house style: dark grey with white text.
- The formula bar stays one line (long text runs on) until you expand it; bigger fx icon.
- No lines above and below the toolbar.
- Merging cells where only one holds a value keeps that value, with no warning.
- GridCraft's own dialogs (Insert link, Paste special, Find and replace…) have Sheets-style titles:
  top left with ✕, no grey title band.
- Dragging a sheet tab: the tab follows the pointer, the others slide aside, and it springs into
  place on release.
- Clicking anywhere on a note starts editing it.
- The sheet tab colour menu uses the same colour menu as the toolbar (custom colours included).
- Removed the Amatic SC font.
- Drop-downs in the conditional formatting and data validation panels show every choice at once;
  lists that don't fit the window scroll with a solid, always-visible scroll bar.
- The font menu shows each font's name in that font, scaled so the names look the same size.
- Right-click on a cell with a note offers Edit note and Delete note.
- Insert note writes straight into the note card (no separate box).

### Fixed
- Previewing a format on a blank sheet no longer saves an empty file (only values, formulas,
  notes, charts and pictures count as content).
- Side panels scroll when they are taller than the window, with a visible scroll bar.
- Note cards can be resized with their corner grip.
- "Today" in the date picker puts today's date in.
- Renaming a sheet tab: Enter keeps the name, Esc cancels.

- One house style for every dialog, menu, card and panel, following Google Sheets: 24 px dialog
  padding, pill-shaped buttons, 36 px fields, padded menus (docs/house-style.md).

### Added
- Spell check while typing in a cell: unknown words are underlined in red (British English word
  list from SCOWL, loaded on first edit); nothing is marked once you leave the cell.
- **Protect sheets and ranges** after Google Sheets (Data menu, and "Protect sheet" on a tab):
  protect a range or a whole sheet (with "Except certain cells"), either restricted (edits are
  refused, kept as Excel sheet protection) or with a "Heads up!" warning before editing.
- **Find and replace** after Google Sheets: all sheets, this sheet or a range; match case, whole
  cell, regular expressions, within formulas; Find, Replace, Replace all.
- **Named ranges** side panel after Google Sheets: a list, "Add a range", and a name + range
  editor (no scope or comment).
- File › Details: name, location, last saved, and how many sheets, filled cells, formulas,
  charts and notes the spreadsheet has.
- Format › Number › **Custom number formats**, after Google Sheets: a code box with Apply, a
  sample of the selected cell, and ready-made formats; the selection shows it as you type.
- **Chart editor**, after Google Sheets: a side panel with Setup (chart type, stacking, data range,
  X-axis, series, switch rows/columns) and Customise (colours, smooth lines, doughnut hole and
  rotation, chart and axis titles, series colour, trendline, data labels, legend, gridlines).
  Changes show on the chart straight away. Opens on Insert › Chart, a double-click on a chart or
  "Edit chart" in its right-click menu.
- Format › Merge cells › **Merge vertically**, and Google's "Heads up!" warning when merging would
  drop values (with "Don't show this again for 5 minutes"). Merge options that can't apply to the
  selection are greyed out.
- Text and fill colour menus like Google Sheets': a tick on the current colour, the standard
  colours, your own custom colours (remembered) with a full picker (colour square, hue slider,
  hex and RGB boxes), and a shortcut to conditional formatting.
- Notes like Google Sheets': Insert › Note, a white card beside the cell when you hover over or
  select it, edit it in place, and drag its corner to resize it. Comments are gone for now:
  notes cover what they were used for.
- A calculation preview while you type a formula, as in Google Sheets: a small bubble shows the
  result so far, even for an unfinished formula, or of just the part you select.

- **File › Settings**: your locale (how typed dates are read: 05/08 is 5 August in the UK), default
  date, time and number formats with examples, and the display language for dialogs. The default
  locale is now English (United Kingdom).
- Format › Number › **Custom date and time**, like Google Sheets': build a format from parts
  (day, month, year, hour…, each with its kinds), pick from ready-made ones, and see it on the
  selected cells as you go.
- Conditional formatting: a custom formula over whole columns, like Google's `=$A:$A=12` on
  A1:F13, now colours whole rows.

### Fixed
- The screen no longer flashes when you change a cell's font size.
- Copied cells no longer get an outline.
- Ctrl+\ (clear formatting) works on UK and other keyboards whose \ key sits next to Z.

## [0.1.0] - 2026-11-21

The first public preview. StarSheets is still early: please expect rough edges, and do tell me
about them.

### Looks and feels like Google Sheets
- Google Sheets' layout: title you can click to rename, menu bar, one-row toolbar, formula bar,
  sheet tabs with colours, drag to reorder, and right-click menus for cells, rows, columns and tabs.
- Google Sheets' sizes, fonts and keyboard shortcuts, and your own mouse pointer.
- 29 fonts, including free look-alikes for Arial, Calibri, Cambria, Comic Sans, Courier New,
  Georgia, Impact, Times New Roman, Trebuchet MS and Verdana. Each one loads the first time a sheet uses it.

### Working with data
- Conditional formatting in a side panel: all of Google's conditions (including "date is today /
  in the past week…"), preset styles and colour scales, previewed live on the sheet as you choose.
- Data validation in a side panel: drop-down lists with coloured items, text, number and date
  rules, tick boxes and custom formulas, with help text and "warn" or "reject". A date picker for
  date cells.
- Filters with Google's filter menu, links that you can type or paste, and named ranges in formula
  suggestions.
- Google Sheets formulas: `QUERY` (select, where, group by, pivot, order by, limit, label,
  format), `ARRAYFORMULA`, and `{…}` to put ranges side by side or one under another.

### Files (self-hosted)
- Your spreadsheets are ordinary .xlsx files in a folder on your server. A sheet saves itself a
  moment after each change; closing the tab saves at once.
- A file window with folders: new folder, rename, copy, cut and paste, drag to move, search, a bin
  you can restore from, and version history (the last six saves).
- Importing .xlsx and .csv files, downloading as .xlsx, PDF, HTML or CSV.

### Self-hosting
- One Docker image (`ghcr.io/domeshifter/starsheets`) with the app and its file storage.
- A password login, on by default (a random password is printed on first start if you don't set one).

[Unreleased]: https://github.com/DomeShifter/StarSheets/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/DomeShifter/StarSheets/releases/tag/v0.4.0
[0.3.0]: https://github.com/DomeShifter/StarSheets/releases/tag/v0.3.0
[0.2.0]: https://github.com/DomeShifter/StarSheets/releases/tag/v0.2.0
[0.1.0]: https://github.com/DomeShifter/StarSheets/releases/tag/v0.1.0
