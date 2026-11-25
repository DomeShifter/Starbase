//! Position mapping and transforming a step over a concurrent one.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;

use wordcraft_doc::{Block, CharProps, Document, StoryRef};

use crate::step::{Cont, Step, apply, blank_cell, is_noop, shift_revs_step};

thread_local! {
    /// While set (the server mapping a late change), steps of that change dropped by [`xform`]
    /// because what they edited was replaced or removed concurrently are counted in [`DROPPED`]
    /// by (dropped step, concurrent step) kind (statistics for the spike's report).
    pub static RECORD_DROPS: Cell<bool> = const { Cell::new(false) };
    pub static DROPPED: RefCell<BTreeMap<(&'static str, &'static str), usize>> = const { RefCell::new(BTreeMap::new()) };
}

fn dropped(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    if a_after && RECORD_DROPS.with(Cell::get) {
        DROPPED.with(|d| *d.borrow_mut().entry((a.kind(), b.kind())).or_default() += 1);
    }
    Vec::new()
}

// ------------------------------------------------------------------------------------------
// Position mapping

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

/// A block index in `b`'s own container, through `b`.
fn map_block_same(b: &Step, k: usize) -> Option<usize> {
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

/// A position (block, offset) in `b`'s own container, through `b`.
fn map_pos_same(b: &Step, k: usize, off: usize, assoc: i8) -> Option<(usize, usize)> {
    match b {
        Step::Splice { block, from, to, text, .. } if *block == k => Some((k, map_off(off, *from, *to, text.len(), assoc))),
        Step::Split { block, off: so, .. } if *block == k => Some(match off.cmp(so) {
            Ordering::Less => (k, off),
            Ordering::Greater => (k + 1, off - so),
            Ordering::Equal if assoc < 0 => (k, off),
            Ordering::Equal => (k + 1, 0),
        }),
        Step::Join { block, at, .. } if *block + 1 == k => Some((*block, at + off)),
        _ => map_block_same(b, k).map(|k| (k, off)),
    }
}

/// A container strictly inside `b`'s container (in a cell of a table there), through `b`:
/// the table moves with block changes, rows and columns with table steps.
fn nest_map(b: &Step, c: &Cont) -> Option<Cont> {
    let cb = b.cont()?;
    let i = cb.path.len();
    let (y, r, k) = (c.path[i] as usize, c.path[i + 1] as usize, c.path[i + 2] as usize);
    let (y2, r2, k2) = match b {
        Step::Split { .. } | Step::Join { .. } | Step::Blocks { .. } => (map_block_same(b, y)?, r, k),
        Step::InsertRow { block, at, .. } if *block == y => (y, if r >= *at { r + 1 } else { r }, k),
        Step::DeleteRow { block, at, .. } if *block == y => match r.cmp(at) {
            Ordering::Equal => return None,
            Ordering::Greater => (y, r - 1, k),
            Ordering::Less => (y, r, k),
        },
        Step::InsertCol { block, at, .. } if *block == y => (y, r, if k >= *at { k + 1 } else { k }),
        Step::DeleteCol { block, at, .. } if *block == y => match k.cmp(at) {
            Ordering::Equal => return None,
            Ordering::Greater => (y, r, k - 1),
            Ordering::Less => (y, r, k),
        },
        _ => (y, r, k),
    };
    let mut c2 = c.clone();
    c2.path[i] = y2 as u32;
    c2.path[i + 1] = r2 as u32;
    c2.path[i + 2] = k2 as u32;
    Some(c2)
}

/// Map a position (container, block, byte offset) through step `b`. `assoc` decides which side a
/// position goes to when `b` inserts exactly there (-1: before, +1: after) or deletes around it.
/// `None`: the position's paragraph, table row/column or story was replaced or removed.
pub fn map_pos(b: &Step, c: &Cont, k: usize, off: usize, assoc: i8) -> Option<(Cont, usize, usize)> {
    if let Step::PartRemove { id } = b
        && c.story == StoryRef::Part(*id)
    {
        return None;
    }
    let Some(cb) = b.cont() else { return Some((c.clone(), k, off)) };
    if c.inside(cb) {
        return nest_map(b, c).map(|c| (c, k, off));
    }
    if cb != c {
        return Some((c.clone(), k, off));
    }
    map_pos_same(b, k, off, assoc).map(|(k, o)| (c.clone(), k, o))
}

// ------------------------------------------------------------------------------------------
// Transform

/// Map a byte range of paragraph `k` through `b` (same container). Returns pieces (block, from,
/// to), in an order that can be applied one after the other (later positions first). A
/// replacement `b` made inside the range is cut out of it (a deletion never swallows text
/// someone typed meanwhile).
fn map_range(b: &Step, k: usize, f: usize, t: usize, a_after: bool) -> Vec<(usize, usize, usize)> {
    match b {
        Step::Split { block, off, .. } if *block == k && f < *off && *off < t => vec![(k + 1, 0, t - off), (k, f, *off)],
        Step::Splice { block, from, to, text, .. } if *block == k && f < t && f <= *from && *to <= t => {
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
            let mut start = map_pos_same(b, k, f, a);
            // Two replacements starting at the same place: the later one's text goes second.
            if let Step::Splice { block, from, text, .. } = b
                && *block == k
                && *from == f
            {
                start = Some((k, if a_after { from + text.len() } else { *from }));
            }
            match (start, map_pos_same(b, k, t, z)) {
                (Some((k1, f1)), Some((k2, t2))) if k1 == k2 => vec![(k1, f1, t2.max(f1))],
                _ => vec![],
            }
        }
    }
}

/// For an insertion at `off`: strictly inside a range `b` replaces, go after `b`'s new text
/// (where `b`, transformed over this insertion, would leave it); otherwise the tie rule.
fn tie_or_inside(b: &Step, k: usize, off: usize, tie: i8) -> i8 {
    match b {
        Step::Splice { block, from, to, .. } if *block == k && *from < off && off < *to => 1,
        _ => tie,
    }
}

/// `a` transformed to apply after `b`, both made on the same document. `a_after`: `a` was ordered
/// after `b` (by the server), so at an exact tie (two insertions at one place) `a` goes second;
/// for last-writer-wins settings the later one wins.
pub fn xform(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    if is_noop(b) {
        return vec![a.clone()];
    }
    if is_noop(a) {
        return vec![];
    }
    let mut out = xform_core(a, b, a_after);
    // Revision references of the other side's new records move up past this one.
    if let Step::RevInsert { at, .. } = b
        && !matches!(a, Step::RevInsert { .. })
    {
        out.iter_mut().for_each(|s| shift_revs_step(s, *at));
    }
    out
}

fn xform_core(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    let same_lww = |x: bool| if x && !a_after { dropped(a, b, a_after) } else { vec![a.clone()] };
    match a {
        Step::PartAdd { .. } => vec![a.clone()],
        Step::PartRemove { id } => match b {
            Step::PartRemove { id: j } if j == id => dropped(a, b, a_after),
            _ => vec![a.clone()],
        },
        Step::PartAttrs { id, .. } => match b {
            Step::PartRemove { id: j } if j == id => dropped(a, b, a_after),
            Step::PartAttrs { id: j, .. } => same_lww(j == id),
            _ => vec![a.clone()],
        },
        Step::Comment { id, .. } => same_lww(matches!(b, Step::Comment { id: j, .. } if j == id)),
        Step::RevInsert { at, rev } => match b {
            Step::RevInsert { at: bat, .. } if *at > *bat || (at == bat && a_after) => vec![Step::RevInsert { at: at + 1, rev: rev.clone() }],
            Step::Revisions { all } => vec![Step::RevInsert { at: (*at).min(all.len()), rev: rev.clone() }],
            _ => vec![a.clone()],
        },
        Step::Revisions { .. } => same_lww(matches!(b, Step::Revisions { .. })),
        Step::Media { key, .. } => same_lww(matches!(b, Step::Media { key: j, .. } if j == key)),
        Step::Field { name, .. } => same_lww(matches!(b, Step::Field { name: j, .. } if j == name)),
        _ => xform_contained(a, b, a_after),
    }
}

/// `a` acts in a container.
fn xform_contained(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    let Some(ca) = a.cont() else { return vec![a.clone()] };
    if let Step::PartRemove { id } = b
        && ca.story == StoryRef::Part(*id)
    {
        return dropped(a, b, a_after);
    }
    let Some(cb) = b.cont() else { return vec![a.clone()] };
    if ca.inside(cb) {
        // `a` is inside a table cell in `b`'s container.
        return match nest_map(b, ca) {
            Some(c2) => {
                let mut a2 = a.clone();
                *a2.cont_mut().expect("contained") = c2;
                vec![a2]
            }
            None => dropped(a, b, a_after),
        };
    }
    if cb.inside(ca) {
        // `b` is inside a table cell in `a`'s container: only a block replace over that table
        // notices (it keeps its record of the blocks it replaces up to date).
        if let Step::Blocks { c, from, to, blocks, old } = a {
            let y = cb.path[ca.path.len()] as usize;
            if *from <= y && y < *to {
                return vec![Step::Blocks { c: c.clone(), from: *from, to: *to, blocks: blocks.clone(), old: update_old(old, ca, *from, b) }];
            }
        }
        return vec![a.clone()];
    }
    if ca != cb {
        return vec![a.clone()];
    }
    xform_same(a, b, a_after)
}

/// Both in the same container.
fn xform_same(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    let tie: i8 = if a_after { 1 } else { -1 };
    match a {
        Step::Splice { .. } => xform_splice(a, b, a_after),
        Step::Format { .. } => xform_format(a, b, a_after),
        Step::ParaProps { c, block, attrs } => match b {
            // Last writer wins.
            Step::ParaProps { block: j, .. } if j == block && !a_after => dropped(a, b, a_after),
            // The paragraph was split meanwhile: both halves get the formatting (as a split
            // after it would have given the tail).
            Step::Split { block: j, .. } if j == block => vec![
                Step::ParaProps { c: c.clone(), block: *block, attrs: attrs.clone() },
                Step::ParaProps { c: c.clone(), block: block + 1, attrs: attrs.clone() },
            ],
            _ => match map_block_same(b, *block) {
                Some(k) => vec![Step::ParaProps { c: c.clone(), block: k, attrs: attrs.clone() }],
                None => dropped(a, b, a_after),
            },
        },
        Step::Split { c, block, off, tail } => {
            // The paragraph's formatting changed meanwhile: the tail gets the new one.
            let tail = match b {
                Step::ParaProps { block: j, attrs, .. } if j == block => attrs.clone(),
                _ => tail.clone(),
            };
            match map_pos_same(b, *block, *off, tie_or_inside(b, *block, *off, tie)) {
                Some((k, o)) => vec![Step::Split { c: c.clone(), block: k, off: o, tail }],
                None => dropped(a, b, a_after),
            }
        }
        Step::Join { .. } => xform_join(a, b, a_after),
        Step::Blocks { .. } => xform_blocks(a, b, a_after),
        _ => xform_table(a, b, a_after),
    }
}

fn xform_splice(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    let tie: i8 = if a_after { 1 } else { -1 };
    let Step::Splice { c, block, from, to, text, runs, objects } = a else { return vec![a.clone()] };
    let same_start = match b {
        Step::Splice { block: j, from: bf, text: bt, .. } if j == block && bf == from => Some(if a_after { bf + bt.len() } else { *bf }),
        _ => None,
    };
    let pieces = if let (true, Some(o)) = (from == to, same_start) {
        vec![(*block, o, o)]
    } else if from == to {
        match map_pos_same(b, *block, *from, tie_or_inside(b, *block, *from, tie)) {
            Some((k, o)) => vec![(k, o, o)],
            None => vec![],
        }
    } else {
        map_range(b, *block, *from, *to, a_after)
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
                Step::Splice { c: c.clone(), block: k, from: f, to: t, text: text.clone(), runs: runs.clone(), objects: objects.clone() }
            } else {
                Step::Splice { c: c.clone(), block: k, from: f, to: t, text: String::new(), runs: vec![], objects: vec![] }
            }
        })
        .filter(|s| !is_noop(s))
        .collect()
}

