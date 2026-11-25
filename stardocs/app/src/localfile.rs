//! Local mode for the desktop program: the document is a file on this computer, kept the way the
//! browser's server link keeps a document on the Starbase server (server.rs), so the window
//! (shell.rs) talks to either through the same `Link` methods.
//!
//! - Open: `stardocs FILE`, File › Open (the system's file dialog), or the recent list.
//! - Save: by itself a couple of seconds after the last change, back to the same file, written
//!   to a temporary file beside it and renamed over it, so a crash never leaves half a file.
//! - Changed elsewhere (another program saved the file since we read or wrote it, spotted by its
//!   modification time and size): the file is never overwritten. This version is kept as a new
//!   file beside it, "<name> (Starbase Docs copy <date>)", and the window carries on with that
//!   copy, saying so: the same rule as the server's 409.
//! - A new document is made in `~/Documents/Starbase` on its first save, named after its first
//!   words as WordCraft suggests.
//! - Recent files are listed in the data folder (`$XDG_DATA_HOME/starbase`), never as hidden
//!   files beside the documents.
//!
//! Where the bytes go is a [`Store`]: [`LocalFile`] today. A Starbase server reached from the
//! desktop (plan job D2b) is another `Store` behind the same `Link`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::SystemTime;

use wordcraft_ui_egui::{Services, WordApp};

/// How long the document must sit unchanged before it saves by itself (milliseconds).
const QUIET_MS: f64 = 2000.0;
/// How long after a failed save it is tried again (milliseconds).
const RETRY_MS: f64 = 15_000.0;
/// How long a message stays in the title row (milliseconds).
const NOTICE_MS: f64 = 20_000.0;
/// Most files kept in the recent list.
const MAX_RECENT: usize = 10;
/// The file types Open offers.
pub const DOC_EXTS: &[&str] = &["docx", "docm", "dotx", "dotm", "doc", "dot", "odt", "ott", "fodt", "rtf", "txt", "md", "html", "htm"];

/// What the title row says about saving (as `server::Saved`).
#[derive(Clone, Default, PartialEq, Debug)]
pub enum Saved {
    #[default]
    Nothing,
    #[allow(dead_code, reason = "local saves are made at once; the title row shows it for the server")]
    Saving,
    Done,
    Failed(String),
}

/// Why a write didn't happen.
#[derive(Debug, PartialEq)]
pub enum WriteError {
    /// Someone else changed the file since we last read or wrote it: never overwrite it.
    ChangedElsewhere,
    Other(String),
}

/// Where a document's bytes live: a file on this computer ([`LocalFile`]), later a Starbase
/// server reached from the desktop. The `Link` does the rest (when to save, the copy rule).
pub trait Store {
    /// The file's name ("Report.docx"); empty before a new document's first save.
    fn name(&self) -> String;
    /// Where it is, for the recent list and the window title; `None` before the first save.
    fn location(&self) -> Option<PathBuf>;
    /// Reads the document and remembers its version.
    fn read(&mut self) -> Result<Vec<u8>, String>;
    /// Writes the document over the version last read or written. A new document is created
    /// under `name` (made unique) on its first write.
    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), WriteError>;
    /// Writes `bytes` as a new file called `name` (made unique) beside this one, and carries on
    /// with that file.
    fn keep_copy(&mut self, name: &str, bytes: &[u8]) -> Result<(), String>;
    /// Gives the file a new name (`stem` plus its own extension).
    fn rename(&mut self, stem: &str) -> Result<(), String>;
}

/// A file's version on disk: its modification time and size.
type Version = (Option<SystemTime>, u64);

fn version_of(path: &Path) -> Option<Version> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok(), m.len()))
}

/// A document file on this computer.
pub struct LocalFile {
    /// The folder a new document is made in.
    dir: PathBuf,
    /// The file; `None` until a new document's first save.
    path: Option<PathBuf>,
    /// The file's version when we last read or wrote it.
    seen: Option<Version>,
}

