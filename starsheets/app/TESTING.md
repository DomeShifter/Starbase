# Testing StarSheets with the test hook

Add `test` to the page address (`?test`, or `&test` after other parameters) and the page gets
`window.starsheets`: a way for scripts and AI agents to run commands and read the app's state
without clicking at screen positions. Without `test` it is not there. Code: `src/testhook.rs`
(the home screen's part: `src/home/test.rs`). StarDocs has the same hook (`stardocs/app/TESTING.md`).

| Call | What it does | Returns |
|---|---|---|
| `starsheets.run(id, params)` | Runs what a menu or toolbar entry runs: a GridCraft command (`home.bold`, `home.insertSheet`, `edit.undo` …), a `panel:` id (our panels and file actions) or a `ui:` dialog; the ids are the ones in `src/shell.rs`. On the home screen: `home:folder {path}`, `home:search {text}`, `home:refresh`, `home:recent`, `home:starred`, `home:shared`, `home:bin`. `params` is a JSON string or an object; leave it out for `{}`. | request number |
| `starsheets.state()` | Takes a snapshot (below). | request number |
| `starsheets.type(text)` | Enters `text` into the active cell, as typing it and pressing Enter would, but stays on the cell. `=` starts a formula. | request number |
| `starsheets.key(name)` | One key: Enter, shift+Enter, Tab, shift+Tab, Up/Down/Left/Right (with shift+ to extend, ctrl+ to jump), ctrl+Home, Delete, Backspace, ctrl+z, ctrl+y, ctrl+a, ctrl+b, ctrl+i, ctrl+u. | request number |
| `starsheets.wait(n)` | Waits (up to 10 s) for request `n`'s answer. | promise of a JSON string |

Requests are answered at the start of the next frame. Each answer is `{"ok": value}` or
`{"err": "message"}`; it also lands in `starsheets.last`, with its number in `starsheets.done`.
A `panel:` or `ui:` id answers `{"opened": id}`: the panel shows on the frame after.

**The snapshot.** On a spreadsheet: `screen: "sheet"`, `file` (its name), `fileId`, `folder`,
`sheet`, `sheetIndex`, `sheets`, `active` (A1), `selection` (`A1:B3,D4`), `cell` (`value` as
shown, `number`, `input`, `formula` or null), `format` (`bold`, `italic`, `underline`, `strike`,
`font`, `size`, `numberFormat`, `hAlign`, `wrap`), `saved` and `saveStatus` (the words the save
symbol shows), `dirty`, `canUndo`, `canRedo`, `live`, `peers` (other people in the file) and
`people`, `compact`. On the home screen: `screen: "home"` and `home` with `view`, `loading`,
`search`, `error`, and the `folders` and `files` it lists, in the order shown.

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
