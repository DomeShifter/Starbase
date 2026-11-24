//! The Starbase server link: when StarDocs is opened from the Starbase home screen
//! (`docs/?file=<id>`, or `docs/?new=1&folder=<folder>` for a new document) it reads and saves
//! the document through the server's files API (`api/files`, one folder up from this page)
//! instead of the browser's downloads.
//!
//! - Open: `GET api/files/<id>`; the reply's `ETag` is the file's version on disk.
//! - Save: `PUT api/files/<id>` with `If-Match: <version>`. Saves go one at a time; a save made
//!   while one is on its way waits, and only the newest waiting one is sent.
//! - Changed elsewhere (409, someone edited the file outside StarDocs or in another tab): the
//!   server's file is never overwritten. This version is kept as a new file beside it,
//!   "<name> (StarDocs copy <date>)", and the page carries on with that copy, saying so.
//! - A new document is made on its first save: `POST api/files?folder=…&name=…`.
//! - Not signed in (401): the page goes to the home screen, which asks to sign in.
//! - Exports (PDF, pictures, other formats) still download in the browser.
//!
//! The page saves by itself ([`Link::tick`]) a couple of seconds after the last change, like
//! Google Docs, and the title row shows "Saving…" / "Saved". A save that failed is tried again
//! every [`RETRY_MS`] until it goes through or a newer save replaces it, and leaving the page
//! asks first while anything is not yet on the server ([`unsaved`]).

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast as _;
use wasm_bindgen_futures::JsFuture;
use wordcraft_ui_egui::{Inbox, Services, WordApp};

/// The browser download WordCraft was given (`Services::download`).
type DownloadFn = Box<dyn Fn(&str, &[u8]) -> Result<(), String>>;

/// How long a message stays in the title row (milliseconds).
const NOTICE_MS: f64 = 20_000.0;

/// How long the document must sit unchanged before it saves by itself (milliseconds).
const QUIET_MS: f64 = 2000.0;

/// How long after a failed save it is tried again (milliseconds).
const RETRY_MS: f64 = 15_000.0;

/// The home screen's preview picture: the top of page 1, the size of a spreadsheet's (the
/// server's `thumbs::WIDTH`/`HEIGHT`), drawn here because the server can't lay out documents.
const THUMB_W: u32 = 720;
const THUMB_H: u32 = 480;
/// At most one new picture this often while someone keeps editing (milliseconds).
const THUMB_MS: f64 = 30_000.0;

thread_local! {
    /// This page's link, for the unsaved-changes guard ([`unsaved`]), which is set up before it.
    static CURRENT: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) };
}

/// Whether something is not on the server yet: a save on its way or waiting, or one that failed.
/// The page's `beforeunload` guard asks the browser to confirm leaving while this is true (the
/// editor's own unsaved flag clears as soon as a save is handed over, before the server has it).
pub fn unsaved() -> bool {
    CURRENT.with(|c| {
        c.borrow().as_ref().is_some_and(|s| {
            let s = s.borrow();
            s.busy || s.waiting.is_some() || s.retry.is_some()
        })
    })
}

/// What the title row says about saving.
#[derive(Clone, Default, PartialEq)]
pub enum Saved {
    #[default]
    Nothing,
    Saving,
    Done,
    Failed(String),
}

#[derive(Default)]
struct State {
    /// The server's id for the file; `None` until a new document's first save.
    id: Option<String>,
    /// The folder a new document goes into (and the home screen returns to).
    folder: String,
    /// The file's version on disk, as the server last told us.
    version: String,
    /// The file's name on the server.
    name: String,
    /// A save is on its way.
    busy: bool,
    /// The newest save made while one was on its way.
    waiting: Option<Vec<u8>>,
    saved: Saved,
    /// Messages for the status line.
    news: Vec<String>,
    /// The newest message, shown in the title row for a while (StarDocs has no status bar).
    notice: Option<(String, f64)>,
    /// When the document was first seen changed since the last save (ms), for [`Link::tick`].
    dirty_since: Option<f64>,
    /// The next save also downloads (File › Download › Word).
    download_next: bool,
    /// A save that failed, and when (ms): tried again after [`RETRY_MS`] unless a newer one comes.
    retry: Option<(Vec<u8>, f64)>,
    /// The signed-in person's name from the server, until the page signs comments with it.
    author: Option<String>,
    /// When the next preview picture is due (after a save), and when the last one was sent (ms).
    thumb_due: Option<f64>,
    thumb_last: f64,
    /// The signed-in person may only view this document: nothing is saved, and the editor is put
    /// in read-only mode once (`view_applied`).
    view_only: bool,
    view_applied: bool,
}

