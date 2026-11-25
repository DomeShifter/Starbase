//! Find a file by its folder path: `GET /api/find-path?path=<path inside the storage folder>`.
//!
//! For the desktop's "open" handler (`starbase-open`, crates/open): a file double-clicked in a
//! file manager that lives in this server's storage folder (the network share, mounted on that
//! computer) opens here, in the browser, rather than in a desktop program writing to the share
//! behind the server's back. The desktop knows only where the storage folder is mounted on it,
//! so it asks with the path inside that folder (`ann/Budget.xlsx`), and gets the file's id.
//!
//! - The path is only a key looked up in the server's own list of files: it is never joined to a
//!   folder or read, and `..`, `.`, hidden parts (`.bin`, `.starsheets`) and backslashes are
//!   refused outright.
//! - A file is found only for someone who can open it AND sees it in their lists (their own,
//!   or shared with them by name). A file reachable only through "anyone with the link" is not
//!   found: its id is the link, and knowing a path must not hand it out.
//! - Not there and not yours answer the same 404, so the answer never says what exists.

use std::collections::HashMap;

use axum::body::Body;
use axum::http::Response;
use serde_json::{Value, json};

use crate::auth::User;
use crate::sharing::{Access, Role};
use crate::store::{self, Store};
use crate::{Shared, blocking, error, send_json};

/// The same answer for "no such file" and "not yours".
pub(crate) const NOT_FOUND: &str = "There's no file at that path that you can open.";

/// The route: `Some(answer)` for `/api/find-path`, `None` for anything else.
pub(crate) async fn route(app: &Shared, m: &str, p: &[String], q: &HashMap<String, String>, me: Option<User>) -> Option<Response<Body>> {
    if p.len() != 1 || p[0] != "find-path" {
        return None;
    }
    if m != "GET" {
        return Some(error(405, "use GET"));
    }
    let path = q.get("path").cloned().unwrap_or_default();
    let found = blocking(app, move |a| find(&a.store, a.auth, me.as_ref(), &path)).await;
    Some(match found {
        Some(Ok(v)) => send_json(&v, 200),
        Some(Err((code, why))) => error(code, why),
        None => error(500, "server error"),
    })
}

/// A file's path inside the storage folder, checked: parts joined by `/`, none empty-only, none
/// starting with `.`, no backslash or control character.
pub(crate) fn clean(path: &str) -> Result<String, (u16, &'static str)> {
    let bad = (400, "That isn't a path inside the storage folder.");
    if path.contains('\\') || path.chars().any(char::is_control) {
        return Err(bad);
    }
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() || parts.iter().any(|s| s.starts_with('.')) {
        return Err(bad);
    }
    Ok(parts.join("/"))
}

