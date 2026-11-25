//! The desktop app connected to a Starbase server ("Connect to a server…" in the Drive's account
//! menu): the sign-in (crate starbase-connect: the browser allows the app, the token goes in the
//! system keyring), the Drive home over the server's files ([`RemoteStore`]: the same files API
//! the web app uses, with the device token), and spreadsheets opened from the server
//! ([`RemoteDoc`], `starsheets --server-file <id>`).
//!
//! Spreadsheets from the server save the plain way, not through live editing (yet): a moment
//! after each change the whole file is sent with the version it was read as (`If-Match`). If
//! someone changed it on the server since, it is never overwritten: these changes go to
//! "<name> (conflicted copy)" beside it, which the window carries on with, the same rule as for
//! files on this computer and for a web tab that wasn't live. Saving waits for the server (a
//! moment on a home network).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{RichText, Ui, vec2};
use starbase_connect::{Renewing, Server, Step};

use crate::localstore::{LocalStore, launch};
use crate::store::{Answer, Backend, Done, Method, Store, enc};
use crate::{house, theme};

/// The app's name on the server's "Your devices" list.
const APP: &str = "Starbase Sheets";

/// The Drive home's store: the connected server, else the local folder.
pub(crate) fn backend() -> Backend {
    match starbase_connect::saved() {
        Some(s) => Backend(Rc::new(RemoteStore::new(s))),
        None => Backend(Rc::new(LocalStore::new(crate::localstore::default_root()))),
    }
}

fn method_name(m: Method) -> &'static str {
    match m {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Delete => "DELETE",
    }
}

/// What a 401 means here: the device was removed on the server (and is forgotten here).
const DISCONNECTED: &str =
    "This computer is no longer connected to the server (it was removed from Your devices there). Connect to the server again to carry on.";

// ------------------------------------------------------------ the Drive over the server

pub(crate) struct RemoteStore {
    server: Renewing,
    tx: Sender<(u64, Answer)>,
    rx: Receiver<(u64, Answer)>,
    waiting: RefCell<HashMap<u64, Done>>,
    next: Cell<u64>,
    /// The home screen's own settings (grid or list, sort) stay on this computer.
    local: LocalStore,
}

impl RemoteStore {
    pub fn new(server: Server) -> RemoteStore {
        let (tx, rx) = channel();
        RemoteStore {
            server: Renewing::new(server),
            tx,
            rx,
            waiting: RefCell::default(),
            next: Cell::new(1),
            local: LocalStore::new(crate::localstore::default_root()),
        }
    }
}

impl Store for RemoteStore {
    fn has_photos(&self) -> bool {
        true
    }

    fn server(&self) -> Option<Server> {
        Some(self.server.server())
    }

    fn call(&self, ctx: &egui::Context, method: Method, url: &str, body: Option<Vec<u8>>, done: Done) {
        self.call_if(ctx, method, url, body, None, done);
    }

    fn call_if(&self, ctx: &egui::Context, method: Method, url: &str, body: Option<Vec<u8>>, if_match: Option<String>, done: Done) {
        let id = self.next.get();
        self.next.set(id + 1);
        self.waiting.borrow_mut().insert(id, done);
        let (tx, ctx, server, url) = (self.tx.clone(), ctx.clone(), self.server.clone(), url.to_string());
        std::thread::spawn(move || {
            let answer = match server.call(method_name(method), &url, body.as_deref(), if_match.as_deref()) {
                Ok(r) if r.status == 401 => Err(DISCONNECTED.to_string()),
                Ok(r) => Ok((r.status, r.body)),
                Err(e) => Err(e),
            };
            let _ = tx.send((id, answer));
            ctx.request_repaint();
        });
    }

    fn poll(&self) {
        while let Ok((id, answer)) = self.rx.try_recv() {
            let done = self.waiting.borrow_mut().remove(&id);
            if let Some(done) = done {
                done(answer);
            }
        }
    }

    fn open(&self, id: &str, name: &str) -> Result<(), String> {
        // Still connected? (Else the window would only say so, over an empty spreadsheet.)
        if self.server.call("GET", "api/devices", None, None)?.status == 401 {
            return Err(DISCONNECTED.into());
        }
        let program = crate::store::program_for(name);
        launch(program, &["--server-file".as_ref(), id.as_ref(), "--name".as_ref(), name.as_ref()])
    }

