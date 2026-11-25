//! The Starbase server link: when Starbase Slides is opened from the Starbase home screen
//! (`slides/?file=<id>`, or `slides/?new=1&folder=<folder>` for a new presentation) it reads and
//! saves the presentation through the server's files API (`api/files`, one folder up from this
//! page) instead of the browser's downloads. The same design as StarDocs' `server.rs`, and the
//! same DeckCraft hook as the host bridge (`Services::host_save`).
//!
//! - Open: `GET api/files/<id>`; the reply's `ETag` is the file's version on disk. The bytes open
//!   with their name (`SlideApp::open_from_host`), so the presentation's file is that .pptx and
//!   every Save hands back a .pptx under the same name.
//! - New: the presentation is made on the server straight away, as a named .pptx
//!   ("Untitled presentation.pptx", `POST api/files?folder=…&name=…`), and then opened like any
//!   other. DeckCraft would save an untitled presentation as `<title>.deckcraft`; Starbase files
//!   stay .pptx, so there is never an untitled one, and a `.deckcraft` save is refused.
//! - Save: `PUT api/files/<id>` with `If-Match: <version>`, one at a time; a save made while one
//!   is on its way waits, and only the newest waiting one is sent.
//! - Changed elsewhere (409): the server's file is never overwritten. This version is kept as a
//!   new file beside it, "<name> (conflicted copy).pptx", and the page carries on with that copy.
//! - Not signed in (401): the page goes to the home screen's sign-in, which comes back to this link.
//! - Exports (PDF, pictures, outlines) still download in the browser.
//!
//! The page saves by itself ([`Link::tick`]) a couple of seconds after the last change, and the
//! title row shows "Saving…" / "Saved to Starbase". A failed save is tried again every
//! [`RETRY_MS`]; leaving the page asks first while anything is not yet on the server.

use std::cell::RefCell;
use std::rc::Rc;

use deckcraft_ui_egui::{Services, SlideApp};
use serde_json::json;
use wasm_bindgen::JsCast as _;
use wasm_bindgen_futures::JsFuture;

use crate::shell::{Drive as _, Saved};

/// How long a message stays in the title row (milliseconds).
const NOTICE_MS: f64 = 20_000.0;
/// How long the presentation must sit unchanged before it saves by itself (milliseconds).
const QUIET_MS: f64 = 2000.0;
/// How long after a failed save it is tried again (milliseconds).
const RETRY_MS: f64 = 15_000.0;
/// The name a new presentation gets (the server makes it free: "… (2).pptx").
/// The home screen's preview picture: the first slide, this wide (the server takes up to 1440 x 960).
const THUMB_W: f64 = 720.0;
/// At most one new picture this often while someone keeps editing (milliseconds).
const THUMB_MS: f64 = 30_000.0;
pub const NEW_NAME: &str = "Untitled presentation.pptx";

thread_local! {
    static CURRENT: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) };
}

/// Whether something is not on the server yet: a save on its way, waiting, or one that failed.
pub fn unsaved() -> bool {
    CURRENT.with(|c| {
        c.borrow().as_ref().is_some_and(|s| {
            let s = s.borrow();
            s.busy || s.waiting.is_some() || s.retry.is_some() || s.dirty
        })
    })
}

#[derive(Default)]
struct State {
    /// The server's id for the file; `None` until a new presentation is made.
    id: Option<String>,
    /// `?new=1`: make a new presentation in `folder` first.
    make_new: bool,
    folder: String,
    /// The file's version on disk, as the server last told us.
    version: String,
    /// The file's name on the server.
    name: String,
    busy: bool,
    waiting: Option<Vec<u8>>,
    saved: Saved,
    /// Messages for the status line, and the newest one for the title row.
    news: Vec<String>,
    notice: Option<(String, f64)>,
    /// When the presentation was first seen changed since the last save (ms).
    dirty_since: Option<f64>,
    /// The presentation has unsaved changes (for the leave-page guard).
    dirty: bool,
    retry: Option<(Vec<u8>, f64)>,
    /// The signed-in person's name, until the page signs comments with it.
    author: Option<String>,
    view_only: bool,
    /// Files read from the server, waiting to open in the app on the next frame.
    opened: Vec<(String, Vec<u8>)>,
    /// When the first-slide preview picture is due (ms), and when the last one went.
    thumb_due: Option<f64>,
    thumb_last: f64,
}

#[derive(Clone)]
pub struct Link {
    state: Rc<RefCell<State>>,
    ctx: egui::Context,
    /// The Share and Version history dialogs.
    dialogs: Rc<RefCell<(crate::dialogs::Share, crate::dialogs::History)>>,
}

