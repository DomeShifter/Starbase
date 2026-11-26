//! StarSheets server: stores spreadsheets as ordinary .xlsx files in a folder tree (STORE) and,
//! with SITE set, serves the web app too. Replaces server/files.py with the same API and the
//! same files on disk (see docs/server.md), as the base for live editing.
//!
//! Settings (environment): STORE (folder, default /data), KEEP (most earlier versions kept per file, 100; store.rs thin), PORT (8080),
//! LOCATION (how people are told where files are kept), STARBASE_AUTH (off = no login),
//! STARBASE_PASSWORD, SITE (a built StarSheets site to serve), the security settings in
//! guard.rs (STARBASE_PUBLIC_URL, STARBASE_ALLOWED_HOSTS, STARBASE_TRUSTED_PROXY,
//! STARBASE_SECURE_COOKIE), single sign-on's STARBASE_OIDC_* (oidc.rs, docs/oidc.md) and the
//! photo index's STARBASE_PHOTOS_WORKERS, STARBASE_PHOTOS_RESCAN and STARBASE_PHOTOS_CACHE (photos/).

// Request handlers return a ready HTTP response as their error (axum's usual way); boxing it to
// make the error smaller would only add an allocation on the error path.
#![allow(clippy::result_large_err)]

mod activity;
mod auth;
mod backup;
mod commenting;
mod contentsearch;
mod devices;
mod docx_comments;
mod findpath;
mod guard;
mod live;
mod log;
mod oidc;
mod photos;
mod preview;
mod publicdeck;
mod publiclink;
mod sharing;
mod spaces;
mod store;
mod thumbs;
mod twostep;
mod zipdl;

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
/// The session cookie over plain HTTP (a home network), and over HTTPS: the `__Host-` prefix
/// means only this exact site can set it (not a sibling or parent domain). Over HTTPS the plain
/// name isn't read at all (guard::drop_plain_cookies), so a session from before the prefix
/// means signing in once more.
const COOKIE: &str = "starsheets_session";
const HOST_COOKIE: &str = "__Host-starsheets_session";
/// Linking an account that has no password needs a single sign-on sign-in this recent.
const LINK_FRESH_SECS: f64 = 600.0;

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
    /// Single sign-on through the owner's identity provider (STARBASE_OIDC_ISSUER set).
    oidc: Option<Arc<oidc::Oidc>>,
}

type Shared = Arc<App>;

fn env(name: &str, default: &str) -> String {
    setting(name).unwrap_or_else(|| default.to_string())
}

/// A setting from the environment. Every `STARBASE_*` setting may also be given by its
/// older name, `STARSHEETS_*`; when both are set the new name wins.
pub(crate) fn setting(name: &str) -> Option<String> {
    setting_from(name, |k| std::env::var(k).ok())
}

