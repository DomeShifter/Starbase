//! SPIKE A of the Docs co-editing design (not for release): editing *steps* worked out by
//! comparing a WordCraft document before and after a command, applying them, mapping positions
//! through them, and transforming a step over a concurrent one (the server orders changes, the
//! clients rebase their pending steps over what the server sent).
//!
//! Scope of the spike: the body story's top-level blocks. Paragraph text, runs and inline objects
//! (comment anchors included) are edited by [`Step::Splice`] / [`Step::Format`]; paragraph
//! formatting by [`Step::ParaProps`]; Enter and joining paragraphs by [`Step::Split`] /
//! [`Step::Join`]. Everything else in the body (tables, several paragraphs pasted, a table
//! inserted) is a whole-block replace ([`Step::Blocks`], TODO: cell-level steps), and every
//! non-body field of the document (comments, revisions, styles, settings, parts…) is one
//! last-writer-wins [`Step::Meta`] (TODO: per-field / per-comment steps, revision index rewrite).

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::Arc;

use wordcraft_doc::{Block, CharProps, Document, InlineObject, ParaProps, Paragraph, Run, SectionProps};

pub mod sim;

/// One editing step on the body of a document. Block indexes are top-level body blocks; offsets
/// are UTF-8 byte offsets into a paragraph's text.
// Spike: ParaProps/Meta make variants big; the real build boxes them (steps are few per command).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Replace bytes `from..to` of paragraph `block` with `text`, formatted by `runs` (covering
    /// exactly `text`) and with `objects` for its U+FFFC characters.
    Splice { block: usize, from: usize, to: usize, text: String, runs: Vec<Run>, objects: Vec<InlineObject> },
    /// Set the character formatting of byte ranges of paragraph `block` (text unchanged).
    Format { block: usize, segs: Vec<(usize, usize, CharProps)> },
    /// Set paragraph `block`'s own formatting (last writer wins).
    ParaProps { block: usize, props: ParaProps, mark: CharProps, section: Option<Box<SectionProps>> },
    /// Split paragraph `block` at `off` (Enter): the tail becomes a new paragraph after it.
    Split { block: usize, off: usize },
    /// Join paragraph `block + 1` onto the end of paragraph `block`, whose length is `at`.
    Join { block: usize, at: usize },
    /// Replace blocks `from..to` (which are `old`) with `blocks` (tables, anything the other
    /// steps don't express). `old` lets a concurrent whole-block replace of an overlapping range
    /// be undone and redone (see [`xform`]); it is kept up to date as the step is transformed.
    Blocks { from: usize, to: usize, blocks: Vec<Arc<Block>>, old: Vec<Arc<Block>> },
    /// Everything outside the body, as a document with an empty body (last writer wins).
    Meta(Box<Document>),
}

impl Step {
    /// A one-line description for failure reports (formatting shown as b/i only).
    pub fn short(&self) -> String {
        let cp = |p: &CharProps| format!("{}{}", if p.bold == Some(true) { "b" } else { "" }, if p.italic == Some(true) { "i" } else { "" });
        match self {
            Step::Splice { block, from, to, text, runs, .. } => {
                format!(
                    "Splice({block}:{from}..{to} {text:?} [{}])",
                    runs.iter().map(|r| format!("{}{}", r.len, cp(&r.props))).collect::<Vec<_>>().join(",")
                )
            }
            Step::Format { block, segs } => {
                format!("Format({block}: {})", segs.iter().map(|(f, t, p)| format!("{f}..{t}={}", cp(p))).collect::<Vec<_>>().join(","))
            }
            Step::ParaProps { block, props, .. } => format!("ParaProps({block}: {:?})", props.style),
            Step::Split { block, off } => format!("Split({block}@{off})"),
            Step::Join { block, at } => format!("Join({block}@{at})"),
            Step::Blocks { from, to, blocks, old } => format!("Blocks({from}..{to} -> {} blocks, old {})", blocks.len(), old.len()),
            Step::Meta(m) => format!("Meta(track={})", m.settings.track_changes),
        }
    }
}

pub fn short(steps: &[Step]) -> String {
    steps.iter().map(Step::short).collect::<Vec<_>>().join(" ")
}

