//! An in-memory "server" and "clients" for the convergence tests: each client is a real
//! WordCraft [`Session`] running commands; after each command its change is worked out with
//! [`diff`], kept as pending steps, sent one change at a time to the server (which transforms a
//! late change over what it applied since the change's base, applies it, numbers it and
//! broadcasts it), and rebased over what the server sends. Clients can undo their own last
//! change after others have edited (selective undo through the inverse steps).

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_doc::{Block, Blocks, Document, InlineObject, Path, Pos, StoryRef};
use wordcraft_engine::{Selection, Session};

use crate::step::{Cont, Step, apply_all, normalized};
use crate::{diff, invert_all, map_pos, xform_lists};

/// A small deterministic random source (SplitMix64), so a failing seed replays exactly.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next_u64() % n as u64) as usize }
    }
    pub fn chance(&mut self, pct: usize) -> bool {
        self.below(100) < pct
    }
}

pub const DOC_NAME: &str = "shared.docx";

thread_local! {
    /// Print every command's steps and every server change (`COEDIT_TRACE`, for replays).
    pub static TRACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn trace() -> bool {
    TRACE.with(std::cell::Cell::get)
}

/// The starting document as `.docx` bytes: a heading, paragraphs, a table, a comment, a tracked
/// insertion, a header and a footnote, made with the engine's own commands and saved by its
/// writer.
pub fn base_docx() -> Vec<u8> {
    let mut s = Session::new(Document::new());
    s.doc = s.blank_document();
    s.clamp_selection();
    let run = |s: &mut Session, id: &str, v: Value| {
        s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e:?}"));
    };
    run(&mut s, "text.insert", json!({"text": "Shared notes"}));
    run(&mut s, "para.style", json!({"style": "Heading 1"}));
    for t in [
        "The quick brown fox jumps over the lazy dog.",
        "Pack my box with five dozen liquor jugs, then wait.",
        "How vexingly quick daft zebras jump in the morning.",
        "Sphinx of black quartz, judge my vow.",
    ] {
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, "text.insert", json!({"text": t}));
    }
    run(&mut s, "para.style", json!({"style": "Normal"}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "insert.table", json!({"rows": 3, "cols": 3}));
    run(&mut s, "text.insert", json!({"text": "Cell"}));
    // Back to the last body paragraph (after the table) and a closing line.
    let last = s.doc.body.len() - 1;
    if s.doc.body[last].as_para().is_none() {
        s.doc.body.push(Arc::new(Block::Para(Default::default())));
    }
    let last = s.doc.body.len() - 1;
    caret(&mut s, &Cont::body(), last, 0);
    run(&mut s, "text.insert", json!({"text": "Closing words of the document."}));
    // A comment on "quick brown" in paragraph 1.
    s.sel = Selection { anchor: pos(&Cont::body(), 1, 4), focus: pos(&Cont::body(), 1, 15) };
    run(&mut s, "review.newComment", json!({"text": "Is this right?"}));
    // A tracked insertion in paragraph 2.
    run(&mut s, "review.trackChanges", json!({"value": true}));
    caret(&mut s, &Cont::body(), 2, 0);
    run(&mut s, "text.insert", json!({"text": "Tracked "}));
    run(&mut s, "review.trackChanges", json!({"value": false}));
    // A footnote in paragraph 3 and a header.
    caret(&mut s, &Cont::body(), 3, 10);
    run(&mut s, "references.footnote", json!({}));
    run(&mut s, "text.insert", json!({"text": "A note."}));
    run(&mut s, "insert.editHeader", json!({}));
    run(&mut s, "text.insert", json!({"text": "Team header"}));
    run(&mut s, "insert.closeHeader", json!({}));
    wordcraft_engine::io::save_bytes(DOC_NAME, &s.doc).expect("save base docx")
}

pub fn open(bytes: &[u8]) -> Document {
    wordcraft_engine::io::open_bytes(DOC_NAME, bytes).expect("open docx")
}

pub fn pos(c: &Cont, k: usize, off: usize) -> Pos {
    let mut path = c.path.clone();
    path.push(k as u32);
    Pos { story: c.story, path: Path(path), off }
}

pub fn body_pos(k: usize, off: usize) -> Pos {
    pos(&Cont::body(), k, off)
}

