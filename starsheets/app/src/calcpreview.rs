//! Google Sheets' calculation preview: while a formula is being typed, a small bubble over the
//! cell shows what it gives so far: of the whole formula, or of the selected part of it. An
//! unfinished formula is closed off first (open brackets and quotes closed, a dangling operator
//! dropped). The × hides it until the edit ends.

use egui::{Align2, Color32, RichText, vec2};
use gridcraft_engine::core::Value;
use gridcraft_ui_egui::SheetApp;

#[derive(Default)]
pub struct CalcPreview {
    /// (text evaluated, shown result) for the current edit.
    cache: Option<(String, Option<String>)>,
    hidden_for: Option<(u64, gridcraft_engine::core::CellRef)>,
}

impl CalcPreview {
    pub fn ui(&mut self, app: &SheetApp, ui: &mut egui::Ui) {
        let Some(ed) = app.editor.as_ref() else {
            self.cache = None;
            self.hidden_for = None;
            return;
        };
        if !ed.text.starts_with('=') || self.hidden_for == Some((ed.doc, ed.cell)) {
            return;
        }
        // The selected part, when there is one inside the formula; else everything after `=`.
        let chars: Vec<char> = ed.text.chars().collect();
        let (lo, hi) = (ed.caret.min(ed.anchor), ed.caret.max(ed.anchor).min(chars.len()));
        let part: String = if hi > lo && lo >= 1 { chars[lo..hi].iter().collect() } else { chars[1..].iter().collect() };
        if self.cache.as_ref().is_none_or(|(k, _)| *k != part) {
            let shown = app.session.active().and_then(|d| {
                let body = complete(&part)?;
                gridcraft_engine::formula::parse(&body).ok()?;
                let v = gridcraft_engine::calc::evaluate(&d.wb, ed.sheet, ed.cell, &body);
                // A half-typed name (`=SUM(` → `SUM`) gives #NAME?: Sheets shows nothing then.
                (v != Value::Error(gridcraft_engine::core::CellError::Name)).then(|| show(&v))
            });
            self.cache = Some((part, shown));
        }
        let Some((_, Some(text))) = self.cache.clone() else { return };
        let Some(cell) = crate::linkcard::cell_rect(app, ed.cell) else { return };
        let mut close = false;
        egui::Area::new(egui::Id::unique("starsheets_calc_preview"))
            .fixed_pos(cell.left_top() + vec2(cell.width().min(160.0) * 0.6, -4.0))
            .pivot(Align2::LEFT_BOTTOM)
            .order(egui::Order::Tooltip)
            .show(ui.ctx(), |ui| {
                crate::house::bubble_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&text).size(14.0).color(Color32::from_rgb(0x1A, 0x73, 0xE8)));
                        if ui
                            .add(egui::Button::new(RichText::new("×").size(14.0).color(Color32::from_gray(0x80))).frame(false))
                            .on_hover_text("Hide")
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });
            });
        if close {
            self.hidden_for = Some((ed.doc, ed.cell));
        }
    }
}

/// An unfinished formula made whole enough to work out: a dangling operator or comma dropped,
/// an open quote and open brackets closed. None when there's nothing to work out yet.
pub fn complete(part: &str) -> Option<String> {
    let mut s = part.trim_end().to_string();
    while let Some(c) = s.chars().last() {
        if "+-*/^&=<>,(".contains(c) {
            s.pop();
            s = s.trim_end().to_string();
        } else {
            break;
        }
    }
    if s.trim().is_empty() {
        return None;
    }
    let mut depth = 0i32;
    let mut in_text = false;
    for c in s.chars() {
        match c {
            '"' => in_text = !in_text,
            '(' if !in_text => depth += 1,
            ')' if !in_text => depth -= 1,
            _ => {}
        }
    }
    if in_text {
        s.push('"');
    }
    if depth < 0 {
        return None;
    }
    s.push_str(&")".repeat(depth as usize));
    Some(s)
}

/// A result as the bubble shows it: the value, or the first value of an array with "…".
fn show(v: &Value) -> String {
    match v {
        Value::Array(a) => {
            let first = a.data.first().map(Value::display).unwrap_or_default();
            if a.data.len() > 1 { format!("{first} …") } else { first }
        }
        Value::Empty => "0".into(),
        v => v.display(),
    }
}

#[cfg(test)]
mod tests {
    use super::complete;

    #[test]
    fn unfinished_formulas_are_closed_off() {
        assert_eq!(complete("SUM(A1:A6)+(22+22)*").as_deref(), Some("SUM(A1:A6)+(22+22)"));
        assert_eq!(complete("(SUM(A1:A6").as_deref(), Some("(SUM(A1:A6))"));
        assert_eq!(complete("CONCAT(\"ab").as_deref(), Some("CONCAT(\"ab\")"));
        assert_eq!(complete("SUM(").as_deref(), Some("SUM"));
        assert_eq!(complete("  "), None);
    }
}
