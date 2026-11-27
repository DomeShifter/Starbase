### Added
- Photos viewer: the Info panel has a People section: each named person in the picture (face and name, opens their page), how many faces are unnamed (opens People), or a quiet line when face recognition is off or no faces were found. New server endpoint `GET /api/photos/faces/ID` (only for pictures the caller may see).

### Fixed
- Photos viewer: big pictures (bigger than the picture memory budget, e.g. 50 MP phone photos) no longer flicker by being freed and downloaded again every frame.
