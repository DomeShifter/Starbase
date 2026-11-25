//! Backups: a copy of every file, version, bin and account, made on a schedule into a folder the
//! owner chooses (a USB disk, a NAS share, any folder the server can reach), encrypted, in restic's
//! repository format (written with rustic_core). Because the format is restic's, any restic or
//! rustic program can read these backups too: if this server is lost, `restic -r <folder> restore
//! latest --target <somewhere>` with the recovery key gets everything back.
//!
//! Settings live in `<base>/.starsheets-backup.json` (next to `.starsheets-root`, outside the files
//! folder, so they aren't in the backups themselves). That file also holds the recovery key (the
//! repository password), which the owner is asked to keep somewhere else as well: a backup can't
//! be opened without it.
//!
//! What a backup holds: the files folder in use (STORE's chosen folder), stored under the path
//! `/starbase` inside the backup, so a restore with restic gives one `starbase` folder.
//!
//! The schedule: once a day at the hour chosen (server time). A run that was missed (the server
//! was off) happens at the next check. After each backup, old ones are thinned out (keep the last
//! 7 days, 4 weeks, 12 months by default) and the space they used is freed.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rustic_backend::BackendOptions;
use rustic_core::{
    BackupOptions, ConfigOptions, Credentials, IndexedTree, KeepOptions, KeyOptions, LocalDestination, LsOptions, OpenStatus, PathList, PruneOptions,
    Repository, RepositoryOptions, RestoreOptions, SnapshotOptions, repofile::Node,
};
use serde_json::{Map, Value, json};

/// Inside a backup, the files folder is called this (restic shows it as a folder `starbase`).
const AS_PATH: &str = "/starbase";
/// Files never worth keeping: half-written saves, a move in progress, and these settings (they
/// hold the recovery key; they sit inside the files folder when that is the top of STORE).
const SKIP: &[&str] = &["!*.tmp", "!.moving-*", "!.starsheets-backup.json"];
/// The name backups are made under (restic's "host"). A container's own name changes whenever it
/// is made again, and restic compares a backup with the last one from the same host.
const HOST: &str = "starbase";

static RUNNING: AtomicBool = AtomicBool::new(false);
/// Settings file reads and writes, one at a time.
static FILE: Mutex<()> = Mutex::new(());

pub fn settings_path(base: &Path) -> PathBuf {
    base.join(".starsheets-backup.json")
}

pub fn running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

