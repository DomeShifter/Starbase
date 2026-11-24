use super::*;
use serde_json::json;

/// A session with a blank workbook and the sync log on.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({})).unwrap();
    s.sync_log = Some(Vec::new());
    s
}

/// A second session holding a copy of `a`'s workbook, as a tab gets it from the server.
fn copy_of(a: &Session) -> Session {
    let mut b = Session::new();
    b.execute("file.new", json!({})).unwrap();
    let wb = workbook_from_json(&workbook_json(&a.active().unwrap().wb), None).unwrap();
    set_workbook(&mut b, wb);
    b
}

fn wb(s: &Session) -> Arc<Workbook> {
    s.active().unwrap().wb.clone()
}

/// What a tab sends for each step its session logged.
fn changes(a: &mut Session) -> Vec<Change> {
    let locale = InputLocale::current();
    a.sync_log
        .replace(Vec::new())
        .unwrap()
        .into_iter()
        .map(|st| {
            if replayable(&st.id) {
                let fp = change_fingerprint(&st.before, &st.after);
                Change::Cmd { sheet: st.sheet, sel: st.selection, id: st.id, params: st.params, locale: locale.clone(), fp }
            } else {
                Change::State { patch: make_patch(&st.before, &st.after) }
            }
        })
        .collect()
}

/// Replays `a`'s changes in `b` and checks every one gives the same result.
fn replay(a: &mut Session, b: &mut Session) -> usize {
    let cs = changes(a);
    let n = cs.len();
    for c in cs {
        let before = wb(b);
        apply(b, &c).unwrap();
        if let Change::Cmd { fp, id, .. } = &c {
            assert_eq!(&change_fingerprint(&before, &wb(b)), fp, "replaying {id} gave a different result");
        }
    }
    assert_eq!(fingerprint(&wb(a)), fingerprint(&wb(b)));
    n
}

#[test]
fn commands_replay_to_the_same_workbook() {
    let mut a = session();
    let mut b = copy_of(&a);
    for (id, p) in [
        ("cell.set", json!({"cell": "A1", "input": "Item"})),
        ("cell.set", json!({"cell": "B1", "input": "12.5"})),
        ("cell.set", json!({"cell": "B2", "input": "=B1*2"})),
        ("cell.set", json!({"cell": "C3", "input": "=NOW()"})),
        ("home.bold", json!({"range": "A1:B1"})),
        ("home.insertRows", json!({"range": "A1"})),
        ("home.mergeCells", json!({"range": "D1:E2"})),
        ("sheet.rename", json!({"name": "Budget"})),
        ("home.insertSheet", json!({})),
    ] {
        a.execute(id, p).unwrap();
    }
    assert_eq!(replay(&mut a, &mut b), 9);
}

#[test]
fn a_change_uses_its_own_selection_not_the_tabs() {
    let mut a = session();
    let mut b = copy_of(&a);
    a.execute("selection.set", json!({"range": "B2:C3"})).unwrap();
    a.execute("home.bold", json!({})).unwrap();
    b.execute("selection.set", json!({"range": "F9"})).unwrap();
    assert_eq!(replay(&mut a, &mut b), 1);
    assert_eq!(b.active().unwrap().selection.active, b.active().unwrap().selection.ranges[0].start, "b keeps its own place");
}

#[test]
fn a_paste_travels_as_its_finished_state() {
    let mut a = session();
    a.execute("cell.set", json!({"cell": "A1", "input": "copy me"})).unwrap();
    let mut b = copy_of(&a);
    a.sync_log = Some(Vec::new());
    a.execute("selection.set", json!({"range": "A1"})).unwrap();
    a.execute("edit.copy", json!({})).unwrap();
    a.execute("selection.set", json!({"range": "C5"})).unwrap();
    a.execute("edit.paste", json!({})).unwrap();
    let cs = changes(&mut a);
    assert!(cs.iter().any(|c| matches!(c, Change::State { .. })), "{cs:?}");
    for c in &cs {
        apply(&mut b, c).unwrap();
    }
    assert_eq!(fingerprint(&wb(&a)), fingerprint(&wb(&b)));
}

#[test]
fn replaying_leaves_history_and_log_alone() {
    let mut a = session();
    let mut b = copy_of(&a);
    b.execute("cell.set", json!({"cell": "Z1", "input": "mine"})).unwrap();
    b.sync_log = Some(Vec::new());
    let undo = b.active().unwrap().undo.len();
    a.execute("cell.set", json!({"cell": "A1", "input": "theirs"})).unwrap();
    for c in changes(&mut a) {
        apply(&mut b, &c).unwrap();
    }
    assert_eq!(b.active().unwrap().undo.len(), undo);
    assert!(b.sync_log.as_ref().unwrap().is_empty());
}

