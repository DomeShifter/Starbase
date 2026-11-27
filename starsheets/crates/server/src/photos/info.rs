//! A picture's details for the viewer's Info panel (docs/photos-api.md, "Info"):
//!
//! - `GET /api/photos/info/ID`: `{"exif", "info", "keywords", "folder", "etag", "writable",
//!   "reason"}` for anyone who may view the picture. `exif` is read from the picture's first
//!   megabyte on each request (camera, lens, ISO, exposure, aperture, focal length, time zone,
//!   altitude, unique id, artist, copyright, description; the position only for people who may
//!   edit, like the listing); `info` is what people typed (title, caption, labels, subject,
//!   copyright, artist, licence) from the sidecar; `keywords` are made from both.
//! - `PUT /api/photos/info/ID` with `If-Match: <etag>` (the sidecar's, shared with the edits API)
//!   and the whole `info` (JSON, at most 16 KB): saves it in the picture's XMP sidecar, never in
//!   the picture. Who may: whoever may edit the picture. `{"etag"}`; 409 with the current etag
//!   when the sidecar changed since.
//!
//! The same three steps as the edits API (edits.rs): checks and bytes under the store lock,
//! parsing without it, then the checks again and the write.

use axum::body::Body;
use axum::http::{HeaderMap, Response, header};
use serde_json::{Value, json};

use super::decode::{self, Details};
use super::edits::{Look, look};
use super::index::Side;
use super::service;
use super::sidecar::{self, Info, Merged, WriteError};
use crate::{Shared, auth, blocking, error, send_json, store};

/// The biggest Info body.
pub const MAX_BODY: usize = 16 * 1024;

/// The bytes read from the start of a picture for its EXIF.
const HEAD: u64 = 1024 * 1024;

fn details_of(f: Option<&std::fs::File>) -> Details {
    use std::io::Read;
    let mut head = Vec::new();
    if let Some(mut f) = f
        && f.by_ref().take(HEAD).read_to_end(&mut head).is_err()
    {
        return Details::default();
    }
    decode::exif_details(&head)
}

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// Search words for a picture: its folders, camera, lens, year and month, then its labels and
/// subject (no repeats, in that order).
pub fn keywords(path: &str, d: &Details, i: &Info) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |s: &str| {
        let s = s.trim();
        if !s.is_empty() && s.chars().count() <= 60 && !out.iter().any(|k| k.eq_ignore_ascii_case(s)) {
            out.push(s.to_string());
        }
    };
    let folder = store::parent_of(path);
    // The first part is the person's own space: not a keyword.
    for part in folder.split('/').skip(1) {
        add(part);
    }
    for s in [&d.make, &d.model, &d.lens].into_iter().flatten() {
        add(s);
    }
    if let Some(t) = d.taken.as_deref().filter(|t| t.len() >= 7) {
        add(&t[..4]);
        if let Some(m) = t[5..7].parse::<usize>().ok().filter(|m| (1..=12).contains(m)) {
            add(MONTHS[m - 1]);
        }
    }
    for l in &i.labels {
        add(l);
    }
    add(&i.subject);
    out
}

/// Why this person can't save here, if so: (reason, 403 rather than 409).
fn why_not(l: &Look) -> Option<(String, bool)> {
    if !l.may_edit {
        Some(("You can view this picture but not change its details.".into(), true))
    } else if let Some(why) = l.read_only {
        Some((why.into(), true))
    } else if let Err(e) = &l.disk {
        Some((e.say(&l.side_name), false))
    } else {
        None
    }
}

fn reply_get(l: &Look) -> Value {
    let mut d = details_of(l.file.as_ref());
    if !l.may_edit {
        (d.lat, d.lon, d.alt) = (None, None, None);
    }
    let info = l.bytes().map(sidecar::read_info).unwrap_or_default();
    let why = why_not(l);
    json!({
        "exif": d, "info": info, "keywords": keywords(&l.path, &d, &info), "folder": store::parent_of(&l.path),
        "etag": l.etag(), "writable": why.is_none(), "reason": why.map(|w| w.0),
    })
}

