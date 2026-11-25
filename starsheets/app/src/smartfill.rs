//! Smart Fill, as in Google Sheets: after you type a few entries in a column next to data
//! ("Ada" beside "Ada Lovelace", "Alan" beside "Alan Turing"), the rest of the column shows the
//! suggested values in grey and a small card beside the next cell offers to fill them. Accept (or Ctrl+Shift+Y) fills it as one
//! undo step; ✕ hides it for that column. Tools › Suggestion controls turns the card off.
//! The pattern is GridCraft's Flash Fill (`fill::flash_fill_preview`, StarSheets 0097).

use egui::{Align2, Button, RichText, vec2};
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

use crate::{house, theme};

/// The `localStorage` key that remembers "suggestions off".
#[cfg(target_arch = "wasm32")]
const OFF_KEY: &str = "starsheets.smartFillOff";
/// Examples needed above the cell before a suggestion shows, as Sheets waits for a pattern.
const EXAMPLES: u32 = 2;
/// How many filled values the card shows.
const SHOWN: usize = 3;
/// How many suggested values are previewed in their cells (the rest fill on Accept).
const PREVIEW_CELLS: usize = 200;
/// The preview's grey (suggested text, as Google greys it).
const PREVIEW: egui::Color32 = theme::BOX;

/// What Smart Fill would write, for the active cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    /// The cell the suggestion was made for (the active cell).
    pub at: CellRef,
    /// Each empty cell of the column and its new text, top to bottom.
    pub cells: Vec<(CellRef, String)>,
}

/// The suggestion for the active cell, if it is an empty cell under at least two typed entries
/// with data to their left, and a pattern fits them all.
pub fn suggestion(app: &SheetApp) -> Option<Suggestion> {
    let d = app.session.active()?;
    let sheet = d.wb.active_sheet();
    let sh = d.wb.active()?;
    let at = d.selection.active;
    if at.col == 0 || !d.selection.is_single_cell() || !sh.value(at).is_empty() || at.row < EXAMPLES {
        return None;
    }
    let left_full = |row: u32| !sh.value(CellRef::new(row, at.col - 1)).is_empty();
    if !left_full(at.row) || (1..=EXAMPLES).any(|k| sh.value(CellRef::new(at.row - k, at.col)).is_empty() || !left_full(at.row - k)) {
        return None;
    }
    let cells = gridcraft_engine::fill::flash_fill_preview(&d.wb, sheet, at);
    (!cells.is_empty()).then_some(Suggestion { at, cells })
}

/// Fills the column as the suggestion (or Flash Fill) says, for the active cell.
pub fn accept(app: &mut SheetApp) {
    let Some(at) = app.session.active().map(|d| d.selection.active) else { return };
    app.run_or_alert("data.flashFill", json!({ "range": at.a1() }));
}

/// Whether the suggestion card is turned off (Tools › Suggestion controls).
pub fn off() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        storage().and_then(|s| s.get_item(OFF_KEY).ok().flatten()).is_some_and(|v| v == "1")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        OFF.with(std::cell::Cell::get)
    }
}

/// Turns the suggestion card on or off, remembered in this browser.
pub fn set_off(off: bool) {
    #[cfg(target_arch = "wasm32")]
    if let Some(s) = storage() {
        let _ = s.set_item(OFF_KEY, if off { "1" } else { "0" });
    }
    #[cfg(not(target_arch = "wasm32"))]
    OFF.with(|c| c.set(off));
}

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The card's state between frames.
#[derive(Default)]
pub struct SmartFill {
    /// What the suggestion was worked out from (file, revision, sheet, cell), so it is worked
    /// out again only when one of them changes.
    key: Option<(u64, u64, usize, CellRef)>,
    shown: Option<Suggestion>,
    /// Columns closed with ✕ (sheet, column), until the file changes.
    dismissed: Vec<(usize, u32)>,
    file: Option<u64>,
}

