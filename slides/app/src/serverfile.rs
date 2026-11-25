//! A presentation on a Starbase server, opened from the desktop program (`starslides
//! --server-file <id>`): a [`Store`] behind the same `Link` as a local file, so saving, the
//! "changed elsewhere" copy rule and renaming work as they do for local files. (Adapted from
//! StarDocs' serverfile.rs.)
//!
//! It speaks the files API the web app uses (server.rs), with the device token from "Connect to
//! a server…" (starbase-connect):
//!
//! - read: `GET api/files/<id>`, remembering its version (ETag);
//! - write: `PUT api/files/<id>` with `If-Match` that version. A 409 means someone changed it on
//!   the server meanwhile: the Link then keeps this version as a new file beside it
//!   (`POST api/files?folder=…&name=…`) and carries on with that one, as the web app does;
//! - rename: `POST api/files/<id>/rename?name=…`.
//!
//! Plain saves, no live editing (Slides has none on the web either). The calls block the window
//! for as long as one request takes (a save every few seconds of quiet, on a home server a few
//! milliseconds); moving them onto a thread is for later.

use std::path::PathBuf;

use serde_json::Value;
use starbase_connect::Reply;

use crate::localfile::{Store, WriteError};

/// One call to the server's API (a connected [`starbase_connect::Server`], or a fake in tests).
pub trait Api {
    fn call(&self, method: &str, path: &str, body: Option<&[u8]>, if_match: Option<&str>) -> Result<Reply, String>;
}

/// The connected server: on a refused token it takes up a newer sign-in (connected again while
/// this window was open), else forgets the removed one so File offers "Connect to a server…".
impl Api for starbase_connect::Renewing {
    fn call(&self, method: &str, path: &str, body: Option<&[u8]>, if_match: Option<&str>) -> Result<Reply, String> {
        starbase_connect::Renewing::call(self, method, path, body, if_match)
    }
}

/// A presentation on the server.
pub struct ServerDoc {
    api: Box<dyn Api>,
    id: String,
    name: String,
    /// Its folder on the server; learnt from a save's answer, or looked up when a copy is made.
    folder: Option<String>,
    /// The version last read or written (the ETag).
    version: String,
}

impl ServerDoc {
    /// The document `id`; `name` is shown until the server says otherwise.
    pub fn new(api: Box<dyn Api>, id: &str, name: &str) -> Self {
        let name = if name.is_empty() { "Presentation.pptx".to_string() } else { name.to_string() };
        Self { api, id: id.to_string(), name, folder: None, version: String::new() }
    }

    /// A save's answer: the new version, and the name, folder and id it now has.
    fn took(&mut self, r: &Reply) {
        let v = r.json();
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        if let Some(ver) = s("version") {
            self.version = ver;
        }
        if let Some(n) = s("name") {
            self.name = n;
        }
        if let Some(f) = s("folder") {
            self.folder = Some(f);
        }
        if let Some(id) = s("id") {
            self.id = id;
        }
    }

    /// The document's folder (from reading it, or a save's answer), else the server's list.
    fn folder(&mut self) -> String {
        if let Some(f) = &self.folder {
            return f.clone();
        }
        let found = self.api.call("GET", "api/files", None, None).ok().filter(Reply::ok).and_then(|r| {
            r.json()
                .get("files")?
                .as_array()?
                .iter()
                .find(|f| f.get("id").and_then(Value::as_str) == Some(self.id.as_str()))?
                .get("folder")?
                .as_str()
                .map(str::to_string)
        });
        // Unknown: the server puts a new file in the person's own folder.
        let f = found.unwrap_or_default();
        self.folder = Some(f.clone());
        f
    }
}

/// Why the server said no, in plain words.
fn reason(r: &Reply) -> String {
    match r.status {
        401 => "this computer isn't connected to the server any more: File › Connect to a server".into(),
        403 => "you can view this presentation but not change it".into(),
        404 => "the presentation isn't on the server any more".into(),
        _ => r.error(),
    }
}

