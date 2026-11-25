//! Data › Filter views, as in Google Sheets: named filters over a range that each person opens for
//! themselves. GridCraft keeps the views in the sheet (saved in the .xlsx and sent to the others
//! like any change) and shows one with `show_filter_view`, which changes only what this tab draws.
//! Which view this tab has open is kept here; after anything changes the sheet (an edit, undo, a
//! change from someone else) the view is shown again as it now filters. The dark bar above the
//! grid names the open view and its range, and closes it.

use std::cell::RefCell;
use std::sync::Arc;

use egui::{Frame, Margin, RichText, Ui, vec2};
use gridcraft_engine::cmd::data::show_filter_view;
use gridcraft_engine::model::FilterView;
use gridcraft_ui_egui::SheetApp;
use serde_json::{Value, json};

use crate::{house, icons};

/// The filter view open in this tab.
#[derive(Clone, Debug, PartialEq)]
struct Open {
    doc: u64,
    /// The sheet's id while the app runs, so moving or renaming sheets doesn't lose it.
    sheet: u64,
    view: u32,
    /// The address of the sheet as last shown with the view: another one needs showing again.
    shown: usize,
}

thread_local! {
    static OPEN: RefCell<Option<Open>> = const { RefCell::new(None) };
}

/// The menu ids this module answers (`panel:filterViewOpen` takes `{id}`).
pub const IDS: &[&str] = &["panel:filterViewNew", "panel:filterViewSave", "panel:filterViewOpen", "panel:filterViewClose"];

/// Whether `id` is one of ours.
pub fn handles(id: &str) -> bool {
    IDS.contains(&id)
}

/// Runs one of our menu entries.
pub fn run(app: &mut SheetApp, id: &str, p: &Value) {
    let has_filter = app.session.active().and_then(|d| d.wb.active()).is_some_and(|sh| sh.autofilter.is_some());
    let made = match id {
        "panel:filterViewNew" => app.run("data.createFilterView", json!({})),
        "panel:filterViewSave" if !has_filter => Err("This sheet has no filter to save. Create a filter first (Data › Create a filter).".into()),
        "panel:filterViewSave" => app.run("data.createFilterView", json!({"fromFilter": true})),
        "panel:filterViewOpen" => p.get("id").cloned().map(|v| json!({ "id": v })).ok_or_else(|| "Which filter view?".to_string()),
        _ => {
            close(app);
            return;
        }
    };
    match made.map(|m| m.get("id").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok())) {
        Ok(Some(view)) => open(app, view),
        Ok(None) => {}
        Err(e) => app.message = Some(("Filter views".into(), e)),
    }
}

/// Opens filter view `view` of the active sheet in this tab (closing the one open before).
pub fn open(app: &mut SheetApp, view: u32) {
    close(app);
    let Some(d) = app.session.active() else { return };
    let Some(sh) = d.wb.active() else { return };
    if !sh.filter_views.iter().any(|v| v.id == view) {
        return;
    }
    let open = Open { doc: d.uid, sheet: sh.uid.get(), view, shown: 0 };
    OPEN.with(|o| *o.borrow_mut() = Some(open));
    refresh(app);
}

/// Closes the open filter view: the sheet shows its own rows again.
pub fn close(app: &mut SheetApp) {
    let Some(o) = OPEN.with(|o| o.borrow_mut().take()) else { return };
    show_here(app, &o, None);
}

/// The open view's id, when it is on the active sheet of the active spreadsheet.
pub fn open_view(app: &SheetApp) -> Option<u32> {
    let o = OPEN.with(|o| o.borrow().clone())?;
    let d = app.session.active().filter(|d| d.uid == o.doc)?;
    d.wb.active().filter(|sh| sh.uid.get() == o.sheet).map(|_| o.view)
}

