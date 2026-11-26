//! Files on the StarSheets file server (crates/server, under /api/ on the same site), the way
//! Google Drive and Sheets work: a spreadsheet saves itself a moment after each change (a new one
//! as soon as any cell has something in it, as "Untitled spreadsheet"), the address bar keeps its
//! file (`?file=<id>`) so a reload reopens it, and File › Open lists the files in the folder.
//! The server picks free names; a save based on an older copy (the same file changed in another
//! tab) never overwrites: it becomes "<name> (conflicted copy <date>)".

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use egui::{RichText, Ui, vec2};
use gridcraft_engine::model::Workbook;
use gridcraft_ui_egui::SheetApp;
use serde_json::{Value as Json, json};
use wasm_bindgen::JsCast as _;

pub(crate) use crate::store::{has_data, xlsx};

/// Asks for the Open dialog, the version list, or another file action (menus set it in egui memory).
pub const REQUEST: &str = "starsheets_files_request";
/// The hover text for the status line: where and when the file was saved.
pub const STATUS_TIP: &str = "starsheets_save_status_tip";
/// "Last edit was 5 minutes ago (14:05) by Ada", for the save symbol's hover text (to-do 3 row 8).
pub const LAST_EDIT: &str = "starsheets_last_edit";
const API: &str = "api/files";
/// Quiet time after the last change before saving (ms).
const SAVE_AFTER: f64 = 1500.0;

#[derive(Clone)]
struct Link {
    doc: u64,
    id: String,
    name: String,
    version: String,
    /// Its folder on the server ("" = the top) and when it was last saved (Unix seconds).
    folder: String,
    saved: i64,
}

enum Reply {
    Saved {
        doc: u64,
        sent: Arc<Workbook>,
        link: Link,
    },
    Conflict {
        doc: u64,
        sent: Arc<Workbook>,
        bytes: Vec<u8>,
    },
    Failed(String),
    Listed(Vec<crate::filebrowser::FileInfo>, Vec<String>),
    Bin(Vec<crate::filebrowser::BinInfo>),
    /// A file or folder changed: list again.
    Refresh,
    /// A folder was made: list again and start renaming it.
    NewFolder(String),
    /// A file was renamed from the file window.
    RenamedId(String, String),
    Download(String, Vec<u8>),
    /// Version history's list, or why it couldn't be had.
    History(Result<Vec<crate::versions::Version>, String>),
    /// A version's content (its id, the .xlsx bytes), to show read-only.
    VersionBytes(String, Result<Vec<u8>, String>),
    /// A version was named: list again.
    HistoryReload,
    /// Restoring, naming or copying a version went wrong: said in the history panel.
    HistoryError(String),
    Opened {
        id: String,
        name: String,
        version: String,
        bytes: Vec<u8>,
        folder: String,
        modified: i64,
    },
    /// How the server describes where files are kept, whether sign-in is needed, and who is
    /// signed in ("" with sign-in off).
    Info(String, bool, String),
    Renamed(String),
    Navigate(String),
    /// An import saved to the server, to open in a new tab.
    OpenTab(String),
}

#[derive(Default)]
pub struct Files {
    link: Option<Link>,
    inbox: Rc<RefCell<Vec<Reply>>>,
    busy: bool,
    booted: bool,
    seen: Option<(u64, usize)>,
    changed_at: f64,
    status: String,
    error: Option<String>,
    /// File › Open, and the keys it took from the grid this frame.
    browser: crate::filebrowser::Browser,
    browser_keys: Vec<egui::Event>,
    /// Open the file window in the open file's folder once the list arrives.
    browser_to_file: bool,
    /// Where the server keeps files, for the status line's hover text (GET /api/info).
    location: String,
    /// The folder a new spreadsheet is saved in (`?folder=` from "New spreadsheet" in a folder).
    new_folder: String,
    /// File › Version history: the full-window history view (versions.rs).
    pub history: crate::versions::History,
    /// A rename sent to the server and not answered yet.
    renaming: bool,
    /// File › Details is showing.
    details: bool,
    /// A big file that arrived and is opened next frame, after "Opening …" has been drawn once:
    /// reading it keeps the page busy for a few seconds (two million cells: about three).
    opening: Option<(Reply, bool)>,
    /// Save the open workbook now even with nothing in it (Share on a new, empty spreadsheet:
    /// sharing needs the file on the server, as in Sheets, where it exists from the start).
    save_now: bool,
    /// Who is signed in ("" with sign-in off), once /api/info has answered: the sheet each
    /// person was last on is remembered per file and person.
    user: Option<String>,
    /// A file just opened: switch to the sheet this person was last on (document uid).
    restore: Option<u64>,
    /// The sheet last remembered: (document uid, sheet name, the file's sheet id).
    remembered: Option<(u64, String, u32)>,
}

/// Asks [`Files`] to save the open workbook at once, empty or not (`ctx.data`, unit value).
pub const SAVE_NOW: &str = "starsheets_files_save_now";

impl Files {
    /// The open file's folder on the server, if it is on the server.
    pub fn folder(&self) -> Option<String> {
        self.link.as_ref().map(|l| l.folder.clone())
    }

    /// The open file on the server: (document uid, file id).
    pub fn linked(&self) -> Option<(u64, String)> {
        self.link.as_ref().map(|l| (l.doc, l.id.clone()))
    }

    /// The version (ETag) of the file as it was opened, for live editing's Hello.
    pub fn linked_version(&self) -> Option<String> {
        self.link.as_ref().map(|l| l.version.clone()).filter(|v| !v.is_empty())
    }

