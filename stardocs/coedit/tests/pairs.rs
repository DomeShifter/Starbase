//! SPIKE A: the transform on its own. Random small documents and two random lists of steps made
//! concurrently on them; transforming each over the other must give steps that apply, and both
//! orders must reach the same document (TP1: what the client rebase and the server's mapping of
//! late changes rely on). Differences only in paragraph formatting are counted and reported
//! separately (known gap, see the design doc's Spike A results).

use std::sync::Arc;

use stardocs_coedit::sim::Rng;
use stardocs_coedit::{Step, apply_all, normalized, short, xform_lists};
use wordcraft_doc::{Block, CharProps, Document, ParaProps, Paragraph, Run};

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
        let _ = apply_all(&mut d, &[Step::Format { block: 0, segs: vec![(a.min(b), a.max(b), props(rng))] }]);
        p = d.body[0].as_para().expect("para").clone();
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

fn doc(rng: &mut Rng) -> Document {
    let mut d = Document::new();
    d.body = (0..1 + rng.below(4)).map(|_| Arc::new(Block::Para(para(rng)))).collect();
    d
}

fn step(rng: &mut Rng, d: &Document) -> Step {
    let n = d.body.len();
    let k = rng.below(n);
    let p = d.body[k].as_para().expect("paragraphs only").clone();
    match rng.below(9) {
        0..=2 => {
            let (a, b) = (off(rng, &p), off(rng, &p));
            let t = text(rng, 3);
            let runs = if t.is_empty() { vec![] } else { vec![Run { len: t.len(), props: props(rng) }] };
            Step::Splice { block: k, from: a.min(b), to: a.max(b), text: t, runs, objects: vec![] }
        }
        3 => {
            let (a, b) = (off(rng, &p), off(rng, &p));
            Step::Format { block: k, segs: vec![(a.min(b), a.max(b), props(rng))] }
        }
        4 => {
            let styles = [None, Some("Heading1"), Some("Title")];
            let props = ParaProps { style: styles[rng.below(3)].map(String::from), ..ParaProps::default() };
            Step::ParaProps { block: k, props, mark: p.mark.clone(), section: p.section.clone() }
        }
        5 => Step::Split { block: k, off: off(rng, &p) },
        6 if k + 1 < n => Step::Join { block: k, at: p.text.len() },
        7 => {
            let f = rng.below(n + 1);
            let t = f + rng.below(n + 1 - f);
            let mut count = rng.below(3);
            if count == 0 && t - f == n {
                count = 1;
            }
            let blocks = (0..count).map(|_| Arc::new(Block::Para(para(rng)))).collect();
            Step::Blocks { from: f, to: t, blocks, old: d.body[f..t].to_vec() }
        }
        8 => {
            let mut m = d.clone();
            m.body.clear();
            m.settings.track_changes = rng.chance(50);
            Step::Meta(Box::new(m))
        }
        _ => Step::Split { block: k, off: 0 },
    }
}

fn steps(rng: &mut Rng, d: &Document) -> Vec<Step> {
    let mut d = d.clone();
    let mut v = Vec::new();
    for _ in 0..1 + rng.below(3) {
        let s = step(rng, &d);
        apply_all(&mut d, std::slice::from_ref(&s)).expect("a generated step applies");
        v.push(s);
    }
    v
}

fn texts(d: &Document) -> Vec<String> {
    d.body.iter().map(|b| b.as_para().map(|p| p.text.clone()).unwrap_or_default()).collect()
}

fn runs_eq(a: &Document, b: &Document) -> bool {
    a.body.iter().zip(&b.body).all(|(x, y)| match (x.as_para(), y.as_para()) {
        (Some(p), Some(q)) => p.text == q.text && p.runs == q.runs && p.objects == q.objects,
        _ => x == y,
    }) && a.body.len() == b.body.len()
}

#[test]
fn transform_pairs_tp1() {
    let iterations: u64 = std::env::var("COEDIT_PAIRS").ok().and_then(|v| v.parse().ok()).unwrap_or(200_000);
    let (mut props_only, mut example) = (0usize, None);
    for seed in 0..iterations {
        let mut rng = Rng::new(seed);
        let d = doc(&mut rng);
        let a = steps(&mut rng, &d);
        let b = steps(&mut rng, &d);
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
        if !runs_eq(&n1, &n2) {
            panic!(
                "seed {seed}: the two orders differ\n d {:?}\n a {}\n b {}\n a2 {}\n b2 {}\n b,a' {:?}\n a,b' {:?}",
                texts(&d),
                short(&a),
                short(&b),
                short(&a2),
                short(&b2),
                texts(&n1),
                texts(&n2)
            );
        }
        if n1 != n2 {
            props_only += 1;
            example.get_or_insert((seed, a, b));
        }
    }
    println!(
        "{iterations} random pairs of concurrent step lists: all apply, text/runs/objects agree; paragraph-formatting-only differences: {props_only}"
    );
    if let Some((seed, a, b)) = example {
        println!("first formatting-only difference: seed {seed}\n a {}\n b {}", short(&a), short(&b));
    }
}