#[test]
fn the_server_runs_only_editing_commands() {
    for id in ["cell.set", "home.bold", "home.insertRows", "edit.fillDown"] {
        assert!(server_allows(id), "{id}");
    }
    for id in ["file.saveAs", "file.open", "macro.run", "automate.runScript", "no.such.command", "selection.set"] {
        assert!(!server_allows(id), "{id}");
    }
}

#[test]
fn messages_round_trip() {
    let m = ToServer::Change {
        cseq: 3,
        base: 9,
        change: Change::State { patch: Patch { rest: serde_json::value::to_raw_value(&json!({})).unwrap(), sheets: vec![], spills: vec![] } },
    };
    let t = serde_json::to_string(&m).unwrap();
    assert!(matches!(serde_json::from_str::<ToServer>(&t).unwrap(), ToServer::Change { cseq: 3, base: 9, .. }));
}

/// Random command runs replay to identical workbooks (seeds 1-8, 150 steps each).
#[test]
fn random_runs_replay_exactly() {
    for seed in 1..=8u64 {
        let mut r = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut next = |n: u64| {
            r ^= r << 13;
            r ^= r >> 7;
            r ^= r << 17;
            r % n
        };
        let mut a = session();
        let mut b = copy_of(&a);
        for _ in 0..150 {
            let cell = format!("{}{}", (b'A' + next(8) as u8) as char, 1 + next(12));
            let range = format!("{cell}:{}{}", (b'A' + 8 + next(3) as u8) as char, 13 + next(5));
            let (id, p) = match next(11) {
                0..=2 => ("cell.set", json!({"cell": cell, "input": format!("{}", next(1000))})),
                3 => ("cell.set", json!({"cell": cell, "input": format!("=SUM(A1:A{})+{}", 1 + next(9), next(9))})),
                4 => ("home.bold", json!({"range": range})),
                5 => ("home.insertRows", json!({"range": cell})),
                6 => ("home.deleteRows", json!({"range": cell})),
                7 => ("home.insertColumns", json!({"range": cell})),
                8 => ("edit.clearContents", json!({"range": range})),
                9 => ("edit.fillDown", json!({"range": range})),
                _ => ("home.fillColor", json!({"range": range, "color": "#FFCC00"})),
            };
            let _ = a.execute(id, p);
        }
        let n = replay(&mut a, &mut b);
        assert!(n > 100, "seed {seed}: only {n} of 150 commands changed the workbook");
    }
}

/// One step's before and after, made in `a`.
fn step(a: &mut Session, id: &str, p: serde_json::Value) -> (Arc<Workbook>, Arc<Workbook>) {
    let before = wb(a);
    a.execute(id, p).unwrap();
    (before, wb(a))
}

fn text_at(s: &Session, cell: &str) -> String {
    let c = gridcraft_engine::core::CellRef::parse(cell).unwrap();
    format!("{:?}", s.active().unwrap().wb.sheets[0].value(c))
}

#[test]
fn undo_your_change_keeps_later_changes_by_others() {
    let mut s = session();
    let (b1, a1) = step(&mut s, "cell.set", json!({"cell": "A1", "input": "mine"}));
    step(&mut s, "cell.set", json!({"cell": "B1", "input": "theirs"}));
    step(&mut s, "cell.set", json!({"cell": "C1", "input": "=LEN(A1)"}));
    let undone = undo_cells(&b1, &a1, &wb(&s)).unwrap();
    set_workbook_recalc(&mut s, undone);
    assert_eq!(text_at(&s, "A1"), "Empty");
    assert!(text_at(&s, "B1").contains("theirs"));
    assert!(text_at(&s, "C1").contains('0'), "follows A1: {}", text_at(&s, "C1"));
    // Redo puts it back.
    let redone = undo_cells(&a1, &b1, &wb(&s)).unwrap();
    set_workbook_recalc(&mut s, redone);
    assert!(text_at(&s, "A1").contains("mine"));
}

