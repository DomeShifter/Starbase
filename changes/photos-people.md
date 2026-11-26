### Added

- Photos: People. A background face worker (off by default; the server owner turns it on, or STARBASE_PHOTOS_FACES) finds faces in the thumbnails with YuNet and AuraFace or SFace (ONNX models from STARBASE_ML_MODELS, builds with `--features ml`), groups them by average linkage, and the People view shows them as round faces: name a person, merge two, take a wrong face out, open their pictures.
- Photos › People: the server's owner can fetch the face models with one button ("Download face models"): YuNet (MIT) and AuraFace (Apache-2.0) from pinned URLs, size and SHA-256 checked, with progress and plain errors. The People list refreshes itself while faces are being found.
- Server Docker image: built with face recognition (`--features ml`, ONNX Runtime linked in statically).