fn query_param(key: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search).ok()?.get(key)
}

/// The server's address for `path` (`api/…`): Starbase Slides sits in `slides/` beside the home page.
pub(crate) fn api(path: &str) -> String {
    format!("../{path}")
}

pub(crate) fn enc(s: &str) -> String {
    js_sys::encode_uri_component(s).as_string().unwrap_or_default()
}

fn now() -> f64 {
    js_sys::Date::now()
}

/// Whether a saved file is the presentation itself (a PowerPoint package) rather than an export.
pub fn is_presentation(name: &str) -> bool {
    let lower = name.to_lowercase();
    [".pptx", ".potx", ".ppsx"].iter().any(|e| lower.ends_with(e))
}

impl Link {
    /// The link for this page, or `None` when it wasn't opened from the home screen.
    pub fn detect(ctx: egui::Context) -> Option<Self> {
        let id = query_param("file").filter(|s| !s.is_empty());
        let make_new = id.is_none() && query_param("new").is_some();
        if id.is_none() && !make_new {
            return None;
        }
        let folder = query_param("folder").unwrap_or_default();
        let state = Rc::new(RefCell::new(State { id, make_new, folder, ..Default::default() }));
        CURRENT.with(|c| *c.borrow_mut() = Some(state.clone()));
        let link = Self { state, ctx, dialogs: Rc::default() };
        link.ask_who();
        Some(link)
    }

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

