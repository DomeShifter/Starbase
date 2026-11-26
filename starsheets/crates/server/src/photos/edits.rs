//! A picture's edits over HTTP (docs/photos-api.md, "Edits"; photo-editor contract §3):
//!
//! - `GET /api/photos/edits/ID`: `{"edits", "etag", "writable", "reason", "foreign", "source",
//!   "thumb_v"}` (anyone who may view the picture; no change when there is no sidecar).
//! - `PUT /api/photos/edits/ID` with `If-Match: <etag>`, the body the edits (JSON, at most
//!   16 KB, refused before it is read when bigger): merges them into the picture's XMP sidecar.
//!   `{"etag", "thumb_v"}`; 409 with the current `etag` when the sidecar changed since that etag
//!   (someone else saved first). `If-Match: *` = whatever is there, but there must be a sidecar.
//! - `DELETE /api/photos/edits/ID` with `If-Match`: takes our edits out of the sidecar, keeping
//!   everything else in it; the sidecar is deleted when nothing else is left. `removed` says
//!   whose edits went: `"crs"` = edits another app (Lightroom) wrote in Adobe's fields.
//!
//! - `POST /api/photos/rotate/ID` with `{"dir": "left" | "right"}`: turns the picture a quarter
//!   (reads its edits, turns them, saves them like the PUT above; the reply is the PUT's, so
//!   `thumb_v` says the thumbnails changed). The Photos grid and viewer's Rotate buttons, and
//!   their undo, use it.
//!
//! Who may change them: whoever may edit the picture file itself (a file shared for editing
//! alone counts: the sidecar is part of the picture). In three steps, so the server's store lock
//! is never held while untrusted XML is parsed: (1) under the lock, the access checks and the
//! sidecar's bytes (at most 256 KB, no parsing); (2) without it, parsing and merging; (3) under
//! the lock again, the checks once more and the write, only if the sidecar is still the one read
//! (else 409); the write itself checks again right before its rename (another app's save). The
//! folder is opened once without links and the sidecar written relative to it (sidecar.rs).
//! Where nothing can be written (read-only storage, a sidecar that is a link or isn't XMP) GET
//! says `writable: false` with the `reason`, and PUT answers 403 or 409 with it.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Response, header};
use serde_json::{Value, json};

use super::index::{self, Side};
use super::model::Edits;
use super::sidecar::{self, Folder, Geo, Merged, OnDisk, SideError, Source, WriteError};
use super::{decode, media_of, service};
use crate::{App, Shared, auth, blocking, error, send_json, sharing, store};

/// The biggest edits body.
pub const MAX_BODY: usize = 16 * 1024;

/// The picture and its sidecar as found under the store lock (step 1 or 3).
struct Look {
    path: String,
    /// The person may edit the picture.
    may_edit: bool,
    supported: bool,
    side_name: String,
    dir: Folder,
    disk: Result<Option<OnDisk>, SideError>,
    read_only: Option<&'static str>,
    /// The picture, opened without following links (its size and orientation are read from it).
    file: Option<std::fs::File>,
}

impl Look {
    fn bytes(&self) -> Option<&[u8]> {
        self.disk.as_ref().ok().and_then(Option::as_ref).map(|d| d.bytes.as_slice())
    }
    fn stat(&self) -> Option<&str> {
        self.disk.as_ref().ok().and_then(Option::as_ref).map(|d| d.stat.as_str())
    }
    fn etag(&self) -> String {
        sidecar::etag(self.bytes())
    }
}

fn look(a: &App, me: Option<&auth::User>, id: &str) -> Result<Look, Response<Body>> {
    let acc = sharing::Access::new(&a.store, a.auth, me);
    let role = acc.file_role(id);
    if role < sharing::Role::View {
        return Err(error(403, "You don't have access to that."));
    }
    let path = acc.file_paths.get(id).cloned().ok_or_else(|| error(404, "no such file"))?;
    a.store.safe_file(&path).ok_or_else(|| error(404, "no such file"))?;
    let Some((_, supported)) = media_of(&path) else { return Err(error(404, "not a picture")) };
    let (folder, name) = (store::parent_of(&path), store::base_of(&path));
    let side_name = sidecar::name_for(&name);
    let dir = Folder::open(&a.store, &folder).ok_or_else(|| error(404, "no such file"))?;
    let disk = dir.read(&side_name);
    let read_only = dir.read_only();
    let file = a.store.open_safe(&path);
    Ok(Look { path, may_edit: role >= sharing::Role::Edit, supported, side_name, dir, disk, read_only, file })
}

/// The bytes read from the start of a picture for its size and EXIF orientation.
const HEAD: u64 = 1024 * 1024;

