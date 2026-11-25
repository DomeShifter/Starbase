//! A commenter's save, rebuilt on the server (commenting.rs). The file a commenter's editor
//! sends is never kept: it is only read for the commenter's own comment changes, which are
//! written into the server's copy of the document. Every other byte of the result comes from the
//! server's copy, so nothing else a commenter sends (text, formatting, pictures, links, settings,
//! macros, properties, other parts) can reach the document.
//!
//! What is taken from the sent file ([`merge`]):
//!
//! - **New comments and replies**: comments whose id the server's copy doesn't have. Only their
//!   text is taken, written as plain paragraphs (no formatting, links or pictures); the author,
//!   initials and date are the server's (the signed-in person, now). A reply's parent comes from
//!   `commentsExtended.xml`. Where a comment sits comes from its `commentRangeStart` /
//!   `commentRangeEnd` marks in the sent document: each is turned into (paragraph number, characters
//!   before it), and the marks are put at that place in the server's document, splitting a run if
//!   they fall inside one. The paragraphs up to the last one used must read the same in both
//!   files, else the save is refused. A reply without marks of its own goes beside its parent's.
//! - **The commenter's own comments** (ones the server recorded them adding this way, still
//!   under their name; a display name alone could be copied): new text, resolved or not, or
//!   deleted (with their marks, their entries in commentsIds / commentsExtensible, and the
//!   author's entry in people.xml once they have no comment left), unless a reply that stays
//!   hangs on it. An edited comment keeps its date, which is part of how the server knows it is
//!   theirs (commenting.rs).
//! - **Ids**: the server gives every new comment its own id, one past the largest comment id
//!   used anywhere in the document (any part, marks included), whatever id the commenter's file
//!   gave it, so a new comment can never take or touch someone else's. When the ids differ from
//!   the sent ones, [`Merged::renumbered`] says so (the commenter's program opens the document
//!   again, so its ids match the server's).
//!
//! Everything else in the sent file is ignored, including changes to other people's comments.
//! After building, the server's text is checked to be unchanged (a guard against a mistake here).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Cursor, Read, Write};

use roxmltree::{Document, Node, ParsingOptions};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const REL_DOC: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const REL_COMMENTS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
const REL_COMMENTS_EX: &str = "http://schemas.microsoft.com/office/2011/relationships/commentsExtended";
const CT_COMMENTS: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml";
const CT_COMMENTS_EX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml";
const REL_COMMENTS_IDS: &str = "http://schemas.microsoft.com/office/2016/09/relationships/commentsIds";
const REL_COMMENTS_EXT: &str = "http://schemas.microsoft.com/office/2018/08/relationships/commentsExtensible";
const REL_PEOPLE: &str = "http://schemas.microsoft.com/office/2011/relationships/people";
const W16CID: &str = "http://schemas.microsoft.com/office/word/2016/wordml/cid";
const W16CEX: &str = "http://schemas.microsoft.com/office/word/2018/wordml/cex";

/// Limits on what is read: parts in a file, bytes in one part, XML nodes, nesting, comments.
const MAX_ENTRIES: usize = 10_000;
const MAX_PART: u64 = 64 * 1024 * 1024;
const MAX_NODES: u32 = 4_000_000;
const MAX_DEPTH: usize = 256;
const MAX_NEW: usize = 500;
const MAX_COMMENT_CHARS: usize = 20_000;
/// Paragraphs in one comment.
const MAX_COMMENT_PARAS: usize = 200;
/// Bytes of the sent comment parts read (they are small: a lot of text is a few hundred KB).
const MAX_COMMENT_PART: usize = 4 * 1024 * 1024;
/// How much one save may make the document bigger, in bytes.
const MAX_GROWTH: usize = 2 * 1024 * 1024;
/// Comments one person may have added to one document this way.
pub const MAX_OWNED: usize = 2_000;

/// Who is commenting, as the comments will say, and which comments are theirs.
pub struct Me {
    pub name: String,
    pub initials: String,
    /// ISO 8601, UTC.
    pub date: String,
    /// The comments the server recorded this person adding to this document: id → the date
    /// the server gave it (an id alone can be used again for someone else's comment).
    pub own: HashMap<u32, String>,
}

/// A rebuilt document, and the comments it added and deleted (for the record of whose they are).
#[derive(Debug, PartialEq)]
pub struct Merged {
    pub bytes: Vec<u8>,
    /// This person's comments in the result: id → date (the new record of whose they are).
    pub own: HashMap<u32, String>,
    /// A new comment got a different id from the one in the sent file.
    pub renumbered: bool,
}

