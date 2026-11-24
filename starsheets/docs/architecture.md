# Architecture

```
browser
└─ app (crate `starsheets`, WebAssembly via trunk)
   ├─ web.rs     eframe web runner, file open/download, preferences   (adapted from GridCraft)
   ├─ shell.rs   title row, menu bar, toolbar: data tables → command ids
   ├─ theme.rs   Sheets palette installed over GridCraft's tokens
   ├─ panels.rs  side panels (formatting, validation, charts, names, protection): one at a time
   ├─ home/      home screen: mod.rs (state, loading, navigation), sidebar, items (cards and
   │             list), actions (menus, Move to, Details), input (drag, keys), bin
   ├─ live.rs    live editing with the server (crates/sync, crates/server)
   └─ GridCraft  SheetApp (ui-egui): formula bar, grid, sheet tabs, dialogs
                 Session (engine): every action is a command `Session::execute(id, params)`
                 calc, formula, functions, model, xlsx, chart …
```

Each frame, `shell.ui` adds its top panels first. Then `SheetApp::ui` draws the formula bar, tabs and grid below them. GridCraft's own title bar, File tab and ribbon are turned off through `Chrome`.

## Who gets the keyboard

GridCraft's grid takes every key when no text box has the keyboard, and our dialogs are drawn
after the grid. `app/src/input.rs` decides, just before the grid draws, whether the grid gets
this frame's keys:

| Situation | The grid… |
|---|---|
| A box let go of the keyboard this frame (Enter/Tab in a panel, menu or toolbar box) | doesn't see the keys: they were the box's |
| A house dialog was showing last frame | doesn't see the keys: they go to the dialogs drawn after it; if the dialog had closed, they are replayed to the grid next frame |
| Anything else | gets the keys |

Every `house::dialog` marks itself, so a new dialog needs no extra code. Clicks behind a dialog
are already blocked by egui's Modal. Widget-specific keys that must win before the grid (spelling
suggestions' Esc, the find bar, Delete on drop-down cells) are handled just before this check in
`web.rs`.

Later phases add a small server (Rust, same engine) for the file list, autosave, version history and sharing. Design notes for that go here when phase 2 starts.
