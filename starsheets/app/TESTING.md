# Testing Starbase Sheets with the test hook

Add `test` to the page address (`?test`, or `&test` after other parameters) and the page gets
`window.starsheets`: a way for scripts and AI agents to run commands and read the app's state
without clicking at screen positions. Without `test` it is not there. Code: `src/testhook.rs`
(the home screen's part: `src/home/test.rs`). Starbase Docs has the same hook (`stardocs/app/TESTING.md`).

| Call | What it does | Returns |
|---|---|---|
| `starsheets.run(id, params)` | Runs what a menu or toolbar entry runs: a GridCraft command (`home.bold`, `home.insertSheet`, `edit.undo` …), a `panel:` id (our panels and file actions) or a `ui:` dialog; the ids are the ones in `src/shell.rs`. Entries worked out from the active cell (`panel:sortSheet {desc}`, `panel:freeze {rows|cols: n or "current"}`, `panel:moveRowUp/Down`, `panel:moveColumnLeft/Right`, `panel:randomise {seed?}`, `panel:trimWhitespace`) run like commands and answer `{"done": id}` or an error. `panel:alternatingColours` with parameters (`{range?, preset?: 0-7, header?, footer?}` or `{remove: true}`) applies them instead of opening the panel; `panel:smartFill` accepts Smart Fill at the active cell. On the home screen: `home:folder {path}`, `home:search {text}`, `home:refresh`, `home:recent`, `home:starred`, `home:shared`, `home:bin`, the Photos commands below, and `home:opens {id}` (what opening that file would do, without opening it: `{"opens": "editor" | "preview" | "download", "url"}`). `params` is a JSON string or an object; leave it out for `{}`. | request number |
| `starsheets.state()` | Takes a snapshot (below). | request number |
| `starsheets.type(text)` | Enters `text` into the active cell, as typing it and pressing Enter would, but stays on the cell. `=` starts a formula. | request number |
| `starsheets.key(name)` | One key: Enter, shift+Enter, Tab, shift+Tab, Up/Down/Left/Right (with shift+ to extend, ctrl+ to jump), ctrl+Home, Delete, Backspace, ctrl+z, ctrl+y, ctrl+a, ctrl+b, ctrl+i, ctrl+u. | request number |
| `starsheets.wait(n)` | Waits (up to 10 s) for request `n`'s answer. | promise of a JSON string |

Requests are answered at the start of the next frame. Each answer is `{"ok": value}` or
`{"err": "message"}`; it also lands in `starsheets.last`, with its number in `starsheets.done`.
A `panel:` or `ui:` id answers `{"opened": id}`: the panel shows on the frame after.

**The snapshot.** On a spreadsheet: `screen: "sheet"`, `file` (its name), `fileId`, `folder`,
`sheet`, `sheetIndex`, `sheets`, `active` (A1), `selection` (`A1:B3,D4`), `freeze` (`{rows, cols}` or null), `smartFill` (`{at, cells: [[A1, text]…]}` the suggestion for the active cell, or null), `alternating` (each banding: `{range, header, band1, band2, footer}`), `pivot` (the pivot table the active cell is in, as GridCraft stores it, or null), `imagesReady` (IMAGE addresses on the sheet whose pictures arrived), `cell` (`value` as
shown, `number`, `input`, `formula` or null, `note` text or null, `comment` `{author, text, replies, resolved}` or null, `validation` `{kind, items, multiple, display, itemColors}` or null), `format` (`bold`, `italic`, `underline`, `strike`,
`font`, `size`, `numberFormat`, `hAlign`, `wrap`), `saved` and `saveStatus` (the words the save
symbol shows), `dirty`, `canUndo`, `canRedo`, `live`, `peers` (other people in the file) and
`people`, `compact`. On the home screen: `screen: "home"` and `home` with `view`, `loading`,
`search`, `error`, and the `folders` and `files` it lists, in the order shown (each file with its `kind`: sheet, doc, image, audio, video, pdf, text or other).

## With ff-shot

A headless-Firefox screenshot tool (ff-shot: `ff-shot.py URL OUT.png --wait SECS --steps "…"`)
runs `js:EXPR` steps in the page and prints each result as `js: …`; it waits for promises. Steps
are split on `;`, so a `js:` step must be one expression with no semicolons: chain with commas,
and wrap a call in `starsheets.wait(...)` to get its answer printed.

```sh
ff-shot.py "$BASE/?new=1&test" /tmp/sum.png --wait 12 --steps \
  "js:starsheets.wait(starsheets.type('2')).then(() => starsheets.wait(starsheets.key('Enter')));\
js:starsheets.wait(starsheets.type('=A1*21'));\
js:starsheets.wait(starsheets.state()).then(s => (s = JSON.parse(s).ok, s.cell.value === '42' ? 'PASS' : 'FAIL ' + JSON.stringify(s)))"
```

Expected output: two `js: "{\"ok\":…}"` lines, then `js: "PASS"`. An answer like
`{"err":"unknown command: home.bould"}` names what went wrong.

## Single sign-on sessions without waiting

`STARBASE_OIDC_SESSION_HOURS` is at least 1, so to test renewal (docs/oidc.md, Sessions) edit
the server's `.starsheets/sessions.json` instead (with `jq`, while it runs; each session is keyed
by a hash of its cookie). Its `expires` (seconds since 1970) set to now − 1 ends a session: the
open page's next request answers 401 with `"renew": true` and the page renews at a quiet moment.
Set to now + 60 before loading the page, the page renews within a minute without any request
failing. Moving `made` back 11 minutes makes a fresh sign-in too old to set a password or link.