    fn has_slides(&self) -> bool {
        self.local.has_slides()
    }

    /// Starbase Slides makes the presentation on the server, then opens it.
    fn new_presentation(&self, folder: &str) -> Result<(), String> {
        if self.server.call("GET", "api/devices", None, None)?.status == 401 {
            return Err(DISCONNECTED.into());
        }
        launch("starslides", &["--server-new".as_ref(), folder.as_ref()])
    }

    fn new_file(&self, folder: &str, document: bool) -> Result<(), String> {
        // Made on the server straight away (an empty file), then opened from there.
        let (name, bytes) = if document {
            ("Untitled document.docx", crate::localstore::empty_docx())
        } else {
            ("Untitled spreadsheet.xlsx", crate::store::xlsx(&gridcraft_engine::model::Workbook::new(), "book.xlsx")?)
        };
        let r = self.server.call("POST", &format!("api/files?name={}&folder={}", enc(name), enc(folder)), Some(&bytes), None)?;
        if !r.ok() {
            return Err(r.error());
        }
        let v = r.json();
        let (id, made) = (v["id"].as_str().unwrap_or(""), v["name"].as_str().unwrap_or(name));
        self.open(id, made)
    }

    fn pick_files(&self, got: Box<dyn Fn(String, Vec<u8>)>) {
        self.local.pick_files(got);
    }

    fn download(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.local.download(name, bytes)
    }

    fn setting(&self, key: &str) -> Option<String> {
        self.local.setting(key)
    }

    fn set_setting(&self, key: &str, value: &str) {
        self.local.set_setting(key, value);
    }
}

// ------------------------------------------------------------ a spreadsheet from the server

/// A spreadsheet opened from the server: where it is there, and the version this window's
/// copy is based on.
pub(crate) struct RemoteDoc {
    server: Renewing,
    pub id: String,
    pub name: String,
    pub folder: String,
    etag: Option<String>,
}

/// Where a file from the server is put for GridCraft to open (removed once it's open).
fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")));
    base.unwrap_or_else(std::env::temp_dir).join("starbase").join("opening")
}

/// "Budget.csv" → ".csv" ("" without one).
fn ext(name: &str) -> String {
    name.rsplit_once('.').map_or(String::new(), |(_, e)| format!(".{e}"))
}

/// Where changes made over someone else's go: "Budget.csv" → "Budget (conflicted copy).csv"
/// (the same name in Starbase Sheets and Docs, on disk and on the server).
pub(crate) fn conflict_name(name: &str) -> String {
    format!("{} (conflicted copy){}", stem(name), ext(name))
}

/// "Budget.xlsx" → "Budget".
pub(crate) fn stem(name: &str) -> String {
    name.rsplit_once('.').map_or(name, |(s, _)| s).to_string()
}