/// The server's document with the commenter's comment changes from `sent`, or `None` when there
/// are none. `Err` says why the save can't be kept.
pub fn merge(server: &[u8], sent: &[u8], me: &Me) -> Result<Option<Merged>, String> {
    let mut szip = open(server).map_err(|e| format!("the document on the server can't be read ({e})"))?;
    let mut uzip = open(sent)?;
    // The server's parts: the main document, its comments and their extra state.
    let root_rels = read(&mut szip, "_rels/.rels")?;
    let main = root_rels.as_deref().and_then(|r| target_of(r, "", REL_DOC)).unwrap_or_else(|| "word/document.xml".into());
    let dir = main.rsplit_once('/').map_or(String::new(), |(d, _)| format!("{d}/"));
    let rels_name = format!("{dir}_rels/{}.rels", main.rsplit('/').next().unwrap_or("document.xml"));
    let doc_rels = read(&mut szip, &rels_name)?;
    let c_name = doc_rels.as_deref().and_then(|r| target_of(r, &dir, REL_COMMENTS));
    let x_name = doc_rels.as_deref().and_then(|r| target_of(r, &dir, REL_COMMENTS_EX));
    let s_doc = read(&mut szip, &main)?.ok_or("the document on the server has no text part")?;
    let s_com = match &c_name {
        Some(n) => read(&mut szip, n)?,
        None => None,
    };
    let s_ext = match &x_name {
        Some(n) => read(&mut szip, n)?,
        None => None,
    };
    // The largest comment id used anywhere in the server's document (new ones go after it).
    let mut used_max: Option<u32> = None;
    let xml_parts: Vec<String> = szip.file_names().filter(|n| n.to_lowercase().ends_with(".xml")).map(str::to_string).collect();
    for n in &xml_parts {
        if let Some(b) = read(&mut szip, n)? {
            used_max = used_max.max(max_comment_id(&b));
        }
    }
    // The sent file, as Starbase Docs writes it.
    let u_doc = read(&mut uzip, "word/document.xml")?.ok_or("not a document")?;
    let u_com = read(&mut uzip, "word/comments.xml")?;
    let u_ext = read(&mut uzip, "word/commentsExtended.xml")?;
    if u_com.as_ref().map_or(0, Vec::len) > MAX_COMMENT_PART || u_ext.as_ref().map_or(0, Vec::len) > MAX_COMMENT_PART {
        return Err("too many comments at once".into());
    }

    let s_doc = text(&s_doc)?;
    let s_com = s_com.as_deref().map(text).transpose()?;
    let s_ext = s_ext.as_deref().map(text).transpose()?;
    let u_doc = text(&u_doc)?;
    let u_com = u_com.as_deref().map(text).transpose()?;
    let u_ext = u_ext.as_deref().map(text).transpose()?;

    let s_comments = match &s_com {
        Some(t) => comments(&parse(t)?),
        None => BTreeMap::new(),
    };
    let u_comments = match &u_com {
        Some(t) => comments(&parse(t)?),
        None => BTreeMap::new(),
    };
    let s_extra = match &s_ext {
        Some(t) => extended(&parse(t)?),
        None => HashMap::new(),
    };
    let u_extra = u_ext.as_deref().map(|t| parse(t).map(|d| extended(&d))).transpose()?;

    // Parent and done, by comment id.
    let s_pid: HashMap<u32, String> = s_comments.iter().filter_map(|(id, c)| Some((*id, c.para_id.clone()?))).collect();
    let s_by_pid: HashMap<&str, u32> = s_pid.iter().map(|(id, p)| (p.as_str(), *id)).collect();
    let s_state = |id: u32| s_pid.get(&id).and_then(|p| s_extra.get(p));
    let s_parent = |id: u32| s_state(id).and_then(|e| e.parent.as_deref()).and_then(|p| s_by_pid.get(p).copied());
    let u_by_pid: HashMap<&str, u32> = u_comments.iter().filter_map(|(id, c)| Some((c.para_id.as_deref()?, *id))).collect();
    let u_state = |id: u32| -> Option<&Extra> { u_extra.as_ref()?.get(u_comments.get(&id)?.para_id.as_ref()?) };
    let u_parent = |id: u32| u_state(id).and_then(|e| e.parent.as_deref()).and_then(|p| u_by_pid.get(p).copied());

    let mine = |id: &u32| me.own.get(id).is_some_and(|date| s_comments.get(id).is_some_and(|c| !c.date.is_empty() && c.date == *date));
    let new: Vec<u32> = u_comments.keys().filter(|id| !s_comments.contains_key(id)).copied().collect();
    if new.len() > MAX_NEW {
        return Err("too many new comments at once".into());
    }
    // Own comments gone from the sent file are deleted, unless a reply that stays (or a new one)
    // hangs on them: each kept reply keeps its parent, and so on up (one pass, a worklist).
    let mut deleted: BTreeSet<u32> = s_comments.keys().filter(|id| mine(id) && !u_comments.contains_key(id)).copied().collect();
    let mut stay: Vec<(u32, Option<u32>)> = s_comments.keys().filter(|c| !deleted.contains(c)).map(|c| (*c, s_parent(*c))).collect();
    stay.extend(new.iter().map(|c| (*c, u_parent(*c))));
    while let Some((_, parent)) = stay.pop() {
        if let Some(p) = parent
            && deleted.remove(&p)
        {
            stay.push((p, s_parent(p)));
        }
    }
    // A new reply's parent: an existing comment that stays, or a new one that is no reply itself.
    let parent_ok = |id: u32| -> Option<u32> {
        let p = u_parent(id).filter(|p| *p != id)?;
        let existing = s_comments.contains_key(&p) && !deleted.contains(&p);
        let new_top = !s_comments.contains_key(&p) && u_comments.contains_key(&p) && u_parent(p).is_none();
        (existing || new_top).then_some(p)
    };
    let edited: BTreeMap<u32, Vec<String>> = s_comments
        .iter()
        .filter(|(id, c)| mine(id) && u_comments.get(id).is_some_and(|u| u.paras != c.paras))
        .filter_map(|(id, _)| Some((*id, u_comments.get(id)?.paras.clone())))
        .collect();
    let done_changed: BTreeMap<u32, bool> = s_comments
        .keys()
        .filter(|id| mine(id) && u_comments.contains_key(id))
        .filter_map(|id| {
            let u = u_state(*id)?.done;
            (u != s_state(*id).is_some_and(|e| e.done)).then_some((*id, u))
        })
        .collect();
    let long = |id: &u32| {
        u_comments
            .get(id)
            .is_some_and(|c| c.paras.len() > MAX_COMMENT_PARAS || c.paras.iter().map(|p| p.chars().count()).sum::<usize>() > MAX_COMMENT_CHARS)
    };
    if me.own.len() + new.len() > MAX_OWNED {
        return Err("you have too many comments on this document".into());
    }
    if new.iter().chain(edited.keys()).any(long) {
        return Err("a comment is too long".into());
    }
    if new.is_empty() && deleted.is_empty() && edited.is_empty() && done_changed.is_empty() {
        return Ok(None);
    }

    // ---- document.xml: take out the deleted comments' marks, then put in the new ones.
    let s_parsed = parse(&s_doc)?;
    let w = prefix_of(&s_parsed, W).ok_or("this document's layout isn't supported for comments")?;
    let (s_paras, _) = walk(&s_parsed)?;
    let s_texts: Vec<String> = s_paras.iter().map(|p| p.text.clone()).collect();
    let mut edits = Vec::new();
    for n in s_parsed.descendants().filter(|n| is(n, W, "commentRangeStart") || is(n, W, "commentRangeEnd") || is(n, W, "commentReference")) {
        let Some(id) = attr_id(&n) else { continue };
        if !deleted.contains(&id) {
            continue;
        }
        // A run holding only the reference goes with it.
        let whole_run = is(&n, W, "commentReference")
            && n.parent().is_some_and(|r| is(&r, W, "r") && r.children().filter(Node::is_element).all(|c| c == n || is(&c, W, "rPr")));
        let target = if whole_run { n.parent().unwrap_or(n) } else { n };
        edits.push(Edit { at: target.range(), seq: 0, text: String::new() });
    }
    let doc1 = apply(&s_doc, edits)?;
    drop(s_parsed);

    let d1 = parse(&doc1)?;
    let (d1_paras, _) = walk(&d1)?;
    let u_parsed = parse(&u_doc)?;
    let (u_paras, u_marks) = walk(&u_parsed)?;
    // Where each new comment goes: (paragraph, characters before) of its start and end.
    let mut first_mark: HashMap<(u32, bool), Option<Place>> = HashMap::new();
    for m in &u_marks {
        first_mark.entry((m.id, m.start)).or_insert(m.para.map(|p| (p, m.off)));
    }
    let mut places: BTreeMap<u32, (Place, Place)> = BTreeMap::new();
    for id in &new {
        let find = |start: bool| first_mark.get(&(*id, start)).copied().flatten();
        if let (Some(a), Some(b)) = (find(true), find(false))
            && a <= b
        {
            places.insert(*id, (a, b));
        }
    }
    let last_para = places.values().map(|(_, b)| b.0).max();
    if let Some(last) = last_para {
        let same = last < u_paras.len() && last < d1_paras.len() && (0..=last).all(|i| u_paras[i].text == d1_paras[i].text);
        if !same {
            return Err("the document changed while you were commenting: open it again".into());
        }
    }
    // Which new comments are kept: placed, or a reply that can sit beside its parent.
    let mut keep: Vec<u32> = Vec::new();
    let mut beside: Vec<(u32, u32)> = Vec::new(); // (reply, existing parent)
    for id in &new {
        if places.contains_key(id) {
            keep.push(*id);
            continue;
        }
        match parent_ok(*id) {
            Some(p) if places.contains_key(&p) => {
                let at = places[&p];
                places.insert(*id, at);
                keep.push(*id);
            }
            Some(p) if s_comments.contains_key(&p) && !deleted.contains(&p) => {
                beside.push((*id, p));
                keep.push(*id);
            }
            _ => {}
        }
    }
    // The server's ids for the new comments: one past the largest used anywhere, in order.
    let mut sid_of: HashMap<u32, u32> = HashMap::new();
    let mut next = used_max.map_or(Some(0), |m| m.checked_add(1));
    for id in &keep {
        let n = next.ok_or("no free comment id")?;
        sid_of.insert(*id, n);
        next = n.checked_add(1);
    }
    let sid = |id: u32| sid_of.get(&id).copied().unwrap_or(id);
    let mut cuts: Vec<Cut> = Vec::new();
    for (seq, id) in keep.iter().enumerate() {
        if let Some((a, b)) = places.get(id) {
            let n = sid(*id);
            cuts.push(Cut { para: a.0, off: a.1, seq: (0, seq), xml: format!("<{w}:commentRangeStart {w}:id=\"{n}\"/>") });
            cuts.push(Cut { para: b.0, off: b.1, seq: (1, seq), xml: end_marks(&w, n) });
        }
    }
    let mut edits = place(&doc1, &d1_paras, &cuts)?;
    let mut placed_beside: HashSet<u32> = HashSet::new();
    // The existing comments' marks, found in one pass.
    let mut starts: HashMap<u32, Node> = HashMap::new();
    let mut refruns: HashMap<u32, Node> = HashMap::new();
    if !beside.is_empty() {
        for n in d1.descendants() {
            if is(&n, W, "commentRangeStart")
                && let Some(id) = attr_id(&n)
            {
                starts.entry(id).or_insert(n);
            } else if is(&n, W, "commentReference")
                && let Some(id) = attr_id(&n)
                && let Some(r) = n.parent().filter(|r| is(r, W, "r"))
            {
                refruns.entry(id).or_insert(r);
            }
        }
    }
    for (seq, (id, parent)) in beside.iter().enumerate() {
        if let (Some(s), Some(r)) = (starts.get(parent), refruns.get(parent)) {
            let seq = 1000 + seq as u32;
            let n = sid(*id);
            edits.push(Edit { at: s.range().end..s.range().end, seq, text: format!("<{w}:commentRangeStart {w}:id=\"{n}\"/>") });
            edits.push(Edit { at: r.range().end..r.range().end, seq, text: end_marks(&w, n) });
            placed_beside.insert(*id);
        }
    }
    keep.retain(|id| places.contains_key(id) || placed_beside.contains(id));
    let doc2 = apply(&doc1, edits)?;
    drop(u_parsed);
    // The guard: the text reads exactly as on the server.
    let check = parse(&doc2)?;
    let (check_paras, _) = walk(&check)?;
    if check_paras.iter().map(|p| &p.text).ne(s_texts.iter()) {
        return Err("internal error: the comment marks would have changed the text".into());
    }

    // ---- comments.xml: new comments, edited and deleted own ones, paragraph ids for parents.
    let mut taken: HashSet<String> = HashSet::new();
    collect_para_ids(&doc2, &mut taken);
    if let Some(t) = &s_com {
        collect_para_ids(t, &mut taken);
    }
    let mut fresh = || fresh_id(&mut taken);
    let mut pid_of: HashMap<u32, String> = s_pid.clone();
    let mut need_pid: HashSet<u32> = HashSet::new();
    for id in &keep {
        pid_of.insert(*id, fresh());
    }
    for id in edited.keys() {
        pid_of.entry(*id).or_insert_with(&mut fresh);
    }
    let parent_of_new: HashMap<u32, u32> = keep.iter().filter_map(|id| Some((*id, parent_ok(*id)?))).collect();
    for p in parent_of_new.values() {
        if !s_pid.contains_key(p) && s_comments.contains_key(p) && !edited.contains_key(p) {
            pid_of.entry(*p).or_insert_with(&mut fresh);
            need_pid.insert(*p);
        }
    }
    for id in done_changed.keys() {
        if !s_pid.contains_key(id) && !edited.contains_key(id) {
            pid_of.entry(*id).or_insert_with(&mut fresh);
            need_pid.insert(*id);
        }
    }
    // An edited comment keeps its date (part of how the server knows it is theirs).
    let new_comment = |w: &str, w14: &str, id: u32, paras: &[String], pid: &str, date: &str| -> String {
        let mut s = format!(
            "<{w}:comment {w}:id=\"{id}\" {w}:author=\"{}\" {w}:date=\"{}\" {w}:initials=\"{}\">",
            esc(&me.name),
            esc(date),
            esc(&me.initials)
        );
        let paras: Vec<String> = if paras.is_empty() { vec![String::new()] } else { paras.to_vec() };
        let last = paras.len() - 1;
        for (i, p) in paras.iter().enumerate() {
            if i == last {
                s.push_str(&format!("<{w}:p {w14}:paraId=\"{pid}\">"));
            } else {
                s.push_str(&format!("<{w}:p>"));
            }
            s.push_str(&runs(w, p));
            s.push_str(&format!("</{w}:p>"));
        }
        s.push_str(&format!("</{w}:comment>"));
        s
    };
    let new_comments_part;
    let com_out: String = match &s_com {
        Some(t) => {
            let d = parse(t)?;
            let root = d.root_element();
            let cw = prefix_of(&d, W).ok_or("this document's comments can't be changed")?;
            let mut edits = Vec::new();
            let (w14, declare) = match prefix_of(&d, W14) {
                Some(p) => (p, false),
                None if root.lookup_namespace_uri(Some("w14")).is_none() => ("w14".to_string(), true),
                None => return Err("this document's comments can't be changed".into()),
            };
            if declare {
                let at = open_tag_name_end(t, &root);
                edits.push(Edit { at: at..at, seq: 0, text: format!(" xmlns:w14=\"{W14}\"") });
            }
            for c in root.children().filter(|n| is(n, W, "comment")) {
                let Some(id) = attr_id(&c) else { continue };
                if deleted.contains(&id) {
                    edits.push(Edit { at: c.range(), seq: 0, text: String::new() });
                } else if let Some(paras) = edited.get(&id) {
                    let pid = pid_of.get(&id).cloned().unwrap_or_else(&mut fresh);
                    let date = s_comments.get(&id).map_or(me.date.as_str(), |c| c.date.as_str());
                    edits.push(Edit { at: c.range(), seq: 0, text: new_comment(&cw, &w14, id, paras, &pid, date) });
                } else if need_pid.contains(&id)
                    && let Some(last) = c.children().rfind(|n| is(n, W, "p"))
                    && let Some(pid) = pid_of.get(&id)
                {
                    let at = open_tag_name_end(t, &last);
                    edits.push(Edit { at: at..at, seq: 0, text: format!(" {w14}:paraId=\"{pid}\"") });
                }
            }
            let close = close_tag_start(t, &root).ok_or("the comments part is damaged")?;
            let added: String = keep
                .iter()
                .map(|id| new_comment(&cw, &w14, sid(*id), &u_comments[id].paras, pid_of.get(id).map_or("", String::as_str), &me.date))
                .collect();
            edits.push(Edit { at: close..close, seq: 1, text: added });
            new_comments_part = false;
            apply(t, edits)?
        }
        None => {
            if keep.is_empty() {
                new_comments_part = false;
                String::new()
            } else {
                new_comments_part = true;
                let mut s =
                    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:comments xmlns:w=\"{W}\" xmlns:w14=\"{W14}\">");
                for id in &keep {
                    s.push_str(&new_comment("w", "w14", sid(*id), &u_comments[id].paras, pid_of.get(id).map_or("", String::as_str), &me.date));
                }
                s.push_str("</w:comments>");
                s
            }
        }
    };
    if !com_out.is_empty() {
        parse(&com_out)?;
    }

    // ---- commentsExtended.xml: replies and resolved, for the new and own comments.
    let entry = |p: &str, id: u32, parent: Option<u32>, done: bool| -> String {
        let parent = parent.and_then(|p| pid_of.get(&p)).map(|pp| format!(" {p}:paraIdParent=\"{pp}\"")).unwrap_or_default();
        format!("<{p}:commentEx {p}:paraId=\"{}\"{parent} {p}:done=\"{}\"/>", pid_of.get(&id).map_or("", String::as_str), u8::from(done))
    };
    let mut wanted: Vec<(u32, Option<u32>, bool)> =
        keep.iter().map(|id| (*id, parent_of_new.get(id).copied(), u_state(*id).is_some_and(|e| e.done))).collect();
    // Own comments: resolved or not; an edited one keeps its place in a thread under its new id.
    let mut rewrite: HashMap<u32, (Option<u32>, bool)> = HashMap::new();
    for id in edited.keys().chain(done_changed.keys()) {
        let done = done_changed.get(id).copied().unwrap_or_else(|| s_state(*id).is_some_and(|e| e.done));
        rewrite.insert(*id, (s_parent(*id), done));
    }
    let mut new_ext_part = false;
    let ext_out: Option<String> = match &s_ext {
        Some(t) => {
            let d = parse(t)?;
            let root = d.root_element();
            let p = prefix_of(&d, W15).ok_or("this document's comments can't be changed")?;
            let mut edits = Vec::new();
            let mut written: HashSet<u32> = HashSet::new();
            for e in root.children().filter(|n| is(n, W15, "commentEx")) {
                let Some(pid) = e.attribute((W15, "paraId")) else { continue };
                let Some(id) = s_by_pid.get(pid.to_uppercase().as_str()).copied() else { continue };
                if deleted.contains(&id) {
                    edits.push(Edit { at: e.range(), seq: 0, text: String::new() });
                } else if let Some((parent, done)) = rewrite.get(&id) {
                    edits.push(Edit { at: e.range(), seq: 0, text: entry(&p, id, *parent, *done) });
                    written.insert(id);
                }
            }
            for (id, (parent, done)) in &rewrite {
                if !written.contains(id) {
                    wanted.push((*id, *parent, *done));
                }
            }
            let close = close_tag_start(t, &root).ok_or("the comments part is damaged")?;
            let added: String = wanted.iter().map(|(id, parent, done)| entry(&p, *id, *parent, *done)).collect();
            edits.push(Edit { at: close..close, seq: 1, text: added });
            Some(apply(t, edits)?)
        }
        None => {
            for (id, (parent, done)) in &rewrite {
                wanted.push((*id, *parent, *done));
            }
            if wanted.is_empty() {
                None
            } else {
                new_ext_part = true;
                let mut s = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w15:commentsEx xmlns:w15=\"{W15}\">");
                for (id, parent, done) in &wanted {
                    s.push_str(&entry("w15", *id, *parent, *done));
                }
                s.push_str("</w15:commentsEx>");
                Some(s)
            }
        }
    };
    if let Some(t) = &ext_out {
        parse(t)?;
    }

    // ---- New parts need a relationship and a content type.
    let mut replace: HashMap<String, Vec<u8>> = HashMap::new();
    let mut add: Vec<(String, Vec<u8>)> = Vec::new();
    replace.insert(main.clone(), doc2.into_bytes());
    let names: HashSet<String> = szip.file_names().map(|n| n.to_lowercase()).collect();
    let free = |want: &str| -> Result<String, String> {
        if !names.contains(&want.to_lowercase()) {
            return Ok(want.to_string());
        }
        (2..100)
            .map(|i| want.replace(".xml", &format!("{i}.xml")))
            .find(|n| !names.contains(&n.to_lowercase()))
            .ok_or("no free part name".to_string())
    };
    let mut rel_adds: Vec<(&str, String)> = Vec::new();
    let mut ct_adds: Vec<(String, &str)> = Vec::new();
    if !com_out.is_empty() {
        match &c_name {
            Some(n) if !new_comments_part => {
                replace.insert(n.clone(), com_out.into_bytes());
            }
            _ => {
                let n = free(&format!("{dir}comments.xml"))?;
                rel_adds.push((REL_COMMENTS, n[dir.len()..].to_string()));
                ct_adds.push((format!("/{n}"), CT_COMMENTS));
                add.push((n, com_out.into_bytes()));
            }
        }
    }
    if let Some(t) = ext_out {
        match &x_name {
            Some(n) if !new_ext_part => {
                replace.insert(n.clone(), t.into_bytes());
            }
            _ => {
                let n = free(&format!("{dir}commentsExtended.xml"))?;
                rel_adds.push((REL_COMMENTS_EX, n[dir.len()..].to_string()));
                ct_adds.push((format!("/{n}"), CT_COMMENTS_EX));
                add.push((n, t.into_bytes()));
            }
        }
    }
    if !rel_adds.is_empty() {
        let rels = doc_rels.as_deref().map(text).transpose()?.unwrap_or_else(|| {
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>".into()
        });
        let d = parse(&rels)?;
        let ids: HashSet<&str> = d.descendants().filter_map(|n| n.attribute("Id")).collect();
        let mut next = 1;
        let mut rel_text = String::new();
        for (ty, target) in &rel_adds {
            while ids.contains(format!("rId{next}").as_str()) {
                next += 1;
            }
            rel_text.push_str(&format!("<Relationship Id=\"rId{next}\" Type=\"{ty}\" Target=\"{}\"/>", esc(target)));
            next += 1;
        }
        let close = close_tag_start(&rels, &d.root_element()).ok_or("the relationships part is damaged")?;
        let out = apply(&rels, vec![Edit { at: close..close, seq: 0, text: rel_text }])?;
        parse(&out)?;
        if doc_rels.is_some() {
            replace.insert(rels_name.clone(), out.into_bytes());
        } else {
            add.push((rels_name.clone(), out.into_bytes()));
        }
        let ct = read(&mut szip, "[Content_Types].xml")?.ok_or("the document has no content types")?;
        let ct = text(&ct)?;
        let d = parse(&ct)?;
        let close = close_tag_start(&ct, &d.root_element()).ok_or("the content types are damaged")?;
        let have: HashSet<String> = d.descendants().filter_map(|n| n.attribute("PartName")).map(str::to_lowercase).collect();
        let added: String = ct_adds
            .iter()
            .filter(|(n, _)| !have.contains(&n.to_lowercase()))
            .map(|(n, t)| format!("<Override PartName=\"{}\" ContentType=\"{t}\"/>", esc(n)))
            .collect();
        let out = apply(&ct, vec![Edit { at: close..close, seq: 0, text: added }])?;
        parse(&out)?;
        replace.insert("[Content_Types].xml".into(), out.into_bytes());
    }
    // ---- Deleted comments' entries in commentsIds, commentsExtensible and people.xml.
    if !deleted.is_empty() {
        prune(&mut szip, doc_rels.as_deref(), &dir, &deleted, &s_pid, &s_comments, (!keep.is_empty()).then_some(me.name.as_str()), &mut replace)?;
    }
    drop(uzip);
    let bytes = write_zip(&mut szip, &replace, &add)?;
    if bytes.len() > server.len().saturating_add(MAX_GROWTH) {
        return Err("too much added at once".into());
    }
    // Whose comments these are now: the recorded ones still there, and the new ones.
    let mut own: HashMap<u32, String> = me.own.iter().filter(|(id, _)| mine(id) && !deleted.contains(id)).map(|(i, d)| (*i, d.clone())).collect();
    own.extend(keep.iter().map(|id| (sid(*id), me.date.clone())));
    let renumbered = keep.iter().any(|id| sid(*id) != *id);
    Ok(Some(Merged { bytes, own, renumbered }))
}