fn xform_format(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    let Step::Format { c, block, segs } = a else { return vec![a.clone()] };
    let mut by_block: Vec<(usize, Vec<(usize, usize, CharProps)>)> = Vec::new();
    // Text inserted concurrently inside the range keeps its own formatting (as it does when the
    // insertion comes second), and an earlier format gives way where a later one overlaps it.
    let later: &[(usize, usize, CharProps)] = match b {
        Step::Format { block: j, segs, .. } if j == block && !a_after => segs,
        _ => &[],
    };
    for (f, t, p) in segs.iter().filter(|(f, t, _)| f < t) {
        let mut pieces = map_range(b, *block, *f, *t, a_after);
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
    by_block.into_iter().map(|(block, segs)| Step::Format { c: c.clone(), block, segs }).collect()
}

fn xform_join(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    let Step::Join { c, block, at, gone } = a else { return vec![a.clone()] };
    let k = *block;
    let join = |block: usize, at: usize, gone| vec![Step::Join { c: c.clone(), block, at, gone }];
    match b {
        Step::Join { block: j, .. } if *j == k => dropped(a, b, a_after), // the same join
        Step::Join { block: j, at: bat, .. } if *j + 1 == k => join(k - 1, bat + at, gone.clone()),
        Step::Splice { block: j, from, to, text, .. } if *j == k => join(k, at + text.len() - (to - from), gone.clone()),
        Step::Split { block: j, off, .. } if *j == k => join(k + 1, at - off, gone.clone()),
        // The joined-away paragraph's formatting changed: keep the record of it current.
        Step::ParaProps { block: j, attrs, .. } if *j == k + 1 => join(k, *at, attrs.clone()),
        _ => match (map_block_same(b, k), map_block_same(b, k + 1)) {
            (Some(k1), Some(k2)) if k2 == k1 + 1 => join(k1, *at, gone.clone()),
            _ => dropped(a, b, a_after),
        },
    }
}

/// `old` (the blocks from index `af` of container `ca`) with `b`, an edit inside them, applied.
fn update_old(old: &[Arc<Block>], ca: &Cont, af: usize, b: &Step) -> Vec<Arc<Block>> {
    let mut s = b.clone();
    let Some(cb) = s.cont_mut() else { return old.to_vec() };
    let n = ca.path.len();
    if cb.path.len() > n {
        // Inside a table among the old blocks: re-root the path there.
        let mut path = cb.path[n..].to_vec();
        path[0] -= af as u32;
        *cb = Cont { story: StoryRef::Body, path };
    } else {
        *cb = Cont::body();
        match s.block_mut() {
            Some(k) if *k >= af => *k -= af,
            _ => return old.to_vec(),
        }
    }
    let mut t = Document::new();
    t.body = old.to_vec();
    let _ = apply(&mut t, &s);
    t.body
}

// A whole-block replace wins over every concurrent edit inside its range, whichever came first
// (it can't be re-made from a smaller step); between two overlapping block replaces the earlier
// one wins.
fn xform_blocks(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    let Step::Blocks { c, from, to, blocks, old } = a else { return vec![a.clone()] };
    let (af, at) = (*from, *to);
    let re = |f: usize, t: usize, old: Vec<Arc<Block>>| vec![Step::Blocks { c: c.clone(), from: f, to: t, blocks: blocks.clone(), old }];
    let inside = |x: usize| af <= x && x < at;
    match b {
        Step::Split { block: x, .. } => {
            if *x < af {
                re(af + 1, at + 1, old.clone())
            } else if *x < at {
                re(af, at + 1, update_old(old, c, af, b))
            } else {
                re(af, at, old.clone())
            }
        }
        Step::Join { block: j, at: jat, gone, .. } => {
            let (j, j1) = (*j, j + 1);
            if j1 < af {
                re(af - 1, at - 1, old.clone())
            } else if j >= at {
                re(af, at, old.clone())
            } else if j >= af && j1 < at {
                re(af, at - 1, update_old(old, c, af, b))
            } else {
                // The join reached across the edge of the range: undo it, then replace.
                let mut v = vec![Step::Split { c: c.clone(), block: j, off: *jat, tail: gone.clone() }];
                v.extend(re(af, at, old.clone()));
                v
            }
        }
        Step::Blocks { from: bf, to: bt, blocks: bl, old: bold, .. } => {
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
                vec![Step::Blocks { c: c.clone(), from: u0, to: shift(u1), blocks: new, old: cur }]
            }
        }
        _ => {
            let mut b2 = b.clone();
            match b2.block_mut() {
                Some(x) if inside(*x) => re(af, at, update_old(old, c, af, b)),
                _ => re(af, at, old.clone()),
            }
        }
    }
}

