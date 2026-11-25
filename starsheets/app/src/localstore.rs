//! Local mode of the desktop app: files in a folder on this computer (`~/Documents/Starbase`),
//! no server and no accounts.
//!
//! [`LocalStore`] answers the Drive home screen's files-API calls (store.rs) from that folder, so
//! the same home screen works on it: listing, preview pictures, new spreadsheets, documents and
//! folders, uploading, renaming, copying, moving, and deleting to the system's Trash. Calls run on
//! a worker thread; their answers are handed back on the UI thread by [`Store::poll`].
//!
//! [`OnDisk`] is one open spreadsheet's file: written atomically (a temporary file, then a
//! rename) and never over a change made outside Starbase since it was read or last written
//! (spotted by its modification time and size). Such a save becomes "<name> (conflicted copy)"
//! next to it, the same rule the server follows for a save based on an older copy.
//!
//! Remembered when each file was last opened (the home screen's Recent) and the home screen's own
//! settings live in the data folder (`$XDG_DATA_HOME/starbase`), never inside Documents.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::SystemTime;

use serde_json::{Value as Json, json};

use crate::store::{Answer, Done, Method, Store, dec, enc};

/// Spreadsheets: open in Starbase Sheets (and get a preview picture).
const SHEETS: &[&str] = &["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv"];
/// Documents: open in Starbase Docs (the list AppRun uses).
const DOCS: &[&str] = &["docx", "docm", "dotx", "dotm", "doc", "odt", "rtf"];
/// Presentations: open in Starbase Slides (the list AppRun uses).
const SLIDES: &[&str] = &["pptx", "potx", "ppsx"];
/// Never handed to the system's default app from the Drive: it would run them (programs,
/// scripts, launchers). They are listed; open them from the file manager if meant.
const RUNS: &[&str] = &[
    "desktop",
    "sh",
    "bash",
    "zsh",
    "fish",
    "csh",
    "run",
    "bin",
    "appimage",
    "flatpakref",
    "jar",
    "py",
    "pl",
    "rb",
    "exe",
    "msi",
    "com",
    "bat",
    "cmd",
    "ps1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "scr",
    "lnk",
    "pif",
    "cpl",
    "hta",
    "reg",
    "app",
    "command",
    "pkg",
    "dmg",
    "deb",
    "rpm",
];
/// Files bigger than this get no preview picture (the home screen shows a plain icon).
const THUMB_MAX: u64 = 8 * 1024 * 1024;

// ------------------------------------------------------------ places

/// `~/Documents` (the XDG documents folder on Linux), or None without a home folder.
pub(crate) fn documents_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("XDG_DOCUMENTS_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    let home = PathBuf::from(std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?);
    let config = std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from);
    // xdg-user-dirs: XDG_DOCUMENTS_DIR="$HOME/Documents"
    if let Ok(text) = fs::read_to_string(config.join("user-dirs.dirs")) {
        for line in text.lines() {
            if let Some(v) = line.trim().strip_prefix("XDG_DOCUMENTS_DIR=") {
                let v = v.trim().trim_matches('"');
                let path = match v.strip_prefix("$HOME") {
                    Some(rest) => home.join(rest.trim_start_matches('/')),
                    None => PathBuf::from(v),
                };
                if path != home {
                    return Some(path);
                }
            }
        }
    }
    Some(home.join("Documents"))
}

/// The local Drive's folder: `STARBASE_LOCAL_DIR`, else "Starbase" in Documents.
pub(crate) fn default_root() -> PathBuf {
    if let Some(d) = std::env::var_os("STARBASE_LOCAL_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    documents_dir().unwrap_or_else(std::env::temp_dir).join("Starbase")
}

/// Where Starbase keeps its own small files (Recent, home screen settings).
pub(crate) fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("STARBASE_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let base = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()).map(PathBuf::from).or_else(|| {
        if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share"))
        }
    });
    base.unwrap_or_else(std::env::temp_dir).join("starbase")
}

/// "4 Mar 2026, 14:05" in local time.
pub(crate) fn date_text(unix: i64) -> String {
    use chrono::TimeZone as _;
    match chrono::Local.timestamp_opt(unix, 0).single() {
        Some(t) => t.format("%-d %b %Y, %H:%M").to_string(),
        None => String::new(),
    }
}

fn now_unix() -> i64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

// ------------------------------------------------------------ names

/// "Budget.xlsx" → ("Budget", ".xlsx"); a folder or a name without a known extension keeps it all.
fn split_name(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && e.len() <= 5 && !e.contains(' ') => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    }
}

/// A name that is safe as one path part: no slashes or control characters, not "." or "..".
fn clean_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '-' } else { c }).collect();
    let s = s.trim().trim_start_matches('.').to_string();
    if s.is_empty() { "Untitled".into() } else { s }
}

/// A path inside the root, from the home screen's relative form ("Work/2026"); refuses anything
/// that would leave it, and anything reached through a link (links are never followed, as the
/// listing doesn't: a link could point anywhere on the computer).
fn rel(root: &Path, path: &str) -> Result<PathBuf, (u16, String)> {
    let p = Path::new(path);
    if p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err((400, "That isn't a place in your files.".into()));
    }
    let mut at = root.to_path_buf();
    for c in p.components() {
        at.push(c);
        if fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err((404, "That isn't there any more.".into()));
        }
    }
    Ok(root.join(p))
}

