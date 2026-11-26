### Added
- Tests that fail if Docs or Slides keep redrawing the screen while nobody touches them (more than 5 redraws a second), and a real-browser check `testing/idle-repaint.py` that counts actual redraws per app.
