//! Photos (job 1 of the Photos plan): an index of the pictures in the folders Photos is turned on
//! for, and thumbnails of them, made in the background at low priority. The HTTP side is in
//! [`routes`]; the API is described in docs/photos-api.md.
//!
//! - **Opt-in per folder**: nothing is indexed until someone who may edit a folder turns on
//!   "View as photos" for it (`POST /api/photos/enable`); it then covers that folder and every
//!   folder inside it. The folder's owner can turn it off, which forgets its pictures and deletes
//!   their thumbnails.
//! - **Index** ([`index`]): SQLite, in the cache folder ([`Config::from_env`]), never inside
//!   anyone's folder: path, size, modified time, content hash, type, EXIF date, orientation,
//!   camera, place, width and height. A damaged index is moved aside and started again.
//! - **Thumbnails** ([`decode`]): 256 and 1024 pixels on the longest side, kept by content hash,
//!   so a renamed, moved or copied picture is never decoded again; at most
//!   STARBASE_PHOTOS_CACHE_MB, the least recently shown going first. Made by a few worker threads
//!   (STARBASE_PHOTOS_WORKERS, default 2) at low CPU and idle I/O priority. The server streams a
//!   picture once to hash it (never holding it whole), then hands the opened file to a child
//!   process that decodes it within limits of its own; workers × the child's memory limit is kept
//!   within half of the server's memory limit (its cgroup's memory.max).
//! - **Finding changes**: one scanner thread does every scan, from one queue. On a local disk
//!   inotify (not following symbolic links) watches the folders Photos is on for; on a network
//!   file system (a NAS share, where other computers' changes never reach inotify) there is no
//!   watcher, only rescans: a stat-only rescan every STARBASE_PHOTOS_RESCAN seconds (default 10
//!   minutes on a network file system, an hour on a local disk), and opening a folder in Photos
//!   rescans that folder. No scan holds the index's lock for more than one batch of 500 files,
//!   and no request waits for a picture to be decoded beyond a short wait for the one thumbnail
//!   it asked for.
//!
//! Symbolic links are never followed (as everywhere in the store), and hidden files and
//! folders (`.starsheets`, `.bin`) are skipped.

pub mod albums;
pub mod crs;
pub mod decode;
pub mod edits;
pub mod export;
pub mod faces;
#[cfg(test)]
mod faces_tests;
pub mod index;
pub mod ml;
pub mod model;
pub mod people;
pub mod routes;
pub mod sidecar;

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
/// Index changes made under one hold of its lock.
const BATCH: usize = 500;

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
    /// Thumbnail threads (each with one child process at a time).
    pub workers: usize,
    /// Each child's address-space limit, bytes.
    pub child_memory: u64,
    /// Seconds between rescans; None = by file system (network: 600, local: 3600).
    pub rescan: Option<u64>,
    /// Where the index and thumbnails go (STARBASE_PHOTOS_CACHE); None = chosen by [`pick_cache`].
    pub cache: Option<PathBuf>,
    /// The most the thumbnails may take, bytes.
    pub cache_cap: u64,
    /// Watch with inotify (local disks only).
    pub watch: bool,
}

/// The server's memory limit (its cgroup's, v2 or v1), if it has one.
fn memory_limit() -> Option<u64> {
    let read = |p: &str| std::fs::read_to_string(p).ok().and_then(|s| s.trim().parse::<u64>().ok());
    read("/sys/fs/cgroup/memory.max").or_else(|| read("/sys/fs/cgroup/memory/memory.limit_in_bytes")).filter(|&l| l < 1 << 60)
}

/// Workers and child memory that fit in half of `limit`: the child's memory goes down first
/// (to its minimum), then the number of workers (to one).
pub fn fit_memory(workers: usize, child: u64, limit: Option<u64>) -> (usize, u64) {
    let Some(limit) = limit else { return (workers, child) };
    let budget = limit / 2;
    let workers = workers.max(1);
    if workers as u64 * child <= budget {
        return (workers, child);
    }
    let child = (budget / workers as u64).max(decode::MIN_CHILD_MEMORY).min(child);
    let workers = ((budget / child) as usize).clamp(1, workers);
    (workers, child)
}