#[test]
fn undo_is_refused_when_someone_changed_those_cells_or_the_layout() {
    let mut s = session();
    let (b1, a1) = step(&mut s, "cell.set", json!({"cell": "A1", "input": "mine"}));
    step(&mut s, "cell.set", json!({"cell": "A1", "input": "overwritten"}));
    assert_eq!(undo_cells(&b1, &a1, &wb(&s)).unwrap_err(), UNDO_CHANGED);
    let (b2, a2) = step(&mut s, "home.columnWidth", json!({"range": "B1", "width": 30}));
    assert!(!Arc::ptr_eq(&b2, &a2), "the width changed");
    assert_eq!(undo_cells(&b2, &a2, &wb(&s)).unwrap_err(), UNDO_LAYOUT);
    // Formatting is cells too.
    let (b3, a3) = step(&mut s, "home.bold", json!({"range": "D4"}));
    step(&mut s, "cell.set", json!({"cell": "E5", "input": "later"}));
    assert!(undo_cells(&b3, &a3, &wb(&s)).is_ok());
}

#[test]
fn a_sheet_copied_to_another_workbook_keeps_its_look() {
    use gridcraft_engine::Session;
    let mut a = Session::new();
    a.execute("file.new", serde_json::json!({})).unwrap();
    a.execute("cell.set", serde_json::json!({"cell": "A1", "input": "Bold one"})).unwrap();
    a.execute("selection.set", serde_json::json!({"range": "A1"})).unwrap();
    a.execute("home.bold", serde_json::json!({})).unwrap();
    let src = a.active().unwrap().wb.clone();
    let mut b = Session::new();
    b.execute("file.new", serde_json::json!({})).unwrap();
    // Give the target a few styles of its own first, so ids differ.
    b.execute("selection.set", serde_json::json!({"range": "C3"})).unwrap();
    b.execute("home.italic", serde_json::json!({})).unwrap();
    let mut dst = (*b.active().unwrap().wb).clone();
    let name = crate::copy_sheet_into(&src, 0, &mut dst).unwrap();
    assert_eq!(name, format!("{} (2)", src.sheets[0].name), "same name gets a number");
    let copied = dst.sheets.last().unwrap();
    let c = gridcraft_engine::core::CellRef { row: 0, col: 0 };
    assert!(dst.styles.get(copied.style_id(c)).font.bold, "bold survives the move");
    assert!(!dst.styles.get(copied.style_id(c)).font.italic);
    assert_ne!(copied.uid.get(), src.sheets[0].uid.get(), "a new identity");
}

/// Whole workbooks and sheets travel as raw JSON text (protocol 3): they survive the round trip
/// through the tagged messages without a `serde_json::Value` tree.
#[test]
fn whole_workbooks_and_patches_round_trip_as_raw_text() {
    let mut a = Session::new();
    a.execute("file.new", json!({})).unwrap();
    a.execute("cell.set", json!({"cell": "B2", "input": "=1+2"})).unwrap();
    let before = a.active().unwrap().wb.clone();
    let m = ToClient::Welcome {
        seq: 4,
        workbook: Some(workbook_json(&before)),
        spills: Vec::new(),
        hash: None,
        can_edit: true,
        last_edit: None,
        you: "c1".into(),
    };
    let t = serde_json::to_string(&m).unwrap();
    let ToClient::Welcome { workbook, .. } = serde_json::from_str::<ToClient>(&t).unwrap() else { panic!("{t}") };
    assert_eq!(fingerprint(&workbook_from_json(&workbook.unwrap(), None).unwrap()), fingerprint(&before));
    a.execute("cell.set", json!({"cell": "C3", "input": "x"})).unwrap();
    let after = a.active().unwrap().wb.clone();
    let m = ToClient::Applied { seq: 5, client: String::new(), cseq: 0, change: Change::State { patch: make_patch(&before, &after) } };
    let t = serde_json::to_string(&m).unwrap();
    let ToClient::Applied { change: Change::State { patch }, .. } = serde_json::from_str::<ToClient>(&t).unwrap() else { panic!("{t}") };
    assert_eq!(fingerprint(&apply_patch(&before, &patch).unwrap()), fingerprint(&after));
}

