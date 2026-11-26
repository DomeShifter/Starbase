//! People over HTTP (docs/photos-api.md, "People"). Faces come from the face worker (ml.rs) and
//! are grouped in faces.rs. Everyone sees only the faces in pictures they may see right now, and
//! a group shows up only when they can see at least one of its pictures.
//!
//! - `GET /api/photos/people`: `{"on", "can_toggle", "forced", "note", "missing", "model",
//!   "progress": {"done", "all", "faces"}, "people": [card], "groups": [card]}`; a card is
//!   `{"id", "name", "count", "cover": {"id", "v", "box": [x, y, w, h]}}` (box 0..1 of the
//!   thumbnail). `people` are named (by name), `groups` unnamed (biggest first).
//! - `GET /api/photos/people/ID`: `{"id", "name", "faces": [{"face", "id", "v", "box"}], "photos", "waiting"}`.
//! - `POST /api/photos/people/ID/name` `{"name"}`; `/merge` `{"into": ID}`; `/remove` `{"face"}`.
//! - `GET|POST /api/photos/faces` `{"on": bool}`: face recognition on or off (the server's owner;
//!   `STARBASE_PHOTOS_FACES` overrides).
//! - `POST /api/photos/faces/models` (owner, `ml` builds): download the missing model files from
//!   their pinned URLs (SHA-256 checked) into the models folder; progress in `"download"`
//!   `{"busy", "file", "got", "total", "error"}`, `"can_download"` says whether the button applies.
//!
//! Deferred hardening: people are one list for the whole server (anyone who can see one of a
//! group's pictures may name or merge it); per-person libraries and an owner-only rename can follow.

use std::collections::{BTreeSet, HashMap};

use axum::body::Body;
use axum::http::Response;
use serde_json::{Value, json};

use super::albums::{Seen, seen};
use super::faces::{self, Face, Who};
use super::routes::{photo_json, tag};
use super::service;
use crate::{App, Shared, auth, blocking, error, send_json, sharing};

const MAX_NAME: usize = 80;
const MAX_GROUPS: usize = 300;

/// The owner's "Download face models": fetches the missing pinned model files in the background.
fn fetch(a: &App, owner: bool) -> Answer {
    if !owner {
        return Err(error(403, "Only the server's owner can download the face models."));
    }
    let s = svc(a)?;
    if !cfg!(feature = "ml") {
        return Err(error(400, "This server was built without face recognition (the `ml` feature), so the models would not be used."));
    }
    s.fetch_models().map_err(|e| error(400, &e))?;
    Ok(state(a, owner))
}

/// The pictures I may see, by thumbnail key.
type Vis = HashMap<String, Vec<Seen>>;

pub async fn route(app: &Shared, m: &str, p: &[String], me: Option<auth::User>, body: &[u8]) -> Response<Body> {
    if app.auth && me.is_none() {
        return error(401, "Sign in first.");
    }
    let b: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let owner = !app.auth || me.as_ref().is_some_and(|u| u.owner);
    let r = match (p[1].as_str(), m, p.len()) {
        ("faces", "GET", 2) => blocking(app, move |a| Ok(state(a, owner))).await,
        ("faces", "POST", 2) => blocking(app, move |a| toggle(a, owner, &b)).await,
        ("faces", "POST", 3) if p[2] == "models" => blocking(app, move |a| fetch(a, owner)).await,
        ("people", "GET", 2) => blocking(app, move |a| list(a, me, owner)).await,
        ("people", "GET", 3) => {
            let id = p[2].clone();
            blocking(app, move |a| open(a, me, &id)).await
        }
        ("people", "POST", 4) => {
            let (id, act) = (p[2].clone(), p[3].clone());
            blocking(app, move |a| change(a, me, &id, &act, &b)).await
        }
        _ => return error(404, "not found"),
    };
    match r {
        Some(Ok(v)) => send_json(&v, 200),
        Some(Err(e)) => e,
        None => error(500, "the server is busy"),
    }
}

type Answer = Result<Value, Response<Body>>;

fn svc(a: &App) -> Result<std::sync::Arc<super::Photos>, Response<Body>> {
    service(&a.store).ok_or_else(|| error(503, "Photos aren't available on this server."))
}

