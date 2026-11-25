//! A picture's edits over HTTP (docs/photos-api.md, "Edits"; photo-editor contract §3):
//!
//! - `GET /api/photos/edits/ID`: `{"edits", "etag", "writable", "reason", "foreign", "source",
//!   "thumb_v"}` (anyone who may view the picture; no change when there is no sidecar).
//! - `PUT /api/photos/edits/ID` with `If-Match: <etag>`, the body the edits (JSON, at most
//!   16 KB): merges them into the picture's XMP sidecar. `{"etag", "thumb_v"}`; 409 with the
//!   current `etag` when the sidecar changed since that etag (someone else saved first).
//! - `DELETE /api/photos/edits/ID` with `If-Match`: takes our edits out of the sidecar, keeping
//!   everything else in it; the sidecar is deleted when nothing else is left.
//!
//! Who may change them: whoever may edit the file (as for a save, main.rs `gate`). Reading,
//! checking the etag and writing happen under the store lock, so two saves can't both pass the
//! check; the folder is opened once without links and the sidecar written relative to it
//! (sidecar.rs). Where nothing can be written (read-only storage, a sidecar that is a link or
//! isn't XMP) GET says `writable: false` with the `reason`, and PUT answers 403 or 409 with it.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Response, header};
use serde_json::{Value, json};

use super::index::{self, Side};
use super::model::Edits;
use super::sidecar::{self, Merged, Source};
use super::{media_of, service};
use crate::{Shared, auth, blocking, error, send_json, sharing, store};

/// The biggest edits body.
pub const MAX_BODY: usize = 16 * 1024;

/// A request's answer before the index is told: (reply, picture path, the sidecar after a save).
type Answer = (Value, String, Option<Side>);

