//! Public links to one file: anyone with the address may open it without an account. A link can
//! expire on a date, be view-only (no download), and be protected by a password. Kept in
//! `publiclinks.json`, one entry per token; a file has at most one link. Sharing's own "anyone
//! signed in with the link" setting (sharing.rs) is separate.

use crate::store::{Store, now, random_hex};
use crate::{auth, preview};
use serde_json::{Map, Value, json};

fn path(store: &Store) -> std::path::PathBuf {
    store.meta().join("publiclinks.json")
}

/// Seconds since 1970 at the end of the given UTC day ("YYYY-MM-DD").
pub fn end_of_day(date: &str) -> Option<f64> {
    let mut it = date.trim().split('-');
    let (y, m, d): (i64, i64, i64) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    if it.next().is_some() || !(1970..=9999).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil (Howard Hinnant's algorithm).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(((days + 1) * 86400) as f64)
}

fn date_of(secs: f64) -> String {
    let days = ((secs - 1.0) / 86400.0).floor() as i64;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

fn load(store: &Store) -> Map<String, Value> {
    store.load_json(&path(store))
}

/// The link of file `id`: (token, entry).
fn find(all: &Map<String, Value>, id: &str) -> Option<(String, Value)> {
    all.iter().find(|(_, v)| v["file"] == id).map(|(t, v)| (t.clone(), v.clone()))
}

/// The link settings as the Share dialog shows them.
pub fn describe(store: &Store, id: &str) -> Value {
    match find(&load(store), id) {
        None => json!({ "on": false }),
        Some((token, v)) => json!({
            "on": true,
            "token": token,
            "expires": v["expires"].as_f64().map(date_of).unwrap_or_default(),
            "no_download": v["no_download"].as_bool().unwrap_or(false),
            "has_password": v["password"].as_str().is_some(),
        }),
    }
}

/// Applies `body.public`: `{on, expires: "YYYY-MM-DD" or "", no_download, password}` (a password
/// that is absent keeps the old one, an empty one removes it).
pub fn update(store: &Store, id: &str, body: &Value) -> Result<(), String> {
    let Some(p) = body.get("public").and_then(Value::as_object) else { return Ok(()) };
    let mut all = load(store);
    let old = find(&all, id);
    if !p.get("on").and_then(Value::as_bool).unwrap_or(false) {
        if let Some((t, _)) = old {
            all.remove(&t);
        }
        return store.save_json(&path(store), &Value::Object(all)).map_err(|e| e.to_string());
    }
    let (token, mut entry) = old.unwrap_or_else(|| (random_hex(16), json!({ "file": id })));
    match p.get("expires").and_then(Value::as_str) {
        Some("") => entry["expires"] = Value::Null,
        Some(d) => entry["expires"] = json!(end_of_day(d).ok_or("The date should look like 2027-01-31.")?),
        None => {}
    }
    if let Some(n) = p.get("no_download").and_then(Value::as_bool) {
        entry["no_download"] = json!(n);
    }
    match p.get("password").and_then(Value::as_str) {
        Some("") => entry["password"] = Value::Null,
        Some(pw) => entry["password"] = json!(auth::hash_new(pw)?),
        None => {}
    }
    if p.get("password").is_some() {
        entry["grants"] = Value::Null; // a new or removed password ends everyone's unlock
    }
    all.insert(token, entry);
    store.save_json(&path(store), &Value::Object(all)).map_err(|e| e.to_string())
}

pub struct Reply {
    /// The password page needs its form to work: the page's CSP then allows forms.
    pub form: bool,
    pub set_cookie: Option<String>,
    pub location: Option<String>,
    pub code: u16,
    pub content_type: &'static str,
    pub disposition: Option<String>,
    pub body: Vec<u8>,
}

fn page(code: u16, title: &str, inner: &str) -> Reply {
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>{t}</title>\
         <style>body{{font:16px system-ui,sans-serif;max-width:48rem;margin:3rem auto;padding:0 1rem;color:#202124}}\
         a.b,button{{display:inline-block;padding:.5rem 1rem;border:0;border-radius:.4rem;background:#1a73e8;color:#fff;text-decoration:none;font:inherit;cursor:pointer}}\
         iframe,img{{max-width:100%}}iframe{{width:100%;height:70vh;border:1px solid #ddd}}</style><h1>{t}</h1>{inner}",
        t = preview::esc(title)
    );
    Reply {
        form: false,
        set_cookie: None,
        location: None,
        code,
        content_type: "text/html; charset=utf-8",
        disposition: None,
        body: html.into_bytes(),
    }
}

/// How long a typed password keeps a link open in that browser.
const UNLOCK_SECS: f64 = 3600.0;

fn cookie_name(token: &str) -> String {
    format!("pl_{token}")
}

fn password_page(token: &str, wrong: bool) -> Reply {
    let note = if wrong { "<p>That password isn't right.</p>" } else { "" };
    let mut r = page(
        if wrong { 401 } else { 200 },
        "Password needed",
        &format!("{note}<form method=post action=\"/api/public/{token}\"><input type=password name=pw autofocus> <button>Open</button></form>"),
    );
    r.form = true;
    r
}

/// A password typed on the link's page (POST): right -> a short-lived cookie for this link's path
/// only and back to the link; wrong -> the page again. The password never sits in an address.
pub fn unlock(store: &Store, token: &str, password: &str) -> Reply {
    let mut all = load(store);
    let Some(entry) = all.get_mut(token) else { return page(404, "Link not found", "<p>This link doesn't exist or was turned off.</p>") };
    if entry["expires"].as_f64().is_some_and(|e| now() >= e) {
        return page(410, "This link has expired", "<p>Ask the person who shared it for a new one.</p>");
    }
    let Some(hash) = entry["password"].as_str() else { return redirect(token, None) };
    if !auth::verify(password, Some(hash)) {
        return password_page(token, true);
    }
    let grant = random_hex(16);
    let t = now();
    let mut grants: Map<String, Value> = entry["grants"].as_object().cloned().unwrap_or_default();
    grants.retain(|_, v| v.as_f64().is_some_and(|e| e > t));
    grants.insert(grant.clone(), json!(t + UNLOCK_SECS));
    entry["grants"] = Value::Object(grants);
    let _ = store.save_json(&path(store), &Value::Object(all));
    redirect(token, Some(format!("{}={grant}; Path=/api/public/{token}; Max-Age={}; HttpOnly; SameSite=Lax", cookie_name(token), UNLOCK_SECS as u64)))
}

fn redirect(token: &str, cookie: Option<String>) -> Reply {
    let mut r = page(303, "Opening", "");
    r.location = Some(format!("/api/public/{token}"));
    r.set_cookie = cookie;
    r
}

/// The unlock cookie's value in a request's `Cookie` header, if any.
pub fn grant_in(cookies: &str, token: &str) -> String {
    let name = format!("{}=", cookie_name(token));
    cookies.split(';').find_map(|p| p.trim().strip_prefix(name.as_str()).map(str::to_string)).unwrap_or_default()
}

/// What may be shown inline without running anything of the file's own.
fn viewable(name: &str) -> Option<&'static str> {
    match name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).as_deref() {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("pdf") => Some("application/pdf"),
        Some("txt" | "csv" | "md") => Some("text/plain; charset=utf-8"),
        _ => None,
    }
}

/// Serves `/api/public/TOKEN[/view|/download]`; `grant` is the unlock cookie's value (see `unlock`).
pub fn serve(store: &Store, token: &str, what: &str, grant: &str) -> Reply {
    let all = load(store);
    let Some(entry) = all.get(token) else { return page(404, "Link not found", "<p>This link doesn't exist or was turned off.</p>") };
    if entry["expires"].as_f64().is_some_and(|e| now() >= e) {
        return page(410, "This link has expired", "<p>Ask the person who shared it for a new one.</p>");
    }
    let id = entry["file"].as_str().unwrap_or_default();
    if entry["password"].as_str().is_some() {
        let ok = !grant.is_empty() && entry["grants"][grant].as_f64().is_some_and(|e| e > now());
        if !ok {
            return password_page(token, false);
        }
    }
    let Some(rel) = store.path_of(id).ok().flatten() else { return page(404, "File not found", "<p>That file is gone.</p>") };
    let name = rel.rsplit('/').next().unwrap_or("file").to_string();
    let no_download = entry["no_download"].as_bool().unwrap_or(false);
    let base = format!("/api/public/{token}");
    match what {
        "" => {
            let shown = match viewable(&name) {
                Some(t) if t.starts_with("image/") => format!("<p><img alt=\"\" src=\"{base}/view\"></p>"),
                Some(_) => format!("<p><iframe sandbox src=\"{base}/view\"></iframe></p>"),
                None if no_download => "<p>This file can't be shown here, and downloading is turned off for this link.</p>".to_string(),
                None => String::new(),
            };
            let button = if no_download { String::new() } else { format!("<p><a class=b href=\"{base}/download\">Download</a></p>") };
            page(200, &name, &format!("{shown}{button}"))
        }
        "view" | "download" => {
            if what == "download" && no_download {
                return page(403, "Download is turned off", "<p>This link is view only.</p>");
            }
            let mime = viewable(&name);
            if what == "view" && mime.is_none() {
                return page(415, "No preview", "<p>This kind of file can't be shown here.</p>");
            }
            let Some(mut f) = store.open_safe(&rel) else { return page(404, "File not found", "<p>That file is gone.</p>") };
            let mut data = Vec::new();
            if std::io::Read::read_to_end(&mut f, &mut data).is_err() {
                return page(500, "Couldn't read the file", "");
            }
            if what == "view" {
                Reply {
                    form: false,
                    set_cookie: None,
                    location: None,
                    code: 200,
                    content_type: mime.unwrap_or("application/octet-stream"),
                    disposition: Some("inline".into()),
                    body: data,
                }
            } else {
                let safe: String = name.chars().filter(|c| !c.is_control() && !matches!(c, '"' | '\\' | '/')).collect();
                Reply {
                    form: false,
                    set_cookie: None,
                    location: None,
                    code: 200,
                    content_type: "application/octet-stream",
                    disposition: Some(format!("attachment; filename=\"{safe}\"")),
                    body: data,
                }
            }
        }
        _ => page(404, "Not found", ""),
    }
}
