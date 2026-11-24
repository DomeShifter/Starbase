# Testing StarDocs with the test hook

Add `test` to the page address (`?test`, or `&test` after other parameters) and the page gets
`window.stardocs`: a way for scripts and AI agents to run commands and read the app's state
without clicking at screen positions. Without `test` it is not there. Code: `src/testhook.rs`.

| Call | What it does | Returns |
|---|---|---|
| `stardocs.run(id, params)` | Runs a command: a WordCraft id (`format.bold`, `insert.table` …), `ui.dialog:<name>`, or a StarDocs menu id (`stardocs.*`). `params` is a JSON string or an object; leave it out for `{}`. | request number |
| `stardocs.state()` | Takes a snapshot: `shell` (what the window shows), `format`, `view`, `words`, `text` (first 100 KB, `textTruncated`), `dirty`, `status`. | request number |
| `stardocs.type(text)` | Types text at the caret (`text.insert`). | request number |
| `stardocs.key(name)` | One key: Enter, Tab, Backspace, Delete, Left/Right/Up/Down, Home, End, ctrl+Home, ctrl+End. | request number |
| `stardocs.wait(n)` | Waits (up to 10 s) for request `n`'s answer. | promise of a JSON string |

Requests are answered at the start of the next frame. Each answer is `{"ok": value}` or
`{"err": "message"}`; it also lands in `stardocs.last`, with its number in `stardocs.done`.

## With ff-shot

`~/.claude/tools/ff-shot.py URL OUT.png --wait SECS --steps "…"` runs `js:EXPR` steps in the page
and prints each result as `js: …`; it waits for promises. Steps are split on `;`, so a `js:` step
must be one expression with no semicolons: chain with commas, and wrap a call in
`stardocs.wait(...)` to get its answer printed.

```sh
ff-shot.py "$BASE/docs/?new=1&test" /tmp/table.png --wait 12 --steps \
  "js:stardocs.wait(stardocs.run('insert.table', {rows: 2, cols: 3}));\
js:stardocs.wait(stardocs.type('Hello'));\
js:stardocs.wait(stardocs.state()).then(s => (s = JSON.parse(s).ok, s.text.includes('Hello') && s.dirty ? 'PASS' : 'FAIL ' + JSON.stringify(s)))"
```

Expected output: two `js: "{\"ok\":…}"` lines, then `js: "PASS"`, and a screenshot showing the
table. An answer like `{"err":"unknown command `insert.tabel`"}` names what went wrong.
