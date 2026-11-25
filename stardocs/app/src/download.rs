//! File › Download, as in Google Docs: the document as Word (.docx), OpenDocument (.odt), Rich
//! Text (.rtf), PDF, plain text, a web page (.html) or Markdown (.md).
//!
//! Word's .docx is the document's own save (`stardocs.download`, in shell.rs: saved, and that
//! save handed to the download too) and PDF is WordCraft's Export PDF dialog. The other formats
//! are a copy written here by WordCraft's `formats` crate and handed to `Services::download`:
//! in the browser a download (or its Save dialog), in the desktop program the system's Save
//! dialog (localfile.rs `save_elsewhere`). It is never the document's own save: `file.saveAs`
//! would make an .odt the server's copy, or mark unsaved changes as saved.
//!
//! File › Make a copy without a Starbase server or a file of its own (the plain web app) is a
//! download too: "Copy of <name>.docx".

use wordcraft_ui_egui::WordApp;

use crate::server::Link;

/// Writes the document as `ext` and hands it to the download, named after the document
/// (`prefix` before the name: "Copy of "). Returns the name it was given.
pub fn download_as(app: &mut WordApp, link: Option<&Link>, ext: &str, prefix: &str) -> Result<String, String> {
    let stem = match link.map(Link::name).filter(|n| !n.is_empty()) {
        Some(n) => n.rsplit_once('.').map_or(n.as_str(), |(s, _)| s).to_string(),
        None => app.title_stem(),
    };
    let name = format!("{prefix}{stem}.{ext}");
    let opts = wordcraft_engine::cmd::file::export_options(&app.session, &serde_json::json!({}));
    let bytes = wordcraft_engine::io::save_bytes_with(&name, &app.session.doc, None, &opts)?;
    match &app.services.download {
        Some(download) => download(&name, &bytes).map(|()| name),
        None => Err("downloads are unavailable here".into()),
    }
}