pub async fn route(app: &Shared, m: &str, id: String, me: Option<auth::User>, headers: &HeaderMap, body: &[u8]) -> Response<Body> {
    let write = match m {
        "GET" => false,
        "PUT" | "DELETE" => true,
        _ => return error(405, "use GET, PUT or DELETE"),
    };
    let new: Option<Edits> = if m == "PUT" {
        if body.len() > MAX_BODY {
            return error(413, "The edits are too big (16 KB at most).");
        }
        match serde_json::from_slice::<Edits>(body) {
            Ok(e) => Some(e),
            Err(e) => return error(400, &format!("These aren't valid edits: {e}")),
        }
    } else {
        None
    };
    let if_match = headers.get(header::IF_MATCH).and_then(|v| v.to_str().ok()).map(|v| v.trim().trim_matches('"').to_string());
    if write && if_match.is_none() {
        return error(428, "Send If-Match with the etag the edits were read with.");
    }
    let done = blocking(app, move |a| -> Result<Answer, Response<Body>> {
        let acc = sharing::Access::new(&a.store, a.auth, me.as_ref());
        let role = acc.file_role(&id);
        if role < sharing::Role::View {
            return Err(error(403, "You don't have access to that."));
        }
        let path = acc.file_paths.get(&id).cloned().ok_or_else(|| error(404, "no such file"))?;
        a.store.safe_file(&path).ok_or_else(|| error(404, "no such file"))?;
        let Some((_, supported)) = media_of(&path) else { return Err(error(404, "not a picture")) };
        let (folder, name) = (store::parent_of(&path), store::base_of(&path));
        let side_name = sidecar::name_for(&name);
        let dir = sidecar::Folder::open(&a.store, &folder).ok_or_else(|| error(404, "no such file"))?;
        let disk = dir.read(&side_name);
        let current = disk.as_ref().ok().and_then(Option::as_ref);
        let bytes: Option<&[u8]> = current.map(|d| d.bytes.as_slice());
        let parsed = bytes.map(sidecar::read);
        let etag = sidecar::etag(bytes);
        // Why this person can't save edits here, if so (in this order); `forbidden` = 403, else 409.
        let read_only = dir.read_only();
        let forbidden = role < sharing::Role::Edit || !supported || read_only.is_some();
        let reason: Option<String> = if role < sharing::Role::Edit {
            Some("You can view this picture but not edit it.".into())
        } else if !supported {
            Some("Starbase can't edit this kind of picture yet.".into())
        } else if let Some(why) = read_only {
            Some(why.into())
        } else if let Err(e) = &disk {
            Some(e.say(&side_name))
        } else if let Some(Err(e)) = &parsed {
            Some(format!("Its sidecar ({side_name}) isn't XMP that Starbase can read ({e}), so Starbase won't change it."))
        } else {
            None
        };
        if !write {
            let p = parsed.and_then(Result::ok);
            let source = match p.as_ref().map(|p| p.source) {
                Some(Source::Starbase) => "starbase",
                Some(Source::Crs) => "crs",
                _ => "none",
            };
            let (edits, foreign) = p.map(|p| (p.edits, p.foreign)).unwrap_or_default();
            let reply = json!({
                "edits": edits, "etag": etag, "writable": reason.is_none(), "reason": reason,
                "foreign": foreign, "source": source,
            });
            return Ok((reply, path, None));
        }
        if let Some(why) = reason {
            return Err(send_json(&json!({"error": why, "etag": etag}), if forbidden { 403 } else { 409 }));
        }
        let wanted = if_match.unwrap_or_default();
        if wanted != "*" && wanted != etag {
            return Err(send_json(&json!({"error": "The edits were changed somewhere else since you opened them.", "etag": etag}), 409));
        }
        let merged = sidecar::merge(bytes, new.as_ref())
            .map_err(|e| error(409, &format!("Its sidecar ({side_name}) couldn't be merged ({e}), so it wasn't changed.")))?;
        let (now, stat): (Option<Vec<u8>>, Option<String>) = match merged {
            // The same edits again: nothing to write.
            Merged::Write(out) if Some(out.as_slice()) == bytes => (Some(out), current.map(|d| d.stat.clone())),
            Merged::Write(out) => {
                let stat = dir.write(&side_name, &out).map_err(|e| error(500, &format!("The sidecar couldn't be saved: {e}")))?;
                (Some(out), Some(stat))
            }
            Merged::Delete => {
                dir.remove(&side_name).map_err(|e| error(500, &format!("The sidecar couldn't be removed: {e}")))?;
                (None, None)
            }
            Merged::Same => (bytes.map(<[u8]>::to_vec), current.map(|d| d.stat.clone())),
        };
        let edits_now = now.as_deref().and_then(|b| sidecar::read(b).ok()).map(|p| p.edits).unwrap_or_default();
        let side = Side { edits: sidecar::edits_hash(&edits_now), stat };
        Ok((json!({"etag": sidecar::etag(now.as_deref())}), path, Some(side)))
    })
    .await
    .unwrap_or_else(|| Err(error(500, "server error")));
    let (mut reply, path, side) = match done {
        Ok(d) => d,
        Err(r) => return r,
    };
    // The index (the thumbnails' key) learns of the new edits outside the store lock.
    let app2 = app.clone();
    let thumb_v = tokio::task::spawn_blocking(move || {
        let svc = service(&app2.store)?;
        if !svc.covers(&path) {
            return None;
        }
        match side {
            Some(side) => svc.edits_saved(&path, &side),
            None => svc.row(&path).filter(|r| r.state == index::State::Ready).and_then(|r| r.thumb_key().map(|k| super::routes::tag(k).to_string())),
        }
    })
    .await
    .ok()
    .flatten();
    reply["thumb_v"] = json!(thumb_v);
    let etag = reply["etag"].as_str().unwrap_or("").to_string();
    let mut r = send_json(&reply, 200);
    if let Ok(v) = HeaderValue::from_str(&format!("\"{etag}\"")) {
        r.headers_mut().insert(header::ETAG, v);
    }
    r
}