    fn ask_role(&self, id: &str) {
        let link = self.clone();
        let url = api(&format!("api/files/{id}/sharing"));
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok((200, _, body)) = fetch("GET", &url, None, None).await {
                let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                if v["you"].as_str() == Some("view") {
                    link.state.borrow_mut().view_only = true;
                    link.ctx.request_repaint();
                }
            }
        });
    }

    /// A presentation without a preview picture (made elsewhere, or never pictured) gets one now.
    fn ask_thumb(&self, id: &str) {
        let link = self.clone();
        let url = api(&format!("api/files/{id}/thumb"));
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok((404, ..)) = fetch("GET", &url, None, None).await {
                link.state.borrow_mut().thumb_due = Some(now());
                link.ctx.request_repaint();
            }
        });
    }

    /// Sends the preview picture (the first slide) once it is due and everything is saved, drawn
    /// from what is open, which then matches the version the server has.
    fn maybe_thumb(&self, app: &SlideApp, ctx: &egui::Context, t: f64) {
        let (id, version) = {
            let s = self.state.borrow();
            let Some(due) = s.thumb_due else { return };
            if s.view_only {
                return;
            }
            if due > t || s.busy || s.waiting.is_some() || s.retry.is_some() || s.dirty {
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
        let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
        if doc.slides.is_empty() {
            return;
        }
        let scale = THUMB_W / doc.slide_size.width.max(1.0);
        let png = deckcraft_render::render_slide(&doc, 0, &deckcraft_render::RenderOpts { scale, ..Default::default() }).to_png();
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

    /// Saves go to the server as .pptx; exports still download (`download` stays the browser's).
    pub fn connect(&self, services: &mut Services) {
        let link = self.clone();
        services.host_save = Some(Box::new(move |name: &str, bytes: &[u8], _save_as: bool| {
            if is_presentation(name) {
                link.save(bytes.to_vec());
                return Ok(());
            }
            if name.to_lowercase().ends_with(".deckcraft") {
                return Err(
                    "Starbase keeps presentations as PowerPoint files (.pptx), so this one is saved as .pptx. Other formats: File › Download.".into(),
                );
            }
            Err(format!("{name} isn't a presentation file"))
        }));
    }

    /// Reads the presentation from the server (or makes the new one first, from the app's blank
    /// presentation as .pptx bytes) and queues it to open.
    pub fn open(&self, app: &mut SlideApp) {
        let make_new = std::mem::take(&mut self.state.borrow_mut().make_new);
        let link = self.clone();
        if make_new {
            let bytes = app.session.active().map(|d| d.doc.clone()).map(|doc| deckcraft_engine::cmd::file::save_bytes(&doc, "pptx"));
            let Some(Ok(bytes)) = bytes else {
                self.fail("Couldn't make a new presentation");
                return;
            };
            wasm_bindgen_futures::spawn_local(async move { link.create_new(bytes).await });
            return;
        }
        let Some(id) = self.id() else { return };
        wasm_bindgen_futures::spawn_local(async move { link.read(&id).await });
    }

    async fn create_new(&self, bytes: Vec<u8>) {
        let folder = self.state.borrow().folder.clone();
        let mut r = self.create(&folder, NEW_NAME, &bytes).await;
        // Not allowed to add files here (a shared folder one may only view): the person's own folder.
        if matches!(r, Ok((403, ..))) {
            self.state.borrow_mut().folder.clear();
            r = self.create("", NEW_NAME, &bytes).await;
        }
        match r {
            Ok((200 | 201, _, body)) => {
                self.took(&body);
                let name = self.name();
                self.state.borrow_mut().opened.push((name, bytes));
                self.state.borrow_mut().saved = Saved::Done;
            }
            Ok((401, ..)) => sign_in_and_return(),
            Ok((code, _, body)) => self.fail(&format!("Couldn't make the presentation ({})", reason(code, &body))),
            Err(e) => self.fail(&format!("Couldn't reach the server: {e}")),
        }
        self.ctx.request_repaint();
    }

    async fn read(&self, id: &str) {
        match fetch("GET", &api(&format!("api/files/{id}")), None, None).await {
            Ok((200, headers, bytes)) => {
                let name = header(&headers, "X-File-Name")
                    .map(|n| serde_json::from_str::<String>(&n).unwrap_or(n))
                    .unwrap_or_else(|| "Presentation.pptx".into());
                let folder = header(&headers, "X-Folder").map(|n| serde_json::from_str::<String>(&n).unwrap_or(n)).unwrap_or_default();
                {
                    let mut s = self.state.borrow_mut();
                    s.version = header(&headers, "ETag").unwrap_or_default();
                    s.name = name.clone();
                    s.folder = folder;
                    s.opened.push((name.clone(), bytes));
                }
                set_title(&name);
                self.ask_role(id);
                self.ask_thumb(id);
            }
            Ok((401, ..)) => sign_in_and_return(),
            Ok((code, _, body)) => {
                self.state.borrow_mut().view_only = true;
                self.fail(&format!("Couldn't open the presentation ({})", reason(code, &body)));
            }
            Err(e) => self.fail(&format!("Couldn't reach the server: {e}")),
        }
        self.ctx.request_repaint();
    }

    pub fn id(&self) -> Option<String> {
        self.state.borrow().id.clone()
    }

    /// Every frame, before DeckCraft's: open what the server sent, sign comments with the
    /// signed-in person's name, show news.
    pub fn before(&self, app: &mut SlideApp) {
        let opened = std::mem::take(&mut self.state.borrow_mut().opened);
        for (name, bytes) in opened {
            app.open_from_host(Some(&name), &bytes);
        }
        // The signed-in person's name signs new comments, as `?author=` does in host mode.
        let author = self.state.borrow_mut().author.take();
        if let Some(author) = author {
            crate::host::Bridge::set_author(app, &author);
        }
        let news = std::mem::take(&mut self.state.borrow_mut().news);
        for msg in news {
            let mut s = self.state.borrow_mut();
            if s.saved != Saved::Failed(msg.clone()) {
                s.notice = Some((msg.clone(), now()));
            }
            drop(s);
            app.set_status(msg);
        }
    }

    /// Every frame, after DeckCraft's: the save made by itself once the presentation has been
    /// left alone for a moment, and failed saves tried again.
    pub fn tick(&self, app: &mut SlideApp, ctx: &egui::Context) {
        let t = now();
        let again = {
            let mut s = self.state.borrow_mut();
            match &s.retry {
                Some((_, at)) if !s.busy && t - at >= RETRY_MS => s.retry.take().map(|(bytes, _)| bytes),
                Some(_) => {
                    ctx.request_repaint_after(std::time::Duration::from_secs(1));
                    None
                }
                None => None,
            }
        };
        if let Some(bytes) = again {
            self.save(bytes);
        }
        let dirty = app.session.documents().iter().any(|d| d.is_dirty());
        {
            let mut s = self.state.borrow_mut();
            s.dirty = dirty && !s.view_only;
        }
        self.maybe_thumb(app, ctx, t);
        let mut s = self.state.borrow_mut();
        if !dirty || s.view_only || s.id.is_none() || app.show.is_some() {
            s.dirty_since = None;
            return;
        }
        let since = *s.dirty_since.get_or_insert(t);
        drop(s);
        if t - since >= QUIET_MS && !ctx.input(|i| i.pointer.any_down()) {
            self.state.borrow_mut().dirty_since = None;
            if let Err(e) = app.run("file.save", json!({})) {
                self.news(format!("Not saved: {e}"));
            }
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    fn news(&self, msg: String) {
        self.state.borrow_mut().news.push(msg);
        self.ctx.request_repaint();
    }

    fn fail(&self, msg: &str) {
        let mut s = self.state.borrow_mut();
        s.saved = Saved::Failed(msg.to_string());
        s.news.push(msg.to_string());
    }

    fn save(&self, bytes: Vec<u8>) {
        if self.view_only_now() {
            self.news("You can view this presentation but not change it, so nothing is saved.".to_string());
            return;
        }
        {
            let mut s = self.state.borrow_mut();
            s.saved = Saved::Saving;
            s.retry = None;
            if s.busy {
                s.waiting = Some(bytes);
                return;
            }
            s.busy = true;
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

    fn view_only_now(&self) -> bool {
        self.state.borrow().view_only
    }

    /// One save: the file itself, or (changed elsewhere) a kept copy. A failed save hands its
    /// bytes back, to try again.
    async fn send(&self, bytes: Vec<u8>) -> Result<(), Vec<u8>> {
        let (id, version, name, folder) = {
            let s = self.state.borrow();
            (s.id.clone(), s.version.clone(), s.name.clone(), s.folder.clone())
        };
        let Some(id) = id else {
            self.fail("Not saved: the presentation isn't on the server yet");
            return Err(bytes);
        };
        let why = match fetch("PUT", &api(&format!("api/files/{id}")), Some(&version), Some(&bytes)).await {
            Ok((200 | 201, _, body)) => {
                self.took(&body);
                return Ok(());
            }
            Ok((409, ..)) => {
                let (stem, ext) = split(&name);
                let copy = format!("{stem} (conflicted copy){ext}");
                match self.create(&folder, &copy, &bytes).await {
                    Ok((201 | 200, _, body)) => {
                        self.took(&body);
                        let shown = self.name();
                        self.news(format!(
                            "Someone else changed \"{name}\" while you were editing. Their version is untouched; yours is kept as \"{shown}\", which is open now."
                        ));
                        return Ok(());
                    }
                    Ok((code, _, body)) => format!("Not saved: the file changed elsewhere and a copy couldn't be made ({})", reason(code, &body)),
                    Err(e) => format!("Not saved: {e}"),
                }
            }
            Ok((401, ..)) => "Not saved: you're signed out. Sign in on the home screen in another tab".to_string(),
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
        s.thumb_due = Some((s.thumb_last + THUMB_MS).max(now()));
        if let Saved::Failed(_) = s.saved {
            s.saved = Saved::Saving;
        }
        let name = s.name.clone();
        drop(s);
        set_title(&name);
    }
}

impl crate::shell::Drive for Link {
    fn file_id(&self) -> Option<String> {
        self.id()
    }

    fn share(&self) {
        if let Some(id) = self.id() {
            let name = self.name();
            self.dialogs.borrow_mut().0.start(&id, &name, &self.ctx);
        }
    }

    fn history(&self) {
        if let Some(id) = self.id() {
            self.dialogs.borrow_mut().1.start(&id, &self.ctx);
        }
    }

    fn dialogs(&self, ctx: &egui::Context) {
        let mut d = self.dialogs.borrow_mut();
        d.0.ui(ctx);
        d.1.ui(ctx);
    }

    fn name(&self) -> String {
        self.state.borrow().name.clone()
    }

    fn saved(&self) -> Saved {
        self.state.borrow().saved.clone()
    }

    fn notice(&self) -> Option<String> {
        let s = self.state.borrow();
        s.notice.as_ref().filter(|(_, at)| now() - at < NOTICE_MS).map(|(m, _)| m.clone())
    }

    fn view_only(&self) -> bool {
        self.state.borrow().view_only
    }

    fn home(&self) {
        let folder = self.state.borrow().folder.clone();
        go_home(&folder);
    }

    fn new_presentation(&self) {
        let folder = self.state.borrow().folder.clone();
        if let Some(w) = web_sys::window() {
            let _ = w.open_with_url_and_target(&format!("?new=1&folder={}", enc(&folder)), "_blank");
        }
    }

    fn rename(&self, stem: &str) {
        let Some(id) = self.id() else { return };
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
}

fn split(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), ".pptx".to_string()),
    }
}

fn set_title(name: &str) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
        doc.set_title(&format!("{stem} - Starbase Slides"));
    }
}

/// Not signed in: the home screen's sign-in page, which brings the person back to this link
/// (`?next=`, as Docs does) instead of leaving them on the file list.
fn sign_in_and_return() {
    if let Some(w) = web_sys::window() {
        let loc = w.location();
        let here = format!("{}{}", loc.pathname().unwrap_or_default(), loc.search().unwrap_or_default());
        let _ = loc.replace(&format!("../?next={}", enc(&here)));
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