pub fn load(base: &Path) -> Map<String, Value> {
    let _g = FILE.lock().unwrap_or_else(|e| e.into_inner());
    std::fs::read(settings_path(base))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Changes some settings (the rest stay), written the safe way (a new file, then renamed), readable
/// only by the server's own user: it holds the recovery key.
pub fn update(base: &Path, change: impl FnOnce(&mut Map<String, Value>)) -> std::io::Result<()> {
    let _g = FILE.lock().unwrap_or_else(|e| e.into_inner());
    let path = settings_path(base);
    let mut s =
        std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| v.as_object().cloned()).unwrap_or_default();
    change(&mut s);
    let tmp = base.join(".starsheets-backup.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&Value::Object(s)).unwrap_or_default())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &path)
}

/// A new recovery key: 32 random hex characters in groups of four, easy to copy and type.
pub fn new_key() -> String {
    let hex = crate::store::random_hex(16);
    hex.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("-")
}

/// A key as someone typed it (spaces, capitals, no dashes) in the form it was given out.
pub fn clean_key(typed: &str) -> String {
    let hex: String = typed.chars().filter(char::is_ascii_hexdigit).map(|c| c.to_ascii_lowercase()).collect();
    if hex.len() == 32 {
        hex.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("-")
    } else {
        // Someone else's restic password (a repository made elsewhere): use it as it is.
        typed.trim().to_string()
    }
}

fn num(s: &Map<String, Value>, key: &str, default: i64) -> i64 {
    s.get(key).and_then(Value::as_i64).unwrap_or(default)
}

fn repo(to: &str) -> Result<Repository<()>, String> {
    let backends = BackendOptions::default().repository(to).to_backends().map_err(|e| e.to_string())?;
    Repository::new(&RepositoryOptions::default(), &backends).map_err(|e| e.to_string())
}

fn open(to: &str, key: &str) -> Result<Repository<OpenStatus>, String> {
    repo(to)?.open(&Credentials::Password(key.to_string())).map_err(|e| {
        let e = e.to_string();
        if e.to_lowercase().contains("password") || e.to_lowercase().contains("key") {
            "The recovery key doesn't open the backups in that folder.".to_string()
        } else {
            e
        }
    })
}

/// Whether a folder already holds a restic repository (its `config` file and `keys` folder).
pub fn is_repository(dir: &Path) -> bool {
    dir.join("config").is_file() && dir.join("keys").is_dir()
}

/// Checks a folder chosen for backups and gets it ready: an empty (or new) folder becomes a new
/// backup store locked with `key`; one that already holds backups must open with `key`.
pub fn prepare(to: &str, key: &str) -> Result<&'static str, String> {
    let dir = Path::new(to);
    if !dir.is_absolute() {
        return Err("Give the whole path of the folder, starting with /.".into());
    }
    if is_repository(dir) {
        open(to, key)?;
        return Ok("found");
    }
    if dir.exists() {
        let busy = std::fs::read_dir(dir)
            .map_err(|e| format!("Can't read that folder: {e}"))?
            .flatten()
            .any(|e| !e.file_name().to_string_lossy().starts_with('.'));
        if busy {
            return Err("That folder has other things in it. Choose an empty folder (or make a new one inside it) for the backups.".into());
        }
    } else {
        if !dir.parent().is_some_and(Path::is_dir) {
            return Err("That folder doesn't exist, and neither does the one it would go in.".into());
        }
        std::fs::create_dir(dir).map_err(|e| format!("Can't make that folder: {e}"))?;
    }
    // Can the server write there? (A read-only disk fails here, not at 3 am.)
    let probe = dir.join(".starbase-write-test");
    std::fs::write(&probe, b"ok").map_err(|e| format!("The server can't write in that folder: {e}"))?;
    let _ = std::fs::remove_file(&probe);
    repo(to)?.init(&Credentials::Password(key.to_string()), &KeyOptions::default(), &ConfigOptions::default()).map_err(|e| e.to_string())?;
    Ok("made")
}

/// Makes one backup of `root` now, then thins out old ones. Returns a short summary for the log.
/// Only one runs at a time; a second call while one runs returns an error at once.
pub fn run(base: &Path, root: &Path, why: &str) -> Result<String, String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("A backup is running already.".into());
    }
    let started = jiff::Timestamp::now();
    update(base, |s| {
        s.insert("started".into(), json!(started.to_string()));
    })
    .ok();
    let r = run_inner(base, root, why);
    let finished = jiff::Timestamp::now();
    update(base, |s| {
        s.remove("started");
        let mut last =
            json!({ "when": finished.to_string(), "ok": r.is_ok(), "message": r.as_ref().map_or_else(|e| e.clone(), Clone::clone), "why": why });
        if r.is_ok() {
            s.insert("last_ok".into(), json!(finished.to_string()));
        } else {
            last["seconds"] = json!((finished - started).get_seconds());
        }
        s.insert("last".into(), last);
    })
    .ok();
    RUNNING.store(false, Ordering::SeqCst);
    r
}

fn run_inner(base: &Path, root: &Path, why: &str) -> Result<String, String> {
    let s = load(base);
    let to = s.get("to").and_then(Value::as_str).filter(|t| !t.is_empty()).ok_or("Backups aren't set up yet.")?.to_string();
    let key = s.get("key").and_then(Value::as_str).ok_or("There is no recovery key.")?.to_string();
    if !Path::new(&to).is_dir() {
        return Err(format!("The backup folder {to} isn't there (a disk not plugged in, or a share not mounted?)."));
    }
    let repo = open(&to, &key)?;
    let repo = repo.to_indexed_ids().map_err(|e| e.to_string())?;
    let mut opts = BackupOptions::default().as_path(PathBuf::from(AS_PATH));
    opts.excludes.globs = SKIP.iter().map(|g| (*g).to_string()).collect();
    let source = PathList::from_iter([root.to_path_buf()]).sanitize().map_err(|e| e.to_string())?;
    let snap = SnapshotOptions::default()
        .label("starbase".to_string())
        .host(HOST.to_string())
        .add_tags(&format!("starbase,{why}"))
        .and_then(|o| o.to_snapshot())
        .map_err(|e| e.to_string())?;
    let snap = repo.backup(&opts, &source, snap).map_err(|e| e.to_string())?;
    let (files, added) = snap.summary.as_ref().map_or((0, 0), |m| (m.total_files_processed, m.data_added_packed));
    let removed = thin_out(&repo, &s)?;
    Ok(format!(
        "Backed up {files} files ({} new){}",
        human(added),
        match removed {
            0 => String::new(),
            1 => "; 1 old backup removed".to_string(),
            n => format!("; {n} old backups removed"),
        }
    ))
}

