//! The server's side of one open document, as a pure state machine (no sockets, no files, no
//! clock): [`DocRoom`] orders changes, transforms a late one over what was applied since its
//! base, applies it, keeps the op log (numbered `seq` from 1, trimmed to the last `keep`
//! entries), answers a reconnecting client with the log tail or "too old: send the whole
//! document", fingerprints the document now and then, and keeps each person's caret mapped
//! through every change.
//!
//! Authors are plain numbers here; the server maps its connection names to them.

use std::collections::{BTreeMap, VecDeque};

use starbase_docs_sync::Pos as WirePos;
use wordcraft_doc::{Document, StoryRef};

use crate::step::{Cont, Step, apply_all};
use crate::{map_pos, xform_lists};

/// One applied change: its number, who made it, the steps as applied (after transforming).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub seq: u64,
    pub author: usize,
    pub steps: Vec<Step>,
}

/// A position in the document: container, paragraph (block index in it), byte offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caret {
    pub cont: Cont,
    pub para: usize,
    pub offset: usize,
}

impl Caret {
    pub fn to_wire(&self) -> WirePos {
        let story = match self.cont.story {
            StoryRef::Body => None,
            StoryRef::Part(id) => Some(id),
        };
        WirePos { story, path: self.cont.path.clone(), para: self.para as u32, offset: self.offset as u32 }
    }
    pub fn from_wire(p: &WirePos) -> Caret {
        let story = p.story.map_or(StoryRef::Body, StoryRef::Part);
        Caret { cont: Cont { story, path: p.path.clone() }, para: p.para as usize, offset: p.offset as usize }
    }
    /// Through one step; `None` when the paragraph, table part or story is gone.
    fn mapped(&self, s: &Step) -> Option<Caret> {
        map_pos(s, &self.cont, self.para, self.offset, 1).map(|(cont, para, offset)| Caret { cont, para, offset })
    }
}

/// Someone's caret and, when text is selected, the other end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Where {
    pub caret: Caret,
    pub selection: Option<Caret>,
}

/// What the room did with a change.
#[derive(Clone, Debug, PartialEq)]
pub enum Received {
    /// Applied as `seq`; `steps` is what everyone applies. `check`: the document's fingerprint
    /// at `seq`, every `check_every` changes.
    Applied { seq: u64, steps: Vec<Step>, check: Option<u64> },
    /// This author's `cseq` was already applied (a re-sent message): nothing happened; `seq` is
    /// where it went, to acknowledge again.
    Duplicate { seq: u64 },
}

/// What a reconnecting client needs.
#[derive(Clone, Debug, PartialEq)]
pub enum Catchup {
    /// The changes after its `last_seq`, in order (empty: up to date).
    Tail(Vec<Entry>),
    /// The log doesn't go back that far (or the client is ahead of the server, or has nothing):
    /// send the whole document.
    Full,
}

pub struct DocRoom {
    pub doc: Document,
    log: VecDeque<Entry>,
    /// Number of the last change trimmed from the log (0: nothing trimmed).
    floor: u64,
    seq: u64,
    keep: usize,
    check_every: u64,
    /// Per author: the last `cseq` applied and the `seq` it got.
    last: BTreeMap<usize, (u64, u64)>,
    wheres: BTreeMap<usize, Where>,
}

impl DocRoom {
    /// A room that keeps every change and never trims.
    pub fn new(doc: Document) -> DocRoom {
        DocRoom::with_limits(doc, usize::MAX, 0)
    }

    /// Keep the last `keep` changes for reconnecting clients; `check_every` (0: never) changes
    /// between fingerprints.
    pub fn with_limits(doc: Document, keep: usize, check_every: u64) -> DocRoom {
        DocRoom { doc, log: VecDeque::new(), floor: 0, seq: 0, keep, check_every, last: BTreeMap::new(), wheres: BTreeMap::new() }
    }