impl LocalFile {
    /// An existing file.
    pub fn open(path: PathBuf) -> Self {
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Self { dir, path: Some(path), seen: None }
    }

    /// A new document, made in `dir` on its first save.
    pub fn new_in(dir: PathBuf) -> Self {
        Self { dir, path: None, seen: None }
    }
}

impl Store for LocalFile {
    fn name(&self) -> String {
        self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn location(&self) -> Option<PathBuf> {
        self.path.clone()
    }

    fn read(&mut self) -> Result<Vec<u8>, String> {
        let path = self.path.as_ref().ok_or("no file yet")?;
        let before = version_of(path);
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.seen = before;
        Ok(bytes)
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<(), WriteError> {
        let path = match &self.path {
            Some(p) => {
                // A file that is gone (moved or deleted elsewhere) is written again where it was.
                if let Some(now) = version_of(p)
                    && self.seen.is_some_and(|seen| seen != now)
                {
                    return Err(WriteError::ChangedElsewhere);
                }
                p.clone()
            }
            None => {
                std::fs::create_dir_all(&self.dir).map_err(|e| WriteError::Other(format!("{}: {e}", self.dir.display())))?;
                unique(&self.dir, name)
            }
        };
        write_atomic(&path, bytes).map_err(|e| WriteError::Other(format!("{}: {e}", path.display())))?;
        self.seen = version_of(&path);
        self.path = Some(path);
        Ok(())
    }

    fn keep_copy(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let path = unique(&self.dir, name);
        write_atomic(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        self.seen = version_of(&path);
        self.path = Some(path);
        Ok(())
    }

    fn rename(&mut self, stem: &str) -> Result<(), String> {
        let stem = stem.trim();
        if stem.is_empty() || stem.contains(['/', '\\']) || stem.starts_with('.') {
            return Err(format!("\"{stem}\" can't be a file name"));
        }
        let Some(old) = self.path.clone() else { return Err("the document isn't saved yet".into()) };
        let ext = old.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let new = old.with_file_name(format!("{stem}{ext}"));
        if new.exists() {
            return Err(format!("there is already a file called \"{stem}{ext}\""));
        }
        std::fs::rename(&old, &new).map_err(|e| e.to_string())?;
        self.path = Some(new);
        Ok(())
    }
}

/// `dir/name`, or `dir/name (2)`, `(3)`… when a file of that name is already there.
fn unique(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = split(name);
    let mut path = dir.join(format!("{stem}{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    path
}

/// Writes `bytes` to a temporary file beside `path` and renames it over `path`, so a failed or
/// interrupted write leaves the previous file whole.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "document".into());
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let written = std::fs::File::create(&tmp).and_then(|mut f| {
        f.write_all(bytes)?;
        f.sync_all()
    });
    let done = written.and_then(|()| std::fs::rename(&tmp, path));
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    done
}

fn split(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), ".docx".to_string()),
    }
}

/// Whether a saved file is the document itself (a Word or OpenDocument package) rather than an
/// export.
fn is_document(name: &str) -> bool {
    let lower = name.to_lowercase();
    [".docx", ".docm", ".dotx", ".dotm", ".odt"].iter().any(|e| lower.ends_with(e))
}

/// "2026-11-24 14.05", local time, for copy names.
fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H.%M").to_string()
}

/// The folders local mode uses.
#[derive(Clone, Debug)]
pub struct Folders {
    /// Where new documents go: `~/Documents/Starbase`.
    pub documents: PathBuf,
    /// Our own data (the recent list): `$XDG_DATA_HOME/starbase`.
    pub data: PathBuf,
}

impl Folders {
    /// The user's folders (XDG on Linux), or `None` without a home folder.
    pub fn of_user() -> Option<Self> {
        let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)?;
        let data = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()).map_or_else(|| home.join(".local/share"), PathBuf::from);
        let config = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()).map_or_else(|| home.join(".config"), PathBuf::from);
        let docs = documents_dir(&home, &config).unwrap_or_else(|| home.join("Documents"));
        Some(Self { documents: docs.join("Starbase"), data: data.join("starbase") })
    }
}

