//! The file store: spreadsheets as ordinary .xlsx files in a folder tree.
//!
//! Same layout and behaviour as the earlier server/files.py, so an existing folder keeps working:
//! readable names (a taken name gets " (2)", claimed with an exclusive create), hidden ids in
//! `.starsheets/index.json`, the last KEEP versions in `.starsheets/versions/<id>/`, and a bin in
//! `.bin` (original places in `.starsheets/bin.json`). Folders and files starting with "." are
//! not shown. Callers hold one lock around every call, as the Python server did.
//!
//! With accounts (`spaces`), each person has a folder at the top named after their username
//! (`<username>/…`), and their bin is `<username>/.bin`; bin.json keys are then
//! "<username>/<name in the bin>". Folders get hidden ids too (`.starsheets/folders.json`), so
//! sharing a folder survives renaming and moving it. Something moved to the bin remembers the ids
//! inside it, so restoring brings back its versions and sharing. Things stay in a bin for
//! BIN_DAYS days.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

/// Days something stays in the bin before it's deleted for good.
pub const BIN_DAYS: i64 = 30;

/// Files the apps edit (Sheets, Docs and Slides). Every other file is stored, listed, shared, moved and
/// binned and versioned the same way, but is never opened in a live room: the Drive
/// previews it (preview.rs) or downloads it.
pub const EXTS: [&str; 8] = [".xlsx", ".xlsm", ".xls", ".csv", ".ods", ".docx", ".odt", ".pptx"];

/// Documents (StarDocs) and presentations (Starbase Slides) rather than spreadsheets: stored,
/// listed, shared and versioned like any file, but never opened in a live spreadsheet room or
/// drawn as a sheet preview.
pub const DOC_EXTS: [&str; 3] = [".docx", ".odt", ".pptx"];

/// Whether a file name is a StarDocs document.
pub fn is_doc(name: &str) -> bool {
    let lower = name.to_lowercase();
    DOC_EXTS.iter().any(|x| lower.ends_with(x))
}

/// The biggest file of another type (photo, PDF, video...) that keeps versions: copies of bigger
/// ones would eat the disk, so their old content goes to the bin instead.
pub const MAX_VERSIONED_OTHER: u64 = 64 * 1024 * 1024;

/// Whether a file is one the apps edit (a spreadsheet, document or presentation).
pub fn is_editable(name: &str) -> bool {
    let lower = name.to_lowercase();
    EXTS.iter().any(|x| lower.ends_with(x))
}

/// A request that can't be done: (http status, message).
#[derive(Debug)]
pub struct Bad {
    pub code: u16,
    pub msg: String,
}

impl Bad {
    pub fn new(code: u16, msg: impl Into<String>) -> Self {
        Bad { code, msg: msg.into() }
    }
}

impl From<io::Error> for Bad {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::NotFound { Bad::new(404, "not found") } else { Bad::new(500, format!("storage error: {e}")) }
    }
}

pub type R<T> = Result<T, Bad>;

fn qs<'a>(q: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    q.get(key).map(String::as_str)
}

/// What a request answers with.
pub enum Reply {
    Json(Value, u16),
    File { data: Vec<u8>, name: String, version: String, folder: String, modified: i64 },
}

fn ok(v: Value) -> R<Reply> {
    Ok(Reply::Json(v, 200))
}

// ---------------------------------------------------------------- small helpers

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::fill(&mut buf[..]);
    hex::encode(buf)
}

fn mtime(meta: &fs::Metadata) -> (i64, u128) {
    let d = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).unwrap_or_default();
    (d.as_secs() as i64, d.as_nanos())
}

/// "YYYYmmdd-HHMMSS" (UTC) for version file names.
fn stamp() -> String {
    let (y, m, d, h, mi, s) = civil(now() as i64);
    format!("{y:04}{m:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// What each earlier version is (when saved, by whom, its name), in its file's versions folder;
/// `current` describes the file as it is now until the next save makes it a version.
const VERSIONS_META: &str = "versions.json";

/// The longest version name.
const MAX_VERSION_NAME: usize = 100;

const DAY: i64 = 86_400;

/// Seconds per step when [`thin`] compares how close versions are (a minute).
const GAP_UNIT: i64 = 60;

/// The versions in a versions folder, oldest first (their names sort by when they were made).
fn version_names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        fs::read_dir(folder).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    names.retain(|n| !n.starts_with('.') && !n.ends_with(".json") && !n.ends_with(".tmp"));
    names.sort();
    names
}

/// A version's name without its extension (".xlsx", ".docx", …).
fn version_stem(name: &str) -> &str {
    name.split_once('.').map_or(name, |(s, _)| s)
}

/// Thins a file's versions the way Google Docs does: every version from the last day, the newest
/// of each hour for a week, and the newest of each day before that. Named versions are always
/// kept. At most `keep` unnamed ones stay: past that, the one closest in time to the next newer
/// one goes first (the oldest of equals), so what is kept stays spread over the file's life.
/// Gaps are compared in whole minutes: save times are whole seconds, so a burst of saves that
/// happens to straddle a second boundary must not keep an older one over a newer one.
fn thin(folder: &Path, meta: &mut Map<String, Value>, keep: usize, now: i64) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut kept: Vec<(String, i64, bool)> = Vec::new();
    let mut gone = Vec::new();
    for n in version_names(folder).into_iter().rev() {
        let e = meta.get(&n);
        let saved = e.and_then(|e| e["saved"].as_i64()).or_else(|| fs::metadata(folder.join(&n)).ok().map(|m| mtime(&m).0)).unwrap_or(0);
        let named = e.and_then(|e| e["name"].as_str()).is_some_and(|s| !s.is_empty());
        let age = now - saved;
        let bucket = if age < DAY {
            None
        } else if age < 7 * DAY {
            Some((0, saved.div_euclid(3600)))
        } else {
            Some((1, saved.div_euclid(DAY)))
        };
        if named || bucket.is_none_or(|b| seen.insert(b)) {
            kept.push((n, saved, named));
        } else {
            gone.push(n);
        }
    }
    let keep = keep.max(1);
    while kept.iter().filter(|k| !k.2).count() > keep {
        let mut best: Option<(i64, usize)> = None;
        for (i, k) in kept.iter().enumerate().filter(|(_, k)| !k.2) {
            let gap = if i == 0 { i64::MAX } else { (kept[i - 1].1 - k.1).div_euclid(GAP_UNIT) };
            if best.is_none_or(|(g, _)| gap <= g) {
                best = Some((gap, i));
            }
        }
        let Some((_, i)) = best else { break };
        gone.push(kept.remove(i).0);
    }
    for n in &gone {
        fs::remove_file(folder.join(n))?;
    }
    let names: BTreeSet<String> = kept.into_iter().map(|k| k.0).collect();
    meta.retain(|k, _| k == "current" || k == COUNTER || names.contains(k));
    Ok(())
}

/// The key in a file's `versions.json` holding the next version number: a counter that only goes
/// up, so numbers are never reused after versions are thinned out, restored or the file moved.
const COUNTER: &str = "next";

/// Takes the next version number. First touch of a file made before numbering: its versions that
/// have no number get 1, 2, … oldest first (after the highest number any already has).
fn take_number(folder: &Path, meta: &mut Map<String, Value>) -> u64 {
    let mut next = meta.get(COUNTER).and_then(Value::as_u64).unwrap_or(0);
    let highest = meta.values().filter_map(|e| e.get("n").and_then(Value::as_u64)).max().unwrap_or(0);
    next = next.max(highest + 1).max(1);
    for name in version_names(folder) {
        if let Some(Value::Object(e)) = meta.get_mut(&name) {
            if e.get("n").is_none() {
                e.insert("n".into(), json!(next));
                next += 1;
            }
        } else if !meta.contains_key(&name) {
            meta.insert(name, json!({"n": next}));
            next += 1;
        }
    }
    meta.insert(COUNTER.into(), json!(next + 1));
    next
}

