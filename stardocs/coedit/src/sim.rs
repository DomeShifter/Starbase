//! An in-memory "server" and "clients" for the convergence tests: each client is a real
//! WordCraft [`Session`] running commands; after each command its change is worked out with
//! [`diff`], kept as pending steps, sent one change at a time to the server (which transforms a
//! late change over what it applied since the change's base, applies it, numbers it and
//! broadcasts it), and rebased over what the server sends.

use std::collections::VecDeque;

use serde_json::{Value, json};
use wordcraft_doc::{Block, Document, Path, Pos, StoryRef};
use wordcraft_engine::{Selection, Session};

use crate::{Step, apply_all, diff, map_block, map_pos, normalized, xform_lists};

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

/// The starting document as `.docx` bytes: a heading, paragraphs, a table, a comment and a
/// tracked insertion, made with the engine's own commands and saved by its writer.
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
    run(&mut s, "insert.table", json!({"rows": 2, "cols": 2}));
    run(&mut s, "text.insert", json!({"text": "Cell"}));
    // Back to the last body paragraph (after the table) and a closing line.
    let last = s.doc.body.len() - 1;
    if s.doc.body[last].as_para().is_none() {
        s.doc.body.push(std::sync::Arc::new(Block::Para(Default::default())));
    }
    let last = s.doc.body.len() - 1;
    caret(&mut s, last, 0);
    run(&mut s, "text.insert", json!({"text": "Closing words of the document."}));
    // A comment on "quick brown" in paragraph 1.
    s.sel = Selection { anchor: body_pos(1, 4), focus: body_pos(1, 15) };
    run(&mut s, "review.newComment", json!({"text": "Is this right?"}));
    // A tracked insertion in paragraph 2.
    run(&mut s, "review.trackChanges", json!({"value": true}));
    caret(&mut s, 2, 0);
    run(&mut s, "text.insert", json!({"text": "Tracked "}));
    run(&mut s, "review.trackChanges", json!({"value": false}));
    wordcraft_engine::io::save_bytes(DOC_NAME, &s.doc).expect("save base docx")
}

pub fn open(bytes: &[u8]) -> Document {
    wordcraft_engine::io::open_bytes(DOC_NAME, bytes).expect("open docx")
}

pub fn body_pos(k: usize, off: usize) -> Pos {
    Pos { story: StoryRef::Body, path: Path::top(k), off }
}

fn caret(s: &mut Session, k: usize, off: usize) {
    s.sel = Selection::caret(body_pos(k, off));
    s.pending = None;
    s.clamp_selection();
}

fn para_indexes(doc: &Document) -> Vec<usize> {
    doc.body.iter().enumerate().filter(|(_, b)| b.as_para().is_some()).map(|(i, _)| i).collect()
}

fn random_off(rng: &mut Rng, doc: &Document, k: usize) -> usize {
    let t = &doc.body[k].as_para().expect("a paragraph").text;
    let mut o = rng.below(t.len() + 1);
    while !t.is_char_boundary(o) {
        o -= 1;
    }
    o
}

/// What kinds of commands the random clients run.
#[derive(Clone, Copy, Debug)]
pub struct Mix {
    /// Backspace, Delete and typing over / deleting a selection.
    pub deletes: bool,
    /// Comments and tables (whole-block / Meta steps).
    pub objects: bool,
}

