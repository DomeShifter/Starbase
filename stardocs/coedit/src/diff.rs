//! Steps from comparing a document before and after a command.

use std::collections::BTreeSet;
use std::sync::Arc;

use wordcraft_doc::{Block, CharProps, Document, Paragraph, StoryRef, Table};

use crate::step::{Attrs, Cont, FIELDS, OBJ, Step, apply, field_eq, fields_of, has_merges, join_para, objs_in, runs_slice, splice_para, split_para};

fn same(a: &Arc<Block>, b: &Arc<Block>) -> bool {
    Arc::ptr_eq(a, b) || **a == **b
}

/// The steps that turn `before` into `after` (up to run normalisation), in every story.
pub fn diff(before: &Document, after: &Document) -> Vec<Step> {
    let mut out = Vec::new();
    // New stories, comments, revisions and media first: later steps may refer to them.
    for (id, p) in &after.parts {
        if !before.parts.contains_key(id) {
            out.push(Step::PartAdd { id: *id, part: Box::new(p.clone()) });
        }
    }
    let ids: BTreeSet<u32> = before.comments.keys().chain(after.comments.keys()).copied().collect();
    for id in ids {
        let (b, a) = (before.comments.get(&id), after.comments.get(&id));
        if b != a {
            out.push(Step::Comment { id, comment: a.map(|c| Box::new(c.clone())) });
        }
    }
    let (br, ar) = (&before.revisions, &after.revisions);
    if br != ar {
        if ar.len() > br.len() && ar[..br.len()] == br[..] {
            for (i, r) in ar.iter().enumerate().skip(br.len()) {
                out.push(Step::RevInsert { at: i, rev: Box::new(r.clone()) });
            }
        } else {
            out.push(Step::Revisions { all: ar.clone() });
        }
    }
    if before.media != after.media {
        let keys: BTreeSet<&String> = before.media.keys().chain(after.media.keys()).collect();
        for k in keys {
            let (b, a) = (before.media.get(k), after.media.get(k));
            if b != a {
                out.push(Step::Media { key: k.clone(), bytes: a.cloned() });
            }
        }
    }
    diff_blocks(&Cont::body(), &before.body, &after.body, &mut out);
    for (id, a) in &after.parts {
        if let Some(b) = before.parts.get(id) {
            if b.blocks != a.blocks {
                diff_blocks(&Cont::story(StoryRef::Part(*id)), &b.blocks, &a.blocks, &mut out);
            }
            if b.kind != a.kind || b.body != a.body {
                let mut p = a.clone();
                p.blocks = Vec::new();
                out.push(Step::PartAttrs { id: *id, part: Box::new(p) });
            }
        }
    }
    for id in before.parts.keys() {
        if !after.parts.contains_key(id) {
            out.push(Step::PartRemove { id: *id });
        }
    }
    let changed: Vec<&'static str> = FIELDS.iter().copied().filter(|n| !field_eq(before, after, n)).collect();
    if !changed.is_empty() {
        let from = Box::new(fields_of(after));
        for name in changed {
            out.push(Step::Field { name, from: from.clone() });
        }
    }
    out
}