    pub fn logic(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        // A new workbook is "Untitled spreadsheet", as in Sheets (GridCraft calls it Book1). The
        // workbook appears after the first frames, so this is checked every frame.
        if self.link.is_none()
            && let Some(d) = app.session.active_mut()
            && d.path.is_none()
            && d.title.strip_prefix("Book").is_some_and(|n| n.chars().all(|c| c.is_ascii_digit()))
        {
            d.title = "Untitled spreadsheet".into();
        }
        if !self.booted {
            self.booted = true;
            self.new_folder = query_param("folder").unwrap_or_default();
            send(Method::Get, "api/info".into(), None, None, self.inbox.clone(), ctx.clone(), |_, _, body| {
                let v: Json = serde_json::from_slice(&body).unwrap_or_default();
                let locked = v["auth"].as_bool().unwrap_or(false) && !v["signed_in"].as_bool().unwrap_or(true);
                let user = v["user"]["username"].as_str().unwrap_or("").to_string();
                Reply::Info(v["location"].as_str().unwrap_or("the server").to_string(), locked, user)
            });
            if let Some(id) = query_file() {
                self.status = "Opening…".into();
                self.busy = true;
                get_file(id, self.inbox.clone(), ctx.clone());
            }
        }
        if let Some((r, shown)) = self.opening.take() {
            if shown {
                self.handle(app, ctx, r);
            } else {
                self.opening = Some((r, false));
            }
        }
        let replies: Vec<Reply> = std::mem::take(&mut *self.inbox.borrow_mut());
        for r in replies {
            if let Reply::Opened { bytes, .. } = &r
                && bytes.len() >= OPENING_NOTICE_FROM
            {
                self.opening = Some((r, false));
                ctx.request_repaint();
                continue;
            }
            self.handle(app, ctx, r);
        }
        if let (Some((name, folder)), Some(l)) = (crate::live::take_moved(), self.link.as_mut()) {
            l.name = name;
            l.folder = folder;
        }
        // While the file window is open it takes its keys before the grid sees them.
        if self.browser.open
            && !self.browser.typing()
            && !ctx.memory(|m| m.focused()).is_some_and(|f| f == egui::Id::unique("starsheets_files_search"))
        {
            ctx.input_mut(|i| {
                let mut keep = Vec::new();
                for ev in std::mem::take(&mut i.events) {
                    let ours = matches!(&ev, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_))
                        || matches!(&ev, egui::Event::Key { key, modifiers, .. } if matches!(key, egui::Key::Enter | egui::Key::F2 | egui::Key::Delete | egui::Key::Backspace) || (*key == egui::Key::A && modifiers.command));
                    if ours { self.browser_keys.push(ev) } else { keep.push(ev) }
                }
                i.events = keep;
            });
        }
        if let Some(req) = ctx.data_mut(|m| m.remove_temp::<String>(egui::Id::unique(REQUEST))) {
            self.request(app, ctx, &req);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O))) {
            self.request(app, ctx, "open");
        }
        if ctx.data_mut(|m| m.remove_temp::<()>(egui::Id::unique(SAVE_NOW))).is_some() {
            self.save_now = true;
        }
        self.autosave(app, ctx);
        self.rename_if_needed(app, ctx);
        self.last_sheet(app);
        let live = app.session.active().and_then(|d| crate::live::status_for(d.uid));
        let unsaved = match &live {
            Some(l) => l.unsaved,
            None => self.busy || app.session.active().is_some_and(|d| self.needs_save(d)),
        };
        set_unsaved_flag(unsaved);
        if let (Some(l), Some(link)) = (&live, &mut self.link)
            && let Some(at) = l.saved_at
        {
            link.saved = at;
        }
        let last_edit = live.as_ref().and_then(|l| l.last_edit.clone()).map(|e| format!("Last edit by {}", e.by)).unwrap_or_default();
        let when = self
            .link
            .as_ref()
            .filter(|l| app.session.active().is_some_and(|d| d.uid == l.doc) && l.saved > 0)
            .map(|l| ago(l.saved))
            .unwrap_or_default();
        use crate::shell::{SaveState, SaveStatus};
        let status = if let Some(l) = live {
            Some(l.status)
        } else if let Some(e) = &self.error {
            Some(SaveStatus::new(SaveState::Problem, format!("Not saved: {e}")))
        } else if unsaved {
            Some(SaveStatus::new(SaveState::Saving, "Saving…"))
        } else if self.link.as_ref().is_some_and(|l| app.session.active().is_some_and(|d| d.uid == l.doc)) {
            Some(SaveStatus::new(SaveState::Saved, "Saved"))
        } else if !self.status.is_empty() {
            // "Opening…": nothing is wrong yet.
            Some(SaveStatus::new(SaveState::Saving, self.status.clone()))
        } else {
            None
        };
        let tip = self.status_tip(app, unsaved);
        crate::shell::set_save_status(ctx, status);
        ctx.data_mut(|m| {
            m.insert_temp(egui::Id::unique(STATUS_TIP), tip);
            m.insert_temp(egui::Id::unique(LAST_EDIT), last_edit);
            m.insert_temp(egui::Id::unique(crate::shell::SAVE_WHEN), when);
        });
    }

    /// Reopens a file on the sheet this person was last on in it (as Sheets does), and
    /// remembers in the browser which sheet that is whenever it changes. The file itself opens
    /// on the sheet of whoever changed it last.
    fn last_sheet(&mut self, app: &mut SheetApp) {
        let (Some(user), Some(link)) = (&self.user, &self.link) else { return };
        let Some(d) = app.session.active().filter(|d| d.uid == link.doc) else { return };
        let key = format!("starsheets_sheet_{}_{user}", link.id);
        let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten());
        if self.restore.take() == Some(link.doc)
            && let Some(v) = storage.as_ref().and_then(|s| s.get_item(&key).ok().flatten()).and_then(|t| serde_json::from_str::<Json>(&t).ok())
        {
            let file_id = v["id"].as_u64().and_then(|i| u32::try_from(i).ok()).unwrap_or(0);
            let at = starsheets_sync::pick_sheet(&d.wb, file_id, v["name"].as_str().unwrap_or(""));
            if at != d.wb.active_sheet() {
                let _ = app.run("sheet.activate", json!({ "sheet": at }));
                app.after_engine();
            }
        }
        let Some(d) = app.session.active().filter(|d| d.uid == link.doc) else { return };
        let Some(sh) = d.wb.sheet(d.wb.active_sheet()) else { return };
        let now = (d.uid, sh.name.clone(), sh.file_id);
        if self.remembered.as_ref() != Some(&now) {
            if let Some(s) = &storage {
                let _ = s.set_item(&key, &json!({ "id": now.2, "name": now.1 }).to_string());
            }
            self.remembered = Some(now);
        }
    }

    fn request(&mut self, app: &mut SheetApp, ctx: &egui::Context, req: &str) {
        match req {
            "open" => {
                // With accounts, everyone starts in their own folder (`<username>/`).
                let home = crate::account::me().map(|m| m.username).unwrap_or_default();
                self.browser.show_in(&home);
                self.browser_to_file = true;
                list_files(self.inbox.clone(), ctx.clone());
            }
            "new" => {
                open_url(&format!("{}?new=1", page_url("")));
            }
            // File › New from a template…: the Drive's gallery in a new tab, for this file's folder.
            "templates" => {
                let dir = self.link.as_ref().map(|l| format!("&dir={}", enc(&l.folder))).unwrap_or_default();
                open_url(&format!("{}?templates=1{dir}", page_url("")));
            }
            // The home screen, in this file's folder.
            // The Files button: the home screen in a new tab, at this file's folder.
            "home_tab" => {
                let url = match self.link.as_ref().filter(|l| !l.folder.is_empty()) {
                    Some(l) => format!("{}?dir={}", page_url(""), enc(&l.folder)),
                    None => page_url(""),
                };
                open_url(&url);
            }
            "home" => match self.link.as_ref().filter(|l| !l.folder.is_empty()) {
                Some(l) => {
                    if let Some(w) = web_sys::window() {
                        let _ = w.location().set_href(&format!("{}?dir={}", page_url(""), enc(&l.folder)));
                    }
                }
                None => go_to(""),
            },
            "versions" => {
                if self.link.is_some() {
                    self.history.show();
                } else {
                    app.message = Some(("Version history".into(), "This spreadsheet has no saved versions yet.".into()));
                }
            }
            "copy" => {
                let Some(d) = app.session.active() else { return };
                let Ok(bytes) = xlsx(&d.wb, "copy.xlsx") else { return };
                let name = format!("Copy of {}", d.title);
                // A copy of a spreadsheet whose web content is blocked stays blocked (external.rs).
                let outside = if crate::external::allowed() { "" } else { "&imported=1" };
                send(
                    Method::Post,
                    format!("{API}?name={}{outside}", enc(&name)),
                    Some(bytes),
                    None,
                    self.inbox.clone(),
                    ctx.clone(),
                    |_, _, body| {
                        let v: Json = serde_json::from_slice(&body).unwrap_or_default();
                        Reply::Navigate(v["id"].as_str().unwrap_or("").to_string())
                    },
                );
            }
            "details" => self.details = true,
            "bin" => {
                let Some(l) = self.link.clone() else {
                    app.message = Some(("Move to bin".into(), "This spreadsheet hasn't been saved yet.".into()));
                    return;
                };
                send(
                    Method::Delete,
                    format!("{API}/{}", l.id),
                    None,
                    None,
                    self.inbox.clone(),
                    ctx.clone(),
                    |_, _, _| Reply::Navigate(String::new()),
                );
            }
            _ => {}
        }
    }

    fn handle(&mut self, app: &mut SheetApp, ctx: &egui::Context, r: Reply) {
        match r {
            Reply::Saved { doc, sent, link } => {
                self.busy = false;
                self.error = None;
                if let Some(d) = app.session.active_mut().filter(|d| d.uid == doc) {
                    d.saved = sent;
                    d.title = stem(&link.name);
                    // An imported file keeps its own name only until it is on the server.
                    d.path = None;
                }
                set_query_file(&link.id);
                self.link = Some(link);
            }
            Reply::Conflict { doc, sent, bytes } => {
                // Someone saved this file elsewhere since we opened it: keep both.
                // No date in the name: the server adds " (2)" if this name is taken too.
                let name = format!("{} (conflicted copy)", self.link.as_ref().map_or("Untitled spreadsheet".into(), |l| stem(&l.name)));
                app.message = Some((
                    "Saved as a copy".into(),
                    format!(
                        "Your latest changes couldn't be added to the file, because it was saved by someone else (or by Starbase after it restarted) since this tab last had it. Nothing is lost: your changes were saved as \"{name}\", next to it."
                    ),
                ));
                self.link = None;
                post_new(doc, sent, bytes, name, &self.new_folder, self.inbox.clone(), ctx.clone());
            }
            Reply::Failed(e) if e == "please sign in" => {
                self.busy = false;
                crate::account::ask_sign_in();
            }
            Reply::Failed(e) => {
                self.busy = false;
                self.renaming = false;
                if self.browser.open && !e.is_empty() {
                    self.browser.error = Some(e);
                    self.refresh(ctx);
                } else if !e.is_empty() {
                    self.error = Some(e);
                }
            }
            Reply::Listed(files, folders) => {
                if self.browser_to_file {
                    self.browser_to_file = false;
                    if let Some(f) = self.link.as_ref().and_then(|l| files.iter().find(|f| f.id == l.id)) {
                        self.browser.show_in(&f.folder.clone());
                    }
                }
                self.browser.files = files;
                self.browser.folders = folders;
                self.browser.loading = false;
            }
            Reply::Bin(items) => {
                self.browser.bin = items;
                self.browser.loading = false;
            }
            Reply::Refresh => self.refresh(ctx),
            Reply::Info(location, locked, user) => {
                self.location = location;
                self.user = Some(user);
                // Signing in is account.rs's (it asks /api/info itself).
                let _ = locked;
            }
            Reply::NewFolder(path) => {
                self.refresh(ctx);
                self.browser.rename_new_folder(&path);
            }
            Reply::RenamedId(id, name) => {
                if let Some(l) = self.link.as_mut().filter(|l| l.id == id) {
                    l.name = name.clone();
                    if let Some(d) = app.session.active_mut() {
                        d.title = stem(&name);
                    }
                }
                self.refresh(ctx);
            }
            Reply::Download(name, bytes) => {
                if let Err(e) = crate::web::download_file(&name, &bytes) {
                    self.browser.error = Some(e);
                }
            }
            Reply::History(list) => self.history.loaded(list),
            Reply::HistoryReload => self.history_ask(app, ctx, crate::versions::Ask::Load),
            Reply::HistoryError(e) => self.history.error = Some(e),
            Reply::VersionBytes(id, bytes) => {
                if self.history.open && self.history.selected == id {
                    let name = self.link.as_ref().map(|l| l.name.clone()).unwrap_or_else(|| "version.xlsx".into());
                    match bytes.and_then(|b| version_preview(app, &name, &b)) {
                        Ok(p) => self.history.preview = Some(p),
                        Err(e) => self.history.error = Some(format!("This version can't be shown: {e}")),
                    }
                }
            }
            Reply::Opened { id, name, version, bytes, folder, modified } => {
                self.busy = false;
                let old = app.session.active().filter(|d| !d.is_dirty()).map(|d| d.uid);
                let b64 = gridcraft_engine::io::base64_encode(&bytes);
                match app.run("file.open", json!({ "name": name, "base64": b64 })) {
                    Ok(_) => {
                        if let Some(i) = old.and_then(|u| app.session.document_index(u)) {
                            app.session.close_document(i);
                        }
                        let doc = app.session.active().map_or(0, |d| d.uid);
                        if let Some(d) = app.session.active_mut() {
                            // As the server does on opening (Google Sheets' Clip, kept in
                            // StarSheets' section of the file), so both have the same workbook and
                            // the server needn't send it again (live.rs Hello `have`).
                            starsheets_sync::clip_from_host(Arc::make_mut(&mut d.wb));
                            d.title = stem(&name);
                            d.path = None;
                            d.saved = d.wb.clone();
                        }
                        self.link = Some(Link { doc, id, name, version, folder, saved: modified });
                        self.restore = Some(doc);
                        app.after_engine();
                    }
                    Err(e) => self.error = Some(gridcraft_ui_egui::clean_error(&e)),
                }
            }
            Reply::Renamed(name) => {
                self.renaming = false;
                if let Some(l) = &mut self.link {
                    l.name = name.clone();
                }
                if let Some(d) = app.session.active_mut() {
                    d.title = stem(&name);
                }
            }
            Reply::OpenTab(id) => {
                if !open_tab(&id) {
                    app.message = Some((
                        "Opened in File › Open".into(),
                        "The browser blocked the new tab. The imported file is saved: open it from File › Open, or allow pop-ups for this site."
                            .into(),
                    ));
                }
            }
            Reply::Navigate(id) => {
                set_unsaved_flag(false);
                go_to(&id);
            }
        }
    }

    /// Imported or dropped files: when the open spreadsheet already has something in it, each one
    /// is converted to .xlsx, saved to the server and opened in a new tab (as Sheets does)
    /// instead of in this tab. Call before GridCraft takes its inbox.
    pub fn take_imports(&mut self, app: &mut SheetApp, inbox: &gridcraft_ui_egui::Inbox, ctx: &egui::Context) {
        if !app.session.active().is_some_and(|d| has_data(&d.wb)) {
            return;
        }
        let arrived: Vec<(String, Vec<u8>)> = std::mem::take(&mut *inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
        for (name, bytes) in arrived {
            match gridcraft_engine::io::open_bytes(&name, &bytes).and_then(|(wb, _)| gridcraft_engine::io::save_bytes(&wb, "import.xlsx")) {
                Ok(xlsx) => send(
                    Method::Post,
                    format!("{API}?name={}&imported=1", enc(&stem(&name))),
                    Some(xlsx),
                    None,
                    self.inbox.clone(),
                    ctx.clone(),
                    |_, _, body| {
                        let v: Json = serde_json::from_slice(&body).unwrap_or_default();
                        Reply::OpenTab(v["id"].as_str().unwrap_or("").to_string())
                    },
                ),
                // A file that needs a password (or can't be read here): GridCraft's own open asks.
                Err(_) => inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((name, bytes)),
            }
        }
    }

    /// Saves the active spreadsheet once it has data and nothing changed for a moment.
    fn autosave(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        let now = gridcraft_ui_egui::monotonic_ms();
        let Some(d) = app.session.active() else { return };
        // A file open for live editing is saved by the server.
        if crate::live::handles(d.uid) {
            return;
        }
        let key = (d.uid, Arc::as_ptr(&d.wb) as usize);
        if self.seen != Some(key) {
            self.seen = Some(key);
            self.changed_at = now;
        }
        let linked = self.link.as_ref().is_some_and(|l| l.doc == d.uid);
        let now_wanted = self.save_now && !linked;
        if linked {
            self.save_now = false;
        }
        if self.busy || self.renaming || !(now_wanted || self.needs_save(d)) || app.editor.is_some() {
            return;
        }
        // The tab is closing or being hidden (index.html sets the flag): save now, don't wait.
        let flush = take_flush_flag() || now_wanted;
        if now - self.changed_at < SAVE_AFTER && !flush {
            ctx.request_repaint_after(std::time::Duration::from_millis(400));
            return;
        }
        let sent = d.wb.clone();
        let doc = d.uid;
        let title = display_name(d);
        let bytes = match xlsx(&sent, "book.xlsx") {
            Ok(b) => b,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        self.busy = true;
        match self.link.clone().filter(|l| l.doc == doc) {
            Some(l) => {
                let inbox = self.inbox.clone();
                let (sent2, bytes2) = (sent.clone(), bytes.clone());
                send(Method::Put, format!("{API}/{}", l.id), Some(bytes), Some(l.version.clone()), inbox, ctx.clone(), move |status, _, body| {
                    if status == 409 {
                        return Reply::Conflict { doc, sent: sent2, bytes: bytes2 };
                    }
                    let v: Json = serde_json::from_slice(&body).unwrap_or_default();
                    let name = v["name"].as_str().map_or(l.name.clone(), str::to_string);
                    let folder = v["folder"].as_str().map_or(l.folder.clone(), str::to_string);
                    Reply::Saved {
                        doc,
                        sent: sent2,
                        link: Link { version: v["version"].as_str().unwrap_or("").to_string(), name, folder, saved: now_unix(), ..l },
                    }
                });
            }
            None => post_new(doc, sent, bytes, title, &self.new_folder, self.inbox.clone(), ctx.clone()),
        }
    }

    /// A title changed in the app (click the title to rename) renames the file on the server.
    fn rename_if_needed(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        if self.busy || self.renaming {
            return;
        }
        let Some(l) = self.link.clone() else { return };
        let Some(d) = app.session.active().filter(|d| d.uid == l.doc) else { return };
        let want = display_name(d);
        if want == stem(&l.name) || want.trim().is_empty() {
            return;
        }
        self.renaming = true;
        send(Method::Post, format!("{API}/{}/rename?name={}", l.id, enc(&want)), None, None, self.inbox.clone(), ctx.clone(), |_, _, body| {
            let v: Json = serde_json::from_slice(&body).unwrap_or_default();
            match v["name"].as_str() {
                Some(n) => Reply::Renamed(n.to_string()),
                None => Reply::Failed("rename failed".into()),
            }
        });
    }

    /// The file window and the version list.
    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        if let Some((Reply::Opened { name, .. }, shown)) = &mut self.opening {
            egui::Area::new(egui::Id::unique("starsheets_opening"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ui.ctx(), |ui| {
                    crate::house::card_frame().show(ui, |ui| {
                        ui.label(egui::RichText::new(format!("Opening {}…", stem(name))).size(crate::house::BODY).color(crate::theme::TEXT))
                    });
                });
            *shown = true;
            ui.ctx().request_repaint();
        }
        let keys = std::mem::take(&mut self.browser_keys);
        for action in self.browser.ui(ui.ctx(), &keys) {
            self.act(app, ui.ctx(), action);
        }
        if self.details {
            self.details_ui(app, ui.ctx());
        }
    }
}

impl Files {
    /// Whether version history is showing (it takes the place of the spreadsheet, web.rs).
    pub fn history_open(&self) -> bool {
        self.history.open
    }

    /// Version history as the test hook reports it: `{selected, versions, shown}`, or null.
    pub fn history_state(&self) -> Json {
        if !self.history.open {
            return Json::Null;
        }
        let ids: Vec<&str> = self.history.list.iter().flatten().map(|v| v.id.as_str()).collect();
        json!({ "selected": self.history.selected, "versions": ids, "shown": self.history.preview.is_some(), "error": self.history.error })
    }

    /// Version history in place of the spreadsheet: the view, then what it asked for. The shown
    /// version is a spreadsheet of its own (read-only), so live editing and saving go on with
    /// the open file underneath, untouched.
    pub fn history_ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        if let Some(p) = self.history.preview.as_mut() {
            p.logic(ui.ctx());
        }
        self.history.ui(ui);
        for ask in self.history.take_asks() {
            self.history_ask(app, ui.ctx(), ask);
        }
    }

    fn history_ask(&mut self, app: &mut SheetApp, ctx: &egui::Context, ask: crate::versions::Ask) {
        use crate::versions::{Ask, CURRENT};
        let Some(l) = self.link.clone() else { return };
        let (inbox, ctx2) = (self.inbox.clone(), ctx.clone());
        match ask {
            Ask::Load => get_then(format!("{API}/{}/history", l.id), inbox, ctx2, |r| {
                Reply::History(r.map(|b| crate::versions::parse(&serde_json::from_slice(&b).unwrap_or_default())))
            }),
            // The file as it is now is the open workbook: no need to fetch it.
            Ask::Show(id) if id == CURRENT => {
                let doc = app
                    .session
                    .active()
                    .map(|d| gridcraft_engine::DocState::from_saved((*d.wb).clone(), None, d.title.clone(), gridcraft_engine::Trust::FromFile));
                if let Some(doc) = doc {
                    self.history.preview = Some(version_app(app, doc));
                }
            }
            Ask::Show(id) => {
                let v = id.clone();
                get_then(format!("{API}/{}/versions/{}", l.id, enc(&v)), inbox, ctx2, move |r| Reply::VersionBytes(id, r));
            }
            // The server keeps the current content as a version first, so a restore loses nothing;
            // the page then opens the file again.
            Ask::Restore(v) => {
                let url = format!("{API}/{}/versions/{}/restore", l.id, enc(&v));
                wasm_bindgen_futures::spawn_local(async move {
                    let reply = match fetch(Method::Post, &url, None, None).await {
                        Ok((s, ..)) if s < 400 => Reply::Navigate(l.id),
                        Ok((s, _, b)) => Reply::HistoryError(server_error(s, &b, "restore failed")),
                        Err(e) => Reply::HistoryError(e),
                    };
                    inbox.borrow_mut().push(reply);
                    ctx2.request_repaint();
                });
            }
            Ask::Name(v, name) => {
                let url = format!("{API}/{}/versions/{}/name?name={}", l.id, enc(&v), enc(&name));
                wasm_bindgen_futures::spawn_local(async move {
                    let reply = match fetch(Method::Post, &url, None, None).await {
                        Ok((s, ..)) if s < 400 => Reply::HistoryReload,
                        Ok((s, _, b)) => Reply::HistoryError(server_error(s, &b, "naming failed")),
                        Err(e) => Reply::HistoryError(e),
                    };
                    inbox.borrow_mut().push(reply);
                    ctx2.request_repaint();
                });
            }
            // A copy of that version as a new file beside this one, opened in a new tab.
            Ask::Copy(v) => {
                let when = self.history.list.iter().flatten().find(|x| x.id == v).map(|x| crate::versions::when(x.saved)).unwrap_or_default();
                let name = format!("Copy of {} ({when})", stem(&l.name));
                let current = if v == CURRENT { app.session.active().and_then(|d| xlsx(&d.wb, "copy.xlsx").ok()) } else { None };
                let get = format!("{API}/{}/versions/{}", l.id, enc(&v));
                let post = format!("{API}?name={}&folder={}", enc(&format!("{name}.xlsx")), enc(&l.folder));
                wasm_bindgen_futures::spawn_local(async move {
                    let bytes = match current {
                        Some(b) => Ok(b),
                        None => match fetch(Method::Get, &get, None, None).await {
                            Ok((s, _, b)) if s < 400 => Ok(b),
                            Ok((s, _, b)) => Err(server_error(s, &b, "copy failed")),
                            Err(e) => Err(e),
                        },
                    };
                    let reply = match bytes {
                        Ok(b) => match fetch(Method::Post, &post, Some(b), None).await {
                            Ok((s, _, body)) if s < 400 => {
                                let v: Json = serde_json::from_slice(&body).unwrap_or_default();
                                Reply::OpenTab(v["id"].as_str().unwrap_or("").to_string())
                            }
                            Ok((s, _, b)) => Reply::HistoryError(server_error(s, &b, "copy failed")),
                            Err(e) => Reply::HistoryError(e),
                        },
                        Err(e) => Reply::HistoryError(e),
                    };
                    inbox.borrow_mut().push(reply);
                    ctx2.request_repaint();
                });
            }
            Ask::Close => {}
        }
    }

    /// Whether the workbook should be (re)saved: it has data and either changed or isn't on the
    /// server yet (an imported file opens unchanged but still needs its first upload).
    fn needs_save(&self, d: &gridcraft_engine::DocState) -> bool {
        let linked = self.link.as_ref().is_some_and(|l| l.doc == d.uid);
        has_data(&d.wb) && (d.is_dirty() || !linked)
    }
}

