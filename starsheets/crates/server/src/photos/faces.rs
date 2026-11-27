//! Faces and people (Photos roadmap items 7 and 8): the tables in the photo index, the grouping
//! of faces into people, and the changes people make (name, merge, remove a wrong face).
//!
//! - **Where**: in the photo index (`index.sqlite` in the cache folder), never in anyone's folder.
//!   Faces are kept by the picture's thumbnail key (its content, with its edits), so a renamed,
//!   moved or copied picture keeps them. Each face has its box (0..1 of the thumbnail), the
//!   detector's score, its size in pixels, the model that made its embedding and the embedding.
//! - **Grouping** ([`regroup`]): faces are first matched against named people (the most similar
//!   of a person's faces; a face someone removed from a person is never matched to them again):
//!   at or above the model's `auto` ("sure") similarity they join the person; at or above
//!   `suggest` ("maybe") they don't join: People asks "Same person?" ([`answer`]), and a "No"
//!   is kept as "not this person" like a removed face. The rest are grouped by **average linkage** (not DBSCAN: the face trial of
//!   2026-11 found DBSCAN chains people together) and cut at the model's `group_cut` distance.
//!   Only good faces (score and size, see [`good`]) start or join a group.
//! - **Thresholds** ([`thresholds`]): the trial's (2026-11) group cuts, AuraFace 0.75 and SFace
//!   0.67. Its join line (AuraFace 0.45, SFace 0.54) blended two friends into one person
//!   (James 2026-11-27), so that line is now only "maybe" (ask), and "sure" is 0.10 above it.
//! - **Removing a face from a person** makes it a new face again: its own unnamed group (it can
//!   be named, merged or grouped with others), and never that person again (`face_not`).
//! - **Ignored and "not a face"** ([`mark`], `hidden` [`IGNORED`] / [`NOT_FACE`]): strangers and
//!   dolls leave People, suggestions, questions and grouping, but nothing is deleted: [`unmark`]
//!   puts them back (with the group they were in), [`assign`] gives them to a person, and
//!   [`keep`] carries the mark over to the same box when a picture is looked at again, so a
//!   re-scan never brings a doll back.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};

use super::index::Index;

/// The faces tables (index schema version 3).
pub const SCHEMA_V3: &str = "BEGIN;
    CREATE TABLE IF NOT EXISTS faces(
        id INTEGER PRIMARY KEY,
        thumb TEXT NOT NULL,
        model TEXT NOT NULL,
        x REAL NOT NULL, y REAL NOT NULL, w REAL NOT NULL, h REAL NOT NULL,
        score REAL NOT NULL,
        px REAL NOT NULL,
        emb BLOB NOT NULL,
        -- The person it belongs to (people.id), whether that was the matcher's doing (auto = 1),
        -- the person offered, and the group of unnamed faces (the smallest face id in it).
        person INTEGER, auto INTEGER NOT NULL DEFAULT 0, suggest INTEGER, grp INTEGER,
        -- 0: normal; 2: taken out of its group by someone (never grouped again, may still be
        -- matched to a named person); 3: ignored (a stranger); 4: not a face. 3 and 4 keep `grp`
        -- (their old group, so undoing brings the group back) and are in no list but their own.
        hidden INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX IF NOT EXISTS faces_thumb ON faces(thumb);
    CREATE INDEX IF NOT EXISTS faces_person ON faces(person);
    CREATE INDEX IF NOT EXISTS faces_grp ON faces(grp);
    -- Pictures looked at by a model (with or without faces): the worker's progress, so it resumes.
    CREATE TABLE IF NOT EXISTS faces_done(thumb TEXT NOT NULL, model TEXT NOT NULL, PRIMARY KEY(thumb, model));
    CREATE TABLE IF NOT EXISTS people(id INTEGER PRIMARY KEY, name TEXT NOT NULL, created INTEGER NOT NULL);
    -- \"Not this person\": never matched again.
    CREATE TABLE IF NOT EXISTS face_not(face INTEGER NOT NULL, person INTEGER NOT NULL, PRIMARY KEY(face, person));
    CREATE TABLE IF NOT EXISTS settings(k TEXT PRIMARY KEY, v TEXT NOT NULL);
    PRAGMA user_version = 3;
    COMMIT;";

/// Names are per account (index schema version 4): `people.name` is no longer used. Everyone
/// names the people in their own pictures; a person in a picture two accounts can see has a
/// name for each of them (or none), never one shared name.
pub const SCHEMA_V4: &str = "BEGIN;
    CREATE TABLE IF NOT EXISTS person_names(person INTEGER NOT NULL, user TEXT NOT NULL, name TEXT NOT NULL, PRIMARY KEY(person, user));
    PRAGMA user_version = 4;
    COMMIT;";

/// Similarities and distances (cosine) for one embedding model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    /// Average-linkage cut: faces closer than this (1 − similarity) form a group.
    pub group_cut: f32,
    /// "Maybe": at or above this similarity (and below `auto`) People asks "Same person?".
    pub suggest: f32,
    /// "Sure": put the face with the person without asking at or above this similarity.
    pub auto: f32,
}

