//! Menu entries that GridCraft runs as one command but whose parameters depend on where you are:
//! Sort sheet, Freeze rows/columns, Edit › Move, Randomise range and Trim whitespace. Each one
//! reads the spreadsheet and returns the GridCraft commands to run, so the menus, the test hook
//! and the tests all go through the same code. Also decides which entries are greyed out.

use gridcraft_engine::Session;
use gridcraft_engine::core::{CellRef, MAX_COLS, RangeRef, Value, col_to_letters};
use serde_json::{Value as Json, json};

/// Commands to run, in order.
pub type Steps = Vec<(&'static str, Json)>;

/// The ids this module answers (`panel:` ids, so the command-id test leaves them alone).
pub const IDS: &[&str] =
    &["panel:sortSheet", "panel:freeze", "panel:moveColumnLeft", "panel:moveColumnRight", "panel:randomise", "panel:trimWhitespace"];

/// Whether `id` is one of ours.
pub fn handles(id: &str) -> bool {
    IDS.contains(&id)
}

/// The active sheet's selection, active cell and the sheet itself.
fn here(s: &Session) -> Result<(RangeRef, CellRef, &gridcraft_engine::model::Sheet), String> {
    let d = s.active().ok_or("There's no spreadsheet open.")?;
    let sh = d.wb.active().ok_or("There's no sheet open.")?;
    Ok((d.selection.current(), d.selection.active, sh))
}

/// The selection cut down to the cells that hold anything (a whole column stays cheap).
fn used_part(s: &Session) -> Option<RangeRef> {
    let (sel, _, sh) = here(s).ok()?;
    sel.intersection(&sh.used_range()?)
}

/// Fills `{col}` and `{row}` in a menu label with the active cell's column letter and row number,
/// as Sheets' "Sort sheet by column B (A to Z)" and "Up to row 4" do.
pub fn label(s: &Session, text: &str) -> String {
    if !text.contains('{') {
        return text.to_string();
    }
    let at = s.active().map(|d| d.selection.active).unwrap_or_default();
    text.replace("{col}", &col_to_letters(at.col)).replace("{row}", &(at.row + 1).to_string())
}

/// Whether one of our entries can run here; `None` for ids that aren't ours.
pub fn enabled(s: &Session, id: &str) -> Option<bool> {
    if !handles(id) {
        return None;
    }
    let Ok((sel, _, sh)) = here(s) else { return Some(false) };
    Some(match id {
        "panel:sortSheet" => sh.used_range().is_some(),
        "panel:moveColumnLeft" => sel.start.col > 0,
        "panel:moveColumnRight" => sel.end.col + 1 < MAX_COLS,
        "panel:randomise" => used_part(s).is_some_and(|r| r.height() > 1),
        _ => true,
    })
}

/// The commands an entry runs, or a message saying why it can't.
pub fn steps(s: &Session, id: &str, p: &Json) -> Result<Steps, String> {
    match id {
        "panel:sortSheet" => sort_sheet(s, p.get("desc").and_then(Json::as_bool).unwrap_or(false)),
        "panel:freeze" => freeze(s, p),
        "panel:moveColumnLeft" => move_columns(s, true),
        "panel:moveColumnRight" => move_columns(s, false),
        "panel:randomise" => randomise(s, p.get("seed").and_then(Json::as_u64).unwrap_or_else(seed)),
        "panel:trimWhitespace" => trim_whitespace(s),
        _ => Err(format!("unknown menu entry {id}")),
    }
}

/// Data › Sort sheet: every row below the frozen ones, by the active cell's column.
fn sort_sheet(s: &Session, desc: bool) -> Result<Steps, String> {
    let (_, at, sh) = here(s)?;
    let used = sh.used_range().ok_or("There's nothing on this sheet to sort.")?;
    let top = sh.freeze.map_or(0, |(rows, _)| rows);
    if used.end.row < top {
        return Err("There's nothing below the frozen rows to sort.".into());
    }
    let range = RangeRef::new(CellRef::new(top, 0), CellRef::new(used.end.row, used.end.col.max(at.col)));
    let order = if desc { "desc" } else { "asc" };
    Ok(vec![("data.sort", json!({"range": range.a1(), "header": false, "keys": [{"column": col_to_letters(at.col), "order": order}]}))])
}

/// View › Freeze: `rows` / `cols` is a count, or "current" for up to the active cell; the one
/// not given keeps what is frozen now, as in Sheets.
fn freeze(s: &Session, p: &Json) -> Result<Steps, String> {
    let (_, at, sh) = here(s)?;
    let (now_rows, now_cols) = sh.freeze.unwrap_or((0, 0));
    let pick = |key: &str, now: u32, current: u32| match p.get(key) {
        Some(Json::String(t)) if t == "current" => current,
        Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(now),
        None => now,
    };
    let rows = pick("rows", now_rows, at.row + 1);
    let cols = pick("cols", now_cols, at.col + 1);
    if rows == 0 && cols == 0 {
        return Ok(vec![("view.unfreezePanes", json!({}))]);
    }
    Ok(vec![("view.freezePanes", json!({"cell": CellRef::new(rows, cols).a1()}))])
}

/// Edit › Move › Column left / right: the selected columns swap places with their neighbour, and
/// the selection follows them.
fn move_columns(s: &Session, left: bool) -> Result<Steps, String> {
    let (sel, at, _) = here(s)?;
    let (from, end) = (sel.start.col, sel.end.col);
    let (before, shift): (u32, i64) = if left {
        (from.checked_sub(1).ok_or("These columns are already at the left edge.")?, -1)
    } else {
        if end + 1 >= MAX_COLS {
            return Err("These columns are already at the right edge.".into());
        }
        (end + 2, 1)
    };
    let moved = |c: CellRef| CellRef::new(c.row, (i64::from(c.col) + shift) as u32);
    let after = RangeRef::new(moved(sel.start), moved(sel.end));
    Ok(vec![
        ("sheet.moveColumns", json!({"cols": RangeRef::cols(from, end).a1(), "before": before})),
        ("selection.set", json!({"range": after.a1(), "active": moved(at).a1()})),
    ])
}

/// A seed for Randomise range from the clock.
fn seed() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() as u64) ^ (((js_sys::Math::random() * 4_294_967_296.0) as u64) << 20)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64)
    }
}

