//! Where the Drive home screen's files live: the seam between the home screen (home/) and the
//! place that keeps the files.
//!
//! The home screen speaks the server's files API (`api/files…`, `api/folders…`, `api/bin…`,
//! crates/server) through a [`Store`]. In the browser that is the server itself ([`ServerStore`],
//! `fetch` on the same site). In the desktop app's local mode it is [`crate::localstore`], which
//! answers the same calls from a folder on disk (`~/Documents/Starbase`). A later "connect to a
//! server" desktop job plugs a native HTTP client in here.
//!
//! What can't be a files-API call (opening a file, starting a new one, picking files to upload,
//! saving a download, small per-person settings) is a method of its own.

use std::rc::Rc;

use gridcraft_engine::model::Workbook;

/// An HTTP method of the files API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Method {
    Get,
    Post,
    /// The web app's spreadsheet saves (files.rs) and photo edits (home/photo_edit.rs).
    Put,
    Delete,
}

/// The answer to one call: the status and the body, or why there was none.
pub(crate) type Answer = Result<(u16, Vec<u8>), String>;
/// What to do with the answer; it runs on the UI thread, during [`Store::poll`] or straight away.
pub(crate) type Done = Box<dyn FnOnce(Answer)>;

pub(crate) trait Store {
    /// One call to the files API, `url` relative to the app (`api/files?…`).
    fn call(&self, ctx: &egui::Context, method: Method, url: &str, body: Option<Vec<u8>>, done: Done);
    /// The same with an `If-Match` header (photo edits); stores without one ignore it.
    fn call_if(&self, ctx: &egui::Context, method: Method, url: &str, body: Option<Vec<u8>>, if_match: Option<String>, done: Done) {
        let _ = if_match;
        self.call(ctx, method, url, body, done);
    }
    /// Runs the `done` of calls answered since last time (each frame, before anything else).
    fn poll(&self) {}
    /// Opens a listed file: a spreadsheet in Starbase Sheets, a document in Starbase Docs.
    fn open(&self, id: &str, name: &str) -> Result<(), String>;
    /// Starts a new spreadsheet, or a new document, saved in `folder`.
    fn new_file(&self, folder: &str, document: bool) -> Result<(), String>;
    /// Asks which files to upload; `got` gets each one picked (name, bytes).
    fn pick_files(&self, got: Box<dyn Fn(String, Vec<u8>)>);
    /// Asks for pictures and videos (a phone offers its camera roll and camera); default: any file.
    fn pick_media(&self, got: Box<dyn Fn(String, Vec<u8>)>) {
        self.pick_files(got);
    }
    /// Hands a downloaded file to the person (the browser's download, or a Save dialog).
    fn download(&self, name: &str, bytes: &[u8]) -> Result<(), String>;
    /// A small per-person setting (grid or list, sort order); missing means the default.
    fn setting(&self, key: &str) -> Option<String>;
    fn set_setting(&self, key: &str, value: &str);
    /// The folder to show first, if one was asked for (`?dir=` on the web).
    fn start_dir(&self) -> Option<String> {
        None
    }
    /// Whether to open "New from a template" at once (`?templates=1` on the web: an editor's
    /// File › New from a template…).
    fn start_templates(&self) -> bool {
        false
    }
    /// Whether presentations can be made here ("New presentation"): in the browser, and on the
    /// desktop with Starbase Slides beside the Drive.
    fn has_slides(&self) -> bool {
        false
    }
    /// Starts a new presentation (a named .pptx) in `folder`, in Starbase Slides.
    fn new_presentation(&self, _folder: &str) -> Result<(), String> {
        Err("Presentations can't be made here.".into())
    }
    /// Whether [`Store::open`] opens every kind of file itself (this computer's own files: Sheets,
    /// Docs, else the system's default app), instead of the Drive's preview or download.
    fn opens_any_file(&self) -> bool {
        false
    }
    /// Whether "View as photos" works here: a Starbase server's photo index (api/photos), not
    /// a folder on this computer.
    fn has_photos(&self) -> bool {
        false
    }
    /// The Starbase server the desktop app is connected to (remote.rs); None on this computer's
    /// own files.
    #[cfg(not(target_arch = "wasm32"))]
    fn server(&self) -> Option<starbase_connect::Server> {
        None
    }
}