/// The name a workbook is saved under: an imported file's own name (renaming it in the title
/// changes that), else its title.
fn display_name(d: &gridcraft_engine::DocState) -> String {
    match d.path.as_deref() {
        Some(p) => stem(p.rsplit(['/', '\\']).next().unwrap_or(p)),
        // An imported file's title can carry its extension ("Data.csv"): the server file is .xlsx.
        None => stem(&d.title),
    }
}

impl Files {
    /// File › Details, after Google Sheets: where the file is and when it was saved, and what it
    /// holds.
    fn details_ui(&mut self, app: &SheetApp, ctx: &egui::Context) {
        let Some(d) = app.session.active() else {
            self.details = false;
            return;
        };
        let place = if self.location.is_empty() { "the server".to_string() } else { self.location.clone() };
        let linked = self.link.as_ref().filter(|l| l.doc == d.uid);
        let (location, saved) = match linked {
            Some(l) => {
                let mut parts = vec![place];
                parts.extend(l.folder.split('/').filter(|p| !p.is_empty()).map(str::to_string));
                (parts.join(" › "), format!("{} ({})", date_text(l.saved), ago(l.saved)))
            }
            None => ("Not saved yet".to_string(), "Not saved yet".to_string()),
        };
        let (mut cells, mut formulas, mut charts, mut notes) = (0usize, 0usize, 0usize, 0usize);
        for sh in &d.wb.sheets {
            for (_, c) in sh.cells.iter() {
                if c.formula.is_some() {
                    formulas += 1;
                }
                if c.formula.is_some() || !c.value.is_empty() {
                    cells += 1;
                }
            }
            charts += sh.charts.len();
            notes += sh.comments.len();
        }
        let rows = [
            ("Name", display_name(d)),
            ("Location", location),
            ("Last saved", saved),
            ("Sheets", d.wb.sheets.len().to_string()),
            ("Cells with data", cells.to_string()),
            ("Formulas", formulas.to_string()),
            ("Charts", charts.to_string()),
            ("Notes", notes.to_string()),
            ("Starbase Sheets", env!("CARGO_PKG_VERSION").to_string()),
        ];
        let mut close = false;
        let mut done = false;
        crate::house::dialog(ctx, "starsheets_details", 440.0, "Document details", Some(&mut close), |ui| {
            egui::Grid::new("starsheets_details_grid").num_columns(2).spacing(vec2(crate::house::L, crate::house::S)).show(ui, |ui| {
                for (k, v) in &rows {
                    ui.label(RichText::new(*k).size(crate::house::BODY).color(crate::theme::TEXT_DIM));
                    ui.label(RichText::new(v).size(crate::house::BODY).color(crate::theme::TEXT));
                    ui.end_row();
                }
            });
            if crate::house::actions(ui, &["Close"]).is_some() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = true;
            }
        });
        if close || done {
            self.details = false;
        }
    }

    /// "Saved to the NAS › Work › Budget.xlsx · 2 minutes ago (14:05)".
    fn status_tip(&self, app: &SheetApp, unsaved: bool) -> String {
        let place = if self.location.is_empty() { "the server" } else { &self.location };
        let linked = self.link.as_ref().filter(|l| app.session.active().is_some_and(|d| d.uid == l.doc));
        let path = |l: &Link| {
            let mut parts = vec![place.to_string()];
            parts.extend(l.folder.split('/').filter(|p| !p.is_empty()).map(str::to_string));
            parts.push(l.name.clone());
            parts.join(" › ")
        };
        if let Some(e) = &self.error {
            return format!("Not saved: {e}\nIt will try again with your next change.");
        }
        match (linked, unsaved) {
            (Some(l), true) => format!("Saving your latest changes to {}…\nLast saved {}.", path(l), ago(l.saved)),
            (Some(l), false) => format!("All changes saved to {}\nLast saved {}.\nClick for version history.", path(l), ago(l.saved)),
            (None, true) => format!("Saving to {place}…"),
            (None, false) => "Not saved yet: a new spreadsheet saves itself as soon as you type in it.".to_string(),
        }
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        if !self.browser.open {
            return;
        }
        list_files(self.inbox.clone(), ctx.clone());
        if self.browser.in_bin {
            list_bin(self.inbox.clone(), ctx.clone());
        }
    }

    /// Carries out what the file window asked for, on the server; each answer lists again.
    fn act(&mut self, app: &mut SheetApp, ctx: &egui::Context, action: crate::filebrowser::Action) {
        use crate::filebrowser::{Action, Item};
        self.browser.error = None;
        let inbox = self.inbox.clone();
        let refresh = |_: u16, _: web_sys::Headers, _: Vec<u8>| Reply::Refresh;
        match action {
            Action::Open(id, _) if self.browser.name_of(&id).is_some_and(|n| is_doc(&n)) => {
                // A document opens in StarDocs (a presentation in Starbase Slides), in a new tab
                // (this tab keeps its spreadsheet).
                open_url(&file_url(&id, &self.browser.name_of(&id).unwrap_or_default()));
            }
            Action::Open(id, new_tab) => {
                // This tab only when the sheet here is completely blank; else a new tab.
                let keep = app.session.active().is_some_and(|d| has_data(&d.wb)) || self.busy;
                if new_tab || keep {
                    open_tab(&id);
                } else {
                    self.browser.open = false;
                    go_to(&id);
                }
            }
            Action::NewSheet(folder) => {
                open_url(&format!("{}?folder={}", page_url(""), enc(&folder)));
            }
            Action::NewFolder(parent) => {
                send(Method::Post, format!("api/folders?path={}&name=New%20folder", enc(&parent)), None, None, inbox, ctx.clone(), |_, _, body| {
                    let v: Json = serde_json::from_slice(&body).unwrap_or_default();
                    Reply::NewFolder(v["path"].as_str().unwrap_or("").to_string())
                });
            }
            Action::Rename(Item::File(id), name) => {
                let id2 = id.clone();
                send(Method::Post, format!("{API}/{id}/rename?name={}", enc(&name)), None, None, inbox, ctx.clone(), move |_, _, body| {
                    let v: Json = serde_json::from_slice(&body).unwrap_or_default();
                    Reply::RenamedId(id2, v["name"].as_str().unwrap_or("").to_string())
                });
            }
            Action::Rename(Item::Folder(path), name) => {
                send(Method::Post, format!("api/folders/rename?path={}&name={}", enc(&path), enc(&name)), None, None, inbox, ctx.clone(), refresh);
            }
            Action::Paste(items, folder, cut) => {
                for item in items {
                    let url = match (&item, cut) {
                        (Item::File(id), true) => format!("{API}/{id}/move?folder={}", enc(&folder)),
                        (Item::File(id), false) => format!("{API}/{id}/copy?folder={}", enc(&folder)),
                        (Item::Folder(p), true) => format!("api/folders/move?path={}&to={}", enc(p), enc(&folder)),
                        (Item::Folder(p), false) => format!("api/folders/copy?path={}&to={}", enc(p), enc(&folder)),
                    };
                    send(Method::Post, url, None, None, inbox.clone(), ctx.clone(), refresh);
                }
            }
            Action::Duplicate(id) => send(Method::Post, format!("{API}/{id}/copy"), None, None, inbox, ctx.clone(), refresh),
            Action::Delete(items) => {
                let open_one = self.link.as_ref().map(|l| l.id.clone());
                for item in items {
                    match item {
                        // The open spreadsheet going to the bin: this tab starts a new one.
                        Item::File(id) if Some(&id) == open_one.as_ref() => {
                            send(Method::Delete, format!("{API}/{id}"), None, None, inbox.clone(), ctx.clone(), |_, _, _| {
                                Reply::Navigate(String::new())
                            });
                        }
                        Item::File(id) => send(Method::Delete, format!("{API}/{id}"), None, None, inbox.clone(), ctx.clone(), refresh),
                        Item::Folder(p) => {
                            send(Method::Delete, format!("api/folders?path={}", enc(&p)), None, None, inbox.clone(), ctx.clone(), refresh)
                        }
                    }
                }
            }
            Action::Download(id, name) => {
                send(Method::Get, format!("{API}/{id}"), None, None, inbox, ctx.clone(), move |_, _, bytes| Reply::Download(name, bytes));
            }
            Action::Restore(name) => send(Method::Post, format!("api/bin/restore?name={}", enc(&name)), None, None, inbox, ctx.clone(), refresh),
            Action::Purge(name) => {
                let url = name.map_or_else(|| "api/bin".to_string(), |n| format!("api/bin?name={}", enc(&n)));
                send(Method::Delete, url, None, None, inbox, ctx.clone(), refresh);
            }
            Action::ShowBin => {
                self.browser.loading = true;
                list_bin(inbox, ctx.clone());
            }
        }
    }
}