fn state(a: &App, owner: bool) -> Value {
    let Some(s) = service(&a.store) else { return json!({"on": false, "note": "Photos aren't available on this server."}) };
    let model = s.face_model().to_string();
    let (done, all, nfaces) = s.with_db(|ix| faces::progress(ix, &model));
    let missing: Vec<Value> = if owner {
        s.missing_models().iter().map(|m| json!({"file": m.file, "what": m.what, "licence": m.licence, "url": m.url})).collect()
    } else {
        Vec::new()
    };
    let d = s.ml.dl.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let fetchable = !missing.is_empty() && s.missing_models().iter().all(|m| m.get.is_some());
    json!({
        "can_download": owner && cfg!(feature = "ml") && (fetchable || d.busy),
        "download": if owner { json!({"busy": d.busy, "file": d.file, "got": d.got, "total": d.total, "error": d.error}) } else { Value::Null },
        "on": s.faces_on(),
        "can_toggle": owner && s.ml.cfg.forced.is_none(),
        "forced": s.ml.cfg.forced.is_some(),
        "note": s.ml.note(),
        "missing": missing,
        "models": if owner { Value::String(s.models_dir().display().to_string()) } else { Value::Null },
        "model": model,
        "progress": {"done": done, "all": all, "faces": nfaces},
    })
}

fn toggle(a: &App, owner: bool, b: &Value) -> Answer {
    if !owner {
        return Err(error(403, "Only the server's owner can turn face recognition on or off."));
    }
    let s = svc(a)?;
    if s.ml.cfg.forced.is_some() {
        return Err(error(400, "STARBASE_PHOTOS_FACES decides this on this server."));
    }
    let on = b["on"].as_bool().ok_or_else(|| error(400, "Send {\"on\": true} or false."))?;
    s.with_db(|ix| faces::set_setting(ix, "faces", if on { "on" } else { "off" })).map_err(|_| error(500, "couldn't save"))?;
    s.ml.poke();
    Ok(state(a, owner))
}

/// The pictures I may see, by thumbnail key.
fn visible(a: &App, s: &super::Photos, me: &Option<auth::User>) -> Vis {
    let acc = sharing::Access::new(&a.store, a.auth, me.as_ref());
    let by_path: HashMap<&str, &str> = acc.file_paths.iter().map(|(id, p)| (p.as_str(), id.as_str())).collect();
    let mut out: Vis = HashMap::new();
    for (thumb, path) in s.with_db(faces::thumb_paths).unwrap_or_default() {
        if let Some(id) = by_path.get(path.as_str())
            && let Some(v) = seen(a, &acc, id)
        {
            out.entry(thumb).or_default().push(v);
        }
    }
    out
}

fn face_json(f: &Face, vis: &Vis) -> Option<Value> {
    let (id, _, _) = vis.get(&f.thumb)?.first()?;
    Some(json!({"face": f.id, "id": id, "v": tag(&f.thumb), "box": f.bx, "suggest": f.suggest.map(|p| Who::Person(p).key())}))
}

fn list(a: &App, me: Option<auth::User>, owner: bool) -> Answer {
    let s = svc(a)?;
    let vis = visible(a, &s, &me);
    let model = s.face_model().to_string();
    let (all, names) = s
        .with_db(|ix| Ok::<_, rusqlite::Error>((faces::faces(ix, &model)?, faces::people(ix)?)))
        .map_err(|_| error(500, "couldn't read the faces"))?;
    let mut groups: HashMap<Who, (BTreeSet<String>, Option<Value>)> = HashMap::new();
    for f in &all {
        let (Some(who), Some(pics)) = (Who::of(f), vis.get(&f.thumb)) else { continue };
        let g = groups.entry(who).or_default();
        g.0.extend(pics.iter().map(|p| p.0.clone()));
        if g.1.is_none() {
            g.1 = face_json(f, &vis);
        }
    }
    let mut named = Vec::new();
    let mut unnamed = Vec::new();
    for (who, (pics, cover)) in groups {
        let name = match who {
            Who::Person(p) => names.get(&p).cloned().unwrap_or_default(),
            Who::Group(_) => String::new(),
        };
        let card = json!({"id": who.key(), "name": name, "count": pics.len(), "cover": cover});
        if name.is_empty() { unnamed.push(card) } else { named.push(card) }
    }
    named.sort_by_key(|c| c["name"].as_str().unwrap_or("").to_lowercase());
    unnamed.sort_by_key(|c| std::cmp::Reverse(c["count"].as_u64().unwrap_or(0)));
    unnamed.truncate(MAX_GROUPS);
    let mut v = state(a, owner);
    v["people"] = Value::Array(named);
    v["groups"] = Value::Array(unnamed);
    Ok(v)
}

