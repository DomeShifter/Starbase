### Added

- Photos: People. A background face worker (off by default; the server owner turns it on, or STARBASE_PHOTOS_FACES) finds faces in the thumbnails with YuNet and AuraFace or SFace (ONNX models from STARBASE_ML_MODELS, builds with `--features ml`), groups them by average linkage, and the People view shows them as round faces: name a person, merge two, take a wrong face out, open their pictures.