/// Files from this size on show "Opening …" before they are read (smaller ones open at once).
const OPENING_NOTICE_FROM: usize = 512 * 1024;

/// A name without its spreadsheet extension (only known ones: "Budget v1.2" keeps its ".2").
fn stem(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((s, ext)) if ["xlsx", "xlsm", "xls", "csv", "tsv", "ods", "txt"].contains(&ext.to_ascii_lowercase().as_str()) => s.to_string(),
        _ => name.to_string(),
    }
}

pub(crate) fn enc(s: &str) -> String {
    js_sys::encode_uri_component(s).as_string().unwrap_or_default()
}

fn now_unix() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

/// "just now", "5 minutes ago", "3 hours ago", then the date, with the time in brackets.
fn ago(unix: i64) -> String {
    if unix <= 0 {
        return "at an unknown time".into();
    }
    let s = (now_unix() - unix).max(0);
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(unix as f64 * 1000.0));
    let time = format!("{:02}:{:02}", d.get_hours(), d.get_minutes());
    let when = match s {
        0..=44 => "just now".to_string(),
        45..=89 => "a minute ago".to_string(),
        90..=3599 => format!("{} minutes ago", (s + 30) / 60),
        3600..=5399 => "an hour ago".to_string(),
        5400..=86_399 => format!("{} hours ago", (s + 1800) / 3600),
        _ => return format!("on {}", date_text(unix)),
    };
    format!("{when} ({time})")
}

