### Added
- Starbase Drive: **Ctrl+Z undoes** the last move (drag, cut and paste, Move to…), rename, New
  folder or Move to bin, and a note "Moved to … · Undo" shows after a move or a delete, as in
  Google Drive. An undo is checked before anything changes: when the item has been moved,
  renamed or deleted since, its old name is taken, or a new folder has had things put in it,
  nothing is touched and the Drive says why. Ctrl+Z in the rename box or search box stays theirs.
- Server: `from=`, `as=`, `exact=1`, `if_empty=1` and `expect=` checks on move, rename, folder
  delete and bin restore (what the Drive's Undo uses); a delete answers with the item's name in
  the bin.

### Changed
- Starbase Drive: the New button is a little less tall and light blue, so it is easy to spot.