/// Whether `path` is its own mount (a volume mounted there), not just a folder.
fn is_mount(path: &Path) -> bool {
    let (Ok(a), Ok(b)) = (std::fs::metadata(path), std::fs::metadata(path.parent().unwrap_or(Path::new("/")))) else { return false };
    a.is_dir() && a.dev() != b.dev()
}

impl Config {
    pub fn from_env() -> Config {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let num = |k: &str| get(k).and_then(|v| v.trim().parse::<u64>().ok());
        let cache = get("STARBASE_PHOTOS_CACHE").map(PathBuf::from);
        let (workers, child_memory) = fit_memory(
            num("STARBASE_PHOTOS_WORKERS").unwrap_or(2).clamp(1, 32) as usize,
            num("STARBASE_PHOTOS_CHILD_MB").map_or(decode::CHILD_MEMORY, |m| m * 1024 * 1024).max(decode::MIN_CHILD_MEMORY),
            memory_limit(),
        );
        Config {
            workers,
            child_memory,
            rescan: num("STARBASE_PHOTOS_RESCAN").map(|s| s.max(30)),
            cache,
            cache_cap: num("STARBASE_PHOTOS_CACHE_MB").unwrap_or(20 * 1024).max(64) * 1024 * 1024,
            watch: true,
        }
    }
}

/// FNV-1a: a short, stable name for a store's path.
fn short_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Where the index and thumbnails go, and why (in plain words, for the log). SQLite's locking
/// does not work on network shares (CIFS/SMB, NFS...), so: the setting, else a volume at /cache,
/// else (store on a network share) a local per-user folder, else inside the store.
pub fn pick_cache(
    setting: Option<PathBuf>,
    cache_mounted: bool,
    store_root: &Path,
    store_meta: &Path,
    is_network: impl Fn(&Path) -> bool,
    xdg_cache: Option<PathBuf>,
    home: Option<PathBuf>,
) -> (PathBuf, String) {
    if let Some(p) = setting {
        return (p, "set by STARBASE_PHOTOS_CACHE".into());
    }
    if cache_mounted {
        return (PathBuf::from("/cache/photos"), "a volume is mounted at /cache".into());
    }
    if is_network(store_root) {
        let base = xdg_cache
            .filter(|p| p.is_absolute())
            .or_else(|| home.filter(|p| p.is_absolute()).map(|h| h.join(".cache")))
            .unwrap_or_else(|| PathBuf::from("/var/cache"));
        let dir = base.join("starbase").join("photos").join(short_hash(&store_root.to_string_lossy()));
        return (dir, "the storage folder is on a network share, where the index cannot work, so a local folder is used".into());
    }
    (store_meta.join("photos-cache"), "the storage folder is on a local disk".into())
}

const NEEDS_LOCAL: &str = "Photos needs a local disk for its index; the store is on a network share. Set STARBASE_PHOTOS_CACHE or mount /cache.";
pub(crate) const NOT_SET_UP: &str = "Photos isn't set up on this server yet \u{2014} ask the person who runs it.";
pub(crate) fn admin_unavailable_message() -> &'static str {
    NEEDS_LOCAL
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

/// Whether `path` is `folder` or inside it ("" holds everything).
fn inside(path: &str, folder: &str) -> bool {
    folder.is_empty() || path == folder || path.starts_with(&format!("{folder}/"))
}

#[derive(Default)]
struct Queue {
    order: VecDeque<String>,
    set: HashSet<String>,
    working: usize,
}

/// What [`Photos::list`] found: pictures (path → size, modified ns) and their sidecars'
/// stats (picture path → stat).
type Listed = (HashMap<String, (u64, i64)>, HashMap<String, String>);

/// Scans waiting for the scanner thread: (folder, deep, ticket).
#[derive(Default)]
struct Scans {
    order: VecDeque<(String, bool, u64)>,
    next: u64,
    /// The tickets finished lately (requests wait for theirs).
    done: VecDeque<u64>,
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
    /// The folders Photos is on for (as in the index).
    enabled: Mutex<Vec<String>>,
    scans: Mutex<Scans>,
    scans_wake: Condvar,
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
    cache_bytes: AtomicU64,
    stop: AtomicBool,
    /// The face worker's state (ml.rs).
    pub ml: ml::State,
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
            eprintln!("Photos: can't open the index: {e}{}", if on_network(&store.root) { format!(". {NEEDS_LOCAL}") } else { String::new() });
            None
        }
    }
}