/// "YYYY-mm-dd HH:MM:SS" (UTC) for the activity log.
pub fn log_stamp() -> String {
    let (y, m, d, h, mi, s) = civil(now() as i64);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

/// "YYYY-mm-dd HH.MM" (UTC) for names people read (no ":" — Windows and SMB refuse it).
pub fn readable_stamp() -> String {
    let (y, m, d, h, mi, _) = civil(now() as i64);
    format!("{y:04}-{m:02}-{d:02} {h:02}.{mi:02}")
}

fn civil(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

/// A safe folder path relative to the store ("" = the top), or Bad.
pub fn rel(path: Option<&str>) -> R<String> {
    let s = path.unwrap_or("").replace('\\', "/");
    let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
    if parts.iter().any(|p| p.starts_with('.')) {
        return Err(Bad::new(400, "bad folder name"));
    }
    Ok(parts.join("/"))
}

pub fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() { name.to_string() } else { format!("{folder}/{name}") }
}

pub fn parent_of(relpath: &str) -> String {
    relpath.rsplit_once('/').map(|(a, _)| a.to_string()).unwrap_or_default()
}

pub fn base_of(relpath: &str) -> String {
    relpath.rsplit_once('/').map(|(_, b)| b).unwrap_or(relpath).to_string()
}

/// A safe display name: no folders, no characters Windows/SMB refuse, not empty.
pub fn clean_name(name: Option<&str>) -> String {
    let swapped: String = name
        .unwrap_or("")
        .trim()
        .chars()
        // Direction marks (U+202A–202E, U+2066–2069, U+200E/F) go: they can make "gpj.exe" look
        // like "exe.jpg".
        .filter(|c| !matches!(*c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}'))
        .map(|c| if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || (c as u32) < 0x20 { '-' } else { c })
        .collect();
    let s: String = swapped.trim_matches(|c| c == '.' || c == ' ').chars().take(150).collect();
    if s.is_empty() { "Untitled spreadsheet".to_string() } else { s }
}

/// Python's os.path.splitext for a bare name: leading dots don't start an extension.
fn splitext(name: &str) -> (String, String) {
    if let Some(d) = name.rfind('.')
        && name[..d].chars().any(|c| c != '.')
    {
        return (name[..d].to_string(), name[d..].to_string());
    }
    (name.to_string(), String::new())
}

fn is_sheet_ext(ext: &str) -> bool {
    EXTS.contains(&ext.to_lowercase().as_str())
}

pub fn split_name(name: &str) -> (String, String) {
    let (stem, ext) = splitext(name);
    if is_sheet_ext(&ext) { (stem, ext) } else { (name.to_string(), ".xlsx".to_string()) }
}

/// An existing file's (stem, extension), whatever its type: a moved, copied or restored file
/// keeps its own extension. Same as [`split_name`] for spreadsheets and documents.
pub fn split_existing(name: &str) -> (String, String) {
    splitext(name)
}

/// The (stem, extension) of the new name `typed` for an existing file `old`: a spreadsheet or
/// document as before ([`split_name`], keeping the old extension); any other file keeps its own
/// extension (none, if it had none), and typing that extension again doesn't double it.
fn renamed(old: &str, typed: &str) -> (String, String) {
    let (_, old_ext) = split_existing(old);
    if is_editable(old) {
        let (stem, ext) = split_name(typed);
        return (stem, if old_ext.is_empty() { ext } else { old_ext });
    }
    let lower = typed.to_lowercase();
    let stem = if !old_ext.is_empty() && lower.ends_with(&old_ext.to_lowercase()) && typed.len() > old_ext.len() {
        typed[..typed.len() - old_ext.len()].to_string()
    } else {
        typed.to_string()
    };
    (stem, old_ext)
}

/// An Undo's check (?from=<path>): the file is still where the action being undone left it.
fn still_at(path: &str, q: &HashMap<String, String>) -> R<()> {
    match qs(q, "from") {
        Some(from) if rel(Some(from))? != path => {
            Err(Bad::new(409, format!("\u{201c}{}\u{201d} has been moved or renamed since, so it was left where it is.", base_of(path))))
        }
        _ => Ok(()),
    }
}

fn taken(name: &str) -> Bad {
    Bad::new(409, format!("Something called \u{201c}{name}\u{201d} is there now, so it wasn't put back."))
}

fn numbered(stem: &str, n: u32, ext: &str) -> String {
    if n == 1 { format!("{stem}{ext}") } else { format!("{stem} ({n}){ext}") }
}

/// A picture's XMP sidecar (`photo.jpg.xmp`: its edits, photos/sidecar.rs) is part of it: it is
/// renamed, moved, copied, binned and restored with it, so edits are never lost and a later file
/// of the same name never inherits them. Only a plain file is carried, never a link.
fn sidecar_of(file: &Path) -> PathBuf {
    let mut s = file.as_os_str().to_owned();
    s.push(".xmp");
    PathBuf::from(s)
}

/// Moves (or copies) the picture `src` to `dst` (a name just claimed) together with its sidecar
/// `side` (if it is a plain file), all or nothing. The sidecar goes first and never replaces a
/// file already at `dst`'s sidecar name, which may be someone else's file in a shared folder: that
/// refuses with 409 like a taken name. If the picture can't follow, the sidecar goes back.
fn with_sidecar(src: &Path, side: &Path, dst: &Path, copy: bool) -> R<()> {
    let drop_claim = || {
        // The empty placeholder claim_name made, so a failed move leaves no stray file.
        if fs::symlink_metadata(dst).is_ok_and(|m| m.is_file() && m.len() == 0) {
            let _ = fs::remove_file(dst);
        }
    };
    let mut carried = None;
    if fs::symlink_metadata(side).is_ok_and(|m| m.is_file()) {
        let to = sidecar_of(dst);
        match if copy { copy_new(side, &to) } else { move_new(side, &to) } {
            Ok(()) => carried = Some(to),
            Err(e) => {
                drop_claim();
                return Err(if e.kind() == io::ErrorKind::AlreadyExists {
                    Bad::new(409, "a file named like its edits (.xmp) is already there")
                } else {
                    e.into()
                });
            }
        }
    }
    let moved = if copy { fs::copy(src, dst).map(|_| ()) } else { move_any(src, dst) };
    if let Err(e) = moved {
        if let Some(to) = carried {
            let _ = if copy { fs::remove_file(&to) } else { move_new(&to, side) };
        }
        drop_claim();
        return Err(e.into());
    }
    Ok(())
}

fn cpath(p: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

/// Renames a file, failing with `AlreadyExists` instead of replacing one at `dst`: atomically by
/// `renameat2(RENAME_NOREPLACE)`, else by link + unlink, else (another file system, or no hard
/// links) by an exclusive copy and removal.
fn move_new(src: &Path, dst: &Path) -> io::Result<()> {
    let (s, d) = (cpath(src)?, cpath(dst)?);
    // SAFETY: both are valid NUL-terminated paths; renameat2 only reads them.
    let r = unsafe { libc::renameat2(libc::AT_FDCWD, s.as_ptr(), libc::AT_FDCWD, d.as_ptr(), libc::RENAME_NOREPLACE) };
    if r == 0 {
        return Ok(());
    }
    let e = io::Error::last_os_error();
    if !matches!(e.raw_os_error(), Some(libc::EINVAL | libc::ENOSYS | libc::EXDEV)) {
        return Err(e);
    }
    match fs::hard_link(src, dst) {
        Ok(()) => return fs::remove_file(src),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(e),
        Err(_) => {}
    }
    copy_new(src, dst)?;
    fs::remove_file(src)
}

/// Copies a file to `dst`, which must not exist yet (not even as a link: never followed), keeping
/// its permissions and modified time.
fn copy_new(src: &Path, dst: &Path) -> io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut from = fs::File::options().read(true).custom_flags(libc::O_NOFOLLOW).open(src)?;
    let m = from.metadata()?;
    let mut to = fs::File::options().write(true).create_new(true).custom_flags(libc::O_NOFOLLOW).mode(m.permissions().mode() & 0o777).open(dst)?;
    let done = io::copy(&mut from, &mut to).and_then(|_| to.sync_all());
    if let Err(e) = done {
        let _ = fs::remove_file(dst);
        return Err(e);
    }
    if let Ok(t) = m.modified() {
        let _ = to.set_modified(t);
    }
    Ok(())
}

/// Where the sidecar of `name` in a bin is kept (a hidden folder: not listed as its own item).
fn binned_sidecar(bin: &Path, name: &str) -> PathBuf {
    bin.join(".sidecars").join(format!("{name}.xmp"))
}

/// Moves a file or folder, copying when it has to cross file systems.
fn move_any(src: &Path, dst: &Path) -> io::Result<()> {
    match fs::rename(src, dst) {
        Err(e) if e.raw_os_error() == Some(18) => {
            // EXDEV
            copy_tree(src, dst)?;
            if src.is_dir() { fs::remove_dir_all(src) } else { fs::remove_file(src) }
        }
        other => other,
    }
}

fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    if src.is_dir() {
        fs::create_dir(dst)?;
        for e in fs::read_dir(src)? {
            let e = e?;
            copy_tree(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        // Like Python's shutil.copy2: the copy keeps the original's modified time.
        fs::copy(src, dst)?;
        if let Ok(t) = fs::metadata(src).and_then(|m| m.modified()) {
            let _ = fs::File::options().write(true).open(dst).and_then(|f| f.set_modified(t));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- the store

pub struct Store {
    pub root: PathBuf,
    pub keep: usize,
    /// A folder per person at the top (sign-in on); false = one shared tree (sign-in off).
    pub spaces: bool,
}

/// The first part of a path: whose space it's in (spaces on).
/// Bytes of the regular files below `dir` (no links followed; bounded depth).
fn dir_bytes(dir: &Path, depth: u32) -> u64 {
    let Ok(rd) = fs::read_dir(dir) else { return 0 };
    let mut n = 0;
    for e in rd.flatten() {
        let Ok(m) = fs::symlink_metadata(e.path()) else { continue };
        if m.is_file() {
            n += m.len();
        } else if m.is_dir() && depth < 64 {
            n += dir_bytes(&e.path(), depth + 1);
        }
    }
    n
}

/// Free bytes on the disk holding `path` (None if it can't be told).
fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: c is a valid NUL-terminated path and st a valid out-pointer.
    (unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0).then(|| st.f_bavail as u64 * st.f_frsize as u64)
}

pub fn space_of(relpath: &str) -> &str {
    relpath.split('/').next().unwrap_or("")
}

type Index = BTreeMap<String, String>;

impl Store {
    pub fn meta(&self) -> PathBuf {
        self.root.join(".starsheets")
    }
    fn index_path(&self) -> PathBuf {
        self.meta().join("index.json")
    }
    fn bin_index_path(&self) -> PathBuf {
        self.meta().join("bin.json")
    }
    fn versions(&self) -> PathBuf {
        self.meta().join("versions")
    }
    fn folders_path(&self) -> PathBuf {
        self.meta().join("folders.json")
    }
    /// The bin for `space` ("" = the shared one, without spaces).
    fn bin(&self, space: &str) -> PathBuf {
        if self.spaces && !space.is_empty() { self.full(space).join(".bin") } else { self.root.join(".bin") }
    }
    fn bin_key(&self, space: &str, name: &str) -> String {
        if self.spaces && !space.is_empty() { format!("{space}/{name}") } else { name.to_string() }
    }

    pub fn full(&self, relpath: &str) -> PathBuf {
        let mut p = self.root.clone();
        if !relpath.is_empty() {
            p.extend(relpath.split('/'));
        }
        p
    }

    pub fn load_json(&self, path: &Path) -> Map<String, Value> {
        fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| if let Value::Object(m) = v { Some(m) } else { None })
            .unwrap_or_default()
    }

    pub fn save_json(&self, path: &Path, data: &Value) -> io::Result<()> {
        fs::create_dir_all(self.meta())?;
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(data).unwrap_or_default())?;
        fs::rename(&tmp, path)
    }

    fn load_index(&self) -> Index {
        self.load_json(&self.index_path()).into_iter().filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string()))).collect()
    }

    fn save_index(&self, index: &Index) -> io::Result<()> {
        self.save_json(&self.index_path(), &json!(index))
    }

    fn load_folders(&self) -> Index {
        self.load_json(&self.folders_path()).into_iter().filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string()))).collect()
    }

    fn save_folders(&self, folders: &Index) -> io::Result<()> {
        self.save_json(&self.folders_path(), &json!(folders))
    }

    /// Folder ids ({id: path}), in line with the folder tree, as `synced_index` does for files.
    pub fn synced_folders(&self) -> io::Result<Index> {
        let mut ids = self.load_folders();
        let (_, folders) = self.walk();
        let present: BTreeSet<String> = folders.into_iter().collect();
        let before = ids.len();
        ids.retain(|_, path| present.contains(path));
        let mut changed = ids.len() != before;
        let known: BTreeSet<String> = ids.values().cloned().collect();
        for path in present.difference(&known) {
            ids.insert(random_hex(8), path.clone());
            changed = true;
        }
        if changed {
            self.save_folders(&ids)?;
        }
        Ok(ids)
    }

    /// After a folder moved from `old` to `new`: the ids inside follow it.
    fn moved_prefix(&self, index: &mut Index, old: &str, new: &str) -> io::Result<()> {
        let follow = |v: &mut String| {
            if v == old {
                *v = new.to_string();
            } else if let Some(tail) = v.strip_prefix(&format!("{old}/")) {
                *v = format!("{new}/{tail}");
            }
        };
        index.values_mut().for_each(follow);
        self.save_index(index)?;
        let mut folders = self.load_folders();
        folders.values_mut().for_each(follow);
        self.save_folders(&folders)
    }

    /// Spreadsheet files and folders as paths relative to the store, skipping hidden ones.
    /// Same order as Python's os.walk with sorted folders.
    pub fn walk(&self) -> (Vec<String>, Vec<String>) {
        let (mut files, mut folders) = (Vec::new(), Vec::new());
        self.walk_dir("", &mut files, &mut folders);
        (files, folders)
    }

    fn walk_dir(&self, here: &str, files: &mut Vec<String>, folders: &mut Vec<String>) {
        let Ok(rd) = fs::read_dir(self.full(here)) else { return };
        let mut dirs = Vec::new();
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            // Symbolic links are never followed, to files or folders: one made over a file share
            // could lead to StarSheets' own data (.starsheets: sign-ins, passwords) or into
            // someone else's folder, past the sharing checks of the folder it sits in.
            let Ok(meta) = fs::symlink_metadata(e.path()) else { continue };
            if meta.is_dir() {
                dirs.push(name);
            } else if meta.is_file() {
                // Every kind of file (not sockets or pipes, which can't be read like one).
                files.push(join(here, &name));
            }
        }
        dirs.sort();
        folders.extend(dirs.iter().map(|d| join(here, d)));
        for d in dirs {
            self.walk_dir(&join(here, &d), files, folders);
        }
    }

    /// Whether no part of `relpath` (below the store's own folder) is a symbolic link or hidden.
    /// Missing parts are fine (checked by whoever needs them to exist).
    pub fn no_links(&self, relpath: &str) -> bool {
        if !self.spaces && crate::mounts::private_path(relpath) {
            return false;
        }
        let mut p = self.root.clone();
        for part in relpath.split('/').filter(|x| !x.is_empty()) {
            if part.starts_with('.') {
                return false;
            }
            p.push(part);
            if fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
                return false;
            }
        }
        true
    }

    /// The file at `relpath` on disk, if it is a plain file reached without any symbolic link
    /// (never a pipe, never a hidden one). Every file read or written by id is checked with this
    /// ([`Self::path_of`]); reads open it with [`Self::open_safe`].
    pub fn safe_file(&self, relpath: &str) -> Option<PathBuf> {
        if relpath.split('/').any(|p| p.is_empty() || p.starts_with('.')) || !self.no_links(relpath) {
            return None;
        }
        let full = self.full(relpath);
        fs::symlink_metadata(&full).ok()?.is_file().then_some(full)
    }

    /// Opens the file at `relpath` for reading, as [`Self::safe_file`] allows, and makes sure the
    /// file opened is that one: the last part is opened without following a link, and the opened
    /// file's real path (or, without /proc, its device and inode) must be the path checked, so a
    /// link swapped in between the check and the open is refused.
    pub fn open_safe(&self, relpath: &str) -> Option<fs::File> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        use std::os::unix::io::AsRawFd;
        let full = self.safe_file(relpath)?;
        let f = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&full).ok()?;
        let m = f.metadata().ok()?;
        if !m.is_file() {
            return None;
        }
        match fs::read_link(format!("/proc/self/fd/{}", f.as_raw_fd())) {
            Ok(real) => (real == fs::canonicalize(&self.root).ok()?.join(relpath)).then_some(f),
            Err(_) => {
                let again = fs::symlink_metadata(self.safe_file(relpath)?).ok()?;
                ((again.dev(), again.ino()) == (m.dev(), m.ino())).then_some(f)
            }
        }
    }

    /// Keeps a copy of the file at `relpath` in its owner's bin (restorable, under its own name),
    /// before it is replaced by an upload: files without versions don't lose what they held.
    pub fn copy_to_bin(&self, relpath: &str) -> io::Result<()> {
        use std::io::Read;
        let space = if self.spaces { space_of(relpath) } else { "" };
        let bin = self.bin(space);
        fs::create_dir_all(&bin)?;
        let name = base_of(relpath);
        let (stem, ext) = splitext(&name);
        let mut target = name;
        let mut n = 1;
        while bin.join(&target).exists() {
            n += 1;
            target = numbered(&stem, n, &ext);
        }
        let mut data = Vec::new();
        self.open_safe(relpath).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))?.read_to_end(&mut data)?;
        fs::write(bin.join(&target), data)?;
        let mut bins = self.load_json(&self.bin_index_path());
        bins.insert(self.bin_key(space, &target), json!({"original": relpath, "deleted": now() as i64, "files": {}, "folders": {}}));
        self.save_json(&self.bin_index_path(), &Value::Object(bins))
    }

    /// Where file `id` is now. The index is trusted while the file is still at the path it
    /// gives, which costs one file check; only when it isn't (moved or binned outside
    /// StarSheets) is the whole folder tree listed again ([`Self::synced_index`]). Listing a big
    /// tree over a network share every few seconds for each open file held up everyone else.
    pub fn path_of(&self, id: &str) -> io::Result<Option<String>> {
        if let Some(path) = self.load_index().get(id)
            && self.safe_file(path).is_some()
        {
            return Ok(Some(path.clone()));
        }
        Ok(self.synced_index()?.get(id).filter(|p| self.safe_file(p).is_some()).cloned())
    }

    /// The index ({id: path}), in line with the folder tree: new files get ids, gone ones drop.
    pub fn synced_index(&self) -> io::Result<Index> {
        let mut index = self.load_index();
        let (files, _) = self.walk();
        let present: BTreeSet<String> = files.into_iter().collect();
        let before = index.len();
        index.retain(|_, path| present.contains(path));
        let mut changed = index.len() != before;
        let known: BTreeSet<String> = index.values().cloned().collect();
        for path in present.difference(&known) {
            index.insert(random_hex(8), path.clone());
            changed = true;
        }
        if changed {
            self.save_index(&index)?;
        }
        Ok(index)
    }

    pub fn version_of(&self, path: &Path) -> io::Result<String> {
        Ok(Self::file_version(&fs::metadata(path)?))
    }

    pub fn file_version(m: &fs::Metadata) -> String {
        format!("{:x}-{:x}", mtime(m).1, m.len())
    }

    /// The name of the newest kept version of a file ("" when none): changes whenever someone's
    /// save keeps a new one, which the file's own timestamp can't tell (it ticks in milliseconds,
    /// so two saves of equal length can share a version string).
    pub fn newest_kept(&self, fid: &str) -> String {
        version_names(&self.versions().join(fid)).pop().unwrap_or_default()
    }

    /// Stages the write beside its destination so replacing it also works on bind mounts.
    pub fn write_file(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        use std::io::Write as _;

        let tmp = path.with_file_name(format!(".{}.tmp", random_hex(6)));
        let mut out = fs::File::options().write(true).create_new(true).open(&tmp)?;
        let written = out.write_all(data);
        drop(out);
        let result = written.and_then(|()| fs::rename(&tmp, path));
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    /// Copies the file as it is now into its versions folder ([`thin`] says which are kept). The
    /// copy remembers when that content was saved, who saved it (`spaces::last_change`, noted
    /// after every save) and the name it was given while it was the current version.
    pub fn keep_version(&self, fid: &str, path: &Path) -> io::Result<()> {
        let saved = match fs::metadata(path) {
            Ok(m) if m.len() > 0 => mtime(&m).0,
            _ => return Ok(()),
        };
        let folder = self.versions().join(fid);
        fs::create_dir_all(&folder)?;
        // Names sort in the order the versions were made, also several within one millisecond:
        // a name that wouldn't sort after the newest one takes the newest name plus "z".
        let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_millis()).unwrap_or(0);
        let existing = version_names(&folder);
        let mut stem = format!("{}-{ms:03}0", stamp());
        if let Some(last) = existing.last().map(|l| version_stem(l))
            && stem.as_str() <= last
        {
            stem = format!("{last}z");
        }
        let lower = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let (_, ext) = splitext(&lower);
        let file = format!("{stem}{}", if ext.is_empty() { ".xlsx" } else { &ext });
        let mut meta = self.load_json(&folder.join(VERSIONS_META));
        // Numbered before the new copy exists, so older unnumbered versions come first.
        let number = take_number(&folder, &mut meta);
        fs::copy(path, folder.join(&file))?;
        let current = match meta.remove("current") {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        let mut entry = Map::new();
        entry.insert("n".into(), json!(number));
        entry.insert("saved".into(), json!(saved));
        entry.insert("author".into(), json!(crate::spaces::last_change(self, fid).map(|c| c.0).unwrap_or_default()));
        for key in ["name", "restored_from"] {
            if let Some(v) = current.get(key).filter(|v| !v.is_null()) {
                entry.insert(key.into(), v.clone());
            }
        }
        meta.insert(file, Value::Object(entry));
        thin(&folder, &mut meta, self.keep, now() as i64)?;
        self.save_json(&folder.join(VERSIONS_META), &Value::Object(meta))
    }

    /// A file's earlier versions, newest first: id, when that content was saved, who saved it,
    /// the name it was given, and its size.
    fn version_list(&self, fid: &str) -> io::Result<Vec<Value>> {
        let folder = self.versions().join(fid);
        let meta = self.load_json(&folder.join(VERSIONS_META));
        let mut out = Vec::new();
        for v in version_names(&folder).iter().rev() {
            let m = fs::metadata(folder.join(v))?;
            let e = meta.get(v);
            // Copies made before versions were described: the copy's own time.
            let saved = e.and_then(|e| e["saved"].as_i64()).unwrap_or(mtime(&m).0);
            out.push(json!({
                "v": v, "number": e.and_then(|e| e["n"].as_u64()), "modified": saved, "saved": saved, "size": m.len(),
                "author": e.and_then(|e| e["author"].as_str()).unwrap_or(""),
                "name": e.and_then(|e| e["name"].as_str()).unwrap_or(""),
                "restored_from": e.and_then(|e| e.get("restored_from")).cloned().unwrap_or(Value::Null),
            }));
        }
        Ok(out)
    }

    /// Sets (an empty name clears) the name of version `v` of a file, or of the file as it is now
    /// with `current` (Docs' "Name current version"). Named versions are never thinned out.
    fn name_version(&self, fid: &str, v: &str, name: &str) -> R<()> {
        let folder = self.versions().join(fid);
        if v != "current" && !version_names(&folder).iter().any(|n| n == v) {
            return Err(Bad::new(404, "no such version"));
        }
        let name: String = name.trim().chars().filter(|c| !c.is_control()).take(MAX_VERSION_NAME).collect();
        fs::create_dir_all(&folder)?;
        let mut meta = self.load_json(&folder.join(VERSIONS_META));
        if let Value::Object(e) = meta.entry(v.to_string()).or_insert_with(|| json!({})) {
            if name.is_empty() {
                e.remove("name");
            } else {
                e.insert("name".into(), json!(name));
            }
        }
        self.save_json(&folder.join(VERSIONS_META), &Value::Object(meta))?;
        Ok(())
    }

    /// Moves a file or folder into the bin (its owner's, with spaces), remembering where it was
    /// and the ids of everything in it.
    fn to_bin(&self, relpath: &str, index: &mut Index) -> io::Result<String> {
        let space = if self.spaces { space_of(relpath) } else { "" };
        let bin = self.bin(space);
        fs::create_dir_all(&bin)?;
        let name = base_of(relpath);
        let (stem, ext) = if self.full(relpath).is_file() { splitext(&name) } else { (name.clone(), String::new()) };
        // Its own name; only a clash with something already in the bin adds " (2)", " (3)"…
        let mut target = name;
        let mut n = 1;
        while bin.join(&target).exists() {
            n += 1;
            target = numbered(&stem, n, &ext);
        }
        let under = |path: &str| -> Option<String> {
            if path == relpath { Some(String::new()) } else { path.strip_prefix(&format!("{relpath}/")).map(str::to_string) }
        };
        let file_ids: Map<String, Value> = index.iter().filter_map(|(id, p)| Some((under(p)?, json!(id)))).collect();
        let mut folders = self.load_folders();
        let folder_ids: Map<String, Value> = folders.iter().filter_map(|(id, p)| Some((under(p)?, json!(id)))).collect();
        let was_file = self.full(relpath).is_file();
        move_any(&self.full(relpath), &bin.join(&target))?;
        let side = sidecar_of(&self.full(relpath));
        if was_file && fs::symlink_metadata(&side).is_ok_and(|m| m.is_file()) {
            fs::create_dir_all(bin.join(".sidecars"))?;
            move_any(&side, &binned_sidecar(&bin, &target))?;
        }
        index.retain(|_, p| under(p).is_none());
        self.save_index(index)?;
        folders.retain(|_, p| under(p).is_none());
        self.save_folders(&folders)?;
        let mut bins = self.load_json(&self.bin_index_path());
        bins.insert(self.bin_key(space, &target), json!({"original": relpath, "deleted": now() as i64, "files": file_ids, "folders": folder_ids}));
        self.save_json(&self.bin_index_path(), &Value::Object(bins))?;
        Ok(target)
    }

    /// The bin's name for what was just binned, and whose bin it is: an Undo puts it back with
    /// `POST /api/bin/restore?name=…&expect=<original>&exact=1`.
    fn binned(&self, relpath: &str, name: String) -> Value {
        json!({"ok": true, "bin": name, "original": relpath, "space": if self.spaces { space_of(relpath) } else { "" }})
    }

    /// A file in `folder` (not below it) whose bytes are exactly `body`: sizes first, then the bytes.
    fn same_bytes_in(&self, folder: &str, body: &[u8]) -> Option<String> {
        for e in fs::read_dir(self.full(folder)).ok()?.flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_file() && m.len() == body.len() as u64 && fs::read(e.path()).is_ok_and(|b| b == body) {
                return Some(join(folder, &e.file_name().to_string_lossy()));
            }
        }
        None
    }

    /// Reserves a free file name in `folder` (stem.ext, stem (2).ext, …) by creating it exclusively.
    pub fn claim_name(&self, folder: &str, stem: &str, ext: &str, own: Option<&str>) -> R<String> {
        use std::os::unix::fs::OpenOptionsExt;
        for n in 1..10000 {
            let name = numbered(stem, n, ext);
            if own == Some(join(folder, &name).as_str()) {
                return Ok(name);
            }
            // A left-over sidecar (or someone's file of that name) would be taken for this
            // picture's edits, or block carrying its own: skip the name like a taken one.
            if fs::symlink_metadata(sidecar_of(&self.full(folder).join(&name))).is_ok() {
                continue;
            }
            match fs::OpenOptions::new().write(true).create_new(true).mode(0o664).open(self.full(folder).join(&name)) {
                Ok(_) => return Ok(name),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(Bad::new(409, "no free name"))
    }

    /// Creates a free folder name in `parent` ("Name", "Name (2)", …).
    fn claim_folder(&self, parent: &str, name: Option<&str>) -> R<String> {
        let name = if name.unwrap_or("").is_empty() { "New folder".to_string() } else { clean_name(name) };
        if self.spaces && join(parent, &name) == crate::mounts::account_mount_dir(parent) && !parent.contains('/') {
            return Err(Bad::new(403, "The deployment manages this mount folder."));
        }
        for n in 1..10000 {
            let candidate = numbered(&name, n, "");
            match fs::create_dir(self.full(parent).join(&candidate)) {
                Ok(()) => return Ok(join(parent, &candidate)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(Bad::new(409, "no free name"))
    }

    fn free_folder_name(&self, parent: &str, name: &str) -> R<String> {
        if self.spaces && join(parent, name) == crate::mounts::account_mount_dir(parent) && !parent.contains('/') {
            return Err(Bad::new(403, "The deployment manages this mount folder."));
        }
        (1..10000).map(|n| numbered(name, n, "")).find(|c| !self.full(parent).join(c).exists()).ok_or_else(|| Bad::new(409, "no free name"))
    }

    /// Exactly stem.ext in `folder` (reserved by creating it), or 409 when that name is taken.
    fn claim_exact(&self, folder: &str, stem: &str, ext: &str, own: Option<&str>) -> R<String> {
        use std::os::unix::fs::OpenOptionsExt;
        let name = numbered(stem, 1, ext);
        if own == Some(join(folder, &name).as_str()) {
            return Ok(name);
        }
        match fs::OpenOptions::new().write(true).create_new(true).mode(0o664).open(self.full(folder).join(&name)) {
            Ok(_) => Ok(name),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(taken(&name)),
            Err(e) => Err(e.into()),
        }
    }

    /// `name` in `parent` if nothing there has it, else 409.
    fn exact_folder_name(&self, parent: &str, name: &str) -> R<String> {
        if self.spaces && join(parent, name) == crate::mounts::account_mount_dir(parent) && !parent.contains('/') {
            return Err(Bad::new(403, "The deployment manages this mount folder."));
        }
        if fs::symlink_metadata(self.full(parent).join(name)).is_ok() { Err(taken(name)) } else { Ok(name.to_string()) }
    }

    fn need_folder(&self, folder: &str) -> R<()> {
        // Never through a symbolic link (walk() doesn't list them either).
        if self.no_links(folder) && self.full(folder).is_dir() { Ok(()) } else { Err(Bad::new(404, "no such folder")) }
    }

    // ------------------------------------------------------------ the API

    /// Answers one /api/ request (after the login check). `p` = path parts after "api".
    pub fn route(&self, method: &str, p: &[String], q: &HashMap<String, String>, if_match: Option<&str>, body: &[u8]) -> R<Reply> {
        fs::create_dir_all(self.meta())?;
        let mut index = self.synced_index()?;
        let head = p.first().map(String::as_str);
        match head {
            Some("files") => self.route_files(method, p, q, if_match, body, &mut index),
            Some("folders") => self.route_folders(method, p, q, &mut index),
            Some("bin") => self.route_bin(method, p, q, &mut index),
            Some("usage") if p.len() == 1 && method == "GET" => self.storage(q),
            _ => Err(Bad::new(404, "not found")),
        }
    }

    /// GET /api/usage: bytes used by the person's own files (their space, hidden folders and the
    /// bin left out), what their bin holds, and the free space of the disk. There is no quota.
    fn storage(&self, q: &HashMap<String, String>) -> R<Reply> {
        let space = if self.spaces { rel(qs(q, "space"))? } else { String::new() };
        if self.spaces && (space.is_empty() || space.contains('/')) {
            return Err(Bad::new(400, "whose storage?"));
        }
        let (mut files, mut folders) = (Vec::new(), Vec::new());
        self.walk_dir(&space, &mut files, &mut folders);
        let used: u64 = files.iter().filter_map(|f| fs::symlink_metadata(self.full(f)).ok()).map(|m| m.len()).sum();
        let bin_bytes = dir_bytes(&self.bin(&space), 0);
        ok(json!({"used": used, "files": files.len(), "bin": bin_bytes, "free": free_bytes(&self.root)}))
    }

    fn route_files(
        &self,
        method: &str,
        p: &[String],
        q: &HashMap<String, String>,
        if_match: Option<&str>,
        body: &[u8],
        index: &mut Index,
    ) -> R<Reply> {
        if p.len() == 1 && method == "GET" {
            let (mut files, mut folders) = self.walk();
            if !self.spaces {
                files.retain(|path| !crate::mounts::private_path(path));
                folders.retain(|path| !crate::mounts::private_path(path));
            }
            let by_path: HashMap<&String, &String> = index.iter().map(|(k, v)| (v, k)).collect();
            let mut out = Vec::new();
            for path in &files {
                let m = fs::metadata(self.full(path))?;
                let (secs, ns) = mtime(&m);
                out.push(json!({
                    "id": by_path.get(path).map(|s| s.as_str()).unwrap_or(""),
                    "name": base_of(path), "folder": parent_of(path), "modified": secs,
                    "size": m.len(), "version": format!("{ns:x}-{:x}", m.len()),
                }));
            }
            if self.spaces {
                let by_path: HashMap<String, String> = self.synced_folders()?.into_iter().map(|(id, path)| (path, id)).collect();
                return ok(json!({"files": out, "folders": folders, "folder_ids": by_path}));
            }
            return ok(json!({"files": out, "folders": folders}));
        }
        if p.len() == 1 && method == "POST" {
            let folder = rel(qs(q, "folder"))?;
            self.need_folder(&folder)?;
            // ?skip_dup=1 (phone uploads): a file already in this folder with exactly these bytes
            // is not saved again; the reply says which one it is ("duplicate", no new "id").
            if qs(q, "skip_dup") == Some("1")
                && let Some(same) = self.same_bytes_in(&folder, body)
            {
                let id = index.iter().find(|(_, p)| **p == same).map(|(i, _)| i.clone()).unwrap_or_default();
                return Ok(Reply::Json(json!({"duplicate": true, "existing": id, "name": base_of(&same), "folder": folder}), 200));
            }
            // ?as_is=1: a file uploaded to the Drive keeps its name and type, whatever it is
            // (photo.jpg stays photo.jpg); otherwise a name that isn't a spreadsheet or document
            // gets .xlsx, as before.
            let cleaned = clean_name(qs(q, "name"));
            let (stem, ext) = if qs(q, "as_is") == Some("1") { split_existing(&cleaned) } else { split_name(&cleaned) };
            let name = self.claim_name(&folder, &stem, &ext, None)?;
            let path = join(&folder, &name);
            self.write_file(&self.full(&path), body)?;
            let fid = random_hex(8);
            index.insert(fid.clone(), path.clone());
            self.save_index(index)?;
            let version = self.version_of(&self.full(&path))?;
            return Ok(Reply::Json(json!({"id": fid, "name": name, "folder": folder, "version": version}), 201));
        }
        if p.len() < 2 {
            return Err(Bad::new(404, "not found"));
        }
        let fid = p[1].as_str();
        let path = index.get(fid).cloned().ok_or_else(|| Bad::new(404, "no such file"))?;
        let fpath = self.safe_file(&path).ok_or_else(|| Bad::new(404, "no such file"))?;
        let rest: Vec<&str> = p[2..].iter().map(String::as_str).collect();
        match (rest.as_slice(), method) {
            ([], "GET") => {
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut self.open_safe(&path).ok_or_else(|| Bad::new(404, "no such file"))?, &mut data)?;
                let modified = mtime(&fs::metadata(&fpath)?).0;
                Ok(Reply::File { data, name: base_of(&path), version: self.version_of(&fpath)?, folder: parent_of(&path), modified })
            }
            ([], "PUT") => {
                if let Some(want) = if_match.filter(|w| !w.is_empty())
                    && want != self.version_of(&fpath)?
                {
                    return Ok(Reply::Json(json!({"error": "changed elsewhere", "name": base_of(&path)}), 409));
                }
                // Every file keeps its earlier versions (Drive's "Manage versions"); only a big file the
                // apps don't edit goes to the bin instead, so versions can't fill the disk.
                if is_editable(&path) || fs::metadata(&fpath).map_or(0, |m| m.len()) <= MAX_VERSIONED_OTHER {
                    self.keep_version(fid, &fpath)?;
                } else {
                    self.copy_to_bin(&path)?;
                }
                self.write_file(&fpath, body)?;
                ok(json!({"version": self.version_of(&fpath)?, "name": base_of(&path), "folder": parent_of(&path)}))
            }
            ([], "DELETE") => {
                let name = self.to_bin(&path, index)?;
                ok(self.binned(&path, name))
            }
            // A file made from a template (or blank) that its tab closed without a single change:
            // gone for good, not in the bin. Only when it is still exactly what was created
            // (?version=<the one it was created with>, ?name=<its name then>, no saved version
            // since); anything else, a rename or a save, keeps it.
            (["discard"], "POST") => {
                let untouched = qs(q, "version") == Some(self.version_of(&fpath)?.as_str())
                    && qs(q, "name") == Some(base_of(&path).as_str())
                    && self.newest_kept(fid).is_empty();
                if !untouched {
                    return ok(json!({"discarded": false}));
                }
                let name = self.to_bin(&path, index)?;
                let space = if self.spaces { space_of(&path) } else { "" };
                let mut bins = self.load_json(&self.bin_index_path());
                self.forget(&self.bin(space), space, &name, &mut bins)?;
                self.save_json(&self.bin_index_path(), &Value::Object(bins))?;
                ok(json!({"discarded": true}))
            }
            // ?from=<path it should be at> (an Undo: refused when it was moved or renamed since);
            // ?exact=1: that name or nothing, never "name (2)".
            (["rename"], "POST") => {
                still_at(&path, q)?;
                let folder = parent_of(&path);
                let (stem, ext) = renamed(&base_of(&path), &clean_name(qs(q, "name")));
                let name = if qs(q, "exact") == Some("1") {
                    self.claim_exact(&folder, &stem, &ext, Some(&path))?
                } else {
                    self.claim_name(&folder, &stem, &ext, Some(&path))?
                };
                let new = join(&folder, &name);
                if new != path {
                    with_sidecar(&fpath, &sidecar_of(&fpath), &self.full(&new), false)?;
                    index.insert(fid.to_string(), new);
                    self.save_index(index)?;
                }
                ok(json!({"name": name, "id": fid}))
            }
            // ?from= and ?as=<name> (exactly that name there, or refused): an Undo moving it back.
            (["move"], "POST") => {
                still_at(&path, q)?;
                let folder = rel(qs(q, "folder"))?;
                self.need_folder(&folder)?;
                if folder == parent_of(&path) {
                    return ok(json!({"name": base_of(&path), "id": fid}));
                }
                let name = match qs(q, "as") {
                    // Named as a rename would be: the file keeps its own extension, so a move
                    // can't turn report.xlsx into report.html.
                    Some(want) => {
                        let (stem, ext) = renamed(&base_of(&path), &clean_name(Some(want)));
                        self.claim_exact(&folder, &stem, &ext, None)?
                    }
                    None => {
                        let (stem, ext) = split_existing(&base_of(&path));
                        self.claim_name(&folder, &stem, &ext, None)?
                    }
                };
                with_sidecar(&fpath, &sidecar_of(&fpath), &self.full(&folder).join(&name), false)?;
                index.insert(fid.to_string(), join(&folder, &name));
                self.save_index(index)?;
                ok(json!({"name": name, "id": fid}))
            }
            (["copy"], "POST") => {
                let parent = parent_of(&path);
                let folder = rel(Some(qs(q, "folder").unwrap_or(&parent)))?;
                self.need_folder(&folder)?;
                let (mut stem, ext) = split_existing(&base_of(&path));
                if folder == parent {
                    stem = format!("Copy of {stem}");
                }
                let name = self.claim_name(&folder, &stem, &ext, None)?;
                with_sidecar(&fpath, &sidecar_of(&fpath), &self.full(&folder).join(&name), true)?;
                let new_id = random_hex(8);
                index.insert(new_id.clone(), join(&folder, &name));
                self.save_index(index)?;
                ok(json!({"id": new_id, "name": name}))
            }
            (["versions"], "GET") => ok(Value::Array(self.version_list(fid)?)),
            (["versions", v], "GET") => {
                let vdir = self.versions().join(fid);
                if !version_names(&vdir).iter().any(|n| n == v) {
                    return Err(Bad::new(404, "no such version"));
                }
                let data = fs::read(vdir.join(v))?;
                Ok(Reply::File { data, name: base_of(&path), version: String::new(), folder: String::new(), modified: 0 })
            }
            // Version history in one reply: the file as it is now, and its earlier versions.
            (["history"], "GET") => {
                let m = fs::metadata(&fpath)?;
                let meta = self.load_json(&self.versions().join(fid).join(VERSIONS_META));
                let cur = meta.get("current");
                ok(json!({
                    "current": {
                        "saved": mtime(&m).0, "size": m.len(), "version": self.version_of(&fpath)?,
                        "author": crate::spaces::last_change(self, fid).map(|c| c.0).unwrap_or_default(),
                        "name": cur.and_then(|c| c["name"].as_str()).unwrap_or(""),
                        "restored_from": cur.and_then(|c| c.get("restored_from")).cloned().unwrap_or(Value::Null),
                    },
                    "versions": self.version_list(fid)?,
                }))
            }
            (["versions", v, "name"], "POST") => {
                self.name_version(fid, v, qs(q, "name").unwrap_or(""))?;
                ok(json!({"ok": true}))
            }
            // Puts an earlier version back as a new version: the current one is kept first, so
            // nothing is lost and a restore can itself be undone.
            (["versions", v, "restore"], "POST") => {
                let vdir = self.versions().join(fid);
                if !version_names(&vdir).iter().any(|n| n == v) {
                    return Err(Bad::new(404, "no such version"));
                }
                let data = fs::read(vdir.join(v))?;
                let from = self.load_json(&vdir.join(VERSIONS_META)).get(*v).and_then(|e| e["saved"].as_i64());
                self.keep_version(fid, &fpath)?;
                self.write_file(&fpath, &data)?;
                let mut meta = self.load_json(&vdir.join(VERSIONS_META));
                meta.insert("current".into(), json!({"restored_from": from}));
                self.save_json(&vdir.join(VERSIONS_META), &Value::Object(meta))?;
                ok(json!({"version": self.version_of(&fpath)?, "name": base_of(&path), "folder": parent_of(&path)}))
            }
            _ => Err(Bad::new(404, "not found")),
        }
    }

    fn route_folders(&self, method: &str, p: &[String], q: &HashMap<String, String>, index: &mut Index) -> R<Reply> {
        let action = p.get(1).map(String::as_str).unwrap_or("");
        if action.is_empty() && method == "POST" {
            let parent = rel(qs(q, "path"))?;
            self.need_folder(&parent)?;
            if self.spaces && parent.is_empty() {
                return Err(Bad::new(400, "Folders go inside someone's own folder."));
            }
            return Ok(Reply::Json(json!({"path": self.claim_folder(&parent, qs(q, "name"))?}), 201));
        }
        let path = rel(qs(q, "path"))?;
        if path.is_empty() {
            return Err(Bad::new(400, "the top folder can't be changed"));
        }
        self.need_folder(&path)?;
        if self.spaces && !path.contains('/') {
            return Err(Bad::new(400, "A person's own folder can't be changed."));
        }
        if (crate::mounts::private_path(&path) && path.split('/').count() <= 3)
            || (!path.contains('/') && self.full(&crate::mounts::account_mount_dir(&path)).symlink_metadata().is_ok())
        {
            return Err(Bad::new(403, "The deployment manages this mount folder."));
        }
        let folder_ids = self.synced_folders()?;
        let fid = folder_ids.iter().find(|(_, p)| **p == path).map(|(k, _)| k.clone()).unwrap_or_default();
        // ?id=<the folder's id> (an Undo): the folder at `path` must still be the one the action
        // being undone moved or renamed, not another that has taken its name since.
        if qs(q, "id").is_some_and(|want| want != fid) {
            return Err(Bad::new(409, format!("\u{201c}{}\u{201d} has been moved or renamed since, so it was left where it is.", base_of(&path))));
        }
        match (action, method) {
            // ?if_empty=1 (an Undo of New folder): refused once something has been put in it.
            ("", "DELETE") => {
                if qs(q, "if_empty") == Some("1")
                    && fs::read_dir(self.full(&path))?.flatten().any(|e| !e.file_name().to_string_lossy().starts_with('.'))
                {
                    return Err(Bad::new(409, format!("\u{201c}{}\u{201d} has things in it now, so it was left as it is.", base_of(&path))));
                }
                let name = self.to_bin(&path, index)?;
                ok(self.binned(&path, name))
            }
            // ?exact=1 (an Undo): that name or nothing.
            ("rename", "POST") => {
                let want = clean_name(qs(q, "name"));
                if want == base_of(&path) {
                    return ok(json!({"path": path}));
                }
                let parent = parent_of(&path);
                let free = if qs(q, "exact") == Some("1") { self.exact_folder_name(&parent, &want)? } else { self.free_folder_name(&parent, &want)? };
                let new = join(&parent, &free);
                fs::rename(self.full(&path), self.full(&new))?;
                self.moved_prefix(index, &path, &new)?;
                ok(json!({"path": new, "id": fid}))
            }
            ("move" | "copy", "POST") => {
                let to = rel(qs(q, "to"))?;
                self.need_folder(&to)?;
                if to == path || to.starts_with(&format!("{path}/")) {
                    return Err(Bad::new(400, "a folder can't go inside itself"));
                }
                if action == "move" && to == parent_of(&path) {
                    return ok(json!({"path": path}));
                }
                // ?as=<name> (an Undo moving it back): exactly that name there, or refused.
                let free = match qs(q, "as") {
                    Some(want) => self.exact_folder_name(&to, &clean_name(Some(want)))?,
                    None => self.free_folder_name(&to, &base_of(&path))?,
                };
                let new = join(&to, &free);
                if action == "move" {
                    move_any(&self.full(&path), &self.full(&new))?;
                    self.moved_prefix(index, &path, &new)?;
                    return ok(json!({"path": new, "id": fid}));
                }
                copy_tree(&self.full(&path), &self.full(&new))?;
                ok(json!({"path": new}))
            }
            _ => Err(Bad::new(404, "not found")),
        }
    }

    /// The bin of `space` (q "space"; "" without spaces). Old things are cleared out first.
    fn route_bin(&self, method: &str, p: &[String], q: &HashMap<String, String>, index: &mut Index) -> R<Reply> {
        let space = if self.spaces { rel(qs(q, "space"))? } else { String::new() };
        if self.spaces && (space.is_empty() || space.contains('/')) {
            return Err(Bad::new(400, "which bin?"));
        }
        let bin = self.bin(&space);
        let mut bins = self.load_json(&self.bin_index_path());
        let mut present: Vec<String> = fs::read_dir(&bin)
            .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.starts_with('.')).collect())
            .unwrap_or_default();
        present.sort();
        let deleted_at = |bins: &Map<String, Value>, name: &str| -> i64 {
            bins.get(&self.bin_key(&space, name))
                .and_then(|i| i.get("deleted"))
                .and_then(Value::as_i64)
                .or_else(|| fs::metadata(bin.join(name)).ok().map(|m| mtime(&m).0))
                .unwrap_or(0)
        };
        // Things deleted more than BIN_DAYS ago go for good.
        let too_old = now() as i64 - BIN_DAYS * 86400;
        let old: Vec<String> = present.iter().filter(|n| deleted_at(&bins, n) < too_old).cloned().collect();
        if !old.is_empty() {
            for name in &old {
                self.forget(&bin, &space, name, &mut bins)?;
            }
            present.retain(|n| !old.contains(n));
            self.save_json(&self.bin_index_path(), &Value::Object(bins.clone()))?;
        }
        let sub: Vec<&str> = p[1..].iter().map(String::as_str).collect();
        match (sub.as_slice(), method) {
            ([], "GET") => {
                let mut out = Vec::new();
                for name in &present {
                    let m = fs::metadata(bin.join(name))?;
                    let info = bins.get(&self.bin_key(&space, name)).cloned().unwrap_or(json!({}));
                    out.push(json!({
                        "name": name,
                        "original": info.get("original").cloned().unwrap_or(json!(name)),
                        "deleted": deleted_at(&bins, name),
                        "folder": m.is_dir(),
                    }));
                }
                out.sort_by(|a, b| b["deleted"].as_f64().unwrap_or(0.0).total_cmp(&a["deleted"].as_f64().unwrap_or(0.0)));
                ok(Value::Array(out))
            }
            (["restore"], "POST") => {
                let name = qs(q, "name").unwrap_or("");
                if !present.iter().any(|n| n == name) || name.contains('/') {
                    return Err(Bad::new(404, "not in the bin"));
                }
                let key = self.bin_key(&space, name);
                let info = bins.get(&key).cloned().unwrap_or(json!({}));
                let original = info.get("original").and_then(Value::as_str).unwrap_or(name).to_string();
                // An Undo (?expect=<where it was>&exact=1) puts back exactly what it binned, exactly
                // where it was, or says why not: never somewhere else or under another name.
                let exact = qs(q, "exact") == Some("1");
                if qs(q, "expect").is_some_and(|e| e != original) {
                    return Err(Bad::new(409, "That isn't in the bin any more."));
                }
                let mut folder = parent_of(&original);
                let private = crate::mounts::private_path(&original);
                if private {
                    let grant = self.spaces
                        && space_of(&original) == space
                        && crate::auth::users(self)
                            .iter()
                            .find(|u| u.username == space)
                            .is_some_and(|u| crate::mounts::choices(self, u).values().any(|root| crate::mounts::within(&folder, root)));
                    if !grant {
                        return Err(Bad::new(409, "The original account mount is unavailable."));
                    }
                }
                if !self.full(&folder).is_dir() || !self.no_links(&folder) || (self.spaces && space_of(&folder) != space) {
                    if exact || private {
                        return Err(Bad::new(409, format!("The folder \u{201c}{}\u{201d} was in isn't there any more.", base_of(&original))));
                    }
                    folder = space.clone();
                }
                let src = bin.join(name);
                let target = if src.is_dir() {
                    let free = if exact {
                        self.exact_folder_name(&folder, &base_of(&original))?
                    } else {
                        self.free_folder_name(&folder, &base_of(&original))?
                    };
                    let t = join(&folder, &free);
                    move_any(&src, &self.full(&t))?;
                    t
                } else {
                    let (stem, ext) = split_existing(&base_of(&original));
                    let claimed = if exact { self.claim_exact(&folder, &stem, &ext, None)? } else { self.claim_name(&folder, &stem, &ext, None)? };
                    let t = join(&folder, &claimed);
                    with_sidecar(&src, &binned_sidecar(&bin, name), &self.full(&t), false)?;
                    t
                };
                // The ids come back with it: versions and sharing are kept.
                let at = |sub: &str| if sub.is_empty() { target.clone() } else { format!("{target}/{sub}") };
                if let Some(ids) = info.get("files").and_then(Value::as_object) {
                    for (sub, id) in ids {
                        if let Some(id) = id.as_str() {
                            index.retain(|_, p| *p != at(sub));
                            index.insert(id.to_string(), at(sub));
                        }
                    }
                    self.save_index(index)?;
                }
                if let Some(ids) = info.get("folders").and_then(Value::as_object) {
                    let mut folders = self.load_folders();
                    for (sub, id) in ids {
                        if let Some(id) = id.as_str() {
                            folders.retain(|_, p| *p != at(sub));
                            folders.insert(id.to_string(), at(sub));
                        }
                    }
                    self.save_folders(&folders)?;
                }
                bins.remove(&key);
                self.save_json(&self.bin_index_path(), &Value::Object(bins))?;
                ok(json!({"path": target}))
            }
            ([], "DELETE") => {
                let names: Vec<String> = match qs(q, "name") {
                    Some(n) => vec![n.to_string()],
                    None => present.clone(),
                };
                for name in names {
                    if !present.contains(&name) || name.contains('/') {
                        continue;
                    }
                    self.forget(&bin, &space, &name, &mut bins)?;
                }
                self.save_json(&self.bin_index_path(), &Value::Object(bins))?;
                ok(json!({"ok": true}))
            }
            _ => Err(Bad::new(404, "not found")),
        }
    }

    /// Deletes something in a bin for good, with the versions of the files in it.
    fn forget(&self, bin: &Path, space: &str, name: &str, bins: &mut Map<String, Value>) -> io::Result<()> {
        let src = bin.join(name);
        if src.is_dir() {
            fs::remove_dir_all(&src)?
        } else {
            fs::remove_file(&src)?;
            let _ = fs::remove_file(binned_sidecar(bin, name));
        }
        if let Some(info) = bins.remove(&self.bin_key(space, name))
            && let Some(ids) = info.get("files").and_then(Value::as_object)
        {
            for id in ids.values().filter_map(Value::as_str) {
                let _ = fs::remove_dir_all(self.versions().join(id));
                crate::thumbs::forget(self, id);
                crate::commenting::forget_file(self, id);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(clean_name(Some("  a/b:c?.xlsx. ")), "a-b-c-.xlsx");
        assert_eq!(clean_name(None), "Untitled spreadsheet");
        assert_eq!(clean_name(Some(" ... ")), "Untitled spreadsheet");
        assert_eq!(split_name("Budget.XLSX"), ("Budget".into(), ".XLSX".into()));
        assert_eq!(split_name("notes.txt"), ("notes.txt".into(), ".xlsx".into()));
        assert_eq!(split_name("Letter.docx"), ("Letter".into(), ".docx".into()));
        assert!(is_doc("a/Letter.DOCX") && is_doc("b.odt") && !is_doc("c.xlsx"));
        assert!(is_doc("Talk.pptx") && is_sheet_ext(".PPTX"), "presentations are kept, not as sheets");
        assert_eq!(split_name(".xlsx"), (".xlsx".into(), ".xlsx".into()));
        assert_eq!(split_existing("photo.JPG"), ("photo".into(), ".JPG".into()));
        assert_eq!(split_existing("README"), ("README".into(), String::new()));
        assert_eq!(renamed("photo.jpg", "Trip"), ("Trip".into(), ".jpg".into()));
        assert_eq!(renamed("photo.jpg", "Trip.JPG"), ("Trip".into(), ".jpg".into()));
        assert_eq!(renamed("README", "NOTES"), ("NOTES".into(), String::new()));
        assert_eq!(renamed("Budget.xlsx", "Plan.txt"), ("Plan.txt".into(), ".xlsx".into()));
        assert!(is_editable("a.CSV") && is_editable("b.docx") && !is_editable("c.png"));
        assert_eq!(clean_name(Some("photo\u{202E}gpj.exe")), "photogpj.exe", "no direction marks");
        assert_eq!(clean_name(Some("\u{2066}a\u{2069}\u{200E}\u{200F}.png")), "a.png");
        assert_eq!(splitext("a.b.csv"), ("a.b".into(), ".csv".into()));
        assert!(rel(Some("a/../b")).is_err());
        assert_eq!(rel(Some("\\a//b/")).unwrap(), "a/b");
        assert_eq!(parent_of("a/b/c.xlsx"), "a/b");
        assert_eq!(base_of("c.xlsx"), "c.xlsx");
        assert_eq!(stamp().len(), 15);
    }

    /// Google-like thinning: all of the last day, one an hour for a week, one a day before
    /// that; named versions stay; past the cap the most crowded go first.
    #[test]
    fn versions_are_thinned_like_google_docs() {
        let dir = std::env::temp_dir().join(format!("starsheets-thin-{}", random_hex(6)));
        fs::create_dir_all(&dir).unwrap();
        let now = 100 * DAY;
        let mut meta = Map::new();
        // (age in seconds, named): name order is age order, oldest first.
        let ages: [(i64, bool); 10] = [
            (20 * DAY - 50, false), // 20 days ago: two in one day → the newer kept
            (20 * DAY - 100, false),
            (10 * DAY, true), // named: always kept
            (10 * DAY - 60, false),
            (3 * DAY + 120, false), // 3 days ago, same hour as the next → the newer kept
            (3 * DAY + 60, false),
            (3 * DAY - 7200, false),
            (3600, false), // the last day: all kept
            (3500, false),
            (60, false),
        ];
        for (i, (age, named)) in ages.iter().enumerate() {
            let n = format!("v{i:02}.docx");
            fs::write(dir.join(&n), b"x").unwrap();
            let mut e = json!({"saved": now - age});
            if *named {
                e["name"] = json!("Draft");
            }
            meta.insert(n, e);
        }
        meta.insert("current".into(), json!({"name": "Now"}));
        thin(&dir, &mut meta, 100, now).unwrap();
        let left = version_names(&dir);
        assert_eq!(left, ["v01.docx", "v02.docx", "v03.docx", "v05.docx", "v06.docx", "v07.docx", "v08.docx", "v09.docx"]);
        assert!(meta.contains_key("current") && !meta.contains_key("v00.docx"));
        // A cap of 3 unnamed: the crowded last hour loses the most; the named one stays.
        thin(&dir, &mut meta, 3, now).unwrap();
        let left = version_names(&dir);
        assert!(left.contains(&"v02.docx".to_string()), "named kept: {left:?}");
        assert!(left.contains(&"v09.docx".to_string()) && left.contains(&"v01.docx".to_string()), "newest and oldest kept: {left:?}");
        assert_eq!(left.len(), 4, "{left:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn thin_drops_the_oldest_of_a_burst_across_a_second_boundary() {
        // Seven saves in about a second, the clock ticking over after the third (parity.py's
        // "version order" check, which failed now and then in CI): the newest two stay.
        let dir = std::env::temp_dir().join(format!("starsheets-burst-{}", random_hex(6)));
        fs::create_dir_all(&dir).unwrap();
        let now = 100 * DAY;
        let mut meta = Map::new();
        for i in 0..7 {
            let n = format!("v{i}.xlsx");
            fs::write(dir.join(&n), b"x").unwrap();
            meta.insert(n, json!({"saved": now - if i < 3 { 1 } else { 0 }}));
        }
        thin(&dir, &mut meta, 2, now).unwrap();
        assert_eq!(version_names(&dir), ["v5.xlsx", "v6.xlsx"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keep_version_names_docx_versions_and_carries_the_current_name() {
        let dir = std::env::temp_dir().join(format!("starsheets-keep-{}", random_hex(6)));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Letter.docx"), b"one").unwrap();
        let store = Store { root: dir.clone(), keep: 5, spaces: false };
        let id = store.synced_index().unwrap().keys().next().unwrap().clone();
        store.name_version(&id, "current", "  First draft\n").unwrap();
        store.keep_version(&id, &dir.join("Letter.docx")).unwrap();
        let list = store.version_list(&id).unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0]["v"].as_str().unwrap().ends_with(".docx"));
        assert_eq!(list[0]["name"], "First draft");
        // The name moved with the version: the file as it is now has none.
        assert!(store.load_json(&store.versions().join(&id).join(VERSIONS_META)).get("current").is_none());
        assert!(store.name_version(&id, "nope.docx", "x").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    fn numbers(store: &Store, id: &str) -> Vec<u64> {
        store.version_list(id).unwrap().iter().filter_map(|v| v["number"].as_u64()).collect()
    }

    #[test]
    fn version_numbers_only_go_up_after_pruning_restore_and_moves() {
        let dir = std::env::temp_dir().join(format!("starsheets-num-{}", random_hex(6)));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.xlsx");
        fs::write(&f, b"0").unwrap();
        let store = Store { root: dir.clone(), keep: 2, spaces: false };
        let id = store.synced_index().unwrap().keys().next().unwrap().clone();
        for i in 1..=5 {
            store.keep_version(&id, &f).unwrap();
            fs::write(&f, format!("{i}")).unwrap();
        }
        // keep = 2: older ones were pruned, yet the newest keeps rising and nothing repeats.
        let n = numbers(&store, &id);
        assert!(n.len() <= 2, "{n:?}");
        assert_eq!(n[0], 5, "newest first: {n:?}");
        // Restore keeps the current file as a new, higher number.
        let v = store.version_list(&id).unwrap()[0]["v"].as_str().unwrap().to_string();
        let vdir = store.versions().join(&id);
        fs::copy(vdir.join(&v), &f).unwrap();
        store.keep_version(&id, &f).unwrap();
        assert_eq!(numbers(&store, &id)[0], 6);
        // A moved file keeps its id, so its counter.
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::rename(&f, dir.join("sub/a.xlsx")).unwrap();
        store.keep_version(&id, &dir.join("sub/a.xlsx")).unwrap();
        assert_eq!(numbers(&store, &id)[0], 7);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_numbers_start_after_the_highest_for_files_from_before_numbering() {
        let dir = std::env::temp_dir().join(format!("starsheets-mig-{}", random_hex(6)));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.xlsx");
        fs::write(&f, b"x").unwrap();
        let store = Store { root: dir.clone(), keep: 10, spaces: false };
        let id = store.synced_index().unwrap().keys().next().unwrap().clone();
        let vdir = store.versions().join(&id);
        fs::create_dir_all(&vdir).unwrap();
        for n in ["20260101-000000-0000.xlsx", "20260102-000000-0000.xlsx"] {
            fs::write(vdir.join(n), b"old").unwrap();
        }
        store.keep_version(&id, &f).unwrap();
        assert_eq!(numbers(&store, &id), vec![3, 2, 1]);
        store.keep_version(&id, &f).unwrap();
        assert_eq!(numbers(&store, &id)[0], 4);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn path_of_follows_a_file_moved_outside_starsheets() {
        let dir = std::env::temp_dir().join(format!("starsheets-store-{}", random_hex(6)));
        fs::create_dir_all(dir.join("old")).unwrap();
        fs::write(dir.join("old/a.xlsx"), b"x").unwrap();
        let store = Store { root: dir.clone(), keep: 3, spaces: false };
        let id = store.synced_index().unwrap().keys().next().unwrap().clone();
        assert_eq!(store.path_of(&id).unwrap().as_deref(), Some("old/a.xlsx"));
        // Moved over the file share: the index is out of date, so the tree is listed again.
        fs::create_dir_all(dir.join("new")).unwrap();
        fs::rename(dir.join("old/a.xlsx"), dir.join("new/a.xlsx")).unwrap();
        let moved = store.path_of(&id).unwrap();
        assert!(moved.is_none() || moved.as_deref() == Some("new/a.xlsx"), "{moved:?}");
        // Deleted: gone.
        let _ = fs::remove_file(dir.join("new/a.xlsx"));
        assert_eq!(store.path_of(&id).unwrap(), None);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Calls the API the way the Drive does: "GET files/x?a=1".
    fn call(store: &Store, method: &str, url: &str) -> (u16, Value) {
        let (path, query) = url.split_once('?').unwrap_or((url, ""));
        let p: Vec<String> = path.split('/').map(str::to_string).collect();
        let q: HashMap<String, String> = query.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
        match store.route(method, &p, &q, None, b"") {
            Ok(Reply::Json(v, code)) => (code, v),
            Ok(Reply::File { .. }) => (200, Value::Null),
            Err(b) => (b.code, json!({"error": b.msg})),
        }
    }

    fn undo_store() -> (PathBuf, Store, String) {
        let dir = std::env::temp_dir().join(format!("starsheets-undo-{}", random_hex(6)));
        fs::create_dir_all(dir.join("A")).unwrap();
        fs::create_dir_all(dir.join("B")).unwrap();
        fs::write(dir.join("A/a.xlsx"), b"x").unwrap();
        let store = Store { root: dir.clone(), keep: 3, spaces: false };
        let id = store.synced_index().unwrap().keys().next().unwrap().clone();
        (dir, store, id)
    }

    #[test]
    fn storage_counts_own_files_and_the_bin_separately() {
        let dir = std::env::temp_dir().join(format!("starsheets-stor-{}", random_hex(6)));
        fs::create_dir_all(dir.join("ann/docs")).unwrap();
        fs::create_dir_all(dir.join("ann/.bin")).unwrap();
        fs::create_dir_all(dir.join("bob")).unwrap();
        fs::write(dir.join("ann/a.xlsx"), vec![0u8; 100]).unwrap();
        fs::write(dir.join("ann/docs/b.bin"), vec![0u8; 50]).unwrap();
        fs::write(dir.join("ann/.bin/old.xlsx"), vec![0u8; 7]).unwrap();
        fs::write(dir.join("bob/c.xlsx"), vec![0u8; 1000]).unwrap();
        let store = Store { root: dir.clone(), keep: 3, spaces: true };
        let (code, v) = call(&store, "GET", "usage?space=ann");
        assert_eq!((code, v["used"].as_u64(), v["bin"].as_u64(), v["files"].as_u64()), (200, Some(150), Some(7), Some(2)), "{v}");
        assert!(v["free"].as_u64().is_some());
        assert_eq!(call(&store, "GET", "usage").0, 400, "no space named");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_file_write_uses_the_destination_filesystem() {
        use std::os::unix::fs::MetadataExt as _;

        let root = std::env::temp_dir();
        let other = Path::new("/dev/shm");
        let base = match (fs::metadata(&root), fs::metadata(other)) {
            (Ok(root_meta), Ok(other_meta)) if root_meta.dev() != other_meta.dev() => other,
            _ => root.as_path(),
        };
        let dir = base.join(format!("starsheets-write-{}", random_hex(6)));
        fs::create_dir(&dir).unwrap();
        let store = Store { root, keep: 3, spaces: true };
        let path = dir.join("Photo.jpg");
        fs::write(&path, b"original").unwrap();
        store.write_file(&path, b"replacement").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"replacement");

        let occupied = dir.join("folder");
        fs::create_dir(&occupied).unwrap();
        assert!(store.write_file(&occupied, b"fails to replace a directory").is_err());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn private_bin_restore_waits_for_the_original_mount() {
        let dir = std::env::temp_dir().join(format!("starsheets-private-bin-{}", random_hex(6)));
        let store = Store { root: dir.clone(), keep: 3, spaces: true };
        let library = dir.join("ann").join(crate::mounts::DIRECTORY).join("Library");
        fs::create_dir_all(&library).unwrap();
        fs::write(library.join("Photo.jpg"), b"fixture").unwrap();
        let file = format!("ann/{}/Library/Photo.jpg", crate::mounts::DIRECTORY);
        let name = store.to_bin(&file, &mut store.synced_index().unwrap()).unwrap();
        fs::remove_dir(&library).unwrap();

        assert_eq!(call(&store, "POST", &format!("bin/restore?space=ann&name={name}")).0, 409);
        assert!(store.bin("ann").join(&name).is_file());
        assert!(!dir.join("ann/Photo.jpg").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_of_a_move_puts_the_file_back_where_it_was() {
        let (dir, store, id) = undo_store();
        let (code, v) = call(&store, "POST", &format!("files/{id}/move?folder=B"));
        assert_eq!((code, v["name"].as_str(), v["id"].as_str()), (200, Some("a.xlsx"), Some(id.as_str())));
        // The Drive's Undo: back to A under its old name, checked to be still in B.
        let (code, _) = call(&store, "POST", &format!("files/{id}/move?folder=A&from=B/a.xlsx&as=a.xlsx"));
        assert_eq!(code, 200);
        assert!(dir.join("A/a.xlsx").is_file() && !dir.join("B/a.xlsx").exists());
        assert_eq!(store.path_of(&id).unwrap().as_deref(), Some("A/a.xlsx"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_of_a_moved_photo_brings_its_sidecar_back_too() {
        let (dir, store, _) = undo_store();
        fs::write(dir.join("A/p.jpg"), b"pic").unwrap();
        fs::write(dir.join("A/p.jpg.xmp"), b"edits").unwrap();
        let id = store.synced_index().unwrap().iter().find(|(_, v)| v.as_str() == "A/p.jpg").unwrap().0.clone();
        let (code, _) = call(&store, "POST", &format!("files/{id}/move?folder=B"));
        assert_eq!(code, 200);
        assert!(dir.join("B/p.jpg.xmp").is_file() && !dir.join("A/p.jpg.xmp").exists());
        let (code, _) = call(&store, "POST", &format!("files/{id}/move?folder=A&from=B/p.jpg&as=p.jpg"));
        assert_eq!(code, 200);
        assert!(dir.join("A/p.jpg").is_file() && !dir.join("B/p.jpg").exists());
        assert_eq!(fs::read(dir.join("A/p.jpg.xmp")).unwrap(), b"edits");
        assert!(!dir.join("B/p.jpg.xmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_of_a_move_is_refused_after_it_moved_again_or_the_name_was_taken() {
        let (dir, store, id) = undo_store();
        fs::create_dir_all(dir.join("C")).unwrap();
        assert_eq!(call(&store, "POST", &format!("files/{id}/move?folder=B")).0, 200);
        // Someone else moves it on to C: the Undo of our move must not drag it back from there.
        assert_eq!(call(&store, "POST", &format!("files/{id}/move?folder=C")).0, 200);
        let (code, v) = call(&store, "POST", &format!("files/{id}/move?folder=A&from=B/a.xlsx&as=a.xlsx"));
        assert_eq!(code, 409, "{v}");
        assert!(v["error"].as_str().unwrap().contains("moved or renamed since"));
        assert!(dir.join("C/a.xlsx").is_file(), "left where it is");
        // Back in B, but a new a.xlsx now sits in A: refused rather than putting back "a (2).xlsx".
        assert_eq!(call(&store, "POST", &format!("files/{id}/move?folder=B")).0, 200);
        fs::write(dir.join("A/a.xlsx"), b"new").unwrap();
        let (code, v) = call(&store, "POST", &format!("files/{id}/move?folder=A&from=B/a.xlsx&as=a.xlsx"));
        assert_eq!(code, 409, "{v}");
        assert!(dir.join("B/a.xlsx").is_file() && !dir.join("A/a (2).xlsx").exists());
        assert_eq!(fs::read(dir.join("A/a.xlsx")).unwrap(), b"new", "the other file is untouched");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_of_renames_folder_moves_new_folders_and_the_bin() {
        let (dir, store, id) = undo_store();
        // A file's rename, undone exactly; refused when the old name was taken meanwhile.
        assert_eq!(call(&store, "POST", &format!("files/{id}/rename?name=Plan")).0, 200);
        assert!(dir.join("A/Plan.xlsx").is_file());
        assert_eq!(call(&store, "POST", &format!("files/{id}/rename?name=a.xlsx&from=A/Plan.xlsx&exact=1")).0, 200);
        assert!(dir.join("A/a.xlsx").is_file());
        assert_eq!(call(&store, "POST", &format!("files/{id}/rename?name=Plan")).0, 200);
        fs::write(dir.join("A/a.xlsx"), b"other").unwrap();
        assert_eq!(call(&store, "POST", &format!("files/{id}/rename?name=a.xlsx&from=A/Plan.xlsx&exact=1")).0, 409);
        fs::remove_file(dir.join("A/a.xlsx")).unwrap();
        // A folder moved into B and back under its own name; refused once one of that name is there.
        assert_eq!(call(&store, "POST", "folders/move?path=A&to=B").1["path"], "B/A");
        assert_eq!(call(&store, "POST", "folders/move?path=B/A&to=&as=A").1["path"], "A");
        assert_eq!(call(&store, "POST", "folders/move?path=A&to=B").1["path"], "B/A");
        fs::create_dir(dir.join("A")).unwrap();
        assert_eq!(call(&store, "POST", "folders/move?path=B/A&to=&as=A").0, 409);
        assert!(dir.join("B/A/Plan.xlsx").is_file());
        // A new folder's Undo only bins it while it is empty.
        let (_, v) = call(&store, "POST", "folders?path=B&name=New");
        assert_eq!(v["path"], "B/New");
        fs::write(dir.join("B/New/x.txt"), b"x").unwrap();
        assert_eq!(call(&store, "DELETE", "folders?path=B/New&if_empty=1").0, 409);
        fs::remove_file(dir.join("B/New/x.txt")).unwrap();
        assert_eq!(call(&store, "DELETE", "folders?path=B/New&if_empty=1").0, 200);
        // Binned, then put back by its Undo: same place, same id.
        let (code, v) = call(&store, "DELETE", &format!("files/{id}"));
        assert_eq!((code, v["bin"].as_str()), (200, Some("Plan.xlsx")));
        assert_eq!(call(&store, "POST", "bin/restore?name=Plan.xlsx&expect=B/A/Wrong.xlsx&exact=1").0, 409);
        let (code, v) = call(&store, "POST", "bin/restore?name=Plan.xlsx&expect=B/A/Plan.xlsx&exact=1");
        assert_eq!((code, v["path"].as_str()), (200, Some("B/A/Plan.xlsx")));
        assert_eq!(store.path_of(&id).unwrap().as_deref(), Some("B/A/Plan.xlsx"));
        // Its folder gone meanwhile: refused (a plain Restore would put it at the top).
        call(&store, "DELETE", &format!("files/{id}"));
        fs::remove_dir_all(dir.join("B/A")).unwrap();
        assert_eq!(call(&store, "POST", "bin/restore?name=Plan.xlsx&expect=B/A/Plan.xlsx&exact=1").0, 409);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_move_back_keeps_the_files_own_extension() {
        let (dir, store, id) = undo_store();
        assert_eq!(call(&store, "POST", &format!("files/{id}/move?folder=B")).0, 200);
        // `as=` names it as a rename would: never a new extension (report.xlsx -> report.html).
        let (code, v) = call(&store, "POST", &format!("files/{id}/move?folder=A&from=B/a.xlsx&as=evil.html"));
        assert_eq!(code, 200, "{v}");
        assert_eq!(v["name"], "evil.html.xlsx");
        assert!(!dir.join("A/evil.html").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_undo_refuses_another_folder_that_took_its_place() {
        let (dir, store, _) = undo_store();
        let (code, v) = call(&store, "POST", "folders/move?path=A&to=B");
        assert_eq!(code, 200);
        let fid = v["id"].as_str().unwrap().to_string();
        assert!(!fid.is_empty());
        // Someone renames ours away and makes a different "A" in B.
        assert_eq!(call(&store, "POST", "folders/rename?path=B/A&name=Mine").1["path"], "B/Mine");
        fs::create_dir(dir.join("B/A")).unwrap();
        let (code, v) = call(&store, "POST", &format!("folders/move?path=B/A&to=&as=A&id={fid}"));
        assert_eq!(code, 409, "{v}");
        assert!(dir.join("B/A").is_dir() && !dir.join("A").exists(), "the other folder is left alone");
        // Ours, with its id: moved back. A rename answers with the id too.
        let (code, v) = call(&store, "POST", &format!("folders/move?path=B/Mine&to=&as=A&id={fid}"));
        assert_eq!((code, v["path"].as_str()), (200, Some("A")), "{v}");
        let (_, v) = call(&store, "POST", "folders/rename?path=A&name=Z");
        assert_eq!(v["id"].as_str(), Some(fid.as_str()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sidecars_never_replace_or_follow_and_moves_are_all_or_nothing() {
        let dir = std::env::temp_dir().join(format!("starsheets-side-{}", random_hex(6)));
        fs::create_dir_all(dir.join("to")).unwrap();
        let (pic, side) = (dir.join("a.jpg"), dir.join("a.jpg.xmp"));
        fs::write(&pic, b"picture").unwrap();
        fs::write(&side, b"ours").unwrap();
        // Someone else's file at the destination's sidecar name: refused (409), nothing moved,
        // their file untouched, the claimed placeholder gone.
        fs::write(dir.join("to/b.jpg.xmp"), b"theirs").unwrap();
        fs::write(dir.join("to/b.jpg"), b"").unwrap();
        let e = with_sidecar(&pic, &side, &dir.join("to/b.jpg"), false).unwrap_err();
        assert_eq!(e.code, 409);
        assert_eq!(fs::read(dir.join("to/b.jpg.xmp")).unwrap(), b"theirs");
        assert!(!dir.join("to/b.jpg").exists());
        assert_eq!((fs::read(&pic).unwrap(), fs::read(&side).unwrap()), (b"picture".to_vec(), b"ours".to_vec()));
        // A link at the sidecar name is neither followed nor replaced, by a copy or a move.
        let outside = dir.join("outside");
        fs::write(&outside, b"keep").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("to/c.jpg.xmp")).unwrap();
        assert!(with_sidecar(&pic, &side, &dir.join("to/c.jpg"), true).is_err());
        assert!(with_sidecar(&pic, &side, &dir.join("to/c.jpg"), false).is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"keep");
        assert!(copy_new(&side, &dir.join("to/c.jpg.xmp")).is_err() && move_new(&side, &dir.join("to/c.jpg.xmp")).is_err());
        // The picture can't follow (gone): the sidecar comes back.
        let e = with_sidecar(&dir.join("missing.jpg"), &side, &dir.join("to/d.jpg"), false);
        assert!(e.is_err() && side.exists() && !dir.join("to/d.jpg.xmp").exists());
        // The normal case: both arrive, and a copy leaves both behind.
        with_sidecar(&pic, &side, &dir.join("to/e.jpg"), true).unwrap();
        assert_eq!(fs::read(dir.join("to/e.jpg.xmp")).unwrap(), b"ours");
        with_sidecar(&pic, &side, &dir.join("to/f.jpg"), false).unwrap();
        assert!(!pic.exists() && !side.exists() && dir.join("to/f.jpg.xmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_whose_sidecar_exists_counts_as_taken() {
        let dir = std::env::temp_dir().join(format!("starsheets-claim-{}", random_hex(6)));
        fs::create_dir_all(dir.join("f")).unwrap();
        let store = Store { root: dir.clone(), keep: 3, spaces: false };
        fs::write(dir.join("f/p.jpg.xmp"), b"theirs").unwrap();
        assert_eq!(store.claim_name("f", "p", ".jpg", None).unwrap(), "p (2).jpg");
        let _ = fs::remove_dir_all(&dir);
    }
}
