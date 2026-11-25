//! Photos over HTTP (docs/photos-api.md):
//!
//! - `GET /api/photos?folder=PATH` (or `folder_id=ID`; `sub=1` with every folder inside;
//!   `sort=taken|name|modified`, `desc=1`): the pictures in a folder, with their EXIF date.
//! - `GET /api/photos/thumb/ID/SIZE` (SIZE 256 or 1024, `?v=` the picture's `v`): a thumbnail.
//! - `POST /api/photos/enable?folder=PATH` (or `folder_id`): turns Photos on for a folder and
//!   everything in it (anyone who may edit the folder). Until then nothing in it is indexed.
//! - `POST /api/photos/disable?folder=PATH`: turns it off and forgets the folder's pictures and
//!   thumbnails (the folder's owner, or the server's).
//! - `GET /api/photos/status`: the index's state (the server's owner only).
//!
//! Who may: a folder's pictures, anyone who may view the folder; a thumbnail, anyone who may
//! view the file (the same checks as a download, sharing.rs). Where a picture was taken (GPS)
//! only for people who may edit the folder through their own account or a share naming them,
//! never through a link. Paths come from the store's own index and are never reached through a
//! symbolic link.
//!
//! Thumbnails are kept by the browser (`private, max-age=31536000, immutable`) only when asked
//! for with `?v=` equal to the picture's content tag: the address then names exactly these bytes,
//! so a changed picture gets a new address and never shows an old thumbnail. `private` keeps them
//! out of shared caches. Without `v`, `private, no-cache` with an ETag. A grid scrolling back and
//! forth then costs nothing, where `no-store` would fetch (and check access for) every thumbnail
//! again each time; someone whose access is taken away keeps only what their browser already
//! showed them, and gets nothing new.

use std::collections::HashMap;
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, Response, header};
use serde_json::{Value, json};

use super::index::State;
use super::{decode, media_of, service};
use crate::{Shared, auth, blocking, error, send_json, sharing, store};

/// How long a thumbnail request waits for its picture to be read before answering 202.
const WAIT: Duration = Duration::from_secs(3);

/// `Some(answer)` for `/api/photos…`, `None` for anything else.
pub async fn route(
    app: &Shared,
    m: &str,
    p: &[String],
    q: &HashMap<String, String>,
    me: Option<auth::User>,
    headers: &HeaderMap,
) -> Option<Response<Body>> {
    if p.first().map(String::as_str) != Some("photos") {
        return None;
    }
    let post = p.len() == 2 && (p[1] == "enable" || p[1] == "disable");
    if (m != "GET" && !post) || (m != "POST" && post) {
        return Some(error(405, if post { "use POST" } else { "use GET" }));
    }
    Some(match p.len() {
        1 => list(app, q, me).await,
        2 if post => switch(app, q, me, p[1] == "enable").await,
        2 if p[1] == "status" => status(app, me).await,
        4 if p[1] == "thumb" => thumb(app, me, p[2].clone(), &p[3], q.get("v").map(String::as_str), headers).await,
        _ => error(404, "not found"),
    })
}

/// A folder asked for by `folder` (path) or `folder_id`, checked: (path, my role there, whether
/// that role comes from my own space or a share naming me rather than a link).
fn folder_of(a: &crate::App, acc: &sharing::Access, q: &HashMap<String, String>) -> Result<(String, sharing::Role, bool), Response<Body>> {
    let folder = match (q.get("folder_id"), q.get("folder")) {
        (Some(id), _) => acc.folder_ids.iter().find(|(_, v)| *v == id).map(|(p, _)| p.clone()).ok_or_else(|| error(404, "no such folder"))?,
        (None, f) => store::rel(f.map(String::as_str)).map_err(|b| error(b.code, &b.msg))?,
    };
    let role = acc.folder_role(&folder);
    if role < sharing::Role::View {
        return Err(error(403, "You don't have access to that."));
    }
    if !a.store.no_links(&folder) || !std::fs::symlink_metadata(a.store.full(&folder)).is_ok_and(|m| m.is_dir()) {
        return Err(error(404, "no such folder"));
    }
    Ok((folder.clone(), role, acc.folder_listed(&folder)))
}

/// POST /api/photos/enable or /disable.
async fn switch(app: &Shared, q: &HashMap<String, String>, me: Option<auth::User>, on: bool) -> Response<Body> {
    let q = q.clone();
    let who = me.as_ref().map(|u| u.username.clone()).unwrap_or_default();
    let checked = blocking(app, move |a| -> Result<String, Response<Body>> {
        let acc = sharing::Access::new(&a.store, a.auth, me.as_ref());
        let (folder, role, _) = folder_of(a, &acc, &q)?;
        if on && role < sharing::Role::Edit {
            return Err(error(403, "Only people who can edit this folder can turn on Photos for it."));
        }
        if !on && role < sharing::Role::Owner {
            return Err(error(403, "Only the folder's owner can turn off Photos for it."));
        }
        Ok(folder)
    })
    .await
    .unwrap_or_else(|| Err(error(500, "server error")));
    let folder = match checked {
        Ok(f) => f,
        Err(r) => return r,
    };
    let a = app.clone();
    let f2 = folder.clone();
    let r = off(move || {
        let svc = service(&a.store).ok_or_else(|| "Photos aren't available on this server.".to_string())?;
        if on { svc.enable(&f2, &who).map(|()| true) } else { svc.disable(&f2) }
    })
    .await
    .unwrap_or_else(|| Err("server error".into()));
    match r {
        Ok(still) => send_json(&json!({ "folder": folder, "enabled": still }), 200),
        Err(e) => error(500, &e),
    }
}