pub(crate) fn date_text(unix: i64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(unix as f64 * 1000.0));
    const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{} {} {}, {:02}:{:02}", d.get_date(), M[d.get_month() as usize % 12], d.get_full_year(), d.get_hours(), d.get_minutes())
}

fn query_file() -> Option<String> {
    query_param("file")
}

fn query_param(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search).ok()?.get(name).filter(|s| !s.is_empty())
}

fn open_url(url: &str) -> bool {
    web_sys::window().and_then(|w| w.open_with_url_and_target(url, "_blank").ok().flatten()).is_some()
}

/// Puts `?file=<id>` in the address bar without reloading.
fn set_query_file(id: &str) {
    let Some(w) = web_sys::window() else { return };
    if query_file().as_deref() == Some(id) {
        return;
    }
    if let Ok(h) = w.history() {
        let _ = h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&format!("?file={id}")));
    }
}

fn page_url(id: &str) -> String {
    let path = web_sys::window().and_then(|w| w.location().pathname().ok()).unwrap_or_else(|| "/".into());
    if id.is_empty() { path } else { format!("{path}?file={id}") }
}

fn go_to(id: &str) {
    if let Some(w) = web_sys::window() {
        let _ = w.location().set_href(&page_url(id));
    }
}

/// Whether a file name is a StarDocs document (.docx, .odt): it opens in StarDocs, not here.
pub use crate::store::is_doc;

