### Added
- Photos: the viewer shows big pictures from a 2560 px screen copy instead of the original (a 50 MP photo no longer flickers or downloads tens of MB just to look at it). The server makes it in JPEG XL in the background when `cjxl` (package `libjxl-tools`) is installed, and a JPEG one on request for browsers that can't show JPEG XL.
