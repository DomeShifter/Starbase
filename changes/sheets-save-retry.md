### Fixed
- Starbase Sheets on the desktop: a save to the server that failed is tried again by itself, even
  while the window is in the background (2 s later, then waiting longer each time, at most 30 s),
  and the save status shows the result straight away. It used to wait until the window was
  raised again.