/// Data › Randomise range: the selected rows in a random order (one undo step). Their contents
/// move; formatting stays where it is. Refused where a row holds something that can't be written
/// back as typed (a spilled result, a picture, a Python cell or an error typed as a value).
fn randomise(s: &Session, seed: u64) -> Result<Steps, String> {
    let (_, _, sh) = here(s)?;
    let r = used_part(s).filter(|r| r.height() > 1).ok_or("Select at least two rows with something in them to randomise.")?;
    let mut rows: Vec<Vec<Json>> = Vec::with_capacity(r.height() as usize);
    for row in r.start.row..=r.end.row {
        let mut out = Vec::with_capacity(r.width() as usize);
        for col in r.start.col..=r.end.col {
            let c = CellRef::new(row, col);
            if sh.spill_anchor(c).is_some() || sh.cell_pictures.contains_key(&c) {
                return Err("Randomise range can't move results that spill from a formula or pictures in cells.".into());
            }
            let v = match sh.cell(c) {
                None => Json::Null,
                Some(cell) if cell.py.is_some() => return Err("Randomise range can't move Python cells.".into()),
                Some(cell) if cell.formula.is_some() => json!(cell.input_text()),
                Some(cell) => match &cell.value {
                    Value::Empty => Json::Null,
                    Value::Number(n) => json!(n),
                    Value::Bool(b) => json!(b),
                    Value::Text(t) if !t.starts_with('=') => json!(t.as_ref()),
                    _ => return Err("Randomise range can't move this range: it holds a typed error or text starting with =.".into()),
                },
            };
            out.push(v);
        }
        rows.push(out);
    }
    // Fisher-Yates with a small xorshift generator: no extra crate for one shuffle.
    let mut x = seed | 1;
    for i in (1..rows.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        rows.swap(i, (x % (i as u64 + 1)) as usize);
    }
    Ok(vec![("range.setValues", json!({"range": r.start.a1(), "values": rows, "parse": false}))])
}