thread_local! {
    /// While set (the server mapping a late change), steps of that change dropped by [`xform`]
    /// because what they edited was replaced or removed concurrently are counted in [`DROPPED`]
    /// by (dropped step, concurrent step) kind (statistics for the spike's report).
    pub static RECORD_DROPS: Cell<bool> = const { Cell::new(false) };
    pub static DROPPED: RefCell<BTreeMap<(&'static str, &'static str), usize>> = const { RefCell::new(BTreeMap::new()) };
}

impl Step {
    pub fn kind(&self) -> &'static str {
        match self {
            Step::Splice { .. } => "splice",
            Step::Format { .. } => "format",
            Step::ParaProps { .. } => "paraProps",
            Step::Split { .. } => "split",
            Step::Join { .. } => "join",
            Step::Blocks { .. } => "blocks",
            Step::Meta(_) => "meta",
        }
    }
}

fn dropped(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    if a_after && RECORD_DROPS.with(Cell::get) {
        DROPPED.with(|d| *d.borrow_mut().entry((a.kind(), b.kind())).or_default() += 1);
    }
    Vec::new()
}

// ------------------------------------------------------------------------------------------
// Paragraph helpers

const OBJ: char = '\u{FFFC}';

fn objs_in(s: &str) -> usize {
    s.matches(OBJ).count()
}

/// The runs covering bytes `a..b` of a paragraph whose runs cover its text.
fn runs_slice(runs: &[Run], a: usize, b: usize) -> Vec<Run> {
    let mut out = Vec::new();
    let mut pos = 0;
    for r in runs {
        let (s, e) = (pos, pos + r.len);
        pos = e;
        let (lo, hi) = (s.max(a), e.min(b));
        if lo < hi {
            out.push(Run { len: hi - lo, props: r.props.clone() });
        }
    }
    out
}

fn bad(what: &str) -> String {
    format!("step does not apply: {what}")
}

fn check_range(p: &Paragraph, from: usize, to: usize) -> Result<(), String> {
    if from > to || to > p.text.len() || !p.text.is_char_boundary(from) || !p.text.is_char_boundary(to) {
        return Err(bad(&format!("range {from}..{to} in a paragraph of {} bytes", p.text.len())));
    }
    if p.objects.len() != objs_in(&p.text) {
        return Err(bad("paragraph objects don't match its U+FFFC count"));
    }
    Ok(())
}

pub fn splice_para(p: &mut Paragraph, from: usize, to: usize, text: &str, runs: &[Run], objects: &[InlineObject]) -> Result<(), String> {
    p.normalize();
    check_range(p, from, to)?;
    if objs_in(text) != objects.len() || runs.iter().map(|r| r.len).sum::<usize>() != text.len() {
        return Err(bad("splice payload is inconsistent"));
    }
    let k0 = objs_in(&p.text[..from]);
    let kd = objs_in(&p.text[from..to]);
    p.objects.splice(k0..k0 + kd, objects.iter().cloned());
    let mut nr = runs_slice(&p.runs, 0, from);
    nr.extend(runs.iter().cloned());
    nr.extend(runs_slice(&p.runs, to, usize::MAX));
    p.text.replace_range(from..to, text);
    p.runs = nr;
    p.normalize();
    p.touch();
    Ok(())
}

fn format_para(p: &mut Paragraph, segs: &[(usize, usize, CharProps)]) -> Result<(), String> {
    p.normalize();
    for (f, t, props) in segs {
        check_range(p, *f, *t)?;
        let mut nr = runs_slice(&p.runs, 0, *f);
        if t > f {
            nr.push(Run { len: t - f, props: props.clone() });
        }
        nr.extend(runs_slice(&p.runs, *t, usize::MAX));
        p.runs = nr;
        p.normalize();
    }
    p.touch();
    Ok(())
}

/// Split `p` at `off`; returns the tail paragraph (same paragraph formatting; the section break,
/// if any, moves to the tail, which now ends the paragraph).
pub fn split_para(p: &mut Paragraph, off: usize) -> Result<Paragraph, String> {
    p.normalize();
    check_range(p, off, off)?;
    let k = objs_in(&p.text[..off]);
    let tail = Paragraph {
        text: p.text[off..].to_string(),
        runs: runs_slice(&p.runs, off, usize::MAX),
        props: p.props.clone(),
        mark: p.mark.clone(),
        objects: p.objects.split_off(k),
        section: p.section.take(),
        ..Paragraph::default()
    };
    p.text.truncate(off);
    p.runs = runs_slice(&p.runs, 0, off);
    p.normalize();
    p.touch();
    let mut tail = tail;
    tail.normalize();
    Ok(tail)
}

