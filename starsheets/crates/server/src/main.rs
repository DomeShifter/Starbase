//! StarSheets server: stores spreadsheets as ordinary .xlsx files in a folder tree (STORE) and,
//! with SITE set, serves the web app too. Replaces server/files.py with the same API and the
//! same files on disk (see docs/server.md), as the base for live editing.
//!
//! Settings (environment): STORE (folder, default /data), KEEP (versions kept, 6), PORT (8080),
//! LOCATION (how people are told where files are kept), STARSHEETS_AUTH (off = no login),
//! STARSHEETS_PASSWORD, SITE (a built StarSheets site to serve), and the security settings in
//! guard.rs (STARSHEETS_PUBLIC_URL, STARSHEETS_ALLOWED_HOSTS, STARSHEETS_TRUSTED_PROXY,
//! STARSHEETS_SECURE_COOKIE).

// Request handlers return a ready HTTP response as their error (axum's usual way); boxing it to
// make the error smaller would only add an allocation on the error path.
#![allow(clippy::result_large_err)]

mod auth;
mod backup;
mod guard;
mod live;
mod log;
mod sharing;
mod spaces;
mod store;
mod thumbs;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, Response, header};
use percent_encoding::percent_decode_str;
use serde_json::{Value, json};

use store::{Bad, Reply, Store};

/// The biggest file that can be sent (uploads, saves); live.rs uses it for its messages too.
const MAX_BYTES: u64 = 200 * 1024 * 1024;
/// The biggest body of any other request (sign-in, sharing, settings: all small JSON).
const SMALL_BYTES: u64 = 1024 * 1024;
/// How long reading a request's body may take (a slow upload of a big file gets longer).
const SMALL_WAIT: Duration = Duration::from_secs(30);
const BIG_WAIT: Duration = Duration::from_secs(600);
/// Big bodies read at once (each can be MAX_BYTES in memory); more wait their turn.
static BIG_AT_ONCE: LazyLock<tokio::sync::Semaphore> = LazyLock::new(|| tokio::sync::Semaphore::new(3));
/// Password hashes worked out at once (Argon2 is slow and uses memory on purpose).
static ARGON_AT_ONCE: LazyLock<tokio::sync::Semaphore> = LazyLock::new(|| tokio::sync::Semaphore::new(4));
/// The wait after a wrong password or code, to slow down guessing.
#[cfg(not(test))]
const WRONG_WAIT: Duration = Duration::from_millis(1500);
#[cfg(test)]
const WRONG_WAIT: Duration = Duration::from_millis(10);
const COOKIE: &str = "starsheets_session";

struct App {
    store: Store,
    /// The folder the server was given (STORE); files are kept in a folder inside it that the
    /// owner can choose (`.starsheets-root` in here, see storage()).
    base: PathBuf,
    lock: Mutex<()>,
    location: String,
    auth: bool,
    site: Option<PathBuf>,
    rooms: live::Rooms,
}

type Shared = Arc<App>;

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

// ---------------------------------------------------------------- responses

/// JSON text with every non-ASCII character escaped, so it can go in an HTTP header
/// (what Python's json.dumps does by default).
fn json_ascii(s: &str) -> String {
    let mut out = String::new();
    for c in serde_json::to_string(s).unwrap_or_default().chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut buf = [0u16; 2];
            for u in c.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{u:04x}"));
            }
        }
    }
    out
}

fn send_json(v: &Value, code: u16) -> Response<Body> {
    Response::builder()
        .status(code)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(serde_json::to_vec(v).unwrap_or_default()))
        .unwrap()
}

fn error(code: u16, msg: &str) -> Response<Body> {
    send_json(&json!({"error": msg}), code)
}

fn reply(r: Result<Reply, Bad>) -> Response<Body> {
    match r {
        Ok(Reply::Json(v, code)) => send_json(&v, code),
        Ok(Reply::File { data, name, version, folder, modified }) => {
            let mut b = Response::builder()
                .status(200)
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header(header::CACHE_CONTROL, "no-store")
                .header("X-File-Name", json_ascii(&name))
                .header("X-Folder", json_ascii(&folder))
                .header("X-Modified", modified.to_string());
            if !version.is_empty() {
                b = b.header(header::ETAG, version);
            }
            // The apps read files with fetch(); a browser sent here directly saves the file
            // rather than showing it.
            b.header(header::CONTENT_DISPOSITION, "attachment").body(Body::from(data)).unwrap()
        }
        Err(e) => error(e.code, &e.msg),
    }
}

// ---------------------------------------------------------------- sign-in

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).collect::<Vec<_>>().join(";");
    cookies.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == COOKIE && !value.is_empty()).then(|| value.to_string())
    })
}

/// Who is asking (`None`: not signed in, or sign-in is off).
fn current_user(app: &App, headers: &HeaderMap) -> Option<auth::User> {
    if !app.auth {
        return None;
    }
    cookie_token(headers).and_then(|t| auth::session_user(&app.store, &t))
}

fn signed_in(app: &App, headers: &HeaderMap) -> bool {
    !app.auth || current_user(app, headers).is_some()
}

/// What the request says about who sent it: their address (for the try limits) and whether
/// it came over HTTPS (the sign-in cookie is then Secure). See guard.rs.
#[derive(Clone, Copy, Debug, Default)]
struct Ask {
    ip: Option<IpAddr>,
    secure: bool,
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn cookie_reply(value: &str, max_age: u64, body: Value, secure: bool) -> Response<Body> {
    let secure = if secure { "; Secure" } else { "" };
    Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::SET_COOKIE, format!("{COOKIE}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict{secure}"))
        .body(Body::from(serde_json::to_vec(&body).unwrap_or_default()))
        .unwrap()
}

/// Runs account work (Argon2 is slow) off the async threads, under the store lock.
async fn blocking<T: Send + 'static>(app: &Shared, f: impl FnOnce(&App) -> T + Send + 'static) -> Option<T> {
    let app = app.clone();
    tokio::task::spawn_blocking(move || {
        let _g = app.lock.lock().unwrap_or_else(|e| e.into_inner());
        f(&app)
    })
    .await
    .ok()
}

/// Runs password hashing or checking (Argon2: slow on purpose) off the async threads and NOT
/// under the store lock, a few at a time, so sign-in tries can't hold up everyone's files.
async fn slow<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let _turn = ARGON_AT_ONCE.acquire().await.ok()?;
    tokio::task::spawn_blocking(f).await.ok()
}

fn too_many() -> Response<Body> {
    error(429, "Too many tries. Wait a few minutes, then try again.")
}

fn session_reply(app: &App, user: &auth::User, secure: bool) -> Response<Body> {
    match auth::new_session(&app.store, &user.username) {
        Ok(t) => cookie_reply(&t, auth::SESSION_DAYS * 86400, json!({ "ok": true, "user": user.json() }), secure),
        Err(_) => error(500, "storage error"),
    }
}

