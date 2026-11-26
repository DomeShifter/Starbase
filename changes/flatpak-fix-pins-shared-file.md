### Fixed

- The Flatpak build is green again: the manifest now ships the one shared source file (emacs_keys.rs) to StarDocs and StarSlides, and its engine commits match the pins. A CI check now fails when they drift apart.
