//! Sheets turns a typed or pasted web or e-mail address into a link. GridCraft doesn't, so we
//! watch for an edit that ends, or a paste, and run its `insert.link` on those cells.

use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

#[derive(Default)]
pub struct AutoLink {
    /// The cell being edited last frame: (document, sheet, cell).
    editing: Option<(u64, usize, CellRef)>,
    /// Frames left in which to check the selection after a paste (the pasted block is selected).
    after_paste: u8,
    /// Linked cells with text in (or being edited in) the selection last frame: (document, sheet, cells).
    linked: Option<(u64, usize, Vec<CellRef>)>,
}

/// The most links looked at per frame for the unlink check (a sheet with more skips it).
const LINK_SCAN_MAX: usize = 5_000;

/// The most cells one paste is checked over (a pasted whole column stays cheap).
const PASTE_SCAN_MAX: u64 = 20_000;

impl AutoLink {
    /// Call from the raw input hook: notes a paste so the next frames check what it filled.
    pub fn saw_input(&mut self, raw: &egui::RawInput) {
        if raw.events.iter().any(|e| matches!(e, egui::Event::Paste(_))) {
            self.after_paste = 3;
        }
    }

    /// Call once per frame, after GridCraft's UI ran.
    pub fn after_frame(&mut self, app: &mut SheetApp) {
        if self.after_paste > 0 && app.editor.is_none() {
            self.after_paste -= 1;
            link_pasted(app);
        }
        self.unlink_emptied(app);
        let now = app.editor.as_ref().map(|e| (e.doc, e.sheet, e.cell));
        if let Some((doc, sheet, cell)) = self.editing
            && now != self.editing
        {
            link_if_address(app, doc, sheet, cell);
        }
        self.editing = now;
    }
}

impl AutoLink {
    /// Sheets drops a cell's link, and its blue underline, when its text goes (Delete, Backspace, or
    /// editing it away). GridCraft keeps both on the empty cell, so: a linked cell with text in the selection that is empty
    /// a frame later loses its link. Only selected cells are watched, so a link on an empty cell
    /// from a file, or one emptied by someone else elsewhere, is left alone.
    fn unlink_emptied(&mut self, app: &mut SheetApp) {
        let editing = app.editor.as_ref().map(|e| e.cell);
        let now = app.session.active().and_then(|d| {
            let sh = d.wb.active()?;
            if sh.hyperlinks.len() > LINK_SCAN_MAX {
                return None;
            }
            let mut text = Vec::new();
            let mut emptied = Vec::new();
            let before = self.linked.as_ref().filter(|(doc, sheet, _)| *doc == d.uid && *sheet == d.wb.active_sheet());
            for &c in sh.hyperlinks.keys() {
                let has_text = sh.cell(c).is_some_and(|x| !x.value.is_empty() || x.formula.is_some());
                if has_text && (Some(c) == editing || d.selection.ranges.iter().any(|r| r.contains(c))) {
                    text.push(c);
                } else if !has_text && before.is_some_and(|(_, _, cells)| cells.contains(&c)) {
                    emptied.push(c);
                }
            }
            Some(((d.uid, d.wb.active_sheet(), text), emptied))
        });
        let Some((now, emptied)) = now else {
            self.linked = None;
            return;
        };
        self.linked = Some(now);
        for c in emptied {
            // One step: the link and its blue underline go together (patch 0053).
            app.run_or_alert("insert.link", json!({ "cell": c.a1(), "remove": true, "plain": true }));
        }
    }
}

/// The link target for a cell's text, when the whole text is one address.
pub fn address(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    let host_ok = |rest: &str| {
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        host.contains('.') && !host.starts_with('.') && !host.ends_with('.')
    };
    if let Some(rest) = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://")) {
        return host_ok(rest).then(|| t.to_string());
    }
    if lower.starts_with("www.") && host_ok(&lower) {
        return Some(format!("https://{t}"));
    }
    if let Some(rest) = lower.strip_prefix("mailto:") {
        return rest.contains('@').then(|| t.to_string());
    }
    None
}

/// Links every address in the selected block(s), within the used part of the sheet.
fn link_pasted(app: &mut SheetApp) {
    let found: Vec<(CellRef, String)> = app
        .session
        .active()
        .and_then(|d| {
            let sh = d.wb.active()?;
            let used = sh.used_range()?;
            let mut out = Vec::new();
            let mut seen = 0u64;
            for r in &d.selection.ranges {
                let (r0, r1) = (r.start.row.max(used.start.row), r.end.row.min(used.end.row));
                let (c0, c1) = (r.start.col.max(used.start.col), r.end.col.min(used.end.col));
                for row in r0..=r1 {
                    for col in c0..=c1 {
                        seen += 1;
                        if seen > PASTE_SCAN_MAX {
                            return Some(out);
                        }
                        let at = CellRef { row, col };
                        let Some(cell) = sh.cell(at) else { continue };
                        if cell.formula.is_some() || sh.hyperlinks.contains_key(&at) {
                            continue;
                        }
                        if let Some(t) = cell.value.as_text().and_then(address) {
                            out.push((at, t));
                        }
                    }
                }
            }
            Some(out)
        })
        .unwrap_or_default();
    for (at, target) in found {
        app.run_or_alert("insert.link", json!({ "cell": at.a1(), "target": target }));
    }
}