/// Takes the `deleted` comments out of commentsIds.xml and commentsExtensible.xml, and their
/// authors out of people.xml when no comment of theirs is left (`adding`: the author of new
/// comments in this save).
#[allow(clippy::too_many_arguments, reason = "the merge's state, passed through once")]
fn prune(
    szip: &mut zip::ZipArchive<Cursor<&[u8]>>,
    doc_rels: Option<&[u8]>,
    dir: &str,
    deleted: &BTreeSet<u32>,
    s_pid: &HashMap<u32, String>,
    s_comments: &BTreeMap<u32, Comment>,
    adding: Option<&str>,
    replace: &mut HashMap<String, Vec<u8>>,
) -> Result<(), String> {
    let gone_pids: HashSet<&str> = deleted.iter().filter_map(|id| s_pid.get(id)).map(String::as_str).collect();
    let mut gone_durable: HashSet<String> = HashSet::new();
    let mut drop_from = |szip: &mut zip::ZipArchive<Cursor<&[u8]>>,
                         rel: &str,
                         ns: &str,
                         name: &str,
                         attr: &str,
                         gone: &dyn Fn(&str) -> bool,
                         found: &mut Vec<String>|
     -> Result<(), String> {
        let Some(n) = doc_rels.and_then(|r| target_of(r, dir, rel)) else { return Ok(()) };
        let Some(b) = read(szip, &n)? else { return Ok(()) };
        let t = text(&b)?;
        let d = parse(&t)?;
        let mut edits = Vec::new();
        for e in d.root_element().children().filter(|e| is(e, ns, name)) {
            if e.attribute((ns, attr)).is_some_and(gone) {
                if ns == W16CID {
                    found.extend(e.attribute((W16CID, "durableId")).map(str::to_uppercase));
                }
                edits.push(Edit { at: e.range(), seq: 0, text: String::new() });
            }
        }
        if !edits.is_empty() {
            replace.insert(n, apply(&t, edits)?.into_bytes());
        }
        Ok(())
    };
    let mut durable = Vec::new();
    drop_from(szip, REL_COMMENTS_IDS, W16CID, "commentId", "paraId", &|p| gone_pids.contains(p.to_uppercase().as_str()), &mut durable)?;
    gone_durable.extend(durable);
    if !gone_durable.is_empty() {
        drop_from(szip, REL_COMMENTS_EXT, W16CEX, "commentExtensible", "durableId", &|x| gone_durable.contains(&x.to_uppercase()), &mut Vec::new())?;
    }
    let mut left: HashSet<&str> = s_comments.iter().filter(|(id, _)| !deleted.contains(id)).map(|(_, c)| c.author.as_str()).collect();
    left.extend(adding);
    let gone_authors: HashSet<&str> =
        deleted.iter().filter_map(|id| s_comments.get(id)).map(|c| c.author.as_str()).filter(|a| !left.contains(a)).collect();
    if !gone_authors.is_empty() {
        drop_from(szip, REL_PEOPLE, W15, "person", "author", &|a| gone_authors.contains(a), &mut Vec::new())?;
    }
    Ok(())
}

