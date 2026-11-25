//! Commenters (sharing.rs `Role::Comment`, as in Google Docs): someone who may read a document
//! and add, reply to and resolve comments, but not change its text.
//!
//! Starbase Docs keeps a document as one .docx and saves it whole, comments included (they are
//! parts of the same file). So a commenter's save is the whole file too, and this is where it is
//! checked: [`put`] takes every `PUT api/files/<id>` from a commenter, and keeps it only when
//! the body is the same as the file on the server; anything else is refused (403) and the
//! file is left as it was. Every other route that changes a file (rename, restore, delete, move,
//! a new file in a folder shared for commenting, the preview picture, the spreadsheets' live
//! room) asks for `Role::Edit`, which a commenter hasn't got. Spreadsheets have no comments
//! here yet: a commenter on one is a viewer.
//!
//! "The same body" ([`same_body`]) is checked on what a reader sees, not on the bytes, because
//! the editor writes every part of the file afresh on each save:
//!
//! - the text of the document, paragraph by paragraph, each character marked as plain, a
//!   suggested insertion or a suggested deletion (so accepting or rejecting a suggestion is a
//!   change). Empty paragraphs are left out, and so is the duplicate text Word keeps for older
//!   programs (`mc:Fallback`); field codes are not text;
//! - the text of the headers, footers, footnotes and endnotes, compared as a whole (the editor
//!   may number those parts differently);
//! - where a picture or other object sits (one mark per object), and no picture that the file
//!   didn't already have.
//!
//! The comments themselves (`word/comments*.xml`, `people.xml`), the comment marks in the text,
//! and the settings, styles and properties parts may differ. Formatting is not compared: a
//! commenter's editor refuses formatting (view.commenting, WordCraft fork change StarDocs
//! 0010), but a hand-made request could still change it. The text can't change.
//!
//! The save must also name the version it was made from (`If-Match`), and that must be the
//! file as it is now (else 409, as for anyone): otherwise an editor's newer text, saved between
//! the commenter opening the file and saving it, would be undone by the commenter's older copy.

use std::io::Read;

use axum::body::{Body, Bytes};
use axum::http::Response;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::sharing::Role;
use crate::{Shared, auth, error, send_json, sharing, spaces, store};

/// Limits for reading a .docx sent by someone: parts, each part, all parts together.
const MAX_ENTRIES: usize = 10_000;
const MAX_PART: u64 = 64 * 1024 * 1024;
const MAX_TOTAL: u64 = 512 * 1024 * 1024;

/// A commenter's save of document `file`. `None` when `me` isn't a commenter on it: the request
/// then goes the usual way (and is refused there if they may only view).
pub(crate) async fn put(app: &Shared, me: Option<auth::User>, file: String, if_match: Option<String>, body: Bytes) -> Option<Response<Body>> {
    if !app.auth {
        return None;
    }
    let me = me?;
    crate::blocking(app, move |a| -> Option<Response<Body>> {
        let role = sharing::file_role(&a.store, a.auth, Some(&me), &file);
        if role != Role::Comment {
            return None;
        }
        let Some(path) = a.store.path_of(&file).ok().flatten() else { return Some(error(404, "no such file")) };
        if !path.to_lowercase().ends_with(".docx") {
            return Some(error(403, "You can only view that."));
        }
        let fpath = a.store.full(&path);
        let Ok(version) = a.store.version_of(&fpath) else { return Some(error(404, "no such file")) };
        if if_match.as_deref() != Some(version.as_str()) {
            return Some(send_json(&json!({"error": "changed elsewhere", "name": store::base_of(&path)}), 409));
        }
        let Ok(old) = std::fs::read(&fpath) else { return Some(error(500, "server error")) };
        if let Err(why) = same_body(&old, &body) {
            return Some(error(403, &format!("You can only comment on this document, not change it ({why}).")));
        }
        let written = a.store.keep_version(&file, &fpath).and_then(|()| a.store.write_file(&fpath, &body));
        if written.is_err() {
            return Some(error(500, "couldn't save the document"));
        }
        let _ = spaces::note_changed(&a.store, &file, &me.username);
        let version = a.store.version_of(&fpath).unwrap_or_default();
        Some(send_json(&json!({"version": version, "name": store::base_of(&path), "folder": store::parent_of(&path)}), 200))
    })
    .await
    .unwrap_or_else(|| Some(error(500, "server error")))
}