/// Claims a free name in `dir` ("Name.xlsx", "Name (2).xlsx", …) by creating the file empty
/// (exclusive create, so two saves never pick the same name); `keep` is a path that may keep
/// its own name.
fn claim_file(dir: &Path, stem: &str, ext: &str, keep: Option<&Path>) -> std::io::Result<PathBuf> {
    for n in 1..10_000 {
        let name = if n == 1 { format!("{stem}{ext}") } else { format!("{stem} ({n}){ext}") };
        let p = dir.join(&name);
        if keep == Some(p.as_path()) {
            return Ok(p);
        }
        match fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(_) => return Ok(p),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free name"))
}

/// A free name in `dir` without creating it (for a rename or a move onto it).
fn free_path(dir: &Path, stem: &str, ext: &str, keep: Option<&Path>) -> PathBuf {
    for n in 1..10_000 {
        let name = if n == 1 { format!("{stem}{ext}") } else { format!("{stem} ({n}){ext}") };
        let p = dir.join(&name);
        if keep == Some(p.as_path()) || fs::symlink_metadata(&p).is_err() {
            return p;
        }
    }
    dir.join(format!("{stem} ({}){ext}", now_unix()))
}

/// Writes `bytes` to `path` all at once: into a temporary file next to it, then renamed over it,
/// so a crash or a full disk never leaves half a file.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.starbase-{}.tmp", std::process::id()));
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

// ------------------------------------------------------------ one open file

/// What a file on disk looked like: its modification time and size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

pub(crate) fn stamp(path: &Path) -> Option<Stamp> {
    let m = fs::metadata(path).ok()?;
    Some(Stamp { modified: m.modified().ok(), len: m.len() })
}

/// How a save went.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Saved {
    /// Written to the file.
    Written,
    /// The file had changed outside Starbase: written as this copy next to it instead, which is
    /// the file from now on.
    Copy(PathBuf),
}

/// A spreadsheet's file on disk, as last read or written here.
#[derive(Clone, Debug)]
pub(crate) struct OnDisk {
    pub path: PathBuf,
    stamp: Option<Stamp>,
}

impl OnDisk {
    /// A file just opened (or about to be): what it looks like now.
    pub fn open(path: PathBuf) -> OnDisk {
        let stamp = stamp(&path);
        OnDisk { path, stamp }
    }

    /// A new file in `dir` named after `stem` (a free name, claimed straight away).
    pub fn create(dir: &Path, stem: &str) -> std::io::Result<OnDisk> {
        fs::create_dir_all(dir)?;
        let path = claim_file(dir, &clean_name(stem), ".xlsx", None)?;
        Ok(OnDisk::open(path))
    }

    /// Whether the file changed outside Starbase since it was read or last written here.
    pub fn changed_outside(&self) -> bool {
        match (stamp(&self.path), self.stamp) {
            // Gone (deleted or moved away): writing it again overwrites nothing.
            (None, _) => false,
            (Some(now), Some(then)) => now != then,
            (Some(_), None) => true,
        }
    }

    /// Takes the file as it is now as the one this copy is based on (after Starbase itself wrote
    /// it some other way, such as GridCraft's own Save).
    pub fn refresh(&mut self) {
        self.stamp = stamp(&self.path);
    }

    /// Saves `bytes`: over the file, unless it changed outside Starbase, then as
    /// "<name> (conflicted copy)" next to it (which this then follows).
    pub fn save(&mut self, bytes: &[u8]) -> std::io::Result<Saved> {
        let mut result = Saved::Written;
        if self.changed_outside() {
            let dir = self.path.parent().unwrap_or(Path::new(".")).to_path_buf();
            let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (stem, ext) = split_name(&name);
            let copy = claim_file(&dir, &format!("{stem} (conflicted copy)"), if ext.is_empty() { ".xlsx" } else { &ext }, None)?;
            self.path = copy.clone();
            result = Saved::Copy(copy);
        }
        write_atomic(&self.path, bytes)?;
        self.stamp = stamp(&self.path);
        Ok(result)
    }

    /// Renames the file to `stem` (its extension stays), to a free name in the same folder.
    pub fn rename(&mut self, stem: &str) -> std::io::Result<()> {
        let dir = self.path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let (_, ext) = split_name(&name);
        let to = free_path(&dir, &clean_name(stem), &ext, Some(&self.path));
        if to != self.path {
            fs::rename(&self.path, &to)?;
            self.path = to;
            self.stamp = stamp(&self.path);
        }
        Ok(())
    }
}

/// A file name's stem as the title bar shows it ("Budget.xlsx" → "Budget").
pub(crate) fn stem_of(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    split_name(&name).0
}

// ------------------------------------------------------------ remembered things

fn read_json(path: &Path) -> Json {
    fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({}))
}