/// Shows `view` (or no view) on the open view's sheet, without making the file look changed.
/// Returns the sheet's address afterwards.
fn show_here(app: &mut SheetApp, o: &Open, view: Option<u32>) -> Option<usize> {
    let d = app.session.active_mut().filter(|d| d.uid == o.doc)?;
    let si = d.wb.sheets.iter().position(|s| s.uid.get() == o.sheet)?;
    // What a tab shows is not a change to the file: a saved file stays saved.
    let saved = Arc::ptr_eq(&d.wb, &d.saved);
    show_filter_view(Arc::make_mut(&mut d.wb), si, view);
    if saved {
        d.saved = d.wb.clone();
    }
    d.wb.sheets.get(si).map(|s| Arc::as_ptr(s) as usize)
}

/// Each frame: shows the open view again when its sheet changed, and closes it when the view was
/// deleted (here or by someone else), its sheet went, or another sheet was chosen (as in Sheets).
pub fn refresh(app: &mut SheetApp) {
    let Some(mut o) = OPEN.with(|o| o.borrow().clone()) else { return };
    let Some(d) = app.session.active().filter(|d| d.uid == o.doc) else { return };
    let Some(sh) = d.wb.sheets.iter().find(|s| s.uid.get() == o.sheet) else {
        OPEN.with(|x| *x.borrow_mut() = None);
        return;
    };
    let elsewhere = d.wb.active().is_none_or(|a| a.uid.get() != o.sheet);
    if elsewhere || !sh.filter_views.iter().any(|v| v.id == o.view) {
        close(app);
        return;
    }
    if Arc::as_ptr(sh) as usize == o.shown {
        return;
    }
    o.shown = show_here(app, &o, Some(o.view)).unwrap_or(0);
    OPEN.with(|x| *x.borrow_mut() = Some(o));
}

/// The open view as the test hook reports it: `{id, name, range, hiddenRows}`, or null.
/// Used by the browser's test hook (testhook.rs) and by this file's tests; not by the desktop app.
#[cfg(any(target_arch = "wasm32", test))]
pub fn state(app: &SheetApp) -> Value {
    let Some(id) = open_view(app) else { return Value::Null };
    let Some(sh) = app.session.active().and_then(|d| d.wb.active()) else { return Value::Null };
    let Some(v) = sh.filter_views.iter().find(|v| v.id == id) else { return Value::Null };
    let hidden = (v.range.start.row + 1..=v.range.end.row).filter(|r| sh.is_row_hidden(*r)).count();
    json!({ "id": v.id, "name": v.name, "range": v.range.a1(), "hiddenRows": hidden })
}

/// The active sheet's filter views, listed under Data › Filter views after its two entries; a ✓
/// by the open one. Choosing one opens it.
pub fn menu_list(app: &mut SheetApp, ui: &mut Ui, width: f32) {
    let views: Vec<FilterView> = app.session.active().and_then(|d| d.wb.active()).map(|sh| sh.filter_views.clone()).unwrap_or_default();
    if views.is_empty() {
        return;
    }
    super::menu_sep_width(ui, width);
    let open_now = open_view(app);
    let names: Vec<String> = views.iter().map(|v| v.name.clone()).collect();
    let current = views.iter().position(|v| Some(v.id) == open_now);
    // Choosing the open one closes it, as in Sheets.
    // In line with the labels of the two rows above, which have icons.
    let text_x = f32::from(house::MENU_INSET) + 18.0 + 14.0;
    match super::pick_rows_from(ui, width, &names, current, text_x).and_then(|i| views.get(i)) {
        Some(v) if open_now == Some(v.id) => close(app),
        Some(v) => open(app, v.id),
        None => {}
    }
}

/// The text boxes of the view bar while they are being typed in.
#[derive(Default)]
pub struct Bar {
    name: String,
    range: String,
    /// The view (and its name and range) the boxes were filled from.
    from: Option<(u32, String, String)>,
}

