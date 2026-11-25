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
//!   "<name> (conflicted copy)" (as in Starbase Sheets), and the page carries on with that copy, saying so.
//! - A new document is made on its first save: `POST api/files?folder=…&name=…`.
//! - A commenter's new comments get the server's ids: the save reply's `ids` are kept and sent
//!   with every later save (`X-Comment-Ids`, comment_ids.rs), so the editor's ids never change
//!   and a save already waiting goes out right. The document is opened again only when the ids
//!   can't be kept or it changed elsewhere ([`Link::reopen`]: waiting saves are dropped first).
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
    /// The signed-in person may comment on this document but not change it (a commenter): the
    /// editor is kept in WordCraft's Commenting state (`view.commenting`, fork change StarDocs
    /// 0010), and the server keeps a save only if its text is unchanged.
    comment_only: bool,
    /// A commenter's comment ids the server gave other ids (ours → the server's, comment_ids.rs),
    /// sent with every save; emptied when the document is read again.
    alias: std::collections::BTreeMap<u32, u32>,
    /// Counts the times the document was (or is being) read again: a save reply whose comment ids
    /// are for an earlier one is not taken.
    generation: u64,
    /// A commenter's document is being read again ([`Link::reopen`]): what is on screen is about
    /// to be replaced, so nothing of it is saved.
    reopening: bool,
    /// File › Make a copy waits for the save on its way (the copy is made from the server's file).
    copy_pending: bool,
    /// Where opened documents go (the app's inbox), kept from [`Link::open`] for version history.
    inbox: Option<Inbox>,
    /// An earlier version shown from version history (read-only, nothing saved), or the way back
    /// to the document under way; `preview_applied`: Viewing has been switched on for it.
    preview: Preview,
    preview_applied: bool,
    /// The server's version history reply, for the panel ([`Link::take_history`]).
    history: Option<Result<Vec<crate::versions::Version>, String>>,
}

#[derive(Default, Clone, PartialEq)]
enum Preview {
    #[default]
    None,
    Showing(String),
    /// The document itself is being read again; nothing is saved until it is.
    Returning,
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
    /// What the title row says once saved.
    pub const SAVED: &str = "Saved to Starbase";
    /// The logo's tooltip: it goes to the home screen.
    pub const HOME_TIP: &str = "Starbase home";

    /// What the title row says once saved (as the desktop's link, which also has local files).
    pub fn saved_text(&self) -> String {
        Self::SAVED.to_string()
    }