/// Removes the backups the keep rules no longer want, and frees their space.
fn thin_out<S: rustic_core::Open>(repo: &Repository<S>, s: &Map<String, Value>) -> Result<usize, String> {
    let keep = KeepOptions::default().keep_daily(num(s, "keep_daily", 7) as i32).keep_weekly(num(s, "keep_weekly", 4) as i32).keep_monthly(num(
        s,
        "keep_monthly",
        12,
    )
        as i32);
    let snaps = repo.get_all_snapshots().map_err(|e| e.to_string())?;
    let decided = keep.apply(snaps, &jiff::Zoned::now()).map_err(|e| e.to_string())?;
    let gone: Vec<_> = decided.iter().filter(|f| !f.keep).map(|f| f.snapshot.id).collect();
    if gone.is_empty() {
        return Ok(0);
    }
    repo.delete_snapshots(&gone).map_err(|e| e.to_string())?;
    let opts = PruneOptions::default();
    let plan = repo.prune_plan(&opts).map_err(|e| e.to_string())?;
    repo.prune(&opts, plan).map_err(|e| e.to_string())?;
    Ok(gone.len())
}

pub fn human(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{bytes} bytes")
    }
}

/// The backups in the chosen folder, newest first: id, when, how many files, how big.
pub fn list(base: &Path) -> Result<Vec<Value>, String> {
    let s = load(base);
    let to = s.get("to").and_then(Value::as_str).filter(|t| !t.is_empty()).ok_or("Backups aren't set up yet.")?;
    let key = s.get("key").and_then(Value::as_str).unwrap_or("");
    if !Path::new(to).is_dir() {
        return Err(format!("The backup folder {to} isn't there (a disk not plugged in, or a share not mounted?)."));
    }
    let repo = open(to, key)?;
    let mut snaps = repo.get_all_snapshots().map_err(|e| e.to_string())?;
    snaps.sort_by(|a, b| b.time.cmp(&a.time));
    Ok(snaps
        .iter()
        .map(|sn| {
            let (files, size) = sn.summary.as_ref().map_or((0, 0), |m| (m.total_files_processed, m.total_bytes_processed));
            json!({
                "id": sn.id.to_string(),
                "short": sn.id.to_string().chars().take(8).collect::<String>(),
                "time": sn.time.timestamp().to_string(),
                // For names people read: "2026-11-24 03.00" (UTC, no ":" for Windows and SMB).
                "label": sn.time.timestamp().strftime("%Y-%m-%d %H.%M").to_string(),
                "files": files,
                "size": human(size),
            })
        })
        .collect())
}

/// What a person sees when something inside rustic goes wrong: never its own words (they can say
/// "please report this bug"). The details go to the server's log.
fn unreadable(what: &str, e: impl std::fmt::Display) -> String {
    eprintln!("backup: {what}: {e}");
    "That backup couldn't be read. The server's log has the details.".to_string()
}

/// The files folder (`/starbase`) inside backup `id`, and the names of the folders at its top
/// (the server's own `.` folders left out).
fn top<S: IndexedTree>(repo: &Repository<S>, id: &str) -> Result<(Node, Vec<String>), String> {
    let node = repo.node_from_snapshot_path(&format!("{id}:{AS_PATH}"), |_| true).map_err(|e| unreadable(id, e))?;
    let Some(tree) = node.subtree else { return Ok((node, Vec::new())) };
    let tree = repo.get_tree(&tree).map_err(|e| unreadable(id, e))?;
    let names = tree.nodes.iter().filter(|n| n.is_dir()).map(|n| n.name().to_string_lossy().into_owned()).filter(|n| !n.starts_with('.')).collect();
    Ok((node, names))
}