impl Bar {
    /// The dark bar above the grid while a filter view is open: its name and range (both can be
    /// changed), a ⋮ menu (Duplicate, Delete) and ✕ to close it.
    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        refresh(app);
        let Some(id) = open_view(app) else {
            self.from = None;
            return;
        };
        let Some(view) = app.session.active().and_then(|d| d.wb.active()).and_then(|sh| sh.filter_views.iter().find(|v| v.id == id).cloned()) else {
            return;
        };
        let now = (view.id, view.name.clone(), view.range.a1());
        if self.from.as_ref() != Some(&now) {
            self.name = now.1.clone();
            self.range = now.2.clone();
            self.from = Some(now);
        }
        egui::Panel::top("starsheets_filter_view")
            .show_separator_line(false)
            .exact_size(house::BUTTON_HEIGHT + 2.0 * house::S)
            .frame(Frame::NONE.fill(house::VIEW_BAR).inner_margin(Margin::symmetric(house::MENU_INSET, 0)))
            .show(ui, |ui| ui.horizontal_centered(|ui| self.contents(app, ui, &view)));
    }

    fn contents(&mut self, app: &mut SheetApp, ui: &mut Ui, view: &FilterView) {
        ui.spacing_mut().item_spacing.x = house::S;
        let label = |ui: &mut Ui, text: &str| ui.label(RichText::new(text).size(house::LABEL).color(house::VIEW_BAR_TEXT));
        label(ui, "Name");
        let name = ui.add(
            house::field(&mut self.name, house::BODY).desired_width(200.0).text_color(house::VIEW_BAR_TEXT).background_color(house::VIEW_BAR_FIELD),
        );
        if name.lost_focus() && self.name.trim() != view.name && !self.name.trim().is_empty() {
            app.run_or_alert("data.updateFilterView", json!({ "view": view.id, "name": self.name.trim() }));
        }
        ui.add_space(house::M);
        label(ui, "Range");
        let range = ui.add(
            house::field(&mut self.range, house::BODY).desired_width(120.0).text_color(house::VIEW_BAR_TEXT).background_color(house::VIEW_BAR_FIELD),
        );
        if range.lost_focus() && self.range.trim() != view.range.a1() {
            if gridcraft_engine::core::RangeRef::parse(self.range.trim()).is_some() {
                app.run_or_alert("data.updateFilterView", json!({ "view": view.id, "range": self.range.trim() }));
            } else {
                app.message = Some(("Filter views".into(), format!("“{}” isn't a range. Type one like A1:D20.", self.range.trim())));
                self.range = view.range.a1();
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // White icons with a faint light circle on hover and no frame, as on Google's dark bar.
            let w = &mut ui.visuals_mut().widgets;
            for (state, fill) in [
                (&mut w.inactive, egui::Color32::TRANSPARENT),
                (&mut w.hovered, house::VIEW_BAR_HOVER),
                (&mut w.active, house::VIEW_BAR_HOVER),
                (&mut w.open, house::VIEW_BAR_HOVER),
            ] {
                state.weak_bg_fill = fill;
                state.bg_fill = fill;
                state.bg_stroke = egui::Stroke::NONE;
                state.corner_radius = egui::CornerRadius::same(16);
                state.expansion = 0.0;
            }
            let icon_button = |name: &str, fallback: &str| {
                match icons::icon_in(name, house::VIEW_BAR_TEXT) {
                    Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(20.0, 20.0))),
                    None => egui::Button::new(RichText::new(fallback).size(house::LABEL).color(house::VIEW_BAR_TEXT)),
                }
                .frame_when_inactive(false)
                .min_size(vec2(32.0, 32.0))
            };
            if ui.add(icon_button("close", "✕")).on_hover_text("Close filter view").clicked() {
                close(app);
            }
            let options = |ui: &mut Ui| {
                house::menu(ui, 220.0);
                if house::menu_row(ui, Some("content_copy"), "Duplicate", "", true) {
                    if let Ok(made) = app.run("data.createFilterView", json!({ "copyOf": view.id }))
                        && let Some(id) = made.get("id").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok())
                    {
                        open(app, id);
                    }
                    ui.close();
                }
                if house::menu_row(ui, Some("delete"), "Delete", "", true) {
                    app.run_or_alert("data.deleteFilterView", json!({ "view": view.id }));
                    close(app);
                    ui.close();
                }
            };
            egui::containers::menu::MenuButton::from_button(icon_button("more_vert", "⋮")).ui(ui, options).0.on_hover_text("Filter view options");
        });
    }
}

#[cfg(test)]
mod tests {
    use gridcraft_engine::Session;
    use gridcraft_ui_egui::Services;

