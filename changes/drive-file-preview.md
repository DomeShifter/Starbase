### Added
- Drive: Google Drive-style file preview over the file list (double click, Enter or right-click › Preview): pictures with zoom and pan, PDFs, films and sound in the browser, text, arrows through the folder, Details panel, Download and Open with; "No preview available" for other files.
- Drive: text previews: plain text and logs (line numbers, wrap), Markdown, CSV/TSV tables, JSON (text or tree), and source code with syntax colours; files over 1 MB show their first part.

### Changed
- Server: a PDF's raw bytes may be framed by Starbase's own pages (X-Frame-Options SAMEORIGIN, frame-ancestors 'self').

### Fixed
- Drive: Esc closes the file preview also after a PDF or a film took the keyboard focus.
