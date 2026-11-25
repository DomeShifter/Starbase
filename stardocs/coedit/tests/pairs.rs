//! The transform on its own. Random small documents (body paragraphs, tables with paragraphs in
//! their cells, a header story, comments, revisions) and two random lists of steps made
//! concurrently on them; transforming each over the other must give steps that apply, and both
//! orders must reach exactly the same document (TP1: what the client rebase and the server's
//! mapping of late changes rely on). Also: every list followed by its inverse gives back the
//! document it started from.

use std::sync::Arc;

use stardocs_coedit::sim::{Rng, all_paras};
use stardocs_coedit::step::{Attrs, Cont, blocks_ref, fields_of, para_ref, table_ref};
use stardocs_coedit::{Step, apply_all, invert_all, normalized, short, xform_lists};
use wordcraft_doc::props::TableProps;
use wordcraft_doc::{Block, Blocks, Cell, CharProps, Comment, Document, ParaProps, Paragraph, Part, PartKind, Revision, Row, Run, StoryRef, Table};

fn props(rng: &mut Rng) -> CharProps {
    let mut p = CharProps::default();
    match rng.below(4) {
        0 => p.bold = Some(true),
        1 => p.italic = Some(true),
        _ => {}
    }
    p
}

fn text(rng: &mut Rng, max: usize) -> String {
    let alphabet = ['a', 'b', 'c', 'é', ' ', '日'];
    (0..rng.below(max + 1)).map(|_| alphabet[rng.below(alphabet.len())]).collect()
}

fn para(rng: &mut Rng) -> Paragraph {
    let t = text(rng, 8);
    let mut p = Paragraph::with_text(&t, CharProps::default());
    if !t.is_empty() {
        let (a, b) = (off(rng, &p), off(rng, &p));
        let mut d = Document::new();
        d.body = vec![Arc::new(Block::Para(p.clone()))];
        let _ = apply_all(&mut d, &[Step::Format { c: Cont::body(), block: 0, segs: vec![(a.min(b), a.max(b), props(rng))] }]);
        p = d.body[0].as_para().expect("para").clone();
    }
    if rng.chance(20) {
        p.props.style = Some("Title".into());
    }
    p
}

fn off(rng: &mut Rng, p: &Paragraph) -> usize {
    let mut o = rng.below(p.text.len() + 1);
    while !p.text.is_char_boundary(o) {
        o -= 1;
    }
    o
}

fn cell(rng: &mut Rng) -> Cell {
    let mut c = Cell::empty();
    c.blocks = (0..1 + rng.below(2)).map(|_| Arc::new(Block::Para(para(rng)))).collect();
    c
}

fn table(rng: &mut Rng) -> Table {
    let cols = 1 + rng.below(3);
    let rows = (0..1 + rng.below(3)).map(|_| Row { cells: (0..cols).map(|_| cell(rng)).collect(), ..Row::default() }).collect();
    Table { grid: vec![100.0; cols], rows, ..Table::default() }
}

fn blocks(rng: &mut Rng, n: usize) -> Blocks {
    (0..n).map(|_| if rng.chance(20) { Arc::new(Block::Table(table(rng))) } else { Arc::new(Block::Para(para(rng))) }).collect()
}

fn doc(rng: &mut Rng) -> Document {
    let mut d = Document::new();
    d.body = (0..1 + rng.below(4)).map(|_| Arc::new(Block::Para(para(rng)))).collect();
    if rng.chance(60) {
        d.body.insert(rng.below(d.body.len() + 1), Arc::new(Block::Table(table(rng))));
    }
    d.parts.insert(
        1,
        Part { kind: PartKind::Header, blocks: (0..1 + rng.below(2)).map(|_| Arc::new(Block::Para(para(rng)))).collect(), ..Part::default() },
    );
    d.comments.insert(0, Comment { author: "A".into(), part: 1, ..Comment::default() });
    d.revisions.push(Revision { author: "R".into(), ..Revision::default() });
    d
}

/// Tables anywhere: (container, block).
fn tables(d: &Document) -> Vec<(Cont, usize)> {
    let mut conts: Vec<Cont> = vec![Cont::body()];
    conts.extend(d.parts.keys().map(|id| Cont::story(StoryRef::Part(*id))));
    conts.extend(all_paras(d).into_iter().map(|(c, _)| c));
    conts.sort();
    conts.dedup();
    let mut out = Vec::new();
    for c in conts {
        if let Some(bs) = blocks_ref(d, &c) {
            for (i, b) in bs.iter().enumerate() {
                if b.as_table().is_some() {
                    out.push((c.clone(), i));
                }
            }
        }
    }
    out
}