    use super::*;

    /// A spreadsheet with a header and four rows: name and team.
    fn app() -> SheetApp {
        let mut app = SheetApp::new(Session::new(), Services::default());
        app.session.execute("file.new", json!({})).unwrap();
        let rows = json!([["name", "team"], ["Ann", "Red"], ["Bob", "Blue"], ["Cy", "Red"], ["Di", "Blue"]]);
        app.session.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
        app
    }

    fn hidden(app: &SheetApp) -> Vec<u32> {
        let sh = app.session.active().unwrap().wb.active().unwrap();
        (0..6).filter(|r| sh.is_row_hidden(*r)).collect()
    }

    #[test]
    fn a_view_opens_filters_for_this_tab_and_closes() {
        let mut app = app();
        run(&mut app, "panel:filterViewNew", &json!({}));
        assert_eq!(state(&app), json!({"id": 1, "name": "Filter 1", "range": "A1:B5", "hiddenRows": 0}));
        // The filter menu adds the open view to its commands.
        app.run("data.filterBy", json!({"column": "B", "values": ["Red"], "view": 1})).unwrap();
        refresh(&mut app);
        assert_eq!(hidden(&app), vec![2, 4]);
        assert_eq!(state(&app)["hiddenRows"], json!(2));
        let d = app.session.active().unwrap();
        assert!(d.wb.active().unwrap().rows.values().all(|r| !r.hidden), "the sheet itself hides nothing");
        // Undo and redo give other sheets; the view is shown on each again.
        app.run("edit.undo", json!({})).unwrap();
        refresh(&mut app);
        assert!(hidden(&app).is_empty());
        app.run("edit.redo", json!({})).unwrap();
        refresh(&mut app);
        assert_eq!(hidden(&app), vec![2, 4]);
        run(&mut app, "panel:filterViewClose", &json!({}));
        assert!(hidden(&app).is_empty());
        assert_eq!(state(&app), Value::Null);
        // Reopened by its id (the menu's list), it filters again.
        run(&mut app, "panel:filterViewOpen", &json!({"id": 1}));
        assert_eq!(hidden(&app), vec![2, 4]);
        // Deleting it closes it.
        app.run("data.deleteFilterView", json!({"view": 1})).unwrap();
        refresh(&mut app);
        assert_eq!(open_view(&app), None);
        assert!(hidden(&app).is_empty());
    }

    #[test]
    fn showing_a_view_leaves_a_saved_file_saved() {
        let mut app = app();
        run(&mut app, "panel:filterViewNew", &json!({}));
        app.run("data.filterBy", json!({"column": "B", "values": ["Blue"], "view": 1})).unwrap();
        run(&mut app, "panel:filterViewClose", &json!({}));
        let d = app.session.active_mut().unwrap();
        d.saved = d.wb.clone();
        run(&mut app, "panel:filterViewOpen", &json!({"id": 1}));
        assert_eq!(hidden(&app), vec![1, 3]);
        assert!(!app.session.active().unwrap().is_dirty());
    }

    #[test]
    fn save_as_filter_view_needs_a_filter() {
        let mut app = app();
        run(&mut app, "panel:filterViewSave", &json!({}));
        assert!(app.message.is_some());
        assert_eq!(open_view(&app), None);
        app.message = None;
        app.run("data.filter", json!({"range": "A1:B5"})).unwrap();
        app.run("data.filterBy", json!({"column": "B", "values": ["Blue"]})).unwrap();
        run(&mut app, "panel:filterViewSave", &json!({}));
        assert_eq!(state(&app)["hiddenRows"], json!(2));
        assert!(app.message.is_none());
    }

    #[test]
    fn another_sheet_closes_the_view() {
        let mut app = app();
        run(&mut app, "panel:filterViewNew", &json!({}));
        assert!(open_view(&app).is_some());
        app.run("home.insertSheet", json!({"name": "Two"})).unwrap();
        app.run("sheet.activate", json!({"sheet": "Two"})).unwrap();
        refresh(&mut app);
        assert_eq!(open_view(&app), None);
    }
}