impl RemoteDoc {
    /// Reads file `id` from the connected server: the file, and a copy on disk for GridCraft to
    /// open (the caller removes it).
    pub fn fetch(id: &str) -> Result<(RemoteDoc, PathBuf), String> {
        let server = starbase_connect::saved().ok_or("This computer isn't connected to a server. Connect from Starbase Drive first.")?;
        let server = Renewing::new(server);
        let r = server.call("GET", &format!("api/files/{}", enc(id)), None, None)?;
        if r.status == 401 {
            return Err(DISCONNECTED.into());
        }
        if !r.ok() {
            return Err(r.error());
        }
        let name = r.name.clone().unwrap_or_else(|| "Spreadsheet.xlsx".into());
        let dir = cache_dir().join(id.chars().filter(char::is_ascii_alphanumeric).collect::<String>());
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let safe: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '-' } else { c }).collect();
        let path = dir.join(safe.trim_start_matches('.'));
        crate::localstore::write_atomic(&path, &r.body).map_err(|e| e.to_string())?;
        Ok((RemoteDoc { server, id: id.to_string(), name, folder: r.folder.clone().unwrap_or_default(), etag: r.etag }, path))
    }

    /// The server, for the save status ("Saved to files.example.org").
    pub fn host(&self) -> String {
        let u = self.url();
        u.split_once("://").map_or(u.as_str(), |(_, r)| r).to_string()
    }

    pub fn url(&self) -> String {
        self.server.server().url
    }

    /// The workbook as the file's own format (a .csv stays a .csv, as the server itself saves
    /// it): never other bytes under its name.
    pub fn bytes(&self, wb: &gridcraft_engine::model::Workbook) -> Result<Vec<u8>, String> {
        crate::store::xlsx(wb, &self.name)
    }

    /// Saves over the version last read or saved (`bytes` from [`RemoteDoc::bytes`]). Changed on
    /// the server since: saved as a conflicted copy beside it instead, which this window carries
    /// on with (`Ok(Some(name))`).
    pub fn save(&mut self, bytes: &[u8]) -> Result<Option<String>, String> {
        let r = self.server.call("PUT", &format!("api/files/{}", enc(&self.id)), Some(bytes), self.etag.as_deref())?;
        match r.status {
            200 => {
                self.etag = r.json()["version"].as_str().map(str::to_string);
                Ok(None)
            }
            409 => {
                let want = conflict_name(&self.name);
                let r = self.server.call("POST", &format!("api/files?name={}&folder={}", enc(&want), enc(&self.folder)), Some(bytes), None)?;
                if !r.ok() {
                    return Err(r.error());
                }
                let v = r.json();
                self.id = v["id"].as_str().unwrap_or_default().to_string();
                self.name = v["name"].as_str().unwrap_or(&want).to_string();
                self.etag = v["version"].as_str().map(str::to_string);
                Ok(Some(self.name.clone()))
            }
            401 => Err(DISCONNECTED.into()),
            _ => Err(r.error()),
        }
    }

    /// A new name (`stem`; the server keeps the extension and makes it free in its folder).
    pub fn rename(&mut self, stem_wanted: &str) -> Result<(), String> {
        let r = self.server.call(
            "POST",
            &format!("api/files/{}/rename?name={}", enc(&self.id), enc(&format!("{stem_wanted}{}", ext(&self.name)))),
            Some(&[]),
            None,
        )?;
        if !r.ok() {
            return Err(r.error());
        }
        if let Some(n) = r.json()["name"].as_str() {
            self.name = n.to_string();
        }
        Ok(())
    }

    /// Moves it to the server's bin.
    pub fn bin(&self) -> Result<(), String> {
        let r = self.server.call("DELETE", &format!("api/files/{}", enc(&self.id)), None, None)?;
        if r.ok() { Ok(()) } else { Err(r.error()) }
    }

    /// A copy beside it ("Copy of …"), opened in a window of its own.
    pub fn copy(&self) -> Result<(), String> {
        let r = self.server.call("POST", &format!("api/files/{}/copy", enc(&self.id)), Some(&[]), None)?;
        if !r.ok() {
            return Err(r.error());
        }
        let v = r.json();
        launch(
            "starsheets",
            &["--server-file".as_ref(), v["id"].as_str().unwrap_or("").as_ref(), "--name".as_ref(), v["name"].as_str().unwrap_or("").as_ref()],
        )
    }
}

// ------------------------------------------------------------ the account menu and dialog

#[derive(Default)]
enum Dialog {
    #[default]
    None,
    /// The server's address being typed.
    Address,
    /// Waiting for the person to allow the app in the browser.
    Waiting {
        user_code: String,
        verify_url: String,
        opened: Result<(), String>,
    },
    Done(String),
}

/// "Connect to a server…": the dialog, and the sign-in under way.
#[derive(Default)]
pub(crate) struct Connect {
    dialog: Dialog,
    address: String,
    error: Option<String>,
    steps: Option<Receiver<Step>>,
    focused: bool,
    /// Disconnecting (on a thread: the server is told).
    leaving: Option<Receiver<Result<(), String>>>,
}

/// A round avatar with the first letter of `name`.
fn avatar(ui: &mut Ui, name: &str, colour: &str) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(32.0, 32.0), egui::Sense::click());
    let c = u32::from_str_radix(colour.trim_start_matches('#'), 16).ok().filter(|_| colour.len() == 7).unwrap_or(0x5F6368);
    let fill = egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
    ui.painter().circle_filled(rect.center(), 16.0, fill);
    let letter: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, letter, egui::FontId::proportional(15.0), egui::Color32::WHITE);
    r
}

