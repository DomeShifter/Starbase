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

pub const EXTS: [&str; 7] = [".xlsx", ".xlsm", ".xls", ".csv", ".ods", ".docx", ".odt"];

/// Documents (StarDocs) rather than spreadsheets: stored, listed, shared and versioned like any
/// file, but never opened in a live spreadsheet room or drawn as a sheet preview.
pub const DOC_EXTS: [&str; 2] = [".docx", ".odt"];

/// Whether a file name is a StarDocs document.
pub fn is_doc(name: &str) -> bool {
    let lower = name.to_lowercase();
    DOC_EXTS.iter().any(|x| lower.ends_with(x))
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

fn numbered(stem: &str, n: u32, ext: &str) -> String {
    if n == 1 { format!("{stem}{ext}") } else { format!("{stem} ({n}){ext}") }
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
            if fs::metadata(e.path()).map(|m| m.is_dir()).unwrap_or(false) {
                dirs.push(name);
            } else if EXTS.iter().any(|x| name.to_lowercase().ends_with(x)) {
                files.push(join(here, &name));
            }
        }
        dirs.sort();
        folders.extend(dirs.iter().map(|d| join(here, d)));
        for d in dirs {
            self.walk_dir(&join(here, &d), files, folders);
        }
    }

    /// Where file `id` is now. The index is trusted while the file is still at the path it
    /// gives, which costs one file check; only when it isn't (moved or binned outside
    /// StarSheets) is the whole folder tree listed again ([`Self::synced_index`]). Listing a big
    /// tree over a network share every few seconds for each open file held up everyone else.
    pub fn path_of(&self, id: &str) -> io::Result<Option<String>> {
        if let Some(path) = self.load_index().get(id)
            && self.full(path).is_file()
        {
            return Ok(Some(path.clone()));
        }
        Ok(self.synced_index()?.get(id).cloned())
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
        let m = fs::metadata(path)?;
        Ok(format!("{:x}-{:x}", mtime(&m).1, m.len()))
    }

    /// Writes a file atomically (temp file in .starsheets, then rename).
    pub fn write_file(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        fs::create_dir_all(self.meta())?;
        let tmp = self.meta().join(format!(".{}.tmp", random_hex(6)));
        fs::write(&tmp, data)?;
        fs::rename(&tmp, path)
    }

    /// Copies the file as it is now into its versions folder, keeping the newest KEEP.
    pub fn keep_version(&self, fid: &str, path: &Path) -> io::Result<()> {
        match fs::metadata(path) {
            Ok(m) if m.len() > 0 => {}
            _ => return Ok(()),
        }
        let folder = self.versions().join(fid);
        fs::create_dir_all(&folder)?;
        // Names sort in the order the versions were made, also several within one millisecond:
        // a name that wouldn't sort after the newest one takes the newest name plus "z".
        let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_millis()).unwrap_or(0);
        let mut existing: Vec<String> = fs::read_dir(&folder)?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        existing.sort();
        let mut stem = format!("{}-{ms:03}0", stamp());
        if let Some(last) = existing.last().map(|l| l.trim_end_matches(".xlsx"))
            && stem.as_str() <= last
        {
            stem = format!("{last}z");
        }
        fs::copy(path, folder.join(format!("{stem}.xlsx")))?;
        let mut old: Vec<_> = fs::read_dir(&folder)?.flatten().map(|e| e.file_name()).collect();
        old.sort();
        let drop = old.len().saturating_sub(self.keep);
        for v in &old[..drop] {
            fs::remove_file(folder.join(v))?;
        }
        Ok(())
    }

    /// Moves a file or folder into the bin (its owner's, with spaces), remembering where it was
    /// and the ids of everything in it.
    fn to_bin(&self, relpath: &str, index: &mut Index) -> io::Result<()> {
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
        move_any(&self.full(relpath), &bin.join(&target))?;
        index.retain(|_, p| under(p).is_none());
        self.save_index(index)?;
        folders.retain(|_, p| under(p).is_none());
        self.save_folders(&folders)?;
        let mut bins = self.load_json(&self.bin_index_path());
        bins.insert(self.bin_key(space, &target), json!({"original": relpath, "deleted": now() as i64, "files": file_ids, "folders": folder_ids}));
        self.save_json(&self.bin_index_path(), &Value::Object(bins))
    }

    /// Reserves a free file name in `folder` (stem.ext, stem (2).ext, …) by creating it exclusively.
    pub fn claim_name(&self, folder: &str, stem: &str, ext: &str, own: Option<&str>) -> R<String> {
        use std::os::unix::fs::OpenOptionsExt;
        for n in 1..10000 {
            let name = numbered(stem, n, ext);
            if own == Some(join(folder, &name).as_str()) {
                return Ok(name);
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
        (1..10000).map(|n| numbered(name, n, "")).find(|c| !self.full(parent).join(c).exists()).ok_or_else(|| Bad::new(409, "no free name"))
    }

    fn need_folder(&self, folder: &str) -> R<()> {
        if self.full(folder).is_dir() { Ok(()) } else { Err(Bad::new(404, "no such folder")) }
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
            _ => Err(Bad::new(404, "not found")),
        }
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
            let (files, folders) = self.walk();
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
            let (stem, ext) = split_name(&clean_name(qs(q, "name")));
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
        let fpath = self.full(&path);
        let rest: Vec<&str> = p[2..].iter().map(String::as_str).collect();
        match (rest.as_slice(), method) {
            ([], "GET") => {
                let data = fs::read(&fpath)?;
                let modified = mtime(&fs::metadata(&fpath)?).0;
                Ok(Reply::File { data, name: base_of(&path), version: self.version_of(&fpath)?, folder: parent_of(&path), modified })
            }
            ([], "PUT") => {
                if let Some(want) = if_match.filter(|w| !w.is_empty())
                    && want != self.version_of(&fpath)?
                {
                    return Ok(Reply::Json(json!({"error": "changed elsewhere", "name": base_of(&path)}), 409));
                }
                self.keep_version(fid, &fpath)?;
                self.write_file(&fpath, body)?;
                ok(json!({"version": self.version_of(&fpath)?, "name": base_of(&path), "folder": parent_of(&path)}))
            }
            ([], "DELETE") => {
                self.to_bin(&path, index)?;
                ok(json!({"ok": true}))
            }
            (["rename"], "POST") => {
                let folder = parent_of(&path);
                let (stem, ext) = split_name(&clean_name(qs(q, "name")));
                let (_, old_ext) = split_name(&base_of(&path));
                let ext = if old_ext.is_empty() { ext } else { old_ext };
                let name = self.claim_name(&folder, &stem, &ext, Some(&path))?;
                let new = join(&folder, &name);
                if new != path {
                    fs::rename(&fpath, self.full(&new))?;
                    index.insert(fid.to_string(), new);
                    self.save_index(index)?;
                }
                ok(json!({"name": name}))
            }
            (["move"], "POST") => {
                let folder = rel(qs(q, "folder"))?;
                self.need_folder(&folder)?;
                if folder == parent_of(&path) {
                    return ok(json!({"name": base_of(&path)}));
                }
                let (stem, ext) = split_name(&base_of(&path));
                let name = self.claim_name(&folder, &stem, &ext, None)?;
                move_any(&fpath, &self.full(&folder).join(&name))?;
                index.insert(fid.to_string(), join(&folder, &name));
                self.save_index(index)?;
                ok(json!({"name": name}))
            }
            (["copy"], "POST") => {
                let parent = parent_of(&path);
                let folder = rel(Some(qs(q, "folder").unwrap_or(&parent)))?;
                self.need_folder(&folder)?;
                let (mut stem, ext) = split_name(&base_of(&path));
                if folder == parent {
                    stem = format!("Copy of {stem}");
                }
                let name = self.claim_name(&folder, &stem, &ext, None)?;
                fs::copy(&fpath, self.full(&folder).join(&name))?;
                let new_id = random_hex(8);
                index.insert(new_id.clone(), join(&folder, &name));
                self.save_index(index)?;
                ok(json!({"id": new_id, "name": name}))
            }
            (["versions", ..], "GET") => {
                let vdir = self.versions().join(fid);
                let mut vs: Vec<String> =
                    fs::read_dir(&vdir).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
                vs.sort();
                vs.reverse();
                if rest.len() == 1 {
                    let mut out = Vec::new();
                    for v in &vs {
                        let m = fs::metadata(vdir.join(v))?;
                        out.push(json!({"v": v, "modified": mtime(&m).0, "size": m.len()}));
                    }
                    return ok(Value::Array(out));
                }
                if !vs.iter().any(|v| v == rest[1]) {
                    return Err(Bad::new(404, "no such version"));
                }
                let data = fs::read(vdir.join(rest[1]))?;
                Ok(Reply::File { data, name: base_of(&path), version: String::new(), folder: String::new(), modified: 0 })
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
        self.synced_folders()?;
        match (action, method) {
            ("", "DELETE") => {
                self.to_bin(&path, index)?;
                ok(json!({"ok": true}))
            }
            ("rename", "POST") => {
                let want = clean_name(qs(q, "name"));
                if want == base_of(&path) {
                    return ok(json!({"path": path}));
                }
                let parent = parent_of(&path);
                let new = join(&parent, &self.free_folder_name(&parent, &want)?);
                fs::rename(self.full(&path), self.full(&new))?;
                self.moved_prefix(index, &path, &new)?;
                ok(json!({"path": new}))
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
                let new = join(&to, &self.free_folder_name(&to, &base_of(&path))?);
                if action == "move" {
                    move_any(&self.full(&path), &self.full(&new))?;
                    self.moved_prefix(index, &path, &new)?;
                } else {
                    copy_tree(&self.full(&path), &self.full(&new))?;
                }
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
                let mut folder = parent_of(&original);
                if !self.full(&folder).is_dir() || (self.spaces && space_of(&folder) != space) {
                    folder = space.clone();
                }
                let src = bin.join(name);
                let target = if src.is_dir() {
                    let t = join(&folder, &self.free_folder_name(&folder, &base_of(&original))?);
                    move_any(&src, &self.full(&t))?;
                    t
                } else {
                    let (stem, ext) = split_name(&base_of(&original));
                    let t = join(&folder, &self.claim_name(&folder, &stem, &ext, None)?);
                    fs::rename(&src, self.full(&t))?;
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
            fs::remove_file(&src)?
        }
        if let Some(info) = bins.remove(&self.bin_key(space, name))
            && let Some(ids) = info.get("files").and_then(Value::as_object)
        {
            for id in ids.values().filter_map(Value::as_str) {
                let _ = fs::remove_dir_all(self.versions().join(id));
                crate::thumbs::forget(self, id);
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
        assert_eq!(split_name(".xlsx"), (".xlsx".into(), ".xlsx".into()));
        assert_eq!(splitext("a.b.csv"), ("a.b".into(), ".csv".into()));
        assert!(rel(Some("a/../b")).is_err());
        assert_eq!(rel(Some("\\a//b/")).unwrap(), "a/b");
        assert_eq!(parent_of("a/b/c.xlsx"), "a/b");
        assert_eq!(base_of("c.xlsx"), "c.xlsx");
        assert_eq!(stamp().len(), 15);
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
}
