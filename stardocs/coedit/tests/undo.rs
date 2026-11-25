//! Per-user undo: undo my last change after other people have edited, through the real engine
//! sessions and the in-memory server. Their edits stay; mine goes.

use std::collections::VecDeque;
use std::sync::OnceLock;

use serde_json::json;
use stardocs_coedit::sim::{Client, Msg, Server, Stats, base_docx, body_pos, drain, open};
use stardocs_coedit::step::{Cont, para_ref};
use wordcraft_doc::Document;
use wordcraft_engine::Selection;

fn base() -> &'static [u8] {
    static BASE: OnceLock<Vec<u8>> = OnceLock::new();
    BASE.get_or_init(base_docx)
}

struct World {
    server: Server,
    clients: Vec<Client>,
    up: Vec<VecDeque<Msg>>,
    down: Vec<VecDeque<Msg>>,
    stats: Stats,
}

impl World {
    fn new(n: usize) -> World {
        World {
            server: Server { doc: open(base()), log: Vec::new() },
            clients: (0..n).map(|i| Client::new(i, open(base()))).collect(),
            up: vec![VecDeque::new(); n],
            down: vec![VecDeque::new(); n],
            stats: Stats::default(),
        }
    }
    fn sync(&mut self) {
        drain(&mut self.server, &mut self.clients, &mut self.up, &mut self.down, &mut self.stats).expect("sync");
        for c in &self.clients {
            assert!(c.confirmed == self.server.doc, "client {} differs from the server", c.id);
        }
    }
    fn type_at(&mut self, who: usize, k: usize, off: usize, text: &str) {
        self.clients[who].command(Some(body_pos(k, off)), "text.insert", json!({"text": text}), &mut self.stats).expect("type");
    }
    fn undo(&mut self, who: usize) {
        self.clients[who].undo(&mut self.stats).expect("undo").expect("something to undo");
    }
    fn text(&self, k: usize) -> String {
        para_ref(&self.server.doc, &Cont::body(), k).expect("a paragraph").text.clone()
    }
}

/// The body paragraph "Sphinx of black quartz, judge my vow." (no objects in it).
fn sphinx(d: &Document) -> usize {
    (0..d.body.len()).find(|k| para_ref(d, &Cont::body(), *k).is_some_and(|p| p.text.starts_with("Sphinx"))).expect("sphinx paragraph")
}

#[test]
fn undo_my_typing_keeps_what_others_typed_around_and_inside_it() {
    let mut w = World::new(2);
    let k = sphinx(&w.server.doc);
    let t = w.text(k);
    w.type_at(0, k, 7, "123");
    w.sync();
    w.type_at(1, k, 0, "45");
    w.sync();
    // Inside my "123", between 1 and 2.
    w.type_at(1, k, 2 + 7 + 1, "6");
    w.sync();
    w.undo(0);
    w.sync();
    assert_eq!(w.text(k), format!("45{}6{}", &t[..7], &t[7..]));
}

#[test]
fn undo_enter_after_others_typed_in_both_halves() {
    let mut w = World::new(2);
    let k = sphinx(&w.server.doc);
    let t = w.text(k);
    w.clients[0].command(Some(body_pos(k, 6)), "text.newParagraph", json!({}), &mut w.stats).expect("enter");
    w.sync();
    w.type_at(1, k, 6, "8");
    w.type_at(1, k + 1, 0, "9");
    w.sync();
    w.undo(0);
    w.sync();
    assert_eq!(w.text(k), format!("{}89{}", &t[..6], &t[6..]));
}

#[test]
fn undo_after_someone_deleted_my_text_does_nothing_harmful() {
    let mut w = World::new(2);
    let k = sphinx(&w.server.doc);
    w.type_at(0, k, 0, "123");
    w.sync();
    let c1 = &mut w.clients[1];
    c1.session.sel = Selection { anchor: body_pos(k, 0), focus: body_pos(k, 10) };
    c1.command(None, "text.delete", json!({}), &mut w.stats).expect("delete");
    w.sync();
    let before = w.text(k);
    w.undo(0);
    w.sync();
    assert_eq!(w.text(k), before, "my text was already gone: nothing to take away");
}

#[test]
fn undo_a_table_row_and_a_comment_while_others_edit() {
    let mut w = World::new(3);
    let d = &w.server.doc;
    let t = (0..d.body.len()).find(|k| d.body[*k].as_table().is_some()).expect("a table");
    let cell = |r: usize| stardocs_coedit::sim::pos(&Cont::body().cell(t, r, 0), 0, 0);
    // Client 0 adds a row; client 1 types in the first row meanwhile; client 2 comments.
    w.clients[0].command(Some(cell(0)), "table.insertRowBelow", json!({}), &mut w.stats).expect("row");
    w.clients[1].command(Some(cell(0)), "text.insert", json!({"text": "77"}), &mut w.stats).expect("type");
    let c2 = &mut w.clients[2];
    let k = sphinx(&c2.session.doc);
    c2.session.sel = Selection { anchor: body_pos(k, 0), focus: body_pos(k, 6) };
    c2.command(None, "review.newComment", json!({"text": "hm"}), &mut w.stats).expect("comment");
    w.sync();
    let rows = w.server.doc.body[t].as_table().expect("table").rows.len();
    let comments = w.server.doc.comments.len();
    w.undo(0);
    w.undo(2);
    w.sync();
    let table = w.server.doc.body[t].as_table().expect("table");
    assert_eq!(table.rows.len(), rows - 1, "the added row is gone");
    assert!(para_ref(&w.server.doc, &Cont::body().cell(t, 0, 0), 0).expect("cell").text.starts_with("77"), "the other person's typing stays");
    assert_eq!(w.server.doc.comments.len(), comments - 1, "the comment is gone");
}
