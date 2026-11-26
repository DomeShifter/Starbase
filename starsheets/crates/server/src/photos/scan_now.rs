//! "Scan now": someone asks Photos to look for new and changed pictures straight away, then for
//! new faces, instead of waiting for the regular rescan and the face worker's next round.
//!
//! - `POST /api/photos/scan`: queues a rescan (urgent, with everything inside) of every folder
//!   with Photos on that I may see (the server's owner: all of them) and wakes the face worker.
//!   `{"queued": n}`; 429 when I asked less than 30 s ago; 401 signed out.
//! - `GET /api/photos/scan`: how my last "Scan now" is getting on: `{"busy", "queued",
//!   "photos_new", "faces_on", "thumbs_waiting", "faces": {"done", "all"}, "faces_new"}`.
//!   `busy` while a queued rescan hasn't finished; "new" = entered the index in my folders since
//!   my previous "Scan now" (the first time: the last 10 minutes), also when the watcher found it first; `faces` is the face worker's progress; `faces_new`
//!   counts only faces in pictures in my folders.
//!
//! Nothing here adds a schedule: the 10-minute rescan, the rescan on opening a folder and the
//! face worker's rounds stay as they are. Only folders I may see are counted or scanned.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::Response;
use serde_json::json;

use super::{Photos, faces, service};
use crate::{Shared, auth, blocking, error, send_json, sharing};

/// At most one accepted "Scan now" per person in this time.
pub const GAP: Duration = Duration::from_secs(30);

/// One person's last "Scan now".
pub struct Run {
    at: Instant,
    /// Unix seconds when this run started: the next run counts what is new since then.
    at_unix: i64,
    /// "New" = entered the index at or after this (the previous run, or 10 minutes back).
    since: i64,
    tickets: Vec<u64>,
    folders: Vec<String>,
}

/// Tests: as if `who`'s last "Scan now" was more than 30 s ago.
#[cfg(test)]
pub(super) fn backdate(ph: &Photos, who: &str) {
    if let Some(r) = ph.scan_now.lock().unwrap_or_else(|e| e.into_inner()).get_mut(who) {
        r.at -= GAP + Duration::from_secs(1);
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The folders with Photos on that `me` may see.
fn my_folders(a: &crate::App, me: Option<&auth::User>, ph: &Photos) -> Vec<String> {
    let acc = sharing::Access::new(&a.store, a.auth, me);
    let on = ph.enabled.lock().unwrap_or_else(|e| e.into_inner()).clone();
    on.into_iter().filter(|f| acc.folder_role(f) >= sharing::Role::View).collect()
}

/// Pictures in `folders` that entered the index after `since`.
fn new_in(ph: &Photos, folders: &[String], since: i64) -> Vec<String> {
    let added: HashSet<String> = ph.with_db(|ix| ix.added_since(since).unwrap_or_default()).into_iter().collect();
    let mine: HashSet<String> = folders.iter().flat_map(|f| ph.rows(f, true)).map(|r| r.path).filter(|p| added.contains(p)).collect();
    mine.into_iter().collect()
}

/// (done, all) of the face worker (the whole server, as the People page shows it).
fn face_counts(ph: &Photos) -> (u64, u64) {
    let model = ph.face_model().to_string();
    let (done, all, _) = ph.with_db(|ix| faces::progress(ix, &model));
    (done, all)
}

/// Faces found in the pictures at `paths` (mine only: never other people's).
fn faces_in(ph: &Photos, paths: &[String]) -> u64 {
    let mine: HashSet<&String> = paths.iter().collect();
    let model = ph.face_model().to_string();
    ph.with_db(|ix| {
        let thumbs: HashSet<String> =
            faces::thumb_paths(ix).unwrap_or_default().into_iter().filter(|(_, p)| mine.contains(p)).map(|(t, _)| t).collect();
        faces::faces(ix, &model).unwrap_or_default().iter().filter(|f| thumbs.contains(&f.thumb)).count() as u64
    })
}

pub async fn route(app: &Shared, m: &str, me: Option<auth::User>) -> Response<Body> {
    if app.auth && me.is_none() {
        return error(401, "Sign in first.");
    }
    let who = me.as_ref().map(|u| u.username.clone()).unwrap_or_default();
    let post = m == "POST";
    if !post && m != "GET" {
        return error(405, "use GET or POST");
    }
    blocking(app, move |a| {
        let Some(ph) = service(&a.store) else { return error(503, super::NOT_SET_UP) };
        if post { start(a, me.as_ref(), &who, &ph) } else { how_far(&who, &ph) }
    })
    .await
    .unwrap_or_else(|| error(500, "server error"))
}

fn start(a: &crate::App, me: Option<&auth::User>, who: &str, ph: &Photos) -> Response<Body> {
    {
        let runs = ph.scan_now.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = runs.get(who)
            && r.at.elapsed() < GAP
        {
            let left = (GAP - r.at.elapsed()).as_secs().max(1);
            return error(429, &format!("A scan was started just now. You can ask again in {left} seconds."));
        }
    }
    let folders = my_folders(a, me, ph);
    let tickets: Vec<u64> = folders.iter().map(|f| ph.request_scan(f, true, true)).collect();
    ph.ml.poke();
    let n = tickets.len();
    let mut runs = ph.scan_now.lock().unwrap_or_else(|e| e.into_inner());
    if runs.len() > 10_000 {
        runs.clear();
    }
    let at_unix = now_unix();
    let since = runs.get(who).map_or(at_unix - 600, |r| r.at_unix);
    runs.insert(who.to_string(), Run { at: Instant::now(), at_unix, since, tickets, folders });
    send_json(&json!({ "queued": n }), 200)
}

fn how_far(who: &str, ph: &Photos) -> Response<Body> {
    let (tickets, folders, since) = {
        let runs = ph.scan_now.lock().unwrap_or_else(|e| e.into_inner());
        match runs.get(who) {
            Some(r) => (r.tickets.clone(), r.folders.clone(), r.since),
            None => (Vec::new(), Vec::new(), 0),
        }
    };
    let new = if tickets.is_empty() { Vec::new() } else { new_in(ph, &folders, since) };
    let busy = tickets.iter().any(|t| !ph.wait_scan(*t, Duration::ZERO));
    let (done, all) = face_counts(ph);
    let (queued, working) = ph.queued();
    send_json(
        &json!({
            "busy": busy,
            "queued": tickets.len(),
            "photos_new": new.len(),
            "faces_on": ph.faces_on(),
            "thumbs_waiting": queued + working,
            "faces": { "done": done, "all": all },
            "faces_new": faces_in(ph, &new),
        }),
        200,
    )
}