fn field(body: &Value, key: &str) -> String {
    body.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn opt_field(body: &Value, key: &str) -> Option<String> {
    body.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Waits a moment after a wrong password or code (slows down guessing), then says so.
async fn wrong(code: u16, why: &str) -> Response<Body> {
    tokio::time::sleep(WRONG_WAIT).await;
    error(code, why)
}

/// /api/info, /api/setup, /api/signup, /api/login, /api/logout, /api/me and /api/users; `None` for other paths.
/// Password hashes are made and checked outside the store lock (slow()); sign-in, sign-up and
/// setup tries are limited per address and per username (guard.rs).
async fn accounts(
    app: &Shared,
    m: &str,
    p: &[String],
    q: &HashMap<String, String>,
    headers: &HeaderMap,
    body: &[u8],
    ask: Ask,
) -> Option<Response<Body>> {
    use guard::{LIMITS, ip_key};
    let path: Vec<&str> = p.iter().map(String::as_str).collect();
    let json: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let me = current_user(app, headers);
    let on = app.auth;
    let secure = ask.secure;
    Some(match (path.as_slice(), m) {
        (["info"], "GET") => {
            let setup = on && !auth::any_users(&app.store);
            let s = auth::signup(&app.store);
            let signup = on && !setup && s.on;
            send_json(
                &json!({ "location": app.location, "auth": on, "signed_in": !on || me.is_some(), "setup": setup, "signup": signup, "signup_code": signup && !s.code.is_empty(), "user": me.map(|u| u.json()) }),
                200,
            )
        }
        (["setup"], "POST") => {
            if !on {
                return Some(error(400, "Sign-in is off on this server."));
            }
            if !LIMITS.take(&ip_key("setup", ask.ip), guard::SETUP_IP) {
                return Some(too_many());
            }
            let (username, name, password, code) = (field(&json, "username"), field(&json, "name"), field(&json, "password"), field(&json, "code"));
            let (u2, n2) = (username.clone(), name.clone());
            let r = blocking(app, move |app| {
                if auth::any_users(&app.store) {
                    return Err((400, "This server already has an owner. Sign in instead.".to_string()));
                }
                if !same(code.trim().as_bytes(), auth::setup_code(&app.store).as_bytes()) {
                    return Err((401, "That setup code isn't right.".to_string()));
                }
                auth::check_new(&app.store, &u2, &n2).map_err(|e| (400, e))
            })
            .await;
            match r {
                Some(Ok(())) => {}
                Some(Err((401, why))) => return Some(wrong(401, &why).await),
                Some(Err((code, why))) => return Some(error(code, &why)),
                None => return Some(error(500, "server error")),
            }
            let hash = match slow(move || auth::hash_new(&password)).await {
                Some(Ok(h)) => h,
                Some(Err(why)) => return Some(error(400, &why)),
                None => return Some(error(500, "server error")),
            };
            let r = blocking(app, move |app| {
                if auth::any_users(&app.store) {
                    return Err((400, "This server already has an owner. Sign in instead.".to_string()));
                }
                let user = auth::add_hashed(&app.store, &username, &name, hash).map_err(|e| (400, e))?;
                let _ = std::fs::remove_file(app.store.meta().join("setup-code"));
                // Files already in the folder become the new owner's.
                spaces::migrate(&app.store).map_err(|e| (500, e.to_string()))?;
                Ok(session_reply(app, &user, secure))
            })
            .await;
            match r {
                Some(Ok(resp)) => resp,
                Some(Err((code, why))) => error(code, &why),
                None => error(500, "server error"),
            }
        }
        (["signup"], "POST") => {
            if !on {
                return Some(error(400, "Sign-in is off on this server."));
            }
            if !LIMITS.take(&ip_key("signup", ask.ip), guard::SIGNUP_IP) {
                return Some(too_many());
            }
            let (username, name, password, code) = (field(&json, "username"), field(&json, "name"), field(&json, "password"), field(&json, "code"));
            let (u2, n2) = (username.clone(), name.clone());
            let r = blocking(app, move |app| {
                auth::signup_allowed(&app.store, &code)?;
                auth::check_new(&app.store, &u2, &n2).map_err(|e| (400, e))
            })
            .await;
            match r {
                Some(Ok(())) => {}
                Some(Err((401, why))) => return Some(wrong(401, &why).await),
                Some(Err((code, why))) => return Some(error(code, &why)),
                None => return Some(error(500, "server error")),
            }
            let hash = match slow(move || auth::hash_new(&password)).await {
                Some(Ok(h)) => h,
                Some(Err(why)) => return Some(error(400, &why)),
                None => return Some(error(500, "server error")),
            };
            let r = blocking(app, move |app| {
                // Sign-up could have been turned off meanwhile.
                auth::signup(&app.store).on.then_some(()).ok_or((403, "This server doesn't let people make their own account.".to_string()))?;
                let user = auth::add_hashed(&app.store, &username, &name, hash).map_err(|e| (400, e))?;
                let _ = spaces::ensure_space(&app.store, &user.username);
                Ok(session_reply(app, &user, secure))
            })
            .await;
            match r {
                Some(Ok(resp)) => resp,
                Some(Err((code, why))) => error(code, &why),
                None => error(500, "server error"),
            }
        }
        (["signup", "settings"], "GET" | "POST") => {
            if me.as_ref().is_none_or(|u| !u.owner) {
                return Some(error(403, "Only the owner can do that."));
            }
            let posting = m == "POST";
            let r = blocking(app, move |app| {
                if posting {
                    auth::set_signup(&app.store, json.get("on").and_then(Value::as_bool).unwrap_or(false), &field(&json, "code"))
                } else {
                    Ok(auth::signup(&app.store))
                }
            })
            .await;
            match r {
                Some(Ok(s)) => send_json(&json!({ "on": s.on, "code": s.code }), 200),
                Some(Err(why)) => error(400, &why),
                None => error(500, "server error"),
            }
        }
        (["login"], "POST") => {
            if !on {
                return Some(send_json(&json!({ "ok": true }), 200));
            }
            let (username, password) = (field(&json, "username"), field(&json, "password"));
            let user_key = format!("login-user:{}", username.trim().to_lowercase());
            // Every try counts for the address; only wrong ones for the username.
            if !LIMITS.can(&user_key, guard::LOGIN_USER) || !LIMITS.take(&ip_key("login", ask.ip), guard::LOGIN_IP) {
                return Some(too_many());
            }
            let found = blocking(app, move |app| auth::login_hash(&app.store, &username)).await.flatten();
            let (hash, user) = found.map_or((None, None), |(u, h)| (Some(h), Some(u)));
            let right = slow(move || auth::verify(&password, hash.as_deref())).await.unwrap_or(false);
            match user.filter(|_| right) {
                Some(user) => blocking(app, move |app| session_reply(app, &user, secure)).await.unwrap_or_else(|| error(500, "server error")),
                None => {
                    LIMITS.take(&user_key, guard::LOGIN_USER);
                    wrong(401, "That username or password isn't right.").await
                }
            }
        }
        (["logout"], "POST") => {
            if let Some(t) = cookie_token(headers) {
                blocking(app, move |app| auth::end_session(&app.store, &t)).await;
            }
            cookie_reply("", 0, json!({ "ok": true }), secure)
        }
        (["me"], "POST") => {
            let Some(me) = me else { return Some(error(401, "please sign in")) };
            let mut new_hash = None;
            if let Some(new_password) = opt_field(&json, "password") {
                let user_key = format!("login-user:{}", me.username);
                if !LIMITS.can(&user_key, guard::LOGIN_USER) {
                    return Some(too_many());
                }
                let (current, who) = (field(&json, "current_password"), me.username.clone());
                let stored = blocking(app, move |app| auth::login_hash(&app.store, &who).map(|(_, h)| h)).await.flatten();
                if !slow(move || auth::verify(&current, stored.as_deref())).await.unwrap_or(false) {
                    LIMITS.take(&user_key, guard::LOGIN_USER);
                    return Some(wrong(400, "Your current password isn't right.").await);
                }
                new_hash = match slow(move || auth::hash_new(&new_password)).await {
                    Some(Ok(h)) => Some(h),
                    Some(Err(why)) => return Some(error(400, &why)),
                    None => return Some(error(500, "server error")),
                };
            }
            let r = blocking(app, move |app| {
                let changed_password = new_hash.is_some();
                auth::update_hashed(&app.store, &me.username, opt_field(&json, "name").as_deref(), opt_field(&json, "colour").as_deref(), new_hash)
                    .and_then(|u| {
                        if changed_password {
                            auth::new_session(&app.store, &u.username).map(|t| (u, Some(t))).map_err(|e| e.to_string())
                        } else {
                            Ok((u, None))
                        }
                    })
            })
            .await;
            match r {
                // A new password signs out everywhere else; this tab gets a new session.
                Some(Ok((u, Some(t)))) => cookie_reply(&t, auth::SESSION_DAYS * 86400, json!({ "ok": true, "user": u.json() }), secure),
                Some(Ok((u, None))) => send_json(&json!({ "ok": true, "user": u.json() }), 200),
                Some(Err(why)) => error(400, &why),
                None => error(500, "server error"),
            }
        }
        (["users"], "GET") => {
            if !signed_in(app, headers) {
                return Some(error(401, "please sign in"));
            }
            let list = blocking(app, |app| auth::users(&app.store)).await.unwrap_or_default();
            send_json(&Value::Array(list.iter().map(auth::User::json).collect()), 200)
        }
        (["users"] | ["users", "password"], "POST" | "DELETE") => {
            let Some(me) = me.filter(|u| u.owner) else { return Some(error(403, "Only the owner can do that.")) };
            let (reset, deleting) = (path.len() == 2, m == "DELETE");
            let who = q.get("username").cloned().unwrap_or_default();
            // A new or reset password is hashed first, outside the store lock.
            let hash = if deleting {
                None
            } else {
                let password = field(&json, "password");
                match slow(move || auth::hash_new(&password)).await {
                    Some(Ok(h)) => Some(h),
                    Some(Err(why)) => return Some(error(400, &why)),
                    None => return Some(error(500, "server error")),
                }
            };
            let r = blocking(app, move |app| {
                if deleting {
                    if who == me.username {
                        return Err("You can't remove your own account.".to_string());
                    }
                    auth::remove(&app.store, &who)?;
                    // Their folder goes to the owner as "Removed – <username>"; their shares go.
                    let moved = spaces::retire(&app.store, &who).map_err(|e| e.to_string())?;
                    app.rooms.recheck_access();
                    if let Some(to) = &moved {
                        log::note(&app.store, &format!("account {who:?} removed; their folder is now {to:?}"));
                    }
                    Ok(json!({ "ok": true, "folder": moved }))
                } else if reset {
                    auth::update_hashed(&app.store, &field(&json, "username"), None, None, hash).map(|u| u.json())
                } else {
                    let hash = hash.unwrap_or_default();
                    auth::add_hashed(&app.store, &field(&json, "username"), &field(&json, "name"), hash).map(|u| {
                        let _ = spaces::ensure_space(&app.store, &u.username);
                        u.json()
                    })
                }
            })
            .await;
            match r {
                Some(Ok(v)) => send_json(&v, 200),
                Some(Err(why)) => error(400, &why),
                None => error(500, "server error"),
            }
        }
        _ => return None,
    })
}

// ---------------------------------------------------------------- sharing

/// Checks a files/folders/bin request against what the person may do, fills in where things go
/// by default (their own folder), and answers sharing and star requests itself. Returns the
/// query to use, or the answer to send.
async fn gate(
    app: &Shared,
    m: &str,
    p: &[String],
    mut q: HashMap<String, String>,
    me: Option<auth::User>,
    body: &[u8],
) -> Result<HashMap<String, String>, Response<Body>> {
    if !app.auth {
        return Ok(q);
    }
    let Some(me) = me else { return Err(error(401, "please sign in")) };
    let (p, m) = (p.to_vec(), m.to_string());
    let json: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let r = blocking(app, move |app| -> Result<HashMap<String, String>, Response<Body>> {
        use sharing::{Item, Role};
        let store = &app.store;
        let _ = spaces::ensure_space(store, &me.username);
        let a = sharing::Access::new(store, true, Some(&me));
        let need = |have: Role, want: Role| -> Result<(), Response<Body>> {
            if have >= want {
                Ok(())
            } else if have == Role::None {
                Err(error(403, "You don't have access to that."))
            } else if want == Role::Owner {
                Err(error(403, "Only its owner can do that."))
            } else {
                Err(error(403, "You can only view that."))
            }
        };
        let home = me.username.clone();
        let rest: Vec<&str> = p.iter().map(String::as_str).collect();
        let path_q = |q: &HashMap<String, String>, k: &str| store::rel(q.get(k).map(String::as_str)).map_err(|b| error(b.code, &b.msg));
        match (rest.as_slice(), m.as_str()) {
            (["files"], "GET") => {}
            (["files"], "POST") => {
                let mut folder = path_q(&q, "folder")?;
                if folder.is_empty() {
                    folder = home.clone();
                }
                need(a.folder_role(&folder), Role::Edit)?;
                q.insert("folder".into(), folder);
            }
            (["files", id, sub @ ..], _) => {
                let role = a.file_role(id);
                let path = a.file_paths.get(*id).cloned().unwrap_or_default();
                match (sub, m.as_str()) {
                    ([], "GET") => {
                        need(role, Role::View)?;
                        let _ = spaces::note_opened(store, &me, id);
                    }
                    (["versions", ..], "GET") => need(role, Role::View)?,
                    ([], "PUT") | (["rename"], "POST") | ([], "DELETE") => need(role, Role::Edit)?,
                    (["copy"], "POST") => {
                        need(role, Role::View)?;
                        let parent = store::parent_of(&path);
                        let target = match q.get("folder") {
                            Some(f) => store::rel(Some(f)).map_err(|b| error(b.code, &b.msg))?,
                            None if a.folder_role(&parent) >= Role::Edit && !parent.is_empty() => parent,
                            None => home.clone(),
                        };
                        need(a.folder_role(&target), Role::Edit)?;
                        q.insert("folder".into(), target);
                    }
                    (["move"], "POST") => {
                        let target = path_q(&q, "folder")?;
                        need(a.folder_role(&target), Role::Edit)?;
                        // Into someone else's folder: it becomes theirs, so only its owner may.
                        need(role, if store::space_of(&target) == store::space_of(&path) { Role::Edit } else { Role::Owner })?;
                    }
                    (["sharing"], "GET") => {
                        need(role, Role::View)?;
                        return Err(send_json(&sharing::describe(store, Item::File(id), &path, role), 200));
                    }
                    (["sharing"], "POST") => {
                        need(role, Role::Owner)?;
                        return Err(match sharing::update(store, Item::File(id), &path, &json) {
                            Ok(()) => send_json(&sharing::describe(store, Item::File(id), &path, role), 200),
                            Err(why) => error(400, &why),
                        });
                    }
                    (["star"], "POST") => {
                        need(role, Role::View)?;
                        let on = json.get("on").and_then(Value::as_bool).unwrap_or(true);
                        return Err(match spaces::set_star(store, &me, &format!("f:{id}"), on) {
                            Ok(()) => send_json(&json!({ "ok": true }), 200),
                            Err(e) => error(500, &e.to_string()),
                        });
                    }
                    _ => need(role, Role::Owner)?,
                }
            }
            (["folders", sub @ ..], _) => {
                let mut path = path_q(&q, "path")?;
                match (sub, m.as_str()) {
                    ([], "POST") => {
                        if path.is_empty() {
                            path = home.clone();
                            q.insert("path".into(), path.clone());
                        }
                        need(a.folder_role(&path), Role::Edit)?;
                    }
                    (["rename"], "POST") | ([], "DELETE") => need(a.folder_role(&path), Role::Edit)?,
                    (["move"], "POST") => {
                        let to = path_q(&q, "to")?;
                        need(a.folder_role(&to), Role::Edit)?;
                        need(a.folder_role(&path), if store::space_of(&to) == store::space_of(&path) { Role::Edit } else { Role::Owner })?;
                    }
                    (["copy"], "POST") => {
                        need(a.folder_role(&path), Role::View)?;
                        if !q.contains_key("to") {
                            q.insert("to".into(), home.clone());
                        }
                        need(a.folder_role(&path_q(&q, "to")?), Role::Edit)?;
                    }
                    (["sharing" | "star" | "colour"], _) => {
                        let role = a.folder_role(&path);
                        let Some(id) = a.folder_ids.get(&path).cloned() else { return Err(error(404, "no such folder")) };
                        if sub == ["sharing"] && (path == home || (!path.contains('/') && !path.is_empty())) {
                            return Err(error(400, "Share the folders inside it instead."));
                        }
                        return Err(match (sub, m.as_str()) {
                            (["sharing"], "GET") => {
                                need(role, Role::View)?;
                                send_json(&sharing::describe(store, Item::Folder(&id), &path, role), 200)
                            }
                            (["sharing"], "POST") => {
                                need(role, Role::Owner)?;
                                match sharing::update(store, Item::Folder(&id), &path, &json) {
                                    Ok(()) => send_json(&sharing::describe(store, Item::Folder(&id), &path, role), 200),
                                    Err(why) => error(400, &why),
                                }
                            }
                            (["colour"], "POST") => {
                                need(role, Role::View)?;
                                match spaces::set_colour(store, &me, &id, json.get("colour").and_then(Value::as_str).unwrap_or("")) {
                                    Ok(()) => send_json(&json!({ "ok": true }), 200),
                                    Err(e) => error(500, &e.to_string()),
                                }
                            }
                            (["star"], "POST") => {
                                need(role, Role::View)?;
                                let on = json.get("on").and_then(Value::as_bool).unwrap_or(true);
                                match spaces::set_star(store, &me, &format!("d:{id}"), on) {
                                    Ok(()) => send_json(&json!({ "ok": true }), 200),
                                    Err(e) => error(500, &e.to_string()),
                                }
                            }
                            _ => error(404, "not found"),
                        });
                    }
                    _ => need(a.folder_role(&path), Role::Owner)?,
                }
            }
            (["bin", ..], _) => {
                q.insert("space".into(), home.clone());
            }
            _ => {}
        }
        Ok(q)
    })
    .await;
    r.unwrap_or_else(|| Err(error(500, "server error")))
}

// ---------------------------------------------------------------- where files are kept

/// The folder (inside the base folder) chosen to keep files in, from `<base>/.starsheets-root`;
/// None or "" = the base folder itself.
fn chosen_root(base: &Path) -> Option<String> {
    let s = std::fs::read_to_string(base.join(".starsheets-root")).ok()?;
    store::rel(Some(s.trim())).ok()
}

/// Folders inside the base folder someone could choose (not hidden ones), three levels deep.
fn storage_folders(base: &Path) -> Vec<String> {
    fn walk(base: &Path, here: &str, depth: usize, out: &mut Vec<String>) {
        if depth > 3 || out.len() > 500 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(if here.is_empty() { base.to_path_buf() } else { base.join(here) }) else { return };
        let mut dirs: Vec<String> = rd
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .collect();
        dirs.sort_by_key(|d| d.to_lowercase());
        for d in dirs {
            let p = store::join(here, &d);
            out.push(p.clone());
            walk(base, &p, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(base, "", 1, &mut out);
    out
}

/// Whether a folder already holds StarSheets files (people's accounts).
fn has_starsheets(dir: &Path) -> bool {
    dir.join(".starsheets").join("users.json").is_file()
}

/// GET/POST /api/storage: where files are kept. The server's owner (or, while the server is
/// being set up, whoever has the setup code: `code` in the query or the body; or anyone with
/// sign-in off) can choose another folder inside the base folder: "move" takes everything there
/// (files, versions, bin, accounts), "use" switches to StarSheets files already there. Refused
/// while a spreadsheet is open. The server then restarts itself (exit; Docker starts it again)
/// so nothing still points at the old place.
async fn storage(app: &Shared, m: &str, me: Option<auth::User>, body: &[u8], q: &HashMap<String, String>, ask: Ask) -> Response<Body> {
    let setting_up = app.auth && !auth::any_users(&app.store);
    if app.auth && !setting_up && !me.as_ref().is_some_and(|u| u.owner) {
        return error(403, "Only the owner of this server can choose where files are kept.");
    }
    let current = chosen_root(&app.base).unwrap_or_default();
    let json: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    if setting_up {
        let code = if m == "GET" { q.get("code").cloned().unwrap_or_default() } else { field(&json, "code") };
        let right = !code.trim().is_empty()
            && blocking(app, move |a| same(code.trim().as_bytes(), auth::setup_code(&a.store).as_bytes())).await.unwrap_or(false);
        if !right {
            let tried = !q.get("code").is_none_or(|c| c.trim().is_empty()) || !field(&json, "code").trim().is_empty();
            if tried && !guard::LIMITS.take(&guard::ip_key("setup", ask.ip), guard::SETUP_IP) {
                return too_many();
            }
            if m == "GET" {
                // The setup page asks again once the code is typed; until then, no folder list.
                return send_json(
                    &json!({ "location": app.location, "current": current, "folders": [], "has_files": [], "top_has_files": false, "locked": true }),
                    200,
                );
            }
            return wrong(401, "That setup code isn't right.").await;
        }
    }
    if m == "GET" {
        let base = app.base.clone();
        let mut folders = tokio::task::spawn_blocking(move || storage_folders(&base)).await.unwrap_or_default();
        // People's own folders (and what's in them) are StarSheets' own: never a place to choose.
        let spaces: Vec<String> = auth::users(&app.store).into_iter().map(|u| store::join(&current, &u.username)).collect();
        folders.retain(|f| !spaces.iter().any(|s| f == s || f.starts_with(&format!("{s}/"))));
        let ready: Vec<String> = folders.iter().filter(|f| has_starsheets(&app.base.join(f))).cloned().collect();
        return send_json(
            &json!({ "location": app.location, "current": current, "folders": folders, "has_files": ready, "top_has_files": has_starsheets(&app.base) }),
            200,
        );
    }
    let Ok(to) = store::rel(json.get("folder").and_then(Value::as_str)) else { return error(400, "That isn't a folder name.") };
    let mode = field(&json, "mode");
    if to == current {
        return send_json(&json!({ "ok": true, "restarting": false }), 200);
    }
    let app2 = app.clone();
    let r = blocking(app, move |_| -> Result<(), String> {
        if app2.rooms.open_count() > 0 {
            return Err("A spreadsheet is open right now (in some tab, maybe someone else's). Close it and try again in a minute.".into());
        }
        let old = app2.store.root.clone();
        let new = if to.is_empty() { app2.base.clone() } else { app2.base.join(&to) };
        std::fs::create_dir_all(&new).map_err(|e| e.to_string())?;
        match mode.as_str() {
            "use" => {
                if !has_starsheets(&new) && auth::any_users(&app2.store) {
                    return Err("There are no StarSheets files in that folder to switch to. Choose \"Move my files there\" instead.".into());
                }
            }
            _ => {
                // A folder inside the one in use is fine unless it is StarSheets' own (someone's
                // folder, the bin, the hidden data).
                if let Ok(rest) = new.strip_prefix(&old) {
                    let first = rest.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
                    if first.is_empty() || first.starts_with('.') || auth::users(&app2.store).iter().any(|u| u.username == first) {
                        return Err("That folder belongs to StarSheets itself. Pick another one.".into());
                    }
                }
                // Everything moves, or nothing: check for clashes first.
                let entries: Vec<std::fs::DirEntry> =
                    std::fs::read_dir(&old).map_err(|e| e.to_string())?.flatten().filter(|e| e.file_name() != ".starsheets-root").collect();
                let clash: Vec<String> = entries
                    .iter()
                    .map(|e| e.file_name())
                    .filter(|n| new.join(n).exists() && old.join(n) != new)
                    .map(|n| n.to_string_lossy().into_owned())
                    .collect();
                if !clash.is_empty() {
                    return Err(format!("That folder already has {} in it. Choose an empty folder, or tidy it first.", clash.join(", ")));
                }
                for e in entries {
                    if e.path() == new || new.starts_with(e.path()) {
                        continue;
                    }
                    let to_path = new.join(e.file_name());
                    if std::fs::rename(e.path(), &to_path).is_err() {
                        // Another file system: copy, then remove.
                        let status = std::process::Command::new("cp").arg("-a").arg(e.path()).arg(&to_path).status().map_err(|e| e.to_string())?;
                        if !status.success() {
                            return Err("Copying to the new folder failed; nothing was removed.".into());
                        }
                        let _ = if e.path().is_dir() { std::fs::remove_dir_all(e.path()) } else { std::fs::remove_file(e.path()) };
                    }
                }
            }
        }
        std::fs::write(app2.base.join(".starsheets-root"), format!("{to}\n")).map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|| Err("server error".into()));
    match r {
        Ok(()) => {
            log::note(
                &app.store,
                &format!(
                    "files now kept in {:?}; restarting",
                    if json["folder"].as_str().unwrap_or("").is_empty() { "the top folder" } else { json["folder"].as_str().unwrap_or("") }
                ),
            );
            tokio::spawn(async {
                tokio::time::sleep(Duration::from_millis(600)).await;
                std::process::exit(0);
            });
            send_json(&json!({ "ok": true, "restarting": true }), 200)
        }
        Err(why) => error(400, &why),
    }
}

// ---------------------------------------------------------------- backups

/// /api/backup… (the owner only; anyone with sign-in off): scheduled, encrypted backups in
/// restic's format (see backup.rs).
/// GET backup: settings and how the last one went. GET backup/key: the recovery key.
/// GET backup/list: the backups kept. POST backup {to, on, hour, key?}: choose the folder (an empty
/// one is set up; one with backups in it opens with the key given, or this server's key).
/// POST backup/run: one now. GET backup/people: whose folders the backups hold. POST
/// backup/restore {id, whose?}: copies a folder out of backup ID into a new folder "Restored from
/// backup <when>" (backup::whose_folder, restore_into): the asker's own, or (the owner) anyone's
/// or "" everyone's.
async fn backup_api(app: &Shared, m: &str, p: &[String], me: Option<auth::User>, body: &[u8]) -> Response<Body> {
    if app.auth && !me.as_ref().is_some_and(|u| u.owner) {
        return error(403, "Only the owner of this server can look after backups.");
    }
    let base = app.base.clone();
    let json: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let what = p.get(1).map(String::as_str).unwrap_or("");
    let work = {
        let app = app.clone();
        let m = m.to_string();
        let what = what.to_string();
        move || -> Result<Value, (u16, String)> {
            let bad = |e: String| (400u16, e);
            match (m.as_str(), what.as_str()) {
                ("GET", "") => {
                    let s = backup::load(&base);
                    let suggest = ["/backups", "/backup"].into_iter().find(|d| Path::new(d).is_dir()).unwrap_or("");
                    Ok(json!({
                        "set_up": s.get("to").and_then(Value::as_str).is_some_and(|t| !t.is_empty()),
                        "on": s.get("on").and_then(Value::as_bool).unwrap_or(false),
                        "to": s.get("to").cloned().unwrap_or(json!("")),
                        "hour": s.get("hour").and_then(Value::as_i64).unwrap_or(3),
                        "timezone": jiff::Zoned::now().time_zone().iana_name().unwrap_or("UTC"),
                        "running": backup::running(),
                        "last": s.get("last").cloned().unwrap_or(Value::Null),
                        "last_ok": s.get("last_ok").cloned().unwrap_or(Value::Null),
                        "suggest": suggest,
                    }))
                }
                ("GET", "key") => Ok(json!({ "key": backup::load(&base).get("key").cloned().unwrap_or(Value::Null) })),
                ("GET", "list") => backup::list(&base).map(|l| json!(l)).map_err(bad),
                ("POST", "") => {
                    let to = json.get("to").and_then(Value::as_str).unwrap_or("").trim().trim_end_matches('/').to_string();
                    let s = backup::load(&base);
                    let mut key = s.get("key").and_then(Value::as_str).map(str::to_string);
                    let given = json.get("key").and_then(Value::as_str).map(backup::clean_key).filter(|k| !k.is_empty());
                    if given.is_some() {
                        key = given;
                    }
                    let key = key.unwrap_or_else(backup::new_key);
                    let hour = json.get("hour").and_then(Value::as_i64).unwrap_or(3).clamp(0, 23);
                    let on = json.get("on").and_then(Value::as_bool).unwrap_or(true);
                    let mut said = "saved";
                    if s.get("to").and_then(Value::as_str) != Some(to.as_str()) || s.get("key").and_then(Value::as_str) != Some(key.as_str()) {
                        if to.is_empty() {
                            return Err(bad("Choose a folder for the backups.".into()));
                        }
                        said = backup::prepare(&to, &key).map_err(bad)?;
                    }
                    backup::update(&base, |s| {
                        s.insert("to".into(), json!(to));
                        s.insert("key".into(), json!(key));
                        s.insert("hour".into(), json!(hour));
                        s.insert("on".into(), json!(on));
                    })
                    .map_err(|e| (500, e.to_string()))?;
                    log::note(&app.store, &format!("backups: {said} in {to}, daily at {hour}:00, {}", if on { "on" } else { "off" }));
                    Ok(json!({ "ok": true, "result": said }))
                }
                ("POST", "run") => {
                    if backup::running() {
                        return Err(bad("A backup is running already.".into()));
                    }
                    let app2 = app.clone();
                    std::thread::spawn(move || {
                        let r = backup::run(&app2.base, &app2.store.root, "now");
                        log::note(&app2.store, &format!("backup (asked for): {}", r.unwrap_or_else(|e| format!("FAILED: {e}"))));
                    });
                    Ok(json!({ "ok": true, "running": true }))
                }
                ("POST", "restore") => {
                    let id = json.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                    let list = backup::list(&base).map_err(bad)?;
                    let Some(snap) = list.iter().find(|s| s["id"].as_str() == Some(id.as_str())) else {
                        return Err(bad("That backup isn't there any more.".into()));
                    };
                    let asker = me.as_ref().map(|u| u.username.as_str());
                    let owner = me.as_ref().is_none_or(|u| u.owner);
                    let whose = backup::whose_folder(asker, owner, json.get("whose").and_then(Value::as_str)).map_err(|e| (403, e))?;
                    let label = snap["label"].as_str().unwrap_or("");
                    let rel = backup::restore_into(label, &whose, asker, |p| app.store.full(p).exists());
                    let files = backup::restore(&base, &id, &whose, &app.store.full(&rel)).map_err(bad)?;
                    log::note(&app.store, &format!("backup {}: restored {files} files into {rel:?}", snap["short"].as_str().unwrap_or("")));
                    Ok(json!({ "ok": true, "folder": rel, "files": files }))
                }
                // Whose files the backups hold (one folder each), and which of them have an account.
                ("GET", "people") => {
                    let have: Vec<String> = auth::users(&app.store).into_iter().map(|u| u.username).collect();
                    let people = backup::people(&base).map_err(bad)?;
                    Ok(json!(people.iter().map(|p| json!({ "name": p, "account": have.contains(p) })).collect::<Vec<_>>()))
                }
                _ => Err((404, "not found".into())),
            }
        }
    };
    // Backups can be slow (a big NAS folder): off the async threads, and not under the store lock.
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok(v)) => send_json(&v, 200),
        Ok(Err((code, why))) => error(code, &why),
        Err(_) => error(500, "server error"),
    }
}

/// Checks once a minute whether a scheduled backup is due (backup::due) and makes it.
async fn backup_schedule(app: Shared) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    loop {
        tick.tick().await;
        let app2 = app.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if backup::running() || !backup::due(&backup::load(&app2.base), &jiff::Zoned::now()) {
                return;
            }
            let r = backup::run(&app2.base, &app2.store.root, "daily");
            log::note(&app2.store, &format!("backup (daily): {}", r.unwrap_or_else(|e| format!("FAILED: {e}"))));
        })
        .await;
    }
}

/// POST /api/files/ID/add-sheets: copies the sheets of the spreadsheet in the body (?name= gives
/// its type) to the end of file ID, through its live room. Needs edit rights on ID.
async fn add_sheets(app: &Shared, me: Option<auth::User>, file: String, q: &HashMap<String, String>, body: Bytes) -> Response<Body> {
    let (app2, f2, me2) = (app.clone(), file.clone(), me.clone());
    let (role, path) =
        blocking(app, move |a| (sharing::file_role(&a.store, a.auth, me2.as_ref(), &f2), a.store.path_of(&f2).ok().flatten().unwrap_or_default()))
            .await
            .unwrap_or_default();
    // A .csv (or other one-sheet type) can't hold another tab.
    if !matches!(path.rsplit_once('.').map(|(_, e)| e.to_lowercase()).as_deref(), Some("xlsx" | "xlsm" | "ods")) && role > sharing::Role::None {
        return error(400, "That file can only hold one sheet (it's a .csv or similar). Pick an .xlsx spreadsheet, or copy to a new spreadsheet.");
    }
    if role < sharing::Role::Edit {
        return error(
            403,
            if role == sharing::Role::None { "You don't have access to that spreadsheet." } else { "You can only view that spreadsheet." },
        );
    }
    let name = q.get("name").cloned().unwrap_or_else(|| "sheet.xlsx".into());
    let by = me.map(|u| u.username).unwrap_or_default();
    let r = tokio::task::spawn_blocking(move || app2.rooms.add_sheets(&app2, &file, name, body.to_vec(), by)).await;
    match r {
        Ok(Ok(names)) => send_json(&json!({ "ok": true, "sheets": names }), 200),
        Ok(Err(why)) => error(400, &why),
        Err(_) => error(500, "server error"),
    }
}

/// GET /api/files/ID/thumb: the file's preview picture (thumbs.rs), drawn if there isn't one for
/// this version yet (a document's is the one StarDocs sent, if it shows this version). Outside the store lock while drawing. 404: no picture (the app shows an icon).
async fn thumb(app: &Shared, me: Option<auth::User>, file: String, headers: &HeaderMap) -> Response<Body> {
    enum Got {
        Have(Vec<u8>, String),
        Draw(String, Vec<u8>, String),
    }
    let f2 = file.clone();
    let got = blocking(app, move |app| -> Result<Got, Response<Body>> {
        if sharing::file_role(&app.store, app.auth, me.as_ref(), &f2) < sharing::Role::View {
            return Err(error(403, "You don't have access to that."));
        }
        let path = app.store.path_of(&f2).ok().flatten().ok_or_else(|| error(404, "no such file"))?;
        let full = app.store.full(&path);
        let version = app.store.version_of(&full).map_err(|_| error(404, "no such file"))?;
        if let Some(png) = thumbs::cached(&app.store, &f2, &version) {
            return Ok(Got::Have(png, version));
        }
        // A document's picture comes from StarDocs (put_thumb); none for this version yet.
        if store::is_doc(&path) {
            return Err(error(404, "no picture of this version yet"));
        }
        if std::fs::metadata(&full).map(|m| m.len()).unwrap_or(0) > thumbs::MAX_READ {
            return Err(error(404, "too big for a picture"));
        }
        let bytes = std::fs::read(&full).map_err(|_| error(404, "no such file"))?;
        Ok(Got::Draw(store::base_of(&path), bytes, version))
    })
    .await
    .unwrap_or_else(|| Err(error(500, "server error")));
    let (png, version) = match got {
        Err(resp) => return resp,
        Ok(Got::Have(png, v)) => (png, v),
        Ok(Got::Draw(name, bytes, v)) => {
            let (store_root, f3, v2) = (app.clone(), file.clone(), v.clone());
            let drawn = tokio::task::spawn_blocking(move || {
                let png = thumbs::from_bytes(&name, &bytes)?;
                let _ = thumbs::keep(&store_root.store, &f3, &v2, &png);
                Ok::<_, String>(png)
            })
            .await;
            match drawn {
                Ok(Ok(png)) => (png, v),
                _ => return error(404, "no picture"),
            }
        }
    };
    let etag = format!("\"{}/{version}\"", thumbs::DRAW_TAG);
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        return Response::builder().status(304).header(header::ETAG, etag).body(Body::empty()).unwrap();
    }
    Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, "image/png")
        .header(header::CACHE_CONTROL, "private, no-cache")
        .header(header::ETAG, etag)
        .body(Body::from(png))
        .unwrap()
}