/// The store the home screen uses (cheap to clone).
#[derive(Clone)]
pub(crate) struct Backend(pub Rc<dyn Store>);

impl Default for Backend {
    fn default() -> Backend {
        #[cfg(target_arch = "wasm32")]
        {
            Backend(Rc::new(ServerStore))
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Backend(Rc::new(crate::localstore::LocalStore::new(crate::localstore::default_root())))
        }
    }
}

impl std::ops::Deref for Backend {
    type Target = dyn Store;
    fn deref(&self) -> &(dyn Store + 'static) {
        &*self.0
    }
}

/// Whether a file is a document (Starbase Docs) or a presentation (Starbase Slides) rather than
/// a spreadsheet.
pub fn is_doc(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".docx") || lower.ends_with(".odt") || is_slides(name)
}

/// Whether a file is a presentation (Starbase Slides, in the browser).
pub fn is_slides(name: &str) -> bool {
    name.to_lowercase().ends_with(".pptx")
}

/// The desktop program that opens a file: Starbase Slides, Docs or Sheets.
#[cfg_attr(target_arch = "wasm32", allow(dead_code, reason = "the desktop's; the web opens pages"))]
pub fn program_for(name: &str) -> &'static str {
    if is_slides(name) {
        "starslides"
    } else if is_doc(name) {
        "stardocs"
    } else {
        "starsheets"
    }
}

/// The app logo the home screen shows for a file: Slides, Docs or Sheets.
pub fn app_icon(name: &str) -> &'static str {
    if is_slides(name) {
        "starslides"
    } else if is_doc(name) {
        "stardocs"
    } else {
        "starsheets"
    }
}

