//! Data › Named ranges, after Google Sheets: a side panel listing each name and its range, with
//! "Add a range", and an editor of just a name and a range (no scope or comment). Takes over
//! GridCraft's Name Manager and Define Name dialogs.

use egui::{RichText, Stroke, Ui, vec2};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

use crate::{house, theme};

const PANEL_W: f32 = 320.0;

/// A name being added or changed: the name it had (None for a new one), and the two boxes.
struct Draft {
    old: Option<String>,
    name: String,
    range: String,
    error: Option<String>,
}

#[derive(Default)]
pub struct NamesPanel {
    pub open: bool,
    pub just_opened: bool,
    draft: Option<Draft>,
}

/// A name's range as Sheets shows it: no `=` or `$`.
fn plain(formula: &str) -> String {
    formula.trim_start_matches('=').replace('$', "")
}

/// A range made absolute for a name (`Sheet1!B2:C4` → `Sheet1!$B$2:$C$4`): a name's relative
/// references move with the cell they're used from, which a named range must not.
pub(crate) fn absolute(range: &str) -> String {
    let (sheet, body) = match range.rsplit_once('!') {
        Some((s, b)) => (Some(s), b),
        None => (None, range),
    };
    // Only cell references: a constant or formula (=0.2, =A1*2) is kept as written.
    let plain = body.replace('$', "");
    let is_ref = gridcraft_engine::core::RangeRef::parse(&plain).is_some()
        || plain.split(':').count() == 2
            && plain.split(':').all(|p| !p.is_empty() && (p.chars().all(|c| c.is_ascii_alphabetic()) || p.chars().all(|c| c.is_ascii_digit())));
    if !is_ref {
        return range.to_string();
    }
    let fix = |part: &str| -> String {
        let p = part.trim().replace('$', "");
        let letters: String = p.chars().take_while(char::is_ascii_alphabetic).collect();
        let digits = &p[letters.len()..];
        match (letters.is_empty(), digits.is_empty()) {
            (false, false) => format!("${letters}${digits}"),
            (false, true) => format!("${letters}"),
            (true, false) => format!("${digits}"),
            _ => p,
        }
    };
    let body: String = body.split(':').map(fix).collect::<Vec<_>>().join(":");
    match sheet {
        Some(s) => format!("{s}!{body}"),
        None => body,
    }
}

/// The selection as a range with its sheet (`Sheet1!A1:B10`).
fn selection(app: &SheetApp) -> String {
    let Some(d) = app.session.active() else { return String::new() };
    let Some(sh) = d.wb.active() else { return String::new() };
    let r = d.selection.ranges.first().copied().unwrap_or_else(|| gridcraft_engine::core::RangeRef::new(d.selection.active, d.selection.active));
    let sheet = if sh.name.chars().all(|c| c.is_alphanumeric() || c == '_') { sh.name.clone() } else { format!("'{}'", sh.name.replace('\'', "''")) };
    format!("{sheet}!{}", r.a1())
}

impl NamesPanel {
    pub fn close(&mut self) {
        self.open = false;
        self.draft = None;
    }

    /// Opens on GridCraft's name dialogs (Data › Named ranges, the cell menu's "Define named range").
    pub fn intercept(&mut self, app: &mut SheetApp) {
        self.just_opened = false;
        let which = app.dialog.as_ref().map(|d| d.name.clone());
        match which.as_deref() {
            Some("nameManager") => {
                app.dialog = None;
                self.open = true;
                self.just_opened = true;
            }
            Some("defineName") => {
                app.dialog = None;
                self.open = true;
                self.just_opened = true;
                self.add(app);
            }
            _ => {}
        }
    }