// ---------------------------------------------------------------- the package

fn open(bytes: &[u8]) -> Result<zip::ZipArchive<Cursor<&[u8]>>, String> {
    let zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "not a .docx file".to_string())?;
    if zip.len() > MAX_ENTRIES {
        return Err("too many parts".into());
    }
    // Two parts of one name (or names differing only in case) could show one thing here and
    // another to Word. The zip reader keeps one of two equal names, so the entries the package
    // says it has are counted too.
    if entries_listed(bytes) != Some(zip.len()) {
        return Err("two parts with the same name".into());
    }
    let mut seen = HashSet::new();
    for i in 0..zip.len() {
        let Some(name) = zip.name_for_index(i) else { return Err("a damaged part".into()) };
        if !seen.insert(name.to_lowercase()) {
            return Err("two parts with the same name".into());
        }
    }
    Ok(zip)
}

/// How many entries the package's end record lists (`None` when it can't be read, or for a
/// zip64 package, which a document never needs).
fn entries_listed(bytes: &[u8]) -> Option<usize> {
    const END: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    let from = bytes.len().saturating_sub(22 + 65_535);
    let at = (from..bytes.len().saturating_sub(21)).rev().find(|&i| bytes.get(i..i + 4) == Some(&END[..]))?;
    let n = u16::from_le_bytes([*bytes.get(at + 10)?, *bytes.get(at + 11)?]);
    (n != u16::MAX).then_some(usize::from(n))
}