fn update_json(path: &Path, change: impl FnOnce(&mut serde_json::Map<String, Json>)) {
    let mut v = read_json(path);
    if !v.is_object() {
        v = json!({});
    }
    if let Some(m) = v.as_object_mut() {
        change(m);
    }
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(b) = serde_json::to_vec_pretty(&v) {
        let _ = write_atomic(path, &b);
    }
}

/// Remembers that a file was opened now (the home screen's Recent).
pub(crate) fn note_opened(data: &Path, file: &Path) {
    let key = file.to_string_lossy().into_owned();
    update_json(&data.join("opened.json"), |m| {
        m.insert(key, json!(now_unix()));
        // Only the latest few hundred.
        if m.len() > 500 {
            let mut all: Vec<(String, i64)> = m.iter().map(|(k, v)| (k.clone(), v.as_i64().unwrap_or(0))).collect();
            all.sort_by_key(|(_, t)| std::cmp::Reverse(*t));
            *m = all.into_iter().take(400).map(|(k, t)| (k, json!(t))).collect();
        }
    });
}

// ------------------------------------------------------------ programs

/// Another Starbase program (`starsheets`, `stardocs`): next to this one (the AppImage, the
/// Flatpak and the tar all put them side by side), else on PATH.
pub(crate) fn program(name: &str) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let here = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(&file)));
    if let Some(p) = here.filter(|p| p.is_file()) {
        return Some(p);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(&file)).find(|p| p.is_file())
}

/// Starts `name` with `args` in its own window (and process); says why when it can't.
pub(crate) fn launch(name: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let Some(mut cmd) = program_command(name, std::env::var_os("APPIMAGE").map(PathBuf::from)) else {
        let app = match name {
            "stardocs" => "Starbase Docs",
            "starslides" => "Starbase Slides",
            _ => "Starbase Sheets",
        };
        return Err(format!("{app} isn't installed next to Starbase Drive, so this can't be opened here."));
    };
    let exe = PathBuf::from(cmd.get_program());
    cmd.args(args);
    start_detached(cmd).map(|_| ()).map_err(|e| format!("Couldn't start {}: {e}", exe.display()))
}

/// The command that starts program `name`. Inside the AppImage (`$APPIMAGE`, set by its runtime)
/// that is the AppImage itself with the program's name first (AppRun picks the program by it):
/// the new program gets a mount of its own and keeps its files when this one, and with it this
/// one's mount, goes. Otherwise this program's own file for `starsheets`, else [`program`].
fn program_command(name: &str, appimage: Option<PathBuf>) -> Option<std::process::Command> {
    if let Some(image) = appimage.filter(|p| p.is_file()) {
        let mut cmd = std::process::Command::new(image);
        cmd.arg(name);
        return Some(cmd);
    }
    let exe = if name == "starsheets" { std::env::current_exe().ok() } else { None }.or_else(|| program(name))?;
    Some(std::process::Command::new(exe))
}

/// Starts a program that lives on by itself: in a process group of its own (Ctrl+C in the
/// terminal that started this one doesn't close it, nor does closing this window), and waited
/// for on a thread of its own, so it doesn't linger as a zombie once closed while this one runs
/// on. Returns its process id.
pub(crate) fn start_detached(mut cmd: std::process::Command) -> std::io::Result<u32> {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn()?;
    let pid = child.id();
    std::thread::Builder::new().name(format!("wait-{pid}")).spawn(move || {
        let _ = child.wait();
    })?;
    Ok(pid)
}

// ------------------------------------------------------------ the person

/// The person's name as the computer knows it, for signing comments in local mode: the full name
/// in the user account (the first part of the passwd "GECOS" field), else the login name.
pub(crate) fn os_user_name() -> String {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            if let (Ok(m), Ok(passwd)) = (fs::metadata("/proc/self"), fs::read_to_string("/etc/passwd"))
                && let Some(n) = passwd_name(&passwd, m.uid())
            {
                return n;
            }
        }
        ["USER", "LOGNAME", "USERNAME"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty())).unwrap_or_else(|| "Me".into())
    })
    .clone()
}

/// The full name of user `uid` in an /etc/passwd text, else its login name.
#[cfg(any(unix, test))]
fn passwd_name(passwd: &str, uid: u32) -> Option<String> {
    passwd.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        if f.len() < 5 || f[2].parse::<u32>().ok()? != uid {
            return None;
        }
        let full = f[4].split(',').next().unwrap_or("").trim();
        let name = if full.is_empty() { f[0].trim() } else { full };
        (!name.is_empty()).then(|| name.to_string())
    })
}

/// Where a file on this computer opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpensWith {
    Sheets,
    Docs,
    Slides,
    /// The system's default app for its type (pictures, PDFs…).
    System,
    /// Not from the Drive: a program, script or launcher (or a file marked runnable).
    Refused,
}

/// Where a file named `name` opens; `runnable`: marked as a program (Unix execute bit).
pub(crate) fn opens_with(name: &str, runnable: bool) -> OpensWith {
    let ext = name.rsplit_once('.').map_or(String::new(), |(_, e)| e.to_ascii_lowercase());
    if SHEETS.contains(&ext.as_str()) {
        OpensWith::Sheets
    } else if DOCS.contains(&ext.as_str()) {
        OpensWith::Docs
    } else if SLIDES.contains(&ext.as_str()) {
        OpensWith::Slides
    } else if runnable || RUNS.contains(&ext.as_str()) {
        OpensWith::Refused
    } else {
        OpensWith::System
    }
}