/// StarDocs' page, which sits in `docs/` beside this one: `""` for a new document, else the
/// file with that id. Extra query text (`&folder=…`) follows as given.
pub fn docs_url(id: &str, extra: &str) -> String {
    let path = web_sys::window().and_then(|w| w.location().pathname().ok()).unwrap_or_else(|| "/".into());
    let dir = path.rsplit_once('/').map_or("/".to_string(), |(d, _)| format!("{d}/"));
    let q = if id.is_empty() { "new=1".to_string() } else { format!("file={id}") };
    format!("{dir}docs/?{q}{extra}")
}

/// Starbase Slides' page, in `slides/` beside this one, as [`docs_url`].
pub fn slides_url(id: &str, extra: &str) -> String {
    docs_url(id, extra).replacen("docs/?", "slides/?", 1)
}

/// The page a file opens on: Starbase Slides for a presentation, StarDocs for a document, this
/// page for a spreadsheet.
pub fn file_url(id: &str, name: &str) -> String {
    if crate::store::is_slides(name) {
        slides_url(id, "")
    } else if is_doc(name) {
        docs_url(id, "")
    } else {
        page_url(id)
    }
}

/// Opens a new tab; false when the browser blocked it.
fn open_tab(id: &str) -> bool {
    web_sys::window().and_then(|w| w.open_with_url_and_target(&page_url(id), "_blank").ok().flatten()).is_some()
}

