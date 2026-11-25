//! The photo index: one SQLite file beside the server's own state (never in people's folders).
//!
//! It is derived and disposable: deleting it only means every picture is read again. The schema
//! has a version (`PRAGMA user_version`) and steps from one version to the next in [`migrate`];
//! each row has a stable integer `id` (kept when a file is renamed or moved), which the later
//! tables for faces, places, tags and albums (Part 2) will point at.

use std::collections::HashSet;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::decode::Meta;

/// The schema version this server writes.
pub const SCHEMA: i32 = 1;

/// Where a picture is in the work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Found, not read yet (or changed since).
    Pending = 0,
    /// Read: thumbnails and EXIF are there.
    Ready = 1,
    /// Couldn't be read (the row's `error` says why); tried again when the file changes.
    Failed = 2,
    /// A kind of picture this server can't read yet (HEIC, RAW, AVIF…): listed, no thumbnails.
    Unsupported = 3,
}

impl State {
    fn from(n: i64) -> State {
        match n {
            1 => State::Ready,
            2 => State::Failed,
            3 => State::Unsupported,
            _ => State::Pending,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            State::Pending => "pending",
            State::Ready => "ready",
            State::Failed => "failed",
            State::Unsupported => "unsupported",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    pub id: i64,
    pub path: String,
    pub folder: String,
    pub name: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub media: String,
    pub hash: Option<String>,
    pub state: State,
    pub error: Option<String>,
    pub meta: Meta,
}

pub struct Index {
    conn: Connection,
}

const COLUMNS: &str =
    "id, path, folder, name, size, mtime_ns, media, hash, state, error, width, height, orientation, taken, make, model, lat, lon, alt";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        path: r.get(1)?,
        folder: r.get(2)?,
        name: r.get(3)?,
        size: r.get::<_, i64>(4)? as u64,
        mtime_ns: r.get(5)?,
        media: r.get(6)?,
        hash: r.get(7)?,
        state: State::from(r.get(8)?),
        error: r.get(9)?,
        meta: Meta {
            width: r.get::<_, Option<u32>>(10)?.unwrap_or(0),
            height: r.get::<_, Option<u32>>(11)?.unwrap_or(0),
            orientation: r.get::<_, Option<u16>>(12)?.unwrap_or(1),
            taken: r.get(13)?,
            make: r.get(14)?,
            model: r.get(15)?,
            lat: r.get(16)?,
            lon: r.get(17)?,
            alt: r.get(18)?,
        },
    })
}

/// The folder part and the name part of a path inside the store.
pub fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// Steps the schema up to [`SCHEMA`]. A newer schema than this server knows (a downgrade) is
/// started afresh: the index is only a cache.
fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let mut v: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if v > SCHEMA {
        conn.execute_batch("DROP TABLE IF EXISTS photos; PRAGMA user_version = 0;")?;
        v = 0;
    }
    if v < 1 {
        conn.execute_batch(
            "BEGIN;
            CREATE TABLE photos(
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                folder TEXT NOT NULL,
                name TEXT NOT NULL,
                size INTEGER NOT NULL,
                mtime_ns INTEGER NOT NULL,
                media TEXT NOT NULL,
                hash TEXT,
                state INTEGER NOT NULL DEFAULT 0,
                error TEXT,
                render TEXT,
                width INTEGER, height INTEGER, orientation INTEGER,
                taken TEXT, make TEXT, model TEXT,
                lat REAL, lon REAL, alt REAL,
                indexed_at INTEGER
            );
            CREATE INDEX photos_folder ON photos(folder);
            CREATE INDEX photos_hash ON photos(hash);
            CREATE INDEX photos_state ON photos(state);
            CREATE INDEX photos_taken ON photos(taken);
            PRAGMA user_version = 1;
            COMMIT;",
        )?;
    }
    Ok(())
}