/// Sheets' whitespace clean-up: no spaces at the start or end, and one space where there were
/// several. Line breaks are kept.
pub fn trimmed(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let mut gap = false;
        for ch in line.trim_matches(|c: char| c == ' ' || c == '\t' || c == '\u{a0}').chars() {
            if ch == ' ' || ch == '\t' || ch == '\u{a0}' {
                gap = true;
            } else {
                if gap {
                    out.push(' ');
                    gap = false;
                }
                out.push(ch);
            }
        }
    }
    out
}

/// Data › Data clean-up › Trim whitespace: text typed into the selected cells (not formulas).
fn trim_whitespace(s: &Session) -> Result<Steps, String> {
    let (_, _, sh) = here(s)?;
    let Some(r) = used_part(s) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for row in r.start.row..=r.end.row {
        for col in r.start.col..=r.end.col {
            let c = CellRef::new(row, col);
            if let Some(cell) = sh.cell(c)
                && cell.formula.is_none()
                && cell.py.is_none()
                && let Value::Text(t) = &cell.value
            {
                let new = trimmed(t);
                if new != t.as_ref() && !new.starts_with('=') {
                    out.push(("range.setValues", json!({"range": c.a1(), "values": [[new]], "parse": false})));
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(cells: &[(&str, &str)]) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        for (cell, v) in cells {
            s.execute("cell.set", json!({"cell": cell, "input": v})).unwrap();
        }
        s
    }

    fn run(s: &mut Session, id: &str, p: Json) {
        for (cmd, params) in steps(s, id, &p).unwrap() {
            s.execute(cmd, params).unwrap();
        }
    }

    fn input(s: &Session, a1: &str) -> String {
        s.active().unwrap().wb.active().unwrap().input_text(CellRef::parse(a1).unwrap())
    }

    fn select(s: &mut Session, range: &str) {
        s.execute("selection.set", json!({"range": range})).unwrap();
    }

    #[test]
    fn sort_sheet_sorts_below_the_frozen_rows_by_the_active_column() {
        let mut s = sheet(&[("A1", "Name"), ("B1", "Age"), ("A2", "Cy"), ("B2", "30"), ("A3", "Al"), ("B3", "10"), ("A4", "Bo"), ("B4", "20")]);
        s.execute("view.freezeTopRow", json!({})).unwrap();
        select(&mut s, "B2");
        run(&mut s, "panel:sortSheet", json!({}));
        assert_eq!([input(&s, "A1"), input(&s, "A2"), input(&s, "A3"), input(&s, "A4")], ["Name", "Al", "Bo", "Cy"]);
        run(&mut s, "panel:sortSheet", json!({"desc": true}));
        assert_eq!([input(&s, "B2"), input(&s, "B3"), input(&s, "B4")], ["30", "20", "10"]);
        assert_eq!(label(&s, "Sort sheet by column {col} (A to Z)"), "Sort sheet by column B (A to Z)");
    }

    #[test]
    fn freeze_counts_and_up_to_the_active_cell_keep_the_other_direction() {
        let mut s = sheet(&[]);
        let frozen = |s: &Session| s.active().unwrap().wb.active().unwrap().freeze;
        run(&mut s, "panel:freeze", json!({"rows": 2}));
        assert_eq!(frozen(&s), Some((2, 0)));
        run(&mut s, "panel:freeze", json!({"cols": 2}));
        assert_eq!(frozen(&s), Some((2, 2)));
        select(&mut s, "C5");
        run(&mut s, "panel:freeze", json!({"rows": "current"}));
        assert_eq!(frozen(&s), Some((5, 2)));
        assert_eq!(label(&s, "Up to row {row}"), "Up to row 5");
        run(&mut s, "panel:freeze", json!({"rows": 0}));
        assert_eq!(frozen(&s), Some((0, 2)));
        run(&mut s, "panel:freeze", json!({"cols": 0}));
        assert!(frozen(&s).is_none_or(|f| f == (0, 0)), "{:?}", frozen(&s));
    }

    #[test]
    fn move_column_left_and_right_swaps_with_the_neighbour() {
        let mut s = sheet(&[("A1", "a"), ("B1", "b"), ("C1", "c")]);
        select(&mut s, "B1");
        assert_eq!(enabled(&s, "panel:moveColumnLeft"), Some(true));
        run(&mut s, "panel:moveColumnLeft", json!({}));
        assert_eq!([input(&s, "A1"), input(&s, "B1"), input(&s, "C1")], ["b", "a", "c"]);
        assert_eq!(s.active().unwrap().selection.active, CellRef::parse("A1").unwrap());
        assert_eq!(enabled(&s, "panel:moveColumnLeft"), Some(false), "column A can't move left");
        run(&mut s, "panel:moveColumnRight", json!({}));
        run(&mut s, "panel:moveColumnRight", json!({}));
        assert_eq!([input(&s, "A1"), input(&s, "B1"), input(&s, "C1")], ["a", "c", "b"]);
        assert_eq!(s.active().unwrap().selection.active, CellRef::parse("C1").unwrap());
    }

    #[test]
    fn randomise_keeps_every_row_together_in_one_undo_step() {
        let mut s = sheet(&[("A1", "1"), ("B1", "one"), ("A2", "2"), ("B2", "two"), ("A3", "3"), ("B3", "two  words"), ("A4", "=A1*10")]);
        select(&mut s, "A1:B4");
        let before: Vec<String> = (1..=4).map(|r| format!("{}|{}", input(&s, &format!("A{r}")), input(&s, &format!("B{r}")))).collect();
        let mut changed = false;
        for seed in 1..20 {
            run(&mut s, "panel:randomise", json!({"seed": seed}));
            let mut now: Vec<String> = (1..=4).map(|r| format!("{}|{}", input(&s, &format!("A{r}")), input(&s, &format!("B{r}")))).collect();
            changed |= now != before;
            now.sort();
            let mut want = before.clone();
            want.sort();
            assert_eq!(now, want, "the same rows, in some order");
            s.execute("edit.undo", json!({})).unwrap();
            assert_eq!((1..=4).map(|r| input(&s, &format!("A{r}"))).collect::<Vec<_>>(), ["1", "2", "3", "=A1*10"], "one undo step");
        }
        assert!(changed, "some seed changes the order");
        select(&mut s, "A1:B1");
        assert_eq!(enabled(&s, "panel:randomise"), Some(false), "one row can't be randomised");
    }

    #[test]
    fn trim_whitespace_cleans_typed_text_only() {
        let mut s = sheet(&[("A3", "=\"  kept  \"")]);
        s.execute("range.setValues", json!({"range": "A1", "values": [["  two   spaces "], ["fine"]], "parse": false})).unwrap();
        select(&mut s, "A1:A4");
        let todo = steps(&s, "panel:trimWhitespace", &json!({})).unwrap();
        assert_eq!(todo.len(), 1, "only A1 is typed text with extra spaces: {todo:?}");
        run(&mut s, "panel:trimWhitespace", json!({}));
        assert_eq!(input(&s, "A1"), "two spaces");
        assert_eq!(input(&s, "A3"), "=\"  kept  \"");
        assert_eq!(trimmed("  a \t b\n  c  "), "a b\nc");
    }

    #[test]
    fn trim_keeps_text_that_looks_like_a_number_as_text() {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        s.execute("range.setValues", json!({"range": "A1", "values": [[" 007 "]], "parse": false})).unwrap();
        select(&mut s, "A1");
        run(&mut s, "panel:trimWhitespace", json!({}));
        let v = s.active().unwrap().wb.active().unwrap().value(CellRef::parse("A1").unwrap());
        assert_eq!(v, Value::text("007"));
    }
}
