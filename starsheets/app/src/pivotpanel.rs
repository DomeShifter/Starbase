//! The pivot table editor as Google Sheets has it: a side panel that shows while the active cell
//! is in a pivot table, with Rows, Columns, Values and Filters, each with "Add". Rows and columns
//! have an order and (for dates) a grouping; values have "Summarise by" and "Show as"; calculated
//! fields have a formula; filters filter by values (tick boxes) or by condition. Everything runs
//! GridCraft's `pivot.*` commands; GridCraft's own fields pane is hidden (StarSheets 0103).
//!
//! A condition filter ticks the items that match it now (`pivot.filter` keeps a list of items):
//! new data that would match later needs the condition applied again. Grouping by number ranges
//! isn't there yet (GridCraft groups dates only).

use egui::{RichText, Ui};
use gridcraft_ui_egui::SheetApp;
use serde_json::{Value as Json, json};

use crate::{house, theme};

const PANEL_W: f32 = 320.0;

/// Summarise by: label and GridCraft's name.
pub const FUNCS: &[(&str, &str)] = &[
    ("SUM", "sum"),
    ("COUNTA", "count"),
    ("COUNT", "countNumbers"),
    ("AVERAGE", "average"),
    ("MAX", "max"),
    ("MIN", "min"),
    ("PRODUCT", "product"),
    ("STDEV", "stdDev"),
    ("STDEVP", "stdDevP"),
    ("VAR", "var"),
    ("VARP", "varP"),
];
/// Show as.
pub const SHOW_AS: &[(&str, &str)] =
    &[("Default", "normal"), ("% of row", "percentOfRowTotal"), ("% of column", "percentOfColumnTotal"), ("% of grand total", "percentOfGrandTotal")];
const ORDERS: &[(&str, &str)] = &[("Ascending", "asc"), ("Descending", "desc")];
const GROUPS: &[(&str, &str)] = &[("No grouping", "none"), ("Year", "years"), ("Quarter", "quarters"), ("Month", "months"), ("Day", "days")];
/// Filter by condition: label and the test.
pub const CONDITIONS: &[&str] = &[
    "None",
    "Text contains",
    "Text does not contain",
    "Text starts with",
    "Text ends with",
    "Text is exactly",
    "Greater than",
    "Greater than or equal to",
    "Less than",
    "Less than or equal to",
    "Is equal to",
    "Is not equal to",
];

/// Whether `item` passes condition `op` (an index into [`CONDITIONS`]) with `value`.
pub fn passes(op: usize, item: &str, value: &str) -> bool {
    let (i, v) = (item.to_lowercase(), value.trim().to_lowercase());
    let num = |s: &str| s.trim().replace(',', "").parse::<f64>().ok();
    let cmp = || num(item).zip(num(value)).map(|(a, b)| a.partial_cmp(&b));
    match op {
        1 => i.contains(&v),
        2 => !i.contains(&v),
        3 => i.starts_with(&v),
        4 => i.ends_with(&v),
        5 => i == v,
        6 => cmp().flatten().is_some_and(|o| o.is_gt()),
        7 => cmp().flatten().is_some_and(|o| o.is_ge()),
        8 => cmp().flatten().is_some_and(|o| o.is_lt()),
        9 => cmp().flatten().is_some_and(|o| o.is_le()),
        10 => cmp().flatten().map_or(i == v, |o| o.is_eq()),
        11 => !cmp().flatten().map_or(i == v, |o| o.is_eq()),
        _ => true,
    }
}

/// A filter's condition being typed (field, condition, value).
#[derive(Default)]
struct Condition {
    field: String,
    op: usize,
    value: String,
}

#[derive(Default)]
pub struct PivotPanel {
    pub open: bool,
    pub just_opened: bool,
    /// Closed with ✕ while in this pivot (it opens again in another one).
    closed_for: Option<String>,
    /// What `pivot.fields` said, for (file, revision, pivot).
    info: Option<((u64, u64, String), Json)>,
    /// A calculated field being added or changed: (old name, name, formula).
    calc: Option<(Option<String>, String, String)>,
    condition: Condition,
    error: Option<String>,
}