fn table_step(rng: &mut Rng, d: &Document) -> Option<Step> {
    let ts = tables(d);
    if ts.is_empty() {
        return None;
    }
    let (c, y) = ts[rng.below(ts.len())].clone();
    let t = table_ref(d, &c, y)?;
    let (rows, cols) = (t.rows.len(), t.rows.first().map_or(0, |r| r.cells.len()));
    Some(match rng.below(7) {
        0 => Step::InsertRow {
            c,
            block: y,
            at: rng.below(rows + 1),
            row: Box::new(Row { cells: (0..cols).map(|_| cell(rng)).collect(), ..Row::default() }),
        },
        1 if rows > 1 => Step::DeleteRow { c, block: y, at: rng.below(rows) },
        2 => Step::InsertCol { c, block: y, at: rng.below(cols + 1), width: 50.0, cells: (0..rows).map(|_| cell(rng)).collect() },
        3 if cols > 1 => Step::DeleteCol { c, block: y, at: rng.below(cols) },
        4 => {
            let mut g = t.grid.clone();
            g.iter_mut().for_each(|w| *w += 1.0 + rng.below(9) as f32);
            Step::TableAttrs { c, block: y, props: Box::new(TableProps { style: Some(format!("T{}", rng.below(3))), ..t.props.clone() }), grid: g }
        }
        5 if rows > 0 => {
            let mut p = t.rows[0].props.clone();
            p.cant_split = !p.cant_split;
            Step::RowAttrs { c, block: y, row: rng.below(rows), props: Box::new(p) }
        }
        _ if rows > 0 && cols > 0 => {
            let (r, k) = (rng.below(rows), rng.below(cols));
            let mut p = t.rows[r].cells[k].props.clone();
            p.width = Some(10.0 + rng.below(50) as f32);
            Step::CellAttrs { c, block: y, row: r, cell: k, props: Box::new(p) }
        }
        _ => return None,
    })
}

fn steps_for(rng: &mut Rng, d: &Document) -> Vec<Step> {
    let ps = all_paras(d);
    let (c, k) = ps[rng.below(ps.len())].clone();
    let p = para_ref(d, &c, k).expect("a paragraph").clone();
    let n = blocks_ref(d, &c).map_or(0, |b| b.len());
    let one = |s: Step| vec![s];
    match rng.below(16) {
        0..=2 => {
            let (a, b) = (off(rng, &p), off(rng, &p));
            let t = text(rng, 3);
            let runs = if t.is_empty() { vec![] } else { vec![Run { len: t.len(), props: props(rng) }] };
            one(Step::Splice { c, block: k, from: a.min(b), to: a.max(b), text: t, runs, objects: vec![] })
        }
        3 => {
            let (a, b) = (off(rng, &p), off(rng, &p));
            one(Step::Format { c, block: k, segs: vec![(a.min(b), a.max(b), props(rng))] })
        }
        4 => {
            let styles = [None, Some("Heading1"), Some("Title")];
            let props = ParaProps { style: styles[rng.below(3)].map(String::from), ..ParaProps::default() };
            one(Step::ParaProps { c, block: k, attrs: Box::new(Attrs { props, mark: p.mark.clone(), section: p.section.clone() }) })
        }
        5 => {
            let mut tail = Attrs::of(&p);
            if rng.chance(30) {
                tail.props.style = Some("Heading2".into());
            }
            one(Step::Split { c, block: k, off: off(rng, &p), tail: Box::new(tail) })
        }
        6 => match para_ref(d, &c, k + 1) {
            Some(q) => one(Step::Join { c: c.clone(), block: k, at: p.text.len(), gone: Box::new(Attrs::of(q)) }),
            None => vec![],
        },
        7 => {
            let bs = blocks_ref(d, &c).expect("container");
            let f = rng.below(n + 1);
            let t = f + rng.below(n + 1 - f);
            let mut count = rng.below(3);
            if count == 0 && t - f == n {
                count = 1;
            }
            one(Step::Blocks { c, from: f, to: t, blocks: blocks(rng, count), old: bs[f..t].to_vec() })
        }
        8..=9 => table_step(rng, d).into_iter().collect(),
        10 => match rng.below(3) {
            0 => one(Step::PartAdd {
                id: 100 + rng.below(1000) as u32,
                part: Box::new(Part { kind: PartKind::Footnote, blocks: vec![Arc::new(Block::Para(para(rng)))], ..Part::default() }),
            }),
            1 => d.parts.keys().nth(rng.below(d.parts.len())).map(|id| Step::PartRemove { id: *id }).into_iter().collect(),
            _ => d
                .parts
                .iter()
                .nth(rng.below(d.parts.len()))
                .map(|(id, p)| Step::PartAttrs { id: *id, part: Box::new(Part { kind: PartKind::Footer, body: p.body, ..Part::default() }) })
                .into_iter()
                .collect(),
        },
        11 => {
            let id = rng.below(3) as u32;
            let comment = rng
                .chance(70)
                .then(|| Box::new(Comment { author: format!("A{}", rng.below(9)), resolved: rng.chance(50), part: 1, ..Comment::default() }));
            one(Step::Comment { id, comment })
        }
        12..=13 => {
            // A suggestion: a new revision record and tracked text pointing at it.
            let at = d.revisions.len();
            let t = text(rng, 2) + "s";
            let mut cp = props(rng);
            cp.ins = Some(at as u32);
            let o = off(rng, &p);
            vec![
                Step::RevInsert { at, rev: Box::new(Revision { author: format!("S{}", rng.below(9)), ..Revision::default() }) },
                Step::Splice { c, block: k, from: o, to: o, runs: vec![Run { len: t.len(), props: cp }], text: t, objects: vec![] },
            ]
        }
        14 => {
            let mut m = d.clone();
            m.settings.track_changes = rng.chance(50);
            one(Step::Field { name: "settings", from: Box::new(fields_of(&m)) })
        }
        _ => one(Step::Media { key: format!("m{}", rng.below(3)), bytes: rng.chance(70).then(|| Arc::new(vec![rng.below(9) as u8])) }),
    }
}

