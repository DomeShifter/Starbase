# StarDocs vs Google Docs: gap list

_Research only, 2026-11-24. Sources: `stardocs/app/src/shell.rs` (MENUS + toolbar GROUPS), `stardocs/app/TESTING.md`, `stardocs/app/src/{server,share}.rs`, and WordCraft's command registry (`upstream/wordcraft/crates/engine/src/cmd/**`, every `CommandSpec::new`). WordCraft has **673** registered commands (more than the ~420 we guessed); StarDocs' menus and toolbar reach about **95** of them._

**Key to "StarDocs today":**
- **Has**: in a StarDocs menu or on the toolbar.
- **Engine-only**: a WordCraft command exists but no StarDocs menu entry. A few are reachable today through WordCraft's own right-click menu on the page (`ui-egui/src/canvas.rs`, table/comment/spelling items). I didn't check that this menu still shows inside StarDocs' chrome.
- **Missing**: no command in the engine.

**Size:** S = a menu entry or small dialog (≤1 day); M = new UI or a server piece (days); L = a new subsystem (weeks).

## 1. Feature table

| Feature | Google Docs: where | StarDocs today | How common | Size | Notes (WordCraft command / crate) |
|---|---|---|---|---|---|
| New / open / rename | File | Has | High | – | `file.new`, `file.open`, `stardocs.rename` |
| Make a copy | File › Make a copy | Missing (server) | High | S | Needs a server `POST api/files/<id>/copy`; the home screen may already do it |
| Share (people, link access) | Share button | Has | High | – | `share.rs` (owner/edit/view, link access). No **commenter** role, which Docs has |
| Commenter role | Share dialog | Missing | Med | S–M | Server role and view-only-plus-comments mode |
| Download .docx / .pdf | File › Download | Has | High | – | `stardocs.download`, `file.exportPdf` |
| Download .odt / .rtf / .txt / .html / .md / .epub | File › Download | Engine-only (not epub) | Med | S | The `formats` crate has odt, rtf, txt, html, markdown, latex and mht writers; `file.saveAs` with an extension. No EPUB |
| Import other formats on open | File › Open | Engine (docx/odt/rtf/txt/md…) | Med | S | Wire the home-screen upload to accept them |
| Email as attachment / email collaborators | File › Email | Missing | Low | M | Needs mail on the server; skip |
| Version history (list, name, restore, copy) | File › Version history (Ctrl+Alt+Shift+H) | Engine-only, in memory | High | M | `file.versions` keeps versions per session only. The server needs to store versions (snapshots per save, named versions) |
| See who changed what / show editors | Version history | Missing | Med | M | Depends on server version history; tracked changes do part of it |
| Autosave + "Saved" | Title row | Has | High | – | `server.rs` |
| Live co-editing (cursors, live edits) | Built in | Missing (one writer; a 409 makes a copy) | High | L | StarSheets has a live-sync Rust server; a text CRDT/OT for a WordCraft doc is a big job |
| Offline | File › Make available offline | Missing | Low–Med | L | PWA + local queue; low value on a home server |
| Page setup (size, orientation, margins) | File › Page setup | Has | Med | – | `ui.dialog:pageSetup` |
| Page colour | File › Page setup | Engine-only | Low | S | `design.pageColor` |
| Pageless mode | File › Page setup / Format | Engine-only (close) | Med | S–M | `view.webLayout` (Word's Web Layout) is the nearest thing; needs a Docs-style text-width choice and to hide headers/footers |
| Print | File › Print | Has | High | – | `file.print` |
| Details / document properties | File › Details | Engine-only | Low | S | `file.properties`, `file.info` |
| Language | File › Language | Engine-only | Low | S | `review.language` |
| Undo / redo / cut / copy / paste / paste plain | Edit | Has | High | – | |
| Find and replace (match case, regex) | Edit › Find and replace | Has | High | – | `ui.dialog:replace`; `edit.advancedFind` exists too |
| Find (Ctrl+F bar) | Ctrl+F | Check | High | S | `edit.find`. Make sure Ctrl+F opens a find bar, not only Ctrl+H |
| Paint format | Toolbar | Has | High | – | `edit.formatPainter` |
| Delete (selection) | Edit › Delete | Has (key) | – | – | |
| Editing / Suggesting / **Viewing** modes | Mode box / View › Mode | Partly: Editing and Suggesting | High | S | Add a **Viewing** mode (read-only + accepted view); `server.view_only()` exists |
| Show ruler, outline, non-printing characters, full screen, zoom | View | Has | Med | – | |
| Show section breaks / print layout | View | Has (print layout) | Low | – | `view.printLayout` |
| Text: bold, italic, underline, strike, super/sub, small caps | Format › Text | Has | High | – | |
| Text size up/down (Ctrl+Shift+. / ,) | Format › Text › Size | Engine-only | Med | S | `format.growFont1` / `format.shrinkFont1`, plus keys |
| Capitalization (lower / UPPER / Title) | Format › Text › Capitalization | Engine-only | Med | S | `format.changeCase` |
| Font, size, colour, highlight | Toolbar | Has | High | – | Palette is the first two rows only; Docs has a full 10×8 grid + custom |
| Paragraph styles Normal/Title/Subtitle/H1–H3 | Format › Paragraph styles | Has | High | – | |
| Headings 4–6 | Format › Paragraph styles | Engine-only | Med | S | `para.style` with "Heading 4–6"; menu and toolbar list only go to H3 |
| Update style to match / save as default styles | Format › Paragraph styles › Options | Engine-only | Med | S | `styles.updateToMatch`, `design.setDefault` |
| Align, indent, indentation options | Format › Align & indent | Has | High | – | |
| Line spacing presets + custom | Format › Line & paragraph spacing | Has | High | – | |
| Add/remove space before/after paragraph | Format › Line & paragraph spacing | Engine-only | Med | S | `para.addSpaceBefore`, `para.removeSpaceAfter` |
| Keep with next / keep lines together | Format › Line & paragraph spacing | Engine-only | Low | S | `para.keepNext`, `para.keepLines` |
| Columns (1/2/3 + options) | Format › Columns | Has | Low | – | `layout.columns` |
| Bulleted / numbered lists | Format › Bullets & numbering | Has | High | – | |
| List presets (bullet glyphs, numbering styles), restart numbering, list options | Format › Bullets & numbering | Engine-only | High | S | `para.bullets {kind}`, `para.defineBullet`, `para.defineNumber`, `para.restartNumbering`, `para.setNumberingValue`, `list.multilevelPreset` |
| **Checklist** (tickable, strikes through when ticked) | Format › Bullets › Checklist (Ctrl+Shift+9) | Missing | High | M | No checklist in the engine (catalog mentions none). A ☐ bullet glyph is possible, but clicking to tick needs engine work (Word's check-box content control `developer.checkBox` + `developer.toggleCheckBox` is a possible base) |
| Headers and footers (incl. different first page, odd/even) | Insert › Headers & footers; Format | Has (basic) / Engine-only (options) | Med | S | `layout.differentFirstPage`, `layout.differentOddEven`, `insert.removeHeader/Footer` |
| Page numbers (position, start at) | Insert › Page numbers | Has (basic) / Engine-only (format) | Med | S | `layout.pageNumberFormat` |
| Page count field | Insert › Page numbers › Page count | Engine-only | Low | S | `insert.field` (NUMPAGES) |
| Watermark (image/text) | Insert › Watermark | Engine-only | Low | S | `design.watermark` |
| Section break (next page / continuous) | Insert › Break | Engine-only | Med | S | `layout.break`. Only page break is in the menu now |
| Column break | Insert › Break | Engine-only | Low | S | `text.columnBreak` |
| Table insert (grid) | Insert › Table | Has | High | – | |
| Table: insert/delete row/column, merge/split, delete table | Format › Table / right-click | Engine-only (maybe right-click) | High | S | `table.insertRowAbove/Below`, `table.insertColumnLeft/Right`, `table.deleteRow/Column/Table`, `table.merge`, `table.split`. A **Table** sub-menu under Format is the single biggest cheap win |
| Table properties (borders, cell colour, column width, alignment, pin header rows, distribute) | Format › Table › Table properties | Engine-only | Med | S | `table.properties`, `table.borders`, `table.shading`, `table.repeatHeader`, `table.distributeRows/Columns`, `table.cellAlign` |
| Sort table | Format › Table › Sort | Engine-only | Low | S | `table.sort` |
| Table templates | Insert › Table › Table templates | Partly (`table.style`, `table.quick`) | Low | S–M | Word-style table styles, not Docs' tracker templates |
| Image insert (upload / by URL / Drive / camera) | Insert › Image | Has (upload) | High | S for URL | By URL needs a fetch proxy on the server (CORS) |
| Image options: wrap text, alt text, crop, size/rotate, recolour, transparency, replace, reset | Format › Image; image toolbar | Engine-only | High | S–M | `arrange.wrap`, `picture.altText`, `picture.crop`, `picture.size`, `arrange.rotate`, `picture.color`, `picture.transparency`, `picture.change`, `picture.reset`. Needs an image toolbar/sub-menu when a picture is selected |
| Drawing (shapes, lines, text boxes) | Insert › Drawing | Engine-only | Med | M | `insert.shape`, `insert.textBox`, `insert.canvas`, `arrange.*`. Docs has a separate drawing canvas; we'd expose Word-style shapes |
| Chart (from Sheets) | Insert › Chart | Engine-only (own data) | Low–Med | M | `insert.chart`; linking to a StarSheets range is the Docs-like version, a later job |
| Horizontal line, page break, special characters, equation, bookmark, TOC | Insert | Has | Med | – | |
| Emoji picker | Insert › Emoji | Missing | Med | S | Typed emoji work if the font has them; a picker is UI only |
| Link (with preview chips) | Insert › Link | Has (basic) | High | S | `ui.dialog:link`. Docs shows a hover card with Copy/Edit/Remove; `insert.removeLink` exists |
| Footnote | Insert › Footnote | Has | Med | – | Endnotes are engine-only (`references.endnote`), not in Docs either |
| Table of contents (update, links vs page numbers) | Insert › Table of contents | Has (insert) / Engine-only (update/remove) | Med | S | `references.updateToc`, `references.removeToc` |
| Smart chips: @people, @date, @file, dropdowns, placeholders | Insert › Smart chips; type `@` | Missing | Med–High | M | Date: `insert.dateField`/`developer.datePicker` are a base. Dropdown: `developer.dropDown`. @person needs the server user list (`api/users` exists) |
| Building blocks / templates (meeting notes, email draft) | Insert › Building blocks; File › New › From template | Engine-only (Word templates) | Med | M | `file.newFromTemplate`, `insert.quickParts`. Needs a Starbase template gallery |
| Document tabs (and sub-tabs) | Left panel, Insert › Tab | Missing | Med | L | Not in Word's model; it would need its own file-format extension |
| Comments: add, reply, resolve, edit, delete | Insert › Comment; comment cards | Has (add + pane) / reply etc. maybe through pane | High | S | `review.reply`, `review.resolveComment`, `review.editComment`, `review.deleteComment` exist; make sure the card buttons show in StarDocs |
| @mention + assign a comment, notifications | Comment box | Missing | Med | M | Needs server users + notifications (StarSheets has accounts) |
| Comment history / emoji reactions | Comments | Missing | Low | M | Skip for now |
| Suggesting: accept/reject one, next/previous | Tools / suggestion cards | Partly: accept/reject all | High | S | `review.accept`, `review.reject`, `review.nextChange`, `review.previousChange` |
| Spelling and grammar | Tools | Has | High | – | `review.spelling`; spell-as-you-type `review.proofing` |
| Personal dictionary | Tools › Spelling › Personal dictionary | Engine-only | Med | S | `review.addToDictionary` (right-click). A manage dialog is small |
| Word count (+ show while typing) | Tools › Word count | Has | High | – | |
| Dictionary / define word | Tools › Dictionary | Missing (thesaurus only) | Low | M | `review.thesaurus` exists; offline definitions need a word list |
| Translate document | Tools › Translate document | Missing | Low | L | Cloud service; could use the local LLM later |
| Voice typing | Tools › Voice typing (Ctrl+Shift+S) | Missing | Med | M | Browser Web Speech API (Chrome only, sends audio to Google) or local Whisper; catalog lists `tools.dictate` but it isn't built |
| Compare documents | Tools › Compare documents | Engine-only | Low | S | `review.compare` |
| Citations (APA/MLA/Chicago) | Tools › Citations | Engine-only | Med (students) | S–M | `references.citation`, `references.bibliography`, `references.citationStyle`, `references.sources`; there's a `zotero` crate too |
| Linked objects | Tools › Linked objects | Missing | Low | – | Skip |
| Preferences (smart quotes, auto-capitalize, auto-detect links/lists, substitutions) | Tools › Preferences | Engine-only | Med | S | `tools.autocorrect` and the AutoCorrect dialog; check the defaults match Docs (smart quotes on, Markdown off) |
| Markdown autodetect (`#`, `**`, `-`) | Tools › Preferences | Check | Med | S | Check what `tools.autocorrect` covers; AutoFormat-as-you-type for `- ` / `1. ` lists is the common case |
| Accessibility (screen reader, braille) | Tools › Accessibility | Engine-only (checker) | Low | L | `file.accessibility`; real screen-reader support on an egui canvas is hard |
| Read aloud | (Chrome extension, not Docs) | Has | Low | – | `review.readAloud` |
| Explore / Gemini / "Help me write" | Tools › Explore; side panel | Missing | Low | – | Don't build (cloud). The local LLM could fill this later |
| Extensions: add-ons, Apps Script | Extensions | Missing | Low | – | Don't build; `window.wordcraft` API exists for scripts |
| Help: search the menus, keyboard shortcuts | Help | Has | Med | – | `tools.search`, `stardocs.shortcuts` |
| Document outline (headings, click to go) | View › Show outline | Has | High | – | `view.navigationPane` |
| Right-click context menu | Right-click | Maybe (WordCraft's) | High | S | Check it shows in StarDocs and use Docs' items/wording |
| Link to heading / bookmark | Insert › Link › Headings and bookmarks | Engine-only | Med | S | `link.targets`; add to the link dialog |
| Copy link to heading | Outline / heading menu | Missing | Low | S | Needs a URL fragment that jumps to a bookmark |
| Drop cap, text effects, WordArt, SmartArt, mail merge, macros, content controls, indexes, fields | (not in Docs) | Engine-only | – | – | Word features, not Google Docs ones; leave out of the menus |

## 2. Top 15 to build next (value ÷ size)

**Cheap wins: the engine has it, it just needs a menu entry or small dialog (S):**
1. **Format › Table sub-menu + right-click table items**: insert/delete rows and columns, merge/split, delete table, table properties, pin header row (`table.*`). Everyday users hit this as soon as they make a table.
2. **Image options**: wrap text, alt text, crop, size and rotate, replace and reset image, shown in a sub-menu and in a small toolbar when an image is selected (`arrange.wrap`, `picture.*`).
3. **List options**: bullet and numbering styles, restart numbering, set the start number (`para.bullets {kind}`, `para.defineNumber`, `para.restartNumbering`, `para.setNumberingValue`).
4. **Suggestions one at a time**: accept/reject this one, next/previous (`review.accept/reject/nextChange/previousChange`), plus a **Viewing** mode in the mode box.
5. **Download as .odt, .rtf, .txt, .html, .md** (`file.saveAs` through the `formats` crate).
6. **Capitalization + text size up/down**, with Docs' keys Ctrl+Shift+. and Ctrl+Shift+, (`format.changeCase`, `format.growFont1/shrinkFont1`).
7. **Headings 4–6** + "Update style to match" + "Save as my default styles" (`para.style`, `styles.updateToMatch`, `design.setDefault`).
8. **Insert › Break**: section break (next page / continuous) and column break (`layout.break`, `text.columnBreak`); **headers/footers options**: different first page, odd/even, page-number format (`layout.*`).
9. **Paragraph spacing quick items**: add/remove space before/after, keep with next, keep lines together (`para.addSpaceBefore`, `para.removeSpaceAfter`, `para.keepNext`, `para.keepLines`).
10. **Comment cards complete**: reply, resolve, edit, delete reachable in StarDocs (`review.reply/resolveComment/editComment/deleteComment`); TOC update/remove (`references.updateToc/removeToc`); link to a heading or bookmark (`link.targets`).
11. **Citations** menu for students (`references.citation/bibliography/citationStyle/sources`).

**Medium jobs (M), high value:**
12. **Version history kept on the server**: snapshots on save, named versions, restore, make a copy. The engine side (`file.versions`) exists in memory only.
13. **Checklists**: tick boxes that strike through, Ctrl+Shift+9. Needs engine work; content-control check boxes are a possible base.
14. **Smart chips, first set**: `@date` (date picker), `@person` (server users), dropdown chip. Engine base: `insert.dateField`, `developer.datePicker`, `developer.dropDown`.
15. **Make a copy** (File menu + server) and a **commenter** share role.

_After these, the next big job is **live co-editing** (L). It's the main thing Docs users expect and we lack, but it's far bigger than all fifteen above put together; plan it as its own project, building on StarSheets' live-sync server._

## 3. Deliberately NOT building (for now)

| Item | Why |
|---|---|
| Explore, Gemini / "Help me write", Translate document, Dictionary lookups | Google-cloud services. If we want AI later, use the local LLM (VM211) as its own project |
| Extensions: add-ons and Apps Script | Huge, and rarely used; `window.wordcraft` already gives a script API |
| Document tabs | Not in the .docx model; would need our own format extension. Wait for users to ask |
| Offline mode | It's a home server, and the work is large (PWA + sync) |
| Email as attachment / email collaborators | Needs mail; Share + link covers it |
| Calendar-event, place, file, time-tracker and building-block chips | Tied to Google Calendar/Maps/Drive; only date, person and dropdown chips make sense here |
| Comment emoji reactions, comment history, assigning comments | Low use; do them after @mentions |
| Docs-style separate drawing canvas | Expose Word-style shapes/text boxes later instead; low use |
| Voice typing (for now) | Web Speech only works in Chrome and sends audio to Google; local Whisper is a medium job for a less common feature. Revisit later |
| Word-only features (WordArt, SmartArt, mail merge, macros, content controls, fields, indexes, drop caps, Developer tab) | Not in Google Docs; showing them would make StarDocs look like Word, not Docs. Keep them engine-only |
| Full screen-reader support | Needs an accessibility layer for an egui canvas: large. Track it separately as an accessibility goal |

## Sources

- Google Docs keyboard shortcuts: https://support.google.com/docs/answer/179738
- Smart chips: https://support.google.com/docs/answer/10710316
- Pages vs pageless: https://support.google.com/docs/answer/11528737
- Version history: https://support.google.com/docs/answer/190843
- Download a file: https://support.google.com/docs/answer/49114
- Document tabs: https://support.google.com/docs/answer/15499791 (found by search; snippet only)
- Tabs announcement: https://workspaceupdates.googleblog.com/2024/10/tabs-in-google-docs.html
- I added details I couldn't fetch, from general knowledge of Docs' current menus: the Download list (docx, odt, rtf, pdf, txt, html, epub, md), checklists, Insert › Emoji/Watermark/Break, and Tools › Voice typing/Dictionary/Translate/Citations/Preferences. Check them against a live Docs menu before quoting them.
