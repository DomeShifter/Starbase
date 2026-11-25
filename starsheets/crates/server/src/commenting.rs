//! Commenters (sharing.rs `Role::Comment`, as in Google Docs): someone who may read a document
//! and add, reply to, resolve and change their own comments, but not change the document.
//!
//! Starbase Docs saves a document whole, comments included (they are parts of the same .docx).
//! A commenter's save is never kept as sent: [`put`] takes every `PUT api/files/<id>` from a
//! commenter and builds the new file from the server's copy, taking only the commenter's own
//! comment changes from what was sent (docx_comments.rs). Every other route that changes a file
//! (rename, restore, delete, move, sharing, new files in a folder shared for commenting, the
//! preview picture, the spreadsheets' live room) needs `Role::Edit`, which a commenter hasn't
//! got. Spreadsheets have no comments here yet: a commenter on one is a viewer.
//!
//! - The save must name the version it was made from (`If-Match`) and that must still be the
//!   server's, else 409: comments are matched by id, which only holds for the same version.
//! - What is sent may be at most 4 times the server's file plus 1 MB.
//! - The file is read and rebuilt outside the server's store lock (a big document doesn't hold
//!   up everyone else's files); the version is checked again under the lock before writing.
//! - Whose comment is whose: the comments a commenter adds this way are recorded per document in
//!   `STORE/.starsheets/commenting.json` ("owned": id and the date the server gave it); only
//!   those, still carrying that date, may they change or delete. A display name could be copied,
//!   and an id can be used again for someone else's comment after an editor deletes one.
//! - A comment's author is the commenter's name with their username, so nobody can comment as
//!   someone else by taking their display name.
//! - Versions: commenters' saves within 10 minutes of the first of them, with no editor's save
//!   in between, replace each other instead of each keeping a version, so many small comment
//!   saves can't push real versions out of the history (KEEP).
//! - At most [`AT_ONCE`] saves are rebuilt at the same time (more get 503: try again).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::{Body, Bytes};
use axum::http::Response;
use serde_json::{Value, json};

use crate::sharing::Role;
use crate::{Shared, auth, error, send_json, sharing, spaces, store};

/// How long a commenter's saves share one version (seconds).
const COALESCE_SECS: u64 = 600;

/// Commenter saves rebuilt at the same time, at most.
const AT_ONCE: usize = 2;
static BUILDING: AtomicUsize = AtomicUsize::new(0);