/// Whether the page asked for an immediate save (closing or hiding the tab); clears the request.
fn take_flush_flag() -> bool {
    let Some(w) = web_sys::window() else { return false };
    let on = js_sys::Reflect::get(&w, &"__ssFlushNow".into()).is_ok_and(|v| v.is_truthy());
    if on {
        let _ = js_sys::Reflect::set(&w, &"__ssFlushNow".into(), &false.into());
    }
    on
}

/// Tells the page (index.html) whether leaving now would lose changes.
/// After signing in or out the page reloads on purpose: nothing to warn about.
pub(crate) fn clear_unsaved_flag() {
    set_unsaved_flag(false);
}

fn set_unsaved_flag(on: bool) {
    if let Some(w) = web_sys::window() {
        let _ = js_sys::Reflect::set(&w, &"__ssUnsaved".into(), &on.into());
    }
}

pub(crate) use crate::store::Method;

/// Sends a request and turns the answer into a Reply (network and server errors become Failed).
fn send(
    method: Method,
    url: String,
    body: Option<Vec<u8>>,
    if_match: Option<String>,
    inbox: Rc<RefCell<Vec<Reply>>>,
    ctx: egui::Context,
    then: impl FnOnce(u16, web_sys::Headers, Vec<u8>) -> Reply + 'static,
) {
    wasm_bindgen_futures::spawn_local(async move {
        let reply = match fetch(method, &url, body, if_match).await {
            Ok((status, headers, bytes)) if status < 400 || status == 409 => then(status, headers, bytes),
            Ok((status, _, bytes)) => {
                let v: Json = serde_json::from_slice(&bytes).unwrap_or_default();
                Reply::Failed(v["error"].as_str().map_or(format!("server said {status}"), str::to_string))
            }
            Err(e) => Reply::Failed(e),
        };
        inbox.borrow_mut().push(reply);
        ctx.request_repaint();
    });
}

/// A plain GET (used for the web fonts).
pub(crate) async fn get_bytes(url: &str) -> Result<(u16, web_sys::Headers, Vec<u8>), String> {
    fetch(Method::Get, url, None, None).await
}

thread_local! {
    /// Requests that change something (saves, uploads, renames…) still on their way: leaving the
    /// page now (to renew the sign-in) could lose them.
    static WRITES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Whether a request that changes something is still on its way.
pub(crate) fn writes_in_flight() -> bool {
    WRITES.with(|w| w.get() > 0)
}

/// Counts a writing request while it lasts.
struct Writing;

impl Writing {
    fn new() -> Writing {
        WRITES.with(|w| w.set(w.get() + 1));
        Writing
    }
}

impl Drop for Writing {
    fn drop(&mut self) {
        WRITES.with(|w| w.set(w.get().saturating_sub(1)));
    }
}

pub(crate) async fn fetch(
    method: Method,
    url: &str,
    body: Option<Vec<u8>>,
    if_match: Option<String>,
) -> Result<(u16, web_sys::Headers, Vec<u8>), String> {
    let _writing = (!matches!(method, Method::Get)).then(Writing::new);
    let (status, headers, bytes) = fetch_inner(method, url, body, if_match).await?;
    // A session made through single sign-on has ended: the server says it can be renewed at the
    // provider without asking, and account.rs does that at a quiet moment instead of showing the
    // sign-in box.
    if status == 401 && serde_json::from_slice::<Json>(&bytes).is_ok_and(|v| v["renew"].as_bool() == Some(true)) {
        crate::account::renewal_needed();
    }
    Ok((status, headers, bytes))
}

async fn fetch_inner(method: Method, url: &str, body: Option<Vec<u8>>, if_match: Option<String>) -> Result<(u16, web_sys::Headers, Vec<u8>), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let init = web_sys::RequestInit::new();
    init.set_method(match method {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Delete => "DELETE",
    });
    if let Some(b) = body {
        // A small save survives the tab closing straight after (the browser's keepalive limit is 64 KB).
        if b.len() < 60_000 {
            let _ = js_sys::Reflect::set(&init, &"keepalive".into(), &true.into());
        }
        init.set_body(&js_sys::Uint8Array::from(b.as_slice()));
    }
    let headers = web_sys::Headers::new().map_err(js)?;
    if let Some(v) = if_match {
        headers.set("If-Match", &v).map_err(js)?;
    }
    init.set_headers(&headers);
    let req = web_sys::Request::new_with_str_and_init(url, &init).map_err(js)?;
    let w = web_sys::window().ok_or("no window")?;
    let resp: web_sys::Response = wasm_bindgen_futures::JsFuture::from(w.fetch_with_request(&req))
        .await
        .map_err(|_| "the file server can't be reached".to_string())?
        .dyn_into()
        .map_err(js)?;
    let buf = wasm_bindgen_futures::JsFuture::from(resp.array_buffer().map_err(js)?).await.map_err(js)?;
    Ok((resp.status(), resp.headers(), js_sys::Uint8Array::new(&buf).to_vec()))
}