/// Steps for one list of blocks (a story's top level or a table cell).
pub fn diff_blocks(c: &Cont, bb: &[Arc<Block>], ab: &[Arc<Block>], out: &mut Vec<Step>) {
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
    if m == 0 && n == 0 {
        return;
    }
    fn paras(blocks: &[Arc<Block>]) -> Option<Vec<&Paragraph>> {
        blocks.iter().map(|b| b.as_para()).collect()
    }
    if m == n {
        for j in 0..m {
            match (&*bb[i + j], &*ab[i + j]) {
                (Block::Para(b), Block::Para(a)) => para_diff(c, i + j, b, a, out),
                (Block::Table(b), Block::Table(a)) => table_diff(c, i + j, b, a, out),
                _ => {
                    out.push(Step::Blocks { c: c.clone(), from: i + j, to: i + j + 1, blocks: vec![ab[i + j].clone()], old: vec![bb[i + j].clone()] })
                }
            }
        }
        return;
    }
    let has_table = |v: &[Arc<Block>]| v.iter().any(|b| b.as_table().is_some());
    if has_table(&bb[i..i + m]) || has_table(&ab[i..i + n]) {
        // Tables removed and/or inserted among paragraphs (a selection deleted across a table,
        // a table inserted mid-paragraph): take the old tables out, fix the paragraphs up with
        // Split / Join, then put the new tables in, so typing in those paragraphs meanwhile
        // isn't lost.
        let mut cur: Vec<Arc<Block>> = bb[i..i + m].to_vec();
        for j in (0..m).rev() {
            if cur[j].as_table().is_some() {
                out.push(Step::Blocks { c: c.clone(), from: i + j, to: i + j + 1, blocks: vec![], old: vec![cur.remove(j)] });
            }
        }
        let new_paras: Vec<Arc<Block>> = ab[i..i + n].iter().filter(|b| b.as_para().is_some()).cloned().collect();
        if cur.is_empty() || new_paras.is_empty() {
            out.push(Step::Blocks { c: c.clone(), from: i, to: i + cur.len(), blocks: new_paras, old: cur });
        } else {
            let before: Vec<Arc<Block>> = bb[..i].iter().chain(&cur).chain(&bb[bn - s..]).cloned().collect();
            let after: Vec<Arc<Block>> = ab[..i].iter().chain(&new_paras).chain(&ab[an - s..]).cloned().collect();
            diff_blocks(c, &before, &after, out);
        }
        for (j, b) in ab[i..i + n].iter().enumerate() {
            if b.as_table().is_some() {
                out.push(Step::Blocks { c: c.clone(), from: i + j, to: i + j, blocks: vec![b.clone()], old: vec![] });
            }
        }
        return;
    }
    match (paras(&bb[i..i + m]), paras(&ab[i..i + n])) {
        (Some(bp), Some(ap)) if m > 0 && n > 0 => {
            // Join the changed paragraphs into one, split it where the new ones start, then
            // fix each one up: Enter and Backspace/Delete at a paragraph edge come out as one
            // Split / Join and nothing else.
            let mut work = bp[0].clone();
            for q in &bp[1..] {
                out.push(Step::Join { c: c.clone(), block: i, at: work.text.len(), gone: Box::new(Attrs::of(q)) });
                join_para(&mut work, q);
            }
            for (k, target) in ap.iter().enumerate().take(n - 1) {
                let mut off = work.text.bytes().zip(target.text.bytes()).take_while(|(x, y)| x == y).count();
                if k == n - 2 {
                    // The last split: where the last paragraph's text starts at the end, if
                    // later (AutoCorrect may have changed the head, e.g. a capital after Enter).
                    let tail = &ap[n - 1].text;
                    let common = work.text.bytes().rev().zip(tail.bytes().rev()).take_while(|(x, y)| x == y).count();
                    if common == tail.len() {
                        off = off.max(work.text.len() - common);
                    }
                }
                while !work.text.is_char_boundary(off) {
                    off -= 1;
                }
                let tail = Attrs::of(&work);
                let rest = match split_para(&mut work, off, &tail) {
                    Ok(r) => r,
                    Err(_) => {
                        return out.push(Step::Blocks { c: c.clone(), from: i, to: i + m, blocks: ab[i..i + n].to_vec(), old: bb[i..i + m].to_vec() });
                    }
                };
                out.push(Step::Split { c: c.clone(), block: i + k, off, tail: Box::new(tail) });
                para_diff(c, i + k, &work, target, out);
                work = rest;
            }
            para_diff(c, i + n - 1, &work, ap[n - 1], out);
        }
        _ => out.push(Step::Blocks { c: c.clone(), from: i, to: i + m, blocks: ab[i..i + n].to_vec(), old: bb[i..i + m].to_vec() }),
    }
}

/// Byte offsets of the U+FFFC characters in `s`.
fn obj_offsets(s: &str) -> Vec<usize> {
    s.match_indices(OBJ).map(|(i, _)| i).collect()
}