pub fn join_para(p: &mut Paragraph, q: &Paragraph) {
    p.normalize();
    let mut q = q.clone();
    q.normalize();
    p.text.push_str(&q.text);
    p.runs.extend(q.runs);
    p.objects.extend(q.objects);
    if q.section.is_some() {
        p.section = q.section;
    }
    p.normalize();
    p.touch();
}

fn para_mut(doc: &mut Document, k: usize) -> Result<&mut Paragraph, String> {
    match doc.body.get_mut(k).map(Arc::make_mut) {
        Some(Block::Para(p)) => Ok(p),
        Some(_) => Err(bad(&format!("block {k} is not a paragraph"))),
        None => Err(bad(&format!("no block {k}"))),
    }
}

fn para_ref(doc: &Document, k: usize) -> Result<&Paragraph, String> {
    match doc.body.get(k).map(|b| &**b) {
        Some(Block::Para(p)) => Ok(p),
        _ => Err(bad(&format!("block {k} is not a paragraph"))),
    }
}

// ------------------------------------------------------------------------------------------
// Apply

pub fn apply(doc: &mut Document, s: &Step) -> Result<(), String> {
    match s {
        Step::Splice { block, from, to, text, runs, objects } => splice_para(para_mut(doc, *block)?, *from, *to, text, runs, objects),
        Step::Format { block, segs } => format_para(para_mut(doc, *block)?, segs),
        Step::ParaProps { block, props, mark, section } => {
            let p = para_mut(doc, *block)?;
            p.props = props.clone();
            p.mark = mark.clone();
            p.section = section.clone();
            p.touch();
            Ok(())
        }
        Step::Split { block, off } => {
            let tail = split_para(para_mut(doc, *block)?, *off)?;
            doc.body.insert(block + 1, Arc::new(Block::Para(tail)));
            Ok(())
        }
        Step::Join { block, at } => {
            let q = para_ref(doc, block + 1)?.clone();
            let p = para_mut(doc, *block)?;
            if p.text.len() != *at {
                return Err(bad(&format!("join expected a paragraph of {at} bytes, found {}", p.text.len())));
            }
            join_para(p, &q);
            doc.body.remove(block + 1);
            Ok(())
        }
        Step::Blocks { from, to, blocks, .. } => {
            if from > to || *to > doc.body.len() {
                return Err(bad("block range"));
            }
            doc.body.splice(*from..*to, blocks.iter().cloned());
            Ok(())
        }
        Step::Meta(m) => {
            let body = std::mem::take(&mut doc.body);
            *doc = (**m).clone();
            doc.body = body;
            Ok(())
        }
    }
}

pub fn apply_all(doc: &mut Document, steps: &[Step]) -> Result<(), String> {
    steps.iter().try_for_each(|s| apply(doc, s))
}

/// A copy with every body paragraph's runs normalised (the engine sometimes leaves adjacent runs
/// with equal formatting unmerged; the steps always merge them).
pub fn normalized(doc: &Document) -> Document {
    let mut d = doc.clone();
    for b in &mut d.body {
        if let Block::Para(p) = &**b {
            let mut q = p.clone();
            q.normalize();
            if q.runs != p.runs {
                *b = Arc::new(Block::Para(q));
            }
        }
    }
    d
}

// ------------------------------------------------------------------------------------------
// Diff

fn same(a: &Arc<Block>, b: &Arc<Block>) -> bool {
    Arc::ptr_eq(a, b) || **a == **b
}

fn meta_of(d: &Document) -> Document {
    let mut m = d.clone();
    m.body = Vec::new();
    m
}

