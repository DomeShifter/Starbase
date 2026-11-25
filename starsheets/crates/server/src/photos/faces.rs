//! Faces and people (Photos roadmap items 7 and 8): the tables in the photo index, the grouping
//! of faces into people, and the changes people make (name, merge, remove a wrong face).
//!
//! - **Where**: in the photo index (`index.sqlite` in the cache folder), never in anyone's folder.
//!   Faces are kept by the picture's thumbnail key (its content, with its edits), so a renamed,
//!   moved or copied picture keeps them. Each face has its box (0..1 of the thumbnail), the
//!   detector's score, its size in pixels, the model that made its embedding and the embedding.
//! - **Grouping** ([`regroup`]): faces are first matched against named people (the most similar
//!   of a person's faces; a face someone removed from a person is never matched to them again):
//!   at or above the model's `auto` similarity they join the person, at or above `suggest` the
//!   person is offered. The rest are grouped by **average linkage** (not DBSCAN: the face trial of
//!   2026-11 found DBSCAN chains people together) and cut at the model's `group_cut` distance.
//!   Only good faces (score and size, see [`good`]) start or join a group.
//! - **Thresholds** ([`thresholds`]) are the ones measured in the trial: AuraFace group 0.75,
//!   suggest 0.35, auto 0.45; SFace group 0.67, suggest 0.43, auto 0.54.

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
        -- matched to a named person).
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

/// Similarities and distances (cosine) for one embedding model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    /// Average-linkage cut: faces closer than this (1 − similarity) form a group.
    pub group_cut: f32,
    /// Offer "Is this X?" at or above this similarity to a person.
    pub suggest: f32,
    /// Put the face with the person without asking at or above this similarity.
    pub auto: f32,
}

/// The measured thresholds of a model (face trial 2026-11, RESULTS.md).
pub fn thresholds(model: &str) -> Thresholds {
    match model {
        "sface" => Thresholds { group_cut: 0.67, suggest: 0.43, auto: 0.54 },
        _ => Thresholds { group_cut: 0.75, suggest: 0.35, auto: 0.45 },
    }
}

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
        c.execute("DELETE FROM faces WHERE thumb = ?1 AND model = ?2", params![thumb, model])?;
        for f in faces {
            c.execute(
                "INSERT INTO faces(thumb, model, x, y, w, h, score, px, emb) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![thumb, model, f.x, f.y, f.w, f.h, f.score, f.px, emb_to_blob(&f.emb)],
            )?;
        }
        c.execute("INSERT OR REPLACE INTO faces_done(thumb, model) VALUES(?1, ?2)", params![thumb, model])?;
        Ok(())
    })();
    c.execute_batch(if r.is_ok() { "RELEASE keep" } else { "ROLLBACK TO keep; RELEASE keep" })?;
    r
}

/// Every face of `model` (without embeddings).
pub fn faces(ix: &Index, model: &str) -> rusqlite::Result<Vec<Face>> {
    let mut st = conn(ix).prepare("SELECT id, thumb, x, y, w, h, person, grp, suggest FROM faces WHERE model = ?1 AND hidden != 1")?;
    st.query_map([model], |r| {
        Ok(Face {
            id: r.get(0)?,
            thumb: r.get(1)?,
            bx: [r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?],
            person: r.get(6)?,
            grp: r.get(7)?,
            suggest: r.get(8)?,
        })
    })?
    .collect()
}

/// People: id → name.
pub fn people(ix: &Index) -> rusqlite::Result<HashMap<i64, String>> {
    let mut st = conn(ix).prepare("SELECT id, name FROM people")?;
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect()
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
        Who::Person(n) => ("SELECT id FROM faces WHERE person = ?1", n),
        Who::Group(n) => ("SELECT id FROM faces WHERE grp = ?1 AND person IS NULL", n),
    };
    let mut st = c.prepare(sql)?;
    st.query_map([n], |r| r.get(0))?.collect()
}

/// Names `who`: a group becomes a person; a person is renamed. Returns the person.
pub fn name(ix: &Index, who: Who, name: &str) -> rusqlite::Result<i64> {
    let c = conn(ix);
    match who {
        Who::Person(p) => {
            c.execute("UPDATE people SET name = ?1 WHERE id = ?2", params![name, p])?;
            Ok(p)
        }
        Who::Group(_) => {
            let ids = ids_of(c, who)?;
            c.execute("INSERT INTO people(name, created) VALUES(?1, ?2)", params![name, now()])?;
            let p = c.last_insert_rowid();
            for id in ids {
                c.execute("UPDATE faces SET person = ?1, auto = 0, grp = NULL, suggest = NULL WHERE id = ?2", params![p, id])?;
            }
            Ok(p)
        }
    }
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
        Who::Group(_) => name(ix, into, "")?,
    };
    for id in ids_of(c, from)? {
        c.execute("UPDATE faces SET person = ?1, auto = 0, grp = NULL, suggest = NULL WHERE id = ?2", params![p, id])?;
    }
    if let Who::Person(old) = from
        && old != p
    {
        c.execute("UPDATE face_not SET person = ?1 WHERE person = ?2", params![p, old])?;
        c.execute("DELETE FROM people WHERE id = ?1", [old])?;
    }
    Ok(p)
}

