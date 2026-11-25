### Added
- Sheets: copying in one Starbase Sheets tab and pasting in another (or in another file) now brings the formulas, formatting, notes, links, merged cells and drop-down/validation rules, as Google Sheets does; before, only the values came across.

### Fixed
- Sheets: Ctrl+Shift+V (paste values only) did nothing in the browser. It now pastes the copied cells' results, without their formulas, formatting or validation, and keeps the formatting and rules already on the cells pasted into.