/// What a reader sees of a document, for [`same_body`].
#[derive(Debug, PartialEq)]
struct Seen {
    /// The document's paragraphs, in order.
    main: Vec<Para>,
    /// The headers', footers', footnotes' and endnotes' paragraphs, sorted.
    other: Vec<Para>,
    /// The pictures' contents (hashes).
    media: Vec<[u8; 32]>,
}

/// A paragraph: each character, and whether it is plain (0), a suggested insertion (1) or a
/// suggested deletion (2). U+FFFC stands for a picture or other object.
type Para = Vec<(char, u8)>;

/// `Ok` when `new` (a .docx) shows the same text as `old`, else what differs.
pub fn same_body(old: &[u8], new: &[u8]) -> Result<(), String> {
    let old = body_of(old).map_err(|e| format!("the document on the server can't be read: {e}"))?;
    let new = body_of(new)?;
    if new.main != old.main {
        return Err("the text differs".into());
    }
    if new.other != old.other {
        return Err("a header, footer or note differs".into());
    }
    if new.media.iter().any(|h| !old.media.contains(h)) {
        return Err("a picture was added or changed".into());
    }
    Ok(())
}

fn body_of(docx: &[u8]) -> Result<Seen, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(docx)).map_err(|_| "not a .docx file".to_string())?;
    if zip.len() > MAX_ENTRIES {
        return Err("too many parts".into());
    }
    let mut total = 0u64;
    let mut read = |zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, i: usize| -> Result<(String, Vec<u8>), String> {
        let f = zip.by_index(i).map_err(|_| "a damaged part".to_string())?;
        let name = f.name().to_string();
        let mut out = Vec::new();
        f.take(MAX_PART + 1).read_to_end(&mut out).map_err(|_| "a damaged part".to_string())?;
        total += out.len() as u64;
        if out.len() as u64 > MAX_PART || total > MAX_TOTAL {
            return Err("too large".into());
        }
        Ok((name, out))
    };
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    // The main document part: from the package's relationships, else Word's usual name.
    let main_name = names
        .iter()
        .position(|n| n == "_rels/.rels")
        .map(|i| read(&mut zip, i))
        .transpose()?
        .and_then(|(_, rels)| main_target(&rels))
        .unwrap_or_else(|| "word/document.xml".into());
    let dir = main_name.rsplit_once('/').map_or(String::new(), |(d, _)| format!("{d}/"));
    let mut main = None;
    let mut other = Vec::new();
    let mut media = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let Some(rest) = name.strip_prefix(&dir) else { continue };
        let story = *name == main_name;
        let side = !rest.contains('/')
            && rest.ends_with(".xml")
            && (rest.starts_with("header") || rest.starts_with("footer") || rest == "footnotes.xml" || rest == "endnotes.xml");
        let picture = rest.starts_with("media/");
        if !(story || side || picture) {
            continue;
        }
        let (_, bytes) = read(&mut zip, i)?;
        if picture {
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&Sha256::digest(&bytes));
            media.push(hash);
            continue;
        }
        let paras = paragraphs(&bytes)?;
        if story {
            main = Some(paras);
        } else {
            other.extend(paras);
        }
    }
    let main = main.ok_or("no document part")?;
    other.sort();
    Ok(Seen { main, other, media })
}

/// The officeDocument relationship's target in `_rels/.rels`.
fn main_target(rels: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(rels).ok()?;
    let doc = roxmltree::Document::parse(text).ok()?;
    let target = doc
        .descendants()
        .find(|n| n.tag_name().name() == "Relationship" && n.attribute("Type").is_some_and(|t| t.ends_with("/officeDocument")))?
        .attribute("Target")?;
    Some(target.trim_start_matches('/').to_string())
}