/// The steps that turn `before` into `after` (up to run normalisation).
pub fn diff(before: &Document, after: &Document) -> Vec<Step> {
    let mut steps = Vec::new();
    let (bb, ab) = (&before.body, &after.body);
    let (bn, an) = (bb.len(), ab.len());
    let mut i = 0;
    while i < bn.min(an) && same(&bb[i], &ab[i]) {
        i += 1;
    }
    let mut s = 0;
    while s < bn.min(an) - i && same(&bb[bn - 1 - s], &ab[an - 1 - s]) {
        s += 1;
    }
    let (mut m, mut n) = (bn - i - s, an - i - s);
    // Paragraphs only added (Enter at a paragraph's end) or only removed: widen by one
    // neighbouring paragraph so it comes out as a Split / Join, which concurrent typing maps
    // through, instead of a whole-block replace.
    if (m == 0) != (n == 0) {
        if i > 0 && bb[i - 1].as_para().is_some() {
            i -= 1;
        } else if s > 0 && bb[bn - s].as_para().is_some() {
            s -= 1;
        }
        (m, n) = (bn - i - s, an - i - s);
    }
    fn paras(blocks: &[Arc<Block>]) -> Option<Vec<&Paragraph>> {
        blocks.iter().map(|b| b.as_para()).collect()
    }
    let (bp, ap) = (paras(&bb[i..i + m]), paras(&ab[i..i + n]));
    match (bp, ap) {
        _ if m == 0 && n == 0 => {}
        (Some(bp), Some(ap)) if m == n => {
            for (j, (b, a)) in bp.iter().zip(&ap).enumerate() {
                para_diff(i + j, b, a, &mut steps);
            }
        }
        (Some(bp), Some(ap)) if m > 0 && n > 0 => {
            // Join the changed paragraphs into one, split it where the new ones start, then
            // fix each one up: Enter and Backspace/Delete at a paragraph edge come out as one
            // Split / Join and nothing else.
            let mut work = bp[0].clone();
            for q in &bp[1..] {
                steps.push(Step::Join { block: i, at: work.text.len() });
                join_para(&mut work, q);
            }
            for (k, target) in ap.iter().enumerate().take(n - 1) {
                let mut off = work.text.bytes().zip(target.text.bytes()).take_while(|(x, y)| x == y).count();
                while !work.text.is_char_boundary(off) {
                    off -= 1;
                }
                steps.push(Step::Split { block: i + k, off });
                let tail = split_para(&mut work, off).expect("split at a char boundary");
                para_diff(i + k, &work, target, &mut steps);
                work = tail;
            }
            para_diff(i + n - 1, &work, ap[n - 1], &mut steps);
        }
        _ => steps.push(Step::Blocks { from: i, to: i + m, blocks: ab[i..i + n].to_vec(), old: bb[i..i + m].to_vec() }),
    }
    if meta_of(before) != meta_of(after) {
        steps.push(Step::Meta(Box::new(meta_of(after))));
    }
    steps
}

/// Byte offsets of the U+FFFC characters in `s`.
fn obj_offsets(s: &str) -> Vec<usize> {
    s.match_indices(OBJ).map(|(i, _)| i).collect()
}