impl Connect {
    /// The account menu at the right of the Drive's top bar: "Connect to a server…" on this
    /// computer's files; who is signed in, and Disconnect, on a server's.
    pub fn menu_ui(&mut self, ui: &mut Ui, store: &Backend) {
        match store.server() {
            None => {
                if ui.add(house::secondary("Connect to a server…")).on_hover_text("Use the files on your Starbase server").clicked() {
                    self.open();
                }
            }
            Some(s) => {
                let colour = s.user["colour"].as_str().unwrap_or("").to_string();
                let r = avatar(ui, &s.name(), &colour).on_hover_text(format!("{} ({}) on {}", s.name(), s.username(), s.url));
                let mut disconnect = false;
                egui::Popup::menu(&r).show(|ui| {
                    house::menu(ui, 280.0);
                    house::menu_inset(ui, |ui| {
                        ui.label(RichText::new(s.name()).size(house::BODY).strong().color(theme::TEXT));
                        house::note(ui, &format!("{} on {}", s.username(), s.url));
                    });
                    house::menu_sep(ui);
                    if house::menu_row(ui, None, "Open in the browser", "", true)
                        && let Err(e) = starbase_connect::open_browser(&s.url)
                    {
                        self.error = Some(e);
                    }
                    if house::menu_row(ui, None, "Disconnect", "", true) {
                        disconnect = true;
                    }
                });
                if disconnect && self.leaving.is_none() {
                    let (tx, rx) = channel();
                    std::thread::spawn(move || {
                        let _ = tx.send(s.disconnect());
                    });
                    self.leaving = Some(rx);
                }
            }
        }
    }

    fn open(&mut self) {
        self.dialog = Dialog::Address;
        self.error = None;
        self.focused = false;
        self.steps = None;
    }