/// Part `name`, or `None` when there is none; at most [`MAX_PART`] bytes.
fn read(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Option<Vec<u8>>, String> {
    let f = match zip.by_name(name) {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(_) => return Err("a damaged part".into()),
    };
    let mut out = Vec::new();
    f.take(MAX_PART + 1).read_to_end(&mut out).map_err(|_| "a damaged part".to_string())?;
    if out.len() as u64 > MAX_PART {
        return Err("a part is too large".into());
    }
    Ok(Some(out))
}

/// The server's parts, each copied as it is, except those in `replace`; then `add`.
fn write_zip(src: &mut zip::ZipArchive<Cursor<&[u8]>>, replace: &HashMap<String, Vec<u8>>, add: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let bad = |_| "couldn't write the document".to_string();
    for i in 0..src.len() {
        let name = src.name_for_index(i).ok_or("a damaged part")?.to_string();
        if let Some(bytes) = replace.get(&name) {
            out.start_file(name, opts).map_err(bad)?;
            out.write_all(bytes).map_err(|_| "couldn't write the document".to_string())?;
        } else {
            let f = src.by_index_raw(i).map_err(|_| "a damaged part".to_string())?;
            out.raw_copy_file(f).map_err(bad)?;
        }
    }
    for (name, bytes) in add {
        out.start_file(name.clone(), opts).map_err(bad)?;
        out.write_all(bytes).map_err(|_| "couldn't write the document".to_string())?;
    }
    Ok(out.finish().map_err(bad)?.into_inner())
}

fn text(bytes: &[u8]) -> Result<String, String> {
    let b = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    String::from_utf8(b.to_vec()).map_err(|_| "a part isn't UTF-8 text".to_string())
}

fn parse(t: &str) -> Result<Document<'_>, String> {
    Document::parse_with_options(t, ParsingOptions { allow_dtd: false, nodes_limit: MAX_NODES }).map_err(|_| "a part isn't readable XML".to_string())
}

