//! Inverse steps, for per-user undo: worked out from the document a step was applied to.

use wordcraft_doc::{CharProps, Document};

use crate::step::{Attrs, Step, apply, objs_in, para_ref, runs_slice, table_ref};

/// The steps that undo `s`, made on `doc` (the document just before `s`). `None` when `s` can't
/// be undone on its own (it doesn't apply to `doc`).
pub fn invert(s: &Step, doc: &Document) -> Option<Vec<Step>> {
    let out = match s {
        Step::Splice { c, block, from, to, text, .. } => {
            let p = para_ref(doc, c, *block)?;
            let mut q = p.clone();
            q.normalize();
            let old = q.text.get(*from..*to)?.to_string();
            let k0 = objs_in(&q.text[..*from]);
            let objects = q.objects.get(k0..k0 + objs_in(&old))?.to_vec();
            vec![Step::Splice {
                c: c.clone(),
                block: *block,
                from: *from,
                to: from + text.len(),
                text: old,
                runs: runs_slice(&q.runs, *from, *to),
                objects,
            }]
        }
        Step::Format { c, block, segs } => {
            // Segments apply one after another: undo them in reverse, each from the runs it found.
            let mut d = doc.clone();
            let mut out = Vec::new();
            for seg in segs {
                let mut q = para_ref(&d, c, *block)?.clone();
                q.normalize();
                let mut pos = 0;
                let mut old: Vec<(usize, usize, CharProps)> = Vec::new();
                for r in runs_slice(&q.runs, seg.0, seg.1) {
                    old.push((seg.0 + pos, seg.0 + pos + r.len, r.props));
                    pos += r.len;
                }
                out.push(Step::Format { c: c.clone(), block: *block, segs: old });
                apply(&mut d, &Step::Format { c: c.clone(), block: *block, segs: vec![seg.clone()] }).ok()?;
            }
            out.reverse();
            out
        }
        Step::ParaProps { c, block, .. } => {
            vec![Step::ParaProps { c: c.clone(), block: *block, attrs: Box::new(Attrs::of(para_ref(doc, c, *block)?)) }]
        }
        Step::Split { c, block, off, tail } => vec![Step::Join { c: c.clone(), block: *block, at: *off, gone: tail.clone() }],
        Step::Join { c, block, at, .. } => {
            let q = para_ref(doc, c, block + 1)?;
            vec![Step::Split { c: c.clone(), block: *block, off: *at, tail: Box::new(Attrs::of(q)) }]
        }
        Step::Blocks { c, from, blocks, old, .. } => {
            vec![Step::Blocks { c: c.clone(), from: *from, to: from + blocks.len(), blocks: old.clone(), old: blocks.clone() }]
        }
        Step::InsertRow { c, block, at, .. } => vec![Step::DeleteRow { c: c.clone(), block: *block, at: *at }],
        Step::DeleteRow { c, block, at } => {
            let row = table_ref(doc, c, *block)?.rows.get(*at)?.clone();
            vec![Step::InsertRow { c: c.clone(), block: *block, at: *at, row: Box::new(row) }]
        }
        Step::InsertCol { c, block, at, .. } => vec![Step::DeleteCol { c: c.clone(), block: *block, at: *at }],
        Step::DeleteCol { c, block, at } => {
            let t = table_ref(doc, c, *block)?;
            let cells = t.rows.iter().map(|r| r.cells.get(*at).cloned()).collect::<Option<Vec<_>>>()?;
            vec![Step::InsertCol { c: c.clone(), block: *block, at: *at, width: t.grid.get(*at).copied().unwrap_or(0.0), cells }]
        }
        Step::TableAttrs { c, block, .. } => {
            let t = table_ref(doc, c, *block)?;
            vec![Step::TableAttrs { c: c.clone(), block: *block, props: Box::new(t.props.clone()), grid: t.grid.clone() }]
        }
        Step::RowAttrs { c, block, row, .. } => {
            let r = table_ref(doc, c, *block)?.rows.get(*row)?;
            vec![Step::RowAttrs { c: c.clone(), block: *block, row: *row, props: Box::new(r.props.clone()) }]
        }
        Step::CellAttrs { c, block, row, cell, .. } => {
            let k = table_ref(doc, c, *block)?.rows.get(*row)?.cells.get(*cell)?;
            vec![Step::CellAttrs { c: c.clone(), block: *block, row: *row, cell: *cell, props: Box::new(k.props.clone()) }]
        }
        Step::PartAdd { id, .. } => vec![Step::PartRemove { id: *id }],
        Step::PartRemove { id } => vec![Step::PartAdd { id: *id, part: Box::new(doc.parts.get(id)?.clone()) }],
        Step::PartAttrs { id, .. } => {
            let mut p = doc.parts.get(id)?.clone();
            p.blocks = Vec::new();
            vec![Step::PartAttrs { id: *id, part: Box::new(p) }]
        }
        Step::Comment { id, .. } => vec![Step::Comment { id: *id, comment: doc.comments.get(id).map(|c| Box::new(c.clone())) }],
        // A revision record nobody refers to any more is harmless: left in place.
        Step::RevInsert { .. } => vec![],
        Step::Revisions { .. } => vec![Step::Revisions { all: doc.revisions.clone() }],
        Step::Media { key, .. } => vec![Step::Media { key: key.clone(), bytes: doc.media.get(key).cloned() }],
        Step::Field { name, .. } => vec![Step::Field { name, from: Box::new(crate::step::fields_of(doc)) }],
    };
    Some(out)
}

/// The steps that undo `steps` (applied in order to `doc`), or `None` if one of them doesn't apply.
pub fn invert_all(steps: &[Step], doc: &Document) -> Option<Vec<Step>> {
    let mut d = doc.clone();
    let mut inv: Vec<Vec<Step>> = Vec::new();
    for s in steps {
        inv.push(invert(s, &d)?);
        apply(&mut d, s).ok()?;
    }
    Some(inv.into_iter().rev().flatten().collect())
}
