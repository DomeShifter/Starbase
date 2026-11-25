//! Photos (job 1 of the Photos plan): an index of every picture in the store and thumbnails of
//! them, made in the background at low priority. The HTTP side is in [`routes`]; the API is
//! described in docs/photos-api.md.
//!
//! - **Index** ([`index`]): SQLite, in `.starsheets/photos-cache/` (or STARBASE_PHOTOS_CACHE),
//!   never inside anyone's folder: path, size, modified time, content hash, type, EXIF date,
//!   orientation, camera, place, width and height.
//! - **Thumbnails** ([`decode`]): 256 and 1024 pixels on the longest side, kept by content hash
//!   in `photos-cache/thumbs/`, so a renamed, moved or copied picture is never decoded again.
//!   Made by a few worker threads (STARBASE_PHOTOS_WORKERS) at low CPU and idle I/O priority,
//!   each picture in a child process with limits of its own.
//! - **Finding changes**: inotify (the `notify` crate) sees changes made through this computer,
//!   including Starbase's own; a stat-only rescan every STARBASE_PHOTOS_RESCAN seconds (default
//!   10 minutes on a network file system such as a NAS share, where other computers' changes
//!   never reach inotify; an hour on a local disk) catches the rest; opening a folder in Photos
//!   rescans that folder. Nothing in a request waits for a picture to be decoded beyond a short
//!   wait for the one thumbnail it asked for.
//!
//! Symbolic links are never followed (as everywhere in the store), and hidden files and
//! folders (`.starsheets`, `.bin`) are skipped.

pub mod decode;
pub mod index;
pub mod routes;

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::store::Store;
use index::{Index, State};

/// The biggest picture file read (bigger ones are listed, marked failed).
pub const MAX_FILE: u64 = 200 * 1024 * 1024;

/// The server's own program, when pictures are to be read in child processes (set by main).
static WORKER_EXE: OnceLock<PathBuf> = OnceLock::new();

/// Read pictures in child processes of `exe` (`exe photo-worker`) from now on.
pub fn use_child_processes(exe: PathBuf) {
    let _ = WORKER_EXE.set(exe);
}

/// What a file is, by its name: its media type and whether thumbnails can be made of it.
pub fn media_of(name: &str) -> Option<(&'static str, bool)> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "jpg" | "jpeg" | "jfif" | "jpe" => ("image/jpeg", true),
        "png" => ("image/png", true),
        "webp" => ("image/webp", true),
        "gif" => ("image/gif", true),
        // Listed, no thumbnails yet (job 5: HEIC through the system's libheif, RAW, AVIF).
        "heic" | "heif" => ("image/heic", false),
        "avif" => ("image/avif", false),
        "tif" | "tiff" => ("image/tiff", false),
        "dng" | "cr2" | "cr3" | "nef" | "arw" | "orf" | "rw2" | "raf" | "pef" | "srw" => ("image/x-raw", false),
        _ => return None,
    })
}

/// Settings (environment).
#[derive(Clone, Debug)]
pub struct Config {
    /// Thumbnail threads.
    pub workers: usize,
    /// Seconds between rescans; None = by file system (network: 600, local: 3600).
    pub rescan: Option<u64>,
    /// Where the index and thumbnails go; None = `.starsheets/photos-cache`.
    pub cache: Option<PathBuf>,
    /// Watch with inotify.
    pub watch: bool,
}

impl Config {
    pub fn from_env() -> Config {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
        Config {
            workers: get("STARBASE_PHOTOS_WORKERS").and_then(|v| v.trim().parse().ok()).unwrap_or((cores / 4).clamp(1, 4)).clamp(1, 32),
            rescan: get("STARBASE_PHOTOS_RESCAN").and_then(|v| v.trim().parse().ok()).map(|s: u64| s.max(30)),
            cache: get("STARBASE_PHOTOS_CACHE").map(PathBuf::from),
            watch: true,
        }
    }
}