/// PUT /api/files/ID/thumb?version=V: StarDocs' picture of the top of page 1 of document ID,
/// taken right after saving version V (thumbs::take says what's refused).
async fn put_thumb(app: &Shared, me: Option<auth::User>, file: String, q: &HashMap<String, String>, body: Bytes) -> Response<Body> {
    let version = q.get("version").cloned().unwrap_or_default();
    let r = blocking(app, move |a| {
        let role = sharing::file_role(&a.store, a.auth, me.as_ref(), &file);
        let path = a.store.path_of(&file).ok().flatten();
        match path {
            None if role == sharing::Role::None => Err(Bad::new(403, "You don't have access to that.")),
            None => Err(Bad::new(404, "no such file")),
            Some(path) => thumbs::take(&a.store, role, &file, &path, &version, &body),
        }
    })
    .await;
    match r {
        Some(Ok(())) => send_json(&json!({ "ok": true }), 200),
        Some(Err(b)) => error(b.code, &b.msg),
        None => error(500, "server error"),
    }
}

/// After a files request, with accounts: the file list shows only what the person may see, with
/// who owns each thing, their role, stars and when they last opened it, and the tops of "Shared
/// with me".
fn after_files(app: &App, me: &auth::User, m: &str, p: &[String], r: Result<Reply, Bad>) -> Result<Reply, Bad> {
    let Ok(Reply::Json(v, code)) = r else { return r };
    if !(p.len() == 1 && p[0] == "files" && m == "GET") {
        return Ok(Reply::Json(v, code));
    }
    let a = sharing::Access::new(&app.store, true, Some(me));
    let starred = spaces::starred(&app.store, me);
    let opened = spaces::opened(&app.store, me);
    let colours = spaces::colours(&app.store, me);
    let changed = spaces::changed_by(&app.store);
    let files: Vec<Value> = v["files"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|mut f| {
            let id = f["id"].as_str()?.to_string();
            if !a.file_listed(&id) {
                return None;
            }
            let folder = f["folder"].as_str().unwrap_or("").to_string();
            f["owner"] = json!(store::space_of(&folder));
            f["role"] = json!(a.file_role(&id).name());
            f["starred"] = json!(starred.contains(&format!("f:{id}")));
            f["shared"] = json!(a.file_shared(&id));
            if let Some(by) = changed.get(&id) {
                f["changed_by"] = by.clone();
            }
            if let Some(t) = opened.get(&id) {
                f["opened"] = t.clone();
            }
            Some(f)
        })
        .collect();
    let folders: Vec<String> = v["folders"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|f| f.as_str().map(str::to_string))
        .filter(|f| a.folder_listed(f))
        .collect();
    let info: serde_json::Map<String, Value> = folders
        .iter()
        .map(|f| {
            let id = a.folder_ids.get(f).cloned().unwrap_or_default();
            (f.clone(), json!({ "id": id, "owner": store::space_of(f), "role": a.folder_role(f).name(), "shared": a.folder_shared(f), "starred": starred.contains(&format!("d:{id}")), "colour": colours.get(&id).cloned().unwrap_or(json!("")) }))
        })
        .collect();
    let (shared_folders, shared_files) = a.shared_roots();
    let people: Vec<Value> = auth::users(&app.store).iter().map(|u| json!({ "username": u.username, "name": u.name, "colour": u.colour })).collect();
    Ok(Reply::Json(
        json!({
            "files": files, "folders": folders, "folder_info": info, "space": me.username, "admin": me.owner,
            "shared": { "folders": shared_folders, "files": shared_files }, "people": people,
        }),
        code,
    ))
}