/// Whether the file is marked as a program.
fn runnable(m: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        false
    }
}

/// Opens a file in the right program: spreadsheets in Starbase Sheets, documents in Starbase Docs,
/// presentations in Starbase Slides, anything else in the system's default app for it (never a program or script, never through a
/// link).
pub(crate) fn open_in_program(path: &Path) -> Result<(), String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let m = fs::symlink_metadata(path).map_err(|_| format!("{name} isn't there any more."))?;
    if !m.is_file() {
        return Err(format!("{name} can't be opened here."));
    }
    match opens_with(&name, runnable(&m)) {
        OpensWith::Sheets => launch("starsheets", &[path.as_os_str()]),
        OpensWith::Docs => launch("stardocs", &[path.as_os_str()]),
        OpensWith::Slides => launch("starslides", &[path.as_os_str()]),
        OpensWith::Refused => {
            Err(format!("{name} is a program or script, so Starbase doesn't open it. Open it from your file manager if you meant to."))
        }
        OpensWith::System => start_detached(system_open(path)).map(|_| ()).map_err(|e| format!("Couldn't open {name}: {e}")),
    }
}

/// The system's "open this file with its default app", the path as one argument (no shell).
fn system_open(path: &Path) -> std::process::Command {
    #[cfg(target_os = "windows")]
    let mut cmd = std::process::Command::new("explorer");
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(path);
    cmd
}

/// A new, empty Word document (the smallest .docx Word and Starbase Docs open).
pub(crate) fn empty_docx() -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let parts: [(&str, &str); 3] = [
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        (
            "word/document.xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p/></w:body></w:document>"#,
        ),
    ];
    let mut out = std::io::Cursor::new(Vec::new());
    let mut z = zip::ZipWriter::new(&mut out);
    for (name, text) in parts {
        if z.start_file(name, SimpleFileOptions::default()).is_err() || z.write_all(text.as_bytes()).is_err() {
            return Vec::new();
        }
    }
    let _ = z.finish();
    out.into_inner()
}

// ------------------------------------------------------------ the store

/// The home screen's files, in a folder on this computer.
pub(crate) struct LocalStore {
    root: PathBuf,
    data: PathBuf,
    tx: Sender<(u64, Answer)>,
    rx: Receiver<(u64, Answer)>,
    waiting: RefCell<HashMap<u64, Done>>,
    next: Cell<u64>,
}

impl LocalStore {
    pub fn new(root: PathBuf) -> LocalStore {
        LocalStore::with_data(root, data_dir())
    }

    pub fn with_data(root: PathBuf, data: PathBuf) -> LocalStore {
        let _ = fs::create_dir_all(&root);
        let (tx, rx) = channel();
        LocalStore { root, data, tx, rx, waiting: RefCell::default(), next: Cell::new(1) }
    }

    fn settings_path(&self) -> PathBuf {
        self.data.join("drive.json")
    }
}

