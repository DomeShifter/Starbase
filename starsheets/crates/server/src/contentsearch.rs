//! Search inside files: `GET /api/content-search?q=<text>`.
//!
//! Drive's search box also finds words inside documents. The home screen already matches file
//! names over the list it has loaded; this adds the files whose TEXT contains the words, each
//! with a short snippet around the match.
//!
//! - Text comes out of .docx/.xlsx/.pptx and .odt/.ods/.odp (the zip's XML, read with the same
//!   zip and roxmltree the server already uses) and out of plain-text files (.txt, .md, .csv,
//!   .tsv, .json). PDF is not searched: there is no pure-Rust text extractor among the server's
//!   dependencies.
//! - The index is a cache of each file's extracted text, filled the first time a search looks at
//!   the file and refreshed when its size or modified time changes (so a save or upload is
//!   picked up by the next search; nothing runs in the background). It holds text only, never
//!   who may see it.
//! - Who may see a hit is decided on every search with the same checks as the file list: the
//!   caller's own files and those shared with them by name (`Access::file_listed` and a role
//!   of at least View). Anyone-with-the-link files that are not in their list are not searched.
//! - Bounds: files over 25 MB are skipped, each file keeps its first 512 KB of text, a zip is
//!   read for at most 16 MB of XML in total, and a search returns at most 50 files.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, LazyLock, Mutex};

use axum::body::Body;
use axum::http::Response;
use serde_json::{Value, json};

use crate::auth::User;
use crate::sharing::{Access, Role};
use crate::store::Store;
use crate::{Shared, blocking, error, send_json};

const MAX_FILE: u64 = 25 * 1024 * 1024;
const MAX_TEXT: usize = 512 * 1024;
const MAX_XML: u64 = 16 * 1024 * 1024;
const MAX_HITS: usize = 50;
const MAX_CACHED: usize = 20_000;
/// Characters of context kept each side of the match.
const AROUND: usize = 45;

/// file id → (modified ms, size, extracted text).
type Cache = HashMap<String, (u128, u64, Arc<str>)>;
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Default::default);

/// The route: `Some(answer)` for `/api/content-search`, `None` for anything else.
pub(crate) async fn route(app: &Shared, m: &str, p: &[String], q: &HashMap<String, String>, me: Option<User>) -> Option<Response<Body>> {
    if p.len() != 1 || p[0] != "content-search" {
        return None;
    }
    if m != "GET" {
        return Some(error(405, "use GET"));
    }
    let text = q.get("q").cloned().unwrap_or_default();
    let found = blocking(app, move |a| search(&a.store, a.auth, me.as_ref(), &text)).await;
    Some(match found {
        Some(Ok(v)) => send_json(&v, 200),
        Some(Err((code, why))) => error(code, why),
        None => error(500, "server error"),
    })
}

/// `{hits: [{id, snippet}]}`: the files `me` can see whose text contains `text`.
pub(crate) fn search(store: &Store, auth: bool, me: Option<&User>, text: &str) -> Result<Value, (u16, &'static str)> {
    if auth && me.is_none() {
        return Err((401, "please sign in"));
    }
    let needle = fold(text.trim());
    if needle.len() < 2 {
        return Ok(json!({ "hits": [] }));
    }
    if needle.len() > 100 {
        return Err((400, "That search is too long."));
    }
    let a = Access::new(store, auth, me);
    let mut files: Vec<(&String, &String)> = a.file_paths.iter().filter(|(_, p)| searchable(p)).collect();
    files.sort_by(|x, y| x.1.cmp(y.1));
    let mut hits = Vec::new();
    for (id, path) in files {
        if hits.len() >= MAX_HITS {
            break;
        }
        if !a.file_listed(id) || a.file_role(id) < Role::View {
            continue;
        }
        let Some(body) = text_of(store, id, path) else { continue };
        if let Some(snippet) = snippet(&body, &needle) {
            hits.push(json!({ "id": id, "snippet": snippet }));
        }
    }
    Ok(json!({ "hits": hits }))
}