// ---------------------------------------------------------------- the web app's files

fn is_hashed(name: &str) -> bool {
    let b = name.as_bytes();
    (0..b.len()).any(|i| {
        b[i] == b'-'
            && b.len() > i + 17
            && b[i + 1..i + 17].iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
            && matches!(b[i + 17], b'.' | b'_')
    })
}

async fn serve_site(site: &Path, path: &str, query: Option<&str>, headers: &HeaderMap) -> Response<Body> {
    let mut rel = PathBuf::new();
    let mut escapes = false;
    for c in Path::new(path.trim_start_matches('/')).components() {
        match c {
            Component::Normal(p) => rel.push(p),
            Component::ParentDir => escapes = true,
            _ => {}
        }
    }
    if rel.as_os_str().is_empty() {
        rel.push("index.html");
    }
    let mut full = site.join(&rel);
    // A folder with its own page (StarDocs in docs/) serves that page.
    if !escapes && full.is_dir() && full.join("index.html").is_file() {
        // `/docs` → `docs/`: the page's files are relative to its folder. The redirect is
        // relative too, so a proxy serving everything under a path of its own still works.
        if !path.ends_with('/') {
            let name = rel.file_name().unwrap_or_default().to_string_lossy();
            let to = format!("{name}/{}", query.map(|q| format!("?{q}")).unwrap_or_default());
            return Response::builder().status(308).header(header::LOCATION, to).body(Body::empty()).unwrap();
        }
        full = full.join("index.html");
    }
    if escapes || !full.is_file() {
        let base = rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if base.contains('.') {
            return error(404, "not found");
        }
        full = site.join("index.html");
        if !full.is_file() {
            return error(404, "not found");
        }
    }
    let name = full.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let kind =
        if name.ends_with(".wasm") { "application/wasm".to_string() } else { mime_guess::from_path(&full).first_or_octet_stream().to_string() };
    let gz = PathBuf::from(format!("{}.gz", full.display()));
    let zipped = headers.get(header::ACCEPT_ENCODING).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("gzip")) && gz.is_file();
    let Ok(data) = tokio::fs::read(if zipped { &gz } else { &full }).await else {
        return error(404, "not found");
    };
    let mut b = Response::builder().status(200).header(header::CONTENT_TYPE, kind).header("X-Content-Type-Options", "nosniff");
    if zipped {
        b = b.header(header::CONTENT_ENCODING, "gzip").header(header::VARY, "Accept-Encoding");
    }
    if name == "index.html" || name == "build.json" {
        b = b.header(header::CACHE_CONTROL, "no-cache");
    } else if is_hashed(&name) {
        b = b.header(header::CACHE_CONTROL, "public, max-age=31536000, immutable");
    }
    b.body(Body::from(data)).unwrap()
}