/// Runs blocking work (the index, the disk) off the async threads, without the store lock.
async fn off<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(f).await.ok()
}

/// The first 16 hex digits of a content hash: the `v` of a picture.
fn tag(hash: &str) -> &str {
    &hash[..hash.len().min(16)]
}

async fn status(app: &Shared, me: Option<auth::User>) -> Response<Body> {
    if app.auth && !me.is_some_and(|u| u.owner) {
        return error(403, "Only the server's owner can see this.");
    }
    let a = app.clone();
    match off(move || service(&a.store).map(|s| s.status())).await.flatten() {
        Some(v) => send_json(&v, 200),
        None => error(503, "Photos aren't available on this server."),
    }
}

/// A folder checked for a listing: its path, the store's ids of the files in it (path → id), my
/// role there, and whether that role is mine by name (not through a link).
type Checked = (String, HashMap<String, String>, sharing::Role, bool);

async fn list(app: &Shared, q: &HashMap<String, String>, me: Option<auth::User>) -> Response<Body> {
    let deep = q.get("sub").is_some_and(|v| v == "1" || v == "true");
    let sort = q.get("sort").cloned().unwrap_or_else(|| "taken".into());
    let desc = q.get("desc").is_some_and(|v| v == "1" || v == "true");
    if !["taken", "name", "modified"].contains(&sort.as_str()) {
        return error(400, "sort is taken, name or modified");
    }
    let q2 = q.clone();
    // Who may see the folder, and the store's ids of the files in it (under the store lock).
    let checked = blocking(app, move |a| -> Result<Checked, Response<Body>> {
        let acc = sharing::Access::new(&a.store, a.auth, me.as_ref());
        let (folder, role, named) = folder_of(a, &acc, &q2)?;
        let inside = |p: &str| {
            let parent = store::parent_of(p);
            parent == folder || (deep && (folder.is_empty() || parent.starts_with(&format!("{folder}/"))))
        };
        let ids = acc.file_paths.iter().filter(|(_, p)| inside(p)).map(|(id, p)| (p.clone(), id.clone())).collect();
        Ok((folder, ids, role, named))
    })
    .await
    .unwrap_or_else(|| Err(error(500, "server error")));
    let (folder, ids, role, named) = match checked {
        Ok(c) => c,
        Err(r) => return r,
    };
    let a = app.clone();
    let f2 = folder.clone();
    let got = off(move || {
        let svc = service(&a.store)?;
        if !svc.covers(&f2) {
            return Some((false, Vec::new(), 0));
        }
        // Rescanned by the scanner thread; one folder is waited for (briefly), a whole tree not.
        if let Some(ticket) = svc.look_at(&f2, deep)
            && !deep
        {
            svc.wait_scan(ticket, Duration::from_millis(1500));
        }
        let (queued, working) = svc.queued();
        Some((true, svc.rows(&f2, deep), queued + working))
    })
    .await
    .flatten();
    let Some((enabled, rows, waiting)) = got else { return error(503, "Photos aren't available on this server.") };
    // Where pictures were taken: for people who may edit, through their account (not a link).
    let places = role >= sharing::Role::Edit && named;
    let mut items: Vec<(String, Value)> = rows
        .into_iter()
        .filter_map(|r| {
            let id = ids.get(&r.path)?;
            let modified = r.mtime_ns.div_euclid(1_000_000_000);
            let key = match sort.as_str() {
                "name" => r.name.to_lowercase(),
                "modified" => format!("{modified:020}"),
                _ => r.meta.taken.clone().map(|t| t.chars().take(19).collect()).unwrap_or_else(|| iso(modified)),
            };
            let ready = r.state == State::Ready;
            let camera = [r.meta.make.as_deref(), r.meta.model.as_deref()].into_iter().flatten().collect::<Vec<_>>().join(" ");
            let v = json!({
                "id": id,
                "pid": r.id,
                "name": r.name,
                "folder": r.folder,
                "media": r.media,
                "size": r.size,
                "modified": modified,
                "state": r.state.name(),
                "error": r.error,
                "v": if ready { r.hash.as_deref().map(tag) } else { None },
                "taken": r.meta.taken,
                "width": if ready { Some(r.meta.width) } else { None },
                "height": if ready { Some(r.meta.height) } else { None },
                "orientation": if ready { Some(r.meta.orientation) } else { None },
                "camera": (!camera.is_empty()).then_some(camera),
                "gps": r.meta.lat.zip(r.meta.lon).filter(|_| places).map(|(lat, lon)| json!({"lat": lat, "lon": lon, "alt": r.meta.alt})),
            });
            Some((format!("{key}\u{0}{}", r.path), v))
        })
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    if desc {
        items.reverse();
    }
    let photos: Vec<Value> = items.into_iter().map(|(_, v)| v).collect();
    send_json(
        &json!({
            "folder": folder, "sub": deep, "role": role.name(), "enabled": enabled,
            "can_enable": role >= sharing::Role::Edit, "can_disable": role >= sharing::Role::Owner,
            "photos": photos, "waiting": waiting,
        }),
        200,
    )
}