    /// Documents on the server can be shared (the desktop's local files can't).
    pub fn can_share(&self) -> bool {
        true
    }

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
                match v["you"].as_str() {
                    Some("view") => link.state.borrow_mut().view_only = true,
                    Some("comment") => link.state.borrow_mut().comment_only = true,
                    _ => {}
                }
                if matches!(v["you"].as_str(), Some("view" | "comment")) {
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

    /// Whether the signed-in person may only comment on this document.
    pub fn comment_only(&self) -> bool {
        self.state.borrow().comment_only
    }

    /// File › Make a copy: once what is on screen is saved, the server copies the file ("Copy of
    /// <name>", beside it, or in your own folder when you can't add files to its folder) and
    /// the copy opens in a new tab.
    pub fn make_copy(&self, app: &mut WordApp) {
        self.state.borrow_mut().copy_pending = true;
        if (app.session.dirty || self.id().is_none()) && !self.view_only() && !self.previewing() {
            self.save_now(app);
        }
        self.ctx.request_repaint();
    }

    /// The copy, once nothing is left to save (see [`Self::make_copy`]).
    fn copy_when_saved(&self, app: &WordApp) {
        let go = {
            let s = self.state.borrow();
            if !s.copy_pending || s.busy || s.waiting.is_some() {
                return;
            }
            s.retry.is_none() && s.id.is_some() && (!app.session.dirty || s.view_only || s.preview != Preview::None)
        };
        if !go {
            // Not saved (a save that failed, or a new document never saved): say so, no copy.
            let failing = self.state.borrow().retry.is_some();
            if failing || !app.session.dirty {
                self.state.borrow_mut().copy_pending = false;
                self.news("Couldn't make a copy: the document isn't saved yet".to_string());
            }
            return;
        }
        self.state.borrow_mut().copy_pending = false;
        let Some(id) = self.id() else { return };
        let link = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match fetch("POST", &api(&format!("api/files/{id}/copy")), None, None).await {
                Ok((200 | 201, _, body)) => {
                    let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                    link.news(format!("Made \"{}\"", v["name"].as_str().unwrap_or("a copy")));
                    if let (Some(new), Some(w)) = (v["id"].as_str(), web_sys::window()) {
                        let _ = w.open_with_url_and_target(&format!("?file={}", enc(new)), "_blank");
                    }
                }
                Ok((code, _, body)) => link.news(format!("Couldn't make a copy ({})", reason(code, &body))),
                Err(e) => link.news(format!("Couldn't reach the server: {e}")),
            }
            link.ctx.request_repaint();
        });
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
            // An earlier version on show (version history) is never saved over the document.
            if !link.previewing() {
                link.save(name, bytes.to_vec());
            }
            Ok(())
        }));
    }

    /// Reads the document from the server into the app (as a dropped file).
    pub fn open(&self, inbox: Inbox) {
        self.state.borrow_mut().inbox = Some(inbox.clone());
        let Some(id) = self.state.borrow().id.clone() else { return };
        let link = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match fetch("GET", &api(&format!("api/files/{id}")), None, None).await {
                Ok((200, headers, bytes)) => {
                    // Back from an earlier version: the document is editable again from here.
                    if link.state.borrow().preview == Preview::Returning {
                        link.state.borrow_mut().preview = Preview::None;
                    }
                    let name = header(&headers, "X-File-Name")
                        .map(|n| serde_json::from_str::<String>(&n).unwrap_or(n))
                        .unwrap_or_else(|| "Document.docx".into());
                    let folder = header(&headers, "X-Folder").map(|n| serde_json::from_str::<String>(&n).unwrap_or(n)).unwrap_or_default();
                    {
                        let mut s = link.state.borrow_mut();
                        s.version = header(&headers, "ETag").unwrap_or_default();
                        s.name = name.clone();
                        s.folder = folder;
                        // The document as the server has it: its ids are the server's.
                        s.alias.clear();
                        s.generation += 1;
                        s.reopening = false;
                    }
                    set_title(&name);
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                    link.ask_role(&id);
                }
                Ok((401, ..)) => go_home(""),
                Ok((code, _, body)) => {
                    link.state.borrow_mut().reopening = false;
                    // Nothing to edit: the blank page is read-only, so typing can't look like work.
                    link.state.borrow_mut().view_only = true;
                    link.fail(&format!("Couldn't open the document ({})", reason(code, &body)));
                }
                Err(e) => {
                    link.state.borrow_mut().reopening = false;
                    link.fail(&format!("Couldn't reach the server: {e}"));
                }
            }
            link.ctx.request_repaint();
        });
    }

    /// A commenter's document read again (changed elsewhere, or comment ids that can't be kept):
    /// the saves waiting or failed are of the document on screen, which is replaced, so they are
    /// dropped (sent, they would name comments by ids the new copy doesn't use), and nothing more
    /// is saved until it is read. Called with no save of ours on its way (from its reply).
    fn reopen(&self) {
        let inbox = {
            let mut s = self.state.borrow_mut();
            s.waiting = None;
            s.retry = None;
            if s.id.is_some() && s.inbox.is_some() {
                s.reopening = true;
                s.generation += 1;
            }
            s.inbox.clone()
        };
        if let Some(inbox) = inbox {
            self.open(inbox);
        }
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
        self.copy_when_saved(app);
        // A commenter: only comments can change (WordCraft refuses the rest), every frame in case
        // the document was opened again (opening ends it).
        if self.comment_only() && !app.session.view.commenting {
            crate::shell::run(app, "view.commenting", serde_json::json!({ "value": true }));
        }
        // Version history shows an earlier version: read-only, and nothing of it is saved. Once
        // the document itself is back, editing is too.
        let (previewing, was_applied) = {
            let mut s = self.state.borrow_mut();
            let was = s.preview_applied;
            s.preview_applied = s.preview != Preview::None;
            (s.preview_applied, was)
        };
        if previewing {
            if !was_applied {
                crate::shell::set_mode(app, crate::shell::Mode::Viewing);
            }
            app.session.dirty = false;
            self.state.borrow_mut().dirty_since = None;
            return;
        }
        if was_applied && !self.view_only() {
            crate::shell::set_mode(app, crate::shell::Mode::Editing);
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

    /// Documents on the server have a version history (the desktop's local files don't).
    pub fn has_history(&self) -> bool {
        true
    }

    /// Asks the server for the version history; the answer comes through [`Self::take_history`].
    pub fn load_history(&self) {
        let Some(id) = self.id() else {
            self.state.borrow_mut().history = Some(Ok(Vec::new()));
            return;
        };
        let link = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let r = match fetch("GET", &api(&format!("api/files/{id}/history")), None, None).await {
                Ok((200, _, body)) => Ok(crate::versions::parse(&serde_json::from_slice(&body).unwrap_or_default())),
                Ok((403, ..)) => Err("Only people who can edit this document can see its version history.".to_string()),
                Ok((code, _, body)) => Err(format!("Couldn't load the version history ({}).", reason(code, &body))),
                Err(e) => Err(format!("Couldn't reach the server: {e}")),
            };
            link.state.borrow_mut().history = Some(r);
            link.ctx.request_repaint();
        });
    }

    pub fn take_history(&self) -> Option<Result<Vec<crate::versions::Version>, String>> {
        self.state.borrow_mut().history.take()
    }

    /// The earlier version shown, if any.
    pub fn previewing(&self) -> bool {
        self.state.borrow().preview != Preview::None
    }

    /// Shows an earlier version, read-only: nothing is saved while it is shown.
    pub fn show_version(&self, v: &str) {
        let (Some(id), Some(inbox)) = (self.id(), self.state.borrow().inbox.clone()) else { return };
        self.state.borrow_mut().preview = Preview::Showing(v.to_string());
        let link = self.clone();
        let v = v.to_string();
        wasm_bindgen_futures::spawn_local(async move {
            match fetch("GET", &api(&format!("api/files/{id}/versions/{}", enc(&v))), None, None).await {
                // Only if it is still the one asked for (another may have been picked since).
                Ok((200, _, bytes)) if link.state.borrow().preview == Preview::Showing(v.clone()) => {
                    let name = link.name();
                    inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                }
                Ok((200, ..)) => {}
                Ok((code, _, body)) => link.news(format!("Couldn't show that version ({})", reason(code, &body))),
                Err(e) => link.news(format!("Couldn't reach the server: {e}")),
            }
            link.ctx.request_repaint();
        });
    }

    /// Back from an earlier version to the document as it is now.
    pub fn end_preview(&self) {
        if !matches!(self.state.borrow().preview, Preview::Showing(_)) {
            return;
        }
        self.state.borrow_mut().preview = Preview::Returning;
        if let Some(inbox) = self.state.borrow().inbox.clone() {
            self.open(inbox);
        }
    }

    /// Puts version `v` back. The server keeps the document as it was as a version first, so the
    /// restore makes a new version and deletes nothing.
    pub fn restore_version(&self, v: &str, label: &str) {
        let Some(id) = self.id() else { return };
        let link = self.clone();
        let (v, label) = (v.to_string(), label.to_string());
        wasm_bindgen_futures::spawn_local(async move {
            match fetch("POST", &api(&format!("api/files/{id}/versions/{}/restore", enc(&v))), None, None).await {
                Ok((200, ..)) => {
                    link.state.borrow_mut().preview = Preview::Returning;
                    if let Some(inbox) = link.state.borrow().inbox.clone() {
                        link.open(inbox);
                    }
                    link.news(format!("Restored the version from {label}"));
                    link.load_history();
                }
                Ok((code, _, body)) => link.news(format!("Couldn't restore that version ({})", reason(code, &body))),
                Err(e) => link.news(format!("Couldn't reach the server: {e}")),
            }
            link.ctx.request_repaint();
        });
    }

    /// Makes a new document from version `v` ("Copy of <name>", beside this one, or in the
    /// person's own folder when they can't add files here) and opens it in a new tab.
    pub fn copy_version(&self, v: &str) {
        let Some(id) = self.id() else { return };
        let link = self.clone();
        let v = v.to_string();
        wasm_bindgen_futures::spawn_local(async move {
            let url = if v == crate::versions::CURRENT { format!("api/files/{id}") } else { format!("api/files/{id}/versions/{}", enc(&v)) };
            let bytes = match fetch("GET", &api(&url), None, None).await {
                Ok((200, _, bytes)) => bytes,
                Ok((code, _, body)) => return link.news(format!("Couldn't copy that version ({})", reason(code, &body))),
                Err(e) => return link.news(format!("Couldn't reach the server: {e}")),
            };
            let (name, folder) = (link.name(), link.state.borrow().folder.clone());
            let copy = format!("Copy of {name}");
            let mut r = link.create(&folder, &copy, &bytes).await;
            if matches!(r, Ok((403, ..))) {
                r = link.create("", &copy, &bytes).await;
            }
            match r {
                Ok((200 | 201, _, body)) => {
                    let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                    let made = v["name"].as_str().unwrap_or(&copy).to_string();
                    link.news(format!("Made \"{made}\""));
                    if let (Some(new), Some(w)) = (v["id"].as_str(), web_sys::window()) {
                        let _ = w.open_with_url_and_target(&format!("?file={}", enc(new)), "_blank");
                    }
                }
                Ok((code, _, body)) => link.news(format!("Couldn't make a copy ({})", reason(code, &body))),
                Err(e) => link.news(format!("Couldn't reach the server: {e}")),
            }
            link.ctx.request_repaint();
        });
    }

    /// Names version `v` (`current`: the document as it is now); an empty name removes it.
    pub fn name_version(&self, v: &str, name: &str) {
        let Some(id) = self.id() else { return };
        let link = self.clone();
        let url = api(&format!("api/files/{id}/versions/{}/name?name={}", enc(v), enc(name)));
        wasm_bindgen_futures::spawn_local(async move {
            match fetch("POST", &url, None, None).await {
                Ok((200, ..)) => link.load_history(),
                Ok((code, _, body)) => link.news(format!("Couldn't name the version ({})", reason(code, &body))),
                Err(e) => link.news(format!("Couldn't reach the server: {e}")),
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
        if self.state.borrow().reopening {
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
                // A newer save went: one that failed before it is out of date.
                let retry = link.send(bytes).await.err().map(|b| (b, now()));
                link.state.borrow_mut().retry = retry;
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
        let (id, version, name, folder, generation, ids) = {
            let s = self.state.borrow();
            // The comment ids the server gave ours, as they are now (a save that waited for the
            // last reply is sent with that reply's ids).
            let ids = if s.comment_only { crate::comment_ids::header(&s.alias) } else { String::new() };
            (s.id.clone(), s.version.clone(), s.name.clone(), s.folder.clone(), s.generation, ids)
        };
        let r = match &id {
            Some(id) => {
                let extra = (!ids.is_empty()).then_some(("X-Comment-Ids", ids.as_str()));
                fetch_with("PUT", &api(&format!("api/files/{id}")), Some(&version), extra, Some(&bytes)).await
            }
            None => self.create(&folder, &name, &bytes).await,
        };
        let why = match r {
            Ok((200 | 201, _, body)) => {
                self.took(&body);
                // A commenter's new comments got other ids on the server (commenting.rs): kept
                // and sent with later saves. Only when they can't be (the reply unreadable, too
                // many) is the document opened again. A reply for a document since read again
                // is left: its ids are the server's already.
                if self.comment_only() {
                    let v = serde_json::from_slice::<serde_json::Value>(&body).unwrap_or_default();
                    let kept = {
                        let mut s = self.state.borrow_mut();
                        s.generation != generation || (crate::comment_ids::apply(&mut s.alias, &v["ids"]) && v["reopen"] != true)
                    };
                    if !kept {
                        self.reopen();
                    }
                }
                return Ok(());
            }
            // A commenter can't keep a copy beside it (it isn't their folder), and their comments
            // are matched to the version they were made on: the document is opened again.
            Ok((409, ..)) if self.comment_only() => {
                self.news(
                    "Someone changed this document while you were commenting, so it was opened again. Add any comment that is missing once more."
                        .to_string(),
                );
                self.reopen();
                return Ok(());
            }
            Ok((409, ..)) => {
                let (stem, ext) = split(&name);
                // The same name Starbase Sheets uses; the server makes it free.
                let copy = format!("{stem} (conflicted copy){ext}");
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

fn set_title(name: &str) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
        doc.set_title(&format!("{stem} - Starbase Docs"));
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
    fetch_with(method, url, if_match, None, body).await
}

/// [`fetch`] with one more header (`extra`: name, value).
async fn fetch_with(
    method: &str,
    url: &str,
    if_match: Option<&str>,
    extra: Option<(&str, &str)>,
    body: Option<&[u8]>,
) -> Result<(u16, web_sys::Headers, Vec<u8>), String> {
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    let headers = web_sys::Headers::new().map_err(|e| format!("{e:?}"))?;
    if let Some(v) = if_match.filter(|v| !v.is_empty()) {
        headers.set("If-Match", v).map_err(|e| format!("{e:?}"))?;
    }
    if let Some((name, value)) = extra {
        headers.set(name, value).map_err(|e| format!("{e:?}"))?;
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
