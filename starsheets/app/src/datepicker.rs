//! Sheets' date picker opens when editing a date-validation cell or double-clicking a cell
//! formatted as a date. Choosing a day writes that date to the cell.

use egui::{Align2, Color32, RichText, Sense, Ui, vec2};
use gridcraft_engine::core::{CellRef, Value, date::datetime_from_serial};
use gridcraft_engine::model::ValidationKind;
use gridcraft_engine::numfmt::{FormatKind, NumberFormat};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

use crate::cfpanel::date_number;
use crate::theme;

const BLUE: Color32 = Color32::from_rgb(0x1A, 0x73, 0xE8);
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const CELL: f32 = 36.0;
const MIN_YEAR: i64 = 1900;
const MAX_YEAR: i64 = 9999;
const YEAR_ROW: f32 = 28.0;
const YEAR_MENU_HEIGHT: f32 = YEAR_ROW * 8.0;

#[cfg(target_arch = "wasm32")]
const TEST_STATE: &str = "starsheets_date_picker_test_state";

type EditedCell = (u64, usize, CellRef);

#[derive(Default)]
pub struct DatePicker {
    /// The cell being edited and the month on show (year, month 1-12).
    shown: Option<(EditedCell, i64, i64)>,
    year_menu_open: bool,
}

impl DatePicker {
    /// Call after GridCraft's UI ran.
    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        let previously_shown = self.shown.take();
        let year_menu_open = std::mem::take(&mut self.year_menu_open);
        let Some(ed) = app.editor.as_ref() else { return };
        if app.view_key() != Some((ed.doc, ed.sheet)) {
            return;
        }
        let cell = ed.cell;
        let target = (ed.doc, ed.sheet, cell);
        let already_open = previously_shown.filter(|(key, ..)| *key == target);
        let Some(rect) = crate::linkcard::cell_rect(app, cell) else { return };
        if already_open.is_none() && !has_date_rule(app, cell) {
            let double_clicked = ui.input(|i| {
                i.pointer.button_double_clicked(egui::PointerButton::Primary) && i.pointer.interact_pos().is_some_and(|p| rect.contains(p))
            });
            if ed.from_formula_bar || !double_clicked || !has_date_format(app, cell) {
                return;
            }
        }
        let pos = rect.left_bottom();
        if app.grid.cells_rect.is_some_and(|r| !r.contains(pos - vec2(0.0, 6.0))) {
            return;
        }
        let today = today_serial();
        // The date in the cell (or being typed), else today.
        let current = cell_serial(app, cell);
        let (_, mut year, mut month) = already_open.unwrap_or_else(|| {
            let (year, month, _) = ymd(current.unwrap_or(today));
            (target, year.clamp(MIN_YEAR, MAX_YEAR), month)
        });
        let mut picked = None;
        egui::Area::new(egui::Id::unique("starsheets_date_picker"))
            .fixed_pos(pos + vec2(0.0, 4.0))
            .pivot(Align2::LEFT_TOP)
            .order(egui::Order::Foreground)
            .movable(false)
            .show(ui.ctx(), |ui| {
                crate::house::card_frame().show(ui, |ui| {
                    ui.set_width(CELL * 7.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        month_menu(ui, &mut month);
                        self.year_menu_open = year_menu(ui, &mut year, year_menu_open);
                        if month_arrow(ui, "‹", "Previous month", "date-picker:previous") {
                            (year, month) = shift_month(year, month, -1);
                        }
                        if month_arrow(ui, "›", "Next month", "date-picker:next") {
                            (year, month) = shift_month(year, month, 1);
                        }
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
                        let (y, m, d) = ymd(day);
                        let c = r.min + vec2(CELL * ((k % 7) as f32 + 0.5), CELL * ((k / 7) as f32 + 1.5));
                        let hit = egui::Rect::from_center_size(c, vec2(CELL, CELL));
                        let valid = (MIN_YEAR..=MAX_YEAR).contains(&y);
                        let resp = ui.interact(hit, ui.scope_id().with(("day", k)), if valid { Sense::click() } else { Sense::hover() });
                        crate::house::target(ui.ctx(), &format!("date-picker:day-{k}"), hit);
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
                        if valid && resp.clicked() {
                            picked = Some(day);
                        }
                    }
                    ui.add_space(4.0);
                    if ui.add(egui::Button::new(RichText::new("Today").size(crate::house::BODY).color(BLUE)).frame(false)).clicked() {
                        picked = Some(today);
                    }
                });
            });
        if let Some(day) = picked {
            let (y, m, d) = ymd(day);
            app.cancel_edit();
            app.run_or_alert("cell.set", json!({ "cell": cell.a1(), "input": format!("{y:04}-{m:02}-{d:02}") }));
        } else {
            self.shown = Some((target, year, month));
            #[cfg(target_arch = "wasm32")]
            {
                let frame = ui.ctx().cumulative_frame_nr();
                ui.ctx().data_mut(|data| data.insert_temp(egui::Id::unique(TEST_STATE), (frame, year, month)));
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn test_state(ctx: &egui::Context) -> Option<serde_json::Value> {
    let (frame, year, month) = ctx.data(|data| data.get_temp::<(u64, i64, i64)>(egui::Id::unique(TEST_STATE)))?;
    // A snapshot can be taken before the picker paints in the same frame.
    (frame + 1 >= ctx.cumulative_frame_nr()).then(|| json!({ "year": year, "month": month }))
}

fn month_menu(ui: &mut Ui, month: &mut i64) {
    let response = egui::ComboBox::from_id_salt("date_picker_month")
        .selected_text(MONTHS[(*month - 1) as usize])
        .width(99.0)
        .show_ui(ui, |ui| {
            for (i, name) in MONTHS.iter().enumerate() {
                let value = i as i64 + 1;
                let row = ui.selectable_value(month, value, *name);
                crate::house::target(ui.ctx(), &format!("date-picker:month-{value}"), row.rect);
            }
        })
        .response;
    crate::house::target(ui.ctx(), "date-picker:month", response.rect);
}

fn year_menu(ui: &mut Ui, year: &mut i64, was_open: bool) -> bool {
    let button = egui::Button::new(year.to_string()).right_text("▾");
    let response = ui.add_sized(vec2(58.0, ui.spacing().interact_size.y), button);
    crate::house::target(ui.ctx(), "date-picker:year", response.rect);
    egui::Popup::menu(&response)
        .id(egui::Id::unique("date_picker_year"))
        .show(|ui| {
            ui.set_min_width(response.rect.width());
            let mut scroll = egui::ScrollArea::vertical().max_height(YEAR_MENU_HEIGHT);
            if !was_open {
                let stride = YEAR_ROW + ui.spacing().item_spacing.y;
                scroll = scroll.vertical_scroll_offset(((*year - MIN_YEAR) as f32 * stride - YEAR_MENU_HEIGHT / 2.0).max(0.0));
            }
            scroll.show_rows(ui, YEAR_ROW, (MAX_YEAR - MIN_YEAR + 1) as usize, |ui, rows| {
                for index in rows {
                    let value = MIN_YEAR + index as i64;
                    let row = ui.add_sized(vec2(ui.available_width(), YEAR_ROW), egui::Button::selectable(*year == value, value.to_string()));
                    if row.rect.intersects(ui.clip_rect()) {
                        crate::house::target(ui.ctx(), &format!("date-picker:year-{value}"), row.rect);
                    }
                    if row.clicked() {
                        *year = value;
                        ui.close();
                    }
                }
            });
        })
        .is_some()
}

fn month_arrow(ui: &mut Ui, glyph: &str, label: &str, target: &str) -> bool {
    let button = egui::Button::new(RichText::new(glyph).size(20.0)).frame_when_inactive(false);
    let response = ui.add_sized(vec2(32.0, 32.0), button).on_hover_text(label);
    crate::house::target(ui.ctx(), target, response.rect);
    response.clicked()
}

fn shift_month(year: i64, month: i64, delta: i64) -> (i64, i64) {
    let index = (year * 12 + month - 1 + delta).clamp(MIN_YEAR * 12, MAX_YEAR * 12 + 11);
    (index / 12, index % 12 + 1)
}

/// Whether the cell's validation rule is about dates.
fn has_date_rule(app: &SheetApp, cell: CellRef) -> bool {
    app.session
        .active()
        .and_then(|d| d.wb.active())
        .and_then(|sh| sh.validations.iter().find(|v| v.ranges.iter().any(|r| r.contains(cell))))
        .is_some_and(|v| v.kind == ValidationKind::Date)
}

fn has_date_format(app: &SheetApp, cell: CellRef) -> bool {
    app.session
        .active()
        .and_then(|doc| {
            let style = doc.wb.active()?.style_id(cell);
            Some(NumberFormat::parse(doc.wb.styles.get(style).num_fmt.as_str()).kind() == FormatKind::Date)
        })
        .unwrap_or(false)
}

/// The date in the editor's text, else the cell's stored date.
fn cell_serial(app: &SheetApp, cell: CellRef) -> Option<i64> {
    let d = app.session.active()?;
    let calendar_day = |n| {
        let date = datetime_from_serial(d.wb.date_system, n)?;
        date_number(&format!("{}/{}/{}", date.day, date.month, date.year))
    };
    if let Some(ed) = &app.editor {
        let v = gridcraft_engine::core::parse::parse_input(&ed.text, d.wb.date_system).value;
        if let Some(day) = v.as_f64().and_then(calendar_day) {
            return Some(day);
        }
    }
    match d.wb.active()?.value(cell) {
        Value::Number(n) => calendar_day(n),
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

    #[test]
    fn month_navigation_carries_the_year_and_respects_calendar_limits() {
        assert_eq!(shift_month(2026, 12, 1), (2027, 1));
        assert_eq!(shift_month(2027, 1, -1), (2026, 12));
        assert_eq!(shift_month(MIN_YEAR, 1, -1), (MIN_YEAR, 1));
        assert_eq!(shift_month(MAX_YEAR, 12, 1), (MAX_YEAR, 12));
    }
}