impl Photos {
    /// Opens the index, then starts the workers, the watcher and the scanner.
    pub fn start(store: &Store, cfg: Config) -> Result<Arc<Photos>, String> {
        let (cache, why) = pick_cache(
            cfg.cache.clone(),
            is_mount(Path::new("/cache")),
            &store.root,
            &store.meta(),
            on_network,
            std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
            std::env::var_os("HOME").map(PathBuf::from),
        );
        eprintln!("Photos: index and thumbnails in {} ({why})", cache.display());
        std::fs::create_dir_all(cache.join("thumbs")).map_err(|e| format!("{}: {e}", cache.display()))?;
        let (db, aside) = Index::open_or_rebuild(&cache.join("index.sqlite")).map_err(|e| e.to_string())?;
        if let Some(a) = aside {
            eprintln!("Photos: the index was damaged; kept it as {} and started a new one (pictures are read again).", a.display());
        }
        db.renderer_changed(decode::RENDER).map_err(|e| e.to_string())?;
        let enabled = db.enabled().map_err(|e| e.to_string())?;
        let p = Arc::new(Photos {
            store: Store { root: store.root.clone(), keep: store.keep, spaces: store.spaces },
            cache,
            db: Mutex::new(db),
            queue: Mutex::default(),
            wake: Condvar::new(),
            network: on_network(&store.root),
            cfg,
            enabled: Mutex::new(enabled),
            scans: Mutex::default(),
            scans_wake: Condvar::new(),
            watching: Mutex::new("not started".into()),
            watcher: Mutex::new(None),
            dirty: Mutex::default(),
            looked: Mutex::default(),
            last_scan: Mutex::new(None),
            scanning: AtomicBool::new(false),
            made: AtomicU64::new(0),
            cache_bytes: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            ml: ml::State::new(ml::MlConfig::from_env()),
        });
        let waiting = p.db().pending().unwrap_or_default();
        p.enqueue(waiting, false);
        for i in 0..p.cfg.workers {
            let me = p.clone();
            std::thread::Builder::new().name(format!("photos-{i}")).spawn(move || me.work()).map_err(|e| e.to_string())?;
        }
        p.start_watching();
        let me = p.clone();
        std::thread::Builder::new().name("photos-scan".into()).spawn(move || me.scanner()).map_err(|e| e.to_string())?;
        let me = p.clone();
        std::thread::Builder::new().name("photos-faces".into()).spawn(move || me.face_worker()).map_err(|e| e.to_string())?;
        Ok(p)
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache
    }

    fn with_db<T>(&self, f: impl FnOnce(&Index) -> T) -> T {
        f(&self.db())
    }

    fn db_lock(&self) -> &Mutex<Index> {
        &self.db
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Index> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stops the threads (tests; the server just exits).
    #[cfg(test)]
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.wake.notify_all();
        self.scans_wake.notify_all();
        *self.watcher.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    // ------------------------------------------------------------ turning it on and off

    /// Whether Photos is on for `path` (a folder or a file): it, or a folder above it, is on.
    pub fn covers(&self, path: &str) -> bool {
        self.enabled.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|f| inside(path, f))
    }

    /// Turns Photos on for `folder` (by `who`) and starts indexing it.
    pub fn enable(&self, folder: &str, who: &str) -> Result<(), String> {
        self.db().enable(folder, who).map_err(|e| e.to_string())?;
        let list = self.db().enabled().map_err(|e| e.to_string())?;
        *self.enabled.lock().unwrap_or_else(|e| e.into_inner()) = list;
        self.watch_folder(folder);
        self.request_scan(folder, true, true);
        Ok(())
    }