/// The thresholds of a model. The group cuts are the face trial's (2026-11, RESULTS.md). Its
/// join line (AuraFace 0.45, SFace 0.54) still joined look-alike friends on real pictures
/// (2026-11-27), so it became the "maybe" line (ask first), and "sure" is 0.10 above it: in the
/// trial nearly all same-person matches of good faces scored well above that, so sure matches
/// still join on their own while the borderline band is asked about. The trial's old offer
/// line (0.35 / 0.43) is dropped: too many wrong questions.
pub fn thresholds(model: &str) -> Thresholds {
    match model {
        "sface" => Thresholds { group_cut: 0.67, suggest: 0.54, auto: 0.64 },
        _ => Thresholds { group_cut: 0.75, suggest: 0.45, auto: 0.55 },
    }
}

/// `faces.hidden`: ignored by someone (a stranger): out of People until they undo it.
pub const IGNORED: i64 = 3;
/// `faces.hidden`: "not a face" (a doll, a poster): out of People until someone says it is one.
pub const NOT_FACE: i64 = 4;
/// SQL: a face People shows and groups (not ignored, not "not a face").
const SHOWN: &str = "hidden IN (0, 2)";

/// How much two boxes (x, y, w, h) overlap: intersection over union, 0..1.
pub fn overlap(a: [f32; 4], b: [f32; 4]) -> f32 {
    let iw = ((a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0])).max(0.0);
    let ih = ((a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1])).max(0.0);
    let i = iw * ih;
    let u = a[2] * a[3] + b[2] * b[3] - i;
    if u <= 0.0 { 0.0 } else { i / u }
}

/// Boxes at least this alike ([`overlap`]) are the same face when a picture is looked at again.
pub const SAME_BOX: f32 = 0.5;

/// The most unnamed faces grouped in one pass (average linkage needs n² memory: 3,000 faces
/// is 36 MB). The newest faces go first; the rest wait for a later pass.
pub const MAX_GROUP: usize = 3000;
/// The most faces of one person compared with a new face.
const PROTOTYPES: usize = 64;

/// A face good enough to start or join a group: a confident detection, not tiny. Weaker faces
/// can still be matched to a named person.
pub fn good(score: f32, px: f32) -> bool {
    score >= 0.8 && px >= 28.0
}