#[derive(Clone)]
pub struct Link {
    state: Rc<RefCell<State>>,
    ctx: egui::Context,
}

fn query_param(key: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search).ok()?.get(key)
}

/// The server's address for `path` (`api/…`): StarDocs sits in `docs/` beside the home page.
pub(crate) fn api(path: &str) -> String {
    format!("../{path}")
}

fn enc(s: &str) -> String {
    js_sys::encode_uri_component(s).as_string().unwrap_or_default()
}

fn now() -> f64 {
    js_sys::Date::now()
}

impl Link {
    /// The link for this page, or `None` when it wasn't opened from the home screen (no `?file=`
    /// and no `?new=`): then StarDocs is the plain app with downloads.
    pub fn detect(ctx: egui::Context) -> Option<Self> {
        let id = query_param("file").filter(|s| !s.is_empty());
        if id.is_none() && query_param("new").is_none() {
            return None;
        }
        let folder = query_param("folder").unwrap_or_default();
        let state = Rc::new(RefCell::new(State { id, folder, ..Default::default() }));
        CURRENT.with(|c| *c.borrow_mut() = Some(state.clone()));
        let link = Self { state, ctx };
        link.ask_who();
        Some(link)
    }

    /// Asks the server who is signed in, so comments and tracked changes carry their name.
    fn ask_who(&self) {
        let link = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok((200, _, body)) = fetch("GET", &api("api/info"), None, None).await {
                let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                let user = &v["user"];
                let name = user["name"].as_str().filter(|n| !n.trim().is_empty()).or_else(|| user["username"].as_str());
                if let Some(name) = name.filter(|n| !n.trim().is_empty()) {
                    link.state.borrow_mut().author = Some(name.to_string());
                    link.ctx.request_repaint();
                }
            }
        });
    }

    /// Asks the server what the signed-in person may do with this document (its sharing reply's
    /// `you`: owner, edit or view).
    fn ask_role(&self, id: &str) {
        let link = self.clone();
        let url = api(&format!("api/files/{id}/sharing"));
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok((200, _, body)) = fetch("GET", &url, None, None).await {
                let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                if v["you"].as_str() == Some("view") {
                    link.state.borrow_mut().view_only = true;
                    link.ctx.request_repaint();
                    return;
                }
            }
            // A document without a preview picture (made elsewhere, or never pictured) gets one now.
            let thumb = api(&format!("api/files/{}/thumb", link.id().unwrap_or_default()));
            if let Ok((404, ..)) = fetch("GET", &thumb, None, None).await {
                let mut s = link.state.borrow_mut();
                s.thumb_due = Some(now());
                drop(s);
                link.ctx.request_repaint();
            }
        });
    }

    /// Whether the signed-in person may only view this document.
    pub fn view_only(&self) -> bool {
        self.state.borrow().view_only
    }

    /// The signed-in person's name, once, when the server has said it (see [`Self::ask_who`]).
    pub fn take_author(&self) -> Option<String> {
        self.state.borrow_mut().author.take()
    }

    /// Saves go to the server; exports still download (`download` is the browser's).
    pub fn connect(&self, services: &mut Services) {
        // WordCraft takes its web save path only while `download` is set; `host_save` then gets
        // every file, and exports are handed back to the browser download here.
        let download: Rc<Option<DownloadFn>> = Rc::new(services.download.take());
        let keep = download.clone();
        services.download = Some(Box::new(move |name: &str, bytes: &[u8]| match keep.as_ref() {
            Some(d) => d(name, bytes),
            None => Err("downloads are unavailable".into()),
        }));
        services.async_saves = None;
        let link = self.clone();
        services.host_save = Some(Box::new(move |name: &str, bytes: &[u8], _save_as: bool| {
            if !is_document(name) {
                return match download.as_ref() {
                    Some(d) => d(name, bytes),
                    None => Err("downloads are unavailable".into()),
                };
            }
            if std::mem::take(&mut link.state.borrow_mut().download_next)
                && let Some(d) = download.as_ref()
            {
                let shown = link.state.borrow().name.clone();
                let _ = d(if shown.is_empty() { name } else { &shown }, bytes);
            }
            link.save(name, bytes.to_vec());
            Ok(())
        }));
    }

    /// Reads the document from the server into the app (as a dropped file).
    pub fn open(&self, inbox: Inbox) {
        let Some(id) = self.state.borrow().id.clone() else { return };
        let link = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match fetch("GET", &api(&format!("api/files/{id}")), None, None).await {
                Ok((200, headers, bytes)) => {
                    let name = header(&headers, "X-File-Name")
                        .map(|n| serde_json::from_str::<String>(&n).unwrap_or(n))
                        .unwrap_or_else(|| "Document.docx".into());
                    let folder = header(&headers, "X-Folder").map(|n| serde_json::from_str::<String>(&n).unwrap_or(n)).unwrap_or_default();
                    {
                        let mut s = link.state.borrow_mut();
                        s.version = header(&headers, "ETag").unwrap_or_default();
                        s.name = name.clone();
                        s.folder = folder;
                    }
                    set_title(&name);
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                    link.ask_role(&id);
                }
                Ok((401, ..)) => go_home(""),
                Ok((code, _, body)) => {
                    // Nothing to edit: the blank page is read-only, so typing can't look like work.
                    link.state.borrow_mut().view_only = true;
                    link.fail(&format!("Couldn't open the document ({})", reason(code, &body)));
                }
                Err(e) => link.fail(&format!("Couldn't reach the server: {e}")),
            }
            link.ctx.request_repaint();
        });
    }

    /// The home screen, at this document's folder.
    pub fn home(&self) {
        let folder = self.state.borrow().folder.clone();
        go_home(&folder);
    }

    /// File › New document: a new tab, in this document's folder.
    pub fn new_document(&self) {
        let folder = self.state.borrow().folder.clone();
        if let Some(w) = web_sys::window() {
            let _ = w.open_with_url_and_target(&format!("?new=1&folder={}", enc(&folder)), "_blank");
        }
    }

    /// The server's id for the file (`None` until a new document's first save).
    pub fn id(&self) -> Option<String> {
        self.state.borrow().id.clone()
    }

    /// Saves now, without waiting for the quiet moment (Share on a document not saved yet).
    pub fn save_now(&self, app: &mut WordApp) {
        let busy = {
            let mut s = self.state.borrow_mut();
            s.dirty_since = None;
            s.busy
        };
        if !busy {
            crate::shell::run(app, "file.save", serde_json::json!({}));
        }
    }

    /// The file's name on the server (empty before it is known).
    pub fn name(&self) -> String {
        self.state.borrow().name.clone()
    }

    /// A message from the last little while (about 20 s) for the title row.
    pub fn notice(&self) -> Option<String> {
        let s = self.state.borrow();
        s.notice.as_ref().filter(|(_, at)| now() - at < NOTICE_MS).map(|(m, _)| m.clone())
    }

    pub fn saved(&self) -> Saved {
        self.state.borrow().saved.clone()
    }

    /// File › Download › Word: save, and download what was saved.
    pub fn download_next(&self) {
        self.state.borrow_mut().download_next = true;
    }

    /// Every frame: news for the status line, and the save made by itself once the document has
    /// been left alone for a moment.
    pub fn tick(&self, app: &mut WordApp, ctx: &egui::Context) {
        // Taken first: a `for` over `take(&mut self.state.borrow_mut().news)` keeps that borrow for
        // the whole loop, and the body's own borrow then panicked (froze the page on any message).
        let news = std::mem::take(&mut self.state.borrow_mut().news);
        for msg in news {
            let mut s = self.state.borrow_mut();
            // A failed save already says so in red beside the name; no second copy in amber.
            if s.saved != Saved::Failed(msg.clone()) {
                s.notice = Some((msg.clone(), now()));
            }
            drop(s);
            app.status(msg);
        }
        let t = now();
        let again = {
            let mut s = self.state.borrow_mut();
            match &s.retry {
                Some((_, at)) if !s.busy && t - at >= RETRY_MS => s.retry.take().map(|(bytes, _)| (s.name.clone(), bytes)),
                Some(_) => {
                    ctx.request_repaint_after(std::time::Duration::from_secs(1));
                    None
                }
                None => None,
            }
        };
        if let Some((name, bytes)) = again {
            self.save(&name, bytes);
        }
        let apply_view = {
            let mut s = self.state.borrow_mut();
            let apply = s.view_only && !s.view_applied;
            if apply {
                s.view_applied = true;
            }
            s.view_only.then_some(apply)
        };
        if let Some(apply) = apply_view {
            if apply {
                // Viewing (WordCraft's session-only read-only state, fork change StarDocs
                // 0005): typing and formatting are refused, and nothing of it is saved in the
                // document, so a downloaded copy isn't protected.
                crate::shell::set_mode(app, crate::shell::Mode::Viewing);
            }
            // Nothing a viewer does is saved (the server would refuse it).
            app.session.dirty = false;
            self.state.borrow_mut().dirty_since = None;
            return;
        }
        self.maybe_thumb(app, ctx, t);
        let since = {
            let mut s = self.state.borrow_mut();
            if !app.session.dirty {
                s.dirty_since = None;
                return;
            }
            *s.dirty_since.get_or_insert(t)
        };
        if t - since >= QUIET_MS && !ctx.input(|i| i.pointer.any_down()) {
            self.state.borrow_mut().dirty_since = None;
            crate::shell::run(app, "file.save", serde_json::json!({}));
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// Sends the preview picture once it is due and everything is saved: drawn from what is on
    /// screen, which then matches the version the server has (unsaved edits wait for their save).
    fn maybe_thumb(&self, app: &WordApp, ctx: &egui::Context, t: f64) {
        let (id, version) = {
            let s = self.state.borrow();
            let Some(due) = s.thumb_due else { return };
            if due > t || s.busy || s.waiting.is_some() || s.retry.is_some() || app.session.dirty {
                ctx.request_repaint_after(std::time::Duration::from_secs(1));
                return;
            }
            match &s.id {
                Some(id) if !s.version.is_empty() => (id.clone(), s.version.clone()),
                _ => return,
            }
        };
        {
            let mut s = self.state.borrow_mut();
            s.thumb_due = None;
            s.thumb_last = t;
        }
        let layout = app.session.export_layout();
        let Some(page) = layout.pages.first() else { return };
        let scale = f64::from(THUMB_W) / f64::from(page.w.max(1.0));
        let picture = wordcraft_render::render_region(&app.session.doc, page, THUMB_W, THUMB_H, kurbo::Affine::scale(scale), &Default::default());
        let png = picture.to_png();
        if png.is_empty() {
            return;
        }
        let url = api(&format!("api/files/{id}/thumb?version={}", enc(&version)));
        wasm_bindgen_futures::spawn_local(async move {
            // 409: a newer save got there first; its own picture follows.
            match fetch("PUT", &url, None, Some(&png)).await {
                Ok((200 | 409, ..)) => {}
                Ok((code, _, body)) => log::warn!("preview picture not kept: {}", reason(code, &body)),
                Err(e) => log::warn!("preview picture not sent: {e}"),
            }
        });
    }

    /// Renames the file on the server (the title row).
    pub fn rename(&self, stem: &str) {
        let Some(id) = self.state.borrow().id.clone() else { return };
        let link = self.clone();
        let url = api(&format!("api/files/{id}/rename?name={}", enc(stem)));
        wasm_bindgen_futures::spawn_local(async move {
            match fetch("POST", &url, None, None).await {
                Ok((200, _, body)) => {
                    let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                    if let Some(name) = v.get("name").and_then(|n| n.as_str()) {
                        link.state.borrow_mut().name = name.to_string();
                        set_title(name);
                    }
                }
                Ok((code, _, body)) => link.news(format!("Couldn't rename: {}", reason(code, &body))),
                Err(e) => link.news(format!("Couldn't rename: {e}")),
            }
            link.ctx.request_repaint();
        });
    }

    fn news(&self, msg: String) {
        self.state.borrow_mut().news.push(msg);
    }

    fn fail(&self, msg: &str) {
        let mut s = self.state.borrow_mut();
        s.saved = Saved::Failed(msg.to_string());
        s.news.push(msg.to_string());
    }

    fn save(&self, name: &str, bytes: Vec<u8>) {
        if self.view_only() {
            self.news("You can view this document but not change it, so nothing is saved.".to_string());
            return;
        }
        {
            let mut s = self.state.borrow_mut();
            s.saved = Saved::Saving;
            // A newer save replaces one that failed.
            s.retry = None;
            if s.busy {
                s.waiting = Some(bytes);
                return;
            }
            s.busy = true;
            if s.name.is_empty() {
                s.name = name.to_string();
            }
        }
        let link = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let mut next = Some(bytes);
            while let Some(bytes) = next.take() {
                if let Err(bytes) = link.send(bytes).await {
                    link.state.borrow_mut().retry = Some((bytes, now()));
                }
                next = link.state.borrow_mut().waiting.take();
            }
            let mut s = link.state.borrow_mut();
            s.busy = false;
            if s.saved == Saved::Saving {
                s.saved = Saved::Done;
            }
            drop(s);
            link.ctx.request_repaint();
        });
    }

    /// One save: the file itself, a new file, or (changed elsewhere) a kept copy. A failed save
    /// hands its bytes back, to try again.
    async fn send(&self, bytes: Vec<u8>) -> Result<(), Vec<u8>> {
        let (id, version, name, folder) = {
            let s = self.state.borrow();
            (s.id.clone(), s.version.clone(), s.name.clone(), s.folder.clone())
        };
        let r = match &id {
            Some(id) => fetch("PUT", &api(&format!("api/files/{id}")), Some(&version), Some(&bytes)).await,
            None => self.create(&folder, &name, &bytes).await,
        };
        let why = match r {
            Ok((200 | 201, _, body)) => {
                self.took(&body);
                return Ok(());
            }
            Ok((409, ..)) => {
                let (stem, ext) = split(&name);
                let copy = format!("{stem} (StarDocs copy {}){ext}", stamp());
                match self.create(&folder, &copy, &bytes).await {
                    Ok((201 | 200, _, body)) => {
                        self.took(&body);
                        let shown = self.name();
                        self.news(format!("Someone else changed \"{name}\" while you were editing. Their version is untouched; yours is kept as \"{shown}\", which is open now."));
                        return Ok(());
                    }
                    Ok((code, _, body)) => format!("Not saved: the file changed elsewhere and a copy couldn't be made ({})", reason(code, &body)),
                    Err(e) => format!("Not saved: {e}"),
                }
            }
            Ok((401, ..)) => "Not saved: you're signed out. Sign in on the home screen in another tab".to_string(),
            // Not allowed: trying again can't help, so it isn't retried.
            Ok((403, _, body)) => {
                self.fail(&format!("Not saved ({}).", reason(403, &body)));
                return Ok(());
            }
            Ok((code, _, body)) => format!("Not saved ({})", reason(code, &body)),
            Err(e) => format!("Not saved: couldn't reach the server ({e})"),
        };
        self.fail(&format!("{why}. Trying again by itself; keep this tab open."));
        Err(bytes)
    }

    async fn create(&self, folder: &str, name: &str, bytes: &[u8]) -> Result<(u16, web_sys::Headers, Vec<u8>), String> {
        fetch("POST", &api(&format!("api/files?folder={}&name={}", enc(folder), enc(name))), None, Some(bytes)).await
    }

    /// A save's reply: the new version, and the id and name for a new file.
    fn took(&self, body: &[u8]) {
        let v: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
        let mut s = self.state.borrow_mut();
        if let Some(ver) = v.get("version").and_then(|x| x.as_str()) {
            s.version = ver.to_string();
        }
        if let Some(name) = v.get("name").and_then(|x| x.as_str()) {
            s.name = name.to_string();
        }
        if let Some(id) = v.get("id").and_then(|x| x.as_str())
            && s.id.as_deref() != Some(id)
        {
            s.id = Some(id.to_string());
            if let Some(h) = web_sys::window().and_then(|w| w.history().ok()) {
                let _ = h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&format!("?file={id}")));
            }
        }
        if let Saved::Failed(_) = s.saved {
            s.saved = Saved::Saving;
        }
        s.thumb_due = Some((s.thumb_last + THUMB_MS).max(now()));
        let name = s.name.clone();
        drop(s);
        set_title(&name);
    }
}