// ---------------------------------------------------------------- requests

/// Requests whose body can be a whole file (up to MAX_BYTES); every other body is small JSON.
fn takes_a_file(m: &str, p: &[String]) -> bool {
    let p: Vec<&str> = p.iter().map(String::as_str).collect();
    matches!((m, p.as_slice()), ("POST", ["files"]) | ("PUT", ["files", _]) | ("POST", ["files", _, "add-sheets"]))
}

/// Paths answered before (or without) sign-in: accounts and, while setting up, storage.
fn before_sign_in(p: &[String]) -> bool {
    matches!(p.first().map(String::as_str), Some("info" | "setup" | "signup" | "login" | "logout" | "me" | "users" | "storage"))
}

/// Reads a request's body: at most `limit` bytes, within `wait`.
async fn read_body(body: Body, headers: &HeaderMap, limit: u64, wait: Duration) -> Result<Bytes, Response<Body>> {
    let length: u64 = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()).unwrap_or(0);
    let too_big = || error(413, if limit == MAX_BYTES { "file too large" } else { "request too large" });
    if length > limit {
        return Err(too_big());
    }
    match tokio::time::timeout(wait, axum::body::to_bytes(body, limit as usize)).await {
        Ok(Ok(b)) => Ok(b),
        Ok(Err(_)) => Err(too_big()),
        Err(_) => Err(error(408, "The request took too long to arrive.")),
    }
}