/// Takes face `face` out of `who`: never matched to that person again; out of a group, it is
/// never grouped again. False if the face isn't in `who`.
pub fn remove(ix: &Index, who: Who, face: i64) -> rusqlite::Result<bool> {
    let c = conn(ix);
    if !ids_of(c, who)?.contains(&face) {
        return Ok(false);
    }
    match who {
        Who::Person(p) => {
            c.execute("INSERT OR IGNORE INTO face_not(face, person) VALUES(?1, ?2)", params![face, p])?;
            c.execute("UPDATE faces SET person = NULL, auto = 0, suggest = NULL, grp = NULL, hidden = 2 WHERE id = ?1", [face])?;
        }
        Who::Group(_) => {
            c.execute("UPDATE faces SET grp = NULL, hidden = 2 WHERE id = ?1", [face])?;
        }
    }
    Ok(true)
}

/// What [`regroup`] needs, read under the index lock in one go.
pub struct Snapshot {
    /// Faces not with a person: (id, embedding, may it be grouped).
    pub loose: Vec<(i64, Vec<f32>, bool)>,
    /// Named (or merged) people's faces, at most [`PROTOTYPES`] each, confirmed ones first.
    pub people: HashMap<i64, Vec<Vec<f32>>>,
    /// Face → people it was taken away from.
    pub not: HashMap<i64, HashSet<i64>>,
}

pub fn snapshot(ix: &Index, model: &str) -> rusqlite::Result<Snapshot> {
    let c = conn(ix);
    let mut st = c.prepare("SELECT id, emb, score, px, hidden FROM faces WHERE model = ?1 AND person IS NULL AND hidden != 1 ORDER BY id DESC")?;
    let loose = st
        .query_map([model], |r| {
            let (score, px, hidden): (f32, f32, i64) = (r.get(2)?, r.get(3)?, r.get(4)?);
            Ok((r.get(0)?, blob_to_emb(&r.get::<_, Vec<u8>>(1)?), hidden == 0 && good(score, px)))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut people: HashMap<i64, Vec<Vec<f32>>> = HashMap::new();
    let mut st = c.prepare("SELECT person, emb FROM faces WHERE model = ?1 AND person IS NOT NULL ORDER BY auto, id DESC")?;
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

/// Matches loose faces to people, then groups what is left (no index lock held: pure).
pub fn plan(s: &Snapshot, t: Thresholds) -> Plan {
    let empty = HashSet::new();
    let mut out: Plan = Vec::with_capacity(s.loose.len());
    let mut rest: Vec<usize> = Vec::new();
    for (i, (id, e, ok)) in s.loose.iter().enumerate() {
        let best = best_person(e, &s.people, s.not.get(id).unwrap_or(&empty));
        match best {
            Some((p, sim)) if sim >= t.auto => out.push((*id, Some(p), None, None)),
            other => {
                let sug = other.filter(|(_, sim)| *sim >= t.suggest).map(|(p, _)| p);
                out.push((*id, None, sug, None));
                if *ok && rest.len() < MAX_GROUP {
                    rest.push(i);
                }
            }
        }
    }
    let embs: Vec<Vec<f32>> = rest.iter().map(|&i| s.loose[i].1.clone()).collect();
    let labels = average_linkage(&embs, t.group_cut);
    // A group is named by its smallest face id; groups of one stay loose.
    let mut members: HashMap<usize, Vec<i64>> = HashMap::new();
    for (k, &i) in rest.iter().enumerate() {
        members.entry(labels[k]).or_default().push(s.loose[i].0);
    }
    let mut grp_of: HashMap<i64, i64> = HashMap::new();
    for ids in members.values() {
        if ids.len() >= 2 {
            let g = *ids.iter().min().unwrap_or(&0);
            for id in ids {
                grp_of.insert(*id, g);
            }
        }
    }
    for row in &mut out {
        if row.1.is_none() {
            row.3 = grp_of.get(&row.0).copied();
        }
    }
    out
}

/// Writes a plan (faces changed by someone since the snapshot are left alone: only faces still
/// loose are touched).
pub fn apply(ix: &Index, plan: &Plan) -> rusqlite::Result<()> {
    let c = conn(ix);
    c.execute_batch("SAVEPOINT regroup")?;
    let r = (|| {
        let mut st = c.prepare("UPDATE faces SET person = ?2, auto = ?3, suggest = ?4, grp = ?5 WHERE id = ?1 AND person IS NULL")?;
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
