//! The Locked folder in Photos (docs/photos-api.md, "Locked").
//!
//! A person can move pictures they own into Locked. A locked picture is hidden by the server from
//! every listing, album, place, memory, search, Drive list, share and public link (the one choke
//! point is `sharing::Access`, which leaves it out for everybody, the owner included). Only the
//! owner sees it, in Locked, after typing their password again; the unlock lasts 5 minutes after
//! the last use of Locked. Nothing is encrypted on disk: this hides, it does not protect against
//! someone with access to the server's files.
//!
//! TODO: when two-step sign-in exists, ask for the code here too.
//!
//! - `GET /api/photos/locked`: `{"unlocked": false}`, or when unlocked
//!   `{"unlocked": true, "photos": [...], "waiting"}` (401 if not signed in).
//! - `POST /api/photos/locked/unlock` `{"password"}`: `{"unlocked": true, "minutes": 5}`; a wrong
//!   password is refused (401) and counts against the sign-in limits.
//! - `POST /api/photos/locked/lock`: relock now.
//! - `POST /api/photos/locked/add` `{"files": [ids]}`: move in (my own pictures; no unlock needed).
//! - `POST /api/photos/locked/remove` `{"files": [ids]}`: move out (needs the unlock).
//! - `GET /api/photos/locked/ids`: just the ids (needs the unlock), for the Info panel.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::Response;
use serde_json::{Value, json};

use super::routes::{mark_stars, photo_json, stars_of};
use crate::store::Store;
use crate::{Shared, auth, blocking, error, guard, send_json, sharing};

/// How long Locked stays open after it was last used.
pub const IDLE: Duration = Duration::from_secs(300);

static OPEN: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Mutex::default);

fn file(store: &Store) -> std::path::PathBuf {
    store.meta().join("locked.json")
}

/// Locked file id -> its owner (username).
pub fn all(store: &Store) -> HashMap<String, String> {
    store
        .load_json(&file(store))
        .get("files")
        .and_then(Value::as_object)
        .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
        .unwrap_or_default()
}

fn save(store: &Store, m: &HashMap<String, String>) -> std::io::Result<()> {
    store.save_json(&file(store), &json!({ "files": m }))
}

pub fn is_locked(store: &Store, id: &str) -> bool {
    all(store).contains_key(id)
}

/// Whether `user` has Locked open right now; `touch` restarts the idle time.
pub fn is_open(user: &str, touch: bool) -> bool {
    let mut o = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    match o.get(user) {
        Some(t) if t.elapsed() < IDLE => {
            if touch {
                o.insert(user.to_string(), Instant::now());
            }
            true
        }
        Some(_) => {
            o.remove(user);
            false
        }
        None => false,
    }
}

fn set_open(user: &str, on: bool) {
    let mut o = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    if on {
        o.insert(user.to_string(), Instant::now());
    } else {
        o.remove(user);
    }
}

/// The locked ids that stay out of `user`'s view: all of them, except the user's own while Locked is open.
pub fn hidden_for(store: &Store, user: Option<&str>, with_mine: bool) -> HashSet<String> {
    let open = with_mine && user.is_some_and(|u| is_open(u, true));
    all(store).into_iter().filter(|(_, owner)| !(open && Some(owner.as_str()) == user)).map(|(id, _)| id).collect()
}

fn ids_of(b: &Value) -> Vec<String> {
    b["files"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).take(20_000).collect()).unwrap_or_default()
}