/// The folders at the top of the files folder in any backup kept (with sign-in on, one per
/// person, including people whose accounts are gone), sorted.
pub fn people(base: &Path) -> Result<Vec<String>, String> {
    let s = load(base);
    let to = s.get("to").and_then(Value::as_str).filter(|t| !t.is_empty()).ok_or("Backups aren't set up yet.")?;
    let key = s.get("key").and_then(Value::as_str).unwrap_or("");
    let repo = open(to, key)?.to_indexed_ids().map_err(|e| unreadable("index", e))?;
    let snaps = repo.get_all_snapshots().map_err(|e| unreadable("snapshots", e))?;
    let mut all = std::collections::BTreeSet::new();
    for sn in &snaps {
        all.extend(top(&repo, &sn.id.to_string())?.1);
    }
    Ok(all.into_iter().collect())
}

/// Whose folder a restore brings back. `asker` is who asks (None: sign-in is off), `whose` what
/// they chose (None: their own; Some("") everyone's). Only the server's owner (or anyone, with
/// sign-in off) may bring back someone else's files.
pub fn whose_folder(asker: Option<&str>, owner: bool, whose: Option<&str>) -> Result<String, String> {
    let whose = whose.map(str::trim);
    let chosen = match (asker, whose) {
        (None, w) => w.unwrap_or("").to_string(),
        (Some(me), None) => me.to_string(),
        (Some(me), Some(w)) if w == me => me.to_string(),
        (Some(_), Some(w)) if owner => w.to_string(),
        (Some(_), Some(_)) => return Err("You can only restore your own files.".into()),
    };
    if chosen.contains(['/', '\\']) || chosen.starts_with('.') {
        return Err("There's no folder of that name in the backups.".into());
    }
    Ok(chosen)
}

/// Where restored files go (a path in the files folder, not there yet), for backup `label`:
/// beside the person's own folder ("ann/Restored from backup <label>"); for someone with no
/// folder here (an account that's gone, or a server made again), or for everyone's files, into
/// the asker's own folder, named after whose they are. With sign-in off, at the top.
pub fn restore_into(label: &str, whose: &str, asker: Option<&str>, exists: impl Fn(&str) -> bool) -> String {
    let stem = format!("Restored from backup {label}");
    let (folder, stem) = match asker {
        _ if !whose.is_empty() && exists(whose) => (whose.to_string(), stem),
        Some(me) if whose.is_empty() => (me.to_string(), format!("{stem} (everyone)")),
        Some(me) => (me.to_string(), format!("{stem} ({whose})")),
        None if whose.is_empty() => (String::new(), stem),
        None => (String::new(), format!("{stem} ({whose})")),
    };
    let at = |name: &str| if folder.is_empty() { name.to_string() } else { format!("{folder}/{name}") };
    let mut name = stem.clone();
    let mut n = 2;
    while exists(&at(&name)) {
        name = format!("{stem} ({n})");
        n += 1;
    }
    at(&name)
}

/// Copies a folder out of backup `id` into `dest` (a new folder), without touching anything else.
/// `inside` is the folder within the files folder ("" = all of it, e.g. "james" = James's own).
/// All of it leaves out the server's own folders at the top (`.starsheets`: accounts, versions);
/// restic itself brings those back. Returns how many files came back.
pub fn restore(base: &Path, id: &str, inside: &str, dest: &Path) -> Result<u64, String> {
    let s = load(base);
    let to = s.get("to").and_then(Value::as_str).filter(|t| !t.is_empty()).ok_or("Backups aren't set up yet.")?;
    let key = s.get("key").and_then(Value::as_str).unwrap_or("");
    if dest.exists() {
        return Err("That folder exists already.".into());
    }
    let repo = open(to, key)?.to_indexed().map_err(|e| unreadable("index", e))?;
    let (node, names) = top(&repo, id)?;
    let node = if inside.is_empty() {
        node
    } else {
        // Looked up first: rustic's own answer for a missing folder is an internal error
        // (a new server whose owner has another name asked for a folder no backup has).
        let missing = || format!("That backup has no files for {inside}.");
        if !names.iter().any(|n| n == inside) {
            return Err(missing());
        }
        let tree = node.subtree.ok_or_else(missing)?;
        repo.node_from_path(tree, Path::new(inside)).map_err(|e| unreadable(inside, e))?
    };
    let all = inside.is_empty();
    let hidden = |p: &Path| p.components().next().is_some_and(|c| c.as_os_str().to_string_lossy().starts_with('.'));
    let ls = repo
        .ls(&node, &LsOptions::default())
        .map_err(|e| unreadable("list", e))?
        .filter(move |r| !all || r.as_ref().map_or(true, |(p, _)| !hidden(p)));
    let dest_s = dest.to_string_lossy().into_owned();
    let target = LocalDestination::new(&dest_s, true, !node.is_dir()).map_err(|e| format!("Can't make the folder for the restored files: {e}"))?;
    let opts = RestoreOptions::default();
    let plan = repo.prepare_restore(&opts, ls.clone(), &target, false).map_err(|e| unreadable("plan", e))?;
    let files = plan.stats.files.restore + plan.stats.files.unchanged + plan.stats.files.verified + plan.stats.files.modify;
    repo.restore(plan, &opts, ls, &target).map_err(|e| unreadable("restore", e))?;
    Ok(files)
}