/// One rebuild under way (see [`AT_ONCE`]); done when dropped.
struct Turn;
impl Turn {
    fn take() -> Option<Turn> {
        if BUILDING.fetch_add(1, Ordering::AcqRel) >= AT_ONCE {
            BUILDING.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(Turn)
    }
}
impl Drop for Turn {
    fn drop(&mut self) {
        BUILDING.fetch_sub(1, Ordering::AcqRel);
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The initials Word shows on a comment: the first letter of each word, at most three.
fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).flat_map(char::to_uppercase).take(3).collect()
}

enum First {
    NotMine,
    Answer(Response<Body>),
    Go { path: String, version: String, old: Vec<u8>, own: HashMap<u32, String> },
}

fn log_path(a: &crate::App) -> std::path::PathBuf {
    a.store.meta().join("commenting.json")
}

/// A commenter's save of document `file`. `None` when `me` isn't a commenter on it: the request
/// then goes the usual way (and is refused there if they may only view).
pub(crate) async fn put(app: &Shared, me: Option<auth::User>, file: String, if_match: Option<String>, body: Bytes) -> Option<Response<Body>> {
    if !app.auth {
        return None;
    }
    let me = me?;
    let (me2, file2) = (me.clone(), file.clone());
    let first = crate::blocking(app, move |a| {
        let role = sharing::file_role(&a.store, a.auth, Some(&me2), &file2);
        if role != Role::Comment {
            return First::NotMine;
        }
        let Some(path) = a.store.path_of(&file2).ok().flatten() else { return First::Answer(error(404, "no such file")) };
        if !path.to_lowercase().ends_with(".docx") {
            return First::Answer(error(403, "You can only view that."));
        }
        let fpath = a.store.full(&path);
        let Ok(version) = a.store.version_of(&fpath) else { return First::Answer(error(404, "no such file")) };
        if if_match.as_deref() != Some(version.as_str()) {
            return First::Answer(send_json(&json!({"error": "changed elsewhere", "name": store::base_of(&path)}), 409));
        }
        let log = a.store.load_json(&log_path(a));
        let own: HashMap<u32, String> = log
            .get("owned")
            .and_then(|o| o.get(&file2))
            .and_then(|f| f.get(&me2.username))
            .and_then(Value::as_object)
            .map(|ids| ids.iter().filter_map(|(id, date)| Some((id.parse().ok()?, date.as_str()?.to_string()))).collect())
            .unwrap_or_default();
        match std::fs::read(&fpath) {
            Ok(old) => First::Go { path, version, old, own },
            Err(_) => First::Answer(error(500, "server error")),
        }
    })
    .await;
    let (path, version, old, own) = match first {
        None => return Some(error(500, "server error")),
        Some(First::NotMine) => return None,
        Some(First::Answer(r)) => return Some(r),
        Some(First::Go { path, version, old, own }) => (path, version, old, own),
    };
    if body.len() > old.len().saturating_mul(4).saturating_add(1024 * 1024) {
        return Some(error(413, "That is much bigger than the document: only comments can be saved."));
    }
    // Read and rebuilt outside the store lock, a few at a time.
    let Some(turn) = Turn::take() else { return Some(error(503, "The server is busy saving comments: trying again shortly.")) };
    let name = if me.name.trim().is_empty() || me.name.trim() == me.username {
        me.username.clone()
    } else {
        format!("{} ({})", me.name.trim(), me.username)
    };
    let who = crate::docx_comments::Me { initials: initials(&name), name, date: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(), own };
    let merged = tokio::task::spawn_blocking(move || crate::docx_comments::merge(&old, &body, &who)).await;
    drop(turn);
    let merged = match merged {
        Ok(Ok(m)) => m,
        Ok(Err(why)) => return Some(error(403, &format!("Only your comments can be saved ({why})."))),
        Err(_) => return Some(error(500, "server error")),
    };
    let r = crate::blocking(app, move |a| {
        // Still a commenter (the share may have changed meanwhile), and still the same version.
        if sharing::file_role(&a.store, a.auth, Some(&me), &file) != Role::Comment {
            return error(403, "You can't comment on that any more.");
        }
        let fpath = a.store.full(&path);
        let now_version = a.store.version_of(&fpath).unwrap_or_default();
        let folder = store::parent_of(&path);
        let name = store::base_of(&path);
        if now_version != version {
            return send_json(&json!({"error": "changed elsewhere", "name": name}), 409);
        }
        let Some(merged) = merged else {
            // No comment changes: nothing written.
            return send_json(&json!({"version": now_version, "name": name, "folder": folder}), 200);
        };
        let log = log_path(a);
        let mut all = a.store.load_json(&log);
        let last = all.get("runs").and_then(|r| r.get(&file)).cloned().unwrap_or(Value::Null);
        let same_run =
            last["version"].as_str() == Some(now_version.as_str()) && now().saturating_sub(last["at"].as_u64().unwrap_or(0)) < COALESCE_SECS;
        if !same_run && a.store.keep_version(&file, &fpath).is_err() {
            return error(500, "couldn't keep the earlier version");
        }
        if a.store.write_file(&fpath, &merged.bytes).is_err() {
            return error(500, "couldn't save the document");
        }
        let version = a.store.version_of(&fpath).unwrap_or_default();
        let at = if same_run { last["at"].as_u64().unwrap_or_else(now) } else { now() };
        let mut runs = all.get("runs").and_then(Value::as_object).cloned().unwrap_or_default();
        runs.insert(file.clone(), json!({"by": me.username, "at": at, "version": version}));
        all.insert("runs".into(), Value::Object(runs));
        // Whose comments these are, for their next save.
        let mut owned = all.get("owned").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut by_file = owned.get(&file).and_then(Value::as_object).cloned().unwrap_or_default();
        let ids: serde_json::Map<String, Value> = merged.own.iter().map(|(id, date)| (id.to_string(), json!(date))).collect();
        by_file.insert(me.username.clone(), Value::Object(ids));
        owned.insert(file.clone(), Value::Object(by_file));
        all.insert("owned".into(), Value::Object(owned));
        let _ = a.store.save_json(&log, &Value::Object(all));
        let _ = spaces::note_changed(&a.store, &file, &me.username);
        send_json(&json!({"version": version, "name": name, "folder": folder}), 200)
    })
    .await;
    Some(r.unwrap_or_else(|| error(500, "server error")))
}

#[cfg(test)]
#[path = "commenting_tests.rs"]
mod tests;
