# Live editing

Goal: open the same spreadsheet in several tabs, or share it with other people, and see every
change everywhere as it happens, like Google Sheets. This is a core feature, so the rest of
StarSheets adapts to it rather than the other way round.

## How it works

| Part | What it does |
|---|---|
| Server (`crates/server`) | Holds the true copy of each open file and runs the same GridCraft engine as the browser. Takes changes in the order they arrive, applies them, numbers them and sends them to every tab. Saves the .xlsx itself, a couple of seconds after things go quiet |
| Tab | Shows your own change at once and sends it to the server. When someone else's change arrives before yours is confirmed, it goes back to the last confirmed copy, applies theirs and then yours again (cheap: copies of the workbook are shared, not duplicated) |
| A change | One GridCraft command with its sheet, selection and settings. Only commands that change the file are sent; scrolling, zoom and your selection stay in your tab |
| Safety checks | Every so often the server sends a fingerprint of the file; a tab that doesn't match reloads its copy. Tabs and server must run the same build, or the tab reloads |

Why this design, and what was rejected: [decisions.md](decisions.md) (2026-11-22).

## Rules

- **Undo** only undoes your own changes. If someone has since changed the same cells, StarSheets
  says so instead of undoing over their work.
- **NOW(), RAND() and similar** are worked out in each tab, as in Google Sheets.
- **Accounts** are local to your server (username and password, hashed with Argon2). No sign-in
  through Google or other companies. The first account (the owner's) is made with a setup code
  from the server's log, so nobody else can claim a new server first; the owner adds people.
- **Presence:** each tab sends where it is (sheet and selection) a few times a second at most;
  others draw it in that person's colour, with their name for a few seconds after it moves.
- **Sharing** per file: owner, can edit, can view, and "anyone with the link" (anyone signed in
  to this server who has it). A new file belongs to whoever made it; files from before sharing
  belong to the server's owner, who runs the server and can open and manage every file. Other
  people see only files that are theirs or shared with them, and can't change folders or the bin.
  A viewer's tab refuses edits (GridCraft's `read_only`), and the server refuses them too.
- Changes from other people never go through the clipboard, files on their computer or macros:
  a paste carries its own contents, and the server only accepts commands on an allow-list.
- **Who is who** (protocol 8): the server names each connection; a tab can't speak, edit or
  undo as another tab. Access is checked again every 30 s and at once after a sharing change:
  someone who lost access is disconnected, someone made a viewer (or an editor) gets the file
  again with the new role.
- **Size limits:** a tab's message may be at most 16 MB as sent and 256 MB unpacked
  (`sync::MAX_FRAME`, `MAX_UNPACKED`); a change bigger than that is undone in the tab with a
  message. A tab gets the whole file again (Resync) at most every 2 s.

## Steps

| Step | What | Done when |
|---|---|---|
| 0 | Rust server with everything server/files.py did | The app works unchanged against it; same answers as the old server |
| 1 | Live sync between tabs, saving on the server | Two tabs editing one file stay identical; random command runs give identical workbooks |
| 2 | Accounts, names and colours, other people's cursors | Two people see each other's cursors |
| 3 | Undo of your own changes only | Undo never wipes someone else's change |
| 4 | Share dialog, view-only mode | A viewer can't edit; the owner can change roles |
| Later | Desktop apps run the same server for local files; changes kept while offline; comment threads | |

## What we learnt building it

- A replayed command is checked by a fingerprint of what it changed (formula *results* are left
  out, since NOW() and RAND() differ). On a mismatch the server asks the tab for the finished
  state of that change, so a command we didn't know reads outside state still ends up right.
- That fingerprint covers only the cells the change made different (`CellStore::diff` visits
  just the 64-row bands it copied), hashed field by field: on a million-cell tab it is under a
  millisecond, where hashing the whole tab as JSON took about 90 ms on every edit. Changing how
  fingerprints are made needs a protocol bump.
- Opening: the tab has already read the .xlsx, so its Hello says which version; if the server's
  copy is still exactly that (nothing changed since it read the file), Welcome brings only a
  fingerprint to check. Whole workbooks and sheets travel with what their array formulas spill
  (JSON leaves it out), so no tab recalculates the whole file to show them.
- Messages of 64 KB or more travel compressed as binary frames (deflate, fast): a whole workbook
  of two million cells is 140 MB of JSON and 15.5 MB packed (about 80 ms to pack, 50 ms to
  unpack). A binary format instead of JSON was not taken: GridCraft's types skip empty fields
  when serialising, which non-self-describing formats can't read back, and packing already
  removes most of the size.
- Saving runs on its own thread; the file lock is held only for the write, and the "changed
  outside StarSheets" check waits while our own save is writing.
- The server finds an open file by its index entry and one file check, and lists the folder
  tree only when the file isn't there any more: listing a big tree on a network share every few
  seconds for each open file held up every other request.
- GridCraft's active sheet is view state stored in the workbook without making a new version, so
  switching sheets is never sent as a change.
- Typed input depends on the person's date and number settings, so each command carries them.
- Undo puts back the cells your change touched (values, formulas, formats), keeping later
  changes by others to other cells. It's refused when someone has changed those same cells since,
  or when the change was more than cells (column widths, rules, sheets) and others have edited
  since. Undo and redo go out as ordinary changes, so every tab stays in step.

Progress is ticked off in [ROADMAP.md](../ROADMAP.md).