impl PivotPanel {
    pub fn close(&mut self) {
        self.open = false;
        self.calc = None;
    }

    /// Opens while the active cell is in a pivot table, as Sheets' editor does.
    pub fn intercept(&mut self, app: &mut SheetApp, _ctx: &egui::Context) {
        app.pivot_pane = false;
        self.just_opened = false;
        let here = gridcraft_ui_egui::pivot_pane::active_pivot(app);
        match here {
            Some(name) if self.closed_for.as_deref() != Some(name.as_str()) => {
                if !self.open {
                    self.open = true;
                    self.just_opened = true;
                    self.error = None;
                }
            }
            Some(_) => {}
            None => {
                self.closed_for = None;
                if self.open {
                    self.close();
                }
            }
        }
    }

    fn info(&mut self, app: &mut SheetApp, name: &str) -> Option<Json> {
        let key = app.session.active().map(|d| (d.uid, d.revision, name.to_string()))?;
        if self.info.as_ref().is_none_or(|(k, _)| *k != key) {
            let v = app.session.run("pivot.fields", json!({ "pivot": name, "items": true })).ok()?;
            self.info = Some((key, v));
        }
        self.info.as_ref().map(|(_, v)| v.clone())
    }

    fn run(&mut self, app: &mut SheetApp, cmd: &str, p: Json) {
        match app.run(cmd, p) {
            Ok(_) => self.error = None,
            Err(e) => self.error = Some(crate::messages::plain_error(&gridcraft_ui_egui::clean_error(&e))),
        }
    }

    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        if !self.open {
            return;
        }
        let Some(name) = gridcraft_ui_egui::pivot_pane::active_pivot(app) else { return self.close() };
        let Some(info) = self.info(app, &name) else { return };
        egui::Panel::right("starsheets_pivot").exact_size(PANEL_W + 32.0 + house::SCROLL_ROOM).resizable(false).frame(house::panel_frame()).show(
            ui,
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Pivot table editor").size(16.0).color(theme::TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if house::close_button(ui) {
                            self.closed_for = Some(name.clone());
                            self.close();
                        }
                    });
                });
                if !self.open {
                    return;
                }
                ui.add_space(house::S);
                egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(house::panel_scroll_height(ui)).show(ui, |ui| {
                    house::form_style(ui);
                    ui.set_width(PANEL_W);
                    house::label(ui, "Range", "");
                    house::text(ui, info["source"].as_str().unwrap_or("").trim_start_matches('='));
                    if let Some(e) = &self.error {
                        ui.add_space(house::S);
                        ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
                    }
                    for (key, title) in [("rows", "Rows"), ("columns", "Columns"), ("values", "Values"), ("filters", "Filters")] {
                        house::gap(ui);
                        self.area(app, ui, &name, &info, key, title);
                    }
                });
            },
        );
    }

    /// One area: its heading with Add, then a card per field.
    fn area(&mut self, app: &mut SheetApp, ui: &mut Ui, pivot: &str, info: &Json, key: &str, title: &str) {
        let fields: Vec<Json> = info["fields"].as_array().cloned().unwrap_or_default();
        ui.horizontal(|ui| {
            house::caps(ui, title);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(RichText::new("Add").size(house::BODY).color(house::GREEN), |ui| {
                    house::menu(ui, 220.0);
                    for f in fields.iter().filter(|f| !f["calculated"].as_bool().unwrap_or(false)) {
                        let n = f["name"].as_str().unwrap_or("");
                        if house::menu_row(ui, None, n, "", true) {
                            self.run(app, "pivot.addField", json!({ "pivot": pivot, "field": n, "area": key }));
                            ui.close();
                        }
                    }
                    if key == "values" {
                        house::menu_sep(ui);
                        if house::menu_row(ui, None, "Calculated field", "", true) {
                            let taken = |s: &str| fields.iter().any(|f| f["name"].as_str() == Some(s));
                            let n = (1..100).map(|i| format!("Calculated field {i}")).find(|s| !taken(s)).unwrap_or_default();
                            self.calc = Some((None, n, "=".into()));
                            ui.close();
                        }
                    }
                });
            });
        });
        let placed: Vec<Json> = info[key].as_array().cloned().unwrap_or_default();
        let shown: Vec<Json> = if key == "filters" { placed.into_iter().filter(|f| f["report"].as_bool().unwrap_or(true)).collect() } else { placed };
        if shown.is_empty() && !(key == "values" && self.calc.as_ref().is_some_and(|c| c.0.is_none())) {
            house::note(ui, "None yet: use Add.");
        }
        for f in &shown {
            house::card_frame().show(ui, |ui| {
                ui.set_width(PANEL_W - 2.0 * house::L);
                self.card(app, ui, pivot, info, key, f);
            });
            ui.add_space(house::S);
        }
        if key == "values" && self.calc.as_ref().is_some_and(|c| c.0.is_none()) {
            house::card_frame().show(ui, |ui| {
                ui.set_width(PANEL_W - 2.0 * house::L);
                self.calc_editor(app, ui, pivot);
            });
        }
    }

    fn card(&mut self, app: &mut SheetApp, ui: &mut Ui, pivot: &str, info: &Json, key: &str, f: &Json) {
        let field = f["field"].as_str().unwrap_or("").to_string();
        let caption = f["name"].as_str().map(str::to_string).unwrap_or_else(|| field.clone());
        let calculated = info["calculatedFields"].as_array().is_some_and(|c| c.iter().any(|c| c["name"].as_str() == Some(field.as_str())));
        ui.horizontal(|ui| {
            ui.label(RichText::new(&caption).size(house::LABEL).strong().color(theme::TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if house::close_button(ui) {
                    let p = if key == "values" {
                        json!({ "pivot": pivot, "field": caption, "area": "values" })
                    } else {
                        json!({ "pivot": pivot, "field": field, "area": key })
                    };
                    self.run(app, "pivot.removeField", p);
                }
            });
        });
        let half = (PANEL_W - 2.0 * house::L - house::S) / 2.0;
        match key {
            "rows" | "columns" => {
                let date = info["fields"]
                    .as_array()
                    .and_then(|a| a.iter().find(|x| x["name"].as_str() == Some(field.as_str())))
                    .is_some_and(|x| x["date"].as_bool().unwrap_or(false));
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        house::note(ui, "Order");
                        let now = f["sort"].as_str().unwrap_or("asc");
                        let mut i = ORDERS.iter().position(|(_, v)| *v == now).unwrap_or(0);
                        let labels: Vec<&str> = ORDERS.iter().map(|(l, _)| *l).collect();
                        if house::select(ui, &format!("pv_order_{key}_{field}"), half, &mut i, &labels, &[]) {
                            self.run(app, "pivot.sort", json!({ "pivot": pivot, "field": field, "order": ORDERS[i].1 }));
                        }
                    });
                    if date {
                        ui.vertical(|ui| {
                            house::note(ui, "Group by");
                            let now = f["group"].as_str().unwrap_or("none");
                            let mut i = GROUPS.iter().position(|(_, v)| *v == now).unwrap_or(0);
                            let labels: Vec<&str> = GROUPS.iter().map(|(l, _)| *l).collect();
                            if house::select(ui, &format!("pv_group_{key}_{field}"), half, &mut i, &labels, &[]) {
                                self.run(app, "pivot.group", json!({ "pivot": pivot, "field": field, "by": GROUPS[i].1 }));
                            }
                        });
                    }
                });
            }
            "values" => {
                if calculated {
                    let formula = info["calculatedFields"]
                        .as_array()
                        .and_then(|c| c.iter().find(|c| c["name"].as_str() == Some(field.as_str())))
                        .and_then(|c| c["formula"].as_str())
                        .unwrap_or("=")
                        .to_string();
                    if self.calc.as_ref().is_some_and(|c| c.0.as_deref() == Some(field.as_str())) {
                        self.calc_editor(app, ui, pivot);
                    } else {
                        ui.horizontal(|ui| {
                            house::note(ui, &formula);
                            if ui.add(house::link("Edit")).clicked() {
                                self.calc = Some((Some(field.clone()), field.clone(), formula.clone()));
                            }
                        });
                    }
                }
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        house::note(ui, "Summarise by");
                        let now = f["func"].as_str().unwrap_or("sum");
                        let mut i = FUNCS.iter().position(|(_, v)| *v == now).unwrap_or(0);
                        let labels: Vec<&str> = FUNCS.iter().map(|(l, _)| *l).collect();
                        if house::select(ui, &format!("pv_func_{caption}"), half, &mut i, &labels, &[]) {
                            self.run(app, "pivot.valueSettings", json!({ "pivot": pivot, "field": caption, "func": FUNCS[i].1 }));
                        }
                    });
                    ui.vertical(|ui| {
                        house::note(ui, "Show as");
                        let now = f["showAs"].as_str().unwrap_or("normal");
                        let mut i = SHOW_AS.iter().position(|(_, v)| *v == now).unwrap_or(0);
                        let labels: Vec<&str> = SHOW_AS.iter().map(|(l, _)| *l).collect();
                        if house::select(ui, &format!("pv_show_{caption}"), half, &mut i, &labels, &[]) {
                            self.run(app, "pivot.valueSettings", json!({ "pivot": pivot, "field": caption, "showAs": SHOW_AS[i].1 }));
                        }
                    });
                });
            }
            _ => self.filter_card(app, ui, pivot, info, &field, f),
        }
    }

    /// Filter by values (tick boxes) and by condition.
    fn filter_card(&mut self, app: &mut SheetApp, ui: &mut Ui, pivot: &str, info: &Json, field: &str, f: &Json) {
        let items: Vec<String> = info["fields"]
            .as_array()
            .and_then(|a| a.iter().find(|x| x["name"].as_str() == Some(field)))
            .and_then(|x| x["items"].as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let selected: Option<Vec<String>> = f["selected"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect());
        let shown = |i: &str| selected.as_ref().is_none_or(|s| s.iter().any(|x| x.eq_ignore_ascii_case(i)));
        let n = items.iter().filter(|i| shown(i)).count();
        house::note(ui, &if n == items.len() { "Showing all items".to_string() } else { format!("Showing {n} of {} items", items.len()) });
        ui.add_space(house::XS);
        house::note(ui, "Filter by condition");
        if self.condition.field != field {
            self.condition = Condition { field: field.to_string(), ..Condition::default() };
        }
        let mut apply_condition = false;
        ui.horizontal(|ui| {
            let half = (PANEL_W - 2.0 * house::L - house::S) / 2.0;
            house::select(ui, &format!("pv_cond_{field}"), half, &mut self.condition.op, CONDITIONS, &[0, 5]);
            if self.condition.op > 0 {
                let r = ui.add_sized([half, house::FIELD_HEIGHT], house::field(&mut self.condition.value, house::BODY));
                apply_condition = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            }
        });
        if self.condition.op > 0 && ui.add(house::link("Apply condition")).clicked() {
            apply_condition = true;
        }
        if apply_condition {
            let keep: Vec<&String> = items.iter().filter(|i| passes(self.condition.op, i, &self.condition.value)).collect();
            if keep.is_empty() {
                self.error = Some("No items match that condition.".into());
            } else {
                self.run(app, "pivot.filter", json!({ "pivot": pivot, "field": field, "selected": keep }));
            }
        }
        ui.add_space(house::XS);
        house::note(ui, "Filter by values");
        ui.horizontal(|ui| {
            if ui.add(house::link("Select all")).clicked() {
                self.run(app, "pivot.filter", json!({ "pivot": pivot, "field": field, "selected": Json::Null }));
            }
            if ui.add(house::link("Clear")).clicked() && !items.is_empty() {
                // A pivot needs one item at least: keep the first, as Sheets keeps the list open.
                self.run(app, "pivot.filter", json!({ "pivot": pivot, "field": field, "selected": [items[0].clone()] }));
            }
        });
        egui::ScrollArea::vertical().id_salt(format!("pv_items_{field}")).max_height(160.0).show(ui, |ui| {
            for item in &items {
                let mut on = shown(item);
                if ui.checkbox(&mut on, item.as_str()).changed() {
                    let mut now: Vec<String> = items.iter().filter(|i| shown(i)).cloned().collect();
                    if on {
                        now.push(item.clone());
                    } else {
                        now.retain(|x| x != item);
                    }
                    let p = if now.len() == items.len() { Json::Null } else { json!(now) };
                    if now.is_empty() {
                        self.error = Some("Keep at least one item ticked.".into());
                    } else {
                        self.run(app, "pivot.filter", json!({ "pivot": pivot, "field": field, "selected": p }));
                    }
                }
            }
        });
    }

    /// The name and formula of a calculated field, with Save and Cancel (and Delete when it exists).
    fn calc_editor(&mut self, app: &mut SheetApp, ui: &mut Ui, pivot: &str) {
        let Some((old, mut name, mut formula)) = self.calc.take() else { return };
        house::note(ui, "Name");
        ui.add_sized([PANEL_W - 2.0 * house::L, house::FIELD_HEIGHT], house::field(&mut name, house::BODY));
        house::note(ui, "Formula, using the fields' names (='Sales'*0.2)");
        ui.add_sized([PANEL_W - 2.0 * house::L, house::FIELD_HEIGHT], house::field(&mut formula, house::BODY));
        let mut keep = true;
        ui.horizontal(|ui| {
            if ui.add(house::primary("Save")).clicked() {
                if let Some(o) = old.as_ref().filter(|o| **o != name) {
                    self.run(app, "pivot.calculatedField", json!({ "pivot": pivot, "name": o, "delete": true }));
                }
                let add_value = old.is_none() || old.as_ref().is_some_and(|o| *o != name);
                self.run(app, "pivot.calculatedField", json!({ "pivot": pivot, "name": name, "formula": formula, "values": add_value }));
                keep = self.error.is_some();
            }
            if ui.add(house::secondary("Cancel")).clicked() {
                keep = false;
            }
            if let Some(o) = &old
                && ui.add(house::link("Delete")).clicked()
            {
                self.run(app, "pivot.calculatedField", json!({ "pivot": pivot, "name": o, "delete": true }));
                keep = false;
            }
        });
        if keep {
            self.calc = Some((old, name, formula));
        }
    }
}

/// The test hook's view of the pivot the active cell is in.
#[cfg(target_arch = "wasm32")]
pub fn state(app: &SheetApp) -> Json {
    let Some(name) = gridcraft_ui_egui::pivot_pane::active_pivot(app) else { return Json::Null };
    let pt = app.session.active().and_then(|d| d.wb.active()).and_then(|sh| sh.pivots.iter().find(|p| p.name == name));
    pt.and_then(|p| serde_json::to_value(p).ok()).unwrap_or(Json::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_read_text_and_numbers() {
        assert!(passes(1, "North East", "east"));
        assert!(!passes(2, "North East", "east"));
        assert!(passes(3, "North", "no"));
        assert!(passes(6, "1,200", "1000"));
        assert!(!passes(6, "abc", "1"));
        assert!(passes(10, "2", "2.0"));
        assert!(passes(10, "West", "west"));
        assert!(passes(11, "West", "East"));
        assert!(passes(0, "anything", ""));
    }
}