impl Index {
    /// Opens (or makes) the index at `path`. Only this server uses it, so it holds the file
    /// for itself (`locking_mode=EXCLUSIVE`): then the write-ahead log needs no shared-memory
    /// file, which network file systems (a store on a NAS share) don't do reliably.
    pub fn open(path: &Path) -> rusqlite::Result<Index> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate(&conn)?;
        Ok(Index { conn })
    }

    /// Pictures made with another renderer (an older server) are made again; failed ones too,
    /// since a newer server may read them.
    pub fn renderer_changed(&self, render: &str) -> rusqlite::Result<usize> {
        self.conn.execute("UPDATE photos SET state = 0 WHERE state IN (1, 2) AND (render IS NULL OR render != ?1)", params![render])
    }

    pub fn get(&self, path: &str) -> rusqlite::Result<Option<Row>> {
        self.conn.query_row(&format!("SELECT {COLUMNS} FROM photos WHERE path = ?1"), params![path], row).optional()
    }

    /// The pictures in `folder` ("" = the top), and in every folder inside it with `deep`.
    pub fn in_folder(&self, folder: &str, deep: bool) -> rusqlite::Result<Vec<Row>> {
        if !deep {
            let mut s = self.conn.prepare_cached(&format!("SELECT {COLUMNS} FROM photos WHERE folder = ?1"))?;
            return s.query_map(params![folder], row)?.collect();
        }
        if folder.is_empty() {
            let mut s = self.conn.prepare_cached(&format!("SELECT {COLUMNS} FROM photos"))?;
            return s.query_map([], row)?.collect();
        }
        // "a/b/" up to "a/b0" ('0' comes right after '/'): every path inside, using the index.
        let mut s = self.conn.prepare_cached(&format!("SELECT {COLUMNS} FROM photos WHERE folder = ?1 OR (folder >= ?2 AND folder < ?3)"))?;
        s.query_map(params![folder, format!("{folder}/"), format!("{folder}0")], row)?.collect()
    }

    /// Notes a file as found with this size and modified time. A new file, or one whose size or
    /// time changed, is (again) waiting to be read. Returns true when it now waits.
    pub fn found(&self, path: &str, size: u64, mtime_ns: i64, media: &str, supported: bool) -> rusqlite::Result<bool> {
        let want = if supported { State::Pending } else { State::Unsupported } as i64;
        let (folder, name) = split(path);
        let had: Option<(i64, i64, i64)> = self
            .conn
            .query_row("SELECT size, mtime_ns, state FROM photos WHERE path = ?1", params![path], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        match had {
            None => {
                self.conn.execute(
                    "INSERT INTO photos(path, folder, name, size, mtime_ns, media, state) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![path, folder, name, size as i64, mtime_ns, media, want],
                )?;
                Ok(supported)
            }
            Some((s, t, st)) if s == size as i64 && t == mtime_ns && (st == want || (supported && st != State::Unsupported as i64)) => {
                Ok(st == State::Pending as i64)
            }
            Some(_) => {
                self.conn.execute(
                    "UPDATE photos SET size = ?2, mtime_ns = ?3, media = ?4, state = ?5, hash = NULL, error = NULL, render = NULL,
                     width = NULL, height = NULL, orientation = NULL, taken = NULL, make = NULL, model = NULL, lat = NULL, lon = NULL, alt = NULL
                     WHERE path = ?1",
                    params![path, size as i64, mtime_ns, media, want],
                )?;
                Ok(supported)
            }
        }
    }

    /// The picture at `path` was read: kept only if the file is still the one read.
    pub fn done(&self, path: &str, size: u64, mtime_ns: i64, hash: &str, meta: &Meta, render: &str) -> rusqlite::Result<bool> {
        let n = self.conn.execute(
            "UPDATE photos SET state = 1, hash = ?4, error = NULL, render = ?5, width = ?6, height = ?7, orientation = ?8, taken = ?9,
             make = ?10, model = ?11, lat = ?12, lon = ?13, alt = ?14, indexed_at = strftime('%s','now')
             WHERE path = ?1 AND size = ?2 AND mtime_ns = ?3",
            params![
                path,
                size as i64,
                mtime_ns,
                hash,
                render,
                meta.width,
                meta.height,
                meta.orientation,
                meta.taken,
                meta.make,
                meta.model,
                meta.lat,
                meta.lon,
                meta.alt
            ],
        )?;
        Ok(n > 0)
    }

    pub fn failed(&self, path: &str, size: u64, mtime_ns: i64, hash: Option<&str>, why: &str, render: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE photos SET state = 2, hash = ?4, error = ?5, render = ?6, indexed_at = strftime('%s','now') WHERE path = ?1 AND size = ?2 AND mtime_ns = ?3",
            params![path, size as i64, mtime_ns, hash, why, render],
        )?;
        Ok(())
    }

    /// The EXIF of another file with the same content, already read (a copy, or a file moved
    /// in a way that looked like a new one).
    pub fn meta_of_hash(&self, hash: &str) -> rusqlite::Result<Option<Meta>> {
        self.conn
            .query_row(&format!("SELECT {COLUMNS} FROM photos WHERE hash = ?1 AND state = 1 LIMIT 1"), params![hash], row)
            .optional()
            .map(|r| r.map(|r| r.meta))
    }

    pub fn remove(&self, path: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM photos WHERE path = ?1", params![path])?;
        Ok(())
    }

    /// A file moved or renamed: its row (id, hash, EXIF) goes with it.
    pub fn moved(&self, from: &str, to: &str) -> rusqlite::Result<()> {
        let (folder, name) = split(to);
        self.conn.execute("DELETE FROM photos WHERE path = ?1", params![to])?;
        self.conn.execute("UPDATE photos SET path = ?2, folder = ?3, name = ?4 WHERE path = ?1", params![from, to, folder, name])?;
        Ok(())
    }

    pub fn pending(&self) -> rusqlite::Result<Vec<String>> {
        let mut s = self.conn.prepare("SELECT path FROM photos WHERE state = 0 ORDER BY folder, name")?;
        s.query_map([], |r| r.get(0))?.collect()
    }

    /// How many pictures are in each state: [pending, ready, failed, unsupported].
    pub fn counts(&self) -> rusqlite::Result<[u64; 4]> {
        let mut out = [0u64; 4];
        let mut s = self.conn.prepare("SELECT state, COUNT(*) FROM photos GROUP BY state")?;
        for r in s.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (st, n) = r?;
            if let Some(slot) = out.get_mut(st as usize) {
                *slot = n as u64;
            }
        }
        Ok(out)
    }

    /// Every content hash in use (cache files for any other one can go).
    pub fn hashes(&self) -> rusqlite::Result<HashSet<String>> {
        let mut s = self.conn.prepare("SELECT DISTINCT hash FROM photos WHERE hash IS NOT NULL")?;
        s.query_map([], |r| r.get(0))?.collect()
    }

    /// Runs `f` in one transaction (a folder's worth of changes at once is far quicker).
    pub fn batch<T>(&mut self, f: impl FnOnce(&Index) -> rusqlite::Result<T>) -> rusqlite::Result<T> {
        self.conn.execute_batch("BEGIN")?;
        match f(self) {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_follow_the_files() {
        let dir = std::env::temp_dir().join(format!("starbase-photo-index-{}", crate::store::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let ix = Index::open(&dir.join("index.sqlite")).unwrap();
        assert!(ix.found("ann/Trip/a.jpg", 10, 1, "image/jpeg", true).unwrap(), "new: waits");
        assert!(!ix.found("ann/Trip/raw.cr2", 10, 1, "image/x-canon-cr2", false).unwrap(), "unsupported: doesn't wait");
        assert!(ix.found("ann/Trip/a.jpg", 10, 1, "image/jpeg", true).unwrap(), "still waiting");
        let meta = Meta { width: 4, height: 3, orientation: 6, taken: Some("2026-01-02T03:04:05".into()), ..Meta::default() };
        assert!(!ix.done("ann/Trip/a.jpg", 11, 1, "h1", &meta, "r1").unwrap(), "a changed file's result is dropped");
        assert!(ix.done("ann/Trip/a.jpg", 10, 1, "h1", &meta, "r1").unwrap());
        assert!(!ix.found("ann/Trip/a.jpg", 10, 1, "image/jpeg", true).unwrap(), "unchanged: nothing to do");
        let id = ix.get("ann/Trip/a.jpg").unwrap().unwrap().id;
        ix.moved("ann/Trip/a.jpg", "ann/Trip/Best/a.jpg").unwrap();
        let r = ix.get("ann/Trip/Best/a.jpg").unwrap().unwrap();
        assert_eq!((r.id, r.folder.as_str(), r.state, r.meta.clone()), (id, "ann/Trip/Best", State::Ready, meta), "moved with its id");
        assert_eq!(ix.in_folder("ann/Trip", false).unwrap().len(), 1);
        assert_eq!(ix.in_folder("ann/Trip", true).unwrap().len(), 2);
        assert_eq!(ix.in_folder("ann/Tri", true).unwrap().len(), 0, "a prefix of a name isn't a folder");
        assert_eq!(ix.counts().unwrap(), [0, 1, 0, 1]);
        assert!(ix.found("ann/Trip/Best/a.jpg", 12, 2, "image/jpeg", true).unwrap(), "changed: waits again");
        assert_eq!(ix.get("ann/Trip/Best/a.jpg").unwrap().unwrap().hash, None);
        assert_eq!(ix.renderer_changed("r2").unwrap(), 0);
        drop(ix);
        let ix = Index::open(&dir.join("index.sqlite")).unwrap();
        assert_eq!(ix.pending().unwrap(), vec!["ann/Trip/Best/a.jpg".to_string()], "kept across restarts");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