/// The Documents folder from `~/.config/user-dirs.dirs` (`XDG_DOCUMENTS_DIR="$HOME/Documents"`).
fn documents_dir(home: &Path, config: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(config.join("user-dirs.dirs")).ok()?;
    let line = text.lines().find_map(|l| l.trim().strip_prefix("XDG_DOCUMENTS_DIR="))?;
    let value = line.trim().trim_matches('"');
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(value),
    };
    // `$HOME/` alone means "no Documents folder": use the default.
    (path != home && path.is_absolute()).then_some(path)
}

/// The recent files, newest first, kept in `<data>/docs-recent.json`.
pub fn recent(folders: &Folders) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(folders.data.join("docs-recent.json")) else { return Vec::new() };
    serde_json::from_str::<Vec<PathBuf>>(&text).unwrap_or_default().into_iter().take(MAX_RECENT).collect()
}

fn remember(folders: &Folders, path: &Path) {
    let mut list = recent(folders);
    list.retain(|p| p != path);
    list.insert(0, path.to_path_buf());
    list.truncate(MAX_RECENT);
    let written = std::fs::create_dir_all(&folders.data)
        .and_then(|()| serde_json::to_vec_pretty(&list).map_err(std::io::Error::other))
        .and_then(|json| write_atomic(&folders.data.join("docs-recent.json"), &json));
    if let Err(e) = written {
        log::warn!("recent files: {e}");
    }
}

/// What the window asks the program to do outside this window: open another file (in a new
/// window when this one already holds a document), or start a new document.
#[derive(Debug, PartialEq)]
pub enum Request {
    Open(PathBuf),
    New,
}

struct State {
    store: Box<dyn Store>,
    folders: Folders,
    saved: Saved,
    news: Vec<String>,
    notice: Option<(String, f64)>,
    dirty_since: Option<f64>,
    retry: Option<(String, Vec<u8>, f64)>,
    download_next: bool,
    /// An Open dialog on its way.
    picking: Option<Receiver<Option<PathBuf>>>,
    /// What the program should do next (see [`Request`]).
    requests: Vec<Request>,
}

/// The document's link to where it is kept (local mode: a file on this computer).
#[derive(Clone)]
pub struct Link {
    state: Rc<RefCell<State>>,
    /// Milliseconds, for the quiet moment before saving (a fake clock in tests).
    clock: Rc<dyn Fn() -> f64>,
}

fn wall_clock() -> f64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

impl Link {
    /// What the title row says once saved.
    pub const SAVED: &str = "Saved to this computer";
    /// The logo's tooltip: it opens a file.
    pub const HOME_TIP: &str = "Open a document";

    pub fn new(store: Box<dyn Store>, folders: Folders) -> Self {
        let state = State {
            store,
            folders,
            saved: Saved::Nothing,
            news: Vec::new(),
            notice: None,
            dirty_since: None,
            retry: None,
            download_next: false,
            picking: None,
            requests: Vec::new(),
        };
        Self { state: Rc::new(RefCell::new(state)), clock: Rc::new(wall_clock) }
    }

    /// The same link with another clock (tests).
    #[cfg(test)]
    pub fn with_clock(mut self, clock: impl Fn() -> f64 + 'static) -> Self {
        self.clock = Rc::new(clock);
        self
    }

    fn now(&self) -> f64 {
        (self.clock)()
    }

