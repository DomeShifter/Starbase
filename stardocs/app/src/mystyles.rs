//! Format › Paragraph styles › Options, as in Google Docs: save the document's paragraph styles
//! as my default styles (kept in the browser), use them in this document, or reset this
//! document's styles to Docs' own. New documents start with my default styles once saved
//! ([`crate::shell::docs_defaults`]).
//!
//! The styles are Docs' set only: Normal text, Title, Subtitle and Headings 1 to 6. Using or
//! resetting them is one undo step.

use wordcraft_doc::Document;
use wordcraft_doc::styles::Style;
use wordcraft_ui_egui::WordApp;

use crate::shell::store;

/// Where my default styles are kept (the browser's `localStorage`), as JSON.
const KEY: &str = "stardocs.myStyles";
/// Docs' paragraph styles, by WordCraft's style ids.
const DOCS_STYLES: &[&str] = &["Normal", "Title", "Subtitle", "Heading1", "Heading2", "Heading3", "Heading4", "Heading5", "Heading6"];
/// The most stored text read back.
const MAX_STORED: usize = 256 << 10;

/// Docs' styles as `doc` has them.
fn docs_styles(doc: &Document) -> Vec<Style> {
    DOCS_STYLES.iter().filter_map(|id| doc.styles.get(id).cloned()).collect()
}

/// My default styles, if saved.
fn saved() -> Option<Vec<Style>> {
    let text = store::get(KEY).filter(|t| t.len() <= MAX_STORED)?;
    serde_json::from_str::<Vec<Style>>(&text).ok().filter(|v| !v.is_empty())
}

/// Puts `styles` in place of `doc`'s styles of the same ids (adding any it lacks).
fn put(doc: &mut Document, styles: Vec<Style>) {
    let sheet = std::sync::Arc::make_mut(&mut doc.styles);
    for st in styles.into_iter().filter(|st| DOCS_STYLES.contains(&st.id.as_str())) {
        match sheet.get_mut(&st.id) {
            Some(old) => *old = st,
            None => sheet.styles.push(st),
        }
    }
}

/// A new document's styles: my default styles, when saved.
pub fn apply_saved(doc: &mut Document) {
    if let Some(styles) = saved() {
        put(doc, styles);
    }
}

/// Whether my default styles are saved (Use my default styles is greyed until they are).
pub fn have_saved() -> bool {
    saved().is_some()
}

/// Save as my default styles: this document's Docs styles, for new documents and Use my
/// default styles.
pub fn save(app: &mut WordApp) {
    match serde_json::to_string(&docs_styles(&app.session.doc)) {
        Ok(json) => {
            store::set(KEY, &json);
            app.status("Saved as your default styles");
        }
        Err(e) => app.status(format!("Couldn't save your default styles: {e}")),
    }
}

/// Use my default styles in this document.
pub fn use_saved(app: &mut WordApp) {
    match saved() {
        Some(styles) => replace(app, "Use my default styles", styles),
        None => app.status("No default styles saved yet: Format › Paragraph styles › Options › Save as my default styles"),
    }
}

/// Reset styles: Docs' own look (Arial, as a new blank document), not my default styles.
pub fn reset(app: &mut WordApp) {
    let mut blank = wordcraft_engine::blank_document(&Default::default());
    crate::shell::docs_look(&mut blank);
    replace(app, "Reset styles", docs_styles(&blank));
}

/// Puts `styles` into the open document as one undo step.
fn replace(app: &mut WordApp, label: &str, styles: Vec<Style>) {
    let s = &mut app.session;
    s.checkpoint(label);
    put(&mut s.doc, styles);
    s.touch();
}