fn para_diff(block: usize, b: &Paragraph, a: &Paragraph, out: &mut Vec<Step>) {
    if b == a {
        return;
    }
    let (bt, at) = (b.text.as_bytes(), a.text.as_bytes());
    let mut p = bt.iter().zip(at).take_while(|(x, y)| x == y).count();
    while !b.text.is_char_boundary(p) {
        p -= 1;
    }
    // Objects in the common prefix must be equal too, else the prefix stops before the first
    // one that differs.
    let (bo, ao) = (obj_offsets(&b.text), obj_offsets(&a.text));
    for (k, &off) in bo.iter().enumerate() {
        if off >= p {
            break;
        }
        if b.objects.get(k) != a.objects.get(k) {
            p = off;
            break;
        }
    }
    let max_s = bt.len().min(at.len()) - p;
    let mut s = bt.iter().rev().zip(at.iter().rev()).take(max_s).take_while(|(x, y)| x == y).count();
    while !b.text.is_char_boundary(bt.len() - s) {
        s -= 1;
    }
    for k in 0..bo.len() {
        let (ob, oa) = (bo[bo.len() - 1 - k], ao.get(ao.len().wrapping_sub(1 + k)));
        if ob < bt.len() - s {
            break;
        }
        let (kb, ka) = (b.objects.len() - 1 - k, a.objects.len().wrapping_sub(1 + k));
        if oa.is_none() || b.objects.get(kb) != a.objects.get(ka) {
            s = bt.len() - ob - OBJ.len_utf8();
            break;
        }
    }
    let (from, to, ato) = (p, bt.len() - s, at.len() - s);
    let mut work = b.clone();
    if from < to || from < ato {
        let text = a.text[from..ato].to_string();
        let k0 = objs_in(&a.text[..from]);
        let objects = a.objects[k0..k0 + objs_in(&text)].to_vec();
        let mut ar = a.clone();
        ar.normalize();
        let runs = runs_slice(&ar.runs, from, ato);
        splice_para(&mut work, from, to, &text, &runs, &objects).expect("diff splice applies");
        out.push(Step::Splice { block, from, to, text, runs, objects });
    }
    // Formatting differences left over (same text now).
    let mut ar = a.clone();
    ar.normalize();
    work.normalize();
    debug_assert_eq!(work.text, ar.text);
    let mut bounds: Vec<usize> = vec![0];
    for runs in [&work.runs, &ar.runs] {
        let mut acc = 0;
        for r in runs {
            acc += r.len;
            bounds.push(acc);
        }
    }
    bounds.sort_unstable();
    bounds.dedup();
    let mut segs: Vec<(usize, usize, CharProps)> = Vec::new();
    for w in bounds.windows(2) {
        let (x, y) = (w[0], w[1]);
        if x == y || y > ar.text.len() {
            continue;
        }
        let (pw, pa) = (props_at(&work.runs, x), props_at(&ar.runs, x));
        if pw != pa {
            match segs.last_mut() {
                Some(l) if l.1 == x && l.2 == *pa => l.1 = y,
                _ => segs.push((x, y, pa.clone())),
            }
        }
    }
    if !segs.is_empty() {
        out.push(Step::Format { block, segs });
    }
    if b.props != a.props || b.mark != a.mark || b.section != a.section {
        out.push(Step::ParaProps { block, props: a.props.clone(), mark: a.mark.clone(), section: a.section.clone() });
    }
}

fn props_at(runs: &[Run], off: usize) -> &CharProps {
    let mut pos = 0;
    for r in runs {
        if off < pos + r.len {
            return &r.props;
        }
        pos += r.len;
    }
    &runs.last().expect("runs cover a non-empty range").props
}

// ------------------------------------------------------------------------------------------
// Position mapping

/// Map a position (block, byte offset) through step `b`. `assoc` decides which side a position
/// goes to when `b` inserts exactly there (-1: before, +1: after) or deletes around it.
/// `None`: the position's block was replaced or removed.
pub fn map_pos(b: &Step, k: usize, off: usize, assoc: i8) -> Option<(usize, usize)> {
    match b {
        Step::Splice { block, from, to, text, .. } if *block == k => Some((k, map_off(off, *from, *to, text.len(), assoc))),
        Step::Split { block, off: so } if *block == k => Some(match off.cmp(so) {
            std::cmp::Ordering::Less => (k, off),
            std::cmp::Ordering::Greater => (k + 1, off - so),
            std::cmp::Ordering::Equal if assoc < 0 => (k, off),
            std::cmp::Ordering::Equal => (k + 1, 0),
        }),
        Step::Split { block, .. } if *block < k => Some((k + 1, off)),
        Step::Join { block, at } if *block + 1 == k => Some((*block, at + off)),
        Step::Join { block, .. } if *block + 1 < k => Some((k - 1, off)),
        Step::Blocks { .. } => map_block(b, k).map(|k| (k, off)),
        _ => Some((k, off)),
    }
}

/// ProseMirror's rule: a position at the start of a replaced range stays before it, at its end
/// goes after the replacement, inside (or at a pure insertion) follows `assoc`.
fn map_off(pos: usize, from: usize, to: usize, ins: usize, assoc: i8) -> usize {
    if pos < from {
        return pos;
    }
    if pos > to {
        return pos + ins - (to - from);
    }
    let side = if from == to {
        assoc
    } else if pos == from {
        -1
    } else if pos == to {
        1
    } else {
        assoc
    };
    if side < 0 { from } else { from + ins }
}

/// Map a block index through `b` (for steps that address a whole paragraph).
pub fn map_block(b: &Step, k: usize) -> Option<usize> {
    match b {
        Step::Split { block, .. } if *block < k => Some(k + 1),
        Step::Join { block, .. } if *block + 1 == k => None,
        Step::Join { block, .. } if *block + 1 < k => Some(k - 1),
        Step::Blocks { from, to, blocks, .. } => {
            if k < *from {
                Some(k)
            } else if k >= *to {
                Some(k + blocks.len() - (to - from))
            } else {
                None
            }
        }
        _ => Some(k),
    }
}