/// `a` is a table step (on the table at its block).
fn xform_table(a: &Step, b: &Step, a_after: bool) -> Vec<Step> {
    let mut a2 = a.clone();
    let y = *a2.block_mut().expect("a table step has a block");
    if !b.is_table_step() {
        // Paragraph steps leave the table alone; block changes move or remove it.
        return match b {
            Step::Split { .. } | Step::Join { .. } | Step::Blocks { .. } => match map_block_same(b, y) {
                Some(y2) => {
                    *a2.block_mut().expect("table step") = y2;
                    vec![a2]
                }
                None => dropped(a, b, a_after),
            },
            _ => vec![a2],
        };
    }
    let mut b2 = b.clone();
    if *b2.block_mut().expect("a table step has a block") != y {
        return vec![a2];
    }
    let row_shift = |r: usize, at: usize, insert: bool| -> Option<usize> {
        match (insert, r.cmp(&at)) {
            (true, Ordering::Less) => Some(r),
            (true, _) => Some(r + 1),
            (false, Ordering::Equal) => None,
            (false, Ordering::Greater) => Some(r - 1),
            (false, Ordering::Less) => Some(r),
        }
    };
    match (&mut a2, b) {
        (Step::InsertRow { at, .. }, Step::InsertRow { at: bt, .. }) if *at > *bt || (at == bt && a_after) => *at += 1,
        (Step::InsertRow { at, .. }, Step::DeleteRow { at: bt, .. }) if *at > *bt => *at -= 1,
        (Step::InsertRow { row, .. }, Step::InsertCol { at: k, .. }) => {
            let k = (*k).min(row.cells.len());
            row.cells.insert(k, blank_cell());
        }
        (Step::InsertRow { row, .. }, Step::DeleteCol { at: k, .. }) if *k < row.cells.len() => {
            row.cells.remove(*k);
        }
        (Step::DeleteRow { at, .. }, Step::InsertRow { at: bt, .. }) if *at >= *bt => *at += 1,
        (Step::DeleteRow { at, .. }, Step::DeleteRow { at: bt, .. }) => match row_shift(*at, *bt, false) {
            Some(r) => *at = r,
            None => return dropped(a, b, a_after),
        },
        (Step::InsertCol { cells, .. }, Step::InsertRow { at: r, .. }) => {
            let r = (*r).min(cells.len());
            cells.insert(r, blank_cell());
        }
        (Step::InsertCol { cells, .. }, Step::DeleteRow { at: r, .. }) if *r < cells.len() => {
            cells.remove(*r);
        }
        (Step::InsertCol { at, .. }, Step::InsertCol { at: k, .. }) if *at > *k || (at == k && a_after) => *at += 1,
        (Step::InsertCol { at, .. }, Step::DeleteCol { at: k, .. }) if *at > *k => *at -= 1,
        (Step::DeleteCol { at, .. }, Step::InsertCol { at: k, .. }) if *at >= *k => *at += 1,
        (Step::DeleteCol { at, .. }, Step::DeleteCol { at: k, .. }) => match row_shift(*at, *k, false) {
            Some(x) => *at = x,
            None => return dropped(a, b, a_after),
        },
        (Step::TableAttrs { .. }, Step::TableAttrs { .. }) if !a_after => return dropped(a, b, a_after),
        (Step::TableAttrs { grid, .. }, Step::InsertCol { at, width, .. }) => {
            let g = (*at).min(grid.len());
            grid.insert(g, *width);
        }
        (Step::TableAttrs { grid, .. }, Step::DeleteCol { at, .. }) if *at < grid.len() => {
            grid.remove(*at);
        }
        (Step::RowAttrs { row, .. } | Step::CellAttrs { row, .. }, Step::InsertRow { at, .. }) => *row = row_shift(*row, *at, true).unwrap_or(*row),
        (Step::RowAttrs { row, .. } | Step::CellAttrs { row, .. }, Step::DeleteRow { at, .. }) => match row_shift(*row, *at, false) {
            Some(r) => *row = r,
            None => return dropped(a, b, a_after),
        },
        (Step::RowAttrs { row, .. }, Step::RowAttrs { row: br, .. }) if row == br && !a_after => return dropped(a, b, a_after),
        (Step::CellAttrs { cell, .. }, Step::InsertCol { at, .. }) => *cell = row_shift(*cell, *at, true).unwrap_or(*cell),
        (Step::CellAttrs { cell, .. }, Step::DeleteCol { at, .. }) => match row_shift(*cell, *at, false) {
            Some(k) => *cell = k,
            None => return dropped(a, b, a_after),
        },
        (Step::CellAttrs { row, cell, .. }, Step::CellAttrs { row: br, cell: bk, .. }) if row == br && cell == bk && !a_after => {
            return dropped(a, b, a_after);
        }
        _ => {}
    }
    vec![a2]
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
