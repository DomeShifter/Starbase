//! Format › Number › Custom number format, after Google Sheets: a box for the format code with
//! Apply beside it, a sample of the selected cell in that format, and a list of ready-made formats
//! (each shown as an example) that fill the box. The selection shows the format while you type;
//! closing the window takes it back.

use egui::{RichText, Stroke, vec2};
use gridcraft_engine::core::Value;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

use crate::{house, theme};

/// Asks for the window (the menus set it in egui's memory).
pub const REQUEST: &str = "starsheets_custom_number";
/// The example number when the selected cell has none.
const SAMPLE: f64 = 1_235.0;

/// Google's ready-made formats (with the pound in place of the dollar).
const PRESETS: &[&str] = &[
    "#,##0.00",
    "#,##0",
    "0.00",
    "0",
    "0.00%",
    "0%",
    "£#,##0.00",
    "£#,##0",
    "£#,##0.00;(£#,##0.00)",
    "#,##0.00;[Red]-#,##0.00",
    "#,##0.00;[Red](#,##0.00)",
    "0.00E+00",
    "# ?/?",
    "#,##0.00\" kg\"",
    "@",
];

#[derive(Default)]
pub struct CustomNumber {
    code: Option<String>,
    /// The live preview: (code applied, undo steps).
    preview: Option<(String, usize)>,
}

impl CustomNumber {
    pub fn ui(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        if ctx.data_mut(|m| m.remove_temp::<bool>(egui::Id::unique(REQUEST))).is_some() {
            let now = app.session.active().and_then(|d| {
                let sh = d.wb.active()?;
                Some(d.wb.styles.get(sh.style_id(d.selection.active)).num_fmt.as_str().to_string())
            });
            self.code = Some(now.filter(|c| !c.is_empty() && c != "General").unwrap_or_else(|| "#,##0.00".into()));
            self.preview = None;
        }
        let Some(code) = self.code.as_mut() else { return };
        let value = app
            .session
            .active()
            .and_then(|d| d.wb.active().map(|sh| sh.value(d.selection.active)))
            .filter(|v| matches!(v, Value::Number(_)))
            .unwrap_or(Value::number(SAMPLE));
        let mut done = None;
        let mut closed = false;
        house::dialog(ctx, "starsheets_custom_number", 470.0, "Custom number formats", Some(&mut closed), |ui| {
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(code)
                        .desired_width(470.0 - 88.0 - house::S - 2.0 * house::M)
                        .margin(vec2(house::M, house::S + 1.0))
                        .hint_text("e.g. #,##0.00"),
                );
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
                if ui.add(house::primary("Apply")).clicked() {
                    done = Some(true);
                }
            });
            ui.add_space(house::XS);
            ui.horizontal(|ui| {
                house::note(ui, "Sample:");
                ui.label(RichText::new(sample(&value, code)).size(house::BODY).color(theme::TEXT));
            });
            house::gap(ui);
            // The list's scroll handle keeps the house grey (the dialog's fields are white).
            ui.visuals_mut().widgets.inactive.bg_fill = house::SCROLL_HANDLE;
            egui::ScrollArea::vertical().max_height(330.0).show(ui, |ui| {
                crate::shell::menu_style_width(ui, ui.available_width());
                for p in PRESETS {
                    let r = ui.add(
                        egui::Button::new(RichText::new(sample(&value, p)).size(house::BODY).color(theme::TEXT))
                            .shortcut_text(RichText::new(*p).size(12.0).color(theme::TEXT_DIM))
                            .frame_when_inactive(false)
                            .min_size(vec2(ui.available_width(), house::ROW_HEIGHT)),
                    );
                    if r.clicked() {
                        *code = p.to_string();
                    }
                    ui.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0, theme::DIVIDER));
                }
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(false);
            }
        });
        if closed {
            done = Some(false);
        }
        let code = code.clone();
        // Live preview: the selection shows the format as it is typed.
        if self.preview.as_ref().is_none_or(|(c, _)| *c != code) && done.is_none() && !code.trim().is_empty() {
            self.undo_preview(app);
            let ok = app.run("home.numberFormat", json!({ "code": code })).is_ok();
            self.preview = Some((code.clone(), usize::from(ok)));
        }
        match done {
            Some(true) => {
                // Keep the preview if it is this code; else apply it now.
                if self.preview.as_ref().is_none_or(|(c, n)| *c != code || *n == 0) {
                    self.undo_preview(app);
                    app.run_or_alert("home.numberFormat", json!({ "code": code }));
                }
                self.preview = None;
                self.code = None;
            }
            Some(false) => {
                self.undo_preview(app);
                self.code = None;
            }
            None => {}
        }
    }

    fn undo_preview(&mut self, app: &mut SheetApp) {
        if let Some((_, n)) = self.preview.take() {
            for _ in 0..n {
                let _ = app.run("edit.undo", json!({}));
            }
        }
    }
}

/// `value` in format `code`, or a note that the code isn't valid.
fn sample(value: &Value, code: &str) -> String {
    gridcraft_engine::numfmt::text_function(value, code, gridcraft_engine::core::date::DateSystem::default())
        .unwrap_or_else(|_| "(not a valid format)".to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_preset_formats_the_sample() {
        let v = gridcraft_engine::core::Value::number(super::SAMPLE);
        for p in super::PRESETS {
            assert_ne!(super::sample(&v, p), "(not a valid format)", "{p}");
        }
        assert_eq!(super::sample(&v, "#,##0.00"), "1,235.00");
    }
}
