//! A commenter's comment ids against the server's (server.rs). The server gives each new comment
//! its own id (starsheets' docx_comments.rs) and its save reply says which differ:
//! `"ids": {"<ours>": <server's>}`. The editor's ids are never changed: the pairs are kept here
//! and sent with every later save in the `X-Comment-Ids` header, which the server reads them by.
//! A save already waiting when a reply comes is sent with the new pairs too, so it can't delete
//! the comment under its new id or add it a second time.

use std::collections::BTreeMap;

/// The most pairs kept: past this the document is opened again (its ids are then the server's).
pub const MAX: usize = 300;

/// The `X-Comment-Ids` header: `ours=server` pairs, comma-separated (empty when there are none).
pub fn header(alias: &BTreeMap<u32, u32>) -> String {
    alias.iter().map(|(a, b)| format!("{a}={b}")).collect::<Vec<_>>().join(",")
}

/// Takes a save reply's `ids` into `alias`. `false`: not readable, or more pairs than [`MAX`];
/// the document must then be opened again.
#[must_use]
pub fn apply(alias: &mut BTreeMap<u32, u32>, ids: &serde_json::Value) -> bool {
    let Some(ids) = ids.as_object() else { return ids.is_null() };
    for (ours, server) in ids {
        let (Ok(ours), Some(server)) = (ours.parse::<u32>(), server.as_u64().and_then(|v| u32::try_from(v).ok())) else { return false };
        // One server id names one comment of ours: an older pair for it is stale.
        alias.retain(|a, b| *a == ours || *b != server);
        if ours == server {
            alias.remove(&ours);
        } else {
            alias.insert(ours, server);
        }
    }
    alias.len() <= MAX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_s_ids_are_kept_and_sent_with_later_saves() {
        let mut alias = BTreeMap::new();
        assert!(apply(&mut alias, &serde_json::Value::Null));
        assert_eq!(header(&alias), "");
        // The first save: our 5 is the server's 0.
        assert!(apply(&mut alias, &serde_json::json!({"5": 0})));
        assert_eq!(header(&alias), "5=0");
        // A later one: our 0 (a newer comment) is the server's 1, our 7 the server's 7.
        assert!(apply(&mut alias, &serde_json::json!({"0": 1, "7": 7})));
        assert_eq!(header(&alias), "0=1,5=0");
        // Our 5 was deleted and put back (undo): now the server's 2; the server's 1 went to our 9.
        assert!(apply(&mut alias, &serde_json::json!({"5": 2, "9": 1})));
        assert_eq!(header(&alias), "5=2,9=1");
        assert!(!apply(&mut alias, &serde_json::json!({"x": 1})));
        assert!(!apply(&mut alias, &serde_json::json!([1])));
        let many: serde_json::Map<String, serde_json::Value> = (0..=MAX as u32).map(|i| (i.to_string(), serde_json::json!(i + 1000))).collect();
        assert!(!apply(&mut BTreeMap::new(), &serde_json::Value::Object(many)));
    }

    /// The pairs only hold if WordCraft keeps a comment's id from the file it opened to the file
    /// it saves (the server matches comments by id). Ids with gaps; one edited, one deleted, one
    /// added: the others keep theirs, in the comments and in their marks, saved twice over.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn wordcraft_keeps_comment_ids_on_save() {
        use std::io::{Read, Write};
        use wordcraft_engine::io::{open_bytes, save_bytes};
        const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
        let para = |id: u32, text: &str| {
            format!(
                r#"<w:p><w:commentRangeStart w:id="{id}"/><w:r><w:t>{text}</w:t></w:r><w:commentRangeEnd w:id="{id}"/><w:r><w:commentReference w:id="{id}"/></w:r></w:p>"#
            )
        };
        let comment = |id: u32, text: &str| {
            format!(
                r#"<w:comment w:id="{id}" w:author="Ann" w:initials="A" w:date="2026-11-25T10:00:00Z"><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:comment>"#
            )
        };
        let parts = [
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/></Types>"#.to_string(),
            ),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string(),
            ),
            (
                "word/_rels/document.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/></Relationships>"#.to_string(),
            ),
            (
                "word/document.xml",
                format!(
                    "<w:document {NS}><w:body>{}{}{}<w:p><w:r><w:t>Delta</w:t></w:r></w:p></w:body></w:document>",
                    para(3, "Alpha"),
                    para(7, "Beta"),
                    para(12, "Gamma")
                ),
            ),
            ("word/comments.xml", format!("<w:comments {NS}>{}{}{}</w:comments>", comment(3, "Three"), comment(7, "Seven"), comment(12, "Twelve"))),
        ];
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, text) in &parts {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(text.as_bytes()).unwrap();
        }
        let file = z.finish().unwrap().into_inner();
        let read = |bytes: &[u8], name: &str| {
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
            let mut s = String::new();
            z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
            s
        };
        // The ids on the `<w:tag …>` elements of `xml`, sorted.
        let ids = |xml: &str, tag: &str| -> Vec<u32> {
            let mut out: Vec<u32> =
                xml.split(&format!("<w:{tag} ")).skip(1).filter_map(|t| t.split("w:id=\"").nth(1)?.split('"').next()?.parse().ok()).collect();
            out.sort_unstable();
            out
        };
        let mut s = wordcraft_engine::Session::new(open_bytes("t.docx", &file).unwrap());
        assert_eq!(s.doc.comments.keys().copied().collect::<Vec<_>>(), vec![3, 7, 12], "read with the file's ids");
        s.run("review.editComment", &serde_json::json!({"id": 7, "text": "Seven, edited"})).unwrap();
        s.run("review.deleteComment", &serde_json::json!({"id": 3})).unwrap();
        s.run("select.all", &serde_json::json!({})).unwrap();
        let new = u32::try_from(s.run("review.newComment", &serde_json::json!({"text": "New"})).unwrap()["id"].as_u64().unwrap()).unwrap();
        assert!(![7, 12].contains(&new), "a new comment takes a free id: {new}");
        let mut want = vec![7, 12, new];
        want.sort_unstable();
        let saved = save_bytes("t.docx", &s.doc).unwrap();
        let (c, d) = (read(&saved, "word/comments.xml"), read(&saved, "word/document.xml"));
        assert_eq!(ids(&c, "comment"), want, "{c}");
        assert_eq!(ids(&d, "commentRangeStart"), want, "{d}");
        assert_eq!(ids(&d, "commentReference"), want, "{d}");
        assert!(c.contains("Seven, edited") && c.contains("Twelve") && !c.contains("Three"), "{c}");
        // Opened and saved again: the same ids.
        let again = save_bytes("t.docx", &open_bytes("t.docx", &saved).unwrap()).unwrap();
        assert_eq!(ids(&read(&again, "word/comments.xml"), "comment"), want);
        assert_eq!(ids(&read(&again, "word/document.xml"), "commentRangeStart"), want);
    }
}