    /// Turns Photos off for `folder` and every folder inside it: their pictures leave the index
    /// and thumbnails no other picture uses are deleted now. True when a folder above it still
    /// has Photos on (then it stays indexed through that one).
    pub fn disable(&self, folder: &str) -> Result<bool, String> {
        let hashes = self.db().disable(folder).map_err(|e| e.to_string())?;
        let list = self.db().enabled().map_err(|e| e.to_string())?;
        *self.enabled.lock().unwrap_or_else(|e| e.into_inner()) = list;
        if let Some(w) = self.watcher.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            use notify::Watcher;
            let _ = w.unwatch(&self.store.full(folder));
        }
        self.scans.lock().unwrap_or_else(|e| e.into_inner()).order.retain(|(f, _, _)| !inside(f, folder));
        {
            let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
            q.order.retain(|p| !inside(p, folder));
            q.set.retain(|p| !inside(p, folder));
        }
        self.forget_thumbs(hashes);
        Ok(self.covers(folder))
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
            if self.covers(&path) {
                self.process(&path);
            }
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

    /// A thumbnail was shown: it counts as recently used (its modified time, at most daily).
    pub fn used(&self, file: &Path) {
        let old = std::fs::metadata(file).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().unwrap_or_default() > Duration::from_secs(86_400));
        if old && let Ok(f) = std::fs::File::options().write(true).open(file) {
            let _ = f.set_modified(SystemTime::now());
        }
    }

    fn keep_thumb(&self, hash: &str, t: &decode::Thumb) -> std::io::Result<()> {
        let to = self.thumb_base(hash, t.size).with_extension(t.ext);
        std::fs::create_dir_all(to.parent().unwrap_or(&self.cache))?;
        let tmp = to.with_extension(format!("{}.{}.tmp", t.ext, crate::store::random_hex(4)));
        std::fs::write(&tmp, &t.bytes)?;
        std::fs::rename(&tmp, &to)?;
        self.cache_bytes.fetch_add(t.bytes.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    /// The edits in the sidecar of the picture at `path` (none when it has no usable sidecar):
    /// their hash for the index, and the edits themselves (what thumbnails are rendered with).
    fn sidecar_edits(&self, path: &str) -> (index::Side, Option<model::Edits>) {
        let (folder, name) = index::split(path);
        let Some(dir) = sidecar::Folder::open(&self.store, folder) else { return (index::Side::default(), None) };
        match dir.read(&sidecar::name_for(name)) {
            Ok(Some(d)) => {
                // Adobe's crop is relative to the stored picture: known once it has been read.
                let geo = self.db().get(path).ok().flatten().filter(|r| r.meta.width > 0).map(|r| {
                    let o = r.meta.orientation;
                    let (w, h) = if (5..=8).contains(&o) { (r.meta.height, r.meta.width) } else { (r.meta.width, r.meta.height) };
                    sidecar::Geo { width: w, height: h, orientation: o }
                });
                let edits = sidecar::read_for(&d.bytes, geo.as_ref()).ok().map(|p| p.edits).filter(|e| !e.is_identity());
                (index::Side { edits: edits.as_ref().and_then(sidecar::edits_hash), stat: Some(d.stat) }, edits)
            }
            _ => (index::Side::default(), None),
        }
    }

    /// Edits were saved for the picture at `path` (the edits API): its thumbnails are made
    /// again with them, soon. Returns its new `v`, None when it hasn't been read yet.
    pub fn edits_saved(&self, path: &str, side: &index::Side) -> Option<String> {
        let key = self.db().edits_saved(path, side).ok().flatten()?;
        if !decode::SIZES.iter().all(|&s| self.thumb_file(&key, s).is_some()) {
            self.enqueue(vec![path.to_string()], true);
        }
        Some(routes::tag(&key).to_string())
    }

    /// Deletes the thumbnails of these contents, unless a picture still has them.
    fn forget_thumbs(&self, hashes: Vec<String>) {
        for h in hashes {
            if self.db().thumb_used(&h).unwrap_or(true) {
                continue;
            }
            for size in decode::SIZES {
                while let Some((f, _)) = self.thumb_file(&h, size) {
                    let n = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
                    if std::fs::remove_file(&f).is_err() {
                        break;
                    }
                    let mut now = self.cache_bytes.load(Ordering::Relaxed);
                    while let Err(was) = self.cache_bytes.compare_exchange_weak(now, now.saturating_sub(n), Ordering::Relaxed, Ordering::Relaxed) {
                        now = was;
                    }
                }
            }
        }
    }

    /// Reads one picture: its hash, EXIF and thumbnails, into the index. The file is streamed
    /// once to hash it; only the child that decodes it holds it whole.
    fn process(&self, path: &str) {
        let Some((media, true)) = media_of(path) else { return };
        // Opened without following links; the file opened is the one checked (store.rs).
        let Some(mut file) = self.store.open_safe(path) else {
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
        // Its edits (XMP sidecar): the thumbnails' key includes them.
        let (side, edits) = self.sidecar_edits(path);
        // Read before and unchanged since (only its sidecar changed): its hash stands.
        let known_hash = self.db().get(path).ok().flatten().filter(|r| r.size == size && r.mtime_ns == mtime).and_then(|r| r.hash);
        let hash = match known_hash {
            Some(h) => h,
            None => {
                let mut hasher = Sha256::new();
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match file.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => hasher.update(&buf[..n]),
                        Err(_) => return,
                    }
                }
                hex::encode(hasher.finalize())
            }
        };
        let key = sidecar::thumb_key(&hash, side.edits.as_deref());
        // The same content (and edits) was read before (a copy, a move seen as delete + new, or
        // edits undone).
        let have_all = decode::SIZES.iter().all(|&s| self.thumb_file(&key, s).is_some());
        let known = if have_all { self.db().meta_of_hash(&hash).ok().flatten() } else { None };
        if let Some(meta) = known {
            let _ = self.db().done(path, size, mtime, &hash, &meta, render, &side);
            return;
        }
        // The same file again, from the start, for the decoder.
        let Some(file) = self.store.open_safe(path) else { return };
        if file.metadata().map(|m| (m.len(), mtime_ns(&m))).ok() != Some((size, mtime)) {
            return; // changed while it was hashed: the next scan queues it again
        }
        let made = match WORKER_EXE.get() {
            Some(exe) => decode::run_isolated(exe, file, self.cfg.child_memory, edits.as_ref()),
            None => {
                let mut bytes = Vec::new();
                if file.take(MAX_FILE).read_to_end(&mut bytes).is_err() {
                    return;
                }
                decode::make_guarded(&bytes, &decode::Limits::for_memory(self.cfg.child_memory), edits.as_ref())
            }
        };
        match made {
            Ok(made) => {
                if made.thumbs.iter().any(|t| self.keep_thumb(&key, t).is_err()) {
                    let _ = self.db().failed(path, size, mtime, Some(&hash), "couldn't keep the thumbnails", render);
                    return;
                }
                let _ = self.db().done(path, size, mtime, &hash, &made.meta, render, &side);
                self.made.fetch_add(1, Ordering::Relaxed);
            }
            Err(why) => {
                let _ = self.db().failed(path, size, mtime, Some(&hash), &why, render);
            }
        }
    }

    // ------------------------------------------------------------ finding files

    /// Picture files at and below `folder` (only that folder unless `deep`): path → (size,
    /// modified ns), and the pictures' sidecars (`photo.jpg.xmp`): picture path → the sidecar's
    /// stat. None when the folder isn't there (or is reached through a link).
    fn list(&self, folder: &str, deep: bool) -> Option<Listed> {
        if !self.store.no_links(folder) || !std::fs::symlink_metadata(self.store.full(folder)).is_ok_and(|m| m.is_dir()) {
            return None;
        }
        let mut out = HashMap::new();
        let mut sides = HashMap::new();
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
                } else if m.is_file()
                    && let Some(picture) = rel.strip_suffix(".xmp")
                {
                    sides.insert(picture.to_string(), sidecar::stat_of(&m));
                }
            }
        }
        sides.retain(|p, _| out.contains_key(p));
        Some((out, sides))
    }

    /// The parts of `scopes` Photos is on for: a scope outside every such folder becomes the
    /// folders inside it that are on (when deep), or nothing.
    fn covered_scopes(&self, scopes: &[(String, bool)]) -> Vec<(String, bool)> {
        let on = self.enabled.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut out: Vec<(String, bool)> = Vec::new();
        for (f, deep) in scopes {
            if on.iter().any(|e| inside(f, e)) {
                out.push((f.clone(), *deep));
            } else if *deep {
                out.extend(on.iter().filter(|e| inside(e, f)).map(|e| (e.clone(), true)));
            }
        }
        out.sort();
        out.dedup();
        // A deep scope covers the scopes inside it.
        let deep: Vec<String> = out.iter().filter(|s| s.1).map(|s| s.0.clone()).collect();
        out.retain(|(f, is_deep)| !deep.iter().any(|d| inside(f, d) && !(*is_deep && f == d)));
        out
    }

    /// Brings the index in line with the files in `scopes` (folder, and below it if deep),
    /// all at once, so a file moved from one to another keeps its row. Only the folders Photos
    /// is on for. Returns how many picture files were found; new or changed ones are queued.
    /// Runs on the scanner thread (tests call it directly).
    fn scan(&self, scopes: &[(String, bool)]) -> usize {
        let scopes = self.covered_scopes(scopes);
        let mut present: HashMap<String, (u64, i64)> = HashMap::new();
        let mut sides: HashMap<String, String> = HashMap::new();
        let mut gone_scopes = Vec::new();
        for (folder, deep) in &scopes {
            match self.list(folder, *deep) {
                Some((found, side)) => {
                    present.extend(found);
                    sides.extend(side);
                }
                None => gone_scopes.push(folder.clone()),
            }
        }
        let found = present.len();
        let mut rows: HashMap<String, index::Row> = HashMap::new();
        for (folder, deep) in &scopes {
            // A folder that is gone: everything in it, at any depth.
            let deep = *deep || gone_scopes.contains(folder);
            for r in self.db().in_folder(folder, deep).unwrap_or_default() {
                rows.insert(r.path.clone(), r);
            }
        }
        // What to do, worked out without the lock; then done in batches.
        enum Op {
            Moved(String, String),
            Removed(String),
            Found(String, u64, i64),
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
        let mut ops = Vec::new();
        let mut removed_hashes = Vec::new();
        for r in &gone {
            let key = (r.size, r.mtime_ns);
            match by_stat.get(&key) {
                Some(to) if to.len() == 1 && gone_stat[&key] == 1 && r.hash.is_some() => ops.push(Op::Moved(r.path.clone(), to[0].clone())),
                _ => {
                    removed_hashes.extend(r.thumb_key().map(str::to_string));
                    ops.push(Op::Removed(r.path.clone()));
                }
            }
        }
        let mut paths: Vec<&String> = present.keys().collect();
        paths.sort();
        // Read pictures whose sidecar changed (edited in another app, or deleted): made again.
        let mut edited = Vec::new();
        for p in paths {
            let (size, mtime) = present[p];
            // Unchanged rows need nothing.
            if let Some(r) = rows.get(p)
                && r.size == size
                && r.mtime_ns == mtime
                && r.state != State::Pending
            {
                if r.state == State::Ready && r.side.as_ref() != sides.get(p) {
                    edited.push(p.clone());
                }
                continue;
            }
            ops.push(Op::Found(p.clone(), size, mtime));
        }
        let mut wait = edited;
        for chunk in ops.chunks(BATCH) {
            let r = self.db().batch(|ix| {
                let mut w = Vec::new();
                for op in chunk {
                    match op {
                        Op::Moved(from, to) => ix.moved(from, to)?,
                        Op::Removed(p) => ix.remove(p)?,
                        Op::Found(p, size, mtime) => {
                            let Some((media, ok)) = media_of(p) else { continue };
                            if ix.found(p, *size, *mtime, media, ok)? {
                                w.push(p.clone());
                            }
                        }
                    }
                }
                Ok(w)
            });
            match r {
                Ok(w) => wait.extend(w),
                Err(e) => eprintln!("Photos: index update failed: {e}"),
            }
        }
        self.enqueue(wait, false);
        self.forget_thumbs(removed_hashes);
        found
    }

    /// Asks the scanner thread for a scan of `folder` (deep: with everything inside); `urgent`
    /// puts it first. Returns its ticket ([`Self::wait_scan`]).
    pub fn request_scan(&self, folder: &str, deep: bool, urgent: bool) -> u64 {
        let mut s = self.scans.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = s.order.iter().find(|(f, d, _)| f == folder && *d == deep).map(|x| x.2) {
            return t;
        }
        s.next += 1;
        let t = s.next;
        if urgent {
            s.order.push_front((folder.to_string(), deep, t));
        } else {
            s.order.push_back((folder.to_string(), deep, t));
        }
        drop(s);
        self.scans_wake.notify_all();
        t
    }

    /// Waits (at most `max`) for scan `ticket` to finish; true when it has.
    pub fn wait_scan(&self, ticket: u64, max: Duration) -> bool {
        let until = Instant::now() + max;
        let mut s = self.scans.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if s.done.contains(&ticket) {
                return true;
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            s = self.scans_wake.wait_timeout(s, left).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// Rescans one folder for a request (Photos mode just opened it): at most every 5 s (a
    /// whole tree: every minute). Returns the scan's ticket, or None when it was looked at just now.
    pub fn look_at(&self, folder: &str, deep: bool) -> Option<u64> {
        let key = format!("{deep}:{folder}");
        let gap = if deep { Duration::from_secs(60) } else { Duration::from_secs(5) };
        {
            let mut looked = self.looked.lock().unwrap_or_else(|e| e.into_inner());
            if looked.get(&key).is_some_and(|t| t.elapsed() < gap) {
                return None;
            }
            if looked.len() > 10_000 {
                looked.clear();
            }
            looked.insert(key, Instant::now());
        }
        Some(self.request_scan(folder, deep, true))
    }

    /// Asks for `path` to be read next (someone is waiting for its thumbnail). Returns false
    /// when it can't be (not a picture this server reads, not there, or Photos is off for it).
    pub fn hurry(&self, path: &str) -> bool {
        let Some((media, true)) = media_of(path) else { return false };
        if !self.covers(path) {
            return false;
        }
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

    /// inotify on the folders Photos is on for, on a local disk only: on a network file system
    /// it would only walk the whole tree (slowly, over the network) to see this computer's own
    /// changes, and never anyone else's.
    fn start_watching(self: &Arc<Self>) {
        let state = if !self.cfg.watch {
            "off".to_string()
        } else if self.network {
            "rescans only (network storage)".to_string()
        } else {
            let me = Arc::downgrade(self);
            let handler = move |res: notify::Result<notify::Event>| {
                let Some(me) = me.upgrade() else { return };
                match res {
                    Ok(ev) if !ev.kind.is_access() => me.dirty.lock().unwrap_or_else(|e| e.into_inner()).extend(ev.paths),
                    Ok(_) => {}
                    // Events were lost (queue overflow): look at everything again.
                    Err(_) => {
                        me.dirty.lock().unwrap_or_else(|e| e.into_inner()).insert(me.store.root.clone());
                    }
                }
            };
            use notify::Watcher;
            match notify::RecommendedWatcher::new(handler, notify::Config::default().with_follow_symlinks(false)) {
                Ok(w) => {
                    *self.watcher.lock().unwrap_or_else(|e| e.into_inner()) = Some(w);
                    "inotify".to_string()
                }
                Err(e) => format!("rescans only (inotify: {e})"),
            }
        };
        *self.watching.lock().unwrap_or_else(|e| e.into_inner()) = state;
        let on = self.enabled.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for f in on {
            self.watch_folder(&f);
        }
    }

    fn watch_folder(&self, folder: &str) {
        use notify::Watcher;
        let mut w = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(w) = w.as_mut()
            && let Err(e) = w.watch(&self.store.full(folder), notify::RecursiveMode::Recursive)
        {
            *self.watching.lock().unwrap_or_else(|e| e.into_inner()) = format!("inotify, with rescans for some folders ({e})");
        }
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
        if !scopes.is_empty() {
            self.scan(&scopes);
        }
    }

    /// How often everything is rescanned.
    pub fn rescan_every(&self) -> Duration {
        Duration::from_secs(self.cfg.rescan.unwrap_or(if self.network { 600 } else { 3600 }))
    }

    /// The one thread that scans: requested scans first, then the regular rescan, then what
    /// inotify reported (every 2 s at most).
    fn scanner(self: Arc<Self>) {
        low_priority();
        let mut next_full = Instant::now();
        let mut next_settle = Instant::now();
        while !self.stop.load(Ordering::Relaxed) {
            let job = self.scans.lock().unwrap_or_else(|e| e.into_inner()).order.pop_front();
            if let Some((folder, deep, ticket)) = job {
                self.scan(&[(folder, deep)]);
                let mut s = self.scans.lock().unwrap_or_else(|e| e.into_inner());
                s.done.push_back(ticket);
                if s.done.len() > 256 {
                    s.done.pop_front();
                }
                drop(s);
                self.scans_wake.notify_all();
                continue;
            }
            if Instant::now() >= next_full {
                self.full_scan();
                next_full = Instant::now() + self.rescan_every();
                continue;
            }
            if Instant::now() >= next_settle {
                self.settle();
                next_settle = Instant::now() + Duration::from_secs(2);
            }
            let s = self.scans.lock().unwrap_or_else(|e| e.into_inner());
            if s.order.is_empty() {
                let _ = self.scans_wake.wait_timeout(s, Duration::from_secs(2));
            }
        }
    }

    /// Rescans every folder Photos is on for, then clears out thumbnails no picture uses and
    /// keeps the cache within its size.
    pub fn full_scan(&self) {
        self.scanning.store(true, Ordering::Relaxed);
        let (started, t) = (now_secs(), Instant::now());
        let on: Vec<(String, bool)> = self.enabled.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|f| (f.clone(), true)).collect();
        let found = if on.is_empty() { 0 } else { self.scan(&on) };
        *self.last_scan.lock().unwrap_or_else(|e| e.into_inner()) = Some((started, t.elapsed().as_secs_f64(), found));
        self.scanning.store(false, Ordering::Relaxed);
        self.clear_unused();
    }

    /// Deletes kept thumbnails of content no picture has now (older than 10 minutes, so one
    /// just made for a picture not yet marked done stays) and of older renderers; then, while
    /// the cache is over its size, the least recently shown. Counts the cache's size.
    fn clear_unused(&self) {
        let Ok(used) = self.db().thumb_keys() else { return };
        let Ok(dirs) = std::fs::read_dir(self.cache.join("thumbs")) else { return };
        let mut kept: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
        for d in dirs.flatten() {
            let Ok(files) = std::fs::read_dir(d.path()) else { continue };
            for f in files.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                let mut parts = name.splitn(3, '-');
                let (hash, render) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
                let Ok(m) = f.metadata() else { continue };
                let when = m.modified().unwrap_or(UNIX_EPOCH);
                let old = when.elapsed().unwrap_or_default() > Duration::from_secs(600);
                if old && (!used.contains(hash) || render != decode::RENDER) {
                    let _ = std::fs::remove_file(f.path());
                } else {
                    kept.push((when, m.len(), f.path()));
                }
            }
        }
        let mut total: u64 = kept.iter().map(|k| k.1).sum();
        if total > self.cfg.cache_cap {
            // Down to 90%, so it isn't done again for every new thumbnail.
            kept.sort();
            for (_, n, p) in kept {
                if total <= self.cfg.cache_cap / 10 * 9 {
                    break;
                }
                if std::fs::remove_file(p).is_ok() {
                    total -= n;
                }
            }
        }
        self.cache_bytes.store(total, Ordering::Relaxed);
    }

    /// For GET /api/photos/status (the server's owner).
    pub fn status(&self) -> Value {
        let (queued, working) = self.queued();
        let [pending, ready, failed, unsupported] = self.db().counts().unwrap_or_default();
        let last = *self.last_scan.lock().unwrap_or_else(|e| e.into_inner());
        json!({
            "queued": queued,
            "working": working,
            "scanning": self.scanning.load(Ordering::Relaxed),
            "folders": self.enabled.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            "pictures": { "pending": pending, "ready": ready, "failed": failed, "unsupported": unsupported },
            "made_since_start": self.made.load(Ordering::Relaxed),
            "workers": self.cfg.workers,
            "child_memory_mb": self.cfg.child_memory / 1024 / 1024,
            "isolation": if WORKER_EXE.get().is_some() { "process" } else { "thread" },
            "watching": self.watching.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            "network_storage": self.network,
            "rescan_seconds": self.rescan_every().as_secs(),
            "cache": { "folder": self.cache.display().to_string(), "bytes": self.cache_bytes.load(Ordering::Relaxed), "cap": self.cfg.cache_cap },
            "last_rescan": match last {
                Some((at, secs, files)) => json!({ "started": at, "seconds": (secs * 10.0).round() / 10.0, "files": files }),
                None => Value::Null,
            },
        })
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