    /// Saves go to the store; exports and File › Download ask where with the system's dialog.
    pub fn connect(&self, services: &mut Services) {
        services.async_saves = None;
        // WordCraft takes its host save path only while `download` is set: then `host_save` gets
        // every file, and exports come back here.
        services.download = Some(Box::new(save_elsewhere));
        let link = self.clone();
        services.host_save = Some(Box::new(move |name: &str, bytes: &[u8], _save_as: bool| {
            if !is_document(name) {
                return save_elsewhere(name, bytes);
            }
            if std::mem::take(&mut link.state.borrow_mut().download_next) {
                let shown = link.name();
                save_elsewhere(if shown.is_empty() { name } else { &shown }, bytes)?;
            }
            link.save(name, bytes.to_vec());
            Ok(())
        }));
    }

    /// Reads the file into the app. `false` (and the reason in the title row) when it can't.
    pub fn open(&self, app: &mut WordApp) -> bool {
        let read = self.state.borrow_mut().store.read();
        let name = self.name();
        match read {
            Ok(bytes) => {
                let data = wordcraft_engine::cmd::insert::base64_encode(&bytes);
                match app.run("file.open", serde_json::json!({"path": name, "data": data})) {
                    Ok(_) => {
                        app.session.dirty = false;
                        self.remember();
                        true
                    }
                    Err(e) => {
                        self.fail(&format!("Couldn't open \"{name}\": {e}"));
                        false
                    }
                }
            }
            Err(e) => {
                self.fail(&format!("Couldn't open the document: {e}"));
                false
            }
        }
    }

    fn remember(&self) {
        let s = self.state.borrow();
        if let Some(p) = s.store.location() {
            remember(&s.folders, &p);
        }
    }

    /// Never view-only in local mode (a read-only file fails to save, and says so).
    pub fn view_only(&self) -> bool {
        false
    }

    /// Local documents have no one to share with yet: that comes with a server.
    pub fn can_share(&self) -> bool {
        false
    }

    /// Version history is kept by a Starbase server; a local file has none (the panel says so,
    /// and the calls below are never made).
    pub fn has_history(&self) -> bool {
        false
    }
    pub fn load_history(&self) {}
    pub fn take_history(&self) -> Option<Result<Vec<crate::versions::Version>, String>> {
        None
    }
    pub fn previewing(&self) -> bool {
        false
    }
    pub fn show_version(&self, _v: &str) {}
    pub fn end_preview(&self) {}
    pub fn restore_version(&self, _v: &str, _label: &str) {}
    pub fn copy_version(&self, _v: &str) {}
    pub fn name_version(&self, _v: &str, _name: &str) {}

    /// File › Open and the logo: the system's Open dialog, without freezing the window.
    pub fn home(&self) {
        if self.state.borrow().picking.is_some() {
            return;
        }
        let start = self.state.borrow().folders.documents.clone();
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("open dialog".into()).spawn(move || {
            let _ = std::fs::create_dir_all(&start);
            let dialog = rfd::AsyncFileDialog::new().add_filter("Documents", DOC_EXTS).set_directory(&start);
            let picked = pollster::block_on(dialog.pick_file()).map(|f| f.path().to_path_buf());
            let _ = tx.send(picked);
        });
        match spawned {
            Ok(_) => self.state.borrow_mut().picking = Some(rx),
            Err(e) => self.news(format!("Couldn't show the Open dialog: {e}")),
        }
    }

    /// File › New document: a new window.
    pub fn new_document(&self) {
        self.state.borrow_mut().requests.push(Request::New);
    }

    /// Opens `path` (File › Open recent, a dropped file).
    pub fn open_path(&self, path: PathBuf) {
        self.state.borrow_mut().requests.push(Request::Open(path));
    }

    /// What the program should do next, taken.
    pub fn take_requests(&self) -> Vec<Request> {
        std::mem::take(&mut self.state.borrow_mut().requests)
    }

    /// The recent files (File › Open recent), newest first, without this one.
    pub fn recent(&self) -> Vec<PathBuf> {
        let s = self.state.borrow();
        let here = s.store.location();
        recent(&s.folders).into_iter().filter(|p| Some(p) != here.as_ref()).collect()
    }