/// The file at `path` for `me`: `{id, name, path, kind}`, kind "doc" (opens in Docs) or "sheet".
pub(crate) fn find(store: &Store, auth: bool, me: Option<&User>, path: &str) -> Result<Value, (u16, &'static str)> {
    let path = clean(path)?;
    if auth && me.is_none() {
        return Err((401, "please sign in"));
    }
    let a = Access::new(store, auth, me);
    let id = a.file_paths.iter().find(|(_, p)| **p == path).map(|(id, _)| id.clone());
    match id {
        Some(id) if a.file_listed(&id) && a.file_role(&id) >= Role::View => {
            let name = store::base_of(&path);
            let kind = if store::is_doc(&name) { "doc" } else { "sheet" };
            Ok(json!({ "id": id, "name": name, "path": path, "kind": kind }))
        }
        _ => Err((404, NOT_FOUND)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{auth, sharing};
    use std::fs;

    fn store() -> Store {
        let dir = std::env::temp_dir().join(format!("starsheets-findpath-{}", store::random_hex(6)));
        fs::create_dir_all(&dir).unwrap();
        let s = Store { root: dir, keep: 2, spaces: true };
        fs::create_dir_all(s.meta()).unwrap();
        s
    }

    fn user(s: &Store, name: &str) -> User {
        auth::users(s).into_iter().find(|u| u.username == name).unwrap()
    }

    /// Boss (the owner), Ann and Bob, with files in Ann's folder.
    fn people() -> (Store, HashMap<String, String>) {
        let s = store();
        auth::add(&s, "boss", "Boss", "bosspassword").unwrap();
        auth::add(&s, "ann", "Ann", "annpassword").unwrap();
        auth::add(&s, "bob", "Bob", "bobpassword").unwrap();
        for f in ["ann/Budget.xlsx", "ann/Work/Plan.ods", "ann/Work/Letter.docx", "ann/Linked.xlsx", "ann/Shared/Team.csv"] {
            let full = s.full(f);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, "x").unwrap();
        }
        fs::create_dir_all(s.full("ann/.bin")).unwrap();
        fs::write(s.full("ann/.bin/Old.xlsx"), "o").unwrap();
        let ids: HashMap<String, String> = s.synced_index().unwrap().into_iter().map(|(id, p)| (p, id)).collect();
        (s, ids)
    }

    fn share(s: &Store, item: sharing::Item, people: &[(&str, Role)], link: Role) {
        let share = sharing::Share { people: people.iter().map(|(p, r)| (p.to_string(), *r)).collect(), link };
        sharing::set(s, item, &share).unwrap();
    }

    #[test]
    fn finds_your_own_files_by_path() {
        let (s, ids) = people();
        let ann = user(&s, "ann");
        let v = find(&s, true, Some(&ann), "ann/Budget.xlsx").unwrap();
        assert_eq!(
            (v["id"].as_str().unwrap(), v["name"].as_str(), v["kind"].as_str()),
            (ids["ann/Budget.xlsx"].as_str(), Some("Budget.xlsx"), Some("sheet"))
        );
        let v = find(&s, true, Some(&ann), "/ann//Work/Letter.docx").unwrap();
        assert_eq!((v["path"].as_str(), v["kind"].as_str()), (Some("ann/Work/Letter.docx"), Some("doc")), "extra slashes don't matter");
        // The server's owner can open everything, so finds everything.
        assert!(find(&s, true, Some(&user(&s, "boss")), "ann/Work/Plan.ods").is_ok());
        // Sign-in off: one shared tree, everyone may do everything.
        assert!(find(&s, false, None, "ann/Budget.xlsx").is_ok());
        assert_eq!(find(&s, true, None, "ann/Budget.xlsx").unwrap_err().0, 401);
        let _ = fs::remove_dir_all(&s.root);
    }

    #[test]
    fn never_finds_what_you_cant_open() {
        let (s, ids) = people();
        let bob = user(&s, "bob");
        // Not shared: the same answer as a file that isn't there.
        let not_yours = find(&s, true, Some(&bob), "ann/Budget.xlsx").unwrap_err();
        let not_there = find(&s, true, Some(&bob), "ann/Nothing.xlsx").unwrap_err();
        assert_eq!(not_yours, (404, NOT_FOUND));
        assert_eq!(not_yours, not_there, "never says what exists");
        // Shared with Bob by name (the file, or a folder it's in): found.
        share(&s, sharing::Item::File(&ids["ann/Budget.xlsx"]), &[("bob", Role::View)], Role::None);
        assert!(find(&s, true, Some(&bob), "ann/Budget.xlsx").is_ok());
        let folders: HashMap<String, String> = s.synced_folders().unwrap().into_iter().map(|(id, p)| (p, id)).collect();
        share(&s, sharing::Item::Folder(&folders["ann/Shared"]), &[("bob", Role::Edit)], Role::None);
        assert!(find(&s, true, Some(&bob), "ann/Shared/Team.csv").is_ok());
        assert!(find(&s, true, Some(&bob), "ann/Work/Plan.ods").is_err(), "only what was shared");
        // "Anyone with the link": Bob could open it with the link, but a path doesn't give it out.
        share(&s, sharing::Item::File(&ids["ann/Linked.xlsx"]), &[], Role::Edit);
        assert_eq!(sharing::file_role(&s, true, Some(&bob), &ids["ann/Linked.xlsx"]), Role::Edit);
        assert_eq!(find(&s, true, Some(&bob), "ann/Linked.xlsx").unwrap_err().0, 404);
        assert!(find(&s, true, Some(&user(&s, "ann")), "ann/Linked.xlsx").is_ok(), "its owner still finds it");
        let _ = fs::remove_dir_all(&s.root);
    }

    #[test]
    fn refuses_paths_that_leave_or_peek() {
        let (s, _) = people();
        let ann = user(&s, "ann");
        for bad in [
            "",
            "/",
            "ann/../bob/x.xlsx",
            "../etc/passwd",
            "ann/./Budget.xlsx",
            "ann/.bin/Old.xlsx",
            ".starsheets/users.json",
            "ann\\Budget.xlsx",
            "ann/Bud\nget.xlsx",
            "ann/Budget.xlsx\0",
        ] {
            assert_eq!(find(&s, true, Some(&ann), bad).unwrap_err().0, 400, "{bad:?}");
        }
        // Folders and unknown kinds of file aren't files the server lists.
        for missing in ["ann", "ann/Work", "ann/Budget.xlsx/more", "ANN/budget.xlsx"] {
            assert_eq!(find(&s, true, Some(&ann), missing).unwrap_err().0, 404, "{missing}");
        }
        let _ = fs::remove_dir_all(&s.root);
    }

    /// Through the router: signed in only, GET only, the path from the query (percent-encoded).
    #[tokio::test]
    async fn the_route_answers_signed_in_people() {
        use axum::http::header;
        use tower::ServiceExt;
        let (s, ids) = people();
        let token = auth::new_session(&s, "ann").unwrap();
        let root = s.root.clone();
        let app: Shared = std::sync::Arc::new(crate::App {
            store: s,
            base: root.clone(),
            lock: std::sync::Mutex::new(()),
            location: "here".into(),
            auth: true,
            site: None,
            rooms: crate::live::Rooms::default(),
            oidc: None,
        });
        let ask = |method: &str, uri: &str, cookie: bool| {
            let mut b = axum::http::Request::builder().method(method).uri(uri).header(header::HOST, "192.168.1.5:8080");
            if cookie {
                b = b.header(header::COOKIE, format!("{}={token}", crate::COOKIE));
            }
            let mut req = b.body(Body::empty()).unwrap();
            req.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::new([10, 9, 8, 7].into(), 50000)));
            let app = app.clone();
            async move {
                let resp = crate::router(app).oneshot(req).await.unwrap();
                let status = resp.status().as_u16();
                let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
                (status, serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null))
            }
        };
        let (status, v) = ask("GET", "/api/find-path?path=ann%2FWork%2FLetter.docx", true).await;
        assert_eq!((status, v["id"].as_str(), v["kind"].as_str()), (200, Some(ids["ann/Work/Letter.docx"].as_str()), Some("doc")), "{v}");
        assert_eq!(ask("GET", "/api/find-path?path=ann/Budget.xlsx", false).await.0, 401, "signed in only");
        assert_eq!(ask("GET", "/api/find-path?path=ann/%2E%2E/bob/x.xlsx", true).await.0, 400);
        assert_eq!(ask("GET", "/api/find-path?path=ann/None.xlsx", true).await.0, 404);
        assert_eq!(ask("GET", "/api/find-path", true).await.0, 400, "no path");
        assert_eq!(ask("POST", "/api/find-path?path=ann/Budget.xlsx", true).await.0, 405);
        let _ = fs::remove_dir_all(&root);
    }
}