/// One random command on a client's session. Returns the command's id (for failure reports).
fn random_command(rng: &mut Rng, s: &mut Session, me: usize, mix: Mix) -> String {
    let paras = para_indexes(&s.doc);
    if !mix.deletes {
        // Typing over a selection left by Bold would delete it.
        s.sel = Selection::caret(s.sel.focus.clone());
    }
    // Mostly keep the caret in a top-level paragraph.
    if s.sel.focus.path.0.len() > 1 && rng.chance(70) || rng.chance(25) {
        let k = paras[rng.below(paras.len())];
        let o = random_off(rng, &s.doc, k);
        caret(s, k, o);
    }
    let marker = char::from(b'1' + me as u8);
    let mut pick = rng.below(100);
    if !mix.deletes && (55..80).contains(&pick) {
        pick = rng.below(55);
    }
    let (id, v) = match pick {
        0..=34 => {
            let n = 1 + rng.below(4);
            let alphabet = [marker, marker, 'a', 'e', ' ', 'é', '日'];
            let t: String = (0..n).map(|_| alphabet[rng.below(alphabet.len())]).collect();
            ("text.insert", json!({ "text": t }))
        }
        35..=44 => ("text.newParagraph", json!({})),
        45..=54 => {
            // Bold or italic over a range in one paragraph.
            let k = paras[rng.below(paras.len())];
            let (a, b) = (random_off(rng, &s.doc, k), random_off(rng, &s.doc, k));
            s.sel = Selection { anchor: body_pos(k, a.min(b)), focus: body_pos(k, a.max(b)) };
            s.pending = None;
            (if rng.chance(50) { "format.bold" } else { "format.italic" }, json!({}))
        }
        55..=66 => (if rng.chance(60) { "text.backspace" } else { "text.delete" }, json!({})),
        67..=79 => {
            // A selection over one or two paragraphs, deleted or typed over.
            let i = rng.below(paras.len());
            let (k1, k2) = (paras[i], paras[(i + rng.below(2)).min(paras.len() - 1)]);
            let (a, b) = (random_off(rng, &s.doc, k1), random_off(rng, &s.doc, k2));
            let (a, b) = if k1 == k2 && b < a { (b, a) } else { (a, b) };
            s.sel = Selection { anchor: body_pos(k1, a), focus: body_pos(k2, b) };
            s.pending = None;
            if rng.chance(50) { ("text.delete", json!({})) } else { ("text.insert", json!({"text": format!("{marker}x")})) }
        }
        80..=91 => {
            let styles = ["Normal", "Heading 1", "Heading 2", "Title", "Quote"];
            ("para.style", json!({ "style": styles[rng.below(styles.len())] }))
        }
        92..=95 if mix.objects => {
            let k = paras[rng.below(paras.len())];
            let (a, b) = (random_off(rng, &s.doc, k), random_off(rng, &s.doc, k));
            s.sel = Selection { anchor: body_pos(k, a.min(b)), focus: body_pos(k, a.max(b)) };
            ("review.newComment", json!({"text": format!("note {me}")}))
        }
        96..=97 if mix.objects => ("insert.table", json!({"rows": 2, "cols": 2})),
        _ => ("text.insert", json!({ "text": marker.to_string() })),
    };
    let _ = s.run(id, &v);
    id.to_string()
}

#[derive(Default, Debug, Clone)]
pub struct Stats {
    pub commands: usize,
    pub changes: usize,
    pub steps: [usize; 7],
    pub remote_rebases: usize,
    /// Whole-block replaces with no table in them (paragraphs added or removed outright).
    pub blocks_paras_only: usize,
}

impl Stats {
    fn count(&mut self, steps: &[Step]) {
        for s in steps {
            let i = match s {
                Step::Splice { .. } => 0,
                Step::Format { .. } => 1,
                Step::ParaProps { .. } => 2,
                Step::Split { .. } => 3,
                Step::Join { .. } => 4,
                Step::Blocks { blocks, old, .. } => {
                    if blocks.iter().chain(old).all(|b| b.as_para().is_some()) {
                        self.blocks_paras_only += 1;
                    }
                    5
                }
                Step::Meta(_) => 6,
            };
            self.steps[i] += 1;
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
        crate::RECORD_DROPS.with(|r| r.set(true));
        let (steps, _) = xform_lists(steps, since);
        crate::RECORD_DROPS.with(|r| r.set(false));
        apply_all(&mut self.doc, &steps).map_err(|e| format!("server: {e}"))?;
        self.log.push((client, steps.clone()));
        Ok(steps)
    }
}

pub struct Client {
    pub id: usize,
    pub session: Session,
    /// The server's document as far as this client has heard.
    pub confirmed: Document,
    pub version: usize,
    /// Sent, not yet acknowledged (transformed over what arrived since sending).
    pub inflight: Option<Vec<Step>>,
    /// Made since, not sent yet.
    pub buffer: Vec<Step>,
}

impl Client {
    pub fn new(id: usize, doc: Document) -> Client {
        let mut session = Session::new(doc.clone());
        session.author = format!("User {id}");
        Client { id, session, confirmed: doc, version: 0, inflight: None, buffer: Vec::new() }
    }

    /// Run one random command and turn what it changed into pending steps.
    pub fn local(&mut self, rng: &mut Rng, mix: Mix, stats: &mut Stats) -> Result<(), String> {
        let before = self.session.doc.clone();
        let id = random_command(rng, &mut self.session, self.id, mix);
        stats.commands += 1;
        let after = &self.session.doc;
        let steps = diff(&before, after);
        if steps.is_empty() {
            return Ok(());
        }
        let mut view = before;
        apply_all(&mut view, &steps).map_err(|e| format!("client {}: own steps of {id}: {e}", self.id))?;
        if normalized(&view) != normalized(after) {
            return Err(format!("client {}: diff of {id} does not reproduce the command's result: {steps:?}", self.id));
        }
        // The session continues from exactly what the steps make.
        self.session.doc = view;
        stats.changes += 1;
        stats.count(&steps);
        self.buffer.extend(steps);
        Ok(())
    }