pub async fn route(app: &Shared, m: &str, id: String, me: Option<auth::User>, headers: &HeaderMap, body: &[u8]) -> Response<Body> {
    if m != "GET" && m != "PUT" {
        return error(405, "use GET or PUT");
    }
    let new: Option<Info> = if m == "PUT" {
        if body.len() > MAX_BODY {
            return error(413, "The details are too big (16 KB at most).");
        }
        match serde_json::from_slice::<Info>(body).map_err(|e| e.to_string()).and_then(Info::cleaned) {
            Ok(i) => Some(i),
            Err(e) => return error(400, &format!("These aren't valid details: {e}")),
        }
    } else {
        None
    };
    let if_match = headers.get(header::IF_MATCH).and_then(|v| v.to_str().ok()).map(|v| v.trim().trim_matches('"').to_string());
    if m == "PUT" && if_match.is_none() {
        return error(428, "Send If-Match with the etag the details were read with.");
    }
    let (me2, id2) = (me.clone(), id.clone());
    let first = match blocking(app, move |a| look(a, me2.as_ref(), &id2)).await {
        Some(Ok(l)) => l,
        Some(Err(r)) => return r,
        None => return error(500, "server error"),
    };
    let Some(new) = new else {
        return match tokio::task::spawn_blocking(move || reply_get(&first)).await {
            Ok(v) => send_json(&v, 200),
            Err(_) => error(500, "server error"),
        };
    };
    if let Some((why, forbidden)) = why_not(&first) {
        return send_json(&json!({"error": why, "etag": first.etag()}), if forbidden { 403 } else { 409 });
    }
    let wanted = if_match.unwrap_or_default();
    let etag = first.etag();
    if (wanted == "*" && first.bytes().is_none()) || (wanted != "*" && wanted != etag) {
        return send_json(&json!({"error": "The details were changed somewhere else since you opened them.", "etag": etag}), 409);
    }
    let merged = {
        let (bytes, new2) = (first.bytes().map(<[u8]>::to_vec), new.clone());
        match tokio::task::spawn_blocking(move || sidecar::merge_info(bytes.as_deref(), &new2)).await {
            Ok(r) => r,
            Err(_) => return error(500, "server error"),
        }
    };
    let out = match merged {
        Ok(Merged::Same) => return send_json(&json!({"etag": etag}), 200),
        Ok(Merged::Write(b)) if Some(b.as_slice()) == first.bytes() => return send_json(&json!({"etag": etag}), 200),
        Ok(Merged::Write(b)) => Some(b),
        Ok(Merged::Delete) => None,
        Err(e) => return error(409, &format!("Its sidecar ({}) couldn't be merged ({e}), so it wasn't changed.", first.side_name)),
    };
    let was = (first.path.clone(), first.stat().map(str::to_string));
    let done = blocking(app, move |a| -> Result<(String, String, Side), Response<Body>> {
        let l = look(a, me.as_ref(), &id)?;
        if !l.may_edit || l.read_only.is_some() {
            return Err(error(403, "You can no longer edit this picture."));
        }
        let changed = || send_json(&json!({"error": "The details were changed somewhere else since you opened them.", "etag": l.etag()}), 409);
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
        let edits = out.as_deref().and_then(|b| sidecar::read(b).ok()).map(|p| p.edits).unwrap_or_default();
        Ok((sidecar::etag(out.as_deref()), l.path, Side { edits: sidecar::edits_hash(&edits), stat }))
    })
    .await
    .unwrap_or_else(|| Err(error(500, "server error")));
    let (etag, path, side) = match done {
        Ok(d) => d,
        Err(r) => return r,
    };
    // The index keeps the sidecar's stat (so it isn't read as a change made elsewhere).
    let app2 = app.clone();
    let _ = tokio::task::spawn_blocking(move || service(&app2.store).filter(|s| s.covers(&path)).and_then(|s| s.edits_saved(&path, &side))).await;
    send_json(&json!({"etag": etag}), 200)
}