fn para_diff(c: &Cont, block: usize, b: &Paragraph, a: &Paragraph, out: &mut Vec<Step>) {
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
        let (ob, oa) = (bo[bo.len() - 1 - k], ao.len().checked_sub(1 + k));
        if ob < bt.len() - s {
            break;
        }
        let kb = b.objects.len().checked_sub(1 + k);
        if oa.is_none() || kb.and_then(|kb| b.objects.get(kb)) != oa.and_then(|ka| a.objects.get(ka)) {
            s = bt.len() - ob - OBJ.len_utf8();
            break;
        }
    }
    let (from, to, ato) = (p, bt.len() - s, at.len() - s);
    let mut work = b.clone();
    let mut ar = a.clone();
    ar.normalize();
    if from < to || from < ato {
        let text = a.text[from..ato].to_string();
        let k0 = objs_in(&a.text[..from]);
        let objects = a.objects.get(k0..k0 + objs_in(&text)).map(<[_]>::to_vec).unwrap_or_default();
        let runs = runs_slice(&ar.runs, from, ato);
        if splice_para(&mut work, from, to, &text, &runs, &objects).is_err() {
            // Not expressible (inconsistent objects): replace the paragraph.
            return out.push(Step::Blocks {
                c: c.clone(),
                from: block,
                to: block + 1,
                blocks: vec![Arc::new(Block::Para(a.clone()))],
                old: vec![Arc::new(Block::Para(b.clone()))],
            });
        }
        out.push(Step::Splice { c: c.clone(), block, from, to, text, runs, objects });
    }
    // Formatting differences left over (same text now).
    work.normalize();
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
            let pa = pa.cloned().unwrap_or_default();
            match segs.last_mut() {
                Some(l) if l.1 == x && l.2 == pa => l.1 = y,
                _ => segs.push((x, y, pa)),
            }
        }
    }
    if !segs.is_empty() {
        out.push(Step::Format { c: c.clone(), block, segs });
    }
    if b.props != a.props || b.mark != a.mark || b.section != a.section {
        out.push(Step::ParaProps { c: c.clone(), block, attrs: Box::new(Attrs::of(a)) });
    }
}

fn props_at(runs: &[wordcraft_doc::Run], off: usize) -> Option<&CharProps> {
    let mut pos = 0;
    for r in runs {
        if off < pos + r.len {
            return Some(&r.props);
        }
        pos += r.len;
    }
    runs.last().map(|r| &r.props)
}

fn replace_table(c: &Cont, y: usize, b: &Table, a: &Table, out: &mut Vec<Step>) {
    out.push(Step::Blocks {
        c: c.clone(),
        from: y,
        to: y + 1,
        blocks: vec![Arc::new(Block::Table(a.clone()))],
        old: vec![Arc::new(Block::Table(b.clone()))],
    });
}