    /// The change to send, if none is in flight.
    pub fn flush(&mut self) -> Option<(usize, Vec<Step>)> {
        if self.inflight.is_some() || self.buffer.is_empty() {
            return None;
        }
        let steps = std::mem::take(&mut self.buffer);
        self.inflight = Some(steps.clone());
        Some((self.version, steps))
    }

    /// A change broadcast by the server.
    pub fn receive(&mut self, origin: usize, steps: Vec<Step>, stats: &mut Stats) -> Result<(), String> {
        apply_all(&mut self.confirmed, &steps).map_err(|e| format!("client {}: confirmed: {e}", self.id))?;
        self.version += 1;
        if origin == self.id {
            let mine = self.inflight.take().ok_or("an echo with nothing in flight")?;
            if mine != steps {
                return Err(format!("client {}: server transformed our change differently:\n ours {mine:?}\n server {steps:?}", self.id));
            }
            return Ok(());
        }
        stats.remote_rebases += 1;
        // Rebase what's pending over the remote change; the remote change, transformed over the
        // pending steps, moves the selection.
        let was_inflight = self.inflight.is_some();
        let (inflight, r1) = xform_lists(self.inflight.take().unwrap_or_default(), steps);
        let (buffer, r2) = xform_lists(std::mem::take(&mut self.buffer), r1);
        let mut view = self.confirmed.clone();
        apply_all(&mut view, &inflight).map_err(|e| format!("client {}: rebased in-flight: {e}", self.id))?;
        apply_all(&mut view, &buffer).map_err(|e| format!("client {}: rebased buffer: {e}", self.id))?;
        self.inflight = was_inflight.then_some(inflight);
        self.buffer = buffer;
        let sel = self.session.sel.clone();
        self.session.doc = view;
        self.session.sel = Selection { anchor: map_sel(&sel.anchor, &r2, &self.session.doc), focus: map_sel(&sel.focus, &r2, &self.session.doc) };
        self.session.clamp_selection();
        Ok(())
    }
}

/// Map a selection end through `steps` (remote steps as they apply to this client's view).
fn map_sel(p: &Pos, steps: &[Step], doc: &Document) -> Pos {
    if p.story != StoryRef::Body || p.path.0.is_empty() {
        return doc.clamp(p);
    }
    let (mut k, mut off) = (p.path.0[0] as usize, p.off);
    let nested = p.path.0.len() > 1;
    for st in steps {
        let mapped = if nested { map_block(st, k).map(|k| (k, off)) } else { map_pos(st, k, off, -1) };
        match mapped {
            Some((k2, o2)) => (k, off) = (k2, o2),
            None => return doc.clamp(&body_pos(k.min(doc.body.len().saturating_sub(1)), 0)),
        }
    }
    let mut path = p.path.clone();
    path.0[0] = k as u32;
    doc.clamp(&Pos { story: StoryRef::Body, path, off })
}

/// How many of client `me`'s marker characters a document holds.
pub fn markers(doc: &Document, me: usize) -> usize {
    let m = char::from(b'1' + me as u8);
    doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.text.matches(m).count()).sum()
}

pub struct Outcome {
    pub server: Document,
    pub clients: Vec<Document>,
    pub stats: Stats,
    /// Per client: markers it added minus markers it removed, by its own commands.
    pub typed: Vec<isize>,
}

type Msg = (usize, Vec<Step>);

fn serve(server: &mut Server, down: &mut [VecDeque<Msg>], c: usize, (base, steps): Msg) -> Result<(), String> {
    let applied = server.receive(c, base, steps)?;
    for d in down.iter_mut() {
        d.push_back((c, applied.clone()));
    }
    Ok(())
}

/// A random session: `events` random actions (local commands, sends, server steps, deliveries
/// in random order), then everything drained. Checks every copy ends equal to the server's.
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
        match rng.below(10) {
            0..=4 => {
                let before = markers(&clients[c].session.doc, c) as isize;
                clients[c].local(&mut rng, mix, &mut stats)?;
                typed[c] += markers(&clients[c].session.doc, c) as isize - before;
            }
            5..=6 => {
                if let Some(m) = clients[c].flush() {
                    up[c].push_back(m);
                }
            }
            7 => {
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
    // Drain: send, serve and deliver until nothing is pending anywhere.
    loop {
        let mut busy = false;
        for c in 0..n {
            if let Some(m) = clients[c].flush() {
                up[c].push_back(m);
            }
            while let Some(m) = up[c].pop_front() {
                serve(&mut server, &mut down, c, m)?;
                busy = true;
            }
        }
        for (c, client) in clients.iter_mut().enumerate() {
            while let Some((o, steps)) = down[c].pop_front() {
                client.receive(o, steps, &mut stats)?;
                busy = true;
            }
        }
        if !busy && clients.iter().all(|c| c.inflight.is_none() && c.buffer.is_empty()) {
            break;
        }
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
