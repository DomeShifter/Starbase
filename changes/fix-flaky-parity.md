### Fixed
- StarSheets server: when a file has more versions than it keeps, a burst of saves that crossed a
  second boundary could keep an older version instead of the newest two (also made the parity
  test's "version order" check fail now and then). Gaps between versions are now compared in
  whole minutes.