impl SmartFill {
    pub fn ui(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        let Some(d) = app.session.active() else { return };
        let key = (d.uid, d.revision, d.wb.active_sheet(), d.selection.active);
        if self.file != Some(d.uid) {
            self.file = Some(d.uid);
            self.dismissed.clear();
        }
        let sheet = key.2;
        if self.key != Some(key) {
            self.key = Some(key);
            self.shown = if off() { None } else { suggestion(app) };
        }
        let Some(s) = self.shown.clone() else { return };
        if app.editor.is_some() || app.dialog.is_some() || self.dismissed.contains(&(sheet, s.at.col)) {
            return;
        }
        let Some(rect) = crate::linkcard::cell_rect(app, s.at) else { return };
        if app.grid.cells_rect.is_some_and(|r| !r.contains(rect.left_top() + vec2(1.0, 1.0))) {
            return;
        }
        // Google's grey preview: each suggested value in its own cell, until accepted or hidden.
        if let Some(clip) = app.grid.cells_rect {
            let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::unique("starsheets_smart_fill_preview"));
            let painter = ctx.layer_painter(layer).with_clip_rect(clip);
            for (cell, text) in s.cells.iter().take(PREVIEW_CELLS) {
                let Some(r) = crate::linkcard::cell_rect(app, *cell) else { continue };
                if !clip.intersects(r) {
                    continue;
                }
                let font = egui::FontId::proportional((r.height() * 0.55).clamp(house::NOTE, house::BODY + 2.0));
                painter.with_clip_rect(r.intersect(clip)).text(r.left_center() + vec2(house::XS, 0.0), Align2::LEFT_CENTER, text, font, PREVIEW);
            }
        }
        let mut close = false;
        let mut fill = false;
        egui::Area::new(egui::Id::unique("starsheets_smart_fill"))
            .fixed_pos(rect.right_top() + vec2(house::S, 0.0))
            .pivot(Align2::LEFT_TOP)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                house::card_frame().show(ui, |ui| {
                    ui.set_max_width(300.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Smart Fill").size(house::LABEL).strong().color(theme::TEXT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            close = ui.add(Button::new(RichText::new("✕").size(14.0)).frame(false)).on_hover_text("Dismiss").clicked();
                        });
                    });
                    let n = s.cells.len();
                    house::note(ui, &format!("Fill {n} {} following the pattern:", if n == 1 { "cell" } else { "cells" }));
                    ui.add_space(house::XS);
                    for (_, text) in s.cells.iter().take(SHOWN) {
                        ui.label(RichText::new(text).size(house::BODY).color(theme::TEXT_DIM).italics());
                    }
                    if n > SHOWN {
                        house::note(ui, "…");
                    }
                    ui.add_space(house::S);
                    ui.horizontal(|ui| {
                        fill = ui.add(house::primary("Accept")).on_hover_text("Ctrl+Shift+Y").clicked();
                        house::note(ui, "Ctrl+Shift+Y");
                    });
                });
            });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.dismissed.push((sheet, s.at.col));
        }
        if fill {
            accept(app);
            self.shown = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(cells: &[(&str, &str)], at: &str) -> SheetApp {
        let mut app = SheetApp::new(gridcraft_engine::Session::new(), gridcraft_ui_egui::Services::default());
        app.session.execute("file.new", json!({})).unwrap();
        for (cell, v) in cells {
            app.session.execute("cell.set", json!({"cell": cell, "input": v})).unwrap();
        }
        app.session.execute("selection.set", json!({"range": at})).unwrap();
        app
    }

    #[test]
    fn suggests_after_two_examples_and_fills_the_rest() {
        let names = [("A1", "Ada Lovelace"), ("A2", "Alan Turing"), ("A3", "Grace Hopper"), ("A4", "Edsger Dijkstra")];
        let mut cells = names.to_vec();
        cells.extend([("B1", "Ada"), ("B2", "Alan")]);
        let mut a = app(&cells, "B3");
        let s = suggestion(&a).expect("a suggestion");
        assert_eq!(s.cells.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(), ["Grace", "Edsger"]);
        accept(&mut a);
        let sh = a.session.active().unwrap().wb.active().unwrap();
        assert_eq!(sh.input_text(CellRef::parse("B4").unwrap()), "Edsger");

        let one = app(&[names[0], names[1], ("B1", "Ada")], "B2");
        assert!(suggestion(&one).is_none(), "one example is not a pattern yet");
        let typed = app(&cells, "B2");
        assert!(suggestion(&typed).is_none(), "the active cell isn't empty");
        let no_left = app(&[("B1", "x"), ("B2", "y")], "B3");
        assert!(suggestion(&no_left).is_none(), "nothing to the left");
    }
}