impl Store for LocalStore {
    fn call(&self, ctx: &egui::Context, method: Method, url: &str, body: Option<Vec<u8>>, done: Done) {
        let id = self.next.get();
        self.next.set(id + 1);
        self.waiting.borrow_mut().insert(id, done);
        let (tx, ctx, root, data, url) = (self.tx.clone(), ctx.clone(), self.root.clone(), self.data.clone(), url.to_string());
        std::thread::spawn(move || {
            let answer = answer(&root, &data, method, &url, body.as_deref());
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

    fn open(&self, id: &str, _name: &str) -> Result<(), String> {
        let path = rel(&self.root, &dec(id)).map_err(|(_, e)| e)?;
        open_in_program(&path)?;
        note_opened(&self.data, &path);
        Ok(())
    }

    fn opens_any_file(&self) -> bool {
        true
    }

    fn new_file(&self, folder: &str, document: bool) -> Result<(), String> {
        let dir = rel(&self.root, folder).map_err(|(_, e)| e)?;
        if document {
            let path = claim_file(&dir, "Untitled document", ".docx", None).map_err(|e| e.to_string())?;
            write_atomic(&path, &empty_docx()).map_err(|e| e.to_string())?;
            note_opened(&self.data, &path);
            launch("stardocs", &[path.as_os_str()])
        } else {
            launch("starsheets", &["--new".as_ref(), dir.as_os_str()])
        }
    }

    fn pick_files(&self, got: Box<dyn Fn(String, Vec<u8>)>) {
        // The Drive keeps any kind of file, as on a server.
        let picked = rfd::FileDialog::new().pick_files().unwrap_or_default();
        for p in picked {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if let Ok(bytes) = fs::read(&p) {
                got(name, bytes);
            }
        }
    }

    fn download(&self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let Some(to) = rfd::FileDialog::new().set_file_name(name).save_file() else { return Ok(()) };
        write_atomic(&to, bytes).map_err(|e| e.to_string())
    }

    fn setting(&self, key: &str) -> Option<String> {
        read_json(&self.settings_path())[key].as_str().map(str::to_string)
    }

    fn set_setting(&self, key: &str, value: &str) {
        update_json(&self.settings_path(), |m| {
            m.insert(key.to_string(), json!(value));
        });
    }
}

// ------------------------------------------------------------ the files API, from a folder

fn reply(status: u16, v: Json) -> Answer {
    Ok((status, serde_json::to_vec(&v).unwrap_or_default()))
}

fn refuse(status: u16, msg: &str) -> Answer {
    reply(status, json!({ "error": msg }))
}

fn io(e: std::io::Error) -> Answer {
    refuse(500, &e.to_string())
}

/// Splits "api/files/ID/rename?name=X" into its path parts and its query.
fn parse(url: &str) -> (Vec<String>, HashMap<String, String>) {
    let (path, q) = url.split_once('?').unwrap_or((url, ""));
    let parts = path.split('/').filter(|p| !p.is_empty()).map(str::to_string).collect();
    let query = q.split('&').filter(|kv| !kv.is_empty()).map(|kv| kv.split_once('=').unwrap_or((kv, ""))).map(|(k, v)| (dec(k), dec(v))).collect();
    (parts, query)
}

fn version(m: &fs::Metadata) -> String {
    let t = m.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    format!("{t:x}-{:x}", m.len())
}

fn modified(m: &fs::Metadata) -> i64 {
    m.modified().ok().and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64)
}

/// Whether the home screen lists a file: every kind (as the server's Drive), except hidden ones
/// (our own temporary files among them), Office's lock files and the systems' folder clutter.
fn listed(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.starts_with('.') && !name.starts_with("~$") && !matches!(lower.as_str(), "desktop.ini" | "thumbs.db")
}

/// The relative form of a path under the root ("Work/2026"), with '/' between parts.
fn rel_text(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

/// Everything under the root: GET api/files, in the server's shape (no "space": no accounts).
pub(crate) fn list(root: &Path, data: &Path) -> Json {
    let opened = read_json(&data.join("opened.json"));
    let (mut files, mut folders) = (Vec::new(), Vec::new());
    let mut todo = vec![root.to_path_buf()];
    while let Some(dir) = todo.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            // Links aren't followed (a link to a parent folder would never end).
            let Ok(m) = fs::symlink_metadata(e.path()) else { continue };
            if name.starts_with('.') {
                continue;
            }
            if m.is_dir() {
                folders.push(rel_text(root, &e.path()));
                todo.push(e.path());
            } else if m.is_file() && listed(&name) {
                let path = rel_text(root, &e.path());
                let full = e.path().to_string_lossy().into_owned();
                files.push(json!({
                    "id": enc(&path),
                    "name": name,
                    "folder": rel_text(root, &dir),
                    "modified": modified(&m),
                    "size": m.len(),
                    "version": version(&m),
                    "opened": opened[&full].as_i64(),
                }));
            }
        }
    }
    folders.sort();
    json!({ "files": files, "folders": folders })
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir(to)?;
    for e in fs::read_dir(from)?.flatten() {
        let m = fs::symlink_metadata(e.path())?;
        if m.is_dir() {
            copy_dir(&e.path(), &to.join(e.file_name()))?;
        } else if m.is_file() {
            fs::copy(e.path(), to.join(e.file_name()))?;
        }
    }
    Ok(())
}

/// Answers one files-API call from the folder `root` (see the server's crates/server/src/store.rs
/// for the same calls on a server). Runs on a worker thread.
pub(crate) fn answer(root: &Path, data: &Path, method: Method, url: &str, body: Option<&[u8]>) -> Answer {
    let (parts, q) = parse(url);
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    let qs = |k: &str| q.get(k).cloned().unwrap_or_default();
    macro_rules! place {
        ($p:expr) => {
            match rel(root, $p) {
                Ok(p) => p,
                Err((s, e)) => return refuse(s, &e),
            }
        };
    }
    match (method, parts.as_slice()) {
        (Method::Get, ["api", "files"]) => reply(200, list(root, data)),
        (Method::Post, ["api", "files"]) => {
            let dir = place!(&qs("folder"));
            if !dir.is_dir() {
                return refuse(404, "That folder isn't there any more.");
            }
            let (stem, ext) = split_name(&clean_name(&qs("name")));
            // An upload keeps its own name (`as_is`: README stays README); a new workbook saved
            // without an extension becomes .xlsx.
            let ext = if ext.is_empty() && !q.contains_key("as_is") { ".xlsx".to_string() } else { ext };
            let path = match claim_file(&dir, &stem, &ext, None) {
                Ok(p) => p,
                Err(e) => return io(e),
            };
            if let Err(e) = write_atomic(&path, body.unwrap_or_default()) {
                let _ = fs::remove_file(&path);
                return io(e);
            }
            let rp = rel_text(root, &path);
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let v = fs::metadata(&path).map(|m| version(&m)).unwrap_or_default();
            reply(201, json!({ "id": enc(&rp), "name": name, "folder": rel_text(root, &dir), "version": v }))
        }
        (_, ["api", "files", id, rest @ ..]) => {
            let path = place!(&dec(id));
            let Ok(m) = fs::metadata(&path) else { return refuse(404, "That file isn't there any more.") };
            let dir = path.parent().unwrap_or(root).to_path_buf();
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match (method, rest) {
                (Method::Get, []) => fs::read(&path).map_or_else(io, |b| Ok((200, b))),
                (Method::Delete, []) => match trash::delete(&path) {
                    Ok(()) => reply(200, json!({ "ok": true })),
                    Err(e) => refuse(500, &format!("Couldn't move it to the Trash: {e}")),
                },
                (Method::Get, ["thumb"]) => {
                    if opens_with(&name, false) != OpensWith::Sheets || m.len() > THUMB_MAX {
                        return refuse(404, "no picture");
                    }
                    let drawn = fs::read(&path)
                        .map_err(|e| e.to_string())
                        .and_then(|b| gridcraft_engine::io::open_bytes(&name, &b).map_err(|e| e.to_string()))
                        .and_then(|(wb, _)| starsheets_sync::thumb::draw(&wb));
                    match drawn {
                        Ok(png) => Ok((200, png)),
                        Err(e) => refuse(404, &e),
                    }
                }
                (Method::Post, ["rename"]) => {
                    let (stem, _) = split_name(&clean_name(&qs("name")));
                    let (_, ext) = split_name(&name);
                    let to = free_path(&dir, &stem, &ext, Some(&path));
                    match fs::rename(&path, &to) {
                        Ok(()) => reply(200, json!({ "name": to.file_name().map(|n| n.to_string_lossy().into_owned()) })),
                        Err(e) => io(e),
                    }
                }
                (Method::Post, ["move"]) => {
                    let to_dir = place!(&qs("folder"));
                    if to_dir == dir {
                        return reply(200, json!({ "name": name }));
                    }
                    let (stem, ext) = split_name(&name);
                    let to = free_path(&to_dir, &stem, &ext, None);
                    match fs::rename(&path, &to) {
                        Ok(()) => reply(200, json!({ "name": to.file_name().map(|n| n.to_string_lossy().into_owned()) })),
                        Err(e) => io(e),
                    }
                }
                (Method::Post, ["copy"]) => {
                    let to_dir = match q.get("folder") {
                        Some(f) => place!(f),
                        None => dir.clone(),
                    };
                    let (mut stem, ext) = split_name(&name);
                    if to_dir == dir {
                        stem = format!("Copy of {stem}");
                    }
                    let to = match claim_file(&to_dir, &stem, &ext, None) {
                        Ok(p) => p,
                        Err(e) => return io(e),
                    };
                    match fs::copy(&path, &to) {
                        Ok(_) => {
                            reply(200, json!({ "id": enc(&rel_text(root, &to)), "name": to.file_name().map(|n| n.to_string_lossy().into_owned()) }))
                        }
                        Err(e) => io(e),
                    }
                }
                _ => refuse(404, "That isn't available for files on this computer."),
            }
        }
        (Method::Post, ["api", "folders"]) => {
            let parent = place!(&qs("path"));
            let name = clean_name(&qs("name"));
            let to = free_path(&parent, &name, "", None);
            match fs::create_dir_all(&to) {
                Ok(()) => reply(201, json!({ "path": rel_text(root, &to) })),
                Err(e) => io(e),
            }
        }
        (Method::Delete, ["api", "folders"]) => {
            let path = place!(&qs("path"));
            if path == root {
                return refuse(400, "That's the top of your files.");
            }
            match trash::delete(&path) {
                Ok(()) => reply(200, json!({ "ok": true })),
                Err(e) => refuse(500, &format!("Couldn't move it to the Trash: {e}")),
            }
        }
        (Method::Post, ["api", "folders", what @ ("rename" | "move" | "copy")]) => {
            let path = place!(&qs("path"));
            if path == root || !path.is_dir() {
                return refuse(404, "That folder isn't there any more.");
            }
            let parent = path.parent().unwrap_or(root).to_path_buf();
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let to = match *what {
                "rename" => free_path(&parent, &clean_name(&qs("name")), "", Some(&path)),
                _ => {
                    let into = place!(&qs("to"));
                    if into.starts_with(&path) {
                        return refuse(400, "A folder can't go inside itself.");
                    }
                    if *what == "move" && into == parent {
                        return reply(200, json!({ "path": rel_text(root, &path) }));
                    }
                    let name = if *what == "copy" && into == parent { format!("Copy of {name}") } else { name };
                    free_path(&into, &name, "", None)
                }
            };
            let done = if *what == "copy" { copy_dir(&path, &to) } else { fs::rename(&path, &to) };
            match done {
                Ok(()) => reply(200, json!({ "path": rel_text(root, &to) })),
                Err(e) => io(e),
            }
        }
        // Deleted files are in the system's Trash, which the file manager shows.
        (Method::Get, ["api", "bin"]) => reply(200, json!([])),
        _ => refuse(404, "That isn't available for files on this computer."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Linux only: it checks /proc, and `true` is a Unix program.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_started_program_is_reaped_when_it_ends() {
        let pid = start_detached(std::process::Command::new("true")).unwrap();
        // Reaped = gone from /proc; an unreaped one stays there as a zombie.
        let proc = PathBuf::from(format!("/proc/{pid}"));
        let gone = (0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            !proc.exists()
        });
        assert!(gone, "process {pid} was left behind as a zombie");
    }

    #[test]
    fn inside_the_appimage_programs_start_through_it() {
        let dir = temp("appimage");
        let image = dir.join("Starbase.AppImage");
        fs::write(&image, b"").unwrap();
        let cmd = program_command("stardocs", Some(image.clone())).unwrap();
        assert_eq!(cmd.get_program(), image.as_os_str());
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["stardocs"], "AppRun picks the program by its name");
        // Not inside an AppImage (or it's gone): this program's own file.
        let cmd = program_command("starsheets", Some(dir.join("missing.AppImage"))).unwrap();
        assert_eq!(PathBuf::from(cmd.get_program()), std::env::current_exe().unwrap());
        assert_eq!(cmd.get_args().count(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_name_comes_from_the_account() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\nada:x:1000:1000:Ada Lovelace,,,:/home/ada:/bin/sh\nbob:x:1001:1001::/home/bob:/bin/sh\n";
        assert_eq!(passwd_name(passwd, 1000).as_deref(), Some("Ada Lovelace"));
        assert_eq!(passwd_name(passwd, 1001).as_deref(), Some("bob"), "no full name: the login name");
        assert_eq!(passwd_name(passwd, 4242), None);
        assert!(!os_user_name().trim().is_empty());
    }

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("starbase-local-{tag}-{}-{}", std::process::id(), now_unix()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn call(root: &Path, method: Method, url: &str, body: Option<&[u8]>) -> (u16, Json) {
        let (s, b) = answer(root, &root.join(".data"), method, url, body).unwrap();
        (s, serde_json::from_slice(&b).unwrap_or(Json::Null))
    }

    #[test]
    fn lists_folders_and_every_kind_of_file() {
        let root = temp("list");
        fs::create_dir_all(root.join("Work/2026")).unwrap();
        fs::write(root.join("Budget.xlsx"), b"x").unwrap();
        fs::write(root.join("Work/2026/Plan.csv"), b"a,b").unwrap();
        fs::write(root.join("Work/notes.txt"), b"text").unwrap();
        fs::write(root.join("Photo.JPG"), b"jpg").unwrap();
        fs::write(root.join("README"), b"no extension").unwrap();
        fs::write(root.join(".hidden.xlsx"), b"not listed").unwrap();
        fs::write(root.join("~$Budget.xlsx"), b"Office's lock file: not listed").unwrap();
        fs::write(root.join("desktop.ini"), b"not listed").unwrap();
        #[cfg(unix)]
        {
            // Links are never listed or followed: not the file, not the folder.
            std::os::unix::fs::symlink(root.join("Budget.xlsx"), root.join("Link.xlsx")).unwrap();
            std::os::unix::fs::symlink(root.join("Work"), root.join("LinkedFolder")).unwrap();
        }
        let (s, v) = call(&root, Method::Get, "api/files", None);
        assert_eq!(s, 200);
        assert_eq!(v["folders"], json!(["Work", "Work/2026"]));
        let mut names: Vec<(String, String)> =
            v["files"].as_array().unwrap().iter().map(|f| (f["folder"].as_str().unwrap().into(), f["name"].as_str().unwrap().into())).collect();
        names.sort();
        let want = [("", "Budget.xlsx"), ("", "Photo.JPG"), ("", "README"), ("Work", "notes.txt"), ("Work/2026", "Plan.csv")];
        assert_eq!(names, want.map(|(a, b)| (a.to_string(), b.to_string())));
        assert!(v.get("space").is_none(), "no accounts locally");
        #[cfg(unix)]
        {
            for id in ["Link.xlsx", "LinkedFolder/notes.txt"] {
                assert_eq!(call(&root, Method::Get, &format!("api/files/{}", enc(id)), None).0, 404, "{id}: never through a link");
            }
            assert!(LocalStore::with_data(root.clone(), root.join(".data")).open(&enc("Link.xlsx"), "Link.xlsx").is_err());
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn each_kind_opens_in_its_place() {
        use OpensWith::*;
        for (name, want) in [
            ("Budget.xlsx", Sheets),
            ("data.TSV", Sheets),
            ("old.xlsb", Sheets),
            ("Letter.docx", Docs),
            ("notes.rtf", Docs),
            ("Talk.pptx", Slides),
            ("Theme.POTX", Slides),
            ("Photo.jpg", System),
            ("scan.pdf", System),
            ("README", System),
            ("setup.sh", Refused),
            ("Game.AppImage", Refused),
            ("x.desktop", Refused),
            ("Setup.exe", Refused),
            ("run.bat", Refused),
        ] {
            assert_eq!(opens_with(name, false), want, "{name}");
        }
        assert_eq!(opens_with("tool", true), Refused, "marked runnable");
        assert_eq!(opens_with("Budget.xlsx", true), Sheets, "a workbook with the execute bit (some shares set it) still opens in Sheets");
        let cmd = system_open(Path::new("/tmp/a b; rm -rf x.jpg"));
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), ["/tmp/a b; rm -rf x.jpg"], "one argument, no shell");
        // A program is refused before anything starts.
        let root = temp("refuse");
        fs::write(root.join("setup.sh"), b"echo hi").unwrap();
        let e = LocalStore::with_data(root.clone(), root.join(".data")).open(&enc("setup.sh"), "setup.sh").unwrap_err();
        assert!(e.contains("program or script"), "{e}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn uploads_keep_their_own_name() {
        let root = temp("upload");
        let (s, v) = call(&root, Method::Post, "api/files?name=README&folder=&as_is=1", Some(b"hello"));
        assert_eq!((s, v["name"].as_str()), (201, Some("README")));
        let (_, v) = call(&root, Method::Post, "api/files?name=Photo.jpg&folder=&as_is=1", Some(b"jpg"));
        assert_eq!(v["name"].as_str(), Some("Photo.jpg"));
        let (_, v) = call(&root, Method::Post, "api/files?name=Untitled&folder=", Some(b"x"));
        assert_eq!(v["name"].as_str(), Some("Untitled.xlsx"), "a new workbook without an extension is still .xlsx");
        assert_eq!(call(&root, Method::Get, &format!("api/files/{}/thumb", enc("Photo.jpg")), None).0, 404, "pictures of sheets only");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn makes_renames_copies_and_moves() {
        let root = temp("ops");
        let (s, v) = call(&root, Method::Post, "api/folders?path=&name=New%20folder", None);
        assert_eq!((s, v["path"].as_str()), (201, Some("New folder")));
        let (_, v) = call(&root, Method::Post, "api/folders?path=&name=New%20folder", None);
        assert_eq!(v["path"].as_str(), Some("New folder (2)"));
        let (s, v) = call(&root, Method::Post, "api/files?name=Budget.xlsx&folder=New%20folder", Some(b"data"));
        assert_eq!(s, 201);
        let id = v["id"].as_str().unwrap().to_string();
        assert_eq!(dec(&id), "New folder/Budget.xlsx");
        let (_, v) = call(&root, Method::Post, &format!("api/files/{id}/rename?name=Plan"), None);
        assert_eq!(v["name"].as_str(), Some("Plan.xlsx"), "the extension stays");
        let id = enc("New folder/Plan.xlsx");
        let (_, v) = call(&root, Method::Post, &format!("api/files/{id}/copy"), None);
        assert_eq!(v["name"].as_str(), Some("Copy of Plan.xlsx"));
        let (_, v) = call(&root, Method::Post, &format!("api/files/{id}/move?folder="), None);
        assert_eq!(v["name"].as_str(), Some("Plan.xlsx"));
        assert_eq!(fs::read(root.join("Plan.xlsx")).unwrap(), b"data");
        let (s, _) = call(&root, Method::Post, "api/folders/move?path=New%20folder&to=New%20folder%2Fx", None);
        assert_eq!(s, 400, "not inside itself");
        let (s, _) = call(&root, Method::Get, &format!("api/files/{}", enc("../outside.xlsx")), None);
        assert_eq!(s, 400, "never outside the folder");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn draws_a_preview_picture() {
        let root = temp("thumb");
        fs::write(root.join("t.csv"), b"Name,Amount\nRent,1200\n").unwrap();
        let (s, b) = answer(&root, &root, Method::Get, &format!("api/files/{}/thumb", enc("t.csv")), None).unwrap();
        assert_eq!((s, &b[1..4]), (200, &b"PNG"[..]));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn saves_atomically_and_keeps_outside_changes() {
        let root = temp("save");
        let mut f = OnDisk::create(&root, "Budget").unwrap();
        assert_eq!(f.path, root.join("Budget.xlsx"));
        assert_eq!(f.save(b"one").unwrap(), Saved::Written);
        assert_eq!(fs::read(&f.path).unwrap(), b"one");
        // Someone else changes the file (another program, or a sync tool).
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(root.join("Budget.xlsx"), b"theirs, longer").unwrap();
        assert!(f.changed_outside());
        let copy = root.join("Budget (conflicted copy).xlsx");
        assert_eq!(f.save(b"mine").unwrap(), Saved::Copy(copy.clone()));
        assert_eq!(fs::read(root.join("Budget.xlsx")).unwrap(), b"theirs, longer", "never overwritten");
        assert_eq!(fs::read(&copy).unwrap(), b"mine");
        assert_eq!(f.save(b"mine again").unwrap(), Saved::Written, "the copy is the file from now on");
        f.rename("Renamed").unwrap();
        assert_eq!(f.path, root.join("Renamed.xlsx"));
        assert_eq!(fs::read(&f.path).unwrap(), b"mine again");
        let leftovers: Vec<_> = fs::read_dir(&root).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
        assert!(leftovers.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_document_is_a_zip() {
        let b = empty_docx();
        assert_eq!(&b[..2], b"PK");
    }
}
