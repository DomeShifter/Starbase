//! docx_comments.rs: a commenter's save keeps only their comment changes. Every attack the
//! security review of PR #44 listed is tried here and must not reach the document.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]

use std::collections::BTreeMap;
use std::io::{Read, Write};

use super::*;

const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math""#;
const CT: &str = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
const ROOT_RELS: &str = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
const DOC_RELS: &str = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;

fn me() -> Me {
    me_owning(&[])
}

fn me_owning(own: &[u32]) -> Me {
    Me {
        name: "Mal".into(),
        initials: "M".into(),
        date: "2026-11-25T10:00:00Z".into(),
        own: own.iter().map(|id| (*id, "2026-11-25T10:00:00Z".to_string())).collect(),
        alias: BTreeMap::new(),
    }
}

fn zip_of(parts: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in parts {
        z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap().into_inner()
}

fn document(body: &str) -> Vec<u8> {
    format!(r#"<?xml version="1.0"?><w:document {NS}><w:body>{body}</w:body></w:document>"#).into_bytes()
}

/// The server's file: `body`, plus standard parts and `extra`.
fn server(body: &str, extra: &[(&str, &str)]) -> Vec<u8> {
    let mut parts: Vec<(&str, Vec<u8>)> = vec![
        ("[Content_Types].xml", CT.into()),
        ("_rels/.rels", ROOT_RELS.into()),
        ("word/_rels/document.xml.rels", DOC_RELS.into()),
        ("word/document.xml", document(body)),
        ("word/styles.xml", format!("<w:styles {NS}/>").into_bytes()),
        ("word/settings.xml", format!("<w:settings {NS}/>").into_bytes()),
        ("docProps/core.xml", b"<cp:coreProperties xmlns:cp=\"x\"><title>Plan</title></cp:coreProperties>".to_vec()),
    ];
    for (n, t) in extra {
        parts.retain(|(p, _)| p != n);
        parts.push((n, t.as_bytes().to_vec()));
    }
    zip_of(&parts)
}

/// What Starbase Docs sends: `body`, `comments` (inside w:comments) and commentsExtended
/// entries, plus `extra` parts.
fn sent(body: &str, comments: &str, ext: &str, extra: &[(&str, &str)]) -> Vec<u8> {
    let mut parts: Vec<(&str, Vec<u8>)> = vec![
        ("[Content_Types].xml", CT.into()),
        ("_rels/.rels", ROOT_RELS.into()),
        ("word/_rels/document.xml.rels", DOC_RELS.into()),
        ("word/document.xml", document(body)),
        ("word/comments.xml", format!("<w:comments {NS}>{comments}</w:comments>").into_bytes()),
        (
            "word/commentsExtended.xml",
            format!(r#"<w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml">{ext}</w15:commentsEx>"#).into_bytes(),
        ),
    ];
    for (n, t) in extra {
        parts.retain(|(p, _)| p != n);
        parts.push((n, t.as_bytes().to_vec()));
    }
    zip_of(&parts)
}

fn comment(id: u32, author: &str, text: &str, pid: &str) -> String {
    format!(r#"<w:comment w:id="{id}" w:author="{author}"><w:p w14:paraId="{pid}"><w:r><w:t>{text}</w:t></w:r></w:p></w:comment>"#)
}

fn parts(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        out.insert(f.name().to_string(), b);
    }
    out
}

fn part(bytes: &[u8], name: &str) -> String {
    String::from_utf8(parts(bytes).remove(name).unwrap_or_default()).unwrap()
}

fn texts(xml: &str) -> Vec<String> {
    let d = parse(xml).unwrap();
    walk(&d).unwrap().0.iter().map(|p| p.text.clone()).collect()
}

/// The merge result, checked: only the comment parts (and their relationship and content type
/// when new) differ from the server's file, and the text reads the same.
fn merged(s: &[u8], u: &[u8]) -> Vec<u8> {
    merged_as(s, u, &me())
}

fn merged_as(s: &[u8], u: &[u8], me: &Me) -> Vec<u8> {
    let out = merge(s, u, me).unwrap().expect("a change").bytes;
    let (before, after) = (parts(s), parts(&out));
    let may_change = ["word/document.xml", "word/comments.xml", "word/commentsExtended.xml", "word/_rels/document.xml.rels", "[Content_Types].xml"];
    for (name, bytes) in &after {
        match before.get(name) {
            Some(b) if !may_change.contains(&name.as_str()) => assert_eq!(b, bytes, "{name} changed"),
            None => assert!(may_change.contains(&name.as_str()), "{name} added"),
            _ => {}
        }
    }
    assert_eq!(before.keys().filter(|n| !after.contains_key(*n)).count(), 0, "nothing removed");
    let (sd, od) = (part(s, "word/document.xml"), part(&out, "word/document.xml"));
    assert_eq!(texts(&sd), texts(&od), "the text reads the same");
    out
}

const PLAIN: &str = r#"<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Hello world</w:t></w:r></w:p><w:p><w:r><w:t>Second</w:t></w:r></w:p>"#;
/// "world" commented (id 5), as Starbase Docs writes it.
const MARKED: &str = r#"<w:p><w:r><w:t xml:space="preserve">Hello </w:t></w:r><w:commentRangeStart w:id="5"/><w:r><w:t>world</w:t></w:r><w:commentRangeEnd w:id="5"/><w:r><w:commentReference w:id="5"/></w:r></w:p><w:p><w:r><w:t>Second</w:t></w:r></w:p>"#;

#[test]
fn a_new_comment_lands_on_the_servers_text_with_the_servers_author() {
    let s = server(PLAIN, &[]);
    let u = sent(MARKED, &comment(5, "Ann (forged)", "Nice", "10000001"), r#"<w15:commentEx w15:paraId="10000001" w15:done="0"/>"#, &[]);
    let out = merged(&s, &u);
    assert_eq!(merge(&s, &u, &me()).unwrap().unwrap().ids, [(5, 0)].into_iter().collect(), "sent as 5, the server's first id is 0");
    let d = part(&out, "word/document.xml");
    // The run is split where the comment starts, both halves keep their bold.
    assert!(d.contains(r#"<w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">Hello </w:t></w:r><w:commentRangeStart w:id="0"/><w:r><w:rPr><w:b/></w:rPr><w:t xml:space="preserve">world</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r>"#), "{d}");
    let c = part(&out, "word/comments.xml");
    assert!(c.contains(r#"w:author="Mal""#) && c.contains(r#"w:initials="M""#) && c.contains(">Nice<") && !c.contains("forged"), "{c}");
    assert!(part(&out, "word/_rels/document.xml.rels").contains("relationships/comments\" Target=\"comments.xml\""));
    assert!(part(&out, "[Content_Types].xml").contains("/word/comments.xml"));
    assert!(part(&out, "word/commentsExtended.xml").contains("w15:done=\"0\""));
    // Opened again (the server's ids) and saved unchanged: nothing written.
    let again = sent(
        &MARKED.replace("w:id=\"5\"", "w:id=\"0\""),
        &comment(0, "Mal", "Nice", "10000001"),
        r#"<w15:commentEx w15:paraId="10000001" w15:done="0"/>"#,
        &[],
    );
    assert_eq!(merge(&out, &again, &me()).unwrap(), None, "the same comment again is not new");
}

#[test]
fn comment_marks_at_a_paragraphs_end_and_in_an_empty_paragraph() {
    let s = server(r#"<w:p><w:r><w:t>One</w:t></w:r></w:p><w:p/><w:p><w:r><w:t>Two</w:t></w:r></w:p>"#, &[]);
    let u = sent(
        r#"<w:p><w:r><w:t>One</w:t></w:r><w:commentRangeStart w:id="1"/></w:p><w:p><w:commentRangeEnd w:id="1"/></w:p><w:p><w:r><w:t>Two</w:t></w:r></w:p>"#,
        &comment(1, "x", "Gap", "10000001"),
        "",
        &[],
    );
    let out = merged(&s, &u);
    let d = part(&out, "word/document.xml");
    assert!(d.contains(r#"<w:t>One</w:t></w:r><w:commentRangeStart w:id="0"/></w:p><w:p><w:commentRangeEnd w:id="0"/>"#), "{d}");
}

/// Each attack goes with a real new comment, so the save is otherwise accepted.
fn attack(body: &str, extra: &[(&str, &str)]) -> Vec<u8> {
    let s = server(PLAIN, &[]);
    let u = sent(body, &comment(5, "Mal", "Nice", "10000001"), "", extra);
    merged(&s, &u)
}

fn nowhere(out: &[u8], needle: &str) {
    for (name, bytes) in parts(out) {
        assert!(!String::from_utf8_lossy(&bytes).contains(needle), "{needle:?} reached {name}");
    }
}

#[test]
fn text_formatting_and_alternate_content_in_the_sent_body_are_ignored() {
    // Word shows mc:Fallback where it can't read the Choice; the server's body is kept as it is.
    let ac = MARKED.replace(
        "<w:t>Second</w:t>",
        "<w:t>Second</w:t></w:r><mc:AlternateContent><mc:Choice Requires=\"w14\"><w:r><w:t>EVILC</w:t></w:r></mc:Choice><mc:Fallback><w:r><w:t>EVILF</w:t></w:r></mc:Fallback></mc:AlternateContent><w:r><w:t/>",
    );
    let out = attack(&ac, &[]);
    nowhere(&out, "EVIL");
    // Hidden, coloured and resized text; an altChunk; a text change after the comment.
    let fmt = MARKED
        .replace("<w:t>world</w:t>", "<w:rPr><w:vanish/><w:color w:val=\"FFFFFF\"/><w:sz w:val=\"2\"/></w:rPr><w:t>world</w:t>")
        .replace("<w:t>Second</w:t>", "<w:t>Second</w:t></w:r><w:r><w:t>EVILT</w:t>")
        + "<w:altChunk r:id=\"rId9\"/>";
    let out = attack(&fmt, &[("word/afchunk.html", "<p>EVILA</p>")]);
    for needle in ["vanish", "FFFFFF", "w:sz", "altChunk", "EVIL"] {
        nowhere(&out, needle);
    }
}

#[test]
fn other_parts_and_relationships_in_the_sent_file_are_ignored() {
    let evil_rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId7" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/oleObject" Target="http://evil.example/x.bin" TargetMode="External"/><Relationship Id="rId8" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="http://evil.example/p.png" TargetMode="External"/><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="http://evil.example/" TargetMode="External"/><Relationship Id="rId10" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header9.xml"/><Relationship Id="rId11" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="vbaProject.bin"/></Relationships>"#;
    let out = attack(
        MARKED,
        &[
            ("word/_rels/document.xml.rels", evil_rels),
            (
                "word/_rels/settings.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/attachedTemplate" Target="http://evil.example/t.dotm" TargetMode="External"/></Relationships>"#,
            ),
            (
                "word/settings.xml",
                "<w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:documentProtection w:edit=\"readOnly\" w:enforcement=\"1\"/><w:attachedTemplate r:id=\"rId1\"/></w:settings>",
            ),
            ("word/vbaProject.bin", "EVILVBA"),
            ("customXml/item1.xml", "<x>EVILXML</x>"),
            ("docProps/core.xml", "<cp:coreProperties xmlns:cp=\"x\"><title>EVILTITLE</title></cp:coreProperties>"),
            (
                "word/header9.xml",
                "<w:hdr xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:p><w:r><w:t>EVILH</w:t></w:r></w:p></w:hdr>",
            ),
            (
                "word/numbering.xml",
                "<w:numbering xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:lvlText w:val=\"EVILN\"/></w:numbering>",
            ),
            ("word/media/image1.png", "EVILPNG"),
        ],
    );
    for needle in ["evil.example", "documentProtection", "attachedTemplate", "EVIL", "vbaProject", "customXml", "header9"] {
        nowhere(&out, needle);
    }
}

#[test]
fn a_new_comment_is_plain_text_only() {
    let s = server(PLAIN, &[]);
    let rich = r#"<w:comment w:id="5" w:author="Mal"><w:p><w:hyperlink r:id="rId9"><w:r><w:rPr><w:vanish/></w:rPr><w:t>Link</w:t></w:r></w:hyperlink><w:r><w:drawing><a r:embed="rId8"/></w:drawing></w:r></w:p><w:altChunk r:id="rId3"/></w:comment>"#;
    let out = merged(&s, &sent(MARKED, rich, "", &[]));
    let c = part(&out, "word/comments.xml");
    assert!(c.contains(">Link<"), "{c}");
    for needle in ["hyperlink", "r:id", "r:embed", "vanish", "drawing", "altChunk"] {
        assert!(!c.contains(needle), "{needle} in {c}");
    }
}

#[test]
fn other_peoples_comments_stay_as_they_are() {
    let marked_ann = r#"<w:p><w:commentRangeStart w:id="0"/><w:r><w:t>Hello world</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r></w:p><w:p><w:r><w:t>Second</w:t></w:r></w:p>"#;
    let ann = comment(0, "Ann", "Ann says", "00000AAA");
    let s = server(
        marked_ann,
        &[
            ("word/comments.xml", &format!("<w:comments {NS}>{ann}</w:comments>")),
            (
                "word/commentsExtended.xml",
                r#"<w15:commentsEx xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w15:commentEx w15:paraId="00000AAA" w15:done="0"/></w15:commentsEx>"#,
            ),
            (
                "word/_rels/document.xml.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/><Relationship Id="rId3" Type="http://schemas.microsoft.com/office/2011/relationships/commentsExtended" Target="commentsExtended.xml"/></Relationships>"#,
            ),
        ],
    );
    // Deleted, edited or resolved by the commenter: nothing to do.
    let none = sent(PLAIN, "", "", &[]);
    assert_eq!(merge(&s, &none, &me()).unwrap(), None, "can't delete Ann's");
    let edited = sent(marked_ann, &comment(0, "Ann", "EVIL", "10000001"), r#"<w15:commentEx w15:paraId="10000001" w15:done="1"/>"#, &[]);
    assert_eq!(merge(&s, &edited, &me()).unwrap(), None, "can't edit or resolve Ann's");
    // A reply to Ann's (no marks of its own): beside her marks, linked to her comment.
    let reply = sent(
        marked_ann,
        &format!("{}{}", comment(0, "Ann", "Ann says", "10000001"), comment(1, "Mal", "Agreed", "10000002")),
        r#"<w15:commentEx w15:paraId="10000001" w15:done="0"/><w15:commentEx w15:paraId="10000002" w15:paraIdParent="10000001" w15:done="0"/>"#,
        &[],
    );
    let out = merged(&s, &reply);
    let d = part(&out, "word/document.xml");
    assert!(d.contains(r#"<w:commentRangeStart w:id="0"/><w:commentRangeStart w:id="1"/>"#), "{d}");
    assert!(d.contains(r#"<w:r><w:commentReference w:id="0"/></w:r><w:commentRangeEnd w:id="1"/><w:r><w:commentReference w:id="1"/></w:r>"#), "{d}");
    let x = part(&out, "word/commentsExtended.xml");
    assert!(x.contains(r#"w15:paraIdParent="00000AAA""#), "{x}");
    assert!(part(&out, "word/comments.xml").contains("Ann says"));
    // Only a comment the server recorded as Mal's is his: one merely under his name isn't.
    let forged = sent(marked_ann, &comment(0, "Ann", "Ann says", "10000001"), r#"<w15:commentEx w15:paraId="10000001" w15:done="0"/>"#, &[]);
    assert_eq!(merge(&out, &forged, &me()).unwrap(), None, "comment 1 isn't recorded as his");
    // Recorded with another date (the id used again for someone else's comment): not his either.
    let other = Me { own: [(1, "2020-01-01T00:00:00Z".to_string())].into_iter().collect(), ..me() };
    assert_eq!(merge(&out, &forged, &other).unwrap(), None, "same id, another comment");
    // Mal now owns comment 1: may edit, resolve and delete it; Ann's stays.
    let edit = sent(
        marked_ann,
        &format!("{}{}", comment(0, "Ann", "Ann says", "10000001"), comment(1, "Mal", "Agreed, twice", "10000002")),
        r#"<w15:commentEx w15:paraId="10000001" w15:done="0"/><w15:commentEx w15:paraId="10000002" w15:paraIdParent="10000001" w15:done="1"/>"#,
        &[],
    );
    let out2 = merged_as(&out, &edit, &me_owning(&[1]));
    let c = part(&out2, "word/comments.xml");
    assert!(c.contains("Agreed, twice") && c.contains("Ann says") && !c.contains(">Agreed<"), "{c}");
    let x = part(&out2, "word/commentsExtended.xml");
    assert!(x.contains(r#"w15:paraIdParent="00000AAA" w15:done="1""#), "resolved, still a reply: {x}");
    let gone = sent(marked_ann, &comment(0, "Ann", "Ann says", "10000001"), r#"<w15:commentEx w15:paraId="10000001" w15:done="0"/>"#, &[]);
    let out3 = merged_as(&out2, &gone, &me_owning(&[1]));
    let (c, d) = (part(&out3, "word/comments.xml"), part(&out3, "word/document.xml"));
    assert!(!c.contains("Agreed") && c.contains("Ann says"), "{c}");
    assert!(!d.contains(r#"w:id="1""#) && d.contains(r#"w:id="0""#), "{d}");
}

#[test]
fn a_changed_text_before_the_comment_is_refused() {
    let s = server(PLAIN, &[]);
    let u = sent(&MARKED.replace("Hello ", "Goodbye "), &comment(5, "Mal", "Nice", "10000001"), "", &[]);
    assert!(merge(&s, &u, &me()).is_err());
}

#[test]
fn damaged_or_doubled_parts_are_refused() {
    let s = server(PLAIN, &[]);
    let good = sent(MARKED, &comment(5, "Mal", "Nice", "10000001"), "", &[]);
    // Two parts named alike, differing only in case.
    let case = sent(MARKED, &comment(5, "Mal", "Nice", "10000001"), "", &[("Word/document.xml", "<x/>")]);
    assert!(merge(&s, &case, &me()).is_err());
    // Two parts with exactly the same name (the second name patched in after writing).
    let twice = sent(MARKED, &comment(5, "Mal", "Nice", "10000001"), "", &[("word/document.xmX", "<x/>")]);
    let twice = twice.windows(12).enumerate().filter(|(_, w)| *w == b"document.xmX").map(|(i, _)| i).collect::<Vec<_>>().iter().fold(
        twice.clone(),
        |mut b, i| {
            b[i + 11] = b'l';
            b
        },
    );
    assert!(merge(&s, &twice, &me()).is_err());
    assert!(merge(&s, b"not a zip", &me()).is_err());
    assert!(merge(b"not a zip", &good, &me()).is_err());
    let deep = format!("<w:p>{}{}</w:p>", "<w:ins>".repeat(300), "</w:ins>".repeat(300));
    assert!(merge(&s, &sent(&deep, &comment(5, "Mal", "Nice", "10000001"), "", &[]), &me()).is_err());
    let long = comment(5, "Mal", &"x".repeat(MAX_COMMENT_CHARS + 1), "10000001");
    assert!(merge(&s, &sent(MARKED, &long, "", &[]), &me()).is_err());
}

/// Files as Starbase Docs (WordCraft) really writes them: the server's document, and the same
/// opened, "world" commented and the comment replied to (a reply has no marks of its own).
#[test]
fn a_comment_and_reply_as_starbase_docs_writes_them() {
    let s: &[u8] = include_bytes!("testdata/wordcraft-s.docx");
    let u: &[u8] = include_bytes!("testdata/wordcraft-u.docx");
    let out = merged(s, u);
    let d = part(&out, "word/document.xml");
    assert!(
        d.contains(r#"<w:t xml:space="preserve">Hello </w:t></w:r><w:commentRangeStart w:id="0"/><w:commentRangeStart w:id="1"/><w:r><w:rPr><w:b/><w:bCs/></w:rPr><w:t xml:space="preserve">world</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r><w:commentRangeEnd w:id="1"/><w:r><w:commentReference w:id="1"/></w:r></w:p>"#),
        "{d}"
    );
    let c = part(&out, "word/comments.xml");
    assert!(c.contains(">Nice<") && c.contains(">Agreed<") && !c.contains("WordCraft User"), "{c}");
    let x = part(&out, "word/commentsExtended.xml");
    assert_eq!(x.matches("paraIdParent").count(), 1, "{x}");
    // Saved again unchanged: nothing to do.
    assert_eq!(merge(&out, u, &me_owning(&[0, 1])).unwrap(), None);
}

/// Second review: threads that can't be, many paragraphs, a parent deleted under a new reply,
/// marks beside math, and spaces Word drops at the ends of text.
#[test]
fn threads_paragraphs_math_and_spaces() {
    let s = server(PLAIN, &[]);
    // A comment of 201 empty paragraphs (no characters, but each one written out).
    let many = format!(r#"<w:comment w:id="5" w:author="Mal">{}</w:comment>"#, "<w:p/>".repeat(MAX_COMMENT_PARAS + 1));
    assert!(merge(&s, &sent(MARKED, &many, "", &[]), &me()).is_err());
    // Its own parent: kept as a comment of its own, no thread.
    let selfish =
        sent(MARKED, &comment(5, "Mal", "Me", "10000001"), r#"<w15:commentEx w15:paraId="10000001" w15:paraIdParent="10000001" w15:done="0"/>"#, &[]);
    let out = merged(&s, &selfish);
    assert!(!part(&out, "word/commentsExtended.xml").contains("paraIdParent"));
    // Math (m:r holds w:t): the marks go before the math, never inside it.
    let math = r#"<w:p><m:oMath><m:r><w:t>x</w:t></m:r></m:oMath><w:r><w:t>y</w:t></w:r></w:p>"#;
    let s2 = server(math, &[]);
    let u2 = sent(
        r#"<w:p><w:commentRangeStart w:id="1"/><m:oMath><m:r><w:t>x</w:t></m:r></m:oMath><w:r><w:t>y</w:t></w:r><w:commentRangeEnd w:id="1"/></w:p>"#,
        &comment(1, "Mal", "Sum", "10000001"),
        "",
        &[],
    );
    let d = part(&merged(&s2, &u2), "word/document.xml");
    assert!(d.contains(r#"<w:p><w:commentRangeStart w:id="0"/><m:oMath>"#), "{d}");
    // Without xml:space="preserve" Word drops the text's outer spaces: the halves do too.
    let spaced = r#"<w:p><w:r><w:t> Hello world </w:t></w:r></w:p>"#;
    let s3 = server(spaced, &[]);
    let u3 = sent(
        r#"<w:p><w:r><w:t xml:space="preserve">Hello </w:t></w:r><w:commentRangeStart w:id="1"/><w:r><w:t>world</w:t></w:r><w:commentRangeEnd w:id="1"/></w:p>"#,
        &comment(1, "Mal", "Hi", "10000001"),
        "",
        &[],
    );
    let d = part(&merged(&s3, &u3), "word/document.xml");
    assert!(
        d.contains(r#"<w:t xml:space="preserve">Hello </w:t></w:r><w:commentRangeStart w:id="0"/><w:r><w:t xml:space="preserve">world</w:t>"#),
        "{d}"
    );
}

/// Review of PR #56 (M1): a new comment gets the server's id, one past the largest used in any
/// part (marks without a comment, a header's marks), so it can't take or touch anyone else's.
#[test]
fn new_comments_get_the_servers_ids() {
    let marked_ann = r#"<w:p><w:commentRangeStart w:id="3"/><w:r><w:t>Hello world</w:t></w:r><w:commentRangeEnd w:id="3"/><w:r><w:commentReference w:id="3"/></w:r></w:p><w:p><w:r><w:t>Second</w:t></w:r></w:p>"#;
    let ann = comment(3, "Ann", "Ann says", "00000AAA");
    let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/></Relationships>"#;
    let comments = format!("<w:comments {NS}>{ann}</w:comments>");
    // A header holding marks of comment 9 (no such comment): new ids go past it.
    let header = format!(r#"<w:hdr {NS}><w:p><w:commentRangeStart w:id="9"/><w:r><w:t>Head</w:t></w:r><w:commentRangeEnd w:id="9"/></w:p></w:hdr>"#);
    let s = server(marked_ann, &[("word/comments.xml", &comments), ("word/_rels/document.xml.rels", rels), ("word/header1.xml", &header)]);
    // Two new comments sent as 3 (Ann's id: not new, ignored) and 4 and 9.
    let body = marked_ann.replace(
        "<w:r><w:t>Second</w:t></w:r>",
        r#"<w:commentRangeStart w:id="4"/><w:commentRangeStart w:id="9"/><w:r><w:t>Second</w:t></w:r><w:commentRangeEnd w:id="4"/><w:commentRangeEnd w:id="9"/>"#,
    );
    let u = sent(
        &body,
        &format!("{}{}{}", comment(3, "Ann", "EVIL", "10000001"), comment(4, "Mal", "Four", "10000002"), comment(9, "Mal", "Nine", "10000003")),
        "",
        &[],
    );
    let m = merge(&s, &u, &me()).unwrap().unwrap();
    assert_eq!(m.ids, [(4, 10), (9, 11)].into_iter().collect());
    let mut own: Vec<u32> = m.own.keys().copied().collect();
    own.sort_unstable();
    assert_eq!(own, vec![10, 11], "the server's ids are Mal's");
    let out = merged(&s, &u);
    let c = part(&out, "word/comments.xml");
    assert!(c.contains(r#"w:id="10""#) && c.contains(r#"w:id="11""#) && c.contains("Ann says") && !c.contains("EVIL"), "{c}");
    assert!(!c.contains(r#"w:id="4""#) && !c.contains(r#"w:id="9""#), "{c}");
    let d = part(&out, "word/document.xml");
    assert!(d.contains(r#"<w:commentRangeStart w:id="10"/>"#) && d.contains(r#"<w:commentRangeStart w:id="11"/>"#), "{d}");
    assert_eq!(d.matches(r#"w:id="3""#).count(), 3, "Ann's marks as they were: {d}");
    assert_eq!(part(&out, "word/header1.xml"), header, "the header as it was");
    // A file that already uses the largest id: refused, never wrapped round to 0.
    let full = server(PLAIN, &[("word/header1.xml", &header.replace("w:id=\"9\"", "w:id=\"4294967295\""))]);
    assert!(merge(&full, &sent(MARKED, &comment(5, "Mal", "Nice", "10000001"), "", &[]), &me()).is_err());
}

/// Review of PR #56 (M2): editing your own comment keeps its date, so it stays yours, saved
/// again and again; a line break in it is a w:br.
#[test]
fn an_edited_comment_keeps_its_date_and_stays_yours() {
    let s = server(PLAIN, &[]);
    let first = Me { date: "2026-11-25T09:00:00Z".into(), ..me() };
    let m = merge(&s, &sent(&MARKED.replace("w:id=\"5\"", "w:id=\"0\""), &comment(0, "Mal", "One", "10000001"), "", &[]), &first).unwrap().unwrap();
    assert_eq!(m.own.get(&0).map(String::as_str), Some("2026-11-25T09:00:00Z"));
    let mut doc = m.bytes;
    let mut own = m.own;
    for (i, text) in ["Two", "Three"].iter().enumerate() {
        let later = Me { date: format!("2026-11-25T1{i}:00:00Z"), own: own.clone(), ..me() };
        let u = sent(&MARKED.replace("w:id=\"5\"", "w:id=\"0\""), &comment(0, "Mal", text, "10000001"), "", &[]);
        let m = merge(&doc, &u, &later).unwrap().expect("an edit of Mal's own comment");
        let c = part(&m.bytes, "word/comments.xml");
        assert!(c.contains(&format!(">{text}<")) && c.contains(r#"w:date="2026-11-25T09:00:00Z""#), "{c}");
        assert_eq!(m.own.get(&0).map(String::as_str), Some("2026-11-25T09:00:00Z"), "still his");
        (doc, own) = (m.bytes, m.own);
    }
    // A line break.
    let later = Me { own: own.clone(), ..me() };
    let br = r#"<w:comment w:id="0" w:author="Mal"><w:p w14:paraId="10000001"><w:r><w:t>Line</w:t><w:br/><w:t>next</w:t></w:r></w:p></w:comment>"#;
    let m = merge(&doc, &sent(&MARKED.replace("w:id=\"5\"", "w:id=\"0\""), br, "", &[]), &later).unwrap().unwrap();
    let c = part(&m.bytes, "word/comments.xml");
    assert!(c.contains(r#"Line</w:t><w:br/><w:t xml:space="preserve">next"#), "{c}");
}

/// Review of PR #56 (low): a deleted comment's entries in commentsIds, commentsExtensible and
/// (its author having no comment left) people.xml go with it.
#[test]
fn a_deleted_comment_leaves_no_entries_behind() {
    let marked = MARKED.replace("w:id=\"5\"", "w:id=\"0\"");
    let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/><Relationship Id="rId3" Type="http://schemas.microsoft.com/office/2016/09/relationships/commentsIds" Target="commentsIds.xml"/><Relationship Id="rId4" Type="http://schemas.microsoft.com/office/2018/08/relationships/commentsExtensible" Target="commentsExtensible.xml"/><Relationship Id="rId5" Type="http://schemas.microsoft.com/office/2011/relationships/people" Target="people.xml"/></Relationships>"#;
    let mine =
        r#"<w:comment w:id="0" w:author="Mal" w:date="2026-11-25T10:00:00Z"><w:p w14:paraId="0000000A"><w:r><w:t>Mine</w:t></w:r></w:p></w:comment>"#;
    let ids = r#"<w16cid:commentsIds xmlns:w16cid="http://schemas.microsoft.com/office/word/2016/wordml/cid"><w16cid:commentId w16cid:paraId="0000000A" w16cid:durableId="1111AAAA"/><w16cid:commentId w16cid:paraId="0000000B" w16cid:durableId="2222BBBB"/></w16cid:commentsIds>"#;
    let ext = r#"<w16cex:commentsExtensible xmlns:w16cex="http://schemas.microsoft.com/office/word/2018/wordml/cex"><w16cex:commentExtensible w16cex:durableId="1111AAAA" w16cex:dateUtc="2026-11-25T10:00:00Z"/><w16cex:commentExtensible w16cex:durableId="2222BBBB"/></w16cex:commentsExtensible>"#;
    let people = r#"<w15:people xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w15:person w15:author="Mal"/><w15:person w15:author="Ann"/></w15:people>"#;
    let comments = format!("<w:comments {NS}>{mine}</w:comments>");
    let s = server(
        &marked,
        &[
            ("word/comments.xml", &comments),
            ("word/_rels/document.xml.rels", rels),
            ("word/commentsIds.xml", ids),
            ("word/commentsExtensible.xml", ext),
            ("word/people.xml", people),
        ],
    );
    let out = merge(&s, &sent(PLAIN, "", "", &[]), &me_owning(&[0])).unwrap().unwrap().bytes;
    let (i, x, p) = (part(&out, "word/commentsIds.xml"), part(&out, "word/commentsExtensible.xml"), part(&out, "word/people.xml"));
    assert!(!i.contains("0000000A") && i.contains("0000000B"), "{i}");
    assert!(!x.contains("1111AAAA") && x.contains("2222BBBB"), "{x}");
    assert!(!p.contains("\"Mal\"") && p.contains("\"Ann\""), "{p}");
    assert!(!part(&out, "word/comments.xml").contains("Mine"));
}

/// Review of PR #56 (2nd, M1): a save that was already waiting when the first one's reply gave
/// its comment a new id still uses the program's own ids. Named in `alias`, they are read as the
/// server's: the first comment is edited (not deleted and added again), and a newer comment that
/// happens to have the server's id is new, not taken for it.
#[test]
fn a_waiting_save_after_new_ids_keeps_every_comment() {
    let s = server(PLAIN, &[]);
    let first = merge(&s, &sent(MARKED, &comment(5, "Mal", "Nice", "10000001"), "", &[]), &me()).unwrap().unwrap();
    assert_eq!(first.ids, [(5, 0)].into_iter().collect());
    // The program still calls it 5; its next comment it called 0.
    let body =
        MARKED.replace("<w:r><w:t>Second</w:t></w:r>", r#"<w:commentRangeStart w:id="0"/><w:r><w:t>Second</w:t></w:r><w:commentRangeEnd w:id="0"/>"#);
    let u = sent(&body, &format!("{}{}", comment(5, "Mal", "Nicer", "10000001"), comment(0, "Mal", "Two", "10000002")), "", &[]);
    let later = Me { own: first.own.clone(), alias: [(5, 0)].into_iter().collect(), date: "2026-11-25T11:00:00Z".into(), ..me() };
    let m = merge(&first.bytes, &u, &later).unwrap().unwrap();
    assert_eq!(m.ids, [(0, 1)].into_iter().collect(), "its 0 is the server's 1");
    let c = part(&m.bytes, "word/comments.xml");
    assert!(c.contains(r#"w:id="0""#) && c.contains(">Nicer<") && c.contains(r#"w:id="1""#) && c.contains(">Two<"), "{c}");
    assert!(c.contains(r#"w:date="2026-11-25T10:00:00Z""#) && c.contains(r#"w:date="2026-11-25T11:00:00Z""#), "the first keeps its date: {c}");
    assert_eq!(m.own.len(), 2);
    let d = part(&m.bytes, "word/document.xml");
    assert_eq!(d.matches(r#"w:id="0""#).count(), 3, "{d}");
    assert_eq!(d.matches(r#"w:id="1""#).count(), 3, "{d}");
    // Without the alias the same file is read wrongly: the first comment would be added again
    // as new (its date lost), and the newer one's text taken for the first.
    let bad = merge(&first.bytes, &u, &Me { alias: BTreeMap::new(), ..later }).unwrap().unwrap();
    assert!(bad.ids.contains_key(&5), "{:?}", bad.ids);
    // Two ids named as one server id: refused.
    let twice = Me { alias: [(5, 0), (6, 0)].into_iter().collect(), ..me() };
    assert!(merge(&first.bytes, &u, &twice).is_err());
}

/// Review of PR #56 (2nd): Word reads comment ids as signed 32-bit numbers, and the largest id in
/// use is looked for only in the parts that hold comments, at most 64 MiB of them.
#[test]
fn ids_stay_under_i32_max_and_the_id_scan_is_bounded() {
    let header = |id: u32| format!(r#"<w:hdr {NS}><w:p><w:commentRangeStart w:id="{id}"/><w:r><w:t>Head</w:t></w:r></w:p></w:hdr>"#);
    let u = sent(MARKED, &comment(5, "Mal", "Nice", "10000001"), "", &[]);
    let ok = server(PLAIN, &[("word/header1.xml", &header(0x7FFF_FFFE))]);
    assert_eq!(merge(&ok, &u, &me()).unwrap().unwrap().ids, [(5, 0x7FFF_FFFF)].into_iter().collect());
    let full = server(PLAIN, &[("word/header1.xml", &header(0x7FFF_FFFF))]);
    assert!(merge(&full, &u, &me()).is_err(), "past i32::MAX");
    // A part that holds no comments isn't read for ids.
    let other = server(PLAIN, &[("customXml/item1.xml", &header(900))]);
    assert_eq!(merge(&other, &u, &me()).unwrap().unwrap().ids, [(5, 0)].into_iter().collect());
    // Comment parts bigger than the limit together: refused.
    let big = format!("<w:ftr {NS}>{}</w:ftr>", " ".repeat(33 << 20));
    let huge = server(PLAIN, &[("word/footer1.xml", &big), ("word/footer2.xml", &big)]);
    let e = merge(&huge, &u, &me()).unwrap_err();
    assert!(e.contains("too large"), "{e}");
    // ... but only looked at when there is a new comment.
    assert!(merge(&huge, &sent(PLAIN, "", "", &[]), &me()).unwrap().is_none());
}
