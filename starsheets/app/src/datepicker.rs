//! Sheets' date picker: editing a cell that has a date validation rule (e.g. "Is valid date")
//! shows a month calendar under the cell. Clicking a day puts that date in the cell; "Today"
//! puts today's date in.

use egui::{Align2, Color32, RichText, Sense, Ui, vec2};
use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::model::ValidationKind;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

use crate::cfpanel::date_number;
use crate::theme;

const BLUE: Color32 = Color32::from_rgb(0x1A, 0x73, 0xE8);
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const CELL: f32 = 36.0;

#[derive(Default)]
pub struct DatePicker {
    /// The cell being edited and the month on show (year, month 1-12).
    shown: Option<(CellRef, i64, i64)>,
}

impl DatePicker {
    /// Call after GridCraft's UI ran.
    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        let Some(cell) = app.editor.as_ref().map(|e| e.cell).filter(|c| has_date_rule(app, *c)) else {
            self.shown = None;
            return;
        };
        let Some(pos) = crate::linkcard::cell_bottom_left(app, cell) else { return };
        if app.grid.cells_rect.is_some_and(|r| !r.contains(pos - vec2(0.0, 6.0))) {
            return;
        }
        let today = today_serial();
        // The date in the cell (or being typed), else today.
        let current = cell_serial(app, cell);
        if self.shown.is_none_or(|(c, ..)| c != cell) {
            let (y, m, _) = ymd(current.unwrap_or(today));
            self.shown = Some((cell, y, m));
        }
        let Some((_, mut year, mut month)) = self.shown else { return };
        let mut picked = None;
        egui::Area::new(egui::Id::unique("starsheets_date_picker"))
            .fixed_pos(pos + vec2(0.0, 4.0))
            .pivot(Align2::LEFT_TOP)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                crate::house::card_frame().show(ui, |ui| {
                    ui.set_width(CELL * 7.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{} {year}", MONTHS[(month - 1) as usize])).size(16.0).color(theme::TEXT));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new(RichText::new("›").size(20.0)).frame(false)).on_hover_text("Next month").clicked() {
                                (year, month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
                            }
                            if ui.add(egui::Button::new(RichText::new("‹").size(20.0)).frame(false)).on_hover_text("Previous month").clicked() {
                                (year, month) = if month == 1 { (year - 1, 12) } else { (year, month - 1) };
                            }
                        });
                    });
                    ui.add_space(6.0);
                    let (r, _) = ui.allocate_exact_size(vec2(CELL * 7.0, CELL * 7.0), Sense::hover());
                    let p = ui.painter().clone();
                    for (i, d) in ["M", "T", "W", "T", "F", "S", "S"].iter().enumerate() {
                        p.text(
                            r.min + vec2(CELL * (i as f32 + 0.5), CELL * 0.5),
                            Align2::CENTER_CENTER,
                            *d,
                            egui::FontId::proportional(13.0),
                            theme::TEXT,
                        );
                    }
                    // Monday on or before the 1st, then six weeks.
                    let first = serial(year, month, 1);
                    let start = first - weekday(first);
                    for k in 0..42 {
                        let day = start + k;
                        let (_, m, d) = ymd(day);
                        let c = r.min + vec2(CELL * ((k % 7) as f32 + 0.5), CELL * ((k / 7) as f32 + 1.5));
                        let hit = egui::Rect::from_center_size(c, vec2(CELL, CELL));
                        let resp = ui.interact(hit, ui.scope_id().with(("day", k)), Sense::click());
                        let chosen = current == Some(day);
                        if chosen {
                            p.circle_filled(c, 15.0, BLUE);
                        } else if resp.hovered() {
                            p.circle_filled(c, 15.0, theme::MENU_HOVER);
                        }
                        let colour = if chosen {
                            Color32::WHITE
                        } else if m != month {
                            Color32::from_gray(0xA0)
                        } else if day == today {
                            BLUE
                        } else {
                            theme::TEXT
                        };
                        p.text(c, Align2::CENTER_CENTER, d.to_string(), egui::FontId::proportional(14.0), colour);
                        if resp.clicked() {
                            picked = Some(day);
                        }
                    }
                    ui.add_space(4.0);
                    if ui.add(egui::Button::new(RichText::new("Today").size(crate::house::BODY).color(BLUE)).frame(false)).clicked() {
                        picked = Some(today);
                    }
                });
            });
        self.shown = Some((cell, year, month));
        if let Some(day) = picked {
            let (y, m, d) = ymd(day);
            app.cancel_edit();
            app.run_or_alert("cell.set", json!({ "cell": cell.a1(), "input": format!("{y:04}-{m:02}-{d:02}") }));
            self.shown = None;
        }
    }
}

/// Whether the cell's validation rule is about dates.
fn has_date_rule(app: &SheetApp, cell: CellRef) -> bool {
    app.session
        .active()
        .and_then(|d| d.wb.active())
        .and_then(|sh| sh.validations.iter().find(|v| v.ranges.iter().any(|r| r.contains(cell))))
        .is_some_and(|v| v.kind == ValidationKind::Date)
}

/// The date in the editor's text, else the cell's stored date.
fn cell_serial(app: &SheetApp, cell: CellRef) -> Option<i64> {
    let d = app.session.active()?;
    if let Some(ed) = &app.editor {
        let v = gridcraft_engine::core::parse::parse_input(&ed.text, d.wb.date_system).value;
        if let Some(n) = v.as_f64().filter(|n| *n >= 1.0) {
            return Some(n.floor() as i64);
        }
    }
    match d.wb.active()?.value(cell) {
        Value::Number(n) if n >= 1.0 => Some(n.floor() as i64),
        _ => None,
    }
}

#[cfg(target_arch = "wasm32")]
fn today_serial() -> i64 {
    // Local date: the browser's clock minus its time-zone offset.
    let ms = js_sys::Date::now() - js_sys::Date::new_0().get_timezone_offset() * 60_000.0;
    (ms / 86_400_000.0).floor() as i64 + 25_569
}

#[cfg(not(target_arch = "wasm32"))]
fn today_serial() -> i64 {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    (s / 86_400) as i64 + 25_569
}

fn serial(y: i64, m: i64, d: i64) -> i64 {
    date_number(&format!("{d}/{m}/{y}")).unwrap_or(0)
}

/// Monday = 0 … Sunday = 6 (serial 0 is Saturday 30 Dec 1899).
fn weekday(n: i64) -> i64 {
    (n + 5).rem_euclid(7)
}

/// A spreadsheet date number as (year, month, day).
pub(crate) fn ymd(n: i64) -> (i64, i64, i64) {
    // Civil from days (Howard Hinnant), from the 1899-12-30 epoch.
    let z = n - 25_569 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_maths() {
        let n = serial(2020, 10, 20);
        assert_eq!(ymd(n), (2020, 10, 20));
        assert_eq!(weekday(n), 1); // a Tuesday
        assert_eq!(weekday(serial(2020, 10, 1)), 3); // Thursday, as in Sheets' picture
        assert_eq!(ymd(serial(2024, 2, 29)), (2024, 2, 29));
    }
}
