//! The server-side room (`DocRoom`): ordering, replay for a reconnecting client, "too old" gives
//! the whole document, a re-sent or stale change, and carets mapped through other people's
//! changes. (Convergence of random sessions runs through the same room: see converge.rs.)

use stardocs_coedit::sim::{base_docx, open};
use stardocs_coedit::step::{Cont, para_ref};
use stardocs_coedit::{Caret, Catchup, DocRoom, Received, Step, Where, apply_all, normalized};
use wordcraft_doc::{CharProps, Document, Run};

fn ins(at: usize, text: &str) -> Vec<Step> {
    vec![Step::Splice {
        c: Cont::body(),
        block: 0,
        from: at,
        to: at,
        text: text.into(),
        runs: vec![Run { len: text.len(), props: CharProps::default() }],
        objects: vec![],
    }]
}

fn room(keep: usize, check_every: u64) -> DocRoom {
    DocRoom::with_limits(open(&base_docx()), keep, check_every)
}

fn text(doc: &Document) -> String {
    para_ref(doc, &Cont::body(), 0).expect("a paragraph").text.clone()
}

fn applied(r: Result<Received, String>) -> (u64, Vec<Step>) {
    match r.expect("accepted") {
        Received::Applied { seq, steps, .. } => (seq, steps),
        other => panic!("expected Applied, got {other:?}"),
    }
}

fn at(offset: usize) -> Where {
    Where { caret: Caret { cont: Cont::body(), para: 0, offset }, selection: None }
}

#[test]
fn changes_are_numbered_and_logged() {
    let mut r = room(usize::MAX, 0);
    let (s1, _) = applied(r.receive(1, 1, 0, ins(0, "hello")));
    let (s2, _) = applied(r.receive(2, 1, 1, ins(5, " world")));
    assert_eq!((s1, s2, r.seq()), (1, 2, 2));
    assert!(text(&r.doc).starts_with("hello world"));
    let log: Vec<_> = r.log().map(|e| (e.seq, e.author)).collect();
    assert_eq!(log, [(1, 1), (2, 2)]);
}

#[test]
fn stale_base_is_transformed() {
    // Both type at the start of the same paragraph on revision 0; the second arrives later.
    let mut r = room(usize::MAX, 0);
    let before = text(&r.doc);
    applied(r.receive(1, 1, 0, ins(0, "AA")));
    applied(r.receive(2, 1, 0, ins(0, "BB")));
    let t = text(&r.doc);
    assert!(t.contains("AA") && t.contains("BB") && t.len() == before.len() + 4, "{t}");
}

#[test]
fn reconnect_replays_the_tail() {
    let mut r = room(usize::MAX, 0);
    let start = r.doc.clone();
    applied(r.receive(1, 1, 0, ins(0, "one ")));
    applied(r.receive(2, 1, 1, ins(0, "two ")));
    let mut c = r.doc.clone(); // a client that has everything up to 2 ...
    applied(r.receive(1, 2, 2, ins(0, "three ")));
    applied(r.receive(2, 2, 3, ins(0, "four ")));
    // ... was away for 3 and 4.
    let Catchup::Tail(tail) = r.catch_up(2) else { panic!("expected a tail") };
    assert_eq!(tail.iter().map(|e| e.seq).collect::<Vec<_>>(), [3, 4]);
    for e in &tail {
        apply_all(&mut c, &e.steps).unwrap();
    }
    assert_eq!(normalized(&c), normalized(&r.doc));
    // Up to date: an empty tail. Replaying the whole log from the start gives the same document.
    assert_eq!(r.catch_up(4), Catchup::Tail(vec![]));
    let mut s = start;
    for e in r.log() {
        apply_all(&mut s, &e.steps).unwrap();
    }
    assert_eq!(normalized(&s), normalized(&r.doc));
}