fn post_new(doc: u64, sent: Arc<Workbook>, bytes: Vec<u8>, name: String, folder: &str, inbox: Rc<RefCell<Vec<Reply>>>, ctx: egui::Context) {
    send(Method::Post, format!("{API}?name={}&folder={}", enc(&name), enc(folder)), Some(bytes), None, inbox, ctx, move |_, _, body| {
        let v: Json = serde_json::from_slice(&body).unwrap_or_default();
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        Reply::Saved { doc, sent, link: Link { doc, id: s("id"), name: s("name"), version: s("version"), folder: s("folder"), saved: now_unix() } }
    });
}

fn get_file(id: String, inbox: Rc<RefCell<Vec<Reply>>>, ctx: egui::Context) {
    send(Method::Get, format!("{API}/{id}"), None, None, inbox, ctx, move |status, headers, bytes| {
        if status != 200 {
            return Reply::Failed("file not found".into());
        }
        let name =
            headers.get("X-File-Name").ok().flatten().and_then(|n| serde_json::from_str::<String>(&n).ok()).unwrap_or_else(|| "file.xlsx".into());
        let version = headers.get("ETag").ok().flatten().unwrap_or_default();
        let folder = headers.get("X-Folder").ok().flatten().and_then(|n| serde_json::from_str::<String>(&n).ok()).unwrap_or_default();
        let modified = headers.get("X-Modified").ok().flatten().and_then(|n| n.parse().ok()).unwrap_or(0);
        Reply::Opened { id, name, version, bytes, folder, modified }
    });
}

fn list_files(inbox: Rc<RefCell<Vec<Reply>>>, ctx: egui::Context) {
    send(Method::Get, API.to_string(), None, None, inbox, ctx, |_, _, body| {
        let v: Json = serde_json::from_slice(&body).unwrap_or_default();
        let s = |x: &Json, k: &str| x[k].as_str().unwrap_or("").to_string();
        let files = v["files"]
            .as_array()
            .into_iter()
            .flatten()
            // The editor's Open dialog: spreadsheets and documents only (the server lists every
            // file since the any-file Drive; photos and the like open from the Drive).
            .filter(|f| {
                let n = f["name"].as_str().unwrap_or("").to_lowercase();
                [".xlsx", ".xlsm", ".xls", ".csv", ".ods", ".docx", ".odt", ".pptx"].iter().any(|x| n.ends_with(x))
            })
            .map(|f| crate::filebrowser::FileInfo {
                id: s(f, "id"),
                name: s(f, "name"),
                folder: s(f, "folder"),
                modified: f["modified"].as_i64().unwrap_or(0),
                size: f["size"].as_u64().unwrap_or(0),
            })
            .collect();
        let folders = v["folders"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect();
        Reply::Listed(files, folders)
    });
}

fn list_bin(inbox: Rc<RefCell<Vec<Reply>>>, ctx: egui::Context) {
    send(Method::Get, "api/bin".to_string(), None, None, inbox, ctx, |_, _, body| {
        let v: Json = serde_json::from_slice(&body).unwrap_or_default();
        let s = |x: &Json, k: &str| x[k].as_str().unwrap_or("").to_string();
        Reply::Bin(
            v.as_array()
                .into_iter()
                .flatten()
                .map(|b| crate::filebrowser::BinInfo {
                    name: s(b, "name"),
                    original: s(b, "original"),
                    deleted: b["deleted"].as_i64().unwrap_or(0),
                    folder: b["folder"].as_bool().unwrap_or(false),
                })
                .collect(),
        )
    });
}

/// A GET whose answer (the body, or why there is none) becomes a Reply. Unlike [`send`], a
/// failure doesn't become the file's save error: version history shows its own.
fn get_then(url: String, inbox: Rc<RefCell<Vec<Reply>>>, ctx: egui::Context, then: impl FnOnce(Result<Vec<u8>, String>) -> Reply + 'static) {
    wasm_bindgen_futures::spawn_local(async move {
        let got = match fetch(Method::Get, &url, None, None).await {
            Ok((s, _, b)) if s < 400 => Ok(b),
            Ok((s, _, b)) => Err(server_error(s, &b, "the server couldn't send it")),
            Err(e) => Err(e),
        };
        inbox.borrow_mut().push(then(got));
        ctx.request_repaint();
    });
}

/// The server's `error` text, or `what` and the status.
fn server_error(status: u16, body: &[u8], what: &str) -> String {
    let v: Json = serde_json::from_slice(body).unwrap_or_default();
    v["error"].as_str().map_or_else(|| format!("{what} ({status})"), str::to_string)
}

/// A spreadsheet of its own for a version from the server (.xlsx bytes), read-only.
fn version_preview(app: &SheetApp, name: &str, bytes: &[u8]) -> Result<SheetApp, String> {
    let (mut wb, _) = gridcraft_engine::io::open_bytes(name, bytes).map_err(|e| gridcraft_ui_egui::clean_error(&e.to_string()))?;
    // As a file opened for editing gets (Google's Clip, from StarSheets' part of the file).
    starsheets_sync::clip_from_host(&mut wb);
    let doc = gridcraft_engine::DocState::from_saved(wb, None, stem(name), gridcraft_engine::Trust::FromFile);
    Ok(version_app(app, doc))
}

/// A read-only spreadsheet showing `doc`, set up like the open one (its language, menus and
/// Sheets keys) but with no formula bar, as Sheets' history view has none.
fn version_app(app: &SheetApp, doc: gridcraft_engine::DocState) -> SheetApp {
    let mut session = gridcraft_engine::Session::new();
    session.add_document(doc);
    session.read_only = true;
    session.prefs.formula_bar = false;
    let mut p = SheetApp::new(session, gridcraft_ui_egui::Services::default());
    p.chrome = app.chrome;
    p.ui = app.ui.clone();
    // The fonts are already set up (by the open spreadsheet): draw from the first frame.
    p.fonts_ready = true;
    p.after_engine();
    p
}