    /// The file's path as text once it exists.
    pub fn id(&self) -> Option<String> {
        self.state.borrow().store.location().map(|p| p.to_string_lossy().into_owned())
    }

    /// Where the file is, once it exists.
    pub fn location(&self) -> Option<PathBuf> {
        self.state.borrow().store.location()
    }

    pub fn name(&self) -> String {
        self.state.borrow().store.name()
    }

    pub fn notice(&self) -> Option<String> {
        let s = self.state.borrow();
        let t = self.now();
        s.notice.as_ref().filter(|(_, at)| t - at < NOTICE_MS).map(|(m, _)| m.clone())
    }

    pub fn saved(&self) -> Saved {
        self.state.borrow().saved.clone()
    }

    /// File › Download › Word: save, and save a copy where the person chooses.
    pub fn download_next(&self) {
        self.state.borrow_mut().download_next = true;
    }

    pub fn rename(&self, stem: &str) {
        let r = self.state.borrow_mut().store.rename(stem);
        match r {
            Ok(()) => self.remember(),
            Err(e) => self.news(format!("Couldn't rename: {e}")),
        }
    }

    /// Saves now, without waiting for the quiet moment (closing the window).
    pub fn save_now(&self, app: &mut WordApp) {
        self.state.borrow_mut().dirty_since = None;
        crate::shell::run(app, "file.save", serde_json::json!({}));
    }

    /// Something isn't written yet: unsaved changes, or a save that failed.
    pub fn unsaved(&self, app: &WordApp) -> bool {
        app.session.dirty || self.state.borrow().retry.is_some()
    }