/// Cosine similarity.
pub fn similarity(a: &[f32], b: &[f32]) -> f32 {
    let (mut d, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        d += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 { 0.0 } else { d / (na.sqrt() * nb.sqrt()) }
}

/// Average-linkage (UPGMA) grouping of `embs`, cut at distance `cut` (1 − cosine similarity):
/// a group label for each item (labels are 0.. in no particular order). Uses the
/// nearest-neighbour chain, O(n²) time and memory.
pub fn average_linkage(embs: &[Vec<f32>], cut: f32) -> Vec<usize> {
    let n = embs.len();
    if n == 0 {
        return Vec::new();
    }
    let mut d = vec![0f32; n * n];
    for i in 0..n {
        for j in i + 1..n {
            let v = 1.0 - similarity(&embs[i], &embs[j]);
            d[i * n + j] = v;
            d[j * n + i] = v;
        }
    }
    let mut size = vec![1usize; n];
    let mut active = vec![true; n];
    let mut left = n;
    let mut chain: Vec<usize> = Vec::new();
    let mut merges: Vec<(usize, usize, f32)> = Vec::with_capacity(n - 1);
    while left > 1 {
        if chain.is_empty() {
            chain.push(active.iter().position(|a| *a).unwrap_or(0));
        }
        let (a, b) = loop {
            let a = chain[chain.len() - 1];
            let prev = if chain.len() >= 2 { Some(chain[chain.len() - 2]) } else { None };
            // Ties go to the previous link, so the chain always ends.
            let mut best = prev.map(|p| (p, d[a * n + p]));
            for k in 0..n {
                if k != a && active[k] && best.is_none_or(|(_, bd)| d[a * n + k] < bd) {
                    best = Some((k, d[a * n + k]));
                }
            }
            let (b, _) = best.unwrap_or((a, 0.0));
            if Some(b) == prev {
                chain.truncate(chain.len() - 2);
                break (a, b);
            }
            chain.push(b);
        };
        merges.push((a, b, d[a * n + b]));
        let (sa, sb) = (size[a] as f32, size[b] as f32);
        for k in 0..n {
            if active[k] && k != a && k != b {
                let v = (sa * d[a * n + k] + sb * d[b * n + k]) / (sa + sb);
                d[a * n + k] = v;
                d[k * n + a] = v;
            }
        }
        size[a] += size[b];
        active[b] = false;
        left -= 1;
    }
    // Average linkage is monotone: the merges at or under the cut are exactly the flat groups.
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for (a, b, dist) in merges {
        if dist <= cut {
            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
            parent[rb] = ra;
        }
    }
    let mut label: HashMap<usize, usize> = HashMap::new();
    (0..n)
        .map(|i| {
            let r = find(&mut parent, i);
            let next = label.len();
            *label.entry(r).or_insert(next)
        })
        .collect()
}

/// What a face matches among named people: `(person, similarity)` of the best one, leaving out
/// the people it was taken away from.
pub fn best_person(emb: &[f32], people: &HashMap<i64, Vec<Vec<f32>>>, not: &HashSet<i64>) -> Option<(i64, f32)> {
    let mut best: Option<(i64, f32)> = None;
    for (p, protos) in people {
        if not.contains(p) {
            continue;
        }
        let s = protos.iter().map(|q| similarity(emb, q)).fold(f32::MIN, f32::max);
        if best.is_none_or(|(_, bs)| s > bs) {
            best = Some((*p, s));
        }
    }
    best
}

pub fn emb_to_blob(e: &[f32]) -> Vec<u8> {
    e.iter().flat_map(|f| f.to_le_bytes()).collect()
}

pub fn blob_to_emb(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

/// A face found by the worker, in thumbnail coordinates 0..1.
#[derive(Clone, Debug)]
pub struct Found {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub score: f32,
    pub px: f32,
    pub emb: Vec<f32>,
}

/// One face as the People view needs it.
#[derive(Clone, Debug)]
pub struct Face {
    pub id: i64,
    pub thumb: String,
    pub bx: [f32; 4],
    pub person: Option<i64>,
    pub grp: Option<i64>,
    pub suggest: Option<i64>,
    /// `faces.hidden` (0 or 2 shown, [`IGNORED`], [`NOT_FACE`]).
    pub state: i64,
}

fn conn(ix: &Index) -> &Connection {
    ix.conn()
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// A setting kept in the index.
pub fn setting(ix: &Index, k: &str) -> Option<String> {
    conn(ix).query_row("SELECT v FROM settings WHERE k = ?1", [k], |r| r.get(0)).optional().ok().flatten()
}

pub fn set_setting(ix: &Index, k: &str, v: &str) -> rusqlite::Result<()> {
    conn(ix).execute("INSERT INTO settings(k, v) VALUES(?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v", params![k, v])?;
    Ok(())
}

/// The next thumbnails the worker should look at for `model`: ready pictures not done yet.
pub fn todo(ix: &Index, model: &str, n: usize) -> rusqlite::Result<Vec<String>> {
    let mut st = conn(ix).prepare(
        "SELECT DISTINCT COALESCE(p.thumb, p.hash) AS t FROM photos p
         WHERE p.state = 1 AND COALESCE(p.thumb, p.hash) IS NOT NULL
           AND NOT EXISTS (SELECT 1 FROM faces_done d WHERE d.thumb = COALESCE(p.thumb, p.hash) AND d.model = ?1)
         ORDER BY p.taken DESC LIMIT ?2",
    )?;
    st.query_map(params![model, n as i64], |r| r.get(0))?.collect()
}

/// Every ready picture: (thumbnail key, path).
pub fn thumb_paths(ix: &Index) -> rusqlite::Result<Vec<(String, String)>> {
    let mut st = conn(ix).prepare("SELECT COALESCE(thumb, hash), path FROM photos WHERE state = 1 AND COALESCE(thumb, hash) IS NOT NULL")?;
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
}

/// (looked at, all ready pictures, faces) for `model`.
pub fn progress(ix: &Index, model: &str) -> (u64, u64, u64) {
    let c = conn(ix);
    let q = |sql: &str| -> u64 { c.query_row(sql, [model], |r| r.get::<_, i64>(0)).unwrap_or(0) as u64 };
    let done = q("SELECT COUNT(*) FROM faces_done WHERE model = ?1");
    let all = c.query_row("SELECT COUNT(DISTINCT COALESCE(thumb, hash)) FROM photos WHERE state = 1", [], |r| r.get::<_, i64>(0)).unwrap_or(0) as u64;
    let faces = q("SELECT COUNT(*) FROM faces WHERE model = ?1");
    (done.min(all), all, faces)
}

/// Keeps the faces found in thumbnail `thumb` (replacing any earlier ones of that model).
pub fn keep(ix: &Index, thumb: &str, model: &str, faces: &[Found]) -> rusqlite::Result<()> {
    let c = conn(ix);
    c.execute_batch("SAVEPOINT keep")?;
    let r = (|| {
        // Ignored and "not a face" boxes are remembered: the same box found again keeps its mark.
        let mut st = c.prepare("SELECT x, y, w, h, hidden FROM faces WHERE thumb = ?1 AND model = ?2 AND hidden IN (3, 4)")?;
        let marked: Vec<([f32; 4], i64)> = st
            .query_map(params![thumb, model], |r| Ok(([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?], r.get(4)?)))?
            .collect::<rusqlite::Result<_>>()?;
        c.execute("DELETE FROM faces WHERE thumb = ?1 AND model = ?2", params![thumb, model])?;
        for f in faces {
            let bx = [f.x, f.y, f.w, f.h];
            let hidden = marked
                .iter()
                .filter(|(m, _)| overlap(*m, bx) >= SAME_BOX)
                .max_by(|a, b| overlap(a.0, bx).total_cmp(&overlap(b.0, bx)))
                .map_or(0, |m| m.1);
            c.execute(
                "INSERT INTO faces(thumb, model, x, y, w, h, score, px, emb, hidden) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![thumb, model, f.x, f.y, f.w, f.h, f.score, f.px, emb_to_blob(&f.emb), hidden],
            )?;
        }
        c.execute("INSERT OR REPLACE INTO faces_done(thumb, model, detect) VALUES(?1, ?2, ?3)", params![thumb, model, DETECT_VERSION])?;
        Ok(())
    })();
    c.execute_batch(if r.is_ok() { "RELEASE keep" } else { "ROLLBACK TO keep; RELEASE keep" })?;
    r
}

/// Thumbnails `model` looked at with an older face finder ([`DETECT_VERSION`]): the re-check's
/// next ones, newest first. Runs after every new picture is done, so it never holds them up.
pub fn recheck_todo(ix: &Index, model: &str, n: usize) -> rusqlite::Result<Vec<String>> {
    let mut st = conn(ix).prepare("SELECT thumb FROM faces_done WHERE model = ?1 AND detect < ?2 ORDER BY rowid DESC LIMIT ?3")?;
    st.query_map(params![model, DETECT_VERSION, n as i64], |r| r.get(0))?.collect()
}

/// The re-check's progress for `model`: (pictures looked at by this finder, all pictures looked
/// at). Equal when there is nothing (left) to re-check.
pub fn recheck_progress(ix: &Index, model: &str) -> (u64, u64) {
    conn(ix)
        .query_row(
            "SELECT COALESCE(SUM(detect >= ?2), 0), COUNT(*) FROM faces_done WHERE model = ?1",
            params![model, DETECT_VERSION],
            |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
        )
        .unwrap_or((0, 0))
}

/// Adds the faces found again in thumbnail `thumb` that are not there yet, and notes the picture
/// as looked at by this finder. Never deletes or changes a face: one that overlaps any face of
/// that picture ([`SAME_BOX`]: named, grouped, ignored or "not a face" alike) is the same face
/// and is left out, so names, "not this person", groups and marks all stay. Returns how many
/// were added; they are new unnamed faces for [`regroup`] (which may ask "Same person?").
pub fn add_new(ix: &Index, thumb: &str, model: &str, found: &[Found]) -> rusqlite::Result<usize> {
    let c = conn(ix);
    c.execute_batch("SAVEPOINT add_new")?;
    let r = (|| {
        let mut st = c.prepare("SELECT x, y, w, h FROM faces WHERE thumb = ?1 AND model = ?2")?;
        let mut have: Vec<[f32; 4]> =
            st.query_map(params![thumb, model], |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?]))?.collect::<rusqlite::Result<_>>()?;
        let mut n = 0;
        for f in found {
            let bx = [f.x, f.y, f.w, f.h];
            if have.iter().any(|h| overlap(*h, bx) >= SAME_BOX) {
                continue;
            }
            c.execute(
                "INSERT INTO faces(thumb, model, x, y, w, h, score, px, emb) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![thumb, model, f.x, f.y, f.w, f.h, f.score, f.px, emb_to_blob(&f.emb)],
            )?;
            have.push(bx);
            n += 1;
        }
        c.execute(
            "INSERT INTO faces_done(thumb, model, detect) VALUES(?1, ?2, ?3) ON CONFLICT(thumb, model) DO UPDATE SET detect = excluded.detect",
            params![thumb, model, DETECT_VERSION],
        )?;
        Ok(n)
    })();
    c.execute_batch(if r.is_ok() { "RELEASE add_new" } else { "ROLLBACK TO add_new; RELEASE add_new" })?;
    r
}

const FACE_COLS: &str = "id, thumb, x, y, w, h, person, grp, suggest, hidden";

fn face_row(r: &rusqlite::Row) -> rusqlite::Result<Face> {
    Ok(Face {
        id: r.get(0)?,
        thumb: r.get(1)?,
        bx: [r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?],
        person: r.get(6)?,
        grp: r.get(7)?,
        suggest: r.get(8)?,
        state: r.get(9)?,
    })
}

/// Every face of `model` People shows (without embeddings): not ignored, not "not a face".
pub fn faces(ix: &Index, model: &str) -> rusqlite::Result<Vec<Face>> {
    let mut st = conn(ix).prepare(&format!("SELECT {FACE_COLS} FROM faces WHERE model = ?1 AND {SHOWN}"))?;
    st.query_map([model], face_row)?.collect()
}

/// The ignored ([`IGNORED`]) and "not a face" ([`NOT_FACE`]) faces of `model`, newest first.
pub fn marked(ix: &Index, model: &str) -> rusqlite::Result<Vec<Face>> {
    let mut st = conn(ix).prepare(&format!("SELECT {FACE_COLS} FROM faces WHERE model = ?1 AND hidden IN (3, 4) ORDER BY id DESC"))?;
    st.query_map([model], face_row)?.collect()
}

/// Face `id` in any state (ignored and "not a face" too: their pictures still show), or none.
pub fn face(ix: &Index, id: i64) -> rusqlite::Result<Option<Face>> {
    let mut st = conn(ix).prepare(&format!("SELECT {FACE_COLS} FROM faces WHERE id = ?1"))?;
    st.query_map([id], face_row)?.next().transpose()
}

/// The square People shows around face box `bx` (0..1 of `img`): 1.5 times the face's longer
/// side, moved inside the picture when it would stick out, scaled to `size` × `size`.
pub fn face_crop(img: &image::RgbImage, bx: [f32; 4], size: u32) -> image::RgbImage {
    let (iw, ih) = (img.width(), img.height());
    if iw == 0 || ih == 0 {
        return image::RgbImage::new(size, size);
    }
    let (w, h) = (iw as f32, ih as f32);
    let side = ((bx[2] * w).max(bx[3] * h) * 1.5).clamp(1.0, w.min(h));
    let (cx, cy) = ((bx[0] + bx[2] / 2.0) * w, (bx[1] + bx[3] / 2.0) * h);
    let x0 = (cx - side / 2.0).clamp(0.0, w - side).round() as u32;
    let y0 = (cy - side / 2.0).clamp(0.0, h - side).round() as u32;
    let s = (side.round() as u32).max(1).min(iw - x0.min(iw - 1)).min(ih - y0.min(ih - 1));
    let sub = image::imageops::crop_imm(img, x0.min(iw - 1), y0.min(ih - 1), s, s).to_image();
    image::imageops::resize(&sub, size, size, image::imageops::FilterType::CatmullRom)
}

/// The people `user` named: id → name (`user` is the account name, "" without sign-in).
pub fn people(ix: &Index, user: &str) -> rusqlite::Result<HashMap<i64, String>> {
    let mut st = conn(ix).prepare("SELECT person, name FROM person_names WHERE user = ?1")?;
    st.query_map([user], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
}

/// A group as the People view names it: `p<person id>` or `g<group id>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Who {
    Person(i64),
    Group(i64),
}

impl Who {
    pub fn parse(s: &str) -> Option<Who> {
        let n: i64 = s.get(1..)?.parse().ok()?;
        match s.as_bytes().first()? {
            b'p' => Some(Who::Person(n)),
            b'g' => Some(Who::Group(n)),
            _ => None,
        }
    }
    pub fn key(self) -> String {
        match self {
            Who::Person(n) => format!("p{n}"),
            Who::Group(n) => format!("g{n}"),
        }
    }
    pub fn of(f: &Face) -> Option<Who> {
        f.person.map(Who::Person).or(f.grp.map(Who::Group))
    }
}

/// The faces in `who`.
fn ids_of(c: &Connection, who: Who) -> rusqlite::Result<Vec<i64>> {
    let (sql, n) = match who {
        Who::Person(n) => ("SELECT id FROM faces WHERE person = ?1 AND hidden IN (0, 2)", n),
        Who::Group(n) => ("SELECT id FROM faces WHERE grp = ?1 AND person IS NULL AND hidden IN (0, 2)", n),
    };
    let mut st = c.prepare(sql)?;
    st.query_map([n], |r| r.get(0))?.collect()
}

/// Names `who` for `user` only (others keep their own name, or none): a group becomes a person;
/// a person is renamed. An empty name names nobody (a group still becomes a person). Returns the
/// person.
pub fn name(ix: &Index, who: Who, user: &str, name: &str) -> rusqlite::Result<i64> {
    let c = conn(ix);
    let p = match who {
        Who::Person(p) => p,
        Who::Group(_) => {
            let ids = ids_of(c, who)?;
            c.execute("INSERT INTO people(name, created) VALUES('', ?1)", [now()])?;
            let p = c.last_insert_rowid();
            for id in ids {
                c.execute("UPDATE faces SET person = ?1, auto = 0, grp = NULL, suggest = NULL WHERE id = ?2", params![p, id])?;
            }
            p
        }
    };
    if !name.is_empty() {
        c.execute("INSERT OR REPLACE INTO person_names(person, user, name) VALUES(?1, ?2, ?3)", params![p, user, name])?;
    }
    Ok(p)
}

/// Puts every face of `from` with `into`. When `into` is a group, it becomes a person first
/// (named like `from` when that is a person, or unnamed). Returns the person.
pub fn merge(ix: &Index, from: Who, into: Who) -> rusqlite::Result<i64> {
    let c = conn(ix);
    // Keep the named one: merging a person into a group goes the other way.
    let (from, into) = match (from, into) {
        (Who::Person(_), Who::Group(_)) => (into, from),
        x => x,
    };
    let p = match into {
        Who::Person(p) => p,
        Who::Group(_) => name(ix, into, "", "")?,
    };
    for id in ids_of(c, from)? {
        c.execute("UPDATE faces SET person = ?1, auto = 0, grp = NULL, suggest = NULL WHERE id = ?2", params![p, id])?;
    }
    if let Who::Person(old) = from
        && old != p
    {
        c.execute("UPDATE face_not SET person = ?1 WHERE person = ?2", params![p, old])?;
        // Each account keeps its name for the merged person (or takes the one it gave `from`).
        c.execute(
            "INSERT OR IGNORE INTO person_names(person, user, name) SELECT ?1, user, name FROM person_names WHERE person = ?2",
            params![p, old],
        )?;
        c.execute("DELETE FROM person_names WHERE person = ?1", [old])?;
        c.execute("DELETE FROM people WHERE id = ?1", [old])?;
    }
    Ok(p)
}

/// Takes face `face` out of `who`. Out of a person it is a new face again: its own unnamed
/// group (until a regroup finds it others), never matched to that person again. Out of a group,
/// it is never grouped again. False if the face isn't in `who`.
pub fn remove(ix: &Index, who: Who, face: i64) -> rusqlite::Result<bool> {
    let c = conn(ix);
    if !ids_of(c, who)?.contains(&face) {
        return Ok(false);
    }
    match who {
        Who::Person(p) => {
            c.execute("INSERT OR IGNORE INTO face_not(face, person) VALUES(?1, ?2)", params![face, p])?;
            c.execute("UPDATE faces SET person = NULL, auto = 0, suggest = NULL, grp = ?1, hidden = 0 WHERE id = ?1", [face])?;
        }
        Who::Group(_) => {
            c.execute("UPDATE faces SET grp = NULL, hidden = 2 WHERE id = ?1", [face])?;
        }
    }
    Ok(true)
}

/// The answer to "Same person?" for face `face` (with its group, when it is in one) and person
/// `p`. Yes: they join `p`. No: "not this person", never asked or joined again. False when the
/// face is already someone's or `p` is gone.
pub fn answer(ix: &Index, face: i64, p: i64, yes: bool) -> rusqlite::Result<bool> {
    let c = conn(ix);
    let Some(f) = self::face(ix, face)?.filter(|f| f.state != IGNORED && f.state != NOT_FACE) else { return Ok(false) };
    let known: Option<i64> = c.query_row("SELECT id FROM people WHERE id = ?1", [p], |r| r.get(0)).optional()?;
    if f.person.is_some() || known.is_none() {
        return Ok(false);
    }
    let ids = match f.grp {
        Some(g) => ids_of(c, Who::Group(g))?,
        None => vec![face],
    };
    for id in ids {
        let not: Option<i64> = c.query_row("SELECT face FROM face_not WHERE face = ?1 AND person = ?2", params![id, p], |r| r.get(0)).optional()?;
        if yes && not.is_some() {
            // Someone said this face isn't `p`: the rest of its group joins, it doesn't.
            continue;
        }
        if yes {
            c.execute("UPDATE faces SET person = ?1, auto = 0, grp = NULL, suggest = NULL, hidden = 0 WHERE id = ?2", params![p, id])?;
        } else {
            c.execute("INSERT OR IGNORE INTO face_not(face, person) VALUES(?1, ?2)", params![id, p])?;
            c.execute("UPDATE faces SET suggest = NULL WHERE id = ?1 AND suggest = ?2", params![id, p])?;
        }
    }
    Ok(true)
}

/// Marks faces `ids` as [`IGNORED`] or [`NOT_FACE`]: out of their person or group (a face that
/// was someone's is never matched to them again, like a removed one), out of every question.
/// They keep `grp` (the group they came from, or their own id when they were someone's), so
/// [`unmark`] brings a group back together. Nothing is deleted.
pub fn mark(ix: &Index, ids: &[i64], state: i64) -> rusqlite::Result<usize> {
    let c = conn(ix);
    let found: Vec<Face> = ids.iter().filter_map(|&id| self::face(ix, id).ok().flatten()).filter(|f| f.state == 0 || f.state == 2).collect();
    // A person's marked faces stay together as one group (named by its smallest face id); only
    // faces taken from a person who keeps others are "not this person".
    let mut of_person: HashMap<i64, Vec<i64>> = HashMap::new();
    for f in &found {
        if let Some(p) = f.person {
            of_person.entry(p).or_default().push(f.id);
        }
    }
    let mut whole = HashSet::new();
    for (p, mine) in &of_person {
        if ids_of(c, Who::Person(*p))?.iter().all(|i| mine.contains(i)) {
            whole.insert(*p);
        }
    }
    let mut n = 0;
    for f in &found {
        let id = f.id;
        if let Some(p) = f.person
            && !whole.contains(&p)
        {
            c.execute("INSERT OR IGNORE INTO face_not(face, person) VALUES(?1, ?2)", params![id, p])?;
        }
        let grp = match f.person {
            Some(p) => of_person.get(&p).and_then(|v| v.iter().min().copied()),
            None => f.grp,
        };
        n += c.execute("UPDATE faces SET hidden = ?2, person = NULL, auto = 0, suggest = NULL, grp = ?3 WHERE id = ?1", params![id, state, grp])?;
    }
    Ok(n)
}

/// Undoes [`mark`] for faces `ids` that are in `state`: back to the unnamed faces, in the group
/// they were in. `fresh`: without that group, so the next regroup finds them a group ("Group
/// again").
pub fn unmark(ix: &Index, ids: &[i64], state: i64, fresh: bool) -> rusqlite::Result<usize> {
    let c = conn(ix);
    let sql = if fresh {
        "UPDATE faces SET hidden = 0, grp = NULL WHERE id = ?1 AND hidden = ?2"
    } else {
        "UPDATE faces SET hidden = 0 WHERE id = ?1 AND hidden = ?2"
    };
    let mut n = 0;
    for &id in ids {
        n += c.execute(sql, params![id, state])?;
    }
    Ok(n)
}

/// Gives ignored or "not a face" faces `ids` to person `p` (confirmed, like naming them).
pub fn assign(ix: &Index, ids: &[i64], p: i64) -> rusqlite::Result<usize> {
    let c = conn(ix);
    let mut n = 0;
    for &id in ids {
        c.execute("DELETE FROM face_not WHERE face = ?1 AND person = ?2", params![id, p])?;
        n += c.execute(
            "UPDATE faces SET person = ?2, auto = 0, grp = NULL, suggest = NULL, hidden = 0 WHERE id = ?1 AND hidden IN (3, 4)",
            params![id, p],
        )?;
    }
    Ok(n)
}

/// What [`regroup`] needs, read under the index lock in one go.
pub struct Snapshot {
    /// Faces not with a person, and the newest [`RECHECK`] put with one automatically:
    /// (id, embedding, may it be grouped), newest first.
    pub loose: Vec<(i64, Vec<f32>, bool)>,
    /// People's faces that someone confirmed (named, merged), at most [`PROTOTYPES`] each.
    /// Faces put with a person automatically are never compared with: one wrong automatic match
    /// would otherwise pull in every later face of that other person (2026-11-26: new people in
    /// new scans all vanished into existing ones).
    pub people: HashMap<i64, Vec<Vec<f32>>>,
    /// Face → people it was taken away from.
    pub not: HashMap<i64, HashSet<i64>>,
}

/// The most automatically matched faces looked at again in one pass (newest first).
pub const RECHECK: usize = 1000;

pub fn snapshot(ix: &Index, model: &str) -> rusqlite::Result<Snapshot> {
    let c = conn(ix);
    let mut st = c.prepare(
        "SELECT id, emb, score, px, hidden FROM faces WHERE model = ?1 AND person IS NULL AND hidden IN (0, 2)
         UNION ALL SELECT * FROM (SELECT id, emb, score, px, hidden FROM faces
           WHERE model = ?1 AND person IS NOT NULL AND auto = 1 AND hidden IN (0, 2) ORDER BY id DESC LIMIT ?2)
         ORDER BY id DESC",
    )?;
    let loose = st
        .query_map(params![model, RECHECK as i64], |r| {
            let (score, px, hidden): (f32, f32, i64) = (r.get(2)?, r.get(3)?, r.get(4)?);
            Ok((r.get(0)?, blob_to_emb(&r.get::<_, Vec<u8>>(1)?), hidden == 0 && good(score, px)))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut people: HashMap<i64, Vec<Vec<f32>>> = HashMap::new();
    let mut st = c.prepare("SELECT person, emb FROM faces WHERE model = ?1 AND person IS NOT NULL AND auto = 0 ORDER BY id DESC")?;
    for r in st.query_map([model], |r| Ok((r.get::<_, i64>(0)?, blob_to_emb(&r.get::<_, Vec<u8>>(1)?))))? {
        let (p, e) = r?;
        let v = people.entry(p).or_default();
        if v.len() < PROTOTYPES {
            v.push(e);
        }
    }
    let mut not: HashMap<i64, HashSet<i64>> = HashMap::new();
    let mut st = c.prepare("SELECT face, person FROM face_not")?;
    for r in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (f, p) = r?;
        not.entry(f).or_default().insert(p);
    }
    Ok(Snapshot { loose, people, not })
}

/// What a regroup decided for each loose face: (face, person it joins, person offered, group).
pub type Plan = Vec<(i64, Option<i64>, Option<i64>, Option<i64>)>;

/// Groups the loose faces the way a first scan does, then matches them to people: a face joins
/// its best person (at [`Thresholds::auto`]) unless it is in a group of two or more that, on
/// average, isn't like that person: then the group stays a new, unnamed group and the person is
/// only offered. Pure (no index lock held).
pub fn plan(s: &Snapshot, t: Thresholds) -> Plan {
    let empty = HashSet::new();
    let n = s.loose.len();
    let not = |i: usize| s.not.get(&s.loose[i].0).unwrap_or(&empty);
    let best: Vec<Option<(i64, f32)>> = (0..n).map(|i| best_person(&s.loose[i].1, &s.people, not(i))).collect();
    let rest: Vec<usize> = (0..n).filter(|&i| s.loose[i].2).take(MAX_GROUP).collect();
    let embs: Vec<Vec<f32>> = rest.iter().map(|&i| s.loose[i].1.clone()).collect();
    let labels = average_linkage(&embs, t.group_cut);
    let mut label_of: Vec<Option<usize>> = vec![None; n];
    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, &i) in rest.iter().enumerate() {
        label_of[i] = Some(labels[k]);
        members.entry(labels[k]).or_default().push(i);
    }
    // How like person `p` face `i` is (its best face of `p`; nothing when taken away from `p`).
    let like = |i: usize, p: i64| -> f32 {
        match s.people.get(&p) {
            Some(protos) if !not(i).contains(&p) => protos.iter().map(|q| similarity(&s.loose[i].1, q)).fold(f32::MIN, f32::max),
            _ => 0.0,
        }
    };
    let mut mean: HashMap<(usize, i64), f32> = HashMap::new();
    let mut joins: Vec<Option<i64>> = vec![None; n];
    for i in 0..n {
        let Some((p, _)) = best[i].filter(|(_, sim)| *sim >= t.auto) else { continue };
        let ok = match label_of[i].and_then(|l| members.get(&l).map(|m| (l, m))) {
            Some((l, m)) if m.len() >= 2 => {
                *mean.entry((l, p)).or_insert_with(|| m.iter().map(|&j| like(j, p)).sum::<f32>() / m.len() as f32) >= t.auto
            }
            _ => true,
        };
        if ok {
            joins[i] = Some(p);
        }
    }
    // Groups of what is left, named by their smallest face id; groups of one stay loose.
    let mut left: HashMap<usize, Vec<i64>> = HashMap::new();
    for &i in &rest {
        if let (None, Some(l)) = (joins[i], label_of[i]) {
            left.entry(l).or_default().push(s.loose[i].0);
        }
    }
    let mut grp_of: HashMap<i64, i64> = HashMap::new();
    for ids in left.values() {
        if ids.len() >= 2 {
            let g = *ids.iter().min().unwrap_or(&0);
            for id in ids {
                grp_of.insert(*id, g);
            }
        }
    }
    (0..n)
        .map(|i| {
            let id = s.loose[i].0;
            match joins[i] {
                Some(p) => (id, Some(p), None, None),
                // A face taken out of a person stays its own group when nothing else is like it.
                None => {
                    let own = s.not.contains_key(&id).then_some(id);
                    (id, None, best[i].filter(|(_, sim)| *sim >= t.suggest).map(|(p, _)| p), grp_of.get(&id).copied().or(own))
                }
            }
        })
        .collect()
}

/// Writes a plan (faces changed by someone since the snapshot are left alone: only faces still
/// loose, or still put with a person automatically, are touched).
pub fn apply(ix: &Index, plan: &Plan) -> rusqlite::Result<()> {
    let c = conn(ix);
    c.execute_batch("SAVEPOINT regroup")?;
    let r = (|| {
        let mut st = c.prepare(
            "UPDATE faces SET person = ?2, auto = ?3, suggest = ?4, grp = ?5 WHERE id = ?1 AND (person IS NULL OR auto = 1) AND hidden IN (0, 2)",
        )?;
        for (id, person, sug, grp) in plan {
            st.execute(params![id, person, person.is_some() as i64, sug, grp])?;
        }
        Ok(())
    })();
    c.execute_batch(if r.is_ok() { "RELEASE regroup" } else { "ROLLBACK TO regroup; RELEASE regroup" })?;
    r
}

/// Groups all loose faces of `model` again (holding `lock` only to read and to write).
pub fn regroup(lock: &std::sync::Mutex<Index>, model: &str) -> rusqlite::Result<usize> {
    let snap = {
        let ix = lock.lock().unwrap_or_else(|e| e.into_inner());
        snapshot(&ix, model)?
    };
    let p = plan(&snap, thresholds(model));
    let ix = lock.lock().unwrap_or_else(|e| e.into_inner());
    apply(&ix, &p)?;
    Ok(p.len())
}