// ------------------------------------------------------------------------------------------
// Transform

/// Map a byte range of paragraph `k` through `b`. Returns pieces (block, from, to), in an order
/// that can be applied one after the other (later positions first). With `keep_inserts`, a
/// replacement `b` made strictly inside the range is cut out of it (a deletion never swallows
/// text someone typed meanwhile); otherwise the range grows over it.
fn map_range(b: &Step, k: usize, f: usize, t: usize, keep_inserts: bool, a_after: bool) -> Vec<(usize, usize, usize)> {
    match b {
        Step::Split { block, off } if *block == k && f < *off && *off < t => vec![(k + 1, 0, t - off), (k, f, *off)],
        Step::Splice { block, from, to, text, .. } if keep_inserts && *block == k && f < t && f <= *from && *to <= t => {
            let d = text.len() as isize - (*to - *from) as isize;
            let rest = (k, from + text.len(), (t as isize + d) as usize);
            if *from == f && a_after {
                // Same start: the later one's new text goes after the earlier one's.
                vec![(k, f, f), rest]
            } else {
                vec![rest, (k, f, *from)]
            }
        }
        _ => {
            let (a, z) = if f == t { (1, 1) } else { (1, -1) };
            let mut start = map_pos(b, k, f, a);
            // Two replacements starting at the same place: the later one's text goes second.
            if let Step::Splice { block, from, text, .. } = b
                && *block == k
                && *from == f
            {
                start = Some((k, if a_after { from + text.len() } else { *from }));
            }
            match (start, map_pos(b, k, t, z)) {
                (Some((k1, f1)), Some((k2, t2))) if k1 == k2 => vec![(k1, f1, t2.max(f1))],
                (None, _) | (_, None) => vec![],
                _ => unreachable!("a range never crosses blocks except over a split"),
            }
        }
    }
}

/// `a` transformed to apply after `b`, both made on the same document. `a_after`: `a` was ordered
/// after `b` (by the server), so at an exact tie (two insertions at one place) `a` goes second.
pub fn xform(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    let tie: i8 = if a_after { 1 } else { -1 };
    if is_noop(b) {
        return vec![a.clone()];
    }
    match a {
        _ if is_noop(a) => vec![],
        // Last writer wins for formatting and non-body fields: the earlier one gives way.
        Step::Meta(_) if !a_after && matches!(b, Step::Meta(_)) => dropped(a, b, a_after),
        Step::Meta(_) => vec![a.clone()],
        Step::ParaProps { block, .. } if !a_after && matches!(b, Step::ParaProps { block: j, .. } if j == block) => dropped(a, b, a_after),
        Step::Splice { .. } => xform_splice(a, b, a_after),
        Step::Format { .. } => xform_format(a, b, a_after),
        Step::ParaProps { block, props, mark, section } => match b {
            // The paragraph was split meanwhile: both halves get the formatting (as a split after
            // it would have copied it), the section break stays at the end.
            Step::Split { block: j, .. } if j == block => vec![
                Step::ParaProps { block: *block, props: props.clone(), mark: mark.clone(), section: None },
                Step::ParaProps { block: block + 1, props: props.clone(), mark: mark.clone(), section: section.clone() },
            ],
            _ => match map_block(b, *block) {
                Some(k) => vec![Step::ParaProps { block: k, props: props.clone(), mark: mark.clone(), section: section.clone() }],
                None => dropped(a, b, a_after),
            },
        },
        Step::Split { block, off } => match map_pos(b, *block, *off, tie_or_inside(b, *block, *off, tie)) {
            Some((k, o)) => vec![Step::Split { block: k, off: o }],
            None => dropped(a, b, a_after),
        },
        Step::Join { .. } => xform_join(a, b, a_after),
        // A whole-block replace wins over every concurrent edit inside its range, whichever came
        // first (it can't be re-made from a smaller step); between two overlapping block replaces
        // the earlier one wins.
        Step::Blocks { from, to, blocks, .. } if from == to && blocks.is_empty() => vec![],
        Step::Blocks { .. } => xform_blocks(a, b, a_after),
    }
}

