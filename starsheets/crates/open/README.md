# starbase-open

What a file manager runs to open a spreadsheet or document (`packaging/linux/io.github.DomeShifter.Starbase.Open.desktop`).
A file inside the connected server's storage folder, as mounted on this computer, opens on the server in the browser.
Any other file opens in Starbase Sheets or Starbase Docs. How it decides: `src/main.rs` (top comment) and `docs/decisions.md`.

## When a file on the share still opens in the desktop app

The handler sends the file's path inside the storage folder, and the server looks for exactly that name in its own list.
These names don't match, so the file opens in Sheets or Docs instead (it isn't lost, just not opened on the server):

- **Accents written two ways.** "é" can be stored as one character (NFC, usual on Linux) or as "e" plus an accent mark (NFD, usual on Macs). If the share and the server see different forms, the names differ.
- **Names that aren't valid UTF-8** (very old files from other systems). They are never sent.
- **SMB short names** (`BUDGET~1.XLS`), when a share is mounted in a mode that shows those instead of the long names.
- **Hidden parts** (`.bin`, `.starsheets`): never the server's files to open.