const W: [&str; 2] = ["http://schemas.openxmlformats.org/wordprocessingml/2006/main", "http://purl.oclc.org/ooxml/wordprocessingml/main"];
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
/// Deepest nesting followed (a hostile file can't run the stack out).
const MAX_DEPTH: usize = 256;

/// A story part's non-empty paragraphs.
fn paragraphs(xml: &[u8]) -> Result<Vec<Para>, String> {
    let text = std::str::from_utf8(xml).map_err(|_| "a part isn't text".to_string())?;
    let doc = roxmltree::Document::parse(text).map_err(|_| "a part isn't XML".to_string())?;
    let mut out: Vec<Para> = Vec::new();
    walk(doc.root_element(), 0, 0, &mut Vec::new(), &mut out)?;
    out.retain(|p| !p.is_empty());
    Ok(out)
}

/// Collects the text under `n`: `open` holds the indexes in `out` of the paragraphs `n` is in
/// (innermost last: a text box's paragraphs sit inside a paragraph); `kind` is 1 inside a
/// suggested insertion, 2 inside a suggested deletion.
fn walk(n: roxmltree::Node, depth: usize, kind: u8, open: &mut Vec<usize>, out: &mut Vec<Para>) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("nested too deeply".into());
    }
    let tag = n.tag_name();
    if tag.namespace() == Some(MC) && tag.name() == "Fallback" {
        return Ok(());
    }
    let w = tag.namespace().is_some_and(|ns| W.contains(&ns));
    let mut kind = kind;
    let mut opened = false;
    if w {
        let push = |out: &mut Vec<Para>, open: &[usize], c: char, kind: u8| {
            if let Some(p) = open.last().and_then(|i| out.get_mut(*i)) {
                p.push((c, kind));
            }
        };
        match tag.name() {
            "p" => {
                out.push(Vec::new());
                open.push(out.len() - 1);
                opened = true;
            }
            "ins" | "moveTo" => kind = 1,
            "del" | "moveFrom" => kind = 2,
            "t" | "delText" => {
                for c in n.text().unwrap_or("").chars() {
                    push(out, open, c, kind);
                }
                return Ok(());
            }
            "tab" if n.parent().is_some_and(|p| p.tag_name().name() == "r") => push(out, open, '\t', kind),
            "br" | "cr" => push(out, open, '\n', kind),
            "noBreakHyphen" => push(out, open, '-', kind),
            "sym" => push(
                out,
                open,
                n.attributes()
                    .find(|a| a.name() == "char")
                    .and_then(|a| u32::from_str_radix(a.value(), 16).ok())
                    .and_then(char::from_u32)
                    .unwrap_or('?'),
                kind,
            ),
            "drawing" | "pict" | "object" => push(out, open, '\u{FFFC}', kind),
            "instrText" | "delInstrText" => return Ok(()),
            _ => {}
        }
    }
    for c in n.children().filter(roxmltree::Node::is_element) {
        walk(c, depth + 1, kind, open, out)?;
    }
    if opened {
        open.pop();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]
    use super::*;
    use std::io::Write;

    const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006""#;

    /// A .docx with `body` (the inside of `w:body`) and the extra parts given.
    pub(crate) fn docx(body: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
        let doc = format!(r#"<?xml version="1.0"?><w:document {NS}><w:body>{body}</w:body></w:document>"#);
        let mut parts: Vec<(&str, Vec<u8>)> = vec![("_rels/.rels", rels.into()), ("word/document.xml", doc.into_bytes())];
        parts.extend(extra.iter().map(|(n, b)| (*n, b.to_vec())));
        for (name, bytes) in parts {
            z.start_file(name, opts).unwrap();
            z.write_all(&bytes).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    fn para(text: &str) -> String {
        format!("<w:p><w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r></w:p>")
    }

    fn comments(text: &str) -> Vec<u8> {
        format!(r#"<w:comments {NS}><w:comment w:id="0" w:author="Ann"><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:comment></w:comments>"#)
            .into_bytes()
    }

    #[test]
    fn comments_may_change_and_the_text_may_not() {
        let old = docx(&format!("{}{}", para("Hello world"), para("Second")), &[]);
        // A comment on "world": marks in the text, the run split, and a comments part.
        let commented = docx(
            r#"<w:p><w:r><w:t xml:space="preserve">Hello </w:t></w:r><w:commentRangeStart w:id="0"/><w:r><w:t>world</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r></w:p><w:p/><w:p><w:r><w:t>Sec</w:t></w:r><w:r><w:t>ond</w:t></w:r></w:p>"#,
            &[("word/comments.xml", &comments("Nice")), ("docProps/core.xml", b"<x/>")],
        );
        assert_eq!(same_body(&old, &commented), Ok(()));
        // Any change to the text is refused, however small.
        for body in [
            format!("{}{}", para("Hello world!"), para("Second")),
            format!("{}{}", para("Second"), para("Hello world")),
            para("Hello worldSecond"),
            format!("{}{}{}", para("Hello world"), para("Second"), para("More")),
            // A suggestion is a change too.
            format!("{}<w:p><w:ins><w:r><w:t>Second</w:t></w:r></w:ins></w:p>", para("Hello world")),
            format!("{}<w:p><w:del><w:r><w:delText>Second</w:delText></w:r></w:del></w:p>", para("Hello world")),
        ] {
            let new = docx(&body, &[("word/comments.xml", &comments("Nice"))]);
            assert!(same_body(&old, &new).is_err(), "{body}");
        }
    }

    #[test]
    fn headers_notes_and_pictures_count_too() {
        let header = |t: &str| format!(r#"<w:hdr {NS}>{}</w:hdr>"#, para(t)).into_bytes();
        let pic = |t: &str| format!(r#"<w:p><w:r><w:drawing>{t}</w:drawing></w:r></w:p>"#);
        let old = docx(&format!("{}{}", para("Body"), pic("")), &[("word/header1.xml", &header("Top")), ("word/media/image1.png", b"PNG one")]);
        // The same, its parts named otherwise: fine.
        let same = docx(&format!("{}{}", para("Body"), pic("")), &[("word/header3.xml", &header("Top")), ("word/media/image7.png", b"PNG one")]);
        assert_eq!(same_body(&old, &same), Ok(()));
        let new = docx(&format!("{}{}", para("Body"), pic("")), &[("word/header1.xml", &header("Bottom")), ("word/media/image1.png", b"PNG one")]);
        assert!(same_body(&old, &new).is_err(), "a header changed");
        let new = docx(&format!("{}{}", para("Body"), pic("")), &[("word/header1.xml", &header("Top")), ("word/media/image1.png", b"PNG two")]);
        assert!(same_body(&old, &new).is_err(), "a picture replaced");
        let new = docx(&para("Body"), &[("word/header1.xml", &header("Top")), ("word/media/image1.png", b"PNG one")]);
        assert!(same_body(&old, &new).is_err(), "a picture removed");
        // Word's copy of a text box for older programs, and field codes, aren't text.
        let fallback = format!(
            "{}<w:p><mc:AlternateContent><mc:Choice Requires=\"wps\"><w:r><w:drawing/></w:r></mc:Choice><mc:Fallback><w:r><w:pict/></w:r></mc:Fallback></mc:AlternateContent></w:p><w:p><w:r><w:instrText> PAGE </w:instrText></w:r></w:p>",
            para("Body")
        );
        let with = docx(&fallback, &[("word/header1.xml", &header("Top")), ("word/media/image1.png", b"PNG one")]);
        assert_eq!(same_body(&old, &with), Ok(()));
    }

    #[test]
    fn what_isnt_a_document_is_refused() {
        let old = docx(&para("Body"), &[]);
        assert!(same_body(&old, b"not a zip").is_err());
        assert!(same_body(b"one", &old).is_err(), "nothing to compare with");
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        z.start_file("word/other.xml", zip::write::SimpleFileOptions::default()).unwrap();
        let empty = z.finish().unwrap().into_inner();
        assert!(same_body(&old, &empty).is_err(), "no document part");
        // Deep nesting is refused, not followed.
        let deep = format!("<w:p>{}{}</w:p>", "<w:ins>".repeat(300), "</w:ins>".repeat(300));
        assert!(same_body(&old, &docx(&deep, &[])).is_err());
    }

    struct Got {
        status: u16,
        etag: String,
        json: serde_json::Value,
        bytes: Vec<u8>,
        cookie: String,
    }

    async fn send(app: &Shared, method: &str, uri: &str, cookie: &str, if_match: Option<&str>, body: Vec<u8>) -> Got {
        use axum::http::header;
        use tower::ServiceExt;
        let mut b = axum::http::Request::builder().method(method).uri(uri).header(header::HOST, "192.168.1.5:8080");
        if !cookie.is_empty() {
            b = b.header(header::COOKIE, cookie);
        }
        if let Some(v) = if_match {
            b = b.header(header::IF_MATCH, v);
        }
        let mut req = b.body(Body::from(body)).unwrap();
        req.extensions_mut().insert(axum::extract::ConnectInfo(std::net::SocketAddr::new("10.9.8.7".parse().unwrap(), 50000)));
        let resp = crate::router(app.clone()).oneshot(req).await.unwrap();
        let status = resp.status().as_u16();
        let h = resp.headers().clone();
        let etag = h.get(header::ETAG).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let cookie = h.get(header::SET_COOKIE).and_then(|v| v.to_str().ok()).unwrap_or("").split(';').next().unwrap_or("").to_string();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec();
        Got { status, etag, json: serde_json::from_slice(&bytes).unwrap_or_default(), bytes, cookie }
    }

    fn js(v: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&v).unwrap()
    }

    /// Through the server's routes: a commenter may save comments and nothing else, and no other
    /// route lets them change the document.
    #[tokio::test]
    async fn a_commenter_can_comment_but_not_change_the_text_by_any_route() {
        use std::sync::{Arc, Mutex};
        let dir = std::env::temp_dir().join(format!("starsheets-commenting-{}", store::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let st = store::Store { root: dir.clone(), keep: 3, spaces: true };
        std::fs::create_dir_all(st.meta()).unwrap();
        let app: Shared = Arc::new(crate::App {
            store: st,
            base: dir.clone(),
            lock: Mutex::new(()),
            location: "here".into(),
            auth: true,
            site: None,
            rooms: crate::live::Rooms::default(),
            oidc: None,
        });
        let code = auth::setup_code(&app.store);
        let g = send(&app, "POST", "/api/setup", "", None, js(json!({"code": code, "username": "boss", "name": "Boss", "password": "bosspassword"})))
            .await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        let boss = g.cookie;
        let mut cookies = Vec::new();
        for who in ["ann", "vic"] {
            let g =
                send(&app, "POST", "/api/users", &boss, None, js(json!({"username": who, "name": who, "password": format!("{who}password")}))).await;
            assert_eq!(g.status, 200, "{:?}", g.json);
            cookies.push(send(&app, "POST", "/api/login", "", None, js(json!({"username": who, "password": format!("{who}password")}))).await.cookie);
        }
        let (ann, vic) = (cookies[0].clone(), cookies[1].clone());
        let original = docx(&para("Hello world"), &[]);
        let id = send(&app, "POST", "/api/files?name=Letter.docx", &boss, None, original).await.json["id"].as_str().unwrap().to_string();
        let sheet = send(&app, "POST", "/api/files?name=Sums.xlsx", &boss, None, b"sheet".to_vec()).await.json["id"].as_str().unwrap().to_string();
        for f in [&id, &sheet] {
            let g =
                send(&app, "POST", &format!("/api/files/{f}/sharing"), &boss, None, js(json!({"people": {"ann": "comment", "vic": "view"}}))).await;
            assert_eq!(g.status, 200, "{:?}", g.json);
        }
        let g = send(&app, "GET", &format!("/api/files/{id}/sharing"), &ann, None, vec![]).await;
        assert_eq!(g.json["you"], "comment");
        assert!(g.json["people"].as_array().unwrap().iter().any(|p| p["username"] == "ann" && p["role"] == "comment"));
        let g = send(&app, "GET", &format!("/api/files/{id}"), &ann, None, vec![]).await;
        assert_eq!(g.status, 200, "a commenter reads the document");
        let v1 = g.etag;
        // A comment: saved.
        let commented = docx(
            r#"<w:p><w:commentRangeStart w:id="0"/><w:r><w:t>Hello world</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r></w:p>"#,
            &[("word/comments.xml", &comments("Nice"))],
        );
        let g = send(&app, "PUT", &format!("/api/files/{id}"), &ann, Some(&v1), commented.clone()).await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        let v2 = g.json["version"].as_str().unwrap().to_string();
        assert_eq!(send(&app, "GET", &format!("/api/files/{id}"), &boss, None, vec![]).await.bytes, commented);
        // Changed text: refused, whatever comes with it.
        let changed = docx(&para("Goodbye world"), &[("word/comments.xml", &comments("Nice"))]);
        let g = send(&app, "PUT", &format!("/api/files/{id}"), &ann, Some(&v2), changed.clone()).await;
        assert_eq!(g.status, 403, "{:?}", g.json);
        assert!(g.json["error"].as_str().unwrap().contains("only comment"));
        // Without the version it was made from, or from an older one: never over newer text.
        assert_eq!(send(&app, "PUT", &format!("/api/files/{id}"), &ann, None, commented.clone()).await.status, 409);
        assert_eq!(send(&app, "PUT", &format!("/api/files/{id}"), &ann, Some(&v1), commented.clone()).await.status, 409);
        // Every other route that changes it.
        let refused = [
            ("POST", format!("/api/files/{id}/rename?name=Mine"), vec![]),
            ("DELETE", format!("/api/files/{id}"), vec![]),
            ("POST", format!("/api/files/{id}/versions/current/name?name=X"), vec![]),
            ("POST", format!("/api/files/{id}/versions/x.docx/restore"), vec![]),
            ("POST", format!("/api/files/{id}/move?folder=ann"), vec![]),
            ("PUT", format!("/api/files/{id}/thumb?version={v2}"), b"png".to_vec()),
            ("POST", format!("/api/files/{id}/sharing"), js(json!({"people": {"ann": "edit"}}))),
            ("POST", "/api/files?folder=boss&name=Letter.docx".to_string(), changed.clone()),
            ("PUT", format!("/api/files/{sheet}"), b"changed".to_vec()),
        ];
        for (m, uri, body) in refused {
            let g = send(&app, m, &uri, &ann, Some(&v2), body).await;
            assert!(g.status == 403 || g.status == 404, "{m} {uri}: {} {:?}", g.status, g.json);
        }
        // A viewer can't save even a comment.
        assert_eq!(send(&app, "PUT", &format!("/api/files/{id}"), &vic, Some(&v2), commented.clone()).await.status, 403);
        assert_eq!(send(&app, "GET", &format!("/api/files/{id}"), &boss, None, vec![]).await.bytes, commented, "as the comment left it");
        // File › Make a copy: into the commenter's own folder, theirs to edit.
        let g = send(&app, "POST", &format!("/api/files/{id}/copy"), &ann, None, vec![]).await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        let copy = g.json["id"].as_str().unwrap().to_string();
        assert_eq!(send(&app, "PUT", &format!("/api/files/{copy}"), &ann, None, changed).await.status, 200);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