#[test]
fn clip_goes_into_the_section_and_comes_back() {
    let mut s = Session::new();
    s.execute("file.new", json!({})).unwrap();
    s.execute("range.setValues", json!({"range": "A1", "values": [["a", "b"], ["c", "d"], ["e", "f"]]})).unwrap();
    s.execute("home.wrapText", json!({"range": "A1:B2", "on": false, "clip": true})).unwrap();
    s.execute("workbook.setHostData", json!({"data": "{\"v\":1,\"dv\":[]}"})).unwrap();
    let mut wb = (*s.active().unwrap().wb).clone();
    clip_into_host(&mut wb);
    let host: Json = serde_json::from_str(wb.host_data.as_deref().unwrap()).unwrap();
    assert_eq!(host["clip"], json!([{"sheet": "Sheet1", "ranges": "A1:B2"}]));
    assert_eq!(host["dv"], json!([]), "other parts kept");
    // Read back into a workbook without the style.
    let mut t = Session::new();
    t.execute("file.new", json!({})).unwrap();
    t.execute("range.setValues", json!({"range": "A1", "values": [["a", "b"], ["c", "d"], ["e", "f"]]})).unwrap();
    let mut back = (*t.active().unwrap().wb).clone();
    back.host_data = wb.host_data.clone();
    clip_from_host(&mut back);
    let clip = |w: &gridcraft_engine::model::Workbook, a: &str| {
        w.styles.get(w.sheets[0].style_id(gridcraft_engine::core::CellRef::parse(a).unwrap())).align.clip
    };
    assert!(clip(&back, "A1") && clip(&back, "B2") && !clip(&back, "A3"));
    assert!(!back.host_data.as_deref().unwrap_or("").contains("clip"));
}

/// A tab that gets the whole file from the server shows what array formulas spill, not only
/// their first cell (James's to-do: `={A2:C2;SORTN(SORT(A3:C,2,0),9^9,2,1,TRUE)}` showed "Item").
#[test]
fn whole_workbook_from_the_server_shows_spilled_values() {
    let mut a = session();
    for (cell, input) in [("A1", "Item"), ("B1", "Cost"), ("A2", "Pears"), ("B2", "2"), ("A3", "Apples"), ("B3", "1")] {
        a.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
    }
    a.execute("cell.set", json!({"cell": "D1", "input": "={A1:B1;SORT(A2:B3,1,TRUE)}"})).unwrap();
    let shown = |w: &Workbook, at: &str| w.sheet(0).unwrap().value(gridcraft_engine::core::CellRef::parse(at).unwrap());
    assert_eq!(format!("{:?}", shown(&wb(&a), "D2")), format!("{:?}", shown(&wb(&a), "A3")));
    let plain = workbook_from_json(&workbook_json(&wb(&a)), None).unwrap();
    // Protocol 5: what spills travels beside the JSON, so nothing is recalculated.
    let fixed = workbook_from_parts(&workbook_json(&wb(&a)), &workbook_spills(&wb(&a)), None).unwrap();
    assert_ne!(format!("{:?}", shown(&plain, "D2")), format!("{:?}", shown(&wb(&a), "A3")), "JSON drops spills");
    for at in ["D1", "E1", "D2", "E2", "D3", "E3"] {
        assert_eq!(format!("{:?}", shown(&fixed, at)), format!("{:?}", shown(&wb(&a), at)), "{at}");
    }
}

/// "Last edit by … at …" rides on Welcome as an optional field: a server without it (or a tab
/// that doesn't know it) still understands the message, so no protocol bump or reload is needed.
#[test]
fn welcome_carries_the_last_edit_and_old_welcomes_still_read() {
    let old = r#"{"t":"welcome","c":{"seq":1,"workbook":null,"can_edit":true}}"#;
    let ToClient::Welcome { last_edit, .. } = serde_json::from_str::<ToClient>(old).unwrap() else { panic!() };
    assert_eq!(last_edit, None);
    let raw = serde_json::value::to_raw_value(&serde_json::Value::Null).unwrap();
    let edit = crate::LastEdit { by: "Ada".into(), at: 1_795_000_000 };
    let t = serde_json::to_string(&ToClient::Welcome {
        seq: 2,
        workbook: Some(raw),
        spills: Vec::new(),
        hash: None,
        can_edit: true,
        last_edit: Some(edit.clone()),
        you: "c1".into(),
    })
    .unwrap();
    let ToClient::Welcome { last_edit, .. } = serde_json::from_str::<ToClient>(&t).unwrap() else { panic!("{t}") };
    assert_eq!(last_edit, Some(edit));
}

#[test]
fn an_edit_fingerprint_tells_different_results_apart() {
    use gridcraft_engine::core::{CellRef, Value};
    use gridcraft_engine::model::Cell;
    let base = wb(&session());
    let edited = |row: u32, v: f64| {
        let mut w = (*base).clone();
        w.sheet_mut(0).unwrap().cells.set(CellRef::new(row, 0), Cell::value(Value::Number(v)));
        w
    };
    let a = change_fingerprint(&base, &edited(5, 1.0));
    assert_eq!(a, change_fingerprint(&base, &edited(5, 1.0)), "the same change, the same fingerprint");
    assert_ne!(a, change_fingerprint(&base, &edited(5, 2.0)), "another value");
    assert_ne!(a, change_fingerprint(&base, &edited(6, 1.0)), "another cell");
    let mut taller = edited(5, 1.0);
    taller.sheet_mut(0).unwrap().rows.insert(5, Default::default());
    assert_ne!(a, change_fingerprint(&base, &taller), "a row size changed too");
}