/// Is a backup due? Once a day at the hour chosen (server time), and straight away if the last
/// good one is more than a day and a bit old (the server was off at that hour).
pub fn due(s: &Map<String, Value>, now: &jiff::Zoned) -> bool {
    if !s.get("on").and_then(Value::as_bool).unwrap_or(false) || s.get("to").and_then(Value::as_str).unwrap_or("").is_empty() {
        return false;
    }
    let hour = num(s, "hour", 3);
    let last = s.get("last_ok").and_then(Value::as_str).and_then(|t| t.parse::<jiff::Timestamp>().ok());
    // A failed run is tried again an hour later, not every minute.
    let last_try = s.get("last").and_then(|l| l.get("when")).and_then(Value::as_str).and_then(|t| t.parse::<jiff::Timestamp>().ok());
    if last_try.is_some_and(|t| (now.timestamp() - t).get_seconds() < 3600) {
        return false;
    }
    let Some(last) = last else { return true };
    let age = (now.timestamp() - last).get_seconds();
    (i64::from(now.hour()) == hour && age > 3 * 3600) || age > 26 * 3600
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("starbase-backup-test-{name}-{}", crate::store::random_hex(4)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Only one backup runs at a time (RUNNING), and tests run side by side: take turns.
    fn run_one(base: &Path, root: &Path) -> Result<String, String> {
        static TURN: Mutex<()> = Mutex::new(());
        let _t = TURN.lock().unwrap_or_else(|e| e.into_inner());
        run(base, root, "test")
    }

    #[test]
    fn keys_are_typed_back_in_any_form() {
        let k = new_key();
        assert_eq!(k.len(), 39);
        assert_eq!(clean_key(&k.to_uppercase().replace('-', " ")), k);
        assert_eq!(clean_key("  my restic password "), "my restic password");
    }

    #[test]
    fn back_up_then_restore_one_persons_folder() {
        let base = tmp("base");
        let root = base.join("files");
        std::fs::create_dir_all(root.join("james/Budget")).unwrap();
        std::fs::create_dir_all(root.join(".starsheets")).unwrap();
        std::fs::write(root.join("james/Budget/2026.xlsx"), b"spreadsheet bytes").unwrap();
        std::fs::write(root.join("james/half.tmp"), b"half a save").unwrap();
        std::fs::write(root.join(".starsheets/users.json"), b"{}").unwrap();
        std::fs::write(root.join(".starsheets-backup.json"), b"{\"key\": \"secret\"}").unwrap();
        let to = base.join("backups");
        let key = new_key();
        assert_eq!(prepare(to.to_str().unwrap(), &key).unwrap(), "made");
        assert_eq!(prepare(to.to_str().unwrap(), &key).unwrap(), "found");
        assert!(prepare(to.to_str().unwrap(), &new_key()).is_err(), "another key must not open it");
        update(&base, |s| {
            s.insert("to".into(), json!(to.to_str().unwrap()));
            s.insert("key".into(), json!(key));
        })
        .unwrap();
        let said = run_one(&base, &root).unwrap();
        assert!(said.starts_with("Backed up 2 files"), "{said}");
        // The file changes after the backup; the backup still has the old one.
        std::fs::write(root.join("james/Budget/2026.xlsx"), b"changed").unwrap();
        let snaps = list(&base).unwrap();
        assert_eq!(snaps.len(), 1);
        let id = snaps[0]["id"].as_str().unwrap();
        let dest = base.join("restored");
        let n = restore(&base, id, "james", &dest).unwrap();
        assert_eq!(n, 1);
        assert_eq!(std::fs::read(dest.join("Budget/2026.xlsx")).unwrap(), b"spreadsheet bytes");
        assert!(!dest.join("half.tmp").exists(), "half-written saves are skipped");
        let all = base.join("everything");
        restore(&base, id, "", &all).unwrap();
        assert!(all.join("james/Budget/2026.xlsx").is_file());
        assert!(!all.join(".starsheets").exists(), "accounts stay out of a folder people can open");
        assert!(!all.join(".starsheets-backup.json").exists(), "the key never goes into a backup");
        assert!(load(&base)["last"]["ok"].as_bool().unwrap());
        std::fs::remove_dir_all(&base).ok();
    }

    /// A server with two people (james the owner, ann), backed up once. Returns (base, root, key, id).
    fn two_people() -> (PathBuf, PathBuf, String, String) {
        let base = tmp("two");
        let root = base.join("files");
        for (who, file, bytes) in [("james", "Budget.xlsx", &b"james's sheet"[..]), ("ann", "Notes.docx", &b"ann's notes"[..])] {
            std::fs::create_dir_all(root.join(who)).unwrap();
            std::fs::write(root.join(who).join(file), bytes).unwrap();
        }
        std::fs::create_dir_all(root.join(".starsheets")).unwrap();
        std::fs::write(root.join(".starsheets/users.json"), b"{}").unwrap();
        let to = base.join("backups");
        let key = new_key();
        prepare(to.to_str().unwrap(), &key).unwrap();
        update(&base, |s| {
            s.insert("to".into(), json!(to.to_str().unwrap()));
            s.insert("key".into(), json!(key));
        })
        .unwrap();
        run_one(&base, &root).unwrap();
        let id = list(&base).unwrap()[0]["id"].as_str().unwrap().to_string();
        (base, root, key, id)
    }

    /// What the restore route does: whose files, where they go, then the copy.
    fn restore_as(base: &Path, root: &Path, id: &str, asker: &str, owner: bool, whose: Option<&str>) -> Result<(String, u64), String> {
        let whose = whose_folder(Some(asker), owner, whose)?;
        let rel = restore_into("2026-11-24 03.00", &whose, Some(asker), |p| root.join(p).exists());
        let n = restore(base, id, &whose, &root.join(&rel))?;
        Ok((rel, n))
    }

    #[test]
    fn restore_as_the_same_owner() {
        let (base, root, _, id) = two_people();
        let (rel, n) = restore_as(&base, &root, &id, "james", true, None).unwrap();
        assert_eq!((rel.as_str(), n), ("james/Restored from backup 2026-11-24 03.00", 1));
        assert_eq!(std::fs::read(root.join(&rel).join("Budget.xlsx")).unwrap(), b"james's sheet");
        // A second time: a new folder beside the first.
        let (rel2, _) = restore_as(&base, &root, &id, "james", true, None).unwrap();
        assert_eq!(rel2, "james/Restored from backup 2026-11-24 03.00 (2)");
        assert_eq!(people(&base).unwrap(), vec!["ann", "james"]);
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn restore_on_a_new_server_whose_owner_has_another_name() {
        let (old, _, key, id) = two_people();
        // The server is lost; a new one (owner "boss") opens the same backups with the key.
        let base = tmp("new");
        let root = base.join("files");
        std::fs::create_dir_all(root.join("boss")).unwrap();
        let to = old.join("backups");
        assert_eq!(prepare(to.to_str().unwrap(), &key).unwrap(), "found");
        update(&base, |s| {
            s.insert("to".into(), json!(to.to_str().unwrap()));
            s.insert("key".into(), json!(key));
        })
        .unwrap();
        // Their own folder isn't in the backup: a plain answer, not rustic's internal error.
        let e = restore_as(&base, &root, &id, "boss", true, None).unwrap_err();
        assert_eq!(e, "That backup has no files for boss.");
        // james's account is gone: his files come back into boss's folder, named after him.
        let (rel, n) = restore_as(&base, &root, &id, "boss", true, Some("james")).unwrap();
        assert_eq!((rel.as_str(), n), ("boss/Restored from backup 2026-11-24 03.00 (james)", 1));
        assert_eq!(std::fs::read(root.join(&rel).join("Budget.xlsx")).unwrap(), b"james's sheet");
        // Everyone's: one folder each, without the server's own (accounts).
        let (rel, n) = restore_as(&base, &root, &id, "boss", true, Some("")).unwrap();
        assert_eq!((rel.as_str(), n), ("boss/Restored from backup 2026-11-24 03.00 (everyone)", 2));
        assert!(root.join(&rel).join("ann/Notes.docx").is_file());
        assert!(!root.join(&rel).join(".starsheets").exists());
        std::fs::remove_dir_all(&base).ok();
        std::fs::remove_dir_all(&old).ok();
    }

    #[test]
    fn owner_restores_another_persons_folder_beside_it() {
        let (base, root, _, id) = two_people();
        std::fs::write(root.join("ann/Notes.docx"), b"changed since").unwrap();
        let (rel, n) = restore_as(&base, &root, &id, "james", true, Some("ann")).unwrap();
        assert_eq!((rel.as_str(), n), ("ann/Restored from backup 2026-11-24 03.00", 1));
        assert_eq!(std::fs::read(root.join(&rel).join("Notes.docx")).unwrap(), b"ann's notes");
        assert_eq!(std::fs::read(root.join("ann/Notes.docx")).unwrap(), b"changed since", "nothing is overwritten");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn others_only_restore_their_own() {
        let (base, root, _, id) = two_people();
        assert_eq!(restore_as(&base, &root, &id, "ann", false, Some("james")).unwrap_err(), "You can only restore your own files.");
        assert_eq!(restore_as(&base, &root, &id, "ann", false, Some("")).unwrap_err(), "You can only restore your own files.");
        assert!(!root.join("james").read_dir().unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("Restored")));
        let (rel, _) = restore_as(&base, &root, &id, "ann", false, Some("ann")).unwrap();
        assert_eq!(rel, "ann/Restored from backup 2026-11-24 03.00");
        assert!(whose_folder(Some("james"), true, Some("../etc")).is_err());
        assert!(whose_folder(Some("james"), true, Some(".starsheets")).is_err());
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_folder_with_other_things_is_refused() {
        let d = tmp("busy");
        std::fs::write(d.join("holiday.jpg"), b"x").unwrap();
        assert!(prepare(d.to_str().unwrap(), &new_key()).is_err());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn due_once_a_day_at_the_hour() {
        let at = |s: &str| s.parse::<jiff::Zoned>().unwrap();
        let mut s = Map::new();
        s.insert("on".into(), json!(true));
        s.insert("to".into(), json!("/backups"));
        s.insert("hour".into(), json!(3));
        assert!(due(&s, &at("2026-11-24T10:00:00+00:00[UTC]")), "never backed up: now");
        s.insert("last_ok".into(), json!("2026-11-24T03:01:00Z"));
        assert!(!due(&s, &at("2026-11-24T10:00:00+00:00[UTC]")));
        assert!(!due(&s, &at("2026-11-24T03:30:00+00:00[UTC]")), "already done this hour");
        assert!(due(&s, &at("2026-11-25T03:00:00+00:00[UTC]")));
        assert!(due(&s, &at("2026-11-26T09:00:00+00:00[UTC]")), "missed one: catch up");
        s.insert("last".into(), json!({ "when": "2026-11-26T08:30:00Z", "ok": false }));
        assert!(!due(&s, &at("2026-11-26T09:00:00+00:00[UTC]")), "failed 30 min ago: wait an hour");
    }

    #[test]
    fn time_zones_work_without_the_systems_list() {
        // Slim Docker images have no /usr/share/zoneinfo: the built-in copy still knows TZ names.
        let db = jiff::tz::TimeZoneDatabase::bundled();
        let london = db.get("Europe/London").expect("Europe/London from the built-in list");
        assert_eq!(london.iana_name(), Some("Europe/London"));
        let summer = "2026-07-01T12:00:00Z".parse::<jiff::Timestamp>().unwrap().to_zoned(london);
        assert_eq!(summer.hour(), 13, "British Summer Time");
    }
}