fn steps(rng: &mut Rng, d: &Document) -> Vec<Step> {
    let mut d = d.clone();
    let mut v = Vec::new();
    for _ in 0..1 + rng.below(3) {
        let s = steps_for(rng, &d);
        let mut t = d.clone();
        if apply_all(&mut t, &s).is_ok() {
            d = t;
            v.extend(s);
        }
    }
    v
}

fn texts(d: &Document) -> Vec<String> {
    all_paras(d).iter().map(|(c, k)| format!("{:?}{:?}#{k}:{}", c.story, c.path, para_ref(d, c, *k).map(|p| p.text.as_str()).unwrap_or(""))).collect()
}

#[test]
fn transform_pairs_tp1() {
    let iterations: u64 = std::env::var("COEDIT_PAIRS").ok().and_then(|v| v.parse().ok()).unwrap_or(200_000);
    let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
    for seed in 0..iterations {
        let mut rng = Rng::new(seed);
        let d = doc(&mut rng);
        let a = steps(&mut rng, &d);
        let b = steps(&mut rng, &d);
        for s in a.iter().chain(&b) {
            *kinds.entry(s.kind()).or_default() += 1;
        }
        // `b` was ordered first (the server's); `a` is the late one.
        let (a2, b2) = xform_lists(a.clone(), b.clone());
        let mut d1 = d.clone();
        apply_all(&mut d1, &b).expect("b applies");
        if let Err(e) = apply_all(&mut d1, &a2) {
            panic!("seed {seed}: a over b doesn't apply: {e}\n d {:?}\n a {}\n b {}\n a2 {}", texts(&d), short(&a), short(&b), short(&a2));
        }
        let mut d2 = d.clone();
        apply_all(&mut d2, &a).expect("a applies");
        if let Err(e) = apply_all(&mut d2, &b2) {
            panic!("seed {seed}: b over a doesn't apply: {e}\n d {:?}\n a {}\n b {}\n b2 {}", texts(&d), short(&a), short(&b), short(&b2));
        }
        let (n1, n2) = (normalized(&d1), normalized(&d2));
        if n1 != n2 {
            let what = if n1.body != n2.body {
                "body"
            } else if n1.parts != n2.parts {
                "stories"
            } else if n1.revisions != n2.revisions {
                "revisions"
            } else if n1.comments != n2.comments {
                "comments"
            } else {
                "other fields"
            };
            panic!(
                "seed {seed}: the two orders differ ({what})\n d {:?}\n a {}\n b {}\n a2 {}\n b2 {}\n b,a' {:?}\n a,b' {:?}",
                texts(&d),
                short(&a),
                short(&b),
                short(&a2),
                short(&b2),
                texts(&n1),
                texts(&n2)
            );
        }
        // Undo: a list followed by its inverse is where it started (revision records stay: nothing
        // refers to them any more).
        let inv = invert_all(&a, &d).unwrap_or_else(|| panic!("seed {seed}: no inverse for {}", short(&a)));
        let mut d3 = d.clone();
        apply_all(&mut d3, &a).expect("a applies");
        if let Err(e) = apply_all(&mut d3, &inv) {
            panic!("seed {seed}: inverse doesn't apply: {e}\n a {}\n inv {}", short(&a), short(&inv));
        }
        let mut want = normalized(&d);
        want.revisions = d3.revisions.clone();
        if normalized(&d3) != want {
            panic!("seed {seed}: inverse doesn't restore\n d {:?}\n a {}\n inv {}\n got {:?}", texts(&d), short(&a), short(&inv), texts(&d3));
        }
    }
    println!(
        "{iterations} random pairs of concurrent step lists: all apply, both orders give the same document, every inverse restores. Steps: {kinds:?}"
    );
}