/// Whether a saved file is the document itself (a Word or OpenDocument package) rather than an
/// export.
fn is_document(name: &str) -> bool {
    let lower = name.to_lowercase();
    [".docx", ".docm", ".dotx", ".dotm", ".odt"].iter().any(|e| lower.ends_with(e))
}

fn split(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), ".docx".to_string()),
    }
}

/// "2026-11-24 14.05", local time, for copy names.
fn stamp() -> String {
    let d = js_sys::Date::new_0();
    format!("{}-{:02}-{:02} {:02}.{:02}", d.get_full_year(), d.get_month() + 1, d.get_date(), d.get_hours(), d.get_minutes())
}

fn set_title(name: &str) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
        doc.set_title(&format!("{stem} - StarDocs"));
    }
}

/// The home screen (one folder up), at `folder` when given.
fn go_home(folder: &str) {
    if let Some(w) = web_sys::window() {
        let url = if folder.is_empty() { "../".to_string() } else { format!("../?dir={}", enc(folder)) };
        let _ = w.location().set_href(&url);
    }
}

fn reason(code: u16, body: &[u8]) -> String {
    let v: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    v.get("error").and_then(|e| e.as_str()).map_or_else(|| format!("error {code}"), str::to_string)
}

fn header(h: &web_sys::Headers, name: &str) -> Option<String> {
    h.get(name).ok().flatten().filter(|s| !s.is_empty())
}