/// The target of the relationship of type `ty`, as a part name (relative to `dir`). External
/// targets don't count.
fn target_of(rels: &[u8], dir: &str, ty: &str) -> Option<String> {
    let t = std::str::from_utf8(rels).ok()?;
    let d = parse(t).ok()?;
    let r = d
        .descendants()
        .find(|n| n.tag_name().name() == "Relationship" && n.attribute("Type") == Some(ty) && n.attribute("TargetMode") != Some("External"))?;
    let target = r.attribute("Target")?;
    let path = match target.strip_prefix('/') {
        Some(abs) => abs.to_string(),
        None => format!("{dir}{target}"),
    };
    // No way out of the package's folders.
    (!path.split('/').any(|s| s == ".." || s == ".")).then_some(path)
}

// ---------------------------------------------------------------- XML helpers

fn is(n: &Node, ns: &str, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(ns)
}

fn attr_id(n: &Node) -> Option<u32> {
    n.attribute((W, "id"))?.trim().parse().ok()
}

/// The prefix the root element binds to `ns` (never the default namespace).
fn prefix_of(d: &Document, ns: &str) -> Option<String> {
    d.root_element().namespaces().find(|n| n.uri() == ns).and_then(|n| n.name()).map(str::to_string)
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => o.push(c),
        }
    }
    o
}

/// Where an element's start tag name ends (for adding an attribute after it).
fn open_tag_name_end(src: &str, n: &Node) -> usize {
    let s = n.range().start;
    s + 1 + src[s + 1..].find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(0)
}

/// Where an element's end tag starts; `None` for an empty-element tag.
fn close_tag_start(src: &str, n: &Node) -> Option<usize> {
    let r = n.range();
    let body = &src[r.clone()];
    if body.ends_with("/>") {
        return None;
    }
    body.rfind("</").map(|i| r.start + i)
}

/// A run holding plain `text` (tabs as w:tab, line breaks as w:br).
fn runs(w: &str, text: &str) -> String {
    let mut s = format!("<{w}:r>");
    for (l, line) in text.split('\n').enumerate() {
        if l > 0 {
            s.push_str(&format!("<{w}:br/>"));
        }
        for (i, part) in line.split('\t').enumerate() {
            if i > 0 {
                s.push_str(&format!("<{w}:tab/>"));
            }
            if !part.is_empty() {
                s.push_str(&format!("<{w}:t xml:space=\"preserve\">{}</{w}:t>", esc(part)));
            }
        }
    }
    s.push_str(&format!("</{w}:r>"));
    s
}

/// The largest comment id in a part: on any comment, commentRangeStart, commentRangeEnd or
/// commentReference element, read from the raw text whatever the prefix (one too many read is
/// harmless: new ids only go higher).
fn max_comment_id(part: &[u8]) -> Option<u32> {
    let t = String::from_utf8_lossy(part);
    let mut best: Option<u32> = None;
    let mut rest: &str = &t;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..end];
        let name_end = tag.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(tag.len());
        let local = tag[..name_end].rsplit(':').next().unwrap_or("");
        if matches!(local, "comment" | "commentRangeStart" | "commentRangeEnd" | "commentReference") {
            let mut attrs = &tag[name_end..];
            while let Some(eq) = attrs.find('=') {
                let key = attrs[..eq].trim().rsplit(':').next().unwrap_or("");
                let after = attrs[eq + 1..].trim_start();
                let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
                let val_end = after[1..].find(q).map_or(after.len(), |k| k + 1);
                if key == "id"
                    && let Ok(v) = after[1..val_end].trim().parse::<u32>()
                {
                    best = best.max(Some(v));
                }
                attrs = after.get(val_end + 1..).unwrap_or("");
            }
        }
        rest = &rest[end..];
    }
    best
}

fn end_marks(w: &str, id: u32) -> String {
    format!("<{w}:commentRangeEnd {w}:id=\"{id}\"/><{w}:r><{w}:commentReference {w}:id=\"{id}\"/></{w}:r>")
}

fn collect_para_ids(t: &str, out: &mut HashSet<String>) {
    if let Ok(d) = parse(t) {
        out.extend(d.descendants().filter_map(|n| n.attribute((W14, "paraId"))).map(str::to_uppercase));
    }
}

/// A paragraph id not used yet (8 hex digits below 0x80000000, as Word's).
fn fresh_id(taken: &mut HashSet<String>) -> String {
    loop {
        let v: u32 = rand::random::<u32>() & 0x7FFF_FFFF;
        let s = format!("{:08X}", v.max(1));
        if taken.insert(s.clone()) {
            return s;
        }
    }
}

/// One change to a part's text: `at` replaced by `text`. Edits at the same place go in `seq`
/// order.
struct Edit {
    at: std::ops::Range<usize>,
    seq: u32,
    text: String,
}

