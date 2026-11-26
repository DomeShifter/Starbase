//! commenting.rs through the server's routes: a commenter's save keeps only their comments, no
//! other route lets them change a file, a save much bigger than the document is refused, and
//! their saves share one version for a while.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::*;

const NS: &str =
    r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#;

fn docx(body: &str, comments: &str) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let mut parts = vec![
        ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#.to_string()),
        ("_rels/.rels", r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string()),
        ("word/document.xml", format!("<w:document {NS}><w:body>{body}</w:body></w:document>")),
    ];
    if !comments.is_empty() {
        parts.push(("word/comments.xml", format!("<w:comments {NS}>{comments}</w:comments>")));
    }
    for (n, t) in parts {
        z.start_file(n, opts).unwrap();
        z.write_all(t.as_bytes()).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    if let Ok(mut f) = z.by_name(name) {
        f.read_to_string(&mut s).unwrap();
    }
    s
}

const PLAIN: &str = "<w:p><w:r><w:t>Hello world</w:t></w:r></w:p>";

/// "world" commented, comment `id` saying `text`.
fn commented(id: u32, text: &str) -> Vec<u8> {
    commented_and(id, text, "")
}

/// [`commented`], with `more` after it in the body.
fn commented_and(id: u32, text: &str, more: &str) -> Vec<u8> {
    let body = format!(
        r#"<w:p><w:r><w:t xml:space="preserve">Hello </w:t></w:r><w:commentRangeStart w:id="{id}"/><w:r><w:t>world</w:t></w:r><w:commentRangeEnd w:id="{id}"/><w:r><w:commentReference w:id="{id}"/></w:r></w:p>{more}"#
    );
    docx(&body, &format!(r#"<w:comment w:id="{id}" w:author="Whoever"><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:comment>"#))
}

struct Got {
    status: u16,
    etag: String,
    json: Value,
    bytes: Vec<u8>,
    cookie: String,
}

async fn send(app: &Shared, method: &str, uri: &str, cookie: &str, if_match: Option<&str>, body: Vec<u8>) -> Got {
    send_ids(app, method, uri, cookie, if_match, "", body).await
}

/// [`send`] with an `X-Comment-Ids` header (when `ids` isn't empty).
async fn send_ids(app: &Shared, method: &str, uri: &str, cookie: &str, if_match: Option<&str>, ids: &str, body: Vec<u8>) -> Got {
    use axum::http::header;
    use tower::ServiceExt;
    let mut b = axum::http::Request::builder().method(method).uri(uri).header(header::HOST, "192.168.1.5:8080");
    if !ids.is_empty() {
        b = b.header("X-Comment-Ids", ids);
    }
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

fn js(v: Value) -> Vec<u8> {
    serde_json::to_vec(&v).unwrap()
}

/// A server with boss (owner), ann (commenter on Letter.docx and Sums.xlsx) and vic (viewer):
/// (app, boss, ann, vic, the document's id, the spreadsheet's id).
async fn setup() -> (Shared, String, String, String, String, String) {
    let dir = std::env::temp_dir().join(format!("starsheets-commenting-{}", store::random_hex(6)));
    std::fs::create_dir_all(&dir).unwrap();
    let st = store::Store { root: dir.clone(), keep: 3, spaces: true };
    std::fs::create_dir_all(st.meta()).unwrap();
    let app: Shared = Arc::new(crate::App {
        store: st,
        base: dir,
        lock: Mutex::new(()),
        location: "here".into(),
        auth: true,
        site: None,
        rooms: crate::live::Rooms::default(),
        oidc: None,
    });
    let code = auth::setup_code(&app.store);
    let g =
        send(&app, "POST", "/api/setup", "", None, js(json!({"code": code, "username": "boss", "name": "Boss", "password": "bosspassword"}))).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    let boss = g.cookie;
    let mut cookies = Vec::new();
    for (who, name) in [("ann", "Ann Lee"), ("vic", "Vic")] {
        let g = send(&app, "POST", "/api/users", &boss, None, js(json!({"username": who, "name": name, "password": format!("{who}password")}))).await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        cookies.push(send(&app, "POST", "/api/login", "", None, js(json!({"username": who, "password": format!("{who}password")}))).await.cookie);
    }
    let id = send(&app, "POST", "/api/files?name=Letter.docx", &boss, None, docx(PLAIN, "")).await.json["id"].as_str().unwrap().to_string();
    let sheet = send(&app, "POST", "/api/files?name=Sums.xlsx", &boss, None, b"sheet".to_vec()).await.json["id"].as_str().unwrap().to_string();
    for f in [&id, &sheet] {
        let g = send(&app, "POST", &format!("/api/files/{f}/sharing"), &boss, None, js(json!({"people": {"ann": "comment", "vic": "view"}}))).await;
        assert_eq!(g.status, 200, "{:?}", g.json);
    }
    (app, boss, cookies[0].clone(), cookies[1].clone(), id, sheet)
}

#[tokio::test]
async fn a_commenters_save_keeps_only_their_comment_and_no_route_changes_the_text() {
    let (app, boss, ann, vic, id, sheet) = setup().await;
    let g = send(&app, "GET", &format!("/api/files/{id}/sharing"), &ann, None, vec![]).await;
    assert_eq!(g.json["you"], "comment");
    let g = send(&app, "GET", &format!("/api/files/{id}"), &ann, None, vec![]).await;
    assert_eq!(g.status, 200, "a commenter reads the document");
    let v1 = g.etag;
    // A comment: kept, with the server's name for its author.
    let g = send(&app, "PUT", &format!("/api/files/{id}"), &ann, Some(&v1), commented(0, "Nice")).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    let v2 = g.json["version"].as_str().unwrap().to_string();
    let now = send(&app, "GET", &format!("/api/files/{id}"), &boss, None, vec![]).await.bytes;
    assert!(part(&now, "word/comments.xml").contains(r#"w:author="Ann Lee (ann)""#), "the server's name for her, with her username");
    assert!(part(&now, "word/document.xml").contains("commentRangeStart"));
    // Text added after the comment, and no comment changes: nothing written.
    let sent = commented_and(0, "Nice", "<w:p><w:r><w:t>EVIL</w:t></w:r></w:p>");
    let g = send(&app, "PUT", &format!("/api/files/{id}"), &ann, Some(&v2), sent).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    assert_eq!(g.json["version"].as_str(), Some(v2.as_str()), "nothing written");
    // Without the version it was made from, or from an older one.
    assert_eq!(send(&app, "PUT", &format!("/api/files/{id}"), &ann, None, commented(1, "x")).await.status, 409);
    assert_eq!(send(&app, "PUT", &format!("/api/files/{id}"), &ann, Some(&v1), commented(1, "x")).await.status, 409);
    // Far bigger than the document.
    let big = vec![b'x'; now.len() * 4 + 1024 * 1024 + 1];
    assert_eq!(send(&app, "PUT", &format!("/api/files/{id}"), &ann, Some(&v2), big).await.status, 413);
    // Every other route that changes it.
    let refused = [
        ("POST", format!("/api/files/{id}/rename?name=Mine"), vec![]),
        ("DELETE", format!("/api/files/{id}"), vec![]),
        ("POST", format!("/api/files/{id}/versions/current/name?name=X"), vec![]),
        ("POST", format!("/api/files/{id}/versions/x.docx/restore"), vec![]),
        ("POST", format!("/api/files/{id}/move?folder=ann"), vec![]),
        ("PUT", format!("/api/files/{id}/thumb?version={v2}"), b"png".to_vec()),
        ("POST", format!("/api/files/{id}/sharing"), js(json!({"people": {"ann": "edit"}}))),
        ("POST", "/api/files?folder=boss&name=Letter.docx".to_string(), commented(3, "x")),
        ("PUT", format!("/api/files/{sheet}"), b"changed".to_vec()),
    ];
    for (m, uri, body) in refused {
        let g = send(&app, m, &uri, &ann, Some(&v2), body).await;
        assert!(g.status == 403 || g.status == 404, "{m} {uri}: {} {:?}", g.status, g.json);
    }
    // A viewer can't save even a comment.
    assert_eq!(send(&app, "PUT", &format!("/api/files/{id}"), &vic, Some(&v2), commented(1, "x")).await.status, 403);
    assert_eq!(send(&app, "GET", &format!("/api/files/{id}"), &boss, None, vec![]).await.bytes, now, "as the comment left it");
    let _ = std::fs::remove_dir_all(&app.store.root);
}

async fn versions(app: &Shared, boss: &str, id: &str) -> usize {
    let g = send(app, "GET", &format!("/api/files/{id}/history"), boss, None, vec![]).await;
    g.json["versions"].as_array().map_or(0, Vec::len)
}

#[tokio::test]
async fn a_commenters_saves_share_one_version_for_ten_minutes() {
    let (app, boss, ann, _vic, id, _sheet) = setup().await;
    let mut v = send(&app, "GET", &format!("/api/files/{id}"), &ann, None, vec![]).await.etag;
    for (i, text) in ["one", "two", "three"].iter().enumerate() {
        let g = send(&app, "PUT", &format!("/api/files/{id}"), &ann, Some(&v), commented(i as u32, text)).await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        v = g.json["version"].as_str().unwrap().to_string();
    }
    assert_eq!(versions(&app, &boss, &id).await, 1, "one version for the three comment saves: the file before them");
    // Someone else's save in between ends the run.
    let now = send(&app, "GET", &format!("/api/files/{id}"), &boss, None, vec![]).await;
    let g = send(&app, "PUT", &format!("/api/files/{id}"), &boss, Some(&now.etag), now.bytes.clone()).await;
    assert_eq!(g.status, 200);
    let v = g.json["version"].as_str().unwrap().to_string();
    let before = versions(&app, &boss, &id).await;
    let g = send(&app, "PUT", &format!("/api/files/{id}"), &ann, Some(&v), commented(7, "again")).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    assert_eq!(versions(&app, &boss, &id).await, before + 1, "the editor's version is kept");
    let _ = std::fs::remove_dir_all(&app.store.root);
}

/// Review of PR #56: new comments get the server's ids (the reply says which, and the program
/// names them in later saves instead of opening the document again), an edited own comment
/// stays theirs, and the record of whose comments are whose goes with the account.
#[tokio::test]
async fn server_ids_edits_stay_yours_and_the_record_goes_with_the_account() {
    let (app, boss, ann, _vic, id, _sheet) = setup().await;
    let v = send(&app, "GET", &format!("/api/files/{id}"), &ann, None, vec![]).await.etag;
    // Sent as comment 7: the server's first id is 0, and the reply says so.
    let g = send(&app, "PUT", &format!("/api/files/{id}"), &ann, Some(&v), commented(7, "Nice")).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    assert_eq!(g.json["ids"], json!({"7": 0}));
    assert!(g.json.get("reopen").is_none(), "{:?}", g.json);
    let now = send(&app, "GET", &format!("/api/files/{id}"), &boss, None, vec![]).await;
    let c = part(&now.bytes, "word/comments.xml");
    assert!(c.contains(r#"w:id="0""#) && !c.contains(r#"w:id="7""#), "{c}");
    // Her program still calls it 7 and says so: edited twice, still hers each time.
    let mut v = now.etag;
    for text in ["Nicer", "Nicest"] {
        let g = send_ids(&app, "PUT", &format!("/api/files/{id}"), &ann, Some(&v), "7=0", commented(7, text)).await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        assert!(g.json.get("ids").is_none(), "{:?}", g.json);
        v = g.json["version"].as_str().unwrap().to_string();
        let c = part(&send(&app, "GET", &format!("/api/files/{id}"), &boss, None, vec![]).await.bytes, "word/comments.xml");
        assert!(c.contains(&format!(">{text}<")), "{text}: {c}");
    }
    let log = || app.store.load_json(&app.store.meta().join("commenting.json"));
    assert!(log()["owned"][&id].get("ann").is_some());
    // Her account goes: so does the record of her comments.
    let g = send(&app, "DELETE", "/api/users?username=ann", &boss, None, vec![]).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    assert!(log().get("owned").and_then(|o| o.get(&id)).and_then(|f| f.get("ann")).is_none(), "{:?}", log());
    let _ = std::fs::remove_dir_all(&app.store.root);
}

/// Review of PR #56 (2nd): a comment's record goes when its file is deleted for good, and a new
/// account named like a removed one can't change the removed one's comment.
#[tokio::test]
async fn a_new_ann_cant_change_the_old_anns_comment_and_the_record_goes_with_the_file() {
    let (app, boss, ann, _vic, id, _sheet) = setup().await;
    let v = send(&app, "GET", &format!("/api/files/{id}"), &ann, None, vec![]).await.etag;
    let g = send(&app, "PUT", &format!("/api/files/{id}"), &ann, Some(&v), commented(0, "Old Ann")).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    let log = || app.store.load_json(&app.store.meta().join("commenting.json"));
    assert!(log()["owned"][&id].get("ann").is_some());
    // Ann's account goes; a new "ann" is made and given commenting again.
    assert_eq!(send(&app, "DELETE", "/api/users?username=ann", &boss, None, vec![]).await.status, 200);
    let g = send(&app, "POST", "/api/users", &boss, None, js(json!({"username": "ann", "name": "Ann Two", "password": "annpassword2"}))).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    let ann2 = send(&app, "POST", "/api/login", "", None, js(json!({"username": "ann", "password": "annpassword2"}))).await.cookie;
    let g = send(&app, "POST", &format!("/api/files/{id}/sharing"), &boss, None, js(json!({"people": {"ann": "comment"}}))).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    let v = send(&app, "GET", &format!("/api/files/{id}"), &ann2, None, vec![]).await.etag;
    let g = send(&app, "PUT", &format!("/api/files/{id}"), &ann2, Some(&v), commented(0, "New Ann")).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    let c = part(&send(&app, "GET", &format!("/api/files/{id}"), &boss, None, vec![]).await.bytes, "word/comments.xml");
    assert!(c.contains(">Old Ann<") && !c.contains("New Ann"), "{c}");
    // A new comment of hers is recorded; the file deleted for good, the record goes.
    let v = send(&app, "GET", &format!("/api/files/{id}"), &ann2, None, vec![]).await.etag;
    let both = docx(
        r#"<w:p><w:commentRangeStart w:id="0"/><w:commentRangeStart w:id="1"/><w:r><w:t>Hello world</w:t></w:r><w:commentRangeEnd w:id="0"/><w:commentRangeEnd w:id="1"/></w:p>"#,
        r#"<w:comment w:id="0" w:author="x"><w:p><w:r><w:t>Old Ann</w:t></w:r></w:p></w:comment><w:comment w:id="1" w:author="x"><w:p><w:r><w:t>Hers</w:t></w:r></w:p></w:comment>"#,
    );
    let g = send(&app, "PUT", &format!("/api/files/{id}"), &ann2, Some(&v), both).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    assert!(log()["owned"][&id].get("ann").is_some(), "{:?}", log());
    crate::commenting::forget_file(&app.store, &id);
    assert!(log().get("owned").and_then(|o| o.get(&id)).is_none(), "{:?}", log());
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[test]
fn comment_id_headers() {
    assert_eq!(parse_ids(""), Some(std::collections::BTreeMap::new()));
    assert_eq!(parse_ids("7=0, 9=12"), Some([(7, 0), (9, 12)].into_iter().collect()));
    assert_eq!(parse_ids("7=0,7=1"), None, "one id twice");
    assert_eq!(parse_ids("7"), None);
    assert_eq!(parse_ids("-1=2"), None);
}

#[test]
fn format_characters_leave_display_names() {
    assert_eq!(clean_name("Ann\u{202E} Lee\u{200B}"), "Ann Lee");
    assert_eq!(clean_name(" \u{2066}\u{FEFF} "), "");
    assert_eq!(clean_name("Zoë  Ng"), "Zoë Ng");
}

/// Re-review of PR #56: the `X-Comment-Ids` header can't reach someone else's comment. Ann's
/// comment V keeps its text, date and owner whatever Vic (another commenter) names in it; a
/// header past the limit, or naming two comments as one, is refused; an editor's save ignores it.
#[tokio::test]
async fn the_comment_ids_header_cant_touch_anyone_elses_comment() {
    let (app, boss, ann, _vic, id, _sheet) = setup().await;
    let g = send(&app, "POST", &format!("/api/files/{id}/sharing"), &boss, None, js(json!({"people": {"ann": "comment", "vic": "comment"}}))).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    let vic = send(&app, "POST", "/api/login", "", None, js(json!({"username": "vic", "password": "vicpassword"}))).await.cookie;
    let v = send(&app, "GET", &format!("/api/files/{id}"), &ann, None, vec![]).await.etag;
    let g = send(&app, "PUT", &format!("/api/files/{id}"), &ann, Some(&v), commented(0, "Ann's V")).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    let before = send(&app, "GET", &format!("/api/files/{id}"), &boss, None, vec![]).await;
    let comments_before = part(&before.bytes, "word/comments.xml");
    assert!(comments_before.contains(">Ann's V<"), "{comments_before}");
    let log = || app.store.load_json(&app.store.meta().join("commenting.json"));
    let owned_before = log()["owned"][&id].clone();
    let url = format!("/api/files/{id}");
    // Vic names his comment 5 as V (0), with new text, and leaves V out of the file.
    for body in [commented(5, "HACK"), docx(PLAIN, "")] {
        let g = send_ids(&app, "PUT", &url, &vic, Some(&before.etag), "5=0", body).await;
        assert_eq!(g.status, 200, "{:?}", g.json);
        assert_eq!(g.json["version"].as_str(), Some(before.etag.as_str()), "nothing written: {:?}", g.json);
    }
    // Two of his ids named as one server id: refused.
    let g = send_ids(&app, "PUT", &url, &vic, Some(&before.etag), "5=9,6=9", commented(5, "HACK")).await;
    assert_eq!(g.status, 403, "{:?}", g.json);
    // More pairs than anyone can own: refused before anything is read.
    let many: Vec<String> = (0..=crate::docx_comments::MAX_OWNED as u32).map(|i| format!("{i}={}", i + 100_000)).collect();
    let g = send_ids(&app, "PUT", &url, &vic, Some(&before.etag), &many.join(","), commented(5, "HACK")).await;
    assert_eq!(g.status, 400, "{:?}", g.json);
    let now = send(&app, "GET", &url, &boss, None, vec![]).await;
    assert_eq!(now.bytes, before.bytes, "V's text and date as they were");
    assert_eq!(log()["owned"][&id], owned_before, "and still Ann's");
    assert!(log()["owned"][&id].get("vic").is_none(), "{:?}", log());
    // An editor's save carrying the header is kept exactly as sent.
    let mine = commented(7, "Boss");
    let g = send_ids(&app, "PUT", &url, &boss, Some(&now.etag), "7=0", mine.clone()).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    assert_eq!(send(&app, "GET", &url, &boss, None, vec![]).await.bytes, mine);
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test]
async fn a_mention_marks_the_file_for_the_person_named_until_they_open_it() {
    let (app, boss, ann, vic, _id, sheet) = setup().await;
    let g = send(&app, "POST", "/api/users", &boss, None, js(json!({"username": "zed", "name": "Zed", "password": "zedpassword"}))).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    // Ann (a commenter) names Vic (has access), Zed (none), herself and a stranger.
    let g = send(&app, "POST", &format!("/api/files/{sheet}/mentions"), &ann, None, js(json!({"users": ["vic", "zed", "ann", "nobody"]}))).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    assert_eq!(g.json["recorded"], json!(["vic"]));
    assert_eq!(g.json["no_access"], json!(["zed"]));
    let listed = |g: &Got| g.json["files"].as_array().unwrap().iter().find(|f| f["id"] == sheet.as_str()).cloned().unwrap();
    let g = send(&app, "GET", "/api/files", &vic, None, vec![]).await;
    assert_eq!(listed(&g)["mentioned_by"], "ann", "{:?}", g.json);
    let g = send(&app, "GET", "/api/files", &ann, None, vec![]).await;
    assert!(listed(&g).get("mentioned_by").is_none());
    // Opening the file clears it.
    assert_eq!(send(&app, "GET", &format!("/api/files/{sheet}"), &vic, None, vec![]).await.status, 200);
    let g = send(&app, "GET", "/api/files", &vic, None, vec![]).await;
    assert!(listed(&g).get("mentioned_by").is_none());
    // A viewer can't record mentions.
    let g = send(&app, "POST", &format!("/api/files/{sheet}/mentions"), &vic, None, js(json!({"users": ["ann"]}))).await;
    assert_eq!(g.status, 403);
    let _ = std::fs::remove_dir_all(&app.store.root);
}