/// One request to the server, with the page's sign-in cookie: (status, headers, body).
pub(crate) async fn fetch(method: &str, url: &str, if_match: Option<&str>, body: Option<&[u8]>) -> Result<(u16, web_sys::Headers, Vec<u8>), String> {
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    let headers = web_sys::Headers::new().map_err(|e| format!("{e:?}"))?;
    if let Some(v) = if_match.filter(|v| !v.is_empty()) {
        headers.set("If-Match", v).map_err(|e| format!("{e:?}"))?;
    }
    init.set_headers(&headers);
    if let Some(b) = body {
        let array = js_sys::Uint8Array::from(b);
        init.set_body(&array.into());
    }
    let req = web_sys::Request::new_with_str_and_init(url, &init).map_err(|e| format!("{e:?}"))?;
    let window = web_sys::window().ok_or("no window")?;
    let resp: web_sys::Response = JsFuture::from(window.fetch_with_request(&req))
        .await
        .map_err(|_| "the network request failed".to_string())?
        .dyn_into()
        .map_err(|_| "bad reply")?;
    let buf = JsFuture::from(resp.array_buffer().map_err(|e| format!("{e:?}"))?).await.map_err(|e| format!("{e:?}"))?;
    Ok((resp.status(), resp.headers(), js_sys::Uint8Array::new(&buf).to_vec()))
}