/// Percent-encodes a query value (as JavaScript's encodeURIComponent).
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl Store for ServerDoc {
    fn name(&self) -> String {
        self.name.clone()
    }

    /// Not a file on this computer: nothing for the recent list.
    fn location(&self) -> Option<PathBuf> {
        None
    }

    fn read(&mut self) -> Result<Vec<u8>, String> {
        let r = self.api.call("GET", &format!("api/files/{}", enc(&self.id)), None, None)?;
        if !r.ok() {
            return Err(reason(&r));
        }
        self.version = r.etag.clone().unwrap_or_default();
        if let Some(n) = r.name.clone().filter(|n| !n.is_empty()) {
            self.name = n;
        }
        if r.folder.is_some() {
            self.folder = r.folder.clone();
        }
        Ok(r.body)
    }

    fn write(&mut self, _name: &str, bytes: &[u8]) -> Result<(), WriteError> {
        let version = (!self.version.is_empty()).then(|| self.version.clone());
        let r = self.api.call("PUT", &format!("api/files/{}", enc(&self.id)), Some(bytes), version.as_deref()).map_err(WriteError::Other)?;
        match r.status {
            200 | 201 => {
                self.took(&r);
                Ok(())
            }
            409 => Err(WriteError::ChangedElsewhere),
            _ => Err(WriteError::Other(reason(&r))),
        }
    }

    fn keep_copy(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let folder = self.folder();
        let r = self.api.call("POST", &format!("api/files?folder={}&name={}", enc(&folder), enc(name)), Some(bytes), None)?;
        if !r.ok() {
            return Err(reason(&r));
        }
        self.took(&r);
        Ok(())
    }

    fn saved_text(&self) -> &'static str {
        "Saved to Starbase"
    }

    fn rename(&mut self, stem: &str) -> Result<(), String> {
        let r = self.api.call("POST", &format!("api/files/{}/rename?name={}", enc(&self.id), enc(stem)), None, None)?;
        if !r.ok() {
            return Err(reason(&r));
        }
        if let Some(n) = r.json().get("name").and_then(Value::as_str) {
            self.name = n.to_string();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A pretend server: answers in order, and remembers what was asked.
    #[derive(Clone, Default)]
    struct Fake {
        answers: Rc<RefCell<Vec<Reply>>>,
        asked: Rc<RefCell<Vec<(String, String, Option<String>)>>>,
    }

    impl Api for Fake {
        fn call(&self, method: &str, path: &str, _body: Option<&[u8]>, if_match: Option<&str>) -> Result<Reply, String> {
            self.asked.borrow_mut().push((method.into(), path.into(), if_match.map(str::to_string)));
            let mut a = self.answers.borrow_mut();
            if a.is_empty() { Err("no answer".into()) } else { Ok(a.remove(0)) }
        }
    }

    fn reply(status: u16, etag: Option<&str>, body: &str) -> Reply {
        Reply { status, etag: etag.map(str::to_string), body: body.as_bytes().to_vec(), ..Default::default() }
    }

    #[test]
    fn saves_over_the_version_read_and_a_conflict_becomes_a_copy() {
        let fake = Fake::default();
        fake.answers.borrow_mut().extend([
            Reply { status: 200, etag: Some("v1".into()), name: Some("Report.pptx".into()), folder: Some("Work".into()), body: b"doc".to_vec() },
            reply(200, None, r#"{"version":"v2","name":"Report.pptx","folder":"Work"}"#),
            reply(409, None, r#"{"error":"changed elsewhere"}"#),
            reply(201, None, r#"{"id":"new1","name":"Report (copy).pptx","folder":"Work","version":"c1"}"#),
            reply(200, None, r#"{"version":"c2"}"#),
        ]);
        let mut d = ServerDoc::new(Box::new(fake.clone()), "abc", "");
        assert_eq!(d.read().unwrap(), b"doc");
        assert_eq!((d.name(), d.saved_text()), ("Report.pptx".to_string(), "Saved to Starbase"), "the server's name for it");
        d.write("x", b"one").unwrap();
        assert_eq!(d.write("x", b"two"), Err(WriteError::ChangedElsewhere), "never over someone else's change");
        d.keep_copy("Report (copy).pptx", b"two").unwrap();
        assert_eq!(d.name(), "Report (copy).pptx");
        d.write("x", b"three").unwrap();
        let asked = fake.asked.borrow();
        assert_eq!(asked[1], ("PUT".into(), "api/files/abc".into(), Some("v1".into())));
        assert_eq!(asked[2].2.as_deref(), Some("v2"), "the version the last save gave");
        assert_eq!(asked[3].1, "api/files?folder=Work&name=Report%20(copy).pptx", "beside it, in its folder");
        assert_eq!(asked[4], ("PUT".into(), "api/files/new1".into(), Some("c1".into())), "carries on with the copy");
        assert!(d.location().is_none());
    }

    #[test]
    fn a_copy_looks_up_the_folder_and_refusals_say_why() {
        let fake = Fake::default();
        fake.answers.borrow_mut().extend([
            reply(200, Some("v1"), "doc"),
            reply(200, None, r#"{"files":[{"id":"zz","folder":"A"},{"id":"abc","folder":"Plans/2026"}]}"#),
            reply(201, None, r#"{"id":"n","name":"c.pptx","version":"1"}"#),
            reply(401, None, r#"{"error":"please sign in"}"#),
        ]);
        let mut d = ServerDoc::new(Box::new(fake.clone()), "abc", "");
        assert_eq!(d.name(), "Presentation.pptx");
        d.read().unwrap();
        d.keep_copy("c.pptx", b"x").unwrap();
        assert_eq!(fake.asked.borrow()[2].1, "api/files?folder=Plans%2F2026&name=c.pptx");
        match d.write("x", b"y") {
            Err(WriteError::Other(why)) => assert!(why.contains("Connect to a server"), "{why}"),
            other => panic!("{other:?}"),
        }
    }
}