fn is_noop(s: &Step) -> bool {
    match s {
        Step::Blocks { from, to, blocks, .. } => from == to && blocks.is_empty(),
        Step::Splice { from, to, text, .. } => from == to && text.is_empty(),
        Step::Format { segs, .. } => segs.iter().all(|(f, t, _)| f >= t),
        _ => false,
    }
}

/// `old` (blocks from index `af`) with `b`, an edit inside them, applied.
fn update_old(old: &[Arc<Block>], af: usize, b: &Step) -> Vec<Arc<Block>> {
    let mut s = b.clone();
    match &mut s {
        Step::Splice { block, .. }
        | Step::Format { block, .. }
        | Step::ParaProps { block, .. }
        | Step::Split { block, .. }
        | Step::Join { block, .. } => *block -= af,
        _ => return old.to_vec(),
    }
    let mut t = Document::new();
    t.body = old.to_vec();
    let _ = apply(&mut t, &s);
    t.body
}

/// For an insertion at `off`: strictly inside a range `b` replaces, go after `b`'s new text
/// (where `b`, transformed over this insertion, would leave it); otherwise the tie rule.
fn tie_or_inside(b: &Step, k: usize, off: usize, tie: i8) -> i8 {
    match b {
        Step::Splice { block, from, to, .. } if *block == k && *from < off && off < *to => 1,
        _ => tie,
    }
}

/// Transform list `a` over list `b` (both made on the same document; `b` was ordered first).
/// Returns (`a` to apply after `b`, `b` to apply after `a`).
pub fn xform_lists(a: Vec<Step>, b: Vec<Step>) -> (Vec<Step>, Vec<Step>) {
    let mut b = b;
    let mut out = Vec::new();
    for x in a {
        let (xs, nb) = xform_one(x, b);
        out.extend(xs);
        b = nb;
    }
    (out, b)
}

fn xform_one(x: Step, b: Vec<Step>) -> (Vec<Step>, Vec<Step>) {
    let mut a = vec![x];
    let mut bout = Vec::new();
    for y in b {
        let (na, ny) = if a.len() == 1 { (xform(&a[0], &y, true), xform(&y, &a[0], false)) } else { xform_lists(a, vec![y]) };
        a = na;
        bout.extend(ny);
    }
    (a, bout)
}

fn xform_splice(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    let tie: i8 = if a_after { 1 } else { -1 };
    let Step::Splice { block, from, to, text, runs, objects } = a else { unreachable!() };
    let same_start = match b {
        Step::Splice { block: j, from: bf, text: bt, .. } if j == block && bf == from => Some(if a_after { bf + bt.len() } else { *bf }),
        _ => None,
    };
    let pieces = if let (true, Some(o)) = (from == to, same_start) {
        vec![(*block, o, o)]
    } else if from == to {
        match map_pos(b, *block, *from, tie_or_inside(b, *block, *from, tie)) {
            Some((k, o)) => vec![(k, o, o)],
            None => vec![],
        }
    } else {
        map_range(b, *block, *from, *to, true, a_after)
    };
    if pieces.is_empty() {
        return dropped(a, b, a_after);
    }
    let n = pieces.len();
    pieces
        .into_iter()
        .enumerate()
        .map(|(i, (k, f, t))| {
            if i + 1 == n {
                Step::Splice { block: k, from: f, to: t, text: text.clone(), runs: runs.clone(), objects: objects.clone() }
            } else {
                Step::Splice { block: k, from: f, to: t, text: String::new(), runs: vec![], objects: vec![] }
            }
        })
        .filter(|s| !matches!(s, Step::Splice { from, to, text, .. } if from == to && text.is_empty()))
        .collect()
}

fn xform_format(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    let Step::Format { block, segs } = a else { unreachable!() };
    let mut by_block: Vec<(usize, Vec<(usize, usize, CharProps)>)> = Vec::new();
    // Text inserted concurrently inside the range keeps its own formatting (as it does
    // when the insertion comes second), and an earlier format gives way where a later
    // one overlaps it.
    let later: &[(usize, usize, CharProps)] = match b {
        Step::Format { block: j, segs } if j == block && !a_after => segs,
        _ => &[],
    };
    for (f, t, p) in segs.iter().filter(|(f, t, _)| f < t) {
        let mut pieces = map_range(b, *block, *f, *t, true, a_after);
        for (lf, lt, _) in later {
            pieces = pieces.into_iter().flat_map(|(k, f, t)| [(k, f, t.min(*lf)), (k, f.max(*lt), t)]).filter(|(_, f, t)| f < t).collect();
        }
        for (k, f2, t2) in pieces {
            if f2 < t2 {
                match by_block.iter_mut().find(|(kk, _)| *kk == k) {
                    Some((_, v)) => v.push((f2, t2, p.clone())),
                    None => by_block.push((k, vec![(f2, t2, p.clone())])),
                }
            }
        }
    }
    if by_block.is_empty() {
        return dropped(a, b, a_after);
    }
    by_block.into_iter().map(|(block, segs)| Step::Format { block, segs }).collect()
}