/// A sheet that comes whole in a change keeps what JSON doesn't carry (unmodelled file content,
/// the file's sheet id), so the server's next save doesn't drop it; and what it spills comes too.
#[test]
fn a_sheet_sent_whole_keeps_its_unmodelled_parts_and_spills() {
    let mut a = session();
    a.execute("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
    a.execute("cell.set", json!({"cell": "A2", "input": "2"})).unwrap();
    let mut before = (*wb(&a)).clone();
    before.sheet_mut(0).unwrap().file_id = 7;
    let before = Arc::new(before);
    set_workbook(&mut a, (*before).clone());
    a.execute("cell.set", json!({"cell": "C1", "input": "=SORT(A1:A2,1,FALSE)"})).unwrap();
    let after = wb(&a);
    let patch = make_patch(&before, &after);
    assert!(!patch.spills.is_empty(), "the SORT spill travels with the sheet");
    let got = apply_patch(&before, &patch).unwrap();
    assert_eq!(got.sheets[0].file_id, 7);
    assert_eq!(got.sheets[0].spill, after.sheets[0].spill);
}

/// A formula whose result alone changed (NOW() a moment later) doesn't change an edit's
/// fingerprint: the tab and the server calculate it at different times.
#[test]
fn a_formula_result_moving_on_its_own_is_not_part_of_the_change() {
    use gridcraft_engine::core::{CellRef, Value};
    use gridcraft_engine::model::{Cell, Formula};
    let mut base = (*wb(&session())).clone();
    let mut now = Cell::formula(Formula::new("=NOW()"));
    now.value = Value::Number(1.0);
    base.sheet_mut(0).unwrap().cells.set(CellRef::new(2, 2), now.clone());
    let base = Arc::new(base);
    let edited = |later: f64| {
        let mut w = (*base).clone();
        let sh = w.sheet_mut(0).unwrap();
        sh.cells.set(CellRef::new(0, 0), Cell::value(Value::Number(5.0)));
        let mut n = now.clone();
        n.value = Value::Number(later);
        sh.cells.set(CellRef::new(2, 2), n);
        w
    };
    assert_eq!(change_fingerprint(&base, &edited(1.0)), change_fingerprint(&base, &edited(2.0)));
}

/// Big messages travel compressed and come back exactly; small ones aren't packed.
#[test]
fn big_messages_pack_and_unpack() {
    assert!(pack("{\"t\":\"undo\"}").is_none());
    let big = format!("[{}]", vec!["{\"v\":1.5,\"style\":0}"; 20_000].join(","));
    let packed = pack(&big).expect("big enough");
    assert!(packed.len() * 10 < big.len(), "repetitive JSON shrinks a lot");
    assert_eq!(unpack(&packed).unwrap(), big);
    assert!(unpack(b"not deflate").is_err());
}

/// A small frame that would unpack to a huge message (a deflate bomb) is refused once it passes
/// the limit, without unpacking the rest. (Shown with a 1 MB limit and 64 MB of spaces, which
/// pack to about 64 KB; `unpack` uses the same code with MAX_UNPACKED.)
#[test]
fn a_deflate_bomb_is_refused() {
    use std::io::Write as _;
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    let spaces = vec![b' '; 1024 * 1024];
    for _ in 0..64 {
        e.write_all(&spaces).unwrap();
    }
    let bomb = e.finish().unwrap();
    assert!(bomb.len() < 1024 * 1024, "a small frame: {}", bomb.len());
    let err = unpack_at_most(&bomb, 1024 * 1024).unwrap_err();
    assert!(err.contains("too big"), "{err}");
    // Right at the limit is fine; one byte over is not.
    let at = pack(&"x".repeat(PACK_FROM * 2)).unwrap();
    assert!(unpack_at_most(&at, PACK_FROM * 2).is_ok());
    assert!(unpack_at_most(&at, PACK_FROM * 2 - 1).is_err());
    assert!(fits("{}", None));
    assert!(!fits("{}", Some(&vec![0; MAX_FRAME + 1])));
}
