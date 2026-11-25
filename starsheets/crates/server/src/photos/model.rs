//! The edit model the server stores (photo-editor contract §1): the `starbase-photo-edit` crate's
//! `Edits`. The sidecar code relies only on serde (the contract's JSON field names), `Default`,
//! `PartialEq` and `is_identity()`.

pub use starbase_photo_edit::Edits;