    /// Every frame: news for the status line, a picked file to open, and the save made by itself
    /// once the document has been left alone for a moment.
    pub fn tick(&self, app: &mut WordApp, ctx: &egui::Context) {
        let news = std::mem::take(&mut self.state.borrow_mut().news);
        let t = self.now();
        for msg in news {
            let mut s = self.state.borrow_mut();
            if s.saved != Saved::Failed(msg.clone()) {
                s.notice = Some((msg.clone(), t));
            }
            drop(s);
            app.status(msg);
        }
        let picked = {
            let mut s = self.state.borrow_mut();
            match s.picking.as_ref().map(Receiver::try_recv) {
                Some(Ok(p)) => {
                    s.picking = None;
                    p
                }
                Some(Err(TryRecvError::Disconnected)) => {
                    s.picking = None;
                    None
                }
                Some(Err(TryRecvError::Empty)) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(200));
                    None
                }
                None => None,
            }
        };
        if let Some(path) = picked {
            self.open_path(path);
        }
        let again = {
            let mut s = self.state.borrow_mut();
            match &s.retry {
                Some((.., at)) if t - at >= RETRY_MS => s.retry.take().map(|(name, bytes, _)| (name, bytes)),
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
        let since = {
            let mut s = self.state.borrow_mut();
            if !app.session.dirty {
                s.dirty_since = None;
                return;
            }
            *s.dirty_since.get_or_insert(t)
        };
        if t - since >= QUIET_MS && !ctx.input(|i| i.pointer.any_down()) {
            self.save_now(app);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    fn news(&self, msg: String) {
        self.state.borrow_mut().news.push(msg);
    }

    fn fail(&self, msg: &str) {
        let mut s = self.state.borrow_mut();
        s.saved = Saved::Failed(msg.to_string());
        s.news.push(msg.to_string());
    }

    /// One save: the file itself, a new file, or (changed elsewhere) a kept copy. A failed save
    /// is tried again after [`RETRY_MS`] unless a newer one comes.
    fn save(&self, name: &str, bytes: Vec<u8>) {
        let was_new = self.location().is_none();
        let r = {
            let mut s = self.state.borrow_mut();
            s.retry = None;
            s.store.write(name, &bytes)
        };
        let failed = match r {
            Ok(()) => None,
            Err(WriteError::ChangedElsewhere) => {
                let old = self.name();
                let (stem, ext) = split(&old);
                let copy = format!("{stem} (Starbase Docs copy {}){ext}", stamp());
                let kept = self.state.borrow_mut().store.keep_copy(&copy, &bytes);
                match kept {
                    Ok(()) => {
                        let shown = self.name();
                        self.news(format!(
                            "\"{old}\" was changed by another program while you were editing. That version is untouched; yours is kept as \"{shown}\", which is open now."
                        ));
                        self.remember();
                        None
                    }
                    Err(e) => Some(format!("Not saved: the file changed elsewhere and a copy couldn't be made ({e})")),
                }
            }
            Err(WriteError::Other(e)) => Some(format!("Not saved: {e}")),
        };
        match failed {
            None => {
                self.state.borrow_mut().saved = Saved::Done;
                if was_new {
                    self.remember();
                }
            }
            Some(why) => {
                self.fail(&format!("{why}. Trying again by itself."));
                let t = self.now();
                self.state.borrow_mut().retry = Some((name.to_string(), bytes, t));
            }
        }
    }
}

/// An export or File › Download: the system's Save dialog, on its own thread so the window
/// keeps drawing; the file is written when a place is chosen.
fn save_elsewhere(name: &str, bytes: &[u8]) -> Result<(), String> {
    let (name, bytes) = (name.to_string(), bytes.to_vec());
    std::thread::Builder::new()
        .name("save dialog".into())
        .spawn(move || {
            let dialog = rfd::AsyncFileDialog::new().set_file_name(&name);
            if let Some(file) = pollster::block_on(dialog.save_file())
                && let Err(e) = write_atomic(file.path(), &bytes)
            {
                log::error!("couldn't save {}: {e}", file.path().display());
            }
        })
        .map(|_| ())
        .map_err(|e| format!("couldn't show the Save dialog: {e}"))
}

/// A fresh folder for a test.
#[cfg(test)]
pub fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("stardocs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]
    use super::*;

    #[test]
    fn a_write_replaces_the_file_whole_and_leaves_no_temporary_file() {
        let dir = temp_dir("atomic");
        let path = dir.join("a.docx");
        std::fs::write(&path, b"old").unwrap();
        let mut f = LocalFile::open(path.clone());
        f.read().unwrap();
        f.write("a.docx", b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["a.docx"]);
    }

    #[test]
    fn a_file_changed_elsewhere_is_never_overwritten() {
        let dir = temp_dir("changed");
        let path = dir.join("a.docx");
        std::fs::write(&path, b"mine").unwrap();
        let mut f = LocalFile::open(path.clone());
        f.read().unwrap();
        std::fs::write(&path, b"someone else's, longer").unwrap();
        assert_eq!(f.write("a.docx", b"mine, edited"), Err(WriteError::ChangedElsewhere));
        assert_eq!(std::fs::read(&path).unwrap(), b"someone else's, longer");
    }

    #[test]
    fn a_new_document_gets_a_name_of_its_own() {
        let dir = temp_dir("new");
        std::fs::write(dir.join("Notes.docx"), b"x").unwrap();
        let mut f = LocalFile::new_in(dir.join("Starbase"));
        f.write("Notes.docx", b"y").unwrap();
        let mut g = LocalFile::new_in(dir.join("Starbase"));
        g.write("Notes.docx", b"z").unwrap();
        assert_eq!((f.name(), g.name()), ("Notes.docx".to_string(), "Notes (2).docx".to_string()));
    }

    #[test]
    fn the_documents_folder_comes_from_user_dirs() {
        let home = temp_dir("home");
        std::fs::create_dir_all(home.join(".config")).unwrap();
        std::fs::write(home.join(".config/user-dirs.dirs"), "XDG_DOCUMENTS_DIR=\"$HOME/Dokumente\"\n").unwrap();
        assert_eq!(documents_dir(&home, &home.join(".config")), Some(home.join("Dokumente")));
    }
}