fn apply(src: &str, mut edits: Vec<Edit>) -> Result<String, String> {
    edits.sort_by_key(|e| (e.at.start, e.at.end, e.seq));
    let mut out = String::with_capacity(src.len() + 1024);
    let mut pos = 0;
    for e in edits {
        if e.at.start < pos || e.at.end > src.len() {
            return Err("internal error: overlapping changes".into());
        }
        out.push_str(&src[pos..e.at.start]);
        out.push_str(&e.text);
        pos = e.at.end;
    }
    out.push_str(&src[pos..]);
    Ok(out)
}

// ---------------------------------------------------------------- comments

struct Comment {
    author: String,
    /// The w:date the comment carries (the server's, for comments added by commenters).
    date: String,
    /// The text, paragraph by paragraph (tabs as '\t', breaks as '\n').
    paras: Vec<String>,
    /// The last paragraph's id (w14:paraId), which commentsExtended refers to.
    para_id: Option<String>,
}

fn comments(d: &Document) -> BTreeMap<u32, Comment> {
    let mut out = BTreeMap::new();
    for c in d.root_element().children().filter(|n| is(n, W, "comment")) {
        let Some(id) = attr_id(&c) else { continue };
        // The comment's own paragraphs (not ones nested inside them, which would be read twice).
        let paras: Vec<Node> = c.children().filter(|n| is(n, W, "p")).collect();
        let mut texts = Vec::new();
        for p in &paras {
            let mut s = String::new();
            for n in p.descendants() {
                if is(&n, W, "t") {
                    s.push_str(n.text().unwrap_or(""));
                } else if is(&n, W, "tab") && n.parent().is_some_and(|r| is(&r, W, "r")) {
                    s.push('\t');
                } else if is(&n, W, "br") || is(&n, W, "cr") {
                    s.push('\n');
                }
            }
            texts.push(s);
        }
        let para_id = paras.last().and_then(|p| p.attribute((W14, "paraId"))).map(str::to_uppercase);
        out.insert(
            id,
            Comment {
                author: c.attribute((W, "author")).unwrap_or("").to_string(),
                date: c.attribute((W, "date")).unwrap_or("").to_string(),
                paras: texts,
                para_id,
            },
        );
    }
    out
}

struct Extra {
    done: bool,
    parent: Option<String>,
}

/// commentsExtended.xml by paragraph id.
fn extended(d: &Document) -> HashMap<String, Extra> {
    d.root_element()
        .children()
        .filter(|n| is(n, W15, "commentEx"))
        .filter_map(|e| {
            let pid = e.attribute((W15, "paraId"))?.to_uppercase();
            let done = matches!(e.attribute((W15, "done")), Some("1" | "true"));
            Some((pid, Extra { done, parent: e.attribute((W15, "paraIdParent")).map(str::to_uppercase) }))
        })
        .collect()
}

// ---------------------------------------------------------------- the document's paragraphs

/// A body paragraph (not one in a text box or a part kept for older programs): its text, one
/// char per character, tab, break and picture or other object (U+FFFC), and where each comes from.
struct Para<'a, 'i> {
    node: Node<'a, 'i>,
    text: String,
    /// (node, first character, characters): a w:t / w:delText, or a one-character item.
    items: Vec<(Node<'a, 'i>, usize, usize)>,
    len: usize,
}

/// A comment mark: which comment, start or end, and where (paragraph, characters before it).
struct Mark {
    id: u32,
    start: bool,
    para: Option<usize>,
    off: usize,
}

fn walk<'a, 'i>(d: &'a Document<'i>) -> Result<(Vec<Para<'a, 'i>>, Vec<Mark>), String> {
    let mut paras = Vec::new();
    let mut marks = Vec::new();
    visit(d.root_element(), 0, None, &mut paras, &mut marks)?;
    Ok((paras, marks))
}

fn visit<'a, 'i>(n: Node<'a, 'i>, depth: usize, cur: Option<usize>, paras: &mut Vec<Para<'a, 'i>>, marks: &mut Vec<Mark>) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("nested too deeply".into());
    }
    let ns = n.tag_name().namespace();
    let name = n.tag_name().name();
    let w = ns == Some(W);
    // Text boxes, pictures and the copies kept for older programs: one object in a paragraph,
    // never paragraphs of the body.
    let object = (w && matches!(name, "drawing" | "pict" | "object" | "txbxContent")) || (ns == Some(MC) && name == "AlternateContent");
    if w && matches!(name, "commentRangeStart" | "commentRangeEnd") {
        if let Some(id) = attr_id(&n) {
            let off = cur.and_then(|p| paras.get(p)).map_or(0, |p| p.len);
            marks.push(Mark { id, start: name == "commentRangeStart", para: cur, off });
        }
        return Ok(());
    }
    let Some(p) = cur else {
        if object {
            return Ok(());
        }
        if w && name == "p" {
            paras.push(Para { node: n, text: String::new(), items: Vec::new(), len: 0 });
            let idx = paras.len() - 1;
            for c in n.children().filter(Node::is_element) {
                visit(c, depth + 1, Some(idx), paras, marks)?;
            }
            return Ok(());
        }
        for c in n.children().filter(Node::is_element) {
            visit(c, depth + 1, None, paras, marks)?;
        }
        return Ok(());
    };
    let push = |paras: &mut Vec<Para<'a, 'i>>, node: Node<'a, 'i>, s: &str| {
        if let Some(para) = paras.get_mut(p) {
            let len = s.chars().count();
            para.items.push((node, para.len, len));
            para.text.push_str(s);
            para.len += len;
        }
    };
    if object {
        push(paras, n, "\u{FFFC}");
        return Ok(());
    }
    if w {
        match name {
            "t" | "delText" => {
                push(paras, n, shown_text(&n));
                return Ok(());
            }
            "tab" if n.parent().is_some_and(|r| is(&r, W, "r")) => {
                push(paras, n, "\t");
                return Ok(());
            }
            "br" | "cr" => {
                push(paras, n, "\n");
                return Ok(());
            }
            "noBreakHyphen" => {
                push(paras, n, "-");
                return Ok(());
            }
            "sym" => {
                push(paras, n, "\u{FFFD}");
                return Ok(());
            }
            "pPr" | "rPr" | "instrText" | "delInstrText" | "p" | "fldData" => return Ok(()),
            _ => {}
        }
    }
    for c in n.children().filter(Node::is_element) {
        visit(c, depth + 1, cur, paras, marks)?;
    }
    Ok(())
}

/// A place in the document: (paragraph, characters before it).
type Place = (usize, usize);
/// The order of marks put at one place: (0 starts, 1 ends; then by comment).
type Order = (u32, usize);
/// A cut inside a run: (the run's child it is in, characters into that child, order, marks).
type RunCut<'x> = (usize, usize, Order, &'x str);