/// File systems where other computers change files without inotify hearing of it.
fn on_network(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: statfs fills the struct we give it; the path is a valid C string.
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut s) } != 0 {
        return false;
    }
    // CIFS, SMB2, SMB, NFS, FUSE (sshfs, rclone…), Ceph, AFS, 9p.
    const NET: [i64; 8] = [0xFF534D42u32 as i64, 0xFE534D42u32 as i64, 0x517B, 0x6969, 0x65735546, 0x00C36400, 0x5346414F, 0x01021997];
    NET.contains(&(s.f_type as i64))
}

/// This thread at low CPU priority and idle I/O priority (Linux: both are per thread).
fn low_priority() {
    // SAFETY: system calls that only change this thread's own priority.
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 10);
        // ioprio_set(IOPRIO_WHO_PROCESS, this thread, IOPRIO_CLASS_IDLE << 13)
        libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13);
    }
}

fn mtime_ns(m: &std::fs::Metadata) -> i64 {
    m.mtime().saturating_mul(1_000_000_000).saturating_add(m.mtime_nsec())
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Default)]
struct Queue {
    order: VecDeque<String>,
    set: HashSet<String>,
    working: usize,
}

/// The photo service of one store.
pub struct Photos {
    store: Store,
    cache: PathBuf,
    db: Mutex<Index>,
    queue: Mutex<Queue>,
    wake: Condvar,
    pub cfg: Config,
    network: bool,
    /// "inotify", or why there's no watcher.
    watching: Mutex<String>,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    /// Paths inotify reported, waiting to be looked at.
    dirty: Mutex<HashSet<PathBuf>>,
    /// When each folder was last rescanned for a request.
    looked: Mutex<HashMap<String, Instant>>,
    /// The last full rescan: (started, seconds it took, files found).
    last_scan: Mutex<Option<(u64, f64, usize)>>,
    scanning: AtomicBool,
    made: AtomicU64,
    stop: AtomicBool,
}

static SERVICES: LazyLock<Mutex<HashMap<PathBuf, Arc<Photos>>>> = LazyLock::new(Default::default);

/// The photo service of `store`, started on first use.
pub fn service(store: &Store) -> Option<Arc<Photos>> {
    let mut all = SERVICES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = all.get(&store.root) {
        return Some(p.clone());
    }
    match Photos::start(store, Config::from_env()) {
        Ok(p) => {
            all.insert(store.root.clone(), p.clone());
            Some(p)
        }
        Err(e) => {
            eprintln!("Photos: can't open the index: {e}");
            None
        }
    }
}