**Photos** ("View as photos", `src/home/photos.rs` and `viewer.rs`): `home:photos {on}` flips the
switch; `home:photos:enable` turns Photos on for the folder shown (as the "Turn on" button does);
`home:photos:sub {on}` and `home:photos:order {newest}` change the listing; `home:photos:open {index}`
opens the viewer; `home:photos:key {key}` with `next`, `prev`, `zoomin`, `zoomout`, `fit`, `play`,
`pause` or `close`; `home:photos:slideshow {every, shuffle, play}`. The snapshot's `home.photos` has
`on`, `shown`, `enabled`, `canEnable`, `count`, `waiting`, `textures` (`count`, `bytes`, `budget`,
`asking`), the first 60 `items` (`thumb`/`large`/`original`: whether that texture is loaded) and
`viewer` (`index`, `zoom`, `playing`, `shuffle`, `every`, `showing`: which size is on screen).
`scripts/photos-grid-smoke.sh` runs all of it in a browser.

**Photo editor** (`src/home/photo_edit.rs`, over the viewer): `home:photos:edit {open}` opens or
closes it on the picture shown; `home:photos:edit:<what>` drives it: `tab {tab}` (crop, adjust,
filters), `slider {key, value}`, `set {…edit fields…}`, `aspect {name}`, `rotate`, `flip`,
`undo`, `redo`, `reset`, `compare {on}`, `cancel`, `save`, `copy {strip_location, max_side}` and
`answer {yes}` (the open question). The snapshot's `home.photos.photo_edit` has `open`, `tab`,
`edits`, `dirty`, `writable`, `whyNot`, `foreign`, `ask`, `error`, `message`, `canUndo`,
`canRedo`, `preview` (none, coarse, full), `renderMs` (the last preview render), `renders` and
`uploads` (texture uploads, which only grow when the picture changes).
`scripts/photo-edit-smoke.sh` runs it in a browser.

The avatar menu's dialogs open with `starsheets.run("account:profile")` (also `account:password`,
`account:people`), on either screen.

**Signed out and the account menu.** Every snapshot has `account`: `shown` (Drive or a file may
be drawn), `signedIn`, `user`, `signInPage` (the sign-in page is up), `menuOpen` (the avatar's
account menu) and `next` (where signing in goes on to, from `?next=`). Signed out the snapshot is
`screen: "signin"` with only `account`. `starsheets.run("account:menu")` opens the account menu
as clicking the avatar does.