    /// The number of the last change applied.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn log(&self) -> impl Iterator<Item = &Entry> {
        self.log.iter()
    }

    /// Take change `cseq` of `author`, made on server revision `base`: transform it over
    /// everything applied since, apply, log, and map everyone else's caret through it.
    /// Errors (the change is dropped, the document untouched): `base` ahead of the server, or
    /// older than the log goes back (the client must resync with [`DocRoom::catch_up`]), or the
    /// steps don't apply.
    pub fn receive(&mut self, author: usize, cseq: u64, base: u64, steps: Vec<Step>) -> Result<Received, String> {
        if let Some(&(c, seq)) = self.last.get(&author)
            && cseq <= c
        {
            return Ok(Received::Duplicate { seq });
        }
        if base > self.seq {
            return Err(format!("base {base} is ahead of the server's {}", self.seq));
        }
        if base < self.floor {
            return Err(format!("base {base} is too old (the log starts after {})", self.floor));
        }
        let skip = (base - self.floor) as usize;
        let since: Vec<Step> = self.log.iter().skip(skip).flat_map(|e| e.steps.iter().cloned()).collect();
        crate::RECORD_DROPS.with(|r| r.set(true));
        let (steps, _) = xform_lists(steps, since);
        crate::RECORD_DROPS.with(|r| r.set(false));
        // Apply to a copy first: a failing change must not leave the document half changed.
        let mut next = self.doc.clone();
        apply_all(&mut next, &steps).map_err(|e| format!("server: {e}\n steps {}", crate::short(&steps)))?;
        self.doc = next;
        self.seq += 1;
        let seq = self.seq;
        self.last.insert(author, (cseq, seq));
        let gone: Vec<usize> = self
            .wheres
            .iter_mut()
            .filter(|(who, _)| **who != author)
            .filter_map(|(who, w)| match map_where(w, &steps) {
                Some(m) => {
                    *w = m;
                    None
                }
                None => Some(*who),
            })
            .collect();
        for who in gone {
            self.wheres.remove(&who);
        }
        self.log.push_back(Entry { seq, author, steps: steps.clone() });
        while self.log.len() > self.keep {
            self.log.pop_front();
            self.floor += 1;
        }
        let check = (self.check_every > 0 && seq.is_multiple_of(self.check_every)).then(|| self.hash().ok()).flatten();
        Ok(Received::Applied { seq, steps, check })
    }

    /// [`DocRoom::receive`] for callers that don't number their messages (the simulation): the
    /// author's next `cseq` is taken. Returns the steps as applied.
    pub fn submit(&mut self, author: usize, base: usize, steps: Vec<Step>) -> Result<Vec<Step>, String> {
        let cseq = self.last.get(&author).map_or(1, |(c, _)| c + 1);
        match self.receive(author, cseq, base as u64, steps)? {
            Received::Applied { steps, .. } => Ok(steps),
            Received::Duplicate { .. } => Err("duplicate".into()),
        }
    }

    /// What a client that has applied everything up to `last_seq` needs (0: nothing).
    pub fn catch_up(&self, last_seq: u64) -> Catchup {
        if last_seq == 0 || last_seq > self.seq || last_seq < self.floor {
            return Catchup::Full;
        }
        Catchup::Tail(self.log.iter().skip((last_seq - self.floor) as usize).cloned().collect())
    }

    /// The document's fingerprint (of its saved bytes), to compare with a client's.
    pub fn hash(&self) -> Result<u64, String> {
        let bytes = wordcraft_engine::io::save_bytes(crate::sim::DOC_NAME, &self.doc)?;
        Ok(starbase_docs_sync::hash(&bytes))
    }

    /// Where `author` says they are now (positions are as of the current `seq`: the client maps
    /// its own pending steps before sending).
    pub fn set_presence(&mut self, author: usize, at: Where) {
        self.wheres.insert(author, at);
    }

    pub fn presence(&self, author: usize) -> Option<&Where> {
        self.wheres.get(&author)
    }

    pub fn leave(&mut self, author: usize) {
        // `last` stays: a client's `cseq` only grows, also across a reconnect, so a change it
        // re-sends because the acknowledgement was lost is recognised.
        self.wheres.remove(&author);
    }
}

/// Through all `steps`; `None` when the caret itself is gone (the selection's other end alone
/// is just dropped).
fn map_where(w: &Where, steps: &[Step]) -> Option<Where> {
    let through = |c: &Caret| steps.iter().try_fold(c.clone(), |c, s| c.mapped(s));
    Some(Where { caret: through(&w.caret)?, selection: w.selection.as_ref().and_then(through) })
}