impl Photos {
    /// Opens the index, then starts the workers, the watcher and the rescans.
    pub fn start(store: &Store, cfg: Config) -> Result<Arc<Photos>, String> {
        let cache = cfg.cache.clone().unwrap_or_else(|| store.meta().join("photos-cache"));
        std::fs::create_dir_all(cache.join("thumbs")).map_err(|e| format!("{}: {e}", cache.display()))?;
        let db = Index::open(&cache.join("index.sqlite")).map_err(|e| e.to_string())?;
        db.renderer_changed(decode::RENDER).map_err(|e| e.to_string())?;
        let p = Arc::new(Photos {
            store: Store { root: store.root.clone(), keep: store.keep, spaces: store.spaces },
            cache,
            db: Mutex::new(db),
            queue: Mutex::default(),
            wake: Condvar::new(),
            network: on_network(&store.root),
            cfg,
            watching: Mutex::new("not started".into()),
            watcher: Mutex::new(None),
            dirty: Mutex::default(),
            looked: Mutex::default(),
            last_scan: Mutex::new(None),
            scanning: AtomicBool::new(false),
            made: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let waiting = p.db().pending().unwrap_or_default();
        p.enqueue(waiting, false);
        for i in 0..p.cfg.workers {
            let me = p.clone();
            std::thread::Builder::new().name(format!("photos-{i}")).spawn(move || me.work()).map_err(|e| e.to_string())?;
        }
        if p.cfg.watch {
            p.start_watching();
        } else {
            *p.watching.lock().unwrap_or_else(|e| e.into_inner()) = "off".into();
        }
        let me = p.clone();
        std::thread::Builder::new().name("photos-scan".into()).spawn(move || me.rescans()).map_err(|e| e.to_string())?;
        Ok(p)
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Index> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stops the threads (tests; the server just exits).
    #[cfg(test)]
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.wake.notify_all();
        *self.watcher.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    // ------------------------------------------------------------ the queue and the workers

    fn enqueue(&self, paths: Vec<String>, urgent: bool) {
        if paths.is_empty() {
            return;
        }
        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        for p in paths {
            if urgent {
                if q.set.contains(&p) {
                    q.order.retain(|x| *x != p);
                }
                q.order.push_front(p.clone());
                q.set.insert(p);
            } else if q.set.insert(p.clone()) {
                q.order.push_back(p);
            }
        }
        drop(q);
        self.wake.notify_all();
    }

    /// How many pictures wait, and how many are being read now.
    pub fn queued(&self) -> (usize, usize) {
        let q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        (q.order.len(), q.working)
    }

    fn work(self: Arc<Self>) {
        low_priority();
        loop {
            let path = {
                let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if self.stop.load(Ordering::Relaxed) {
                        return;
                    }
                    if let Some(p) = q.order.pop_front() {
                        q.set.remove(&p);
                        q.working += 1;
                        break p;
                    }
                    q = self.wake.wait(q).unwrap_or_else(|e| e.into_inner());
                }
            };
            self.process(&path);
            self.queue.lock().unwrap_or_else(|e| e.into_inner()).working -= 1;
        }
    }

    /// Where a thumbnail of content `hash` is kept (without the extension).
    fn thumb_base(&self, hash: &str, size: u32) -> PathBuf {
        self.cache.join("thumbs").join(&hash[..2]).join(format!("{hash}-{}-{size}", decode::RENDER))
    }

    /// The kept thumbnail of content `hash` at `size`: (file, "jpg"/"png").
    pub fn thumb_file(&self, hash: &str, size: u32) -> Option<(PathBuf, &'static str)> {
        if hash.len() < 2 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let base = self.thumb_base(hash, size);
        ["jpg", "png"].into_iter().map(|ext| (base.with_extension(ext), ext)).find(|(p, _)| p.is_file())
    }

    fn keep_thumb(&self, hash: &str, t: &decode::Thumb) -> std::io::Result<()> {
        let to = self.thumb_base(hash, t.size).with_extension(t.ext);
        std::fs::create_dir_all(to.parent().unwrap_or(&self.cache))?;
        let tmp = to.with_extension(format!("{}.{}.tmp", t.ext, crate::store::random_hex(4)));
        std::fs::write(&tmp, &t.bytes)?;
        std::fs::rename(&tmp, &to)
    }

    /// Reads one picture: its hash, EXIF and thumbnails, into the index.
    fn process(&self, path: &str) {
        let Some((media, true)) = media_of(path) else { return };
        // Opened without following links; the file opened is the one checked (store.rs).
        let Some(file) = self.store.open_safe(path) else {
            let _ = self.db().remove(path);
            return;
        };
        let Ok(m) = file.metadata() else { return };
        let (size, mtime) = (m.len(), mtime_ns(&m));
        let render = decode::RENDER;
        if self.db().found(path, size, mtime, media, true).is_err() {
            return;
        }
        if size > MAX_FILE {
            let _ = self.db().failed(path, size, mtime, None, "the file is too big", render);
            return;
        }
        let mut bytes = Vec::with_capacity(size as usize);
        if file.take(MAX_FILE).read_to_end(&mut bytes).is_err() {
            return;
        }
        let hash = hex::encode(Sha256::digest(&bytes));
        // The same content was read before (a copy, or a move seen as delete + new).
        let have_all = decode::SIZES.iter().all(|&s| self.thumb_file(&hash, s).is_some());
        let known = if have_all { self.db().meta_of_hash(&hash).ok().flatten() } else { None };
        if let Some(meta) = known {
            let _ = self.db().done(path, size, mtime, &hash, &meta, render);
            return;
        }
        let made = match WORKER_EXE.get() {
            Some(exe) => decode::run_isolated(exe, &bytes),
            None => decode::make_guarded(&bytes, &decode::LIMITS),
        };
        drop(bytes);
        match made {
            Ok(made) => {
                if made.thumbs.iter().any(|t| self.keep_thumb(&hash, t).is_err()) {
                    let _ = self.db().failed(path, size, mtime, Some(&hash), "couldn't keep the thumbnails", render);
                    return;
                }
                let _ = self.db().done(path, size, mtime, &hash, &made.meta, render);
                self.made.fetch_add(1, Ordering::Relaxed);
            }
            Err(why) => {
                let _ = self.db().failed(path, size, mtime, Some(&hash), &why, render);
            }
        }
    }

    // ------------------------------------------------------------ finding files

    /// Picture files at and below `folder` (only that folder unless `deep`): path → (size,
    /// modified ns). None when the folder isn't there (or is reached through a link).
    fn list(&self, folder: &str, deep: bool) -> Option<HashMap<String, (u64, i64)>> {
        if !self.store.no_links(folder) || !std::fs::symlink_metadata(self.store.full(folder)).is_ok_and(|m| m.is_dir()) {
            return None;
        }
        let mut out = HashMap::new();
        let mut todo = vec![folder.to_string()];
        while let Some(here) = todo.pop() {
            let Ok(rd) = std::fs::read_dir(self.store.full(&here)) else { continue };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                // Never through a symbolic link (store.rs walk_dir says why).
                let Ok(m) = std::fs::symlink_metadata(e.path()) else { continue };
                let rel = crate::store::join(&here, &name);
                if m.is_dir() {
                    if deep {
                        todo.push(rel);
                    }
                } else if m.is_file() && media_of(&name).is_some() {
                    out.insert(rel, (m.len(), mtime_ns(&m)));
                }
            }
        }
        Some(out)
    }

    /// Brings the index in line with the files in `scopes` (folder, and below it if deep),
    /// all at once, so a file moved from one to another keeps its row. Returns how many picture
    /// files were found; new or changed ones are queued.
    pub fn scan(&self, scopes: &[(String, bool)]) -> usize {
        let mut present: HashMap<String, (u64, i64)> = HashMap::new();
        let mut gone_scopes = Vec::new();
        for (folder, deep) in scopes {
            match self.list(folder, *deep) {
                Some(found) => present.extend(found),
                None => gone_scopes.push(folder.clone()),
            }
        }
        let found = present.len();
        let mut db = self.db();
        let r = db.batch(|ix| {
            let mut rows: HashMap<String, index::Row> = HashMap::new();
            for (folder, deep) in scopes {
                // A folder that is gone: everything in it, at any depth.
                let deep = *deep || gone_scopes.contains(folder);
                for r in ix.in_folder(folder, deep)? {
                    rows.insert(r.path.clone(), r);
                }
            }
            let gone: Vec<&index::Row> = rows.values().filter(|r| !present.contains_key(&r.path)).collect();
            let mut new: Vec<&String> = present.keys().filter(|p| !rows.contains_key(*p)).collect();
            new.sort();
            // Moves: a gone file and a new one with the same size and modified time, one each.
            let mut by_stat: HashMap<(u64, i64), Vec<&String>> = HashMap::new();
            for p in &new {
                by_stat.entry(present[*p]).or_default().push(p);
            }
            let mut gone_stat: HashMap<(u64, i64), usize> = HashMap::new();
            for r in &gone {
                *gone_stat.entry((r.size, r.mtime_ns)).or_default() += 1;
            }
            for r in &gone {
                let key = (r.size, r.mtime_ns);
                match by_stat.get(&key) {
                    Some(to) if to.len() == 1 && gone_stat[&key] == 1 && r.hash.is_some() => ix.moved(&r.path, to[0])?,
                    _ => ix.remove(&r.path)?,
                }
            }
            let mut wait = Vec::new();
            let mut paths: Vec<&String> = present.keys().collect();
            paths.sort();
            for p in paths {
                let (size, mtime) = present[p];
                let Some((media, ok)) = media_of(p) else { continue };
                if ix.found(p, size, mtime, media, ok)? {
                    wait.push(p.clone());
                }
            }
            Ok(wait)
        });
        drop(db);
        match r {
            Ok(wait) => self.enqueue(wait, false),
            Err(e) => eprintln!("Photos: index update failed: {e}"),
        }
        found
    }

    /// Rescans one folder for a request (Photos mode just opened it), at most every 5 s.
    pub fn look_at(&self, folder: &str, deep: bool) {
        let key = format!("{deep}:{folder}");
        let gap = if deep { Duration::from_secs(60) } else { Duration::from_secs(5) };
        {
            let mut looked = self.looked.lock().unwrap_or_else(|e| e.into_inner());
            if looked.get(&key).is_some_and(|t| t.elapsed() < gap) {
                return;
            }
            if looked.len() > 10_000 {
                looked.clear();
            }
            looked.insert(key, Instant::now());
        }
        self.scan(&[(folder.to_string(), deep)]);
    }

    /// Asks for `path` to be read next (someone is waiting for its thumbnail). Returns false
    /// when it can't be (not a picture this server reads, or not there).
    pub fn hurry(&self, path: &str) -> bool {
        let Some((media, true)) = media_of(path) else { return false };
        let Some(full) = self.store.safe_file(path) else { return false };
        let Ok(m) = std::fs::symlink_metadata(&full) else { return false };
        let _ = self.db().found(path, m.len(), mtime_ns(&m), media, true);
        self.enqueue(vec![path.to_string()], true);
        true
    }

    pub fn row(&self, path: &str) -> Option<index::Row> {
        self.db().get(path).ok().flatten()
    }

    pub fn rows(&self, folder: &str, deep: bool) -> Vec<index::Row> {
        self.db().in_folder(folder, deep).unwrap_or_default()
    }

    /// Whether the row still describes the file on disk.
    pub fn fresh(&self, r: &index::Row) -> bool {
        self.store.safe_file(&r.path).and_then(|f| std::fs::symlink_metadata(f).ok()).is_some_and(|m| m.len() == r.size && mtime_ns(&m) == r.mtime_ns)
    }

    // ------------------------------------------------------------ watching and rescans

    fn start_watching(self: &Arc<Self>) {
        use notify::Watcher;
        let me = Arc::downgrade(self);
        let made = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Some(me) = me.upgrade() else { return };
            match res {
                Ok(ev) if !ev.kind.is_access() => me.dirty.lock().unwrap_or_else(|e| e.into_inner()).extend(ev.paths),
                Ok(_) => {}
                // Events were lost (queue overflow): look at everything again.
                Err(_) => {
                    me.dirty.lock().unwrap_or_else(|e| e.into_inner()).insert(me.store.root.clone());
                }
            }
        });
        let state = match made {
            Ok(mut w) => match w.watch(&self.store.root, notify::RecursiveMode::Recursive) {
                Ok(()) => {
                    *self.watcher.lock().unwrap_or_else(|e| e.into_inner()) = Some(w);
                    "inotify".to_string()
                }
                Err(e) => format!("rescans only (inotify: {e})"),
            },
            Err(e) => format!("rescans only (inotify: {e})"),
        };
        *self.watching.lock().unwrap_or_else(|e| e.into_inner()) = state;
    }

    /// What inotify reported since last time, as scan scopes.
    fn settle(&self) {
        let dirty: Vec<PathBuf> = std::mem::take(&mut *self.dirty.lock().unwrap_or_else(|e| e.into_inner())).into_iter().collect();
        if dirty.is_empty() {
            return;
        }
        let mut scopes: Vec<(String, bool)> = Vec::new();
        for p in dirty {
            let Ok(rel) = p.strip_prefix(&self.store.root) else { continue };
            let rel: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            if rel.iter().any(|c| c.starts_with('.')) {
                continue; // .starsheets (our own cache too), bins, hidden files
            }
            let rel = rel.join("/");
            let is_file = std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_file());
            scopes.push((crate::store::parent_of(&rel), false));
            if !is_file {
                // A folder that appeared (moved in) or went: everything in it.
                scopes.push((rel, true));
            }
        }
        scopes.sort();
        scopes.dedup();
        // A deep scope covers the scopes inside it.
        let deep: Vec<String> = scopes.iter().filter(|s| s.1).map(|s| s.0.clone()).collect();
        let inside = |f: &str, d: &String| d.is_empty() || f == d || f.starts_with(&format!("{d}/"));
        scopes.retain(|(f, is_deep)| !deep.iter().any(|d| inside(f, d) && !(*is_deep && f == d)));
        if !scopes.is_empty() {
            self.scan(&scopes);
        }
    }

    /// How often everything is rescanned.
    pub fn rescan_every(&self) -> Duration {
        Duration::from_secs(self.cfg.rescan.unwrap_or(if self.network { 600 } else { 3600 }))
    }

    /// Everything, then every rescan interval; inotify's changes every 2 s in between.
    fn rescans(self: Arc<Self>) {
        low_priority();
        let mut next = Instant::now();
        while !self.stop.load(Ordering::Relaxed) {
            if Instant::now() >= next {
                self.full_scan();
                next = Instant::now() + self.rescan_every();
            }
            self.settle();
            std::thread::sleep(Duration::from_secs(2));
        }
    }

    /// Rescans the whole store, then clears out thumbnails no picture uses any more.
    pub fn full_scan(&self) {
        self.scanning.store(true, Ordering::Relaxed);
        let (started, t) = (now_secs(), Instant::now());
        let found = self.scan(&[(String::new(), true)]);
        *self.last_scan.lock().unwrap_or_else(|e| e.into_inner()) = Some((started, t.elapsed().as_secs_f64(), found));
        self.scanning.store(false, Ordering::Relaxed);
        self.clear_unused();
    }

    /// Deletes kept thumbnails of content no picture has now (older than 10 minutes, so one
    /// just made for a picture not yet marked done stays) and of older renderers.
    fn clear_unused(&self) {
        let Ok(used) = self.db().hashes() else { return };
        let Ok(dirs) = std::fs::read_dir(self.cache.join("thumbs")) else { return };
        for d in dirs.flatten() {
            let Ok(files) = std::fs::read_dir(d.path()) else { continue };
            for f in files.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                let mut parts = name.splitn(3, '-');
                let (hash, render) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                let old = f.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().unwrap_or_default() > Duration::from_secs(600));
                if old && (!used.contains(hash) || render != decode::RENDER) {
                    let _ = std::fs::remove_file(f.path());
                }
            }
        }
    }

    /// For GET /api/photos/status. `all`: with the whole store's counts (the owner).
    pub fn status(&self, all: bool) -> Value {
        let (queued, working) = self.queued();
        let mut v = json!({
            "queued": queued,
            "working": working,
            "scanning": self.scanning.load(Ordering::Relaxed),
        });
        if all {
            let [pending, ready, failed, unsupported] = self.db().counts().unwrap_or_default();
            let last = *self.last_scan.lock().unwrap_or_else(|e| e.into_inner());
            v["pictures"] = json!({ "pending": pending, "ready": ready, "failed": failed, "unsupported": unsupported });
            v["made_since_start"] = json!(self.made.load(Ordering::Relaxed));
            v["workers"] = json!(self.cfg.workers);
            v["isolation"] = json!(if WORKER_EXE.get().is_some() { "process" } else { "thread" });
            v["watching"] = json!(self.watching.lock().unwrap_or_else(|e| e.into_inner()).clone());
            v["network_storage"] = json!(self.network);
            v["rescan_seconds"] = json!(self.rescan_every().as_secs());
            v["last_rescan"] = match last {
                Some((at, secs, files)) => json!({ "started": at, "seconds": (secs * 10.0).round() / 10.0, "files": files }),
                None => Value::Null,
            };
        }
        v
    }

    /// Waits (at most `max`) until `path` has been read (or failed); false on running out of time.
    pub fn wait_for(&self, path: &str, max: Duration) -> bool {
        let until = Instant::now() + max;
        loop {
            if let Some(r) = self.row(path)
                && r.state != State::Pending
            {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }
}

#[cfg(test)]
mod tests;