fn ext_of(path: &str) -> String {
    path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

fn searchable(path: &str) -> bool {
    !path.split('/').any(|p| p.starts_with('.'))
        && matches!(ext_of(path).as_str(), "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" | "txt" | "md" | "csv" | "tsv" | "json")
}

/// The file's text, from the cache when its size and time are unchanged.
fn text_of(store: &Store, id: &str, path: &str) -> Option<Arc<str>> {
    let full = store.safe_file(path)?;
    let meta = std::fs::metadata(&full).ok()?;
    if meta.len() > MAX_FILE {
        return None;
    }
    let stamp = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    if let Some((t, n, text)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).get(id)
        && (*t, *n) == (stamp, meta.len())
    {
        return Some(text.clone());
    }
    let text: Arc<str> = extract(&full, &ext_of(path)).unwrap_or_default().into();
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= MAX_CACHED {
        cache.clear();
    }
    cache.insert(id.to_string(), (stamp, meta.len(), text.clone()));
    Some(text)
}

fn extract(full: &std::path::Path, ext: &str) -> Option<String> {
    let raw = match ext {
        "txt" | "md" | "csv" | "tsv" | "json" => {
            let mut buf = Vec::new();
            std::fs::File::open(full).ok()?.take(MAX_TEXT as u64).read_to_end(&mut buf).ok()?;
            String::from_utf8_lossy(&buf).into_owned()
        }
        _ => zip_text(full, ext)?,
    };
    // One line of single spaces, so a snippet reads cleanly and a match can't hide in a break.
    let mut out = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.len() > MAX_TEXT {
        let mut cut = MAX_TEXT;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    Some(out)
}

/// The text of an Office / OpenDocument file: its XML parts, paragraph by paragraph.
fn zip_text(full: &std::path::Path, ext: &str) -> Option<String> {
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(std::fs::File::open(full).ok()?)).ok()?;
    let mut names: Vec<String> = zip
        .file_names()
        .filter(|n| match ext {
            "docx" => *n == "word/document.xml",
            "xlsx" => *n == "xl/sharedStrings.xml" || (n.starts_with("xl/worksheets/") && n.ends_with(".xml")),
            "pptx" => n.starts_with("ppt/slides/slide") && n.ends_with(".xml"),
            _ => *n == "content.xml",
        })
        .map(str::to_string)
        .collect();
    names.sort();
    names.truncate(300);
    let (paras, runs): (&[&str], &[&str]) = match ext {
        "docx" | "pptx" => (&["p"], &["t"]),
        "xlsx" => (&["si", "is"], &["t"]),
        _ => (&["p", "h"], &[]),
    };
    let mut budget = MAX_XML;
    let mut out = String::new();
    for name in names {
        if budget == 0 || out.len() > MAX_TEXT {
            break;
        }
        let mut xml = String::new();
        let Ok(mut part) = zip.by_name(&name) else { continue };
        if part.by_ref().take(budget).read_to_string(&mut xml).is_err() {
            continue;
        }
        drop(part);
        budget = budget.saturating_sub(xml.len() as u64);
        let Ok(doc) = roxmltree::Document::parse(&xml) else { continue };
        for para in doc.descendants().filter(|n| n.is_element() && paras.contains(&n.tag_name().name())) {
            for t in para.descendants().filter(|n| n.is_text()) {
                let under = t.parent().map(|p| p.tag_name().name()).unwrap_or("");
                if runs.is_empty() || runs.contains(&under) {
                    out.push_str(t.text().unwrap_or(""));
                }
            }
            out.push(' ');
        }
    }
    Some(out)
}

/// Lower-case, one char for one char (so positions line up with the original text).
fn fold(s: &str) -> Vec<char> {
    s.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}

/// A short piece of `text` around the first match of `needle`, with "…" where it is cut.
fn snippet(text: &str, needle: &[char]) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let low = fold(text);
    let at = low.windows(needle.len()).position(|w| w == needle)?;
    let from = at.saturating_sub(AROUND);
    let to = (at + needle.len() + AROUND).min(chars.len());
    let mut s: String = chars[from..to].iter().collect();
    if from > 0 {
        s.insert(0, '…');
    }
    if to < chars.len() {
        s.push('…');
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{auth, sharing, store};
    use std::io::Write;

    fn zipped(path: &std::path::Path, parts: &[(&str, &str)]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, xml) in parts {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(xml.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }

    fn docx(path: &std::path::Path, runs: &[&str]) {
        let body: String = runs.iter().map(|r| format!("<w:r><w:t>{r}</w:t></w:r>")).collect();
        let xml = format!(r#"<w:document xmlns:w="w"><w:body><w:p>{body}</w:p></w:body></w:document>"#);
        zipped(path, &[("word/document.xml", &xml)]);
    }

    #[test]
    fn text_comes_out_of_each_kind_of_file() {
        let dir = std::env::temp_dir().join(format!("starsheets-cs-{}", store::random_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        // A word split over two runs is still one word.
        docx(&dir.join("a.docx"), &["The kan", "garoo jumped"]);
        zipped(&dir.join("b.xlsx"), &[("xl/sharedStrings.xml", r#"<sst xmlns="x"><si><t>Quarterly Kangaroo</t></si></sst>"#)]);
        zipped(
            &dir.join("c.pptx"),
            &[("ppt/slides/slide1.xml", r#"<p:sld xmlns:p="p" xmlns:a="a"><a:p><a:r><a:t>Slide kangaroo</a:t></a:r></a:p></p:sld>"#)],
        );
        zipped(&dir.join("d.odt"), &[("content.xml", r#"<o xmlns:t="t"><t:p>An <t:span>ODF</t:span> kangaroo</t:p></o>"#)]);
        std::fs::write(dir.join("e.md"), "# Notes\n\nthe  Kangaroo\nhops").unwrap();
        let got: Vec<String> = ["a.docx", "b.xlsx", "c.pptx", "d.odt", "e.md"].iter().map(|f| extract(&dir.join(f), &ext_of(f)).unwrap()).collect();
        assert_eq!(got[0], "The kangaroo jumped");
        assert!(got[1..].iter().all(|t| t.to_lowercase().contains("kangaroo")), "{got:?}");
        assert_eq!(got[3], "An ODF kangaroo");
        assert_eq!(got[4], "# Notes the Kangaroo hops");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snippets_are_short_and_marked_where_cut() {
        let long = format!("{} Kangaroo {}", "a".repeat(200), "b".repeat(200));
        let s = snippet(&long, &fold("kangaroo")).unwrap();
        assert!(s.starts_with('…') && s.ends_with('…') && s.contains("Kangaroo") && s.chars().count() < 120, "{s}");
        assert_eq!(snippet("a kangaroo", &fold("kangaroo")).as_deref(), Some("a kangaroo"));
        assert_eq!(snippet("nothing here", &fold("kangaroo")), None);
    }

    /// Through the real router: signed-in people, an owner's document, a stranger, a person it is shared with.
    #[tokio::test]
    async fn only_people_who_can_open_a_file_find_its_words() {
        let (app, ip, boss) = crate::tests::owned().await;
        for u in ["ann", "bob", "cy"] {
            let g = crate::tests::post(&app, &ip, "/api/users", &boss, json!({"username": u, "name": u, "password": format!("{u}password")})).await;
            assert_eq!(g.status, 200);
        }
        let login = |u: &'static str| {
            let (app, ip) = (app.clone(), ip.clone());
            async move {
                crate::tests::cookie_of(
                    &crate::tests::post(&app, &ip, "/api/login", "", json!({"username": u, "password": format!("{u}password")})).await,
                )
            }
        };
        let (ann, bob, cy) = (login("ann").await, login("bob").await, login("cy").await);
        docx(&app.store.full("ann/Plan.docx"), &["A kangaroo in the plan"]);
        std::fs::write(app.store.full("ann/Other.txt"), "nothing to see").unwrap();
        let ids: HashMap<String, String> = app.store.synced_index().unwrap().into_iter().map(|(id, p)| (p, id)).collect();
        let id = ids["ann/Plan.docx"].clone();
        let ask = |who: &str, q: &str| {
            let (app, ip, who, q) = (app.clone(), ip.clone(), who.to_string(), q.to_string());
            async move { crate::tests::send(&app, &ip, "GET", &format!("/api/content-search?q={q}"), &who, &[], vec![]).await }
        };
        let hit =
            |g: &crate::tests::Got| g.json["hits"].as_array().unwrap().iter().map(|h| h["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        // The owner of the file, and the server's owner.
        let g = ask(&ann, "KANGAROO").await;
        assert_eq!((g.status, hit(&g)), (200, vec![id.clone()]), "{:?}", g.json);
        assert!(g.json["hits"][0]["snippet"].as_str().unwrap().contains("kangaroo"));
        assert_eq!(hit(&ask(&boss, "kangaroo").await), vec![id.clone()]);
        // A stranger finds nothing.
        assert_eq!(hit(&ask(&bob, "kangaroo").await), Vec::<String>::new());
        // Shared with Bob: Bob finds it, Cy still doesn't.
        let share = sharing::Share { people: [("bob".to_string(), Role::View)].into(), link: Role::None, ..Default::default() };
        sharing::set(&app.store, sharing::Item::File(&id), &share).unwrap();
        assert_eq!(hit(&ask(&bob, "kangaroo").await), vec![id.clone()]);
        assert_eq!(hit(&ask(&cy, "kangaroo").await), Vec::<String>::new());
        // Signed out, a word too short, and a save is picked up.
        assert_eq!(ask("", "kangaroo").await.status, 401);
        assert_eq!(hit(&ask(&ann, "k").await), Vec::<String>::new());
        docx(&app.store.full("ann/Plan.docx"), &["A wombat now"]);
        assert_eq!(hit(&ask(&ann, "kangaroo").await), Vec::<String>::new());
        assert_eq!(hit(&ask(&ann, "wombat").await), vec![id]);
        let _ = auth::users(&app.store);
        let _ = std::fs::remove_dir_all(&app.store.root);
    }
}
