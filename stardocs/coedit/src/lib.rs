//! Docs co-editing engine (not wired into the app yet): editing *steps* worked out by
//! comparing a WordCraft document before and after a command, applying them, mapping positions
//! through them, transforming a step over a concurrent one (the server orders changes, the
//! clients rebase their pending steps over what the server sent), and inverting them for
//! per-user undo.
//!
//! Every story is covered (body, headers, footers, notes, comment text, text boxes). Paragraph
//! steps act in a container: a story's top level or a table cell at any depth ([`Cont`]).
//! - Paragraphs: [`Step::Splice`] (text, runs, inline objects), [`Step::Format`],
//!   [`Step::ParaProps`], [`Step::Split`] (Enter), [`Step::Join`] (Backspace/Delete at an edge).
//! - Tables: rows and columns added/removed, table/row/cell formatting; cell content through
//!   the paragraph steps. Merged cells and anything else fall back to [`Step::Blocks`].
//! - Everything else, per item: stories added/removed, comments by id, revision records (with
//!   reference renumbering), media by key, and each other document field (last writer wins).

pub mod diff;
pub mod invert;
pub mod room;
pub mod sim;
pub mod step;
pub mod xform;

pub use diff::diff;
pub use invert::{invert, invert_all};
pub use room::{Caret, Catchup, DocRoom, Entry, Received, Where};
pub use step::{Attrs, Cont, Step, apply, apply_all, normalized, short};
pub use xform::{DROPPED, RECORD_DROPS, map_pos, xform, xform_lists};