fn link_if_address(app: &mut SheetApp, doc: u64, sheet: usize, cell: CellRef) {
    // Only on the sheet still showing (the command works on the active sheet).
    let Some(target) = app.session.active().filter(|d| d.uid == doc && d.wb.active_sheet() == sheet).and_then(|d| {
        let sh = d.wb.active()?;
        if sh.hyperlinks.contains_key(&cell) || sh.cell(cell).is_some_and(|c| c.formula.is_some()) {
            return None;
        }
        address(sh.value(cell).as_text()?)
    }) else {
        return;
    };
    app.run_or_alert("insert.link", json!({ "cell": cell.a1(), "target": target }));
}

#[cfg(test)]
mod tests {
    use super::address;

    #[test]
    fn spots_addresses() {
        assert_eq!(address("https://example.com/a?b=1").as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(address(" http://x.org ").as_deref(), Some("http://x.org"));
        assert_eq!(address("www.bbc.co.uk").as_deref(), Some("https://www.bbc.co.uk"));
        assert_eq!(address("mailto:a@b.com").as_deref(), Some("mailto:a@b.com"));
        for no in ["hello", "http://", "https://localhost", "see www.x.com", "www.", "3.14", "mailto:nobody"] {
            assert_eq!(address(no), None, "{no}");
        }
    }
}

#[cfg(test)]
mod refit_tests {
    use gridcraft_engine::Session;
    use serde_json::json;

    /// Patch 0050 (to-do row 76): widening a column shrinks a wrapped row back to one line.
    #[test]
    fn widening_a_column_refits_wrapped_rows() {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        s.execute("cell.set", json!({ "cell": "B2", "input": "Please make drop-down lists only as wide as they need to be to fit the options" }))
            .unwrap();
        s.execute("home.wrapText", json!({ "range": "B2", "on": true })).unwrap();
        let h = |s: &Session| s.active().unwrap().wb.active().unwrap().row_height(1);
        let one_line = s.active().unwrap().wb.active().unwrap().default_row_height;
        assert!(h(&s) > one_line, "wraps in a 100 px column");
        s.execute("home.columnWidth", json!({ "cols": "B:B", "width": 900 })).unwrap();
        assert_eq!(h(&s), one_line, "fits on one line once the column is wide");
        s.execute("home.columnWidth", json!({ "cols": "B:B", "width": 100 })).unwrap();
        assert!(h(&s) > one_line, "grows again when narrowed");
    }

    /// Patch 0053 (to-do row 74): removing a link with `plain` also takes off its blue underline.
    #[test]
    fn removing_a_link_plainly_drops_its_look() {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        s.execute("insert.link", json!({ "cell": "A1", "target": "https://example.com" })).unwrap();
        s.execute("edit.clearContents", json!({ "range": "A1" })).unwrap();
        s.execute("insert.link", json!({ "cell": "A1", "remove": true, "plain": true })).unwrap();
        let d = s.active().unwrap();
        let sh = d.wb.active().unwrap();
        let a1 = gridcraft_engine::core::CellRef::new(0, 0);
        assert!(sh.hyperlinks.is_empty());
        let st = d.wb.styles.get(sh.style_id(a1));
        assert_eq!(st.font.underline, gridcraft_engine::model::Underline::None);
        assert_eq!(st.font.color, gridcraft_engine::model::Color::Auto);
    }
}

#[cfg(test)]
mod border_tests {
    use gridcraft_engine::Session;
    use gridcraft_engine::core::CellRef;
    use serde_json::json;

    /// Patch 0054 (to-do row 79): a border drawn on a shared edge replaces the neighbour's line.
    #[test]
    fn a_drawn_edge_replaces_the_neighbours_line() {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        s.execute("home.borders", json!({ "range": "B2", "preset": "right", "style": "thick", "color": "#FF0000" })).unwrap();
        s.execute("home.borders", json!({ "range": "C2", "preset": "left", "style": "thin" })).unwrap();
        let d = s.active().unwrap();
        let sh = d.wb.active().unwrap();
        let b2 = d.wb.styles.get(sh.style_id(CellRef::new(1, 1))).borders;
        let c2 = d.wb.styles.get(sh.style_id(CellRef::new(1, 2))).borders;
        assert!(b2.right.is_none(), "B2's thick red right edge gave way");
        assert!(!c2.left.is_none(), "C2's new thin left edge is the line now");
    }
}

#[cfg(test)]
mod tab_tests {
    use gridcraft_engine::Session;
    use serde_json::json;

    /// Patch 0056 (to-do 2 row 19): deleting the middle tab you're on goes to the tab on its left.
    #[test]
    fn deleting_the_middle_tab_goes_left() {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        s.execute("home.insertSheet", json!({})).unwrap();
        s.execute("home.insertSheet", json!({})).unwrap();
        let order: Vec<String> = s.active().unwrap().wb.sheets.iter().map(|x| x.name.clone()).collect();
        assert_eq!(order.len(), 3);
        s.execute("sheet.activate", json!({ "sheet": 1 })).unwrap();
        s.execute("home.deleteSheet", json!({})).unwrap();
        let d = s.active().unwrap();
        assert_eq!(d.wb.sheet(d.wb.active_sheet()).unwrap().name, order[0]);
    }
}