async fn handle(State(app): State<Shared>, req: Request) -> Response<Body> {
    let (parts, body) = req.into_parts();
    let (method, uri, headers) = (parts.method, parts.uri, parts.headers);
    let peer = guard::peer(&parts.extensions);
    let ask = Ask { ip: guard::client_ip(peer, &headers, guard::config()), secure: guard::secure(peer, &headers, guard::config()) };
    let path = uri.path().to_string();
    if path == "/healthz" {
        return Response::builder().header(header::CONTENT_TYPE, "text/plain").body(Body::from("ok")).unwrap();
    }
    if let Some(site) = &app.site
        && !path.starts_with("/api/")
        // HEAD too (status checks use it); hyper leaves the body out of a HEAD reply.
        && (method == Method::GET || method == Method::HEAD)
    {
        return serve_site(site, &path, uri.query(), &headers).await;
    }
    let parts: Vec<String> = path.split('/').filter(|x| !x.is_empty()).map(|x| percent_decode_str(x).decode_utf8_lossy().into_owned()).collect();
    if parts.first().map(String::as_str) != Some("api") {
        return error(404, "not found");
    }
    let p: Vec<String> = parts[1..].to_vec();
    let mut q = HashMap::new();
    for (k, v) in form_urlencoded::parse(uri.query().unwrap_or("").as_bytes()) {
        q.entry(k.into_owned()).or_insert_with(|| v.into_owned());
    }
    let m = method.as_str().to_string();
    // Nothing is read (a body can be big) before we know who is asking: only the account and
    // setup requests come before sign-in, with small bodies.
    let early = before_sign_in(&p);
    if !early && !signed_in(&app, &headers) {
        return error(401, "please sign in");
    }
    let body: Bytes = if !early && takes_a_file(&m, &p) {
        let Ok(_turn) = BIG_AT_ONCE.acquire().await else { return error(503, "server busy") };
        match read_body(body, &headers, MAX_BYTES, BIG_WAIT).await {
            Ok(b) => b,
            Err(resp) => return resp,
        }
    } else {
        match read_body(body, &headers, SMALL_BYTES, SMALL_WAIT).await {
            Ok(b) => b,
            Err(resp) => return resp,
        }
    };
    if let Some(resp) = accounts(&app, &m, &p, &q, &headers, &body, ask).await {
        return resp;
    }
    if p.first().map(String::as_str) == Some("storage") && p.len() == 1 && (m == "GET" || m == "POST") {
        let me = current_user(&app, &headers);
        return storage(&app, &m, me, &body, &q, ask).await;
    }
    if !signed_in(&app, &headers) {
        return error(401, "please sign in");
    }
    let me = current_user(&app, &headers);
    if p.first().map(String::as_str) == Some("backup") && p.len() <= 2 {
        return backup_api(&app, &m, &p, me, &body).await;
    }
    if m == "POST" && p.len() == 3 && p[0] == "files" && p[2] == "add-sheets" {
        return add_sheets(&app, me, p[1].clone(), &q, body).await;
    }
    if m == "GET" && p.len() == 3 && p[0] == "files" && p[2] == "thumb" {
        return thumb(&app, me, p[1].clone(), &headers).await;
    }
    if m == "PUT" && p.len() == 3 && p[0] == "files" && p[2] == "thumb" {
        return put_thumb(&app, me, p[1].clone(), &q, body).await;
    }
    // StarDocs reads a document with a plain GET (spreadsheets open through their live room,
    // which notes this itself): it counts as opened, for Recent.
    if m == "GET"
        && p.len() == 2
        && p[0] == "files"
        && let Some(u) = me.clone()
    {
        let id = p[1].clone();
        blocking(&app, move |a| {
            if a.store.path_of(&id).ok().flatten().is_some_and(|path| store::is_doc(&path)) {
                let _ = spaces::note_opened(&a.store, &u, &id);
            }
        })
        .await;
    }
    let q = match gate(&app, &m, &p, q, me.clone(), &body).await {
        Ok(q) => q,
        Err(resp) => {
            // Sharing is answered inside the gate: open live rooms check access straight away.
            if m == "POST" && p.last().is_some_and(|l| l == "sharing") && resp.status().is_success() {
                app.rooms.recheck_access();
            }
            return resp;
        }
    };
    let if_match = headers.get(header::IF_MATCH).and_then(|v| v.to_str().ok()).map(str::to_string);
    // A file replaced through the API (a version restored) is read again by its live room.
    let replaced = (m == "PUT" && p.len() == 2 && p[0] == "files").then(|| p[1].clone());
    let (m2, p2) = (m.clone(), p.clone());
    let app2 = app.clone();
    let out = tokio::task::spawn_blocking(move || {
        let _g = app2.lock.lock().unwrap_or_else(|e| e.into_inner());
        app2.store.route(&m, &p, &q, if_match.as_deref(), &body)
    })
    .await;
    match out {
        Ok(mut r) => {
            if let (Some(id), Ok(Reply::Json(_, 200))) = (&replaced, &r) {
                app.rooms.replaced(id);
            }
            if let (Some(id), Ok(Reply::Json(_, 409))) = (&replaced, &r) {
                let who = me.as_ref().map_or_else(|| "a guest".to_string(), |u| u.username.clone());
                let id = id.clone();
                blocking(&app, move |a| log::note(&a.store, &format!("CONFLICT: {who} saved file {id} from an older copy (the tab wasn't live); the tab keeps their changes as a conflicted copy"))).await;
            }
            // Who changed it last (Details): a save through the API, or a new file / copy.
            if let Some(me) = &me {
                let made = match (&r, m2.as_str(), p2.as_slice()) {
                    (Ok(Reply::Json(_, 200)), "PUT", [_, id]) => Some(id.clone()),
                    (Ok(Reply::Json(v, 200 | 201)), "POST", [_] | [_, _, _]) if p2[0] == "files" => {
                        v.get("id").and_then(Value::as_str).map(str::to_string)
                    }
                    _ => None,
                };
                if let Some(id) = made {
                    let who = me.username.clone();
                    blocking(&app, move |a| spaces::note_changed(&a.store, &id, &who)).await;
                }
            }
            // Sharing changed, or something moved (ownership follows the folder): open live rooms
            // check everyone's access again now, rather than at their next 30-second check.
            if r.is_ok()
                && (m2 == "DELETE" || m2 == "POST")
                && p2.last().is_some_and(|l| m2 == "DELETE" || matches!(l.as_str(), "sharing" | "move" | "rename" | "restore"))
            {
                app.rooms.recheck_access();
            }
            if app.auth
                && let Some(me) = me
            {
                r = blocking(&app, move |app| after_files(app, &me, &m2, &p2, r)).await.unwrap_or(Err(Bad::new(500, "server error")));
            }
            reply(r)
        }
        Err(_) => error(500, "server error"),
    }
}