fn caret(s: &mut Session, c: &Cont, k: usize, off: usize) {
    s.sel = Selection::caret(pos(c, k, off));
    s.pending = None;
    s.clamp_selection();
}

/// Every paragraph in the document: (container, block), in every story and table cell.
pub fn all_paras(doc: &Document) -> Vec<(Cont, usize)> {
    fn walk(c: &Cont, blocks: &Blocks, out: &mut Vec<(Cont, usize)>) {
        for (i, b) in blocks.iter().enumerate() {
            match &**b {
                Block::Para(_) => out.push((c.clone(), i)),
                Block::Table(t) => {
                    for (r, row) in t.rows.iter().enumerate() {
                        for (k, cell) in row.cells.iter().enumerate() {
                            walk(&c.cell(i, r, k), &cell.blocks, out);
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&Cont::body(), &doc.body, &mut out);
    for (id, p) in &doc.parts {
        walk(&Cont::story(StoryRef::Part(*id)), &p.blocks, &mut out);
    }
    out
}

fn random_off(rng: &mut Rng, doc: &Document, c: &Cont, k: usize) -> usize {
    let Some(p) = crate::step::para_ref(doc, c, k) else { return 0 };
    let mut o = rng.below(p.text.len() + 1);
    while !p.text.is_char_boundary(o) {
        o -= 1;
    }
    o
}

/// What kinds of commands the random clients run.
#[derive(Clone, Copy, Debug)]
pub struct Mix {
    /// Backspace, Delete and typing over / deleting a selection.
    pub deletes: bool,
    /// Tables, comments, footnotes, headers, tracked changes.
    pub objects: bool,
    /// Undo of the client's own last change.
    pub undo: bool,
}

/// One random command on a client's session. Returns the command's id (for failure reports).
fn random_command(rng: &mut Rng, s: &mut Session, me: usize, mix: Mix) -> String {
    let body: Vec<(Cont, usize)> = all_paras(&s.doc).into_iter().filter(|(c, _)| c.story == StoryRef::Body && c.path.is_empty()).collect();
    let everywhere = all_paras(&s.doc);
    if !mix.deletes {
        // Typing over a selection left by Bold would delete it.
        s.sel = Selection::caret(s.sel.focus.clone());
    }
    // Mostly in a top-level body paragraph; with objects on, anywhere (cells, notes, headers).
    if rng.chance(30) || s.sel.focus.story != StoryRef::Body && rng.chance(40) {
        let pool = if mix.objects && rng.chance(40) { &everywhere } else { &body };
        let (c, k) = pool[rng.below(pool.len())].clone();
        let o = random_off(rng, &s.doc, &c, k);
        caret(s, &c, k, o);
    }
    let marker = char::from(b'1' + me as u8);
    let mut pick = rng.below(100);
    if !mix.objects && pick >= 90 {
        pick = rng.below(90);
    }
    if !mix.deletes && (55..80).contains(&pick) {
        pick = rng.below(55);
    }
    let (fc, fk) =
        (Cont { story: s.sel.focus.story, path: s.sel.focus.path.0[..s.sel.focus.path.0.len().saturating_sub(1)].to_vec() }, s.sel.focus.path.last());
    let (id, v): (&str, Value) = match pick {
        0..=34 => {
            let n = 1 + rng.below(4);
            let alphabet = [marker, marker, 'a', 'e', ' ', 'é', '日'];
            let t: String = (0..n).map(|_| alphabet[rng.below(alphabet.len())]).collect();
            ("text.insert", json!({ "text": t }))
        }
        35..=44 => ("text.newParagraph", json!({})),
        45..=54 => {
            // Bold or italic over a range in the caret's paragraph.
            let (a, b) = (random_off(rng, &s.doc, &fc, fk), random_off(rng, &s.doc, &fc, fk));
            s.sel = Selection { anchor: pos(&fc, fk, a.min(b)), focus: pos(&fc, fk, a.max(b)) };
            s.pending = None;
            (if rng.chance(50) { "format.bold" } else { "format.italic" }, json!({}))
        }
        55..=66 => (if rng.chance(60) { "text.backspace" } else { "text.delete" }, json!({})),
        67..=79 => {
            // A selection over one or two body paragraphs, deleted or typed over.
            let i = rng.below(body.len());
            let ((c1, k1), (c2, k2)) = (body[i].clone(), body[(i + rng.below(2)).min(body.len() - 1)].clone());
            let (a, b) = (random_off(rng, &s.doc, &c1, k1), random_off(rng, &s.doc, &c2, k2));
            let (a, b) = if k1 == k2 && b < a { (b, a) } else { (a, b) };
            s.sel = Selection { anchor: pos(&c1, k1, a), focus: pos(&c2, k2, b) };
            s.pending = None;
            if rng.chance(50) { ("text.delete", json!({})) } else { ("text.insert", json!({"text": format!("{marker}x")})) }
        }
        80..=89 => {
            let styles = ["Normal", "Heading 1", "Heading 2", "Title", "Quote"];
            ("para.style", json!({ "style": styles[rng.below(styles.len())] }))
        }
        90..=93 => {
            // A table command in a random cell (or a new table).
            let cells: Vec<(Cont, usize)> = everywhere.iter().filter(|(c, _)| !c.path.is_empty()).cloned().collect();
            if cells.is_empty() || rng.chance(10) {
                ("insert.table", json!({"rows": 2, "cols": 2}))
            } else {
                let (c, k) = cells[rng.below(cells.len())].clone();
                caret(s, &c, k, 0);
                let ids = [
                    "table.insertRowBelow",
                    "table.insertRowAbove",
                    "table.insertColumnRight",
                    "table.insertColumnLeft",
                    "table.deleteRow",
                    "table.deleteColumn",
                ];
                (ids[rng.below(ids.len())], json!({}))
            }
        }
        94..=96 => {
            // Comments: new, reply, edit, resolve, delete.
            let ids: Vec<u32> = s.doc.comments.keys().copied().collect();
            match (rng.below(5), ids.is_empty()) {
                (0, _) | (_, true) => {
                    let (a, b) = (random_off(rng, &s.doc, &fc, fk), random_off(rng, &s.doc, &fc, fk));
                    s.sel = Selection { anchor: pos(&fc, fk, a.min(b)), focus: pos(&fc, fk, a.max(b)) };
                    ("review.newComment", json!({"text": format!("note {me}")}))
                }
                (1, _) => ("review.reply", json!({"id": ids[rng.below(ids.len())], "text": format!("reply {me}")})),
                (2, _) => ("review.editComment", json!({"id": ids[rng.below(ids.len())], "text": format!("edited by {me}")})),
                (3, _) => ("review.resolveComment", json!({"id": ids[rng.below(ids.len())]})),
                _ => ("review.deleteComment", json!({"id": ids[rng.below(ids.len())]})),
            }
        }
        97 => ("references.footnote", json!({})),
        98 => {
            let ids = ["insert.editHeader", "insert.editFooter", "insert.closeHeader"];
            (ids[rng.below(ids.len())], json!({}))
        }
        _ => ("text.insert", json!({ "text": marker.to_string() })),
    };
    // Suggesting: some typing and deleting is tracked (this person's session only).
    let suggest = mix.objects && rng.chance(15) && matches!(id, "text.insert" | "text.backspace" | "text.delete");
    let was = s.doc.settings.track_changes;
    if suggest {
        s.doc.settings.track_changes = true;
    }
    let _ = s.run(id, &v);
    if suggest {
        s.doc.settings.track_changes = was;
    }
    id.to_string()
}

/// Give story and comment ids a command just made this client's own (`next` counts up in a range
/// no other client uses), so two people adding a comment or note at once never collide.
pub fn own_new_ids(before: &Document, doc: &mut Document, next: &mut u32) {
    let mut comments: BTreeMap<u32, u32> = BTreeMap::new();
    let mut parts: BTreeMap<u32, u32> = BTreeMap::new();
    for id in doc.comments.keys().filter(|k| !before.comments.contains_key(k)) {
        comments.insert(*id, *next);
        *next += 1;
    }
    for id in doc.parts.keys().filter(|k| !before.parts.contains_key(k)) {
        parts.insert(*id, *next);
        *next += 1;
    }
    if comments.is_empty() && parts.is_empty() {
        return;
    }
    let pm = |id: &mut u32| {
        if let Some(n) = parts.get(id) {
            *id = *n;
        }
    };
    let cm = |id: &mut u32| {
        if let Some(n) = comments.get(id) {
            *id = *n;
        }
    };
    doc.comments = std::mem::take(&mut doc.comments)
        .into_iter()
        .map(|(k, mut c)| {
            pm(&mut c.part);
            if let Some(p) = c.parent.as_mut() {
                cm(p);
            }
            (comments.get(&k).copied().unwrap_or(k), c)
        })
        .collect();
    doc.parts = std::mem::take(&mut doc.parts)
        .into_iter()
        .map(|(k, mut p)| {
            if let Some(n) = p.body.next.as_mut() {
                pm(n);
            }
            (parts.get(&k).copied().unwrap_or(k), p)
        })
        .collect();
    let refers = |o: &InlineObject| match o {
        InlineObject::CommentStart { id } | InlineObject::CommentEnd { id } => comments.contains_key(id),
        InlineObject::NoteRef { id, .. } => parts.contains_key(id),
        InlineObject::Shape { story: Some(id), .. } => parts.contains_key(id),
        _ => false,
    };
    let hf = |s: &mut wordcraft_doc::SectionProps| {
        for h in [&mut s.headers, &mut s.footers] {
            for id in [&mut h.default, &mut h.first, &mut h.even].into_iter().flatten() {
                pm(id);
            }
        }
    };
    fn fix(blocks: &mut Blocks, refers: &dyn Fn(&InlineObject) -> bool, fix_para: &dyn Fn(&mut wordcraft_doc::Paragraph)) {
        for b in blocks.iter_mut() {
            let hit = match &**b {
                Block::Para(p) => p.objects.iter().any(refers) || p.section.is_some(),
                Block::Table(_) => true,
            };
            if !hit {
                continue;
            }
            match Arc::make_mut(b) {
                Block::Para(p) => fix_para(p),
                Block::Table(t) => {
                    for r in &mut t.rows {
                        for c in &mut r.cells {
                            fix(&mut c.blocks, refers, fix_para);
                        }
                    }
                }
            }
        }
    }
    let fix_para = |p: &mut wordcraft_doc::Paragraph| {
        for o in &mut p.objects {
            match o {
                InlineObject::CommentStart { id } | InlineObject::CommentEnd { id } => cm(id),
                InlineObject::NoteRef { id, .. } => pm(id),
                InlineObject::Shape { story: Some(id), .. } => pm(id),
                _ => {}
            }
        }
        if let Some(s) = p.section.as_deref_mut() {
            hf(s);
        }
    };
    fix(&mut doc.body, &refers, &fix_para);
    for p in doc.parts.values_mut() {
        fix(&mut p.blocks, &refers, &fix_para);
    }
    let mut last = doc.last_section.clone();
    hf(&mut last);
    if last != doc.last_section {
        doc.last_section = last;
    }
}

#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub commands: usize,
    pub changes: usize,
    pub steps: BTreeMap<&'static str, usize>,
    pub remote_rebases: usize,
    pub undos: usize,
    /// Undo steps lost because what they'd restore was replaced or removed meanwhile.
    pub undo_steps_dropped: usize,
}

impl Stats {
    fn count(&mut self, steps: &[Step]) {
        for s in steps {
            *self.steps.entry(s.kind()).or_default() += 1;
            if let Step::Blocks { c, blocks, old, .. } = s
                && trace()
            {
                let t = |v: &[Arc<Block>]| v.iter().map(|b| if b.as_table().is_some() { 'T' } else { 'P' }).collect::<String>();
                println!("BLOCKS {:?} depth {} {} -> {}", c.story, c.path.len() / 3, t(old), t(blocks));
            }
        }
    }
}

pub struct Server {
    pub doc: Document,
    /// Every applied change: (client, its steps as applied).
    pub log: Vec<(usize, Vec<Step>)>,
}

impl Server {
    /// Take a change made on version `base`: transform it over everything applied since, apply,
    /// log it. Returns the steps as applied (what is broadcast).
    pub fn receive(&mut self, client: usize, base: usize, steps: Vec<Step>) -> Result<Vec<Step>, String> {
        let since: Vec<Step> = self.log[base..].iter().flat_map(|(_, s)| s.iter().cloned()).collect();
        let sent = steps.clone();
        crate::RECORD_DROPS.with(|r| r.set(true));
        let (steps, _) = xform_lists(steps, since);
        crate::RECORD_DROPS.with(|r| r.set(false));
        apply_all(&mut self.doc, &steps).map_err(|e| format!("server: {e}\n steps {}", crate::short(&steps)))?;
        if trace() {
            println!("server #{} from {client} base {base}: {}\n    as applied: {}", self.log.len(), crate::short(&sent), crate::short(&steps));
        }
        self.log.push((client, steps.clone()));
        Ok(steps)
    }
}

/// One of this client's changes as the server applied it, with the steps that undo it.
pub struct Done {
    pub seq: usize,
    pub undo: Vec<Step>,
    /// This client's marker characters the change added (for the typing checks).
    pub markers: isize,
}

pub struct Client {
    pub id: usize,
    pub session: Session,
    /// The server's document as far as this client has heard.
    pub confirmed: Document,
    pub version: usize,
    /// Every change the server broadcast, by number (to move undo steps past later edits).
    pub log: Vec<Vec<Step>>,
    /// Sent, not yet acknowledged (transformed over what arrived since sending), and whether
    /// it is an undo (undos don't go on the undo list themselves).
    pub inflight: Option<(Vec<Step>, bool)>,
    /// Made since, not sent yet.
    pub buffer: Vec<Step>,
    pub buffer_is_undo: bool,
    pub done: Vec<Done>,
    /// The next story/comment id this client gives out.
    pub next_id: u32,
}

impl Client {
    pub fn new(id: usize, doc: Document) -> Client {
        let mut session = Session::new(doc.clone());
        session.author = format!("User {id}");
        Client {
            id,
            session,
            confirmed: doc,
            version: 0,
            log: Vec::new(),
            inflight: None,
            buffer: Vec::new(),
            buffer_is_undo: false,
            done: Vec::new(),
            next_id: 10_000_000 * (id as u32 + 1),
        }
    }

    /// Run one random command and turn what it changed into pending steps.
    pub fn local(&mut self, rng: &mut Rng, mix: Mix, stats: &mut Stats) -> Result<(), String> {
        let me = self.id;
        self.edit(stats, |s| random_command(rng, s, me, mix))
    }

    /// Run `id` with `params` at the caret `at` (or where the caret is) and turn what it changed
    /// into pending steps.
    pub fn command(&mut self, at: Option<Pos>, id: &str, params: Value, stats: &mut Stats) -> Result<(), String> {
        self.edit(stats, |s| {
            if let Some(p) = at {
                s.sel = Selection::caret(p);
                s.pending = None;
                s.clamp_selection();
            }
            let _ = s.run(id, &params);
            id.to_string()
        })
    }

    fn edit(&mut self, stats: &mut Stats, run: impl FnOnce(&mut Session) -> String) -> Result<(), String> {
        if self.buffer_is_undo {
            // Keep an undo a change of its own.
            return Ok(());
        }
        let before = self.session.doc.clone();
        let id = run(&mut self.session);
        stats.commands += 1;
        own_new_ids(&before, &mut self.session.doc, &mut self.next_id);
        self.session.clamp_selection();
        let after = &self.session.doc;
        let steps = diff(&before, after);
        if steps.is_empty() {
            return Ok(());
        }
        let mut view = before;
        apply_all(&mut view, &steps).map_err(|e| format!("client {}: own steps of {id}: {e}\n {}", self.id, crate::short(&steps)))?;
        if normalized(&view) != normalized(after) {
            return Err(format!("client {}: diff of {id} does not reproduce the command's result: {}", self.id, crate::short(&steps)));
        }
        // The session continues from exactly what the steps make.
        self.session.doc = view;
        stats.changes += 1;
        stats.count(&steps);
        if trace() {
            println!("client {} {id}: {}", self.id, crate::short(&steps));
        }
        self.buffer.extend(steps);
        Ok(())
    }

    /// Undo this client's own last change that the server has applied, over everything applied
    /// since. Only with nothing pending. Returns the marker characters it takes away.
    pub fn undo(&mut self, stats: &mut Stats) -> Result<Option<isize>, String> {
        if self.inflight.is_some() || !self.buffer.is_empty() {
            return Ok(None);
        }
        let Some(d) = self.done.pop() else { return Ok(None) };
        let later: Vec<Step> = self.log[d.seq + 1..].iter().flatten().cloned().collect();
        let n = d.undo.len();
        let (steps, _) = xform_lists(d.undo, later);
        stats.undos += 1;
        stats.undo_steps_dropped += n.saturating_sub(steps.len());
        let mut view = self.confirmed.clone();
        let before = crate::sim::markers(&view, self.id) as isize;
        apply_all(&mut view, &steps).map_err(|e| format!("client {}: undo: {e}\n {}", self.id, crate::short(&steps)))?;
        let after = crate::sim::markers(&view, self.id) as isize;
        let sel = self.session.sel.clone();
        self.session.doc = view;
        self.session.sel = sel;
        self.session.clamp_selection();
        if !steps.is_empty() {
            self.buffer = steps;
            self.buffer_is_undo = true;
        }
        Ok(Some(before - after))
    }

    /// The change to send, if none is in flight.
    pub fn flush(&mut self) -> Option<(usize, Vec<Step>)> {
        if self.inflight.is_some() || self.buffer.is_empty() {
            return None;
        }
        let steps = std::mem::take(&mut self.buffer);
        self.inflight = Some((steps.clone(), self.buffer_is_undo));
        self.buffer_is_undo = false;
        Some((self.version, steps))
    }

    /// A change broadcast by the server.
    pub fn receive(&mut self, origin: usize, steps: Vec<Step>, stats: &mut Stats) -> Result<(), String> {
        let seq = self.version;
        if origin == self.id {
            let (mine, is_undo) = self.inflight.take().ok_or("an echo with nothing in flight")?;
            if mine != steps {
                return Err(format!(
                    "client {}: server transformed our change differently:\n ours {}\n server {}",
                    self.id,
                    crate::short(&mine),
                    crate::short(&steps)
                ));
            }
            if !is_undo && let Some(undo) = invert_all(&steps, &self.confirmed) {
                let before = markers(&self.confirmed, self.id) as isize;
                apply_all(&mut self.confirmed, &steps).map_err(|e| format!("client {}: confirmed: {e}", self.id))?;
                let markers = markers(&self.confirmed, self.id) as isize - before;
                self.done.push(Done { seq, undo, markers });
            } else {
                apply_all(&mut self.confirmed, &steps).map_err(|e| format!("client {}: confirmed: {e}", self.id))?;
            }
            self.version += 1;
            self.log.push(steps);
            return Ok(());
        }
        apply_all(&mut self.confirmed, &steps).map_err(|e| format!("client {}: confirmed: {e}", self.id))?;
        self.version += 1;
        self.log.push(steps.clone());
        stats.remote_rebases += 1;
        // Rebase what's pending over the remote change; the remote change, transformed over the
        // pending steps, moves the selection.
        let (inflight, was_undo) = match self.inflight.take() {
            Some((s, u)) => (Some(s), u),
            None => (None, false),
        };
        let was_inflight = inflight.is_some();
        let (inflight, r1) = xform_lists(inflight.unwrap_or_default(), steps);
        let (buffer, r2) = xform_lists(std::mem::take(&mut self.buffer), r1);
        let mut view = self.confirmed.clone();
        apply_all(&mut view, &inflight).map_err(|e| format!("client {}: rebased in-flight: {e}", self.id))?;
        apply_all(&mut view, &buffer).map_err(|e| format!("client {}: rebased buffer: {e}", self.id))?;
        self.inflight = was_inflight.then_some((inflight, was_undo));
        if buffer.is_empty() {
            self.buffer_is_undo = false;
        }
        self.buffer = buffer;
        let sel = self.session.sel.clone();
        self.session.doc = view;
        self.session.sel = Selection { anchor: map_sel(&sel.anchor, &r2), focus: map_sel(&sel.focus, &r2) };
        self.session.clamp_selection();
        Ok(())
    }
}

/// Map a selection end through `steps` (remote steps as they apply to this client's view).
fn map_sel(p: &Pos, steps: &[Step]) -> Pos {
    let Some((&k, rest)) = p.path.0.split_last() else { return p.clone() };
    let (mut c, mut k, mut off) = (Cont { story: p.story, path: rest.to_vec() }, k as usize, p.off);
    for st in steps {
        match map_pos(st, &c, k, off, -1) {
            Some((c2, k2, o2)) => (c, k, off) = (c2, k2, o2),
            None => return body_pos(0, 0),
        }
    }
    pos(&c, k, off)
}

/// How many of client `me`'s marker characters a document holds (every story).
pub fn markers(doc: &Document, me: usize) -> usize {
    let m = char::from(b'1' + me as u8);
    all_paras(doc).iter().filter_map(|(c, k)| crate::step::para_ref(doc, c, *k)).map(|p| p.text.matches(m).count()).sum()
}

pub struct Outcome {
    pub server: Document,
    pub clients: Vec<Document>,
    pub stats: Stats,
    /// Per client: markers it added minus markers it removed, by its own commands and undos.
    pub typed: Vec<isize>,
}

pub type Msg = (usize, Vec<Step>);

pub fn serve(server: &mut Server, down: &mut [VecDeque<Msg>], c: usize, (base, steps): Msg) -> Result<(), String> {
    let applied = server.receive(c, base, steps)?;
    for d in down.iter_mut() {
        d.push_back((c, applied.clone()));
    }
    Ok(())
}

/// Send, serve and deliver until nothing is pending anywhere.
pub fn drain(
    server: &mut Server,
    clients: &mut [Client],
    up: &mut [VecDeque<Msg>],
    down: &mut [VecDeque<Msg>],
    stats: &mut Stats,
) -> Result<(), String> {
    loop {
        let mut busy = false;
        for (c, client) in clients.iter_mut().enumerate() {
            if let Some(m) = client.flush() {
                up[c].push_back(m);
            }
            while let Some(m) = up[c].pop_front() {
                serve(server, down, c, m)?;
                busy = true;
            }
        }
        for (c, client) in clients.iter_mut().enumerate() {
            while let Some((o, steps)) = down[c].pop_front() {
                client.receive(o, steps, stats)?;
                busy = true;
            }
        }
        if !busy && clients.iter().all(|c| c.inflight.is_none() && c.buffer.is_empty()) {
            return Ok(());
        }
    }
}

/// A random session: `events` random actions (local commands, undos, sends, server steps,
/// deliveries in random order), then everything drained. Checks every copy ends equal to the
/// server's.
pub fn run_scenario(seed: u64, n: usize, events: usize, mix: Mix, base: &[u8]) -> Result<Outcome, String> {
    let mut rng = Rng::new(seed);
    let mut stats = Stats::default();
    let mut server = Server { doc: open(base), log: Vec::new() };
    let mut clients: Vec<Client> = (0..n).map(|i| Client::new(i, open(base))).collect();
    let mut typed = vec![0isize; n];
    let mut up: Vec<VecDeque<Msg>> = vec![VecDeque::new(); n];
    let mut down: Vec<VecDeque<Msg>> = vec![VecDeque::new(); n];
    for _ in 0..events {
        let c = rng.below(n);
        match rng.below(20) {
            0..=8 => {
                let before = markers(&clients[c].session.doc, c) as isize;
                clients[c].local(&mut rng, mix, &mut stats)?;
                typed[c] += markers(&clients[c].session.doc, c) as isize - before;
            }
            9..=10 if mix.undo => {
                if let Some(gone) = clients[c].undo(&mut stats)? {
                    typed[c] -= gone;
                }
            }
            9..=12 => {
                if let Some(m) = clients[c].flush() {
                    up[c].push_back(m);
                }
            }
            13..=14 => {
                if let Some(m) = up[c].pop_front() {
                    serve(&mut server, &mut down, c, m)?;
                }
            }
            _ => {
                if let Some((o, steps)) = down[c].pop_front() {
                    clients[c].receive(o, steps, &mut stats)?;
                }
            }
        }
    }
    drain(&mut server, &mut clients, &mut up, &mut down, &mut stats)?;
    if mix.undo {
        // Some people undo their last change now that everyone else's edits are in.
        for (c, client) in clients.iter_mut().enumerate() {
            if rng.chance(50)
                && let Some(gone) = client.undo(&mut stats)?
            {
                typed[c] -= gone;
            }
        }
        drain(&mut server, &mut clients, &mut up, &mut down, &mut stats)?;
    }
    for c in &clients {
        if c.confirmed != server.doc {
            return Err(format!("client {}: confirmed document differs from the server's", c.id));
        }
        if normalized(&c.session.doc) != normalized(&server.doc) {
            return Err(format!("client {}: its editing view differs from the server's", c.id));
        }
    }
    Ok(Outcome { server: server.doc, clients: clients.into_iter().map(|c| c.session.doc).collect(), stats, typed })
}