/// The faces of `who` I may see (and all of them), or 404.
fn faces_of(a: &App, s: &super::Photos, me: &Option<auth::User>, who: Who) -> Result<(Vec<Face>, Vis), Response<Body>> {
    let vis = visible(a, s, me);
    let model = s.face_model().to_string();
    let all = s.with_db(|ix| faces::faces(ix, &model)).map_err(|_| error(500, "couldn't read the faces"))?;
    let mine: Vec<Face> = all.into_iter().filter(|f| Who::of(f) == Some(who) && vis.contains_key(&f.thumb)).collect();
    if mine.is_empty() {
        return Err(error(404, "No such person or group."));
    }
    Ok((mine, vis))
}

fn who_of(id: &str) -> Result<Who, Response<Body>> {
    Who::parse(id).ok_or_else(|| error(404, "No such person or group."))
}

fn open(a: &App, me: Option<auth::User>, id: &str) -> Answer {
    let s = svc(a)?;
    let who = who_of(id)?;
    let (mine, vis) = faces_of(a, &s, &me, who)?;
    let name = match who {
        Who::Person(p) => s.with_db(faces::people).ok().and_then(|n| n.get(&p).cloned()).unwrap_or_default(),
        Who::Group(_) => String::new(),
    };
    let fj: Vec<Value> = mine.iter().filter_map(|f| face_json(f, &vis)).collect();
    let mut seen_ids = BTreeSet::new();
    let mut rows = Vec::new();
    for f in &mine {
        for (fid, path, gps) in vis.get(&f.thumb).into_iter().flatten() {
            if seen_ids.insert(fid.clone())
                && let Some(r) = s.row(path)
            {
                rows.push((fid.clone(), r, *gps));
            }
        }
    }
    let key = |r: &super::index::Row| r.meta.taken.clone().unwrap_or_default();
    rows.sort_by_key(|x| std::cmp::Reverse(key(&x.1)));
    let photos: Vec<Value> = rows.iter().map(|(id, r, g)| photo_json(r, id, *g)).collect();
    Ok(json!({"id": who.key(), "name": name, "faces": fj, "photos": photos, "waiting": 0}))
}

fn change(a: &App, me: Option<auth::User>, id: &str, act: &str, b: &Value) -> Answer {
    let s = svc(a)?;
    let who = who_of(id)?;
    let (mine, _) = faces_of(a, &s, &me, who)?;
    let db_err = |_| error(500, "couldn't save");
    let out = match act {
        "name" => {
            let n = b["name"].as_str().unwrap_or("").trim().to_string();
            if n.is_empty() || n.chars().count() > MAX_NAME || n.chars().any(char::is_control) {
                return Err(error(400, "A name is 1 to 80 characters."));
            }
            let p = s.with_db(|ix| faces::name(ix, who, &n)).map_err(db_err)?;
            json!({"id": Who::Person(p).key()})
        }
        "merge" => {
            let into = who_of(b["into"].as_str().unwrap_or(""))?;
            if into == who {
                return Err(error(400, "That's the same group."));
            }
            faces_of(a, &s, &me, into)?;
            let p = s.with_db(|ix| faces::merge(ix, who, into)).map_err(db_err)?;
            json!({"id": Who::Person(p).key()})
        }
        "remove" => {
            let face = b["face"].as_i64().ok_or_else(|| error(400, "Which face?"))?;
            if !mine.iter().any(|f| f.id == face) || !s.with_db(|ix| faces::remove(ix, who, face)).map_err(db_err)? {
                return Err(error(404, "That face isn't in this group."));
            }
            json!({"id": who.key()})
        }
        _ => return Err(error(404, "not found")),
    };
    s.ml.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    s.ml.poke();
    Ok(out)
}