/// Seconds since 1970 as `YYYY-MM-DDTHH:MM:SS` (UTC), to sort pictures without an EXIF date.
fn iso(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// A thumbnail found: its bytes, "jpg"/"png", the picture's `v` and its file name.
type Found = (Vec<u8>, &'static str, String, String);

async fn thumb(app: &Shared, me: Option<auth::User>, id: String, size: &str, v: Option<&str>, headers: &HeaderMap) -> Response<Body> {
    let Some(size) = size.parse::<u32>().ok().filter(|s| decode::SIZES.contains(s)) else {
        return error(400, "size is 256 or 1024");
    };
    let path = blocking(app, move |a| -> Result<String, Response<Body>> {
        let acc = sharing::Access::new(&a.store, a.auth, me.as_ref());
        if acc.file_role(&id) < sharing::Role::View {
            return Err(error(403, "You don't have access to that."));
        }
        let path = acc.file_paths.get(&id).cloned().ok_or_else(|| error(404, "no such file"))?;
        a.store.safe_file(&path).ok_or_else(|| error(404, "no such file"))?;
        Ok(path)
    })
    .await
    .unwrap_or_else(|| Err(error(500, "server error")));
    let path = match path {
        Ok(p) => p,
        Err(r) => return r,
    };
    if media_of(&path).is_none() {
        return error(404, "not a picture");
    }
    let a = app.clone();
    let got = off(move || -> Result<Found, (u16, Value)> {
        let svc = service(&a.store).ok_or((503, json!({"error": "Photos aren't available on this server."})))?;
        if !svc.covers(&path) {
            return Err((404, json!({"error": "Photos isn't turned on for this folder.", "state": "off"})));
        }
        for tries in 0..2 {
            if let Some(r) = svc.row(&path).filter(|r| svc.fresh(r)) {
                match r.state {
                    State::Unsupported => {
                        return Err((404, json!({"error": "No thumbnails of this kind of picture yet.", "state": "unsupported"})));
                    }
                    State::Failed => return Err((404, json!({"error": r.error, "state": "failed"}))),
                    State::Ready => {
                        if let Some(hash) = &r.hash
                            && let Some((file, ext)) = svc.thumb_file(hash, size)
                            && let Ok(bytes) = std::fs::read(&file)
                        {
                            svc.used(&file);
                            return Ok((bytes, ext, tag(hash).to_string(), r.name));
                        }
                    }
                    State::Pending => {}
                }
            }
            // Not read yet, changed since, or its thumbnail was cleared: read it next.
            if tries == 1 {
                break;
            }
            if !svc.hurry(&path) {
                return Err((404, json!({"error": "No thumbnails of this kind of picture yet.", "state": "unsupported"})));
            }
            if !svc.wait_for(&path, WAIT) {
                break;
            }
        }
        Err((202, json!({"state": "pending", "retry": 2})))
    })
    .await
    .unwrap_or(Err((500, json!({"error": "server error"}))));
    let (bytes, ext, vtag, name) = match got {
        Ok(g) => g,
        Err((202, body)) => {
            let mut r = send_json(&body, 202);
            r.headers_mut().insert(header::RETRY_AFTER, header::HeaderValue::from_static("2"));
            return r;
        }
        Err((code, body)) => return send_json(&body, code),
    };
    let etag = format!("\"{vtag}-{size}\"");
    let cache = if v == Some(vtag.as_str()) { "private, max-age=31536000, immutable" } else { "private, no-cache" };
    let b = Response::builder()
        .header(header::ETAG, &etag)
        .header(header::CACHE_CONTROL, cache)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CONTENT_SECURITY_POLICY, crate::preview::SANDBOX);
    if headers.get(header::IF_NONE_MATCH).and_then(|h| h.to_str().ok()) == Some(etag.as_str()) {
        return b.status(304).body(Body::empty()).unwrap();
    }
    let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s);
    b.status(200)
        .header(header::CONTENT_TYPE, if ext == "png" { "image/png" } else { "image/jpeg" })
        .header(header::CONTENT_DISPOSITION, crate::preview::disposition("inline", &format!("{stem}-{size}.{ext}")))
        .body(Body::from(bytes))
        .unwrap()
}