fn setting_from(name: &str, get: impl Fn(&str) -> Option<String>) -> Option<String> {
    match name.strip_prefix("STARBASE_").or_else(|| name.strip_prefix("STARSHEETS_")) {
        Some(rest) => get(&format!("STARBASE_{rest}")).or_else(|| get(&format!("STARSHEETS_{rest}"))),
        None => get(name),
    }
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

/// The browser session's token. A request that carries a desktop app's device token
/// (`Authorization: Bearer sbd_…`) is signed in by that alone: its cookies aren't read, so a token
/// can never be mixed with someone's browser session. Other Authorization headers (a proxy's own
/// Basic password, which browsers resend by themselves) don't change anything.
fn cookie_token(headers: &HeaderMap) -> Option<String> {
    if guard::device_token(headers).is_some() {
        return None;
    }
    named_cookie(headers, HOST_COOKIE).or_else(|| named_cookie(headers, COOKIE))
}

/// Who is asking (`None`: not signed in, or sign-in is off): a browser session, or a desktop
/// app's device token (devices.rs), with exactly the same rights.
fn current_user(app: &App, headers: &HeaderMap) -> Option<auth::User> {
    if !app.auth {
        return None;
    }
    if let Some(token) = guard::device_token(headers) {
        let username = devices::user_of(&app.store, &token)?;
        return auth::users(&app.store).into_iter().find(|u| u.username == username);
    }
    cookie_token(headers).and_then(|t| auth::session_user(&app.store, &t))
}

/// Who is asking through a browser session only (never a device token): approving a new device
/// needs the person themselves, in their browser.
fn session_user(app: &App, headers: &HeaderMap) -> Option<auth::User> {
    if guard::device_token(headers).is_some() { None } else { current_user(app, headers) }
}

fn signed_in(app: &App, headers: &HeaderMap) -> bool {
    !app.auth || current_user(app, headers).is_some()
}

/// The answer to a request that needs sign-in. `renew`: the session was made through single
/// sign-on and has ended, so the app renews it at the provider without asking (docs/oidc.md)
/// rather than showing the sign-in box.
fn please_sign_in(app: &App, headers: &HeaderMap) -> Response<Body> {
    let renew = app.oidc.is_some() && cookie_token(headers).is_some_and(|t| auth::session_renewable(&app.store, &t));
    send_json(&json!({ "error": "please sign in", "renew": renew }), 401)
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

fn session_cookie(value: &str, max_age: u64, secure: bool) -> String {
    if secure {
        format!("{HOST_COOKIE}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict; Secure")
    } else {
        format!("{COOKIE}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict")
    }
}

/// The Set-Cookie lines for a session (the first is the session itself): over HTTPS the old
/// name is cleared too; signing out (`value` empty) clears both names.
fn session_cookies(value: &str, max_age: u64, secure: bool) -> Vec<String> {
    let mut v = vec![session_cookie(value, max_age, secure)];
    if secure {
        v.push(session_cookie("", 0, false));
    } else if value.is_empty() {
        v.push(session_cookie("", 0, true));
    }
    v
}

/// A JSON answer that sets cookies.
fn json_with_cookies(body: Value, cookies: &[String]) -> Response<Body> {
    let mut b = Response::builder().status(200).header(header::CONTENT_TYPE, "application/json").header(header::CACHE_CONTROL, "no-store");
    for c in cookies {
        b = b.header(header::SET_COOKIE, c);
    }
    b.body(Body::from(serde_json::to_vec(&body).unwrap_or_default())).unwrap()
}

fn cookie_reply(value: &str, max_age: u64, body: Value, secure: bool) -> Response<Body> {
    json_with_cookies(body, &session_cookies(value, max_age, secure))
}

/// Sends the browser on to `location` (single sign-on), setting `cookies`.
fn see_other(location: &str, cookies: &[String]) -> Response<Body> {
    // Built without a fallible builder: an address a header can't hold (only visible ASCII can)
    // goes to "/" instead of panicking (safe_next already refuses such addresses).
    let mut r = Response::new(Body::empty());
    *r.status_mut() = axum::http::StatusCode::SEE_OTHER;
    let h = r.headers_mut();
    h.insert(header::LOCATION, header::HeaderValue::from_str(location).unwrap_or(header::HeaderValue::from_static("/")));
    h.insert(header::CACHE_CONTROL, header::HeaderValue::from_static("no-store"));
    for c in cookies {
        if let Ok(v) = header::HeaderValue::from_str(c) {
            h.append(header::SET_COOKIE, v);
        }
    }
    r
}

/// The cookie that ties a sign-in at the identity provider to this browser. SameSite=Lax: the
/// provider sends the browser back from its own site, and a Strict cookie wouldn't come along.
/// Over HTTPS it has the `__Host-` prefix, so a sibling subdomain can't plant its own (which
/// would sign this browser in as someone else).
fn oidc_cookie(value: &str, max_age: u64, secure: bool) -> String {
    if secure {
        format!("{}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax; Secure", oidc::HOST_COOKIE)
    } else {
        format!("{}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax", oidc::COOKIE)
    }
}

fn named_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookies = headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).collect::<Vec<_>>().join(";");
    cookies.split(';').find_map(|part| {
        let (n, value) = part.trim().split_once('=')?;
        (n == name && !value.is_empty()).then(|| value.to_string())
    })
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
            let sso = app.oidc.as_ref().filter(|_| on).map(|o| {
                let linked = me.as_ref().is_some_and(|u| auth::is_linked(&app.store, &u.username));
                let password = me.as_ref().is_some_and(|u| auth::has_password(&app.store, &u.username));
                // A session made through single sign-on that has ended: the app renews it at the
                // provider without asking.
                let renew = me.is_none() && cookie_token(headers).is_some_and(|t| auth::session_renewable(&app.store, &t));
                // Signed in through single sign-on just now, with the provider asking who it was
                // (an account with no password may then link another sign-in).
                let fresh = cookie_token(headers)
                    .and_then(|t| auth::fresh_sso_session_made(&app.store, &t))
                    .is_some_and(|made| store::now() - made < LINK_FRESH_SECS);
                // Seconds until this session (made through single sign-on) ends: the app renews it
                // at the provider shortly before, at a quiet moment, so nothing is interrupted.
                let expires_in = cookie_token(headers).and_then(|t| auth::sso_session_expires(&app.store, &t)).map(|e| (e - store::now()).max(0.0).floor());
                json!({ "label": o.cfg.label, "only": o.cfg.only, "owner_group": o.cfg.owner_group.is_some(), "linked": linked, "password": password, "renew": renew, "fresh": fresh, "expires_in": expires_in })
            });
            let only = app.oidc.as_ref().is_some_and(|o| o.cfg.only);
            let signup = on && !setup && s.on && !only;
            send_json(
                &json!({ "location": app.location, "auth": on, "signed_in": !on || me.is_some(), "setup": setup, "signup": signup, "signup_code": signup && !s.code.is_empty(), "user": me.as_ref().map(|u| u.json()), "two_step": me.as_ref().is_some_and(|u| twostep::is_on(&app.store, &u.username)), "sso": sso }),
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
            if app.oidc.as_ref().is_some_and(|o| o.cfg.only) {
                return Some(error(403, "This server signs people in through single sign-on only."));
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
            // STARBASE_OIDC_ONLY: passwords only for owners (a way in when the provider is down).
            // A right password for anyone else gets exactly the answer a wrong one gets, so the
            // sign-in box can't be used to check passwords.
            let only = app.oidc.as_ref().is_some_and(|o| o.cfg.only);
            let code = field(&json, "code");
            match user.filter(|u| right && (u.owner || !only)) {
                Some(user) => {
                    // Two-step sign-in: a right password is not enough. No code yet is not a wrong
                    // try (the sign-in box then asks for it); a wrong code counts like a wrong password.
                    let who = user.username.clone();
                    if blocking(app, move |app| twostep::is_on(&app.store, &who)).await.unwrap_or(false) {
                        if code.trim().is_empty() {
                            return Some(send_json(
                                &json!({ "error": "Enter the 6-digit code from your authenticator app (or a backup code).", "two_step": true }),
                                401,
                            ));
                        }
                        let who = user.username.clone();
                        if !blocking(app, move |app| twostep::check(&app.store, &who, &code)).await.unwrap_or(false) {
                            LIMITS.take(&user_key, guard::LOGIN_USER);
                            return Some(wrong(401, "That code isn't right.").await);
                        }
                    }
                    blocking(app, move |app| session_reply(app, &user, secure)).await.unwrap_or_else(|| error(500, "server error"))
                }
                None => {
                    LIMITS.take(&user_key, guard::LOGIN_USER);
                    let why = if only {
                        "That username or password isn't right. This server signs people in through single sign-on: use the button above."
                    } else {
                        "That username or password isn't right."
                    };
                    wrong(401, why).await
                }
            }
        }
        (["logout"], "POST") => {
            let mut answer = json!({ "ok": true });
            if let Some(t) = cookie_token(headers) {
                let id_token = blocking(app, move |app| {
                    let id_token = auth::session_id_token(&app.store, &t);
                    auth::end_session(&app.store, &t);
                    id_token
                })
                .await
                .flatten();
                // Signed in through single sign-on: sign out at the provider too, when it can.
                if let (Some(id_token), Some(o)) = (id_token, app.oidc.clone())
                    && let Ok(Some(url)) = tokio::task::spawn_blocking(move || o.logout_url(&id_token)).await
                {
                    answer["logout_url"] = json!(url);
                }
            }
            cookie_reply("", 0, answer, secure)
        }
        (["me", "2fa", what], "POST") => {
            let Some(me) = me else { return Some(please_sign_in(app, headers)) };
            let user_key = format!("login-user:{}", me.username);
            let code = field(&json, "code");
            let who = me.username.clone();
            match *what {
                "start" => match blocking(app, move |app| twostep::start(&app.store, &who, "Starbase")).await {
                    Some(Ok(s)) => send_json(&json!({ "ok": true, "secret": s.secret, "uri": s.uri, "qr": s.qr }), 200),
                    Some(Err(why)) => error(400, &why),
                    None => error(500, "server error"),
                },
                "on" | "off" => {
                    if !LIMITS.can(&user_key, guard::LOGIN_USER) {
                        return Some(too_many());
                    }
                    let on = *what == "on";
                    let r = blocking(app, move |app| {
                        if on {
                            return twostep::turn_on(&app.store, &who, &code).map(Some);
                        }
                        if !twostep::check(&app.store, &who, &code) {
                            return Err("That code isn't right.".to_string());
                        }
                        twostep::turn_off(&app.store, &who).map(|_| None)
                    })
                    .await;
                    match r {
                        Some(Ok(codes)) => send_json(&json!({ "ok": true, "backup_codes": codes }), 200),
                        Some(Err(why)) => {
                            LIMITS.take(&user_key, guard::LOGIN_USER);
                            wrong(400, &why).await
                        }
                        None => error(500, "server error"),
                    }
                }
                _ => error(404, "No such thing."),
            }
        }
        (["users", "2fa"], "DELETE") => {
            if me.as_ref().is_none_or(|u| !u.owner) {
                return Some(error(403, "Only the owner can do that."));
            }
            let who = q.get("username").cloned().unwrap_or_default();
            match blocking(app, move |app| twostep::turn_off(&app.store, &who)).await {
                Some(Ok(())) => send_json(&json!({ "ok": true }), 200),
                Some(Err(why)) => error(400, &why),
                None => error(500, "server error"),
            }
        }
        (["oidc", ..], _) => sso(app, &path, m, q, headers, &json, ask).await,
        (["devices", ..], _) => device_api(app, &path, m, q, headers, &json, ask).await,
        (["me"], "POST") => {
            let Some(me) = me else { return Some(please_sign_in(app, headers)) };
            let mut new_hash = None;
            if let Some(new_password) = opt_field(&json, "password") {
                let user_key = format!("login-user:{}", me.username);
                if !LIMITS.can(&user_key, guard::LOGIN_USER) {
                    return Some(too_many());
                }
                let (current, who, token) = (field(&json, "current_password"), me.username.clone(), cookie_token(headers).unwrap_or_default());
                let found = blocking(app, move |app| {
                    let stored = auth::login_hash(&app.store, &who).map(|(_, h)| h).filter(|h| !h.is_empty());
                    let fresh = auth::fresh_sso_session_made(&app.store, &token).is_some_and(|made| store::now() - made < LINK_FRESH_SECS);
                    (stored, fresh)
                })
                .await;
                let Some((stored, fresh)) = found else { return Some(error(500, "server error")) };
                if stored.is_none() {
                    // An account made through single sign-on has no password: setting its first
                    // one needs a fresh sign-in at the provider (as linking another sign-in does),
                    // so a stolen session can't add a password that outlives it.
                    if !fresh {
                        return Some(error(
                            403,
                            "To set a password, first confirm who you are at the sign-in service (Your name and colour › Confirm who you are), then set it within 10 minutes.",
                        ));
                    }
                } else if !slow(move || auth::verify(&current, stored.as_deref())).await.unwrap_or(false) {
                    LIMITS.take(&user_key, guard::LOGIN_USER);
                    return Some(wrong(400, "Your current password isn't right.").await);
                }
                new_hash = match slow(move || auth::hash_new(&new_password)).await {
                    Some(Ok(h)) => Some(h),
                    Some(Err(why)) => return Some(error(400, &why)),
                    None => return Some(error(500, "server error")),
                };
            }
            let token = cookie_token(headers).unwrap_or_default();
            let r = blocking(app, move |app| {
                let changed_password = new_hash.is_some();
                // A password changed in a session made through single sign-on keeps the link
                // (whoever did it can sign in at the provider); otherwise the link goes, so
                // someone who learned the old password and linked their own sign-in is cut off.
                let via_sso = auth::sso_session_made(&app.store, &token).is_some();
                auth::update_hashed(&app.store, &me.username, opt_field(&json, "name").as_deref(), opt_field(&json, "colour").as_deref(), new_hash)
                    .and_then(|u| {
                        if changed_password {
                            if !via_sso {
                                auth::drop_links(&app.store, &u.username)?;
                            }
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
                return Some(please_sign_in(app, headers));
            }
            // The owner also sees whom each account is linked to at the identity provider.
            // The provider by name (STARBASE_OIDC_LABEL, else its host) and what it calls the
            // person (email, else username, as at their last sign-in); the raw ids for a tooltip.
            let owner = me.as_ref().is_some_and(|u| u.owner);
            let (issuer, provider) = app.oidc.as_ref().map(|o| (o.cfg.issuer.clone(), o.cfg.provider.clone())).unwrap_or_default();
            let list = blocking(app, move |app| {
                auth::users(&app.store)
                    .iter()
                    .map(|u| {
                        let mut v = u.json();
                        if owner && let Some(l) = auth::link_of(&app.store, &u.username) {
                            let name = if l.iss == issuer { provider.clone() } else { oidc::host_of(&l.iss) };
                            let known = if l.email.is_empty() { l.username.clone() } else { l.email.clone() };
                            v["sso"] = json!({ "issuer": l.iss, "subject": l.sub, "provider": name, "who": known });
                        }
                        if owner {
                            v["password"] = json!(auth::has_password(&app.store, &u.username));
                            v["two_step"] = json!(twostep::is_on(&app.store, &u.username));
                        }
                        v
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            send_json(&Value::Array(list), 200)
        }
        (["users", "sso"], "DELETE") => {
            if me.as_ref().is_none_or(|u| !u.owner) {
                return Some(error(403, "Only the owner can do that."));
            }
            let who = q.get("username").cloned().unwrap_or_default();
            let only = app.oidc.as_ref().is_some_and(|o| o.cfg.only);
            let r = blocking(app, move |app| {
                let target = auth::users(&app.store).into_iter().find(|u| u.username == who);
                // With STARBASE_OIDC_ONLY only owners may sign in with a password: anyone else
                // unlinked couldn't sign in at all.
                if only && target.as_ref().is_some_and(|u| !u.owner) {
                    return Err(format!(
                        "This server signs people in through single sign-on only, so {who}'s account must stay linked: without the link they couldn't sign in at all."
                    ));
                }
                if target.is_some() && !auth::has_password(&app.store, &who) {
                    return Err(format!(
                        "{who} has no password, so they sign in only through single sign-on: removing the link would lock them out. They can set a password themselves first (Your name and colour › Set a password), then the link can go."
                    ));
                }
                auth::unlink(&app.store, &who)
            })
            .await;
            match r {
                Some(Ok(())) => send_json(&json!({ "ok": true }), 200),
                Some(Err(why)) => error(400, &why),
                None => error(500, "server error"),
            }
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
                    // A password set by the owner removes the person's single sign-on link: they
                    // link again themselves if they want it.
                    let u = auth::update_hashed(&app.store, &field(&json, "username"), None, None, hash)?;
                    auth::drop_links(&app.store, &u.username)?;
                    Ok(u.json())
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

// ---------------------------------------------------------------- single sign-on

/// /api/oidc/login, /api/oidc/link, /api/oidc/callback and /api/oidc/unlink (oidc.rs). The
/// browser comes to login and callback itself (links and redirects), so problems are shown by
/// sending it back to StarSheets with `?sso=<what happened>`, which the sign-in box explains.
async fn sso(app: &Shared, path: &[&str], m: &str, q: &HashMap<String, String>, headers: &HeaderMap, json: &Value, ask: Ask) -> Response<Body> {
    use guard::{LIMITS, ip_key};
    let me = current_user(app, headers);
    let Some(o) = app.oidc.clone().filter(|_| app.auth) else { return error(404, "Single sign-on isn't set up on this server.") };
    let secure = ask.secure || o.cfg.https();
    let who = |next: String| oidc::Start {
        next,
        from: ask.ip.map_or_else(|| "?".to_string(), guard::ip_prefix),
        net: ask.ip.map_or_else(|| "?".to_string(), guard::ip_network),
        ..Default::default()
    };
    let back = |next: &str, what: &str| see_other(&oidc::with_param(next, "sso", what), &[oidc_cookie("", 0, secure)]);
    match (path, m) {
        // ?silent=1: renewing an ended session without asking; ?fresh=1: the provider asks who
        // it is again (oidc.rs, Start).
        (["oidc", "login"], "GET") => {
            let next = q.get("next").and_then(|n| oidc::safe_next(n)).unwrap_or_else(|| o.cfg.base_path());
            if !LIMITS.take(&ip_key("oidc", ask.ip), guard::OIDC_IP) {
                return back(&next, "busy");
            }
            let mut start = who(next.clone());
            start.silent = q.get("silent").is_some_and(|v| v == "1");
            start.fresh = !start.silent && q.get("fresh").is_some_and(|v| v == "1");
            // The session being renewed: this request is a same-site navigation, so its cookie
            // comes along (the provider's redirect back won't bring it).
            start.renew = cookie_token(headers).filter(|_| start.silent);
            let o2 = o.clone();
            match tokio::task::spawn_blocking(move || o2.start(start)).await {
                Ok(Ok((url, binding))) => see_other(&url, &[oidc_cookie(&binding, oidc::PENDING_FOR.as_secs(), secure)]),
                Ok(Err(why)) => {
                    eprintln!("Single sign-on: {why}");
                    back(&next, "unreachable")
                }
                Err(_) => back(&next, "failed"),
            }
        }
        // Linking the signed-in account: a POST (so guard.rs's cross-site checks apply) with the
        // account's current password, or, for an account with no password, a fresh sign-in
        // (?fresh=1) through single sign-on in the last LINK_FRESH_SECS. Answers where to send
        // the browser.
        (["oidc", "link"], "POST") => {
            let Some(me) = me else { return please_sign_in(app, headers) };
            let next = json.get("next").and_then(Value::as_str).and_then(oidc::safe_next).unwrap_or_else(|| o.cfg.base_path());
            let user_key = format!("login-user:{}", me.username);
            if !LIMITS.can(&user_key, guard::LOGIN_USER) || !LIMITS.take(&ip_key("oidc", ask.ip), guard::OIDC_IP) {
                return too_many();
            }
            let (name, token) = (me.username.clone(), cookie_token(headers).unwrap_or_default());
            let found = blocking(app, move |app| {
                let hash = auth::login_hash(&app.store, &name).map(|(_, h)| h).filter(|h| !h.is_empty());
                let fresh = auth::fresh_sso_session_made(&app.store, &token).is_some_and(|made| store::now() - made < LINK_FRESH_SECS);
                (hash, fresh)
            })
            .await;
            let Some((hash, fresh)) = found else { return error(500, "server error") };
            match hash {
                Some(h) => {
                    let current = field(json, "current_password");
                    if !slow(move || auth::verify(&current, Some(&h))).await.unwrap_or(false) {
                        LIMITS.take(&user_key, guard::LOGIN_USER);
                        return wrong(400, "Your current password isn't right.").await;
                    }
                }
                None if !fresh => {
                    return error(403, "To link a different sign-in, first confirm who you are at the sign-in service, then link within 10 minutes.");
                }
                None => {}
            }
            let mut start = who(next);
            start.link = Some(me.username);
            let o2 = o.clone();
            match tokio::task::spawn_blocking(move || o2.start(start)).await {
                Ok(Ok((url, binding))) => json_with_cookies(json!({ "url": url }), &[oidc_cookie(&binding, oidc::PENDING_FOR.as_secs(), secure)]),
                Ok(Err(why)) => {
                    eprintln!("Single sign-on: {why}");
                    error(502, "The sign-in service couldn't be reached. Try again in a moment.")
                }
                Err(_) => error(500, "server error"),
            }
        }
        (["oidc", "callback"], "GET") => {
            let home = o.cfg.base_path();
            if !LIMITS.take(&ip_key("oidc-back", ask.ip), guard::LOGIN_IP) {
                return back(&home, "busy");
            }
            let state = q.get("state").cloned().unwrap_or_default();
            // Over HTTPS only the __Host- cookie: a sibling site can set the plain one.
            let binding = named_cookie(headers, if secure { oidc::HOST_COOKIE } else { oidc::COOKIE }).unwrap_or_default();
            let Some(pending) = o.take(&state, &binding) else { return back(&home, "expired") };
            // Renewing: the ended session goes, whatever the provider answers (a new one is
            // made if it says yes).
            if let Some(t) = pending.start.renew.clone() {
                blocking(app, move |app| {
                    if auth::session_user(&app.store, &t).is_none() {
                        auth::end_session(&app.store, &t);
                    }
                })
                .await;
            }
            if let Some(e) = q.get("error") {
                // Renewing and the provider needs to ask something (signed out there, say): the
                // sign-in box is shown.
                let what = if pending.start.silent {
                    "renew"
                } else if e == "access_denied" {
                    "denied"
                } else {
                    "failed"
                };
                return back(&pending.start.next, what);
            }
            let code = q.get("code").cloned().unwrap_or_default();
            let o2 = o.clone();
            let found = tokio::task::spawn_blocking(move || {
                let r = o2.finish(&pending, &code);
                (r, pending)
            })
            .await;
            let (who, id_token, pending) = match found {
                Ok((Ok((who, id_token)), pending)) => (who, id_token, pending),
                Ok((Err(why), pending)) => {
                    eprintln!("Single sign-on failed: {why}");
                    return back(&pending.start.next, "failed");
                }
                Err(_) => return back(&home, "failed"),
            };
            let (next, link, fresh, renewed) =
                (pending.start.next.clone(), pending.start.link.clone(), pending.start.fresh, pending.start.renew.clone());
            let cfg = o.cfg.clone();
            let r = blocking(app, move |app| {
                let user = oidc::account(&app.store, &cfg, &who, link.as_deref())?;
                let sso = auth::SsoSession { id_token: &id_token, secs: cfg.session_hours * 3600, fresh };
                let token = auth::new_session_with(&app.store, &user.username, Some(sso)).map_err(|_| "failed")?;
                // Renewed before it ended (the app does that at a quiet moment): the new session
                // replaces it.
                if let Some(t) = renewed {
                    auth::end_session(&app.store, &t);
                }
                Ok(token)
            })
            .await;
            match r {
                Some(Ok(token)) => {
                    let mut cookies = vec![oidc_cookie("", 0, secure)];
                    cookies.extend(session_cookies(&token, auth::SESSION_DAYS * 86400, secure));
                    see_other(&next, &cookies)
                }
                Some(Err(what)) => back(&next, what),
                None => back(&next, "failed"),
            }
        }
        (["oidc", "unlink"], "POST") => {
            let Some(me) = me else { return please_sign_in(app, headers) };
            // With STARBASE_OIDC_ONLY only owners may use passwords: anyone else unlinked
            // couldn't sign in at all.
            if o.cfg.only && !me.owner {
                return error(400, "This server signs people in through single sign-on only, so your account must stay linked.");
            }
            match blocking(app, move |app| auth::unlink(&app.store, &me.username)).await {
                Some(Ok(())) => send_json(&json!({ "ok": true }), 200),
                Some(Err(why)) => error(400, &why),
                None => error(500, "server error"),
            }
        }
        _ => error(404, "not found"),
    }
}

// ---------------------------------------------------------------- desktop apps

/// The page a person opens to let a desktop app in (devices.rs).
const CONNECT_PAGE: &str = include_str!("connect.html");

/// The connect page's address as the person's browser reaches this server: STARBASE_PUBLIC_URL
/// when set, else the address the app used. Never with the code in it: the person types the code
/// the app shows, so a link someone else sends can't take them straight to "Allow".
fn connect_url(headers: &HeaderMap, ask: Ask) -> String {
    let base = setting("STARBASE_PUBLIC_URL").map(|u| u.trim().trim_end_matches('/').to_string()).filter(|u| !u.is_empty()).unwrap_or_else(|| {
        let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("localhost");
        format!("{}://{host}", if ask.secure { "https" } else { "http" })
    });
    format!("{base}/api/connect")
}

/// /api/devices…: desktop sign-in (start, poll; the connect page's code and approve) and the
/// list of connected desktop apps (GET, DELETE).
async fn device_api(
    app: &Shared,
    path: &[&str],
    m: &str,
    q: &HashMap<String, String>,
    headers: &HeaderMap,
    json: &Value,
    ask: Ask,
) -> Response<Body> {
    use guard::{LIMITS, ip_key};
    if !app.auth {
        return error(400, "Sign-in is off on this server, so there is nothing to connect to: open files without signing in.");
    }
    match (path, m) {
        (["devices", "start"], "POST") => {
            if !LIMITS.take(&ip_key("device", ask.ip), guard::DEVICE_IP) {
                return too_many();
            }
            let from = ask.ip.map_or_else(|| "?".to_string(), guard::ip_prefix);
            match devices::WAITING.start(&field(json, "name"), &from, store::now()) {
                Ok((device_code, user_code)) => send_json(
                    &json!({
                        "device_code": device_code,
                        "user_code": devices::show_code(&user_code),
                        "verify_url": connect_url(headers, ask),
                        "expires_in": devices::CODE_SECS,
                        "interval": devices::INTERVAL,
                    }),
                    200,
                ),
                Err(why) => error(429, why),
            }
        }
        (["devices", "poll"], "POST") => {
            if !LIMITS.take(&ip_key("device-poll", ask.ip), guard::DEVICE_POLL_IP) {
                return send_json(&json!({ "error": "slow down", "status": "slow_down" }), 429);
            }
            match devices::WAITING.poll(&field(json, "device_code"), store::now()) {
                devices::Poll::Waiting => send_json(&json!({ "status": "pending", "interval": devices::INTERVAL }), 200),
                devices::Poll::Allowed(token) => {
                    // Made when it was allowed: if the account's devices were removed since (a new
                    // password, the account removed), it no longer works and isn't handed out.
                    let user = blocking(app, move |app| {
                        let username = devices::user_of(&app.store, &token)?;
                        auth::users(&app.store).into_iter().find(|u| u.username == username).map(|u| (u, token))
                    })
                    .await
                    .flatten();
                    match user {
                        Some((user, token)) => send_json(&json!({ "status": "ok", "token": token, "user": user.json() }), 200),
                        None => send_json(&json!({ "status": "denied", "error": "The sign-in was cancelled on the server. Start again." }), 403),
                    }
                }
                devices::Poll::Denied => send_json(&json!({ "status": "denied", "error": "The sign-in wasn't allowed." }), 403),
                devices::Poll::Expired => send_json(&json!({ "status": "expired", "error": "The code ran out. Start again." }), 410),
                devices::Poll::Unknown => send_json(&json!({ "status": "unknown", "error": "No such sign-in. Start again." }), 404),
            }
        }
        // The connect page: what is asking (a signed-in browser session only).
        (["devices", "code"], "GET") => {
            let Some(me) = session_user(app, headers) else { return please_sign_in(app, headers) };
            let key = format!("device-code:{}", me.username);
            if !LIMITS.can(&key, guard::DEVICE_CODE_USER) {
                return too_many();
            }
            let code = q.get("code").cloned().unwrap_or_default();
            match devices::WAITING.about(&code, store::now()) {
                // Where the request came from and when, so a request someone else started (and
                // sent the code for) stands out: another address than this browser's.
                Some((name, from, made)) => {
                    let here = ask.ip.map_or_else(|| "?".to_string(), guard::ip_prefix);
                    send_json(
                        &json!({
                            "name": name,
                            "code": devices::show_code(&devices::clean_code(&code)),
                            "user": me.json(),
                            "from": from,
                            "same_place": from == here && from != "?",
                            "secs_ago": (store::now() - made).max(0.0).floor(),
                        }),
                        200,
                    )
                }
                None => {
                    LIMITS.take(&key, guard::DEVICE_CODE_USER);
                    wrong(404, "That code isn't right, or it has run out. Start again in the app.").await
                }
            }
        }
        // Allowing (or not) a desktop app: a browser session, from this server's own page (the
        // Origin header must be there; guard.rs checks it is this site's).
        (["devices", "approve"], "POST") => {
            let Some(me) = session_user(app, headers) else { return please_sign_in(app, headers) };
            if !headers.contains_key(header::ORIGIN) {
                return error(403, "Allow the app from the connect page in your browser.");
            }
            let key = format!("device-code:{}", me.username);
            if !LIMITS.can(&key, guard::DEVICE_CODE_USER) {
                return too_many();
            }
            let allow = json.get("allow").and_then(Value::as_bool).unwrap_or(false);
            // Allowed from a single sign-on session: the device goes when those do.
            let via_sso = cookie_token(headers).is_some_and(|t| auth::sso_session_made(&app.store, &t).is_some());
            let (app2, who) = (app.clone(), me.username.clone());
            let mint = move |name: &str| devices::new_token(&app2.store, &who, name, via_sso).ok();
            match devices::WAITING.answer(&field(json, "code"), store::now(), allow.then_some(&mint as devices::Mint)) {
                Ok(name) => {
                    // Never the token itself.
                    let (who, what) = (me.username.clone(), if allow { "allowed" } else { "refused" });
                    blocking(app, move |a| log::note(&a.store, &format!("{who} {what} a desktop app ({name:?})"))).await;
                    send_json(&json!({ "ok": true }), 200)
                }
                Err(why) => {
                    LIMITS.take(&key, guard::DEVICE_CODE_USER);
                    wrong(404, why).await
                }
            }
        }
        // Connected desktop apps: your own, or (the owner) someone else's (?username=).
        (["devices"], "GET" | "DELETE") => {
            let Some(me) = current_user(app, headers) else { return please_sign_in(app, headers) };
            let who = q.get("username").cloned().filter(|u| !u.is_empty()).unwrap_or_else(|| me.username.clone());
            if who != me.username && !me.owner {
                return error(403, "Only the owner can do that.");
            }
            let current = guard::device_token(headers).map(|t| devices::hash(&t));
            let token = guard::device_token(headers);
            let (id, deleting) = (q.get("id").cloned().unwrap_or_default(), m == "DELETE");
            let r = blocking(app, move |app| {
                if !deleting {
                    return Ok(Value::Array(devices::list(&app.store, &who, current.as_deref())));
                }
                // "this": the app disconnecting itself.
                let gone = if id == "this" {
                    token.is_some_and(|t| devices::remove_token(&app.store, &t).unwrap_or(false))
                } else {
                    devices::remove(&app.store, &who, &id).unwrap_or(false)
                };
                if !gone {
                    return Err("There's no such device.");
                }
                log::note(&app.store, &format!("{} removed a desktop app of {who}", me.username));
                Ok(json!({ "ok": true }))
            })
            .await;
            match r {
                Some(Ok(v)) => send_json(&v, 200),
                Some(Err(why)) => error(404, why),
                None => error(500, "server error"),
            }
        }
        _ => error(404, "not found"),
    }
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
            } else if have == Role::Comment {
                Err(error(403, "You can only comment on that."))
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
                    // Version history is for editors, as in Google Docs: viewers see the file only.
                    (["versions", ..] | ["history"], "GET") | (["versions", _, "name" | "restore"], "POST") => need(role, Role::Edit)?,
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
                    // Activity: anyone who can see the file may read what happened to it.
                    (["activity"], "GET") => {
                        need(role, Role::View)?;
                        return Err(send_json(&activity::list(store, id), 200));
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
                    // External content (IMAGE pictures from the web): blocked in files that
                    // aren't the person's own until they allow it (docs/privacy.md).
                    (["external"], "GET") => {
                        need(role, Role::View)?;
                        let own = store::space_of(&path) == home && spaces::last_change(store, id).is_none_or(|(by, _)| by == home);
                        return Err(send_json(&json!({ "allowed": spaces::external_allowed(store, &me, id, own) }), 200));
                    }
                    (["external"], "POST") => {
                        need(role, Role::View)?;
                        let on = json.get("allow").and_then(Value::as_bool).unwrap_or(false);
                        return Err(match spaces::set_external(store, &me, id, on) {
                            Ok(()) => send_json(&json!({ "allowed": on }), 200),
                            Err(e) => error(500, &e.to_string()),
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
            (["bin" | "usage", ..], _) => {
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
                    return Err("There are no Starbase files in that folder to switch to. Choose \"Move my files there\" instead.".into());
                }
            }
            _ => {
                // A folder inside the one in use is fine unless it is StarSheets' own (someone's
                // folder, the bin, the hidden data).
                if let Ok(rest) = new.strip_prefix(&old) {
                    let first = rest.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
                    if first.is_empty() || first.starts_with('.') || auth::users(&app2.store).iter().any(|u| u.username == first) {
                        return Err("That folder belongs to Starbase itself. Pick another one.".into());
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
    /// Draws one picture (run on a blocking thread).
    type DrawJob = Box<dyn FnOnce(&App) -> Result<Vec<u8>, String> + Send>;
    enum Got {
        Have(Vec<u8>, String),
        Draw(String, Vec<u8>, String),
        Photo(String, String),
        Pdf(Vec<u8>, String),
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
        // Photos: their Photos thumbnail as a tile; PDFs: the top of page 1. Other files (videos:
        // no ffmpeg in the server) have no picture here: the Drive shows their type's icon.
        if photos::media_of(&path).is_some_and(|(_, thumbs)| thumbs) {
            return Ok(Got::Photo(path, version));
        }
        if path.to_ascii_lowercase().ends_with(".pdf") {
            if std::fs::metadata(&full).map(|m| m.len()).unwrap_or(u64::MAX) > thumbs::MAX_PDF {
                return Err(error(404, "too big for a picture"));
            }
            let bytes = std::fs::read(&full).map_err(|_| error(404, "no such file"))?;
            return Ok(Got::Pdf(bytes, version));
        }
        if !store::is_editable(&path) {
            return Err(error(404, "no picture for this kind of file"));
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
        Ok(got) => {
            let (store_root, f3) = (app.clone(), file.clone());
            let (job, v): (DrawJob, String) = match got {
                Got::Draw(name, bytes, v) => (Box::new(move |_| thumbs::from_bytes(&name, &bytes)), v),
                Got::Photo(path, v) => (Box::new(move |a| thumbs::tile(&photos::drive_thumb(&a.store, &path)?)), v),
                Got::Pdf(bytes, v) => (Box::new(move |_| thumbs::from_pdf(bytes)), v),
                Got::Have(..) => unreachable!(),
            };
            let v2 = v.clone();
            let drawn = tokio::task::spawn_blocking(move || {
                let png = job(&store_root)?;
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

/// Paths answered before (or without) sign-in: accounts (single sign-on too) and, while
/// setting up, storage.
/// The request after "New from a template": `POST api/templates/<key>?folder=…` becomes
/// `POST api/files?name=<name> (copy).<ext>&folder=…` with the template's file as the body.
type Upload = (Vec<String>, HashMap<String, String>, Bytes);

/// See [`Upload`]; any other request passes through unchanged.
fn template_upload(m: &str, p: Vec<String>, mut q: HashMap<String, String>, body: Bytes) -> Result<Upload, Response<Body>> {
    if p.first().map(String::as_str) != Some("templates") {
        return Ok((p, q, body));
    }
    let t = match (m, p.as_slice()) {
        ("POST", [_, key]) => starbase_templates::find(key).ok_or_else(|| error(404, "no such template"))?,
        _ => return Err(error(404, "not found")),
    };
    let bytes = starbase_templates::bytes(t.key).ok_or_else(|| error(404, "no such template"))?;
    q.insert("name".into(), starbase_templates::upload_name(t));
    q.remove("as_is");
    Ok((vec!["files".into()], q, Bytes::from_static(bytes)))
}

fn before_sign_in(p: &[String]) -> bool {
    matches!(p.first().map(String::as_str), Some("info" | "setup" | "signup" | "login" | "logout" | "me" | "users" | "storage" | "oidc" | "devices"))
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

/// Every request but the live WebSocket.
async fn handle(State(app): State<Shared>, req: Request) -> Response<Body> {
    let (parts, body) = req.into_parts();
    let (method, uri, headers) = (parts.method, parts.uri, parts.headers);
    let peer = guard::peer(&parts.extensions);
    let ask = Ask { ip: guard::client_ip(peer, &headers, guard::config()), secure: guard::secure(peer, &headers, guard::config()) };
    let path = uri.path().to_string();
    if path == "/healthz" {
        return Response::builder().header(header::CONTENT_TYPE, "text/plain").body(Body::from("ok")).unwrap();
    }
    // The page where a desktop app is allowed in (devices.rs), with or without the web app.
    if path == "/api/connect" && (method == Method::GET || method == Method::HEAD) {
        return Response::builder()
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::from(CONNECT_PAGE))
            .unwrap();
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
    // A public link (publiclink.rs): no account needed.
    if p.first().map(String::as_str) == Some("public") && p.len() >= 2 {
        let token = p[1].clone();
        let reply = if method == Method::POST && p.len() == 2 {
            // The password page's form: a small body, no account.
            let b = match read_body(body, &headers, 4096, SMALL_WAIT).await {
                Ok(b) => b,
                Err(resp) => return resp,
            };
            let pw = form_urlencoded::parse(&b).find(|(k, _)| k == "pw").map(|(_, v)| v.into_owned()).unwrap_or_default();
            blocking(&app, move |a| publiclink::unlock(&a.store, &token, &pw)).await
        } else if method == Method::GET || method == Method::HEAD {
            let what = p.get(2..).map(|r| r.join("/")).unwrap_or_default();
            let cookies = headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).collect::<Vec<_>>().join(";");
            blocking(&app, move |a| publiclink::serve(&a.store, &token, &what, &publiclink::grant_in(&cookies, &token))).await
        } else {
            return error(405, "method not allowed");
        };
        return match reply {
            Some(r) => {
                let csp = if r.script {
                    "sandbox allow-scripts allow-same-origin; default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; script-src 'self'"
                } else if r.form {
                    "sandbox allow-forms allow-same-origin; default-src 'none'; style-src 'unsafe-inline'; form-action 'self'"
                } else {
                    "sandbox allow-same-origin; default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; frame-src 'self'"
                };
                let mut b = Response::builder()
                    .status(r.code)
                    .header(header::CONTENT_TYPE, r.content_type)
                    .header(header::CACHE_CONTROL, "no-store")
                    .header("x-content-type-options", "nosniff")
                    .header("content-security-policy", csp);
                if let Some(d) = r.disposition {
                    b = b.header(header::CONTENT_DISPOSITION, d);
                }
                if let Some(c) = r.set_cookie {
                    b = b.header(header::SET_COOKIE, c);
                }
                if let Some(l) = r.location {
                    b = b.header(header::LOCATION, l);
                }
                b.body(Body::from(r.body)).unwrap()
            }
            None => error(500, "server error"),
        };
    }
    let m = method.as_str().to_string();
    // Nothing is read (a body can be big) before we know who is asking: only the account and
    // setup requests come before sign-in, with small bodies.
    let early = before_sign_in(&p);
    if !early && !signed_in(&app, &headers) {
        return please_sign_in(&app, &headers);
    }
    let body: Bytes = if !early && takes_a_file(&m, &p) {
        let Ok(_turn) = BIG_AT_ONCE.acquire().await else { return error(503, "server busy") };
        match read_body(body, &headers, MAX_BYTES, BIG_WAIT).await {
            Ok(b) => b,
            Err(resp) => return resp,
        }
    } else {
        // Picture edits and exports: their own, smaller limit, before anything is buffered.
        let photos = p.len() == 3 && p[0] == "photos" && (p[1] == "edits" || p[1] == "export");
        let limit = if photos { photos::edits::MAX_BODY as u64 } else { SMALL_BYTES };
        match read_body(body, &headers, limit, SMALL_WAIT).await {
            Ok(b) => b,
            Err(resp) => return resp,
        }
    };
    // New from a template: made exactly like an upload of the template's file, so the folder
    // checks, naming ("CV (copy).docx", then "(2)") and reply are the Drive's own.
    let (p, q, body) = match template_upload(&m, p, q, body) {
        Ok(x) => x,
        Err(resp) => return resp,
    };
    if let Some(resp) = accounts(&app, &m, &p, &q, &headers, &body, ask).await {
        return resp;
    }
    if p.first().map(String::as_str) == Some("storage") && p.len() == 1 && (m == "GET" || m == "POST") {
        let me = current_user(&app, &headers);
        return storage(&app, &m, me, &body, &q, ask).await;
    }
    if !signed_in(&app, &headers) {
        return please_sign_in(&app, &headers);
    }
    let me = current_user(&app, &headers);
    if let Some(resp) = contentsearch::route(&app, &m, &p, &q, me.clone()).await {
        return resp;
    }
    if let Some(resp) = findpath::route(&app, &m, &p, &q, me.clone()).await {
        return resp;
    }
    // Photos: folder listings with EXIF dates, thumbnails, edits, the index's status (photos/).
    if let Some(resp) = photos::routes::route(&app, &m, &p, &q, me.clone(), &headers, &body).await {
        return resp;
    }
    if p.first().map(String::as_str) == Some("backup") && p.len() <= 2 {
        return backup_api(&app, &m, &p, me, &body).await;
    }
    if m == "POST" && p.len() == 3 && p[0] == "files" && p[2] == "add-sheets" {
        return add_sheets(&app, me, p[1].clone(), &q, body).await;
    }
    if m == "GET" && p.len() == 3 && p[0] == "files" && p[2] == "thumb" {
        return thumb(&app, me, p[1].clone(), &headers).await;
    }
    if m == "PUT"
        && p.len() == 2
        && p[0] == "files"
        && let Some(resp) = commenting::put(
            &app,
            me.clone(),
            p[1].clone(),
            headers.get(header::IF_MATCH).and_then(|v| v.to_str().ok()).map(str::to_string),
            headers.get("x-comment-ids").and_then(|v| v.to_str().ok()).map(str::to_string),
            body.clone(),
        )
        .await
    {
        if resp.status().is_success()
            && let Some(me) = &me
        {
            let (who, id) = (me.username.clone(), p[1].clone());
            blocking(&app, move |a| activity::note(&a.store, &id, &who, "commented", "")).await;
        }
        return resp;
    }
    if m == "PUT" && p.len() == 3 && p[0] == "files" && p[2] == "thumb" {
        return put_thumb(&app, me, p[1].clone(), &q, body).await;
    }
    // Files that aren't spreadsheets or documents: previews, and downloads streamed from disk
    // (preview.rs says how they are kept safe).
    if m == "GET" && p.len() == 3 && p[0] == "files" && p[2] == "preview" {
        return preview::preview(&app, me, p[1].clone(), &headers).await;
    }
    if m == "GET" && p.len() == 1 && p[0] == "zip" {
        return zipdl::zip(&app, me, &q).await;
    }
    if m == "GET" && p.len() == 3 && p[0] == "files" && p[2] == "raw" {
        return preview::raw(&app, me, p[1].clone(), &headers).await;
    }
    if m == "GET" && p.len() == 2 && p[0] == "files" && preview::is_other(&app, p[1].clone()).await {
        return preview::download(&app, me, p[1].clone(), &headers).await;
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
                if let (Some(me), true) = (&me, p.len() == 3 && p[0] == "files") {
                    let (who, id) = (me.username.clone(), p[1].clone());
                    let body: Value = serde_json::from_slice(&body).unwrap_or_default();
                    blocking(&app, move |a| {
                        for (kind, detail) in activity::from_sharing(&body) {
                            activity::note(&a.store, &id, &who, kind, &detail);
                        }
                    })
                    .await;
                }
            }
            return resp;
        }
    };
    let if_match = headers.get(header::IF_MATCH).and_then(|v| v.to_str().ok()).map(str::to_string);
    // A file replaced through the API (a version restored) is read again by its live room.
    let restored = m == "POST" && p.len() == 5 && p[0] == "files" && p[2] == "versions" && p[4] == "restore";
    let replaced = ((m == "PUT" && p.len() == 2 && p[0] == "files") || restored).then(|| p[1].clone());
    // A file uploaded from this computer, or a copy, came from outside: its external content
    // stays blocked for the person who brought it in until they allow it.
    let from_outside = m == "POST"
        && ((p.len() == 1 && p[0] == "files" && (q.get("as_is").is_some_and(|v| v == "1") || q.get("imported").is_some_and(|v| v == "1")))
            || (p.len() == 3 && p[0] == "files" && p[2] == "copy"));
    let (m2, p2) = (m.clone(), p.clone());
    let act_q = q.clone();
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
                    (Ok(Reply::Json(_, 200)), "POST", [_, id, _, _, _]) if restored => Some(id.clone()),
                    (Ok(Reply::Json(v, 200 | 201)), "POST", [_] | [_, _, _]) if p2[0] == "files" => {
                        v.get("id").and_then(Value::as_str).map(str::to_string)
                    }
                    _ => None,
                };
                // Activity (Details): what was done, by whom.
                if let (Ok(Reply::Json(v, 200 | 201)), true) = (&r, p2.first().is_some_and(|f| f == "files")) {
                    let ev: Option<(String, &str, String)> = match (m2.as_str(), p2.as_slice()) {
                        ("PUT", [_, id]) => Some((id.clone(), "edited", String::new())),
                        ("POST", [_]) => v.get("id").and_then(Value::as_str).map(|i| (i.to_string(), "created", String::new())),
                        ("POST", [_, _, c]) if c == "copy" => {
                            v.get("id").and_then(Value::as_str).map(|i| (i.to_string(), "created", "a copy".to_string()))
                        }
                        ("POST", [_, id, c]) if c == "rename" => Some((id.clone(), "renamed", act_q.get("name").cloned().unwrap_or_default())),
                        ("POST", [_, id, c]) if c == "move" => Some((id.clone(), "moved", act_q.get("folder").cloned().unwrap_or_default())),
                        ("POST", [_, id, _, v, c]) if c == "restore" => Some((id.clone(), "restored", v.clone())),
                        ("DELETE", [_, id]) => Some((id.clone(), "trashed", String::new())),
                        _ => None,
                    };
                    if let Some((fid, kind, detail)) = ev {
                        let who = me.username.clone();
                        blocking(&app, move |a| activity::note(&a.store, &fid, &who, kind, &detail)).await;
                    }
                }
                if let Some(id) = made {
                    let who = me.username.clone();
                    if from_outside {
                        let (u, id) = (me.clone(), id.clone());
                        blocking(&app, move |a| spaces::set_external(&a.store, &u, &id, false)).await;
                    }
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

fn main() {
    // A child that reads one picture for the photo index (photos/decode.rs), before anything
    // else starts: no async runtime, no threads, its own limits.
    if std::env::args().nth(1).as_deref() == Some("photo-worker") {
        photos::decode::child_main();
    }
    serve();
}

#[tokio::main]
async fn serve() {
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
        keep: env("KEEP", "100").parse().unwrap_or(100),
        spaces: !matches!(env("STARBASE_AUTH", "on").trim().to_lowercase().as_str(), "off" | "false" | "0" | "no"),
    };
    let port: u16 = env("PORT", "8080").parse().unwrap_or(8080);
    let auth = !matches!(env("STARBASE_AUTH", "on").trim().to_lowercase().as_str(), "off" | "false" | "0" | "no");
    if let Some(why) = auth::given_setup_code().and_then(|c| auth::weak_setup_code(&c)) {
        eprintln!("{why}");
        std::process::exit(1);
    }
    let site = Some(env("SITE", "")).filter(|s| !s.is_empty()).map(PathBuf::from);
    let oidc = match oidc::Config::from(setting) {
        Ok(c) => c.map(|c| Arc::new(oidc::Oidc::new(c))),
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(1);
        }
    };
    if let Some(o) = &oidc {
        for w in o.cfg.warnings() {
            eprintln!("{w}");
        }
        if auth {
            println!("Single sign-on: {}.", o.cfg.issuer);
            let o = o.clone();
            tokio::task::spawn_blocking(move || match o.check() {
                Ok(()) => println!("Single sign-on: the identity provider answered."),
                Err(why) => {
                    eprintln!("Single sign-on: {why}. Sign-in through it won't work until this is fixed (it is tried again at each sign-in).")
                }
            });
        } else {
            println!("Single sign-on is set, but sign-in is off (STARBASE_AUTH=off), so it isn't used.");
        }
    }
    let app =
        Arc::new(App { store, base, lock: Mutex::new(()), location: env("LOCATION", "the server"), auth, site, rooms: live::Rooms::default(), oidc });
    if let Err(e) = std::fs::create_dir_all(app.store.meta()) {
        eprintln!("Can't use the folder {}: {e}", app.store.root.display());
        std::process::exit(1);
    }
    println!("Starbase server {}: {} on :{port}, keeping {} versions", env!("CARGO_PKG_VERSION"), app.store.root.display(), app.store.keep);
    if !auth {
        println!("Sign-in is OFF (STARBASE_AUTH=off): anyone who can reach this port can read and change every file.");
    } else if !auth::any_users(&app.store) {
        if auth::given_setup_code().is_some() {
            println!("No accounts yet. Open Starbase and create the owner account with the setup code set in STARBASE_PASSWORD.");
        } else {
            println!("No accounts yet. Open Starbase and create the owner account with this setup code: {}", auth::setup_code(&app.store));
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
    // The photo index starts now (its first rescan runs in the background); pictures are read in
    // child processes of this program, each with limits of its own.
    match std::env::current_exe() {
        Ok(exe) => photos::use_child_processes(exe),
        Err(e) => eprintln!("Photos: pictures are read in this process ({e})"),
    }
    if let Some(p) = photos::service(&app.store) {
        if p.cache_dir().starts_with(&app.store.root) {
            println!(
                "Photos: index and thumbnails in {} (inside the storage folder; set STARBASE_PHOTOS_CACHE or mount a volume at /cache to keep them on a local disk).",
                p.cache_dir().display()
            );
        } else {
            println!("Photos: index and thumbnails in {}.", p.cache_dir().display());
        }
        println!("Photos: {} workers, {} MB each.", p.cfg.workers, p.cfg.child_memory / 1024 / 1024);
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

    #[test]
    fn a_redirect_a_header_cant_hold_goes_home_instead_of_panicking() {
        // A line break can't be in a header ("/é" can, as raw bytes; safe_next refuses both).
        let r = see_other("/a\nb", &["a=b".into()]);
        assert_eq!((r.status().as_u16(), &r.headers()[header::LOCATION]), (303, &header::HeaderValue::from_static("/")));
        assert_eq!(r.headers()[header::SET_COOKIE], "a=b");
        assert_eq!(see_other("/docs/?file=1", &[]).headers()[header::LOCATION], "/docs/?file=1");
    }

    #[test]
    fn settings_take_the_new_name_and_the_old_one() {
        fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
            move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        }
        let old = env(&[("STARSHEETS_PASSWORD", "old")]);
        assert_eq!(setting_from("STARBASE_PASSWORD", &old).as_deref(), Some("old"), "the old name still works");
        let both = env(&[("STARSHEETS_PASSWORD", "old"), ("STARBASE_PASSWORD", "new")]);
        assert_eq!(setting_from("STARBASE_PASSWORD", &both).as_deref(), Some("new"), "the new name wins");
        assert_eq!(setting_from("STARSHEETS_PASSWORD", &both).as_deref(), Some("new"), "either name asks for the same setting");
        assert_eq!(setting_from("PORT", env(&[("PORT", "1")])).as_deref(), Some("1"), "other names are read as they are");
        assert_eq!(setting_from("STARBASE_AUTH", &both), None);
    }

    fn app(auth: bool) -> Shared {
        let dir = std::env::temp_dir().join(format!("starsheets-main-{}", store::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store { root: dir.clone(), keep: 3, spaces: auth };
        std::fs::create_dir_all(store.meta()).unwrap();
        Arc::new(App { store, base: dir, lock: Mutex::new(()), location: "here".into(), auth, site: None, rooms: live::Rooms::default(), oidc: None })
    }

    /// A made-up address per call site, so the try limits of one test don't touch another's.
    fn address() -> String {
        let r = store::random_hex(3);
        let b = |i: usize| u8::from_str_radix(&r[i..i + 2], 16).unwrap();
        format!("10.{}.{}.{}", b(0), b(2), b(4))
    }

    pub(crate) struct Got {
        pub(crate) status: u16,
        pub(crate) headers: HeaderMap,
        pub(crate) json: Value,
        pub(crate) bytes: Vec<u8>,
    }

    pub(crate) async fn send(app: &Shared, ip: &str, method: &str, uri: &str, cookie: &str, extra: &[(&str, &str)], body: Vec<u8>) -> Got {
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

    pub(crate) async fn post(app: &Shared, ip: &str, uri: &str, cookie: &str, v: Value) -> Got {
        send(app, ip, "POST", uri, cookie, &[], serde_json::to_vec(&v).unwrap()).await
    }

    pub(crate) fn cookie_of(g: &Got) -> String {
        g.headers.get(header::SET_COOKIE).and_then(|v| v.to_str().ok()).unwrap_or("").split(';').next().unwrap_or("").to_string()
    }

    /// A server with its owner ("boss"), signed in: (app, address, owner's cookie).
    pub(crate) async fn owned() -> (Shared, String, String) {
        let app = app(true);
        let ip = address();
        let code = auth::setup_code(&app.store);
        let g = post(&app, &ip, "/api/setup", "", json!({"code": code, "username": "boss", "name": "Boss", "password": "bosspassword"})).await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        (app, ip, cookie_of(&g))
    }

    fn totp_for(secret: &str, ahead: u64) -> String {
        format!("{:06}", twostep::code_at(&twostep::unbase32(secret), (store::now() / 30.0) as u64 + ahead))
    }

    #[tokio::test]
    async fn two_step_sign_in() {
        let (app, ip, boss) = owned().await;
        assert_eq!(post(&app, &ip, "/api/users", &boss, json!({"username": "ann", "name": "Ann", "password": "annpassword"})).await.status, 200);
        assert_eq!(post(&app, &ip, "/api/users", &boss, json!({"username": "bob", "name": "Bob", "password": "bobpassword"})).await.status, 200);
        let ann = cookie_of(&post(&app, &ip, "/api/login", "", json!({"username": "ann", "password": "annpassword"})).await);
        let start = post(&app, &ip, "/api/me/2fa/start", &ann, json!({})).await;
        assert_eq!(start.status, 200, "{:?}", start.json);
        let secret = start.json["secret"].as_str().unwrap().to_string();
        assert!(start.json["uri"].as_str().unwrap().starts_with("otpauth://totp/Starbase:ann?secret="));
        assert!(start.json["qr"].as_array().unwrap().len() > 20, "a QR code comes with it");
        // Not on until a right code confirms it.
        assert_eq!(post(&app, &ip, "/api/login", "", json!({"username": "ann", "password": "annpassword"})).await.status, 200);
        assert_eq!(post(&app, &ip, "/api/me/2fa/on", &ann, json!({"code": "000000"})).await.status, 400, "wrong code refused");
        let on = post(&app, &ip, "/api/me/2fa/on", &ann, json!({"code": totp_for(&secret, 0)})).await;
        assert_eq!(on.status, 200, "{:?}", on.json);
        let backups: Vec<String> = on.json["backup_codes"].as_array().unwrap().iter().map(|c| c.as_str().unwrap().to_string()).collect();
        assert_eq!(backups.len(), 10);
        let users = std::fs::read_to_string(app.store.meta().join("users.json")).unwrap();
        assert!(!users.contains(&backups[0]), "backup codes are stored hashed");
        let login = |code: Option<String>| {
            let body = match code {
                Some(c) => json!({"username": "ann", "password": "annpassword", "code": c}),
                None => json!({"username": "ann", "password": "annpassword"}),
            };
            post(&app, &ip, "/api/login", "", body)
        };
        let g = login(None).await;
        assert_eq!((g.status, g.json["two_step"].as_bool()), (401, Some(true)), "password alone no longer signs in");
        assert!(g.headers.get(header::SET_COOKIE).is_none());
        assert_eq!(login(Some("123456".into())).await.status, 401, "wrong code");
        assert_eq!(login(Some(totp_for(&secret, 0))).await.status, 401, "the code used to turn it on is spent");
        let g = login(Some(totp_for(&secret, 1))).await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        assert_eq!(login(Some(totp_for(&secret, 1))).await.status, 401, "replay in the same step refused");
        // A backup code works once.
        assert_eq!(login(Some(backups[0].clone())).await.status, 200);
        assert_eq!(login(Some(backups[0].clone())).await.status, 401, "a backup code is used up");
        // Someone without two-step sign-in is unaffected.
        assert_eq!(post(&app, &ip, "/api/login", "", json!({"username": "bob", "password": "bobpassword"})).await.status, 200);
        // Turning off needs a code; the owner can do it for someone who lost their phone.
        assert_eq!(post(&app, &ip, "/api/me/2fa/off", &ann, json!({"code": "111111"})).await.status, 400);
        assert_eq!(post(&app, &ip, "/api/me/2fa/off", &ann, json!({"code": backups[1]})).await.status, 200);
        assert_eq!(login(None).await.status, 200, "off again");
        let ann = cookie_of(&login(None).await);
        let secret = post(&app, &ip, "/api/me/2fa/start", &ann, json!({})).await.json["secret"].as_str().unwrap().to_string();
        assert_eq!(post(&app, &ip, "/api/me/2fa/on", &ann, json!({"code": totp_for(&secret, 0)})).await.status, 200);
        assert_eq!(send(&app, &ip, "DELETE", "/api/users/2fa?username=ann", &ann, &[], vec![]).await.status, 403, "only the owner");
        assert_eq!(send(&app, &ip, "DELETE", "/api/users/2fa?username=ann", &boss, &[], vec![]).await.status, 200);
        assert_eq!(login(None).await.status, 200, "the owner turned it off");
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
    async fn external_content_is_blocked_in_files_from_outside_until_allowed() {
        let (app, ip, boss) = owned().await;
        let ext = |id: &str| format!("/api/files/{id}/external");
        let made = send(&app, &ip, "POST", "/api/files?name=Mine.xlsx", &boss, &[], b"one".to_vec()).await;
        let mine = made.json["id"].as_str().unwrap().to_string();
        assert_eq!(send(&app, &ip, "GET", &ext(&mine), &boss, &[], vec![]).await.json["allowed"], true, "own file: as before");
        let up = send(&app, &ip, "POST", "/api/files?name=From%20mail.xlsx&as_is=1", &boss, &[], b"two".to_vec()).await;
        let up = up.json["id"].as_str().unwrap().to_string();
        assert_eq!(send(&app, &ip, "GET", &ext(&up), &boss, &[], vec![]).await.json["allowed"], false, "uploaded: blocked");
        let copy = send(&app, &ip, "POST", &format!("/api/files/{mine}/copy"), &boss, &[], vec![]).await;
        let copy = copy.json["id"].as_str().unwrap().to_string();
        assert_eq!(send(&app, &ip, "GET", &ext(&copy), &boss, &[], vec![]).await.json["allowed"], false, "a copy: blocked");
        assert_eq!(post(&app, &ip, &ext(&up), &boss, json!({"allow": true})).await.status, 200);
        assert_eq!(send(&app, &ip, "GET", &ext(&up), &boss, &[], vec![]).await.json["allowed"], true, "remembered");
        assert_eq!(send(&app, &ip, "GET", &ext(&up), "", &[], vec![]).await.status, 401);
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
    async fn public_links_expire_hide_download_and_take_a_password() {
        let (app, ip, boss) = owned().await;
        let id = send(&app, &ip, "POST", "/api/files?name=Pic.png&as_is=1", &boss, &[], b"PNGDATA".to_vec()).await.json["id"]
            .as_str()
            .unwrap()
            .to_string();
        let sh = format!("/api/files/{id}/sharing");
        let g = post(&app, &ip, &sh, &boss, json!({"public": {"on": true, "expires": "2999-01-01", "no_download": true}})).await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        assert_eq!(g.json["public"]["expires"], "2999-01-01");
        let t = g.json["public"]["token"].as_str().unwrap().to_string();
        let page = send(&app, &ip, "GET", &format!("/api/public/{t}"), "", &[], vec![]).await;
        assert_eq!(page.status, 200, "signed out");
        let html = String::from_utf8_lossy(&page.bytes).to_string();
        assert!(html.contains("/view") && !html.contains("Download"), "view only: no download button");
        assert_eq!(send(&app, &ip, "GET", &format!("/api/public/{t}/download"), "", &[], vec![]).await.status, 403);
        assert_eq!(send(&app, &ip, "GET", &format!("/api/public/{t}/view"), "", &[], vec![]).await.bytes, b"PNGDATA");
        // Download allowed again.
        post(&app, &ip, &sh, &boss, json!({"public": {"on": true, "no_download": false}})).await;
        assert_eq!(send(&app, &ip, "GET", &format!("/api/public/{t}/download"), "", &[], vec![]).await.bytes, b"PNGDATA");
        // Password.
        post(&app, &ip, &sh, &boss, json!({"public": {"on": true, "password": "opensesame"}})).await;
        let dl = format!("/api/public/{t}/download");
        let get = |uri: String, cookie: String| {
            let (app, ip) = (app.clone(), ip.clone());
            async move { send(&app, &ip, "GET", &uri, &cookie, &[], vec![]).await }
        };
        assert_eq!(get(dl.clone(), String::new()).await.status, 200, "asks for the password");
        assert!(!get(dl.clone(), String::new()).await.bytes.starts_with(b"PNGDATA"));
        // The password goes in a form POST, never in the address.
        let unlock = |pw: &str| {
            let (app, ip, uri, body) = (app.clone(), ip.clone(), format!("/api/public/{t}"), format!("pw={pw}").into_bytes());
            async move { send(&app, &ip, "POST", &uri, "", &[("content-type", "application/x-www-form-urlencoded")], body).await }
        };
        let bad = unlock("nope").await;
        assert_eq!(bad.status, 401);
        assert!(bad.headers.get(header::SET_COOKIE).is_none());
        assert!(!get(format!("{dl}?pw=opensesame"), String::new()).await.bytes.starts_with(b"PNGDATA"), "?pw= no longer works");
        let ok = unlock("opensesame").await;
        assert_eq!(ok.status, 303);
        let set = ok.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap().to_string();
        assert!(set.contains("HttpOnly") && set.contains(&format!("Path=/api/public/{t}")), "{set}");
        let cookie = set.split(';').next().unwrap().to_string();
        assert_eq!(get(dl.clone(), cookie.clone()).await.bytes, b"PNGDATA");
        assert!(!get(dl.clone(), "pl_x=1".into()).await.bytes.starts_with(b"PNGDATA"), "a made-up cookie is not a key");
        // Expired.
        post(&app, &ip, &sh, &boss, json!({"public": {"on": true, "expires": "2001-02-03"}})).await;
        let g = get(format!("/api/public/{t}"), cookie.clone()).await;
        assert_eq!(g.status, 410);
        assert!(String::from_utf8_lossy(&g.bytes).contains("This link has expired"));
        // Bad date refused; off removes it.
        assert_eq!(post(&app, &ip, &sh, &boss, json!({"public": {"on": true, "expires": "soon"}})).await.status, 400);
        let g = post(&app, &ip, &sh, &boss, json!({"public": {"on": false}})).await;
        assert_eq!(g.json["public"]["on"], false);
        assert_eq!(send(&app, &ip, "GET", &format!("/api/public/{t}"), "", &[], vec![]).await.status, 404);
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

    /// New from a template (every template): a copy "<name> (copy).<ext>" in the folder asked
    /// for, holding the template's file; again → "(copy) (2)"; the templates themselves are
    /// unchanged; someone without edit access to the folder can't; an unknown key is a 404.
    #[tokio::test]
    async fn new_from_template_copies_into_the_folder() {
        let (app, ip, boss) = owned().await;
        let g = post(&app, &ip, "/api/users", &boss, json!({"username": "vic", "name": "vic", "password": "vicpassword"})).await;
        assert_eq!(g.status, 200);
        let vic = cookie_of(&post(&app, &ip, "/api/login", "", json!({"username": "vic", "password": "vicpassword"})).await);
        let folder = send(&app, &ip, "POST", "/api/folders?name=Work", &boss, &[], vec![]).await.json["path"].as_str().unwrap().to_string();
        let before: Vec<Vec<u8>> = starbase_templates::ALL.iter().map(|t| starbase_templates::bytes(t.key).unwrap().to_vec()).collect();
        for t in starbase_templates::ALL {
            let url = format!("/api/templates/{}?folder={folder}", t.key);
            let g = send(&app, &ip, "POST", &url, &boss, &[], vec![]).await;
            assert_eq!(g.status, 201, "{}: {:?}", t.key, g.json);
            assert_eq!(g.json["name"], format!("{} (copy).{}", t.name, t.kind.ext()), "{}", t.key);
            assert_eq!(g.json["folder"], folder.as_str(), "{}", t.key);
            let id = g.json["id"].as_str().unwrap().to_string();
            let got = send(&app, &ip, "GET", &format!("/api/files/{id}"), &boss, &[], vec![]).await;
            assert_eq!(got.status, 200, "{}", t.key);
            assert_eq!(got.bytes, starbase_templates::bytes(t.key).unwrap(), "{}: the copy is the template", t.key);
            let again = send(&app, &ip, "POST", &url, &boss, &[], vec![]).await;
            assert_eq!(again.json["name"], format!("{} (copy) (2).{}", t.name, t.kind.ext()), "{}", t.key);
            assert_eq!(send(&app, &ip, "POST", &url, &vic, &[], vec![]).await.status, 403, "{}: not vic's folder", t.key);
        }
        let after: Vec<Vec<u8>> = starbase_templates::ALL.iter().map(|t| starbase_templates::bytes(t.key).unwrap().to_vec()).collect();
        assert_eq!(before, after, "the templates are unchanged");
        // The Drive opened without `dir` (no folder known yet): the copy goes to the person's
        // own top folder, never to a folder with an empty name.
        for url in ["/api/templates/monthly-budget", "/api/templates/monthly-budget?folder="] {
            let g = send(&app, &ip, "POST", url, &boss, &[], vec![]).await;
            assert_eq!(g.status, 201, "{url}: {:?}", g.json);
            assert_eq!(g.json["folder"], "boss", "{url}: into the top folder");
            assert_eq!(send(&app, &ip, "POST", url, &vic, &[], vec![]).await.json["folder"], "vic", "{url}: vic's own top folder");
        }
        assert_eq!(send(&app, &ip, "POST", "/api/templates/nope", &boss, &[], vec![]).await.status, 404);
        assert_eq!(send(&app, &ip, "GET", "/api/templates/cv", &boss, &[], vec![]).await.status, 404);
    }

    /// Details > Activity: what happened to a file, newest first; a sharee (a viewer, not the
    /// owner) reads it, a person with no access gets a 403.
    #[tokio::test]
    async fn file_activity_for_a_sharee_and_not_a_stranger() {
        let (app, ip, boss) = owned().await;
        for who in ["ann", "vic"] {
            let g = post(&app, &ip, "/api/users", &boss, json!({"username": who, "name": who, "password": format!("{who}password")})).await;
            assert_eq!(g.status, 200);
        }
        let ann = cookie_of(&post(&app, &ip, "/api/login", "", json!({"username": "ann", "password": "annpassword"})).await);
        let vic = cookie_of(&post(&app, &ip, "/api/login", "", json!({"username": "vic", "password": "vicpassword"})).await);
        let id = send(&app, &ip, "POST", "/api/files?name=Letter.docx", &boss, &[], b"one".to_vec()).await.json["id"].as_str().unwrap().to_string();
        let url = format!("/api/files/{id}/activity");
        assert_eq!(send(&app, &ip, "GET", &url, &ann, &[], vec![]).await.status, 403, "no access yet");
        assert_eq!(send(&app, &ip, "GET", &url, &vic, &[], vec![]).await.status, 403, "a stranger sees nothing");
        assert_eq!(post(&app, &ip, &format!("/api/files/{id}/sharing"), &boss, json!({"people": {"ann": "view"}, "link": "view"})).await.status, 200);
        assert_eq!(send(&app, &ip, "POST", &format!("/api/files/{id}/rename?name=Memo"), &boss, &[], vec![]).await.status, 200);
        assert_eq!(send(&app, &ip, "PUT", &format!("/api/files/{id}"), &boss, &[], b"two".to_vec()).await.status, 200);
        let g = send(&app, &ip, "GET", &url, &ann, &[], vec![]).await;
        assert_eq!(g.status, 200, "a view-only sharee may read it: {:?}", g.json);
        let kinds: Vec<&str> = g.json["activity"].as_array().unwrap().iter().map(|e| e["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["edited", "renamed", "link", "shared", "created"], "newest first");
        assert_eq!(g.json["activity"][0]["by"], "boss");
        assert_eq!(g.json["activity"][3]["detail"], "ann (can view)");
        let _ = std::fs::remove_dir_all(&app.store.root);
    }

    /// Docs' version history: a document's saves become versions with who saved them; editors
    /// see, name and restore them (a restore is a new version); viewers can't, as in Google Docs.
    #[tokio::test]
    async fn document_version_history() {
        let (app, ip, boss) = owned().await;
        for who in ["ann", "vic"] {
            let g = post(&app, &ip, "/api/users", &boss, json!({"username": who, "name": who, "password": format!("{who}password")})).await;
            assert_eq!(g.status, 200);
        }
        let ann = cookie_of(&post(&app, &ip, "/api/login", "", json!({"username": "ann", "password": "annpassword"})).await);
        let vic = cookie_of(&post(&app, &ip, "/api/login", "", json!({"username": "vic", "password": "vicpassword"})).await);
        let id = send(&app, &ip, "POST", "/api/files?name=Letter.docx", &boss, &[], b"one".to_vec()).await.json["id"].as_str().unwrap().to_string();
        let g = post(&app, &ip, &format!("/api/files/{id}/sharing"), &boss, json!({"people": {"ann": "edit", "vic": "view"}})).await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        assert_eq!(send(&app, &ip, "PUT", &format!("/api/files/{id}"), &ann, &[], b"two".to_vec()).await.status, 200);
        let h = send(&app, &ip, "GET", &format!("/api/files/{id}/history"), &ann, &[], vec![]).await;
        assert_eq!(h.status, 200);
        assert_eq!(h.json["current"]["author"], "ann", "who saved the file as it is now");
        let vs = h.json["versions"].as_array().unwrap().clone();
        assert_eq!(vs.len(), 1, "a .docx save keeps the earlier version: {vs:?}");
        assert_eq!(vs[0]["author"], "boss");
        let v = vs[0]["v"].as_str().unwrap().to_string();
        assert!(v.ends_with(".docx"), "{v}");
        // Viewers see the document but not its history.
        for url in [format!("/api/files/{id}/history"), format!("/api/files/{id}/versions"), format!("/api/files/{id}/versions/{v}")] {
            assert_eq!(send(&app, &ip, "GET", &url, &vic, &[], vec![]).await.status, 403, "{url}");
        }
        assert_eq!(send(&app, &ip, "POST", &format!("/api/files/{id}/versions/{v}/restore"), &vic, &[], vec![]).await.status, 403);
        assert_eq!(send(&app, &ip, "POST", &format!("/api/files/{id}/versions/current/name?name=X"), &vic, &[], vec![]).await.status, 403);
        // Name the current version; the next save carries the name to the version it becomes.
        let g = send(&app, &ip, "POST", &format!("/api/files/{id}/versions/current/name?name=Final%20draft"), &ann, &[], vec![]).await;
        assert_eq!(g.status, 200);
        // Restore the first version: a new version, nothing deleted, and the restorer is its author.
        let g = send(&app, &ip, "POST", &format!("/api/files/{id}/versions/{v}/restore"), &boss, &[], vec![]).await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        assert_eq!(send(&app, &ip, "GET", &format!("/api/files/{id}"), &boss, &[], vec![]).await.bytes, b"one");
        let h = send(&app, &ip, "GET", &format!("/api/files/{id}/history"), &boss, &[], vec![]).await;
        assert_eq!(h.json["current"]["author"], "boss");
        assert!(h.json["current"]["restored_from"].is_i64(), "{:?}", h.json);
        let vs = h.json["versions"].as_array().unwrap();
        assert_eq!(vs.len(), 2, "{vs:?}");
        assert_eq!((vs[0]["author"].as_str(), vs[0]["name"].as_str()), (Some("ann"), Some("Final draft")));
        let named = vs[0]["v"].as_str().unwrap();
        assert_eq!(send(&app, &ip, "GET", &format!("/api/files/{id}/versions/{named}"), &ann, &[], vec![]).await.bytes, b"two");
        let _ = std::fs::remove_dir_all(&app.store.root);
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

    // ------------------------------------------------------------ desktop apps (devices.rs)

    const SAME: (&str, &str) = ("origin", "http://192.168.1.5:8080");
    const EVIL: (&str, &str) = ("origin", "https://evil.example");

    /// A desktop app asking to connect: (device_code, user_code).
    async fn start_device(app: &Shared, ip: &str, name: &str) -> (String, String) {
        let g = post(app, ip, "/api/devices/start", "", json!({ "name": name })).await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        let uc = g.json["user_code"].as_str().unwrap().to_string();
        assert_eq!(g.json["verify_url"], "http://192.168.1.5:8080/api/connect", "never with the code in it");
        (g.json["device_code"].as_str().unwrap().to_string(), uc)
    }

    async fn approve(app: &Shared, ip: &str, cookie: &str, extra: &[(&str, &str)], code: &str, allow: bool) -> Got {
        send(app, ip, "POST", "/api/devices/approve", cookie, extra, serde_json::to_vec(&json!({ "code": code, "allow": allow })).unwrap()).await
    }

    async fn poll_device(app: &Shared, ip: &str, dc: &str) -> Got {
        post(app, ip, "/api/devices/poll", "", json!({ "device_code": dc })).await
    }

    /// A device token for the account signed in with `cookie`, through the whole flow.
    async fn device_token(app: &Shared, ip: &str, cookie: &str, name: &str) -> String {
        let (dc, uc) = start_device(app, ip, name).await;
        assert_eq!(approve(app, ip, cookie, &[SAME], &uc, true).await.status, 200);
        let g = poll_device(app, ip, &dc).await;
        assert_eq!(g.json["status"], "ok", "{:?}", g.json);
        g.json["token"].as_str().unwrap().to_string()
    }

    fn bearer(t: &str) -> String {
        format!("Bearer {t}")
    }

    fn body_of(v: Value) -> Vec<u8> {
        serde_json::to_vec(&v).unwrap()
    }

    #[tokio::test]
    async fn desktop_sign_in_needs_the_person_in_their_browser() {
        let (app, ip, boss) = owned().await;
        let (dc, uc) = start_device(&app, &ip, "Work laptop").await;
        assert_eq!(uc.len(), 9, "ABCD-EFGH");
        assert_eq!(poll_device(&app, &ip, &dc).await.json["status"], "pending");
        // The connect page's questions need a browser session.
        let ask = format!("/api/devices/code?code={uc}");
        assert_eq!(send(&app, &ip, "GET", &ask, "", &[], vec![]).await.status, 401);
        let g = send(&app, &ip, "GET", &ask, &boss, &[], vec![]).await;
        assert_eq!((g.status, g.json["name"].as_str()), (200, Some("Work laptop")));
        // Where it came from, so a request someone else started stands out.
        assert_eq!((g.json["from"].as_str(), g.json["same_place"].as_bool()), (Some(ip.as_str()), Some(true)));
        let elsewhere = send(&app, &address(), "GET", &ask, &boss, &[], vec![]).await;
        assert_eq!(elsewhere.json["same_place"], false);
        // Approving: signed in, from this site's own page (CSRF: another site, or no Origin, is refused).
        assert_eq!(approve(&app, &ip, "", &[SAME], &uc, true).await.status, 401, "signed in");
        assert_eq!(approve(&app, &ip, &boss, &[EVIL], &uc, true).await.status, 403, "another site");
        assert_eq!(approve(&app, &ip, &boss, &[], &uc, true).await.status, 403, "no Origin");
        assert_eq!(poll_device(&app, &ip, &dc).await.json["status"], "pending", "none of those counted");
        // A device token can't approve another device (only the person, in their browser).
        let other = device_token(&app, &ip, &boss, "Other").await;
        let other = bearer(&other);
        assert_eq!(approve(&app, &ip, "", &[SAME, ("authorization", &other)], &uc, true).await.status, 401);
        assert_eq!(approve(&app, &ip, &boss, &[SAME, ("authorization", &other)], &uc, true).await.status, 401, "with a token, the cookie isn't read");
        assert_eq!(approve(&app, &ip, &boss, &[SAME], &uc.to_lowercase(), true).await.status, 200, "typed in lower case");
        assert_eq!(approve(&app, &ip, &boss, &[SAME], &uc, true).await.status, 404, "a code is used once");
        let g = poll_device(&app, &ip, &dc).await;
        assert_eq!(g.json["status"], "ok");
        assert_eq!(g.json["user"]["username"], "boss");
        let token = g.json["token"].as_str().unwrap().to_string();
        assert_eq!(poll_device(&app, &ip, &dc).await.json["status"], "unknown", "the token is handed out once");
        // The token signs the app in on the API (no Origin: not a browser).
        let auth = bearer(&token);
        assert_eq!(send(&app, &ip, "GET", "/api/files", "", &[("authorization", &auth)], vec![]).await.status, 200);
        let g = send(&app, &ip, "POST", "/api/folders?name=Desk", "", &[("authorization", &auth)], vec![]).await;
        assert_eq!(g.status, 201, "{:?}", g.json);
        // Never in the log, nor in the server's own files.
        let log = std::fs::read_to_string(app.store.meta().join("log.txt")).unwrap_or_default();
        assert!(log.contains("allowed a desktop app") && !log.contains(&token));
        for f in std::fs::read_dir(app.store.meta()).unwrap() {
            let text = std::fs::read_to_string(f.unwrap().path()).unwrap_or_default();
            assert!(!text.contains(&token[devices::PREFIX.len()..]));
        }
        // A new password between "Allow" and the app collecting the token: it never works.
        let (dc, uc) = start_device(&app, &ip, "Slow app").await;
        assert_eq!(approve(&app, &ip, &boss, &[SAME], &uc, true).await.status, 200);
        let g =
            send(&app, &ip, "POST", "/api/me", &boss, &[SAME], body_of(json!({"current_password": "bosspassword", "password": "bossnewpassword"})))
                .await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        let boss = cookie_of(&g);
        let g = poll_device(&app, &ip, &dc).await;
        assert_eq!((g.status, g.json["status"].as_str(), g.json.get("token")), (403, Some("denied"), None));
        // A proxy's own Basic password (browsers resend it by themselves) changes nothing: the
        // cookie still signs in, and another site is still refused.
        let basic = ("authorization", "Basic dXNlcjpwYXNz");
        assert_eq!(send(&app, &ip, "GET", "/api/files", &boss, &[basic], vec![]).await.status, 200);
        assert_eq!(send(&app, &ip, "POST", "/api/folders?name=Z", "", &[EVIL, basic], vec![]).await.status, 403);
        // Not allowed.
        let (dc, uc) = start_device(&app, &ip, "Stranger").await;
        assert_eq!(approve(&app, &ip, &boss, &[SAME], &uc, false).await.status, 200);
        assert_eq!(poll_device(&app, &ip, &dc).await.json["status"], "denied");
        assert_eq!(send(&app, &ip, "GET", "/api/devices/code?code=XXXX-XXXX", &boss, &[], vec![]).await.status, 404);
        let _ = std::fs::remove_dir_all(&app.store.root);
    }

    #[tokio::test]
    async fn device_tokens_get_their_account_rights_and_no_more() {
        let (app, ip, boss) = owned().await;
        let g = send(&app, &ip, "POST", "/api/users", &boss, &[SAME], body_of(json!({"username": "ann", "name": "Ann", "password": "annpassword"})))
            .await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        let ann = cookie_of(&post(&app, &ip, "/api/login", "", json!({"username": "ann", "password": "annpassword"})).await);
        let token = device_token(&app, &ip, &ann, "Ann's laptop").await;
        let auth = bearer(&token);
        let me = send(&app, &ip, "GET", "/api/info", "", &[("authorization", &auth)], vec![]).await;
        assert_eq!(me.json["user"]["username"], "ann");
        // Owner-only: no.
        let eve = body_of(json!({"username": "eve", "name": "Eve", "password": "evepassword"}));
        assert_eq!(send(&app, &ip, "POST", "/api/users", "", &[("authorization", &auth)], eve.clone()).await.status, 403);
        assert_eq!(send(&app, &ip, "GET", "/api/devices?username=boss", "", &[("authorization", &auth)], vec![]).await.status, 403);
        // Cross-site: a token without cookies is the app's own (nothing ambient for another site
        // to borrow); with the browser's cookies too, another site is refused, and the cookie
        // never adds anything: a wrong token with the owner's cookie is simply signed out.
        assert_eq!(send(&app, &ip, "POST", "/api/folders?name=X", "", &[EVIL, ("authorization", &auth)], vec![]).await.status, 201);
        assert_eq!(send(&app, &ip, "POST", "/api/folders?name=Y", &boss, &[EVIL, ("authorization", &auth)], vec![]).await.status, 403);
        assert_eq!(send(&app, &ip, "GET", "/api/files", &boss, &[("authorization", "Bearer sbd_wrong")], vec![]).await.status, 401);
        let g = send(&app, &ip, "POST", "/api/users", &boss, &[SAME, ("authorization", &auth)], eve).await;
        assert_eq!(g.status, 403, "Ann's token with the owner's cookie is still Ann");
        // Your devices, and the owner in People.
        let g = send(&app, &ip, "GET", "/api/devices", "", &[("authorization", &auth)], vec![]).await;
        assert_eq!(g.json.as_array().unwrap().len(), 1);
        assert_eq!(g.json[0]["current"], true);
        assert_eq!(g.json[0]["name"], "Ann's laptop");
        let g = send(&app, &ip, "GET", "/api/devices?username=ann", &boss, &[], vec![]).await;
        let id = g.json[0]["id"].as_str().unwrap().to_string();
        assert_eq!(send(&app, &ip, "DELETE", &format!("/api/devices?username=ann&id={id}"), &boss, &[SAME], vec![]).await.status, 200);
        assert_eq!(send(&app, &ip, "GET", "/api/files", "", &[("authorization", &auth)], vec![]).await.status, 401, "removed: stops at once");
        // The app disconnecting itself; a new password removes every device.
        let t2 = bearer(&device_token(&app, &ip, &ann, "Desk").await);
        assert_eq!(send(&app, &ip, "DELETE", "/api/devices?id=this", "", &[("authorization", &t2)], vec![]).await.status, 200);
        assert_eq!(send(&app, &ip, "GET", "/api/files", "", &[("authorization", &t2)], vec![]).await.status, 401);
        let t3 = bearer(&device_token(&app, &ip, &ann, "Desk").await);
        let g = send(&app, &ip, "POST", "/api/me", &ann, &[SAME], body_of(json!({"current_password": "annpassword", "password": "a new password"})))
            .await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        assert_eq!(send(&app, &ip, "GET", "/api/files", "", &[("authorization", &t3)], vec![]).await.status, 401);
        let _ = std::fs::remove_dir_all(&app.store.root);
    }

    /// Reads one WebSocket frame from the server: (opcode, payload).
    async fn read_frame(s: &mut tokio::net::TcpStream) -> Option<(u8, Vec<u8>)> {
        use tokio::io::AsyncReadExt;
        let mut h = [0u8; 2];
        s.read_exact(&mut h).await.ok()?;
        let len = match h[1] & 0x7f {
            126 => {
                let mut l = [0u8; 2];
                s.read_exact(&mut l).await.ok()?;
                u16::from_be_bytes(l) as usize
            }
            127 => {
                let mut l = [0u8; 8];
                s.read_exact(&mut l).await.ok()?;
                u64::from_be_bytes(l) as usize
            }
            n => n as usize,
        };
        let mut body = vec![0u8; len];
        s.read_exact(&mut body).await.ok()?;
        Some((h[0] & 0x0f, body))
    }

    /// A removed device's open live connection ends at once (over a real socket).
    #[tokio::test]
    async fn removing_a_device_ends_its_live_connection() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (app, ip, boss) = owned().await;
        let token = device_token(&app, &ip, &boss, "Laptop").await;
        let g = send(&app, &ip, "POST", "/api/files?name=t.csv", &boss, &[SAME], b"a,b\n1,2\n".to_vec()).await;
        let file = g.json["id"].as_str().unwrap().to_string();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let served = router(app.clone()).into_make_service_with_connect_info::<SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, served).await });
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let req = format!(
            "GET /api/live HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nAuthorization: Bearer {token}\r\n\r\n"
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut b = [0u8; 1];
            s.read_exact(&mut b).await.unwrap();
            head.push(b[0]);
        }
        assert!(head.starts_with(b"HTTP/1.1 101"), "{}", String::from_utf8_lossy(&head));
        // One masked text frame: the hello.
        let hello = serde_json::to_vec(&starsheets_sync::ToServer::Hello {
            file,
            build: "test".into(),
            client: "t1".into(),
            protocol: starsheets_sync::PROTOCOL,
            have: None,
        })
        .unwrap();
        let mut frame = vec![0x81u8];
        if hello.len() < 126 {
            frame.push(0x80 | hello.len() as u8);
        } else {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(hello.len() as u16).to_be_bytes());
        }
        let mask = [1u8, 2, 3, 4];
        frame.extend_from_slice(&mask);
        frame.extend(hello.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        s.write_all(&frame).await.unwrap();
        // The welcome: the app is in.
        let (op, _) = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut s)).await.unwrap().unwrap();
        assert!(op == 1 || op == 2, "a message");
        // The owner removes the device in the browser.
        let g = send(&app, &ip, "GET", "/api/devices", &boss, &[], vec![]).await;
        let id = g.json[0]["id"].as_str().unwrap().to_string();
        assert_eq!(send(&app, &ip, "DELETE", &format!("/api/devices?id={id}"), &boss, &[SAME], vec![]).await.status, 200);
        let mut told = false;
        loop {
            match tokio::time::timeout(Duration::from_secs(10), read_frame(&mut s)).await.expect("the connection ends at once") {
                Some((1, body)) if String::from_utf8_lossy(&body).contains("disconnected from your account") => told = true,
                Some((8, _)) | None => break,
                Some(_) => {}
            }
        }
        assert!(told, "the app is told why");
        // And it can't connect again.
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        s.write_all(req.as_bytes()).await.unwrap();
        let mut head = [0u8; 12];
        s.read_exact(&mut head).await.unwrap();
        assert_eq!(&head, b"HTTP/1.1 401");
        let _ = std::fs::remove_dir_all(&app.store.root);
    }

    #[tokio::test]
    async fn the_connect_page_is_served() {
        let (app, ip, _) = owned().await;
        let g = send(&app, &ip, "GET", "/api/connect?code=ABCD-EFGH", "", &[], vec![]).await;
        assert_eq!(g.status, 200);
        let page = String::from_utf8_lossy(&g.bytes);
        assert!(page.contains("Connect an app") && page.contains("devices/approve"));
        assert_eq!(g.headers[header::X_FRAME_OPTIONS], "DENY", "never inside another site's frame");
        let _ = std::fs::remove_dir_all(&app.store.root);
    }

    /// The any-file Drive: every type is uploaded, listed, downloaded, previewed, renamed and
    /// moved; previews never run a file as a page; no way out of the store.
    #[tokio::test]
    async fn any_file_drive() {
        let (app, ip, boss) = owned().await;
        assert_eq!(post(&app, &ip, "/api/users", &boss, json!({"username": "ann", "name": "Ann", "password": "annpassword"})).await.status, 200);
        let ann = cookie_of(&post(&app, &ip, "/api/login", "", json!({"username": "ann", "password": "annpassword"})).await);
        let png = b"\x89PNG\r\n\x1a\nfake picture".to_vec();
        let up = |name: &str| format!("/api/files?name={name}&as_is=1");
        let g = send(&app, &ip, "POST", &up("photo.png"), &boss, &[], png.clone()).await;
        assert_eq!((g.status, g.json["name"].as_str(), g.json["folder"].as_str()), (201, Some("photo.png"), Some("boss")));
        let photo = g.json["id"].as_str().unwrap().to_string();
        // Without as_is a name that isn't a spreadsheet still becomes one (the apps' own saves).
        assert_eq!(send(&app, &ip, "POST", "/api/files?name=notes.txt", &boss, &[], b"x".to_vec()).await.json["name"], "notes.txt.xlsx");
        let id_of = |g: &Got| g.json["id"].as_str().unwrap().to_string();
        let html = id_of(&send(&app, &ip, "POST", &up("page.html"), &boss, &[], b"<script>alert(1)</script>".to_vec()).await);
        let svg = id_of(&send(&app, &ip, "POST", &up("pic.svg"), &boss, &[], b"<svg onload=alert(1)/>".to_vec()).await);
        let txt = id_of(&send(&app, &ip, "POST", &up("read%20me.md"), &boss, &[], b"# Hi <script>x</script>".to_vec()).await);
        let pdf = id_of(&send(&app, &ip, "POST", &up("paper.pdf"), &boss, &[], b"%PDF-1.4 fake".to_vec()).await);
        let exe = id_of(&send(&app, &ip, "POST", &up("tool.exe"), &boss, &[], b"MZ".to_vec()).await);
        let listed = send(&app, &ip, "GET", "/api/files", &boss, &[], vec![]).await.json;
        let names: Vec<&str> = listed["files"].as_array().unwrap().iter().filter_map(|f| f["name"].as_str()).collect();
        for n in ["photo.png", "page.html", "pic.svg", "read me.md", "paper.pdf", "tool.exe", "notes.txt.xlsx"] {
            assert!(names.contains(&n), "{n} listed: {names:?}");
        }
        // Download: the bytes, as an attachment, never sniffed.
        let g = send(&app, &ip, "GET", &format!("/api/files/{photo}"), &boss, &[], vec![]).await;
        assert_eq!((g.status, &g.bytes), (200, &png));
        assert!(g.headers[header::CONTENT_DISPOSITION].to_str().unwrap().starts_with("attachment"));
        assert_eq!(g.headers[header::CONTENT_TYPE], "application/octet-stream");
        assert_eq!(g.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        // The preview page: sandboxed, no script, the picture from raw.
        let g = send(&app, &ip, "GET", &format!("/api/files/{photo}/preview"), &boss, &[], vec![]).await;
        let page = String::from_utf8_lossy(&g.bytes).to_string();
        let csp = g.headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap().to_string();
        assert_eq!(g.status, 200);
        assert!(csp.starts_with("sandbox") && csp.contains("default-src 'none'") && !csp.contains("allow-scripts"), "{csp}");
        assert!(page.contains("<img src=\"raw?v=") && !page.contains("<script"), "{page}");
        // Raw: the exact type, inline, sandboxed; byte ranges.
        let g = send(&app, &ip, "GET", &format!("/api/files/{photo}/raw"), &boss, &[], vec![]).await;
        assert_eq!((g.status, g.headers[header::CONTENT_TYPE].to_str().unwrap()), (200, "image/png"));
        assert!(g.headers[header::CONTENT_DISPOSITION].to_str().unwrap().starts_with("inline"));
        assert!(g.headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap().starts_with("sandbox"));
        let g = send(&app, &ip, "GET", &format!("/api/files/{photo}/raw"), &boss, &[("range", "bytes=1-3")], vec![]).await;
        assert_eq!((g.status, g.bytes.as_slice()), (206, &png[1..4]));
        assert_eq!(g.headers[header::CONTENT_RANGE], format!("bytes 1-3/{}", png.len()).as_str());
        let g = send(&app, &ip, "GET", &format!("/api/files/{photo}/raw"), &boss, &[("range", "bytes=999-")], vec![]).await;
        assert_eq!(g.status, 416);
        // A video (Photos plays it from raw): its own type, and byte ranges so it can seek.
        let clip = b"\0\0\0\x18ftypmp42 fake clip bytes".to_vec();
        let clip_id = id_of(&send(&app, &ip, "POST", &up("clip.mp4"), &boss, &[], clip.clone()).await);
        let g = send(&app, &ip, "GET", &format!("/api/files/{clip_id}/raw"), &boss, &[("range", "bytes=4-9")], vec![]).await;
        assert_eq!((g.status, g.bytes.as_slice()), (206, &clip[4..10]));
        assert_eq!(g.headers[header::CONTENT_TYPE], "video/mp4");
        assert_eq!(g.headers[header::CONTENT_RANGE], format!("bytes 4-9/{}", clip.len()).as_str());
        // HTML, SVG, text and unknown types are never served to be shown as they are.
        for id in [&html, &svg, &exe, &txt] {
            assert_eq!(send(&app, &ip, "GET", &format!("/api/files/{id}/raw"), &boss, &[], vec![]).await.status, 415, "raw {id}");
        }
        for id in [&html, &svg] {
            let g = send(&app, &ip, "GET", &format!("/api/files/{id}/preview"), &boss, &[], vec![]).await;
            let page = String::from_utf8_lossy(&g.bytes).to_string();
            assert!(page.contains("data-kind=\"none\"") && !page.contains("alert(1)"), "{page}");
        }
        // Text is shown escaped.
        let page = String::from_utf8_lossy(&send(&app, &ip, "GET", &format!("/api/files/{txt}/preview"), &boss, &[], vec![]).await.bytes).to_string();
        assert!(page.contains("&lt;script&gt;x&lt;/script&gt;") && !page.contains("<script>"), "{page}");
        // A PDF is shown by the browser's viewer (no sandbox: it would block it), never sniffed.
        let g = send(&app, &ip, "GET", &format!("/api/files/{pdf}/preview"), &boss, &[], vec![]).await;
        assert_eq!((g.status, g.headers[header::CONTENT_TYPE].to_str().unwrap()), (200, "application/pdf"));
        assert_eq!(g.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        // Someone without access gets nothing.
        for url in [format!("/api/files/{photo}"), format!("/api/files/{photo}/preview"), format!("/api/files/{photo}/raw")] {
            assert_eq!(send(&app, &ip, "GET", &url, &ann, &[], vec![]).await.status, 403, "{url}");
        }
        assert_eq!(g.headers[header::CACHE_CONTROL], "no-store", "never cached: access can be taken away");
        // Replacing it keeps no versions, but the old content goes to the bin (restorable);
        // renaming keeps its type; moving too.
        assert_eq!(send(&app, &ip, "PUT", &format!("/api/files/{photo}"), &boss, &[], b"new".to_vec()).await.status, 200);
        assert_eq!(send(&app, &ip, "GET", &format!("/api/files/{photo}/versions"), &boss, &[], vec![]).await.json, json!([]));
        let bin = send(&app, &ip, "GET", "/api/bin", &boss, &[], vec![]).await.json;
        assert!(bin.as_array().unwrap().iter().any(|b| b["name"] == "photo.png" && b["original"] == "boss/photo.png"), "{bin}");
        assert_eq!(std::fs::read(app.store.full("boss/.bin/photo.png")).unwrap(), png, "the replaced content");
        assert_eq!(send(&app, &ip, "GET", &format!("/api/files/{photo}"), &boss, &[], vec![]).await.bytes, b"new");
        let g = send(&app, &ip, "POST", &format!("/api/files/{photo}/rename?name=Holiday"), &boss, &[], vec![]).await;
        assert_eq!(g.json["name"], "Holiday.png");
        let g = send(&app, &ip, "POST", &format!("/api/files/{photo}/rename?name=Trip.PNG"), &boss, &[], vec![]).await;
        assert_eq!(g.json["name"], "Trip.png");
        assert_eq!(send(&app, &ip, "POST", "/api/folders?path=boss&name=Pics", &boss, &[], vec![]).await.status, 201);
        let g = send(&app, &ip, "POST", &format!("/api/files/{photo}/move?folder=boss/Pics"), &boss, &[], vec![]).await;
        assert_eq!(g.json["name"], "Trip.png");
        assert!(app.store.full("boss/Pics/Trip.png").is_file());
        // Delete goes to the bin.
        assert_eq!(send(&app, &ip, "DELETE", &format!("/api/files/{exe}"), &boss, &[], vec![]).await.status, 200);
        // No way out: names and folders can't climb; links out of the store are neither listed nor read.
        let g = send(&app, &ip, "POST", &up("..%2F..%2Fescape.txt"), &boss, &[], b"x".to_vec()).await;
        assert_eq!((g.status, g.json["folder"].as_str()), (201, Some("boss")));
        assert!(!g.json["name"].as_str().unwrap().contains('/'));
        assert_eq!(send(&app, &ip, "POST", &format!("{}&folder=..%2Fx", up("a.txt")), &boss, &[], vec![]).await.status, 400);
        let outside = std::env::temp_dir().join(format!("starsheets-outside-{}", store::random_hex(4)));
        std::fs::write(&outside, b"secret").unwrap();
        std::os::unix::fs::symlink(&outside, app.store.full("boss/link.txt")).unwrap();
        // Links are never followed, even inside the store: not to StarSheets' own data, not into
        // someone else's folder.
        std::os::unix::fs::symlink(app.store.full("boss/read me.md"), app.store.full("boss/inside.md")).unwrap();
        std::os::unix::fs::symlink(app.store.meta().join("users.json"), app.store.full("boss/users.json")).unwrap();
        std::os::unix::fs::symlink(app.store.full("boss/Pics/Trip.png"), app.store.full("ann/boss-photo.png")).unwrap();
        std::os::unix::fs::symlink(app.store.full("boss/Pics"), app.store.full("ann/boss-pics")).unwrap();
        let names = send(&app, &ip, "GET", "/api/files", &boss, &[], vec![]).await.json.to_string();
        for n in ["link.txt", "inside.md", "users.json", "boss-photo.png", "boss-pics"] {
            assert!(!names.contains(n), "{n} listed: {names}");
        }
        let anns = send(&app, &ip, "GET", "/api/files", &ann, &[], vec![]).await.json.to_string();
        assert!(!anns.contains("boss-photo") && !anns.contains("Trip.png") && !anns.contains("boss-pics"), "{anns}");
        // Nothing can be put through a linked folder either.
        assert_eq!(send(&app, &ip, "POST", "/api/files?name=x.txt&as_is=1&folder=ann/boss-pics", &ann, &[], b"x".to_vec()).await.status, 404);
        assert!(!app.store.full("boss/Pics/x.txt").exists());
        // A listed file swapped for a link to StarSheets' own data afterwards isn't read.
        let md = std::fs::read(app.store.full("boss/read me.md")).unwrap();
        std::fs::remove_file(app.store.full("boss/read me.md")).unwrap();
        std::os::unix::fs::symlink(app.store.meta().join("users.json"), app.store.full("boss/read me.md")).unwrap();
        for url in [format!("/api/files/{txt}"), format!("/api/files/{txt}/preview"), format!("/api/files/{txt}/raw")] {
            let g = send(&app, &ip, "GET", &url, &boss, &[], vec![]).await;
            assert!(g.status == 404 && !String::from_utf8_lossy(&g.bytes).contains("hash"), "{url}: {}", g.status);
        }
        std::fs::remove_file(app.store.full("boss/read me.md")).unwrap();
        std::fs::write(app.store.full("boss/read me.md"), md).unwrap();
        // A listed file swapped for a link out of the store afterwards isn't read either.
        std::fs::remove_file(app.store.full("boss/read me.md")).unwrap();
        std::os::unix::fs::symlink(&outside, app.store.full("boss/read me.md")).unwrap();
        for url in [format!("/api/files/{txt}"), format!("/api/files/{txt}/preview")] {
            let g = send(&app, &ip, "GET", &url, &boss, &[], vec![]).await;
            assert!(g.status == 404 && !String::from_utf8_lossy(&g.bytes).contains("secret"), "{url}: {}", g.status);
        }
        let _ = std::fs::remove_file(&outside);
        let _ = std::fs::remove_dir_all(&app.store.root);
    }
}