pub async fn route(app: &Shared, m: &str, p: &[String], me: Option<auth::User>, ip: Option<IpAddr>, body: &[u8]) -> Option<Response<Body>> {
    if p.len() < 2 || p[0] != "photos" || p[1] != "locked" || p.len() > 3 {
        return None;
    }
    let Some(me) = me else { return Some(error(401, "Locked needs a signed-in account.")) };
    let b: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let act = p.get(2).map(String::as_str);
    Some(match (m, act) {
        ("GET", None) => list(app, me).await,
        ("GET", Some("ids")) => {
            if !is_open(&me.username, true) {
                return Some(error(403, "Locked is closed: type your password."));
            }
            let mine: Vec<String> = blocking(app, move |a| all(&a.store).into_iter().filter(|(_, o)| *o == me.username).map(|(i, _)| i).collect())
                .await
                .unwrap_or_default();
            send_json(&json!({ "ids": mine }), 200)
        }
        ("POST", Some("unlock")) => unlock(app, me, ip, &b).await,
        ("POST", Some("lock")) => {
            set_open(&me.username, false);
            send_json(&json!({ "unlocked": false }), 200)
        }
        ("POST", Some("add")) => change(app, me, &b, true).await,
        ("POST", Some("remove")) => change(app, me, &b, false).await,
        _ => error(404, "not found"),
    })
}

async fn unlock(app: &Shared, me: auth::User, ip: Option<IpAddr>, b: &Value) -> Response<Body> {
    let password = b["password"].as_str().unwrap_or_default().to_string();
    let user_key = format!("login-user:{}", me.username.to_lowercase());
    if !guard::LIMITS.can(&user_key, guard::LOGIN_USER) || !guard::LIMITS.take(&guard::ip_key("login", ip), guard::LOGIN_IP) {
        return crate::too_many();
    }
    let name = me.username.clone();
    let found = blocking(app, move |a| auth::login_hash(&a.store, &name)).await.flatten();
    let hash = found.map(|(_, h)| h);
    let right = crate::slow(move || auth::verify(&password, hash.as_deref())).await.unwrap_or(false);
    if !right {
        guard::LIMITS.take(&user_key, guard::LOGIN_USER);
        return crate::wrong(401, "That password isn't right.").await;
    }
    set_open(&me.username, true);
    send_json(&json!({ "unlocked": true, "minutes": IDLE.as_secs() / 60 }), 200)
}

async fn change(app: &Shared, me: auth::User, b: &Value, into: bool) -> Response<Body> {
    if !into && !is_open(&me.username, true) {
        return error(403, "Locked is closed: type your password.");
    }
    let ids = ids_of(b);
    let r = blocking(app, move |a| -> Result<usize, String> {
        let mut m = all(&a.store);
        let mut n = 0;
        if into {
            let acc = sharing::Access::new(&a.store, a.auth, Some(&me));
            for id in ids {
                let ok = acc.file_paths.get(&id).is_some_and(|p| super::media_of(p).is_some())
                    && acc.file_role(&id) >= sharing::Role::Owner
                    && !m.contains_key(&id);
                if ok {
                    m.insert(id, me.username.clone());
                    n += 1;
                }
            }
        } else {
            for id in ids {
                if m.get(&id) == Some(&me.username) {
                    m.remove(&id);
                    n += 1;
                }
            }
        }
        save(&a.store, &m).map_err(|e| e.to_string())?;
        Ok(n)
    })
    .await;
    match r {
        Some(Ok(n)) => send_json(&json!({ "changed": n }), 200),
        Some(Err(e)) => error(500, &e),
        None => error(500, "server error"),
    }
}

async fn list(app: &Shared, me: auth::User) -> Response<Body> {
    if !is_open(&me.username, true) {
        return send_json(&json!({ "unlocked": false }), 200);
    }
    let stars = stars_of(app, Some(&me)).await;
    let vis = blocking(app, move |a| {
        let acc = sharing::Access::with_locked(&a.store, a.auth, Some(&me));
        let mine: Vec<String> = all(&a.store).into_iter().filter(|(_, o)| *o == me.username).map(|(i, _)| i).collect();
        mine.iter().filter_map(|id| super::albums::seen(a, &acc, id)).collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    let app = app.clone();
    let (rows, waiting) = tokio::task::spawn_blocking(move || super::albums::rows_of(&app, vis)).await.unwrap_or_default();
    let mut photos: Vec<Value> = rows.iter().map(|(id, r, g)| photo_json(r, id, *g)).collect();
    mark_stars(&mut photos, &stars);
    send_json(&json!({ "unlocked": true, "photos": photos, "waiting": waiting }), 200)
}