/// Every route, behind the checks in guard.rs (cross-site requests, host names, how many at
/// once, safety headers). Bodies are read and limited by handle() itself.
fn router(app: Shared) -> Router {
    Router::new()
        .route("/api/live", axum::routing::get(live::ws))
        .fallback(handle)
        .with_state(app)
        .layer(axum::extract::DefaultBodyLimit::disable())
        .layer(axum::middleware::from_fn(guard::middleware))
}

async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}

/// `starsheets-server healthcheck`: exits 0 when the server on PORT answers /healthz
/// (for Docker's HEALTHCHECK; the image has no curl or wget).
fn healthcheck() -> ! {
    use std::io::{Read, Write};
    let port = env("PORT", "8080");
    let ok = std::net::TcpStream::connect_timeout(&format!("127.0.0.1:{port}").parse().expect("port"), Duration::from_secs(4))
        .and_then(|mut s| {
            s.set_read_timeout(Some(Duration::from_secs(4)))?;
            s.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n")?;
            let mut buf = [0u8; 16];
            let n = s.read(&mut buf)?;
            Ok(buf[..n].starts_with(b"HTTP/1.1 200") || buf[..n].starts_with(b"HTTP/1.0 200"))
        })
        .unwrap_or(false);
    std::process::exit(if ok { 0 } else { 1 })
}

