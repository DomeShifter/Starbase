//! What working out steps costs per keystroke in a 100-page document (printed with
//! `--nocapture`; the assertion is only a loose guard). Run with `--release` for real numbers.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use stardocs_coedit::sim::body_pos;
use stardocs_coedit::{Cont, Step, apply_all, diff, xform_lists};
use wordcraft_doc::{Block, CharProps, Document, Paragraph};
use wordcraft_engine::{Selection, Session};

fn hundred_pages() -> Document {
    let mut s = Session::new(Document::new());
    let mut d = s.blank_document();
    let words = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua";
    // ~1,500 paragraphs of ~35 words: about 52,000 words, 100 pages of 12 pt text.
    d.body = (0..1500)
        .map(|i| {
            let t = format!("{i}. {words} {words}");
            Arc::new(Block::Para(Paragraph::with_text(&t, CharProps::default())))
        })
        .collect();
    s.doc = d;
    s.doc
}

fn stats(name: &str, v: &[Duration]) -> f64 {
    let mut ms: Vec<f64> = v.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    ms.sort_by(f64::total_cmp);
    let avg = ms.iter().sum::<f64>() / ms.len() as f64;
    println!("{name:<28} avg {avg:8.3} ms  median {:8.3} ms  max {:8.3} ms  (n={})", ms[ms.len() / 2], ms[ms.len() - 1], ms.len());
    avg
}

#[test]
fn keystroke_cost_100_pages() {
    let doc = hundred_pages();
    let mut s = Session::new(doc.clone());
    let mut replica = doc;
    s.sel = Selection::caret(body_pos(750, 20));
    let (mut cmd, mut dif, mut app, mut xf) = (vec![], vec![], vec![], vec![]);
    // A concurrent keystroke by someone else in the same paragraph, to transform over.
    let remote = vec![Step::Splice {
        c: Cont::body(),
        block: 750,
        from: 5,
        to: 5,
        text: "z".into(),
        runs: vec![wordcraft_doc::Run { len: 1, props: CharProps::default() }],
        objects: vec![],
    }];
    for i in 0..400 {
        let before = s.doc.clone();
        let t = Instant::now();
        let id = if i % 40 == 39 { "text.newParagraph" } else { "text.insert" };
        s.run(id, &json!({"text": "k"})).expect("command");
        cmd.push(t.elapsed());
        let t = Instant::now();
        let steps = diff(&before, &s.doc);
        dif.push(t.elapsed());
        assert!(!steps.is_empty());
        let t = Instant::now();
        apply_all(&mut replica, &steps).expect("apply");
        app.push(t.elapsed());
        let t = Instant::now();
        let _ = xform_lists(steps, remote.clone());
        xf.push(t.elapsed());
    }
    println!("100-page document (1,500 paragraphs), 400 keystrokes (every 40th is Enter):");
    stats("engine command (Session::run)", &cmd);
    let d = stats("diff -> steps", &dif);
    stats("apply steps (other copy)", &app);
    stats("transform over a remote step", &xf);
    assert!(d < 50.0, "diff should be far below a frame");
}