/// The picture's stored size and EXIF orientation, from its first bytes (headers only, no
/// decoding): what Adobe's crop and orientation fields are relative to.
pub(super) fn geo_of(f: Option<&std::fs::File>) -> Option<Geo> {
    use std::io::Read;
    let mut head = Vec::new();
    f?.take(HEAD).read_to_end(&mut head).ok()?;
    let format = decode::sniff(&head)?;
    let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(&head), format).into_dimensions().ok()?;
    let orientation = if format == image::ImageFormat::Gif { 1 } else { decode::exif_meta(&head).orientation };
    Some(Geo { width, height, orientation })
}

/// What a request does once its sidecar is read and parsed (step 2).
enum Plan {
    /// GET: the answer.
    Reply(Value),
    /// PUT/DELETE: write these bytes (None = delete the sidecar), then the sidecar's edits.
    Write { out: Option<Vec<u8>>, edits: Edits, removed: &'static str },
    /// Nothing to change (the same edits again, or a reset with nothing of ours in it).
    Same { removed: &'static str },
}

fn source_name(s: Option<Source>) -> &'static str {
    match s {
        Some(Source::Starbase) => "starbase",
        Some(Source::Crs) => "crs",
        _ => "none",
    }
}

/// Step 2, without the store lock: parse, decide, merge.
fn plan(l: &Look, m: &str, new: Option<&Edits>, if_match: Option<&str>) -> Result<Plan, Response<Body>> {
    let bytes = l.bytes();
    let geo = if l.supported { geo_of(l.file.as_ref()) } else { None };
    let parsed = bytes.map(|b| sidecar::read_for(b, geo.as_ref()));
    let etag = l.etag();
    // Why this person can't save edits here, if so (in this order); `forbidden` = 403, else 409.
    let forbidden = !l.may_edit || !l.supported || l.read_only.is_some();
    let reason: Option<String> = if !l.may_edit {
        Some("You can view this picture but not edit it.".into())
    } else if !l.supported {
        Some("Starbase can't edit this kind of picture yet.".into())
    } else if let Some(why) = l.read_only {
        Some(why.into())
    } else if let Err(e) = &l.disk {
        Some(e.say(&l.side_name))
    } else if let Some(Err(e)) = &parsed {
        Some(format!("Its sidecar ({}) isn't XMP that Starbase can read ({e}), so Starbase won't change it.", l.side_name))
    } else if parsed.as_ref().is_some_and(|p| p.as_ref().is_ok_and(|p| p.newer)) {
        Some(format!("Its edits were saved by a newer version of Starbase ({}), so this one won't change them.", l.side_name))
    } else if geo.is_none() {
        Some("Starbase couldn't read this picture's size, so it can't save edits for it.".into())
    } else {
        None
    };
    let p = parsed.and_then(Result::ok);
    let source = p.as_ref().map(|p| p.source);
    if m == "GET" {
        let (edits, foreign) = p.map(|p| (p.edits, p.foreign)).unwrap_or_default();
        return Ok(Plan::Reply(json!({
            "edits": edits, "etag": etag, "writable": reason.is_none(), "reason": reason,
            "foreign": foreign, "source": source_name(source),
        })));
    }
    if let Some(why) = reason {
        return Err(send_json(&json!({"error": why, "etag": etag}), if forbidden { 403 } else { 409 }));
    }
    let wanted = if_match.unwrap_or_default();
    if (wanted == "*" && bytes.is_none()) || (wanted != "*" && wanted != etag) {
        return Err(send_json(&json!({"error": "The edits were changed somewhere else since you opened them.", "etag": etag}), 409));
    }
    let removed = if m == "DELETE" { source_name(source) } else { "none" };
    let Some(geo) = geo else { return Err(error(409, "Starbase couldn't read this picture's size.")) };
    let merged = sidecar::merge(bytes, new, &geo)
        .map_err(|e| error(409, &format!("Its sidecar ({}) couldn't be merged ({e}), so it wasn't changed.", l.side_name)))?;
    Ok(match merged {
        Merged::Write(out) if Some(out.as_slice()) == bytes => Plan::Same { removed },
        Merged::Write(out) => {
            let edits = sidecar::read(&out).map(|p| p.edits).unwrap_or_default();
            Plan::Write { out: Some(out), edits, removed }
        }
        Merged::Delete => Plan::Write { out: None, edits: Edits::default(), removed },
        Merged::Same => Plan::Same { removed },
    })
}

pub async fn route(app: &Shared, m: &str, id: String, me: Option<auth::User>, headers: &HeaderMap, body: &[u8]) -> Response<Body> {
    let m = match m {
        "GET" => "GET",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
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
    if m != "GET" && if_match.is_none() {
        return error(428, "Send If-Match with the etag the edits were read with.");
    }
    // 1. Under the store lock: who may do what, and the sidecar's bytes (no parsing).
    let (me2, id2) = (me.clone(), id.clone());
    let first = match blocking(app, move |a| look(a, me2.as_ref(), &id2)).await {
        Some(Ok(l)) => l,
        Some(Err(r)) => return r,
        None => return error(500, "server error"),
    };
    // 2. Without the lock: parse and merge.
    let planned = tokio::task::spawn_blocking(move || {
        let p = plan(&first, m, new.as_ref(), if_match.as_deref());
        (first, p)
    })
    .await;
    let Ok((first, planned)) = planned else { return error(500, "server error") };
    let planned = match planned {
        Ok(p) => p,
        Err(r) => return r,
    };
    // 3. Under the lock again: still allowed, still the sidecar read, then the write.
    let done = match planned {
        Plan::Reply(reply) => Ok((reply, first.path, None)),
        Plan::Same { removed } => Ok((json!({"etag": first.etag(), "removed": removed}), first.path.clone(), None)),
        Plan::Write { out, edits, removed } => {
            let was = (first.path.clone(), first.stat().map(str::to_string));
            blocking(app, move |a| -> Result<(Value, String, Option<Side>), Response<Body>> {
                let l = look(a, me.as_ref(), &id)?;
                if !l.may_edit || l.read_only.is_some() {
                    return Err(error(403, "You can no longer edit this picture."));
                }
                let changed = || send_json(&json!({"error": "The edits were changed somewhere else since you opened them.", "etag": l.etag()}), 409);
                if l.path != was.0 || l.disk.is_err() || l.stat().map(str::to_string) != was.1 {
                    return Err(changed());
                }
                let fail = |e: WriteError, what: &str| match e {
                    WriteError::Changed => changed(),
                    WriteError::Io(e) => error(500, &format!("The sidecar couldn't be {what}: {e}")),
                };
                let stat = match &out {
                    Some(b) => Some(l.dir.write(&l.side_name, b, was.1.as_deref()).map_err(|e| fail(e, "saved"))?),
                    None => {
                        if let Some(s) = &was.1 {
                            l.dir.remove(&l.side_name, s).map_err(|e| fail(e, "removed"))?;
                        }
                        None
                    }
                };
                let side = Side { edits: sidecar::edits_hash(&edits), stat };
                Ok((json!({"etag": sidecar::etag(out.as_deref()), "removed": removed}), l.path, Some(side)))
            })
            .await
            .unwrap_or_else(|| Err(error(500, "server error")))
        }
    };
    let (mut reply, path, side) = match done {
        Ok(d) => d,
        Err(r) => return r,
    };
    if m == "PUT"
        && let Some(o) = reply.as_object_mut()
    {
        o.remove("removed");
    }
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

/// `POST /api/photos/rotate/ID`: a quarter turn saved as an edit (GET, turn, PUT; same checks).
pub async fn rotate(app: &Shared, m: &str, id: String, me: Option<auth::User>, body: &[u8]) -> Response<Body> {
    if m != "POST" {
        return error(405, "use POST");
    }
    let left = match serde_json::from_slice::<Value>(body).ok().and_then(|v| v["dir"].as_str().map(str::to_string)).as_deref() {
        Some("left") => true,
        Some("right") => false,
        _ => return error(400, "Send {\"dir\": \"left\"} or {\"dir\": \"right\"}."),
    };
    let got = route(app, "GET", id.clone(), me.clone(), &HeaderMap::new(), b"").await;
    if got.status() != 200 {
        return got;
    }
    let Ok(bytes) = axum::body::to_bytes(got.into_body(), MAX_BODY * 8).await else { return error(500, "server error") };
    let v: Value = serde_json::from_slice(&bytes).unwrap_or_default();
    if v["writable"].as_bool() != Some(true) {
        return error(403, v["reason"].as_str().unwrap_or("This picture's folder can't be changed."));
    }
    let Ok(edits) = serde_json::from_value::<Edits>(v["edits"].clone()) else { return error(409, "This picture's saved edits couldn't be read.") };
    let Ok(tag) = HeaderValue::from_str(v["etag"].as_str().unwrap_or("none")) else { return error(500, "server error") };
    let mut h = HeaderMap::new();
    h.insert(header::IF_MATCH, tag);
    let new = serde_json::to_vec(&edits.turned(left)).unwrap_or_default();
    route(app, "PUT", id, me, &h, &new).await
}
