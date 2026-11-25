//! Albums and Places over HTTP (docs/photos-api.md, "Albums and Places").
//!
//! Albums are virtual: a small JSON file per person in the server's own folder
//! (`.starsheets/albums-<hex of the name>.json`) that holds file ids. Files never move, and an
//! album shows a picture only to someone who may see that picture right now (so a picture in a
//! folder that stopped being shared simply doesn't show up for them).
//!
//! - `GET /api/photos/albums`: `{"albums": [{"id", "name", "count", "cover"}]}` (count = what I can see).
//! - `POST /api/photos/albums` `{"name", "files"?: [ids]}`: a new album, `{"id", "name", "count"}`.
//! - `GET /api/photos/albums/ID`: `{"id", "name", "photos", "waiting"}`, newest first.
//! - `POST /api/photos/albums/ID/rename` `{"name"}`, `/add` `{"files": [ids]}`, `/remove`
//!   `{"files": [ids]}`, `/delete` (the pictures stay where they are).
//! - `GET /api/photos/places`: `{"places": [{"name", "count", "cover"}]}`: the pictures I can see
//!   with a GPS position (only for people who may edit the picture's folder through their own
//!   account, as in the listing), grouped by the nearest town of a small list bundled in the
//!   program (GeoNames cities of 25,000 people or more, CC BY 4.0). Nothing is sent anywhere. A
//!   picture with no town within 80 km is grouped by its position rounded to 0.1 degree.
//! - `GET /api/photos/places?place=NAME`: `{"place", "photos", "waiting"}`.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use axum::body::Body;
use axum::http::Response;
use serde_json::{Map, Value, json};

use super::index::Row;
use super::routes::photo_json;
use super::{media_of, service};
use crate::{App, Shared, auth, blocking, error, send_json, sharing};

const MAX_ALBUMS: usize = 200;
const MAX_FILES: usize = 20_000;
const MAX_NAME: usize = 80;

/// GeoNames cities of 25,000 people or more: name, latitude, longitude, country (tab separated).
const CITIES: &str = include_str!("cities.tsv");
/// How far a town may be from a picture to name it, in kilometres.
const NEAR_KM: f64 = 80.0;

static TOWNS: LazyLock<Vec<(&'static str, f64, f64, &'static str)>> = LazyLock::new(|| {
    CITIES
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            Some((f.next()?, f.next()?.parse().ok()?, f.next()?.parse().ok()?, f.next()?))
        })
        .collect()
});

/// The name of the place at `lat`, `lon`: "Town, CC", or "52.1°N, 0.2°E" far from any town.
pub fn place_name(lat: f64, lon: f64) -> String {
    let mut best: Option<(f64, &str, &str)> = None;
    let k = lat.to_radians().cos().max(0.01);
    for (name, la, lo, cc) in TOWNS.iter() {
        let dlon = ((lon - lo + 180.0).rem_euclid(360.0)) - 180.0;
        let d = ((lat - la).powi(2) + (dlon * k).powi(2)).sqrt() * 111.0;
        if best.is_none_or(|b| d < b.0) {
            best = Some((d, name, cc));
        }
    }
    match best {
        Some((d, name, cc)) if d <= NEAR_KM => format!("{name}, {cc}"),
        _ => format!("{:.1}°{}, {:.1}°{}", lat.abs(), if lat < 0.0 { 'S' } else { 'N' }, lon.abs(), if lon < 0.0 { 'W' } else { 'E' }),
    }
}

fn file_of(a: &App, me: &Option<auth::User>) -> std::path::PathBuf {
    let who = me.as_ref().map_or("local", |u| u.username.as_str());
    let hex: String = who.bytes().map(|b| format!("{b:02x}")).collect();
    a.store.meta().join(format!("albums-{hex}.json"))
}

fn load(a: &App, me: &Option<auth::User>) -> Vec<Value> {
    a.store.load_json(&file_of(a, me)).remove("albums").and_then(|v| if let Value::Array(v) = v { Some(v) } else { None }).unwrap_or_default()
}

fn save(a: &App, me: &Option<auth::User>, albums: Vec<Value>) -> Result<(), Response<Body>> {
    let mut m = Map::new();
    m.insert("albums".into(), Value::Array(albums));
    a.store.save_json(&file_of(a, me), &Value::Object(m)).map_err(|_| error(500, "couldn't save the album"))
}

