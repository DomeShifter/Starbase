//! Hovering over a cell that shows an error (#VALUE!, #DIV/0! …) explains it in plain words, as
//! Sheets does, beside the cell. When the formula uses a cell that already shows an error, it
//! says which: fixing that one usually fixes this one too.

use egui::{Align2, RichText, vec2};
use gridcraft_engine::core::{CellError, CellRef, Value};
use gridcraft_ui_egui::SheetApp;

use crate::house;
use crate::theme;

/// What an error means, without maths words.
pub fn explain(e: &CellError) -> &'static str {
    match e {
        CellError::Div0 => "This formula divides by zero, or by a cell that is empty or 0.",
        CellError::Value => {
            "Something in this formula is the wrong kind of value, such as text where a number is needed (for example =\"apple\"+1), or a date that can't be read."
        }
        CellError::Ref => "This formula points to a cell that isn't there any more: a row, column or sheet it used was deleted.",
        CellError::Name => {
            "This formula uses a word Starbase Sheets doesn't know: a function name may be misspelt, a named range may not exist, or text may be missing its quotation marks."
        }
        CellError::NA => "A value isn't available: a lookup (such as VLOOKUP, XLOOKUP or MATCH) didn't find what it was looking for.",
        CellError::Num => {
            "A number here is impossible or too big, such as the square root of a negative number, or a calculation that never settles."
        }
        CellError::Null => {
            "Two ranges in this formula were meant to overlap but don't. A space between two ranges means \"where they cross\"; use a comma to list them instead."
        }
        CellError::Spill => {
            "The result needs more cells than are free. Clear the cells it would spill into (below and to the right) and it will appear."
        }
        CellError::Calc => "The calculation couldn't give a result, for example FILTER found nothing to show, or a result was empty.",
        CellError::Circ => "This formula depends on its own result (a circular reference): it uses this cell, directly or through other cells.",
        CellError::GettingData => "Still working this out…",
        CellError::Python => "The Python code in this cell stopped with an error.",
        CellError::Busy => "This cell's code is waiting to run.",
        CellError::Timeout => "This cell's code took too long and was stopped.",
        CellError::Blocked => "This cell's code can't run here.",
    }
}

/// Cells (on the same sheet) named in a formula's text, in order, without repeats.
fn cells_in(formula: &str) -> Vec<CellRef> {
    let mut out = Vec::new();
    let b = formula.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    while i < b.len() {
        let ch = b[i];
        if ch == b'"' {
            in_string = !in_string;
        }
        // A reference starts at a letter (or $) not preceded by a letter, digit, '!' or '.'.
        let starts = !in_string
            && (ch.is_ascii_alphabetic() || ch == b'$')
            && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || matches!(b[i - 1], b'!' | b'.' | b'_')));
        if starts {
            let mut j = i;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'$') {
                j += 1;
            }
            let word = &formula[i..j];
            // Not a function name (followed by "(") nor a sheet name (followed by "!").
            if b.get(j) != Some(&b'(')
                && b.get(j) != Some(&b'!')
                && let Some(c) = CellRef::parse(&word.replace('$', ""))
                && !out.contains(&c)
            {
                out.push(c);
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

#[derive(Default)]
pub struct ErrorTip;

impl ErrorTip {
    pub fn ui(&mut self, app: &SheetApp, ctx: &egui::Context) {
        if app.editor.is_some() || ctx.dragged_id().is_some() {
            return;
        }
        let Some(cell) = app.grid.hover_cell else { return };
        let Some(d) = app.session.active() else { return };
        let Some(sh) = d.wb.active() else { return };
        // A note shows on hover itself, beside the cell: the error then shows above the cell, so
        // both can be read (to-do 2 row 21).
        let has_note = sh.comments.contains_key(&cell);
        let Value::Error(e) = sh.value(cell) else { return };
        // A picture in a cell keeps #VALUE! only as a fallback for formulas: not an error.
        if sh.cell_pictures.contains_key(&cell) {
            return;
        }
        let mut text = explain(&e).to_string();
        if let Some(f) = sh.cell(cell).and_then(|c| c.formula.as_ref()) {
            let broken: Vec<String> =
                cells_in(&f.text).into_iter().filter(|c| *c != cell && matches!(sh.value(*c), Value::Error(_))).map(|c| c.a1()).take(3).collect();
            if !broken.is_empty() {
                text = format!(
                    "This formula uses {} {}, which already {} an error. Fix that first and this cell may sort itself out.\n\n{}",
                    if broken.len() == 1 { "cell" } else { "cells" },
                    broken.join(", "),
                    if broken.len() == 1 { "shows" } else { "show" },
                    text
                );
            }
        }
        let Some(rect) = crate::linkcard::cell_rect(app, cell) else { return };
        if app.grid.cells_rect.is_some_and(|r| !r.contains(rect.center())) {
            return;
        }
        let (at, pivot) =
            if has_note { (rect.left_top() - vec2(0.0, 6.0), Align2::LEFT_BOTTOM) } else { (rect.right_top() + vec2(6.0, 0.0), Align2::LEFT_TOP) };
        egui::Area::new(egui::Id::unique("starsheets_error_tip")).fixed_pos(at).pivot(pivot).order(egui::Order::Tooltip).interactable(false).show(
            ctx,
            |ui| {
                house::card_frame().inner_margin(egui::Margin::same(house::M as i8)).show(ui, |ui| {
                    ui.set_max_width(300.0);
                    ui.label(RichText::new("Error").size(house::BODY).strong().color(house::ERROR));
                    ui.add_space(house::XS);
                    ui.add(egui::Label::new(RichText::new(text).size(house::BODY).color(theme::TEXT)).wrap());
                });
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_cells_a_formula_uses() {
        let got: Vec<String> = cells_in("=SUM(A1,B$2)+C3*Sheet2!D4+\"E5\"+LOG10(F6)+a1").iter().map(|c| c.a1()).collect();
        assert_eq!(got, ["A1", "B2", "C3", "F6"]);
    }

    #[test]
    fn every_error_has_words() {
        for e in
            [CellError::Div0, CellError::Value, CellError::Ref, CellError::Name, CellError::NA, CellError::Num, CellError::Null, CellError::Spill]
        {
            assert!(explain(&e).len() > 20);
        }
    }
}