/// A table edited in place: rows or columns added or removed, formatting, and the cells'
/// content (recursively). Anything else (merged cells, several structural changes at once)
/// replaces the table.
fn table_diff(c: &Cont, y: usize, b: &Table, a: &Table, out: &mut Vec<Step>) {
    if b == a {
        return;
    }
    let plain = |t: &Table| {
        !has_merges(t) && t.controls.is_empty() && t.rows.iter().all(|r| r.controls.is_empty() && r.cells.iter().all(|c| c.controls.is_empty()))
    };
    if !plain(b) || !plain(a) {
        return replace_table(c, y, b, a, out);
    }
    // Rows and cells are matched by content; formatting differences (widths change when a
    // column is added) come out as attribute steps afterwards.
    let same_cells = |x: &[wordcraft_doc::Cell], z: &[wordcraft_doc::Cell]| x.len() == z.len() && x.iter().zip(z).all(|(p, q)| p.blocks == q.blocks);
    let same_row = |x: &wordcraft_doc::Row, z: &wordcraft_doc::Row| same_cells(&x.cells, &z.cells);
    let mut steps = Vec::new();
    let mut cur = b.clone();
    let (bn, an) = (b.rows.len(), a.rows.len());
    if bn != an {
        let mut i = 0;
        while i < bn.min(an) && same_row(&b.rows[i], &a.rows[i]) {
            i += 1;
        }
        let mut s = 0;
        while s < bn.min(an) - i && same_row(&b.rows[bn - 1 - s], &a.rows[an - 1 - s]) {
            s += 1;
        }
        let (m, n) = (bn - i - s, an - i - s);
        if m == 0 {
            for j in 0..n {
                steps.push(Step::InsertRow { c: c.clone(), block: y, at: i + j, row: Box::new(a.rows[i + j].clone()) });
            }
        } else if n == 0 {
            for _ in 0..m {
                steps.push(Step::DeleteRow { c: c.clone(), block: y, at: i });
            }
        } else {
            return replace_table(c, y, b, a, out);
        }
    } else if b.rows.iter().zip(&a.rows).any(|(x, z)| x.cells.len() != z.cells.len()) {
        // A column added or removed: the same cell index in every row, the rest unchanged.
        let d = a.rows[0].cells.len() as isize - b.rows[0].cells.len() as isize;
        let col = |x: &wordcraft_doc::Row, z: &wordcraft_doc::Row| x.cells.iter().zip(&z.cells).take_while(|(p, q)| p.blocks == q.blocks).count();
        let k = b.rows.iter().zip(&a.rows).map(|(x, z)| col(x, z)).min().unwrap_or(0);
        let fits = d.abs() == 1
            && b.rows.iter().zip(&a.rows).all(|(x, z)| {
                let (long, short) = if d > 0 { (z, x) } else { (x, z) };
                long.cells.len() == short.cells.len() + 1
                    && same_cells(&long.cells[..k], &short.cells[..k])
                    && same_cells(&long.cells[k + 1..], &short.cells[k..])
            });
        if !fits {
            return replace_table(c, y, b, a, out);
        }
        if d > 0 {
            let width = a.grid.get(k).copied().unwrap_or(0.0);
            steps.push(Step::InsertCol { c: c.clone(), block: y, at: k, width, cells: a.rows.iter().map(|r| r.cells[k].clone()).collect() });
        } else {
            steps.push(Step::DeleteCol { c: c.clone(), block: y, at: k });
        }
    }
    // Simulate the structural steps on a copy, then compare what's left cell by cell.
    if !steps.is_empty() {
        let mut d = Document::new();
        d.body = vec![Arc::new(Block::Table(cur))];
        for s in &steps {
            let mut t = s.clone();
            *t.cont_mut().expect("table step") = Cont::body();
            *t.block_mut().expect("table step") = 0;
            if apply(&mut d, &t).is_err() {
                return replace_table(c, y, b, a, out);
            }
        }
        cur = d.body[0].as_table().expect("still a table").clone();
    }
    if cur.rows.len() != a.rows.len() || cur.rows.iter().zip(&a.rows).any(|(x, z)| x.cells.len() != z.cells.len()) {
        return replace_table(c, y, b, a, out);
    }
    out.append(&mut steps);
    if cur.props != a.props || cur.grid != a.grid {
        out.push(Step::TableAttrs { c: c.clone(), block: y, props: Box::new(a.props.clone()), grid: a.grid.clone() });
    }
    for (r, (x, z)) in cur.rows.iter().zip(&a.rows).enumerate() {
        if x.props != z.props {
            out.push(Step::RowAttrs { c: c.clone(), block: y, row: r, props: Box::new(z.props.clone()) });
        }
        for (k, (p, q)) in x.cells.iter().zip(&z.cells).enumerate() {
            if p.props != q.props {
                out.push(Step::CellAttrs { c: c.clone(), block: y, row: r, cell: k, props: Box::new(q.props.clone()) });
            }
            if p.blocks != q.blocks {
                diff_blocks(&c.cell(y, r, k), &p.blocks, &q.blocks, out);
            }
        }
    }
}