fn files_of(album: &Value) -> Vec<String> {
    album["files"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect()
}

fn body_json(body: &[u8]) -> Value {
    serde_json::from_slice(body).unwrap_or(Value::Null)
}

fn clean_name(v: &Value) -> Result<String, Response<Body>> {
    let n = v["name"].as_str().unwrap_or("").trim().to_string();
    if n.is_empty() || n.chars().count() > MAX_NAME || n.chars().any(char::is_control) {
        return Err(error(400, "An album's name is 1 to 80 characters."));
    }
    Ok(n)
}

/// A picture I may see (id, path, whether its position may be given to me).
pub(super) type Seen = (String, String, bool);

/// A picture as listed: id, index row, whether its position may be given.
pub(super) type Pic = (String, Row, bool);

pub(super) fn seen(a: &App, acc: &sharing::Access, id: &str) -> Option<Seen> {
    let path = acc.file_paths.get(id)?;
    if media_of(path).is_none() || acc.file_role(id) < sharing::Role::View || a.store.safe_file(path).is_none() {
        return None;
    }
    let gps = acc.file_role(id) >= sharing::Role::Edit && acc.folder_listed(&crate::store::parent_of(path));
    Some((id.to_string(), path.clone(), gps))
}

/// Pictures as the listing shows them, newest first, and how many are still being read.
pub(super) fn rows_of(a: &Shared, seen: Vec<Seen>) -> (Vec<(String, Row, bool)>, u64) {
    let Some(svc) = service(&a.store) else { return (Vec::new(), 0) };
    let mut out: Vec<(String, Row, bool)> = seen.into_iter().filter_map(|(id, path, gps)| svc.row(&path).map(|r| (id, r, gps))).collect();
    let key = |r: &Row| r.meta.taken.clone().map(|t| t.chars().take(19).collect::<String>()).unwrap_or_default();
    out.sort_by(|x, y| (key(&y.1), &y.1.path).cmp(&(key(&x.1), &x.1.path)));
    let (q, w) = svc.queued();
    (out, (q + w) as u64)
}

pub(super) fn cover(rows: &[(String, Row, bool)]) -> Value {
    rows.iter()
        .find_map(|(id, r, g)| photo_json(r, id, *g).get("v").filter(|v| v.is_string()).map(|v| json!({"id": id, "v": v})))
        .unwrap_or(Value::Null)
}

pub async fn route(
    app: &Shared,
    m: &str,
    p: &[String],
    q: &std::collections::HashMap<String, String>,
    me: Option<auth::User>,
    body: &[u8],
) -> Response<Body> {
    if app.auth && me.is_none() {
        return error(401, "Sign in first.");
    }
    let b = body_json(body);
    let what = p.get(1).map(String::as_str);
    let id = p.get(2).cloned();
    let act = p.get(3).map(String::as_str);
    match (what, m, id, act, p.len()) {
        (Some("albums"), "GET", None, _, 2) => list_albums(app, me).await,
        (Some("albums"), "POST", None, _, 2) => create(app, me, b).await,
        (Some("albums"), "GET", Some(id), None, 3) => open_album(app, me, id).await,
        (Some("albums"), "POST", Some(id), Some(act @ ("rename" | "add" | "remove" | "delete")), 4) => change(app, me, id, act.to_string(), b).await,
        (Some("places"), "GET", None, _, 2) => places(app, me, q.get("place").cloned()).await,
        (Some("events"), "GET", None, _, 2) => super::events::events(app, me, q.get("event").cloned()).await,
        _ => error(404, "not found"),
    }
}

async fn list_albums(app: &Shared, me: Option<auth::User>) -> Response<Body> {
    let got = blocking(app, move |a| {
        let acc = sharing::Access::new(&a.store, a.auth, me.as_ref());
        load(a, &me)
            .into_iter()
            .map(|al| {
                let vis: Vec<Seen> = files_of(&al).iter().filter_map(|f| seen(a, &acc, f)).collect();
                (al["id"].clone(), al["name"].clone(), vis)
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    let app = app.clone();
    let out = tokio::task::spawn_blocking(move || {
        got.into_iter()
            .map(|(id, name, vis)| {
                let (rows, _) = rows_of(&app, vis);
                json!({"id": id, "name": name, "count": rows.len(), "cover": cover(&rows)})
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    send_json(&json!({"albums": out}), 200)
}

async fn create(app: &Shared, me: Option<auth::User>, b: Value) -> Response<Body> {
    let r = blocking(app, move |a| -> Result<Value, Response<Body>> {
        let acc = sharing::Access::new(&a.store, a.auth, me.as_ref());
        let name = clean_name(&b)?;
        // Optionally with pictures to start with (the ones I may see).
        let mut files: Vec<String> = Vec::new();
        for f in b["files"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            if files.len() < MAX_FILES && !files.iter().any(|x| x == f) && seen(a, &acc, f).is_some() {
                files.push(f.to_string());
            }
        }
        let mut all = load(a, &me);
        if all.len() >= MAX_ALBUMS {
            return Err(error(400, "That's the most albums one person can have."));
        }
        let id = crate::store::random_hex(6);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let files_n = files.len();
        all.push(json!({"id": id, "name": name, "created": now, "files": files}));
        save(a, &me, all)?;
        Ok(json!({"id": id, "name": name, "count": files_n}))
    })
    .await;
    match r {
        Some(Ok(v)) => send_json(&v, 200),
        Some(Err(e)) => e,
        None => error(500, "server error"),
    }
}

async fn open_album(app: &Shared, me: Option<auth::User>, id: String) -> Response<Body> {
    let got = blocking(app, move |a| -> Result<(Value, Vec<Seen>), Response<Body>> {
        let acc = sharing::Access::new(&a.store, a.auth, me.as_ref());
        let al = load(a, &me).into_iter().find(|al| al["id"] == id.as_str()).ok_or_else(|| error(404, "no such album"))?;
        let vis = files_of(&al).iter().filter_map(|f| seen(a, &acc, f)).collect();
        Ok((al, vis))
    })
    .await;
    let (al, vis) = match got {
        Some(Ok(g)) => g,
        Some(Err(e)) => return e,
        None => return error(500, "server error"),
    };
    let app = app.clone();
    let (rows, waiting) = tokio::task::spawn_blocking(move || rows_of(&app, vis)).await.unwrap_or_default();
    let photos: Vec<Value> = rows.iter().map(|(id, r, g)| photo_json(r, id, *g)).collect();
    send_json(&json!({"id": al["id"], "name": al["name"], "photos": photos, "waiting": waiting}), 200)
}

async fn change(app: &Shared, me: Option<auth::User>, id: String, act: String, b: Value) -> Response<Body> {
    let r = blocking(app, move |a| -> Result<Value, Response<Body>> {
        let acc = sharing::Access::new(&a.store, a.auth, me.as_ref());
        let mut all = load(a, &me);
        let at = all.iter().position(|al| al["id"] == id.as_str()).ok_or_else(|| error(404, "no such album"))?;
        let ids: Vec<String> = b["files"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
        let mut added = 0;
        match act.as_str() {
            "delete" => {
                all.remove(at);
            }
            "rename" => all[at]["name"] = Value::String(clean_name(&b)?),
            "add" => {
                let mut files = files_of(&all[at]);
                let have: HashSet<String> = files.iter().cloned().collect();
                for f in ids.iter().filter(|f| !have.contains(*f)) {
                    // Only pictures I may see (no hint about files I may not).
                    if seen(a, &acc, f).is_some() && files.len() < MAX_FILES && !files[files.len().saturating_sub(added)..].contains(f) {
                        files.push(f.clone());
                        added += 1;
                    }
                }
                all[at]["files"] = json!(files);
            }
            _ => {
                let gone: HashSet<&String> = ids.iter().collect();
                let files: Vec<String> = files_of(&all[at]).into_iter().filter(|f| !gone.contains(f)).collect();
                all[at]["files"] = json!(files);
            }
        }
        let out = if act == "delete" {
            json!({"deleted": true})
        } else {
            // `added`: how many were new (pictures already in the album aren't added twice).
            json!({"id": all[at]["id"], "name": all[at]["name"], "count": files_of(&all[at]).len(), "added": added})
        };
        save(a, &me, all)?;
        Ok(out)
    })
    .await;
    match r {
        Some(Ok(v)) => send_json(&v, 200),
        Some(Err(e)) => e,
        None => error(500, "server error"),
    }
}

async fn places(app: &Shared, me: Option<auth::User>, place: Option<String>) -> Response<Body> {
    // Pictures whose position I may know: ids by path, under the store lock.
    let mine = blocking(app, move |a| {
        let acc = sharing::Access::new(&a.store, a.auth, me.as_ref());
        acc.file_paths.keys().filter_map(|id| seen(a, &acc, id)).filter(|s| s.2).collect::<Vec<Seen>>()
    })
    .await
    .unwrap_or_default();
    let app = app.clone();
    let (groups, waiting) = tokio::task::spawn_blocking(move || {
        let (rows, waiting) = rows_of(&app, mine);
        let mut cache: HashMap<(i64, i64), String> = HashMap::new();
        let mut groups: Vec<(String, Vec<Pic>)> = Vec::new();
        for x in rows {
            let Some((lat, lon)) = x.1.meta.lat.zip(x.1.meta.lon) else { continue };
            let name = cache.entry(((lat * 20.0).round() as i64, (lon * 20.0).round() as i64)).or_insert_with(|| place_name(lat, lon)).clone();
            match groups.iter_mut().find(|g| g.0 == name) {
                Some(g) => g.1.push(x),
                None => groups.push((name, vec![x])),
            }
        }
        groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
        (groups, waiting)
    })
    .await
    .unwrap_or_default();
    if let Some(name) = place {
        let Some(g) = groups.iter().find(|g| g.0 == name) else { return error(404, "no such place") };
        let photos: Vec<Value> = g.1.iter().map(|(id, r, gps)| photo_json(r, id, *gps)).collect();
        return send_json(&json!({"place": name, "photos": photos, "waiting": waiting}), 200);
    }
    let out: Vec<Value> = groups.iter().map(|(n, rows)| json!({"name": n, "count": rows.len(), "cover": cover(rows)})).collect();
    send_json(&json!({"places": out, "waiting": waiting}), 200)
}

#[cfg(test)]
mod tests {
    use super::place_name;

    #[test]
    fn towns_name_places_offline() {
        assert!(place_name(51.51, -0.12).starts_with("London"), "{}", place_name(51.51, -0.12));
        assert_eq!(place_name(-60.0, -100.0), "60.0°S, 100.0°W");
    }
}