    /// Each frame: the sign-in's progress and the dialog. Returns the Drive's new store when
    /// the connection changed (connected, or disconnected).
    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Backend> {
        let mut changed = None;
        // The server refused this computer's token (the device was removed there): it was
        // forgotten, so the Drive shows this computer's files again and says why.
        if starbase_connect::take_changed() && self.leaving.is_none() {
            let b = backend();
            if b.server().is_none() {
                self.error = Some(DISCONNECTED.into());
                self.dialog = Dialog::Done(String::new());
            }
            changed = Some(b);
        }
        if let Some(rx) = &self.leaving
            && let Ok(r) = rx.try_recv()
        {
            self.leaving = None;
            if let Err(e) = r {
                self.error = Some(e);
                self.dialog = Dialog::Done(String::new());
            }
            changed = Some(backend());
        }
        while let Some(step) = self.steps.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match step {
                Step::Code { user_code, verify_url, opened } => self.dialog = Dialog::Waiting { user_code, verify_url, opened },
                Step::Waiting => {}
                Step::Done(server) => {
                    self.steps = None;
                    self.dialog = Dialog::Done(format!("Connected as {} ({}) on {}.", server.name(), server.username(), server.url));
                    changed = Some(Backend(Rc::new(RemoteStore::new(server))));
                }
                Step::Failed(why) => {
                    self.steps = None;
                    self.error = Some(why);
                    self.dialog = Dialog::Address;
                }
            }
        }
        if self.steps.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
        self.dialog_ui(ctx);
        changed
    }

    fn dialog_ui(&mut self, ctx: &egui::Context) {
        let mut close = false;
        let mut cancel = false;
        let mut go = false;
        match &self.dialog {
            Dialog::None => return,
            Dialog::Address => {
                house::dialog(ctx, "starbase_connect", 420.0, "Connect to a server", Some(&mut close), |ui| {
                    house::note(
                        ui,
                        "Use the files on your Starbase server. Your browser opens on the server, where you allow this app; the app never sees your password.",
                    );
                    house::gap(ui);
                    house::label(ui, "Server address", "As you open it in your browser.");
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.address)
                            .hint_text("https://files.example.org")
                            .desired_width(f32::INFINITY)
                            .margin(vec2(house::M, house::S + 1.0)),
                    );
                    if !self.focused {
                        r.request_focus();
                        self.focused = true;
                    }
                    go |= (r.lost_focus() || r.has_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if self.address.trim().to_lowercase().starts_with("http://") {
                        house::note(ui, "http:// isn't encrypted: it works only for a server on your own network, never one on the internet.");
                    }
                    if let Some(e) = &self.error {
                        ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
                    }
                    match house::actions(ui, &["Cancel", "Connect"]) {
                        Some(0) => cancel = true,
                        Some(_) => go = true,
                        None => {}
                    }
                });
            }
            Dialog::Waiting { user_code, verify_url, opened } => {
                let (code, url, opened) = (user_code.clone(), verify_url.clone(), opened.clone());
                house::dialog(ctx, "starbase_connect", 420.0, "Connect to a server", Some(&mut close), |ui| {
                    house::note(
                        ui,
                        if opened.is_ok() {
                            "Your browser opened the server's connect page. Sign in there if it asks, type this code, and allow Starbase Sheets."
                        } else {
                            "Open this address in your browser, sign in if it asks, type this code, and allow Starbase Sheets:"
                        },
                    );
                    if opened.is_err() {
                        ui.add(egui::Label::new(RichText::new(&url).size(house::NOTE).color(theme::TEXT)).selectable(true));
                    }
                    house::gap(ui);
                    ui.vertical_centered(|ui| {
                        ui.add(egui::Label::new(RichText::new(&code).monospace().size(28.0).color(theme::TEXT)).selectable(true));
                    });
                    house::gap(ui);
                    house::note(ui, "Waiting for you to allow it…");
                    if house::actions(ui, &["Cancel"]).is_some() {
                        cancel = true;
                    }
                });
            }
            Dialog::Done(text) => {
                let text = text.clone();
                house::dialog(ctx, "starbase_connect", 420.0, "Connect to a server", Some(&mut close), |ui| {
                    if !text.is_empty() {
                        house::note(ui, &text);
                    }
                    if let Some(e) = &self.error {
                        ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
                    }
                    if house::actions(ui, &["Done"]).is_some() {
                        cancel = true;
                    }
                });
            }
        }
        if close || cancel {
            // Dropping the receiver stops the sign-in.
            self.steps = None;
            self.dialog = Dialog::None;
        } else if go && self.steps.is_none() {
            match starbase_connect::clean_url(&self.address) {
                Ok(url) => {
                    self.error = None;
                    self.steps = Some(starbase_connect::connect(&url, APP));
                }
                Err(e) => self.error = Some(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflicted_copies_keep_the_files_own_type() {
        assert_eq!(conflict_name("Budget.csv"), "Budget (conflicted copy).csv");
        assert_eq!(conflict_name("Plan.xlsx"), "Plan (conflicted copy).xlsx");
        // A csv is saved as csv, never as a zip (an .xlsx) under its name.
        let wb = gridcraft_engine::model::Workbook::new();
        let csv = crate::store::xlsx(&wb, "Budget.csv").unwrap();
        assert!(!csv.starts_with(b"PK"), "{csv:?}");
        assert!(crate::store::xlsx(&wb, "Plan.xlsx").unwrap().starts_with(b"PK"));
    }

    #[test]
    fn a_csv_is_written_with_crlf_and_read_back_from_either_line_ending() {
        // CRLF between rows (RFC 4180, as Google Sheets and Excel export CSV); a file with
        // plain LF line endings still opens the same.
        let mut s = gridcraft_engine::Session::new();
        s.new_workbook();
        s.execute("cell.set", serde_json::json!({"cell": "A1", "input": "Item"})).unwrap();
        s.execute("cell.set", serde_json::json!({"cell": "B1", "input": "Cost"})).unwrap();
        s.execute("cell.set", serde_json::json!({"cell": "A2", "input": "Rent"})).unwrap();
        s.execute("cell.set", serde_json::json!({"cell": "B2", "input": "850"})).unwrap();
        let csv = crate::store::xlsx(&s.active().unwrap().wb, "Budget.csv").unwrap();
        assert_eq!(csv, b"Item,Cost\r\nRent,850\r\n");
        let read = |bytes: &[u8]| {
            let (wb, _) = gridcraft_engine::io::open_bytes("Budget.csv", bytes).unwrap();
            crate::store::xlsx(&wb, "Budget.csv").unwrap()
        };
        assert_eq!(read(&csv), csv, "CRLF round-trips unchanged");
        assert_eq!(read(b"Item,Cost\nRent,850\n"), csv, "LF reads the same");
    }

    #[test]
    fn names_without_their_extension() {
        assert_eq!(stem("Budget.xlsx"), "Budget");
        assert_eq!(stem("Budget (conflicted copy).xlsx"), "Budget (conflicted copy)");
        assert_eq!(stem("plain"), "plain");
    }

    #[test]
    fn methods_are_named_for_http() {
        assert_eq!((method_name(Method::Get), method_name(Method::Post), method_name(Method::Delete)), ("GET", "POST", "DELETE"));
    }
}