/// A w:t's text as Word shows it: without xml:space="preserve", the spaces at its two ends are
/// dropped.
fn shown_text<'a>(n: &Node<'a, '_>) -> &'a str {
    let raw = n.text().unwrap_or("");
    if n.attribute(("http://www.w3.org/XML/1998/namespace", "space")) == Some("preserve") { raw } else { raw.trim_matches(char::is_whitespace) }
}

/// A place to put comment marks: paragraph, characters before, order, the marks.
struct Cut {
    para: usize,
    off: usize,
    seq: Order,
    xml: String,
}

/// The edits putting `cuts` into the document `src` (whose paragraphs are `paras`): between runs,
/// inside a run (split in two, each half keeping its formatting), or at a paragraph's end.
fn place(src: &str, paras: &[Para], cuts: &[Cut]) -> Result<Vec<Edit>, String> {
    // Cuts by run (the run, the child it is before, characters into that child) or by place.
    let mut in_runs: BTreeMap<usize, (Node, Vec<RunCut>)> = BTreeMap::new();
    let mut edits = Vec::new();
    let mut para_ends: BTreeMap<usize, Vec<(Order, &str)>> = BTreeMap::new();
    for cut in cuts {
        let para = paras.get(cut.para).ok_or("internal error: no such paragraph")?;
        let seq = cut.seq.0.saturating_mul(100_000).saturating_add(u32::try_from(cut.seq.1).unwrap_or(u32::MAX));
        let Some((item, start, _)) = para.items.iter().find(|(_, s, l)| *s <= cut.off && cut.off < s + l) else {
            if cut.off == para.len {
                para_ends.entry(cut.para).or_default().push((cut.seq, &cut.xml));
                continue;
            }
            return Err("internal error: no such place".into());
        };
        let into = cut.off - start;
        // The run it is in, and the run's child that holds it.
        let mut child = *item;
        let mut run = None;
        while let Some(parent) = child.parent() {
            if parent == para.node {
                break;
            }
            if is(&parent, W, "r") {
                run = Some(parent);
                break;
            }
            child = parent;
        }
        match run {
            Some(r) => {
                if child != *item && into != 0 {
                    return Err("internal error: a place inside an object".into());
                }
                let idx = r.children().filter(|c| c.is_element() && !is(c, W, "rPr")).position(|c| c == child).ok_or("internal error: run")?;
                in_runs.entry(r.range().start).or_insert((r, Vec::new())).1.push((idx, into, cut.seq, &cut.xml));
            }
            None => {
                if into != 0 {
                    return Err("internal error: a place inside an object".into());
                }
                // Not in a run (an object, or math): before the paragraph's child that holds it,
                // never inside something that can't hold a run.
                let at = child.range().start;
                edits.push(Edit { at: at..at, seq, text: cut.xml.clone() });
            }
        }
    }
    for (_, (r, mut list)) in in_runs {
        list.sort_by_key(|(i, o, s, _)| (*i, *o, *s));
        edits.push(Edit { at: r.range(), seq: 0, text: split_run(src, &r, &list)? });
    }
    for (p, mut list) in para_ends {
        list.sort_by_key(|(s, _)| *s);
        let node = paras[p].node;
        let marks: String = list.iter().map(|(_, x)| *x).collect();
        match close_tag_start(src, &node) {
            Some(at) => edits.push(Edit { at: at..at, seq: u32::MAX, text: marks }),
            None => {
                // An empty paragraph written as <w:p/>.
                let r = node.range();
                let open = src[r.start..r.end - 2].trim_end().to_string();
                let s = r.start;
                let qname = &src[s + 1..s + 1 + src[s + 1..].find(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/').unwrap_or(0)];
                edits.push(Edit { at: r, seq: 0, text: format!("{open}>{marks}</{qname}>") });
            }
        }
    }
    Ok(edits)
}

/// Run `r` cut at each (child, characters into it): the pieces each keep the run's own
/// properties, with the marks between them.
fn split_run(src: &str, r: &Node, cuts: &[RunCut]) -> Result<String, String> {
    let first = r.first_child().ok_or("internal error: empty run")?;
    let last = r.last_child().ok_or("internal error: empty run")?;
    let open = &src[r.range().start..first.range().start];
    let close = &src[last.range().end..r.range().end];
    let rpr = r.children().find(|c| is(c, W, "rPr")).map_or("", |c| &src[c.range()]);
    let mut out = String::new();
    let mut cur = String::new();
    let flush = |out: &mut String, cur: &mut String| {
        if !cur.is_empty() {
            out.push_str(open);
            out.push_str(rpr);
            out.push_str(cur);
            out.push_str(close);
            cur.clear();
        }
    };
    let children: Vec<Node> = r.children().filter(|c| c.is_element() && !is(c, W, "rPr")).collect();
    for (i, c) in children.iter().enumerate() {
        let here: Vec<&RunCut> = cuts.iter().filter(|(ci, ..)| *ci == i).collect();
        let text_child = is(c, W, "t") || is(c, W, "delText");
        if here.is_empty() {
            cur.push_str(&src[c.range()]);
            continue;
        }
        if !text_child {
            for (_, _, _, x) in &here {
                flush(&mut out, &mut cur);
                out.push_str(x);
            }
            cur.push_str(&src[c.range()]);
            continue;
        }
        let s = c.range().start;
        let qname_end = s + 1 + src[s + 1..].find(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/').unwrap_or(0);
        let qname = &src[s + 1..qname_end];
        let chars: Vec<char> = c.text().unwrap_or("").chars().collect();
        // Without xml:space="preserve" Word drops spaces at the text's two ends: the pieces keep
        // that (the outer ends trimmed) and keep the spaces at the cuts.
        let preserve = c.attribute(("http://www.w3.org/XML/1998/namespace", "space")) == Some("preserve");
        let (lo, hi) = if preserve {
            (0, chars.len())
        } else {
            let lo = chars.iter().position(|ch| !ch.is_whitespace()).unwrap_or(chars.len());
            let hi = chars.iter().rposition(|ch| !ch.is_whitespace()).map_or(lo, |i| i + 1);
            (lo, hi.max(lo))
        };
        let mut from = 0;
        let piece = |cur: &mut String, a: usize, b: usize| {
            let (a, b) = (a.max(lo), b.min(hi));
            if b > a {
                let t: String = chars[a..b].iter().collect();
                cur.push_str(&format!("<{qname} xml:space=\"preserve\">{}</{qname}>", esc(&t)));
            }
        };
        for (_, off, _, x) in &here {
            // Places count the text as shown (see [`shown_text`]): from its first kept character.
            let off = (lo + *off).min(hi);
            piece(&mut cur, from, off);
            from = off;
            flush(&mut out, &mut cur);
            out.push_str(x);
        }
        piece(&mut cur, from, chars.len());
    }
    flush(&mut out, &mut cur);
    Ok(out)
}

#[cfg(test)]
#[path = "docx_comments_tests.rs"]
mod tests;