    fn add(&mut self, app: &SheetApp) {
        let n = app.session.active().map_or(0, |d| d.wb.names.len());
        let taken = |s: &str| app.session.active().is_some_and(|d| d.wb.names.iter().any(|x| x.name.eq_ignore_ascii_case(s)));
        let name = (n + 1..n + 1000).map(|i| format!("NamedRange{i}")).find(|s| !taken(s)).unwrap_or_else(|| "NamedRange".into());
        self.draft = Some(Draft { old: None, name, range: selection(app), error: None });
    }

    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        if !self.open {
            return;
        }
        egui::Panel::right("starsheets_names").exact_size(PANEL_W + 32.0 + house::SCROLL_ROOM).resizable(false).frame(house::panel_frame()).show(
            ui,
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Named ranges").size(16.0).color(theme::TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if house::close_button(ui) {
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
                    if self.draft.is_some() {
                        self.editor(app, ui);
                    } else {
                        self.list(app, ui);
                    }
                });
            },
        );
    }

    fn list(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        let names: Vec<(String, String)> = app
            .session
            .active()
            .map(|d| {
                d.wb.names
                    .iter()
                    .filter(|n| !n.hidden && !n.name.starts_with("_sc_") && !n.name.starts_with("_gc_"))
                    .map(|n| (n.name.clone(), plain(&n.formula)))
                    .collect()
            })
            .unwrap_or_default();
        if ui.add(house::link("+ Add a range")).clicked() {
            self.add(app);
            return;
        }
        ui.add_space(house::S);
        if names.is_empty() {
            house::note(ui, "No named ranges yet. A named range lets formulas say =SUM(Sales) instead of =SUM(B2:B50).");
        }
        for (name, range) in names {
            let r = ui.add(
                egui::Button::new(RichText::new(&name).size(house::BODY).strong().color(theme::TEXT))
                    .right_text(RichText::new(&range).size(house::NOTE).color(theme::TEXT_DIM))
                    .frame_when_inactive(false)
                    .min_size(vec2(PANEL_W, 44.0)),
            );
            ui.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0, theme::DIVIDER));
            if r.clicked() {
                self.draft = Some(Draft { old: Some(name.clone()), name, range, error: None });
            }
        }
    }

    fn editor(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        let Some(d) = self.draft.as_mut() else { return };
        house::label(ui, "Name", "Letters, numbers and _ (no spaces), not like a cell (A1).");
        ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(PANEL_W).margin(vec2(house::M, house::S + 1.0)));
        house::gap(ui);
        house::label(ui, "Range", "");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut d.range).desired_width(PANEL_W - 40.0 - house::S).margin(vec2(house::M, house::S + 1.0)));
            let pick = match crate::icons::icon("grid_on") {
                Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(20.0, 20.0))),
                None => egui::Button::new("⊞"),
            };
            if ui.add(pick.frame(false).min_size(vec2(36.0, 36.0))).on_hover_text("Use the selected cells").clicked() {
                d.range = selection(app);
            }
        });
        if let Some(e) = &d.error {
            ui.label(RichText::new(e).size(house::NOTE).color(house::ERROR));
        }
        let editing = d.old.is_some();
        let labels: &[&str] = if editing { &["Remove", "Cancel", "Done"] } else { &["Cancel", "Done"] };
        match house::actions(ui, labels).map(|i| labels[i]) {
            Some("Remove") => {
                if let Some(old) = d.old.clone() {
                    app.run_or_alert("formulas.deleteName", json!({ "name": old }));
                }
                self.draft = None;
            }
            Some("Cancel") => self.draft = None,
            Some("Done") => {
                let name = d.name.trim().to_string();
                let range = d.range.trim().trim_start_matches('=').to_string();
                let mut p = json!({ "name": name, "refersTo": format!("={}", absolute(&range)) });
                if let Some(old) = &d.old {
                    p["oldName"] = json!(old);
                }
                match app.run("formulas.defineName", p) {
                    Ok(_) => self.draft = None,
                    Err(e) => d.error = Some(e.to_string()),
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use gridcraft_engine::Session;
    use serde_json::json;

    #[test]
    fn ranges_are_made_absolute() {
        assert_eq!(super::absolute("Sheet1!B2:C4"), "Sheet1!$B$2:$C$4");
        assert_eq!(super::absolute("'My sheet'!$A1"), "'My sheet'!$A$1");
        assert_eq!(super::absolute("A:A"), "$A:$A");
        assert_eq!(super::absolute("0.2"), "0.2");
        assert_eq!(super::absolute("Sheet1!A1*2"), "Sheet1!A1*2");
    }

    #[test]
    fn a_name_is_added_renamed_and_removed_like_the_panel_does() {
        let mut s = Session::new();
        s.execute("file.new", json!({})).unwrap();
        s.execute("formulas.defineName", json!({ "name": "Sales", "refersTo": "=Sheet1!B2:B9" })).unwrap();
        s.execute("formulas.defineName", json!({ "name": "Takings", "refersTo": "=Sheet1!B2:B12", "oldName": "Sales" })).unwrap();
        let names = &s.active().unwrap().wb.names;
        assert!(names.iter().any(|n| n.name == "Takings" && super::plain(&n.formula) == "Sheet1!B2:B12"));
        assert!(!names.iter().any(|n| n.name == "Sales"));
        s.execute("formulas.deleteName", json!({ "name": "Takings" })).unwrap();
        assert!(s.active().unwrap().wb.names.iter().all(|n| n.name != "Takings"));
    }
}