#[tokio::main]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        healthcheck();
    }
    let base = PathBuf::from(env("STORE", "/data"));
    let root = match chosen_root(&base) {
        Some(sub) if !sub.is_empty() => base.join(sub),
        _ => base.clone(),
    };
    let _ = std::fs::create_dir_all(&root);
    let store = Store {
        root,
        keep: env("KEEP", "6").parse().unwrap_or(6),
        spaces: !matches!(env("STARSHEETS_AUTH", "on").trim().to_lowercase().as_str(), "off" | "false" | "0" | "no"),
    };
    let port: u16 = env("PORT", "8080").parse().unwrap_or(8080);
    let auth = !matches!(env("STARSHEETS_AUTH", "on").trim().to_lowercase().as_str(), "off" | "false" | "0" | "no");
    if let Some(why) = auth::given_setup_code().and_then(|c| auth::weak_setup_code(&c)) {
        eprintln!("{why}");
        std::process::exit(1);
    }
    let site = Some(env("SITE", "")).filter(|s| !s.is_empty()).map(PathBuf::from);
    let app = Arc::new(App { store, base, lock: Mutex::new(()), location: env("LOCATION", "the server"), auth, site, rooms: live::Rooms::default() });
    if let Err(e) = std::fs::create_dir_all(app.store.meta()) {
        eprintln!("Can't use the folder {}: {e}", app.store.root.display());
        std::process::exit(1);
    }
    println!("StarSheets server {}: {} on :{port}, keeping {} versions", env!("CARGO_PKG_VERSION"), app.store.root.display(), app.store.keep);
    if !auth {
        println!("Sign-in is OFF (STARSHEETS_AUTH=off): anyone who can reach this port can read and change every file.");
    } else if !auth::any_users(&app.store) {
        if auth::given_setup_code().is_some() {
            println!("No accounts yet. Open StarSheets and create the owner account with the setup code set in STARSHEETS_PASSWORD.");
        } else {
            println!("No accounts yet. Open StarSheets and create the owner account with this setup code: {}", auth::setup_code(&app.store));
        }
    } else {
        println!("Sign-in is on ({} accounts).", auth::users(&app.store).len());
        match spaces::migrate(&app.store) {
            Ok(0) => {}
            Ok(n) => println!("Moved {n} files and folders into people's own folders (once, for accounts)."),
            Err(e) => {
                eprintln!("Couldn't move files into people's own folders: {e}");
                std::process::exit(1);
            }
        }
    }
    tokio::spawn(backup_schedule(app.clone()));
    let router = router(app);
    let listener = match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Can't listen on port {port}: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown(shutdown()).await {
        eprintln!("server stopped: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn app(auth: bool) -> Shared {
        let dir = std::env::temp_dir().join(format!("starsheets-main-{}", store::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store { root: dir.clone(), keep: 3, spaces: auth };
        std::fs::create_dir_all(store.meta()).unwrap();
        Arc::new(App { store, base: dir, lock: Mutex::new(()), location: "here".into(), auth, site: None, rooms: live::Rooms::default() })
    }

    /// A made-up address per call site, so the try limits of one test don't touch another's.
    fn address() -> String {
        let r = store::random_hex(3);
        let b = |i: usize| u8::from_str_radix(&r[i..i + 2], 16).unwrap();
        format!("10.{}.{}.{}", b(0), b(2), b(4))
    }

    struct Got {
        status: u16,
        headers: HeaderMap,
        json: Value,
        bytes: Vec<u8>,
    }

    async fn send(app: &Shared, ip: &str, method: &str, uri: &str, cookie: &str, extra: &[(&str, &str)], body: Vec<u8>) -> Got {
        let mut b = axum::http::Request::builder().method(method).uri(uri).header(header::HOST, "192.168.1.5:8080");
        if !cookie.is_empty() {
            b = b.header(header::COOKIE, cookie);
        }
        for (k, v) in extra {
            b = b.header(*k, *v);
        }
        let mut req = b.body(Body::from(body)).unwrap();
        req.extensions_mut().insert(axum::extract::ConnectInfo(SocketAddr::new(ip.parse().unwrap(), 50000)));
        let resp = router(app.clone()).oneshot(req).await.unwrap();
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        Got { status, headers, json: serde_json::from_slice(&bytes).unwrap_or(Value::Null), bytes: bytes.to_vec() }
    }

    async fn post(app: &Shared, ip: &str, uri: &str, cookie: &str, v: Value) -> Got {
        send(app, ip, "POST", uri, cookie, &[], serde_json::to_vec(&v).unwrap()).await
    }

    fn cookie_of(g: &Got) -> String {
        g.headers.get(header::SET_COOKIE).and_then(|v| v.to_str().ok()).unwrap_or("").split(';').next().unwrap_or("").to_string()
    }

    /// A server with its owner ("boss"), signed in: (app, address, owner's cookie).
    async fn owned() -> (Shared, String, String) {
        let app = app(true);
        let ip = address();
        let code = auth::setup_code(&app.store);
        let g = post(&app, &ip, "/api/setup", "", json!({"code": code, "username": "boss", "name": "Boss", "password": "bosspassword"})).await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        (app, ip, cookie_of(&g))
    }

    #[tokio::test]
    async fn setup_needs_the_code_and_storage_does_too() {
        let app = app(true);
        let ip = address();
        // Storage while setting up: no folders, nothing moves, without the code.
        std::fs::create_dir_all(app.base.join("Elsewhere")).unwrap();
        let g = send(&app, &ip, "GET", "/api/storage", "", &[], vec![]).await;
        assert_eq!((g.status, g.json["locked"].as_bool(), g.json["folders"].as_array().map(Vec::len)), (200, Some(true), Some(0)));
        let g = post(&app, &ip, "/api/storage", "", json!({"folder": "Elsewhere", "mode": "move"})).await;
        assert_eq!(g.status, 401);
        let code = auth::setup_code(&app.store);
        let g = send(&app, &ip, "GET", &format!("/api/storage?code={code}"), "", &[], vec![]).await;
        assert_eq!(g.json["folders"], json!(["Elsewhere"]), "with the code: the folders");
        // Setup.
        let g = post(&app, &ip, "/api/setup", "", json!({"code": "wrong-code-here", "username": "boss", "name": "Boss", "password": "bosspassword"}))
            .await;
        assert_eq!(g.status, 401);
        let long = "x".repeat(auth::MAX_PASSWORD + 1);
        let g = post(&app, &ip, "/api/setup", "", json!({"code": code, "username": "boss", "name": "Boss", "password": long})).await;
        assert_eq!(g.status, 400, "a password over 1 KiB");
        let g = post(&app, &ip, "/api/setup", "", json!({"code": code, "username": "boss", "name": "Boss", "password": "bosspassword"})).await;
        assert_eq!(g.status, 200);
        let set = g.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string();
        assert!(set.contains("HttpOnly") && !set.contains("Secure"), "plain HTTP on a home network keeps working: {set}");
        // Now only the owner chooses where files are kept.
        assert_eq!(send(&app, &ip, "GET", &format!("/api/storage?code={code}"), "", &[], vec![]).await.status, 403);
        let g = send(&app, &ip, "GET", "/api/storage", &cookie_of(&g), &[], vec![]).await;
        assert_eq!((g.status, g.json.get("locked")), (200, None));
    }

    #[tokio::test]
    async fn too_many_wrong_passwords() {
        let (app, _, _) = owned().await;
        for i in 0..10 {
            let g = post(&app, &address(), "/api/login", "", json!({"username": "boss", "password": format!("wrong {i}")})).await;
            assert_eq!(g.status, 401);
        }
        let g = post(&app, &address(), "/api/login", "", json!({"username": "boss", "password": "bosspassword"})).await;
        assert_eq!(g.status, 429, "the username is resting, from any address");
        // Unknown usernames answer just like wrong passwords.
        let g = post(&app, &address(), "/api/login", "", json!({"username": "nobody", "password": "bosspassword"})).await;
        assert_eq!((g.status, g.json["error"].as_str()), (401, Some("That username or password isn't right.")));
        // One address can't try endlessly either.
        let ip = address();
        let mut statuses = Vec::new();
        for i in 0..21 {
            statuses.push(post(&app, &ip, "/api/login", "", json!({"username": format!("someone{i}"), "password": "whatever1"})).await.status);
        }
        assert!(statuses.iter().take(20).all(|s| *s == 401) && statuses[20] == 429, "{statuses:?}");
    }

    #[tokio::test]
    async fn sign_up_is_limited_per_address() {
        let (app, ip, boss) = owned().await;
        assert_eq!(post(&app, &ip, "/api/signup/settings", &boss, json!({"on": true})).await.status, 200);
        let other = address();
        for i in 0..5 {
            let g = post(&app, &other, "/api/signup", "", json!({"username": format!("new{i}"), "name": "N", "password": "longenough"})).await;
            assert_eq!(g.status, 200, "{:?}", g.json);
        }
        let g = post(&app, &other, "/api/signup", "", json!({"username": "new9", "name": "N", "password": "longenough"})).await;
        assert_eq!(g.status, 429);
    }

    #[tokio::test]
    async fn nothing_is_read_before_sign_in() {
        let (app, ip, boss) = owned().await;
        let big = vec![b'x'; 2 * 1024 * 1024];
        // Signed out: refused before the body is read, whatever its size.
        assert_eq!(send(&app, &ip, "PUT", "/api/files/abc", "", &[], big.clone()).await.status, 401);
        // Small requests: at most 1 MiB.
        assert_eq!(send(&app, &ip, "POST", "/api/login", "", &[], big.clone()).await.status, 413);
        assert_eq!(send(&app, &ip, "POST", "/api/folders?name=A", &boss, &[], big.clone()).await.status, 413);
        // A file: big is fine once signed in.
        let g = send(&app, &ip, "POST", "/api/files?name=Big.csv", &boss, &[], big).await;
        assert_eq!(g.status, 201, "{:?}", g.json);
        // Downloads are attachments; every answer has the safety headers.
        let g = send(&app, &ip, "GET", &format!("/api/files/{}", g.json["id"].as_str().unwrap()), &boss, &[], vec![]).await;
        assert_eq!(g.headers.get(header::CONTENT_DISPOSITION).unwrap(), "attachment");
        assert_eq!(g.headers.get(header::X_FRAME_OPTIONS).unwrap(), "DENY");
        assert_eq!(g.headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");
        assert!(g.headers.get(header::CONTENT_SECURITY_POLICY).unwrap().to_str().unwrap().contains("frame-ancestors 'none'"));
        assert_eq!(g.headers.get(header::REFERRER_POLICY).unwrap(), "same-origin");
    }

    #[tokio::test]
    async fn other_sites_cant_send_requests() {
        let (app, ip, boss) = owned().await;
        let evil = [("origin", "https://evil.example")];
        let same = [("origin", "http://192.168.1.5:8080")];
        assert_eq!(send(&app, &ip, "POST", "/api/folders?name=A", &boss, &evil, vec![]).await.status, 403);
        assert_eq!(send(&app, &ip, "GET", "/api/live", &boss, &evil, vec![]).await.status, 403, "the live WebSocket");
        assert_eq!(send(&app, &ip, "POST", "/api/folders?name=A", &boss, &same, vec![]).await.status, 201);
        // StarDocs: a plain same-origin PUT of the file's bytes, no special headers.
        let g = send(&app, &ip, "POST", "/api/files?name=Doc.docx", &boss, &same, b"one".to_vec()).await;
        let id = g.json["id"].as_str().unwrap().to_string();
        let put = [("origin", "http://192.168.1.5:8080"), ("content-type", "application/octet-stream")];
        assert_eq!(send(&app, &ip, "PUT", &format!("/api/files/{id}"), &boss, &put, b"two".to_vec()).await.status, 200);
    }

    #[tokio::test]
    async fn removing_someone_moves_their_folder_to_the_owner() {
        let (app, ip, boss) = owned().await;
        assert_eq!(post(&app, &ip, "/api/users", &boss, json!({"username": "ann", "name": "Ann", "password": "annpassword"})).await.status, 200);
        std::fs::write(app.store.full("ann/Notes.csv"), "a,b").unwrap();
        let g = send(&app, &ip, "DELETE", "/api/users?username=ann", &boss, &[], vec![]).await;
        assert_eq!((g.status, g.json["folder"].as_str()), (200, Some("boss/Removed – ann")));
        assert!(app.store.full("boss/Removed – ann/Notes.csv").is_file());
        assert_eq!(post(&app, &ip, "/api/users", &boss, json!({"username": "ann", "name": "Ann 2", "password": "annpassword"})).await.status, 200);
        let g = post(&app, &ip, "/api/login", "", json!({"username": "ann", "password": "annpassword"})).await;
        let files = send(&app, &ip, "GET", "/api/files", &cookie_of(&g), &[], vec![]).await;
        assert_eq!(files.json["files"], json!([]), "the new ann starts empty");
    }

    /// StarDocs sends a picture of a document; the home screen gets it back until the file changes.
    #[tokio::test]
    async fn documents_picture_round_trip() {
        let app = app(false);
        std::fs::write(app.store.root.join("Letter.docx"), b"doc").unwrap();
        let id = app.store.synced_index().unwrap().keys().next().unwrap().clone();
        let full = app.store.full("Letter.docx");
        let v = app.store.version_of(&full).unwrap();
        let ip = address();
        let url = format!("/api/files/{id}/thumb");
        assert_eq!(send(&app, &ip, "GET", &url, "", &[], vec![]).await.status, 404, "no picture yet");
        let pic = tiny_skia::Pixmap::new(thumbs::WIDTH, thumbs::HEIGHT).unwrap().encode_png().unwrap();
        assert_eq!(send(&app, &ip, "PUT", &format!("{url}?version=0-0"), "", &[], pic.clone()).await.status, 409);
        let g = send(&app, &ip, "PUT", &format!("{url}?version={v}"), "", &[], pic.clone()).await;
        assert_eq!((g.status, g.json), (200, json!({"ok": true})));
        let g = send(&app, &ip, "GET", &url, "", &[], vec![]).await;
        assert_eq!((g.status, &g.bytes), (200, &pic));
        let etag = g.headers[header::ETAG].to_str().unwrap().to_string();
        assert_eq!(send(&app, &ip, "GET", &url, "", &[("if-none-match", &etag)], vec![]).await.status, 304);
        // Saved again (StarDocs hasn't sent the new picture yet): the old one isn't shown.
        std::fs::write(&full, b"a longer document").unwrap();
        assert_eq!(send(&app, &ip, "GET", &url, "", &[], vec![]).await.status, 404);
        let _ = std::fs::remove_dir_all(&app.store.root);
    }

    #[tokio::test]
    async fn a_second_app_in_a_folder_of_the_site() {
        // The Starbase image: StarSheets at the top of SITE, StarDocs in SITE/docs.
        let site = std::env::temp_dir().join(format!("starsheets-site-{}", store::random_hex(6)));
        std::fs::create_dir_all(site.join("docs")).unwrap();
        std::fs::write(site.join("index.html"), b"sheets").unwrap();
        std::fs::write(site.join("docs/index.html"), b"docs").unwrap();
        std::fs::write(site.join("docs/app-0123456789abcdef_bg.wasm"), b"\0asm").unwrap();
        std::fs::write(site.join("docs/app-0123456789abcdef_bg.wasm.gz"), b"zipped").unwrap();
        let none = HeaderMap::new();
        let body = |r: Response<Body>| async move { axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap().to_vec() };

        let r = serve_site(&site, "/docs", Some("file=1"), &none).await;
        assert_eq!(r.status(), 308);
        assert_eq!(r.headers()[header::LOCATION], "docs/?file=1");
        assert_eq!(body(serve_site(&site, "/docs/", None, &none).await).await, b"docs");
        assert_eq!(body(serve_site(&site, "/", None, &none).await).await, b"sheets");

        let mut gzip = HeaderMap::new();
        gzip.insert(header::ACCEPT_ENCODING, "gzip, br".parse().unwrap());
        let r = serve_site(&site, "/docs/app-0123456789abcdef_bg.wasm", None, &gzip).await;
        assert_eq!(r.headers()[header::CONTENT_TYPE], "application/wasm");
        assert_eq!(r.headers()[header::CONTENT_ENCODING], "gzip");
        assert!(r.headers()[header::CACHE_CONTROL].to_str().unwrap().contains("immutable"));
        assert_eq!(body(r).await, b"zipped");
        let _ = std::fs::remove_dir_all(&site);
    }
}