fn xform_join(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    let Step::Join { block, at } = a else { unreachable!() };
    let k = *block;
    match b {
        Step::Join { block: j, .. } if *j == k => dropped(a, b, a_after), // the same join
        Step::Join { block: j, at: bat } if *j + 1 == k => vec![Step::Join { block: k - 1, at: bat + at }],
        Step::Splice { block: j, from, to, text, .. } if *j == k => {
            vec![Step::Join { block: k, at: at + text.len() - (to - from) }]
        }
        Step::Split { block: j, off } if *j == k => vec![Step::Join { block: k + 1, at: at - off }],
        _ => match (map_block(b, k), map_block(b, k + 1)) {
            (Some(k1), Some(k2)) if k2 == k1 + 1 => vec![Step::Join { block: k1, at: *at }],
            _ => dropped(a, b, a_after),
        },
    }
}

fn xform_blocks(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    let Step::Blocks { from, to, blocks, old } = a else { unreachable!() };
    let (af, at) = (*from, *to);
    let re = |f: usize, t: usize, old: Vec<Arc<Block>>| vec![Step::Blocks { from: f, to: t, blocks: blocks.clone(), old }];
    let inside = |x: usize| af <= x && x < at;
    match b {
        Step::Meta(_) => re(af, at, old.clone()),
        Step::Splice { block: x, .. } | Step::Format { block: x, .. } | Step::ParaProps { block: x, .. } => {
            re(af, at, if inside(*x) { update_old(old, af, b) } else { old.clone() })
        }
        Step::Split { block: x, .. } => {
            if *x < af {
                re(af + 1, at + 1, old.clone())
            } else if *x < at {
                re(af, at + 1, update_old(old, af, b))
            } else {
                re(af, at, old.clone())
            }
        }
        Step::Join { block: j, at: jat } => {
            let (j, j1) = (*j, j + 1);
            if j1 < af {
                re(af - 1, at - 1, old.clone())
            } else if j >= at {
                re(af, at, old.clone())
            } else if j >= af && j1 < at {
                re(af, at - 1, update_old(old, af, b))
            } else {
                // The join reached across the edge of the range: undo it, then replace.
                let mut v = vec![Step::Split { block: j, off: *jat }];
                v.extend(re(af, at, old.clone()));
                v
            }
        }
        Step::Blocks { from: bf, to: bt, blocks: bl, old: bold } => {
            let (bf, bt) = (*bf, *bt);
            let d = bl.len() as isize - (bt - bf) as isize;
            let shift = |x: usize| (x as isize + d) as usize;
            if af == at && bf == bt && af == bf {
                if a_after { re(shift(af), shift(at), old.clone()) } else { re(af, at, old.clone()) }
            } else if at <= bf {
                re(af, at, old.clone())
            } else if af >= bt {
                re(shift(af), shift(at), old.clone())
            } else if a_after {
                dropped(a, b, a_after)
            } else {
                // Undo `b` over the union of the two ranges and make `a` there instead.
                let (u0, u1) = (af.min(bf), at.max(bt));
                let orig = |i: usize| if af <= i && i < at { old[i - af].clone() } else { bold[i - bf].clone() };
                let mut new: Vec<Arc<Block>> = (u0..af).map(orig).collect();
                new.extend(blocks.iter().cloned());
                new.extend((at..u1).map(orig));
                let mut cur: Vec<Arc<Block>> = (u0..bf).map(orig).collect();
                cur.extend(bl.iter().cloned());
                cur.extend((bt..u1).map(orig));
                vec![Step::Blocks { from: u0, to: shift(u1), blocks: new, old: cur }]
            }
        }
    }
}
