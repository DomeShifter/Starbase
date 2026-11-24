//! Merging cells the Google Sheets way: Merge all, Merge vertically, Merge horizontally and
//! Unmerge, each greyed out when it can't apply to the selection, and Google's "Heads up!" warning
//! when merging would drop values (only the top-left one of each merged block is kept), with
//! "Don't show this again for 5 minutes".

use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

/// Asks for a merge command, through egui's memory (menus and toolbar set it).
pub const REQUEST: &str = "starsheets_merge";

#[derive(Default)]
pub struct MergeGuard {
    pending: Option<String>,
    dont_ask: bool,
    /// When (in seconds, egui's clock) the warning may show again.
    quiet_until: f64,
}

/// Whether a merge command can do anything to the selection (as Sheets greys out the rest).
pub fn enabled(app: &SheetApp, cmd: &str) -> bool {
    let Some(d) = app.session.active() else { return false };
    let ranges = &d.selection.ranges;
    match cmd {
        "home.unmergeCells" => d.wb.active().is_some_and(|sh| sh.merges.iter().any(|m| ranges.iter().any(|r| m.intersects(r)))),
        "home.mergeDown" => ranges.iter().any(|r| r.end.row > r.start.row),
        "home.mergeAcross" => ranges.iter().any(|r| r.end.col > r.start.col),
        _ => ranges.iter().any(|r| !r.is_single()),
    }
}

/// The blocks a merge command makes from the selection.
fn blocks(r: RangeRef, cmd: &str) -> Vec<RangeRef> {
    match cmd {
        "home.mergeDown" => (r.start.col..=r.end.col).map(|c| RangeRef::new(CellRef::new(r.start.row, c), CellRef::new(r.end.row, c))).collect(),
        "home.mergeAcross" => (r.start.row..=r.end.row).map(|w| RangeRef::new(CellRef::new(w, r.start.col), CellRef::new(w, r.end.col))).collect(),
        _ => vec![r],
    }
}

/// The filled cells of a block (a value or a formula).
fn filled(sh: &gridcraft_engine::model::Sheet, b: RangeRef) -> Vec<CellRef> {
    sh.cells.iter_range(b).filter(|(_, cell)| cell.formula.is_some() || !cell.value.is_empty()).map(|(c, _)| c).collect()
}

/// Whether merging would drop a value: some block has more than one filled cell. A block with a
/// single value keeps it, wherever it is (Sheets moves it to the merged cell).
fn loses_data(app: &SheetApp, cmd: &str) -> bool {
    let Some(d) = app.session.active() else { return false };
    let Some(sh) = d.wb.active() else { return false };
    d.selection.ranges.iter().flat_map(|r| blocks(*r, cmd)).any(|b| filled(sh, b).len() > 1)
}

/// Merges the selection the Sheets way: a block whose only value isn't in its top-left cell has
/// that value moved there first, so the merged cell shows it.
fn merge(app: &mut SheetApp, cmd: &str) {
    let moves: Vec<(CellRef, CellRef)> = app
        .session
        .active()
        .and_then(|d| {
            let sh = d.wb.active()?;
            Some(
                d.selection
                    .ranges
                    .iter()
                    .flat_map(|r| blocks(*r, cmd))
                    .filter_map(|b| match filled(sh, b).as_slice() {
                        [only] if *only != b.start => Some((*only, b.start)),
                        _ => None,
                    })
                    .collect(),
            )
        })
        .unwrap_or_default();
    // One command (patch 0046), so one Undo also puts the value back where it was (to-do row 65).
    let moves: Vec<[String; 2]> = moves.into_iter().map(|(from, to)| [from.a1(), to.a1()]).collect();
    app.run_or_alert(cmd, if moves.is_empty() { json!({}) } else { json!({ "moves": moves }) });
}

impl MergeGuard {
    pub fn ui(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        if let Some(cmd) = ctx.data_mut(|m| m.remove_temp::<String>(egui::Id::unique(REQUEST))) {
            let now = ctx.input(|i| i.time);
            if cmd == "home.unmergeCells" {
                app.run_or_alert(&cmd, json!({}));
            } else if now >= self.quiet_until && loses_data(app, &cmd) {
                self.pending = Some(cmd);
                self.dont_ask = false;
            } else {
                merge(app, &cmd);
            }
        }
        let Some(cmd) = self.pending.clone() else { return };
        let mut answer = None;
        let mut closed = false;
        crate::house::dialog(ctx, "starsheets_merge_warning", 460.0, "Heads up!", Some(&mut closed), |ui| {
            crate::house::text(ui, "Merging cells will only preserve the top-leftmost value. Merge anyway?");
            ui.checkbox(&mut self.dont_ask, "Don't show this again for 5 minutes");
            match crate::house::actions(ui, &["Cancel", "OK"]) {
                Some(0) => answer = Some(false),
                Some(_) => answer = Some(true),
                None => {}
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                answer = Some(false);
            }
        });
        if closed {
            answer = Some(false);
        }
        match answer {
            Some(true) => {
                if self.dont_ask {
                    self.quiet_until = ctx.input(|i| i.time) + 300.0;
                }
                self.pending = None;
                merge(app, &cmd);
            }
            Some(false) => self.pending = None,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use gridcraft_engine::Session;
    use gridcraft_engine::core::{CellRef, RangeRef, Value};
    use serde_json::json;

    #[test]
    fn a_single_value_moves_to_the_merged_cell() {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        s.execute("cell.set", json!({ "cell": "B1", "input": "kept" })).unwrap();
        // What merge() sends for A1:B1 with only B1 filled.
        s.execute("home.mergeCells", json!({ "range": "A1:B1", "moves": [["B1", "A1"]] })).unwrap();
        let sh = s.active().unwrap().wb.active().unwrap();
        assert_eq!(sh.value(CellRef::parse("A1").unwrap()), Value::text("kept"));
        assert_eq!(sh.merges.len(), 1);
    }

    #[test]
    fn one_undo_puts_a_moved_value_back() {
        // To-do row 65: undoing the merge left the value in the top-left cell.
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        s.execute("cell.set", json!({ "cell": "B2", "input": "kept" })).unwrap();
        s.execute("home.mergeCells", json!({ "range": "A1:B2", "moves": [["B2", "A1"]] })).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        let sh = s.active().unwrap().wb.active().unwrap();
        assert_eq!(sh.value(CellRef::parse("B2").unwrap()), Value::text("kept"));
        assert_eq!(sh.value(CellRef::parse("A1").unwrap()), Value::Empty);
        assert!(sh.merges.is_empty());
    }

    #[test]
    fn merge_vertically_makes_one_block_per_column() {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        for (c, v) in [("A1", "1"), ("A2", "2"), ("B1", "3"), ("B2", "4")] {
            s.execute("cell.set", json!({"cell": c, "input": v})).unwrap();
        }
        s.execute("home.mergeDown", json!({"range": "A1:B2"})).unwrap();
        let sh = s.active().unwrap().wb.active().unwrap().clone();
        let mut merges: Vec<String> = sh.merges.iter().map(RangeRef::a1).collect();
        merges.sort();
        assert_eq!(merges, ["A1:A2", "B1:B2"]);
        assert_eq!(sh.value(CellRef::parse("B1").unwrap()), Value::number(3.0));
        assert_eq!(sh.value(CellRef::parse("A2").unwrap()), Value::Empty);
    }
}