#[test]
fn too_old_or_unknown_seq_gets_the_full_document() {
    let mut r = room(2, 0); // keeps the last two changes
    for i in 0..5u64 {
        applied(r.receive(1, i + 1, i, ins(0, "x")));
    }
    assert_eq!(r.seq(), 5);
    assert_eq!(r.catch_up(0), Catchup::Full, "has nothing");
    assert_eq!(r.catch_up(1), Catchup::Full, "log starts after 3");
    assert_eq!(r.catch_up(2), Catchup::Full);
    assert!(matches!(r.catch_up(3), Catchup::Tail(t) if t.len() == 2));
    assert!(matches!(r.catch_up(5), Catchup::Tail(t) if t.is_empty()));
    assert_eq!(r.catch_up(99), Catchup::Full, "ahead of the server (it restarted)");
    // A change whose base fell out of the log is refused, untouched document.
    let before = r.doc.clone();
    let err = r.receive(2, 1, 1, ins(0, "late")).unwrap_err();
    assert!(err.contains("too old"), "{err}");
    assert_eq!(r.doc, before);
    assert_eq!(r.seq(), 5);
    assert!(r.receive(2, 1, 99, ins(0, "future")).is_err());
}

#[test]
fn a_resent_change_is_applied_once_and_a_lost_one_leaves_a_gap() {
    let mut r = room(usize::MAX, 0);
    applied(r.receive(1, 1, 0, ins(0, "once ")));
    let after = r.doc.clone();
    // The acknowledgement was lost, the client sends cseq 1 again (also after reconnecting).
    assert_eq!(r.receive(1, 1, 0, ins(0, "once ")), Ok(Received::Duplicate { seq: 1 }));
    r.leave(1);
    assert_eq!(r.receive(1, 1, 0, ins(0, "once ")), Ok(Received::Duplicate { seq: 1 }));
    assert_eq!((r.seq(), &r.doc), (1, &after));
    // cseq 2 never arrived (dropped): 3 is still taken, and 2 arriving late is not.
    applied(r.receive(1, 3, 1, ins(0, "three ")));
    assert_eq!(r.receive(1, 2, 1, ins(0, "two ")), Ok(Received::Duplicate { seq: 2 }));
    assert_eq!(r.seq(), 2);
}

#[test]
fn caret_moves_when_another_user_types_before_it() {
    let mut r = room(usize::MAX, 0);
    applied(r.receive(1, 1, 0, ins(0, "hello world")));
    r.set_presence(2, at(5));
    r.set_presence(1, at(11));
    applied(r.receive(1, 2, 1, ins(0, "XX")));
    assert_eq!(r.presence(2).unwrap().caret.offset, 7, "B's caret moved right by A's insert");
    assert_eq!(r.presence(1).unwrap().caret.offset, 11, "the author's own caret is theirs to move");
    // Typing after the caret leaves it where it is; a selection's other end moves too.
    r.set_presence(2, Where { caret: Caret { cont: Cont::body(), para: 0, offset: 3 }, selection: at(9).caret.into() });
    applied(r.receive(1, 3, 2, ins(8, "YYY")));
    let w = r.presence(2).unwrap();
    assert_eq!((w.caret.offset, w.selection.as_ref().unwrap().offset), (3, 12));
}

#[test]
fn caret_wire_round_trip() {
    let c = Caret { cont: Cont::body().cell(2, 1, 0), para: 4, offset: 9 };
    assert_eq!(Caret::from_wire(&c.to_wire()), c);
}

#[test]
fn fingerprint_follows_the_document() {
    let mut a = room(usize::MAX, 2);
    let mut b = room(usize::MAX, 0);
    assert_eq!(a.hash(), b.hash());
    let Received::Applied { check, .. } = a.receive(1, 1, 0, ins(0, "x")).unwrap() else { panic!() };
    assert_eq!(check, None, "every second change");
    let Received::Applied { check, .. } = a.receive(1, 2, 1, ins(0, "y")).unwrap() else { panic!() };
    assert_eq!(check, Some(a.hash().unwrap()));
    assert_ne!(a.hash(), b.hash());
    applied(b.receive(1, 1, 0, ins(0, "x")));
    applied(b.receive(1, 2, 1, ins(0, "y")));
    assert_eq!(a.hash(), b.hash(), "same changes, same fingerprint");
}