/// Percent-encodes `s` for a URL query or path part, as JavaScript's `encodeURIComponent`.
pub(crate) fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Undoes [`enc`] (and `+` as a space, as in a form-encoded query).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn dec(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                Ok(v) => {
                    out.push(v);
                    i += 3;
                    continue;
                }
                Err(_) => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether a workbook holds anything worth a file: a value or formula, a note, a chart or a
/// picture. Formatting alone (a format previewed on a blank sheet) doesn't count.
pub(crate) fn has_data(wb: &Workbook) -> bool {
    wb.sheets.iter().any(|s| {
        s.cells.iter().any(|(_, c)| c.formula.is_some() || !matches!(c.value, gridcraft_engine::core::Value::Empty))
            || !s.comments.is_empty()
            || !s.charts.is_empty()
            || !s.images.is_empty()
            || !s.shapes.is_empty()
    })
}

/// The workbook as .xlsx bytes, with Google Sheets' Clip put back in StarSheets' section of the
/// file (cells carry it while open: the server's and this tab's copies are read the same way).
pub(crate) fn xlsx(wb: &Workbook, name: &str) -> Result<Vec<u8>, String> {
    let mut wb = wb.clone();
    starsheets_sync::clip_into_host(&mut wb);
    gridcraft_engine::io::save_bytes(&wb, name).map_err(|e| e.to_string())
}

/// The server, on the same site as the web app.
#[cfg(target_arch = "wasm32")]
pub(crate) struct ServerStore;

#[cfg(target_arch = "wasm32")]
impl Store for ServerStore {
    fn call(&self, ctx: &egui::Context, method: Method, url: &str, body: Option<Vec<u8>>, done: Done) {
        self.call_if(ctx, method, url, body, None, done);
    }

    fn call_if(&self, ctx: &egui::Context, method: Method, url: &str, body: Option<Vec<u8>>, if_match: Option<String>, done: Done) {
        let (ctx, url) = (ctx.clone(), url.to_string());
        wasm_bindgen_futures::spawn_local(async move {
            done(crate::files::fetch(method, &url, body, if_match).await.map(|(s, _, b)| (s, b)));
            ctx.request_repaint();
        });
    }

    fn open(&self, id: &str, name: &str) -> Result<(), String> {
        // Files always open in a new tab, as from Google Drive, so the home screen stays where it was.
        if let Some(w) = web_sys::window() {
            let _ = w.open_with_url_and_target(&crate::files::file_url(id, name), "_blank");
        }
        Ok(())
    }

    fn new_file(&self, folder: &str, document: bool) -> Result<(), String> {
        let url = if document {
            crate::files::docs_url("", &format!("&folder={}", enc(folder)))
        } else {
            let path = web_sys::window().and_then(|w| w.location().pathname().ok()).unwrap_or_else(|| "/".into());
            format!("{path}?folder={}", enc(folder))
        };
        if let Some(w) = web_sys::window() {
            let _ = w.location().set_href(&url);
        }
        Ok(())
    }

    fn has_slides(&self) -> bool {
        true
    }

    fn has_photos(&self) -> bool {
        true
    }

    fn new_presentation(&self, folder: &str) -> Result<(), String> {
        if let Some(w) = web_sys::window() {
            let _ = w.location().set_href(&crate::files::slides_url("", &format!("&folder={}", enc(folder))));
        }
        Ok(())
    }

    fn pick_files(&self, got: Box<dyn Fn(String, Vec<u8>)>) {
        wasm_bindgen_futures::spawn_local(async move {
            // The Drive keeps any kind of file.
            let Some(picked) = rfd::AsyncFileDialog::new().pick_files().await else {
                return;
            };
            for f in picked {
                let (name, bytes) = (f.file_name(), f.read().await);
                got(name, bytes);
            }
        });
    }

    fn pick_media(&self, got: Box<dyn Fn(String, Vec<u8>)>) {
        crate::web::pick_media(got);
    }

    fn download(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        crate::web::download_file(name, bytes)
    }

    fn setting(&self, key: &str) -> Option<String> {
        web_sys::window()?.local_storage().ok()??.get_item(key).ok()?
    }

    fn set_setting(&self, key: &str, value: &str) {
        if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
            let _ = s.set_item(key, value);
        }
    }

    fn start_dir(&self) -> Option<String> {
        let search = web_sys::window()?.location().search().ok()?;
        web_sys::UrlSearchParams::new_with_str(&search).ok()?.get("dir").filter(|d| !d.is_empty())
    }

    fn start_templates(&self) -> bool {
        let search = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
        web_sys::UrlSearchParams::new_with_str(&search).ok().and_then(|p| p.get("templates")).as_deref() == Some("1")
    }
}

#[cfg(test)]
mod tests {
    use super::{dec, enc, is_doc, is_slides, program_for};

    #[test]
    fn encodes_like_encode_uri_component() {
        assert_eq!(enc("a b/c&d=é"), "a%20b%2Fc%26d%3D%C3%A9");
        assert_eq!(enc("Budget (2).xlsx"), "Budget%20(2).xlsx");
        for s in ["a b/c&d=é", "100% sure", "x+y", ""] {
            assert_eq!(dec(&enc(s)), s);
        }
        assert_eq!(dec("50%"), "50%");
        assert_eq!(dec("a+b"), "a b");
    }

    #[test]
    fn documents_are_docx_and_odt() {
        assert!(is_doc("Letter.DOCX") && is_doc("notes.odt"));
        assert!(!is_doc("Budget.xlsx"));
        assert!(is_doc("Talk.PPTX") && is_slides("Talk.PPTX") && !is_slides("Letter.docx"));
        assert_eq!([program_for("Talk.pptx"), program_for("Letter.docx"), program_for("Budget.xlsx")], ["starslides", "stardocs", "starsheets"]);
    }
}
